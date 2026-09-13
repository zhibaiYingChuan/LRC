# HCSE 韧性审计清单（LRC）

> 本文件按 HCSE 通用框架「智能体调用后的处置规则」第 2 条回写，收录**本轮全维度代码审查未能预警、或属现有检测范围之外**的故障模式，转为可复用的检查项。
> 来源：`docs/GLOBAL_CODE_REVIEW_REPORT.md` 第五节「HCSE 回写建议」第 1、2 项。
> 适用对象：任何新增/修改 UI 交互、异步调用、超时逻辑、包含全局状态（环境变量）的测试的变更。

---

## 一、检查项 1：超时机制验证

**针对的故障模式**：代码中"存在超时"不等于"超时真正触发"。本轮审查发现项目有 76 处 `fetchWithTimeout` 与 5 处 `invokeWithTimeout`，但**超时/卡死路径无任何端到端测试**——超时参数写错、被吞掉、或兜底反馈缺失，都不会被门禁发现。

### 1.1 现状基线（实测，变更时须比对）

| 层 | 机制 | 位置 | 参数 |
|---|---|---|---|
| 前端 HTTP | `fetchWithTimeout(url, options, timeout=10000)` | [app.js:342](file:///g:/code-memory/static/app.js#L342-L344) | `AbortController` + `setTimeout` 默认 10s |
| 前端 Tauri IPC | `invokeWithTimeout(invokeFn, cmdName, args, timeoutMs)` | [app.js:4875](file:///g:/code-memory/static/app.js#L4875-L4889) | `Promise.race` + `setTimeout`，5 处调用点（3s / 10s / 15s） |
| 后端 handler | `tokio::time::timeout(...)` | [v1_api.rs:90](file:///g:/code-memory/src/v1_api.rs#L88-L92)（锁 2s）、`:1517`、`:1879`、`:3869`（各 15s）、`:=4350`（10s） | 逐端点显式 |
| 关联探索 | 内部预算 `ASSOCIATION_EXPLORE_TIME_BUDGET` | [v1_api.rs:603](file:///g:/code-memory/src/v1_api.rs#L603)、`:907` | 内层 10s ＜ 外层 HTTP 15s |
| 桌面端健康检查 | 单请求 2s / 总预算 40s / 取消粒度 10 端口 | [sidecar_manager.rs:62-67](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L53-L67) | 三者集中定义，注释/实现/日志同源 |

### 1.2 强制检查步骤

1. **参数可证**：新增/修改任何超时调用，必须能用 Grep 指出**唯一的常量化定义**（禁止在调用点散落魔法数字）。若同一语义存在多处定义，视为缺陷。
2. **分层单调**：内层预算必须严格小于外层超时。关联探索已有断言守护（[v1_api.rs:7840](file:///g:/code-memory/src/v1_api.rs#L7839-L7840) `max_a < HTTP_TIMEOUT_SECS`），新增链路必须补同类断言。
3. **取消真正生效**：长循环内必须有取消/超时检查点（如 `HEALTH_CHECK_CANCEL_CHECK_STRIDE` 每 10 个端口一次），禁止只检查一次。
4. **兜底反馈存在**：超时后 UI 必须产出**用户可见**的失败态（错误提示 + 可重试），不得静默卡死。审查本项时须回答：超时抛出后，调用方 `catch` 是否有渲染分支？
5. **异常路径四件套**（每项须有明确答案）：
   - 超时路径：长时间无响应时 UI 是否有兜底反馈？
   - 卡死路径：底层调用永不返回时 UI 能否恢复？
   - 错误路径：失败时是否有明确提示 + 状态恢复？
   - 取消路径：用户取消时能否正确中断 + 清理（含 `removeEventListener`、`clearTimeout`、`pendingRequestCount` 归还）？

### 1.3 已知未闭环缺口（须持续跟踪）

- ~~**UI 层超时路径无端到端测试**~~ → **已闭环（2026-09-12）**：详见下方 1.4。
- **验证方式建议**（已落地）：在 Playwright 冒烟中注入"永不返回的端点"，断言超时后出现错误提示且页面仍可交互。

### 1.4 超时路径端到端验证（已落地，2026-09-12）

**背景**：本检查项原判定"前端超时后 UI 是否恢复无任何自动化验证"。实测发现该判定**低估了问题严重性**——不仅缺测试，**该 UI 分支本身就是不可达死代码**。

**根因（实测确证）**：`loadDashboard` 用 `Promise.allSettled` 并发三个健康检查请求，其**不抛出**的语义抹平了 rejection 的 `reason`；随后 [`!systemData && !daoData` 分支](file:///g:/code-memory/static/app.js#L1434) **无条件**抛 `无法连接到 API 服务`，导致 catch 中的 [`e.name === 'SidecarTimeoutError'` 分支](file:///g:/code-memory/static/app.js#L1483-L1492)（渲染"请求超时" + 重试按钮）**永不执行**。用户遇到"底层调用卡死"时，被误导为"服务未启动"，且拿不到超时专属恢复入口。

**修复**：在无任何数据可用时，优先回查三个 `allSettled` 结果的 rejection `reason`，若存在 `SidecarTimeoutError` 则还原该错误类型再抛出（**不改变"何时进入错误态"的判定，只改变错误类型**）。

**验证方法与实测结果（真实浏览器执行）**：

| 步骤 | 手段 | 实测 |
|---|---|---|
| 注入卡死 | 本地静态服务托管 `static/` + Playwright `page.route('**/v1/health/**', () => {})`（handler 不调用任何 route 方法 → 请求永久 pending） | — |
| 触发 | `window.loadDashboard()` | 超时耗时 **11121ms**（与 `fetchWithTimeout` 默认 10s 硬超时吻合，**证明超时真正触发**而非仅存在于代码中） |
| 断言 | 错误态文案 / 重试入口 / loading 收起 / 页面可交互 | `errorText="请求超时，请检查网络连接后重试"`、`hasRetryButton=true`、`retryDisabled=false`、`loadingHidden=true`、`pageInteractive=true` → **全部通过** |
| **对照实验** | 临时还原旧逻辑（无条件抛"无法连接"）后复跑 | **失败**：`await-timeout-ui` 阶段 25s 超时，**永不出现"请求超时"文案** → 确证原分支为死代码，修复具因果必要性 |
| 恢复复检 | 还原修复后再次复跑 | **通过**，`elapsedMs=11153`，结果与首次一致 |

**固化位置**：断言块已内嵌 [playwright-smoke.js](file:///g:/code-memory/tests/frontend/playwright-smoke.js#L85-L127)（"超时/卡死路径韧性验证"段），随 CI `frontend-test` 的 `Run Chromium browser regression` 步骤执行；同时输出 `dashboard-timeout.png` 与 `timeout-path.json` 作为可复核证据。

**新增强制步骤**：任何改动 `loadDashboard` 错误分类逻辑、或新增基于 `Promise.allSettled` 的并发请求聚合，**必须保留 rejection `reason` 的语义**——`allSettled` 的"不抛"特性会静默抹平错误类型，使下游 `catch` 的分类分支退化为死代码。该类缺陷**编译期与常规单元测试均不可见**，只能由本项 E2E 拦截。

**已知边界**：本 E2E 仅在 sidecar 存活且**未处于 `lock_busy`** 时触发超时分支（若 `lock_busy=true`，`loadDashboard` 会走降级渲染而非抛错），CI 侧依赖"启动后即跑"的时序前提。

---

## 二、检查项 2：测试隔离性

**针对的故障模式**：测试通过 `std::env::set_var` 写入进程级全局状态，在并行测试下相互污染，导致**非确定性通过/失败**。本轮审查发现该类写入 **25 处**，且分布在库代码的测试模块内（库测试与集成测试可能同进程交互）。

### 2.1 现状基线（实测）

`std::env::set_var` 命中 25 处，按文件：

| 文件 | 处数 | 说明 |
|---|---|---|
| `src/memory_store.rs` | 10 | 多为"保存原值→改写→断言→还原"模式（见 [:4975](file:///g:/code-memory/src/memory_store.rs#L4974-L5007)） |
| `src/bin/server.rs` | 5 | 代理/镜像/开发模式开关 |
| `src/engine/model_downloader.rs` | 3 | 镜像源切换 |
| `src/v1_api.rs` | 2 | 关联特性开关 |
| `src/process_guard.rs` | 2 | LLM API 注入 |
| `tests/benchmarks.rs` | 2 | 状态偏置 |
| `desktop/.../main.rs` / `commands.rs` | 各 1 | WebView2 参数 |

**已落地的缓解**：CI `test` Job 与 `coverage` Job 均以 `--test-threads=1` 串行执行（[ci.yml:138](file:///g:/code-memory/.github/workflows/ci.yml#L137-L150)、[:201](file:///g:/code-memory/.github/workflows/ci.yml#L197-L201)），消除了跨测试并发竞态。

### 2.2 强制检查步骤

1. **串行门禁不可移除**：`--test-threads=1` 是当前隔离性的唯一保障，**任何移除该参数的变更必须同时完成 `set_var` 消除**（改为参数注入或线程局部状态），否则视为回归。
2. **还原必须成对**：新增 `set_var` 的测试必须使用 `match previous { Some(v) => set_var(..), None => remove_var(..) }` 模式还原原值，禁止直接 `set_var` 后不还原。
3. **跨文件污染自查**：修改上述任一段代码后，须确认其测试**不依赖执行顺序**（用 `--test-threads=1` 正序与倒序各跑一次均可复现）。
4. **新增环境变量开关**：必须登记到本表，并评估是否需要加入 CI `env` 白名单（避免 CI 环境残留值影响断言）。

---

## 三、检查项 3：五层交互韧性覆盖

按 HCSE 框架第四节模型，每次涉及交互层的变更须对照下表自评：

| 层级 | 本轮覆盖情况 | 缺口 |
|---|---|---|
| L1 一级页面 | 有（`playwright-smoke.js` 仪表盘冒烟） | 数据为空/超时态未覆盖 |
| L2 二级弹窗 | 有（`cdp-regression.js`） | 打开失败/操作超时未覆盖 |
| L3 三级卡片 | 部分（联想中心） | 卡片内容加载失败未覆盖 |
| L4 四级嵌套 | 无 | 嵌套操作超时/状态不恢复未覆盖 |
| L5 异常全局 | 无 | 网络断开/进程崩溃未覆盖 |

**检查项**：变更若触及 L3–L5，须在 PR 描述中显式回应"异常路径是否已覆盖"，未覆盖者须登记为跟踪项。

---

## 四、检查项 4：错误传播链完整性（2026-09-13 第六轮新增）

### 4.1 现状基线（实测，2026-09-13）

| 错误类型 | `Display` | `std::error::Error` | `source()` |
|---|---|---|---|
| `GuardError`（`process_guard.rs`） | ✅ | ✅ | — |
| `SearchError`（`server.rs`） | ✅（b8a 补） | ✅（b8a 补） | — |
| `ApiError`（`server.rs`） | ✅（b8a 补） | ✅（b8a 补） | — |
| `PersistenceError`（`persistence/mod.rs`） | ✅ | ✅ | ✅（b8a 补） |
| `EmbedError`（`engine/embedder.rs`） | ✅ | ✅ | ✅ |
| `DownloadError`（`engine/model_downloader.rs`） | ✅ | ✅ | ✅（b8a 补） |
| `IntegrityError`（桌面端） | ✅ | ✅（b8a 补） | — |

**实测复检命令**：`grep -n 'Display for \w*Error\|Error for \w*Error'`，要求 **6/6** 同时命中。

### 4.2 强制检查步骤

1. **新增错误类型必须实现三件套**：`Debug` + `Display` + `std::error::Error`。
   缺 `Display` 会导致调用方无法打印人类可读原因；缺 `Error` 会导致无法用 `?`
   融入 `Box<dyn Error>` 生态（两者都是**运行时可用性**问题，不是风格问题）。
2. **包裹底层错误时必须实现 `source()`**：若 enum 变体持有 `io::Error` / `serde_json::Error`
   等底层错误，空的 `impl Error for X {}` 会让 `source()` 恒为 `None`，
   **根因在错误链中丢失**。须显式返回 `Some(e)`。
3. **错误链须可逐层打印**：涉及多级错误的路径（如 `PersistenceError::Io(io::Error)`），
   验证方式为 `err.source()` 非空且指向真实根因。
4. **未闭环边界**：全仓仍有 **74 处 `Result<_, String>`**（多为 trait 契约与 CLI 边界）。
   这些**不代表缺陷**——它们是既有的边界形态；但**新增**的库内错误路径应优先用
   enum 而非 `String`。

### 4.3 已知未闭环缺口

- 74 处 `Result<_, String>` 签名改造（需引入 `thiserror` 并改动跨层签名）属**独立排期**。
- `GuardError` / `SearchError` / `ApiError` / `IntegrityError` 的变体**不包裹**底层错误，
  故 `source()` 返回 `None` 是**正确**的（无链可回溯），非缺口。

---

## 五、检查项 5：锁序与临界区（2026-09-13 第六轮新增）

### 5.1 现状基线（实测，2026-09-13）

| 模块 | 锁 | 与其它锁的关系 |
|---|---|---|
| `persistence/json.rs` | `cache`(RwLock) **→** `JSON_WRITE_LOCK`(Mutex) **→** 进程文件锁 | 三级固定顺序，**禁止逆向**（见该文件「锁序契约」注释） |
| `backup.rs` | `BACKUP_OPERATION_LOCK`(Mutex) | 与持久层**无环路**（备份不持持久层锁；持久层不调备份） |

### 5.2 强制检查步骤

1. **新增锁必须登记锁序**：在本表补充该锁的获取顺序与相对位置；若无法给出全局顺序，
   须说明为何不存在环路（如"两模块互不调用"）。
2. **持锁期间禁止跨模块调用**：持锁期间调用会获取其它锁的函数 = 潜在 ABBA。
   例外：备份的"持锁磁盘 IO"是**正确性要求**（快照语义），须显式注明为有意接受。
3. **`#[allow(dead_code)]` 判断禁用人工阅读**：对声称"已过时"的 allow，
   须执行「**移除 → `cargo check --all-targets`**」，以编译器输出为唯一判据
   （第六轮实测：报告称 19 处过时，实际仅 3 处）。
4. **原子写入必须唯一实现**：`atomic_file::write_atomic` 是全仓唯一实现
   （UUID 临时名 + 失败清理）。**禁止**再手写"固定 `.tmp` + rename"。

---

### 5.3 第七轮补充：注释化死代码与易误用 API（2026-09-13）

**（1）注释掉的代码不受任何静态检查覆盖（M7）**

`dead_code` / clippy **只分析活代码**。因此把废弃代码用 `/* ... */` 包起来，
会造成"**既不生效、也不报警**"的盲区——比直接删除更危险。

> 实例：`src/server.rs` 中 168 行桌面环境探测代码（`detect_command_tool` / `which_path` /
> `check_windows_install_path` / `check_vscode_extension`）曾被块注释包裹，Grep 反查确认零引用后删除。
> **处置原则**：废弃代码应**删除**，版本历史由 git 承载，不要留在源码里"以防万一"。

**（2）消除易误用 API 优于文档警告（M8）**

`SidecarManager` 曾有 `start()` / `start_for_project()` / `restart_project()` 三个接收 `&mut self`
的方法，会在**持锁状态**下执行 Phase 2 健康检查（最多 40s）。文档已写明"应使用三阶段编排"，
但类型仍允许误用。删除这三个方法后，唯一可用的编排是
`prepare_start`（持锁）→ `spawn_and_wait`（不持锁）→ `insert_handle`（持锁），
**"持锁跨 Phase"在类型层面不可达**。

**（3）检查项：新增/修改锁相关 API 时**

1. 若方法接收 `&mut self`（或 `&self` 配合内部可变性）且内部执行 **I/O**，
   须确认调用方是否会因此持有锁跨 I/O。
2. 优先设计为**关联函数**（不接收 `self`），使调用方可自由决定持锁边界。
3. 若存在"文档要求 A、类型允许 B"的偏差，**修类型**而非加文档。

---

## 六、检查项 6：门禁有效性（2026-09-13 第七轮新增）

### 6.1 原则

**门禁必须能失败**——一个从不失败的检查等于没有检查。新增任何门禁后，
须用**对照实验**证明其在违规输入下确实返回失败。

> 实例（b12）：为 `validate_frontend_contract.js` 新增「后端注册但前端零调用的命令须显式申报」检查后，
> 临时移除白名单中的一条 → `MUTATED_EXIT=1`（并报出双向错误：未申报 + 白名单腐化）；
> 恢复后 `RESTORED_EXIT=0` 且文件**逐字节一致**（`RESTORE_BYTE_EXACT=True`）。

### 6.2 强制检查步骤

1. **新增门禁必做对照实验**：构造一个违规输入 → 确认失败；恢复 → 确认通过；确认恢复是**逐字节**的。
2. **门禁清单须双向校验**：白名单类门禁（如孤儿命令白名单）不仅要检查"未申报的失败"，
   还要检查"已在白名单但已不再是例外的失败"——否则白名单会随时间腐化。
3. **区分"依赖缺失"与"代码缺陷"**：构建失败时先看是否为
   `error: failed to download ... --offline` 一类**环境问题**（如 `postgres` feature 在离线环境
   因 `atoi` 未缓存而失败），不应记为代码风险。

---

## 七、检查项 7：异步硬限制下的补偿路径（2026-09-13 第八轮新增）

### 7.1 针对的故障模式

**"超时保护"只是返回了错误，底层工作仍在继续**。Rust 运行时中
`tokio::task::spawn_blocking` 提交的任务**无法被强杀**——`timeout(...)` 到点只会
drop `JoinHandle`（分离），阻塞线程仍会把整个计算跑完。其后果是：

1. **CPU/线程池被白耗**：最坏情况（如 O(n²) 聚类 12.5 万次比较）在超时后仍跑满；
2. **结果被静默丢弃**：任务跑完后返回值无人接收，属纯浪费；
3. **误判为"已完成"**：若调用方按"超时即本轮结束"处理，可能错误清除待重试标记。

> 实例（c2）：`consolidation.rs` 的 `run_cycle` 用 `timeout(120s, ...)` 包裹，
> `v1_api.rs` 的 `/v1/memories/consolidate` 外层是 `TimeoutLayer(30s)`——
> 两者都只"返回错误"，`spawn_blocking` 内的 `plan_luoshu` / `plan_jaccard`
> 仍会在后台跑满 O(n²)。

### 7.2 强制检查步骤

1. **凡 `timeout` 包裹 `spawn_blocking` 处，逐处判定**：该任务能否被协作式取消？
   不能则须实现补偿路径。
2. **补偿路径三要件**（缺一不可）：
   - **标志位**：`Arc<AtomicBool>`，写入用 `Release`、读取用 `Acquire`（**必须配对**；
     `Relaxed` 读与 `Release` 写**不构成同步**）；
   - **检查点**：置于**循环体内**且间隔可控（如每 N 次迭代），使"多跑的剩余工作"
     有上界。检查点过密会侵蚀计算本身，过疏则补偿失效；
   - **不完整结果处置**：取消时必须**丢弃**不完整结果（不得写回），
     并以**错误**（而非 `Ok(0)`）返回，以便调用方**保留待重试标记**。
3. **失败注入验证**：不能用"代码里写了检查点"代替验证。应构造让超时**真正触发**的
   用例（如注入永不返回的依赖），断言取消路径被走到且数据未被污染。

> **反例警示（务必避免）**：若取消路径返回 `Ok(0)`，调用方的成功分支会清除
> `synthesis_pending`——于是"本轮未完成"被误记为"已完成"，下一轮不再重试，
> **数据永久不结晶**。这是比"超时"本身更隐蔽的缺陷。

### 7.3 已知边界

- **不可取消的临界区必须标出**：如"持锁写回"阶段一旦开始就不应中途放弃
  （否则留下半写状态）。本项目的处置是——**只在 Phase 2（锁外纯计算）设检查点**，
  Phase 1/3（短临界区）不设。
- 补偿路径只降低**浪费**，不改变**语义**：超时后调用方仍应视本轮为失败。

---

## 八、检查项 8：前置条件须被验证而非采信（2026-09-13 第八轮新增）

### 8.1 原则

**任何"必须先做 X 才能做 Y"的论断，都是待验证的假设，不是事实。** 伪前置的代价是
**无限期阻塞**：一个本可独立完成的重构，会因一个不成立的前提被登记为"独立排期"。

> 实例（c7）：报告判定"`MemoryStore` 字段级拆分**需先完成 `RefCell → Sync` 并发模型改造**"。
> 实测：全仓 8 处共享方式**均为** `Arc<Mutex<MemoryStore<P>>>`，而标准库
> `impl<T: ?Sized + Send> Sync for Mutex<T>` 表明 **`Mutex<T>: Sync` 只要求 `T: Send`**。
> `RefCell`/`Cell` 在 `T: Send` 时即为 `Send`，故 `Sync` **从来不是前置条件**——
> 该重构当场即可完成（最终以 629 tests 不变证明等价性）。

### 8.2 强制检查步骤

1. **拆解前置条件为可判定的类型/语义命题**：
   - ❌ "需要先改并发模型"（不可判定）
   - ✅ "`Arc<Mutex<T>>` 要求 `T: Send` 还是 `Sync`？"（可查标准库 trait bound）
2. **在标准库/语言规范层面求证**，而非在项目代码里"感觉"。
   常见伪前置：
   - "`RefCell` 不是 `Sync` ⇒ 不能跨线程"（**错**：套 `Mutex` 即可，只需 `Send`）；
   - "改了 A 就必须先改 B"（**须查**：B 的约束是否真的作用到 A）；
   - "离线构建失败 ⇒ 代码有问题"（**须查**：是否仅依赖未缓存，见检查项 6.2 第 3 条）。
3. **区分"物理耦合"与"逻辑耦合"**：字段级拆分只要求**类型可独立编译**
   （物理），不要求**并发模型改变**（逻辑）。
4. **结论须落盘更正**：伪前置一旦被证伪，应在报告中原判定处**显式标注更正**
   （含证伪依据），而非仅在下游改代码——否则下一位维护者会重蹈覆辙。

---

**回写溯源**：本清单由 `GLOBAL_CODE_REVIEW_REPORT.md` 第五节 1、2 项转化；所有计数（76 / 5 / 25）均为审查时点实测值，变更时须重新取证而非沿用。检查项 1.4 于 2026-09-12 由第二轮修复轮补充（对应报告 8.8 r11）；**检查项 4、5 于 2026-09-13 由第六轮（b1~b8）补充，检查项 5.3、6 由第七轮（b10~b13）补充**（对应报告 8.12 / 8.13）；**检查项 7、8 于 2026-09-13 由第八轮（c2~c8）补充**（对应报告 8.14）。

**交付状态（2026-09-13）**：本文件原被 `.gitignore:318`（`docs/hcse_resilience_*.md` 通配）忽略——即回写动作产出的清单**自身不可交付**。经用户裁定解除忽略（`.gitignore:424-425` 负向规则），复检 `git ls-files --others --exclude-standard` 已列出本文件；但**仍为未跟踪状态，须 `git add` 后方可随克隆交付**。详见同级 [HCSE_RELEASE_PROTOCOL.md](file:///g:/code-memory/docs/HCSE_RELEASE_PROTOCOL.md) 检查项 3.1「自指事故」与检查项 4.1。
