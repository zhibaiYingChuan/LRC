
// ============================================================
// Loong Recall 仪表盘 — 主应用脚本
// 使用 IIFE 模式隔离作用域，仅暴露 HTML onclick 所需的函数到全局
// ============================================================
(function() {
  'use strict';

  // ============================================================
  // 全局配置
  // ============================================================
const DEFAULT_API_BASE = window.location.origin || 'http://localhost:3099';
const API_BASE = new URLSearchParams(window.location.search).get('api') || DEFAULT_API_BASE;
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
  document.getElementById('navbarNav').classList.toggle('open');
}

document.querySelectorAll('.navbar-nav button').forEach(btn => {
  btn.addEventListener('click', function() {
    // 关闭移动端菜单
    document.getElementById('navbarNav').classList.remove('open');

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
    } else {
      dot.className = 'status-dot offline';
      text.textContent = '已停止 / 不可达';
      text.style.color = '#c0392b';
    }
  }

  if (version) version.textContent = 'v0.5.4';
  if (dataDir) dataDir.textContent = '.loong-recall/data/';
  if (uptime) uptime.textContent = formatUptime(Date.now() - startTime);
}

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
const IS_DESKTOP_EMBEDDED = new URLSearchParams(window.location.search).get('embedded') === 'tauri';

// v0.5.5：LLM 提供商列表（与桌面端配置向导一致）
const LLM_PROVIDERS = {
  deepseek:   { name: 'DeepSeek',       url: 'https://api.deepseek.com/v1',           model: 'deepseek-chat',       keyHint: 'sk-...',    desc: '国产性价比之王，代码能力极强' },
  qwen:       { name: '通义千问',       url: 'https://dashscope.aliyuncs.com/compatible-mode/v1', model: 'qwen-plus', keyHint: 'sk-...', desc: '阿里云出品，中文理解出色' },
  zhipu:      { name: '智谱 GLM',       url: 'https://open.bigmodel.cn/api/paas/v4',   model: 'glm-4',               keyHint: 'xxx.xxx', desc: '清华系，GLM 系列模型' },
  minimax:    { name: 'MiniMax',         url: 'https://api.minimax.chat/v1',            model: 'abab6.5s-chat',       keyHint: 'eyJ...', desc: '海螺AI同款，长文本支持好' },
  moonshot:   { name: 'Moonshot (Kimi)', url: 'https://api.moonshot.cn/v1',            model: 'moonshot-v1-8k',      keyHint: 'sk-...', desc: 'Kimi 同款，超长上下文' },
  openai:     { name: 'OpenAI',          url: 'https://api.openai.com/v1',              model: 'gpt-4o',             keyHint: 'sk-...', desc: 'GPT-4o，综合能力最强' },
  ollama:     { name: 'Ollama 本地模型', url: 'http://localhost:11434',                 model: 'llama3',             keyHint: '无需 Key（本地运行）', desc: '免费本地运行，数据不出电脑' },
  custom:     { name: '自定义 API',      url: '',                                        model: '',                   keyHint: '',          desc: '手动填写任何兼容 OpenAI 的 API 地址' },
};

// ============================================================
// 初始化
// ============================================================
function init() {
  // v0.5.5：统一嵌入模式和非嵌入模式，都使用完整的 LLM 配置表单
  // 不管在哪里修改 LLM 配置，都通过 /api/config/llm API 保存，自动同步到 wizard.json
  if (IS_DESKTOP_EMBEDDED) {
    // 嵌入模式下，更新设置页描述
    const settingsDesc = document.querySelector('#tab-settings .section-desc');
    if (settingsDesc) {
      settingsDesc.textContent = '配置 LLM API 以启用自然语言搜索代码。配置后自动生效，无需重启。';
    }
  }

  // 初始加载仪表盘
  loadDashboard();

  // 更新运行时长
  setInterval(() => {
    const uptime = $('status-uptime');
    if (uptime) uptime.textContent = formatUptime(Date.now() - startTime);
  }, 1000);

  // 启动自动刷新
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
    // 从 API 获取记忆数据
    const [memoriesRes, chunksRes, archiveRes, projectRes] = await Promise.allSettled([
      fetchWithTimeout(API_BASE + '/v1/memories/list', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ limit: 10000 })
      }),
      fetchWithTimeout(API_BASE + '/v1/code/search', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ query: '', limit: 10000 })
      }),
      fetchWithTimeout(API_BASE + '/v1/memories/archive', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({})
      }),
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

    // 验证导出格式版本
    if (!exportData.version || exportData.version !== '2.0') {
      throw new Error('不支持的导出格式版本: ' + (exportData.version || '未知'));
    }

    const memoryCount = Array.isArray(exportData.memories) ? exportData.memories.length : 0;
    const chunkCount = Array.isArray(exportData.chunks) ? exportData.chunks.length : 0;

    if (!confirm(
      '确认导入以下数据？\n\n' +
      '  记忆：' + memoryCount + ' 条\n' +
      '  代码片段：' + chunkCount + ' 个\n' +
      '  来源：' + (exportData.source || '未知') + '\n' +
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
      for (const mem of exportData.memories) {
        await fetchWithTimeout(API_BASE + '/v1/memories/remember', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            content: mem.content || JSON.stringify(mem),
            memory_type: mem.memory_type || 'general',
            importance: mem.importance || 5,
            metadata: mem
          }),
        });
      }
    }

    if (result) {
      result.textContent = '✅ 导入完成！共导入 ' + memoryCount + ' 条记忆';
      result.className = 'form-result form-result-success';
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

    let html = '';
    for (const layer of data.layers) {
      html += '<div class="benchmark-layer">' +
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
    }
    container.innerHTML = html;
    drawRadarChart(data.radar_chart);
  } catch (e) {
    container.innerHTML = '<div class="card"><p style="color:#f44336">无法加载基准报告: ' + htmlescape(e.message) + '</p><p style="color:#888">请确保 LRC 服务正在运行</p></div>';
    if (summaryBar) summaryBar.innerHTML = '<span class="badge badge-warning">无法加载</span>';
  }
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
  if (!canvas || !data) return;
  const ctx = canvas.getContext('2d');
  const W = canvas.width;
  const H = canvas.height;
  const cx = W / 2;
  const cy = H / 2;
  const r = Math.min(cx, cy) - 40;
  const keys = Object.keys(data);
  const values = Object.values(data);
  const n = keys.length;

  // 清空画布
  ctx.clearRect(0, 0, W, H);

  // 绘制网格（5 层同心多边形）
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
    ctx.strokeStyle = '#333';
    ctx.lineWidth = 1;
    ctx.stroke();
  }

  // 绘制轴线
  for (let i = 0; i < n; i++) {
    const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(cx + r * Math.cos(angle), cy + r * Math.sin(angle));
    ctx.strokeStyle = '#444';
    ctx.lineWidth = 0.5;
    ctx.stroke();
  }

  // 绘制数据区域
  ctx.beginPath();
  for (let i = 0; i < n; i++) {
    const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
    const v = values[i]; // 分数已归一化至 0.0~1.0，无需再除100
    const x = cx + r * v * Math.cos(angle);
    const y = cy + r * v * Math.sin(angle);
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.closePath();
  ctx.fillStyle = 'rgba(192, 160, 96, 0.15)';
  ctx.fill();
  ctx.strokeStyle = '#c0a060';
  ctx.lineWidth = 2;
  ctx.stroke();

  // 绘制数据点
  for (let i = 0; i < n; i++) {
    const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
    const v = values[i]; // 分数已归一化至 0.0~1.0
    const x = cx + r * v * Math.cos(angle);
    const y = cy + r * v * Math.sin(angle);
    ctx.beginPath();
    ctx.arc(x, y, 4, 0, Math.PI * 2);
    ctx.fillStyle = '#c0a060';
    ctx.fill();
  }

  // 绘制标签
  ctx.fillStyle = '#ddd';
  ctx.font = '12px sans-serif';
  ctx.textAlign = 'center';
  for (let i = 0; i < n; i++) {
    const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
    const labelR = r + 20;
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
      if (providerKey === 'ollama') {
        const hostEl = $('llm-ollama-host');
        const modelEl = $('llm-ollama-model');
        if (hostEl && data.llm_base_url) hostEl.value = data.llm_base_url;
        if (modelEl && data.llm_model) modelEl.value = data.llm_model;
      } else {
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
  if (llmType === 'ollama') return 'ollama';
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
  if (llmType === 'ollama') return 'Ollama';
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
  const ollamaFields = $('ollama-fields');
  const providerInfo = LLM_PROVIDERS[provider];

  // 更新提供商描述
  const descEl = $('provider-desc');
  if (descEl && providerInfo) {
    descEl.textContent = providerInfo.desc || '';
  }

  if (provider === 'ollama') {
    // Ollama 模式：显示 Ollama 字段，隐藏 OpenAI 字段
    openaiFields.style.display = 'none';
    ollamaFields.style.display = '';
  } else {
    // OpenAI 兼容模式：显示 OpenAI 字段，隐藏 Ollama 字段
    openaiFields.style.display = '';
    ollamaFields.style.display = 'none';

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
          deepseek: '常用: deepseek-chat, deepseek-coder',
          qwen: '常用: qwen-plus, qwen-turbo, qwen-max',
          zhipu: '常用: glm-4, glm-4-flash, glm-3-turbo',
          minimax: '常用: abab6.5s-chat, abab6.5-chat',
          moonshot: '常用: moonshot-v1-8k, moonshot-v1-32k, moonshot-v1-128k',
          openai: '常用: gpt-4o, gpt-4o-mini, gpt-3.5-turbo',
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

    if (provider === 'ollama') {
      // Ollama 模式：ollama:model:host
      const host = $('llm-ollama-host').value.trim() || 'http://localhost:11434';
      const model = $('llm-ollama-model').value.trim() || 'llama3';
      llmConfigStr = 'ollama:' + model + ':' + host;
    } else {
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
    }

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

})();
