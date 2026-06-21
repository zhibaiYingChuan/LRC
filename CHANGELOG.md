# Changelog

所有重要变更记录。遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

---

## [0.5.5] - 2026-06-21

### 新增
- **MCP 配置自动升级**：Sidecar 启动时自动检测并升级旧版本 MCP 配置（stdio `loong-recall` → HTTP `lrc-memory`）
- **`auto_upgrade_configs()` 方法**：在 `agent_detector.rs` 中新增，sidecar 启动后自动调用
- **`config_needs_upgrade()` 方法**：检查配置是否包含旧的 stdio 模式配置项
- 用户升级 LRC Desktop 后无需重新运行配置向导，旧配置自动迁移

### 修复
- **MCP 工具不显示（"no tools yet"）**：根因是配置文件中是 stdio 模式 `loong-recall`，但 LRC Desktop 运行的是 HTTP sidecar。修复 `generate_config()` 始终生成 HTTP 模式配置；修复 `write_or_merge_config()` 清理旧配置名称
- **AI 主动调用 recall 未生效**：根因是 Trae 规则文件路径错误（`.trae/rules.md` → `.trae/rules/lrc-memory.md`）且缺少 `alwaysApply: true` frontmatter。修复 `get_rules_file_template()` 路径；添加 YAML frontmatter
- **AI 工具检测不准确**（检测出 9 个实际只有 2 个）：改进 `check_known_tool()` 检测策略，无 `binary_paths` 且无 `mcp_config_template` 的工具不自动检测
- **仪表盘"修改配置"按钮无反应**：移除只读卡片逻辑，统一使用完整 LLM 配置表单（多提供商选择）

### 变更
- `generate_config()` 始终生成 HTTP 模式配置（`type: "http"`, `url: "http://127.0.0.1:{port}/mcp"`）
- `write_or_merge_config()` 合并时自动清理旧配置名称（`loong-recall`, `lrc`, `lrc-memory-stdio`, `lrc-stdio`）
- `get_rules_file_template()` 路径更新为各 IDE 的标准规则文件路径
- `generate_ai_rules_content()` 添加 YAML frontmatter（Trae/Cursor）
- `write_ai_rules()` 清理旧路径规则文件，提取用户自定义内容并迁移
- `commands.rs` 在 `start_sidecar` 和 `start_sidecar_for_project` 中调用 `auto_upgrade_configs`

### 性能优化
- **关闭 `ml` feature 默认启用**：`default = ["server"]`，减少 sidecar 基线内存占用（candle 等重型依赖不再编译进二进制）
- **关闭后台结晶流水线 `run_on_start`**：延迟首次合成，避免启动内存峰值

### 安全
- 编译产物保密性确认：`Cargo.toml` 配置 `strip = true` + `lto = true` + `opt-level = "z"` + `codegen-units = 1` + `panic = "abort"`，符号信息已剥离

---

## [0.5.4] - 2026-06-20

### 新增
- 全项目静态代码审计报告
- 桌面端 URL 导航白名单验证（仅允许 127.0.0.1）
- 敏感数据使用后内存清零（SecureString 模式）
- 编译时与运行时反逆向工程保护增强（具体实现受 DaoTi Research License 保护）
- DPAPI 密钥损坏自动恢复机制

### 修复
- 修复所有 Clippy 警告（doc_lazy_continuation 等）
- 消除 tray.rs 中的 unwrap() 调用
- PostgresPersistence 新增 `block_on_async` 封装 tokio 运行时处理
- encoder_codebert::encode 返回 Result 类型，正确传播错误

---

## [0.5.1] - 2026-06-18

### 修复
- **P1-1**: server.rs 巨型函数拆分（964行 → 5个函数）
- **P1-2**: 模型加载逻辑重复（提取共享 PoolingStrategy 到 `src/engine/pooling.rs`）
- **P1-3**: RRF 融合逻辑重复（提取共享 `src/engine/rrf.rs` 模块）
- **P1-4**: synthesis_engine 循环内重复构建 HashSet
- **P1-5**: synthesis_engine 测试覆盖（14 个测试用例）
- **P1-6**: 速率限制器集成（AppStore 集成 + 关键命令保护）
- **P1-7**: SidecarManager Drop 等待退出（进程泄漏修复）
- **P1-8**: Agent 检测器扫描深度限制（MAX_SCAN_ENTRIES=200）
- **P2-1**: Dockerfile 缺少 static/ 复制
- **P2-2**: JSON 全量读写 O(n) 瓶颈（RwLock 内存缓存）
- **P2-3**: CI 仅 Windows runner（三平台矩阵策略）
- **P2-4**: 前端 CSS 内联 1260 行（提取到 `static/app.css`）
- **P2-5**: 前端 app.js 全局变量污染（IIFE 隔离）

### 变更
- 前端版本号一致性（统一从 Cargo.toml 读取）
- 新增 `src/engine/pooling.rs`、`src/engine/rrf.rs`、`static/app.css`

---

## [0.5.0] - 2026-06-17

### 新增
- 一键安装脚本（`scripts/install.ps1` / `scripts/install.sh`）
- v0.5.0 用户使用手册（`docs/v0.5.0_用户使用手册.md`）
- v0.5.0 开发者指南（`docs/v0.5.0_开发者指南.md`）
- v0.5.0 安全架构白皮书（`docs/v0.5.0_安全架构白皮书.md`）
- v0.5.0 综合修复与发布方案（`docs/v0.5.0_综合修复与发布方案.md`）
- 控制流平坦化反逆向工程支持
- 反内存 dump 保护（敏感数据使用后清零）
- DPAPI 密钥损坏自动恢复机制

### 修复
- **P0-1**: 多项目/多窗口/多IDE 隔离（Sidecar 进程管理 + 项目指纹）
- **P0-2**: wizard.js XSS 风险（HTML 转义 + CSP 头）
- **P0-3**: 密钥与密文同目录存储（AES-256-GCM + DPAPI 加密）
- **P0-4**: SHA-256 完整性校验（build.rs 编译时生成 + 运行时校验）
- **P0-5**: Qdrant 数据持久化（添加 collection 存在性检查）
- **P0-6**: 系统托盘面板（动态 tooltip + 项目切换菜单）
- **P0-7**: 前端版本号一致性（统一从 Cargo.toml 读取）
- **P1-1**: server.rs 巨型函数拆分（964行 → 5个函数）
- **P1-2**: 模型加载逻辑重复（提取共享 PoolingStrategy）
- **P1-3**: RRF 融合逻辑重复（提取共享 rrf.rs 模块）
- **P1-4**: synthesis_engine 循环内重复构建 HashSet
- **P1-5**: synthesis_engine 测试覆盖（14 个测试用例）
- **P1-6**: 速率限制器集成（AppStore 集成 + 关键命令保护）
- **P1-7**: SidecarManager Drop 等待退出（进程泄漏修复）
- **P1-8**: Agent 检测器扫描深度限制（MAX_SCAN_ENTRIES=200）
- **P2-1**: Dockerfile 缺少 static/ 复制
- **P2-2**: JSON 全量读写 O(n) 瓶颈（RwLock 内存缓存）
- **P2-3**: CI 仅 Windows runner（三平台矩阵策略）
- **P2-4**: 前端 CSS 内联 1260 行（提取到 app.css）
- **P2-5**: 前端 app.js 全局变量污染（IIFE 隔离）
- **T-01**: Neo4j subgraph 真正使用 Cypher 查询（可变长度路径 + 本地兜底）
- **T-11**: wizard.js 空 catch 块（添加错误日志和用户提示）
- **T-12**: desktop/commands.rs eval 安全加固（URL 白名单验证 + 单引号转义）
- DPAPI 密钥损坏自动恢复（解密失败时删除损坏文件并重新生成）

### 安全
- 桌面端 URL 导航白名单验证（仅允许 127.0.0.1）
- 敏感数据使用后内存清零（SecureString 模式）
- 字符串编译时混淆（obfstr）
- 代码签名文档（自签名 + EV 证书方案）

### 文档
- 新增 4 份 v0.5.0 文档（用户手册、开发者指南、安全白皮书、综合方案）
- 更新 README.md 版本号和安装方式
- 新增安装脚本（Windows + Linux/macOS）

---

## [0.4.0] - 2026-06-15

### 新增
- 洛书 9 维坐标编码器
- 镜像梯形递归算子
- 八卦分类投影
- 双重衰减模型（时间+拓扑双因子）
- 合成引擎（并查集聚类 + 洛书递归）
- Dao 自适应调节器（自愈系统）
- RRF 双路检索融合
- MCP 协议接口（13 个工具）
- REST v1 API（11 个端点）
- Web 仪表盘（Tauri 2 桌面端）
- 系统托盘集成
- JSON/PostgreSQL/Qdrant/Neo4j 多后端支持
- 多语言代码切分器（chunker）
- 审计追踪（audit_trail）
- 复杂度预算与红线检查
- 道枢演化协议
- 用户反馈回路
- 系统健康报告
- A/B 测试框架
- 基准测试框架

### 安全
- 反逆向防护（IsDebuggerPresent + CheckRemoteDebuggerPresent）
- 进程守护（process_guard）
- 数据加密（AES-256-GCM）
- 配置持久化

---

## 许可证说明

- **公开层** (L1): Apache 2.0 — `src/bin/`, `src/persistence/`, `src/chunker.rs`, `static/`, `desktop/`
- **引擎层** (L2): DaoTi Research License v1.0 — `src/engine/`