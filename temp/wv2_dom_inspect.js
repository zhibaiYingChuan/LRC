// 检查 WebView2 中实际的 DOM 结构
const WebSocket = require('ws');
const http = require('http');

async function getWsUrl() {
    return new Promise((resolve, reject) => {
        http.get('http://127.0.0.1:9222/json', (res) => {
            let data = '';
            res.on('data', chunk => data += chunk);
            res.on('end', () => {
                try { const pages = JSON.parse(data); resolve(pages[0].webSocketDebuggerUrl); }
                catch (e) { reject(e); }
            });
        }).on('error', reject);
    });
}

async function connectCDP() {
    const wsUrl = await getWsUrl();
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
        ws.on('open', resolve);
        ws.on('error', reject);
        setTimeout(() => reject(new Error('超时')), 5000);
    });
    let msgId = 1;
    const pending = new Map();
    ws.on('message', (data) => {
        try {
            const msg = JSON.parse(data.toString());
            if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
        } catch (e) {}
    });
    async function send(method, params = {}) {
        const id = msgId++;
        return new Promise((resolve, reject) => {
            pending.set(id, resolve);
            ws.send(JSON.stringify({ id, method, params }));
            setTimeout(() => { if (pending.has(id)) { pending.delete(id); reject(new Error(`超时: ${method}`)); } }, 10000);
        });
    }
    await send('Runtime.enable');
    async function evalJS(expression) {
        const resp = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
        if (resp.error) throw new Error(resp.error.message);
        const outer = resp.result;
        if (outer.exceptionDetails) throw new Error(outer.exceptionDetails.text || 'JS exception');
        return outer.result;
    }
    return { evalJS, close: () => ws.close() };
}

async function run() {
    const cdp = await connectCDP();
    console.log('=== WebView2 DOM 结构检查 ===\n');

    // 1. 所有主要 section/div 的 ID 和 class
    console.log('--- 1. 页面主要元素 ---');
    const mainElements = await cdp.evalJS(`(function(){
        const els = document.querySelectorAll('[id], [class]');
        const result = [];
        const seen = new Set();
        els.forEach(el => {
            const id = el.id ? '#' + el.id : '';
            const cls = el.className && typeof el.className === 'string' ? '.' + el.className.split(' ').filter(Boolean).join('.') : '';
            const tag = el.tagName.toLowerCase();
            const key = tag + id + cls;
            if (!seen.has(key) && key.length < 100) {
                seen.add(key);
                result.push(key);
            }
        });
        return JSON.stringify(result.slice(0, 80));
    })()`);
    console.log(mainElements.value);

    // 2. 语义编码模型相关
    console.log('\n--- 2. 语义编码模型 ---');
    const embedder = await cdp.evalJS(`(function(){
        const sel = document.getElementById('embedder-model');
        if (sel) {
            return JSON.stringify({ found: true, tag: sel.tagName, type: sel.type, value: sel.value, parent: sel.parentElement?.className });
        }
        // 尝试查找任何 select/input 元素
        const allSelects = document.querySelectorAll('select');
        const allInputs = document.querySelectorAll('input[type="text"], input[type="search"]');
        return JSON.stringify({
            found: false,
            selectCount: allSelects.length,
            inputCount: allInputs.length,
            selects: Array.from(allSelects).slice(0,5).map(s => ({ id: s.id, name: s.name, className: s.className })),
            inputs: Array.from(allInputs).slice(0,5).map(i => ({ id: i.id, name: i.name, className: i.className }))
        });
    })()`);
    console.log(embedder.value);

    // 3. 工具检测相关
    console.log('\n--- 3. 工具检测 ---');
    const tools = await cdp.evalJS(`(function(){
        const keywords = ['tools', 'detect', 'agent', 'ide', 'scan', 'configure'];
        const result = {};
        keywords.forEach(k => {
            const byClass = document.querySelectorAll('[class*="' + k + '"], [id*="' + k + '"]');
            const byText = Array.from(document.querySelectorAll('h1, h2, h3, h4, h5, h6, button, a, span')).filter(el => 
                el.textContent.toLowerCase().includes(k)
            );
            result[k] = {
                bySelector: byClass.length,
                byText: byText.length,
                samples: Array.from(byClass).slice(0,3).map(el => el.tagName + (el.id ? '#' + el.id : '') + (el.className ? '.' + el.className.split(' ').filter(Boolean).join('.') : ''))
            };
        });
        return JSON.stringify(result);
    })()`);
    console.log(tools.value);

    // 4. 导航/侧边栏
    console.log('\n--- 4. 导航栏 ---');
    const nav = await cdp.evalJS(`(function(){
        const navs = document.querySelectorAll('nav, .nav, .sidebar, [role="navigation"], [class*="sidebar"], [class*="nav-"]');
        return JSON.stringify({
            count: navs.length,
            elements: Array.from(navs).slice(0,5).map(n => ({
                tag: n.tagName,
                id: n.id,
                className: n.className,
                children: n.children.length,
                text: n.textContent.trim().substring(0, 100)
            }))
        });
    })()`);
    console.log(nav.value);

    cdp.close();
}

run().catch(err => { console.error('FATAL:', err.message); process.exit(1); });