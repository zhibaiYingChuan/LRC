# 联想记忆精度提升计划 · 分析与分期（P7，2026-09-09）

> 版本：v1.2（2026-09-09）
> 状态：现状分析与诊断完成；**P7.2 联想边反馈闭环已完成（E1-E4 验收通过）**；**P7.3 路径级评分已完成（F1-F4 验收通过）**；P7.4 待执行
> 前置结论：道体导航三模式（PREREG_NAV / PREREG_ASSOC_NAV / PREREG_CLOSED_LOOP）
> 已一致否证"64 维卦象信号在 768 维 BGE 之上产生召回增益"，本计划不再投入
> 道体信号方向，聚焦可解释的关系检索与用户反馈闭环。
> 关联文档：`docs/DAOTI_STATE_MACHINE_PLAN.md`（P6 已完结，v1.8）

---

## 一、现状梳理：联想记忆功能的两条链路

### 1.1 普通记忆召回（"哪些记忆最相关？"）

入口 [memory_store.rs](file:///G:/code-memory/src/memory_store.rs#L3315-L3348) `recall`：

```
查询 → 过滤（项目/类型/标签/重要性/隐私/过期）
     → 中文 bigram + 英文词分词
     → BM25 打分（k1=1.2, b=0.75，TF 饱和归一）
     → 完整查询命中 +0.4 / 标签 +0.15 / 重要性 ×0.01
     → 类型加权（偏好/决定）+ 合成置信度 + 洛书/八卦辅助分
     → 回归校验（道体再次校验，剔除发散噪声）
     → 排序输出
```

- 中文分词用 bigram（P1-9 修复），词边界用 `contains_word`（防 "cat" 匹配 "category"）。
- 联想桥扩展（LRC_STATE_BIAS=1）：活跃记忆词并入查询，仅零词重叠时以 0.20 权重加分。
- 候选索引（domain-nolang 已默认启用）：项目+语言域分桶 + fallback 全量回退 + verify 审计，离线 42/42 命中、线上序列与 baseline 一致（记忆 #1）。

### 1.2 联想探索（"从一条记忆还能联想到什么？"）

入口 [v1_api.rs](file:///G:/code-memory/src/v1_api.rs#L385-L399) `run_association_explore`：

```
查询/memory_id → root 起点门禁 → BFS 多跳扩散（max_depth≤4, width≤3）
     → 每跳以父记忆内容为查询调用 recall
     → 回归校验锚定 root 主题（防多跳漂移）
     → 生活 root 不混入代码节点；弱匹配诚实空态
```

root 门禁三层：
1. 词面实质共鸣（泛指 bigram 过滤后 token 重叠 ≥ min_required）；
2. 语义旁路（bge 余弦 ≥ 0.55，**不放行代码记忆**，硬时限 6s）；
3. 生活记忆优先于代码记忆充当起点。

护栏：10s 总预算共享、可取消、`weak_match` 诚实空态、回归校验 `regression_query`。

---

## 二、精确度现状评估（分层）

| 能力 | 判断 | 依据 |
|---|---|---|
| 直接词面召回 | 较成熟 | BM25 + bigram + 词边界 + 回归校验 |
| 中文查询处理 | 较成熟 | P1-9 系列修复、泛指 bigram 过滤 |
| 代码/生活隔离 | 较成熟 | 语义旁路禁代码、生活 root 禁代码子节点 |
| 隐私隔离 | 已有明确保护 | `is_visible` 过滤 + feedback 持久化本地 |
| root 起点准确性 | 偏保守但可靠 | 三层门禁 + 诚实空态 |
| 多跳主题稳定性 | 有护栏但牺牲召回 | root 回归校验 |
| 深层语义联想 | 能力有，效果不稳定 | 语义旁路仅词面零命中时触发 |
| 道体导航增益 | **已否证** | P3.5 −4.7pp；闭环 −4.7pp 且 L/L-off 逐字节一致 |
| 长期状态驱动召回 | **已否证** | 四前提满足仍零差异 |

**结论**：当前联想功能"防错强、扩展弱"——护栏完善，但没有被验证能稳定找到
更多深层正确联想。

---

## 三、问题诊断（四大短板）

### 3.1 普通召回仍以词面重叠为主
"上次那个重要的日子" ↔ "结婚纪念日是 5 月 20 日" 这类语义改写仅靠语义旁路补救，
而旁路只在词面零命中时触发，覆盖面有限。

### 3.2 多跳扩散缺"联想边"，关系是临时推断
当前边 = "A 的内容召回了 B"，不携带用户确认历史。用户点"就是这个"只激活 B，
**不记录"从 A 出发到 B 是一条可靠边"**。全局活跃锚点无法替代边级证据。

### 3.3 用户反馈粒度不足且不参与排序
- `/v1/feedback` 的 `AssociationRelevance` **只观测不排序**（message 明确声明）；
- `/v1/feedback/association-stats` 只按**记忆**聚合，无**起点维度**；
- 无法区分：root 错 / child 错 / 边不合理 / 只是有趣但不相关。

### 3.4 缺少分层评测指标
只有整体 recall@10，无法回答"root 找错 vs child 找错 vs 空态误报"，定位不到病灶。

---

## 四、提升方向（按优先级）

| # | 方向 | 价值 | 成本 | 与道体关系 |
|---|---|---|---|---|
| 1 | **联想边反馈闭环**（用户确认/拒绝形成边证据，探索排序消费） | 高——直接用真实用户判断 | 中 | 无关（替代道体状态） |
| 2 | root/edge/child 分开评分（路径级评分替代硬过滤） | 高 | 中高 | 无关 |
| 3 | 分层评测指标（root_precision / noise / 空态准确率） | 高——定位病灶前提 | 低 | 无关 |
| 4 | 查询意图分类（事实/回忆/联想/偏好路由） | 中 | 中 | 无关 |
| 5 | 真实用户反馈评测集 | 中（长期） | 高 | 无关 |

本计划**不再投入**：道体导航、卦宫信号、活性偏置扩展（均已否证或属于独立研究方向）。

---

## 五、分期实施（P7）

| 阶段 | 内容 | 交付物 | 状态 |
|---|---|---|---|
| P7.1 | 本分析文档 + 判据锁定（见第六节） | `docs/ASSOCIATION_ACCURACY_PLAN.md` | ✅ 本文件 |
| **P7.2** | **联想边反馈闭环**：数据层边记录 + confirm/feedback 记边 + explore 排序消费 | user_feedback.rs + v1_api.rs + 单元测试 | ✅ 已完成（2026-09-09，判据 E1-E4 全过，详细见第七节） |
| P7.3 | 路径级评分（root 主题一致性软评分层，门控 `LRC_ASSOC_PATH_SCORE` 默认关） | v1_api.rs explore 排序增强 + 单元测试 | ✅ 已完成（2026-09-09，判据 F1-F4 全过，详细见第八节） |
| P7.4 | 分层评测集与指标脚本（复用 `_fair_corpus.py`） | daoti/ 评测脚本 | 待执行 |

执行纪律：
1. **零影响承诺**：P7.2 门控 `LRC_ASSOC_EDGE_FEEDBACK=1` 默认关，关闭时 explore 行为
   与现状逐字节一致（复用 P3.5 的降级契约精神）。
2. 判据先于实现（第六节写死后再动代码）。
3. 每阶段本地提交 + 回写本档 + 记忆库同步。

---

## 六、P7.2 判据（数据产生前锁定）

**目标**：用户确认/拒绝联想形成边证据，探索排序消费后提升联想精度且零负向。

- **E1 边记录生效**：confirm 请求携带 `from_id` 时，记录 (from→to, positive) 边；
  feedback 的 AssociationRelevance 携带 `from_id` 时记录对应正/负边；JSONL 持久化
  后可重启恢复（serde default 兼容旧记录）。
- **E2 排序消费生效**：门控开启时，explore BFS 对 (from, candidate) 有正边证据的
  候选排序前置、负边证据的后置；关闭时结果与无反馈基线逐字节一致（消融对照）。
- **E3 兼容与回归**：旧 JSONL（无 from_id 字段）加载不报错；全量测试 0 失败；
  `cargo check --features qdrant` 通过。
- **E4 不回归护栏**：边调整只改变候选**顺序**，不豁免 CodeContext 过滤、泛指
  bigram 过滤、root 主导性门禁、时间预算（继承全部既有防线）。

---

## 六B、P7.3 判据（数据产生前锁定，2026-09-09）

**目标**：路径级评分——联想探索对每个候选施加"root 主题一致性"软评分
（与 root 实义词共享 ≥2 加成 / =1 无调整 / =0 漂移惩罚），把多跳扩散的
主题漂移从"仅靠回归硬门禁"升级为"硬门禁 + 软排序"双保险，且零负向。

设计边界（逆向推导）：现有回归校验（regression_recheck）已把"与 root 零共享"
的候选硬剔除，因此软评分的作用域是**通过硬门禁的候选**（original_overlap ≥1），
对零共享候选的惩罚仅影响"带标签共鸣等证据通过门禁"的边缘样本。

- **F1 软评分生效**：门控 `LRC_ASSOC_PATH_SCORE=1` 开启时，同一候选的节点分数
  相对基线发生三档偏移——root 实义词共享 ≥2 的候选 +ASSOC_PATH_TOPIC_BONUS、
  =1 的无偏移、=0 的 −ASSOC_PATH_DRIFT_PENALTY（差分断言，容差 1e-3）。
- **F2 消融对照**：门控关闭（默认）时 explore 输出与现状逐字节一致；开启后
  还原门控，输出回到基线（零影响承诺）。
- **F3 兼容与回归**：全量测试 0 失败；`cargo check --features qdrant` 通过。
- **F4 不豁免护栏**：路径评分只调候选**分数/顺序**，不豁免 CodeContext 过滤、
  回归硬门禁、root 门禁、width 与时间预算；对零共享候选不重新放行。

---

## 七、结果回写（2026-09-09，P7.2 完成）

### 7.1 实现结果

| 项 | 结果 |
|---|---|
| P7.2 状态 | ✅ 完成 |
| 数据层 | [user_feedback.rs](file:///G:/code-memory/src/engine/user_feedback.rs)：`FeedbackRecord` 增加可选 `association_from_id`；新增 `record_association_edge_feedback` 与 `get_edge_adjustments`；正/负/中立按 `(from→to)` 聚合，平滑归一化到 [-1, 1] |
| confirm 接口 | [v1_api.rs](file:///G:/code-memory/src/v1_api.rs#L1600-L1675)：`ConfirmAssociationRequest` 增加可选 `from_id`；门控开启时确认操作记录正边；无 `from_id` 时保持旧行为 |
| feedback 接口 | [v1_api.rs](file:///G:/code-memory/src/v1_api.rs#L2260-L2350)：`AssociationRelevance` 支持可选 `from_id`，携带时记录边级正/负反馈，无起点时兼容原记忆级反馈 |
| explore 消费 | [v1_api.rs](file:///G:/code-memory/src/v1_api.rs#L581-L675)：门控开启时 BFS 按 `(from→candidate)` 净调整稳定重排；只改变候选顺序，不绕过既有防线 |
| 门控 | `LRC_ASSOC_EDGE_FEEDBACK=1` 才启用，默认关闭；关闭时边调整表为空，现有行为逐字节保持 |

### 7.2 判据验收

| 判据 | 验收证据 | 判定 |
|---|---|---|
| E1 边记录生效 | 边方向字段、正/负/中立聚合、confirm/feedback 入口、JSONL 持久化与重启恢复测试通过 | ✅ PASS |
| E2 排序消费生效 | `test_assoc_edge_feedback_rerank_in_explore`：门控开启后确认边将 child_b 前置，门控关闭恢复基线顺序 | ✅ PASS |
| E3 兼容与回归 | 旧 JSONL 无 `association_from_id` 可正常加载；`cargo check --offline --features qdrant` 通过；全量测试无失败 | ✅ PASS |
| E4 不回归护栏 | 边调整仅作用于 BFS 候选相对顺序，CodeContext 过滤、root 门禁、回归校验、width 与时间预算均保留 | ✅ PASS |

### 7.3 测试结果

| 测试集 | 结果 |
|---|---|
| P7.2 edge 相关测试 | 10 passed / 0 failed |
| lib 单元测试 | 623 passed / 0 failed |
| benchmarks | 11 passed / 0 failed |
| luoshu invariants | 13 passed / 0 failed |
| memory state machine e2e | 8 passed / 0 failed |
| 合计 | **655 passed / 0 failed**（另有 8 ignored） |
| qdrant 特性编译 | `cargo check --offline --features qdrant` ✅ |

### 7.4 设计边界与后续

本阶段只把**明确的用户联想反馈**转化为 `(from→to)` 边证据，并以 opt-in 方式参与
联想探索排序；没有改变普通 recall 的默认排序，也没有放宽任何精度护栏。

P7.3 继续处理路径级评分（root / edge / child 分离与漂移惩罚）；P7.4 再建立分层
评测集和指标脚本。在 P7.3/P7.4 的离线评测证明收益前，不扩大边调整幅度、不默认
开启该门控。

| 项 | 值 |
|---|---|
| 代码验证 | ✅ 编译、P7.2 专项测试、全量回归均通过 |
| 提交 | 待本地提交 |
| 记忆同步 | 待提交完成后记录本次 P7.2 架构变更 |

---

## 八、结果回写（2026-09-09，P7.3 完成）

### 8.1 实现结果

| 项 | 结果 |
|---|---|
| P7.3 状态 | ✅ 完成 |
| 路径主题 token | [v1_api.rs](file:///G:/code-memory/src/v1_api.rs#L388-L425)：新增 `ASSOC_PATH_TOPIC_BONUS=0.10`、`ASSOC_PATH_DRIFT_PENALTY=0.20` 与 `LRC_ASSOC_PATH_SCORE` 门控；root 内容提取实义 bigram，过滤泛指 bigram |
| 候选路径评分 | [v1_api.rs](file:///G:/code-memory/src/v1_api.rs#L605-L700)：每轮 BFS 预计算候选与 root 的主题重叠数；≥2 加成、=1 不调整、=0 惩罚，再与 P7.2 边反馈叠加后稳定重排 |
| 设计边界 | 仅改变候选分数/顺序，不重新放行零证据候选；CodeContext 过滤、root 门禁、回归硬校验、width 与时间预算均保留 |
| 默认行为 | `LRC_ASSOC_PATH_SCORE` 默认关闭；关闭时 path adjustment 为 0，现有 explore 行为保持不变 |

### 8.2 判据验收

| 判据 | 验收证据 | 判定 |
|---|---|---|
| F1 软评分生效 | `test_assoc_path_score_topic_rerank_in_explore` 验证三档差分：共享 ≥2 为 +0.10、共享 1 为 0、共享 0 为 −0.20 | ✅ PASS |
| F2 消融对照 | 测试在关闭/开启/恢复关闭三个状态运行；恢复后节点分数回到基线 | ✅ PASS |
| F3 兼容与回归 | `cargo check --offline --features qdrant` 通过；全量测试无失败 | ✅ PASS |
| F4 不豁免护栏 | 路径评分只在候选重排层生效，未修改 root/CodeContext/回归校验硬门禁 | ✅ PASS |

### 8.3 测试结果

| 测试集 | 结果 |
|---|---|
| P7.3 专项测试 | 1 passed / 0 failed |
| lib 单元测试 | 624 passed / 0 failed |
| benchmarks | 11 passed / 0 failed |
| luoshu invariants | 13 passed / 0 failed |
| memory state machine e2e | 8 passed / 0 failed |
| 合计 | **656 passed / 0 failed**（另有 8 ignored） |
| qdrant 特性编译 | `cargo check --offline --features qdrant` ✅ |

### 8.4 结论与后续

P7.3 已完成，但它是**默认关闭的可选排序实验能力**，不是默认产品行为。当前
路径评分的主要效果是将 root 主题一致性显式化为可观测的软分，不会绕过已有护栏，
也没有证明最终召回率已经提升。是否开启必须由 P7.4 分层评测结果决定。

P7.4 将建立分层评测集与指标脚本，分别测量 root、edge、child、深度、噪声和空态
准确性；在 P7.4 证明净收益前，不默认开启 `LRC_ASSOC_PATH_SCORE`。

| 项 | 值 |
|---|---|
| 代码验证 | ✅ P7.3 专项、全量回归、qdrant check 均通过 |
| 提交 | 本轮 P7.3 变更已本地提交 |
| 记忆同步 | 待提交完成后记录 P7.3 路径级评分架构变更 |