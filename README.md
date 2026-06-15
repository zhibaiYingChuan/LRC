# Loong Recall (L-RC)

**AI 编程助手的记忆与检索插件 — 接入 IDE，AI 就能按需检索代码、跨会话记住关键约定。**

[![License](https://img.shields.io/badge/Code-Apache%202.0-blue.svg)](LICENSE_CODE)
[![License](https://img.shields.io/badge/Engine-DaoTi%20Research%20License-red.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

**前置条件**：Windows / Linux / macOS | 无需 Rust 基础 | 无需 GPU | 会基本命令行操作

> 🪄 **不想手动敲命令？** 双击 `install.bat` 快速安装，自动完成编译和 IDE 配置（需已安装 Rust 环境）。

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

配置好规则文件后，AI 会在需要时调用这些工具。你只管正常对话：

```
你：search_code("authenticate_user")      → 找到 authenticate_user()
你："我们之前约定的 API 端口是啥？"         → AI 自动 recall → "8080，你上次定的"
你："记得：包管理器用 pnpm"                → AI 调用 remember → 下次会话可检索
```

> 💡 **重要**：AI 助手需要规则文件引导才能自动调用记忆工具。我们提供了[配置模板](docs/USER_GUIDE.md)，3 分钟即可完成。LLM 增强模式下，AI 可以直接用自然语言搜代码。

LRC 的编码能力源自 [道体（DaoTi）基座模型](https://github.com/zhibaiYingChuan/DaoTi) 的道枢层，但 LRC **不需要安装 DaoTi**——它是一个独立封装的 MCP 插件，零运行时依赖。

***

## 5 分钟体验

```bash
git clone https://github.com/zhibaiYingChuan/LRC.git
# 国内用户如遇 GitHub 下载缓慢，可使用镜像：
# git clone https://gitcode.com/gcw_M73FIiUo/LRC
cd LRC
cargo build --release --features server
# 国内用户如遇 crates.io 下载缓慢，可配置 Cargo 镜像：
# 在 ~/.cargo/config.toml 中添加：
# [source.crates-io]
# replace-with = 'ustc'
# [source.ustc]
# registry = 'sparse+https://mirrors.ustc.edu.cn/crates.io-index/'

# 启动
./target/release/code-memory-server --src-dir ./src --port 3099
```

启动后浏览器会自动打开仪表盘（`http://127.0.0.1:3099/dashboard`），无需手动输入网址。在另一个终端试试：

```bash
# 搜索代码（Fast Match：精确关键词匹配）
curl -X POST http://127.0.0.1:3099/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_code","arguments":{"query":"authenticate_user","top_k":3}}}'

# 搜索代码（LLM 增强模式：用自然语言描述，需配置 --llm-api）
curl -X POST http://127.0.0.1:3099/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_code","arguments":{"query":"处理用户登录的逻辑","top_k":3}}}'

# 写一条记忆
curl -X POST http://127.0.0.1:3099/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"remember","arguments":{"content":"项目使用 pnpm 作为包管理器","memory_type":"preference","tags":["tooling"]}}}'

# 检索记忆
curl -X POST http://127.0.0.1:3099/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"recall","arguments":{"query":"包管理器偏好","top_k":3}}}'
```

### 接入 IDE（3 分钟）

在 IDE 的 MCP 配置文件中添加（以 Trae 为例，`%APPDATA%/Trae/User/mcp.json`）：

```json
{
  "mcpServers": {
    "loong-recall": {
      "command": "你的安装路径/target/release/code-memory-server.exe",
      "args": ["--src-dir", "你的项目路径/src", "--stdio"]
    }
  }
}
```

| IDE | 配置文件位置 | 规则配置（让 AI 自动使用） |
|-----|------------|-------------------------|
| **Trae** | `%APPDATA%/Trae/User/mcp.json` | `.trae/rules/project-rules.md` |
| **Cursor** | `%APPDATA%/Cursor/mcp.json` | `.cursor/rules/memory.md` |
| **VS Code** | `%APPDATA%/Code/User/settings.json` | `.github/copilot-instructions.md` |

> 💡 详细配置步骤（含 AI 自动调用规则模板）见 [用户使用说明书](docs/USER_GUIDE.md)。

重启 IDE 后，AI 自动发现 12 个 MCP 工具。为使 AI 主动调用这些工具，还需配置项目规则文件（见上方各 IDE 的规则配置列）。

### 🪄 快速安装脚本（推荐给不想手动敲命令的用户）

Windows 用户下载仓库后，双击项目根目录下的 `install.bat`（需已安装 Rust 环境），脚本会自动：
1. 检测 Rust 环境
2. 编译 Loong Recall
3. 搜索本地 IDE（Trae / Cursor / VS Code），自动创建 MCP 配置文件（如配置文件已存在则提示手动合并）

Linux / macOS 用户请在终端运行 `bash install.sh`。

完成后重启 IDE，就能直接使用。脚本会自动跳过已配置的 IDE，多次运行不会重复配置。

***

## 两种搜索模式 + 一个可选增强

| | Fast Match（默认） | Smart Match（`--features ml`） | LLM 增强（`--llm-api`） |
|---|---|---|---|
| **怎么搜** | 精确关键词匹配 | 本地语义模型 | 你的 LLM 翻译查询 → Fast Match 检索 |
| **适合** | 你知道函数名/变量名，懒得翻文件 | 离线语义搜索 | 用自然语言描述，AI 帮你找到 |
| **启动速度** | 即时 | 首次需下载模型（~500MB） | 即时（依赖 LLM 响应） |
| **内存占用** | < 10 MB | ~500 MB | < 10 MB |
| **依赖** | 零，纯 Rust | 自动从 hf-mirror.com 镜像下载 | 需要 LLM API（DeepSeek / 通义千问等）或本地 Ollama |

```bash
# 默认 Fast Match（推荐日常使用）
cargo build --features server

# Smart Match（离线语义搜索）
cargo build --features server,ml

# LLM 增强（用你的 LLM 做查询翻译，不下载模型）
# 方式一：命令行启动时配置
# 推荐：使用 DeepSeek（国产模型，性价比极高）
code-memory-server --src-dir ./src --stdio --llm-api "openai:sk-your-deepseek-key:deepseek-v4-flash:https://api.deepseek.com/v1"
# 或使用本地 Ollama（无 API 费用）
code-memory-server --src-dir ./src --stdio --llm-api ollama:localhost:llama3
#
# 方式二：HTTP 模式启动后，在仪表盘可视化配置（推荐新用户）
# code-memory-server --src-dir ./src --port 3099
# 启动后浏览器自动打开 → 进入「⚙️ 设置」页面 → 填写 LLM API 保存即可
```

> **LLM 增强的原理**：把你的自然语言查询（"处理用户登录的逻辑"）发给 LLM，翻译成代码关键词（`authenticate_user, login, handle_login`），再用 Fast Match 精确检索。不配置 `--llm-api` 就还是原来的 Fast Match，行为完全不变。
>
> 💡 **推荐**：新用户建议用 HTTP 模式启动，通过仪表盘「设置」页面可视化配置 LLM，配置后即时生效，无需重启。

> 日常场景 Fast Match 够用。Smart Match 默认使用 **GraphCodeBERT**（比 CodeBERT 检索精度高 12.3%），详见 [模型评估报告](docs/MODEL_EVALUATION.md)。

### 💰 成本说明

LLM 增强模式会调用你的 LLM API 进行查询翻译，每次消耗约 **40-50 Token**（约 30 Token 输入 + 15 Token 输出）。

| 模型 | 单次翻译成本 | 每天 100 次 | 每月 3000 次 |
|------|------------|-----------|------------|
| DeepSeek | < ¥0.0001 | < ¥0.01 | < ¥0.3 |
| 通义千问 Qwen-Turbo | < ¥0.00002 | < ¥0.002 | < ¥0.06 |
| 本地 Ollama（千问/LLaMA） | **免费** | **免费** | **免费** |

> 💡 **省下的远比花掉的多**：LLM 翻译帮你把"整个文件粘贴给 AI"变成了"只返回精确的 5 个代码片段"。每次查询节省的上下文 Token（500-2000 Token），是翻译本身消耗的 10-50 倍。
>
> 💰 国产模型性价比极高，推荐优先选用。DeepSeek 与通义千问均为国内合规服务，延迟低、无需科学上网。实际费用以各平台最新公告为准。
>
> ⚠️ 不配置 `--llm-api` 则不产生任何 API 调用，Fast Match 照常使用。

***

## 性能概览

| 规模    | 检索延迟   | 说明          |
| ----- | ------ | ----------- |
| 万条记忆  | < 5ms  | 日常开发规模，完全无感 |
| 十万条记忆 | < 15ms | 大型项目规模      |
| 百万条记忆 | < 30ms | 理论验证规模      |

> 以上数据基于消费级 CPU（Intel i7 / AMD R7 级别），未使用 GPU 加速。性能复现方法见 [性能测试指南](docs/BENCHMARK.md)。

### 功能清单

| 能力             | 状态 | 说明               |
| --------------- | -- | ------------------ |
| 跨会话持久化       | ✅ | 记忆持久化存储，支持 TTL 配置 |
| 语义检索          | ✅ | 双路检索融合（关键词 + 语义排序） |
| 自动知识抽象       | ✅ | 递归合成，自动合并同类记忆 |
| 记忆演化与衰减     | ✅ | 重要记忆天然优先，不活跃记忆自然降权 |
| 可逆组合（Unfold） | ✅ | 合成记忆可拆解回原始子记忆 |
| 区域聚焦检索       | ✅ | 仅扫描局部区域，检索延迟可控 |
| 零外部依赖（快速模式）| ✅ | 无需 API 密钥，无需模型下载 |
| 本地运行           | ✅ | 完全离线，数据不出本机 |
| Web 仪表盘         | ✅ | 可视化记忆健康度、船长日志、API 文档、LLM 设置 |
| 自动化质量守门      | ✅ | 10 道 CI 守门，零 unwrap 残留、零算法泄露 |
| 自动打开浏览器      | ✅ | HTTP 模式启动后自动打开仪表盘（v0.3.1+） |
| LLM 可视化配置      | ✅ | 仪表盘设置页面配置 LLM API，即时生效（v0.3.1+） |
| 桌面端应用 (Tauri)   | ✅ | Windows 原生桌面应用，系统托盘 + 仪表盘内嵌（v0.4.0+） |
| 跨 IDE 记忆同步       | ✅ | 项目指纹识别，同一项目跨 IDE 共享记忆（v0.4.0+） |
| 记忆导出/导入        | ✅ | JSON 格式导出导入，支持项目级/全局模式（v0.4.0+） |

***

## 桌面端应用（v0.4.0+）

LRC 提供基于 Tauri 2 的原生 Windows 桌面应用，内置仪表盘，无需手动启动命令行。

### 功能亮点

- **配置向导**：首次启动自动引导完成项目目录、LLM 配置
- **系统托盘**：最小化到托盘，右键快捷操作
- **仪表盘内嵌**：无需浏览器，桌面端直接展示记忆管理面板
- **配置加密存储**：API Key 使用 AES-256-GCM 加密存储
- **Sidecar 自动管理**：桌面端自动管理后台服务生命周期

### 构建桌面端

```bash
# 进入桌面端目录
cd desktop

# 安装前端依赖
pnpm install

# 构建桌面端（需要 Rust 环境）
pnpm tauri build
```

构建产物位于 `desktop/src-tauri/target/release/bundle/`。

### 数据目录

v0.4.0 起，记忆数据统一存储在 `~/.loong-recall/` 目录下：

```
~/.loong-recall/
├── config.json                    # 全局配置
└── projects/
    └── {项目指纹}/                  # SHA256 哈希，跨 IDE 识别同一项目
        └── data/
            ├── memories.json      # 记忆数据
            ├── cache/             # 嵌入向量缓存
            └── .migration_done    # 迁移标记
```

> 项目指纹基于规范化路径的 SHA256 哈希生成，不同 IDE 打开同一项目时识别为同一项目，实现记忆自动同步。

***

## 透明手册

> **我们的承诺**：除核心算法（L2 层，受 DaoTi Research License 保护）外，LRC 的所有工作逻辑、数据流向、隐私边界均在此公开。我们相信，用户有权知道工具在自己的机器上做了什么。

### 数据流向与隐私边界

**一句话总结：LRC 是一个纯本地工具。你的代码和记忆数据永远不会主动离开你的机器。**

| 数据类别 | 存储位置 | 是否离开本机 | 说明 |
|---------|---------|------------|------|
| **源代码索引** | 内存（进程内） | 否 | 源代码被切分为片段后编码为向量，全部驻留在进程内存中。不写入磁盘，不发送到任何远程服务器。 |
| **项目记忆** | `~/.loong-recall/projects/{项目指纹}/data/memories.json` | 否 | 你通过 `remember` 工具写入的所有记忆，以 JSON 文件存储在统一数据目录下。同一项目跨 IDE 共享（v0.4.0+ 新目录结构）。 |
| **嵌入向量缓存** | `~/.loong-recall/projects/{项目指纹}/data/cache/` | 否 | Smart Match 模式下，编码后的向量缓存到此目录，用于下次启动秒加载。仅在本地磁盘。 |
| **模型文件** | `models/` 或 HuggingFace 缓存目录 | 否（首次下载除外） | Smart Match 依赖的 GraphCodeBERT 模型（约 500MB），首次启动时从 hf-mirror.com 下载一次。下载后完全离线使用。 |
| **LLM 查询翻译** | 你配置的 LLM API | **是（仅当配置了 `--llm-api`）** | 如果你配置了 LLM 增强模式，你的自然语言查询会发送到你指定的 LLM API（OpenAI / DeepSeek / 通义千问 / Ollama）。**仅发送查询文本本身，不包含任何源代码。** |
| **HF 模型下载** | hf-mirror.com（国内镜像） | **是（仅首次下载模型时）** | 首次使用 Smart Match 时，从 hf-mirror.com 下载模型文件。这是一次性的，后续启动完全离线。 |

**关键隐私保证：**

- **不收集遥测**：LRC 不包含任何埋点、统计上报、使用数据收集代码。
- **不发送源代码**：你的代码永远不会被发送到任何远程服务器。LLM 翻译模式下，发送的只有你的查询文本（如"处理用户登录的逻辑"），不含任何代码片段。
- **不依赖云端**：核心功能（Fast Match + Smart Match + 记忆管理）全部在本地完成，无需网络连接。
- **数据完全由你控制**：记忆数据存储在项目目录的 `.loong-recall/` 下，你可以随时查看、编辑、删除这些 JSON 文件。

### 三种模式工作原理详解

#### 1. Fast Match（默认模式）

```
你的查询（如 "authenticate_user"）
    │
    ▼
FastEncoder（纯词袋编码，零外部依赖）
    │  └─ 将查询文本与预定义的 ~250 个代码关键词比对
    │  └─ 生成一个 250 维的位向量
    │
    ▼
LocalRetriever（余弦相似度匹配）
    │  └─ 与所有已索引的代码片段向量逐一计算相似度
    │  └─ 按相似度降序排列，返回 Top-K 结果
    │
    ▼
返回结果（文件路径、行号、代码片段、相似度评分）
```

**适用场景**：你知道函数名/变量名，想快速定位代码位置。
**限制**：只能匹配精确关键词，不支持自然语言。查"处理用户登录的逻辑"可能返回空结果。

#### 2. Smart Match（`--mode smart` 或 `--features ml`）

```
你的查询（如 "处理用户登录的逻辑"）
    │
    ▼
CodeBertEncoder（GraphCodeBERT 模型，本地推理）
    │  └─ 将查询文本编码为 768 维语义向量
    │  └─ 理解语义，而非匹配关键词
    │
    ▼
HnswRetriever（近似最近邻检索，图搜索加速）
    │  └─ 在向量空间中查找语义最接近的代码片段
    │  └─ 按语义距离排序，返回 Top-K 结果
    │
    ▼
返回结果（文件路径、行号、代码片段、语义相似度评分）
```

**适用场景**：用自然语言描述意图，不需要记住精确的函数名。离线环境可用。
**代价**：首次启动需下载模型（约 500MB，一次性的），内存占用约 500MB。

#### 3. Smart Match + LLM 增强（`--mode smart --llm-api`）

```
你的自然语言查询（如"处理用户登录的逻辑"）
    │
    ▼
LLM 翻译器（你配置的 LLM API）
    │  └─ 将自然语言翻译为代码关键词列表
    │  └─ 例如: "authenticate_user, login, handle_login, auth"
    │  └─ 消耗约 40-50 Token，耗时约 1-2 秒
    │
    ▼
Smart Match 检索（同上）
    │  └─ 用翻译后的关键词进行语义检索
    │  └─ 返回语义最匹配的代码片段
    │
    ▼
返回结果
```

**适用场景**：你想要最自然的交互方式——用日常语言描述需求，AI 帮你找到代码。
**代价**：依赖 LLM API，产生少量 Token 消耗（详见上方成本说明）。

### 启动行为与缓存策略

LRC 默认使用**镜像启动模式**（`--mode auto`），设计目标：**启动后搜索立即可用，语义精度在后台自动提升**。

```
时间线：
  0s  ─── 启动，Fast Match 索引项目代码
  0s  ─── MCP 服务就绪，IDE 可以开始搜索 ← 用户感知：立即可用
  0s  ─── [后台线程] 开始加载 Smart Match 模型
  ... ─── [后台线程] 模型下载/加载中（首次约 1-5 分钟）
  ... ─── [后台线程] 加载嵌入向量缓存（如果存在）
  Ns  ─── [后台线程] 语义编码完成，原子替换检索器
  Ns  ─── Smart Match 就绪 ← 搜索精度自动提升，用户无感知
```

**缓存策略**：

| 缓存类型 | 文件位置 | 作用 | 失效条件 |
|---------|---------|------|---------|
| **嵌入向量缓存** | `.loong-recall/cache/embedding_cache.json` | 保存代码片段的编码结果，下次启动时快速恢复索引，无需重新编码 | 源码文件发生变更时需手动删除缓存以触发重新索引 |
| **模型缓存** | `models/` 或 HuggingFace 缓存目录 | 保存 GraphCodeBERT 模型文件，避免重复下载 | 手动删除模型文件后需重新下载 |
| **记忆数据** | `.loong-recall/data/memories.json` | 持久化所有项目记忆 | 不会自动失效，由你手动管理 |

**首次启动 vs 后续启动**：

| | 首次启动 | 后续启动 |
|---|---|---|
| Fast Match | 索引项目代码 | 索引项目代码 |
| Smart Match | 下载模型（1-5 分钟）+ 编码（数分钟） | 从缓存恢复 |
| 记忆 | 创建空数据库 | 加载已有记忆 |

> **注意**：当前版本中，源码变更后嵌入向量缓存不会自动失效。如果你修改了源代码，需要手动删除 `.loong-recall/cache/embedding_cache.json` 以触发重新索引。后续版本将实现基于文件哈希的自动失效。

### LRC 不做什么

明确边界，避免误解：

| LRC 不做的 | 原因 |
|-----------|------|
| **不自动扫描你的整个硬盘** | 只索引你通过 `--src-dir` 指定的目录 |
| **不上传你的代码到云端** | 所有代码索引和检索都在本地完成 |
| **不收集使用数据** | 没有遥测、没有埋点、没有统计上报 |
| **不替代 AI 助手** | LRC 是一个检索工具，帮你找到代码，然后由 AI 助手理解和修改 |
| **不修改你的源代码** | LRC 只读取代码，不写入、不修改任何源文件 |
| **不保证 100% 的检索召回率** | 语义搜索是近似匹配，不是精确查找。Fast Match 也受限于关键词覆盖范围 |
| **不自动管理记忆生命周期** | 记忆的创建、更新、删除由你（或 AI 助手）主动控制，LRC 不会自动删除你的记忆 |

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

**核心体验**：你只管正常写代码、正常聊天。AI 自动判断什么时候该搜代码、什么时候该查记忆。

> 💡 详细配置步骤（含 AI 自动调用规则模板）见 [用户使用说明书](docs/USER_GUIDE.md)。

***

## 更多使用场景

除了 IDE 中的代码搜索和对话记忆，Loong Recall 还适用于：

* **AI 客服** — 记住每个用户的偏好和历史问题，跨会话延续服务上下文

* **个人知识管家** — 将日常对话中的关键信息（决策、偏好、事实）自动沉淀为永久记忆

* **项目知识库** — 为团队项目维护一个自动演化的知识图谱，新人接手时直接检索历史决策

详见 [使用场景文档](docs/USE_CASES.md)。

***

## 全部 12 个 MCP 工具

接入 IDE 后，AI 助手能直接调用的全部工具：

### 代码搜索（2 个）

| 工具               | 用途                      | 必填参数             | 可选参数                       |
| ---------------- | ----------------------- | ---------------- | -------------------------- |
| `search_code`    | 在项目代码库中搜索代码片段         | `query` — 搜索关键词（Fast Match）或自然语言（LLM增强） | `top_k` — 返回条数（默认 5，最大 20） |
| `codebase_stats` | 查看代码库索引状态（文件数、片段数、类型分布） | 无                | 无                          |

### 记忆管理（10 个）

| 工具              | 用途                  | 必填参数                  | 可选参数                                                                                                               |
| --------------- | ------------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `remember`      | 写入一条永久记忆            | `content` — 记忆内容      | `memory_type`（fact/preference/decision/code\_context/conversation）、`project`、`tags`、`importance`（1-10）、`ttl_days`  |
| `recall`        | 检索历史记忆            | `query` — 搜索关键词或自然语言描述      | `top_k`、`memory_type`、`project`、`tags`、`min_importance`                                                            |
| `forget`        | 删除一条记忆              | `memory_id`           | —                                                                                                                  |
| `update_memory` | 更新记忆内容              | `memory_id`、`content` | `importance`                                                                                                       |
| `list_memories` | 分页列表查看记忆            | 无                     | `memory_type`、`project`、`tags`、`sort_by`（created\_at/importance/last\_accessed）、`order`（desc/asc）、`limit`、`offset` |
| `memory_stats`  | 记忆库统计（总数、类型分布、项目分布） | 无                     | 无                                                                                                                  |
| `archive`       | 归档过期记忆到冷存储          | 无                     | 无                                                                                                                  |
| `correct_memory`| 修正记忆内容，保留版本历史      | `memory_id`、`content` | `reason` — 修正原因（可选）                                                                                        |
| `recall_enhanced` | 增强检索（双路 RRF 融合）     | `query`               | `top_k`、`user_id`、`session_id`                                                                                   |
| `dao_metrics`   | 道同构度健康指标             | 无                     | 无                                                                                                                  |

***

## 架构全景

Loong Recall 采用**分层架构**，每一层职责清晰、单向依赖。下面从外到内逐层展开：

```
                            ┌──────────────────────────────────────────┐
                            │          IDE / AI 助手                    │
                            │    (Trae, Cursor, VS Code 等)            │
                            └─────────────┬────────────────────────────┘
                                          │ MCP 协议 (JSON-RPC 2.0)
                                          │ HTTP 或 Stdio 传输
                            ┌─────────────▼────────────────────────────┐
                            │     ① MCP 传输层 (src/server.rs)         │
                            │                                          │
                            │   路由分发 → 参数解析 → 结果格式化         │
                            │   暴露 12 个工具：                         │
                            │   代码搜索 2 个 + 记忆管理 10 个           │
                            │                                          │
                            │   HTTP:  POST /mcp  (调试用)              │
                            │   Stdio: stdin → stdout (IDE 标准模式)    │
                            └──┬──────────────────────┬────────────────┘
                               │                      │
                    代码搜索通道（向量检索）     记忆管理通道（语义检索 + 衰减排序）
                               │                      │
              ┌────────────────▼──────────┐  ┌───────▼──────────────────┐
              │  ② 引擎编排层              │  │  ③ 记忆存储层             │
              │  engine/manager.rs        │  │  memory_store.rs         │
              │                           │  │                          │
              │  协调三阶段流水线：         │  │  领域聚合根：             │
              │  切分 → 编码 → 检索        │  │  增删改查 + 语义召回      │
              │                           │  │  分页列表 + 过期归档      │
              │  对外接口：index_project() │  │  衰减因子 + 自动合成      │
              │          search()         │  │                          │
              │          get_stats()      │  │  对外接口：remember()     │
              └──┬──────────┬─────────────┘  │          recall()        │
                 │          │                 │          forget()        │
    ┌────────────▼──┐  ┌───▼──────────────┐  │          list_memories() │
    │ ④ 切分器层     │  │ ⑤ 编码与检索层   │  │          stats()         │
    │ chunker.rs    │  │ engine/          │  └──┬───────────────────────┘
    │               │  │                  │     │
    │ 多语言语法切分 │  │ encoder.rs       │  ┌──▼───────────────────────┐
    │ 按函数/类/段落 │  │  FastEncoder     │  │ ⑥ 持久化层               │
    │ 边界拆分代码   │  │  (词袋编码)       │  │ persistence/json.rs     │
    │               │  │                  │  │                          │
    │ RustChunker   │  │ encoder_codebert │  │ Persistence trait (抽象) │
    │ PythonChunker │  │  (语义编码, 可选) │  │ JsonPersistence (默认)   │
    │ TsJsChunker   │  │                  │  │                          │
    │ GoChunker     │  │ retriever.rs     │  │ 文件结构：               │
    │ GenericChunker│  │  余弦相似度匹配   │  │ memories.json           │
    │ Conversation  │  │  Top-K 排序      │  │ chunks.json             │
    │ Chunker       │  │                  │  │ archive.json            │
    └───────────────┘  │ hnsw.rs          │  └──────────────────────────┘
                       │  近似最近邻索引   │
                       │  图搜索加速       │     ┌──────────────────────┐
                       │                  │     │ ⑦ 运行时防护层         │
                       │ encoder_registry │     │ guard.rs             │
                       │  按语言路由编码器 │     │                      │
                       └──────────────────┘     │ 反调试 + PE 校验     │
                                                │ 字符串混淆 + 控制流  │
                                                │ 混淆，启动时自动执行  │
                                                └──────────────────────┘
```

### 从用户请求到返回结果 — 一次完整的搜索过程

以 `search_code` 为例，一个请求经过的完整路径：

```
IDE 发送 JSON-RPC 请求
        │
        ▼
① server.rs — 解析 JSON，路由到 `handle_tools_call("search_code")`
        │
        ▼
② manager.rs — 调用 `search(query, top_k)`
        │
        ├─→ ④ chunker.rs — 启动时已完成：遍历项目目录，按语法边界切分所有文件
        │       └─ 产出：Vec<CodeChunk>（每个片段含文件路径、行号、代码内容）
        │
        ├─→ ⑤ encoder.rs — 启动时已完成：每个 CodeChunk 被 FastEncoder 编码为向量
        │       └─ 产出：Vec<EmbeddingVector>（等长浮点数数组）
        │
        └─→ ⑤ retriever.rs — 运行时：用户查询同样被编码，与所有片段向量计算余弦相似度
                └─ 产出：Vec<ScoredChunk>（按相似度倒序排列的 Top-K 结果）
        │
        ▼
① server.rs — 格式化结果（文件路径、行号、代码块、评分），返回 JSON-RPC 响应
        │
        ▼
IDE 在 AI 对话中展示搜索结果
```

### 一次记忆请求的完整路径

以 `recall` 为例：

```
IDE 发送 JSON-RPC 请求
        │
        ▼
① server.rs — 路由到 `handle_tools_call("recall")`，解析参数
        │
        ▼
③ memory_store.rs — 调用 `recall(query, filter)`
        │
        ├─→ 从持久化层加载所有记忆
        ├─→ 按类型/项目/标签/重要性过滤
        ├─→ 编码查询并与记忆库中所有记忆计算相似度
        ├─→ 按相似度 + 衰减因子综合排序，重要且活跃的记忆天然优先
        └─→ 返回 Top-K 记忆，附带衰减权重
        │
        ▼
① server.rs — 格式化结果（记忆 ID、类型、标签、深度），返回响应
```

### 各层组件详解

| 层级              | 源文件                              | 一句话职责      | 详细说明                                                                                                                                                                   |
| --------------- | -------------------------------- | ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **① MCP 传输层**   | `src/server.rs`                  | 协议适配与工具路由  | 实现 MCP 协议全部生命周期：`initialize`（握手）→ `tools/list`（声明工具）→ `tools/call`（执行调用）。HTTP 模式基于 Axum 框架，Stdio 模式基于 stdin/stdout 管道。所有工具共享同一个 `AppState`（内存中的索引管理器 + 记忆存储），无需外部数据库。  |
| **② 引擎编排层**     | `src/engine/manager.rs`          | 三阶段流水线调度   | 调用了④⑤两层完成索引和检索。`index_project()` 遍历目录、按扩展名过滤文本文件、调用 chunker 切分、调用 encoder 编码、存入 retriever。是代码搜索功能的唯一对外入口。                                                              |
| **③ 记忆存储层**     | `src/memory_store.rs`            | 记忆领域聚合根    | 封装所有记忆 CRUD 操作和语义检索逻辑。`recall()` 通过向量相似度计算 + 衰减因子排序召回候选记忆，重要且活跃的记忆天然优先返回。同时集成指数衰减模型，让不活跃的记忆自然降权。`archive_expired()` 将过期记忆迁入冷存储（`archive.json`），保持活跃记忆库轻量。            |
| **④ 切分器层**      | `src/chunker.rs`                 | 按语法边界拆分代码  | 包含 6 个切分器实现。Rust 用大括号深度匹配，Python 用缩进层级检测，TS/JS 支持箭头函数识别，Go 支持接收者方法，Conversation 按角色前缀切分，其余格式（Markdown、YAML 等）按段落边界切分。所有切分器共享同一个 `CodeChunker` trait。                   |
| **⑤-1 编码器**     | `src/engine/encoder.rs`          | 代码文本 → 向量  | 核心抽象是 `CodeEncoder` trait。默认 `FastEncoder` 基于预定义关键词词袋生成位向量（dim\~250），零外部依赖、零模型下载。语义模式下 `CodeBertEncoder` 使用 candle 推理框架生成 768 维语义向量，默认使用 GraphCodeBERT 模型。             |
| **⑤-2 编码器注册表**  | `src/engine/encoder_registry.rs` | 按语言路由编码策略  | 维护 `语言 → 编码器` 的映射表。支持按语言注册专用编码器（如为 Rust 注册更精准的编码器），未注册语言自动回退到默认编码器。实现 Strategy + Registry 组合模式。                                                                        |
| **⑤-3 检索器**     | `src/engine/retriever.rs`        | 向量相似度匹配    | `LocalRetriever` 维护所有片段向量，查询时计算余弦相似度并排序返回 Top-K。`threshold` 参数过滤低相似度结果。`CodeRetriever` trait 定义统一检索接口，便于替换为远程检索后端。                                                     |
| **⑤-4 HNSW 索引** | `src/engine/hnsw.rs`             | 近似最近邻加速    | 基于 Navigable Small World 图算法。每个节点最多连接 M=16 个邻居，搜索时束宽 ef\_search=50。相比暴力遍历，百万级片段下检索延迟从 O(n) 降至 O(log n)。当前 HnswRetriever 同时实现 CodeRetriever trait，与 LocalRetriever 可互换。 |
| **⑥ 持久化层**      | `src/persistence/`               | 记忆与切片的文件存储 | `Persistence` trait 定义 11 个抽象方法（save/load/delete/clear/archive）。`JsonPersistence` 是默认实现，数据存储在 `data_dir/` 下的 3 个 JSON 文件中。trait 设计使后续可平滑切换到 SQLite、Redis 等后端而不影响上层代码。  |
| **⑦ 运行时防护层**    | `src/guard.rs`                   | 防逆向工程保护    | 启动时在 `main()` 第一行自动调用 `risk_aware_guard()`。包含 6 层防护：编译时字符串 XOR 加密（`obfuscated!` 宏）、三级反调试联动、反单步时序检测、函数入口断点扫描、PE 代码段 CRC32 自校验、不透明谓词 + 状态机控制流混淆。检测到威胁时随机延迟后静默退出。         |

### 数据模型速览

```
CodeChunk (代码片段)                    Memory (记忆条目)
─────────────────────                   ──────────────────
id: "src/main.rs:L10-L25"              id: "uuid-v4"
file_path: "src/main.rs"               content: "项目使用 pnpm"
start_line: 10                         memory_type: Preference
end_line: 25                          project: Option<"my-app">
chunk_type: "fn"                       tags: ["pnpm", "tooling"]
name: "authenticate_user"              importance: 1-10
signature: "fn authenticate_user()"    ttl_days: Option<u32>
content: "fn authenticate_user() {..}" created_at / updated_at / last_accessed
doc_comment: Option<"验证用户身份">    decay_factor(): 指数衰减
language: "rust"
```

两个数据模型完全独立，各自有独立的存储文件和检索通道。代码片段由切分器自动生成，记忆由 AI 助手通过 MCP 工具手动管理。

***

## CLI 选项

```
用法: code-memory-server [选项]

  --src-dir <路径>    要索引的源码目录 [默认: 当前目录]
  --host <地址>       HTTP 绑定地址 [默认: 127.0.0.1]
  --port <端口>       HTTP 绑定端口 [默认: 3099]
  --stdio             使用 stdio 传输模式（推荐 IDE 部署）
  --global            记忆数据存到 ~/.loong-recall/data/（跨项目共享）
  --db-path <路径>    自定义记忆数据库路径（优先级最高，覆盖 --global）
  --llm-api <配置>    配置 LLM 查询翻译（格式: openai:sk-xxx:model 或 ollama:host:model）
                      也可在仪表盘「设置」页面可视化配置
  --mode <模式>       搜索模式: auto（默认，镜像启动）| fast（秒启动）| smart（语义搜索）
  --proxy <代理地址>   HTTP/HTTPS 代理（如 http://127.0.0.1:7890）
  --daemon            后台守护模式，无控制台窗口
  --multi-window <N>  允许同项目最多 N 个窗口同时运行 [默认: 1]
  --tray              启用系统托盘图标（Windows）
  --help, -h          显示帮助信息
```

六种典型用法：

```bash
# 场景 1：单项目 IDE 标准 MCP（最常用，镜像启动，立即可用）
code-memory-server --src-dir ./src --stdio

# 场景 2：单项目 HTTP 模式调试
code-memory-server --src-dir ./src --port 3099

# 场景 3：全局记忆，跨项目共享 + 自定义存储路径
code-memory-server --global --db-path /data/my-memories --stdio

# 场景 4：快速模式，跳过模型下载，秒启动
code-memory-server --src-dir ./src --stdio --mode fast

# 场景 5：Smart Match 语义搜索，直接加载模型
code-memory-server --src-dir ./src --stdio --mode smart

# 场景 6：LLM 增强，自然语言搜索代码
code-memory-server --src-dir ./src --stdio --llm-api openai:sk-xxx:deepseek-v4-flash:https://api.deepseek.com/v1
```

***

## 切分器支持的语言

按文件扩展名自动选择切分策略，无需手动配置：

| 切分器                   | 扩展名                                                                                           | 识别单元                                                        | 切分方式                   |
| --------------------- | --------------------------------------------------------------------------------------------- | ----------------------------------------------------------- | ---------------------- |
| `RustChunker`         | `.rs`                                                                                         | `fn` / `struct` / `trait` / `enum` / `impl` / `mod`         | 大括号深度匹配 + `///` 文档注释提取 |
| `PythonChunker`       | `.py`                                                                                         | `def` / `async def` / `class`                               | 缩进层级检测 + `#` 注释提取      |
| `TsJsChunker`         | `.ts` `.tsx` `.js` `.jsx` `.mjs` `.cjs`                                                       | `function` / `class` / `interface` / `type` / `enum` / 箭头函数 | 大括号匹配 + JSDoc 提取       |
| `GoChunker`           | `.go`                                                                                         | `func` / `type`（支持接收者方法）                                    | 大括号匹配                  |
| `ConversationChunker` | —                                                                                             | 对话轮次（"用户:" / "助手:" / "系统:" 等）                               | 按角色前缀切分，支持中英文、全角/半角冒号  |
| `GenericChunker`      | `.md` `.txt` `.yaml` `.toml` `.json` `.html` `.css` `.xml` `.sql` `.sh` `.java` `.c` `.cpp` 等 | Markdown 按 `#` 标题切分；其余按段落(`\n\n`)切分                         | 标题边界 / 段落边界            |

***

## 两种编码模式

编译时通过 Cargo feature 切换，决定代码搜索的精度和资源消耗：

| <br />   | 🚀 快速模式（默认）            | 🧠 语义模式（`--features ml`）                  |
| -------- | ---------------------- | ----------------------------------------- |
| **编码器**  | `FastEncoder`（内联词袋编码器） | `CodeBertEncoder`（candle + GraphCodeBERT） |
| **默认模型** | 无（纯词袋匹配）               | **GraphCodeBERT**（比 CodeBERT 检索精度高 12.3%） |
| **外部依赖** | **零**，纯 Rust 实现        | 首次启动自动下载模型（\~500MB）                       |
| **内存占用** | < 10 MB                | \~500 MB（模型加载后）                           |
| **启动时间** | 即时                | 首次 1\~5 分钟（下载模型），后续即时                     |
| **检索精度** | Token 关键词匹配            | 真实语义理解（同义词、中英文自然语言）                       |
| **适用场景** | 精确函数名/变量名查找            | 模糊意图描述（"处理重试逻辑的代码"）                       |

### 🚀 快速模式（默认，推荐日常使用）

```bash
cargo build --features server
```

编译后零配置立即可用。编码器基于代码 token 分割和词袋匹配——如果你习惯用函数名、变量名查代码，精度够用。这也是忆在 Loong Agent OS 中作为默认模式运行的方式。

### 🧠 语义模式（高精度，按需启用）

```bash
cargo build --features server,ml
```

首次启动时自动从 HuggingFace 镜像站（hf-mirror.com）下载模型（\~500MB），存储在本地缓存。下载仅一次，后续启动直接加载缓存。

**默认使用 GraphCodeBERT**（`microsoft/graphcodebert-base`），相比 CodeBERT 在代码检索任务上精度提升 12.3%，模型体积和推理速度完全相同。可通过环境变量切换：

```bash
# 回退到 CodeBERT 基线
$env:LRC_MODEL_ID="microsoft/codebert-base"

# 试验其他 RoBERTa 架构模型（需兼容 candle BertModel）
$env:LRC_MODEL_ID="your-org/your-model"
```

**模型格式兼容**：自动识别 `model.safetensors` 和 `pytorch_model.bin` 两种格式，无需手动转换。

**启动模式选择**：编译 `--features ml` 后，通过 `--mode` 参数控制启动行为：

```bash
# 镜像启动（默认）：Fast Match 立即可用，后台自动升级 Smart Match
code-memory-server --src-dir ./src --stdio

# 纯 Fast Match：跳过模型，秒启动
code-memory-server --src-dir ./src --stdio --mode fast

# 纯 Smart Match：直接加载模型，同步等待
code-memory-server --src-dir ./src --stdio --mode smart
```

语义模式的优势：你可以用自然语言描述意图，而不是记函数名：

* "处理用户登录的逻辑在哪里？" → `fn authenticate_user()`

* "错误重试的代码" → `fn retry_with_backoff()`

> ⚠️ 如果只需按函数名/关键词查代码，快速模式够用，且无需额外模型下载。
> 详细模型评估与替代方案对比见 [模型评估报告](docs/MODEL_EVALUATION.md)。

***

## 原理

Loong Recall 的语义记忆能力源于道体（DaoTi）的规范场论结构——退化基态（Degenerate Ground State）发现。道枢层在消费级 CPU 上完成预训练后，其核心参数被冻结（"道体"），作为稳定的语义编码基础。

Loong Recall 提取了道枢层的编码-检索范式，工程化为独立 MCP 服务：

1. **切分** — 将源码按语法边界切分为独立片段（函数、结构体、类、段落等）
2. **编码** — 每个片段通过语义编码器转换为高维向量
3. **检索** — 查询文本同样编码后，在向量空间中通过余弦相似度匹配 Top-K 片段

会话记忆系统同样利用编码器对记忆内容进行语义索引，使得 `recall` 工具能跨越精确关键词匹配，理解自然语言查询的意图。

### LRC 与道体（DaoTi）的关系

一个常见问题：**使用 LRC 需要安装或部署道体基座模型吗？**

**不需要。** LRC 是一个完全独立的 MCP 服务。道枢层的编码参数已在预训练后被冻结（"道体"），并直接内置于 LRC 的 `FastEncoder` 中。这意味着：

* 快速模式下，LRC 启动即用，无需下载任何模型文件

* CodeBERT/GraphCodeBERT 模式下，LRC 自动从 HuggingFace 镜像站（hf-mirror.com）下载开源模型，同样不依赖 DaoTi

* LRC 与 DaoTi 的关系是**算法传承**而非**运行时依赖**——就像一个发动机被独立封装后装入了不同的车型

如果你对底层模型感兴趣，可以访问 [DaoTi 项目主页](https://github.com/zhibaiYingChuan/DaoTi)。

***

## 记忆系统详解

### 数据类型

| 类型             | 用途示例                                                |
| -------------- | --------------------------------------------------- |
| `fact`         | "此项目使用 PostgreSQL 16"                               |
| `preference`   | "用户偏好 4 空格缩进，不用 Tab"                                |
| `decision`     | "决定用 Axum 而非 Actix，因为生态更活跃"                         |
| `code_context` | "`auth.rs` 中的 `validate_token` 依赖 JWT\_SECRET 环境变量" |
| `conversation` | "上次讨论：需要在下周完成 API 文档"                               |

每条记忆支持：重要性评分（1-10）、项目关联、标签列表、TTL 过期时间。

### 记忆衰减机制

记忆不会永远保持同等权重。Loong Recall 内置指数衰减模型：

* **新鲜记忆**（刚写入或刚被访问）：衰减因子 ≈ 1.0，保持最高权重

* **冷记忆**（长期未访问）：衰减因子随时间递减，在检索结果中自然排后

* **高重要性记忆**（importance ≥ 8）：衰减速度大幅减缓，长期保持较高权重

* **低重要性记忆**（importance ≤ 3）：衰减更快，检索时优先被过滤

这个机制确保你频繁关注的信息始终排在最前面，而不再相关的信息自然沉淀。

### 存储位置

* 默认：项目目录下的 `.loong-recall/data/`（3 个 JSON 文件：`memories.json`、`chunks.json`、`archive.json`）

* `--global` 模式：`~/.loong-recall/data/`（所有项目共用同一记忆库）

* `--db-path` 自定义：你指定的任意路径

数据以 JSON 文件形式存储，可直接用文本编辑器查看和手动修改。

***

## 贡献指南

```bash
# 运行测试（347 项）
cargo test --all-targets --features server

# 代码风格检查
cargo clippy --all-targets --features server -- -D warnings
cargo fmt --check
```

### 运行时安全

Loong Recall 在启动时自动执行多层运行时防护（详见 `src/guard.rs`），保护核心算法不被逆向工程：

| 防护层       | 说明                                                                      |
| --------- | ----------------------------------------------------------------------- |
| **字符串混淆** | 编译时 XOR 加密敏感字符串（`obfuscated!` 宏），运行时动态解密                                |
| **反调试**   | 三级联动检测调试器附加（IsDebuggerPresent + CheckRemoteDebuggerPresent + DebugPort） |
| **反单步**   | 执行时序异常检测，识别单步调试行为                                                       |
| **断点扫描**  | 检测函数入口 `int3` (0xCC) 软件断点                                               |
| **完整性校验** | 代码段 CRC32 自校验 + 源码 SHA-256 哈希验证，防止运行时篡改                                 |
| **控制流混淆** | 基于费马小定理的不透明谓词 + 状态机平坦化，增加反汇编分析难度                                        |

检测到威胁时，服务延迟随机时间后静默退出，不暴露任何检测逻辑。

***

## 文档导航

| 文档                                 | 说明                                  |
| ---------------------------------- | ----------------------------------- |
| [用户使用说明书](docs/USER_GUIDE.md)      | AI 大模型如何主动调用 MCP 服务 — 解决用户最常见的问题    |
| [模型评估报告](docs/MODEL_EVALUATION.md) | CodeBERT vs GraphCodeBERT 对比与替代方案评估 |
| [算法概述](docs/ALGORITHM_OVERVIEW.md) | 记忆架构的高层原理（安全版本，不泄露核心算法）             |
| [性能测试指南](docs/BENCHMARK.md)        | 如何复现性能测试                            |
| [使用场景](docs/USE_CASES.md)          | 典型应用场景与最佳实践                         |

***

## 更新日志

### v0.4.0 (2026-06-15) — 桌面端应用 + 跨 IDE 记忆同步

**桌面端应用**

- 基于 Tauri 2 的原生 Windows 桌面应用，内置配置向导
- 系统托盘支持，最小化后台运行
- 仪表盘内嵌展示，无需浏览器
- API Key AES-256-GCM 加密存储
- Sidecar 进程自动管理（启动/停止/健康检查）

**跨 IDE 记忆同步**

- 项目指纹（SHA256 哈希）识别同一项目，跨 Trae/Cursor/VS Code 共享记忆
- 统一数据目录 `~/.loong-recall/projects/{fingerprint}/data/`
- 旧版数据自动迁移，复制策略保护数据安全

**记忆导出/导入**

- `lrc export` 命令：JSON 格式导出记忆，支持项目级/全局模式
- `lrc import` 命令：导入记忆，追加模式，基于 ID 去重

**改进**

- 端口参数 CLI 优先级高于配置文件
- 仪表盘 API 自动检测 origin，不再硬编码 localhost
- 根路径 `/` 自动跳转 `/dashboard`

### v0.3.1 (2026-06-12) — 一键体验优化：自动打开浏览器 + LLM 可视化配置

**🖥️ 用户体验**

- **自动打开浏览器**：HTTP 模式启动后自动打开默认浏览器访问仪表盘，新用户无需手动输入网址
- **LLM 可视化配置**：仪表盘新增「⚙️ 设置」页面，支持图形界面配置 LLM API，不再需要命令行输入
  - 支持 OpenAI 兼容 API（DeepSeek、通义千问等）和 Ollama 本地模型两种方式
  - 配置后即时生效，无需重启服务
  - API Key 仅保存在本地配置文件，绝不上传服务器
- 更新终端启动提示，引导新用户通过仪表盘配置 LLM

**🐛 问题修复**

- 添加 `[workspace]` 声明到 Cargo.toml，修复克隆到父级 workspace 目录时的 Cargo 冲突

---

### v0.3.0 (2026-06-09) — 桌面端 Agent 全面支持

**✨ 新增核心功能**

- **配置持久化**：保存端口、LLM API、多窗口设置到 `%APPDATA%\LoongRecall\config.json`，重启不丢失
- **后台守护模式**（`--daemon`）：无控制台后台运行，供各种桌面端 Agent 长期调用
- **系统托盘**：Windows 原生托盘图标，右键菜单支持快速打开仪表盘/退出服务
- **多窗口支持**（`--multi-window N`）：允许同项目最多 N 个窗口同时运行 LRC
- **进程守护**：单例锁避免僵尸进程、端口自适应避免冲突、优雅关闭自动清理

**🔧 问题修复**

- 全局镜像守卫：程序入口强制设置 `HF_ENDPOINT=https://hf-mirror.com`，绝不触碰 huggingface.co
- 交互式提示超时：5 秒超时机制，防止 Hidden 窗口环境 stdin 阻塞
- 搜索模式优先级：Fast Match (第1) > LLM API (第2) > Smart Match (最后)
- 模型下载：仅用户确认后才从国内镜像下载，绝不访问外网

---

### v0.2.0 (2026-06-07) — 代码质量与安全加固

**🛡️ 代码质量**

- 全项目静态代码审计（347+ 测试 + Clippy pedantic/nursery），修复全部 Clippy 警告
- 消除所有非测试代码中的 `.unwrap()` / `.expect()` 残留，杜绝生产环境 Panic 风险
- 修复切片越界、类型转换截断等潜在运行时错误

**🔒 核心算法保护**

- 全部 23 个引擎文件添加 DaoTi Research License v1.0 许可证头
- CI 自动运行代码质量与安全检查，确保核心算法安全

**🚦 自动化 CI**

- `.github/workflows/ci.yml`：push/PR 时自动运行编译、测试、Clippy、格式、unwrap 检测、代码重复、XSS 安全等检查
- 不合格代码不得合并

**🖥️ 用户体验**

- 仪表盘新增"📖 指标说明"面板，用大白话解释「道同构度」「八卦分布熵」等专业术语
- 「船长日志生成器」输入框优化：明确提示示例路径 + 辅助说明文字
- 修复仪表盘 `app.js` 404 错误，前端功能完全恢复
- 删除 `index.html` 内联 `<script>`（约 800 行），统一由 `app.js` 管理

**📚 文档**

- 更新 README.md 和 USER_GUIDE.md，补充仪表盘使用说明、快速安装脚本、守门人质量检查
- 新增 CI 守门人综合报告，每次提交自动生成质量裁决

***

## 分层开源许可

| 层级          | 范围                                                                                                         | 许可证                                        |
| ----------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| **L1 公开层**  | `src/chunker.rs`、`src/memory_store.rs`、`src/memory_types.rs`、`src/persistence/`、`src/server.rs`、`src/bin/` | Apache 2.0 — 可修改、可分叉、可商用                   |
| **L2 受保护层** | `src/engine/` 核心编码与检索算法                                                                                    | DaoTi Research License v1.0 — 源码可见，仅限研究/审计 |

> ⚠️ L2 层代码禁止用于逆向工程或训练竞争模型。商业使用需联系项目所有者获取授权。详见 [LICENSE](LICENSE)。

***

*Loong Recall — 忆 · 来自道体，用于龙*
