# 复现指南（LongMemEval-S fast 232）

目标：第三方用**本仓库源码构建的非 ML sidecar** + 本仓库评测资产，就能跑出与报告一致的数字（±1.3pp）。

> 发布安装包里的 sidecar 是 `server,ml` 构建，**不能用于 fast 232 复现**；
> fast 口径必须单独构建 `--features server`（见第二节）。

## 一、环境

| 项 | 要求 |
|---|---|
| OS | Windows 10/11（当前脚本为 PowerShell；macOS/Linux 可用 `pwsh`） |
| Rust | 1.99.0（与发布流水线一致） |
| Python | 3.10+，仅标准库 + `numpy` |
| 磁盘 | ≥ 1.5GB（数据集 277MB + 数据目录 + 二进制） |
| 内存 | ≥ 8GB（3 分片 × sidecar） |

## 二、构建（fast 口径必须非 ML）

```bash
cargo build --release --features server
```

产物：`target/release/code-memory-server`（Windows 为 `.exe`）。

> **不要带 `ml` / `cuda`**：ML 构建写入时会强制语义编码，fast 口径下会出现 404s/实例的假慢；评测脚本默认在单实例注入超过 30s 时直接失败并提示。

## 三、数据

```bash
python eval/longmemeval/scripts/download_data.py
```

- 来源：HuggingFace `xiaowu0162/longmemeval-cleaned`（hf-mirror.com）。
- 自动校验 SHA256：
  - `longmemeval_s_cleaned.json` = `d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442`
  - `longmemeval_oracle.json` = `821a2034d219ab45846873dd14c14f12cfe7776e73527a483f9dac095d38620c`
- 输出到 `eval/longmemeval/data/`。

分片清单（250 条、abstention、分片号、题型）已固化在
[`eval/longmemeval/splits/fast_232.json`](eval/longmemeval/splits/fast_232.json)，
可用来核对“到底跑了哪 232 条”。

## 四、运行

```bash
pwsh eval/longmemeval/scripts/run_fast_232.ps1 \
  -Exe target/release/code-memory-server.exe
```

Windows PowerShell 5：

```powershell
powershell -ExecutionPolicy Bypass -File eval/longmemeval/scripts/run_fast_232.ps1 `
  -Exe target/release/code-memory-server.exe
```

脚本会启动 3 个分片，日志写到 `eval/longmemeval/logs/fast_232_s{0,1,2}.log`，末尾打印加权汇总。

## 五、预期结果

| 指标 | 预期 | 容差 |
|---|---|---|
| Session Recall@10 | 92.24% | ±1.3pp |
| Turn Recall@10 | 83.19% | ±1.3pp |
| Session MRR | 0.8221 | ±0.02 |
| Turn MRR | 0.5546 | ±0.02 |

完整分题型结果与 caveats 见
[`eval/longmemeval/expected/fast_232.json`](eval/longmemeval/expected/fast_232.json)。

## 六、二进制校验与可复现构建

发布产物应附带：

```bash
# Windows
Get-FileHash .\lrc-desktop-vX.Y.Z-windows-x86_64-setup.exe -Algorithm SHA256
# macOS / Linux
shasum -a 256 lrc-desktop-vX.Y.Z-...
```

> 注意：安装包内的 sidecar 是 `server,ml` 构建，其哈希只能证明安装包一致，
> **不能用于 fast 232 复现**。评测用的非 ML sidecar 应单独记录
> `cargo build --release --features server` 的产物哈希。

构建信息建议记录：

```json
{
  "commit": "<git rev-parse HEAD>",
  "rustc": "<rustc -V>",
  "features": "server",
  "target": "x86_64-pc-windows-msvc",
  "binary_sha256": "<sha256>",
  "dataset_sha256": "d6f21e...a442"
}
```

可复现构建要点：固定 Rust 1.99.0、`--locked`、同一 target、同一 feature；不要在构建时注入本机路径或时间戳。桌面端安装包还受 Tauri/NSIS 打包影响，第三方至少应能复现 **sidecar 二进制哈希** 与 **评测数字**。

## 七、常见问题

**注入很慢（>30s/实例）**
：用了 ML 构建。改用 `--features server` 重新构建；或显式 `-MaxInjectSeconds 0` 关闭护栏（不推荐，数字会不可比）。

**数据集 SHA256 不匹配**
：下载到了不同版本/截断文件。删除 `eval/longmemeval/data/longmemeval_s_cleaned.json` 后重跑 `download_data.py`。

**结果偏离超过容差**
：先核对数据集哈希、分片清单、Top-K、importance 是否统一为 5；再确认是否误开了 `ml` 或指定了 LLM API。

**端口/进程冲突**
：评测使用 stdio 模式，每个实例独立进程；确保没有残留 `code-memory-server` 进程占用数据目录。

## 八、本复现包不覆盖什么

- 不覆盖 deep / fusion 232 口径（尚未固化；ML 写入成本是前置阻塞）。
- 不覆盖写入质量（`absorb` 过滤、合成污染、错忆代价）。
- 不覆盖端到端答题（LongMemEval 的 LLM 问答环节不属于 LRC 检索职责）。
- 不覆盖外部基线（mem0 / Zep / Letta / Graphiti）。
