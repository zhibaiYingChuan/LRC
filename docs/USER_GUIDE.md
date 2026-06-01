# Loong Recall 用户说明书

> **AI 编程助手的记忆与检索插件** — 接入 IDE，AI 就能记住一切、找到一切。
>
> 版本：v0.1.1 | 适用于：Trae / Cursor / VS Code

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
| **代码定位** | `search_code` | 忘了函数名？说个大概，AI 帮你找到它 |
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

## 两种搜索模式

| | Fast Match（默认） | Smart Match（`--features ml`） |
|---|---|---|
| **怎么搜** | 精确关键词匹配 | 语义理解（理解自然语言意思） |
| **适合** | 你知道函数名/变量名，懒得翻文件 | 用自然语言描述意图（"处理重试的代码"） |
| **启动速度** | 即时（毫秒级） | 首次需下载模型（~500MB） |
| **内存占用** | < 10 MB | ~500 MB |
| **依赖** | 零，纯 Rust | 自动从 hf-mirror.com 镜像下载 |

```bash
# 默认 Fast Match（推荐日常使用）
cargo build --features server

# Smart Match（需要语义理解时）
cargo build --features server,ml
```

> 90% 的日常场景 Fast Match 完全够用。Smart Match 在模糊查询上更有优势，详见 [模型评估报告](MODEL_EVALUATION.md)。
> 内网/离线环境？参考 [Smart Match 离线安装指南](OFFLINE_MODEL_GUIDE.md)。

---

## 三步上手（5 分钟）

### 第 1 步：下载

```bash
# 方式一：直接下载编译好的二进制（推荐）
# 从 Release 页面下载 code-memory-server.exe

# 方式二：源码编译
git clone https://gitcode.com/loong/loong-recall.git
cd loong-recall
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
      "command": "G:/code-memory/target/release/code-memory-server.exe",
      "args": [
        "--src-dir", "G:/your-project/src",
        "--global",
        "--stdio"
      ]
    }
  }
}
```

> ⚠️ 路径必须用正斜杠 `/`，不要用反斜杠 `\`。`your-project` 换成你实际的项目路径。

**② 配置项目规则（让 AI 自动使用记忆系统）**

打开 `G:/your-project/.trae/rules/project-rules.md`，在文件末尾追加：

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
      "command": "G:/code-memory/target/release/code-memory-server.exe",
      "args": [
        "--src-dir", "G:/your-project/src",
        "--global",
        "--stdio"
      ]
    }
  }
}
```

**② 配置项目规则**

打开 `G:/your-project/.cursor/rules` 目录，新建 `memory.md` 文件：

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
      "command": "G:/code-memory/target/release/code-memory-server.exe",
      "args": [
        "--src-dir", "G:/your-project/src",
        "--global"
      ]
    }
  }
}
```

**③ 配置 GitHub Copilot 指令**

打开 `G:/your-project/.github/copilot-instructions.md`，写入：

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
你：项目里处理 token 刷新的逻辑在哪？
AI：找到了，在 src/auth/token.rs 的 refresh_token 函数（第 42 行）。
    它会在 token 过期前 5 分钟自动刷新，错误重试 3 次。
    [内部：自动调用 search_code 搜索 "token 刷新"]
```

### 场景 3：决策追溯

```
你：我们为什么选了 PostgreSQL 而不是 MySQL？
AI：（根据记忆 #2）你在 5 月 15 日决定用 PostgreSQL，
    主要原因是项目需要 JSONB 类型和全文搜索能力。
    [内部：自动调用 recall 检索到当时的决策记录和理由]
```

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