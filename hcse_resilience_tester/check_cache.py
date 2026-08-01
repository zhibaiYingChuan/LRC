#!/usr/bin/env python3
"""检查页面加载的 app.js 是否包含最新修复"""
import json, requests, websocket

CDP_URL = 'http://127.0.0.1:9223'
resp = requests.get(f'{CDP_URL}/json', timeout=5)
targets = resp.json()
target = None
for t in targets:
    if t.get('type') == 'page':
        target = t
        break

ws = websocket.create_connection(target['webSocketDebuggerUrl'], suppress_origin=True)

# 检查 _abortActiveTabRequests 和 window.onerror 的源码
js_code = """
JSON.stringify({
    abortFnSource: _abortActiveTabRequests.toString().substring(0, 800),
    hasWindowSync: _abortActiveTabRequests.toString().indexOf('window.daoAbortController = null') !== -1,
    onerrorType: typeof window.onerror,
    onerrorIsNull: window.onerror === null,
    onerrorSource: typeof window.onerror === 'function' ? window.onerror.toString().substring(0, 300) : 'not a function',
    registered: window._lrcGlobalErrorRegistered
})
"""

ws.send(json.dumps({
    'id': 1,
    'method': 'Runtime.evaluate',
    'params': {
        'expression': js_code,
        'returnByValue': True
    }
}))

result = json.loads(ws.recv())
value = result['result']['result']['value']
data = json.loads(value)

print("=== _abortActiveTabRequests 源码 ===")
print(data['abortFnSource'])
print()
print(f"hasWindowSync (包含 window.daoAbortController = null): {data['hasWindowSync']}")
print()
print(f"onerrorType: {data['onerrorType']}")
print(f"onerrorIsNull: {data['onerrorIsNull']}")
print(f"onerrorSource: {data['onerrorSource']}")
print(f"registered: {data['registered']}")

ws.close()
