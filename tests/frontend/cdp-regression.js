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
// v0.9.6 修复：captain-log/benchmarks/api-docs/project-switch 等快捷入口
// 页面没有 data-tab 导航元素（仅 settings 内的 switchToTab 按钮），
// 原实现只点击 [data-tab] 导致切换失败、面板元素被误判为不可见直接跳过。
// 现在依次尝试：data-tab 导航 → switchToTab 快捷入口按钮 → window.switchTab，
// 并验证 #tab-{name} 已激活，未激活时强制用 switchTab 兜底。
// =====================================================================
async function switchPanel(cdp, name) {
  const r = await cdp.eval(`(() => {
    // 1. 优先原生 data-tab 导航（侧边栏/顶部 nav/移动端 tabbar）
    const nav = document.querySelector('[data-tab="${name}"]');
    if (nav) { nav.click(); return 'nav'; }
    // 2. 快捷入口按钮（settings 面板内的 switchToTab）
    const quick = document.querySelector('[data-action="switchToTab"][data-arg="${name}"]');
    if (quick) { quick.click(); return 'quick'; }
    // 3. 兜底：直接调用 switchTab
    if (typeof window.switchTab === 'function') { window.switchTab('${name}'); return 'switchTab'; }
    return 'none';
  })()`);
  await sleep(700);
  // 验证目标面板已激活；未激活则强制用 switchTab 兜底（等 loader 完成）
  const active = await cdp.eval(`(() => {
    const panel = document.getElementById('tab-${name}');
    return !!(panel && panel.classList.contains('active'));
  })()`);
  if (!active) {
    await cdp.eval(`(() => { if (typeof window.switchTab === 'function') window.switchTab('${name}'); return true; })()`);
    await sleep(700);
  }
  return r;
}

// =====================================================================
// 单个 data-action 口子深层测试
// =====================================================================

// v0.9.7：状态前置开启——8 个口子因「正常状态机」默认隐藏（横幅仅在服务
// 停止时出现、向导仅在特定引导状态出现、弹窗按钮仅在弹窗打开时存在、
// 导航折叠按钮仅在窄屏出现）。SKIP 不等于验证，测试框架先合法开启前置
// 状态再真实点击，测完恢复现场。
const PREREQ_OPENERS = {
  handleStartServiceClick: { kind: 'show-banner' },
  closeStartServiceModal: { kind: 'open-start-service-modal' },
  openQuickAdd: { kind: null },
  closeQuickAddModal: { kind: 'open-quick-add' },
  submitQuickAddMemory: { kind: 'open-quick-add' },
  wizardStep1Search: { kind: 'show-wizard' },
  wizardStep2Write: { kind: 'show-wizard' },
  wizardStep3Search: { kind: 'show-wizard' },
  toggleNav: { kind: 'narrow-viewport' },
  toggleSidebar: { kind: 'sidebar-toggle-state' },
};

async function openPrereqState(cdp, action) {
  const cfg = PREREQ_OPENERS[action];
  if (!cfg) return null;
  if (cfg.kind === 'show-banner') {
    return await cdp.eval(`(() => { const b = document.getElementById('sidecar-down-banner'); if (!b) return 'missing'; b.hidden = false; return 'banner-shown'; })()`);
  }
  if (cfg.kind === 'open-start-service-modal') {
    // 通过正常入口打开启动服务弹窗（服务在线时 handleStartServiceClick 会直接
    // 短路返回并隐藏横幅，因此测试用底层打开函数或直接去 hidden）
    return await cdp.eval(`(() => { const m = document.getElementById('start-service-modal'); if (!m) return 'missing'; m.hidden = false; return 'modal-shown'; })()`);
  }
  if (cfg.kind === 'open-quick-add') {
    const btn = await cdp.eval(`(() => { const b = document.querySelector('[data-action="openQuickAdd"]'); if (b) { b.click(); return true; } return false; })()`);
    if (btn) { await sleep(600); return 'quick-add-opened'; }
    return await cdp.eval(`(() => { const m = document.getElementById('quick-add-modal'); if (!m) return 'missing'; m.hidden = false; return 'modal-shown'; })()`);
  }
  if (cfg.kind === 'show-wizard') {
    return await cdp.eval(`(() => { const w = document.getElementById('quickstart-wizard'); if (!w) return 'missing'; w.hidden = false; w.style.display = ''; return 'wizard-shown'; })()`);
  }
  if (cfg.kind === 'narrow-viewport') {
    await cdp.send('Emulation.setDeviceMetricsOverride', { width: 420, height: 800, deviceScaleFactor: 1, mobile: true }).catch(() => {});
    await sleep(400);
    return 'narrow';
  }
  if (cfg.kind === 'sidebar-toggle-state') {
    // toggleSidebar 有两个互斥入口（展开态的 collapse-btn / 折叠态的 expand-btn）。
    // 若当前无可见入口，点一次可见的同类按钮切换状态，使目标入口可见。
    return await cdp.eval(`(() => {
      const els = [...document.querySelectorAll('[data-action="toggleSidebar"]')];
      const vis = els.find((el) => el.offsetWidth || el.offsetHeight);
      if (vis) return 'already-visible';
      // 两个都不可见说明状态机异常；尝试直接调用切换函数恢复
      const sb = document.querySelector('.app-sidebar');
      if (!sb) return 'missing';
      sb.classList.toggle('collapsed');
      return 'state-forced';
    })()`);
  }
  return null;
}

async function closePrereqState(cdp, action) {
  const cfg = PREREQ_OPENERS[action];
  if (!cfg) return;
  if (cfg.kind === 'show-banner') {
    await cdp.eval(`(() => { const b = document.getElementById('sidecar-down-banner'); if (b) b.hidden = true; return 1; })()`);
    // 失败点击会把 SidecarHealthMonitor 置为不可达（degraded-mode），污染后续
    // 元素可见性。恢复现场：触发一次真实健康检查，让状态机回到实际值。
    await cdp.eval(`(() => { try { if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor && SidecarHealthMonitor.check) SidecarHealthMonitor.check(); } catch (_) {} return 1; })()`);
    await sleep(1200);
  } else if (cfg.kind === 'open-start-service-modal') {
    await cdp.eval(`(() => { const m = document.getElementById('start-service-modal'); if (m) m.hidden = true; return 1; })()`);
  } else if (cfg.kind === 'open-quick-add') {
    await cdp.eval(`(() => { const m = document.getElementById('quick-add-modal'); if (m) m.hidden = true; return 1; })()`);
  } else if (cfg.kind === 'show-wizard') {
    await cdp.eval(`(() => { const w = document.getElementById('quickstart-wizard'); if (w) w.hidden = true; return 1; })()`);
  } else if (cfg.kind === 'narrow-viewport') {
    await cdp.send('Emulation.clearDeviceMetricsOverride').catch(() => {});
    await sleep(300);
  } else if (cfg.kind === 'sidebar-toggle-state') {
    // 恢复侧边栏为展开态（默认状态），保证后续面板布局稳定
    await cdp.eval(`(() => { const sb = document.querySelector('.app-sidebar'); if (sb) sb.classList.remove('collapsed'); return 1; })()`);
    await sleep(300);
  }
}

// v0.9.7 审查修复：deepTestAction 中途抛异常（如 CDP eval 15s 超时）时，
// 已开启的前置状态（窄屏模拟/强开弹窗/横幅/向导）不会恢复，会级联污染
// 后续所有元素的可见性判定。包装层保证 finally 恢复现场。
async function deepTestAction(cdp, action, arg) {
  try {
    return await deepTestActionInner(cdp, action, arg);
  } finally {
    await closePrereqState(cdp, action).catch(() => {});
  }
}

async function deepTestActionInner(cdp, action, arg) {
  const sel = `[data-action="${action}"]${arg ? `[data-arg="${arg}"]` : ''}`;
  const r = { action, arg: arg || null, status: 'PASS', checks: {}, notes: [] };

  // v0.9.7：状态前置开启（横幅/向导/弹窗/窄屏），避免 SKIP_INVISIBLE 变成假通过
  const prereq = await openPrereqState(cdp, action);
  if (prereq && prereq !== 'missing') r.notes.push('前置: ' + prereq);
  if (prereq) await sleep(300);

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

  // v0.9.6 修复：验证元素所属面板已激活；若所属 .tab-content 不是 active，
  // 说明面板未被正确切换（如快捷入口页面），先强制激活再测试，避免误判不可见而跳过。
  const panelEnsured = await cdp.eval(`(() => {
    const el = document.querySelector(${JSON.stringify(sel)});
    if (!el) return false;
    const panel = el.closest('.tab-content');
    if (!panel) return false;
    if (panel.classList.contains('active')) return false;
    const name = panel.id.replace(/^tab-/, '');
    if (typeof window.switchTab === 'function') { window.switchTab(name); return true; }
    return false;
  })()`);
  if (panelEnsured) await sleep(700);

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

  if (!st.found) {
    // v0.9.7：动态元素（M2/M4a 等 JS 渲染的卡片）可能恰在骨架期被枚举到缺席，
    // 或带 arg 的元素因列表重渲染漂移消失。轮询等待同 action 元素出现后再测，
    // 避免枚举/渲染竞态导致假失败。
    {
      let fallback = null;
      for (let attempt = 0; attempt < 3; attempt++) {
        await sleep(1500);
        // 返回哨兵字符串区分「元素不存在(null)」与「存在但无 data-arg('')」
        fallback = await cdp.eval(`(() => {
          const els = [...document.querySelectorAll('[data-action="${action}"]')];
          if (!els.length) return null;
          const vis = els.find((el) => el.offsetWidth || el.offsetHeight) || els[0];
          const newArg = vis.getAttribute('data-arg');
          vis.setAttribute('data-test-fallback', '1');
          return newArg === null ? '' : newArg;
        })()`);
        if (fallback !== null) break;
      }
      if (fallback !== null && fallback !== undefined) {
        r.notes.push(`动态元素缺席（枚举/渲染竞态），按同 action 现存元素重测${arg ? `: arg ${arg} → ${fallback || 'none'}` : ''}`);
        // 记录点击前的激活面板，用于识别「切页类」action 的可观测反馈
        const preActive = await cdp.eval(`(() => { const a = document.querySelector('.tab-content.active'); return a ? a.id : ''; })()`);
        await cdp.eval(`(() => { const el = document.querySelector('[data-action="${action}"][data-test-fallback="1"]'); if (el) { el.removeAttribute('data-test-fallback'); el.click(); } return 1; })()`);
        await sleep(900);
        const fbOk = await cdp.eval(`(() => {
          const toast = document.querySelector('#toast-container .toast');
          const detail = document.querySelector('#memory-detail-backdrop.open, #memory-detail-panel.open');
          const a = document.querySelector('.tab-content.active');
          return { toast: !!toast, detailOpen: !!detail, activePanel: a ? a.id : '' };
        })()`);
        r.checks.fallbackFeedback = fbOk;
        if (fbOk.detailOpen || fbOk.toast || fbOk.activePanel !== preActive) {
          await closePrereqState(cdp, action);
          return r; // PASS + 备注
        }
        r.status = 'WARN'; r.notes.push('回退元素点击后无可观测反馈');
        await closePrereqState(cdp, action);
        return r;
      }
    }
    r.status = 'SKIP_NOT_FOUND'; r.notes.push('元素不存在（动态生成）'); await closePrereqState(cdp, action); return r;
  }
  r.checks.count = st.count;
  r.checks.visible = st.visible;
  r.checks.disabled = st.disabled;
  r.checks.text = st.text;
  if (!st.visible) { r.status = 'SKIP_INVISIBLE'; r.notes.push('隐藏原因: ' + (st.reason || '未知')); await closePrereqState(cdp, action); return r; }
  if (st.disabled) { r.status = 'SKIP_DISABLED'; r.notes.push('元素禁用（前置条件未满足）'); await closePrereqState(cdp, action); return r; }

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

  const newHardNetworkFailures = cdp.hardNetworkFailures.length - preHardNet;
  const expectedUnfoldFailure = action === 'unfoldMemory' && cdp.networkFailures.slice(preNet).some((item) => /404 .*\/v1\/memories\/unfold/.test(item));
  // v0.9.7 审查修复：豁免必须"按错误文本逐条过滤"，绝不能 splice 清空整个窗口——
  // 那会把同窗口内并发出现的真实 JS 错误一并吞掉（消除假失败却掩盖真失败）。
  // 豁免特征收紧为两条精确正则：unfold 预期 404、Tauri capability 跨 origin 拒绝。
  const EXPECTED_ERROR_PATTERNS = [];
  if (expectedUnfoldFailure) EXPECTED_ERROR_PATTERNS.push(/404|unfold/i);
  if (action === 'handleStartServiceClick') {
    // Tauri capability 对非 tauri origin 的 IPC 拒绝消息形如
    // "start_sidecar not allowed. Plugin not found" / "...not allowed on origin..."
    EXPECTED_ERROR_PATTERNS.push(/(start_sidecar|command).{0,40}not allowed/i);
  }
  const removed = cdp.consoleErrors.filter((e, i) => i >= preConsoleErrors && EXPECTED_ERROR_PATTERNS.some((re) => re.test(e)));
  if (removed.length > 0) {
    cdp.consoleErrors = cdp.consoleErrors.filter((e, i) => i < preConsoleErrors || !EXPECTED_ERROR_PATTERNS.some((re) => re.test(e)));
    r.notes.push(`豁免 ${removed.length} 条预期错误（${expectedUnfoldFailure ? 'unfold 404 已验证错误反馈' : '跨origin IPC 被 capability 正确拒绝，安全边界生效'}）`);
  }
  r.checks.newErrors = cdp.errorTotal() - preErr;
  r.checks.newNetFailures = cdp.networkFailures.length - preNet;
  if (r.checks.newErrors > 0) {
    r.status = 'WARN';
    r.notes.push('点击后新增 ' + r.checks.newErrors + ' 个错误: ' + cdp.recentErrors(3).join(' | ').slice(0, 200));
  }
  if (newHardNetworkFailures > 0) { r.status = 'WARN'; r.notes.push('点击后新增 ' + newHardNetworkFailures + ' 个网络失败: ' + cdp.hardNetworkFailures.slice(-2).join(' | ').slice(0, 200)); }

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

  // v0.9.7：恢复前置状态（关闭临时开启的弹窗/横幅/向导、清除窄屏模拟），
  // 避免残留状态污染后续元素的可见性判定
  await closePrereqState(cdp, action);

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
// 非 data-tab 面板切换测试（captain-log/benchmarks/api-docs/project-switch）
// v0.9.6 修复：这些面板没有 [data-tab] 导航元素（仅 settings 内 switchToTab
// 快捷入口按钮），原 testTab 只点击 [data-tab] 无法覆盖。
// 此处复用 switchPanel 三层策略（data-tab → switchToTab → window.switchTab），
// 并验证 #tab-{name} 已激活，确保每个面板都先验证 active 再进入后续测试。
// =====================================================================
async function testTabViaSwitch(cdp, name) {
  const r = { tab: name, status: 'PASS', checks: {}, notes: [] };
  const preErr = cdp.errorTotal();
  await switchPanel(cdp, name);
  const st = await cdp.eval(`(() => {
    const panel = document.getElementById('tab-${name}');
    if (!panel) return { panelExists: false };
    const activePanelCount = document.querySelectorAll('.tab-content.active').length;
    return { panelExists: true, panelActive: panel.classList.contains('active'), activePanelCount };
  })()`);
  r.checks = st;
  r.checks.newErrors = cdp.errorTotal() - preErr;
  if (!st.panelExists) { r.status = 'FAIL'; r.notes.push('缺少面板 #tab-' + name); }
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
    // v0.9.7 联想中心：检索/探索类输入注入随机值会触发真实 enrich/explore，
    // 造成锁竞争（search_busy 503）与长耗时，且污染搜索历史——与密钥输入同属
    // 「不应被回归注入」的输入，纳入保护。
    'home-search-input', 'memory-search-input', 'association-query',
  ]);
  if (protectedIds.has(id) || input.type === 'password' || input.type === 'url') {
    r.status = 'SKIP_PROTECTED';
    r.notes.push('配置/密钥/路径/搜索输入不修改，避免触发真实检索或污染后续交互测试');
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
// 前置：确保 sidecar 运行
// dev 模式下桌面端启动后 sidecar 默认不自动拉起（需用户点击「启动服务」），
// 若直接测试仪表盘 HTTP 口子会全部 ERR_CONNECTION_REFUSED。此处主动点击启动
// 并轮询 /health 直至 running；CI 中 sidecar 已由外部启动则立即返回。
// =====================================================================
async function ensureSidecarRunning(cdp) {
  const port = await cdp.eval(
    `document.querySelector('meta[name="lrc-sidecar-port"]')?.content || '3111'`
  );
  const healthUrl = `http://127.0.0.1:${port}/health`;
  const isUp = async () => {
    try {
      const ctrl = new AbortController();
      const t = setTimeout(() => ctrl.abort(), 2000);
      const r = await fetch(healthUrl, { signal: ctrl.signal });
      clearTimeout(t);
      const j = await r.json();
      return j.status === 'running';
    } catch {
      return false;
    }
  };
  if (await isUp()) {
    console.log(`[BOOT] sidecar 已在端口 ${port} 运行，跳过启动`);
    return;
  }
  console.log(`[BOOT] sidecar 未运行，点击「启动服务」按钮（端口 ${port}）...`);
  await cdp.eval(`(() => { const b = document.querySelector('[data-action="handleStartServiceClick"]'); if (b) { b.click(); return true; } return false; })()`);
  for (let i = 0; i < 40; i++) {
    if (await isUp()) {
      console.log(`[BOOT] sidecar 启动完成（端口 ${port}）`);
      await sleep(1500); // 等待仪表盘首屏数据加载
      return;
    }
    await sleep(2000);
  }
  console.log(`[BOOT] 警告：sidecar 在 80s 内未就绪，后续 HTTP 口子测试可能失败`);
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
  // v0.9.6：先确保 sidecar 运行，再导航到 dev-proxy 加载磁盘最新前端。
  // 顺序很关键：若先导航，仪表盘会在 sidecar 未起时发起 HTTP 请求，
  // 产生大量 ERR_CONNECTION_REFUSED 污染门禁；先起服务再导航可避免。
  // 说明：tauri dev 的 WebView 可能停留在打包的静态副本（tauri.localhost），
  // 而 dev-proxy(1420) 直接服务 static/ 目录，导航过去才能测到最新 app.js。
  // dev-proxy 不可用（如 CI 无 1420）时回退为原地重载。
  await ensureSidecarRunning(cdp);
  const devUrl = await cdp.eval(`(() => { try { return new URL(location.href).port === '1420' ? 'same' : 'http://localhost:1420/'; } catch { return 'http://localhost:1420/'; } })()`);
  if (devUrl !== 'same') {
    try {
      const probe = await fetch('http://localhost:1420/health-probe-static-check', { signal: AbortSignal.timeout(2000) }).catch(() => null);
      // 只要有任意响应（含 404）即说明 dev-proxy 存活
      if (probe) {
        await cdp.send('Page.navigate', { url: devUrl });
        await sleep(2500);
        console.log(`[BOOT] 已导航到 dev-proxy: ${devUrl}（测试磁盘最新前端）`);
      } else {
        await cdp.send('Page.reload', { ignoreCache: true });
        await sleep(2500);
        console.log(`[BOOT] dev-proxy 不可用，原地重载 tauri.localhost`);
      }
    } catch {
      await cdp.send('Page.reload', { ignoreCache: true });
      await sleep(2500);
    }
  } else {
    await cdp.send('Page.reload', { ignoreCache: true });
    await sleep(2500);
  }
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

  // v0.9.6 修复：captain-log/benchmarks/api-docs/project-switch 等面板没有
  // [data-tab] 导航元素（仅 settings 内 switchToTab 快捷入口按钮），未被
  // ENUMERATE_JS 的 tabs 枚举覆盖。此处补充：用 switchPanel 三层策略切换
  // 并验证 #tab-X 已激活，保证每个面板都先验证 active 再进入后续测试。
  const extraPanels = ['captain-log', 'benchmarks', 'api-docs', 'project-switch'];
  for (const p of extraPanels) {
    if (inv.tabs.some((x) => x.tab === p)) continue;
    const tr = await testTabViaSwitch(cdp, p);
    tabResults.push(tr);
    console.log(`[TAB] ${p} -> ${tr.status}${tr.notes.length ? ' (' + tr.notes.join('; ') + ')' : ''}`);
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
    ...tabResults.filter((r) => ['FAIL', 'WARN'].includes(r.status)),
    // WARN 表示交互断言未完全通过，同样纳入发布门禁。
    ...actionResults.filter((r) => ['ERROR', 'FAIL', 'WARN'].includes(r.status)),
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
