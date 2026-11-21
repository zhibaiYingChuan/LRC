# Codex 接入（B1 · notify）

任务结束时（`agent-turn-complete`），Codex 会执行 `notify` 配置的外部程序，
并把一份 JSON 作为**最后一个命令行参数**传入。LRC 直接复用 `lrc absorb` 接收它。

## 安装

编辑 `~/.codex/config.toml`（Windows 通常是 `C:\Users\<你>\.codex\config.toml`）：

```toml
# 方式一：lrc 已在 PATH 中
notify = ["lrc", "absorb", "--host", "codex", "--no-spawn", "--timeout", "4000", "--json"]
```

```toml
# 方式二：写绝对路径（Windows 的 TOML 字符串里反斜杠要转义，或用正斜杠）
notify = ["C:/Users/you/.cargo/bin/lrc.exe", "absorb", "--host", "codex", "--no-spawn", "--timeout", "4000", "--json"]
```

> `--no-spawn`：钩子场景不自动拉起 sidecar。daemon 未运行时直接失败退出，
> 不让 Codex 的一轮对话等一次模型冷加载（钩子预算 4 秒）。
>
> `--timeout 4000`：CLI 的 HTTP 超时按毫秒计。

## Codex 传给 LRC 的载荷

```json
{
  "type": "agent-turn-complete",
  "thread-id": "0199a1c2-...",
  "turn-id": "turn-3",
  "cwd": "G:\\code-memory",
  "last-assistant-message": "已完成。\n[LRC]\n- decision: 采用 RwLock\n- constraint: 不要提交 secrets",
  "input-messages": ["帮我把锁改成读写锁"]
}
```

LRC 读取的字段（兼容 snake_case / kebab-case）：

| 用途 | 字段 |
|---|---|
| 本轮文本 | `last-assistant-message`（也接受 `last_assistant_message` / `text` / `response`） |
| 会话 | `thread-id`（也接受 `thread_id` / `session_id` / `sessionId`） |
| 项目归属 | 命令行 `--project` > 载荷 `project` > 载荷 `cwd` 的目录名 |

## 验证（不依赖 Codex）

```bash
# 模拟 Codex 的 argv 传参（JSON 作为最后一个参数）
lrc absorb --host codex --no-spawn --json \
  '{"type":"agent-turn-complete","thread-id":"t1","cwd":"/tmp/code-memory","last-assistant-message":"已完成。\n[LRC]\n- decision: 采用 RwLock，读并发优先"}'
```

```powershell
# Windows PowerShell：单引号包住 JSON，避免 $ 与反斜杠被展开
lrc absorb --host codex --no-spawn --json '{"type":"agent-turn-complete","thread-id":"t1","cwd":"G:\\code-memory","last-assistant-message":"已完成。\n[LRC]\n- decision: 采用 RwLock，读并发优先"}'
```

预期：输出一行 JSON，`candidates >= 1`；不带 `--dry-run` 时 `written >= 1`。
随后 `lrc recall "RwLock"` 应能命中。

## 边界

- `notify` 目前只在 `agent-turn-complete` 触发；若未来 Codex 增加其他事件，
  LRC 仍按"载荷里有文本才沉淀"处理，不会把非回复事件写进库。
- 自动沉淀固定 `importance_cap=4`、打 `auto-summary` 标签、绝不写 `Global`；
  命中删除语义时整轮跳过（不复活已删除内容）。
- 未配置 `[LRC]` 结论块时回退到规则提炼，只收结论类句子（宁可漏，不可滥）。
