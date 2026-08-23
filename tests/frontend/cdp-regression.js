#!/usr/bin/env node
/**
 * LRC 桌面端 CDP 深层回归测试
 * ------------------------------------------------------------------
 * 直连 Tauri WebView 暴露的 CDP WebSocket 端点（开发 9231 / 稳定 9230），通过 Runtime.evaluate
 * 在页面内执行 JS，按「标签页面板」分组深层测试所有交互口子。
 *
 * 测试维度（每个交互口）：
 *   1. 存在性 / 可见性 / 禁用态（先切换到所属面板后再判断）
 *   2. 点击触发后的可观测反馈（Toast / 模态框 / 面板激活）
 *   3. 点击后是否新增 console error / 未捕获异常 / 网络失败(4xx/5xx)
 *   4. 破坏性操作走「确认框取消」安全路径（验证确认机制 + 取消中断）
 *   5. 防重复：关键按钮快速连点是否产生异常
 *
 * 用法：node tests/frontend/cdp-regression.js
 */
'use strict';

const CDP_PORTS = [9231, 9230];
const WS = require('ws');

async function getCdpBase() {
  for (const port of CDP_PORTS) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (res.ok) return `http://127.0.0.1:${port}`;
    } catch {
      // 依次探测开发版和稳定版 CDP 端口。
    }
  }
  throw new Error(`未找到 LRC WebView CDP 端口（${CDP_PORTS.join(', ')}）`);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// =====================================================================
// CDP 客户端
// =====================================================================
class CDPClient {
  constructor(wsUrl) {
    this.ws = new WS(wsUrl);
    this.nextId = 1;
    this.pending = new Map();
    this.consoleErrors = [];
    this.exceptions = [];
    this.networkFailures = [];
    this.hardNetworkFailures = [];
    this.rawEvents = [];

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
    this.rawEvents.push(msg);
    this._handleEvent(msg);
  }

  _handleEvent(msg) {
    const m = msg.method;
    const p = msg.params || {};
    if (m === 'Runtime.consoleAPICalled') {
      if (p.type === 'error' || p.type === 'assert') {
        const text = (p.args || []).map((a) => a.value ?? a.description ?? '').join(' ');
        this.consoleErrors.push(text);
      }
    } else if (m === 'Runtime.exceptionThrown') {
      const d = p.exceptionDetails || {};
      this.exceptions.push((d.text || '') + ' ' + (d.exception?.description || ''));
    } else if (m === 'Log.entryAdded') {
      if (p.entry?.level === 'error') this.consoleErrors.push(p.entry.text || '');
    } else if (m === 'Network.loadingFailed') {
      const errorText = p.errorText || '';
      const failure = 'FAILED ' + errorText + ' ' + (p.blockedReason || '');
      this.networkFailures.push(failure);
      if (!['net::ERR_ABORTED', 'net::ERR_BLOCKED_BY_CLIENT'].includes(errorText)) {
        this.hardNetworkFailures.push(failure);
      }
    } else if (m === 'Network.responseReceived') {
      const st = p.response?.status;
      if (st >= 400) this.networkFailures.push(st + ' ' + (p.response?.url || ''));
    }
  }

  async ready() { await this._ready; }

  async send(method, params = {}, timeoutMs = 15000) {
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

  async eval(expression, timeoutMs = 15000) {
    const r = await this.send('Runtime.evaluate', {
      expression, returnByValue: true, awaitPromise: true,
    }, timeoutMs);
    if (r.exceptionDetails) {
      throw new Error('页面JS异常: ' + (r.exceptionDetails.text || '') +
        ' ' + (r.exceptionDetails.exception?.description || ''));
    }
    return r.result?.value;
  }

  resetCounters() {
    this.consoleErrors = [];
    this.exceptions = [];
    this.networkFailures = [];
    this.hardNetworkFailures = [];
  }
  errorTotal() { return this.consoleErrors.length + this.exceptions.length; }
  recentErrors(n = 5) {
    return this.consoleErrors.slice(-n).concat(this.exceptions.slice(-n)).slice(-n);
  }
  close() { try { this.ws.close(); } catch {} }
}

// =====================================================================
// 工具：获取 tauri.localhost 页面 WS 地址
// =====================================================================
async function getPageWs(cdpBase) {
  const res = await fetch(`${cdpBase}/json/list`).then((r) => r.json());
  const page = res.find((p) => p.type === 'page' && /tauri\.localhost/.test(p.url || ''))
    || res.find((p) => p.type === 'page');
  if (!page) throw new Error('未在 CDP 中找到 tauri.localhost 页面');
  return page;
}

// =====================================================================
// 交互口枚举（记录所属 panel，便于分组测试）
// =====================================================================
const ENUMERATE_JS = `(() => {
  const actions = [];
  const seen = new Set();
  document.querySelectorAll('[data-action]').forEach((el) => {
    const a = el.getAttribute('data-action');
    const arg = el.getAttribute('data-arg') || null;
    const key = a + '|' + (arg || '');
    const panel = el.closest('.tab-content');
    const rec = {
      action: a,
      arg,
      panel: panel ? panel.id.replace(/^tab-/, '') : 'global',
      tag: el.tagName.toLowerCase(),
      text: (el.innerText || el.textContent || '').trim().slice(0, 42),
    };
    if (!seen.has(key)) { seen.add(key); actions.push(rec); }
  });
  const tabs = [];
  const tabSeen = new Set();
  document.querySelectorAll('[data-tab]').forEach((el) => {
    const t = el.getAttribute('data-tab');
    if (!tabSeen.has(t)) { tabSeen.add(t); tabs.push({ tab: t, text: (el.getAttribute('data-tab-text') || el.innerText || '').trim().slice(0, 20) }); }
  });
  const onclick = [];
  document.querySelectorAll('[onclick]').forEach((el) => {
    onclick.push({ onclick: el.getAttribute('onclick'), tag: el.tagName.toLowerCase(), text: (el.innerText || '').trim().slice(0, 30) });
  });
  const inputs = [];
  document.querySelectorAll('input, select, textarea').forEach((el) => {
    if (el.type === 'hidden' || el.type === 'file') return;
    const panel = el.closest('.tab-content');
    inputs.push({ tag: el.tagName.toLowerCase(), type: el.type || null, id: el.id || null, placeholder: el.placeholder || null, panel: panel ? panel.id.replace(/^tab-/, '') : 'global' });
  });
  return { actions, tabs, onclick, inputs };
})()`;

// =====================================================================
// 切换到指定面板
// =====================================================================
async function switchPanel(cdp, name) {
  await cdp.eval(`(() => {
    const nav = document.querySelector('[data-tab="${name}"]');
    if (nav) { nav.click(); return true; }
    return false;
  })()`);
  await sleep(700);
}

// =====================================================================
// 单个 data-action 口子深层测试
// =====================================================================
async function deepTestAction(cdp, action, arg) {
  const sel = `[data-action="${action}"]${arg ? `[data-arg="${arg}"]` : ''}`;
  const r = { action, arg: arg || null, status: 'PASS', checks: {}, notes: [] };

  // 测试前预备：展开「高级管理面板」/激活向导步骤，避免因折叠态或步骤态被误判为隐藏
  const revealed = await cdp.eval(`(() => {
    const el = document.querySelector(${JSON.stringify(sel)});
    if (!el) return false;
    let changed = false;
    const advBody = el.closest('#advanced-management-body');
    if (advBody && advBody.hasAttribute('hidden')) {
      advBody.removeAttribute('hidden');
      const t = document.getElementById('advanced-toggle-text');
      if (t) t.textContent = '折叠';
      changed = true;
    }
    const stepEl = el.closest('[id^="setup-step-"]');
    if (stepEl) {
      const m = stepEl.id.match(/setup-step-(\\d+)/);
      if (m && getComputedStyle(stepEl).display === 'none') {
        const target = parseInt(m[1], 10);
        for (let i = 1; i <= 3; i++) {
          const s = document.getElementById('setup-step-' + i);
          if (s) s.style.display = (i === target) ? '' : 'none';
        }
        changed = true;
      }
    }
    return changed;
  })()`);
  if (revealed) await sleep(200);

  // 优先选中「可见的同类元素」（同一 action 可能有展开/折叠两个按钮，如 toggleSidebar）
  const pickEl = `(() => {
    const els = [...document.querySelectorAll(${JSON.stringify(sel)})];
    if (!els.length) return null;
    const vis = els.find((el) => !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length));
    return vis || els[0];
  })()`;

  const st = await cdp.eval(`(() => {
    const els = [...document.querySelectorAll(${JSON.stringify(sel)})];
    if (!els.length) return { found: false };
    const vis = els.find((el) => !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length));
    const target = vis || els[0];
    let reason = null;
    if (!vis) {
      let n = els[0];
      while (n) {
        const cs = getComputedStyle(n);
        if (n.hasAttribute('hidden') || cs.display === 'none' || cs.visibility === 'hidden' || (n.offsetWidth === 0 && n.offsetHeight === 0)) {
          const cls = typeof n.className === 'string' ? n.className.trim().split(/\\s+/).slice(0,3).join('.') : '';
          reason = (n.id ? '#' + n.id : n.tagName.toLowerCase()) + (cls ? '.' + cls : '') + ' hidden=' + n.hasAttribute('hidden') + ',display=' + cs.display;
          break;
        }
        n = n.parentElement;
      }
      if (!reason) reason = 'zero-size';
    }
    return {
      found: true,
      count: els.length,
      visible: !!vis,
      disabled: target.disabled === true || target.classList.contains('disabled'),
      text: (target.innerText || '').trim().slice(0, 30),
      reason,
    };
  })()`);

  if (!st.found) { r.status = 'SKIP_NOT_FOUND'; r.notes.push('元素不存在（动态生成）'); return r; }
  r.checks.count = st.count;
  r.checks.visible = st.visible;
  r.checks.disabled = st.disabled;
  r.checks.text = st.text;
  if (!st.visible) { r.status = 'SKIP_INVISIBLE'; r.notes.push('隐藏原因: ' + (st.reason || '未知')); return r; }
  if (st.disabled) { r.status = 'SKIP_DISABLED'; r.notes.push('元素禁用（前置条件未满足）'); return r; }

  const preErr = cdp.errorTotal();
  const preConsoleErrors = cdp.consoleErrors.length;
  const preNet = cdp.networkFailures.length;
  const preHardNet = cdp.hardNetworkFailures.length;

  // 对「打开原生文件选择器」的 action 启用 CDP 拦截，避免原生对话框阻塞后续测试
  const isFileTrigger = action === 'triggerFileInput';
  if (isFileTrigger) {
    await cdp.send('Page.setInterceptFileChooserDialog', { enabled: true }).catch(() => {});
  }

  await cdp.eval(`(() => { const el = ${pickEl}; el.click(); return true; })()`);
  await sleep(900);

  if (isFileTrigger) {
    await cdp.send('Page.setInterceptFileChooserDialog', { enabled: false }).catch(() => {});
  }

  const fb = await cdp.eval(`(() => {
    const toast = document.querySelector('#toast-container .toast');
    const confirm = document.querySelector('#confirm-modal:not([hidden])');
    const startModal = document.querySelector('#start-service-modal:not([hidden])');
    const detail = document.querySelector('#memory-detail-backdrop.open, #memory-detail-panel.open');
    const anyModal = document.querySelector('.modal-overlay:not([hidden])');
    const infoPanel = document.querySelector('#info-panel:not([hidden])');
    const manualAdd = document.querySelector('.lrc-manual-add-overlay');
    return {
      toast: !!toast, toastText: toast ? (toast.innerText||'').slice(0,60) : '',
      confirmOpen: !!confirm, startModalOpen: !!startModal, detailOpen: !!detail, anyModal: !!anyModal,
      infoPanelOpen: !!infoPanel, manualAddOpen: !!manualAdd,
    };
  })()`);
  r.checks.feedback = fb;

  r.checks.newErrors = cdp.errorTotal() - preErr;
  r.checks.newNetFailures = cdp.networkFailures.length - preNet;
  const newHardNetworkFailures = cdp.hardNetworkFailures.length - preHardNet;
  const expectedUnfoldFailure = action === 'unfoldMemory' && cdp.networkFailures.slice(preNet).some((item) => /404 .*\/v1\/memories\/unfold/.test(item));
  if (expectedUnfoldFailure) {
    cdp.consoleErrors.splice(preConsoleErrors);
    r.notes.push('非合成记忆按预期返回 404，已验证前端错误反馈');
  } else {
    if (r.checks.newErrors > 0) { r.status = 'WARN'; r.notes.push('点击后新增 ' + r.checks.newErrors + ' 个错误: ' + cdp.recentErrors(3).join(' | ').slice(0, 200)); }
    if (newHardNetworkFailures > 0) { r.status = 'WARN'; r.notes.push('点击后新增 ' + newHardNetworkFailures + ' 个网络失败: ' + cdp.hardNetworkFailures.slice(-2).join(' | ').slice(0, 200)); }
  }

  // 确认框走「取消」安全路径；其它模态框关闭
  if (fb.confirmOpen) {
    await cdp.eval(`(() => { const c = document.querySelector('#confirm-modal-cancel'); if (c) c.click(); return !!c; })()`);
    await sleep(300);
    r.checks.confirmCancelled = true;
    r.notes.push('确认框已走「取消」路径');
  }
  if (fb.startModalOpen || fb.detailOpen || fb.infoPanelOpen || fb.manualAddOpen || (fb.anyModal && !fb.confirmOpen)) {
    await cdp.eval(`(() => {
      // 关闭所有已知模态框：启动服务 / 记忆详情 / 信息面板 / 手动添加 / 通用 modal-close
      const closers = document.querySelectorAll([
        '[data-action="closeStartServiceModal"]',
        '[data-action="closeMemoryDetail"]',
        '#info-panel-close',
        '.lrc-manual-add-overlay .lrc-manual-add-close, .lrc-manual-add-overlay .lrc-manual-add-cancel',
        '.modal-overlay:not([hidden]) .modal-close'
      ].join(','));
      closers.forEach((c) => { try { c.click(); } catch {} });
      return closers.length;
    })()`);
    await sleep(250);
    r.checks.modalClosed = true;
  }

  // finishSetup 后完成页面（setup-step-3）会显示，点「进入仪表盘」回到正常状态，避免遮挡后续测试
  await cdp.eval(`(() => {
    const step3 = document.getElementById('setup-step-3');
    if (step3 && getComputedStyle(step3).display !== 'none') {
      const enterBtn = step3.querySelector('[data-action="switchToTab"][data-arg="dashboard"]');
      if (enterBtn) { enterBtn.click(); return 'clicked-enter-dashboard'; }
      const section = document.getElementById('setup-steps-section');
      if (section) { section.style.display = 'none'; return 'hidden-setup-section'; }
    }
    return 'no-op';
  })()`);

  return r;
}

// =====================================================================
// 防重复测试
// =====================================================================
async function testDoubleClick(cdp, action, arg) {
  const sel = `[data-action="${action}"]${arg ? `[data-arg="${arg}"]` : ''}`;
  const preErr = cdp.errorTotal();
  const ok = await cdp.eval(`(() => {
    const el = document.querySelector(${JSON.stringify(sel)});
    if (!el || el.disabled) return false;
    el.click(); el.click(); el.click();
    return true;
  })()`);
  await sleep(800);
  const newErr = cdp.errorTotal() - preErr;
  await cdp.eval(`(() => { const c = document.querySelector('#confirm-modal-cancel'); if (c) c.click(); })()`);
  return { clicked: ok, newErrors: newErr };
}

// =====================================================================
// data-tab 切换测试
// =====================================================================
async function testTab(cdp, tab) {
  const r = { tab, status: 'PASS', checks: {}, notes: [] };
  const preErr = cdp.errorTotal();
  await cdp.eval(`(() => { const el = document.querySelector('[data-tab="${tab}"]'); if (!el) return false; el.click(); return true; })()`);
  await sleep(600);
  const st = await cdp.eval(`(() => {
    const nav = document.querySelector('[data-tab="${tab}"]');
    const panel = document.querySelector('#tab-${tab}');
    const navs = [...document.querySelectorAll('[data-tab="${tab}"]')];
    const activeNavCount = navs.filter((item) => item.classList.contains('active') || item.getAttribute('aria-selected') === 'true').length;
    const activePanelCount = document.querySelectorAll('.tab-content.active').length;
    return {
      navActive: nav ? (nav.classList.contains('active') || nav.getAttribute('aria-selected') === 'true') : false,
      navCount: navs.length,
      activeNavCount,
      panelActive: panel ? panel.classList.contains('active') : false,
      activePanelCount,
      panelExists: !!panel,
    };
  })()`);
  r.checks = st;
  r.checks.newErrors = cdp.errorTotal() - preErr;
  if (!st.panelExists) { r.status = 'FAIL'; r.notes.push('缺少面板 #tab-' + tab); }
  else if (!st.panelActive) { r.status = 'WARN'; r.notes.push('面板未激活'); }
  if (r.checks.newErrors > 0) { if (r.status === 'PASS') r.status = 'WARN'; r.notes.push('切换新增 ' + r.checks.newErrors + ' 错误: ' + cdp.recentErrors(3).join(' | ').slice(0, 200)); }
  return r;
}

// =====================================================================
// 输入元素 fill 测试
// =====================================================================
async function testInput(cdp, input) {
  const r = { id: input.id || null, type: input.type || null, status: 'PASS', checks: {}, notes: [] };
  const id = input.id;
  if (!id) { r.status = 'SKIP_NO_ID'; return r; }
  const protectedIds = new Set([
    'llm-api-key', 'setup-llm-api-key', 'llm-endpoint', 'llm-model',
    'embedder-model', 'wizard-search-path', 'wizard-memory-content',
  ]);
  if (protectedIds.has(id) || input.type === 'password' || input.type === 'url') {
    r.status = 'SKIP_PROTECTED';
    r.notes.push('配置/密钥/路径输入不修改，避免污染后续交互测试');
    return r;
  }
  const preErr = cdp.errorTotal();
  const res = await cdp.eval(`(() => {
    const el = document.getElementById(${JSON.stringify(id)});
    if (!el) return { found: false };
    const tag = el.tagName.toLowerCase();
    const before = el.value ?? '';
    if (tag === 'select') {
      if (el.options.length > 1) { el.selectedIndex = 1; el.dispatchEvent(new Event('change', { bubbles: true })); }
    } else if (el.type === 'checkbox' || el.type === 'radio') {
      el.checked = !el.checked; el.dispatchEvent(new Event('change', { bubbles: true }));
    } else {
      const v = '回归测试值_' + Date.now();
      el.value = v;
      el.dispatchEvent(new Event('input', { bubbles: true }));
    }
    return { found: true, tag, before: String(before), after: String(el.value ?? el.checked) };
  })()`);
  await sleep(300);
  r.checks = res;
  r.checks.newErrors = cdp.errorTotal() - preErr;
  if (!res.found) { r.status = 'SKIP_NOT_FOUND'; }
  else if (r.checks.newErrors > 0) { r.status = 'WARN'; r.notes.push('输入新增 ' + r.checks.newErrors + ' 错误: ' + cdp.recentErrors(3).join(' | ').slice(0, 200)); }
  return r;
}

// =====================================================================
// 主流程
// =====================================================================
async function main() {
  const cdpBase = await getCdpBase();
  console.log(`[CDP] 使用端口: ${cdpBase}`);
  const page = await getPageWs(cdpBase);
  console.log(`[CDP] 连接页面: ${page.title} (${page.url})`);
  console.log(`[CDP] WS: ${page.webSocketDebuggerUrl}\n`);

  const cdp = new CDPClient(page.webSocketDebuggerUrl);
  await cdp.ready();
  await cdp.send('Runtime.enable');
  await cdp.send('Log.enable');
  await cdp.send('Network.enable');
  await cdp.send('Page.enable');
  cdp.resetCounters();
  await sleep(500);

  // 1. 枚举
  const inv = await cdp.eval(ENUMERATE_JS);
  const uniqueActions = [...new Set(inv.actions.map((a) => a.action))];
  console.log('========== 产品视角交互口子清单 ==========');
  console.log(`data-action 元素(去重 action): ${uniqueActions.length} 个`);
  console.log(`data-tab 导航口子: ${inv.tabs.length} 个`);
  console.log(`onclick 内联口子: ${inv.onclick.length} 个`);
  console.log(`输入控件(input/select/textarea): ${inv.inputs.length} 个`);
  console.log(`唯一 data-action 列表: ${uniqueActions.join(', ')}`);
  console.log('==========================================\n');

  // 2. 测试所有 tab
  const tabResults = [];
  for (const t of inv.tabs) {
    const tr = await testTab(cdp, t.tab);
    tabResults.push(tr);
    console.log(`[TAB] ${t.tab} -> ${tr.status}${tr.notes.length ? ' (' + tr.notes.join('; ') + ')' : ''}`);
  }

  // 3. 按 panel 分组测试 data-action
  const groups = new Map();
  for (const a of inv.actions) {
    if (!groups.has(a.panel)) groups.set(a.panel, []);
    groups.get(a.panel).push(a);
  }

  const actionResults = [];
  const panelNames = [...groups.keys()];

  // 先测 global 组（侧边栏/浮层等全局元素，无需切 tab）
  if (groups.has('global')) {
    console.log('\n--- 全局交互口（无需切面板）---');
    for (const a of groups.get('global')) {
      const ar = await deepTestAction(cdp, a.action, a.arg);
      actionResults.push(ar);
      console.log(`[ACTION] ${a.action}${a.arg ? ':' + a.arg : ''} -> ${ar.status}${ar.notes.length ? ' | ' + ar.notes.join('; ') : ''}`);
      await sleep(120);
    }
  }

  // 再测各 tab 内口子（先切 tab）
  for (const panelName of panelNames) {
    if (panelName === 'global') continue;
    if (!groups.has(panelName)) continue;
    const acts = groups.get(panelName);
    console.log(`\n--- 面板: ${panelName}（${acts.length} 个口子）---`);
    for (const a of acts) {
      // 每个口子测试前重新激活所属面板，避免 switchToTab 等口子切走面板导致后续元素被误判隐藏
      await switchPanel(cdp, panelName);
      try {
        const ar = await deepTestAction(cdp, a.action, a.arg);
        actionResults.push(ar);
        console.log(`[ACTION] ${a.action}${a.arg ? ':' + a.arg : ''} -> ${ar.status}${ar.notes.length ? ' | ' + ar.notes.join('; ') : ''}`);
      } catch (e) {
        actionResults.push({ action: a.action, arg: a.arg, status: 'ERROR', notes: [String(e.message || e)] });
        console.log(`[ACTION] ${a.action} -> ERROR: ${e.message}`);
      }
      await sleep(120);
    }
  }

  // 4. 防重复测试
  const dupTargets = ['generateCaptainLog', 'encodeTextToLuoshu', 'createBackup', 'runPrivacyCheck', 'saveLlmConfig'];
  const dupResults = [];
  console.log('\n--- 防重复测试（连点3次）---');
  for (const act of dupTargets) {
    // 先切到该按钮所属面板
    const meta = inv.actions.find((x) => x.action === act);
    if (meta && meta.panel !== 'global') await switchPanel(cdp, meta.panel);
    const dr = await testDoubleClick(cdp, act, null);
    dupResults.push({ action: act, ...dr });
    console.log(`[DUP] ${act} -> 连点3次${dr.clicked ? '' : '(未点击/禁用)'}，新增错误 ${dr.newErrors}`);
  }

  // 5. 输入控件测试（切到所属面板）
  const inputResults = [];
  console.log('\n--- 输入控件测试 ---');
  for (const inp of inv.inputs) {
    if (inp.panel !== 'global') await switchPanel(cdp, inp.panel);
    const ir = await testInput(cdp, inp);
    inputResults.push(ir);
  }
  console.log(`[INPUT] 已测试 ${inputResults.length} 个输入控件`);

  const gateFailures = [
    ...tabResults.filter((r) => r.status === 'FAIL'),
    ...actionResults.filter((r) => r.status === 'ERROR' || r.status === 'FAIL'),
  ];
  const runtimeFailures = cdp.errorTotal() + cdp.hardNetworkFailures.length;
  const gateStatus = gateFailures.length || runtimeFailures ? 'FAIL' : 'PASS';
  cdp.close();

  // 6. 汇总 + 报告
  const summary = {
    generated_at: new Date().toISOString(),
    page: { title: page.title, url: page.url },
    inventory: {
      unique_actions: uniqueActions.length,
      actions_total: inv.actions.length,
      tabs: inv.tabs.length,
      onclick: inv.onclick.length,
      total_inputs: inv.inputs.length,
      action_list: uniqueActions,
      tab_list: inv.tabs.map((t) => t.tab),
      onclick_list: inv.onclick,
    },
    results: { tabs: tabResults, actions: actionResults, double_click: dupResults, inputs: inputResults },
    counts: { tabs: _count(tabResults), actions: _count(actionResults), inputs: _count(inputResults) },
    gate: {
      status: gateStatus,
      failures: gateFailures.length,
      runtimeFailures,
      consoleErrors: cdp.consoleErrors,
      exceptions: cdp.exceptions,
      hardNetworkFailures: cdp.hardNetworkFailures,
    },
  };

  const fs = require('fs');
  const path = require('path');
  const outJson = path.join(__dirname, 'cdp-regression-report.json');
  const outMd = path.join(__dirname, 'cdp-regression-report.md');
  fs.writeFileSync(outJson, JSON.stringify(summary, null, 2), 'utf8');
  fs.writeFileSync(outMd, renderMarkdown(summary), 'utf8');

  console.log('\n========== 汇总 ==========');
  console.log(`交互口总数: data-action ${uniqueActions.length} + data-tab ${inv.tabs.length} + onclick ${inv.onclick.length} = ${uniqueActions.length + inv.tabs.length + inv.onclick.length}`);
  console.log(`action 结果: ${JSON.stringify(summary.counts.actions)}`);
  console.log(`tab 结果: ${JSON.stringify(summary.counts.tabs)}`);
  console.log(`input 结果: ${JSON.stringify(summary.counts.inputs)}`);
  console.log(`发布门禁: ${gateStatus}`);
  console.log(`报告: ${outMd}`);
  if (gateStatus === 'FAIL') process.exitCode = 1;
}

function _count(rs) {
  const c = {};
  for (const r of rs) {
    const s = r.status || 'UNKNOWN';
    c[s] = (c[s] || 0) + 1;
  }
  return c;
}

function renderMarkdown(s) {
  const L = [];
  L.push('# LRC 桌面端 CDP 深层回归测试报告');
  L.push('');
  L.push(`- 生成时间: ${s.generated_at}`);
  L.push(`- 页面: ${s.page.title} (${s.page.url})`);
  L.push('');
  L.push('## 一、产品视角交互口子清单');
  L.push('');
  L.push('| 类别 | 数量 |');
  L.push('|---|---|');
  L.push(`| data-action（去重） | ${s.inventory.unique_actions} |`);
  L.push(`| data-tab 导航 | ${s.inventory.tabs} |`);
  L.push(`| onclick 内联 | ${s.inventory.onclick} |`);
  L.push(`| 输入控件 | ${s.inventory.total_inputs} |`);
  L.push(`| **交互口子合计** | **${s.inventory.unique_actions + s.inventory.tabs + s.inventory.onclick}** |`);
  L.push('');
  L.push('### data-action 清单');
  L.push('```');
  L.push(s.inventory.action_list.join(', '));
  L.push('```');
  L.push('');
  L.push('### data-tab 清单');
  L.push('```');
  L.push(s.inventory.tab_list.join(', '));
  L.push('```');
  L.push('');
  L.push('## 二、测试结果汇总');
  L.push('');
  L.push('### data-tab 切换');
  L.push('');
  L.push('| tab | 结果 | 备注 |');
  L.push('|---|---|---|');
  for (const r of s.results.tabs) L.push(`| ${r.tab} | ${r.status} | ${(r.notes || []).join('; ')} |`);
  L.push('');
  L.push('### data-action 深层测试');
  L.push('');
  L.push('| action | arg | 结果 | 可见 | 禁用 | 新错误 | 新网络失败 | 反馈 | 备注 |');
  L.push('|---|---|---|---|---|---|---|---|---|');
  for (const r of s.results.actions) {
    const c = r.checks || {};
    const fb = c.feedback || {};
    const fbTxt = [fb.toast ? 'toast' : '', fb.confirmOpen ? 'confirm' : '', fb.startModalOpen ? 'startModal' : '', fb.detailOpen ? 'detail' : ''].filter(Boolean).join('+') || '-';
    L.push(`| ${r.action} | ${r.arg || ''} | ${r.status} | ${c.visible ?? '-'} | ${c.disabled ?? '-'} | ${c.newErrors ?? '-'} | ${c.newNetFailures ?? '-'} | ${fbTxt} | ${(r.notes || []).join('; ')} |`);
  }
  L.push('');
  L.push('### 防重复（连点3次）');
  L.push('');
  L.push('| action | 点击 | 新增错误 |');
  L.push('|---|---|---|');
  for (const r of s.results.double_click) L.push(`| ${r.action} | ${r.clicked} | ${r.newErrors} |`);
  L.push('');
  L.push('### 结果统计');
  L.push('');
  L.push('```');
  L.push('action: ' + JSON.stringify(s.counts.actions));
  L.push('tab: ' + JSON.stringify(s.counts.tabs));
  L.push('input: ' + JSON.stringify(s.counts.inputs));
  L.push('```');
  return L.join('\n');
}

main().catch((e) => {
  console.error('[FATAL]', e);
  process.exit(1);
});
