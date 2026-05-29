# Loong Recall (L-RC / 忆)

**源于道体·道枢层 —— 代码语义记忆 MCP 服务**

[![License](https://img.shields.io/badge/Code-Apache%202.0-blue.svg)](LICENSE_CODE)
[![License](https://img.shields.io/badge/Engine-DaoTi%20Research%20License-red.svg)](LICENSE)

---

## 简介

**Loong Recall**（缩写 **L-RC**，中文名 **"忆"**）是 Loong Agent OS 的代码语义记忆子系统。

其核心算法源于 [**道体（DaoTi）基座模型**](https://github.com/zhibaiYingChuan/DaoTi) 的 **道枢层（Core Layer）**——道体是一个预训练的神经网络语义基座模型，采用"冻结道体 + 轻量适配器"范式，在消费级 CPU 上完成训练。

Loong Recall 将道枢层的语义编码与检索能力独立为 MCP (Model Context Protocol) 服务，为 AI 助手提供代码上下文记忆，突破上下文窗口限制。

## 快速开始

```bash
# 1. 克隆仓库
git clone https://github.com/zhibaiYingChuan/LRC.git
cd LRC

# 2. 编译 MCP 服务（约 2 分钟）
cargo build --release --features server

# 3. 启动服务
# HTTP 模式：
./target/release/code-memory-server --src-dir ./src --port 3099

# Stdio 模式（推荐 IDE 全局部署）：
./target/release/code-memory-server --src-dir ./src --stdio
```

## 架构

```
用户查询 → MCP HTTP/Stdio → Chunker（切分）→ Engine::Encoder（编码）
                                                    ↓
                                           Engine::Retriever（检索）
                                                    ↓
                                              Top-K 结果返回
```

| 组件 | 文件 | 说明 |
|------|------|------|
| **切分器** | `src/chunker.rs` | 按 fn/struct/trait/impl 边界切分 Rust 代码 |
| **编码器** | `src/engine/encoder.rs` | 语义向量编码（快速模式 / CodeBERT 模式） |
| **检索器** | `src/engine/retriever.rs` | 向量相似度 Top-K 检索 |
| **编排器** | `src/engine/manager.rs` | 整合三阶段流水线 |
| **MCP 服务** | `src/server.rs` | HTTP + JSON-RPC 2.0 / Stdio 双传输 |

## MCP 工具

### `search_code`

在项目代码库中语义搜索相关代码片段。

```
输入:
  query  (必填) — 自然语言查询，如 "MemoryManager 的 retrieve 方法"
  top_k  (可选) — 返回结果数量（默认 5，最大 20）

输出:
  Top-K 代码片段，含文件路径、行号、相似度评分、代码内容
```

### `codebase_stats`

获取代码库索引统计信息。

```
输出:
  已索引文件数、代码片段数、类型分布（fn/struct/trait/impl/enum）
```

## IDE 配置

### Trae / Cursor / VS Code

在 MCP 配置文件中添加：

```json
{
  "mcpServers": {
    "loong-recall": {
      "command": "/path/to/code-memory-server",
      "args": ["--src-dir", "/path/to/your/project/src", "--stdio"],
      "type": "stdio"
    }
  }
}
```

### 手动 HTTP 测试

```bash
# 健康检查
curl http://127.0.0.1:3099/health

# MCP 初始化
curl -X POST http://127.0.0.1:3099/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'

# 搜索代码
curl -X POST http://127.0.0.1:3099/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_code","arguments":{"query":"memory retrieval","top_k":3}}}'
```

## 分层开源许可

本项目遵循 [道体（DaoTi）分层开源协议](https://gitcode.com/gcw_M73FIiUo/DaoTi?tab=license)：

| 层级 | 范围 | 许可证 | 权限 |
|------|------|--------|------|
| **L1 公开层** | `src/chunker.rs`, `src/server.rs`, `src/bin/` | Apache 2.0 | 可修改、可交易、可分发 |
| **L2 受保护层** | `src/engine/` | DaoTi Research License v1.0 | 源码可见，仅限研究/审计，禁止逆向/训练竞品 |
| **L3 编译层** | `code-memory-server` 二进制 | 双重保护 | Rust 编译天然混淆 + 许可证约束 |

> **⚠️ 商业使用限制**：此开源 MCP 服务（包括所有层级代码及编译产物）**不可用于未经授权的商业用途**。如需商业授权，请联系项目所有者。

**关于 L2 受保护层**：`src/engine/` 目录下的文件包含从道体道枢层衍生的核心编码与检索算法。这些文件在仓库中**源码可见**——允许阅读和学习接口设计，但禁止逆向工程和训练竞争模型。

公开层代码（切分器、MCP 协议服务、CLI 入口）可修改、可交易、可分发，欢迎贡献。

## 原理

Loong Recall 的语义记忆能力源于道体（DaoTi）的规范场论结构——退化基态（Degenerate Ground State）发现。道枢层在消费级 CPU 上完成预训练后，其核心参数被冻结（"道体"），作为稳定的语义编码基础。

Loong Recall 提取了道枢层的编码-检索范式，将其工程化为独立 MCP 服务：

1. **切分**：将源码按语法边界切分为独立代码片段
2. **编码**：每个片段通过语义编码器转换为高维向量
3. **检索**：查询文本同样编码后，在向量空间中通过余弦相似度匹配 Top-K 片段

### 两种编码模式

Loong Recall 内置两种编码器，编译时通过 Cargo feature 切换：

| | 🚀 快速模式（默认） | 🧠 CodeBERT 模式（`--features ml`） |
|------|------|------|
| **编码器** | `FastEncoder`（内联词袋编码器） | `CodeBertEncoder`（candle + CodeBERT） |
| **外部依赖** | **零**，纯 Rust 实现 | 需下载 CodeBERT 模型（~200MB，仅首次） |
| **编译命令** | `cargo build --features server` | `cargo build --features server,ml` |
| **启动时间** | 即时（毫秒级） | 首次需下载模型（视网络 1~5 分钟），后续即时 |
| **检索精度** | 基于 token 关键词匹配 | 真实语义理解（同义词、自然语言查询） |
| **内存占用** | < 10 MB | ~500 MB（模型加载后） |
| **适用场景** | 精确函数名/变量名查找、日常开发 | 自然语言描述查询、模糊意图检索 |

#### 快速模式（默认，推荐日常使用）

```bash
cargo build --features server
./target/release/code-memory-server --src-dir ./src --port 3099
```

无需任何额外配置，编译后立即可用。编码器基于代码 token 分割和词袋匹配——如果你用函数名、变量名或代码片段检索，精度完全够用。这也是忆在 Loong Agent OS 中作为默认模式运行的方式。

#### CodeBERT 模式（高精度，按需启用）

```bash
cargo build --features server,ml
./target/release/code-memory-server --src-dir ./src --port 3099
```

**首次启动时会自动从 HuggingFace Hub 下载 `microsoft/codebert-base` 模型（~200MB）**，存储在本地缓存目录。下载仅执行一次，后续启动直接加载缓存。

CodeBERT 模式的优势在于你可以用**自然语言描述**来检索代码，例如：
- "处理用户登录的逻辑在哪里？" → 匹配到 `fn authenticate_user()`
- "错误重试的代码" → 匹配到 `fn retry_with_backoff()`

> ⚠️ **请勿在未理解上述差异的情况下贸然启用 `ml` feature**。如果只需要按函数名/关键词查代码，快速模式完全够用，且零成本启动。

## 开发

```bash
# 运行测试
cargo test

# 跳过需要下载 CodeBERT 模型（~200MB）的测试
SKIP_ML_TESTS=1 cargo test

# 编译 MCP 服务
cargo build --features server

# 运行泄露检测
python scripts/check_algorithm_leak.py
```

## 贡献

**公开层** (`src/chunker.rs`, `src/server.rs`, `src/bin/`) 欢迎社区贡献。请提交 PR 前运行测试和泄露检测。

**受保护层** (`src/engine/`) 的贡献需签署 CLA（贡献者许可协议），将版权转让给项目所有者。详见 [LICENSE](LICENSE)。

---

*Loong Recall — 忆 · 来自道体，用于龙*