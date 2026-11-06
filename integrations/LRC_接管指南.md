# LRC 接管指南

> 面向使用者，不是开发者。照着做，10 分钟能让 AI 记住你的项目。
>
> 如果你只想快速跑通，直接跳到「三、Claude Code」或「四、DSH」。

---

## 一、这套东西解决什么问题

AI 编程助手有个通病：**换个会话就失忆**。

- 你上周定的"用 pnpm 不用 npm"，这周它又给你 `npm install`
- 你解释过三遍的架构，第四遍还得再讲
- 上下文一满，它自动压缩，把关键约束压没了

LRC 的做法是：**把结论存到本地，下次开局自动喂回去**。

它不改变 AI 怎么工作，只在三个时机插一脚：

| 时机 | 动作 |
|---|---|
| 会话开始 | 把最近的记忆摘要塞进上下文 |
| 你每次提问 | 提醒它"涉及历史先查记忆" |
| 上下文快满 | 把这段对话的结论提炼出来存好，再让它压缩 |

---

## 二、三个文件分别是什么

`integrations/` 下有三类东西，**按需装，不用全装**：

| 目录 | 是什么 | 装不装 |
|---|---|---|
| `rules/` | 规则文件（纯文本）。告诉 AI"必须先查记忆再搜索" | **建议装**，零风险 |
| `hooks/` | 钩子脚本。在会话开始/提问时自动注入记忆 | 装了效果最好 |
| `dsh-plugin/` | DSH 专用插件。压缩时自动迁移 | 只有用 DSH 才装 |

**先装规则，再装钩子。** 规则是纯文本，装错了删掉就行；钩子要改宿主配置，装完必须验证。

---

## 三、Claude Code

### 3.1 装规则（1 分钟）

把 `integrations/rules/AGENTS.md` 复制到你的项目根目录，改名为 `AGENTS.md`。

就这样。Claude Code 会自动读它。

### 3.2 装钩子（3 分钟）

```bash
# 1) 脚本放到 Claude Code 的 hooks 目录
mkdir -p ~/.claude/hooks
cp integrations/hooks/claude-code/pre-compact.sh  ~/.claude/hooks/lrc-pre-compact.sh
cp integrations/hooks/claude-code/session-start.sh ~/.claude/hooks/lrc-session-start.sh
chmod +x ~/.claude/hooks/lrc-*.sh

# 2) 合并配置（⚠ 是合并，不是覆盖）
#    把 integrations/hooks/claude-code/settings.json 里的 hooks 字段
#    合并进 ~/.claude/settings.json
```

### 3.3 验证

```bash
# 服务在线？
curl -s http://127.0.0.1:3099/health

# 索引能取到？
bash ~/.claude/hooks/lrc-session-start.sh
# 应输出：【LRC 记忆索引 · 项目 xxx · 最近 N 条】

# 迁移能跑通？（用自带的自动化测试，不需要真 Claude Code）
bash integrations/hooks/claude-code/tests/test-pre-compact.sh
# 应输出：通过 14 项，失败 0 项
```

最后在 Claude Code 里手动执行一次 `/compact`，压缩后的上下文应该出现 `[归档]` 开头的行。

---

## 四、DSH（DeepSeek Harness）

### 4.1 装插件

```bash
# 用 DSH 自带 CLI（路径按你的安装位置调整）
dsh plugin --profile web add <LRC 仓库路径>/integrations/dsh-plugin

# 装完必须重启 DSH
```

### 4.2 确认真的装上了

```bash
dsh --profile web --dump-config | grep dsh-plugin-lrc
```

**这一步不能省。** 如果 `package.json` 缺 `dsh.bundle.patch`，包会被当成普通 npm 依赖装上，
但**不会**进入插件树 —— 表现为"装成功了但没反应"。

### 4.3 启用压缩后端（可选，但推荐）

默认只装工具插件（模型可以主动调用 `lrc_migrate_context`）。
要让 `/compact` **自动**迁移，还要把压缩后端挂上：

1. 打开 `~/.dsh/profiles/web/cordis.patch.yml`
2. **删掉**原有的 `@deepseek-ai/dsh-compaction-basic` 那一行
   （同一时刻只能有一个压缩后端，两个都挂会冲突）
3. 把 `integrations/dsh-plugin/cordis.patch.yml` 末尾注释掉的那段取消注释，追加进去
4. 重启 DSH

### 4.4 验证

```bash
# 记下当前记忆数
curl -s http://127.0.0.1:3099/v1/memories/stats

# 在 DSH 里手动 /compact

# 记忆数应该增加
curl -s http://127.0.0.1:3099/v1/memories/stats
```

---

## 五、Trae

Trae 官方支持 Hook，配置在 `<项目>/.trae/hooks.json`。

```powershell
# 1) 脚本
New-Item -ItemType Directory -Force .\.trae\hooks | Out-Null
Copy-Item integrations\hooks\trae\session-start.ps1      .\.trae\hooks\lrc-session-start.ps1
Copy-Item integrations\hooks\trae\user-prompt-submit.ps1 .\.trae\hooks\lrc-user-prompt-submit.ps1

# 2) 配置
Copy-Item integrations\hooks\trae\hooks.json .\.trae\hooks.json

# 3) 重启 Trae（Hook 在会话启动时读取，当前会话不会热加载）
```

验证：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\.trae\hooks\lrc-session-start.ps1
```

新会话开局应自动出现 `【LRC 记忆索引 · 项目 xxx · 最近 N 条】`。

---

## 六、出问题怎么查

按这个顺序排查，90% 的问题在前三步。

| 现象 | 原因 | 怎么修 |
|---|---|---|
| 完全没反应 | LRC 服务没起 | `curl http://127.0.0.1:3099/health`，没响应就启动 LRC |
| 完全没反应 | 端口不对 | 正式环境是 **3099**，开发环境是 **3111**。检查 `LRC_BASE` |
| 完全没反应 | Hook 配置格式错 | 必须是 `{"hooks":{"事件":[{"hooks":[{...}]}]}}`，**中间那层 `hooks` 数组不能漏** |
| 中文乱码 / 语法报错 | `.ps1` 存成了 UTF-8 无 BOM | 用编辑器另存为「UTF-8 带 BOM」 |
| 提示 401 | 服务端设了 `LRC_API_TOKEN` | 设环境变量 `LRC_TOKEN` |
| 记忆数不增加 | 提炼没产出 | 对话里没有"决策/约束/结论"类句子，属正常 |
| 记忆数不增加 | 迁移被截断 | 看响应里的 `truncated` 和 `remaining`，分批提交 |
| DSH 装了没反应 | 没进插件树 | `dsh --profile web --dump-config \| grep dsh-plugin-lrc` |

### 一条命令看全貌

```bash
curl -s http://127.0.0.1:3099/health
curl -s http://127.0.0.1:3099/v1/memories/stats
curl -s "http://127.0.0.1:3099/v1/memories/recent?limit=5"
```

---

## 七、怎么卸载

| 装了什么 | 怎么删 |
|---|---|
| 规则文件 | 删掉项目根的 `AGENTS.md`（或 `.cursorrules` / `.trae/rules/*.md`） |
| Claude Code 钩子 | 删 `~/.claude/hooks/lrc-*.sh`，并从 `settings.json` 移除对应 hooks 条目 |
| Trae 钩子 | 删 `.trae/hooks.json` 和 `.trae/hooks/lrc-*.ps1` |
| DSH 插件 | `dsh plugin --profile web remove dsh-plugin-lrc`，并删掉 `cordis.patch.yml` 里的 insert 段 |

**已经存进 LRC 的记忆不会自动删除。** 要清就调 `forget` 或直接删数据目录。

---

## 八、几个你该知道的取舍

1. **精度优先，召回其次。** 提炼引擎宁可漏，也不把闲聊塞进记忆库。
   觉得漏得多，调低 `DistillConfig.threshold`（默认 2.0）。
2. **报错日志整块留存。** `panic:` / `Traceback` / `报错` 这类标记会触发整块保存，
   因为调用栈切碎了就没法回溯。
3. **压缩不接管 system prompt。** 这是宿主限制，LRC 只能接管对话历史部分。
4. **迁移有上限。** 单次最多 200 条、10 秒预算，超了返回 `truncated: true`，分批提交即可。

---

## 九、想自己验证一遍？

三个宿主都有可自动跑的测试，不需要真的开宿主：

```bash
# Claude Code 钩子（14 项）
bash integrations/hooks/claude-code/tests/test-pre-compact.sh

# DSH 插件客户端（11 项）
node integrations/dsh-plugin/scripts/smoke.ts
```

两个都应该是「全部通过」。
