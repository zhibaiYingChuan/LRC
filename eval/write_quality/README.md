# absorb 写质量评测

> 状态：**骨架 + 20 条最小标注集**。目前没有对外数字，先建立可重复的测量方法。

LongMemEval 只测检索召回，不测写入。这个评测补的是另一半：`absorb`
到底该写的写了没有、不该写的有没有乱写、重复沉淀会不会膨胀。

## 一、测什么

| 指标 | 定义 |
|---|---|
| 精确率 | 被判为“应写入”的用例中，标注确实为应写入的比例 |
| 召回率 | 标注为应写入的用例中，`absorb` 实际判定写入的比例 |
| 特异度 | 噪声用例中，被正确拒绝（不写）的比例 |
| 误吸收率 | 噪声用例中被写成记忆的比例（= 1 - 特异度） |
| 内容命中率 | 实际写入内容包含预期关键词的比例 |
| 误吸收 | 闲聊 / 提问 / 表格 / 标题 / 纯链接 / 删除意图被写成记忆 |
| 幂等率 | 同一结论连续沉淀两次，第二次被去重（不新增）的比例 |

## 二、用例

`cases/absorb_cases.jsonl`，每行一条：

```json
{
  "id": "pos-decision-001",
  "category": "decision",
  "text": "决定数据库选 PostgreSQL，因为需要事务和行级锁。",
  "expect_absorb": true,
  "expect_keywords": ["PostgreSQL"],
  "forbid_keywords": []
}
```

当前 20 条：12 条正例（决策 / 约束 / 结论 / 报错根因 / 偏好），
8 条负例（闲聊 / 提问 / 表格 / 标题 / 纯链接 / 临时状态 / 删除意图 / 代码块）。

## 三、运行

```bash
# 1) 构建 sidecar（fast 口径用非 ML 即可）
cargo build --release --features server

# 2) 跑评测（自动启动独立 sidecar + 临时数据目录，跑完删除）
python eval/write_quality/scripts/run_write_quality.py \
  --exe target/release/code-memory-server.exe
```

只做判定、不写库：

```bash
python eval/write_quality/scripts/run_write_quality.py \
  --exe target/release/code-memory-server.exe --skip-duplicate-probe
```

结果写入 `results/write_quality_<时间>.json`（已被 `.gitignore` 排除），
终端同时打印汇总。

## 四、口径与边界

- 判定用 `absorb` 的 `dry_run` 结果（`would_write` / `written` / `duplicate` / `no_op` / `failed` / `dropped`），
  不引入 LLM judge，避免评测本身不可复现。
- 当前 20 条用例的 `forbid_keywords` 均为空：误吸收由“负例是否被判写入”衡量；
  该字段预留给后续“内容级污染”用例。
- 幂等探针会真实写库，但只写入临时目录；不会接触 3099 / 3111 实例。
- 20 条是**最小可用集**，不是全分布；新增用例必须同时给正负例，避免只调召回。
- 不覆盖：合成污染（synthesis）、长期错忆代价、跨会话冲突修正。这些需要单独设计。
- 没有外部基线（mem0 / Zep / Letta）的同口径写质量对比；做不了就写“暂无”。
