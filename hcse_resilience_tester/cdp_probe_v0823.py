#!/usr/bin/env python3
"""CDP 探针 - 检查 SidecarHealthMonitor 方法名和 DOM 结构"""
import asyncio, json, http.client, websockets

async def probe():
    conn = http.client.HTTPConnection("127.0.0.1", 9222, timeout=5)
    conn.request("GET", "/json")
    data = json.loads(conn.getresponse().read().decode())
    conn.close()
    ws_url = data[0]["webSocketDebuggerUrl"]

    ws = await websockets.connect(ws_url, max_size=10*1024*1024)
    msg_id = 0
    pending = {}

    async def send(method, params=None):
        nonlocal msg_id
        msg_id += 1
        fut = asyncio.get_event_loop().create_future()
        pending[msg_id] = fut
        await ws.send(json.dumps({"id": msg_id, "method": method, "params": params or {}}))
        return await asyncio.wait_for(fut, timeout=10)

    async def recv_loop():
        async for msg in ws:
            m = json.loads(msg)
            if m.get("id") in pending:
                pending[m["id"]].set_result(m)

    asyncio.create_task(recv_loop())

    await send("Runtime.enable")
    await asyncio.sleep(0.5)

    # 1. SidecarHealthMonitor 方法
    r = await send("Runtime.evaluate", {
        "expression": "Object.getOwnPropertyNames(SidecarHealthMonitor).filter(function(k) { return typeof SidecarHealthMonitor[k] === 'function'; }).join(',')",
        "returnByValue": True
    })
    print("SidecarHealthMonitor methods:", r.get("result",{}).get("result",{}).get("value"))

    # 2. DOM - 所有 tab 和 section
    r = await send("Runtime.evaluate", {
        "expression": "Array.from(document.querySelectorAll('[data-tab], .tab, .nav-item, [id^=tab-]')).map(function(e) { return (e.textContent || '').trim() + ' [data-tab=' + (e.getAttribute('data-tab') || '') + '] [id=' + e.id + ']'; }).join(' | ')",
        "returnByValue": True
    })
    print("Tabs:", r.get("result",{}).get("result",{}).get("value"))

    # 3. 检查 handleHttpError 503 是否返回 cancel
    r = await send("Runtime.evaluate", {
        "expression": "handleHttpError.toString().includes('return { action:')",
        "returnByValue": True
    })
    print("handleHttpError has return action:", r.get("result",{}).get("result",{}).get("value"))

    # 4. 检查 502/504 分支
    r = await send("Runtime.evaluate", {
        "expression": "handleHttpError.toString().includes('502') && handleHttpError.toString().includes('504')",
        "returnByValue": True
    })
    print("handleHttpError has 502/504:", r.get("result",{}).get("result",{}).get("value"))

    # 5. 检查 _retryCounters
    r = await send("Runtime.evaluate", {
        "expression": "typeof _retryCounters !== 'undefined' ? 'size=' + _retryCounters.size : 'not found (IIFE)'; typeof window._retryCounters !== 'undefined' ? 'window size=' + window._retryCounters.size : 'window not found'",
        "returnByValue": True
    })
    print("Retry counters:", r.get("result",{}).get("result",{}).get("value"))

    # 6. 检查 SidecarHealthMonitor._updateStatus 是否存在
    r = await send("Runtime.evaluate", {
        "expression": "typeof SidecarHealthMonitor._updateStatus",
        "returnByValue": True
    })
    print("SidecarHealthMonitor._updateStatus:", r.get("result",{}).get("result",{}).get("value"))

    # 7. 检查 SidecarHealthMonitor._setReachable 是否存在
    r = await send("Runtime.evaluate", {
        "expression": "typeof SidecarHealthMonitor._setReachable",
        "returnByValue": True
    })
    print("SidecarHealthMonitor._setReachable:", r.get("result",{}).get("result",{}).get("value"))

    # 8. 检查 SidecarHealthMonitor 的 updateStatus（可能没有下划线）
    r = await send("Runtime.evaluate", {
        "expression": "typeof SidecarHealthMonitor.updateStatus",
        "returnByValue": True
    })
    print("SidecarHealthMonitor.updateStatus:", r.get("result",{}).get("result",{}).get("value"))

    await ws.close()

asyncio.run(probe())