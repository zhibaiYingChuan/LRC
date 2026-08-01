// HCSE Phase 3 RV-Monitor 前端探针 — 一次性采集所有不变量相关状态
(function() {
    const r = {};
    // 基础环境
    r.url = location.href;
    r.title = document.title;
    r.readyState = document.readyState;

    // INV-01 IPC 自定义协议不变量：window.__TAURI__ 必须可用
    r.hasTauriInternals = !!window.__TAURI_INTERNALS__;
    r.hasTauriCore = !!(window.__TAURI__ && window.__TAURI__.core);
    r.hasInvoke = typeof (window.__TAURI_INTERNALS__?.invoke || window.__TAURI__?.core?.invoke || window.__TAURI__?.invoke) === 'function';
    r.ipcFallbackToPostMessage = !r.hasInvoke;

    // INV-02 启动取消机制不变量：startServiceAbortController 必须可读
    r.startServiceAbortController = window.startServiceAbortController
        ? { aborted: window.startServiceAbortController.signal.aborted }
        : null;

    // INV-03 健康监控不变量：SidecarHealthMonitor 必须存在且配置合理
    try {
        if (typeof sidecarHealthMonitor !== 'undefined') {
            r.healthMonitor = {
                pollInterval: sidecarHealthMonitor._pollInterval,
                maxBackoff: sidecarHealthMonitor._MAX_BACKOFF,
                reachable: sidecarHealthMonitor._reachable,
                consecutiveFailures: sidecarHealthMonitor._consecutiveFailures || 0
            };
        } else {
            r.healthMonitor = 'undefined';
        }
    } catch (e) { r.healthMonitor = { error: e.message }; }

    // INV-04 状态栏 UI 不变量：必须显示真实 sidecar 状态（不能状态矛盾）
    const statusSelectors = ['#sidecar-status', '.status-indicator', '[data-sidecar-status]', '.status-bar', '#status-bar'];
    let statusEl = null;
    for (const sel of statusSelectors) {
        statusEl = document.querySelector(sel);
        if (statusEl) { r.statusSelectorUsed = sel; break; }
    }
    r.statusText = statusEl ? statusEl.textContent.trim().substring(0, 200) : null;
    const dotSelectors = ['.status-dot', '.status-light', '.dot', '[class*="status"]'];
    let dotEl = null;
    for (const sel of dotSelectors) {
        dotEl = document.querySelector(sel);
        if (dotEl) { r.statusDotClass = dotEl.className; break; }
    }

    // INV-05 错误反馈不变量：lock_busy 期间必须有可见提示
    const bodyText = document.body.innerText;
    r.lockBusyVisible = /lock_busy|后台合成|正在执行后台/.test(bodyText);
    r.indexingVisible = /索引中|正在索引/.test(bodyText);
    r.serviceNotRunning = /未启动|不可达|无法连接/.test(bodyText);
    r.runningVisible = /运行中/.test(bodyText);
    r.toastVisible = !!document.querySelector('.toast, [class*="toast"]');

    // 卡片层级 L3/L4
    r.cards = Array.from(document.querySelectorAll('.card, .stat-card, [data-card]')).slice(0, 8).map(c => ({
        text: c.innerText.trim().substring(0, 100),
        hasRetryBtn: !!c.querySelector('[data-action="retry"], .retry-btn, #retry-btn, button[class*="retry"]'),
        hasRefreshBtn: !!c.querySelector('[data-action="refresh"], .refresh-btn, button[class*="refresh"]')
    }));

    // 道同构度卡片（L6-03 修复点）
    r.daoMetricsCard = (function() {
        const all = Array.from(document.querySelectorAll('div, section'));
        const target = all.find(e => /道同构度|dao/i.test(e.className) || /道同构度/.test(e.textContent || ''));
        return target ? target.innerText.trim().substring(0, 200) : null;
    })();

    // 错误反馈详情
    r.errorTexts = Array.from(document.querySelectorAll('[class*="error"], [class*="warn"], [class*="fail"]'))
        .map(e => e.textContent.trim()).filter(t => t.length > 5 && t.length < 300).slice(0, 5);

    // 模态框状态（L2）
    r.modals = Array.from(document.querySelectorAll('[class*="modal"], [role="dialog"]')).map(m => ({
        visible: m.offsetWidth > 0 && m.offsetHeight > 0,
        class: m.className
    }));

    // 全局未捕获错误（如果有）
    r.globalError = window.__lastError || null;

    // 网络请求统计（粗略）
    r.lastHealthCheck = window.__lastHealthCheckTime || null;

    // body 文本前 1200 字符（用于状态矛盾检测）
    r.bodyTextSample = bodyText.substring(0, 1200);

    return JSON.stringify(r, null, 2);
})();
