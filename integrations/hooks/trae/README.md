# Trae 接入（P0-2）

两个 Hook：会话启动注入记忆索引、每次提交 prompt 前重新强调规则（"稀释"）。

## 一、Trae 确实支持 Hook（已核实）

官方文档：[Hook 配置详解](https://docs.trae.cn/ide_hook-configuration-reference)

| 项 | 事实 |
|---|---|
| 配置文件（全局） | `%userprofile%/.trae-cn/hooks.json`（macOS/Linux：`~/.trae-cn/hooks.json`） |
| 配置文件（项目） | `<项目根>/.trae/hooks.json` |
| 支持的事件 | `SessionStart` / `UserPromptSubmit` / `PreToolUse` / `PostToolUse` / `Stop` |
| 输入 | stdin 传 JSON |
| 输出 | stdout 内容按事件类型解析为 JSON 或纯文本；退出码 0 = 正常 |
| 环境变量 | `TRAE_PROJECT_DIR`、`CLAUDE_PROJECT_DIR`（均为工作区目录） |
| 兼容性 | Trae 也能直接读 Claude Code 的 Hook 配置 |

> 早期版本的本文档写过"Trae 可能不支持 hooks，请降级用规则文件"——**那是错的**，已更正。

## 二、安装

### 方式 A：项目级（推荐，只影响当前项目）

```powershell
# 1) 脚本放到项目的 .trae/hooks/
New-Item -ItemType Directory -Force .\.trae\hooks | Out-Null
Copy-Item .\session-start.ps1      .\.trae\hooks\lrc-session-start.ps1
Copy-Item .\user-prompt-submit.ps1 .\.trae\hooks\lrc-user-prompt-submit.ps1

# 2) 把 hooks.json 复制到项目根
Copy-Item .\hooks.json .\.trae\hooks.json
```

### 方式 B：全局（对本机所有工作区生效）

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE\.trae-cn\hooks" | Out-Null
Copy-Item .\session-start.ps1      "$env:USERPROFILE\.trae-cn\hooks\lrc-session-start.ps1"
Copy-Item .\user-prompt-submit.ps1 "$env:USERPROFILE\.trae-cn\hooks\lrc-user-prompt-submit.ps1"
# 再把 hooks.json 合并进 $env:USERPROFILE\.trae-cn\hooks.json
```

> 全局安装时，命令里的相对路径 `.\.trae\hooks\...` 要改成绝对路径。

## 三、配置格式（必须嵌套 `hooks` 数组）

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [ { "type": "command", "command": "powershell ... -File .\\.trae\\hooks\\lrc-session-start.ps1", "timeout": 15 } ] }
    ],
    "UserPromptSubmit": [
      { "hooks": [ { "type": "command", "command": "powershell ... -File .\\.trae\\hooks\\lrc-user-prompt-submit.ps1", "timeout": 10 } ] }
    ]
  }
}
```

**常见错误**：把 `{"type":"command","command":"..."}` 直接放进事件数组，漏掉中间那层 `"hooks": [...]`。
漏了不会报错，只是**静默不生效**。

## 四、需要改的变量

| 变量 | 何时改 |
|---|---|
| `LRC_BASE` | 只在开发联调时改（`http://127.0.0.1:3111`）。**不要改脚本默认值** |
| `LRC_TOKEN` | 服务端设置了 `LRC_API_TOKEN` 时必须设 |
| `LRC_PROJECT` | 缺省取当前目录名 |
| `LRC_ALWAYS_REMIND` | 默认 `1`（每轮注入一行提醒）。设 `0` 则只在命中关键词时注入 |
| `LRC_INDEX_LIMIT` | 索引条数，默认 8 |

## 五、prompt 的三条输入通道

| 优先级 | 通道 | 用法 |
|---|---|---|
| ① | `-PromptText` 参数 | `powershell -File ... -PromptText "..."` |
| ② | `LRC_PROMPT` 环境变量 | `$env:LRC_PROMPT = '...'` |
| ③ | stdin | Trae 默认走这条 |

**为什么 stdin 排最后**：若宿主把 stdin 当管道打开却不关闭，读 stdin 会一直阻塞。

## 六、文件编码（必须）

两个 `.ps1` **必须保存为 UTF-8 with BOM**。

PowerShell 5.1 读取无 BOM 的 `.ps1` 时按系统 ANSI（中文环境是 GBK）解码，
中文会变乱码并导致语法错误（实测报错：`字符串缺少终止符`）。

## 七、验证（2026-11-06 实测）

```powershell
# 1) 服务在线
curl.exe -s http://127.0.0.1:3099/health

# 2) SessionStart：应输出记忆索引
powershell -NoProfile -ExecutionPolicy Bypass -File .\.trae\hooks\lrc-session-start.ps1

# 3) UserPromptSubmit 命中关键词：索引 + 强制提示
'{"prompt":"上次那个检索接口是怎么改的"}' | powershell -NoProfile -ExecutionPolicy Bypass -File .\.trae\hooks\lrc-user-prompt-submit.ps1

# 4) UserPromptSubmit 不命中：只输出一行提醒
'{"prompt":"帮我写个快排"}' | powershell -NoProfile -ExecutionPolicy Bypass -File .\.trae\hooks\lrc-user-prompt-submit.ps1
```

实测结果（本机 Trae + 3099 真实库）：

| 用例 | 结果 |
|---|---|
| `hooks.json` 解析 | ✅ 事件 `SessionStart, UserPromptSubmit` |
| SessionStart 输出索引 | ✅ 8 条 + 回退提示 |
| UserPromptSubmit 命中 | ✅ 索引 + `【LRC 强制】…` |
| UserPromptSubmit 不命中 | ✅ 仅一行提醒 |

## 八、降级路径

| 情况 | 行为 |
|---|---|
| Trae 版本过旧不支持 Hook | 改用 `integrations/rules/trae-rules.md`（规则文件） |
| LRC 服务未启动 | 索引块为空，只输出一行提醒，不报错 |
| prompt 不是 JSON | 按纯文本处理，关键词判定照常工作 |
| 宿主不关闭 stdin | 用 `-PromptText` 或 `LRC_PROMPT` |

## 九、已知限制

- 关键词表是规则式的，会漏判（例如"我们当时为什么选这个"不含表内词）。
  漏判只影响"是否注入索引"，不影响规则提醒。
- 每轮注入一行提醒会占用少量 token。长会话里这是刻意的取舍：规则被稀释的代价更高。
- 索引内容用 `Invoke-WebRequest` + 显式 UTF-8 解码获取。
  直接用 `Invoke-RestMethod` 会因 PS 5.1 按 ISO-8859-1 解码而中文乱码（实测）。
