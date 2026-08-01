// 简单测试 - 先验证 CDP 连接和页面基础信息
(async () => {
    return JSON.stringify({
        title: document.title,
        url: window.location.href,
        bodyExists: !!document.body,
        bodyLength: document.body?.innerText?.length || 0,
        isDesktop: typeof window.__TAURI__ !== 'undefined' || typeof window.__TAURI_INTERNALS__ !== 'undefined',
        hasTauri: typeof window.__TAURI__ !== 'undefined',
        hasTauriInternals: typeof window.__TAURI_INTERNALS__ !== 'undefined',
        timing: {
            domContentLoaded: performance.timing?.domContentLoadedEventEnd,
            loadComplete: performance.timing?.loadEventEnd,
        }
    });
})();