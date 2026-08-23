'use strict';

const fs = require('fs');
const path = require('path');

const root = path.resolve(__dirname, '..');
const staticDir = path.join(root, 'static');
const html = fs.readFileSync(path.join(staticDir, 'index.html'), 'utf8');
const js = fs.readFileSync(path.join(staticDir, 'app.js'), 'utf8');
const cssFiles = fs.readdirSync(staticDir).filter((name) => name.endsWith('.css'));
const css = cssFiles.map((name) => fs.readFileSync(path.join(staticDir, name), 'utf8')).join('\n');
const errors = [];

const ids = [...html.matchAll(/\bid=["']([^"']+)["']/g)].map((m) => m[1]);
const duplicateIds = [...new Set(ids.filter((id, index) => ids.indexOf(id) !== index))];
if (duplicateIds.length) errors.push(`重复 DOM id: ${duplicateIds.join(', ')}`);

const tabs = [...html.matchAll(/data-tab=["']([^"']+)["']/g)].map((m) => m[1]);
for (const tab of [...new Set(tabs)]) {
  if (!new RegExp(`id=["']tab-${tab}["']`).test(html)) {
    errors.push(`导航缺少对应内容面板: ${tab}`);
  }
}

for (const src of [...html.matchAll(/<(?:script|img|link)[^>]+(?:src|href)=["']([^"']+)["']/g)].map((m) => m[1])) {
  if (/^(https?:|data:|blob:|#)/.test(src)) continue;
  const relative = src.replace(/^\//, '');
  if (!fs.existsSync(path.join(staticDir, relative))) errors.push(`静态资源不存在: ${src}`);
}

for (const icon of [...js.matchAll(/['"](icon-[a-z0-9-]+\.svg)['"]/g)].map((m) => m[1])) {
  if (!fs.existsSync(path.join(staticDir, 'assets', 'icons', icon))) errors.push(`脚本引用图标不存在: ${icon}`);
}

if (!html.includes('class="app-layout"')) errors.push('缺少 app-layout 根布局');
if (!html.includes('meta name="version"')) errors.push('缺少前端版本 meta');
if (!js.includes('window.__LRC_VERSION__')) errors.push('缺少前端版本运行时标识');
if (html.includes('v0.9.2')) errors.push('仍存在 v0.9.2 前端回退版本');
if (html.includes('onclick=')) errors.push('禁止继续新增内联 onclick 事件');

const cssRefs = [
  ...css.matchAll(/url\(\s*["']?([^"')]+)["']?\s*\)/g),
  ...css.matchAll(/@import\s+["']([^"']+)["']/g),
].map((m) => m[1]);
for (const ref of cssRefs) {
  if (/^(https?:|data:|blob:|#)/.test(ref)) continue;
  const relative = ref.replace(/^\//, '');
  if (!fs.existsSync(path.join(staticDir, relative))) errors.push(`CSS 资源不存在: ${ref}`);
}

const actionNames = [...html.matchAll(/data-action=["']([^"']+)["']/g)].map((m) => m[1]);
for (const action of [...new Set(actionNames)]) {
  if (!new RegExp(`(?:function\\s+${action}\\b|window\\.${action}\\s*=)`).test(js)) {
    errors.push(`data-action 缺少处理函数: ${action}`);
  }
}

if (errors.length) {
  console.error(errors.map((error) => `ERROR: ${error}`).join('\n'));
  process.exit(1);
}

console.log(`前端契约通过：${new Set(tabs).size} 个导航、${ids.length} 个 DOM id、静态资源完整`);
