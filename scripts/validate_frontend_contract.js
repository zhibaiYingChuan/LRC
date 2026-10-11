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

// ===== 跨层契约校验（P1-2）：API 路径 / HTTP 方法 / Tauri invoke 命令名 =====
// 目的：前端调用面与后端路由面、桌面端命令注册面在 CI 中强制对齐，
//       避免"前端调用不存在的端点"或"invoke 调用未注册命令"这类跨层漂移。

const serverRs = fs.readFileSync(path.join(root, 'src', 'server.rs'), 'utf8');
const v1ApiRs = fs.readFileSync(path.join(root, 'src', 'v1_api.rs'), 'utf8');
const desktopMainRs = fs.readFileSync(
  path.join(root, 'desktop', 'src-tauri', 'src', 'main.rs'),
  'utf8'
);

/** 解析 Rust 侧 `.route("路径", 方法(...))` → Map<路径, Set<HTTP 方法>> */
function parseRoutes(source) {
  const routes = new Map();
  const re = /\.route\(\s*"([^"]+)"\s*,\s*(get|post|put|delete|patch)\s*\(/g;
  for (const match of source.matchAll(re)) {
    const [, routePath, verb] = match;
    if (!routes.has(routePath)) routes.set(routePath, new Set());
    routes.get(routePath).add(verb.toUpperCase());
  }
  return routes;
}

/**
 * 把源码中的注释与字符串内容替换为空格（保留引号与代码骨架，字符索引长度不变）。
 * 用途：在"无注释、无字符串干扰"的视图上做括号配对，避免注释里的伪调用被误判。
 */
function maskCode(source) {
  const chars = source.split('');
  let state = 'code';
  for (let i = 0; i < source.length; i++) {
    const ch = source[i];
    const next = source[i + 1];
    if (state === 'code') {
      if (ch === '/' && next === '/') {
        chars[i] = ' ';
        chars[i + 1] = ' ';
        i++;
        state = 'line-comment';
      } else if (ch === '/' && next === '*') {
        chars[i] = ' ';
        chars[i + 1] = ' ';
        i++;
        state = 'block-comment';
      } else if (ch === '"' || ch === "'" || ch === '`') {
        state = ch;
      }
      continue;
    }
    if (state === 'line-comment') {
      if (ch === '\n') state = 'code';
      else chars[i] = ' ';
      continue;
    }
    if (state === 'block-comment') {
      if (ch === '*' && next === '/') {
        chars[i] = ' ';
        chars[i + 1] = ' ';
        i++;
        state = 'code';
      } else {
        chars[i] = ' ';
      }
      continue;
    }
    // 字符串 / 模板字符串内部
    if (ch === '\\') {
      chars[i] = ' ';
      if (i + 1 < source.length) {
        chars[i + 1] = ' ';
        i++;
      }
      continue;
    }
    if (ch === state) state = 'code';
    else chars[i] = ' ';
  }
  return chars.join('');
}

const jsMasked = maskCode(js);

/** 收集前端函数调用的第一参数与其余参数文本（括号配对，跨行安全） */
function collectCallSites(fnName, masked, original) {
  const sites = [];
  let from = 0;
  for (;;) {
    const idx = masked.indexOf(`${fnName}(`, from);
    if (idx === -1) break;
    const prev = idx > 0 ? masked[idx - 1] : '';
    if (/[\w$.]/.test(prev)) {
      from = idx + fnName.length;
      continue;
    }
    const argStart = idx + fnName.length + 1;
    let depth = 1;
    let commaIdx = -1;
    let endIdx = -1;
    for (let i = argStart; i < masked.length; i++) {
      const ch = masked[i];
      if (ch === '(' || ch === '[' || ch === '{') depth++;
      else if (ch === ')' || ch === ']' || ch === '}') {
        depth--;
        if (depth === 0) {
          endIdx = i;
          break;
        }
      } else if (ch === ',' && depth === 1 && commaIdx === -1) {
        commaIdx = i;
      }
    }
    if (endIdx === -1) break;
    sites.push({
      arg1: original.slice(argStart, commaIdx === -1 ? endIdx : commaIdx),
      rest: commaIdx === -1 ? '' : original.slice(commaIdx + 1, endIdx),
    });
    from = endIdx + 1;
  }
  return sites;
}

// —— 检查 1/2：前端 API 调用路径 + HTTP 方法与后端路由对齐 ——
const callSites = [
  ...collectCallSites('fetchWithTimeout', jsMasked, js),
  ...collectCallSites('getJson', jsMasked, js),
];
const apiCalls = [];
for (const { arg1, rest } of callSites) {
  let routePath = null;
  const scoped = arg1.match(/\/(?:v1|api)\/[A-Za-z0-9_/-]+/);
  if (scoped) {
    routePath = scoped[0];
  } else {
    const bare = arg1.match(/\/(mcp|health)\b/);
    if (bare) routePath = `/${bare[1]}`;
  }
  if (!routePath) continue;
  const methodMatch = rest.match(/method\s*:\s*['"]([A-Za-z]+)['"]/);
  apiCalls.push({
    routePath,
    method: methodMatch ? methodMatch[1].toUpperCase() : 'GET',
  });
}

const v1Routes = parseRoutes(v1ApiRs);
const serverRoutes = parseRoutes(serverRs);
for (const { routePath, method } of apiCalls) {
  // /v1 由 server.rs 的 nest_service 挂载，v1_api.rs 内的路径不含 /v1 前缀
  const isV1 = routePath.startsWith('/v1/');
  const verbs = isV1 ? v1Routes.get(routePath.slice(3)) : serverRoutes.get(routePath);
  if (!verbs) {
    errors.push(`前端调用未注册的路径: ${routePath}`);
  } else if (!verbs.has(method)) {
    errors.push(`HTTP 方法不匹配: ${routePath} 前端 ${method}，后端仅 ${[...verbs].join('/')}`);
  }
}

// —— 检查 3：前端 invoke 命令名与桌面端 generate_handler! 注册清单对齐 ——
const invokeMapped = [
  ...js.matchAll(/['"]lrc-[a-z0-9-]+['"]\s*:\s*['"]([a-z0-9_]+)['"]/g),
].map((m) => m[1]);
const invokeDirect = [
  ...js.matchAll(/\binvokeFn\(\s*['"]([a-z0-9_]+)['"]/g),
  ...js.matchAll(/\binvokeWithTimeout\(\s*[A-Za-z_$][\w$]*\s*,\s*['"]([a-z0-9_]+)['"]/g),
].map((m) => m[1]);
const invokedCommands = [...new Set([...invokeMapped, ...invokeDirect])];

const handlerBlock = desktopMainRs.match(/generate_handler!\[([\s\S]*?)\]/);
const registeredCommands = new Set(
  [...(handlerBlock ? handlerBlock[1] : '').matchAll(/commands::([a-z0-9_]+)/g)].map((m) => m[1])
);
if (!registeredCommands.size) {
  errors.push('未能从桌面端 main.rs 解析出 Tauri 命令注册清单');
} else {
  for (const command of invokedCommands) {
    if (!registeredCommands.has(command)) {
      errors.push(`前端 invoke 调用未注册的 Tauri 命令: ${command}`);
    }
  }
}

// —— 检查 4：后端已注册但前端零调用的命令，必须在白名单中显式声明 ——
// 目的（v0.9.7，GLOBAL_CODE_REVIEW_REPORT「Tauri 命令归属不明」）：
//   把"注册了却不被调用"从**静默状态**变为**显式申报**。
//   此类命令可能是：桌面端内部直接调用的入口、对外 IPC 契约、或真正的死代码。
//   无论哪种，都必须在下方白名单中给出保留理由——否则门禁失败，
//   迫使新增的孤儿命令被及时发现（避免接口面无意识膨胀）。
const ORPHAN_ALLOWLIST = new Map([
  // 命令名 → 保留理由（须与实际调用链一致）
  [
    'open_dashboard_window',
    '桌面端内部改由 show_dashboard_in_main_window 直接调用；此命令为对外 IPC 契约',
  ],
  [
    'navigate_main_to_dashboard',
    '同上，向导完成后显示仪表盘的 IPC 入口',
  ],
  [
    'update_tray_tooltip',
    '桌面端内部已在 Agent 配置完成后直接调 tray::update_tooltip；此命令保留为 IPC 入口',
  ],
  [
    'bulk_apply_agent_overrides',
    '批量手动修正（含 HCSE FM-09 10s 超时 + 指数退避），供跨端同步/恢复场景',
  ],
]);

const orphans = [...registeredCommands].filter((c) => !invokedCommands.includes(c));
const undeclaredOrphans = orphans.filter((c) => !ORPHAN_ALLOWLIST.has(c));
const staleAllowlist = [...ORPHAN_ALLOWLIST.keys()].filter((c) => !orphans.includes(c));

if (undeclaredOrphans.length) {
  errors.push(
    `后端注册但前端零调用的命令未在白名单申报（新增孤儿命令须显式说明理由）: ${undeclaredOrphans.join(', ')}`
  );
}
if (staleAllowlist.length) {
  errors.push(
    `白名单中的命令已不再是孤儿（应移出白名单，避免理由腐化）: ${staleAllowlist.join(', ')}`
  );
}

// —— 检查 5：响应字段契约（v0.9.11 / G10 / CR-50）——
// 目的：此前本脚本只对齐「路径 + HTTP 方法 + invoke 命令名」，**完全不校验响应字段**
//   ⇒ `body.memories` vs `body.data` 这类**响应结构漂移在 CI 中不可见**
//     （本项目历史上已多次发生：改了后端字段名/漏加 `#[serde(rename)]`，
//      前端静默读到 undefined，页面表现为"空数据"而非报错）。
// 做法（静态、确定性、无需起服务）：
//   ① 解析 `src/v1_api.rs` 中响应结构体的**对外序列化字段**（尊重 `#[serde(rename)]`）；
//   ② 对契约表里每个"前端确实消费的 key"，断言它仍是该结构体的对外字段；
//   ③ 反向断言该 key **仍被前端消费** —— 否则说明契约表已腐化（前端改了、表没改）。
// 三条断言共同保证：**后端改名 ⇒ 门禁红；前端弃用 ⇒ 门禁红**，漂移无处可藏。

/** 解析 `pub struct X { … }` → Set<对外序列化字段名>（尊重 `#[serde(rename = "…")]`） */
function parseSerdeStructFields(source) {
  const structs = new Map();
  const re = /pub struct (\w+)\s*\{([\s\S]*?)\n\}/g;
  for (const match of source.matchAll(re)) {
    const [, name, body] = match;
    const fields = new Set();
    let pendingRename = null;
    for (const rawLine of body.split('\n')) {
      const line = rawLine.trim();
      const rename = line.match(/#\[serde\(rename\s*=\s*"([^"]+)"/);
      if (rename) {
        pendingRename = rename[1];
        continue;
      }
      if (!line || line.startsWith('//') || line.startsWith('#[')) continue;
      const field = line.match(/^pub\s+(\w+)\s*:/);
      if (field) {
        fields.add(pendingRename || field[1]);
        pendingRename = null;
      }
    }
    structs.set(name, fields);
  }
  return structs;
}

/** 前端是否确实读取了某个响应 key（`.key` / `['key']` / `["key"]` 三种写法） */
function frontendReadsKey(jsSource, key) {
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`[.\\['"]${escaped}\\b`).test(jsSource);
}

const serdeStructs = parseSerdeStructFields(v1ApiRs);

/**
 * 响应字段契约表：**只登记"前端确实在消费"的 key**（由 `frontendReadsKey` 反向验证）。
 * 新增/修改前端消费的响应字段时，必须同步更新本表 —— 这正是门禁的意义。
 */
const RESPONSE_CONTRACT = [
  { struct: 'EnrichResponse', keys: ['memories', 'explanation'] },
  { struct: 'EnrichExplanation', keys: ['query', 'items'] },
  { struct: 'AssociationExploreResponse', keys: ['nodes', 'edges'] },
  { struct: 'UnfoldResponse', keys: ['sub_memories'] },
];

let responseFieldCount = 0;
for (const { struct, keys } of RESPONSE_CONTRACT) {
  const fields = serdeStructs.get(struct);
  if (!fields) {
    errors.push(`响应契约引用了不存在的结构体: ${struct}（后端已改名或删除，请同步本表）`);
    continue;
  }
  for (const key of keys) {
    if (!fields.has(key)) {
      errors.push(
        `响应字段契约漂移: ${struct} 已不含前端消费的字段 \`${key}\`` +
          `（当前对外字段: ${[...fields].join(', ') || '（空）'}）`
      );
      continue;
    }
    if (!frontendReadsKey(js, key)) {
      errors.push(
        `响应契约表腐化: 前端已不再消费 \`${key}\`，请从 ${struct} 的契约中移除该项`
      );
      continue;
    }
    responseFieldCount++;
  }
}

if (errors.length) {
  console.error(errors.map((error) => `ERROR: ${error}`).join('\n'));
  process.exit(1);
}

const apiPathCount = new Set(apiCalls.map((call) => call.routePath)).size;
console.log(
  `前端契约通过：${new Set(tabs).size} 个导航、${ids.length} 个 DOM id、静态资源完整、` +
    `跨层契约 ${apiPathCount} 条 API 路径与 ${invokedCommands.length} 个 invoke 命令全部对齐、` +
    `${responseFieldCount} 个响应字段与后端结构体对齐、` +
    `${orphans.length} 个孤儿命令已申报`
);
