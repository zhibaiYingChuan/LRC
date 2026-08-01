// WebView2 CDP 测试 v2 - 先检查页面加载状态
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
    if (!pages || pages.length === 0) throw new Error('No pages');
    return pages[0].webSocketDebuggerUrl;
}

async function run() {
    const wsUrl = await getWsUrl();
    console.log('WS:', wsUrl);
    
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
        ws.on('open', resolve);
        ws.on('error', reject);
        setTimeout(() => reject(new Error('Connect timeout')), 5000);
    });
    console.log('Connected');

    let msgId = 1;
    const pending = {};

    ws.on('message', (data) => {
        try {
            const msg = JSON.parse(data.toString());
            if (msg.id && pending[msg.id]) {
                pending[msg.id](msg);
                delete pending[msg.id];
            } else {
                console.log('Event:', data.toString().substring(0, 200));
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

    // Step 1: 启用 Page 域
    console.log('\n1. Enabling Page domain...');
    try {
        await send('Page.enable');
        console.log('Page enabled');
    } catch(e) {
        console.log('Page.enable error:', e.message);
    }

    // Step 2: 获取页面资源树
    console.log('\n2. Getting resource tree...');
    try {
        const tree = await send('Page.getResourceTree');
        console.log('Frame:', tree.frameTree.frame.url);
        console.log('Resources:', tree.frameTree.resources ? tree.frameTree.resources.length : 0);
    } catch(e) {
        console.log('getResourceTree error:', e.message);
    }

    // Step 3: 尝试简单的 Runtime.evaluate
    console.log('\n3. Simple evaluate...');
    try {
        const result = await send('Runtime.evaluate', {
            expression: '1+1',
            returnByValue: true
        });
        console.log('1+1 =', result.result.value);
    } catch(e) {
        console.log('evaluate error:', e.message);
    }

    // Step 4: 尝试获取 document.title
    console.log('\n4. Getting document.title...');
    try {
        const result = await send('Runtime.evaluate', {
            expression: 'document.title',
            returnByValue: true
        });
        console.log('title:', result.result.value);
    } catch(e) {
        console.log('title error:', e.message);
    }

    // Step 5: 检查页面加载状态
    console.log('\n5. Checking page load state...');
    try {
        const result = await send('Runtime.evaluate', {
            expression: 'document.readyState',
            returnByValue: true
        });
        console.log('readyState:', result.result.value);
    } catch(e) {
        console.log('readyState error:', e.message);
    }

    // Step 6: 获取可见的 body 内容
    console.log('\n6. Getting body innerText...');
    try {
        const result = await send('Runtime.evaluate', {
            expression: 'document.body && document.body.innerText ? document.body.innerText.substring(0, 500) : "no body"',
            returnByValue: true,
            timeout: 5000
        });
        console.log('body:', result.result.value);
    } catch(e) {
        console.log('body error:', e.message);
    }

    ws.close();
    console.log('\nDone');
}

run().catch(err => {
    console.error('FATAL:', err.message);
    process.exit(1);
});