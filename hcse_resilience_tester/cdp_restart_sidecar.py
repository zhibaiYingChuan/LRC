"""
通过 Tauri invoke 重启 sidecar，验证用户恢复路径
"""
import json
import os
import time

for _k in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"]:
    os.environ.pop(_k, None)
os.environ["NO_PROXY"] = "*"
os.environ["no_proxy"] = "*"

import websocket
import requests

CDP_HTTP = "http://127.0.0.1:9223"
NO_PROXY = {"http": "", "https": ""}

targets = requests.get(f"{CDP_HTTP}/json", timeout=5, proxies=NO_PROXY).json()
target = next(t for t in targets if "tauri.localhost" in t.get("url", ""))
ws_url = target["webSocketDebuggerUrl"]
ws = websocket.WebSocket()
ws.connect(ws_url, suppress_origin=True, timeout=15)
mid = [0]

def ev(expr, await_p=False, timeout_ms=30000):
    mid[0] += 1
    payload = {"id": mid[0], "method": "Runtime.evaluate", "params": {
        "expression": expr, "returnByValue": True, "awaitPromise": await_p, "timeout": timeout_ms
    }}
    ws.send(json.dumps(payload))
    deadline = time.time() + (timeout_ms / 1000) + 10
    while time.time() < deadline:
        try:
            ws.settimeout(max(0.5, deadline - time.time()))
            msg = json.loads(ws.recv())
            if msg.get("id") == mid[0]:
                return msg
        except Exception:
            break
    return {"error": "timeout"}

ws.send(json.dumps({"id": 999, "method": "Runtime.enable"}))
time.sleep(0.5)
try:
    while True:
        ws.settimeout(0.3)
        ws.recv()
except Exception:
    pass
ws.settimeout(30)

# 1. 获取 invoke 函数
print("=== [1] 获取 invoke 函数 ===")
r = ev("(()=>{"
       "if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') return {source: 'internals'};"
       "if (window.__TAURI__ && window.__TAURI__.core && typeof window.__TAURI__.core.invoke === 'function') return {source: 'core'};"
       "if (window.__TAURI__ && typeof window.__TAURI__.invoke === 'function') return {source: 'tauri'};"
       "return {source: null, internalsKeys: window.__TAURI_INTERNALS__ ? Object.keys(window.__TAURI_INTERNALS__) : null, tauriKeys: window.__TAURI__ ? Object.keys(window.__TAURI__) : null};"
       "})()")
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

# 2. 停止 sidecar
print("\n=== [2] 停止 sidecar（invoke stop_sidecar）===")
r = ev("(async ()=>{"
       "try {"
       "  let invoke;"
       "  if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) invoke = window.__TAURI_INTERNALS__.invoke;"
       "  else if (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) invoke = window.__TAURI__.core.invoke;"
       "  else if (window.__TAURI__ && window.__TAURI__.invoke) invoke = window.__TAURI__.invoke;"
       "  if (!invoke) return {hasInvoke: false};"
       "  const result = await invoke('stop_sidecar');"
       "  return {hasInvoke: true, result: result};"
       "} catch(e) { return {hasInvoke: true, error: e.message}; }"
       "})()", await_p=True, timeout_ms=20000)
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

# 3. 等待 3 秒
print("\n=== [3] 等待 3 秒 ===")
time.sleep(3)

# 4. 启动 sidecar
print("\n=== [4] 启动 sidecar（invoke start_sidecar）===")
r = ev("(async ()=>{"
       "try {"
       "  let invoke;"
       "  if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) invoke = window.__TAURI_INTERNALS__.invoke;"
       "  else if (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) invoke = window.__TAURI__.core.invoke;"
       "  else if (window.__TAURI__ && window.__TAURI__.invoke) invoke = window.__TAURI__.invoke;"
       "  if (!invoke) return {hasInvoke: false};"
       "  const result = await invoke('start_sidecar');"
       "  return {hasInvoke: true, result: result};"
       "} catch(e) { return {hasInvoke: true, error: e.message}; }"
       "})()", await_p=True, timeout_ms=60000)
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

# 5. 等待 5 秒让 sidecar 完全启动
print("\n=== [5] 等待 10 秒让 sidecar 完全启动 ===")
time.sleep(10)

# 6. 验证 sidecar 恢复
print("\n=== [6] 验证 sidecar 恢复（monitor.check + /health）===")
r = ev("(async ()=>{"
       "const m = window.sidecarHealthMonitor;"
       "const before = {isReachable: m._isReachable, online: m.online, status: m._sidecarStatus, failCount: m._failCount};"
       "const checkResult = await m.check();"
       "const after = {isReachable: m._isReachable, online: m.online, status: m._sidecarStatus, failCount: m._failCount, checkResult: checkResult};"
       "const b = document.getElementById('sidecar-down-banner');"
       "const banner = {hidden: b.hidden, display: getComputedStyle(b).display};"
       "return {before, after, banner};"
       "})()", await_p=True, timeout_ms=15000)
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

# 7. CDP 内部 fetch /health 测试
print("\n=== [7] CDP 内部 fetch /health 测试（5 次）===")
r = ev("(async ()=>{"
       "const results = [];"
       "for (let i = 0; i < 5; i++) {"
       "  const t0 = performance.now();"
       "  try {"
       "    const r = await fetch('http://127.0.0.1:3099/health');"
       "    const t1 = performance.now();"
       "    results.push({i: i, status: r.status, ms: Math.round(t1-t0)});"
       "  } catch(e) { results.push({i: i, error: e.message}); }"
       "}"
       "return {results: results};"
       "})()", await_p=True, timeout_ms=30000)
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

# 8. 验证信任中心端点
print("\n=== [8] 验证信任中心 4 端点 ===")
r = ev("(async ()=>{"
       "const endpoints = ['/v1/audit-trail', '/v1/trust/data-location', '/v1/trust/network-audit', '/v1/trust/audit-integrity'];"
       "const results = {};"
       "for (const ep of endpoints) {"
       "  try {"
       "    const r = await fetch('http://127.0.0.1:3099' + ep);"
       "    const j = await r.json();"
       "    results[ep] = {status: r.status, body: j};"
       "  } catch(e) { results[ep] = {error: e.message}; }"
       "}"
       "return results;"
       "})()", await_p=True, timeout_ms=30000)
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

ws.close()
print("\n[OK] sidecar 重启 + 验证完成")
