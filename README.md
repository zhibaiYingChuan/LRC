# Loong Recall (LRC)

**给 AI 装上记忆的本地服务 — 跨会话记住你的代码和决策。**

[![License](https://img.shields.io/badge/Code-Apache%202.0-blue.svg)](LICENSE_CODE)
[![License](https://img.shields.io/badge/Engine-DaoTi%20Research%20License-red.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.80%2B-orange.svg)](https://www.rust-lang.org)

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
>
> **数据版本说明**：上述数据基于 v0.5.6 基准测试（2026-06-23），当前 v0.8.7 版本的检索引擎已有演进，最新数据以重新测试为准。

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

### 通用语义引擎

v0.6.0 将默认嵌入模型从 CodeBERT 切换为 **BGE-small-zh**（中文用户开箱最优）或 **MiniLM-L6-v2**（英文环境），并支持本地嵌入完成记忆结晶，无需 LLM API 即可享受记忆融合能力。

**模型管理 CLI**：

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
| BAAI/bge-small-zh | 512 | ~100MB | 中文默认推荐 |
| sentence-transformers/all-MiniLM-L6-v2 | 384 | ~80MB | 英文默认 |
| BAAI/bge-base-zh | 768 | ~400MB | 中文高精度 |
| multilingual-e5-small | 384 | ~120MB | 多语言通用 |

### v0.6.0 龙忆设计系统 v1.0（UI 重构）

v0.6.0 同步完成 LRC 全案界面重构，基于"形现代，意古风"设计理念，构建完整的龙忆设计系统 v1.0。

**核心设计资源**（位于 `static/` 目录）：

| 资源 | 文件 | 说明 |
|------|------|------|
| 色阶与排版 Token | [colors_and_type.css](static/colors_and_type.css) | 6 组色阶（墨韵/宣纸/金色/玉色/朱砂/水蓝，每色 10 级）+ 语义别名 + 排版/间距/圆角/阴影/动效 |
| 全局组件库 | [components.css](static/components.css) | 按钮（5 种变体 + 3 种尺寸 + 洛书加载动画）、卡片（含记忆类型色条）、输入框、模态框、侧边栏 |
| SVG 图标集 | [static/assets/icons/](static/assets/icons) | 15 个极简线性图标（24x24px 栅格） |
| SVG Logo 集 | [static/assets/logo/](static/assets/logo) | 4 种 Logo 形态（主标/横版/纵版/纯文字） |

**记忆类型色条系统**：信任中心 6 张卡片按记忆类型添加左侧色条，实现"一眼可辨"的视觉分组。

| 记忆类型 | 色条颜色 | CSS 类 |
|---------|---------|--------|
| fact（事实） | 玉色 | `card-memory-fact` |
| preference（偏好） | 金色 | `card-memory-preference` |
| decision（决策） | 朱砂 | `card-memory-decision` |
| code_context（代码上下文） | 水蓝 | `card-memory-code` |
| conversation（对话） | 墨韵 | `card-memory-conversation` |

**已实现功能**：

- **预设场景模板**：4 套场景模板选择器（个人笔记/项目管理/学习助手/编程助手），位于仪表盘顶部。
- **结晶历史时间线**：从审计日志加载结晶事件并渲染为成长轨迹时间线。

> 更多功能规划详见 [产品路线图](docs/PRODUCT_ROADMAP_v1.0.md)。

**暗色模式**：通过 `prefers-color-scheme: dark` 自动适配系统暗色主题，所有色值使用 CSS 变量，无硬编码颜色。

> 设计资源位于 `static/` 目录，详见上方表格中的文件引用。

---

## 12 个 MCP 工具

| 类别 | 工具 | 用途 |
|------|------|------|
| **代码搜索** | `search_code` `codebase_stats` | 关键词定位代码、查看索引状态 |
| **记忆管理** | `remember` `recall` `forget` `update_memory` `list_memories` `memory_stats` `archive` `correct_memory` `recall_enhanced` `dao_metrics` | 写入、检索、删除、更新、列表、统计、归档、修正、增强检索、健康指标 |

---

## 性能

基于 BEIR 标准基准测试（500 文档 / 100 查询），TF-IDF 模式平均检索延迟 13-21ms，LLM 增强模式平均 1.0-1.3s（含 LLM API 调用）。LongMemEval（470 实例）Session Recall@10 = 85.74%。

> 上述数据来自 v0.5.6 基准测试（2026-06-23），基于 Fast Match 模式（纯 Rust，零外部依赖），消费级 CPU，未使用 GPU。详见 [基准测试汇总](benchmarks/reports/LRC_BENCHMARK_SUMMARY.md)。

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
