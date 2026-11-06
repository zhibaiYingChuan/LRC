# Claude Code 接入（P0-2）

两个 Hook：会话启动注入索引、上下文压缩前迁移记忆。

## 适用版本

Claude Code 支持 `hooks` 配置的版本（`SessionStart` / `PreCompact` 事件）。
若你的版本没有 `PreCompact`，只装 `SessionStart` 也能拿到 80% 收益。

## 安装

```bash
# 1) 脚本放到 Claude Code 的 hooks 目录
mkdir -p ~/.claude/hooks
cp pre-compact.sh session-start.sh ~/.claude/hooks/
mv ~/.claude/hooks/pre-compact.sh  ~/.claude/hooks/lrc-pre-compact.sh
mv ~/.claude/hooks/session-start.sh ~/.claude/hooks/lrc-session-start.sh
chmod +x ~/.claude/hooks/lrc-*.sh

# 2) 合并 settings.json 的 hooks 字段到 ~/.claude/settings.json
#    ⚠ 是"合并"，不是覆盖 —— 覆盖会丢掉你已有的 hooks
```

## 需要改的变量

| 变量 | 何时改 |
|---|---|
| `LRC_BASE` | 只在开发联调时改（`http://127.0.0.1:3111`）。**不要改脚本里的默认值** |
| `LRC_TOKEN` | 服务端设置了 `LRC_API_TOKEN` 时必须设，否则 401 |
| `LRC_PROJECT` | 缺省取当前目录名；多项目共用记忆库时建议显式指定 |

写进 `~/.claude/settings.json` 的 `env` 字段即可全局生效：

```json
{ "env": { "LRC_BASE": "http://127.0.0.1:3099" } }
```

## 验证

```bash
# 1) 服务在线
curl -s http://127.0.0.1:3099/health

# 2) 索引注入（应打印【LRC 记忆索引 · ...】）
bash ~/.claude/hooks/lrc-session-start.sh

# 3) 迁移（手动喂一个 transcript 路径）
echo '{"transcript_path":"/path/to/transcript.jsonl"}' \
  | bash ~/.claude/hooks/lrc-pre-compact.sh

# 4) 确认记忆数增加
curl -s http://127.0.0.1:3099/v1/memories/stats
```

在 Claude Code 里手动执行 `/compact`，压缩后的上下文应出现 `[归档]` 开头的索引行。

## 降级路径

| 情况 | 行为 |
|---|---|
| `/v1/context/distill` 未实现（P1-1 未上线） | 自动降级为"只迁移不提炼"：把对话尾部原文写成一条 `code_context` 记忆，`tags` 含 `archived` |
| LRC 服务未启动 | `session-start.sh` 静默退出（不污染上下文）；`pre-compact.sh` 打印一行失败提示 |
| 没有 `jq` | 脚本直接报错退出。先装 jq |

## 已知限制

- 降级模式写入的是**原文尾部**，噪声较大。P1-1 上线后自动切换到提炼模式，无需改脚本。
- `PreCompact` 的 `timeout` 设为 60s。若你的库很大（>20 万条），迁移可能超时，可调大。
- 脚本按 POSIX 编写，只用 `curl` / `jq` / `tail` / `head` 的通用参数，macOS 与 Linux 均可直接跑。
  Windows 上 Claude Code 用 Git Bash 执行，同样适用。
- 实测环境：`jq 1.7.1`、`bash 5.x`。jq 1.6 及以上均可。
