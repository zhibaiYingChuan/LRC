# LRC · 忆（Loong Recall）

**一个装在自己电脑上的「记忆库 + 代码搜索」，让你的 AI 工具真的记住你。**

[![License: 外壳与前端](https://img.shields.io/badge/%E5%A4%96%E5%A3%B3%E4%B8%8E%E5%89%8D%E7%AB%AF-Apache%202.0-blue.svg)](LICENSE_CODE)
[![License: 核心引擎](https://img.shields.io/badge/%E6%A0%B8%E5%BF%83%E5%BC%95%E6%93%8E-%E4%B8%93%E6%9C%89%E8%AE%B8%E5%8F%AF-red.svg)](LICENSE)

LRC 把你说过的话、做过的决定、写过的代码存成本地记忆。下次你（或你的 AI）问起时，
它能把相关的那几条翻出来，并告诉你「为什么是这几条」。

所有数据都在你自己的电脑上；除了首次下载语义模型，核心功能不需要联网。

---

## 它解决什么问题

- **换个会话就忘**：昨天定好的方案，今天要重新讲一遍。
- **找不到旧代码**：只记得「我好像写过类似的东西」，不知道在哪个项目。
- **知识太散**：笔记、聊天记录、代码注释里各有一点，拼不起来。
- **AI 不了解你**：每次都要重新解释项目背景、技术选型和历史决策。

AI 本身不会跨会话记住这些；LRC 就是给它补上这块记忆。

---

## 30 秒上手

到 [v0.11.1 Releases](https://github.com/zhibaiYingChuan/LRC/releases/tag/v0.11.1)
下载对应平台的桌面安装包：

| 平台 | 安装包 |
|---|---|
| Windows | `lrc-desktop-v0.11.1-windows-x86_64-setup.exe` |
| macOS（Apple 芯片） | `lrc-desktop-v0.11.1-macos-arm64.dmg` |
| Linux | `lrc-desktop-v0.11.1-linux-amd64.deb` 或 `lrc-desktop-v0.11.1-linux-x86_64.AppImage` |

安装后打开 LRC Desktop：

1. 首次启动会自动下载**本地语义模型**（约 100~400MB，默认走国内镜像），下载完即可使用。
2. 在「仪表盘」看记忆概览；在「设置」配置模型与 AI 工具接入。
3. 在「信任中心」备份、导出、导入你的记忆库。

> **安装包未做商业代码签名**：macOS 首次打开若被 Gatekeeper 拦截，
> 请右键点击应用 →「打开」；Windows SmartScreen 提示时选择「更多信息 → 仍要运行」。

---

## 三种用法

### 1）桌面端（推荐给普通用户）

图形界面包含：记忆列表与检索、联想探索、演化时间线、系统健康评分、
记忆库维护（重新编码 / 归档过期 / 导出备份）、设置与信任中心。

### 2）命令行 `lrc`（推荐给开发者和 AI 工具）

`lrc` 是一个**纯客户端**：它不加载模型，只通过 HTTP 调用常驻的 sidecar。
把下面两个文件下载到**同一个目录**即可使用：

- CLI：`lrc-cli-v0.11.1-windows-x86_64.exe` / `lrc-cli-v0.11.1-macos-arm64` / `lrc-cli-v0.11.1-linux-x86_64`
- Sidecar（稳定别名）：`lrc-sidecar-windows-x64.exe` / `lrc-sidecar-darwin-arm64` / `lrc-sidecar-linux-x64`

CLI 会在同目录自动识别这些稳定别名；若你下载的是带版本号的 sidecar
（`lrc-v0.11.1-*`），把它重命名为 `lrc-sidecar`（Windows 为 `lrc-sidecar.exe`）即可。

```bash
lrc doctor                      # 检查 daemon、版本、模型与数据目录
lrc remember "项目统一使用 pnpm，不用 npm"
lrc recall "包管理器偏好"
lrc list --limit 20
lrc search "MemoryStore"
```

常用命令：

| 命令 | 作用 |
|---|---|
| `lrc recall <查询>` | 检索记忆（默认三路融合） |
| `lrc remember <内容>` / `-` | 写入一条记忆（`-` 从 stdin 读取） |
| `lrc list` / `lrc stats` | 列出记忆 / 查看统计；`--duplicates` 输出近重复报告 |
| `lrc search <关键词>` | 搜索已索引的代码片段 |
| `lrc forget <id> --yes` | 删除一条记忆（不可撤销） |
| `lrc correct <id> <新内容>` | 修正一条记忆（保留历史版本） |
| `lrc associations <id>` | 查看一条记忆的关联 |
| `lrc distill [文本\|--transcript 文件]` | 从对话提炼候选记忆（不写库） |
| `lrc absorb [文本\|-]` | 任务结束自动沉淀：过滤噪声、去重、写库 |
| `lrc migrate < - \| --file 文件>` | 把提炼结果批量写入（幂等） |
| `lrc backup [--mode full\|core]` | 为当前实例创建记忆快照（默认 full） |
| `lrc reclassify` | 全库重编码（`--progress` 看进度 / `--cancel` 取消 / `--resume` 续跑） |
| `lrc archive [--auto-summary]` | 归档已过期记忆；`--auto-summary` 按标签批量归档 |
| `lrc unarchive <id> [--ttl-days N]` | 从归档恢复（从现在重新计时） |
| `lrc purge-archived <id> --yes` | 从归档彻底删除（不可恢复） |
| `lrc doctor [--capture [N]]` | 自检 daemon / 模型 / 数据目录；`--capture` 附带最近沉淀结果 |
| `lrc daemon start\|stop\|status` | 管理常驻 sidecar |
| `lrc mcp` | 把 stdio MCP 请求桥接到本地 sidecar |

约定：

- 所有命令支持 `--json`（稳定 schema）与 `--base <URL>`（默认自动探测 3099–3118）。
- 退出码：`0` 成功、`1` 一般错误、`2` 参数错误、`3` daemon 不可用、`4` 超时、`5` 无结果。
- `forget` / `purge-archived` 等破坏性命令必须显式加 `--yes`，且不会隐式拉起新实例。

### 3）接入 AI 工具（MCP）

LRC 的 sidecar 暴露标准 MCP 端点：

```
http://127.0.0.1:3099/mcp
```

把该地址配置到支持 MCP 的 AI 工具中，即可使用以下 16 个工具：

| 工具 | 用途 |
|---|---|
| `remember` / `batch_remember` | 写入单条 / 批量记忆 |
| `absorb` | 任务结束自动沉淀（宿主钩子直接调用，自动过滤与去重） |
| `recall` / `recall_enhanced` | 检索记忆（标准 / 增强） |
| `list_memories` | 分页列出与过滤 |
| `update_memory` / `correct_memory` | 更新 / 修正 |
| `forget` | 删除 |
| `associations` / `association_graph` | 查看关联 / 多跳联想图 |
| `search_code` / `codebase_stats` | 代码片段检索 / 索引统计 |
| `memory_stats` / `system_health` | 记忆统计 / 系统健康 |
| `archive` | 归档过期记忆 |

多租户部署（设置了 `LRC_API_TOKEN`）时，所有工具都需要同时提供 `user_id`；
提供 `session_id` 时也必须提供 `user_id`。本地可信域（未设置 token）无需这些参数。

---

## 核心功能

### 记忆

- **多种类型**：事实、偏好、决策、代码上下文、对话、经历。
- **共同经历**：用 `event_id` 把同一次经历产生的多条记忆串起来，即使内容不相似也能互相关联。
- **实体**：记录人、地、时、物，作为关联与提问的锚点。
- **过期与归档**：可为记忆设置存活天数；过期后默认不再出现在检索与列表中，
  可一键归档到冷存储；归档可恢复（从现在重新计时），也可彻底删除。

### 记忆联想

LRC 的「联想」不是内容相似度，而是**记录下来的关系**——两条记忆为什么相关，
是有依据可查的，不是猜的。

| 关系类型 | 含义 |
|---|---|
| `same_event` | 同一次经历产生（知情者断言，证据最强） |
| `same_event_auto` | 系统按会话自动推断的同经历关系（与断言分开标注） |
| `shared_entity` | 共享人 / 地 / 时 / 物等实体 |
| `derived_from` | 派生来源（合成记忆 → 原始记忆） |
| `same_artifact` | 指向同一代码文件 / 提交等产物 |

- **内容不相似也能关联**：一次出行里「游西湖」与「吃楼外楼」语义毫不相关，
  但同属一个 `event_id`，因此能互相联想。
- **多跳扩散**：从一条记忆或一个查询出发做有界多跳探索，返回路径与「为什么相关」。
- **入口**：桌面端「联想探索」、`lrc associations <id>`、MCP
  `associations` / `association_graph`、REST `/v1/associations/explore`。

### 上下文沉淀

把「这一轮任务得出的结论」自动存进记忆库，不必手动整理：

- **三种接法**：MCP `absorb` 工具、`lrc absorb`、宿主钩子（Trae / Codex / Cursor / Claude Code，
  见 [integrations/](integrations/)）。
- **自动降噪**：过滤 Markdown 表格 / 标题 / 纯链接与闲聊，只留决策、约束、结论、报错根因。
- **幂等去重**：同内容重复沉淀走合并，不会膨胀；无有效内容时直接跳过（NoOp 不写库）。
- **可回看**：`lrc doctor --capture` 查看最近几次沉淀的结果，确认钩子是否真的在工作。

与 `distill` / `migrate` 的分工：`distill` 只提炼、不写库；`absorb` 提炼 + 过滤 + 写库一步到位；
`migrate` 用于把已有提炼结果批量幂等导入。

### 检索

- **三路融合**：关键词快路 + 深度语义路 + 语义向量路，按结果融合排序。
- **代码搜索**：对项目代码建立片段索引，按关键词检索文件、函数与行号。
- **可解释**：检索结果带通路来源与分数分解，便于判断「为什么是这几条」。

### 健康与维护

仪表盘的「系统健康评分」是**综合健康**（道同构度、八卦均衡、洛书偏离、
合成占比与调节器耦合度的加权结果）。它反映的是记忆结构，**不代表记忆内容损坏**。

- **洛书偏差**：9 维坐标偏离幻和约束的程度，越低越好。
- **重新编码**：用当前编码器重算全部记忆的坐标与八卦。
  **只有在你更换过嵌入模型或编码器版本后才会改变结果**；编码器没变时重算不会提升评分。
  任务可取消、可续跑，进度会落盘，重启后 `--resume` 从断点继续。
- **近重复报告**：`lrc list --duplicates`（REST `GET /v1/memories/duplicates`）
  按 MinHash-LSH 找出内容高度重叠的记忆组，先看报告再决定合并或归档。
- **批量归档**：`lrc archive --auto-summary`（REST `POST /v1/memories/archive-by-tag`）
  按标签 / 天数 / 条数批量归档，支持 `--dry-run` 先预览；归档可恢复。
- **归档过期记忆**：把过期记忆移出检索范围（可恢复），不会删除原始内容。
- **导出备份**：把记忆、代码片段与归档一起导出为 JSON，建议在批量维护前先备份。

### 隐私与隔离

- **本地优先**：记忆、索引、审计日志都在本地；不上传任何数据。
- **scope 隔离**：记忆支持 Global / User / Session 三级可见性；
  多租户部署由 `LRC_API_TOKEN` 启用，租户由凭据派生，客户端自报的 `user_id` 会被加上租户前缀。
- **管理员操作**：备份、恢复、归档、全库统计、审计追踪等运维能力在多租户域仅限本地可信域。
- **如实披露**：当前是单 token 部署，同一 token 内的用户隔离依赖客户端诚实自报；
  真正的 per-token 身份属后续架构工作。

### 备份、导出与迁移

- **备份**：本地快照（保留最近 4 份），包含基文件、追加日志、归档、审计与反馈。
- **导出**：JSON 文件（记忆 + 代码片段 + 归档），可跨机器保存。
- **导入**：追加到现有库，保留隐私级别、归属、事件、实体与原始时间。
- **迁移**：扫描历史数据目录，按 id 去重合并到当前实例。

---

## 数据存在哪里

默认位置：`~/.loong-recall/`（Windows 为 `C:\Users\<你>\.loong-recall\`）。

| 文件 | 内容 |
|---|---|
| `memories.json` + `memories.journal.jsonl` | 记忆基文件与追加日志 |
| `archive.json` | 归档记忆 |
| `chunks.json` | 代码片段索引 |
| `audit.jsonl` | 审计日志（哈希链） |
| `feedback.jsonl` | 反馈记录 |
| `backups/` | 本地备份快照（最多 4 份） |

自定义数据目录：桌面端开发模式或 `code-memory-server --data-dir <路径>`。

---

## 常见问题

**模型下载很慢或失败？**
桌面端默认走国内镜像（`hf-mirror.com`）；也可以手动下载模型后放进
`~/.loong-recall/models/`。详见 [离线模型指南](docs/OFFLINE_MODEL_GUIDE.md)。

**安装包打不开 / 提示未知开发者？**
安装包未做商业签名。macOS 右键 →「打开」；Windows SmartScreen →「更多信息 → 仍要运行」。

**健康评分不高，需要处理吗？**
评分是结构指标，不是数据损坏。低于 50 分时再看维护建议：先备份，
再归档过期记忆；只有换过模型时才需要重新编码。

**重新编码后评分没变？**
这是正常的：编码器未变时，重算会得到相同的坐标与偏差。
重新编码只解决「模型/编码器变更后的旧向量」问题。

**MCP 连不上？**
确认 sidecar 正在运行（`lrc doctor` 或桌面端已启动），地址为
`http://127.0.0.1:3099/mcp`；开发模式端口可能是 3111。

**端口被占用？**
桌面端默认 3099，开发模式 3111；CLI 会在 3099–3118 内自动探测已运行实例。

---

## 当前边界（如实说明）

- 语义模型首次需要联网下载；下载失败会自动回退到统计算法，基础检索仍可用。
- macOS/Windows 安装包未签名，首次打开需要手动放行。
- 多租户隔离目前是「单 token + 客户端自报 user」，不是 per-token 身份。
- 归档恢复不会重建历史关联边；关联会在后续构建时重新推导。
- 全库语义向量缓存不会在归档时立即裁剪（不影响检索正确性，只占磁盘）。

---

## 文档

- [用户指南](docs/USER_GUIDE.md)
- [离线模型指南](docs/OFFLINE_MODEL_GUIDE.md)
- [使用场景](docs/USE_CASES.md)
- [基准数据（v0.10.3）](docs/BENCHMARK_v0.10.3.md)
- [更新日志](CHANGELOG.md)

---

## 许可证

- 外壳与前端：[Apache 2.0](LICENSE_CODE)
- 核心引擎：[DaoTi Research License](LICENSE)
