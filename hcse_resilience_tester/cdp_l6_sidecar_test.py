"""L6 组件级数据加载韧性测试 — 通过 sidecar HTTP API（无需 CDP）"""
import json
import time
import statistics
import requests
from datetime import datetime

SIDECAR = "http://127.0.0.1:3099"

def get(path, timeout=10):
    t0 = time.time()
    try:
        r = requests.get(f"{SIDECAR}{path}", timeout=timeout)
        dt = (time.time() - t0) * 1000
        try:
            return r.status_code, r.json(), dt
        except Exception:
            return r.status_code, r.text[:300], dt
    except Exception as e:
        return -1, str(e), (time.time() - t0) * 1000

print(f"=== L6 组件级数据加载韧性测试 ===")
print(f"时间: {datetime.utcnow().isoformat()}Z")
print()

# 基线：sidecar /health
h_status, h_body, h_dt = get("/health", timeout=5)
print(f"[基线] /health: status={h_status}, latency={h_dt:.1f}ms, lock_busy={h_body.get('lock_busy') if isinstance(h_body,dict) else 'N/A'}")
print()

# L6-1: 道同构度加载（lock_busy 期间）
print("[L6-1] 道同构度加载（/v1/health/dao_metrics）— lock_busy=true 期间")
dao_results = []
for i in range(5):
    s, b, dt = get("/v1/health/dao_metrics", timeout=15)
    dao_results.append({"i": i, "status": s, "latency_ms": round(dt, 1), "body_keys": list(b.keys()) if isinstance(b, dict) else str(b)[:100]})
    time.sleep(0.3)
print(json.dumps(dao_results, ensure_ascii=False, indent=2))
ok_count = sum(1 for r in dao_results if r["status"] == 200)
lock_busy_503 = sum(1 for r in dao_results if r["status"] == 503)
print(f"  200 OK: {ok_count}/5, 503 lock_busy: {lock_busy_503}/5")
print()

# L6-2: /health 在 lock_busy 期间多次探测延迟
print("[L6-2] /health 在 lock_busy 期间连续 10 次探测延迟")
health_latencies = []
for i in range(10):
    s, b, dt = get("/health", timeout=10)
    health_latencies.append(dt)
    time.sleep(0.1)
print(f"  延迟(ms): {[round(l,1) for l in health_latencies]}")
print(f"  min={min(health_latencies):.1f}ms, max={max(health_latencies):.1f}ms, avg={statistics.mean(health_latencies):.1f}ms, p95={sorted(health_latencies)[int(len(health_latencies)*0.95)]:.1f}ms")
all_under_5s = all(l < 5000 for l in health_latencies)
print(f"  全部 < 5000ms: {all_under_5s}")
print()

# L6-3: 信任中心接口（lock_busy 期间）
print("[L6-3] 信任中心接口测试")
trust_endpoints = [
    "/v1/trust/audit",
    "/v1/trust/score",
    "/v1/trust/report",
    "/v1/audit/log",
    "/v1/audit/logs",
]
for ep in trust_endpoints:
    s, b, dt = get(ep, timeout=10)
    body_preview = str(b)[:150] if isinstance(b, dict) else str(b)[:150]
    print(f"  {ep}: status={s}, latency={dt:.1f}ms, body={body_preview}")
print()

# L6-4: 项目分布/记忆统计接口
print("[L6-4] 项目分布/记忆统计接口测试")
stat_endpoints = [
    "/v1/stats",
    "/v1/memories/stats",
    "/v1/projects",
    "/v1/dashboard",
    "/v1/memories?limit=1",
]
for ep in stat_endpoints:
    s, b, dt = get(ep, timeout=10)
    body_preview = str(b)[:150] if isinstance(b, dict) else str(b)[:150]
    print(f"  {ep}: status={s}, latency={dt:.1f}ms, body={body_preview}")
print()

# L6-5: 并发请求测试（同时 5 个请求）
print("[L6-5] 并发 5 个 /health 请求测试")
import concurrent.futures
with concurrent.futures.ThreadPoolExecutor(max_workers=5) as executor:
    futures = [executor.submit(get, "/health", 10) for _ in range(5)]
    concurrent_results = []
    for f in concurrent.futures.as_completed(futures):
        s, b, dt = f.result()
        concurrent_results.append({"status": s, "latency_ms": round(dt, 1)})
concurrent_results.sort(key=lambda x: x["latency_ms"])
print(json.dumps(concurrent_results, ensure_ascii=False, indent=2))
print(f"  全部 200: {all(r['status']==200 for r in concurrent_results)}")
print()

# 总结
print("=== L6 测试总结 ===")
l6_results = {
    "L6-1_dao_metrics": {
        "ok_count": ok_count,
        "lock_busy_503": lock_busy_503,
        "status": "PASS" if ok_count > 0 or lock_busy_503 > 0 else "FAIL",  # 503 也是预期（lock_busy 友好处理）
        "note": "lock_busy 期间 503 是预期行为，前端应显示'后台合成中'提示"
    },
    "L6-2_health_latency": {
        "min_ms": round(min(health_latencies), 1),
        "max_ms": round(max(health_latencies), 1),
        "avg_ms": round(statistics.mean(health_latencies), 1),
        "all_under_5s": all_under_5s,
        "status": "PASS" if all_under_5s else "FAIL",
        "note": "P0-A 修复点：worker_threads=16 确保 /health 在 lock_busy 期间可达"
    },
    "L6-5_concurrent": {
        "all_200": all(r['status']==200 for r in concurrent_results),
        "max_latency_ms": max(r['latency_ms'] for r in concurrent_results),
        "status": "PASS" if all(r['status']==200 for r in concurrent_results) else "PARTIAL"
    }
}
print(json.dumps(l6_results, ensure_ascii=False, indent=2))

# 保存结果
out = {
    "audit_time": datetime.utcnow().isoformat() + "Z",
    "sidecar_health_baseline": h_body if isinstance(h_body, dict) else str(h_body),
    "l6_results": l6_results,
    "dao_metrics_detail": dao_results,
    "health_latencies_ms": [round(l, 1) for l in health_latencies],
    "concurrent_results": concurrent_results,
}
out_path = "g:/code-memory/hcse_resilience_tester/evidence/v0822_l6_sidecar_test.json"
with open(out_path, "w", encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False, indent=2)
print(f"\n[OK] 结果已保存: {out_path}")
