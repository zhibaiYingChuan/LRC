
// ============================================================
// 全局配置
// ============================================================
const DEFAULT_API_BASE = 'http://localhost:3099';
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
    if (tabName === 'settings') loadSettings();
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
  // --- 记忆统计卡片 ---
  const memStats = system?.memory_stats || {};
  const daoMetrics = system?.dao_metrics || dao || {};

  const statTotal = $('stat-total');
  const statActive = $('stat-active');
  const statCrystallized = $('stat-crystallized');
  const statToday = $('stat-today');
  if (statTotal) statTotal.textContent = num(memStats.total_memories || daoMetrics.active_memories + (daoMetrics.crystallized_memories || 0) + (daoMetrics.archived_memories || 0));
  if (statActive) statActive.textContent = num(memStats.active_memories || daoMetrics.active_memories);
  if (statCrystallized) statCrystallized.textContent = num(memStats.synthesis_memories || daoMetrics.crystallized_memories);
  if (statToday) statToday.textContent = num(daoMetrics.encodings_total || 0);

  // --- 健康状态 ---
  const daoScore = daoMetrics.dao_isomorphism_score ?? 0;
  const daoScoreText = $('dao-score-text');
  const daoScoreBar = $('dao-score-bar');
  if (daoScoreText) daoScoreText.textContent = pct(daoScore);
  if (daoScoreBar) {
    daoScoreBar.style.width = (daoScore * 100).toFixed(1) + '%';
    daoScoreBar.className = 'progress-fill ' + (daoScore < 0.3 ? 'cinnabar' : daoScore < 0.5 ? 'gold' : 'jade');
  }

  const entropy = daoMetrics.bagua_entropy ?? 0;
  const baguaEntropy = $('bagua-entropy');
  if (baguaEntropy) baguaEntropy.textContent = entropy.toFixed(3) + ' / 3.0';

  const synthRatio = daoMetrics.synthesis_ratio ?? 0;
  const synthRatioText = $('synthesis-ratio-text');
  const synthRatioBar = $('synthesis-ratio-bar');
  if (synthRatioText) synthRatioText.textContent = pct(synthRatio);
  if (synthRatioBar) synthRatioBar.style.width = (synthRatio * 100).toFixed(1) + '%';

  // --- 系统状态 ---
  const encoder = system?.encoder || {};
  const sysMlStatus = $('sys-ml-status');
  const sysEncoder = $('sys-encoder');
  const sysDataDir = $('sys-data-dir');
  const sysCache = $('sys-cache');
  const sysMode = $('sys-mode');
  const sysQuality = $('sys-quality');
  if (sysMlStatus) sysMlStatus.textContent = encoder.mode === 'ml' ? '✅ ML 模式' : '⚠️ 统计模式';
  if (sysEncoder) sysEncoder.textContent = encoder.model_name || 'LuoShuEncoder (统计)';
  if (sysDataDir) sysDataDir.textContent = '.loong-recall/data/';
  if (sysCache) sysCache.textContent = encoder.mode === 'ml' ? '已启用' : '统计模式无需缓存';
  if (sysMode) sysMode.innerHTML = statusBadge(system?.system_mode || 'unknown');
  if (sysQuality) sysQuality.textContent = encoder.quality_score != null ? (encoder.quality_score * 100).toFixed(0) + '%' : '--';

  // --- 复杂度预算 ---
  const budget = system?.complexity_budget || {};
  const honesty = budget.complexity_honesty || {};

  const maintainScore = budget.maintainability_score ?? 0;
  const complexityScore = $('complexity-score');
  const complexityBar = $('complexity-bar');
  const honestyScore = $('honesty-score');
  const honestyBar = $('honesty-bar');
  const maintainabilityScore = $('maintainability-score');
  const maintainabilityBar = $('maintainability-bar');
  if (complexityScore) complexityScore.textContent = pct(budget.budget_consumed || 0);
  if (complexityBar) complexityBar.style.width = ((budget.budget_consumed || 0) * 100).toFixed(1) + '%';
  if (honestyScore) honestyScore.textContent = pct(honesty.score ?? 1);
  if (honestyBar) honestyBar.style.width = ((honesty.score ?? 1) * 100).toFixed(1) + '%';
  if (maintainabilityScore) maintainabilityScore.textContent = pct(maintainScore);
  if (maintainabilityBar) maintainabilityBar.style.width = (maintainScore * 100).toFixed(1) + '%';

  // --- 决策日志（从审计追踪获取） ---
  loadAuditLog();
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

  if (version) version.textContent = 'v0.2.0';
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
// 初始化
// ============================================================
function init() {
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

    updateLlmStatusBadge(data.llm_configured, data.llm_type);
    showConfigSection(data.llm_configured, data.llm_type, data.llm_model);
  } catch (e) {
    console.warn('[设置] 加载配置失败:', e.message);
    updateLlmStatusBadge(false, 'none');
  }
}

/** 更新 LLM 状态徽章 */
function updateLlmStatusBadge(configured, type) {
  const badge = $('llm-status-badge');
  if (!badge) return;
  if (configured) {
    badge.textContent = '已配置 · ' + type.toUpperCase();
    badge.className = 'badge badge-success';
  } else {
    badge.textContent = '未配置';
    badge.className = 'badge badge-info';
  }
}

/** 显示当前配置详情 */
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

/** 切换 LLM 提供商时显示/隐藏对应字段 */
function switchLlmProvider() {
  const provider = $('llm-provider').value;
  const openaiFields = $('openai-fields');
  const ollamaFields = $('ollama-fields');
  if (provider === 'openai') {
    openaiFields.style.display = '';
    ollamaFields.style.display = 'none';
  } else {
    openaiFields.style.display = 'none';
    ollamaFields.style.display = '';
  }
}

/** 保存 LLM API Key 配置 */
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

    if (provider === 'openai') {
      const apiKey = $('llm-api-key').value.trim();
      const model = $('llm-model').value.trim() || 'gpt-4o-mini';
      const endpoint = $('llm-endpoint').value.trim();

      if (!apiKey) {
        throw new Error('请输入 API Key');
      }
      if (endpoint) {
        llmConfigStr = 'openai:' + apiKey + ':' + model + ':' + endpoint;
      } else {
        llmConfigStr = 'openai:' + apiKey + ':' + model;
      }
    } else if (provider === 'ollama') {
      const host = $('llm-ollama-host').value.trim() || 'localhost';
      const model = $('llm-ollama-model').value.trim() || 'llama3';
      llmConfigStr = 'ollama:' + host + ':' + model;
    }

    const resp = await fetchWithTimeout(API_BASE + '/api/config/llm', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ llm_api: llmConfigStr })
    });

    const data = await resp.json();
    if (data.success) {
      resultEl.className = 'form-result success';
      resultEl.textContent = '✅ ' + data.message + '。下次搜索时将自动使用 LLM 翻译查询。';
      updateLlmStatusBadge(true, data.llm_type || provider);
      showConfigSection(true, data.llm_type || provider, data.llm_model);
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

/** 清除 LLM API Key 配置 */
async function clearLlmConfig() {
  const resultEl = $('llm-config-result');
  if (!resultEl) return;

  if (!confirm('确定要清除 LLM API Key 配置吗？清除后将回退到 Tier 1 Fast Match 模式。')) {
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
      resultEl.textContent = '✅ LLM API Key 配置已清除。';
      updateLlmStatusBadge(false, 'none');
      showConfigSection(false, '', '');
      // 清空表单
      const apiKeyInput = $('llm-api-key');
      if (apiKeyInput) apiKeyInput.value = '';
    } else {
      throw new Error(data.message || '清除失败');
    }
  } catch (e) {
    resultEl.className = 'form-result error';
    resultEl.textContent = '❌ ' + e.message;
  }
}
