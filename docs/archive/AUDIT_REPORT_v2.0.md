# 全局动态审计报告 v2.0

> 审计日期：2026-07-28
> 审计范围：g:\code-memory 全项目（前端 + 后端 + 桌面端 + 文档 + 配置 + 构建）
> 审计视角：前后端API一致性、代码质量、安全性、桌面端集成、配置构建、文档一致性
> 审计动因：v1.0 测试报告存在虚假性（服务未真正运行），需要重新审计并编译桌面端进行真实测试

---

## 一、审计总览

| 审计维度 | 严重 | 中等 | 低等 | 总计 |
|---------|------|------|------|------|
| 桌面端集成与构建 | 4 | 2 | 1 | 7 |
| 代码质量与错误处理 | 2 | 4 | 2 | 8 |
| 安全性 | 1 | 2 | 2 | 5 |
| 前后端API一致性 | 0 | 2 | 1 | 3 |
| 配置一致性 | 2 | 1 | 1 | 4 |
| 文档一致性 | 0 | 2 | 1 | 3 |
| **合计** | **9** | **13** | **8** | **30** |

---

## 二、桌面端集成与构建审计（核心阻断问题）

### 2.1 🔴 P0-A：lrc-sidecar 二进制文件不存在

**位置**：`desktop/src-tauri/` 目录
**问题**：整个项目找不到 `lrc-sidecar.exe` 二进制文件，但 `tauri.conf.json` 的 `resources` 配置了 `"lrc-sidecar*"`，`main.rs` 也引用它作为 sidecar 进程。

**根因**：
- `build.rs` 的 `sync_sidecar_binary()` 函数会从 `target/release/code-memory-server.exe` 复制到 `desktop/src-tauri/lrc-sidecar.exe`
- 但主项目尚未执行 `cargo build --release --features server`，因此源文件不存在
- `build.rs` 仅打印 warning 但不阻断构建，导致桌面端可能打包成功但运行时找不到 sidecar

**影响**：桌面端编译后无法启动后端服务，所有功能不可用
**修复方案**：编译桌面端前必须先执行 `cargo build --release --features server`

### 2.2 🔴 P0-B：桌面端前端资源指向错误

**位置**：`desktop/src-tauri/tauri.conf.json:7`
**问题**：`frontendDist` 指向 `"../src"`（即 `desktop/src/`），但实际功能完整的前端资源在 `g:\code-memory\static\`。

**当前状态**：
- `desktop/src/index.html` 是配置向导页面（简陋）
- `desktop/src/wizard.js` 是向导脚本
- 真正的仪表盘在 `g:\code-memory\static\index.html`（功能完整）

**影响**：桌面端窗口只能显示配置向导，无法显示仪表盘
**修复方案**：将 `frontendDist` 指向 `"../../static"`，或将 static 目录复制到 desktop/src/

### 2.3 🔴 P0-C：桌面端版本号不一致

**位置**：多处版本号配置
| 文件 | 当前版本 | 应有版本 |
|------|---------|---------|
| `Cargo.toml`（主项目） | 0.6.0 | 0.6.0 ✅ |
| `desktop/package.json` | 0.5.12 | 0.6.0 ❌ |
| `desktop/src-tauri/Cargo.toml` | 0.5.18 | 0.6.0 ❌ |
| `desktop/src-tauri/tauri.conf.json` | 0.5.18 | 0.6.0 ❌ |

**影响**：版本号混乱，无法标识本次迭代
**修复方案**：统一为 0.6.0

### 2.4 🔴 P0-D：桌面端缺少 sidecar 启动前置条件检查

**位置**：`desktop/src-tauri/src/main.rs:44-54`
**问题**：`SidecarManager::new()` 在二进制不存在时仅打印 warning，不返回错误。用户运行桌面端时会看到"sidecar 未运行"但无法定位原因。

**修复方案**：启动时检查二进制是否存在，不存在则弹出友好提示

### 2.5 🟡 P1-A：build.rs 错误处理过于宽松

**位置**：`desktop/src-tauri/build.rs:168-174`
**问题**：未找到源二进制时仅 `println!("cargo:warning=...")` 然后 `return`，不触发编译错误。这会导致桌面端打包成功但运行时失败。

**修复方案**：在 release 构建时，若未找到源二进制应 `panic!` 或返回非零退出码

### 2.6 🟡 P1-B：桌面端窗口尺寸过小

**位置**：`desktop/src-tauri/tauri.conf.json:19-20`
**问题**：窗口尺寸 `600x700`，但仪表盘设计文档要求最小 `1280x800`。当前尺寸无法完整显示仪表盘内容。

**修复方案**：调整为 `1280x800`，并启用 `resizable: true`

### 2.7 🟢 P2-A：图标资源不完整

**位置**：`desktop/src-tauri/icons/`
**问题**：目录中存在大量临时文件（`894.png`、`894_real.png`、`Square*Logo.png` 等），但缺少 macOS 需要的 `icon.icns`（实际存在但未在 tauri.conf.json 中引用）。

---

## 三、代码质量与错误处理审计

### 3.1 ✅ P0-E：server.rs 大量 expect() — 误报修正

**位置**：`src/server.rs` 行 2886、2899、2959、2980、3018、3104、3156、3193、3218、3258、3285、3345、3386、3421、3446
**状态**：✅ 误报。所有 expect() 调用都在 `#[cfg(test)]` 模块中（测试代码），测试中使用 expect() 是合理的，测试失败应该 panic。

### 3.2 ✅ P0-F：v1_api.rs 错误处理缺失 — 误报修正

**位置**：`src/v1_api.rs` 行 335-353、388-404
**状态**：✅ 误报。consolidate 和 enrich handler 中已使用 `unwrap_or_else(|e| { eprintln!(...); 默认值 })` 模式处理错误，不会 panic。
**改进建议**（降级为 P2）：将 eprintln! 替换为 tracing::warn!，并考虑返回 HTTP 错误响应而非默认值。

### 3.3 🟡 P1-C：persistence/postgres.rs 错误隐式忽略

**位置**：`src/persistence/postgres.rs` 行 365-400、447-459、466-477
**问题**：
- `save_memory` 中所有绑定参数使用 `.as_str().unwrap_or("")`，隐式忽略错误
- `delete_memory` 和 `clear_memories` 未校验执行结果
- `size_bytes` 直接返回 0 而非实际查询数据库

**修复方案**：返回 Result 类型，错误向上传播

### 3.4 ✅ P1-D：app.js 中大量 innerHTML 拼接 — 误报修正

**位置**：`static/app.js` 行 247、267、291、335、352、372、420、710、755、791 等
**状态**：✅ 误报。经检查，所有用户内容（如记忆内容、项目名称、描述等）均已使用 `htmlescape()` 转义。未转义的插值均为程序内部值（如时间、数字、百分比），不存在 XSS 风险。

### 3.5 🟡 P1-E：engine/manager.rs 边界条件未处理

**位置**：`src/engine/manager.rs` 行 629-632、671-672、730-735
**问题**：
- `save_embedding_cache` 未校验 `data_dir` 路径有效性
- `load_embedding_cache` 未校验 `chunks` 和 `vectors` 长度一致性
- `get_stats` 计算 `avg_lines` 时分母可能为 0

**修复方案**：添加边界检查

### 3.6 🟡 P1-F：bin/server.rs 路径处理薄弱

**位置**：`src/bin/server.rs` 行 512-514、536-543、553-568
**问题**：
- `canonicalize()` 失败时使用 `unwrap_or(default)` 但 default 可能无效
- `project_fingerprint_with_path` 未处理源码路径不存在的情况
- migration 调用缺少错误处理

### 3.7 🟢 P2-B：desktop/sidecar_manager.rs 子进程错误处理

**位置**：`desktop/src-tauri/src/sidecar_manager.rs` 行 86-89、136-139
**问题**：子进程创建和 kill 时未做错误处理，但已有 `Drop` 守卫，影响较小。

### 3.8 🟢 P2-C：desktop/agent_detector.rs 检测失败处理

**位置**：`desktop/src-tauri/src/agent_detector.rs` 行 78-82、107-113
**问题**：`detect_agents` 和 `check_agent` 中 spawn 失败未处理，但实际使用 filesystem 检测，影响较小。

---

## 四、安全性审计

### 4.1 🔴 P0-G：CSP 配置过于宽松

**位置**：`desktop/src-tauri/tauri.conf.json:28`
**问题**：CSP 允许 `http://127.0.0.1:*` 和 `http://localhost:*` 任意端口，`style-src` 允许 `'unsafe-inline'`，存在 XSS 风险。

**当前 CSP**：
```
default-src 'self' http://127.0.0.1:* http://localhost:*;
script-src 'self' http://127.0.0.1:* http://localhost:*;
style-src 'self' 'unsafe-inline' https://fonts.googleapis.com ...;
```

**修复方案**：收紧为具体端口号（如 3099-3198）

### 4.2 🟡 P1-G：embedder_apply_handler 配置文件写入

**位置**：`src/server.rs:2396-2400`
**问题**：使用 `format!` 拼接 TOML 内容，若 `model_id` 包含特殊字符（如 `"`）可能导致配置文件注入。

**当前防护**：已有白名单校验（行 2359），但应追加 TOML 转义。

**修复方案**：对 model_id 进行 TOML 字符串转义

### 4.3 🟡 P1-H：app.js 中未转义的 innerHTML

**位置**：`static/app.js` 多处（见 3.4）
**问题**：虽然后端返回的数据相对可信，但前端应防御性转义。

### 4.4 🟢 P2-D：CORS 配置已收紧（v1.0 修复确认）

**位置**：`src/server.rs:2766-2770`
**状态**：✅ 已从 `permissive` 收紧为白名单，允许 localhost、127.0.0.1、tauri 协议。

### 4.5 🟢 P2-E：路径遍历防护已加固（v1.0 修复确认）

**位置**：`src/server.rs:2267`
**状态**：✅ 已添加显式路径遍历字符检查。

---

## 五、前后端API一致性审计

### 5.1 ✅ v1.0 修复确认

- `/v1/memories/list` ✅ 已实现
- `/v1/memories/archive` ✅ 已实现
- `/v1/memories/remember` ✅ 已实现
- `/api/embedder/status` ✅ 已实现
- `/api/embedder/download` ✅ 已实现（含路径遍历防护）
- `/api/embedder/apply` ✅ 已实现
- `/api/embedder/test` ✅ 已实现
- `/api/tools/detect` ✅ 已实现

### 5.2 🟡 P1-I：tools_detect 检测逻辑真实可用性

**位置**：`src/server.rs:2489-2492`
**问题**：检测逻辑依赖 `Command::new` 调用外部命令，但实际代码使用 filesystem 检查（`std::path::Path::exists()`），检测准确性依赖于 PATH 环境变量。

**修复方案**：增加安装路径回退检测（基于注册表或常见安装目录）

### 5.3 🟡 P1-J：embedder/test 网络超时处理

**位置**：`src/server.rs` embedder_test_handler
**问题**：v1.0 测试中发现网络超时，但代码中未设置 HTTP 超时。

**修复方案**：添加 5 秒超时

### 5.4 🟢 P2-F：后端有但前端未调用的 API

- `POST /v1/feedback`
- `GET /v1/version/check`

---

## 六、配置一致性审计

### 6.1 🔴 P0-H：主项目与桌面端版本号不一致

（见 2.3）

### 6.2 🔴 P0-I：tauri.conf.json frontendDist 指向错误

（见 2.2）

### 6.3 🟡 P1-K：Cargo.toml 中 desktop/src-tauri 被 exclude

**位置**：`Cargo.toml:3`
**问题**：`exclude = ["desktop/src-tauri"]`，桌面端是独立 workspace，但实际构建时需要主项目先编译。

**状态**：这是设计决策，非问题，但需在文档中说明。

### 6.4 🟢 P2-G：缺少 .env.example

**问题**：项目缺少环境变量示例文件。
**修复方案**：创建 `.env.example` 列出所有可配置环境变量。

---

## 七、文档一致性审计

### 7.1 🟡 P1-L：CHANGELOG.md 未更新 v0.6.0

**位置**：`CHANGELOG.md`
**问题**：未记录 v0.6.0 的变更内容。
**修复方案**：补充 v0.6.0 变更记录。

### 7.2 🟡 P1-M：TEST_REPORT_v1.0.md 测试结果虚假

**位置**：`docs/TEST_REPORT_v1.0.md`
**问题**：测试报告声称"服务运行中"，但实际服务未启动，测试结果不可信。
**修复方案**：本次审计后重新执行真实测试，形成 v2.0 报告。

### 7.3 🟢 P2-H：README.md 未更新

**问题**：README 中版本号和功能描述可能过时。
**修复方案**：更新为 v0.6.0。

---

## 八、优先修复排序

### P0 阻断性问题（必须立即修复，影响桌面端编译和运行）

| 编号 | 问题 | 文件 | 工作量 |
|------|------|------|--------|
| P0-A | lrc-sidecar 二进制不存在 | 需先编译主项目 | 编译命令 |
| P0-B | frontendDist 指向错误 | tauri.conf.json | 1 行 |
| P0-C | 版本号不一致 | 3 个文件 | 3 行 |
| P0-D | sidecar 启动前置检查 | main.rs | 10 行 |
| P0-E | server.rs expect() panic | server.rs | 15 处 |
| P0-F | v1_api.rs 错误处理 | v1_api.rs | 5 处 |
| P0-G | CSP 配置过宽 | tauri.conf.json | 1 行 |
| P0-H | 版本号不一致（同 P0-C） | - | - |
| P0-I | frontendDist（同 P0-B） | - | - |

### P1 严重问题（本轮迭代修复）

| 编号 | 问题 | 文件 |
|------|------|------|
| P1-A | build.rs 错误处理 | build.rs |
| P1-B | 窗口尺寸过小 | tauri.conf.json |
| P1-C | postgres.rs 错误忽略 | postgres.rs |
| P1-D | app.js innerHTML 转义 | app.js |
| P1-E | manager.rs 边界条件 | manager.rs |
| P1-F | bin/server.rs 路径处理 | bin/server.rs |
| P1-G | embedder_apply TOML 转义 | server.rs |
| P1-H | app.js XSS 防护 | app.js |
| P1-I | tools_detect 路径回退 | server.rs |
| P1-J | embedder/test 超时 | server.rs |
| P1-K | 文档说明 | Cargo.toml |
| P1-L | CHANGELOG 更新 | CHANGELOG.md |
| P1-M | 测试报告重做 | TEST_REPORT_v2.0.md |

### P2 中等问题（后续迭代修复）

| 编号 | 问题 |
|------|------|
| P2-A | 图标资源清理 |
| P2-B | sidecar 错误处理 |
| P2-C | agent_detector 错误处理 |
| P2-D | CORS（已修复确认） |
| P2-E | 路径遍历（已修复确认） |
| P2-F | 未使用 API 对接 |
| P2-G | .env.example |
| P2-H | README 更新 |

---

## 九、与 v1.0 审计的差异说明

### v1.0 误报修正
- ✅ wizardStep4Search() 确为误报
- ✅ tools_detect 命令注入确为误报（实际是 PATH 劫持风险）

### v1.0 遗漏的问题（v2.0 新发现）
- ❌ v1.0 未发现 lrc-sidecar 二进制不存在（最严重阻断问题）
- ❌ v1.0 未发现 frontendDist 指向错误
- ❌ v1.0 未发现版本号不一致
- ❌ v1.0 未发现 server.rs 大量 expect() panic 风险
- ❌ v1.0 未发现 v1_api.rs 错误处理缺失
- ❌ v1.0 测试报告虚假（服务未运行）

### v1.0 已修复确认
- ✅ CORS 收紧
- ✅ 路径遍历防护
- ✅ 3个新增 API 实现
- ✅ console.log 清理
- ✅ 重复 id 修复

---

## 十、结论

本次审计共发现 30 个问题（9严重/13中等/8低等），其中 9 个 P0 问题直接阻断桌面端编译和运行。v1.0 审计虽然修复了部分问题，但遗漏了最关键的桌面端集成问题（sidecar 二进制不存在、frontendDist 错误、版本号不一致），且测试报告虚假。

**当前项目状态**：不可发布。必须修复 P0 问题后才能编译桌面端进行真实测试。
