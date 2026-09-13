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

**启用提交前检查（推荐）**：仓库自带版本控制的 Git 钩子，克隆后执行一次即可启用（提交前自动跑 fmt / clippy / check / test / 算法泄露检测 5 项检查）：

```powershell
.\scripts\enable_git_hooks.ps1          # 等价于 git config --local core.hooksPath .githooks
git config --local --unset core.hooksPath   # 需要撤销时
```

> 性能数据会随版本、硬件和配置变化，发布前以当前版本的 CI 结果为准。

**本地开发链路（桌面端 CDP 调试 / 双实例隔离）**：以下脚本随仓库交付，供本地开发与 CDP 回归测试使用（非生产运行时依赖）：

```powershell
# 1) 启动开发代理（端口 1420，直接服务磁盘 static/，并代理 API 到 sidecar）
#    作用：WebView2 会启发式缓存旧 app.js；经 1420 访问可确保测到磁盘最新前端，并禁用缓存。
python .\scripts\dev-proxy.py 1420 --dev

# 2) 启动隔离的开发版实例（端口 3100 + 独立数据目录，不影响稳定版 3099）
.\scripts\run-dev.ps1              # 或双击 scripts\run-dev.bat
.\scripts\run-dev.ps1 -Build       # 先编译再启动
```

> `tests/frontend/` 下的 CDP 测试（`cdp-regression.js`、`association-dashboard-cdp.js`、`association-desktop-cdp.js`）优先经 `localhost:1420` 验证磁盘最新前端；dev-proxy 未启动时自动回退原地重载并告警（此时不保证验证的是最新前端）。

---

## v0.9.7 稳定版特性

**本版本最大特性：deep 语义通路接入本地高精度 BGE 模型，把"关键词召回"升级为真正的"记忆联想"——即使查询与记忆没有共同关键词，也能沿语义联想链一层层召回。**

### 一、语义驱动的记忆联想（核心）+ 道体协作式感知

传统检索只能找到"字面像"的记忆；LRC v0.9.7 的 deep 通路由本地 BGE 语义模型驱动，实现"想到相关的记忆"。

- **联想链路（已验证有效）**：LRC 检索（fast 关键词 + deep 语义双路 RRF 融合）→ BGE 语义编码把无字面重合的记忆沿语义邻近逐跳召回。生活场景实测显示：查询"今晚吃什么"能联想出"和谁吃 / 在哪吃 / 饮食约束 / 饭后惯例"等无共同关键词的多层记忆，头部方向正确率较历史统计编码器翻倍、链扩散深度 2.2→4.0 层（详见 `benchmarks/V0.9.7_BENCHMARK_REPORT.md` 第三节）。
- **高精度语义底座**：deep 通路默认挂载本地 BGE 语义模型（`BAAI/bge-base-zh`，768 维，中文高精度），把 BERT 隐藏层经洛书投影矩阵映射到 9 维几何空间，替代此前区分度不足的纯统计字符特征编码器。
- **可证明的优越性**：历史统计编码器下 LongMemEval 的 deep 深度语义通路 5 实例全 0% 召回；启用 bge-base-zh 后，30 实例 × 全 6 题型分层评测 Session Recall@10 达 **90%**、Turn Recall@10 达 63.3%，首次让深度语义通路达到关键词通路的历史精度（详见 `benchmarks/V0.9.7_BENCHMARK_REPORT.md` 第四节）。
- **道体（Daoti）协作式感知（研究侧，未接入排序）**：LRC 已支持道体预判元数据（`daoti_preview_*`）的存储与候选保留门控。本轮对"道体推演结果回传注入 LRC 排序"做了预注册判据的严格验证（符号核三轮 + trigram 正交性 + 公平版状态机 hop 分桶 + BGE 入口混淆排除）：道体连续亲和与 BGE 内容语义**统计正交（ρ≈0.13），但在 BGE 之上的排序增量不成立**（发散联想 hop3-5 仅 +0.7~1.7pp，远低于 8pp 门槛，互补案例为零）。随后按"先导航再捞再回归"设计落地了**检索前导航信号接口**（`recall` 的 `navigation` 参数，门控默认关闭，关闭时行为逐字节一致），并用词典、trigram 网络轨迹、网络亲和三种信号源在同口径产品路径复测：**接口与"改变候选集"机制验证有效（25~28/30 查询候选集变化、A/A 噪声 0），但 hop3-5 净增量为 +1~2pp，与后处理重排一同落在 NO-GO 区间**。据此，道体**不参与产品检索与排序决策**，保留为写入侧感知元数据；记忆联想的实际能力由 BGE 语义底座兑现。推演引擎（DaoTi Research License）为独立研究资产，不随产品分发（详见 `benchmarks/V0.9.7_BENCHMARK_REPORT.md` 第五、六节）。

### 二、仪表盘：让用户直观看见"记忆在替你做什么"

- **M1 价值陈述**：一句话讲清"你的记忆系统正在为你工作"——已存 N 条记忆、被 AI 检索使用 M 次、自动沉淀 K 条结晶，并标注"全部数据来自本机，不上传"。
- **M2 当前记忆**：展示各 AI 工具最近替你写入的记忆，每条含类型徽章、两行内容摘要、项目/时间/重要性。
- **M3 系统替你做的事**：活动流水展示后台完成的结晶归纳、记忆整理。
- **M4 我的记忆资产**：构成条形 / 谁在用 Top5 / 近 7 天成长趋势 / 结晶成果，四子卡图表化。
- **M5 技术细节**：单层折叠，按"运行观测""工具箱与系统"分组，一卡一主题，不与主区重复。
- 修复仪表盘"永久卡后台整理中"的前端并发自锁（改顺序拉取 + 指数退避自愈）；全前端 emoji 图标迁移为 SVG 体系；明确本地工具数据边界，不再读取任何用户反馈埋点。

### 三、安全与韧性加固（全量审查）

- 备份恢复：快照路径强制约束在备份目录内、恢复期间持有存储锁、恢复后失效内存缓存、失败返回正确状态码。
- 后台合成三阶段锁解耦在写回超时时仍保证阈值恢复，杜绝临时配置永久固化；enrich 超时显式置取消标志，防失控任务持锁。
- 桌面端：外部 sidecar 身份校验修复全局模式复用断裂；四条启动路径统一超时口径（40s/120s），取消与超时真正生效。
- 测试门禁：CDP 深层回归对状态前置隐藏建立合法开启与现场恢复机制，错误豁免改为按文本逐条过滤。

## v0.9.6 稳定版特性

- 稳定版 Sidecar 固定使用 `3099`，开发版使用 `3111`，避免两个运行环境互相复用。
- 修复稳定版错误接管开发版 Sidecar 的问题。
- 修复 Release 构建混入 Debug Sidecar 的问题。
- 发布流程增加版本一致性校验、前端资源校验和真实桌面端 CDP 回归门禁。
- 支持 Windows、macOS 和 Linux 桌面端安装包，以及跨平台 CLI Sidecar。

## v0.9.5 特性

- 全局前端导航与发布资源稳定性修复。
- 发布前真实 CDP 回归门禁。
- 结晶历史、演化时间线和道同构度展示闭环。
- AI 工具扫描、快捷方式识别和规则配置链路增强。

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

1. 前往 [v0.9.7 Releases](https://github.com/zhibaiYingChuan/LRC/releases/tag/v0.9.7) 下载**桌面安装包**（注意文件名，勿下载 CLI 二进制）：
   - Windows：`lrc-desktop-v0.9.7-windows-x86_64-setup.exe`
   - macOS：`lrc-desktop-v0.9.7-macos-arm64.dmg`
   - Linux：`lrc-desktop-v0.9.7-linux-amd64.deb` 或 `lrc-desktop-v0.9.7-linux-x86_64.AppImage`
2. 双击安装，启动 LRC Desktop
3. 按向导选择项目、配置 LLM（可选）、连接 AI 工具
4. 重启 IDE，AI 通过 MCP 自动发现并使用记忆与代码搜索能力

> 桌面端自动完成所有配置：检测 AI 工具、写入 MCP 配置、写入 AI 规则文件。
>
> **端口说明**：稳定版默认使用 `3099`；本地 Debug 开发版使用 `3111`。稳定版不会复用开发版 Sidecar。
>
> **注意**：Release 中 `lrc-v0.9.7-windows-x86_64.exe` 等文件是 **CLI 命令行工具**（Sidecar 二进制），供开发者和脚本调用，**不是安装包**，双击无法安装。安装请使用 `lrc-desktop-*` 开头的安装包。

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
| SVG 图标集 | [static/assets/icons/](static/assets/icons) | 56 个极简线性图标（53 个通用图标 + 3 个洛书能力图标，24x24px 栅格） |
| SVG Logo 集 | [static/assets/logo/](static/assets/logo) | 2 种 SVG Logo 形态（主标/横版）+ 1 张设计稿 PNG |

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

## MCP 工具

桌面端和 CLI 通过 MCP 提供记忆、检索、代码搜索与系统管理能力。当前内置工具数量和名称以运行中的 `tools/list` 返回为准，避免文档与实际版本漂移。

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
| [v0.9.7 基准测试报告](benchmarks/V0.9.7_BENCHMARK_REPORT.md) | 记忆联想优越性验证（内部 A/B + 道体消融 + LongMemEval） |
| [v0.9.5 基准测试报告](benchmarks/V0.9.5_BENCHMARK_REPORT.md) | 内置基准回归结果（报告文件保留历史命名） |
| [使用场景](docs/USE_CASES.md) | 典型应用场景与最佳实践 |
| [Smart Match 离线安装](docs/OFFLINE_MODEL_GUIDE.md) | 内网/离线环境模型安装 |

---

## License

- 代码部分：[Apache 2.0](LICENSE_CODE)
- 检索引擎：[DaoTi Research License](LICENSE)
