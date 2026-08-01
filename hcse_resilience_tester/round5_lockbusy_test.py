# -*- coding: utf-8 -*-
"""
HCSE Round 5 lock_busy 触发测试 — 验证 P1-NEW-01/02 + P2-NEW-03
- P2-NEW-03: consolidate handler 三阶段锁安全模式（spawn_blocking 不阻塞 tokio worker）
- P1-NEW-01: /v1/health/* 返回 200+lock_busy=true 降级（前端 hasLockBusy200 识别）
- /health 在 lock_busy 期间仍快速响应（AtomicBool + try_lock）
- CPU 采样验证 spawn_blocking 隔离
"""
import requests, threading, time, json, sys, subprocess

SIDECAR = "http://127.0.0.1:3099"
PID = 25960
results = []
lock = threading.Lock()


def post_consolidate(mem_count):
    mems = [{"content": "Round5 HCSE lock_busy 测试记忆 %d - 验证 P2-NEW-03 三阶段锁安全模式 spawn_blocking 隔离" % i,
             "memory_type": "fact", "project": "hcse-round5", "tags": ["hcse", "round5", "lockbusy"],
             "importance": 5, "privacy_level": "public", "session_id": "r5", "user_id": "r5"}
            for i in range(mem_count)]
    t0 = time.time()
    try:
        r = requests.post(SIDECAR + "/v1/memories/consolidate", json={"memories": mems}, timeout=40)
        dt = (time.time() - t0) * 1000
        with lock:
            results.append({"type": "consolidate", "mem_count": mem_count, "code": r.status_code,
                            "ms": int(dt), "body": r.text[:600]})
    except Exception as e:
        dt = (time.time() - t0) * 1000
        with lock:
            results.append({"type": "consolidate", "mem_count": mem_count, "code": None,
                            "ms": int(dt), "err": str(e)[:200]})


def probe(path, idx):
    t0 = time.time()
    try:
        r = requests.get(SIDECAR + path, timeout=10)
        dt = (time.time() - t0) * 1000
        try:
            body = r.json()
        except Exception:
            body = r.text[:200]
        lock_busy = None
        if isinstance(body, dict):
            lock_busy = body.get("lock_busy")
            if lock_busy is None and isinstance(body.get("data"), dict):
                lock_busy = body["data"].get("lock_busy")
        with lock:
            results.append({"type": "health", "path": path, "idx": idx, "code": r.status_code,
                            "ms": int(dt), "lock_busy": lock_busy})
    except Exception as e:
        dt = (time.time() - t0) * 1000
        with lock:
            results.append({"type": "health", "path": path, "idx": idx, "code": None,
                            "ms": int(dt), "err": str(e)[:200]})


def cpu_sample(pid, duration_s, label):
    """通过 PowerShell 采样进程 CPU（2s 窗口）"""
    try:
        ps_cmd = (
            "$p = Get-Process -Id %d -ErrorAction SilentlyContinue; "
            "if ($p) { $t0 = $p.CPU; Start-Sleep -Seconds %d; $p.Refresh(); "
            "$t1 = $p.CPU; Write-Output ('cpu_delta_%s=' + [math]::Round($t1-$t0,3) + 's cpu_total=' + [math]::Round($t1,3) + 's') }"
        ) % (pid, duration_s, label)
        out = subprocess.check_output(
            ["powershell", "-NoProfile", "-Command", ps_cmd],
            stderr=subprocess.STDOUT, timeout=duration_s + 10
        ).decode("utf-8", errors="replace").strip()
        return out
    except Exception as e:
        return "cpu_sample_err: " + str(e)[:200]


# ============ 阶段 1: 基线探测（lock_busy=false 预期）============
print("=== 阶段1: 基线探测（lock_busy=false 预期）===", file=sys.stderr)
baseline = []
for p in ["/health", "/v1/health/dao_metrics", "/v1/health/system", "/v1/health/detailed"]:
    probe(p, 0)
baseline = list(results)
results.clear()
print(json.dumps(baseline, ensure_ascii=False, indent=2))

# ============ 阶段 2: 触发 consolidate + 并发探测（lock_busy 窗口捕获）============
print("\n=== 阶段2: 触发 consolidate(30 记忆) + 并发探测 ===", file=sys.stderr)

# 启动 consolidate 线程（30 个记忆，延长 luoshu_synthesize 耗时）
t_cons = threading.Thread(target=post_consolidate, args=(30,))
t_cons.start()
time.sleep(0.02)  # 让 consolidate 先拿锁

# 高频并发探测：/health（验证 AtomicBool + try_lock 快速响应）+ /v1/health/*（验证 200+lock_busy 降级）
threads = []
# /health 探测 10 次（验证 lock_busy 期间仍快速响应）
for i in range(10):
    threads.append(threading.Thread(target=probe, args=("/health", i)))
# /v1/health/* 各探测 6 次（验证 200+lock_busy=true 降级）
for p in ["/v1/health/dao_metrics", "/v1/health/system", "/v1/health/detailed"]:
    for i in range(6):
        threads.append(threading.Thread(target=probe, args=(p, i)))

for th in threads:
    th.start()
for th in threads:
    th.join(timeout=12)

# 同时做 CPU 采样（consolidate 期间，验证 spawn_blocking 不阻塞 worker）
cpu_dur = cpu_sample(PID, 2, "consolidate_period")
print("CPU_SAMPLE: " + cpu_dur, file=sys.stderr)

t_cons.join(timeout=45)

phase2 = list(results)
results.clear()
print(json.dumps(phase2, ensure_ascii=False, indent=2))

# ============ 阶段 3: consolidate 完成后探测（lock_busy 恢复 false）============
print("\n=== 阶段3: consolidate 完成后探测（lock_busy 恢复 false）===", file=sys.stderr)
for p in ["/health", "/v1/health/dao_metrics", "/v1/health/system", "/v1/health/detailed"]:
    probe(p, 0)
phase3 = list(results)
results.clear()
print(json.dumps(phase3, ensure_ascii=False, indent=2))

# ============ 汇总分析 ============
print("\n=== 汇总分析 ===", file=sys.stderr)
all_phase2 = phase2
cons_result = [r for r in all_phase2 if r["type"] == "consolidate"]
health_results = [r for r in all_phase2 if r["type"] == "health"]

# /health 在 lock_busy 期间的表现
health_probe = [r for r in health_results if r["path"] == "/health"]
health_reachable = [r for r in health_probe if r.get("code") == 200]
health_times = [r["ms"] for r in health_reachable]
health_lockbusy = [r for r in health_reachable if r.get("lock_busy") == True]

# /v1/health/* 在 lock_busy 期间的表现
v1_probe = [r for r in health_results if r["path"].startswith("/v1/health")]
v1_reachable = [r for r in v1_probe if r.get("code") == 200]
v1_lockbusy = [r for r in v1_reachable if r.get("lock_busy") == True]

summary = {
    "consolidate": {
        "result": cons_result[0] if cons_result else None,
        "duration_ms": cons_result[0]["ms"] if cons_result else None,
    },
    "health_during_lockbusy": {
        "total_probes": len(health_probe),
        "reachable": len(health_reachable),
        "lock_busy_true": len(health_lockbusy),
        "times_ms": health_times,
        "avg_ms": round(sum(health_times) / len(health_times), 2) if health_times else None,
        "max_ms": max(health_times) if health_times else None,
    },
    "v1_health_during_lockbusy": {
        "total_probes": len(v1_probe),
        "reachable_200": len(v1_reachable),
        "lock_busy_true": len(v1_lockbusy),
        "sample_lockbusy": v1_lockbusy[:3] if v1_lockbusy else [],
    },
    "cpu_sample_consolidate": cpu_dur,
    "p2_new_03_verdict": "PASS" if (health_reachable and len(health_reachable) >= 8) else "NEED_REVIEW",
    "p1_new_01_verdict": "PASS" if v1_lockbusy else "NEED_REVIEW (lock_busy 窗口可能过短)",
}

print(json.dumps(summary, ensure_ascii=False, indent=2))

# 保存完整证据
full_evidence = {
    "baseline": baseline,
    "phase2_consolidate_and_probe": phase2,
    "phase3_after_consolidate": phase3,
    "summary": summary,
}
with open("evidence/round5_lockbusy_evidence.json", "w", encoding="utf-8") as f:
    json.dump(full_evidence, f, ensure_ascii=False, indent=2)
print("\n证据已保存: evidence/round5_lockbusy_evidence.json", file=sys.stderr)
