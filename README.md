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

## 质量与验证

LRC 的核心功能通过 Rust 单元测试、集成测试、前端契约检查和桌面端 CDP 回归门禁持续验证。

> 性能数据会随版本、硬件和配置变化，发布前以当前版本的 CI 结果为准。

---

## v0.9.5 新特性

- 全局前端导航与发布资源稳定性修复。
- 发布前真实 CDP 回归门禁。

## v0.9.3 新特性

- 修复 ML 编码器塌缩

| 领域 | 变更 | 价值 |
|------|------|------|
| **自动结晶** | ML 编码器对比度增强（中心化 + softmax），分散八卦分类 | 修复编码塌缩导致稳定版自动结晶长期产出为 0 的根因 |
| **稳定性** | 记忆搜索中文查询按字符截断 | 修复中文搜索触发 Rust panic 导致 sidecar 退出的 P0 崩溃 |

> v0.9.1 特性（含三阶段锁解耦消除 lock_busy、审计 15 类事件接线、解冻调节器等）继续生效。

```text
自动结晶：稳定版 ML 模式原先 3606/3703 条记忆塌缩到同一八卦类别（坤·地），
洛书合成全部被信息增量守卫拦截；v0.9.3 修复后编码分布分散，自动结晶恢复工作。
```

## 快速开始

### 方式一：下载桌面端（推荐）

1. 前往 [Releases](https://github.com/zhibaiYingChuan/LRC/releases) 下载**桌面安装包**（注意文件名，勿下载 CLI 二进制）：
   - Windows：`lrc-desktop-v0.9.5-windows-x86_64-setup.exe`
   - macOS：`lrc-desktop-v0.9.5-macos-arm64.dmg`
   - Linux：`lrc-desktop-v0.9.5-linux-amd64.deb` 或 `lrc-desktop-v0.9.5-linux-x86_64.AppImage`
2. 双击安装，启动 LRC Desktop
3. 按向导选择项目、配置 LLM（可选）、连接 AI 工具
4. 重启 IDE，AI 自动发现 13 个 MCP 工具

> 桌面端自动完成所有配置：检测 AI 工具、写入 MCP 配置、写入 AI 规则文件。
>
> **注意**：Release 中 `lrc-v0.9.4-windows-x86_64.exe` 等文件是 **CLI 命令行工具**（sidecar 二进制），供开发者/脚本调用，**不是安装包**，双击无法安装。安装请使用 `lrc-desktop-*` 开头的安装包。

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

**暗色模式**：通过 `prefers-color-scheme: dark` 自动适配系统暗色主题，所有色值使用 CSS 变量，无硬编码颜色。

> 设计资源位于 `static/` 目录，详见上方表格中的文件引用。

---

## 13 个 MCP 工具

| 类别 | 工具 | 用途 |
|------|------|------|
| **代码搜索** | `search_code` `codebase_stats` | 关键词定位代码、查看索引状态 |
| **记忆管理** | `remember` `batch_remember` `recall` `forget` `update_memory` `list_memories` `memory_stats` `archive` `correct_memory` `recall_enhanced` | 写入、批量写入、检索、删除、更新、列表、统计、归档、修正、增强检索 |
| **系统监控** | `system_health` | 查看系统健康状态 |

---

## 性能

性能表现取决于硬件、数据规模和运行模式。请以当前版本的实际测试结果为准。

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
| [基准测试目录](benchmarks/README.md) | 当前版本基准与外部对比结果 |
| [v0.9.5 基准测试报告](benchmarks/V0.9.5_BENCHMARK_REPORT.md) | 当前版本可复现基准结果 |
| [使用场景](docs/USE_CASES.md) | 典型应用场景与最佳实践 |
| [Smart Match 离线安装](docs/OFFLINE_MODEL_GUIDE.md) | 内网/离线环境模型安装 |

---

## License

- 代码部分：[Apache 2.0](LICENSE_CODE)
- 检索引擎：[DaoTi Research License](LICENSE)
