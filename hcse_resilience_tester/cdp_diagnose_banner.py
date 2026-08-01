"""
诊断脚本：banner 显示逻辑 + 通过 UI 触发 sidecar 重启
"""
import json
import os
import time

# 禁用代理
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

ws.send(json.dumps({"id": 999, "method": "Runtime.enable"}))
time.sleep(0.5)
try:
    while True:
        ws.settimeout(0.3)
        ws.recv()
except Exception:
    pass
ws.settimeout(20)

print("=== banner 显示逻辑诊断 ===")
# 1. 检查 banner 元素完整状态
r = ev("(()=>{const b = document.getElementById('sidecar-down-banner');"
       "if (!b) return {exists: false};"
       "const cs = getComputedStyle(b);"
       "const m = window.sidecarHealthMonitor;"
       "return {"
       "exists: true, "
       "tagName: b.tagName, "
       "className: b.className, "
       "inlineDisplay: b.style.display, "
       "computedDisplay: cs.display, "
       "computedVisibility: cs.visibility, "
       "computedOpacity: cs.opacity, "
       "computedZIndex: cs.zIndex, "
       "text: b.textContent.trim().substring(0, 100), "
       "parentExists: !!b.parentElement, "
       "parentDisplay: b.parentElement ? getComputedStyle(b.parentElement).display : null, "
       "monitorReachable: m._isReachable, "
       "monitorStatus: m._sidecarStatus, "
       "monitorIsRunning: m.isRunning, "
       "monitorPollTimer: m._pollTimer"
       "};})()")
val = r.get("result", {}).get("result", {}).get("value")
print(f"banner 完整状态: {json.dumps(val, ensure_ascii=False, indent=2)}")

# 2. 检查 _setReachable 逻辑
r = ev("(()=>{const m = window.sidecarHealthMonitor; return {"
       "hasSetReachable: typeof m._setReachable === 'function', "
       "hasShowBanner: typeof m._showBanner === 'function', "
       "hasHideBanner: typeof m._hideBanner === 'function', "
       "hasUpdateBanner: typeof m._updateBanner === 'function', "
       "methodList: Object.getOwnPropertyNames(Object.getPrototypeOf(m)).filter(n => n.startsWith('_') || n.startsWith('show') || n.startsWith('hide') || n.startsWith('update'))"
       "};})()")
val = r.get("result", {}).get("result", {}).get("value")
print(f"\nmonitor banner 方法: {json.dumps(val, ensure_ascii=False, indent=2)}")

# 3. 手动调用 _setReachable(false) 看 banner 是否显示
print("\n=== 手动调用 _setReachable(false) ===")
r = ev("(async ()=>{const m = window.sidecarHealthMonitor;"
       "const before = m._isReachable;"
       "if (typeof m._setReachable === 'function') { m._setReachable(false); }"
       "await new Promise(r => setTimeout(r, 500));"
       "const b = document.getElementById('sidecar-down-banner');"
       "const after = {"
       "  monitorReachable: m._isReachable, "
       "  bannerVisible: b ? getComputedStyle(b).display !== 'none' : null, "
       "  bannerDisplay: b ? getComputedStyle(b).display : null"
       "};"
       "return {before, after};})()", await_p=True)
val = r.get("result", {}).get("result", {}).get("value")
print(f"手动 _setReachable(false) 结果: {json.dumps(val, ensure_ascii=False, indent=2)}")

# 4. 检查 banner CSS 规则（是否有 !important 隐藏）
print("\n=== banner CSS 规则检查 ===")
r = ev("(()=>{const b = document.getElementById('sidecar-down-banner');"
       "if (!b) return {exists: false};"
       "const sheets = Array.from(document.styleSheets);"
       "const rules = [];"
       "try {"
       "  for (const sheet of sheets) {"
       "    try {"
       "      for (const rule of sheet.cssRules) {"
       "        if (rule.selectorText && (rule.selectorText.includes('sidecar-down-banner') || rule.selectorText.includes('sidecar-down'))) {"
       "          rules.push({selector: rule.selectorText, display: rule.style.display, cssText: rule.cssText.substring(0, 200)});"
       "        }"
       "      }"
       "    } catch(e) {}"
       "  }"
       "} catch(e) {}"
       "return {ruleCount: rules.length, rules: rules};})()")
val = r.get("result", {}).get("result", {}).get("value")
print(f"banner CSS 规则: {json.dumps(val, ensure_ascii=False, indent=2)}")

# 5. 检查 lrc-desktop 是否还活着（IPC）
print("\n=== lrc-desktop IPC 可用性 ===")
r = ev("(()=>{return {"
       "hasTauriInternals: typeof window.__TAURI_INTERNALS__ !== 'undefined', "
       "hasTauriCore: typeof window.__TAURI_CORE__ !== 'undefined', "
       "hasInvoke: typeof window.__TAURI_INVOKE__ !== 'undefined' || (window.__TAURI__ && typeof window.__TAURI__.invoke === 'function')"
       "};})()")
val = r.get("result", {}).get("result", {}).get("value")
print(f"IPC 可用性: {json.dumps(val, ensure_ascii=False, indent=2)}")

# 6. 检查所有 console 错误
print("\n=== 检查 console 错误（注入监听器）===")
r = ev("(()=>{"
       "if (!window._lrcConsoleErrors) {"
       "  window._lrcConsoleErrors = [];"
       "  const origError = console.error;"
       "  console.error = function() { window._lrcConsoleErrors.push(Array.from(arguments).map(a => String(a).substring(0, 200)).join(' ')); origError.apply(console, arguments); };"
       "}"
       "return {existingErrors: window._lrcConsoleErrors.slice(-10)};"
       "})()")
val = r.get("result", {}).get("result", {}).get("value")
print(f"最近 console 错误: {json.dumps(val, ensure_ascii=False, indent=2)}")

ws.close()
print("\n[OK] 诊断完成")
