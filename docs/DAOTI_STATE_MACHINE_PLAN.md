# 道体状态机接入 LRC 记忆系统 · 推进计划

> 版本：v1.1（2026-09-08）
> 状态：P0 已完成，P1 待执行
> 最近进度：P0 本地提交 50304ef；未推送
> 前置文档：`daoti/PREREG_NAV.md`、`daoti/PREREG_FAIR_STATE_MACHINE.md`
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

### P1 点亮调节器心跳（道体血统的活性保证，纯 Rust 侧）

| 任务 | 内容 | 涉及文件 |
|------|------|---------|
| P1.1 | sidecar 后台周期任务（tokio interval，建议 30 分钟）调用 `store.regulate()` | `src/server.rs` |
| P1.2 | 调节动作落地审计：`AdjustDecayRate`/`AdjustRetrievalWeights` 等执行结果写入 audit-trail | `src/engine/audit_trail.rs` |
| P1.3 | 前端系统状态卡展示"调节器心跳"状态（最近调节时间/最近动作），接入 TAB_LOADERS | `static/app.js`、`static/index.html` |
| P1.4 | 单测：模拟阴阳失衡 → regulate 返回动作 → 衰减率真实变更 → 审计事件存在 | 新增测试 |

**验收标准**：sidecar 运行 30 分钟后，`/v1/trust/audit-integrity` 可见调节器心跳事件；防振荡机制（冷却/冻结）在单测中验证。

**降级安全**：调节器已内置防振荡 v2.0（历史追踪/冲突仲裁/自适应步长/冷却）与冻结机制，异常时 NoAction，不改变检索行为。

### P2 导航信号生产者：daoti_daemon 常驻进程（License 合规）

| 任务 | 内容 | 涉及文件 |
|------|------|---------|
| P2.1 | 新建 `daoti/daoti_daemon.py`：常驻 HTTP 服务（纯 stdlib，对齐 lrc_bridge 风格），暴露 `/health`、`/deduce`（查询+会话id → NavigationSignal JSON）、`/reflect`（检索结果回传 → 更新推演状态）、`/state` | 新文件（研究资产侧） |
| P2.2 | 常驻推演节拍器：空闲期按固定周期推进状态演化，满足两次交互之间状态持续变化 | 新文件 |
| P2.3 | 推演状态持久化：`~/.loong-recall/daoti/session_state.json`（跨查询存活，原子写入，重启恢复） | 新文件 |
| P2.4 | 编码器走四级校准路线第④步成果：GuaLexiconEncoder（纯 stdlib 词典计数，已实现） | `daoti/calibrate_lrc_mapping.py` |
| P2.5 | LRC 侧降级客户端：`explore`/`recall` 调 daoti_daemon 失败（超时 2s/不可达）→ 信号缺省 → 基线行为 | `src/server.rs`、`src/v1_api.rs` |
| P2.6 | 桌面端以 sidecar 管理模式拉起/回收 `daoti_daemon`，并做健康检查、PID 校验与异常降级 | `desktop/src-tauri/` |
| P2.7 | PyInstaller 打包 `daoti_daemon.exe`，免用户 Python 环境依赖 | `daoti/` 打包脚本 |
| P2.8 | 走完四级校准：④ 映射校验（已有脚本）→ ③ 链路验证 → ① 权重标定 → ② 阈值标定 | `daoti/` 校准脚本 |

**验收标准**：`python daoti/daoti_daemon.py` 启动后，`Invoke-RestMethod http://127.0.0.1:3222/health` 返回 ok；同一会话连续两次 `/deduce` 的推演状态演进（state/target_gua 变化）且 `/state` 可见状态、重启进程后状态可恢复。

**端口约定**：daoti_daemon 固定 `127.0.0.1:3222`（避开 sidecar 3099/3111、桌面 dev 1420），环境变量 `DAOTI_SERVICE_URL` 可覆盖。

### P3 联想中心接入导航 + 预注册实验（先写判据，后接数据）

| 任务 | 内容 | 涉及文件 |
|------|------|---------|
| P3.1 | **预注册判据文档先行**（见第四节），写入 `daoti/PREREG_ASSOC_NAV.md`，数据产生前锁定 | 新文档 |
| P3.2 | `/v1/associations/explore` 增加 `navigation` 消费：有信号 → `navigated_deep_recall` 产生的候选进入根节点门禁双通路（词面+语义旁路）；无信号 → 现状 `explore_pure` 逐字节一致 | `src/v1_api.rs` |
| P3.3 | 导航候选仍受 CodeContext 过滤与泛指 bigram 过滤约束（复用现有防线） | `src/v1_api.rs` |
| P3.4 | 时间预算：daoti 往返 + 多视图检索合计 ≤10s，超时收敛返回部分结果 | `src/v1_api.rs` |
| P3.5 | 实验执行：按预注册语料（100 记忆 + 30 查询）跑三臂对照 | `daoti/` 评测脚本 |

**预注册判据**（在 P3.1 中写死，全部满足才 go，任一不满足即 no-go 并诚实收场）：

- **G1 互补性**：基线臂 top1 错误的查询子集上，导航臂把 hop≥2 同链记忆拉进 top-5 比例 ≥30%
- **G2 净增量**：导航臂 hop3-5 recall@10 − 基线 ≥8pp，且 hop1-2 recall@3 下降 ≤2pp
- **G3 显著性**：按查询 bootstrap 重采样 2000 次，P(G2 增益 ≥8pp) ≥0.95
- **G4 噪声护栏**：导航臂 top10 异链噪声率不高于基线 +3pp
- **G5 体验护栏**：生活类查询（今晚吃什么）CDP 实测无代码噪声回潮；"量子物理是什么"仍为诚实空态

**No-go 收场**（预注册纪律）：导航信号退回"用户确认联想时作为元数据记录"（同 PREREG_FAIR 的收场规则），联想中心保持 CHAIN1+词面校验现状——该现状已通过 7 项 CDP 专项与 88 交互口回归门禁。

### P4 回归验证层升级：道体状态级 recheck（词面级之上叠加）

| 任务 | 内容 | 涉及文件 |
|------|------|---------|
| P4.1 | `regression_recheck` 增加第四证据位：**原意图回应度**——daoti_daemon `/reflect` 返回的意图相关分（≥阈值 0.5 判"回应原意图"） | `src/engine/memory_state_machine.rs` |
| P4.2 | 道体离线时第四证据缺省跳过（三证据照常），行为回退现状 | 同上 |
| P4.3 | 前端证据标签映射：新增"和你的本意对得上"白话文案 | `static/app.js` |

**验收标准**：四证据单测 + 道体离线回退单测；CDP 实测证据标签渲染正确。

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
P1 调节器心跳（道体血统活性上线）              ← 下一步
P2 daoti_daemon 常驻进程 + 四级校准            ← 道体持续运行落地（生产者就绪）
P3 预注册判据 → 联想中心接入 → 三臂实验         ← 判据先写死，go/no-go 都诚实收场
P4 道体状态级回归校验（第四证据位）             ← 实验通过后叠加
P5 打包发布（MSI 重打包 + 全量回归）           ← 收口
```
