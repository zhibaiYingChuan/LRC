# LongMemEval-S fast 232 复现包

## 口径

| 项 | 值 |
|---|---|
| 数据集 | `longmemeval_s_cleaned.json`（HuggingFace `xiaowu0162/longmemeval-cleaned`） |
| 实例 | 500 取前 250，跳过 18 条 abstention → **232 有效** |
| 分片 | 3 片，`index % 3`；每片独立数据目录 |
| Top-K | 10 |
| 注入 | 会话级 + Turn 级；importance 统一 5；不使用 `has_answer` / `answer_session_ids` 做注入 |
| 模式 | `--mode fast`（TF-IDF/BM25 词面 + 洛书几何） |
| 合成 | `--disable-synthesis` |
| 构建 | **非 ML**：`cargo build --release --features server` |
| LLM | 不指定（纯关键词匹配）；指定后是另一套口径 |

## 复现

```bash
# 1) 下载数据（自动校验 SHA256）
python eval/longmemeval/scripts/download_data.py

# 2) 构建非 ML sidecar
cargo build --release --features server

# 3) 跑 232（3 分片并行）
pwsh eval/longmemeval/scripts/run_fast_232.ps1 \
  -Exe target/release/code-memory-server.exe

# Windows PowerShell 5 也可直接运行：
# powershell -ExecutionPolicy Bypass -File eval/longmemeval/scripts/run_fast_232.ps1
```

结果日志写到 `eval/longmemeval/logs/fast_232_s{0,1,2}.log`，脚本末尾会打印加权汇总。

## 预期结果

见 [`expected/fast_232.json`](expected/fast_232.json)：

| 指标 | 值 |
|---|---|
| Session Recall@10 | **92.24%** |
| Turn Recall@10 | **83.19%** |
| Session MRR | **0.8221** |
| Turn MRR | **0.5546** |
| 分片波动 | ±1.3pp |

题型短板：`single-session-preference` Session 63.33% / Turn 46.67%（30 条）。

历史三份分片日志在 [`logs/`](logs/)，可与 `expected/fast_232.json` 逐项核对。

## 重要边界

- **fast ≠ 三路融合**：这里测的是词面快路 + 洛书几何，不是 README 主打的 deep/语义融合。
- **232 / 470 / 30 不能相加、不能互相替代**：232 是 fast 公平口径；470 是另一套 fast 全量口径；30 是 deep 分层口径。
- **LongMemEval 只测检索**：不测 `absorb` 写入质量、合成污染、错忆代价。
- **必须用非 ML 构建**：ML 构建写入时会强制语义编码，fast 口径下会出现 404s/实例的假慢；脚本默认 `--max-inject-seconds 30` 会在超时时直接失败并提示。
- 机器/线程数变化会影响耗时，不影响召回；若召回偏离超过 ±1.3pp，先核对数据集 SHA256 与分片清单。

## 文件

| 路径 | 作用 |
|---|---|
| `scripts/lrc_fair_eval_v3.py` | 评测器（公平注入 + 检索 + 指标） |
| `scripts/run_fast_232.ps1` | 三分片启动与加权汇总 |
| `scripts/make_splits.py` | 生成可审计分片清单 |
| `scripts/download_data.py` | 数据集下载 + SHA256 校验 |
| `splits/fast_232.json` | 250 条实例、abstention、分片、题型分布 |
| `configs/fast_232.json` | 本次口径的完整配置 |
| `expected/fast_232.json` | 预期指标、容差、分题型结果、caveats |
| `logs/v0103_fast_250_s*.txt` | 历史三分片原始日志 |
| `reports/` | 历史 470 公平对比与 v0.5.5 最终报告（仅参考，不与 232 混用） |
