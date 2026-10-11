# 跨平台审计报告 v4.0

> 审计日期：2026-07-28
> 审计目标：验证项目能否通过 CI/CD 自动编译 macOS/Linux/Windows 三平台桌面端，且各平台直接使用不出现 bug
> 审计动机：用户要求上传到仓库，通过 CI/CD 自动编译三平台，审计各平台兼容性

---

## 一、审计总览

| 审计维度 | P0 阻断 | P1 严重 | P2 中等 | P3 低等 | 总计 |
|---------|---------|---------|---------|---------|------|
| CI/CD 配置 | 1 | 0 | 0 | 0 | 1 |
| 跨平台 CORS/WebView 源 | 0 | 1 | 0 | 0 | 1 |
| 跨平台前端环境检测 | 0 | 1 | 0 | 0 | 1 |
| 跨平台进程管理 | 0 | 0 | 0 | 0 | 0 |
| 跨平台二进制命名/路径 | 0 | 0 | 0 | 0 | 0 |
| 跨平台构建脚本 | 0 | 0 | 0 | 1 | 1 |
| CSP 跨平台兼容性 | 0 | 0 | 1 | 0 | 1 |
| **合计** | **1** | **2** | **1** | **1** | **5** |

---

## 二、P0 阻断问题

### P0-A：没有 CI/CD 配置（GitHub Actions）

**位置**：`.github/workflows/` 目录不存在
**问题**：用户要求上传到仓库后自动编译三平台，但没有 GitHub Actions workflow 配置
**影响**：无法实现自动编译，完全阻断发布流程
**修复方案**：创建 `.github/workflows/build-desktop.yml`，配置 macOS/Linux/Windows 三平台矩阵构建

---

## 三、P1 严重问题

### P1-A：app.js Tauri 环境检测在 macOS/Linux 失败

**位置**：[static/app.js:14-15](file:///g:/code-memory/static/app.js#L14)
**问题**：
```javascript
const isTauriEnv = (typeof window.__TAURI__ !== 'undefined') ||
  (window.location.origin && window.location.origin.includes('tauri.localhost'));
```
- macOS/Linux 的 WebView 源是 `tauri://localhost`
- `tauri://localhost`.includes(`tauri.localhost`) → **false**（因为 `tauri://` 不等于 `tauri.`）
- 导致 macOS/Linux 上 `isTauriEnv = false`，`API_BASE` 错误使用 `window.location.origin`

**影响**：macOS/Linux 桌面端左下角会显示"服务已停止"（与 Windows 修复前相同的问题）

**修复方案**：
```javascript
const isTauriEnv = (typeof window.__TAURI__ !== 'undefined') ||
  (typeof window.__TAURI_INTERNALS__ !== 'undefined') ||
  (window.location.origin && (
    window.location.origin.includes('tauri.localhost') ||
    window.location.origin.startsWith('tauri://')
  ));
```

### P1-B：CORS 白名单缺少 `http://tauri.localhost`（Linux fallback）

**位置**：[src/server.rs:2774-2784](file:///g:/code-memory/src/server.rs#L2774)
**问题**：当前白名单有 `tauri://`（macOS/Linux）和 `https://tauri.localhost`（Windows useHttpsScheme），但缺少 `http://tauri.localhost`（Tauri 2.x Linux WebKitGTK 在某些配置下的源）

**影响**：某些 Linux 发行版上可能 CORS 被拒绝
**修复方案**：添加 `http://tauri.localhost` 到白名单

---

## 四、P2 中等问题

### P2-A：CSP 配置未包含 `tauri://` 协议

**位置**：[tauri.conf.json:30](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json#L30)
**问题**：CSP 的 `connect-src` 只包含 `http://127.0.0.1:*` 和 `http://localhost:*`，但没有 `tauri://` 协议
**影响**：macOS/Linux 上 Tauri IPC 可能被 CSP 阻止
**修复方案**：CSP 中添加 `tauri://` 到 `connect-src`

---

## 五、P3 低等问题

### P3-A：build.rs 错误信息硬编码 Windows 命令

**位置**：[build.rs:267-269](file:///g:/code-memory/desktop/src-tauri/build.rs#L267)
**问题**：错误信息硬编码了 `taskkill /F /IM code-memory-server.exe`，这在 macOS/Linux 上不适用
**影响**：仅影响错误提示，不影响功能
**修复方案**：根据平台显示不同的错误信息

---

## 六、跨平台兼容性验证（通过的方面）

### 6.1 sidecar 二进制命名 ✅
- [sidecar_manager.rs:218](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L218)：使用 `std::env::consts::EXE_SUFFIX`
- [main.rs:47](file:///g:/code-memory/desktop/src-tauri/src/main.rs#L47)：使用 `std::env::consts::EXE_EXTENSION`
- [build.rs:63-75](file:///g:/code-memory/desktop/src-tauri/build.rs#L63)：根据 `CARGO_CFG_TARGET_OS` 选择文件名

### 6.2 sidecar 搜索路径 ✅
- [sidecar_manager.rs:224-231](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L224)：包含 Windows/macOS/Linux 的搜索路径

### 6.3 进程管理 ✅
- [sidecar_manager.rs:380-381](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L380)：Windows 使用 `CREATE_NO_WINDOW`
- [sidecar_manager.rs:689-693](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L689)：Unix 使用 `libc::kill(pid, 0)`

### 6.4 日志路径 ✅
- [sidecar_manager.rs:36-46](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L36)：Windows 用 `$APPDATA`，Unix 用 `$HOME`

### 6.5 tauri.conf.json 资源配置 ✅
- `resources: ["lrc-sidecar*"]`：通配符匹配所有平台的二进制文件名

---

## 七、Tauri 2.x WebView 源参考（来自官方文档和社区）

| 平台 | WebView 引擎 | Origin（默认） | Origin（useHttpsScheme） |
|------|------------|--------------|----------------------|
| Windows | WebView2 (Edge) | `http://tauri.localhost` | `https://tauri.localhost` |
| macOS | WKWebView (Safari) | `tauri://localhost` | `tauri://localhost`（不受影响） |
| Linux | WebKitGTK | `tauri://localhost` | `tauri://localhost`（不受影响） |

**来源**：
- [Tauri 2.x IPC 深度解析](https://github.com/gxcsoccer/wrymium/blob/main/docs/ipc-deep-dive.md)
- [gptme Linux CORS issue #2226](https://github.com/gptme/gptme/issues/2226)
- [OpenJarvis Tauri CORS fix #292](https://github.com/open-jarvis/OpenJarvis/pull/292)
- [Tauri 配置参考](https://v2.tauri.app/reference/config/)

---

## 八、修复优先级

| 优先级 | 编号 | 问题 | 修复方式 |
|--------|------|------|---------|
| P0 | P0-A | 无 CI/CD 配置 | 创建 GitHub Actions workflow |
| P1 | P1-A | app.js 检测在 macOS/Linux 失败 | 添加 `tauri://` 检测 |
| P1 | P1-B | CORS 缺少 `http://tauri.localhost` | 添加到白名单 |
| P2 | P2-A | CSP 缺少 `tauri://` 协议 | 添加到 connect-src |
| P3 | P3-A | build.rs 错误信息硬编码 | 平台条件显示 |

---

## 九、结论

**当前状态**：无法直接上传仓库实现三平台自动编译。

**阻断原因**：
1. 没有 CI/CD 配置（P0-A）
2. macOS/Linux 上前端无法连接 sidecar（P1-A，与 Windows 修复前相同的问题）

**修复后预期**：三平台均可直接使用，无 bug。

**修复工作量**：5 个问题，分步修复，预计需要创建 1 个新文件（GitHub Actions）+ 修改 3 个文件。
