# HCSE 发布前检查清单（LRC）

> 本文件按 HCSE 通用框架「智能体调用后的处置规则」第 2 条回写，收录**本轮全维度代码审查未能预警、或属现有检测范围之外**的发布期故障模式。
> 来源：`docs/GLOBAL_CODE_REVIEW_REPORT.md` 第五节「HCSE 回写建议」第 3、4 项。
> 适用对象：任何修改跨层调用（前端 → HTTP/IPC）、工作区目录结构、构建/开发脚本的变更。

---

## 一、检查项 1：跨层契约校验

**针对的故障模式**：前端与后端分属不同语言、不同进程，**契约断裂不会导致任一侧编译失败**——前端请求 `/v1/xxx` 而后端未注册该路由，或前端 invoke 一个未注册的 Tauri 命令，都只会在运行时静默 404 / reject。本轮审查发现 `validate_frontend_contract.js` 此前仅校验 DOM id 与静态资源，**不校验跨层调用面**。

### 1.1 现状基线（已落盘，变更时须回归）

| 校验对象 | 解析方式 | 实测值 |
|---|---|---|
| 前端 API 调用路径 | `collectCallSites('fetchWithTimeout', ...)` + `maskCode` 遮蔽注释/字符串 | 36 条，与 `src/v1_api.rs` / `src/server.rs` 路由表对齐 |
| 前端 Tauri invoke 命令名 | `collectCallSites('invokeWithTimeout', ...)` | 33 条，与 `desktop/src-tauri/src/main.rs` 的 `generate_handler!` 对齐 |
| **后端孤儿命令（注册但前端零调用）** | `registeredCommands` 差集 | **4 条**，须在白名单显式申报（v0.9.7 新增，见 1.2 第 6 条） |
| 导航项 / DOM id / 静态资源 | 原有校验 | 6 个导航、259 个 DOM id |

**脚本**：[validate_frontend_contract.js](file:///g:/code-memory/scripts/validate_frontend_contract.js)（三工具函数 `parseRoutes` / `maskCode` / `collectCallSites`）
**门禁位置**：CI [ci.yml:657-662](file:///g:/code-memory/.github/workflows/ci.yml#L657-L662)、Release [release.yml:124-130](file:///g:/code-memory/.github/workflows/release.yml#L124-L130)

### 1.2 强制检查步骤

1. **新增/改名路由后必须双向核对**：后端 `.route("/xxx", ...)` 与前端调用点**同名同数**。路由经 [server.rs:3741](file:///g:/code-memory/src/server.rs#L3741) `.nest_service("/v1", ...)` 挂载，故 `src/v1_api.rs` 内写 `/xxx`（**无** `/v1` 前缀），前端写 `/v1/xxx`——**此差异是历史高发错误点**，命名空间前缀不得在任一侧重复或遗漏。
2. **新增/改名 Tauri 命令后必须同步三处**：`#[tauri::command]` 定义、`generate_handler![...]` 注册、前端 `invokeWithTimeout('cmd_name', ...)`。缺任一环 → 运行时 reject。
3. **门禁口径必须一致**：CI 与 Release 的 `node --check` 清单**已统一为 6 个脚本**（v0.9.7 b5 修复）；新增前端脚本时须**同时**登记到两处，否则出现"CI 绿、发版红"或反向漏检。
4. **语法门禁覆盖所有产出脚本**：`node --check` 清单须覆盖 `static/` 与 `tests/frontend/` 下**全部** `.js`（`association-user-view-check.js` 已于 b5 纳入）。
5. **禁止裸 `fetch` / 裸 `invoke`**：所有跨层调用必须走 `fetchWithTimeout` / `invokeWithTimeout`（[app.js:342](file:///g:/code-memory/static/app.js#L342)、[:4875](file:///g:/code-memory/static/app.js#L4875)），否则既失去超时保护，也**逃逸契约脚本的解析范围**（成为门禁盲区）。
6. **后端孤儿命令必须显式申报**（v0.9.7 b12 新增）：若某 `#[tauri::command]` 已注册但前端零 invoke 调用，**须在 `validate_frontend_contract.js` 的 `ORPHAN_ALLOWLIST` 中说明保留理由**，否则门禁失败。该类命令可能是①桌面端内部直接调用的入口、②对外 IPC 契约、③真正的死代码——无论哪种都须显式声明。白名单**双向校验**：未申报的失败，已不再是例外却仍留的也失败（防理由腐化）。

### 1.3 已知未闭环缺口

- ~~`tests/frontend/association-user-view-check.js` 未接入 CI `node --check`~~ → **已闭环（b5）**：CI/Release 清单统一为 6 个脚本。
- ~~CI 与 Release 的前端语法门禁清单不一致~~ → **已闭环（b5）**。
- **当前唯一缺口**：`generate_handler!` 中新增命令若**既未被前端调用、又未在白名单申报**，门禁会失败——这是**有意设计**（迫使显式决策），非遗漏。

---

## 二、检查项 2：工作区物理残留扫描（含 ReparsePoint 显式判定）

**针对的故障模式**：`.gitignore` 覆盖**不等于**磁盘不存在。本轮审查中，用 `Get-ChildItem -Recurse -File` 扫描工作区时**静默跳过了 Junction**，导致子代理把链接计数误读为"61 个重复 `.rs` 文件"，产出了**虚构风险**。该故障模式属现有检测范围之外，必须显式回写。

### 2.1 现状基线（实测，2026-09-12）

| 事实 | 实测值 |
|---|---|
| `competition/` 递归条目总数（含链接） | 68 |
| 其中 ReparsePoint 条目 | **2**（均为 Junction） |
| `competition/rust-src/src` | → `G:\code-memory\src` |
| `competition/rust-src/static` | → `G:\code-memory\static` |
| Git 跟踪状态 | `competition/` 整体被 [.gitignore:238](file:///g:/code-memory/.gitignore#L238) 忽略 |

**结论**：`competition/rust-src/` 下的 `.rs` 与静态资源**不是副本**，而是指向主源码的链接。任何"重复文件数量"统计若不显式排除 ReparsePoint，结论必然失真。

### 2.2 强制检查步骤

1. **扫描命令必须显式判定 ReparsePoint**（PS 5.1 合规写法）：

```powershell
# 显式判定 ReparsePoint/Junction：-Recurse -File 会静默跳过链接，不可用于残留统计
$repoRoot = 'g:\code-memory'
$entries = Get-ChildItem -LiteralPath $repoRoot -Recurse -Force -ErrorAction SilentlyContinue
$links = $entries | Where-Object -FilterScript {
    ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0
}
Write-Host ("条目总数={0}  链接数={1}" -f $entries.Count, $links.Count)
$links | ForEach-Object -Process { Write-Host ("  [LINK] {0} -> {1}" -f $_.FullName, ($_.Target -join '; ')) }
```

   - PS 5.1 下 `$_.Target` 为**字符串数组**，须 `-join` 展开，否则打印 `System.String[]` 造成误读。
2. **区分三类状态**：`已跟踪` / `未跟踪且被忽略` / `未跟踪且未忽略`。判定须用 `git ls-files --error-unmatch` 与 `git check-ignore -q` 分别取证，**不可用 `git status` 代替**（被忽略项根本不出现在 status 中）。
3. **Junction 不得计数为"残留副本"**：发布前清理扫描中，Junction 应单独归类为"结构链接"并**核对目标路径是否存在**（目标缺失会形成悬挂链接）。
4. **确认 Junction 不参与打包**：构建产物（crate 打包 / Tauri bundle）若沿链接进入主源码，会造成同一文件被重复打包或泄露 `engine/` 内容，须在打包前确认排除。

### 2.3 本次取证暴露的结构性事实

`competition/` 通过 Junction 复用主仓源码，其"自包含"程度是**假象**——脱离主仓后 `competition/rust-src/` 为空壳。该目录去留需明确决策（见报告 8.6）。

---

## 三、检查项 3：本地开发链路可交付性

**针对的故障模式**：**开发者在本地"能用"，不等于新克隆者能用**。本轮审查发现本地开发链路的关键脚本**物理存在但被 `.gitignore` 忽略**，克隆后无法复现开发环境，而 CI 只跑构建与测试，**不校验本地链路完整性**，故该断裂无任何门禁覆盖。

### 3.1 现状基线（实测，2026-09-12）

| 文件 | 物理存在 | 已被 Git 跟踪 | 被 .gitignore 忽略 |
|---|---|---|---|
| `scripts/dev-proxy.py` | 是 | **否（待提交）** | **原被 [:303](file:///g:/code-memory/.gitignore#L298-L305) 忽略 → 已解除** |
| `scripts/run-dev.ps1` | 是 | **否（待提交）** | **原被 [:304](file:///g:/code-memory/.gitignore#L298-L305) 忽略 → 已解除** |
| `scripts/run-dev.bat` | 是 | **否（待提交）** | **原被 [:305](file:///g:/code-memory/.gitignore#L298-L305) 忽略 → 已解除** |
| `scripts/cdp-exec.ps1` | 否（规则残留） | 否 | 是（[:301](file:///g:/code-memory/.gitignore#L298-L305)） |
| `scripts/check_algorithm_leak.py` | 是 | 是 | 否 |
| `.githooks/pre-commit` | 是 | **否（新文件待提交）** | 否 |
| `scripts/enable_git_hooks.ps1` | 是 | **否（新文件待提交）** | 否 |
| `docs/HCSE_RESILIENCE_AUDIT.md` | 是 | 否（待提交） | **原被 [:318](file:///g:/code-memory/.gitignore#L312-L319) 忽略 → 已解除** |
| `docs/HCSE_RELEASE_PROTOCOL.md` | 是 | 否（待提交） | **原被 [:339](file:///g:/code-memory/.gitignore#L337-L342) 忽略 → 已解除** |

> **本检查项的自指事故（已修复，2026-09-12）**：r6/r7 落盘的两份 HCSE 回写清单**自身**被 `.gitignore:317/:318/:339` 忽略——即"回写检查清单"的动作产出了**不可交付**的文档。`git blame` 证实该三条规则由 `local-dev` 于 2026-08-17 提交，注释为"v0.9.1 发布清理：开发/发布流程文档（**不交付用户，本地开发自用**）"，**属刻意排除而非疏漏**。已在 [.gitignore:418-426](file:///g:/code-memory/.gitignore#L418-L426) 追加负向规则（`!docs/HCSE_RELEASE_PROTOCOL.md` / `!docs/HCSE_RESILIENCE_AUDIT.md`）解除忽略。
>
> **复检方法警示**：解除后 `git check-ignore -v` 对两文档返回 **EXIT=0**（命中**负向规则**），故**不能凭退出码判定忽略与否**——必须检视打印出的规则是否以 `!` 开头。权威判据应改用 `git ls-files --others --exclude-standard`（列出即"未忽略"）。实测：两文档已出现在该列表中，`docs/` 下被忽略条目 **116 → 114**。
>
> **边界**：Cargo 打包走 `Cargo.toml` 的 `include`/`exclude`，与本文件无关；本例外仅使两文档**可提交**，不会使其进入 crate / Tauri 发布产物。历史归档 `docs/HCSE_RESILIENCE_AUDIT_REPORT_L6.md`、`docs/hcse_resilience_v0.8.42.md` 仍被 `:318` 通配忽略（有意为之）。**注意**：解除忽略 ≠ 已交付，两文档仍为**未跟踪状态**，须 `git add` 后方可随克隆交付。

**开发链路脚本解除忽略（2026-09-12，用户裁定）**：三脚本原被 [:303-305](file:///g:/code-memory/.gitignore#L298-L305) 忽略（`local-dev` 于 v0.8.45 归入"本地开发运行目录（非交付内容）"），但**已被 3 个【已跟踪】测试文件引用**——按本条检查步骤 1「引用即契约」，被忽略即等同于克隆后必然失效。已在 [.gitignore:427-441](file:///g:/code-memory/.gitignore#L427-L441) 追加负向规则解除忽略（负向规则本体位于 [:439-441](file:///g:/code-memory/.gitignore#L439-L441)）。

| 引用方（均已跟踪） | 引用方式 | 降级能力（修复前 → 修复后） |
|---|---|---|
| [cdp-regression.js](file:///g:/code-memory/tests/frontend/cdp-regression.js#L734-L762) | probe 1420，可用则导航，否则原地重载 | 已有 → 保持 |
| [association-dashboard-cdp.js](file:///g:/code-memory/tests/frontend/association-dashboard-cdp.js#L108-L113) | probe 1420，可用则导航，否则保留现状 | 已有 → 保持 |
| [association-desktop-cdp.js](file:///g:/code-memory/tests/frontend/association-desktop-cdp.js#L164-L181) | **无条件**导航到 1420 | **无（硬失败）→ 已补 probe 降级** |

**实测验证（真实运行，2026-09-12）**：

| 验证项 | 实测 |
|---|---|
| 解忽略复检 | 三脚本 `check-ignore -v` 命中 **`!` 负向规则**（[:439/:440/:441](file:///g:/code-memory/.gitignore#L438-L441)）；`ls-files --others --exclude-standard` 均列出 |
| `python scripts/dev-proxy.py 1420 --dev` 实跑 | 端口 1420 `LISTENING`；`GET /` 返回 **200 / 130170 字节**（等于磁盘 `index.html` 长度，含 `value-hero`） |
| 缓存头 | `Cache-Control: no-store, no-cache, must-revalidate`（解决 WebView2 启发式缓存的既有设计意图生效） |
| dev 模式端口改写 | `meta lrc-sidecar-port` → **3111**（dev 与稳定版 3099 隔离） |
| API 代理 | `GET /v1/memories/stats` → 502（预期：3111 无 sidecar；本机已安装实例在 3099），证明代理路径可达 |
| 语法门禁 | `node --check association-desktop-cdp.js` → EXIT=0 |
| 文档落地 | [README.md](file:///g:/code-memory/README.md#L36-L49) 新增「本地开发链路」段（含启动命令与降级说明） |

**边界**：三脚本无硬编码绝对路径（分别用相对路径 / `Get-Location` / `%~dp0..`），无 token/secret；Cargo 打包走 `Cargo.toml` 的 `include`/`exclude`，本例外不使其进入发布产物。

**风险**：引用上述被忽略脚本的文档/代码（如 D-6 涉及的 dev server 配置）在**新克隆环境必然失效**。D-6 已裁定改为"使用 Tauri 内置 dev server"，移除了该依赖；开发链路脚本的交付性已于本轮闭环（见上）。

### 3.2 强制检查步骤

1. **文档引用即契约**：README / USER_GUIDE / 配置文件中引用的任何脚本路径，必须是**已跟踪**文件。引用被 `.gitignore` 覆盖的路径 = 缺陷。
2. **删除被忽略脚本前先改引用**：修改配置（如 `tauri.conf.json` 的 `beforeDevCommand`）时必须同步确认被移除引用的文件不再被文档提及。
3. **新增开发脚本禁止落入忽略规则**：`.gitignore` 中的 `scripts/cdp_*.js`、`scripts/run-dev.*` 等通配规则会**静默吞掉**新脚本，新增后须用 `git ls-files --others --exclude-standard -- <path>` 验证是否可达版本控制（**勿用 `check-ignore` 退出码判定**，负向规则命中时它同样返回 0）。
4. **钩子类文件必须随克隆生效**：`.githooks/` 依赖 `core.hooksPath` 本地配置，克隆后需执行 [enable_git_hooks.ps1](file:///g:/code-memory/scripts/enable_git_hooks.ps1)（README「质量与验证」已补启用说明）；发布前须确认该两个文件**已提交**（当前均为未跟踪状态）。
5. **新增被引用脚本必须同步交付**：任何被【已跟踪】文件（测试/文档/配置）引用的脚本，必须在同一次变更中解除忽略并 `git add`——否则克隆环境必然失效，且该断裂**无任何门禁覆盖**（CI 只跑构建与测试，不校验本地链路完整性）。

### 3.3 已知未闭环缺口

- `.githooks/pre-commit` 与 `scripts/enable_git_hooks.ps1` 尚为**未跟踪文件**，未提交前对任何克隆者均不可见（D-11 修复的落盘动作未完成）。
- ~~本地开发链路（dev-proxy / run-dev）无交付路径，亦无文档说明"如何获取"~~ → **已闭环（2026-09-12）**：三脚本已解除忽略、补齐降级保护、README 增说明，并实跑验证（见 3.1）。
- 本清单及其同级 HCSE 文档**当前仍未 `git add`**：解除忽略只解决了"可达版本控制"，未解决"已进入版本控制"。发布前须确认 3.1 表中所有标"待提交"的文件已提交。**该条同样适用于本轮新解忽略的 3 个开发链路脚本**。

### 3.4 交付性缺陷的通用模式（本轮新增认知）

本轮两次交付性缺陷（r10 HCSE 文档、r13/r14 开发链路脚本）**同源**：`local-dev` 在 v0.8.45/v0.9.0/v0.9.1 三次"发布清理"中，以"不交付用户，本地开发自用"为注释批量排除文件。**该判断对"纯本地辅助文件"成立，但对"被已跟踪文件引用的文件"不成立**——后者一旦被排除，引用方在克隆环境必然失效。

**识别方法（强制）**：每次涉及 `.gitignore` 变更，须执行"引用反向扫描"——对本次新增/修改的每条忽略规则，逐一检查是否有**已跟踪文件**引用该路径：

```powershell
# 对候选被忽略路径，反向查找引用它的已跟踪文件（PS 5.1 合规）
$repoRoot = 'g:\code-memory'
$candidate = 'dev-proxy'          # 被忽略文件的特征串
$trackedFiles = & git -C $repoRoot ls-files
$trackedFiles | Where-Object -FilterScript {
    $name = [string]$_
    $name -match '\.(md|js|json|toml|yml|yaml|rs|html|txt)$' -and
    (Get-Content -LiteralPath (Join-Path $repoRoot $name) -Raw -ErrorAction SilentlyContinue) -match $candidate
}
```

**本项源于两次真实事故**（非推演）：r10 的 HCSE 文档"回写即不可交付"，r13 的 dev-proxy"被引用却未交付"。其教训具通用性——**`.gitignore` 的正确性不能只看"是否应排除"，必须叠加"是否已被引用"**。

---

## 四、检查项 4：交付物可达性复检（2026-09-13 第六轮新增）

### 4.1 现状基线（实测，2026-09-13）

八轮修复累计新增 **16 个交付物**，**全部仍为未跟踪文件**（实测 `git ls-files --others --exclude-standard`，2026-09-13）：

| 交付物 | 归属 | 状态 |
|---|---|---|
| `.githooks/pre-commit` | r2 | 未跟踪 |
| `scripts/enable_git_hooks.ps1` | r2 | 未跟踪 |
| `docs/HCSE_RESILIENCE_AUDIT.md` | r6 | 未跟踪（已解除忽略） |
| `docs/HCSE_RELEASE_PROTOCOL.md` | r7 | 未跟踪（已解除忽略） |
| `scripts/dev-proxy.py` | r14 | 未跟踪（已解除忽略） |
| `scripts/run-dev.ps1` | r14 | 未跟踪（已解除忽略） |
| `scripts/run-dev.bat` | r14 | 未跟踪（已解除忽略） |
| `src/model_ids.rs` | b1 | 未跟踪（**新增模块，源文件**） |
| `src/atomic_file.rs` | b8b | 未跟踪（**新增模块，源文件**） |
| `src/v1_api_tests.rs` | b8b | 未跟踪（**新增模块，源文件**） |
| `src/memory_store_tests.rs` | b8b | 未跟踪（**新增模块，源文件**） |
| `src/memory_state_machine.rs` | b7 | 未跟踪（**上提 Layer 1，源文件**；原 `src/engine/memory_state_machine.rs` 已删除） |
| `src/errors.rs` | **c6** | 未跟踪（**新增模块，源文件**：统一错误契约） |
| `src/memory_store_types.rs` | **c3** | 未跟踪（**新增模块，源文件**：数据契约外提） |
| `src/memory_store_cache.rs` | **c7** | 未跟踪（**新增模块，源文件**：缓存子系统外提） |
| `docs/GLOBAL_CODE_REVIEW_REPORT.md` | 审查 | 未跟踪 |

> **⚠ 趋势警示（M18）**：**每一轮"物理外提/拆分"重构都会净增未跟踪源文件**——
> 第七轮 5 个 `src/*.rs` → 第八轮 **8 个**（新增 `errors.rs` / `memory_store_types.rs`
> / `memory_store_cache.rs`）。这类重构本身正确（降低耦合），但**其交付风险随轮次单调上升**：
> 只要漏 `git add` 一个，克隆环境即编译失败。故**"外提完成后立即 `git add`"应固化为重构的收尾步骤**。

### 4.2 强制检查步骤

1. **发布前必须执行**：`git ls-files --others --exclude-standard` 列出全部"会随克隆交付的候选"，
   逐项确认其**是否需要交付**。不需要交付的（如临时 evidence）应删除或加入 `.gitignore`；
   需要交付的必须 `git add`。
2. **新增源文件的高危性**：`src/*.rs` 若为未跟踪状态，**克隆环境将编译失败**
   （`mod` 声明找不到文件）。这类断裂比"脚本缺失"更严重——它直接阻塞构建。
   第六轮新增的 3 个、**第八轮新增的 3 个** `src/*.rs` 文件均属此类，**发布前必须提交**。
3. **禁止用"文件已写盘"代替"已可交付"**：`Write` 工具成功 ≠ 进入版本控制。
   须以 `git status --short` 中的 `??` / `M` 标记为准。
4. **临时脚本清理**：`temp/*.ps1` 类取证脚本属**过程产物**，不应交付；
   但对应的 `temp/*.txt` evidence **建议保留至 PR 合并**，供复核引用。
5. **重构收尾即提交（新增，第八轮）**：任何"外提模块 / 拆分文件"类重构，
   **在验证通过后立即 `git add` 新增的 `src/*.rs`**，不要批量留到发布前——
   遗漏概率随未跟踪文件数线性上升。

### 4.3 已知未闭环缺口

- **✅ 已闭环（2026-09-13）**：上述 **16 个未跟踪交付物已全部 `git add` 并提交**——
  提交 `b284ca2`（分支 `fix/code-review-8rounds`）。其中 **8 个 `src/*.rs` 源文件**
  （`model_ids.rs` / `atomic_file.rs` / `v1_api_tests.rs` / `memory_store_tests.rs` /
  `memory_state_machine.rs` / `errors.rs` / `memory_store_types.rs` /
  `memory_store_cache.rs`）若缺失会导致克隆后编译失败，本次**已优先纳入同一提交**。
  复核命令 `git ls-files --others --exclude-standard` 现返回**空**；
  提交亦经**预提交钩子 5 项检查全部通过**（fmt + clippy + check + test + 泄露检测）。
- **剩余跟踪项（非缺陷）**：本地提交尚未推送（`git branch -vv` 显示分支无 upstream）。
  这是**有意为之**——推送属对外可见操作，建议先复核 diff 再决定。
- `temp/` 目录下的第六~八轮取证脚本（`b8a-check.ps1`、`b8d-*.ps1`、`b8f-regression.ps1`、`b9-*.ps1`、**`c2-*.ps1`、`c5-regression.ps1`、`c6-errsigs.ps1`** 等）
  属过程产物，`temp/` 整体被 `.gitignore:324` 忽略（不随克隆交付），**无需手动清理**；
  对应 `.txt` evidence 保留供复核。

### 4.4 第六轮暴露的方法论（与前四项并列）

| 编号 | 方法论 | 来源 |
|---|---|---|
| M1 | **判断 `#[allow(dead_code)]` 是否过时，不能靠人工阅读**——"看起来在用"与"编译器认为在用"是两回事。须用「**移除 + `cargo check --all-targets`**」作判据 | b8d（报告称 19 处过时，实测仅 3 处） |
| M2 | **断言"框架行为缺失"前须核对框架源码/官方文档**，而非仅检查项目是否显式配置 | r16（axum 默认 2MB 曾误判为"缺限制"） |
| M3 | **判定"残留/垃圾"前须核对项目既有约定**（CHANGELOG / 文档） | P0-1（Junction 误判）、b8d（`engine/archive/` 实为 CHANGELOG 约定的归档位） |
| M4 | **结构性重构前先核对许可证边界**——"抽象放错层"与"许可归属放错目录"的修复方式完全不同（后者收敛为"移动 + 再导出"） | b7（P0-2 依赖倒置） |
| M5 | **测试岛外提是 God Object 的最小风险切片**——以"测试用例数不变"作为等价性证明（625 tests 前后一致） | b8b |
| M6 | **`include!()` 引入的文件禁用 `//!` 内部文档注释**（E0753），此约束须就地注明以防后续误改 | b8d（`url_safety.rs`） |
| M18 | **每一轮"外提/拆分"重构都会净增未跟踪源文件，交付风险随轮次单调上升**——第七轮 5 个 `src/*.rs` → 第八轮 8 个。故"**外提完成后立即 `git add`**"应固化为重构的收尾步骤，而非留到发布前批量处理 | c3/c6/c7（`errors.rs` / `memory_store_types.rs` / `memory_store_cache.rs`） |

---

## 五、检查项 5：门禁对照实验的假阴性防护（2026-09-14 第九轮新增）

**针对的故障模式**：验证"门禁能否失败"（M9）时，对照实验的标准做法是「**变异被测逻辑 → 期望失败 → 还原 → 期望通过**」。但若用 `[System.IO.File]::Copy` 做备份/还原，
**该 API 保留源文件的 `LastWriteTime`**——还原会把被测文件的 mtime **回退**到变异前的时刻，
而此刻**变异产物的二进制 mtime 更新**。cargo/增量构建依 mtime 判定"源未变更"，
于是**跳过重编译、复用变异产物**，导致"还原后仍失败"的**假阴性**。

**本项源于一次真实误判（非推演）**：第九轮 §10.23 联想门禁加固中，首轮对照实验报出
`RESTORED_EXIT=101`，一度被误读为「**H1 真实退化 ⇒ 生产联想质量下降**」这一高危结论。
复核后确认：① 日志首行 `Finished release profile in 1.40s`（**未重编**）；② 日志打印
`11 / 16（阈值 ≥10）→ FAIL`——源码 `11 >= 10` 必为 `true`，**打印值与源码计算逻辑直接冲突**；
③ 二进制 mtime（`03:48:48`）晚于源文件（`02:14:30`）。三证合一方定位为取证缺陷。

### 5.1 强制检查步骤

1. **还原后必须强制重编**：`File.Copy` 之后追加显式推进 mtime，`File.Copy` 的
   mtime 保真行为**正是陷阱本体**：

```powershell
# 还原后强制 cargo 重编：File.Copy 保留源 mtime，会使 cargo 误判"无变更"而复用变异产物
[System.IO.File]::Copy($backup, $target, $true)
$stamp = (Get-Date).AddSeconds(5)
[System.IO.File]::SetLastWriteTime($target, $stamp)
```

2. **哈希一致 ≠ 二进制新鲜**：`RESTORE_BYTE_EXACT=True`（SHA256 相同）**只证明磁盘内容已还原**，
   **不证明即将运行的二进制由该内容编译**。二者是独立事实，**不可用哈希校验替代重编校验**。
3. **必须交叉核对"打印值 vs 源码逻辑"**：若日志打印的数值与源码对同一表达式的计算结果
   相矛盾（如 `11 >= 10` 却打印 `FAIL`），**该矛盾即为二进制陈旧的可判定证据**，
   须立即中断结论采信并核查构建新鲜度。
4. **必须核对构建新鲜度**：对照实验日志须保留 cargo 的 `Finished ... in <T>` 行；
   `T` 若远小于正常编译耗时（本项目 `server,ml` release ≈ 3m20s），即为**未重编**的信号。
5. **用 `cmd /c` 落盘原始输出**：PS 5.1 下 `2>&1 | Out-String` 会把原生命令的 stderr
   逐行包装为 `ErrorRecord`，其 `ToString()` **截断长行**——本项目曾因此丢失关键的
   `H1 救回率` 诊断行、并把 panic 行截断在 `src\v1`。**取证须经 OS 层重定向**：

```powershell
# 绕开 PS5.1 ErrorRecord 截断：由 cmd.exe 在 OS 层做重定向，保留原始字节
& cmd.exe /c 'cargo test ... -- --nocapture > temp\evidence.txt 2>&1'
```

### 5.2 与检查项 4 的关系

检查项 4 的 M1（"移除 + `cargo check` 作判据"）与本法同源：**编译器的判定依据（mtime/哈希）
与实际意图（内容变更）之间存在可被工具默认行为打破的隐含假设**。任何"改文件 → 跑门禁"
的验证，都必须确认**门禁实际运行的是改动后的产物**，而非缓存产物。

---

**回写溯源（更新）**：本清单由 `GLOBAL_CODE_REVIEW_REPORT.md` 第五节 3、4 项转化；
所有计数（36 / 33 / 68 / 2）均为审查时点实测值，变更时须重新取证而非沿用。
第 2 项源于一次**真实误判**（虚构"61 个重复 .rs"），其教训是通用性的：**递归扫描工具
必须显式处理 ReparsePoint**。第 4 项源于第六轮（b1~b8）修复，其教训同样通用：
**新增源文件未跟踪 = 克隆后编译失败，比缺脚本更严重**。检查项 4 于 2026-09-13 由第六轮
补充，其 4.4 节方法论 **M18 由第八轮（c2~c8）补充**（对应报告 8.14）。
**检查项 5 于 2026-09-14 由第九轮补充**，其教训同样源于**真实误判**（`RESTORED_EXIT=101`
曾被误读为生产质量退化），对应 `docs/ASSOCIATION_ACCURACY_PLAN.md` §10.23。
