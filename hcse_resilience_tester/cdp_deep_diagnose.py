"""
深度诊断：banner.hidden 属性 + invoke 真实可用性 + sidecar 重启尝试
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

def ev(expr, await_p=False, timeout_ms=15000):
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

ws.send(json.dumps({"id": 999, "method": "Runtime.enable"}))
time.sleep(0.5)
try:
    while True:
        ws.settimeout(0.3)
        ws.recv()
except Exception:
    pass
ws.settimeout(20)

# 1. 检查 banner.hidden 属性
print("=== [1] banner.hidden 属性检查 ===")
r = ev("(()=>{const b = document.getElementById('sidecar-down-banner');"
       "const m = window.sidecarHealthMonitor;"
       "return {"
       "bannerHidden: b ? b.hidden : null, "
       "bannerHasHiddenAttr: b ? b.hasAttribute('hidden') : null, "
       "bannerStyleDisplay: b ? b.style.display : null, "
       "bannerGetAttributeHidden: b ? b.getAttribute('hidden') : null, "
       "monitorReachable: m._isReachable, "
       "monitorFailCount: m._failCount, "
       "monitorStatus: m._sidecarStatus, "
       "monitorBackoffStep: m._backoffStep"
       "};})()")
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

# 2. 强制显示 banner（手动设置 banner.hidden = false）
print("\n=== [2] 强制显示 banner（测试 UI 渲染）===")
r = ev("(async ()=>{const b = document.getElementById('sidecar-down-banner');"
       "b.hidden = false;"
       "b.style.display = 'flex';"
       "await new Promise(r => setTimeout(r, 200));"
       "const cs = getComputedStyle(b);"
       "return {hidden: b.hidden, display: b.style.display, computedDisplay: cs.display};})()", await_p=True)
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

# 3. 检查 invoke 真实可用性
print("\n=== [3] invoke 函数真实可用性 ===")
r = ev("(()=>{"
       "let invokeFn = null;"
       "if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') { invokeFn = 'internals'; }"
       "else if (window.__TAURI__ && typeof window.__TAURI__.invoke === 'function') { invokeFn = 'tauri'; }"
       "else if (window.__TAURI_CORE__) { invokeFn = 'core'; }"
       "return {"
       "invokeSource: invokeFn, "
       "hasInternals: typeof window.__TAURI_INTERNALS__, "
       "internalsKeys: window.__TAURI_INTERNALS__ ? Object.keys(window.__TAURI_INTERNALS__).slice(0, 20) : null, "
       "hasTauri: typeof window.__TAURI__, "
       "tauriKeys: window.__TAURI__ ? Object.keys(window.__TAURI__).slice(0, 20) : null"
       "};})()")
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

# 4. 尝试通过 Tauri invoke 获取 sidecar 状态
print("\n=== [4] 尝试通过 Tauri invoke 获取 sidecar 状态 ===")
r = ev("(async ()=>{"
       "try {"
       "  let invoke;"
       "  if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) invoke = window.__TAURI_INTERNALS__.invoke;"
       "  else if (window.__TAURI__ && window.__TAURI__.invoke) invoke = window.__TAURI__.invoke;"
       "  else if (window.__TAURI_INVOKE__) invoke = window.__TAURI_INVOKE__;"
       "  if (!invoke) return {hasInvoke: false};"
       "  const result = await invoke('get_sidecar_status');"
       "  return {hasInvoke: true, result: result};"
       "} catch(e) { return {hasInvoke: true, error: e.message, stack: e.stack ? e.stack.substring(0, 200) : null}; }"
       "})()", await_p=True, timeout_ms=10000)
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

# 5. 检查 dashboard 数据加载状态
print("\n=== [5] dashboard 数据加载状态 ===")
r = ev("(()=>{"
       "const cards = document.querySelectorAll('.card, .stat-card, .metric-card');"
       "const loaders = document.querySelectorAll('.loading, .spinner, .skeleton, [class*=loading]');"
       "const errorEls = document.querySelectorAll('.error, .error-message, [class*=error]');"
       "return {"
       "cardCount: cards.length, "
       "loadingCount: loaders.length, "
       "errorCount: errorEls.length, "
       "cardTexts: Array.from(cards).slice(0, 3).map(c => c.textContent.trim().substring(0, 80))"
       "};})()")
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

# 6. 恢复 banner 隐藏（避免影响后续测试）
print("\n=== [6] 恢复 banner 隐藏（清理）===")
r = ev("(()=>{const b = document.getElementById('sidecar-down-banner'); b.hidden = true; b.style.display = ''; return {hidden: b.hidden, display: b.style.display};})()")
val = r.get("result", {}).get("result", {}).get("value")
print(json.dumps(val, ensure_ascii=False, indent=2))

ws.close()
print("\n[OK] 深度诊断完成")
