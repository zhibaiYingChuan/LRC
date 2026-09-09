# 道体状态机接入 LRC 记忆系统 · 推进计划

> 版本：v1.7（2026-09-09）
> 状态：P0-P4 基础设施已完成；P3.5 已完成（本模式 NO-GO，见第八节）；**P6 完全体闭环已立项（判据已锁定，见第九节）**；P5 待执行
> 最近进度：P0 50304ef/26dbb66；P1 faa4cae；P2 fe4342d；P3 64f93b9；P4 c9921cb；P3.5 结果 acc9bb7；均未推送
> 前置文档：`daoti/PREREG_NAV.md`、`daoti/PREREG_FAIR_STATE_MACHINE.md`
> 结果文档：`daoti/PREREG_ASSOC_NAV.md`（判据 + 结果回写 + 方法论批判）
> 立项文档：`daoti/PREREG_CLOSED_LOOP.md`（P6 完全体闭环预注册，判据先于实现锁定）
> 许可约束：道体引擎与卦词典属研究资产（DaoTi Research License v1.0），产品侧只消费信号、不内置引擎

---

## 一、背景与本轮代码考古结论（2026-09-08 事实核对）

### 1.1 已确认的事实

| # | 事实 | 证据位置 |
|---|------|---------|
| 1 | LRC 内置记忆联想状态机是**事件驱动**的（激活→衰减→转移），两次交互之间无自发演化 | `src/engine/memory_state_machine.rs`（`activate`/`transition`/`DECAY_PER_STEP=0.85`） |
| 2 | 原版道体 = V23 推理引擎（五行生克符号因果推演 + coherence 动态阈值 + 不确定判据），**未**以 Rust 重写形式进入 LRC | `daoti/inference_engine_v23.py`（167KB）；Rust 侧文件头为 "Apache 2.0 / LRC 内置记忆联想状态机" |
| 3 | 道体血统组件 **DaoRegulator（道同构度调节器）代码完整、已接线 MemoryStore、调节动作真实生效（可改写运行时衰减率），但运行时无人调用**（唯一调用点在测试） | `src/memory_store.rs` L303/L1830/L1913；调用点仅 L6320（测试） |
| 4 | **导航层消费端已存在且已接入**：`NavigationSignal` 解析 + 多视图 RRF 融合（6 个单测），挂接在 recall 的 `navigation` 参数 | `src/engine/navigation.rs`；`src/server.rs` L1035-1071 |
| 5 | **回归校验层已存在**：`regression_recheck` 三证据检验（原查询词面命中/联想桥强关联/标签共鸣），已在联想链路运转——但它是**词面级**，不是道体状态级 | `src/engine/memory_state_machine.rs`；`src/memory_store.rs` L3048/L3069 |
| 6 | 导航信号**生产者缺席**：预注册契约要求信号由 daoti 研究资产推演产生后注入，产品侧不计算、不内置道体引擎 | `src/engine/navigation.rs` 文件头生产者契约 |
| 7 | 预注册实验结论：后处理重排 NO-GO（+0.7~1.7pp）；纯方向导航 +2.0~2.3pp（P=96.3~98.3%）未达 8pp 门槛；**CHAIN1 一跳种子扩散是全谱最优且已落地** | `daoti/PREREG_NAV.md` 结论节 |
| 8 | `memory_state_machine.rs` 与 `navigation.rs` **从未提交 git**（untracked），而 `mod.rs` 对它们的引用已提交——CI 从 git 克隆构建会因缺文件编译失败 | `git status --short src/engine/` |

### 1.2 对"道体缺席"评估的修正

用户评估中"导航层不存在、回归验证层也不存在"需修正为：**两层的消费端产品代码均已存在并接线，真正缺席的是生产者与运行时心跳**。具体三层缺口：

1. **生产者缺口**：没有任何进程在持续运行道体推演、产出导航信号
2. **心跳缺口**：DaoRegulator 的 `regulate()` 无运行时触发——"心跳机装好未通电"
3. **消费缺口**：联想中心 `explore_pure` 路径未消费导航信号（导航目前只挂在 MCP recall 路径）

---

## 二、目标架构：道体常驻推演循环（六钥匙结论）

### 2.1 方案比选

| 方案 | 形态 | 活性强度 | 部署成本 | License 合规 |
|------|------|---------|---------|-------------|
| **A. 道体常驻服务（选定，用户明确要求"让道体持续运行"）** | 桌面端拉起 daoti_daemon 常驻进程（Python 或 PyInstaller 打包 exe），进程持续存活、推演状态持续演化 | 强（真·持续推演） | 高（用户环境依赖） | ✅ 引擎独立进程 |
| B′. 会话级推演循环（降级模式） | daoti_daemon 不可达时，LRC 联想请求触发多步往返推演，会话内状态活着、结束持久化 | 中强 | 低 | ✅ |
| C. 信号预生成缓存 | 离线/低频推演，检索消费缓存 | 弱（无实时推演） | 低 | ✅ |
| D. Rust 重写引擎内嵌 | 把 V23 移植进 sidecar | 强 | 极高 | ❌ 违反导航层契约（产品侧不内置） |

**选 A 为骨架、B′ 为道体服务不可用时的降级、C 为再兜底**。理由：

- 用户明确要求："真正把道体 V23 集成到 LRC 的状态机循环中（高成本）——让道体持续运行，产生状态信号，驱动 LRC 的检索方向"
- 常驻进程的推演状态**跨查询持续存活且随时间演化**（衰减/收敛/再推演），满足状态机三特征的定义性要求：持续运行的内部状态、状态随输入和时间演化、状态决定系统对输入的响应方式
- PyInstaller 打包 exe 免除用户 Python 环境依赖（daoti 资产侧打包，仍属研究资产，License 合规）
- V23 的"不确定步触发检索"节奏（信息熵 + margin、连续 3 步 target_gua 不变收敛）在常驻进程内天然运转
- daoti_daemon 不可达时逐级降级：B′ 会话循环 → C 缓存 → 无信号基线（行为与现版本逐字节一致，navigation.rs 契约已保证）

### 2.2 目标数据流

```
用户在联想中心输入查询
        ↓
LRC sidecar /v1/associations/explore
        ↓
[步骤1] 调 daoti_daemon（localhost:3222）/deduce
        ← 携带查询 + 会话 id；道体在常驻推演状态上继续演算，产出导航信号
           {palaces, probes, version, state}
        ↓
[步骤2] LRC 接收并执行：带导航信号做多视图检索（navigated_deep_recall，已就绪）
        + CHAIN1 一跳种子扩散（现状保留）
        ↓
[步骤3] 检索结果回归道体：候选记忆回传 /reflect，道体更新推演状态
        + 回归校验：词面三证据（现状）+ 道体状态级 recheck（P4 升级）
        ↓
[步骤4] 循环步骤1-3 直到收敛（连续 3 步 target_gua 不变）或预算耗尽（≤2 轮，≤10s）
        ↓
[步骤5] 道体继续推演（不随查询结束而停止）：
        状态按节拍自检演化（衰减/收敛推进/再推演），周期性落盘
        ~/.loong-recall/daoti/session_state.json
        + 确认写回 LRC 状态机（现状机制保留）
        ↓
常驻进程持续运行：状态跨查询存活、随时间演化 → 下次查询从最新状态继续推演
```

**接口重设计三段契约**（对应用户要求"道体输出导航信号 → LRC 接收并执行 → 检索结果回归道体"）：

| 段 | 端点 | 方向 | 内容 |
|----|------|------|------|
| 输出导航信号 | `POST /deduce` | LRC → 道体 | 查询 + 会话 id → NavigationSignal JSON（palaces/probes/version） |
| LRC 接收并执行 | （已有）`navigated_deep_recall` | LRC 内部 | 多视图检索 + RRF 融合，改变候选集 |
| 结果回归道体 | `POST /reflect` | LRC → 道体 | 检索结果摘要 → 道体修正推演方向、更新状态 |

---

## 三、分阶段推进计划

> 交付纪律：每阶段完成后必须走四角色闭环（韧性审计 → 评估 → 修复 → 回归循环到零问题）才可进入下一阶段。所有修改逐文件逐步执行，禁止批量改动。

### P0 仓库卫生与止血（已完成，2026-09-08）

| 任务 | 内容 | 验收标准 |
|------|------|---------|
| P0.1 | 先跑全量测试确认未提交代码可编译：`cargo test --features server --lib` | ✅ 605 passed / 0 failed |
| P0.2 | 提交 untracked 文件：`src/engine/memory_state_machine.rs`、`src/engine/navigation.rs` | ✅ 随 50304ef 提交，不再 untracked |
| P0.3 | 提交 10 个已修改引擎文件（audit_trail/luoshu_encoder_ml/rrf 等，均属联想精确度修复） | ✅ 随 50304ef 提交 |
| P0.4 | 泄露检测：按 `docs/PUSH_STANDARD.md` 规则扫描后推送 | 泄露检测通过；**尚未推送**（用户选择仅本地提交） |

**P0 执行发现（重要，已记录）**：
- 提交钩子第 5 项引用的 `scripts/check_algorithm_leak.py` **在 3d7acce 被误删**，导致所有提交被钩子拦截（找不到脚本 → exit 1）。已从 git 历史恢复该脚本，并为 `daoti_preview_*` 字段、`preview_gua/bagua/version` 局部变量、`LRC_DAOTI_NAVIGATE` 门控、「道体再次校验」用户可见文案等**契约性引用**补充最小白名单（排除规则仅限字段名/UI 文案/变量名，未放宽对「洛书/几何坐标/剪枝」等真算法关键词的检测）。
- 泄露检测现通过：`python scripts/check_algorithm_leak.py` → "通过: 公开层文件无核心算法泄露"。

**风险说明**：P0 不做，下次打 tag 触发 CI 必然编译失败（mod.rs 引用了不存在的文件）。—— 此风险已解除（50304ef 已提交全部引用文件）。

### P1 点亮调节器心跳（已完成，2026-09-08，提交 faa4cae）

| 任务 | 内容 | 验收标准 |
|------|------|---------|
| P1.1 | sidecar 后台周期任务（tokio interval，建议 30 分钟）调用 `store.regulate()` | ✅ 新增 `regulator_heartbeat_loop`，周期可用 `LRC_REGULATE_INTERVAL_MIN` 覆盖（默认 30 分钟）；`try_lock` 锁忙跳过，`spawn_blocking` 避免阻塞 HTTP worker |
| P1.2 | 调节动作落地审计：`AdjustDecayRate`/`AdjustRetrievalWeights` 等执行结果写入 audit-trail | ✅ 已有机制复用（`regulate()` 内 `record_audit`），并新增 `RegulatorHeartbeat::record` 统一记录心跳状态 |
| P1.3 | 前端系统状态卡展示"调节器心跳"状态（最近调节时间/最近动作），接入 TAB_LOADERS | ✅ 新增 `renderRegulatorHeartbeat()`（static/app.js）+ 心跳展示区（index.html 系统健康评分卡内） |
| P1.4 | 单测：模拟阴阳失衡 → regulate 返回动作 → 衰减率真实变更 → 审计事件存在 | ✅ 新增 3 个契约测试（心跳记录/NoAction 可观测/审计可查询） |

**P1 执行记录（重要）**：
- **心跳状态数据结构**：`RegulatorHeartbeat { last_run_ms, last_action, run_count, last_reason }`，位于 `src/memory_store.rs`，作为 `MemoryStore` 字段，随 `regulate()` 自动更新。
- **运行时验证通过**：用 `LRC_REGULATE_INTERVAL_MIN=1` + 隔离数据目录启动 sidecar 冒烟，`GET /v1/health/system` 返回 `{"last_action":"adjust_synthesis_threshold → 2","run_count":1,...}` —— 调节器心跳真实执行了调节动作（合成最小聚类 3→2）。
- **License 合规**：server.rs 公开层注释不出现受保护术语（避免泄露检测告警），仅用 "DaoRegulator（自适应调节的活性保证）" 描述。

**验收标准**：✅ sidecar 运行时 `/v1/health/system` 可见 `regulator_heartbeat` 心跳状态；防振荡机制（冷却/冻结）在 dao_regulator 已有单测覆盖。

**降级安全**：调节器已内置防振荡 v2.0（历史追踪/冲突仲裁/自适应步长/冷却）与冻结机制，异常时 NoAction，不改变检索行为。

### P2 导航信号生产者：daoti_daemon 常驻进程（核心已完成，2026-09-08，提交 fe4342d）

| 任务 | 内容 | 涉及文件 | 状态 |
|------|------|---------|------|
| P2.1 | 新建 `daoti/daoti_daemon.py`：常驻 HTTP 服务（纯 stdlib，对齐 lrc_bridge 风格），暴露 `/health`、`/deduce`（查询+会话id → NavigationSignal JSON）、`/reflect`（检索结果回传 → 更新推演状态）、`/state` | `daoti/daoti_daemon.py` | ✅ 完成 |
| P2.2 | 常驻推演节拍器：空闲期按固定周期推进状态演化，满足两次交互之间状态持续变化 | 同上（`advance_beat` 每 60s） | ✅ 完成 |
| P2.3 | 推演状态持久化：`~/.loong-recall/daoti/session_state.json`（跨查询存活，原子写入，重启恢复） | 同上（`atomic_write_json`） | ✅ 完成（重启恢复实测通过） |
| P2.4 | 编码器走四级校准路线第④步成果：GuaLexiconEncoder（纯 stdlib 词典计数，已实现） | `daoti/calibrate_lrc_mapping.py` | ✅ 完成（V23 优先，lexicon 兜底） |
| P2.5 | LRC 侧降级客户端：`explore`/`recall` 调 daoti_daemon 失败（超时 2s/不可达）→ 信号缺省 → 基线行为 | `src/server.rs`（`fetch_daoti_navigation` + handle_recall 接入） | ✅ 完成（2 个契约测试：在线解析 / 离线降级） |
| P2.6 | 桌面端以 sidecar 管理模式拉起/回收 `daoti_daemon`，并做健康检查、PID 校验与异常降级 | `desktop/src-tauri/` | ⏳ 发布项待执行 |
| P2.7 | PyInstaller 打包 `daoti_daemon.exe`，免用户 Python 环境依赖 | `daoti/` 打包脚本 | ⏳ 发布项待执行 |
| P2.8 | 走完四级校准：④ 映射校验（已有脚本）→ ③ 链路验证 → ① 权重标定 → ② 阈值标定 | `daoti/` 校准脚本 | ⏳ 随 P3 实验执行 |

**P2 执行记录（重要）**：
- **V23 真实引擎驱动**：发现 `trigram_emergence_v2.pt`（325MB）实际存在于 `daoti/`（此前 Test-Path 误判缺失）。daemon 优先加载 V23 引擎（2.2s 加载 / 0.5s 推演），`GuaLexiconEncoder` 作为 checkpoint 不可用时的降级后端。`/health` 与 `/state` 如实暴露 `engine: v23`，不编造信号来源。
- **导航信号契约对齐**：`/deduce` 返回 `{palaces, probes, version}`，与 [navigation.rs](file:///g:/code-memory/src/engine/navigation.rs) 的 `NavigationSignal` 解析完全一致（palaces 由 V23 推演链每步 target_gua 映射卦宫去重取前 4，probes 为宫义探测词）。
- **端到端验证通过**：真实请求 `/deduce 今晚吃什么` → `palaces:["艮宫","震宫"]`、`converged:true`、`active_gua:"随"/震宫`；`/reflect` 情绪记忆 → 坎宫修正；连续 deduce 状态演进（step 递增、stable_steps 累积）；**重启后 step=2 保留**（跨进程持久化）。
- **LRC 侧接入**：`handle_recall` 在 `LRC_DAOTI_NAVIGATE=1` 且外部无 navigation 参数时，自动向 daemon 拉取信号；daemon 不可达（2s 超时）→ None → 无导航基线，行为与既有版本逐字节一致。
- **License 合规**：daemon 位于研究资产侧（DaoTi License），LRC 仅消费 JSON 信号；泄露检测白名单仅限进程名/函数名/环境变量/协议版本标识，未放宽真算法关键词检测。

**验收标准**：✅ `/health` 返回 ok、连续 deduce 状态演进、`/state` 可见、重启恢复——全部实测通过。

**端口约定**：daoti_daemon 固定 `127.0.0.1:3222`（避开 sidecar 3099/3111、桌面 dev 1420），环境变量 `DAOTI_SERVICE_URL` 可覆盖。

### P3 联想中心接入导航 + 预注册实验（判据+接入已完成，2026-09-08，提交 64f93b9；实验待执行）

| 任务 | 内容 | 涉及文件 | 状态 |
|------|------|---------|------|
| P3.1 | **预注册判据文档先行**（见第四节），写入 `daoti/PREREG_ASSOC_NAV.md`，数据产生前锁定 | `daoti/PREREG_ASSOC_NAV.md` | ✅ 完成（诚实引用 PREREG_NAV 先验：召回增益 NO-GO，G5 体验护栏为决定性判据） |
| P3.2 | `/v1/associations/explore` 增加 `navigation` 消费：有信号 → `navigated_deep_recall` 产生的候选进入根节点门禁双通路（词面+语义旁路）；无信号 → 现状 `explore_pure` 逐字节一致 | `src/v1_api.rs`（`run_association_explore` 增 `navigation` 参数 + root 候选池切换） | ✅ 完成（2 个契约测试） |
| P3.3 | 导航候选仍受 CodeContext 过滤与泛指 bigram 过滤约束（复用现有防线） | `src/v1_api.rs` | ✅ 完成（导航候选走同一门禁路径，CodeContext 过滤测试验证） |
| P3.4 | 时间预算：daoti 往返 + 多视图检索合计 ≤10s，超时收敛返回部分结果 | `src/v1_api.rs` | ✅ 完成（复用 `ASSOCIATION_EXPLORE_TIME_BUDGET` 10s + fetch 2s 超时） |
| P3.5 | 实验执行：按预注册语料（100 记忆 + 30 查询）跑三臂对照 | `daoti/` 评测脚本 | ✅ 完成（2026-09-09：**本模式 NO-GO**，结果与方法论批判见 `PREREG_ASSOC_NAV.md` 第五节与本文第八节） |

**P3 执行记录（重要）**：
- **预注册判据**（`daoti/PREREG_ASSOC_NAV.md`）：G1-G5 判据在实验数据产生前写死。诚实声明 `PREREG_NAV.md` 先验（导航召回增益 NO-GO +1~2pp），将 G5 体验护栏定位为决定性判据，G1/G2 为强门槛，并预设"判定矩阵"（部分有效/NO-GO 的诚实收场路径）。
- **修复真实缺陷**：`navigated_deep_recall` 返回 `Some(空候选)` 时，explore 原逻辑用空候选池导致 root=None（导航"变了候选但捞空"产生空起点）。已修复：导航候选为空时回退单查询基线召回。
- **导航不豁免防线**：P3.2-1 契约测试验证导航信号下 CodeContext 代码块仍被过滤、生活记忆经词面门禁上位；P3.2-2 验证空 palaces（无有效方向）时诚实回退基线。
- **License 合规**：`fetch_daoti_navigation` 提为 `pub(crate)` 供联想中心消费，仍只消费 JSON 信号、不内置引擎；泄露检测通过。

**预注册判据**（G1-G5 详见 `daoti/PREREG_ASSOC_NAV.md`，判据在实验前锁定）：

- **G1 互补性**：基线 top1 错误子集上，导航臂把 hop≥2 同链记忆拉进 top-5 比例 ≥30%
- **G2 净增量**：导航臂 hop3-5 recall@10 − 基线 ≥8pp，且 hop1-2 recall@3 下降 ≤2pp（强门槛，历史 NO-GO 先验）
- **G3 显著性**：bootstrap 2000 次 P(G2 增益 ≥8pp) ≥0.95
- **G4 噪声护栏**：导航臂 top10 异链噪声率不高于基线 +3pp
- **G5 体验护栏（决定性）**：生活类查询 CDP 实测无代码噪声回潮；"量子物理是什么"仍为诚实空态

**No-go 收场**（预注册纪律）：导航信号退回"用户确认联想时作为元数据记录"（同 PREREG_FAIR 的收场规则），联想中心保持 CHAIN1+词面校验现状——该现状已通过 7 项 CDP 专项与 88 交互口回归门禁。
**实际结果（2026-09-09）**：P3.5 判定为本模式 NO-GO/无召回增益（G1 5%、G2 −4.7pp、G3 P=0；G4/G5 通过）——收场执行：导航保持默认关闭（`LRC_DAOTI_NAVIGATE` 未设置不触发，现状即合规），接口保留为架构能力，P4 运行时接线冻结。详见第八节。

### P4 回归验证层升级：道体状态级 recheck（基础设施已完成，2026-09-08，提交 c9921cb；运行时接线已冻结）

| 任务 | 内容 | 涉及文件 | 状态 |
|------|------|---------|------|
| P4.1 | `regression_recheck` 增加第四证据位：**原意图回应度**——daemon `/reflect` 返回的意图相关分（≥阈值 0.5 判"回应原意图"） | `src/engine/memory_state_machine.rs`（`regression_recheck` 增 `intent_score: Option<f32>` + `ASSOCIATION_INTENT_RESPONSE_THRESHOLD=0.5`） | ✅ 基础设施完成（纯函数第四证据位 + 4 个契约测试）；⏸️ 运行时接线已冻结（P3.5 实验未通过，按"实验通过后叠加"纪律不启用） |
| P4.2 | daemon 离线时第四证据缺省跳过（三证据照常），行为回退现状 | 同上（`intent_score=None` 时跳过） | ✅ 完成（契约测试验证与三证据现状逐字节一致） |
| P4.3 | 前端证据标签映射：新增"和你的本意对得上"白话文案 | `static/app.js` | ✅ 完成（`'原意图回应度': '和你的本意对得上'`） |

**P4 执行记录（重要）**：
- `regression_recheck(original_overlap, bridge_hits, tag_hits, intent_score: Option<f32>)`——第四证据位为**纯函数扩展**：
  - `Some(score) >= 0.5` → 保留，证据"原意图回应度"（词面零重叠但语义回应原意图）
  - `Some(score) < 0.5` → 第四证据不通过，回退三证据裁决
  - `None`（daemon 离线）→ 缺省跳过，与现状逐字节一致
- 4 个契约测试：高分保留 / 低分回退 / 词面证据优先 / 离线缺省跳过。
- **运行时接线已冻结（2026-09-09）**：P3.5 实验判定本模式 NO-GO（见第八节），按"实验通过后叠加"纪律，daemon `/reflect` 逐候选意图分注入 explore 校验链路的工作暂停。第四证据位代码保留（纯函数 + 契约测试），不产生运行时行为变化（`intent_score=None` 路径即现状）。

**验收标准**：四证据单测 ✅（4 契约测试）+ 道体离线回退单测 ✅；CDP 实测证据标签渲染随运行时接线冻结而搁置。

### P5 打包发布

| 任务 | 内容 |
|------|------|
| P5.1 | 版本号 10 处同步 + `cargo check` 更新 Cargo.lock（规则见 project_memory） |
| P5.2 | MSI 重打包：当前 MSI 内嵌 9/7 版 sidecar，本次必须包含道体心跳 + 导航接入的新 sidecar |
| P5.3 | CHANGELOG 记录：调节器心跳上线、导航信号接入（含实验结论，无论 go/no-go 如实记录） |
| P5.4 | release.yml 构建 + 产物版本校验（`--version` 输出 = tag 版本，不一致 FAIL） |
| P5.5 | 桌面端 CDP 全量回归（83+ 交互口）+ 联想专项 7 场景 |

---

## 四、验证命令（PowerShell 5.1 兼容，已经 PowerShell 专家规则审核）

> 执行前须知：含中文的 .ps1 脚本必须以 **UTF-8 with BOM** 保存；以下命令可直接逐条在控制台执行。

```powershell
# ---- 编码自保护块（每次会话执行一次）----
if ($PSVersionTable.PSVersion.Major -lt 6) { chcp 65001 > $null }
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()

# ---- P0: 仓库卫生检查（已完成）----
git status --short src/engine/
cargo test --features server --lib

# ---- P1: sidecar 健康与调节器心跳审计 ----
try {
    $health = Invoke-RestMethod -Uri 'http://127.0.0.1:3111/health' -TimeoutSec 5
    "sidecar 在线: $($health | ConvertTo-Json -Compress)"
} catch {
    Write-Warning "sidecar 未响应: $($_.Exception.Message)"
}

# ---- P2: 道体服务探活（离线=降级基线，属预期，不算失败）----
try {
    $dao = Invoke-RestMethod -Uri 'http://127.0.0.1:3222/health' -TimeoutSec 3
    "道体服务在线: $($dao | ConvertTo-Json -Compress)"
} catch {
    "道体服务离线，LRC 将以无导航基线运行（降级设计，非故障）"
}

# ---- P2: 四级校准（映射校验示例）----
$env:LRC_BASE_URL = 'http://127.0.0.1:3111'
python daoti\calibrate_lrc_mapping.py

# ---- P5: 编译产物版本校验（PS5.1 禁止直接 & exe --version，stdout 会被吞）----
$versionFile = Join-Path $env:TEMP 'lrc-sidecar-version.txt'
Start-Process -FilePath 'G:\rust-target\release\code-memory-server.exe' `
    -ArgumentList '--version' `
    -RedirectStandardOutput $versionFile `
    -NoNewWindow -Wait
$builtVersion = (Get-Content -LiteralPath $versionFile -Encoding UTF8 -Raw).Trim()
"编译产物版本: $builtVersion"
Remove-Item -LiteralPath $versionFile -ErrorAction SilentlyContinue
```

---

## 五、风险与回退

| 风险 | 缓解 | 回退 |
|------|------|------|
| 导航信号与 explore_pure 教训冲突（活性偏置曾污染结果） | 导航是**方向性扩展**（改变候选集），非活性偏置（改变排序权重）；且预注册 G4/G5 设噪声与体验护栏 | 判据 no-go → 信号退回元数据记录，联想中心回滚现状 |
| daoti_daemon 增加部署复杂度 | 纯 stdlib、可选组件、离线降级基线逐字节一致 | 桌面端默认不启用，仅开发环境开启 |
| 调节器调节过度（衰减率震荡） | 防振荡 v2.0 + 冻结机制 + 审计可观测 | 动作全部可审计回放，异常时冻结 |
| 跨语言调用延迟（推演往返） | 每轮预算 ≤10s 硬时限；超时收敛返回部分结果（现状机制） | 超时即无导航基线 |
| License 合规 | 引擎/词典全程留在 daoti/ 研究资产侧，产品侧仅消费 JSON 信号（navigation.rs 契约） | 契约已内建版本协商（source_version 不识别即忽略） |

## 六、与既有交付纪律的关系

- 每阶段进入下一阶段前：interaction-resilience-auditor（五层交互 L1-L5）+ hcse-resilience-validator 双审计，覆盖桌面端与 Web 双模式
- 修复一律六钥匙辅助、全局考虑、禁止挤牙膏
- 未修复的 P0/P1 问题必须在 CHANGELOG 如实记录
- 所有进度与结论回写本档与 LRC 记忆库

## 七、里程碑顺序总览

```
P0 止血（已完成：50304ef 本地提交，泄露检测通过，未推送）  ✅
P1 调节器心跳（已完成：faa4cae 本地提交 + 运行时验证通过）  ✅
P2 daoti_daemon 常驻进程（核心完成：fe4342d；P2.6-2.8 发布项待执行）  ✅
P3 联想中心接入导航（判据+接入完成：64f93b9；P3.5 实验 NO-GO，2026-09-09）  ✅
P4 第四证据位（基础设施完成：c9921cb；运行时接线已冻结）  ✅
P6 完全体闭环（立项 + 判据锁定：acc9bb7 后续提交；CL1-CL4 实现待执行）  ← 当前
P5 打包发布（MSI 重打包 + 全量回归；闭环判定 GO 前不含默认开启行为）  待执行
```

---

## 八、P3.5 实验结果与方法论批判（2026-09-09）

### 8.1 实验结果摘要

三臂对照（A 基线 / N 导航，公平语料 100 记忆 + 30 查询，daemon v23 全程在线）：

| 判据 | 结果 | 判定 |
|---|---|---|
| G1 互补性 | 5%（门槛 ≥30%） | ❌ |
| G2 净增量 | hop3-5 R@10 −4.7pp（门槛 ≥+8pp）；hop1-2 R@3 −8.9pp（护栏 ≤2pp） | ❌ |
| G3 显著性 | P(增益≥8pp)=0.0000，P(增益≤0pp)=1.0000 | ❌ |
| G4 噪声护栏 | −5.3pp（护栏 ≤+3pp） | ✅ |
| G5 体验护栏 | 无代码噪声回潮，诚实空态保持 | ✅ |

**判定**：导航信号在"查询时快照注入"模式下无召回增益且显著负向，无体验伤害。
收场：导航保持默认关闭（现状合规），接口保留为架构能力，P4 运行时接线冻结。
意外收获：第 1 次运行 daemon 中途死亡，N 臂与 A 臂逐字节一致——**离线降级契约
获得实证**。完整数据见 `daoti/PREREG_ASSOC_NAV.md` 第五节。

### 8.2 方法论批判：静态快照 vs 持续状态机（用户提出，代码事实核对后记录）

**用户核心论点**：本实验测错了方向。道体的设计是**持续运行的状态机**——状态持续
演化、状态影响响应方式、响应又改变状态、持续循环。而实验把道体当作查询时临时
调用的函数：跑一次推演 → 输出一个信号 → 调整检索 → 结束。在这种模式下道体没有
机会建立历史上下文，它的输出只是一次快照，与用 BGE 算一个语义向量没有本质区别。
用低维信号（64 类卦象）覆盖高维信号（768 维 BGE）的结果当然难有增益——但这否定
的是"静态快照"用法，不是"持续演化"能力。这好比用瞬时爆发力的测试结果否定运动员
的持续耐力价值——被测能力与设计价值不是同一个能力。

**代码事实核对（诚实记录，含修正）**：

| 论点 | 核对结果 |
|---|---|
| 道体状态未参与信号计算 | ✅ 确认为真：`V23Engine.deduce(query, state)` 接收 `state` 却从未使用（daoti_daemon.py L151-158），推演链完全由当前查询驱动，daemon 状态只是记账 |
| Hebbian 在线学习未使用 | ⚠️ 需修正：V23 v23.2 在线 Hebbian 默认开启，收敛后更新推理用原型库（`dim_manager.gua_protos_base`），常驻引擎下跨查询缓慢演化；但学习率极小（0.005），不改变"信号由当前查询驱动"的本质 |
| 探索进程（WeightSpaceExplorer）未运行 | ✅ 确认为真：组件存在于 daoti_lrc_runtime.py，daemon 未接入 |
| 完整模块池（23 模块）未全参与 | ✅ 确认为真：daemon 仅封装 V23 引擎 + 词典编码器 |
| 检索结果回归道体的闭环未运行 | ✅ 确认为真：/reflect 端点存在，但 explore 运行时路径从未调用（P4 接线冻结），实验期间闭环是断开的——道体只收到 /deduce，从未收到 /reflect |

**结论修正**：P3.5 的 NO-GO 只否证"查询时静态快照信号注入检索"这一用法，**不否证
"道体作为持续状态机闭环驱动检索"的假说**。后者（状态参与计算 + reflect 闭环 +
探索进程 + 完整模块池）从未被任何实验检验。

### 8.3 后续路径（已决策：立项，2026-09-09）

若要检验"持续状态机"假说，需另立预注册协议，接入方式必须满足四个前提：
1. daemon 状态真实参与 deduce 计算（当前 `state` 参数被忽略）；
2. `/reflect` 闭环接入运行时：检索结果回传 → 状态更新 → 影响下次 deduce；
3. Hebbian 演化与探索进程持续运行，信号携带历史累积；
4. 判据设计转向"闭环整体价值"而非"单次信号质量"。

**用户决策（2026-09-09）**：立项，先锁判据。预注册已写入
`daoti/PREREG_CLOSED_LOOP.md`（C1-C5 会话级判据 + 判定矩阵 + 四前提验收标准），
详见第九节。判据锁定提交后才开始实现（CL1-CL4 分期）。在闭环实验判定 GO 之前，
本计划的 P5（打包发布）不含任何默认开启的导航/闭环行为——产品行为与 v0.9.7 现状一致。

---

## 九、P6 完全体闭环（已立项，2026-09-09；判据已锁定，实现待执行）

> 预注册全文：`daoti/PREREG_CLOSED_LOOP.md`（本地研究资产，判据先于实现锁定）

### 9.1 假说与四前提（全部可工程验收）

**假说**：道体状态信号携带历史交互的累积上下文时，能产生 BGE 无法替代的检索价值。
可否证形式：四前提满足后 C1-C5 判据仍不达 → 假说否证，道体信号退役为元数据记录。

| # | 前提 | 验收标准 |
|---|------|---------|
| ① | 状态真实参与计算 | 消融测试：同查询不同状态下 `/deduce` 产出不同 palaces 的比例 ≥50% |
| ② | reflect 闭环接入运行时 | explore 会话中 `/state` step 随查询递增、active_palace 随结果变化 |
| ③ | 探索进程持续运行 | daemon 启动探索线程，跨查询原型漂移量可测且 >0 |
| ④ | 判据转向闭环整体价值 | 会话级 C1-C5 判据（见下） |

### 9.2 判据摘要（完整版见预注册文档第三节）

- **C1 会话内增益趋势（核心区分指标）**：会话后半段 hop≥2 进 top-5 比例 − 前半段
  ≥ **+10pp**，基线同口径 < +3pp
- **C2 会话整体召回**：hop3-5 recall@10 − 基线 ≥ **+8pp**（历史门槛不放松）
- **C3 显著性**：bootstrap 2000 次，P(C2≥8pp)≥0.95 且 P(C1≥10pp)≥0.90
- **C4 噪声护栏**：异链噪声率 ≤ 基线 +3pp
- **C5 体验护栏（决定性）**：无代码噪声回潮、诚实空态、**无方向漂移劫持**
  （连续 3 查询被拉向同一无关主题即 FAIL——闭环特有正反馈风险）

三臂：A 基线 / L 闭环（完全体 daemon）/ L-off 消融（常驻但无闭环，用于归因）。
会话构造：6 主题会话 × 5 递进查询，会话内模拟"确认"回传（对齐"就是这个"按钮链路）。

### 9.3 实现分期

| 阶段 | 内容 | 状态 |
|---|---|---|
| CL1 | 前提①：daemon 状态参与 deduce（状态宫偏置进推演输入等） | ✅ 完成（2026-09-09：`StateBiasEncoder` 状态偏置注入编码，消融验收 6/6 对 100% 不同，门槛 50%，PASS） |
| CL2 | 前提②：LRC explore 运行时自动回传 /reflect（fire-and-forget 不阻塞） | ✅ 完成（2026-09-09：`post_daoti_reflect` 客户端 + explore 成功后 `tokio::spawn` 回传发现序前 10 条节点；门控 `LRC_DAOTI_REFLECT=1` 默认关，与 `LRC_DAOTI_NAVIGATE` 独立（支撑 L-off 消融臂）；`session_id` 贯通 deduce/reflect 同会话。单测 2 项（在线 applied / 离线静默降级）+ 运行时观测 PASS：daemon step 4→5→7（query1 空结果仅 deduce +1，query2 deduce+reflect +2）、palace 兑宫→艮宫→坤宫随检索结果修正、explore 耗时 0.36/0.51s 未被回传阻塞） |
| CL3 | 前提③：daemon 接入探索进程，漂移量可观测 | ⏳ 待执行 |
| CL4 | 前提④：会话级三臂实验 + 判定 + 回写 | ⏳ 待执行（前提①②③验收全过才允许采数） |

执行纪律：每阶段本地提交 + 回写本档；前提验收失败即暂停修复，不得带病实验；
本协议只执行一轮主实验（工程缺陷最多重跑一次并如实记录）。
