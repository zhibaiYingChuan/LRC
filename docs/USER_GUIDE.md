# Loong Recall 用户说明书

> **AI 编程助手的记忆与检索插件** — 接入 IDE，AI 就能按需检索代码、跨会话记住关键约定。
>
> 版本：v0.9.4 | 适用于：Trae / Cursor / VS Code / Claude Desktop 等支持 MCP 协议的 AI 工具

---

## 推荐方式：LRC Desktop 桌面端（v0.5.0+）

**最简单的方式**：下载并安装 [LRC Desktop 桌面端](https://github.com/zhibaiYingChuan/LRC/releases)，启动后会自动完成所有配置：

1. **自动检测 AI 工具**：扫描本机已安装的 Trae / Cursor / VS Code / Claude Desktop 等 36+ 工具（IDE 优先）
2. **自动写入 MCP 配置**：使用 HTTP 模式（`http://127.0.0.1:3099/mcp`），无需手动编辑任何 JSON 文件
3. **自动写入 AI 规则文件**：在用户目录下生成 `~/.trae-cn/user_rules/rules.md` 等规则文件，引导 AI 主动调用 `recall` / `remember`
4. **自动升级旧配置**：从旧版本升级时，sidecar 启动会自动把 stdio 模式 `loong-recall` 升级为 HTTP 模式 `lrc-memory`
5. **内置本地语义模型**：无需配置即可使用代码语义搜索（详见下文「本地语义模型」）

> **v0.6.0 重要变更**：LLM 配置全面国产化，支持 DeepSeek、通义千问、智谱 GLM、MiniMax、Moonshot、豆包、阶跃星辰、百川智能、讯飞星火、腾讯混元等国产模型提供商。

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
| **代码定位** | `search_code` | 知道函数名/变量名，AI 快速定位。配置 LLM 后可用自然语言 |
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
| **怎么搜** | 精确关键词匹配 | 本地语义理解（理解自然语言意思） | 你的 LLM 翻译查询 → Fast Match |
| **适合** | 你知道函数名/变量名，懒得翻文件 | 离线环境下用自然语言描述意图 | 有 LLM API，用自然语言描述意图 |
| **启动速度** | 即时 | 首次需下载模型（约 95MB） | 即时（依赖 LLM 响应） |
| **内存占用** | < 10 MB | 约 200 MB | < 10 MB |
| **依赖** | 零，纯 Rust | 自动从国内镜像下载 | 需要 LLM API（DeepSeek / 通义千问等国产模型） |

```bash
# 默认 Fast Match（推荐日常使用）
cargo build --features server

# Smart Match（需要语义理解时）
cargo build --features server,ml

# LLM 增强（用你的 LLM 做查询翻译，不下载模型）
# 推荐：使用 DeepSeek（国产模型，性价比极高）
code-memory-server --src-dir ./src --stdio --llm-api "openai:sk-your-deepseek-key:deepseek-chat:https://api.deepseek.com/v1"
```

> 日常场景 Fast Match 够用。Smart Match 在模糊查询上更有优势，且 v0.6.0 起默认使用轻量级本地嵌入模型（约 95MB），首次启动从国内镜像自动下载。
> 内网/离线环境？参考 [Smart Match 离线安装指南](OFFLINE_MODEL_GUIDE.md)。

---

## 本地语义模型（v0.6.0 内置）

LRC 内置本地小模型用于代码语义搜索，**无需任何配置即可使用**。默认根据系统语言自动选择：

| 系统语言 | 模型 | 维度 | 体积 | 说明 |
|---------|------|------|------|------|
| **中文环境** | `BAAI/bge-small-zh` | 512 维 | 约 95MB | 中文 SOTA 嵌入模型 |
| **其他语言** | `sentence-transformers/all-MiniLM-L6-v2` | 384 维 | 约 80MB | 多语言轻量模型 |

### 模型下载策略

- **国内镜像**：首次启动时从 HfMirror / ModelScope 国内镜像下载，避免访问国际网络
- **断点续传**：支持下载中断后续传，不重复下载
- **指数退避重试**：下载失败时按 2s / 4s / 8s 间隔自动重试
- **TF-IDF 降级**：若所有重试均失败，自动回退到 TF-IDF 统计算法，保证基础功能可用
- **手动切换**：通过环境变量 `LRC_LUOSHU_MODEL_ID` 可切换其他嵌入模型

### 环境变量参考

| 变量名 | 用途 | 默认值 | 说明 |
|--------|------|--------|------|
| `LRC_LUOSHU_MODEL_ID` | 嵌入模型标识 | `BAAI/bge-small-zh` | 切换语义搜索使用的嵌入模型 |
| `LRC_NETWORK_REQUESTS` | 网络请求追踪 | 空（未设置） | 信任中心内部使用，以 `|` 分隔记录网络请求地址。通常无需手动设置，用于测试验证网络请求白名单 |

> 💡 本地语义模型仅用于代码搜索和记忆结晶聚类，不涉及 LLM 对话功能。LLM 对话能力需通过 LLM 配置启用。

---

## LLM 增强模式（v0.2.0 新增）

**不想下载模型，但又想用自然语言搜索代码？** 配置你的 LLM API，LRC 会自动用你的 LLM 把自然语言翻译成代码关键词，然后用 Fast Match 精确检索。

### 原理（30 秒理解）

```
你问："处理用户登录的那个函数在哪？"
        │
        ▼
  你的 LLM（DeepSeek / 通义千问）
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

LLM 只做查询翻译，不参与存储、检索、或记忆。Prompt 消耗 < 50 Token，每次查询成本极低。

### 支持的国产模型提供商

v0.6.0 起，LRC 全面支持国产模型提供商，摒弃国外模型：

| 提供商 | 推荐场景 | 获取 API Key |
|--------|---------|-------------|
| **DeepSeek** | 代码能力极强，性价比之王 | [获取 →](https://platform.deepseek.com/api_keys) |
| **通义千问** | 阿里云出品，中文理解出色 | [获取 →](https://dashscope.console.aliyun.com/apiKey) |
| **智谱 GLM** | 清华系，GLM 系列模型 | [获取 →](https://open.bigmodel.cn/usercenter/apikeys) |
| **MiniMax** | 海螺AI同款，长文本支持好 | 设置面板中查看 |
| **Moonshot (Kimi)** | 超长上下文，阅读能力强 | 设置面板中查看 |
| **豆包** | 字节跳动出品，性价比高 | 设置面板中查看 |
| **阶跃星辰** | 多模态能力强 | 设置面板中查看 |
| **百川智能** | 金融医疗领域强 | 设置面板中查看 |
| **讯飞星火** | 科大讯飞出品，语音能力强 | 设置面板中查看 |
| **腾讯混元** | 腾讯出品，腾讯生态深度集成 | 设置面板中查看 |
| **自定义 API** | 任何兼容 OpenAI 协议的 API 地址 | — |

### 配置方式

**方式一：LRC Desktop 桌面端可视化配置（推荐）**

1. 启动 LRC Desktop 桌面端
2. 配置向导第 2 步选择模型提供商（或完成后在「设置」页面修改）
3. 填写 API Key 和模型名称 → 点击「保存配置」
4. 配置即时生效，无需重启服务

**方式二：仪表盘可视化配置**

1. 启动 HTTP 模式：`code-memory-server --src-dir ./src --port 3099`
2. 浏览器自动打开仪表盘 → 点击「⚙️ 设置」标签
3. 选择模型提供商（国产模型）
4. 填写 API Key 和模型名称 → 点击「保存配置」
5. 配置即时生效，无需重启服务

**方式三：命令行配置**

```bash
# 推荐：使用 DeepSeek（国产模型，性价比极高）
code-memory-server --src-dir ./src --stdio \
  --llm-api "openai:sk-your-deepseek-key:deepseek-chat:https://api.deepseek.com/v1"

# 使用通义千问 Qwen-Turbo（阿里云百炼，¥0.3/百万 Token 输入）
code-memory-server --src-dir ./src --stdio \
  --llm-api "openai:sk-your-qwen-key:qwen-turbo:https://dashscope.aliyuncs.com/compatible-mode/v1"
```

### 在 IDE 中配置

> **注意**：以下 stdio 模式配置仅适用于从源码编译并直接通过命令行启动的场景。如果你使用 LRC Desktop 桌面端，它会自动使用 HTTP 模式配置，无需手动添加 `--llm-api` 参数（LLM 配置通过仪表盘设置页面完成）。

在 MCP 配置文件中添加 `--llm-api` 参数即可（stdio 模式）：

```json
{
  "mcpServers": {
    "lrc-memory": {
      "command": "你的安装路径/target/release/code-memory-server.exe",
      "args": [
        "--src-dir", "你的项目路径/src",
        "--stdio",
        "--llm-api", "openai:sk-your-deepseek-key:deepseek-chat:https://api.deepseek.com/v1"
      ]
    }
  }
}
```

### 注意事项

- **不配置 `--llm-api`**：Fast Match 照常用，行为完全不变
- **翻译失败时**：自动回退到原始查询，不影响搜索功能
- **隐私**：只有查询文本发给 LLM，不发送任何代码
- **成本**：DeepSeek 每天 100 次查询 < ¥0.01/月，通义千问 Qwen-Turbo 更是不足 ¥0.01/月。详见下方「成本与优化」章节。
- **安全**：API Key 使用 AES-256-GCM 加密存储在本地，绝不上传服务器

> 💡 如果你已经在用 Trae/Cursor（它们内置了 LLM），这个模式让你无需额外下载模型即可获得语义搜索。

---

## 三步上手（5 分钟）

### 第 1 步：下载

**方式一：LRC Desktop 桌面端（推荐，零配置）**

从 [Releases 页面](https://github.com/zhibaiYingChuan/LRC/releases) 下载对应平台的安装包：
- Windows：`LRC Desktop_0.6.0_x64_zh-CN.msi` 或 `LRC Desktop_0.6.0_x64-setup.exe`
- macOS：`LRC Desktop_0.6.0_x64.dmg`（Intel）/ `LRC Desktop_0.6.0_aarch64.dmg`（Apple Silicon）
- Linux：`LRC Desktop_0.6.0_amd64.AppImage`

**方式二：源码编译（需要 Rust 环境）**

```bash
git clone https://github.com/zhibaiYingChuan/LRC.git
# 国内用户如遇 GitHub 下载缓慢，可使用镜像：
# git clone https://gitcode.com/gcw_M73FIiUo/LRC
cd LRC
cargo build --release --features server
```

### 第 2 步：配置 IDE

> **推荐**：如果你使用 LRC Desktop 桌面端，这一步会自动完成，跳到第 3 步。
>
> 以下手动配置方式仅适用于从源码编译并直接通过命令行启动的高级用户。

把你用的 IDE 的配置，复制粘贴进去就行。

#### 🟢 Trae（推荐）

**① 配置 MCP 服务**

打开 `%APPDATA%/Trae/User/mcp.json`（没有就新建），写入：

```json
{
  "mcpServers": {
    "lrc-memory": {
      "type": "http",
      "url": "http://127.0.0.1:3099/mcp"
    }
  }
}
```

**② 配置 AI 规则**

在项目根目录创建 `.trae/rules/lrc-memory.md`（或用户级 `~/.trae/rules/lrc-memory.md`），写入：

```markdown
---
alwaysApply: true
---
# LRC 记忆系统规则
（详见下方「LRC 记忆系统规则」章节）
```

#### 🔵 Cursor

在项目根目录创建 `.cursor/mcp.json`：

```json
{
  "mcpServers": {
    "lrc-memory": {
      "type": "http",
      "url": "http://127.0.0.1:3099/mcp"
    }
  }
}
```

> ⚠️ Cursor 需在项目目录下手动创建 mcp.json，LRC 无法自动写入项目级配置。

#### 🟣 VS Code

在项目根目录创建 `.vscode/mcp.json`：

```json
{
  "mcpServers": {
    "lrc-memory": {
      "type": "http",
      "url": "http://127.0.0.1:3099/mcp"
    }
  }
}
```

> ⚠️ VS Code 需在项目目录下手动创建 mcp.json，LRC 无法自动写入项目级配置。

### 第 3 步：验证一切正常

1. 启动 LRC Desktop（或运行 `code-memory-server --src-dir ./src --port 3099`）
2. 浏览器自动打开 `http://127.0.0.1:3099/dashboard`
3. 在 IDE 中打开 AI 对话窗口
4. 问 AI："我们之前的项目技术栈是什么？"
5. AI 应该自动调用 `recall` 工具检索记忆（首次使用无记忆时会提示"暂无相关记忆"）

---

## LRC 记忆系统（Loong Recall Code Memory）

### 规则 1：会话开始时必须 recall

**在回复用户的任何问题之前，必须先调用 recall 检索项目记忆，获取已有上下文。** 这是强制性的第一步，不可跳过。

### 规则 2：遇到不确定的模块时优先 recall

当需要了解某个模块、函数、概念或实现逻辑时，**优先使用 recall 检索记忆**，而不是直接读取源文件。

### 规则 3：完成任务后自动同步记忆

每次完成代码修改任务后，**自动检查并同步记忆库**，无需用户提醒。

### 记忆工具说明

| 工具 | 用途 | 关键参数 |
|------|------|---------|
| `remember` | 记录新记忆 | content（内容）、memory_type（类型）、tags（标签）、importance（重要性 1-10） |
| `recall` | 语义检索历史记忆 | query（自然语言查询）、top_k（返回数量，建议 3-5） |
| `update_memory` | 更新已有记忆 | memory_id（记忆 ID）、content（新内容） |
| `forget` | 删除记忆 | memory_id（记忆 ID） |
| `list_memories` | 列出记忆库 | 支持分页、过滤、排序 |
| `search_code` | 代码语义搜索 | query（查询关键词） |

**记忆类型**：
- `code_context` — 代码位置和结构
- `decision` — 架构决策
- `preference` — 约定偏好
- `fact` — 事实信息

---

## Web 仪表盘：可视化你的记忆系统

启动 LRC 后，浏览器**自动打开** `http://127.0.0.1:3099/dashboard`，你会看到一个完整的 Web 控制台。

### 仪表盘能做什么？

| 功能 | 说明 |
|------|------|
| **记忆健康总览** | 道同构度、八卦分布熵、记忆衰减率等指标实时展示 |
| **船长日志生成器** | 输入项目路径，生成代码库记忆健康全景报告 |
| **用户文档** | 内置完整用户指南（快速上手 + 配置指南 + FAQ + 使用技巧 + API 参考），v0.6.0 替代旧版 API 文档 |
| **⚙️ 设置页面** | 可视化配置 LLM API（国产模型提供商），即时生效 |
| **指标说明** | 每个专业术语都有大白话解释，新用户也能看懂 |

### v0.6.0 仪表盘新功能

1. **状态栏交互**：
   - 点击左下角"已停止 / 不可达"文本 → 弹出启动服务弹窗，一键启动 sidecar
   - 点击右下角"数据目录"路径 → 直接打开数据文件夹

2. **用户文档页面**：v0.6.0 将原"API 文档"页面改造为综合「用户文档」模块，包含：
   - 一、快速上手
   - 二、配置指南（LLM 配置 + 本地语义模型 + MCP 配置 + 数据存储位置 + 端口自适应）
   - 三、常见问题（FAQ）
   - 四、使用技巧
   - 五、REST API 参考
   - 六、更多资源

### ⚙️ 设置页面：LLM 可视化配置

点击导航栏的「⚙️ 设置」标签，你可以：

1. **选择模型提供商**：国产模型（DeepSeek、通义千问、智谱 GLM、MiniMax、Moonshot、豆包、阶跃星辰、百川智能、讯飞星火、腾讯混元）
2. **填写 API Key**：API Key 使用 AES-256-GCM 加密存储在本地，绝不上传服务器
3. **点击保存**：配置即时生效，无需重启服务

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

## 🪄 快速安装脚本（不想手动敲命令？用这个）

如果你不想手动敲命令，可以用项目自带的安装脚本（需已安装 Rust 环境）：

- **Windows**：双击 `install.bat`
- **Linux / macOS**：终端运行 `bash install.sh`

脚本会自动完成：
1. 检测 Rust 环境（未安装则提示并退出）
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
# 运行全部测试
cargo test --all-targets --features server

# 代码风格检查
cargo clippy --all-targets --features server -- -D warnings
cargo fmt --check
```

守门人检查在 GitHub Actions 上也会自动运行，不合格的 PR 无法合并。

---

## 核心原理：自动化是怎么实现的？

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
（会话 A）
你：我们以后用 pnpm 吧，别用 npm 了
AI：（自动 remember）好的，已记录你的偏好：使用 pnpm。

（会话 B，第二天）
你：帮我初始化一个新项目
AI：（自动 recall）根据你的偏好，使用 pnpm 初始化...
```

### 场景 2：代码定位

```
你：处理用户登录的那个函数在哪？
AI：（自动 search_code "login authenticate"）
找到 3 个相关位置：
1. src/auth/login.rs:45 - `handle_login()` 主入口
2. src/middleware/auth.rs:23 - `verify_token()` 中间件
3. src/models/user.rs:78 - `find_user()` 数据查询
```

### 场景 3：决策追溯

```
你：我们上次为什么选了 PostgreSQL？
AI：（自动 recall "数据库 PostgreSQL 决策"）
（根据记忆 #3）你之前选择了 PostgreSQL，原因是：
1. 需要 JSONB 类型存储复杂 JSON
2. 需要全文搜索功能
3. 团队有 PostgreSQL 运维经验
```

---

## 成本与优化

如果你使用 LLM 增强模式，了解成本情况有助于你做出合适选择。

### LLM 翻译器的成本模型

LLM 增强模式的原理是：把你的自然语言查询发送给 LLM，翻译成代码关键词，再用 Fast Match 精确检索。每次翻译消耗约 **40-50 Token**（约 30 Token 输入 + 15 Token 输出）。

| 模型 | 单次翻译成本 | 每天 100 次 | 每月 3000 次 |
|------|------------|-----------|------------|
| DeepSeek V4-Flash | < ¥0.00007 | < ¥0.007 | < ¥0.21 |
| 通义千问 Qwen-Turbo | < ¥0.00002 | < ¥0.002 | < ¥0.06 |

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

---

## v0.6.0 新增 REST API 端点

以下端点在 v0.6.0 迭代中新增，供仪表盘和前端界面使用：

### 记忆备份与恢复

| 端点 | 方法 | 用途 |
|------|------|------|
| `/v1/memories/list` | POST | 获取全量记忆列表（备份导出用），body: `{"limit": 10000}` |
| `/v1/memories/archive` | POST | 获取归档记忆列表（备份导出用） |
| `/v1/memories/remember` | POST | 写入单条记忆（导入恢复用），body: `{"content": "...", "memory_type": "fact", "importance": 5}` |

### 嵌入模型管理

| 端点 | 方法 | 用途 |
|------|------|------|
| `/api/embedder/status` | GET | 获取嵌入模型状态 |
| `/api/embedder/download` | POST | 启动模型下载（后台），body: `{"model_id": "BAAI/bge-small-zh", "mirror": "hf-mirror"}` |
| `/api/embedder/apply` | POST | 设为默认模型，body: `{"model_id": "BAAI/bge-small-zh"}` |
| `/api/embedder/test` | POST | 测试镜像源连通性，返回延迟 ms |

### IDE/Agent 工具检测

| 端点 | 方法 | 用途 |
|------|------|------|
| `/api/tools/detect` | GET | 检测系统已安装的 IDE 和 Agent 工具（IDE 优先扫描） |

### 数据目录查询

| 端点 | 方法 | 用途 |
|------|------|------|
| `/v1/trust/data-location` | GET | 获取实际数据目录路径（字段：`data_directory`） |

### 安全配置变更

v0.6.0 起，CORS 策略从 `permissive`（完全宽松）收紧为显式白名单：
- 允许来源：`localhost` / `127.0.0.1` / `0.0.0.0` 任意端口，以及 `tauri://` 协议
- 允许方法：GET、POST、OPTIONS
- 允许头：Content-Type、Authorization
- 不允许凭证（credentials: false）

---

## 常见问题

### Q：我需要每次都说"请用 recall 搜索"吗？

**不需要。** 配置好规则文件后，AI 会自动判断何时该搜索。你只需要正常问。

### Q：AI 会不会滥用记忆，把无关的东西也记下来？

**不会。** 规则中明确写了触发条件（"技术决策"、"用户偏好"），AI 不会记录闲聊内容。你也可以在规则中调整触发条件。

### Q：记忆存在哪里？安全吗？

所有记忆存储在本地（默认项目目录下的 `.loong-recall/data/` 或全局 `~/.loong-recall/data/`），不上传任何服务器。你可以随时用 `forget` 删除、用 `update_memory` 修改。

### Q：我换了电脑，记忆能迁移吗？

可以。使用 `--global` 模式时，复制 `~/.loong-recall/` 目录到新电脑即可。

### Q：Fast Match 和 Smart Match 怎么选？

| 日常推荐 | 特殊场景 |
|---------|---------|
| **Fast Match（默认）** | **Smart Match（`--features ml`）** |
| 搜函数名、变量名 | 模糊描述（"处理重试的代码"） |
| 极低延迟、零依赖 | 首次需下载模型（约 95MB） |
| 多数场景够用 | 复杂项目语义搜索 |

> 详见 [Smart Match 离线安装指南](OFFLINE_MODEL_GUIDE.md)。

### Q：本地语义模型下载失败怎么办？

v0.6.0 起支持三种降级策略：
1. **自动重试**：按 2s / 4s / 8s 间隔指数退避重试
2. **切换镜像**：HfMirror 失败后自动尝试 ModelScope
3. **TF-IDF 降级**：所有重试失败后回退到 TF-IDF 统计算法，保证基础功能可用

### Q：为什么 AI 回复里出现了"（根据记忆 #3）"？

这是我们设计的功能——让记忆**可见**。当 AI 引用了你之前保存的记忆时，会标注来源编号。这样你能看到记忆在起作用，也能信任它的准确性。

### Q：LLM 配置支持哪些模型？

v0.6.0 起全面支持国产模型：DeepSeek、通义千问、智谱 GLM、MiniMax、Moonshot (Kimi)、豆包、阶跃星辰、百川智能、讯飞星火、腾讯混元，以及自定义 API（兼容 OpenAI 协议的任何 API 地址）。

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

# 错误：os error 32（文件被占用）
# 原因：sidecar 进程在运行，build.rs 无法清理日志文件
# 解决：先停止 sidecar 进程，再编译
```

### Q：启动时报"端口被占用"？

```bash
# 错误：Address already in use (os error 10048)
# 原因：3099 端口已被其他程序占用
# 解决：LRC 支持端口自适应，会自动尝试 3099-3198 范围内的端口
# 也可手动指定端口
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

### Q：状态栏显示"已停止 / 不可达"？

v0.6.0 起，点击状态栏的"已停止 / 不可达"文本会弹出启动服务弹窗：
1. 点击"启动服务"按钮
2. 等待 sidecar 启动（约 3-5 秒）
3. 仪表盘会自动刷新并恢复连接

### Q：记忆突然搜不到了？

1. 检查记忆文件是否存在：`.loong-recall/data/memories.json`
2. 确认没有误删记忆文件
3. 如果使用了 `--global` 模式，检查 `~/.loong-recall/data/` 路径
4. 搜索词太模糊？试试用更精确的关键词，或开启 LLM 增强模式
5. 点击仪表盘右下角的"数据目录"路径，直接打开文件夹检查

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

> **装上 MCP，配好规则，然后忘掉它的存在。** 你只管正常写代码、正常聊天，AI 会自己记住该记住的、找到该找到的。

---

## 参考链接

- [Smart Match 离线安装指南](OFFLINE_MODEL_GUIDE.md) — 内网/离线环境配置
- [性能测试指南](BENCHMARK.md) — 百万条记忆 < 30ms

> 算法概述文档受 DaoTi Research License 保护，不公开分发。如需了解算法原理，请参阅源码 `src/engine/` 目录下的相关模块。

---

## 更新历史

### v0.6.0 (2026-07-28)

**新增功能**
- **LLM 配置全面国产化**：支持 DeepSeek、通义千问、智谱 GLM、MiniMax、Moonshot、豆包、阶跃星辰、百川智能、讯飞星火、腾讯混元等国产模型提供商，摒弃国外模型
- **本地语义模型内置**：无需配置即可使用代码语义搜索（BAAI/bge-small-zh 中文 / all-MiniLM-L6-v2 多语言）
- **模型下载国内镜像**：HfMirror / ModelScope 国内镜像，断点续传 + 指数退避重试 + TF-IDF 降级
- **用户文档页面**：将原"API 文档"改造为综合「用户文档」模块（快速上手 + 配置指南 + FAQ + 使用技巧 + API 参考 + 更多资源）
- **状态栏交互优化**：点击"已停止/不可达"文本弹出启动服务弹窗；点击数据目录路径直接打开文件夹
- **AI 工具扫描优化**：IDE 优先扫描策略，支持 36+ 工具检测，跨平台路径支持（Windows/macOS/Linux）
- **非 MCP 工具配置指南**：为 20+ 不支持 MCP 的工具提供手动配置指南
- **端口自适应**：3099-3198 端口范围自动探测，避免端口冲突
- **CORS 安全收紧**：从 permissive 收紧为显式白名单（localhost / 127.0.0.1 / tauri://）

**问题修复**
- 修复 sidecar 静态资源修改后不生效问题（需重新编译 sidecar）
- 修复 `open_data_dir` 命令字段名错误（`data_path` → `data_directory`）
- 修复 CSS `.modal-overlay` 覆盖 `[hidden]` 属性导致模态框无法隐藏问题
- 修复仪表盘跨域 iframe 无法调用 Tauri API 问题（通过 postMessage 桥接）

### v0.5.5 (2026-06-21)

**新增功能**
- MCP 配置自动升级：Sidecar 启动时自动检测并升级旧版本 MCP 配置（stdio `loong-recall` → HTTP `lrc-memory`）
- AI 主动调用修复：修复 Trae 规则文件路径（`.trae/rules.md` → `.trae/rules/lrc-memory.md`），添加 `alwaysApply: true` frontmatter
- AI 工具检测改进：避免残留 dot 目录导致的误报

**问题修复**
- 修复 MCP 工具不显示（"no tools yet"）问题：统一 HTTP 模式配置 + 清理旧配置
- 修复 AI 主动调用 recall 未生效问题：修复规则文件路径 + 添加 frontmatter
- 修复仪表盘"修改配置"按钮无反应问题：统一使用完整 LLM 配置表单

### v0.5.4 (2026-06-20)

**新增功能**
- 全项目静态代码审计，修复所有 Clippy 警告
- 桌面端 URL 导航白名单验证（仅允许 127.0.0.1）
- 敏感数据使用后内存清零（SecureString 模式）
- 字符串编译时混淆（obfstr）
- DPAPI 密钥损坏自动恢复机制

### v0.5.1 (2026-06-18)

**新增功能**
- 前端版本号一致性（统一从 Cargo.toml 读取）
- 前端 CSS 内联 1260 行提取到 app.css
- 前端 app.js 全局变量污染（IIFE 隔离）
- server.rs 巨型函数拆分（964行 → 5个函数）
- 模型加载逻辑重复（提取共享 PoolingStrategy）
- RRF 融合逻辑重复（提取共享 rrf.rs 模块）

### v0.5.0 (2026-06-17)

**新增功能**
- LRC Desktop 桌面端应用（基于 Tauri 2）
- 配置向导：首次启动自动引导完成项目目录、LLM 配置和 Agent 连接
- LLM 可视化配置：桌面端内置 LLM 设置面板，支持 DeepSeek、通义千问等国产模型提供商
- Agent 配置引导：分类展示 IDE 类、桌面应用、命令行工具等 AI 产品的 MCP 配置方法
- 系统托盘：最小化到托盘，右键快捷操作
- 配置加密存储：API Key 使用 AES-256-GCM 加密存储
- 一键安装脚本（`scripts/install.ps1` / `scripts/install.sh`）

**问题修复**
- 修复 TraeDetector 在配置文件不存在时返回 None 导致 MCP 配置从未写入的 Bug
- MCP 配置使用绝对路径，IDE 无需依赖 PATH 环境变量

### v0.3.1 (2026-06-12)

**新增功能**
- 自动打开浏览器：HTTP 模式启动后自动打开默认浏览器访问仪表盘
- LLM 可视化配置：仪表盘「设置」页面支持图形界面配置 LLM API，无需命令行
- 配置即时生效：通过仪表盘修改 LLM 配置后无需重启服务

**问题修复**
- 添加 `[workspace]` 声明，修复克隆到父级 workspace 时的 Cargo 冲突

### v0.3.0 (2026-06-09)

**新增功能**
- 配置持久化：LLM API、端口等设置保存到本地配置文件，重启不丢失
- 后台守护模式（`--daemon`）：无控制台后台运行，供桌面端 Agent 长期调用
- 系统托盘：Windows 原生托盘图标，右键菜单快速打开仪表盘/退出
- 多窗口支持（`--multi-window N`）：同项目最多 N 个窗口同时运行
- 进程守护：单例锁 + 端口自适应 + 优雅关闭

### v0.2.0 (2026-06-07)

**新增功能**
- LLM 增强模式：用你的 LLM 做自然语言查询翻译，不下载模型也能语义搜索
- Web 仪表盘：`http://127.0.0.1:3099/dashboard` 可视化记忆健康度
- 船长日志生成器：生成项目代码库记忆全景报告
- 快速安装脚本：`install.bat`（Windows）/ `install.sh`（Linux/macOS）
- 自动化守门人系统：10 道质量检查，CI 自动运行

**问题修复**
- 修复仪表盘 `app.js` 404 错误，前端功能完全恢复
- 消除 60+ 处 `.unwrap()` 残留，杜绝生产环境 Panic 风险
- 修复全部 Clippy 警告（标准模式 + 严格模式）
- 修复切片越界、类型转换截断等潜在运行时错误

**文档更新**
- 新增仪表盘使用说明和专业名词大白话解释
- 新增快速安装脚本使用说明
- 新增代码质量守门人系统说明
- README.md 新增完整更新日志
