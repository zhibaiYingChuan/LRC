# LRC 项目全量审计报告 v0.5.4

> 审计日期：2026-06-18
> 审计范围：`g:\code-memory` 全项目，每文件每行代码
> 审计方法：自动化静态分析 + 子智能体交叉验证

---

## 一、MCP 配置问题

### 1.1 二进制命名不一致 — 严重

**状态**：✅ 已修复

**问题**：`generate_config()` 方法在生成 MCP 配置时，stdio 传输模式下所有 Agent 都硬编码了 `"command": "lrc"`，但系统中不存在名为 `lrc` 的二进制文件。实际二进制名为 `lrc-sidecar.exe`。

| 文件 | 行号 | 说明 |
|---|---|---|
| `desktop/src-tauri/src/agent_detector.rs` | 784 | `DotDirDetector::generate_config()` |
| `desktop/src-tauri/src/agent_detector.rs` | 910 | `TraeDetector::generate_config()` |
| `desktop/src-tauri/src/agent_detector.rs` | 968 | `TraeCNDetector::generate_config()` |
| `desktop/src/index.html` | 240,253,266,279,296,313,326,343 | 8 处手动配置示例 |

**影响**：IDE 尝试启动 `lrc` 进程失败（ENOENT），MCP 连接完全不可用。

**修复**：所有 `"command": "lrc"` → `"command": "lrc-sidecar"`，替换逻辑同时匹配两者保持向后兼容。

### 1.2 替换逻辑脆弱 — 中等

**状态**：✅ 已修复

**问题**：`configure()` 中第 1371 行的字符串比较 `cmd == "lrc"` 是精确匹配，如果 `generate_config()` 格式变化，替换将静默失败。

**修复**：改为 `cmd_str == "lrc" || cmd_str == "lrc-sidecar"` 并添加 `.to_string()` 避免借用冲突。

### 1.3 配置写入后无验证 — 中等

**状态**：⚠️ 待修复

**问题**：`write_or_merge_config()` 写入后无回读验证，磁盘满或权限问题可能导致静默失败。

**位置**：`agent_detector.rs` 第 1477-1483 行、第 1499-1504 行

**建议**：写入后回读文件内容，与预期 JSON 进行比对。

---

## 二、Agent 检测问题

### 2.1 CodeBuddy 等工具无法被检测 — 严重

**状态**：✅ 已修复

**问题**：`check_known_tool()` 对无 `binary_paths` 的工具（CodeBuddy、Cline、Roo 等）直接返回 `false`。

**影响**：这些工具在 Agent 列表中永远显示为"未安装"，用户无法为其配置 MCP。

**修复**：添加策略 2 回退检测 — 检查 `primary_marker` 目录是否存在（如 `~/.codebuddy`）。

### 2.2 扫描卡住 — 严重

**状态**：✅ 已修复

**问题**：`detect_agents` 等命令使用 `block_in_place` 内部调用 `emit` 发送进度事件，导致 Tauri 事件循环死锁。

**修复**：移除 `block_in_place`，改为直接同步调用；移除 `emit` 进度回调。

---

## 三、LLM 配置问题

### 3.1 仪表盘显示"未配置" — 严重

**状态**：✅ 已修复

**问题**：桌面端 `save_llm_config` 仅写入 `wizard.json`，不调用 Sidecar 的 `POST /api/config/llm`。仪表盘读取的是 Sidecar 内存中的 `state.llm_api`，两者不同步。

**数据流**：
```
桌面端 save_llm_config → wizard.json ✅
                       → Sidecar POST /api/config/llm ❌（缺失）
仪表盘 GET /api/config → Sidecar state.llm_api → 未配置
```

**修复**：`save_llm_config` 和 `clear_llm_config` 成功后自动调用 Sidecar API 同步。

### 3.2 状态栏显示"未配置" — 严重

**状态**：✅ 已修复

**问题**：`updateStatusBar` 依赖前端 JS 内存变量 `config.llmApiKey`，页面刷新后为 `null`。

**修复**：改为调用 `get_llm_config` 从后端实时查询，`WizardStateResponse` 新增 `llm_type` 和 `llm_model` 字段。

### 3.3 Sidecar 崩溃恢复使用旧 LLM 配置 — 中等

**状态**：✅ 已修复

**问题**：`recover_dead_instances` 使用 `SidecarHandle` 中保存的旧 `llm_api`，用户更新配置后恢复的 Sidecar 使用旧值。

**修复**：`recover_dead_instances` 接受 `fresh_llm_api` 参数，优先使用最新配置。

### 3.4 LLM 配置存在两套独立存储 — 中等

**状态**：⚠️ 部分修复（已同步，但仍是两套文件）

**两套存储**：
- `%APPDATA%\LoongRecall\wizard.json`（桌面端）
- `%APPDATA%\LoongRecall\config.json`（Sidecar 独立配置）

**当前方案**：通过 `POST /api/config/llm` 同步内存状态，但 `config.json` 和 `wizard.json` 依然独立。建议长期统一到单一数据源。

---

## 四、配置流程问题

### 4.1 缺少"重新配置 Agent MCP"入口 — 严重

**状态**：✅ 已修复

**问题**：用户无法在设置面板中重新配置 Agent MCP。唯一方式是"重置"清空所有配置。

**修复**：设置面板新增"🔄 重新检测并配置 Agent MCP"按钮。

### 4.2 快速路径（跳过检测）MCP 配置缺失 — 中等

**状态**：⚠️ 待修复

**问题**：用户点击"跳过检测"后，`configure_agents` 不会立即调用，仅在步骤 2 完成时补配。如果后台 Agent 检测失败或超时，MCP 配置完全缺失。

**位置**：`wizard.js` 第 294-318 行（`btn-step-1-skip` 事件处理）

**建议**：Agent 后台检测完成后立即调用 `configure_agents`。

### 4.3 MCP 配置结果无前端反馈 — 低

**状态**：⚠️ 待修复

**问题**：`configure_agents` 结果仅 `console.log`，用户无可见反馈。

**位置**：`wizard.js` 第 278-284 行、第 592-599 行

**建议**：将配置结果（成功/失败详情）展示给用户。

---

## 五、架构与代码质量问题

### 5.1 未使用的导入 — 低

**状态**：✅ 已修复

**问题**：`commands.rs` 中 `use tauri::Emitter;` 不再使用。

### 5.2 二进制命名链路复杂 — 低

**状态**：⚠️ 记录（不修复）

三个二进制名称共存：
| 名称 | 用途 | 定义位置 |
|---|---|---|
| `code-memory-server` | Rust 包二进制名 | `Cargo.toml` |
| `lrc-sidecar` | Tauri sidecar 名 | `tauri.conf.json` / `build.rs` |
| `lrc-sidecar` | MCP 配置命令名 | `agent_detector.rs`（已修复） |

`build.rs` 负责将 `code-memory-server.exe` 复制为 `lrc-sidecar.exe`。

---

## 六、修复统计

| 严重程度 | 总数 | 已修复 | 待修复 |
|----------|------|--------|--------|
| 严重 | 6 | 6 | 0 |
| 中等 | 5 | 3 | 2 |
| 低 | 2 | 1 | 1 |
| **合计** | **13** | **10** | **3** |

### 待修复项

1. **配置写入后无验证**（中等）— `write_or_merge_config` 添加回读验证
2. **快速路径 MCP 配置缺失**（中等）— `btn-step-1-skip` 中立即调用 `configure_agents`
3. **MCP 配置结果无前端反馈**（低）— 展示配置结果给用户

---

## 七、产品流程诊断

### 核心问题：用户旅程不清晰

1. **首次配置**：向导包含 Agent 检测、项目选择、MCP 配置，但零散的配置说明散落在 HTML 静态页面中
2. **已就绪面板**：状态显示正确，但缺乏"重新配置 Agent"入口
3. **设置面板**：之前只有静态 HTML 配置说明，现在新增了"重新配置"按钮
4. **LLM 配置**：三处显示不一致（仪表盘、状态栏、已就绪面板），现已修复

### 建议改进方向

1. **统一配置中心**：将 LLM、Agent MCP、项目配置集中到一个面板
2. **配置状态仪表盘**：实时显示各 Agent 的 MCP 连接状态
3. **配置验证流程**：写入后自动验证，失败时给出明确修复指引
4. **快速修复向导**：检测到配置问题时，一键修复