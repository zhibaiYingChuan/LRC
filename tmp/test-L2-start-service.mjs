// L2 二级弹窗审计：启动服务全流程 + 异常路径
import { cdpBatchWithDelay, cdpBatch } from './cdp-audit-client.mjs';

// 阶段1：触发 handleStartServiceClick，90s 内每 3s 采样
const startTrigger = {
  name: '触发 handleStartServiceClick',
  await: false,
  expr: `(async () => {
    try {
      if (typeof handleStartServiceClick !== 'function') return 'handleStartServiceClick NOT FOUND';
      if (typeof _startServiceInProgress !== 'undefined' && _startServiceInProgress) return 'already in progress';
      handleStartServiceClick(); // 不 await，后台运行
      return 'triggered';
    } catch (e) {
      return 'error: ' + e.message;
    }
  })()`
};

// 采样函数：每 3s 采样一次 UI 状态
function sampleState(idx) {
  return {
    name: `采样 #${idx} (${idx * 3}s)`,
    await: false,
    expr: `JSON.stringify({
      ts: ${idx * 3},
      banner: (() => { const b = document.getElementById('sidecar-down-banner'); return b ? { hidden: b.hidden, btnText: b.querySelector('button')?.textContent, btnDisabled: b.querySelector('button')?.disabled } : 'NOT_FOUND'; })(),
      modal: (() => { const m = document.getElementById('start-service-modal') || document.querySelector('.modal'); return m ? { display: getComputedStyle(m).display, visible: m.offsetParent !== null, btnText: m.querySelector('#modal-btn-start-service')?.textContent, btnDisabled: m.querySelector('#modal-btn-start-service')?.disabled } : 'NO_MODAL'; })(),
      shm: typeof SidecarHealthMonitor !== 'undefined' ? {
        reachable: SidecarHealthMonitor._isReachable,
        status: SidecarHealthMonitor._sidecarStatus,
        lockBusy: SidecarHealthMonitor._lockBusy,
        failCount: SidecarHealthMonitor._failCount,
        inProgress: typeof _startServiceInProgress !== 'undefined' ? _startServiceInProgress : 'unknown'
      } : 'NO_SHM',
      statusText: (() => { const t = document.getElementById('status-text'); return t ? t.innerText : 'NOT_FOUND'; })(),
      statusDot: (() => { const d = document.getElementById('status-dot'); return d ? d.className : 'NOT_FOUND'; })(),
      dashError: (() => { const e = document.getElementById('dashboard-error'); return e ? { show: e.classList.contains('show'), text: e.innerText.substring(0, 120) } : 'NOT_FOUND'; })(),
      toastVisible: (() => { const t = document.querySelector('.toast, #toast-container'); return t ? t.innerText.substring(0, 100) : 'NO_TOAST'; })()
    })`
  };
}

// 构建 90s 采样序列（30 次，每 3s）
const samples = [];
for (let i = 1; i <= 30; i++) {
  samples.push(sampleState(i));
}

console.log('=== L2 启动服务审计：触发启动 + 90s 采样 ===\n');
console.log('阶段1：触发 handleStartServiceClick...\n');

try {
  // 先触发启动
  const triggerResult = await cdpBatch([startTrigger], 15000);
  console.log('触发结果:', JSON.stringify(triggerResult.results[0]?.result?.result?.value || triggerResult.results[0], null, 2));
  console.log('\n触发后 console 日志:');
  triggerResult.consoleLogs.forEach(l => console.log(`  [${l.type}] ${l.text.substring(0, 200)}`));

  console.log('\n阶段2：90s 状态采样（每 3s 一次）...\n');
  // 90s 采样，总超时 120s
  const sampleResult = await cdpBatchWithDelay(samples, 3000, 120000);

  console.log('=== 采样结果 ===\n');
  sampleResult.results.forEach(r => {
    try {
      const val = r.result?.result?.value;
      if (val) {
        const s = JSON.parse(val);
        console.log(`[${s.ts}s] reachable=${s.shm?.reachable} status=${s.shm?.status} lockBusy=${s.shm?.lockBusy} failCnt=${s.shm?.failCount} inProg=${s.shm?.inProgress} | statusText="${s.statusText}" dot=${s.statusDot} | banner.hidden=${s.banner?.hidden} bannerBtn="${s.banner?.btnText}" | modal=${typeof s.modal==='string'?s.modal:'vis'} modalBtn="${typeof s.modal==='object'?s.modal?.btnText:''}" | dashErr="${s.dashErr?.text||''}" | toast="${s.toastVisible?.substring(0,60)||''}"`);
      } else if (r.error) {
        console.log(`  ERROR: ${r.error}`);
      }
    } catch (e) {
      console.log(`  解析失败: ${e.message}`);
    }
  });

  console.log('\n=== 启动过程 console 日志（关键）===');
  // 过滤关键日志
  const keyLogs = sampleResult.consoleLogs.filter(l =>
    l.text.includes('sidecar') || l.text.includes('Sidecar') || l.text.includes('启动') ||
    l.text.includes('start') || l.text.includes('Start') || l.text.includes('LRC') ||
    l.text.includes('health') || l.text.includes('Health') || l.text.includes('错误') ||
    l.text.includes('失败') || l.text.includes('success') || l.text.includes('indexing') ||
    l.text.includes('progress') || l.text.includes('E008') || l.text.includes('cancel')
  );
  keyLogs.forEach(l => console.log(`  [${l.type}] ${l.text.substring(0, 250)}`));

  console.log('\n=== 异常事件 ===');
  if (sampleResult.exceptions && sampleResult.exceptions.length) {
    sampleResult.exceptions.forEach(e => console.log(`  EX: ${e.text.substring(0, 300)}`));
  } else {
    console.log('  （无异常）');
  }

  // 阶段3：最终状态快照（启动后）
  console.log('\n阶段3：启动后最终状态快照...\n');
  const finalState = await cdpBatch([{
    name: '最终状态',
    expr: `JSON.stringify({
      shm: typeof SidecarHealthMonitor !== 'undefined' ? {
        reachable: SidecarHealthMonitor._isReachable,
        status: SidecarHealthMonitor._sidecarStatus,
        lockBusy: SidecarHealthMonitor._lockBusy,
        failCount: SidecarHealthMonitor._failCount
      } : null,
      statusText: document.getElementById('status-text')?.innerText,
      statusDot: document.getElementById('status-dot')?.className,
      bannerHidden: document.getElementById('sidecar-down-banner')?.hidden,
      dashError: (() => { const e = document.getElementById('dashboard-error'); return e ? { show: e.classList.contains('show'), text: e.innerText.substring(0,200) } : null; })(),
      dashLoading: (() => { const l = document.getElementById('dashboard-loading'); return l ? { hidden: l.classList.contains('hidden') } : null; })()
    })`
  }], 15000);
  console.log('最终状态:', finalState.results[0]?.result?.result?.value);

} catch (e) {
  console.error('测试执行失败:', e.message);
  console.error(e.stack);
}
