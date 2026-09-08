#!/usr/bin/env node
/**
 * v0.9.7 联想中心桌面端 CDP 专项测试
 * ------------------------------------------------------------------
 * 直连 Tauri WebView2 的 CDP 端口（开发 9231 / 稳定 9230），
 * 在真实桌面 WebView 内验证联想中心的三类用户场景：
 *   A. 「今晚吃什么？」→ 联想结果全部是生活内容（无代码噪声），
 *      且每条可点「就是这个」确认（点击后出现"已确认"+成功 toast）。
 *   B. 「量子物理是什么」→ 记忆库无相关内容时展示诚实空态
 *      （weak_match 文案），而不是硬凑代码记忆。
 *   C. 「我以前记过什么重要日子？」→ 语义旁路生效：词面零重叠
 *      也能想起结婚纪念日/生日等记忆。
 *
 * 用法：node tests/frontend/association-desktop-cdp.js
 */
'use strict';

const fs = require('fs');
const path = require('path');
const CDP_PORTS = [9231, 9230];
const WS = require('ws');

const artifactDir = path.join(process.cwd(), 'playwright-artifacts');
const results = [];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function getCdpBase() {
  for (const port of CDP_PORTS) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (res.ok) return { base: `http://127.0.0.1:${port}`, port };
    } catch { /* 依次探测 */ }
  }
  throw new Error(`未找到 LRC WebView CDP 端口（${CDP_PORTS.join(', ')}）`);
}

class CDPClient {
  constructor(wsUrl) {
    this.ws = new WS(wsUrl);
    this.nextId = 1;
    this.pending = new Map();
    this.consoleErrors = [];
    this.exceptions = [];
    this.hardNetworkFailures = [];
    this._ready = new Promise((resolve, reject) => {
      this.ws.on('open', resolve);
      this.ws.on('error', reject);
    });
    this.ws.on('message', (data) => this._onMessage(data));
  }
  _onMessage(data) {
    let msg;
    try { msg = JSON.parse(data.toString()); } catch { return; }
    if (msg.id && this.pending.has(msg.id)) {
      const { resolve, reject, timer } = this.pending.get(msg.id);
      clearTimeout(timer);
      this.pending.delete(msg.id);
      if (msg.error) reject(new Error(msg.error.message || JSON.stringify(msg.error)));
      else resolve(msg.result);
      return;
    }
    this._handleEvent(msg);
  }
  _handleEvent(msg) {
    const m = msg.method;
    const p = msg.params || {};
    if (m === 'Runtime.consoleAPICalled' && (p.type === 'error' || p.type === 'assert')) {
      this.consoleErrors.push((p.args || []).map((a) => a.value ?? a.description ?? '').join(' '));
    } else if (m === 'Runtime.exceptionThrown') {
      const d = p.exceptionDetails || {};
      this.exceptions.push((d.text || '') + ' ' + (d.exception?.description || ''));
    } else if (m === 'Log.entryAdded' && p.entry?.level === 'error') {
      this.consoleErrors.push(p.entry.text || '');
    } else if (m === 'Network.loadingFailed') {
      if (!['net::ERR_ABORTED', 'net::ERR_BLOCKED_BY_CLIENT'].includes(p.errorText || '')) {
        this.hardNetworkFailures.push('FAILED ' + (p.errorText || ''));
      }
    } else if (m === 'Network.responseReceived' && (p.response?.status || 0) >= 400) {
      // 联想探索对无相关内容可能返回 200+weak_match，4xx/5xx 一律记录
      this.hardNetworkFailures.push(`${p.response.status} ${p.response?.url || ''}`);
    }
  }
  async ready() { await this._ready; }
  send(method, params = {}, timeoutMs = 20000) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.pending.has(id)) {
          this.pending.delete(id);
          reject(new Error(`CDP 超时(${timeoutMs}ms): ${method}`));
        }
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
  async eval(expression, timeoutMs = 20000) {
    const r = await this.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }, timeoutMs);
    if (r.exceptionDetails) {
      throw new Error('页面JS异常: ' + (r.exceptionDetails.text || '') + ' ' +
        (r.exceptionDetails.exception?.description || ''));
    }
    return r.result?.value;
  }
}

async function screenshot(cdp, name) {
  try {
    const r = await cdp.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true });
    fs.mkdirSync(artifactDir, { recursive: true });
    fs.writeFileSync(path.join(artifactDir, name), Buffer.from(r.data, 'base64'));
  } catch (e) {
    console.error(`[截图失败] ${name}: ${e.message}`);
  }
}

function record(scenario, ok, detail) {
  results.push({ scenario, ok, detail });
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${scenario}  ${JSON.stringify(detail)}`);
}

// 在页面内等待联想完成（结论区出现且不在加载态），最多 timeoutMs
const WAIT_EXPLORE_JS = `
(q, timeoutMs) => new Promise((resolve) => {
  const t0 = Date.now();
  const poll = () => {
    const c = document.getElementById('association-conclusion')?.textContent || '';
    const done = c.trim() && !c.includes('正在回想') && !c.includes('正在联想');
    if (done || Date.now() - t0 > timeoutMs) {
      resolve({ done, elapsed: Date.now() - t0, conclusion: c.trim() });
    } else { setTimeout(poll, 400); }
  };
  poll();
})`;

async function runExplore(cdp, query, timeoutMs = 30000) {
  return cdp.eval(`(() => {
    const input = document.getElementById('association-query');
    if (!input) return { started: false, reason: 'missing #association-query' };
    input.value = ${JSON.stringify(query)};
    window.startAssociationExplore();
    return { started: true };
  })()`).then(async (r) => {
    if (!r.started) return r;
    return { ...r, ...(await cdp.eval(`(${WAIT_EXPLORE_JS})(${JSON.stringify(query)}, ${timeoutMs})`, timeoutMs + 5000)) };
  });
}

async function main() {
  const { base, port } = await getCdpBase();
  console.log(`CDP 端口: ${port}`);
  const list = await (await fetch(`${base}/json/list`)).json();
  const page = list.find((t) => t.type === 'page');
  if (!page) throw new Error('未找到 page 目标');
  console.log(`页面: ${page.url}`);

  const cdp = new CDPClient(page.webSocketDebuggerUrl);
  await cdp.ready();
  await cdp.send('Runtime.enable');
  await cdp.send('Log.enable');
  await cdp.send('Network.enable');
  await cdp.send('Page.enable');

  // 【强制刷新】桌面 WebView 可能停留在旧版静态页——dev-proxy 直接服务 static/，
  // 必须带时间戳重新导航，确保页面加载最新 app.js（否则测的是过期前端）。
  await cdp.send('Page.navigate', { url: `http://localhost:1420/?r=${Date.now()}` });
  await sleep(3000);
  const appReady = await cdp.eval(`(() => ({
    query: !!document.getElementById('association-query'),
    hasWeakRender: String(window.startAssociationExplore || '').includes('weak_match'),
    hasConfirmBtn: String(window.startAssociationExplore || '').includes('association-confirm-btn'),
  }))()`);
  if (!appReady.query) throw new Error('刷新后页面未就绪（#association-query 不存在）');
  console.log(`前端版本检查: weak_match渲染=${appReady.hasWeakRender} 确认按钮=${appReady.hasConfirmBtn}`);
  if (!appReady.hasWeakRender || !appReady.hasConfirmBtn) {
    throw new Error('页面加载的 app.js 缺少 v0.9.7 联想修复（weak_match/确认按钮）——dev-proxy 服务的前端不是最新');
  }

  const preErrors = () => cdp.consoleErrors.length + cdp.exceptions.length + cdp.hardNetworkFailures.length;

  // ---------- 进入联想中心 ----------
  await cdp.eval(`(() => {
    if (window.gotoAssociationCenter) { window.gotoAssociationCenter(); return 'invoked'; }
    return 'missing';
  })()`);
  const tabVisible = await cdp.eval(`(() => {
    const el = document.getElementById('tab-association-center');
    if (!el) return false;
    el.scrollIntoView({ block: 'center' });
    return !!(el.offsetParent !== null || getComputedStyle(el).visibility !== 'hidden');
  })()`);
  if (!tabVisible) throw new Error('联想中心面板未可见');
  await sleep(600);

  // ---------- 场景 A：今晚吃什么？ ----------
  {
    const before = preErrors();
    const r = await runExplore(cdp, '今晚吃什么？');
    const info = await cdp.eval(`(() => {
      const nodes = [...document.querySelectorAll('#association-story .association-story-node')];
      const texts = nodes.map(n => (n.textContent || '').trim());
      return {
        storyNodes: nodes.length,
        texts: texts.slice(0, 8),
        codeNoise: texts.filter(t => /src\\/|来源：|\\.rs:|memory_store|dao_regulator/.test(t)).length,
        confirmBtns: document.querySelectorAll('#association-story .association-confirm-btn').length,
        conclusion: document.getElementById('association-conclusion')?.textContent?.trim() || '',
      };
    })()`);
    await screenshot(cdp, 'assoc-desk-A-eat.png');
    const okA = r.started && r.done && info.storyNodes > 0 && info.codeNoise === 0 && info.confirmBtns > 0;
    record('A_今晚吃什么_纯生活节点', okA, {
      started: r.started, done: r.done, elapsed: r.elapsed,
      nodes: info.storyNodes, codeNoise: info.codeNoise, confirmBtns: info.confirmBtns,
      sample: info.texts.slice(0, 3),
    });

    // ---------- 场景 A2：点击「就是这个」确认 ----------
    if (info.confirmBtns > 0) {
      await cdp.eval(`(() => {
        const btn = document.querySelector('#association-story .association-confirm-btn');
        if (!btn) return 'missing';
        btn.scrollIntoView({ block: 'center' });
        btn.click();
        return 'clicked';
      })()`);
      await sleep(1500);
      const confirmInfo = await cdp.eval(`(() => {
        const btn = document.querySelector('#association-story .association-confirm-btn');
        const toasts = [...document.querySelectorAll('#toast-container .toast, #toast-container [class*=toast]')]
          .map(t => t.textContent || '').join('|');
        return { btnText: btn?.textContent?.trim() || '', confirmed: btn?.classList.contains('association-confirmed') || false, toasts };
      })()`);
      await screenshot(cdp, 'assoc-desk-A2-confirm.png');
      const okA2 = confirmInfo.confirmed && /已确认/.test(confirmInfo.btnText)
        && /已记下|下次联想/.test(confirmInfo.toasts);
      record('A2_就是这个_确认写回', okA2, confirmInfo);
      if (!okA2) console.log('  [提示] 确认按钮态:', confirmInfo);
    }
    if (preErrors() > before) {
      record('A_无新增运行时错误', false, {
        consoleErrors: cdp.consoleErrors, exceptions: cdp.exceptions, net: cdp.hardNetworkFailures,
      });
    } else {
      record('A_无新增运行时错误', true, {});
    }
  }

  // ---------- 场景 B：量子物理是什么 → 诚实空态 ----------
  {
    const before = preErrors();
    const r = await runExplore(cdp, '量子物理是什么');
    const info = await cdp.eval(`(() => {
      const conclusion = document.getElementById('association-conclusion')?.textContent?.trim() || '';
      const empty = document.querySelector('#association-story .association-empty')?.textContent?.trim() || '';
      const nodes = document.querySelectorAll('#association-story .association-story-node').length;
      return { conclusion, empty, nodes };
    })()`);
    await screenshot(cdp, 'assoc-desk-B-empty.png');
    const okB = r.started && r.done && info.nodes === 0
      && /记忆库里还没有.*相关的内容/.test(info.conclusion)
      && /没有想起相关的念头/.test(info.empty);
    record('B_无相关内容_诚实空态', okB, info);
    if (preErrors() > before) {
      record('B_无新增运行时错误', false, {
        consoleErrors: cdp.consoleErrors, exceptions: cdp.exceptions, net: cdp.hardNetworkFailures,
      });
    } else {
      record('B_无新增运行时错误', true, {});
    }
  }

  // ---------- 场景 C：重要的日子 → 语义旁路 ----------
  {
    const before = preErrors();
    const r = await runExplore(cdp, '我以前记过什么重要日子？');
    const info = await cdp.eval(`(() => {
      const nodes = [...document.querySelectorAll('#association-story .association-story-node')];
      const texts = nodes.map(n => (n.textContent || '').trim());
      const hit = texts.filter(t => /结婚|纪念|生日|认识/.test(t)).length;
      const codeNoise = texts.filter(t => /src\\/|来源：|\\.rs:|memory_store|dao_regulator/.test(t)).length;
      return { nodes: nodes.length, hit, codeNoise, sample: texts.slice(0, 4) };
    })()`);
    await screenshot(cdp, 'assoc-desk-C-days.png');
    const okC = r.started && r.done && info.nodes > 0 && info.hit > 0 && info.codeNoise === 0;
    record('C_重要日子_语义旁路', okC, info);
    if (preErrors() > before) {
      record('C_无新增运行时错误', false, {
        consoleErrors: cdp.consoleErrors, exceptions: cdp.exceptions, net: cdp.hardNetworkFailures,
      });
    } else {
      record('C_无新增运行时错误', true, {});
    }
  }

  // ---------- 汇总 ----------
  const failed = results.filter((x) => !x.ok);
  console.log('\n========== 联想桌面端 CDP 专项结果 ==========');
  console.log(`共 ${results.length} 项，通过 ${results.length - failed.length}，失败 ${failed.length}`);
  if (cdp.consoleErrors.length) console.log('console errors:', JSON.stringify(cdp.consoleErrors, null, 2));
  if (cdp.exceptions.length) console.log('page exceptions:', JSON.stringify(cdp.exceptions, null, 2));
  if (cdp.hardNetworkFailures.length) console.log('network failures:', JSON.stringify(cdp.hardNetworkFailures, null, 2));
  fs.mkdirSync(artifactDir, { recursive: true });
  fs.writeFileSync(path.join(artifactDir, 'association-desktop-cdp-result.json'),
    JSON.stringify({ results, consoleErrors: cdp.consoleErrors, exceptions: cdp.exceptions, hardNetworkFailures: cdp.hardNetworkFailures }, null, 2));
  cdp.ws.close();
  if (failed.length) process.exitCode = 1;
}

main().catch((e) => { console.error('[association-desktop-cdp] FAIL:', e.message); process.exitCode = 1; });
