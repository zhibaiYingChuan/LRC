# v0.9.9 交互可用性修复 · 实测报告（2026-09-18）

> 触发：用户反馈四件事 —— ①能否编译 0.9.9 并重跑 CDP ②所有交互功能要从**用户角度**考虑
> ③结晶是否还有问题 ④**每次打开要手动点启动服务，能否自动启动**
> 方法：记忆库 recall → 代码勘察 → 实测定位 → 委托 subagent 全量审计 → **主 agent 逐条核实** →
> 修复 → 页面级取证 → CDP 三套件回归
> 结论：**四件事全部闭环**；CDP 三套件全绿

---

## 一、自动启动：根因与修复（用户报的第 4 件事）

### 1.1 根因（有日志实锤）

`temp/tauri-dev-run.log` 第 38 行：

```
[v0.8.16 自动启动] wizard 未完成配置（setup_complete=false, file_existed=true），跳过自动启动
```

而同一台机器上：记忆库 **3175 条**、AI 规则已写入 **4 个工具**
⇒ 用户明显在正常使用，配置**实际可用**，却被判为"未配置"。

**根因是判据选错了对象**：

| 项 | 说明 |
|---|---|
| 原判据 | `wizard.setup_complete`（= **是否走过向导**） |
| 但正常使用不需要走向导 | 全局模式（不选项目目录）⇒ `project_dir == None`；不用 LLM ⇒ `llm_configured == false` |
| 而自动迁移要求 | `project_dir.is_some() && llm_configured`（`config_wizard.rs:330`） |
| ⇒ 结果 | 该路径下 `setup_complete` **永远为 false** ⇒ 自动启动被永久跳过 |

### 1.2 修复：判据改为「能否正常工作」

新增 `WizardState::has_usage_history()`（`desktop/src-tauri/src/config_wizard.rs`）：

```
setup_complete ∨ 有项目目录 ∨ 配过 LLM ∨ 写过规则(rules_agents) ∨ configured_agents
```

`main.rs` 的自动启动判据改用它。**首次安装**（所有痕迹为空）仍不自动启动，引导走向导
—— 原有意图完整保留。

### 1.3 实测验证（两次，均未手动点击）

| 次数 | 前置 | 结果 |
|---|---|---|
| 第 1 次 | 先 `Stop-Process` 杀掉 3111 | 桌面端日志：`Sidecar 已启动: PID=24832, port=3111`、`[自动启动] sidecar 启动成功，端口 3111` |
| 第 2 次 | 杀 lrc-desktop + lrc-sidecar | 桌面端日志：`Sidecar 已启动: PID=26508, port=3111` |

⇒ **不再需要手动点击**。失败时仍走原有 `sidecar-auto-start-failed` 事件 → 前端横幅提示手动启动
（用户要求的降级路径保留）。

### 1.4 测试与 mutation 验证

新增 3 条测试（`config_wizard.rs`）：

| 测试 | 锁定 |
|---|---|
| `test_has_usage_history_false_for_fresh_install` | 全新安装**不得**被判为有痕迹（否则首次打开跳过向导） |
| `test_has_usage_history_true_for_global_mode_with_rules` | **复刻实测现场**：`setup_complete=false` + 全局模式 + 不用 LLM + 有 rules_agents |
| `test_has_usage_history_each_signal_alone_suffices` | 判据取「或」而非「且」 |

**Mutation 验证**：把 `has_usage_history` 还原为只 `setup_complete`
⇒ **2 条新测试当场变红**（`1 passed; 2 failed`）⇒ 测试确实锁住了这个 bug。

---

## 二、结晶：实测结论（用户问的第 3 件事）

### 2.1 结晶**本身正常**

`/v1/memories/stats` / `synthesis-timeline` / `health/system` 实测（Python 按 UTF-8 解码，
不用 PowerShell —— 后者按 Latin-1 解码会把中文变乱码）：

| 指标 | 实测值 |
|---|---|
| `by_type.synthesis` | **33** |
| `dao_metrics.crystallized_memories` | **33** |
| `dao_metrics.synthesis_ratio` | **1.04%** |
| 时间线产物 | 真实存在（如「洛书合成·愉悦表达」77 条来源融合，confidence 0.9999） |
| 结晶流水线 | 启动日志：`[LRC·结晶] 后台结晶流水线已启动（间隔 5 分钟，本地统计合成模式）` |

⇒ **不是"结晶坏了"**。

### 2.2 但发现两个真问题（已如实记录）

| # | 问题 | 证据 | 性质 |
|---|---|---|---|
| 1 | **`system_mode = degraded`** | ML 编码器未启用，退回统计编码器；启动日志：`编码器模式: 基础模式（建议配置 LLM 或下载 ML 模型增强语义理解）` | 与记忆 #2（v0.8.46 诊断）预判的风险一致：**降级模式下合成质量下降** |
| 2 | **告警系统自相矛盾** | 两条 `action_hints` 已**连续出现 449 次**且"级别提升"，其中一条却写「用户反馈正面率 100.0，**系统输出质量良好**」 | 告警系统缺陷：一条降级告警与一条"质量良好"同时要求 `action_required` |

> 第 2 项是**新发现**（本轮首次暴露）：`action_hints` 的"连续出现 N 次 + 级别提升"机制
> 与文案内容未做一致性校验。建议后续单独处理，本轮不扩大范围。

---

## 三、交互审计：从用户角度（用户要求的第 2 件事）

### 3.1 方法

委托 subagent 全量审计 12 个 tab 的界面结构与渲染/事件处理，
产出 **3 个 P0 + 11 个 P1 + 8 个 P2**；**主 agent 逐条用代码核实**后才修
（三个 P0 均以 grep 证实，非凭 subagent 结论）。

### 3.2 已修复（P0 三项）

| # | 问题 | 核实证据 | 修法 |
|---|---|---|---|
| **P0-1** | 记忆类型**裸英文枚举**上屏（`code_context`/`synthesis`） | 全仓有 3 份类型映射表，但**最高频的三条路径**（列表卡片/详情面板/联想补全）都没用它们 | 新增**全局唯一** `memoryTypeLabel()`，替换三处裸枚举 |
| **P0-2** | 预设场景承诺"一键启用"，**实际不产生任何效果** | ①`#preset-scenario-info` 容器**在 HTML 中不存在**（全仓仅 app.js 一处引用）⇒ 说明永不显示；②`src/` 下 grep `scenario` **零匹配**；③文案里的 `note`/`task`/`knowledge` **都不是后端合法类型** | 补容器；文案改为**如实说明**（"当前版本仅记录偏好，尚未自动改写记忆类型与标签体系"）；补 toast 回执 |
| **P0-3** | 「查看结晶历史 →」点了**没有历史可看** | `#crystallization-timeline` **只被 app.js:8310 读**，HTML 中不存在 ⇒ 函数首行就 return | 按钮文案改「查看结晶成果 →」（与实际展示一致） |

### 3.3 已修复（P1 / P2 代表项）

| # | 问题 | 修法 |
|---|---|---|
| P1-7 | 搜索超时时**两条互相矛盾的提示**（toast 说"后台合成中"、面板说"搜索超时"） | 503 按 `errorDetail` 细分文案（`search_timeout`/`search_busy`/`lock_busy`）；顺带让 503 也走 `humanizeErrorDetail` 的正确文案 |
| P1-5 | 「联想足迹」把记忆 **UUID 当正文**展示 | 新增 `shortIdLabel()`：优先内容预览，退化才显示 ID 前 8 位 |
| P1-4 | 「运行模式」显示英文枚举（`degraded`） | `statusBadge` 补文本映射（与系统状态页中文口径一致） |
| P1-8 | 列表卡片显示**未格式化 ISO 时间戳** | 新增 `formatAnyTime()` 同时处理毫秒与 ISO 字符串 |
| P1-10 | 筛选面板**缺「经历」**，但首页可跳转到它 | 补 `<span data-filter-type="experience">经历</span>` |
| P1-9 | 「结晶**次数**」「衰减**次数**」标签口径错误（实为**条数**） | 改「已结晶记忆（条）」「已过期记忆（条）」 |
| P1-6 | 联想小统计暴露 `fast`/`deep` 通路名 | 改「关键词命中 / 语义命中」 |
| P2-1 | 原生 `window.confirm`（阻塞 WebView JS 线程） | 改用全站统一的 `showConfirm` |
| P2-4 | 搜索失败**无重试入口** | 补「重试」按钮 + `retryMemorySearch` |
| P2-5 | 空态只说"暂无内容"，不给下一步 | 改「点右上角「记一笔」写入第一条」 |

### 3.4 页面级取证（不只改源码，还在真实 WebView 里验）

| 项 | 实测输出 |
|---|---|
| 类型映射 | `code_context→代码`、`synthesis→结晶知识`、`unknown_x→其他`、`''→其他` |
| 详情面板（真实点击） | `记忆类型=结晶知识`、`重要性=8/10`、`创建时间=2026/9/8 17:40:05` |
| 列表卡片 | `cardType=结晶知识`（原为 `synthesis`） |
| 筛选标签 | 全部/事实/偏好/决策/代码/对话/**经历**/结晶知识 |
| 统计卡标签 | 已结晶记忆（条）/ 已过期记忆（条）/ 合成执行次数 / 总记忆数 |
| 按钮文案 | `查看结晶成果 →` |
| `statusBadge` | 无英文枚举残留（实测输出 `未知` / `部分数据暂不可用`） |
| `shortIdLabel` | 有预览 → `游西湖那天`；无预览 → `9f3a1c2e…` |
| `formatAnyTime` | ISO → `2026/9/19 18:00:00`；毫秒 → `2026/9/19 00:07:11`；非法 → `--` |

---

## 四、跨语言契约测试（本轮新增）

前端无法 `use` Rust 函数，只能在 `app.js` 里**再列一份**符号层类型清单
—— 这正是「两处列举必然漂移」的典型场景。新增测试把它钉住：

`test_frontend_symbolic_relations_matches_backend_authority`（`v1_api_tests.rs`）
从 `static/app.js` 抽取 `SYMBOLIC_RELATIONS` 字面量，与后端 `is_symbolic_edge_type`
**双向比对**。

**Mutation 验证**：故意从 `SYMBOLIC_RELATIONS` 删掉 `coordinate`
⇒ 当场变红，报错正是预期的诊断文案：
> ★★后端认定 `coordinate` 是符号层，但前端 SYMBOLIC_RELATIONS **漏列**它
> ⇒ 该类型的边会被静默渲染成「记录关联」（把可能不成立的推测说成必然事实）

---

## 五、★过程中被 CDP 抓出的一个**我自己引入的 bug**

改 `loadAssociationCenterObservation` 时，我把 `const recent = ...` 那一行删掉了，
但保留了 `if (recent)` 的使用 ⇒ `ReferenceError: recent is not defined`。

| 项 | 说明 |
|---|---|
| 抓出者 | `cdp-regression.js`（tab 切换含该函数） |
| 静态检查 | `node --check` **通过**（这是运行时错误，语法检查抓不到） |
| 单测 | `cargo test` 全绿（Rust 侧不覆盖前端运行时） |
| **唯一能抓到它的** | **CDP 回归** |

> **教训**：前端改动后，"语法通过 + 单测全绿"**不等于**能用。
> CDP 回归是前端可用性的**唯一门禁**，不可跳过。

---

## 六、CDP 回归结果（三套件全绿）

> **§6.1 更正说明（2026-09-19 追记）**：本节原先记录的"全绿"是在**未启用 ML** 的
> 开发端上取得的。启用 ML 后重跑，联想套件一度只有 **3/7**。根因与最终修复见 §十。
> 下表为**修复后**（ML 已启用）的复测结果。

| 套件 | 结果 |
|---|---|
| `cdp-regression.js` | **发布门禁 PASS**：`action{PASS:100, SKIP_INVISIBLE:4}`、`tab{PASS:10}`、`input{SKIP_PROTECTED:9, PASS:14}`，**0 WARN** |
| `association-desktop-cdp.js` | **7/7 PASS**（B 场景返回诚实空态、C 场景命中 10 条真实记忆） |
| `symbolic-layer-desktop-cdp.js` | **7/7 PASS**（`symbolicTagCount=1`、文案「结构推导」、`hasRecordTag=false`） |

环境：3111（**由桌面端自动拉起**）+ CDP 9231 + 页面 `https://tauri.localhost/`。

---

## 七、测试与门禁总览

| 项 | 结果 |
|---|---|
| `cargo test --features server` | **787 passed / 0 failed**（786 → +1 跨语言契约） |
| `cargo test`（desktop crate） | **96 passed / 0 failed**（93 → +3） |
| `cargo clippy --all-targets` | 无 warning / error |
| `check_algorithm_leak.py` | 退出码 **0** |
| 版本号 | 7 处已升 **0.9.9** |

---

## 八、收尾轮：剩余项全部闭环（2026-09-19）

### 8.1 `action_hints` 自相矛盾 —— **已修**（根因与 §二 的初始判断不同）

初始只记录了"现象"。深挖后找到**根因**：

`HintEscalationTracker::process_hints`（`src/engine/health_report.rs`）的判据是
`h.severity != "action_required"` —— **无条件升级所有非最高级的提示，包括 `info` 级**
（`info` 在此的语义是「正常 / 无需干预」）。

于是 `generate_action_hints` 里那条 `severity: "info"` 的
「用户反馈正面率 100.0，**系统输出质量良好**」，连续出现 3 次后就被升级成：

```text
[feedback/action_required] 用户反馈正面率 100.0，系统输出质量良好
                           [已连续 449 次出现此警告，级别提升]
suggested_action: 此问题已持续 449 次未解决，请优先处理。
```

——**把「质量良好」当成「需优先处理的故障」**，属语义反转。
同类的还有 `SystemMode::Healthy` 的「系统运行正常，所有子系统健康。无需干预」。

**修法**：只升级 `warning`（`info` = 正常/告知，升级它自相矛盾）；
并修正升级时空 `suggested_action` 产生的前导空格。

**验证**（重编译 sidecar 后实测）：

| 项 | 修前 | 修后 |
|---|---|---|
| 「输出质量良好」的 severity | `action_required` | **`info`** |
| 是否含"级别提升" | 是（449 次） | **否** |
| `badEscalation` / `leadingSpace` | 各 1 条 | **均为 `[]`** |

新增 3 条测试 + mutation 验证（还原旧判据 ⇒ 第 3 轮即变红）。

### 8.2 `system_mode = degraded` 根因 —— **不是环境问题**

初始判断为"需下载 ML 模型"。实测**推翻了它**：

| 证据 | 结果 |
|---|---|
| 模型文件 | **已下载**：`~/.loong-recall/models/BAAI--bge-base-zh`（**390.6 MB**，含 `model.safetensors`） |
| `/api/embedder/status` | 返回 `ready` |
| 健康报告 `encoder.mode` | **仍是 `statistical`**，`degradation_reason = "ML 编码器未启用"` |

**真正根因**：`Cargo.toml` 的 `ml = [...]` **不在 `default` 里**
（`default = ["server"]`），而桌面端 dev 脚本用
`cargo run --no-default-features --features server` —— **主动排除了 ml**（为省内存）。
这是**设计选择**，但导致一个真实的用户问题：

> 用户下载 390MB 模型 → 界面显示「模型已就绪」→ 点「应用」→ 提示「重启服务后生效」
> → **重启后毫无变化，且没有任何地方说明原因**。

**修法**（只改"说法"，不改构建策略）：

1. `checkEmbedderStatus` 增加**运行时**校验：读健康报告的 `encoder.mode`（权威），
   区分「文件就绪」与「**正在生效**」。实测文案已变为：
   `模型文件已就绪，但当前构建未启用语义编码（正在用关键词模式兜底）`
2. `applyEmbedderModel` 的成功提示改为如实说明，不再让用户反复重启试。

> **仍未做**（需用户决策，非代码问题）：是否把 `ml` 编进构建。
> 若需要语义编码，须用 `cargo build --release --features server,ml`；代价是体积与内存上升。

### 8.3 剩余 P2 四项 —— **全部已修**

| # | 问题 | 修法 |
|---|---|---|
| P2-2 | 联想「贡献分」四位小数上屏（`0.0164`，代码注释自认"用户看不懂"） | 从**列表卡片**移除（详情面板的折叠「技术详情」里保留，那是合理位置） |
| P2-3 | 「应用模型」无二次确认即触发**全库重新编码** | 加 `showConfirm`，说明后果（"全部记忆用新模型重新编码，可能较长时间"）；**下载**不加确认（只写磁盘） |
| P2-6 | 「结晶」与「合成」命名混用 | 统一为「**结晶知识**」（快速记一笔下拉 + 类型映射表） |
| P2-7/8 | 记忆详情裸露 UUID；空值字段输出 `--` 与普通字段无差别 | 记忆 ID 移入折叠「技术详情」；「涉及实体」「事件 ID」**无值时整行隐藏**（后者改名「同一次经历」更易懂） |

**顺带修掉一处漏网**：记忆**列表卡片**的时间仍是未格式化 ISO 串
（`2026-09-08T09:40:05.012Z`）—— 实测发现后改用 `formatAnyTime()`，
现为 `2026/9/8 17:40:05`。

---

## 九、最终验证（2026-09-19）

| 项 | 结果 |
|---|---|
| `cargo test --features server` | **790 passed / 0 failed**（786 → +3 升级测试 +1 契约） |
| `cargo clippy --all-targets` | 无 warning / error |
| `check_algorithm_leak.py` | 退出码 **0** |
| `cdp-regression.js` | **发布门禁 PASS**：`action{PASS:103, SKIP_INVISIBLE:4}`、`tab{PASS:10}`、`input{SKIP_PROTECTED:9, PASS:14}`，0 WARN |
| `association-desktop-cdp.js` | **7/7 PASS** |
| `symbolic-layer-desktop-cdp.js` | **7/7 PASS** |

**自动启动共实测 5 次**（每次都是先精确杀掉 3111，再看桌面端是否自己拉起），
**5/5 全部无需手动点击**。

### 9.1 ★本轮踩到的两个操作坑（已记入记忆库）

| # | 坑 | 后果 | 正确做法 |
|---|---|---|---|
| 1 | 用 `Stop-Process -Name lrc-sidecar` 清理开发端 | **连带杀掉稳定版 3099**（MCP 记忆库依赖它）⇒ MCP 连续报 `list tools failed` | **只按 PID 精确杀** |
| 2 | 用 `Start-Process` 拉起 3099 | 进程被 PowerShell 会话回收、**秒退**（端口短暂监听后消失） | 用 `Invoke-CimMethod Win32_Process Create`（RV=0） |
| 3 | 恢复 3099 时随手用了 `desktop/src-tauri/lrc-sidecar.exe`（debug 开发版） | 稳定端短暂跑上了**开发版二进制** | 稳定端应用装版：`%LOCALAPPDATA%\LRC Desktop\lrc-sidecar.exe`（release, 7.8MB） |

> 第 3 条已修正：3099 现运行装版 exe，`/health` 返回 `version: 0.9.8`、4580 条记忆。

### 9.2 构建产物关系（易踩，记下）

| 产物 | 用途 | 桌面端是否使用 |
|---|---|---|
| `code-memory-server.exe` | `cargo build --features server` 的产物 | ❌ **不直接用** |
| `lrc-sidecar.exe` | 桌面端实际加载的 sidecar | ✅ 必须**替换**为此名 |

⇒ 改 Rust 代码后仅编译 `code-memory-server` **不够**，
还要把它复制成 `lrc-sidecar.exe`（覆盖 `G:\rust-target\debug\` 与
`desktop/src-tauri/` 两处），桌面端才会加载新逻辑。
本轮 `action_hints` 修复初次验证"没生效"就是这个原因。


---

## 十、启用 ML 后的 explore 超时根因（2026-09-19）

### 10.1 触发

用户要求「启用 ML」（`--features server,ml`，模型走界面下载通道、不进安装包）。
开发端启用后重跑 CDP，**联想套件由 7/7 掉到 3/7**：B（诚实空态）、C（语义旁路）
两条用例稳定报 `explore_timeout` 503。

### 10.2 决定性对照实验（同一份代码 + 同一份数据）

这是定位过程中最有价值的一步——**只切换构建模式，其他全部相同**：

| 查询 | RELEASE | DEBUG |
|---|---|---|
| 今晚吃什么 | 1.95s ✓ nodes=19 | 10.75s ✓ nodes=19 |
| 量子物理是什么 | 8.83s ✓ nodes=0 | **15.00s ✗ 503** |
| 我以前记过什么重要日子？ | 4.82s ✓ nodes=29 | **15.05s ✗ 503** |

⇒ **不是代码缺陷，也不是本轮改动引入**：同一份源码 release 全通过。
ML 句向量前向在未优化构建下膨胀 2~4 倍。

### 10.3 根因：dev profile 对**依赖**也用 opt-level=0

联想探索在词面零命中时走语义旁路，对候选池做 ML 句向量前向。
`candle` 是**依赖 crate**，而 Cargo 默认对 dev profile 的依赖同样用
`opt-level = 0` —— 张量运算未优化，把原本约 9s 的请求推过 15s 外墙。

后果：`v1_api.rs` 设计好的「优雅收敛」全部不可达 ——
内部 10s 预算本应返回 `interrupted=true`（部分结果）或 `weak_match=true`（诚实空态），
实际却由 15s 外墙先触发、返回 503。

### 10.4 修复（两处）

**① `Cargo.toml`：dev profile 下只优化 ML 依赖（不是全部依赖）**

```toml
# ML 主链路
[profile.dev.package.candle-core]        opt-level = 3
[profile.dev.package.candle-nn]          opt-level = 3
[profile.dev.package.candle-transformers] opt-level = 3
[profile.dev.package.tokenizers]         opt-level = 3
[profile.dev.package.hf-hub]             opt-level = 3
# candle-core 的**矩阵乘法后端**（真正的张量热点，漏掉则优化无效）
[profile.dev.package.gemm]               opt-level = 3
[profile.dev.package.gemm-common]        opt-level = 3
[profile.dev.package.gemm-f32]           opt-level = 3
[profile.dev.package.gemm-f16]           opt-level = 3
[profile.dev.package.gemm-f64]           opt-level = 3
[profile.dev.package.gemm-c32]           opt-level = 3
[profile.dev.package.gemm-c64]           opt-level = 3
[profile.dev.package.half]               opt-level = 3
```

**为什么不写 `[profile.dev.package."*"]`（优化全部依赖）**：
那会让 **CI** 的 dev profile 作业（`clippy` / `test` / `check`）把
tokio / axum / serde 等约 **200 个依赖**全部重编为优化版 ——
而线上仓库是在线编译的，这会显著拉长 `ci.yml` 的
`clippy`(30min) / `test`(45min) 作业，有撞上 `timeout-minutes` 的风险。
实测表明瓶颈**只在 ML 张量运算**，故收窄为上述 13 个包：
线上 CI 只需多编这 13 个包。

> **`gemm` 系列为何必须在列**：它不在 `candle-*` 命名空间下，
> 但 `candle-core` 直接依赖它做矩阵乘法 —— 即前向传播真正的热点。
> 第一版只列了 5 个 `candle-*` 包，实测 `量子物理是什么` **仍 503**；
> 补入 `gemm*` + `half` 后才通过（见 10.5 对比）。

**② `static/app.js`：前端超时 15s → 30s**

原先前端 `fetchWithTimeout` 也是 15000ms，与后端外墙**完全相等** ⇒
前端 abort 与后端超时同时触发，前端总是先报"请求超时"，
使后端优雅收敛不可达。现形成单调阶梯：

```
10s(后端内部预算)  <  15s(后端外墙)  <  30s(前端兜底)
```

### 10.5 实测验证（修复后）

**① 三条查询（debug 构建，同一份数据）**：

| 查询 | 修复前 | 仅优化 5 个 candle 包 | **收窄为 13 包（含 gemm）** |
|---|---|---|---|
| 今晚吃什么 | 10.75s ✓ | 10.48s ✓ | **9.60s ✓** |
| 量子物理是什么 | **15.00s ✗ 503** | 仍 **503** | **9.91s ✓ nodes=0** |
| 我以前记过什么重要日子？ | **15.05s ✗ 503** | 11.60s ✓ | **12.61s ✓ nodes=19** |

⇒ 补入 `gemm*` 是关键：`量子物理是什么` 由「仍 503」变为「9.91s 通过」。

**② CDP 三套件（ML 已启用）**：

| 套件 | 修复前 | 修复后 |
|---|---|---|
| `cdp-regression.js` | PASS | **PASS**（action 103 PASS / 0 WARN） |
| `association-desktop-cdp.js` | **3/7** | **7/7 PASS** |
| `symbolic-layer-desktop-cdp.js` | 2/3 | **7/7 PASS** |

**③ 其他门禁**：`cargo test --features server` **790 passed / 0 failed**；
`check_algorithm_leak.py` 退出码 **0**。

### 10.6 遗留与边界（如实记录）

- 验证 `Cargo.toml` 改动需**重编 ML 依赖**（一次性，耗时约 15 分钟）。
- 本轮编译期间为守住 CPU 约束，曾用 `-j 1` 串行；实测单核 `-j 1` 仍会
  在 LLVM codegen 阶段顶满 CPU，最终经用户同意临时放宽上限后完成。
- **产品发布走 release**（`release.yml` 用 `--release --features server,ml`），
  release 下三条查询本就全部通过（实测 1.82s / 4.90s / 11.71s），
  本修复的价值在于**让 debug 开发环境与 release 行为一致**，
  避免开发期误判"功能坏了"。

### 10.7 本轮排查的方法论教训（留档）

1. **必须做"仅切换单一变量"的对照**：早期把根因归为"ML 冷启动需 180s"、
   "端口串行扫描"，两者都被"同一份数据换 release 二进制"的对照推翻。
   若一开始就做构建模式对照，可省下大量试错。
2. **探针端口必须落在服务自适应范围内**（`3099..3198`）：
   我多次用 3160/3300/3441 等端口，导致 sidecar 绑定失败、探针"未就绪"，
   被我误判为"代码卡死"。
3. **不能用符号名判断 release 二进制是否含某次改动**：
   `[profile.release]` 有 `strip = true`，符号会被剥离，据此得出的结论不可靠。
4. **失败修复要立刻回退**：曾把后端外墙放宽到 25s，结果三条查询**全部**由
   "约 10s 返回"变为"25s 才失败"——因为拖长的是真实计算而非等待，
   放宽外墙只把失败点后移。已回退。
