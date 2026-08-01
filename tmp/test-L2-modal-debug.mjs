// L2 模态框卡住根因诊断 + L3/L4/L6 卡片韧性测试
import { cdpBatch } from './cdp-audit-client.mjs';

const expressions = [
  {
    name: '模态框元素详细诊断',
    expr: `JSON.stringify({
      modalById: (() => { const m = document.getElementById('start-service-modal'); return m ? {
        exists: true,
        display: m.style.display,
        hiddenAttr: m.hasAttribute('hidden'),
        classList: Array.from(m.classList),
        offsetParent: m.offsetParent !== null,
        computedDisplay: getComputedStyle(m).display,
        computedVisibility: getComputedStyle(m).visibility,
        computedOpacity: getComputedStyle(m).opacity,
        zIndex: getComputedStyle(m).zIndex,
        innerHTML: m.innerHTML.substring(0, 400),
        btnText: m.querySelector('#modal-btn-start-service')?.textContent,
        btnDisabled: m.querySelector('#modal-btn-start-service')?.disabled
      } : { exists: false }; })(),
      modalByClass: (() => { const ms = document.querySelectorAll('.modal'); return Array.from(ms).map(m => ({ id: m.id, display: getComputedStyle(m).display, visible: m.offsetParent !== null })); })(),
      startServiceInProgress: typeof _startServiceInProgress !== 'undefined' ? _startServiceInProgress : 'NOT_GLOBAL',
      abortController: typeof startServiceAbortController !== 'undefined' ? startServiceAbortController : 'NOT_GLOBAL',
      bannerBtn: (() => { const b = document.querySelector('#sidecar-down-banner .banner-btn'); return b ? { text: b.textContent, disabled: b.disabled } : 'NOT_FOUND'; })()
    })`
  },
  {
    name: '尝试手动调用 closeStartServiceModal',
    await: false,
    expr: `(async () => {
      try {
        if (typeof closeStartServiceModal !== 'function') return 'closeStartServiceModal NOT FOUND';
        closeStartServiceModal();
        await new Promise(r => setTimeout(r, 500));
        const m = document.getElementById('start-service-modal');
        return JSON.stringify({ called: true, modalExists: !!m, modalDisplay: m ? m.style.display : null, modalVisible: m ? m.offsetParent !== null : null });
      } catch (e) {
        return 'error: ' + e.message;
      }
    })()`
  },
  {
    name: 'L3 道同构度卡片内容',
    expr: `JSON.stringify({
      daoCard: (() => {
        const candidates = ['#dao-metrics-card', '#card-dao', '[data-card="dao"]', '#dao-metrics', '#dao-section'];
        for (const sel of candidates) {
          const el = document.querySelector(sel);
          if (el) return { selector: sel, text: el.innerText.substring(0, 300), html: el.innerHTML.substring(0, 400) };
        }
        // 搜索包含"道同构度"的元素
        const all = document.querySelectorAll('div, section, article, card');
        for (const el of all) {
          if (el.innerText && el.innerText.includes('道同构度') && el.innerText.length < 500) {
            return { selector: 'text-match', id: el.id, className: el.className, text: el.innerText.substring(0, 300) };
          }
        }
        return 'NOT_FOUND';
      })()
    })`
  },
  {
    name: 'L3 记忆统计卡片内容',
    expr: `JSON.stringify({
      statsCard: (() => {
        const candidates = ['#memories-stats', '#card-memories', '[data-card="memories"]', '#memory-stats', '#stats-card'];
        for (const sel of candidates) {
          const el = document.querySelector(sel);
          if (el) return { selector: sel, text: el.innerText.substring(0, 300) };
        }
        const all = document.querySelectorAll('div, section, article, card');
        for (const el of all) {
          if (el.innerText && (el.innerText.includes('记忆统计') || el.innerText.includes('记忆总数')) && el.innerText.length < 500) {
            return { selector: 'text-match', id: el.id, className: el.className, text: el.innerText.substring(0, 300) };
          }
        }
        return 'NOT_FOUND';
      })()
    })`
  },
  {
    name: 'L3/L6 仪表盘所有卡片概览',
    expr: `JSON.stringify({
      cards: (() => {
        const cards = document.querySelectorAll('.card, .stats-card, .dashboard-card, [class*="card"]');
        return Array.from(cards).slice(0, 15).map(c => ({
          id: c.id,
          className: c.className.substring(0, 80),
          text: c.innerText.substring(0, 120).replace(/\\n/g, ' | '),
          visible: c.offsetParent !== null
        }));
      })(),
      dashboardHTML: document.getElementById('dashboard')?.innerHTML?.substring(0, 800) || 'NO_DASHBOARD'
    })`
  },
  {
    name: 'L6 道同构度加载函数检查',
    expr: `JSON.stringify({
      loadDaoMetricsExists: typeof loadDaoMetrics === 'function',
      daoRetryCount: typeof _daoRetryCount !== 'undefined' ? _daoRetryCount : 'NOT_GLOBAL',
      daoMaxRetries: typeof _DAO_MAX_RETRIES !== 'undefined' ? _DAO_MAX_RETRIES : 'NOT_GLOBAL',
      daoRetryTimerActive: typeof _daoRetryTimer !== 'undefined' ? (_daoRetryTimer !== null) : 'NOT_GLOBAL',
      currentDaoMetrics: typeof window._currentDaoMetrics !== 'undefined' ? window._currentDaoMetrics : 'NOT_SET'
    })`
  }
];

console.log('=== L2 模态框诊断 + L3/L6 卡片审计 ===\n');
try {
  const result = await cdpBatch(expressions, 45000);
  console.log('=== Console 日志 ===');
  result.consoleLogs.slice(-15).forEach(l => console.log(`  [${l.type}] ${l.text.substring(0, 200)}`));
  console.log('\n=== 测试结果 ===');
  for (const r of result.results) {
    console.log('\n--- ' + r.name + ' ---');
    if (r.result?.result?.value) {
      try {
        const parsed = JSON.parse(r.result.result.value);
        console.log(JSON.stringify(parsed, null, 2));
      } catch (e) {
        console.log(r.result.result.value);
      }
    } else if (r.error) {
      console.log('ERROR:', r.error);
    } else {
      console.log(JSON.stringify(r, null, 2));
    }
  }
  if (result.exceptions?.length) {
    console.log('\n=== 异常事件 ===');
    result.exceptions.forEach(e => console.log('  EX:', e.text.substring(0, 300)));
  }
} catch (e) {
  console.error('测试失败:', e.message);
}
