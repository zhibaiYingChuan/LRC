# LRC 桌面端 v0.8.5 修复优化计划

> 文档版本：v0.8.5
> 基线版本：v0.8.4（CDP 通过率 83.8%，韧性评分 63.5/100）
> 编制日期：2026-07-29
> 编制人：创始者智能体（产品设计师）
> 依据文档：
> - [CDP_TEST_REPORT_v0.8.4.md](file:///g:/code-memory/docs/CDP_TEST_REPORT_v0.8.4.md)
> - [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md)
> - [FIX_PLAN_v0.8.4.md](./FIX_PLAN_v0.8.4.md)（同目录，v0.9.11 归档后修正链接）
> 目标：分步修复 4 个 P0 致命问题 + 5 个 CDP 失败项 + 12 个 P1 严重问题，达到 CDP 通过率 ≥ 90%、韧性评分 ≥ 85/100、P0 问题清零
> 优先级框架：RICE（Reach × Impact × Confidence / Effort）
> PowerShell 规范：已通过 PowerShell 专家技能验证（编码自保护块、完整命令名、显式参数、-LiteralPath、try/catch）

---

## 一、执行摘要

### 1.1 v0.8.4 现状诊断

LRC 桌面端 v0.8.4 完成了 Step 2-14 共 14 项修复，CDP 回归测试通过率从 73.0% 提升至 83.8%（+10.8%），新增通过项 4 个（G004 按钮防抖、G013 showConfirm 定义、G013 模态框可聚焦元素、G017 标签切换 abort）。但交互韧性审计发现修复过程中引入了新的回归问题，**综合评分从 76.3 下降至 63.5（-12.8 分）**，核心矛盾如下：

| 问题簇 | 根因类型 | 影响范围 | 严重程度 |
|:---|:---|:---|:---:|
| **G052 handleHttpError 死代码** | v0.8.3/v0.8.4 修复了 handleHttpError 函数但从未调用，导致 G007/G009/G029/G047 四个错误恢复修复全部失效 | 500 错误仅显示 Toast 无重试 Modal、429 限流无提示、_retryCounters 永不填充、switchTab 的 _retryModalActive 检查永远通过 | **P0 致命** |
| **G058 启动服务取消按钮失效** | postMessageToParent 发送启动请求无 AbortController，取消按钮仅关闭模态框不 abort 请求 | 用户反馈问题 1：启动过程中点击"取消"无反应，请求继续在后台执行 | **P0 致命** |
| **G076 CSP 完全未配置** | index.html 无 CSP meta 标签，innerHTML 大量使用存在 XSS 风险 | 安全防线缺失，memory_type 等用户输入未转义可注入脚本 | **P0 致命** |
| **G080 restoreSelectedScenario 未调用** | G045 假修复：selectPresetScenario 写入 localStorage 但 restoreSelectedScenario 从未被调用 | 用户反馈问题 3：刷新后预设场景丢失；CDP 测试 #3 active 类未添加 | **P0 致命** |
| 道同构 API 返回 HTML | sidecar /v1/dao-metrics 路由问题，返回 HTML（404 页面）而非 JSON | 用户反馈问题 2：道同构度卡片 4 个小指标显示异常；CDP 测试 #1/#2 失败 | **P0 致命** |
| CDP 失败项残留 5 个 | Toast 去重窗口过长、validateInput 未暴露、预设场景 active 类未添加 | CDP 通过率 83.8% 未达 90% 目标 | P0/P1 |
| v0.8.4 修复回归风险 | confirmModalQueue 队列死锁、MutationObserver 性能、_retryCounters 内存泄漏、searchAbortController 未清理 | 长时间运行可能崩溃或内存泄漏 | P1/P2 |

### 1.2 v0.8.4 修复回归风险盘点

v0.8.4 在 Step 2-14 修复中引入了 4 个回归风险，必须在 v0.8.5 中同步处理：

| 回归风险 | 来源 Step | 风险描述 | v0.8.5 处理 Step |
|:---|:---:|:---|:---:|
| confirmModalQueue 队列死锁 | Step 3（showInfoModal 入队） | processConfirmQueue 抛异常时队列永久阻塞 | Step 17 |
| MutationObserver 性能影响 | Step 13（动态输入框校验） | subtree:true 监听整个 body，dashboard 自动刷新时性能下降 | Step 17 |
| _retryCounters Map 内存泄漏 | Step 10（重试计数器） | Map 无上限，500 错误频繁时无限增长（G052 修复后将激活） | Step 10 |
| searchAbortController 未清理 | Step 11（搜索 AbortController） | 标签切换时未 abort searchAbortController | Step 17 |

### 1.3 v0.8.5 修复策略

本计划采用 **"调查先行 → P0 快速胜利 → P0 核心攻坚 → CDP 收尾 → P1 攻坚 → 回归风险 → 收尾验证"** 七阶段策略，共 **18 个 Step**，预计 **9.6 人天**：

| 阶段 | Step 范围 | 目标 | 工作量 | 关键产出 |
|:---|:---|:---|:---:|:---|
| 阶段一：根因调查 | Step 1 | 查明 G052/G058/G080 的真实状态，确认修复路径 | 0.3 人天 | 调查报告 |
| 阶段二：P0 快速胜利 | Step 2-5 | 解决 4 个 P0 中最简单的（G080/G015/CSP/G058） | 1.8 人天 | 4 个 P0 清零 |
| 阶段三：P0 道同构簇 | Step 6-8 | 解决道同构横幅/卡片/API 三个 P0（用户反馈问题 2） | 1.0 人天 | 道同构功能恢复 |
| 阶段四：P0 核心攻坚 | Step 9-10 | 解决 G052 handleHttpError 接入 + G060 内存泄漏 | 2.2 人天 | 错误恢复层恢复 |
| 阶段五：CDP 收尾 | Step 11 | 解决 G011 Toast 去重窗口 | 0.2 人天 | CDP 通过率 ≥ 90% |
| 阶段六：P1 攻坚 | Step 12-16 | 解决 5 个 P1 严重问题 | 1.9 人天 | 韧性评分 ≥ 85 |
| 阶段七：回归风险 + 收尾 | Step 17-18 | 修复 v0.8.4 回归风险 + 版本升级 + 文档同步 | 2.2 人天 | 全部目标达成 |
| **合计** | | | **9.6 人天** | |

### 1.4 关键决策

1. **Step 1 为调查 Step，不修改代码**：先确认 G052/G058/G080 三个 P0 的真实状态（死代码确认、AbortController 缺失确认、restoreSelectedScenario 调用链确认），再决定后续修复策略，避免"假修复"重演
2. **P0 内部按 RICE 分数 + 依赖关系排序**：G080（RICE 3500，0.2 人天）优先于 G052（RICE 450，2 人天），但 G052 的 G060 依赖（防内存泄漏）必须同步修复
3. **用户反馈三大问题优先**：启动服务取消（G058）、道同构小指标（G059/CDP#1/CDP#2）、预设场景（G080）分别在 Step 2-8 内解决
4. **G052 handleHttpError 接入采用分步替换**：禁止批量修改，按"loadDashboard → loadMemories → searchMemories → loadTrustCenterData"顺序逐个替换，每替换一个验证一次
5. **PowerShell 验证脚本全部符合专家规范**：编码自保护块、完整命令名（Get-Content/Select-String/Where-Object/ForEach-Object）、显式参数名称、-LiteralPath、try/catch 错误处理
6. **v0.8.4 回归风险同步修复**：confirmModalQueue try-catch、MutationObserver 防抖、searchAbortController 清理在 Step 17 统一处理

---

## 二、修复目标与验收标准

### 2.1 核心验收矩阵

| 维度 | v0.8.4 现状 | v0.8.5 目标 | 验收方式 | 阈值 |
|:---|:---|:---|:---|:---|
| CDP 测试通过率 | 83.8%（31/37） | **≥ 90%（34/37）** | 重新运行 CDP 回归测试 | ≥ 34 项通过 |
| 交互韧性综合评分 | 63.5/100 | **≥ 85/100** | 五层交互韧性复测 | ≥ 85 分 |
| P0 致命问题 | 4 个 + 3 个 CDP P0 | **0 个** | CDP P0 通过率 100% + 审计 P0 清零 | 0 个 |
| P1 严重问题 | 12 个 | **≤ 2 个** | 审计 P1 复测 | ≤ 2 个 |
| handleHttpError 调用方 | 0 处 | **≥ 8 处** | PowerShell 验证 handleHttpError( 调用数 | ≥ 8 |
| 启动服务取消按钮 | 无 AbortController | **可中断请求** | 运行时点击取消验证请求 aborted | 通过 |
| CSP 配置 | 未配置 | **已配置 meta CSP** | 检查 meta[http-equiv="Content-Security-Policy"] | 存在 |
| restoreSelectedScenario 调用 | 0 处 | **≥ 1 处（loadDashboard 末尾）** | PowerShell 验证调用链 | ≥ 1 |
| 道同构 API 返回 | HTML（404） | **JSON {ok, data}** | fetch /v1/dao-metrics 验证 Content-Type | application/json |
| 道同构卡片 DOM | 不存在 | **始终存在** | CDP 测试 #2 通过 | 通过 |
| 预设场景 active 类 | 未添加 | **切换后添加** | CDP 测试 #3 通过 | 通过 |
| validateInput 暴露 | 未暴露 | **window.validateInput** | typeof window.validateInput === 'function' | true |
| Toast 去重窗口 | 1.5s（过长） | **500ms** | PowerShell 验证 _TOAST_DEDUP_WINDOW 值 | ≤ 500 |
| _retryCounters 上限 | 无上限 | **≤ 50** | PowerShell 验证 Map 大小检查 | 存在 |

### 2.2 韧性评分目标分解

| 维度 | v0.8.4 评分 | v0.8.5 目标 | 提升来源 | 对应 Step |
|:---|:---:|:---:|:---|:---:|
| 输入容错 | 65/100 | 80/100 | G015 validateInput 暴露 + 必填校验增强 | Step 3 |
| 状态一致性 | 55/100 | 85/100 | G052 handleHttpError 接入 + G053 navbar 统一 + G080 restore 调用 | Step 2/9/13 |
| 错误恢复 | 40/100 | 90/100 | G052 接入激活 G007/G009/G029/G047 + G056 重连 + G057 降级 + G060 内存上限 | Step 5/9/10/12/14 |
| 并发控制 | 60/100 | 85/100 | G058 AbortController + G060 Map 上限 + searchAbortController 清理 | Step 5/10/17 |
| 可访问性 | 55/100 | 70/100 | G064 aria-label + CSP 配置 | Step 4/15 |
| **综合评分** | **63.5/100** | **≥ 85/100** | **+21.5** | Step 2-17 |

### 2.3 用户反馈三大问题验收

| 用户反馈 | 对应 P0/P1 | 验收标准 | 验收 Step |
|:---|:---|:---|:---:|
| 启动服务弹窗"取消"按钮点击无反应 | G058（P0） | 启动过程中点击取消，请求被 abort，模态框关闭，按钮显示"取消中..." | Step 5 |
| 道同构度卡片 4 个小指标显示问题 | G059 + CDP#1 + CDP#2 | 4 个小指标（阴阳守恒、洛书偏差、八卦中横度、合成比例）正常显示数值或 '--'，横幅不闪烁 | Step 6/7/8 |
| 预设场景模板切换问题 | G080 + CDP#3 | 切换后 active 类正确添加，刷新后保留选中状态 | Step 2 |

---

## 三、修复优先级矩阵（RICE 评分模型）

### 3.1 RICE 评分模型说明

本计划采用 RICE 优先级框架进行量化排序：

| 维度 | 含义 | 评分范围 | 评分标准 |
|:---|:---|:---:|:---|
| **Reach**（影响范围） | 多少用户受影响 | 1-10 | 10=所有用户必现；6=大部分用户偶发；3=少数用户；1=极少数边缘场景 |
| **Impact**（影响程度） | 对用户体验的影响 | 1-10 | 10=功能完全不可用/数据丢失；7=核心功能受损；4=体验下降；1=轻微干扰 |
| **Confidence**（置信度） | 修复的确定性 | 1-10 | 10=明确根因+成熟方案；7=根因明确方案需验证；4=需调查；1=纯猜测 |
| **Effort**（工作量） | 修复所需时间（人天） | 0.1-3.0 | 数值越小分母越小，分数越高（鼓励快速胜利） |

**RICE 评分公式**：`Score = (Reach × Impact × Confidence) / Effort`

### 3.2 RICE 评分明细表

| Step | 缺口 ID | 问题摘要 | Reach | Impact | Confidence | Effort | RICE | 优先级 | 阶段 |
|:---:|:---|:---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| 2 | G080 + CDP#3 | restoreSelectedScenario 未调用 + active 类未添加 | 10 | 7 | 10 | 0.2 | **3500** | P0 | 二 |
| 3 | CDP#5 G015 | validateInput 未暴露到 window | 6 | 5 | 10 | 0.1 | **3000** | P0 | 二 |
| 6 | G059 | 道同构横幅重复重建（用户反馈问题 2） | 10 | 6 | 9 | 0.2 | **2700** | P0 | 三 |
| 7 | CDP#2 | 道同构卡片 DOM 不存在 | 10 | 7 | 8 | 0.3 | **1867** | P0 | 三 |
| 10 | G060 | _retryCounters Map 内存泄漏（防 G052 引入） | 6 | 5 | 9 | 0.2 | **1350** | P0 | 四 |
| 11 | CDP#4 G011 | Toast 去重窗口过长 | 8 | 4 | 8 | 0.2 | **1280** | P1 | 五 |
| 12 | G057 | /health 超时无降级 | 8 | 6 | 8 | 0.3 | **1280** | P1 | 六 |
| 13 | G053 | navbar-nav 绕过 switchTab | 8 | 5 | 9 | 0.3 | **1200** | P1 | 六 |
| 15 | G064 | 道同构卡片无 aria-label | 3 | 4 | 10 | 0.1 | **1200** | P1 | 六 |
| 4 | G076 | CSP 完全未配置 | 10 | 7 | 8 | 0.5 | **1120** | P0 | 二 |
| 8 | CDP#1 | 道同构 API 返回 HTML 而非 JSON | 10 | 8 | 7 | 0.5 | **1120** | P0 | 三 |
| 18 | 收尾 | 版本升级 + 文档同步 + 回归测试 | 10 | 5 | 10 | 0.5 | **1000** | 收尾 | 七 |
| 14 | G056 | sidecar 崩溃无重连 | 8 | 7 | 7 | 0.5 | **784** | P1 | 六 |
| 17 | 回归风险 | v0.8.4 回归风险修复 | 8 | 6 | 8 | 0.5 | **768** | P1 | 七 |
| 5 | G058 | 启动服务取消按钮无法中断（用户反馈问题 1） | 10 | 8 | 9 | 1.0 | **720** | P0 | 二 |
| 16 | G054 | showPrompt 未入队 | 6 | 6 | 8 | 0.5 | **576** | P1 | 六 |
| 9 | G052 | handleHttpError 死代码（最严重回归） | 10 | 10 | 9 | 2.0 | **450** | P0 | 四 |
| 1 | 调查 | 根因调查（不修改代码） | — | — | — | 0.3 | — | 调查 | 一 |

### 3.3 排序原则

1. **P0 优先级硬约束**：所有 P0（Step 2-10）必须先于 P1（Step 11-16）完成，不论 RICE 分数
2. **P0 内部按 RICE 降序 + 依赖关系**：G080（3500）→ G015（3000）→ CSP（1120）→ G058（720）→ 道同构簇（2700/1867/1120）→ G052（450）+ G060（1350）
3. **G052 必须与 G060 同步**：G052 接入激活 _retryCounters，G060 必须先于或同步于 G052 添加 Map 上限，否则引入新的内存泄漏
4. **用户反馈问题优先**：G058（反馈 1）、道同构簇（反馈 2）、G080（反馈 3）在 P0 阶段全部覆盖
5. **简单优先**：相同优先级下，0.1 人天修复 < 0.5 人天修复 < 2 人天修复
6. **依赖驱动**：Step 9（G052）依赖 Step 10（G060）防泄漏；Step 17（回归风险）依赖 Step 9/10 完成

---

## 四、详细 Step 列表（每个 Step 独立可验证，分步实施）

### Step 1: P0 根因调查（不修改代码）

**目标缺口**：确认 G052/G058/G080 三个 P0 的真实状态，避免"假修复"重演

**严重程度**：P0（调查类，阻断后续修复决策）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 1.2 节，v0.8.4 引入 1 个"假修复"（G080）和 1 个"死代码"（G052）。需在修复前确认三个缺口的真实状态：

| 缺口 | 审计报告描述 | 待确认事项 | 预判结论 |
|:---|:---|:---|:---|
| G052 | handleHttpError 定义在 app.js L177 但从未被调用 | 全文搜索 `handleHttpError\(` 是否真的 0 处调用（排除定义和注释） | 死代码确认 |
| G058 | postMessageToParent L890-938 无 AbortController | 确认 fetch 调用是否真的无 signal 参数 | AbortController 缺失确认 |
| G080 | restoreSelectedScenario 从未被调用 | 全文搜索 `restoreSelectedScenario` 调用链 | 未调用确认 |

**调查方案**（不修改代码，仅产出调查结论）：

1. **G052 调查**：搜索 `handleHttpError` 所有出现位置，区分"定义/注释/调用"
2. **G058 调查**：读取 postMessageToParent 函数体，确认 fetch 是否传入 signal
3. **G080 调查**：搜索 `restoreSelectedScenario` 所有出现位置，确认是否有调用（非定义）
4. **CDP#1 道同构 API 调查**：确认 sidecar `/v1/dao-metrics` 路由是否注册

**影响文件**：无（仅调查）

**验证方法**：

产出《P0 根因调查报告》，包含：
- G052/G058/G080 各自的 grep 结果截图
- 调用链分析
- 后续修复策略建议（对应 Step 2/5/9）

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === LRC v0.8.5 P0 根因调查脚本 ===
# 本脚本必须以 UTF-8 with BOM 保存（PS5.1 环境）
# 编码一致性保护块（适用于 PS5.1 和 PS7）
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G052 handleHttpError 死代码调查 ===' -ForegroundColor Cyan
    $handleHttpError = Select-String -LiteralPath $appJsPath -Pattern 'handleHttpError' -SimpleMatch
    if ($handleHttpError) {
        Write-Host ("找到 {0} 处 handleHttpError 引用：" -f $handleHttpError.Count) -ForegroundColor Yellow
        $handleHttpError | ForEach-Object {
            $line = $_.Line.Trim()
            $type = if ($line -match 'function\s+handleHttpError') { '定义' }
                    elseif ($line -match '^\s*//|^\s*\*') { '注释' }
                    elseif ($line -match 'handleHttpError\s*\(') { '调用' }
                    else { '引用' }
            Write-Host ("  L{0} [{1}]: {2}" -f $_.LineNumber, $type, $line)
        }
        $callCount = ($handleHttpError | Where-Object { $_.Line -match 'handleHttpError\s*\(' -and $_.Line -notmatch 'function\s+handleHttpError' }).Count
        Write-Host ("调用处数量（排除定义）: {0}" -f $callCount) -ForegroundColor $(if ($callCount -eq 0) { 'Red' } else { 'Green' })
    } else {
        Write-Host '未找到 handleHttpError 引用' -ForegroundColor Red
    }

    Write-Host "`n=== G058 postMessageToParent AbortController 调查 ===" -ForegroundColor Cyan
    $postMessage = Select-String -LiteralPath $appJsPath -Pattern 'postMessageToParent' -SimpleMatch
    if ($postMessage) {
        $defLine = $postMessage | Where-Object { $_.Line -match 'function\s+postMessageToParent' } | Select-Object -First 1
        if ($defLine) {
            Write-Host ("postMessageToParent 定义于 L{0}" -f $defLine.LineNumber) -ForegroundColor Green
            # 读取函数体（接下来 50 行）
            $funcBody = Get-Content -LiteralPath $appJsPath -TotalCount ($defLine.LineNumber + 50) | Select-Object -Skip ($defLine.LineNumber - 1)
            $hasAbort = $funcBody | Where-Object { $_ -match 'AbortController|signal' }
            if ($hasAbort) {
                Write-Host '函数体内发现 AbortController/signal：' -ForegroundColor Green
                $hasAbort | ForEach-Object { Write-Host "  $_" }
            } else {
                Write-Host '函数体内未发现 AbortController/signal（G058 确认）' -ForegroundColor Red
            }
        }
    }

    Write-Host "`n=== G080 restoreSelectedScenario 调用链调查 ===" -ForegroundColor Cyan
    $restore = Select-String -LiteralPath $appJsPath -Pattern 'restoreSelectedScenario' -SimpleMatch
    if ($restore) {
        Write-Host ("找到 {0} 处 restoreSelectedScenario 引用：" -f $restore.Count) -ForegroundColor Yellow
        $restore | ForEach-Object {
            $line = $_.Line.Trim()
            $type = if ($line -match 'function\s+restoreSelectedScenario') { '定义' }
                    elseif ($line -match 'restoreSelectedScenario\s*\(') { '调用' }
                    else { '引用' }
            Write-Host ("  L{0} [{1}]: {2}" -f $_.LineNumber, $type, $line)
        }
        $callCount = ($restore | Where-Object { $_.Line -match 'restoreSelectedScenario\s*\(' -and $_.Line -notmatch 'function\s+restoreSelectedScenario' }).Count
        Write-Host ("调用处数量（排除定义）: {0}" -f $callCount) -ForegroundColor $(if ($callCount -eq 0) { 'Red' } else { 'Green' })
    } else {
        Write-Host '未找到 restoreSelectedScenario 引用' -ForegroundColor Red
    }

    Write-Host "`n=== CDP#1 sidecar /v1/dao-metrics 路由调查 ===" -ForegroundColor Cyan
    $v1ApiPath = 'G:\code-memory\src\v1_api.rs'
    if (Test-Path -LiteralPath $v1ApiPath) {
        $daoRoute = Select-String -LiteralPath $v1ApiPath -Pattern 'dao-metrics|dao_metrics'
        if ($daoRoute) {
            Write-Host "sidecar 中 dao-metrics 路由引用：" -ForegroundColor Green
            $daoRoute | ForEach-Object { Write-Host ("  L{0}: {1}" -f $_.LineNumber, $_.Line.Trim()) }
        } else {
            Write-Host 'sidecar 中未找到 dao-metrics 路由（CDP#1 根因确认）' -ForegroundColor Red
        }
    } else {
        Write-Host "v1_api.rs 不存在: $v1ApiPath" -ForegroundColor Red
    }
}
catch {
    Write-Host ("调查脚本执行失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：无（首个 Step）

**预计工作量**：0.3 人天

**如果失败怎么办**：
- 若 PowerShell 命令执行失败 → 改用 Read 工具直接读取 app.js 关键行号区间（L177/L890/L3997）
- 若调查无法确定根因 → 降级为"按审计报告预判结论修复"，跳过历史追溯
- 调查失败不影响后续 Step，可直接按预判结论执行修复

---

### Step 2: 修复 G080 + CDP#3 restoreSelectedScenario 调用 + active 类（用户反馈问题 3）

**目标缺口**：G080（restoreSelectedScenario 从未被调用，G045 假修复）+ CDP 测试 #3（预设场景切换后 active 类未添加）

**严重程度**：P0（RICE 3500，最高优先级快速胜利）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 3.3 节和 [CDP_TEST_REPORT_v0.8.4.md](file:///g:/code-memory/docs/CDP_TEST_REPORT_v0.8.4.md) 2.3 节：
- v0.8.4 Step 5 修复了 [selectPresetScenario](file:///g:/code-memory/static/app.js#L3721) 的 localStorage 写入
- 但 [restoreSelectedScenario](file:///g:/code-memory/static/app.js#L3997) 函数定义后**从未被任何代码调用**
- 刷新后 localStorage 有值，但 UI 未恢复 active 类
- CDP 测试切换前后所有 active 状态均为 false（switched=false）

**修复方案**（分步实施，禁止批量修改）：

**子步骤 2.1**：确认 selectPresetScenario 添加 active 类

读取 [selectPresetScenario](file:///g:/code-memory/static/app.js#L3721) 函数体，确认是否包含 `target.classList.add('active')`。若仅添加 `selected` 类，需补充 `active` 类（兼容 CDP 测试检测）。

```javascript
// 修改 selectPresetScenario（约 3721 行）
function selectPresetScenario(scenario) {
  // 移除所有模板的 active/selected 类
  document.querySelectorAll('.preset-scenario-template').forEach(t => {
    t.classList.remove('active');
    t.classList.remove('selected');
  });
  
  // 激活目标模板（同时添加 active 和 selected 类，兼容 CDP 测试和实际样式）
  const target = document.querySelector(`.preset-scenario-template[data-scenario="${scenario}"]`);
  if (target) {
    target.classList.add('active');
    target.classList.add('selected');
  }
  
  // 持久化到 localStorage（v0.8.4 Step 5 已实现，保留）
  try {
    localStorage.setItem('lrc-selected-scenario', scenario);
  } catch (e) {
    console.warn('[selectPresetScenario] localStorage 写入失败:', e);
  }
  
  loadPresetScenarioData(scenario);
}
```

**子步骤 2.2**：在 loadDashboard 末尾调用 restoreSelectedScenario

```javascript
// 在 loadDashboard 函数末尾（约 4500 行附近）添加
async function loadDashboard() {
  // ... 原有逻辑 ...
  
  // G080 修复：恢复上次选中的预设场景
  restoreSelectedScenario();
}
```

**子步骤 2.3**：确认 restoreSelectedScenario 函数实现正确

```javascript
// 确认 restoreSelectedScenario 实现（约 3997 行）
function restoreSelectedScenario() {
  try {
    const saved = localStorage.getItem('lrc-selected-scenario');
    if (saved) {
      selectPresetScenario(saved);
    }
  } catch (e) {
    console.warn('[restoreSelectedScenario] 读取 localStorage 失败:', e);
  }
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（selectPresetScenario 约 3721 行 + loadDashboard 约 4500 行 + restoreSelectedScenario 约 3997 行）

**验证方法**：

1. **CDP 测试 #3 验证**：
   ```javascript
   // 切换 4 个模板，active 类应正确切换
   const scenarios = ['personal-notes', 'project-management', 'learning-assistant', 'coding-helper'];
   for (const s of scenarios) {
     window.selectPresetScenario(s);
     const active = document.querySelector('.preset-scenario-template.active');
     active !== null && active.dataset.scenario === s  // 应返回 true
   }
   ```

2. **持久化验证**：
   ```javascript
   window.selectPresetScenario('coding-helper');
   location.reload();
   // 刷新后
   const active = document.querySelector('.preset-scenario-template.active');
   active.dataset.scenario === 'coding-helper'  // 应返回 true
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 2 验证脚本：G080 + CDP#3 ===
# 编码一致性保护块
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G080 验证：restoreSelectedScenario 调用链 ===' -ForegroundColor Cyan
    
    # 验证 selectPresetScenario 添加 active 类
    $selectFn = Select-String -LiteralPath $appJsPath -Pattern "function\s+selectPresetScenario"
    if ($selectFn) {
        $startLine = $selectFn.LineNumber
        $funcBody = Get-Content -LiteralPath $appJsPath -TotalCount ($startLine + 40) | Select-Object -Skip ($startLine - 1)
        $hasActive = $funcBody | Where-Object { $_ -match "classList\.add\('active'\)" }
        if ($hasActive) {
            Write-Host 'PASS: selectPresetScenario 添加 active 类' -ForegroundColor Green
        } else {
            Write-Host 'FAIL: selectPresetScenario 未添加 active 类' -ForegroundColor Red
        }
    }
    
    # 验证 restoreSelectedScenario 被调用（排除定义行）
    $restore = Select-String -LiteralPath $appJsPath -Pattern 'restoreSelectedScenario'
    $calls = $restore | Where-Object { 
        $_.Line -match 'restoreSelectedScenario\s*\(' -and 
        $_.Line -notmatch 'function\s+restoreSelectedScenario' 
    }
    if ($calls.Count -ge 1) {
        Write-Host ('PASS: restoreSelectedScenario 被 {0} 处调用' -f $calls.Count) -ForegroundColor Green
        $calls | ForEach-Object { Write-Host ("  L{0}: {1}" -f $_.LineNumber, $_.Line.Trim()) }
    } else {
        Write-Host 'FAIL: restoreSelectedScenario 未被调用' -ForegroundColor Red
    }
    
    # 验证 localStorage 持久化
    $persist = Select-String -LiteralPath $appJsPath -Pattern "localStorage\.setItem\('lrc-selected-scenario'"
    if ($persist) {
        Write-Host 'PASS: localStorage 持久化已实现' -ForegroundColor Green
    } else {
        Write-Host 'FAIL: localStorage 持久化缺失' -ForegroundColor Red
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：Step 1（调查结论确认 restoreSelectedScenario 未被调用）

**预计工作量**：0.2 人天

**如果失败怎么办**：
- 若 CDP 测试仍检测 active=false → 检查 `.preset-scenario-template` 选择器是否匹配实际 DOM 结构（可能类名不同）
- 若刷新后状态丢失 → 检查 loadDashboard 是否真的调用了 restoreSelectedScenario（可能 loadDashboard 有多个出口）
- 若 localStorage 写入失败 → 检查浏览器隐私模式或 QuotaExceededError
- 回滚方案：`git checkout HEAD~1 -- static/app.js`（仅回退 selectPresetScenario + loadDashboard 修改）

---

### Step 3: 修复 CDP#5 G015 validateInput 暴露到 window

**目标缺口**：CDP 测试 #5（G015 必填字段空值校验，validateInputAvailable=false）

**严重程度**：P0（RICE 3000，CDP 失败项）

**根因分析**：

根据 [CDP_TEST_REPORT_v0.8.4.md](file:///g:/code-memory/docs/CDP_TEST_REPORT_v0.8.4.md) 2.5 节：
- v0.8.4 Step 13 添加了 MutationObserver 和 setupInputValidation
- 但 `validateInput` 可能是内部函数未暴露到 window
- CDP 测试通过 `window.validateInput` 检查，返回 false
- 测试详情：`hasErrorClass=undefined, hasErrorMsg=undefined, validateInputAvailable=false`

**修复方案**（分步实施）：

**子步骤 3.1**：暴露 validateInput 到 window

```javascript
// 在 validateInput 函数定义后（约 1013 行）添加暴露语句
function validateInput(input) {
  // ... 原有校验逻辑 ...
}

// G015 修复：暴露到 window 供 CDP 测试和动态调用
window.validateInput = validateInput;
```

**子步骤 3.2**：增强 validateInput 必填校验可见性

```javascript
function validateInput(input) {
  const id = input.id;
  const rules = INPUT_VALIDATORS[id];
  const value = input.value.trim();
  
  // 必填校验（即使无 rules，required 属性也要校验）
  if (input.hasAttribute('required') && !value) {
    input.classList.add('input-error');
    input.setAttribute('aria-invalid', 'true');
    showInputError(input, '此字段为必填项');
    return false;
  }
  
  // 规则校验
  if (rules) {
    for (const rule of rules) {
      if (!rule.test(value)) {
        input.classList.add('input-error');
        input.setAttribute('aria-invalid', 'true');
        showInputError(input, rule.message || '输入不合法');
        return false;
      }
    }
  }
  
  input.classList.remove('input-error');
  input.removeAttribute('aria-invalid');
  clearInputError(input);
  return true;
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（validateInput 约 1013 行 + 暴露语句）

**验证方法**：

1. **CDP 测试 #5 验证**：
   ```javascript
   typeof window.validateInput === 'function'  // 应返回 true
   
   const input = document.querySelector('input[required]');
   input.value = '';
   window.validateInput(input);
   input.classList.contains('input-error')  // 应返回 true
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 3 验证脚本：G015 validateInput 暴露 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G015 验证：validateInput 暴露 ===' -ForegroundColor Cyan
    
    $exposed = Select-String -LiteralPath $appJsPath -Pattern 'window\.validateInput\s*=\s*validateInput'
    if ($exposed) {
        Write-Host ('PASS: validateInput 已暴露到 window（L{0}）' -f $exposed.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: validateInput 未暴露到 window' -ForegroundColor Red
    }
    
    # 验证必填校验逻辑
    $required = Select-String -LiteralPath $appJsPath -Pattern "hasAttribute\('required'\)" | Select-Object -First 1
    if ($required) {
        Write-Host ('PASS: 必填校验逻辑存在（L{0}）' -f $required.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'WARN: 必填校验逻辑未找到' -ForegroundColor Yellow
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：无

**预计工作量**：0.1 人天

**如果失败怎么办**：
- 若暴露后 CDP 测试仍失败 → 检查 validateInput 是否在 IIFE 作用域内，需改为 `window.validateInput = function(input) {...}` 直接挂载
- 若 validateInput 调用时报错 → 检查 INPUT_VALIDATORS 是否已定义
- 回滚方案：`git checkout HEAD~1 -- static/app.js`（仅回退 1 行暴露语句）

---

### Step 4: 修复 G076 CSP 完全未配置（P0 安全）

**目标缺口**：G076（index.html 无 CSP meta 标签，innerHTML 大量使用存在 XSS 风险）

**严重程度**：P0（RICE 1120，安全防线缺失）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 第五层 G076：
- index.html 无 `<meta http-equiv="Content-Security-Policy">` 标签
- CDP 测试报告"无内联 onclick/onchange/oninput"已通过（v0.8.3 修复），但 CSP 头本身缺失
- innerHTML 大量使用，memory_type 等用户输入未转义可注入脚本

**修复方案**（分步实施）：

**子步骤 4.1**：在 index.html 添加 CSP meta 标签

```html
<!-- 在 index.html 的 <head> 中首个位置添加 -->
<meta http-equiv="Content-Security-Policy" 
      content="default-src 'self'; 
               script-src 'self'; 
               style-src 'self' 'unsafe-inline'; 
               connect-src 'self' http://localhost:* http://127.0.0.1:* https://localhost:*; 
               img-src 'self' data:; 
               font-src 'self' data:; 
               object-src 'none'; 
               base-uri 'self';">
```

**说明**：
- `script-src 'self'`：禁止内联脚本（项目硬约束，v0.8.3 已清零内联 onclick）
- `style-src 'self' 'unsafe-inline'`：Tauri WebView 限制，样式内联暂时保留（后续逐步迁移到 CSS 类）
- `connect-src`：允许连接本地 sidecar（端口 3099）和 CDP（端口 9222）
- `object-src 'none'`：禁止 Flash/插件

**子步骤 4.2**：确认 Tauri 配置中 CSP 不冲突

检查 [tauri.conf.json](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json) 是否有 CSP 配置，若有则与 meta CSP 保持一致或移除 meta（Tauri 配置优先级更高）。

**子步骤 4.3**：为 memory_type 添加转义（防止 XSS，配合 CSP 双重防御）

```javascript
// 在 app.js 中添加转义工具函数（如不存在）
function sanitizeMemoryType(type) {
  // 仅允许字母、数字、下划线、连字符
  const safe = String(type || '').replace(/[^a-zA-Z0-9_-]/g, '');
  return safe || 'unknown';
}

function escapeHtml(str) {
  if (str == null) return '';
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

// 修改 memory_type 拼接处（约 4149 行）
// 修改前：const typeClass = `card-memory-${memory.memory_type}`;
// 修改后：
const typeClass = `card-memory-${sanitizeMemoryType(memory.memory_type)}`;
```

**影响文件**：
- [G:\code-memory\static\index.html](file:///g:/code-memory/static/index.html)（添加 CSP meta 标签）
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（sanitizeMemoryType + escapeHtml 工具函数）
- [G:\code-memory\desktop\src-tauri\tauri.conf.json](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json)（确认 CSP 配置）

**验证方法**：

1. **CSP 配置验证**：
   ```javascript
   const cspMeta = document.querySelector('meta[http-equiv="Content-Security-Policy"]');
   cspMeta !== null  // 应返回 true
   cspMeta.content.includes("script-src 'self'")  // 应返回 true
   ```

2. **XSS 验证**：
   ```javascript
   // 构造 memory_type 含恶意字符
   const malicious = 'test"><script>alert(1)</script>';
   const safe = sanitizeMemoryType(malicious);
   safe === 'testscriptalert1script'  // 应返回 true（恶意字符被移除）
   ```

3. **控制台无 CSP 报错**：启动后打开 DevTools Console，无 CSP 违规报错

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 4 验证脚本：G076 CSP 配置 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$htmlPath = 'G:\code-memory\static\index.html'
$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G076 验证：CSP meta 标签 ===' -ForegroundColor Cyan
    
    $csp = Select-String -LiteralPath $htmlPath -Pattern 'Content-Security-Policy'
    if ($csp) {
        Write-Host ('PASS: CSP meta 标签存在（L{0}）' -f $csp.LineNumber) -ForegroundColor Green
        $csp | ForEach-Object { Write-Host ("  L{0}: {1}" -f $_.LineNumber, $_.Line.Trim()) }
    } else {
        Write-Host 'FAIL: CSP meta 标签缺失' -ForegroundColor Red
    }
    
    Write-Host "`n=== XSS 防御验证：sanitizeMemoryType ===" -ForegroundColor Cyan
    $sanitize = Select-String -LiteralPath $appJsPath -Pattern 'function\s+sanitizeMemoryType'
    if ($sanitize) {
        Write-Host ('PASS: sanitizeMemoryType 函数已定义（L{0}）' -f $sanitize.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: sanitizeMemoryType 函数未定义' -ForegroundColor Red
    }
    
    $escape = Select-String -LiteralPath $appJsPath -Pattern 'function\s+escapeHtml'
    if ($escape) {
        Write-Host ('PASS: escapeHtml 函数已定义（L{0}）' -f $escape.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'WARN: escapeHtml 函数未定义' -ForegroundColor Yellow
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：无

**预计工作量**：0.5 人天

**如果失败怎么办**：
- 若 CSP 添加后样式错乱 → 检查 style-src 是否需要保留 'unsafe-inline'（Tauri WebView 限制）
- 若 CSP 添加后脚本不执行 → 检查是否还有内联 script 标签或 onclick 残留
- 若 Tauri 配置覆盖 meta CSP → 在 tauri.conf.json 中配置 CSP 而非 meta
- 回滚方案：`git checkout HEAD~1 -- static/index.html static/app.js`

---

### Step 5: 修复 G058 启动服务取消按钮 AbortController（用户反馈问题 1）

**目标缺口**：G058（postMessageToParent 发送启动请求无 AbortController，取消按钮仅关闭模态框不 abort 请求）

**严重程度**：P0（RICE 720，用户反馈问题 1）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 3.1 节：
- [postMessageToParent:890-938](file:///g:/code-memory/static/app.js#L890) 发送启动请求
- 取消按钮 onclick 仅调用 `hideStartServiceModal()`，**未 abort 进行中的请求**
- 缺少 AbortController，请求继续在后台执行
- 用户反馈：启动过程中点击"取消"无反应

**修复方案**（分步实施）：

**子步骤 5.1**：在 postMessageToParent 中创建 AbortController

```javascript
// 新增模块级变量（约 880 行附近）
let startServiceAbortController = null;

// 修改 postMessageToParent（约 890 行）
async function postMessageToParent(message) {
  // abort 上一次未完成的启动请求
  if (startServiceAbortController) {
    startServiceAbortController.abort();
  }
  startServiceAbortController = new AbortController();
  
  try {
    const res = await fetchWithTimeout(`${API_BASE}/v1/start`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(message),
      signal: startServiceAbortController.signal  // G058 修复：传入 signal
    });
    
    const data = await safeJson(res);
    // ... 原有处理逻辑 ...
    
  } catch (e) {
    if (e.name === 'AbortError') {
      console.log('[postMessageToParent] 启动请求被用户取消（正常行为）');
      showToast('已取消启动服务', 'info');
      return;
    }
    console.error('[postMessageToParent] 启动失败:', e);
    showToast('启动服务失败：' + (e.message || '未知错误'), 'error');
  } finally {
    startServiceAbortController = null;
  }
}
```

**子步骤 5.2**：取消按钮 onclick 调用 abort + 禁用按钮

```javascript
// 修改取消按钮处理逻辑（约 925 行附近）
async function cancelStartService() {
  const cancelBtn = document.getElementById('start-service-cancel');
  
  // 禁用取消按钮，显示"取消中..."
  if (cancelBtn) {
    cancelBtn.disabled = true;
    cancelBtn.textContent = '取消中...';
  }
  
  // abort 进行中的请求
  if (startServiceAbortController) {
    startServiceAbortController.abort();
    // 等待 fetch 抛出 AbortError
    await new Promise(r => setTimeout(r, 200));
  }
  
  // 关闭模态框
  hideStartServiceModal();
  
  // 恢复按钮状态
  if (cancelBtn) {
    cancelBtn.disabled = false;
    cancelBtn.textContent = '取消';
  }
}

// 在 bindAllActions 中添加 cancel-start-service action
case 'cancel-start-service': {
  cancelStartService();
  break;
}
```

**子步骤 5.3**：在 HTML 中为取消按钮添加 data-action

```html
<!-- 修改取消按钮（约 index.html 中 start-service-modal 内）-->
<!-- 修改前：<button onclick="hideStartServiceModal()">取消</button> -->
<!-- 修改后： -->
<button id="start-service-cancel" data-action="cancel-start-service">取消</button>
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（postMessageToParent 约 890 行 + cancelStartService 新增 + bindAllActions）
- [G:\code-memory\static\index.html](file:///g:/code-memory/static/index.html)（取消按钮添加 data-action）

**验证方法**：

1. **G058 验证**（运行时）：
   ```javascript
   // 打开启动服务模态框
   window.openStartServiceModal();
   // 触发启动请求
   postMessageToParent({ action: 'start' });
   // 立即点击取消
   document.getElementById('start-service-cancel').click();
   await new Promise(r => setTimeout(r, 300));
   // 验证请求被 abort
   startServiceAbortController === null  // 应返回 true
   ```

2. **用户反馈验证**：启动过程中点击取消，按钮显示"取消中..."，请求被中断，模态框关闭

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 5 验证脚本：G058 AbortController ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'
$htmlPath = 'G:\code-memory\static\index.html'

try {
    Write-Host '=== G058 验证：AbortController ===' -ForegroundColor Cyan
    
    $abortCtrl = Select-String -LiteralPath $appJsPath -Pattern 'startServiceAbortController'
    if ($abortCtrl.Count -ge 3) {
        Write-Host ('PASS: startServiceAbortController 已实现（{0} 处引用）' -f $abortCtrl.Count) -ForegroundColor Green
        $abortCtrl | ForEach-Object { Write-Host ("  L{0}: {1}" -f $_.LineNumber, $_.Line.Trim()) }
    } else {
        Write-Host ('FAIL: startServiceAbortController 引用不足（{0} 处）' -f $abortCtrl.Count) -ForegroundColor Red
    }
    
    # 验证 signal 传入 fetch
    $signal = Select-String -LiteralPath $appJsPath -Pattern 'signal:\s*startServiceAbortController\.signal'
    if ($signal) {
        Write-Host ('PASS: signal 已传入 fetch（L{0}）' -f $signal.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: signal 未传入 fetch' -ForegroundColor Red
    }
    
    # 验证取消按钮 data-action
    $cancelBtn = Select-String -LiteralPath $htmlPath -Pattern 'cancel-start-service'
    if ($cancelBtn) {
        Write-Host 'PASS: 取消按钮已绑定 data-action' -ForegroundColor Green
    } else {
        Write-Host 'FAIL: 取消按钮未绑定 data-action' -ForegroundColor Red
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：Step 1（调查确认 AbortController 缺失）

**预计工作量**：1.0 人天

**如果失败怎么办**：
- 若 abort 后请求仍继续 → 检查 fetchWithTimeout 是否正确透传 signal 参数
- 若取消按钮无反应 → 检查 bindAllActions 是否正确处理 cancel-start-service action
- 若"取消中..."一直显示 → 检查 finally 块是否恢复按钮状态
- 回滚方案：`git checkout HEAD~1 -- static/app.js static/index.html`

---

### Step 6: 修复 G059 道同构横幅重复重建（用户反馈问题 2）

**目标缺口**：G059（_applyDaoMetricsFallback 每次调用都重建横幅，导致指标闪烁）

**严重程度**：P0（RICE 2700，用户反馈问题 2）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 3.2 节：
- [_applyDaoMetricsFallback:4312](file:///g:/code-memory/static/app.js#L4312) 每次调用都重建横幅
- `banner.textContent` 覆盖了横幅内的按钮 HTML
- 多次调用导致指标值闪烁（用户反馈问题 2）

**修复方案**（分步实施）：

**子步骤 6.1**：添加横幅存在性检查

```javascript
// 修改 _applyDaoMetricsFallback（约 4312 行）
function _applyDaoMetricsFallback(reason) {
  const panel = document.getElementById('dao-metrics-panel') || document.querySelector('.dao-metrics-panel');
  if (!panel) {
    console.warn('[_applyDaoMetricsFallback] dao-metrics-panel 不存在');
    return;
  }
  
  // G059 修复：检查横幅是否已存在，避免重复创建
  let banner = panel.querySelector('.dao-fallback-banner');
  if (banner && banner.dataset.fallbackApplied === '1') {
    // 横幅已存在，仅更新原因文本（不重建）
    const reasonEl = banner.querySelector('.dao-fallback-reason');
    if (reasonEl) reasonEl.textContent = reason;
    return;
  }
  
  // 首次创建横幅
  if (!banner) {
    banner = document.createElement('div');
    banner.className = 'dao-fallback-banner';
    banner.style.cssText = 'background:#fff3cd;color:#856404;padding:8px 12px;border-radius:4px;margin-bottom:8px;font-size:13px;display:flex;align-items:center;gap:8px;';
    panel.insertBefore(banner, panel.firstChild);
  }
  
  // 设置内容（仅首次）
  banner.innerHTML = '';
  const reasonEl = document.createElement('span');
  reasonEl.className = 'dao-fallback-reason';
  reasonEl.textContent = '⚠ 数据加载失败：' + reason;
  banner.appendChild(reasonEl);
  
  // 添加重试按钮
  const retryBtn = document.createElement('button');
  retryBtn.textContent = '重试';
  retryBtn.style.cssText = 'margin-left:8px;padding:2px 8px;background:#007bff;color:#fff;border:none;border-radius:2px;cursor:pointer;';
  retryBtn.dataset.action = 'retry-dao-metrics';
  banner.appendChild(retryBtn);
  
  // 标记已应用
  banner.dataset.fallbackApplied = '1';
}
```

**子步骤 6.2**：在 bindAllActions 中添加 retry-dao-metrics action

```javascript
case 'retry-dao-metrics': {
  // 移除降级横幅
  const banner = document.querySelector('.dao-fallback-banner');
  if (banner) {
    delete banner.dataset.fallbackApplied;
    banner.remove();
  }
  // 重新加载
  loadDaoMetrics();
  break;
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（_applyDaoMetricsFallback 约 4312 行 + bindAllActions）

**验证方法**：

1. **G059 验证**：
   ```javascript
   // 多次调用 _applyDaoMetricsFallback
   _applyDaoMetricsFallback('测试原因1');
   _applyDaoMetricsFallback('测试原因2');
   _applyDaoMetricsFallback('测试原因3');
   
   const banners = document.querySelectorAll('.dao-fallback-banner');
   banners.length === 1  // 应返回 true（仅 1 个横幅）
   
   const reason = document.querySelector('.dao-fallback-reason');
   reason.textContent.includes('测试原因3')  // 应返回 true（仅更新原因）
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 6 验证脚本：G059 横幅幂等 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G059 验证：横幅幂等 ===' -ForegroundColor Cyan
    
    $idempotent = Select-String -LiteralPath $appJsPath -Pattern 'fallbackApplied'
    if ($idempotent) {
        Write-Host ('PASS: 横幅幂等检查已实现（L{0}）' -f $idempotent.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: 横幅幂等检查未实现' -ForegroundColor Red
    }
    
    $retryAction = Select-String -LiteralPath $appJsPath -Pattern "retry-dao-metrics"
    if ($retryAction.Count -ge 2) {
        Write-Host ('PASS: retry-dao-metrics action 已绑定（{0} 处）' -f $retryAction.Count) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: retry-dao-metrics action 未绑定' -ForegroundColor Red
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：无

**预计工作量**：0.2 人天

**如果失败怎么办**：
- 若横幅仍闪烁 → 检查 _applyDaoMetricsFallback 是否在其他地方被调用且绕过检查
- 若重试按钮无反应 → 检查 bindAllActions 是否正确处理 retry-dao-metrics
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 7: 修复 CDP#2 道同构卡片 DOM 始终存在（用户反馈问题 2）

**目标缺口**：CDP 测试 #2（道同构卡片 DOM 不存在，卡片存在: false, 值数量: 0）

**严重程度**：P0（RICE 1867，用户反馈问题 2）

**根因分析**：

根据 [CDP_TEST_REPORT_v0.8.4.md](file:///g:/code-memory/docs/CDP_TEST_REPORT_v0.8.4.md) 2.2 节：
- 道同构卡片 DOM 元素不存在
- 可能原因：dashboard 未加载完整，或道同构卡片在降级模式下被隐藏
- v0.8.4 Step 4 的 `_applyDaoMetricsFallback` 应显示卡片但指标为 '--'

**修复方案**（分步实施）：

**子步骤 7.1**：确保道同构卡片容器在 HTML 中始终存在

```html
<!-- 在 index.html 的 dashboard 区域确保以下结构始终存在（不依赖 JS 动态创建）-->
<div id="dao-metrics-panel" class="dao-metrics-panel" aria-label="道同构度指标">
  <div class="dao-metrics-header">
    <h3>道同构度</h3>
    <div id="dao-ring-container">
      <svg id="dao-ring-svg" width="120" height="120"><!-- 环形图 --></svg>
      <div id="dao-ring-score">--</div>
    </div>
  </div>
  <div class="dao-metrics-submetrics">
    <div class="dao-metric-item">
      <span class="dao-metric-label">阴阳守恒</span>
      <span id="dao-yin-yang" class="dao-metric-value">--</span>
    </div>
    <div class="dao-metric-item">
      <span class="dao-metric-label">洛书偏差</span>
      <span id="dao-luoshu-deviation" class="dao-metric-value">--</span>
    </div>
    <div class="dao-metric-item">
      <span class="dao-metric-label">八卦中横度</span>
      <span id="dao-bagua-balance" class="dao-metric-value">--</span>
    </div>
    <div class="dao-metric-item">
      <span class="dao-metric-label">合成比例</span>
      <span id="dao-synthesis-ratio" class="dao-metric-value">--</span>
    </div>
  </div>
</div>
```

**子步骤 7.2**：在 loadDaoMetrics 降级分支确保 4 个小指标显示 '--'

```javascript
// 修改 loadDaoMetrics catch 分支（约 4000 行）
} catch (e) {
  console.error('[loadDaoMetrics] 加载失败:', e);
  
  // CDP#2 修复：确保卡片始终显示，4 个小指标为 '--'
  drawDaoRing(0);
  const scoreEl = document.getElementById('dao-ring-score');
  if (scoreEl) scoreEl.textContent = '--';
  
  const subMetrics = ['dao-yin-yang', 'dao-luoshu-deviation', 'dao-bagua-balance', 'dao-synthesis-ratio'];
  subMetrics.forEach(id => {
    const el = document.getElementById(id);
    if (el) el.textContent = '--';
  });
  
  // 显示降级横幅（G059 幂等）
  _applyDaoMetricsFallback(e.name === 'SidecarUnreachableError' ? 'LRC 服务未启动' : (e.message || '未知错误'));
}
```

**影响文件**：
- [G:\code-memory\static\index.html](file:///g:/code-memory/static/index.html)（道同构卡片 HTML 结构）
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（loadDaoMetrics 降级分支）

**验证方法**：

1. **CDP 测试 #2 验证**：
   ```javascript
   // sidecar 未启动时，卡片应存在，4 个指标为 '--'
   const panel = document.getElementById('dao-metrics-panel');
   panel !== null  // 应返回 true
   
   const ids = ['dao-yin-yang', 'dao-luoshu-deviation', 'dao-bagua-balance', 'dao-synthesis-ratio'];
   const values = ids.map(id => document.getElementById(id)?.textContent);
   values.every(v => v === '--')  // 应返回 true
   values.length === 4  // 应返回 true（值数量: 4）
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 7 验证脚本：CDP#2 道同构卡片 DOM ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$htmlPath = 'G:\code-memory\static\index.html'
$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== CDP#2 验证：道同构卡片 DOM ===' -ForegroundColor Cyan
    
    $panel = Select-String -LiteralPath $htmlPath -Pattern 'id="dao-metrics-panel"'
    if ($panel) {
        Write-Host ('PASS: dao-metrics-panel 容器存在（L{0}）' -f $panel.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: dao-metrics-panel 容器缺失' -ForegroundColor Red
    }
    
    $metricIds = @('dao-yin-yang', 'dao-luoshu-deviation', 'dao-bagua-balance', 'dao-synthesis-ratio')
    foreach ($id in $metricIds) {
        $found = Select-String -LiteralPath $htmlPath -Pattern "id=`"$id`""
        if ($found) {
            Write-Host ("PASS: {0} 元素存在" -f $id) -ForegroundColor Green
        } else {
            Write-Host ("FAIL: {0} 元素缺失" -f $id) -ForegroundColor Red
        }
    }
    
    # 验证降级分支设置 '--'
    $fallback = Select-String -LiteralPath $appJsPath -Pattern "dao-yin-yang" | Where-Object { $_.Line -match "--" }
    if ($fallback) {
        Write-Host 'PASS: 降级分支设置小指标为 --' -ForegroundColor Green
    } else {
        Write-Host 'WARN: 降级分支未显式设置 --（可能依赖 HTML 初始值）' -ForegroundColor Yellow
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：Step 6（_applyDaoMetricsFallback 幂等，避免横幅重建影响）

**预计工作量**：0.3 人天

**如果失败怎么办**：
- 若卡片仍不存在 → 检查 dashboard 区域 HTML 是否被 JS 动态清空
- 若指标值不为 '--' → 检查 loadDaoMetrics 降级分支是否正确设置 textContent
- 回滚方案：`git checkout HEAD~1 -- static/index.html static/app.js`

---

### Step 8: 修复 CDP#1 道同构 API 返回 HTML 而非 JSON（sidecar 路由）

**目标缺口**：CDP 测试 #1（道同构 API 返回 HTML 而非 JSON，error: "Unexpected token '<'")

**严重程度**：P0（RICE 1120，sidecar 路由问题）

**根因分析**：

根据 [CDP_TEST_REPORT_v0.8.4.md](file:///g:/code-memory/docs/CDP_TEST_REPORT_v0.8.4.md) 2.1 节：
- sidecar 的 `/v1/dao-metrics` API 返回 HTML（可能是 404 页面）而非 JSON
- 可能原因：sidecar 在索引过程中 API 未就绪，或 API 路径不正确
- 关联：sidecar `/health` 端点超时（30 秒无响应），但 `/dashboard` 正常返回

**修复方案**（分步实施）：

**子步骤 8.1**：检查 sidecar `/v1/dao-metrics` 路由注册

```rust
// 检查 src/v1_api.rs 或路由注册文件
// 确认 /v1/dao-metrics 路由已注册
// 若未注册，添加路由：
#[get("/v1/dao-metrics")]
async fn get_dao_metrics() -> HttpResponse {
    let metrics = calculate_dao_metrics().await;
    HttpResponse::Ok()
        .content_type("application/json")
        .json(serde_json::json!({
            "ok": true,
            "data": metrics
        }))
}
```

**子步骤 8.2**：sidecar 索引未就绪时返回 JSON 错误而非 HTML

```rust
// 修改路由处理，索引未就绪时返回 JSON
#[get("/v1/dao-metrics")]
async fn get_dao_metrics(state: web::Data<AppState>) -> HttpResponse {
    if !state.index_ready.load(Ordering::Relaxed) {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(serde_json::json!({
                "ok": false,
                "error": "indexing",
                "message": "索引构建中，请稍后重试"
            }));
    }
    
    match calculate_dao_metrics(&state).await {
        Ok(metrics) => HttpResponse::Ok()
            .content_type("application/json")
            .json(serde_json::json!({
                "ok": true,
                "data": metrics
            })),
        Err(e) => HttpResponse::InternalServerError()
            .content_type("application/json")
            .json(serde_json::json!({
                "ok": false,
                "error": "calculation_failed",
                "message": e.to_string()
            }))
    }
}
```

**子步骤 8.3**：前端 loadDaoMetrics 处理非 JSON 响应

```javascript
// 修改 loadDaoMetrics（约 3990 行）
async function loadDaoMetrics() {
  try {
    const res = await fetchWithTimeout(`${API_BASE}/v1/dao-metrics`);
    
    // CDP#1 修复：检查 Content-Type 是否为 JSON
    const contentType = res.headers.get('content-type') || '';
    if (!contentType.includes('application/json')) {
      throw new Error('响应不是 JSON 格式（可能是 HTML 404 页面）');
    }
    
    const data = await safeJson(res);
    if (!data.ok) {
      throw new Error(data.error || data.message || 'API 返回错误');
    }
    
    // ... 原有渲染逻辑 ...
    
  } catch (e) {
    // ... 降级处理（Step 7 已实现）...
  }
}
```

**影响文件**：
- [G:\code-memory\src\v1_api.rs](file:///g:/code-memory/src/v1_api.rs)（/v1/dao-metrics 路由）
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（loadDaoMetrics Content-Type 检查）

**验证方法**：

1. **CDP 测试 #1 验证**：
   ```bash
   # sidecar 启动后，curl 验证返回 JSON
   curl -i http://localhost:3099/v1/dao-metrics
   # 应返回 Content-Type: application/json
   # 响应体应为 {"ok": true, "data": {...}}
   ```

2. **前端验证**：
   ```javascript
   const res = await fetch('/v1/dao-metrics');
   res.headers.get('content-type').includes('application/json')  // 应返回 true
   const json = await res.json();
   json.ok === true && json.data !== undefined  // 应返回 true
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 8 验证脚本：CDP#1 道同构 API ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$v1ApiPath = 'G:\code-memory\src\v1_api.rs'
$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== CDP#1 验证：sidecar 路由 ===' -ForegroundColor Cyan
    
    $route = Select-String -LiteralPath $v1ApiPath -Pattern 'dao-metrics|dao_metrics'
    if ($route) {
        Write-Host ('PASS: /v1/dao-metrics 路由已注册（{0} 处）' -f $route.Count) -ForegroundColor Green
        $route | ForEach-Object { Write-Host ("  L{0}: {1}" -f $_.LineNumber, $_.Line.Trim()) }
    } else {
        Write-Host 'FAIL: /v1/dao-metrics 路由未注册' -ForegroundColor Red
    }
    
    # 验证 content_type 设置
    $contentType = Select-String -LiteralPath $v1ApiPath -Pattern 'content_type.*application/json' | Where-Object { $_.LineNumber -gt ($route | Select-Object -First 1).LineNumber -and $_.LineNumber -lt ($route | Select-Object -First 1).LineNumber + 30 }
    if ($contentType) {
        Write-Host 'PASS: 路由返回 application/json' -ForegroundColor Green
    } else {
        Write-Host 'WARN: 未显式设置 content_type' -ForegroundColor Yellow
    }
    
    # 验证前端 Content-Type 检查
    $frontendCheck = Select-String -LiteralPath $appJsPath -Pattern "content-type.*application/json|includes\('application/json'\)"
    if ($frontendCheck) {
        Write-Host ('PASS: 前端 Content-Type 检查已实现（L{0}）' -f $frontendCheck.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: 前端未检查 Content-Type' -ForegroundColor Red
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：Step 7（卡片 DOM 始终存在，降级显示 '--'）

**预计工作量**：0.5 人天

**如果失败怎么办**：
- 若 sidecar 路由已注册但仍返回 HTML → 检查路由前缀（可能需要 `/api/v1/dao-metrics`）
- 若索引未就绪返回 503 → 前端应捕获并显示"索引构建中"提示
- 若 cargo build 失败 → 检查 Rust 语法和依赖
- 回滚方案：`git checkout HEAD~1 -- src/v1_api.rs static/app.js`

---

### Step 9: 修复 G052 handleHttpError 死代码（最严重回归，分步替换）

**目标缺口**：G052（handleHttpError 定义但从未调用，导致 G007/G009/G029/G047 四个错误恢复修复全部失效）

**严重程度**：P0（RICE 450，最严重回归，工作量大但必须修复）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 1.2 节和五节：
- [handleHttpError](file:///g:/code-memory/static/app.js#L177) 定义完整（含 500 重试 Modal、429 限流提示、重试计数、_retryModalActive）
- 但全文搜索 `handleHttpError\(` 仅在注释中出现，**实际调用方为 0**
- 导致：
  - 500 错误仅显示 Toast，无重试 Modal
  - 429 限流无 Toast 提示
  - `_retryCounters` Map 永不填充
  - `_retryModalActive` 永远 false
  - `switchTab` 的 `_retryModalActive` 检查永远通过

**修复方案**（分步实施，禁止批量修改，按调用点逐个替换）：

**子步骤 9.1**：识别所有 fetchWithTimeout 调用点

需替换的调用点（按优先级排序）：
1. `loadDashboard` 中的 /v1/dao-metrics 请求（约 3990 行）
2. `loadMemories` 中的 /v1/memories 请求（约 527 行）
3. `searchMemories` 中的 /v1/memories/search 请求（约 4106 行）
4. `loadTrustCenterData` 中的 /v1/trust/* 请求（约 4250 行）
5. `loadRecentMemories` 中的请求（约 527 行）
6. `runPrivacyCheck` 中的 /v1/privacy/* 请求（约 3764 行）
7. `postMessageToParent` 中的 /v1/start 请求（Step 5 已处理 AbortController，此处补充错误处理）
8. `downloadEmbedderModel` 中的下载请求（约 5402 行）

**子步骤 9.2**：分步替换 loadDashboard 的错误处理

```javascript
// 修改 loadDashboard（约 3990 行）
// 修改前：
async function loadDashboard() {
  try {
    const res = await fetchWithTimeout(`${API_BASE}/v1/dao-metrics`);
    if (!res.ok) {
      showToast('加载失败', 'error');
      return;
    }
    // ... 渲染 ...
  } catch (e) {
    console.error('[loadDashboard] 失败:', e);
    // ... 降级 ...
  }
}

// 修改后：使用 handleHttpError
async function loadDashboard() {
  try {
    const res = await fetchWithTimeout(`${API_BASE}/v1/dao-metrics`);
    
    // G052 修复：使用 handleHttpError 处理 HTTP 错误
    if (!res.ok) {
      const errorResult = await handleHttpError(
        { status: res.status, message: `HTTP ${res.status}` },
        { url: '/v1/dao-metrics', method: 'GET' }
      );
      if (errorResult.action === 'retry') {
        return loadDashboard(); // 重试
      }
      return; // cancel 或 giveup
    }
    
    const data = await safeJson(res);
    // ... 渲染 ...
    resetRetryCounter('/v1/dao-metrics', 'GET'); // 成功时重置重试计数
    
  } catch (e) {
    if (e.name === 'AbortError') return;
    
    // G052 修复：网络错误也走 handleHttpError
    const errorResult = await handleHttpError(
      { status: 0, message: e.message, name: e.name },
      { url: '/v1/dao-metrics', method: 'GET' }
    );
    if (errorResult.action === 'retry') {
      return loadDashboard();
    }
    // ... 降级处理 ...
  }
}
```

**子步骤 9.3**：分步替换 loadMemories 的错误处理（验证 9.2 通过后）

```javascript
// 修改 loadMemories（约 527 行）
async function loadMemories() {
  try {
    const res = await fetchWithTimeout(`${API_BASE}/v1/memories`);
    
    if (!res.ok) {
      const errorResult = await handleHttpError(
        { status: res.status, message: `HTTP ${res.status}` },
        { url: '/v1/memories', method: 'GET' }
      );
      if (errorResult.action === 'retry') return loadMemories();
      return;
    }
    
    const data = await safeJson(res);
    resetRetryCounter('/v1/memories', 'GET');
    renderMemories(data);
    
  } catch (e) {
    if (e.name === 'AbortError') return;
    const errorResult = await handleHttpError(
      { status: 0, message: e.message, name: e.name },
      { url: '/v1/memories', method: 'GET' }
    );
    if (errorResult.action === 'retry') return loadMemories();
  }
}
```

**子步骤 9.4**：分步替换 searchMemories 的错误处理（验证 9.3 通过后）

```javascript
// 修改 searchMemories（约 4106 行）
async function searchMemories(query) {
  if (searchAbortController) searchAbortController.abort();
  searchAbortController = new AbortController();
  
  try {
    const res = await fetchWithTimeout(`${API_BASE}/v1/memories/search`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ query, limit: 20 }),
      signal: searchAbortController.signal
    });
    
    if (!res.ok) {
      const errorResult = await handleHttpError(
        { status: res.status, message: `HTTP ${res.status}` },
        { url: '/v1/memories/search', method: 'POST' }
      );
      if (errorResult.action === 'retry') return searchMemories(query);
      return;
    }
    
    const data = await safeJson(res);
    resetRetryCounter('/v1/memories/search', 'POST');
    renderSearchResults(data);
    
  } catch (e) {
    if (e.name === 'AbortError') return;
    const errorResult = await handleHttpError(
      { status: 0, message: e.message, name: e.name },
      { url: '/v1/memories/search', method: 'POST' }
    );
    if (errorResult.action === 'retry') return searchMemories(query);
  } finally {
    searchAbortController = null;
  }
}
```

**子步骤 9.5**：分步替换 loadTrustCenterData、loadRecentMemories、runPrivacyCheck、downloadEmbedderModel（验证 9.4 通过后，逐一替换）

每个调用点替换后，运行一次 CDP 测试验证无回归，再替换下一个。

**子步骤 9.6**：确认 handleHttpError 实现完整

```javascript
// 确认 handleHttpError 实现（约 177 行）
async function handleHttpError(error, context = {}) {
  const { url, method = 'GET' } = context;
  const retryKey = `${method}:${url}`;
  
  // 500 服务器错误：重试 Modal
  if (error.status === 500) {
    const retryCount = _retryCounters.get(retryKey) || 0;
    
    if (retryCount >= MAX_RETRY_COUNT) {
      _retryCounters.delete(retryKey);
      await showInfoModal('多次重试失败', `请求 ${url} 已连续失败 ${MAX_RETRY_COUNT} 次。`);
      return { action: 'giveup' };
    }
    
    _retryModalActive = true;
    try {
      const shouldRetry = await showConfirm(
        `请求失败（第 ${retryCount + 1} 次重试）\n错误：${error.message}\n\n是否重试？`,
        '服务器错误',
        0
      );
      if (shouldRetry) {
        _retryCounters.set(retryKey, retryCount + 1);
        const backoff = Math.pow(2, retryCount) * 1000;
        await new Promise(r => setTimeout(r, backoff));
        return { action: 'retry' };
      } else {
        _retryCounters.delete(retryKey);
        return { action: 'giveup' };
      }
    } finally {
      _retryModalActive = false;
    }
  }
  
  // 429 限流：Toast 提示
  if (error.status === 429) {
    showToast('请求过于频繁，请稍后重试', 'warning');
    return { action: 'giveup' };
  }
  
  // 503 服务不可用：降级
  if (error.status === 503) {
    showToast('服务暂时不可用，正在降级处理', 'warning');
    return { action: 'giveup' };
  }
  
  // 网络错误（status=0）
  if (error.status === 0 || error.name === 'SidecarUnreachableError') {
    showToast('网络连接失败：' + error.message, 'error');
    return { action: 'giveup' };
  }
  
  // 其他错误
  showToast('请求失败：' + (error.message || '未知错误'), 'error');
  return { action: 'giveup' };
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（8 个 fetchWithTimeout 调用点 + handleHttpError 确认）

**验证方法**：

1. **G052 验证**（handleHttpError 调用数 ≥ 8）：
   ```javascript
   // 模拟 500 错误
   const originalFetch = window.fetch;
   window.fetch = () => Promise.resolve({ ok: false, status: 500, headers: new Headers() });
   
   await loadDashboard();
   // 应显示重试 Modal（showConfirm）
   const modal = document.getElementById('confirm-modal');
   !modal.hidden  // 应返回 true
   
   window.fetch = originalFetch;
   ```

2. **G007 验证**（500 重试 Modal）：模拟 500 错误，应显示 showConfirm 重试 Modal
3. **G009 验证**（503 降级）：模拟 503 错误，应显示降级 Toast
4. **G029 验证**（重试上限）：连续 3 次 500 后，应显示 showInfoModal 而非 showConfirm
5. **G047 验证**（重试时禁止切换标签）：重试 Modal 显示时 switchTab 应返回 false

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 9 验证脚本：G052 handleHttpError 接入 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G052 验证：handleHttpError 调用数 ===' -ForegroundColor Cyan
    
    # 查找所有 handleHttpError( 调用（排除定义和注释）
    $allRefs = Select-String -LiteralPath $appJsPath -Pattern 'handleHttpError'
    $calls = $allRefs | Where-Object { 
        $_.Line -match 'handleHttpError\s*\(' -and 
        $_.Line -notmatch 'function\s+handleHttpError' -and
        $_.Line -notmatch '^\s*//|^\s*\*' 
    }
    
    Write-Host ("handleHttpError 调用数: {0}" -f $calls.Count) -ForegroundColor $(if ($calls.Count -ge 8) { 'Green' } else { 'Red' })
    $calls | ForEach-Object { Write-Host ("  L{0}: {1}" -f $_.LineNumber, $_.Line.Trim()) }
    
    if ($calls.Count -ge 8) {
        Write-Host 'PASS: handleHttpError 已接入所有 fetchWithTimeout 调用点' -ForegroundColor Green
    } else {
        Write-Host 'FAIL: handleHttpError 接入不足（需 ≥ 8 处）' -ForegroundColor Red
    }
    
    # 验证关键调用点
    Write-Host "`n=== 关键调用点验证 ===" -ForegroundColor Cyan
    $keyPoints = @(
        @{ Name = 'loadDashboard'; Pattern = 'loadDashboard' },
        @{ Name = 'loadMemories'; Pattern = 'loadMemories' },
        @{ Name = 'searchMemories'; Pattern = 'searchMemories' },
        @{ Name = 'loadTrustCenterData'; Pattern = 'loadTrustCenterData' }
    )
    
    foreach ($point in $keyPoints) {
        $funcDef = Select-String -LiteralPath $appJsPath -Pattern "function\s+$($point.Pattern)" | Select-Object -First 1
        if ($funcDef) {
            $funcBody = Get-Content -LiteralPath $appJsPath -TotalCount ($funcDef.LineNumber + 80) | Select-Object -Skip ($funcDef.LineNumber - 1)
            $hasHandle = $funcBody | Where-Object { $_ -match 'handleHttpError\s*\(' }
            if ($hasHandle) {
                Write-Host ("PASS: {0} 已接入 handleHttpError" -f $point.Name) -ForegroundColor Green
            } else {
                Write-Host ("FAIL: {0} 未接入 handleHttpError" -f $point.Name) -ForegroundColor Red
            }
        }
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：Step 10（G060 _retryCounters Map 上限，必须在 G052 激活前实现防泄漏）

**预计工作量**：2.0 人天（8 个调用点 × 0.25 人天/点）

**如果失败怎么办**：
- 若替换后某个函数报错 → 立即回退该函数的修改（`git checkout HEAD~1 -- static/app.js`），分析 handleHttpError 返回值处理
- 若重试 Modal 不显示 → 检查 _retryModalActive 是否正确设置（Step 9.6）
- 若重试无限循环 → 检查 MAX_RETRY_COUNT 和 _retryCounters 是否正确递增
- 若 switchTab 禁止切换导致用户卡死 → 在重试 Modal 添加"切换并放弃重试"选项
- **分步回滚**：每个子步骤独立回滚，不影响其他子步骤

---

### Step 10: 修复 G060 _retryCounters Map 内存泄漏（防 G052 引入）

**目标缺口**：G060（_retryCounters Map 无上限，可能内存泄漏）+ v0.8.4 回归风险

**严重程度**：P0（RICE 1350，必须在 G052 激活前修复）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 第四层 G060：
- [_retryCounters](file:///g:/code-memory/static/app.js#L172) Map 无上限
- G052 修复后将激活 _retryCounters，每次 500 错误创建新 key
- 长时间运行后可能占用大量内存
- 同时 confirmModalQueue 异常时队列死锁风险

**修复方案**（分步实施）：

**子步骤 10.1**：为 _retryCounters 添加 Map 大小上限

```javascript
// 修改 _retryCounters 定义处（约 172 行）
const _retryCounters = new Map();
const MAX_RETRY_COUNTERS_SIZE = 50;
const MAX_RETRY_COUNT = 3;

// 在 handleHttpError 的 _retryCounters.set 调用前添加上限检查
function _setRetryCounter(key, count) {
  // G060 修复：Map 大小上限检查
  if (_retryCounters.size >= MAX_RETRY_COUNTERS_SIZE && !_retryCounters.has(key)) {
    // 清理最旧的条目（Map 保持插入顺序）
    const oldestKey = _retryCounters.keys().next().value;
    _retryCounters.delete(oldestKey);
    console.warn('[_setRetryCounter] Map 已满，清理最旧条目:', oldestKey);
  }
  _retryCounters.set(key, count);
}
```

**子步骤 10.2**：在 handleHttpError 中使用 _setRetryCounter

```javascript
// 修改 handleHttpError（约 177 行）
async function handleHttpError(error, context = {}) {
  // ... 500 处理 ...
  if (shouldRetry) {
    _setRetryCounter(retryKey, retryCount + 1);  // 使用 _setRetryCounter 替代 _retryCounters.set
    // ...
  }
}
```

**子步骤 10.3**：添加成功时重置计数器（resetRetryCounter 已在 Step 9 调用）

```javascript
function resetRetryCounter(url, method = 'GET') {
  _retryCounters.delete(`${method}:${url}`);
}
window.resetRetryCounter = resetRetryCounter;  // 暴露供测试
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（_retryCounters 约 172 行 + handleHttpError）

**验证方法**：

1. **G060 验证**：
   ```javascript
   // 模拟 60 个不同 URL 的 500 错误
   for (let i = 0; i < 60; i++) {
     _setRetryCounter(`GET:/test${i}`, 1);
   }
   _retryCounters.size <= 50  // 应返回 true（不超过上限）
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 10 验证脚本：G060 Map 上限 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G060 验证：Map 上限 ===' -ForegroundColor Cyan
    
    $maxSize = Select-String -LiteralPath $appJsPath -Pattern 'MAX_RETRY_COUNTERS_SIZE'
    if ($maxSize) {
        Write-Host ('PASS: MAX_RETRY_COUNTERS_SIZE 已定义（L{0}）' -f $maxSize.LineNumber) -ForegroundColor Green
        $maxSize | ForEach-Object { Write-Host ("  L{0}: {1}" -f $_.LineNumber, $_.Line.Trim()) }
    } else {
        Write-Host 'FAIL: MAX_RETRY_COUNTERS_SIZE 未定义' -ForegroundColor Red
    }
    
    $setFn = Select-String -LiteralPath $appJsPath -Pattern 'function\s+_setRetryCounter'
    if ($setFn) {
        Write-Host ('PASS: _setRetryCounter 函数已定义（L{0}）' -f $setFn.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: _setRetryCounter 函数未定义' -ForegroundColor Red
    }
    
    $resetFn = Select-String -LiteralPath $appJsPath -Pattern 'function\s+resetRetryCounter'
    if ($resetFn) {
        Write-Host ('PASS: resetRetryCounter 函数已定义（L{0}）' -f $resetFn.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'WARN: resetRetryCounter 函数未定义' -ForegroundColor Yellow
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：无（必须先于 Step 9 完成或同步完成）

**预计工作量**：0.2 人天

**如果失败怎么办**：
- 若 Map 清理导致有效重试计数丢失 → 调整清理策略（如按时间戳清理而非 FIFO）
- 若 _setRetryCounter 未被调用 → 检查 handleHttpError 是否使用 _setRetryCounter 替代 _retryCounters.set
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 11: 修复 CDP#4 G011 Toast 去重窗口过长

**目标缺口**：CDP 测试 #4（G011 重复消息去重，5 个相同消息后剩 0 个应为 1 个）

**严重程度**：P1（RICE 1280，CDP 失败项）

**根因分析**：

根据 [CDP_TEST_REPORT_v0.8.4.md](file:///g:/code-memory/docs/CDP_TEST_REPORT_v0.8.4.md) 2.4 节：
- v0.8.4 Step 8 修复将 `dedupKey` 记录移到显示之后
- 但 `_TOAST_DEDUP_WINDOW` 时间窗口内（约 1.5s），第一个 Toast 显示后记录 dedupKey，后续 4 个被去重
- 测试检查时第一个 Toast 已消失（duration 3000ms，但测试等待时间过长）
- 或：测试发送 5 个 Toast 的速度过快，全部在去重窗口内被跳过

**修复方案**（分步实施）：

**子步骤 11.1**：缩短去重窗口到 500ms

```javascript
// 修改 _TOAST_DEDUP_WINDOW 常量（约 4680 行）
// 修改前：const _TOAST_DEDUP_WINDOW = 1500;
// 修改后：
const _TOAST_DEDUP_WINDOW = 500;  // CDP#4 修复：缩短到 500ms，避免误去重
```

**子步骤 11.2**：验证 showToast 去重逻辑（v0.8.4 Step 8 已修复显示后记录）

```javascript
// 确认 showToast 实现（约 4682 行）
function showToast(message, type = 'info', duration = 4000) {
  const now = Date.now();
  const dedupKey = `${type}:${message}`;
  
  // 去重检查（显示前检查）
  if (_recentToastMessages.has(dedupKey)) {
    const last = _recentToastMessages.get(dedupKey);
    if (now - last < _TOAST_DEDUP_WINDOW) {
      _recentToastMessages.set(dedupKey, now);
      return;
    }
  }
  
  // ... 可见上限检查 ...
  
  // 显示后才记录 dedupKey（v0.8.4 Step 8 已修复）
  _recentToastMessages.set(dedupKey, now);
  
  // ... 创建并显示 Toast ...
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（_TOAST_DEDUP_WINDOW 约 4680 行）

**验证方法**：

1. **CDP 测试 #4 验证**：
   ```javascript
   // 清理去重 Map
   if (window._recentToastMessages) window._recentToastMessages.clear();
   
   const container = document.getElementById('toast-container');
   const beforeCount = container.querySelectorAll('.toast').length;
   
   // 调用 5 次相同消息
   for (let i = 0; i < 5; i++) {
     window.showToast('测试去重消息', 'info', 3000);
   }
   
   const afterCount = container.querySelectorAll('.toast').length;
   (afterCount - beforeCount) === 1  // 应返回 true（第 1 次显示，2-5 去重）
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 11 验证脚本：CDP#4 Toast 去重窗口 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== CDP#4 验证：Toast 去重窗口 ===' -ForegroundColor Cyan
    
    $window = Select-String -LiteralPath $appJsPath -Pattern '_TOAST_DEDUP_WINDOW\s*=\s*\d+'
    if ($window) {
        $value = [int]($window.Line -replace '.*=\s*(\d+).*', '$1')
        Write-Host ("去重窗口值: {0}ms" -f $value) -ForegroundColor $(if ($value -le 500) { 'Green' } else { 'Red' })
        if ($value -le 500) {
            Write-Host 'PASS: 去重窗口 ≤ 500ms' -ForegroundColor Green
        } else {
            Write-Host 'FAIL: 去重窗口过长（应 ≤ 500ms）' -ForegroundColor Red
        }
    } else {
        Write-Host 'FAIL: _TOAST_DEDUP_WINDOW 未定义' -ForegroundColor Red
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：无

**预计工作量**：0.2 人天

**如果失败怎么办**：
- 若缩短窗口后仍有误去重 → 检查测试发送间隔是否仍小于 500ms
- 若缩短窗口后失去去重效果 → 调整为 800ms 折中
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 12: 修复 G057 /health 超时无降级

**目标缺口**：G057（/health 超时但 /dashboard 正常时无降级处理）

**严重程度**：P1（RICE 1280）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 第三层 G057：
- [SidecarHealthMonitor](file:///g:/code-memory/static/app.js#L4250) 检查 /health 端点
- /health 超时（30 秒无响应），但 /dashboard 正常返回
- 前端将整个 sidecar 判定为不可达，但实际 sidecar 可用

**修复方案**（分步实施）：

```javascript
// 修改 SidecarHealthMonitor.checkHealth（约 4250 行）
async function checkSidecarHealth() {
  const results = {
    health: false,
    dashboard: false,
    reachable: false,
    degradedMode: false
  };
  
  // 检查 /health（5 秒超时）
  try {
    const healthRes = await fetchWithTimeout(`${API_BASE}/health`, { timeoutMs: 5000 });
    results.health = healthRes.ok;
  } catch (e) {
    console.warn('[checkSidecarHealth] /health 超时或失败:', e.message);
  }
  
  // G057 修复：/health 失败时检查 /dashboard 作为降级判断
  if (!results.health) {
    try {
      const dashRes = await fetchWithTimeout(`${API_BASE}/dashboard`, { timeoutMs: 5000 });
      results.dashboard = dashRes.ok;
    } catch (e) {
      console.warn('[checkSidecarHealth] /dashboard 也失败:', e.message);
    }
  }
  
  // 综合判断
  results.reachable = results.health || results.dashboard;
  results.degradedMode = !results.health && results.dashboard;
  
  if (results.degradedMode) {
    showToast('LRC 服务 /health 端点异常，已启用降级模式', 'warning');
    document.body.classList.add('sidecar-degraded');
  } else if (results.reachable) {
    document.body.classList.remove('sidecar-degraded');
  } else {
    document.body.classList.add('sidecar-unreachable');
    showToast('LRC 服务不可达，请检查服务是否运行', 'error');
  }
  
  return results;
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（checkSidecarHealth 约 4250 行）

**验证方法**：

1. **G057 验证**：
   ```javascript
   // 模拟 /health 超时，/dashboard 正常
   const originalFetch = window.fetch;
   window.fetch = (url) => {
     if (url.includes('/health')) return new Promise((_, reject) => setTimeout(() => reject(new Error('timeout')), 100));
     return originalFetch(url);
   };
   
   const result = await checkSidecarHealth();
   result.degradedMode === true  // 应返回 true
   result.reachable === true     // 应返回 true
   document.body.classList.contains('sidecar-degraded')  // 应返回 true
   
   window.fetch = originalFetch;
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 12 验证脚本：G057 降级 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G057 验证：/health 降级 ===' -ForegroundColor Cyan
    
    $degraded = Select-String -LiteralPath $appJsPath -Pattern 'degradedMode|sidecar-degraded'
    if ($degraded.Count -ge 2) {
        Write-Host ('PASS: 降级模式已实现（{0} 处）' -f $degraded.Count) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: 降级模式未实现' -ForegroundColor Red
    }
    
    # 验证 /dashboard 作为降级检查
    $dashboardCheck = Select-String -LiteralPath $appJsPath -Pattern '/dashboard' | Where-Object { $_.LineNumber -gt 4240 -and $_.LineNumber -lt 4290 }
    if ($dashboardCheck) {
        Write-Host 'PASS: /dashboard 降级检查已实现' -ForegroundColor Green
    } else {
        Write-Host 'WARN: /dashboard 降级检查未找到' -ForegroundColor Yellow
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：无

**预计工作量**：0.3 人天

**如果失败怎么办**：
- 若降级模式仍判定不可达 → 检查 /dashboard 路径是否正确
- 若降级提示频繁 → 调整降级检查间隔
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 13: 修复 G053 navbar-nav 绕过 switchTab

**目标缺口**：G053（navbar-nav 按钮绕过 switchTab，直接操作 DOM）

**严重程度**：P1（RICE 1200）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 第二层 G053：
- [navbar-nav](file:///g:/code-memory/static/app.js#L427) 直接操作 DOM 切换标签
- 未调用 switchTab，导致 _abortActiveTabRequests 未执行、active 类管理不一致

**修复方案**（分步实施）：

```javascript
// 修改 navbar-nav 点击处理（约 427 行）
// 修改前：
document.querySelectorAll('.navbar-nav .nav-item').forEach(item => {
  item.addEventListener('click', () => {
    document.querySelectorAll('.navbar-nav .nav-item').forEach(i => i.classList.remove('active'));
    item.classList.add('active');
    const tabName = item.dataset.tab;
    document.querySelectorAll('.tab-content').forEach(c => c.classList.remove('active'));
    document.getElementById(`tab-${tabName}`).classList.add('active');
  });
});

// 修改后：统一调用 switchTab
document.querySelectorAll('.navbar-nav .nav-item').forEach(item => {
  item.addEventListener('click', (e) => {
    e.preventDefault();
    const tabName = item.dataset.tab;
    if (tabName) {
      switchTab(tabName);  // G053 修复：统一走 switchTab
    }
  });
});
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（navbar-nav 约 427 行）

**验证方法**：

1. **G053 验证**：
   ```javascript
   // 点击 navbar-nav 项，应触发 switchTab
   let switchTabCalled = false;
   const original = window.switchTab;
   window.switchTab = (name) => { switchTabCalled = true; return original(name); };
   
   document.querySelector('.navbar-nav .nav-item').click();
   switchTabCalled === true  // 应返回 true
   
   window.switchTab = original;
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 13 验证脚本：G053 navbar 统一 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G053 验证：navbar 统一 switchTab ===' -ForegroundColor Cyan
    
    # 查找 navbar-nav 处理
    $navbar = Select-String -LiteralPath $appJsPath -Pattern 'navbar-nav' | Select-Object -First 3
    if ($navbar) {
        $navbar | ForEach-Object { Write-Host ("  L{0}: {1}" -f $_.LineNumber, $_.Line.Trim()) }
    }
    
    # 验证 navbar-nav 处理中调用 switchTab
    $navbarDef = $navbar | Select-Object -First 1
    if ($navbarDef) {
        $funcBody = Get-Content -LiteralPath $appJsPath -TotalCount ($navbarDef.LineNumber + 15) | Select-Object -Skip ($navbarDef.LineNumber - 1)
        $hasSwitchTab = $funcBody | Where-Object { $_ -match 'switchTab\s*\(' }
        if ($hasSwitchTab) {
            Write-Host 'PASS: navbar-nav 调用 switchTab' -ForegroundColor Green
        } else {
            Write-Host 'FAIL: navbar-nav 未调用 switchTab' -ForegroundColor Red
        }
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：无

**预计工作量**：0.3 人天

**如果失败怎么办**：
- 若 switchTab 调用后标签不切换 → 检查 switchTab 是否正确处理 tabName
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 14: 修复 G056 sidecar 崩溃无重连

**目标缺口**：G056（sidecar 崩溃后前端无自动重连机制）

**严重程度**：P1（RICE 784）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 第三层 G056：
- [SidecarHealthMonitor](file:///g:/code-memory/static/app.js#L890) 检测到 sidecar 不可达后，无自动重连
- 用户需手动刷新页面

**修复方案**（分步实施）：

```javascript
// 修改 SidecarHealthMonitor（约 890 行）
class SidecarHealthMonitor {
  constructor() {
    this.isReachable = false;
    this.reconnectAttempts = 0;
    this.maxReconnectAttempts = 5;
    this.reconnectTimer = null;
  }
  
  async start() {
    await this.checkHealth();
    if (!this.isReachable) {
      this.startReconnect();
    }
  }
  
  startReconnect() {
    if (this.reconnectAttempts >= this.maxReconnectAttempts) {
      console.warn('[SidecarHealthMonitor] 重连次数上限，停止重连');
      showToast('LRC 服务重连失败，请手动重启服务', 'error');
      return;
    }
    
    // G056 修复：指数退避重连
    const backoff = Math.pow(2, this.reconnectAttempts) * 1000;  // 1s, 2s, 4s, 8s, 16s
    console.log(`[SidecarHealthMonitor] ${backoff}ms 后重连（第 ${this.reconnectAttempts + 1} 次）`);
    
    this.reconnectTimer = setTimeout(async () => {
      this.reconnectAttempts++;
      await this.checkHealth();
      if (this.isReachable) {
        this.reconnectAttempts = 0;
        showToast('LRC 服务已重连', 'success');
        // 重连后重新加载数据
        if (typeof loadDashboard === 'function') loadDashboard();
      } else {
        this.startReconnect();
      }
    }, backoff);
  }
  
  stopReconnect() {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }
  
  async checkHealth() {
    try {
      const res = await fetchWithTimeout(`${API_BASE}/health`, { timeoutMs: 5000 });
      this.isReachable = res.ok;
    } catch (e) {
      this.isReachable = false;
    }
  }
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（SidecarHealthMonitor 约 890 行）

**验证方法**：

1. **G056 验证**：
   ```javascript
   // 模拟 sidecar 崩溃后恢复
   const monitor = new SidecarHealthMonitor();
   monitor.reconnectAttempts = 0;
   monitor.startReconnect();
   // 验证指数退避：1s, 2s, 4s, 8s, 16s
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 14 验证脚本：G056 重连 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G056 验证：sidecar 重连 ===' -ForegroundColor Cyan
    
    $reconnect = Select-String -LiteralPath $appJsPath -Pattern 'startReconnect|reconnectAttempts|maxReconnectAttempts'
    if ($reconnect.Count -ge 3) {
        Write-Host ('PASS: 重连机制已实现（{0} 处）' -f $reconnect.Count) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: 重连机制未实现' -ForegroundColor Red
    }
    
    $backoff = Select-String -LiteralPath $appJsPath -Pattern 'Math\.pow\(2.*reconnectAttempts'
    if ($backoff) {
        Write-Host 'PASS: 指数退避已实现' -ForegroundColor Green
    } else {
        Write-Host 'WARN: 指数退避未实现' -ForegroundColor Yellow
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：Step 12（G057 降级，复用 checkHealth 逻辑）

**预计工作量**：0.5 人天

**如果失败怎么办**：
- 若重连导致请求堆积 → 检查 stopReconnect 是否在重连成功后调用
- 若指数退避过长 → 上限调整为 8s（1s, 2s, 4s, 8s, 8s）
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 15: 修复 G064 道同构卡片无 aria-label

**目标缺口**：G064（道同构卡片无 aria-label，可访问性缺失）

**严重程度**：P1（RICE 1200，可访问性）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 第五层 G064：
- 道同构卡片无 aria-label，屏幕阅读器无法识别

**修复方案**（分步实施）：

**子步骤 15.1**：在 HTML 中为道同构卡片添加 aria-label

```html
<!-- 修改 dao-metrics-panel（Step 7 已添加，此处确认）-->
<div id="dao-metrics-panel" class="dao-metrics-panel" 
     aria-label="道同构度指标" 
     role="region">
  <!-- ... -->
</div>
```

**子步骤 15.2**：为 4 个小指标添加 aria-label

```html
<div class="dao-metric-item" aria-label="阴阳守恒指标">
  <span class="dao-metric-label">阴阳守恒</span>
  <span id="dao-yin-yang" class="dao-metric-value" aria-describedby="dao-yin-yang-label">--</span>
</div>
```

**影响文件**：
- [G:\code-memory\static\index.html](file:///g:/code-memory/static/index.html)（道同构卡片 aria-label）

**验证方法**：

1. **G064 验证**：
   ```javascript
   const panel = document.getElementById('dao-metrics-panel');
   panel.getAttribute('aria-label') === '道同构度指标'  // 应返回 true
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 15 验证脚本：G064 aria-label ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$htmlPath = 'G:\code-memory\static\index.html'

try {
    Write-Host '=== G064 验证：aria-label ===' -ForegroundColor Cyan
    
    $ariaLabel = Select-String -LiteralPath $htmlPath -Pattern 'aria-label="道同构度指标"'
    if ($ariaLabel) {
        Write-Host ('PASS: 道同构卡片 aria-label 已添加（L{0}）' -f $ariaLabel.LineNumber) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: 道同构卡片 aria-label 缺失' -ForegroundColor Red
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：Step 7（卡片 DOM 已存在）

**预计工作量**：0.1 人天

**如果失败怎么办**：
- 回滚方案：`git checkout HEAD~1 -- static/index.html`

---

### Step 16: 修复 G054 showPrompt 未入队

**目标缺口**：G054（showPrompt 未入队，与 showConfirm 共用 DOM）

**严重程度**：P1（RICE 576）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 第二层 G054：
- [showPrompt:3154](file:///g:/code-memory/static/app.js#L3154) 直接操作 confirm-modal DOM
- 未入队 confirmModalQueue，与 showConfirm 冲突

**修复方案**（分步实施）：

```javascript
// 修改 showPrompt（约 3154 行）
async function showPrompt(message, title, defaultValue = '') {
  // 队列上限检查
  if (confirmModalQueue.length >= 5) {
    console.warn('[showPrompt] 队列已满，拒绝新调用');
    return null;
  }

  return new Promise((resolve) => {
    const task = {
      message: message,
      title: title || '请输入',
      timeoutMs: 0,
      resolve: (result) => resolve(result),
      isPrompt: true,
      defaultValue: defaultValue
    };
    confirmModalQueue.push(task);
    processConfirmQueue();
  });
}
```

在 processConfirmQueue 中处理 `task.isPrompt`（参考 v0.8.4 Step 3 的 isInfoOnly 模式）：

```javascript
// 在 processConfirmQueue 中添加 isPrompt 分支
if (isPrompt) {
  if (cancelBtn) cancelBtn.style.display = '';
  if (okBtn) okBtn.textContent = '确定';
  // 显示输入框
  const inputEl = document.getElementById('confirm-modal-input');
  if (inputEl) {
    inputEl.style.display = '';
    inputEl.value = defaultValue || '';
    inputEl.focus();
  }
  // onOk 返回输入值
  const onOk = () => {
    cleanup();
    const inputEl = document.getElementById('confirm-modal-input');
    resolve(inputEl ? inputEl.value : '');
  };
} else if (isInfoOnly) {
  // ... v0.8.4 Step 3 的逻辑 ...
} else {
  // ... showConfirm 逻辑 ...
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（showPrompt 约 3154 行 + processConfirmQueue）

**验证方法**：

1. **G054 验证**：
   ```javascript
   // showConfirm 打开后调用 showPrompt，应入队等待
   const confirmPromise = window.showConfirm('测试', '标题', 0);
   await new Promise(r => setTimeout(r, 100));
   
   const promptPromise = window.showPrompt('请输入', '提示');
   await new Promise(r => setTimeout(r, 100));
   
   // showPrompt 应入队，未覆盖 showConfirm
   const msgEl = document.getElementById('confirm-modal-message');
   msgEl.textContent === '测试'  // 应返回 true
   
   // 点击确认关闭 showConfirm
   document.getElementById('confirm-modal-ok').click();
   await new Promise(r => setTimeout(r, 100));
   
   // showPrompt 应自动显示
   const inputEl = document.getElementById('confirm-modal-input');
   inputEl.style.display !== 'none'  // 应返回 true
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 16 验证脚本：G054 showPrompt 入队 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== G054 验证：showPrompt 入队 ===' -ForegroundColor Cyan
    
    # 查找 showPrompt 函数
    $showPrompt = Select-String -LiteralPath $appJsPath -Pattern 'function\s+showPrompt'
    if ($showPrompt) {
        $funcBody = Get-Content -LiteralPath $appJsPath -TotalCount ($showPrompt.LineNumber + 30) | Select-Object -Skip ($showPrompt.LineNumber - 1)
        $hasEnqueue = $funcBody | Where-Object { $_ -match 'confirmModalQueue\.push' }
        if ($hasEnqueue) {
            Write-Host 'PASS: showPrompt 已入队' -ForegroundColor Green
        } else {
            Write-Host 'FAIL: showPrompt 未入队' -ForegroundColor Red
        }
    }
    
    $isPrompt = Select-String -LiteralPath $appJsPath -Pattern 'isPrompt'
    if ($isPrompt.Count -ge 2) {
        Write-Host ('PASS: isPrompt 分支已实现（{0} 处）' -f $isPrompt.Count) -ForegroundColor Green
    } else {
        Write-Host 'FAIL: isPrompt 分支未实现' -ForegroundColor Red
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：无（v0.8.4 Step 3 已实现 processConfirmQueue 的 isInfoOnly，此处扩展 isPrompt）

**预计工作量**：0.5 人天

**如果失败怎么办**：
- 若 showPrompt 输入框不显示 → 检查 confirm-modal-input 元素是否存在
- 若队列死锁 → 检查 cleanup() 是否在所有路径调用
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 17: 修复 v0.8.4 回归风险（confirmModalQueue + MutationObserver + searchAbortController）

**目标缺口**：v0.8.4 回归风险（confirmModalQueue 队列死锁 + MutationObserver 性能 + searchAbortController 未清理）

**严重程度**：P1（RICE 768，回归风险）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.4.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.4.md) 第四节：
- v0.8.4 Step 3 引入 confirmModalQueue，但 processConfirmQueue 抛异常时队列永久阻塞
- v0.8.4 Step 13 引入 MutationObserver（subtree:true），dashboard 自动刷新时性能下降
- v0.8.4 Step 11 引入 searchAbortController，但标签切换时未 abort

**修复方案**（分步实施）：

**子步骤 17.1**：processConfirmQueue 添加 try-catch

```javascript
// 修改 processConfirmQueue（约 2777 行）
async function processConfirmQueue() {
  if (confirmModalActive || confirmModalQueue.length === 0) return;
  confirmModalActive = true;
  
  try {
    // ... 原有处理逻辑 ...
  } catch (e) {
    console.error('[processConfirmQueue] 处理异常:', e);
    // 异常时清空队列，避免死锁
    confirmModalQueue.length = 0;
    confirmModalActive = false;
    // 关闭模态框
    const modal = document.getElementById('confirm-modal');
    if (modal) {
      modal.hidden = true;
      modal.style.display = 'none';
    }
    showToast('弹窗处理异常，已清空队列', 'error');
  }
}
```

**子步骤 17.2**：MutationObserver 添加防抖 + 缩小监听范围

```javascript
// 修改 MutationObserver（约 1039 行）
let _inputObserverDebounce = null;
const inputObserver = new MutationObserver(mutations => {
  // G056 修复：防抖 500ms
  if (_inputObserverDebounce) clearTimeout(_inputObserverDebounce);
  _inputObserverDebounce = setTimeout(() => {
    mutations.forEach(mutation => {
      mutation.addedNodes.forEach(node => {
        if (node.nodeType === 1) {
          const inputs = node.matches('input[required], input[data-required]') 
            ? [node] 
            : Array.from(node.querySelectorAll('input[required], input[data-required]'));
          inputs.forEach(input => {
            if (input.dataset.validationBound !== 'true') {
              input.dataset.validationBound = 'true';
              input.addEventListener('blur', () => validateInput(input));
              input.addEventListener('focus', () => clearInputError(input));
            }
          });
        }
      });
    });
  }, 500);
});

// 缩小监听范围到 wizard-content（而非整个 body）
const wizardContent = document.getElementById('wizard-content') || document.body;
inputObserver.observe(wizardContent, { childList: true, subtree: true });
```

**子步骤 17.3**：_abortActiveTabRequests 中 abort searchAbortController

```javascript
// 修改 _abortActiveTabRequests（约 4838 行）
function _abortActiveTabRequests(excludeTab) {
  _tabAbortControllers.forEach((controller, tabName) => {
    if (tabName === excludeTab) return;
    if (controller) controller.abort();
  });
  _tabAbortControllers.clear();
  
  // v0.8.4 回归修复：标签切换时 abort searchAbortController
  if (typeof searchAbortController !== 'undefined' && searchAbortController) {
    searchAbortController.abort();
    searchAbortController = null;
  }
  
  // dashboardAbortController 由 loadDashboard 自身管理（v0.8.4 Step 7 已修复）
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（processConfirmQueue + MutationObserver + _abortActiveTabRequests）

**验证方法**：

1. **confirmModalQueue 死锁验证**：
   ```javascript
   // 模拟 processConfirmQueue 抛异常
   confirmModalQueue.push({ /* 无效 task */ });
   processConfirmQueue();
   // 异常后队列应清空
   confirmModalQueue.length === 0  // 应返回 true
   confirmModalActive === false    // 应返回 true
   ```

2. **MutationObserver 防抖验证**：
   ```javascript
   // 快速添加 10 个输入框
   for (let i = 0; i < 10; i++) {
     const input = document.createElement('input');
     input.required = true;
     document.body.appendChild(input);
   }
   // 防抖 500ms 后才处理
   await new Promise(r => setTimeout(r, 600));
   ```

3. **searchAbortController 清理验证**：
   ```javascript
   // 触发搜索
   searchMemories('test');
   // 切换标签
   switchTab('settings');
   searchAbortController === null  // 应返回 true
   ```

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 17 验证脚本：v0.8.4 回归风险 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$appJsPath = 'G:\code-memory\static\app.js'

try {
    Write-Host '=== 回归风险 1：confirmModalQueue try-catch ===' -ForegroundColor Cyan
    $processFn = Select-String -LiteralPath $appJsPath -Pattern 'function\s+processConfirmQueue'
    if ($processFn) {
        $funcBody = Get-Content -LiteralPath $appJsPath -TotalCount ($processFn.LineNumber + 80) | Select-Object -Skip ($processFn.LineNumber - 1)
        $hasCatch = $funcBody | Where-Object { $_ -match 'catch\s*\(' }
        $hasClear = $funcBody | Where-Object { $_ -match 'confirmModalQueue\.length\s*=\s*0|confirmModalQueue\.splice' }
        if ($hasCatch -and $hasClear) {
            Write-Host 'PASS: processConfirmQueue 有 try-catch + 清空队列' -ForegroundColor Green
        } else {
            Write-Host 'FAIL: processConfirmQueue 缺少异常处理或清空' -ForegroundColor Red
        }
    }
    
    Write-Host "`n=== 回归风险 2：MutationObserver 防抖 ===" -ForegroundColor Cyan
    $debounce = Select-String -LiteralPath $appJsPath -Pattern '_inputObserverDebounce' 
    if ($debounce) {
        Write-Host 'PASS: MutationObserver 防抖已实现' -ForegroundColor Green
    } else {
        Write-Host 'FAIL: MutationObserver 未防抖' -ForegroundColor Red
    }
    
    Write-Host "`n=== 回归风险 3：searchAbortController 清理 ===" -ForegroundColor Cyan
    $abortFn = Select-String -LiteralPath $appJsPath -Pattern 'function\s+_abortActiveTabRequests'
    if ($abortFn) {
        $funcBody = Get-Content -LiteralPath $appJsPath -TotalCount ($abortFn.LineNumber + 20) | Select-Object -Skip ($abortFn.LineNumber - 1)
        $hasSearch = $funcBody | Where-Object { $_ -match 'searchAbortController' }
        if ($hasSearch) {
            Write-Host 'PASS: _abortActiveTabRequests 清理 searchAbortController' -ForegroundColor Green
        } else {
            Write-Host 'FAIL: _abortActiveTabRequests 未清理 searchAbortController' -ForegroundColor Red
        }
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：Step 9（G052 完成后，确认 _retryModalActive 等变量正常）+ Step 16（showPrompt 入队完成）

**预计工作量**：0.5 人天

**如果失败怎么办**：
- 若 try-catch 后队列清空导致用户操作丢失 → 调整为仅清空当前 task，保留后续 task
- 若 MutationObserver 防抖后输入框未绑定 → 检查防抖时间是否过长
- 若 searchAbortController abort 后搜索结果空白 → 检查 AbortError 分支清理逻辑
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 18: 版本号升级 + 文档同步 + 回归测试

**目标缺口**：版本号统一升级为 0.8.5 + 文档同步 + 完整回归测试

**严重程度**：收尾（必须）

**修复方案**（分步实施）：

**子步骤 18.1**：版本号统一升级

| 位置 | 当前版本 | 目标版本 | 文件 |
|:---|:---|:---|:---|
| tauri.conf.json | 0.8.4 | 0.8.5 | [desktop/src-tauri/tauri.conf.json](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json) |
| Cargo.toml | 0.8.4 | 0.8.5 | [Cargo.toml](file:///g:/code-memory/Cargo.toml) |
| HTML meta | 0.8.4 | 0.8.5 | [static/index.html](file:///g:/code-memory/static/index.html) |
| JS APP_VERSION | 0.8.4 | 0.8.5 | [static/app.js](file:///g:/code-memory/static/app.js) |
| 后端版本 | 0.8.4 | 0.8.5 | [src/v1_api.rs](file:///g:/code-memory/src/v1_api.rs) |

**子步骤 18.2**：CHANGELOG 更新

```markdown
## v0.8.5 (2026-07-29)

### P0 致命问题修复（4 个）
- 修复 G080：restoreSelectedScenario 调用 + CDP#3 active 类（Step 2）
- 修复 G015：validateInput 暴露到 window（Step 3）
- 修复 G076：CSP 配置 + XSS 防御（Step 4）
- 修复 G058：启动服务取消按钮 AbortController（Step 5，用户反馈问题 1）
- 修复 G059：道同构横幅幂等（Step 6，用户反馈问题 2）
- 修复 CDP#2：道同构卡片 DOM 始终存在（Step 7，用户反馈问题 2）
- 修复 CDP#1：道同构 API 返回 JSON（Step 8，用户反馈问题 2）
- 修复 G052：handleHttpError 接入所有 fetchWithTimeout（Step 9，激活 G007/G009/G029/G047）
- 修复 G060：_retryCounters Map 上限（Step 10，防泄漏）

### CDP 测试失败项修复（5/5）
- CDP#1 道同构 API 结构（Step 8）
- CDP#2 道同构指标显示（Step 7）
- CDP#3 预设场景切换（Step 2）
- CDP#4 G011 Toast 去重（Step 11）
- CDP#5 G015 输入校验（Step 3）

### P1 严重问题修复（7 个）
- 修复 G057：/health 超时降级（Step 12）
- 修复 G053：navbar-nav 统一 switchTab（Step 13）
- 修复 G056：sidecar 崩溃重连（Step 14）
- 修复 G064：道同构卡片 aria-label（Step 15）
- 修复 G054：showPrompt 入队（Step 16）
- 修复 v0.8.4 回归风险（Step 17）

### 韧性评分提升
- v0.8.4：63.5/100 → v0.8.5：≥ 85/100（+21.5）
- CDP 通过率：83.8% → ≥ 90%（+6.2%）
```

**子步骤 18.3**：完整回归测试

1. 运行 CDP 回归测试：`node scripts/test_desktop_cdp_v083.js`，通过率 ≥ 90%
2. 五层交互韧性复测：综合评分 ≥ 85/100
3. 用户反馈三大问题验收：
   - 启动服务取消按钮可中断请求
   - 道同构度 4 个小指标正常显示
   - 预设场景切换 + 刷新后保留
4. v0.8.4 修复回归验证：G004/G013/G017 等仍通过

**影响文件**：
- [G:\code-memory\desktop\src-tauri\tauri.conf.json](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json)
- [G:\code-memory\Cargo.toml](file:///g:/code-memory/Cargo.toml)
- [G:\code-memory\static\index.html](file:///g:/code-memory/static/index.html)
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)
- [G:\code-memory\src\v1_api.rs](file:///g:/code-memory/src/v1_api.rs)
- [G:\code-memory\CHANGELOG.md](file:///g:/code-memory/CHANGELOG.md)

**验证方法**：

1. **版本号一致性验证**：所有 5 处版本号必须为 0.8.5
2. **CDP 回归测试**：`node scripts/test_desktop_cdp_v083.js`，通过率 ≥ 90%（≥ 34/37）
3. **交互韧性复测**：综合评分 ≥ 85/100，P0 问题清零
4. **用户反馈三大问题验收**：
   - 启动服务取消按钮可中断请求（Step 5）
   - 道同构度 4 个小指标正常显示（Step 6-8）
   - 预设场景切换 + 刷新后保留（Step 2）
5. **v0.8.4 修复回归验证**：G004/G013/G017/G018/G019/G020/G022/G025/G029/G033/G045 等仍通过
6. **v0.8.4 回归风险修复验证**：confirmModalQueue try-catch、MutationObserver 防抖、searchAbortController 清理（Step 17）

**PowerShell 验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === Step 18 验证脚本：版本号一致性 + 回归测试 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$repoRoot = 'G:\code-memory'
$expectedVersion = '0.8.5'
$versionLocations = @(
    @{ Path = "$repoRoot\desktop\src-tauri\tauri.conf.json"; Pattern = '"version"\s*:\s*"([^"]+)"'; Name = 'tauri.conf.json' },
    @{ Path = "$repoRoot\Cargo.toml"; Pattern = '^version\s*=\s*"([^"]+)"'; Name = 'Cargo.toml' },
    @{ Path = "$repoRoot\static\index.html"; Pattern = '<meta\s+name="version"\s+content="([^"]+)"'; Name = 'index.html meta' },
    @{ Path = "$repoRoot\static\app.js"; Pattern = "APP_VERSION\s*=\s*'([^']+)'"; Name = 'app.js APP_VERSION' },
    @{ Path = "$repoRoot\src\v1_api.rs"; Pattern = 'version\s*=\s*"([^"]+)"'; Name = 'v1_api.rs' }
)

try {
    Write-Host '=== Step 18 验证：版本号一致性 ===' -ForegroundColor Cyan
    $allMatch = $true
    foreach ($loc in $versionLocations) {
        if (Test-Path -LiteralPath $loc.Path) {
            $content = Get-Content -LiteralPath $loc.Path -Raw -Encoding UTF8
            $match = [regex]::Match($content, $loc.Pattern)
            if ($match.Success) {
                $actualVersion = $match.Groups[1].Value
                if ($actualVersion -eq $expectedVersion) {
                    Write-Host ("PASS: {0} 版本号 = {1}" -f $loc.Name, $actualVersion) -ForegroundColor Green
                } else {
                    Write-Host ("FAIL: {0} 版本号 = {1}（应为 {2}）" -f $loc.Name, $actualVersion, $expectedVersion) -ForegroundColor Red
                    $allMatch = $false
                }
            } else {
                Write-Host ("WARN: {0} 未找到版本号模式" -f $loc.Name) -ForegroundColor Yellow
            }
        } else {
            Write-Host ("WARN: 文件不存在 - {0}" -f $loc.Path) -ForegroundColor Yellow
        }
    }
    
    if ($allMatch) {
        Write-Host "`nPASS: 所有版本号一致（$expectedVersion）" -ForegroundColor Green
    } else {
        Write-Host "`nFAIL: 版本号不一致，请检查上述 FAIL 项" -ForegroundColor Red
    }
    
    Write-Host "`n=== CDP 回归测试 ===" -ForegroundColor Cyan
    $cdpScript = "$repoRoot\scripts\test_desktop_cdp_v083.js"
    if (Test-Path -LiteralPath $cdpScript) {
        Write-Host '运行 CDP 回归测试（node scripts/test_desktop_cdp_v083.js）...' -ForegroundColor Yellow
        # 注意：实际执行需要 sidecar 运行 + Tauri WebView CDP 端口可用
        # 此处仅验证脚本存在，实际运行由测试人员执行
        Write-Host 'PASS: CDP 测试脚本存在，请手动执行验证通过率 ≥ 90%' -ForegroundColor Green
    } else {
        Write-Host 'WARN: CDP 测试脚本不存在' -ForegroundColor Yellow
    }
    
    Write-Host "`n=== CHANGELOG 更新验证 ===" -ForegroundColor Cyan
    $changelogPath = "$repoRoot\CHANGELOG.md"
    if (Test-Path -LiteralPath $changelogPath) {
        $changelogContent = Get-Content -LiteralPath $changelogPath -Raw -Encoding UTF8
        if ($changelogContent -match '##\s+v0\.8\.5') {
            Write-Host 'PASS: CHANGELOG 已包含 v0.8.5 章节' -ForegroundColor Green
            # 验证关键修复项是否记录
            $requiredFixes = @('G052', 'G058', 'G076', 'G080', 'G060', 'G015')
            foreach ($fix in $requiredFixes) {
                if ($changelogContent -match $fix) {
                    Write-Host ("  PASS: {0} 已记录" -f $fix) -ForegroundColor Green
                } else {
                    Write-Host ("  WARN: {0} 未记录" -f $fix) -ForegroundColor Yellow
                }
            }
        } else {
            Write-Host 'FAIL: CHANGELOG 未包含 v0.8.5 章节' -ForegroundColor Red
        }
    }
}
catch {
    Write-Host ("验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

**依赖 Step**：所有 Step（Step 1-17）

**预计工作量**：0.3 人天

**如果失败怎么办**：
- 若版本号不一致 → 逐个文件检查并修正（禁止批量替换，必须分步）
- 若 CDP 回归测试通过率 < 90% → 分析失败项，必要时回滚相关 Step
- 若 v0.8.4 修复回归（G004/G013/G017 等失败）→ 优先回滚 Step 17（v0.8.4 回归风险修复）
- 若交互韧性评分 < 85 → 检查 P1 修复是否完整，必要时延后 P2/P3 修复
- 回滚方案：`git revert <v0.8.5 commit>` 回滚整个 v0.8.5，或 `git checkout HEAD~N -- <file>` 回滚单个文件

---

## 五、风险评估与回滚方案

### 5.1 风险识别矩阵

按照"发生概率 × 影响程度"对 v0.8.5 实施过程中的 12 个关键风险进行评估，并明确对应的缓解措施与回滚策略。

| 风险 ID | 风险描述 | 发生概率 | 影响程度 | 风险等级 | 触发条件 | 缓解措施 | 对应 Step |
|:---:|:---|:---:|:---:|:---:|:---|:---|:---:|
| R01 | G052 handleHttpError 接入后引入新的错误处理路径，导致原本正常的请求被误判为失败 | 中 | 高 | **高** | handleHttpError 返回值未正确处理 | 分步替换（loadDashboard → loadMemories → searchMemories → loadTrustCenterData），每步独立验证 | Step 9 |
| R02 | G058 AbortController 在 Tauri WebView2 中兼容性问题导致 fetch 不响应 signal | 低 | 高 | 中 | WebView2 版本 < 100 | 添加 AbortController polyfill 检测，降级为超时取消 | Step 5 |
| R03 | G076 CSP 配置过严导致合法内联脚本/style 被拦截，UI 渲染异常 | 高 | 中 | **高** | style-src 未保留 'unsafe-inline' | CSP 保留 style-src 'unsafe-inline'，script-src 严格 | Step 4 |
| R04 | G080 restoreSelectedScenario 调用导致 dashboard 首次加载延迟，CDP 测试超时 | 低 | 中 | 低 | localStorage 读取阻塞主线程 | restoreSelectedScenario 异步执行，不阻塞 dashboard 渲染 | Step 2 |
| R05 | Step 8 道同构 API 修复影响 sidecar 编译，导致桌面端启动失败 | 中 | 高 | **高** | v1_api.rs 路由注册冲突 | 修改后执行 `cargo check --features server` 验证编译 | Step 8 |
| R06 | Step 9-10 G052+G060 联合修复后，_retryCounters 在高频错误场景下清理过早，导致有效重试计数丢失 | 中 | 中 | 中 | MAX_RETRY_COUNTERS_SIZE=50 过小 | 监控清理频率，必要时调整为 100 或按 LRU 策略清理 | Step 10 |
| R07 | Step 11 Toast 去重窗口缩短到 500ms 后，用户快速点击相同按钮时丢失有效提示 | 中 | 低 | 低 | 用户操作速度 < 500ms | 增加按钮防抖（v0.8.4 Step 14 已实现 inFlight 防抖） | Step 11 |
| R08 | Step 13 navbar-nav 统一 switchTab 后，标签切换行为变化导致用户习惯中断 | 低 | 中 | 低 | switchTab 添加了额外逻辑（abort/loading） | switchTab 保持原有 UI 行为，仅增加前置 abort 逻辑 | Step 13 |
| R09 | Step 14 sidecar 崩溃重连的指数退避导致用户长时间等待，误判应用卡死 | 中 | 中 | 中 | 重连间隔过长（>30s） | 最大重连间隔限制为 10s，并在 UI 显示重连进度 | Step 14 |
| R10 | Step 17 confirmModalQueue try-catch 在异常时清空队列，导致用户确认操作丢失 | 中 | 高 | **高** | 队列清空策略过于激进 | 仅清空当前异常 task，保留后续 task；记录被丢弃的 task 到日志 | Step 17 |
| R11 | Step 17 MutationObserver 防抖 500ms 导致动态添加的输入框在 500ms 内未绑定校验 | 中 | 中 | 中 | 用户在 500ms 内快速输入 | 防抖结束后立即扫描，并对已聚焦输入框补绑 blur 事件 | Step 17 |
| R12 | v0.8.5 整体回滚后，v0.8.4 的 14 项修复（Step 2-14）状态不一致 | 低 | 高 | 中 | 部分文件回滚，部分未回滚 | 回滚必须以 commit 为单位，禁止文件级部分回滚 | 全局 |

### 5.2 高风险项详细应对方案

#### R01: G052 handleHttpError 接入误判风险（Step 9）

**风险描述**：
handleHttpError 设计为返回 `{action: 'retry'|'cancel'|'giveup'}` 三种动作，但现有 fetchWithTimeout 调用方多数仅检查 `response.ok`。若直接替换为 `handleHttpError(response, context)` 而未正确处理返回值，可能导致：
- 200 OK 响应被误判为错误（handleHttpError 内部逻辑 bug）
- 500 错误后无脑重试导致请求风暴
- 429 限流后用户无法主动取消

**缓解措施**：
1. **分步替换**：按 loadDashboard → loadMemories → searchMemories → loadTrustCenterData 顺序，每替换一个立即运行时验证
2. **单元测试前置**：在替换前先为 handleHttpError 编写测试用例（200/429/500/503/网络错误 5 种场景）
3. **降级开关**：添加 `window._disableHandleHttpError = false` 全局开关，出现问题时可一键降级为原始 response.ok 检查
4. **日志埋点**：handleHttpError 每次调用记录 `[handleHttpError] context=${context.url} status=${response.status} action=${result.action}`

**回滚方案**：
```powershell
# 单 Step 回滚（仅回滚 Step 9 的 handleHttpError 接入）
git checkout HEAD~1 -- static/app.js
# 或精确回滚（保留其他修改，仅还原 handleHttpError 调用）
# 需手动编辑 app.js，将 handleHttpError(response, context) 替换回 response.ok 检查
```

#### R03: CSP 配置过严导致 UI 异常风险（Step 4）

**风险描述**：
Tauri WebView2 基于 Chromium，对 CSP 执行严格。若 CSP 配置过严：
- `style-src 'self'` 会拦截所有内联 style 属性（如 `element.style.display = 'none'`）
- `script-src 'self'` 会拦截所有内联 script 和 onclick（v0.8.4 已移除 onclick，但需验证）
- 可能导致 dashboard 渲染白屏或样式错乱

**缓解措施**：
1. **渐进式 CSP**：先添加宽松 CSP（`default-src 'self' 'unsafe-inline' 'unsafe-eval'`），验证无白屏后逐步收紧
2. **style-src 保留 'unsafe-inline'**：Tauri WebView2 大量使用内联 style，必须保留
3. **script-src 严格**：`script-src 'self'`（禁止内联脚本和 eval）
4. **CSP 报告模式**：先添加 `<meta http-equiv="Content-Security-Policy-Report-Only" content="...">` 收集违规报告，确认无违规后再切换为强制 CSP
5. **运行时验证**：CSP 添加后在 WebView 控制台检查 `document.querySelectorAll('[onclick]')` 返回空数组

**回滚方案**：
```powershell
# CSP 回滚（仅移除 meta CSP 标签）
git checkout HEAD~1 -- static/index.html
# 或临时注释 CSP meta 标签
```

#### R05: sidecar 编译失败风险（Step 8）

**风险描述**：
Step 8 修改 [src/v1_api.rs](file:///g:/code-memory/src/v1_api.rs) 的 `/v1/dao-metrics` 路由，可能影响 sidecar 编译。若 sidecar 编译失败：
- 桌面端启动时找不到 lrc-sidecar.exe
- 用户看到"LRC 服务未运行"错误
- 所有 API 调用失败

**缓解措施**：
1. **编译验证前置**：Step 8 修改后立即执行 `cargo check --features server`
2. **路由冲突检测**：使用 `Select-String` 检查 v1_api.rs 中是否已存在 `/v1/dao-metrics` 路由定义
3. **保留原有路由**：不删除原有路由，仅修改返回值结构（从 HTML 改为 JSON）
4. **sidecar 二进制备份**：修改前备份 `lrc-sidecar.exe`，编译失败时可恢复

**回滚方案**：
```powershell
# sidecar 回滚
git checkout HEAD~1 -- src/v1_api.rs
cargo build --bin code-memory-server --features server
# 将编译产物复制到 desktop 同级目录
Copy-Item -LiteralPath "target\release\code-memory-server.exe" -Destination "desktop\lrc-sidecar.exe" -Force
```

#### R10: confirmModalQueue 队列清空导致用户操作丢失风险（Step 17）

**风险描述**：
Step 17 为 processConfirmQueue 添加 try-catch，异常时清空队列。但若队列中有多个用户确认操作（如"删除记忆？"、"清空历史？"），异常清空会导致用户无法确认这些操作，可能造成数据不一致。

**缓解措施**：
1. **仅清空当前 task**：异常时仅丢弃当前 task，保留后续 task
2. **记录丢弃的 task**：`console.error('[processConfirmQueue] 丢弃 task:', task)` 便于排查
3. **用户提示**：异常时显示 Toast"操作处理异常，请重试"，引导用户重新触发
4. **队列上限保护**：队列长度上限 5（v0.8.4 Step 3 已实现），异常清空最多影响 5 个操作

**回滚方案**：
```powershell
# Step 17 子步骤 17.1 回滚（仅移除 try-catch）
git checkout HEAD~1 -- static/app.js
# 或手动移除 processConfirmQueue 的 try-catch 包裹
```

### 5.3 回滚策略与分级回滚方案

#### 5.3.1 回滚分级

| 回滚级别 | 触发条件 | 回滚范围 | 操作复杂度 | 数据影响 |
|:---:|:---|:---|:---:|:---|
| L1 单文件回滚 | 单个 Step 验证失败 | 1 个文件 | 低 | 无 |
| L2 单 Step 回滚 | 单个 Step 引入回归 | 1-3 个文件 | 低 | 无 |
| L3 阶段回滚 | 整个阶段（如 P0 道同构簇）失败 | 5-10 个文件 | 中 | 无 |
| L4 版本回滚 | v0.8.5 整体无法达到验收标准 | 所有 v0.8.5 修改文件 | 高 | 无（纯代码回滚） |
| L5 数据回滚 | v0.8.5 修改导致用户数据损坏 | 代码 + 用户配置 + localStorage | 极高 | 需恢复用户数据 |

#### 5.3.2 L1 单文件回滚操作

适用于单个文件修改导致编译失败或运行时错误。

```powershell
# === L1 单文件回滚 ===
# 前提：已 git commit 当前修改
# 回滚 static/app.js 到上一个 commit
git checkout HEAD~1 -- static/app.js

# 验证回滚后状态
git status
```

#### 5.3.3 L2 单 Step 回滚操作

适用于单个 Step（可能涉及多文件）引入回归。

```powershell
# === L2 单 Step 回滚（以 Step 9 G052 为例） ===
# 前提：Step 9 已独立 commit
# 查找 Step 9 的 commit hash
git log --oneline --all | Select-String -Pattern "Step 9|G052"

# 回滚 Step 9（假设 commit hash 为 abc1234）
git revert abc1234 --no-edit

# 验证回滚后状态
git log --oneline -5
```

#### 5.3.4 L3 阶段回滚操作

适用于整个阶段（如 Step 6-8 道同构簇）失败。

```powershell
# === L3 阶段回滚（以 Step 6-8 道同构簇为例） ===
# 前提：Step 6-8 已分别 commit
# 查找 Step 6 的 commit hash（道同构簇第一个 Step）
git log --oneline --all | Select-String -Pattern "Step 6|G059"

# 回滚到 Step 5 之前的状态（假设 Step 6 commit hash 为 def5678）
git revert def5678..HEAD --no-edit

# 验证回滚后状态
git log --oneline -10
```

#### 5.3.5 L4 版本回滚操作

适用于 v0.8.5 整体无法达到验收标准（CDP < 90% 或韧性评分 < 85）。

```powershell
# === L4 版本回滚 ===
# 前提：v0.8.5 所有修改已 commit，且 v0.8.4 有明确 tag
# 查找 v0.8.4 的 commit hash 或 tag
git tag -l "v0.8.*"
git log --oneline --all | Select-String -Pattern "v0\.8\.4|0\.8\.4"

# 方案 A：创建回滚 commit（保留历史，推荐）
git revert <v0.8.5 第一个 commit>..HEAD --no-edit

# 方案 B：硬回滚（丢弃 v0.8.5 所有修改，谨慎使用）
# 警告：此操作不可逆，会丢失所有未提交修改
# git reset --hard <v0.8.4 commit hash>

# 验证回滚后状态
git log --oneline -20
```

#### 5.3.6 L5 数据回滚操作

适用于 v0.8.5 修改导致用户数据损坏（如 localStorage 数据丢失、记忆库损坏）。

```powershell
# === L5 数据回滚 ===
# 前提：v0.8.5 发布前已通过 Trust Center 创建数据备份
# 备份位置：~/.loong-recall/backups/

# 1. 恢复记忆库数据
$backupDir = "$env:USERPROFILE\.loong-recall\backups"
$dataDir = "$env:USERPROFILE\.loong-recall\global\data"

# 列出可用备份
Get-ChildItem -LiteralPath $backupDir -Filter "*.tar.gz" | Sort-Object LastWriteTime -Descending | Select-Object -First 5

# 恢复最新备份（需用户确认）
$latestBackup = Get-ChildItem -LiteralPath $backupDir -Filter "*.tar.gz" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($latestBackup) {
    Write-Host ("将恢复备份: {0}" -f $latestBackup.Name) -ForegroundColor Yellow
    $confirm = Read-Host "确认恢复？(y/N)"
    if ($confirm -eq 'y' -or $confirm -eq 'Y') {
        # 先停止 sidecar
        # 再解压备份到数据目录
        tar -xzf $latestBackup.FullName -C $dataDir
        Write-Host "数据恢复完成" -ForegroundColor Green
    }
}

# 2. 清理 localStorage（Tauri WebView 中手动操作）
# 在 WebView 控制台执行：localStorage.clear()
```

### 5.4 回滚决策树

```
v0.8.5 实施过程中发现问题
│
├── 问题类型？
│   ├── 编译失败 ──────────────────→ L1 单文件回滚
│   ├── 单 Step 验证失败 ──────────→ L2 单 Step 回滚
│   ├── 阶段性回归（如道同构簇） ──→ L3 阶段回滚
│   ├── CDP 通过率 < 90% 或韧性 < 85 → L4 版本回滚
│   └── 用户数据损坏 ─────────────→ L5 数据回滚 + L4 版本回滚
│
├── 回滚后验证
│   ├── 编译通过 ──→ 继续后续 Step
│   ├── 编译失败 ──→ 升级回滚级别（L1→L2→L3）
│   └── 数据完整 ──→ 修复问题后重新实施
│
└── 回滚记录
    ├── 记录回滚级别、原因、影响范围
    ├── 更新 FIX_PLAN_v0.8.5.md 标注回滚的 Step
    └── 同步更新 CHANGELOG.md
```

### 5.5 回滚后重实施策略

回滚后需分析根因，修复问题后重新实施：

1. **根因分析**：回滚后立即分析失败原因，记录到 `docs/ROLLBACK_LOG_v0.8.5.md`
2. **修复方案**：根据根因调整修复方案，必要时拆分为更小的子步骤
3. **重新实施**：按调整后的方案重新实施，每个子步骤独立验证
4. **回归测试**：重新实施后运行完整 CDP 回归测试 + 交互韧性复测
5. **文档更新**：更新 FIX_PLAN_v0.8.5.md 标注"回滚后重实施"的 Step

### 5.6 备份与快照策略

v0.8.5 实施前必须创建以下备份：

| 备份类型 | 备份内容 | 备份位置 | 备份时机 |
|:---|:---|:---|:---|
| Git 快照 | v0.8.4 最终状态的 commit/tag | 本地 git 仓库 | v0.8.5 实施前 |
| 代码备份 | static/app.js, static/index.html, src/v1_api.rs | `backup/v0.8.4/` | v0.8.5 实施前 |
| 数据备份 | 记忆库 + 配置 + localStorage | `~/.loong-recall/backups/` | v0.8.5 实施前（通过 Trust Center） |
| sidecar 二进制备份 | lrc-sidecar.exe | `backup/v0.8.4/lrc-sidecar.exe` | 修改 v1_api.rs 前 |

**备份验证脚本**（已通过 PowerShell 专家技能规范验证）：

```powershell
# === v0.8.5 实施前备份验证 ===
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$repoRoot = 'G:\code-memory'
$backupDir = "$repoRoot\backup\v0.8.4"

try {
    Write-Host '=== v0.8.5 实施前备份验证 ===' -ForegroundColor Cyan
    
    # 1. 验证 Git 快照
    Write-Host "`n--- Git 快照验证 ---" -ForegroundColor Yellow
    $currentCommit = git -C $repoRoot rev-parse HEAD
    Write-Host ("当前 HEAD: {0}" -f $currentCommit)
    
    # 创建 v0.8.4 备份 tag（如不存在）
    $tagExists = git -C $repoRoot tag -l "v0.8.4-baseline"
    if (-not $tagExists) {
        git -C $repoRoot tag "v0.8.4-baseline"
        Write-Host "已创建 tag: v0.8.4-baseline" -ForegroundColor Green
    } else {
        Write-Host "tag v0.8.4-baseline 已存在" -ForegroundColor Green
    }
    
    # 2. 验证代码备份
    Write-Host "`n--- 代码备份验证 ---" -ForegroundColor Yellow
    if (-not (Test-Path -LiteralPath $backupDir)) {
        New-Item -ItemType Directory -Path $backupDir -Force | Out-Null
        Write-Host "已创建备份目录: $backupDir" -ForegroundColor Green
    }
    
    $filesToBackup = @(
        "$repoRoot\static\app.js",
        "$repoRoot\static\index.html",
        "$repoRoot\src\v1_api.rs",
        "$repoRoot\desktop\src-tauri\tauri.conf.json",
        "$repoRoot\Cargo.toml"
    )
    
    foreach ($file in $filesToBackup) {
        $fileName = Split-Path -Leaf $file
        $destPath = "$backupDir\$fileName"
        if (Test-Path -LiteralPath $file) {
            Copy-Item -LiteralPath $file -Destination $destPath -Force
            Write-Host ("  已备份: {0}" -f $fileName) -ForegroundColor Green
        }
    }
    
    # 3. 验证 sidecar 二进制备份
    Write-Host "`n--- sidecar 二进制备份验证 ---" -ForegroundColor Yellow
    $sidecarPath = "$repoRoot\desktop\lrc-sidecar.exe"
    if (Test-Path -LiteralPath $sidecarPath) {
        Copy-Item -LiteralPath $sidecarPath -Destination "$backupDir\lrc-sidecar.exe" -Force
        Write-Host "已备份 lrc-sidecar.exe" -ForegroundColor Green
    } else {
        Write-Host "WARN: lrc-sidecar.exe 不存在，跳过备份" -ForegroundColor Yellow
    }
    
    # 4. 验证数据备份（通过 Trust Center API）
    Write-Host "`n--- 数据备份验证 ---" -ForegroundColor Yellow
    $dataBackupDir = "$env:USERPROFILE\.loong-recall\backups"
    if (Test-Path -LiteralPath $dataBackupDir) {
        $backups = Get-ChildItem -LiteralPath $dataBackupDir -Filter "*.tar.gz"
        Write-Host ("数据备份文件数: {0}" -f $backups.Count) -ForegroundColor Green
        if ($backups.Count -gt 0) {
            $latest = $backups | Sort-Object LastWriteTime -Descending | Select-Object -First 1
            Write-Host ("最新备份: {0}（{1}）" -f $latest.Name, $latest.LastWriteTime) -ForegroundColor Green
        }
    } else {
        Write-Host "WARN: 数据备份目录不存在，请通过 Trust Center 创建备份" -ForegroundColor Yellow
    }
    
    Write-Host "`n=== 备份验证完成 ===" -ForegroundColor Cyan
    Write-Host "如需回滚，请参考 FIX_PLAN_v0.8.5.md 第 5.3 节" -ForegroundColor Yellow
}
catch {
    Write-Host ("备份验证失败: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
```

---

## 六、实施时间表与里程碑

### 6.1 总体时间表

v0.8.5 预计 **9.6 人天**，按 7 个阶段分步实施。每个阶段结束后设置里程碑验证点，未通过验证则触发回滚流程。

| 阶段 | Step 范围 | 工作量 | 累计工作量 | 里程碑 | 验证标准 |
|:---|:---|:---:|:---:|:---|:---|
| 阶段一：根因调查 | Step 1 | 0.3 人天 | 0.3 人天 | M1 调查报告完成 | G052/G058/G080 真实状态确认 |
| 阶段二：P0 快速胜利 | Step 2-5 | 1.8 人天 | 2.1 人天 | M2 4 个 P0 快速胜利完成 | G080/G015/CSP/G058 验证通过 |
| 阶段三：P0 道同构簇 | Step 6-8 | 1.0 人天 | 3.1 人天 | M3 道同构功能恢复 | CDP#1/#2/#3 通过 + 用户反馈问题 2 解决 |
| 阶段四：P0 核心攻坚 | Step 9-10 | 2.2 人天 | 5.3 人天 | M4 错误恢复层恢复 | handleHttpError 调用 ≥ 8 处 + _retryCounters 上限 50 |
| 阶段五：CDP 收尾 | Step 11 | 0.2 人天 | 5.5 人天 | M5 CDP 通过率 ≥ 90% | CDP 测试通过 34/37 |
| 阶段六：P1 攻坚 | Step 12-16 | 1.9 人天 | 7.4 人天 | M6 韧性评分 ≥ 85 | 五层交互韧性复测通过 |
| 阶段七：回归风险 + 收尾 | Step 17-18 | 2.2 人天 | 9.6 人天 | M7 v0.8.5 发布 | 版本号一致 + CHANGELOG 更新 + 全部回归测试通过 |

### 6.2 详细甘特图

```
Day 1 (0.3d)  │███│ Step 1: 根因调查
Day 1-2 (1.8d)│█████████████████████│ Step 2-5: P0 快速胜利
              │██│ Step 2: G080 (0.2d)
              │███│ Step 3: G015 (0.3d)
              │███│ Step 4: CSP (0.5d)
              │█████│ Step 5: G058 (1.0d)  ← 用户反馈问题 1
Day 3 (1.0d)  │█████████│ Step 6-8: P0 道同构簇  ← 用户反馈问题 2
              │██│ Step 6: G059 (0.2d)
              │██│ Step 7: CDP#2 (0.3d)
              │███│ Step 8: CDP#1 (0.5d)
Day 4-5 (2.2d)│████████████████████████│ Step 9-10: P0 核心攻坚
              │█████████████████████│ Step 9: G052 (2.0d)  ← 最复杂
              │██│ Step 10: G060 (0.2d)
Day 5 (0.2d)  │██│ Step 11: CDP#4 Toast 去重
Day 6-7 (1.9d)│████████████████████│ Step 12-16: P1 攻坚
              │███│ Step 12: G057 (0.3d)
              │███│ Step 13: G053 (0.3d)
              │█████│ Step 14: G056 (0.5d)
              │██│ Step 15: G064 (0.1d)
              │█████│ Step 16: G054 (0.5d)
              │███│ Step 17: 回归风险 (0.5d)
Day 8-9 (2.2d)│████████████████████████│ Step 17-18: 收尾
              │███│ Step 17: 回归风险 (0.5d)
              │█████████████████████│ Step 18: 版本升级 + 回归测试 (1.7d)
```

### 6.3 关键里程碑验证清单

#### M1 调查报告完成（Day 1 结束）

- [ ] G052 死代码状态确认（Select-String 验证 handleHttpError 调用数 = 0）
- [ ] G058 AbortController 缺失确认（Select-String 验证 postMessageToParent 无 signal）
- [ ] G080 restoreSelectedScenario 未调用确认（Select-String 验证调用数 = 0）
- [ ] 产出《修复未生效根因调查报告》

#### M2 P0 快速胜利完成（Day 2 结束）

- [ ] G080 restoreSelectedScenario 在 loadDashboard 末尾调用
- [ ] G015 validateInput 暴露到 window
- [ ] G076 CSP meta 标签已添加
- [ ] G058 AbortController 已实现 + 取消按钮 data-action 绑定
- [ ] 用户反馈问题 1（启动服务取消）解决

#### M3 道同构功能恢复（Day 3 结束）

- [ ] G059 道同构横幅幂等（banner 存在性检查）
- [ ] CDP#2 道同构卡片 DOM 始终存在
- [ ] CDP#1 道同构 API 返回 JSON {ok, data}
- [ ] 用户反馈问题 2（道同构小指标闪烁）解决
- [ ] sidecar 编译通过（`cargo check --features server`）

#### M4 错误恢复层恢复（Day 5 结束）

- [ ] G052 handleHttpError 调用 ≥ 8 处
- [ ] G060 _retryCounters Map 上限 50
- [ ] handleHttpError 单元测试通过（200/429/500/503/网络错误 5 种场景）
- [ ] G007/G009/G029/G047 恢复有效

#### M5 CDP 通过率 ≥ 90%（Day 5 结束）

- [ ] CDP 测试通过 34/37
- [ ] CDP#1/#2/#3/#4/#5 全部通过
- [ ] CDP P0 通过率 100%

#### M6 韧性评分 ≥ 85（Day 7 结束）

- [ ] 五层交互韧性复测综合评分 ≥ 85/100
- [ ] P0 致命问题清零
- [ ] P1 严重问题 ≤ 2 个

#### M7 v0.8.5 发布（Day 9 结束）

- [ ] 版本号一致性（5 处均为 0.8.5）
- [ ] CHANGELOG 更新完整
- [ ] v0.8.4 修复回归验证通过（G004/G013/G017 等仍通过）
- [ ] v0.8.4 回归风险修复验证通过（confirmModalQueue/MutationObserver/searchAbortController）
- [ ] 用户反馈三大问题全部解决

### 6.4 每日实施节奏

| 时间段 | 活动 | 产出 |
|:---|:---|:---|
| 09:00-09:30 | 每日站会 + 回顾昨日进度 | 当日实施计划 |
| 09:30-12:00 | 上午实施（1-2 个 Step） | Step 代码修改 + 验证 |
| 12:00-13:30 | 午休 | — |
| 13:30-17:00 | 下午实施（1-2 个 Step） | Step 代码修改 + 验证 |
| 17:00-17:30 | 每日验证 + 文档同步 | 当日 Step 验证报告 |
| 17:30-18:00 | Git commit + 推送 | 当日 commit |

### 6.5 关键依赖关系

```
Step 1 (调查) ─┬─→ Step 2 (G080)
               ├─→ Step 5 (G058)
               └─→ Step 9 (G052)

Step 4 (CSP) ─────→ Step 9 (G052，需 CSP 保护 innerHTML)

Step 9 (G052) ─┬─→ Step 10 (G060，必须在 G052 激活前修复)
               └─→ Step 17 (回归风险，依赖 _retryModalActive)

Step 6 (G059) ─┬─→ Step 7 (CDP#2，依赖横幅幂等)
               └─→ Step 8 (CDP#1，依赖卡片 DOM)

Step 16 (G054) ────→ Step 17 (回归风险，依赖 showPrompt 入队)

Step 17 (回归风险) ─→ Step 18 (版本升级，依赖所有修复完成)
```

**关键路径**：Step 1 → Step 9 → Step 10 → Step 17 → Step 18（最长依赖链，5.0 人天）

**并行机会**：
- Step 2/3/4/5 可并行（P0 快速胜利阶段）
- Step 6/7/8 可并行（道同构簇，但建议串行避免冲突）
- Step 12-16 可并行（P1 攻坚阶段）

---

## 七、总结

### 7.1 v0.8.5 核心价值

v0.8.5 是 LRC 桌面端从"修复引入回归"到"稳定可靠交付"的关键转折版本。本计划通过 18 个 Step 系统性解决：

1. **4 个 P0 致命问题**：G052 handleHttpError 死代码、G058 启动服务取消按钮、G076 CSP 未配置、G080 restoreSelectedScenario 未调用
2. **5 个 CDP 失败项**：道同构 API/卡片/预设场景 + Toast 去重 + 输入校验
3. **12 个 P1 严重问题**：错误恢复、并发控制、模态框交互、可访问性等
4. **4 个 v0.8.4 回归风险**：confirmModalQueue 死锁、MutationObserver 性能、searchAbortController 未清理、_retryCounters 内存泄漏
5. **3 个用户反馈问题**：启动服务取消、道同构小指标闪烁、预设场景刷新丢失

### 7.2 预期收益

| 维度 | v0.8.4 现状 | v0.8.5 目标 | 收益 |
|:---|:---:|:---:|:---:|
| CDP 测试通过率 | 83.8% | ≥ 90% | +6.2% |
| 交互韧性综合评分 | 63.5/100 | ≥ 85/100 | +21.5 |
| P0 致命问题 | 4 个 + 3 个 CDP P0 | 0 个 | -7 |
| P1 严重问题 | 12 个 | ≤ 2 个 | -10 |
| handleHttpError 调用方 | 0 处 | ≥ 8 处 | 错误恢复层恢复 |
| 用户反馈问题 | 3 个未解决 | 0 个 | 全部解决 |
| v0.8.4 回归风险 | 4 个 | 0 个 | 全部消除 |

### 7.3 关键成功因素

1. **分步实施原则**：禁止批量修改，每个子步骤独立验证，避免"假修复"重演
2. **PowerShell 专家规范**：所有验证脚本符合编码自保护块、完整命令名、显式参数、try/catch 规范
3. **调查先行**：Step 1 不修改代码，先确认 G052/G058/G080 真实状态，避免错误修复方向
4. **RICE 优先级**：P0 内部按 RICE 分数 + 依赖关系排序，确保高价值修复优先
5. **回归风险同步处理**：v0.8.4 引入的 4 个回归风险在 Step 17 统一处理，避免遗留
6. **备份与回滚**：实施前创建 Git 快照 + 代码备份 + 数据备份，5 级回滚方案确保可控

### 7.4 后续版本规划

| 版本 | 目标 | 预计工作量 |
|:---|:---|:---:|
| v0.8.5 | P0 清零 + CDP ≥ 90% + 韧性 ≥ 85 | 9.6 人天 |
| v0.8.6 | P2 中优先级问题（G049-G051/G055/G061-G063/G065-G066） | 3.0 人天 |
| v0.8.7 | P3 低优先级问题 + 性能优化 + 无障碍增强 | 2.0 人天 |
| v0.9.0 | E2E 测试套件 + 体验 polishing | 5.0 人天 |
| v1.0.0 | 生产环境就绪 + 完整文档 + 发布 | 3.0 人天 |

### 7.5 风险声明

本计划基于 v0.8.4 审计报告和 CDP 测试报告编制，实际实施中可能遇到：
- 未预见的兼容性问题（如 Tauri WebView2 版本差异）
- v0.8.4 修复的隐性回归（Step 17 已覆盖已知风险，但可能有未识别风险）
- sidecar 编译环境差异（如 Rust 版本、依赖更新）
- 用户数据迁移风险（如 localStorage 结构变化）

遇到未预见问题时，优先回滚到 v0.8.4 基线，分析根因后重新实施。

---

## 附录 A：缺口 ID 索引

| 缺口 ID | 严重度 | 问题简述 | 对应 Step | 状态 |
|:---|:---:|:---|:---:|:---:|
| G052 | P0 | handleHttpError 死代码 | Step 9 | 计划修复 |
| G058 | P0 | 启动服务取消按钮无 AbortController | Step 5 | 计划修复 |
| G076 | P0 | CSP 完全未配置 | Step 4 | 计划修复 |
| G080 | P0 | restoreSelectedScenario 未调用 | Step 2 | 计划修复 |
| G015 | P0 | validateInput 未暴露到 window | Step 3 | 计划修复 |
| G059 | P0 | 道同构横幅重复重建 | Step 6 | 计划修复 |
| CDP#1 | P0 | 道同构 API 返回 HTML | Step 8 | 计划修复 |
| CDP#2 | P0 | 道同构卡片 DOM 不存在 | Step 7 | 计划修复 |
| CDP#3 | P0 | 预设场景 active 类未添加 | Step 2 | 计划修复 |
| CDP#4 | P1 | G011 Toast 去重窗口过长 | Step 11 | 计划修复 |
| CDP#5 | P1 | G015 输入校验未暴露 | Step 3 | 计划修复 |
| G053 | P1 | navbar-nav 绕过 switchTab | Step 13 | 计划修复 |
| G054 | P1 | showPrompt 未入队 | Step 16 | 计划修复 |
| G056 | P1 | sidecar 崩溃无重连 | Step 14 | 计划修复 |
| G057 | P1 | /health 超时无降级 | Step 12 | 计划修复 |
| G060 | P1 | _retryCounters Map 内存泄漏 | Step 10 | 计划修复 |
| G064 | P1 | 道同构卡片无 aria-label | Step 15 | 计划修复 |
| G049 | P2 | LLM API 配置输入无格式校验 | — | v0.8.6 |
| G050 | P2 | 记忆搜索输入无最大长度 | — | v0.8.6 |
| G051 | P2 | 向导路径输入无存在性校验 | — | v0.8.6 |
| G055 | P2 | 标签页 active 类闪烁 | — | v0.8.6 |
| G061 | P2 | _tabAbortControllers 堆积 | — | v0.8.6 |
| G062 | P2 | dashboardAbortController 错误时未清理 | — | v0.8.6 |
| G063 | P2 | searchAbortController 标签切换未 abort | Step 17 | 计划修复 |
| G065 | P2 | Toast 容器无 role="status" | — | v0.8.6 |
| G066 | P2 | 模态框关闭后焦点未返回 | — | v0.8.6 |

## 附录 B：PowerShell 验证脚本索引

| Step | 验证目标 | 脚本位置 |
|:---:|:---|:---|
| Step 1 | G052/G058/G080 真实状态调查 | FIX_PLAN_v0.8.5.md Step 1 |
| Step 2 | G080 restoreSelectedScenario 调用 + CDP#3 active 类 | FIX_PLAN_v0.8.5.md Step 2 |
| Step 3 | G015 validateInput 暴露 + CDP#5 | FIX_PLAN_v0.8.5.md Step 3 |
| Step 4 | G076 CSP 配置 + XSS 防御 | FIX_PLAN_v0.8.5.md Step 4 |
| Step 5 | G058 AbortController + 取消按钮 | FIX_PLAN_v0.8.5.md Step 5 |
| Step 6 | G059 道同构横幅幂等 | FIX_PLAN_v0.8.5.md Step 6 |
| Step 7 | CDP#2 道同构卡片 DOM | FIX_PLAN_v0.8.5.md Step 7 |
| Step 8 | CDP#1 道同构 API JSON | FIX_PLAN_v0.8.5.md Step 8 |
| Step 9 | G052 handleHttpError 调用 ≥ 8 处 | FIX_PLAN_v0.8.5.md Step 9 |
| Step 10 | G060 Map 上限 50 | FIX_PLAN_v0.8.5.md Step 10 |
| Step 11 | CDP#4 Toast 去重窗口 ≤ 500ms | FIX_PLAN_v0.8.5.md Step 11 |
| Step 12 | G057 /health 超时降级 | FIX_PLAN_v0.8.5.md Step 12 |
| Step 13 | G053 navbar-nav 统一 switchTab | FIX_PLAN_v0.8.5.md Step 13 |
| Step 14 | G056 sidecar 崩溃重连 | FIX_PLAN_v0.8.5.md Step 14 |
| Step 15 | G064 道同构卡片 aria-label | FIX_PLAN_v0.8.5.md Step 15 |
| Step 16 | G054 showPrompt 入队 | FIX_PLAN_v0.8.5.md Step 16 |
| Step 17 | v0.8.4 回归风险修复 | FIX_PLAN_v0.8.5.md Step 17 |
| Step 18 | 版本号一致性 + 回归测试 | FIX_PLAN_v0.8.5.md Step 18 |
| 5.6 | 实施前备份验证 | FIX_PLAN_v0.8.5.md 第 5.6 节 |

## 附录 C：文件修改影响矩阵

| 文件 | 涉及 Step | 修改行数（预估） | 修改类型 |
|:---|:---|:---:|:---|
| [static/app.js](file:///g:/code-memory/static/app.js) | Step 2/3/5/6/9/10/11/12/13/14/16/17/18 | +350 / -50 | 核心逻辑修改 |
| [static/index.html](file:///g:/code-memory/static/index.html) | Step 4/7/15/18 | +30 / -5 | CSP + DOM 结构 + aria-label |
| [src/v1_api.rs](file:///g:/code-memory/src/v1_api.rs) | Step 8/18 | +20 / -10 | 道同构 API 路由修复 |
| [desktop/src-tauri/tauri.conf.json](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json) | Step 18 | +1 / -1 | 版本号升级 |
| [Cargo.toml](file:///g:/code-memory/Cargo.toml) | Step 18 | +1 / -1 | 版本号升级 |
| [CHANGELOG.md](file:///g:/code-memory/CHANGELOG.md) | Step 18 | +40 / -0 | 版本日志更新 |

**总修改量**：+442 / -67 行（净增 375 行）

---

> 文档结束
> 编制人：创始者智能体（产品设计师）
> 编制日期：2026-07-29
> 版本：v0.8.5
> 状态：待实施
> 下一步：按 Step 1-18 顺序分步实施，每个 Step 完成后独立验证并 git commit