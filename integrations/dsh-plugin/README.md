# DSH 插件（P2-1）

把 DeepSeek Harness（DSH）的会话上下文迁移进 LRC 记忆库。

## 一、调研结论（2026-11-06，已核实来源）

原计划写的是"确认 DSH 是否开放 `clearConversation`"。调研结果：**没有这个 API，但有更好的东西**。

| 问题 | 结论 | 依据 |
|---|---|---|
| DSH 是什么 | DeepSeek Harness，Cordis 插件框架，"一切皆插件" | 官方仓库 `deepseek-ai/deepseek-harness` |
| 插件形态 | 导出 `apply(ctx, config)` 的 TS 模块；`inject` 声明依赖；`Config` 用 Schemastery 校验 | 官方入门文档 |
| 安装方式 | `dsh plugin --profile web add <路径\|git+...\|github:owner/repo>`，装完必须重启 | 多个插件 README |
| 有没有 `clearConversation` | **没有**。压缩由 `/compact` 命令与自动压力触发，且官方明确"不注册面向模型的压缩工具" | `@deepseek-ai/dsh-compaction` README |
| 有没有可接管的压缩扩展点 | **有**：`ctx.compaction` seam，继承 `CompactionEngine` 实现三个操作 | 同上 |
| 能否拿到会话历史 | 能。会话日志落盘（`session.jsonl`），且压缩事件 `compaction/*` 写入会话日志 | `dsh-token-cost` 插件 + 压缩包 README |

### `ctx.compaction` 的三个操作（官方原文）

| 操作 | 触发场景 |
|---|---|
| `compactIfNeeded` | 自动触发：`pressure` 或 `context-overflow` |
| `compactNow` | 显式按需压缩（`/compact` 走这条） |
| `compactRegion` | 调用方指定的表层范围 |

配套事实：

- 压缩后，较早区间被替换为**一条 user 角色摘要消息**（`surfaceOp: { op: 'replace', startSeq, endSeq }`），近期历史不动。
- 事件序列：`compaction/start`（取锁）→ `compaction/summary` → 表层替换 → `compaction/end`（释放锁）。
- 边界对齐用 `toolPairingBalancedBefore/After(session, seq)`，避免切断未回答的工具调用对。
- 失败抛 `ManualCompactionError`，带稳定 `code`。
- **已知限制**：压缩只缩减派生历史，**不缩减** system prompt / 工具定义 / 会话前缀。

### 这意味着什么

LRC 的正确接入点不是"清空会话"，而是**做一个压缩后端**：
在 `compactNow` / `compactIfNeeded` 里，先把待压缩区间的文本送去
`/v1/context/distill` + `/v1/context/migrate`，再产出摘要。
这样"压缩"和"记忆迁移"是同一次操作，不需要用户额外动作。

## 二、当前实现范围

| 能力 | 状态 |
|---|---|
| `lrc_migrate_context` 工具（模型显式调用，提炼 + 迁移） | ✅ 已实现 |
| LRC HTTP 客户端（distill / migrate / index） | ✅ 已实现，11 项冒烟测试通过 |
| 压缩后端（`LrcCompactionEngine`，`/compact` 时自动迁移） | ✅ 已实现（源码级核对签名） |

### 压缩后端为什么只重写 `summarize()`

官方 `BasicCompactionEngine` 的注释写得很明确：

> `summarize()` is the sole subclass customization hook; the replay and durable
> mutation strategy stays fixed so every pricing decision uses the singleton token meter.

也就是说"选范围 / 写 `compaction/start` 标记 / 替换表层 / 计价"全部固定在
`compactSurfaceRegion` 里，子类只该改**摘要内容**。所以本后端不碰锁与事件序列 ——
那些是官方契约，改了就破坏回放确定性。

### 已核对的官方签名（`@deepseek-ai/dsh-compaction`）

```ts
export abstract class CompactionEngine extends Service {
  constructor(ctx: Context) { super(ctx, 'compaction') }

  abstract compactIfNeeded(
    agent: CompactionAgentContext,
    trigger: 'pressure' | 'context-overflow',
    signal: AbortSignal,
  ): Promise<CompactionResult | null>

  abstract compactNow(
    agent: ManualCompactAgentContext,
    signal: AbortSignal,
    sourceCommandId?: CommandId,
  ): Promise<CompactionResult | null>

  abstract compactRegion(
    start: SessionSeq,
    end: SessionSeq,
    agent: CompactionAgentContext,
    signal?: AbortSignal,
  ): Promise<CompactionResult>
}
```

`CompactionResult` 字段：`compactionId` / `startSeq` / `summarySeq` / `endSeq` /
`summary` / `shadowedRange` / `shadowedSeqs` / `shadowedTokenCount`。

`ManualCompactionErrorCode`：`busy | cancelled | changed | summary | commit | persistence`。

`BasicCompactionEngine` 的钩子：

```ts
protected async summarize(
  input: SummarizationInput,   // { tools?: ToolSchema[]; messages: readonly Message[] }
  agent: Agent,
  signal?: AbortSignal,
): Promise<SummaryResult>       // { summary: ContentBlock[]; provider; model; ... }
```

### 失败策略

LRC 不可达时**不阻断压缩**：记一条 `warn`，返回模型原始摘要。
理由：压缩是宿主的核心能力，不能因为一个旁路记忆服务挂掉就让用户卡住。

## 三、安装

```bash
# 1) 装进 web profile（本地路径）
dsh plugin --profile web add ./integrations/dsh-plugin

# 2) 重启 DSH（装/改/删插件后必须重启）

# 3) 确认插件真的进了插件树
dsh --profile web --dump-config | grep dsh-plugin-lrc
```

> 第 3 步不能省。`package.json` 缺 `dsh.bundle.patch` 时，包会被当成普通 npm 依赖装上，
> 但**不会**进入插件树 —— 表现为"装成功了但没反应"。

## 四、配置

改 `cordis.patch.yml`：

| 字段 | 缺省 | 说明 |
|---|---|---|
| `baseUrl` | `http://127.0.0.1:3099` | 开发联调改 `3111` |
| `token` | 空 | 服务端设了 `LRC_API_TOKEN` 时必填 |
| `maxItems` | `40` | 单次迁移最多条数 |

## 五、验证

### 5.1 客户端冒烟（不需要 DSH，也不需要 LRC）

```bash
node integrations/dsh-plugin/scripts/smoke.ts
```

在进程内起 mock 服务，验证请求形状与错误处理。实测 11 项全通过：

```
1) distill        PASS 返回 1 条 / 索引行含条数 / 请求体含 text+project+max_items
2) migrate        PASS written=1 / 请求体含 items+project
3) index          PASS count=1 / 查询串带 project 与 limit
4) migrateText    PASS distilled=1 且 written=1
5) 5xx            PASS 抛出且带状态码
6) 服务不可达      PASS 抛出异常
```

### 5.2 插件端到端（需要 DSH + LRC）

```bash
# 1) LRC 在线
curl -s http://127.0.0.1:3099/health

# 2) 在 DSH 对话里说：
#    "用 lrc_migrate_context 把这段结论记下来：我们决定用 pnpm 替代 npm。"

# 3) 确认落库
curl -s http://127.0.0.1:3099/v1/memories/stats
```

## 六、启用压缩后端

默认只装工具插件。要让 `/compact` 自动迁移，需要把压缩后端也挂上：

1. 打开 profile 的 `cordis.patch.yml`（`$DSH_HOME/profiles/web/`）。
2. **删掉**原有的 `@deepseek-ai/dsh-compaction-basic` 那一行 ——
   同一 context 只能有一个 `ctx.compaction` 实现，两个都挂会冲突。
3. 把本插件 `cordis.patch.yml` 末尾注释掉的那段 `dsh-plugin-lrc-compaction` 取消注释并追加进去。
4. 重启 DSH。
5. 确认：`dsh --profile web --dump-config | grep compaction` 应只看到 `dsh-plugin-lrc-compaction`。

### 验证

```bash
# 1) 记下当前记忆数
curl -s http://127.0.0.1:3099/v1/memories/stats

# 2) 在 DSH 里手动 /compact

# 3) 记忆数应增加；压缩后的上下文应含 [LRC 归档] 行
curl -s http://127.0.0.1:3099/v1/memories/stats
```

### 需要你核对的一处

`compaction-backend.ts` 从 `@deepseek-ai/dsh-compaction-basic/summarizer` 导入
`SummarizationInput` / `SummaryResult` 两个**类型**。若你的版本没有导出这个子路径，
把这两个类型声明直接抄进 `compaction-backend.ts` 即可（它们是纯类型，运行时无影响）：

```ts
interface SummarizationInput { tools?: readonly ToolSchema[]; messages: readonly Message[] }
type SummaryResult = { summary: ContentBlock[]; provider: string; model: string; maxTokens?: number }
```

## 七、已知限制

- DSH 处于 Developer Preview，插件 API 可能不兼容变更。**锁版本**，不要长期用 `*`。
- 插件以 DSH 进程权限运行，能读你的文件、用你的凭据。只装可信来源。
- 压缩不缩减 system prompt / 工具定义，所以"上下文接管"只能接管**对话历史**部分。
- 本插件不处理图片与附件，只处理文本（`extractText` 只取 `type === 'text'` 的块）。
- 单次送进 LRC 的文本上限 20 万字符（LRC 请求体上限 2 MiB，留足余量）。
