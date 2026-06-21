# Loong Recall (LRC)

**本地运行的自我演化记忆服务 — 为 AI 应用提供持久、可检索、可验证的长期记忆。**

[![License](https://img.shields.io/badge/Code-Apache%202.0-blue.svg)](LICENSE_CODE)
[![License](https://img.shields.io/badge/Engine-DaoTi%20Research%20License-red.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

**前置条件**：Windows / Linux / macOS | 无需 GPU

**两种安装方式**：
- **方式一（推荐）**：下载 [LRC Desktop 桌面端安装包](https://github.com/zhibaiYingChuan/LRC/releases)，双击安装即可，无需安装 Rust 或命令行操作
- **方式二**：从源码编译，需要安装 Rust 1.75+ 和基本命令行操作能力（见下方「快速开始」）

***

## 快速开始（5 分钟）

### 方式一：下载桌面端安装包（推荐）

1. 前往 [Releases 页面](https://github.com/zhibaiYingChuan/LRC/releases) 下载最新版安装包
2. 双击安装，启动 LRC Desktop
3. 按照配置向导选择项目目录、配置 LLM（可选）、连接 AI 工具
4. 重启 IDE，AI 自动发现 12 个 MCP 工具

> LRC Desktop 会自动完成所有配置：检测 AI 工具、写入 MCP 配置、写入 AI 规则文件。用户无需手动编辑任何文件。

### 方式二：从源码编译（高级用户）

```bash
# 1. 安装 Rust 1.75+
# Windows: 下载 https://rustup.rs/
# macOS/Linux: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. 克隆并编译（默认 Fast Match 模式，零外部依赖）
git clone https://github.com/zhibaiYingChuan/LRC.git
cd LRC
cargo build --release --features server

# 3. 编译产物在 target/release/code-memory-server.exe（Windows）
#    或 target/release/code-memory-server（macOS/Linux）
```

如需 Smart Match（离线语义搜索），使用 `cargo build --release --features server,ml`（首次启动需下载模型 ~500MB）。

### 接入 IDE

**LRC Desktop 用户**：启动桌面端后自动完成，跳过此步。

**源码编译用户**：在 IDE 的 MCP 配置文件中添加（以 Trae CN 为例，`~/.trae-cn/trae-mcp.json`）：

```json
{
  "mcpServers": {
    "lrc-memory": {
      "type": "http",
      "url": "http://127.0.0.1:3099/mcp",
      "description": "LRC — 本地代码记忆与语义搜索"
    }
  }
}
```

> **重要**：LRC Desktop 总是启动 HTTP 模式的 sidecar（端口 3099），因此所有 AI 工具都应使用 HTTP 模式配置。stdio 模式仅适用于从源码编译并直接通过命令行启动的场景。

| IDE | 配置文件位置 |
|-----|------------|
| **Trae CN** | `~/.trae-cn/trae-mcp.json` 或 `%APPDATA%/Trae CN/User/mcp.json` |
| **Trae** | `~/.trae/mcp.json` 或 `%APPDATA%/Trae/User/mcp.json` |
| **Cursor** | `%APPDATA%/Cursor/mcp.json` |
| **VS Code** | `%APPDATA%/Code/User/settings.json` |
| **Claude Desktop** | `%APPDATA%/Claude/claude_desktop_config.json` |
| **通用 MCP** | HTTP 端点 `http://127.0.0.1:3099/mcp` |

重启 IDE 后，AI 自动发现 12 个 MCP 工具。

> **AI 自动调用规则**：LRC Desktop 启动时会自动为检测到的 AI 工具写入规则文件（如 `.trae/rules/lrc-memory.md`），引导 AI 在会话开始时主动调用 `recall` 检索记忆，完成任务后调用 `remember` 同步记忆。用户无需手动编写任何规则。详细说明见 [用户使用说明书](docs/USER_GUIDE.md)。

***

## 解决两个最痛的场景

用 AI 写代码时，你有没有遇到过——

**场景一**：你想让 AI 改某个功能，但不知道那个功能在哪个文件里。你只能手动翻目录→搜索关键词→复制粘贴几百行代码给 AI。每次都要重复这个流程。

**场景二**：你跟 AI 聊了很久，约定了很多事——"数据库用 PostgreSQL"、"端口 8080"、"用 pnpm 别用 npm"。但第二天新开一个会话，AI 全忘了，你得重新说一遍。

**Loong Recall 解决的就是这两个问题。** 它给 AI 助手装上两个能力：

| 能力 | 做什么 | 一句话说清楚 |
|------|--------|------------|
| **代码定位** `search_code` | 知道函数名/变量名，AI 快速定位。配置 LLM 后可用自然语言描述 | "关键词匹配定位，无需手动翻文件" |
| **项目记忆** `remember / recall` | 告诉 AI 一次约定，以后每次对话它都记得 | "给 AI 装个记事本，但它是活的" |

LRC Desktop 自动写入规则文件后，AI 会在需要时调用这些工具。你只管正常对话：

```
你：search_code("authenticate_user")      → 找到 authenticate_user()
你："我们之前约定的 API 端口是啥？"         → AI 自动 recall → "8080，你上次定的"
你："记得：包管理器用 pnpm"                → AI 调用 remember → 下次会话可检索
```

> **重要**：LRC Desktop 启动时会自动为检测到的 AI 工具写入规则文件（如 `.trae/rules/lrc-memory.md`），引导 AI 主动调用记忆工具，用户无需手动编写任何规则。从源码编译的用户请参考 [用户使用说明书](docs/USER_GUIDE.md) 手动配置规则文件。

LRC 是一个独立封装的 MCP 插件，Fast Match 模式下零外部依赖（纯 Rust 实现）。

---

## HTTP 模式调试（可选）

编译后可直接运行 HTTP 模式进行调试：

```bash
# 启动服务
./target/release/code-memory-server --src-dir ./src --port 3099

# 搜索代码
curl -X POST http://127.0.0.1:3099/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_code","arguments":{"query":"authenticate_user","top_k":3}}}'

# 写一条记忆
curl -X POST http://127.0.0.1:3099/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"remember","arguments":{"content":"项目使用 pnpm 作为包管理器","memory_type":"preference","tags":["tooling"]}}}'

# 检索记忆
curl -X POST http://127.0.0.1:3099/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"recall","arguments":{"query":"包管理器偏好","top_k":3}}}'
```

***

## 搜索模式

LRC 提供两种搜索模式和一个可选增强，编译时通过 Cargo feature 切换：

| | Fast Match（默认） | Smart Match（`--features ml`） | LLM 增强（`--llm-api`） |
|---|---|---|---|
| **编码器** | FastEncoder（内联词袋编码器） | CodeBertEncoder（candle + GraphCodeBERT） | FastEncoder + LLM 查询翻译 |
| **怎么搜** | 精确关键词匹配 | 本地语义模型 | 你的 LLM 翻译查询 → Fast Match 检索 |
| **适合** | 你知道函数名/变量名，懒得翻文件 | 离线语义搜索 | 用自然语言描述，AI 帮你找到 |
| **启动速度** | 即时 | 首次需下载模型（~500MB） | 即时（依赖 LLM 响应） |
| **内存占用** | < 10 MB | ~500 MB | < 10 MB |
| **依赖** | 零，纯 Rust | 自动从 hf-mirror.com 镜像下载 | 需要 LLM API（DeepSeek / 通义千问等）或本地 Ollama |

```bash
# 默认 Fast Match（推荐日常使用，零外部依赖）
cargo build --features server

# Smart Match（离线语义搜索，需下载模型）
cargo build --features server,ml

# LLM 增强（用你的 LLM 做查询翻译，不下载模型）
code-memory-server --src-dir ./src --stdio --llm-api "openai:sk-your-deepseek-key:deepseek-v4-flash:https://api.deepseek.com/v1"
# 或使用本地 Ollama（无 API 费用）
code-memory-server --src-dir ./src --stdio --llm-api ollama:localhost:llama3
```

> **LLM 增强的原理**：把你的自然语言查询发给 LLM，翻译成代码关键词，再用 Fast Match 精确检索。不配置不影响其他功能。
>
> 新用户建议用 HTTP 模式启动，通过仪表盘「设置」页面可视化配置 LLM，配置后即时生效。

**Smart Match 模型切换**（可选）：

```bash
# 默认使用 GraphCodeBERT，可回退到 CodeBERT 基线
$env:LRC_MODEL_ID="microsoft/codebert-base"
```

### 成本说明

LLM 增强模式会调用你的 LLM API 进行查询翻译，每次消耗约 **40-50 Token**。

| 模型 | 单次翻译成本 | 每天 100 次 | 每月 3000 次 |
|------|------------|-----------|------------|
| DeepSeek | < 0.0001 元 | < 0.01 元 | < 0.3 元 |
| 通义千问 Qwen-Turbo | < 0.00002 元 | < 0.002 元 | < 0.06 元 |
| 本地 Ollama | **免费** | **免费** | **免费** |

> 不配置 `--llm-api` 则不产生任何 API 调用，Fast Match 照常使用。

***

## 性能概览

| 规模    | 检索延迟   | 说明          |
| ----- | ------ | ----------- |
| 万条记忆  | < 5ms  | 日常开发规模，完全无感 |
| 十万条记忆 | < 15ms | 大型项目规模      |
| 百万条记忆 | < 30ms | 理论验证规模      |

> 以上数据基于消费级 CPU（Intel i7 / AMD R7 级别），未使用 GPU 加速。

### 功能清单

| 能力             | 状态 | 说明               |
| --------------- | -- | ------------------ |
| 跨会话持久化       | ✅ | 记忆持久化存储，支持 TTL 配置 |
| 语义检索          | ✅ | 双路检索融合（关键词 + 语义排序） |
| 自动知识抽象       | ✅ | 自动合并同类记忆 |
| 记忆演化与衰减     | ✅ | 重要记忆天然优先，不活跃记忆自然降权 |
| 区域聚焦检索       | ✅ | 检索延迟可控 |
| 零外部依赖（快速模式）| ✅ | 无需 API 密钥，无需模型下载 |
| 本地运行           | ✅ | 完全离线，数据不出本机 |
| Web 仪表盘         | ✅ | 可视化记忆健康度、API 文档、LLM 设置 |
| 自动打开浏览器      | ✅ | HTTP 模式启动后自动打开仪表盘 |
| LLM 可视化配置      | ✅ | 仪表盘设置页面配置 LLM API，即时生效 |
| 桌面端应用 (Tauri)   | ✅ | Windows 原生桌面应用，系统托盘 + 仪表盘内嵌 |
| 跨 IDE 记忆同步       | ✅ | 项目指纹识别，同一项目跨 IDE 共享记忆 |
| 记忆导出/导入        | ✅ | JSON 格式导出导入，支持项目级/全局模式 |
| MCP 配置自动升级     | ✅ | v0.5.5 sidecar 启动时自动升级旧版本 MCP 配置 |
| AI 规则自动写入      | ✅ | v0.5.5 自动为 AI 工具写入规则文件，引导 AI 主动调用记忆工具 |
| 反逆向工程保护       | ✅ | v0.5.4 多层编译时与运行时保护（具体实现受 DaoTi Research License 保护） |
| 敏感数据内存清零     | ✅ | v0.5.4 API Key 等敏感数据使用后内存清零 |
| 基准测试框架         | ✅ | v0.5.0 内置基准测试，通过 `/v1/benchmarks/report` API 暴露 |

***

## 桌面端应用

LRC 提供基于 Tauri 2 的原生桌面应用（Windows）：

- **配置向导**：首次启动自动引导完成项目目录、LLM 配置和 Agent 连接
- **LLM 可视化配置**：桌面端内置 LLM 设置面板，支持 DeepSeek、通义千问、Ollama 等 10+ 模型提供商
- **Agent 配置引导**：分类展示 IDE 类、桌面应用、命令行工具等 AI 产品的 MCP 配置方法
- **系统托盘**：最小化到托盘，右键快捷操作，启动不自动打开窗口
- **已就绪迷你面板**：显示后台服务运行状态、Agent 连接数、项目路径
- **配置加密存储**：API Key 使用 AES-256-GCM 加密存储

**获取方式**：前往 [Releases 页面](https://github.com/zhibaiYingChuan/LRC/releases) 下载安装包。

**从源码构建桌面端**（高级用户）：

```bash
# 1. 先编译 sidecar（默认 Fast Match 模式）
cargo build --release --features server

# 2. 编译桌面端
cd desktop
pnpm install
pnpm tauri build
```

构建产物位于 `desktop/src-tauri/target/release/bundle/`。

### 数据目录

记忆数据统一存储在 `~/.loong-recall/` 目录下：

```
~/.loong-recall/
├── config.json                    # 全局配置
└── projects/
    └── {项目指纹}/                  # 跨 IDE 识别同一项目
        └── data/
            ├── memories.json      # 记忆数据
            ├── cache/             # 嵌入向量缓存
            └── .migration_done    # 迁移标记
```

***

## 透明手册

> **我们的承诺**：除核心检索引擎（受 DaoTi Research License 保护）外，LRC 的所有工作逻辑、数据流向、隐私边界均在此公开。

### 隐私边界

**LRC 是一个纯本地工具。你的代码和记忆数据永远不会主动离开你的机器。**

| 数据类别 | 存储位置 | 是否离开本机 | 说明 |
|---------|---------|------------|------|
| **源代码索引** | 内存（进程内） | 否 | 源代码索引全部驻留在进程内存中，不写入磁盘，不发送到任何远程服务器 |
| **项目记忆** | `~/.loong-recall/projects/data/memories.json` | 否 | 所有记忆以 JSON 文件存储在本地数据目录 |
| **嵌入向量缓存** | `~/.loong-recall/projects/data/cache/` | 否 | Smart Match 模式的向量缓存，仅在本地磁盘 |
| **模型文件** | `models/` 或 HuggingFace 缓存目录 | 否（首次下载除外） | Smart Match 依赖的 GraphCodeBERT 模型，首次启动时从 hf-mirror.com 下载一次 |
| **LLM 查询翻译** | 你配置的 LLM API | **是（仅当配置了 `--llm-api`）** | 仅发送查询文本本身，不包含任何源代码 |
| **HF 模型下载** | hf-mirror.com（国内镜像） | **是（仅首次下载模型时）** | 首次使用 Smart Match 时的一次性下载 |

**关键隐私保证：**

- **不收集遥测**：LRC 不包含任何埋点、统计上报、使用数据收集代码
- **不发送源代码**：你的代码永远不会被发送到任何远程服务器
- **不依赖云端**：核心功能全部在本地完成，无需网络连接
- **数据完全由你控制**：记忆数据存储在本地目录，你可以随时查看、编辑、删除

### LRC 不做什么

| LRC 不做的 | 原因 |
|-----------|------|
| **不自动扫描你的整个硬盘** | 只索引你通过 `--src-dir` 指定的目录 |
| **不上传你的代码到云端** | 所有代码索引和检索都在本地完成 |
| **不收集使用数据** | 没有遥测、没有埋点、没有统计上报 |
| **不替代 AI 助手** | LRC 是一个检索工具，帮你找到代码，然后由 AI 助手理解和修改 |
| **不修改你的源代码** | LRC 只读取代码，不写入、不修改任何源文件 |

***

## 日常使用流程

装好之后，你的日常工作流会变成这样：

```
早上开工：
  你："继续写昨天的 API 吧"
  AI 自动 recall → "（根据记忆 #1）昨天你决定用 Axum + PostgreSQL，路由结构已经搭好了"
  你不需要重新说一遍上下文

开发中：
  你："帮我找一下认证中间件的代码"
  AI 自动 search_code → "在 src/auth/middleware.rs 第 42 行"
  你不需要手动翻目录

做决策时：
  你："这个项目我们用 Redis 做缓存"
  AI 自动 remember → "已记录，下次会话我会记得"
  你不需要手动记笔记
```

***

## 全部 12 个 MCP 工具

### 代码搜索（2 个）

| 工具               | 用途                      | 必填参数             | 可选参数                       |
| ---------------- | ----------------------- | ---------------- | -------------------------- |
| `search_code`    | 在项目代码库中搜索代码片段         | `query` — 搜索关键词或自然语言 | `top_k` — 返回条数（默认 5，最大 20） |
| `codebase_stats` | 查看代码库索引状态（文件数、片段数、类型分布） | 无                | 无                          |

### 记忆管理（10 个）

| 工具              | 用途                  | 必填参数                  | 可选参数                                                                                                               |
| --------------- | ------------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `remember`      | 写入一条永久记忆            | `content` — 记忆内容      | `memory_type`（fact/preference/decision/code\_context/conversation）、`project`、`tags`、`importance`（1-10）、`ttl_days`  |
| `recall`        | 检索历史记忆            | `query` — 搜索关键词或自然语言描述      | `top_k`、`memory_type`、`project`、`tags`、`min_importance`                                                            |
| `forget`        | 删除一条记忆              | `memory_id`           | —                                                                                                                  |
| `update_memory` | 更新记忆内容              | `memory_id`、`content` | `importance`                                                                                                       |
| `list_memories` | 分页列表查看记忆            | 无                     | `memory_type`、`project`、`tags`、`sort_by`、`order`、`limit`、`offset` |
| `memory_stats`  | 记忆库统计（总数、类型分布、项目分布） | 无                     | 无                                                                                                                  |
| `archive`       | 归档过期记忆到冷存储          | 无                     | 无                                                                                                                  |
| `correct_memory`| 修正记忆内容，保留版本历史      | `memory_id`、`content` | `reason` — 修正原因（可选）                                                                                        |
| `recall_enhanced` | 增强检索                 | `query`               | `top_k`、`user_id`、`session_id`                                                                                   |
| `dao_metrics`   | 记忆库健康指标             | 无                     | 无                                                                                                                  |

***

## CLI 选项

```
用法: code-memory-server [选项]

  --src-dir <路径>    要索引的源码目录 [默认: 当前目录]
  --host <地址>       HTTP 绑定地址 [默认: 127.0.0.1]
  --port <端口>       HTTP 绑定端口 [默认: 3099]
  --stdio             使用 stdio 传输模式（仅适用于从源码编译直接启动的场景；LRC Desktop 使用 HTTP 模式）
  --global            记忆数据存到 ~/.loong-recall/data/（跨项目共享）
  --db-path <路径>    自定义记忆数据库路径（优先级最高，覆盖 --global）
  --llm-api <配置>    配置 LLM 查询翻译（格式: openai:sk-xxx:model 或 ollama:host:model）
  --mode <模式>       搜索模式: auto（默认）| fast | smart
  --proxy <代理地址>   HTTP/HTTPS 代理（如 http://127.0.0.1:7890）
  --daemon            后台守护模式，无控制台窗口
  --multi-window <N>  允许同项目最多 N 个窗口同时运行 [默认: 1]
  --tray              启用系统托盘图标（Windows）
  --help, -h          显示帮助信息
```

六种典型用法：

```bash
# 场景 1：单项目 HTTP 模式（LRC Desktop 内部使用此模式）
code-memory-server --src-dir ./src --port 3099

# 场景 2：单项目 stdio 模式（从源码编译直接接入 IDE 时使用）
code-memory-server --src-dir ./src --stdio

# 场景 3：全局记忆，跨项目共享
code-memory-server --global --db-path /data/my-memories --stdio

# 场景 4：快速模式，跳过模型下载，秒启动
code-memory-server --src-dir ./src --stdio --mode fast

# 场景 5：Smart Match 语义搜索
code-memory-server --src-dir ./src --stdio --mode smart

# 场景 6：LLM 增强，自然语言搜索代码
code-memory-server --src-dir ./src --stdio --llm-api openai:sk-xxx:deepseek-v4-flash:https://api.deepseek.com/v1
```

***

## 切分器支持的语言

按文件扩展名自动选择切分策略，无需手动配置：

| 切分器                   | 扩展名                                                                                           | 识别单元                                                        |
| --------------------- | --------------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| `RustChunker`         | `.rs`                                                                                         | `fn` / `struct` / `trait` / `enum` / `impl` / `mod`         |
| `PythonChunker`       | `.py`                                                                                         | `def` / `async def` / `class`                               |
| `TsJsChunker`         | `.ts` `.tsx` `.js` `.jsx` `.mjs` `.cjs`                                                       | `function` / `class` / `interface` / `type` / `enum` / 箭头函数 |
| `GoChunker`           | `.go`                                                                                         | `func` / `type`（支持接收者方法）                                    |
| `ConversationChunker` | —                                                                                             | 对话轮次（"用户:" / "助手:" / "系统:" 等）                               |
| `GenericChunker`      | `.md` `.txt` `.yaml` `.toml` `.json` `.html` `.css` `.xml` `.sql` `.sh` `.java` `.c` `.cpp` 等 | Markdown 按 `#` 标题切分；其余按段落切分                         |

***

## 贡献指南

```bash
# 运行测试
cargo test --all-targets --features server

# 代码风格检查
cargo clippy --all-targets --features server -- -D warnings
cargo fmt --check
```

### 运行时安全

Loong Recall 在启动时自动执行多层运行时防护，保护核心检索引擎不被逆向工程。检测到威胁时服务静默退出。

**编译时保护**（v0.5.4+）：
- 符号信息剥离（`strip = true`）
- 链接时优化（`lto = true`）
- 体积优化（`opt-level = "z"` + `codegen-units = 1`）
- Panic 时立即终止（`panic = "abort"`）

**运行时保护**：
- 多层反逆向工程检测（具体实现受 DaoTi Research License 保护，不公开）
- 进程守护（process_guard）
- 敏感数据使用后内存清零（SecureString 模式）
- API Key AES-256-GCM 加密存储
- URL 导航白名单验证（仅允许 127.0.0.1）

***

## 文档导航

| 文档                                 | 说明                                  |
| ---------------------------------- | ----------------------------------- |
| [用户使用说明书](docs/USER_GUIDE.md)      | AI 大模型如何主动调用 MCP 服务 |
| [变更日志](CHANGELOG.md)      | 版本变更记录 |
| [模型评估报告](docs/MODEL_EVALUATION.md) | CodeBERT vs GraphCodeBERT 对比与替代方案评估 |
| [性能测试指南](docs/BENCHMARK.md)        | 如何复现性能测试 |
| [基准测试方法论](tests/BENCHMARK_METHODOLOGY.md)        | 基准测试设计哲学与实现细节 |
| [使用场景](docs/USE_CASES.md)          | 典型应用场景与最佳实践 |
| [Smart Match 离线安装指南](docs/OFFLINE_MODEL_GUIDE.md) | 内网/离线环境下手动安装模型 |

***

## 更新日志

### v0.5.5 (2026-06-21) — MCP 配置自动升级 + AI 主动调用修复

**MCP 配置自动升级（重要）**

- Sidecar 启动时自动检测并升级旧版本 MCP 配置（stdio 模式 `loong-recall` → HTTP 模式 `lrc-memory`）
- 用户升级 LRC Desktop 后无需重新运行配置向导，旧配置自动迁移
- 合并配置时自动清理旧的 stdio 模式配置项，避免 stdio/http 模式冲突

**AI 主动调用修复（重要）**

- 修复 Trae 规则文件路径（`.trae/rules.md` → `.trae/rules/lrc-memory.md`）
- 添加 YAML frontmatter（`alwaysApply: true`），确保规则始终生效
- 修复 Cursor 规则文件路径（`.cursorrules` → `.cursor/rules/lrc-memory.mdc`）
- 升级时自动清理旧路径规则文件，提取用户自定义内容并迁移

**AI 工具检测改进**

- 改进检测策略：无 `binary_paths` 且无 `mcp_config_template` 的工具不自动检测
- 避免残留 dot 目录导致的误报（如检测出 9 个实际只有 2 个）

**仪表盘交互统一**

- 移除"修改配置"按钮的只读卡片逻辑
- 统一使用完整 LLM 配置表单（多提供商选择）
- 桌面端所有相同功能/内容统一为同一实现

**内存优化**

- 关闭 `ml` feature 默认启用：sidecar 基线内存占用降低（candle 等重型依赖不再编译进二进制）
- 关闭后台结晶流水线 `run_on_start`：延迟首次合成，避免启动内存峰值

### v0.5.4 (2026-06-20) — 全项目静态审计与修复

- 全项目静态代码审计，修复所有 Clippy 警告
- 桌面端 URL 导航白名单验证（仅允许 127.0.0.1）
- 敏感数据使用后内存清零（SecureString 模式）
- 编译时与运行时反逆向工程保护增强
- DPAPI 密钥损坏自动恢复机制

### v0.5.1 (2026-06-18) — 用户体验统一与代码治理

- 前端版本号一致性（统一从 Cargo.toml 读取）
- 前端 CSS 内联 1260 行提取到 app.css
- 前端 app.js 全局变量污染（IIFE 隔离）
- server.rs 巨型函数拆分（964行 → 5个函数）
- 模型加载逻辑重复（提取共享 PoolingStrategy）
- RRF 融合逻辑重复（提取共享 rrf.rs 模块）

### v0.5.0 (2026-06-17) — 桌面端体验优化 + MCP 集成修复

**桌面端体验优化**

- 启动不再自动打开仪表盘网页，纯托盘运行
- 设置面板新增 LLM 配置（支持 DeepSeek、通义千问、智谱、MiniMax、Moonshot、豆包、阶跃星辰、百川、OpenAI、Ollama）
- 向导完成页新增「配置 LLM 和 Agent」按钮，方便随时修改设置
- 已就绪迷你面板：显示后台服务状态、Agent 连接数、项目路径

**MCP 集成修复（重要）**

- 修复 TraeDetector 在配置文件不存在时返回 None 导致 MCP 配置从未写入的 Bug
- MCP 配置使用绝对路径，IDE 无需依赖 PATH 环境变量
- 新增 AppData 路径检测（`%APPDATA%/Trae CN/User/mcp.json`）
- stdio 模式配置添加 `LRC_MODE` 环境变量
- Agent 配置引导支持分类：IDE 类、桌面应用、命令行工具、其他 AI 产品

**代码质量**

- 修复所有 Clippy 警告（doc_lazy_continuation 等）
- PostgresPersistence 新增 `block_on_async` 封装 tokio 运行时处理
- encoder_codebert::encode 返回 Result 类型，正确传播错误
- 消除 tray.rs 中的 unwrap() 调用

### v0.4.0 (2026-06-15) — 桌面端应用 + 跨 IDE 记忆同步

**桌面端应用**

- 基于 Tauri 2 的原生 Windows 桌面应用，内置配置向导
- 系统托盘支持，最小化后台运行
- 仪表盘内嵌展示，无需浏览器
- API Key AES-256-GCM 加密存储
- Sidecar 进程自动管理

**跨 IDE 记忆同步**

- 项目指纹识别同一项目，跨 Trae/Cursor/VS Code 共享记忆
- 统一数据目录 `~/.loong-recall/projects/{fingerprint}/data/`
- 旧版数据自动迁移

**记忆导出/导入**

- `lrc export` 命令：JSON 格式导出记忆
- `lrc import` 命令：导入记忆，基于 ID 去重

### v0.3.1 (2026-06-12) — 一键体验优化

- 自动打开浏览器：HTTP 模式启动后自动打开仪表盘
- LLM 可视化配置：仪表盘新增「设置」页面
- 添加 `[workspace]` 声明，修复 Cargo 冲突

### v0.3.0 (2026-06-09) — 桌面端 Agent 全面支持

- 配置持久化、后台守护模式、系统托盘、多窗口支持、进程守护

### v0.2.0 (2026-06-07) — 代码质量与安全加固

- 全项目静态代码审计，消除所有非测试代码中的 `.unwrap()` 残留
- 全部引擎文件添加 DaoTi Research License v1.0 许可证头
- CI 自动运行代码质量与安全检查
- 仪表盘新增"指标说明"面板
- 删除 `index.html` 内联 `<script>`，统一由 `app.js` 管理

***

## 分层开源许可

| 层级          | 范围                                                                                                         | 许可证                                        |
| ----------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| **L1 公开层**  | `src/chunker.rs`、`src/memory_store.rs`、`src/memory_types.rs`、`src/persistence/`、`src/server.rs`、`src/bin/` | Apache 2.0 — 可修改、可分叉、可商用                   |
| **L2 受保护层** | `src/engine/` 核心编码与检索算法                                                                                    | DaoTi Research License v1.0 — 源码可见，仅限研究/审计 |

> L2 层代码禁止用于逆向工程或训练竞争模型。商业使用需联系项目所有者获取授权。详见 [LICENSE](LICENSE)。

***

*Loong Recall —— AI 编程助手的记忆与检索插件*