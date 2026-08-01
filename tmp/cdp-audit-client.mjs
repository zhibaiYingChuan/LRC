// CDP 审计客户端 — 连接 Tauri WebView2 远程调试端口
// 使用 Node.js v24 内置 WebSocket + http 模块（避免 undici fetch 对 127.0.0.1 的问题）
import http from 'node:http';
// Node.js v24 全局 WebSocket（无需 import）

const HTTP_TARGET = 'http://127.0.0.1:9223/json';

// 用 http.get 获取 CDP target（避免 fetch 的 undici 问题）
function fetchTargetWsUrl() {
  return new Promise((resolve, reject) => {
    const req = http.get(HTTP_TARGET, { timeout: 5000 }, (res) => {
      let data = '';
      res.on('data', (chunk) => { data += chunk; });
      res.on('end', () => {
        try {
          const targets = JSON.parse(data);
          const page = targets.find(t => t.type === 'page' && t.webSocketDebuggerUrl);
          if (page) resolve(page.webSocketDebuggerUrl);
          else reject(new Error('未找到 page target'));
        } catch (e) {
          reject(new Error('JSON 解析失败: ' + e.message + ' data=' + data.substring(0, 200)));
        }
      });
    });
    req.on('error', reject);
    req.on('timeout', () => { req.destroy(); reject(new Error('获取 target 超时')); });
  });
}

// 单次 evaluate
export async function cdpEvaluate(expression, awaitPromise = true, timeoutMs = 30000) {
  const wsUrl = await fetchTargetWsUrl();
  const ws = new WebSocket(wsUrl);
  let msgId = 1;
  const pending = new Map();

  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      try { ws.close(); } catch (e) {}
      reject(new Error(`CDP evaluate 超时 ${timeoutMs}ms`));
    }, timeoutMs);

    ws.addEventListener('open', () => {
      ws.send(JSON.stringify({ id: msgId, method: 'Runtime.enable' }));
      pending.set(msgId++, { resolve: () => {}, reject: () => {} });
      ws.send(JSON.stringify({
        id: msgId,
        method: 'Runtime.evaluate',
        params: { expression, awaitPromise, returnByValue: true, userGesture: true }
      }));
      pending.set(msgId, { resolve: (r) => { clearTimeout(timer); try { ws.close(); } catch(e){} resolve(r); }, reject });
    });

    ws.addEventListener('message', (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id && pending.has(msg.id)) {
        const p = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.error) p.reject(new Error(JSON.stringify(msg.error)));
        else p.resolve(msg.result);
      }
    });

    ws.addEventListener('error', (e) => {
      clearTimeout(timer);
      reject(new Error('CDP WebSocket 错误: ' + (e.message || e.error || 'unknown')));
    });
  });
}

// 批量测试：连接一次，执行多个 evaluate，收集 console 日志
export async function cdpBatch(expressions, timeoutMs = 60000) {
  const wsUrl = await fetchTargetWsUrl();
  const ws = new WebSocket(wsUrl);
  let msgId = 1;
  const pending = new Map();
  const consoleLogs = [];
  const exceptions = [];
  const results = [];

  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      try { ws.close(); } catch (e) {}
      reject(new Error(`CDP batch 超时 ${timeoutMs}ms`));
    }, timeoutMs);

    ws.addEventListener('open', () => {
      ws.send(JSON.stringify({ id: msgId, method: 'Runtime.enable' }));
      pending.set(msgId++, { resolve: () => {}, reject: () => {} });
      (async () => {
        for (const expr of expressions) {
          const id = msgId++;
          const r = await new Promise((res, rej) => {
            pending.set(id, { resolve: res, reject: rej });
            ws.send(JSON.stringify({
              id,
              method: 'Runtime.evaluate',
              params: { expression: expr.expr, awaitPromise: expr.await !== false, returnByValue: true, userGesture: true }
            }));
          }).catch(e => ({ error: e.message }));
          results.push({ name: expr.name, result: r });
        }
        clearTimeout(timer);
        try { ws.close(); } catch (e) {}
        resolve({ results, consoleLogs, exceptions });
      })();
    });

    ws.addEventListener('message', (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id && pending.has(msg.id)) {
        const p = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.error) p.reject(new Error(JSON.stringify(msg.error)));
        else p.resolve(msg.result);
      }
      if (msg.method === 'Runtime.consoleAPICalled') {
        const args = (msg.params.args || []).map(a => a.value !== undefined ? JSON.stringify(a.value) : (a.description || a.type || '')).join(' ');
        consoleLogs.push({ type: msg.params.type, text: args, ts: Date.now() });
      }
      if (msg.method === 'Runtime.exceptionThrown') {
        exceptions.push({ text: msg.params.exceptionDetails?.exception?.description || JSON.stringify(msg.params.exceptionDetails), ts: Date.now() });
      }
    });

    ws.addEventListener('error', (e) => {
      clearTimeout(timer);
      reject(new Error('CDP WebSocket 错误: ' + (e.message || e.error || 'unknown')));
    });
  });
}

// 带延迟的批量测试（每个表达式之间等待指定时间，用于观察异步行为）
export async function cdpBatchWithDelay(expressions, delayMs = 1000, timeoutMs = 120000) {
  const wsUrl = await fetchTargetWsUrl();
  const ws = new WebSocket(wsUrl);
  let msgId = 1;
  const pending = new Map();
  const consoleLogs = [];
  const exceptions = [];
  const results = [];

  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      try { ws.close(); } catch (e) {}
      reject(new Error(`CDP batch 超时 ${timeoutMs}ms`));
    }, timeoutMs);

    ws.addEventListener('open', () => {
      ws.send(JSON.stringify({ id: msgId, method: 'Runtime.enable' }));
      pending.set(msgId++, { resolve: () => {}, reject: () => {} });
      (async () => {
        for (const expr of expressions) {
          const id = msgId++;
          const r = await new Promise((res, rej) => {
            pending.set(id, { resolve: res, reject: rej });
            ws.send(JSON.stringify({
              id,
              method: 'Runtime.evaluate',
              params: { expression: expr.expr, awaitPromise: expr.await !== false, returnByValue: true, userGesture: true }
            }));
          }).catch(e => ({ error: e.message }));
          results.push({ name: expr.name, result: r });
          if (delayMs > 0) await new Promise(r => setTimeout(r, delayMs));
        }
        clearTimeout(timer);
        try { ws.close(); } catch (e) {}
        resolve({ results, consoleLogs, exceptions });
      })();
    });

    ws.addEventListener('message', (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id && pending.has(msg.id)) {
        const p = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.error) p.reject(new Error(JSON.stringify(msg.error)));
        else p.resolve(msg.result);
      }
      if (msg.method === 'Runtime.consoleAPICalled') {
        const args = (msg.params.args || []).map(a => a.value !== undefined ? JSON.stringify(a.value) : (a.description || a.type || '')).join(' ');
        consoleLogs.push({ type: msg.params.type, text: args, ts: Date.now() });
      }
      if (msg.method === 'Runtime.exceptionThrown') {
        exceptions.push({ text: msg.params.exceptionDetails?.exception?.description || JSON.stringify(msg.params.exceptionDetails), ts: Date.now() });
      }
    });

    ws.addEventListener('error', (e) => {
      clearTimeout(timer);
      reject(new Error('CDP WebSocket 错误: ' + (e.message || e.error || 'unknown')));
    });
  });
}
