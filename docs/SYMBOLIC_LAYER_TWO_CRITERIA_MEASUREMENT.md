# 符号层两条判据 · 落地与实测报告

> 日期：2026-09-18
> 判据来源：用户裁定（**取代**此前的四条统计判据）
> 状态：**两条判据的落地与实测均已完成**，实测 13/13、单测 10/10

---

## 一、判据（用户原话）

> 「符号层，只要做到两件事情就可以了，
>  一保持它是个状态机，而且是活性的。
>  二它是推理型的。也就是说用户给到目标的时候，它是能够进行推理和快速接受反应的。」

拆成可验证的形式：

| 判据 | 可观测形式 |
|---|---|
| **一 · 状态机 + 活性** | ① 状态跨请求存活（不是每次从零开始）<br>② 状态**跨时间自发演化**（不依赖用户请求） |
| **二 · 推理型** | 给定 target 能**推理**（朝目标几跳）并**快速反应** |

---

## 二、诊断：两条判据其实是**同一件事**

诊断时发现一个此前未被点明的结构事实：符号层其实是**两个服务**。

| 服务 | 端口 | 有推理？ | 有活性？ |
|---|---|---|---|
| `daoti/daoti_daemon.py` | 3222 | ❌ 走 lexicon / v15 旋转 | ✅ `run_beat()` 每 60s 推进 + 状态持久化 |
| `temp/daoti_assoc/assoc_service.py` | 3223 | ✅ L3 结构算子 + `target_hops` | ❌ `serve_forever()`，**零后台线程** |

⇒ **"活性"与"推理"分居两个进程，各占一半。**

### 2.1 判据一为什么不成立

3223 每次 `/cycle` 都无状态——`parse()` 开头就 `bus.new_cycle()` **清空**信号。

而设计文档 [§4.4.1](../../11_记忆联想专项_选型包/记忆联想系统设计文档.md) 早已核实并写下结论：

> `new_cycle()` = `_signals.clear()`、`publish()` = 只累加计数
> ⇒ **它是可观测性组件（计数器 + 日志），不是状态机**：
> 无状态演化、无反馈回路、无转移规则。
> ……若确实需要状态机语义（跨周期状态、反馈），**这是新建工作，不是"接入"**。

### 2.2 判据二为什么只成立一半

| 环节 | 状态 |
|---|---|
| L3 结构引擎（`yijing_cli.exe`） | ✅ 存在，`_structural_hops` 走真 BFS |
| `target_hops`（朝目标几跳） | ✅ 已落地，目标敏感性 T2 实测 PASS |
| `change_ratio`（状态变化率） | ❌ **结构性缺失，永远降级为 0.5** |

`change_ratio` 的缺失原因（`assoc_service.py` 注释原文）：

> 它 = `‖Δstate‖ / ‖state‖`，定义在**推演循环内部** ——
> 单次 interpret 没有"上一步 state"，**该量在单次调用中不存在**。

### 2.3 ★关键：判据二缺的那块，正是判据一的产物

```
没有跨请求存活的 state
      ⇒ 算不出 change_ratio
      ⇒ 调度器缺一个真读数（deviation / 收敛判据不完整）
      ⇒ "快速接受反应"没有依据
```

⇒ **两条判据咬合，必须一起做。** 这决定了实现方案：状态与推理必须在**同一进程**内。

---

## 三、落地

### 3.1 宿主选择（用户裁定）

**在 3223（`assoc_service`）内建状态机**——因为它是**有推理能力**的那一侧；
状态与推理同进程 ⇒ `change_ratio` 立即可算，无需跨服务往返。
不动 3222 既有的导航信号链路。

### 3.2 活性强度（用户裁定）

**轻量演化**：节拍只做纯数值推进（衰减 / 漂移 / 节拍累加）+ 持久化，
**不跑 BGE 推理** ⇒ CPU 增量可忽略，守住用户硬约束（≤70%）。

理由：BGE 单次前向数百毫秒且占 CPU，按节拍反复跑会撞 70% 上限；
而"活性"的定义性要求是**状态跨时间自发变化**，不是"每拍都重新推理"。

### 3.3 新增模块 `temp/daoti_assoc/assoc_state.py`

| 组件 | 职责 |
|---|---|
| `ReasoningState` | 跨请求存活的状态：`state_vec`(176维) / `gua_idx` / `coherence` / `alpha` / `change_ratio` / `beats` / `updated_by` |
| `ReasoningStateStore` | 单例 + 线程安全（HTTP 线程与节拍线程并发）+ 原子持久化（tmp+rename）+ 重启恢复 |
| `start_beat_thread()` | 常驻节拍（默认 60s，`DAOTI_ASSOC_BEAT_SECONDS` 可覆盖） |
| `atomic_write_json()` | 复用 3222 已验证的模式 |

**接入点**（`assoc_service.py`）：

| 位置 | 改动 |
|---|---|
| `_readout_from_forward` | 取出 176 维 `state`（**零额外推理成本**——本就在 forward 输出里） |
| `parse` | 经模块级 `_last_state_vec` **带外**传递（不进 `frame`：避免 176 个浮点数进 HTTP 响应 + 内部表征外泄） |
| `scheduler_decision` | 把本次推理并入状态 ⇒ 取回 `change_ratio`；**首次仍如实降级**（不填 0） |
| `server.py` | 启动节拍线程 + 新增 `GET /state`（可观测入口） |

---

## 四、实测证据（13/13 通过）

脚本：`temp/daoti_assoc/verify_state_liveness.py`
环境：3223，节拍加速至 3s，**隔离状态目录**

### 判据一 · 状态机 + 活性

```
[1] 状态跨请求存活
    第1次: gua_idx=16  state.gua_idx=16  state_dim=176
    第2次: gua_idx=46  state.gua_idx=46  updated_by='request'
  PASS  ★① 状态已更新为后一次推理（跨请求存活 + 被覆盖）
  PASS  ★① 推理后 state_vec 出现且为 176 维（change_ratio 的载体）

[3] 空闲期（不提问，等 12s）
    等待前: beats=129 alpha=0.616330
    等待后: beats=133 alpha=0.607299
  PASS  ★③ 空闲期 beats 增长（节拍线程真的在跑）
  PASS  ★③ 空闲期 alpha 变化（状态真的在演化，不只是计数）
  PASS  ③ 演化方向正确：alpha 向中性值 0.5 衰减
```

### 判据二 · 推理型

```
[2] change_ratio 是真读数
  PASS  ★② change_ratio 非 null（有上一步 ⇒ 算得出）    实际 1.3011
  PASS  ★② change_ratio 有区分度
        换内容: 1.3010903551882655
        同内容: 0.023761447866427043   ← 相差 55 倍
```

> ★"有区分度"是关键：只断言"非 null"无法区分
> "真的算对了"与"随便给了个数"。换内容 >> 同内容 才说明该量**确实反映输入差异**。

### 配套单测（10/10 通过）

脚本：`temp/daoti_assoc/assoc_state_test.py`——覆盖**组件行为**（可复现）：

| 测试 | 锁住的契约 |
|---|---|
| `test_first_observe_returns_none_not_zero` | 首次无上一步 ⇒ **None 而非 0**（不伪造） |
| `test_change_ratio_is_real_reading_on_second_observe` | 用可手算向量验证数值（`√2`） |
| `test_change_ratio_zero_when_state_unchanged` | 状态逐位相同 ⇒ 真 0（与"算不出"相反的一端） |
| `test_state_survives_store_recreation` | ★重启后状态恢复（判据一的核心） |
| `test_beat_actually_changes_state` | 节拍**真的**改状态（不是只涨计数） |
| `test_beat_does_not_touch_change_ratio` | 节拍不得改动该量（否则伪造证据） |
| `test_beat_drift_is_bounded_not_accumulating` | ★漂移**有界**（见 §6 缺陷 1） |
| `test_corrupt_state_file_does_not_crash` | 状态文件损坏 ⇒ 降级不崩 |
| `test_no_tmp_file_left_after_persist` | 原子写不残留 `.tmp` |

---

## 五、★实测发现的新缺陷（超出本轮目标）

### 缺陷 A：`coherence` 恒为精确 0.5 —— **常数冒充读数**

| 项 | 内容 |
|---|---|
| 现象 | 多次不同输入后 `coherence` 恒为 `0.500000` |
| 取证 | `probes/probe_coherence_default.py`：键 `cavity_coherence` **存在**，但其值就是 0.5 |
| 根因 | `trigram_space.py::compute_coherence` 首行 `if not self.initialized: return 0.5`；而唯一能置位的 `update_standing_wave`（经 `update_resonance`）**全仓零调用**（已 grep）⇒ 驻波从未建立 |
| 性质 | **该项目已知问题**——`light_daoti/diagnose_and_fix.py` 首行写着「相干性始终 0.500 (卡死) — resonance_cavity.initialized=False, standing_wave 全零」 |
| 危害 | 0.5 会被当作**真读数**流进状态机与调度器（`state_signals.coherence`）⇒ "状态在演化"的部分证据来自一个**常数** |
| 处置 | 新增 `_resonance_initialized()`（读 `initialized` 缓冲区这一**直接证据**，而非"值等不等于 0.5"）；为真时把 `coherence` 加入 `degraded`，并在 `/state` 暴露 `coherence_is_default` |
| **未处置** | 驻波初始化本身（属道体研究资产内部，需其侧决定是否在线学习时调用 `update_resonance`） |

> **★这是本轮最值得记的一条**：如果直接用 0.5 而不检测，
> 状态机的"活性"会有一部分建立在常数上——**而测试会全绿**，
> 因为 0.5 确实在变化（节拍让它回归），只是它**从来不是输入的函数**。

---

## 六、实现过程中修正的自有缺陷

### 缺陷 1：漂移无界累积（初版设计错误）

| 项 | 内容 |
|---|---|
| 初版 | `v + DRIFT_PER_BEAT`（每拍给每个分量加 0.002） |
| 后果 | **无界增长**：1440 拍/天 × 0.002 = **每天 +2.88**，而 state 分量本身 O(1) ⇒ 一天后语义被漂移项主导；且 `change_ratio`（相对量）趋近 0，**看起来像"状态很稳"**——一个自我掩盖的失效 |
| 修法 | 漂移量 = `DRIFT_REL × RMS(state) × cos(i + beats)`（**与状态同量级**、有界、确定性） |
| 锁定 | `test_beat_drift_is_bounded_not_accumulating`（2000 拍后断言量级不发散） |

### 缺陷 2：`state_vec` 进 HTTP 响应

初版把它放进 `_readout_from_forward` 返回值 ⇒ 会随 `frame` 序列化进 `/parse`、`/cycle` 响应。
两个坏处：响应体积每次多 3~4KB；内部表征外泄（state 是推理引擎中间层，属研究资产边界内）。
⇒ 改为经模块级 `_last_state_vec` **带外**传递。

---

## 七、踩坑记录（环境/工具）

### 坑 1：`.ps1` 里的中文注释会**吞掉下一行**

| 项 | 内容 |
|---|---|
| 现象 | 同一脚本里 `DAOTI_ASSOC_BEAT_SECONDS` 生效，而**下一行**的 `DAOTI_ASSOC_STATE_DIR` 未生效 |
| 取证 | 隔离实验证明环境变量传递正常；服务启动日志的"状态目录来源"显示走了缺省分支 |
| 根因 | PowerShell 以 **ANSI 码页**解码 UTF-8 文件 ⇒ 中文注释的字节被误判，**换行被并入注释** |
| 依据 | 本项目 `.ps1` **既有约定就是 ASCII-only**（多个既有启动脚本头部均标注）——本次是**我违反了它** |
| 修法 | 全部注释改 ASCII；并在脚本里回显 `CHILD_ENV_*`（让"子进程将继承什么"成为**第一手证据**而非推断） |

### 坑 2：CPU 看门狗返回 503 被误当故障

连续调 `/cycle`（每个跑 BGE 前向）会撞上 `_guard()` 的 503 ——
那是**设计内的保护**（用户硬约束 ≤70%），不是缺陷。
⇒ 测试对 503 做**退避重试**；否则会把正确的保护当成 bug。

### 坑 3：实测的前置假设不可靠

初版实测断言"首次 `change_ratio` 必为 null"、"`beats` 必为 0"——
但这两条要求**服务是全新的**，而实测是分次执行的（启动与运行间隔任意）。
⇒ **分工**：组件行为靠单测（可复现），系统性质靠实测（跨 HTTP 边界）。
实测脚本已移除这两条前置断言，并注明原因。

---

## 八、仍未做（诚实声明）

| 项 | 说明 |
|---|---|
| **驻波初始化** | `coherence` 仍恒为 0.5（已如实标注，见缺陷 A）。真修需道体资产侧在线学习时调 `update_resonance` |
| **MoCo 排序（§4.4 步骤 6）** | 仍以「目标 0 跳优先 + 结构算子原序」替代。§3.2.1 的循环依赖（正样本=边、边需 MoCo 筛）未解 |
| **`slot_five_w` 五何映射** | 仍为占位（`slot_mapping_status: PENDING_SLOT_MAP`） |
| **2048 维隐状态** | 本状态机存的是 176 维**投影**；2048 是 V23 引擎隐状态，另一套架构，未接入 |
| **目标驱动价值的统计检验** | 此前 T4 为**弱证据**（`mean_hops` p=0.0415）。本轮的"推理型"是**功能性**判据（能否推理并反应），**未**重做统计检验 |

---

## 九、结论

| 判据 | 结论 | 证据 |
|---|---|---|
| **一 · 状态机 + 活性** | ✅ **成立** | 状态跨请求存活（`gua_idx` 随请求更新）；空闲 12s 无请求而 `beats`/`alpha` 自发演化 |
| **二 · 推理型** | ✅ **成立（功能性）** | L3 真 BFS 算 `target_hops`；`change_ratio` 从降级值变为真读数，且换内容/同内容相差 55 倍 |

**两条判据咬合**：判据二缺的 `change_ratio` 正是判据一的产物 —— 这也解释了
为什么此前"分开看"总有一条不成立。
