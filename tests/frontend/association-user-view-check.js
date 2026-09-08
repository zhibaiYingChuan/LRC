'use strict';
// 用户视角验证：在联想中心输入"今晚吃什么"并执行，抓取渲染结果与控制台错误
const fs = require('fs');
const path = require('path');
const { chromium } = require('playwright');

const baseUrl = process.env.LRC_BROWSER_BASE_URL || 'http://127.0.0.1:1420';
const artifactDir = path.join(process.cwd(), 'playwright-artifacts');
const consoleErrors = [];

async function main() {
  fs.mkdirSync(artifactDir, { recursive: true });
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });
  page.on('console', m => { if (m.type() === 'error') consoleErrors.push(m.text()); });
  page.on('pageerror', e => consoleErrors.push(`页面异常: ${e.message}`));

  try {
    await page.goto(`${baseUrl}/`, { waitUntil: 'domcontentloaded', timeout: 15000 });
    // 切换到联想中心
    await page.evaluate(() => { window.gotoAssociationCenter && window.gotoAssociationCenter(); });
    await page.locator('#tab-association-center').waitFor({ state: 'visible', timeout: 8000 });
    // 点击一键示例"今晚吃什么？"
    await page.locator('[data-explore-preset="今晚吃什么？"]').click();
    // 等待联想结果
    await page.locator('#association-conclusion').waitFor({ state: 'visible', timeout: 20000 });
    await page.waitForFunction(() => {
      const t = document.getElementById('association-conclusion')?.textContent || '';
      return t.includes('个念头') && !t.includes('正在回想');
    }, null, { timeout: 20000, polling: 500 });

    const result = await page.evaluate(() => {
      const q = document.getElementById('association-query')?.value || '';
      const conclusion = document.getElementById('association-conclusion')?.textContent?.trim() || '';
      const storyNodes = document.querySelectorAll('#association-story .association-story-node').length;
      const groups = [...document.querySelectorAll('#association-story .association-story-group')].map(g => g.querySelector('.association-subtitle')?.textContent?.trim());
      const techSummary = document.querySelector('.association-tech-details summary')?.textContent?.trim() || '';
      const trailSteps = document.querySelectorAll('#association-trail .association-trail-step').length;
      const evidenceItems = document.querySelectorAll('#association-evidence .association-evidence-item').length;
      const hasEmoji = /[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}\u{2B00}-\u{2BFF}]/u.test(document.body.innerText);
      const techOpen = document.querySelector('.association-tech-details')?.open || false;
      return { q, conclusion, storyNodes, groups, techSummary, trailSteps, evidenceItems, hasEmoji, techOpen };
    });

    await page.screenshot({ path: path.join(artifactDir, 'association-user-view.png'), fullPage: true });
    console.log(JSON.stringify({ ok: true, baseUrl, consoleErrors, result }, null, 2));
  } catch (error) {
    console.error(`[user-view] FAIL: ${error.message}`);
    await page.screenshot({ path: path.join(artifactDir, 'association-user-view-failure.png'), fullPage: true }).catch(() => {});
    console.log(JSON.stringify({ ok: false, consoleErrors }, null, 2));
    process.exitCode = 1;
  } finally {
    await browser.close();
  }
}

main().catch(e => { console.error(e); process.exitCode = 1; });
