# Cursor 接入（P0-2）

一个 Hook：模型即将 Read / Grep / Shell 之前，若参数明显在"翻历史"，提示先 `recall`。

## 适用版本（先读这段）

Cursor 的 hooks 事件名与输出字段**随版本变化**。本模板按
`beforeReadFile` / `beforeShellExecution` 编写，输出 `{permission, agent_message}`。

安装前先确认你的 Cursor 版本：

- **支持上述事件**：按下面步骤装。
- **不支持**：改用降级方案 —— 把 `integrations/rules/.cursorrules` 复制到项目根目录。
  规则文件是 Cursor 稳定支持的通道，约束力弱于 Hook，但零配置。

## 安装

```bash
# 1) 脚本放到项目的 .cursor/hooks/ 目录（项目级，不污染全局）
mkdir -p .cursor/hooks
cp pre-tool-use.sh .cursor/hooks/lrc-pre-tool-use.sh
chmod +x .cursor/hooks/lrc-pre-tool-use.sh

# 2) 合并 hooks.json 到 .cursor/hooks.json（合并，不要覆盖）
```

## 两种模式

| 模式 | 设置 | 行为 | 建议 |
|---|---|---|---|
| `warn` | 默认 | 放行 + 附提示，模型自己改道 | **先跑一周**，观察误伤率 |
| `deny` | `export LRC_HOOK_MODE=deny` | 直接拒绝本次调用 | 误伤率确认低之后再切 |

> 为什么默认 `warn`：`deny` 会打断正常读代码。比如用户说"改一下上次那个函数"，
> 模型需要先 Read 才能改 —— 这时 deny 是误伤。warn 只提示，不阻断。

## 验证

```bash
# 1) 命中关键词：应输出 permission=allow + 提示
echo '{"tool_name":"Grep","tool_input":{"pattern":"上次怎么改的"}}' \
  | bash .cursor/hooks/lrc-pre-tool-use.sh

# 2) 不命中：应无输出
echo '{"tool_name":"Grep","tool_input":{"pattern":"fn main"}}' \
  | bash .cursor/hooks/lrc-pre-tool-use.sh

# 3) deny 模式
echo '{"tool_name":"Grep","tool_input":{"pattern":"历史决策"}}' \
  | LRC_HOOK_MODE=deny bash .cursor/hooks/lrc-pre-tool-use.sh
```

## 降级路径

| 情况 | 行为 |
|---|---|
| Cursor 版本不支持 hooks | 改用 `integrations/rules/.cursorrules` |
| 没有 `jq` | 脚本静默退出（`exit 0`），不阻断任何调用 |
| 字段名与模板不符 | 脚本对 `tool_name/toolName`、`tool_input/toolInput` 都做了兼容；仍不符则静默放行 |

## 已知限制

- 判定依据是**工具参数里的关键词**，不是语义。模型用 `Read` 读一个叫 `history.md`
  的文件会被误判（warn 模式下无害）。
- 本 Hook 不访问 LRC 服务，因此不涉及端口配置。
