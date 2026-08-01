// WebView2 CDP 桌面端完整交互测试
// 通过 Node.js WebSocket 连接 Tauri WebView2
const WebSocket = require('ws');
const http = require('http');

const CDP_PORT = 9223;

async function getWsUrl() {
    const pages = await new Promise((resolve, reject) => {
        http.get(`http://127.0.0.1:${CDP_PORT}/json`, (res) => {
            let data = '';
            res.on('data', chunk => data += chunk);
            res.on('end', () => { try { resolve(JSON.parse(data)); } catch(e) { reject(e); } });
        }).on('error', reject);
    });
    return pages[0].webSocketDebuggerUrl;
}

async function run() {
    const wsUrl = await getWsUrl();
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
        ws.on('open', resolve);
        ws.on('error', reject);
        setTimeout(() => reject(new Error('Connect timeout')), 5000);
    });

    let msgId = 1;
    const pending = {};
    ws.on('message', (data) => {
        try {
            const msg = JSON.parse(data.toString());
            if (msg.id && pending[msg.id]) {
                pending[msg.id](msg);
                delete pending[msg.id];
            }
        } catch(e) {}
    });

    function send(method, params = {}) {
        return new Promise((resolve, reject) => {
            const id = msgId++;
            pending[id] = resolve;
            ws.send(JSON.stringify({ id, method, params }));
            setTimeout(() => {
                if (pending[id]) {
                    delete pending[id];
                    reject(new Error(`Timeout: ${method}`));
                }
            }, 10000);
        });
    }

    // 启用 Runtime
    await send('Runtime.enable');

    // 辅助函数：执行 JS 表达式并返回结果
    async function evalJS(expression) {
        const resp = await send('Runtime.evaluate', {
            expression,
            returnByValue: true,
            generatePreview: false
        });
        return resp.result;
    }

    const results = {};

    // 1. 页面基础信息
    console.log('\n=== 1. 页面基础信息 ===');
    const pageInfo = await evalJS(`
        JSON.stringify({
            title: document.title,
            url: location.href,
            bodyLen: (document.body && document.body.innerText) ? document.body.innerText.length : 0,
            isDesktop: (typeof window.__TAURI__ !== 'undefined' || typeof window.__TAURI_INTERNALS__ !== 'undefined'),
            hasTauri: typeof window.__TAURI__ !== 'undefined',
            hasTauriInternals: typeof window.__TAURI_INTERNALS__ !== 'undefined'
        })
    `);
    results.pageInfo = JSON.parse(pageInfo.value);
    console.log(results.pageInfo);

    // 2. 导航路由
    console.log('\n=== 2. 导航路由 ===');
    const route = await evalJS('JSON.stringify({hash:location.hash,path:location.pathname})');
    results.route = JSON.parse(route.value);
    console.log(results.route);

    // 3. 状态栏
    console.log('\n=== 3. 状态栏 ===');
    const statusBar = await evalJS(`
        JSON.stringify({
            statusText: (document.getElementById('status-text') ? document.getElementById('status-text').textContent : null),
            statusDot: (document.getElementById('status-dot') ? document.getElementById('status-dot').className : null),
            bannerHidden: (document.getElementById('sidecar-down-banner') ? document.getElementById('sidecar-down-banner').hidden : null),
            version: (document.getElementById('status-version') ? document.getElementById('status-version').textContent : null),
            sysVersion: (document.getElementById('sys-version') ? document.getElementById('sys-version').textContent : null)
        })
    `);
    results.statusBar = JSON.parse(statusBar.value);
    console.log(results.statusBar);

    // 4. 语义编码模型
    console.log('\n=== 4. 语义编码模型 ===');
    const embedder = await evalJS(`
        JSON.stringify({
            inputValue: (document.getElementById('embedder-model') ? document.getElementById('embedder-model').value : null),
            inputExists: !!document.getElementById('embedder-model'),
            activeCard: (document.querySelector('.provider-card.active[data-arg]') ? document.querySelector('.provider-card.active[data-arg]').getAttribute('data-arg') : null),
            activeCardText: (document.querySelector('.provider-card.active[data-arg]') ? document.querySelector('.provider-card.active[data-arg]').textContent.trim() : null),
            testBtn: (document.querySelector('[data-action="testEmbedderConnection"]') ? document.querySelector('[data-action="testEmbedderConnection"]').textContent : null)
        })
    `);
    results.embedder = JSON.parse(embedder.value);
    console.log(results.embedder);

    // 5. 工具检测
    console.log('\n=== 5. 工具检测 ===');
    const tools = await evalJS(`
        JSON.stringify({
            detectBtn: (document.querySelector('[data-action="detectAgents"]') ? document.querySelector('[data-action="detectAgents"]').textContent : null),
            detectDisabled: (document.querySelector('[data-action="detectAgents"]') ? document.querySelector('[data-action="detectAgents"]').disabled : null),
            retryBtn: (document.querySelector('[data-action="retryDetectAgents"]') ? document.querySelector('[data-action="retryDetectAgents"]').textContent : null),
            toolItems: Array.from(document.querySelectorAll('.tool-item, .agent-item, [class*="tool"]')).map(function(e) { return e.textContent.trim(); }),
            toolTable: Array.from(document.querySelectorAll('table tr')).map(function(e) { return e.textContent.trim(); })
        })
    `);
    results.tools = JSON.parse(tools.value);
    console.log(results.tools);

    // 6. 船长日志
    console.log('\n=== 6. 船长日志 ===');
    const captainLog = await evalJS(`
        JSON.stringify({
            section: (document.querySelector('[data-section="captain-log"], #captain-log, .captain-log, [class*="captain"]') ? document.querySelector('[data-section="captain-log"], #captain-log, .captain-log, [class*="captain"]').textContent.trim().substring(0, 300) : null),
            logContent: (document.querySelector('.log-content, [class*="log"]') ? document.querySelector('.log-content, [class*="log"]').textContent.trim().substring(0, 300) : null),
            refreshBtn: (document.querySelector('[data-action="refreshCaptainLog"], [onclick*="captain"], [onclick*="log"]') ? document.querySelector('[data-action="refreshCaptainLog"], [onclick*="captain"], [onclick*="log"]').textContent : null)
        })
    `);
    results.captainLog = JSON.parse(captainLog.value);
    console.log(results.captainLog);

    // 7. 数据存储
    console.log('\n=== 7. 数据存储 ===');
    const storage = await evalJS(`
        JSON.stringify({
            section: (document.querySelector('[data-section="data-storage"], #data-storage, .data-storage, [class*="storage"]') ? document.querySelector('[data-section="data-storage"], #data-storage, .data-storage, [class*="storage"]').textContent.trim().substring(0, 300) : null),
            status: (document.querySelector('[class*="storage"] [class*="status"], [class*="storage"] [class*="state"]') ? document.querySelector('[class*="storage"] [class*="status"], [class*="storage"] [class*="state"]').textContent.trim() : null)
        })
    `);
    results.storage = JSON.parse(storage.value);
    console.log(results.storage);

    // 8. 信任中心
    console.log('\n=== 8. 信任中心 ===');
    const trustCenter = await evalJS(`
        JSON.stringify({
            exists: !!document.querySelector('.trust-center, #trust-center, [data-section="trust-center"]'),
            text: (document.querySelector('.trust-center, #trust-center, [data-section="trust-center"]') ? document.querySelector('.trust-center, #trust-center, [data-section="trust-center"]').textContent.trim().substring(0, 500) : null),
            buttons: Array.from((document.querySelector('.trust-center, #trust-center, [data-section="trust-center"]') || document).querySelectorAll('button, [role="button"]')).map(function(b) { return { text: b.textContent.trim(), disabled: b.disabled, action: b.getAttribute('data-action') || '' }; })
        })
    `);
    results.trustCenter = JSON.parse(trustCenter.value);
    console.log(results.trustCenter);

    // 9. 所有按钮
    console.log('\n=== 9. 所有按钮 ===');
    const buttons = await evalJS(`
        JSON.stringify(Array.from(document.querySelectorAll('button, [role="button"], .btn, [class*="btn"]')).map(function(b) {
            return { text: b.textContent.trim().substring(0, 100), id: b.id, disabled: b.disabled, visible: (b.offsetParent !== null), action: b.getAttribute('data-action') || '' };
        }))
    `);
    results.buttons = JSON.parse(buttons.value);
    console.log('Total buttons:', results.buttons.length);
    results.buttons.forEach(function(b) { if (b.text) console.log('  -', b.text, b.disabled ? '(disabled)' : '', b.action ? '[' + b.action + ']' : ''); });

    // 10. 所有输入框
    console.log('\n=== 10. 输入框 ===');
    const inputs = await evalJS(`
        JSON.stringify(Array.from(document.querySelectorAll('input, select, textarea')).map(function(el) {
            return { id: el.id, type: (el.type || el.tagName), value: (el.value || '').substring(0, 100), placeholder: el.placeholder || '', disabled: el.disabled };
        }))
    `);
    results.inputs = JSON.parse(inputs.value);
    console.log('Total inputs:', results.inputs.length);
    results.inputs.forEach(function(i) { if (i.id || i.value) console.log('  -', i.id || '(unnamed)', '=', i.value, i.disabled ? '(disabled)' : ''); });

    // 11. Toast 容器
    console.log('\n=== 11. Toast ===');
    const toasts = await evalJS(`
        JSON.stringify({
            exists: !!document.getElementById('toast-container'),
            toasts: Array.from(document.querySelectorAll('#toast-container .toast')).map(function(t) { return { className: t.className, text: t.textContent.trim() }; })
        })
    `);
    results.toasts = JSON.parse(toasts.value);
    console.log(results.toasts);

    // 12. 确认对话框
    console.log('\n=== 12. 确认对话框 ===');
    const confirmDlg = await evalJS(`
        JSON.stringify({
            exists: !!document.querySelector('.confirm-modal, .modal-backdrop, [class*="confirm"]'),
            text: (document.querySelector('.confirm-modal, .modal-backdrop, [class*="confirm"]') ? document.querySelector('.confirm-modal, .modal-backdrop, [class*="confirm"]').textContent.trim().substring(0, 200) : null)
        })
    `);
    results.confirmDlg = JSON.parse(confirmDlg.value);
    console.log(results.confirmDlg);

    // 13. 控制台错误
    console.log('\n=== 13. 控制台错误 ===');
    const errors = await evalJS(`
        JSON.stringify((window.__capturedErrors || []).slice(0, 20))
    `);
    results.consoleErrors = JSON.parse(errors.value);
    console.log('Errors:', results.consoleErrors.length);

    // 输出汇总
    console.log('\n' + '='.repeat(60));
    console.log('测试结果汇总');
    console.log('='.repeat(60));
    console.log(JSON.stringify(results, null, 2));

    ws.close();
    return results;
}

run().then(results => {
    console.log('\n测试完成');
    process.exit(0);
}).catch(err => {
    console.error('FATAL:', err.message);
    process.exit(1);
});