// HCSE Round 5 前端状态采集 + 修复点运行时验证
// 直连 CDP 9223 Tauri WebView2
(() => {
  const result = {};

  // 1. 环境信息
  result.env = {
    hasTauri: typeof window.__TAURI__ !== 'undefined' || typeof window.__TAURI_INTERNALS__ !== 'undefined',
    url: window.location.href,
    appVersion: window.APP_VERSION || 'unknown',
    title: document.title,
    readyState: document.readyState
  };

  // 2. SidecarHealthMonitor 状态（INV-STATE-002）
  const shm = window.sidecarHealthMonitor;
  result.monitor = shm ? {
    exists: true,
    isReachable: shm.isReachable,
    sidecarStatus: shm.sidecarStatus,
    lockBusy: shm._lockBusy,
    failCount: shm._failCount,
    backoffStep: shm._backoffStep,
    hasCheckHealth: typeof shm.checkHealth === 'function'
  } : { exists: false };

  // 3. dashboard 状态
  const loading = document.getElementById('loading-overlay');
  const error = document.getElementById('dashboard-error');
  const statTotal = document.getElementById('stat-total');
  const statActive = document.getElementById('stat-active');
  const statCrystallized = document.getElementById('stat-crystallized');
  result.dashboard = {
    loadingVisible: loading ? !loading.classList.contains('hidden') : null,
    errorShow: error ? error.classList.contains('show') : null,
    statTotal: statTotal ? statTotal.textContent : null,
    statActive: statActive ? statActive.textContent : null,
    statCrystallized: statCrystallized ? statCrystallized.textContent : null
  };

  // 4. P1-NEW-01: loadDashboard 函数源码运行时验证
  result.loadDashboard = { exists: typeof window.loadDashboard === 'function' };
  if (typeof window.loadDashboard === 'function') {
    const src = window.loadDashboard.toString();
    result.loadDashboard.hasLockBusy503 = src.includes('hasLockBusy503');
    result.loadDashboard.hasLockBusy200 = src.includes('hasLockBusy200');
    result.loadDashboard.throwsLockBusy = src.includes("throw new Error('LOCK_BUSY')");
    result.loadDashboard.srcLength = src.length;
    // 提取关键片段（前 600 字符包含 hasLockBusy 逻辑）
    const idx503 = src.indexOf('hasLockBusy503');
    if (idx503 >= 0) {
      result.loadDashboard.snippet = src.substring(idx503, Math.min(idx503 + 320, src.length));
    }
  }

  // 5. P1-NEW-02: renderDashboard 函数源码运行时验证
  result.renderDashboard = { exists: typeof window.renderDashboard === 'function' };
  if (typeof window.renderDashboard === 'function') {
    const src = window.renderDashboard.toString();
    result.renderDashboard.hasLockBusyCheck = src.includes('lock_busy');
    result.renderDashboard.srcLength = src.length;
    // 提取防御性检查片段
    const idx = src.indexOf('lock_busy');
    if (idx >= 0) {
      result.renderDashboard.snippet = src.substring(Math.max(0, idx - 80), Math.min(idx + 200, src.length));
    }
  }

  // 6. 状态点（INV-STATE-002）
  const statusDot = document.querySelector('.status-dot');
  result.statusDot = statusDot ? {
    className: statusDot.className,
    isOnline: statusDot.classList.contains('online')
  } : null;

  // 7. pendingRequestCount（Round 3 P1-LOCKBUSY-07 回归）
  result.pendingRequestCount = window._pendingRequestCount || 0;

  // 8. globalError（IA-02）
  result.globalError = { onerror: !!window.onerror };

  // 9. daoAbortController（IA-01）
  result.daoAbortController = {
    exists: !!window.daoAbortController,
    signalAborted: window.daoAbortController ? window.daoAbortController.signal.aborted : null
  };

  // 10. handleHttpError 503 冷却期（P0-4 回归）
  result.handleHttpError = { exists: typeof window.handleHttpError === 'function' };
  if (typeof window.handleHttpError === 'function') {
    const src = window.handleHttpError.toString();
    result.handleHttpError.has503Cooldown = src.includes('503_cooldown') || src.includes('cooldown');
  }

  // 11. manualRefreshDashboard（GAP-01+GAP-04 回归）
  result.manualRefreshDashboard = {
    exists: typeof window.manualRefreshDashboard === 'function'
  };

  return result;
})()
