#!/usr/bin/env node
// HCSE Phase 3 RV-Monitor — CDP 直连评估（Node.js v24 内置 WebSocket）
// 用法: node cdp_eval.js <pageId> <scriptFile>
const http = require('http');

// Phase 6 路径白名单
const path = require('path');
const fs = require('fs');
const allowedRoots = [
  path.resolve('g:/code-memory/hcse_resilience_tester'),
  path.resolve('g:/code-memory/temp'),
  path.resolve('g:/code-memory/logs')
];

// Phase 6 数据脱敏
function sanitize(text) {
  if (!text) return text;
  text = text.replace(/"value"\s*:\s*"[^"]*"/gi, '"value":"[COOKIE_VALUE_REDACTED]"');
  text = text.replace(/(authorization["\s:]+bearer\s+)[A-Za-z0-9\-_\.]+/gi, '$1[BEARER_TOKEN_REDACTED]');
  text = text.replace(/"authorization"\s*:\s*"[^"]*"/gi, '"authorization":"[BEARER_TOKEN_REDACTED]"');
  text = text.replace(/[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}/g, '[EMAIL_REDACTED]');
  text = text.replace(/(?<![0-9])1[3-9][0-9]{9}(?![0-9])/g, '[PHONE_REDACTED]');
  return text;
}

// Phase 6 资源看门狗 MAX_CPU_TIME=60s
const watchdogStart = Date.now();
const MAX_CPU_SEC = 60;
function checkWatchdog() {
  if ((Date.now() - watchdogStart) / 1000 > MAX_CPU_SEC) {
    console.error(`[Watchdog] > MAX_CPU_TIME=${MAX_CPU_SEC}s Hard Halt`);
    process.exit(3);
  }
}

async function main() {
  // 1. 动态获取 page id（如未指定）
  let pageId = process.argv[2];
  const scriptFile = process.argv[3] || 'g:/code-memory/hcse_resilience_tester/probe_frontend.js';

  // Phase 6 路径白名单校验
  const scriptAbs = path.resolve(scriptFile);
  const inWhitelist = allowedRoots.some(r => scriptAbs.startsWith(r));
  if (!inWhitelist) {
    console.error(`[PathValidator] 拒绝执行白名单外脚本: ${scriptAbs} (HCSE Phase 6 Hard Halt)`);
    process.exit(2);
  }

  if (!pageId) {
    pageId = await new Promise((resolve, reject) => {
      http.get('http://127.0.0.1:9223/json', res => {
        let data = '';
        res.on('data', c => data += c);
        res.on('end', () => {
          try {
            const pages = JSON.parse(data);
            const page = pages.find(p => p.type === 'page' && p.url.includes('tauri'));
            resolve(page ? page.id : null);
          } catch (e) { reject(e); }
        });
      }).on('error', reject);
    });
    if (!pageId) {
      console.error('[CDP] 未找到 tauri 页面');
      process.exit(4);
    }
  }

  const wsUrl = `ws://127.0.0.1:9223/devtools/page/${pageId}`;
  console.log(`[CDP] connecting ${wsUrl}`);

  // 2. 连接 WebSocket
  const ws = new WebSocket(wsUrl);
  let msgId = 1;
  const pending = new Map();

  ws.addEventListener('message', ev => {
    try {
      const msg = JSON.parse(ev.data);
      if (msg.id && pending.has(msg.id)) {
        pending.get(msg.id)(msg);
        pending.delete(msg.id);
      }
    } catch (e) {}
  });

  await new Promise((resolve, reject) => {
    ws.addEventListener('open', resolve);
    ws.addEventListener('error', reject);
    setTimeout(() => reject(new Error('connect timeout 10s')), 10000);
  });
  console.log(`[CDP] connected state=${ws.readyState}`);

  function send(method, params = {}) {
    const id = msgId++;
    return new Promise((resolve, reject) => {
      pending.set(id, resolve);
      ws.send(JSON.stringify({ id, method, params }));
      setTimeout(() => {
        if (pending.has(id)) {
          pending.delete(id);
          reject(new Error(`method ${method} timeout 25s`));
        }
      }, 25000);
    });
  }

  // Phase 3 RV-Monitor：CDP 存活预检（避免假阴性）
  try {
    const liv = await send('Browser.getVersion', {});
    console.log(`[CDP Liveness] ${JSON.stringify(liv.result || liv).substring(0, 200)}`);
  } catch (e) {
    console.error(`[CDP Liveness] FAIL: ${e.message}`);
  }

  // 3. 执行目标 JS
  const jsExpr = fs.readFileSync(scriptAbs, 'utf-8');
  try {
    const t0 = Date.now();
    const result = await send('Runtime.evaluate', {
      expression: jsExpr,
      returnByValue: true,
      awaitPromise: true,
      timeout: 20000
    });
    const elapsed = Date.now() - t0;
    console.log(`[CDP Eval Result] elapsed=${elapsed}ms`);
    const out = JSON.stringify(result, null, 2);
    console.log(sanitize(out));
  } catch (e) {
    console.error(`[CDP Eval] FAIL: ${e.message}`);
  }

  try { ws.close(); } catch (e) {}
  setTimeout(() => process.exit(0), 200);
}

main().catch(e => {
  console.error(`[CDP] fatal: ${e.message}`);
  process.exit(5);
});

setInterval(checkWatchdog, 1000);
