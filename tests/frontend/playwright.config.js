/**
 * v0.8.26 CI-02 新增：Playwright 配置
 * 仪表盘前端冒烟测试配置
 */
const { defineConfig } = require('@playwright/test');

module.exports = defineConfig({
  testDir: '.',
  testMatch: '*.spec.js',
  timeout: 30000,
  expect: {
    timeout: 10000,
  },
  use: {
    baseURL: process.env.LRC_DASHBOARD_URL || 'http://127.0.0.1:3099',
    headless: true,
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: {
        browserName: 'chromium',
      },
    },
  ],
});