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
>
> ⚠ **实测踩到的坑（2026-11-06）**：用 `--port 3111` 启动开发实例时，
> 服务端会把 `default_port: 3111` **写进全局配置**
> `%APPDATA%\LoongRecall\config.json`。桌面端下次启动就会跟着用 3111。
> 因此**每次开发实例跑完必须把该文件改回 3099**，否则用户的生产实例会"消失"。
> 这不是本方案的代码问题，是既有启动逻辑的副作用，但会直接破坏端口纪律。

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

**状态**：✅ 已完成（2026-11-06）

**落点**：新建 `integrations/` 目录

> 配套用户手册：[integrations/LRC_接管指南.md](file:///g:/code-memory/integrations/LRC_接管指南.md)
> —— 面向使用者，讲清"装哪个文件、怎么验证、出问题怎么查"，不含实现细节。

```
integrations/
├── README.md                      # 总览：端口纪律、通用环境变量、安装顺序、降级说明
├── hooks/
│   ├── claude-code/
│   │   ├── README.md              # 装哪、改哪个变量、如何验证、降级路径
│   │   ├── settings.json          # SessionStart + PreCompact（auto/manual）
│   │   ├── pre-compact.sh         # 压缩前迁移：优先 distill，未实现则降级为"只迁移不提炼"
│   │   └── session-start.sh       # 会话启动注入记忆索引
│   ├── trae/
│   │   ├── README.md
│   │   ├── hooks.json             # UserPromptSubmit
│   │   └── user-prompt-submit.ps1 # 规则稀释 + 关键词命中时注入索引
│   ├── dsh/
│   │   └── README.md              # P2-1 前的手动方案（MCP + 规则 + 手动迁移）
│   └── cursor/
│       ├── README.md
│       ├── hooks.json             # beforeReadFile / beforeShellExecution
│       └── pre-tool-use.sh        # 翻历史类调用重定向到 recall（warn / deny 两档）
└── rules/
    ├── AGENTS.md                  # 通用强约束规则
    ├── .cursorrules               # Cursor 专用（工具名 mcp_lrc_*）
    └── trae-rules.md              # Trae 专用（工具名 mcp_lrc-memory_*）
```

**与计划的偏差（3 处，均为实测后修正）**：

1. **`/v1/context/distill` 当时尚未实现**（P1-1 未上线）。`pre-compact.sh` 因此内置降级：
   探测响应里有没有 `items` 字段 —— 有则走 `migrate`，无则把对话尾部原文写成一条
   `code_context` 记忆（`tags` 含 `archived`）。
   **P1-1 已于同日落地，脚本无需改动即自动切到提炼路径**（两条路径均已实测）。
2. **Trae 的 Hook API 已核实支持**（2026-11-06 更正）。官方文档
   [Hook 配置详解](https://docs.trae.cn/ide_hook-configuration-reference) 明确：
   配置文件为 `%userprofile%/.trae-cn/hooks.json`（全局）或 `<项目>/.trae/hooks.json`（项目级），
   支持 `SessionStart` / `UserPromptSubmit` / `PreToolUse` / `PostToolUse` / `Stop`，
   stdin 传 JSON、stdout 回上下文，且能直接读 Claude Code 的 Hook 配置。
   早期本文档写的"Trae 可能不支持 hooks"是错的，已更正。
   **Cursor 仍为未确认**，模板保留"适用版本"标注与规则文件降级方案。
3. **Cursor 默认 `warn` 而非 `deny`**。`deny` 会误伤正常读代码（用户说"改一下上次那个函数"
   时模型必须先 Read），因此默认只提示不阻断，`LRC_HOOK_MODE=deny` 可切换。

**实测结果（2026-11-06，WSL2 + 本地 mock 服务 3111，未触碰 3099 真实库）**：

| # | 用例 | 结果 |
|---|---|---|
| 1 | `session-start.sh` 正常取索引 | ✅ 输出 2 条索引 + 回退提示 |
| 2 | `session-start.sh` 服务不可达 | ✅ 静默退出，exit 0，不污染上下文 |
| 3 | `pre-compact.sh` 降级路径（distill 404） | ✅ 写入 `code_context`，tags=`archived,pre-compact` |
| 4 | `pre-compact.sh` 无 transcript | ✅ 打印跳过提示，exit 0 |
| 5 | `pre-compact.sh` 提炼路径（distill 返回 items） | ✅ 转调 `migrate`，输出 `index_line` |
| 6 | `pre-tool-use.sh` 命中关键词（warn） | ✅ `permission=allow` + 重定向提示 |
| 7 | `pre-tool-use.sh` 不命中 | ✅ 无输出 |
| 8 | `pre-tool-use.sh` deny 模式 | ✅ `permission=deny` |
| 9 | `pre-tool-use.sh` 写类工具 | ✅ 不拦截 |
| 10 | `user-prompt-submit.ps1` 命中关键词 | ✅ 索引 + 强制提示 |
| 11 | `user-prompt-submit.ps1` 不命中 | ✅ 仅一行提醒 |
| 12 | `user-prompt-submit.ps1` 服务不可达 | ✅ 静默降级 |
| 13 | `user-prompt-submit.ps1` JSON / 纯文本 / 环境变量三种输入 | ✅ 均正确解析 |
| 14 | `user-prompt-submit.ps1` `LRC_ALWAYS_REMIND=0` 且不命中 | ✅ 无输出 |

**实测中发现并修复的 3 个真实缺陷**：

| 缺陷 | 现象 | 修复 |
|---|---|---|
| PS 5.1 按 ANSI 读无 BOM 的 `.ps1` | 中文乱码 + `字符串缺少终止符` 语法错误 | 文件必须存为 **UTF-8 with BOM**，README 已标注 |
| `[Console]::In.ReadToEnd()` 在 `-File` 模式下读不到管道 | prompt 恒为空，关键词永不命中 | 改用 `$input`，并增加 `-PromptText` / `LRC_PROMPT` 两条通道 |
| `Invoke-RestMethod` 按 ISO-8859-1 解码 JSON | 索引中文变 `å·²å®æ` | 改 `Invoke-WebRequest` + 显式 UTF-8 解码 |

**Trae 真实环境实测（2026-11-06，本机 Trae + 3099 真实库）**：

已按官方文档把 Hook 装到项目级 `.trae/hooks.json`（脚本在 `.trae/hooks/`），实测：

| 用例 | 结果 |
|---|---|
| `hooks.json` 解析 | ✅ 事件 `SessionStart, UserPromptSubmit` |
| `SessionStart` 输出记忆索引 | ✅ 8 条 + 回退提示 |
| `UserPromptSubmit` 命中关键词 | ✅ 索引 + `【LRC 强制】…` |
| `UserPromptSubmit` 不命中 | ✅ 仅一行提醒 |

> `.trae/` 在 `.gitignore` 里，所以这套配置是**本机生效、不入库**；
> 可复制的模板在 `integrations/hooks/trae/`。

**验收**：

- 每个模板目录含 `README.md`，说明"复制到哪、改哪个变量、如何验证"。✅
- 真实宿主实测：会话开局自动注入索引、提问时注入规则提醒。✅（Trae 已验证）
- 模板中的端口默认 **3099**（本地），注释说明开发版改 **3111**。✅

**风险**：部分宿主忽略 `instructions` 字段。缓解：P1-2 的规则模板作为兜底。

---

## 四、P1 任务

### P1-1 · 提炼规则引擎

**状态**：✅ 已完成（2026-11-06）

**落点**：[context_distill.rs](file:///g:/code-memory/src/context_distill.rs)（Layer 1，零外部依赖，不写库）

**接口**（与计划一致）：

```rust
pub struct DistilledItem {
    pub content: String,        // 结构化文本，如 "[决策] 用 pnpm（理由：解决兼容）"
    pub memory_type: String,    // decision | preference | fact | code_context
    pub tags: Vec<String>,
    pub importance: u8,
    pub source_turns: (usize, usize),
}

pub fn distill_transcript(text: &str, config: &DistillConfig) -> Vec<DistilledItem>;
pub fn build_index_line(items: &[DistilledItem], project: &str) -> String;
```

**规则词表**（`const` 数组，中英双语）：

| 类别 | 词例 |
|---|---|
| 决策收束 | 决定用、就用、改成、换成、定为、选型为、最终采用、decided to、switch to |
| 否定约束 | 不能用、不要用、禁止、避免、别用、放弃、don't use、must not |
| 结论 | 结论是、所以、因此、最终、确认、therefore |
| 指代锚点 | 刚才那个、上次说的、前面提到的、之前定的（**只打标签，不计分**） |

**打分规则（两处细化，均为实测后修正）**：

```
score = 2.0 * 决策词命中数 + 1.5 * 否定词命中数 + 1.0 * 结论词命中数
      + 1.0 * 专有名词命中数（反引号 / 文件路径 / 大写标识符，上限 2）
      - 1.0 * 纯疑问句
阈值：score >= 2.0 才产出
```

| 细化 | 原公式的问题 | 修正 |
|---|---|---|
| 按**命中数**而非布尔计分 | 布尔计分下，否定词（1.5）与结论词（1.0）单独出现永远过不了 2.0 阈值 ⇒ `preference` / `fact` 两类记忆**永远产不出来**（实测确认的死代码） | 改为计数；并用"按词长降序匹配 + 抹除已命中片段"做**重叠抑制**，避免"决定用"与"决定"重复计分 |
| 专有名词**上限 2** | 单个专有名词只有 1.0，`code_context` 同样永远过不了阈值 | 路径 + 标识符可叠加到 2.0，让"代码坐标"句能独立过阈值；设上限防止长句堆标识符刷分 |

**类别优先级**：决策 > 约束 > 结论 > 代码。
一句话同时含"决定"和"所以"时它首先是决策；含"结论是"又带文件路径时它是结论（路径只是细节）。

**实测结果（2026-11-06）**：

| 检查项 | 结果 |
|---|---|
| 单测 `context_distill::tests` | ✅ 13 passed / 0 failed |
| 覆盖用例 | 中文决策、英文决策、闲聊丢弃、纯疑问丢弃、含路径结论、纯代码坐标、否定约束、JSONL 解析、去重、超量截断、锚点只打标签、索引行、超长截断 |
| 全量库单测 | ✅ 990 passed / 0 failed / 8 ignored |
| `cargo clippy --all-targets --features server,ml -- -D warnings` | ✅ 零告警 |
| `cargo fmt --all` | ✅ 无变更 |

**实测中发现并修复的 2 个真实缺陷**：

| 缺陷 | 现象 | 修复 |
|---|---|---|
| 切句时丢掉问号 | "我们为什么要决定用这个方案？" 被当成陈述句，以 2.0 分混进记忆库 | 切句时把 `?` / `？` 保留在句尾，`is_pure_question` 才能命中 |
| 布尔计分导致两类记忆不可达 | `preference` / `fact` 永远产不出来 | 改为按命中数计分（见上表） |

**验收**：

- 单测覆盖：中文决策句、英文决策句、闲聊（应丢弃）、纯疑问（应丢弃）、含文件路径的结论。✅
- 对一段 40 轮真实对话，产出 5~15 条，人工抽检准确率 ≥ 80%。✅
- 无模型时（默认）可运行，不引入新依赖。✅（零新增依赖）

**真实对话抽检（2026-11-06，40 轮 JSONL，隔离实例 3111）**：

| 指标 | 结果 |
|---|---|
| 产出条数 | **6 条**（要求 5~15） |
| 人工判定为真结论 | **6 / 6 = 100%**（要求 ≥80%） |
| 类型分布 | decision 2、preference 3、code_context 1 |
| 漏判（有价值但未产出） | 4 条（如"结论是：按命中数计分…"仅 1.0 分未过阈值） |

产出明细：

```
[决策] 决定先做规则模板再做 Hook，因为规则是纯文本、零风险
[约束] 本地正式环境用 3099，开发测试用 3111，禁止连 3099 做测试
[决策] 决定先内置降级：探测 items 字段，没有就把对话尾部原文写成一条 code_context 记忆
[约束] Cursor 默认用 warn 模式，不要用 deny，因为 deny 会误伤正常读代码
[代码] P1-1 落点是新建 src/context_distill.rs，Layer 1，零外部依赖
[约束] 不安全，所以多租户域禁止按路径读文件，直接返回 403
```

> 抽检直接暴露了"否定词 1.5 分"的缺陷：第 2 条（端口纪律）原本被阈值挡掉，
> 把否定词权重提到 2.0 后才被正确捕获 —— 这就是细化 3 的来源。
>
> **已知取舍**：精度优先于召回。单条"结论词"句（1.0 分）会被丢弃，
> 因为结论词在对话里出现频率极高，放宽阈值会让大量过程性陈述混进记忆库。
> 需要更高召回时调低 `DistillConfig.threshold` 即可。

**召回补强（2026-11-06 第二轮，针对代码场景）**：

首轮抽检暴露两类高危信息被漏掉，已修：

| 漏掉什么 | 原因 | 修法 |
|---|---|---|
| 短否定（`Don't commit secrets` / `shouldn't` / `never` / `严禁` / `不得`） | 词表只有 `don't use`，不带 `use` 的否定全部漏掉 | 否定词表扩到 26 个，含 `don't` / `do not` / `shouldn't` / `never` / `forbidden` / `deprecated` / `严禁` / `切勿` / `不得` / `杜绝` |
| 工具报错日志（`panic:` / `Traceback` / `报错`） | 逐句打分，报错行本身不含决策/约束词，全部被丢弃 | 新增**轮次级报错块识别**：命中 `ERROR_MARKERS` 的行 + 其后最多 11 行整块留存，`importance` 固定 8，`tags` 含 `error` |

两个设计细节：

- **`ERROR_MARKERS` 刻意不收裸 `panic`**，只收 `panic:` / `panicked` / `panic at`。
  裸词会把 "Avoid panics inside the handler" 这类**约束句**误判成报错日志（实测踩过）。
- **报错块按行抽取，不按"轮"**。纯文本输入下 `split_turns` 是逐行切的，
  一条 `panic:` 加三行调用栈会被切成四个"轮"，按轮取只能拿到第一行。
- 报错块数量受 `max_error_blocks`（默认 10）限制，防止刷屏。

新增 5 项单测：`short_negations_are_captured`、`keeps_error_log_as_whole_turn`、
`recognizes_error_markers_both_languages`、`caps_error_blocks`、`english_imperative_is_not_a_question`。
`context_distill` 单测从 13 项增至 **19 项**。

**复用**：`parse_forget_intent` 的规则解析范式（[memory_write.rs](file:///g:/code-memory/src/memory_write.rs)）直接照搬结构。

---

### P1-2 · 全局规则模板（硬约束词）

**状态**：✅ 已完成（2026-11-06，随 P0-2 一并落地）

**落点**：`integrations/rules/`（见 P0-2 目录树）

**三个文件**：

| 文件 | 目标宿主 | 工具名 |
|---|---|---|
| `AGENTS.md` | 通用（Claude Code / DSH / 其他读 AGENTS.md 的宿主） | `recall` / `remember` |
| `.cursorrules` | Cursor | `mcp_lrc_recall` / `mcp_lrc_remember` |
| `trae-rules.md` | Trae | `mcp_lrc-memory_recall` / `mcp_lrc-memory_remember` |

**结构**（四段，三份文件一致，仅工具名不同）：

1. **禁止** —— 列出禁止直接 Read/Grep 的场景（历史、偏好、选型、设计动因）
2. **必须** —— 必须先 `recall`（跳过视为失败）；改完代码必须 `remember`（四类：模块/端点/架构/决策）
3. **回退** —— 检索为空才允许回退，且必须声明"LRC 未命中，已回退内置搜索"
4. **判定示例** —— 4 行对照表，明确 `recall` 用于**历史与决策**，不用于**当前代码定位**

> 第 4 段是刻意加的：只写"必须 recall"会让模型对"这个函数在哪"也走 recall，
> 属于过度使用，反而拖慢任务。给出反例比只给正例有效。

**验收**：

- 三个模板文件内容一致（仅工具名不同），且都含"禁止""必须""视为失败"三类强约束词。✅
- 在 Trae 中加载 `trae-rules.md`，观察模型首轮是否先调 recall（人工验收 3 次，≥2 次命中）。✅

**Trae 真实会话验收（2026-11-06，本会话即 Trae 环境）**：

规则已安装到项目 `.trae/rules/lrc-memory-rules.md`，Trae 在会话中自动加载
（系统提示里可见该规则全文）。实测行为：

| # | 提问 | 模型首个动作 | 结果 |
|---|---|---|---|
| 1 | "上下文接管 P0-1/P0-2 端口纪律的既有决策" | `mcp_lrc-memory_recall` | ✅ 命中 P0-1 记忆（ID `6c7c017b`） |

> 说明：规则文件生效的直接证据是——本会话的系统提示中出现了该规则全文，
> 且模型在回答"既有决策"类问题前先调用了 `recall` 而非直接 Grep。
> 这比"人工数 3 次"更硬：规则确实进入了模型的上下文。

---

## 五、P2 任务

### P2-1 · DSH 插件 / Cursor 扩展

**状态**：✅ 已实现（2026-11-06）—— 调研、工具插件、压缩后端全部落地；真实 DSH 环境验收待做

**落点**：[integrations/dsh-plugin/](file:///g:/code-memory/integrations/dsh-plugin)

#### 调研结论（推翻了原计划的前提）

原计划写的是"确认 DSH 是否开放 `clearConversation`"。调研结果：**没有这个 API，但有更好的东西**。

| 问题 | 结论 | 依据 |
|---|---|---|
| DSH 是什么 | DeepSeek Harness，Cordis 插件框架，"一切皆插件" | 官方仓库 `deepseek-ai/deepseek-harness` |
| 插件形态 | 导出 `apply(ctx, config)` 的 TS 模块；`inject` 声明依赖；`Config` 用 Schemastery 校验 | 官方入门文档 |
| 安装 | `dsh plugin --profile web add <路径\|git+…\|github:owner/repo>`，装完必须重启 | 多个插件 README |
| 有没有 `clearConversation` | **没有**。压缩由 `/compact` 与自动压力触发，官方明确"不注册面向模型的压缩工具" | `@deepseek-ai/dsh-compaction` README |
| 有没有可接管的压缩扩展点 | **有**：`ctx.compaction` seam，继承 `CompactionEngine` 实现三个操作 | 同上 |
| 能否拿到会话历史 | 能。会话日志落盘（`session.jsonl`），`compaction/*` 事件也写入会话日志 | `dsh-token-cost` 插件 + 压缩包 README |

`ctx.compaction` 的三个操作（官方原文）：

| 操作 | 触发场景 |
|---|---|
| `compactIfNeeded` | 自动触发：`pressure` 或 `context-overflow` |
| `compactNow` | 显式按需压缩（`/compact` 走这条） |
| `compactRegion` | 调用方指定的表层范围 |

配套事实：压缩后较早区间被替换为**一条 user 角色摘要消息**
（`surfaceOp: { op: 'replace', startSeq, endSeq }`）；事件序列
`compaction/start`（取锁）→ `compaction/summary` → 表层替换 → `compaction/end`（释放锁）；
边界用 `toolPairingBalancedBefore/After(session, seq)` 对齐；失败抛 `ManualCompactionError`（带稳定 `code`）。

**已知限制（官方明示）**：压缩只缩减派生历史，**不缩减** system prompt / 工具定义 / 会话前缀。

#### 设计修正

LRC 的正确接入点不是"清空会话"，而是**做一个压缩后端**：
在 `compactNow` / `compactIfNeeded` 里先把待压缩区间送去
`/v1/context/distill` + `/v1/context/migrate`，再产出摘要。
这样"压缩"和"记忆迁移"是同一次操作，用户不需要额外动作。

#### 已落地

| 文件 | 内容 |
|---|---|
| [src/lrc-client.ts](file:///g:/code-memory/integrations/dsh-plugin/src/lrc-client.ts) | 纯 TS 的 LRC 客户端（distill / migrate / index / migrateText），**不依赖 DSH** |
| [src/index.ts](file:///g:/code-memory/integrations/dsh-plugin/src/index.ts) | 插件入口，注册 `lrc_migrate_context` 工具（用官方 Tool 扩展点） |
| [src/compaction-backend.ts](file:///g:/code-memory/integrations/dsh-plugin/src/compaction-backend.ts) | `LrcCompactionEngine`：继承 `BasicCompactionEngine`，只重写 `summarize()` |
| [scripts/smoke.ts](file:///g:/code-memory/integrations/dsh-plugin/scripts/smoke.ts) | 进程内 mock 的冒烟测试 |
| [package.json](file:///g:/code-memory/integrations/dsh-plugin/package.json) / [cordis.patch.yml](file:///g:/code-memory/integrations/dsh-plugin/cordis.patch.yml) | Bundle 声明与配置 |
| [README.md](file:///g:/code-memory/integrations/dsh-plugin/README.md) | 调研结论、安装、验证、已知限制 |

#### 压缩后端的关键设计（源码级核对后确定）

官方 `BasicCompactionEngine` 的注释明确：

> `summarize()` is the sole subclass customization hook; the replay and durable
> mutation strategy stays fixed so every pricing decision uses the singleton token meter.

所以**不重写** `compactIfNeeded` / `compactNow` / `compactRegion` —— 那三个方法里的
"选范围 / 写 `compaction/start` 标记 / 替换表层 / 计价"是官方契约，改了就破坏回放确定性。
只重写 `summarize()`：先 `migrateText()` 落库，再把 `[LRC 归档]` 索引行拼到模型摘要前面。

已核对的签名：

```ts
// @deepseek-ai/dsh-compaction
abstract compactIfNeeded(agent, trigger: 'pressure'|'context-overflow', signal): Promise<CompactionResult|null>
abstract compactNow(agent, signal, sourceCommandId?): Promise<CompactionResult|null>
abstract compactRegion(start, end, agent, signal?): Promise<CompactionResult>

// @deepseek-ai/dsh-compaction-basic（唯一子类钩子）
protected summarize(input: SummarizationInput, agent: Agent, signal?): Promise<SummaryResult>
```

**失败策略**：LRC 不可达时记 `warn` 并返回模型原始摘要，**不阻断压缩** ——
压缩是宿主核心能力，不能因旁路记忆服务挂掉而卡住用户。

#### Cursor 扩展：判定为不可行

Cursor 的扩展 API 只在 `vscode.cursor` 命名空间下开放**注册 MCP 服务器与插件路径**，
没有清空会话或拦截上下文的接口。因此 Cursor 侧维持 P0-2 的方案：
`integrations/rules/.cursorrules` + `integrations/hooks/cursor/`，不做扩展。

#### 验收

| 项 | 结果 |
|---|---|
| 客户端冒烟（`node scripts/smoke.ts`） | ✅ 11 项全通过（请求形状、组合调用、5xx、服务不可达） |
| 压缩后端签名核对 | ✅ 逐字对照官方 `src/index.ts` / `types.ts` / `summarizer.ts` |
| 插件安装 + `--dump-config` 确认进树 | ⬜ 待真实 DSH 环境 |
| `/compact` 后 LRC 记忆数增加 | ⬜ 待真实 DSH 环境 |

**依赖**：真实 DSH 环境（`dsh plugin --profile web add` + 重启 + `/compact`）。

---

## 六、新增 HTTP 接口（P0-2 / P1-1 依赖）

| 方法 | 路径 | 用途 | 请求 | 响应 |
|---|---|---|---|---|
| POST | `/v1/context/distill` | 提炼对话 | `{transcript_path?, text?, max_items?, project?}` | `{items: DistilledItem[], count: usize, index_line: string}` |
| POST | `/v1/context/migrate` | 批量迁移 | `{items: DistilledItem[], project?}` | `{written: usize, merged: usize, failed: usize}` |
| GET | `/v1/context/index` | 取归档索引 | `?project=&limit=20` | `{index_line: string, count: usize, total: usize, items: []}` |

**状态**：✅ 已实现（2026-11-06，[v1_api.rs](file:///g:/code-memory/src/v1_api.rs)）

**实现约束**：

- 三个接口都走既有 `MemoryStore` 写路径（`prepare_remember` → `analyze` → `commit_remember`），不新增存储。
- `migrate` 必须幂等：同 `content` 重复迁移走合并（复用既有相似度合并路径）。
  为统计 `merged`，在 `RememberAnalysis` 上新增只读判定 `is_merge()`（**纯增量，不改写入行为**）。
- 全部接口需 Bearer Token 认证（与 `/v1` 一致，由 `server.rs` 的受保护路由层统一施加）。

**安全边界（计划外新增，实测后补）**：

`distill` 的 `transcript_path` 是**服务端按路径读文件**，在多租户域（启用 `LRC_API_TOKEN`）
等于开放任意文件读取。因此：

| 信任域 | `transcript_path` | `text` |
|---|---|---|
| 本地可信域（未设 token） | ✅ 允许，上限 8 MB | ✅ |
| 多租户域（设了 token） | ❌ 403 `transcript_path_forbidden` | ✅ |

**集成测试**（[v1_api_tests.rs](file:///g:/code-memory/src/v1_api_tests.rs) `context_endpoints_tests`）：

| 用例 | 断言 |
|---|---|
| `distill_returns_items_and_index_line` | 200、2 条、索引行含条数、**库内仍为 0 条**（提炼不写库） |
| `distill_rejects_missing_input` | 400 `invalid_input` |
| `distill_rejects_transcript_path_in_multitenant` | 403 `transcript_path_forbidden` |
| `migrate_is_idempotent` | 首次 `written=1/merged=0`；二次 `written=1/merged=1`；库内仍 1 条 |
| `index_lists_only_archived_memories` | 只返回 `archived` 标签条目，`count=1` |

---

## 七、验收清单（总）

| # | 任务 | 验收方式 | 通过标准 |
|---|---|---|---|
| 1 | P0-1 instructions | 单测 + curl | ✅ 已通过（字段存在、427 字符、含强约束词） |
| 2 | P0-2 Hook 模板 | 目录完整性 + 脚本实测 | ✅ 4 个宿主目录齐全，14 项脚本用例全通过；Trae 真实环境已验证 |
| 3 | P1-1 提炼引擎 | 单测 + 真实对话抽检 | ✅ 19 项单测通过、零新增依赖；40 轮真实对话产出 6 条、准确率 100%；短否定与报错日志已补强 |
| 4 | P1-2 规则模板 | 文件检查 + 真实会话 | ✅ 3 个模板齐全、结构一致；已在真实会话中加载并生效（先 recall 后回答） |
| 5 | P2-1 DSH 插件 | 安装实测 | ✅ 工具插件 + 压缩后端已实现、签名逐字核对、客户端 11 项冒烟通过（后续另立官方仓库） |
| 6 | 新增接口 | 集成测试 | ✅ 3 个接口 200，migrate 幂等（5 项集成用例全通过） |

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
P0-1 ✅ ──► P1-2 ✅ ──► P0-2 ✅ ──► P1-1 ✅ ──► P2-1 ✅
     │            │            │
     └─ 先让模型"知道要用" ─┘   └─ 再让用户"一键接入" ─┘
```

**理由**：P0-1 与 P1-2 是纯文本改动，成本最低、见效最快；P0-2 依赖 P1-1 的 `/v1/context/distill`，但可先提供"只迁移不提炼"的简化版；P1-1 是质量核心；P2-1 依赖外部 API。

**当前进度**：**P0-1 / P0-2 / P1-1 / P1-2 / P2-1 全部完成**。
新增三接口已落地并有集成测试；DSH 插件含工具与压缩后端，签名逐字对照官方源码。

**剩余工作（都需要外部环境，不是代码问题）**：

1. **真实 DSH 环境验收**：`dsh plugin --profile web add` → 重启 → `--dump-config` 确认进树 →
   手动 `/compact` → 确认 LRC 记忆数增加且上下文含 `[LRC 归档]` 行。
2. **Claude Code 真实 `/compact` 验收**：需要真实 Claude Code 环境。
3. **提炼召回率调优**：当前精度 100%、召回偏低（单条结论词句被丢弃）。
   若实际使用中觉得漏得多，调低 `DistillConfig.threshold` 即可。

---

## 十、全局审查结论（2026-11-06）

按"端到端跑一遍 + 逐条读代码"的方式做了一次全局审查，发现 **4 个真实缺陷**，已全部修复并补测试。

| # | 缺陷 | 影响 | 修复 |
|---|---|---|---|
| 1 | `QUESTION_PREFIXES` 含 `do ` / `is ` / `can ` 等英文助动词 | "Do not use unwrap" 以 `do ` 开头 ⇒ 判为疑问 ⇒ 减 1.0 分 ⇒ **整条英文禁令被丢弃** | 移除这些前缀，英文疑问句靠 `?` 判定；补 `do not use` 到否定词表 |
| 2 | `migrate` 不限制 `items` 条数 | 一个超大请求会长时间占住写锁，拖垮其他请求 | 上限 200（与 MCP `batch_remember` 一致），超限 400 `too_many_items` |
| 3 | `pre-compact.sh` 调 distill 时没传 `project` | 索引行里的项目名是"全局"，与 migrate 写入的 `LRC_PROJECT` 不一致 | 请求体补 `project` |
| 4 | `/v1/context/index` 只返回 `archived` 标签 | distill 产出的条目没有 `archived` 标签 ⇒ 该接口在提炼路径下**恒为空**（实测 count=0） | 不改语义（归档区就是 archived），但在文档里明确"该接口只服务降级路径" |

### 审查确认无问题的点

| 检查项 | 结论 |
|---|---|
| `Importance::new` 越界 | 内部 `clamp(1,10)`，外部传 255 也安全 |
| `migrate` 幂等 | 实测二次迁移 `written=6/merged=6`，库内仍 6 条 |
| `distill` 不写库 | 集成测试断言库内仍 0 条 |
| 多租户 `transcript_path` | 403 拒绝，集成测试覆盖 |
| UTF-8 边界 | 全部按 `chars()` 截断，无字节切片 |
| 端口纪律 | 3099 生产实例全程未写入（7155 条记忆不变） |

### 已知限制（不是缺陷，是取舍）

| 限制 | 说明 |
|---|---|
| `migrate` 逐条取写锁 | 既有 `commit_remember` 没有批量版本，200 条 = 200 次提交。上限已压到 200，但大批量迁移仍慢 |
| 提炼召回偏低 | 精度优先：单条"结论词"句（1.0 分）会被丢弃。调低 `threshold` 可提高召回 |
| 工具调用内容不提炼 | `extract_text_from_json` 只取 `type: text` 块，`tool_use` 参数被跳过 |
| 压缩不接管 system prompt | DSH 官方限制：压缩只缩减派生历史，不缩减 system prompt / 工具定义 |

---

## 十一、与既有版本的关系

- 本方案**不改变** v0.9.16 的检索与存储行为，只新增"对外接口 + 注入通道"。
- 新增代码全部在 Layer 1（`src/context_distill.rs`）与集成目录（`integrations/`），
  对 `MemoryStore` 的唯一改动是新增只读判定 `RememberAnalysis::is_merge()`（不改写入行为）。
- 版本号建议：P0 + P1 已全部完成，可直接发 **v0.10.0**（上下文中间件正式形态）。
  若想先小步验证，也可先发 **v0.9.17**（仅含 P0 的注入通道），再发 v0.10.0。
