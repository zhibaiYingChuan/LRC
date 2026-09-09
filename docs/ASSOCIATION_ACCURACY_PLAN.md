# 联想记忆精度提升计划 · 分析与分期（P7，2026-09-09）

> 版本：v1.9（2026-09-09）
> 状态：P7 全阶段完结（P7.2/P7.3 完成、P7.4 分层评测 NO-GO，门控默认关）。**P8 root 语义召回已立项（前置可行性收尾：归因完成、ml 构建可行、bge 权重网络隔离不可获取 → P8.3 外部阻塞；P8.2c 旁路活性自检落地+HTTP 实证+评测脚本化，P8.3 判据 H1-H4 已脚本化待 B 臂接入，判据锁定待用，详见第十节）**
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
| P7.4 | 分层评测集与指标脚本（三臂 A/P/E，真实 sidecar） | daoti/_assoc_accuracy_eval.py + 结果 JSON | ✅ 已完成（2026-09-09，判据 G1-G5，**判定 NO-GO**，详细见第九节） |

执行纪律：
1. **零影响承诺**：P7.2 门控 `LRC_ASSOC_EDGE_FEEDBACK=1`、P7.3 门控
   `LRC_ASSOC_PATH_SCORE=1` 均默认关，关闭时 explore 行为与现状逐字节一致
   （复用 P3.5 的降级契约精神）。
2. 判据先于实现（第六/六B/六C节写死后才动代码）。
3. 每阶段本地提交 + 回写本档 + 记忆库同步。
4. **P7.4 收场纪律**：分层评测未证明净收益前，两个门控一律不开；联想中心维持
   现状（CHAIN1 + 词面校验 + 语义旁路 + 回归硬门禁）。

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

## 六C、P7.4 判据（数据产生前锁定，2026-09-09）

**目标**：分层评测——在真实 sidecar 上对联想探索做三臂对照（A 基线 / P 路径
评分 / E 边反馈），分层测量 root 召回、联想链纯度（逐跳）、噪声、空态准确性，
回答"P7.3/P7.2 的可选排序能力是否带来净收益，是否进入默认开启决策"。

**分层原则**：联想链"纯度"只在 root 正确的查询子集上统计（root 错误时围绕错误
起点发散不代表联想质量）；root 召回与空态在三臂上必须保持一致（软评分不得改变
root 选择与空态判定）。

**指标**（探索 depth=4, width=3，复用 `_fair_corpus.py` 30 查询 + 2 无关联查询）：
- root_recall：root 记忆属于查询 gold 链的比例（30 应命中查询）
- 联想链同链率：root 正确子集上，depth≥1 节点中属于 gold 链的比例（全树 + 按
  depth1/2/3 分层）
- noise_top10：root 正确子集上，前 10 节点中异于 gold 链（含干扰）的比例
- 空态：2 条无关联查询（"量子物理是什么"等）weak_match=true；应命中查询不误报空态

**判据（G1-G5，全过才进入默认开启决策）**：

| 判据 | 门槛 |
|---|---|
| G1 root 选择不破坏 | P/E 的 root_recall 与 A 一致（软评分不改变 root 门禁） |
| G2 联想链纯度提升（核心） | P 相对 A 的全树同链率提升 ≥ **+5pp**（root 正确子集）；E 相对 A 不降 |
| G3 噪声护栏 | P/E 的 noise_top10 ≤ A +3pp |
| G4 空态护栏 | 无关联查询三臂均诚实空态；应命中查询误报空态率 P/E ≤ A +3pp |
| G5 判定 | G2 达成且 G1/G3/G4 全过 → P7.3 路径评分"有净收益"，进入默认开启决策；否则保持默认关、如实记录（诚实收场，不预设理由） |

**诚实先验声明**：CL4 实测软信号在真实探索中效应量极小（闭环零差异）；P7.3 的
软评分只微调候选相对顺序，G2 的 +5pp 是强门槛，可能不达。本评测没有为 GO 预设
任何理由。

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
| 提交 | 已本地提交（c91e810） |
| 记忆同步 | 已同步 P7.2 联想边反馈架构变更（记忆库 ID 9bec2bcf） |

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
| 记忆同步 | 已同步 P7.3 路径级评分架构变更（记忆库 ID 4e2440d9） |

---

## 九、结果回写（2026-09-09，P7.4 完成）

### 9.1 执行记录

| 项 | 值 |
|---|---|
| 评测脚本 | `daoti/_assoc_accuracy_eval.py`，复用 `_fair_corpus.py` |
| 评测形态 | 真实 sidecar 三臂对照：A 基线 / P 路径评分 / E 边反馈 |
| 配置 | depth=4、width=3；30 条应命中查询（6 主题链 × 5）+ 2 条无关联查询 |
| E 臂预置 | 6 条主题链的 hop1→hop2 同链确认边，共 48 条，模拟历史用户确认 |
| 环境 | A :3130（无门控）/ P :3131（`LRC_ASSOC_PATH_SCORE=1`）/ E :3132（`LRC_ASSOC_EDGE_FEEDBACK=1`）；三套独立 `--db-path` |
| 结果文件 | `daoti/assoc_accuracy_results.json`（全量 records + summary + criteria） |
| 环境清理 | 评测完成后终止 3 个 sidecar，3130/3131/3132 端口已释放 |

### 9.2 分层结果

| 指标 | A 基线 | P 路径评分 | E 边反馈 |
|---|---:|---:|---:|
| root_recall | 46.7%（14/30） | 46.7% | 46.7% |
| root 正确查询数 | 14 | 14 | 14 |
| 全树 child 同链率（root 正确子集） | 53.3% | 53.3% | 53.3% |
| depth1 同链率 | 56.4% | 56.4% | 56.4% |
| depth2 同链率 | 78.6% | 78.6% | 78.6% |
| depth3 同链率 | 0.0% | 0.0% | 0.0% |
| noise_top10 | 32.3% | 32.3% | 32.3% |
| 应命中查询误报空态率 | 53.3% | 53.3% | 53.3% |
| 无关联查询诚实空态率 | 100% | 100% | 100% |

### 9.3 判据验收

| 判据 | 结果 | 判定 |
|---|---|---|
| G1 root 选择不破坏 | P/E root_recall 均与 A 一致，14/30 | ✅ PASS |
| G2 联想链纯度提升 | P−A = **0.0pp**，未达 +5pp；E−A = 0.0pp | ❌ FAIL |
| G3 噪声护栏 | P/E = 32.3%，未超过 A +3pp | ✅ PASS |
| G4 空态护栏 | 2 条无关联查询三臂均 weak_match=true 且 0 节点；应命中查询三臂表现一致 | ✅ PASS |
| G5 总体判定 | G2 未达，不能进入默认开启决策 | ❌ FAIL |

**门控有效性核验**：评测并非因门控失效而无效。独立探针选择有结果的查询
"想去看红叶什么时候去合适"：A 的"看红叶最佳时段"节点分数为 8.8124，P 为
8.9124，差值 **+0.1000**，精确等于 `ASSOC_PATH_TOPIC_BONUS`。因此 P7.3
路径评分确实生效；但该候选与下一候选的原始分差（8.81 vs 4.70）远大于
+0.10，未改变候选集合或输出顺序。E 臂同理没有产生可测排序变化。

### 9.4 真实瓶颈与结论

P7.4 暴露了比排序层更基础的瓶颈：30 条应命中查询只有 **46.7%** 找到正确
root，**53.3%** 直接进入 weak_match 空态；在 root 正确的子集上，depth1 同链率
为 56.4%、depth2 为 78.6%，depth3 降至 0%。因此当前主要问题不是"已找到候选
如何排序"，而是"能否先召回正确 root，以及如何支持跨词面语义起点"。

**NO-GO（P7.4 净收益未达预注册门槛）**：G1/G3/G4 通过，G2/G5 失败。按预注册
纪律执行：

1. `LRC_ASSOC_PATH_SCORE` 保持默认关闭；
2. `LRC_ASSOC_EDGE_FEEDBACK` 保持默认关闭；
3. 联想中心维持现状硬护栏，不把实验排序能力宣称为精度提升；
4. `daoti/_assoc_accuracy_eval.py` 与结果 JSON 保留为可复现实验资产；
5. 若未来继续投入，应另立计划专门解决 **root 语义召回**（同义改写、事件/实体
   抽取、时间与项目上下文），不能继续单纯放大排序偏置。

### 9.5 P7 总结

P7.2 已证明用户确认边可以在 opt-in 模式下改变候选顺序；P7.3 已证明 root 主题
软评分可以产生精确的分数偏移；P7.4 证明二者在公平真实语料上的**整体效应量为
0**，未改变 root、child、深度或噪声指标。P7 计划按预注册标准完成，结论是：
当前联想记忆功能的防错护栏有效，但排序层改造不足以解决根召回瓶颈。

---

## 十、P8 root 语义召回（已立项，前置可行性阶段）

> 续接 P7.4 第九节 9.4 第 5 条结论：若继续投入，应另立计划解决 root 语义召回，
> 不能继续放大排序偏置。本计划从 root 召回瓶颈的**归因**出发，诚实评估
> 可行的介入点及其前置依赖。

### 10.1 归因结论（P8.1，离线分析，2026-09-09）

对 P7.4 的 30 条应命中查询（A 臂）做词面归因（对齐 Rust `tokenize_query` /
`is_generic_bigram` 口径，root 门禁门槛 `min_required = clamp(len,1,2)`）：

| 分类 | 数量 | 含义 |
|---|---:|---|
| 词面可救（lexical_rescuable） | 0 | 无"词面本可过 root 门禁但被候选池吞掉"的查询——**不是候选池/排序问题** |
| 弱词桥（semantic_needed） | 14 | 查询与同链记忆仅 1 个实义词面共享（< min_required=2）——语义扩展可救空间 |
| 纯语义孤儿（orphan） | 2 | 词面零桥（"肚子饿了晚上吃点啥""爬山那天都注意点啥"）——纯语义空间 |
| root 正确 | 14 | 正确示例 shared≥2 |

**结论**：16 个 MISS 全部落在词面门槛之下，词面机制（BM25 bigram + 门槛 ≥2）
结构性覆盖不到；唯一已存在但从未被测的救回机制是 **root 语义旁路**（bge 余弦
≥0.55，见 [v1_api.rs](file:///G:/code-memory/src/v1_api.rs#L501-L522)）。

### 10.2 旁路现状核查（关键发现）

语义旁路机制完备（BGE 检索指令前缀 + 并行编码 + 硬时限 6s + **不放行代码记忆**），
但**从未在任何真实评测中启用**：

- sidecar 默认构建为 `server` feature（无 `ml`）→ [memory_store.rs](file:///G:/code-memory/src/memory_store.rs#L1167-L1169)
  的 `semantic_similarities` 直接返回 `vec![None; ...]`，旁路自动失效；
- 即使 `ml` feature 构建，[models/](file:///G:/code-memory/models) 下
  `BAAI/bge-small-zh` **缺 model.safetensors**（仅 tokenizer/config），
  `encode_embedding` 无法产出句向量；
- P7.4 三臂（乃至 CL4）全部运行在统计编码器上，旁路从未进入被测状态。

因此 P7.4 的"53.3% 查询 weak_match"是**旁路缺席**下的基线，不是旁路失效。

### 10.3 前置依赖实测（P8.2，2026-09-09）

| 前置 | 状态 | 实测证据 |
|---|---|---|
| `ml` feature 构建 | ✅ **可行** | 本地 cargo 缓存含 candle-core/nn/transformers、tokenizers、hf-hub 全部依赖；`cargo build --offline --features server,ml --bin code-memory-server` 54.3s 构建成功（仅无害 LNK4098 链接警告） |
| bge-small-zh 模型权重 | ❌ **缺失且当前环境不可获取** | hf-mirror 与 huggingface.co 的 model.safetensors HEAD 请求双超时；`curl.exe` 直接请求 exit 28（超时）→ 网络隔离确认；G:/D: 浅层搜索无 bge safetensors 备份；本地 HF 缓存目录不存在 |
| 旁路救回率实测（P8.3） | ⏸ 待 bge 权重就绪 | 对 14 弱桥 + 2 孤儿查询测 bge 余弦 ≥0.55 的可救回数 |

**备选模型探针（已弃用，如实记录）**：曾尝试以本地唯一完整模型
`microsoft/graphcodebert-base`（代码 BERT，623MB，models/ 下 config+权重齐全）顶替
bge 运行旁路。两次实测（`--mode smart` + `LRC_LUOSHU_MODEL_ID=microsoft/graphcodebert-base`）
中，sidecar err.log 均无任何 ML 编码器加载/降级日志，编码器加载路径未能确证生效
（存在 mode/feature 配置路径疑点）——**探针判定无效，不作为证据**。该尝试不改变
P8 结论：语义旁路实验必须使用 bge-small-zh 权重，代码模型即使可加载也不构成
替代（领域不匹配，且无法回答原判据）。

### 10.4 实验判据（P8.3 实测前锁定，数据产生前写死）

- **H1 救回率**：语义旁路对 16 个 MISS 查询的救回数 ≥ **10/16**（root 正确），
  且被救回的 root 不得含代码记忆（旁路禁代码契约）；
- **H2 无新增噪声**：开启旁路后，原 root 正确 14 查询的 root 仍正确（不退化），
  无关联查询（量子物理/股市）仍诚实空态；
- **H3 门槛一致**：救回 root 的余弦阈值维持 0.55（不因追求召回而下调）；
- **H4 判定**：H1-H3 全过 → root 语义旁路进入默认开启评审（配套 ml 构建与
  模型打包策略）；否则如实记录"旁路在公平语料上救回率不足"，维持现状空态。

### 10.5 旁路活性自检（P8.2c，2026-09-09）

**背景**：P7.4（旁路缺席被误读为旁路无效）与 P8.2 探针（编码器加载状态无法
观测）的共同教训——语义旁路在统计模式下**静默失效且不可观测**，导致任何在
统计模式编码器上跑的评测都无法区分"旁路缺席"与"旁路无效"。

**实现**（[v1_api.rs](file:///G:/code-memory/src/v1_api.rs#L250-L260)，零行为影响）：
`AssociationExploreResponse` 新增 `semantic_bypass` 字符串字段：

| 值 | 含义 |
|---|---|
| `unused` | 词面起点门禁已有人通过，未触发语义旁路 |
| `applied` | 旁路触发且编码器产出向量参与判定（ML 编码器可用） |
| `unavailable` | 旁路触发但编码器无向量（统计模式/模型缺失）→ 旁路实际未参与 |

**验收**：`test_assoc_explore_semantic_bypass_activity_probe`（统计模式弱匹配
查询 → unavailable 且诚实空态；词面通过查询 → unused）+ 字段名序列化断言；
全量 lib 625 / 全测试集 **657 passed / 0 failed**，qdrant check 通过。

**HTTP 层实证（P8.2d，2026-09-09）**：用当前 debug 二进制（ml feature 构建，
默认不加 `--mode smart`/模型参数）启动真实 sidecar，写入公平语料后发代表性查询，
实测 `semantic_bypass` 在 HTTP 响应中的输出：

| 查询类型 | 示例 | weak_match | semantic_bypass |
|---|---|---|---|
| 弱桥（P7.4 归因 shared=1） | 今晚吃什么好呢 / 周末爬山安排得怎么样了 / 猫咪不吃饭这事后来怎么样 | true | `unavailable` |
| 词面通过 | 孩子下周三期末考试怎么备战 | false（root=应用题记忆） | `unused` |
| 无关联 | 量子物理是什么 / 今天股市行情怎么样 | true | `unavailable` |

**点评**：（1）P7.4 报告的弱桥查询在 HTTP 层确认为 `unavailable`——"旁路缺席"
获得端到端实证，与 10.2 的代码推断一致；（2）ml 二进制在无 ml 参数时正确回退
统计模式（bge 缺失 → local_ml_model_ready=false），不会误启用旁路；（3）词面
通过查询的 root 不受新字段影响（行为零变化）。

**评测脚本级自检（P8.2e，2026-09-09）**：把 10.5 的"前置纪律"从文档要求固化为
评测脚本能力——`daoti/_assoc_accuracy_eval.py` 更新后：
- `record_metrics` 采集 `semantic_bypass`；
- `summarize` 输出 `weak_queries` 与 `weak_bypass_distribution`（弱匹配查询的
  applied/unavailable 计数）；
- `evaluate` 输出 `bypass_active_probe`（applied>0 → `APPLIED`；否则
  `ABSENT（旁路缺席：判定 H1 不可执行）`）。

**复测（三臂，结果文件 `daoti/assoc_accuracy_results_v2.json`）**：用更新后的脚本
在真实 sidecar 上重跑 P7.4 全量：A/P/E 三臂指标与 P7.4 原结果**逐项完全一致**
（root_recall 46.7%、root 正确 14、同链率 53.3%、noise 32.3%、空态 100%），确认
P8.2c 字段零行为影响且 P7.4 结论可复现；三臂 `weak_bypass_distribution` 均为
`{"unavailable": 16}`，`bypass_active_probe = ABSENT`——**16 个弱匹配查询在全量
评测上全部确认旁路缺席**。

**关键收益**：bge 权重就绪后，用**同一脚本同一命令**重跑即可——`weak_bypass_distribution`
自动出现 `applied` 计数、`bypass_active_probe` 自动翻转为 `APPLIED`，H1 才可执行；
无需改写任何评测逻辑。当前 16/16 `unavailable` 明确标识"不能判定旁路有效性"。

**P8.3 判据脚本化（P8.2f，2026-09-09）**：把 10.4 的 H1-H4 判定也固化为脚本——
`daoti/_assoc_accuracy_eval.py` 新增：
- `--base-b <url>` 旁路臂 B（ml 构建 + bge 权重 + `--mode smart` 启动的第 4 个
  sidecar，可选）；
- `evaluate_h()`：H1（A 臂 miss 查询中 B 臂救回 ≥10）、H2（A 臂 root 正确查询在
  B 不退化 + 无关联查询仍诚实空态）、H3（阈值 0.55 引擎固定，脚本不做调整）、
  H4 判定；**前置**：B 臂弱匹配查询须至少一个 `semantic_bypass == "applied"`，
  否则返回 `BLOCKED`（10.5 纪律）；
- 输出 `criteria_h`（无 B 臂时 `NOT_RUN`）。

**回归验证**：三臂复测（结果 `daoti/assoc_accuracy_results_v3.json`）——G1-G5 与
P7.4/P8.2e **逐项完全一致**（行为零破坏），`criteria_h = NOT_RUN（未提供 --base-b）`
并按预期给出续跑指引。

**一键执行（bge 权重就绪后）**：放入 `models/BAAI--bge-small-zh/model.safetensors`
→ 以 `--mode smart` 启动第 4 个 sidecar（隔离 db-path/端口）→ 评测命令追加
`--base-b http://127.0.0.1:<port>` → 脚本自动输出 `criteria_h` 的 H1-H4 判定。
**无需改写任何脚本**；当前已实现全部判据逻辑并回归验证。

**P8.3 前置纪律（追加）**：评测脚本必须对每个弱匹配查询断言
`semantic_bypass == "applied"`，才允许计入 H1 救回率分母；任何 `unavailable`
（旁路缺席）的查询不计入、不当作"旁路无效"证据。此自检在 bge 权重就绪后自动
生效，杜绝重蹈 P7.4/探针的误读。

### 10.6 当前状态与后续

P8 处于**前置可行性阶段收尾**：
- 归因（10.1）：16 个 MISS 全在词面门槛之下（14 弱词桥 + 2 孤儿），词面机制覆盖不到；
- 旁路核查（10.2）：语义旁路机制完备但从未被真实启用（默认无 ml + bge 权重缺失）；
- 前置实测（10.3）：`ml` feature 构建可行，但 **bge-small-zh 权重在当前离线环境不可获取**
  （网络隔离 + 本地无备份），备选模型探针无效弃用；
- P8.3（旁路救回率实测）因此**外部阻塞**，判据 H1-H4 保持锁定待用。

**执行门槛（需其一满足才能续跑 P8.3）**：
1. 提供 bge-small-zh `model.safetensors` 离线文件（放入
   `models/BAAI--bge-small-zh/`），或
2. 开放可用的网络通道下载权重。

门槛满足后按 10.4 判据续跑，无需重写判据。在此之前，`LRC_ASSOC_PATH_SCORE` /
`LRC_ASSOC_EDGE_FEEDBACK` 继续默认关闭，联想中心维持现状。
