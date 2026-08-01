// WebView2 CDP 桌面端交互测试 - 通过 CDP Runtime.evaluate 执行
// 测试覆盖：语义编码模型、船长日志、数据存储、工具检测、基础交互

(async () => {
    const results = {};
    
    // 1. 基础页面状态
    results.pageInfo = {
        title: document.title,
        url: window.location.href,
        isDesktop: typeof window.__TAURI__ !== 'undefined' || typeof window.__TAURI_INTERNALS__ !== 'undefined',
        version: document.getElementById('status-version')?.textContent,
        sysVersion: document.getElementById('sys-version')?.textContent,
    };
    
    // 2. 状态栏检查
    results.statusBar = {
        statusText: document.getElementById('status-text')?.textContent,
        statusDot: document.getElementById('status-dot')?.className,
        bannerHidden: document.getElementById('sidecar-down-banner')?.hidden,
        bannerText: document.getElementById('sidecar-down-banner')?.textContent?.trim(),
    };
    
    // 3. 导航路由检查
    const currentRoute = window.location.hash || window.location.pathname;
    results.currentRoute = currentRoute;
    
    // 4. 语义编码模型检查
    const embedderModelInput = document.getElementById('embedder-model');
    const embedderModelSelect = document.querySelector('[data-action="selectEmbedderModel"]');
    const activeCard = document.querySelector('.provider-card.active[data-arg]');
    results.embedderModel = {
        inputValue: embedderModelInput?.value,
        inputExists: !!embedderModelInput,
        activeCardExists: !!activeCard,
        activeCardArg: activeCard?.getAttribute('data-arg'),
        activeCardText: activeCard?.textContent?.trim(),
        selectBtnExists: !!embedderModelSelect,
        testBtn: document.querySelector('[data-action="testEmbedderConnection"]')?.textContent,
    };
    
    // 5. 模型选择下拉
    const modelSelect = document.getElementById('embedder-model');
    if (modelSelect && modelSelect.tagName === 'SELECT') {
        results.embedderOptions = Array.from(modelSelect.options).map(o => ({
            value: o.value,
            text: o.text,
            selected: o.selected
        }));
    }
    
    // 6. 工具检测
    results.toolDetection = {
        detectBtn: document.querySelector('[data-action="detectAgents"]')?.textContent,
        detectBtnDisabled: document.querySelector('[data-action="detectAgents"]')?.disabled,
        toolList: Array.from(document.querySelectorAll('.tool-item, .agent-item, [class*="tool"]')).map(el => ({
            name: el.textContent?.trim(),
            className: el.className,
            installed: el.querySelector('[class*="check"], [class*="installed"]') ? true : false,
        })),
        toolTable: Array.from(document.querySelectorAll('table.tools-table tr, .tools-list li')).map(el => el.textContent?.trim()),
    };
    
    // 7. 船长日志
    results.captainLog = {
        section: document.querySelector('[data-section="captain-log"], #captain-log, .captain-log, [class*="captain"]')?.textContent?.trim(),
        logContent: document.querySelector('[data-section="captain-log"] .log-content, #captain-log .log-content, .captain-log .log-content, [class*="captain"] [class*="log"]')?.textContent?.trim(),
        refreshBtn: document.querySelector('[data-action="refreshCaptainLog"], [onclick*="captain"], [onclick*="log"]')?.textContent,
    };
    
    // 8. 数据存储
    results.dataStorage = {
        section: document.querySelector('[data-section="data-storage"], #data-storage, .data-storage, [class*="storage"]')?.textContent?.trim(),
        status: document.querySelector('[class*="storage"] [class*="status"], [class*="storage"] [class*="state"]')?.textContent?.trim(),
    };
    
    // 9. 信任中心
    const trustCenter = document.querySelector('.trust-center, #trust-center, [data-section="trust-center"]');
    results.trustCenter = {
        exists: !!trustCenter,
        text: trustCenter?.textContent?.trim().substring(0, 500),
        buttons: trustCenter ? Array.from(trustCenter.querySelectorAll('button, [role="button"], a')).map(b => ({
            text: b.textContent?.trim(),
            className: b.className,
            disabled: b.disabled,
        })) : [],
    };
    
    // 10. 所有可点击按钮
    results.allButtons = Array.from(document.querySelectorAll('button, [role="button"], .btn, [class*="btn"]')).map(b => ({
        text: b.textContent?.trim().substring(0, 100),
        id: b.id,
        className: b.className?.substring(0, 100),
        disabled: b.disabled,
        visible: b.offsetParent !== null,
        action: b.getAttribute('data-action') || b.getAttribute('onclick') || '',
    }));
    
    // 11. 所有输入框
    results.allInputs = Array.from(document.querySelectorAll('input, select, textarea')).map(el => ({
        id: el.id,
        type: el.type || el.tagName,
        value: (el.value || '').substring(0, 100),
        placeholder: el.placeholder,
        disabled: el.disabled,
    }));
    
    // 12. 控制台错误检查
    results.consoleErrors = (window.__capturedErrors || []).slice(0, 20);
    
    // 13. Toast 容器
    results.toastContainer = {
        exists: !!document.getElementById('toast-container'),
        toasts: Array.from(document.querySelectorAll('#toast-container .toast')).map(t => ({
            type: t.className,
            text: t.textContent?.trim(),
        })),
    };
    
    // 14. 确认对话框
    results.confirmDialog = {
        exists: !!document.querySelector('.confirm-modal, .modal, [class*="confirm"]'),
        modalText: document.querySelector('.confirm-modal, .modal, [class*="confirm"]')?.textContent?.trim(),
    };
    
    return JSON.stringify(results, null, 2);
})();