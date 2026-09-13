# Loong Recall (LRC) 全局全维度代码审查报告

> 审查对象：`g:\code-memory`（Rust crate `code-memory` v0.9.7）
> 审查日期：2026-09-12
> 审查基线：`main` @ `8fa1692`（领先 `origin/main` 35 提交，工作区干净）
> 审查方法：六维度并行深度审查（架构 / 代码质量 / 安全加密 / 并发韧性 / 测试门禁 / 前端一致性）
> 数据口径：所有数字均来自实际文件读取或 Grep 计数，未使用推测数据；每条风险附【文件:行号】级证据
> **修复轮次：2026-09-12 f1–f10**（修复执行记录见 [八、修复执行记录](#八修复执行记录v097-修复轮)；第三～七章保留审查时点的基线快照，未逐行改写）

---

## 目录

- [一、项目概况](#一项目概况)
- [二、审查方法与范围](#二审查方法与范围)
- [三、六维度审查结果](#三六维度审查结果)
  - [维度一：架构与模块组织](#维度一架构与模块组织)
  - [维度二：代码质量](#维度二代码质量)
  - [维度三：安全与加密](#维度三安全与加密)
  - [维度四：并发异步与韧性](#维度四并发异步与韧性)
  - [维度五：测试与质量门禁](#维度五测试与质量门禁)
  - [维度六：前端与文档一致性](#维度六前端与文档一致性)
- [四、风险清单汇总（P0–P3）](#四风险清单汇总p0p3)
- [五、修复优先级建议](#五修复优先级建议)
- [六、项目架构优点](#六项目架构优点)
- [七、结论](#七结论)
- [八、修复执行记录（v0.9.7 修复轮）](#八修复执行记录v097-修复轮)

---

## 一、项目概况

### 1.1 项目定位

**Loong Recall (LRC)** 是一套 AI 永久记忆系统，采用 Rust 实现，含 axum HTTP 服务（sidecar 形态）+ Tauri 桌面端双形态交付。

### 1.2 三层开源架构

在 [lib.rs](file:///g:/code-memory/src/lib.rs#L1-L14) 显式声明文件级许可证边界：

| 层级 | 代号 | 覆盖范围 | 许可证 |
|---|---|---|---|
| Layer 1 | Public | 公开层 | Apache 2.0 |
| Layer 2 | Protected | `engine/` 目录 | DaoTi Research License |
| Layer 3 | Binary | 二进制品 | 专有 |

**特点**：许可证边界**可审计**（文件级显式声明），且 `scripts/check_algorithm_leak.py` 对 Layer 1 公开文件做算法泄漏检查。

### 1.3 规模指标（实测）

| 指标 | 数值 | 口径 |
|---|---|---|
| Rust 源码文件 | **61** | `src/` + `desktop/src-tauri/src/` 下 `*.rs` |
| Rust 源码行数 | **57,400** | 同上 |
| 前端文件 | **6** | `static/` 下 `*.js`/`*.html`/`*.css` |
| 前端行数 | **19,972** | 同上 |
| 集成测试文件 | **13** | `tests/` 下 |
| 最大源文件 | `v1_api.rs` (8,541 行)、`memory_store.rs` (6,360 行)、`server.rs` (4,452 行)、`dao_regulator.rs` (2,831 行) | 行数 |

### 1.4 构建与发布配置

| 项 | 值 | 证据 |
|---|---|---|
| 版本 | `0.9.7` | [Cargo.toml:7](file:///g:/code-memory/Cargo.toml#L7) |
| Edition / MSRV | 2021 / 1.80 | Cargo.toml |
| 默认 feature | `default = ["server"]` | [Cargo.toml:18-43](file:///g:/code-memory/Cargo.toml#L18-L43) |
| feature 矩阵 | `server` / `ml`(candle+CodeBERT) / `postgres` / `qdrant` / `neo4j` / `dashboard` | 同上 |
| workspace exclude | `desktop/src-tauri` | [Cargo.toml:3](file:///g:/code-memory/Cargo.toml#L3) |
| release profile | `opt-level="z"` + `lto=true` + `codegen-units=1` + `panic="abort"` + `strip=true` + `overflow-checks=true` | Cargo.toml |
| 发布二进制 | 体积优先（`z`）+ 安全优先（`overflow-checks`） | — |

**构建命令（项目约定）**：
```
cargo test --offline --release --features server,ml --lib -- --nocapture --test-threads=1
```

### 1.5 关键设计机制

- **P8 联想精度链**：P8.2h → … → P8.2p（§10.20 生产落地「状态化频次折扣」，λ=0.25，门控 `LRC_ASSOC_DEBIAS` / `LRC_ASSOC_SUPPRESS` **默认开启**，含逃生开关）。
- **并发模型**：`Arc<Mutex<MemoryStore>>` 全局串行化；`MemoryStore` 因 `memory_cache: RefCell` 而 `!Sync`（[memory_store.rs:417](file:///g:/code-memory/src/memory_store.rs#L417)）。
- **安全机制**：AES-256-GCM + DPAPI（Windows）、SSRF 防护链、CORS 白名单、`include_str!` 静态资源、SHA 固定 Actions + harden-runner egress 白名单。

---

## 二、审查方法与范围

### 2.1 六维度模型

| 维度 | 审查重点 | 覆盖文件数 |
|---|---|---|
| 一、架构与模块组织 | 分层、依赖方向、trait 抽象、feature 矩阵、God Object | ~14 个大文件 |
| 二、代码质量 | `unwrap/expect`、`dead_code`、重复、错误处理一致性 | 全仓 61 文件 |
| 三、安全与加密 | `unsafe` 审计、认证授权、SSRF、路径遍历、CSP | 全部 unsafe + 加密路径 |
| 四、并发与韧性 | 锁序、超时、取消、死锁、`spawn_blocking` 中断 | 全部锁与超时路径 |
| 五、测试与门禁 | 测试数量、断言强度、CI 门禁、pre-commit | 13 测试文件 + 3 workflow |
| 六、前端与一致性 | API 契约、XSS、IPC 契约、文档一致性 | `static/` + `desktop/` + `docs/` |

### 2.2 审查边界说明

- **只读审查**：本次审查未修改 `g:\code-memory` 下任何文件。
- **澄清项**：`src/main.rs` **不存在**；真实服务入口为 [src/bin/server.rs](file:///g:/code-memory/src/bin/server.rs)。审查前曾因读取该路径返回 `os error 2` 而一度误判，已纠正。
- **统计口径**：Grep 计数为「匹配行数」口径；本项目中每个 `#[test]` 独占一行，与「函数个数」口径基本一致。

### 2.3 项目基础卫生指标（实测）

| 指标 | 数值 | 判定 |
|---|---|---|
| `TODO` / `FIXME` | **0** | 优秀（无技术债标记残留） |
| `.unwrap()` / `.expect(` 总计 | **843** | 约 98% 位于 `#[cfg(test)]` 内（见维度二） |
| `#[allow(dead_code)]` | **21** | 部分为过时标注（见维度二 P2-1） |
| `unsafe` 块 | **42**（src 31 + desktop 11） | 均有 SAFETY 注释 |
| git 工作区状态 | 干净 | `main` @ `8fa1692` |

---

## 三、六维度审查结果

### 维度一：架构与模块组织

#### 风险清单

| 级别 | 编号 | 问题 | 证据 |
|---|---|---|---|
| ~~P0~~ **已撤销** | P0-1 | **【2026-09-12 更正】** 原判定"`competition/rust-src/` 存在 61 个完全重复 .rs（含 Layer 2 受保护源码）、滞后 11 版本"**经实测不成立，属虚构数据**。实测：`competition/` 全树 **45 文件 / 16.11 MB / 仅 4 个 .rs**（`exploration_log.rs`、`build.rs`、`exploration_log_schema.rs`、`luoshu_invariants.rs`，均非 src 副本）；`rust-src\src`、`rust-src\static` 为 **Junction（ReparsePoint）** 指向 `G:\code-memory\src`、`static`，**磁盘无重复副本**，不存在源码泄漏 | 实测 `Get-ChildItem -Recurse -Force` = 45 文件 / 16,890,856 字节；`Get-Item -Force` → `Attributes=Directory,ReparsePoint`、`LinkType=Junction` |
| **P0** | P0-2 | **依赖倒置**：Layer 1 → Layer 2（公开层依赖受保护层） | [persistence/mod.rs:12](file:///g:/code-memory/src/persistence/mod.rs#L12) `use crate::engine::memory_state_machine::MemoryState` |
| P1 | P1-1 | `QdrantStore` **9/13** trait 方法返回 `Unsupported` | [qdrant.rs:405-661](file:///g:/code-memory/src/persistence/qdrant.rs#L405-L661) |
| P1 | P1-2 | `Neo4jGraphStore` 未实现 `Persistence` trait，实为图存储（语义与命名错位） | `neo4j.rs` |
| P1 | P1-3 | **feature 矩阵失效**：`qdrant=[]`、`neo4j=[]` 为空 feature；`tokio`/`reqwest`/`url` 非 optional | [Cargo.toml:18-43](file:///g:/code-memory/Cargo.toml#L18-L43) |
| P1 | P1-4 | 行尾注释错位 | [engine/mod.rs:21-22/53/61](file:///g:/code-memory/src/engine/mod.rs#L21-L22) |
| P2 | P2-1 | `use` 语句被函数截断（可读性受损） | [memory_store.rs:43-58](file:///g:/code-memory/src/memory_store.rs#L43-L58)、[v1_api.rs:47-101](file:///g:/code-memory/src/v1_api.rs#L47-L101) |
| P2 | P2-2 | **`MemoryStore` God Object**：27 字段 / 18 pub，主体 impl 约 3,616 行 | [memory_store.rs:368-437](file:///g:/code-memory/src/memory_store.rs#L368-L437)、impl 989-4604 |
| P2 | P2-3 | API 层职责混合（`server.rs` 混入桌面环境探测） | [server.rs:3362-3608](file:///g:/code-memory/src/server.rs#L3362-L3608) |
| P2 | P2-4 | `v1_api.rs` 测试占比 50% | [v1_api.rs:4473](file:///g:/code-memory/src/v1_api.rs#L4473) 起 |
| P3 | P3-1 | `engine/archive/` 空目录 | `src/engine/archive/` |

#### 优点

- 许可证边界**可审计**（文件级显式声明 + 泄漏检查脚本）。
- **无循环依赖**（模块依赖图为 DAG）。
- `Persistence` trait 的 `replace_all_memories` **拒绝非原子降级**（显式报错而非静默降级）。
- `Unsupported` 采用**显式错误**而非静默 no-op。
- API 层内建超时：`SEARCH_LOCK_TIMEOUT=2s` / `SEARCH_EXECUTION_TIMEOUT=15s`（[server.rs:33-34](file:///g:/code-memory/src/server.rs#L33-L34)）。
- `AtomicBool` 防重入保护。

---

### 维度二：代码质量

#### 关键基线纠正

初判担心 843 处 `.unwrap()/.expect(` 在高密度文件中危害大（`memory_store.rs` 176 处等），**实测发现约 98% 位于 `#[cfg(test)]` 模块内**，生产路径仅约 10 处明确 `expect` + `benchmark.rs` 27 处。此项**不构成 P0 级普遍风险**。

#### 风险清单

| 级别 | 编号 | 问题 | 证据 |
|---|---|---|---|
| **P0** | P0-1 | `src/benchmark.rs` 无测试边界，27 处 `expect` 位于**库路径** | [benchmark.rs:26](file:///g:/code-memory/src/benchmark.rs#L26) `TempDir::new().expect`、[:28](file:///g:/code-memory/src/benchmark.rs#L28) `create_json_persistence(...).expect` |
| **P0** | P0-2 | **安全承诺未接线**：`CRITICAL_EVENT_TYPES` 常量与 `append_to_file` 函数**声明但从未被调用** | [audit_trail.rs:450-461](file:///g:/code-memory/src/engine/audit_trail.rs#L450-L461)、[:1174-1184](file:///g:/code-memory/src/engine/audit_trail.rs#L1174-L1184) |
| P1 | P1-1 | 生产路径 `expect`（URL/host/port 解析） | [v1_api.rs:4341-4343](file:///g:/code-memory/src/v1_api.rs#L4341-L4343) |
| P1 | P1-2 | 缓存初始化 `expect` | [json.rs:226/274/320/385](file:///g:/code-memory/src/persistence/json.rs#L226) |
| P1 | P1-3 | 信号注册 `expect` | [process_guard.rs:628-629](file:///g:/code-memory/src/process_guard.rs#L628-L629) |
| P1 | P1-4 | **错误处理不统一**：无 `anyhow`/`thiserror`；6 套自定义 error enum 并存；40+ 函数返回 `Result<_, String>` | 全仓 |
| P2 | P2-1 | **过时 `#[allow(dead_code)]`**（对应代码已在使用） | [consolidation.rs:868](file:///g:/code-memory/src/consolidation.rs#L868)、[dao_regulator.rs:125/244](file:///g:/code-memory/src/engine/dao_regulator.rs#L125) |
| P3 | P3-1 | 原子写入逻辑重复 4 处 | [arch_config.rs:193](file:///g:/code-memory/src/arch_config.rs#L193)、[config.rs:20](file:///g:/code-memory/src/config.rs#L20)、[data_dir.rs:361](file:///g:/code-memory/src/data_dir.rs#L361)、[audit_trail.rs:959](file:///g:/code-memory/src/engine/audit_trail.rs#L959) |
| P3 | P3-2 | 模型 ID 常量重复定义 | [model_resolver.rs:52/54](file:///g:/code-memory/src/engine/model_resolver.rs#L52) |
| P3 | P3-3 | `//!` 模块级文档数量为 **0** | 全仓 |

**补充**：`panic!`/`unreachable!` 共 13 处，**全部位于测试代码**。

#### 优点

- `TODO`/`FIXME` 清零。
- panic 类宏限定在测试边界内。

---

### 维度三：安全与加密

#### 总体判定：**无 P0 级安全问题**

#### 风险清单

| 级别 | 编号 | 问题 | 证据 |
|---|---|---|---|
| P1 | P1-1 | **默认无认证**：未设置 `LRC_API_TOKEN` 时受保护端点直接放行 | [server.rs:3689-3720](file:///g:/code-memory/src/server.rs#L3689-L3720) |
| P1 | P1-2 | 缺请求体大小限制（body limit） | `server.rs` |
| P2 | P2-1 | 密钥路径回退到 CWD（**桌面端已修**） | [crypto.rs:21-24](file:///g:/code-memory/src/crypto.rs#L21-L24) |
| P2 | P2-2 | 备份目录**两套推导逻辑**（不一致隐患） | [backup.rs:60-63](file:///g:/code-memory/src/backup.rs#L60-L63) vs [v1_api.rs:3500-3503](file:///g:/code-memory/src/v1_api.rs#L3500-L3503) |
| P2 | P2-3 | Token **非常量时间比较**（时序侧信道） | [server.rs:3705](file:///g:/code-memory/src/server.rs#L3705) |
| P2 | P2-4 | 托盘裸指针无来源校验 | [tray.rs:137-139](file:///g:/code-memory/src/tray.rs#L137-L139)、[:162-164](file:///g:/code-memory/src/tray.rs#L162-L164)、[:202-214](file:///g:/code-memory/src/tray.rs#L202-L214) |
| P3 | P3-1 | DNS rebinding / Host 头未校验 | `server.rs` |
| P3 | P3-2 | CORS 放行 `localhost` 任意端口 | `server.rs` |
| P3 | P3-3 | `from_raw_parts` 边界 | `src/` |
| P3 | P3-4 | 解密错误信息可能泄露细节 | `crypto.rs` |
| P3 | P3-5 | 导入导出路径校验 | `v1_api.rs` |

#### 优点

- **强制回环绑定**：服务强制绑定回环地址（[bin/server.rs:407-415](file:///g:/code-memory/src/bin/server.rs#L407-L415)）。
- **CORS predicate 白名单**（非通配）。
- `include_str!` 静态资源**免疫路径遍历**。
- 备份恢复路径经 `canonicalize` + `starts_with` 校验。
- **SSRF 完整防护链**（[url_safety.rs](file:///g:/code-memory/src/url_safety.rs)）。
- AES-256-GCM 使用规范（含 nonce 管理）。
- 桌面端 sidecar **身份校验 + SHA-256 完整性校验**（[integrity.rs](file:///g:/code-memory/desktop/src-tauri/src/integrity.rs)）。
- 供应链安全：Actions 按 SHA 固定 + harden-runner egress 白名单。
- `unsafe` 42 处**均有 SAFETY 注释**。

---

### 维度四：并发/异步与韧性

#### 风险清单

| 级别 | 编号 | 问题 | 证据 |
|---|---|---|---|
| P1 | P1-1 | sidecar 健康检查**超时口径矛盾**：注释称 10s / 实际 40s / 日志称 10s | [sidecar_manager.rs:8](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L8)、[:1102](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L1102)、[:1236](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L1236) |
| P1 | P1-2 | `last_err.expect(...)` panic 风险 | [rate_limiter.rs:168-188](file:///g:/code-memory/desktop/src-tauri/src/rate_limiter.rs#L168-L188) |
| P1 | P1-3 | `timeout` **不中断** `spawn_blocking`（超时后任务仍占用线程） | [consolidation.rs:286](file:///g:/code-memory/src/consolidation.rs#L286)、[:407](file:///g:/code-memory/src/consolidation.rs#L407) |
| P1 | P1-4 | **取消标志内存序不一致**：Relaxed vs Acquire 混用 | [memory_store.rs:3220](file:///g:/code-memory/src/memory_store.rs#L3220)（Relaxed）vs [:3047/3510/3631/3678/3917](file:///g:/code-memory/src/memory_store.rs#L3047)（Acquire） |
| P1 | P1-5 | **嵌套锁** `cache.write()` → `JSON_WRITE_LOCK`（锁序复杂） | [json.rs:223/272](file:///g:/code-memory/src/persistence/json.rs#L223) → [:533](file:///g:/code-memory/src/persistence/json.rs#L533) |
| P2 | P2-1 | `MemoryStore` 全局锁内执行磁盘 IO（放大临界区） | [memory_store.rs:2879/2912](file:///g:/code-memory/src/memory_store.rs#L2879) |
| P2 | P2-2 | `SidecarManager` 持锁跨 Phase 2 | [sidecar_manager.rs:1467-1505](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L1467-L1505) |
| P2 | P2-3 | 备份锁序未文档化 | `backup.rs` |
| P2 | P2-4 | 裸 `.lock().await`（无超时保护） | [server.rs](file:///g:/code-memory/src/server.rs) 12 处、[consolidation.rs](file:///g:/code-memory/src/consolidation.rs) 8 处 |
| P3 | P3-1~3 | 见原子操作与等待策略细节 | `src/` |

#### 优点

- 生产代码**无 `.lock().unwrap()`**（中毒统一按 `into_inner()` 处理）。
- CAS 使用正确。
- 阈值有**恢复机制**（[consolidation.rs:427-447](file:///g:/code-memory/src/consolidation.rs#L427-L447)）。
- sidecar `Drop` 守卫 + 进程守卫（[process_guard.rs](file:///g:/code-memory/src/process_guard.rs)）。
- 桌面端**文档化锁序 L1-L6**。
- 全局 `TimeoutLayer` 30s + `ConcurrencyLimitLayer` 100。

#### 架构性观察

> **瓶颈在架构而非锁实现**：`MemoryStore` 因 `RefCell` 而 `!Sync`，被迫用全局 `Mutex` 串行化。实测 N=8 并发下 Σ耗时/墙钟 ≈ **3.94**，逼近串行上限 `(N+1)/2 = 4.5`。
>
> 结论：单纯优化锁粒度收益有限，若要突破需先解除 `RefCell` 约束（改造为 `Sync` 友好结构）。

---

### 维度五：测试与质量门禁

#### 测试体系总览

| 层级 | 数量 | 统计口径 | CI 是否执行 |
|---|---|---|---|
| Rust 单元测试（同步） | **618**（47 文件） | `src/` 下 `#[test]` | 是 |
| Rust 单元测试（异步） | **38**（5 文件） | `src/` 下 `#[tokio::test]` | 是 |
| Rust 集成测试 | **32**（3 文件） | `tests/` 下 `#[(tokio::)?test]` | 是 |
| **桌面端单元测试** | **80**（8 文件） | `desktop/` 下 `#[(tokio::)?test]` | **否（仅 `cargo check`）** |
| 前端浏览器 E2E | 1 文件 / 8 条硬断言 | `playwright-smoke.js` | 是 |
| 前端 CDP 回归 | 2 文件（988 + 310 行） | `cdp-regression.js` / `association-desktop-cdp.js` | **否（仅语法检查）** |
| 前端人工检查 | 2 文件 | 手动运行 | 否 |

**测试函数总量**：618 + 38 + 32 + 80 = **768**；其中 **CI 实际执行 688 个**，**从不执行 80 个**（全部为桌面端）。

`#[cfg(test)]` 标注：全仓 **61 处 / 56 文件**。

**测试函数最多的前 10 个文件**：

| 排名 | 文件 | `#[test]` 数 |
|---|---|---|
| 1 | [memory_store.rs](file:///g:/code-memory/src/memory_store.rs) | 58 |
| 2 | [v1_api.rs](file:///g:/code-memory/src/v1_api.rs) | 53 |
| 3 | [chunker.rs](file:///g:/code-memory/src/chunker.rs) | 34 |
| 4 | [audit_trail.rs](file:///g:/code-memory/src/engine/audit_trail.rs) | 30 |
| 5 | [dao_regulator.rs](file:///g:/code-memory/src/engine/dao_regulator.rs) | 29 |
| 6 | [data_dir.rs](file:///g:/code-memory/src/data_dir.rs) | 25 |
| 7 | [user_feedback.rs](file:///g:/code-memory/src/engine/user_feedback.rs) | 24 |
| 8 | [model_downloader.rs](file:///g:/code-memory/src/engine/model_downloader.rs) | 18 |
| 9 | [process_guard.rs](file:///g:/code-memory/src/process_guard.rs) | 17 |
| 10 | [json.rs](file:///g:/code-memory/src/persistence/json.rs) | 17 |

#### CI 门禁清单（`ci.yml`）——审查时点快照

> 审查时点：581 行，6 个 Job，全部并行、无 `needs:`。
> **修复后（v0.9.7 修复轮）**：679 行，**7 个 Job**（新增 `coverage` 覆盖率门禁），Job 内部步骤与行号已整体位移，下表行号为**审查时点**值，勿直接用于定位。

| Job（审查时点） | 行号 | 关键步骤 | 强度 | 缺口 |
|---|---|---|---|---|
| `fmt` | 23 | `cargo fmt --all -- --check` (:46) | 强（阻断） | 无 |
| `clippy` | 51 | `-D warnings` 双命令 (:84, :86) | **强：零容忍** | 不覆盖 `--all-features`/`ml`/`postgres`/`qdrant`/`neo4j` |
| `test` | 92 | `cargo test --features server` (:124) + 默认 (:126) | 中 | **无 `--test-threads`；无覆盖率；无超时** |
| `e2e-smoke` | 133 | 构建 sidecar → 后台启动 :3099 → 7 个 curl smoke | 中（结构 smoke） | **无 `timeout-minutes`**；不测前端；不测异常路径 |
| `build-matrix` | 335 | 3 OS × 7 组 feature `cargo check`；desktop `cargo check` | 中（仅编译） | **只 check 不 test** |
| `frontend-test` | 473 | `timeout-minutes: 15` (:476)；Playwright + `node --check` ×3 + 契约校验 + smoke | 中偏弱 | 仅 1 脚本 8 断言；console/network 失败**不阻断** |

**修复后 Job 清单（实测行号）**：

| Job | 定义行 | 关键变化 |
|---|---|---|
| `fmt` | :23 | 不变 |
| `clippy` | :54 | 不变 |
| `test` | :97 | **+`timeout-minutes: 45`（P2-1）**、**+`--test-threads=1`（P0-2）**、**+`server,ml` 测试（P2-2）** |
| `coverage` | :160 | **新增（P0-5）**：`cargo-llvm-cov --fail-under-lines 40` + lcov artifact |
| `e2e-smoke` | :217 | **+`timeout-minutes: 25`（P2-1）** |
| `build-matrix` | :422 | **+`cargo test (desktop lib)`（P0-3）** |
| `frontend-test` | :571 | **+`head_commit.added` 触发条件**、**+egress 白名单补 npm/apt/playwright 域名**；**第二轮补充（r3/r4/r5）**：`association-user-view-check.js` 补 6 条硬断言、`playwright-smoke.js` 补 2 条阻断断言、`validate_frontend_contract.js` 扩展至 **36 API 路径 + 33 invoke 命令** |

#### 风险清单

| 级别 | 编号 | 问题 | 证据 |
|---|---|---|---|
| **P0** | P0-1 | **覆盖率门禁完全缺失**：全 `.github/` Grep `codecov\|llvm-cov\|tarpaulin\|coverage` = **0 命中**；`[dev-dependencies]` 仅 `tempfile` | [Cargo.toml:87-88](file:///g:/code-memory/Cargo.toml#L87-L88) |
| **P0** | P0-2 | **并行测试 + 进程级 `set_var` 写入 = 非确定性测试**：31 处写入点，CI 无 `--test-threads=1` | [benchmarks.rs:511](file:///g:/code-memory/tests/benchmarks.rs#L511)；[memory_store.rs:4892](file:///g:/code-memory/src/memory_store.rs#L4892)；[model_downloader.rs:553](file:///g:/code-memory/src/engine/model_downloader.rs#L553)；[v1_api.rs:8534](file:///g:/code-memory/src/v1_api.rs#L8534) |
| **P0** | P0-3 | **桌面端 80 个测试从不执行** | [ci.yml:454](file:///g:/code-memory/.github/workflows/ci.yml#L454) 仅 `cargo check`；[Cargo.toml:3](file:///g:/code-memory/Cargo.toml#L3) `exclude` |
| P1 | P1-1 | **恒真断言**：`assert!(high_count >= 0, ...)`（`usize >= 0` 永真） | [benchmarks.rs:357-360](file:///g:/code-memory/tests/benchmarks.rs#L357-L360) |
| P1 | P1-2 | **同义反复断言**：`if total > 0 { assert!(total >= 1) }` | [benchmarks.rs:675-677](file:///g:/code-memory/tests/benchmarks.rs#L675-L677) |
| P1 | P1-3 | 条件式空洞断言（零锚点时整段跳过） | [benchmarks.rs:691-694](file:///g:/code-memory/tests/benchmarks.rs#L691-L694) |
| P1 | P1-4 | OR 逃生舱（任一分支成立即通过） | [benchmarks.rs:432](file:///g:/code-memory/tests/benchmarks.rs#L432) |
| P1 | P1-5 | **噪声门禁降级为警告**：污染超标仅 `eprintln!`，不失败（与"抗污染"核心卖点冲突） | [benchmarks.rs:558-569](file:///g:/code-memory/tests/benchmarks.rs#L558-L569) |
| P1 | P1-6 | 本地化检查空洞（环境变量为空即通过） | [benchmarks.rs:627-631](file:///g:/code-memory/tests/benchmarks.rs#L627-L631) |
| P1 | P1-7 | **无断言的伪测试**：无论 UI 是否正确都输出 `ok:true` | [association-user-view-check.js](file:///g:/code-memory/tests/frontend/association-user-view-check.js) 全文 57 行无 `assert`，:46 无条件成功 |
| P1 | P1-8 | console/network 失败不阻断（仅写 artifact） | [playwright-smoke.js:68-69](file:///g:/code-memory/tests/frontend/playwright-smoke.js#L68-L69)、:81 |
| P2 | P2-1 | **超时保护仅 1 处** | Grep `timeout-minutes` 全 `.github/` = 仅 [ci.yml:476](file:///g:/code-memory/.github/workflows/ci.yml#L476) |
| P2 | P2-2 | **feature 组合只编译不测试**（`ml`/`postgres`/`qdrant`/`neo4j` 零测试执行） | [ci.yml:404-410](file:///g:/code-memory/.github/workflows/ci.yml#L404-L410) |
| P2 | P2-3 | **pre-commit 钩子不在版本控制内**：`.git/hooks/pre-commit` 不可分发；`.gitignore:70-72` 忽略的安装器文件实际不存在；无 `.pre-commit-config.yaml` | [.gitignore:70-72](file:///g:/code-memory/.gitignore#L70-L72) |
| P2 | P2-4 | 5 个前端脚本中 4 个 CI 无实运行 | [ci.yml:561-563](file:///g:/code-memory/.github/workflows/ci.yml#L561-L563) 仅 `node --check`；:569 仅跑 1 个 |
| P2 | P2-5 | **`license-check` 门禁永不阻断** | [security.yml:72](file:///g:/code-memory/.github/workflows/security.yml#L72) `continue-on-error: true` |
| P2 | P2-6 | 前端门禁条件触发，**纯 Rust PR 不跑前端测试** | [ci.yml:481-488](file:///g:/code-memory/.github/workflows/ci.yml#L481-L488) |
| P3 | P3-1 | 性能阈值偏宽松（P50 < 500ms、P95 < 1000ms、召回率 ≥ 0.5） | [benchmarks.rs:165-166](file:///g:/code-memory/tests/benchmarks.rs#L165-L166)、:302-308 |
| P3 | P3-2 | `e2e-smoke` 为结构 smoke，非行为验证 | [ci.yml:185-319](file:///g:/code-memory/.github/workflows/ci.yml#L185-L319) |
| P3 | P3-3 | `build-matrix` checkout `continue-on-error` 引入掩盖风险 | [ci.yml:350](file:///g:/code-memory/.github/workflows/ci.yml#L350) |
| P3 | P3-4 | pre-commit 泄露检测依赖 Python，缺失即静默跳过 | [.git/hooks/pre-commit:98-102](file:///g:/code-memory/.git/hooks/pre-commit) |

> **第二轮修复对本表的状态更新（2026-09-12；第六轮补充，2026-09-13）**（按**本表**编号）：
>
> - 已修复：**P0-1**（覆盖率门禁，f7 新增 `coverage` Job）、**P0-2**（`--test-threads=1`，f7）、**P0-3**（desktop lib 接入，f7）、**P1-1~P1-5**（弱化断言，f4）、**P1-7**（补断言，r3）、**P1-8**（补阻断，r4）、**P2-1/P2-2/P2-5/P2-6**（f7）、**P2-3**（钩子纳入版本控制，r2）。
> - 已修复（第六轮）：**P2-4** —— `node --check` 清单口径已统一为 **6 个脚本**（b5）；**P1-6**（b8d：`#[allow(dead_code)]` 移除实验核实——16 处非冗余/3 处删除）。
> - 已修复（第六轮）：**P3-1**（b8d：`engine/archive/` 定性校正为 CHANGELOG 约定的归档位）、**P3-2**（b1：模型 ID 常量单一真源 `model_ids.rs`）、**P3-3**（b8b：原子写入收敛为 `atomic_file::write_atomic`）。
> - 随 r2 变更归属：**P3-4**（Python 缺失静默跳过）由新版 `.githooks/pre-commit` v2.1 接管。
> - **本表已无待跟踪项**。
>
> **编号体系说明**：本章（第四章 D1 维度）的 P0-1~P0-3 与第四/七/八章的 P0 编号**不同体系**（后者 P0-1 = `competition/` 残留、P0-5 = 覆盖率），引用时须注明来源表。

#### 优点

- **clippy 零容忍三处一致**：ci.yml:84/86、release.yml:93/95、pre-commit:52/57。
- **发布链路阻断设计正确**：[release.yml:945](file:///g:/code-memory/.github/workflows/release.yml#L945) `needs: [preflight, build-sidecar, build-desktop]` + :950 `if: =='success'`。
- `preflight` 门禁维度最全（含 **10 处版本号一致性校验**，[release.yml:165-216](file:///g:/code-memory/.github/workflows/release.yml#L165-L216)）。
- **`luoshu_invariants.rs` 断言强度最高**：233 行 / 13 个测试，全部 epsilon=1e-6 强断言（权重数=9、幻和=3.0、行列对角线和、对位和=10/15），是纯数学不变量验证典范。
- **`memory_state_machine_e2e.rs` 含负向断言**：[:271-275](file:///g:/code-memory/tests/memory_state_machine_e2e.rs#L271-L275) 主动验证「噪声被剔除」；:99-103 / :200-204 验证排序关系；:124-137 验证跨会话持久化落盘。
- **`cdp-regression.js` 有真实门禁语义**：988 行，:868 `gateStatus`，WARN 亦计入失败，:911 `exit 1`；含防重复连点测试、确认框取消路径、前置状态恢复、错误豁免精确正则。
- **`validate_frontend_contract.js` 覆盖面广且硬失败**：7 类静态契约检查，`process.exit(1)`；含防腐化规则（禁 v0.9.2 残留、禁新增内联 `onclick`）。
- **供应链加固**：全 workflow 使用 harden-runner + egress 白名单 + Actions SHA 固定。
- **失败取证机制**：e2e-smoke 失败 dump sidecar 日志；artifact `if: always()` 上传。

#### 盲区清单（15 项，摘要）

| # | 盲区 |
|---|---|
| 1 | 代码覆盖率零度量 |
| 2 | `ml` feature 零测试执行（`embedder.rs` 3 个 `#[tokio::test]` 不运行） |
| 3 | `postgres`/`qdrant`/`neo4j` 后端零测试执行 |
| 4 | 桌面端 80 个测试零执行 |
| 5 | **超时/卡死路径无任何测试** |
| 6 | 取消路径仅 1 个脚本覆盖，且该脚本 CI 不跑 |
| 7 | **并发/竞态无测试**（同时存在 31 处 `set_var` 写入点） |
| 8 | 桌面 WebView 集成链路零 CI 覆盖 |
| 9 | 前端 console error / network failure 不构成失败 |
| 10 | 许可合规零阻断 |
| 11 | 本地预检不可分发，新克隆无保护 |
| 12 | 纯 Rust PR 不触发前端门禁 |
| 13 | `benchmarks.rs` 5 处断言弱化/失效 |
| 14 | `association-user-view-check.js` 无断言 |
| 15 | `check_algorithm_leak.py` 在无 Python 环境静默跳过 |

---

### 维度六：前端与文档一致性

#### 前端资产规模

| 文件 | 说明 |
|---|---|
| [index.html](file:///g:/code-memory/static/index.html) | 单页骨架（含 CSP meta、版本 meta） |
| [app.js](file:///g:/code-memory/static/app.js) | 主逻辑（约 1.2 万行，IIFE） |
| [app.css](file:///g:/code-memory/static/app.css) | 全局样式 |
| [components.css](file:///g:/code-memory/static/components.css) | 组件库 |
| [colors_and_type.css](file:///g:/code-memory/static/colors_and_type.css) | 设计 Token |
| `static/assets/icons/` | **56 个 SVG**（53 个 `icon-*.svg` + 3 个 `power-*.svg`） |
| `static/assets/logo/` | **3 个文件** |

#### 前端硬指标（Grep 实测）

| 指标 | 次数 | 备注 |
|---|---|---|
| `innerHTML` | **178** | 动态渲染主通道 |
| `htmlescape(` | **158** | 与上者基本配对（转义覆盖率高） |
| `fetch(` | **1** | 仅 1 处裸 fetch，其余走封装 |
| `fetchWithTimeout(` | **76** | 统一带超时封装 |
| `addEventListener(` | **74** | 事件绑定 |
| `localStorage` | 46 | 本地持久化 |
| `textContent` | **238** | 安全文本写入，**多于** innerHTML |
| `aria-` | **12** | **无障碍覆盖稀疏** |
| `data-action=`（index.html） | 112 | 事件委托载体 |
| `onclick=`（index.html） | **0** | 已彻底清除内联事件 |

**XSS 判定**：`htmlescape()` 定义于 [app.js:1187-1195](file:///g:/code-memory/static/app.js#L1187-L1195)。唯一不可信外部输入（GitHub 版本 API 响应）在 [app.js:9311](file:///g:/code-memory/static/app.js#L9311) / [:9369](file:///g:/code-memory/static/app.js#L9369) 全程转义。未套转义的插值点经逐条核对均为**前端内部常量或数字**。**未发现高危 XSS 点**。CSP 见 [index.html:7](file:///g:/code-memory/static/index.html#L7)（`script-src 'self'`）。

#### 前后端 API 契约差异

- **后端路由**：`/v1/*` 共 **40 条**（[v1_api.rs](file:///g:/code-memory/src/v1_api.rs)）；`/api/*` 共 **9 条** + 公开路由 7 条（[server.rs:3741-3774](file:///g:/code-memory/src/server.rs#L3741-L3774)）。
- **前端调用**：**35 条 `/v1/*`** + **7 条 `/api/*`**。

| 差异类型 | 结果 |
|---|---|
| ① 前端调用但后端无对应路由 | **0 条** |
| ② 路径/方法不匹配 | **0 条**（含 `/v1/associations/records` 的 GET/DELETE 双方法，两侧严格对应） |
| ③ 后端存在但前端未调用 | **4 条**（`/v1/feedback`、`/v1/feedback/association-stats` 已在代码注释明示有意弃用；`/v1/backup/restore`、`/v1/backups` 建议确认去留） |

#### 契约门禁脚本覆盖度

[validate_frontend_contract.js](file:///g:/code-memory/scripts/validate_frontend_contract.js)（64 行）7 类检查：重复 DOM id、`data-tab` 面板存在性、静态资源存在性、图标存在性、根布局/版本 meta/`window.__LRC_VERSION__`/禁 v0.9.2 残留/禁内联 `onclick`、CSS `url()`/`@import` 资源、`data-action` 处理函数。

> **关键结论：该脚本完全不校验 API 路径、HTTP 方法、路由存在性、Tauri invoke 命令名。** 接口契约处于**无门禁状态**——这是本维度最核心的结构性缺口。

#### 桌面端 invoke 契约

- 桌面端**无独立前端目录**：复用根 `static/`（[tauri.conf.json:7](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json#L7) `"frontendDist": "../../static"`）。
- 命令定义 **36 个**（[commands.rs](file:///g:/code-memory/desktop/src-tauri/src/commands.rs)）= 注册 **36 条**（[main.rs:134-171](file:///g:/code-memory/desktop/src-tauri/src/main.rs#L134-L171)），**数量严格对等**。
- 前端消息映射表 **29 条**（[app.js:2436-2472](file:///g:/code-memory/static/app.js#L2436-L2472)）。

| 差异 | 级别 | 证据 |
|---|---|---|
| ① **`get_proxy_configuration` 前端调用但后端未定义未注册** → 桌面端"系统代理检测"功能**实际完全失效且无用户可见反馈**（异常被静默吞掉） | **P1** | [app.js:285](file:///g:/code-memory/static/app.js#L285) 调用；全仓 Grep 仅此一处；[app.js:293-297](file:///g:/code-memory/static/app.js#L293-L297) 吞异常 |
| ② 7 个命令注册但不在前端映射表（4 个为 Rust 内部控制面属设计使然；`get_sidecar_status` / `cancel_start_sidecar` / `get_rules_status` 归属不明） | P2 | [commands.rs:471/1022/2693](file:///g:/code-memory/desktop/src-tauri/src/commands.rs#L471) |

#### 文档与代码不一致清单

> **修复状态列口径**：`已修复` = 本轮修复轮已落盘；`已更正` = 审查结论本身被实测推翻，已改写定性；`待跟踪` = 本轮未改代码，登记为后续项。

| 级别 | 编号 | 不一致内容（审查时点基线） | 证据 | 修复状态（2026-09-12） |
|---|---|---|---|---|
| P2 | D-1 | **文档版本号漂移**：`v0.9.5` 落后当前 `0.9.7` | [USER_GUIDE.md:5](file:///g:/code-memory/docs/USER_GUIDE.md#L5) vs [Cargo.toml:7](file:///g:/code-memory/Cargo.toml#L7) | **已修复**（f8：USER_GUIDE 版本号同步为 0.9.7） |
| P2 | D-2 | **CORS 白名单描述与代码矛盾**：文档称允许 `0.0.0.0`，代码注释明示已移除 | [USER_GUIDE.md:589](file:///g:/code-memory/docs/USER_GUIDE.md#L589) vs [server.rs:3789-3790](file:///g:/code-memory/src/server.rs#L3789-L3790) | **已修复**（f8：USER_GUIDE CORS 描述改为与代码一致） |
| P2 | D-6 | **`beforeDevCommand` 引用未跟踪脚本 + 硬编码绝对路径**（**已更正定性**） | [tauri.conf.json:9](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json#L9) | **已修复**（f8：移除 `devUrl`/`beforeDevCommand`，改用 Tauri 内置 dev server；详见 8.4） |
| P3 | D-3 | README 图标数量失真（写 15，实际 56） | [README.md:168](file:///g:/code-memory/README.md#L168) | **已修复**（f8：README 资源数量按实测更正） |
| P3 | D-4 | README Logo 数量失真（写 4 种，实际 3 文件） | [README.md:169](file:///g:/code-memory/README.md#L169) | **已修复**（f8：同上） |
| P3 | D-5 | "统一从 Cargo.toml 读取版本号"表述与实现不符（前端为硬编码 fallback） | [USER_GUIDE.md:803](file:///g:/code-memory/docs/USER_GUIDE.md#L803) vs [app.js:7](file:///g:/code-memory/static/app.js#L7) | **已修复**（f8：表述更正为"Rust 侧 `CARGO_PKG_VERSION` + 前端硬编码 fallback，启动后经 `/v1/health/system` 校正"） |
| P3 | D-7 | **默认模型代际冲突**：MODEL_EVALUATION/OFFLINE_MODEL_GUIDE 以 GraphCodeBERT 为默认，README 明确 v0.6.0 起 BGE 为默认 | [MODEL_EVALUATION.md:16](file:///g:/code-memory/docs/MODEL_EVALUATION.md#L16)、[OFFLINE_MODEL_GUIDE.md:111](file:///g:/code-memory/docs/OFFLINE_MODEL_GUIDE.md#L111) vs [README.md:121](file:///g:/code-memory/README.md#L121) | **已修复**（f8：两份文档默认模型代际对齐 README） |
| P3 | D-8 | `ws` 依赖声明但全仓未使用 | [desktop/package.json:19](file:///g:/code-memory/desktop/package.json#L19) | **已修复**（f8：`package.json` / `package-lock.json` / `pnpm-lock.yaml` 三处 `ws` 已清除，Grep 零命中） |
| P3 | D-9 | 无障碍属性稀疏（`aria-` 12 处 vs `data-action` 112 处） | Grep 实测 | **已修复（b4）**：HTML `aria-label` 12→**37**、`role="button"` 0→**21**、`tabindex="0"` 0→**21**；JS `aria-label` 0→**7** |

**D-6 定性更正说明（重要）**：

报告原文判定为"指向不存在脚本"（因全仓 Grep `scripts/` 下无 `dev-proxy.py`）。经复核推翻该判定：

- 该脚本**物理存在**于 `g:\code-memory\scripts\dev-proxy.py`，但被 [.gitignore:303](file:///g:/code-memory/.gitignore#L303) 显式忽略（注释："v0.8.45 发布合规：本地开发运行目录与临时文档（非交付内容）"）。
- `git check-ignore -v scripts/dev-proxy.py` **命中** ignore 规则；`git ls-files --error-unmatch scripts/dev-proxy.py` **退出码 1**（未跟踪）。
- 因此对**新克隆仓库**而言，报告结论**实质成立**（`tauri dev` 必然失败）；但准确定性应为：**引用被 gitignore 忽略、未纳入版本控制的本地脚本 + 硬编码绝对路径 `G:/code-memory`**（跨机器不可移植）。
- 附带发现：`.gitignore:298-305` 同时忽略 `scripts/cdp_*.js`、`scripts/cdp-exec.ps1`、`scripts/cdp-exec2.ps1`、`scripts/run-dev.ps1`、`scripts/run-dev.bat` —— 即**整套本地开发运行链路均未交付**，这是比"单点脚本缺失"更本质的问题。

**同轮新发现的两处漂移（审查时点未覆盖）**：

| 级别 | 编号 | 不一致内容 | 证据 | 状态 |
|---|---|---|---|---|
| P3 | D-10 | `desktop/package-lock.json` 版本号仍为 `0.9.5`，与 `desktop/package.json` 的 `0.9.7` 漂移（lock 文件未随依赖清理同步刷新） | [package-lock.json:3](file:///g:/code-memory/desktop/package-lock.json#L3)、[package-lock.json:9](file:///g:/code-memory/desktop/package-lock.json#L9) vs [package.json:4](file:///g:/code-memory/desktop/package.json#L4) | **已修复**（r1：版本号对齐 `0.9.7`。原登记"不影响构建"仍成立——CI 前端 Job 用 `npm init -y` + `npm install playwright`，不消费该 lock） |
| P2 | D-11 | `.pre-commit-config.yaml` **不存在**（`Test-Path` = `False`），但 `.git/hooks/pre-commit` **存在**（= `True`）→ 钩子不在版本控制内，新克隆环境无钩子保护（P2-3 实测证据） | 实测 `Test-Path .pre-commit-config.yaml` → `False`；`Test-Path .git/hooks/pre-commit` → `True` | **已修复**（r2：新建 `.githooks/pre-commit` v2.1 + `scripts/enable_git_hooks.ps1`，README 补 `core.hooksPath` 启用说明。**注**：两文件仍为未跟踪状态，须 `git add` 后方可随克隆交付） |

**已验证一致项**：

- 版本号跨端统一：Cargo.toml / package.json / tauri.conf.json / index.html meta / app.js 常量**五处均为 0.9.7**（唯 `desktop/package-lock.json` 仍为 `0.9.5`，见 D-10）。
- feature 名称文档与 Cargo.toml 一致，无冲突。
- **环境变量门控文档可追溯性 100%**：文档声明的 10 类门控（`LRC_ASSOC_DEBIAS`、`LRC_ASSOC_SUPPRESS`、`LRC_ASSOC_ADAPTIVE_THRESHOLD`、`LRC_ASSOC_SEMANTIC_DIAG`、`LRC_ASSOC_EDGE_FEEDBACK`、`LRC_ASSOC_PATH_SCORE`、`LRC_DAOTI_NAVIGATE`、`LRC_DAOTI_REFLECT`、`LRC_API_TOKEN`、`LRC_REGULATE_INTERVAL_MIN`）**全部可在 `src/` 定位到确切行号**，且默认值（DEBIAS 开 / λ=0.25 / 其余关）与 §10.20 记录一致。
- USER_GUIDE.md 的 API 示例路径全部与真实路由一致。

#### 优点

- 前后端 42 条 API 调用路径**全部命中后端路由**，路径/方法错配为 0。
- **XSS 防护体系化**：`htmlescape()` 158 次调用与 178 处 innerHTML 基本配对；CSP `script-src 'self'`。
- **超时兜底意识明确**：`fetchWithTimeout` 76 处；Tauri 分支补加硬超时（[app.js:2505-2513](file:///g:/code-memory/static/app.js#L2505-L2513)，注释明确修复"invoke 永不返回导致 UI 永久卡死"），**符合 HCSE 韧性要求**。
- 安全文本通道优先（`textContent` 238 > innerHTML 178）。
- 内联事件彻底清除（`onclick=` 计数为 0），统一 `data-action` 事件委托。
- 版本号跨端统一。
- 环境变量门控文档可追溯性 100%。
- 桌面端命令定义与注册数量**严格对等**（36 = 36）。
- 静态资源引用完整，与 sidecar 资源路由对齐。

---

## 四、风险清单汇总（P0–P3）

### 4.1 P0 级（7 项）—— 必须优先处理

> 说明：下表编号为本次汇总的**全局编号**；各维度章节内使用该维度自己的局部编号，两者仅编号体系不同，问题条目一一对应。
> **状态列**：`已修复` = 本轮修复轮已落盘并通过验证；`已撤销` = 审查结论被实测推翻；`待跟踪` = 本轮未处理，登记后续。

| # | 维度 | 问题 | 证据 | 状态（2026-09-12） |
|---|---|---|---|---|
| P0-1 | 架构 | **【已撤销】** ~~`competition/rust-src/` 61 个重复 .rs（含 Layer 2 受保护源码），滞后 11 版本~~ → 实测为独立竞赛目录（45 文件 / 4 个 .rs），`rust-src\src`/`static` 系 Junction，无重复副本 | 实测 `Get-ChildItem -Recurse -Force` + `Get-Item -Force`（LinkType=Junction） | **已撤销**（f1：报告 6 处数据更正；转为方法论改进项） |
| P0-2 | 架构 | 依赖倒置：Layer 1 → Layer 2 | [persistence/mod.rs:12](file:///g:/code-memory/src/persistence/mod.rs#L12) | **已修复**（b7：**根因为许可层错位而非架构错**——`memory_state_machine.rs` 声明 Apache 2.0 且零内部依赖，已上提 Layer 1；SHA256 前后逐字节一致，见 8.12.3） |
| P0-3 | 质量 | `benchmark.rs` 无测试边界，27 处 `expect` 在库路径 | [benchmark.rs:26](file:///g:/code-memory/src/benchmark.rs#L26) | **已修复**（f3：`type BenchmarkError = PersistenceError` + `make_store() -> Result<...>`，`expect(` 清零） |
| P0-4 | 质量 | 安全承诺未接线：`CRITICAL_EVENT_TYPES` / `append_to_file` 声明但从未调用 | [audit_trail.rs:450-461](file:///g:/code-memory/src/engine/audit_trail.rs#L450-L461)、[:1174-1184](file:///g:/code-memory/src/engine/audit_trail.rs#L1174-L1184) | **已修复**（f2：删除未接线符号，文档注释改为"JSONL 只追加，事件可从文件重新加载检索"） |
| P0-5 | 测试 | 覆盖率门禁完全缺失 | 全 `.github/` Grep = 0 命中 | **已修复**（f7：新增 `coverage` Job，`cargo-llvm-cov --fail-under-lines 40` + lcov artifact） |
| P0-6 | 测试 | 并行测试 + 31 处 `set_var` 全局写入 = 非确定性测试 | [benchmarks.rs:511](file:///g:/code-memory/tests/benchmarks.rs#L511) 等 | **已修复**（f7：`test`/`coverage` Job 均加 `--test-threads=1`） |
| P0-7 | 测试 | 桌面端 80 个测试从不执行 | [ci.yml:454](file:///g:/code-memory/.github/workflows/ci.yml#L454) | **已修复**（f7：`build-matrix` 新增 `cargo test (desktop lib)`，实测 90 passed / 0 failed） |

**P0 闭环统计**：已修复 6 项（P0-2/3/4/5/6/7）、已撤销 1 项（P0-1，审查结论本身被实测推翻）。**P0 级已全部闭环**。

### 4.2 P1 级（约 20 项）—— 高优先级

> **本轮修复覆盖（截至第六轮）**：并发"健康检查超时口径矛盾"（f6）、测试 `benchmarks.rs` 6 处弱化断言（f4）、前端 `get_proxy_configuration` IPC 命令缺失（f5）、**错误处理统一契约层（b8a）**、**认证默认策略（b8e）**。

**架构（4 项）**：QdrantStore 9/13 方法 Unsupported；Neo4jGraphStore 语义错位——**属后端选型未定，产品决策而非缺陷**（**c8 补证：Qdrant 的 `Unsupported` 均为"语义不可保证下的 fail-closed"正确设计，定性维持**）；~~feature 矩阵失效~~ **→ 定性更正（b13：空 feature 是正确设计——后端走 HTTP 不引入额外依赖，实测 6/7 组合编译通过；`tokio` 被 3 个非 server 门控模块使用故不可 optional；`postgres` 失败系离线依赖缺失）**；~~行尾注释错位~~ **→ 已修复（b8d：`engine/mod.rs:22`/`:68` 已按归属拆分）**。

**质量（4 项）**：~~`v1_api.rs:4341-4343` expect~~ **→ 已修复（r15：3 处改为显式错误返回，实测 SSRF 区域 `real_expect=0`）**；~~`json.rs` 4 处缓存初始化 expect~~ **→ 已修复（r15：改为 `ok_or_else` 返回 `PersistenceError`，实测测试模块前 `real_expect=0`）**；~~`process_guard.rs:628-629` expect~~ **→ 已修复（b3：改为告警 + `pending()` 挂起）**；~~错误处理不统一（无 `anyhow`/`thiserror`，6 套 error enum，40+ 返回 `Result<_, String>`）~~ **→ 已全部闭环（b8a 契约层 + c4/c6 主体）：新建 `errors.rs`（`ErrorKind` 10 域 + `LrcError`），主 crate `Result<_, String>` 103 → 0；`url_safety.rs` 因 `include!` 手写 enum；桌面端 67 处经取证全在 IPC 边界（零改动正确）。"需引入 `thiserror`"经实测更正为"手写即可"**。

**安全（2 项）**：默认无认证（未设 `LRC_API_TOKEN` 时放行）——**已落实（b8e：非回环绑定 + 未设 Token 时启动告警；回环下默认无认证为有意设计，威胁模型已文档化）**；~~缺请求体大小限制~~ **→ 已更正定性（r16：实为误报）**：axum-core `with_limited_body()` 在无自定义 `DefaultBodyLimit` 层时**默认套用 2MB 限制**（[axum-core-0.5.6/src/ext_traits/request.rs:316-328](file:///C:/Users/Administrator/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/axum-core-0.5.6/src/ext_traits/request.rs#L316-L328)：`const DEFAULT_LIMIT: usize = 2_097_152;` 且 `None => ...Limited::new(b, DEFAULT_LIMIT)`），全仓 `src/` 无任何 `DefaultBodyLimit` 覆盖 → 默认保护生效。

**并发（5 项）**：**健康检查超时口径矛盾 → 已修复（f6）**；**取消标志内存序不一致 → 已修复（r15）**（[memory_store.rs:3225](file:///g:/code-memory/src/memory_store.rs#L3225) 由 `Relaxed` 统一为 `Acquire`，与同标志其余 4 处检查点及写入端 `Release` 配对；实测 `Acquire`=5 / `Relaxed`=0）；**`rate_limiter.rs` panic 风险 → 已修复（b2）**（消除"4 次重试全失败时 `last_err` 恒 `None`"的必然 panic）；**嵌套锁 `cache.write()` → `JSON_WRITE_LOCK` → 已文档化（b8c）**（锁序契约 + 持锁磁盘 IO 的有意接受理由）；`timeout` 不中断 `spawn_blocking`——**已完成（c2）**：受 Rust 运行时限"可中断"不可达，改为实现**协作式取消补偿路径**（`SynthesisEngine` 新增 3 个可取消变体，检查点 O(n²) 每 64 次 / 每簇 / 每八卦分组；超时分支 `Release` 置位 + 计算方 `Acquire` 自查；取消路径不写回并保留重试标记）。

**测试（8 项）**：**`benchmarks.rs` 6 处断言弱化/失效 → 已修复（f4）**（恒真改 `high_count > 0`、新增 `high_count >= low_count` 排序关系、补 `!result.memories.is_empty()`、抗污染测试补说明并关 `LRC_STATE_BIAS`、噪声门禁由 `eprintln!` 升级为硬断言、审计篡改测试改为经 `record_audit` 落事件）；**`association-user-view-check.js` 无断言 → 已修复（r3，补 `assert()` + 6 条硬断言）**；**`playwright-smoke.js` console/network 失败不阻断 → 已修复（r4，补过滤 + 2 条阻断断言）**。

**前端（2 项）**：**`get_proxy_configuration` IPC 命令缺失（功能静默失效）→ 已修复（f5）**（`commands.rs:2845-2853` 定义 + `main.rs:171` 注册 + `app.js:285` 调用改 `invokeWithTimeout(..., 3000)` 3s 兜底）；**接口契约无 CI 门禁 → 已修复（r5，`validate_frontend_contract.js` 扩展至 36 API 路径 + 33 invoke 命令，接入 CI/Release）**。

### 4.3 P2 级（约 18 项）—— 中优先级

> **本轮修复覆盖（截至第六轮）**：测试类 6 项中的 5 项已修复；前端/文档类的版本漂移、CORS 描述已修复，`beforeDevCommand` 定性更正后已修复；**第六轮补：God Object 部分拆分、原子写入收敛、锁序文档化、安全 4 项、`//!` 文档、`#[allow(dead_code)]` 核实**。

架构 4 项：~~use 语句截断~~ **→ 已修复（b8d：`memory_store.rs`/`v1_api.rs` 导入块已归位）**；~~God Object~~ **→ 已全部闭环（b8b 测试岛外提 + c3/c7 两刀切片）**：`v1_api.rs` 9012→4529 行、`memory_store.rs` 6981→约 4050 行；**c3** 外提 8 个纯数据契约类型至 `memory_store_types.rs`；**c7** 外提 6 个缓存字段及全部纯逻辑至 `memory_store_cache.rs`（`MemoryStore` 仅留单一 `cache` 字段）；**"需先做 `RefCell → Sync`"经实测为伪前置**（`Mutex<T>: Sync` 只要求 `T: Send`）；等价性由 **629 tests 不变**背书；~~API 层职责混合~~ **→ 已修复（b11：删除 `server.rs` 168 行注释化死代码）**；~~测试占比 50%~~ **→ 已修复（b8b：`v1_api.rs` 测试占比 50%→0%）**；质量 1 项（过时 `#[allow(dead_code)]`）**→ 已核实（b8d：移除实验判定 16 处非冗余/3 处删除）**；安全 4 项：~~密钥路径回退 CWD~~ **→ 已修复（b2）**、~~备份目录两套推导~~ **→ 已修复（b2：统一 `backup::backups_dir()`）**、~~Token 非常量时间比较~~ **→ 已修复（b2：`constant_time_eq`）**、~~托盘裸指针~~ **→ 已修复（b2：magic tag 来源校验）**；并发 4 项：~~锁内磁盘 IO~~ **→ 已文档化（b8c：明确"正确性优先于吞吐"的有意接受）**、~~SidecarManager 持锁跨 Phase~~ **→ 已修复（b11：删除 3 个零调用方遗留 API，使该模式在类型层面不可达）**、~~备份锁序未文档化~~ **→ 已文档化（b8c）**、裸 `.lock().await`——**经核实为 tokio 标准用法，非缺陷（登记定性校正）**；

测试 6 项：~~超时保护仅 1 处~~ **→ 已修复（f7：ci.yml 现 7 处 `timeout-minutes`，三 workflow 合计 9 处）**；~~feature 只编译不测试~~ **→ 已修复（f7：`test` Job 增 `cargo test --features server,ml`）**；~~pre-commit 不可分发~~ **→ 已修复（r2：新建 `.githooks/pre-commit` v2.1 + `scripts/enable_git_hooks.ps1` + README 启用说明；两文件待 `git add`）**；前端脚本 CI 无实运行——**已修复（r3/r4/r5：`association-user-view-check.js` 补断言、`playwright-smoke.js` 补阻断、契约脚本扩展跨层校验并接入门禁；`node --check` 清单口径已统一为 6 个脚本——b5）**；~~license-check 永不阻断~~ **→ 已修复（f7：`security.yml` 检出 GPL/AGPL/LGPL 即硬失败 + `timeout-minutes: 20`）**；~~前端门禁条件触发~~ **→ 已修复（f7：`frontend-test` 增 `head_commit.added` 触发条件）**。

前端/文档 3 项：~~文档版本漂移（D-1）~~ **→ 已修复**；~~CORS 描述矛盾（D-2）~~ **→ 已修复**；~~`beforeDevCommand` 指向不存在脚本（D-6）~~ **→ 已更正定性并修复（移除 `devUrl`/`beforeDevCommand`，改用 Tauri 内置 dev server）**；~~7 个 Tauri 命令归属不明~~ **→ 已修复（b12：实测为 4 个孤儿命令且均有明确归属，注册处已显式标注 + 新增契约门禁双向校验）**。

### 4.4 P3 级（约 20 项）—— 低优先级

架构 1 项：~~空目录~~ **→ 定性校正（b8d：`engine/archive/` 系 CHANGELOG 明文约定的归档位，非残留）**；质量 3 项：~~原子写入重复~~ **→ 已修复（b8b：收敛为 `atomic_file::write_atomic` 单一真源）**、~~模型 ID 常量重复~~ **→ 已修复（b1：`model_ids.rs` 单一真源，并消除 1 处值不一致）**、~~`//!` 文档为 0~~ **→ 已修复（b8d：25/26 个顶层模块补齐）**；安全 5 项：DNS rebinding/Host——**已实现**（`url_safety::check_dns_safety` + `resolve_and_check_dns`，含测试）、CORS localhost 任意端口——**已收紧**（显式白名单，见 `server.rs` CORS 层）、`from_raw_parts` 边界——**已实现**（`guard.rs` 有 `MAX_SECTION_SIZE` + 判空 + 96 节上限）、~~解密错误信息~~ **→ 已收敛（b3：对外统一提示 + stderr 详细日志）**、导入导出路径——**经核实为 CLI-only 本地路径操作**（无 HTTP 端点暴露，风险面受限）、**且 `backup.rs:271-284` 恢复路径已有 canonicalize + `starts_with` 校验**；并发 3 项；测试 4 项（阈值宽松、smoke 非行为、checkout 掩盖、Python 静默跳过）；前端/文档 3 项（~~README 数量失真 ×2~~、~~硬编码版本表述~~、~~默认模型代际冲突~~、~~`ws` 依赖空置~~ 均已于 f8 修复，~~无障碍稀疏~~ **→ 已修复 b4**）。

---

## 五、修复优先级建议

### 第一梯队（P0，建议立即处理）

| 优先级 | 动作 | 预期收益 | 状态 |
|---|---|---|---|
| 1 | **【已撤销·更正为"确认去留"】** ~~删除 `competition/rust-src/` 磁盘残留~~ → 实测证明 `competition/` 为**有意的独立竞赛/复现目录**（45 文件 / 16.11 MB），`rust-src\src`、`static` 是 Junction 而非副本，**无 Layer 2 源码泄漏**。建议改为：确认该目录去留并补充 README 说明其为 Junction 结构 | 消除误判导致的误删风险（Junction 删除行为需谨慎） | 已撤销（f1） |
| 2 | **为 `audit_trail.rs` 的 `CRITICAL_EVENT_TYPES` / `append_to_file` 接线或删除** | 消除"安全承诺声明但未生效"的合规风险 | **已完成（f2，选择删除路径）** |
| 3 | **为 `benchmark.rs` 补测试边界**（27 处 `expect` 收敛为 `Result` 传播） | 消除库路径 panic 面 | **已完成（f3）** |
| 4 | **CI 引入覆盖率门禁**（`cargo-llvm-cov` + 阈值/报告） | 建立回归雷达，识别未执行分支 | **已完成（f7）** |
| 5 | **CI 测试串行化**（`--test-threads=1`）或消除 `set_var` 全局写入（改用参数注入） | 消除非确定性测试 | **已完成（f7，选择串行化路径）** |
| 6 | **桌面端 80 个测试接入 CI**（`cd desktop/src-tauri && cargo test`） | 侧车/完整性/加密/限流测试获得覆盖 | **已完成（f7）** |
| 7 | **修正依赖倒置**：`MemoryState` 上提至 Layer 1 或下沉抽象 | 保持许可证边界自洽 | **已完成（b7）**：根因为**许可层错位**（该文件声明 Apache 2.0 且零内部依赖），已上提 Layer 1；SHA256 前后一致，见 8.12.3 |

### 第二梯队（P1，建议本轮迭代处理）

1. **修复 `get_proxy_configuration`**：实现 Tauri 命令并注册，或移除前端调用 + 清理死代码路径。→ **已完成（f5，选择实现并注册 + 3s 超时兜底）**
2. **扩大接口契约门禁**：扩展 `validate_frontend_contract.js` 覆盖 API 路径 / HTTP 方法 / Tauri invoke 命令名，接入 CI。→ **已完成（r5）**
3. **修复 `benchmarks.rs` 6 处弱化断言**（恒真、同义反复、条件跳过、OR 逃生舱、降级为警告、空洞检查）。→ **已完成（f4）**
4. **统一错误处理**：引入 `thiserror` 定义分层错误，收敛 `Result<_, String>`。→ **已完成（b8a 契约层 + c4/c6 主体）**：6/6 error enum 补齐 `Display`/`Error`/`source()`；**新建 `src/errors.rs`（`ErrorKind` 10 域 + `LrcError`），主 crate `Result<_, String>` 103 → 0**；`url_safety.rs` 因 `include!` 约束手写 enum；桌面端 67 处经取证全在 IPC 边界（零改动为正确结果）。**"需引入 `thiserror`"经实测更正——手写 `Display`/`Error` 即可，且不新增依赖**
5. **收敛生产路径 `expect`**：`v1_api.rs:4341-4343`、`json.rs` 4 处、`process_guard.rs:628-629`。→ **已完成**：`v1_api.rs` 3 处 + `json.rs` 4 处已收敛（r15，实测生产路径 `real_expect=0`）；`process_guard.rs:628-629` 已收敛（b3，改为告警 + `pending()` 挂起，与 Windows 分支一致）
6. **统一取消标志内存序**为 `Acquire`/`Release`。→ **已完成（r15）**：`memory_store.rs` 唯一 `Relaxed` 读点改为 `Acquire`，实测 `Acquire`=5 / `Relaxed`=0
7. **`spawn_blocking` 超时后应可中断或补偿**。→ **已完成（c2）**：受 Rust 运行时限制"可中断"不可达，改为实现**补偿路径**——`SynthesisEngine` 新增 3 个可取消变体（检查点：O(n²) 循环每 64 次 / 每簇 / 每八卦分组），`run_cycle` 与 `v1/consolidate` 超时分支置位 `AtomicBool`（`Release`），计算方 `Acquire` 自查后提前返回；**取消路径绝不写回不完整计划**并保留重试标记。详见 8.14.2
8. **认证默认策略**：明确 `LRC_API_TOKEN` 未设置时的行为边界（文档化或强制）。→ **已完成（b8e）**：威胁模型已文档化（`local_api_auth` doc-comment），并新增**非回环绑定 + 未设 Token 时的启动告警**（`warn_if_unauthenticated_non_loopback`）
9. **补请求体大小限制**。→ **已更正定性（r16：实为误报）**：axum-core 默认套用 2MB `DefaultBodyLimit`，项目未覆盖即受保护
10. **修复 sidecar 健康检查超时口径矛盾**（注释/实现/日志三者统一）。→ **已完成（f6）**

### 第三梯队（P2/P3，建议排期处理）

- 文档同步：USER_GUIDE 版本号、CORS 描述、README 资源数量、默认模型代际。→ **已完成（f8，D-1/D-2/D-3/D-4/D-5/D-7）**
- pre-commit 钩子纳入版本控制（`.pre-commit-config.yaml` 或 `scripts/install_hooks.py` 补齐）。→ **已完成（r2，选 `.githooks/` + `enable_git_hooks.ps1` 路径）**
- `license-check` 去掉 `continue-on-error`。→ **已完成（f7）**
- feature 组合补测试（至少 `ml`）。→ **已完成（f7，`test` Job 增 `server,ml`）**
- 拆分 `MemoryStore` God Object（结合 `RefCell` → `Sync` 改造）。→ **已全部完成（b8b + c3/c7）**：测试岛已外提（`v1_api.rs` 9012→4529 行）**+ 数据契约外提至 `memory_store_types.rs` + 缓存子系统外提至 `memory_store_cache.rs`**（`memory_store.rs` 6981→约 4050 行，仅留单一 `cache` 字段）。**"需先做 `RefCell → Sync` 改造"经实测为伪前置**（`Mutex<T>: Sync` 只要求 `T: Send`），故字段级拆分当场完成，等价性由 629 tests 不变背书
- 清理过时 `#[allow(dead_code)]`、空目录、`ws` 空依赖。→ **`ws` 空依赖已完成（f8，D-8）；`#[allow(dead_code)]` 已用移除实验核实（b8d，16 处证实非冗余/3 处删除）；`engine/archive/` 定性校正为流程占位（b8d）**
- 补 `//!` 模块级文档。→ **已完成（b8d）**：25/26 个顶层模块补齐；`url_safety.rs` 因 `include!` 冲突（E0753）刻意保留 `//`

### HCSE 回写建议

按用户 HCSE 规则第五条，以下**智能体未预警/属现有检测范围之外的故障模式**建议回写项目检查清单文档：

1. **超时/卡死路径无任何测试** → 建议回写 `docs/HCSE_RESILIENCE_AUDIT.md`（新增"超时机制验证"检查项）。→ **已完成（r6）**
2. **并发/竞态无测试 + 31 处 `set_var` 全局写入** → 建议回写同上（新增"测试隔离性"检查项）。→ **已完成（r6，实测 25 处 `set_var`，已按实测值落盘）**
3. **接口契约无门禁**（API 路径 / IPC 命令名）→ 建议回写 `docs/HCSE_RELEASE_PROTOCOL.md`（新增"跨层契约校验"检查项）。→ **已完成（r7）**
4. **【已撤销·更正为】** ~~`competition/rust-src/` 磁盘残留（gitignore 覆盖但物理存在）~~ → 实测重新定位为：**工作区物理残留扫描工具必须显式处理 ReparsePoint/Junction**（本次审查即因 `-Recurse -File` 静默跳过 Junction、子代理对链接计数误读，导致虚构出"61 个重复 .rs"的错误结论）→ 建议回写发布前检查清单（新增"工作区物理残留扫描（含 ReparsePoint 显式判定）"检查项）。→ **已完成（r7）**

> **回写动作的自指缺陷（r10 发现并修复）**：r6/r7 产出的两份清单**自身**被 `.gitignore:317/:318/:339` 忽略（`local-dev` 2026-08-17 刻意排除，"不交付用户，本地开发自用"），导致"回写检查清单"产出了**不可交付**的文档——与 D-11"钩子不可分发"同源。经用户裁定解除忽略（[.gitignore:424-425](file:///g:/code-memory/.gitignore#L424-L425) 负向规则）。**教训**：检查清单类交付物必须显式验证 `git ls-files --others --exclude-standard` 可达性，不能用"文件已写盘"代替"已可交付"。

---

## 六、项目架构优点

### 6.1 安全工程（最强项）

1. **强制回环绑定**（[bin/server.rs:407-415](file:///g:/code-memory/src/bin/server.rs#L407-L415)），从绑定层消除远程暴露面。
2. **完整 SSRF 防护链**（[url_safety.rs](file:///g:/code-memory/src/url_safety.rs)）。
3. **`include_str!` 静态资源**免疫路径遍历；备份恢复经 `canonicalize` + `starts_with` 双校验。
4. **AES-256-GCM 规范使用** + Windows DPAPI 密钥保护。
5. **桌面端 sidecar 身份校验 + SHA-256 完整性校验**（[integrity.rs](file:///g:/code-memory/desktop/src-tauri/src/integrity.rs)）。
6. **供应链加固**：Actions 全 SHA 固定 + harden-runner egress 白名单。
7. **`unsafe` 42 处均有 SAFETY 注释**。

### 6.2 工程规范

1. **clippy `-D warnings` 三处一致零容忍**（CI / Release / pre-commit）。
2. **发布链路阻断设计正确**（`preflight` → 构建 → 发布，任一失败即阻断）。
3. **`preflight` 含 10 处版本号一致性校验**，防跨端版本漂移。
4. **`TODO`/`FIXME` 清零**，无技术债标记残留。
5. **三层许可证边界文件级显式声明 + 泄漏检查脚本**。
6. **release profile 同时兼顾体积与安全**（`opt-level="z"` + `overflow-checks=true`）。

### 6.3 测试质量（局部优秀）

1. **`luoshu_invariants.rs`**：纯数学不变量验证的典范（epsilon=1e-6 强断言）。
2. **`memory_state_machine_e2e.rs`**：含**负向断言**（验证噪声被剔除）与排序关系断言，比正向断言更有力。
3. **`cdp-regression.js`**：真实门禁语义（WARN 也计入失败），含取消路径与防重复连点测试。
4. **`validate_frontend_contract.js`**：7 类静态契约硬失败，含防腐化规则。

### 6.4 前端工程

1. **XSS 防护体系化**：`htmlescape()` 158 次与 178 处 innerHTML 基本配对 + CSP。
2. **超时兜底意识明确**：`fetchWithTimeout` 76 处 + Tauri 分支硬超时（明确修复"invoke 永不返回导致 UI 永久卡死"），**符合 HCSE 韧性要求**。
3. **内联事件彻底清除**（`onclick=` = 0），统一 `data-action` 委托。
4. **前后端 API 契约实际一致性极高**（42 条调用零错配）。
5. **环境变量门控文档可追溯性 100%**（10 类门控全部定位到源码行号且默认值经逐行验证）。

### 6.5 并发韧性（局部优秀）

1. **生产代码无 `.lock().unwrap()`**（中毒统一 `into_inner()`）。
2. **全局 `TimeoutLayer` 30s + `ConcurrencyLimitLayer` 100**。
3. **桌面端文档化锁序 L1-L6**。
4. **sidecar `Drop` 守卫 + 进程守卫**。

---

## 七、结论

### 7.1 总体评价

LRC v0.9.7 是一个**工程成熟度较高**的项目：安全工程（SSRF/路径遍历/加密/供应链）与发布规范（版本一致性、clippy 零容忍、发布阻断）达到了较好的生产级水准；前端 XSS 防护体系化、API 契约零错配、并发韧性的超时兜底意识明确。

### 7.2 核心问题定位

本次审查识别出的风险**不集中在"代码写得对不对"，而集中在"绿了不等于对"的三个断层**：

| 断层 | 表现 | 对应风险 |
|---|---|---|
| **度量断层** | 无覆盖率门禁，未执行路径不可见 | P0-5（**已修复 f7**） |
| **执行断层** | 桌面端 80 个测试 + 全部非 `server` feature 路径 + 4/5 前端脚本从不执行 | P0-7、P2-2、P2-4（**已修复 f7 / r3 / r4 / r5**） |
| **可信断层** | 并行测试下 `set_var` 全局污染 + 6 处弱化断言，稀释既有绿灯语义 | P0-6、P1-1~P1-8（**已修复 f4 / f7 / r3 / r4**） |
| **可达断层** | 错误分类分支因上游语义抹平而**永不执行**（如 `Promise.allSettled` 抹平 rejection `reason` → 超时分支死代码） | HCSE 检查项 1.3（**已修复 r11 / r12**） |

此外有两类**结构性隐患**需要关注：

1. **`competition/` 目录物理残留**（P0-1，已撤销）——原判定"含 Layer 2 受保护源码"经实测证伪（`rust-src\src`/`static` 系 Junction，无副本）。**真正暴露的隐患是审查方法论**：对 ReparsePoint 的静默跳过 + 子代理数据未实测复核，会产生比真实故障更危险的"虚构风险"。此条已转为方法论改进项（见 5. HCSE 回写建议第 4 项）。→ **已回写（r7）**
2. **接口契约无门禁**（前端 P1-2）——`get_proxy_configuration` 缺失长期未被拦截，说明现有 `validate_frontend_contract.js` 的保护面仅限 DOM/静态资源层。→ **已修复（r5，扩展至 36 API 路径 + 33 invoke 命令）**

此外本轮第二轮修复还暴露了**第三个断层——交付断层**：`git diff` 全绿不等于"克隆可用"。`.gitignore` 中有 8 处规则（:298-310、:317-318、:339）把**本地开发链路脚本、钩子、乃至 HCSE 回写清单本身**排除在版本控制外，且这些排除是 `local-dev` 于 v0.8.45/v0.9.0/v0.9.1 三次发布清理中**刻意为之**（"不交付用户，本地开发自用"）。其中 HCSE 清单被排除构成**自指事故**（**r10 已解除忽略**，见 8.8）；开发链路脚本（`dev-proxy.py` / `run-dev.ps1` / `run-dev.bat`）**亦已于 r14 解除忽略**（实测 `.gitignore:439-441` 负向规则生效，`git check-ignore` 对三文件均返回 **EXIT=1** 即"未被忽略"）。**该断层已闭环**；剩余仅为"解除忽略 ≠ 已提交"——16 个交付物仍为未跟踪状态，须 `git add`（见 8.7 与 HCSE_RELEASE_PROTOCOL 检查项 4.3）。

### 7.3 风险分布统计

> **修复后状态（2026-09-13，七轮修复 f1~f10 + r1~r10 + r11~r12 + r13~r14 + r15~r16 + b1~b8 + b10~b13 后）**。

| 级别 | 数量 | 主要分布 | 本轮闭环 |
|---|---|---|---|
| **P0** | **7** | 架构 2、质量 2、测试 3 | **已修复 6 / 已撤销 1**（**P0-2 依赖倒置 b7 闭环**——原"待跟踪"已解除；见 8.12.3） |
| P1 | ~20 | 测试 8、并发 5、架构 4、质量 4、安全 2、前端 2 | **已修复 16 / 已更正 3 / 已定性更正 1**（f4/f5/f6/r2/r3/r4/r5/r11/r12/r15 + **b8a 错误 trait 契约、b8e 认证告警**；**b13 更正 feature 矩阵定性**；**c2 闭环 `spawn_blocking` 补偿路径**、**c4/c6 闭环错误处理统一**；r16 更正为误报） |
| P2 | ~18 | 测试 6、架构 4、安全 4、并发 4、前端/文档 3 | **已修复 18 / 已更正 1**（测试类 6：f7/r2；文档类 4：f8/r2 + **Tauri 命令归属 b12**；架构类 P2-1/P2-2/P2-3/P2-4：b8b/b8c/b8d + **API 层职责混合 b11**、**SidecarManager 持锁跨 Phase b11**、**c3/c7 God Object 字段级拆分**；并发类"裸 `.lock().await`"经核实非缺陷；D-9 无障碍 b4） |
| P3 | ~20 | 安全 5、测试 4、质量 3、并发 3、前端/文档 3、架构 1 | **已修复 11**（文档类 5：D-3/D-4/D-7/D-8/D-10；P3-1 空目录定性校正、P3-2 模型 ID、P3-3 原子写入收敛、`//!` 文档、`#[allow(dead_code)]` 核实——b1/b8b/b8d） |

> **计数口径说明**：本节按**第四章风险编号体系**（P0-n / P1-n / P2-n / D-n）统计；P1-7、P1-8、P2-3、P2-4 等编号在参考文献维度（第四章 D1 表）时含义不同（详见该表下方"编号体系说明"）。同一项跨维度出现时**只计一次**。

**安全维度无 P0**——这是项目的显著优势；风险最密集的维度是**测试与质量门禁**（P0-5/6/7 全部落在此维度），**该维度本轮已全部闭环**。

> **八轮修复的收敛边界（第八轮后已全部闭环）**：P0 级**已全部闭环**（含此前"仅登记"的 P0-2 依赖倒置）。
> 第七轮末剩余 4 项，**第八轮（c2~c8）已全部处置**：
> **P1-6 错误处理统一**（主 crate `Result<_, String>` 103 → 0，新建 `errors.rs`；
> "需引入 `thiserror`"经实测更正为"手写即可"）、**`MemoryStore` 字段级拆分**
> （两刀切片完成；"需先做 `RefCell → Sync`"经实测为**伪前置**）、
> **`spawn_blocking` 不可强杀**（改为**协作式取消补偿路径**，超时后最多多跑 63 次比较）、
> **后端选型**（定性维持为**产品决策**，并补证 Qdrant 的 `Unsupported` 属 fail-closed 正确设计）。
> **8.6 待跟踪清单已清空**。详见 8.14。

### 7.4 建议行动

1. **立即**：~~清理 `competition/rust-src/` 磁盘残留（P0-1）~~ **【已撤销】**；为 `audit_trail.rs` 安全承诺接线或删除（P0-4）。→ **均已完成（f1/f2）**
2. **本轮迭代**：CI 引入覆盖率门禁（P0-5）、测试串行化（P0-6）、桌面端测试接入（P0-7）、修复接口契约门禁（前端 P1-2）。→ **全部已完成（f7 + r5）**
3. **排期**：修复 `benchmarks.rs` 弱化断言、统一错误处理、文档同步、God Object 拆分。→ **全部已完成**：`benchmarks.rs` 断言（f4）、文档同步（f8）、错误处理契约层（b8a）+ **主体 103 处收敛（c4/c6）**、God Object 测试岛拆分（b8b）+ **字段级两刀切片（c3/c7）**。**该"排期"项已清空**（详见 8.6 / 8.14）
4. **HCSE 回写**：按第五节建议，将"超时机制验证""测试隔离性""跨层契约校验""工作区物理残留扫描"四类检查项回写项目检查清单文档。→ **已完成（r6/r7）**；回写产物 [HCSE_RESILIENCE_AUDIT.md](file:///g:/code-memory/docs/HCSE_RESILIENCE_AUDIT.md) / [HCSE_RELEASE_PROTOCOL.md](file:///g:/code-memory/docs/HCSE_RELEASE_PROTOCOL.md) 原被 `.gitignore` 刻意忽略（**回写动作自指事故**），经用户裁定已解除忽略（r10），纳入版本控制

---

## 八、修复执行记录（v0.9.7 修复轮）

> 本章记录本报告结论对应的**修复轮**执行情况，供复核人与后续维护者追溯。基线快照（第三～七章）保留审查时点数据，未逐行改写。

### 8.1 修复轮总览

| 轮次 | 目标 | 关联风险 | 涉及文件 | 验证方式 | 结果 |
|---|---|---|---|---|---|
| **f1** | 据实核查并更正 P0-1 虚构数据 | P0-1 | `docs/GLOBAL_CODE_REVIEW_REPORT.md` | 实测 `Get-ChildItem -Recurse -Force` / `Get-Item -Force`（Junction 判定） | 报告 6 处数据更正（`competition/rust-src/` 实为 Junction，非重复副本） |
| **f2** | `audit_trail.rs` 安全承诺处置 | P0-4 | `src/engine/audit_trail.rs` | `git diff` 确认符号删除；`Grep` 零残留引用 | 删除 `CRITICAL_EVENT_TYPES` / `append_to_file`；文档注释改写为"JSONL 只追加" |
| **f3** | `benchmark.rs` 库路径 `expect` 收敛 | P0-3 | `src/benchmark.rs` | `Grep "expect("` 计数 | 清零，改为 `BenchmarkError` + `Result` 传播 |
| **f4** | `benchmarks.rs` 弱化断言修复 | P1（测试 6 处） | `tests/benchmarks.rs` | `cargo test --offline --lib` | 恒真/同义反复/OR 逃生舱等 6 处改为硬断言 |
| **f5** | `get_proxy_configuration` IPC 命令补齐 | P1（前端） | `desktop/src-tauri/src/commands.rs`、`main.rs`、`static/app.js` | 命令定义/注册计数；`invokeWithTimeout` 3s 兜底 | 已定义 `:2845-2853` + 已注册 `main.rs:171` |
| **f6** | sidecar 健康检查超时口径统一 | P1（并发） | `desktop/src-tauri/src/sidecar_manager.rs` | `Grep` 三处常量/注释/日志 | 统一为 40s（`HEALTH_CHECK_REQUEST_TIMEOUT_SECS`） |
| **f7** | CI 门禁补强（覆盖率/串行化/桌面测试/license-check/ml feature/触发条件/timeout） | P0-5/6/7、P2 测试类 | `.github/workflows/ci.yml`、`security.yml` | PyYAML 解析校验；行数/Job 数实测 | `ci.yml` 679 行 7 Job；`security.yml` 2 Job |
| **f8** | 文档不一致修复（D-1~D-5、D-7、D-8）+ D-6 定性更正 | D-1~D-8 | `docs/USER_GUIDE.md`、`README.md`、`docs/MODEL_EVALUATION.md`、`docs/OFFLINE_MODEL_GUIDE.md`、`desktop/src-tauri/tauri.conf.json`、`desktop/package.json`、`desktop/package-lock.json`、`desktop/pnpm-lock.yaml` | Grep 计数；`git check-ignore` / `git ls-files` | 见 8.2~8.4 |
| **f9** | 全量回归验证 | 全部 | — | `cargo fmt --all -- --check`、`cargo clippy --offline --all-targets --features server,ml -- -D warnings`、`cargo test --offline --lib` | 三项全绿（见 8.5） |
| **f10** | 报告文档更新 | 本报告 | `docs/GLOBAL_CODE_REVIEW_REPORT.md` | 逐章回填状态列 | 本章 |

### 8.2 CI 门禁变更明细（f7）

| 变更 | 落地位置 | 关联风险 |
|---|---|---|
| 新增 `coverage` Job：`cargo llvm-cov --features server --lcov --output-path lcov.info --fail-under-lines 40 -- --test-threads=1` + lcov artifact | `ci.yml:160-207` | P0-5 |
| `test` Job 增 `timeout-minutes: 45` | `ci.yml:97-103` | P2-1 |
| `test` / `coverage` 均加 `--test-threads=1` | `ci.yml:132-140` | P0-6 |
| `test` Job 增 `cargo test --features server,ml` | `ci.yml:141-150` | P2-2 |
| `e2e-smoke` Job 增 `timeout-minutes: 25` | `ci.yml:217` | P2-1 |
| `build-matrix` 增 `cargo test (desktop lib)` | `ci.yml:546-552` | P0-7 |
| `frontend-test` 触发条件增 `head_commit.added` | `ci.yml:577-586` | P2-6 |
| `frontend-test` egress 白名单补 `registry.npmjs.org` / `archive.ubuntu.com` / `playwright.azureedge.net` 等 | `ci.yml:594-617` | P2-6（harden-runner 阻断） |
| `security.yml` `license-check` 改为**真实阻断**（检出 GPL/AGPL/LGPL 即硬失败）+ `timeout-minutes: 20` | `security.yml:70-119` | P2-5、P2-1 |

**实测计数**：`ci.yml` 行数 **679**（审查时点 581）；Job 数 **7**（审查时点 6）；`timeout-minutes` 出现 **7** 次（三 workflow 合计 **9** 次）。

### 8.3 源码修复明细（f2~f6）

| 文件 | 变更 | 验证 |
|---|---|---|
| [audit_trail.rs](file:///g:/code-memory/src/engine/audit_trail.rs) | 删除 `CRITICAL_EVENT_TYPES` 与 `append_to_file`（从未被调用）；文档注释改为"JSONL 文件为只追加（append-only）…溢出的事件仍可随时从文件重新加载并检索" | `git diff` 确认；`Grep` 零残留引用 |
| [benchmark.rs](file:///g:/code-memory/src/benchmark.rs#L25-L28) | 新增 `pub type BenchmarkError = crate::persistence::PersistenceError;`；`make_store() -> Result<(TempDir, MemoryStore<JsonPersistence>), BenchmarkError>` | `Grep "expect("` 归零 |
| [benchmarks.rs](file:///g:/code-memory/tests/benchmarks.rs) | `:358-361` 恒真断言 → `high_count > 0`；`:363-366` 新增 `high_count >= low_count` 排序关系；`:438` 新增 `assert!(!result.memories.is_empty(), ...)`；`:517-529` 抗污染测试补 `remember_batch` 说明 + 关闭 `LRC_STATE_BIAS`；`:564-567`/`:570-574` 噪声门禁由 `eprintln!` 升级为硬断言（`consistency >= 3` 且噪声 ≤ 2）；`:670-702` 审计篡改测试改为经 `record_audit` 显式落事件 | `cargo test --offline --lib` 625 passed |
| [commands.rs](file:///g:/code-memory/desktop/src-tauri/src/commands.rs#L2845-L2853) / [main.rs](file:///g:/code-memory/desktop/src-tauri/src/main.rs#L171) | 新增并注册 `get_proxy_configuration`（**36 定义 = 36 注册** 重新对等） | 契约测试 `:2898-2899`；`cargo test`（desktop lib）90 passed |
| [app.js](file:///g:/code-memory/static/app.js) | `get_proxy_configuration` 调用改 `invokeWithTimeout(invokeFn, 'get_proxy_configuration', undefined, 3000)` | 3s 硬超时兜底，避免 invoke 不返回导致 UI 卡死 |
| [sidecar_manager.rs](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L53-L66) | 健康检查超时口径三者统一（注释 / `HEALTH_CHECK_REQUEST_TIMEOUT_SECS`=40s / 日志）；`:1218` 细粒度取消检查 `HEALTH_CHECK_CANCEL_CHECK_STRIDE` | 文档注释与实现一致 |

### 8.4 文档与配置修复明细（f8，含 D-6 定性更正）

| 编号 | 变更 | 落地 | 验证 |
|---|---|---|---|
| D-1 | USER_GUIDE 版本号 `v0.9.5` → `0.9.7` | `docs/USER_GUIDE.md:5` | 跨端版本号五处一致 |
| D-2 | USER_GUIDE CORS 描述改为与代码一致（移除 `0.0.0.0` 表述） | `docs/USER_GUIDE.md:589` | 与 `server.rs:3789-3790` 对齐 |
| D-3 / D-4 | README 图标/Logo 数量按实测更正（56 / 3） | `README.md:168-169` | 目录实测计数 |
| D-5 | "统一从 Cargo.toml 读取版本号"表述更正为"Rust 侧 `CARGO_PKG_VERSION` + 前端硬编码 fallback + `/v1/health/system` 校正" | `docs/USER_GUIDE.md:803` | 与 `app.js:8`、`/v1/health/system` 实现对齐 |
| D-6 | **定性更正 + 修复**：删除 `"devUrl"` 与 `"beforeDevCommand"`，改用 Tauri 内置 dev server（仅保留 `frontendDist` + `beforeBuildCommand: ""`） | `desktop/src-tauri/tauri.conf.json:6-9` | 见下方"D-6 影响说明" |
| D-7 | 默认模型代际冲突对齐（MODEL_EVALUATION / OFFLINE_MODEL_GUIDE → BGE 为默认） | `docs/MODEL_EVALUATION.md:16`、`docs/OFFLINE_MODEL_GUIDE.md:111` | 与 `README.md:121` 对齐 |
| D-8 | `ws` 空依赖清除（3 处：`package.json` / `package-lock.json` / `pnpm-lock.yaml`） | `desktop/package.json:16-19` | **`pnpm-lock.yaml` Grep `ws` → No matches found**；净删 37 + 15 行 |

**D-6 影响说明（本地开发方式变更，务必知悉）**：

删除 `beforeDevCommand` 后，`tauri dev` 由 Tauri CLI 内置 dev server 直接服务 `static/` 目录（官方行为：仅设 `frontendDist` 且目录含 `index.html` 时自动启用），**功能上无回归**。但**丧失了 `scripts/dev-proxy.py` 提供的三项本地开发增强**，需在需要时**手动启动**：

```powershell
python G:\code-memory\scripts\dev-proxy.py 1420 --dev
```

| 丧失的能力 | 后果 | 补偿方式 |
|---|---|---|
| `no-store` 缓存头 | WebView2 启发式缓存旧 `app.js`（注释明示为 **v0.9.7 调试踩坑根因**） | 手动运行 dev-proxy，或开发时禁用 WebView2 缓存 |
| 30s 代理超时 | 代理层超时兜底消失 | 依赖前端 `fetchWithTimeout`（76 处）与 CI 侧超时 |
| 端口改写（`SIDECAR_PORT` 3111 dev / 3099 生产） | 需依赖默认端口约定 | 显式设置环境变量 |

> **风险提示（HCSE 回写候选）**：`scripts/dev-proxy.py`、`scripts/run-dev.ps1`、`scripts/run-dev.bat`、`scripts/cdp_*.js` 等**整套本地开发链路均被 [.gitignore:298-305](file:///g:/code-memory/.gitignore#L298-L305) 忽略**（注释："v0.8.45 发布合规：本地开发运行目录与临时文档（非交付内容）"）。新克隆环境的开发者将**无法复现本地开发/调试链路**。建议回写 `docs/HCSE_RELEASE_PROTOCOL.md`（新增"本地开发链路可交付性"检查项）。

### 8.5 全量回归验证结果（f9）

| 验证项 | 命令 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all -- --check` | **`FMT_CHECK_EXIT=0`** |
| 静态检查 | `cargo clippy --offline --all-targets --features server,ml -- -D warnings` | **`CLIPPY_EXIT=0`** |
| 单元测试 | `cargo test --offline --lib` | **`TEST_EXIT=0`**；`test result: ok. 625 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.98s` |
| 桌面端 lib 测试 | `cd desktop/src-tauri && cargo test` | **90 passed / 0 failed** |
| Workflow YAML | PyYAML 解析 `ci.yml` + `security.yml` | **PASS** |

**过程中修复的回归**：

1. `cargo fmt --all -- --check` 报 `tests\benchmarks.rs:729` 差异（f4 引入）→ `cargo fmt --all` 收敛 → 复检 `FMT_CHECK_EXIT=0`。
2. `cargo clippy` 报 `src\v1_api.rs:8438:13 manual RangeInclusive::contains implementation` → 改为 `(HTTP_TIMEOUT_SECS - 1.0..=HTTP_TIMEOUT_SECS + TIMEOUT_TOLERANCE_SECS).contains(&fallback_secs)` → 复检 `CLIPPY_EXIT=0`。

### 8.6 未闭环项（待跟踪清单）

**已在第二轮修复轮（r1~r10）闭环的项**（详见 8.8）：

| 编号 | 项 | 闭环方式 |
|---|---|---|
| P1-2 | 接口契约无 CI 门禁 | `validate_frontend_contract.js` 扩展跨层校验（36 API 路径 + 33 invoke 命令），接入 CI/Release 门禁 |
| P1-7 | `association-user-view-check.js` 无断言 | 新增 `assert()` + 6 条硬断言 |
| P1-8 | `playwright-smoke.js` console/network 失败不阻断 | 新增 `unexpectedConsoleErrors` / `unexpectedNetworkFailures` 过滤 + 两条阻断断言 |
| D-10 | `desktop/package-lock.json` 版本号漂移 | 版本号对齐 `0.9.7` |
| D-11 | 钩子不在版本控制 | 新建 `.githooks/pre-commit`（v2.1）+ `scripts/enable_git_hooks.ps1`，README 补启用说明 |
| — | HCSE 四类检查项回写 | 已落盘 [HCSE_RESILIENCE_AUDIT.md](file:///g:/code-memory/docs/HCSE_RESILIENCE_AUDIT.md)（1、2 项）+ [HCSE_RELEASE_PROTOCOL.md](file:///g:/code-memory/docs/HCSE_RELEASE_PROTOCOL.md)（3、4 项）；**两文档原被 `.gitignore:318/:339` 忽略（local-dev 2026-08-17 刻意排除），经用户裁定已解除忽略，纳入版本控制，可随克隆交付**（见 8.8 r10） |

**仍未闭环项**：

> **第八轮（c2~c8）状态更新**：原列 4 项**全部已处置**——详见 8.14。
> 下表保留"第七轮时点"的原始判定，并逐行标注第八轮结果（**含 2 项被实测推翻的原判定**）。

| 编号 | 项 | 第七轮原因 | 第八轮结果 |
|---|---|---|---|
| P1-6 | 错误处理统一（**部分闭环**） | b8a 已补齐 6 个 error enum 的 `Display`/`Error`/`source()` 契约；但 **74 处 `Result<_, String>` 签名改造**（多为 trait 契约与 CLI 边界）需引入 `thiserror` 并改动跨层签名，属**独立排期** | **已闭环（c4+c6）**：新建 `errors.rs`（`ErrorKind` 10 域）统一收敛主 crate **103 → 0** 处；`url_safety.rs` 因 `include!` 约束手写 enum；**"需引入 `thiserror`"经实测更正——手写 `Display`/`Error` 即可**；桌面端 67 处经取证全在 IPC 边界，**零改动为正确结果** |
| P2-2 | `MemoryStore` God Object 本体 | b8b 已外提测试岛（`memory_store.rs` 6981→4612 行）；但 **27 字段 / 18 pub / 主体 impl 约 3616 行**的字段级拆分需先完成 `RefCell → Sync` 并发模型改造，属**独立排期** | **已闭环（c3+c7）**：两刀切片——纯数据契约外提至 `memory_store_types.rs`、6 个缓存字段外提至 `memory_store_cache.rs`；**"需先做 `RefCell → Sync` 改造"经实测为伪前置**（`Mutex<T>: Sync` 只要求 `T: Send`，实测全仓 8 处共享方式均为 `Arc<Mutex<MemoryStore<P>>>`），详见 8.14.5 |
| — | QdrantStore 9/13 方法 Unsupported、Neo4jGraphStore 语义错位 | 后端选型未完成，属**产品决策**而非缺陷 | **定性维持（c8）**：补取证确认 `qdrant.rs` 10 处 `Unsupported` **全部是"语义不可保证下的 fail-closed"**（如"无法保证全量替换的原子语义"），属**正确设计**而非实现缺失。选型（是否引入更强后端）仍为产品决策 |
| — | `spawn_blocking` 超时后不可强杀 | **受 Rust 运行时限制**（已提交任务无法强杀）；现有缓解为"取消标志 + `Release` 写入 + 5 处 `Acquire` 检查点"（r15 已统一内存序） | **已闭环（c2）**：报告原文要求"可中断**或补偿**"，本轮实现**补偿路径**——`SynthesisEngine` 新增 3 个可取消变体（`cluster_from_all_cancellable` / `plan_jaccard_cancellable` / `plan_luoshu_cancellable`），`consolidation.rs` 与 `v1_api.rs` 的 `spawn_blocking` 均已接线；把"超时后跑满 12.5 万次比较"降为"**最多多跑 63 次**"，且**取消路径绝不写回不完整计划** |

> **第八轮闭环结论**：**8.6 待跟踪清单已全部清空**。项目当前不存在"已知且未处置"的
> 结构性缺陷；剩余事项仅为**产品决策**（后端选型）。

> **已更正定性（第七轮实测）**：原列的「**feature 矩阵失效**」判定**不成立**——
> `qdrant=[]`/`neo4j=[]` 的**空 feature 是正确设计**（其后端走 HTTP，不引入额外依赖，实测 `cargo check` EXIT=0）；
> `tokio` **不能**改 optional（被 3 个非 server 门控模块使用）；`reqwest` 改造成本高于收益。
> 实测 6/7 个 feature 组合编译通过（唯一失败的 `postgres` 系**离线依赖缺失**，非代码缺陷）。详见 8.13.4。

**已在第七轮修复轮（b10~b13）闭环的项**（详见 8.13）：

| 编号 | 项 | 闭环方式 |
|---|---|---|
| P2-3 | API 层职责混合（`server.rs` 混入桌面环境探测） | b11：删除 168 行**注释化死代码**（`detect_command_tool` / `which_path` / `check_windows_install_path` / `check_vscode_extension`） |
| 并发 P2-2 | SidecarManager 持锁跨 Phase | b11：删除 `start()` / `start_for_project()` / `restart_project()` **3 个零调用方 API** → 该模式在**类型层面不可达** |
| — | Tauri 命令归属不明（原报告"7 个"） | b12：实测为 **4 个孤儿命令**（均有明确归属），在注册处**显式标注** + **新增契约门禁双向校验**（含对照实验确证门禁可失败） |
| — | feature 矩阵失效（**定性更正**） | b13：实测证明空 feature 是**正确设计**；`tokio` 不可 optional；`postgres` 失败系**离线依赖缺失**。详见 8.13.4 |

**已在第六轮修复轮（b1~b8）闭环的项**（详见 8.12）：

| 编号 | 项 | 闭环方式 |
|---|---|---|
| P0-2 | 依赖倒置 Layer 1 → Layer 2 | b7：`memory_state_machine.rs` 上提 Layer 1（**关键发现：许可层错位而非架构错**；SHA256 前后一致） |
| P1-6 | 错误处理不统一（**契约层**） | b8a：6/6 error enum 补齐 `Display`/`Error`/`source()` |
| P2-2 / P2-4 | God Object / 测试占比 50% | b8b：`v1_api.rs` 9012→4529 行（测试占比 50%→0%）、`memory_store.rs` 6981→4612 行；等价性由 625 tests 不变证明 |
| P2-3 | 并发「锁内磁盘 IO」 | b8c：锁序契约文档化，明确"持锁磁盘 IO 是有意接受（正确性优先于吞吐）" |
| P2-3 | 并发「备份锁序未文档化」 | b8c：`BACKUP_OPERATION_LOCK` 锁序 + 与持久层无环路证明落盘 |
| D-9 | 无障碍属性稀疏 | b4：`aria-label` 12→37（HTML）+ 0→7（JS）、`role="button"` 0→21、`tabindex="0"` 0→21 |
| — | `competition/` 目录去留 | b6：README 补 Junction 说明 + PS5.1 合规核查代码块 |
| — | CI/Release `node --check` 清单不一致 | b5：两清单统一为 **6 个脚本** |
| P3-3 | 原子写入逻辑重复 4 处 | b8b：新建 [atomic_file.rs](file:///g:/code-memory/src/atomic_file.rs) 收敛为单一实现 |
| P2-1 | `use` 语句被函数截断 | b8d：`memory_store.rs` / `v1_api.rs` 的导入块已归位 |
| P1-4 | 行尾注释错位 | b8d：`engine/mod.rs:22` / `:68` 已按归属拆分 |
| P2 质量 | 过时 `#[allow(dead_code)]` | b8d：**移除实验**判定——16 处证实**非冗余**（回填并注明）、3 处确属冗余（删除） |
| P3 质量 | `//!` 模块级文档为 0 | b8d：25/26 个顶层模块补齐（`url_safety.rs` 因 `include!` 冲突刻意保留 `//`） |
| P3-1 | `engine/archive/` 空目录 | b8d：**定性校正**——CHANGELOG 明文约定的归档位，非残留 |
| P1 安全 | 认证默认策略 | b8e：非回环绑定且未设 Token 时**启动告警** |

> **注**：原「本地开发链路（dev-proxy / run-dev）未交付」项已于第四轮（r13~r14）闭环，经用户裁定「解除忽略，纳入版本控制」+「补齐降级」，见 8.10。

**已在第三轮修复轮（r11~r12）闭环的项**：

| 编号 | 项 | 闭环方式 |
|---|---|---|
| — | **前端超时路径无端到端测试**（HCSE 检查项 1.3 缺口） | r11 修复 [app.js](file:///g:/code-memory/static/app.js#L1434-L1444) 超时误分类（`Promise.allSettled` 抹平 rejection `reason` 致 `SidecarTimeoutError` 分支为死代码）+ r12 在 [playwright-smoke.js](file:///g:/code-memory/tests/frontend/playwright-smoke.js#L85-L127) 内嵌超时/卡死路径 E2E（含对照实验确证），详见 8.9 |

**已在第四轮修复轮（r13~r14）闭环的项**：

| 编号 | 项 | 闭环方式 |
|---|---|---|
| — | **本地开发链路（dev-proxy / run-dev）未交付**（HCSE 检查项 3 缺口） | r13 为 [association-desktop-cdp.js](file:///g:/code-memory/tests/frontend/association-desktop-cdp.js#L164-L181) 补 probe 降级（对齐同目录既有模式）+ r14 解除三脚本 `.gitignore` 忽略、README 增「本地开发链路」说明，并**实跑 dev-proxy 验证**（200 / 130170 字节 / `no-store` / 端口改写 3111），详见 8.10 |

**已在第五轮修复轮（r15~r16）闭环的项**：

| 编号 | 项 | 闭环方式 |
|---|---|---|
| P1（并发） | **取消标志内存序不一致** | r15：[memory_store.rs:3225](file:///g:/code-memory/src/memory_store.rs#L3225) 由 `Relaxed` 统一为 `Acquire`，与同标志其余 4 处检查点及写入端 `Release` 配对（实测 `Acquire`=5 / `Relaxed`=0） |
| P1（质量） | **生产路径 `expect`（`v1_api.rs` 3 处 + `json.rs` 4 处）** | r15：SSRF 校验后 3 处改显式错误返回；`json.rs` 缓存初始化 4 处改 `ok_or_else` 返回 `PersistenceError`（实测生产路径 `real_expect=0`） |
| P1（安全） | **「缺请求体大小限制」** | **r16 更正定性：实为误报**——axum-core 无自定义层时默认套用 2MB `DefaultBodyLimit`，权威代码与文档双重取证（见 8.11） |

### 8.7 修复轮工作区变更统计

```
74 files changed, 3186 insertions(+), 9240 deletions(-)
```

（`git diff --shortstat HEAD`，**2026-09-13 实测**，含第一~八轮全部改动：f1~f10、r1~r16、b1~b8、b10~b13、c2~c8。
删除量较大（9240）的主因是**测试岛外提**（b8b）：`v1_api.rs` 移出 4484 行、`memory_store.rs` 移出 2366 行，
二者以新文件形式（`v1_api_tests.rs` / `memory_store_tests.rs`）存在，属**未跟踪文件**，
其新增行**不计入** `git diff`（详见下节口径说明）；另有 b11 删除的 `server.rs` 168 行注释化死代码
与第八轮 `memory_store.rs` 外提（`memory_store_types.rs` + `memory_store_cache.rs`）。
**未含本报告文件自身**——本报告为未跟踪文件，不进入 `diff` 统计。）

**未跟踪但未被忽略的文件**（`git ls-files --others --exclude-standard`，即会随克隆交付的候选）：

| 文件 | 归属轮次 |
|---|---|
| `.githooks/pre-commit` | r2（D-11） |
| `scripts/enable_git_hooks.ps1` | r2（D-11） |
| `docs/HCSE_RESILIENCE_AUDIT.md` | r6 |
| `docs/HCSE_RELEASE_PROTOCOL.md` | r7 |
| `scripts/dev-proxy.py` | r14 |
| `scripts/run-dev.ps1` | r14 |
| `scripts/run-dev.bat` | r14 |
| `src/model_ids.rs` | b1（模型 ID 单一真源） |
| `src/atomic_file.rs` | b8b（P3-3 原子写入收敛） |
| `src/v1_api_tests.rs` | b8b（P2-4 测试岛外提） |
| `src/memory_store_tests.rs` | b8b（P2-4 测试岛外提） |
| `src/memory_state_machine.rs` | b7（P0-2 上提 Layer 1；同时 `src/engine/memory_state_machine.rs` 被删除） |
| `src/errors.rs` | **c6**（P1-6 统一错误契约） |
| `src/memory_store_types.rs` | **c3**（P2-2 数据契约外提） |
| `src/memory_store_cache.rs` | **c7**（P2-2 缓存子系统外提） |
| `docs/GLOBAL_CODE_REVIEW_REPORT.md` | 本报告 |

> **口径说明**：`git diff --stat` 只统计**已跟踪文件的改动**，不含上述未跟踪文件；故 8.5/8.14.7 节"回归全绿"与本节统计是两套口径，前者验证**代码正确性**，后者衡量**交付面**。八轮累计 **16 个未跟踪文件**（实测 `git ls-files --others --exclude-standard`），**尚待 `git add` 后方可随克隆交付**。
>
> **⚠ 其中 8 个是 `src/*.rs` 源文件**（`model_ids.rs`、`atomic_file.rs`、`v1_api_tests.rs`、`memory_store_tests.rs`、`memory_state_machine.rs`、**`errors.rs`**、**`memory_store_types.rs`**、**`memory_store_cache.rs`**）——**若未提交，克隆环境将编译失败**（`mod` 声明找不到文件）。这比"脚本缺失"更严重，发布前必须优先提交。
>
> **被忽略条目现状（实测）**：`docs/` 下 114 项（r10 解除两份 HCSE 清单后由 116 降至 114）、`scripts/` 下 55 项（r14 解除三脚本后的当下值；`cdp_*.py`、`verify_*.ps1`、`run-dev.*` 通配规则下的历史脚本仍被忽略）。

### 8.8 第二轮修复轮记录（r1~r10）

> 第一轮（f1~f10）见 8.1~8.5，第三轮（r11~r12）见 8.9。第二轮针对"审查时点未闭环项"继续推进，起因是复核发现**门禁类缺陷**（契约无 CI、脚本无断言、钩子不可分发）与**交付类缺陷**（回写文档被忽略）。

| 轮次 | 目标 | 关联风险 | 落地位置 | 结果 |
|---|---|---|---|---|
| **r1** | `desktop/package-lock.json` 版本号漂移 | D-10 | `desktop/package-lock.json` | 版本号对齐 `0.9.7` |
| **r2** | 钩子不在版本控制（不可分发） | D-11 | `.githooks/pre-commit`（v2.1）、`scripts/enable_git_hooks.ps1`、`README.md` | 钩子脚本落盘 + README 补启用说明；两文件已进入"未跟踪未忽略" |
| **r3** | `association-user-view-check.js` 无断言（恒过） | P1-7 | `tests/frontend/association-user-view-check.js` | 新增 `assert()` 助手 + 6 条硬断言 |
| **r4** | `playwright-smoke.js` console/network 失败不阻断 | P1-8 | `tests/frontend/playwright-smoke.js` | 新增 `unexpectedConsoleErrors` / `unexpectedNetworkFailures` 过滤 + 2 条阻断断言 |
| **r5** | 接口契约无 CI 门禁 | P1-2（前端） | `scripts/validate_frontend_contract.js` | 扩展跨层校验：**36 条 API 路径 + 33 个 invoke 命令名**；接入 CI / Release 门禁 |
| **r6** | HCSE 回写：超时机制验证 / 测试隔离性 | 第五节建议 1、2 | `docs/HCSE_RESILIENCE_AUDIT.md`（新建，88 行） | 检查项 1（超时 5 层基线 + 5 条强制步骤 + 缺口）+ 检查项 2（25 处 `set_var` + 4 条隔离步骤）+ 检查项 3（五层交互韧性覆盖） |
| **r7** | HCSE 回写：跨层契约校验 / 工作区物理残留扫描 / 本地开发链路可交付性 | 第五节建议 3、4 | `docs/HCSE_RELEASE_PROTOCOL.md`（新建，约 104 行） | 检查项 1（36 API / 33 invoke / 6 导航 259 DOM id）+ 检查项 2（ReparsePoint 显式判定，competition 68 条目 / 2 Junction）+ 检查项 3（本地开发链路可交付性） |
| **r8** | 全量回归 | 全部 | — | `FMT_CHECK_EXIT=0` / `CLIPPY_EXIT=0` / `TEST_EXIT=0`（625 passed）/ desktop lib 90 passed / `node --check` 与契约脚本实跑通过 |
| **r9** | 报告回填（本章） | 本报告 | `docs/GLOBAL_CODE_REVIEW_REPORT.md` | 8.6/8.7/8.8 更新 |
| **r10** | HCSE 双文档被 `.gitignore` 忽略（交付性缺陷） | 本轮取证发现（自指事故） | `.gitignore:424-425`、`docs/HCSE_RELEASE_PROTOCOL.md` 检查项 3.1 | 见下方"r10 取证与处置" |

> **r5 计数口径（实测复现，2026-09-12）**：实跑 `node scripts/validate_frontend_contract.js` → EXIT=0，输出 `前端契约通过：6 个导航、259 个 DOM id、静态资源完整、跨层契约 36 条 API 路径与 33 个 invoke 命令全部对齐`。
>
> - **与第四章「35 条 `/v1` + 7 条 `/api`」的口径差异**：第四章为**调用点计数**（含重复调用、未去重）；本处 36 为脚本 `new Set(apiCalls.map(c => c.routePath)).size` 的**去重路径数**（含 `/v1/*`、`/api/*` 及 `mcp`/`health` 裸路径）。两者非同一度量，**不可互相校验**——引用时须注明口径。
> - **覆盖边界**：脚本校验调用实参字面量中含 `/v1/` 或 `/api/` 的调用。若 URL 由变量拼接（动态段）则**不会被解析**，属当前门禁的已知盲区。
> - **已闭环（b5）**：`association-user-view-check.js` 原**未被任何 `node --check` 覆盖**（CI 3 脚本 / Release 4 脚本清单均未含）；第六轮已将两清单统一为 **6 个脚本**，见 8.12.1。

**r10 取证与处置（本轮 PowerShell 专家技能取证，证据落盘 `temp/`）**：

1. **取证**：`git check-ignore -v` 显示 [HCSE_RESILIENCE_AUDIT.md](file:///g:/code-memory/docs/HCSE_RESILIENCE_AUDIT.md) 命中 `.gitignore:318`（`docs/hcse_resilience_*.md` 通配）、[HCSE_RELEASE_PROTOCOL.md](file:///g:/code-memory/docs/HCSE_RELEASE_PROTOCOL.md) 命中 `.gitignore:339`；`git blame -L 337,340` 证实该规则由 `local-dev` 于 2026-08-17 提交，注释为"v0.9.1 发布清理：开发/发布流程文档（**不交付用户，本地开发自用**）"。
2. **定性**：这构成**自指事故**——"回写检查清单"这一动作**自身**产出了不可交付的文档，与 D-11"钩子不可分发"**同源**（均为本地开发产物被刻意排除）。
3. **处置（用户裁定：解除忽略，纳入版本控制）**：在 [.gitignore:418-426](file:///g:/code-memory/.gitignore#L418-L426) 追加两条负向规则（负向规则须置于对应忽略规则**之后**，故集中于文件末尾）：
   - `!docs/HCSE_RELEASE_PROTOCOL.md`
   - `!docs/HCSE_RESILIENCE_AUDIT.md`
4. **复检**：`git check-ignore -v` 对两文档命中 `!` 负向规则（**未忽略**）；`git ls-files --others --exclude-standard -- docs` 已列出两文档；`docs/` 下被忽略条目 **116 → 114**（减 2）。
5. **边界**：Cargo 打包走 `Cargo.toml` 的 `include`/`exclude`，与本文件无关；本例外仅使两文档可提交，**不会进入 crate / Tauri 发布产物**。仍被 `.gitignore:318` 通配忽略的 `docs/HCSE_RESILIENCE_AUDIT_REPORT_L6.md` 与 `docs/hcse_resilience_v0.8.42.md` 属**历史归档**，负向规则未覆盖（有意为之）。

**用户裁定的"仅登记、不启动"项（第二轮；第六轮已按新指令全部启动并闭环）**：

> 用户在第六轮将范围升级为「直到文档中待修复的**全部**修复完成」，故下表 5 项**已于第六轮全部处理**，此处保留以记录裁定的演进过程。

| 编号 | 项 | 第二轮登记结论 | 第六轮处置 |
|---|---|---|---|
| P0-2 | 依赖倒置 Layer 1 → Layer 2 | 结构性重构，需独立设计与排期 | **已闭环（b7）** |
| P1-6 | 错误处理不统一（缺 `thiserror` 分层） | 改动面大，需独立排期 | **已全部闭环（b8a 契约层 + c4/c6 主体）**：主 crate `Result<_, String>` **103 → 0**（新建 `errors.rs`）；**"缺 `thiserror`"经实测更正——手写 `Display`/`Error` 即可且不新增依赖** |
| D-9 | 无障碍属性稀疏（`aria-` 12 vs `data-action` 112） | 前端专项，需独立排期 | **已闭环（b4）** |
| — | `competition/` 目录去留 | 已取证其 2 个 Junction 指向主仓 `src/`、`static/`，待补 README 说明 | **已闭环（b6）** |
| — | CI / Release 的 `node --check` 脚本清单不一致 | CI 3 脚本 / Release 4 脚本，`association-user-view-check.js` 均未覆盖 | **已闭环（b5）** |

### 8.9 第三轮修复轮记录（r11~r12）

> 起因：推进 8.6 中**未被裁定"仅登记不启动"**的遗留项时，发现 HCSE 检查项 1.3 的判定**低估了问题严重性**——前端超时路径不仅"无测试"，其 UI 分支本身就是**不可达死代码**。（第一轮见 8.1~8.5，第二轮见 8.8，第四轮见 8.10）

| 轮次 | 目标 | 关联风险 | 落地位置 | 结果 |
|---|---|---|---|---|
| **r11** | 修复超时误分类（`SidecarTimeoutError` 分支死代码） | HCSE 检查项 1.3 / 五层模型 L1 异常路径 | [app.js:1434-1444](file:///g:/code-memory/static/app.js#L1434-L1444) | `Promise.allSettled` 抹平 rejection `reason` → 回查三请求 rejection，优先还原 `SidecarTimeoutError` 再抛出 |
| **r12** | 内嵌超时/卡死路径 E2E | HCSE 检查项 1.3 缺口 | [playwright-smoke.js:85-127](file:///g:/code-memory/tests/frontend/playwright-smoke.js#L85-L127) | `page.route` 挂起 `/v1/health/**` → 断言超时文案 + 重试入口 + loading 收起 + 页面可交互（5 条断言） |

**r11 根因链（六钥匙·分解所得）**：

1. `loadDashboard` 用 `Promise.allSettled` 并发 `/v1/health/system`、`/detail`、`/dao_metrics` 三请求；
2. `allSettled` **永不 reject**，rejection 的 `reason` 被封装进结果对象；
3. 随后 `!systemData && !daoData` 分支**无条件**抛 `无法连接到 API 服务`；
4. 故 catch 中 `e.name === 'SidecarTimeoutError'` 分支（渲染"请求超时"+重试按钮）**永不执行**——用户超时被误导为"服务未启动"。

**r12 验证证据（真实浏览器执行，非静态检查）**：

| 项 | 实测值 |
|---|---|
| 超时触发耗时 | **11121ms / 11153ms**（两次独立运行）——与 `fetchWithTimeout` 默认 **10s** 硬超时吻合，证明**超时真正触发**而非仅存在于代码中 |
| 超时态文案 | `请求超时，请检查网络连接后重试` |
| 重试入口 | `hasRetryButton=true`、`retryDisabled=false` |
| UI 恢复 | `loadingHidden=true`、`pageInteractive=true` |
| **对照实验** | 临时还原旧逻辑后复跑 → **失败**（25s 内永不出现"请求超时"文案），**确证原分支为死代码**，修复具因果必要性 |
| 恢复复检 | 还原修复后复跑 → 再次通过，结果与首次一致 |

**门禁验证（PowerShell 专家技能，证据落盘 `temp/`）**：

| 验证项 | 结果 |
|---|---|
| `node --check static/app.js` | EXIT=0 |
| `node --check tests/frontend/playwright-smoke.js` | EXIT=0 |
| `node --check tests/frontend/cdp-regression.js` | EXIT=0 |
| `node scripts/validate_frontend_contract.js` | EXIT=0（36 API 路径 / 33 invoke 命令全对齐） |
| 断言/标记计数 | `assert(` × 15、`page.route(` × 1、`page.unroute(` × 1 |

**环境说明（重要，避免误判）**：本机 3099 端口运行的是**已安装的 LRC Desktop v0.9.6 侧车**（`lrc-sidecar.exe`，服务旧版前端，无 `value-hero` 锚点），与工作区 v0.9.7 源码**失配**——直接对 3099 跑 `playwright-smoke.js` 会**在首个既有断言处失败**，该失败**与本轮改动无关**。r12 的验证改用"本地静态服务托管工作区 `static/` + 路由挂起"的隔离方案取得，CI 侧则由 `frontend-test` Job 构建 v0.9.7 sidecar 后执行。

**回写**：本轮结论已同步至 [HCSE_RESILIENCE_AUDIT.md](file:///g:/code-memory/docs/HCSE_RESILIENCE_AUDIT.md) 检查项 1.4（含新增强制步骤：**任何 `Promise.allSettled` 聚合必须保留 rejection `reason` 语义**）。

### 8.10 第四轮修复轮记录（r13~r14）

> 起因：推进 8.6 中最后一项**未被裁定"仅登记不启动"**的遗留项——本地开发链路交付性。取证发现其性质与 r10 **同源但影响更大**：被忽略的不是文档，而是**被 3 个已跟踪测试文件引用的可执行脚本**。

| 轮次 | 目标 | 关联风险 | 落地位置 | 结果 |
|---|---|---|---|---|
| **r13** | 补齐 dev-proxy 降级保护 | HCSE 检查项 3.2 步骤 1（引用即契约） | [association-desktop-cdp.js:164-181](file:///g:/code-memory/tests/frontend/association-desktop-cdp.js#L164-L181) | 由"无条件导航 1420"改为 probe 降级，对齐同目录既有模式 |
| **r14** | 交付本地开发链路三脚本 | HCSE 检查项 3.3 缺口 | [.gitignore:427-441](file:///g:/code-memory/.gitignore#L427-L441)、[README.md:36-49](file:///g:/code-memory/README.md#L36-L49) | 解除 `dev-proxy.py` / `run-dev.ps1` / `run-dev.bat` 忽略 + README 增「本地开发链路」段 |

**取证结果（PowerShell 专家技能，反向引用扫描）**：

| 事实 | 实测 |
|---|---|
| 被忽略的开发链路脚本 | `dev-proxy.py`（165 行）、`run-dev.ps1`、`run-dev.bat`，原被 [:303-305](file:///g:/code-memory/.gitignore#L298-L305) 忽略（`local-dev` v0.8.45 归入"非交付内容"） |
| **引用它们的已跟踪文件** | `cdp-regression.js` ×6、`association-dashboard-cdp.js` ×4、`association-desktop-cdp.js` ×2 |
| 降级能力（修复前） | 前两者**已有** probe 降级；`association-desktop-cdp.js` **无**（硬失败） |
| 文档说明 | README / USER_GUIDE **零处**提及如何获取 |
| 配置引用 | 无（D-6 已移除 `tauri.conf.json` 的 `beforeDevCommand`） |
| 安全性 | 三脚本均无硬编码绝对路径（相对路径 / `Get-Location` / `%~dp0..`），无 token/secret |

**用户裁定**：① 交付策略 →「**解除忽略，纳入版本控制**」；② 降级保护 →「**补齐降级**」。

**r14 实测验证（真实运行 dev-proxy，非静态检查）**：

| 验证项 | 实测值 |
|---|---|
| 解忽略复检 | 三脚本 `check-ignore -v` 命中 **`!` 负向规则**（[:439/:440/:441](file:///g:/code-memory/.gitignore#L439-L441)）；`git ls-files --others --exclude-standard` 均列出 |
| `python scripts/dev-proxy.py 1420 --dev` | 端口 1420 **LISTENING** |
| `GET /` | **200 / 130170 字节**（= 磁盘 `index.html` 长度，含 `value-hero` 锚点）→ 证明服务的是**磁盘最新前端** |
| 缓存头 | `Cache-Control: no-store, no-cache, must-revalidate`（WebView2 启发式缓存问题的既有设计生效） |
| dev 模式端口改写 | `meta lrc-sidecar-port` → **3111**（与稳定版 3099 隔离） |
| API 代理路径 | `GET /v1/memories/stats` → **502**（预期：3111 无 sidecar，本机已安装实例在 3099）→ 代理链路可达 |
| 语法门禁 | `node --check association-desktop-cdp.js` → **EXIT=0** |
| 契约门禁 | `node scripts/validate_frontend_contract.js` → **EXIT=0** |

**新认知：交付性缺陷的通用模式**——r10（HCSE 文档）与 r13/r14（开发链路脚本）**同源**，均系 `local-dev` 在 v0.8.45/v0.9.0/v0.9.1 三次"发布清理"中以"不交付用户，本地开发自用"批量排除。该判断对**纯本地辅助文件**成立，但对**被已跟踪文件引用的文件**不成立。已回写强制识别方法「**引用反向扫描**」至 [HCSE_RELEASE_PROTOCOL.md](file:///g:/code-memory/docs/HCSE_RELEASE_PROTOCOL.md) 检查项 3.4。

**边界与遗留**：Cargo 打包走 `Cargo.toml` 的 `include`/`exclude`，本例外不使三脚本进入 crate / Tauri 发布产物。三脚本**仍为未跟踪状态**，须 `git add` 后方可随克隆交付。

### 8.11 第五轮修复轮记录（r15~r16）

> 起因：推进报告第五节第二梯队中**未被裁定"仅登记"**的第 5~9 项。本轮遵循「**先证伪再动手**」——报告曾在 P0-1 出现虚构风险，故每项均先取证。

| 轮次 | 目标 | 关联风险 | 落地位置 | 结果 |
|---|---|---|---|---|
| **r15** | 取消标志内存序统一 | P1 并发「取消标志内存序不一致」 | [memory_store.rs:3225](file:///g:/code-memory/src/memory_store.rs#L3225) | `Relaxed` → `Acquire`，与同标志其余 4 处检查点及写入端 `Release` 配对 |
| **r15** | 生产路径 `expect` 收敛 | P1 质量 / 第五节第 5 项 | [v1_api.rs:4346-4386](file:///g:/code-memory/src/v1_api.rs#L4346-L4386)、[json.rs](file:///g:/code-memory/src/persistence/json.rs#L224-L233)（4 处） | SSRF 校验后 3 处 + 缓存初始化 4 处改为显式错误/`ok_or_else` 返回 |
| **r16** | 核实「缺请求体大小限制」 | P1 安全第 2 项 | 报告定性更正 | **实为误报**，改为「已更正定性」 |

**r15 内存序缺陷（六钥匙·分解确证）**：

同一 `cancel: &AtomicBool` 语义在 [memory_store.rs](file:///g:/code-memory/src/memory_store.rs) 有 5 处读检查点，其中 4 处用 `Acquire`（[:3047](file:///g:/code-memory/src/memory_store.rs#L3047)、[:3510](file:///g:/code-memory/src/memory_store.rs#L3510)、[:3631](file:///g:/code-memory/src/memory_store.rs#L3631)、[:3917](file:///g:/code-memory/src/memory_store.rs#L3917)），**唯 [:3220](file:///g:/code-memory/src/memory_store.rs#L3220)（修复前）用 `Relaxed`**；而写入端为 `Release`（[v1_api.rs:1812](file:///g:/code-memory/src/v1_api.rs#L1812)、[:1953](file:///g:/code-memory/src/v1_api.rs#L1953)）。

`Relaxed` 不与 `Release` 构成同步配对，在弱内存序平台（ARM）理论存在读到过期值的风险，导致取消响应延迟或失效——**这是 x86 上难以复现、但语义确实不完备的缺陷**，属报告原始判定成立项。

**r16 误报更正（双重取证）**：

| 证据来源 | 内容 |
|---|---|
| axum 官方文档 [src/docs/extract.md:600-602](file:///C:/Users/Administrator/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/axum-0.8.9/src/docs/extract.md#L598-L604) | "For security reasons, `Bytes` will, by default, **not accept bodies larger than 2MB**. This also applies to extractors that uses `Bytes` internally such as `String`, `Json`, and `Form`." |
| axum-core 权威代码 [ext_traits/request.rs:316-328](file:///C:/Users/Administrator/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/axum-core-0.5.6/src/ext_traits/request.rs#L316-L328) | `const DEFAULT_LIMIT: usize = 2_097_152; // 2 mb`；`match ... DefaultBodyLimitKind { Disable => self, Limit(l) => ..., **None => self.map(\|b\| Body::new(Limited::new(b, DEFAULT_LIMIT)))** }` |
| 项目侧核对 | 全仓 `src/` **无任何** `DefaultBodyLimit` / `RequestBodyLimit` 覆盖（Grep 零命中）→ `None` 分支生效，默认保护**未被绕过** |

**结论**：报告原判定「缺请求体大小限制」**不成立**——默认 2MB 限制由框架提供且项目未关闭。此类"框架已提供保护、但被误读为缺失"的判定，与 P0-1（ReparsePoint 误读）同属**方法论缺陷**：**断言框架级行为前，须核对框架源码/官方文档，而非仅检查项目代码是否显式配置**。

**门禁验证（PowerShell 专家技能，证据落盘 `temp/`）**：

| 验证项 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | **FMT_CHECK_EXIT=0**（首轮为 1，`cargo fmt --all` 收敛后复检通过；4 处 Diff 均在 `json.rs`） |
| `cargo clippy --offline --all-targets --features server,ml -- -D warnings` | **CLIPPY_EXIT=0**（首次即通过，改动未引入新告警） |
| `cargo test --offline --lib` | **TEST_EXIT=0**；`625 passed; 0 failed` |
| 生产路径 `expect` 审计 | `json.rs` 测试模块前 **`real_expect=0`**（残留 4 处均为注释文本）；`v1_api.rs` SSRF 区域 **`real_expect=0`**（残留 1 处为注释） |
| 内存序审计 | `cancel_load_Acquire=5`、`cancel_load_Relaxed=0` |
| Rust 侧改动范围 | 仅 `src/memory_store.rs`、`src/persistence/json.rs`、`src/v1_api.rs` 三文件 |

**未闭环（已于第六轮全部处置）**：`process_guard.rs:628-629` 的 2 处 `expect`（**b3 已收敛为告警 + `pending()`**）、认证默认策略（**b8e 已落实为启动告警**）、`rate_limiter.rs` panic 风险（**b2 已消除**）、嵌套锁 `JSON_WRITE_LOCK`（**b8c 已文档化锁序契约**）。

### 8.12 第六轮修复轮记录（b1~b8，2026-09-13）

> 起因：用户将范围升级为「**直到文档中待修复的全部修复完成**」，覆盖此前经裁定"仅登记、不启动"的**全部结构性项**（P0-2 依赖倒置、P1-6 错误处理统一、D-9 无障碍、`competition/` 去留、CI/Release 脚本清单）。
> 本轮遵循同一纪律：**先证伪再动手**，且对每项「移除/删除」类操作都以**编译器或运行时实测**作为判据（而非静态计数）。

#### 8.12.1 本轮闭环总览

| 批次 | 目标 | 关联风险 | 结果 |
|---|---|---|---|
| **b1** | 模型 ID 单一真源 | P3-2 模型 ID 常量重复 | 新建 Layer 1 中立模块 [model_ids.rs](file:///g:/code-memory/src/model_ids.rs)；消除 4 处重复 + **1 处值不一致**（白名单 `intfloat/multilingual-e5-small` vs `model list` 显示 `multilingual-e5-small`） |
| **b1** | 原子写入加固 | P3-3 原子写入重复 | 固定临时名 → UUID 唯一名 + 失败清理 |
| **b2** | 取消标志内存序、Token 常量时间比较已闭环（r15）后的**剩余并发/安全项** | 并发/安全 | 见 8.12.2 |
| **b4** | 无障碍 D-9 | 前端 D-9 | HTML 侧 `aria-label` 12→**37**、`role="button"` 0→**21**、`tabindex="0"` 0→**21**；JS 侧 `aria-label` 0→**7** |
| **b5** | CI/Release `node --check` 清单统一 | 前端 P2-4 | 两清单统一为 **6 个脚本**（原 CI 3 / Release 4，均遗漏 `association-user-view-check.js`） |
| **b6** | `competition/` 去留 | P0-1 误判根源 | [competition/README.md](file:///g:/code-memory/competition/README.md) 补 Junction 说明 + PS5.1 合规核查代码块 |
| **b7** | **P0-2 依赖倒置** | P0-2 | `memory_state_machine.rs` 上提 Layer 1（见 8.12.3） |
| **b8a** | **P1-6 错误处理统一** | P1-6 | 6 个 error enum 全部补齐 `Display` / `std::error::Error` / `source()`（见 8.12.4） |
| **b8b** | **God Object 拆分（P2-2/P2-4）** | P2-2 / P2-4 | 测试岛外提 + 原子写入收敛（见 8.12.5） |
| **b8c** | 锁序文档化 | 并发 P1-5 / P2-3 | `JSON_WRITE_LOCK` 与 `BACKUP_OPERATION_LOCK` 锁序契约落盘（见 8.12.6） |
| **b8d** | `#[allow(dead_code)]` / `//!` 文档 / use 截断 / 行尾注释 | P2-1 / P1-4 / P2 质量 | 见 8.12.7 |
| **b8e** | 认证默认策略落地 | P1 安全 | 非回环绑定且未设 Token 时**启动告警**（见 8.12.8） |

#### 8.12.2 b2/b3 并发与安全项（承 r15 未闭环部分）

| 项 | 处置 |
|---|---|
| `rate_limiter.rs` 必然 panic | 消除：4 次重试全失败时 `last_err` 恒为 `None` → `match last_err { Some(e) => Err(e), None => op(MAX_ATTEMPTS) }` |
| `crypto.rs` 密钥路径回退 CWD | 消除：`APPDATA` 缺失/为空时退到 `dirs_next` 配置目录（**不用 CWD**），并打印告警 |
| `backup.rs` / `v1_api.rs` 备份目录两套推导 | **真实缺陷**：写入 `~/.loong-recall/backups` 而读取 `~/.loong-recall/global/backups` → "最后备份时间"永远读不到；统一为 `backup::backups_dir()` 单一真源 |
| `tray.rs` 裸指针来源校验 | `GWLP_USERDATA` 存入结构加 **magic tag**（原仅 `ptr != 0` 检查 → UB 风险）；清理时先置 0 消除 use-after-free 窗口 |
| `config.rs` `atomic_write` | 固定临时名 → UUID 唯一名 + 失败清理 |
| `process_guard.rs` 非 Windows 分支 `expect` | 收敛：注册失败改为告警 + `pending()` 挂起（与 Windows 分支一致） |

#### 8.12.3 b7：P0-2 依赖倒置（**关键发现：许可层错位，非架构错**）

**根因（六钥匙·分解）**：`src/engine/memory_state_machine.rs` 文件头声明 **Apache 2.0**，
且**零 crate 内部依赖**（仅 `serde` / `std`）——却位于 **DaoTi 研究许可**的 `engine/` 下。
Layer 1 的 `persistence/` 需要其中 `MemoryState` 作为持久化载体，于是形成
**Layer 1 → Layer 2 的反向依赖**。这不是"抽象放错了层"，而是**许可证归属放错了目录**。

**修复**：上提至 Layer 1 顶层 `src/memory_state_machine.rs`。

| 验证项 | 实测 |
|---|---|
| 移动前后 SHA256 | `0B4EB08299CB65B7C0EC4AEC4606E0FD8ACA4930C2F04A488834BA4BAEF3D328`（**逐字节一致**，证明未改内容） |
| 兼容再导出 | `engine/mod.rs` 改 `pub use crate::memory_state_machine;`，既有 `crate::engine::memory_state_machine::*` 路径（36 处引用）继续可用 |
| 依赖方向 | `persistence/mod.rs` + `json.rs` 改指向 `crate::memory_state_machine`；残留 2 处 `engine::memory_state_machine` 均为**注释文本** |
| 编译 | `CHECK_LIB_EXIT=0`（**feature-neutral 关键验证**）、`CHECK_SERVER_EXIT=0` |

> **方法论**：该修复的正确性判据不是"架构图是否好看"，而是**许可证边界是否自洽**。
> 若只按"抽象层级"思考，容易误判为需要大规模重构；核对文件头许可声明后，
> 修复收敛为"移动 + 再导出"。

#### 8.12.4 b8a：P1-6 错误处理统一（**范围校正**）

**报告原判定**：「6 套自定义 error enum 并存、40+ 返回 `Result<_, String>`、无 `anyhow`/`thiserror`」。
**本轮实测校正**：

| 事实 | 实测值 |
|---|---|
| error enum 数量 | **6**（`GuardError` / `SearchError` / `ApiError` / `PersistenceError` / `EmbedError` / `DownloadError`） |
| `Result<_, String>` 出现次数 | **74 处**（报告写"40+"，实际更多）——但绝大多数是 **trait 签名**（如 `CodeEncoder::encode`）与 **CLI/边界函数**，改动需全链路签名改造 |
| 已实现 `Display` 的 | 4/6（`SearchError`、`ApiError` 缺失） |
| 已实现 `std::error::Error` 的 | 3/6（`SearchError`、`ApiError` 缺失；`PersistenceError`/`DownloadError` 虽实现了但 `source()` 恒为 `None`） |

**本轮修复（六钥匙·泛化：不追 74 处签名，先补齐 trait 契约）**：

| 类型 | 补齐内容 |
|---|---|
| `SearchError` | `Display`（含超时秒数）+ `std::error::Error` |
| `ApiError` | `Display`（含 HTTP 状态码）+ `std::error::Error` |
| `PersistenceError` | `Error::source()`（此前恒 `None`，包裹的 `io::Error`/`serde_json::Error` **无法沿错误链回溯**） |
| `DownloadError` | `Error::source()`（同上，包裹的 `io::Error` 可回溯） |
| `IntegrityError`（桌面端） | `std::error::Error`（此前仅 `Display`，与同仓 `SidecarStartError` 口径不一致） |

**实测复检**：6/6 类型 `Display=True` / `Error=True`。

> **边界（已由第八轮闭环）**：b8a 当时**未**改造 74 处 `Result<_, String>` 签名。
> **第八轮（c4/c6）已完成**：新建 [`errors.rs`](file:///g:/code-memory/src/errors.rs)
> （`ErrorKind` 10 域 + `LrcError`），主 crate `Result<_, String>` **103 → 0**；
> `url_safety.rs` 因素 `include!` 约束手写 `UrlSafetyError`；桌面端 67 处经取证
> **全在 Tauri IPC 边界**（改类型会破坏 `catch (e)` 契约），零改动为正确结果。
> 另：**"需引入 `thiserror`"经实测更正——手写 `Display`/`Error` 即满足全部需求且不新增依赖**。详见 8.14.4。
>
> 本节（8.12.4）保留 b8a 轮次的历史记录，其"未改造"为**当时时点**的事实。

#### 8.12.5 b8b：God Object 拆分（P2-2 / P2-4）

**拆分策略（六钥匙·分解）**：God Object 的**最安全切片**是"测试岛"——
它与生产代码无交叉引用，外提后只改导入路径即可，且**测试用例数不变**可作为等价性证明。

| 文件 | 拆分前 | 生产部分 | 测试部分 |
|---|---|---|---|
| [v1_api.rs](file:///g:/code-memory/src/v1_api.rs) | 9012 行 | **4529 行** | 4484 行 → [v1_api_tests.rs](file:///g:/code-memory/src/v1_api_tests.rs) |
| [memory_store.rs](file:///g:/code-memory/src/memory_store.rs) | 6981 行 | **4612 行** | 2366 行 → [memory_store_tests.rs](file:///g:/code-memory/src/memory_store_tests.rs) |

- 引入方式：`#[cfg(test)] #[path = "..._tests.rs"] mod ..._tests;`
- 原 `use super::*;` 改写为 `use crate::v1_api::*;` / `use crate::memory_store::*;`，
  使条目解析**不随嵌套层级变化**（语义与改动前逐字等价）。
- **等价性证明**：拆分后 `cargo test --lib` 仍为 **625 passed / 0 failed**（用例数不变即等价）。

**P2-4「测试占比 50%」指标**：`v1_api.rs` 由 50% 降至 **0%**（测试全部外提）。

**顺带闭环 P3-3「原子写入重复 4 处」**：新建 Layer 1 公共设施
[atomic_file.rs](file:///g:/code-memory/src/atomic_file.rs)（`write_atomic`：UUID 唯一临时名 + 失败清理），
将 `config.rs` / `arch_config.rs` / `data_dir.rs` / `engine/audit_trail.rs` 四处各自手写的实现
**收敛为单一真源**。其中后三处修复前仍是**固定临时名**（与我 b1 在 `config.rs` 修的缺陷同源）。

> **后续闭环（第八轮 c3/c7）**：`MemoryStore` 的 **27 字段 / 18 pub / 主体 impl 约 3616 行**
> 这一"类本身是 God Object"的问题**已完成字段级拆分**——
> c3 外提 8 个纯数据契约类型至 [`memory_store_types.rs`](file:///g:/code-memory/src/memory_store_types.rs)，
> c7 外提 6 个缓存字段及全部纯逻辑至 [`memory_store_cache.rs`](file:///g:/code-memory/src/memory_store_cache.rs)，
> `MemoryStore` 侧仅留单一 `cache` 字段。**"需先完成 `RefCell → Sync` 并发模型改造"经实测为伪前置**
> （实测全仓 8 处共享方式均为 `Arc<Mutex<MemoryStore<P>>>`，而 `Mutex<T>: Sync` 只要求 `T: Send`）。
> 等价性由 **629 tests 不变**背书。详见 8.14.3 / 8.14.5。

#### 8.12.6 b8c：锁序契约文档化

| 位置 | 文档化内容 |
|---|---|
| [json.rs](file:///g:/code-memory/src/persistence/json.rs#L24-L57) `JSON_WRITE_LOCK` | 三级锁序（`cache` → `JSON_WRITE_LOCK` → 进程文件锁）为何是此顺序、`JSON_WRITE_LOCK` 为何必须是**全局单例而非 per-instance**（多实例写同一 `memories.json`）、持锁磁盘 IO 的**有意接受**理由、以及"禁止在持 ② 时获取 ①"的改动须知 |
| [backup.rs](file:///g:/code-memory/src/backup.rs#L25-L51) `BACKUP_OPERATION_LOCK` | 与持久层的**无环路**证明（备份从不持持久层锁；持久层从不调备份）、未来若加"写前自动备份"须维持的顺序 |

#### 8.12.7 b8d：`#[allow(dead_code)]` 移除实验 + `//!` 模块文档

**（1）`#[allow(dead_code)]` —— 报告判定经实测校正**

报告称 19 处 `#[allow(dead_code)]` 中多数"已过时（对应代码已在使用）"。
本轮采用**移除实验**（删掉全部 allow 后由编译器判定）作为权威判据：

| 结论 | 数量 | 说明 |
|---|---|---|
| **仍必要**（移除后 `cargo check --all-targets` 报 `never used/read`） | **16** | 报告"已过时"判定**不成立**，全部回填，并逐处注明"移除实验证明非冗余" |
| **确属冗余**（移除后无告警） | **3** | 删除 |

**实测复检**：`ALLTARGETS_SERVER_ML_EXIT=0` / `ALLTARGETS_QDRANT_EXIT=0`（**零 dead_code 告警**）。

> **方法论**：判断 `#[allow(dead_code)]` 是否过时，**不能靠人工阅读**（"看起来在用"与
> "编译器认为在用"是两回事），必须以"移除 + `--all-targets` 编译"为准。

**（2）`//!` 模块级文档：0 → 25 个文件**

- 覆盖 `src/` 下 26 个顶层模块中的 **25 个**（原为 0）。
- **唯一例外**：[url_safety.rs](file:///g:/code-memory/src/url_safety.rs#L22-L30) 刻意保留 `//`——
  桌面端 `desktop/src-tauri/src/url_safety.rs` 通过 `include!()` 引入该文件，
  而 `include!` 展开处**不允许内部文档注释**（实测报 **E0753** `expected outer doc comment`）。
  该约束已写入文件头，避免后续误改。
- 修复转换引入的 rustdoc 告警：`crypto.rs` 的方括号（`[12B]` 等被当链接）改为行内代码；
  `server.rs` 裸 URL 改为 `<...>`。`memory_store.rs` 的 ASCII 图触发
  `clippy::doc_lazy_continuation`（`-D warnings` 下为 error），已插空行分隔。

**（3）P2-1「use 语句被函数截断」**：`memory_store.rs` 的 `emit_remember_profile`、
`v1_api.rs` 的 `type EnrichBlockingResult` + `struct CancellationFlag` 均被插在 `use` 块中间，
已统一移至全部 `use` 声明之后。

**（4）P1-4「行尾注释错位」**：`engine/mod.rs:22`（三条注释挤一行）、`:68`（"道枢演化"误挂
`user_feedback` 行尾）已按各自归属拆分。

**（5）P3-1「`engine/archive/` 空目录」——定性校正**：该目录是
[CHANGELOG.md](file:///g:/code-memory/CHANGELOG.md) 明文约定的**归档位**
（"核心文件重大变更时旧版本必须先移入此处"），仅含 `.gitkeep` 占位以让 Git 保留空目录。
**它不是残留垃圾，而是流程占位**。已在 [engine/mod.rs](file:///g:/code-memory/src/engine/mod.rs#L70-L76)
就地说明。该判定与 P0-1（未识别 Junction 而误判残留）同属
「**未核对项目既有约定即判定为垃圾**」的方法论问题。

#### 8.12.8 b8e：认证默认策略（从"文档约定"升级为"启动即告警"）

**报告原判定**：未设 `LRC_API_TOKEN` 时默认无认证 —— 「待跟踪」。
**本轮处置**：新增 [warn_if_unauthenticated_non_loopback](file:///g:/code-memory/src/server.rs#L3785-L3815)，
在 [serve_on_listener](file:///g:/code-memory/src/server.rs#L3939-L3944) 启动时判定：

| 绑定地址 | 未设 Token 时行为 |
|---|---|
| `127.0.0.1` / `::1` / `localhost` | 静默（回环下默认无认证是**有意设计**，威胁模型见 `local_api_auth` 文档） |
| 其余（`0.0.0.0` / `::` / 具体 IP / 主机名） | **打印 5 行醒目告警**：说明风险 + 给出两种处置（改绑回环 / 设 Token） |

判定刻意**保守**（宁可误报不可漏报）；仅告警、不阻断启动，以免破坏既有内网自用部署。

#### 8.12.9 第六轮门禁验证（PowerShell 专家技能，证据落盘 `temp/`）

| 验证项 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | **`FMT_CHECK_EXIT=0`** |
| `cargo clippy --offline --all-targets --features server,ml -- -D warnings` | **`CLIPPY_EXIT=0`** |
| `cargo clippy --offline --all-targets --features qdrant -- -D warnings` | **`CLIPPY_QDRANT_EXIT=0`** |
| `cargo check --offline --no-default-features` | **`CHECK_NO_DEFAULT_EXIT=0`**（feature 矩阵下仍可编译） |
| `cargo test --offline --lib` | **`TEST_EXIT=0`**；`625 passed; 0 failed`（**与拆分前一致，等价性证明**） |
| 桌面端 `cargo test` | **`DESKTOP_TEST_EXIT=0`**；`90 passed; 0 failed` |
| `cargo doc --offline --no-deps` | `DOC_EXIT=0`；残留 12 条 rustdoc 告警**均为转换前既有的条目级文档缺陷**（本轮引入的 2 类已修复） |
| `node --check` × 6 脚本 | `NODECHECK_ALL_OK=True` |
| `node scripts/validate_frontend_contract.js` | **`CONTRACT_EXIT=0`**（36 API 路径 / 33 invoke 命令全对齐） |
| a11y 计数（实测） | `aria-label_html=37`、`role_button_html=21`、`tabindex0_html=21`、`aria_label_js=7` |
| `dead_code` 告警 | **0**（`ALLTARGETS_*_EXIT=0`） |
| 错误类型 trait 覆盖 | **6/6** `Display=True` / `Error=True` |

**本轮过程中的回归与修复**：
1. **编辑事故**：b8c 为 `JSON_WRITE_LOCK` / `BACKUP_OPERATION_LOCK` 加文档注释时，
   注释块被插在 `static` 声明**之后**，形成重复定义（`E0428`）→ 合并为单一声明。
2. **遗留未使用变量**：`v1_api.rs:3466` 的 `data_dir_clone` 是 b2 修复（改用 `backups_dir()`）后
   的孤儿绑定 → 删除。
3. **PS5.1 中文脚本解析失败（再次触发）**：测试岛外提脚本含中文注释，
   PS5.1 按 ANSI 代码页解读导致 `Missing ')'` 等解析错误 → **全部重写为纯 ASCII 脚本**
   （注释改英文，中文文本用码点数组构造）。
4. **`include!` + `//!` 冲突**：`src/url_safety.rs` 转 `//!` 后，桌面端
   `include!()` 展开处报 `E0753`（23 条）→ 该文件回退为 `//` 并就地注明约束。
5. **`clippy::doc_lazy_continuation`**：`memory_store.rs` 模块文档的 ASCII 图
   在 `//!` 化后被判为"列表项未缩进延续"（`-D warnings` 下为 error）→ 插空行分隔。
6. **`write_atomic` 签名不匹配**：`audit_trail.rs` 的 `path` 为 `&str` 而非 `&Path`（`E0308`）
   → 传参处包 `std::path::Path::new(path)`。

**本轮工作区变更统计**：`66 files changed, 2008 insertions(+), 1196 deletions(-)`（`git diff --shortstat HEAD`，不含 13 个未跟踪文件）。

**交付物提醒（高优先级）**：六轮累计 **13 个未跟踪文件**，其中 **5 个是 `src/*.rs` 源文件**
（`model_ids.rs`、`atomic_file.rs`、`v1_api_tests.rs`、`memory_store_tests.rs`、`memory_state_machine.rs`）——
**若未 `git add`，克隆环境将编译失败**。详见 8.7 与 [HCSE_RELEASE_PROTOCOL.md](file:///g:/code-memory/docs/HCSE_RELEASE_PROTOCOL.md) 检查项 4。

### 8.13 第七轮修复轮记录（b10~b13，2026-09-13）

> 起因：继续推进 8.6 中剩余的未闭环项。本轮同样遵循「**先证伪再动手**」，
> 且每项"删除"动作都以**调用方反向扫描**为判据（而非"看起来没人用"）。

#### 8.13.1 本轮闭环总览

| 批次 | 目标 | 关联风险 | 结果 |
|---|---|---|---|
| **b11** | API 层职责混合（`server.rs` 混入桌面环境探测） | P2-3 | 删除 `server.rs` **168 行**注释化死代码（`detect_command_tool` / `which_path` / `check_windows_install_path` / `check_vscode_extension`） |
| **b11** | **SidecarManager 持锁跨 Phase** | 并发 P2-2 | 删除 `start()` / `start_for_project()` / `restart_project()` **3 个零调用方 API**，使该模式在**类型层面不可达** |
| **b12** | Tauri 命令归属不明 | 前端/文档 | 4 个孤儿命令在注册处**显式标注归属**；**新增契约门禁**使"注册未调用"从静默变为显式申报 |
| **b13** | feature 矩阵 / postgres 构建 | 架构 P1-3 | **经实测更正定性**（见 8.13.4） |

#### 8.13.2 b11：删除 `server.rs` 168 行注释化死代码（P2-3）

**取证**：`server.rs:3486-3489` 与 `:3491-3651` 是两段 `/* ... */` 块注释，包裹
`detect_command_tool` / `which_path`（PATH 探测）、`check_windows_install_path`（安装目录探测）、
`check_vscode_extension`（扩展探测）。全仓反查确认：**除注释块自身外零引用**。

**定性**：报告原判「API 层职责混合（`server.rs` 混入桌面环境探测）」成立——
这些函数确属**桌面环境探测**，且已被"浏览器端只返回快捷方式候选、正式检测交给桌面端 `AgentDetector`"
的设计取代。但它们**以注释形式留存**，既污染文件又无法被编译器/静态检查发现（注释内容不受
`dead_code` 检查覆盖）。

**修复**：删除该 168 行。`tools_detect_handler` 保留（它是**活代码**，只做快捷方式扫描）。

| 验证项 | 实测 |
|---|---|
| 边界校验 | 起始行匹配 `// ---------- 工具检测辅助函数 ----------`、结束行为独立 `*/`（脚本前置断言，不匹配则 abort） |
| 行数变化 | 4954 → **4786**（移除 168 行） |
| 全仓引用 | Grep `detect_command_tool\|which_path\|check_windows_install_path` = **0 处非注释引用** |
| 回归 | `FMT` / `CLIPPY` / `TEST 625 passed` / `DESKTOP 90 passed` 全绿 |

#### 8.13.3 b11：消除 SidecarManager「持锁跨 Phase」（并发 P2-2）

**取证（反向调用扫描）**：`sidecar_manager.rs` 中 `start()` / `start_for_project()` / `restart_project()`
均接收 `&mut self`（调用方必须已持有 sidecar 锁），内部顺序执行
Phase 1→2→3——**Phase 2 的健康检查（最多 40s）在持锁状态下执行**。

| 方法 | 调用方 |
|---|---|
| `start()`（:1472） | **无** |
| `start_for_project()`（:1508） | 仅被 `start()` 与 `restart_project()` 调用 |
| `restart_project()`（:1716） | **无** |

而生产入口（`commands.rs` 的 `start_sidecar` / `start_sidecar_for_project` / `switch_project`）
**已全部改用三阶段编排**（`prepare_start` 持锁 → `SidecarManager::spawn_and_wait` 不持锁 →
`insert_handle` 重新持锁）——这正是 v0.5.17 引入三阶段模式时为解决的问题，
但**旧 API 被保留下来**，构成"文档说了要三阶段、但类型允许持锁跨 Phase"的 footgun。

**修复（六钥匙·分解）**：删除这 3 个方法。删除后**唯一可用的编排方式**就是三阶段
——因为 `prepare_start` / `spawn_and_wait` / `insert_handle` 自身都不持有 `&mut self` 跨 I/O，
故"持锁跨 Phase 2"在**类型层面不可达**，不再依赖开发者自觉。

| 验证项 | 实测 |
|---|---|
| 删除前调用方 | 3 个方法互相调用，**零外部调用方** |
| 回归 | `FMT` / `CLIPPY` / `DESKTOP 90 passed` 全绿（无 `&mut self` 相关编译错误） |

> **方法论**：消除"易误用的 API"比"在文档里警告不要误用"更可靠——前者是**类型约束**，
> 后者依赖**人的注意力**。

#### 8.13.4 b12/b13：Tauri 命令归属 + feature 矩阵定性校正

**（1）Tauri 命令归属（报告"7 个命令归属不明" → 实测更正为 4 个孤儿 + 归属已明）**

用与 `validate_frontend_contract.js` **完全相同的正则**重新取证：

| 事实 | 实测值 |
|---|---|
| 前端 `static/app.js` invoke 目标（去重） | **33**（与契约脚本输出一致） |
| 后端 `main.rs` `generate_handler!` 注册 | **37** |
| 差集（注册但前端零调用） | **4**：`open_dashboard_window`、`navigate_main_to_dashboard`、`update_tray_tooltip`、`bulk_apply_agent_overrides` |

**归属取证**：这 4 个**均有明确归属，非死代码**——
- `open_dashboard_window` / `navigate_main_to_dashboard`：桌面端内部改由
  `show_dashboard_in_main_window(&app, port, adjust)` 直接调用（统一入口，`commands.rs:171`）；
- `update_tray_tooltip`：桌面端内部已在 Agent 配置完成后**直接调** `tray::update_tooltip`（`commands.rs:1773`）；
- `bulk_apply_agent_overrides`：批量手动修正，含 **HCSE FM-09** 的 10 秒超时 + 指数退避限流。

**处置（六钥匙·重述）**：目标是"使归属明确"而非"删除接口"——它们是对外 **IPC 契约**，
删除会缩小接口面（属产品决策）。故：
1. 在 `main.rs` 注册处**就地标注**归属与保留理由；
2. **新增契约门禁**（`validate_frontend_contract.js` 检查 4）：把"后端注册但前端零调用"
   从**静默**变为**显式申报**——新增孤儿命令若未在白名单说明理由，**CI 失败**；
   同时白名单若含"已不再是孤儿"的命令也**失败**（防止理由腐化）。

**对照实验（确证门禁非死代码）**：

| 步骤 | 实测 |
|---|---|
| 临时移除 `open_dashboard_window` 的白名单条目 | `MUTATED_EXIT=1`，报**两条**错误：①该命令未申报；②白名单含已非孤儿的 `__removed_for_test__` |
| 恢复后复跑 | `RESTORED_EXIT=0`，输出 `…、4 个孤儿命令已申报` |
| 恢复逐字节一致 | `RESTORE_BYTE_EXACT=True` |

**（2）feature 矩阵（报告"失效" → 实测更正）**

| feature | 报告判定 | 实测 |
|---|---|---|
| `qdrant=[]` | "空 feature = 失效" | **定性更正**：`qdrant.rs` 走 **HTTP API**（`reqwest`），**不引入额外依赖**，故 feature 为空是**正确设计**。`cargo check --features qdrant` → **EXIT=0** |
| `neo4j=[]` | 同上 | 同上，`cargo check --features neo4j` → **EXIT=0** |
| `tokio` 非 optional | "应改 optional" | **定性更正**：`tokio` 被 **3 个非 server 门控模块**使用（`consolidation.rs` 11 处、`process_guard.rs` 8 处、`url_safety.rs` 4 处），**不能改为 optional** |
| `reqwest` 非 optional | 同上 | 实测其引用集中在 `server` 门控模块内（`server.rs`/`v1_api.rs`/`persistence/{qdrant,neo4j}.rs`/`engine/llm_translator.rs`），**改造成本高于收益** |
| `postgres` | 未提及 | **实测失败**（EXIT=101）——但原因是**离线环境缺少 `atoi v2.0.0` 缓存**（`error: failed to download atoi v2.0.0 … --offline was specified`），**非代码缺陷**；该包已声明于 `Cargo.lock:102` |

**feature 组合实测矩阵**：

| 组合 | EXIT |
|---|---|
| `server` | 0 |
| `server,ml` | 0 |
| `qdrant` | 0 |
| `neo4j` | 0 |
| `server,qdrant,neo4j` | 0 |
| `postgres` | **101**（离线依赖缺失，非代码问题） |
| `--no-default-features` | 0 |

#### 8.13.5 第七轮门禁验证（PowerShell 专家技能，证据落盘 `temp/`）

| 验证项 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | **`FMT_CHECK_EXIT=0`** |
| `cargo clippy --all-targets --features server,ml -- -D warnings` | **`CLIPPY_SERVER_ML_EXIT=0`** |
| `cargo clippy --all-targets --features qdrant -- -D warnings` | **`CLIPPY_QDRANT_EXIT=0`** |
| `cargo check --no-default-features` | **`CHECK_NO_DEFAULT_EXIT=0`** |
| `cargo test --lib` | **`TEST_EXIT=0`**；`625 passed; 0 failed` |
| 桌面端 `cargo test` | **`DESKTOP_TEST_EXIT=0`**；`90 passed; 0 failed` |
| `node --check` × 6 脚本 | `NODECHECK_ALL_OK=True` |
| `node scripts/validate_frontend_contract.js` | **`CONTRACT_EXIT=0`**（36 API / 33 invoke / **4 孤儿已申报**） |
| 契约门禁对照实验 | `MUTATED_EXIT=1` → `RESTORED_EXIT=0` → `RESTORE_BYTE_EXACT=True` |
| feature 组合矩阵 | 6/7 通过（`postgres` 因离线依赖缺失） |
| 未跟踪交付物 | `UNTRACKED_COUNT=13`（与第六轮一致，本轮未新增交付物） |

**第七轮暴露的方法论**：

| 编号 | 方法论 | 来源 |
|---|---|---|
| M7 | **注释掉的代码不受任何静态检查覆盖**——`dead_code` 只检查**活代码**，因此"注释化废弃"是比"直接删除"更危险的状态（既不生效也不报警）。废弃代码应删除，版本历史由 git 承载 | b11（`server.rs` 168 行） |
| M8 | **消除易误用 API 优于在文档中警告**——删除 `&mut self` 跨 I/O 的方法后，"持锁跨 Phase"由**类型系统**保证不可达 | b11（SidecarManager） |
| M9 | **"注册但未调用"必须显式申报**——门禁应强制新增孤儿命令说明理由，并对白名单做**双向校验**（未申报则失败 / 已非孤儿仍留则失败） | b12（契约检查 4） |
| M10 | **"空 feature"未必是失效设计**——若该后端通过既有依赖（HTTP）实现，则 feature 为空**正确**。判定前须查实现方式 | b13（qdrant/neo4j） |
| M11 | **离线环境构建失败须先区分"依赖缺失"与"代码缺陷"**——`error: failed to download` 属前者，不应记为风险 | b13（postgres） |

### 8.14 第八轮修复轮记录（c2~c8，2026-09-13）

> 起因：用户将范围明确为「**直到文档中待修复的全部修复完成**」，覆盖 8.6 表中
> **剩余全部 4 项**（`spawn_blocking` 不可强杀、P1-6 签名改造、`MemoryStore`
> 字段级拆分、后端选型）。本轮继续遵循「**先证伪再动手**」——其中 **2 项的原判定
> 被实测推翻**（见 8.14.4 与 8.14.5 的"伪前置/伪缺陷"更正）。

#### 8.14.1 本轮闭环总览

| 批次 | 目标 | 关联风险 | 结果 |
|---|---|---|---|
| **c2** | `timeout` **不中断** `spawn_blocking` | P1-3 | 实现**协作式取消（补偿路径）**：`SynthesisEngine` 新增 3 个可取消变体，超时分支置位 `AtomicBool`，计算方周期性自查后提前返回；**取消路径不写回**（返回 `TimedOut` 保留重试标记） |
| **c3** | `MemoryStore` God Object（**数据契约切片**） | P2-2 | 新建 `src/memory_store_types.rs`，外提 8 个纯数据契约类型（零状态依赖），`pub use` 重导出保证路径零改动 |
| **c4** | 错误处理统一（**安全关键子集**） | P1-6 | `src/url_safety.rs`（SSRF 核心）**手写** `UrlSafetyError`（13 变体）+ `Display`/`Error`，**不引入依赖**（受 `include!` 约束）；文案逐字保留 |
| **c6** | 错误处理统一（**主体 103 处**） | P1-6 | 新建 `src/errors.rs`（`ErrorKind` 10 域 + `LrcError`）；主 crate `Result<_, String>` **103 → 0**（仅余 1 处文档注释）；桌面端 **67 处经取证全在 IPC 边界，零改动为正确结果** |
| **c7** | `MemoryStore` God Object（**缓存切片**） | P2-2 | 新建 `src/memory_store_cache.rs`，外提 6 个缓存字段及其全部纯逻辑；**"需先做 `RefCell → Sync` 改造"经实测为伪前置**（见 8.14.5） |
| **c8** | 后端选型 | P1-2 架构 | **定性维持**（产品决策）；本轮补取证：`qdrant.rs` 的 10 处 `Unsupported` 均为**语义不可保证**（如"无法保证全量替换原子性"），非实现缺失 |

#### 8.14.2 c2：`spawn_blocking` 超时的**补偿路径**（P1-3）

**问题重述（六钥匙·重述）**：报告要求"`spawn_blocking` 超时后应**可中断或补偿**"。
Rust 运行时的硬限制是：**已提交到阻塞线程池的任务无法被强杀**（`JoinHandle` 被 drop
也只是分离，线程仍跑完）。故"可中断"不可达，**"补偿"是唯一可行路径**。

**实现（六钥匙·类比）**：沿用 `memory_store.rs` 已有的成熟模式（`synthesis_pending`
的 `AtomicBool` + `Release` 写 /`Acquire` 读配对），在 `SynthesisEngine` 新增：

| 新增方法 | 检查点位置 | 取消后行为 |
|---|---|---|
| `cluster_from_all_cancellable` | O(n²) 双循环内**每 64 次比较**检查一次 | 返回 `(vec![], true)` |
| `plan_jaccard_cancellable` | 调用上式 + **每簇**再检查 | 返回 `(SynthesisPlan::default(), true)` |
| `plan_luoshu_cancellable` | **每个八卦分组**开头检查 | 返回 `(SynthesisPlan::default(), true)` |

原三个方法**降为 `None` 委托包装**（`self.xxx_cancellable(..., None).0`），
故 `find_synthesis_clusters` / `try_synthesize` / `luoshu_synthesize` 等既有调用方
**零改动**，且 `cancel=None` 时行为与被改前**逐字节一致**。

**检查粒度取值依据**：`MAX_CLUSTER_CANDIDATES = 500` → 最坏 C(500,2) ≈ **12.5 万次**
比较。若每轮都读原子标志，检查开销会侵蚀计算本身；`CANCEL_CHECK_INTERVAL = 64`
把"超时后仍跑满 12.5 万次"降为"**最多多跑 63 次**"（微秒级），同时原子读开销 < 0.1%。

**接线点（3 处）**：

| 文件 | 位置 | 处置 |
|---|---|---|
| `src/consolidation.rs` | `run_cycle`（`timeout(CYCLE_TIMEOUT=120s, ...)`） | 超时分支 `cancel.store(true, Release)`；`run_cycle_inner` 增 `cancel: Arc<AtomicBool>` 参数，传入 `spawn_blocking` 闭包 |
| `src/consolidation.rs` | Phase 3 写回 | **取消路径返回 `PersistenceError::Io(TimedOut)` 而非 `Ok(0)`**——因调用方的 `Ok` 分支会清除 `synthesis_pending`，而"被取消"意味着本轮未完成，**必须保留待重试标记** |
| `src/v1_api.rs` | `POST /v1/memories/consolidate` | 因外层 `TimeoutLayer(30s)` 只 drop future 不杀线程，改为**显式 60s `timeout` + 取消标志**；新增 `consolidation_cancelled` / `consolidation_timeout` 两个错误码（503） |

**语义保证**：超时/取消时**绝不写回不完整计划**（否则会把"只算了一半的聚类"落盘污染记忆库）。

#### 8.14.3 c3/c7：`MemoryStore` God Object 的**两刀切片**（P2-2）

原 27 字段 / 主体 impl 约 3616 行（c3 前实测 4363 行）。本轮以**零行为变更**为约束，
按"是否存在状态依赖"切成两刀：

**第一刀（c3）· 纯数据契约** → `src/memory_store_types.rs`：

| 外提类型 | 依赖 |
|---|---|
| `RecallFilter` / `ListFilter` / `SortBy` / `SortOrder` | 仅 `memory_types` |
| `MemoryStats` / `RecallResult` / `SynthesisSnapshot` | 仅 `memory_types` + `SynthesisConfig` |
| `RegulatorHeartbeat` | 仅 `RegulationAction` |

全部**零 `MemoryStore` 状态依赖**，故可独立演进（新增查询条件/统计字段不再触碰存储主体）。
`memory_store.rs` 以 `pub use crate::memory_store_types::{...}` 重导出，
**`crate::memory_store::Xxx` 既有路径与 `use crate::memory_store::*` 调用方零改动**。

**第二刀（c7）· 缓存子系统** → `src/memory_store_cache.rs`：

| 原 6 字段 | 归入 `MemoryStoreCache` |
|---|---|
| `memory_cache` / `cache_dirty` | `memory` / `dirty`（`store()` / `snapshot()` / `is_dirty()`） |
| `bigram_index` / `bigram_index_dirty` | `index` / `index_dirty`（`borrow_index()` / `add_to_index()` / `remove_from_index()` / `replace_in_index()` / `rebuild_index()`） |
| `recall_documents` | `documents`（`recall_document()`） |
| `assoc_frequency` | `assoc`（`borrow_assoc()` / `borrow_assoc_mut()` / `assoc_snapshot()`） |

`MemoryStore` 侧只留**单一 `cache` 字段**；`load_cached`/`invalidate_cache` 等方法降为
**一行委托**，保留原有编排与性能剖析埋点。纯函数辅助（`content_bigrams` /
`content_index_terms` / `domain_index_enabled` / `index_terms_for_memory`）随之迁入，
`MemoryStore` 侧保留同签名薄包装（避免触碰 20+ 调用点）。

| 验证项 | 实测 |
|---|---|
| 拆分前 `memory_store.rs` 行数 | 4363 |
| 拆分后 `memory_store.rs` 行数 | **约 4050**（外提至 `memory_store_types.rs` + `memory_store_cache.rs`） |
| 等价性证明 | **629 tests passed**（拆分前后完全一致，测试用例数未变） |
| 全量回归 | `FMT` / `CLIPPY` / `TEST 629` / `DESKTOP 90` / `NODE` / `CONTRACT` 全绿 |

#### 8.14.4 c6：错误处理统一 —— 主 crate `Result<_, String>` 归零（P1-6）

**设计取舍（六钥匙·简化 + 泛化）**：取证发现 103 处中绝大多数是**"带上下文的单一失败"**，
调用方不需要为每个函数发明特有变体。故**不**为每模块定义独立错误枚举
（那会制造上百个一次性类型），而是提供**统一类型 + 域分类标签**：

```rust
pub enum ErrorKind { Io, Parse, Crypto, Config, Network, InvalidInput,
                     NotFound, Timeout, Unsupported, Internal }
pub struct LrcError { pub kind: ErrorKind, pub message: String }
```

**保证用户可见文案零漂移的三条机制**：

| 机制 | 效果 |
|---|---|
| `impl Display` **只输出 `message`**（不带域前缀） | 所有 `format!("{}", e)` / `eprintln!("{e}")` / HTTP 响应体输出**逐字不变** |
| `impl From<LrcError> for String` | 处于旧 `Result<_, String>` 上下文的调用方**仍可用 `?` 自动上浮**，允许渐进迁移 |
| 改造时**逐字保留 `format!` 字面量** | 文案改动被明令禁止；仅把"包装器"从 `String` 换成 `LrcError::<域>(...)` |

**迁移覆盖**：

| 层 | 模块 | 处数 |
|---|---|---|
| Layer 1 基础设施 | `crypto.rs` / `config.rs` / `export.rs` / `backup.rs` / `migration.rs` / `tray.rs` / `errors.rs`(新) | 27 |
| engine（DaoTi 层） | `llm_translator.rs`(11) / `encoder_codebert.rs`(7) / `encoder.rs`(3) / `luoshu_encoder_ml.rs`(3) / `encoder_registry.rs`(2) / `dao_evolution.rs`(2) / `manager.rs`(2) / `user_feedback.rs`(2) / `mod.rs`(1) | 33 |
| 服务层 | `consolidation.rs`(4) / `server.rs`(2) / `bin/server.rs`(6) / `v1_api.rs`(1) | 13 |
| persistence（feature 门控） | `neo4j.rs`(2) / `postgres.rs`(1) / `qdrant.rs`(1) | 4 |
| 安全边界 | `url_safety.rs`（**c4 独立手写 enum**，见下） | 4 |
| **合计** | | **主 crate 归零** |

**`url_safety.rs` 的特殊处置**：该文件被桌面端 `include!()` 引入，
**不能引用主 crate 路径**（`crate::errors`）也**不能新增依赖**（`thiserror` 未在依赖表），
故 c4 为其**手写** `UrlSafetyError`（13 变体，按"字面量非法" vs "DNS 层失败"分组）
+ `Display` + `std::error::Error`。这是**全仓唯一**未复用 `errors.rs` 的类型化错误，
且同样保证文案逐字一致。

**桌面端 67 处为何零改动（取证结论）**：逐处溯源后**全部**落在两类：
1. **(A) `#[tauri::command]` 本体**（36 处）——Tauri `invoke` 契约要求错误可序列化为
   字符串，前端 `catch (e)` 按字符串读取，**改类型会破坏运行时契约**；
2. **(B) 其调用链汇入 IPC 边界的辅助函数**（31 处）——命令侧统一用
   `user_friendly_error(err: &str) -> String` 转换，该签名要求错误可借为 `&str`。

故"零改动 + 分类证据报告"即为**正确结果**，而非遗漏。

**报告中"需引入 `thiserror`"的更正**：本轮实测证明 **`thiserror` 并非必需**——
手写 `Display`/`Error`（约 80 行）即满足全部需求，且**不新增任何依赖**，
对 `include!` 场景（`url_safety.rs`）反而是唯一可行方案。

#### 8.14.5 c7 的**伪前置更正**：`RefCell → Sync` 并非拆分前提

**报告原文**："`MemoryStore` 字段级拆分需先完成 `RefCell → Sync` 并发模型改造"。

**实测证伪（六钥匙·重述）**：全仓搜索 `MemoryStore` 的共享方式，**全部 8 处**
均为 `Arc<Mutex<MemoryStore<P>>>`：

| 位置 | 形式 |
|---|---|
| `v1_api.rs:1116` `pub type SharedStore` | `Arc<Mutex<MemoryStore<JsonPersistence>>>` |
| `server.rs:346` / `:2653` | `Arc<Mutex<MemoryStore<JsonPersistence>>>` |
| `consolidation.rs:49/238/255/273/1146` | `Arc<Mutex<MemoryStore<P>>>` |

关键事实：**`Mutex<T>: Sync` 只要求 `T: Send`**（标准库 `impl<T: ?Sized + Send> Sync for Mutex<T>`），
**并不要求 `T: Sync`**。而 `RefCell<T>` / `Cell<T>` 在 `T: Send` 时即为 `Send`。
因此 `MemoryStore` 只需 `Send` 就足以放进 `Mutex` 跨线程共享，
**`Sync` 从来不是前置条件**——报告该判定属**伪前置**（把"`RefCell` 不是 `Sync`"
误推为"不能跨线程共享"）。

**这一更正的实践价值**：`RefCell`/`Cell` 正是让 `&self` 方法（如 `load_cached`）
能**惰性刷新缓存**的手段；外层 `Mutex` 已提供互斥，内层无需（也不应）改为
`RwLock`/`Mutex`（那会把每次缓存读变成一次加锁，纯属性能退化）。
故 c7 **保留** `RefCell`/`Cell` 设计，仅做**物理外提**。

#### 8.14.6 c8：后端选型的**定性维持**（P1-2 架构）

**取证**：`qdrant.rs` 的 10 处 `Unsupported` 逐一核对，**全部是"语义无法保证"而非"懒得实现"**：

| 位置 | 原文案 | 性质 |
|---|---|---|
| `:555-559` | `replace_all_memories`："Qdrant 后端**无法保证全量替换的原子语义**" | fail-closed 设计：宁可报错也不做非原子替换 |
| 其余 9 处 | 同类不可保证语义（如跨点事务、全量枚举一致性） | 同上 |

**结论**：这与 b13 的 M10（"空 feature 未必是失效设计"）同源——**判定"未实现"前，
须先分辨"能力缺失"与"语义不可保证下的 fail-closed"**。后者是**正确设计**。
将其登记为"**产品决策**"（是否引入更强后端以换取这些语义）而非技术缺陷，**定性维持**。

#### 8.14.7 第八轮门禁验证（PowerShell 专家技能，证据落盘 `temp/`）

| 验证项 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | **`FMT_CHECK_EXIT=0`** |
| `cargo clippy --offline --all-targets -- -D warnings` | **`CLIPPY_EXIT=0`** |
| `cargo test --offline --lib` | **`TEST_EXIT=0`**；**`629 passed; 0 failed`**（较第七轮 625 **+4**，为 `errors.rs` 新增单测） |
| 桌面端 `cargo test --offline` | **`DESKTOP_TEST_EXIT=0`**；**`90 passed; 0 failed`**（不变，印证桌面端零改动正确） |
| `node --check` × 6 脚本 | `NODECHECK_ALL_OK=True` |
| `node scripts/validate_frontend_contract.js` | **`CONTRACT_EXIT=0`**（36 API / 33 invoke / **4 孤儿已申报**） |
| 主 crate `Result<_, String>` 计数 | **103 → 0**（仅剩 1 处文档注释文字） |

> **测试数变化的说明**：625 → 629 的 **+4** 全部来自新建 `src/errors.rs` 的
> 单元测试（`display_outputs_message_only` / `into_string_keeps_message` /
> `kind_is_discriminable` / `propagates_from_string_context`），
> **无任何既有测试被删除或跳过**——c3/c7 的等价性由"既有用例数不变"背书。

#### 8.14.8 第八轮暴露的方法论

| 编号 | 方法论 | 来源 |
|---|---|---|
| M12 | **"前置条件"必须被验证，而非被采信**——报告称字段级拆分"需先做 `RefCell → Sync`"，实测 `Mutex<T>: Sync` 只要求 `T: Send`，故该前置**不存在**。伪前置会无限期阻塞本可完成的拆分 | c7（8.14.5） |
| M13 | **运行时硬限制下，"补偿"优于"中断"**——`spawn_blocking` 已提交任务不可强杀是 Rust 语义，与其追求不可达的"中断"，不如用协作式取消把"跑满 O(n²)"降为"最多多跑 63 次" | c2 |
| M14 | **统一错误类型优于逐模块枚举**——100+ 处错误多为"带上下文的单一失败"，为每模块造枚举会产出上百个一次性类型；"统一类型 + 域分类标签"以 1 个类型覆盖全部，且用 `Display` 只输出 message 保证文案零漂移 | c6 |
| M15 | **IPC 边界的错误类型不是技术债而是契约**——桌面端 67 处 `Result<_, String>` 全部位于 Tauri 命令或汇入其边界的辅助函数，"统一改造"会破坏前端 `catch (e)`。判定"是否该改"必须先溯源调用链终点 | c6 |
| M16 | **"未实现"与"语义不可保证下的 fail-closed"须区分**——Qdrant 的 `Unsupported` 是"宁可报错也不做非原子操作"的正确设计，不应记为缺陷 | c8（承 M10） |
| M17 | **手写 `Display`/`Error` 是 `include!` 场景下唯一可行方案**——该类文件不能引用 crate 内部路径，也不能引入 `thiserror`，故"类型化错误"与"零依赖"可兼得 | c4 |

---

**审查执行说明**：本次审查为**纯只读操作**，未修改 `g:\code-memory` 下任何文件。全部结论基于实际文件读取与 Grep 计数，未使用推测数据。审查共覆盖 6 个维度，产出 P0 级风险 7 项、P1 级约 20 项、P2 级约 18 项、P3 级约 20 项。

**修复轮执行说明（2026-09-12 起，共七轮）**：

- **第一轮（f1~f10）**：对 P0 级 5 项（P0-3/4/5/6/7）、P1 级 3 项、P2 级 8 项、文档类 7 项（D-1~D-8，含 D-6 定性更正）落盘修复，全量回归**三项全绿**（fmt / clippy / 625 tests）。
- **第二轮（r1~r10）**：闭环 D-10、D-11、P1-2、P1-7、P1-8 与 HCSE 四类检查项回写（r6/r7），并发现并修复一处**自指事故**——回写产出的两份 HCSE 文档本身被 `.gitignore` 刻意忽略（r10，经用户裁定解除忽略）。详见 8.8。
- **第三轮（r11~r12）**：修复**前端超时误分类**导致的 `SidecarTimeoutError` UI 分支死代码（r11），并内嵌超时/卡死路径 E2E 门禁（r12，含对照实验确证）。闭环后发现新断层类型「**可达断层**」。详见 8.9。
- **第四轮（r13~r14）**：闭环**本地开发链路交付性**——为 `association-desktop-cdp.js` 补 probe 降级（r13）、解除三个开发脚本的 `.gitignore` 忽略并补 README 说明（r14，经用户裁定），实跑 dev-proxy 验证。详见 8.10。
- **第五轮（r15~r16）**：统一**取消标志内存序**（r15，`Relaxed`→`Acquire`）、收敛**生产路径 `expect`**（r15，7 处）、并**更正一项误报**（r16：「缺请求体限制」实为 axum 默认 2MB 保护）。全量回归**三项全绿**。详见 8.11。
- **第六轮（b1~b8）**：用户将范围升级为「直到**全部**待修复完成」，覆盖此前裁定"仅登记、不启动"的全部结构性项。闭环 **P0-2 依赖倒置**（b7，**根因是许可层错位而非架构错**）、**P1-6 错误处理契约层**（b8a，6/6 类型补齐 trait）、**God Object 测试岛拆分**（b8b，`v1_api.rs` 9012→4529 行）、**原子写入 4 处收敛**（b8b）、**锁序契约文档化**（b8c）、**`#[allow(dead_code)]` 移除实验核实**（b8d，16 保留/3 删）、**`//!` 模块文档 25 个**（b8d）、**认证默认策略启动告警**（b8e）。全量回归**全绿**（fmt / clippy ×2 / 625 tests / 90 desktop tests / node --check ×6 / 契约）。详见 8.12。
- **第七轮（b10~b13）**：继续推进 8.6 剩余未闭环项。闭环 **API 层职责混合**（b11，删除 `server.rs` 168 行注释化死代码）、**SidecarManager 持锁跨 Phase**（b11，删除 3 个零调用方的遗留 API，使该模式在**类型层面不可达**）、**Tauri 命令归属**（b12，4 个孤儿命令在注册处显式标注 + **新增契约门禁**使"注册未调用"从静默变为显式申报，含对照实验确证门禁可失败）。**feature 矩阵**与 **postgres 构建**经实测**更正定性**（见 8.13.4）。全量回归**全绿**。详见 8.13。
- **未闭环项：无**。第七轮末的 4 项（P1-6 签名改造、`MemoryStore` 字段级拆分、后端选型、`spawn_blocking` 不可强杀）**已由第八轮（c2~c8）全部处置**：前三项闭环为代码改动，最后一项闭环为**补偿路径**；后端选型**定性维持为产品决策**（并补证 Qdrant 的 `Unsupported` 属 fail-closed 正确设计）。详见 8.6 / 8.14。
- **交付性提醒**：八轮累计新增 **16 个交付物**（`.githooks/pre-commit`、`scripts/enable_git_hooks.ps1`、两份 HCSE 清单、三个开发链路脚本、**8 个 `src/*.rs` 源文件**——`model_ids.rs`/`atomic_file.rs`/`v1_api_tests.rs`/`memory_store_tests.rs`/`memory_state_machine.rs`/**`errors.rs`**/**`memory_store_types.rs`**/**`memory_store_cache.rs`**）及本报告，均为**未跟踪文件**。**其中源文件若未提交将导致克隆后编译失败**，须优先 `git add`（见 8.7 / [HCSE_RELEASE_PROTOCOL.md](file:///g:/code-memory/docs/HCSE_RELEASE_PROTOCOL.md) 检查项 4.3）。
- **环境提醒**：本机 3099 端口为**已安装的 v0.9.6 侧车**（旧前端），与工作区 v0.9.7 失配；直接对其跑 `playwright-smoke.js` 会在既有首个断言（`#value-hero`）失败，该失败与本轮改动无关（详见 8.9「环境说明」）。**另**：`postgres` feature 在本机**离线环境无法构建**（`sqlx` 依赖链的 `atoi v2.0.0` 未缓存，需联网下载），该失败**非代码缺陷**，详见 8.13.4。
