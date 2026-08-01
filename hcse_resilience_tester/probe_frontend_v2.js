// HCSE Phase 3 RV-Monitor 前端探针 v2 — 修正变量名 + 触发 sidecar 启动验证
(async function() {
    const r = {};
    r.url = location.href;
    r.title = document.title;
    r.readyState = document.readyState;
    r.timestamp = new Date().toISOString();

    // INV-01 IPC 自定义协议不变量
    r.hasTauriInternals = !!window.__TAURI_INTERNALS__;
    r.hasTauriCore = !!(window.__TAURI__ && window.__TAURI__.core);
    r.hasInvoke = typeof (window.__TAURI_INTERNALS__?.invoke || window.__TAURI__?.core?.invoke || window.__TAURI__?.invoke) === 'function';
    r.tauriEventApi = !!(window.__TAURI__?.event?.listen || window.__TAURI__?.core?.event?.listen);

    // INV-02 启动取消机制
    r.startServiceAbortController = window.startServiceAbortController
        ? { aborted: window.startServiceAbortController.signal.aborted }
        : null;

    // INV-03 健康监控（PascalCase 修正）
    try {
        if (typeof SidecarHealthMonitor !== 'undefined') {
            r.healthMonitor = {
                type: typeof SidecarHealthMonitor,
                isObject: typeof SidecarHealthMonitor === 'object' && SidecarHealthMonitor !== null,
                pollInterval: SidecarHealthMonitor._pollInterval,
                maxBackoff: SidecarHealthMonitor._MAX_BACKOFF,
                reachable: SidecarHealthMonitor._isReachable,
                failCount: SidecarHealthMonitor._failCount,
                failThreshold: SidecarHealthMonitor._FAIL_THRESHOLD,
                backoffStep: SidecarHealthMonitor._backoffStep,
                sidecarStatus: SidecarHealthMonitor._sidecarStatus,
                hasSetReachable: typeof SidecarHealthMonitor._setReachable === 'function',
                hasStart: typeof SidecarHealthMonitor.start === 'function'
            };
        } else if (typeof sidecarHealthMonitor !== 'undefined') {
            r.healthMonitor = { type: 'camelCase', pollInterval: sidecarHealthMonitor._pollInterval };
        } else {
            r.healthMonitor = 'BOTH_UNDEFINED';
            // 列出 window 上所有包含 sidecar/health 的 key
            r.sidecarKeys = Object.keys(window).filter(k => /sidecar|health|monitor/i.test(k));
        }
    } catch (e) { r.healthMonitor = { error: e.message }; }

    // INV-04 状态栏矛盾检测
    const statusEl = document.querySelector('.status-bar, #sidecar-status, [data-sidecar-status]');
    r.statusText = statusEl ? statusEl.textContent.trim().substring(0, 300) : null;
    r.statusDotClass = document.querySelector('.status-dot, .status-light')?.className || null;

    // 矛盾检测：状态栏"运行中" vs 卡片"LRC 服务未启动"
    const bodyText = document.body.innerText;
    r.lockBusyVisible = /lock_busy|后台合成|正在执行后台/.test(bodyText);
    r.indexingVisible = /索引中|正在索引/.test(bodyText);
    r.serviceNotRunning = /未启动|不可达|无法连接/.test(bodyText);
    r.runningVisible = /运行中/.test(bodyText);
    r.statusContradiction = r.serviceNotRunning && r.runningVisible;

    // L6-03 道同构度卡片
    r.daoMetricsError = (function() {
        const m = bodyText.match(/道同构度数据加载失败[：:][^\n]+/);
        return m ? m[0] : null;
    })();
    r.daoRetryBtnExists = !!Array.from(document.querySelectorAll('button, a')).find(b => /重试/.test(b.textContent));

    // 卡片数据加载状态
    r.statCardsStatus = Array.from(document.querySelectorAll('.stat-card, .stat-value, [class*="stat"]')).slice(0, 8).map(c => ({
        text: c.textContent.trim().substring(0, 50)
    }));

    // 最近记忆区域
    r.recentMemoriesState = (function() {
        const el = Array.from(document.querySelectorAll('div, section')).find(e => /最近记忆/.test(e.textContent || '') && e.textContent.length < 500);
        return el ? el.textContent.trim().substring(0, 200) : null;
    })();

    // toast 内容
    r.toastContent = (function() {
        const t = document.querySelector('.toast, [class*="toast"]');
        return t ? t.textContent.trim().substring(0, 200) : null;
    })();

    // 模态框（L2）
    r.visibleModals = Array.from(document.querySelectorAll('[class*="modal"], [role="dialog"]'))
        .filter(m => m.offsetWidth > 0 && m.offsetHeight > 0)
        .map(m => ({ class: m.className, text: m.textContent.trim().substring(0, 100) }));

    // 网络请求统计（performance API）
    try {
        const entries = performance.getEntriesByType('resource').filter(e => e.name.includes('3099') || e.name.includes('127.0.0.1'));
        r.networkRequests = entries.slice(-10).map(e => ({
            url: e.name.split('/').slice(-2).join('/'),
            duration: Math.round(e.duration),
            status: e.responseStatus || 'unknown',
            transferSize: e.transferSize
        }));
    } catch (e) { r.networkRequests = { error: e.message }; }

    // 未捕获错误
    r.globalError = window.__lastError || null;

    // INV-06：尝试触发 sidecar 启动并验证（异步）
    r.startSidecarAttempt = null;
    if (r.hasInvoke) {
        try {
            const invoke = window.__TAURI_INTERNALS__?.invoke || window.__TAURI__?.core?.invoke || window.__TAURI__?.invoke;
            r.startSidecarAttempt = 'invoking start_sidecar...';
            const t0 = Date.now();
            // 设置 15s 超时（HCSE Phase 3 验证超时机制）
            const timeoutPromise = new Promise((_, reject) =>
                setTimeout(() => reject(new Error('invoke timeout 15s')), 15000)
            );
            const invokePromise = invoke('start_sidecar', { srcDir: null, port: null, multiWindow: null });
            const result = await Promise.race([invokePromise, timeoutPromise]);
            r.startSidecarAttempt = {
                success: true,
                elapsed: Date.now() - t0,
                result: typeof result === 'object' ? JSON.stringify(result).substring(0, 200) : String(result)
            };
        } catch (e) {
            r.startSidecarAttempt = {
                success: false,
                error: e.message || String(e),
                elapsed: 'unknown'
            };
        }
    }

    return JSON.stringify(r, null, 2);
})();
