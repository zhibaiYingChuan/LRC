// WebView2 CDP 测试 v3 - 调试响应结构
const WebSocket = require('ws');
const http = require('http');

async function run() {
    const pages = await new Promise((resolve, reject) => {
        http.get('http://127.0.0.1:9223/json', (res) => {
            let data = '';
            res.on('data', chunk => data += chunk);
            res.on('end', () => { try { resolve(JSON.parse(data)); } catch(e) { reject(e); } });
        }).on('error', reject);
    });
    
    const wsUrl = pages[0].webSocketDebuggerUrl;
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
            console.log('RAW RESPONSE:', JSON.stringify(msg, null, 2).substring(0, 500));
            if (msg.id && pending[msg.id]) {
                pending[msg.id](msg);
                delete pending[msg.id];
            }
        } catch(e) {
            console.log('Parse error:', e.message);
        }
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

    // 先启用 Runtime
    console.log('\nEnabling Runtime...');
    await send('Runtime.enable');

    // 简单表达式
    console.log('\nEvaluating 1+1...');
    const resp = await send('Runtime.evaluate', {
        expression: '1+1',
        returnByValue: true,
        generatePreview: false
    });
    console.log('Full response:', JSON.stringify(resp, null, 2));

    // 获取 document.title
    console.log('\nEvaluating document.title...');
    const resp2 = await send('Runtime.evaluate', {
        expression: 'document.title',
        returnByValue: true
    });
    console.log('title response:', JSON.stringify(resp2, null, 2));

    ws.close();
    console.log('\nDone');
}

run().catch(err => {
    console.error('FATAL:', err.message);
    process.exit(1);
});