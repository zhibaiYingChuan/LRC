# Loong Recall 用户说明书

> **AI 编程助手的记忆与检索插件** — 接入 IDE，AI 就能按需检索代码、跨会话记住关键约定。
>
> 版本：v0.2.0 | 适用于：Trae / Cursor / VS Code

---

## 它解决什么？

用 AI 写代码时，你大概率遇到过这两种尴尬：

**尴尬一：不知道代码在哪**

你想让 AI 改某个功能，但不知道它在哪个文件里。你只能手动翻目录 → 搜索关键词 → 复制粘贴几百行代码给 AI。每次都这样。

**尴尬二：AI 每次都失忆**

你跟 AI 聊了很久，约定了"用 pnpm"、"端口 8080"、"数据库 PostgreSQL"。但第二天新开会话，AI 全忘了，你得重新说一遍。

**Loong Recall 就是解决这两个问题的。** 它给 AI 装上两个能力：

| 能力 | 工具 | 一句话说清楚 |
|------|------|------------|
| **代码定位** | `search_code` | 知道函数名/变量名，AI 瞬间定位。配置 LLM 后可用自然语言 |
| **项目记忆** | `remember` / `recall` | 告诉 AI 一次约定，以后每次对话它都记得 |

**最关键的是：你不需要做任何额外操作。** 配好规则后，AI 会自动判断什么时候该搜代码、什么时候该查记忆。你只管正常聊天。

---

## 30 秒理解：它是怎么"无感"工作的

```
你问："我们上次决定用哪个数据库来着？"
        ↓
AI 内部自动调用 recall 工具，检索你的历史记忆
        ↓
AI 回复："（根据记忆 #3）你之前选择了 PostgreSQL，原因是需要 JSONB 和全文搜索"
```

整个过程，你只看到 AI 的回答。中间的工具调用对你是**完全透明的**。

就像你开车时不需要关心发动机怎么点火——你踩油门，车就走。

---

## 两种搜索模式 + LLM 增强

> ⏭️ **还没装好？** 先跳到下方的「三步上手」，装好后再回来看搜索模式的选择。默认 Fast Match 够用，不用纠结。

| | Fast Match（默认） | Smart Match（`--features ml`） | LLM 增强（`--llm-api`） |
|---|---|---|---|
| **怎么搜** | 精确关键词匹配 | 语义理解（理解自然语言意思） | 你的 LLM 翻译查询 → Fast Match |
| **适合** | 你知道函数名/变量名，懒得翻文件 | 离线环境下用自然语言描述意图 | 有 LLM API，用自然语言描述意图 |
| **启动速度** | 即时（毫秒级） | 首次需下载模型（~500MB） | 即时（依赖 LLM 响应） |
| **内存占用** | < 10 MB | ~500 MB | < 10 MB |
| **依赖** | 零，纯 Rust | 自动从 hf-mirror.com 镜像下载 | 需要 LLM API（DeepSeek / 通义千问等）或本地 Ollama |

```bash
# 默认 Fast Match（推荐日常使用）
cargo build --features server

# Smart Match（需要语义理解时）
cargo build --features server,ml

# LLM 增强（用你的 LLM 做查询翻译，不下载模型）
# 推荐：使用 DeepSeek（国产模型，性价比极高）
code-memory-server --src-dir ./src --stdio --llm-api "openai:sk-your-deepseek-key:deepseek-v4-flash:https://api.deepseek.com/v1"
```

> 90% 的日常场景 Fast Match 完全够用。Smart Match 在模糊查询上更有优势，详见 [模型评估报告](MODEL_EVALUATION.md)。
> 内网/离线环境？参考 [Smart Match 离线安装指南](OFFLINE_MODEL_GUIDE.md)。

---

## LLM 增强模式（v0.2.0 新增）

**不想下载 500MB 模型，但又想用自然语言搜索代码？** 配置你的 LLM API，LRC 会自动用你的 LLM 把自然语言翻译成代码关键词，然后用 Fast Match 精确检索。

### 原理（30 秒理解）

```
你问："处理用户登录的那个函数在哪？"
        │
        ▼
  你的 LLM（DeepSeek V4 / 通义千问 / Ollama）
        │  Prompt: "将模糊查询翻译成代码关键词"
        ▼
  "authenticate_user, login, handle_login, auth"
        │
        ▼
  LRC Fast Match 用这些关键词精确检索
        │
        ▼
  返回准确的代码片段
```

LLM 只做查询翻译，不参与存储、检索、或记忆。Prompt 消耗 < 50 Token，每次查询成本几乎为零。

### 配置方式

```bash
# 推荐：使用 DeepSeek V4-Flash（国产模型，性价比极高）
code-memory-server --src-dir ./src --stdio \
  --llm-api "openai:sk-your-deepseek-key:deepseek-v4-flash:https://api.deepseek.com/v1"

# 使用通义千问 Qwen-Turbo（阿里云百炼，¥0.3/百万 Token 输入）
code-memory-server --src-dir ./src --stdio \
  --llm-api "openai:sk-your-qwen-key:qwen-turbo:https://dashscope.aliyuncs.com/compatible-mode/v1"

# 使用本地 Ollama（零成本，完全离线）
code-memory-server --src-dir ./src --stdio \
  --llm-api "ollama:localhost:llama3"
```

### 在 IDE 中配置

在 MCP 配置文件中添加 `--llm-api` 参数即可：

```json
{
  "mcpServers": {
    "loong-recall": {
      "command": "你的安装路径/target/release/code-memory-server.exe",
      "args": [
        "--src-dir", "你的项目路径/src",
        "--stdio",
        "--llm-api", "openai:sk-your-deepseek-key:deepseek-v4-flash:https://api.deepseek.com/v1"
      ]
    }
  }
}
```

### 注意事项

- **不配置 `--llm-api`**：Fast Match 照常用，行为完全不变
- **翻译失败时**：自动回退到原始查询，不影响搜索功能
- **隐私**：只有查询文本发给 LLM，不发送任何代码
- **成本**：DeepSeek V4-Flash 每天 100 次查询 < ¥0.01/月，通义千问 Qwen-Turbo 更是不足 ¥0.01/月。详见下方「成本与优化」章节。

> 💡 如果你已经在用 Trae/Cursor（它们内置了 LLM），这个模式让你零成本获得高精度语义搜索——不需要下载任何模型。

---

## 三步上手（5 分钟）

### 第 1 步：下载

```bash
# 方式一：直接下载编译好的二进制（推荐）
# 从 Release 页面下载 code-memory-server.exe

# 方式二：源码编译
git clone https://github.com/zhibaiYingChuan/LRC.git
# 国内用户如遇 GitHub 下载缓慢，可使用镜像：
# git clone https://gitcode.com/gcw_M73FIiUo/LRC
cd LRC
cargo build --release --features server
```

### 第 2 步：配置 IDE

把你用的 IDE 的配置，复制粘贴进去就行。

#### 🟢 Trae（推荐）

**① 配置 MCP 服务**

打开 `%APPDATA%/Trae/User/mcp.json`（没有就新建），写入：

```json
{
  "mcpServers": {
    "loong-recall": {
      "command": "你的安装路径/target/release/code-memory-server.exe",
      "args": [
        "--src-dir", "你的项目路径/src",
        "--global",
        "--stdio"
      ]
    }
  }
}
```

> ⚠️ 路径必须用正斜杠 `/`，不要用反斜杠 `\`。`你的项目路径` 换成你实际的项目路径。

**② 配置项目规则（让 AI 自动使用记忆系统）**

打开 `你的项目路径/.trae/rules/project-rules.md`，在文件末尾追加：

```markdown
## MCP 记忆与检索插件（Loong Recall）

本项目的 AI 助手已接入 Loong Recall。以下规则让 AI 自动、无感地使用它：

### 自动记忆规则
- 当用户做出技术决策时（如"我们用 PostgreSQL"），自动用 remember 记录
- 当用户表达偏好时（如"我喜欢用 pnpm"），自动用 remember 记录
- 当完成重要功能开发后，自动记录关键设计决策
- 记录时自动添加合适的类型标签（fact/preference/decision/code_context）

### 自动检索规则
- 当用户问"我们之前..."、"上次说到..."、"之前决定..."时，先用 recall 检索
- 当用户问代码位置时（如"XX 功能在哪？"），先用 search_code 搜索
- 当用户提到某个技术概念但不确定项目里有没有时，先用 search_code 搜索
- 引用记忆时使用「（根据记忆 #N）」格式标注来源，让用户看见记忆的存在

### 透明规则
- 调用工具时不要在回复中显式展示工具调用过程
- 把检索结果自然地融入回答，就像你本来就知道一样
- 只在用户明确要求时才列出工具调用详情
```

> 💡 这就是"零操作"的秘密：有了这些规则，AI 会**自动判断**何时该用记忆、何时该搜代码。你只管正常对话。

**③ 重启 Trae**

重启后，底部状态栏 MCP 图标显示绿色，即配置成功。

#### 🔵 Cursor

**① 配置 MCP 服务**

打开 `%APPDATA%/Cursor/mcp.json`，写入：

```json
{
  "mcpServers": {
    "loong-recall": {
      "command": "你的安装路径/target/release/code-memory-server.exe",
      "args": [
        "--src-dir", "你的项目路径/src",
        "--global",
        "--stdio"
      ]
    }
  }
}
```

**② 配置项目规则**

打开 `你的项目路径/.cursor/rules` 目录，新建 `memory.md` 文件：

```markdown
# Memory & Search Rules (Loong Recall)

This project uses Loong Recall for code search and cross-session memory.

- When user makes technical decisions, auto-record with remember tool
- When user expresses preferences, auto-record with remember tool
- When user asks "we previously..." or "last time we...", search with recall first
- When user asks about code location, search with search_code first
- Cite memories as "（根据记忆 #N）" format
- Do NOT show tool call details in responses unless explicitly asked
- Integrate memory results naturally into your answers
```

**③ 重启 Cursor**

打开 Cursor 设置 → MCP → 确认 loong-recall 状态为 Connected。

#### 🟣 VS Code

**① 安装 MCP 扩展**

在 VS Code 扩展市场搜索并安装 "MCP" 相关扩展，或使用 GitHub Copilot Chat 的 MCP 支持。

**② 配置 MCP 服务**

打开 `%APPDATA%/Code/User/settings.json`，添加：

```json
{
  "mcp.servers": {
    "loong-recall": {
      "command": "你的安装路径/target/release/code-memory-server.exe",
      "args": [
        "--src-dir", "你的项目路径/src",
        "--global"
      ]
    }
  }
}
```

**③ 配置 GitHub Copilot 指令**

打开 `你的项目路径/.github/copilot-instructions.md`，写入：

```markdown
This project has Loong Recall for code search and session memory.
When user asks about past decisions, code locations, or project history,
use the MCP tools (recall, search_code, remember) to provide accurate answers.
Cite memories as "（根据记忆 #N）". Do not mention tool usage in responses.
```

### 第 3 步：验证一切正常

重启 IDE 后，随便问一句：

```
你：我们上次聊到哪了？
```

如果 AI 回复中提到了你们之前的对话内容（哪怕说"这是我们第一次对话"），说明 MCP 已正常工作。

想更精确验证？问：

```
你：请列出你当前可用的 MCP 工具
```

如果 AI 列出了 `remember`、`recall`、`search_code` 等工具，说明一切就绪。

> 💡 装好了？回头看看上方的「两种搜索模式」了解不同模式的适用场景。默认 Fast Match 日常够用，想用自然语言搜代码可以配 LLM 增强。

---

## Web 仪表盘：可视化你的记忆系统

启动 LRC 后，浏览器打开 `http://127.0.0.1:3099/dashboard`，你会看到一个完整的 Web 控制台。

### 仪表盘能做什么？

| 功能 | 说明 |
|------|------|
| **记忆健康总览** | 道同构度、八卦分布熵、记忆衰减率等指标实时展示 |
| **船长日志生成器** | 输入项目路径，一键生成代码库记忆健康全景报告 |
| **API 文档浏览器** | 内置 18 个 API 端点的交互式文档，可直接测试 |
| **指标说明** | 每个专业术语都有大白话解释，新用户也能看懂 |

### 专业名词大白话解释

仪表盘自带"📖 指标说明"面板，把晦涩的术语翻译成日常语言：

| 术语 | 大白话 |
|------|--------|
| **道同构度** | 系统记忆的"整齐度"评分。就像整理房间，越高说明记忆越井井有条 |
| **八卦分布熵** | 记忆在 8 个分类中分散得是否均匀。就像图书馆的书，如果全堆在一个角落就不健康 |
| **记忆衰减率** | 不活跃的记忆自然"降温"的速度。重要记忆降温慢，不重要的降温快 |
| **合成覆盖率** | 同类记忆自动合并的比例。越高说明系统越"聪明"，不需要你手动整理 |

> 💡 仪表盘在 HTTP 模式下可用（`--port 3099`）。Stdio 模式（IDE 标准模式）下仪表盘不可用，因为 Stdio 不开放 HTTP 端口。

---

## 🪄 一键安装脚本（不想敲命令？用这个）

如果你不想手动敲命令，可以直接用项目自带的一键安装脚本：

- **Windows**：双击 `install.bat`
- **Linux / macOS**：终端运行 `bash install.sh`

脚本会自动完成：
1. 检测 Rust 环境（没有的话会提示安装）
2. 编译 Loong Recall
3. 搜索本地 IDE（Trae / Cursor / VS Code），自动创建 MCP 配置文件

> ⚠️ 如果 IDE 配置文件已存在，脚本会提示手动合并，不会覆盖你已有的配置。

---

## 代码质量保障：守门人系统

LRC 项目内置了自动化质量守门系统，确保每次提交的代码都经过严格检查。

### 对用户意味着什么？

你不需要关心守门人怎么工作——它在你背后运行。但你应该知道：
- **生产环境不会随机崩溃**：所有可能导致 Panic 的 `.unwrap()` 已被消除
- **核心算法不会泄露**：每次提交自动检测，确保算法保护完整
- **代码风格一致**：自动格式化 + Clippy 静态分析，代码质量统一

### 贡献者如何使用？

如果你要修改 LRC 源码并提交 PR：

```bash
# 运行全部质量检查
.\scripts\gatekeeper.ps1

# 自动修复可修复的问题
.\scripts\gatekeeper.ps1 -Fix

# 提交前自动检查（需先安装钩子）
python scripts/install_hooks.py
```

守门人检查在 GitHub Actions 上也会自动运行，不合格的 PR 无法合并。

---

## 核心原理：为什么"零操作"能实现？

### 对比：有规则 vs 无规则

| 场景 | 无 MCP 规则 | 有 MCP 规则 |
|------|-----------|-----------|
| 你问 "我们之前决定用哪个数据库？" | AI 说 "我不记得了" | AI 自动调用 recall，回答 "（根据记忆 #3）PostgreSQL，上次你选了它因为..." |
| 你说 "帮我找一下认证中间件" | AI 猜一个文件路径，可能不对 | AI 自动调用 search_code，精准定位到 `src/auth/middleware.rs` |
| 你说 "我们以后用 pnpm 吧" | AI 说 "好的"，下次忘了 | AI 自动调用 remember，下次主动提醒 |
| 你问 "这个项目的错误处理怎么做的？" | AI 只能看当前打开的文件 | AI 搜索整个代码库，给出全局视图 |

### 规则就是 AI 的工作说明书

MCP 规则告诉 AI 三件事：

1. **你有什么工具**（`remember`、`recall`、`search_code` 等）
2. **什么时候该用**（技术决策 → remember，回顾历史 → recall，找代码 → search_code）
3. **怎么用不出戏**（透明规则让 AI 自然地融入回答，不破坏对话体验）

**你不需要**：
- ❌ 手动输入 `/remember` 指令
- ❌ 每次都说"请用 recall 搜索一下"
- ❌ 关心 AI 调了哪个工具、传了什么参数
- ❌ 切换会话时重新告诉 AI 上下文

**你只需要**：
- ✅ 正常对话
- ✅ 该做什么做什么

---

## 实际效果演示

### 场景 1：跨会话记忆

```
=== 第 1 天 ===
你：这个项目我想用 Rust 2024 edition，加上 Axum 框架
AI：好的，Rust 2024 + Axum，已记录。
    [内部：自动调用 remember 记录技术栈决策]

=== 第 3 天（新会话） ===
你：继续写 API 吧，我们之前说的框架是什么来着？
AI：（根据记忆 #1）你之前选择了 Axum 框架，Rust 2024 edition。
    需要我帮你搭建基础路由结构吗？
    [内部：自动调用 recall 检索到第 1 天的记忆]
```

### 场景 2：代码定位

```
你：search_code 搜索 authenticate_user
AI：找到了，在 src/auth/token.rs 的 authenticate_user 函数（第 42 行）。
    它会在 token 过期前 5 分钟自动刷新，错误重试 3 次。
    [内部：自动调用 search_code 搜索 "authenticate_user"]
```

> 💡 配置 LLM 增强（`--llm-api`）后，你也可以直接用自然语言："项目里处理 token 刷新的逻辑在哪？"

### 场景 3：决策追溯

```
你：我们为什么选了 PostgreSQL 而不是 MySQL？
AI：（根据记忆 #2）你在 5 月 15 日决定用 PostgreSQL，
    主要原因是项目需要 JSONB 类型和全文搜索能力。
    [内部：自动调用 recall 检索到当时的决策记录和理由]
```

---

## 成本与优化

如果你使用 LLM 增强模式，了解成本情况有助于你做出最佳选择。

### LLM 翻译器的成本模型

LLM 增强模式的原理是：把你的自然语言查询发送给 LLM，翻译成代码关键词，再用 Fast Match 精确检索。每次翻译消耗约 **40-50 Token**（约 30 Token 输入 + 15 Token 输出）。

| 模型 | 单次翻译成本 | 每天 100 次 | 每月 3000 次 |
|------|------------|-----------|------------|
| DeepSeek V4-Flash | < ¥0.00007 | < ¥0.007 | < ¥0.21 |
| 通义千问 Qwen-Turbo | < ¥0.00002 | < ¥0.002 | < ¥0.06 |
| 本地 Ollama（千问/LLaMA） | **免费** | **免费** | **免费** |

> 📊 **价格来源**：以上数据基于各平台 2026 年 6 月官方 API 定价计算。DeepSeek V4-Flash：输入 ¥1/百万 Token、输出 ¥2/百万 Token；通义千问 Qwen-Turbo：输入 ¥0.3/百万 Token、输出 ¥0.6/百万 Token。实际费用以各平台最新公告为准。

### 有无 LRC 的成本对比

用一个实际的日常使用场景来说明：

| 场景 | 无 LRC 的做法 | Token 消耗 | 有 LRC + LLM 翻译 | Token 消耗 |
|------|------------|-----------|-----------------|-----------|
| 找"登录验证逻辑" | 手动翻文件，复制粘贴 `auth.rs` 整个文件给 AI | 500-2000 Token | LLM 翻译成关键词 → Fast Match → 返回 5 个精确片段 | < 50 Token（翻译）+ < 200 Token（结果） |
| 找"数据库连接池配置" | 粘贴 `database.rs` + `config.rs` | 1000-3000 Token | LLM 翻译 → 检索 | < 50 Token + < 200 Token |

**结论**：LLM 翻译器帮你把"整个文件粘贴给 AI"变成了"只返回精确的 5 个代码片段"。每次查询节省的上下文 Token 是翻译本身消耗的 10-50 倍。

### 缓存命中率的改善

因为 LRC 的检索结果可复现（相同查询 → 相同结果），AI 助手端（如 Trae）的上下文前缀缓存命中率会大幅提升，进一步节省 Token 和加速响应。

### 如何用本地 Ollama 实现零成本

```bash
# 1. 安装 Ollama（https://ollama.com）
# 2. 拉取国产模型（推荐）
ollama pull qwen3
# 或拉取 LLaMA
ollama pull llama3

# 3. 启动 LRC 时指定 Ollama
code-memory-server --src-dir ./src --stdio --llm-api ollama:localhost:qwen3
```

配置完成后，搜索时完全零成本，且不依赖网络。

---

## 常见问题

### Q：我需要每次都说"请用 recall 搜索"吗？

**不需要。** 配置好规则文件后，AI 会自动判断何时该搜索。你只需要正常问。

### Q：AI 会不会滥用记忆，把无关的东西也记下来？

**不会。** 规则中明确写了触发条件（"技术决策"、"用户偏好"），AI 不会记录闲聊内容。你也可以在规则中调整触发条件。

### Q：记忆存在哪里？安全吗？

所有记忆存储在本地（默认项目目录下的 `.loong-recall/data/` 或全局 `~/.loong-recall/data/`），不上传任何服务器。你可以随时用 `forget` 删除、用 `update_memory` 修改。

### Q：我换了电脑，记忆能迁移吗？

可以。使用 `--global` 模式时，复制 `~/.loong-recall/` 目录到新电脑即可。后续版本会支持云端同步。

### Q：Fast Match 和 Smart Match 怎么选？

| 日常推荐 | 特殊场景 |
|---------|---------|
| **Fast Match（默认）** | **Smart Match（`--features ml`）** |
| 搜函数名、变量名 | 模糊描述（"处理重试的代码"） |
| 零延迟、零依赖 | 首次需下载模型 |
| 90% 场景够用 | 复杂项目语义搜索 |

> 详见 [模型评估报告](MODEL_EVALUATION.md) 和 [离线安装指南](OFFLINE_MODEL_GUIDE.md)。

### Q：为什么 AI 回复里出现了"（根据记忆 #3）"？

这是我们设计的功能——让记忆**可见**。当 AI 引用了你之前保存的记忆时，会标注来源编号。这样你能看到记忆在起作用，也能信任它的准确性。

---

## 故障排查

遇到问题？先看这里。

### Q：编译失败怎么办？

```bash
# 错误："could not find `server` in `features`"
# 原因：Rust 版本太旧
rustup update stable

# 错误：crates.io 下载超时
# 解决：配置国内镜像（见 5 分钟体验中的 Cargo 镜像配置）

# 错误：link.exe 找不到
# 解决：安装 Visual Studio Build Tools，勾选"C++ 桌面开发"
```

### Q：启动时报"端口被占用"？

```bash
# 错误：Address already in use (os error 10048)
# 原因：3099 端口已被其他程序占用
# 解决：换个端口
code-memory-server --src-dir ./src --port 3098

# 或者：找到占用端口的程序并关闭
netstat -ano | findstr :3099
taskkill /PID <进程ID> /F
```

### Q：IDE 中 MCP 图标灰色/不显示？

1. 检查 MCP 配置文件路径是否正确（注意是正斜杠 `/` 不是反斜杠 `\`）
2. 检查 `code-memory-server.exe` 文件是否存在
3. 在终端手动运行 MCP 命令，看是否有报错
4. 重启 IDE（有时需要完全退出再打开）
5. 确认 IDE 的 MCP 功能已启用（Trae 默认启用，Cursor 需在设置中开启）

### Q：仪表盘打开是空白页？

1. 确认服务以 HTTP 模式启动（加了 `--port 3099` 参数）
2. Stdio 模式下仪表盘不可用（IDE 标准模式不开放 HTTP 端口）
3. 尝试强制刷新浏览器（Ctrl+F5）
4. 检查浏览器控制台是否有报错（F12 → Console）

### Q：记忆突然搜不到了？

1. 检查记忆文件是否存在：`.loong-recall/data/memories.json`
2. 确认没有误删记忆文件
3. 如果使用了 `--global` 模式，检查 `~/.loong-recall/data/` 路径
4. 搜索词太模糊？试试用更精确的关键词，或开启 LLM 增强模式

---

## 规则维护指南

### 调整触发条件

如果你觉得 AI 记太多或记太少，修改规则文件中的触发条件即可：

```markdown
### 自动记忆规则（调整后——更保守）
- 只有当用户明确说"记住这个"或"帮我记录一下"时，才用 remember 记录
- 不要自动记录，除非用户明确表达
```

### 针对不同项目定制

不同项目可以用不同的规则：

```markdown
# 前端项目规则（追加）
- 当用户提到组件设计时，用 search_code 搜索已有的类似组件
- 当用户做出 UI 决策时，自动记录（component_preference 类型）

# 后端项目规则（追加）
- 当用户提到 API 设计时，用 search_code 搜索已有的路由定义
- 当用户做出数据库决策时，自动记录（database_decision 类型）
```

### 规则更新后需要重启吗？

不需要。规则文件是 `.md` 格式，Trae 每次对话会自动读取最新内容。改了就能生效。

---

## 一句话总结

> **装上 MCP，配好规则，然后忘掉它的存在。** 你只管正常写代码、正常聊天，AI 会自己记住该记住的、找到该找到的。这就是"零操作"——不是没有功能，而是功能自然到你不觉得它是一个功能。

---

## 参考链接

- [模型评估报告](MODEL_EVALUATION.md) — 为什么默认用 GraphCodeBERT
- [Smart Match 离线安装指南](OFFLINE_MODEL_GUIDE.md) — 内网/离线环境配置
- [性能测试指南](BENCHMARK.md) — 百万条记忆 < 30ms
- [算法概述](ALGORITHM_OVERVIEW.md) — 记忆系统的高层原理

---

## 更新历史

### v0.2.0 (2026-06-07)

**新增功能**
- LLM 增强模式：用你的 LLM 做自然语言查询翻译，不下载模型也能语义搜索
- Web 仪表盘：`http://127.0.0.1:3099/dashboard` 可视化记忆健康度
- 船长日志生成器：一键生成项目代码库记忆全景报告
- 一键安装脚本：`install.bat`（Windows）/ `install.sh`（Linux/macOS）
- 自动化守门人系统：10 道质量检查，CI 自动运行

**问题修复**
- 修复仪表盘 `app.js` 404 错误，前端功能完全恢复
- 消除 60+ 处 `.unwrap()` 残留，杜绝生产环境 Panic 风险
- 修复全部 Clippy 警告（标准模式 + 严格模式）
- 修复切片越界、类型转换截断等潜在运行时错误

**文档更新**
- 新增仪表盘使用说明和专业名词大白话解释
- 新增一键安装脚本使用说明
- 新增代码质量守门人系统说明
- README.md 新增完整更新日志