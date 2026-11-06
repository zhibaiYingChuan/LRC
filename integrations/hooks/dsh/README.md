# DSH 接入（P0-2 · 手动方案）

DSH 的插件 API 尚未确认（`clearConversation` 是否开放未知，见 P2-1）。
在插件落地之前，用**手动方案**拿到同样的效果：MCP + 规则文件 + 两条手动命令。

## 手动方案（三步）

### 1. 接 MCP

在 DSH 的 MCP 配置里加一条（stdio 或 HTTP 均可，取决于你的 DSH 版本）：

```json
{
  "mcpServers": {
    "lrc": {
      "url": "http://127.0.0.1:3099/mcp"
    }
  }
}
```

接上后，`initialize` 会自动下发 `instructions`（P0-1），模型开局就知道要用 `recall`。

### 2. 装规则

把 `integrations/rules/AGENTS.md` 的内容放进 DSH 读取的规则文件位置
（通常是项目根的 `AGENTS.md`）。这是 MCP `instructions` 的兜底 ——
部分宿主会忽略 `instructions` 字段。

### 3. 手动执行"清空 + 迁移"

DSH 没有 Hook 时，上下文快满时手动跑一次：

```bash
# 把当前对话的结论写进 LRC（替换成你实际的对话导出路径）
curl -sS -X POST http://127.0.0.1:3099/v1/memories/remember \
  -H 'Content-Type: application/json' \
  -d '{
    "content": "[归档] 本轮结论：<在这里粘贴关键决策>",
    "memory_type": "decision",
    "importance": 7,
    "project": "<项目名>",
    "tags": ["archived", "manual-migrate"]
  }'

# 然后在 DSH 里新建会话（等价于清空上下文）
```

## 验证

```bash
curl -s http://127.0.0.1:3099/v1/memories/stats          # 记忆数应增加
curl -s "http://127.0.0.1:3099/v1/memories/recent?limit=3"  # 应看到刚写的归档
```

## 为什么不做自动化

DSH 的插件 API 未确认，任何"猜 API"的脚本都会在版本升级后静默失效，
比手动方案更危险。等 P2-1 拿到官方 API 文档后再做插件。

## 降级路径

| 情况 | 行为 |
|---|---|
| DSH 忽略 MCP `instructions` | 靠第 2 步的规则文件兜底 |
| DSH 不支持 MCP | 只能走 HTTP：`curl` 调 `/v1/memories/remember` 与 `/v1/memories/enrich` |
| 端口不符 | 检查 `LRC_BASE`；开发环境是 3111，正式是 3099 |
