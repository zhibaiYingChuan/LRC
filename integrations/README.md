# LRC 宿主接入模板

把 LRC 从"一个可选的记忆工具"变成"上下文中间件"的接入材料。分三类：

| 目录 | 作用 | 对应任务 |
|---|---|---|
| `hooks/` | 各宿主的生命周期钩子：会话启动注入索引、上下文压缩前迁移记忆 | P0-2 |
| `rules/` | 各宿主的全局规则文件：强制模型先 `recall` 再搜索 | P1-2 |
| `dsh-plugin/` | DeepSeek Harness 插件：**已独立成仓**，见 https://github.com/zhibaiYingChuan/dsh-plugin-lrc（npm `dsh-plugin-lrc`） | P2-1 |

## 端口纪律（强制）

| 环境 | 端口 | 数据目录 | 用途 |
|---|---|---|---|
| 本地正式 | **3099** | `~/.loong-recall/global/data` | 用户日常使用 |
| 开发 / 测试 | **3111** | 隔离目录（如 `G:\tmp\lrc-dev-3111\data`） | 联调、Hook 实测 |

**所有模板默认写 3099。** 开发联调时用环境变量覆盖，不要改模板文件：

```bash
export LRC_BASE=http://127.0.0.1:3111
```

> 违反后果：测试写入会污染真实记忆库，且 Hook 可能把测试数据迁移进生产库。

> ⚠ **实测踩到的坑**：用 `--port 3111` 启动开发实例时，服务端会把
> `default_port: 3111` 写进全局配置 `%APPDATA%\LoongRecall\config.json`，
> 桌面端下次启动就会跟着用 3111。**开发实例跑完必须把该文件改回 3099。**

## 通用环境变量

所有脚本都读同一组变量，缺省值即"本地正式环境"：

| 变量 | 缺省 | 说明 |
|---|---|---|
| `LRC_BASE` | `http://127.0.0.1:3099` | LRC 服务地址 |
| `LRC_TOKEN` | 空 | 服务端设置了 `LRC_API_TOKEN` 时必填，脚本自动加 `Authorization: Bearer` |
| `LRC_PROJECT` | 当前目录名 | 记忆归属项目 |
| `LRC_MAX_ITEMS` | `40` | 单次迁移的最大条数 |

## 依赖

- `curl`：所有平台自带或易装。
- `jq`：`pre-compact.sh` / `pre-tool-use.sh` 用它拼 JSON。Windows 用 `winget install jqlang.jq`，macOS 用 `brew install jq`。
- PowerShell 5.1+：仅 Trae 模板需要。

## 安装顺序（建议）

1. **先装规则**（`rules/`）：纯文本，零风险，立刻让模型"知道要用 LRC"。
2. **再装 Hook**（`hooks/`）：需要改宿主配置，装完必须按各目录 README 的验证步骤实测。

## 降级说明（重要）

`hooks/` 里的迁移脚本优先调用 `/v1/context/distill`（提炼接口，P1-1 提供）。
**该接口尚未实现**，因此脚本内置了降级路径：接口不可用时，改为把对话尾部原文
作为一条 `code_context` 记忆写入（`tags` 含 `archived`），保证"只迁移不提炼"也能用。

P1-1 上线后无需改脚本 —— 脚本探测到 `items` 字段就自动走提炼路径。

## 验证清单

| 检查项 | 命令 |
|---|---|
| 服务在线 | `curl -s http://127.0.0.1:3099/health` |
| 记忆数 | `curl -s http://127.0.0.1:3099/v1/memories/stats` |
| 最近记忆 | `curl -s "http://127.0.0.1:3099/v1/memories/recent?limit=5"` |
| 索引注入 | `bash hooks/claude-code/session-start.sh`（应打印一段索引文本） |
