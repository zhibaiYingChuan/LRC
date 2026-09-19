#!/usr/bin/env node
/**
 * 符号层桌面端 CDP 专项验证（2026-09-18）
 *
 * 目标：在真实桌面 WebView（tauri.localhost + CDP 9231）里验证**符号层**的四件事：
 *   S1. relationPlain 的五类符号层关系都走「结构推导：…」文案
 *   S2. 联想中心节点带 `source="symbolic"` 时渲染出「结构推导」徽章，且文案正确
 *   S2c.符号层节点**不戴**「记录关联」徽章（证据性质不得混同）
 *   S3. 符号层节点的说明文案走 `relationPlain` 的「结构推导：…」系列
 *       （而非兜底「由记录推导出的关联」）
 *
 * ★与 association-desktop-cdp.js 的分工：
 *   那个脚本测**记录层**与空态（回归门禁）；本脚本专测**符号层渲染**。
 *
 * ## 前置（必须满足，否则本脚本无法验证）
 *   图里必须有符号层边 ⇒ 需 `LRC_DAOTI_WRITE_BACK=1` 让 /build_edges 落图，
 *   或手工调 `/v1/memories/external-edge` 写一条。
 *   若前置不满足，本脚本会**明确报告"前置不满足"**，
 *   而不是把"没数据"误报成"渲染坏了"。
 *
 * 用法：node tests/frontend/symbolic-layer-desktop-cdp.js
 */
'use strict';

const fs = require('fs');
const path = require('path');
const WS = require('ws');

const CDP_PORTS = [9231, 9230];
const API_BASE = process.env.LRC_API_BASE || 'http://127.0.0.1:3111';
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
    this._ready = new Promise((resolve, reject) => {
      this.ws.on('open', resolve);
      this.ws.on('error', reject);
    });
    this.ws.on('message', (d) => this._onMessage(d));
  }
  _onMessage(data) {
    let msg; try { msg = JSON.parse(data.toString()); } catch { return; }
    if (msg.id && this.pending.has(msg.id)) {
      const { resolve, reject, timer } = this.pending.get(msg.id);
      clearTimeout(timer); this.pending.delete(msg.id);
      if (msg.error) reject(new Error(msg.error.message || JSON.stringify(msg.error)));
      else resolve(msg.result);
    }
  }
  async ready() { await this._ready; }
  send(method, params = {}, timeoutMs = 30000) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.pending.has(id)) { this.pending.delete(id); reject(new Error(`CDP 超时: ${method}`)); }
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
  async eval(expression, timeoutMs = 30000) {
    const r = await this.send('Runtime.evaluate',
      { expression, returnByValue: true, awaitPromise: true }, timeoutMs);
    if (r.exceptionDetails) {
      throw new Error('页面JS异常: ' + (r.exceptionDetails.text || ''));
    }
    return r.result?.value;
  }
}

function record(name, ok, detail) {
  results.push({ name, ok, detail });
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}  ${JSON.stringify(detail)}`);
}

(async () => {
  console.log('='.repeat(74));
  console.log('符号层桌面端 CDP 专项');
  console.log('='.repeat(74));

  // ---------- 前置 0：后端是否真的有符号层边 ----------
  // 这是"能不能验证"的前提。没有它，后面的断言只是在测"空数据渲染"。
  let hasSymbolicEdge = null;
  let probeErr = null;
  try {
    // 用探索接口看是否已有 symbolic 节点（1 个种子足够）
    const r = await fetch(`${API_BASE}/v1/associations/explore`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ query: '联想 状态机', depth: 2, width: 3 }),
      signal: AbortSignal.timeout(120000),
    });
    if (r.ok) {
      const j = await r.json();
      const nodes = Array.isArray(j.nodes) ? j.nodes : [];
      hasSymbolicEdge = nodes.some((n) => n.source === 'symbolic');
      const dist = {};
      nodes.forEach((n) => { dist[n.source] = (dist[n.source] || 0) + 1; });
      console.log(`  后端节点 source 分布: ${JSON.stringify(dist)}`);
    } else {
      probeErr = `explore HTTP ${r.status}`;
    }
  } catch (e) { probeErr = e.message; }

  console.log(`  图里有符号层边(symbolic 节点)? ${hasSymbolicEdge}` +
    (probeErr ? `  (探测出错: ${probeErr})` : ''));
  record('S0_前置_后端存在符号层边', hasSymbolicEdge === true, {
    hasSymbolicEdge, probeErr,
    hint: hasSymbolicEdge === true ? undefined
      : '图里无符号层边 ⇒ 需 LRC_DAOTI_WRITE_BACK=1 落图，或手工调 /v1/memories/external-edge',
  });

  // ---------- CDP 连接 ----------
  const { base, port } = await getCdpBase();
  console.log(`  CDP 端口: ${port}`);
  const list = await (await fetch(`${base}/json/list`)).json();
  const page = list.find((t) => t.type === 'page');
  if (!page) throw new Error('未找到 page 目标');
  console.log(`  页面: ${page.url}`);
  if (!page.url.includes('tauri.localhost')) {
    console.warn(`  [WARN] 页面 origin 非 tauri.localhost（实测 ${page.url}）`);
  }

  const cdp = new CDPClient(page.webSocketDebuggerUrl);
  await cdp.ready();
  await cdp.send('Runtime.enable');
  await cdp.send('Page.enable');

  // ---------- 静态断言：渲染源码里符号层映射存在（不依赖数据） ----------
  // ★为什么要有这组：即使图里暂时没有符号层边，也必须保证"有边时能正确渲染"。
  //   它由 app.js 源码文本直接判定，**不依赖数据**，故一定能验。
  //
  // ★注意：relationPlain 是联想渲染内的闭包，不在 window 上（实测
  //   hasRelationPlain=false），因此不能靠"调函数"验证，改抓源码文本。
  const srcProbe = await cdp.eval(`(async () => {
    try {
      const r = await fetch('/app.js');
      const s = await r.text();
      const rels = ['cause', 'temporal', 'constraint', 'facilitate', 'coordinate'];
      const mapped = {};
      rels.forEach(k => {
        // 匹配 'cause': '结构推导：...' 形式
        const re = new RegExp("'" + k + "'\\\\s*:\\\\s*'([^']*)'");
        const m = s.match(re);
        mapped[k] = m ? m[1] : null;
      });
      return {
        bytes: s.length,
        mapped,
        hasSymbolicTagClass: s.includes('association-symbolic-tag'),
        hasIsSymbolic: s.includes('isSymbolic'),
        hasFallback: s.includes('由记录推导出的关联'),
      };
    } catch (e) { return { err: String(e) }; }
  })()`);
  console.log(`  app.js 源码探针: bytes=${srcProbe.bytes} hasSymbolicTagClass=${srcProbe.hasSymbolicTagClass} hasIsSymbolic=${srcProbe.hasIsSymbolic}`);
  if (srcProbe.err) {
    record('S1_relationPlain_五类符号关系均为「结构推导：」前缀', false, srcProbe);
  } else {
    const allSymbolic = ['cause', 'temporal', 'constraint', 'facilitate', 'coordinate']
      .every((k) => typeof srcProbe.mapped[k] === 'string' && srcProbe.mapped[k].startsWith('结构推导'));
    record('S1_relationPlain_五类符号关系均为「结构推导：」前缀', allSymbolic, srcProbe.mapped);
  }
  record('S1b_渲染层存在符号层专用徽章分支', srcProbe.hasSymbolicTagClass === true && srcProbe.hasIsSymbolic === true,
    { hasSymbolicTagClass: srcProbe.hasSymbolicTagClass, hasIsSymbolic: srcProbe.hasIsSymbolic });

  // ---------- 动态断言：真实探索里若有 symbolic 节点，检查徽章/文案 ----------
  if (hasSymbolicEdge === true) {
    // ★关键：不再固定 sleep。实测 explore 响应耗时 10.7s~13.5s 波动，
    //   曾因"固定等 11s"落在响应到达之前而误读为"UI 渲染时有时无"
    //   （见 temp/daoti_assoc/probes/cdp_timing_probe.js 的时间线证据）。
    //   改为劫持 fetch，轮询"UI 自己那次响应是否已到达"。
    await cdp.eval(`(() => {
      if (window.gotoAssociationCenter) window.gotoAssociationCenter();
      if (!window.__slHooked) {
        window.__slHooked = true;
        window.__slRespAt = null;
        const orig = window.fetch;
        window.fetch = function (...a) {
          const url = String(a[0] && a[0].url ? a[0].url : a[0]);
          if (url.includes('/associations/explore')) { window.__slRespAt = null; }
          const p = orig.apply(this, a);
          if (url.includes('/associations/explore')) {
            p.then(() => { window.__slRespAt = Date.now(); }).catch(() => {});
          }
          return p;
        };
      }
      return true;
    })()`);
    await sleep(500);

    let respElapsed = null;
    let dom = null;
    // 最多重试 3 次：符号层边可能被记录层配额挤占（G5a 已知机理）
    for (let attempt = 1; attempt <= 3; attempt++) {
      await cdp.eval(`(() => {
        window.__slRespAt = null;
        const s = document.getElementById('association-story');
        if (s) s.innerHTML = '';
        const i = document.getElementById('association-query');
        if (i) { i.value = '联想 状态机'; }
        if (window.startAssociationExplore) window.startAssociationExplore();
        return true;
      })()`);
      const t0 = Date.now();
      let arrived = false;
      while (Date.now() - t0 < 60000) {
        const got = await cdp.eval(`(window.__slRespAt != null)`);
        if (got) { arrived = true; respElapsed = Date.now() - t0; break; }
        await sleep(300);
      }
      if (!arrived) { console.log(`  第 ${attempt} 次: 响应 60s 未到达`); continue; }
      await sleep(400); // 渲染是同步的（innerHTML），留一拍读 DOM
      dom = await cdp.eval(`(() => {
        const nodes = [...document.querySelectorAll('#association-story .association-story-node')];
        const symNodes = nodes.filter(n => n.querySelector('.association-symbolic-tag'));
        const symTags = [...document.querySelectorAll('#association-story .association-symbolic-tag')];
        const recTags = [...document.querySelectorAll('#association-story .association-record-tag')];
        return {
          nodeCount: nodes.length,
          symbolicTagCount: symTags.length,
          symbolicTagText: symTags.map(e => (e.textContent || '').trim()).slice(0, 3),
          recordTagCount: recTags.length,
          symbolicNodes: symNodes.map(n => ({
            reason: (n.querySelector('.text-sm.text-dim')?.textContent || '').trim(),
            hasRecordTag: !!n.querySelector('.association-record-tag'),
          })),
        };
      })()`);
      console.log(`  第 ${attempt} 次: 响应到达 ${respElapsed}ms，符号层徽章=${dom.symbolicTagCount}`);
      if (dom.symbolicTagCount > 0) break;
    }

    await (async () => {
      try {
        const r = await cdp.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true });
        fs.mkdirSync(artifactDir, { recursive: true });
        fs.writeFileSync(path.join(artifactDir, 'symbolic-layer-desktop.png'),
          Buffer.from(r.data, 'base64'));
      } catch (e) { console.error('  截图失败: ' + e.message); }
    })();

    const domOk = dom && dom.symbolicTagCount > 0;
    record('S2a_符号层节点渲染出「结构推导」徽章', domOk, dom || {});
    record('S2b_徽章文案为「结构推导」',
      domOk && dom.symbolicTagText.every((t) => t === '结构推导'), { texts: dom?.symbolicTagText });
    record('S2c_符号层节点不戴「记录关联」徽章',
      domOk && dom.symbolicNodes.every((n) => n.hasRecordTag === false), { nodes: dom?.symbolicNodes });
    record('S3_符号层节点说明文案走「结构推导：…」而非兜底',
      domOk && dom.symbolicNodes.every((n) => n.reason.startsWith('结构推导：') && !n.reason.startsWith('由记录推导出的关联')),
      { nodes: dom?.symbolicNodes });
  } else {
    console.log('  （无符号层边 ⇒ 跳过动态徽章断言，S0 已如实报告前置不满足）');
  }

  const failed = results.filter((x) => !x.ok);
  console.log('\n' + '='.repeat(74));
  console.log(`共 ${results.length} 项，通过 ${results.length - failed.length}，失败 ${failed.length}`);
  fs.mkdirSync(artifactDir, { recursive: true });
  fs.writeFileSync(path.join(artifactDir, 'symbolic-layer-desktop-cdp-result.json'),
    JSON.stringify({ results }, null, 2));
  cdp.ws.close();
  if (failed.length) process.exitCode = 1;
})().catch((e) => {
  console.error('[symbolic-layer-desktop-cdp] FAIL:', e.message);
  process.exitCode = 1;
});
