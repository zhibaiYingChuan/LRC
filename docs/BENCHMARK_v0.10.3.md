# Loong Recall v0.10.3 基准测试报告 — LongMemEval 检索精度

- 测试版本：v0.10.3
- 测试日期：2026-11-09
- 测试环境：Windows 10 专业版，Intel Core i5-9600KF @ 3.70GHz（6 核 6 线程），24 GB RAM
- 被测二进制：`cargo build --release --bin code-memory-server --features server`（版本校验 0.10.3，**不含 `ml` 特性**）
- 检索模式：`--mode fast`（TF-IDF + 洛书几何）
- 评测隔离：`--stdio` 模式，每实例独立空库，`--disable-synthesis` 禁用后台合成；不占用 3099 / 3111 端口，不触碰生产记忆库
- 数据集：LongMemEval-S（`longmemeval_s_cleaned.json`，500 实例）

---

## 〇、结论摘要

| 指标 | v0.10.3（232 实例） | v0.5.6（470 实例） | 变化 |
|---|---:|---:|---:|
| Session Recall@10 | **92.24%** | 85.74% | +6.50pp |
| Turn Recall@10 | **83.19%** | 61.70% | +21.49pp |
| Session MRR | **0.8221** | 0.5499 | +0.2722 |
| Turn MRR | **0.5546** | 0.2864 | +0.2682 |
| 平均注入耗时 / 实例 | 0.417s | 2.789s | 快 6.7× |
| 平均检索耗时 / 实例 | 0.085s | 3.494s | 快 41× |

**一句话**：在 fast + TF-IDF 口径下，v0.10.3 的 Session Recall@10 从 85.74% 提升到 92.24%，Turn Recall@10 从 61.70% 提升到 83.19%，同时注入与检索都显著变快。

---

## 一、方法

### 1.1 样本

- 从 LongMemEval-S 的 500 实例中取**前 250 条**（500 的中位数规模）
- 跳过 18 条 abstention（无标准答案的拒答类），**有效实例 232 条**
- 3 分片并行，每片约 77–78 条

### 1.2 公平性约束

沿用 `lrc_fair_eval_v3.py` 的公平评估口径：

1. 不利用任何 ground truth（`has_answer`、`answer_session_ids` 均不参与）
2. 所有记忆 `importance` 统一为 5，不做差异化加权
3. `tags` 只含自然属性（`session_id` / `date` / `role` / `turn_idx` / `level`）
4. 注入策略：会话级 + Turn 级，统一 importance
5. Top-K = 10

### 1.3 为什么用非 ML 构建

ML 构建（`--features server,ml`）在**写入时强制做语义编码**，实测约 **404 秒/实例**（`bge-base-zh`，6 核），250 实例即使 3 分片也需 9 小时以上，不具备可重复性。

非 ML 构建跳过写入期语义编码，注入降到 **0.417 秒/实例**，3 分钟完成全量。

因此本报告测的是 **TF-IDF 快路**，不是 ML 语义通路。两者能力边界不同，数字不可互换。

---

## 二、总体结果

| 指标 | 值 |
|---|---:|
| 有效实例 | 232 |
| Session Recall@10 | **92.24%** |
| Turn Recall@10 | **83.19%** |
| Session MRR | 0.8221 |
| Turn MRR | 0.5546 |
| 平均注入耗时 / 实例 | 0.417s |
| 平均检索耗时 / 实例 | 0.085s |
| 平均记忆数 / 实例 | 583.7 |
| 错误数 | 0 |

### 分片一致性

| 分片 | 实例数 | Session R@10 | Turn R@10 | Session MRR | Turn MRR |
|---|---:|---:|---:|---:|---:|
| s0 | 78 | 0.9231 | 0.8205 | 0.8160 | 0.5361 |
| s1 | 77 | 0.9091 | 0.8442 | 0.7999 | 0.5672 |
| s2 | 77 | 0.9351 | 0.8312 | 0.8505 | 0.5608 |

三片波动在 ±1.3pp 以内，无异常分片。

---

## 三、按题型

| 题型 | 数量 | Session R@10 | Turn R@10 |
|---|---:|---:|---:|
| temporal-reasoning | 17 | **100.00%** | 88.24% |
| single-session-user | 64 | 96.88% | 89.06% |
| multi-session | 121 | 95.87% | 88.43% |
| single-session-preference | 30 | **63.33%** | **46.67%** |

**短板**：`single-session-preference`（偏好类）明显低于其他三类，Session 63.33%、Turn 46.67%。偏好类问题的表述通常更口语、更短，与记忆原文的字面重合度低，TF-IDF 快路天然吃亏。这是下一步最值得优化的方向。

---

## 四、与历史口径对照

| 口径 | 版本 | 样本 | 模式 | Session R@10 | Turn R@10 |
|---|---|---|---:|---:|---:|
| 本报告 | v0.10.3 | 232 | fast + TF-IDF | 92.24% | 83.19% |
| 量级参照 | v0.5.6 | 470 | fast + TF-IDF | 85.74% | 61.70% |
| 官网主指标 | v0.9.7 | 30（分层） | deep + bge-base-zh | 90.00% | — |

**三套数字不能相加、不能互相替代、不能混着说**：

- 232 实例是中位数抽样，470 实例是接近全量，30 实例是分层小样本
- fast 是 TF-IDF 快路，deep 是 ML 语义通路
- 不同版本的编码器、合并策略、召回折叠逻辑都不同

历史报告保留在私有核心仓库 `core/lrc-algo/benchmarks/`，作为对照：

- `V0.9.5_BENCHMARK_REPORT.md`
- `V0.9.7_BENCHMARK_REPORT.md`
- `comparative/LRC_LONGMEMEVAL_REPORT.md`
- `comparative/LRC_BENCHMARK_SUMMARY.md`

---

## 五、已知问题：ML 构建写入慢

本次测试暴露一个产品级问题，与检索精度无关，但影响导入场景：

| 构建 | 写入期语义编码 | 注入耗时 / 实例 | 相对倍率 |
|---|---|---:|---:|
| `--features server`（非 ML） | 无 | 0.417s | 1× |
| `--features server,ml` | 有（洛书向量 + 八卦分类 + 后台语义向量） | 404s | ~970× |

**影响**：日常单条写入感知不到；批量导入历史记忆（如 LongMemEval 注入、迁移旧库）会非常慢。

**状态**：已定位到写入路径的两处编码调用，优化方案评估中，修复后将以 500 实例全量重测。

---

## 六、复现步骤

```powershell
# 1. 构建非 ML 二进制
cargo build --release --bin code-memory-server --features server

# 2. 运行 250 实例（3 分片）
cd G:\LongMemEval
powershell -NoProfile -ExecutionPolicy Bypass -File .\run_v0103_fast_250.ps1

# 3. 结果
#    G:\LongMemEval\results\v0103_fast_250_s{0,1,2}.log
```

评测脚本：`G:\LongMemEval\lrc_fair_eval_v3.py`（公平评估 v3）

---

## 七、局限

1. **非 ML 构建**：本报告不代表 ML 语义通路的能力；deep 口径需另跑。
2. **中位数抽样**：232 实例不等于 500 全量，稀有题型的权重与全量不同。
3. **单机单次**：延迟受测试机器影响，不作为跨机器对比依据。
4. **无 LLM 查询翻译**：本次未启用 `--llm-api`，纯关键词 + 洛书几何。
5. **偏好类偏弱**：`single-session-preference` 仅 63.33%，会拉低整体均值。

---

## 八、变更记录

| 日期 | 变更 |
|---|---|
| 2026-11-09 | 建立；v0.10.3 fast + TF-IDF，232 有效实例 |
