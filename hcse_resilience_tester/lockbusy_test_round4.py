# -*- coding: utf-8 -*-
"""lock_busy 触发测试：POST /v1/memories/consolidate + 并发探测 /v1/health/* 验证 P1-02 降级"""
import requests, threading, time, json, sys

SIDECAR = "http://127.0.0.1:3099"
results = []
lock = threading.Lock()

def post_consolidate(mem_count=0):
    mems = [{"content": f"lock_busy 测试记忆 {i}", "memory_type": "fact",
             "project": "test", "tags": [], "importance": 5,
             "privacy_level": "public", "session_id": "t", "user_id": "t"}
            for i in range(mem_count)]
    t0 = time.time()
    try:
        r = requests.post(SIDECAR + "/v1/memories/consolidate",
                          json={"memories": mems}, timeout=40)
        dt = (time.time() - t0) * 1000
        with lock:
            results.append({"type": "consolidate", "mem_count": mem_count,
                            "code": r.status_code, "ms": int(dt),
                            "body": r.text[:300]})
    except Exception as e:
        dt = (time.time() - t0) * 1000
        with lock:
            results.append({"type": "consolidate", "mem_count": mem_count,
                            "code": None, "ms": int(dt), "err": str(e)[:200]})

def probe_health(path, idx):
    t0 = time.time()
    try:
        r = requests.get(SIDECAR + path, timeout=10)
        dt = (time.time() - t0) * 1000
        body = None
        lock_busy = None
        try:
            body = r.json()
            if isinstance(body, dict):
                lock_busy = body.get("lock_busy")
                if lock_busy is None and isinstance(body.get("data"), dict):
                    lock_busy = body["data"].get("lock_busy")
                if lock_busy is None and isinstance(body.get("raw"), dict):
                    lock_busy = body["raw"].get("lock_busy")
        except Exception:
            body = r.text[:200]
        with lock:
            results.append({"type": "health", "path": path, "idx": idx,
                            "code": r.status_code, "ms": int(dt),
                            "lock_busy": lock_busy,
                            "body_prefix": json.dumps(body, ensure_ascii=False)[:250] if isinstance(body, (dict, list)) else str(body)[:250]})
    except Exception as e:
        dt = (time.time() - t0) * 1000
        with lock:
            results.append({"type": "health", "path": path, "idx": idx,
                            "code": None, "ms": int(dt), "err": str(e)[:200]})

# 阶段1：基线（无 lock_busy）
print("=== 阶段1: 基线探测（lock_busy=false 预期）===", file=sys.stderr)
for p in ["/v1/health/dao_metrics", "/v1/health/system", "/v1/health/detailed"]:
    probe_health(p, 0)
print(json.dumps(results, ensure_ascii=False, indent=2))
results.clear()

# 阶段2：触发 lock_busy + 并发探测
print("=== 阶段2: 触发 consolidate + 并发探测 health（期望捕获 lock_busy 降级）===", file=sys.stderr)
t_cons = threading.Thread(target=post_consolidate, args=(3,))
t_cons.start()
time.sleep(0.05)  # 让 consolidate 先拿锁

threads = []
for i in range(6):
    threads.append(threading.Thread(target=probe_health, args=("/v1/health/dao_metrics", i)))
for p in ["/v1/health/system", "/v1/health/detailed"]:
    for i in range(3):
        threads.append(threading.Thread(target=probe_health, args=(p, i)))
for th in threads:
    th.start()
for th in threads:
    th.join(timeout=12)
t_cons.join(timeout=45)

print(json.dumps(results, ensure_ascii=False, indent=2))
