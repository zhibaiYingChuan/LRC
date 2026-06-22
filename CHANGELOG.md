# Changelog

所有重要变更记录。遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

---

## [0.5.6] - 2026-06-23

### 修复

#### 修复一：写回性能瓶颈（O(N²) → O(N)）

- **问题**：每次 `recall` 后全量重写所有记忆（`clear_memories()` + 循环 `save_memory()`），3633 条记忆时单次 recall 写回耗时 ~105s，严重阻碍大规模记忆检索
- **修复**：
  - 在 `Persistence` trait 增加 `update_memories` 批量更新方法（默认实现为循环 `save_memory`，推荐具体后端重写）
  - `JsonPersistence` 重写 `update_memories` 为单次序列化 + 单次磁盘写入，仅更新被检索到的记忆（通常 ≤ top_k=10 条）
  - `recall` 函数写回逻辑从全量重写改为增量批量更新
- **效果**：大规模记忆场景下 recall 写回从 ~105s 降至毫秒级，3633 条记忆场景性能提升 10000 倍+
- **涉及文件**：`src/persistence/mod.rs`、`src/persistence/json.rs`、`src/memory_store.rs`

#### 修复二：TF-IDF 词边界检测

- **问题**：TF-IDF 检索使用 `contains()` 子串匹配，导致 "cat" 错误匹配 "category"、"rust" 匹配 "frustrated" 等英文单词误匹配
- **修复**：
  - 新增 `contains_word` 和 `count_word_occurrences` 辅助函数
  - 对长度 ≥ 3 的英文单词做词边界检测（检查匹配位置前后字符是否为非字母字符）
  - CJK bigram 和 2 字符 ASCII bigram 保留 `contains()` 子串匹配（适配中文检索和短词匹配）
- **效果**：英文检索精度提升，避免子串误匹配，同时保持中文检索能力
- **涉及文件**：`src/memory_store.rs`

### 新增

- **公平性改革 — 基准测试从"测架构"转变为"测能力"**：
  - 改革核心：将"验证架构"（测有没有洛书编码/LLM翻译器）转变为"验证效果"（测能不能做到知识更新/模糊查询/双关词区分）
  - 公平原则：不利用 ground truth，所有文档 importance=5（统一），蓄水池抽样随机文档
  - LRC 原生基准公平版：TF-IDF 模式 11/11 PASS（总评分 0.94），LLM 模式 9/11 PASS（总评分 0.79）
  - LongMemEval 公平版 v3：Session Recall@10=85.74%（不利用 has_answer 差异化）

- **6 次基准测试完整报告**：
  - MS MARCO BEIR 测试：TF-IDF MRR@10=0.7749，LLM MRR@10=0.8895（LLM 增益 +14.8%）
  - Natural Questions BEIR 测试：TF-IDF MRR@10=0.5389，LLM MRR@10=0.8016（LLM 增益 +48.7%）
  - HotpotQA BEIR 测试：TF-IDF MRR@10=0.7964，LLM MRR@10=0.9383（LLM 增益 +17.8%）
  - FiQA BEIR 测试：TF-IDF MRR@10=0.2729，LLM MRR@10=0.4453（LLM 增益 +63.2%）
  - LRC 原生基准测试（公平版）：TF-IDF 11/11 PASS，总评分 0.94
  - LongMemEval 基准测试（公平版 v3）：Session Recall@10=85.74%，Turn Recall@10=61.70%

- **BEIR 基准测试评估脚本**：
  - MS MARCO 评估脚本（`lrc_msmarco_eval.py`）：500 文档，100 查询，支持 TF-IDF 和 LLM 两种模式
  - Natural Questions 评估脚本（`lrc_nq_eval.py`）：500 文档，100 查询，适配 NQ 数据集特征（title + text 文档内容）
  - HotpotQA 评估脚本（`lrc_hotpotqa_eval.py`）：500 文档，100 查询，适配多跳推理场景
  - FiQA 评估脚本（`lrc_fiqa_eval.py`）：500 文档，100 查询，含多字节字符处理（避免 panic）
  - 蓄水池抽样随机文档，跳过合成记忆，BEIR 标准指标（MRR@10, Recall@10, Hit Rate@10）

- **LRC 原生基准测试公平版脚本**（`lrc_native_benchmark.py`）：
  - 11 项测试覆盖三层模型：通用检索、高级记忆能力、综合能力与信任
  - 公平性改革：L2 和 L3 的 6 个测试函数全部重构，从"测架构"变为"测能力"
  - 支持 TF-IDF 和 LLM 两种模式

- **LongMemEval 公平版评估脚本**：
  - v1（`lrc_real_retrieval_eval.py`）：仅会话级注入，importance=5（公平）
  - v2（`lrc_real_retrieval_eval_v2.py`）：Turn 级注入 + has_answer=8（不公平，对比用）
  - v3（`lrc_fair_eval_v3.py`）：Turn 级注入 + 统一 importance=5（公平，推荐）

- **基准测试报告目录**（`benchmarks/reports/`）：
  - 7 份分项报告 + 1 份汇总对比报告
  - 完整的评估方法、结果、分析和使用建议

### 性能优化

- **大规模记忆检索性能释放**：v0.5.6 修复一使 LRC 能够高效处理 500+ 文档的检索场景
  - 500 文档场景下 TF-IDF 平均检索仅 13ms（MS MARCO）/ 18ms（NQ）/ 21ms（HotpotQA）/ 19ms（FiQA）
  - P99 检索耗时仅 27ms（MS MARCO）/ 32ms（NQ）/ 39ms（HotpotQA）/ 44ms（FiQA）
  - LongMemEval 470 实例评估 < 60 秒，平均检索 2.6ms/查询

### 变更

- `Cargo.toml` 版本号 0.5.4 → 0.5.6
- `desktop/src-tauri/Cargo.toml` 版本号 0.5.4 → 0.5.6
- `desktop/src-tauri/tauri.conf.json` 版本号 0.5.5 → 0.5.6
- `desktop/package.json` 版本号 0.5.4 → 0.5.6

### 测试

- 新增 6 个单元测试：
  - `test_update_memories_partial_update`：验证增量批量更新
  - `test_update_memories_empty`：验证空输入处理
  - `test_contains_word_english_boundary`：验证英文词边界检测
  - `test_contains_word_cjk_bigram`：验证 CJK bigram 子串匹配
  - `test_contains_word_short_ascii_bigram`：验证短 ASCII bigram 子串匹配
  - `test_count_word_occurrences_boundary`：验证词频统计的词边界检测
- 全项目 406 个单元测试全部通过，clippy 无警告

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