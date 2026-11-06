# Trae 接入（P0-2）

一个 Hook：每次提交 prompt 前，把 LRC 规则重新推到模型注意力里（"稀释"），
命中"历史 / 偏好 / 决策"类关键词时额外注入记忆索引。

## 适用版本（先读这段）

Trae 各版本对 `hooks.json` 的支持不一致。**先确认你的版本是否支持 `UserPromptSubmit` 事件**：

- **支持**：按下面步骤装 Hook。
- **不支持**：直接用降级方案 —— 把 `integrations/rules/trae-rules.md` 的内容
  追加到项目的 `.trae/rules/` 目录。规则文件是 Trae 稳定支持的通道，
  约束力弱于 Hook，但零配置、零风险。

> 不要为了装 Hook 去改 Trae 安装目录或注入脚本，那会随版本升级失效。

## 安装（支持 Hook 的版本）

```powershell
# 1) 脚本放到 Trae 的 hooks 目录
New-Item -ItemType Directory -Force "$env:USERPROFILE\.trae\hooks" | Out-Null
Copy-Item .\user-prompt-submit.ps1 "$env:USERPROFILE\.trae\hooks\lrc-user-prompt-submit.ps1"

# 2) 合并 hooks.json 到 Trae 的 hooks 配置（合并，不要覆盖）
```

## 需要改的变量

| 变量 | 何时改 |
|---|---|
| `LRC_BASE` | 只在开发联调时改（`http://127.0.0.1:3111`）。**不要改脚本默认值** |
| `LRC_TOKEN` | 服务端设置了 `LRC_API_TOKEN` 时必须设 |
| `LRC_PROJECT` | 缺省取当前目录名 |
| `LRC_ALWAYS_REMIND` | 默认 `1`（每轮都注入一行提醒）。嫌费 token 就设 `0`，只在命中关键词时注入 |

设置方式（用户级环境变量，一次生效）：

```powershell
[Environment]::SetEnvironmentVariable('LRC_BASE', 'http://127.0.0.1:3099', 'User')
```

## prompt 的三条输入通道

不同宿主传 prompt 的方式不同，脚本按优先级依次尝试：

| 优先级 | 通道 | 用法 |
|---|---|---|
| ① | `-PromptText` 参数 | `powershell -File lrc-user-prompt-submit.ps1 -PromptText "..."` |
| ② | `LRC_PROMPT` 环境变量 | `$env:LRC_PROMPT = '...'` |
| ③ | stdin | `'...' \| powershell -File ...` |

**为什么 stdin 排最后**：若宿主把 stdin 当管道打开却不关闭，读 stdin 会一直阻塞，
Hook 就卡死了。能传参或传环境变量时，优先用前两条。

## 文件编码（必须）

`user-prompt-submit.ps1` **必须保存为 UTF-8 with BOM**。

PowerShell 5.1 读取无 BOM 的 `.ps1` 时按系统 ANSI（中文环境是 GBK）解码，
中文会变乱码并导致语法错误（实测报错：`字符串缺少终止符`）。
用编辑器另存为「UTF-8 带 BOM」即可；不要存成 UTF-8 无 BOM。

## 验证

```powershell
# 1) 服务在线
curl.exe -s http://127.0.0.1:3099/health

# 2) 命中关键词：应输出索引 + 强制提示
powershell -NoProfile -File .\user-prompt-submit.ps1 -PromptText '上次那个检索接口是怎么改的'

# 3) 不命中关键词：只输出一行提醒
powershell -NoProfile -File .\user-prompt-submit.ps1 -PromptText '帮我写个快排'

# 4) 服务未启动：应静默降级（不报错、不输出索引）
$env:LRC_BASE='http://127.0.0.1:3999'
powershell -NoProfile -File .\user-prompt-submit.ps1 -PromptText '上次怎么改的'
```

## 降级路径

| 情况 | 行为 |
|---|---|
| Trae 版本不支持 `UserPromptSubmit` | 改用 `integrations/rules/trae-rules.md` |
| LRC 服务未启动 | 索引块为空，只输出一行提醒，不报错 |
| prompt 不是 JSON | 按纯文本处理，关键词判定照常工作 |
| 宿主不关闭 stdin | 用 `-PromptText` 或 `LRC_PROMPT`，不要走 stdin |

## 已知限制

- 关键词表是规则式的，会漏判（例如"我们当时为什么选这个"不含表内词）。
  漏判只影响"是否注入索引"，不影响规则提醒。
- 每轮注入一行提醒会占用少量 token。长会话里这是刻意的取舍：规则被稀释的代价更高。
- 索引内容用 `Invoke-WebRequest` + 显式 UTF-8 解码获取。
  直接用 `Invoke-RestMethod` 会因 PS 5.1 按 ISO-8859-1 解码而中文乱码（实测）。
