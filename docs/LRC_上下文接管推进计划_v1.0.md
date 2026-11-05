# LRC 上下文接管与记忆强化 · 工程推进文档

> 版本：v1.0（2026-11-06）
> 基线：LRC v0.9.16（commit `03186ea`）
> 性质：**工程落地文档**。本文把产品方案翻译为可执行任务，每项任务标注落点文件、接口签名、验收标准。
> 前置结论：日常实测写入 P50 **29ms**、检索 P50 **238ms**、冷启动 **2.4s** —— 性能已具备"上下文中间件"的物理资格。
>
> **端口纪律（强制）**：
> - **本地正式环境：3099**（真实库 `~/.loong-recall/global/data`，用户日常使用）
> - **开发 / 测试环境：3111**（隔离数据目录，禁止连 3099）
> - 本方案所有开发、联调、Hook 实测**一律使用 3111**；只有最终交付给用户的模板才写 3099。
> - 违反后果：测试写入会污染真实记忆库，且 Hook 可能把测试数据迁移进生产库。

---

## 一、现状盘点

### 1.1 已具备（可直接复用）

| 能力 | 落点 | 说明 |
|---|---|---|
| MCP 服务 | [server.rs:4936](file:///g:/code-memory/src/server.rs#L4936) `build_mcp_router` | `/mcp` 端点，JSON-RPC 2024-11-05 |
| MCP 初始化 | [server.rs:436](file:///g:/code-memory/src/server.rs#L436) `handle_initialize` | ✅ 已注入 `instructions`（P0-1 已完成） |
| 工具清单 | [server.rs:413](file:///g:/code-memory/src/server.rs#L413) `handle_tools_list` | remember / recall / forget 等已注册 |
| 自然语言意图解析 | [memory_write.rs](file:///g:/code-memory/src/memory_write.rs) `parse_forget_intent` | 规则式中文/英文指令识别，**提炼引擎的现成范式** |
| 记忆合成 | `synthesis_engine`（子模块） | 已有簇发现 + 摘要生成，可复用为"提炼"的模型侧 |
| 结构化记忆模型 | `Memory`（memory_types.rs） | 含 `memory_type` / `tags` / `event_id` / `entities` / `importance` |
| 生命周期 | `lifecycle.rs` + `/v1/memories/lifecycle` | 支持"被取代/被抑制"，对应三层衰减的归档语义 |
| 观测 | [obs.rs](file:///g:/code-memory/src/obs.rs) | 锁统计 + 时间戳，可用于 Hook 链路埋点 |

### 1.2 缺口（本方案要补的）

| 缺口 | 影响 | 对应任务 |
|---|---|---|
| MCP 初始化无 `instructions` | 模型不知道"必须先用 LRC" | P0-1 |
| 无宿主 Hook 模板 | 用户无法一键接入 80% 迁移 | P0-2 |
| 无对话提炼引擎 | 只能整段写入，噪声大 | P1-1 |
| 无全局规则模板 | 各工具规则各写各的，约束力弱 | P1-2 |
| 无深度插件 | DSH/Cursor 无法一键接管 | P2-1 |

---

## 二、总体架构

```
┌──────────────────────────────────────────────────────────────┐
│ 宿主（Trae / Claude Code / DSH / Cursor）                     │
│                                                              │
│  ① 会话启动 ──► 注入 LRC 索引（Hook: SessionStart）           │
│  ② 每轮对话 ──► 提炼暂存（规则切分 + 弱模型判定）              │
│  ③ 上下文 80% ─► 批量迁移（Hook: PreCompact）                 │
│  ④ 用户提问 ──► 强制 recall（Server Instructions + 规则）      │
└───────────────────────────┬──────────────────────────────────┘
                            │ MCP / HTTP
┌───────────────────────────▼──────────────────────────────────┐
│ LRC v0.9.16+                                                 │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────┐  │
│  │ 提炼引擎     │  │ 迁移接口      │  │ 索引注入接口        │  │
│  │ distill.rs  │  │ /v1/context/ │  │ /v1/context/index  │  │
│  │ (P1-1)      │  │ migrate      │  │                    │  │
│  └─────────────┘  └──────────────┘  └────────────────────┘  │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ 既有：MemoryStore / 检索 / 合成 / 生命周期 / 观测      │   │
│  └──────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

**三层衰减**（与既有 `lifecycle` 对齐，不新增存储层）：

| 层 | 载体 | 生命周期 | 检索优先级 |
|---|---|---|---|
| 活跃区 | 宿主当前上下文 | 当前会话 | 最高（宿主原生） |
| 归档区 | LRC `Memory`（`memory_type=decision/fact`，`tags` 含 `archived`） | 30 天 | 中 |
| 长期区 | LRC 全库 | 永久（受 `lifecycle` 衰减） | 低（按需） |

> **设计约束**：不引入新的存储引擎。归档区就是带 `archived` 标签的普通记忆，复用现有检索与生命周期。

---

## 三、P0 任务

### P0-1 · MCP Server Instructions 注入

**状态**：✅ 已完成（2026-11-06）

**落点**：[server.rs:436](file:///g:/code-memory/src/server.rs#L436) `handle_initialize`

**改动**：

1. `InitializeResult` 结构体新增字段：

```rust
#[serde(skip_serializing_if = "Option::is_none")]
instructions: Option<String>,
```

2. `handle_initialize` 填充：

```rust
instructions: Some(context_instructions()),
```

3. 新增函数（同文件，`handle_initialize` 上方）：

```rust
/// MCP 初始化时下发给宿主的强制规则。
///
/// 设计依据：MCP 协议 2024-11-05 的 `InitializeResult.instructions` 是
/// **唯一**由服务端主动注入、且宿主必须转述给模型的通道。相比在用户规则
/// 文件里写"建议使用"，这里的约束力更强（宿主会把它拼进 system prompt）。
fn context_instructions() -> String {
    r#"【LRC 强制规则 · 不可跳过】

1. 任何涉及「项目历史 / 用户偏好 / 之前做过的决定 / 上次怎么改的」的查询，
   禁止直接使用内置全文搜索（Read / Grep / 文件遍历），必须先调用
   `recall` 工具。若跳过该工具，本次任务视为失败。

2. 完成任何代码修改后，必须调用 `remember` 记录：新增模块、API 端点、
   架构变更、关键决策。这是任务的自然收尾，不需要用户提醒。

3. 用户表达技术偏好（"用 pnpm"）、做出选型决策（"数据库选 PostgreSQL"）、
   或产生跨会话约定时，立即 `remember`，不要等到会话结束。

4. 上下文接近窗口上限时，先调用 `recall` 取回关键决策，再继续；
   不要依赖宿主的自动摘要（它会丢失约束与指代锚点）。

5. 检索结果为空时，才允许回退到内置搜索；回退时必须在回复中说明
   "LRC 未命中，已回退内置搜索"。"#
        .to_string()
}
```

**验收**：

- `curl -X POST localhost:3099/mcp -d '{"jsonrpc":"2.0","id":1,"method":"initialize",...}'` 返回体含 `result.instructions`，且长度 > 200 字符。
- 单测：`test_initialize_includes_instructions`（断言字段存在且含"禁止直接使用内置全文搜索"）。
- 在 Trae 中重连 MCP，确认模型首轮即引用该规则（人工验收）。

**实测结果（2026-11-06，开发端口 3111 + 隔离数据目录 `G:\tmp\lrc-dev-3111\data`）**：

| 检查项 | 结果 |
|---|---|
| `initialize` 响应含 `result.instructions` | ✅ 存在 |
| `instructions` 长度 | ✅ 427 字符（> 200） |
| 含"禁止直接使用内置全文搜索" | ✅ |
| 含 `recall` / `remember` 工具名 | ✅ |
| 首行 | `【LRC 强制规则 · 不可跳过】` |
| 单测 `test_initialize_includes_instructions` | ✅ 通过 |
| 全量库单测 | ✅ 972 passed / 0 failed |
| `cargo clippy --all-targets --features server,ml -- -D warnings` | ✅ 零告警 |

> 注：PowerShell 控制台直接打印中文会显示为乱码（控制台编码问题，非服务端缺陷）；以 UTF-8 解码 `RawContentStream` 后内容正确。

**风险**：部分宿主忽略 `instructions` 字段。缓解：P1-2 的规则模板作为兜底。

---

### P0-2 · 各宿主 Hook 配置模板

**落点**：新建 `integrations/hooks/` 目录

```
integrations/
├── hooks/
│   ├── claude-code/
│   │   ├── settings.json          # PreCompact + SessionStart 配置
│   │   ├── pre-compact.sh         # 80% 触发时调用 /v1/context/migrate
│   │   └── session-start.sh       # 会话启动注入索引
│   ├── trae/
│   │   ├── hooks.json             # UserPromptSubmit 稀释
│   │   └── user-prompt-submit.ps1
│   ├── dsh/
│   │   └── README.md              # 插件安装指引（P2-1 前的手动方案）
│   └── cursor/
│       ├── hooks.json             # preToolUse 重定向
│       └── pre-tool-use.sh
└── rules/
    ├── AGENTS.md                  # 通用强约束规则
    ├── .cursorrules               # Cursor 专用
    └── trae-rules.md              # Trae 专用
```

**Claude Code `pre-compact.sh` 核心逻辑**：

```bash
#!/usr/bin/env bash
# PreCompact Hook：上下文即将压缩时，把提炼结果迁移到 LRC
set -euo pipefail
LRC="${LRC_BASE:-http://127.0.0.1:3099}"
TRANSCRIPT="${CLAUDE_TRANSCRIPT_PATH:-}"

# 1) 调用提炼接口（P1-1 提供），返回结构化记忆数组
DISTILLED=$(curl -sS -X POST "$LRC/v1/context/distill" \
  -H 'Content-Type: application/json' \
  -d "{\"transcript_path\":\"$TRANSCRIPT\",\"max_items\":40}")

# 2) 批量写入（复用既有 remember 批量路径）
echo "$DISTILLED" | curl -sS -X POST "$LRC/v1/context/migrate" \
  -H 'Content-Type: application/json' -d @-

# 3) 输出极简索引，替换原生摘要
echo "[归档] 前 N 轮已迁移至 LRC，索引：$(echo "$DISTILLED" | jq -r '.index_line')"
```

**验收**：

- 每个模板目录含 `README.md`，说明"复制到哪、改哪个变量、如何验证"。
- 在 Claude Code 中实测：手动触发 `/compact`，LRC 记忆数增加，且压缩后的上下文含 `[归档]` 索引行。
- 模板中的端口默认 **3099**（本地），注释说明开发版改 **3111**。

---

## 四、P1 任务

### P1-1 · 提炼规则引擎

**落点**：新建 `src/context_distill.rs`（Layer 1，无外部依赖）

**接口**：

```rust
/// 一条提炼结果（尚未写入 LRC）
pub struct DistilledItem {
    pub content: String,        // 结构化文本，如 "[决策] 用 pnpm（理由：解决兼容）"
    pub memory_type: String,    // decision | preference | fact | code_context
    pub tags: Vec<String>,
    pub importance: u8,
    pub source_turns: (usize, usize), // 原文轮次区间，便于回溯
}

/// 从对话文本中提炼结构化记忆。
///
/// 两阶段：
///   ① 强规则切分：按"收束词/转折词"截取候选片段（纯规则，零成本）；
///   ② 弱模型判定：对候选片段判断是否含决策/约束/结论/指代锚点。
///      未启用模型时退化为"规则打分 + 阈值"，保证离线可用。
pub fn distill_transcript(
    text: &str,
    config: &DistillConfig,
) -> Vec<DistilledItem>;
```

**规则词表**（`const` 数组，可配置）：

| 类别 | 词例 |
|---|---|
| 决策收束 | 决定用、就用、改成、换成、定为、选型为、最终采用 |
| 否定约束 | 不能用、不要用、禁止、避免、别用、放弃 |
| 结论 | 结论是、所以、因此、最终、确认 |
| 指代锚点 | 刚才那个、上次说的、前面提到的、之前定的 |

**打分规则**（无模型时的兜底）：

```
score = 2.0 * 命中决策词 + 1.5 * 命中否定词 + 1.0 * 命中结论词
      + 1.0 * 含专有名词（大写开头/反引号包裹/文件路径）
      - 1.0 * 纯疑问句
阈值：score >= 2.0 才产出
```

**验收**：

- 单测覆盖：中文决策句、英文决策句、闲聊（应丢弃）、纯疑问（应丢弃）、含文件路径的结论。
- 对一段 40 轮真实对话，产出 5~15 条，人工抽检准确率 ≥ 80%。
- 无模型时（默认）可运行，不引入新依赖。

**复用**：`parse_forget_intent` 的规则解析范式（[memory_write.rs](file:///g:/code-memory/src/memory_write.rs)）直接照搬结构。

---

### P1-2 · 全局规则模板（硬约束词）

**落点**：`integrations/rules/`（见 P0-2 目录树）

**`AGENTS.md` 核心段落**（禁用原则性描述）：

```markdown
## 记忆检索（强制）

**禁止**在以下场景直接使用内置全文搜索（Read / Grep / 文件遍历）：
- 询问"之前怎么做的""上次的决定""项目历史"
- 询问用户偏好、约定、架构选型

**必须**先执行 `mcp__lrc__recall`。若跳过，本次任务视为失败。

**必须**在完成代码修改后执行 `mcp__lrc__remember`，记录：
新增模块 / API 端点 / 架构变更 / 关键决策。

检索为空时才允许回退内置搜索，且必须在回复中声明"LRC 未命中，已回退"。
```

**验收**：

- 三个模板文件内容一致（仅工具名不同），且都含"禁止""必须""视为失败"三类强约束词。
- 在 Trae 中加载 `trae-rules.md`，观察模型首轮是否先调 recall（人工验收 3 次，≥2 次命中）。

---

## 五、P2 任务

### P2-1 · DSH 插件 / Cursor 扩展

**落点**：新建 `integrations/dsh-plugin/`（TypeScript）

**最小能力**：

1. 注册命令 `lrc.clear-context`：调用宿主 API 清空对话，同时把提炼结果写入 LRC。
2. 注册命令 `lrc.inject-index`：把 LRC 的归档索引注入当前会话。
3. 关闭宿主原生自动压缩（若插件 API 支持）。

**验收**：

- 插件安装后，`lrc.clear-context` 执行成功，LRC 记忆数增加，宿主上下文清空。
- 插件 README 含"不支持的宿主版本"说明。

**依赖**：DSH 插件 API 文档（需确认是否开放 `clearConversation`）。

---

## 六、新增 HTTP 接口（P0-2 / P1-1 依赖）

| 方法 | 路径 | 用途 | 请求 | 响应 |
|---|---|---|---|---|
| POST | `/v1/context/distill` | 提炼对话 | `{transcript_path?, text?, max_items?}` | `{items: DistilledItem[], index_line: string}` |
| POST | `/v1/context/migrate` | 批量迁移 | `{items: DistilledItem[], project?}` | `{written: usize, merged: usize}` |
| GET | `/v1/context/index` | 取归档索引 | `?project=&limit=20` | `{index_line: string, count: usize}` |

**实现约束**：

- 三个接口都走既有 `MemoryStore` 写路径（`remember_batch`），不新增存储。
- `migrate` 必须幂等：同 `content` 重复迁移走合并（复用 `apply_merge`）。
- 全部接口需 Bearer Token 认证（与 `/v1` 一致）。

---

## 七、验收清单（总）

| # | 任务 | 验收方式 | 通过标准 |
|---|---|---|---|
| 1 | P0-1 instructions | 单测 + curl | ✅ 已通过（字段存在、427 字符、含强约束词） |
| 2 | P0-2 Hook 模板 | 目录完整性 + Claude Code 实测 | 4 个宿主目录齐全，PreCompact 实测生效 |
| 3 | P1-1 提炼引擎 | 单测 + 真实对话抽检 | 5 项单测通过，准确率 ≥80% |
| 4 | P1-2 规则模板 | 文件检查 + Trae 实测 | 3 个模板齐全，3 次中 ≥2 次先调 recall |
| 5 | P2-1 DSH 插件 | 安装实测 | 清空 + 注入两条命令可用 |
| 6 | 新增接口 | 集成测试 | 3 个接口 200，migrate 幂等 |

---

## 八、风险与依赖

| 风险 | 影响 | 缓解 |
|---|---|---|
| 宿主忽略 `instructions` | P0-1 失效 | P1-2 规则模板兜底；P2-1 插件硬拦截 |
| Hook API 版本差异 | 模板失效 | 每个模板标注适用版本，README 给降级方案 |
| 提炼误判（把闲聊当决策） | 记忆污染 | 阈值可调 + `importance` 低分 + 人工可删 |
| 迁移重复写入 | 库膨胀 | `migrate` 幂等（`apply_merge`） |
| 弱模型不可用 | 提炼质量下降 | 规则打分兜底，离线可用 |
| 端口混淆（3099/3111） | Hook 连错服务 | 模板默认 3099，注释显式说明 |

---

## 九、执行顺序建议

```
P0-1（半天）──► P1-2（半天）──► P0-2（1~2 天）──► P1-1（2~3 天）──► P2-1（待 API 确认）
     │                │                  │
     └─ 先让模型"知道要用" ─┘            └─ 再让用户"一键接入" ─┘
```

**理由**：P0-1 与 P1-2 是纯文本改动，成本最低、见效最快；P0-2 依赖 P1-1 的 `/v1/context/distill`，但可先提供"只迁移不提炼"的简化版；P1-1 是质量核心；P2-1 依赖外部 API。

---

## 十、与既有版本的关系

- 本方案**不改变** v0.9.16 的检索与存储行为，只新增"对外接口 + 注入通道"。
- 新增代码全部在 Layer 1（`src/context_distill.rs`）与集成目录（`integrations/`），不触碰 `MemoryStore` 核心。
- 版本号建议：P0 完成发 **v0.9.17**（minor 特性），P1 完成发 **v0.10.0**（上下文中间件正式形态）。
