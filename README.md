# Loong Recall (L-RC / 忆)

**几何坐标驱动的 AI 永久记忆系统 —— 具备自动深度演化的语义记忆 MCP 服务**

[![License](https://img.shields.io/badge/Code-Apache%202.0-blue.svg)](LICENSE_CODE)
[![License](https://img.shields.io/badge/Engine-DaoTi%20Research%20License-red.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

---

## 它不是又一个 RAG 工具

大多数记忆系统的工作方式：query → embedding → 全库向量相似度排序 → top-k。这是 RAG 的标准范式，优点是简单，缺点是随着记忆量增长，检索越来越慢，且无法区分"重要的旧知识"和"新鲜的噪音"。

Loong Recall 走了一条完全不同的路：

| | 传统 RAG / 向量检索 | Loong Recall |
|---|---|---|
| **检索方式** | 全库 ANN 相似度排序 | 几何坐标定位 + 区域剪枝 |
| **检索复杂度** | O(N log N) 或更高 | O(roi_ratio × N) |
| **记忆组织** | 扁平向量空间 | 分层几何坐标空间（洛书九宫格） |
| **重要记忆** | 依赖人工标注重要性分数 | 深度越大的记忆天然居中，自动优先召回 |
| **知识抽象** | 不支持 | 递归合成 → 从具体操作自动抽象出通用程序 |
| **长期记忆** | 靠 TTL 过期删除 | 中心记忆半衰期无限，外围自然衰减 |
| **细粒度回溯** | 不支持 | RecursiveUnfold 将程序记忆拆回原始子步骤 |

---

## 它是什么

把 Loong Recall 接入 IDE 之后，你的 AI 助手获得三个关键能力：

1. **语义搜索代码库** — 用自然语言描述你想找的代码，而不是靠盲猜文件名
2. **跨会话永久记忆** — 让 AI 助手记住你的偏好、决策、项目约定，下次对话自动延续
3. **记忆自动演化** — 同类记忆自动融合形成高层知识，越重要的知识越容易被检索到

> 你问 "处理用户登录的逻辑在哪？" → 它找到 `fn authenticate_user()`
> 你问 "之前说好的 API 端口是哪个？" → 它从记忆中调出 "API 端口约定为 8080"
> 你反复讨论数据库选型 → 系统自动合成 "项目数据库选型决策" 的程序记忆

它的语义编码能力来自 [道体（DaoTi）基座模型](https://github.com/zhibaiYingChuan/DaoTi) 的道枢层，被独立封装为零外部依赖的 MCP 服务。

---

## 记忆架构（高层原理）

Loong Recall 的记忆系统基于三个核心设计：

### 1. 几何坐标空间

每条记忆不是存储为高维浮点向量，而是被映射到一个**基于洛书九宫格的低维整数坐标空间**。这个坐标空间具有明确的几何结构，记忆之间的关系由它们在空间中的位置决定——相近位置的记忆语义相近，无需每次计算相似度。

### 2. 深度演化

记忆具有 0~5 层拓扑深度：

```
感觉记忆 (depth 0) —— 原始输入，如 "用户说要用 pnpm"
    ↓ 自动合成
情节记忆 (depth 1) —— 事件片段，如 "用户偏好 pnpm 包管理"
    ↓ 自动合成
语义记忆 (depth 2) —— 概念抽象，如 "项目构建工具选型"
    ↓ 自动合成
程序记忆 (depth 3) —— 通用模式，如 "新项目初始化流程"
    ↓ 自动合成
架构记忆 (depth 4+) —— 系统级知识，半衰期无限，永久存储
```

当同类记忆积累到一定条件后，系统自动触发**递归合成**，将多条低层记忆融合为一条更高层的抽象记忆。这个过程完全自动，无需人工干预。

### 3. 区域检索

查询时，系统先确定查询在坐标空间中的位置，然后只在一个**可配置的感兴趣区域**内遍历候选记忆——而非扫描全库。这意味着：

- 百万级记忆规模下，检索延迟仍保持在毫秒级
- 深度越大的记忆天然位于坐标空间中心附近，抽象知识总是被优先召回
- 外围的噪声记忆自动边缘化，不会干扰检索结果

### 4. 可逆组合

大多数记忆系统只做"聚合"，无法"展开"。Loong Recall 支持将程序记忆拆解回原始子步骤，便于调试和细粒度回溯。

> 以上为设计原理的高层描述，具体坐标映射、合成阈值等实现细节属于受保护的核心算法，未在公开文档中披露。详见 [算法概述](docs/ALGORITHM_OVERVIEW.md)。

---

## 性能概览

| 规模 | 检索延迟 | 说明 |
|---|---|---|
| 万条记忆 | < 5ms | 日常开发规模，完全无感 |
| 十万条记忆 | < 15ms | 大型项目规模 |
| 百万条记忆 | < 30ms | 理论验证规模 |

> 以上数据基于消费级 CPU（Intel i7 / AMD R7 级别），未使用 GPU 加速。性能复现方法见 [性能测试指南](docs/BENCHMARK.md)。

### 与主流方案的功能对比

| 能力 | Loong Recall | Mem0 | Zep | LangChain Memory |
|---|---|---|---|---|
| 跨会话持久化 | ✅ | ✅ | ✅ | ✅ |
| 语义检索 | ✅ | ✅ | ✅ | ✅ |
| 自动知识抽象 | ✅ 递归合成 | ❌ | ❌ | ❌ |
| 记忆深度演化 | ✅ 5 层 | ❌ | ❌ | ❌ |
| 抗遗忘（中心偏好） | ✅ 几何驱动 | ❌ | ❌ | ❌ |
| 可逆组合（Unfold） | ✅ | ❌ | ❌ | ❌ |
| 检索复杂度 | O(roi_ratio×N) | O(N log N) | O(N log N) | O(N log N) |
| 零外部依赖（快速模式） | ✅ | ❌ 需 API | ❌ 需 API | ❌ |
| 本地运行 | ✅ | ❌ 云端 | ❌ 云端 | ✅ |

---

## 快速开始

下面 5 步，从零到能用：

### 第 1 步：克隆并编译（约 2 分钟）

```bash
git clone https://github.com/zhibaiYingChuan/LRC.git
cd LRC
cargo build --release --features server
```

编译产物在 `target/release/code-memory-server.exe`（Windows）或 `target/release/code-memory-server`（Linux/macOS）。

### 第 2 步：启动服务验证

先用 HTTP 模式跑起来，确认服务正常：

```bash
./target/release/code-memory-server --src-dir ./src --port 3099
```

看到 `Loong Recall (L-RC / 忆) MCP 服务启动: http://127.0.0.1:3099` 就说明服务已启动。

### 第 3 步：试试代码搜索

新开一个终端，发送搜索请求：

```bash
curl -X POST http://127.0.0.1:3099/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_code","arguments":{"query":"memory retrieval","top_k":3}}}'
```

你应该能看到匹配到的代码片段，包含文件路径、行号和相似度评分。

### 第 4 步：试试写入和检索记忆

```bash
# 写入一条记忆
curl -X POST http://127.0.0.1:3099/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"remember","arguments":{"content":"项目使用 pnpm 作为包管理器","memory_type":"preference","tags":["tooling"]}}}'

# 检索记忆
curl -X POST http://127.0.0.1:3099/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"recall","arguments":{"query":"包管理器偏好","top_k":3}}}'
```

### 第 5 步：接入 IDE

验证通过后，关掉 HTTP 模式（Ctrl+C），然后配置 IDE 使用 Stdio 模式（推荐）：

```json
{
  "mcpServers": {
    "loong-recall": {
      "command": "C:/path/to/code-memory-server.exe",
      "args": ["--src-dir", "C:/path/to/your/project/src", "--stdio"]
    }
  }
}
```

| IDE | 配置文件位置（Windows） | 配置说明 |
|-----|----------------------|----------|
| Trae | `%APPDATA%/Trae/User/mcp.json` | 直接编辑此 JSON 文件，或通过 Trae 的 MCP 设置界面添加 |
| Cursor | `%APPDATA%/Cursor/mcp.json` | 在 Cursor 设置 → MCP 中添加 |
| VS Code | `.vscode/mcp.json` 或用户级全局配置 | 需安装 MCP 扩展 |

> **路径注意**：Windows 下需使用正斜杠 `/`（如 `G:/code-memory/target/release/code-memory-server.exe`），不要使用反斜杠。
> 如果已有其他 MCP 服务器，将 `loong-recall` 条目合并到现有 `mcpServers` 对象中即可。

重启 IDE 后，AI 助手自动发现全部 9 个工具，无需任何额外配置。

---

## 更多使用场景

除了 IDE 中的代码搜索和对话记忆，Loong Recall 还适用于：

- **AI 客服** — 记住每个用户的偏好和历史问题，跨会话延续服务上下文
- **个人知识管家** — 将日常对话中的关键信息（决策、偏好、事实）自动沉淀为永久记忆
- **项目知识库** — 为团队项目维护一个自动演化的知识图谱，新人接手时直接检索历史决策

详见 [使用场景文档](docs/USE_CASES.md)。

---

## 全部 9 个 MCP 工具

接入 IDE 后，AI 助手能直接调用的全部工具：

### 代码搜索（2 个）

| 工具 | 用途 | 必填参数 | 可选参数 |
|------|------|----------|----------|
| `search_code` | 在项目代码库中语义搜索 | `query` — 自然语言查询 | `top_k` — 返回条数（默认 5，最大 20） |
| `codebase_stats` | 查看代码库索引状态（文件数、片段数、类型分布） | 无 | 无 |

### 记忆管理（7 个）

| 工具 | 用途 | 必填参数 | 可选参数 |
|------|------|----------|----------|
| `remember` | 写入一条永久记忆 | `content` — 记忆内容 | `memory_type`（fact/preference/decision/code_context/conversation）、`project`、`tags`、`importance`（1-10）、`ttl_days` |
| `recall` | 语义检索历史记忆 | `query` — 自然语言查询 | `top_k`、`memory_type`、`project`、`tags`、`min_importance` |
| `forget` | 删除一条记忆 | `memory_id` | — |
| `update_memory` | 更新记忆内容 | `memory_id`、`content` | `importance` |
| `list_memories` | 分页列表查看记忆 | 无 | `memory_type`、`project`、`tags`、`sort_by`（created_at/importance/last_accessed）、`order`（desc/asc）、`limit`、`offset` |
| `memory_stats` | 记忆库统计（总数、类型分布、项目分布） | 无 | 无 |
| `archive` | 归档过期记忆到冷存储 | 无 | 无 |

---

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
                            │   暴露 9 个工具：                          │
                            │   代码搜索 2 个 + 记忆管理 7 个            │
                            │                                          │
                            │   HTTP:  POST /mcp  (调试用)              │
                            │   Stdio: stdin → stdout (IDE 标准模式)    │
                            └──┬──────────────────────┬────────────────┘
                               │                      │
                    代码搜索通道（向量检索）     记忆管理通道（几何检索）
                               │                      │
              ┌────────────────▼──────────┐  ┌───────▼──────────────────┐
              │  ② 引擎编排层              │  │  ③ 记忆存储层             │
              │  engine/manager.rs        │  │  memory_store.rs         │
              │                           │  │                          │
              │  协调三阶段流水线：         │  │  领域聚合根：             │
              │  切分 → 编码 → 检索        │  │  增删改查 + 几何召回      │
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
        ├─→ 编码查询并将其定位到几何坐标空间中的位置
        ├─→ 在坐标空间的感兴趣区域（ROI）内遍历候选记忆
        ├─→ 按几何距离排序，深度大的记忆天然优先
        └─→ 返回 Top-K 记忆，附带衰减权重
        │
        ▼
① server.rs — 格式化结果（记忆 ID、类型、标签、深度），返回响应
```

### 各层组件详解

| 层级 | 源文件 | 一句话职责 | 详细说明 |
|------|--------|-----------|----------|
| **① MCP 传输层** | `src/server.rs` | 协议适配与工具路由 | 实现 MCP 协议全部生命周期：`initialize`（握手）→ `tools/list`（声明工具）→ `tools/call`（执行调用）。HTTP 模式基于 Axum 框架，Stdio 模式基于 stdin/stdout 管道。所有工具共享同一个 `AppState`（内存中的索引管理器 + 记忆存储），无需外部数据库。 |
| **② 引擎编排层** | `src/engine/manager.rs` | 三阶段流水线调度 | 调用了④⑤两层完成索引和检索。`index_project()` 遍历目录、按扩展名过滤文本文件、调用 chunker 切分、调用 encoder 编码、存入 retriever。是代码搜索功能的唯一对外入口。 |
| **③ 记忆存储层** | `src/memory_store.rs` | 记忆领域聚合根 | 封装所有记忆 CRUD 操作和几何检索逻辑。`recall()` 通过几何坐标定位 + ROI 区域剪枝快速召回候选记忆，深度越大的记忆天然优先返回。同时集成指数衰减模型，让不活跃的记忆自然降权。`archive_expired()` 将过期记忆迁入冷存储（`archive.json`），保持活跃记忆库轻量。 |
| **④ 切分器层** | `src/chunker.rs` | 按语法边界拆分代码 | 包含 6 个切分器实现。Rust 用大括号深度匹配，Python 用缩进层级检测，TS/JS 支持箭头函数识别，Go 支持接收者方法，Conversation 按角色前缀切分，其余格式（Markdown、YAML 等）按段落边界切分。所有切分器共享同一个 `CodeChunker` trait。 |
| **⑤-1 编码器** | `src/engine/encoder.rs` | 代码文本 → 向量 | 核心抽象是 `CodeEncoder` trait。默认 `FastEncoder` 基于预定义关键词词袋生成位向量（dim~43），零外部依赖、零模型下载。CodeBERT 模式下 `CodeBertEncoder` 使用 candle 推理框架生成 768 维语义向量。 |
| **⑤-2 编码器注册表** | `src/engine/encoder_registry.rs` | 按语言路由编码策略 | 维护 `语言 → 编码器` 的映射表。支持按语言注册专用编码器（如为 Rust 注册更精准的编码器），未注册语言自动回退到默认编码器。实现 Strategy + Registry 组合模式。 |
| **⑤-3 检索器** | `src/engine/retriever.rs` | 向量相似度匹配 | `LocalRetriever` 维护所有片段向量，查询时计算余弦相似度并排序返回 Top-K。`threshold` 参数过滤低相似度结果。`CodeRetriever` trait 定义统一检索接口，便于替换为远程检索后端。 |
| **⑤-4 HNSW 索引** | `src/engine/hnsw.rs` | 近似最近邻加速 | 基于 Navigable Small World 图算法。每个节点最多连接 M=16 个邻居，搜索时束宽 ef_search=50。相比暴力遍历，百万级片段下检索延迟从 O(n) 降至 O(log n)。当前 HnswRetriever 同时实现 CodeRetriever trait，与 LocalRetriever 可互换。 |
| **⑥ 持久化层** | `src/persistence/` | 记忆与切片的文件存储 | `Persistence` trait 定义 11 个抽象方法（save/load/delete/clear/archive）。`JsonPersistence` 是默认实现，数据存储在 `data_dir/` 下的 3 个 JSON 文件中。trait 设计使后续可平滑切换到 SQLite、Redis 等后端而不影响上层代码。 |
| **⑦ 运行时防护层** | `src/guard.rs` | 防逆向工程保护 | 启动时在 `main()` 第一行自动调用 `risk_aware_guard()`。包含 6 层防护：编译时字符串 XOR 加密（`obfuscated!` 宏）、三级反调试联动、反单步时序检测、函数入口断点扫描、PE 代码段 CRC32 自校验、不透明谓词 + 状态机控制流混淆。检测到威胁时随机延迟后静默退出。 |

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

---

## CLI 选项

```
用法: code-memory-server [选项]

  --src-dir <路径>    要索引的源码目录 [默认: 当前目录]
  --host <地址>       HTTP 绑定地址 [默认: 127.0.0.1]
  --port <端口>       HTTP 绑定端口 [默认: 3099]
  --stdio             使用 stdio 传输模式（推荐 IDE 部署）
  --global            记忆数据存到 ~/.loong-recall/data/（跨项目共享）
  --db-path <路径>    自定义记忆数据库路径（优先级最高，覆盖 --global）
  --help, -h          显示帮助信息
```

三种典型用法：

```bash
# 场景 1：单项目 IDE 标准 MCP（最常用）
code-memory-server --src-dir ./src --stdio

# 场景 2：单项目 HTTP 模式调试
code-memory-server --src-dir ./src --port 3099

# 场景 3：全局记忆，跨项目共享 + 自定义存储路径
code-memory-server --global --db-path /data/my-memories --stdio
```

---

## 切分器支持的语言

按文件扩展名自动选择切分策略，无需手动配置：

| 切分器 | 扩展名 | 识别单元 | 切分方式 |
|--------|--------|----------|----------|
| `RustChunker` | `.rs` | `fn` / `struct` / `trait` / `enum` / `impl` / `mod` | 大括号深度匹配 + `///` 文档注释提取 |
| `PythonChunker` | `.py` | `def` / `async def` / `class` | 缩进层级检测 + `#` 注释提取 |
| `TsJsChunker` | `.ts` `.tsx` `.js` `.jsx` `.mjs` `.cjs` | `function` / `class` / `interface` / `type` / `enum` / 箭头函数 | 大括号匹配 + JSDoc 提取 |
| `GoChunker` | `.go` | `func` / `type`（支持接收者方法） | 大括号匹配 |
| `ConversationChunker` | — | 对话轮次（"用户:" / "助手:" / "系统:" 等） | 按角色前缀切分，支持中英文、全角/半角冒号 |
| `GenericChunker` | `.md` `.txt` `.yaml` `.toml` `.json` `.html` `.css` `.xml` `.sql` `.sh` `.java` `.c` `.cpp` 等 | Markdown 按 `#` 标题切分；其余按段落(`\n\n`)切分 | 标题边界 / 段落边界 |

---

## 两种编码模式

编译时通过 Cargo feature 切换，决定代码搜索的精度和资源消耗：

| | 🚀 快速模式（默认） | 🧠 CodeBERT 模式（`--features ml`） |
|---|---|---|
| **编码器** | `FastEncoder`（内联词袋编码器） | `CodeBertEncoder`（candle + CodeBERT） |
| **外部依赖** | **零**，纯 Rust 实现 | 首次启动自动下载模型（~200MB） |
| **内存占用** | < 10 MB | ~500 MB（模型加载后） |
| **启动时间** | 即时（毫秒级） | 首次 1~5 分钟（下载模型），后续即时 |
| **检索精度** | Token 关键词匹配 | 真实语义理解（同义词、中英文自然语言） |
| **适用场景** | 精确函数名/变量名查找 | 模糊意图描述（"处理重试逻辑的代码"） |

### 🚀 快速模式（默认，推荐日常使用）

```bash
cargo build --features server
```

编译后零配置立即可用。编码器基于代码 token 分割和词袋匹配——如果你习惯用函数名、变量名查代码，精度完全够用。这也是忆在 Loong Agent OS 中作为默认模式运行的方式。

### 🧠 CodeBERT 模式（高精度，按需启用）

```bash
cargo build --features server,ml
```

首次启动时自动从 HuggingFace Hub 下载 `microsoft/codebert-base` 模型（~200MB），存储在本地缓存。下载仅一次，后续启动直接加载缓存。

CodeBERT 模式的优势：你可以用自然语言描述意图，而不是记函数名：

- "处理用户登录的逻辑在哪里？" → `fn authenticate_user()`
- "错误重试的代码" → `fn retry_with_backoff()`

> ⚠️ 如果只需按函数名/关键词查代码，快速模式完全够用，且零成本启动。

---

## 原理

Loong Recall 的语义记忆能力源于道体（DaoTi）的规范场论结构——退化基态（Degenerate Ground State）发现。道枢层在消费级 CPU 上完成预训练后，其核心参数被冻结（"道体"），作为稳定的语义编码基础。

Loong Recall 提取了道枢层的编码-检索范式，工程化为独立 MCP 服务：

1. **切分** — 将源码按语法边界切分为独立片段（函数、结构体、类、段落等）
2. **编码** — 每个片段通过语义编码器转换为高维向量
3. **检索** — 查询文本同样编码后，在向量空间中通过余弦相似度匹配 Top-K 片段

会话记忆系统同样利用编码器对记忆内容进行语义索引，使得 `recall` 工具能跨越精确关键词匹配，理解自然语言查询的意图。

---

## 记忆系统详解

### 数据类型

| 类型 | 用途示例 |
|------|----------|
| `fact` | "此项目使用 PostgreSQL 16" |
| `preference` | "用户偏好 4 空格缩进，不用 Tab" |
| `decision` | "决定用 Axum 而非 Actix，因为生态更活跃" |
| `code_context` | "`auth.rs` 中的 `validate_token` 依赖 JWT_SECRET 环境变量" |
| `conversation` | "上次讨论：需要在下周完成 API 文档" |

每条记忆支持：重要性评分（1-10）、项目关联、标签列表、TTL 过期时间。

### 记忆衰减机制

记忆不会永远保持同等权重。Loong Recall 内置指数衰减模型：

- **新鲜记忆**（刚写入或刚被访问）：衰减因子 ≈ 1.0，保持最高权重
- **冷记忆**（长期未访问）：衰减因子随时间递减，在检索结果中自然排后
- **高重要性记忆**（importance ≥ 8）：衰减速度大幅减缓，长期保持较高权重
- **低重要性记忆**（importance ≤ 3）：衰减更快，检索时优先被过滤

这个机制确保你频繁关注的信息始终排在最前面，而不再相关的信息自然沉淀。

### 存储位置

- 默认：项目目录下的 `.loong-recall/data/`（3 个 JSON 文件：`memories.json`、`chunks.json`、`archive.json`）
- `--global` 模式：`~/.loong-recall/data/`（所有项目共用同一记忆库）
- `--db-path` 自定义：你指定的任意路径

数据以 JSON 文件形式存储，可直接用文本编辑器查看和手动修改。

---

## 贡献指南

以下命令供贡献者在修改代码后运行，确保提交质量：

```bash
# 运行测试（145+ 项）
cargo test

# 跳过需下载 CodeBERT 模型的测试（~200MB）
SKIP_ML_TESTS=1 cargo test

# 代码风格检查
cargo clippy --features server -- -D warnings

# 核心算法泄露检测（预提交钩子也会自动运行）
python scripts/check_algorithm_leak.py
```

### 运行时安全

Loong Recall 在启动时自动执行多层运行时防护（详见 `src/guard.rs`），保护核心算法不被逆向工程：

| 防护层 | 说明 |
|--------|------|
| **字符串混淆** | 编译时 XOR 加密敏感字符串（`obfuscated!` 宏），运行时动态解密 |
| **反调试** | 三级联动检测调试器附加（IsDebuggerPresent + CheckRemoteDebuggerPresent + DebugPort） |
| **反单步** | 执行时序异常检测，识别单步调试行为 |
| **断点扫描** | 检测函数入口 `int3` (0xCC) 软件断点 |
| **完整性校验** | 代码段 CRC32 自校验 + 源码 SHA-256 哈希验证，防止运行时篡改 |
| **控制流混淆** | 基于费马小定理的不透明谓词 + 状态机平坦化，增加反汇编分析难度 |

检测到威胁时，服务延迟随机时间后静默退出，不暴露任何检测逻辑。

---

## 文档导航

| 文档 | 说明 |
|------|------|
| [算法概述](docs/ALGORITHM_OVERVIEW.md) | 记忆架构的高层原理（安全版本，不泄露核心算法） |
| [性能测试指南](docs/BENCHMARK.md) | 如何复现性能测试 |
| [使用场景](docs/USE_CASES.md) | 典型应用场景与最佳实践 |

---

## 分层开源许可

| 层级 | 范围 | 许可证 |
|------|------|--------|
| **L1 公开层** | `src/chunker.rs`、`src/memory_store.rs`、`src/memory_types.rs`、`src/persistence/`、`src/server.rs`、`src/bin/` | Apache 2.0 — 可修改、可分叉、可商用 |
| **L2 受保护层** | `src/engine/` 核心编码与检索算法 | DaoTi Research License v1.0 — 源码可见，仅限研究/审计 |

> ⚠️ L2 层代码禁止用于逆向工程或训练竞争模型。商业使用需联系项目所有者获取授权。详见 [LICENSE](LICENSE)。

---

*Loong Recall — 忆 · 来自道体，用于龙*