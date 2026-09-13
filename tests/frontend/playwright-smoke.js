'use strict';

const fs = require('fs');
const path = require('path');
const { chromium } = require('playwright');

const baseUrl = process.env.LRC_BROWSER_BASE_URL || 'http://127.0.0.1:3111';
const artifactDir = process.env.LRC_BROWSER_ARTIFACT_DIR || path.join(process.cwd(), 'playwright-artifacts');
const failures = [];
const consoleErrors = [];
const networkFailures = [];

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function main() {
  fs.mkdirSync(artifactDir, { recursive: true });
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });

  page.on('console', message => {
    if (message.type() === 'error') consoleErrors.push(message.text());
  });
  page.on('pageerror', error => consoleErrors.push(`页面异常: ${error.message}`));
  page.on('requestfailed', request => {
    const failure = `${request.method()} ${request.url()} ${request.failure()?.errorText || ''}`;
    if (!request.url().startsWith('data:')) networkFailures.push(failure);
  });

  try {
    const response = await page.goto(`${baseUrl}/dashboard`, { waitUntil: 'domcontentloaded', timeout: 15000 });
    assert(response && response.ok(), `仪表盘响应异常: ${response?.status() || '无响应'}`);

    // v0.9.7：仪表盘重构为 M0-M5，首屏就绪锚点改为 M1 价值 Hero。
    await page.locator('#value-hero').waitFor({ state: 'visible', timeout: 10000 });
    await page.waitForFunction(() => {
      const el = document.getElementById('value-hero-content');
      const text = el?.textContent || '';
      return text.includes('条记忆') && !text.includes('加载') && !text.includes('后台整理中');
    }, null, { timeout: 20000, polling: 500 });

    // v0.9.7：联想仪表盘等已折叠进 M5「技术细节」，先展开再断言
    await page.locator('#home-advanced-details').evaluate(el => { el.open = true; });
    await page.waitForTimeout(800);
    await page.locator('#association-dashboard').waitFor({ state: 'visible', timeout: 10000 });
    await page.locator('#association-dashboard-status').waitFor({ state: 'visible', timeout: 10000 });

    const dashboard = await page.evaluate(() => ({
      heroSentence: document.getElementById('value-hero-sentence')?.textContent?.trim() || '',
      currentMemory: Boolean(document.getElementById('current-memory-card')),
      cmItems: document.querySelectorAll('#home-association-list .cm-item').length,
      activityFeed: document.querySelectorAll('#activity-feed .af-item').length,
      typeBars: document.querySelectorAll('#memory-type-bars-list .bar-row').length,
      status: document.querySelector('#association-dashboard-status')?.textContent?.trim() || '',
      feedbackDomRemoved: !document.getElementById('association-feedback-total'),
      emojiInBody: /[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}\u{2B00}-\u{2BFF}]/u.test(document.body.innerText),
    }));
    assert(dashboard.heroSentence.includes('条记忆'), 'M1 价值陈述句未渲染');
    assert(dashboard.currentMemory, 'M2「当前记忆」卡缺失');
    assert(dashboard.cmItems > 0, 'M2 未渲染当前记忆条目');
    assert(dashboard.status && dashboard.status !== '加载中...', `联想仪表盘未完成加载: ${dashboard.status}`);
    assert(dashboard.feedbackDomRemoved, '反馈埋点 DOM 仍存在（应已移除）');
    assert(!dashboard.emojiInBody, '页面仍含 emoji 图标（应已全部替换为 SVG）');

    await page.screenshot({ path: path.join(artifactDir, 'dashboard.png'), fullPage: true });
    fs.writeFileSync(path.join(artifactDir, 'dashboard.html'), await page.content(), 'utf8');
    fs.writeFileSync(path.join(artifactDir, 'console-errors.json'), JSON.stringify(consoleErrors, null, 2), 'utf8');
    fs.writeFileSync(path.join(artifactDir, 'network-failures.json'), JSON.stringify(networkFailures, null, 2), 'utf8');

    // P1-8 修复：成功路径此前只落盘 console-errors/network-failures 而不参与判定，
    // 导致"主断言通过但控制台报错 / 资源 404"仍以退出码 0 放行。
    // 现改为阻断；豁免口径与 cdp-regression.js 保持一致（仅豁免已核实的预期错误），
    // 且 unfold 豁免必须以"同批次存在 unfold 404 网络记录"为前置，避免正则过宽吞掉真实 404。
    const hasUnfold404 = networkFailures.some((item) => /404/.test(item) && /\/v1\/memories\/unfold/.test(item));
    const EXPECTED_ERROR_PATTERNS = [];
    if (hasUnfold404) EXPECTED_ERROR_PATTERNS.push(/404|unfold/i);
    EXPECTED_ERROR_PATTERNS.push(/(start_sidecar|command).{0,40}not allowed/i);

    const unexpectedConsoleErrors = consoleErrors.filter((text) => !EXPECTED_ERROR_PATTERNS.some((re) => re.test(text)));
    const unexpectedNetworkFailures = networkFailures.filter((text) => !EXPECTED_ERROR_PATTERNS.some((re) => re.test(text)));
    assert(unexpectedConsoleErrors.length === 0, `控制台存在 ${unexpectedConsoleErrors.length} 条非预期错误: ${unexpectedConsoleErrors.slice(0, 3).join(' | ')}`);
    assert(unexpectedNetworkFailures.length === 0, `存在 ${unexpectedNetworkFailures.length} 条非预期网络失败: ${unexpectedNetworkFailures.slice(0, 3).join(' | ')}`);

    // ===== 超时/卡死路径韧性验证（闭环 HCSE_RESILIENCE_AUDIT 检查项 1.3）=====
    // 目的：验证"底层调用永不返回"时 UI 是否有兜底反馈、是否可恢复——此前该路径无任何自动化验证。
    // 手段：用 page.route 挂起三个健康检查端点（handler 不调用任何 route 方法 → 请求永久 pending），
    //      触发 fetchWithTimeout 的 10s 硬超时，断言出现"请求超时"错误态 + 重试入口 + loading 已收起。
    // 位置：置于 console/network 阻断断言之后，避免本测试预期的 abort 事件污染正常路径判定。
    // 前提：sidecar 存活且未处于 lock_busy（若 lock_busy，loadDashboard 会走降级渲染而非超时分支）。
    const consoleBaseline = consoleErrors.length;
    const networkBaseline = networkFailures.length;
    const timeoutProbeStartedAt = Date.now();
    await page.route('**/v1/health/**', () => { /* 永久挂起：不 fulfill / 不 abort，模拟底层调用卡死 */ });
    await page.evaluate(() => { window.loadDashboard(); });
    await page.waitForFunction(() => {
      const element = document.getElementById('dashboard-error');
      return Boolean(element) && element.classList.contains('show') && element.textContent.includes('请求超时');
    }, null, { timeout: 25000, polling: 500 });

    const timeoutState = await page.evaluate(() => {
      const errorElement = document.getElementById('dashboard-error');
      const retryButton = errorElement
        ? errorElement.querySelector('[data-action="manualRefreshDashboard"]')
        : null;
      return {
        errorText: errorElement?.textContent?.trim() || '',
        hasRetryButton: Boolean(retryButton),
        retryDisabled: retryButton ? retryButton.disabled : null,
        loadingHidden: document.getElementById('dashboard-loading')?.classList.contains('hidden') ?? false,
        pageInteractive: document.readyState === 'complete',
      };
    });
    assert(timeoutState.errorText.includes('请求超时'), `超时态未给出"请求超时"提示: ${timeoutState.errorText}`);
    assert(timeoutState.hasRetryButton, '超时态缺少重试入口（用户无恢复路径）');
    assert(timeoutState.retryDisabled === false, '超时态重试按钮处于禁用态（无法恢复）');
    assert(timeoutState.loadingHidden, '超时后 loading 未收起（UI 卡死）');
    assert(timeoutState.pageInteractive, '超时后页面不可交互');

    await page.screenshot({ path: path.join(artifactDir, 'dashboard-timeout.png'), fullPage: true });
    fs.writeFileSync(path.join(artifactDir, 'timeout-path.json'), JSON.stringify({
      elapsedMs: Date.now() - timeoutProbeStartedAt,
      ...timeoutState,
      consoleErrorsSinceProbe: consoleErrors.slice(consoleBaseline),
      networkFailuresSinceProbe: networkFailures.slice(networkBaseline),
    }, null, 2), 'utf8');
    await page.unroute('**/v1/health/**');

    console.log(JSON.stringify({
      ok: true,
      baseUrl,
      dashboard,
      timeoutPath: { elapsedMs: Date.now() - timeoutProbeStartedAt, ...timeoutState },
      consoleErrorCount: consoleErrors.length,
      networkFailureCount: networkFailures.length,
      exemptedErrors: consoleErrors.length - unexpectedConsoleErrors.length,
      exemptedNetworkFailures: networkFailures.length - unexpectedNetworkFailures.length,
    }, null, 2));
  } catch (error) {
    failures.push(error.message);
    await page.screenshot({ path: path.join(artifactDir, 'failure.png'), fullPage: true }).catch(() => {});
    fs.writeFileSync(path.join(artifactDir, 'failure.html'), await page.content().catch(() => ''), 'utf8');
    fs.writeFileSync(path.join(artifactDir, 'console-errors.json'), JSON.stringify(consoleErrors, null, 2), 'utf8');
    fs.writeFileSync(path.join(artifactDir, 'network-failures.json'), JSON.stringify(networkFailures, null, 2), 'utf8');
    console.error(`[playwright-smoke] FAIL: ${error.message}`);
    process.exitCode = 1;
  } finally {
    await browser.close();
    fs.writeFileSync(path.join(artifactDir, 'summary.json'), JSON.stringify({ failures, consoleErrors, networkFailures }, null, 2), 'utf8');
  }
}

main().catch(error => {
  console.error(`[playwright-smoke] FATAL: ${error.message}`);
  process.exitCode = 1;
});
