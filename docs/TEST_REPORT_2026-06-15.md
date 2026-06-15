# 桌面端新用户模拟测试报告

**日期**: 2026-06-15（原始）/ 2026-06-16（更新）  
**测试人**: AI Agent (工程文化教练监督)  
**测试环境**: Windows 10, Rust, pnpm 11.6.0  
**测试方法**: 六钥匙分析法 (Shannon Six Keys) + 工程文化教练目标驱动模式

---

## 测试目标

模拟新用户从仓库 `git clone` 后的完整首次体验链路：
1. 构建桌面端
2. 走通引导流程
3. 验证 Agent 自动检测
4. 验证 MCP 配置
5. 验证记忆功能

---

## 测试通过项

| 项目 | 状态 | 说明 |
|------|------|------|
| Rust 单元测试 | ✅ 398/398 + 28/28 通过 | 主项目 398 测试 + 桌面端 28 测试，0 失败 |
| `cargo build --release` | ✅ 成功 | 0 错误 0 警告 |
| `pnpm tauri build` | ✅ 成功 | 生成 MSI + NSIS 安装包，需先关闭 MCP 服务 |
| `--install-ide trae-cn` | ✅ 成功 | 正确写入配置到 ~/.trae-cn/trae-mcp.json |
| `--list-ides` | ✅ 新增 | 支持列出 14 种 IDE/工具 |
| MCP Server 启动 | ✅ 成功 | v0.4.0, --stdio 模式正常, 索引正常 |
| Sidecar 进程管理 | ✅ 正确 | 启动/健康检查/残留清理逻辑正确 |
| 前端 E2E 测试 | ✅ 新增 | wizard.test.js 覆盖向导流程 10+ 用例 |

---

## 发现的问题

### P1 (高优先级) — 全部已修复

#### P1-01: Sidecar 二进制版本不同步 ✅ 已修复
- **文件**: `desktop/src-tauri/build.rs`
- **问题**: 该文件是旧版本(7:09, 4,731,392 字节)，最新编译产物在 `G:/rust-target/release/` (17:27, 4,785,152 字节)
- **根因**: 全局 `~/.cargo/config.toml` 中 `target-dir = "G:/rust-target"` 导致编译产物不在默认的 `target/release/`
- **修复**: 在 `build.rs` 中添加 `sync_sidecar_binary()` 函数，自动从 `CARGO_TARGET_DIR` 或 workspace `target/release/` 复制最新二进制
- **修复日期**: 2026-06-16

#### P1-02: 构建时文件锁定冲突 ✅ 已修复
- **问题**: 当 MCP Server 正在运行时，`pnpm tauri build` 因 `code-memory-server.exe` 被锁定而失败 (os error 32)
- **修复**: 在 `build.rs` 中添加文件锁定检测，通过 `rename` 操作检测文件是否被占用，若被占用则输出清晰的中文提示和 `taskkill` 命令
- **修复日期**: 2026-06-16

#### P1-03: 前端 fallback Agent 列表与后端不匹配 ✅ 已修复
- **文件**: `desktop/src/wizard.js` 第 76-87 行
- **问题**: 当 Tauri invoke 不可用时，fallback 只列了 7 种工具
- **修复**: 移除硬编码 fallback 列表，改为显示"无法获取 AI 工具列表，请确认 LRC Desktop 后端服务已启动"提示
- **修复日期**: 2026-06-16

### P2 (中优先级) — 全部已修复

#### P2-04: 向导状态残留导致跳过引导 ✅ 已修复
- **文件**: `desktop/src-tauri/src/config_wizard.rs`
- **问题**: `setup_complete: true` 时桌面端直接跳到仪表盘
- **修复**: 添加 `config_version` 字段（当前版本 1），版本不匹配时自动重置配置（保留已加密的 API Key）
- **修复日期**: 2026-06-16

#### P2-05: configured_agents 未持久化 ✅ 已修复
- **文件**: `desktop/src-tauri/src/commands.rs`
- **问题**: `configured_agents` 为空数组 `[]`，配置完成后未回写
- **修复**: 在 `configure_agents` 命令中添加 `wizard.save_configured_agents(agent_ids)` 调用，确保持久化
- **修复日期**: 2026-06-16

#### P2-06: API Key 明文存储 ✅ 已修复
- **文件**: `src/config.rs` + `src/crypto.rs`
- **问题**: `llm_api` 以明文格式存储: `openai:sk-test-key-12345:gpt-4o-mini`
- **修复**: `config.json` 保存时自动加密 `llm_api` 到 `encrypted_api_key` 字段（AES-256-GCM），加载时自动解密恢复
- **修复日期**: 2026-06-16

### P3 (低优先级) — 全部已修复

#### P3-07: 缺少 --list-ides 命令 ✅ 已修复
- **文件**: `src/bin/server.rs`
- **问题**: 用户无法通过 CLI 查询支持的 IDE 列表
- **修复**: 添加 `--list-ides` 子命令 + `print_ides_list()` 函数 + `SUPPORTED_IDES` 数据库（14 种 IDE/工具）
- **修复日期**: 2026-06-16

#### P3-08: 编译警告: project_config_path 未使用 ✅ 已修复
- **文件**: `desktop/src-tauri/src/agent_detector.rs`
- **问题**: `AgentDetector` trait 中 `project_config_path` 方法未被任何地方调用
- **修复**: 从 trait 定义及 `TraeDetector`/`TraeCNDetector` 实现中移除该方法
- **修复日期**: 2026-06-16

#### P3-09: 项目扫描范围有限 ✅ 已修复
- **文件**: `desktop/src-tauri/src/agent_detector.rs` `scan_roots()` 函数
- **问题**: 项目扫描缺失 `C:\` 盘根目录
- **修复**: 在 `scan_roots()` 驱动器扫描列表中添加 `C:\`
- **修复日期**: 2026-06-16

---

## 六钥匙回顾

### Simplify: 核心链路已验证
`git clone → build → 启动 → 向导 → 配置 → 完成` 链路可走通

### Decompose: 各模块独立测试
- Rust 后端: 398/398 + 28/28 测试全部通过
- 前端向导: 3 步流程正确 + E2E 测试覆盖
- Agent 检测: 30+ 工具数据库完备
- MCP 配置: 写入/合并逻辑正确
- `--list-ides`: 14 种 IDE/工具可查询

### Generalize: 共性问题（已解决）
- ~~路径不一致问题（target-dir、sidecar 同步）~~ → build.rs 自动同步
- ~~状态持久化不完整（空 configured_agents）~~ → commands.rs 持久化
- ~~前端/后端数据不同步（fallback 7 vs 30+）~~ → 移除硬编码改为动态检测

### Proliferate: 反向推导（已缓解）
新用户从下载到正常使用的体验中，主要障碍已修复：
1. ~~构建时需要关闭 MCP 服务（P1-02）~~ → build.rs 锁定检测 + 提示
2. ~~前端 fallback 显示不完整（P1-03）~~ → 动态检测 + 连接状态提示
3. ~~旧状态残留（P2-04）~~ → config_version 版本迁移

---

## 工程文化教练评估

### 契约优先
- ✅ `AgentDetector` trait 定义了清晰的契约接口
- ✅ ~~`project_config_path` 方法定义了但未使用~~ → 已移除，契约一致

### 测试驱动
- ✅ 398+28 个 Rust 单元测试覆盖核心逻辑
- ✅ ~~前端无任何测试~~ → 已添加 wizard.test.js E2E 测试套件

### 安全第一
- ✅ API Key 加密存储（L1 AES-256-GCM）
- ✅ ~~config.json 中仍有明文 API Key~~ → 已实现加密存储，保存时自动加密

### 文档即代码
- ✅ Rust 代码有良好的文档注释
- ⚠️ 前端代码注释较少（非阻塞，可在后续迭代中改进）

---

## 建议的产品经理评估事项

所有 P0/P1/P2/P3 问题已修复，建议产品经理关注：
1. **新用户全链路测试**: 安排真实用户执行 `git clone → build → 向导 → 配置` 流程并进行可用性测试
2. **安装包分发**: 考虑提供预编译 MSI/NSIS 安装包，降低新用户构建门槛
3. **前端代码注释**: 在后续迭代中为 wizard.js, index.html 添加中文注释
4. **CI/CD 自动化**: 建议在 CI 中自动运行 `cargo test` 和 `cargo clippy`，确保每次提交质量
5. **跨平台测试**: 当前仅在 Windows 10 测试，建议补充 macOS/Linux 测试