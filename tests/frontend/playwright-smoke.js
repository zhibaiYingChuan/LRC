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
    console.log(JSON.stringify({ ok: true, baseUrl, dashboard }, null, 2));
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
