"""验证桌面端实际加载的 app.js 是否包含 IA-01 修复代码"""
import json
import time
import requests
import websocket

CDP = "http://127.0.0.1:9223"
targets = requests.get(f"{CDP}/json", timeout=5).json()
target = next(t for t in targets if "tauri.localhost" in t.get("url", ""))
ws_url = target["webSocketDebuggerUrl"]
ws = websocket.WebSocket()
ws.connect(ws_url, suppress_origin=True, timeout=10)

mid = [0]
def evaluate(expr, await_promise=False):
    mid[0] += 1
    payload = {"id": mid[0], "method": "Runtime.evaluate", "params": {
        "expression": expr, "returnByValue": True, "awaitPromise": await_promise, "timeout": 15000
    }}
    ws.send(json.dumps(payload))
    deadline = time.time() + 20
    while time.time() < deadline:
        try:
            msg = json.loads(ws.recv())
            if msg.get("id") == mid[0]:
                return msg
        except Exception:
            continue
    return {"error": "timeout"}

ws.send(json.dumps({"id": 999, "method": "Runtime.enable"}))
time.sleep(0.5)

# 1. 通过 fetch 获取 app.js 源码搜索 IA-01 修复
print("[1] 通过 fetch 获取桌面端实际加载的 app.js，搜索 IA-01 修复代码...")
r = evaluate("""
(async()=>{
  try {
    const r = await fetch('/app.js?v=' + Date.now());
    const txt = await r.text();
    return {
      length: txt.length,
      has_daoAbortController_let: txt.includes('let daoAbortController'),
      has_IA01_comment: txt.includes('IA-01'),
      has_IA02_comment: txt.includes('IA-02'),
      has_IA03_comment: txt.includes('IA-03'),
      has_window_sidecarHealthMonitor: txt.includes('window.sidecarHealthMonitor'),
      has_window_addEventListener_error: txt.includes("window.addEventListener('error'"),
      has_daoAbort_in_switch: txt.includes('切换离开 dashboard'),
      // 找到 daoAbortController 第一次出现的位置上下文
      daoAbort_first_occurrence: txt.indexOf('daoAbortController') >= 0 ? txt.substring(Math.max(0, txt.indexOf('daoAbortController')-100), txt.indexOf('daoAbortController')+200) : null,
      // 找到 IA-01 注释位置
      IA01_comment_pos: txt.indexOf('v0.8.22 IA-01'),
      IA01_context: txt.indexOf('v0.8.22 IA-01') >= 0 ? txt.substring(txt.indexOf('v0.8.22 IA-01'), txt.indexOf('v0.8.22 IA-01')+500) : null
    };
  } catch(e) { return {error: String(e)}; }
})()
""", await_promise=True)
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

# 2. 直接尝试访问 daoAbortController（不用 typeof）
print("\n[2] 直接访问 daoAbortController（不同方式）:")
r = evaluate("""
(()=>{
  const results = {};
  // 方式1: typeof
  results.typeof_daoAbortController = typeof daoAbortController;
  // 方式2: 直接访问（可能抛 ReferenceError）
  try { results.direct_access = String(daoAbortController); } catch(e) { results.direct_access_err = String(e); }
  // 方式3: window.daoAbortController
  results.window_daoAbortController = typeof window.daoAbortController;
  // 方式4: globalThis
  results.globalThis_daoAbortController = typeof globalThis.daoAbortController;
  // 方式5: 检查 loadDaoMetrics 函数源码
  try {
    results.loadDaoMetrics_source = (typeof loadDaoMetrics==='function') ? loadDaoMetrics.toString().substring(0, 500) : 'not a function';
  } catch(e) { results.loadDaoMetrics_err = String(e); }
  return results;
})()
""")
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

# 3. 检查 _tabAbortControllers
print("\n[3] _tabAbortControllers 与 switchTab 函数源码:")
r = evaluate("""
(()=>{
  const results = {};
  results.typeof_tabAbortControllers = typeof _tabAbortControllers;
  try { results.window_tabAbortControllers = typeof window._tabAbortControllers; } catch(e) {}
  try { results.window_abortActiveTabRequests = typeof window._abortActiveTabRequests; } catch(e) {}
  // switchTab 源码（前 800 字符）
  try {
    if (typeof switchTab === 'function') {
      results.switchTab_source = switchTab.toString().substring(0, 800);
    } else if (typeof window.switchTab === 'function') {
      results.switchTab_source = window.switchTab.toString().substring(0, 800);
    } else {
      results.switchTab_status = 'not found';
    }
  } catch(e) { results.switchTab_err = String(e); }
  return results;
})()
""")
print(json.dumps(r.get("result",{}).get("result",{}).get("value"), ensure_ascii=False, indent=2))

ws.close()
