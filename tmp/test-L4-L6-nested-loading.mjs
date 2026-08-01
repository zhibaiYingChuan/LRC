// L4 四级嵌套审计 + L6 组件级数据加载韧性测试
import { cdpBatch, cdpBatchWithDelay } from './cdp-audit-client.mjs';

// 阶段1：当前状态 + 道同构度"重试"按钮定位
const phase1 = [
  {
    name: 'L4 道同构度重试按钮定位 + 当前状态',
    expr: `JSON.stringify({
      daoPanel: (() => {
        const panels = document.querySelectorAll('.dao-metrics-panel, [class*="dao"]');
        for (const p of panels) {
          if (p.innerText && p.innerText.includes('道同构度')) {
            const retryBtn = Array.from(p.querySelectorAll('button, a')).find(b => b.innerText.includes('重试'));
            return {
              found: true,
              id: p.id,
              className: p.className.substring(0, 80),
              text: p.innerText.substring(0, 200),
              retryBtn: retryBtn ? { text: retryBtn.text, disabled: retryBtn.disabled, onclick: retryBtn.onclick?.toString()?.substring(0, 100) } : 'NO_RETRY_BTN'
            };
          }
        }
        return 'NOT_FOUND';
      })(),
      shm: typeof SidecarHealthMonitor !== 'undefined' ? {
        reachable: SidecarHealthMonitor._isReachable,
        status: SidecarHealthMonitor._sidecarStatus,
        lockBusy: SidecarHealthMonitor._lockBusy,
        failCount: SidecarHealthMonitor._failCount
      } : null,
      dashError: (() => { const e = document.getElementById('dashboard-error'); return e ? { show: e.classList.contains('show'), text: e.innerText.substring(0, 200) } : null; })()
    })`
  }
];

// 阶段2：点击道同构度"重试"按钮，观察 15s
const retryClick = {
  name: 'L4 点击道同构度重试按钮',
  await: false,
  expr: `(async () => {
    try {
      const panels = document.querySelectorAll('.dao-metrics-panel, [class*="dao"]');
      for (const p of panels) {
        if (p.innerText && p.innerText.includes('道同构度')) {
          const retryBtn = Array.from(p.querySelectorAll('button, a')).find(b => b.innerText.includes('重试'));
          if (retryBtn) {
            retryBtn.click();
            return JSON.stringify({ clicked: true, btnText: retryBtn.innerText });
          }
          // 没有重试按钮，尝试调用 loadDaoMetrics
          if (typeof loadDaoMetrics === 'function') {
            loadDaoMetrics();
            return JSON.stringify({ clicked: false, fallback: 'loadDaoMetrics called' });
          }
          return 'NO_RETRY_BTN_NO_FALLBACK';
        }
      }
      return 'DAO_PANEL_NOT_FOUND';
    } catch (e) {
      return 'error: ' + e.message;
    }
  })()`
};

// 阶段3：点击后 15s 内每 3s 采样
const postClickSamples = [];
for (let i = 1; i <= 5; i++) {
  postClickSamples.push({
    name: `L4 重试后采样 #${i} (${i*3}s)`,
    await: false,
    expr: `JSON.stringify({
      ts: ${i*3},
      daoPanel: (() => {
        const panels = document.querySelectorAll('.dao-metrics-panel, [class*="dao"]');
        for (const p of panels) {
          if (p.innerText && p.innerText.includes('道同构度')) {
            return { text: p.innerText.substring(0, 200), hasRetry: !!Array.from(p.querySelectorAll('button')).find(b => b.innerText.includes('重试')) };
          }
        }
        return 'NOT_FOUND';
      })(),
      shm: typeof SidecarHealthMonitor !== 'undefined' ? { reachable: SidecarHealthMonitor._isReachable, lockBusy: SidecarHealthMonitor._lockBusy } : null,
      toast: (() => { const t = document.querySelector('.toast, #toast-container'); return t ? t.innerText.substring(0, 100) : 'NO_TOAST'; })()
    })`
  });
}

// 阶段4：L6 组件级数据加载韧性 — 手动触发 loadDashboard + loadDaoMetrics
const phase4 = [
  {
    name: 'L6 手动触发 loadDashboard',
    await: true,
    expr: `(async () => {
      try {
        if (typeof loadDashboard !== 'function') return 'loadDashboard NOT FOUND';
        loadDashboard();
        await new Promise(r => setTimeout(r, 3000));
        const err = document.getElementById('dashboard-error');
        const loading = document.getElementById('dashboard-loading');
        return JSON.stringify({
          triggered: true,
          dashError: err ? { show: err.classList.contains('show'), text: err.innerText.substring(0, 200) } : null,
          dashLoading: loading ? { hidden: loading.classList.contains('hidden') } : null
        });
      } catch (e) {
        return 'error: ' + e.message;
      }
    })()`
  },
  {
    name: 'L6 手动触发 loadDaoMetrics',
    await: true,
    expr: `(async () => {
      try {
        if (typeof loadDaoMetrics !== 'function') return 'loadDaoMetrics NOT FOUND';
        loadDaoMetrics();
        await new Promise(r => setTimeout(r, 3000));
        const panels = document.querySelectorAll('.dao-metrics-panel, [class*="dao"]');
        for (const p of panels) {
          if (p.innerText && p.innerText.includes('道同构度')) {
            return JSON.stringify({ triggered: true, daoText: p.innerText.substring(0, 250) });
          }
        }
        return JSON.stringify({ triggered: true, daoPanel: 'NOT_FOUND_AFTER_LOAD' });
      } catch (e) {
        return 'error: ' + e.message;
      }
    })()`
  }
];

console.log('=== L4 嵌套操作 + L6 组件数据加载审计 ===\n');

try {
  // 阶段1
  console.log('阶段1：道同构度重试按钮定位...\n');
  const r1 = await cdpBatch(phase1, 20000);
  for (const r of r1.results) {
    console.log('--- ' + r.name + ' ---');
    try { console.log(JSON.stringify(JSON.parse(r.result?.result?.value), null, 2)); } catch(e) { console.log(r.result?.result?.value || r.error); }
  }

  // 阶段2+3：点击重试 + 采样
  console.log('\n阶段2：点击道同构度"重试"按钮 + 15s 采样...\n');
  const r2 = await cdpBatchWithDelay([retryClick, ...postClickSamples], 3000, 60000);
  console.log('点击结果:', r2.results[0]?.result?.result?.value || r2.results[0]?.error);
  r2.results.slice(1).forEach(r => {
    try {
      const s = JSON.parse(r.result?.result?.value || '{}');
      console.log(`[${s.ts}s] daoText="${(s.daoPanel?.text||'').substring(0,80)}" hasRetry=${s.daoPanel?.hasRetry} shm.reachable=${s.shm?.reachable} lockBusy=${s.shm?.lockBusy} toast="${(s.toast||'').substring(0,50)}"`);
    } catch(e) { console.log('  解析失败:', r.error || r.result?.result?.value); }
  });

  // 阶段4：L6 手动触发
  console.log('\n阶段4：L6 手动触发 loadDashboard + loadDaoMetrics...\n');
  const r4 = await cdpBatch(phase4, 30000);
  for (const r of r4.results) {
    console.log('--- ' + r.name + ' ---');
    try { console.log(JSON.stringify(JSON.parse(r.result?.result?.value), null, 2)); } catch(e) { console.log(r.result?.result?.value || r.error); }
  }

  // 关键 console 日志
  console.log('\n=== 关键 console 日志（最后 20 条）===');
  [...r1.consoleLogs, ...r2.consoleLogs, ...r4.consoleLogs].slice(-20).forEach(l => console.log(`  [${l.type}] ${l.text.substring(0, 200)}`));

} catch (e) {
  console.error('测试失败:', e.message);
}
