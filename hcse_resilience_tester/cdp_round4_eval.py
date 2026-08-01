# -*- coding: utf-8 -*-
"""
CDP Round4 通用探测脚本 — 直连 Tauri WebView2 (9223)
用法:
  echo "(() => { return {...} })()" | python cdp_round4_eval.py
  python cdp_round4_eval.py --file probe.js
  python cdp_round4_eval.py --screenshot out.png
"""
import sys, json, time, base64, argparse, urllib.request
import websocket

CDP_HTTP = "http://127.0.0.1:9223"

def get_ws_url():
    try:
        resp = urllib.request.urlopen(CDP_HTTP + "/json/list", timeout=5)
        pages = json.loads(resp.read())
    except Exception as e:
        return None, f"获取页面列表失败: {e}"
    # 优先 tauri.localhost
    for p in pages:
        if "tauri" in p.get("url", "") or "tauri" in p.get("title", "").lower() or "loong" in p.get("title", "").lower():
            return p["webSocketDebuggerUrl"], None
    if pages:
        return pages[0]["webSocketDebuggerUrl"], None
    return None, "无可用页面"

def cdp_call(ws_url, method, params=None, timeout=20):
    ws = websocket.create_connection(ws_url, timeout=timeout, suppress_origin=True)
    msg = {"id": 1, "method": method, "params": params or {}}
    ws.send(json.dumps(msg))
    raw = ws.recv()
    ws.close()
    return json.loads(raw)

def cdp_eval(ws_url, js, await_promise=True, timeout=20):
    resp = cdp_call(ws_url, "Runtime.evaluate", {
        "expression": js,
        "returnByValue": True,
        "awaitPromise": await_promise,
        "userGesture": True,
    }, timeout)
    if "error" in resp:
        return {"_cdp_error": resp["error"]}
    res = resp.get("result", {}).get("result", {})
    exc = resp.get("result", {}).get("exceptionDetails")
    if exc:
        return {"_eval_exception": exc.get("exception", {}).get("description", str(exc))}
    if res.get("type") in ("undefined",) or res.get("subtype") == "null":
        return None
    return res.get("value")

def cdp_screenshot(ws_url, path, full_page=True):
    resp = cdp_call(ws_url, "Page.captureScreenshot", {
        "format": "png",
        "captureBeyondViewport": full_page,
    })
    data = resp.get("result", {}).get("data")
    if data:
        with open(path, "wb") as f:
            f.write(base64.b64decode(data))
        return True, None
    return False, "截图失败: " + json.dumps(resp.get("error", resp))[:200]

if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--file", help="JS 文件路径")
    ap.add_argument("--screenshot", help="截图保存路径")
    ap.add_argument("--no-await", action="store_true")
    args = ap.parse_args()

    ws_url, err = get_ws_url()
    print("WS_URL:", ws_url or err, file=sys.stderr)

    if args.screenshot:
        ok, e = cdp_screenshot(ws_url, args.screenshot)
        print(json.dumps({"screenshot": ok, "error": e}, ensure_ascii=False))
        sys.exit(0)

    if args.file:
        with open(args.file, "r", encoding="utf-8") as f:
            js = f.read()
    else:
        js = sys.stdin.read()

    if not js.strip():
        print(json.dumps({"error": "无 JS 输入"}, ensure_ascii=False))
        sys.exit(1)

    result = cdp_eval(ws_url, js, await_promise=not args.no_await)
    print(json.dumps(result, ensure_ascii=False, indent=2))
