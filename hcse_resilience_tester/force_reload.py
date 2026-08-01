#!/usr/bin/env python3
"""通过 CDP 强制重新加载页面（忽略缓存）"""
import json, requests, websocket, time

CDP_URL = 'http://127.0.0.1:9223'
resp = requests.get(f'{CDP_URL}/json', timeout=5)
targets = resp.json()
target = None
for t in targets:
    if t.get('type') == 'page':
        target = t
        break

ws = websocket.create_connection(target['webSocketDebuggerUrl'], suppress_origin=True)

# 启用 Page domain
ws.send(json.dumps({'id': 1, 'method': 'Page.enable'}))
ws.recv()

# 强制重新加载（ignoreCache=true）
ws.send(json.dumps({
    'id': 2,
    'method': 'Page.reload',
    'params': {
        'ignoreCache': True
    }
}))

# 等待重新加载完成
time.sleep(5)

# 检查加载后的状态
ws.send(json.dumps({
    'id': 3,
    'method': 'Runtime.evaluate',
    'params': {
        'expression': """
        JSON.stringify({
            readyState: document.readyState,
            title: document.title,
            hasAbortActiveTabRequests: typeof window._abortActiveTabRequests === 'function',
            hasWindowSync: typeof window._abortActiveTabRequests === 'function' && _abortActiveTabRequests.toString().indexOf('window.daoAbortController = null') !== -1,
            onerrorType: typeof window.onerror,
            onerrorIsNull: window.onerror === null,
            onerrorIsFunction: typeof window.onerror === 'function'
        })
        """,
        'returnByValue': True
    }
}))

# 读取所有响应直到找到 id=3
for _ in range(20):
    result = json.loads(ws.recv())
    if result.get('id') == 3:
        data = json.loads(result['result']['result']['value'])
        print("=== 强制刷新后的页面状态 ===")
        for k, v in data.items():
            print(f"  {k}: {v}")
        break

ws.close()
