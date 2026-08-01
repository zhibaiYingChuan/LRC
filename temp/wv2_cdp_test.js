// WebView2 CDP 桌面端交互测试
// 通过 Node.js WebSocket 连接 Tauri WebView2
const WebSocket = require('ws');
const http = require('http');

const CDP_PORT = 9222;

async function getWsUrl() {
    return new Promise((resolve, reject) => {
        http.get(`http://127.0.0.1:${CDP_PORT}/json`, (res) => {
            let data = '';
            res.on('data', chunk => data += chunk);
            res.on('end', () => {
                try {
                    const pages = JSON.parse(data);
                    if (!pages || pages.length === 0) {
                        reject(new Error('No pages found'));
                        return;
                    }
                    resolve(pages[0].webSocketDebuggerUrl);
                } catch (e) { reject(e); }
            });
        }).on('error', reject);
    });
}

async function connectCDP() {
    const wsUrl = await getWsUrl();
    console.log(`WebSocket URL: ${wsUrl}`);
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
        ws.on('open', resolve);
        ws.on('error', reject);
        setTimeout(() => reject(new Error('连接超时')), 5000);
    });

    let msgId = 1;
    const pending = new Map();

    ws.on('message', (data) => {
        try {
            const msg = JSON.parse(data.toString());
            if (msg.id && pending.has(msg.id)) {
                pending.get(msg.id)(msg);
                pending.delete(msg.id);
            }
        } catch (e) { /* ignore parse errors */ }
    });

    async function send(method, params = {}) {
        const id = msgId++;
        return new Promise((resolve, reject) => {
            pending.set(id, resolve);
            ws.send(JSON.stringify({ id, method, params }));
            setTimeout(() => {
                if (pending.has(id)) {
                    pending.delete(id);
                    reject(new Error(`命令超时: ${method}`));
                }
            }, 10000);
        });
    }

    await send('Runtime.enable');
    await send('Page.enable');

    async function evalJS(expression) {
        const resp = await send('Runtime.evaluate', {
            expression,
            returnByValue: true,
            awaitPromise: true
        });
        if (resp.error) throw new Error(resp.error.message);
        const outer = resp.result;
        if (outer.exceptionDetails) {
            throw new Error(outer.exceptionDetails.text || 'JS exception');
        }
        return outer.result;
    }

    return { send, evalJS, ws, close: () => ws.close() };
}

async function run() {
    console.log('='.repeat(60));
    console.log('LRC Desktop — WebView2 CDP 桌面端交互测试');
    console.log('='.repeat(60));

    const cdp = await connectCDP();
    console.log('\n✓ CDP 连接成功\n');

    const results = { pass: 0, fail: 0, tests: [] };

    function record(name, passed, detail) {
        results.tests.push({ name, passed, detail });
        if (passed) {
            results.pass++;
            console.log(`  ✓ ${name}`);
        } else {
            results.fail++;
            console.log(`  ✗ ${name}: ${detail}`);
        }
    }

    // ===== 1. 页面基础信息 =====
    console.log('\n=== 1. 页面基础信息 ===');
    try {
        const r = await cdp.evalJS(`(function(){
            return JSON.stringify({
                title: document.title,
                url: location.href,
                bodyLength: document.body ? document.body.innerText.length : 0,
                isDesktop: typeof window.__TAURI__ !== 'undefined' || typeof window.__TAURI_INTERNALS__ !== 'undefined'
            });
        })()`);
        const info = JSON.parse(r.value);
        record('页面标题非空', !!info.title, info.title);
        record('桌面端检测', info.isDesktop, `isDesktop=${info.isDesktop}`);
        record('页面内容非空', info.bodyLength > 0, `bodyLength=${info.bodyLength}`);
        console.log(`  标题: ${info.title}`);
        console.log(`  URL: ${info.url}`);
        console.log(`  桌面端: ${info.isDesktop}`);
    } catch (e) {
        record('页面基础信息', false, e.message);
    }

    // ===== 2. 状态栏 =====
    console.log('\n=== 2. 状态栏 ===');
    try {
        const r = await cdp.evalJS(`(function(){
            const el = document.querySelector('#sidecar-down-banner, .status-bar, .banner');
            return el ? el.textContent.trim().substring(0, 100) : 'NOT_FOUND';
        })()`);
        record('状态栏/横幅存在', r.value !== 'NOT_FOUND', r.value);
    } catch (e) {
        record('状态栏', false, e.message);
    }

    // ===== 3. 导航栏/侧边栏 =====
    console.log('\n=== 3. 导航栏 ===');
    try {
        const r = await cdp.evalJS(`(function(){
            const sidebar = document.querySelector('aside.sidebar');
            if (!sidebar) return JSON.stringify({ found: false });
            const navItems = sidebar.querySelectorAll('a.nav-item');
            return JSON.stringify({
                found: true,
                navItemCount: navItems.length,
                navItems: Array.from(navItems).map(a => a.textContent.trim())
            });
        })()`);
        const nav = JSON.parse(r.value);
        record('侧边栏存在', nav.found, `items=${nav.navItemCount}`);
        if (nav.found) {
            console.log(`  导航项: ${nav.navItems.join(', ')}`);
        }
    } catch (e) {
        record('导航栏', false, e.message);
    }

    // ===== 4. 仪表盘主区域 =====
    console.log('\n=== 4. 仪表盘 ===');
    try {
        const r = await cdp.evalJS(`(function(){
            const dashboard = document.querySelector('#tab-dashboard');
            if (!dashboard) return JSON.stringify({ found: false });
            const sections = dashboard.querySelectorAll('.card, .wizard-card, .section, .dao-metrics-panel');
            return JSON.stringify({
                found: true,
                sectionCount: sections.length,
                hasWizard: !!dashboard.querySelector('.wizard-card'),
                hasMetrics: !!dashboard.querySelector('.dao-metrics-panel'),
                hasQuickActions: !!dashboard.querySelector('.quick-actions-grid')
            });
        })()`);
        const dash = JSON.parse(r.value);
        record('仪表盘存在', dash.found, `sections=${dash.sectionCount}`);
        if (dash.found) {
            console.log(`  向导: ${dash.hasWizard}, 指标: ${dash.hasMetrics}, 快捷操作: ${dash.hasQuickActions}`);
        }
    } catch (e) {
        record('仪表盘', false, e.message);
    }

    // ===== 5. AI 工具列表 =====
    console.log('\n=== 5. AI 工具检测 ===');
    try {
        const r = await cdp.evalJS(`(function(){
            const el = document.querySelector('#ai-tools-list');
            if (!el) return JSON.stringify({ found: false });
            const items = el.querySelectorAll('.tool-item, .agent-item, li, .item');
            return JSON.stringify({
                found: true,
                itemCount: items.length,
                text: el.textContent.trim().substring(0, 200)
            });
        })()`);
        const tools = JSON.parse(r.value);
        record('AI 工具列表存在', tools.found, `items=${tools.itemCount}`);
    } catch (e) {
        record('AI 工具列表', false, e.message);
    }

    // ===== 6. 语义编码模型设置 =====
    console.log('\n=== 6. 语义编码模型 ===');
    try {
        const r = await cdp.evalJS(`(function(){
            const mirror = document.getElementById('embedder-mirror');
            const setupSelect = document.getElementById('setup-llm-provider');
            return JSON.stringify({
                mirrorExists: !!mirror,
                mirrorValue: mirror ? mirror.value : '',
                setupSelectExists: !!setupSelect,
                settingSelects: document.querySelectorAll('select').length
            });
        })()`);
        const model = JSON.parse(r.value);
        record('镜像选择器存在', model.mirrorExists, `value=${model.mirrorValue}`);
        record('设置选择器存在', model.setupSelectExists, `totalSelects=${model.settingSelects}`);
    } catch (e) {
        record('语义编码模型', false, e.message);
    }

    // ===== 7. 船长日志 =====
    console.log('\n=== 7. 船长日志 ===');
    try {
        const r = await cdp.evalJS(`(function(){
            // 按内容查找船长日志相关元素
            const allEls = document.querySelectorAll('h1, h2, h3, h4, h5, h6, .card-title, .section-title');
            const logHeaders = Array.from(allEls).filter(el => 
                el.textContent.includes('船长') || el.textContent.includes('日志') || el.textContent.includes('Log')
            );
            return JSON.stringify({
                headerCount: logHeaders.length,
                headers: logHeaders.map(h => h.textContent.trim())
            });
        })()`);
        const log = JSON.parse(r.value);
        record('船长日志标题存在', log.headerCount > 0, `count=${log.headerCount}`);
        if (log.headerCount > 0) console.log(`  标题: ${log.headers.join(', ')}`);
    } catch (e) {
        record('船长日志', false, e.message);
    }

    // ===== 8. 信任中心 =====
    console.log('\n=== 8. 信任中心 ===');
    try {
        const r = await cdp.evalJS(`(function(){
            const allEls = document.querySelectorAll('h1, h2, h3, h4, h5, h6, .card-title, .section-title, .nav-item');
            const trustEls = Array.from(allEls).filter(el => 
                el.textContent.includes('信任') || el.textContent.includes('Trust') || el.textContent.includes('规则') || el.textContent.includes('Rules')
            );
            return JSON.stringify({
                count: trustEls.length,
                texts: trustEls.map(h => h.textContent.trim())
            });
        })()`);
        const trust = JSON.parse(r.value);
        record('信任中心元素存在', trust.count > 0, `count=${trust.count}`);
        if (trust.count > 0) console.log(`  内容: ${trust.texts.join(', ')}`);
    } catch (e) {
        record('信任中心', false, e.message);
    }

    // ===== 9. 截图 =====
    console.log('\n=== 9. 截图 ===');
    try {
        const screenshot = await cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true });
        if (screenshot.result && screenshot.result.data) {
            const fs = require('fs');
            const buf = Buffer.from(screenshot.result.data, 'base64');
            fs.writeFileSync('G:\\code-memory\\temp\\wv2_desktop_screenshot.png', buf);
            record('截图成功', true, `大小: ${buf.length} bytes`);
        } else {
            record('截图', false, '无数据返回');
        }
    } catch (e) {
        record('截图', false, e.message);
    }

    // ===== 汇总 =====
    console.log('\n' + '='.repeat(60));
    console.log(`测试结果汇总: ${results.pass}/${results.pass + results.fail} 通过`);
    if (results.fail > 0) {
        console.log(`失败项: ${results.fail}`);
        results.tests.filter(t => !t.passed).forEach(t => {
            console.log(`  ✗ ${t.name}: ${t.detail}`);
        });
    }
    console.log('='.repeat(60));

    cdp.close();
    return results;
}

run().then(results => {
    process.exit(results.fail > 0 ? 1 : 0);
}).catch(err => {
    console.error('FATAL:', err.message);
    process.exit(1);
});