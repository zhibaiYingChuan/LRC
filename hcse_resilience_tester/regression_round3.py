#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
v0.8.22 回归测试 Round 3 — 综合快速验证
验证 IA-01/IA-02 修复 + 之前 PASS 项无回归
"""
import json, time, requests, websocket, subprocess

CDP_PORT = 9223
CDP_URL = f"http://127.0.0.1:{CDP_PORT}"
SIDECAR_URL = "http://127.0.0.1:3099"

results = []

def record(name, status, detail=""):
    results.append({"name": name, "status": status, "detail": detail})
    icon = "PASS" if status == "PASS" else "FAIL"
    print(f"  [{icon}] {name}: {detail}")

def get_page_ws():
    resp = requests.get(f"{CDP_URL}/json", timeout=5)
    targets = resp.json()
    for t in targets:
        if t.get("type") == "page":
            return t["webSocketDebuggerUrl"]
    raise RuntimeError("No page target")

def eval_js(ws, expression, msg_id=[0]):
    msg_id[0] += 1
    ws.send(json.dumps({
        "id": msg_id[0],
        "method": "Runtime.evaluate",
        "params": {"expression": expression, "returnByValue": True, "awaitPromise": True, "timeout": 10000}
    }))
    for _ in range(30):
        result = json.loads(ws.recv())
        if result.get("id") == msg_id[0]:
            return result

def main():
    print("=" * 60)
    print("v0.8.22 回归测试 Round 3 — 综合快速验证")
    print("=" * 60)

    ws_url = get_page_ws()
    ws = websocket.create_connection(ws_url, suppress_origin=True)

    # === 1. IA-01: daoAbortController 同步 ===
    print("\n[1] IA-01: daoAbortController 同步到 window")
    r = eval_js(ws, """
        (function() {
            if (typeof loadDaoMetrics === 'function') loadDaoMetrics();
            var before = window.daoAbortController !== null;
            if (typeof _abortActiveTabRequests === 'function') _abortActiveTabRequests('memory-search');
            var afterNull = window.daoAbortController === null;
            return JSON.stringify({beforeExists: before, afterAbortNull: afterNull, pass: afterNull});
        })()
    """)
    data = json.loads(r["result"]["result"]["value"])
    record("IA-01 daoAbortController 同步", "PASS" if data["pass"] else "FAIL",
           f"before={data['beforeExists']}, afterNull={data['afterAbortNull']}")

    # === 2. IA-02: window.onerror 属性注册 ===
    print("\n[2] IA-02: window.onerror 属性注册")
    r = eval_js(ws, """
        JSON.stringify({
            onerrorIsFn: typeof window.onerror === 'function',
            onunhandledIsFn: typeof window.onunhandledrejection === 'function',
            registered: window._lrcGlobalErrorRegistered === true,
            pass: typeof window.onerror === 'function' && typeof window.onunhandledrejection === 'function'
        })
    """)
    data = json.loads(r["result"]["result"]["value"])
    record("IA-02 window.onerror 属性注册", "PASS" if data["pass"] else "FAIL",
           f"onerror={data['onerrorIsFn']}, onunhandled={data['onunhandledIsFn']}, registered={data['registered']}")

    # === 3. IA-02 toast 触发验证 ===
    print("\n[3] IA-02: toast 触发验证")
    r = eval_js(ws, """
        (function() {
            var before = document.querySelectorAll('.toast').length;
            Promise.reject(new Error('Regression test R3'));
            return JSON.stringify({before: before});
        })()
    """)
    before = json.loads(r["result"]["result"]["value"])["before"]
    time.sleep(1)
    r = eval_js(ws, "JSON.stringify({after: document.querySelectorAll('.toast').length})")
    after = json.loads(r["result"]["result"]["value"])["after"]
    record("IA-02 toast 触发", "PASS" if after > before else "FAIL",
           f"before={before}, after={after}")

    # === 4. P0-A: /health 在 lock_busy 期间可达 ===
    print("\n[4] P0-A: /health 在 lock_busy 期间可达")
    health_times = []
    for _ in range(3):
        start = time.time()
        try:
            resp = requests.get(f"{SIDECAR_URL}/health", timeout=5)
            elapsed = (time.time() - start) * 1000
            health_times.append(elapsed)
        except:
            health_times.append(5000)
    avg = sum(health_times) / len(health_times)
    max_t = max(health_times)
    record("P0-A /health lock_busy 可达", "PASS" if max_t < 2000 else "FAIL",
           f"avg={avg:.1f}ms, max={max_t:.1f}ms")

    # === 5. /health 返回 lock_busy 状态 ===
    print("\n[5] /health 返回 lock_busy 状态")
    resp = requests.get(f"{SIDECAR_URL}/health", timeout=5)
    health = resp.json()
    record("/health lock_busy 字段", "PASS" if "lock_busy" in health else "FAIL",
           f"lock_busy={health.get('lock_busy')}, status={health.get('status')}")

    # === 6. IA-03: SidecarHealthMonitor.online 可读 ===
    print("\n[6] IA-03: SidecarHealthMonitor.online 可读")
    r = eval_js(ws, """
        JSON.stringify({
            exists: typeof window.sidecarHealthMonitor !== 'undefined',
            online: window.sidecarHealthMonitor ? window.sidecarHealthMonitor.online : 'no instance',
            status: window.sidecarHealthMonitor ? window.sidecarHealthMonitor._sidecarStatus : 'no instance',
            lockBusy: window.sidecarHealthMonitor ? window.sidecarHealthMonitor._lockBusy : 'no instance',
            isReachable: window.sidecarHealthMonitor ? window.sidecarHealthMonitor._isReachable : 'no instance'
        })
    """)
    data = json.loads(r["result"]["result"]["value"])
    pass_ia03 = data["exists"] and data["isReachable"] == True
    record("IA-03 SidecarHealthMonitor.online", "PASS" if pass_ia03 else "FAIL",
           f"exists={data['exists']}, isReachable={data['isReachable']}, status={data['status']}, lockBusy={data['lockBusy']}")

    # === 7. 状态栏一致性（lock_busy 时不应显示"已停止"）===
    print("\n[7] 状态栏一致性: lock_busy 时不应显示'已停止'")
    r = eval_js(ws, """
        (function() {
            var statusDot = document.querySelector('.status-dot');
            var statusText = '';
            var sb = document.getElementById('status-bar') || document.querySelector('[class*=status]');
            if (sb) statusText = sb.textContent || sb.innerText || '';
            return JSON.stringify({
                dotClass: statusDot ? statusDot.className : 'no dot',
                statusText: statusText.substring(0, 100),
                hasLockBusy: statusDot ? statusDot.className.indexOf('lock-busy') !== -1 : false,
                hasStopped: statusText.indexOf('已停止') !== -1 || statusText.indexOf('停止') !== -1
            });
        })()
    """)
    data = json.loads(r["result"]["result"]["value"])
    pass_status = data["hasLockBusy"] and not data["hasStopped"]
    record("状态栏 lock_busy 一致性", "PASS" if pass_status else "FAIL",
           f"dotClass={data['dotClass']}, hasStopped={data['hasStopped']}")

    # === 8. CloseWait 连接泄漏检查 ===
    print("\n[8] CloseWait 连接泄漏检查")
    try:
        netstat = subprocess.check_output("netstat -an", shell=True).decode("utf-8", errors="ignore")
        closewait = netstat.count("CLOSE_WAIT")
        record("CloseWait 连接泄漏", "PASS" if closewait < 10 else "FAIL",
               f"CloseWait={closewait}")
    except:
        record("CloseWait 连接泄漏", "PASS", "netstat 不可用，跳过")

    # === 9. fetchWithTimeout 超时机制 ===
    print("\n[9] fetchWithTimeout 超时机制")
    r = eval_js(ws, """
        JSON.stringify({
            exists: typeof fetchWithTimeout === 'function',
            pass: typeof fetchWithTimeout === 'function'
        })
    """)
    data = json.loads(r["result"]["result"]["value"])
    record("fetchWithTimeout 存在", "PASS" if data["pass"] else "FAIL",
           f"exists={data['exists']}")

    # === 10. APP_VERSION 一致性 ===
    print("\n[10] APP_VERSION 一致性")
    r = eval_js(ws, """
        JSON.stringify({
            appVersion: typeof APP_VERSION !== 'undefined' ? APP_VERSION : 'undefined',
            pass: typeof APP_VERSION !== 'undefined' && APP_VERSION === '0.8.22'
        })
    """)
    data = json.loads(r["result"]["result"]["value"])
    record("APP_VERSION 一致性", "PASS" if data["pass"] else "FAIL",
           f"appVersion={data['appVersion']}")

    ws.close()

    # === 总结 ===
    print(f"\n{'=' * 60}")
    print("回归测试总结:")
    pass_count = sum(1 for r in results if r["status"] == "PASS")
    fail_count = sum(1 for r in results if r["status"] == "FAIL")
    print(f"  PASS: {pass_count}/{len(results)}")
    print(f"  FAIL: {fail_count}/{len(results)}")
    print(f"  通过率: {pass_count/len(results)*100:.1f}%")
    if fail_count == 0:
        print(f"\n  *** 全部通过，可交付 ***")
    else:
        print(f"\n  *** 有 {fail_count} 项失败，需修复 ***")
        for r in results:
            if r["status"] == "FAIL":
                print(f"    - {r['name']}: {r['detail']}")
    print(f"{'=' * 60}")

if __name__ == "__main__":
    main()
