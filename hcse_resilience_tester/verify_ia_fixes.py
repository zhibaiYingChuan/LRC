#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
v0.8.22 IA-01 + IA-02 修复验证脚本
通过 CDP WebSocket 直连 Tauri WebView2 (端口 9223) 验证修复
"""
import json
import time
import requests
import websocket
from threading import Thread

CDP_PORT = 9223
CDP_URL = f"http://127.0.0.1:{CDP_PORT}"

def get_page_target():
    """获取 Tauri 页面的 WebSocket URL"""
    resp = requests.get(f"{CDP_URL}/json", timeout=5)
    targets = resp.json()
    for t in targets:
        if t.get("type") == "page" and "tauri.localhost" in t.get("url", ""):
            return t
    # 如果没有 tauri.localhost，返回第一个 page
    for t in targets:
        if t.get("type") == "page":
            return t
    raise RuntimeError("No page target found")

def eval_js(ws, expression, msg_id=1):
    """通过 CDP Runtime.evaluate 执行 JavaScript"""
    ws.send(json.dumps({
        "id": msg_id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "returnByValue": True,
            "awaitPromise": True,
            "timeout": 10000
        }
    }))
    # 等待响应
    while True:
        result = json.loads(ws.recv())
        if result.get("id") == msg_id:
            return result

def main():
    print("=" * 60)
    print("v0.8.22 IA-01 + IA-02 修复验证（CDP 桌面端）")
    print("=" * 60)

    # Step 1: 获取页面 target
    target = get_page_target()
    ws_url = target.get("webSocketDebuggerUrl")
    print(f"\n[1] Page target: {target.get('title', '(no title)')}")
    print(f"    URL: {target.get('url')}")
    print(f"    WebSocket: {ws_url}")

    if not ws_url:
        print("ERROR: No webSocketDebuggerUrl found")
        return

    # Step 2: 连接 WebSocket
    ws = websocket.create_connection(ws_url, suppress_origin=True)
    print(f"\n[2] WebSocket connected")

    msg_id = 0

    # Step 3: 验证页面基本状态
    msg_id += 1
    result = eval_js(ws, """
        JSON.stringify({
            readyState: document.readyState,
            title: document.title,
            appVersion: typeof APP_VERSION !== 'undefined' ? APP_VERSION : 'undefined',
            hasJquery: typeof $ !== 'undefined',
            hasShowToast: typeof window.showToast === 'function',
            hasLoadDaoMetrics: typeof loadDaoMetrics === 'function',
            hasSidecarHealthMonitor: typeof window.sidecarHealthMonitor !== 'undefined',
            hasAbortActiveTabRequests: typeof window._abortActiveTabRequests === 'function',
            daoAbortControllerType: typeof window.daoAbortController,
            hasGlobalErrorRegistered: window._lrcGlobalErrorRegistered,
            onerrorType: typeof window.onerror,
            onerrorIsFunction: typeof window.onerror === 'function',
            onunhandledType: typeof window.onunhandledrejection,
            onunhandledIsFunction: typeof window.onunhandledrejection === 'function'
        })
    """, msg_id)
    basic = json.loads(result["result"]["result"]["value"])
    print(f"\n[3] 页面基本状态:")
    for k, v in basic.items():
        print(f"    {k}: {v}")

    # Step 4: IA-01 验证
    print(f"\n[4] IA-01 验证: daoAbortController 同步到 window")
    msg_id += 1
    result = eval_js(ws, """
        (function() {
            var results = {};
            // 检查初始状态
            results.before = {
                exists: typeof window.daoAbortController !== 'undefined',
                type: typeof window.daoAbortController,
                isNull: window.daoAbortController === null
            };
            // 调用 loadDaoMetrics 创建 AbortController
            if (typeof loadDaoMetrics === 'function') {
                loadDaoMetrics();
            }
            // 检查创建后
            results.afterCreate = {
                exists: typeof window.daoAbortController !== 'undefined',
                type: typeof window.daoAbortController,
                hasAbort: window.daoAbortController && typeof window.daoAbortController.abort === 'function',
                signalAborted: window.daoAbortController && window.daoAbortController.signal && window.daoAbortController.signal.aborted
            };
            // 调用 _abortActiveTabRequests('memory-search') 切换离开 dashboard
            if (typeof _abortActiveTabRequests === 'function') {
                _abortActiveTabRequests('memory-search');
            }
            // 检查 abort 后
            results.afterAbort = {
                exists: typeof window.daoAbortController !== 'undefined',
                isNull: window.daoAbortController === null,
                type: typeof window.daoAbortController
            };
            results.fixVerified = results.afterAbort.isNull === true;
            return JSON.stringify(results);
        })()
    """, msg_id)
    ia01 = json.loads(result["result"]["result"]["value"])
    print(f"    before: {ia01['before']}")
    print(f"    afterCreate: {ia01['afterCreate']}")
    print(f"    afterAbort: {ia01['afterAbort']}")
    print(f"    IA-01 修复验证: {'PASS' if ia01['fixVerified'] else 'FAIL'}")

    # Step 5: IA-02 验证
    print(f"\n[5] IA-02 验证: window.onerror 属性注册")
    msg_id += 1
    result = eval_js(ws, """
        (function() {
            var results = {};
            // 检查 window.onerror 和 window.onunhandledrejection
            results.onerrorType = typeof window.onerror;
            results.onerrorIsFunction = typeof window.onerror === 'function';
            results.onunhandledType = typeof window.onunhandledrejection;
            results.onunhandledIsFunction = typeof window.onunhandledrejection === 'function';
            results.registered = window._lrcGlobalErrorRegistered;

            // 注入错误，检查 toast
            results.toastCountBefore = document.querySelectorAll('.toast').length;

            // 触发 unhandledrejection
            Promise.reject(new Error('CDP test: IA-02 verification'));

            return JSON.stringify(results);
        })()
    """, msg_id)
    ia02_before = json.loads(result["result"]["result"]["value"])
    print(f"    onerrorType: {ia02_before['onerrorType']}")
    print(f"    onerrorIsFunction: {ia02_before['onerrorIsFunction']}")
    print(f"    onunhandledType: {ia02_before['onunhandledType']}")
    print(f"    onunhandledIsFunction: {ia02_before['onunhandledIsFunction']}")
    print(f"    registered: {ia02_before['registered']}")
    print(f"    toastCountBefore: {ia02_before['toastCountBefore']}")

    # 等待 1s 让 toast 出现
    time.sleep(1)

    # 检查 toast 数量
    msg_id += 1
    result = eval_js(ws, """
        JSON.stringify({
            toastCountAfter: document.querySelectorAll('.toast').length,
            toastText: Array.from(document.querySelectorAll('.toast')).map(function(t) { return t.textContent; }),
            toastIncreased: document.querySelectorAll('.toast').length > """ + str(ia02_before['toastCountBefore']) + """
        })
    """, msg_id)
    ia02_after = json.loads(result["result"]["result"]["value"])
    print(f"    toastCountAfter: {ia02_after['toastCountAfter']}")
    print(f"    toastText: {ia02_after['toastText']}")
    print(f"    toastIncreased: {ia02_after['toastIncreased']}")

    ia02_pass = ia02_before['onerrorIsFunction'] and ia02_before['onunhandledIsFunction']
    print(f"    IA-02 修复验证: {'PASS' if ia02_pass else 'FAIL'}")

    # Step 6: 总结
    print(f"\n{'=' * 60}")
    print(f"验证总结:")
    print(f"  IA-01 (daoAbortController 同步): {'PASS' if ia01['fixVerified'] else 'FAIL'}")
    print(f"  IA-02 (window.onerror 属性注册): {'PASS' if ia02_pass else 'FAIL'}")
    print(f"{'=' * 60}")

    ws.close()

if __name__ == "__main__":
    main()
