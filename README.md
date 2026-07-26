# Loong Recall (LRC)

**给 AI 装上记忆的本地服务 — 跨会话记住你的代码和决策。**

[![License](https://img.shields.io/badge/Code-Apache%202.0-blue.svg)](LICENSE_CODE)
[![License](https://img.shields.io/badge/Engine-DaoTi%20Research%20License-red.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

---

## 它解决什么问题

| 痛点 | LRC 的方案 |
|------|-----------|
| AI 每次对话都忘记之前的约定 | `remember` / `recall` — 写一次，永久记住 |
| 想改某个功能但不知道代码在哪 | `search_code` — 关键词定位，无需手动翻文件 |

**一行话说清楚**：给 AI 装个记事本，但它是活的 — 跨会话、跨 IDE、本地运行、零云端依赖。

---

## 基准测试评分

6 次标准基准测试，对比 BM25 基线：

| 基准测试 | 评估目标 | TF-IDF MRR@10 | LLM 增强 MRR@10 | vs BM25 |
|:--------:|:--------:|:-------------:|:----------------:|:-------:|
| [MS MARCO](benchmarks/reports/LRC_MSMARCO_REPORT.md) | 关键词检索 | 0.7749 | **0.8895** | +383% |
| [Natural Questions](benchmarks/reports/LRC_NQ_REPORT.md) | 自然语言问题 | 0.5389 | **0.8016** | +163% |
| [HotpotQA](benchmarks/reports/LRC_HOTPOTQA_REPORT.md) | 多跳推理 | 0.7964 | **0.9383** | +48% |
| [FiQA](benchmarks/reports/LRC_FIQA_REPORT.md) | 金融领域 | 0.2729 | **0.4453** | +89% |
| [LRC 原生基准](benchmarks/reports/LRC_NATIVE_BENCHMARK_TFIDF.md) | 综合记忆能力 | **11/11 PASS** (评分 0.94) | [9/11 PASS](benchmarks/reports/LRC_NATIVE_BENCHMARK_LLM.md) (评分 0.79) | — |
| [LongMemEval](benchmarks/reports/LRC_LONGMEMEVAL_REPORT.md) | 长时记忆 | Session Recall@10 = **85.74%** | Turn Recall@10 = **61.70%** | — |

> 完整报告见 [基准测试汇总](benchmarks/reports/LRC_BENCHMARK_SUMMARY.md)。

---

## 快速开始

### 方式一：下载桌面端（推荐）

1. 前往 [Releases](https://github.com/zhibaiYingChuan/LRC/releases) 下载最新安装包
2. 双击安装，启动 LRC Desktop
3. 按向导选择项目、配置 LLM（可选）、连接 AI 工具
4. 重启 IDE，AI 自动发现 12 个 MCP 工具

> 桌面端自动完成所有配置：检测 AI 工具、写入 MCP 配置、写入 AI 规则文件。

### 方式二：从源码编译

```bash
git clone https://github.com/zhibaiYingChuan/LRC.git
cd LRC
cargo build --release --features server
./target/release/code-memory-server --src-dir ./src --port 3099
```

如需离线语义搜索：`cargo build --release --features server,ml`（首次下载模型 ~500MB）。

### v0.6.0 通用语义引擎（新）

v0.6.0 将默认嵌入模型从 CodeBERT 切换为 **BGE-small-zh**（中文用户开箱最优）或 **MiniLM-L6-v2**（英文环境），并支持本地嵌入完成记忆结晶，无需 LLM API 即可享受记忆融合能力。

**模型管理 CLI**（v0.6.0 新增）：

```bash
# 列出本地已下载模型
code-memory-server model list

# 下载模型（默认使用 hf-mirror.com 国内镜像）
code-memory-server model download BAAI/bge-small-zh

# 切换默认模型
code-memory-server model use BAAI/bge-small-zh

# 删除模型文件
code-memory-server model remove BAAI/bge-small-zh
```

**镜像源配置**：

| 镜像源 | 配置方式 | 适用场景 |
|--------|---------|---------|
| HF-Mirror（默认） | `HF_ENDPOINT=https://hf-mirror.com` | 国内用户首选 |
| ModelScope | `LRC_MODEL_MIRROR=modelscope` | HF 镜像不可达时备用 |
| 自动选择 | `LRC_MODEL_MIRROR=auto` | 优先 HF-Mirror，失败回退 ModelScope |

下载失败时自动重试 3 次（2s/4s/8s 指数退避），3 次均失败后输出手动下载指引并降级到 TF-IDF 模式。

**推荐模型对比**：

| 模型 | 维度 | 大小 | 推荐场景 |
|------|------|------|---------|
| BAAI/bge-small-zh | 512 | ~100MB | 中文默认（v0.6.0 推荐） |
| sentence-transformers/all-MiniLM-L6-v2 | 384 | ~80MB | 英文默认 |
| BAAI/bge-base-zh | 768 | ~400MB | 中文高精度 |
| multilingual-e5-small | 384 | ~120MB | 多语言通用 |

---

## 12 个 MCP 工具

| 类别 | 工具 | 用途 |
|------|------|------|
| **代码搜索** | `search_code` `codebase_stats` | 关键词定位代码、查看索引状态 |
| **记忆管理** | `remember` `recall` `forget` `update_memory` `list_memories` `memory_stats` `archive` `correct_memory` `recall_enhanced` `dao_metrics` | 写入、检索、删除、更新、列表、统计、归档、修正、增强检索、健康指标 |

---

## 性能

| 规模 | 检索延迟 | 内存占用 |
|------|---------|---------|
| 万条记忆 | < 5ms | < 10 MB |
| 十万条记忆 | < 15ms | < 10 MB |
| 百万条记忆 | < 30ms | < 10 MB |

> 基于 Fast Match 模式（纯 Rust，零外部依赖），消费级 CPU，未使用 GPU。

---

## 隐私

**LRC 是纯本地工具。你的代码和记忆永远不会主动离开你的机器。**

- 不收集遥测、不埋点、不上报
- 源代码索引驻留内存，不写磁盘
- 记忆数据存储在 `~/.loong-recall/` 本地目录
- 仅当你配置 `--llm-api` 时，查询文本（非源代码）会发送到你的 LLM API

---

## 文档导航

| 文档 | 说明 |
|------|------|
| [用户使用说明书](docs/USER_GUIDE.md) | 详细使用指南与 AI 调用规则 |
| [变更日志](CHANGELOG.md) | 版本变更记录 |
| [基准测试汇总](benchmarks/reports/LRC_BENCHMARK_SUMMARY.md) | 6 次基准测试完整报告 |
| [性能测试指南](docs/BENCHMARK.md) | 如何复现性能测试 |
| [使用场景](docs/USE_CASES.md) | 典型应用场景与最佳实践 |
| [Smart Match 离线安装](docs/OFFLINE_MODEL_GUIDE.md) | 内网/离线环境模型安装 |

---

## License

- 代码部分：[Apache 2.0](LICENSE_CODE)
- 检索引擎：[DaoTi Research License](LICENSE)
