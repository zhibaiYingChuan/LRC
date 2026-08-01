// HCSE Round 5 前端组合测试：consolidate + loadDashboard 同时触发
// 验证 P1-NEW-01（hasLockBusy200 识别 200+lock_busy 降级）+ P1-NEW-02（renderDashboard 防御性检查）
(async () => {
  const result = {};
  const SIDECAR = 'http://127.0.0.1:3099';

  // 记录 loadDashboard 前的 dashboard 状态
  const errorEl = document.getElementById('dashboard-error');
  const statTotal = document.getElementById('stat-total');
  const statActive = document.getElementById('stat-active');
  result.before = {
    statTotal: statTotal ? statTotal.textContent : null,
    statActive: statActive ? statActive.textContent : null,
    errorShow: errorEl ? errorEl.classList.contains('show') : null,
    errorText: errorEl ? (errorEl.textContent || '').trim().substring(0, 120) : null
  };

  // 构造 50 个记忆，延长 luoshu_synthesize 耗时（扩大 lock_busy 窗口）
  const mems = [];
  for (let i = 0; i < 50; i++) {
    mems.push({
      content: 'Round5 前端组合测试记忆 ' + i + ' - 验证 P1-NEW-01 hasLockBusy200 + P1-NEW-02 renderDashboard 防御性检查',
      memory_type: 'fact', project: 'hcse-round5-frontend', tags: ['hcse', 'round5', 'frontend'],
      importance: 5, privacy_level: 'public', session_id: 'r5f', user_id: 'r5f'
    });
  }

  // 1. 同时触发 consolidate（POST，触发 lock_busy）
  const consolidatePromise = fetch(SIDECAR + '/v1/memories/consolidate', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ memories: mems })
  });

  // 2. 等待 15ms 让 consolidate 先拿锁进入 lock_busy
  await new Promise(r => setTimeout(r, 15));

  // 3. 记录 loadDashboard 调用时的 monitor 状态
  const shm = window.sidecarHealthMonitor;
  result.atLoadDashboard = {
    monitorLockBusy: shm ? shm._lockBusy : null,
    monitorFailCount: shm ? shm._failCount : null
  };

  // 4. 调用 loadDashboard（会在 lock_busy 期间发 GET /v1/health/* 请求）
  //    期望：收到 200+lock_busy=true → hasLockBusy200=true → throw LOCK_BUSY → catch 显示"后台合成中"
  let loadDashboardError = null;
  let loadDashboardDuration = 0;
  const ldStart = Date.now();
  try {
    await window.loadDashboard();
  } catch (e) {
    loadDashboardError = e.message;
  }
  loadDashboardDuration = Date.now() - ldStart;

  // 5. 等待 consolidate 完成
  let consResult = null;
  try {
    const consResp = await consolidatePromise;
    const consBody = await consResp.json();
    consResult = { status: consResp.status, stored: consBody.stored, synthesized: consBody.synthesized, total: consBody.total_memories };
  } catch (e) {
    consResult = { error: e.message };
  }

  // 6. 采集 loadDashboard 后的 dashboard 状态
  result.after = {
    statTotal: statTotal ? statTotal.textContent : null,
    statActive: statActive ? statActive.textContent : null,
    errorShow: errorEl ? errorEl.classList.contains('show') : null,
    errorText: errorEl ? (errorEl.textContent || '').trim().substring(0, 200) : null,
    loadDashboardError: loadDashboardError,
    loadDashboardDurationMs: loadDashboardDuration
  };
  result.consolidate = consResult;

  // 7. monitor 最终状态
  result.afterMonitor = {
    lockBusy: shm ? shm._lockBusy : null,
    failCount: shm ? shm._failCount : null,
    sidecarStatus: shm ? shm.sidecarStatus : null
  };

  // 8. P1-NEW-01 验证：loadDashboard 应识别 200+lock_busy
  //    证据：errorShow=true 且 errorText 包含"后台合成"（LOCK_BUSY catch 分支）
  //    或 statTotal 保持非 0（未被降级数据覆盖）
  const errorShowsLockBusy = result.after.errorShow === true &&
    (result.after.errorText.includes('后台合成') || result.after.errorText.includes('LOCK_BUSY') || result.after.errorText.includes('合成'));
  const statTotalPreserved = result.after.statTotal !== '0' && result.after.statTotal !== null;

  // 9. P1-NEW-02 验证：renderDashboard 不渲染 0 记忆
  //    证据：statTotal 不是 '0'（防御性检查跳过渲染，保留原值）
  const noZeroRender = result.after.statTotal !== '0';

  result.verdict = {
    p1_new_01_lockbusy_recognized: errorShowsLockBusy,
    p1_new_02_no_zero_render: noZeroRender,
    statTotalPreserved: statTotalPreserved,
    loadDashboardDurationMs: loadDashboardDuration
  };

  return result;
})()
