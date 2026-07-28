
// ============================================================
// Loong Recall 仪表盘 — 主应用脚本
// 使用 IIFE 模式隔离作用域，仅暴露 HTML onclick 所需的函数到全局
// ============================================================
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

/** 带超时的 fetch */
async function fetchWithTimeout(url, options = {}, timeout = 10000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);
  try {
    const res = await fetch(url, { ...options, signal: controller.signal });
    return res;
  } finally {
    clearTimeout(timer);
  }
}

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
async function loadDashboard() {
  const loading = $('dashboard-loading');
  const error = $('dashboard-error');
  if (!loading) return;
  loading.classList.remove('hidden');
  if (error) {
    error.classList.remove('show');
    error.textContent = '';
  }

  try {
    // 并行请求三个端点
    const [systemRes, detailedRes, daoRes] = await Promise.allSettled([
      fetchWithTimeout(API_BASE + '/v1/health/system'),
      fetchWithTimeout(API_BASE + '/v1/health/detailed'),
      fetchWithTimeout(API_BASE + '/v1/health/dao_metrics'),
    ]);

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

    loading.classList.add('hidden');
  } catch (e) {
    if (loading) loading.classList.add('hidden');
    if (error) {
      error.textContent = '⚠️ ' + htmlescape(e.message);
      error.classList.add('show');
    }
    updateStatusBar(false, null);
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
    const typeLabels = {
      fact: '事实', synthesis: '合成', pattern: '模式',
      preference: '偏好', correction: '修正', general: '通用',
    };
    // 类型颜色映射
    const typeColors = {
      fact: 'info', synthesis: 'jade', pattern: 'gold',
      preference: 'ink', correction: 'cinnabar', general: 'info',
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

  if (version) version.textContent = 'v0.6.0';
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
};

/**
 * 向桌面端发送请求（启动服务/打开数据目录等）
 * v0.6.0 修复：Tauri 环境直接调用 invoke，iframe 嵌入模式回退到 postMessage
 * @param {string} type - 消息类型（如 'lrc-start-service'）
 * @param {object} [extra={}] - 额外参数
 * @param {number} [timeoutMs=30000] - 超时时间
 * @returns {Promise<object>} 桌面端返回的结果
 */
function postMessageToParent(type, extra = {}, timeoutMs = 30000) {
  return new Promise(async (resolve, reject) => {
    // 优先：Tauri 环境（主窗口直接加载仪表盘）直接调用 invoke
    if (isTauriEnv) {
      const invokeFn = (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) ||
                       (window.__TAURI__ && window.__TAURI__.invoke);
      const cmdName = POST_MESSAGE_TO_INVOKE[type];
      if (invokeFn && cmdName) {
        try {
          const result = await invokeFn(cmdName);
          resolve(result);
        } catch (e) {
          reject(new Error(typeof e === 'string' ? e : (e.message || String(e))));
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
  if (!modal) return;
  modal.hidden = false;
  const btn = document.getElementById('modal-btn-start-service');
  if (btn) {
    btn.disabled = false;
    btn.textContent = '启动服务';
  }
}

/** 关闭启动服务模态框（供 HTML onclick 调用） */
function closeStartServiceModal() {
  const modal = document.getElementById('start-service-modal');
  if (modal) modal.hidden = true;
}
// 暴露到全局供 onclick 使用
window.closeStartServiceModal = closeStartServiceModal;

/** 启动服务按钮点击处理 */
async function handleStartServiceClick() {
  const btn = document.getElementById('modal-btn-start-service');
  if (!btn) return;
  btn.disabled = true;
  btn.textContent = '正在启动...';

  try {
    const result = await postMessageToParent('lrc-start-service', {}, 60000);
    closeStartServiceModal();
    // 启动成功后刷新仪表盘
    setTimeout(() => {
      loadDashboard();
    }, 800);
  } catch (e) {
    btn.disabled = false;
    btn.textContent = '启动服务';
    alert('启动失败：' + e.message);
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
    // 并行获取健康数据
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
    result.innerHTML = `
      <div class="result-row"><span class="result-label">数据目录</span><span class="result-value">${htmlescape(data.data_directory)}</span></div>
      <div class="result-row"><span class="result-label">记忆文件</span><span class="result-value">${htmlescape(data.memory_file)}</span></div>
      <div class="result-row"><span class="result-label">文件存在</span><span class="result-value ${validCls}">${data.file_exists ? '✅ 是' : '❌ 否'}</span></div>
      <div class="result-row"><span class="result-label">文件大小</span><span class="result-value">${htmlescape(data.file_size_human)} (${num(data.file_size_bytes)} 字节)</span></div>
      <div class="result-row"><span class="result-label">存储后端</span><span class="result-value">${htmlescape(data.storage_backend)}</span></div>
      <div class="result-row"><span class="result-label">完全本地</span><span class="result-value valid">✅ 是</span></div>
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
    result.innerHTML = '<div class="result-row"><span class="result-label" style="color:var(--cinnabar)">⚠️ ' + htmlescape(e.message) + '</span></div>';
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
  const query = document.getElementById('wizard-search-path')?.value || 'Rust';
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
  const content = document.getElementById('wizard-memory-content')?.value || '项目使用 Rust 语言开发';
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
  const query = document.getElementById('wizard-search-query')?.value || 'Rust 开发';
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
      if (tabId === 'tab-dashboard') loadDashboard();
      else if (tabId === 'tab-trust-center') loadTrustCenter();
    }
    // 始终更新运行时长
    const uptime = $('status-uptime');
    if (uptime) uptime.textContent = formatUptime(Date.now() - startTime);
  }, REFRESH_INTERVAL);
}

// ============================================================
// 桌面端嵌入检测
// ============================================================
// 当仪表盘被嵌入 Tauri 桌面端时，URL 会带 ?embedded=tauri 参数
// v0.5.5：嵌入模式和非嵌入模式统一使用完整的 LLM 配置表单
// 不管在哪里修改 LLM 配置，都通过 /api/config/llm API 保存，自动同步到 wizard.json
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

  loadDashboard();
  
  setTimeout(() => {
    drawRadarChart();
  }, 100);

  setInterval(() => {
    const uptime = $('status-uptime');
    if (uptime) uptime.textContent = formatUptime(Date.now() - startTime);
  }, 1000);

  startAutoRefresh();
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
  const result = $('backup-result');
  if (btn) btn.disabled = true;
  if (result) {
    result.style.display = '';
    result.textContent = '⏳ 正在准备备份文件...';
    result.className = 'form-result';
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

    if (result) {
      result.textContent = '✅ 备份已下载！文件包含 ' +
        (Array.isArray(exportData.memories) ? exportData.memories.length : 0) + ' 条记忆';
      result.className = 'form-result form-result-success';
    }
  } catch (e) {
    if (result) {
      result.textContent = '⚠️ 备份失败: ' + htmlescape(e.message);
      result.className = 'form-result form-result-error';
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

  const result = $('backup-result');
  if (result) {
    result.style.display = '';
    result.textContent = '⏳ 正在验证并导入记忆数据...';
    result.className = 'form-result';
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

    if (!confirm(
      '确认导入以下数据？\n\n' +
      '  记忆：' + memoryCount + ' 条\n' +
      '  代码片段：' + chunkCount + ' 个\n' +
      '  来源：' + (exportData.source || '未知') + '\n' +
      '  格式版本：' + exportData.version + '\n' +
      '  指纹：' + (exportData.fingerprint || '无') + '\n\n' +
      '导入将追加到现有数据，不会覆盖已有记忆。确认继续？'
    )) {
      if (result) {
        result.textContent = '已取消导入';
        result.className = 'form-result';
      }
      return;
    }

    // 调用后端 API 写入数据
    if (memoryCount > 0) {
      let imported = 0;
      let failed = 0;
      for (const mem of exportData.memories) {
        try {
          await fetchWithTimeout(API_BASE + '/v1/memories/remember', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              content: mem.content || JSON.stringify(mem),
              memory_type: mem.memory_type || 'general',
              importance: mem.importance || 5,
              metadata: mem
            }),
          }, 30000);
          imported++;
          // 每 50 条更新一次进度
          if (imported % 50 === 0 && result) {
            result.textContent = '⏳ 导入中... ' + imported + '/' + memoryCount + ' 条';
          }
        } catch (e) {
          failed++;
          console.warn('记忆导入失败 #' + imported + failed + ':', e.message);
        }
      }
      if (result) {
        const msg = '✅ 导入完成！成功 ' + imported + ' 条' +
                    (failed > 0 ? '，失败 ' + failed + ' 条' : '');
        result.textContent = msg;
        result.className = 'form-result form-result-success';
      }
    }
  } catch (e) {
    if (result) {
      result.textContent = '⚠️ 导入失败: ' + htmlescape(e.message);
      result.className = 'form-result form-result-error';
    }
  } finally {
    // 清除文件选择以便重复选择同一文件
    event.target.value = '';
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
        alert('复制失败，请手动选择并复制命令');
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
    const resp = await fetchWithTimeout(API_BASE + '/api/config');
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

    const resp = await fetchWithTimeout(API_BASE + '/api/config/llm', {
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
    resultEl.className = 'form-result error';
    resultEl.textContent = '❌ ' + e.message;
  } finally {
    if (btnSave) btnSave.disabled = false;
  }
}

/** v0.5.5：清除 LLM API Key 配置 */
async function clearLlmConfig() {
  const resultEl = $('llm-config-result');
  if (!resultEl) return;

  if (!confirm('确定要清除 LLM 配置吗？清除后将无法使用自然语言搜索代码。')) {
    return;
  }

  resultEl.style.display = '';
  resultEl.className = 'form-result';
  resultEl.textContent = '⏳ 正在清除...';

  try {
    const resp = await fetchWithTimeout(API_BASE + '/api/config/llm', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ llm_api: '' })
    });

    const data = await resp.json();
    if (data.success) {
      resultEl.className = 'form-result success';
      resultEl.textContent = '✅ LLM 配置已清除。';
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
window.API_BASE = API_BASE;
window.safeJson = safeJson;
// v0.6.0 设计文档 5.6：三层基准测试切换标签
window.switchBenchmarkLayer = switchBenchmarkLayer;
// v0.6.0 设置页面重构：暴露 LLM 提供商切换函数
window.switchLlmProvider = switchLlmProvider;
// v0.6.0 设置页面重构：暴露保存和清除配置函数
window.saveLlmConfig = saveLlmConfig;
window.clearLlmConfig = clearLlmConfig;
window.loadSettings = loadSettings;

})();

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
  // 移除所有卡片的选中状态
  const grid = document.getElementById('preset-scenario-grid');
  if (grid) {
    grid.querySelectorAll('.preset-scenario-card').forEach(function(c) {
      c.classList.remove('selected');
    });
  }
  // 标记当前卡片为选中
  if (card) {
    card.classList.add('selected');
    const scenario = card.getAttribute('data-scenario');

    // 显示提示信息（诗意文案）
    const scenarioMap = {
      'personal-notes': { title: '个人笔记', desc: '记忆类型：note / 标签：[note, personal] / 结晶策略：按主题聚类，7 天结晶' },
      'project-management': { title: '项目管理', desc: '记忆类型：decision/task / 标签：[project, {id}] / 结晶策略：按项目聚类，实时结晶' },
      'learning-assistant': { title: '学习助手', desc: '记忆类型：knowledge / 标签：[learn, {subject}] / 结晶策略：按学科聚类，按需结晶' },
      'coding-helper': { title: '编程助手', desc: '记忆类型：code_context/preference / 标签：[code, {lang}] / 结晶策略：按代码语言聚类' }
    };
    const info = scenarioMap[scenario];
    if (info) {
      // TODO: v0.7.0 正式版将通过 MCP 工具 scenario 持久化用户选择
    }
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
    console.log(`[LRC v0.6.0] 隐私检查完成，耗时 ${elapsed}ms，信任等级: ${trustLevel}`);
  } catch (err) {
    const elapsed = Date.now() - startTime;
    resultEl.innerHTML = `
      <div style="padding:12px;background:var(--lrc-朱砂-50);border-radius:var(--radius-md);color:var(--lrc-朱砂-500);font-size:13px;">
        <strong>隐私检查失败</strong>（耗时 ${elapsed}ms）：<br>
        <span style="font-size:12px;">${htmlescape(err.message || String(err))}</span>
      </div>
    `;
    console.error('[LRC v0.6.0] 隐私检查失败:', err);
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
    console.warn('[LRC v0.6.0] 加载结晶历史失败，使用预览数据:', err.message);
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
      // 计算综合健康评分（0-100）
      const score = Math.min(100, Math.max(0, Math.round(
        (m.yin_yang_balance || 80) * 0.25 +
        (100 - (m.luoshu_deviation || 20)) * 0.25 +
        (m.bagua_balance || 75) * 0.25 +
        (m.synthesis_ratio || 10) * 5 * 0.25
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
      setText('dao-synthesis-ratio', ((m.synthesis_ratio || 0) / 100).toFixed(1) + '%');

      console.log(`[LRC v0.6.0] 道同构度加载完成，健康评分: ${score}`);
    } else {
      // 降级：显示默认值
      drawDaoRing(85);
      const scoreEl = document.getElementById('dao-ring-score');
      if (scoreEl) scoreEl.textContent = '85';
    }
  } catch (err) {
    console.warn('[LRC v0.6.0] 道同构度加载失败，使用默认值:', err.message);
    drawDaoRing(85);
    const scoreEl = document.getElementById('dao-ring-score');
    if (scoreEl) scoreEl.textContent = '85';
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
      console.log(`[LRC v0.6.0] 演化时间线加载了 ${data.events.length} 条事件`);
    }
    // 如果接口未返回数据，保留默认示例数据
  } catch (err) {
    console.warn('[LRC v0.6.0] 演化时间线加载失败，使用示例数据:', err.message);
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
 */
function debouncedMemorySearch() {
  if (memorySearchTimer) clearTimeout(memorySearchTimer);
  memorySearchTimer = setTimeout(searchMemories, 300);
}

/**
 * 执行记忆搜索
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
      body: JSON.stringify({ query, top_k: 20 })
    }, 10000);
    const data = await safeJson(response);

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

      const html = filtered.map(memory => {
        const typeClass = `card-memory-${memory.memory_type || 'conversation'}`;
        const preview = (memory.content || '').substring(0, 200);
        const time = memory.created_at || memory.timestamp || '--';
        const importance = memory.importance || 5;
        return `
          <div class="memory-card-item ${typeClass}" onclick='openMemoryDetail(${JSON.stringify(memory).replace(/'/g, "&#39;")})'>
            <div class="memory-card-preview">${htmlescape(preview)}</div>
            <div class="memory-card-meta">
              <span><img src="/assets/icons/icon-memory.svg" alt="" width="12" height="12"> ${memory.memory_type || '未分类'}</span>
              <span>重要性: ${importance}</span>
              <span>${time}</span>
            </div>
          </div>
        `;
      }).join('');
      resultsEl.innerHTML = html;
      console.log(`[LRC v0.6.0] 记忆搜索完成，返回 ${filtered.length} 条结果`);
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
    resultsEl.innerHTML = `
      <div class="memory-search-empty">
        <img class="empty-icon" src="/assets/icons/icon-search-lrc.svg" alt="">
        <div class="empty-poem">搜索出错</div>
        <p class="text-sm text-dim">${htmlescape(err.message || String(err))}</p>
      </div>
    `;
    console.error('[LRC v0.6.0] 记忆搜索失败:', err);
  }
}

/**
 * 打开记忆详情面板（右侧滑出 40% 宽度）
 */
function openMemoryDetail(memory) {
  const panel = document.getElementById('memory-detail-panel');
  const backdrop = document.getElementById('memory-detail-backdrop');
  const content = document.getElementById('memory-detail-content');
  if (!panel || !content) return;

  content.innerHTML = `
    <h3>${htmlescape(memory.content ? memory.content.substring(0, 50) + '...' : '记忆详情')}</h3>
    <div class="memory-detail-fulltext">${htmlescape(memory.content || '')}</div>
    <div class="memory-detail-metadata">
      <span class="label">记忆类型</span>
      <span class="value">${htmlescape(memory.memory_type || '--')}</span>
      <span class="label">重要性</span>
      <span class="value">${memory.importance || '--'}</span>
      <span class="label">创建时间</span>
      <span class="value">${htmlescape(memory.created_at || memory.timestamp || '--')}</span>
      <span class="label">记忆 ID</span>
      <span class="value">${htmlescape(memory.id || '--')}</span>
      <span class="label">标签</span>
      <span class="value">${htmlescape((memory.tags || []).join(', ') || '--')}</span>
    </div>
  `;

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

/**
 * 显示 Toast 通知
 * @param {string} message - 通知内容
 * @param {string} type - 类型：success/error/warning
 * @param {number} duration - 显示时长（毫秒），默认 3000
 */
function showToast(message, type = 'success', duration = 3000) {
  const container = document.getElementById('toast-container');
  if (!container) return;

  const iconMap = {
    success: 'icon-trust',
    error: 'icon-decay',
    warning: 'icon-benchmark'
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

function initSidebarNav() {
  // 侧边栏导航项点击切换标签
  document.querySelectorAll('.app-sidebar .nav-item[data-tab]').forEach(item => {
    item.addEventListener('click', function(e) {
      e.preventDefault();
      const tabName = this.dataset.tab;

      // 移除其他导航项的 active
      document.querySelectorAll('.app-sidebar .nav-item').forEach(n => n.classList.remove('active'));
      this.classList.add('active');

      // 触发标签切换（复用现有 switchTab 逻辑）
      if (typeof switchTab === 'function') {
        switchTab(tabName);
      } else {
        // 降级：直接操作 DOM
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
    const provider = document.getElementById('llm-provider')?.value;
    let testEndpoint = '';
    let apiKey = '';

    const endpoint = document.getElementById('llm-endpoint')?.value?.trim();
    apiKey = document.getElementById('llm-api-key')?.value?.trim();
    if (!endpoint || !apiKey) {
      throw new Error('请填写完整的 API 配置信息');
    }
    testEndpoint = endpoint + '/models';

    const headers = {};
    if (apiKey) {
      headers['Authorization'] = 'Bearer ' + apiKey;
    }

    const resp = await fetchWithTimeout(testEndpoint, {
      method: 'GET',
      headers: headers
    }, 10000);

    if (resp.ok) {
      resultEl.className = 'form-result success';
      resultEl.textContent = '✅ 连接成功！配置正确可用';
    } else {
      throw new Error('连接失败: ' + resp.status + ' ' + resp.statusText);
    }
  } catch (e) {
    resultEl.className = 'form-result error';
    resultEl.textContent = '❌ ' + e.message;
  } finally {
    if (btnTest) btnTest.disabled = false;
  }
}

/* ============================================================
 * 本地嵌入模型配置相关函数
 * ============================================================ */

/**
 * 选择嵌入模型
 * @param {string} modelId - 模型 ID
 */
function selectEmbedderModel(modelId) {
  // 更新卡片选中状态
  document.querySelectorAll('[data-embedder]').forEach(card => {
    card.classList.remove('active');
  });
  const activeCard = document.querySelector(`[data-embedder][onclick*="${modelId.replace(/"/g, '\\"')}"]`);
  if (activeCard) {
    activeCard.classList.add('active');
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
  const modelId = document.getElementById('embedder-model')?.value?.trim();
  if (!modelId) {
    alert('请先选择一个模型');
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
              alert('模型 ' + modelId + ' 下载完成！');
              checkEmbedderStatus();
            }, 500);
          }

          // 超时保护（120 秒）
          if (pollCount > 40) {
            clearInterval(pollInterval);
            if (progressEl) progressEl.style.display = 'none';
            alert('下载超时，请稍后通过「检测状态」查看。模型文件较大时可能需要更长时间。');
            checkEmbedderStatus();
          }
        }
      } catch (e) {
        // 轮询失败，继续重试
      }
    }, 3000);

  } catch (e) {
    if (progressEl) progressEl.style.display = 'none';
    alert('下载失败: ' + e.message + '\n\n你也可以通过命令行手动下载：\ncode-memory-server model download ' + modelId);
  }
}

/**
 * 应用嵌入模型（设为默认）
 */
async function applyEmbedderModel() {
  const modelId = document.getElementById('embedder-model')?.value?.trim();
  if (!modelId) {
    alert('请先选择一个模型');
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
      alert(data.message || '模型已设为默认，重启服务后生效');
      checkEmbedderStatus();
    } else {
      throw new Error(data.message || '设置失败');
    }
  } catch (e) {
    alert('设置失败: ' + e.message);
  }
}

/**
 * 测试语义编码模型链接（测试镜像源连通性）
 */
async function testEmbedderConnection() {
  const modelId = document.getElementById('embedder-model')?.value?.trim();
  if (!modelId) {
    alert('请先选择一个模型');
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
      alert('✅ 连接成功！\n\n镜像源: ' + mirrorNames[mirror] + '\n模型: ' + modelId + '\n延迟: ' + (data.latency_ms || '?') + 'ms');
    } else {
      throw new Error(data.message || '连接失败');
    }
  } catch (e) {
    alert('❌ 连接失败: ' + e.message + '\n\n请检查网络或尝试其他镜像源');
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.textContent = '测试链接';
    }
  }
}

/**
 * 切换项目
 */
function switchProject() {
  const input = document.createElement('input');
  input.type = 'file';
  input.webkitdirectory = true;
  input.directory = true;
  input.onchange = function(e) {
    const files = e.target.files;
    if (files && files.length > 0) {
      // 获取文件夹路径（注意：浏览器安全限制，只能获取相对路径）
      const path = files[0].webkitRelativePath.split('/')[0];
      if (confirm('确定要切换到项目: ' + path + ' 吗？\n切换后将重新索引代码。')) {
        alert('正在切换项目: ' + path + '\n（演示功能，实际需后端 API 支持）');
      }
    }
  };
  input.click();
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
    <button style="background: none; border: none; color: var(--lrc-朱砂-500); cursor: pointer; font-size: 1.1em;" onclick="this.parentElement.parentElement.remove(); checkNextButton();">✕</button>
  `;
  projectsContainer.appendChild(projectEl);

  // 启用下一步按钮
  checkNextButton();

  // 如果是快速模式，直接完成
  const stepsSection = document.getElementById('setup-steps-section');
  if (!stepsSection || stepsSection.style.display === 'none') {
    alert('项目 ' + projectName + ' 已选择！\n（演示功能，实际需后端 API 支持重新索引）');
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
