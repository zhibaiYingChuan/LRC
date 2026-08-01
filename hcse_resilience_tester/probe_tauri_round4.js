(async () => {
  const r = {};
  // 环境识别
  r.env = {
    hasTauri: !!(window.__TAURI__ || window.__TAURI_INTERNALS__ || window.__TAURI_IPC__),
    iframeMode: (window.parent !== window),
    appVersion: (typeof APP_VERSION !== 'undefined') ? APP_VERSION : (window.APP_VERSION || null),
    url: location.href,
    title: document.title
  };
  // SidecarHealthMonitor (L6)
  const m = window.sidecarHealthMonitor;
  r.monitor = m ? {
    exists: true, isReachable: m._isReachable, sidecarStatus: m._sidecarStatus,
    lockBusy: m._lockBusy, failCount: m._failCount, failThreshold: m._FAIL_THRESHOLD,
    backoffStep: m._backoffStep, isIndexing: m._isIndexing,
    hasCheck: typeof m.check === 'function', hasStart: typeof m.start === 'function'
  } : { exists: false };
  // IA-01 daoAbortController
  r.daoAbortController = window.daoAbortController ? {
    exists: true, signalAborted: window.daoAbortController.signal ? window.daoAbortController.signal.aborted : null
  } : { exists: false };
  // IA-02 全局错误处理
  r.globalError = {
    onerror: typeof window.onerror === 'function',
    hasErrorListener: !!(window.__globalErrorHandlerRegistered)
  };
  // toast 队列 (GAP-12)
  const tc = document.getElementById('toast-container');
  r.toast = {
    containerExists: !!tc,
    visibleCount: tc ? tc.querySelectorAll('.toast:not(.toast-leaving)').length : 0,
    errorCount: tc ? tc.querySelectorAll('.toast-error:not(.toast-leaving)').length : 0
  };
  // handleHttpError 503 分支 (P0-4)
  const he = (typeof handleHttpError === 'function') ? handleHttpError.toString() : '';
  const idx503 = he.indexOf('status === 503');
  r.handleHttpError = {
    exists: he.length > 0, len: he.length,
    has503Cooldown: he.includes('503_cooldown'),
    hasRetryCountersDelete: he.includes('_retryCounters.delete'),
    snippet503: idx503 >= 0 ? he.substring(idx503, idx503 + 500) : '(无503分支)'
  };
  // pendingRequestCount (P1-LOCKBUSY-07 回归)
  r.pendingRequestCount = (typeof pendingRequestCount !== 'undefined') ? pendingRequestCount : null;
  // manualRefreshDashboard (GAP-04 防抖)
  r.manualRefresh = {
    exists: typeof manualRefreshDashboard === 'function',
    isRefreshing: (typeof _isManualRefreshing !== 'undefined') ? _isManualRefreshing : null
  };
  // dashboard 状态
  const dl = document.getElementById('dashboard-loading');
  const de = document.getElementById('dashboard-error');
  r.dashboard = {
    loadingVisible: dl ? !dl.classList.contains('hidden') : null,
    errorShow: de ? de.classList.contains('show') : null,
    errorText: de ? (de.textContent||'').replace(/\s+/g,' ').substring(0, 200) : null
  };
  // banner (矛盾检测)
  const banner = document.getElementById('sidecar-down-banner');
  r.banner = {
    exists: !!banner, visible: banner ? !banner.classList.contains('hidden') : null,
    text: banner ? (banner.textContent||'').replace(/\s+/g,' ').substring(0, 200) : null
  };
  // status bar
  const sb = document.getElementById('status-text') || document.getElementById('status-bar');
  r.statusBar = sb ? (sb.textContent||'').replace(/\s+/g,' ').substring(0, 200) : null;
  // dao ring
  const ring = document.getElementById('dao-ring-score');
  r.daoRingScore = ring ? (ring.textContent||'').trim() : null;
  // 统计卡片
  r.stats = {
    memoryCount: (document.getElementById('stat-memory-count')||{}).textContent || null,
    totalChunks: (document.getElementById('stat-total-chunks')||{}).textContent || null,
    fileCount: (document.getElementById('stat-file-count')||{}).textContent || null
  };
  // 矛盾检测
  r.contradiction = {
    bannerMonitorMismatch: r.banner.exists && r.monitor.exists && r.banner.visible && r.monitor.isReachable,
    detail: r.banner.visible && r.monitor.isReachable ? 'banner显示服务未运行 但 monitor.isReachable=true' : null
  };
  return r;
})()
