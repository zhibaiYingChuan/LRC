# LRC 基准与口径

> 原则：**不同协议的数字不能相加、不能互相替代**。每行必须带协议、模式、版本、构建。
> 可复现的评测资产在 [`eval/longmemeval/`](eval/longmemeval/README.md)。

## 一、公开数字总表

| 协议 | 模式 | 版本 | 有效 N | Session R@10 | Turn R@10 | Session MRR | Turn MRR | 可复现 |
|---|---|---|---|---|---|---|---|---|
| LongMemEval-S 前 250（跳过 abstention） | **fast**（TF-IDF + 洛书几何） | v0.10.3 | **232** | **92.24%** | **83.19%** | **0.8221** | **0.5546** | ✅ 本仓库 `eval/longmemeval/` |
| LongMemEval-S 全量 | fast（v3 公平：统一 importance=5） | v0.5.6 | 470 | 85.74% | 61.70% | 0.5499 | 0.2864 | ⚠️ 历史报告，脚本未固化 |
| LongMemEval-S 全量 | fast（历史协议：重要性加权，注入未固化） | v0.5.5 | 470 | 95.53% | 89.36% | 0.7864 | 0.6385 | ⚠️ 历史报告，协议不可比 |
| LongMemEval-S 分层 30 | deep（ML） | v0.9.7 | 30 | 90.0% | 63.3% | 0.683 | — | ❌ 未入复现包；历史日志有 1 条超时 |
| LongMemEval-S 前 250 | deep / fusion | — | 232 | **未测** | **未测** | **未测** | **未测** | — |

**读法**：

- 92.24% 是 **fast** 口径，证明的是词面快路 + 洛书几何，不是三路融合/深度语义。
- 90.0% 是 **30 实例 deep** 口径，样本小、未固化，不能拿来替代 232。
- 95.53% 是 **v0.5.5 历史协议**（重要性加权、注入策略未固化），与 v3 公平的 85.74% 不是同一协议；
  两者不能并列成“提升”，更不能与 232 相加或互相替代。
- 232 / 470 / 30 三套数字不能相加、不能互相替代。

## 二、232 fast 口径

| 项 | 值 |
|---|---|
| 数据集 | `longmemeval_s_cleaned.json`，SHA256 `d6f21e…a442` |
| 选取 | 500 取前 250，跳过 18 条 abstention → 232 |
| 分片 | 3 片，`index % 3`，独立数据目录 |
| Top-K | 10 |
| 注入 | 会话级 + Turn 级；importance 统一 5；不使用 ground truth |
| 构建 | `cargo build --release --features server`（**不带 ml**） |
| 合成 | 禁用 |
| LLM | 不指定（纯关键词）；指定后是另一套口径 |
| 波动 | 三分片 ±1.3pp |

复现：

```bash
python eval/longmemeval/scripts/download_data.py
cargo build --release --features server
pwsh eval/longmemeval/scripts/run_fast_232.ps1 -Exe target/release/code-memory-server.exe
```

预期输出与分题型结果见 [`eval/longmemeval/expected/fast_232.json`](eval/longmemeval/expected/fast_232.json)。

## 三、已知短板

- **偏好类**：232 fast 中 `single-session-preference` Session 63.33% / Turn 46.67%（30 条），是最大短板。
- **deep 写入成本**：ML 构建写入会强制语义编码；历史 deep 运行约 923~974s/实例，232 deep 不可行。fast 口径必须用非 ML 构建，否则会出现 404s/实例假慢。
- **只测检索**：LongMemEval 不测 `absorb` 写入质量、合成污染、错忆代价、端到端答题。
- **无外部基线**：尚未与 mem0 / Zep / Letta / Graphiti 做同口径对比。

## 四、写质量（非检索）

LongMemEval 不覆盖写入。`absorb` 的判定精确率 / 召回 / 误吸收 / 幂等率
用 [`eval/write_quality/`](eval/write_quality/README.md) 的 20 条标注用例测量；
目前是**方法已固化、基线未跑**，不对外给数字。

## 五、下一步要补的数字

1. 232 的 **deep / fusion** 同口径结果（先解决写入成本）。
2. 写质量基线：`absorb` 精确率/召回、误吸收率、幂等率（跑 `eval/write_quality/`）。
3. 合成污染率、错忆代价。
4. 端到端：写入 → 检索 → 答题，而不只是 Recall@10。
5. 外部基线：同语料、同 Top-K、同 judge；做不了就明确写“暂无”。
