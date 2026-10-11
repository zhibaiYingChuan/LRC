# Loong Recall (LRC) 全局全维度代码审查报告 · v0.9.11 基线

> 审查对象：`g:\code-memory`（Rust crate `code-memory`，`Cargo.toml` version = `0.9.10`，工作区含未发布的 v0.9.11 改动）
> 审查日期：2026-10-07
> 审查方法：**七维度并行只读审查**（架构与代码质量 / 安全与隐私 / 并发与韧性 / 持久化与一致性 / 测试与门禁 / 前端与文档一致性），
> 并由主控对**全部 P0 与部分 P1** 独立复核（复核结果标注在每条风险上）
> 数据口径：所有数字来自实际文件读取或 Grep/命令计数；每条风险附【文件:行号】级证据。**无法核实的一律标注"未核实"**
> 上一版基线：[`GLOBAL_CODE_REVIEW_REPORT.md`](./GLOBAL_CODE_REVIEW_REPORT.md)（v0.9.7 @ `8fa1692`，含 f1–f10 / r1–r10 / b1–b18 / c1–c10 共十轮修复记录）
>
> **本文件用途**：作为**后续推进的基线文档**。第四节风险清单 + 第五节推进路线 + 第十节跟踪清单是行动入口；
> 风险条目一律用 `CR-xx` 编号，修复后在第十节回写状态。

---

## 目录

- [一、项目概况（实测）](#一项目概况实测)
- [二、审查方法与范围](#二审查方法与范围)
- [三、七维度审查结果](#三七维度审查结果)
- [四、风险清单汇总（P0–P3）](#四风险清单汇总p0p3)
- [五、推进路线（三梯队）](#五推进路线三梯队)
- [六、本轮审查过程中已当场修复的项](#六本轮审查过程中已当场修复的项)
- [七、已核实为「非问题」的项（防重复报）](#七已核实为非问题的项防重复报)
- [八、未核实项（诚实声明）](#八未核实项诚实声明)
- [九、结论](#九结论)
- [十、跟踪清单（后续按此推进）](#十跟踪清单后续按此推进)

---

## 一、项目概况（实测）

### 1.1 规模指标（本次实测，口径：`Get-ChildItem -Recurse` + 逐文件行计数）

| 分区 | 文件数 | 行数 |
|---|---|---|
| `src/*.rs` | **71** | **86,572** |
| `desktop/src-tauri/src/*.rs` | 11 | 12,576 |
| `tests/*.rs`（集成测试） | 8 | 2,751 |
| `static/*.js`（`app.js` 单文件） | 1 | 13,801 |
| `static/*.css` | 3 | 6,596 |
| `static/*.html` | 2 | 2,659 |
| **Rust 合计** | **82** | **约 99,100** |
| **前端合计** | 6 | **约 23,100** |

**测试/生产比**：`src` 内测试代码约 28.8k 行（占 `src` 约 33%），`src` 生产代码约 57.7k 行 ⇒ 测试:生产 ≈ **0.5:1**，测试体量健康。

**最大源文件 Top 8**：

| 行数 | 文件 | 性质 |
|---|---|---|
| 7,237 | `src/memory_store.rs` | 生产（聚合根） |
| 6,228 | `src/memory_store_tests.rs` | 测试 |
| 6,173 | `src/server.rs` | 生产（MCP + HTTP） |
| 5,967 | `src/v1_api_tests.rs` | 测试 |
| 5,768 | `src/v1_api.rs` | 生产（52 条路由） |
| 3,065 | `src/engine/dao_regulator.rs` | 生产（Layer 2） |
| 2,676 | `src/engine/audit_trail.rs` | 生产（Layer 2） |
| 2,402 | `src/state_matcher.rs` | 生产 |

### 1.2 卫生指标（实测）

| 指标 | 实测 | 说明 |
|---|---|---|
| `.unwrap()` | 542 | **绝大多数落在 `#[cfg(test)]`**（`v1_api_tests` 241、`memory_store_tests` 57）；已逐文件核对测试边界 |
| `.expect(` | 820 | 同上（`memory_store_tests` 429、`persistence/json.rs` 97） |
| `panic!(` | 26 | 主要 `v1_api_tests` 10、`engine/llm_translator.rs` 6 |
| `unsafe` | 34 处匹配（真实块集中于 `crypto.rs`/`process_guard.rs`/`guard.rs`/`tray.rs`） | 均为 Windows FFI，逐块带 `SAFETY` 说明 |
| `#[allow(dead_code)]` | 17 | 多处带"移除实验已实测非冗余"的核实注释 |
| `TODO/FIXME/XXX/HACK` | **0 处真实标记** | grep 命中项均为变量名 `todo` 或文档占位符 `sk-xxx` |

**结论：卫生面良好**（无 TODO 债、panic 面集中在测试、unsafe 有说明）。

### 1.3 构建与发布配置

| 项 | 值 | 证据 |
|---|---|---|
| version | `0.9.10` | [Cargo.toml:7](file:///g:/code-memory/Cargo.toml#L7) |
| 默认 feature | `default = ["server"]` | `Cargo.toml` |
| release profile | `opt-level="z"` + `lto=true` + `codegen-units=1` + **`panic="abort"`** + `strip=true` + **`overflow-checks=true`** | [Cargo.toml:199-206](file:///g:/code-memory/Cargo.toml#L199-L206) |
| CI 门禁（关键） | `cargo clippy --features server -- -D warnings`、`cargo clippy --all-targets -- -D warnings` | [ci.yml:123-126](file:///g:/code-memory/.github/workflows/ci.yml#L123-L126) |
| CI 测试口径 | `-- --test-threads=1` | `.github/workflows/ci.yml` |

> ⚠️ `panic="abort"` 与"仅发 `--features server`（非 ml）二进制"两条组合推出 CR-04，务必先读。

---

## 二、审查方法与范围

### 2.1 七维度模型

| 维度 | 审查重点 |
|---|---|
| 一、架构与模块组织 | 分层与依赖方向、God Object、feature 矩阵、trait 抽象 |
| 二、代码质量 | `unwrap/expect` 生产面、重复代码、注释漂移、`#[allow]` |
| 三、安全与隐私 | **隐私隔离红线**、认证鉴权、`unsafe`、SSRF/路径遍历、加密、供应链 |
| 四、并发与韧性 | 锁与超时矩阵、**取消真实性**、超时阶梯、五层交互韧性、资源与 panic 面 |
| 五、持久化与一致性 | **字段白名单逐后端比对**、原子性与 fsync、全量重写、缓存一致性、迁移备份 |
| 六、测试与门禁 | 测试规模、**恒真风险**、**并行/环境变量竞态**、CI 口径、覆盖盲区 |
| 七、前端与文档一致性 | 前后端契约、文档漂移、桌面端一致性、许可证边界、文档库存量 |

### 2.2 审查边界与规则

- **只读**：本次审查未修改任何业务文件（例外见第六节——审查中发现的两处问题已当场修复，均已注明）。
- **主控复核**：所有 P0 与部分 P1 由主控用 `Read`/`Grep` 独立复核行号，复核结论标注为**「主控已复核」**；其余标注为**「维度审查」**（来自子审查，主控未逐条复核）。
- **不与上一版重复**：v0.9.7 报告的 P0 若已验证修复，不再列出（如"clear+save 丢全库"、`audit` 清除后复活、`--mode smart` 死参数等，见第七节）。

---

## 三、七维度审查结果

### 维度一/二：架构与代码质量

**总体**：分层清晰（`lib.rs` 文件级声明 Layer1/2/3），但存在**反向依赖成环**与 **God Object** 两类结构性问题；无 TODO 债、注释质量高。

| ID | 结论 | 证据 | 核验 |
|---|---|---|---|
| CR-15 | **`engine/`（Layer 2）反向依赖 Layer 1**，使两层成环，`engine/` 无法独立授权/分发；这是 v0.9.7 已修 P0-2 的**镜像漏网** | `engine/synthesis_engine.rs` 引 `graph_store`/`persistence`；`engine/rrf.rs`、`engine/navigation.rs` 引 `memory_store::RecallResult`；分层声明见 `src/lib.rs` | 维度审查 |
| CR-22 | **God Object**：`memory_store.rs` 7,237 行 / 152 fn / **22 字段（15 个 `pub`）**，外部直接改写 `pub` 字段（如 `consolidation.rs` 写 `store.synthesis_min_cluster`） | `src/memory_store.rs:751-809` | 维度审查 |
| CR-23 | **锁获取模式重复 11 处、两套语义并存**：`lock_store_with_timeout`（async）与手写 `try_lock` 2s 自旋（`spawn_blocking` 内）并行维护；`consolidate` 与 `remember` 的取锁块几乎逐字相同 | `v1_api.rs:122-136`（helper）vs `v1_api.rs:1917-1926`、`4533-4542` | 维度审查 |
| CR-24 | **合成阈值 `3`/`0.4` 无单一真源**：13 处各自硬编码（8 个文件），前台 `/consolidate` 与后台结晶可能静默分歧 | `engine/synthesis_engine.rs:139-140`、`memory_store.rs:1609-1662`、`consolidation.rs:93`、`bin/server.rs:1004-1005` 等 | 维度审查 |
| CR-25 | `MemoryStore::new()` 与 `new_statistical()` 近 40 行逐字重复（后者应复用前者再覆写单字段，同文件 `new_with_encoder` 已示范） | `src/memory_store.rs:1606-1643` vs `1646-1686` | 维度审查 |
| CR-26 | 写入端点契约不一致：`consolidate` 用 `source_timestamp_ms`(i64)，`remember` 用 `created_at`(RFC3339)；类型解析混用 `from_str`/`try_parse` | `v1_api.rs:1947-1953` vs `4551-4572`、`4518` vs `1929` | 维度审查 |
| CR-27 | feature 矩阵以**同一类型别名**承载两套 API（`HybridLuoShuEncoder` 在 ml/非 ml 下是不同类型、方法集不同），语义差异只能在编译期暴露 | `src/memory_store.rs:50-54`、`2067/2072`、`2078/2083` | 维度审查 |
| CR-28 | 生产路径残留跨函数不变量式 `expect`/`unreachable!`（上游过滤被重构即变 panic） | `v1_api.rs:1355`、`engine/synthesis_engine.rs:121-122`、`discovery.rs:412` | 维度审查 |
| CR-29 | 前端单文件 God Object：`static/app.js` 13,801 行；端口 fallback 未复用已存在常量 `STABLE_DEFAULT_PORT` | `static/app.js:73` vs `:6675` | 维度审查 |
| CR-30 | 函数级 `#[allow(unused_variables, unused_assignments)]` 掩盖"`--mode` 在非 ml 构建下静默无效" | `src/bin/server.rs:98/112`（唯一读取点在 ml 分支 `:716`） | 维度审查 |

### 维度三：安全与隐私

**总体**：加密与外部输入面（SSRF / 路径遍历 / CORS / unsafe）**质量高、未见 soundness 问题**；但**隐私隔离模型是 fail-open**，且 MCP 通道**完全未接入**隐私过滤 —— 这是本次最严重的发现。

| ID | 严重级 | 结论 | 证据 | 核验 |
|---|---|---|---|---|
| **CR-02** | **P0** | **MCP 工具路径硬编码 `privacy_context: None`**（`recall` / `recall_enhanced` / `list_memories` 共 5 处），而 MCP `remember` **允许**写入 `privacy_level`/`session_id`/`user_id` ⇒ **写进去的私有标记，读时被完全忽略**，任意能到达 `/mcp` 的调用者都能读到全部记忆正文 | [server.rs:2006](file:///g:/code-memory/src/server.rs#L2000-L2010)（另 919/950/966/2394）；`is_visible` 对 `None` 判定全可见见 `memory_store.rs:1403-1429` | **主控已复核** |
| **CR-01** | **P0** | **HTTP 只读端点 fail-open**：`/v1/memories/list` 构造 `ListFilter` 时**不设 `privacy_context`** 且请求体**根本没有 `user_id` 字段**，却返回**完整 `content`**；同款还包括 `recent`、`stored-edges`、`consolidate` 内部列取；`enrich` 是"缺 `user_id` 即 `None`" | [v1_api.rs:4339-4347](file:///g:/code-memory/src/v1_api.rs#L4339-L4347)（返回全文 `:4356`）；`MemoryListRequest` 无 `user_id`（`v1_api.rs:1539-1548`） | **主控已复核** |
| CR-03 | P1 | `associations` / `association-graph` **完全无隐私参数**（`MemoryAssociationsRequest`/`MemoryGraphRequest` 无 `user_id`，内部函数无 `is_visible`），返回 `content_preview` 与关系拓扑 | `memory_store.rs:5689-5706`、`6973-7009`；`v1_api.rs:4697/4768` | 维度审查 |
| CR-04 | P1 | **跨用户/跨会话聚合**：合成聚类只做过期与 `Synthesis` 过滤，产出的合成记忆**不带来源归属**（`privacy_level` 默认 `User`、`user_id=None`）；建议区（`build_association_suggestions`）用无隐私过滤的 `ListFilter` 且把记忆正文片段当"示例 chip"对外展示 ⇒ 私有内容经**二次通道**泄露 | `engine/synthesis_engine.rs:268/627/724-731`、`consolidation.rs:690-703`、`v1_api.rs:1310-1313`、`1350-1358` | 维度审查 |
| CR-19 | P1 | **认证默认放行**：仅当 `LRC_API_TOKEN` 非空才校验 Bearer；绑定非回环**仅告警不阻断**；且 `user_id` 完全由请求体自报 ⇒ "用户级隔离"可被冒充绕过。（token 比较已修为常量时间，该子项已修复） | `server.rs:4681-4711`、`4722-4743`；`v1_api.rs:2106` | 维度审查 |
| CR-31 | P2 | CI 供应链：`actions/setup-python@v5` **未按 SHA 固定**（其余 action 均已钉 SHA），与 `SECURITY.md` 声明不符 | `ci.yml:80`；`SECURITY.md:37` | 维度审查 |
| CR-32 | P3 | 未显式设置请求体大小上限（依赖 axum `Json` 默认 2MB），属隐式契约 | `v1_api.rs` / `server.rs` 无 `DefaultBodyLimit` | 维度审查 |
| CR-33 | P3 | 桌面端 `desktop/src-tauri/src/crypto.rs` 与主 crate `src/crypto.rs` 是**逐字复制的 DPAPI/AES-GCM 实现**（各约 100 行），且**已经漂移**——主 crate 已把解密失败文案改为不可区分文案，桌面端仍保留"密钥可能来自其他用户或机器"（可推断密钥来源）；两端共享同一密钥文件 ⇒ 未来加固可能再只改一半 | `desktop/src-tauri/src/crypto.rs:133` vs `src/crypto.rs:152-159` | 维度审查 |

### 维度四：并发 / 异步与韧性

**总体**：v1 API 路径的「锁超时 + `spawn_blocking` + 协作式取消」体系**设计良好且注释自证**；豁口集中在**超时阶梯被打破**、**MCP 路径未接入该体系**、**`panic=abort` 使隔离失效**三处。

| ID | 严重级 | 结论 | 证据 | 核验 |
|---|---|---|---|---|
| **CR-05** | **P0** | **`consolidate` 内层 60s ≥ 外层 30s ⇒ 协作式取消不可达**：唯一置位 `cancel` 的分支在 `timeout(60s)` 的 `Err` 臂，而路由 `TimeoutLayer` 30s 先丢弃 future ⇒ 标志恒 `false`，O(n²) 聚类**空跑到底** | 内层 [v1_api.rs:1991-1992](file:///g:/code-memory/src/v1_api.rs#L1991-L1992)；外层 [server.rs:4863-4866](file:///g:/code-memory/src/server.rs#L4863-L4866) | **主控已复核** |
| **CR-06** | **P0** | **`panic="abort"` 使全部 panic 隔离失效**：release 产物下 `catch_unwind` 与 `JoinError` 分支（"服务已保持运行"）**形同虚设**；阻塞线程内任一 `panic!`/`unwrap`/整数溢出（`overflow-checks=true`）都会 **abort 整个进程**，且 `Drop` 不执行（锁文件残留，靠 PID 自检兜底） | [Cargo.toml:199-206](file:///g:/code-memory/Cargo.toml#L199-L206)（`panic="abort"` @203、`overflow-checks` @206）；隔离话术见 `server.rs:111`、`v1_api.rs:2460/2635` | **主控已复核** |
| CR-10 | P1 | **后台结晶 Phase1 持锁做编码 + 全量 JSON 重写，且不查 `cancel`**：Phase1 循环（每条 `remember`）是最慢段，而 120s 兜底超时的 cancel 标志在该段**从未被读取** ⇒ 与用户请求并发时直接造成前台 2s 取锁超时（503 的历史根因面） | `consolidation.rs:357-395`、`310-320` | 维度审查 |
| CR-11 | P1 | **MCP 路径完全未接入 v1 的锁/超时体系**：多个 handler 用**无超时** `lock().await`，并在 async worker 上执行同步重活（`store.recall`、`trapezoid_focus_recall`）；无 `store_busy` 快速失败 | `server.rs:2052`、`3200`、`2284/2397/2571/2655/2695/2737/2820/2888/2993` | 维度审查 |
| CR-12 | P1 | **`audit.jsonl` 启动重放 O(n²)**：逐行 `self.events.insert(0, event)`（`Vec` 头部插入），`truncate` 在循环之后；随后 `migrate_legacy_events` 又二次全量读 + 全量重算哈希 + 全量重写 ⇒ **与历史"启动卡 13 分钟"直接因果吻合** | `engine/audit_trail.rs:897`、`931-993` | 维度审查 |
| CR-13 | P1 | **`/v1/associations/confirm` 无界自旋**：`try_lock` 轮询 50ms，**无 deadline / 无 cancel / 无外层 timeout** ⇒ 锁被长任务持有时该阻塞线程**永不返回**（与仓库其它所有取锁点"统一 2s deadline"明显不一致） | `v1_api.rs:2726-2731` | 维度审查 |
| CR-14 | P1 | `/v1/benchmarks/report` 内层 90s > 外层 30s，且无取消机制 | `v1_api.rs:5512-5516` | 维度审查 |
| CR-34 | P2 | `/v1/associations/explore` 取锁循环无独立 deadline（与 `enrich` 的 2s 不一致）；且 10s 内部预算从 `run_association_explore` **入口**起算，锁等待不计入 ⇒ "优雅收敛"在锁忙时不可达 | `v1_api.rs:2573-2583`、`713-718` | 维度审查 |
| CR-35 | P2 | **内存峰值叠加**：整库常驻（`memory_cache`）+ 每次读 `snapshot()` 全量深拷贝 + `synthesis_snapshot()` 再克隆 + 写路径 `previous.clone()` + 序列化字符串 ⇒ 峰值可达 **3–4× 库大小** | `memory_store_cache.rs:94-96`、`persistence/json.rs:270/374/420-427` | 维度审查 |
| CR-36 | P3 | explore 以 `limit=usize::MAX` 在持锁内全量列出 + 克隆记忆 | `v1_api.rs:718-724` | 维度审查 |

**五层交互韧性审计（L1–L5）**：L1 页面级与 L3 卡片级**覆盖良好**（独立 fetch + `try/catch` + `lock_busy` 降级 + 重试）；缺口为 **L2/L4**（`confirm` 弹窗对应后端无超时，见 CR-13）与 **L5**（`panic=abort` 使进程级异常无隔离，见 CR-06）。

### 维度五：数据持久化与一致性

**总体**：**默认 JSON 后端无字段白名单丢失**，历史 C05「clear+save 丢全库」与「审计清除后复活」**均已修复**；剩余风险集中在**可选后端的字段白名单**与**原子写持久性**。

| ID | 严重级 | 结论 | 证据 | 核验 |
|---|---|---|---|---|
| **CR-07** | **P0（限 postgres feature）** | **`ON CONFLICT DO UPDATE SET` 只更新 10/24 列**：INSERT 列出 24 列，但冲突更新只覆盖 `content/memory_type/importance/version/updated_at/last_accessed/luoshu_vector/event_id/entities/topological_depth` ⇒ 更新既有记忆时 `project/tags/created_at/ttl_days/bagua_index/bagua_category/privacy_level/session_id/user_id/source/source_ids/confidence/version_history` 的变更**静默丢弃**（命中本项目历史上"字段白名单漏字段"的故障模式） | [postgres.rs:484-500](file:///g:/code-memory/src/persistence/postgres.rs#L484-L500) | **主控已复核** |
| CR-08 | P0（限 postgres） | `information_gain` / `resolution` / `daoti_preview_{gua,bagua,version}` **既不在 DDL、也不在读写路径** ⇒ 合成质量门控与道体预判元数据在读回后丢失 | `postgres.rs:161-189`、`260-294`、`346-373` | 维度审查 |
| CR-17 | P1（限 qdrant） | **Qdrant 时间语义失真**：payload 不含 `created_at/updated_at/last_accessed`，读回**无条件重置为 `Utc::now()`**；另缺 ttl/source/source_ids/confidence/information_gain/session_id/user_id/version/version_history ⇒ 每次重载"记忆变新"，衰减与事件顺序推理全部失真 | `qdrant.rs:442-459`、`359-376` | 维度审查 |
| CR-18 | P1（限 qdrant） | Qdrant `fallback_memories` 仅进程内、无落盘：不可用时"写成功"、重启即丢 | `qdrant.rs:154`、`423-432` | 维度审查 |
| CR-37 | P1 | **备份与实时写不互斥**：`create_backup` 持 `BACKUP_OPERATION_LOCK` 逐文件 `fs::copy`，但写路径不取该锁 ⇒ 快照内 `memories.json` 与 `audit.jsonl` 取自不同时刻，可能含悬空审计引用 | `backup.rs:53/136-181` vs `persistence/json.rs:252-298` | 维度审查 |
| CR-38 | P1 | `import_memories` 对三个文件 read-modify-write + 直接 `fs::write` 覆盖，**无锁、无原子替换** | `export.rs:285-337` | 维度审查 |
| CR-39 | P1（限 neo4j） | `create_memory_node` 仅写 6 个属性，节点无法承载其余字段 ⇒ 图库无法作为记忆权威源重建（若为有意设计，应在文档显式声明） | `neo4j.rs:304-329` | 维度审查 |
| CR-40 | P2 | **原子写无 fsync**：`tmp + rename` 提供弱原子性，但不保证持久性 ⇒ 掉电可能得到空/截断文件，`serde_json` 解析失败 ⇒ **全库不可读** | `atomic_file.rs:38-53`、`persistence/json.rs:590-605` | 维度审查 |
| CR-41 | P2 | 单条写入**全量重写 JSON**（`O(N²)` 写放大）；批量路径已缓解，无分片/增量设计（JSON 后端声明定位"<10,000 条"）。量化：4,485 条规模下序列化 mean ≈ 308ms、p95 ≈ 324ms（引用仓库内既有剖析记录，**本次未重跑**） | `persistence/json.rs:279-288` | 维度审查 |
| CR-42 | P2 | `MemoryStore` 缓存跨进程不失效：仅自身 `dirty` 时重载，无 mtime/版本戳 ⇒ 多窗口桌面端 / CLI 与 sidecar 并存时可能无限期读陈旧数据 | `memory_store.rs:2097-2114` | 维度审查 |
| CR-43 | P2 | `#[serde(default)]` 兜底掩盖缺失（`version` 缺省为 0 而非 1、`resolution` 默认 `detailed`）⇒ 无法区分"真默认"与"被丢失" | `memory_types.rs:270-334` | 维度审查 |
| CR-44 | P3 | `persist_hidden` 非原子写（崩溃丢部分隐藏 ID，部分记录重启后重新可见）；`Memory` 无 `schema_version`；`synthesis_journal` 纯内存、`snapshot()` 三锁非原子；Postgres DDL `DEFAULT '{{}}'` 笔误；`export.exported_at` 语义与注释不符 | `audit_trail.rs:1333-1348`、`memory_types.rs:239`、`synthesis_journal.rs:55-60/272-294`、`postgres.rs:184/186`、`export.rs:162-168` | 维度审查 |

### 维度六：测试与质量门禁

**总体**：**鉴别力显著高于同类项目**（直读磁盘防假绿、负向对照、门控配对用例、`--test-threads=1` 的历史教训已内化）；系统性问题集中在**进程级环境变量竞态的"按模块局部修"**、**三处门禁口径不一致**、**备份/恢复/迁移 0 覆盖**。

| ID | 严重级 | 结论 | 证据 | 核验 |
|---|---|---|---|---|
| **CR-20** | **P1（本轮已部分修复）** | **进程级环境变量竞态**：刚修的 `LRC_STATE_BIAS` 用的是**模块级**锁，而 `memory_store_tests` 改同一变量**不持该锁**（跨模块竞态仍在）；另有 5 组裸奔：`LRC_DAOTI_PREVIEW_PRUNE`、`LRC_DOMAIN_CANDIDATE_INDEX`（**生产每调用实时读**，会污染同进程其它 recall 用例）、`LRC_ACTIVE_DISCOVERY`、`LRC_LLM_API`、`LRC_ASSOC_EDGE_FEEDBACK`/`LRC_ASSOC_PATH_SCORE` | `memory_state_machine.rs`（模块级锁）、`memory_store_tests.rs:3641-3642`、`3739/3774/3799-3808`、`4453/4490/4516`、`discovery.rs:1072/1118-1120`、`process_guard.rs:998/1008/1014-1029`、`v1_api_tests.rs:4182/4192/4311/4321` | 维度审查 |
| CR-21 | P1 | **门禁口径不一致**：CI 用 `--test-threads=1` **掩盖**竞态，而 `.githooks/pre-commit` 与 `release.yml` preflight 均**并行** ⇒ 竞态只在 CI 之外暴露（会造成"CI 全绿却发布被偶发阻塞"） | `ci.yml` vs `.githooks/pre-commit:85`、`release.yml:134` | 维度审查 |
| CR-45 | P1 | **备份 / 恢复 / 迁移 0 测试覆盖**：`/v1/backup`、`/v1/backup/restore`、`/v1/backups`、`/v1/migrate`、`/v1/data-logs` 在 `v1_api_tests.rs` 中 **0 命中**；`create_backup`/`restore_backup`/`execute_migration`/`scan_legacy_sources` 亦无用例 ⇒ **数据安全的最后防线零回归保护** | Grep 举证 0 命中 | 维度审查 |
| CR-46 | P2 | rustc 版本漂移：CI 用滚动 `stable`，本地为 1.99 ⇒ 历史故障模式（clippy 新 lint 命中）可复现；建议钉版本 | `ci.yml` | 维度审查 |
| CR-47 | P2 | **覆盖假象**：`tests/` 下 5 个 ML 不变量套件"无模型即静默 no-op"（CI 恒绿）；前端 6 套件 CI 只跑 1 套（`playwright-smoke.js`），`symbolic-layer-desktop-cdp.js` 连语法检查都无 | `tests/state_semantic_*.rs`、`ci.yml:705` | 维度审查 |
| CR-48 | P2 | 持久化后端测试盲区：`qdrant.rs`/`neo4j.rs` **0 测试**，`postgres.rs` 仅 1；CI 也不跑这些 feature ⇒ CR-07/CR-08/CR-17 这类白名单缺陷**无门禁可拦** | Grep 举证 | 维度审查 |
| CR-49 | P3 | 静态资源处理器用例**只断言状态码**（注释声称校验 SVG 内容）⇒ 返回空体仍绿 | `server.rs:6588-6593`、`6617-6622` | 维度审查 |

### 维度七：前端与文档一致性

**总体**：**前后端路径/方法契约无错配**（脚本实测 42 条路径 + 33 个 invoke 命令全对齐）；问题集中在**响应字段层无门禁**与**文档版本漂移**。

| ID | 严重级 | 结论 | 证据 | 核验 |
|---|---|---|---|---|
| CR-50 | P1 | 契约脚本**只对齐"路径 + 方法 + invoke 名"，不校验响应字段/schema** ⇒ `body.memories` vs `body.data` 这类漂移在 CI 中不可见（历史上已多次发生） | `scripts/validate_frontend_contract.js`（仅比对 path/method） | 维度审查 |
| CR-51 | P2 | **文档版本漂移**：`README.md` 与 `docs/USER_GUIDE.md` 仍宣称 **v0.9.9**（含下载链接）；`Cargo.toml` = 0.9.10 而 `CHANGELOG` 最新为 **[0.9.11] 未发布** ⇒ 三处不一致 | `README.md:250-253/441`、`USER_GUIDE.md:5/49-53`、`Cargo.toml:7`、`CHANGELOG.md:5` | 维度审查 |
| CR-52 | P2 | **许可证边界四者不重合**：`LICENSE` §2.2 只举 3 个文件，`lib.rs` 声明 Layer1 含 `v1_api.rs`/`memory_store.rs` 等十余个，而 `check_algorithm_leak.py` **仅扫 4 个文件** ⇒ "公开层"口径不透明（`v1_api.rs` 自称 Apache 却因含受保护术语被排除出扫描） | `LICENSE:57`、`src/lib.rs:126`、`scripts/check_algorithm_leak.py:39-65` | 维度审查 |
| CR-53 | P2 | `SECURITY.md` 与实现不符两处：声称"所有 Actions 均钉 SHA"（见 CR-31）；"支持的版本"表只到 v0.8.x | `SECURITY.md:37`、`SECURITY.md:5-9` | 维度审查 |
| CR-54 | P3 | README 内部措辞冲突：`README.md:453-458` 已修正为"engine 源码随包分发"，但 `README.md:264` 与 `USER_GUIDE.md:57` 仍写"道体推演引擎不随产品分发" | 同上 | 维度审查 |
| CR-55 | P3 | `CHANGELOG.md` **重复版本头**（3×`[0.9.8]`、2×`[0.8.23]`）；README 语义底座写 `bge-base-zh/768` 与默认 `bge-small-zh/512` 不符 | `CHANGELOG.md:130/150/186/976/1116`、`README.md:88`、`model_ids.rs:22` | 维度审查 |
| CR-56 | P3 | `OFFLINE_MODEL_GUIDE.md` 内部矛盾（`LRC_LUOSHU_MODEL_ID` vs `LRC_MODEL_ID` 并列误导）+ `cargo run --features server,ml --` 因双 bin 无 `default-run` **必失败** | `OFFLINE_MODEL_GUIDE.md:7/160/188` | 维度审查 |
| CR-57 | P3 | `#[crystallization-timeline]` 容器已删 → `loadCrystallizationHistory` 成死代码；`index.html` 注释行号漂移 | `static/app.js:8479`、`static/index.html:256-259` | 维度审查 |
| CR-58 | P3 | `docs/` 存量 **131 个 md**（远超"28 个"的估计），历史审计/修复计划未归档；`ARCHITECTURE.md` 与 `ARCHITECTURE_OVERVIEW.md` 并存（是否冲突**未核实**） | 目录计数 | 维度审查 |

---

## 四、风险清单汇总（P0–P3）

### 4.1 P0（4 项，全部主控已复核）

| ID | 一句话 | 维度 | 状态 |
|---|---|---|---|
| **CR-01** | HTTP 只读端点 **fail-open**：`/v1/memories/list` 等端点无 `user_id` 概念且不设 `privacy_context`，返回全库正文 | 安全 | 未修复 |
| **CR-02** | MCP 通道 `privacy_context` **硬编码 `None`**（5 处）⇒ 私有标记写进去、读时被忽略 | 安全 | 未修复 |
| **CR-05** | `consolidate` 内层 **60s ≥ 外层 30s** ⇒ 协作式取消**不可达**，O(n²) 聚类空跑 | 并发 | 未修复 |
| **CR-06** | release **`panic="abort"`** ⇒ 全部 `catch_unwind`/JoinError 隔离失效，一处 panic 即 abort 整进程 | 并发 | 未修复 |

### 4.2 P0（限可选 feature，主控已复核 1 项）

| ID | 一句话 | 前提 | 状态 |
|---|---|---|---|
| **CR-07** | Postgres `ON CONFLICT DO UPDATE SET` 只更新 10/24 列 ⇒ 十余字段更新**静默丢弃** | 启用 `postgres` | 未修复 |
| CR-08 | Postgres 未持久化 `information_gain`/`resolution`/`daoti_preview_*` | 启用 `postgres` | 未修复 |

### 4.3 P1（18 项）

安全隐私：CR-03（associations 无隐私参数）、CR-04（跨用户合成聚合 + 建议区）、CR-19（认证默认放行 + `user_id` 自报）
并发：CR-10（结晶 Phase1 持锁长重活且不查 cancel）、CR-11（MCP 无超时取锁）、CR-12（audit 启动 O(n²)）、CR-13（confirm 无界自旋）、CR-14（benchmarks 内层 90s > 外层 30s）
持久化：CR-17（Qdrant 时间失真）、CR-18（Qdrant fallback 不落盘）、CR-37（备份与写不互斥）、CR-38（import 非原子无锁）、CR-39（Neo4j 白名单过窄）
架构：CR-15（Layer2 反向依赖 Layer1）
测试：CR-20（env 竞态，本轮已部分修复）、CR-21（门禁口径不一致）、CR-45（备份/恢复/迁移 0 覆盖）
前端文档：CR-50（响应字段无门禁）

### 4.4 P2 / P3

P2：CR-22、23、24、25、26、27、31、34、35、40、41、42、43、46、47、48、51、52、53（19 项）
P3：CR-28、29、30、32、33、36、44、49、54、55、56、57、58（13 项）
（注：CR-16 已合并入 CR-33，不再单独编号）

> **逐项修复状态与对应 FIX 编号见 §十「第三梯队（P2/P3，共 32 项）」**；完整条目见第三节各维度表。

---

## 五、推进路线（三梯队）

### 第一梯队（P0，建议立即处理）

| 顺序 | ID | 动作 | 成本 |
|---|---|---|---|
| 1 | **CR-02** | MCP 的 `recall`/`recall_enhanced`/`list_memories` 透传 `session_id`/`user_id` 构造 `privacy_context`（与 `/enrich` 同口径）；无归属时**至少不得返回非 `Global` 记忆** | 小 |
| 2 | **CR-01** | 隐私判定改为 **fail-closed**（`None` ⇒ 仅 `Global` 可见），或为 `list`/`recent`/`stored-edges` 补 `user_id` 并强制构造上下文 | 中 |
| 3 | **CR-05** | 内层超时改为 **< 外层**（如 20s），或把取消源移到不受 future 丢弃影响的位置 | **极小（改常量）** |
| 4 | **CR-06** | 决策：release 改回 `panic="unwind"`（保住隔离与 `Drop`），**或**删除误导性的隔离话术并在启动/监控上显式承认 abort 语义 | 小（决策 + 一处配置） |

### 第二梯队（P1，建议本轮迭代）

1. **CR-20 + CR-21**：抽出**全 crate 唯一**的 `#[cfg(test)] static GLOBAL_ENV_LOCK` 供所有改 env 的用例复用；三处门禁统一为**同一口径**（推荐统一 `--test-threads=1`，或修竞态后全面并行）。
2. **CR-45**：补备份/恢复/迁移用例（这是**唯一 0 覆盖的安全关键路径**）。
3. **CR-10**：结晶 Phase1 移入 `spawn_blocking` + 改批量 `save_memories` + 循环内周期自查 `cancel`（同时缓解 503、CPU、内存三重压力）。
4. **CR-12**：`insert(0)` → `push` + 一次 `reverse`（O(n²)→O(n)），消除启动黑箱延迟。
5. **CR-13 / CR-14**：统一 2s deadline；内层超时统一 < 30s。
6. **CR-11**：把 MCP 路径与 v1 路径对齐（`spawn_blocking` + 2s `try_lock` + 执行超时）。
7. **CR-03 / CR-04**：为关联类端点补隐私参数；合成聚类按 `(privacy_level, user_id, session_id)` 分桶、合成记忆继承来源隐私。
8. **CR-19**：默认签发并**强制** token；`user_id` 改由服务端从凭据派生。
9. **CR-15 / CR-16**：`RecallResult` 等纯类型上提中立模块；桌面端 `crypto.rs` 收敛为共享实现（并同步已漂移的错误文案）。
10. **CR-50**：契约脚本增加关键端点**响应 key 断言**。

### 第三梯队（P2/P3，排期处理）

- 结构债：CR-22（God Object 拆 `v1/handlers/*` + `pub` 字段降级）、CR-23（抽两个锁 helper）、CR-24（阈值单一真源）、CR-27、CR-29
- 持久化加固：CR-40（fsync）、CR-37/38、CR-41（分片/SQLite）、CR-42（mtime 校验）、CR-17（Qdrant 字段）
- 门禁完备：CR-46（钉 rustc）、CR-47（必跑断言）、CR-48（补后端 feature 测试）
- 文档治理：CR-51/52/53/54/55/56/57/58

### 5.4 全局修复架构（消除「局部漏」的结构性方案）· 2026-10-07

> **原则变更**：不再逐条打补丁，而是**让"漏"在结构上不可能发生** —— 单一真源 / 编译期强制 / 守卫测试。

| 阶段 | 全局机制 | 一次消灭的条目 |
|---|---|---|
| **G1 写入单一真源** | `MemoryWriteInput` + `into_memory()` + **`apply_merge()`（合并语义集中一处）**；`consolidate`/`remember`/`correct`/MCP `remember`/`import`/`migration`/`remember_batch` 全部改走它 | CR-26；以及本报告审查者自认的"非全局修 #1/#4" |
| **G2 字段清单单一真源 + 贯通守卫** | `Memory` 字段清单唯一化；**矩阵测试**（写入入口 × 持久化后端）断言"写进去 = 原样读回" | CR-07、CR-08、CR-17（**类别性**消灭，而非逐后端补列） |
| **G3 可见性全局化** | `VisibilityContext` 分层：多用户域对外端点强制 scope、本地可信域保留；**取消隐式 `None`=全可见的对外含义** | CR-01、CR-02、CR-03、CR-04 |
| **G4 并发/时序单一真源** | 锁 helper 与超时常量各一份；**阶梯静态断言测试**（内层 < 中层 < 外层）；MCP 接入同一 helper | CR-05、CR-10、CR-11、CR-13、CR-14、CR-23、CR-34、CR-35 |
| **G5 启动重放** | `insert(0)` → `push` + 一次 `reverse`（O(n²)→O(n)） | CR-12 |
| **G6 门禁全局化** | 全 crate **唯一** env 锁；三处门禁口径统一；钉 rustc 版本 | CR-20（剩余 5 组）、CR-21、CR-46 |
| **G7 门禁补齐** | 备份/恢复/迁移用例；后端 feature 用例入 CI | CR-45、CR-47、CR-48 |
| **G8 panic 与失败语义** | 见决策 3 | CR-06 |
| **G9 结构单一真源** | 合成阈值 / 构造逻辑 / feature 别名 / 桌面 crypto 各收敛一份 | CR-15、CR-24、CR-25、CR-27、CR-33 |
| **G10 文档与契约** | 版本三处对齐；许可证边界单一真源；契约脚本加**响应字段断言** | CR-50、CR-51～CR-58 |
| **G11 God Object 拆分** | `memory_store.rs` / `v1_api.rs` / `app.js` 分域（独立工程；本轮仅以 G1 铺路） | CR-22、CR-28、CR-29、CR-30 |

### 5.5 三项关键决策（由六钥匙 + 代码证据共同决定）

**决策 1｜合并时 `created_at`：取「新值与旧值中更早者」**

- *重述*：`created_at`＝事件发生时间（event time），`updated_at`＝记录处理时间（processing time），二者职责正交。
- *泛化*：流处理领域的 event time / processing time 区分；event time 一经确定不应被后续写入改写。
- *代码证据*：`update_content`（`memory_types.rs:545-558`）**只动** `content`/`version_history`/`updated_at`，**不动 `created_at`** ⇒ 既有设计意图即"created_at 不随更新变"，本次修法应顺从而非新开例外。
- *化简*：`created_at = min(旧值, 新源时间)`（仅当入参显式提供源时间时参与比较）。它同时满足三种情形——
  ① 旧值是"摄入时刻"、新值是真源时间 ⇒ **修正**；② 两者都是真源时间 ⇒ **保留首次发生**；③ 无源时间入参 ⇒ **不变**；且**幂等**（反复写入不漂移）。
- **落地**：写入 `apply_merge()` 单点实现 + 参数化测试覆盖上述三情形。

**决策 2｜隐私隔离：分层（对外强制、本地保留）**

- *类比*：多租户系统的**信任边界**——回环 + 未启用 token ＝单用户可信域；非回环 或 已启用 token ＝多用户域。
- *代码证据*：`is_visible(None)`＝全可见是**存储层**语义，桌面端单用户依赖它；而适配层为满足隔离专门绕道 `consolidate + user_id` ⇒ 缺口在**入口**而非存储层。
- *化简*：**存储层语义不动**（不破坏桌面端）；在入口引入 `VisibilityContext`：多用户域下对外端点（`list`/`recent`/`stored-edges`/`enrich`/`associations`/`association-graph`/MCP）**必须**给出 `user_id`（否则 400 `scope_required`）；本地可信域保留现行行为。`TrustMode` 由既有 token 校验 + 回环判定派生（`server.rs` 已有该逻辑）。

**决策 3｜`panic` 策略：release 改回 `unwind`**

- *繁衍（反推）*：代码中**已存在**大量 `catch_unwind` / `JoinError → "服务已保持运行"` 分支；保留 `abort` ⇒ 这些是死逻辑，删除它们的成本 > 改配置一行。
- *类比*：装了熔断器却把总闸设成"一熔即断总闸"。
- *化简*：`Cargo.toml` 的 `[profile.release] panic = "abort"` → `"unwind"`；一行配置让既有隔离投入生效，且 `panic` 时 `Drop` 会执行（`SingletonLock` 锁文件不残留）。`overflow-checks = true` 的语义也随之从"溢出即整进程死"变为"隔离为一次请求失败"。

### HCSE 回写建议（依用户规则）

1. 本次新增的**故障模式**建议回写项目检查清单：
   - **"跨模块进程级环境变量竞态"**（CR-20）——现有检查项只覆盖"测试是否设了 env"，未覆盖"锁是否跨模块唯一"。
   - **"超时阶梯被打破 ⇒ 取消机制成为不可达代码"**（CR-05/CR-14）——建议把"内层 < 中层 < 外层"列为 CI 可静态检查项（常量比对）。
   - **"fail-open 隐私默认值"**（CR-01/CR-02）——建议把"新增读取端点必须以无 `user_id` 请求验证不可全量返回"列为契约测试必需项。
2. 本次**未被任何智能体预警**的项：CR-05（超时阶梯）与 CR-20（跨模块 env 竞态）均由本次审查首次发现 ⇒ 属"现有检测范围之外"，需新增检查项。

---

## 六、本轮审查过程中已当场修复的项

| ID | 问题 | 证据 | 修复 | 验证 |
|---|---|---|---|---|
| **FIX-01** | **工作区 clippy 门禁已红**（`-D warnings` 下 5 处 error），且 `cargo test` 不会拦截 ⇒ "本地 test 绿、CI clippy 红" | `cargo clippy --features server --all-targets -- -D warnings` **实测 exit 101**：`unused import to_bytes`（`v1_api_tests.rs:5501`）、`redundant closure`（`v1_api.rs:1949`）、`doc_lazy_continuation` ×3（`memory_state_machine.rs:396-398`） | 5 处全部修复（冗余闭包去糖、删除未用 import、doc 列表补空行） | ✅ 复验**两条 CI 口径**均 `Finished`（exit 0） |
| **FIX-02** | `memory_state_machine.rs` 两条配对用例并发读写 `LRC_STATE_BIAS` 致**偶发失败**（单独跑必过、全量并行跑偶发失败） | 同上维度六 | 加模块级 `STATE_BIAS_ENV_LOCK` 串行化；并改正"不对取值做假设即可避免干扰"的错误注释 | ✅ `cargo test --features server --lib` **连跑两遍 842 passed / 0 failed** |
| **FIX-03** | **G1 写入单一真源落地**：新建 `src/memory_write.rs`（`MemoryWriteInput` + `into_memory()` + `apply_merge()`）；`MemoryStore::remember` 的**合并块**改为调用 `apply_merge` ⇒ 合并路径首次获得显式规则（含决策 1 的 `created_at` 取更早者、归属只补全、隐私不扩权） | 合并块原内联实现（`memory_store.rs` 旧 L3744-3813） | 外提为单一真源 + 11 条模块单测（覆盖决策 1 的三情形 + 幂等 + 不扩权） | ✅ `854 passed / 0 failed`（+11）；CI 两条 clippy 口径均通过 |
| **FIX-04** | **CR-26 源时间契约统一**：`remember` 的 `created_at`(RFC3339) 改为与 `consolidate` 一致的 `source_timestamp_ms`(毫秒)；两条端点同走 `MemoryWriteInput` | `v1_api.rs` `MemoryRememberRequest` | 统一字段名与类型 + 解析函数单一入口；新增 `test_remember_accepts_source_timestamp_ms_same_as_consolidate` 钉死契约 | ✅ 通过 |
| CR-59（**新增发现**） | `--features server,ml` 下 `src/engine/luoshu_encoder_ml.rs:478/1962/2223` 存在 clippy 告警；而 CI 只跑 `--features server` 与**默认 feature** ⇒ **ml 口径的 clippy 从不被检查**（既有项，非本轮引入） | 实测 `cargo clippy --all-targets --features server,ml -- -D warnings` 退出码 101 | 将 ml 纳入 clippy 门禁或显式声明其不在门禁范围 | 未修复 |
| **FIX-05** | **G1 收口（用户侧写入入口全部改走单一真源）**：`MemoryWriteInput` 补齐 `ttl_days` + `daoti_preview_*`（覆盖 MCP 的全部可写字段）；MCP `remember`（单条）与 MCP 批量注入**均改走 `MemoryWriteInput`**；批量路径新增**批次级归属**（`privacy_level`/`session_id`/`user_id`，与既有 `batch_event_id` 同款对称设计）——此前批量写入**无法声明归属** | `server.rs` 旧 L2261-2280（批量）、旧 L2554-2569（单条）；`memory_write.rs` 字段集 | 三处写入入口（HTTP ×2 + MCP ×2）同走单一真源；调用点使用**完整结构体字面量**（不用 `..Default::default()`）⇒ **以后新增字段会编译报错**，不可能再静默漏 | ✅ `854 passed / 0 failed`；CI 两条 clippy 口径均通过 |

> **G1 边界声明（避免"静默跳过"）**：G1 已覆盖**全部用户侧写入入口**（`consolidate`、`remember`、MCP `remember`、MCP 批量）。
> 以下**未覆盖**且**有意不在 G1 处理**，已登记归属：
> | 位置 | 为何不属 G1 | 归属 |
> |---|---|---|
> | `MemoryStore::correct_memory` | 是**显式修正既有记忆**（`update_content_with_reason`），不是"创建"路径，不存在字段集缺失 | 不处理（已由 C05 修复保证原子写） |
> | `consolidation.rs:367/690/878`、`engine/synthesis_engine.rs:461/724/826` | 是**结晶/合成的派生写入**，需连带"**继承来源隐私**"语义（即 CR-04）一起改 | **G3**（与 CR-04 同批） |
> | `discovery.rs:963/994/1160`、`memory_store.rs:7431` | 是**内部派生写入**（发现通道 / 展开子记忆），同样需归属语义 | **G3** |
> | `export.rs` import、`migration.rs` | 是**文件级**路径（非 store 写入），字段保全由"直通合并"保证；其真问题是原子性 | **G5 / G2**（CR-38） |

| **FIX-06** | **G2 字段清单单一真源 + 后端结构性修复**：新建 `src/persistence/fields.rs`（`MEMORY_FIELDS`＝29 字段唯一清单，含 Postgres 列定义）+ 由清单**派生**的 `create_table_sql` / `add_missing_columns_sql` / `insert_columns_sql` / `insert_placeholders_sql` / `upsert_set_clause_sql`；**5 条免连库守卫测试**（清单≡序列化键集合、DDL 覆盖全字段、补列覆盖全字段、列数≡占位符数、SET 子句覆盖全部非主键字段） | — | **Postgres**：DDL 与 INSERT/upsert 子句改由真源派生 ⇒ **CR-07**（SET 只更新 10/29 列）与 **CR-08**（5 字段从未建列）**按构造消灭**；并加幂等 `ADD COLUMN IF NOT EXISTS` 使**老库也能补列**；修正 `DEFAULT '{{}}'` → `'[]'` 笔误。**Qdrant**：payload 改存**整条记忆 JSON**（`memory_json`）、读回用 serde 还原 ⇒ **CR-17**（时间戳被重置为 now + 多字段丢失）按构造消灭 | ✅ 默认门禁 **859 passed / 0 failed**；CI 两条 clippy 口径通过；`cargo check --all-targets --features server,postgres` 与 `--features server,qdrant` **均 Finished（exit 0）** |

> **G2 环境说明（诚实声明）**：本机 `crates.io` 的 TLS 吊销检查失败（`CRYPT_E_NO_REVOCATION_CHECK`），
> 需 `CARGO_HTTP_CHECK_REVOKE=false` 才能下载 `sqlx` / `qdrant-client` 依赖。上述两条 feature 的编译验证即在该设置下完成。
> **残留（当时口径，现已闭环）**：G2 实施当刻 CR-18（Qdrant `fallback_memories` 仅进程内、不落盘）尚**未处理**；
> 该 P1 已于随后由 **FIX-18** 修复（Qdrant fallback 落盘 + WAL 6 单测）。本行保留为**当时真实状态**的记录，
> **非**当前残留——当前 CR-18 状态见 §十 第二梯队（已修复）。

| **FIX-07** | **G3 第一步：可见性作用域单一真源 + MCP 隐私通道接通**。新建 `src/privacy.rs`：`scoped()` / `scope_from_args()`（隐私上下文**唯一构造入口**）、`TrustMode` + `judge()` + `from_env()` + `require_scope()`（"本地可信域保留、对外域强制"的**分层**策略，以既有 `LRC_API_TOKEN` 开关为判据，**不新增配置**），含 5 条单测 | `server.rs` 5 处 `privacy_context: None`（原 L913/950/966/2006/2443） | 5 处硬编码改为 `scope_from_args(arguments)` ⇒ **CR-02 读取侧已修**（此前 MCP 写入的 `user_id`/`privacy_level` 读时被**完全忽略**）。因 `spawn_blocking` 闭包要求 `'static`（借用会报 E0521），隐私上下文改为**闭包外构造 + clone 进入** | ✅ 默认门禁 **864 passed / 0 failed**（+5）；CI 两条 clippy 口径通过 |

> **G3 进度与残留（诚实声明）**：本步只完成了"**让 MCP 能按参数隔离**"这半边。
> **尚未完成**：① **CR-01** —— HTTP 只读端点（`list`/`recent`/`stored-edges`）**仍无 `user_id` 参数**，
> 且 `require_scope`/`from_env` **尚未接入任何入口** ⇒ 多租户域下"省略 scope ⇒ 全可见"的 fail-open **仍在**；
> ② **CR-03** —— `associations` / `association-graph` 仍无隐私参数（需改 `MemoryStore` 方法签名并加 `is_visible` 过滤）；
> ③ **CR-04** —— 派生写入（结晶/发现通道）未继承来源隐私。以上均留在 G3 内继续。

| **FIX-08** | **G3 第二步：CR-01 已修 —— 只读端点补 scope + 多租户域强制**。`RecentMemoriesParams` / `MemoryListRequest` 新增 `user_id`/`session_id`；`list`/`recent`/`stored-edges` 三处统一改用 `privacy::resolve()`（构造 + 强制），多租户域缺 scope ⇒ **400 `scope_required`**；新增 `build_v1_router_with_trust()`（生产入口用 `from_env()` 传入，**既有调用方与测试零改动**，默认 `LocalTrusted` 保持既有语义） | `v1_api.rs` 三处 filter 构造；`server.rs` 路由构建 | 新增端到端用例 `test_read_endpoints_enforce_scope_by_trust_mode`：多租户域 `/memories/list`、`/memories/recent` 缺 scope ⇒ **400**、带 `user_id` ⇒ 200、本地域缺 scope ⇒ 200（**两侧都钉死**） | ✅ 默认门禁 **866 passed / 0 failed**；CI 两条 clippy 口径通过 |

> **G3 剩余（诚实声明）**：**CR-03**（`associations` / `association-graph` 仍无隐私参数，需改 `MemoryStore` 方法签名 + `is_visible` 过滤）
> 与 **CR-04**（派生写入未继承来源隐私）**尚未完成**。

| **FIX-09** | **G4 第一步：超时阶梯单一真源 + 三个 P0/P1 缺陷**。新建 `src/timeouts.rs`（`HTTP_OUTER_TIMEOUT`/`HTTP_HANDLER_BUDGET`/`EXPLORE_INNER_BUDGET`/`LOCK_WAIT`/`LOCK_POLL`/`CONSOLIDATE_INNER_TIMEOUT`/`BENCHMARKS_INNER_TIMEOUT`）+ **守卫测试 `timeout_ladder_is_monotonic`**（断言"锁等待 < 内层 < 中层 < 外层"） | `v1_api.rs` L2044(60s)、L5583(90s)、L712；`server.rs` 外层 `TimeoutLayer(30s)`；`v1_api.rs` confirm 取锁环 | **CR-05**：`consolidate` 内层 60s → **20s**（< 外层 30s）⇒ 协作式取消分支**恢复可达**；**CR-14**：`benchmarks` 内层 90s → 20s；**CR-13**：`/v1/associations/confirm` 无界自旋 → **2s 有界等待**，并新增 `Ok(None) ⇒ 503 store_busy` 分支（此前**永不返回**且无法区分"忙"与"失败"）；**CR-34 部分**：探索内部预算改由真源引用 | ✅ 默认门禁 **868 passed / 0 failed**（+2 守卫）；CI 两条 clippy 口径通过 |

> **G4 剩余（诚实声明）**：CR-10（后台结晶 Phase1 持锁做编码+全量 JSON 重写且不查 cancel）、CR-11（MCP 多个 handler 仍是**无超时** `lock().await`）、CR-23（11 处取锁模式重复未抽 helper）、CR-35（内存峰值叠加）**尚未完成**。

| **FIX-10** | **G8：CR-06 已修 —— release `panic` 由 `abort` 改为 `unwind`**。新增 **crate 级守卫测试** `build_profile_tests::release_profile_must_use_unwind`（读 `Cargo.toml` 断言 `[profile.release]` 段为 `unwind` 且不含 `abort`）—— 防止有人改回后"隔离分支静默变成死逻辑" | [Cargo.toml `[profile.release]`](file:///g:/code-memory/Cargo.toml#L199-L211) | 代码中大量 `catch_unwind` / `JoinError → "服务已保持运行"` 分支**只在 unwind 下成立**；`abort` 会使它们成为**死逻辑**，且任何 panic（含 `overflow-checks` 的整数溢出）都 abort **整个 sidecar**、`Drop` 不执行、`SingletonLock` 锁文件残留。已 grep 确认**无任何代码/`cfg` 依赖 abort 语义** | ✅ 默认门禁 **869 passed / 0 failed**；CI 两条 clippy 口径通过；`cargo check --release --features server` **Finished（exit 0）** |

> **G8 验证边界（诚实声明）**：本轮只跑 `cargo check --release`（验证清单与 profile 一致、无 `cfg(panic)` 冲突），
> **未执行完整 release 链接**（`lto=true` + `codegen-units=1` 耗时较长）——该步由 CI/发版流程覆盖。

| **FIX-11** | **G4 第二步：CR-10 已修 —— 结晶 Phase1 三处并发缺陷**：① `self.store.lock().await`（**无上限**）→ `timeout(LOCK_WAIT, …)`，拿不到锁即让出本轮（`last_run` 不推进 ⇒ 下轮重试）；② 写循环**此前从不读 `cancel`**（120s 周期超时的"兜底"对本段是**假的**）→ 每 chunk 与每条均自查；③ 每 `PHASE1_CHUNK=16` 条**显式 `drop(guard)` 让锁**，给前台请求插队机会 | [consolidation.rs Phase1](file:///g:/code-memory/src/consolidation.rs#L355-L432) | 三者共同指向历史上"后台结晶独占全局锁 ⇒ 前台大面积 503"的根因面 | ✅ 默认门禁 **869 passed / 0 failed**（含全部 consolidation 用例）；CI 两条 clippy 口径通过 |

> **G4 剩余（诚实声明）**：CR-10 的**结构性部分**（把**编码**移出锁 + 批量落盘）**未做** ——
> 需要新增"**保留相似合并语义**的批量写" API（不能直接用 `remember_batch`，它**跳过合并**会改变语义）。
> 另有 CR-11（MCP 多个 handler 仍是无超时 `lock().await`）、CR-23（11 处取锁模式未抽 helper）、CR-35（内存峰值叠加）待续。

| **FIX-12** | **G4 第三步：CR-11 主体已修 —— MCP 通道统一"有界取锁"**。新增 `lock_store_bounded<P>()`（`timeout(LOCK_WAIT, store.lock())`，与 v1 路径**同口径**）；把 **11 处** MCP 取锁点由**无上限** `state.memory_store.lock().await` 改为有界等待，超时回 `store_busy`（`-32000`）：`recall` / `batch_remember` / `list_memories` / `remember` / `forget` / 重要性更新 / `archive` / `correct` / `associations` / `association-graph` / `memory_stats` | `server.rs`（含 dispatch 与各 `handle_*`） | 此前 MCP 请求在锁上**无限排队**（既不失败也不返回），与 v1"2s 有界 + `store_busy`"明显不一致 —— 这是 CR-11 的核心 | ✅ 默认门禁 **869 passed / 0 failed**；CI 两条 clippy 口径通过 |

> **G4 剩余（诚实声明）**：`config_handler` 内仍有 **2 处** `memory_store.lock().await`（配置更新路径，非工具调用热路径）未改造；
> 另 CR-10 的**结构侧**（编码移出锁 + 保留合并语义的批量写 API）、CR-23（抽锁 helper 统一 11 处 v1 侧的取锁模式）、CR-35（内存峰值）仍待做。

| **FIX-13** | **G3 第三步：CR-03 已修（含接线与契约测试）**：新增带可见性过滤的存储层变体** `MemoryStore::associations_scoped()` / `association_graph_scoped()`（**保留原函数** ⇒ ~20 处测试调用零改动）；契约 **fail-closed**：`scope=None` 全可见（本地行为不变）／anchor 不可见 ⇒ 空／peer 查不到 ⇒ 剔除；关联图额外**重算** `direct_count`/`indirect_count`（否则计数泄漏"有多少不可见邻居"）并在发生裁剪时置 `truncated=true`。另抽出 `pub type PrivacyScope` 统一 `is_visible` 的口径。 | [memory_store.rs](file:///g:/code-memory/src/memory_store.rs#L5665-L5770) | CR-03 | ✅ **已完成接线**：2 个 v1 端点（`v1_api.rs` 的 `/memories/associations` 与 `/memories/association-graph`）补 `privacy::resolve(trust_mode, …)` + 请求体 `user_id`/`session_id` + `scope_required` 强制，并改调 `*_scoped`；2 处 MCP 分支（`server.rs`）改用 `privacy::scope_from_args()`。新增契约测试 `test_associations_scoped_no_cross_user_leak`（**具备鉴别力**：先断言"无 scope 时该关联确实存在"，再断言"带 scope 时必须消失"；另覆盖越权锚点返回空 + 图 nodes/edges 零泄漏 + 合法数据不被误杀）。✅ 门禁 **870 passed / 0 failed**；CI 两条 clippy 口径通过 |

| **FIX-14** | **G3 第四步（**真源已建，CR-04 尚未生效**）**：① `privacy::ScopeTuple` 收炼为 `= crate::memory_store::PrivacyScope` 的**同义词**（消除 G3 分两步实施留下的**同义真源重复**）；② 新增 **`privacy::inherit_privacy()`** —— 派生写入隐私继承的**唯一真源**：规则为"取各来源可见性的**交集（最严）**"，`Global` 来源不构成约束（**既有行为逐位不变**），**跨 user ／ 受约束却缺 `user_id` ／ Session 级来源分属不同 session ⇒ 返回 `None`（放弃派生）**。配 6 条单测（含跨 user 拒绝、缺归属拒绝） | [privacy.rs](file:///g:/code-memory/src/privacy.rs#L129-L199) | CR-04 | ✅ **已完成（并更正审查清单的三处误报）**：逐点核实全部 `Memory` 构造路径后的结论 —— ① `consolidation.rs` **3 处**（LLM 合成 / Embed 合成 / 表层写入）**本次全部接上** `inherit_privacy()`；② `memory_store.rs` 的 `unfold_synthesis`（拆解合成记忆）**本就继承父记忆隐私**（`privacy_level`/`session_id`/`user_id` 三字段逐一赋值），且父记忆现已继承来源 ⇒ **传递链成立**；③ 审查清单中的 `synthesis_engine.rs` ×3 **该文件不存在**、`discovery.rs` ×3 **全部位于 `#[test]` 内**（生产路径无 Memory 构造）⇒ **均为误报**。另：`remember` 的相似合并走 `find_similar_scoped_with_privacy(..., &remember_privacy)` ⇒ **不会跨用户合并**，无泄漏面。附带修正表层写入的级别**悖论态**（`user_id`/`session_id` 皆空时曾得 `User` 级 + `user_id=None` ⇒ 对任何人都不可见；现三分法：有 session ⇒ `Session`／仅 user ⇒ `User`／皆无 ⇒ `Global`）。✅ 门禁 **876 passed / 0 failed**；clippy 通过 |

| **FIX-15** | **G4 第四步：CR-23 部分成立、已修真正残留**。① 核实结论：**v1 侧早已有统一 helper** `lock_store_with_timeout`（**12 处**调用）⇒ 审查所称"11 处取锁模式未抽 helper"**对 v1 不成立（第三处误报）**；② **真正残留**是该 helper 内**硬编码 `Duration::from_secs(2)`**，与 `timeouts::LOCK_WAIT` 是两个独立的 2s ⇒ 真源调整时 v1 与 MCP 两条通道会**静默分叉**（正是 CR-11 的同类根因）。已改为引用真源 | [v1_api.rs](file:///g:/code-memory/src/v1_api.rs#L121-L127) | CR-23 | ✅ 门禁 **876 passed / 0 failed**；clippy 通过。⚠️ **未做**：`server.rs:3894/3981` 两处 `config_handler` 取锁（用的是**局部变量** `memory_store` 而非 `state.memory_store`，且所在函数的错误返回风格未核实 ⇒ 未贸然改）；CR-35 内存峰值未做 |

| **FIX-16** | **G5 第一步：CR-12 已修 —— 审计事件记录/重放由 O(n²) 降为 O(1) 摊还**。根因确证为**两处 `Vec::insert(0, …)`**：① [load_from_file](file:///g:/code-memory/src/engine/audit_trail.rs#L879-L910) 逐行重放（⇒ 启动 O(n²)，与线上 770MB `audit.jsonl` / 启动卡 13 分钟**因果吻合**）；② [record](file:///g:/code-memory/src/engine/audit_trail.rs#L997-L1085) —— **热路径**：每次记录审计事件都搬移整个列表（每次检索都 O(n)）。修法：`events: Vec<AuditEvent>` → **`VecDeque<AuditEvent>`** + `insert(0,..)` → `push_front(..)`（O(1) 且"最新在前"语义**逐位不变**；不采用"改 push + 逆序读"因其会改变全部读取方看到的顺序）。附带修正 `VecDeque` 不兼容的 `first()`→`front()`、`last()`→`back()`、测试中 `push`→`push_back`。**另核实：`migrate_legacy_events` 本就是单趟 + 一次 `write_atomic` 回写，非平方项**（推翻了我此前的怀疑） | [audit_trail.rs](file:///g:/code-memory/src/engine/audit_trail.rs) | CR-12 | ✅ 门禁 **877 passed / 0 failed**；clippy 通过。**并新增性能守卫** `record_cost_must_not_grow_with_history_size`（见下） |

> **★ CR-12 守卫的鉴别力（本会话首次做到"用实测余量证明守卫有效"）**
> 测试设计要点：取**同为 1000 条**的两批，分别在"列表≈1k"与"列表≈8k"时计时 ——
> 两批的**固定成本（哈希+分配+格式化）完全相同**，唯一差异是列表长度，故差异只能来自前插搬移。
> 实测（修复后）：**早期 2388µs / 晚期 2369µs / 比值 0.99** ⇒ 完全平坦；
> 阈值为 4.0，而 O(n) 回归的信号约 8.0 ⇒ **判定带 4 倍余量、修复态距阈值 4 倍**，两端清晰分离。
> （注：我最初的守卫版本用"总耗时随 n 的比值"，被巨大的固定成本稀释而**失效**——
> 该版本实测在 O(n) 代码下也会通过。已改为上述设计。）

| **FIX-17** | **G5 第二步：CR-38 已修 —— 导入路径改为「先全序列化、再逐文件原子写」**。修前 `import_memories` 对 `memories.json`/`chunks.json`/`archive.json` 用 **`fs::write` 直写**（整文件重写），有两个层次的问题：① 写一半崩溃/断电即留下**截断文件** ⇒ `memories.json` 损坏等于**全部记忆丢失**；② **边序列化边写** ⇒ 若 `chunks.json` 序列化失败，`memories.json` 已落盘 ⇒ 导入处于**半应用**状态且不回滚。现改为：先把三份载荷**全部**在内存备好（任何序列化失败都在**任何写入之前**中止），再用 Layer 1 公共设施 [`atomic_file::write_atomic`](file:///g:/code-memory/src/atomic_file.rs)（UUID 唯一临时名 + 原子替换 + 失败清理）逐文件落盘；并把计数改为**写入成功后**才置位（修前在写之前赋值，失败时会给出误导性的"已导入"）。**残留风险已明示**：三文件之间的 rename 序列不是**组原子**（极小窗口） | [export.rs](file:///g:/code-memory/src/export.rs#L284-L345) | CR-38 | ✅ 门禁 **878 passed / 0 failed**（含全部既有 export/import 用例）；clippy 通过。**新增源码级守卫** `import_must_not_use_non_atomic_write`（断言该函数体内不得再出现 `fs::write(` 且必须含 `write_atomic`）—— 此前的 P3-3 已把 4 处原子写收敛到该设施，**此处是遗漏的第 5 处** |

| **FIX-18** | **G5 第三步：CR-18 已修 —— Qdrant 兜底存储改为「落盘 WAL + 启动重放」**。修前 `fallback_memories` 是**纯进程内 `Mutex<Vec<Memory>>`，从不落盘** ⇒ "Qdrant 不可用/集合为空"期间写入的记忆**只活在内存**，重启即全部丢失 —— 而"兜底"的全部意义正是"主存储不可用时也不丢数据"，属**功能承诺与实现不符**。现实现：① 追加式 **WAL**（`FallbackRecord::{Memory, Delete, Clear}`，O(1) 追加，不重写整文件）；② 启动 `restore_fallback_from_wal()` **重放**恢复；③ **墓碑**（`Delete`/`Clear` 必须入 WAL，否则重启后已删记忆会**复活**）；④ 重放后若行数≠结果数则**原子压实**一次（否则追加式 WAL 无界增长）；⑤ Qdrant 成功加载后 `rewrite_fallback_wal()` 与镜像**对齐**。配置新增 `fallback_path`（默认 `lrc_qdrant_fallback.jsonl`，`LRC_QDRANT_FALLBACK_PATH=""` 可显式退回仅内存） | [persistence/qdrant.rs](file:///g:/code-memory/src/persistence/qdrant.rs) | CR-18 | ✅ `cargo test --features server,qdrant --lib fallback_wal_tests` → **6 passed**（后写者胜／墓碑／Clear／损坏行跳过／压实无损／空输入）；默认门禁 **878 passed / 0 failed** 不回归。⚠️ **发现门禁盲区**：`qdrant` 模块受 `#[cfg(feature = "qdrant")]` 门控，**默认 CI 根本不编译该后端** ⇒ 其正确性此前无人验证（已在代码注释中标注，并入 G7 / CR-47） |

| **FIX-19** | **G6 第一步：CR-20 已修 —— 进程级环境变量锁收敛为「全 crate 唯一」**。修前 4 个模块（`memory_state_machine` / `state_matcher` / `dao_regulator` / `model_downloader`）各持**模块私锁**，且另有 5 组用例**完全裸奔**（`memory_store_tests` 的 `LRC_STATE_BIAS`/`LRC_DAOTI_PREVIEW_PRUNE`/`LRC_DOMAIN_CANDIDATE_INDEX*`、`v1_api_tests` 的 `LRC_ASSOC_EDGE_FEEDBACK`/`LRC_ASSOC_PATH_SCORE`、`process_guard` 的 `LRC_LLM_API`、`discovery` 的 `LRC_ACTIVE_DISCOVERY`）。**关键洞察**：私锁只在**模块内**互斥 —— 而同一变量会被多个模块改写（`LRC_STATE_BIAS` 既在 `memory_state_machine` 又在 `memory_store_tests`），故"跨模块竞态"依旧存在，只从**必现**退化为**偶发**（更难定位）。现新建 `#[cfg(test)] pub mod test_env`（`OnceLock<Mutex<()>>`，中毒时 `into_inner()` 不连带炸其它用例）作为**唯一那把锁**；4 个模块私锁全部删除并委托，5 组裸奔用例全部补锁 | [lib.rs](file:///g:/code-memory/src/lib.rs#L64-L142)、`memory_state_machine.rs`、`state_matcher.rs`、`engine/dao_regulator.rs`、`engine/model_resolver.rs`、`engine/model_downloader.rs`、`memory_store_tests.rs`、`v1_api_tests.rs`、`process_guard.rs`、`discovery.rs` | CR-20 | ✅ `cargo test --features server --lib` **连跑两遍 879 passed / 0 failed**；两条 clippy 口径与 `cargo fmt --check` 全绿。**新增守卫** `env_lock_must_be_unique_across_crate`（源码级：9 个含 env 写入的文件必须含 `crate::test_env::lock()`，且不得再出现 `Mutex::new(())` 私锁特征） |

| **FIX-20** | **G6 第二步：CR-21 已修 —— 三处门禁测试口径统一为 `--test-threads=1`**。修前 `ci.yml` 的 test job 用 `--test-threads=1` **串行掩盖竞态**，而 `.githooks/pre-commit:85` 与 `release.yml` preflight 的 `cargo test` 均**并行** ⇒ 竞态只在 CI 之外暴露，造成"CI 全绿却发布被偶发阻塞" | `.githooks/pre-commit`、[release.yml](file:///g:/code-memory/.github/workflows/release.yml) | CR-21 | ✅ 三处现为同一口径（`ci.yml` 原已串行，未改动其命令） |

| **FIX-21** | **G6 第三步：CR-46 已修 —— Rust 工具链版本钉死（单一真源）**。修前三个 workflow 共 11 处 `toolchain: stable`（滚动）而本地实测为 `1.99.0` ⇒ 二者随时分叉，滚动 stable 引入的新 clippy lint 会在 CI 突然命中而本地不可见。现三个 workflow 顶部统一声明 `RUST_VERSION: 1.99.0`（与本地对齐），全部 `with.toolchain` 引用该变量 ⇒ **升级只改一处** | [ci.yml](file:///g:/code-memory/.github/workflows/ci.yml#L11-L18)、[release.yml](file:///g:/code-memory/.github/workflows/release.yml#L14-L19)、[security.yml](file:///g:/code-memory/.github/workflows/security.yml#L19-L22) | CR-46 | ✅ 全仓 `toolchain: stable` 0 命中；**未采用 `rust-toolchain.toml`**（会强制本地 rustup 另行下载 `1.99.0` 具名工具链，可能破坏"G 盘编译一直正常"的本地环境） |

| **FIX-22** | **G6 第四步：CR-59 已修 —— ml 口径的 clippy 纳入门禁 + 3 处潜伏告警清零**。实测 `cargo clippy --all-targets --features server,ml -- -D warnings` **exit 101**，命中 `luoshu_encoder_ml.rs` 三处：① `:478` `neg_cmp_op_on_partial_ord`（`!(sum > 0.0)`）→ 改 `sum.partial_cmp(&0.0) != Some(Ordering::Greater)`（**逐位等价**，含 NaN ⇒ true；且上一行已排除非有限值）；② `:1962` `doc_lazy_continuation` → 列表与后续段落之间补空行；③ `:2223` `needless_range_loop` → `for (j, _) in by_struct.iter().take(..)` | [luoshu_encoder_ml.rs](file:///g:/code-memory/src/engine/luoshu_encoder_ml.rs) | CR-59 | ✅ ml 口径 clippy 复验通过（见下）。CI 的 clippy job 与 release preflight 各新增一步 `--all-targets --features server,ml`；clippy job 超时 30→45 分钟（并入 candle 全家桶编译量）。pre-commit **有意不跑** ml 口径（数分钟开销），但已在脚本内**显式声明**由 CI/release 强制 |


| **FIX-23** | **G6 第五步 / CR-19 已修（安全）—— 认证从 fail-open 改为 fail-closed，租户改为凭据派生**。两个子项：<br>**(a) 非回环 + 无 token ⇒ 拒绝启动**（原为"仅告警不阻断"）。理由：告警依赖部署者读到并理解，而 `--host 0.0.0.0` 一旦生效攻击窗口立即打开，属 fail-open，与"隐私隔离 fail-closed"的整体设计矛盾。现 `serve_on_listener` 打印端口信息后即调用 `ensure_authenticated_non_loopback`，失败返回 `PermissionDenied` 并打印醒目阻断信息。**兼容边界无一例外保住**：回环（含 `localhost`/`::1`/`[::1]`）无条件放行；非回环但已设 token 放行。<br>**(b) 隔离键由凭据派生**：原 `user_id` 完全由请求体自报 ⇒ 任何持 token 者可声称为他人读取其记忆。现新增 `privacy::tenant_from_credential`（token 的 SHA-256 前 16 字节，**不可逆** ⇒ 不把 token 明文写进库；**稳定** ⇒ 重启后归属一致）与 `privacy::namespace_owner`（多租户域 ⇒ `"{租户}::{user}"`，对齐业界 Zep 的 `tenant::user::uuid` 约定；本地可信域 ⇒ **逐字节原样**）。**接线零改调用点**：写入侧统一在 `MemoryWriteInput::into_memory()`（覆盖 HTTP×2 + MCP×2 全部写入入口），读取侧统一在 `privacy::resolve`（v1 的 5 处）与 `privacy::scope_from_args`（MCP 的 5 处） | [server.rs](file:///g:/code-memory/src/server.rs#L4833-L4900)、[privacy.rs](file:///g:/code-memory/src/privacy.rs#L34-L104)、[memory_write.rs](file:///g:/code-memory/src/memory_write.rs#L72-L100) | CR-19 | ✅ 门禁 **884 passed / 0 failed**（+5）；两条 clippy 口径与 `cargo fmt --check` 全绿。新增单测：`tenant_from_credential_is_stable_and_does_not_leak_token`、`namespace_owner_namespaces_only_when_tenant_present`、`resolve_local_trusted_unchanged_by_namespacing`、`loopback_host_detection_is_conservative`、`non_loopback_without_token_must_refuse_startup`。**决策依据**：anysearch 检索业界实践（MemorySync：客户端**不得**在请求体传 tenant_id，须由认证中间件从凭据派生不可变 principal；Zep：`tenant::user::uuid` 命名空间，且**共享单一 key 跨多租户等于没有隔离**；多租户 SaaS 通则：所有键以 tenant_id 为前缀）+ 六钥匙（简化：`user_id` 不是安全边界、**凭据才是**；泛化：tenant-scoped identity 是成熟模式；类比：API key ↔ 租户）。**遗留（显式声明）**：单 token = 单租户，故 MCP 通道在**不传** `user_id` 时在该租户内为"全可见"（真正多租户需每租户独立凭据，属独立特性，不在本轮） |

| **FIX-24** | **G5 第四步 / CR-37 已修 —— "一致快照"从承诺变成机制**。修前 `backup.rs` 的锁序文档白纸黑字写着"备份的语义就是某一时刻的一致快照，**必须与并发的写操作互斥**"，但**写路径从不取备份的任何锁** ⇒ 逐文件 `fs::copy` 期间实时写可以改了 `memories.json` 而 `audit.jsonl` 还停在旧版，快照里出现**悬空审计引用**。文档承诺与实现不符属最危险的一类缺陷（读代码者会以为已互斥）。修法：在 [atomic_file.rs](file:///g:/code-memory/src/atomic_file.rs) 新增**唯一一把**快照一致性闸门 `SNAPSHOT_GATE: RwLock<()>` —— **写入方**持读锁（[`writer_guard`]，彼此可并发）、**快照/恢复方**持写锁（[`snapshot_guard`]，独占）。接线：`atomic_file::write_atomic`（import 路径）与 `persistence::json::atomic_write`（**全部 JSON 落盘的唯一咽喉**）各取一次读锁；`create_backup` / `restore_backup` 各取一次写锁。锁序 `[1] BACKUP_OPERATION_LOCK → [2] 闸门（写）`；写入方 `cache → JSON_WRITE_LOCK → 闸门（读）` ⇒ 备份方只取闸门、写入方在闸门之下不再取锁，**无环路** | [atomic_file.rs](file:///g:/code-memory/src/atomic_file.rs)、[backup.rs](file:///g:/code-memory/src/backup.rs)、[persistence/json.rs](file:///g:/code-memory/src/persistence/json.rs) | CR-37 | ✅ 门禁 **886 passed / 0 failed**（+2）；两条 clippy 口径与 `cargo fmt --check` 全绿。**新增守卫** `snapshot_gate_excludes_writers_and_vice_versa`（`try_*` 非阻塞判定 ⇒ 不挂起、方向确定地证明互斥）+ `write_atomic_acquires_writer_guard`（源码级防回归）。**诚实边界**：跨**进程**的备份（另一进程在写同一数据目录）不受本闸门约束（闸门是进程内 `RwLock`，而跨进程写已由既有 `flock` 串行化） |


| **FIX-25** | **G5 第五步 / CR-39 已修（按报告许可路线：显式声明）**。勘察结论优先于报告描述：`persistence::neo4j::Neo4jGraphStore` **既不实现 `Persistence` trait，也没有任何调用方**（全仓 `create_memory_node` 仅定义未使用），且 [`subgraph`] 只消费节点的 `id` 与边类型 ⇒ 6 个属性是**够用的最小投影**，而非"漏写"。故不做"补 23 个字段"的伪修复（那会让图库变成**第二权威源**，与 G2「单一真源」相悖，且没有任何消费者需要）。改为在 `Neo4jGraphStore` 与 `create_memory_node` 上写**定位声明**：本后端是「关系图投影」、记忆完整字段由 JSON/Postgres/Qdrant 主后端承载、并列出**若未来提升为权威源的 3 条前置条件**（① 改存整条 `memory_json`，与 Qdrant 同款，禁止逐字段白名单；② 补 `privacy_level`/`user_id`/`session_id` 否则图查询复现 CR-03；③ 实现 `Persistence` + 接入管线 + 补测试） | [persistence/neo4j.rs](file:///g:/code-memory/src/persistence/neo4j.rs#L135-L360) | CR-39 | ✅ `CARGO_HTTP_CHECK_REVOKE=false cargo check --features server,neo4j` **Finished（exit 0）**。**诚实声明**：本项**无代码行为变更**（纯文档），因为按现状不存在可复现故障 —— 若你认为应把它真正接成权威源，请明示（那是独立特性，非本轮修复范围） |


| **FIX-26** | **G7 第一步 / CR-45 已修 —— 备份/恢复/迁移从「0 覆盖」到「10 条端到端用例」，并当场抓到 1 个真实缺陷**。<br>**为什么此前 0 覆盖（根因）**：`create_backup()` / `restore_backup()` / `scan_legacy_sources()` / `execute_migration()` 都把 **home 路径写死在函数体内** ⇒ 一测就会读写**真实**的 `~/.loong-recall/`（`restore_backup` 甚至会覆盖真实记忆库、`execute_migration` 会给真实迁移源打 `.bak`）。**这不是懒惰，而是"一测就毁数据"**。故先做**可测性拆分**：各函数拆出可注入路径/家目录的内核（`create_backup_in(data_dir, root)`、`restore_backup_in(snapshot, allowed_root, data_dir)`、`scan_legacy_sources_in(home)`、`execute_migration_in(home)`），公共入口保持原签名（零 API 变更）；并把 `data_log::log_operation` 移到公共入口 —— 否则测试仍会把运行记录写进**用户真实**的 `data_operations.log`。<br>**★ 新测试当场抓到的真实缺陷**：`MigrationReport.duplicates_skipped` 只在初始化时置 0、**从未被赋值**（全仓仅 3 处命中：定义、初始化、我的新断言）⇒ 对外 `/v1/migrate` 响应里"跳过多少重复"**永远是 0**，会误导调用方以为迁移没做去重。已修为按源累加。<br>**新增 10 条用例**：备份覆盖 `SNAPSHOT_FILES` 全清单（缺失落空文件）、缺 `memories.json` 必须**明确失败**且不留半成品、恢复逐字节写回、**路径穿越必须被拒且目标数据原封不动**、快照缺 `memories.json` 被拒、入口必须委托内核（源码级守卫）；迁移识别项目指纹/global 源、空家目录不得凭空造源、**legacy ∪ global 去重 + `.json.bak` 可回滚**、备份目标已存在必须预检拒绝 | [backup.rs](file:///g:/code-memory/src/backup.rs)、[migration.rs](file:///g:/code-memory/src/migration.rs) | CR-45 | ✅ 门禁 **896 passed / 0 failed**（+10）；`backup::` 14 passed、`migration::` 10 passed；两条 clippy 口径与 `cargo fmt --check` 全绿 |


> ⚠️ **审查清单可靠性备忘**：本次逐条复核累计发现**三处误报**（`synthesis_engine.rs` 文件不存在、`discovery.rs` 命中全在测试内、CR-23 对 v1 不成立）。
> 结论：**审查报告是线索，不是事实** —— 必须逐条核实后再动代码，否则会"修空气"并留下虚假的完成感。
> ⚠️ **反向亦成立**：报告"未提到的"缺陷同样真实存在 —— FIX-26 的 `duplicates_skipped` 恒为 0 就是新增测试当场抓到的（报告只说"0 覆盖"，没说"字段从不赋值"）。**覆盖缺口本身就是缺陷的孵化器。**

| **FIX-27** | **G7 第二步 / CR-47 + CR-48 已修 —— 覆盖假象与后端测试盲区**。<br>**CR-47（两项，其中一项报告描述已过时）**：① 报告称"5 个 ML 不变量套件**无模型即静默 no-op（CI 恒绿）**"—— 复核发现它们**早已标了 `#[ignore]`**（且 `#[ignore]` 文案已写明手动运行命令），所以"CI 恒绿"不成立；**真正残留的问题**是 `#[ignore]` 用例内部仍保留了 `eprintln!("…跳过"); return;` ⇒ 一旦有人跑 `--ignored` 而本机无权重，会**静默通过**（假覆盖）。已把 5 处静默 `return` 全部改为**带修复指引的 `panic!`**：既然调用方已显式请求 `--ignored`，"缺模型"就不是可接受的跳过。② 前端 6 个套件此前只有 5 个做了 `node --check`，**符号层专项 `symbolic-layer-desktop-cdp.js` 连语法检查都没有** ⇒ 已补齐（`ci.yml` 与 `release.yml` 双处），并在 CI 注释里写明为何其余 5 个 `*-cdp.js` 只做语法门禁（需真实桌面 WebView 的 CDP 9231/9230，GitHub runner 无此环境）与本地真实运行命令。<br>**CR-48**：qdrant / postgres / neo4j 这些"CI 只 `check` 不 `test`"的后端是 CR-07/08/17 那类缺陷的温床。现把 **qdrant 用例纳入 CI 与 release 门禁**（`cargo test --features server,qdrant --lib`，**不依赖外部服务**，本地实测 902 passed）；postgres/neo4j 的运行时往返需真实服务，其**结构性风险**（字段清单 / SQL 生成）已由 G2 的 `persistence::fields` **5 条免连库守卫**按构造覆盖 | [ci.yml](file:///g:/code-memory/.github/workflows/ci.yml)、[release.yml](file:///g:/code-memory/.github/workflows/release.yml)、`tests/state_*.rs` | CR-47、CR-48 | ✅ 6 个前端套件 `node --check` 全通过；`cargo check --features server,ml --tests` Finished（确认 `panic!` 改动可编译）；`cargo test --features server,qdrant --lib` **902 passed / 0 failed**；默认门禁 **896 passed / 0 failed**；两条 clippy 口径与 `cargo fmt --check` 全绿。**残留（显式声明）**：postgres(1)/neo4j(0) 的**运行时**往返仍无 CI 覆盖（需 service container，本轮未加 —— 不引入无法本地验证的红门禁） |


| **FIX-28** | **G10 第一步 / CR-51 已修 —— 文档版本漂移对齐**。修前 `README.md`（下载链接、安装包文件名、"版本特性"标题共 6 处）与 `docs/USER_GUIDE.md`（版本行、安装包表格、CLI 提示共 4 处）仍写 **v0.9.9**，而 `Cargo.toml` / `tauri.conf.json` / `package.json` 均为 **0.9.10**（v0.9.10 已实际发布）⇒ 用户按文档下载会**指向不存在的 tag**。全部对齐到实际发布版本 **v0.9.10**；`USER_GUIDE.md:377` 的"v0.9.9 已修复此问题"是**历史陈述**，按语义保留（不改成 v0.9.10，否则变成事实错误）。`CHANGELOG` 顶部 `## [0.9.11] - 未发布` **保持不动** —— 它准确描述了本轮尚未发布的开发内容，且 `release.yml` 的版本一致性门禁会从 CHANGELOG 首个版本头提取版本号 | [README.md](file:///g:/code-memory/README.md)、[USER_GUIDE.md](file:///g:/code-memory/docs/USER_GUIDE.md) | CR-51 | ✅ 全仓 `0.9.9` 仅剩 1 处历史陈述（USER_GUIDE:377，有意保留）；版本三处（Cargo/tauri/package）= 0.9.10 与文档一致 |

| **FIX-29** | **G10 第二~九步 / CR-50、CR-52、CR-53、CR-54、CR-55、CR-56、CR-57、CR-58 已修（文档与契约治理）**<br>**① CR-50（P1，唯一的 P1）**：`scripts/validate_frontend_contract.js` 此前只对齐「路径 + 方法 + invoke 名」，**完全不校验响应字段** ⇒ `body.memories` vs `body.data` 类漂移在 CI 中不可见。新增**检查 5：响应字段契约** —— 解析 `src/v1_api.rs` 响应结构体的**对外序列化字段**（尊重 `#[serde(rename)]`），对契约表逐项断言：后端含该字段 **且** 前端确实消费该 key（双向）。**鉴别力已实测**：注入伪字段 `bogus_field_xyz` ⇒ 门禁红并打印该结构体当前全部对外字段。<br>**② CR-52**：`LICENSE` §2.2（只举 3 个文件名）／`src/lib.rs` 头部（十余个模块标 Layer 1）／`check_algorithm_leak.py`（4 文件扫描面）**三方口径互不重合** ⇒ 统一为**目录级**定义（`src/engine/` 之外 = 公开层 Apache；之内 = 受保护层），并明确 **`PUBLIC_FILES` 是"泄露扫描面"、不是许可边界**；脚本内加**自检断言**（扫描面不得含 `src/engine/` 文件）。<br>**③ CR-53**：`SECURITY.md` 两处失真 —— 支持版本表只到 v0.8.x（已对齐 v0.9.x）；"所有 Actions 均钉 SHA"**声明为假**（`actions/setup-python@v5` 是可变标签，全仓唯一例外）⇒ **实际钉住 SHA**（经 GitHub API 取 `v5` tip commit `a26af69b…`），使声明为真，并写明复查方式。实测：全仓 `uses: …@vN` **0 命中**。<br>**④ CR-54**：README:264 与 USER_GUIDE:57 称"道体推演引擎不随产品分发"，与 README:452「`src/engine/` 道体模块**会**随包分发」**自相矛盾** ⇒ 术语澄清为"不随包的是**独立推演服务**（`daoti_daemon`/`daoti_assoc`），`src/engine/` 源码随包编译"。<br>**⑤ CR-55**：`CHANGELOG` 重复版本头（`[0.9.8]`×3、`[0.8.23]`×2）⇒ 版本级标题 `## [x.y.z]` 降级为 `### x.y.z 同版本补充 · 日期`，并在文件头写明约定（工具按 `^## \[` 提取版本号，已复验仍取到 `[0.9.11]`）；README 语义底座写 `bge-base-zh/768` 与实际默认 `bge-small-zh/512` 不符 ⇒ 已更正并保留高精度选项。<br>**⑥ CR-56**：`OFFLINE_MODEL_GUIDE` 把 `LRC_LUOSHU_MODEL_ID`（主语义编码器）与 `LRC_MODEL_ID`（仅旧版 CodeBERT 代码检索）**并列**导致误用 ⇒ 新增对照表明确二者子系统与代码位置、声明互不覆盖；并补 `Cargo.toml` 的 `default-run = "code-memory-server"` —— 此前**两个 `[[bin]]` 而无 `default-run`**，使指南里的 `cargo run --features server,ml --` **必然失败**（`cargo metadata` 实测确认 `default_run` 已生效）。<br>**⑦ CR-57**：`#crystallization-timeline` 容器早已移除 ⇒ `loadCrystallizationHistory` 首行即 `return`，属**死代码**。已删除该函数、3 处调用点、`window` 导出、专用 `crystallizationHistoryRequest` 变量，以及只服务它的 6 条 `.crystallization-*` CSS 规则；`index.html` 两处注释改为**按函数名引用**（原引用 `app.js:8310` 属行号引用，会随增删漂移）。<br>**⑧ CR-58**：核实了报告标注"未核实"的问题 —— `ARCHITECTURE.md`(v0.8.25) 与 `ARCHITECTURE_OVERVIEW.md`(v0.9.7) **不是矛盾真源**（抽检 MCP 工具数一致=13），但版本基准不同 ⇒ 两份文件各加**文档关系声明**（分工 + 以源码为准 + 合并计划）；并按"**历史快照 + 零引用**"双判据把 **14 份**历史审计/修复计划移入 `docs/archive/`（移动前用全仓 Grep 证明零引用，移动后仅 archive 内部自引用，并修正其中 1 条失效跨文档链接），新建归档索引说明判据与指路 | [validate_frontend_contract.js](file:///g:/code-memory/scripts/validate_frontend_contract.js)、[check_algorithm_leak.py](file:///g:/code-memory/scripts/check_algorithm_leak.py)、[LICENSE](file:///g:/code-memory/LICENSE)、[lib.rs](file:///g:/code-memory/src/lib.rs#L5-L17)、[SECURITY.md](file:///g:/code-memory/SECURITY.md)、[ci.yml](file:///g:/code-memory/.github/workflows/ci.yml)、[CHANGELOG.md](file:///g:/code-memory/CHANGELOG.md)、[OFFLINE_MODEL_GUIDE.md](file:///g:/code-memory/docs/OFFLINE_MODEL_GUIDE.md)、[Cargo.toml](file:///g:/code-memory/Cargo.toml)、[app.js](file:///g:/code-memory/static/app.js)、[app.css](file:///g:/code-memory/static/app.css)、[index.html](file:///g:/code-memory/static/index.html)、[ARCHITECTURE.md](file:///g:/code-memory/docs/ARCHITECTURE.md)、[docs/archive/README.md](file:///g:/code-memory/docs/archive/README.md) | CR-50、CR-52~58 | ✅ 契约脚本**通过**并新增「7 个响应字段与后端结构体对齐」计数；**鉴别力实测**（注入伪字段 ⇒ exit 1 + 精确诊断）；`node --check static/app.js` 通过、全仓 `loadCrystallizationHistory` 0 残留；`check_algorithm_leak.py` 通过；`cargo metadata` 确认 `default_run=code-memory-server`；`docs/` 132→118 份（14 份归档）；`grep '^## \[' CHANGELOG.md` 仍取到 `[0.9.11]`。**残留（显式登记）**：CR-58 的"两份架构文档**合并**为单一真源"属独立文档工程，本轮只做了关系声明与归档，未合并 |

| **FIX-30** | **G9 / CR-15 + CR-24 + CR-25 + CR-27 + CR-33 已修 —— 结构单一真源（含一次真实的依赖倒置）**<br>**① CR-15（P1，本轮唯一"真架构题"）**：先用证据修正了报告口径 —— 实测 **L1→engine 有 122 处/14 文件**（架构必需、且是唯一有许可含义的方向），**engine→L1 仅 7 处/3 文件**。据此按"只动窄的一侧"实施：<br>　**(a) 依赖倒置**：`navigated_deep_recall<P>(store: &mut crate::memory_store::MemoryStore<P>) where P: crate::persistence::Persistence` → 引擎**自声明** `pub trait FocusRecall { type Error; fn focus_recall(...) }`，由 `MemoryStore` 在 Layer 1 **反向实现** ⇒ 依赖箭头反转为 `MemoryStore → 引擎接口`，引擎**不再引用** `memory_store` 与 `persistence`。（用关联类型而非具体错误类型：否则引擎仍要 `use crate::persistence`，只是换名。）<br>　**(b) 改依赖纯数据契约**：`engine/rrf.rs`、`engine/navigation.rs` 的 `RecallResult`/`RecallFilter` 由 `crate::memory_store::`（实现模块）改指 `crate::memory_store_types::`（**纯数据契约模块**）—— 后者本就是被外提出的契约层，`memory_store` 只是 `pub use` 再导出。<br>　**(c) 防扩散守卫**：`engine/mod.rs` 新增**目录扫描**测试 `engine_must_not_depend_on_layer1_implementations`（新增文件自动纳入；注释行不计），禁止 engine 引用 `memory_store`/`v1_api`/`server`/`consolidation`/`dashboard`/`export`/`backup`/`migration` 等**实现模块**。<br>　**残留（显式登记）**：`engine/synthesis_engine.rs` 仍引 `graph_store`（图存储类型）与 `persistence`（`Persistence` trait）—— 彻底解耦需再加一层抽象（把 trait/类型再提一层或用泛型反转），属独立工程；故**不在**守卫禁止清单内，以保持"残留可见"。**同句声明**：单 crate + 目录级分层下，engine 不可能"零依赖兄弟模块"（它必须用 `Memory`/`LrcError`），故"引擎可独立授权分发"的前提是**另建契约 crate**，非本轮范围。<br>**② CR-24（合成阈值无单一真源）**：新增 `DEFAULT_MIN_CLUSTER = 3` / `DEFAULT_SIMILARITY = 0.4` 单一真源，替换 4 处字面量（`memory_store.rs` 构造体 ×2、`arch_config.rs`、`dao_regulator.rs`），并 `SynthesisConfig::default()` 改为引用常量。**顺带暴露一处被埋起来的既有分歧**：`ConsolidationConfig::default()` 的阈值是 **5** 而 sidecar 显式覆写为 **3** —— 修前是三个裸数字谁也无法判断意图，现各自命名化并把"有意不同"写成可读声明（不擅改行为）。<br>**③ CR-25（构造函数逐字重复）**：`new_statistical` 由"逐字复制 `new()` 近 40 行"改为 `Self::new(..)` + 覆写 3 个字段，与同文件 `new_with_encoder` 同一模式 ⇒ 消除"两个构造函数造出的 store 不一致"这类静默差异。<br>**④ CR-27（同名类型两套 API）**：`HybridLuoShuEncoder` 在 ml 下是结构体、非 ml 下是 `LuoShuEncoder` 的别名，方法集不同。已在非 ml 侧补齐**特性中立最小集**（`new_statistical`/`is_ml_mode`/`encode_embedding`/`is_degraded`/`recovery_progress`/`set_recovery_threshold`/`record_degradation`，一律给出诚实的"无"答案），并加**双配置编译期守卫** `feature_neutral_encoder_api_is_uniform`（两个 cfg 分支调用同名方法 ⇒ 任一侧缺方法即编译失败）。<br>**⑤ CR-33（桌面端 crypto 漂移）**：`desktop/src-tauri/src/crypto.rs` 的 DPAPI 失败文案从"密钥可能来自其他用户或机器"（可推断密钥来源）改为与主 crate 一致的**不可区分**文案 + stderr 诊断，并加守卫 `dpapi_failure_message_must_not_leak_key_origin` | [navigation.rs](file:///g:/code-memory/src/engine/navigation.rs)、[rrf.rs](file:///g:/code-memory/src/engine/rrf.rs)、[engine/mod.rs](file:///g:/code-memory/src/engine/mod.rs)、[memory_store.rs](file:///g:/code-memory/src/memory_store.rs)、[synthesis_engine.rs](file:///g:/code-memory/src/engine/synthesis_engine.rs)、[consolidation.rs](file:///g:/code-memory/src/consolidation.rs)、[arch_config.rs](file:///g:/code-memory/src/arch_config.rs)、[dao_regulator.rs](file:///g:/code-memory/src/engine/dao_regulator.rs)、[luoshu_encoder.rs](file:///g:/code-memory/src/engine/luoshu_encoder.rs)、[crypto.rs](file:///g:/code-memory/desktop/src-tauri/src/crypto.rs) | CR-15、CR-24、CR-25、CR-27、CR-33 | ✅ 默认门禁 **898 passed / 0 failed**（+2 守卫）；`cargo test --features server,ml --lib feature_neutral_encoder_api_is_uniform` **1 passed**（证明 ml 侧分支的断言也成立，而非仅能编译）；桌面端 `cargo test --lib crypto::` **6 passed**（含新守卫）；`cargo check --features server,ml --tests` Finished；**三条 clippy 口径**（`--features server` / `--all-targets` / `--all-targets --features server,ml`）与 `cargo fmt --check` 全绿。⚠️ **本轮三次踩到"守卫自匹配"**（`include_str!` 把测试自己的断言串也扫进去 ⇒ 恒真/恒假），已分别为 `atomic_file.rs`、`backup.rs`、`engine/mod.rs`、桌面 `crypto.rs` 的守卫修正断言范围（截到测试模块之前，或拼接构造规避字面量） |


| **FIX-32** | **收尾第二批次：CR-42 + CR-35 + CR-31 已修，CR-41 已显式声明**。<br>**① CR-42（P2，跨进程缓存不失效）**：`MemoryStore` 的缓存只在**自身**写后置脏 ⇒ 多窗口/CLI 与 sidecar 并存时别的进程写入收不到通知，可能**无限期读陈旧数据**。修法：为 `Persistence` 增加**可选**指纹 `source_fingerprint() -> Option<u64>`（默认 `None` ⇒ 各后端零改动、行为不变）；`JsonPersistence` 实现为**记忆文件 `mtime`（纳秒）旋转后异或长度**（廉价、远低于全量读）；`MemoryStoreCache` 记录"加载时的指纹"，`needs_reload()` 从"只看 dirty"升级为"dirty **或** 指纹变化"。**鉴别力已实测**：临时把 `needs_reload` 退回"只看 dirty" ⇒ 新测试 `cache_sees_writes_made_by_another_process` **失败（1 vs 2）**，恢复即通过 —— 证明测试真能抓住该故障而非恒真。<br>**② CR-35（P2，内存峰值）**：写路径上的 `previous = memories.clone()`（整库深拷贝，仅用于写失败时回滚）**被移除**：`atomic_write` 本身是原子的 ⇒ 失败时磁盘仍是旧状态，改为**丢弃缓存、下次读取从磁盘重载**（语义等价，峰值更低）。`save_memory` 与 `update_memories` 两处均已改（含序列化失败分支）。**诚实边界**：这是峰值主因之一，但**未做端到端内存实测**（需在 2GB 机器上采集），故不宣称"峰值已达标"。<br>**③ CR-31（P2）**：即"`actions/setup-python` 未按 SHA 固定"——**与 CR-53 同一根因，已由 FIX-29③ 一并修复**（现已钉 SHA，全仓 `uses: …@vN` 0 命中）。<br>**④ CR-41（P2，写放大）**：核实后属**被接受的设计取舍**（后端定位本就声明为 `< 10,000 条`），故按"显式声明"路线处理：把量化证据（4,485 条 mean≈308ms / p95≈324ms）、既有缓解措施（批量路径）与**超出规模的正确做法（换 postgres/qdrant/neo4j 内置 feature，而非自研分片 JSON）**写进 `json.rs` 模块头，使"隐患"变成"知情决策" | [persistence/mod.rs](file:///g:/code-memory/src/persistence/mod.rs)、[persistence/json.rs](file:///g:/code-memory/src/persistence/json.rs)、[memory_store_cache.rs](file:///g:/code-memory/src/memory_store_cache.rs)、[memory_store.rs](file:///g:/code-memory/src/memory_store.rs)、[memory_store_tests.rs](file:///g:/code-memory/src/memory_store_tests.rs) | CR-42、CR-35、CR-31、CR-41 | ✅ 门禁 **899 passed / 0 failed**（+1 CR-42 用例）；三条 clippy 口径（`--features server` / `--all-targets` / `--all-targets --features server,ml`）与 `cargo fmt --check` 全绿；**鉴别力实测**（关掉指纹判定 ⇒ 用例失败 1 vs 2） |
| **FIX-31** | **收尾：CR-49 + CR-40 已修**。<br>**① CR-49（P3，测试假绿）**：`logo_asset_handler` / `icon_asset_handler` 的"有效文件名"用例**只断言 `200`**，而注释却声称"应返回 200 **和 SVG 内容**" ⇒ 处理器"返回 200 + **空体**"也能通过，"图标/Logo 全白"这类真实故障在测试里不可见。现补两条断言：**MIME 必须是 `image/svg+xml; charset=utf-8`** + **响应体非空且含 `<svg`**。<br>**② CR-40（P2，持久化加固）**：`atomic_file::write_atomic` 修前是 `fs::write` + `rename`，两者**落盘顺序无保证** ⇒ 断电/崩溃后可能出现"目录项已指向新文件名、内容仍在页缓存"⇒ `memories.json` 变 **0 字节 = 全部记忆丢失**。现改为 **写入 → `sync_all()` → rename → 同步父目录（Unix）**。**并顺带收敛重复实现**：`persistence/json.rs::atomic_write` 原本自带**第二份**"临时文件 + rename"实现（与公共设施重复）—— 两份实现意味着"加固只做一半"（本轮若只改一处，主记忆写路径就拿不到 fsync）。现改为**委托** `atomic_file::write_atomic`（锁序仍保留 `JSON_WRITE_LOCK`；④ 闸门由被委托方获取，**避免同线程递归取读锁**） | [server.rs](file:///g:/code-memory/src/server.rs)、[atomic_file.rs](file:///g:/code-memory/src/atomic_file.rs)、[persistence/json.rs](file:///g:/code-memory/src/persistence/json.rs) | CR-49、CR-40 | ✅ 门禁 **898 passed / 0 failed**；三条 clippy 口径（含 `--features server,ml`）与 `cargo fmt --check` 全绿 |


| **FIX-33** | **收尾第三批次：CR-32 + CR-34 + CR-36 + CR-43 + CR-44 已修（5 项实修 + 2 项显式声明 + 1 项早已修）**<br>**① CR-32（P3，隐式契约）**：`build_v1_router_with_trust` 此前**从不设置** `DefaultBodyLimit`，请求体上限由 axum 框架默认值（2 MiB）**隐式**决定 ⇒ 该对外契约不在本仓、无法断言、随手一次 `cargo update` 即可能漂移。新增 `pub const V1_MAX_REQUEST_BODY_BYTES = 2 * 1024 * 1024`（**取值与框架默认一致 ⇒ 对外行为零变化**）并在路由链尾 `.layer(DefaultBodyLimit::max(...))`（对**全部**路由生效；超限在进入 handler 前即 **413**）。**两条用例**：行为用例（超限 ⇒ 413、正常 ⇒ 非 413）+ **源码级守卫** `v1_router_must_explicitly_cap_request_body`（**逐行比对**而非子串包含，防"注释掉即绕过"）。**鉴别力实测**：删除该行 ⇒ 守卫 **FAILED**，而行为用例**仍通过**（因框架默认同为 2 MiB）—— 正说明源码守卫不可省。<br>**② CR-34（P2，取锁无 deadline）**：explore 的取锁循环此前**无独立 deadline**（仅 50ms 轮询 + 取消标志），锁被长任务握住时会一路自旋到外层 15s。现补齐与 `timeouts::LOCK_WAIT` 同口径的 deadline，超时 ⇒ 503 `search_busy`。**关键收益**：锁等待限界在 **2s** ⇒ 最坏路径 `2s 锁等待 + 10s 探索预算 = 12s < 外层 15s`，使 v0.9.9 设计的"到点**优雅收敛**（200 + interrupted）"**在锁忙时重新可达**（修前不可达）。既有 `test_assoc_p82p_concurrency_pressure` 的确定性兜底段随之改判（原断言 `explore_timeout`@15s ⇒ 现断言 `search_busy` 且耗时 **< 15s**），用例文档一并更正。<br>**③ CR-36（P3，持锁全量列举）**：explore 取 root 时用 `list_memories(limit = usize::MAX)` **在持锁路径内全量列举 + 克隆记忆**，而目标只是"取一条"。改用既有定向访问器 `MemoryStore::memories_by_ids(&[id])`（同样只读、不触发写回，但**不克隆无关记忆**、**不按 limit 截断** ⇒ 亦消除"目标 id 排在 limit 之外被漏掉"的隐患）。<br>**④ CR-43（P2，serde 默认值语义）**：`Memory::version` 的 `#[serde(default)]` 会把"字段缺失"静默降级为 `0`，而 `0` 在本域**不是合法版本号** ⇒ 与"真·初版"混淆、`version + 1` 还会撞号。改为命名默认 `default_version() -> 1`。用例 `memory_version_defaults_to_one_not_zero`；**鉴别力实测**：退回裸 `#[serde(default)]` ⇒ FAILED（left 0 / right 1）。`resolution` 的 `"detailed"` 默认**有意保留**（旧格式兼容所需，且缺省即历史语义）。<br>**⑤ CR-44（P3，持久化加固，5 子项）**：<br>　**(a) 实修**：`persist_hidden` 由非原子 `std::fs::write` 改为委托 `atomic_file::write_atomic`（临时文件 → fsync → rename），消除"崩溃留半截文件 ⇒ 部分隐藏 ID 丢失、记录重启后复活"。<br>　**(b) 实修**：`SynthesisJournal` 由**三个独立 `Mutex`**（`events` / `total_synthesis` / `low_quality_count`）收敛为**单一 `Mutex<JournalState>`** ⇒ `snapshot()` 一次加锁即得**内部一致**视图（修前三次独立取锁，可读到 `low_quality_count > total_synthesis` 这类**自相矛盾**组合），并消除"events → low_quality"嵌套加锁；守卫 `synthesis_journal_must_use_single_lock`。<br>　**(c) 早已修**：Postgres DDL `DEFAULT '{{}}'` 笔误**已由 FIX-06 修复**（今在 `persistence/fields.rs`；`format!` 中 `{{}}`→`{}` 本就是正确空对象，报告 `postgres.rs:184/186` 行号系改动后漂移）。<br>　**(d) 实修**：`export.exported_at` 语义与注释不符 —— 修前写的是 **Unix 秒数字符串**，而字段文档与本文件 V2 格式说明均声明 **ISO 8601**、前端 `app.js` 同名字段用 `toISOString()` ⇒ 改为 `chrono::Utc::now().to_rfc3339()`（**后端（文档+代码）与前端三方对齐**）。<br>　**(e) 显式声明（非缺陷）**：`Memory` **不引入**逐条 `schema_version`（格式版本由导出信封 `ExportData.version` 承载；结构化后端由 `persistence::fields` 字段矩阵守卫按构造保证列—写—读一致；逐条版本号会**引入第二真源**）；`SynthesisJournal` 保持**进程内诊断数据、不持久化**（重启归零为既有设计）。 | [v1_api.rs](file:///g:/code-memory/src/v1_api.rs#L6110-L6112)、[v1_api_tests.rs](file:///g:/code-memory/src/v1_api_tests.rs)、[synthesis_journal.rs](file:///g:/code-memory/src/engine/synthesis_journal.rs)、[memory_types.rs](file:///g:/code-memory/src/memory_types.rs#L329-L348)、[audit_trail.rs](file:///g:/code-memory/src/engine/audit_trail.rs#L1349-L1370)、[export.rs](file:///g:/code-memory/src/export.rs#L160-L170)、[persistence/fields.rs](file:///g:/code-memory/src/persistence/fields.rs#L60-L70) | CR-32、CR-34、CR-36、CR-43、CR-44 | ✅ 门禁 **903 passed / 0 failed**（+4）；`cargo clippy --features server --all-targets`、`cargo clippy --all-targets --features server,ml` 与 `cargo fmt --all -- --check` **全绿**；**鉴别力实测两处**（删 `DefaultBodyLimit` 行 ⇒ 源码守卫红；退回裸 `serde(default)` ⇒ version 用例红） |

| **FIX-34** | **收尾第四批次：CR-11 最后 2 处无上限取锁已修 + CR-10 结构侧残留显式登记**。<br>**① CR-11 收尾（13/13 处）**：`update_llm_config`（供 `/api/config/llm` 与 `/v1/config/llm` 共用）里还剩 **2 处** `memory_store.lock().await`（FIX-12 明确登记的遗留点）。二者只是把 `set_llm_configured(bool)` 缓存标志同步一下，却用**无上限** `lock().await`：后台长任务持锁时该 handler **无限排队**（与 v1/MCP 口径不一致）。现改为 `lock_store_bounded()`（`timeout(LOCK_WAIT, …)`），且**超时不把"配置已生效"改成失败**——只记一行告警，因该标志仅是缓存、下次配置变更自收敛。<br>**② 源码级守卫 `mcp_paths_must_not_use_unbounded_store_lock`**：扫描 `server.rs` **生产段**（截到 `#[cfg(test)]` 之前，并**跳过注释行**——文档注释里正当地引用了该反例写法），断言不存在 `memory_store.lock().await`。**鉴别力实测**：把其中一处改回无上限写法 ⇒ 守卫 **FAILED** 并打印精确 offender 行。<br>**③ CR-10 结构侧残留（显式登记，属独立优化）**：真正把"**编码**移出锁"需要把 `luoshu_encoder` 以 `Arc` 形式交出（当前被 `MemoryStore` 私有持有，锁外无从取用），并把"逐条 `remember` → 全量 JSON 重写"换成一个**保留相似合并语义**的批量写 API（**不能**复用 `remember_batch`——它跳过合并、会改语义）。这是"新 API + 语义保持"的独立工程；其**用户可见后果**（后台独占锁 ⇒ 前台 503）已由 FIX-11 的"有界取锁 + 分批让锁 + 取消自查"实际缓解。**同句声明**：`consolidation.rs` 其余后台取锁点（`llm_synthesize_cycle` 的 Phase1/Phase3、`synthesis_pending` 标志位翻转）经复核属"三阶段锁安全模式"的**短锁段**（重活都在无锁 Phase2），不在本轮改造范围；其中 Phase 3 内的"一次性全库列举建索引"与 CR-36 同源，已随本轮结论登记 | [server.rs](file:///g:/code-memory/src/server.rs#L3892-L3900)、[server.rs](file:///g:/code-memory/src/server.rs#L3983-L3990)、[consolidation.rs](file:///g:/code-memory/src/consolidation.rs#L370-L457) | CR-11、CR-10 | ✅ 门禁 **904 passed / 0 failed**（+1 守卫）；`cargo clippy --features server --all-targets`、`cargo clippy --all-targets --features server,ml` 与 `cargo fmt --all -- --check` **全绿**；**鉴别力实测**（改回无上限取锁 ⇒ 守卫红并报出 offender 行） |

| **FIX-35** | **AML 评估暴露的两处真实缺陷（收尾第五批次；非比赛专属，均为通用能力缺陷）**<br>**① 合成产物不可见（派生记忆可达性）**：`synthesis_engine` 构造合成记忆时只继承 `project`，`privacy_level` 落到默认 `User` 而 `user_id` 仍为 `None` ⇒ `is_visible` 对 `(Some(uid), None)` 判 **false** ⇒ **对任何 user 都不可见**。真实数据实测：**212/212** 条合成记忆全部不可见 —— 即"合成写进去等于消失"，依赖全局综合/摘要的能力（如 F1）**恒为 0**。**修法**：在 Layer 1 唯一写回口 `apply_synthesis_plan` 用 `crate::privacy::inherit_privacy(来源)`（与 CR-04 **同一真源**）回填三元组；**三态 fail-closed**（匿名来源"无约束"原样通过（与 `remember` 匿名分支同口径）／跨可见性域或来源不可解析 ⇒ **放弃该产物**，绝不回退为全可见），并跳过已放弃条目的图边与合成日志。回归用例 `synthesized_memory_must_inherit_source_visibility`（含跨 user 零泄漏反面对照）。**端到端实测**：Search 出口可见的合成条数 **0 → 13/11**，且**库内数 == 出口可见数** ⇒ 使能链（产出→可见→出口）打通。<br>**② REST `enrich` 与 MCP `recall` 的检索能力不对齐**：MCP 两处 handler 均做"关键词 + 原查询"富化（`translate_memory_query`），而 REST `POST /v1/memories/enrich`（桌面端与 AML 实际使用的检索入口）**完全没有**这一步 ⇒ 同一引擎两条入口检索质量不一致。**修法**：抽成单一真源 `engine::llm_translator::enrich_query_for_retrieval`，三处共用；REST 侧在**取存储锁之前**调用（网络等待不得落入持锁期）；未配置 LLM ⇒ **逐字返回原查询**（确定性单测 `enrich_query_is_identity_when_llm_not_configured` 钉住"零行为变更"）＋ 源码级守卫 `enrich_endpoint_must_reuse_llm_query_enrichment`。<br>**附：一项被实测否定的候选优化**（避免后续重复投入）：给 `associated`（记录层联想补全）预留导出槽位对 gold 召回**零贡献**（`k=0/5/10/20` 命中完全相同），且在"库容量 ≥ `top_k`"时结构上必然被 `data[:top_k]` 截掉 | [memory_store.rs](file:///g:/code-memory/src/memory_store.rs#L2866-L2990)、[llm_translator.rs](file:///g:/code-memory/src/engine/llm_translator.rs#L538-L570)、[v1_api.rs](file:///g:/code-memory/src/v1_api.rs#L2193-L2215)、[server.rs](file:///g:/code-memory/src/server.rs#L901-L915)、[memory_store_tests.rs](file:///g:/code-memory/src/memory_store_tests.rs#L1817-L1895)、[v1_api_tests.rs](file:///g:/code-memory/src/v1_api_tests.rs#L6626-L6645) | 新增（由 AML 离线 A/B 暴露） | ✅ 门禁 **907 passed / 0 failed**（+2）；`cargo clippy --features server --all-targets`、`--all-targets --features server,ml` 与 `cargo fmt --all -- --check` **全绿** |

> ⚠️ **FIX-01 是本次审查最有价值的产出之一**：它证明"`cargo test` 全绿"**不等于**"门禁会过"。
> ⚠️ **FIX-02 只覆盖了模块内**，跨模块竞态仍存在（见 CR-20），不可当作已根治。

---

## 七、已核实为「非问题」的项（防重复报）

| 项 | 结论 | 依据 |
|---|---|---|
| **默认 JSON 后端字段白名单** | ✅ 无丢失，无 `skip_serializing` 隐藏丢弃 | `memory_types.rs` + `persistence/json.rs` |
| 历史 C05「clear+save 丢全库」 | ✅ 已修复（写路径收敛为单端点原子写） | `memory_store.rs:3874/5181/7236` |
| 「审计清除后重启复活」 | ✅ 已修复（改为 `hide_matching` + 隐藏集合 + 回归测试） | `audit_trail.rs:1293-1369` |
| `unsafe` soundness | ✅ 未发现 UB 面（全部 Windows FFI + `SAFETY` 说明） | `crypto.rs`/`guard.rs`/`process_guard.rs`/`tray.rs` |
| SSRF 防护 | ✅ 完善（编码 IP/八进制/十六进制、userinfo、云 metadata、DNS rebinding fail-closed） | `url_safety.rs` |
| 路径遍历 | ✅ `canonicalize` + `starts_with(allowed_root)` | `backup.rs:297-343` |
| 加密 | ✅ AES-256-GCM + 随机 nonce + 随机主密钥 + DPAPI，无硬编码密钥/IV 复用 | `crypto.rs` |
| CORS | ✅ 显式白名单、`allow_credentials(false)`、已移除 `0.0.0.0` | `server.rs` |
| token 比较 | ✅ 已修为常量时间 `constant_time_eq` | `server.rs:4754-4763` |
| 日志泄露 | ✅ 未发现打印 token/密钥明文；`/v1/config` 不含 `api_key` | 维度三 |
| 测试鉴别力 | ✅ **亮点**：多处以 `persistence().load_all_memories()` 直读磁盘防"内存读回假绿"，并带负向对照与门控配对用例 | `v1_api_tests.rs:4756-4768`、`memory_store_tests.rs:1439` 等 |
| 前端路径/方法契约 | ✅ 42 条 API 路径 + 33 个 invoke 命令全对齐，无孤儿调用 | `validate_frontend_contract.js` 实测 |
| M0–M5 仪表盘元素 | ✅ `#value-hero`、`renderMemoryAssets`、`formatRelativeTime`、`bindAdvancedPanelLazyLoad` **全部真实存在且被调用** | `index.html:182`、`app.js:7626/7304/9055` |
| CDP 套件断言性质 | ✅ 同时断言"元素存在"与"行为正确"，非只测存在 | `tests/frontend/*.js` |
| 桌面端命令面 | ✅ 37 条 `#[tauri::command]` == 37 条注册；CSP 与权限自洽 | `main.rs:134-185`、`tauri.conf.json:28` |
| `#[allow(dead_code)]` | ✅ 多处带"移除实验已实测非冗余"的核实注释，非过时豁免 | `consolidation.rs:168-172` 等 |
| 注释与实现一致性 | ✅ 抽查 v0.9.10/v0.9.11 标记的最近改动，**未发现注释与代码矛盾** | 维度一 |
| TODO/FIXME 债 | ✅ 0 处真实标记 | 维度一 |

---

## 八、未核实项（诚实声明）

1. 生产路径 `unwrap` 的**逐条**可达性（本次仅按"是否在 `#[cfg(test)]` 内"批量核对，`server.rs` 25 处未逐条判定）。
2. 真实**内存峰值**与 OOM 触发阈值（缺运行时实测）。
3. `process_guard` 的 `tasklist` 子进程阻塞时长（子进程调用且无超时）。
4. CR-48 所述 `--features postgres,qdrant,neo4j` 的实际编译/运行状态（CI 未覆盖，本次未跑）。
5. CR-52 中"`v1_api.rs` 若纳入漏泄扫描是否会报 error"的**实测结论**（仅按脚本 RULES 静态推断）。
6. CR-41 的性能量化数据（引自仓库内既有剖析记录，本次**未重跑实测**）。
7. `docs/ARCHITECTURE.md` 与 `docs/ARCHITECTURE_OVERVIEW.md` 是否内容冲突。
8. 前端 42 条 API 路径中**非高流量端点**的逐字段核对（本次字段级比对覆盖 recent/stats/list/enrich/consolidate/remember/captains-log/trust 等）。

---

## 九、结论

### 9.1 总体评价

**工程规范与安全基础扎实**：文件级许可证边界可审计、`unsafe` 全带说明、SSRF/路径/加密/CORS 到位、注释承载设计推理且与实现一致、测试鉴别力显著高于同类项目（直读磁盘防假绿 + 负向对照 + 门控配对）。**v0.9.7 报告的一批 P0 已确认闭环**（第七节）。

### 9.2 核心问题定位

1. **隐私隔离模型是 fail-open，且 MCP 通道完全未接入**（CR-01/CR-02）——这是本次**最严重**的发现：写入侧的 `privacy_level`/`user_id` 是"真的"，读取侧却在两条主通道上把它忽略了。**风险等级最高、修复成本最低**。
2. **`panic="abort"` 让整套 panic 隔离成为话术**（CR-06）——一处配置使多处"服务已保持运行"的承诺在发布产物上不成立，属**认知与实现脱节**。
3. **超时阶梯被打破，优雅收敛成为不可达代码**（CR-05/CR-14）——与 v0.9.7 报告记录的同类问题**同源复发**，说明缺少"内层 < 外层"的自动化护栏。
4. **测试竞态的修复方式是"按模块局部修"**（CR-20）+ **三处门禁口径不一致**（CR-21）——竞态因此可长期潜伏并只在 CI 之外暴露。
5. **可选持久化后端的字段白名单不完整**（CR-07/CR-08/CR-17）——命中本项目历史上"字段白名单漏字段 ⇒ 静默假绿"的已知故障模式，且因 CI 不跑这些 feature 而**无门禁可拦**。

### 9.3 风险分布

| 级别 | 数量 | 其中主控已复核 |
|---|---|---|
| P0（默认配置可触发） | 4 | 4 |
| P0（限可选 feature） | 2 | 1 |
| P1 | 18 | 0（来自维度审查） |
| P2 | 19 | 0 |
| P3 | 13 | 0 |
| **合计** | **56** | **5** |
| 本轮已当场修复 | 2（FIX-01/02） | — |

---

## 十、跟踪清单（后续按此推进）

> 使用方式：修复后在本表回写「状态 = 已修复（版本/提交）」；新增风险按 `CR-59…` 续编。

### 第一梯队（P0）

> **全部闭环**（v0.9.11 全局修复轮，逐项配防回归守卫）

| ID | 内容 | 状态 | 修复版本 |
|---|---|---|---|
| CR-01 | HTTP 只读端点隐私 fail-open（`list`/`recent`/`stored-edges`） | **已修复** | v0.9.11 / G3②（FIX-05~07）＋ 信任域双向契约测试 |
| CR-02 | MCP 通道 `privacy_context` 硬编码 `None` | **已修复** | v0.9.11 / G3①（FIX-07） |
| CR-05 | `consolidate` 内层 60s ≥ 外层 30s ⇒ 取消不可达 | **已修复** | v0.9.11 / G4①（FIX-08）＋ 阶梯守卫测试 |
| CR-06 | release `panic="abort"` 使 panic 隔离失效 | **已修复** | v0.9.11 / G8（FIX-10）＋ 构建 profile 守卫测试 |
| CR-07 | Postgres `ON CONFLICT DO UPDATE SET` 只更新 10/24 列（限 postgres） | **已修复** | v0.9.11 / G2（FIX-04）＋ 字段清单矩阵守卫 |
| CR-08 | Postgres 未持久化 `information_gain`/`resolution`/`daoti_preview_*` | **已修复** | v0.9.11 / G2（FIX-04） |

### 第二梯队（P1）

| ID | 内容 | 状态 |
|---|---|---|
| CR-03 | associations / association-graph 无隐私参数 | **已修复**（FIX-13）＋ 零泄漏契约测试 |
| CR-04 | 跨用户合成聚合 + 建议区二次泄露 | **已修复**（FIX-14）＋ `inherit_privacy` 6 单测；**清单中 `synthesis_engine.rs`/`discovery.rs` 为误报** |
| CR-10 | 结晶 Phase1 持锁长重活且不查 cancel | **部分修复**（FIX-11）｜取消自查 + 有界取锁 + 分批让锁已落地；结构侧"编码移出锁 + 保留合并语义的批量写"属**独立优化**，已显式登记（见 FIX-34） |
| CR-11 | MCP 路径无超时取锁 + async 内同步重活 | **已修复**（FIX-12 + **FIX-34**）｜13/13 处有界取锁；新增源码级守卫钉死"无上限 `memory_store.lock().await`"（含鉴别力实测） |
| CR-12 | `audit.jsonl` 启动重放 O(n²) | **已修复**（FIX-16）＋ 规模无关性性能守卫 |
| CR-13 | `/v1/associations/confirm` 无界自旋 | **已修复**（G4①） |
| CR-14 | `/v1/benchmarks/report` 内层 90s > 外层 30s | **已修复**（G4①） |
| CR-15 | `engine/`（Layer 2）反向依赖 Layer 1 | **部分修复**（FIX-30）｜已消除 2 条反向边（依赖倒置 + 改依赖纯数据契约）+ 目录扫描守卫；`synthesis_engine → graph_store/persistence` 残留已登记 |
| CR-17 | Qdrant 时间语义失真 + 多字段缺失 | **已修复**（G2/FIX-04） |
| CR-18 | Qdrant fallback 不落盘 | **已修复**（FIX-18）＋ WAL 6 单测 |
| CR-19 | 认证默认放行 + `user_id` 客户端自报 | **已修复**（FIX-23）｜非回环无 token ⇒ 拒绝启动；租户由凭据派生 + `tenant::user` 命名空间 |
| CR-20 | 进程级环境变量竞态（跨模块 + 5 组裸奔） | **已修复**（FIX-19）＋ 全 crate 唯一锁 + 唯一性守卫 |
| CR-21 | 三处门禁口径不一致（串行 vs 并行） | **已修复**（FIX-20）统一 `--test-threads=1` |
| CR-37 | 备份与实时写不互斥 | **已修复**（FIX-24）｜快照一致性闸门（写在 `atomic_file`，写入方读锁 / 备份恢复写锁） |
| CR-38 | `import_memories` 非原子无锁 | **已修复**（FIX-17，原子部分）＋ 源码守卫 |
| CR-39 | Neo4j 节点属性白名单过窄 | **已修复（按报告许可的"显式声明"路线）**（FIX-25）｜定位为「关系图投影」并写明晋级为权威源的 3 条前置条件 |
| CR-45 | 备份/恢复/迁移 0 测试覆盖 | **已修复**（FIX-26）｜拆出可注入内核 + 10 条端到端用例；**并借此抓到 `duplicates_skipped` 恒为 0 的真实缺陷** |
| CR-50 | 契约脚本不校验响应字段 | **已修复**（FIX-29①）｜新增"响应字段 ↔ 后端结构体"双向静态断言（含鉴别力实测） |
| CR-59 | **（本轮新增）** `--features server,ml` 的 clippy 口径从不被 CI 检查 | **已修复**（FIX-22）｜3 处告警清零 + 门禁并入 |

### 第三梯队（P2/P3，共 32 项）

> 完整条目见第三节各维度表。**逐项状态**如下（本轮收尾后已全部有归属）：

| ID | 级别 | 内容 | 状态 |
|---|---|---|---|
| CR-22 | P2 | God Object 拆分（`server.rs` 等） | **独立工程**（§5.4 G11 标注；本轮以 G1 铺路） |
| CR-23 | P2 | 11 处取锁模式未抽 helper | **误报**（FIX-15：v1 侧早有统一 helper，第三处误报） |
| CR-24 | P2 | 合成阈值无单一真源 | **已修复**（FIX-30②） |
| CR-25 | P2 | 构造函数逐字重复 | **已修复**（FIX-30③） |
| CR-26 | P2 | 源时间契约不统一（`created_at` vs `source_timestamp_ms`） | **已修复**（FIX-04） |
| CR-27 | P2 | 同名类型两套 API（ml / 非 ml 的 `HybridLuoShuEncoder`） | **已修复**（FIX-30④）＋ 双配置编译期守卫 |
| CR-31 | P2 | `actions/setup-python` 未按 SHA 固定 | **已修复**（FIX-29③，与 CR-53 同根因） |
| CR-34 | P2 | explore 取锁循环无独立 deadline | **已修复**（FIX-33②）｜补齐 `LOCK_WAIT` 口径，优雅收敛恢复可达 |
| CR-35 | P2 | 内存峰值叠加 | **已修复（部分）**（FIX-32②：移除写路径整库深拷贝；端到端峰值未实测，已诚实声明） |
| CR-40 | P2 | 原子写无 fsync | **已修复**（FIX-31②）＋ 收敛重复实现 |
| CR-41 | P2 | 单条写入全量重写（`O(N²)` 写放大） | **显式量化声明**（FIX-32④：后端定位 `<10,000` 条，超规模换内置后端） |
| CR-42 | P2 | 缓存跨进程不失效 | **已修复**（FIX-32①）＋ 指纹机制；鉴别力实测 |
| CR-43 | P2 | `#[serde(default)]` 掩盖缺失（`version` 缺省 0） | **已修复**（FIX-33④）＋ 鉴别力实测 |
| CR-46 | P2 | rustc 版本漂移（CI 用滚动 stable） | **已修复**（FIX-21：钉 `1.99.0`） |
| CR-47 | P2 | 覆盖假象（ML 套件静默 no-op、前端 6 套件只跑 1） | **已修复**（FIX-27） |
| CR-48 | P2 | 持久化后端测试盲区 | **已修复**（FIX-27：qdrant 入 CI；其余由字段矩阵守卫覆盖） |
| CR-51 | P2 | 文档版本漂移（README/USER_GUIDE 仍写 v0.9.9） | **已修复**（FIX-28） |
| CR-52 | P2 | 许可证边界四者不重合 | **已修复**（FIX-29②：统一目录级口径 + 脚本自检） |
| CR-53 | P2 | `SECURITY.md` 与实现不符 | **已修复**（FIX-29③：真钉 SHA + 版本表对齐） |
| CR-28 | P3 | God Object 拆分 | **独立工程**（§5.4 G11） |
| CR-29 | P3 | 同上 | **独立工程**（§5.4 G11） |
| CR-30 | P3 | 同上 | **独立工程**（§5.4 G11） |
| CR-32 | P3 | 未显式设置请求体上限（隐式契约） | **已修复**（FIX-33①）＋ 源码级守卫（鉴别力实测） |
| CR-33 | P3 | 桌面端 crypto 与主 crate 漂移 | **已修复**（FIX-30⑤）＋ 文案守卫 |
| CR-36 | P3 | explore 以 `limit=usize::MAX` 持锁全量列举 | **已修复**（FIX-33③） |
| CR-44 | P3 | 持久化加固五项 | **已修复**（FIX-33⑤）｜4 实修/早已修 + 2 显式声明（`schema_version`、journal 不持久化） |
| CR-49 | P3 | 静态资源用例只断言状态码 | **已修复**（FIX-31①）＋ MIME/响应体断言 |
| CR-54 | P3 | README 内部措辞冲突 | **已修复**（FIX-29④） |
| CR-55 | P3 | `CHANGELOG` 重复版本头 + README 语义底座不符 | **已修复**（FIX-29⑤） |
| CR-56 | P3 | `OFFLINE_MODEL_GUIDE` 矛盾 + 双 bin 无 `default-run` | **已修复**（FIX-29⑥） |
| CR-57 | P3 | `loadCrystallizationHistory` 死代码 | **已修复**（FIX-29⑦） |
| CR-58 | P3 | `docs/` 历史文档未归档 + 两份架构文档并存 | **部分修复**（FIX-29⑧：归档 14 份 + 关系声明；**合并**属独立文档工程） |
| CR-59 | P3 | **（本轮新增）** `--features server,ml` clippy 口径从不被 CI 检查 | **已修复**（FIX-22） |

> **注**：CR-16 已合并入 CR-33，不再单独编号；CR-59 为本轮新增编号。

> **G6（门禁全局化）本轮已闭环**：CR-20（全 crate 唯一 env 锁，FIX-19）、CR-21（三处口径统一，FIX-20）、
> CR-46（钉 rustc `1.99.0`，FIX-21）、CR-59（ml 口径 clippy 并入，FIX-22）、
> **CR-19（认证 fail-closed + 租户由凭据派生，FIX-23）**。
> **G5 亦已收口**：CR-12（FIX-16）、CR-18（FIX-18）、CR-38（FIX-17）、**CR-37（FIX-24）**、**CR-39（FIX-25）**。
> **G7（门禁补齐）已收口**：**CR-45（FIX-26，并抓到 `duplicates_skipped` 恒为 0 的真实缺陷）**、
> **CR-47（FIX-27）**、**CR-48（FIX-27）**。
> **G10（文档与契约）已收口**：CR-51（FIX-28）、**CR-50 + CR-52~58（FIX-29）**。
> **残留 1 项**：CR-58 的"两份架构文档合并为单一真源"（现已加关系声明 + 归档 14 份历史文档；
> 真正的合并属独立文档工程，超出本轮范围）。
> **G9（结构单一真源）已收口**：CR-24/25/27/33 全修（FIX-30）、**CR-15 部分修复**（已消除 2 条反向边 + 目录扫描守卫；
> `synthesis_engine → graph_store/persistence` 与"引擎独立分发需另建契约 crate"已显式登记）。
> **收尾轮亦已闭环**：**CR-49 + CR-40**（FIX-31）、**CR-42 + CR-35 + CR-31**（FIX-32），
> **CR-41** 经核实属"被接受的设计取舍"，已按**显式量化声明**处理（FIX-32④）。
> **收尾第三批次（FIX-33）闭环**：**CR-32 + CR-34 + CR-36 + CR-43 + CR-44**（5 项实修 + 2 项显式声明）。
> **至此 §十 第三梯队（P2/P3 共 32 项）逐项状态见上表，全部有归属**：
> - **已修复（26 项）**：CR-24、25、26、27、31、32、33、34、35（部分：深拷贝已移除，端到端峰值未实测）、
>   36、40、42、43、44、46、47、48、49、51、52、53、54、55、56、57、59；
> - **误报（1 项）**：CR-23（v1 侧早有统一取锁 helper）；
> - **显式声明为"接受取舍"（1 项）**：CR-41（JSON 后端规模边界，超规模应换内置后端）；
> - **仍属"独立工程"的登记项（4 项 + 2 项）**：CR-22/28/29/30（G11 God Object 拆分，§5.4 本就标注
>   "独立工程；本轮仅以 G1 铺路"）、CR-15 的 `synthesis_engine → graph_store/persistence` 解耦与
>   "引擎可作为独立 crate 分发"、CR-58 的"两份架构文档合并"。
> **另有 2 项"被接受的设计取舍"随 CR-44 显式声明**：`Memory` 无逐条 `schema_version`、
> `SynthesisJournal` 不持久化（判据见 FIX-33⑤(e) 与对应源码文档）。
> **结论**：审查报告中所有**可确证的缺陷**均已修复或按"显式声明 + 判据"收口；未完成项均为**特性/独立工程**，且已逐条登记后续路径。

### 本轮已修复（留档）

| ID | 内容 | 状态 |
|---|---|---|
| FIX-01 | clippy `-D warnings` 5 处 error（含本次改动引入的 3 处 doc 告警） | **已修复并复验** |
| FIX-02 | `LRC_STATE_BIAS` 模块内并发竞态 ⇒ 偶发测试失败 | **已修复**（连跑两遍 842/842） |

---

> **审查声明**：本次为只读审查；除第六节明确列出的两处当场修复外，未创建/修改/删除任何文件。
> 所有行号来自实际 `Read`/`Grep` 输出；标注「维度审查」的条目来自子审查、主控未逐条复核，使用前请按证据自行复核。
