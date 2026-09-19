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
python .\scripts\dev-proxy.py 1420 --dev   # 开发代理：服务磁盘最新 static/（禁用 WebView2 缓存），API 代理到 sidecar
.\scripts\run-dev.ps1                      # 开发版实例：端口 3100 + 独立数据目录，不影响稳定版 3099
.\scripts\run-dev.ps1 -Build               # 先编译再启动
```

> 端口约定：稳定版 `3099`；`run-dev.ps1` 开发实例 `3100`；`--dev` 模式锁定 `3111`。三者数据目录相互隔离。
>
> `tests/frontend/` 下的 CDP 测试优先经 `localhost:1420` 验证磁盘最新前端；dev-proxy 未启动时自动回退原地重载并告警。

---

## v0.9.9 稳定版特性

**本版本主题：交互可用性修复 + ML 模式联想超时根因 + 符号层证据性质分离。**

- **修复「每次打开都要手动点启动服务」**：自动启动判据原为 `setup_complete`（是否走过向导），而正常使用不需要走向导（全局模式不选项目目录、不用 LLM）⇒ 该判据**永远为 false**，自动启动被永久跳过。改为按「**是否确有使用痕迹**」判定后，实测多次重开**全部自动拉起**。
- **符号层接入界面（证据性质分离）**：联想的两类证据**分开显示**，不再混淆——「**记录关联**」（记录必成立）与「**结构推导**」（由结构算子推出，**可能不成立**）。后者带独立徽章，用户一眼可辨可信度。
- **修复联想探索超时阶梯**：前端超时原与后端**完全相等**，导致后端的优雅收敛（返回部分结果 / 诚实空态）**不可达**、只能报超时。现改为单调阶梯，慢环境下也能拿到"已展示完成的部分"或"记忆库里还没有相关内容"。
- **修复 ML 模式下开发端联想超时**：Cargo 默认对 dev profile 的依赖不做优化，ML 张量前向慢 2~4 倍而击穿超时。现只优化 **ML 链路依赖**（含易漏的 `gemm` 矩阵乘法后端），使 **debug 与 release 行为一致**——线上 CI 只需多编 13 个包。
- **修复 `action_hints` 语义反转**：健康提示原会把「系统输出质量**良好**」升级成「请**优先处理**」；现只升级真正的 `warning`。

> v0.9.8 及更早版本特性见 [CHANGELOG.md](CHANGELOG.md)；v0.9.7 联想能力的优越性验证（LongMemEval Session Recall@10 达 90%）见 [基准测试报告](benchmarks/V0.9.7_BENCHMARK_REPORT.md)。

**道体（Daoti）边界**：推演引擎为独立研究资产（DaoTi Research License），不随产品分发；LRC 仅内置降级 HTTP 客户端（默认关闭、离线静默降级），道体不参与检索与排序决策。记忆联想的实际能力由 BGE 语义底座 + 记录层确定性逻辑兑现。

---

## 记忆联想（核心能力）

**一句话**：搜索是"找字面像的"，记忆联想是"想起相关的"——即使你和原始记录**没有任何共同关键词**。

### 它怎么工作

系统会从你的记忆里自动识别**四类关联**，全部来自已有记录，**不编造**：

| 关联类型 | 触发条件 | 例子 |
|---------|---------|------|
| **同一次经历** | 同一条记忆里手填了相同的 `event_id`（知情者断言） | 那次杭州之行记的"游西湖"与"楼外楼吃饭" |
| **同一时段** | 同项目 + 时间窗口内自动推断（`auto:` 前缀，系统推断，标注来源） | 相隔 37 分钟写下的两条工作记录 |
| **共享实体** | 两条记忆提到同一个具体对象 | 都提到 `commands.rs` 或同一份文件 |
| **互相结晶** | 由同一批记忆融合而成（`derived_from` / `crystallized_into`） | 36 条来源融合出的结晶条目 |

> **诚实边界**：以上属**记录型关联**——由记录字段**必然**成立。
> 另有**推理型关联**（由因果/时序/约束等逻辑关系推出，**可能不成立**）
> 在界面中明确标记为「结构推导」，与「记录关联」**分开显示**、不混淆：
> 用户一眼能看出哪条必然可信、哪条只是推测。

### 三种用法

**1. 联想补全（自动）**——搜索结果里，语义不相似但由记录必然关联的记忆**单独分区**展示，带「凭什么关联」的理由，可核验、不参与排序。

**2. 联想中心 · 探索（主动提问）**——进入「联想中心」，输入一句话，例如：

```text
今晚吃什么          → 想起：和谁吃 / 在哪吃 / 饮食约束 / 饭后惯例
我以前记过什么重要日子？ → 想起：结婚纪念日 / 家人生日 / 认识十周年
```

系统沿关联链逐层扩散（有界 BFS，带时间预算），返回**部分结果**时明确告知"想得有点远，已展示完成的部分"；记忆库里确实没有相关内容时，返回**诚实空态**而不是硬凑答案：

```text
你的记忆库里还没有和「量子物理是什么」相关的内容。
这次没有想起相关的念头——不是联想坏了，是记忆里还没有记过这类事情。
```

**3. 联想足迹**——查看历史联想记录，可一键隐藏（隐藏后重启不复活，同时保持审计哈希链完整）。

### 关联的技术底座

| 层 | 作用 | 可靠性 |
|---|---|---|
| **BGE 语义底座** | 无共同关键词时的语义邻近召回（`BAAI/bge-base-zh`，768 维） | 语义相近，**可能错** |
| **记录层确定性逻辑** | `event_id` / 共享实体 / 同项目时段等字段推导 | 记录必然成立，**不会错** |
| **符号层结构算子** | 由图结构推导出的因果/时序/约束关系 | 结构推导，**可能不成立**，界面标注「结构推导」 |

> **隐私**：联想只在本机进行，数据不上传。联想过程记录仅存于 `~/.loong-recall/`。

---

## 快速开始

### 方式一：下载桌面端（推荐）

1. 前往 [v0.9.9 Releases](https://github.com/zhibaiYingChuan/LRC/releases/tag/v0.9.9) 下载**桌面安装包**（注意文件名，勿下载 CLI 二进制）：
   - Windows：`lrc-desktop-v0.9.9-windows-x86_64-setup.exe`
   - macOS：`lrc-desktop-v0.9.9-macos-arm64.dmg`
   - Linux：`lrc-desktop-v0.9.9-linux-amd64.deb` 或 `lrc-desktop-v0.9.9-linux-x86_64.AppImage`
2. 双击安装，启动 LRC Desktop
3. 按向导选择项目、配置 LLM（可选）、连接 AI 工具
4. 重启 IDE，AI 通过 MCP 自动发现并使用记忆与代码搜索能力

> 桌面端自动完成所有配置：检测 AI 工具、写入 MCP 配置、写入 AI 规则文件。
>
> **端口说明**：稳定版默认使用 `3099`；开发实例端口约定见上文「质量与验证」一节。稳定版不会复用开发版 Sidecar。
>
> **注意**：Release 中 `lrc-v0.9.9-windows-x86_64.exe` 等文件是 **CLI 命令行工具**（Sidecar 二进制），供开发者和脚本调用，**不是安装包**，双击无法安装。安装请使用 `lrc-desktop-*` 开头的安装包。
>
> **安装包体积说明**：桌面安装包仅数 MB 是设计使然——语义模型按需下载（首次约 100~400MB，自动走国内镜像），道体推演引擎为独立研究资产不随产品分发（见 v0.9.9 特性节的边界说明）。

### 方式二：从源码编译

```bash
git clone https://github.com/zhibaiYingChuan/LRC.git
cd LRC
cargo build --release --features server
./target/release/code-memory-server --src-dir ./src --port 3099
```

如需离线语义搜索：`cargo build --release --features server,ml`（首次下载模型 ~500MB）。

### 通用语义引擎

默认嵌入模型为 **BGE-small-zh**（中文用户开箱最优）或 **MiniLM-L6-v2**（英文环境），并支持本地嵌入完成记忆结晶，无需 LLM API 即可享受记忆融合能力。

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
