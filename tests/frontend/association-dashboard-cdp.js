#!/usr/bin/env node
/**
 * LRC 记忆联想执行仪表盘 CDP 契约测试。
 * 固定连接开发版 WebView CDP 9231，并校验开发 sidecar 3111 的统计结果。
 */
'use strict';

const WS = require('ws');
const CDP_PORT = 9231;
const API_BASE = 'http://127.0.0.1:3111';

class CdpClient {
  constructor(wsUrl) {
    this.ws = new WS(wsUrl);
    this.nextId = 1;
    this.pending = new Map();
    this.readyPromise = new Promise((resolve, reject) => {
      this.ws.once('open', resolve);
      this.ws.once('error', reject);
    });
    this.ws.on('message', data => {
      let message;
      try { message = JSON.parse(data.toString()); } catch { return; }
      if (!message.id || !this.pending.has(message.id)) return;
      const entry = this.pending.get(message.id);
      clearTimeout(entry.timer);
      this.pending.delete(message.id);
      if (message.error) entry.reject(new Error(message.error.message));
      else entry.resolve(message.result);
    });
  }

  async eval(expression) {
    await this.readyPromise;
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error('CDP Runtime.evaluate 超时'));
      }, 15000);
      this.pending.set(id, { resolve: result => {
        if (result.exceptionDetails) {
          reject(new Error(result.exceptionDetails.text || '页面脚本异常'));
        } else {
          resolve(result.result?.value);
        }
      }, reject, timer });
      this.ws.send(JSON.stringify({
        id,
        method: 'Runtime.evaluate',
        params: { expression, returnByValue: true, awaitPromise: true }
      }));
    });
  }

  close() { this.ws.close(); }
}

async function getPage() {
  const response = await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`);
  if (!response.ok) throw new Error(`CDP 页面列表失败: ${response.status}`);
  const pages = await response.json();
  // v0.9.6：dev 模式下 WebView 可能停留在打包副本（tauri.localhost）
  // 或 dev-proxy（localhost:1420，直接服务磁盘最新 static/），两者均有效
  // 优先选择 dev-proxy，确保验证磁盘上的最新 static/app.js；打包副本仅作回退
  const page = pages.find(item => item.type === 'page'
    && /localhost:1420/.test(item.url || ''))
    || pages.find(item => item.type === 'page'
      && /tauri\.localhost/.test(item.url || ''));
  if (!page?.webSocketDebuggerUrl) throw new Error('未找到 LRC 开发版 WebView');
  return page;
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  // v0.9.7：前端已不再读取 /v1/feedback/association-stats（本地工具不收集用户反馈埋点），
  // 本契约测试同步移除反馈统计断言，仅保留执行活动与记忆统计契约。
  const memoryStatsResponse = await fetch(`${API_BASE}/v1/memories/stats`);
  assert(memoryStatsResponse.ok, `记忆统计接口失败: ${memoryStatsResponse.status}`);
  const memoryStats = await memoryStatsResponse.json();
  assert(typeof memoryStats.total_memories === 'number', '记忆统计缺少 total_memories 字段');
  assert(typeof memoryStats.recent_added === 'number', '记忆统计缺少 recent_added 字段');
  assert(memoryStats.recent_added >= 0, '近7天新增不能为负数');

  // v0.9.6：执行活动契约——只要发生过 enrich，仪表盘上排指标必须有真实数据
  const actResponse = await fetch(`${API_BASE}/v1/feedback/association-activity`);
  assert(actResponse.ok, `执行活动接口失败: ${actResponse.status}`);
  const actData = await actResponse.json();
  assert(typeof actData.total_executions === 'number', '执行活动缺少 total_executions 字段');
  assert(actData.total_executions > 0, '执行活动应为非零（dev 库已有 enrich 审计事件）');
  assert(Array.isArray(actData.recent), '执行活动缺少 recent 明细数组');
  // v0.9.6 P1-1：结晶时间线需带来源记忆数/置信度，支撑成长链路展示
  const timelineResponse = await fetch(`${API_BASE}/v1/memories/synthesis-timeline?limit=3`);
  assert(timelineResponse.ok, `结晶时间线接口失败: ${timelineResponse.status}`);
  const timelineData = await timelineResponse.json();
  const firstItem = (timelineData.items || [])[0];
  assert(firstItem && typeof firstItem.source_count === 'number',
    '结晶时间线条目缺少 source_count 字段');

  const page = await getPage();
  const client = new CdpClient(page.webSocketDebuggerUrl);
  // 关键（v0.9.7 调试踩坑固化）：CDP 附加的是"已加载"页面，其中可能仍运行导航时刻的
  // 旧 app.js/旧打包 HTML。本用例验证磁盘 static 的最新代码，因此无论页面当前停在
  // 打包副本（tauri.localhost）还是 1420，都必须强制重新导航到 dev-proxy(1420)。
  // dev-proxy 不可用（无任何 HTTP 响应）时才回退原地状态。
  const proxyAlive = await fetch('http://localhost:1420/')
    .then(() => true).catch(() => false);
  if (proxyAlive) {
    await client.eval(`(function(){ location.href = 'http://localhost:1420/?r=' + Date.now(); return 1; })()`).catch(() => {});
    await new Promise(r => setTimeout(r, 3000));
  }
  // 短小纯 DOM 求值（单条 < 3s，规避 CdpClient 15s eval 超时）
  const domSettled = () => client.eval(`(() => {
    const cm = document.querySelectorAll('#home-association-list .cm-item').length;
    const bars = document.querySelectorAll('#memory-type-bars-list .bar-row').length;
    const hero = document.getElementById('value-hero-content')?.textContent || '';
    const heroTotal = Number((hero.match(/存下\\s*([\\d,]+)\\s*条记忆/) || [])[1]?.replace(/,/g, '') || 0);
    const busy = hero.includes('后台整理中') || hero.includes('加载');
    return JSON.stringify({ cm, bars, heroTotal, busy });
  })()`);
  const crystalCount = () => client.eval(`document.querySelectorAll('#crystal-highlights-list .ch-card').length`);
  const execText = () => client.eval(`(document.getElementById('association-execution-count')||{}).textContent || ''`);
  try {
    // v0.9.7：仪表盘重构为 M0-M5。就绪判定「触发一次 + 静默轮询」：
    // 高频重触发会 abort 上一轮请求并把 M1 重置回骨架，页面永远得不到
    // 一个安静的完整请求轮（abort 风暴）。触发后静默轮询 10s，未落定再补触发。
    // 开发库在后台合成，stats/recent/synth 的 lock_busy 窗口会错开漂移，
    // 故不就单一 API 快照做逐位相等——只要求页面渲染出真实数字。
    const triggerHome = () => client.eval(`(function(){ try { window.loadHomeData(); } catch (e) {} return 1; })()`).catch(() => {});
    const deadline = Date.now() + 60000;
    let settled = null;
    while (Date.now() < deadline && !settled) {
      await triggerHome();
      for (let quiet = 0; quiet < 5 && !settled; quiet++) {
        await new Promise(r => setTimeout(r, 2000));
        const d = JSON.parse(await domSettled());
        if (d.cm > 0 && d.bars > 0 && d.heroTotal > 0 && !d.busy) settled = d;
      }
    }
    assert(settled, '页面首页数据未在 ~60s 内落定（M1 总览 + M2 当前记忆 + M4a 构成）');
    // 就绪时刻抓一份实时快照，用于后续总数/近7天的漂移容差比对
    const liveResp = await fetch(`${API_BASE}/v1/memories/stats`);
    const liveBody = await liveResp.json();
    const liveStats = { total: liveBody.total_memories, recent: liveBody.recent_added };
    // v0.9.7：技术细节区默认折叠，联想仪表盘等断言前先展开 M5
    await client.eval(`(() => { const d = document.getElementById('home-advanced-details'); if (d) d.open = true; return !!d; })()`);
    // M5 内联想执行指标由 loadAssociationStats 驱动（非 loadHomeData），触发一次
    const triggerAll = () => client.eval(`(function(){
      try { window.loadHomeData(); } catch(e){}
      try { window.loadAssociationStats(); } catch(e){}
      return 1;
    })()`).catch(() => {});
    const allSettled = () => client.eval(`(() => {
      const cm = document.querySelectorAll('#home-association-list .cm-item').length;
      const bars = document.querySelectorAll('#memory-type-bars-list .bar-row').length;
      const crystal = document.querySelectorAll('#crystal-highlights-list .ch-card').length;
      const hero = document.getElementById('value-hero-content')?.textContent || '';
      const heroTotal = Number((hero.match(/存下\\s*([\\d,]+)\\s*条记忆/) || [])[1]?.replace(/,/g, '') || 0);
      const busy = hero.includes('后台整理中') || hero.includes('加载');
      const exec = document.getElementById('association-execution-count')?.textContent || '';
      // v0.9.7：联想执行指标在降级刷新时会清空命中原因列表（保留旧 exec 数字），
      // 就绪判定必须同时要求列表已渲染，否则快照期竞态导致偶发假失败
      const reasons = document.querySelectorAll('#association-recent-list .association-recent-item').length;
      return JSON.stringify({ cm, bars, crystal, heroTotal, busy, reasons, execOk: exec && exec !== '--' });
    })()`);
    // 关键：整页加载每次都会 abort 上一轮并把 M1 重置回骨架，绝不能高频重触发
    // （否则永远得不到一个安静的完整请求轮）。策略：触发一次 → 静默轮询 18s 让
    // 页面自身 2s×8 自愈收敛 → 未全绿再补触发。M4d 结晶/M5 执行指标各自 busy
    // 窗口独立，用同一"安静窗口"覆盖，最后一次性快照，保证跨模块数字自洽。
    await triggerAll();
    let fully = null;
    const hardDeadline = Date.now() + 60000;
    while (Date.now() < hardDeadline && !fully) {
      for (let quiet = 0; quiet < 18 && !fully; quiet++) {
        await new Promise(r => setTimeout(r, 1000));
        const s = JSON.parse(await allSettled());
        if (s.cm > 0 && s.bars > 0 && s.crystal > 0 && s.heroTotal > 0 && !s.busy && s.execOk && s.reasons > 0) fully = s;
      }
      if (!fully) await triggerAll();
    }
    assert(fully, '页面首页数据未在 ~60s 内全量落定（M1/M2/M4a/M4d/M5 执行）');
    const snapshot = await client.eval(`(() => ({
      apiBase: window.API_BASE,
      dashboard: !!document.getElementById('association-dashboard'),
      feedbackDomRemoved: !document.getElementById('association-feedback-total') && !document.getElementById('association-positive-count'),
      status: document.getElementById('association-dashboard-status')?.textContent || '',
      execCount: document.getElementById('association-execution-count')?.textContent || '',
      avgCandidates: document.getElementById('association-avg-candidates')?.textContent || '',
      dualPathRate: document.getElementById('association-dual-path-rate')?.textContent || '',
      // v0.9.7 M1：价值 Hero 真实总数
      heroText: document.getElementById('value-hero-content')?.textContent || '',
      // v0.9.7 M1 KPI1 副注：+N 近7天
      heroNote: document.querySelector('#value-hero-content .vh-kpi-note')?.textContent || '',
      // v0.9.7 M4a：记忆构成条形（六类）
      typeBars: document.querySelectorAll('#memory-type-bars-list .bar-row').length,
      // v0.9.7 M4d：结晶成果卡
      crystalCards: document.querySelectorAll('#crystal-highlights-list .ch-card').length,
      // v0.9.7 M5 整合：记忆分类六卡与 M4a 构成条形重复，已删除；此处反向断言其不再存在
      memoryOverviewRemoved: !document.getElementById('memory-category-grid')
        && !document.getElementById('memory-summary-main'),
      recentReasons: document.querySelectorAll('#association-recent-list .association-recent-item').length,
      // v0.9.7 M2：当前记忆三要素列表
      homeSearch: !!document.getElementById('home-search-input'),
      currentMemoryCard: !!document.getElementById('current-memory-card'),
      homeAssociation: !!document.getElementById('home-association-list'),
      homeQuickAdd: !!document.querySelector('[data-action="openQuickAdd"]'),
      cmItems: document.querySelectorAll('#home-association-list .cm-item').length,
      cmSummaryClamped: (() => {
        const el = document.querySelector('#home-association-list .cm-summary');
        return el ? getComputedStyle(el).webkitLineClamp : null;
      })(),
      cmHasText: (() => {
        const el = document.querySelector('#home-association-list .cm-summary');
        return el ? (el.textContent || '').trim().length > 0 : false;
      })(),
      // v0.9.7 M3：活动流
      activityFeed: !!document.getElementById('activity-feed'),
      afItems: document.querySelectorAll('#activity-feed .af-item').length,
      navHome: !!document.querySelector('.app-sidebar .nav-item[data-tab="dashboard"]'),
      navSearch: !!document.querySelector('.app-sidebar .nav-item[data-tab="memory-search"]'),
      navCaptainRemoved: !document.querySelector('.app-sidebar .nav-item[data-tab="captain-log"]'),
      quickLinks: document.querySelectorAll('.settings-quick-links .btn').length
    }))()`);
    assert(snapshot.apiBase === API_BASE, `页面 API 基址错误: ${snapshot.apiBase}`);
    assert(snapshot.dashboard, '技术细节内联想仪表盘 DOM 不存在');
    // v0.9.7：反馈埋点展示必须已从前端移除
    assert(snapshot.feedbackDomRemoved, '反馈埋点 DOM 未被移除（本地工具不应展示用户反馈统计）');
    assert(snapshot.status && snapshot.status !== '加载中...', `联想仪表盘未完成加载: ${snapshot.status}`);
    // v0.9.7：执行次数是单调增长的审计计数，只断言"页面不少于测试启动时快照"，
    // 且必须已渲染为真实数字（开发库在持续产生 enrich，逐位相等是伪契约）。
    const pageExec = Number(snapshot.execCount.replace(/,/g, ''));
    assert(Number.isFinite(pageExec) && pageExec >= 0 && snapshot.execCount !== '--',
      `执行次数未渲染真实数字: 页面=${snapshot.execCount}`);
    assert(pageExec >= actData.total_executions,
      `执行次数回退: 页面=${pageExec}, 启动快照=${actData.total_executions}`);
    assert(snapshot.avgCandidates !== '--' && snapshot.dualPathRate !== '--',
      `执行活动指标未渲染: avg=${snapshot.avgCandidates}, dual=${snapshot.dualPathRate}`);
    // v0.9.7 M1：价值 Hero 数字与就绪时刻实时快照比对（开发库持续写入，容差 ±500）
    const heroTotal = Number((snapshot.heroText.match(/存下\s*([\d,]+)\s*条记忆/) || [])[1]?.replace(/,/g, '') || NaN);
    assert(Number.isFinite(heroTotal) && Math.abs(heroTotal - liveStats.total) <= 500,
      `价值总览总数漂移过大: 页面=${heroTotal}, 就绪快照=${liveStats.total}`);
    // DOM 内部一致性：陈述句总数必须等于 KPI1 数字（同一渲染轮，应严格相等）
    const kpi1 = await client.eval(`(function(){var e=document.querySelector('#value-hero-content .vh-kpi-value');return Number((e?e.textContent:'').replace(/,/g,'')||NaN);})()`);
    assert(kpi1 === heroTotal, `陈述句与 KPI 不一致: 陈述句=${heroTotal}, KPI1=${kpi1}`);
    const noteMatch = snapshot.heroNote.match(/\+([\d,]+)\s*近7天/);
    const pageRecentAdded = noteMatch ? Number(noteMatch[1].replace(/,/g, '')) : NaN;
    assert(Number.isFinite(pageRecentAdded) && Math.abs(pageRecentAdded - liveStats.recent) <= 500,
      `价值总览近7天新增漂移过大: 页面=${snapshot.heroNote}, 就绪快照=${liveStats.recent}`);
    // v0.9.7 M4a：类型条形图有数据
    assert(snapshot.typeBars > 0, '记忆构成条形图未渲染');
    // v0.9.7 M5 整合：与主仪表盘重复的「记忆总览」区必须已删除
    assert(snapshot.memoryOverviewRemoved, 'M5 记忆总览区仍存在（与 M1/M4a 重复，应已删除）');
    // v0.9.6 P1-2：最近联想命中原因列表应渲染真实执行明细
    assert(snapshot.recentReasons > 0, '联想命中原因列表未渲染');
    // v0.9.7：首页用户视角模块（M1 已验证，这里验证 M2/M3 与导航）
    assert(snapshot.homeSearch, '首页搜索框缺失');
    assert(snapshot.currentMemoryCard, 'M2「当前记忆」卡缺失');
    assert(snapshot.homeAssociation, 'M2 记忆列表容器缺失');
    assert(snapshot.homeQuickAdd, '首页「记一笔」按钮缺失');
    assert(snapshot.cmItems > 0, 'M2 未渲染任何当前记忆');
    assert(snapshot.cmHasText, 'M2 记忆摘要为空（content_preview 修复失效）');
    assert(snapshot.cmSummaryClamped === '2', `M2 摘要未按规范两行裁剪: ${snapshot.cmSummaryClamped}`);
    assert(snapshot.activityFeed, 'M3 活动流容器缺失');
    assert(snapshot.afItems > 0, 'M3 活动流未渲染任何事件');
    // v0.9.6 G2：导航精简
    assert(snapshot.navHome && snapshot.navSearch, '主导航缺失首页/全部记忆入口');
    assert(snapshot.navCaptainRemoved, '船长日志不应再出现在主导航');
    assert(snapshot.quickLinks >= 4, '设置页快捷入口数量不足（应有 4+ 个）');

    const refreshed = await client.eval(`(async () => {
      await window.loadAssociationStats();
      return {
        status: document.getElementById('association-dashboard-status')?.textContent || '',
        execCount: document.getElementById('association-execution-count')?.textContent || ''
      };
    })()`);
    assert(refreshed.status && refreshed.status !== '加载中...', `手动刷新未完成: ${refreshed.status}`);
    assert(Number(refreshed.execCount) >= actData.total_executions, '手动刷新后执行次数回退');

    // v0.9.6 修复：结晶历史接口契约（API 层仍须返回真实结晶数据）
    const timelineResponse = await fetch(`${API_BASE}/v1/memories/synthesis-timeline?limit=10`);
    assert(timelineResponse.ok, `结晶时间线接口失败: ${timelineResponse.status}`);
    const timelineData = await timelineResponse.json();
    assert(Array.isArray(timelineData.items), '结晶时间线缺少 items 字段');
    assert(typeof timelineData.total === 'number', '结晶时间线缺少 total 字段');
    assert(timelineData.total > 0, '结晶时间线应为非零（dev 库已有 synthesis 记忆）');

    // v0.9.7：结晶成果由 M4d「结晶成果」子卡渲染（旧 #crystallization-timeline 卡已删除），
    // 由 loadHomeData 数据流驱动，断言页面卡片数与 API 一致（最多 3 张）。
    assert(snapshot.crystalCards > 0, 'M4d 结晶成果卡未渲染');
    assert(snapshot.crystalCards <= 3, `M4d 结晶成果卡超上限: ${snapshot.crystalCards}`);
    assert(timelineData.total >= snapshot.crystalCards,
      `结晶成果卡数超出接口总数: total=${timelineData.total}, cards=${snapshot.crystalCards}`);

    console.log(JSON.stringify({
      ok: true,
      apiBase: snapshot.apiBase,
      status: refreshed.status,
      totalMemories: memoryStats.total_memories,
      recentAdded: memoryStats.recent_added,
      executions: Number(refreshed.execCount),
      crystallizations: timelineData.total
    }, null, 2));
  } finally {
    client.close();
  }
})().catch(error => {
  console.error(`[association-dashboard-cdp] FAIL: ${error.message}`);
  process.exitCode = 1;
});
