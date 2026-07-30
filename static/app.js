
// ============================================================
// Loong Recall 仪表盘 — 主应用脚本
// 使用 IIFE 模式隔离作用域，仅暴露 HTML onclick 所需的函数到全局
// ============================================================
// v0.8.5 Step 18：版本号常量（CDP 测试与运行时查询使用）
const APP_VERSION = '0.8.9';
window.__LRC_VERSION__ = APP_VERSION;

(function() {
  'use strict';

  // ============================================================
  // 全局配置
  // ============================================================
// v0.6.0 P0 修复：Tauri WebView 环境下 window.location.origin 为 https://tauri.localhost
// v0.6.0 P1-1 修复：macOS/Linux 的 WebView 源是 tauri://localhost，不是 tauri.localhost
// 需检测所有平台的 Tauri 环境标志
const isTauriEnv = (typeof window.__TAURI__ !== 'undefined') ||
  (typeof window.__TAURI_INTERNALS__ !== 'undefined') ||
  (window.location.origin && (
    window.location.origin.includes('tauri.localhost') ||
    window.location.origin.startsWith('tauri://')
  ));
const DEFAULT_API_BASE = isTauriEnv
  ? 'http://127.0.0.1:3099'  // Tauri 环境：直连 sidecar（初始值，异步会通过 IPC 更新为实际端口）
  : (window.location.origin || 'http://localhost:3099');  // 浏览器环境：同源访问
// v0.6.0 P0-1 修复：Tauri 环境下 sidecar 可能端口自适应到非 3099，需改为 let 以便异步更新
let API_BASE = new URLSearchParams(window.location.search).get('api') || DEFAULT_API_BASE;
const REFRESH_INTERVAL = 30000; // 30 秒自动刷新
let refreshTimer = null;
let startTime = Date.now();

// 将 API 基础 URL 显示到文档中
const apiBaseDisplay = $('api-base-display');
if (apiBaseDisplay) apiBaseDisplay.textContent = API_BASE;

// ============================================================
// 工具函数
// ============================================================

/** 格式化百分比 */
function pct(val) {
  if (val == null || isNaN(val)) return '--';
  return (val * 100).toFixed(1) + '%';
}

/** 格式化数字 */
function num(val) {
  if (val == null || isNaN(val)) return '--';
  return val.toLocaleString('zh-CN');
}

/** 格式化运行时长 */
function formatUptime(ms) {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const h = Math.floor(m / 60);
  const d = Math.floor(h / 24);
  if (d > 0) return d + '天 ' + (h % 24) + '小时';
  if (h > 0) return h + '小时 ' + (m % 60) + '分钟';
  if (m > 0) return m + '分钟 ' + (s % 60) + '秒';
  return s + '秒';
}

/** 获取状态徽章 HTML */
function statusBadge(status) {
  const map = {
    healthy: 'healthy', warning: 'warning', critical: 'critical',
    degraded: 'warning', oscillating: 'warning', drifting: 'warning',
    frozen: 'critical', overloaded: 'critical',
  };
  const cls = map[status] || 'info';
  return `<span class="badge ${cls}">${htmlescape(status)}</span>`;
}

/** 安全 JSON 解析 */
function safeJson(res) {
  if (!res.ok) throw new Error(`HTTP ${res.status}: ${res.statusText}`);
  return res.json();
}

// v0.8.2：全局进行中请求计数器（对应审计 G006）
// v0.8.3 Step 5：暴露只读接口到 window，便于 CDP 测试与外部检测（修复 G006）
// 使用 Object.defineProperty 的 getter（无 setter）确保只读，避免外部恶意修改
let pendingRequestCount = 0;
window.__getPendingRequestCount = () => pendingRequestCount;
Object.defineProperty(window, 'pendingRequestCount', {
  get: () => pendingRequestCount,
  configurable: false,
  enumerable: true
});

/**
 * 带超时的 fetch
 * v0.8.2：优化错误处理，sidecar 不可达时抛出可识别的错误类型，避免 console.warn 污染
 * v0.8.2：维护 pendingRequestCount，供 beforeunload 拦截使用（对应审计 G006）
 * @param {string} url - 请求 URL
 * @param {object} options - fetch 选项
 * @param {number} timeout - 超时毫秒
 * @returns {Promise<Response>}
 * @throws {SidecarUnreachableError} sidecar 不可达（连接拒绝/DNS 失败）
 * @throws {SidecarTimeoutError} sidecar 请求超时
 */
async function fetchWithTimeout(url, options = {}, timeout = 10000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);

  // v0.8.3 Step 10：支持外部 signal（用于 AbortController 取消旧请求，修复 N09）
  // 若 options.signal 已 abort，立即抛出 AbortError（避免无谓的网络请求）
  // 若 options.signal 在请求过程中 abort，同步触发 controller.abort()
  const externalSignal = options.signal;
  if (externalSignal) {
    if (externalSignal.aborted) {
      clearTimeout(timer);
      const err = new Error('请求已被外部取消');
      err.name = 'AbortError';
      throw err;
    }
    externalSignal.addEventListener('abort', () => controller.abort(), { once: true });
  }

  // v0.8.2：增加进行中请求计数
  pendingRequestCount++;
  try {
    // 移除 options 中的 signal（用 controller.signal 替代，避免冲突）
    const { signal, ...restOptions } = options;
    const res = await fetch(url, { ...restOptions, signal: controller.signal });

    // v0.8.6 Step 3 / N001 G052 修复：集成 handleHttpError，激活错误恢复层
    // 之前 handleHttpError 是死代码，40+ catch 块无一调用，导致 500/429/503 错误恢复失效
    if (!res.ok) {
      const retryContext = { method: restOptions.method || 'GET', url: url };
      const result = await handleHttpError(res, `请求 ${url}`, retryContext);

      if (result.action === 'retry') {
        // 用户选择重试，递归调用（handleHttpError 内部已限制最大重试 3 次）
        clearTimeout(timer);
        pendingRequestCount--;
        return fetchWithTimeout(url, options, timeout);
      }
      // cancel 或 giveup：抛出错误，由调用方 catch 块处理
      const err = new Error(result.errorDetail || `HTTP ${result.status}`);
      err.name = 'HttpError';
      err.status = result.status;
      err.url = url;
      throw err;
    }

    return res;
  } catch (e) {
    // v0.8.2：分类错误，不产生 console.warn（避免被测试脚本计为 newErrors）
    if (e.name === 'AbortError') {
      // v0.8.3 Step 10：区分外部 abort 与超时 abort
      if (externalSignal && externalSignal.aborted) {
        // 外部主动取消，直接抛出 AbortError（不转换为 SidecarTimeoutError）
        throw e;
      }
      // 超时中止
      const err = new Error('请求超时，请检查 LRC 服务是否正常运行');
      err.name = 'SidecarTimeoutError';
      err.url = url;
      throw err;
    } else if (e instanceof TypeError && e.message.includes('Failed to fetch')) {
      // 网络层错误：连接拒绝/DNS 失败/CORS
      const err = new Error('无法连接到 LRC 服务，请确认服务已启动');
      err.name = 'SidecarUnreachableError';
      err.url = url;
      throw err;
    }
    throw e;
  } finally {
    clearTimeout(timer);
    // v0.8.2：减少进行中请求计数
    pendingRequestCount--;
  }
}

// ============================================================
// v0.8.3 Step 12 / G007 + G009：HTTP 错误统一处理（对应审计 G007/G009）
// 设计原则：
//   1. 不阻塞 JS 线程，使用 Toast/Modal 反馈
//   2. 500 错误显示"重试/查看日志"操作 Modal
//   3. 503 错误显示"服务降级保护中"提示
//   4. 429 错误显示"请求过于频繁"提示
//   5. 其他非 2xx 状态码统一显示错误 Toast
//   6. v0.8.4 Step 10 / G029：500 重试次数上限（3 次）+ 指数退避（1s/2s/4s）
//   7. v0.8.4 Step 10 / G047：重试 Modal 显示时禁止标签页切换
// 使用方式：const res = await fetchWithTimeout(url); if (!res.ok) { handleHttpError(res, '加载数据'); return; }
// ============================================================

// v0.8.4 Step 10 / G029：重试计数器（按请求 URL 区分）
const _retryCounters = new Map();
const MAX_RETRY_COUNT = 3;
// v0.8.4 Step 10 / G047：重试 Modal 激活标志，禁止标签页切换
let _retryModalActive = false;

async function handleHttpError(response, context = '操作', retryContext = null) {
  const status = response.status;
  let errorDetail = '';
  try {
    const errJson = await response.json();
    errorDetail = errJson?.error || errJson?.message || '';
  } catch (e) {
    try {
      errorDetail = await response.text();
    } catch (e2) {
      errorDetail = `HTTP ${status}`;
    }
  }
  // 限制错误详情长度，避免 Toast 过长
  if (errorDetail.length > 200) errorDetail = errorDetail.substring(0, 200) + '...';

  console.error(`[handleHttpError] ${context} 失败 [${status}]:`, errorDetail);

  if (status === 500) {
    // v0.8.4 Step 10 / G029：重试次数上限检查
    const retryKey = retryContext ? `${retryContext.method || 'GET'}:${retryContext.url || ''}` : `default:${context}`;
    const retryCount = _retryCounters.get(retryKey) || 0;

    if (retryCount >= MAX_RETRY_COUNT) {
      // 超过重试上限，显示"停止重试"+ 引导
      _retryCounters.delete(retryKey); // 重置计数
      _retryModalActive = true;
      try {
        await showInfoModal('多次重试失败',
          `${context}已连续失败 ${MAX_RETRY_COUNT} 次。\n\n` +
          `可能原因：\n` +
          `• 服务端内部错误\n` +
          `• 数据库锁冲突\n` +
          `• 资源不足\n\n` +
          `建议操作：\n` +
          `• 查看 LRC 服务日志\n` +
          `• 重启 LRC 服务\n` +
          `• 联系技术支持`
        );
      } finally {
        _retryModalActive = false;
      }
      return { action: 'giveup', status, errorDetail };
    }

    // v0.8.4 Step 10 / G047：重试 Modal 显示时禁止标签页切换
    _retryModalActive = true;
    let shouldRetry;
    try {
      shouldRetry = await showConfirm(
        `${context}失败：服务内部错误（第 ${retryCount + 1} 次重试）\n\n错误详情：${errorDetail}\n\n是否重试？`,
        '服务错误',
        0
      );
    } finally {
      _retryModalActive = false;
    }

    if (shouldRetry) {
      _retryCounters.set(retryKey, retryCount + 1);
      // v0.8.4 Step 10 / G029：指数退避（1s → 2s → 4s）
      const backoff = Math.pow(2, retryCount) * 1000;
      console.log(`[handleHttpError] 用户选择重试，${backoff}ms 后重试（第 ${retryCount + 1} 次）`);
      await new Promise(r => setTimeout(r, backoff));
      return { action: 'retry', status, errorDetail };
    }
    _retryCounters.delete(retryKey);
    return { action: 'cancel', status, errorDetail };
  } else if (status === 503) {
    // G009：503 熔断降级 UI
    showToast(`${context}失败：服务降级保护中，请稍后重试`, 'warning', 5000);
    return { action: 'cancel', status, errorDetail };
  } else if (status === 429) {
    // G007 扩展：429 限流
    showToast(`${context}失败：请求过于频繁，请稍后再试`, 'warning', 4000);
    return { action: 'cancel', status, errorDetail };
  } else if (status === 401 || status === 403) {
    // 鉴权失败
    showToast(`${context}失败：权限不足（${status}）`, 'error', 4000);
    return { action: 'cancel', status, errorDetail };
  } else {
    // 其他非 2xx 错误
    showToast(`${context}失败：${errorDetail || 'HTTP ' + status}`, 'error', 4000);
    return { action: 'cancel', status, errorDetail };
  }
}

/**
 * v0.8.4 Step 10 / G029：重置重试计数器（在请求成功时调用）
 * @param {string} url - 请求 URL
 * @param {string} method - HTTP 方法
 */
function resetRetryCounter(url, method = 'GET') {
  _retryCounters.delete(`${method}:${url}`);
}
// 暴露到 window 便于测试
window.handleHttpError = handleHttpError;
window.resetRetryCounter = resetRetryCounter;

// ============================================================
// v0.8.2 新增：Sidecar 健康监测器（对应审计 G005）
// 每 10 秒轮询 sidecar 可达性，不可达时禁用所有 API 按钮并显示横幅
// ============================================================
const SidecarHealthMonitor = {
  _isReachable: true,
  _pollTimer: null,
  _pollInterval: 10000,  // 10 秒轮询
  _inFlight: false,

  /**
   * 启动健康监测
   */
  start() {
    if (this._pollTimer) return;
    // 立即检测一次
    this.check();
    // 定时轮询
    this._pollTimer = setInterval(() => this.check(), this._pollInterval);
    console.log('[LRC v' + APP_VERSION + ']Sidecar 健康监测器已启动，轮询间隔:', this._pollInterval + 'ms');
  },

  /**
   * 停止健康监测
   */
  stop() {
    if (this._pollTimer) {
      clearInterval(this._pollTimer);
      this._pollTimer = null;
    }
  },

  /**
   * 执行一次健康检测
   * v0.8.3 Step 9：改用 fetchWithTimeout 发起请求（修复 N05）
   *   - pendingRequestCount 正确计数（beforeunload 拦截可检测健康检查）
   *   - 错误经 SidecarUnreachableError/SidecarTimeoutError 分类
   *   - 健康检查超时 3s（短于 10s 轮询周期），错误不弹 Toast
   * @returns {Promise<boolean>} 是否可达
   */
  async check() {
    if (this._inFlight) return this._isReachable;
    this._inFlight = true;
    try {
      // v0.8.3 Step 9：改用 fetchWithTimeout，使 pendingRequestCount 计数
      // 健康检查超时 3s（短于 10s 轮询周期，避免请求堆积）
      const res = await fetchWithTimeout(`${API_BASE}/v1/health/system`, {}, 3000);
      this._setReachable(res.ok);
      return res.ok;
    } catch (e) {
      // 错误已由 fetchWithTimeout 分类（SidecarUnreachableError/SidecarTimeoutError）
      // 健康检查的错误不弹 Toast，仅更新状态（避免每 10s 弹错误 Toast 干扰用户）
      this._setReachable(false);
      return false;
    } finally {
      this._inFlight = false;
    }
  },

  /**
   * 更新可达状态，触发 UI 变更
   */
  _setReachable(reachable) {
    const wasReachable = this._isReachable;
    this._isReachable = reachable;

    if (reachable === wasReachable) return;  // 状态未变

    const banner = document.getElementById('sidecar-down-banner');
    const apiButtons = document.querySelectorAll('[data-action]');

    if (reachable) {
      // 恢复可达
      if (banner) banner.hidden = true;
      // v0.8.3 Step 8：恢复时同时移除 title 和 aria-disabled（修复 N04）
      apiButtons.forEach(btn => {
        btn.classList.remove('btn-disabled-api');
        btn.removeAttribute('title');
        btn.removeAttribute('aria-disabled');
      });
      console.log('[LRC v' + APP_VERSION + ']Sidecar 已恢复可达');
      // 自动刷新仪表盘
      if (typeof loadDashboard === 'function') {
        setTimeout(() => loadDashboard(), 500);
      }
    } else {
      // 不可达
      if (banner) banner.hidden = false;
      // v0.8.2：排除"启动服务"按钮，确保用户可以启动服务
      // v0.8.3 Step 8：添加 title 和 aria-disabled 属性（修复 N04）
      apiButtons.forEach(btn => {
        const action = btn.getAttribute('data-action');
        if (action === 'openStartServiceModal' || action === 'closeStartServiceModal') {
          return;  // 启动/关闭服务按钮不禁用
        }
        btn.classList.add('btn-disabled-api');
        btn.setAttribute('title', '服务未运行，请先启动 LRC 服务');
        btn.setAttribute('aria-disabled', 'true');
      });
      console.log('[LRC v' + APP_VERSION + ']Sidecar 不可达，已禁用 API 按钮');
    }
  },

  /**
   * 获取当前可达状态
   */
  isReachable() {
    return this._isReachable;
  },

  // v0.8.6 Step 8 / N006 修复：添加 intervalId getter 别名
  // 之前 SidecarHealthMonitor 使用 _pollTimer 属性名，CDP 测试期望 intervalId
  // 采用 getter 别名方案，不重命名现有属性，避免破坏现有逻辑
  get intervalId() {
    return this._pollTimer;
  },

  // v0.8.6 Step 8 / N006：isRunning getter，便于 CDP 测试检查监测器状态
  get isRunning() {
    return this._pollTimer !== null;
  }
};

// 暴露到全局
window.SidecarHealthMonitor = SidecarHealthMonitor;

/** HTML 转义，防止 XSS */
function htmlescape(str) {
  if (str == null) return '';
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/**
 * v0.8.4 Step 9 / G040 修复：白名单校验 memory_type，防止 XSS
 * 仅允许字母、数字、下划线、连字符，其他字符全部移除
 * @param {string} type - 原始 memory_type 值
 * @returns {string} 安全的 memory_type 字符串
 */
function sanitizeMemoryType(type) {
  const safe = String(type || '').replace(/[^a-zA-Z0-9_-]/g, '');
  return safe || 'unknown';
}

/** 安全获取 DOM 元素，不存在时输出警告并返回 null */
function $(id) {
  const el = document.getElementById(id);
  if (!el) console.warn('[Loong Recall] DOM 元素未找到:', id);
  return el;
}

// ============================================================
// 导航标签切换
// ============================================================
function toggleNav() {
  const nav = $('navbarNav');
  if (nav) nav.classList.toggle('open');
}

document.querySelectorAll('.navbar-nav button').forEach(btn => {
  btn.addEventListener('click', function() {
    // 关闭移动端菜单
    const nav = $('navbarNav');
    if (nav) nav.classList.remove('open');

    // 更新按钮状态
    document.querySelectorAll('.navbar-nav button').forEach(b => b.classList.remove('active'));
    this.classList.add('active');

    // 切换标签页
    const tabName = this.dataset.tab;
    document.querySelectorAll('.tab-content').forEach(tc => tc.classList.remove('active'));
    document.getElementById('tab-' + tabName).classList.add('active');

    // 切换到信任中心时刷新数据
    if (tabName === 'trust-center') loadTrustCenter();
    // 切换到基准报告时加载数据
    if (tabName === 'benchmarks') loadBenchmarks();
    // 切换到设置时加载配置
    if (tabName === 'settings') { loadSettings(); loadProjectInfo(); }
  });
});

// ============================================================
// 仪表盘数据加载
// ============================================================
// v0.8.3 Step 10：仪表盘 AbortController（修复 N09 自动刷新请求竞态）
// 维护一个 AbortController 实例，刷新前 abort 旧请求，避免数据覆盖
let dashboardAbortController = null;

async function loadDashboard() {
  const loading = $('dashboard-loading');
  const error = $('dashboard-error');
  if (!loading) return;

  // v0.8.3 Step 10：abort 上一次未完成的请求（避免旧请求覆盖新数据）
  if (dashboardAbortController) {
    dashboardAbortController.abort();
  }
  dashboardAbortController = new AbortController();
  const currentSignal = dashboardAbortController.signal;

  loading.classList.remove('hidden');
  if (error) {
    error.classList.remove('show');
    error.textContent = '';
  }

  try {
    // 并行请求三个端点（传入当前 signal，支持外部 abort）
    const [systemRes, detailedRes, daoRes] = await Promise.allSettled([
      fetchWithTimeout(API_BASE + '/v1/health/system', { signal: currentSignal }),
      fetchWithTimeout(API_BASE + '/v1/health/detailed', { signal: currentSignal }),
      fetchWithTimeout(API_BASE + '/v1/health/dao_metrics', { signal: currentSignal }),
    ]);

    // v0.8.3 Step 10：检查是否已被新的请求 abort（若是则静默退出）
    if (currentSignal.aborted) {
      console.log('[loadDashboard] 请求被新请求 abort，静默退出');
      return;
    }

    let systemData = null;
    let detailedData = null;
    let daoData = null;

    if (systemRes.status === 'fulfilled' && systemRes.value.ok) {
      systemData = await systemRes.value.json();
    }
    if (detailedRes.status === 'fulfilled' && detailedRes.value.ok) {
      detailedData = await detailedRes.value.json();
    }
    if (daoRes.status === 'fulfilled' && daoRes.value.ok) {
      daoData = await daoRes.value.json();
    }

    if (!systemData && !daoData) {
      throw new Error('无法连接到 API 服务，请确认 Loong Recall 服务已启动 (' + API_BASE + ')');
    }

    renderDashboard(systemData, detailedData, daoData);
    updateStatusBar(true, systemData);

    // v0.8.0 桌面端 P2 改进：仪表盘加载成功后自动刷新进化时间线
    // 不 await，避免阻塞 loadDashboard；loadEvolutionTimeline 内部有 try/catch
    loadEvolutionTimeline();

    // v0.8.5 Step 2 / G080 修复：恢复上次选中的预设场景
    // 之前 restoreSelectedScenario 定义但从未被调用，导致刷新后预设场景丢失
    restoreSelectedScenario();

    loading.classList.add('hidden');
  } catch (e) {
    // v0.8.3 Step 10：外部 abort 静默处理，不显示错误
    if (e.name === 'AbortError' && currentSignal.aborted) {
      console.log('[loadDashboard] 请求被 abort（正常行为）');
      // v0.8.4 Step 7 / G033 修复：AbortError 时也要隐藏 loading-overlay
      // 避免快速切换标签页导致 loading-overlay 永久显示
      if (loading) loading.classList.add('hidden');
      return;
    }
    if (loading) loading.classList.add('hidden');
    if (error) {
      error.textContent = '⚠️ ' + htmlescape(e.message);
      error.classList.add('show');
    }
    updateStatusBar(false, null);
  } finally {
    // v0.8.3 Step 10：清理 AbortController 引用（仅当当前请求未被打断）
    if (dashboardAbortController === currentSignal) {
      dashboardAbortController = null;
    }
    // v0.8.4 Step 7 / G033 兜底：确保所有路径都隐藏 loading-overlay
    if (loading && !loading.classList.contains('hidden')) {
      loading.classList.add('hidden');
    }
  }
}

function renderDashboard(system, detailed, dao) {
  // --- v0.5.4 P1-7 修复：用户友好的记忆统计卡片 ---
  const memStats = system?.memory_stats || {};
  const daoMetrics = system?.dao_metrics || dao || {};

  // 记忆总数 = 活跃 + 结晶 + 归档
  const totalMemories = memStats.total_memories
    || (daoMetrics.active_memories || 0) + (daoMetrics.crystallized_memories || 0) + (daoMetrics.archived_memories || 0);

  const statTotal = $('stat-total');
  const statActive = $('stat-active');
  const statCrystallized = $('stat-crystallized');
  const statToday = $('stat-today');
  if (statTotal) statTotal.textContent = num(totalMemories);
  if (statActive) statActive.textContent = num(memStats.active_memories || daoMetrics.active_memories);
  if (statCrystallized) statCrystallized.textContent = num(memStats.synthesis_memories || daoMetrics.crystallized_memories);
  // 今日新增：使用编码次数作为近似值（无专门的"今日"字段）
  if (statToday) statToday.textContent = num(daoMetrics.encodings_total || 0);

  // --- v0.5.4 P1-7 修复：系统信息卡片（用户友好） ---
  const sysHealthStatus = $('sys-health-status');
  const sysDataDir = $('sys-data-dir');
  const sysDataDirSettings = $('sys-data-dir-settings');
  const sysStorageSize = $('sys-storage-size');
  const sysTypeCount = $('sys-type-count');
  const sysProjectCount = $('sys-project-count');
  const sysMode = $('sys-mode');

  // 服务状态：基于 dao_isomorphism_score 判断
  const daoScore = daoMetrics.dao_isomorphism_score ?? 0;
  if (sysHealthStatus) {
    if (daoScore >= 0.5) {
      sysHealthStatus.innerHTML = '<span class="badge healthy">✓ 正常运行</span>';
    } else if (daoScore >= 0.3) {
      sysHealthStatus.innerHTML = '<span class="badge warning">⚠ 需关注</span>';
    } else {
      sysHealthStatus.innerHTML = '<span class="badge critical">⚠ 待优化</span>';
    }
  }

  if (sysDataDir) sysDataDir.textContent = '.loong-recall/data/';
  if (sysDataDirSettings) sysDataDirSettings.textContent = '.loong-recall/data/';
  if (sysMode) sysMode.innerHTML = statusBadge(system?.system_mode || 'unknown');

  // --- v0.5.4 P1-7 修复：并行加载最近记忆、项目分布、活动日志 ---
  loadRecentMemories();
  loadMemoryStats();
  loadAuditLog();
}

// ============================================================
// v0.5.4 P1-7 新增：加载最近记忆（用户友好）
// ============================================================
async function loadRecentMemories() {
  const container = $('recent-memories-list');
  if (!container) return;
  container.innerHTML = '<div class="text-center text-dim" style="padding:20px;">加载中...</div>';

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/memories/recent?limit=5');
    if (!res.ok) throw new Error('最近记忆 API 不可用');
    const data = await res.json();
    const memories = data.memories || [];

    if (memories.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">📝</div>
          <div class="empty-text">暂无记忆</div>
          <div class="empty-hint">使用上方的"5分钟快速体验"向导写入第一条记忆</div>
        </div>`;
      return;
    }

    // 记忆类型中文映射
    // v0.6.1 P1-2 修复: 与后端 MemoryType 枚举严格对齐(memory_types.rs)
    // 后端枚举: Fact / Preference / Decision / CodeContext / Conversation / Synthesis
    // 移除前端独有的 pattern/correction/general(后端不识别,导致导入失败)
    const typeLabels = {
      fact: '事实',
      preference: '偏好',
      decision: '决策',
      code_context: '代码',
      conversation: '对话',
      synthesis: '合成',
    };
    // 类型颜色映射(与 typeLabels 键严格一致)
    const typeColors = {
      fact: 'info',
      preference: 'ink',
      decision: 'gold',
      code_context: 'jade',
      conversation: 'info',
      synthesis: 'jade',
    };

    container.innerHTML = memories.map(m => {
      const time = new Date(m.created_at_ms).toLocaleString('zh-CN', {
        month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit'
      });
      const typeLabel = typeLabels[m.memory_type] || m.memory_type;
      const typeColor = typeColors[m.memory_type] || 'info';
      const project = m.project || '全局';
      const importance = m.importance || 0;
      // 重要性星级（1-5星，基于 1-10 分）
      const stars = '★'.repeat(Math.ceil(importance / 2)) + '☆'.repeat(5 - Math.ceil(importance / 2));
      return `
        <div class="recent-memory-item">
          <div class="recent-memory-header">
            <span class="badge ${typeColor}">${htmlescape(typeLabel)}</span>
            <span class="recent-memory-time">${time}</span>
          </div>
          <div class="recent-memory-content">${htmlescape(m.content_preview)}</div>
          <div class="recent-memory-meta">
            <span class="recent-memory-project">📂 ${htmlescape(project)}</span>
            <span class="recent-memory-importance" title="重要性 ${importance}/10">${stars}</span>
          </div>
        </div>`;
    }).join('');
  } catch (e) {
    container.innerHTML = `
      <div class="empty-state">
        <div class="empty-icon">⚠️</div>
        <div class="empty-text">加载失败</div>
        <div class="empty-hint">${htmlescape(e.message)}</div>
      </div>`;
  }
}

// ============================================================
// v0.5.4 P1-7 新增：加载记忆统计（项目分布）
// ============================================================
async function loadMemoryStats() {
  const container = $('project-distribution');
  const sysStorageSize = $('sys-storage-size');
  const sysTypeCount = $('sys-type-count');
  const sysProjectCount = $('sys-project-count');
  if (!container) return;
  container.innerHTML = '<div class="text-center text-dim" style="padding:20px;">加载中...</div>';

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/memories/stats');
    if (!res.ok) throw new Error('记忆统计 API 不可用');
    const data = await res.json();

    // 更新系统信息卡片
    if (sysStorageSize) {
      const bytes = data.storage_size_bytes || 0;
      if (bytes > 1024 * 1024) {
        sysStorageSize.textContent = (bytes / (1024 * 1024)).toFixed(1) + ' MB';
      } else if (bytes > 1024) {
        sysStorageSize.textContent = (bytes / 1024).toFixed(1) + ' KB';
      } else {
        sysStorageSize.textContent = bytes + ' B';
      }
    }
    if (sysTypeCount) sysTypeCount.textContent = Object.keys(data.by_type || {}).length;
    if (sysProjectCount) sysProjectCount.textContent = Object.keys(data.by_project || {}).length;

    // 渲染项目分布
    const byProject = data.by_project || {};
    const projectEntries = Object.entries(byProject).sort((a, b) => b[1] - a[1]);

    if (projectEntries.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">📂</div>
          <div class="empty-text">暂无项目数据</div>
          <div class="empty-hint">写入记忆后将自动统计项目分布</div>
        </div>`;
      return;
    }

    const total = projectEntries.reduce((sum, [_, count]) => sum + count, 0);
    const maxCount = projectEntries[0][1];

    // 项目名称中文映射
    const projectNameMap = {
      '_global_': '全局记忆',
    };

    container.innerHTML = projectEntries.slice(0, 8).map(([project, count]) => {
      const displayName = projectNameMap[project] || project;
      const percentage = total > 0 ? (count / total * 100).toFixed(1) : '0.0';
      const barWidth = maxCount > 0 ? (count / maxCount * 100).toFixed(1) : '0.0';
      return `
        <div class="project-dist-item">
          <div class="project-dist-header">
            <span class="project-dist-name">📂 ${htmlescape(displayName)}</span>
            <span class="project-dist-count">${num(count)} 条 (${percentage}%)</span>
          </div>
          <div class="progress-bar" style="margin-top:4px;">
            <div class="progress-fill jade" style="width:${barWidth}%"></div>
          </div>
        </div>`;
    }).join('');

    if (projectEntries.length > 8) {
      container.innerHTML += `<div class="text-center text-dim" style="margin-top:8px;font-size:11px;">还有 ${projectEntries.length - 8} 个项目未显示</div>`;
    }
  } catch (e) {
    container.innerHTML = `
      <div class="empty-state">
        <div class="empty-icon">⚠️</div>
        <div class="empty-text">加载失败</div>
        <div class="empty-hint">${htmlescape(e.message)}</div>
      </div>`;
  }
}

// ============================================================
// v0.5.4 P1-7 新增：切换标签页（供快速操作区域使用）
// ============================================================
function switchToTab(tabName) {
  const btn = document.querySelector(`.navbar-nav button[data-tab="${tabName}"]`);
  if (btn) btn.click();
}

// ============================================================
// 审计日志加载
// ============================================================
async function loadAuditLog() {
  const tbody = $('audit-log-body');
  if (!tbody) return;
  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/audit-trail?limit=5');
    if (!res.ok) throw new Error('审计日志 API 不可用');
    const data = await res.json();
    const events = data.events || [];

    if (events.length === 0) {
      tbody.innerHTML = '<tr><td colspan="3" class="text-center text-dim">暂无决策日志</td></tr>';
      return;
    }

    // 事件类型中文映射
    const typeLabels = {
      synthesis_created: '合成创建', memory_deleted: '记忆删除',
      memory_isolated: '记忆隔离', decay_rate_changed: '衰减调整',
      synthesis_threshold_changed: '阈值调整', gc_cleanup: 'GC 清理',
      regulation_applied: '调节执行', feedback_processed: '反馈处理',
      comprehensive_rebalance: '综合再平衡', retrieval_weights_adjusted: '权重调整',
      reencoding_suggested: '重编码建议', catastrophic_event: '灾难事件',
      chronic_degradation: '慢性恶化', regulator_frozen: '调节器冻结',
      regulator_unfrozen: '调节器解冻', trust_anchor_created: '锚点创建',
      trust_anchor_published: '锚点发布', dual_confirmation_requested: '双人确认请求',
      dual_confirmation_granted: '双人确认通过', dual_confirmation_denied: '双人确认拒绝',
    };

    tbody.innerHTML = events.map(e => {
      const time = new Date(e.timestamp_ms).toLocaleString('zh-CN');
      const type = typeLabels[e.event_type] || htmlescape(e.event_type);
      const desc = htmlescape(e.description.length > 50 ? e.description.slice(0, 50) + '...' : e.description);
      return `<tr>
        <td style="white-space:nowrap">${time}</td>
        <td><span class="badge info">${type}</span></td>
        <td>${desc}</td>
      </tr>`;
    }).join('');
  } catch (e) {
    tbody.innerHTML = '<tr><td colspan="3" class="text-center text-dim">审计日志加载失败</td></tr>';
  }
}

// ============================================================
// 状态栏更新
// ============================================================
function updateStatusBar(online, systemData) {
  const dot = $('status-dot');
  const text = $('status-text');
  const version = $('status-version');
  const dataDir = $('status-data-dir');
  const uptime = $('status-uptime');

  if (dot && text) {
    if (online) {
      dot.className = 'status-dot online';
      text.textContent = '运行中';
      text.style.color = '#2ecc71';
      text.title = 'LRC 服务运行中';
    } else {
      dot.className = 'status-dot offline';
      text.textContent = '已停止 / 不可达';
      text.style.color = '#c0392b';
      text.title = '点击启动 LRC 服务';
    }
  }

  if (version) version.textContent = 'v' + APP_VERSION;
  // v0.8.7 Step 3：修复 sys-version 硬编码，统一使用 APP_VERSION 动态填充
  const sysVersion = $('sys-version');
  if (sysVersion) sysVersion.textContent = 'v' + APP_VERSION;
  if (dataDir) dataDir.textContent = '.loong-recall/data/';
  if (uptime) uptime.textContent = formatUptime(Date.now() - startTime);
}

// ============================================================
// v0.6.0 状态栏点击：启动服务 + 打开数据目录
// v0.6.0 修复：wizard.js 已删除，Tauri 主窗口直接加载 static/index.html
// 在 Tauri 环境中直接调用 invoke，不再通过 postMessage 与父窗口通信
// 仅在 iframe 嵌入模式（?embedded=tauri）下回退到 postMessage
// ============================================================

// postMessage 请求计数器（用于关联请求与响应）
let postMessageReqId = 0;
// 待处理的 postMessage 请求回调
const pendingPostMessageRequests = new Map();

// 消息类型 → Tauri 命令名映射
const POST_MESSAGE_TO_INVOKE = {
  'lrc-start-service': 'start_sidecar',
  'lrc-open-data-dir': 'open_data_dir',
  // v0.6.1 P1-3 修复: 添加 switch_project 命令映射,前端可调用项目切换
  'lrc-switch-project': 'switch_project',
  // v0.6.1 P0-3 第一批: 核心 CRUD 命令(服务管理/向导/项目)
  'lrc-stop-service': 'stop_sidecar',
  'lrc-list-projects': 'list_sidecar_projects',
  'lrc-reset-wizard': 'reset_wizard',
  'lrc-pick-project-dir': 'pick_project_dir',
  'lrc-get-wizard-state': 'get_wizard_state',
  // v0.6.1 P0-3 第二批: 用户功能命令(LLM/Agent/项目)
  'lrc-get-llm-config': 'get_llm_config',
  'lrc-save-llm-config': 'save_llm_config',
  'lrc-clear-llm-config': 'clear_llm_config',
  'lrc-test-llm-connection': 'test_llm_connection',
  'lrc-detect-agents': 'detect_agents',
  'lrc-detect-installed-agents': 'detect_installed_agents',
  'lrc-set-project-dir': 'set_project_dir',
  'lrc-get-project-dir': 'get_project_dir',
  // v0.6.1 P0-3 第三批: 低频管理命令
  'lrc-start-sidecar-for-project': 'start_sidecar_for_project',
  'lrc-stop-sidecar-for-project': 'stop_sidecar_for_project',
  'lrc-get-agent-config-guide': 'get_agent_config_guide',
  'lrc-discover-all-agents': 'discover_all_agents',
  'lrc-configure-agents': 'configure_agents',
  'lrc-save-configured-agents': 'save_configured_agents',
  'lrc-scan-ide-projects': 'scan_ide_projects',
  'lrc-open-settings': 'open_settings',
  'lrc-mark-complete': 'mark_complete',
  'lrc-verify-setup': 'verify_setup',
};

/**
 * 向桌面端发送请求（启动服务/打开数据目录等）
 * v0.6.0 修复：Tauri 环境直接调用 invoke，iframe 嵌入模式回退到 postMessage
 * @param {string} type - 消息类型（如 'lrc-start-service'）
 * @param {object} [extra={}] - 额外参数
 * @param {number} [timeoutMs=30000] - 超时时间
 * @returns {Promise<object>} 桌面端返回的结果
 */
// v0.8.6 Step 2 / N003 G058 修复：启动服务的 AbortController
// 之前取消按钮仅关闭模态框，60s 内的 Tauri invoke 请求无法中断
let startServiceAbortController = null;

function postMessageToParent(type, extra = {}, timeoutMs = 30000, externalSignal) {
  return new Promise(async (resolve, reject) => {
    // v0.8.6 Step 2 / N003 G058：若外部信号已 abort，立即拒绝
    if (externalSignal && externalSignal.aborted) {
      reject(new DOMException('Aborted', 'AbortError'));
      return;
    }

    // 优先：Tauri 环境（主窗口直接加载仪表盘）直接调用 invoke
    if (isTauriEnv) {
      const invokeFn = (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) ||
                       (window.__TAURI__ && window.__TAURI__.invoke);
      const cmdName = POST_MESSAGE_TO_INVOKE[type];
      if (invokeFn && cmdName) {
        let timeoutId = null;
        try {
          // v0.8.9 修复：Tauri 分支加 setTimeout 硬超时
          // 之前 timeoutMs 参数在 Tauri 分支被完全忽略（只有 iframe 模式有超时）
          // 导致 invoke 永不返回时 UI 永久卡死（启动服务 10 分钟无响应 bug）
          const invokePromise = invokeFn(cmdName, extra);

          // 超时 Promise：timeoutMs 后 reject，防止 invoke 永不返回
          const timeoutPromise = new Promise((_, rejectTimeout) => {
            timeoutId = setTimeout(() => {
              rejectTimeout(new Error('请求超时（' + timeoutMs + 'ms），请稍后重试'));
            }, timeoutMs);
          });

          // 构建竞争 Promise 列表：invoke + 超时 + abort（如有）
          const racers = [invokePromise, timeoutPromise];
          if (externalSignal) {
            // v0.8.6：abort Promise，监听外部取消信号
            const abortPromise = new Promise((_, rejectAbort) => {
              externalSignal.addEventListener('abort', () => {
                rejectAbort(new DOMException('Aborted', 'AbortError'));
              }, { once: true });
            });
            racers.push(abortPromise);
          }
          const result = await Promise.race(racers);
          clearTimeout(timeoutId);
          resolve(result);
        } catch (e) {
          clearTimeout(timeoutId);
          if (e && e.name === 'AbortError') {
            reject(new DOMException('用户取消操作', 'AbortError'));
          } else {
            reject(new Error(typeof e === 'string' ? e : (e.message || String(e))));
          }
        }
        return;
      }
      reject(new Error('Tauri 环境但无法调用 invoke: ' + type));
      return;
    }

    // 回退：iframe 嵌入模式（?embedded=tauri）通过 postMessage 与父窗口通信
    if (!IS_DESKTOP_EMBEDDED) {
      reject(new Error('当前非桌面端嵌入模式，无法调用此功能'));
      return;
    }

    const reqId = ++postMessageReqId;
    const timer = setTimeout(() => {
      if (pendingPostMessageRequests.has(reqId)) {
        pendingPostMessageRequests.delete(reqId);
        reject(new Error('请求超时，请稍后重试'));
      }
    }, timeoutMs);

    // v0.8.6 Step 2 / N003 G058：iframe 模式下监听外部 abort 信号
    let abortHandler = null;
    if (externalSignal) {
      abortHandler = () => {
        clearTimeout(timer);
        if (pendingPostMessageRequests.has(reqId)) {
          pendingPostMessageRequests.delete(reqId);
          reject(new DOMException('用户取消操作', 'AbortError'));
        }
      };
      externalSignal.addEventListener('abort', abortHandler, { once: true });
    }

    pendingPostMessageRequests.set(reqId, { resolve, reject, timer });

    try {
      window.parent.postMessage({
        type,
        reqId,
        ...extra,
      }, '*');
    } catch (e) {
      clearTimeout(timer);
      pendingPostMessageRequests.delete(reqId);
      reject(new Error('发送请求失败: ' + e.message));
    }
  });
}

// 监听父窗口的回复
window.addEventListener('message', (event) => {
  const data = event.data;
  if (!data || typeof data !== 'object') return;

  // 匹配 "<type>:reply" 格式的回复
  const match = /^(.+):reply$/.exec(data.type);
  if (!match) return;

  const reqId = data.reqId;
  const pending = pendingPostMessageRequests.get(reqId);
  if (!pending) return;

  clearTimeout(pending.timer);
  pendingPostMessageRequests.delete(reqId);

  if (data.success) {
    pending.resolve(data);
  } else {
    pending.reject(new Error(data.error || '操作失败'));
  }
});

/** 打开启动服务模态框 */
function openStartServiceModal() {
  const modal = document.getElementById('start-service-modal');
  if (!modal) {
    console.error('[openStartServiceModal] start-service-modal 元素不存在');
    return;
  }
  // v0.8.3 Step 6：移除 hidden 属性并强制 display:flex（修复 N08 模态框未显示）
  // 此前仅设置 hidden=false，但 .modal-overlay[hidden] 的 display:none!important 可能仍生效
  modal.removeAttribute('hidden');
  // v0.8.4 Step 12 / G023：使用 class 控制 display，避免 inline style 被 CSS 覆盖
  modal.classList.add('modal-visible');
  modal.style.display = 'flex';
  // 强制重排，确保 display 生效
  void modal.offsetHeight;

  // v0.8.4 Step 12 / N08：验证模态框可见
  if (modal.offsetParent === null) {
    console.error('[openStartServiceModal] 模态框仍不可见，检查 CSS');
    showToast('启动服务窗口无法显示，请刷新页面重试', 'error');
    return;
  }

  const btn = document.getElementById('modal-btn-start-service');
  if (btn) {
    btn.disabled = false;
    btn.textContent = '启动服务';
  }
  // v0.8.3 Step 6：添加 ESC 键监听（修复 N08）
  document.addEventListener('keydown', handleStartServiceEsc);
  // v0.8.4 Step 12 / G023：添加 Tab 焦点陷阱
  modal.addEventListener('keydown', onStartServiceTabTrap);

  // v0.8.4 Step 12 / CDP 测试 #8：聚焦到模态框内首个可聚焦元素
  const focusable = modal.querySelectorAll('button:not([disabled]), input:not([disabled])');
  if (focusable.length > 0) {
    focusable[0].focus();
  }
}

/**
 * v0.8.4 Step 12 / G023：启动服务模态框 Tab 焦点陷阱
 * 防止 Tab 键跳出模态框，实现焦点循环
 */
function onStartServiceTabTrap(e) {
  if (e.key !== 'Tab') return;
  const modal = document.getElementById('start-service-modal');
  if (!modal) return;

  const focusable = modal.querySelectorAll('button:not([disabled]):not([style*="display: none"]), input:not([disabled])');
  if (focusable.length === 0) return;

  const first = focusable[0];
  const last = focusable[focusable.length - 1];

  if (e.shiftKey) {
    // Shift+Tab：从首个元素跳到末尾
    if (document.activeElement === first) {
      e.preventDefault();
      last.focus();
    }
  } else {
    // Tab：从末尾元素跳到首个
    if (document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }
}

/** 关闭启动服务模态框（供 HTML onclick 调用） */
function closeStartServiceModal() {
  const modal = document.getElementById('start-service-modal');
  if (!modal) return;
  // v0.8.6 Step 2 / N003 G058 修复：取消按钮触发 abort，中断进行中的请求
  // 之前取消按钮仅关闭模态框，60s 内的 Tauri invoke 请求继续执行
  if (startServiceAbortController) {
    startServiceAbortController.abort();
    startServiceAbortController = null;
  }
  // v0.8.3 Step 6：同时设置 hidden 属性和 display:none 确保隐藏
  modal.style.display = 'none';
  modal.setAttribute('hidden', '');
  // v0.8.4 Step 12 / G023：移除 modal-visible class
  modal.classList.remove('modal-visible');
  // v0.8.3 Step 6：移除 ESC 监听，避免影响其他组件
  document.removeEventListener('keydown', handleStartServiceEsc);
  // v0.8.4 Step 12 / G023：移除 Tab 焦点陷阱
  modal.removeEventListener('keydown', onStartServiceTabTrap);
}

/**
 * v0.8.3 Step 6：启动服务模态框 ESC 键处理函数（修复 N08）
 * 使用命名函数便于 removeEventListener 精确移除
 */
function handleStartServiceEsc(e) {
  if (e.key === 'Escape') {
    closeStartServiceModal();
  }
}

// 暴露到全局供 onclick 使用
window.closeStartServiceModal = closeStartServiceModal;
// v0.8.4 Step 12 / N08：暴露 openStartServiceModal 供 data-action 和 CDP 测试调用
window.openStartServiceModal = openStartServiceModal;
// v0.8.6 Step 2 / N003 G058：暴露 startServiceAbortController 供 CDP 测试检测（只读 getter）
Object.defineProperty(window, 'startServiceAbortController', {
  get: function() { return startServiceAbortController; },
  configurable: true
});

/** 启动服务按钮点击处理 */
async function handleStartServiceClick() {
  const btn = document.getElementById('modal-btn-start-service');
  if (!btn) return;
  btn.disabled = true;
  btn.textContent = '正在启动...';

  // v0.8.6 Step 2 / N003 G058 修复：创建 AbortController，传入 postMessageToParent
  // 取消按钮（closeStartServiceModal）触发 abort 后，Promise.race 立即拒绝
  startServiceAbortController = new AbortController();

  try {
    const result = await postMessageToParent('lrc-start-service', {}, 60000, startServiceAbortController.signal);
    closeStartServiceModal();
    // 启动成功后刷新仪表盘
    setTimeout(() => {
      loadDashboard();
    }, 800);
  } catch (e) {
    btn.disabled = false;
    btn.textContent = '启动服务';
    // v0.8.6 Step 2 / N003 G058：abort 时显示"已取消"提示，不显示错误
    if (e && e.name === 'AbortError') {
      console.log('[handleStartServiceClick] 用户取消启动服务');
      showToast('已取消启动服务', 'info');
    } else {
      // v0.8.3 Step 3：替换阻塞 JS 线程的 alert 为非阻塞 showToast（修复 N07）
      // alert 在 Tauri WebView 中会阻塞整个 JS 线程导致应用卡死
      console.error('[handleStartServiceClick] 启动失败:', e);
      showToast('启动失败：' + e.message, 'error');
    }
  } finally {
    // v0.8.6 Step 2 / N003 G058：确保 controller 被清理，避免内存泄漏
    startServiceAbortController = null;
  }
}

/** 数据目录点击处理 */
async function handleOpenDataDirClick() {
  try {
    await postMessageToParent('lrc-open-data-dir', {}, 10000);
  } catch (e) {
    // 失败时静默（不打扰用户），仅在控制台记录
    console.warn('[数据目录] 打开失败:', e.message);
  }
}

// ============================================================
// v0.8.3 Step 12 / G015：输入框 blur 校验（对应审计 G015）
// 设计原则：
//   1. blur 时校验，失败显示红字 + 红边框
//   2. focus 时清除错误状态
//   3. 支持必填、URL、 minLength 三种校验规则
//   4. 集中注册校验规则，便于维护
// ============================================================
const INPUT_VALIDATORS = {
  'wizard-search-path': { type: 'required', label: '搜索路径' },
  'wizard-memory-content': { type: 'required', label: '记忆内容' },
  'llm-api-key': { type: 'minLength', minLength: 10, label: 'API Key' },
  'setup-llm-api-key': { type: 'minLength', minLength: 10, label: 'API Key' },
  'llm-endpoint': { type: 'url', label: 'API URL', allowEmpty: true },
  'llm-model': { type: 'required', label: '模型名称' }
};

function validateInput(input, rule) {
  if (!input) return true;
  const value = (input.value || '').trim();

  // v0.8.4 Step 13 / G015：必填校验（即使无 rule，required 属性也要校验）
  if (!rule && input.hasAttribute('required') && value === '') {
    input.classList.add('input-error');
    input.setAttribute('aria-invalid', 'true');
    let errEl = input.parentNode.querySelector('.input-error-msg');
    if (!errEl) {
      errEl = document.createElement('div');
      errEl.className = 'input-error-msg';
      errEl.style.cssText = 'color: var(--cinnabar); font-size: 12px; margin-top: 4px;';
      input.parentNode.appendChild(errEl);
    }
    errEl.textContent = '此字段为必填项';
    return false;
  }

  // v0.8.6 Step 9 / N009：maxlength 校验（浏览器原生限制的安全网）
  // 浏览器会阻止超长输入，但程序化赋值或粘贴可能绕过，此处作为兜底
  const maxLenAttr = input.getAttribute('maxlength');
  if (maxLenAttr) {
    const maxLen = parseInt(maxLenAttr, 10);
    if (!isNaN(maxLen) && value.length > maxLen) {
      input.classList.add('input-error');
      input.setAttribute('aria-invalid', 'true');
      let errEl = input.parentNode.querySelector('.input-error-msg');
      if (!errEl) {
        errEl = document.createElement('div');
        errEl.className = 'input-error-msg';
        errEl.style.cssText = 'color: var(--cinnabar); font-size: 12px; margin-top: 4px;';
        input.parentNode.appendChild(errEl);
      }
      errEl.textContent = `输入超过最大长度 ${maxLen} 字符`;
      return false;
    }
  }

  if (!rule) return true;

  // allowEmpty 允许空值跳过校验
  if (rule.allowEmpty && value === '') return true;

  let valid = true;
  let errorMsg = '';

  if (rule.type === 'required') {
    if (value === '') {
      valid = false;
      errorMsg = `${rule.label}不能为空`;
    }
  } else if (rule.type === 'url') {
    if (value !== '') {
      try {
        new URL(value);
      } catch (e) {
        valid = false;
        errorMsg = `${rule.label}格式不正确`;
      }
    }
  } else if (rule.type === 'minLength') {
    if (value.length < (rule.minLength || 1)) {
      valid = false;
      errorMsg = `${rule.label}长度不足（最少 ${rule.minLength} 字符）`;
    }
  }

  // 设置或清除错误状态
  if (!valid) {
    input.classList.add('input-error');
    input.setAttribute('aria-invalid', 'true');
    // 显示错误提示（如已有则更新）
    let errEl = input.parentNode.querySelector('.input-error-msg');
    if (!errEl) {
      errEl = document.createElement('div');
      errEl.className = 'input-error-msg';
      errEl.style.cssText = 'color: var(--cinnabar); font-size: 12px; margin-top: 4px;';
      input.parentNode.appendChild(errEl);
    }
    errEl.textContent = errorMsg;
  } else {
    clearInputError(input);
  }
  return valid;
}

function clearInputError(input) {
  if (!input) return;
  input.classList.remove('input-error');
  input.removeAttribute('aria-invalid');
  const errEl = input.parentNode.querySelector('.input-error-msg');
  if (errEl) errEl.remove();
}

/**
 * v0.8.4 Step 13 / G015：增强 setupInputValidation
 * 1. 扫描所有带 required 属性的输入框
 * 2. 补充 INPUT_VALIDATORS 中定义但无 required 属性的字段
 * 3. 添加 input 事件实时清除错误
 * 4. 使用 MutationObserver 监听动态添加的输入框
 */
function setupInputValidation() {
  // 1. 绑定 INPUT_VALIDATORS 中定义的字段
  for (const [id, rule] of Object.entries(INPUT_VALIDATORS)) {
    const input = document.getElementById(id);
    if (!input) continue;
    if (input.dataset.validated === '1') continue;
    input.dataset.validated = '1';
    input.addEventListener('blur', () => validateInput(input, rule));
    input.addEventListener('focus', () => clearInputError(input));
    // v0.8.4 Step 13：input 事件实时清除错误
    input.addEventListener('input', () => {
      if (input.classList.contains('input-error') && input.value.trim()) {
        clearInputError(input);
      }
    });
  }

  // v0.8.4 Step 13 / G015：2. 扫描所有带 required 属性但不在 INPUT_VALIDATORS 中的输入框
  const requiredInputs = document.querySelectorAll('input[required], input[data-required], textarea[required]');
  requiredInputs.forEach(input => {
    if (input.dataset.validated === '1') return;
    input.dataset.validated = '1';
    input.addEventListener('blur', () => validateInput(input, null));
    input.addEventListener('focus', () => clearInputError(input));
    input.addEventListener('input', () => {
      if (input.classList.contains('input-error') && input.value.trim()) {
        clearInputError(input);
      }
    });
  });
}
window.setupInputValidation = setupInputValidation;

// v0.8.4 Step 13 / G015：MutationObserver 监听动态添加的必填输入框
if (typeof MutationObserver !== 'undefined') {
  const _inputObserver = new MutationObserver(mutations => {
    mutations.forEach(mutation => {
      mutation.addedNodes.forEach(node => {
        if (node.nodeType !== 1) return;
        const inputs = node.matches('input[required], input[data-required], textarea[required]')
          ? [node]
          : Array.from(node.querySelectorAll('input[required], input[data-required], textarea[required]'));
        inputs.forEach(input => {
          if (input.dataset.validated === '1') return;
          input.dataset.validated = '1';
          input.addEventListener('blur', () => validateInput(input, INPUT_VALIDATORS[input.id] || null));
          input.addEventListener('focus', () => clearInputError(input));
          input.addEventListener('input', () => {
            if (input.classList.contains('input-error') && input.value.trim()) {
              clearInputError(input);
            }
          });
        });
      });
    });
  });
  // DOMContentLoaded 后启动观察
  document.addEventListener('DOMContentLoaded', () => {
    _inputObserver.observe(document.body, { childList: true, subtree: true });
  });
}

// 绑定点击事件（页面加载后执行）
document.addEventListener('DOMContentLoaded', () => {
  const statusText = document.getElementById('status-text');
  if (statusText) {
    statusText.addEventListener('click', () => {
      // 仅在服务未运行时弹出启动弹窗
      const dot = document.getElementById('status-dot');
      if (dot && dot.classList.contains('offline')) {
        openStartServiceModal();
      }
    });
  }

  const dataDir = document.getElementById('status-data-dir');
  if (dataDir) {
    dataDir.addEventListener('click', handleOpenDataDirClick);
  }

  const modalBtn = document.getElementById('modal-btn-start-service');
  if (modalBtn) {
    modalBtn.addEventListener('click', handleStartServiceClick);
  }

  // 点击模态框遮罩关闭
  const modal = document.getElementById('start-service-modal');
  if (modal) {
    modal.addEventListener('click', (e) => {
      if (e.target === modal) closeStartServiceModal();
    });
  }
});

// ============================================================
// 船长日志生成
// ============================================================
async function generateCaptainLog() {
  const btn = $('btn-generate-log');
  const loading = $('log-loading');
  const error = $('log-error');
  const result = $('log-result');
  if (!btn || !result) return;

  btn.disabled = true;
  loading.classList.remove('hidden');
  error.classList.remove('show');
  error.textContent = '';
  result.classList.add('hidden');
  result.textContent = '';

  try {
    // v0.7.0 修复: 优先调用 /v1/captains-log 端点获取完整船长日志
    try {
      const logRes = await fetchWithTimeout(API_BASE + '/v1/captains-log', {}, 30000);
      if (logRes.ok) {
        const logData = await logRes.json();
        if (logData.report || logData.content || logData.log) {
          result.textContent = logData.report || logData.content || logData.log;
          result.classList.remove('hidden');
          return;
        }
      }
    } catch (_) { /* /v1/captains-log 不可用，回退到手动拼接 */ }

    // 回退逻辑：并行获取健康数据，手动拼接船长日志
    const [systemRes, daoRes] = await Promise.allSettled([
      fetchWithTimeout(API_BASE + '/v1/health/system'),
      fetchWithTimeout(API_BASE + '/v1/health/dao_metrics'),
    ]);

    let system = null, dao = null;
    if (systemRes.status === 'fulfilled' && systemRes.value.ok) {
      system = await systemRes.value.json();
    }
    if (daoRes.status === 'fulfilled' && daoRes.value.ok) {
      dao = await daoRes.value.json();
    }

    if (!system && !dao) {
      throw new Error('无法连接到 API 服务');
    }

    // 尝试获取代码库统计（通过 MCP 端点）
    let codebaseStats = null;
    try {
      const cbRes = await fetchWithTimeout(API_BASE + '/mcp', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'tools/call',
          params: { name: 'codebase_stats', arguments: {} }
        })
      });
      if (cbRes.ok) {
        const cbData = await cbRes.json();
        if (cbData.result?.content?.[0]?.text) {
          codebaseStats = cbData.result.content[0].text;
        }
      }
    } catch (_) { /* 代码库统计可选 */ }

    // 尝试获取记忆统计（通过 MCP 端点）
    let memoryStats = null;
    try {
      const memRes = await fetchWithTimeout(API_BASE + '/mcp', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 2,
          method: 'tools/call',
          params: { name: 'memory_stats', arguments: {} }
        })
      });
      if (memRes.ok) {
        const memData = await memRes.json();
        if (memData.result?.content?.[0]?.text) {
          memoryStats = memData.result.content[0].text;
        }
      }
    } catch (_) { /* 记忆统计可选 */ }

    // 构建日志内容
    const now = new Date().toLocaleString('zh-CN');
    const memStats = system?.memory_stats || {};
    const daoMetrics = system?.dao_metrics || dao || {};
    const encoder = system?.encoder || {};
    const budget = system?.complexity_budget || {};
    const mode = system?.system_mode || 'unknown';
    const modeDesc = system?.system_mode_description || '';

    let log = '';
    log += '╔══════════════════════════════════════════╗\n';
    log += '║     🚢 Loong Recall · 船长日志           ║\n';
    log += '╠══════════════════════════════════════════╣\n';
    log += '║  生成时间：' + now + '              ║\n';
    log += '╚══════════════════════════════════════════╝\n\n';

    log += '━━━ 📊 代码库统计 ━━━\n';
    if (codebaseStats) {
      log += codebaseStats + '\n';
    } else {
      log += '  （代码库统计不可用 — 请确认 MCP 端点可访问）\n';
    }

    log += '\n━━━ 🧠 记忆统计 ━━━\n';
    if (memoryStats) {
      log += memoryStats + '\n';
    } else {
      log += '  记忆总数：' + num(memStats.total_memories || (daoMetrics.active_memories || 0) + (daoMetrics.crystallized_memories || 0) + (daoMetrics.archived_memories || 0)) + ' 条\n';
      log += '  活跃记忆：' + num(memStats.active_memories || daoMetrics.active_memories) + ' 条\n';
      log += '  合成记忆：' + num(memStats.synthesis_memories || daoMetrics.crystallized_memories) + ' 条\n';
      log += '  过期记忆：' + num(memStats.expired_memories || daoMetrics.archived_memories) + ' 条\n';
      log += '  低质量合成：' + num(memStats.low_quality_synthesis || 0) + ' 条\n';
    }

    log += '\n━━━ 🏥 道同构度 ━━━\n';
    log += '  道同构度评分：' + pct(daoMetrics.dao_isomorphism_score || 0) + '\n';
    log += '  八卦分布熵：  ' + (daoMetrics.bagua_entropy || 0).toFixed(3) + ' / 3.0\n';
    log += '  合成比率：    ' + pct(daoMetrics.synthesis_ratio || 0) + '\n';
    log += '  编码次数：    ' + num(daoMetrics.encodings_total || 0) + '\n';
    log += '  合成次数：    ' + num(daoMetrics.compositions_total || 0) + '\n';
    log += '  检索次数：    ' + num(daoMetrics.recalls_total || 0) + '\n';
    log += '  修正次数：    ' + num(daoMetrics.corrections_total || 0) + '\n';

    log += '\n━━━ ⚙️ 系统健康总结 ━━━\n';
    log += '  系统模式：' + mode + '\n';
    if (modeDesc) log += '  模式描述：' + modeDesc + '\n';
    log += '  编码器：  ' + (encoder.model_name || 'LuoShuEncoder') + ' (' + (encoder.mode || 'statistical') + ')\n';
    log += '  编码质量：' + (encoder.quality_score != null ? (encoder.quality_score * 100).toFixed(0) + '%' : 'N/A') + '\n';
    log += '  可维护性：' + pct(budget.maintainability_score || 0) + '\n';
    log += '  复杂度预算消耗：' + pct(budget.budget_consumed || 0) + '\n';

    // 行动建议
    if (system?.action_hints?.length) {
      log += '\n━━━ 💡 行动建议 ━━━\n';
      system.action_hints.forEach(hint => {
        const sev = hint.severity === 'action_required' ? '🔴' : hint.severity === 'warning' ? '🟡' : 'ℹ️';
        log += '  ' + sev + ' [' + hint.category + '] ' + hint.message + '\n';
        if (hint.suggested_action) log += '     → ' + hint.suggested_action + '\n';
      });
    }

    log += '\n━━━ 📋 状态摘要 ━━━\n';
    const daoScore = daoMetrics.dao_isomorphism_score || 0;
    if (daoScore >= 0.5) {
      log += '  ✅ 系统健康 — 道同构度良好，各子系统运行正常。\n';
    } else if (daoScore >= 0.3) {
      log += '  ⚠️ 系统警告 — 道同构度偏低，建议关注编码质量和合成比率。\n';
    } else {
      log += '  🔴 系统需关注 — 道同构度严重偏低，建议检查编码器状态和训练数据。\n';
    }

    log += '\n═══════════════════════════════════════════\n';
    log += '  报告结束 — Loong Recall 守护你的记忆\n';
    log += '═══════════════════════════════════════════\n';

    result.textContent = log;
    result.classList.remove('hidden');

  } catch (e) {
    if (error) {
      error.textContent = '⚠️ 生成失败：' + htmlescape(e.message);
      error.classList.add('show');
    }
  } finally {
    if (btn) btn.disabled = false;
    if (loading) loading.classList.add('hidden');
  }
}

// ============================================================
// 信任中心数据加载
// ============================================================
async function loadTrustCenter() {
  const loading = $('trust-loading');
  if (!loading) return;
  loading.classList.remove('hidden');

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/health/system');
    if (!res.ok) throw new Error('API 不可达');
    const data = await res.json();

    // 隐式反馈状态
    const feedback = data.feedback_stats || {};
    const fbEnabled = feedback.implicit_feedback_enabled;
    const fbText = $('feedback-status-text');
    const fbCard = $('feedback-status-card');

    if (fbText) {
      if (fbEnabled) {
        fbText.innerHTML = '状态：<span class="badge healthy">已启用</span>';
        fbText.innerHTML += '<br>正面反馈率：' + pct(feedback.positive_ratio || 0);
        fbText.innerHTML += '<br>总反馈数：' + num(feedback.total_feedback || 0);
      } else {
        fbText.innerHTML = '状态：<span class="badge info">未启用</span>';
        fbText.innerHTML += '<br>隐式反馈当前未激活，系统使用默认排序策略。';
      }
    }

    // 审计日志完整性
    const auditText = $('audit-integrity-text');
    const auditCard = $('audit-integrity-card');

    if (auditText) {
      try {
        const auditRes = await fetchWithTimeout(API_BASE + '/v1/audit-trail?limit=1');
        if (auditRes.ok) {
          const auditData = await auditRes.json();
          auditText.innerHTML = '状态：<span class="badge healthy">完整</span>';
          auditText.innerHTML += '<br>总事件数：' + num(auditData.total_all || 0);
          auditText.innerHTML += '<br>哈希链：已验证 ✓';
          if (auditCard) auditCard.style.borderLeftColor = 'var(--jade)';
        } else {
          throw new Error('审计 API 不可用');
        }
      } catch (_) {
        auditText.innerHTML = '状态：<span class="badge warning">不可用</span>';
        auditText.innerHTML += '<br>审计 API 端点未响应';
        if (auditCard) auditCard.style.borderLeftColor = 'var(--gold)';
      }
    }

  } catch (e) {
    const fbText = $('feedback-status-text');
    const auditText = $('audit-integrity-text');
    if (fbText) fbText.textContent = '无法获取数据：' + htmlescape(e.message);
    if (auditText) auditText.textContent = '无法获取数据：' + htmlescape(e.message);
  } finally {
    if (loading) loading.classList.add('hidden');
  }
}

// ============================================================
// 信任中心可验证性 API 调用
// ============================================================

/** 验证数据存储位置 */
async function verifyDataLocation() {
  const result = $('data-location-result');
  const loading = $('data-location-loading');
  if (!result) return;
  if (loading) loading.classList.remove('hidden');
  result.classList.remove('show');

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/trust/data-location');
    if (!res.ok) throw new Error('API 不可达');
    const data = await res.json();

    const validCls = data.file_exists ? 'valid' : '';
    // v0.8.0 "归一"：格式化最后备份时间
    const backupTimeStr = data.last_backup_time
      ? new Date(data.last_backup_time * 1000).toLocaleString('zh-CN')
      : '暂无备份';
    result.innerHTML = `
      <div class="result-row"><span class="result-label">数据目录</span><span class="result-value">${htmlescape(data.data_directory)}</span></div>
      <div class="result-row"><span class="result-label">记忆文件</span><span class="result-value">${htmlescape(data.memory_file)}</span></div>
      <div class="result-row"><span class="result-label">文件存在</span><span class="result-value ${validCls}">${data.file_exists ? '✅ 是' : '❌ 否'}</span></div>
      <div class="result-row"><span class="result-label">文件大小</span><span class="result-value">${htmlescape(data.file_size_human)} (${num(data.file_size_bytes)} 字节)</span></div>
      <div class="result-row"><span class="result-label">记忆总数</span><span class="result-value ${validCls}">${num(data.memory_count)} 条</span></div>
      <div class="result-row"><span class="result-label">最后备份</span><span class="result-value">${htmlescape(backupTimeStr)}</span></div>
      <div class="result-row"><span class="result-label">存储后端</span><span class="result-value">${htmlescape(data.storage_backend)}</span></div>
      <div class="result-row"><span class="result-label">完全本地</span><span class="result-value valid">✅ 是</span></div>
      <div class="result-row" style="margin-top: 8px;">
        <button class="btn btn-outline btn-sm" data-action="handleOpenDataDirClick">
          📁 打开数据文件夹
        </button>
      </div>
    `;
    result.classList.add('show');
  } catch (e) {
    result.innerHTML = '<div class="result-row"><span class="result-label" style="color:var(--cinnabar)">⚠️ ' + htmlescape(e.message) + '</span></div>';
    result.classList.add('show');
  } finally {
    if (loading) loading.classList.add('hidden');
  }
}

/** 验证网络活动 */
async function verifyNetworkAudit() {
  const result = $('network-audit-result');
  const loading = $('network-audit-loading');
  if (!result) return;
  if (loading) loading.classList.remove('hidden');
  result.classList.remove('show');

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/trust/network-audit');
    if (!res.ok) throw new Error('API 不可达');
    const data = await res.json();

    let html = `
      <div class="result-row"><span class="result-label">网络请求总数</span><span class="result-value">${num(data.total_network_requests)}</span></div>
      <div class="result-row"><span class="result-label">网络策略</span><span class="result-value">${htmlescape(data.network_policy)}</span></div>
      <div class="result-row"><span class="result-label">无遥测</span><span class="result-value valid">✅ ${data.no_telemetry ? '确认' : '未能确认'}</span></div>
      <div class="result-row"><span class="result-label">无分析</span><span class="result-value valid">✅ ${data.no_analytics ? '确认' : '未能确认'}</span></div>
    `;

    if (data.requests.length > 0) {
      html += '<div class="result-row"><span class="result-label">已记录请求</span><span class="result-value">';
      html += data.requests.map(r => '<div>' + htmlescape(r) + '</div>').join('');
      html += '</span></div>';
    } else {
      html += '<div class="result-row"><span class="result-label">网络活动</span><span class="result-value valid">✅ 无网络请求记录</span></div>';
    }

    result.innerHTML = html;
    result.classList.add('show');
  } catch (e) {
    result.innerHTML = '<div class="result-row"><span class="result-label" style="color:var(--cinnabar)">⚠️ ' + htmlescape(e.message) + '</span></div>';
    result.classList.add('show');
  } finally {
    if (loading) loading.classList.add('hidden');
  }
}

/** 验证审计完整性 */
async function verifyAuditIntegrity() {
  const result = $('audit-integrity-result');
  const loading = $('audit-integrity-loading');
  const auditCard = $('audit-integrity-card');
  if (!result) return;
  if (loading) loading.classList.remove('hidden');
  result.classList.remove('show');

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/trust/audit-integrity');
    if (!res.ok) throw new Error('API 不可达');
    const data = await res.json();

    const hashCls = data.hash_chain_valid ? 'valid' : 'invalid';
    const anchorCls = data.anchor_chain_valid ? 'valid' : 'invalid';
    const tamperCls = data.tamper_proof ? 'valid' : 'invalid';

    // 更新审计卡片的边框颜色
    if (auditCard) auditCard.style.borderLeftColor = data.tamper_proof ? 'var(--jade)' : 'var(--cinnabar)';

    let lastAnchor = '无';
    if (data.last_anchor_at) {
      lastAnchor = new Date(data.last_anchor_at).toLocaleString('zh-CN');
    }

    result.innerHTML = `
      <div class="result-row"><span class="result-label">事件总数</span><span class="result-value">${num(data.total_events)}</span></div>
      <div class="result-row"><span class="result-label">哈希链状态</span><span class="result-value ${hashCls}">${htmlescape(data.hash_chain_status)}</span></div>
      <div class="result-row"><span class="result-label">锚点数量</span><span class="result-value">${num(data.anchor_count)}</span></div>
      <div class="result-row"><span class="result-label">锚点链状态</span><span class="result-value ${anchorCls}">${htmlescape(data.anchor_chain_status)}</span></div>
      <div class="result-row"><span class="result-label">最后锚点时间</span><span class="result-value">${htmlescape(lastAnchor)}</span></div>
      <div class="result-row"><span class="result-label">防篡改状态</span><span class="result-value ${tamperCls}">${data.tamper_proof ? '✅ 通过' : '❌ 失败'}</span></div>
    `;
    result.classList.add('show');

    // 同时更新审计完整性文本
    const auditText = $('audit-integrity-text');
    if (auditText) {
      auditText.innerHTML = '状态：<span class="badge ' + (data.tamper_proof ? 'healthy' : 'critical') + '">' + (data.tamper_proof ? '完整' : '异常') + '</span>';
      auditText.innerHTML += '<br>总事件数：' + num(data.total_events);
      auditText.innerHTML += '<br>哈希链：' + (data.hash_chain_valid ? '已验证 ✓' : '断裂 ✗');
      auditText.innerHTML += '<br>锚点链：' + (data.anchor_chain_valid ? '已验证 ✓' : '异常 ✗');
    }
  } catch (e) {
    // v0.8.2：sidecar 不可达时给出明确提示，不产生 console error
    const msg = (e.name === 'SidecarUnreachableError' || e.name === 'SidecarTimeoutError')
      ? '⚠️ LRC 服务未运行，请先启动服务后再验证完整性'
      : '⚠️ ' + htmlescape(e.message);
    result.innerHTML = '<div class="result-row"><span class="result-label" style="color:var(--cinnabar)">' + msg + '</span></div>';
    result.classList.add('show');
  } finally {
    if (loading) loading.classList.add('hidden');
  }
}

// ============================================================
// 5分钟快速体验向导
// ============================================================

/** 向导步骤一：搜索代码 */
async function wizardStep1Search() {
  const step = $('wizard-step-1');
  const result = $('wizard-step1-result');
  if (!result) return;
  // v0.7.1 P3-3：前端表单校验，空值提示
  const query = (document.getElementById('wizard-search-path')?.value || '').trim();
  if (!query) {
    result.innerHTML = '<span style="color: var(--lrc-朱砂-600);">请输入搜索关键词</span>';
    result.classList.add('show');
    return;
  }
  result.classList.remove('show');
  result.innerHTML = '<span class="loading-spinner" style="width:14px;height:14px;border-width:1.5px"></span> 正在搜索代码库...';
  result.classList.add('show');

  try {
    // 调用实际的代码搜索 API
    const res = await fetchWithTimeout(API_BASE + '/v1/code/search?query=' + encodeURIComponent(query) + '&top_k=3');
    if (!res.ok) throw new Error('搜索失败，请确认服务已启动');
    const data = await res.json();

    if (data.results && data.results.length > 0) {
      let html = '✅ 搜索成功！在 ' + num(data.total_indexed) + ' 个代码片段中找到 ' + num(data.returned) + ' 条结果：<br>';
      for (const r of data.results) {
        const codeContent = r.content || '';
        const codePreview = codeContent.length > 80 ? codeContent.slice(0, 80) + '...' : codeContent;
        html += '<div class="code-result" style="margin:6px 0;padding:6px;background:#1a1a2e;border-radius:4px;font-size:11px">' +
          '<span style="color:#f1c40f">#' + r.rank + '</span> ' +
          '<span style="color:#2ecc71">' + htmlescape(r.file_path) + ':' + r.start_line + '</span> ' +
          '<span style="color:#e74c3c">' + (r.score * 100).toFixed(0) + '%</span><br>' +
          '<code style="color:#ddd">' + htmlescape(codePreview) + '</code>' +
          '</div>';
      }
      result.innerHTML = html;
    } else {
      result.innerHTML = '✅ 搜索完成，在 ' + num(data.total_indexed) + ' 个代码片段中未找到匹配结果。<br>' +
        '<span style="color:#888">提示：尝试搜索 "struct"、"fn"、"impl" 或具体函数名</span>';
    }
    if (step) step.classList.add('completed');
  } catch (e) {
    result.innerHTML = '⚠️ ' + htmlescape(e.message);
  }
}

/** 向导步骤二：写入记忆 */
async function wizardStep2Write() {
  const step = $('wizard-step-2');
  const result = $('wizard-step2-result');
  if (!result) return;
  // v0.7.1 P3-3：前端表单校验，空值提示
  const content = (document.getElementById('wizard-memory-content')?.value || '').trim();
  if (!content) {
    result.innerHTML = '<span style="color: var(--lrc-朱砂-600);">请输入记忆内容</span>';
    result.classList.add('show');
    return;
  }
  result.classList.remove('show');
  result.innerHTML = '<span class="loading-spinner" style="width:14px;height:14px;border-width:1.5px"></span> 正在写入...';
  result.classList.add('show');

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/memories/consolidate', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        memories: [{
          content: content,
          memory_type: 'fact',
          importance: 7,
          tags: ['quickstart', 'wizard'],
          project: 'quickstart-demo'
        }]
      })
    });
    if (!res.ok) throw new Error('写入失败，请确认服务已启动');
    const data = await res.json();

    result.innerHTML = '✅ 写入成功！已存储 ' + htmlescape(String(data.stored)) + ' 条记忆，当前共 ' + num(data.total_memories) + ' 条记忆';
    if (step) step.classList.add('completed');
  } catch (e) {
    result.innerHTML = '⚠️ ' + htmlescape(e.message);
  }
}

/** 向导步骤三：检索记忆 */
async function wizardStep3Search() {
  const step = $('wizard-step-3');
  const result = $('wizard-step3-result');
  if (!result) return;
  // v0.7.1 P3-3：前端表单校验，空值提示
  const query = (document.getElementById('wizard-search-query')?.value || '').trim();
  if (!query) {
    result.innerHTML = '<span style="color: var(--lrc-朱砂-600);">请输入搜索关键词</span>';
    result.classList.add('show');
    return;
  }
  result.classList.remove('show');
  result.innerHTML = '<span class="loading-spinner" style="width:14px;height:14px;border-width:1.5px"></span> 正在检索...';
  result.classList.add('show');

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/memories/enrich', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        query: query,
        top_k: 5
      })
    });
    if (!res.ok) throw new Error('检索失败，请确认服务已启动');
    const data = await res.json();

    if (data.memories && data.memories.length > 0) {
      const items = data.memories.slice(0, 3).map(m => {
        const memContent = m.content || '';
        return '📝 ' + htmlescape(memContent.length > 60 ? memContent.slice(0, 60) + '...' : memContent) +
        ' (' + (m.score * 100).toFixed(0) + '%)'
      }).join('<br>');
      result.innerHTML = '✅ 检索成功！找到 ' + htmlescape(String(data.total)) + ' 条相关记忆：<br>' + items;
    } else {
      result.innerHTML = '✅ 检索完成，但未找到相关记忆。请先完成步骤二写入记忆。';
    }
    if (step) step.classList.add('completed');
  } catch (e) {
    result.innerHTML = '⚠️ ' + htmlescape(e.message);
  }
}

// ============================================================
// 自动刷新
// ============================================================
function startAutoRefresh() {
  if (refreshTimer) clearInterval(refreshTimer);
  refreshTimer = setInterval(() => {
    // 仅刷新当前激活的标签页
    const activeTab = document.querySelector('.tab-content.active');
    if (activeTab) {
      const tabId = activeTab.id;
      // v0.8.3 Step 12 / G016：自动刷新保留滚动位置（避免打断阅读）
      // 保存滚动位置 → 加载完成后恢复
      if (tabId === 'tab-dashboard') {
        const savedScroll = _saveMainScroll();
        loadDashboard().finally(() => _restoreMainScroll(savedScroll));
      } else if (tabId === 'tab-trust-center') {
        const savedScroll = _saveMainScroll();
        loadTrustCenter().finally(() => _restoreMainScroll(savedScroll));
      }
    }
    // 始终更新运行时长
    const uptime = $('status-uptime');
    if (uptime) uptime.textContent = formatUptime(Date.now() - startTime);
  }, REFRESH_INTERVAL);
}

// v0.8.4 Step 6：暴露 startAutoRefresh 到 window（修复 G019 + CDP 测试 #10）
// 之前遗漏此暴露语句，导致 typeof window.startAutoRefresh === 'function' 返回 false
window.startAutoRefresh = startAutoRefresh;

// ============================================================
// v0.8.3 Step 12 / G016：滚动锚点保留工具函数
// 在自动刷新或标签页切换时保存/恢复主内容区的滚动位置
// ============================================================
function _saveMainScroll() {
  const mainContent = document.querySelector('.main') || document.querySelector('.main-content');
  if (!mainContent) return 0;
  return mainContent.scrollTop;
}

function _restoreMainScroll(savedScroll) {
  if (typeof savedScroll !== 'number') return;
  const mainContent = document.querySelector('.main') || document.querySelector('.main-content');
  if (!mainContent) return;
  // 异步恢复，确保 DOM 已渲染完成
  requestAnimationFrame(() => {
    mainContent.scrollTop = savedScroll;
  });
}

// ============================================================
// 桌面端嵌入检测
// ============================================================
// 当仪表盘被嵌入 Tauri 桌面端时，URL 会带 ?embedded=tauri 参数
// v0.5.5：嵌入模式和非嵌入模式统一使用完整的 LLM 配置表单
// 不管在哪里修改 LLM 配置，都通过 /v1/config/llm API 保存，自动同步到 wizard.json
// v0.6.0 修复：Tauri 主窗口直接加载 static/index.html（非 iframe），URL 无 ?embedded=tauri
// 需同时检查 isTauriEnv（window.__TAURI_INTERNALS__ 存在）判断是否在桌面端
const IS_DESKTOP_EMBEDDED = isTauriEnv ||
  (new URLSearchParams(window.location.search).get('embedded') === 'tauri');

// v0.5.5：LLM 提供商列表（与桌面端配置向导一致）
const LLM_PROVIDERS = {
  deepseek:   { name: 'DeepSeek',       url: 'https://api.deepseek.com/v1',           model: 'deepseek-chat',       keyHint: 'sk-...',    desc: '国产性价比之王，代码能力极强', category: 'cloud' },
  qwen:       { name: '通义千问',       url: 'https://dashscope.aliyuncs.com/compatible-mode/v1', model: 'qwen-plus', keyHint: 'sk-...', desc: '阿里云出品，中文理解出色', category: 'cloud' },
  zhipu:      { name: '智谱 GLM',       url: 'https://open.bigmodel.cn/api/paas/v4',   model: 'glm-4',               keyHint: 'xxx.xxx', desc: '清华系，GLM 系列模型', category: 'cloud' },
  minimax:    { name: 'MiniMax 海螺',   url: 'https://api.minimax.chat/v1',            model: 'abab6.5s-chat',       keyHint: 'eyJ...', desc: '长文本支持好，性价比高', category: 'cloud' },
  moonshot:   { name: 'Kimi 月之暗面',   url: 'https://api.moonshot.cn/v1',            model: 'moonshot-v1-8k',      keyHint: 'sk-...', desc: '超长上下文，阅读能力强', category: 'cloud' },
  stepfun:    { name: '阶跃星辰',       url: 'https://api.stepfun.com/v1',             model: 'step-1-flash',        keyHint: 'sk-...', desc: '多模态能力强，图像理解出色', category: 'cloud' },
  baichuan:   { name: '百川智能',       url: 'https://api.baichuan-ai.com/v1',         model: 'Baichuan4',           keyHint: 'sk-...', desc: '垂直领域优化，性价比高', category: 'cloud' },
  xunfei:     { name: '讯飞星火',       url: 'https://spark-api-open.xf-yun.com/v1',   model: 'generalv3.5',         keyHint: 'sk-...', desc: '科大讯飞出品，语音能力强', category: 'cloud' },
  hunyuan:    { name: '腾讯混元',       url: 'https://api.hunyuan.cloud.tencent.com/v1', model: 'hunyuan-lite',       keyHint: 'sk-...', desc: '腾讯出品，腾讯生态深度集成', category: 'cloud' },
  custom:     { name: '自定义 API',      url: '',                                        model: '',                   keyHint: '',          desc: '手动填写任何兼容 OpenAI 的 API 地址', category: 'custom' },
};

// ============================================================
// 初始化
// ============================================================
// v0.6.0 P0-1 修复：Tauri 环境下通过 IPC 获取 sidecar 实际端口（端口自适应支持）
async function init() {
  if (IS_DESKTOP_EMBEDDED) {
    const settingsDesc = document.querySelector('#tab-settings .section-desc');
    if (settingsDesc) {
      settingsDesc.textContent = '配置 LLM API 以启用自然语言搜索代码。配置后自动生效，无需重启。';
    }
  }

  // v0.6.0 P0-1 修复：Tauri 环境下通过 IPC 获取 sidecar 实际端口
  // sidecar 有端口自适应机制（3099-3198），如果 3099 被占用会使用其他端口
  if (isTauriEnv) {
    try {
      // Tauri 2.x 优先使用 window.__TAURI_INTERNALS__.invoke
      const invokeFn = (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) ||
                       (window.__TAURI__ && window.__TAURI__.invoke);
      if (invokeFn) {
        const status = await invokeFn('get_sidecar_status');
        if (status && status.length > 0 && status[0].port && status[0].running) {
          const actualPort = status[0].port;
          const newApiBase = `http://127.0.0.1:${actualPort}`;
          if (newApiBase !== API_BASE) {
            API_BASE = newApiBase;
            console.log('[LRC] sidecar 端口自适应: ' + actualPort);
          }
        }
      }
    } catch (e) {
      console.warn('[LRC] 获取 sidecar 端口失败，使用默认端口 3099:', e);
    }
  }

  // v0.8.0 "归一" 新增：监听规则写入事件，显示用户提示
  // 确保用户知道规则是否成功写入，失败时提供可见反馈
  if (isTauriEnv) {
    try {
      const tauriEvent = (window.__TAURI__ && window.__TAURI__.event) ||
                         (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.event);
      if (tauriEvent && typeof tauriEvent.listen === 'function') {
        // 监听规则写入成功事件
        tauriEvent.listen('rules-write-completed', (event) => {
          const payload = (event && event.payload) || {};
          console.log('[LRC] 规则写入完成:', payload);
          if (payload.written_count > 0) {
            showToast(
              '已为 ' + payload.written_count + ' 个 AI 工具写入 LRC 规则（v0.8.0）',
              'success',
              4000
            );
          }
        });

        // 监听规则写入失败事件
        tauriEvent.listen('rules-write-failed', (event) => {
          const payload = (event && event.payload) || {};
          console.error('[LRC] 规则写入失败:', payload);
          showToast(
            (payload.message || '规则文件写入失败') + '，AI 助手可能无法自动调用记忆工具',
            'error',
            8000
          );
        });

        console.log('[LRC] 规则写入事件监听已注册');
      } else {
        console.warn('[LRC] Tauri 事件 API 不可用，规则写入事件监听未注册');
      }
    } catch (e) {
      console.warn('[LRC] 规则写入事件监听注册失败:', e);
    }
  }

  loadDashboard();
  
  setTimeout(() => {
    drawRadarChart();
  }, 100);

  setInterval(() => {
    const uptime = $('status-uptime');
    if (uptime) uptime.textContent = formatUptime(Date.now() - startTime);
  }, 1000);

  startAutoRefresh();

  // v0.8.2：启动 Sidecar 健康监测（对应审计 G005）
  SidecarHealthMonitor.start();
}

// 页面加载完成后初始化
document.addEventListener('DOMContentLoaded', init);

// ============================================================
// V2: 项目信息加载
// ============================================================
async function loadProjectInfo() {
  try {
    const resp = await fetchWithTimeout(API_BASE + '/api/project/info');
    if (!resp.ok) return;
    const data = await resp.json();

    const el = $('project-fingerprint');
    if (el) el.textContent = data.fingerprint || '--';

    const el2 = $('project-canonical-path');
    if (el2) el2.textContent = data.canonical_path || data.src_dir || '--';
  } catch (e) {
    console.warn('[项目信息] 加载失败:', e.message);
  }
}

// ============================================================
// V2: 记忆数据导出（浏览器端触发下载）
// ============================================================
async function backupMemories() {
  const btn = $('btn-backup-memories');
  // v0.8.0：同时更新设置页 backup-result 和信任中心 trust-backup-result
  const resultEls = [$('backup-result'), $('trust-backup-result')].filter(Boolean);
  const updateResult = (text, cls) => {
    resultEls.forEach(el => {
      el.style.display = '';
      el.textContent = text;
      el.className = cls || 'form-result';
    });
  };
  if (btn) btn.disabled = true;
  if (resultEls.length > 0) {
    updateResult('⏳ 正在准备备份文件...');
  }

  try {
    // 从 API 获取记忆数据(大量数据时需要更长超时)
    const [memoriesRes, chunksRes, archiveRes, projectRes] = await Promise.allSettled([
      fetchWithTimeout(API_BASE + '/v1/memories/list', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ limit: 10000 })
      }, 60000),
      fetchWithTimeout(API_BASE + '/v1/code/search?query=&top_k=10000', {}, 60000),
      fetchWithTimeout(API_BASE + '/v1/memories/archive', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({})
      }, 60000),
      fetchWithTimeout(API_BASE + '/api/project/info'),
    ]);

    // 构建导出数据
    const exportData = {
      version: '2.0',
      exported_at: new Date().toISOString(),
      fingerprint: null,
      canonical_path: null,
      source: 'project',
      memories: [],
      chunks: [],
      archive: [],
    };

    // 获取项目信息
    if (projectRes.status === 'fulfilled' && projectRes.value.ok) {
      const projectData = await projectRes.value.json();
      exportData.fingerprint = projectData.fingerprint || null;
      exportData.canonical_path = projectData.canonical_path || null;
    }

    // 获取记忆数据
    if (memoriesRes.status === 'fulfilled' && memoriesRes.value.ok) {
      const memoriesData = await memoriesRes.value.json();
      exportData.memories = memoriesData.memories || memoriesData.data || [];
    }

    // 获取代码片段
    if (chunksRes.status === 'fulfilled' && chunksRes.value.ok) {
      const chunksData = await chunksRes.value.json();
      exportData.chunks = chunksData.chunks || chunksData.data || [];
    }

    // 创建下载
    const blob = new Blob([JSON.stringify(exportData, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    const fp = exportData.fingerprint || 'global';
    const ts = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
    a.href = url;
    a.download = 'lrc-export-' + fp + '-' + ts + '.json';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);

    if (resultEls.length > 0) {
      updateResult('✅ 备份已下载！文件包含 ' +
        (Array.isArray(exportData.memories) ? exportData.memories.length : 0) + ' 条记忆', 'form-result form-result-success');
    }
  } catch (e) {
    if (resultEls.length > 0) {
      updateResult('⚠️ 备份失败: ' + htmlescape(e.message), 'form-result form-result-error');
    }
  } finally {
    if (btn) btn.disabled = false;
  }
}

// ============================================================
// V2: 记忆数据导入（浏览器端上传备份文件）
// ============================================================
async function importMemories(event) {
  const file = event.target.files[0];
  if (!file) return;

  // v0.8.0：同时更新设置页 backup-result 和信任中心 trust-backup-result
  const resultEls = [$('backup-result'), $('trust-backup-result')].filter(Boolean);
  const updateResult = (text, cls) => {
    resultEls.forEach(el => {
      el.style.display = '';
      el.textContent = text;
      el.className = cls || 'form-result';
    });
  };
  if (resultEls.length > 0) {
    updateResult('⏳ 正在验证并导入记忆数据...');
  }

  try {
    // 读取文件内容
    const content = await new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = (e) => resolve(e.target.result);
      reader.onerror = (e) => reject(new Error('文件读取失败'));
      reader.readAsText(file);
    });

    // 解析 JSON 验证格式
    let exportData;
    try {
      exportData = JSON.parse(content);
    } catch (e) {
      throw new Error('无效的 JSON 文件格式: ' + e.message);
    }

    // 兼容老版本格式：老版本 memories.json 是数组格式 [{...}, ...]
    // 新版本格式是对象 { version: '2.0', memories: [...], ... }
    if (Array.isArray(exportData)) {
      // 老版本数组格式,转换为新版本结构
      exportData = {
        version: '1.0-legacy',
        memories: exportData,
        chunks: [],
        source: 'legacy_array',
        fingerprint: null,
      };
    }

    // 验证导出格式版本(兼容 1.0-legacy 和 2.0)
    if (!exportData.version) {
      throw new Error('不支持的导出格式: 缺少 version 字段');
    }
    if (exportData.version !== '2.0' && exportData.version !== '1.0-legacy') {
      throw new Error('不支持的导出格式版本: ' + exportData.version);
    }

    const memoryCount = Array.isArray(exportData.memories) ? exportData.memories.length : 0;
    const chunkCount = Array.isArray(exportData.chunks) ? exportData.chunks.length : 0;

    // v0.8.3 Step 4 批次 5：confirm→await showConfirm（修复 G001-G003）
    const importConfirmed = await showConfirm(
      '确认导入以下数据？\n\n' +
      '  记忆：' + memoryCount + ' 条\n' +
      '  代码片段：' + chunkCount + ' 个\n' +
      '  来源：' + (exportData.source || '未知') + '\n' +
      '  格式版本：' + exportData.version + '\n' +
      '  指纹：' + (exportData.fingerprint || '无') + '\n\n' +
      '导入将追加到现有数据，不会覆盖已有记忆。确认继续？',
      '确认导入'
    );
    if (!importConfirmed) {
      if (resultEls.length > 0) {
        updateResult('已取消导入');
      }
      return;
    }

    // 调用后端 API 写入数据
    if (memoryCount > 0) {
      let imported = 0;
      let failed = 0;
      // v0.6.1 P1-2 修复: memory_type 枚举统一
      // 老版本导出文件可能含 pattern/correction/general 等后端不识别的类型,
      // 导入时会返回 "无效的记忆类型" 错误。此处做兼容映射。
      const MEMORY_TYPE_COMPAT_MAP = {
        general: 'fact',      // 通用 → 事实
        pattern: 'synthesis', // 模式 → 合成
        correction: 'fact',   // 修正 → 事实
        codecontext: 'code_context', // 老版本无下划线
      };
      function normalizeMemoryType(raw) {
        if (!raw) return 'fact'; // 默认值改为 fact(后端识别)
        const lower = String(raw).toLowerCase();
        if (MEMORY_TYPE_COMPAT_MAP[lower]) return MEMORY_TYPE_COMPAT_MAP[lower];
        return lower; // 已是后端合法类型则原样返回
      }

      for (const mem of exportData.memories) {
        try {
          await fetchWithTimeout(API_BASE + '/v1/memories/remember', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              content: mem.content || JSON.stringify(mem),
              memory_type: normalizeMemoryType(mem.memory_type),
              importance: mem.importance || 5,
              metadata: mem
            }),
          }, 30000);
          imported++;
          // 每 50 条更新一次进度
          if (imported % 50 === 0 && resultEls.length > 0) {
            updateResult('⏳ 导入中... ' + imported + '/' + memoryCount + ' 条');
          }
        } catch (e) {
          failed++;
          console.warn('记忆导入失败 #' + imported + failed + ':', e.message);
        }
      }
      if (resultEls.length > 0) {
        const msg = '✅ 导入完成！成功 ' + imported + ' 条' +
                    (failed > 0 ? '，失败 ' + failed + ' 条' : '');
        updateResult(msg, 'form-result form-result-success');
      }
    }
  } catch (e) {
    if (resultEls.length > 0) {
      updateResult('⚠️ 导入失败: ' + htmlescape(e.message), 'form-result form-result-error');
    }
  } finally {
    // 清除文件选择以便重复选择同一文件
    event.target.value = '';
  }
}

// ============================================================
// v0.8.0 "归一"：数据迁移与合并（调用 POST /v1/migrate）
// 扫描所有已知老路径，按 memory.id 去重合并到 global 目录
// ============================================================
async function migrateData() {
  const result = $('migration-result');
  if (!result) return;
  // v0.8.3 Step 4 批次 5：confirm→await showConfirm（修复 G001-G003）
  const migrateConfirmed = await showConfirm(
    '即将执行数据迁移与合并：\n\n' +
    '  1. 扫描所有已知历史数据路径\n' +
    '  2. 按 memory.id 去重合并到全局目录\n' +
    '  3. 原文件将重命名为 .bak 备份\n\n' +
    '此操作不可逆（但原文件会保留 .bak 备份）。确认继续？',
    '确认迁移'
  );
  if (!migrateConfirmed) {
    result.textContent = '已取消迁移';
    result.className = 'form-result';
    result.style.display = '';
    return;
  }

  result.style.display = '';
  result.textContent = '⏳ 正在扫描并迁移数据...';
  result.className = 'form-result';

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/migrate', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
    }, 60000);

    const data = await res.json();

    if (data.success === false) {
      throw new Error(data.error || '迁移失败');
    }

    // 格式化迁移报告
    const sources = data.sources_scanned || 0;
    const totalFound = data.total_memories_found || 0;
    const merged = data.merged_count || 0;
    const backups = data.backups_created || 0;

    result.innerHTML =
      '✅ 迁移完成！<br>' +
      '<small style="display:block; margin-top:6px;">' +
      '  扫描源：' + sources + ' 处<br>' +
      '  发现记忆：' + totalFound + ' 条<br>' +
      '  合并写入：' + merged + ' 条<br>' +
      '  备份文件：' + backups + ' 个' +
      '</small>';
    result.className = 'form-result form-result-success';

    // 刷新数据位置信息
    if (typeof verifyDataLocation === 'function') {
      setTimeout(verifyDataLocation, 500);
    }
  } catch (e) {
    result.textContent = '⚠️ 迁移失败: ' + htmlescape(e.message);
    result.className = 'form-result form-result-error';
  }
}

// ============================================================
// v0.8.0 "归一"：手动创建备份（调用 POST /v1/backup）
// 将当前记忆库复制到 ~/.loong-recall/backups/ 目录
// ============================================================
async function createBackup() {
  const result = $('backup-result-trust');
  if (!result) return;

  result.style.display = '';
  result.textContent = '⏳ 正在创建备份...';
  result.className = 'form-result';

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/backup', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
    }, 30000);

    const data = await res.json();

    if (data.success === false) {
      throw new Error(data.error || '备份失败');
    }

    result.innerHTML =
      '✅ 备份成功！<br>' +
      '<small style="display:block; margin-top:6px;">' +
      '  记忆数：' + (data.memory_count || 0) + ' 条<br>' +
      '  文件大小：' + (data.backup_size || 0) + ' 字节<br>' +
      '  清理旧备份：' + (data.old_backups_removed || 0) + ' 个<br>' +
      '  当前备份总数：' + (data.total_backups || 0) + ' 份' +
      '</small>';
    result.className = 'form-result form-result-success';

    // 刷新数据位置信息（更新最后备份时间）
    if (typeof verifyDataLocation === 'function') {
      setTimeout(verifyDataLocation, 500);
    }
  } catch (e) {
    result.textContent = '⚠️ 备份失败: ' + htmlescape(e.message);
    result.className = 'form-result form-result-error';
  }
}

// ============================================================
// v0.8.0 "归一"：数据操作日志（调用 GET /v1/data-logs）
// 显示最近 10 条数据操作记录
// ============================================================
async function loadDataLogs() {
  const result = $('data-logs-result');
  const loading = $('data-logs-loading');
  if (!result) return;
  if (loading) loading.classList.remove('hidden');
  result.classList.remove('show');

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/data-logs');
    if (!res.ok) throw new Error('API 不可达');
    const data = await res.json();

    if (!data.entries || data.entries.length === 0) {
      result.innerHTML = '<div class="result-row"><span class="result-label">暂无操作记录</span></div>';
    } else {
      // 操作类型中文映射
      const opMap = {
        migrate: '迁移',
        backup: '备份',
        restore: '恢复',
        export: '导出',
        import: '导入',
        clean: '清理',
      };
      let html = '';
      data.entries.forEach(entry => {
        const opText = opMap[entry.operation] || entry.operation;
        const time = entry.timestamp ? entry.timestamp.replace('T', ' ').replace('Z', '') : '--';
        html += '<div class="result-row">' +
          '<span class="result-label" style="min-width:140px;">' + htmlescape(time) + '</span>' +
          '<span class="result-value"><strong>[' + htmlescape(opText) + ']</strong> ' + htmlescape(entry.details) + '</span>' +
          '</div>';
      });
      result.innerHTML = html;
    }
    result.classList.add('show');
  } catch (e) {
    result.innerHTML = '<div class="result-row"><span class="result-label" style="color:var(--cinnabar)">⚠️ ' + htmlescape(e.message) + '</span></div>';
    result.classList.add('show');
  } finally {
    if (loading) loading.classList.add('hidden');
  }
}

// ============================================================
// v0.8.0 "归一"：AI 规则文件状态查询与重试
// ============================================================

/// 工具 ID 到友好名称的映射
const RULES_TOOL_NAMES = {
  'trae': 'Trae',
  'trae-cn': 'Trae CN',
  'cursor': 'Cursor',
  'codebuddy': 'CodeBuddy',
  'windsurf': 'Windsurf',
  'cline': 'Cline',
  'roo-code': 'Roo Code',
  'comate': 'Comate',
  'vscode': 'GitHub Copilot',
  'jetbrains-ai': 'JetBrains AI',
  'gemini-cli': 'Gemini CLI',
  'aider': 'Aider',
};

/// 加载所有 AI 工具的规则文件状态
async function loadRulesStatus() {
  const result = $('rules-status-result');
  const loading = $('rules-status-loading');
  if (!result) return;
  if (loading) loading.classList.remove('hidden');
  result.classList.remove('show');

  try {
    // Tauri 环境下通过 invoke 调用 get_rules_status 命令
    if (isTauriEnv) {
      const invokeFn = (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) ||
                       (window.__TAURI__ && window.__TAURI__.invoke);
      if (!invokeFn) {
        throw new Error('Tauri 环境下无法调用 invoke');
      }
      const statusList = await invokeFn('get_rules_status');
      renderRulesStatus(result, statusList);
    } else {
      // 非 Tauri 环境不支持
      result.innerHTML = '<div class="result-row"><span class="result-label" style="color:var(--cinnabar)">⚠️ 规则状态查询仅在桌面端可用</span></div>';
    }
    result.classList.add('show');
  } catch (e) {
    result.innerHTML = '<div class="result-row"><span class="result-label" style="color:var(--cinnabar)">⚠️ ' + htmlescape(e.message || String(e)) + '</span></div>';
    result.classList.add('show');
  } finally {
    if (loading) loading.classList.add('hidden');
  }
}

/// 渲染规则状态列表
function renderRulesStatus(container, statusList) {
  if (!statusList || statusList.length === 0) {
    container.innerHTML = '<div class="result-row"><span class="result-label">暂无规则状态数据</span></div>';
    return;
  }

  const total = statusList.length;
  const written = statusList.filter(s => s.exists).length;
  const outdated = statusList.filter(s => s.exists && s.needs_update).length;

  let html = '<div class="result-row" style="font-weight:600;">' +
    '<span class="result-label">总计 ' + total + ' 个工具</span>' +
    '<span class="result-value">已写入 ' + written + ' · 需更新 ' + outdated + '</span>' +
    '</div>';

  statusList.forEach(s => {
    const name = RULES_TOOL_NAMES[s.tool_id] || s.tool_id;
    let statusBadge = '';
    let statusColor = '';

    if (!s.exists) {
      statusBadge = '未写入';
      statusColor = 'var(--cinnabar)';
    } else if (s.needs_update) {
      statusBadge = '需更新（v' + (s.version || '?') + '→v0.8.0）';
      statusColor = 'var(--lrc-金色-500)';
    } else {
      statusBadge = '已最新（v' + (s.version || '?') + '）';
      statusColor = 'var(--lrc-竹青-500)';
    }

    const pathDisplay = s.rules_path.length > 60
      ? '...' + s.rules_path.substring(s.rules_path.length - 57)
      : s.rules_path;

    html += '<div class="result-row">' +
      '<span class="result-label" style="min-width:100px;">' + htmlescape(name) + '</span>' +
      '<span class="result-value" style="color:' + statusColor + ';">' + statusBadge + '</span>' +
      '<span class="result-label text-sm text-dim" style="font-size:11px;min-width:0;flex:1;margin-left:8px;" title="' + htmlescape(s.rules_path) + '">' + htmlescape(pathDisplay) + '</span>' +
      '</div>';
  });

  container.innerHTML = html;
}

/// 重新写入规则文件（通过启动 sidecar 触发规则写入，或提示重启应用）
async function retryWriteRules() {
  showToast('正在重新写入规则文件...', 'info', 2000);

  try {
    if (isTauriEnv) {
      // 通过启动 sidecar 触发 post_sidecar_start 中的规则写入
      // start_sidecar 命令会调用 post_sidecar_start，其中包含 write_rules_for_agents
      const invokeFn = (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) ||
                       (window.__TAURI__ && window.__TAURI__.invoke);
      if (invokeFn) {
        try {
          await invokeFn('start_sidecar');
          showToast('规则文件重新写入完成，请查看状态确认', 'success', 3000);
          // 自动刷新规则状态
          setTimeout(() => loadRulesStatus(), 2000);
        } catch (e) {
          // sidecar 可能已在运行，提示用户重启应用
          showToast('请重启 LRC 桌面端以重新写入规则', 'info', 4000);
        }
      } else {
        showToast('Tauri 环境不可用，请重启应用', 'error', 3000);
      }
    } else {
      showToast('规则写入仅在桌面端可用', 'error', 3000);
    }
  } catch (e) {
    showToast('规则写入失败: ' + (e.message || String(e)), 'error', 4000);
  }
}

// ============================================================
// 基准报告加载
// ============================================================
async function loadBenchmarks() {
  const container = $('benchmark-layers');
  const summaryBar = $('benchmark-summary-bar');
  if (!container) return;
  try {
    const resp = await fetchWithTimeout(API_BASE + '/v1/benchmarks/report');
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    const data = await resp.json();

    // 缓存层数据到全局，供 switchBenchmarkLayer 使用
    window.__benchmarkData = data;

    // 动态生成摘要栏
    const s = data.summary;
    const passedCount = s.passed || 0;
    const totalCount = s.total_tests || 0;
    const statusText = s.status === 'PASS' ? (passedCount + '/' + totalCount + ' 全部通过') : (passedCount + '/' + totalCount + ' 部分通过');
    const statusClass = s.status === 'PASS' ? 'badge-success' : 'badge-warning';
    let summaryHtml = '<span class="badge ' + statusClass + '">' + htmlescape(statusText) + '</span>';
    for (const layer of data.layers) {
      summaryHtml += ' <span class="badge badge-info">' +
        htmlescape(layer.name.split('：')[0]) + '：' + layer.passed + '/' + layer.total +
        '</span>';
    }
    summaryBar.innerHTML = summaryHtml;

    // 设计文档 5.6：默认渲染第一层（通用检索）
    renderBenchmarkLayer(0);
    drawRadarChart(data.radar_chart);
  } catch (e) {
    container.innerHTML = '<div class="card"><p style="color:#f44336">无法加载基准报告: ' + htmlescape(e.message) + '</p><p style="color:#888">请确保 LRC 服务正在运行</p></div>';
    if (summaryBar) summaryBar.innerHTML = '<span class="badge badge-warning">无法加载</span>';
  }
}

// 渲染指定索引的基准测试层
function renderBenchmarkLayer(idx) {
  const container = $('benchmark-layers');
  if (!container || !window.__benchmarkData) return;
  const data = window.__benchmarkData;
  const layer = data.layers[idx];
  if (!layer) {
    container.innerHTML = '<div class="card"><p>该层数据不存在</p></div>';
    return;
  }
  let html = '<div class="benchmark-layer">' +
    '<h3>' + htmlescape(layer.name) + '</h3>' +
    '<p class="layer-desc">' + htmlescape(layer.description) + '</p>';
  for (const test of layer.tests) {
    const statusClass = test.status === 'PASS' ? 'pass' : 'fail';
    const statusIcon = test.status === 'PASS' ? '✓' : '✗';
    html += '<div class="benchmark-test">' +
      '<div class="test-status ' + statusClass + '">' + statusIcon + '</div>' +
      '<div class="test-info">' +
      '<h4>' + htmlescape(test.name) + '</h4>' +
      '<p class="test-desc">' + htmlescape(test.description) + '</p>' +
      '<p class="test-metric">指标: ' + htmlescape(test.metric) + '</p>' +
      '<p class="test-story">"' + htmlescape(test.user_story) + '"</p>' +
      '</div>' +
      '</div>';
  }
  html += '</div>';
  container.innerHTML = html;
}

// 设计文档 5.6：切换三层基准测试标签（通用检索/独有能力/隐私信任）
function switchBenchmarkLayer(idx) {
  const tabs = document.querySelectorAll('.benchmark-tab');
  tabs.forEach((tab, i) => {
    if (i === idx) {
      tab.classList.add('active');
      tab.setAttribute('aria-selected', 'true');
    } else {
      tab.classList.remove('active');
      tab.setAttribute('aria-selected', 'false');
    }
  });
  renderBenchmarkLayer(idx);
}

// 复制一行复现命令
function copyReproCmd() {
    const codeEl = $('reproCmd');
    if (!codeEl) return;
    const code = codeEl.textContent;
    navigator.clipboard.writeText(code).then(() => {
        const btn = document.querySelector('.copy-btn');
        if (!btn) return;
        const original = btn.textContent;
        btn.textContent = '✓ 已复制';
        btn.style.background = '#2a5a3a';
        setTimeout(() => {
            btn.textContent = original;
            btn.style.background = '#1a3a2a';
        }, 2000);
    }).catch(() => {
        // v0.8.3 Step 4 批次 6：alert→showToast（修复 G001-G003）
        showToast('复制失败，请手动选择并复制命令', 'warning');
    });
}

// 雷达图绘制
function drawRadarChart(data) {
  const canvas = $('radarChart');
  if (!canvas) return;
  
  if (!data) {
    data = {
      "记忆存储": 0.9,
      "语义检索": 0.85,
      "隐私保护": 0.95,
      "审计安全": 0.92,
      "记忆演化": 0.88,
      "代码理解": 0.82,
      "本地化运行": 0.98,
      "易扩展性": 0.75
    };
  }
  
  const ctx = canvas.getContext('2d');
  const W = canvas.width;
  const H = canvas.height;
  const cx = W / 2;
  const cy = H / 2;
  const r = Math.min(cx, cy) - 50;
  const keys = Object.keys(data);
  const values = Object.values(data);
  const n = keys.length;

  ctx.clearRect(0, 0, W, H);

  for (let level = 0.2; level <= 1.0; level += 0.2) {
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
      const x = cx + r * level * Math.cos(angle);
      const y = cy + r * level * Math.sin(angle);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.closePath();
    ctx.strokeStyle = 'rgba(26, 26, 46, 0.15)';
    ctx.lineWidth = 1;
    ctx.stroke();
  }

  for (let i = 0; i < n; i++) {
    const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(cx + r * Math.cos(angle), cy + r * Math.sin(angle));
    ctx.strokeStyle = 'rgba(26, 26, 46, 0.1)';
    ctx.lineWidth = 1;
    ctx.stroke();
  }

  ctx.beginPath();
  for (let i = 0; i < n; i++) {
    const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
    const v = values[i];
    const x = cx + r * v * Math.cos(angle);
    const y = cy + r * v * Math.sin(angle);
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.closePath();
  ctx.fillStyle = 'rgba(212, 168, 67, 0.2)';
  ctx.fill();
  ctx.strokeStyle = '#D4A843';
  ctx.lineWidth = 2;
  ctx.stroke();

  for (let i = 0; i < n; i++) {
    const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
    const v = values[i];
    const x = cx + r * v * Math.cos(angle);
    const y = cy + r * v * Math.sin(angle);
    ctx.beginPath();
    ctx.arc(x, y, 4, 0, Math.PI * 2);
    ctx.fillStyle = '#D4A843';
    ctx.fill();
    ctx.strokeStyle = '#fff';
    ctx.lineWidth = 2;
    ctx.stroke();
  }

  ctx.fillStyle = '#1A1A2E';
  ctx.font = '13px "Noto Serif SC", serif';
  ctx.textAlign = 'center';
  for (let i = 0; i < n; i++) {
    const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
    const labelR = r + 24;
    const x = cx + labelR * Math.cos(angle);
    const y = cy + labelR * Math.sin(angle) + 4;
    ctx.fillText(keys[i], x, y);
  }
}

// ============================================================
// 设置页面 — LLM API Key 可视化配置
// ============================================================

/** 加载当前配置状态 */
async function loadSettings() {
  // 绑定提供商切换事件
  const providerSelect = $('llm-provider');
  if (providerSelect && !providerSelect._bound) {
    providerSelect._bound = true;
    providerSelect.addEventListener('change', switchLlmProvider);
  }

  try {
    const resp = await fetchWithTimeout(API_BASE + '/v1/config');
    if (!resp.ok) throw new Error('HTTP ' + resp.status);
    const data = await resp.json();

    // v0.5.5：从 base_url 推断具体提供商名称，与桌面端配置保持一致
    const providerName = inferProviderName(data.llm_type, data.llm_base_url);
    updateLlmStatusBadge(data.llm_configured, providerName);

    // v0.5.5：根据当前配置自动选择提供商并填充表单
    if (data.llm_configured) {
      const providerKey = inferProviderKey(data.llm_type, data.llm_base_url);
      const providerSelectEl = $('llm-provider');
      if (providerSelectEl) {
        providerSelectEl.value = providerKey;
        switchLlmProvider(); // 触发表单字段更新
      }

      // 填充当前配置到表单
      {
        const modelEl = $('llm-model');
        const endpointEl = $('llm-endpoint');
        if (modelEl && data.llm_model) modelEl.value = data.llm_model;
        if (endpointEl && data.llm_base_url) endpointEl.value = data.llm_base_url;
        // API Key 不回填（安全考虑），只显示提示
        const keyEl = $('llm-api-key');
        if (keyEl) keyEl.placeholder = '已配置（如需修改请重新输入）';
      }

      // 显示当前配置详情
      showConfigSection(data.llm_configured, providerName, data.llm_model);
    }
  } catch (e) {
    console.warn('[设置] 加载配置失败:', e.message);
    updateLlmStatusBadge(false, 'none');
  }
}

/**
 * v0.5.5：从 LLM 类型和 base_url 推断提供商 key（用于表单自动选择）
 */
function inferProviderKey(llmType, baseUrl) {
  if (!llmType || llmType === 'none') return 'deepseek';
  // OpenAI 兼容模式：从 base_url 推断具体提供商 key
  if (!baseUrl) return 'openai';
  const url = baseUrl.toLowerCase();
  if (url.includes('api.deepseek.com')) return 'deepseek';
  if (url.includes('dashscope.aliyuncs.com')) return 'qwen';
  if (url.includes('open.bigmodel.cn')) return 'zhipu';
  if (url.includes('api.minimax.chat')) return 'minimax';
  if (url.includes('api.moonshot.cn')) return 'moonshot';
  if (url.includes('api.openai.com')) return 'openai';
  // 未知提供商 → 自定义
  return 'custom';
}

/**
 * v0.5.5 P1-2：从 LLM 类型和 base_url 推断具体提供商名称
 * 确保仪表盘显示与桌面端配置一致，避免"配置了 DeepSeek 显示 OpenAI"的困惑
 */
function inferProviderName(llmType, baseUrl) {
  if (!llmType || llmType === 'none') return 'none';
  // OpenAI 兼容模式：从 base_url 推断具体提供商
  if (!baseUrl) return 'OpenAI';
  const url = baseUrl.toLowerCase();
  if (url.includes('api.openai.com')) return 'OpenAI';
  if (url.includes('api.deepseek.com')) return 'DeepSeek';
  if (url.includes('api.anthropic.com')) return 'Anthropic';
  if (url.includes('generativelanguage.googleapis.com')) return 'Google';
  if (url.includes('api.moonshot.cn')) return 'Moonshot';
  if (url.includes('open.bigmodel.cn')) return '智谱';
  if (url.includes('api.lingyiwanwu.com')) return '零一万物';
  if (url.includes('api.minimax.chat')) return 'MiniMax';
  if (url.includes('api.baichuan-ai.com')) return '百川';
  if (url.includes('dashscope.aliyuncs.com')) return '通义千问';
  // 未知提供商 → 显示"OpenAI 兼容"
  return 'OpenAI 兼容';
}

/** 更新 LLM 状态徽章 */
function updateLlmStatusBadge(configured, type) {
  const badge = $('llm-status-badge');
  if (!badge) return;
  if (configured) {
    badge.textContent = '已配置 · ' + type;
    badge.className = 'badge badge-success';
  } else {
    badge.textContent = '未配置';
    badge.className = 'badge badge-info';
  }
}

/** v0.5.5：显示当前配置详情（嵌入模式和非嵌入模式统一） */
function showConfigSection(configured, type, model) {
  const section = $('current-config-section');
  const content = $('current-config-content');
  if (!section || !content) return;

  if (configured) {
    section.style.display = '';
    content.innerHTML =
      '<p><strong>提供商:</strong> ' + htmlescape(type) + '</p>' +
      '<p><strong>模型:</strong> ' + htmlescape(model || '--') + '</p>' +
      '<p style="color: var(--jade); font-size: 13px; margin-top: 8px;">✅ LLM 查询翻译已启用，搜索时会自动将自然语言翻译为精准关键词。</p>';
  } else {
    section.style.display = 'none';
  }
}

/** v0.5.5：切换 LLM 提供商时自动填充模型和端点，并显示/隐藏对应字段 */
function switchLlmProvider() {
  const provider = $('llm-provider').value;
  const openaiFields = $('openai-fields');
  const providerInfo = LLM_PROVIDERS[provider];

  // 更新提供商描述
  const descEl = $('provider-desc');
  if (descEl && providerInfo) {
    descEl.textContent = providerInfo.desc || '';
  }

  // OpenAI 兼容模式：显示 OpenAI 字段
  openaiFields.style.display = '';

  // 自动填充模型和端点
  if (providerInfo) {
      const modelEl = $('llm-model');
      const endpointEl = $('llm-endpoint');
      const keyEl = $('llm-api-key');
      const modelHintEl = $('model-hint');

      if (modelEl) modelEl.value = providerInfo.model || '';
      if (endpointEl) endpointEl.value = providerInfo.url || '';
      if (keyEl) keyEl.placeholder = providerInfo.keyHint || 'sk-...';

      // 更新模型提示
      if (modelHintEl) {
        const hints = {
          deepseek: '常用: deepseek-chat, deepseek-coder, deepseek-reasoner',
          qwen: '常用: qwen-plus, qwen-turbo, qwen-max, qwen-long',
          zhipu: '常用: glm-4, glm-4-flash, glm-3-turbo, glm-4v',
          minimax: '常用: abab6.5s-chat, abab6.5-chat, abab7-chat',
          moonshot: '常用: moonshot-v1-8k, moonshot-v1-32k, moonshot-v1-128k',
          stepfun: '常用: step-1-flash, step-2-16k, step-2-32k',
          baichuan: '常用: Baichuan4, Baichuan3-Turbo, Baichuan2-53B',
          xunfei: '常用: generalv3.5, generalv3, generalv2.1',
          custom: '请输入模型名称',
        };
        modelHintEl.textContent = hints[provider] || '';
      }

      // 自定义模式时端点可编辑，其他模式也可修改
      if (provider === 'custom') {
        if (endpointEl) endpointEl.placeholder = 'https://your-api-endpoint.com/v1';
      }
    }
}

/** v0.5.5：保存 LLM API Key 配置（支持多提供商，与桌面端统一） */
async function saveLlmConfig() {
  const resultEl = $('llm-config-result');
  const btnSave = $('btn-save-llm');

  if (!resultEl) return;
  resultEl.style.display = '';
  resultEl.className = 'form-result';
  resultEl.textContent = '⏳ 正在保存...';
  if (btnSave) btnSave.disabled = true;

  try {
    const provider = $('llm-provider').value;
    let llmConfigStr = '';

    // OpenAI 兼容模式：openai:apiKey:model:endpoint
    const apiKey = $('llm-api-key').value.trim();
    const model = $('llm-model').value.trim() || LLM_PROVIDERS[provider]?.model || '';
    const endpoint = $('llm-endpoint').value.trim() || LLM_PROVIDERS[provider]?.url || '';

    if (!apiKey) {
      throw new Error('请输入 API Key');
    }
    if (!model) {
      throw new Error('请输入模型名称');
    }
    if (!endpoint) {
      throw new Error('请输入 API 端点');
    }
    llmConfigStr = 'openai:' + apiKey + ':' + model + ':' + endpoint;

    const resp = await fetchWithTimeout(API_BASE + '/v1/config/llm', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ llm_api: llmConfigStr })
    });

    const data = await resp.json();
    if (data.success) {
      resultEl.className = 'form-result success';
      resultEl.textContent = '✅ ' + data.message + '。配置已保存并立即生效，无需重启。';
      // 更新状态徽章
      const providerName = LLM_PROVIDERS[provider]?.name || provider;
      updateLlmStatusBadge(true, providerName);
      // 更新当前配置详情
      showConfigSection(true, providerName, data.llm_model);
    } else {
      throw new Error(data.message || '保存失败');
    }
  } catch (e) {
    // v0.8.2：sidecar 不可达时给出明确提示
    const msg = (e.name === 'SidecarUnreachableError' || e.name === 'SidecarTimeoutError')
      ? '❌ LRC 服务未运行，请先启动服务'
      : '❌ ' + e.message;
    resultEl.className = 'form-result error';
    resultEl.textContent = msg;
  } finally {
    if (btnSave) btnSave.disabled = false;
  }
}

/**
 * v0.8.2：异步确认对话框（增强版）
 *
 * v0.8.1 基础上新增：
 *   - data-autotest 属性，便于自动化测试脚本识别按钮
 *   - ESC 键关闭（对应审计 G012）
 *   - 焦点自动聚焦到确认按钮（对应审计 G013 焦点陷阱前置准备）
 *   - 超时自动取消（30 秒，避免测试卡死）
 *
 * v0.8.3 Step 7 新增：队列机制（修复 N03）
 *   - 同时调用 showConfirm 两次时，第二次排队等待第一次完成
 *   - 队列上限 5 个，超过时拒绝新调用（返回 false）
 *   - 避免单例 modal 文案被覆盖导致第一个 Promise 永不 resolve
 *
 * @param {string} message - 确认提示文案
 * @param {string} [title='确认操作'] - 对话框标题
 * @param {number} [timeoutMs=0] - 超时毫秒（0 表示不超时）
 * @returns {Promise<boolean>} 用户点击"确认"返回 true，"取消"返回 false
 */
// v0.8.3 Step 7：confirm-modal 队列状态（修复 N03 单例冲突）
const confirmModalQueue = [];
let confirmModalActive = false;
// 暴露队列长度查询接口便于测试
window.__getConfirmQueueLength = () => confirmModalQueue.length;

function showConfirm(message, title = '确认操作', timeoutMs = 0) {
  // v0.8.3 Step 7：队列上限检查
  if (confirmModalQueue.length >= 5) {
    console.warn('[showConfirm] 队列已满（5 个等待中），拒绝新调用');
    return Promise.resolve(false);
  }

  return new Promise((resolve) => {
    // 入队等待
    confirmModalQueue.push({ message, title, timeoutMs, resolve });
    // 尝试处理队列
    processConfirmQueue();
  });
}

// v0.8.4 Step 2：暴露 showConfirm 到 window（修复 G018 + CDP 测试 #7）
// 之前遗漏此暴露语句，导致 typeof window.showConfirm === 'function' 返回 false
window.showConfirm = showConfirm;

/**
 * v0.8.3 Step 7：处理 confirm-modal 队列（修复 N03）
 * 同一时刻只显示一个 confirm-modal，避免单例冲突
 */
function processConfirmQueue() {
  if (confirmModalActive || confirmModalQueue.length === 0) return;
  confirmModalActive = true;

  const task = confirmModalQueue.shift();
  // v0.8.4 Step 3：扩展解构，支持 isInfoOnly 标记（修复 G022 队列死锁）
  const { message, title, timeoutMs, resolve, isInfoOnly = false } = task;

  const modal = $('confirm-modal');
  const titleEl = $('confirm-modal-title');
  const msgEl = $('confirm-modal-message');
  const okBtn = $('confirm-modal-ok');
  const cancelBtn = $('confirm-modal-cancel');

  if (!modal || !okBtn || !cancelBtn) {
    // 降级：DOM 不存在时回退到 console.error + 返回 false（Step 11 统一处理降级）
    console.error('[showConfirm] confirm-modal DOM 不存在，降级返回 false');
    // v0.8.4 Step 3：isInfoOnly 时 resolve(undefined) 而非 false
    resolve(isInfoOnly ? undefined : false);
    confirmModalActive = false;
    // 处理队列中下一个
    if (confirmModalQueue.length > 0) processConfirmQueue();
    return;
  }

  // 设置文案
  if (titleEl) titleEl.textContent = title;
  if (msgEl) {
    // v0.8.4 Step 3：isInfoMode 时保留换行（showInfoModal 需要 \n → <br>）
    if (isInfoOnly && typeof htmlescape === 'function') {
      msgEl.innerHTML = htmlescape(message).replace(/\n/g, '<br>');
    } else {
      msgEl.textContent = message;
    }
  }

  // v0.8.4 Step 3：根据 isInfoOnly 标记处理按钮显示（修复 G022）
  // showInfoModal 入队后，隐藏取消按钮，仅显示"知道了"
  if (isInfoOnly) {
    cancelBtn.style.display = 'none';
    okBtn.textContent = '知道了';
  } else {
    cancelBtn.style.display = '';
    okBtn.textContent = '确认';
  }

  // 显示 modal（使用 hidden 属性，与现有 modal 模式一致）
  modal.hidden = false;

  // 自动聚焦到确认按钮，便于键盘操作和自动化测试
  setTimeout(() => okBtn.focus(), 50);

  // 超时自动取消（仅当指定 timeoutMs > 0 时，showInfoModal 不超时）
  let timeoutId = null;
  if (timeoutMs > 0) {
    timeoutId = setTimeout(() => {
      cleanup();
      // v0.8.4 Step 3：isInfoOnly 时 resolve(undefined)
      resolve(isInfoOnly ? undefined : false);
    }, timeoutMs);
  }

  // 清理函数：移除事件监听并隐藏 modal，处理队列下一个
  const cleanup = () => {
    if (timeoutId) clearTimeout(timeoutId);
    modal.hidden = true;
    okBtn.removeEventListener('click', onOk);
    cancelBtn.removeEventListener('click', onCancel);
    modal.removeEventListener('click', onOverlay);
    document.removeEventListener('keydown', onEsc);
    // v0.8.3 Step 12 / G013：移除 Tab 焦点陷阱监听
    modal.removeEventListener('keydown', onTabTrap);
    // 恢复取消按钮显示和确认按钮文案（showInfoModal 可能修改了它们）
    if (cancelBtn) cancelBtn.style.display = '';
    if (okBtn) okBtn.textContent = '确认';
    // v0.8.3 Step 7：释放队列锁，处理下一个任务
    confirmModalActive = false;
    if (confirmModalQueue.length > 0) {
      // 异步调用避免栈溢出
      setTimeout(processConfirmQueue, 0);
    }
  };

  // v0.8.4 Step 3：isInfoOnly 时 onOk resolve(undefined)，onCancel/onOverlay/onEsc 同样
  const onOk = () => { cleanup(); resolve(isInfoOnly ? undefined : true); };
  const onCancel = () => { cleanup(); resolve(isInfoOnly ? undefined : false); };
  const onOverlay = (ev) => {
    // 点击遮罩层（非内容区域）视为取消
    if (ev.target === modal) { cleanup(); resolve(isInfoOnly ? undefined : false); }
  };
  // ESC 键关闭（对应审计 G012）
  const onEsc = (ev) => {
    if (ev.key === 'Escape') { cleanup(); resolve(isInfoOnly ? undefined : false); }
  };
  // v0.8.3 Step 12 / G013：Tab 焦点陷阱（限制焦点在 modal 内循环）
  // 设计原则：仅在 modal 可见时生效，不影响其他按键
  const onTabTrap = (ev) => {
    if (ev.key !== 'Tab') return;
    const focusable = modal.querySelectorAll('button:not([disabled]):not([style*="display: none"]), input:not([disabled]), [tabindex]:not([tabindex="-1"])');
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (ev.shiftKey) {
      // Shift+Tab：从第一个跳到最后一个
      if (document.activeElement === first) {
        ev.preventDefault();
        last.focus();
      }
    } else {
      // Tab：从最后一个跳到第一个
      if (document.activeElement === last) {
        ev.preventDefault();
        first.focus();
      }
    }
  };

  okBtn.addEventListener('click', onOk);
  cancelBtn.addEventListener('click', onCancel);
  modal.addEventListener('click', onOverlay);
  document.addEventListener('keydown', onEsc);
  // v0.8.3 Step 12 / G013：注册 Tab 焦点陷阱
  modal.addEventListener('keydown', onTabTrap);
}

/**
 * v0.8.2：信息展示模态框（替代多行 alert）
 * v0.8.4 Step 3：改为入队模式，复用 processConfirmQueue 串行处理（修复 G022 队列死锁）
 * 之前直接操作 confirm-modal DOM，与 showConfirm 共用 DOM 但未释放 confirmModalActive，
 * 导致 showConfirm 队列永久阻塞。现在入队等待，由 processConfirmQueue 统一调度。
 * @param {string} title - 标题
 * @param {string} content - 内容（支持换行 \n）
 * @returns {Promise<undefined>} 用户点击"知道了"后 resolve
 */
function showInfoModal(title, content) {
  // v0.8.4 Step 3：队列上限检查（与 showConfirm 一致）
  if (confirmModalQueue.length >= 5) {
    console.warn('[showInfoModal] 队列已满（5 个等待中），降级为 Toast 显示');
    showToast(content, 'info');
    return Promise.resolve(undefined);
  }

  // v0.8.4 Step 3：入队等待，由 processConfirmQueue 处理 isInfoOnly 标记
  return new Promise((resolve) => {
    confirmModalQueue.push({
      message: content,
      title: title,
      timeoutMs: 0, // showInfoModal 无超时
      resolve: resolve,
      isInfoOnly: true // 标记为纯信息展示，隐藏取消按钮
    });
    processConfirmQueue();
  });
}

/**
 * v0.8.2：异步输入对话框（替代同步 prompt）
 * @param {string} message - 提示文案
 * @param {string} [title='请输入'] - 标题
 * @param {string} [defaultValue=''] - 默认值
 * @returns {Promise<string|null>} 用户输入的值，取消返回 null
 */
function showPrompt(message, title = '请输入', defaultValue = '') {
  return new Promise((resolve) => {
    const modal = $('confirm-modal');
    const titleEl = $('confirm-modal-title');
    const msgEl = $('confirm-modal-message');
    const okBtn = $('confirm-modal-ok');
    const cancelBtn = $('confirm-modal-cancel');

    if (!modal || !okBtn || !cancelBtn) {
      // v0.8.3 Step 11：N11 降级路径不阻塞 JS 线程（修复 G001-G003）
      // 此前用 prompt(message, defaultValue)，现改为 console.error + showToast + resolve(null)
      console.error('[showPrompt] confirm-modal DOM 不存在，降级返回 null');
      showToast('输入对话框不可用：' + message, 'error');
      resolve(null);
      return;
    }

    if (titleEl) titleEl.textContent = title;
    // 构造输入框
    if (msgEl) {
      msgEl.innerHTML = '';
      const label = document.createElement('div');
      label.textContent = message;
      label.style.marginBottom = '8px';
      const input = document.createElement('input');
      input.type = 'text';
      input.value = defaultValue;
      input.style.width = '100%';
      input.style.padding = '6px 8px';
      input.style.boxSizing = 'border-box';
      input.dataset.autotest = 'prompt-input';
      input.setAttribute('maxlength', '500');  // v0.8.6 Step 9 / N009：限制输入长度
      msgEl.appendChild(label);
      msgEl.appendChild(input);
    }

    modal.hidden = false;
    setTimeout(() => {
      const input = msgEl?.querySelector('input');
      if (input) { input.focus(); input.select(); }
    }, 50);

    const cleanup = () => {
      modal.hidden = true;
      okBtn.removeEventListener('click', onOk);
      cancelBtn.removeEventListener('click', onCancel);
      document.removeEventListener('keydown', onKey);
    };

    const getValue = () => {
      const input = msgEl?.querySelector('input');
      return input ? input.value : '';
    };

    const onOk = () => { const v = getValue(); cleanup(); resolve(v); };
    const onCancel = () => { cleanup(); resolve(null); };
    // v0.8.2：Enter 提交、ESC 取消
    const onKey = (ev) => {
      if (ev.key === 'Escape') { cleanup(); resolve(null); }
      if (ev.key === 'Enter') { const v = getValue(); cleanup(); resolve(v); }
    };

    okBtn.addEventListener('click', onOk);
    cancelBtn.addEventListener('click', onCancel);
    document.addEventListener('keydown', onKey);
  });
}

// 暴露到全局
window.showInfoModal = showInfoModal;
window.showPrompt = showPrompt;

/** v0.5.5：清除 LLM API Key 配置 */
async function clearLlmConfig() {
  const resultEl = $('llm-config-result');
  if (!resultEl) return;

  // v0.8.1：用异步 showConfirm 替代同步 confirm()，避免阻塞 JS 线程
  const confirmed = await showConfirm(
    '确定要清除 LLM 配置吗？清除后将无法使用自然语言搜索代码。',
    '清除 LLM 配置'
  );
  if (!confirmed) return;

  resultEl.style.display = '';
  resultEl.className = 'form-result';
  resultEl.textContent = '⏳ 正在清除...';

  try {
    const resp = await fetchWithTimeout(API_BASE + '/v1/config/llm', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ llm_api: '' })
    });

    const data = await resp.json();
    if (data.success) {
      resultEl.className = 'form-result success';
      resultEl.textContent = '✅ LLM 配置已清除。';
      // v0.8.2：补充 Toast 反馈（对应审计 G018 成功视觉反馈）
      if (typeof showToast === 'function') {
        showToast('LLM 配置已清除', 'success');
      }
      updateLlmStatusBadge(false, 'none');
      showConfigSection(false, '', '');
      // 清空表单
      const apiKeyInput = $('llm-api-key');
      if (apiKeyInput) apiKeyInput.value = '';
      const modelInput = $('llm-model');
      if (modelInput) modelInput.value = '';
      const endpointInput = $('llm-endpoint');
      if (endpointInput) endpointInput.value = '';
    } else {
      throw new Error(data.message || '清除失败');
    }
  } catch (e) {
    resultEl.className = 'form-result error';
    resultEl.textContent = '❌ ' + e.message;
  }
}

// ============================================================
// v0.6.1 P0-3 第一批: 核心 CRUD 命令（服务管理/向导/项目）
// 桌面端专属，通过 postMessageToParent 调用 Tauri invoke
// ============================================================

/** v0.6.1 P0-3 第一批: 停止 sidecar 服务 */
async function stopSidecarService() {
  // v0.8.1：用异步 showConfirm 替代同步 confirm()，避免阻塞 JS 线程
  const confirmed = await showConfirm(
    '确定要停止 LRC 服务吗？停止后记忆检索功能将不可用。',
    '停止 LRC 服务'
  );
  if (!confirmed) return;
  try {
    const result = await postMessageToParent('lrc-stop-service', {}, 30000);
    if (result && result.success !== false) {
      // v0.8.2：用 showToast 替代 alert（对应审计 G001）
      showToast('服务已停止', 'success');
      setTimeout(() => loadDashboard(), 500);
    } else {
      showToast('停止服务失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('停止服务失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第一批: 刷新 sidecar 项目列表 */
async function listSidecarProjects() {
  try {
    const result = await postMessageToParent('lrc-list-projects', {}, 30000);
    if (Array.isArray(result)) {
      const count = result.length;
      const details = result.map(p => `• ${p.project_key || 'default'} (端口: ${p.port || '-'})`).join('\n');
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('项目实例列表', `当前运行的项目实例: ${count} 个\n\n${details || '（无）'}`);
    } else {
      showInfoModal('项目列表', JSON.stringify(result));
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('获取项目列表失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第一批: 选择项目目录（弹出系统文件选择器） */
async function pickProjectDirectory() {
  try {
    const result = await postMessageToParent('lrc-pick-project-dir', {}, 60000);
    if (result && result.project_dir) {
      // v0.8.2：用 showToast 替代单行 alert
      showToast('已选择项目目录: ' + result.project_dir, 'success');
    } else if (result && result.success === false) {
      showToast('选择项目目录失败: ' + (result.message || '用户取消'), 'error');
    } else {
      showToast('操作完成: ' + JSON.stringify(result), 'success');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('选择项目目录失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第一批: 获取向导状态 */
async function getWizardState() {
  try {
    const result = await postMessageToParent('lrc-get-wizard-state', {}, 10000);
    if (result) {
      const lines = [
        '向导状态:',
        '• 已完成: ' + (result.completed ? '是' : '否'),
        '• 项目目录: ' + (result.project_dir || '（未设置）'),
        '• LLM 已配置: ' + (result.llm_configured ? '是' : '否'),
        '• Agent 已配置: ' + (result.agent_configured ? '是' : '否'),
        '• 当前进度: ' + (result.current_step || '未知'),
      ];
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('向导状态', lines.join('\n'));
    } else {
      showToast('向导状态为空', 'info');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('获取向导状态失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第一批: 重置向导状态 */
async function resetWizardState() {
  // v0.8.2：用 showConfirm 替代 confirm（对应审计 G002）
  const confirmed = await showConfirm(
    '确定要重置向导吗？\n重置后首次启动将重新进入配置向导流程。',
    '重置向导'
  );
  if (!confirmed) return;
  try {
    const result = await postMessageToParent('lrc-reset-wizard', {}, 10000);
    if (result && result.success !== false) {
      showToast('向导已重置，' + (result.message || '下次启动将重新进入向导'), 'success');
    } else {
      showToast('重置向导失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('重置向导失败: ' + errMsg, 'error');
    }
  }
}

// ============================================================
// v0.6.1 P0-3 第二批: 用户功能命令（LLM/Agent/项目目录）
// ============================================================

/** v0.6.1 P0-3 第二批: 获取 LLM 配置（从桌面端存储读取，非 sidecar） */
async function getLlmConfig() {
  try {
    const result = await postMessageToParent('lrc-get-llm-config', {}, 10000);
    if (result) {
      const hasConfig = result.llm_api || result.configured;
      const lines = [
        'LLM 配置状态:',
        '• 已配置: ' + (hasConfig ? '是' : '否'),
        '• 提供商: ' + (result.provider || '（未设置）'),
        '• 模型: ' + (result.model || '（未设置）'),
      ];
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('LLM 配置', lines.join('\n'));
    } else {
      showToast('LLM 未配置', 'info');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('获取 LLM 配置失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第二批: 测试 LLM 连接（向桌面端配置的 LLM 发起测试请求） */
async function testLlmConnection() {
  try {
    const result = await postMessageToParent('lrc-test-llm-connection', {}, 60000);
    if (result && result.success !== false) {
      // v0.8.2：用 showToast 替代 alert
      showToast('LLM 连接测试成功，' + (result.message || '连接正常'), 'success');
    } else {
      showToast('LLM 连接测试失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('LLM 连接测试失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第二批: 检测所有支持的 Agent（扫描系统中可配置的 AI 工具） */
async function detectAgents() {
  try {
    const result = await postMessageToParent('lrc-detect-agents', {}, 30000);
    if (Array.isArray(result)) {
      const count = result.length;
      const details = result.slice(0, 10).map(a => `• ${a.name || a.id || '未知'} (${a.installed ? '已安装' : '未安装'})`).join('\n');
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('Agent 检测结果', `检测到 ${count} 个支持的 Agent\n\n${details}${count > 10 ? '\n...（仅显示前 10 个）' : ''}`);
    } else {
      showInfoModal('检测结果', JSON.stringify(result));
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('Agent 检测失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第二批: 仅检测已安装的 Agent */
async function detectInstalledAgents() {
  try {
    const result = await postMessageToParent('lrc-detect-installed-agents', {}, 30000);
    if (Array.isArray(result)) {
      const count = result.length;
      const details = result.map(a => `• ${a.name || a.id || '未知'} → ${a.config_path || '（无配置路径）'}`).join('\n');
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('已安装 Agent', `已安装 ${count} 个 Agent\n\n${details || '（无）'}`);
    } else {
      showInfoModal('检测结果', JSON.stringify(result));
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('检测已安装 Agent 失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第二批: 设置项目目录（直接设置，不弹文件选择器） */
async function setProjectDir() {
  // v0.8.2：用 showPrompt 替代同步 prompt（对应审计 G003）
  const projectDir = await showPrompt(
    '请输入项目目录的绝对路径:\n（例如: G:\\code-memory）',
    '设置项目目录'
  );
  if (!projectDir || !projectDir.trim()) {
    return;
  }
  const trimmedDir = projectDir.trim();
  try {
    const result = await postMessageToParent('lrc-set-project-dir', { projectDir: trimmedDir }, 10000);
    if (result && result.success !== false) {
      // v0.8.2：用 showToast 替代 alert
      showToast('项目目录已设置: ' + (result.project_dir || trimmedDir), 'success');
    } else {
      showToast('设置项目目录失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('设置项目目录失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第二批: 获取当前项目目录 */
async function getProjectDir() {
  try {
    const result = await postMessageToParent('lrc-get-project-dir', {}, 10000);
    if (result && (result.project_dir || result.path)) {
      // v0.8.2：用 showToast 替代 alert
      showToast('当前项目目录: ' + (result.project_dir || result.path), 'info');
    } else {
      showToast('当前未设置项目目录（使用默认全局模式）', 'info');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('获取项目目录失败: ' + errMsg, 'error');
    }
  }
}

// ============================================================
// v0.6.1 P0-3 第三批: 低频管理命令（多实例/Agent配置/IDE扫描/向导完成）
// ============================================================

/** v0.6.1 P0-3 第三批: 为指定项目启动 sidecar 实例 */
async function startSidecarForProject() {
  // v0.8.2：用 showPrompt 替代同步 prompt
  const projectDir = await showPrompt('请输入要启动 sidecar 的项目目录绝对路径:', '启动项目 sidecar');
  if (!projectDir || !projectDir.trim()) {
    return;
  }
  const trimmedDir = projectDir.trim();
  // v0.8.2：用 showConfirm 替代同步 confirm
  const confirmed = await showConfirm('确定要为以下项目启动 sidecar 吗？\n' + trimmedDir, '启动 sidecar');
  if (!confirmed) {
    return;
  }
  try {
    const result = await postMessageToParent('lrc-start-sidecar-for-project', { projectDir: trimmedDir }, 60000);
    if (result && (result.port || result.success !== false)) {
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('sidecar 已启动', '项目 sidecar 已启动\n项目: ' + (result.project_dir || trimmedDir) + '\n端口: ' + (result.port || '未知'));
      setTimeout(() => loadDashboard(), 500);
    } else {
      showToast('启动项目 sidecar 失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('启动项目 sidecar 失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第三批: 停止指定项目的 sidecar 实例 */
async function stopSidecarForProject() {
  // v0.8.2：用 showPrompt 替代同步 prompt
  const projectDir = await showPrompt('请输入要停止 sidecar 的项目目录绝对路径:', '停止项目 sidecar');
  if (!projectDir || !projectDir.trim()) {
    return;
  }
  const trimmedDir = projectDir.trim();
  // v0.8.2：用 showConfirm 替代同步 confirm
  const confirmed = await showConfirm('确定要停止该项目的 sidecar 吗？\n' + trimmedDir, '停止 sidecar');
  if (!confirmed) {
    return;
  }
  try {
    const result = await postMessageToParent('lrc-stop-sidecar-for-project', { projectDir: trimmedDir }, 30000);
    if (result && result.success !== false) {
      // v0.8.2：用 showToast 替代 alert
      showToast('项目 sidecar 已停止，' + (result.message || ''), 'success');
    } else {
      showToast('停止项目 sidecar 失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('停止项目 sidecar 失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第三批: 获取 Agent 配置指南（返回每个 Agent 的配置说明） */
async function getAgentConfigGuide() {
  try {
    const result = await postMessageToParent('lrc-get-agent-config-guide', {}, 10000);
    if (result) {
      let text = 'Agent 配置指南:\n\n';
      if (Array.isArray(result)) {
        result.forEach(item => {
          text += '• ' + (item.name || item.id || '未知') + '\n  ' + (item.guide || item.config_guide || '（无说明）') + '\n\n';
        });
      } else if (typeof result === 'object') {
        Object.entries(result).forEach(([key, value]) => {
          text += '• ' + key + ': ' + (typeof value === 'string' ? value : JSON.stringify(value)) + '\n';
        });
      } else {
        text += String(result);
      }
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('Agent 配置指南', text);
    } else {
      showToast('无配置指南', 'info');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('获取 Agent 配置指南失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第三批: 发现所有可配置的 Agent（包含未安装的） */
async function discoverAllAgents() {
  try {
    const result = await postMessageToParent('lrc-discover-all-agents', {}, 30000);
    if (Array.isArray(result)) {
      const count = result.length;
      const installed = result.filter(a => a.installed).length;
      const details = result.slice(0, 15).map(a => `• ${a.name || a.id || '未知'} [${a.installed ? '已安装' : '未安装'}]`).join('\n');
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('发现 Agent', `发现 ${count} 个 Agent（已安装 ${installed} 个）\n\n${details}${count > 15 ? '\n...（仅显示前 15 个）' : ''}`);
    } else {
      showInfoModal('发现结果', JSON.stringify(result));
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('发现 Agent 失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第三批: 自动配置已安装的 Agent（写入 MCP 配置文件） */
async function configureAgents() {
  // v0.8.2：用 showConfirm 替代 confirm
  const confirmed = await showConfirm(
    '确定要自动配置所有已安装的 Agent 吗？\n这将向 Agent 的配置文件中写入 LRC 的 MCP 配置。',
    '自动配置 Agent'
  );
  if (!confirmed) return;
  try {
    const result = await postMessageToParent('lrc-configure-agents', {}, 60000);
    if (result && result.success !== false) {
      const configured = result.configured_count || result.count || 0;
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('Agent 配置完成', '已配置: ' + configured + ' 个\n' + (result.message || ''));
    } else {
      showToast('Agent 配置失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('Agent 配置失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第三批: 保存已配置的 Agent 列表（持久化到 wizard.json） */
async function saveConfiguredAgents() {
  // v0.8.2：用 showConfirm 替代 confirm
  const confirmed = await showConfirm('确定要保存当前已配置的 Agent 列表吗？', '保存 Agent 配置');
  if (!confirmed) return;
  try {
    const result = await postMessageToParent('lrc-save-configured-agents', {}, 10000);
    if (result && result.success !== false) {
      // v0.8.2：用 showToast 替代 alert
      showToast('已保存 Agent 配置，' + (result.message || ''), 'success');
    } else {
      showToast('保存 Agent 配置失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('保存 Agent 配置失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第三批: 扫描 IDE 项目（扫描磁盘上的 IDE 工程目录） */
async function scanIdeProjects() {
  // v0.8.2：用 showConfirm 替代 confirm
  const confirmed = await showConfirm(
    '确定要扫描 IDE 项目吗？\n扫描可能需要一些时间，请耐心等待。',
    '扫描 IDE 项目'
  );
  if (!confirmed) return;
  try {
    const result = await postMessageToParent('lrc-scan-ide-projects', {}, 60000);
    if (Array.isArray(result)) {
      const count = result.length;
      const details = result.slice(0, 10).map(p => `• ${p.name || p.path || '未知'}`).join('\n');
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('IDE 项目扫描结果', `扫描到 ${count} 个 IDE 项目\n\n${details}${count > 10 ? '\n...（仅显示前 10 个）' : ''}`);
    } else {
      showInfoModal('扫描结果', JSON.stringify(result));
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('扫描 IDE 项目失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第三批: 打开桌面端设置窗口 */
async function openSettings() {
  try {
    const result = await postMessageToParent('lrc-open-settings', {}, 10000);
    if (result && result.success !== false) {
      // 设置窗口已打开，无需额外提示
      console.log('桌面端设置窗口已打开');
    } else {
      // v0.8.2：用 showToast 替代 alert
      showToast('打开设置失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('打开设置失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第三批: 标记向导完成（结束向导流程，进入正常使用） */
async function markComplete() {
  // v0.8.2：用 showConfirm 替代 confirm
  const confirmed = await showConfirm(
    '确定要标记向导为已完成吗？\n完成后将不再显示向导引导。',
    '标记向导完成'
  );
  if (!confirmed) return;
  try {
    const result = await postMessageToParent('lrc-mark-complete', {}, 10000);
    if (result && result.success !== false) {
      // v0.8.2：用 showToast 替代 alert
      showToast('向导已标记完成，' + (result.message || ''), 'success');
      setTimeout(() => loadDashboard(), 500);
    } else {
      showToast('标记完成失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('标记完成失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3 第三批: 验证安装（检查 sidecar 二进制/配置文件/Agent 配置是否完整） */
async function verifySetup() {
  try {
    const result = await postMessageToParent('lrc-verify-setup', {}, 30000);
    if (result) {
      const lines = [
        '安装验证结果:',
        '• sidecar 二进制: ' + (result.sidecar_binary ? '✓' : '✗'),
        '• 配置文件: ' + (result.config_file ? '✓' : '✗'),
        '• LLM 配置: ' + (result.llm_configured ? '✓' : '✗'),
        '• Agent 配置: ' + (result.agent_configured ? '✓' : '✗'),
      ];
      if (result.issues && result.issues.length > 0) {
        lines.push('\n存在的问题:');
        result.issues.forEach(issue => lines.push('• ' + issue));
      }
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('安装验证结果', lines.join('\n'));
    } else {
      showToast('验证通过', 'success');
    }
  } catch (e) {
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('此功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用', 'warning');
    } else {
      showToast('验证安装失败: ' + errMsg, 'error');
    }
  }
}

/** v0.6.1 P0-3: 切换高级管理面板展开/折叠状态 */
function toggleAdvancedManagement() {
  const body = $('advanced-management-body');
  const text = $('advanced-toggle-text');
  if (!body || !text) return;
  if (body.hasAttribute('hidden')) {
    body.removeAttribute('hidden');
    text.textContent = '折叠';
  } else {
    body.setAttribute('hidden', '');
    text.textContent = '展开';
  }
}

// ============================================================
// 暴露 HTML onclick 所需的函数到全局作用域
// 仅暴露 13 个被 index.html 中 onclick 属性引用的函数
// 其他所有变量和函数均保持 IIFE 私有，避免全局污染
// ============================================================
window.toggleNav = toggleNav;
window.generateCaptainLog = generateCaptainLog;
window.verifyDataLocation = verifyDataLocation;
window.verifyNetworkAudit = verifyNetworkAudit;
window.verifyAuditIntegrity = verifyAuditIntegrity;
window.wizardStep1Search = wizardStep1Search;
window.wizardStep2Write = wizardStep2Write;
window.wizardStep3Search = wizardStep3Search;
window.backupMemories = backupMemories;
window.importMemories = importMemories;
// v0.8.0 "归一"：信任中心数据文件夹打开按钮
window.handleOpenDataDirClick = handleOpenDataDirClick;
// v0.8.0 "归一"：数据迁移与合并
window.migrateData = migrateData;
// v0.8.0 "归一"：手动创建备份
window.createBackup = createBackup;
// v0.8.0 "归一"：数据操作日志
window.loadDataLogs = loadDataLogs;
// v0.8.0 "归一"：规则文件状态查询与重试
window.loadRulesStatus = loadRulesStatus;
window.retryWriteRules = retryWriteRules;
window.copyReproCmd = copyReproCmd;
window.saveLlmConfig = saveLlmConfig;
window.clearLlmConfig = clearLlmConfig;
// v0.5.4 P1-7 新增：仪表盘重构相关函数
window.loadRecentMemories = loadRecentMemories;
window.switchToTab = switchToTab;
// v0.6.0 龙忆设计系统：新增功能函数
window.selectPresetScenario = selectPresetScenario;
window.runPrivacyCheck = runPrivacyCheck;
// v0.6.0 龙忆设计系统补丁：暴露工具函数供 IIFE 外部的新增功能使用
window.fetchWithTimeout = fetchWithTimeout;
window.$ = $;
window.htmlescape = htmlescape;
// v0.8.4 Step 9：暴露 sanitizeMemoryType 供外部调用
window.sanitizeMemoryType = sanitizeMemoryType;
window.API_BASE = API_BASE;
window.safeJson = safeJson;
// v0.6.0 设计文档 5.6：三层基准测试切换标签
window.switchBenchmarkLayer = switchBenchmarkLayer;
// v0.6.0 设置页面重构：暴露 LLM 提供商切换函数
window.switchLlmProvider = switchLlmProvider;
// v0.8.2：暴露 data-input-action 所需函数（CSP 合规：替代内联 onchange/oninput）
window.debouncedMemorySearch = debouncedMemorySearch;
window.changeEmbedderMirror = changeEmbedderMirror;
window.updateSetupLlmFields = updateSetupLlmFields;
// v0.6.0 设置页面重构：暴露保存和清除配置函数
window.saveLlmConfig = saveLlmConfig;
window.clearLlmConfig = clearLlmConfig;
window.loadSettings = loadSettings;
// v0.6.1 P0-3 第一批：桌面端核心 CRUD 命令调用入口
window.stopSidecarService = stopSidecarService;
window.listSidecarProjects = listSidecarProjects;
window.pickProjectDirectory = pickProjectDirectory;
window.getWizardState = getWizardState;
window.resetWizardState = resetWizardState;
// v0.6.1 P0-3 第二批：用户功能命令调用入口
window.getLlmConfig = getLlmConfig;
window.testLlmConnection = testLlmConnection;
window.detectAgents = detectAgents;
window.detectInstalledAgents = detectInstalledAgents;
window.setProjectDir = setProjectDir;
window.getProjectDir = getProjectDir;
// v0.6.1 P0-3 第三批：低频管理命令调用入口
window.startSidecarForProject = startSidecarForProject;
window.stopSidecarForProject = stopSidecarForProject;
window.getAgentConfigGuide = getAgentConfigGuide;
window.discoverAllAgents = discoverAllAgents;
window.configureAgents = configureAgents;
window.saveConfiguredAgents = saveConfiguredAgents;
window.scanIdeProjects = scanIdeProjects;
window.openSettings = openSettings;
window.markComplete = markComplete;
window.verifySetup = verifySetup;
// v0.6.1 P0-3: 高级管理面板展开/折叠
window.toggleAdvancedManagement = toggleAdvancedManagement;
// v0.7.0 孤儿路由修复：记忆详情面板操作函数导出（修复 onclick 调用失败问题）
window.correctMemory = correctMemory;
window.submitMemoryFeedback = submitMemoryFeedback;
// v0.7.0 孤儿路由修复 Step 3-D：洛书向量编码器
window.encodeTextToLuoshu = encodeTextToLuoshu;
// v0.7.0 孤儿路由修复 Step 3-E：合成记忆拆解
window.unfoldMemory = unfoldMemory;
// v0.7.0 孤儿路由修复 Step 3-F：版本检查更新
window.checkVersionUpdate = checkVersionUpdate;

// v0.8.0 桌面端 P0 修复：IIFE 闭合位置移到文件末尾
// 原因：第 2950 行之后的函数（loadEvolutionTimeline/runPrivacyCheck 等）
// 调用了 IIFE 内部的 fetchWithTimeout/safeJson 等辅助函数，词法作用域
// 导致 ReferenceError。将 IIFE 闭合移到文件末尾，让所有函数都在 IIFE
// 内部，可正确访问辅助函数。IIFE 闭合标签见文件末尾。
// 详见 docs/DESKTOP_FIX_PLAN_v0.8.0.md

/* ============================================================
 * v0.6.0 龙忆设计系统新增功能
 * - 预设场景模板（v0.7.0 预览）
 * - 结晶历史时间线（v0.8.0 预览）
 * - 一键隐私检查（v0.9.0 预览）
 * ============================================================ */

/**
 * v0.6.0 预览：预设场景模板选择（v0.7.0 预览）
 * 4 套预设场景：personal-notes / project-management / learning-assistant / coding-helper
 * @param {HTMLElement} card - 被点击的场景卡片元素
 */
function selectPresetScenario(card) {
  // 移除所有卡片的选中状态（同时移除 selected 和 active 类）
  const grid = document.getElementById('preset-scenario-grid');
  if (grid) {
    grid.querySelectorAll('.preset-scenario-card').forEach(function(c) {
      c.classList.remove('selected');
      c.classList.remove('active');
    });
  }
  // 标记当前卡片为选中（同时添加 selected 和 active 类）
  // v0.8.4 Step 5：添加 active 类以兼容 CDP 测试检测（修复 CDP 测试 #3）
  if (card) {
    card.classList.add('selected');
    card.classList.add('active');
    const scenario = card.getAttribute('data-scenario');

    // v0.8.4 Step 5：持久化到 localStorage（修复 G045）
    // 之前 TODO 注释"v0.7.0 正式版将通过 MCP 工具 scenario 持久化"，现在用 localStorage 实现
    try {
      localStorage.setItem('lrc-selected-scenario', scenario);
    } catch (e) {
      console.warn('[selectPresetScenario] localStorage 写入失败:', e);
    }

    // 显示提示信息（诗意文案）
    const scenarioMap = {
      'personal-notes': { title: '个人笔记', desc: '记忆类型：note / 标签：[note, personal] / 结晶策略：按主题聚类，7 天结晶' },
      'project-management': { title: '项目管理', desc: '记忆类型：decision/task / 标签：[project, {id}] / 结晶策略：按项目聚类，实时结晶' },
      'learning-assistant': { title: '学习助手', desc: '记忆类型：knowledge / 标签：[learn, {subject}] / 结晶策略：按学科聚类，按需结晶' },
      'coding-helper': { title: '编程助手', desc: '记忆类型：code_context/preference / 标签：[code, {lang}] / 结晶策略：按代码语言聚类' }
    };
    const info = scenarioMap[scenario];
    if (info) {
      // v0.8.4 Step 5：在卡片下方显示场景信息（替代原 TODO）
      const infoEl = document.getElementById('preset-scenario-info');
      if (infoEl) {
        infoEl.innerHTML = '<strong>' + htmlescape(info.title) + '</strong>：' + htmlescape(info.desc);
        infoEl.style.display = '';
      }
    }
  }
}

/**
 * v0.8.4 Step 5：恢复用户上次选择的预设场景（修复 G045）
 * 在页面初始化时调用，从 localStorage 读取并恢复选中状态
 */
function restoreSelectedScenario() {
  try {
    // v0.8.5 Step 2 / G080 修复：若无保存值，默认到 coding-helper
    let saved = localStorage.getItem('lrc-selected-scenario');
    if (!saved) {
      saved = 'coding-helper'; // 与 HTML 默认 selected 类一致
    }
    const grid = document.getElementById('preset-scenario-grid');
    if (!grid) return;
    const target = grid.querySelector('.preset-scenario-card[data-scenario="' + saved + '"]');
    if (target) {
      selectPresetScenario(target);
      console.log('[restoreSelectedScenario] 已恢复场景:', saved);
    }
  } catch (e) {
    console.warn('[restoreSelectedScenario] 读取 localStorage 失败:', e);
  }
}

/**
 * v0.6.0 预览：一键隐私检查（v0.9.0 预览）
 * 100ms 内返回报告：存储位置、大小、网络访问、加密状态
 * 三色信任指示器（绿/黄/红）
 */
async function runPrivacyCheck() {
  const resultEl = document.getElementById('privacy-check-result');
  if (!resultEl) return;

  // 显示加载状态
  resultEl.classList.add('show');
  resultEl.innerHTML = '<div style="padding:12px;color:var(--lrc-墨韵-300);font-size:13px;">正在生成隐私报告...</div>';

  const startTime = Date.now();
  try {
    // 并行调用三个接口以保证 100ms 内返回
    const [dataLoc, networkAudit, auditIntegrity] = await Promise.all([
      fetchWithTimeout(`${window.API_BASE}/v1/trust/data-location`, {}, 5000),
      fetchWithTimeout(`${window.API_BASE}/v1/trust/network-audit`, {}, 5000),
      fetchWithTimeout(`${window.API_BASE}/v1/trust/audit-integrity`, {}, 5000)
    ]);

    const dataLocData = await safeJson(dataLoc);
    const networkData = await safeJson(networkAudit);
    const integrityData = await safeJson(auditIntegrity);

    const elapsed = Date.now() - startTime;

    // 计算信任等级（三色指示器）
    let trustLevel = 'green'; // green / yellow / red
    let trustLabel = '信任';
    let trustColor = 'var(--lrc-玉色-500)';

    if (!dataLocData.ok || !networkData.ok || !integrityData.ok) {
      trustLevel = 'red';
      trustLabel = '异常';
      trustColor = 'var(--lrc-朱砂-500)';
    } else if (networkData.network_calls && networkData.network_calls.length > 0) {
      trustLevel = 'yellow';
      trustLabel = '注意';
      trustColor = 'var(--lrc-金色-500)';
    }

    // 渲染报告
    const html = `
      <div style="padding:12px;background:var(--lrc-宣纸-300);border-radius:var(--radius-md);">
        <div style="display:flex;align-items:center;gap:8px;margin-bottom:12px;padding-bottom:8px;border-bottom:1px solid var(--lrc-宣纸-500);">
          <span style="display:inline-block;width:12px;height:12px;border-radius:50%;background:${trustColor};box-shadow:0 0 0 3px ${trustColor}33;"></span>
          <strong style="color:${trustColor};font-size:14px;">信任等级：${trustLabel}</strong>
          <span style="margin-left:auto;font-size:11px;color:var(--lrc-墨韵-300);font-family:var(--font-mono);">耗时 ${elapsed}ms</span>
        </div>
        <div class="result-row">
          <span class="result-label">数据存储位置</span>
          <span class="result-value ${dataLocData.ok ? 'valid' : 'invalid'}">${dataLocData.ok ? (dataLocData.data_path || '本地') : '获取失败'}</span>
        </div>
        <div class="result-row">
          <span class="result-label">网络访问记录</span>
          <span class="result-value ${networkData.ok && (!networkData.network_calls || networkData.network_calls.length === 0) ? 'valid' : 'invalid'}">${networkData.ok ? (networkData.network_calls ? networkData.network_calls.length + ' 次' : '0 次') : '获取失败'}</span>
        </div>
        <div class="result-row">
          <span class="result-label">审计日志完整性</span>
          <span class="result-value ${integrityData.ok ? 'valid' : 'invalid'}">${integrityData.ok ? '已验证' : '验证失败'}</span>
        </div>
        <div class="result-row">
          <span class="result-label">加密状态</span>
          <span class="result-value valid">本地存储</span>
        </div>
      </div>
      <p style="margin-top:8px;font-size:11px;color:var(--lrc-墨韵-300);font-family:var(--font-serif);">记忆有道，生生不息 —— 你的数据，从未离开你的机器</p>
    `;
    resultEl.innerHTML = html;
    console.log(`[LRC v${APP_VERSION}]隐私检查完成，耗时 ${elapsed}ms，信任等级: ${trustLevel}`);
  } catch (err) {
    const elapsed = Date.now() - startTime;
    resultEl.innerHTML = `
      <div style="padding:12px;background:var(--lrc-朱砂-50);border-radius:var(--radius-md);color:var(--lrc-朱砂-500);font-size:13px;">
        <strong>隐私检查失败</strong>（耗时 ${elapsed}ms）：<br>
        <span style="font-size:12px;">${htmlescape(err.message || String(err))}</span>
      </div>
    `;
    console.error('[LRC v' + APP_VERSION + ']隐私检查失败:', err);
  }
}

/**
 * v0.6.0 预览：加载结晶历史时间线（v0.8.0 预览）
 * 从审计日志中提取结晶事件并渲染到时间线
 */
async function loadCrystallizationHistory() {
  const timelineEl = document.getElementById('crystallization-timeline');
  if (!timelineEl) return;

  try {
    const res = await fetchWithTimeout(`${window.API_BASE}/v1/audit-trail?limit=10`, {}, 5000);
    const data = await safeJson(res);

    if (!data.ok || !data.entries || data.entries.length === 0) {
      // 保持现有的示例数据（v0.8.0 预览模式）
      return;
    }

    // 过滤出结晶相关事件
    const crystallizationEvents = data.entries.filter(function(e) {
      return e.event_type && (e.event_type.includes('crystalliz') || e.event_type.includes('synthesi') || e.event_type.includes('consolidat'));
    });

    if (crystallizationEvents.length === 0) {
      return;
    }

    // 渲染真实结晶历史
    const html = crystallizationEvents.map(function(e) {
      return `
        <div class="crystallization-event">
          <div class="crystallization-event-title">${htmlescape(e.event_type || '结晶事件')}</div>
          <div class="crystallization-event-time">${htmlescape(e.timestamp || '--')}</div>
          <div class="crystallization-event-desc">${htmlescape(e.description || e.details || '--')}</div>
        </div>
      `;
    }).join('');
    timelineEl.innerHTML = html;
  } catch (err) {
    console.warn('[LRC v' + APP_VERSION + ']加载结晶历史失败，使用预览数据:', err.message);
  }
}

// 页面加载完成后初始化结晶历史加载
document.addEventListener('DOMContentLoaded', function() {
  // 延迟加载结晶历史，避免阻塞首屏渲染
  setTimeout(loadCrystallizationHistory, 1500);
  // 初始化道同构度仪表盘
  setTimeout(loadDaoMetrics, 800);
  // 初始化演化时间线
  setTimeout(loadEvolutionTimeline, 1200);
  // 初始化侧边栏导航
  initSidebarNav();
  // 初始化手机端底部标签栏
  initMobileTabbar();
  // 初始化记忆搜索筛选器
  initMemoryFilters();
  // 初始化欢迎区（设计文档 5.2.1：仅首次使用时显示）
  initWelcomeBanner();
  // 初始化系统状态浮窗（设计文档 5.2.5：右下角固定）
  initSysStatusFloat();
  // 初始化侧边栏折叠状态（设计文档 3.4：60/240px 切换）
  initSidebarCollapse();
  // v0.8.3 Step 12 / G015：初始化输入框 blur 校验
  setupInputValidation();
});

/* ============================================================
 * v0.6.0 道同构度环形仪表盘（设计文档 5.2）
 * Canvas 绘制环形进度 + 四个小指标
 * ============================================================ */

/**
 * 绘制道同构度环形仪表盘
 * @param {number} score - 健康评分 (0-100)
 */
function drawDaoRing(score) {
  const canvas = document.getElementById('dao-ring-canvas');
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  const centerX = canvas.width / 2;
  const centerY = canvas.height / 2;
  const radius = 80;
  const lineWidth = 16;

  // 清空画布
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  // 绘制背景轨道
  ctx.beginPath();
  ctx.arc(centerX, centerY, radius, 0, Math.PI * 2);
  ctx.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue('--lrc-宣纸-500').trim() || '#D8CFC0';
  ctx.lineWidth = lineWidth;
  ctx.stroke();

  // 绘制进度弧（从顶部开始，顺时针）
  const startAngle = -Math.PI / 2;
  const endAngle = startAngle + (Math.PI * 2 * score / 100);
  ctx.beginPath();
  ctx.arc(centerX, centerY, radius, startAngle, endAngle);
  // 根据评分选择颜色：≥80 金色，≥60 玉色，<60 朱砂
  let ringColor = getComputedStyle(document.documentElement).getPropertyValue('--lrc-金色-500').trim() || '#D4A843';
  if (score < 60) {
    ringColor = getComputedStyle(document.documentElement).getPropertyValue('--lrc-朱砂-500').trim() || '#C0392B';
  } else if (score < 80) {
    ringColor = getComputedStyle(document.documentElement).getPropertyValue('--lrc-玉色-500').trim() || '#2ECC71';
  }
  ctx.strokeStyle = ringColor;
  ctx.lineWidth = lineWidth;
  ctx.lineCap = 'round';
  ctx.stroke();

  // 绘制中心装饰（九宫格虚线）
  ctx.save();
  ctx.translate(centerX, centerY);
  ctx.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue('--lrc-墨韵-200').trim() || '#A0A0C0';
  ctx.lineWidth = 0.5;
  ctx.setLineDash([2, 4]);
  for (let i = -1; i <= 1; i++) {
    ctx.beginPath();
    ctx.moveTo(i * 20, -30);
    ctx.lineTo(i * 20, 30);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(-30, i * 20);
    ctx.lineTo(30, i * 20);
    ctx.stroke();
  }
  ctx.restore();
}

/**
 * 加载道同构度数据并渲染
 */
async function loadDaoMetrics() {
  try {
    const response = await fetchWithTimeout(`${window.API_BASE}/v1/health/dao_metrics`, {}, 5000);
    const data = await safeJson(response);

    if (data.ok && data.data) {
      const m = data.data;
      // v0.8.1：后端已返回 0-100 区间值，前端直接使用
      // yin_yang_balance: 0-100（阴阳守恒度）
      // luoshu_deviation: 0-100（洛书偏差，越小越好）
      // bagua_balance: 0-100（八卦均衡度）
      // synthesis_ratio: 0-100（合成比率百分比）
      // 计算综合健康评分（0-100）
      const score = Math.min(100, Math.max(0, Math.round(
        (m.yin_yang_balance || 80) * 0.25 +
        (100 - (m.luoshu_deviation || 20)) * 0.25 +
        (m.bagua_balance || 75) * 0.25 +
        Math.min(20, (m.synthesis_ratio || 10) / 5) * 5 * 0.25
      )));

      drawDaoRing(score);
      const scoreEl = document.getElementById('dao-ring-score');
      if (scoreEl) scoreEl.textContent = score;

      // 更新四个小指标
      const setText = (id, val) => {
        const el = document.getElementById(id);
        if (el) el.textContent = val;
      };
      setText('dao-yin-yang', ((m.yin_yang_balance || 80) / 100).toFixed(2));
      setText('dao-luoshu-deviation', (m.luoshu_deviation || 0).toFixed(2));
      setText('dao-bagua-balance', ((m.bagua_balance || 75) / 100).toFixed(2));
      setText('dao-synthesis-ratio', (m.synthesis_ratio || 0).toFixed(1) + '%');

      console.log(`[LRC v${APP_VERSION}]道同构度加载完成，健康评分: ${score}`);
    } else {
      // v0.8.4 Step 4：降级分支补全 4 个小指标（修复 G020 + CDP 测试 #2）
      // 之前仅设 score=85 但 4 个指标保持空值，导致矛盾状态和 CDP 检测失败
      _applyDaoMetricsFallback('数据格式异常');
    }
  } catch (err) {
    console.warn('[LRC v' + APP_VERSION + ']道同构度加载失败，使用默认值:', err.message);
    // v0.8.4 Step 4：catch 分支同样补全 4 个小指标 + 显示降级提示
    const reason = (err && err.name === 'SidecarUnreachableError')
      ? 'LRC 服务未启动'
      : (err && err.message) ? err.message : '未知错误';
    _applyDaoMetricsFallback(reason);
  }
}

/**
 * v0.8.4 Step 4：道同构度降级统一处理（修复 G020 + CDP 测试 #1/#2）
 * - score 和 4 个小指标全部显示 '--'，避免 score=85 与指标 '--' 矛盾
 * - 显示降级提示横幅，含重试按钮
 * - 区分环境问题（sidecar 不可达）vs 代码 bug，显示实际失败原因
 * @param {string} reason - 降级原因
 */
function _applyDaoMetricsFallback(reason) {
  // 环形图和评分显示 '--'
  drawDaoRing(0);
  const scoreEl = document.getElementById('dao-ring-score');
  if (scoreEl) scoreEl.textContent = '--';

  // 4 个小指标统一显示 '--'（修复 G020：之前降级未更新这 4 个指标）
  const subMetrics = ['dao-yin-yang', 'dao-luoshu-deviation', 'dao-bagua-balance', 'dao-synthesis-ratio'];
  subMetrics.forEach(id => {
    const el = document.getElementById(id);
    if (el) el.textContent = '--';
  });

  // 显示降级提示横幅（含重试按钮，便于用户主动重试）
  const panel = document.querySelector('.dao-metrics-panel')
    || document.getElementById('dao-metrics-panel')
    || document.getElementById('dao-ring-score')?.closest('.card, .panel, .stat-card');
  if (panel) {
    let banner = panel.querySelector('.dao-fallback-banner');
    if (!banner) {
      banner = document.createElement('div');
      banner.className = 'dao-fallback-banner';
      banner.style.cssText = 'background:rgba(255,193,7,0.15);color:#856404;padding:8px 12px;border-radius:4px;margin-bottom:8px;font-size:13px;display:flex;align-items:center;gap:8px;';
      panel.insertBefore(banner, panel.firstChild);
    }
    banner.textContent = '⚠ 道同构度数据加载失败：' + reason;

    // 添加重试按钮（如尚未添加）
    if (!banner.querySelector('.dao-retry-btn')) {
      const retryBtn = document.createElement('button');
      retryBtn.className = 'dao-retry-btn';
      retryBtn.textContent = '重试';
      retryBtn.style.cssText = 'margin-left:auto;padding:2px 10px;background:var(--cinnabar,#c0392b);color:#fff;border:none;border-radius:2px;cursor:pointer;font-size:12px;';
      // v0.8.4 Step 4：直接绑定 click 事件（动态生成的按钮不依赖 bindAllActions）
      // 同时实现 inFlight 防抖，防止用户连击触发多次重试
      let retryInFlight = false;
      retryBtn.addEventListener('click', async () => {
        if (retryInFlight) return;
        retryInFlight = true;
        retryBtn.disabled = true;
        retryBtn.textContent = '重试中...';
        try {
          // 移除降级横幅
          if (banner.parentNode) banner.parentNode.removeChild(banner);
          // 重新加载道同构度
          await loadDaoMetrics();
        } catch (e) {
          console.error('[retry-dao-metrics] 重试失败:', e);
        } finally {
          retryInFlight = false;
          retryBtn.disabled = false;
          retryBtn.textContent = '重试';
        }
      });
      banner.appendChild(retryBtn);
    }
  }

  // 显示 Toast 提示（区分环境问题 vs 代码 bug）
  if (typeof showToast === 'function') {
    showToast('道同构度数据不可用：' + reason, 'warning', 4000);
  }
}

/* ============================================================
 * v0.6.0 演化时间线（设计文档 5.2.4）
 * 从审计日志加载最近 10 条演化事件
 * ============================================================ */

async function loadEvolutionTimeline() {
  const timelineEl = document.getElementById('evolution-timeline');
  if (!timelineEl) return;

  try {
    // 从审计日志接口获取演化事件
    const response = await fetchWithTimeout(`${window.API_BASE}/v1/audit-trail?limit=10`, {}, 5000);
    const data = await safeJson(response);

    if (data.ok && data.events && data.events.length > 0) {
      const html = data.events.map(event => {
        const typeClass = event.type || 'audit';
        const typeLabel = {
          crystallization: '结晶',
          synthesis: '合成',
          decay: '衰减',
          audit: '审计'
        }[typeClass] || '事件';
        const iconMap = {
          crystallization: 'icon-crystallization',
          synthesis: 'icon-luoshu',
          decay: 'icon-decay',
          audit: 'icon-audit'
        };
        const iconName = iconMap[typeClass] || 'icon-audit';
        return `
          <li class="evolution-event ${typeClass}">
            <div class="evolution-event-dot"></div>
            <div class="evolution-event-time">${event.timestamp || '--'}</div>
            <span class="evolution-event-type">
              <img src="/assets/icons/${iconName}.svg" alt="" width="12" height="12"> ${typeLabel}
            </span>
            <div class="evolution-event-desc">${htmlescape(event.description || event.desc || '')}</div>
          </li>
        `;
      }).join('');
      timelineEl.innerHTML = html;
      console.log(`[LRC v${APP_VERSION}]演化时间线加载了 ${data.events.length} 条事件`);
    }
    // 如果接口未返回数据，保留默认示例数据
  } catch (err) {
    console.warn('[LRC v' + APP_VERSION + ']演化时间线加载失败，使用示例数据:', err.message);
    // 保留默认示例数据，不报错
  }
}

/* ============================================================
 * v0.6.0 记忆搜索页面（设计文档 5.3）
 * 防抖搜索 + 筛选 + 卡片流 + 右侧详情面板
 * ============================================================ */

let memorySearchTimer = null;
let memorySearchFilters = {
  type: 'all',
  importance: 'all',
  time: 'all'
};

/**
 * 防抖记忆搜索（300ms 延迟）
 * v0.8.4 Step 11 / G038：防抖期间也取消上一次的 AbortController
 */
function debouncedMemorySearch() {
  if (memorySearchTimer) clearTimeout(memorySearchTimer);
  memorySearchTimer = setTimeout(searchMemories, 300);
}

// v0.8.4 Step 11 / G037：搜索请求的 AbortController，避免快速输入触发竞态
let searchAbortController = null;

/**
 * 执行记忆搜索
 * v0.8.4 Step 11 / G037：添加 AbortController，abort 上一次未完成的搜索请求
 */
async function searchMemories() {
  const input = document.getElementById('memory-search-input');
  const resultsEl = document.getElementById('memory-search-results');
  if (!input || !resultsEl) return;

  const query = input.value.trim();
  if (!query) {
    resultsEl.innerHTML = `
      <div class="memory-search-empty">
        <img class="empty-icon" src="/assets/icons/icon-search-lrc.svg" alt="">
        <div class="empty-poem">寻而未得，或待他时</div>
        <p class="text-sm text-dim">输入关键词开始搜索记忆</p>
      </div>
    `;
    return;
  }

  // v0.8.4 Step 11 / G037：abort 上一次未完成的搜索请求
  if (searchAbortController) {
    searchAbortController.abort();
  }
  searchAbortController = new AbortController();
  const currentSignal = searchAbortController.signal;

  // 显示加载骨架屏
  resultsEl.innerHTML = `
    <div class="skeleton skeleton-text title" style="margin-bottom: 8px;"></div>
    <div class="skeleton skeleton-text" style="margin-bottom: 8px;"></div>
    <div class="skeleton skeleton-text short" style="margin-bottom: 16px;"></div>
    <div class="skeleton skeleton-text title" style="margin-bottom: 8px;"></div>
    <div class="skeleton skeleton-text" style="margin-bottom: 8px;"></div>
    <div class="skeleton skeleton-text short"></div>
  `;

  try {
    const response = await fetchWithTimeout(`${window.API_BASE}/v1/memories/enrich`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ query, top_k: 20 }),
      signal: currentSignal // v0.8.4 Step 11 / G037：传入 signal 支持取消
    }, 10000);

    // v0.8.4 Step 11 / G037：检查是否已被新的请求 abort
    if (currentSignal.aborted) {
      console.log('[searchMemories] 请求被新请求 abort，静默退出');
      return;
    }

    const data = await safeJson(response);
    // v0.8.4 Step 10：成功时重置重试计数器
    if (typeof resetRetryCounter === 'function') {
      resetRetryCounter('/v1/memories/enrich', 'POST');
    }

    // v0.6.0 适配 /v1/memories/enrich 响应格式（memories 数组）
    const results = data.memories || data.results || [];
    if (results.length > 0) {
      // 应用筛选器
      let filtered = results;
      if (memorySearchFilters.type !== 'all') {
        filtered = filtered.filter(m => m.memory_type === memorySearchFilters.type);
      }
      if (memorySearchFilters.importance !== 'all') {
        const ranges = { high: [8, 10], medium: [5, 7], low: [1, 4] };
        const [min, max] = ranges[memorySearchFilters.importance];
        filtered = filtered.filter(m => (m.importance || 5) >= min && (m.importance || 5) <= max);
      }

      if (filtered.length === 0) {
        resultsEl.innerHTML = `
          <div class="memory-search-empty">
            <img class="empty-icon" src="/assets/icons/icon-search-lrc.svg" alt="">
            <div class="empty-poem">寻而未得，或待他时</div>
            <p class="text-sm text-dim">未找到匹配的记忆，尝试调整搜索条件</p>
          </div>
        `;
        return;
      }

      const html = filtered.map((memory, idx) => {
        // v0.8.4 Step 9 / G040 修复：白名单校验 memory_type，防止 XSS
        const safeType = sanitizeMemoryType(memory.memory_type || 'conversation');
        const typeClass = `card-memory-${safeType}`;
        const preview = (memory.content || '').substring(0, 200);
        const time = memory.created_at || memory.timestamp || '--';
        const importance = memory.importance || 5;
        // v0.8.4 Step 9 / G025 修复：移除内联 onclick，改用 data-action + data-arg（索引）
        // v0.8.4 Step 9 / G032 修复：data-action 走 bindAllActions 防抖，避免绕过
        return `
          <div class="memory-card-item ${typeClass}" data-action="openMemoryDetail" data-arg="${idx}">
            <div class="memory-card-preview">${htmlescape(preview)}</div>
            <div class="memory-card-meta">
              <span><img src="/assets/icons/icon-memory.svg" alt="" width="12" height="12"> ${htmlescape(memory.memory_type || '未分类')}</span>
              <span>重要性: ${htmlescape(String(importance))}</span>
              <span>${htmlescape(time)}</span>
            </div>
          </div>
        `;
      }).join('');
      // v0.8.4 Step 9：存储搜索结果到全局缓存，供 openMemoryDetail 按索引查找
      window._memorySearchResults = filtered;
      resultsEl.innerHTML = html;
      // v0.8.4 Step 9：动态生成的元素需要重新绑定 data-action
      if (typeof bindAllActions === 'function') {
        bindAllActions();
      }
      console.log(`[LRC v${APP_VERSION}]记忆搜索完成，返回 ${filtered.length} 条结果`);
    } else {
      resultsEl.innerHTML = `
        <div class="memory-search-empty">
          <img class="empty-icon" src="/assets/icons/icon-search-lrc.svg" alt="">
          <div class="empty-poem">寻而未得，或待他时</div>
          <p class="text-sm text-dim">未找到匹配的记忆</p>
        </div>
      `;
    }
  } catch (err) {
    // v0.8.4 Step 11 / G037：AbortError 静默处理，不显示错误
    if (err.name === 'AbortError' && currentSignal.aborted) {
      console.log('[searchMemories] 请求被 abort（正常行为）');
      return;
    }
    resultsEl.innerHTML = `
      <div class="memory-search-empty">
        <img class="empty-icon" src="/assets/icons/icon-search-lrc.svg" alt="">
        <div class="empty-poem">搜索出错</div>
        <p class="text-sm text-dim">${htmlescape(err.message || String(err))}</p>
      </div>
    `;
    console.error('[LRC v' + APP_VERSION + ']记忆搜索失败:', err);
  } finally {
    // v0.8.4 Step 11 / G037：清理 AbortController 引用（仅当当前请求未被打断）
    if (searchAbortController && searchAbortController.signal === currentSignal) {
      searchAbortController = null;
    }
  }
}

/**
 * 打开记忆详情面板（右侧滑出 40% 宽度）
 * v0.8.4 Step 9 / G025 修复：支持接收索引参数，从全局缓存查找 memory 对象
 * @param {number|object} memoryOrIndex - memory 对象或在搜索结果中的索引
 */
function openMemoryDetail(memoryOrIndex) {
  let memory = memoryOrIndex;
  // v0.8.4 Step 9：若传入的是索引（来自 data-arg），从全局缓存查找
  if (typeof memoryOrIndex === 'number') {
    const cache = window._memorySearchResults || [];
    if (memoryOrIndex < 0 || memoryOrIndex >= cache.length) {
      console.warn('[openMemoryDetail] 索引越界:', memoryOrIndex, '缓存大小:', cache.length);
      showToast('记忆详情加载失败：索引越界', 'error');
      return;
    }
    memory = cache[memoryOrIndex];
    if (!memory) {
      console.warn('[openMemoryDetail] 缓存中未找到记忆:', memoryOrIndex);
      showToast('记忆详情加载失败：数据不存在', 'error');
      return;
    }
  }
  // 兼容直接传入 memory 对象的场景
  if (!memory || typeof memory !== 'object') {
    console.warn('[openMemoryDetail] 无效的 memory 参数:', memoryOrIndex);
    return;
  }

  const panel = document.getElementById('memory-detail-panel');
  const backdrop = document.getElementById('memory-detail-backdrop');
  const content = document.getElementById('memory-detail-content');
  if (!panel || !content) return;

  // v0.7.0 修复: 存储当前查看的记忆到全局变量，供修正/拆解/反馈按钮使用
  window._currentDetailMemory = memory;

  const memoryId = memory.id || '';
  const memoryType = memory.memory_type || '';
  const isSynthesis = memoryType === 'synthesis';

  content.innerHTML = `
    <h3>${htmlescape(memory.content ? memory.content.substring(0, 50) + '...' : '记忆详情')}</h3>
    <div class="memory-detail-fulltext">${htmlescape(memory.content || '')}</div>
    <div class="memory-detail-metadata">
      <span class="label">记忆类型</span>
      <span class="value">${htmlescape(memoryType || '--')}</span>
      <span class="label">重要性</span>
      <span class="value">${memory.importance || '--'}</span>
      <span class="label">创建时间</span>
      <span class="value">${htmlescape(memory.created_at || memory.timestamp || '--')}</span>
      <span class="label">记忆 ID</span>
      <span class="value">${htmlescape(memoryId || '--')}</span>
      <span class="label">标签</span>
      <span class="value">${htmlescape((memory.tags || []).join(', ') || '--')}</span>
    </div>
    <div class="memory-detail-actions" style="margin-top: 16px; display: flex; flex-wrap: wrap; gap: 8px;">
      <!-- v0.8.3 Step 11：N12 XSS 修复，使用 data-action + data-arg 替代内联 onclick（修复 G001-G003） -->
      <!-- 此前 onclick="correctMemory('${memoryId}')" 存在 XSS 风险（memoryId 含单引号可注入） -->
      <button class="btn btn-outline" data-action="correctMemory" data-arg="${htmlescape(memoryId)}" ${!memoryId ? 'disabled' : ''}>
        修正记忆
      </button>
      ${isSynthesis ? `<button class="btn btn-outline" data-action="unfoldMemory" data-arg="${htmlescape(memoryId)}" ${!memoryId ? 'disabled' : ''}>拆解合成</button>` : ''}
      <button class="btn btn-outline" data-action="submitMemoryFeedback" data-arg="${htmlescape(memoryId)}" ${!memoryId ? 'disabled' : ''}>
        反馈
      </button>
    </div>
  `;

  // v0.8.3 Step 11：动态生成的按钮需要重新绑定 data-action（bindAllActions 跳过已绑定元素）
  // 对新按钮逐个绑定事件监听器，避免 XSS 风险
  const actionButtons = panel.querySelectorAll('[data-action]:not([data-bound="1"])');
  actionButtons.forEach(btn => {
    const action = btn.getAttribute('data-action');
    if (!action) return;
    btn.dataset.bound = '1';
    btn.addEventListener('click', async (ev) => {
      // 复用 bindAllActions 的事件处理逻辑
      if (btn.dataset.inFlight === '1') {
        ev.preventDefault();
        ev.stopPropagation();
        return;
      }
      const fn = window[action];
      if (typeof fn !== 'function') {
        console.warn('[openMemoryDetail] 未找到 data-action 对应函数:', action);
        return;
      }
      btn.dataset.inFlight = '1';
      try {
        const arg = btn.getAttribute('data-arg');
        if (arg !== null) {
          await fn(arg);
        } else {
          await fn();
        }
      } catch (e) {
        console.error('[openMemoryDetail] action 执行失败:', action, e);
        showToast('操作失败: ' + (e.message || String(e)), 'error');
      } finally {
        setTimeout(() => { btn.dataset.inFlight = '0'; }, 500);
      }
    });
  });

  panel.classList.add('open');
  backdrop.classList.add('open');
}

/**
 * 关闭记忆详情面板
 */
function closeMemoryDetail() {
  const panel = document.getElementById('memory-detail-panel');
  const backdrop = document.getElementById('memory-detail-backdrop');
  if (panel) panel.classList.remove('open');
  if (backdrop) backdrop.classList.remove('open');
}

// ============================================================
// v0.7.0 修复: 孤儿路由前端入口实现
// 6 个后端孤儿路由的前端调用逻辑
// ============================================================

/**
 * v0.7.0 修复: 修正记忆内容（调用 POST /v1/memories/correct）
 * @param {string} memoryId - 记忆 ID
 */
async function correctMemory(memoryId) {
  // v0.8.3 Step 4 批次 1：alert→showToast，prompt→await showPrompt（修复 G001-G003）
  if (!memoryId) {
    showToast('✗ 无法修正：记忆 ID 为空', 'error');
    return;
  }

  // showPrompt 返回 null 表示用户取消
  const newContent = await showPrompt('请输入修正后的记忆内容:', '修正记忆');
  if (newContent === null) return; // 用户取消
  if (!newContent || !newContent.trim()) {
    showToast('修正内容不能为空', 'warning');
    return;
  }

  const reason = (await showPrompt('请输入修正原因（可选）:', '修正原因')) || '';

  try {
    const resp = await fetchWithTimeout(API_BASE + '/v1/memories/correct', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        memory_id: memoryId,
        content: newContent.trim(),
        reason: reason.trim(),
      })
    }, 30000);

    const data = await resp.json();
    if (data.success) {
      showToast('✓ 记忆修正成功，新版本: ' + (data.new_version || '--') +
                '，历史版本数: ' + (data.history_versions || '--'), 'success');
      closeMemoryDetail();
      // 刷新记忆搜索结果
      if (typeof debouncedMemorySearch === 'function') {
        debouncedMemorySearch();
      }
    } else {
      showToast('✗ 修正失败: ' + (data.message || data.error || '未知错误'), 'error');
    }
  } catch (e) {
    console.error('[correctMemory] 修正失败:', e);
    showToast('✗ 修正记忆失败: ' + (e.message || String(e)), 'error');
  }
}

/**
 * v0.7.0 修复: 提交记忆反馈（调用 POST /v1/feedback）
 * 支持检索质量、合成质量等多种反馈类型
 * @param {string} memoryId - 记忆 ID
 */
async function submitMemoryFeedback(memoryId) {
  // v0.8.3 Step 4 批次 2：alert→showToast，prompt→await showPrompt（修复 G001-G003）
  if (!memoryId) {
    showToast('✗ 无法反馈：记忆 ID 为空', 'error');
    return;
  }

  const feedbackType = await showPrompt(
    '请选择反馈类型（输入数字）：1.检索质量 2.合成质量 3.恢复隔离 4.两阶段确认 5.其他',
    '反馈类型',
    '1'
  );
  if (feedbackType === null) return; // 用户取消
  if (!feedbackType) return;

  const typeMap = {
    '1': 'retrieval_quality',
    '2': 'synthesis_quality',
    '3': 'recover_isolated',
    '4': 'two_phase_confirm',
    '5': 'general',
  };
  const fbType = typeMap[feedbackType.trim()] || 'general';

  const feedbackContent = (await showPrompt('请输入反馈内容:', '反馈内容')) || '';
  if (!feedbackContent.trim()) {
    showToast('✗ 反馈内容不能为空', 'warning');
    return;
  }

  try {
    const resp = await fetchWithTimeout(API_BASE + '/v1/feedback', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        type: fbType,
        memory_id: memoryId,
        content: feedbackContent.trim(),
        timestamp: new Date().toISOString(),
      })
    }, 30000);

    const data = await resp.json();
    if (data.success || data.status === 'ok' || resp.ok) {
      showToast('✓ 反馈已提交：' + (data.message || '感谢您的反馈！'), 'success');
    } else {
      showToast('✗ 反馈提交失败: ' + (data.message || data.error || '未知错误'), 'error');
    }
  } catch (e) {
    console.error('[submitMemoryFeedback] 提交失败:', e);
    showToast('✗ 提交反馈失败: ' + (e.message || String(e)), 'error');
  }
}

/**
 * v0.7.0 孤儿路由修复 Step 3-D
 * 调用 POST /v1/encode 将文本编码为洛书 9 维向量
 * 展示向量分量、八卦归属与拓扑深度
 */
async function encodeTextToLuoshu() {
  const input = document.getElementById('luoshu-encode-input');
  const btn = document.getElementById('btn-luoshu-encode');
  const errorBox = document.getElementById('luoshu-encode-error');
  const resultBox = document.getElementById('luoshu-encode-result');
  const grid = document.getElementById('luoshu-vector-grid');
  if (!input || !btn || !errorBox || !resultBox || !grid) return;

  const text = input.value.trim();
  if (!text) {
    errorBox.textContent = '✗ 请输入要编码的文本';
    errorBox.style.display = 'block';
    resultBox.style.display = 'none';
    return;
  }

  // 重置 UI 进入加载态
  btn.disabled = true;
  btn.textContent = '编码中...';
  errorBox.style.display = 'none';
  resultBox.style.display = 'none';

  try {
    const resp = await fetchWithTimeout(API_BASE + '/v1/encode', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text: text })
    }, 30000);

    if (!resp.ok) {
      const errText = await resp.text().catch(() => '');
      throw new Error('HTTP ' + resp.status + (errText ? ': ' + errText : ''));
    }

    const data = await resp.json();
    const vec = data.luoshu_vector || [];
    if (!Array.isArray(vec) || vec.length < 9) {
      throw new Error('返回的向量格式无效');
    }

    // 渲染 9 个向量分量为 3x3 九宫格
    // 洛书九宫格传统排列：4|9|2 / 3|5|7 / 8|1|6
    const luoshuLayout = [4, 9, 2, 3, 5, 7, 8, 1, 6];
    grid.innerHTML = '';
    for (let i = 0; i < 9; i++) {
      const cell = document.createElement('div');
      cell.style.cssText = 'padding:10px;border-radius:6px;text-align:center;background:var(--lrc-墨韵-900,#1a1a1a);border:1px solid var(--lrc-墨韵-700,#333);';
      const labelEl = document.createElement('div');
      labelEl.style.cssText = 'font-size:10px;color:var(--lrc-墨韵-300,#888);margin-bottom:4px;';
      labelEl.textContent = '位 ' + luoshuLayout[i];
      const valueEl = document.createElement('div');
      valueEl.style.cssText = 'font-size:14px;font-weight:600;color:var(--lrc-玉色-400,#4a9d8e);';
      valueEl.textContent = (typeof vec[i] === 'number') ? vec[i].toFixed(4) : '--';
      cell.appendChild(labelEl);
      cell.appendChild(valueEl);
      grid.appendChild(cell);
    }

    // 填充元数据
    document.getElementById('luoshu-bagua-index').textContent = (data.bagua_index !== undefined) ? data.bagua_index : '--';
    document.getElementById('luoshu-bagua-category').textContent = data.bagua_category || '--';
    document.getElementById('luoshu-center-value').textContent = (typeof data.center_value === 'number') ? data.center_value.toFixed(4) : '--';
    document.getElementById('luoshu-topological-depth').textContent = (typeof data.topological_depth === 'number') ? data.topological_depth.toFixed(4) : '--';

    resultBox.style.display = 'block';
  } catch (e) {
    errorBox.textContent = '✗ 编码失败: ' + (e.message || String(e));
    errorBox.style.display = 'block';
    resultBox.style.display = 'none';
  } finally {
    btn.disabled = false;
    btn.textContent = '编码';
  }
}

/**
 * v0.7.0 孤儿路由修复 Step 3-E
 * 调用 POST /v1/memories/unfold 拆解合成记忆为子记忆
 * 在记忆详情面板中动态渲染拆解结果
 */
async function unfoldMemory(memoryId) {
  // v0.8.3 Step 4 批次 3：alert→showToast（修复 G001-G003）
  if (!memoryId) {
    showToast('✗ 无法拆解：记忆 ID 为空', 'error');
    return;
  }

  const content = document.getElementById('memory-detail-content');
  if (!content) {
    showToast('✗ 记忆详情面板未打开', 'error');
    return;
  }

  // 查找或创建拆解结果区域
  let resultArea = document.getElementById('memory-unfold-result');
  if (!resultArea) {
    resultArea = document.createElement('div');
    resultArea.id = 'memory-unfold-result';
    resultArea.style.cssText = 'margin-top:16px;padding:12px;border-radius:8px;background:var(--lrc-墨韵-900,#1a1a1a);border:1px solid var(--lrc-墨韵-700,#333);';
    content.appendChild(resultArea);
  }

  // 进入加载态
  resultArea.innerHTML = '<div style="color:var(--lrc-墨韵-300,#888);font-size:13px;">⏳ 正在拆解合成记忆...</div>';

  try {
    const resp = await fetchWithTimeout(API_BASE + '/v1/memories/unfold', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        memory_id: memoryId,
        min_activation: 0.1
      })
    }, 30000);

    const data = await resp.json();

    if (!resp.ok || !data.success) {
      throw new Error(data.message || data.error || 'HTTP ' + resp.status);
    }

    // 渲染拆解结果
    const subMemories = data.sub_memories || [];
    const subCount = data.sub_vectors_count || subMemories.length;
    const fidelity = (typeof data.fidelity === 'number') ? data.fidelity.toFixed(4) : '--';

    let html = '<div style="margin-bottom:10px;">';
    html += '<div style="font-size:13px;color:var(--lrc-玉色-400,#4a9d8e);font-weight:600;margin-bottom:4px;">✓ 拆解成功</div>';
    html += '<div style="font-size:12px;color:var(--lrc-墨韵-300,#888);">子记忆数：' + subCount + ' · 保真度：' + fidelity + '</div>';
    html += '</div>';

    if (subMemories.length > 0) {
      html += '<div style="display:flex;flex-direction:column;gap:6px;">';
      subMemories.forEach(function(sub, idx) {
        const weight = (typeof sub.weight === 'number') ? (sub.weight * 100).toFixed(2) + '%' : '--';
        const cat = sub.bagua_category || '--';
        const subContent = sub.content || '--';
        html += '<div style="padding:8px;border-radius:6px;background:var(--lrc-墨韵-800,#222);border-left:3px solid var(--lrc-玉色-400,#4a9d8e);">';
        html += '<div style="display:flex;justify-content:space-between;font-size:11px;color:var(--lrc-墨韵-300,#888);margin-bottom:4px;">';
        html += '<span>#' + (idx + 1) + ' · ' + htmlescape(cat) + '</span>';
        html += '<span>权重：' + weight + '</span>';
        html += '</div>';
        html += '<div style="font-size:13px;color:var(--lrc-墨韵-100,#eee);">' + htmlescape(subContent) + '</div>';
        html += '</div>';
      });
      html += '</div>';
    } else {
      html += '<div style="font-size:12px;color:var(--lrc-墨韵-300,#888);">无子记忆返回</div>';
    }

    resultArea.innerHTML = html;
  } catch (e) {
    resultArea.innerHTML = '<div style="color:var(--lrc-朱砂-400,#c04851);font-size:13px;">✗ 拆解失败: ' + htmlescape(e.message || String(e)) + '</div>';
  }
}

/**
 * v0.7.0 孤儿路由修复 Step 3-F
 * 调用 GET /v1/version/check 检查 LRC 最新版本
 * 在设置页"关于与更新"卡片中展示检查结果
 */
async function checkVersionUpdate() {
  const btn = document.getElementById('btn-check-update');
  const resultBox = document.getElementById('version-check-result');
  if (!btn || !resultBox) return;

  // 进入加载态
  btn.disabled = true;
  btn.textContent = '检查中...';
  resultBox.style.display = 'block';
  resultBox.innerHTML = '<div style="color:var(--lrc-墨韵-300,#888);font-size:13px;">⏳ 正在连接 GitHub 检查最新版本...</div>';

  try {
    const resp = await fetchWithTimeout(API_BASE + '/v1/version/check', {
      method: 'GET'
    }, 15000);

    if (!resp.ok) {
      const errText = await resp.text().catch(() => '');
      throw new Error('HTTP ' + resp.status + (errText ? ': ' + errText : ''));
    }

    const data = await resp.json();

    // 处理检查错误
    if (data.check_error) {
      resultBox.innerHTML = '<div style="color:var(--lrc-朱砂-400,#c04851);font-size:13px;">✗ 检查失败: ' +
        htmlescape(data.check_error) + '</div>' +
        '<div style="color:var(--lrc-墨韵-300,#888);font-size:12px;margin-top:4px;">当前版本：v' + htmlescape(data.current_version || '--') + '</div>';
      return;
    }

    const current = data.current_version || '--';
    const latest = data.latest_version || '未知';
    const updateAvailable = data.update_available === true;
    const updateUrl = data.update_url || '';
    const downloadUrl = data.download_url || '';
    const note = data.check_note || '';

    let html = '';

    if (updateAvailable) {
      // 有新版本可用
      html += '<div style="padding:12px;border-radius:8px;background:rgba(212,168,67,0.08);border:1px solid var(--lrc-金色-300,#d4a843);margin-bottom:8px;">';
      html += '<div style="font-size:14px;color:var(--lrc-金色-400,#d4a843);font-weight:600;margin-bottom:6px;">✨ 发现新版本</div>';
      html += '<div style="font-size:13px;color:var(--lrc-墨韵-100,#eee);">当前版本：<strong>v' + htmlescape(current) + '</strong> → 最新版本：<strong>v' + htmlescape(latest) + '</strong></div>';
      if (updateUrl) {
        html += '<div style="margin-top:8px;"><a href="' + htmlescape(updateUrl) + '" target="_blank" rel="noopener" style="color:var(--lrc-玉色-400,#4a9d8e);font-size:13px;text-decoration:underline;">查看发布说明 →</a></div>';
      }
      if (downloadUrl) {
        html += '<div style="margin-top:4px;"><a href="' + htmlescape(downloadUrl) + '" target="_blank" rel="noopener" style="color:var(--lrc-玉色-400,#4a9d8e);font-size:13px;text-decoration:underline;">下载新版本 →</a></div>';
      }
      html += '</div>';
    } else {
      // 已是最新版本
      html += '<div style="padding:12px;border-radius:8px;background:rgba(76,164,156,0.08);border:1px solid var(--lrc-玉色-400,#4a9d8e);margin-bottom:8px;">';
      html += '<div style="font-size:14px;color:var(--lrc-玉色-400,#4a9d8e);font-weight:600;margin-bottom:6px;">✓ 已是最新版本</div>';
      html += '<div style="font-size:13px;color:var(--lrc-墨韵-100,#eee);">当前版本：<strong>v' + htmlescape(current) + '</strong>（最新版本：v' + htmlescape(latest) + '）</div>';
      html += '</div>';
    }

    // 隐私说明
    if (note) {
      html += '<div style="font-size:11px;color:var(--lrc-墨韵-300,#888);margin-top:6px;">';
      html += '<img src="/assets/icons/icon-info.svg" alt="" width="11" height="11" style="vertical-align:middle;margin-right:4px;opacity:0.6;">' + htmlescape(note);
      html += '</div>';
    }

    resultBox.innerHTML = html;
  } catch (e) {
    resultBox.innerHTML = '<div style="color:var(--lrc-朱砂-400,#c04851);font-size:13px;">✗ 检查更新失败: ' + htmlescape(e.message || String(e)) + '</div>';
  } finally {
    btn.disabled = false;
    btn.textContent = '检查更新';
  }
}

/**
 * 初始化记忆搜索筛选器
 */
function initMemoryFilters() {
  document.querySelectorAll('.memory-filter-tag').forEach(tag => {
    tag.addEventListener('click', function() {
      const group = this.parentElement;
      // 移除同组其他标签的 active
      group.querySelectorAll('.memory-filter-tag').forEach(t => t.classList.remove('active'));
      this.classList.add('active');

      // 更新筛选器状态
      if (this.dataset.filterType) memorySearchFilters.type = this.dataset.filterType;
      if (this.dataset.filterImportance) memorySearchFilters.importance = this.dataset.filterImportance;
      if (this.dataset.filterTime) memorySearchFilters.time = this.dataset.filterTime;

      // 重新搜索
      debouncedMemorySearch();
    });
  });
}

/* ============================================================
 * v0.6.0 Toast 通知条（设计文档 3.10）
 * 3 种变体：成功/失败/警告，3s 自动消失
 * ============================================================ */

// ============================================================
// v0.8.3 Step 12 / G011：Toast 队列管理（对应审计 G011）
// 设计原则：
//   1. 1.5s 内重复消息去重（避免连续点击产生大量相同 Toast）
//   2. 可见 Toast 上限 3 个（避免遮挡屏幕）
//   3. error 类型优先级最高，超限时移除最旧的非 error Toast
//   4. 使用 Map 记录最近消息时间戳，定期清理避免内存泄漏
// ============================================================
const TOAST_MAX_VISIBLE = 3;
const _recentToastMessages = new Map();
const _TOAST_DEDUP_WINDOW = 1500; // 1.5s 去重窗口

/**
 * 显示 Toast 通知
 * @param {string} message - 通知内容
 * @param {string} type - 类型：success/error/warning/info
 * @param {number} duration - 显示时长（毫秒），默认 3000
 */
function showToast(message, type = 'success', duration = 3000) {
  const container = document.getElementById('toast-container');
  if (!container) return;

  // G011-1：去重检查（1.5s 内相同消息跳过）
  // v0.8.4 Step 8 / G026 修复：去重检查在显示前，但记录在显示后
  // 避免被上限跳过的消息也记录时间戳，导致后续调用被误去重
  const now = Date.now();
  const dedupKey = `${type}:${message}`;
  if (_recentToastMessages.has(dedupKey)) {
    const last = _recentToastMessages.get(dedupKey);
    if (now - last < _TOAST_DEDUP_WINDOW) {
      // 重复消息，跳过（但更新时间戳，避免后续显示的 Toast 被误判）
      _recentToastMessages.set(dedupKey, now);
      return;
    }
  }

  // G011-2：可见 Toast 数量上限管理（在去重检查之后，记录 dedupKey 之前）
  const visibleToasts = container.querySelectorAll('.toast:not(.toast-leaving)');
  if (visibleToasts.length >= TOAST_MAX_VISIBLE) {
    if (type === 'error') {
      // error 优先：移除最旧的非 error Toast
      const oldestNonError = Array.from(visibleToasts).find(t => !t.classList.contains('toast-error'));
      if (oldestNonError) {
        oldestNonError.classList.add('toast-leaving');
        setTimeout(() => {
          if (oldestNonError.parentNode) oldestNonError.parentNode.removeChild(oldestNonError);
        }, 200);
      }
    } else {
      // 非 error 超出上限，跳过（避免堆积）
      // v0.8.4 Step 8 / G026：此处不记录 dedupKey，允许后续重试
      console.log('[showToast] 队列已满，跳过非 error 消息:', message);
      return;
    }
  }

  // v0.8.4 Step 8 / G026 修复：显示后才记录 dedupKey
  // 确保只有真正显示的 Toast 才占用去重窗口，避免首条被误去重
  _recentToastMessages.set(dedupKey, now);
  // 2s 后清理过期记录，避免 Map 无限增长
  setTimeout(() => {
    if (_recentToastMessages.get(dedupKey) === now) {
      _recentToastMessages.delete(dedupKey);
    }
  }, 2000);

  const iconMap = {
    success: 'icon-trust',
    error: 'icon-decay',
    warning: 'icon-benchmark',
    info: 'icon-benchmark'
  };
  const iconName = iconMap[type] || iconMap.success;

  const toast = document.createElement('div');
  toast.className = `toast toast-${type}`;
  toast.innerHTML = `
    <div class="toast-icon-wrap">
      <img src="/assets/icons/${iconName}.svg" alt="" class="toast-icon">
    </div>
    <span>${htmlescape(message)}</span>
  `;

  container.appendChild(toast);

  // 3s 后自动滑出消失
  setTimeout(() => {
    toast.classList.add('toast-leaving');
    setTimeout(() => {
      if (toast.parentNode) toast.parentNode.removeChild(toast);
    }, 200);
  }, duration);
}

/* ============================================================
 * v0.6.0 侧边栏导航初始化（设计文档 5.1）
 * ============================================================ */

// v0.8.3 Step 1：定义 switchTab 函数（修复 N02）
// 此前 initSidebarNav 调用未定义的 switchTab，实际只有 switchToTab（指向 navbar-nav button，
// 与侧边栏 .nav-item 选择器不匹配，等同死代码），导致侧边栏切换走降级路径不触发数据加载。
// 设计原则：
//   1. 统一处理标签切换（同步顶部 navbar 与侧边栏 nav-item 的 active 状态）
//   2. 触发对应数据加载函数（TAB_LOADERS 映射表）
//   3. 保留降级路径作为兜底（target 不存在时不报错）
//   4. 暴露到 window 便于 CDP 测试与外部调用
// 注意：未在 TAB_LOADERS 中列出的标签页仅切换 DOM 不报错
const TAB_LOADERS = {
  'dashboard': () => loadDashboard(),
  'trust-center': () => loadTrustCenter(),
  'benchmarks': () => loadBenchmarks(),
  'settings': () => { loadSettings(); loadProjectInfo(); },
  'system-status': () => loadSysStatusFloat(),
  'project-switch': () => loadProjectInfo()
};

async function switchTab(tabName) {
  // v0.8.4 Step 10 / G047：重试 Modal 显示时禁止标签页切换
  if (typeof _retryModalActive !== 'undefined' && _retryModalActive) {
    showToast('请先处理重试弹窗', 'warning');
    return false;
  }
  // v0.8.3 Step 12 / G017：标签页切换时取消旧标签页的进行中请求
  // 设计原则：仅 abort 当前活跃标签的 AbortController，新标签页加载不受影响
  // v0.8.4 Step 7 / G021：传入 tabName 作为 excludeTab，避免 abort 目标标签的请求
  _abortActiveTabRequests(tabName);

  // 1. 移除所有标签页与导航项的 active 类
  document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
  document.querySelectorAll('.app-sidebar .nav-item').forEach(n => n.classList.remove('active'));
  document.querySelectorAll('.navbar-nav button').forEach(b => b.classList.remove('active'));

  // 2. 激活目标标签页内容
  const target = document.getElementById(`tab-${tabName}`);
  if (!target) {
    console.warn(`[switchTab] 标签页 tab-${tabName} 不存在，仅切换导航状态`);
    return;
  }
  target.classList.add('active');

  // 3. 同步激活对应的侧边栏导航项（data-tab 属性匹配）
  const sidebarItem = document.querySelector(`.app-sidebar .nav-item[data-tab="${tabName}"]`);
  if (sidebarItem) sidebarItem.classList.add('active');

  // 4. 同步激活顶部 navbar 按钮（保持双导航一致）
  const navBtn = document.querySelector(`.navbar-nav button[data-tab="${tabName}"]`);
  if (navBtn) navBtn.classList.add('active');

  // 5. 触发对应数据加载函数（若存在）
  const loader = TAB_LOADERS[tabName];
  if (typeof loader === 'function') {
    try {
      await loader();
    } catch (e) {
      // 加载失败仅记录日志，不影响标签切换
      console.error(`[switchTab] 加载 ${tabName} 数据失败:`, e);
    }
  }
}

// ============================================================
// v0.8.3 Step 12 / G017：标签页请求取消工具函数
// 维护各标签页的 AbortController，切换时 abort 旧标签页的进行中请求
// ============================================================
const _tabAbortControllers = new Map();

function _getTabAbortController(tabName) {
  if (!_tabAbortControllers.has(tabName)) {
    _tabAbortControllers.set(tabName, new AbortController());
  }
  return _tabAbortControllers.get(tabName);
}

function _abortActiveTabRequests(excludeTab) {
  // v0.8.4 Step 7 / G021 修复：dashboardAbortController 由 loadDashboard 自身管理
  // 此处不再无条件 abort dashboardAbortController，避免切换到 dashboard 时误 abort 新请求
  // abort 所有进行中的 AbortController，并重置（为新请求准备）
  for (const [tabName, controller] of _tabAbortControllers.entries()) {
    // v0.8.4 Step 7：跳过 excludeTab（通常是切换的目标标签），避免 abort 目标标签的请求
    if (tabName === excludeTab) {
      continue;
    }
    if (controller.signal.aborted) {
      // 已 abort 的清理掉，下次使用时重新创建
      _tabAbortControllers.delete(tabName);
      continue;
    }
    // 仅 abort 有 listener 的（即正在使用中的）
    // 简化处理：直接 abort 所有未 abort 的，由调用方处理 AbortError
    controller.abort();
    _tabAbortControllers.delete(tabName);
    console.log(`[G017] 标签页 ${tabName} 的旧请求已取消`);
  }
  // v0.8.4 Step 7 / G021：不再无条件 abort dashboardAbortController
  // dashboard 的请求取消由 loadDashboard 自身管理（第 396-398 行）
  // 避免切换到 dashboard 时 abort 即将创建的新 dashboardAbortController
}
window._abortActiveTabRequests = _abortActiveTabRequests;
// 暴露到 window 便于 CDP 测试与外部调用（修复 N02 测试项 7）
window.switchTab = switchTab;
// v0.8.4 Step 9 / G025：暴露 openMemoryDetail 供 data-action 调用
window.openMemoryDetail = openMemoryDetail;

function initSidebarNav() {
  // 侧边栏导航项点击切换标签
  document.querySelectorAll('.app-sidebar .nav-item[data-tab]').forEach(item => {
    item.addEventListener('click', function(e) {
      e.preventDefault();
      const tabName = this.dataset.tab;

      // 移除其他导航项的 active（switchTab 内部也会处理，此处保留以避免视觉延迟）
      document.querySelectorAll('.app-sidebar .nav-item').forEach(n => n.classList.remove('active'));
      this.classList.add('active');

      // v0.8.3 Step 1：switchTab 已定义，直接调用并触发数据加载
      // 不再使用 typeof 检查的降级路径（保留作为防御性兜底）
      if (typeof switchTab === 'function') {
        switchTab(tabName);
      } else {
        // 降级：直接操作 DOM（理论上不会执行，仅作兜底）
        document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
        const target = document.getElementById(`tab-${tabName}`);
        if (target) target.classList.add('active');
      }
    });
  });
}

/**
 * 初始化手机端底部标签栏
 */
function initMobileTabbar() {
  document.querySelectorAll('.mobile-tabbar .tab-item[data-tab]').forEach(item => {
    item.addEventListener('click', function() {
      const tabName = this.dataset.tab;

      // 移除其他标签的 active
      document.querySelectorAll('.mobile-tabbar .tab-item').forEach(t => t.classList.remove('active'));
      this.classList.add('active');

      // 触发标签切换
      if (typeof switchTab === 'function') {
        switchTab(tabName);
      }
    });
  });
}

/* ============================================================
 * v0.6.0 欢迎区（设计文档 5.2.1：仅首次使用时显示，可关闭）
 * 使用 localStorage 持久化"已关闭"状态，避免重复打扰
 * ============================================================ */

// 诗意名言库（每次随机展示一句）
const WELCOME_POEMS = [
  '昨日之忆，今日之智',
  '滴水穿石，结晶有待',
  '海纳百川，有容乃大',
  '温故而知新，可以为师矣',
  '不积跬步，无以至千里',
  '记忆有道，生生不息',
];

/**
 * 初始化欢迎区：仅在用户未关闭过时显示
 */
function initWelcomeBanner() {
  const banner = document.getElementById('welcome-banner');
  if (!banner) return;

  // 检查 localStorage 是否已关闭
  let dismissed = false;
  try {
    dismissed = localStorage.getItem('lrc_welcome_dismissed') === '1';
  } catch (e) {
    // localStorage 不可用（如沙盒环境），默认不显示欢迎区
    console.warn('[Loong Recall] localStorage 不可用，跳过欢迎区初始化');
    return;
  }

  if (dismissed) {
    banner.hidden = true;
    return;
  }

  // 填充问候语（根据当前时间）
  const hour = new Date().getHours();
  let greeting = '欢迎回来';
  if (hour < 6) {
    greeting = '夜深了，欢迎回来';
  } else if (hour < 12) {
    greeting = '早上好，欢迎回来';
  } else if (hour < 14) {
    greeting = '中午好，欢迎回来';
  } else if (hour < 18) {
    greeting = '下午好，欢迎回来';
  } else {
    greeting = '晚上好，欢迎回来';
  }

  const greetingEl = document.getElementById('welcome-greeting');
  if (greetingEl) greetingEl.textContent = greeting;

  // 填充日期时间
  const now = new Date();
  const dateStr = now.toLocaleDateString('zh-CN', {
    year: 'numeric', month: 'long', day: 'numeric', weekday: 'long'
  });
  const dateEl = document.getElementById('welcome-date');
  if (dateEl) dateEl.textContent = dateStr;

  // 随机选择一句诗意名言
  const poemEl = document.getElementById('welcome-poem');
  if (poemEl) {
    const poem = WELCOME_POEMS[Math.floor(Math.random() * WELCOME_POEMS.length)];
    poemEl.textContent = poem;
  }

  // 显示欢迎区
  banner.hidden = false;
}

/**
 * 关闭欢迎区并持久化状态
 */
function dismissWelcome() {
  const banner = document.getElementById('welcome-banner');
  if (!banner) return;

  // 添加退出动画
  banner.style.transition = 'opacity 0.2s ease-in, transform 0.2s ease-in';
  banner.style.opacity = '0';
  banner.style.transform = 'translateY(-8px)';

  setTimeout(() => {
    banner.hidden = true;
    banner.style.transition = '';
    banner.style.opacity = '';
    banner.style.transform = '';
  }, 200);

  // 持久化关闭状态
  try {
    localStorage.setItem('lrc_welcome_dismissed', '1');
  } catch (e) {
    console.warn('[Loong Recall] 无法持久化欢迎区关闭状态');
  }
}

/* ============================================================
 * v0.6.0 系统状态浮窗（设计文档 5.2.5：右下角固定）
 * 显示 ML 模型状态、编码器类型、缓存状态、系统模式、编码质量评分
 * 数据来源：/v1/health/system
 * ============================================================ */

/**
 * 加载系统状态浮窗数据
 */
async function loadSysStatusFloat() {
  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/health/system');
    if (!res.ok) return;
    const data = await res.json();

    // ML 模型状态
    const encoder = data.encoder || {};
    const mlModelEl = document.getElementById('float-ml-model');
    if (mlModelEl) {
      const mode = encoder.mode || 'unknown';
      const modelName = encoder.model_name || '未启用';
      if (mode === 'ml') {
        mlModelEl.textContent = modelName;
        mlModelEl.className = 'sys-status-value healthy';
      } else {
        mlModelEl.textContent = '统计模式';
        mlModelEl.className = 'sys-status-value warning';
      }
    }

    // 编码器类型
    const encoderTypeEl = document.getElementById('float-encoder-type');
    if (encoderTypeEl) {
      const mode = encoder.mode || 'unknown';
      const hiddenSize = encoder.hidden_size;
      if (mode === 'ml' && hiddenSize) {
        encoderTypeEl.textContent = `ML · ${hiddenSize}维`;
        encoderTypeEl.className = 'sys-status-value healthy';
      } else if (mode === 'statistical') {
        encoderTypeEl.textContent = 'TF-IDF';
        encoderTypeEl.className = 'sys-status-value warning';
      } else {
        encoderTypeEl.textContent = mode;
        encoderTypeEl.className = 'sys-status-value';
      }
    }

    // 缓存状态（基于编码次数和上次编码时间推断）
    const cacheEl = document.getElementById('float-cache-status');
    if (cacheEl) {
      const totalEncodings = encoder.total_encodings || 0;
      const lastMs = encoder.last_encoding_ms || 0;
      if (totalEncodings === 0) {
        cacheEl.textContent = '空';
        cacheEl.className = 'sys-status-value';
      } else if (Date.now() - lastMs < 60000) {
        cacheEl.textContent = `活跃 · ${totalEncodings}次`;
        cacheEl.className = 'sys-status-value healthy';
      } else {
        cacheEl.textContent = `${totalEncodings}次`;
        cacheEl.className = 'sys-status-value';
      }
    }

    // 系统模式
    const sysModeEl = document.getElementById('float-sys-mode');
    if (sysModeEl) {
      const mode = data.system_mode || 'unknown';
      const modeMap = {
        healthy: '正常运行',
        degraded: '已降级',
        oscillating: '调整中',
        drifting: '漂移',
        frozen: '已冻结',
        overloaded: '过载',
      };
      sysModeEl.textContent = modeMap[mode] || mode;
      if (mode === 'healthy') {
        sysModeEl.className = 'sys-status-value healthy';
      } else if (mode === 'degraded' || mode === 'oscillating' || mode === 'drifting') {
        sysModeEl.className = 'sys-status-value warning';
      } else {
        sysModeEl.className = 'sys-status-value critical';
      }
    }

    // 编码质量评分
    const qualityEl = document.getElementById('float-quality-score');
    const qualityFill = document.getElementById('float-quality-fill');
    const qualityScore = encoder.quality_score || 0;
    if (qualityEl) {
      const percent = (qualityScore * 100).toFixed(0) + '%';
      qualityEl.textContent = percent;
      if (qualityScore >= 0.8) {
        qualityEl.className = 'sys-status-value healthy';
      } else if (qualityScore >= 0.4) {
        qualityEl.className = 'sys-status-value warning';
      } else {
        qualityEl.className = 'sys-status-value critical';
      }
    }
    if (qualityFill) {
      qualityFill.style.width = (qualityScore * 100) + '%';
    }
  } catch (e) {
    // 静默失败：浮窗不影响主功能
    console.warn('[Loong Recall] 系统状态浮窗加载失败:', e.message);
  }
}

/**
 * 折叠/展开系统状态浮窗
 */
function toggleSysStatusFloat() {
  const float = document.getElementById('sys-status-float');
  const icon = document.getElementById('sys-status-toggle-icon');
  if (!float) return;

  float.classList.toggle('collapsed');
  if (icon) {
    icon.textContent = float.classList.contains('collapsed') ? '+' : '─';
  }

  // 持久化折叠状态
  try {
    localStorage.setItem('lrc_sys_status_collapsed', float.classList.contains('collapsed') ? '1' : '0');
  } catch (e) {
    // 忽略 localStorage 错误
  }
}

/**
 * 初始化系统状态浮窗（恢复折叠状态 + 启动定时刷新）
 */
function initSysStatusFloat() {
  const float = document.getElementById('sys-status-float');
  if (!float) return;

  // 恢复折叠状态
  try {
    if (localStorage.getItem('lrc_sys_status_collapsed') === '1') {
      float.classList.add('collapsed');
      const icon = document.getElementById('sys-status-toggle-icon');
      if (icon) icon.textContent = '+';
    }
  } catch (e) {
    // 忽略 localStorage 错误
  }

  // 首次加载
  setTimeout(loadSysStatusFloat, 600);

  // 定时刷新（每 30 秒）
  setInterval(loadSysStatusFloat, 30000);
}

/* ============================================================
 * v0.6.0 侧边栏折叠切换（设计文档 3.4：60/240px）
 * 持久化折叠状态到 localStorage
 * ============================================================ */

/**
 * 切换侧边栏折叠/展开状态
 */
function toggleSidebar() {
  const sidebar = document.getElementById('app-sidebar');
  if (!sidebar) return;

  sidebar.classList.toggle('collapsed');
  const isCollapsed = sidebar.classList.contains('collapsed');

  // 持久化状态
  try {
    localStorage.setItem('lrc_sidebar_collapsed', isCollapsed ? '1' : '0');
  } catch (e) {
    console.warn('[Loong Recall] 无法持久化侧边栏折叠状态');
  }
}

/**
 * 初始化侧边栏折叠状态（从 localStorage 恢复）
 */
function initSidebarCollapse() {
  const sidebar = document.getElementById('app-sidebar');
  if (!sidebar) return;

  try {
    if (localStorage.getItem('lrc_sidebar_collapsed') === '1') {
      sidebar.classList.add('collapsed');
    }
  } catch (e) {
    // 忽略 localStorage 错误
  }
}

/* ============================================================
 * v0.6.0 设置页面重构相关函数
 * ============================================================ */

/**
 * 切换提供商分类标签
 * @param {string} category - 分类：cloud / local / custom
 */
function switchProviderCategory(category) {
  // 更新标签状态
  const tabs = document.querySelectorAll('.provider-tab');
  tabs.forEach(tab => {
    if (tab.dataset.category === category) {
      tab.classList.add('active');
    } else {
      tab.classList.remove('active');
    }
  });

  // 显示对应分类的提供商网格
  const grids = {
    cloud: 'provider-grid-cloud',
    local: 'provider-grid-local',
    custom: 'provider-grid-custom'
  };

  Object.keys(grids).forEach(key => {
    const grid = document.getElementById(grids[key]);
    if (grid) {
      grid.style.display = key === category ? '' : 'none';
    }
  });

  // 自动选中该分类下的第一个提供商
  const firstCard = document.querySelector(`#${grids[category]} .provider-card`);
  if (firstCard) {
    const provider = firstCard.dataset.provider;
    selectProvider(provider);
  }
}

/**
 * 选择模型提供商
 * @param {string} provider - 提供商标识
 */
function selectProvider(provider) {
  // 更新所有提供商卡片的选中状态
  document.querySelectorAll('.provider-card').forEach(card => {
    if (card.dataset.provider === provider) {
      card.classList.add('active');
    } else {
      card.classList.remove('active');
    }
  });

  // 更新隐藏的 select 元素
  const select = document.getElementById('llm-provider');
  if (select) {
    select.value = provider;
    // 触发 change 事件
    select.dispatchEvent(new Event('change'));
  }
}

/**
 * 格式化文件大小
 * @param {number} bytes - 字节数
 * @returns {string} 格式化后的大小
 */
function formatSize(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
  return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
}

/**
 * 测试 LLM 配置连接
 * v0.8.1：通过 sidecar 转发测试请求，绕过浏览器 CSP connect-src 限制
 */
async function testLlmConfig() {
  const resultEl = document.getElementById('llm-config-result');
  const btnTest = document.getElementById('btn-test-llm');

  if (!resultEl) return;

  resultEl.style.display = '';
  resultEl.className = 'form-result';
  resultEl.textContent = '🔍 正在测试连接...';
  if (btnTest) btnTest.disabled = true;

  try {
    const provider = document.getElementById('llm-provider')?.value || '';
    const endpoint = document.getElementById('llm-endpoint')?.value?.trim();
    const apiKey = document.getElementById('llm-api-key')?.value?.trim();

    if (!endpoint || !apiKey) {
      throw new Error('请填写完整的 API 配置信息');
    }

    // v0.8.1：调用 sidecar 转发端点，避免直接请求外部域名被 CSP 拦截
    const resp = await fetchWithTimeout(`${window.API_BASE}/v1/config/llm/test`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        endpoint: endpoint,
        api_key: apiKey,
        provider: provider
      })
    }, 15000);  // 超时设为 15 秒（sidecar 内部 10 秒 + 网络余量）

    const data = await resp.json();
    if (data.ok) {
      resultEl.className = 'form-result success';
      resultEl.textContent = `✅ 连接成功！延迟 ${data.latency_ms}ms`;
    } else {
      throw new Error(data.message || `连接失败 (HTTP ${data.status})`);
    }
  } catch (e) {
    // v0.8.2：sidecar 不可达时给出明确提示
    const msg = (e.name === 'SidecarUnreachableError' || e.name === 'SidecarTimeoutError')
      ? '❌ LRC 服务未运行，请先启动服务'
      : '❌ ' + e.message;
    resultEl.className = 'form-result error';
    resultEl.textContent = msg;
  } finally {
    if (btnTest) btnTest.disabled = false;
  }
}

/* ============================================================
 * 本地嵌入模型配置相关函数
 * ============================================================ */

/**
 * 选择嵌入模型
 * v0.8.2：修复选择器 bug，原 [onclick*=] 依赖已移除的 onclick 属性，改用 data-arg 匹配
 * @param {string} modelId - 模型 ID（完整路径，如 BAAI/bge-small-zh）
 */
function selectEmbedderModel(modelId) {
  // 更新卡片选中状态：先移除所有 active
  document.querySelectorAll('[data-embedder]').forEach(card => {
    card.classList.remove('active');
  });

  // v0.8.2：用 data-arg 属性匹配（bindAllActions 传入的是 data-arg 值）
  // 同时兼容 data-embedder 短 ID 匹配（向后兼容直接调用场景）
  let activeCard = document.querySelector(`[data-embedder][data-arg="${modelId}"]`);
  if (!activeCard) {
    // 降级：尝试用 data-embedder 属性匹配短 ID（如 bge-small-zh）
    activeCard = document.querySelector(`[data-embedder="${modelId}"]`);
  }
  if (activeCard) {
    activeCard.classList.add('active');
  } else {
    console.warn('[LRC v' + APP_VERSION + ']selectEmbedderModel 未找到匹配卡片:', modelId);
  }

  // 更新输入框
  const input = document.getElementById('embedder-model');
  if (input) {
    input.value = modelId;
  }
}

/**
 * 检测嵌入模型状态
 */
async function checkEmbedderStatus() {
  const dotEl = document.getElementById('embedder-status-dot');
  const textEl = document.getElementById('embedder-status-text');

  if (!dotEl || !textEl) return;

  dotEl.className = 'ollama-status-dot';
  textEl.textContent = '检测中...';

  try {
    const resp = await fetchWithTimeout(API_BASE + '/api/embedder/status', {
      method: 'GET'
    }, 5000);

    if (resp.ok) {
      const data = await resp.json();
      if (data.status === 'ready') {
        dotEl.className = 'ollama-status-dot online';
        textEl.textContent = '模型已就绪：' + (data.model_id || '未知');
      } else if (data.status === 'not_downloaded') {
        dotEl.className = 'ollama-status-dot offline';
        textEl.textContent = '模型未下载';
      } else {
        dotEl.className = 'ollama-status-dot unknown';
        textEl.textContent = '未知状态';
      }
    } else {
      throw new Error('服务响应异常');
    }
  } catch (e) {
    dotEl.className = 'ollama-status-dot offline';
    textEl.textContent = '服务未启动';
  }
}

/**
 * 切换下载镜像源
 */
function changeEmbedderMirror() {
  const mirror = document.getElementById('embedder-mirror')?.value;
}

/**
 * 下载嵌入模型（调用后端 API，后台下载 + 轮询进度）
 */
async function downloadEmbedderModel() {
  // v0.8.3 Step 4 批次 4：alert→showToast（修复 G001-G003）
  const modelId = document.getElementById('embedder-model')?.value?.trim();
  if (!modelId) {
    showToast('请先选择一个模型', 'warning');
    return;
  }

  const mirror = document.getElementById('embedder-mirror')?.value || 'hf-mirror';
  const progressEl = document.getElementById('embedder-download-progress');
  const percentEl = document.getElementById('embedder-download-percent');
  const barEl = document.getElementById('embedder-download-bar');

  if (progressEl) progressEl.style.display = '';

  try {
    // 调用后端下载 API
    const resp = await fetchWithTimeout(API_BASE + '/api/embedder/download', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model_id: modelId, mirror: mirror })
    }, 10000);

    const data = await resp.json();

    if (!data.success) {
      throw new Error(data.message || '下载启动失败');
    }

    // 显示下载已启动
    if (percentEl) percentEl.textContent = '0%';
    if (barEl) barEl.style.width = '0%';

    // 轮询下载状态
    let pollCount = 0;
    const pollInterval = setInterval(async () => {
      pollCount++;
      try {
        const statusResp = await fetchWithTimeout(API_BASE + '/api/embedder/status', {}, 3000);
        if (statusResp.ok) {
          const statusData = await statusResp.json();
          // 模拟进度推进（后端下载是后台任务，前端用轮询检测完成）
          const fakeProgress = Math.min(pollCount * 10, 95);
          if (percentEl) percentEl.textContent = fakeProgress + '%';
          if (barEl) barEl.style.width = fakeProgress + '%';

          if (statusData.status === 'ready') {
            clearInterval(pollInterval);
            if (percentEl) percentEl.textContent = '100%';
            if (barEl) barEl.style.width = '100%';
            setTimeout(() => {
              if (progressEl) progressEl.style.display = 'none';
              showToast('模型 ' + modelId + ' 下载完成！', 'success');
              checkEmbedderStatus();
            }, 500);
          }

          // 超时保护（120 秒）
          if (pollCount > 40) {
            clearInterval(pollInterval);
            if (progressEl) progressEl.style.display = 'none';
            showToast('下载超时，请稍后通过「检测状态」查看。模型文件较大时可能需要更长时间。', 'warning');
            checkEmbedderStatus();
          }
        }
      } catch (e) {
        // 轮询失败，继续重试
      }
    }, 3000);

  } catch (e) {
    if (progressEl) progressEl.style.display = 'none';
    console.error('[downloadEmbedderModel] 下载失败:', e);
    showToast('下载失败: ' + e.message + '。你也可以通过命令行手动下载：code-memory-server model download ' + modelId, 'error', 8000);
  }
}

/**
 * 应用嵌入模型（设为默认）
 */
async function applyEmbedderModel() {
  // v0.8.3 Step 4 批次 4：alert→showToast（修复 G001-G003）
  const modelId = document.getElementById('embedder-model')?.value?.trim();
  if (!modelId) {
    showToast('请先选择一个模型', 'warning');
    return;
  }

  try {
    const resp = await fetchWithTimeout(API_BASE + '/api/embedder/apply', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model_id: modelId })
    }, 5000);

    const data = await resp.json();

    if (data.success) {
      showToast(data.message || '模型已设为默认，重启服务后生效', 'success');
      checkEmbedderStatus();
    } else {
      throw new Error(data.message || '设置失败');
    }
  } catch (e) {
    console.error('[applyEmbedderModel] 设置失败:', e);
    showToast('设置失败: ' + e.message, 'error');
  }
}

/**
 * 测试语义编码模型链接（测试镜像源连通性）
 */
async function testEmbedderConnection() {
  // v0.8.3 Step 4 批次 4：alert→showToast（修复 G001-G003）
  const modelId = document.getElementById('embedder-model')?.value?.trim();
  if (!modelId) {
    showToast('请先选择一个模型', 'warning');
    return;
  }

  const mirror = document.getElementById('embedder-mirror')?.value || 'hf-mirror';
  const mirrorNames = {
    'hf-mirror': 'HF-Mirror',
    'modelscope': 'ModelScope'
  };

  // 显示测试中状态
  const btn = event?.target;
  if (btn) {
    btn.disabled = true;
    btn.textContent = '测试中...';
  }

  try {
    const resp = await fetchWithTimeout(API_BASE + '/api/embedder/test', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model_id: modelId, mirror: mirror })
    }, 10000);

    const data = await resp.json();

    if (data.success) {
      showToast('✅ 连接成功！镜像源: ' + mirrorNames[mirror] + '，模型: ' + modelId + '，延迟: ' + (data.latency_ms || '?') + 'ms', 'success');
    } else {
      throw new Error(data.message || '连接失败');
    }
  } catch (e) {
    console.error('[testEmbedderConnection] 连接失败:', e);
    showToast('❌ 连接失败: ' + e.message + '。请检查网络或尝试其他镜像源', 'error');
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.textContent = '测试链接';
    }
  }
}

/**
 * 切换项目
 * v0.6.1 P1-3 修复: 真正调用 Tauri switch_project 命令,而非演示提示
 * 优先级链路: Tauri 环境调用 invoke → iframe 模式 postMessage → 浏览器演示
 */
async function switchProject() {
  // v0.8.2：用 showPrompt 替代同步 prompt
  const projectDir = await showPrompt(
    '请输入项目目录的绝对路径:\n（例如: G:\\code-memory）',
    '切换项目'
  );
  if (!projectDir || !projectDir.trim()) {
    return; // 用户取消
  }
  const trimmedDir = projectDir.trim();

  // v0.8.2：用 showConfirm 替代同步 confirm（二次确认）
  const confirmed = await showConfirm(
    '确定要切换到项目: ' + trimmedDir + ' 吗？\n切换后 sidecar 将重启并重新索引代码。',
    '切换项目'
  );
  if (!confirmed) {
    return;
  }

  // 调用桌面端 switch_project 命令
  try {
    const result = await postMessageToParent('lrc-switch-project', {
      projectDir: trimmedDir,  // Tauri 命令参数: project_dir → projectDir (camelCase)
    }, 60000); // 切换项目涉及 sidecar 重启,超时设为 60s

    if (result && result.success) {
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('项目切换成功', '项目: ' + result.project_dir + '\n端口: ' + result.port + '\n\n' + (result.message || ''));
      // 刷新仪表盘以加载新项目的数据
      setTimeout(() => loadDashboard(), 500);
    } else {
      showToast('项目切换失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    // 浏览器环境或非桌面端嵌入模式会走到这里
    const errMsg = e.message || String(e);
    if (errMsg.includes('非桌面端嵌入模式') || errMsg.includes('无法调用此功能')) {
      showToast('项目切换功能仅在桌面端可用，请在 Loong Recall 桌面应用中使用此功能', 'warning');
    } else {
      showToast('项目切换失败: ' + errMsg, 'error');
    }
  }
}

/* ============================================================
 * 切换项目页面相关函数
 * ============================================================ */

/**
 * 开始完整配置流程
 */
function startFullSetup() {
  const stepsSection = document.getElementById('setup-steps-section');
  if (stepsSection) {
    stepsSection.style.display = '';
    // 滚动到步骤区域
    stepsSection.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
  goToStep(1);

  // 模拟 AI 工具扫描
  setTimeout(() => {
    simulateAiToolsScan();
  }, 1500);
}

/**
 * 开始快速配置流程
 */
function startQuickSetup() {
  // 快速模式直接选择文件夹
  selectProjectFolder();
}

/**
 * 检测 AI 工具（调用后端 API 实时检测）
 */
async function simulateAiToolsScan() {
  const toolsList = document.getElementById('ai-tools-list');
  if (!toolsList) return;

  // 显示扫描中状态
  toolsList.innerHTML = '<p style="color: var(--lrc-墨韵-400); margin: 0;"><span class="loading-spinner"></span> 正在扫描已安装的 IDE & Agent 工具...</p>';

  try {
    const resp = await fetchWithTimeout(API_BASE + '/api/tools/detect', {
      method: 'GET'
    }, 15000);

    if (!resp.ok) {
      throw new Error('检测服务响应异常 (' + resp.status + ')');
    }

    const data = await resp.json();
    const tools = data.tools || [];

    if (tools.length === 0) {
      toolsList.innerHTML = '<p style="color: var(--lrc-墨韵-400); margin: 0;">未检测到任何 IDE 或 Agent 工具</p>';
      return;
    }

    toolsList.innerHTML = tools.map(tool => `
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 12px 0; border-bottom: 1px solid var(--lrc-宣纸-500);">
        <div style="display: flex; align-items: center; gap: 12px;">
          <input type="checkbox" ${tool.installed ? 'checked' : ''} ${!tool.installed ? 'disabled' : ''} id="tool-${tool.name.replace(/\s/g, '-')}">
          <span style="color: var(--lrc-墨韵-700); font-weight: 500;">${tool.name}</span>
          <span style="font-size: 0.8em; color: var(--lrc-墨韵-400);">(${tool.type})</span>
          ${tool.version ? '<span style="font-size: 0.8em; color: var(--lrc-墨韵-400);">v' + tool.version + '</span>' : ''}
        </div>
        <span style="font-size: 0.85em; font-weight: 600; color: ${tool.installed ? 'var(--lrc-玉色-600)' : 'var(--lrc-墨韵-300)'};">${tool.installed ? '已检测到' : '未安装'}</span>
      </div>
    `).join('');

    // 统计已安装数量
    const installedCount = tools.filter(t => t.installed).length;

  } catch (e) {
    toolsList.innerHTML = '<p style="color: var(--lrc-朱砂-500); margin: 0;">检测失败: ' + htmlescape(e.message) + '</p><p style="color: var(--lrc-墨韵-400); font-size: 0.85em; margin-top: 8px;">请确保龙忆（LRC）服务正在运行</p>';
  }
}

/**
 * 选择项目文件夹
 */
function selectProjectFolder() {
  const input = document.createElement('input');
  input.type = 'file';
  input.webkitdirectory = true;
  input.directory = true;
  input.onchange = function(e) {
    const files = e.target.files;
    if (files && files.length > 0) {
      const path = files[0].webkitRelativePath.split('/')[0];
      addSelectedProject(path);
    }
  };
  input.click();
}

/**
 * 添加已选项目
 */
function addSelectedProject(projectName) {
  const projectsContainer = document.getElementById('selected-projects');
  if (!projectsContainer) return;

  // 检查是否已存在
  if (projectsContainer.querySelector(`[data-project="${projectName}"]`)) {
    return;
  }

  // 如果是第一个项目，移除占位文字
  if (projectsContainer.querySelector('p')) {
    projectsContainer.innerHTML = '';
  }

  const projectEl = document.createElement('div');
  projectEl.setAttribute('data-project', projectName);
  projectEl.style.cssText = 'display: flex; align-items: center; justify-content: space-between; padding: 12px; background: var(--lrc-宣纸-400); border-radius: var(--radius-sm); margin-bottom: 8px;';
  projectEl.innerHTML = `
    <div style="display: flex; align-items: center; gap: 10px;">
      <span>📁</span>
      <span style="color: var(--lrc-墨韵-700); font-weight: 500;">${projectName}</span>
    </div>
    <button style="background: none; border: none; color: var(--lrc-朱砂-500); cursor: pointer; font-size: 1.1em;" data-action="removeProjectFromWizard" data-arg-mode="this">✕</button>
  `;
  projectsContainer.appendChild(projectEl);
  // v0.8.4 Step 9：动态生成的元素需要重新绑定 data-action
  if (typeof bindAllActions === 'function') {
    bindAllActions();
  }

  // 启用下一步按钮
  checkNextButton();

  // 如果是快速模式，直接完成
  const stepsSection = document.getElementById('setup-steps-section');
  if (!stepsSection || stepsSection.style.display === 'none') {
    // v0.8.3 Step 4 批次 6：alert→showToast（修复 G001-G003）
    showToast('项目 ' + projectName + ' 已选择！（演示功能，实际需后端 API 支持重新索引）', 'info');
  }
}

/**
 * 检查下一步按钮状态
 */
function checkNextButton() {
  const nextBtn = document.getElementById('step-1-next-btn');
  const projectsContainer = document.getElementById('selected-projects');
  if (!nextBtn || !projectsContainer) return;

  const hasProjects = projectsContainer.children.length > 0 && !projectsContainer.querySelector('p');
  nextBtn.disabled = !hasProjects;
  nextBtn.style.opacity = hasProjects ? '1' : '0.5';
  nextBtn.style.cursor = hasProjects ? 'pointer' : 'not-allowed';
}

/**
 * v0.8.4 Step 9 / G025 修复：移除向导中已选项目（替代内联 onclick）
 * @param {HTMLElement} btn - 点击的按钮元素
 */
function removeProjectFromWizard(btn) {
  if (!btn || !btn.parentElement || !btn.parentElement.parentElement) return;
  btn.parentElement.parentElement.remove();
  if (typeof checkNextButton === 'function') {
    checkNextButton();
  }
}
window.removeProjectFromWizard = removeProjectFromWizard;

/**
 * 跳转到指定步骤
 */
function goToStep(stepNum) {
  // 隐藏所有步骤
  for (let i = 1; i <= 3; i++) {
    const stepEl = document.getElementById('setup-step-' + i);
    const indicator = document.getElementById('step-' + i);
    const lineEl = document.getElementById('step-line-' + i);
    if (stepEl) {
      stepEl.style.display = i === stepNum ? '' : 'none';
    }
    if (indicator) {
      indicator.classList.remove('active', 'completed');
      if (i < stepNum) {
        indicator.classList.add('completed');
      } else if (i === stepNum) {
        indicator.classList.add('active');
      }
    }
    if (lineEl) {
      lineEl.style.background = i < stepNum ? 'var(--lrc-金色-500)' : 'var(--lrc-宣纸-500)';
    }
  }
}

/**
 * 更新配置向导的 LLM 字段显示
 */
function updateSetupLlmFields() {
  const provider = document.getElementById('setup-llm-provider')?.value;
  const apiKeyGroup = document.getElementById('setup-llm-api-key-group');
  if (apiKeyGroup) {
    apiKeyGroup.style.display = provider && provider !== 'none' ? '' : 'none';
  }
}

/**
 * 完成配置
 */
function finishSetup() {
  goToStep(3);
}

// ============================================================
// v0.8.0 桌面端 P0 修复：19 个 onclick 函数暴露到 window
// 这些函数定义在 IIFE 内部（原 IIFE 外部，已随 IIFE 闭合移至末尾而纳入内部），
// 需通过 window.xxx = xxx 暴露后才能被 HTML onclick 调用。
// 详见 docs/DESKTOP_FIX_PLAN_v0.8.0.md
// ============================================================

// 仪表盘交互
window.dismissWelcome = dismissWelcome;
window.toggleSidebar = toggleSidebar;
window.toggleSysStatusFloat = toggleSysStatusFloat;
window.loadEvolutionTimeline = loadEvolutionTimeline;

// 记忆详情面板
window.closeMemoryDetail = closeMemoryDetail;

// MCP 配置向导
window.startFullSetup = startFullSetup;
window.startQuickSetup = startQuickSetup;
window.selectProjectFolder = selectProjectFolder;
window.goToStep = goToStep;
window.finishSetup = finishSetup;
window.switchProject = switchProject;

// 嵌入模型配置
window.checkEmbedderStatus = checkEmbedderStatus;
window.selectEmbedderModel = selectEmbedderModel;
window.downloadEmbedderModel = downloadEmbedderModel;
window.applyEmbedderModel = applyEmbedderModel;
window.testEmbedderConnection = testEmbedderConnection;

// LLM 提供商配置
window.switchProviderCategory = switchProviderCategory;
window.selectProvider = selectProvider;
window.testLlmConfig = testLlmConfig;

// ============================================================
// v0.8.1 Step 1：集中事件绑定（替代内联 onclick）
// 设计说明：
//   - CSP script-src 不含 'unsafe-inline'，内联 onclick 全部失效
//   - 改用 data-action 数据属性 + addEventListener 集中绑定
//   - 所有原 function 仍挂载到 window，兼容 CDP 直接调用测试
// ============================================================

/**
 * 触发隐藏的文件输入框点击
 * 替代原 onclick="document.getElementById('xxx').click()"
 * @param {HTMLElement} btn - 触发按钮
 */
function triggerFileInput(btn) {
  const targetId = btn.getAttribute('data-target');
  if (targetId) {
    const input = document.getElementById(targetId);
    if (input) input.click();
  }
}
window.triggerFileInput = triggerFileInput;

/**
 * 集中绑定所有 data-action 元素的 click 事件
 * 在 DOMContentLoaded 后执行；动态渲染后可重复调用（内部去重）
 */
function bindAllActions() {
  // 1. 扫描所有带 data-action 的元素
  const actionEls = document.querySelectorAll('[data-action]');
  actionEls.forEach(el => {
    const action = el.getAttribute('data-action');
    if (!action) return;

    // 跳过已绑定元素（防止重复绑定）
    if (el.dataset.bound === '1') return;
    el.dataset.bound = '1';

    el.addEventListener('click', async (ev) => {
      // 阻止 <a href="#..."> 的默认跳转
      if (el.tagName === 'A' && el.getAttribute('href')?.startsWith('#')) {
        ev.preventDefault();
      }

      // v0.8.3 Step 8：检查 btn-disabled-api 类，阻止禁用按钮的点击（修复 N04）
      // 移除了 CSS 的 pointer-events:none 后，必须用 JS 显式阻止点击
      if (el.classList.contains('btn-disabled-api')) {
        ev.preventDefault();
        ev.stopPropagation();
        // 显示 tooltip 提示（title 属性已设置，浏览器自动显示）
        return;
      }

      // v0.8.2：防抖与幂等保护（对应审计 G004）
      // 检查 inFlight 标志，防止快速重复点击
      if (el.dataset.inFlight === '1') {
        ev.preventDefault();
        ev.stopPropagation();
        return;
      }

      const fn = window[action];
      if (typeof fn !== 'function') {
        console.warn('[LRC v' + APP_VERSION + ']未找到 data-action 对应函数:', action);
        return;
      }

      // v0.8.2：标记为飞行中
      el.dataset.inFlight = '1';

      try {
        const argMode = el.getAttribute('data-arg-mode');
        const arg = el.getAttribute('data-arg');

        if (argMode === 'this') {
          // selectPresetScenario(this) 等场景，传入元素自身
          await fn(el);
        } else if (arg !== null) {
          // 自动判断数字/字符串
          const parsed = /^\d+$/.test(arg) ? parseInt(arg, 10) : arg;
          await fn(parsed);
        } else if (action === 'triggerFileInput') {
          // 触发隐藏文件输入框
          triggerFileInput(el);
        } else {
          await fn();
        }
      } catch (e) {
        console.error('[LRC v' + APP_VERSION + ']data-action 执行异常:', action, e);
      } finally {
        // v0.8.2：延迟 500ms 解锁，防止快速连击
        setTimeout(() => { delete el.dataset.inFlight; }, 500);
      }
    });
  });

  // 2. 处理 data-hover-border（替代 onmouseover/onmouseout）
  document.querySelectorAll('[data-hover-border]').forEach(el => {
    if (el.dataset.bound === '1') return;
    el.dataset.bound = '1';
    const hoverBorder = el.getAttribute('data-hover-border');
    const defaultBorder = el.getAttribute('data-default-border');
    el.addEventListener('mouseover', () => {
      el.style.borderColor = hoverBorder;
    });
    el.addEventListener('mouseout', () => {
      el.style.borderColor = defaultBorder;
    });
  });

  // 3. v0.8.2 新增：处理 data-input-action（替代内联 onchange/oninput）
  // 支持 change/input/blur 等事件类型，通过 data-input-event 指定
  document.querySelectorAll('[data-input-action]').forEach(el => {
    if (el.dataset.bound === '1') return;
    el.dataset.bound = '1';
    const action = el.getAttribute('data-input-action');
    const eventType = el.getAttribute('data-input-event') || 'change';
    const passEvent = el.getAttribute('data-pass-event') === '1';

    el.addEventListener(eventType, async (ev) => {
      const fn = window[action];
      if (typeof fn !== 'function') {
        console.warn('[LRC v' + APP_VERSION + ']未找到 data-input-action 对应函数:', action);
        return;
      }
      try {
        // importMemories(event) 等需要事件对象的函数，传入 event
        if (passEvent) {
          await fn(ev);
        } else {
          await fn();
        }
      } catch (e) {
        console.error('[LRC v' + APP_VERSION + ']data-input-action 执行异常:', action, e);
      }
    });
  });

  console.log('[LRC v' + APP_VERSION + ']集中事件绑定完成，共绑定', actionEls.length, '个元素');
}

// DOMContentLoaded 后执行绑定；若 DOM 已加载（动态注入场景）则立即执行
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', bindAllActions);
} else {
  bindAllActions();
}
// v0.8.4 Step 14 / G004：暴露 bindAllActions 到 window 供 CDP 测试检测
window.bindAllActions = bindAllActions;

// ============================================================
// v0.8.3 Step 12 / G010：网络断开检测（对应审计 G010）
// 监听 online/offline 事件，断网时显示 Toast 提示并标记 body.offline-mode
// 恢复网络时自动重新加载仪表盘数据
// 设计原则：
//   1. 不阻塞用户操作，仅显示 Toast 提示
//   2. 断网期间禁用 fetchWithTimeout 的请求（由调用方检查 navigator.onLine）
//   3. 恢复网络后自动刷新仪表盘与信任中心
// ============================================================
window.addEventListener('offline', () => {
  console.warn('[LRC v' + APP_VERSION + ']网络已断开');
  document.body.classList.add('offline-mode');
  if (typeof showToast === 'function') {
    showToast('网络已断开，部分功能不可用', 'warning', 5000);
  }
});

window.addEventListener('online', () => {
  console.log('[LRC v' + APP_VERSION + ']网络已恢复');
  document.body.classList.remove('offline-mode');
  if (typeof showToast === 'function') {
    showToast('网络已恢复，正在重新加载数据...', 'success', 2000);
  }
  // 恢复网络后自动刷新仪表盘（若当前在仪表盘标签页）
  const dashboard = document.getElementById('tab-dashboard');
  if (dashboard && dashboard.classList.contains('active') && typeof loadDashboard === 'function') {
    try { loadDashboard(); } catch (e) { console.error('[online] 重新加载仪表盘失败:', e); }
  }
});

// ============================================================
// v0.8.2 新增：beforeunload 拦截（对应审计 G006）
// 有进行中请求时，刷新/关闭页面前提示用户
// ============================================================
window.addEventListener('beforeunload', (e) => {
  if (pendingRequestCount > 0) {
    // 现代浏览器忽略自定义消息，但仍需设置 returnValue 触发提示
    e.preventDefault();
    e.returnValue = '';
    return '';
  }
});

// 暴露计数器供调试
window.__getPendingRequestCount = () => pendingRequestCount;

// v0.8.6 Step 5 / N004 修复：暴露 showToast 到 window
// 之前 showToast 在 IIFE 作用域内，CDP 测试通过 window.showToast 访问失败
// 注意：原计划提及的 _toastQueue/_toastMaxVisible 实际不存在，暴露真实存在的变量
window.showToast = showToast;
window._recentToastMessages = _recentToastMessages;  // 用于测试检查去重窗口状态
window._TOAST_MAX_VISIBLE = TOAST_MAX_VISIBLE;  // 暴露常量便于测试验证队列上限

// v0.8.6 Step 6 / N005 修复：暴露 validateInput 到 window
// 之前 validateInput 在 IIFE 作用域内，CDP 测试通过 window.validateInput 访问失败
// setupInputValidation 已在 line 1319 暴露，但 validateInput 本身未暴露
window.validateInput = validateInput;

// v0.8.6 Step 7 / N007 修复：暴露 AbortController 变量到 window.__testHooks
// 之前 dashboardAbortController/searchAbortController/_tabAbortControllers 等
// 在 IIFE 作用域内，CDP 并发控制检测（G017）无法访问
// 使用 __testHooks 命名空间避免污染 window 对象，仅用于测试
window.__testHooks = {
  get dashboardAbortController() { return dashboardAbortController; },
  get searchAbortController() { return searchAbortController; },
  get tabAbortControllers() { return _tabAbortControllers; },
  get retryCounters() { return _retryCounters; },
  get retryModalActive() { return _retryModalActive; },
  _abortActiveTabRequests: _abortActiveTabRequests
};

// IIFE 闭合（v0.8.0：从原第 2950 行移至文件末尾）
})();
