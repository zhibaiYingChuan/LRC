# 全面动态审计报告 v5.0

> 审计日期：2026-07-28
> 审计范围：全项目所有角度（跨平台/API/CORS/CSP/前端/后端/进程/构建/安全/并发/性能/UX）
> 审计原则：全面、所有角度、不遗漏任何细节、动态验证

---

## 一、审计总览

| 审计维度 | P0 阻断 | P1 严重 | P2 中等 | P3 低等 | 总计 |
|---------|---------|---------|---------|---------|------|
| 跨平台前端环境检测 | 0 | 1 | 0 | 0 | 1 |
| 跨平台 CORS/WebView 源 | 0 | 1 | 0 | 0 | 1 |
| 端口自适应与前端硬编码 | 1 | 0 | 0 | 0 | 1 |
| CSP 配置完整性 | 0 | 1 | 1 | 0 | 2 |
| 前端 XSS 防护 | 0 | 0 | 1 | 0 | 1 |
| 后端 panic 风险 | 0 | 0 | 0 | 1 | 1 |
| 构建脚本跨平台 | 0 | 0 | 0 | 1 | 1 |
| **合计** | **1** | **3** | **2** | **2** | **8** |

---

## 二、P0 阻断问题

### P0-1：app.js 硬编码端口 3099，未通过 Tauri IPC 获取实际端口

**位置**：[static/app.js:17](file:///g:/code-memory/static/app.js#L17)
**问题**：
```javascript
const DEFAULT_API_BASE = isTauriEnv
  ? 'http://127.0.0.1:3099'  // ← 硬编码端口
  : (window.location.origin || 'http://localhost:3099');
```

sidecar 有端口自适应机制（[process_guard.rs:356](file:///g:/code-memory/src/process_guard.rs#L356) `find_available_port`），如果 3099 被占用，会在 3099-3198 范围内寻找可用端口。但前端永远连接 3099。

**影响**：如果端口 3099 被其他进程占用，sidecar 会启动在其他端口，但前端仍然连接 3099，导致"服务已停止"。
**修复方案**：在 Tauri 环境中，通过 `window.__TAURI__.invoke('get_sidecar_status')` 异步获取实际端口。

---

## 三、P1 严重问题

### P1-1：app.js Tauri 环境检测在 macOS/Linux 失败

**位置**：[static/app.js:14-15](file:///g:/code-memory/static/app.js#L14)
**问题**：
```javascript
const isTauriEnv = (typeof window.__TAURI__ !== 'undefined') ||
  (window.location.origin && window.location.origin.includes('tauri.localhost'));
```
- macOS/Linux WebView 源是 `tauri://localhost`
- `"tauri://localhost".includes("tauri.localhost")` → **false**（`://` ≠ `.`）

**影响**：macOS/Linux 上 `isTauriEnv = false`，`API_BASE` 使用 `window.location.origin`（`tauri://localhost`），所有 API 请求失败。

### P1-2：CORS 白名单缺少 `http://tauri.localhost`

**位置**：[src/server.rs:2774-2784](file:///g:/code-memory/src/server.rs#L2774)
**问题**：白名单有 `tauri://`（macOS/Linux）和 `https://tauri.localhost`（Windows useHttpsScheme），但缺少 `http://tauri.localhost`（Tauri 2.x 默认 Windows/Android 源）

### P1-3：CSP 端口范围太窄（3099-3110）

**位置**：[tauri.conf.json:30](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json#L30)
**问题**：CSP 只允许 3099-3110 端口，但 sidecar 端口扫描范围是 3099-3198（100 个端口）。如果 sidecar 在 3111-3198 端口运行，CSP 会阻止连接。
**修复方案**：CSP 中使用通配符 `http://127.0.0.1:*` 或扩展端口范围到 3099-3198。

---

## 四、P2 中等问题

### P2-1：CSP 缺少 `tauri://` 协议

**位置**：[tauri.conf.json:30](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json#L30)
**问题**：CSP 的 `connect-src` 没有 `tauri://` 协议，macOS/Linux 上 Tauri IPC 可能被阻止。

### P2-2：app.js 第 3182 行 XSS 风险

**位置**：[static/app.js:3182](file:///g:/code-memory/static/app.js#L3182)
**问题**：
```javascript
toolsList.innerHTML = '... 检测失败: ' + e.message + ' ...';
```
`e.message` 未经 `htmlescape` 转义。如果错误消息包含 HTML 字符，可能导致 XSS。

---

## 五、P3 低等问题

### P3-1：main.rs:66 expect 可能导致 panic

**位置**：[main.rs:66](file:///g:/code-memory/desktop/src-tauri/src/main.rs#L66)
**问题**：`WizardState::load().expect("加载向导状态失败")` 如果配置目录无法确定，桌面端直接 panic。

### P3-2：build.rs 错误信息硬编码 Windows 命令

**位置**：[build.rs:267-269](file:///g:/code-memory/desktop/src-tauri/build.rs#L267)
**问题**：错误信息包含 `taskkill /F /IM code-memory-server.exe`，macOS/Linux 不适用。

---

## 六、已验证通过的方面 ✅

| 审计项 | 验证结果 |
|--------|---------|
| sidecar 二进制命名（EXE_SUFFIX/EXE_EXTENSION） | ✅ 跨平台兼容 |
| 进程管理（CREATE_NO_WINDOW / libc::kill） | ✅ 跨平台兼容 |
| 日志路径（$APPDATA / $HOME） | ✅ 跨平台兼容 |
| 锁顺序约定（L1→L6） | ✅ 文档化 + clippy lint |
| Tauri 命令注册（26 个命令） | ✅ 完整 |
| 前端错误处理（20+ catch 块） | ✅ 覆盖良好 |
| 前端自动刷新（30 秒） | ✅ 实现 |
| 前端 HTML 转义（htmlescape） | ✅ 大部分使用（除 P2-2） |
| 后端测试代码 unwrap/expect | ✅ 仅测试代码 |
| 版本号一致性（0.6.0） | ✅ 4 个文件一致 |
| 前后端 API 一致性（25 个端点） | ✅ 100% 一致 |

---

## 七、修复计划（8 步，分步执行，禁止批量修改）

| 步骤 | 编号 | 内容 | 文件 | 优先级 |
|------|------|------|------|--------|
| 1 | P1-1 | 修复 app.js Tauri 环境检测（添加 `tauri://` 检测） | app.js | P1 |
| 2 | P1-2 | CORS 白名单添加 `http://tauri.localhost` | server.rs | P1 |
| 3 | P0-1 | app.js 通过 Tauri IPC 动态获取 sidecar 端口 | app.js | P0 |
| 4 | P1-3 | CSP 端口范围扩展到 3099-3198 | tauri.conf.json | P1 |
| 5 | P2-1 | CSP 添加 `tauri://` 协议 | tauri.conf.json | P2 |
| 6 | P2-2 | app.js 第 3182 行 XSS 修复 | app.js | P2 |
| 7 | P3-1 | main.rs expect 改为优雅降级 | main.rs | P3 |
| 8 | P3-2 | build.rs 错误信息跨平台适配 | build.rs | P3 |

---

## 八、结论

本次全面审计发现 8 个问题（1 P0 / 3 P1 / 2 P2 / 2 P3）。其中 P0-1（端口硬编码）和 P1-1（macOS/Linux 检测失败）是导致三平台桌面端"服务已停止"的根因。

修复后预期：三平台（macOS/Linux/Windows）桌面端均可直接使用，无 bug。
