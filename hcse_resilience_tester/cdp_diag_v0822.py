"""快速 CDP 诊断脚本 — 验证 IA-03 真实状态"""
import json
import time
import requests
import websocket

CDP = "http://127.0.0.1:9223"

# 获取 target
targets = requests.get(f"{CDP}/json", timeout=5).json()
target = next(t for t in targets if "tauri.localhost" in t.get("url", ""))
ws_url = target["webSocketDebuggerUrl"]
print(f"Target: {target['title']} | {target['url']}")
print(f"WS: {ws_url}")

ws = websocket.WebSocket()
ws.connect(ws_url, suppress_origin=True, timeout=10)

mid = [0]
def evaluate(expr):
    mid[0] += 1
    payload = {"id": mid[0], "method": "Runtime.evaluate", "params": {
        "expression": expr, "returnByValue": True, "awaitPromise": False, "timeout": 10000
    }}
    ws.send(json.dumps(payload))
    deadline = time.time() + 15
    while time.time() < deadline:
        try:
            msg = json.loads(ws.recv())
            if msg.get("id") == mid[0]:
                return msg
        except Exception:
            continue
    return {"error": "timeout"}

# 启用 Runtime
ws.send(json.dumps({"id": 999, "method": "Runtime.enable"}))
time.sleep(0.5)

# 1. window.sidecarHealthMonitor（IA-03 修复点）
r = evaluate("(()=>{const m=window.sidecarHealthMonitor;return{type:typeof m,exists:m!==undefined&&m!==null,hasCheck:m&&typeof m.check==='function',hasStart:m&&typeof m.start==='function',isReachable:m&&m._isReachable,sidecarStatus:m&&m._sidecarStatus,lockBusy:m&&m._lockBusy};})()")
print("\n[1] window.sidecarHealthMonitor (IA-03 修复点):")
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

# 2. window.SidecarHealthMonitor（原始对象）
r = evaluate("(()=>{const m=window.SidecarHealthMonitor;return{type:typeof m,exists:m!==undefined&&m!==null,hasCheck:m&&typeof m.check==='function'};})()")
print("\n[2] window.SidecarHealthMonitor (原始对象):")
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

# 3. _lrcGlobalErrorRegistered（IA-02 修复点）
r = evaluate("(()=>{return{_lrcGlobalErrorRegistered:window._lrcGlobalErrorRegistered===true,hasAddEventListener:true};})()")
print("\n[3] IA-02 全局错误处理注册标志:")
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

# 4. daoAbortController（IA-01 修复点）
r = evaluate("(()=>{return{daoAbortExists:typeof daoAbortController!=='undefined',aborted:typeof daoAbortController!=='undefined'&&daoAbortController&&daoAbortController.signal.aborted};})()")
print("\n[4] IA-01 daoAbortController:")
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

# 5. 检查 APP_VERSION
r = evaluate("(typeof APP_VERSION!=='undefined'?APP_VERSION:'undefined')")
print("\n[5] APP_VERSION:")
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

# 6. 检查 init 函数
r = evaluate("(()=>{return{initType:typeof init,hasDocListener:true,readyState:document.readyState};})()")
print("\n[6] init 函数与文档状态:")
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

# 7. 检查 sidecar-down-banner
r = evaluate("(()=>{const b=document.getElementById('sidecar-down-banner');return{exists:!!b,visible:b&&getComputedStyle(b).display!=='none',text:b&&b.textContent.trim().substring(0,80)};})()")
print("\n[7] sidecar-down-banner:")
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

# 8. 检查当前 active tab
r = evaluate("(()=>{const t=document.querySelector('.tab-button.active')||document.querySelector('[class*=active][data-tab]');return{activeTab:t?(t.dataset.tab||t.textContent.trim().substring(0,30)):null,hash:location.hash};})()")
print("\n[8] 当前 active tab:")
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

# 9. 检查 sidecar /health 实际状态
print("\n[9] sidecar /health:")
h = requests.get("http://127.0.0.1:3099/health", timeout=5).json()
print(json.dumps(h, ensure_ascii=False, indent=2))

ws.close()
