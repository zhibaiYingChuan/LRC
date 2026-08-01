"""
快速复检 L2-1 / L4-2 / L5-4 的真实状态
"""
import websocket, json, time

WS_URL = "ws://127.0.0.1:9223/devtools/page/F33530DE6985D0FD12B73120964107AA"

def main():
    ws = websocket.create_connection(WS_URL, timeout=10, suppress_origin=True)
    msg_id = [0]

    def send(method, params=None):
        msg_id[0] += 1
        ws.send(json.dumps({"id": msg_id[0], "method": method, "params": params or {}}))
        deadline = time.time() + 10
        while time.time() < deadline:
            raw = ws.recv()
            data = json.loads(raw)
            if data.get("id") == msg_id[0]:
                return data
        raise TimeoutError(method)

    # 1. 检查 SidecarHealthMonitor 真实挂载情况
    expr1 = """
    (function(){
        var result = {
            hasWindowSidecarHealthMonitor: typeof window.sidecarHealthMonitor !== 'undefined',
            hasSidecarHealthMonitor: typeof SidecarHealthMonitor !== 'undefined',
            sidecarHealthMonitorType: typeof SidecarHealthMonitor,
            isSidecarHealthMonitorObject: typeof SidecarHealthMonitor === 'object',
            isSidecarHealthMonitorFunction: typeof SidecarHealthMonitor === 'function',
            hasStart: typeof SidecarHealthMonitor !== 'undefined' && typeof SidecarHealthMonitor.start === 'function',
            hasCheck: typeof SidecarHealthMonitor !== 'undefined' && typeof SidecarHealthMonitor.check === 'function',
            sidecarStatus: (typeof SidecarHealthMonitor !== 'undefined') ? SidecarHealthMonitor._sidecarStatus : 'N/A',
            isReachable: (typeof SidecarHealthMonitor !== 'undefined') ? SidecarHealthMonitor._isReachable : 'N/A',
            lockBusy: (typeof SidecarHealthMonitor !== 'undefined') ? SidecarHealthMonitor._lockBusy : 'N/A',
        };
        return result;
    })()
    """
    r1 = send("Runtime.evaluate", {"expression": expr1, "returnByValue": True})
    print("[1] SidecarHealthMonitor 状态:")
    print(json.dumps(r1.get("result", {}).get("result", {}).get("value"), indent=2, ensure_ascii=False))

    # 2. 检查 banner 和 handleStartServiceClick 真实情况
    expr2 = """
    (function(){
        var banner = document.getElementById('sidecar-down-banner');
        var bannerBtn = document.querySelector('#sidecar-down-banner button[data-action="handleStartServiceClick"]');
        var modalBtn = document.getElementById('modal-btn-start-service');
        var isDesktopEmbedded = (typeof IS_DESKTOP_EMBEDDED !== 'undefined') ? IS_DESKTOP_EMBEDDED : 'undef';
        return {
            bannerExists: !!banner,
            bannerHidden: banner ? banner.hidden : 'N/A',
            bannerBtnExists: !!bannerBtn,
            bannerBtnText: bannerBtn ? bannerBtn.textContent : 'N/A',
            modalBtnExists: !!modalBtn,
            isDesktopEmbedded: isDesktopEmbedded,
            hasHandleStartServiceClick: typeof handleStartServiceClick === 'function',
            startServiceInProgress: (typeof _startServiceInProgress !== 'undefined') ? _startServiceInProgress : 'undef',
        };
    })()
    """
    r2 = send("Runtime.evaluate", {"expression": expr2, "returnByValue": True})
    print("\n[2] Banner 和启动服务状态:")
    print(json.dumps(r2.get("result", {}).get("result", {}).get("value"), indent=2, ensure_ascii=False))

    # 3. 检查"下一步"按钮 loading 状态
    expr3 = """
    (function(){
        var loadingBtns = document.querySelectorAll('button.is-loading, button[disabled], button.loading');
        var nextBtns = document.querySelectorAll('button');
        var nextBtnInfo = [];
        nextBtns.forEach(function(b){
            if (b.textContent && b.textContent.indexOf('下一步') >= 0) {
                nextBtnInfo.push({
                    text: b.textContent.trim().substring(0, 50),
                    disabled: b.disabled,
                    class: b.className,
                    parentModal: b.closest('.modal') ? b.closest('.modal').id : 'no-modal',
                    isInModal: !!b.closest('.modal'),
                });
            }
        });
        return {
            loadingBtnCount: loadingBtns.length,
            loadingBtns: Array.from(loadingBtns).map(function(b){return {text:b.textContent.trim().substring(0,50),class:b.className};}),
            nextBtnCount: nextBtnInfo.length,
            nextBtns: nextBtnInfo,
            openModals: Array.from(document.querySelectorAll('.modal')).filter(function(m){
                return m.style.display !== 'none' && getComputedStyle(m).display !== 'none';
            }).map(function(m){return {id:m.id, class:m.className.substring(0,80)};}),
        };
    })()
    """
    r3 = send("Runtime.evaluate", {"expression": expr3, "returnByValue": True})
    print("\n[3] 按钮和 modal 状态:")
    print(json.dumps(r3.get("result", {}).get("result", {}).get("value"), indent=2, ensure_ascii=False))

    # 4. 检查全局错误处理
    expr4 = """
    (function(){
        return {
            hasWindowOnError: !!window.onerror,
            hasWindowOnUnhandledRejection: !!window.onunhandledrejection,
            onErrorType: typeof window.onerror,
            onRejectionType: typeof window.onunhandledrejection,
            hasErrorEventListener: false,
            hasRejectionEventListener: false,
        };
    })()
    """
    r4 = send("Runtime.evaluate", {"expression": expr4, "returnByValue": True})
    print("\n[4] 全局错误处理:")
    print(json.dumps(r4.get("result", {}).get("result", {}).get("value"), indent=2, ensure_ascii=False))

    # 5. 检查 dao_metrics 真实端点
    expr5 = """
    (function(){
        var daoEl = document.querySelector('#dao-metrics, [data-component="dao-metrics"], .dao-metrics, #dao-metrics-card');
        return {
            daoElementExists: !!daoEl,
            daoElementId: daoEl ? daoEl.id : 'N/A',
            daoElementClass: daoEl ? daoEl.className.substring(0, 100) : 'N/A',
            daoText: daoEl ? daoEl.textContent.trim().substring(0, 300) : 'N/A',
            // 检查页面是否有"道同构度"相关文案
            bodyHasDaoText: document.body.innerText.includes('道同构度') || document.body.innerText.includes('同构'),
            bodyDaoSnippet: (function(){
                var text = document.body.innerText;
                var idx = text.indexOf('道同构度');
                if (idx < 0) idx = text.indexOf('同构');
                return idx >= 0 ? text.substring(idx, idx+200) : 'N/A';
            })(),
        };
    })()
    """
    r5 = send("Runtime.evaluate", {"expression": expr5, "returnByValue": True})
    print("\n[5] 道同构度卡片状态:")
    print(json.dumps(r5.get("result", {}).get("result", {}).get("value"), indent=2, ensure_ascii=False))

    ws.close()

if __name__ == "__main__":
    main()
