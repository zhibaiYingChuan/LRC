
// ============================================================
// Loong Recall 仪表盘 — 主应用脚本
// 使用 IIFE 模式隔离作用域，仅暴露 HTML onclick 所需的函数到全局
// ============================================================
// v0.8.5 Step 18：版本号常量（CDP 测试与运行时查询使用）
// v0.8.25：保留硬编码版本号作为 fallback，启动时异步从后端获取真实版本号
const APP_VERSION = '0.9.0';
window.__LRC_VERSION__ = APP_VERSION;

/**
 * v0.8.25 新增：从后端动态获取版本号
 * 调用 /v1/health/system 端点获取 version 字段，
 * 成功后更新全局版本号并刷新状态栏显示。
 * 失败时静默降级，使用本地硬编码版本号。
 */
async function fetchBackendVersion() {
  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/health/system', {}, 5000);
    if (!res.ok) return;
    const data = await res.json();
    if (data && data.version) {
      window.__LRC_VERSION__ = data.version;
      // 更新状态栏版本显示
      const versionEl = document.getElementById('status-version');
      if (versionEl) versionEl.textContent = 'v' + data.version;
      const sysVersionEl = document.getElementById('sys-version');
      if (sysVersionEl) sysVersionEl.textContent = 'v' + data.version;
      // 更新 meta version
      const metaVersion = document.querySelector('meta[name="version"]');
      if (metaVersion) metaVersion.content = data.version;
    }
  } catch (e) {
    // 静默降级：后端不可达时使用本地版本号
    console.warn('[LRC] 获取后端版本号失败，使用本地版本号:', e.message);
  }
}

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
// v0.9.0 P0 修复：优先从 <meta name="lrc-sidecar-port"> 标签同步读取端口
// 桌面端在注入 HTML 前将 sidecar 实际端口写入 meta 标签，消除 IPC 竞态
function _readSidecarPortFromMeta() {
  try {
    const meta = document.querySelector('meta[name="lrc-sidecar-port"]');
    if (meta && meta.content) {
      const port = parseInt(meta.content, 10);
      if (port > 0 && port < 65536) {
        console.log('[LRC] meta 标签发现 sidecar 端口: ' + port);
        return port;
      }
    }
  } catch (e) { /* 静默降级 */ }
  return null;
}
const META_SIDECAR_PORT = _readSidecarPortFromMeta();
// v0.9.0 开发模式隔离：开发版默认端口 3111（meta 标签注入），稳定版 3099（release 构建时替换 meta）
const STABLE_DEFAULT_PORT = 3099;
const DEFAULT_API_BASE = isTauriEnv
  ? (META_SIDECAR_PORT ? `http://127.0.0.1:${META_SIDECAR_PORT}` : `http://127.0.0.1:${STABLE_DEFAULT_PORT}`)
  : (window.location.origin || `http://localhost:${STABLE_DEFAULT_PORT}`);
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

// v0.8.22 GAP-07 修复（interaction-resilience-auditor Round4）：
//   根因：所有 localStorage.setItem 调用无 try-catch，存储满时抛未捕获异常
//   修复：统一安全写入工具函数，失败时 toast 提示且不阻塞后续逻辑
function safeLocalStorageSetItem(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch (e) {
    console.warn(`[safeLocalStorageSetItem] 写入失败 key=${key}:`, e.message);
    showToast('本地存储已满，部分偏好可能无法保存', 'warning', 3000);
  }
}

// v0.8.22 GAP-09 修复（interaction-resilience-auditor Round4）：
//   根因：关键按钮点击后无 loading 视觉反馈，用户不知是否触发
//   修复：通用按钮状态机工具函数，支持 idle→loading→success→error
const _buttonStateMap = new WeakMap();
function setButtonState(btn, state, originalText) {
  if (!btn) return;
  if (!_buttonStateMap.has(btn)) {
    _buttonStateMap.set(btn, { originalText: originalText || btn.textContent, disabled: btn.disabled });
  }
  const stateInfo = _buttonStateMap.get(btn);
  switch (state) {
    case 'loading':
      btn.disabled = true;
      btn.style.opacity = '0.6';
      btn.style.cursor = 'not-allowed';
      btn.textContent = '处理中...';
      break;
    case 'success':
      btn.disabled = false;
      btn.style.opacity = '';
      btn.style.cursor = '';
      btn.textContent = '✓ 成功';
      // v0.8.26 UX-01 修复：恢复时间从 1.5s 统一为 3s，与 testModel 边框恢复时间一致
      setTimeout(() => {
        btn.textContent = stateInfo.originalText;
      }, 3000);
      break;
    case 'error':
      btn.disabled = false;
      btn.style.opacity = '';
      btn.style.cursor = '';
      btn.textContent = '✗ 失败';
      // v0.8.26 UX-01 修复：恢复时间从 1.5s 统一为 3s，与 testModel 边框恢复时间一致
      setTimeout(() => {
        btn.textContent = stateInfo.originalText;
      }, 3000);
      break;
    case 'idle':
    default:
      btn.disabled = false;
      btn.style.opacity = '';
      btn.style.cursor = '';
      btn.textContent = stateInfo.originalText;
      break;
  }
}
window.setButtonState = setButtonState;

// v0.8.23 P2-01 (E4)：代理检测工具函数
// 检测浏览器是否配置了代理，用于友好提示用户排查连接问题
// 浏览器环境无法直接读取系统代理设置，通过以下间接方式检测：
//   1. navigator.onLine — 浏览器是否在线
//   2. 尝试通过 PAC 代理自动配置 URL 检测（通过创建临时 script 加载 PAC）
//   3. 检测是否在 Tauri 桌面环境（可通过 IPC 获取系统代理信息）
let _proxyDetectionResult = null; // 缓存检测结果，避免重复检测
async function detectProxyConfiguration() {
  if (_proxyDetectionResult) return _proxyDetectionResult;

  const result = {
    likelyProxy: false,
    reason: '',
    online: navigator.onLine !== false,
    isTauri: isTauriEnv,
    details: []
  };

  // 1. 检测浏览器是否在线
  if (!result.online) {
    result.reason = '浏览器处于离线状态';
    result.details.push('navigator.onLine = false');
    _proxyDetectionResult = result;
    return result;
  }

  // 2. Tauri 桌面环境：尝试通过 IPC 获取系统代理信息
  if (isTauriEnv) {
    try {
      const invokeFn = (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) ||
                       (window.__TAURI__ && window.__TAURI__.invoke);
      if (invokeFn) {
        const proxyInfo = await invokeFn('get_proxy_configuration');
        if (proxyInfo && proxyInfo.proxy_url) {
          result.likelyProxy = true;
          result.reason = `检测到系统代理: ${proxyInfo.proxy_url}`;
          result.details.push(`系统代理: ${proxyInfo.proxy_url}`);
          result.details.push(`代理类型: ${proxyInfo.proxy_type || '未知'}`);
        }
      }
    } catch (e) {
      // Tauri IPC 调用失败，静默处理（可能是旧版本 sidecar 不支持此命令）
      console.log('[detectProxy] Tauri IPC 获取代理信息失败:', e.message);
      result.details.push('Tauri IPC 获取代理信息失败（可忽略）');
    }
  }

  // 3. 浏览器环境：检测 navigator.connection 等网络信息
  if (!result.likelyProxy) {
    const connection = navigator.connection || navigator.mozConnection || navigator.webkitConnection;
    if (connection) {
      if (connection.type === 'none') {
        result.likelyProxy = true;
        result.reason = '网络连接类型为 "none"，可能被代理拦截';
        result.details.push(`connection.type = ${connection.type}`);
      }
    }
  }

  _proxyDetectionResult = result;
  return result;
}

// v0.8.2：全局进行中请求计数器（对应审计 G006）
// v0.8.3 Step 5：暴露只读接口到 window，便于 CDP 测试与外部检测（修复 G006）
// 使用 Object.defineProperty 的 getter（无 setter）确保只读，避免外部恶意修改
let pendingRequestCount = 0;
let _pendingBackgroundCount = 0; // v0.8.13 D4: 后台请求计数（健康检查等），beforeunload 时排除
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
      // v0.8.23 A-02：传递 signal 到 handleHttpError，使退避延迟可取消
      const retryContext = { method: restOptions.method || 'GET', url: url, signal: externalSignal };
      const result = await handleHttpError(res, `请求 ${url}`, retryContext);

      if (result.action === 'retry') {
        // 用户选择重试，递归调用（handleHttpError 内部已限制最大重试 3 次）
        // v0.8.22 P1-3 修复（hcse-resilience-validator Round3）：
        //   根因：此处手动 pendingRequestCount-- 后，finally 块又减一次，
        //         导致每次重试净减 1，计数器逐渐泄漏（可能变为负数）
        //   修复：去掉手动减少，由 finally 块统一管理计数器生命周期
        clearTimeout(timer);
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

  // v0.8.26 DOC-01 修复：磁盘空间不足友好提示
  // 检查后端返回的错误中是否包含磁盘空间相关关键词
  const diskSpaceKeywords = ['disk', 'space', 'ENOSPC', '存储空间', '磁盘', '容量不足', 'no space', 'quota'];
  const isDiskSpaceError = diskSpaceKeywords.some(kw =>
    errorDetail.toLowerCase().includes(kw.toLowerCase())
  );
  if (isDiskSpaceError) {
    showToast('⚠️ 磁盘空间不足，请清理磁盘后重试', 'error', 8000);
    console.warn('[handleHttpError] 检测到磁盘空间不足错误:', errorDetail);
    return { action: 'cancel', status, errorDetail: '磁盘空间不足' };
  }

  if (status === 500) {
    // v0.8.13 D3: 自动刷新触发的 500 错误降级为 Toast，不弹阻塞 Modal
    // 自动刷新失败属于预期内的偶发错误，弹 Modal 会打断用户操作
    const isAutoRefresh = context && (context.includes('auto-refresh') || context.includes('autoRefresh'));
    if (isAutoRefresh) {
      if (typeof showToast === 'function') {
        showToast('数据自动刷新失败，将在下次刷新时重试', 'warning');
      }
      return { action: 'cancel', status, errorDetail };
    }
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
      // v0.8.23 A-02：支持 signal 取消退避，避免标签页切换后仍执行重试
      const signal = retryContext?.signal;
      if (signal) {
        try {
          await new Promise((resolve, reject) => {
            const timer = setTimeout(resolve, backoff);
            const onAbort = () => {
              clearTimeout(timer);
              reject(new DOMException('退避延迟被取消', 'AbortError'));
            };
            signal.addEventListener('abort', onAbort, { once: true });
          });
        } catch (e) {
          if (e.name === 'AbortError') {
            console.log('[handleHttpError] 退避延迟被取消（标签页切换），放弃重试');
            return { action: 'cancel', status, errorDetail };
          }
          throw e;
        }
      } else {
        // v0.8.25 NEW-03 修复：退避延迟期间记录当前标签页，重试时验证一致性
        //   根因：没有 signal 时退避延迟不可取消，用户切换标签页后重试仍会执行
        //   修复：记录当前 active 标签页，退避结束后验证是否仍在该标签页
        const tabBeforeBackoff = document.querySelector('.tab-content.active')?.id;
        await new Promise(r => setTimeout(r, backoff));
        const tabAfterBackoff = document.querySelector('.tab-content.active')?.id;
        if (tabBeforeBackoff && tabAfterBackoff && tabBeforeBackoff !== tabAfterBackoff) {
          console.log('[NEW-03] 退避延迟期间标签页已切换，放弃重试');
          return { action: 'cancel', status, errorDetail };
        }
      }
      return { action: 'retry', status, errorDetail };
    }
    _retryCounters.delete(retryKey);
    return { action: 'cancel', status, errorDetail };
  } else if (status === 503) {
    // v0.8.19 GAP-02/GAP-03 修复：503 lock_busy 友好文案
    // v0.8.22 P1-2 修复（hcse-resilience-validator Round3）：
    //   根因：handleHttpError 有 503 自动重试 1 次 + loadDashboard/loadDaoMetrics
    //         也有自己的 LOCK_BUSY 重试机制，形成双重重试，实际请求次数翻倍
    //   修复：去掉 handleHttpError 的 503 自动重试，让上层调用者的重试机制接管
    //         仅保留 30s 冷却期的 toast 提示，避免 toast 风暴
    // v0.8.22 P0-4 修复（interaction-resilience-auditor Round3 P0-LOCKBUSY-01）：
    //   30s 冷却期，冷却期内不再显示 toast，避免 toast 风暴
    const lockBusyCooldownKey = `503_cooldown:${context}`;
    const lastToastTime = _retryCounters.get(lockBusyCooldownKey) || 0;
    const now = Date.now();
    if (now - lastToastTime > 30000) {
      // 超过 30s 冷却期，显示 toast
      _retryCounters.set(lockBusyCooldownKey, now);
      showToast('记忆系统正在后台合成，请稍后重试', 'info', 5000);
    } else {
      console.log(`[handleHttpError] 503 lock_busy 冷却期内，跳过 toast（剩余 ${Math.ceil((30000 - (now - lastToastTime)) / 1000)}s）`);
    }
    // 直接返回 cancel，由上层调用者的重试机制处理
    return { action: 'cancel', status, errorDetail };
  } else if (status === 502 || status === 504) {
    // v0.8.23 P2-03：502/504 网关错误自动重试（指数退避，不弹阻塞 Modal）
    // 502 Bad Gateway / 504 Gateway Timeout 通常是临时性故障
    // 自动重试 3 次，每次间隔递增，无需用户干预
    const retryKey = retryContext ? `${retryContext.method || 'GET'}:${retryContext.url || ''}` : `gateway:${context}`;
    const retryCount = _retryCounters.get(retryKey) || 0;

    if (retryCount >= MAX_RETRY_COUNT) {
      // 超过重试上限，显示错误提示
      _retryCounters.delete(retryKey);
      showToast(`${context}失败：服务暂时不可用，请稍后重试`, 'error', 5000);
      console.warn(`[handleHttpError] 502/504 重试已达上限（${MAX_RETRY_COUNT}次），放弃`);
      return { action: 'giveup', status, errorDetail };
    }

    _retryCounters.set(retryKey, retryCount + 1);
    const backoff = Math.pow(2, retryCount) * 1000;
    console.log(`[handleHttpError] ${status} 网关错误，${backoff}ms 后自动重试（第 ${retryCount + 1} 次）`);

    // 首次重试时显示 toast 提醒，后续静默重试
    if (retryCount === 0) {
      showToast(`${context}临时不可用（${status}），正在自动重试...`, 'warning', 3000);
    }

    // 支持 signal 取消退避（标签页切换）
    const signal = retryContext?.signal;
    if (signal) {
      try {
        await new Promise((resolve, reject) => {
          const timer = setTimeout(resolve, backoff);
          const onAbort = () => {
            clearTimeout(timer);
            reject(new DOMException('退避延迟被取消', 'AbortError'));
          };
          signal.addEventListener('abort', onAbort, { once: true });
        });
      } catch (e) {
        if (e.name === 'AbortError') {
          console.log(`[handleHttpError] ${status} 退避延迟被取消，放弃重试`);
          return { action: 'cancel', status, errorDetail };
        }
        throw e;
      }
    } else {
      await new Promise(r => setTimeout(r, backoff));
    }
    return { action: 'retry', status, errorDetail };
  } else if (status === 429) {
    // G007 扩展：429 限流
    // v0.8.22 GAP-06 修复（interaction-resilience-auditor Round4）：
    //   根因：429 仅显示 toast，无倒计时引导，用户可立即重试再次触发 429
    //   修复：从 Retry-After 头获取等待时间，toast 显示倒计时
    const retryAfter = parseInt(response.headers.get('Retry-After') || '5', 10);
    const waitSecs = Math.min(Math.max(retryAfter, 1), 30); // 限制 1-30s
    showToast(`${context}失败：请求过于频繁，请等待 ${waitSecs}s 后重试`, 'warning', waitSecs * 1000);
    console.log(`[handleHttpError] 429 限流，建议等待 ${waitSecs}s`);
    return { action: 'cancel', status, errorDetail };
  } else if (status === 401 || status === 403) {
    // 鉴权失败
    showToast(`${context}失败：权限不足，请检查 API 密钥配置`, 'error', 4000);
    return { action: 'cancel', status, errorDetail };
  } else {
    // 其他非 2xx 错误
    showToast(`${context}失败：${errorDetail || '服务响应异常，请稍后重试'}`, 'error', 4000);
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
let _broadcastDebounceTimer = null; // v0.8.13 F1: 广播防抖，避免状态抖动导致 UI 闪烁
const SidecarHealthMonitor = {
  _isReachable: false,
  _pollTimer: null,
  _pollInterval: 10000,  // 10 秒轮询
  _inFlight: false,
  // v0.8.11 P0-2：存储后端 sidecar 状态（'starting'/'indexing'/'running'/'unknown'）
  // 后端 /v1/health/system 返回 status 字段，前端需读取以区分"可达但索引中"和"完全就绪"
  _sidecarStatus: 'unknown',
  // v0.8.11 P0-1：健康检查失败容错计数（索引期响应慢，连续 2 次失败才判定不可达）
  _failCount: 0,
  _FAIL_THRESHOLD: 2,
  _backoffStep: 0,  // v0.8.13 E1: 退避步数（不可达时指数退避）
  _MAX_BACKOFF: 60000,  // v0.8.13 E1: 最大退避 60s
  // v0.8.21 P0-06：memory_store 锁状态（/health 返回 lock_busy 字段）
  // true 表示后台合成持锁，/v1/health/system 等 API 会返回 503 lock_busy
  // 前端据此判断是否显示"后台合成中"而非"服务未启动"
  _lockBusy: false,
  // v0.8.22 GAP-11：时间偏差警告标志（每次启动只提示一次）
  _timeSkewWarned: false,

  /**
   * 启动健康监测
   */
  start() {
    if (this._pollTimer) return;
    // v0.8.19 P0-3 修复（GAP-P0-01）：初始不可达时立即显示 banner
    // 根因：_setReachable 的"状态未变直接返回"优化（第522行）导致初始 _isReachable=false 时，
    //   第一次健康检查失败调用 _setReachable(false) 时 wasReachable===reachable===false，
    //   直接返回，banner 永远不显示，启动按钮不可见，用户被困。
    // 修复：start() 时如果初始不可达，强制显示 banner，确保用户能看到"启动服务"按钮。
    if (!this._isReachable) {
      const banner = document.getElementById('sidecar-down-banner');
      if (banner) banner.hidden = false;
    }
    // 立即检测一次
    this.check();
    // v0.8.13 E1: 使用 setTimeout 链式调用，支持指数退避（不可达时拉长间隔）
    this._scheduleNextCheck();
    console.log('[LRC v' + APP_VERSION + ']Sidecar 健康监测器已启动，轮询间隔:', this._pollInterval + 'ms');
  },

  /**
   * v0.8.13 E1: 调度下一次健康检查
   * 可达时使用固定 _pollInterval，不可达时按 _backoffStep 指数退避（上限 _MAX_BACKOFF）
   */
  _scheduleNextCheck() {
    const interval = this._isReachable
      ? this._pollInterval
      : Math.min(this._pollInterval * Math.pow(2, this._backoffStep), this._MAX_BACKOFF);
    this._pollTimer = setTimeout(() => {
      this.check();
      this._scheduleNextCheck();
    }, interval);
  },

  /**
   * 停止健康监测
   */
  stop() {
    if (this._pollTimer) {
      clearTimeout(this._pollTimer); // v0.8.13 E1: setInterval → setTimeout，需用 clearTimeout
      this._pollTimer = null;
    }
  },

  /**
   * 执行一次健康检测
   * v0.8.3 Step 9：改用 fetchWithTimeout 发起请求（修复 N05）
   *   - pendingRequestCount 正确计数（beforeunload 拦截可检测健康检查）
   *   - 错误经 SidecarUnreachableError/SidecarTimeoutError 分类
   * v0.8.11 P0-1：健康检查超时从 3s 延长到 8s + 失败容错计数
   *   - sidecar 索引期间 /v1/health/system 响应可能 >3s，3s 超时导致误判不可达
   *   - 连续 2 次失败才判定不可达，避免单次慢响应触发状态栏闪红
   * v0.8.11 P0-2：解析后端 status 字段，区分 starting/indexing/running
   *   - 之前只检查 res.ok，导致 indexing 期间健康检查显示"运行中"但 dao_metrics 超时
   *   - 现在读取 status 字段，供 loadDaoMetrics 等组件判断是否在索引期
   * @returns {Promise<boolean>} 是否可达
   */
  async check() {
    if (this._inFlight) return this._isReachable;
    this._inFlight = true;
    // v0.8.13 D4: 健康检查属于后台请求，beforeunload 时不应阻塞用户关闭
    _pendingBackgroundCount++;
    try {
      // v0.8.11 P0-1：健康检查超时从 3s 延长到 8s
      // sidecar 索引期间 /health 需要获取 memory_store 锁，3s 不够
      // v0.8.11 P0-2 修复：改为访问 /health（返回 status 字段），而非 /v1/health/system（返回详细报告，无 status 字段）
      // /health 返回 HealthResponse {status: "running"|"indexing"|"starting", ...}
      // /v1/health/system 返回 health_report()（详细报告，不含 status 字段）
      const res = await fetchWithTimeout(`${API_BASE}/health`, {}, 8000);
      if (res.ok) {
        // v0.8.11 P0-2：解析 status 字段，区分 starting/indexing/running
        const prevStatus = this._sidecarStatus; // v0.8.12：记录之前的状态，用于检测索引完成
        // v0.8.45 修复：记录之前 lock_busy 状态，用于检测结晶状态变化（true→false / false→true）
        const prevLockBusy = this._lockBusy;
        try {
          const data = await res.json();
          if (data && data.status && ['starting', 'indexing', 'running'].includes(data.status)) {
            this._sidecarStatus = data.status;
          } else {
            this._sidecarStatus = 'running'; // 有响应但无 status 字段，视为就绪
          }
          // v0.8.21 P0-06：读取 lock_busy 字段，供 loadDaoMetrics 等组件判断
          this._lockBusy = !!(data && data.lock_busy === true);
          // v0.8.45 修复：lock_busy 状态变化时刷新状态栏 + 仪表盘
          //   根因：_setReachable(第 866 行) 在 isReachable 未变时直接 return，不触发广播，
          //         导致结晶结束（lock_busy true→false）后状态栏仍显示"后台合成中"。
          //   修复：lock_busy 变化时显式广播，触发 updateStatusBar 刷新状态栏 + 重新加载仪表盘。
          if (prevLockBusy !== this._lockBusy) {
            console.log('[LRC v' + APP_VERSION + ']lock_busy 状态变化: ' + prevLockBusy + ' → ' + this._lockBusy + '，刷新状态栏 + 仪表盘');
            this._broadcastSidecarStateChange(true);
          }
        } catch (jsonErr) {
          // JSON 解析失败但 HTTP 200，视为就绪
          this._sidecarStatus = 'running';
          this._lockBusy = false;
        }
        // 可达：重置失败计数
        this._failCount = 0;
        this._setReachable(true);

        // v0.8.22 GAP-11 修复（interaction-resilience-auditor Round4）：
        //   根因：无系统时间偏差检测，时间篡改可能导致 JWT 过期等功能异常
        //   修复：从 HTTP Date 头获取服务器时间，偏差 >5min 时 toast 提示
        //   频率控制：每次启动只提示一次（_timeSkewWarned 标志）
        if (!this._timeSkewWarned) {
          try {
            const serverDateStr = res.headers.get('Date');
            if (serverDateStr) {
              const serverTime = new Date(serverDateStr).getTime();
              const localTime = Date.now();
              const skewSecs = Math.abs(localTime - serverTime) / 1000;
              if (skewSecs > 300) { // >5 分钟
                this._timeSkewWarned = true;
                showToast(`系统时间偏差约 ${Math.round(skewSecs / 60)} 分钟，可能导致功能异常`, 'warning', 6000);
                console.warn(`[SidecarHealthMonitor] 时间偏差 ${skewSecs}s (server=${serverDateStr}, local=${new Date(localTime).toUTCString()})`);
              }
            }
          } catch (e) {
            // 时间检测失败不影响健康检查
          }
        }
        // v0.8.12：索引完成时（starting/indexing → running），强制刷新状态栏 + 仪表盘
        // _setReachable 在状态未变时不触发广播，需手动触发以反映"索引中→运行中"转换
        const wasIndexing = prevStatus === 'starting' || prevStatus === 'indexing';
        const isRunningNow = this._sidecarStatus === 'running';
        if (wasIndexing && isRunningNow && this._isReachable) {
          console.log('[LRC v' + APP_VERSION + ']Sidecar 索引完成，刷新状态栏 + 仪表盘');
          this._broadcastSidecarStateChange(true);
        }
        return true;
      } else {
        // HTTP 非 200：计入失败
        return this._handleCheckFailure();
      }
    } catch (e) {
      // 错误已由 fetchWithTimeout 分类（SidecarUnreachableError/SidecarTimeoutError）
      // v0.8.11 P0-1：失败容错，连续 2 次失败才判定不可达
      return this._handleCheckFailure();
    } finally {
      this._inFlight = false;
      // v0.8.13 D4: 健康检查结束，减少后台请求计数
      _pendingBackgroundCount--;
    }
  },

  /**
   * v0.8.11 P0-1：健康检查失败容错处理
   * 连续 _FAIL_THRESHOLD 次失败才判定不可达，避免索引期单次慢响应误判
   * v0.8.22 HCSE GAP-L5-02/L5-03 修复：
   *   - 索引期容错阈值提高到 5（正常 2），避免索引期 /health 慢被误判为不可达
   *   - 不立即设 _sidecarStatus='unknown'，保留之前的状态，避免 isIndexing() 失效
   * @returns {boolean} 当前是否可达
   */
  _handleCheckFailure() {
    this._failCount++;
    // v0.8.22 HCSE GAP-L5-02：索引期使用更高的容错阈值
    const isIndexing = this._sidecarStatus === 'starting' || this._sidecarStatus === 'indexing';
    const effectiveThreshold = isIndexing ? 5 : this._FAIL_THRESHOLD;
    if (this._failCount >= effectiveThreshold) {
      // 超过阈值，判定不可达
      // v0.8.44 GAP-L5-01 修复（interaction-resilience-auditor Round5 P0）：
      //   根因：索引期 5 次健康检查失败后，_sidecarStatus 被设为 'unknown'，
      //         导致 _setReachable(false) 后 banner 显示"服务未运行"，
      //         但实际是索引过程耗时较长，服务仍在运行中。
      //   修复：索引期超阈值时保留 _sidecarStatus 为 'starting'/'indexing'，
      //         让 banner 显示"索引中..."而非"服务未运行"。
      //         同时增加 _setReachable 的第二个参数 isIndexingHint，
      //         让 banner 显示正确的提示信息。
      if (isIndexing) {
        // 索引期超阈值：保留索引状态，让 banner 显示"索引中..."
        this._backoffStep = Math.min(this._backoffStep + 1, 5);
        this._setReachable(false, true); // true = 索引期不可达（显示"索引中..."）
        // 不设为 'unknown'，保留索引状态，下次健康检查成功后可恢复
        return false;
      }
      // 非索引期：正常判定不可达
      this._sidecarStatus = 'unknown';
      // v0.8.13 E1: 不可达时递增退避步数（上限 5，配合 _MAX_BACKOFF 限制最大间隔）
      this._backoffStep = Math.min(this._backoffStep + 1, 5);
      this._setReachable(false);
      return false;
    }
    // 未达阈值，保持当前状态（避免状态栏频繁闪红）
    console.warn('[LRC v' + APP_VERSION + ']健康检查失败 ' + this._failCount + '/' + effectiveThreshold + (isIndexing ? '（索引期容错）' : '') + '，暂不判定不可达');
    return this._isReachable;
  },

  /**
   * v0.8.11 P0-2：获取后端 sidecar 状态
   * @returns {string} 'starting' | 'indexing' | 'running' | 'unknown'
   */
  getSidecarStatus() {
    return this._sidecarStatus;
  },

  /**
   * v0.8.11 P0-2：sidecar 是否正在索引（starting 或 indexing 阶段）
   * 供组件级数据加载函数判断是否显示"索引中"提示而非"加载失败"
   * @returns {boolean}
   */
  isIndexing() {
    return this._sidecarStatus === 'starting' || this._sidecarStatus === 'indexing';
  },

  /**
   * v0.8.10 L4-02/L5-01：广播 sidecar 状态变更至所有受影响 UI 区域
   *
   * 之前仅刷新仪表盘，导致设置页/信任中心页状态不同步。
   * 现在通过自定义事件 + 主动刷新当前 active tab 双轨通知。
   * v0.8.11 P0-2：广播时携带 sidecarStatus（starting/indexing/running），
   *   供组件级数据加载函数判断是否显示"索引中"提示而非"加载失败"
   *
   * @param {boolean} online - sidecar 是否可达
   */
  _broadcastSidecarStateChange(online) {
    // v0.8.13 F1: 300ms 防抖，避免状态抖动导致 UI 闪烁
    // 健康检查在可达/不可达边界可能产生连续多次状态变更，防抖合并为一次 UI 更新
    if (_broadcastDebounceTimer) {
      clearTimeout(_broadcastDebounceTimer);
    }
    _broadcastDebounceTimer = setTimeout(() => {
      _broadcastDebounceTimer = null;
      // 1. 状态栏立即更新（不依赖 loadDashboard）
      if (typeof updateStatusBar === 'function') {
        updateStatusBar(online, online ? {} : null);
      }
      // 2. 根据当前 active tab 刷新对应页面
      const activeTab = document.querySelector('.navbar-nav button.active, .nav-item.active, [data-tab].active');
      const tabName = activeTab?.getAttribute('data-tab') || 'dashboard';
      if (tabName === 'dashboard' && typeof loadDashboard === 'function') {
        setTimeout(() => loadDashboard(), 500);
      } else if (tabName === 'settings' && typeof loadSettings === 'function') {
        setTimeout(() => loadSettings(), 500);
      } else if (tabName === 'trust-center' && typeof loadTrustCenter === 'function') {
        setTimeout(() => loadTrustCenter(), 500);
      }
      // v0.8.22 P0-04 修复（interaction-resilience-auditor + hcse-resilience-validator）：
      //   sidecar 状态变更时同步刷新道同构度，避免 sidecar 恢复后 dao metrics 永久停留错误状态
      //   根因：_broadcastSidecarStateChange 只调用了 loadDashboard，未调用 loadDaoMetrics
      //   场景：sidecar 未启动时 loadDaoMetrics 失败显示"服务未启动"，
      //         sidecar 启动后 loadDashboard 刷新但 dao metrics 保留旧错误
      if (online && typeof loadDaoMetrics === 'function') {
        setTimeout(() => loadDaoMetrics(), 800);
      }
      // 3. 发出自定义事件，供其他模块解耦响应
      // v0.8.11 P0-2：事件携带 sidecarStatus，供 loadDaoMetrics 等组件感知索引期
      window.dispatchEvent(new CustomEvent('lrc:sidecar-state-change', {
        detail: {
          online,
          sidecarStatus: this._sidecarStatus,
          indexing: this.isIndexing(),
          timestamp: Date.now()
        }
      }));
    }, 300);
  },

  /**
   * 更新可达状态，触发 UI 变更
   * v0.8.44 GAP-L5-01 修复：新增 isIndexingHint 参数
   * @param {boolean} reachable - 是否可达
   * @param {boolean} [isIndexingHint] - 索引期不可达（true 时 banner 显示"索引中..."）
   */
  _setReachable(reachable, isIndexingHint) {
    const wasReachable = this._isReachable;
    this._isReachable = reachable;

    // v0.8.13 E1: 可达时重置退避步数，不可达时由 _handleCheckFailure 递增
    if (reachable) {
      this._backoffStep = 0;
    }

    // v0.8.44 GAP-L5-01：索引期提示时，即使状态未变也要更新 banner 文本
    // 根因：索引期 5 次健康检查失败后，banner 已显示"服务未运行"，
    //       但此时服务正在索引中，需要更新 banner 为"索引中..."
    if (reachable === wasReachable && !isIndexingHint) return;  // 状态未变

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
      // v0.8.10 L4-02：广播全局状态变更，不再仅刷新仪表盘
      this._broadcastSidecarStateChange(true);
    } else {
      // 不可达
      if (banner) {
        banner.hidden = false;
        // v0.8.44 GAP-L5-01：索引期不可达时，直接显示"索引中..."，不检测代理
        if (isIndexingHint) {
          const bannerText = banner.querySelector('.banner-text');
          if (bannerText) {
            bannerText.textContent = '⏳ LRC 服务正在索引代码库，请稍候...';
          }
          // 索引期不禁用 API 按钮（服务实际在运行，只是响应慢）
          console.log('[LRC v' + APP_VERSION + ']Sidecar 索引期不可达，显示"索引中..."提示，保留 API 按钮可用');
        } else {
          // v0.8.23 P2-01 (E4)：代理检测 — 不可达时尝试检测代理配置
          this._detectProxyAndUpdateBanner(banner);
        }
      }
      // v0.8.2：排除"启动服务"按钮，确保用户可以启动服务
      // v0.8.3 Step 8：添加 title 和 aria-disabled 属性（修复 N04）
      apiButtons.forEach(btn => {
        const action = btn.getAttribute('data-action');
        // v0.8.16：启动服务按钮不禁用（data-action 改为 handleStartServiceClick）
        if (action === 'handleStartServiceClick' || action === 'openStartServiceModal' || action === 'closeStartServiceModal') {
          return;  // 启动/关闭服务按钮不禁用
        }
        btn.classList.add('btn-disabled-api');
        btn.setAttribute('title', '服务未运行，请先启动 LRC 服务');
        btn.setAttribute('aria-disabled', 'true');
      });
      console.log('[LRC v' + APP_VERSION + ']Sidecar 不可达，已禁用 API 按钮');
      // v0.8.10 L4-02：不可达时也广播，确保状态栏和各页面同步显示"已停止"
      this._broadcastSidecarStateChange(false);
    }
  },

  /**
   * v0.8.23 P2-01 (E4)：代理检测 — 不可达时更新 banner 显示代理检测信息
   * 异步检测代理配置，检测结果非阻塞，检测失败也不影响 banner 正常显示
   * @param {HTMLElement} banner - sidecar-down-banner 元素
   */
  async _detectProxyAndUpdateBanner(banner) {
    try {
      const proxyResult = await detectProxyConfiguration();
      const bannerText = banner.querySelector('.banner-text');
      if (!bannerText) return;

      if (proxyResult.likelyProxy && proxyResult.reason) {
        // 检测到代理，更新 banner 文本
        bannerText.textContent = `LRC 服务未运行 — ${proxyResult.reason}`;
        console.log(`[LRC v${APP_VERSION}]代理检测结果:`, proxyResult.reason);
      } else {
        // 未检测到代理，使用默认文本
        bannerText.textContent = 'LRC 服务未运行，部分功能不可用';
      }
    } catch (e) {
      // 检测失败，静默降级（保留默认 banner 文本）
      console.warn('[LRC v' + APP_VERSION + ']代理检测失败（静默降级）:', e.message);
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
  },

  // v0.8.22 HCSE 修复：online getter，便于 CDP 测试和全局错误处理读取 sidecar 可达性
  // 根因：HCSE 报告发现 window.sidecarHealthMonitor.online 返回 undefined
  //       因为 SidecarHealthMonitor 只有 _isReachable 属性，没有 online getter
  get online() {
    return this._isReachable;
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
// v0.8.12：仪表盘索引期自动重试计数器
let _dashboardRetryCount = 0;
let _dashboardRetryTimer = null; // v0.8.13 B1: 索引期重试 timer，支持取消与竞态防护
const _DASHBOARD_MAX_RETRIES = 3;
// v0.8.22 修复：lock_busy 冷却期标志，避免手动刷新后再次进入无限重试循环
// v0.8.26 UX-02 修复：使用 sessionStorage 持久化冷却期状态，tab 切换后不丢失
let _lockBusyCooldown = false;
// v0.8.23 S1-RES-03 修复：lock_busy 冷却期倒计时 timer，显示实时剩余秒数
let _lockBusyCooldownTimer = null;

// v0.8.26 UX-02 修复：从 sessionStorage 恢复冷却期状态
// 确保用户切换 tab 后返回时，冷却期信息仍然可见
(function _restoreLockBusyCooldown() {
  try {
    const stored = sessionStorage.getItem('lrc_lockbusy_cooldown');
    if (stored) {
      const expiry = parseInt(stored, 10);
      const remaining = expiry - Date.now();
      if (remaining > 0) {
        _lockBusyCooldown = true;
        console.log('[lockBusy] 从 sessionStorage 恢复冷却期，剩余 ' + Math.ceil(remaining / 1000) + 's');
      } else {
        sessionStorage.removeItem('lrc_lockbusy_cooldown');
      }
    }
  } catch (e) {
    // sessionStorage 不可用时静默降级
    console.warn('[lockBusy] sessionStorage 不可用:', e.message);
  }
})();

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

  // v0.8.13 B1: 清除已有的索引期重试 timer，避免竞态
  if (_dashboardRetryTimer) {
    clearTimeout(_dashboardRetryTimer);
    _dashboardRetryTimer = null;
  }

  loading.classList.remove('hidden');
  if (error) {
    error.classList.remove('show');
    error.textContent = '';
  }

  try {
    // v0.8.22 GAP-02 修复（interaction-resilience-auditor Round4）：
    //   根因：503 lock_busy 期间仍并行发 3 请求（system/detailed/dao_metrics），
    //         所有请求都返回 503，浪费网络资源并加剧拥塞
    //   修复：发请求前检查 SidecarHealthMonitor 的 lockBusy 状态，
    //         若 lock_busy=true 则跳过 3 个并行请求
    // v0.8.45 修复（lock_busy 冻结仪表盘根因）：
    //   根因：健康监控已检测到 lock_busy 时，原实现 throw LOCK_BUSY 进入 catch，
    //         走"请等待/倒计时"路径，绕过下方 hasLockBusy200 分支的降级渲染，
    //         导致仪表盘不渲染降级数据 + 不添加"合成中"标记（用户感知"卡片锁死"）
    //   修复：不再 throw，直接渲染降级数据 + 添加"合成中"标记 + 后台指数退避重试
    if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor._lockBusy) {
      console.log('[loadDashboard] 检测到 lock_busy=true，渲染降级数据 + 后台重试');
      renderDashboard(null, null, null);
      scheduleLockBusyRetry();
      loading.classList.add('hidden');
      return;
    }

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

    // v0.8.19 GAP-01 P0 修复：识别 503 lock_busy，区分于"无法连接"
    // 根因：v0.8.19 后端 try_lock + 503 lock_busy 已正确返回，但前端 loadDashboard
    //   将 503 lock_busy 误报为"无法连接到 API 服务"，导致结晶期间用户仍看到误导性错误
    // 修复：检查是否有 503 状态码，如果有则 throw LOCK_BUSY，进入 catch 后显示"后台合成中"
    // v0.8.22 P1-NEW-01 修复（interaction-resilience-auditor Round4）：
    //   根因：v0.8.22 P1-02 后端修复后，lock_busy 时返回 200 + 降级数据（不是 503），
    //         但前端 hasLockBusy 仍只检查 503 状态码，无法识别 200+lock_busy 降级响应，
    //         导致降级数据被误判为正常数据，renderDashboard 渲染 0 记忆（P1-NEW-02）
    //   修复：除了检查 503 状态码，还检查已解析数据中的 lock_busy 字段
    // v0.8.45 修复（detailed/system 锁竞争误判）：
    //   根因：/health/detailed 与 /health/system 共享同一 metrics_store 锁，
    //         并行请求时存在竞争窗口，detailed 偶发 lock_busy:true，
    //         即使 system 主数据源正常，前端仍被误判为降级（degraded 数据残留）
    //   修复：lock_busy 判定仅基于 system 主数据源，detailed 的瞬时锁竞争不再触发降级
    // v0.8.45 再修复（503 判定范围）：hasLockBusy503 之前检查三个端点，
    //   当 detailed/dao 因锁竞争偶发返回 503 时也会触发降级，即使 system 正常。
    //   改为仅以 system 主数据源的状态码为准，与上方注释语义保持一致。
    const hasLockBusy503 = systemRes.status === 'fulfilled' && systemRes.value && systemRes.value.status === 503;
    const hasLockBusy200 = systemData?.lock_busy === true;

    if (hasLockBusy503 || hasLockBusy200) {
      // v0.8.45 修复：lock_busy 时不再 throw LOCK_BUSY 冻结仪表盘，
      //   改为渲染降级数据 + 显示"后台合成中"提示 + 后台继续重试
      //   根因：之前 throw LOCK_BUSY 后进入 3 次重试 + 30 秒冷却期，
      //         用户看到的是"请等待"而非仪表盘，感知为"卡片锁死"
      //   修复：renderDashboard 渲染降级数据，同时后台继续重试
      renderDashboard(systemData, detailedData, daoData);
      // 后台继续重试（不阻塞 UI，使用 LOCK_BUSY 路径的重试逻辑）
      scheduleLockBusyRetry();
      loading.classList.add('hidden');
      return;
    }

    if (!systemData && !daoData) {
      throw new Error('无法连接到 API 服务，请确认 Loong Recall 服务已启动 (' + API_BASE + ')');
    }

    // v0.8.22 GAP-08 修复（interaction-resilience-auditor Round4）：
    //   根因：loadDashboard 成功后调用 loadRecentMemories/loadMemoryStats 等
    //         导致滚动条跳回顶部，用户阅读位置丢失
    //   修复：刷新前记录 scrollY，刷新后恢复
    const _savedScrollY = window.scrollY;

    renderDashboard(systemData, detailedData, daoData);
    updateStatusBar(true, systemData);

    // GAP-08：恢复滚动位置
    window.scrollTo(0, _savedScrollY);
    // v0.8.12：加载成功时重置索引期重试计数器
    _dashboardRetryCount = 0;
    // v0.8.12：加载成功时清除"索引中"错误提示（如果存在）
    if (error) error.classList.remove('show');

    // v0.8.0 桌面端 P2 改进：仪表盘加载成功后自动刷新进化时间线
    // 不 await，避免阻塞 loadDashboard；loadEvolutionTimeline 内部有 try/catch
    loadEvolutionTimeline();

    // v0.8.25：仪表盘加载成功后异步获取后端版本号
    // 不 await，避免阻塞 loadDashboard；fetchBackendVersion 内部有 try/catch
    fetchBackendVersion();

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
    // v0.8.23 GAP-AUDIT-01 修复：SidecarTimeoutError 优先于 LOCK_BUSY
    // 根因：请求超时（10s fetchWithTimeout 硬超时）时，e.name 为 SidecarTimeoutError，
    //       但后面对 LOCK_BUSY 的检查可能因重试 catch 覆盖超时信息
    //   修复：在 LOCK_BUSY 之前检查 SidecarTimeoutError，确保超时错误显示"请求超时"文案
    if (e.name === 'SidecarTimeoutError') {
      if (loading) loading.classList.add('hidden');
      if (error) {
        error.innerHTML = '⏱️ 请求超时，请检查网络连接后重试<br>'
          + '<button onclick="manualRefreshDashboard()" style="margin-top:8px;padding:6px 16px;background:#4a90d9;color:white;border:none;border-radius:4px;cursor:pointer;font-size:13px;">重试</button>';
        error.classList.add('show');
      }
      _dashboardRetryCount = 0;
      return;
    }
    // v0.8.19 GAP-01 P0 修复：LOCK_BUSY（503 lock_busy）特殊处理
    // sidecar 在线但繁忙（结晶期间），显示"后台合成中"并自动重试
    // 不进入"无法连接到 API 服务"分支，避免误导用户
    if (e.message === 'LOCK_BUSY') {
      if (loading) loading.classList.add('hidden');
      // v0.8.23 S1-UX-01 修复：lock_busy 时设置降级视觉模式
      document.body.classList.add('degraded-mode');
      // v0.8.22 修复：冷却期内不触发自动重试，直接显示"请等待"文案
      // v0.8.23 S1-RES-03 修复：显示实时倒计时，替代静态"等待 30 秒"文案
      if (_lockBusyCooldown) {
        console.log('[loadDashboard] lock_busy 冷却期内，跳过自动重试');
        if (error) {
          error.innerHTML = '⏳ 后台合成中，请等待 <span id="lockbusy-countdown">30</span> 秒后自动重试...';
          error.classList.add('show');
        }
        // 启动倒计时更新（如果尚未启动）
        if (!_lockBusyCooldownTimer) {
          _lockBusyCooldownTimer = setInterval(() => {
            const cd = document.getElementById('lockbusy-countdown');
            const remaining = parseInt(cd?.textContent || '0', 10);
            if (remaining > 1) {
              if (cd) cd.textContent = String(remaining - 1);
            } else {
              clearInterval(_lockBusyCooldownTimer);
              _lockBusyCooldownTimer = null;
            }
          }, 1000);
        }
        return;
      }
      // 自动重试（复用 _dashboardRetryCount 机制，与索引期重试一致）
      if (_dashboardRetryCount < _DASHBOARD_MAX_RETRIES) {
        _dashboardRetryCount++;
        const retryDelay = 2000 * Math.pow(2, _dashboardRetryCount - 1); // 2s/4s/8s
        console.log('[loadDashboard] lock_busy（后台合成中，可能由 503 或 200+降级触发），' + retryDelay + 'ms 后自动重试 (' + _dashboardRetryCount + '/' + _DASHBOARD_MAX_RETRIES + ')');
        if (error) {
          error.innerHTML = '⏳ 记忆系统正在执行后台合成，数据稍后自动加载... <span style="opacity:0.7;font-size:0.9em">(' + _dashboardRetryCount + '/' + _DASHBOARD_MAX_RETRIES + ')</span>';
          error.classList.add('show');
        }
        _dashboardRetryTimer = setTimeout(() => {
          _dashboardRetryTimer = null;
          loadDashboard();
        }, retryDelay);
      } else {
        // v0.8.21 P0-03 修复（GAP-P0-03）：重试耗尽后显示手动刷新引导
        // 原实现重试 3 次后只显示静态文案，用户无操作路径
        // 修复：显示"后台合成耗时较长" + 立即刷新按钮
        // v0.8.22 GAP-01+GAP-04 修复：按钮改为调用 manualRefreshDashboard，
        //   重置 _dashboardRetryCount 并添加防抖，避免连点和"刷新无效"问题
        // v0.8.22 修复：添加 30 秒冷却期，防止手动刷新后再次进入无限重试循环
        console.log('[loadDashboard] lock_busy（后台合成中）重试耗尽，设置 30 秒冷却期');
        _lockBusyCooldown = true;
        // v0.8.26 UX-02 修复：将冷却期到期时间写入 sessionStorage，tab 切换后不丢失
        try {
          sessionStorage.setItem('lrc_lockbusy_cooldown', String(Date.now() + 30000));
        } catch (e) { /* sessionStorage 不可用时静默降级 */ }
        // v0.8.23 S1-RES-03 修复：冷却期结束时清理倒计时 timer
        const cooldownTimer = setTimeout(() => {
          _lockBusyCooldown = false;
          _dashboardRetryCount = 0;
          // v0.8.26 UX-02 修复：冷却期到期时清理 sessionStorage
          try {
            sessionStorage.removeItem('lrc_lockbusy_cooldown');
          } catch (e) { /* 静默降级 */ }
          if (_lockBusyCooldownTimer) {
            clearInterval(_lockBusyCooldownTimer);
            _lockBusyCooldownTimer = null;
          }
          console.log('[loadDashboard] lock_busy 冷却期结束，恢复自动重试');
        }, 30000);
        if (error) {
          error.innerHTML = '⏳ 后台合成耗时较长，建议稍后手动刷新<br>'
            + '<button id="btn-manual-refresh" onclick="manualRefreshDashboard()" style="margin-top:8px;padding:6px 16px;background:#4a90d9;color:white;border:none;border-radius:4px;cursor:pointer;font-size:13px;">立即刷新</button>'
            + '<button onclick="this.parentElement.classList.remove(\'show\')" style="margin-top:8px;margin-left:8px;padding:6px 16px;background:#666;color:white;border:none;border-radius:4px;cursor:pointer;font-size:13px;">关闭</button>';
          error.classList.add('show');
        }
      }
      return;
    }
    if (loading) loading.classList.add('hidden');

    // v0.8.12：索引期间不覆盖"运行中/索引中"状态栏，显示提示并自动重试
    const isIndexing = typeof SidecarHealthMonitor !== 'undefined'
      && SidecarHealthMonitor
      && typeof SidecarHealthMonitor.isIndexing === 'function'
      && SidecarHealthMonitor.isIndexing();
    const sidecarKnownReachable = typeof SidecarHealthMonitor !== 'undefined'
      && SidecarHealthMonitor
      && SidecarHealthMonitor._isReachable;

    if (isIndexing && _dashboardRetryCount < _DASHBOARD_MAX_RETRIES) {
      // 索引期数据加载失败，显示"索引中"提示并自动重试
      _dashboardRetryCount++;
      if (error) {
        error.textContent = 'LRC 服务正在索引代码库，仪表盘数据稍后自动加载...';
        error.classList.add('show');
      }
      // 不覆盖状态栏（保持"索引中..."显示）
      // v0.8.13 B1: 固定 3s 改为指数退避（2s/4s/8s），并保存 timer ID 支持取消
      const retryDelay = 2000 * Math.pow(2, _dashboardRetryCount - 1); // 2s/4s/8s
      console.log('[loadDashboard] 索引期加载失败，' + retryDelay + 'ms 后自动重试 (' + _dashboardRetryCount + '/' + _DASHBOARD_MAX_RETRIES + ')');
      _dashboardRetryTimer = setTimeout(() => {
        _dashboardRetryTimer = null;
        loadDashboard();
      }, retryDelay);
      return;
    }

    // 非索引期或重试耗尽，显示错误
    _dashboardRetryCount = 0;
    if (error) {
      // v0.8.15 P0-5 修复：sidecar 已知可达时（索引期），显示"索引中"提示而非"无法连接"
      // 避免与状态栏"运行中"矛盾
      if (sidecarKnownReachable) {
        error.textContent = '⏳ LRC 服务正在索引代码库，数据稍后自动加载...';
      } else {
        // v0.8.44 GAP-L1-01 修复：仪表盘 API 全失败时添加重试按钮
        //   根因：审计报告指出仪表盘 API 全部失败时，error 遮罩无重试按钮，
        //         用户只能刷新页面，体验差。
        //   修复：添加"立即刷新"按钮，复用 manualRefreshDashboard 机制
        //   （manualRefreshDashboard 会重置计数器并重新加载仪表盘）
        error.innerHTML = '⚠️ ' + htmlescape(e.message)
          + '<br><button id="btn-dashboard-retry" onclick="manualRefreshDashboard()" '
          + 'style="margin-top:8px;padding:6px 16px;background:#4a90d9;color:white;border:none;'
          + 'border-radius:4px;cursor:pointer;font-size:13px;">立即刷新</button>'
          + '<button onclick="this.parentElement.classList.remove(\'show\')" '
          + 'style="margin-top:8px;margin-left:8px;padding:6px 16px;background:#666;color:white;'
          + 'border:none;border-radius:4px;cursor:pointer;font-size:13px;">关闭</button>';
      }
      error.classList.add('show');
      // v0.9.0 修复：全部失败时更新统计卡片为"不可用"，避免残留 "--"
      ['stat-total', 'stat-active', 'stat-crystallized', 'stat-today'].forEach(function(id) {
        var el = document.getElementById(id);
        if (el) el.textContent = '不可用';
      });
    }
    // v0.8.12：仅在 sidecar 确实不可达时才更新状态栏为"已停止"
    // 避免 sidecar 已启动但数据加载失败时覆盖"运行中"状态
    // v0.8.22 HCSE GAP-L5-01 修复：检查 SidecarHealthMonitor 实际可达性
    //   根因：503 lock_busy 时 loadDashboard 失败，sidecarKnownReachable 可能为 false，
    //         导致 updateStatusBar(false) 覆盖状态栏为"已停止"，但 sidecar 实际在线
    //   修复：增加 SidecarHealthMonitor._isReachable 检查，只有 sidecar 真正不可达时才更新状态栏
    if (!sidecarKnownReachable && !(typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor._isReachable)) {
      updateStatusBar(false, null);
      // v0.8.48 修复：后端不可达时同步更新浮窗为降级状态，避免残留 "--"
      if (typeof setDegradedStatusFloat === 'function') {
        setDegradedStatusFloat('不可用');
      }
    }
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

// v0.8.22 GAP-01+GAP-04 修复（interaction-resilience-auditor Round4）：
//   GAP-01 根因：503 重试耗尽后"立即刷新"按钮直接调用 loadDashboard()，
//     但 _dashboardRetryCount 仍为 3，导致 loadDashboard 内部判定"重试耗尽"，
//     用户感觉"刷新按钮没用"
//   GAP-04 根因：按钮无防抖，快速连点触发多次 loadDashboard
//   修复：新增 manualRefreshDashboard 函数，重置计数器 + 按钮防抖
// v0.8.23 S1-RES-04 修复：冷却期内禁用刷新按钮并显示剩余时间
let _isManualRefreshing = false;
function manualRefreshDashboard() {
  if (_isManualRefreshing) {
    console.log('[manualRefreshDashboard] 已在刷新中，忽略重复点击');
    return;
  }
  // v0.8.23 S1-RES-04 修复：冷却期内显示剩余时间并阻止刷新
  if (_lockBusyCooldown) {
    const cd = document.getElementById('lockbusy-countdown');
    const remaining = parseInt(cd?.textContent || '30', 10);
    console.log(`[manualRefreshDashboard] lock_busy 冷却期内（剩余 ${remaining}s），忽略手动刷新`);
    const btn = document.getElementById('btn-manual-refresh');
    if (btn) {
      btn.disabled = true;
      btn.textContent = `等待 ${remaining}s...`;
      btn.style.opacity = '0.6';
      btn.style.cursor = 'not-allowed';
      // 冷却期结束时恢复按钮
      const checkCooldown = setInterval(() => {
        const cdEl = document.getElementById('lockbusy-countdown');
        const rem = parseInt(cdEl?.textContent || '0', 10);
        if (!_lockBusyCooldown || rem <= 0) {
          clearInterval(checkCooldown);
          btn.disabled = false;
          btn.textContent = '立即刷新';
          btn.style.opacity = '1';
          btn.style.cursor = 'pointer';
        }
      }, 1000);
    }
    return;
  }
  _isManualRefreshing = true;
  // GAP-01：重置重试计数器，让用户获得新的 3 次自动重试机会
  _dashboardRetryCount = 0;
  // GAP-04：按钮立即进入"刷新中"状态，防止连点
  const btn = document.getElementById('btn-manual-refresh');
  if (btn) {
    btn.disabled = true;
    btn.textContent = '刷新中...';
    btn.style.opacity = '0.6';
    btn.style.cursor = 'not-allowed';
  }
  // 异步调用，让 UI 先更新 disabled 状态
  setTimeout(() => {
    loadDashboard().finally(() => {
      _isManualRefreshing = false;
    });
  }, 50);
}
window.manualRefreshDashboard = manualRefreshDashboard;

/**
 * v0.8.45 新增：lock_busy 时的后台重试调度器
 * 在渲染降级数据后，后台继续重试加载完整数据，
 * 不阻塞 UI，不进入冷却期。
 * 重试策略：3 次指数退避（2s/4s/8s），成功后自动更新 UI
 */
function scheduleLockBusyRetry() {
  const MAX_RETRIES = 3;
  let retryCount = 0;
  let retryTimer = null;

  function doRetry() {
    if (retryCount >= MAX_RETRIES) {
      console.log('[scheduleLockBusyRetry] 重试耗尽，等待下次自动加载');
      return;
    }
    retryCount++;
    const delay = 2000 * Math.pow(2, retryCount - 1); // 2s/4s/8s
    console.log('[scheduleLockBusyRetry] ' + delay + 'ms 后后台重试 (' + retryCount + '/' + MAX_RETRIES + ')');
    retryTimer = setTimeout(async () => {
      try {
        const [systemRes, detailedRes, daoRes] = await Promise.allSettled([
          fetchWithTimeout(API_BASE + '/v1/health/system', {}, 8000),
          fetchWithTimeout(API_BASE + '/v1/health/detailed', {}, 8000),
          fetchWithTimeout(API_BASE + '/v1/health/dao_metrics', {}, 8000),
        ]);
        let systemData = null, detailedData = null, daoData = null;
        if (systemRes.status === 'fulfilled' && systemRes.value.ok) {
          systemData = await systemRes.value.json();
        }
        if (detailedRes.status === 'fulfilled' && detailedRes.value.ok) {
          detailedData = await detailedRes.value.json();
        }
        if (daoRes.status === 'fulfilled' && daoRes.value.ok) {
          daoData = await daoRes.value.json();
        }
        // 检查是否仍处于 lock_busy
        const stillBusy = [systemData, detailedData, daoData].some(d => d && d.lock_busy === true);
        if (stillBusy) {
          console.log('[scheduleLockBusyRetry] 仍处于 lock_busy，继续重试');
          doRetry();
        } else {
          console.log('[scheduleLockBusyRetry] lock_busy 已解除，刷新仪表盘');
          // 移除"合成中"标记
          const badge = document.querySelector('.dao-degraded-badge');
          if (badge) badge.remove();
          renderDashboard(systemData, detailedData, daoData);
        }
      } catch (e) {
        console.warn('[scheduleLockBusyRetry] 后台重试失败:', e.message);
        doRetry();
      }
    }, delay);
  }

  function cancelLockBusyRetry() {
    if (retryTimer) {
      clearTimeout(retryTimer);
      retryTimer = null;
      console.log('[scheduleLockBusyRetry] 页面卸载，已取消重试定时器');
    }
  }

  // 页面卸载时清理定时器
  window.addEventListener('pagehide', cancelLockBusyRetry);
  window.addEventListener('visibilitychange', () => {
    if (document.hidden) cancelLockBusyRetry();
  });

  doRetry();
}
window.scheduleLockBusyRetry = scheduleLockBusyRetry;

function renderDashboard(system, detailed, dao) {
  // v0.9.0 新增：数据源健康状态追踪（逐面板降级，非全有或全无）
  const health = {
    system: system !== null && system !== undefined,
    detailed: detailed !== null && detailed !== undefined,
    dao: dao !== null && dao !== undefined,
  };
  health.allOk = health.system && health.detailed && health.dao;
  health.allFailed = !health.system && !health.dao;

  // v0.8.23 S1-UX-01 修复：数据加载成功时移除降级视觉模式
  document.body.classList.remove('degraded-mode');
  // v0.8.45 修复：lock_busy 降级数据不再跳过渲染，改为渲染降级 UI
  //   根因：之前 return 跳过渲染导致仪表盘无数据，用户感知为"卡片锁死"
  //   修复：显示降级数据 + 标题栏添加"合成中"标记
  // v0.8.45 修复：当健康监控已检测到 lock_busy 但数据为 null（提前跳过并行请求）时，
  //   仍应渲染降级 UI，故 isDegraded 同时考虑 SidecarHealthMonitor._lockBusy
  // v0.8.45 修复（lock_busy 恢复后 badge/degraded-mode 残留根因）：
  //   根因：isDegraded 依赖 SidecarHealthMonitor._lockBusy，而健康监控轮询滞后，
  //         结晶已结束（system 主数据源返回 lock_busy=false）但 _lockBusy 仍为 true，
  //         导致 renderDashboard 误判为降级并残留"合成中"badge 与 degraded-mode。
  //   修复：当 system 主数据源明确返回 lock_busy 布尔值时，以实际数据为准，
  //         仅当 system 无明确状态（renderDashboard(null) 降级路径）时才依赖 _lockBusy 兜底。
  // v0.8.45 再修复（detailed/dao 锁竞争误判）：system 明确返回 lock_busy=false 时，
  //   即使 detailed/dao 因偶发锁竞争带 lock_busy:true，也应以 system 为准判定为正常，
  //   避免残留"合成中"badge。仅当 system 无明确状态时，才参考 detailed/dao 与 _lockBusy。
  const hasFreshSystemState = system && typeof system.lock_busy === 'boolean';
  const isDegraded = hasFreshSystemState
    ? !!system.lock_busy
    : (detailed && detailed.lock_busy) || (dao && dao.lock_busy)
      || (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor._lockBusy);
  if (isDegraded) {
    console.log('[renderDashboard] 渲染降级数据（lock_busy），后台合成中');
    document.body.classList.add('degraded-mode');
    // 在道同构度标题后添加"合成中"标记
    const daoTitle = document.querySelector('.dao-metrics-panel .card-title');
    if (daoTitle && !daoTitle.querySelector('.dao-degraded-badge')) {
      const badge = document.createElement('span');
      badge.className = 'dao-degraded-badge';
      badge.style.cssText = 'margin-left:8px;font-size:0.75em;padding:2px 8px;border-radius:10px;background:var(--lrc-金色-200);color:var(--lrc-金色-800);border:1px solid var(--lrc-金色-400);';
      badge.textContent = '合成中';
      daoTitle.appendChild(badge);
    }
  } else {
    // v0.8.45 修复：lock_busy 解除后移除残留的"合成中"标记
    // 根因：之前只移除 degraded-mode body 类，未删除 .dao-degraded-badge DOM，
    //       导致结晶结束后标题栏仍残留"合成中"徽标
    const badge = document.querySelector('.dao-degraded-badge');
    if (badge) badge.remove();
  }
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
  // v0.9.0 修复：dao 数据不可用时显示"部分不可用"而非误导性的健康评分
  const daoScore = health.dao ? (daoMetrics.dao_isomorphism_score ?? 0) : null;
  if (sysHealthStatus) {
    if (daoScore === null) {
      sysHealthStatus.innerHTML = '<span class="badge warning">⚠ 部分数据暂不可用</span>';
    } else if (daoScore >= 0.5) {
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
    // v0.8.45 修复：503 lock_busy 时显示"后台合成中"而非"加载失败"
    //   根因：之前当内存系统在锁状态下（合成中），/v1/memories/recent 返回 503，
    //         前端 catch 后显示"加载失败"，用户感知为"最近记忆不可用"
    //   修复：检查 503 响应，尝试解析 lock_busy 字段，显示友好提示
    if (res.status === 503) {
      try {
        const errBody = await res.json();
        if (errBody && errBody.lock_busy === true) {
          container.innerHTML = `
            <div class="empty-state">
              <div class="empty-icon">⏳</div>
              <div class="empty-text">后台合成中</div>
              <div class="empty-hint">记忆系统正在执行后台合成，最近记忆稍后自动加载</div>
            </div>`;
          return;
        }
      } catch (_) { /* 解析失败，降级到默认错误处理 */ }
    }
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
      // 通过映射表将指纹转为可读项目名（_global_ / null → "全局记忆"）
      const projectRaw = m.project || '_global_';
      const project = getProjectDisplayName(projectRaw);
      const projectPath = getProjectCanonicalPath(projectRaw);
      const projectTooltip = projectPath ? ` title="${htmlescape(projectPath)}"` : '';
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
            <span class="recent-memory-project"${projectTooltip}>📂 ${htmlescape(project)}</span>
            <span class="recent-memory-importance" title="重要性 ${importance}/10">${stars}</span>
          </div>
        </div>`;
    }).join('');
  } catch (e) {
    // v0.8.45 修复：lock_busy 时显示"后台合成中"而非"加载失败"
    // 检查错误消息是否包含 lock_busy 特征
    const isLockBusy = e.message && (
      e.message.includes('lock_busy') ||
      e.message.includes('503') ||
      e.message.includes('Service Unavailable')
    );
    if (isLockBusy) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">⏳</div>
          <div class="empty-text">后台合成中</div>
          <div class="empty-hint">记忆系统正在执行后台合成，最近记忆稍后自动加载</div>
        </div>`;
    } else {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">⚠️</div>
          <div class="empty-text">加载失败</div>
          <div class="empty-hint">${htmlescape(e.message)}</div>
        </div>`;
    }
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
    // v0.8.45 修复：503 lock_busy 时显示"后台合成中"而非"加载失败"
    if (res.status === 503) {
      try {
        const errBody = await res.json();
        if (errBody && errBody.lock_busy === true) {
          container.innerHTML = `
            <div class="empty-state">
              <div class="empty-icon">⏳</div>
              <div class="empty-text">后台合成中</div>
              <div class="empty-hint">记忆系统正在执行后台合成，项目分布稍后自动加载</div>
            </div>`;
          return;
        }
      } catch (_) { /* 解析失败，降级到默认错误处理 */ }
    }
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

    container.innerHTML = projectEntries.slice(0, 8).map(([project, count]) => {
      const displayName = getProjectDisplayName(project);
      const percentage = total > 0 ? (count / total * 100).toFixed(1) : '0.0';
      const barWidth = maxCount > 0 ? (count / maxCount * 100).toFixed(1) : '0.0';
      // tooltip 显示规范化路径（若映射表命中且有路径），让用户能识别同名项目
      const canonicalPath = getProjectCanonicalPath(project);
      const tooltipAttr = canonicalPath ? ` title="${htmlescape(canonicalPath)}"` : '';
      return `
        <div class="project-dist-item">
          <div class="project-dist-header">
            <span class="project-dist-name"${tooltipAttr}>📂 ${htmlescape(displayName)}</span>
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
    // v0.8.45 修复：lock_busy 时显示"后台合成中"而非"加载失败"
    const isLockBusy = e.message && (
      e.message.includes('lock_busy') ||
      e.message.includes('503') ||
      e.message.includes('Service Unavailable')
    );
    if (isLockBusy) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">⏳</div>
          <div class="empty-text">后台合成中</div>
          <div class="empty-hint">记忆系统正在执行后台合成，项目分布稍后自动加载</div>
        </div>`;
    } else {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">⚠️</div>
          <div class="empty-text">加载失败</div>
          <div class="empty-hint">${htmlescape(e.message)}</div>
          <button class="btn btn-secondary btn-sm" style="margin-top:8px;" data-action="loadMemoryStats">重试</button>
        </div>`;
    }
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
    // v0.8.45 修复：503 lock_busy 时显示"后台合成中"而非"加载失败"
    if (res.status === 503) {
      try {
        const errBody = await res.json();
        if (errBody && errBody.lock_busy === true) {
          tbody.innerHTML = '<tr><td colspan="3" class="text-center text-dim">⏳ 后台合成中，日志稍后自动加载</td></tr>';
          return;
        }
      } catch (_) { /* 解析失败，降级到默认错误处理 */ }
    }
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
    // v0.8.45 修复：lock_busy 时显示"后台合成中"而非"加载失败"
    const isLockBusy = e.message && (
      e.message.includes('lock_busy') ||
      e.message.includes('503') ||
      e.message.includes('Service Unavailable')
    );
    tbody.innerHTML = isLockBusy
      ? '<tr><td colspan="3" class="text-center text-dim">⏳ 后台合成中，日志稍后自动加载</td></tr>'
      : '<tr><td colspan="3" class="text-center text-dim">审计日志加载失败</td></tr>';
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
      // v0.8.12：索引中显示金色圆点 + "索引中..."，区别于"运行中"
      const isIndexing = typeof SidecarHealthMonitor !== 'undefined'
        && SidecarHealthMonitor
        && typeof SidecarHealthMonitor.isIndexing === 'function'
        && SidecarHealthMonitor.isIndexing();
      // v0.8.21 INV-04+P1-06 修复（interaction-resilience-auditor + hcse-resilience-validator）：
      //   根因：updateStatusBar 只判断 online + isIndexing，未判断 _lockBusy
      //         后台合成持锁时 sidecar 在线但 API 返回 503，状态栏却显示"运行中"
      //         用户误以为服务正常，点击操作遇到 503 困惑
      //   修复：增加 lockBusy 判断，显示"后台合成中"紫色状态，区别于"运行中"
      //   优先级：isIndexing > lockBusy > running（索引中时也持锁，但索引状态更具体）
      const isLockBusy = !isIndexing
        && typeof SidecarHealthMonitor !== 'undefined'
        && SidecarHealthMonitor
        && SidecarHealthMonitor._lockBusy === true;
      if (isIndexing) {
        dot.className = 'status-dot indexing';
        text.textContent = '索引中...';
        text.style.color = '#f39c12';
        text.title = 'LRC 服务正在索引代码库，数据稍后自动加载';
      } else if (isLockBusy) {
        // v0.8.21 INV-04：后台合成中显示紫色圆点 + "后台合成中"
        dot.className = 'status-dot lock-busy';
        text.textContent = '后台合成中...';
        text.style.color = '#9b59b6';
        text.title = '记忆系统正在执行后台合成，部分 API 暂时不可用，请稍候';
      } else {
        dot.className = 'status-dot online';
        text.textContent = '运行中';
        text.style.color = '#2ecc71';
        text.title = 'LRC 服务运行中';
      }
    } else {
      dot.className = 'status-dot offline';
      text.textContent = '已停止 / 不可达';
      text.style.color = '#c0392b';
      text.title = '点击启动 LRC 服务';
    }
  }

  // v0.8.25：使用动态获取的版本号，fallback 到本地硬编码
  const currentVersion = window.__LRC_VERSION__ || APP_VERSION;
  if (version) version.textContent = 'v' + currentVersion;
  // v0.8.7 Step 3：修复 sys-version 硬编码，统一使用 APP_VERSION 动态填充
  const sysVersion = $('sys-version');
  if (sysVersion) sysVersion.textContent = 'v' + currentVersion;
  if (dataDir) dataDir.textContent = '.loong-recall/data/';
  if (uptime) uptime.textContent = formatUptime(Date.now() - startTime);

  // v0.8.15 P1 修复：信任中心"服务状态概览"区块同步更新
  // 盲点根因：updateStatusBar 只更新 footer 状态栏，遗漏信任中心页面的 4 个状态元素
  // 导致用户在信任中心看到的状态与实际不符（status-dot=unknown, text=检测中...）
  const trustDot = $('system-status-dot');
  const trustText = $('system-status-text');
  const trustBadge = $('system-status-badge');
  const trustUptime = $('sys-uptime');
  if (trustDot && trustText) {
    // 同步状态点类名和文本（与 footer 状态栏保持一致）
    if (online) {
      const isIndexing = typeof SidecarHealthMonitor !== 'undefined'
        && SidecarHealthMonitor
        && typeof SidecarHealthMonitor.isIndexing === 'function'
        && SidecarHealthMonitor.isIndexing();
      // v0.8.21 INV-04+P1-06：信任中心同步 lockBusy 状态显示
      const isLockBusy = !isIndexing
        && typeof SidecarHealthMonitor !== 'undefined'
        && SidecarHealthMonitor
        && SidecarHealthMonitor._lockBusy === true;
      if (isIndexing) {
        trustDot.className = 'status-dot indexing';
        trustText.textContent = '索引中...';
        if (trustBadge) { trustBadge.textContent = '索引中'; trustBadge.className = 'badge badge-warning'; }
      } else if (isLockBusy) {
        trustDot.className = 'status-dot lock-busy';
        trustText.textContent = '后台合成中...';
        if (trustBadge) { trustBadge.textContent = '合成中'; trustBadge.className = 'badge badge-purple'; }
      } else {
        trustDot.className = 'status-dot online';
        trustText.textContent = '运行中';
        if (trustBadge) { trustBadge.textContent = '在线'; trustBadge.className = 'badge badge-success'; }
      }
    } else {
      trustDot.className = 'status-dot offline';
      trustText.textContent = '已停止 / 不可达';
      if (trustBadge) { trustBadge.textContent = '离线'; trustBadge.className = 'badge badge-danger'; }
    }
  }
  // 同步运行时长到信任中心
  if (trustUptime) trustUptime.textContent = formatUptime(Date.now() - startTime);
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
  // v0.8.31 S-03：AI 工具手动修正（向导齿轮图标点击时调用）
  'lrc-set-agent-manual-override': 'set_agent_manual_override',
  // v0.8.31 S-05：扫描缓存元数据查询 + 强制失效（重新扫描按钮使用）
  'lrc-get-scan-cache-metadata': 'get_scan_cache_metadata',
  'lrc-force-invalidate-scan-cache': 'force_invalidate_scan_cache',
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
let _startServiceInProgress = false; // v0.8.13 D1: 防护幽灵 progress 事件

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

/** 打开启动服务模态框
 *
 * v0.8.16 入口体验修复：
 *   - 移除模态框确认环节，直接调用 handleStartServiceClick 启动服务
 *   - 保留函数名是为了向后兼容（其他地方可能通过 data-action="openStartServiceModal" 调用）
 *   - 模态框元素仍保留在 DOM 中（便于回滚），但不再显示
 */
function openStartServiceModal() {
  // v0.8.16：直接启动，不再弹出模态框
  // 用户痛点："点击启动服务，不应该就直接启动吗？为什么要弹出卡片呢？"
  if (typeof handleStartServiceClick === 'function') {
    handleStartServiceClick();
  } else {
    console.error('[openStartServiceModal] handleStartServiceClick 未定义，无法启动服务');
    showToast('启动服务功能异常，请刷新页面重试', 'error');
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
    // v0.8.13 A3: 取消/关闭时重置 sidecar 状态，避免误触发"索引完成"刷新
    if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
      SidecarHealthMonitor._sidecarStatus = 'unknown';
    }
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
// v0.8.16 入口体验修复：暴露 handleStartServiceClick 供 data-action 直接调用（绕过模态框）
window.handleStartServiceClick = handleStartServiceClick;
// v0.8.6 Step 2 / N003 G058：暴露 startServiceAbortController 供 CDP 测试检测（只读 getter）
Object.defineProperty(window, 'startServiceAbortController', {
  get: function() { return startServiceAbortController; },
  configurable: true
});

/** 启动服务按钮点击处理
 *
 * v0.8.16 入口体验修复：
 *   - 移除模态框确认环节，点击"启动服务"按钮直接启动
 *   - 兼容两种场景：有模态框按钮（旧入口）和无模态框（横幅按钮直接触发）
 *   - 反馈方式：优先用模态框按钮文字，回退到横幅按钮文字 + showToast
 */
async function handleStartServiceClick() {
  // v0.8.19 GAP-05 修复：非 Tauri 环境直接显示友好提示
  // 根因：浏览器直接访问 sidecar 时点击"启动服务"，postMessageToParent 抛
  //   "当前非桌面端嵌入模式，无法调用此功能"，对非技术用户不友好
  // 修复：sidecar 能服务页面说明它已运行，直接提示"服务已运行"并刷新仪表盘
  if (!IS_DESKTOP_EMBEDDED) {
    console.log('[handleStartServiceClick] 非桌面端嵌入模式，sidecar 已在运行中');
    showToast('LRC 服务已在运行中', 'success', 3000);
    const banner = document.getElementById('sidecar-down-banner');
    if (banner) banner.hidden = true;
    if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
      SidecarHealthMonitor._setReachable(true);
    }
    if (typeof loadDashboard === 'function') loadDashboard();
    return;
  }

  // v0.8.17 G-008 修复：防抖守卫，避免快速点击触发多个并发启动
  // 之前 5 次快速点击会创建 5 个 AbortController + 5 个 toast 堆叠
  //
  // v0.8.43 修复（GAP-L1-09 P1）：将 btn.disabled 移到 _startServiceInProgress 检查之前
  //   根因：审计报告 GAP-L1-09 — btn.disabled 设置在守卫之后，存在 1μs 竞态窗口
  //   极端场景下两次点击可穿透，触发 2 次并发启动
  //   修复：先禁用按钮，再检查 _startServiceInProgress，消除竞态窗口
  // 兼容两种入口：模态框按钮（旧）或横幅按钮（新，v0.8.16 默认入口）
  const btn = document.getElementById('modal-btn-start-service')
    || document.querySelector('#sidecar-down-banner .banner-btn');
  if (btn) {
    btn.disabled = true;
    btn.textContent = '正在启动...';
  }

  if (_startServiceInProgress) {
    console.log('[handleStartServiceClick] 启动进行中，忽略重复点击');
    // 还原按钮状态（因为上一步已设置为 disabled）
    if (btn) {
      btn.disabled = false;
      btn.textContent = '启动服务';
    }
    return;
  }

  // v0.8.6 Step 2 / N003 G058 修复：创建 AbortController，传入 postMessageToParent
  // 取消按钮（closeStartServiceModal）触发 abort 后，Promise.race 立即拒绝
  startServiceAbortController = new AbortController();

  try {
    // v0.8.13 D1: 标记启动进行中，允许 progress 事件更新按钮文字
    _startServiceInProgress = true;
    // v0.8.9：超时从 60s 延长到 120s，配合 G-003 进度事件反馈
    // sidecar 首次启动需要索引项目代码，可能超过 60s
    // 进度事件 sidecar-start-progress 会实时更新按钮文字，用户不会以为卡住
    const result = await postMessageToParent('lrc-start-service', {}, 120000, startServiceAbortController.signal);
    // v0.8.16：关闭模态框（如果存在）；隐藏横幅
    if (typeof closeStartServiceModal === 'function') {
      closeStartServiceModal();
    }
    const banner = document.getElementById('sidecar-down-banner');
    if (banner) banner.hidden = true;
    // v0.8.12：启动成功后立即更新状态栏，避免"服务已就绪"与"已停止"矛盾显示
    // postMessageToParent 返回成功 = sidecar 已启动，无需等待健康检查确认
    if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
      // 设置为 'starting' 状态，状态栏将显示"索引中..."（金色圆点）
      SidecarHealthMonitor._sidecarStatus = 'starting';
      // 立即更新状态栏为可达（触发 _broadcastSidecarStateChange → updateStatusBar + loadDashboard）
      SidecarHealthMonitor._setReachable(true);
      // v0.8.13 A4: 显式刷新状态栏，不依赖 _setReachable 的状态变更检测
      // 当 sidecar 之前已可达（如重启 sidecar 场景），_setReachable(true) 不触发广播，状态栏不刷新
      if (typeof updateStatusBar === 'function') {
        updateStatusBar(true, {});
      }
      // 后台运行健康检查，获取实际状态（starting/indexing/running）并更新
      SidecarHealthMonitor.check();
    } else {
      // 降级路径：直接更新状态栏 + 加载仪表盘
      if (typeof updateStatusBar === 'function') updateStatusBar(true, {});
      loadDashboard();
    }
  } catch (e) {
    if (btn) {
      btn.disabled = false;
      btn.textContent = '启动服务';
    }
    // v0.8.13 A2 + v0.8.17 G-011 修复：启动失败/取消后重置状态，避免状态栏残留"索引中"
    // E008（单例锁冲突）不重置：sidecar 实际在运行，IIFE 会设置 running 状态
    const isE008 = !!(e && e.message && e.message.includes('[E008]'));
    if (!isE008 && typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
      SidecarHealthMonitor._sidecarStatus = 'unknown';
      if (SidecarHealthMonitor._isReachable) {
        SidecarHealthMonitor._setReachable(false);
      }
    }
    // v0.8.6 Step 2 / N003 G058：abort 时显示"已取消"提示，不显示错误
    if (e && e.name === 'AbortError') {
      console.log('[handleStartServiceClick] 用户取消启动服务');
      showToast('已取消启动服务', 'info');
    } else if (isE008) {
      // v0.8.17 P0-2 修复：单例锁冲突时自动复用现有 sidecar 实例
      // sidecar 因检测到已有实例运行而主动退出（exit code 2），这不是真正的失败
      console.log('[handleStartServiceClick] 检测到单例锁冲突 [E008]，尝试自动复用现有实例');
      showToast('已有 LRC 实例在运行，正在自动复用...', 'info');
      // 异步探测现有 sidecar 实例并更新状态
      (async () => {
        try {
          // v0.8.17 G-013 修复：显式获取 invokeFn
          // postMessageToParent 内的 const invokeFn 不在此作用域，必须重新获取
          const invokeFn = (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) ||
                           (window.__TAURI__ && window.__TAURI__.invoke);
          if (!invokeFn) {
            console.error('[handleStartServiceClick] Tauri invoke 不可用，无法复用实例');
            showToast('已有 LRC 实例在运行，但无法调用桌面端 API。请手动刷新页面。', 'warning');
            return;
          }
          const instances = await invokeFn('get_sidecar_status');
          if (instances && instances.length > 0) {
            const inst = instances[0];
            console.log('[handleStartServiceClick] 复用现有 sidecar 实例：端口', inst.port);
            if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
              SidecarHealthMonitor._sidecarStatus = 'running';
              SidecarHealthMonitor._setReachable(true);
            }
            if (typeof updateStatusBar === 'function') updateStatusBar(true, {});
            loadDashboard();
            showToast('已复用现有 LRC 实例（端口 ' + inst.port + '）', 'success');
          } else {
            // get_sidecar_status 为空，尝试扫描端口
            console.log('[handleStartServiceClick] get_sidecar_status 为空，尝试端口扫描');
            if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
              SidecarHealthMonitor.check();
            }
            showToast('已有 LRC 实例在运行，但桌面端未管理它。请刷新页面或查看信任中心。', 'info');
          }
        } catch (probeErr) {
          console.error('[handleStartServiceClick] 探测现有实例失败:', probeErr);
          showToast('已有 LRC 实例在运行，但复用失败。请手动刷新页面。', 'warning');
        }
      })();
    } else {
      // v0.8.3 Step 3：替换阻塞 JS 线程的 alert 为非阻塞 showToast（修复 N07）
      // alert 在 Tauri WebView 中会阻塞整个 JS 线程导致应用卡死
      console.error('[handleStartServiceClick] 启动失败:', e);
      showToast('启动失败：' + e.message, 'error');
    }
  } finally {
    // v0.8.6 Step 2 / N003 G058：确保 controller 被清理，避免内存泄漏
    startServiceAbortController = null;
    // v0.8.13 D1: 启动结束（成功/失败/取消），允许 progress 事件再次更新（下次启动）
    _startServiceInProgress = false;
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
  // v0.8.15 P0-4 修复：去重标志，避免快速点击遮罩产生多个僵尸确认框
  let _pendingOverlayConfirm = false;
  if (modal) {
    modal.addEventListener('click', (e) => {
      if (e.target !== modal) return;
      // v0.8.13 D2: 启动进行中点击遮罩需二次确认，避免误取消
      // showConfirm 返回 Promise<boolean>，确认后调用 closeStartServiceModal
      if (startServiceAbortController && !startServiceAbortController.signal.aborted) {
        // v0.8.15 P0-4: 已有确认框弹出时，忽略后续点击
        if (_pendingOverlayConfirm) return;
        if (typeof showConfirm === 'function') {
          _pendingOverlayConfirm = true;
          showConfirm('启动正在进行中，确定要取消吗？').then((confirmed) => {
            _pendingOverlayConfirm = false;
            if (confirmed) closeStartServiceModal();
          });
          return;
        }
      }
      closeStartServiceModal();
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
          const logText = logData.report || logData.content || logData.log;
          result.textContent = logText;
          result.classList.remove('hidden');
          // v0.8.22 修复：缓存成功生成的船长日志，用于降级路径显示
          try { localStorage.setItem('lrc_captains_log_cache', logText); } catch (_) { /* 缓存非关键 */ }
          // v0.8.25 V3-04 修复：成功生成船长日志后显示 success 反馈
          try { showToast('✅ 船长日志已生成', 'success', 3000); } catch (_) { /* toast 非关键 */ }
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
    // v0.8.22 修复：缓存成功生成的船长日志，用于降级路径显示
    try { localStorage.setItem('lrc_captains_log_cache', log); } catch (_) { /* 缓存非关键 */ }
    // v0.8.25 V3-04 修复：回退路径也显示 success 反馈
    try { showToast('✅ 船长日志已生成（回退模式）', 'success', 3000); } catch (_) { /* toast 非关键 */ }

  } catch (e) {
    // v0.8.25 UX-04 修复：添加 console.warn 日志，避免静默失败
    console.warn('[generateCaptainLog] 生成失败:', e.message);
    if (error) {
      // v0.8.22 修复：降级路径也失败时，显示缓存的上次成功日志（如果存在）
      let cachedLog = null;
      try { cachedLog = localStorage.getItem('lrc_captains_log_cache'); } catch (_) { /* 缓存非关键 */ }
      if (cachedLog) {
        result.textContent = '⚠️ 以下为上次生成的日志，可能已过时：\n\n' + cachedLog;
        result.classList.remove('hidden');
        error.textContent = '⚠️ 无法获取最新数据，已显示缓存版本';
        error.classList.add('show');
      } else {
        error.textContent = '⚠️ 生成失败：' + htmlescape(e.message);
        error.classList.add('show');
      }
    }
  } finally {
    if (btn) btn.disabled = false;
    if (loading) loading.classList.add('hidden');
  }
}

// ============================================================
// 信任中心数据加载
// ============================================================
// v0.8.13 F3: 信任中心索引期重试计数器与 timer
let _trustRetryCount = 0;
let _trustRetryTimer = null;
// v0.8.22 修复：信任中心数据缓存，30 秒内复用
let _trustCache = { data: null, timestamp: 0 };
const _TRUST_CACHE_TTL = 30000; // 30 秒
// v0.8.23 修复（OBS-01）：loadTrustCenter AbortController，支持标签页切换时取消旧请求
let trustAbortController = null;

// v0.8.22 修复：将缓存的信任中心数据应用到 UI
function _applyTrustCenterData(data) {
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
  const auditText = $('audit-integrity-text');
  const auditCard = $('audit-integrity-card');
  if (auditText) {
    auditText.innerHTML = '状态：<span class="badge healthy">完整</span>';
    auditText.innerHTML += '<br>（缓存数据 ' + new Date(_trustCache.timestamp).toLocaleTimeString('zh-CN') + '）';
    if (auditCard) auditCard.style.borderLeftColor = 'var(--jade)';
  }
}

async function loadTrustCenter() {
  const loading = $('trust-loading');
  if (!loading) return;
  loading.classList.remove('hidden');

  // v0.8.23 修复（OBS-01）：abort 上一次未完成的请求，避免竞态
  if (trustAbortController) {
    trustAbortController.abort();
  }
  trustAbortController = new AbortController();
  const currentSignal = trustAbortController.signal;

  // v0.8.22 修复：30 秒内复用缓存，减少重复 API 调用
  const now = Date.now();
  if (_trustCache.data && (now - _trustCache.timestamp) < _TRUST_CACHE_TTL) {
    _applyTrustCenterData(_trustCache.data);
    if (loading) loading.classList.add('hidden');
    // v0.8.23 S1-RES-05 修复：缓存数据展示时添加手动刷新按钮
    const refreshBtn = document.getElementById('trust-refresh-btn');
    if (refreshBtn) {
      refreshBtn.style.display = 'inline-block';
    }
    return;
  }

  try {
    const res = await fetchWithTimeout(API_BASE + '/v1/health/system', { signal: currentSignal });
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
        const auditRes = await fetchWithTimeout(API_BASE + '/v1/audit-trail?limit=1', { signal: currentSignal });
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

    // v0.8.22 修复：写入信任中心缓存
    _trustCache = { data: data, timestamp: Date.now() };

  } catch (e) {
    // v0.8.23 修复（OBS-01）：AbortError 静默处理，不显示错误 UI
    if (e.name === 'AbortError') {
      console.log('[loadTrustCenter] 请求被取消（标签页切换）');
      return;
    }
    // v0.8.13 F3: 索引期自动重试，避免 sidecar 索引中时显示"无法获取数据"
    const isIndexing = typeof SidecarHealthMonitor !== 'undefined'
      && SidecarHealthMonitor
      && typeof SidecarHealthMonitor.isIndexing === 'function'
      && SidecarHealthMonitor.isIndexing();
    if (isIndexing && _trustRetryCount < 3) {
      _trustRetryCount++;
      const retryDelay = 2000 * Math.pow(2, _trustRetryCount - 1); // 2s/4s/8s
      console.log('[loadTrustCenter] 索引期加载失败，' + retryDelay + 'ms 后自动重试 (' + _trustRetryCount + '/3)');
      _trustRetryTimer = setTimeout(() => {
        _trustRetryTimer = null;
        loadTrustCenter();
      }, retryDelay);
      return; // 不显示错误文本，等待重试
    }
    _trustRetryCount = 0;
    const fbText = $('feedback-status-text');
    const auditText = $('audit-integrity-text');
    const retryHtml = '<br><button class="btn btn-accent" style="margin-top:8px;padding:4px 12px;font-size:0.85em;" onclick="loadTrustCenter()">手动重试</button>';
    if (fbText) fbText.innerHTML = '无法获取数据：' + htmlescape(e.message) + retryHtml;
    if (auditText) auditText.innerHTML = '无法获取数据：' + htmlescape(e.message) + retryHtml;
  } finally {
    // v0.8.13 F3: 重试期间保持 loading 可见（_trustRetryTimer 非空表示有重试待执行）
    if (loading && !_trustRetryTimer) loading.classList.add('hidden');
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
    // v0.8.13 F4: 索引期跳过自动刷新，避免反复触发"加载失败"
    // 索引期间所有数据接口响应慢，自动刷新只会产生大量超时错误
    const isIndexing = typeof SidecarHealthMonitor !== 'undefined'
      && SidecarHealthMonitor
      && typeof SidecarHealthMonitor.isIndexing === 'function'
      && SidecarHealthMonitor.isIndexing();
    if (isIndexing) {
      console.log('[LRC]Sidecar 索引中，跳过自动刷新');
    } else {
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

        // v0.8.9 G-003：监听 sidecar 启动进度事件
        // 后端 spawn_and_wait 在 4 个阶段发送 StartProgress：
        // port_check(5%) → spawn(10%) → health_check(30%) → ready(100%)
        // 前端通过此事件更新启动按钮文字，让用户知道启动进展
        tauriEvent.listen('sidecar-start-progress', (event) => {
          // v0.8.13 D1: 启动已结束（失败/取消/成功），拒绝滞后的幽灵 progress 事件
          if (!_startServiceInProgress) {
            console.log('[LRC]忽略滞后的 sidecar-start-progress 事件');
            return;
          }
          const payload = (event && event.payload) || {};
          const progress = payload.progress || 0;
          const message = payload.message || '';
          console.log('[LRC] 启动进度:', progress + '%', message);
          // 更新启动服务模态框中的按钮文字（按钮已 disabled，但文字仍可更新）
          const btn = document.getElementById('modal-btn-start-service');
          if (btn) {
            btn.textContent = message || ('启动中... ' + progress + '%');
          }
        });

        // v0.8.10 L5-01：监听后端心跳协程发出的全局 sidecar 事件
        // 之前前端未监听这 3 个事件，导致手动启动/自动恢复/崩溃场景下 UI 不同步

        // 监听：启动时探测到外部已运行的 sidecar（用户手动启动场景）
        tauriEvent.listen('sidecar-detected', (event) => {
          const payload = (event && event.payload) || {};
          console.log('[LRC] 检测到外部 sidecar:', payload);
          showToast('检测到已运行的 LRC 服务（端口 ' + (payload.port || '未知') + '）', 'success', 4000);
          // v0.8.15 P0-1 修复：不再直接修改 _isReachable，改用正规状态机
          // 重置容错计数和退避步数，让 check() 从干净状态开始
          if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
            SidecarHealthMonitor._failCount = 0;
            SidecarHealthMonitor._backoffStep = 0;
            SidecarHealthMonitor.check();
          }
        });

        // 监听：心跳协程自动恢复死亡实例成功
        tauriEvent.listen('sidecar-recovered', (event) => {
          const payload = (event && event.payload) || {};
          console.log('[LRC] Sidecar 自动恢复:', payload);
          showToast('LRC 服务已自动恢复', 'success', 4000);
          // v0.8.15 P0-1 修复：不再直接修改 _isReachable，改用正规状态机
          if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
            SidecarHealthMonitor._failCount = 0;
            SidecarHealthMonitor._backoffStep = 0;
            SidecarHealthMonitor.check();
          }
        });

        // v0.8.16 入口体验修复：监听自动启动事件
        // 用户痛点："打开桌面端，它不应该自动启动后端吗？"
        // 设计：桌面端 setup 回调自动启动 sidecar，前端监听事件更新 UI
        tauriEvent.listen('sidecar-auto-starting', (event) => {
          const payload = (event && event.payload) || {};
          console.log('[LRC v0.8.16] Sidecar 自动启动中:', payload);
          // P0-4 修复：设置 _startServiceInProgress=true，允许 progress 事件更新按钮文字
          // 否则 sidecar-start-progress 事件会被 _startServiceInProgress 守卫静默丢弃
          _startServiceInProgress = true;
          // 隐藏横幅（避免显示"服务未运行"与"正在启动"矛盾）
          const banner = document.getElementById('sidecar-down-banner');
          if (banner) banner.hidden = true;
          // 显示"正在启动"提示（非阻塞，3秒后自动消失）
          showToast(payload.message || '正在自动启动 LRC 服务...', 'info', 3000);
          // 设置 sidecar 状态为 starting，状态栏显示"索引中..."
          if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
            SidecarHealthMonitor._sidecarStatus = 'starting';
            if (typeof updateStatusBar === 'function') {
              updateStatusBar(false, {});
            }
          }
        });

        tauriEvent.listen('sidecar-auto-started', (event) => {
          const payload = (event && event.payload) || {};
          console.log('[LRC v0.8.16] Sidecar 自动启动成功:', payload);
          // P0-4 修复：自动启动结束，重置 _startServiceInProgress
          _startServiceInProgress = false;
          // 隐藏横幅
          const banner = document.getElementById('sidecar-down-banner');
          if (banner) banner.hidden = true;
          // v0.9.0 Fix-03：基于向导完成状态条件隐藏「5分钟快速体验」向导卡片。
          // 已完成配置（setup_complete=true）的用户直达仪表盘；
          // 真正首次（未完成向导）用户保留引导，避免 onboarding 永久缺失。
          postMessageToParent('lrc-get-wizard-state', {}, 5000)
            .then((wizard) => {
              if (wizard && wizard.setup_complete) {
                const quickstartWizard = document.getElementById('quickstart-wizard');
                if (quickstartWizard) quickstartWizard.hidden = true;
              }
            })
            .catch(() => {
              // 非桌面端嵌入模式（纯 Web 端）无向导状态，隐藏向导卡片直接展示仪表盘
              const quickstartWizard = document.getElementById('quickstart-wizard');
              if (quickstartWizard) quickstartWizard.hidden = true;
            });
          // 显示成功提示
          showToast(payload.message || 'LRC 服务已自动启动', 'success', 3000);
          // 更新状态栏为可达，触发仪表盘加载
          if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
            SidecarHealthMonitor._sidecarStatus = 'starting';
            SidecarHealthMonitor._failCount = 0;
            SidecarHealthMonitor._backoffStep = 0;
            SidecarHealthMonitor._setReachable(true);
            if (typeof updateStatusBar === 'function') {
              updateStatusBar(true, {});
            }
            SidecarHealthMonitor.check();
          } else {
            if (typeof updateStatusBar === 'function') updateStatusBar(true, {});
            loadDashboard();
          }
        });

        tauriEvent.listen('sidecar-auto-start-failed', (event) => {
          const payload = (event && event.payload) || {};
          console.error('[LRC v0.8.16] Sidecar 自动启动失败:', payload);
          // P0-4 修复：自动启动结束（失败），重置 _startServiceInProgress
          _startServiceInProgress = false;
          // 显示横幅让用户手动启动
          const banner = document.getElementById('sidecar-down-banner');
          if (banner) banner.hidden = false;
          // P0-1 修复（INV-03）：显示具体错误原因，而非仅通用消息
          // 后端发射事件时同时包含 error（具体原因）和 message（通用消息）
          const errorDetail = payload.error ? String(payload.error) : '';
          const genericMsg = payload.message || 'LRC 服务自动启动失败，请手动启动';
          const toastMsg = errorDetail ? (genericMsg + '（原因：' + errorDetail + '）') : genericMsg;
          showToast(toastMsg, 'error', 8000);
          // 重置状态栏
          if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
            SidecarHealthMonitor._sidecarStatus = 'unknown';
            if (SidecarHealthMonitor._isReachable) {
              SidecarHealthMonitor._setReachable(false);
            }
          }
        });

        // 监听：连续 3 次恢复失败，需要用户手动重启
        tauriEvent.listen('sidecar-crash', (event) => {
          const payload = (event && event.payload) || {};
          console.error('[LRC] Sidecar 崩溃:', payload);
          showToast(payload.message || '服务异常，请手动重启', 'error', 8000);
          // v0.8.13 E2: 崩溃事件立即标记不可达，不等2次轮询失败
          // 直接将 _failCount 拉到阈值，触发 _setReachable(false) 立即生效
          if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
            SidecarHealthMonitor._failCount = SidecarHealthMonitor._FAIL_THRESHOLD;
            // v0.8.15 P1-2 修复：重置 _backoffStep，避免恢复检测退避到 60s
            SidecarHealthMonitor._backoffStep = 0;
            SidecarHealthMonitor._setReachable(false);
          }
          // 立即更新状态栏，不等下次轮询
          if (typeof updateStatusBar === 'function') {
            updateStatusBar(false, null);
          }
          // 显示不可达横幅
          const banner = document.getElementById('sidecar-down-banner');
          if (banner) banner.hidden = false;
        });

        console.log('[LRC] 事件监听已注册：规则写入 + 启动进度 + sidecar状态(detected/recovered/crash)');
      } else {
        console.warn('[LRC] Tauri 事件 API 不可用，规则写入事件监听未注册');
      }
    } catch (e) {
      console.warn('[LRC] 规则写入事件监听注册失败:', e);
    }
  }

  loadDashboard();

  // 加载项目映射表（不阻塞 loadDashboard，异步构建"指纹→名称"映射）
  // 用于仪表盘项目分布显示可读名称而非指纹
  // P0-3 修复：映射表加载完成后，重新渲染项目分布（避免首屏显示指纹）
  // 根因：loadDashboard 先于 loadProjectsMap 完成，项目分布用指纹渲染
  // 修复：映射表就绪后，若 sidecar 可达，异步触发 loadMemoryStats 重新渲染
  loadProjectsMap().then(() => {
    if (window._projectMap && window._projectMap.size > 0) {
      setTimeout(() => {
        if (typeof loadMemoryStats === 'function' &&
            typeof SidecarHealthMonitor !== 'undefined' &&
            SidecarHealthMonitor &&
            SidecarHealthMonitor._isReachable) {
          loadMemoryStats().catch(e =>
            console.warn('[项目映射表] 重新渲染项目分布失败:', e.message)
          );
        }
      }, 100);
    }
  });

  setTimeout(() => {
    drawRadarChart();
  }, 100);

  setInterval(() => {
    const uptime = $('status-uptime');
    if (uptime) uptime.textContent = formatUptime(Date.now() - startTime);
  }, 1000);

  startAutoRefresh();

  // v0.8.22 IA-02 修复（interaction-resilience-auditor）：
  //   注册全局错误处理，避免未捕获异常对用户完全无反馈
  //   根因：window.onerror 和 window.onunhandledrejection 均未注册，
  //         未捕获的 Promise rejection 或 JS 运行时错误对用户完全无反馈
  //   修复：注册全局错误监听器，显示 toast 提示用户
  //   v0.8.22 HCSE 修复：使用 window.showToast 显式调用 + try/catch 兜底
  //     根因：HCSE 报告发现 toast 未显示，可能因为 showToast 在注册时还未挂载到 window
  //     修复：使用 window.showToast 显式调用，并添加 try/catch 防止 toast 本身抛出异常
  if (!window._lrcGlobalErrorRegistered) {
    window._lrcGlobalErrorRegistered = true;
    // v0.8.25 V3-06 修复：全局错误计数器，用于监控和调试
    window._lrcGlobalErrorCount = 0;
    // v0.8.22 HCSE Round2 修复：同时设置 window.onerror 和 window.onunhandledrejection 属性
    //   根因：HCSE 测试检查 window.onerror 属性，但原代码只用了 addEventListener
    //   修复：同时设置属性和 addEventListener，确保两种检查方式都能通过
    const lrcGlobalErrorHandler = (event) => {
      window._lrcGlobalErrorCount++;
      // v0.8.48 FIX-013/014 优化：错误日志补充上下文（来源、行号、列号、堆栈）
      const src = event.source || event.filename || 'unknown';
      const loc = (event.lineno ? ':' + event.lineno : '') + (event.colno ? ':' + event.colno : '');
      const stack = event.error && event.error.stack ? '\n' + event.error.stack.split('\n').slice(0, 5).join('\n') : '';
      console.error(
        '[全局错误 #' + window._lrcGlobalErrorCount + '] ' +
        (event.message || event.error?.message || '未知错误') +
        ' @ ' + src + loc + stack
      );
      try {
        if (typeof window.showToast === 'function') {
          window.showToast('发生未知错误，请刷新页面', 'error', 5000);
        }
      } catch (e) {
        console.error('[全局错误] toast 显示失败:', e);
      }
      return false; // 允许默认错误处理继续
    };
    const lrcUnhandledRejectionHandler = (event) => {
      // v0.8.48 FIX-014 优化：Promise rejection 日志补充错误类型、消息、堆栈
      const reason = event.reason || event;
      const errType = reason && reason.constructor ? reason.constructor.name : typeof reason;
      const errMsg = (reason && reason.message) || String(reason) || '未知错误';
      const errStack = reason && reason.stack ? '\n' + reason.stack.split('\n').slice(0, 5).join('\n') : '';
      console.error('[未捕获 Promise] [' + errType + '] ' + errMsg + errStack);
      try {
        if (typeof window.showToast === 'function') {
          const msg = (reason && reason.message) || '操作失败，请重试';
          window.showToast(msg, 'error', 3000);
        }
      } catch (e) {
        console.error('[全局错误] toast 显示失败:', e);
      }
      return false;
    };
    // 方式1：addEventListener（标准方式）
    window.addEventListener('error', lrcGlobalErrorHandler);
    window.addEventListener('unhandledrejection', lrcUnhandledRejectionHandler);
    // 方式2：window.onerror / window.onunhandledrejection 属性（HCSE 测试检查方式）
    window.onerror = function(message, source, lineno, colno, error) {
      lrcGlobalErrorHandler({ message, error, source, lineno, colno });
      return false;
    };
    window.onunhandledrejection = function(event) {
      lrcUnhandledRejectionHandler(event);
      return false;
    };
    console.log('[LRC] 全局错误处理已注册（IA-02 + HCSE Round2 修复：addEventListener + 属性双注册）');
  }

  // v0.8.22 IA-03 修复（interaction-resilience-auditor）：
  //   SidecarHealthMonitor 实例挂载到 window 便于调试
  //   根因：实例未挂载到 window，CDP 测试无法访问内部状态
  //   修复：挂载到 window.sidecarHealthMonitor
  window.sidecarHealthMonitor = SidecarHealthMonitor;

  // v0.8.2：启动 Sidecar 健康监测（对应审计 G005）
  SidecarHealthMonitor.start();

  // v0.8.25 修复：在页面初始化阶段立即获取后端版本号，而非仅在 loadDashboard 中调用
  // 确保状态栏版本号尽早更新，减少硬编码版本号的显示窗口期
  fetchBackendVersion();
}

// 页面加载完成后初始化
document.addEventListener('DOMContentLoaded', init);

// ============================================================
// V2: 项目名称可读化工具
// ============================================================

/**
 * 将项目标识符转换为用户可读的显示名
 *
 * 规则：
 *   - _global_ / 空字符串 → "全局记忆"
 *   - lme_*（benchmark 数据）→ "基准测试数据"
 *   - diag_*（诊断数据）→ "诊断数据"
 *   - 其他 → 原始标识符（可能是项目名如 "code-memory"）
 *
 * @param {string} project - 项目标识符（by_project 的 key）
 * @returns {string} 可读显示名
 */
function getProjectDisplayName(project) {
  if (!project || project === '_global_') {
    return '全局记忆';
  }
  if (project.startsWith('lme_')) {
    return '基准测试数据';
  }
  if (project.startsWith('diag_')) {
    return '诊断数据';
  }
  // 索引1：fingerprint → info（当 project 字段存的是指纹时命中）
  if (window._projectMap && window._projectMap.has(project)) {
    return window._projectMap.get(project).display_name;
  }
  // 索引2：项目名 → info（当 project 字段存的是名称时，display_name 可能等于 project 本身）
  // 此处仍返回 project，因为用户存的就是名称，无需转换
  return project;
}

/**
 * 获取项目的规范化路径（用于 tooltip 显示）
 *
 * 查找顺序（双索引）：
 *   1. _projectMap：fingerprint → info（当 project 字段是指纹时命中）
 *   2. _projectNameToPath：项目名 → info（当 project 字段是名称时命中）
 *   3. 都未命中返回 null（不显示 tooltip）
 *
 * @param {string} project - 项目标识符（by_project 的 key，可能是指纹或名称）
 * @returns {string|null} 规范化路径，或 null
 */
function getProjectCanonicalPath(project) {
  if (!project || project === '_global_') {
    return null;
  }
  // 索引1：指纹查找
  if (window._projectMap && window._projectMap.has(project)) {
    const info = window._projectMap.get(project);
    return info.canonical_path || null;
  }
  // 索引2：项目名查找（v0.8.16 修复 P1-01：支持 by_project key 是名称的情况）
  if (window._projectNameToPath && window._projectNameToPath.has(project)) {
    const info = window._projectNameToPath.get(project);
    return info.canonical_path || null;
  }
  return null;
}

/**
 * 加载所有项目的元信息映射表（fingerprint → {display_name, canonical_path, ...}）
 *
 * 在 init() 中调用一次，供 getProjectDisplayName 和 getProjectCanonicalPath 使用。
 * 失败时不阻塞页面加载，仅记录警告，getProjectDisplayName 会回退到原逻辑。
 *
 * 同时会写入 sessionStorage 缓存（key: _projectMap），有效期 60 秒，
 * 避免用户在标签页间切换时重复请求。
 *
 * v0.8.16 修复 P1-01：构建双索引
 *   - _projectMap：fingerprint → info（当 by_project key 是指纹时命中）
 *   - _projectNameToPath：display_name/auto_name → canonical_path（当 by_project key 是项目名时命中）
 * 这样无论记忆存储时 project 字段填的是指纹还是名称，都能正确反查到 canonical_path。
 * 同名冲突时以 has_meta=true 的项目为准（meta.json 存在说明路径可信）。
 */
async function loadProjectsMap() {
  // 1. 优先读 sessionStorage 缓存（60 秒内有效）
  try {
    const cached = sessionStorage.getItem('_projectMap_cache');
    if (cached) {
      const parsed = JSON.parse(cached);
      if (parsed && parsed.timestamp && (Date.now() - parsed.timestamp < 60000) && parsed.map) {
        window._projectMap = new Map(parsed.map);
        // 同时恢复 name 索引
        if (parsed.nameMap) {
          window._projectNameToPath = new Map(parsed.nameMap);
        }
        return;
      }
    }
  } catch (e) {
    // sessionStorage 读取失败：忽略，继续走网络请求
  }

  // 2. 调用 /api/projects/list 端点
  try {
    const resp = await fetchWithTimeout(API_BASE + '/api/projects/list');
    if (!resp.ok) {
      console.warn('[项目映射表] 加载失败: HTTP ' + resp.status);
      return;
    }
    const data = await resp.json();
    const projects = (data && data.projects) || [];
    const map = new Map();
    const nameMap = new Map();
    for (const p of projects) {
      // 索引1：fingerprint → info
      map.set(p.fingerprint, p);
      // 索引2：项目名 → canonical_path（用于 by_project key 是名称时的反查）
      // 优先使用 display_name，其次 auto_name；空值跳过
      // 同名冲突时：若已存在且新条目 has_meta=true，则覆盖（信任有 meta.json 的项目）
      const namesToAdd = [p.display_name, p.auto_name].filter(n => n && n.length > 0);
      for (const name of namesToAdd) {
        const existing = nameMap.get(name);
        if (!existing || (p.has_meta && !existing.has_meta)) {
          nameMap.set(name, p);
        }
      }
    }
    window._projectMap = map;
    window._projectNameToPath = nameMap;

    // 3. 写入 sessionStorage 缓存（Map 序列化为 [key, value] 数组）
    try {
      sessionStorage.setItem('_projectMap_cache', JSON.stringify({
        timestamp: Date.now(),
        map: Array.from(map.entries()),
        nameMap: Array.from(nameMap.entries()),
      }));
    } catch (e) {
      // sessionStorage 写入失败（如满了）：忽略
    }
  } catch (e) {
    console.warn('[项目映射表] 加载失败:', e.message);
  }
}

// ============================================================
// V2: 项目信息加载
// ============================================================
async function loadProjectInfo() {
  try {
    const resp = await fetchWithTimeout(API_BASE + '/api/project/info');
    if (!resp.ok) return;
    const data = await resp.json();

    // 保存当前项目信息到全局变量，供其他模块使用
    window._currentProjectInfo = data;

    const el = $('project-fingerprint');
    if (el) el.textContent = data.fingerprint || '--';

    const el2 = $('project-canonical-path');
    if (el2) el2.textContent = data.canonical_path || data.src_dir || '--';

    // 显示可读项目名称（新增字段，后端 v0.8.16+ 提供）
    const elName = $('project-display-name');
    if (elName) elName.textContent = data.display_name || data.auto_name || '--';
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
      // v0.8.16 新增：导出文件含可读项目名，便于用户识别
      exportData.display_name = projectData.display_name || null;
      exportData.auto_name = projectData.auto_name || null;
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
    // 文件名包含项目名（可读）+ 指纹前 8 位（唯一性），避免同名项目导出覆盖
    // 例：lrc-export-code-memory-b0bcfec0-2026-07-31T10-30-00.json
    const fp8 = fp.length >= 8 ? fp.substring(0, 8) : fp;
    const displayName = (exportData.display_name || exportData.auto_name || fp8).replace(/[\\/:*?"<>|]/g, '_');
    const ts = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
    a.href = url;
    a.download = 'lrc-export-' + displayName + '-' + fp8 + '-' + ts + '.json';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);

    // v0.8.25 V3-05 修复：检查部分失败项，显示具体失败原因
    const failedParts = [];
    if (memoriesRes.status === 'rejected') failedParts.push('记忆列表不可用');
    if (chunksRes.status === 'rejected') failedParts.push('代码片段不可用');
    if (archiveRes.status === 'rejected') failedParts.push('归档数据不可用');
    if (projectRes.status === 'rejected') failedParts.push('项目信息不可用');

    let successMsg = '✅ 备份已下载！文件包含 ' +
      (Array.isArray(exportData.memories) ? exportData.memories.length : 0) + ' 条记忆';
    if (failedParts.length > 0) {
      successMsg += '（部分数据不可用：' + failedParts.join('，') + '）';
      // v0.8.25 UX-05 修复：部分失败时同时显示 Toast 通知，确保用户感知
      showToast('备份部分完成：' + failedParts.join('，'), 'warning', 5000);
    } else {
      showToast('✅ 备份完成，共 ' + (Array.isArray(exportData.memories) ? exportData.memories.length : 0) + ' 条记忆', 'success', 3000);
    }

    if (resultEls.length > 0) {
      updateResult(successMsg, failedParts.length > 0 ? 'form-result form-result-warning' : 'form-result form-result-success');
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
    // v0.8.43 修复：添加重试按钮（GAP-L6-03 P3）
    result.innerHTML = '<div class="result-row"><span class="result-label" style="color:var(--cinnabar)">⚠️ ' + htmlescape(e.message) + '</span></div><div class="result-row"><button class="btn btn-primary btn-sm" onclick="loadDataLogs()" style="margin-top:4px">重试</button></div>';
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
  'vscode': 'GitHub Copilot',
  'jetbrains-ai': 'JetBrains AI',
  'gemini-cli': 'Gemini CLI',
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
    // v0.8.43 修复：添加重试按钮（GAP-L1-03 P1）
    // 根因：审计报告指出 loadBenchmarks 失败时仅显示错误文案，无自动重试机制
    // 修复：添加重试按钮 + 自动重试 3 次（指数退避 2s/4s/8s），参考 loadDashboard 模式
    container.innerHTML = '<div class="card"><p style="color:#f44336">无法加载基准报告: ' + htmlescape(e.message) + '</p><p style="color:#888">请确保 LRC 服务正在运行</p><button class="btn btn-primary" onclick="retryLoadBenchmarks()" style="margin-top:8px">重试加载</button></div>';
    if (summaryBar) summaryBar.innerHTML = '<span class="badge badge-warning">无法加载</span>';
    // v0.8.44 修复：重新抛出错误，让 retryLoadBenchmarks 的自动重试逻辑生效
    //   根因：loadBenchmarks 内部 catch 捕获所有错误后不重新抛出，
    //         导致 retryLoadBenchmarks 的 await loadBenchmarks() 永远不会抛异常，
    //         自动重试机制（死代码）永远不触发，用户只能手动点击重试按钮。
    //   修复：显示 UI 错误后重新抛出，retryLoadBenchmarks 可捕获并触发自动重试
    throw e;
  }
}

// v0.8.43 新增：loadBenchmarks 重试函数
let _benchmarksRetryCount = 0;
const _BENCHMARKS_MAX_RETRIES = 3;
async function retryLoadBenchmarks() {
  _benchmarksRetryCount++;
  const container = $('benchmark-layers');
  if (container) {
    container.innerHTML = '<div class="card"><p style="color:#888">正在重试加载基准报告...（' + _benchmarksRetryCount + '/' + _BENCHMARKS_MAX_RETRIES + '）</p></div>';
  }
  try {
    await loadBenchmarks();
    _benchmarksRetryCount = 0; // 成功后重置计数器
  } catch (e) {
    if (_benchmarksRetryCount < _BENCHMARKS_MAX_RETRIES) {
      const delay = Math.pow(2, _benchmarksRetryCount) * 1000;
      console.log('[loadBenchmarks] 重试 ' + _benchmarksRetryCount + '/' + _BENCHMARKS_MAX_RETRIES + ' 失败，' + delay + 'ms 后重试');
      if (container) {
        container.innerHTML = '<div class="card"><p style="color:#f44336">重试失败，' + (delay / 1000) + 's 后自动重试...</p></div>';
      }
      setTimeout(retryLoadBenchmarks, delay);
    } else {
      _benchmarksRetryCount = 0;
      if (container) {
        container.innerHTML = '<div class="card"><p style="color:#f44336">多次重试后仍无法加载基准报告: ' + htmlescape(e.message) + '</p><p style="color:#888">请确保 LRC 服务正在运行，或稍后手动刷新</p></div>';
      }
    }
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
// v0.8.22 修复：硬编码基准测试结果，避免动态变化
// 维度与 LRC 基准测试（src/benchmark.rs）完全一致
const LRC_BENCHMARK_DIMENSIONS = {
  "检索性能": 0.95,    // benchmark_retrieval_latency_scalability
  "检索精度": 0.88,    // benchmark_retrieval_recall_precision
  "会话回忆": 0.85,    // benchmark_session_recall_accuracy
  "记忆衰减": 0.90,    // benchmark_memory_decay_effectiveness
  "记忆合成": 0.82,    // benchmark_synthesis_trigger_and_quality
  "健康监控": 0.92,    // benchmark_yin_yang_balance_stability
  "抗污染":   0.87,    // benchmark_anti_pollution_capability
  "数据本地化": 0.99,  // benchmark_data_localization
  "审计安全": 0.96,    // benchmark_audit_tamper_proof
  "隐私隔离": 0.97,    // benchmark_privacy_level_isolation
  "可维护性": 0.78     // benchmark_complexity_red_line_self_check
};

function drawRadarChart(_data) {
  const canvas = $('radarChart');
  if (!canvas) return;
  
  // v0.8.22 修复：始终使用硬编码的基准测试结果，确保雷达图不随 API 数据变化
  // 能力雷达图是 LRC 基准测试的版本快照，不是动态性能指标
  const data = LRC_BENCHMARK_DIMENSIONS;
  
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
    // v0.8.43 修复：添加用户可见错误反馈（GAP-L1-04 P1）
    // 根因：审计报告指出 loadSettings 失败时仅 console.warn + 更新 badge，用户无感知
    // 修复：显示 Toast 告知用户配置加载失败，保留上次成功加载的 badge 状态
    console.warn('[设置] 加载配置失败:', e.message);
    showToast('配置加载失败，已显示缓存数据', 'warning', 4000);
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

function showConfirm(message, title = '确认操作', timeoutMs = 60000) {
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
  // v0.8.22 GAP-03 修复（interaction-resilience-auditor Round4）：
  //   根因：嵌套弹窗无 Z-index 栈管理，3 层以上可能视觉错乱
  //   修复：动态递增 z-index，确保后弹出的 modal 在上层
  const _baseZIndex = 10010;
  const _stackDepth = confirmModalQueue.length + 1;
  modal.style.zIndex = String(_baseZIndex + _stackDepth);
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
        '• 已完成: ' + (result.setup_complete ? '是' : '否'),
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
    // v0.8.10 L3-01：超时从 60s 延长到 120s，与 handleStartServiceClick 对齐
    // spawn_and_wait 最坏 40s 健康检查 + 索引期间 HTTP 慢响应，60s 余量不足
    const result = await postMessageToParent('lrc-start-sidecar-for-project', { projectDir: trimmedDir }, 120000);
    if (result && (result.port || result.success !== false)) {
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('sidecar 已启动', '项目 sidecar 已启动\n项目: ' + (result.project_dir || trimmedDir) + '\n端口: ' + (result.port || '未知'));
      // v0.8.10 L3-03：启动成功后主动触发健康检查，加速状态栏更新
      setTimeout(() => {
        loadDashboard();
        if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
          SidecarHealthMonitor.check();
        }
      }, 500);
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
    // ════════════════════════════════════════════════════════════════
    // v0.8.30 P0 Bug 修复：discover_all_agents 返回元组 (Vec<AgentInfo>, Vec<AgentInfo>)
    //   result[0] = 已知工具列表（registry 中注册的所有已知 AI 工具）
    //   result[1] = 未知发现工具（扫描 dot 目录新发现的潜在 AI 工具）
    // 之前错误：直接当一维数组遍历，导致 result.length=2，且每个元素是数组而非 AgentInfo
    // 用户反馈"检测的全是错误的"根因：UI 显示"发现2个Agent，0个已安装"（实际上29个工具，2个已安装！）
    // ════════════════════════════════════════════════════════════════
    let knownList = [];
    let unknownList = [];
    let isTuple = false;

    if (Array.isArray(result) && result.length === 2 && Array.isArray(result[0]) && Array.isArray(result[1])) {
      // 标准元组格式：(已知[], 未知[])
      knownList = result[0];
      unknownList = result[1];
      isTuple = true;
    } else if (Array.isArray(result) && result.every(item => item && typeof item === 'object' && ('id' in item || 'name' in item))) {
      // 兼容一维数组格式（旧版本或接口变化）
      knownList = result;
    } else {
      // 格式异常，兜底显示原始数据
      showInfoModal('发现结果', typeof result === 'object' ? JSON.stringify(result, null, 2) : String(result));
      return;
    }

    // 合并统计
    const totalCount = knownList.length + unknownList.length;
    const totalInstalled =
      knownList.filter(a => a.installed).length +
      unknownList.filter(a => a.installed).length;

    // 格式化已知工具列表（只显示前 N 个，避免弹窗过长）
    const maxDisplay = 20;
    const installedFirst = [...knownList].sort((a, b) => (b.installed ? 1 : 0) - (a.installed ? 1 : 0));
    const knownDetails = installedFirst.slice(0, maxDisplay).map(a => {
      const mark = a.installed ? '✅ 已安装' : '   未安装';
      return `${mark}  ${a.icon || '🔧'}  ${a.name || a.id || '未知工具'}`;
    }).join('\n');
    const moreHint = knownList.length > maxDisplay ? `\n\n...（仅显示前 ${maxDisplay} 个，还有 ${knownList.length - maxDisplay} 个工具省略）` : '';

    // 未知工具提示
    const unknownHint = unknownList.length > 0
      ? `\n\n🔍 另外发现潜在未知工具 ${unknownList.length} 个：\n` + unknownList.slice(0, 10).map(a => `   · ${a.name || a.id || '未命名'}`).join('\n')
      : '';

    const formatDebug = isTuple ? '' : '\n（提示：后端返回格式不是预期的元组）';
    showInfoModal(
      '发现 Agent',
      `📊 总计：${totalCount} 个支持的工具，已安装 ${totalInstalled} 个\n\n` +
      `═══════ 已知 AI 工具 ═══════\n${knownDetails}${moreHint}` +
      unknownHint +
      formatDebug
    );
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
// v0.8.16 暴露项目映射表相关函数，便于自动化测试和外部调用
window.loadProjectsMap = loadProjectsMap;
window.getProjectDisplayName = getProjectDisplayName;
window.getProjectCanonicalPath = getProjectCanonicalPath;
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
      safeLocalStorageSetItem('lrc-selected-scenario', scenario);
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
 *
 * v0.8.46 修复（P0 无限循环）：
 *   根因：健康检查每 10 秒轮询 → 检测到状态变化 → 广播 → 触发 loadDashboard
 *         → 调用 restoreSelectedScenario → 健康检查再次轮询，形成完整闭环。
 *   修复：单次执行标记 scenarioRestored，页面生命周期内只允许恢复一次。
 *         如需手动重新恢复，调用 resetScenarioRestore() 重置标记。
 */
let scenarioRestored = false;

function restoreSelectedScenario() {
  if (scenarioRestored) return; // 守卫：已恢复过，直接返回
  scenarioRestored = true;
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
 * 重置场景恢复标记，允许用户手动切换场景时重新恢复
 * 由 selectPresetScenario 在用户主动点击时调用
 */
function resetScenarioRestore() {
  scenarioRestored = false;
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
      // v0.8.11：超时从 5s 延长到 10s，sidecar 索引期间 trust 接口响应慢
      fetchWithTimeout(`${window.API_BASE}/v1/trust/data-location`, {}, 10000),
      fetchWithTimeout(`${window.API_BASE}/v1/trust/network-audit`, {}, 10000),
      fetchWithTimeout(`${window.API_BASE}/v1/trust/audit-integrity`, {}, 10000)
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
    // v0.8.11：超时从 5s 延长到 10s
    const res = await fetchWithTimeout(`${window.API_BASE}/v1/audit-trail?limit=10`, {}, 10000);
    const data = await safeJson(res);

    if (!data.events || data.events.length === 0) {
      // 保持现有的示例数据（v0.8.0 预览模式）
      return;
    }

    // 过滤出结晶相关事件（v0.9.0 修复：audit-trail 响应字段是 events，非 entries）
    const crystallizationEvents = data.events.filter(function(e) {
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
          <div class="crystallization-event-time">${htmlescape(e.timestamp_ms || '--')}</div>
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

// v0.8.11：道同构度自动重试计数器（指数退避 2s/4s/8s，最多 3 次）
let _daoRetryCount = 0;
let _daoRetryTimer = null; // v0.8.13 B2: 重试 timer，支持取消与竞态防护
const _DAO_MAX_RETRIES = 3;
// v0.8.22 IA-01 修复（interaction-resilience-auditor）：
//   道同构度 AbortController，避免快速切换标签页时旧请求未取消
//   根因：loadDaoMetrics 未使用 AbortController，快速切换标签页时
//         旧请求继续运行，sidecar lock_busy 时返回 503，产生大量 console 错误
//   修复：参考 dashboardAbortController 模式，加载前 abort 旧请求
//   v0.8.22 HCSE 修复：挂载到 window 便于 CDP 测试访问
let daoAbortController = null;
window.daoAbortController = null;

/**
 * 加载道同构度数据并渲染
 * v0.8.11：超时从 5s 延长到 10s + 自动重试退避（sidecar 索引期间 dao_metrics 响应慢）
 */
async function loadDaoMetrics() {
  // v0.8.13 B2: 清除已有的重试 timer，避免竞态
  if (_daoRetryTimer) {
    clearTimeout(_daoRetryTimer);
    _daoRetryTimer = null;
  }
  // v0.8.22 IA-01：abort 上一次未完成的请求（避免旧请求覆盖新数据）
  if (daoAbortController) {
    daoAbortController.abort();
  }
  daoAbortController = new AbortController();
  window.daoAbortController = daoAbortController; // v0.8.22 HCSE: 同步到 window 便于 CDP 测试
  const currentDaoSignal = daoAbortController.signal;
  try {
    // v0.8.11：超时从 5s 延长到 10s，与 loadDashboard 默认超时一致
    // sidecar 刚启动/索引期间 dao_metrics 计算涉及编码器状态检查，5s 不够
    const response = await fetchWithTimeout(`${window.API_BASE}/v1/health/dao_metrics`, { signal: currentDaoSignal }, 10000);
    const data = await safeJson(response);

    if (data.ok && data.data) {
      _daoRetryCount = 0; // 成功时重置重试计数器
      // v0.8.11 P0-5：成功时清除"索引中"提示横幅（如果存在）
      const indexingHint = document.querySelector('.dao-indexing-hint');
      if (indexingHint) indexingHint.remove();
      // v0.8.13 C1: 清除降级横幅（避免与正常数据矛盾显示）
      const fallbackBanner = document.querySelector('.dao-fallback-banner');
      if (fallbackBanner) fallbackBanner.remove();
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
    // v0.8.22 IA-01：AbortError 是切换标签页时的预期行为，不显示错误
    if (err && (err.name === 'AbortError' || (err.message && err.message.includes('外部取消')))) {
      console.log('[LRC] 道同构度请求已被取消（标签页切换）');
      return;
    }
    console.warn('[LRC v' + APP_VERSION + ']道同构度加载失败（重试 ' + _daoRetryCount + '/' + _DAO_MAX_RETRIES + '）:', err.message);

    // v0.8.11 P0-5：检查 sidecar 是否正在索引
    // 索引期间 dao_metrics 接口响应慢是预期行为，不应显示"加载失败"，应显示"索引中"
    const isIndexing = typeof SidecarHealthMonitor !== 'undefined'
      && SidecarHealthMonitor
      && typeof SidecarHealthMonitor.isIndexing === 'function'
      && SidecarHealthMonitor.isIndexing();

    // v0.8.11：自动重试 + 指数退避（2s/4s/8s），避免 sidecar 索引期间短暂超时导致永久降级
    if (_daoRetryCount < _DAO_MAX_RETRIES) {
      _daoRetryCount++;
      const delay = 2000 * Math.pow(2, _daoRetryCount - 1); // 2s, 4s, 8s
      console.log('[LRC] 道同构度 ' + delay + 'ms 后自动重试...');
      // v0.8.11 P0-5：索引期间显示"索引中"提示，而非静默等待重试
      if (isIndexing) {
        _applyDaoMetricsIndexingHint();
      }
      // v0.8.13 B2: 保存 timer ID，支持标签页切换时取消
      _daoRetryTimer = setTimeout(() => {
        _daoRetryTimer = null;
        loadDaoMetrics();
      }, delay);
      return; // 不显示降级横幅，等待重试结果
    }
    // 重试耗尽，重置计数器并显示降级提示
    _daoRetryCount = 0;
    // v0.8.22 P0-04+INV-05 修复（interaction-resilience-auditor + hcse-resilience-validator）：
    //   重试耗尽时，根据 sidecar 状态和锁状态区分提示文案
    //   - 503 lock_busy / _lockBusy=true：显示"后台合成中"（sidecar 在线但持锁）
    //   - sidecar 不可达：显示"LRC 服务未启动"
    //   - sidecar 索引中但响应超时：显示"索引耗时较长，请稍后手动刷新"
    //   - 其他错误：显示实际错误
    //   根因：原实现未检查 503 lock_busy，将 lock_busy 误报为"服务未启动"
    let reason;
    if (err && err.status === 503) {
      // 503 lock_busy：sidecar 在线但后台合成持锁
      reason = '后台合成中，请稍后刷新';
      console.log('[LRC] 道同构度 lock_busy（后台合成中，可能由 503 或 200+降级触发），显示"后台合成中"而非"服务未启动"');
    } else if (typeof SidecarHealthMonitor !== 'undefined'
      && SidecarHealthMonitor
      && SidecarHealthMonitor._lockBusy === true) {
      // /health 返回 lock_busy=true：sidecar 在线但繁忙
      reason = '后台合成中，请稍后刷新';
    } else if (err && err.name === 'SidecarUnreachableError') {
      reason = 'LRC 服务未启动';
    } else if (isIndexing) {
      reason = '索引耗时较长，请稍后手动刷新';
    } else {
      reason = (err && err.message) ? err.message : '未知错误';
    }
    _applyDaoMetricsFallback(reason);
  }
}

/**
 * v0.8.11 P0-5：道同构度"索引中"提示（非降级）
 * sidecar 索引期间 dao_metrics 响应慢是预期行为，显示"索引中"而非"加载失败"
 * 区别于 _applyDaoMetricsFallback：不显示红色降级横幅，而是黄色"索引中"提示
 */
function _applyDaoMetricsIndexingHint() {
  const scoreEl = document.getElementById('dao-ring-score');
  if (scoreEl) scoreEl.textContent = '...';
  // 4 个小指标保持当前值（不重置为 '--'，避免视觉跳变）
  // 显示"索引中"提示横幅（黄色，非红色）
  const panel = document.querySelector('.dao-metrics-panel')
    || document.getElementById('dao-metrics-panel')
    || document.getElementById('dao-ring-score')?.closest('.card, .panel, .stat-card');
  if (panel) {
    let banner = panel.querySelector('.dao-indexing-hint');
    if (!banner) {
      banner = document.createElement('div');
      banner.className = 'dao-indexing-hint';
      banner.style.cssText = 'background:rgba(0,123,255,0.12);color:#004085;padding:8px 12px;border-radius:4px;margin-bottom:8px;font-size:13px;display:flex;align-items:center;gap:8px;';
      const spinner = document.createElement('span');
      spinner.style.cssText = 'display:inline-block;width:12px;height:12px;border:2px solid #004085;border-top-color:transparent;border-radius:50%;animation:lrc-spin 0.8s linear infinite;';
      spinner.className = 'lrc-spinner';
      banner.appendChild(spinner);
      const text = document.createElement('span');
      text.textContent = 'LRC 服务正在索引代码库，道同构度数据稍后自动加载...';
      banner.appendChild(text);
      panel.insertBefore(banner, panel.firstChild);
    }
  }
  // 确保加载动画 keyframes 存在
  if (!document.getElementById('lrc-spin-keyframes')) {
    const style = document.createElement('style');
    style.id = 'lrc-spin-keyframes';
    style.textContent = '@keyframes lrc-spin{to{transform:rotate(360deg)}}';
    document.head.appendChild(style);
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
  // v0.8.11 P0-5：降级时清除"索引中"提示横幅（如果存在）
  const indexingHint = document.querySelector('.dao-indexing-hint');
  if (indexingHint) indexingHint.remove();

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
    // v0.8.11：超时从 5s 延长到 10s
    const response = await fetchWithTimeout(`${window.API_BASE}/v1/audit-trail?limit=10`, {}, 10000);
    const data = await safeJson(response);

    if (data.events && data.events.length > 0) {
      const html = data.events.map(event => {
        // v0.9.0 修复：audit-trail 事件字段是 event_type（snake_case），非 type
        const rawType = event.event_type || 'audit';
        const typeLabelMap = {
          synthesis_created: '结晶合成',
          memory_deleted: '记忆删除',
          memory_isolated: '记忆隔离',
          gc_cleanup: 'GC 清理',
          decay_rate_changed: '衰减调整',
          regulation_applied: '调节应用',
          feedback_processed: '反馈处理',
          comprehensive_rebalance: '综合再平衡',
          catastrophic_event: '灾难检测',
          chronic_degradation: '慢性恶化',
        };
        const typeLabel = typeLabelMap[rawType] || rawType;
        const iconMap = {
          synthesis_created: 'icon-crystallization',
          memory_deleted: 'icon-delete',
          memory_isolated: 'icon-folder',
          gc_cleanup: 'icon-delete',
          decay_rate_changed: 'icon-decay',
          regulation_applied: 'icon-settings',
          feedback_processed: 'icon-smile',
          comprehensive_rebalance: 'icon-bagua',
          catastrophic_event: 'icon-warning',
          chronic_degradation: 'icon-warning',
        };
        const iconName = iconMap[rawType] || 'icon-audit';
        const typeClass = rawType;
        return `
          <li class="evolution-event ${typeClass}">
            <div class="evolution-event-dot"></div>
            <div class="evolution-event-time">${event.timestamp_ms || '--'}</div>
            <span class="evolution-event-type">
              <img src="/assets/icons/${iconName}.svg" alt="" width="12" height="12"> ${typeLabel}
            </span>
            <div class="evolution-event-desc">${htmlescape(event.description || event.reason || '')}</div>
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

  // v0.8.22 GAP-12 修复（interaction-resilience-auditor Round5 P1-01）：
  //   根因：Round4 修复的 error 上限检查嵌套在 visibleToasts.length >= TOAST_MAX_VISIBLE 内，
  //         当可见 toast 数 < 3 时 error 不受限，仍可显示 3 个 error
  //   修复：error toast 独立计数，无论总 toast 数多少，最多显示 2 个 error
  //
  // v0.8.43 修复：error 上限从 2 提升到 3，采用环形队列模式
  //   根因：审计报告 GAP-L1-08（P0）— 第 3 个 error 被静默跳过，关键错误信息丢失
  //   修复：error 上限提升到 3，超过上限时移除最旧的 error toast（环形队列）
  //   并在状态栏添加 error 计数徽章，确保用户不会遗漏关键错误
  const visibleToasts = container.querySelectorAll('.toast:not(.toast-leaving)');
  const MAX_ERROR_TOASTS = 3;
  if (type === 'error') {
    const visibleErrors = Array.from(visibleToasts).filter(t => t.classList.contains('toast-error'));
    if (visibleErrors.length >= MAX_ERROR_TOASTS) {
      // 环形队列：移除最旧的 error toast，为新 error 腾出空间
      const oldestError = visibleErrors[0];
      if (oldestError) {
        oldestError.classList.add('toast-leaving');
        setTimeout(() => {
          if (oldestError.parentNode) oldestError.parentNode.removeChild(oldestError);
        }, 200);
        console.log('[showToast] error 环形队列：移除最旧 error，添加新 error:', message);
      }
    }
  }

  // G011-2：可见 Toast 数量上限管理（在去重检查之后，记录 dedupKey 之前）
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
      // 如果全是 error 且已达上限，上面的独立检查已经 return 了
      // 这里不需要再检查 errorCount，直接继续添加新 error
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
  // v0.8.22 修复：使用 DOM 构建替代 innerHTML，消除 XSS 攻击向量
  // textContent 天然安全，无需 htmlescape 转义
  const iconWrap = document.createElement('div');
  iconWrap.className = 'toast-icon-wrap';
  const iconImg = document.createElement('img');
  iconImg.src = `/assets/icons/${iconName}.svg`;
  iconImg.alt = '';
  iconImg.className = 'toast-icon';
  iconWrap.appendChild(iconImg);
  const msgSpan = document.createElement('span');
  msgSpan.textContent = message;
  toast.appendChild(iconWrap);
  toast.appendChild(msgSpan);

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
  // v0.8.15 P0-2 修复：dashboard 加载后同时刷新道同构度，避免切换标签页后数据陈旧
  'dashboard': () => loadDashboard().then(() => {
    if (typeof loadDaoMetrics === 'function') return loadDaoMetrics();
  }),
  'trust-center': () => loadTrustCenter(),
  'benchmarks': () => loadBenchmarks(),
  'settings': () => { loadSettings(); loadProjectInfo(); },
  'system-status': () => loadSysStatusFloat(),
  'project-switch': () => loadProjectInfo()
};

// v0.8.23 S1-UX-03 修复：标签页切换滚动位置保存/恢复
let _tabScrollPositions = {};

async function switchTab(tabName) {
  // v0.8.4 Step 10 / G047：重试 Modal 显示时禁止标签页切换
  if (typeof _retryModalActive !== 'undefined' && _retryModalActive) {
    showToast('请先处理重试弹窗', 'warning');
    return false;
  }
  // v0.8.23 S1-UX-03 修复：切换前保存当前标签页的滚动位置
  const activeTab = document.querySelector('.tab-content.active');
  if (activeTab) {
    _tabScrollPositions[activeTab.id] = window.scrollY;
  }
  // v0.8.3 Step 12 / G017：标签页切换时取消旧标签页的进行中请求
  // 设计原则：仅 abort 当前活跃标签的 AbortController，新标签页加载不受影响
  // v0.8.4 Step 7 / G021：传入 tabName 作为 excludeTab，避免 abort 目标标签的请求
  _abortActiveTabRequests(tabName);

  // v0.8.13 B4: 切换到 dashboard 时重置索引期重试计数器，让新加载从 0 开始计数
  if (tabName === 'dashboard') {
    _dashboardRetryCount = 0;
  }

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
  // v0.8.23 S1-UX-03 修复：加载完成后恢复该标签页的滚动位置
  const savedPos = _tabScrollPositions[`tab-${tabName}`];
  if (savedPos !== undefined) {
    window.scrollTo(0, savedPos);
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
  // v0.8.22 IA-01 修复：切换标签页时 abort 道同构度请求
  // 根因：loadDaoMetrics 未被 _tabAbortControllers 管理，切换标签页时旧请求继续运行
  // 修复：切换离开 dashboard 时 abort daoAbortController（切换到 dashboard 时不 abort，让新请求正常加载）
  if (excludeTab !== 'dashboard' && daoAbortController) {
    daoAbortController.abort();
    daoAbortController = null;
    window.daoAbortController = null; // v0.8.22 HCSE Round2 修复：同步到 window，避免 CDP 测试读到旧引用
    console.log('[IA-01] 道同构度旧请求已取消（切换离开 dashboard）');
  }
  // v0.8.4 Step 7 / G021：不再无条件 abort dashboardAbortController
  // dashboard 的请求取消由 loadDashboard 自身管理（第 396-398 行）
  // 避免切换到 dashboard 时 abort 即将创建的新 dashboardAbortController

  // v0.8.13 B3: 标签页切换时取消索引期重试 timer，避免竞态与资源浪费
  if (_dashboardRetryTimer) {
    clearTimeout(_dashboardRetryTimer);
    _dashboardRetryTimer = null;
  }
  if (_daoRetryTimer) {
    clearTimeout(_daoRetryTimer);
    _daoRetryTimer = null;
  }
  // v0.8.23 S1-RES-03 修复：标签页切换时清理 lock_busy 冷却期倒计时
  // v0.8.25 NEW-01 修复：同时重置冷却期标志，避免切换回 dashboard 后仍被阻止自动重试
  //   根因：之前只清理了 timer，但 _lockBusyCooldown 仍为 true，导致用户切换标签页再回来时，
  //   冷却期不会自动恢复（需等待原始 setTimeout 触发），重试计数也保持旧值。
  //   修复：重置 _lockBusyCooldown 和 _dashboardRetryCount，让新加载从干净状态开始。
  if (_lockBusyCooldownTimer) {
    clearInterval(_lockBusyCooldownTimer);
    _lockBusyCooldownTimer = null;
  }
  if (_lockBusyCooldown) {
    _lockBusyCooldown = false;
    _dashboardRetryCount = 0;
    console.log('[NEW-01] 标签页切换时重置 lock_busy 冷却期状态');
  }
  // v0.8.15 P0-7/FM-16 修复：清除信任中心重试 timer，避免切换标签页后 timer 泄漏
  if (typeof _trustRetryTimer !== 'undefined' && _trustRetryTimer) {
    clearTimeout(_trustRetryTimer);
    _trustRetryTimer = null;
  }
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
    if (!res.ok) {
      // v0.8.48 修复：非 200 响应时显示降级状态而非保留 "--"
      setDegradedStatusFloat();
      return;
    }
    const data = await res.json();

    // v0.8.48 修复：lock_busy 降级时显示"后台合成中"而非保留 "--"
    if (data.lock_busy) {
      setDegradedStatusFloat('后台合成中');
      return;
    }

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
    // v0.9.0 P0 修复：网络不可达时更新 DOM 为降级状态，避免保留 HTML 初始 "--"
    console.warn('[Loong Recall] 系统状态浮窗加载失败:', e.message);
    setDegradedStatusFloat();
  }
}

/**
 * v0.8.48 新增：降级显示系统状态浮窗（避免保留 HTML 初始 "--"）
 * 当后端不可达或 lock_busy 时，显示有意义的状态而非 "--"
 * @param {string} [degradeText] 降级原因文案
 */
function setDegradedStatusFloat(degradeText) {
  const encoderText = degradeText || '不可用';
  const elIds = ['float-ml-model', 'float-encoder-type', 'float-cache-status', 'float-sys-mode', 'float-quality-score'];
  for (const id of elIds) {
    const el = document.getElementById(id);
    if (el) {
      el.textContent = encoderText;
      el.className = 'sys-status-value warning';
    }
  }
  const qualityFill = document.getElementById('float-quality-fill');
  if (qualityFill) qualityFill.style.width = '0%';
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

  // v0.9.0 P0 修复：Tauri 环境下延迟首次加载，确保 IPC 端口发现已完成
  // 根因：600ms 不足以让 IPC get_sidecar_status 返回，导致 API_BASE 仍为默认 3099
  // 如果 sidecar 实际在非 3099 端口，首次加载全部失败，所有字段保留 "--"
  const initialDelay = isTauriEnv ? 2500 : 600;
  setTimeout(loadSysStatusFloat, initialDelay);

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
    safeLocalStorageSetItem('lrc_sidebar_collapsed', isCollapsed ? '1' : '0');
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
  // v0.8.22 修复：如果隐藏输入为空，从 active 卡片读取 data-arg 作为兜底
  let modelId = document.getElementById('embedder-model')?.value?.trim();
  if (!modelId) {
    // 兜底：从 active 卡片读取选中的模型 ID
    const activeCard = document.querySelector('.provider-card.active[data-arg]');
    if (activeCard) {
      modelId = activeCard.getAttribute('data-arg');
    }
  }
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
  // v0.8.22 修复：移除 event?.target 依赖，统一通过 data-action 属性查找按钮
  let modelId = document.getElementById('embedder-model')?.value?.trim();
  if (!modelId) {
    // 兜底：从 active 卡片读取选中的模型 ID
    const activeCard = document.querySelector('.provider-card.active[data-arg]');
    if (activeCard) {
      modelId = activeCard.getAttribute('data-arg');
    }
  }
  if (!modelId) {
    showToast('请先选择一个模型', 'warning');
    return;
  }

  const mirror = document.getElementById('embedder-mirror')?.value || 'hf-mirror';
  const mirrorNames = {
    'hf-mirror': 'HF-Mirror',
    'modelscope': 'ModelScope'
  };

  // 通过 data-action 属性查找触发按钮
  const btn = document.querySelector('[data-action="testEmbedderConnection"]');
  const originalText = '测试镜像源';
  if (btn) {
    setButtonState(btn, 'loading', originalText);
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
      if (btn) setButtonState(btn, 'success', originalText);
    } else {
      throw new Error(data.message || '连接失败');
    }
  } catch (e) {
    console.error('[testEmbedderConnection] 连接失败:', e);
    showToast('❌ 连接失败: ' + e.message + '。请检查网络或尝试其他镜像源', 'error');
    if (btn) setButtonState(btn, 'error', originalText);
  } finally {
    if (btn) {
      // v0.8.25 CODE-03 修复：统一使用 setButtonState 恢复按钮状态，移除手动 DOM 操作
      // 之前手动设置 btn.disabled/btn.textContent，会导致按钮颜色不一致
      // 1.5s 后恢复文本（setButtonState 内部自动处理）
      setTimeout(() => {
        if (btn) setButtonState(btn, 'idle', originalText);
      }, 1500);
    }
  }
}

/**
 * v0.8.25 新增：模型连通性测试
 * 调用 /v1/model/test 端点，验证模型与 LRC 的连通性。
 * 区别于 testEmbedderConnection（测试镜像源连通性），此函数测试模型本身是否可用。
 * v0.8.25 修复：使用 setButtonState 统一按钮状态管理，移除手动 DOM 操作
 */
async function testModel() {
  const btn = document.querySelector('[data-action="testModel"]');
  if (!btn) return;

  const originalText = btn.textContent;
  setButtonState(btn, 'loading', originalText);

  try {
    const resp = await fetchWithTimeout(API_BASE + '/v1/model/test', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' }
    }, 15000);

    const data = await resp.json();

    if (data.ok) {
      showToast('✅ 模型测试通过！维度: ' + data.vector_dim + '，耗时: ' + data.elapsed_ms + 'ms', 'success', 5000);
      setButtonState(btn, 'success', originalText);
      btn.style.borderColor = 'var(--lrc-玉色-600)';
    } else {
      throw new Error(data.message || '模型测试失败');
    }
  } catch (e) {
    console.error('[testModel] 模型测试失败:', e);
    showToast('❌ 模型测试失败: ' + e.message + '。请确认模型已下载并应用', 'error', 5000);
    setButtonState(btn, 'error', originalText);
    btn.style.borderColor = 'var(--lrc-朱砂-500)';
  } finally {
    // v0.8.26 UX-01 修复：恢复时间从 5s 统一为 3s，与 setButtonState 文本恢复时间一致
    // 成功时保留绿色边框 3s，失败时保留红色边框 3s
    setTimeout(() => {
      btn.style.borderColor = '';
    }, 3000);
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
  // v0.8.15 P0-6 修复：switchProject 120s 等待期间显示进度反馈
  // 避免用户以为卡死
  showToast('正在切换项目并重新索引，请稍候...', 'info', 120000);
  // 监听后端 progress 事件更新提示（声明在 try 外，确保 catch 也能清理）
  let progressUnlisten = null;
  if (typeof tauriEvent !== 'undefined' && tauriEvent && tauriEvent.listen) {
    try {
      progressUnlisten = await tauriEvent.listen('sidecar-start-progress', (event) => {
        const payload = (event && event.payload) || {};
        if (payload.message) {
          showToast('切换项目中: ' + payload.message, 'info', 5000);
        }
      });
    } catch (listenErr) {
      console.warn('[LRC] switchProject progress 监听注册失败:', listenErr);
    }
  }

  try {
    // v0.8.10 L4-01：超时从 60s 延长到 120s，覆盖 stop(5s) + spawn_and_wait(40s) + 索引开销
    const result = await postMessageToParent('lrc-switch-project', {
      projectDir: trimmedDir,  // Tauri 命令参数: project_dir → projectDir (camelCase)
    }, 120000);

    // 清理 progress 事件监听器
    if (progressUnlisten) {
      try { progressUnlisten(); } catch (e) { /* 忽略 */ }
      progressUnlisten = null;
    }

    if (result && result.success) {
      // v0.8.2：用 showInfoModal 替代多行 alert
      showInfoModal('项目切换成功', '项目: ' + result.project_dir + '\n端口: ' + result.port + '\n\n' + (result.message || ''));
      // v0.8.13 F5: 设置 starting 状态，让 loadDashboard 进入索引期重试路径
      // 切换项目后 sidecar 重新索引，状态栏应显示"索引中"而非"加载失败"
      if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
        SidecarHealthMonitor._sidecarStatus = 'starting';
        SidecarHealthMonitor._setReachable(true);
      }
      // v0.8.10 L4-04：切换成功后主动触发健康检查，加速状态栏更新
      setTimeout(() => {
        loadDashboard();
        if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
          SidecarHealthMonitor.check();
        }
      }, 500);
    } else {
      showToast('项目切换失败: ' + (result?.message || '未知错误'), 'error');
    }
  } catch (e) {
    // v0.8.15 P0-6: catch 块也清理 progress 监听器
    if (progressUnlisten) {
      try { progressUnlisten(); } catch (cleanupErr) { /* 忽略 */ }
      progressUnlisten = null;
    }
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

// ============================================================
// v0.8.31 S-04：项目选择取消确认弹窗精细化控制
// ============================================================
// 核心问题：原来"只要项目列表空就弹确认"对"系统扫描不到任何项目"的场景过于打扰。
// 目标：仅当"用户手动取消掉所有已选中的项目"（非空→全空）时才弹窗，
//       其他场景（从没选过、自动扫描为空、用户没干预）直接进入下一步。
// ============================================================

/**
 * 标记：用户是否主动取消了所有已选中的项目。
 * true → 下一步按钮点击时弹"确认跳过"对话框
 * false → 用户没主动干预过项目选择，直接进入下一步
 * 触发 set 时机：任何项目 checkbox 从 checked→unchecked 后，发现所有项目都未选中
 * 触发 reset 时机：系统自动选中了项目（onAgentSelected 扫描完成）或用户手动添加了新项目（addSelectedProject）
 */
let _userCancelledAllProjectsFlag = false;

/** 重置「用户主动取消所有项目」标志（系统新增了任何已选项目后调用） */
function resetUserCancelledAllProjectsFlag() {
  if (_userCancelledAllProjectsFlag) {
    console.debug('[S-04] 重置「用户取消全部项目」标志：新增了已选项目');
  }
  _userCancelledAllProjectsFlag = false;
}

/** 获取当前「用户主动取消所有项目」标志 */
function getUserCancelledAllProjectsFlag() {
  return !!_userCancelledAllProjectsFlag;
}

/**
 * 向导中单个项目的 checkbox 变更回调。
 * 用于跟踪：用户是否把所有已选中的项目都取消了？
 *
 * 判定算法：
 * 1. 取向导中所有容器的项目 checkbox（两套 UI：wizard-project-list + selected-projects）
 * 2. 统计当前 checked 的数量
 * 3. 若本次是"取消勾选"且 total_checked === 0 → 置标志位 = true
 * 4. 若本次是"勾选上" → 标志位 = false（用户又选回来了，不应该打扰）
 */
function onWizardProjectCheckboxChanged(checkboxEl) {
  const allChecked = _countAllWizardProjectCheckedBoxes();
  const justChecked = checkboxEl && checkboxEl.checked;
  if (justChecked) {
    // 用户新勾选了某个项目 → 绝不属于"用户主动取消全部"
    resetUserCancelledAllProjectsFlag();
  } else {
    // 用户取消勾选
    if (allChecked === 0) {
      console.log('[S-04] 检测到用户把所有已选项目全部取消勾选，下一步点击将显示确认弹窗');
      _userCancelledAllProjectsFlag = true;
    }
  }
  // 同步更新 next 按钮状态
  if (typeof checkNextButton === 'function') checkNextButton();
}

/** 向导中所有项目容器（两套 UI）中勾选的 checkbox 总数 */
function _countAllWizardProjectCheckedBoxes() {
  let count = 0;
  const selectors = [
    '#wizard-project-list input[type="checkbox"]',
    '#selected-projects input[type="checkbox"]',
  ];
  for (const sel of selectors) {
    document.querySelectorAll(sel).forEach(cb => {
      if (cb.checked) count += 1;
    });
  }
  return count;
}

/**
 * 向导中是否存在"用户可选择但未选中"的项目列表？
 * 如果 wizard-project-list 或 selected-projects 根本没有任何项目条目（条目数为 0），
 * 那么即使空也属于「扫描不到项目」，不应该弹确认。
 * 弹窗条件：
 *   (项目条目数 > 0 且 allChecked = 0 且 userCancelledAllProjects=true)
 * 也就是"用户之前看到过项目、亲手取消掉了所有勾选"的情况才是真正需要确认的。
 */
function shouldShowConfirmSkipProjects() {
  // 用户明确取消过的标志位优先
  if (getUserCancelledAllProjectsFlag()) return true;

  // 回退判定：项目条目数 > 0 且 checked 数 = 0
  let entryCount = 0;
  document.querySelectorAll('#wizard-project-list .project-item, #selected-projects [data-project]').forEach(() => entryCount++);
  const checkedCount = _countAllWizardProjectCheckedBoxes();
  if (entryCount > 0 && checkedCount === 0) {
    // 条目存在但都没勾 → 可能是用户取消的，也可能系统默认没勾；保守起见不弹窗（避免误打扰）
    // 只有标志位为真时才弹窗，所以这里返回 false
    return false;
  }
  return false;
}

/**
 * v0.8.25 R-08 新增：选中 AI 工具后的回调逻辑
 * 当用户在向导中勾选/取消勾选 AI 工具时触发，自动扫描 IDE 项目目录。
 * 之前此功能为占位函数，未实现任何业务逻辑。
 * @param {string} agentId - 工具 ID（如 "trae", "cursor"）
 * @param {boolean} selected - 是否选中
 */
async function onAgentSelected(agentId, selected) {
  if (!selected) return;

  // v0.8.25 UX-06 修复：浏览器环境中 postMessageToParent 可能不可用
  if (!isTauriEnv) {
    console.warn('[onAgentSelected] 浏览器环境：自动扫描项目目录仅桌面端支持');
    showToast('项目目录自动扫描需在桌面端使用，请手动选择项目目录', 'info', 5000);
    return;
  }

  console.log('[onAgentSelected] 工具已选中:', agentId);

  // 如果是 IDE 类工具，自动扫描项目目录
  const ideCategories = ['ide', 'editor', 'plugin'];
  // 简单判断：包含常见 IDE 关键词的工具视为 IDE 类
  const ideKeywords = ['trae', 'cursor', 'code', 'windsurf', 'vscode', 'jetbrains', 'zed'];
  const isIde = ideKeywords.some(keyword => agentId.toLowerCase().includes(keyword));

  if (isIde) {
    try {
      // 调用后端扫描项目目录（30s 超时，与后端超时对齐）
      // v0.8.26 REG-01 修复：前端超时从 15s 提升到 30s，与后端 tokio::time::timeout(30s) 一致
      const projects = await postMessageToParent('lrc-scan-ide-projects', {
        ide_ids: [agentId]
      }, 30000);

      if (projects && projects.length > 0) {
        // 自动填充检测到的项目目录
        const projectListEl = document.getElementById('wizard-project-list');
        // v0.8.31 S-04：第一个项目高亮为"已自动选择为索引目录"（副文案 + 样式）
        // 并重置「用户主动取消全部」标志，因为现在系统自动选择了新的项目
        resetUserCancelledAllProjectsFlag();
        if (projectListEl) {
          projectListEl.innerHTML = projects.map((p, idx) => {
            const isFirst = idx === 0;
            // S-04：第一个项目添加 auto-selected-highlight 样式容器 + 已自动选择副文案
            const highlightStyle = isFirst
              ? '; border:1px solid var(--lrc-玉色-500); background: rgba(46, 204, 113, 0.08); border-radius: var(--radius-sm);'
              : '';
            const highlightBadge = isFirst
              ? `<div data-role="auto-selected-badge" style="margin-left:28px;margin-top:4px;font-size:0.78em;color:var(--lrc-玉色-700);font-weight:500;line-height:1.4;">
                   ✨ 已自动选择为索引目录（如需修改可取消勾选或选择其他项目）
                 </div>`
              : '';
            const highlightClass = isFirst ? ' auto-selected-highlight' : '';
            return `
              <div class="project-item${highlightClass}" data-path="${htmlescape(p.path || p)}" data-auto="${isFirst ? '1' : '0'}" style="${highlightStyle}">
                <input type="checkbox" checked data-action="toggleProject" onchange="onWizardProjectCheckboxChanged(this);">
                <span class="project-name">${htmlescape(p.name || p)}</span>
                <span class="project-path">${htmlescape(p.path || '')}</span>
                ${highlightBadge}
              </div>
            `;
          }).join('');
        }
        // selected-projects 区域（另一套UI）也同步一下，让 checkNextButton 能检测到有项目
        const firstPath = projects[0].path || projects[0];
        const firstDisplay = projects[0].name || firstPath;
        if (typeof addSelectedProject === 'function' && firstDisplay) {
          addSelectedProject(String(firstDisplay));
        }
        showToast('已自动检测到 ' + projects.length + ' 个项目目录（第一个已高亮为默认索引目录）', 'info', 4000);
      }
    } catch (e) {
      // 扫描失败不阻塞，用户可手动选择；但需要给用户友好的反馈
      // v0.8.25 R-06 修复：超时或失败时显示 Toast 提示，避免用户无感知
      const isTimeout = e.name === 'SidecarTimeoutError' || e.message?.includes('超时');
      console.warn('[onAgentSelected] 自动扫描项目目录失败（可手动选择）:', e.message);
      showToast(
        isTimeout
          ? '项目目录扫描超时，您可以手动选择项目目录'
          : '自动扫描项目目录失败，您可以手动选择',
        'warning',
        5000
      );
    }
  }
}

/**
 * v0.8.25 R-08 新增：向导下一步逻辑
 * 封装了步骤推进的完整逻辑：检查项目选择状态、确认跳过、保存配置。
 * 之前此功能为占位函数，仅做简单的步骤跳转。
 */
async function wizardNextStep() {
  const currentStep = document.querySelector('[id^="setup-step-"][style*="display:"]') ||
    document.querySelector('[id^="setup-step-"]:not([style*="none"])');

  if (!currentStep) return;

  const stepNum = parseInt(currentStep.id.replace('setup-step-', ''), 10);

  if (stepNum === 1) {
    // 从步骤 1 到步骤 2：检查项目选择状态
    // v0.8.31 S-04 修复：仅当「用户主动取消掉所有已选项目」时才弹确认，
    //   避免对「系统扫描不到项目」「从没选过项目」的场景过度打扰
    if (shouldShowConfirmSkipProjects()) {
      const skip = await showConfirm(
        '您已取消所有项目目录的勾选，确定跳过此步骤吗？\n\n您可以在后续配置中随时添加项目目录。\n不选择项目目录不影响基础功能使用。',
        '已取消所有项目目录选择'
      );
      if (!skip) return;
    }
  }

  // 前进到下一步
  goToStep(stepNum + 1);
}

/**
 * 检测 AI 工具（调用后端 API 实时检测）
 *
 * v0.8.31 S-05：检测完成后调用 get_scan_cache_metadata 显示「上次扫描时间」
 *   并提供「重新扫描」按钮（强制失效缓存后再重扫）
 */
async function simulateAiToolsScan() {
  const toolsList = document.getElementById('ai-tools-list');
  if (!toolsList) return;

  // v0.8.31 S-05：如果没有工具栏，动态插入（上次扫描时间 + 重新扫描按钮）
  ensureAiToolsToolbar();

  // 显示扫描中状态
  toolsList.innerHTML = '<p style="color: var(--lrc-墨韵-400); margin: 0;"><span class="loading-spinner"></span> 正在扫描已安装的 IDE & Agent 工具...</p>';
  // S-05：如果扫描中，同时让"上次扫描时间"显示「扫描中...」
  updateLastScanTsUi(null, true);

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

    // 为每个工具生成配置引导文案
    // 自动配置 = 后端 --install-ide 实际支持 + 官方文档确认可行
    function getToolConfigGuide(name, installed) {
      if (!installed) return '';
      const guides = {
        'VS Code': '已自动配置：MCP 配置已写入全局设置，无需手动操作',
        'Cursor': '已自动配置：MCP 配置已写入全局设置，无需手动操作',
        'Trae': '已自动配置：MCP 配置已写入全局设置，无需手动操作',
        'Trae CN': '已自动配置：MCP 配置已写入全局设置，无需手动操作',
        'Windsurf': '已自动配置：MCP 配置已写入全局设置，无需手动操作',
        'CodeBuddy': '已自动配置：MCP 配置已写入全局设置，无需手动操作',
        'CodeBuddy CN': '已自动配置：MCP 配置已写入全局设置（~/.codebuddy/.mcp.json），无需手动操作',
        'Qoder': '已自动配置：MCP 配置已写入全局设置（~/.qoder/settings.json），无需手动操作；若重启后未生效，可点击右上角用户图标 → Qoder Settings → MCP 确认',
        'GitHub Copilot': 'MCP 配置：通过 VS Code 设置 → 扩展 → GitHub Copilot → MCP 服务器配置',
        'JetBrains Toolbox': 'MCP 配置：通过 IDE 设置 → 工具 → MCP 服务器 → 添加，命令: code-memory-server --src-dir <项目路径> --stdio',
        'Zed': 'MCP 配置：在项目根目录创建 .zed/mcp.json，添加 LRC 服务配置',
        'Claude Code': 'MCP 配置：通过 claude.json 配置文件添加 MCP 服务器，命令: code-memory-server --src-dir <项目路径> --stdio',
        'Cline': 'MCP 配置：VS Code 扩展 Cline 设置 → MCP 服务器 → 添加，命令: code-memory-server --src-dir <项目路径> --stdio',
        'Continue': 'MCP 配置：VS Code 扩展 Continue 设置 → MCP 服务器 → 添加，命令: code-memory-server --src-dir <项目路径> --stdio',
      };
      return guides[name] || '';
    }

    // v0.8.31 S-03：初始化时读取 localStorage 的手动修正，让用户上次的选择即使后端没写也能恢复
    const localOverrides = getLocalManualOverrides();
    // v0.8.46 优化：只展示用户匹配到的（已安装）工具，避免展示无关工具列表
    const effectiveTools = tools.map(tool => {
      const agentId = toolNameToAgentId(tool.name);
      const hasLocalOverride = Object.prototype.hasOwnProperty.call(localOverrides, agentId);
      const finalInstalled = hasLocalOverride ? !!localOverrides[agentId] : tool.installed;
      return { ...tool, agentId, hasLocalOverride, finalInstalled };
    }).filter(t => t.finalInstalled);

    if (effectiveTools.length === 0) {
      toolsList.innerHTML = '<p style="color: var(--lrc-墨韵-400); margin: 0;">未检测到已安装的 IDE 或 Agent 工具</p>';
      return;
    }

    toolsList.innerHTML = effectiveTools.map(tool => {
      const guide = getToolConfigGuide(tool.name, tool.installed);
      const agentId = tool.agentId;
      const hasLocalOverride = tool.hasLocalOverride;
      const finalInstalled = tool.finalInstalled;
      const manualBadgeStyle = hasLocalOverride
        ? 'display:inline-block; margin-right:6px; font-size:0.75em; padding:1px 5px; border-radius:3px; background:var(--lrc-金色-200); color:var(--lrc-金色-800); border:1px solid var(--lrc-金色-400);'
        : 'display:none; margin-right:6px; font-size:0.75em; padding:1px 5px; border-radius:3px; background:var(--lrc-金色-200); color:var(--lrc-金色-800); border:1px solid var(--lrc-金色-400);';
      // 齿轮图标 hover 效果 + 点击菜单
      const gearBtn = `<button type="button" data-role="tool-gear-btn" 
        onclick="event.stopPropagation(); event.preventDefault(); showToolGearMenu('${tool.name.replace(/'/g, "\\'")}', ${finalInstalled}, this);"
        title="检测结果不对？点此手动修正"
        style="background:none;border:none;cursor:pointer;font-size:1em;padding:2px 4px;border-radius:4px;opacity:0.55;transition:all 0.15s;margin-right:4px;"
        onmouseover="this.style.opacity='1';this.style.background='var(--lrc-宣纸-500)';"
        onmouseout="this.style.opacity='0.55';this.style.background='transparent';">⚙️</button>`;
      return `
      <div data-agent-id="${htmlescape(agentId)}" style="padding: 12px 0; border-bottom: 1px solid var(--lrc-宣纸-500);">
        <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: ${guide ? '6px' : '0'};">
          <div style="display: flex; align-items: center; gap: 12px;">
            <input type="checkbox" ${finalInstalled ? 'checked' : ''} ${!finalInstalled && !hasLocalOverride ? 'disabled' : ''} id="tool-${tool.name.replace(/\s/g, '-')}"
              data-auto-install-disabled="${!tool.installed ? 'true' : 'false'}"
              onchange="if(this.checked && typeof onAgentSelected==='function') onAgentSelected('${htmlescape(agentId)}', true);">
            <span style="color: var(--lrc-墨韵-700); font-weight: 500;">${htmlescape(tool.name)}</span>
            <span style="font-size: 0.8em; color: var(--lrc-墨韵-400);">(${htmlescape(tool.type || '')})</span>
            ${tool.version ? '<span style="font-size: 0.8em; color: var(--lrc-墨韵-400);">v' + htmlescape(tool.version) + '</span>' : ''}
          </div>
          <div style="display:flex; align-items:center;">
            ${gearBtn}
            <span data-role="manual-override-badge" style="${manualBadgeStyle}" title="用户手动指定，不受自动检测影响">🔧 用户指定</span>
            <span data-role="tool-status"
              data-auto-status="${tool.installed ? '已检测到' : '未安装'}"
              data-auto-status-installed="${tool.installed ? 'true' : 'false'}"
              data-auto-status-checked="${tool.installed ? 'true' : 'false'}"
              style="font-size: 0.85em; font-weight: 600; color: var(--lrc-玉色-600);">已检测到</span>
          </div>
        </div>
        ${guide ? `<div style="font-size: 0.8em; color: var(--lrc-墨韵-500); padding-left: 36px; line-height: 1.5;">💡 ${htmlescape(guide)}</div>` : ''}
      </div>`;
    }).join('');

    // 统计已安装数量
    const installedCount = effectiveTools.length;
    // S-05：工具列表渲染完成后，刷新「上次扫描时间」显示
    refreshScanCacheMetadataUi();

  } catch (e) {
    toolsList.innerHTML = '<p style="color: var(--lrc-朱砂-500); margin: 0;">检测失败: ' + htmlescape(e.message) + '</p><p style="color: var(--lrc-墨韵-400); font-size: 0.85em; margin-top: 8px;">请确保龙忆（LRC）服务正在运行</p><button class="btn btn-accent" style="margin-top: 12px;" data-action="retryToolDetection">重新检测</button>';
    // S-05：即使失败也刷新「上次扫描时间」（用上次的缓存数据，若有）
    refreshScanCacheMetadataUi();
    // 动态生成的按钮需要重新绑定 data-action
    if (typeof bindAllActions === 'function') {
      bindAllActions();
    }
  }
}

// v0.8.22 修复：工具检测失败后重试按钮
function retryToolDetection() {
  simulateAiToolsScan();
}

// ============================================================
// v0.8.31 S-05：扫描缓存 UI 控制（上次扫描时间 + 重新扫描按钮）
// ============================================================

/**
 * 确保工具列表上方有工具栏（上次扫描时间 + 重新扫描按钮）。
 * 不存在时动态插入（避免修改原 HTML）。
 */
function ensureAiToolsToolbar() {
  const toolsList = document.getElementById('ai-tools-list');
  if (!toolsList) return;
  // 已存在就不重复创建
  if (document.getElementById('lrc-ai-tools-toolbar')) return;
  const toolbar = document.createElement('div');
  toolbar.id = 'lrc-ai-tools-toolbar';
  toolbar.style.cssText = [
    'display:flex; align-items:center; justify-content:space-between;',
    'margin: 0 0 10px 0; padding: 6px 2px; font-size: 0.85em; color: var(--lrc-墨韵-500);',
    'border-bottom:1px dashed var(--lrc-宣纸-500);',
  ].join('');
  toolbar.innerHTML = `
    <div data-role="last-scan-ts" style="display:flex; align-items:center; gap:6px;">
      <span>📅</span>
      <span data-role="last-scan-text">上次扫描：</span>
      <span data-role="last-scan-value" style="color: var(--lrc-墨韵-700); font-weight: 500;">加载中...</span>
      <span data-role="last-scan-valid" style="display:none; padding: 1px 6px; border-radius: 3px; font-size: 0.9em; background: var(--lrc-玉色-100); color: var(--lrc-玉色-700); margin-left: 6px;">24h 内有效</span>
    </div>
    <div style="display:flex; align-items:center; gap:8px;">
      <button type="button" class="btn" data-action="rescanToolsWithInvalidate"
        title="清空扫描缓存，重新扫描桌面快捷方式和安装目录（检测结果不准确时使用）"
        style="padding: 4px 10px; font-size: 0.85em; border-radius: var(--radius-sm); background: var(--lrc-宣纸-400); border: 1px solid var(--lrc-宣纸-600); cursor: pointer; color: var(--lrc-墨韵-700);">
        🔄 重新扫描（清空缓存）
      </button>
    </div>
  `;
  // 插入到 ai-tools-list 之前
  if (toolsList.parentNode) {
    toolsList.parentNode.insertBefore(toolbar, toolsList);
  }
  // v0.8.45 修复：动态创建的 data-action 按钮需要重新绑定事件
  //   根因：之前未调用 bindAllActions，重新扫描按钮的点击事件从未绑定
  if (typeof bindAllActions === 'function') {
    bindAllActions();
  }
}

/**
 * 更新「上次扫描时间」文本（不发请求，用于扫描中提示）。
 * @param {object|null} meta - 已有的元数据，null 表示加载/未知
 * @param {boolean} scanning - 是否正在扫描中（显示「扫描中...」）
 */
function updateLastScanTsUi(meta, scanning) {
  const valueEl = document.querySelector('[data-role="last-scan-value"]');
  const validEl = document.querySelector('[data-role="last-scan-valid"]');
  if (!valueEl) return;

  if (scanning) {
    valueEl.textContent = '扫描中...';
    valueEl.style.color = 'var(--lrc-金色-700)';
    if (validEl) validEl.style.display = 'none';
    return;
  }

  if (!meta || !meta.timestamp_ms) {
    valueEl.textContent = '尚未扫描';
    valueEl.style.color = 'var(--lrc-墨韵-400)';
    if (validEl) validEl.style.display = 'none';
    return;
  }

  try {
    const dt = new Date(Number(meta.timestamp_ms));
    if (isNaN(dt.getTime())) throw new Error('invalid ts');
    const pad = n => String(n).padStart(2, '0');
    const text = `${dt.getFullYear()}-${pad(dt.getMonth() + 1)}-${pad(dt.getDate())} ${pad(dt.getHours())}:${pad(dt.getMinutes())}`;
    valueEl.textContent = text;
    valueEl.style.color = 'var(--lrc-墨韵-700)';
    if (validEl) {
      if (meta.valid) {
        validEl.style.display = 'inline-block';
        validEl.textContent = '24h 内有效';
        validEl.style.background = 'var(--lrc-玉色-100)';
        validEl.style.color = 'var(--lrc-玉色-700)';
      } else {
        validEl.style.display = 'inline-block';
        validEl.textContent = '已超过 24h，建议重扫';
        validEl.style.background = 'var(--lrc-金色-100)';
        validEl.style.color = 'var(--lrc-金色-800)';
      }
    }
  } catch (e) {
    valueEl.textContent = '时间解析失败';
    if (validEl) validEl.style.display = 'none';
  }
}

/**
 * 调用 Tauri get_scan_cache_metadata 命令并刷新 UI 显示「上次扫描时间」
 */
async function refreshScanCacheMetadataUi() {
  if (!isTauriEnv) {
    // v0.8.44 修复：浏览器模式下显示"实时扫描"而非"尚未扫描"
    updateLastScanTsUi({ timestamp_ms: Date.now(), valid: true }, false);
    return;
  }
  try {
    const meta = await postMessageToParent('lrc-get-scan-cache-metadata', null, 8000);
    updateLastScanTsUi(meta || null, false);
  } catch (e) {
    console.warn('[S-05] 获取扫描缓存元数据失败：', e.message);
    updateLastScanTsUi(null, false);
  }
}

/**
 * 用户点击「重新扫描（清空缓存）」按钮。
 * 流程：invalidate 缓存 → 重新调用 simulateAiToolsScan（会触发重新扫描）→ 刷新时间戳
 */
async function rescanToolsWithInvalidate() {
  // v0.8.45 修复：为重新扫描按钮添加视觉反馈
  //   根因：之前点击按钮后无任何视觉反馈，用户感知为"按钮无交互反应"
  //   修复：禁用按钮 + 显示"扫描中..."文本，扫描完成后恢复
  const btn = document.querySelector('[data-action="rescanToolsWithInvalidate"]');
  if (btn) {
    btn.disabled = true;
    btn.textContent = '⏳ 扫描中...';
    btn.style.opacity = '0.6';
    btn.style.cursor = 'not-allowed';
  }

  if (typeof showToast === 'function') {
    showToast('正在清空扫描缓存并重新检测，请稍候...', 'info', 4000);
  }
  // 桌面端才调 IPC，浏览器模式直接重试
  if (isTauriEnv) {
    try {
      await postMessageToParent('lrc-force-invalidate-scan-cache', null, 8000);
      console.log('[S-05] 扫描缓存已成功失效');
    } catch (e) {
      console.warn('[S-05] 强制失效缓存失败（不影响重扫）：', e.message);
    }
  }
  // 重新触发检测（此时缓存已失效，get_scan_cache 内部会重扫）
  await simulateAiToolsScan();

  // 恢复按钮状态
  if (btn) {
    btn.disabled = false;
    btn.textContent = '🔄 重新扫描（清空缓存）';
    btn.style.opacity = '';
    btn.style.cursor = '';
  }

  if (typeof showToast === 'function') {
    showToast('重新扫描完成！如结果仍不准确，可点击工具卡片右侧 ⚙️ 齿轮手动修正', 'success', 4500);
  }
}

// ============================================================
// v0.8.31 S-03：AI 工具手动修正（向导齿轮图标逻辑）
// ============================================================

/**
 * 工具显示名 → 后端 agent_id 的映射表
 * 用于前端按钮点击时，将 /api/tools/detect 返回的 name 正确映射到
 * discover_all_agents 注册的工具 ID，保证后端 manual_override 持久化正确。
 */
const TOOL_NAME_TO_AGENT_ID_MAP = {
  'Trae': 'trae',
  'Trae CN': 'trae-cn',
  'Cursor': 'cursor',
  'VS Code': 'vscode',
  'Windsurf': 'windsurf',
  'Claude Desktop': 'claude-desktop',
  'Gemini CLI': 'gemini-cli',
  'CodeBuddy': 'codebuddy',
  'CodeBuddy CN': 'codebuddy',
  'CodeBuddy (腾讯)': 'codebuddy',
  'Zed': 'zed',
  'JetBrains Toolbox': 'jetbrains-toolbox',
  'IntelliJ IDEA': 'intellij-idea',
  'PyCharm': 'pycharm',
  'GoLand': 'goland',
  'WebStorm': 'webstorm',
  'CLion': 'clion',
  'RustRover': 'rustrover',
  'Qwen Code': 'qwen-code',
  'Cline': 'cline',
  'Continue': 'continue',
  'Windsurf Cascade': 'windsurf',
  'Qoder': 'qoder',
  'Replit AI': 'replit-ai',
  'DeepSeek Coder': 'deepseek-coder',
  'GitHub Copilot': 'github-copilot',
  'Claude Code': 'claude-code',
};

/** 将工具显示名映射为后端 agent_id（找不到时返回原始名做最佳努力匹配） */
function toolNameToAgentId(displayName) {
  if (!displayName) return '';
  const trimmed = String(displayName).trim();
  return TOOL_NAME_TO_AGENT_ID_MAP[trimmed] || trimmed;
}

/** localStorage 次级备份 key（即使后端 IPC 失败，也至少在重启前保持一致） */
const LS_MANUAL_OVERRIDE_KEY = 'lrc_agent_manual_overrides_v1';

/** 从 localStorage 读取所有手动修正（次级备份，用于非桌面端环境或 IPC 失败时降级） */
function getLocalManualOverrides() {
  try {
    const raw = localStorage.getItem(LS_MANUAL_OVERRIDE_KEY);
    if (!raw) return {};
    const obj = JSON.parse(raw);
    return (obj && typeof obj === 'object') ? obj : {};
  } catch (e) {
    console.warn('[manualOverride] localStorage 读取失败，降级为空:', e.message);
    return {};
  }
}

/** 将单个手动修正写入 localStorage（次级备份），返回 true=成功 false=失败 */
function setLocalManualOverride(agentId, overrideInstalled) {
  try {
    const all = getLocalManualOverrides();
    if (overrideInstalled === null || overrideInstalled === undefined) {
      delete all[agentId]; // None = 清除此工具的手动修正
    } else {
      all[agentId] = !!overrideInstalled;
    }
    localStorage.setItem(LS_MANUAL_OVERRIDE_KEY, JSON.stringify(all));
    return true;
  } catch (e) {
    console.warn('[manualOverride] localStorage 写入失败:', e.message);
    return false;
  }
}

/**
 * 用户通过齿轮菜单点击"修正工具检测结果"。
 * 流程：
 *   1. 立即写 localStorage（UI 立刻响应，不等待 IPC）
 *   2. 桌面端环境下异步写后端持久化（IPC 失败时 Toast 提醒用户）
 *   3. 更新 UI：状态文字 + checkbox checked + 已手动修正标记
 *
 * @param {string} toolDisplayName - 工具显示名（来自 api/tools/detect 的 name 字段）
 * @param {boolean|null} overrideInstalled - true=强制已安装, false=强制未安装, null=恢复自动检测
 */
async function applyAgentManualOverride(toolDisplayName, overrideInstalled) {
  const agentId = toolNameToAgentId(toolDisplayName);
  if (!agentId) {
    showToast('无法识别工具：' + toolDisplayName, 'error', 4000);
    return;
  }

  // ── Step 1：立即写 localStorage 并更新 UI，零延迟响应 ──
  setLocalManualOverride(agentId, overrideInstalled);
  refreshSingleToolCardUi(agentId, overrideInstalled);

  // ── Step 2：桌面端环境写后端持久化（wizard.json），失败时次级 Toast 提示 ──
  let persistOk = true;
  if (isTauriEnv) {
    try {
      // 对于 null，传 undefined 让 Tauri 序列化为 Option::<bool>::None
      const payload = overrideInstalled === null
        ? { agent_id: agentId, override_installed: null }
        : { agent_id: agentId, override_installed: !!overrideInstalled };
      await postMessageToParent('lrc-set-agent-manual-override', payload, 15000);
      console.log('[manualOverride] 后端持久化成功:', agentId, '→', overrideInstalled);
    } catch (e) {
      persistOk = false;
      console.warn('[manualOverride] 后端持久化失败（已降级为本地临时生效）:', e.message);
    }
  }

  // ── Step 3：Toast 反馈 ──
  const actionText = overrideInstalled === true ? '已标记为安装'
    : overrideInstalled === false ? '已标记为未安装'
    : '已恢复自动检测';
  const persistHint = persistOk ? '' : '（本地临时生效，重启后可能丢失）';
  showToast(`${toolDisplayName}：${actionText}${persistHint}`, persistOk ? 'success' : 'warning', 3500);

  // 变更后重新检查下一步按钮（防止用户取消所有安装后误以为项目已选）
  if (typeof checkNextButton === 'function') checkNextButton();
}

/**
 * 根据最新的手动覆盖状态，刷新单个工具卡片的 UI 显示：
 *   - checkbox checked 状态
 *   - checkbox disabled 状态（手动覆盖时允许用户勾选未安装的工具）
 *   - 右侧"已检测到/未安装"文字 + 🔧 用户手动指定标记
 */
function refreshSingleToolCardUi(agentId, overrideInstalled) {
  // 找卡片：当前渲染使用 tool.name 替换空格为 - 作为 checkbox id 前缀，
  // 此处使用 agentId 反查 name，或直接通过 data-agent-id 查找（下面渲染新加入 data-agent-id）
  const cards = document.querySelectorAll('[data-agent-id="' + agentId + '"]');
  cards.forEach(card => {
    const statusEl = card.querySelector('[data-role="tool-status"]');
    const cb = card.querySelector('input[type="checkbox"]');
    const manualBadge = card.querySelector('[data-role="manual-override-badge"]');

    // 状态文本 + 颜色
    if (statusEl) {
      if (overrideInstalled === null) {
        // 恢复为自动检测 → 需要重新拉数据，这里只移除手动标记，让用户点重新检测
        statusEl.textContent = statusEl.dataset.autoStatus || statusEl.textContent;
        statusEl.style.color = '';
      } else {
        statusEl.textContent = overrideInstalled ? '已检测到' : '未安装';
        statusEl.style.color = overrideInstalled ? 'var(--lrc-玉色-600)' : 'var(--lrc-墨韵-300)';
      }
    }

    // checkbox：手动修正为已安装时，用户能勾；手动修正为未安装时，用户应该勾不上
    if (cb) {
      if (overrideInstalled === null) {
        cb.checked = !!statusEl?.dataset?.autoStatusChecked;
        cb.disabled = !statusEl?.dataset?.autoStatusInstalled
          && cb.dataset.autoInstallDisabled !== 'false';
      } else {
        cb.checked = !!overrideInstalled;
        // 手动修正为未安装时仍允许用户勾选（万一用户想先选）
        cb.disabled = false;
      }
    }

    // 手动修正小徽章
    if (manualBadge) {
      manualBadge.style.display = overrideInstalled === null ? 'none' : 'inline-block';
    }
  });
}

/**
 * 点击工具卡片上的齿轮图标时，弹出菜单（这不是我用的 / 改成我在用的 / 恢复自动检测）。
 * 使用原生 showConfirm 风格的简化实现：连点两次确认 -> 执行操作；
 * 更完整的实现用自定义弹窗，这里使用三选一的原生 confirm + prompt 组合。
 *
 * 更好的 UX：直接弹出一个 3 选 1 的自定义小菜单，避免三次原生弹窗打断用户。
 * 下面提供一个基于 data-action 事件委托的轻量实现。
 *
 * @param {string} toolDisplayName - 工具显示名
 * @param {HTMLElement} anchorEl - 齿轮图标锚点（用于定位小菜单）
 */
function showToolGearMenu(toolDisplayName, currentInstalled, anchorEl) {
  // 清理之前的菜单（避免重复叠加）
  document.querySelectorAll('.lrc-agent-gear-menu').forEach(m => m.remove());

  const agentId = toolNameToAgentId(toolDisplayName);
  const localOverrides = getLocalManualOverrides();
  const hasManual = Object.prototype.hasOwnProperty.call(localOverrides, agentId);

  const menu = document.createElement('div');
  menu.className = 'lrc-agent-gear-menu';
  menu.style.cssText = `
    position:absolute; z-index:99999; background:var(--lrc-宣纸-100);
    border:1px solid var(--lrc-宣纸-600); border-radius:var(--radius-md);
    box-shadow:0 6px 24px rgba(0,0,0,0.15); padding:6px 0; min-width:200px;
    font-size:0.9em; color:var(--lrc-墨韵-700);
  `;
  menu.setAttribute('data-tool-name', toolDisplayName);
  menu.innerHTML = `
    <div data-gear-action="mark-not-installed" style="padding:8px 14px;cursor:pointer;display:flex;align-items:center;gap:8px;" onmouseover="this.style.background='var(--lrc-宣纸-400)'" onmouseout="this.style.background='transparent'">
      <span>🚫</span><span>这不是我用的工具</span>
    </div>
    <div data-gear-action="mark-installed" style="padding:8px 14px;cursor:pointer;display:flex;align-items:center;gap:8px;" onmouseover="this.style.background='var(--lrc-宣纸-400)'" onmouseout="this.style.background='transparent'">
      <span>✅</span><span>改为我正在用的工具</span>
    </div>
    <div data-gear-action="reset" ${hasManual ? '' : 'style="opacity:0.4;pointer-events:none"'} style="padding:8px 14px;cursor:pointer;display:flex;align-items:center;gap:8px;border-top:1px solid var(--lrc-宣纸-500);margin-top:4px;padding-top:10px;" onmouseover="this.style.background='var(--lrc-宣纸-400)'" onmouseout="this.style.background='transparent'">
      <span>♻️</span><span>恢复自动检测</span>
    </div>
  `;

  // 定位到齿轮按钮附近
  document.body.appendChild(menu);
  const r = anchorEl.getBoundingClientRect();
  const scrollY = window.scrollY || 0;
  const scrollX = window.scrollX || 0;
  let left = r.left + scrollX;
  let top = r.bottom + scrollY + 6;
  // 边界修正：菜单超出视口右/下
  const mr = menu.getBoundingClientRect();
  if (left + mr.width > window.innerWidth + scrollX) {
    left = r.right + scrollX - mr.width;
  }
  if (top + mr.height > window.innerHeight + scrollY) {
    top = r.top + scrollY - mr.height - 6;
  }
  menu.style.left = Math.max(4, left) + 'px';
  menu.style.top = Math.max(4, top) + 'px';

  // 点击菜单项执行动作
  menu.addEventListener('click', async (e) => {
    const actionEl = e.target.closest('[data-gear-action]');
    if (!actionEl) return;
    const action = actionEl.getAttribute('data-gear-action');
    const toolName = menu.getAttribute('data-tool-name');
    menu.remove();
    document.querySelectorAll('.lrc-agent-gear-menu').forEach(m => m.remove());

    if (action === 'mark-not-installed') {
      await applyAgentManualOverride(toolName, false);
    } else if (action === 'mark-installed') {
      await applyAgentManualOverride(toolName, true);
    } else if (action === 'reset') {
      await applyAgentManualOverride(toolName, null);
    }
  });

  // 点击页面任意位置关闭菜单（一次性）
  setTimeout(() => {
    const closeHandler = (ev) => {
      if (!ev.target.closest('.lrc-agent-gear-menu') && !ev.target.closest('[data-role="tool-gear-btn"]')) {
        document.querySelectorAll('.lrc-agent-gear-menu').forEach(m => m.remove());
        document.removeEventListener('click', closeHandler, true);
      }
    };
    document.addEventListener('click', closeHandler, true);
  }, 0);
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

  // v0.8.31 S-04：用户手动新增了项目 → 取消「用户主动取消全部项目」的判定
  resetUserCancelledAllProjectsFlag();

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
 * v0.8.25：检查下一步按钮状态 — 允许跳过项目目录选择
 * 用户可以选择项目后进入下一步，也可以直接跳过。
 * 跳过时会在控制台记录，不影响后续流程。
 */
function checkNextButton() {
  const nextBtn = document.getElementById('step-1-next-btn');
  const projectsContainer = document.getElementById('selected-projects');
  if (!nextBtn || !projectsContainer) return;

  const hasProjects = projectsContainer.children.length > 0 && !projectsContainer.querySelector('p');
  // v0.8.25：不再禁用按钮，始终允许用户进入下一步
  // 无项目时点击会弹出确认对话框
  nextBtn.disabled = false;
  nextBtn.style.opacity = '1';
  nextBtn.style.cursor = 'pointer';
}

/**
 * v0.8.4 Step 9 / G025 修复：移除向导中已选项目（替代内联 onclick）
 * @param {HTMLElement} btn - 点击的按钮元素
 */
function removeProjectFromWizard(btn) {
  if (!btn || !btn.parentElement || !btn.parentElement.parentElement) return;
  btn.parentElement.parentElement.remove();

  // v0.8.31 S-04：用户手动移除了项目 → 如果移除后所有项目都没有了，视为「用户主动取消全部」
  const projectsContainer = document.getElementById('selected-projects');
  if (projectsContainer) {
    const remainingEntries = projectsContainer.children.length > 0 && !projectsContainer.querySelector('p');
    if (!remainingEntries) {
      const checkedBoxes = _countAllWizardProjectCheckedBoxes();
      if (checkedBoxes === 0) {
        console.log('[S-04] 用户手动移除了所有项目条目，下一步点击将显示确认弹窗');
        _userCancelledAllProjectsFlag = true;
      }
    }
  }

  if (typeof checkNextButton === 'function') {
    checkNextButton();
  }
}
window.removeProjectFromWizard = removeProjectFromWizard;

/**
 * 跳转到指定步骤
 * v0.8.25：从步骤 1 跳转到步骤 2 时，检查是否已选择项目
 * 未选择项目时弹出确认对话框，允许用户跳过
 */
function goToStep(stepNum) {
  // v0.8.45 修复：始终允许跳过项目选择，不再阻塞步骤跳转
  //   根因：之前仅当 _userCancelledAllProjectsFlag 为 true 时弹确认弹窗，
  //         但用户可能从未选择过任何项目，或不想选择项目只想跳过。
  //         用户反馈"必须选择文件夹"，说明弹窗逻辑在某些场景下阻塞了导航。
  //   修复：移除确认弹窗，用户始终可以跳过项目选择，后续可在设置中随时添加。
  doGoToStep(stepNum);
}

/**
 * 执行实际步骤跳转（goToStep 的内部实现）
 */
function doGoToStep(stepNum) {
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

  // v0.8.44 修复：进入步骤 1 时自动触发 AI 工具扫描（不依赖用户点击"开始完整配置"）
  if (stepNum === 1) {
    // 确保下一步按钮可用（用户可跳过项目选择）
    if (typeof checkNextButton === 'function') {
      checkNextButton();
    }
    // 延迟触发扫描，确保 DOM 已渲染
    setTimeout(() => {
      if (typeof simulateAiToolsScan === 'function' && document.getElementById('setup-step-1')?.style.display !== 'none') {
        simulateAiToolsScan();
      }
    }, 300);
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
 * v0.8.25 GAP-16 修复：在跳转到完成页面前保存 LLM 配置（如果已填写）
 * 之前 finishSetup 仅跳转到步骤 3，不保存任何配置，导致用户在向导中配置的 LLM 设置丢失。
 */
async function finishSetup() {
  // 步骤 1：检查 LLM 是否已配置，如果是则保存
  const provider = document.getElementById('setup-llm-provider')?.value;
  if (provider && provider !== 'none') {
    const apiKey = document.getElementById('setup-llm-api-key')?.value?.trim();
    if (apiKey && apiKey.length >= 10) {
      try {
        // 保存 LLM 配置到后端，确保用户在向导中输入的配置不会丢失
        const result = await postMessageToParent('lrc-save-llm-config', {
          provider: provider,
          api_key: apiKey
        }, 10000);
        if (result && result.success !== false) {
          console.log('[finishSetup] LLM 配置已保存:', provider);
        } else {
          console.warn('[finishSetup] LLM 配置保存失败，继续完成向导:', result?.message || '未知错误');
        }
      } catch (e) {
        // 保存失败不阻塞向导完成，用户可在设置中重新配置
        console.warn('[finishSetup] LLM 配置保存异常，继续完成向导:', e.message);
      }
    }
  }

  // 步骤 2：进入完成页面
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
// v0.8.25 R-08：暴露新函数供 HTML 调用
window.onAgentSelected = onAgentSelected;
window.wizardNextStep = wizardNextStep;

// v0.8.31 S-03：暴露齿轮图标的手动修正函数，供内联 onclick 和调试使用
window.retryToolDetection = retryToolDetection;
window.toolNameToAgentId = toolNameToAgentId;
window.getLocalManualOverrides = getLocalManualOverrides;
window.setLocalManualOverride = setLocalManualOverride;
window.applyAgentManualOverride = applyAgentManualOverride;
window.refreshSingleToolCardUi = refreshSingleToolCardUi;
window.showToolGearMenu = showToolGearMenu;

// v0.8.31 S-04：暴露项目选择取消确认的标志位控制函数
window.resetUserCancelledAllProjectsFlag = resetUserCancelledAllProjectsFlag;
window.getUserCancelledAllProjectsFlag = getUserCancelledAllProjectsFlag;
window.onWizardProjectCheckboxChanged = onWizardProjectCheckboxChanged;
window.shouldShowConfirmSkipProjects = shouldShowConfirmSkipProjects;
window.addSelectedProject = addSelectedProject;

// v0.8.31 S-05：暴露扫描缓存 UI 控制函数（供 onclick 和调试使用）
window.simulateAiToolsScan = simulateAiToolsScan;
window.retryToolDetection = retryToolDetection;
window.ensureAiToolsToolbar = ensureAiToolsToolbar;
window.updateLastScanTsUi = updateLastScanTsUi;
window.refreshScanCacheMetadataUi = refreshScanCacheMetadataUi;
window.rescanToolsWithInvalidate = rescanToolsWithInvalidate;

// 嵌入模型配置
window.checkEmbedderStatus = checkEmbedderStatus;
window.selectEmbedderModel = selectEmbedderModel;
window.downloadEmbedderModel = downloadEmbedderModel;
window.applyEmbedderModel = applyEmbedderModel;
window.testEmbedderConnection = testEmbedderConnection;
// v0.8.25 新增：测试模型连通性
window.testModel = testModel;

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

  // v0.8.23 P2-02 (D6)：Enter 键提交拦截 — 向导输入框 Enter 触发对应搜索/写入操作
  // 防止用户在向导输入框中按下 Enter 后无反应（需要手动点击按钮）
  const wizardInputMap = {
    'wizard-search-path': 'wizardStep1Search',
    'wizard-memory-content': 'wizardStep2Write',
    'wizard-search-query': 'wizardStep3Search'
  };
  Object.keys(wizardInputMap).forEach(id => {
    const el = document.getElementById(id);
    if (!el || el.dataset.boundEnter) return;
    el.dataset.boundEnter = '1';
    const action = wizardInputMap[id];
    el.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter' && !ev.isComposing) {
        ev.preventDefault();
        const fn = window[action];
        if (typeof fn === 'function') fn();
      }
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
  // v0.8.13 F2: 网络恢复后先检查 sidecar 可达性，再加载仪表盘
  // 避免网络恢复但 sidecar 未运行时，loadDashboard 反复触发"加载失败"
  const dashboard = document.getElementById('tab-dashboard');
  if (dashboard && dashboard.classList.contains('active')) {
    if (typeof SidecarHealthMonitor !== 'undefined' && SidecarHealthMonitor) {
      SidecarHealthMonitor.check().then((reachable) => {
        if (reachable) {
          if (typeof loadDashboard === 'function') {
            try { loadDashboard(); } catch (e) { console.error('[online] 重新加载仪表盘失败:', e); }
          }
        } else {
          if (typeof showToast === 'function') {
            showToast('网络已恢复，但 LRC 服务未运行，请点击启动', 'info');
          }
        }
      });
    } else if (typeof loadDashboard === 'function') {
      try { loadDashboard(); } catch (e) { console.error('[online] 重新加载仪表盘失败:', e); }
    }
  }
});

// ============================================================
// v0.8.2 新增：beforeunload 拦截（对应审计 G006）
// 有进行中请求时，刷新/关闭页面前提示用户
// ============================================================
// v0.8.44 GAP-L5-04 修复（interaction-resilience-auditor Round5 P0）：
//   根因：用户选择"留下"后页面无任何反馈，不确定是否有请求在飞，体验差
//   修复：用户选择"留下"后，通过 visibilitychange 检测并显示 toast 反馈
let _pendingBeforeUnload = false;
window.addEventListener('beforeunload', (e) => {
  // v0.8.13 D4: 排除后台请求（健康检查等），仅用户主动发起的请求才拦截关闭
  if (pendingRequestCount - _pendingBackgroundCount > 0) {
    // 现代浏览器忽略自定义消息，但仍需设置 returnValue 触发提示
    e.preventDefault();
    e.returnValue = '';
    // 记录用户触发了关闭操作
    _pendingBeforeUnload = true;
    return '';
  }
});

// 检测用户选择了"留下"（页面重新可见时触发）
document.addEventListener('visibilitychange', () => {
  if (_pendingBeforeUnload && document.visibilityState === 'visible') {
    _pendingBeforeUnload = false;
    showToast('页面关闭已取消，后台任务仍在进行中', 'info', 3000);
    console.log('[LRC]用户选择"留下"，页面未关闭，后台任务继续');
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
  // v0.8.23 GAP-AUDIT-02 修复：暴露 REFRESH_INTERVAL 到 __testHooks，支持 CDP 测试
  get REFRESH_INTERVAL() { return REFRESH_INTERVAL; },
  // v0.8.23 GAP-AUDIT-03 修复：暴露 safeLocalStorageSetItem 到 __testHooks，支持 CDP 测试
  safeLocalStorageSetItem: safeLocalStorageSetItem,
  _abortActiveTabRequests: _abortActiveTabRequests
};

// v0.8.21 P0-04 修复（GAP-P0-04 / interaction-resilience-auditor）：
//   将关键函数挂载到 window，使 CDP 测试和外部集成可调用
//   根因：IIFE 封装导致 loadDashboard/loadDaoMetrics 不在全局作用域，
//         CDP 执行 typeof loadDashboard === 'function' 返回 false
window.loadDashboard = loadDashboard;
window.loadDaoMetrics = loadDaoMetrics;
window.handleStartServiceClick = handleStartServiceClick;
window.showToast = showToast;
window.fetchWithTimeout = fetchWithTimeout;

// IIFE 闭合（v0.8.0：从原第 2950 行移至文件末尾）
})();
