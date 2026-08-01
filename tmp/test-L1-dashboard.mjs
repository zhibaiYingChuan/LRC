// L1 一级页面审计：仪表盘真实状态快照
import { cdpBatch } from './cdp-audit-client.mjs';

const expressions = [
  {
    name: '页面标题与基础信息',
    expr: `JSON.stringify({
      title: document.title,
      url: location.href,
      readyState: document.readyState,
      appVersion: typeof APP_VERSION !== 'undefined' ? APP_VERSION : 'unknown',
      apiBase: typeof API_BASE !== 'undefined' ? API_BASE : 'unknown'
    })`
  },
  {
    name: 'SidecarHealthMonitor 状态',
    expr: `JSON.stringify({
      exists: typeof SidecarHealthMonitor !== 'undefined',
      isReachable: typeof SidecarHealthMonitor !== 'undefined' ? SidecarHealthMonitor._isReachable : null,
      sidecarStatus: typeof SidecarHealthMonitor !== 'undefined' ? SidecarHealthMonitor._sidecarStatus : null,
      lockBusy: typeof SidecarHealthMonitor !== 'undefined' ? SidecarHealthMonitor._lockBusy : null,
      failCount: typeof SidecarHealthMonitor !== 'undefined' ? SidecarHealthMonitor._failCount : null,
      failThreshold: typeof SidecarHealthMonitor !== 'undefined' ? SidecarHealthMonitor._FAIL_THRESHOLD : null,
      isRunning: typeof SidecarHealthMonitor !== 'undefined' ? SidecarHealthMonitor.isRunning : null,
      isIndexing: typeof SidecarHealthMonitor !== 'undefined' && typeof SidecarHealthMonitor.isIndexing === 'function' ? SidecarHealthMonitor.isIndexing() : null,
      backoffStep: typeof SidecarHealthMonitor !== 'undefined' ? SidecarHealthMonitor._backoffStep : null
    })`
  },
  {
    name: '关键 UI 元素状态',
    expr: `JSON.stringify({
      sidecarDownBanner: (() => { const b = document.getElementById('sidecar-down-banner'); return b ? { hidden: b.hidden, display: getComputedStyle(b).display, text: b.innerText.substring(0,200) } : 'NOT_FOUND'; })(),
      dashboardLoading: (() => { const b = document.getElementById('dashboard-loading'); return b ? { hidden: b.classList.contains('hidden'), display: getComputedStyle(b).display } : 'NOT_FOUND'; })(),
      dashboardError: (() => { const b = document.getElementById('dashboard-error'); return b ? { show: b.classList.contains('show'), text: b.innerText.substring(0,300), html: b.innerHTML.substring(0,500) } : 'NOT_FOUND'; })(),
      statusBar: (() => { const b = document.getElementById('status-bar') || document.querySelector('.status-bar'); return b ? { text: b.innerText.substring(0,200), html: b.innerHTML.substring(0,400) } : 'NOT_FOUND'; })(),
      statusDot: (() => { const b = document.getElementById('status-dot') || document.querySelector('.status-dot'); return b ? { className: b.className, text: b.innerText } : 'NOT_FOUND'; })()
    })`
  },
  {
    name: '仪表盘卡片内容快照',
    expr: `JSON.stringify({
      daoMetrics: (() => { const b = document.getElementById('dao-metrics') || document.querySelector('[data-card="dao"]') || document.querySelector('#dao-metrics-card'); return b ? { text: b.innerText.substring(0,300), html: b.innerHTML.substring(0,500) } : 'NOT_FOUND'; })(),
      memoriesStats: (() => { const b = document.getElementById('memories-stats') || document.querySelector('[data-card="memories"]'); return b ? { text: b.innerText.substring(0,300) } : 'NOT_FOUND'; })(),
      projectDist: (() => { const b = document.getElementById('project-distribution') || document.querySelector('[data-card="projects"]'); return b ? { text: b.innerText.substring(0,300) } : 'NOT_FOUND'; })()
    })`
  },
  {
    name: 'IPC/Tauri 环境检测',
    expr: `JSON.stringify({
      hasInvoke: typeof window.__TAURI__ !== 'undefined' && typeof window.__TAURI__.core !== 'undefined' && typeof window.__TAURI__.core.invoke === 'function',
      hasTauri: typeof window.__TAURI__ !== 'undefined',
      tauriKeys: typeof window.__TAURI__ !== 'undefined' ? Object.keys(window.__TAURI__) : [],
      hasPostMessage: typeof window.postMessage === 'function',
      userAgent: navigator.userAgent.substring(0, 200)
    })`
  },
  {
    name: '重试计数器与定时器状态',
    expr: `JSON.stringify({
      dashboardRetryCount: typeof _dashboardRetryCount !== 'undefined' ? _dashboardRetryCount : 'unknown',
      dashboardMaxRetries: typeof _DASHBOARD_MAX_RETRIES !== 'undefined' ? _DASHBOARD_MAX_RETRIES : 'unknown',
      dashboardRetryTimerActive: typeof _dashboardRetryTimer !== 'undefined' ? (_dashboardRetryTimer !== null) : 'unknown',
      retryCounters: typeof _retryCounters !== 'undefined' && _retryCounters ? Object.fromEntries(_retryCounters) : 'unknown',
      retryModalActive: typeof _retryModalActive !== 'undefined' ? _retryModalActive : 'unknown',
      pendingRequestCount: typeof pendingRequestCount !== 'undefined' ? pendingRequestCount : 'unknown',
      pendingBackgroundCount: typeof _pendingBackgroundCount !== 'undefined' ? _pendingBackgroundCount : 'unknown'
    })`
  }
];

console.log('=== L1 一级页面审计：执行 6 项状态快照 ===');
try {
  const result = await cdpBatch(expressions, 45000);
  console.log('\n=== Console 日志（执行期间）===');
  if (result.consoleLogs && result.consoleLogs.length) {
    result.consoleLogs.forEach(l => console.log(`[${l.type}] ${l.text.substring(0, 200)}`));
  } else {
    console.log('（无 console 日志）');
  }
  console.log('\n=== 测试结果 ===');
  for (const r of result.results) {
    console.log('\n--- ' + r.name + ' ---');
    if (r.result && r.result.result) {
      const val = r.result.result.value;
      try {
        const parsed = JSON.parse(val);
        console.log(JSON.stringify(parsed, null, 2));
      } catch (e) {
        console.log(val);
      }
    } else if (r.error) {
      console.log('ERROR:', r.error);
    } else {
      console.log(JSON.stringify(r, null, 2));
    }
  }
} catch (e) {
  console.error('测试执行失败:', e.message);
  console.error(e.stack);
}
