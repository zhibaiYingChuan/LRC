"""
补充测试：主动调用 monitor.check() 验证状态恢复 + P0-A 真实响应时间
"""
import json
import os
import time
import sys

# 彻底禁用代理（必须在 import requests 之前）
for _k in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"]:
    os.environ.pop(_k, None)
os.environ["NO_PROXY"] = "*"
os.environ["no_proxy"] = "*"

import requests
import websocket

CDP_HTTP = "http://127.0.0.1:9223"
SIDECAR = "http://127.0.0.1:3099"
NO_PROXY = {"http": "", "https": ""}

print("=== 补充测试：monitor.check() + P0-A 真实响应 ===\n")

# 1. Sidecar 直接探测（绕过代理）
print("[1] Sidecar 直接探测（10 次 /health）:")
latencies = []
for i in range(10):
    sw = time.time()
    try:
        r = requests.get(f"{SIDECAR}/health", timeout=5, proxies=NO_PROXY)
        ms = round((time.time() - sw) * 1000, 1)
        latencies.append({"i": i, "status": r.status_code, "ms": ms})
        print(f"  #{i}: {r.status_code} ({ms}ms)")
    except Exception as e:
        latencies.append({"i": i, "error": str(e)[:80]})
        print(f"  #{i}: ERROR {str(e)[:80]}")

# 2. 4 个信任中心端点
print("\n[2] 4 个信任中心端点（IA-22-01 验证）:")
for ep in ["/v1/audit-trail", "/v1/trust/data-location", "/v1/trust/network-audit", "/v1/trust/audit-integrity"]:
    sw = time.time()
    try:
        r = requests.get(f"{SIDECAR}{ep}", timeout=8, proxies=NO_PROXY)
        ms = round((time.time() - sw) * 1000, 1)
        try:
            body = r.json()
        except Exception:
            body = r.text[:150]
        print(f"  {ep}: {r.status_code} ({ms}ms) {str(body)[:120]}")
    except Exception as e:
        print(f"  {ep}: ERROR {str(e)[:80]}")

# 3. CDP 主动调用 monitor.check()
print("\n[3] CDP 主动调用 monitor.check() 验证状态恢复:")
targets = requests.get(f"{CDP_HTTP}/json", timeout=5, proxies=NO_PROXY).json()
target = next(t for t in targets if "tauri.localhost" in t.get("url", ""))
ws_url = target["webSocketDebuggerUrl"]
ws = websocket.WebSocket()
ws.connect(ws_url, suppress_origin=True, timeout=15)
mid = [0]

def ev(expr, await_p=False, timeout_ms=12000):
    mid[0] += 1
    payload = {"id": mid[0], "method": "Runtime.evaluate", "params": {
        "expression": expr, "returnByValue": True, "awaitPromise": await_p, "timeout": timeout_ms
    }}
    ws.send(json.dumps(payload))
    deadline = time.time() + (timeout_ms / 1000) + 5
    while time.time() < deadline:
        try:
            ws.settimeout(max(0.5, deadline - time.time()))
            msg = json.loads(ws.recv())
            if msg.get("id") == mid[0]:
                return msg
        except Exception:
            break
    return {"error": "timeout"}

# 启用 Runtime
ws.send(json.dumps({"id": 999, "method": "Runtime.enable"}))
time.sleep(0.5)
try:
    while True:
        ws.settimeout(0.3)
        ws.recv()
except Exception:
    pass
ws.settimeout(20)

# 主动调用 monitor.check() (异步)
r = ev("(async ()=>{const m = window.sidecarHealthMonitor; const before = {isReachable: m._isReachable, online: m.online, status: m._sidecarStatus, lockBusy: m._lockBusy, failCount: m._failCount}; const result = await m.check(); const after = {isReachable: m._isReachable, online: m.online, status: m._sidecarStatus, lockBusy: m._lockBusy, failCount: m._failCount, checkResult: result}; return {before, after};})()", await_p=True, timeout_ms=15000)
val = r.get("result", {}).get("result", {}).get("value")
print(f"  monitor.check() 前: {val.get('before') if val else None}")
print(f"  monitor.check() 后: {val.get('after') if val else None}")

# 4. 5 并发 /health + 10 次串行 /health 测试（CDP 视角）
print("\n[4] CDP 视角 5 并发 /health:")
r = ev("(async ()=>{const t0 = performance.now(); const results = await Promise.all([fetch('http://127.0.0.1:3099/health').then(r=>r.status), fetch('http://127.0.0.1:3099/health').then(r=>r.status), fetch('http://127.0.0.1:3099/health').then(r=>r.status), fetch('http://127.0.0.1:3099/health').then(r=>r.status), fetch('http://127.0.0.1:3099/health').then(r=>r.status)]); const t1 = performance.now(); return {allStatus: results, all200: results.every(s=>s===200), elapsedMs: Math.round(t1-t0)};})()", await_p=True)
val = r.get("result", {}).get("result", {}).get("value")
print(f"  5 并发: {val}")

# 5. 检查 banner 显示状态（monitor.check 后）
print("\n[5] banner 显示状态（monitor.check 后）:")
r = ev("(()=>{const b = document.getElementById('sidecar-down-banner'); const m = window.sidecarHealthMonitor; return {bannerExists: !!b, bannerVisible: b ? getComputedStyle(b).display !== 'none' : null, bannerDisplay: b ? getComputedStyle(b).display : null, monitorReachable: m._isReachable, monitorOnline: m.online, monitorStatus: m._sidecarStatus};})()")
val = r.get("result", {}).get("result", {}).get("value")
print(f"  {val}")

ws.close()
print("\n[OK] 补充测试完成")
