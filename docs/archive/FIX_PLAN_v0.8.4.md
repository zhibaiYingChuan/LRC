# LRC 桌面端 v0.8.4 修复优化计划

> 文档版本：v0.8.4
> 基线版本：v0.8.3（CDP 通过率 73.0%，韧性评分 76.3/100）
> 编制日期：2026-07-29
> 编制人：创始者智能体（产品设计师）
> 依据文档：
> - [CDP_TEST_REPORT_v0.8.3.md](file:///g:/code-memory/docs/CDP_TEST_REPORT_v0.8.3.md)
> - [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md)
> - [FIX_PLAN_v0.8.3.md](file:///g:/code-memory/docs/FIX_PLAN_v0.8.3.md)
> 目标：分步修复 CDP 测试 9 个失败项 + 交互韧性审计 30 个缺口，达到 CDP 通过率 ≥ 90%、韧性评分 ≥ 90/100

---

## 一、执行摘要

### 1.1 v0.8.3 现状诊断

LRC 桌面端 v0.8.3 在同步 API 清零（-32 处）、Z-index 规范化、标签页管理、错误处理等维度取得重大进展（韧性评分从 59.4 提升至 76.3），但 CDP 回归测试暴露了 **3 个真实 P0 bug** 和 **6 个 P1/P2 问题**，核心矛盾集中在：

| 问题簇 | 根因类型 | 影响 |
|:---|:---|:---|
| 函数暴露缺失（G018/G019） | v0.8.3 修复时遗漏 `window.xxx = xxx` 暴露语句 | showConfirm/startAutoRefresh 无法被 CDP 测试和动态按钮访问 |
| 模态框队列死锁（G022） | showInfoModal 复用 confirm-modal DOM 但未入队 | 所有 showConfirm 调用永久阻塞 |
| 道同构度降级不完整（G020） | 降级分支仅设 score=85，未更新 4 个小指标 | 仪表盘显示矛盾状态（评分 85 但指标全 '--'） |
| AbortController 竞态（G021/G033） | _abortActiveTabRequests 误 abort dashboard | loading-overlay 永久显示 |
| XSS 残留（G025/G032/G040） | 动态按钮 onclick 注入 + memory_type 未转义 | 安全风险 + 防抖绕过 |

### 1.2 v0.8.4 修复策略

本计划采用 **"调查先行 → P0 清零 → P1 攻坚 → P2/P3 收尾"** 四阶段策略，共 **20 个 Step**，预计 **9.7 人天**：

| 阶段 | Step 范围 | 目标 | 工作量 |
|:---|:---|:---|:---|
| 阶段一：根因调查 | Step 1 | 查明 v0.8.2/v0.8.3 修复为何在 v0.8.3 失效 | 0.3 人天 |
| 阶段二：P0 清零 | Step 2-5 | 解决 5 个 P0 致命问题（3 CDP + 2 审计） | 1.2 人天 |
| 阶段三：P1 攻坚 | Step 6-14 | 解决 11 个 P1 严重问题 + 6 个 CDP 失败项 | 4.1 人天 |
| 阶段四：P2/P3 收尾 | Step 15-20 | 解决 17 个 P2/P3 问题 + 版本升级 | 4.0 人天 |
| **合计** | | | **9.6 人天** |

### 1.3 关键决策

1. **Step 1 为调查 Step，不修改代码**：先查明 G004/showConfirm/startAutoRefresh 的"修复未生效"真相（回滚？从未修复？测试检测问题？），再决定后续修复策略
2. **P0 优先级最高**：5 个 P0 问题（G018/G022/G020/G045 + CDP 道同构 API）必须在 Step 2-5 内全部解决
3. **CDP 失败项全覆盖**：9 个 CDP 失败项分布在 Step 2/3/4/5/6/8/12/13/14，每个都有独立验证方法
4. **历史回归独立验证**：G004/showConfirm/startAutoRefresh 三个"v0.8.2 修复在 v0.8.3 失效"的问题在 Step 1 调查后，分别在 Step 2/6/14 独立验证
5. **简单优先**：相同优先级下，1 行修复（如 showConfirm 暴露）优先于 100 行修复（如模态框队列重构）

---

## 二、修复目标与验收标准

### 2.1 核心验收矩阵

| 维度 | v0.8.3 现状 | v0.8.4 目标 | 验收方式 |
|:---|:---|:---|:---|
| CDP 测试通过率 | 73.0%（27/37） | **≥ 90%（34/37）** | 重新运行 CDP 回归测试 |
| 交互韧性综合评分 | 76.3/100 | **≥ 90/100** | 五层交互韧性复测 |
| P0 致命问题 | 5 个（3 CDP + 2 审计） | **0 个** | CDP P0 通过率 100% + 审计 P0 清零 |
| 同步阻塞 API | 0 处 | **0 处** | 保持清零状态 |
| showConfirm 暴露 | 失败 | **通过** | `typeof window.showConfirm === 'function'` |
| startAutoRefresh 暴露 | 失败 | **通过** | `typeof window.startAutoRefresh === 'function'` |
| 道同构指标显示 | 失败（4 个 '--'） | **通过** | 降级时 4 个指标显示 '--' + 黄色横幅提示 |
| 预设场景切换 | 失败 | **通过** | 切换后 active 状态正确 + 刷新后保留 |
| 模态框队列死锁 | 存在 | **消除** | showConfirm 打开后调用 showInfoModal 两者都能关闭 |
| 动态按钮 XSS | 存在 | **消除** | memory_type 含恶意字符不触发 XSS |
| 500 重试次数上限 | 无限 | **≤ 3 次** | 超过 3 次显示"停止重试"选项 |

### 2.2 韧性评分目标分解

| 维度 | v0.8.3 评分 | v0.8.4 目标 | 提升来源 |
|:---|:---|:---|:---|
| 同步 API 阻塞 | 100/100 | 100/100 | 保持 |
| 按钮防抖 | 85/100 | 95/100 | G025/G032 动态按钮纳入防抖 |
| 错误分类 | 80/100 | 90/100 | G029 重试次数上限 + G047 竞态修复 |
| 全局状态检测 | 90/100 | 95/100 | G027 offline-mode 禁用按钮 |
| 模态框交互 | 75/100 | 95/100 | G022 死锁修复 + G023 焦点陷阱 + G046 队列 |
| 标签页管理 | 80/100 | 95/100 | G021/G033 竞态修复 |
| 输入校验 | 75/100 | 85/100 | G024 Enter 提交 + G015 验证 |
| 视觉反馈 | 45/100 | 70/100 | G039 下载进度 + G028 差异化提示 |
| 错误修复路径 | 60/100 | 85/100 | G029 重试上限 + 指数退避 |
| 无障碍 | 40/100 | 65/100 | G023 焦点陷阱 + G041 移动端同步 |
| 函数暴露完整性 | 75/100 | 100/100 | G018/G019 暴露 |
| 并发请求控制 | 70/100 | 90/100 | G037/G038/G036/G044 AbortController |
| **综合评分** | **76.3/100** | **≥ 90/100** | **+13.7** |

---

## 三、修复优先级矩阵（按用户影响×流失风险×修复成本排序）

### 3.1 优先级评分模型

每个缺口按以下三维评分（1-5 分），综合分 = 用户影响 × 0.4 + 流失风险 × 0.4 + 修复成本反向 × 0.2：

| 缺口 ID | 用户影响 | 流失风险 | 修复成本(反向) | 综合分 | 优先级 | 对应 Step |
|:---|:---:|:---:|:---:|:---:|:---:|:---:|
| G018 showConfirm 未暴露 | 5 | 5 | 5（1 行） | 5.0 | P0 | Step 2 |
| G022 队列死锁 | 5 | 5 | 3（中等） | 4.6 | P0 | Step 3 |
| G020 道同构降级不完整 | 4 | 4 | 4（简单） | 4.0 | P0 | Step 4 |
| G045 预设场景无持久化 | 4 | 3 | 4（简单） | 3.6 | P0 | Step 5 |
| CDP#1 道同构 API 环境 | 3 | 3 | 4（简单） | 3.2 | P0 | Step 4 |
| G019 startAutoRefresh 未暴露 | 3 | 2 | 5（1 行） | 3.0 | P2 | Step 6 |
| G021/G033 竞态+loading 卡死 | 4 | 4 | 3（中等） | 3.8 | P1 | Step 7 |
| G026 Toast 去重边界 | 3 | 3 | 4（简单） | 3.2 | P1 | Step 8 |
| G025/G032/G040 XSS+防抖绕过 | 5 | 4 | 2（复杂） | 3.8 | P1 | Step 9 |
| G029/G047 重试无限+竞态 | 4 | 4 | 3（中等） | 3.8 | P1 | Step 10 |
| G037/G038 搜索无 Abort | 3 | 3 | 3（中等） | 3.0 | P1 | Step 11 |
| G023 焦点陷阱+N08 模态框 | 3 | 3 | 3（中等） | 3.0 | P1 | Step 12 |
| G015 输入校验 | 3 | 2 | 4（简单） | 2.8 | P1 | Step 13 |
| G004 CDP 检测问题 | 2 | 2 | 4（简单） | 2.4 | P1 | Step 14 |
| G024/G027/G030/G041 中优先级 | 2 | 2 | 3（中等） | 2.2 | P2 | Step 15 |
| G046/G031 模态框队列冲突 | 2 | 2 | 2（复杂） | 2.0 | P2 | Step 16 |
| G036/G044 并发请求 | 2 | 2 | 2（复杂） | 2.0 | P2 | Step 17 |
| G039/G028 其他 P2 | 2 | 1 | 3（中等） | 1.8 | P2 | Step 18 |
| G043/G034/G035 P3 | 1 | 1 | 4（简单） | 1.4 | P3 | Step 19 |

### 3.2 排序原则

1. **P0 致命（Step 2-5）**：综合分 ≥ 3.6，必须在前 5 个 Step 内解决
2. **P1 严重（Step 6-14）**：综合分 2.4-3.8，2 周内解决
3. **P2 中优先级（Step 15-18）**：综合分 1.8-2.2，1 月内解决
4. **P3 低优先级（Step 19）**：综合分 < 1.5，后续迭代
5. **简单优先**：相同综合分下，1 行修复 < 10 行修复 < 100 行修复

---

## 四、详细 Step 列表（每个 Step 独立可验证）

### Step 1: 修复未生效根因调查（不修改代码）

**目标缺口**：查明 G004/showConfirm/startAutoRefresh 三个"v0.8.2/v0.8.3 修复在 v0.8.3 失效"的真相

**严重程度**：P0（调查类，阻断后续修复决策）

**根因分析**：

根据 [CDP_TEST_REPORT_v0.8.3.md](file:///g:/code-memory/docs/CDP_TEST_REPORT_v0.8.3.md) 4.1 节和 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 1.3 节，三个"修复未生效"问题存在矛盾：

| 缺口 | CDP 测试报告 | 审计报告代码检查 | 矛盾点 |
|:---|:---|:---|:---|
| G004 inFlight 防抖 | hasInFlight=undefined, hasSetTimeout=undefined | [app.js:5892](file:///g:/code-memory/static/app.js#L5892) 有 `el.dataset.inFlight === '1'`，[5928 行](file:///g:/code-memory/static/app.js#L5928) 有 setTimeout 解锁 | 代码存在但 CDP 检测不到 |
| showConfirm 暴露 | showConfirm 定义=false | [app.js:2758](file:///g:/code-memory/static/app.js#L2758) 定义后未执行 `window.showConfirm = showConfirm` | 真实 bug，从未暴露 |
| startAutoRefresh 暴露 | startAutoRefresh=false | [app.js:1603](file:///g:/code-memory/static/app.js#L1603) 定义后未暴露 | 真实 bug，从未暴露 |

**调查方案**（不修改代码，仅产出调查报告）：

1. **Git 历史调查**：
   ```powershell
   # 检查 static/app.js 的 git 历史，查找 showConfirm/startAutoRefresh 暴露语句是否曾经存在
   git log --oneline --all -- static/app.js | Select-Object -First 20
   git log -p --all -S "window.showConfirm = showConfirm" -- static/app.js
   git log -p --all -S "window.startAutoRefresh = startAutoRefresh" -- static/app.js
   git log -p --all -S "dataset.inFlight" -- static/app.js
   ```

2. **当前代码验证**：
   ```powershell
   # 验证当前 app.js 中 showConfirm/startAutoRefresh/inFlight 的实际状态
   Select-String -Path "G:\code-memory\static\app.js" -Pattern "window\.showConfirm\s*=" 
   Select-String -Path "G:\code-memory\static\app.js" -Pattern "window\.startAutoRefresh\s*="
   Select-String -Path "G:\code-memory\static\app.js" -Pattern "dataset\.inFlight|inFlight"
   Select-String -Path "G:\code-memory\static\app.js" -Pattern "window\.(showInfoModal|showPrompt|switchTab|selectPresetScenario)\s*="
   ```

3. **CDP 测试脚本检查**：
   ```powershell
   # 检查 CDP 测试脚本如何检测 G004 inFlight 防抖
   Select-String -Path "G:\code-memory\scripts\test_desktop_cdp_v083.js" -Pattern "inFlight|bindAllActions|G004"
   ```

4. **得出结论**（三选一）：
   - **结论 A（回滚）**：修复曾提交但被后续 commit 覆盖 → 需要重新应用修复 + 加强 CI 防回归
   - **结论 B（从未修复）**：v0.8.2/v0.8.3 修复计划声称修复但实际未执行 → 需要执行真实修复
   - **结论 C（测试检测问题）**：代码已修复但 CDP 测试检测方法有误 → 需要修复测试脚本

**初步预判**（基于审计报告）：
- G004：**结论 C**（代码已修复，CDP 测试用 `fn.toString()` 正则检测，可能未匹配 `el.dataset.inFlight` 驼峰写法）
- showConfirm：**结论 B**（v0.8.3 Step 7 实现了 confirmModalQueue 队列机制，但漏了 `window.showConfirm = showConfirm` 暴露语句）
- startAutoRefresh：**结论 B**（v0.8.3 Step 10 实现了 dashboardAbortController，但漏了 `window.startAutoRefresh = startAutoRefresh` 暴露语句）

**影响文件**：无（仅调查）

**验证方法**：

1. 产出《修复未生效根因调查报告》，明确三个缺口各自属于结论 A/B/C
2. 调查报告需包含：
   - Git commit 历史截图（commit hash + 时间 + 作者）
   - 当前代码 grep 结果
   - CDP 测试脚本检测逻辑分析
   - 后续修复策略建议（对应 Step 2/6/14）

**PowerShell 验证脚本**（需先经 PowerShell 专家技能验证）：

```powershell
# 调查脚本：检查三个缺口的真实状态
Write-Host "=== G004 inFlight 防抖调查 ===" -ForegroundColor Cyan
$inFlight = Select-String -Path "G:\code-memory\static\app.js" -Pattern "dataset\.inFlight|inFlight" -SimpleMatch
if ($inFlight) {
    Write-Host "代码中存在 inFlight 检查（共 $($inFlight.Count) 处）:" -ForegroundColor Green
    $inFlight | ForEach-Object { Write-Host "  L$($_.LineNumber): $($_.Line.Trim())" }
} else {
    Write-Host "代码中未找到 inFlight 检查" -ForegroundColor Red
}

Write-Host "`n=== showConfirm 暴露调查 ===" -ForegroundColor Cyan
$showConfirmExpose = Select-String -Path "G:\code-memory\static\app.js" -Pattern "window\.showConfirm\s*="
if ($showConfirmExpose) {
    Write-Host "showConfirm 已暴露:" -ForegroundColor Green
    $showConfirmExpose | ForEach-Object { Write-Host "  L$($_.LineNumber): $($_.Line.Trim())" }
} else {
    Write-Host "showConfirm 未暴露到 window（真实 bug）" -ForegroundColor Red
}

Write-Host "`n=== startAutoRefresh 暴露调查 ===" -ForegroundColor Cyan
$startAutoRefreshExpose = Select-String -Path "G:\code-memory\static\app.js" -Pattern "window\.startAutoRefresh\s*="
if ($startAutoRefreshExpose) {
    Write-Host "startAutoRefresh 已暴露:" -ForegroundColor Green
} else {
    Write-Host "startAutoRefresh 未暴露到 window（真实 bug）" -ForegroundColor Red
}

Write-Host "`n=== 对比：已暴露的函数 ===" -ForegroundColor Cyan
$exposed = Select-String -Path "G:\code-memory\static\app.js" -Pattern "window\.(showInfoModal|showPrompt|switchTab|selectPresetScenario|handleHttpError)\s*="
$exposed | ForEach-Object { Write-Host "  L$($_.LineNumber): $($_.Line.Trim())" -ForegroundColor Green }
```

**依赖 Step**：无（首个 Step）

**预计工作量**：0.3 人天

**如果失败怎么办**：
- 若 Git 历史调查无法确定回滚点 → 降级为"按当前代码状态修复"，跳过历史追溯
- 若 PowerShell 命令执行失败 → 改用 Read 工具直接读取 app.js 关键行号区间
- 调查失败不影响后续 Step，可直接按预判结论 B/C 执行修复

---

### Step 2: 暴露 showConfirm 到 window（修复 G018 + CDP 测试 #7）

**目标缺口**：G018（showConfirm 未暴露到 window）+ CDP 测试 #7（G013 showConfirm 已定义）

**严重程度**：P0

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 1.3 节 P1-7 行：
- [showConfirm](file:///g:/code-memory/static/app.js#L2758) 在 IIFE 内定义
- v0.8.3 Step 7 实现了 [confirmModalQueue](file:///g:/code-memory/static/app.js#L2753) 队列机制和 [processConfirmQueue](file:///g:/code-memory/static/app.js#L2777)，但**遗漏了 `window.showConfirm = showConfirm` 暴露语句**
- 对比已暴露的函数：`window.handleHttpError`、`window.showInfoModal`、`window.showPrompt`、`window.selectPresetScenario`、`window.switchTab` 均有暴露语句，唯独 showConfirm 缺失
- 影响：CDP 测试 `typeof window.showConfirm === 'function'` 返回 false；[openMemoryDetail:4233](file:///g:/code-memory/static/app.js#L4233) 动态按钮用 `window[action]` 获取函数时若 action='showConfirm' 会失败

**修复方案**：

在 [showConfirm 函数定义后](file:///g:/code-memory/static/app.js#L2771)（processConfirmQueue 定义之前）添加暴露语句：

```javascript
// 在 app.js 约 2771 行，showConfirm 函数闭合 } 之后添加：
window.showConfirm = showConfirm;
```

**修复位置精确定位**：
- 查找 `async function showConfirm(message, title, timeoutMs) {` 定义（约 2758 行）
- 找到该函数的闭合 `}`（约 2771 行）
- 在闭合 `}` 后插入 `window.showConfirm = showConfirm;`

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（第 2771 行后插入 1 行）

**验证方法**：

1. **CDP 测试验证**（CDP 测试 #7）：
   ```javascript
   // 在 Tauri WebView 控制台执行
   typeof window.showConfirm === 'function'  // 应返回 true
   ```

2. **PowerShell 验证脚本**（需先经 PowerShell 专家技能验证）：
   ```powershell
   $result = Select-String -Path "G:\code-memory\static\app.js" -Pattern "window\.showConfirm\s*=\s*showConfirm"
   if ($result) {
       Write-Host "PASS: showConfirm 已暴露到 window" -ForegroundColor Green
       $result | ForEach-Object { Write-Host "  L$($_.LineNumber): $($_.Line.Trim())" }
   } else {
       Write-Host "FAIL: showConfirm 未暴露" -ForegroundColor Red
   }
   ```

3. **运行时验证**：在 Tauri WebView 中执行 `await window.showConfirm('测试', '标题', 0)` 应弹出确认模态框

4. **InteractionGuard 验证**：`FunctionExposeGuard.assertShowConfirmExposed()` 通过

**依赖 Step**：Step 1（调查结论确认是"从未修复"而非"回滚"）

**预计工作量**：0.1 人天

**如果失败怎么办**：
- 若添加暴露语句后 CDP 测试仍失败 → 检查 showConfirm 是否在 IIFE 作用域外不可访问，需将 showConfirm 改为 `window.showConfirm = async function(...)` 直接挂载
- 若 showConfirm 调用时报错 → 检查 confirmModalQueue/processConfirmQueue 是否在同一作用域
- 回滚方案：`git checkout HEAD~1 -- static/app.js`（仅回退 1 行修改）

---

### Step 3: 修复 G022 showInfoModal/showConfirm 队列死锁（关联 CDP 测试 #8）

**目标缺口**：G022（showInfoModal 与 showConfirm 共用 DOM 未释放 confirmModalActive 导致队列死锁）+ CDP 测试 #8（G013 模态框有可聚焦元素）

**严重程度**：P0

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 6.2 节：
- [showInfoModal:2886](file:///g:/code-memory/static/app.js#L2886) 直接操作 confirm-modal DOM，隐藏 cancelBtn
- showInfoModal 的 onOk 点击后 `modal.hidden = true`，但**未设置 `confirmModalActive = false`**
- 导致 [confirmModalQueue](file:///g:/code-memory/static/app.js#L2753) 中下一个 showConfirm 无法推进
- **死锁场景**：handleHttpError 调用 showConfirm → 用户未点击 → 另一处调用 showInfoModal → showInfoModal 覆盖 DOM → showInfoModal onOk 点击 → confirmModalActive 未释放 → 所有后续 showConfirm 永久阻塞

**修复方案**：

将 showInfoModal 改为入队模式，复用 [processConfirmQueue](file:///g:/code-memory/static/app.js#L2777) 串行处理：

```javascript
// 修改 showInfoModal 函数（约 2886 行）
// 修改前：直接操作 DOM
async function showInfoModal(title, content, options) {
  const modal = document.getElementById('confirm-modal');
  if (!modal) {
    console.error('[showInfoModal] confirm-modal 不存在');
    showToast(content, 'info');
    return;
  }
  // ... 直接操作 DOM，不走队列 ...
}

// 修改后：入队等待
async function showInfoModal(title, content, options = {}) {
  // 队列上限检查（与 showConfirm 一致）
  if (confirmModalQueue.length >= 5) {
    console.warn('[showInfoModal] 队列已满，拒绝新调用');
    return;
  }

  return new Promise((resolve) => {
    const task = {
      message: content,
      title: title,
      timeoutMs: 0, // showInfoModal 无超时
      resolve: (result) => resolve(result),
      isInfoOnly: true, // 标记为纯信息展示，隐藏取消按钮
      options: options
    };
    confirmModalQueue.push(task);
    processConfirmQueue();
  });
}
```

**processConfirmQueue 修改**（约 2777 行）：

在 [processConfirmQueue](file:///g:/code-memory/static/app.js#L2777) 中处理 `task.isInfoOnly` 标记：

```javascript
async function processConfirmQueue() {
  if (confirmModalActive || confirmModalQueue.length === 0) return;
  confirmModalActive = true;

  const task = confirmModalQueue.shift();
  const { message, title, timeoutMs, resolve, isInfoOnly, options } = task;

  try {
    const modal = document.getElementById('confirm-modal');
    const cancelBtn = document.getElementById('confirm-modal-cancel');
    const okBtn = document.getElementById('confirm-modal-ok');
    const msgEl = document.getElementById('confirm-modal-message');
    const titleEl = document.getElementById('confirm-modal-title');

    if (!modal) {
      console.error('[processConfirmQueue] confirm-modal 不存在');
      resolve(isInfoOnly ? undefined : false);
      return;
    }

    // 设置文案
    if (titleEl) titleEl.textContent = title || '提示';
    if (msgEl) msgEl.textContent = message;

    // 根据类型显示/隐藏取消按钮
    if (isInfoOnly) {
      if (cancelBtn) cancelBtn.style.display = 'none';
      if (okBtn) okBtn.textContent = options.okText || '知道了';
    } else {
      if (cancelBtn) cancelBtn.style.display = '';
      if (okBtn) okBtn.textContent = '确认';
    }

    // 显示模态框
    modal.hidden = false;
    modal.style.display = 'flex';

    // 绑定按钮事件（一次性）
    const onOk = () => {
      cleanup();
      resolve(isInfoOnly ? undefined : true);
    };
    const onCancel = () => {
      cleanup();
      resolve(false);
    };
    const onEsc = (e) => {
      if (e.key === 'Escape') {
        cleanup();
        resolve(isInfoOnly ? undefined : false);
      }
    };

    function cleanup() {
      modal.hidden = true;
      modal.style.display = 'none';
      if (okBtn) okBtn.removeEventListener('click', onOk);
      if (cancelBtn) cancelBtn.removeEventListener('click', onCancel);
      document.removeEventListener('keydown', onEsc);
      confirmModalActive = false; // 关键：释放锁
      // 处理队列中下一个
      if (confirmModalQueue.length > 0) {
        processConfirmQueue();
      }
    }

    if (okBtn) okBtn.addEventListener('click', onOk);
    if (cancelBtn && !isInfoOnly) cancelBtn.addEventListener('click', onCancel);
    document.addEventListener('keydown', onEsc);

    // 注册焦点陷阱（G023 关联，Step 12 完善）
    if (typeof onTabTrap === 'function') {
      modal.addEventListener('keydown', onTabTrap);
    }

    // 超时处理（仅 showConfirm）
    if (timeoutMs > 0) {
      setTimeout(() => {
        if (!modal.hidden) {
          cleanup();
          resolve(false);
        }
      }, timeoutMs);
    }

    // 聚焦到模态框内首个可聚焦元素（CDP 测试 #8）
    const focusable = modal.querySelectorAll('button:not([disabled]):not([style*="display: none"]):not([style*="display:none"]), input:not([disabled])');
    if (focusable.length > 0) {
      focusable[0].focus();
    }
  } catch (e) {
    console.error('[processConfirmQueue] 处理异常:', e);
    confirmModalActive = false;
    resolve(isInfoOnly ? undefined : false);
  }
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（showInfoModal 约 2886 行 + processConfirmQueue 约 2777 行）

**验证方法**：

1. **CDP 测试验证**（CDP 测试 #8）：
   ```javascript
   // 模态框有可聚焦元素
   const modal = document.getElementById('confirm-modal');
   const focusable = modal.querySelectorAll('button:not([disabled]):not([style*="display: none"]), input:not([disabled])');
   focusable.length > 0  // 应返回 true
   ```

2. **死锁验证**（InteractionGuard）：
   ```javascript
   // 1. 调用 showConfirm，不点击
   const confirmPromise = window.showConfirm('测试消息', '测试标题', 0);
   await new Promise(r => setTimeout(r, 100));
   
   // 2. 调用 showInfoModal
   window.showInfoModal('信息标题', '信息内容');
   await new Promise(r => setTimeout(r, 100));
   
   // 3. 验证 showInfoModal 入队等待，未覆盖 showConfirm
   const msgEl = document.getElementById('confirm-modal-message');
   msgEl.textContent === '测试消息'  // 应为 true（showConfirm 文案未被覆盖）
   
   // 4. 点击确认关闭 showConfirm
   document.getElementById('confirm-modal-ok').click();
   await new Promise(r => setTimeout(r, 100));
   
   // 5. 验证 showInfoModal 自动显示
   msgEl.textContent === '信息内容'  // 应为 true
   ```

3. **PowerShell 验证脚本**：
   ```powershell
   # 验证 showInfoModal 改为入队模式
   $result = Select-String -Path "G:\code-memory\static\app.js" -Pattern "confirmModalQueue\.push" 
   $showInfoModalEnqueue = $result | Where-Object { $_.Line -match "showInfoModal|isInfoOnly" }
   if ($showInfoModalEnqueue) {
       Write-Host "PASS: showInfoModal 已改为入队模式" -ForegroundColor Green
   } else {
       Write-Host "FAIL: showInfoModal 未入队" -ForegroundColor Red
   }
   ```

**依赖 Step**：Step 2（showConfirm 已暴露，便于测试）

**预计工作量**：0.5 人天

**如果失败怎么办**：
- 若 showInfoModal 入队后无法正常显示 → 检查 processConfirmQueue 的 isInfoOnly 分支逻辑
- 若 showConfirm 仍被 showInfoModal 覆盖 → 检查 confirmModalQueue.push 是否在 showInfoModal 入口处执行
- 若 confirmModalActive 未正确释放 → 检查 cleanup() 函数是否在所有退出路径调用
- 回滚方案：`git checkout HEAD~1 -- static/app.js`（恢复单例模式，但 G022 死锁问题恢复）

---

### Step 4: 修复 G020 loadDaoMetrics 降级不更新小指标（修复 CDP 测试 #1, #2）

**目标缺口**：G020（loadDaoMetrics 降级分支仅设 score=85，未更新 4 个小指标）+ CDP 测试 #1（道同构 API {ok, data} 结构）+ CDP 测试 #2（道同构卡片指标显示）

**严重程度**：P0

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 1.3 节 P0-2：
- [loadDaoMetrics:4000](file:///g:/code-memory/static/app.js#L4000) 降级分支仅执行 `drawDaoRing(85)` + `scoreEl.textContent = '85'`
- **未更新 4 个小指标**：dao-yin-yang、dao-luoshu-deviation、dao-bagua-balance、dao-synthesis-ratio
- 这 4 个指标保持 '--' 初始值，CDP 检测"值数量: 0"
- 同时 score=85 与指标 '--' 矛盾，用户困惑

**CDP 测试 #1（道同构 API {ok, data} 结构）根因**：
- 后端 [v1_api.rs:603](file:///g:/code-memory/src/v1_api.rs#L603) 实际返回 `{ok: true, data: {...}, raw: {...}}` 结构
- 但 CDP 测试时 sidecar 可能未启动，fetchWithTimeout 抛 SidecarUnreachableError
- 前端走 catch 分支（降级），CDP 检测 `hasOk=undefined, hasData=undefined`
- **这是测试环境问题 + 前端降级不完整双重原因**

**修复方案**：

1. **修复 loadDaoMetrics 降级分支**（约 4000 行）：

```javascript
// 修改前（约 4000 行）：
} catch (e) {
  console.error('[loadDaoMetrics] 加载失败:', e);
  drawDaoRing(85);
  const scoreEl = document.getElementById('dao-ring-score');
  if (scoreEl) scoreEl.textContent = '85';
}

// 修改后：
} catch (e) {
  console.error('[loadDaoMetrics] 加载失败:', e);
  
  // 降级时所有指标显示 '--'，避免 score=85 与指标 '--' 矛盾
  drawDaoRing(0); // 环形图显示空状态
  const scoreEl = document.getElementById('dao-ring-score');
  if (scoreEl) scoreEl.textContent = '--';
  
  // 更新 4 个小指标为 '--'（G020 核心修复）
  const subMetrics = ['dao-yin-yang', 'dao-luoshu-deviation', 'dao-bagua-balance', 'dao-synthesis-ratio'];
  subMetrics.forEach(id => {
    const el = document.getElementById(id);
    if (el) el.textContent = '--';
  });
  
  // 显示降级提示横幅（E11-NEW 修复）
  const panel = document.getElementById('dao-metrics-panel') || document.querySelector('.dao-metrics-panel');
  if (panel) {
    let banner = panel.querySelector('.dao-fallback-banner');
    if (!banner) {
      banner = document.createElement('div');
      banner.className = 'dao-fallback-banner';
      banner.style.cssText = 'background:#fff3cd;color:#856404;padding:8px 12px;border-radius:4px;margin-bottom:8px;font-size:13px;';
      panel.insertBefore(banner, panel.firstChild);
    }
    banner.textContent = '⚠ 数据加载失败，显示默认值。请检查 LRC 服务是否运行。';
    
    // 添加重试按钮
    const retryBtn = document.createElement('button');
    retryBtn.textContent = '重试';
    retryBtn.style.cssText = 'margin-left:8px;padding:2px 8px;background:#007bff;color:#fff;border:none;border-radius:2px;cursor:pointer;';
    retryBtn.dataset.action = 'retry-dao-metrics';
    banner.appendChild(retryBtn);
  }
  
  // 显示错误 Toast（区分环境问题 vs 代码 bug）
  const errorMsg = e.name === 'SidecarUnreachableError' 
    ? 'LRC 服务未启动，道同构度数据不可用'
    : '道同构度数据加载失败：' + (e.message || '未知错误');
  showToast(errorMsg, 'warning');
}
```

2. **在 bindAllActions 中添加 retry-dao-metrics action**（约 5892 行附近）：

```javascript
// 在 bindAllActions 的 action 映射中添加：
case 'retry-dao-metrics': {
  // 移除降级横幅
  const banner = document.querySelector('.dao-fallback-banner');
  if (banner) banner.remove();
  // 重新加载
  loadDaoMetrics();
  break;
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（loadDaoMetrics 约 4000 行 + bindAllActions 约 5892 行）

**验证方法**：

1. **CDP 测试 #2 验证**：
   ```javascript
   // sidecar 未启动时，4 个指标应显示 '--'
   const ids = ['dao-yin-yang', 'dao-luoshu-deviation', 'dao-bagua-balance', 'dao-synthesis-ratio'];
   const values = ids.map(id => document.getElementById(id)?.textContent);
   values.every(v => v === '--')  // 应返回 true
   ```

2. **CDP 测试 #1 验证**（需先确保 sidecar 启动）：
   ```javascript
   // sidecar 启动后，fetch /v1/health/dao_metrics 返回 {ok, data}
   const res = await fetch('/v1/health/dao_metrics');
   const json = await res.json();
   json.ok === true && json.data !== undefined  // 应返回 true
   ```

3. **降级提示验证**：
   ```javascript
   // sidecar 未启动时查看仪表盘
   const banner = document.querySelector('.dao-fallback-banner');
   banner !== null  // 应存在降级横幅
   banner.textContent.includes('数据加载失败')  // 应包含提示文案
   ```

4. **PowerShell 验证脚本**：
   ```powershell
   # 验证降级分支更新 4 个小指标
   $result = Select-String -Path "G:\code-memory\static\app.js" -Pattern "dao-yin-yang|dao-luoshu-deviation|dao-bagua-balance|dao-synthesis-ratio"
   $fallbackUpdates = $result | Where-Object { $_.Line -match "--|setText|textContent" }
   if ($fallbackUpdates.Count -ge 4) {
       Write-Host "PASS: 降级分支更新 4 个小指标" -ForegroundColor Green
   } else {
       Write-Host "FAIL: 降级分支未更新所有小指标" -ForegroundColor Red
   }
   ```

**依赖 Step**：Step 2（showConfirm 暴露，bindAllActions 防抖正常）

**预计工作量**：0.2 人天

**如果失败怎么办**：
- 若降级横幅不显示 → 检查 dao-metrics-panel 选择器是否匹配实际 DOM
- 若重试按钮无反应 → 检查 bindAllActions 是否正确处理 retry-dao-metrics action
- 若 CDP 测试 #1 仍失败 → 确认 CDP 测试前 sidecar 已启动（测试环境问题，非代码 bug）
- 回滚方案：`git checkout HEAD~1 -- static/app.js`（恢复 score=85 + 指标 '--' 矛盾状态）

---

### Step 5: 修复 G045 selectPresetScenario 持久化（修复 CDP 测试 #3）

**目标缺口**：G045（selectPresetScenario 无持久化，刷新页面丢失选中状态）+ CDP 测试 #3（预设场景模板切换）

**严重程度**：P0

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 1.3 节 P0-3：
- [selectPresetScenario](file:///g:/code-memory/static/app.js#L3721) 已暴露 window（[3646 行](file:///g:/code-memory/static/app.js#L3646)）
- bindAllActions 支持 data-arg-mode="this"（[5911 行](file:///g:/code-memory/static/app.js#L5911)）
- 代码逻辑正确，但 CDP 测试检测 `.active` 类，实际代码用 `.selected` 类
- 另外 [selectPresetScenario:3743](file:///g:/code-memory/static/app.js#L3743) 有 TODO 注释"v0.7.0 正式版将通过 MCP 工具 scenario 持久化"，切换后刷新页面丢失选中状态

**修复方案**：

1. **修复 selectPresetScenario 持久化**（约 3721 行）：

```javascript
// 修改 selectPresetScenario 函数
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
  
  // 持久化到 localStorage（G045 修复）
  try {
    localStorage.setItem('lrc-selected-scenario', scenario);
  } catch (e) {
    console.warn('[selectPresetScenario] localStorage 写入失败:', e);
  }
  
  // 触发场景加载逻辑
  loadPresetScenarioData(scenario);
}

window.selectPresetScenario = selectPresetScenario;
```

2. **页面加载时恢复选中状态**（在 init 或 DOMContentLoaded 中）：

```javascript
// 在初始化函数中添加
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

// 在 init() 或 DOMContentLoaded 中调用
restoreSelectedScenario();
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（selectPresetScenario 约 3721 行 + init 函数）

**验证方法**：

1. **CDP 测试 #3 验证**：
   ```javascript
   // 切换 4 个模板，active 类应正确切换
   const scenarios = ['project-management', 'code-review', 'api-doc', 'data-analysis'];
   for (const s of scenarios) {
     window.selectPresetScenario(s);
     const active = document.querySelector('.preset-scenario-template.active');
     active !== null && active.dataset.scenario === s  // 应返回 true
   }
   ```

2. **持久化验证**：
   ```javascript
   // 切换场景后刷新页面
   window.selectPresetScenario('code-review');
   location.reload();
   // 刷新后
   const active = document.querySelector('.preset-scenario-template.active');
   active.dataset.scenario === 'code-review'  // 应返回 true
   ```

3. **PowerShell 验证脚本**：
   ```powershell
   # 验证持久化逻辑
   $persist = Select-String -Path "G:\code-memory\static\app.js" -Pattern "localStorage\.setItem\('lrc-selected-scenario'"
   $restore = Select-String -Path "G:\code-memory\static\app.js" -Pattern "localStorage\.getItem\('lrc-selected-scenario'"
   if ($persist -and $restore) {
       Write-Host "PASS: selectPresetScenario 已持久化" -ForegroundColor Green
   } else {
       Write-Host "FAIL: 持久化逻辑缺失" -ForegroundColor Red
   }
   ```

**依赖 Step**：无

**预计工作量**：0.3 人天

**如果失败怎么办**：
- 若 CDP 测试仍检测 active=false → 检查 `.preset-scenario-template` 选择器是否匹配实际 DOM 结构
- 若刷新后状态丢失 → 检查 restoreSelectedScenario 是否在 init 中调用
- 若 localStorage 写入失败 → 检查浏览器隐私模式或 QuotaExceededError
- 回滚方案：`git checkout HEAD~1 -- static/app.js`（恢复无持久化状态）

---

### Step 6: 暴露 startAutoRefresh 到 window（修复 G019 + CDP 测试 #10）

**目标缺口**：G019（startAutoRefresh 未暴露到 window）+ CDP 测试 #10（G016 自动刷新函数已定义）

**严重程度**：P2（CDP 测试要求，但功能正常）

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 1.3 节 P2-10：
- [startAutoRefresh:1603](file:///g:/code-memory/static/app.js#L1603) 在 IIFE 内定义
- v0.8.3 Step 10 实现了 dashboardAbortController，但**遗漏了 `window.startAutoRefresh = startAutoRefresh` 暴露语句**
- CDP 测试 `typeof window.startAutoRefresh === 'function'` 返回 false
- 功能本身正常（内部调用正常），仅外部测试和动态调用不可达

**修复方案**：

在 [startAutoRefresh 函数定义后](file:///g:/code-memory/static/app.js#L1624) 添加暴露语句：

```javascript
// 在 app.js 约 1624 行，startAutoRefresh 函数闭合 } 之后添加：
window.startAutoRefresh = startAutoRefresh;
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（第 1624 行后插入 1 行）

**验证方法**：

1. **CDP 测试 #10 验证**：
   ```javascript
   typeof window.startAutoRefresh === 'function'  // 应返回 true
   ```

2. **PowerShell 验证脚本**：
   ```powershell
   $result = Select-String -Path "G:\code-memory\static\app.js" -Pattern "window\.startAutoRefresh\s*=\s*startAutoRefresh"
   if ($result) {
       Write-Host "PASS: startAutoRefresh 已暴露" -ForegroundColor Green
   } else {
       Write-Host "FAIL: startAutoRefresh 未暴露" -ForegroundColor Red
   }
   ```

3. **运行时验证**：`window.startAutoRefresh()` 应启动自动刷新（无报错）

**依赖 Step**：Step 1（调查结论确认）

**预计工作量**：0.1 人天

**如果失败怎么办**：
- 若暴露后调用报错 → 检查 startAutoRefresh 是否依赖 IIFE 内的私有变量
- 回滚方案：`git checkout HEAD~1 -- static/app.js`（仅回退 1 行）

---

### Step 7: 修复 G021/G033 AbortController 竞态 + loading 永久显示

**目标缺口**：G021（_abortActiveTabRequests 误 abort dashboardAbortController）+ G033（loadDashboard 被 abort 后 loading-overlay 永久显示）

**严重程度**：P1

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 6.3 节：
- [_abortActiveTabRequests:4838](file:///g:/code-memory/static/app.js#L4838) 无条件 abort dashboardAbortController
- switchTab 切换到 dashboard 时先调用 `_abortActiveTabRequests()`，abort 旧的 dashboardAbortController
- 然后 `loadDashboard()` 创建新的 dashboardAbortController
- **但 loadDashboard 内部 [396 行](file:///g:/code-memory/static/app.js#L396) 会 abort 上一次的 dashboardAbortController**
- 竞态：快速切换 dashboard → settings → dashboard，第二次 loadDashboard 可能被第一次的 abort 影响
- [loadDashboard:452](file:///g:/code-memory/static/app.js#L452) 捕获 AbortError 静默退出，**但 loading-overlay 未隐藏**

**修复方案**：

1. **修复 _abortActiveTabRequests 误 abort**（约 4838 行）：

```javascript
// 修改前（约 4838 行）：
function _abortActiveTabRequests() {
  // abort 所有标签页的 controller
  _tabAbortControllers.forEach(controller => {
    if (controller) controller.abort();
  });
  _tabAbortControllers.clear();
  
  // 误 abort dashboardAbortController
  if (typeof dashboardAbortController !== 'undefined' && dashboardAbortController) {
    dashboardAbortController.abort();
  }
}

// 修改后：dashboard 由 loadDashboard 自身管理，不在此处 abort
function _abortActiveTabRequests(excludeTab) {
  _tabAbortControllers.forEach((controller, tabName) => {
    // 跳过 excludeTab（通常是切换的目标标签）
    if (tabName === excludeTab) return;
    if (controller) controller.abort();
  });
  // 不再无条件 abort dashboardAbortController
  // dashboard 的请求取消由 loadDashboard 自身管理
}
```

2. **修改 switchTab 调用**（约 4776 行）：

```javascript
// 修改前：
function switchTab(tabName) {
  _abortActiveTabRequests();
  // ... 切换逻辑 ...
}

// 修改后：传入 excludeTab 参数
function switchTab(tabName) {
  _abortActiveTabRequests(tabName); // 排除目标标签
  // ... 切换逻辑 ...
}
```

3. **修复 loadDashboard AbortError 分支**（约 452 行）：

```javascript
// 修改前（约 452 行）：
} catch (e) {
  if (e.name === 'AbortError') {
    console.log('[loadDashboard] 请求被 abort（正常行为）');
    return; // loading-overlay 未隐藏！
  }
  // ... 其他错误处理 ...
}

// 修改后：
} catch (e) {
  if (e.name === 'AbortError') {
    console.log('[loadDashboard] 请求被 abort（正常行为）');
    // G033 修复：AbortError 时也要隐藏 loading-overlay
    const loading = document.getElementById('dashboard-loading');
    if (loading) loading.classList.add('hidden');
    return;
  }
  // ... 其他错误处理 ...
} finally {
  // 确保所有路径都隐藏 loading
  const loading = document.getElementById('dashboard-loading');
  if (loading) loading.classList.add('hidden');
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（_abortActiveTabRequests 约 4838 行 + switchTab 约 4776 行 + loadDashboard 约 452 行）

**验证方法**：

1. **G021 验证**（InteractionGuard）：
   ```javascript
   // 切换到 dashboard 时不应 abort dashboardAbortController
   const originalAbort = AbortController.prototype.abort;
   let abortCalled = false;
   AbortController.prototype.abort = function() {
     if (this === window.dashboardAbortController) abortCalled = true;
     return originalAbort.call(this);
   };
   window.switchTab('dashboard');
   await new Promise(r => setTimeout(r, 200));
   !abortCalled  // 应返回 true（未误 abort）
   AbortController.prototype.abort = originalAbort;
   ```

2. **G033 验证**：
   ```javascript
   // 切换标签页后 loading-overlay 应隐藏
   window.switchTab('dashboard');
   await new Promise(r => setTimeout(r, 500));
   window.switchTab('settings');
   await new Promise(r => setTimeout(r, 100));
   window.switchTab('dashboard');
   await new Promise(r => setTimeout(r, 500));
   const loading = document.getElementById('dashboard-loading');
   loading.classList.contains('hidden')  // 应返回 true
   ```

3. **PowerShell 验证脚本**：
   ```powershell
   # 验证 _abortActiveTabRequests 不再 abort dashboard
   $result = Select-String -Path "G:\code-memory\static\app.js" -Pattern "excludeTab"
   if ($result) {
       Write-Host "PASS: _abortActiveTabRequests 已增加 excludeTab 参数" -ForegroundColor Green
   } else {
       Write-Host "FAIL: 仍无条件 abort dashboard" -ForegroundColor Red
   }
   
   # 验证 loadDashboard AbortError 分支隐藏 loading
   $abortLoading = Select-String -Path "G:\code-memory\static\app.js" -Pattern "AbortError" -Context 0,3
   $hasLoadingHide = $abortLoading | Where-Object { $_.Context.PostContext -match "loading.*hidden" }
   if ($hasLoadingHide) {
       Write-Host "PASS: AbortError 分支隐藏 loading" -ForegroundColor Green
   } else {
       Write-Host "FAIL: AbortError 分支未隐藏 loading" -ForegroundColor Red
   }
   ```

**依赖 Step**：无

**预计工作量**：0.7 人天

**如果失败怎么办**：
- 若 excludeTab 参数导致旧请求未取消 → 检查 switchTab 调用处是否正确传参
- 若 loading-overlay 仍永久显示 → 检查 finally 块是否在所有路径执行
- 回滚方案：`git checkout HEAD~1 -- static/app.js`（恢复误 abort + loading 卡死状态）

---

### Step 8: 修复 G026 Toast 去重边界 bug（修复 CDP 测试 #6）

**目标缺口**：G026（Toast 去重边界 bug，首条消息可能被误去重）+ CDP 测试 #6（G011 重复消息去重剩 0 个）

**严重程度**：P1

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 1.3 节 P1-6：
- [showToast:4689](file:///g:/code-memory/static/app.js#L4689) 去重逻辑：`if (_recentToastMessages.has(dedupKey)) { if (now - last < 1500) return; }`
- **边界 bug**：若测试前已有相同 dedupKey 记录（前 1.5s 内触发过），则第 1 次调用也被去重跳过
- 导致 5 次相同消息全部不显示（应为 1 个）
- 另外 [4706 行](file:///g:/code-memory/static/app.js#L4706) `visibleToasts.length >= 3` 上限检查，若已有 3 个 Toast 则新消息被丢弃

**修复方案**：

修改 [showToast](file:///g:/code-memory/static/app.js#L4682) 去重逻辑，改为"显示后才记录 dedupKey"：

```javascript
// 修改前（约 4689 行）：
function showToast(message, type = 'info', duration = 4000) {
  const now = Date.now();
  const dedupKey = `${type}:${message}`;
  
  // 去重检查（显示前）
  if (_recentToastMessages.has(dedupKey)) {
    const last = _recentToastMessages.get(dedupKey);
    if (now - last < _TOAST_DEDUP_WINDOW) return; // 1.5s 内重复，跳过
  }
  _recentToastMessages.set(dedupKey, now); // 显示前记录（BUG）
  
  // ... 创建 Toast ...
}

// 修改后：
function showToast(message, type = 'info', duration = 4000) {
  const now = Date.now();
  const dedupKey = `${type}:${message}`;
  
  // 去重检查（显示前检查，但不在此时记录）
  if (_recentToastMessages.has(dedupKey)) {
    const last = _recentToastMessages.get(dedupKey);
    if (now - last < _TOAST_DEDUP_WINDOW) {
      // 重复消息，跳过显示（但更新时间戳，避免后续显示的 Toast 被误判）
      _recentToastMessages.set(dedupKey, now);
      return;
    }
  }
  
  // 可见上限检查（在去重之后，避免去重消息占用配额）
  const container = document.getElementById('toast-container');
  if (container) {
    const visibleToasts = container.querySelectorAll('.toast:not(.toast-leaving)');
    if (visibleToasts.length >= TOAST_MAX_VISIBLE) {
      if (type === 'error') {
        // error 优先，移除最旧的非 error Toast
        const oldest = Array.from(visibleToasts).find(t => !t.classList.contains('toast-error'));
        if (oldest) oldest.remove();
      } else {
        return; // 非 error 超出上限，跳过
      }
    }
  }
  
  // 显示后才记录 dedupKey（G026 修复：避免首条被误去重）
  _recentToastMessages.set(dedupKey, now);
  
  // 定期清理过期 dedupKey（避免 Map 无限增长）
  if (_recentToastMessages.size > 100) {
    const expiredKeys = [];
    _recentToastMessages.forEach((time, key) => {
      if (now - time > _TOAST_DEDUP_WINDOW * 2) expiredKeys.push(key);
    });
    expiredKeys.forEach(key => _recentToastMessages.delete(key));
  }
  
  // ... 创建并显示 Toast ...
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（showToast 约 4682-4721 行）

**验证方法**：

1. **CDP 测试 #6 验证**：
   ```javascript
   // 清理 _recentToastMessages（如可访问）
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

2. **InteractionGuard 验证**：`ToastDedupGuard.assertFirstToastNotDeduplicated()` 通过

3. **PowerShell 验证脚本**：
   ```powershell
   # 验证去重逻辑改为"显示后记录"
   $result = Select-String -Path "G:\code-memory\static\app.js" -Pattern "_recentToastMessages\.set" 
   $setDisplayAfter = $result | Where-Object { $_.LineNumber -gt 4700 }
   if ($setDisplayAfter) {
       Write-Host "PASS: dedupKey 在显示后记录" -ForegroundColor Green
   } else {
       Write-Host "FAIL: dedupKey 仍在显示前记录" -ForegroundColor Red
   }
   ```

**依赖 Step**：无

**预计工作量**：0.3 人天

**如果失败怎么办**：
- 若首条仍被去重 → 检查是否有其他地方在 showToast 入口前记录 dedupKey
- 若 5 次调用显示 5 个 → 检查去重窗口时间戳是否正确更新
- 回滚方案：`git checkout HEAD~1 -- static/app.js`（恢复去重边界 bug）

---

### Step 9: 修复 G025/G032/G040 动态按钮 onclick XSS + 防抖绕过

**目标缺口**：G025（openMemoryDetail onclick 注入 JSON.stringify(memory)）+ G032（动态生成按钮 onclick 残留，绕过防抖）+ G040（CSP 缺失 + memory_type 未转义）

**严重程度**：P1

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 4.1 节和 7 节：
- [openMemoryDetail:4154](file:///g:/code-memory/static/app.js#L4154) `onclick='openMemoryDetail(...)'` 注入 JSON.stringify(memory)，XSS 风险
- [app.js:4149](file:///g:/code-memory/static/app.js#L4149) typeClass 拼接，memory_type 未转义
- 动态按钮 onclick 绕过 bindAllActions 防抖，10 次点击触发 10 次请求
- [app.js:1359](file:///g:/code-memory/static/app.js#L1359) 和 [app.js:5730](file:///g:/code-memory/static/app.js#L5730) 也有内联 onclick 残留

**修复方案**：

1. **修复 openMemoryDetail 的 onclick 注入**（约 4154 行）：

```javascript
// 修改前（约 4154 行）：
card.innerHTML = `<div onclick='openMemoryDetail(${JSON.stringify(memory)})'>...</div>`;

// 修改后：使用 data-memory-id + 事件委托
card.innerHTML = `<div data-action="open-memory-detail" data-memory-id="${escapeHtml(memory.id)}">...</div>`;
```

2. **修复 memory_type 转义**（约 4149 行）：

```javascript
// 修改前：
const typeClass = `card-memory-${memory.memory_type}`;

// 修改后：白名单校验 + 转义
function sanitizeMemoryType(type) {
  // 仅允许字母、数字、下划线、连字符
  const safe = String(type || '').replace(/[^a-zA-Z0-9_-]/g, '');
  return safe || 'unknown';
}
const typeClass = `card-memory-${sanitizeMemoryType(memory.memory_type)}`;
```

3. **添加 escapeHtml 工具函数**（如不存在）：

```javascript
function escapeHtml(str) {
  if (str == null) return '';
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}
```

4. **在 bindAllActions 中添加 open-memory-detail action**（约 5892 行）：

```javascript
case 'open-memory-detail': {
  const memoryId = el.dataset.memoryId;
  if (memoryId) openMemoryDetail(memoryId);
  break;
}
```

5. **修改 openMemoryDetail 签名**（约 4190 行）：

```javascript
// 修改前：接收完整 memory 对象
function openMemoryDetail(memory) { ... }

// 修改后：接收 memoryId，内部查找
async function openMemoryDetail(memoryId) {
  // 从缓存或 API 获取 memory 详情
  let memory = memoryCache.get(memoryId);
  if (!memory) {
    try {
      memory = await fetchMemoryById(memoryId);
      memoryCache.set(memoryId, memory);
    } catch (e) {
      showToast('加载记忆详情失败：' + e.message, 'error');
      return;
    }
  }
  // ... 原有渲染逻辑 ...
}
```

6. **修复其他内联 onclick 残留**：
   - [app.js:1359](file:///g:/code-memory/static/app.js#L1359) `onclick="handleOpenDataDirClick()"` → 改用 data-action="open-data-dir"
   - [app.js:5730](file:///g:/code-memory/static/app.js#L5730) 动态生成 → 改用 data-action + 事件委托

7. **添加 CSP 头**（在 index.html 或 Tauri 配置中）：

```html
<!-- 在 index.html 的 <head> 中添加 -->
<meta http-equiv="Content-Security-Policy" 
      content="default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' http://localhost:* http://127.0.0.1:*; img-src 'self' data:;">
```

**注意**：CSP 不允许 'unsafe-inline'（项目硬约束），style-src 的 'unsafe-inline' 是 Tauri WebView 限制，需确认是否可移除。

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（openMemoryDetail 约 4154/4190 行 + bindAllActions 约 5892 行 + 1359/5730 行）
- [G:\code-memory\static\index.html](file:///g:/code-memory/static/index.html)（添加 CSP meta 标签）

**验证方法**：

1. **G025 XSS 验证**：
   ```javascript
   // 构造 memory_type 含恶意字符
   const maliciousMemory = { id: '1', memory_type: 'test"><script>alert(1)</script>', content: '...' };
   // 渲染后检查 class 是否转义
   const card = document.querySelector('.card-memory-testscriptalert1script');
   card === null  // 应返回 true（恶意字符被移除）
   ```

2. **G032 防抖验证**：
   ```javascript
   // 快速点击记忆卡片 10 次
   const card = document.querySelector('[data-action="open-memory-detail"]');
   for (let i = 0; i < 10; i++) card.click();
   // 应只触发 1 次 openMemoryDetail（inFlight 防抖）
   ```

3. **G040 CSP 验证**：
   ```javascript
   // 检查 CSP 头是否存在
   const cspMeta = document.querySelector('meta[http-equiv="Content-Security-Policy"]');
   cspMeta !== null  // 应返回 true
   ```

4. **PowerShell 验证脚本**：
   ```powershell
   # 验证无内联 onclick
   $inlineOnclick = Select-String -Path "G:\code-memory\static\app.js" -Pattern "onclick\s*=" | Where-Object { $_.Line -notmatch "//|/\*|escapeHtml|addEventListener" }
   if ($inlineOnclick.Count -eq 0) {
       Write-Host "PASS: 无内联 onclick 残留" -ForegroundColor Green
   } else {
       Write-Host "FAIL: 仍有内联 onclick:" -ForegroundColor Red
       $inlineOnclick | ForEach-Object { Write-Host "  L$($_.LineNumber): $($_.Line.Trim())" }
   }
   
   # 验证 memory_type 转义
   $sanitize = Select-String -Path "G:\code-memory\static\app.js" -Pattern "sanitizeMemoryType|escapeHtml"
   if ($sanitize) {
       Write-Host "PASS: memory_type 转义函数已添加" -ForegroundColor Green
   } else {
       Write-Host "FAIL: 未添加转义函数" -ForegroundColor Red
   }
   ```

**依赖 Step**：Step 2（showConfirm 暴露，bindAllActions 正常）

**预计工作量**：0.8 人天

**如果失败怎么办**：
- 若 openMemoryDetail 改为接收 memoryId 后无法获取 memory → 检查 memoryCache 实现，确保 searchMemories 时已缓存
- 若 CSP 添加后样式错乱 → 检查 style-src 是否需要保留 'unsafe-inline'（Tauri WebView 限制）
- 若 data-action 不触发 → 检查 bindAllActions 的 action 映射是否正确
- 回滚方案：`git checkout HEAD~1 -- static/app.js static/index.html`（恢复 onclick + XSS 风险）

---

### Step 10: 修复 G029/G047 500 重试次数上限 + 标签页切换竞态

**目标缺口**：G029（handleHttpError 的 500 重试无次数上限）+ G047（500 重试 Modal 显示后切换标签页，modal 仍显示但数据已 abort）

**严重程度**：P1

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 3.2 节和 7 节：
- [handleHttpError:187](file:///g:/code-memory/static/app.js#L187) 显示 showConfirm"是否重试？"
- 用户点击"确认" → 重新发起请求 → 又 500 → 又显示 showConfirm
- **无限循环**，用户可无限点击重试
- 无重试次数上限、无指数退避、无"查看日志"引导
- 另外，重试 Modal 显示时用户切换标签页，modal 仍显示但数据已 abort（G047）

**修复方案**：

1. **修改 handleHttpError 增加重试次数上限**（约 187 行）：

```javascript
// 维护重试计数器 Map（按请求 URL 区分）
const _retryCounters = new Map();
const MAX_RETRY_COUNT = 3;

async function handleHttpError(error, context = {}) {
  const { url, method = 'GET' } = context;
  const retryKey = `${method}:${url}`;
  
  // ... 现有错误分类逻辑 ...
  
  if (error.status === 500) {
    const retryCount = _retryCounters.get(retryKey) || 0;
    
    if (retryCount >= MAX_RETRY_COUNT) {
      // 超过重试上限，显示"停止重试"+ 引导
      _retryCounters.delete(retryKey); // 重置计数
      await showInfoModal('多次重试失败', 
        `请求 ${url} 已连续失败 ${MAX_RETRY_COUNT} 次。\n\n` +
        `可能原因：\n` +
        `• 服务端内部错误\n` +
        `• 数据库锁冲突\n` +
        `• 资源不足\n\n` +
        `建议操作：\n` +
        `• 查看 LRC 服务日志\n` +
        `• 重启 LRC 服务\n` +
        `• 联系技术支持`, 
        { okText: '查看日志' }
      );
      // 打开日志查看（如存在）
      if (typeof openLogViewer === 'function') openLogViewer();
      return { action: 'giveup' };
    }
    
    // 询问是否重试
    const shouldRetry = await showConfirm(
      `请求失败（第 ${retryCount + 1} 次重试）\n错误：${error.message}\n\n是否重试？`,
      '服务器错误',
      0
    );
    
    if (shouldRetry) {
      _retryCountors.set(retryKey, retryCount + 1);
      // 指数退避：1s → 2s → 4s
      const backoff = Math.pow(2, retryCount) * 1000;
      await new Promise(r => setTimeout(r, backoff));
      return { action: 'retry' };
    } else {
      _retryCounters.delete(retryKey);
      return { action: 'giveup' };
    }
  }
  
  // ... 其他错误处理 ...
}

// 成功时重置计数器（在 fetchWithTimeout 成功路径调用）
function resetRetryCounter(url, method = 'GET') {
  _retryCounters.delete(`${method}:${url}`);
}
```

2. **修复 G047 标签页切换竞态**（约 187 行 + switchTab）：

```javascript
// 在 handleHttpError 显示重试 Modal 时，禁用标签页切换
let _retryModalActive = false;

async function handleHttpError(error, context = {}) {
  // ... 500 处理 ...
  if (shouldRetry || retryCount < MAX_RETRY_COUNT) {
    _retryModalActive = true;
    try {
      const result = await showConfirm(...);
      return result ? { action: 'retry' } : { action: 'giveup' };
    } finally {
      _retryModalActive = false;
    }
  }
}

// 修改 switchTab，重试 Modal 显示时禁止切换
function switchTab(tabName) {
  if (_retryModalActive) {
    showToast('请先处理重试弹窗', 'warning');
    return false;
  }
  // ... 原有切换逻辑 ...
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（handleHttpError 约 167-214 行 + switchTab 约 4773 行）

**验证方法**：

1. **G029 重试上限验证**：
   ```javascript
   // 模拟 500 错误连续重试
   for (let i = 0; i < 5; i++) {
     const result = await handleHttpError({ status: 500, message: 'test' }, { url: '/test' });
     if (i >= 3 && result.action === 'giveup') break;
   }
   // 第 4 次应显示 showInfoModal 而非 showConfirm
   ```

2. **G047 标签页切换验证**：
   ```javascript
   // 重试 Modal 显示时切换标签页
   handleHttpError({ status: 500, message: 'test' }, { url: '/test' });
   await new Promise(r => setTimeout(r, 100));
   const result = window.switchTab('settings');
   result === false  // 应返回 false（禁止切换）
   ```

3. **PowerShell 验证脚本**：
   ```powershell
   # 验证重试次数上限
   $result = Select-String -Path "G:\code-memory\static\app.js" -Pattern "MAX_RETRY_COUNT|_retryCounters"
   if ($result) {
       Write-Host "PASS: 重试次数上限已实现" -ForegroundColor Green
   } else {
       Write-Host "FAIL: 无重试次数上限" -ForegroundColor Red
   }
   
   # 验证指数退避
   $backoff = Select-String -Path "G:\code-memory\static\app.js" -Pattern "Math\.pow\(2.*retryCount"
   if ($backoff) {
       Write-Host "PASS: 指数退避已实现" -ForegroundColor Green
   } else {
       Write-Host "WARN: 未实现指数退避" -ForegroundColor Yellow
   }
   ```

**依赖 Step**：Step 2（showConfirm 暴露）+ Step 3（showInfoModal 入队，避免与 showConfirm 冲突）

**预计工作量**：0.7 人天

**如果失败怎么办**：
- 若重试计数器不重置 → 在请求成功路径调用 resetRetryCounter
- 若指数退避导致用户等待过久 → 上限调整为 2s（1s → 2s → 2s）
- 若 switchTab 禁止切换导致用户卡死 → 在 Modal 上添加"切换并放弃重试"选项
- 回滚方案：`git checkout HEAD~1 -- static/app.js`（恢复无限重试）

---

### Step 11: 修复 G037/G038 searchMemories 无 AbortController

**目标缺口**：G037（searchMemories 无 AbortController，快速输入触发竞态）+ G038（debouncedMemorySearch 防抖期间请求仍进行）

**严重程度**：P1

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 7 节：
- [searchMemories:4116](file:///g:/code-memory/static/app.js#L4116) fetchWithTimeout 无 signal
- [debouncedMemorySearch:4081](file:///g:/code-memory/static/app.js#L4081) 300ms 防抖但无 AbortController
- 快速输入触发多个请求，旧请求覆盖新结果

**修复方案**：

```javascript
// 新增 searchAbortController（约 4080 行附近）
let searchAbortController = null;

// 修改 searchMemories（约 4106 行）
async function searchMemories(query) {
  // abort 上一次未完成的搜索请求
  if (searchAbortController) {
    searchAbortController.abort();
  }
  searchAbortController = new AbortController();
  
  try {
    const res = await fetchWithTimeout(`${API_BASE}/v1/memories/search`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ query, limit: 20 }),
      signal: searchAbortController.signal // 传入 signal
    });
    
    const data = await safeJson(res);
    resetRetryCounter('/v1/memories/search', 'POST'); // 成功时重置重试计数
    renderSearchResults(data);
  } catch (e) {
    if (e.name === 'AbortError') {
      console.log('[searchMemories] 请求被 abort（正常行为）');
      return; // 静默处理
    }
    // ... 其他错误处理 ...
  } finally {
    searchAbortController = null;
  }
}

// 修改 debouncedMemorySearch（约 4081 行）
function debouncedMemorySearch(query) {
  clearTimeout(_searchDebounceTimer);
  _searchDebounceTimer = setTimeout(() => {
    searchMemories(query);
  }, 300);
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（searchMemories 约 4106 行 + debouncedMemorySearch 约 4081 行）

**验证方法**：

1. **G037/G038 验证**：
   ```javascript
   // 快速输入 5 个字符
   const input = document.getElementById('memory-search-input');
   for (const ch of ['a', 'b', 'c', 'd', 'e']) {
     input.value += ch;
     input.dispatchEvent(new Event('input'));
     await new Promise(r => setTimeout(r, 50)); // 小于 300ms 防抖
   }
   // 仅最后 1 次请求完成（前 4 次被 abort）
   const results = document.querySelectorAll('.memory-search-result');
   // 验证结果对应 'abcde' 而非中间状态
   ```

2. **PowerShell 验证脚本**：
   ```powershell
   $result = Select-String -Path "G:\code-memory\static\app.js" -Pattern "searchAbortController"
   if ($result.Count -ge 3) {
       Write-Host "PASS: searchMemories AbortController 已实现" -ForegroundColor Green
   } else {
       Write-Host "FAIL: 未实现 AbortController" -ForegroundColor Red
   }
   ```

**依赖 Step**：无

**预计工作量**：0.5 人天

**如果失败怎么办**：
- 若 abort 后结果显示空白 → 检查 AbortError 分支是否 return 前清理骨架屏
- 若快速输入仍触发多个请求 → 检查 debouncedMemorySearch 的 clearTimeout 逻辑
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 12: 修复 G023 start-service-modal Tab 焦点陷阱 + N08 模态框显示（修复 CDP 测试 #4, #8）

**目标缺口**：G023（start-service-modal 无 Tab 焦点陷阱）+ CDP 测试 #4（N08 启动服务模态框 ESC 关闭，模态框未能显示）+ CDP 测试 #8（G013 模态框有可聚焦元素）

**严重程度**：P1

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 1.3 节 P1-4 和 4.5 节：
- [openStartServiceModal:890](file:///g:/code-memory/static/app.js#L890) 仅添加 ESC 监听，无 Tab 陷阱
- CDP 测试报告"模态框未能显示"，根因：CSS 可能有 `display:none !important` 覆盖
- [handleStartServiceEsc:921](file:///g:/code-memory/static/app.js#L921) 已实现 ESC，但模态框未显示时 ESC 无意义

**修复方案**：

1. **修复 start-service-modal 显示问题**（约 890 行）：

```javascript
// 修改 openStartServiceModal（约 890 行）
function openStartServiceModal() {
  const modal = document.getElementById('start-service-modal');
  if (!modal) {
    console.error('[openStartServiceModal] start-service-modal 元素不存在');
    return;
  }
  
  // 移除 hidden 属性（避免 [hidden] 与 display:flex 冲突）
  modal.removeAttribute('hidden');
  // 使用 class 控制 display，避免 inline style 被 CSS 覆盖
  modal.classList.add('modal-visible');
  modal.style.display = 'flex';
  // 强制重排，确保 display 生效
  void modal.offsetHeight;
  
  // 验证模态框可见
  if (modal.offsetParent === null) {
    console.error('[openStartServiceModal] 模态框仍不可见，检查 CSS');
    showToast('启动服务窗口无法显示，请刷新页面重试', 'error');
    return;
  }
  
  // 添加 ESC 监听
  document.addEventListener('keydown', handleStartServiceEsc);
  
  // 添加 Tab 焦点陷阱（G023 修复）
  modal.addEventListener('keydown', onStartServiceTabTrap);
  
  // 聚焦到模态框内首个可聚焦元素
  const focusable = modal.querySelectorAll('button:not([disabled]), input:not([disabled])');
  if (focusable.length > 0) {
    focusable[0].focus();
  }
}

// Tab 焦点陷阱（复用 onTabTrap 逻辑）
function onStartServiceTabTrap(e) {
  if (e.key !== 'Tab') return;
  const modal = document.getElementById('start-service-modal');
  if (!modal) return;
  
  const focusable = modal.querySelectorAll('button:not([disabled]):not([style*="display: none"]), input:not([disabled])');
  if (focusable.length === 0) return;
  
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  
  if (e.shiftKey) {
    if (document.activeElement === first) {
      e.preventDefault();
      last.focus();
    }
  } else {
    if (document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }
}

// 修改 closeStartServiceModal（约 925 行）
function closeStartServiceModal() {
  const modal = document.getElementById('start-service-modal');
  if (!modal) return;
  
  modal.style.display = 'none';
  modal.setAttribute('hidden', '');
  modal.classList.remove('modal-visible');
  
  // 移除监听
  document.removeEventListener('keydown', handleStartServiceEsc);
  modal.removeEventListener('keydown', onStartServiceTabTrap);
}
```

2. **检查 CSS 是否有 display:none !important 覆盖**：

```css
/* 在 app.css 中检查 start-service-modal 相关样式 */
/* 如有 [hidden] { display: none !important; }，需调整为：*/
[hidden] { display: none !important; }
/* 但 start-service-modal.modal-visible 优先级更高 */
#start-service-modal.modal-visible {
  display: flex !important;
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（openStartServiceModal 约 890 行 + closeStartServiceModal 约 925 行）
- [G:\code-memory\static\app.css](file:///g:/code-memory/static/app.css)（start-service-modal 样式，如有冲突）

**验证方法**：

1. **CDP 测试 #4 验证**：
   ```javascript
   // 打开模态框
   window.openStartServiceModal();
   await new Promise(r => setTimeout(r, 100));
   
   const modal = document.getElementById('start-service-modal');
   modal.offsetParent !== null  // 应返回 true（模态框可见）
   
   // ESC 关闭
   document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
   await new Promise(r => setTimeout(r, 100));
   modal.offsetParent === null  // 应返回 true（模态框已关闭）
   ```

2. **CDP 测试 #8 验证**（模态框有可聚焦元素）：
   ```javascript
   window.openStartServiceModal();
   const modal = document.getElementById('start-service-modal');
   const focusable = modal.querySelectorAll('button:not([disabled]), input:not([disabled])');
   focusable.length > 0  // 应返回 true
   ```

3. **G023 焦点陷阱验证**：
   ```javascript
   window.openStartServiceModal();
   const modal = document.getElementById('start-service-modal');
   const focusable = modal.querySelectorAll('button:not([disabled]), input:not([disabled])');
   // Tab 在模态框内循环
   focusable[focusable.length - 1].focus();
   modal.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
   document.activeElement === focusable[0]  // 应返回 true（焦点循环到首个）
   ```

**依赖 Step**：Step 3（processConfirmQueue 焦点陷阱逻辑可复用）

**预计工作量**：0.5 人天

**如果失败怎么办**：
- 若模态框仍不可见 → 检查 CSS 优先级，用 DevTools 查看计算样式
- 若 Tab 陷阱不生效 → 检查 keydown 事件是否冒泡到 modal
- 回滚方案：`git checkout HEAD~1 -- static/app.js static/app.css`

---

### Step 13: 修复 G015 必填字段空值校验（修复 CDP 测试 #9）

**目标缺口**：G015（必填字段空值校验失败）+ CDP 测试 #9（G015 必填字段空值校验失败，hasErrorClass=undefined）

**严重程度**：P1

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 1.3 节 P1-9：
- [setupInputValidation:1039](file:///g:/code-memory/static/app.js#L1039) 已注册 blur 监听
- [validateInput:1013](file:///g:/code-memory/static/app.js#L1013) 失败时 `input.classList.add('input-error')`
- CDP 测试可能用 `input.value = ''` + `input.blur()` 但未触发 blur 事件（需 dispatchEvent）
- 或测试的 input 不在 INPUT_VALIDATORS 规则表中

**修复方案**：

1. **增强 setupInputValidation，确保所有必填字段被覆盖**（约 1039 行）：

```javascript
// 修改 setupInputValidation（约 1039 行）
function setupInputValidation() {
  // 扫描所有带 required 属性或 data-required 的输入框
  const requiredInputs = document.querySelectorAll('input[required], input[data-required], textarea[required]');
  
  requiredInputs.forEach(input => {
    // 避免重复绑定
    if (input.dataset.validationBound === 'true') return;
    input.dataset.validationBound = 'true';
    
    input.addEventListener('blur', () => validateInput(input));
    input.addEventListener('focus', () => clearInputError(input));
    
    // 增加 input 事件实时校验（可选，仅清除错误）
    input.addEventListener('input', () => {
      if (input.classList.contains('input-error') && input.value.trim()) {
        clearInputError(input);
      }
    });
  });
  
  // 补充 INPUT_VALIDATORS 中定义但无 required 属性的字段
  Object.keys(INPUT_VALIDATORS).forEach(id => {
    const input = document.getElementById(id);
    if (input && input.dataset.validationBound !== 'true') {
      input.dataset.validationBound = 'true';
      input.addEventListener('blur', () => validateInput(input));
      input.addEventListener('focus', () => clearInputError(input));
    }
  });
}

// 在 init 中调用
setupInputValidation();

// 用 MutationObserver 监听动态添加的输入框
const inputObserver = new MutationObserver(mutations => {
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
});
inputObserver.observe(document.body, { childList: true, subtree: true });
```

2. **增强 validateInput，确保错误状态可见**（约 1013 行）：

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
  
  // 校验通过
  input.classList.remove('input-error');
  input.removeAttribute('aria-invalid');
  clearInputError(input);
  return true;
}

function showInputError(input, message) {
  let errorEl = input.parentElement.querySelector('.input-error-message');
  if (!errorEl) {
    errorEl = document.createElement('div');
    errorEl.className = 'input-error-message';
    errorEl.style.cssText = 'color:#dc3545;font-size:12px;margin-top:4px;';
    input.parentElement.appendChild(errorEl);
  }
  errorEl.textContent = message;
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（setupInputValidation 约 1039 行 + validateInput 约 1013 行）

**验证方法**：

1. **CDP 测试 #9 验证**：
   ```javascript
   // 找一个必填输入框
   const input = document.querySelector('input[required]');
   input.value = '';
   input.dispatchEvent(new Event('blur')); // 触发 blur 事件
   
   input.classList.contains('input-error')  // 应返回 true
   input.getAttribute('aria-invalid')  // 应返回 'true'
   ```

2. **PowerShell 验证脚本**：
   ```powershell
   # 验证动态绑定
   $result = Select-String -Path "G:\code-memory\static\app.js" -Pattern "MutationObserver|validationBound"
   if ($result) {
       Write-Host "PASS: 动态输入框校验已实现" -ForegroundColor Green
   } else {
       Write-Host "FAIL: 未实现动态绑定" -ForegroundColor Red
   }
   ```

**依赖 Step**：无

**预计工作量**：0.3 人天

**如果失败怎么办**：
- 若 blur 后无错误类 → 检查 INPUT_VALIDATORS 是否包含目标字段，或用 required 属性触发
- 若 MutationObserver 性能问题 → 限制观察范围到特定容器
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 14: 修复 G004 CDP 测试检测问题（修复 CDP 测试 #5）

**目标缺口**：G004（bindAllActions inFlight 防抖）+ CDP 测试 #5（G004 bindAllActions 有 inFlight 防抖，hasInFlight=undefined）

**严重程度**：P1

**根因分析**：

根据 [INTERACTION_AUDIT_REPORT_v0.8.3.md](file:///g:/code-memory/docs/INTERACTION_AUDIT_REPORT_v0.8.3.md) 1.3 节 P1-5 和 Step 1 调查结论：
- [bindAllActions:5892](file:///g:/code-memory/static/app.js#L5892) 有 `el.dataset.inFlight === '1'` 检查
- [5928 行](file:///g:/code-memory/static/app.js#L5928) 有 `setTimeout(() => { delete el.dataset.inFlight; }, 500)` 解锁
- **代码已存在**，但 CDP 测试检测不到（hasInFlight=undefined）
- 根因：CDP 测试用正则 `dataset\.inFlight` 搜索 fn.toString()，但实际代码是 `el.dataset.inFlight`（驼峰访问），可能正则不匹配
- 或 CDP 测试搜索 `setTimeout` 但未匹配到 `delete el.dataset.inFlight`

**修复方案**（基于 Step 1 调查结论 C：测试检测问题）：

1. **修改 CDP 测试脚本**，改用运行时检测而非源码字符串匹配：

```javascript
// 修改 scripts/test_desktop_cdp_v083.js 中的 G004 测试
// 修改前：通过 fn.toString() 检测
const bindAllActionsSrc = await page.evaluate(() => window.bindAllActions.toString());
const hasInFlight = /dataset\.inFlight/.test(bindAllActionsSrc);
const hasSetTimeout = /setTimeout/.test(bindAllActionsSrc);

// 修改后：运行时行为检测
const result = await page.evaluate(async () => {
  // 找一个有 data-action 的按钮
  const btn = document.querySelector('[data-action]');
  if (!btn) return { error: '无 data-action 按钮' };
  
  // 记录点击次数
  let clickCount = 0;
  const originalAction = btn.dataset.action;
  
  // 模拟 10 次快速点击
  for (let i = 0; i < 10; i++) {
    btn.click();
  }
  
  // 等待 600ms（超过 500ms 防抖窗口）
  await new Promise(r => setTimeout(r, 600));
  
  // 检查 inFlight 状态
  const hasInFlight = btn.dataset.inFlight !== undefined;
  
  return {
    hasInFlight,
    clickCount,
    note: '运行时检测 inFlight 防抖'
  };
});
```

2. **在 bindAllActions 中添加注释，便于源码检测（兼容旧测试）**：

```javascript
// 在 bindAllActions 的 inFlight 检查处添加明确注释（约 5892 行）
function bindAllActions() {
  // G004 inFlight 防抖：防止快速点击触发多次请求
  document.querySelectorAll('[data-action]').forEach(el => {
    el.addEventListener('click', async (e) => {
      // inFlight 检查（dataset.inFlight）
      if (el.dataset.inFlight === '1') {
        e.preventDefault();
        return;
      }
      el.dataset.inFlight = '1';
      
      try {
        // ... action 处理 ...
      } finally {
        // setTimeout 解锁（500ms）
        setTimeout(() => { 
          delete el.dataset.inFlight; 
        }, 500);
      }
    });
  });
}
```

**影响文件**：
- [G:\code-memory\scripts\test_desktop_cdp_v083.js](file:///g:/code-memory/scripts/test_desktop_cdp_v083.js)（G004 测试用例）
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（bindAllActions 注释，约 5892 行）

**验证方法**：

1. **CDP 测试 #5 验证**（修改后测试脚本）：
   ```javascript
   // 运行时检测：快速点击 10 次应只触发 1 次
   const btn = document.querySelector('[data-action]');
   let triggerCount = 0;
   const originalHandler = btn.onclick;
   
   for (let i = 0; i < 10; i++) {
     btn.click();
   }
   
   // inFlight 应在 500ms 后解锁
   await new Promise(r => setTimeout(r, 600));
   btn.dataset.inFlight === undefined  // 应返回 true（已解锁）
   ```

2. **PowerShell 验证脚本**：
   ```powershell
   # 验证 bindAllActions 源码含 inFlight 检查
   $result = Select-String -Path "G:\code-memory\static\app.js" -Pattern "dataset\.inFlight|inFlight.*===.*1"
   if ($result.Count -ge 2) {
       Write-Host "PASS: bindAllActions 含 inFlight 防抖" -ForegroundColor Green
       $result | ForEach-Object { Write-Host "  L$($_.LineNumber): $($_.Line.Trim())" }
   } else {
       Write-Host "FAIL: 未找到 inFlight 防抖" -ForegroundColor Red
   }
   ```

**依赖 Step**：Step 1（调查结论确认是测试检测问题）

**预计工作量**：0.2 人天

**如果失败怎么办**：
- 若 Step 1 调查结论是 A（回滚）→ 需要重新应用 inFlight 防抖代码（参考 v0.8.3 Step 6）
- 若运行时检测仍失败 → 检查 bindAllActions 是否在 DOMContentLoaded 后正确绑定
- 回滚方案：`git checkout HEAD~1 -- scripts/test_desktop_cdp_v083.js static/app.js`

---

### Step 15: 修复 G024/G027/G030/G041 中优先级问题

**目标缺口**：G024（输入框无 Enter 提交）+ G027（offline-mode 未禁用按钮）+ G030（switchLlmProvider 暴露）+ G041（移动端 tabbar 同步）

**严重程度**：P2

**根因分析**：

- G024：[wizard-search-path](file:///g:/code-memory/static/app.js) 等输入框按 Enter 无响应
- G027：[app.js:5996](file:///g:/code-memory/static/app.js#L5996) body.offline-mode 无对应 CSS 禁用按钮
- G030：[index.html:1944](file:///g:/code-memory/static/index.html#L1944) select#llm-provider 用 data-input-action="switchLlmProvider"，但函数可能未定义
- G041：[switchTab:4773](file:///g:/code-memory/static/app.js#L4773) 未更新 .mobile-tabbar .tab-item

**修复方案**：

1. **G024 输入框 Enter 提交**（在 setupInputValidation 或单独函数中）：

```javascript
function setupEnterSubmit() {
  // 为带 data-enter-action 的输入框添加 Enter 提交
  document.querySelectorAll('input[data-enter-action]').forEach(input => {
    if (input.dataset.enterBound === 'true') return;
    input.dataset.enterBound = 'true';
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        const actionBtn = document.querySelector(`[data-action="${input.dataset.enterAction}"]`);
        if (actionBtn) actionBtn.click();
      }
    });
  });
}

// 在 HTML 中为输入框添加 data-enter-action 属性
// <input id="wizard-search-path" data-enter-action="wizardStep1Search">
```

2. **G027 offline-mode 禁用按钮**（在 app.css 中）：

```css
/* offline-mode 下禁用 API 按钮 */
body.offline-mode .btn-primary[data-action],
body.offline-mode .btn-api {
  pointer-events: none;
  opacity: 0.5;
  cursor: not-allowed;
}

/* 但允许重试按钮 */
body.offline-mode [data-action="retry-connection"] {
  pointer-events: auto;
  opacity: 1;
  cursor: pointer;
}
```

3. **G030 switchLlmProvider 暴露**（在 app.js 中）：

```javascript
// 确认 switchLlmProvider 已定义并暴露
function switchLlmProvider(provider) {
  // ... 切换逻辑 ...
}
window.switchLlmProvider = switchLlmProvider;

// 在 bindAllActions 中处理 data-input-action
document.querySelectorAll('[data-input-action]').forEach(el => {
  el.addEventListener('change', (e) => {
    const action = el.dataset.inputAction;
    const fn = window[action];
    if (typeof fn === 'function') {
      fn(el.value);
    } else {
      console.error(`[bindAllActions] data-input-action="${action}" 对应函数未定义`);
    }
  });
});
```

4. **G041 移动端 tabbar 同步**（修改 switchTab）：

```javascript
function switchTab(tabName) {
  // ... 原有 active 类处理 ...
  
  // 同步移动端 tabbar active 状态
  document.querySelectorAll('.mobile-tabbar .tab-item').forEach(item => {
    item.classList.remove('active');
  });
  const mobileTab = document.querySelector(`.mobile-tabbar .tab-item[data-tab="${tabName}"]`);
  if (mobileTab) mobileTab.classList.add('active');
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（setupEnterSubmit + switchLlmProvider + switchTab）
- [G:\code-memory\static\app.css](file:///g:/code-memory/static/app.css)（offline-mode 样式）
- [G:\code-memory\static\index.html](file:///g:/code-memory/static/index.html)（输入框添加 data-enter-action）

**验证方法**：

1. **G024 验证**：在输入框按 Enter，应触发对应按钮点击
2. **G027 验证**：断网后 body.offline-mode，API 按钮 pointer-events:none
3. **G030 验证**：`typeof window.switchLlmProvider === 'function'`
4. **G041 验证**：`switchTab('dashboard')` 后 `.mobile-tabbar .tab-item[data-tab="dashboard"]` 有 active 类

**PowerShell 验证脚本**：
```powershell
Write-Host "=== G024 Enter 提交 ===" -ForegroundColor Cyan
$enter = Select-String -Path "G:\code-memory\static\app.js" -Pattern "data-enter-action|setupEnterSubmit"
Write-Host ($enter.Count -gt 0 ? "PASS" : "FAIL")

Write-Host "=== G027 offline-mode 禁用 ===" -ForegroundColor Cyan
$offline = Select-String -Path "G:\code-memory\static\app.css" -Pattern "offline-mode.*btn|offline-mode.*pointer-events"
Write-Host ($offline.Count -gt 0 ? "PASS" : "FAIL")

Write-Host "=== G030 switchLlmProvider ===" -ForegroundColor Cyan
$llm = Select-String -Path "G:\code-memory\static\app.js" -Pattern "window\.switchLlmProvider"
Write-Host ($llm.Count -gt 0 ? "PASS" : "FAIL")

Write-Host "=== G041 移动端 tabbar ===" -ForegroundColor Cyan
$mobile = Select-String -Path "G:\code-memory\static\app.js" -Pattern "mobile-tabbar.*active|tab-item.*active"
Write-Host ($mobile.Count -gt 0 ? "PASS" : "FAIL")
```

**依赖 Step**：无

**预计工作量**：0.8 人天

**如果失败怎么办**：
- 若 Enter 提交不生效 → 检查 data-enter-action 属性是否正确设置
- 若 offline-mode 仍可点击 → 检查 CSS 选择器优先级
- 回滚方案：`git checkout HEAD~1 -- static/app.js static/app.css static/index.html`

---

### Step 16: 修复 G046/G031 模态框队列冲突

**目标缺口**：G046（showPrompt 与 showConfirm 队列冲突）+ G031（模态框堆叠时 ESC 优先级未管理）

**严重程度**：P2

**根因分析**：

- G046：[showPrompt:2929](file:///g:/code-memory/static/app.js#L2929) 直接操作 confirm-modal DOM，未入队，与 showConfirm 冲突
- G031：多个 modal 同时打开时 ESC 关闭顺序不确定

**修复方案**：

1. **G046 showPrompt 改为入队**（参考 Step 3 的 showInfoModal 模式）：

```javascript
async function showPrompt(message, title, defaultValue = '') {
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

在 processConfirmQueue 中处理 `task.isPrompt`：

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
  // ... Step 3 的逻辑 ...
} else {
  // ... showConfirm 逻辑 ...
}
```

2. **G031 实现 ModalStack**：

```javascript
const _modalStack = [];

function pushModal(modalId) {
  _modalStack.push(modalId);
}

function popModal() {
  return _modalStack.pop();
}

function getTopModal() {
  return _modalStack[_modalStack.length - 1];
}

// 修改全局 ESC 监听，仅关闭栈顶 modal
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && _modalStack.length > 0) {
    const topModalId = getTopModal();
    const topModal = document.getElementById(topModalId);
    if (topModal) {
      // 触发对应关闭逻辑
      const closeFn = window[`close${topModalId.replace(/-modal/, 'Modal').replace(/-./g, c => c[1].toUpperCase())}`];
      if (typeof closeFn === 'function') closeFn();
    }
  }
});
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（showPrompt 约 2929 行 + processConfirmQueue + 全局 ESC）

**验证方法**：

1. **G046 验证**：showConfirm 打开后调用 showPrompt，应入队等待
2. **G031 验证**：多个 modal 打开时，ESC 仅关闭栈顶

**PowerShell 验证脚本**：
```powershell
$promptEnqueue = Select-String -Path "G:\code-memory\static\app.js" -Pattern "isPrompt"
$modalStack = Select-String -Path "G:\code-memory\static\app.js" -Pattern "_modalStack|pushModal|popModal"
Write-Host ($promptEnqueue.Count -gt 0 ? "PASS: showPrompt 入队" : "FAIL")
Write-Host ($modalStack.Count -gt 0 ? "PASS: ModalStack 已实现" : "FAIL")
```

**依赖 Step**：Step 3（processConfirmQueue 已支持 isInfoOnly，需扩展 isPrompt）

**预计工作量**：0.8 人天

**如果失败怎么办**：
- 若 showPrompt 输入框不显示 → 检查 confirm-modal-input 元素是否存在
- 若 ModalStack ESC 顺序错乱 → 检查 pushModal/popModal 调用时机
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 17: 修复 G036/G044 并发请求 AbortController

**目标缺口**：G036（runPrivacyCheck 无 AbortController）+ G044（loadRecentMemories 等无 AbortController）

**严重程度**：P2

**根因分析**：

- G036：[runPrivacyCheck:3764](file:///g:/code-memory/static/app.js#L3764) Promise.all 无 signal，快速点击多次触发堆积
- G044：[loadRecentMemories:527](file:///g:/code-memory/static/app.js#L527) 等函数无 AbortController

**修复方案**：

```javascript
// G036 runPrivacyCheck
let privacyCheckAbortController = null;

async function runPrivacyCheck() {
  if (privacyCheckAbortController) {
    privacyCheckAbortController.abort();
  }
  privacyCheckAbortController = new AbortController();
  
  try {
    const [check1, check2, check3] = await Promise.allSettled([
      fetchWithTimeout(`${API_BASE}/v1/privacy/check1`, { signal: privacyCheckAbortController.signal }),
      fetchWithTimeout(`${API_BASE}/v1/privacy/check2`, { signal: privacyCheckAbortController.signal }),
      fetchWithTimeout(`${API_BASE}/v1/privacy/check3`, { signal: privacyCheckAbortController.signal })
    ]);
    // ... 渲染结果 ...
  } catch (e) {
    if (e.name === 'AbortError') return;
    // ... 错误处理 ...
  } finally {
    privacyCheckAbortController = null;
  }
}

// G044 在 _tabAbortControllers 中为每个标签页维护 controller
function _getTabAbortController(tabName) {
  if (!_tabAbortControllers.has(tabName)) {
    _tabAbortControllers.set(tabName, new AbortController());
  }
  return _tabAbortControllers.get(tabName);
}

async function loadRecentMemories() {
  const signal = _getTabAbortController('memories').signal;
  try {
    const res = await fetchWithTimeout(`${API_BASE}/v1/memories?limit=10`, { signal });
    // ... 渲染 ...
  } catch (e) {
    if (e.name === 'AbortError') return;
    // ... 错误处理 ...
  }
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（runPrivacyCheck 约 3764 行 + loadRecentMemories 约 527 行等）

**验证方法**：

1. 快速点击"立即检查隐私状态"10 次，应只触发 1 次完整请求
2. 切换标签页时，旧标签的请求被 abort

**PowerShell 验证脚本**：
```powershell
$privacy = Select-String -Path "G:\code-memory\static\app.js" -Pattern "privacyCheckAbortController"
$tabAbort = Select-String -Path "G:\code-memory\static\app.js" -Pattern "_getTabAbortController|_tabAbortControllers\.get"
Write-Host ($privacy.Count -gt 0 ? "PASS: G036 已修复" : "FAIL")
Write-Host ($tabAbort.Count -gt 1 ? "PASS: G044 已修复" : "FAIL")
```

**依赖 Step**：Step 7（_abortActiveTabRequests 已修改，支持 excludeTab）

**预计工作量**：0.8 人天

**如果失败怎么办**：
- 若 abort 后结果显示空白 → 检查 AbortError 分支清理逻辑
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 18: 修复 G039/G028 其他 P2 问题

**目标缺口**：G039（模型下载无进度细节）+ G028（beforeunload 无差异化提示）

**严重程度**：P2

**修复方案**：

1. **G039 模型下载进度细节**（约 5402 行）：

```javascript
async function downloadEmbedderModel(modelId) {
  // ... 现有逻辑 ...
  
  // 显示下载速度 + 剩余时间
  let lastTime = Date.now();
  let lastLoaded = 0;
  
  // 模拟或实际进度更新
  const progressEl = document.getElementById('download-progress');
  const speedEl = document.getElementById('download-speed');
  const etaEl = document.getElementById('download-eta');
  
  const updateInterval = setInterval(() => {
    const now = Date.now();
    const elapsed = (now - lastTime) / 1000;
    const loaded = parseInt(progressEl.dataset.loaded || 0);
    const total = parseInt(progressEl.dataset.total || 1);
    
    const speed = (loaded - lastLoaded) / elapsed; // bytes/sec
    const remaining = total - loaded;
    const eta = speed > 0 ? remaining / speed : 0;
    
    if (speedEl) speedEl.textContent = formatSpeed(speed);
    if (etaEl) etaEl.textContent = formatEta(eta);
    if (progressEl) progressEl.style.width = `${(loaded / total * 100).toFixed(1)}%`;
    
    lastTime = now;
    lastLoaded = loaded;
  }, 1000);
  
  // 下载完成后清理
  // clearInterval(updateInterval);
}

function formatSpeed(bytesPerSec) {
  if (bytesPerSec < 1024) return `${bytesPerSec.toFixed(0)} B/s`;
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`;
  return `${(bytesPerSec / 1024 / 1024).toFixed(1)} MB/s`;
}

function formatEta(seconds) {
  if (seconds < 60) return `${seconds.toFixed(0)}s`;
  if (seconds < 3600) return `${(seconds / 60).toFixed(0)}m ${Math.floor(seconds % 60)}s`;
  return `${(seconds / 3600).toFixed(0)}h ${Math.floor((seconds % 3600) / 60)}m`;
}
```

2. **G028 beforeunload 差异化提示**（约 6019 行）：

```javascript
// 修改 beforeunload 监听
window.addEventListener('beforeunload', (e) => {
  const count = window.__getPendingRequestCount();
  if (count > 0) {
    // 现代浏览器限制，但设置 returnValue 仍会显示通用提示
    e.preventDefault();
    e.returnValue = '';
    
    // 页面内显示提示横幅（在 beforeunload 触发前）
    showPendingRequestBanner(count);
  }
});

function showPendingRequestBanner(count) {
  let banner = document.getElementById('pending-request-banner');
  if (!banner) {
    banner = document.createElement('div');
    banner.id = 'pending-request-banner';
    banner.style.cssText = 'position:fixed;top:0;left:0;right:0;background:#ffc107;color:#000;padding:8px;text-align:center;z-index:10040;font-size:14px;';
    document.body.appendChild(banner);
  }
  banner.textContent = `⚠ 有 ${count} 个请求进行中，请勿关闭页面`;
}
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（downloadEmbedderModel 约 5402 行 + beforeunload 约 6019 行）

**验证方法**：

1. G039：下载模型时显示速度和剩余时间
2. G028：请求进行中关闭页面，显示横幅提示

**PowerShell 验证脚本**：
```powershell
$speed = Select-String -Path "G:\code-memory\static\app.js" -Pattern "formatSpeed|formatEta|download-speed"
$banner = Select-String -Path "G:\code-memory\static\app.js" -Pattern "pending-request-banner|showPendingRequestBanner"
Write-Host ($speed.Count -gt 0 ? "PASS: G039 已修复" : "FAIL")
Write-Host ($banner.Count -gt 0 ? "PASS: G028 已修复" : "FAIL")
```

**依赖 Step**：无

**预计工作量**：0.5 人天

**如果失败怎么办**：
- 若下载速度显示为 0 → 检查 progressEl.dataset.loaded 是否正确更新
- 回滚方案：`git checkout HEAD~1 -- static/app.js`

---

### Step 19: 修复 P3 低优先级问题（G043/G034/G035）

**目标缺口**：G043（status-version 硬编码 v0.8.2）+ G034（initSidebarNav 与 switchTab active 类同步冗余）+ G035（道同构度 score 计算公式硬编码，与后端不一致）

**严重程度**：P3

**根因分析**：

- G043：[app.js:756](file:///g:/code-memory/static/app.js#L756) `version.textContent = 'v0.8.2'` 硬编码，与实际版本 v0.8.4 不一致
- G034：[initSidebarNav:4855](file:///g:/code-memory/static/app.js#L4855) 和 [switchTab:4779](file:///g:/code-memory/static/app.js#L4779) 两次操作 active 类，可能闪烁
- G035：[app.js:3978](file:///g:/code-memory/static/app.js#L3978) 前端计算 score，[v1_api.rs:590](file:///g:/code-memory/src/v1_api.rs#L590) 后端计算 status，两者可能不一致

**修复方案**：

1. **G043 版本号常量化**（约 756 行）：

```javascript
// 修改前：
version.textContent = 'v0.8.2';

// 修改后：
// 在文件头部定义 APP_VERSION 常量（如不存在）
const APP_VERSION = '0.8.4';
// 在状态栏更新处
version.textContent = 'v' + APP_VERSION;
```

2. **G034 active 类同步去重**（约 4855 行）：

```javascript
// 修改 initSidebarNav，仅设置 sidebar active，由 switchTab 统一处理
function initSidebarNav() {
  document.querySelectorAll('.app-sidebar .nav-item').forEach(item => {
    item.addEventListener('click', (e) => {
      e.preventDefault();
      const tabName = item.dataset.tab;
      if (tabName) switchTab(tabName); // switchTab 内部统一处理 active 类
    });
  });
  // 不在此处设置 active 类，避免与 switchTab 冗余
}
```

3. **G035 后端返回 score 字段**（src/v1_api.rs）：

```rust
// 修改 v1_api.rs 的 dao_metrics 端点，返回 score 字段
// 修改前：
HttpResponse::Ok().json(serde_json::json!({
    "ok": true,
    "data": {
        "status": status,
        // ... 其他字段 ...
    }
}));

// 修改后：
HttpResponse::Ok().json(serde_json::json!({
    "ok": true,
    "data": {
        "status": status,
        "score": calculate_score(&metrics), // 新增 score 字段
        // ... 其他字段 ...
    }
}));
```

前端 loadDaoMetrics 优先使用后端 score：

```javascript
// 修改 loadDaoMetrics，优先使用后端 score
const score = data.score !== undefined ? data.score : calculateScoreLocal(metrics);
drawDaoRing(score);
```

**影响文件**：
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)（约 756 行 + 3978 行 + 4855 行）
- [G:\code-memory\src\v1_api.rs](file:///g:/code-memory/src/v1_api.rs)（dao_metrics 端点）

**验证方法**：

1. **G043 验证**：`document.querySelector('#status-version').textContent` 返回 'v0.8.4'
2. **G034 验证**：切换标签页时无视觉闪烁
3. **G035 验证**：后端返回 score 字段，前端直接使用

**PowerShell 验证脚本**：
```powershell
$version = Select-String -Path "G:\code-memory\static\app.js" -Pattern "APP_VERSION|version\.textContent.*APP_VERSION"
Write-Host ($version.Count -gt 0 ? "PASS: G043 已修复" : "FAIL")

$sidebar = Select-String -Path "G:\code-memory\static\app.js" -Pattern "initSidebarNav" -Context 0,10
Write-Host ($sidebar.LineMatches.Count -gt 0 ? "PASS: G034 已优化" : "FAIL")
```

**依赖 Step**：无

**预计工作量**：0.6 人天

**如果失败怎么办**：
- 若版本号显示异常 → 检查 APP_VERSION 常量定义位置
- 若后端 score 计算错误 → 保留前端计算作为降级
- 回滚方案：`git checkout HEAD~1 -- static/app.js src/v1_api.rs`

---

### Step 20: 版本号统一升级 + 文档同步

**目标缺口**：版本号统一升级为 0.8.4 + 文档同步更新

**严重程度**：收尾（必须）

**修复方案**：

1. **版本号统一升级**：

| 位置 | 当前版本 | 目标版本 | 文件 |
|:---|:---|:---|:---|
| tauri.conf.json | 0.8.3 | 0.8.4 | [desktop/src-tauri/tauri.conf.json](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json) |
| Cargo.toml | 0.8.3 | 0.8.4 | [Cargo.toml](file:///g:/code-memory/Cargo.toml) |
| HTML meta | 0.8.3 | 0.8.4 | [static/index.html](file:///g:/code-memory/static/index.html) |
| JS APP_VERSION | 0.8.3 | 0.8.4 | [static/app.js](file:///g:/code-memory/static/app.js) |
| 后端版本 | 0.8.3 | 0.8.4 | [src/v1_api.rs](file:///g:/code-memory/src/v1_api.rs) |

2. **CHANGELOG 更新**：

```markdown
## v0.8.4 (2026-07-29)

### P0 致命问题修复
- 修复 G018：showConfirm 暴露到 window（Step 2）
- 修复 G022：showInfoModal/showConfirm 队列死锁（Step 3）
- 修复 G020：loadDaoMetrics 降级更新 4 个小指标（Step 4）
- 修复 G045：selectPresetScenario 持久化（Step 5）

### CDP 测试失败项修复（9/9）
- CDP#1 道同构 API 结构（Step 4，环境问题修复）
- CDP#2 道同构指标显示（Step 4）
- CDP#3 预设场景切换（Step 5）
- CDP#4 N08 模态框 ESC（Step 12）
- CDP#5 G004 inFlight 防抖（Step 14）
- CDP#6 G011 Toast 去重（Step 8）
- CDP#7 G013 showConfirm 定义（Step 2）
- CDP#8 G013 模态框可聚焦元素（Step 3/12）
- CDP#9 G015 必填字段校验（Step 13）
- CDP#10 G016 startAutoRefresh 定义（Step 6）

### P1 严重问题修复
- 修复 G019：startAutoRefresh 暴露（Step 6）
- 修复 G021/G033：AbortController 竞态 + loading 卡死（Step 7）
- 修复 G026：Toast 去重边界 bug（Step 8）
- 修复 G025/G032/G040：XSS + 防抖绕过（Step 9）
- 修复 G029/G047：500 重试上限 + 竞态（Step 10）
- 修复 G037/G038：searchMemories AbortController（Step 11）
- 修复 G023：start-service-modal 焦点陷阱（Step 12）

### P2/P3 修复
- 修复 G024/G027/G030/G041（Step 15）
- 修复 G046/G031 模态框队列冲突（Step 16）
- 修复 G036/G044 并发请求 AbortController（Step 17）
- 修复 G039/G028 下载进度 + beforeunload 提示（Step 18）
- 修复 G043/G034/G035 版本号 + active 同步 + score（Step 19）
```

**影响文件**：
- [G:\code-memory\desktop\src-tauri\tauri.conf.json](file:///g:/code-memory/desktop/src-tauri/tauri.conf.json)
- [G:\code-memory\Cargo.toml](file:///g:/code-memory/Cargo.toml)
- [G:\code-memory\static\index.html](file:///g:/code-memory/static/index.html)
- [G:\code-memory\static\app.js](file:///g:/code-memory/static/app.js)
- [G:\code-memory\src\v1_api.rs](file:///g:/code-memory/src/v1_api.rs)
- [G:\code-memory\CHANGELOG.md](file:///g:/code-memory/CHANGELOG.md)

**验证方法**：

1. **版本号一致性验证**：
   ```powershell
   # 检查所有版本号是否为 0.8.4
   $tauri = (Get-Content "G:\code-memory\desktop\src-tauri\tauri.conf.json" | ConvertFrom-Json).version
   $cargo = (Select-String -Path "G:\code-memory\Cargo.toml" -Pattern "^version\s*=").Line -replace ".*=\s*""","" -replace """",""
   $html = (Select-String -Path "G:\code-memory\static\index.html" -Pattern 'name="version"').Line -replace ".*content=""","" -replace """.*",""
   $js = (Select-String -Path "G:\code-memory\static\app.js" -Pattern "APP_VERSION\s*=\s*'").Line -replace ".*'","" -replace "'",""
   
   Write-Host "tauri: $tauri"
   Write-Host "cargo: $cargo"
   Write-Host "html: $html"
   Write-Host "js: $js"
   
   if ($tauri -eq "0.8.4" -and $cargo -eq "0.8.4" -and $html -eq "0.8.4" -and $js -eq "0.8.4") {
       Write-Host "PASS: 版本号一致" -ForegroundColor Green
   } else {
       Write-Host "FAIL: 版本号不一致" -ForegroundColor Red
   }
   ```

2. **CDP 测试版本号验证**：`document.querySelector('meta[name="version"]').content` 返回 '0.8.4'

3. **后端编译验证**：`cargo build --release` 无错误

**依赖 Step**：所有 Step 1-19 完成

**预计工作量**：0.5 人天

**如果失败怎么办**：
- 若版本号遗漏 → 全局搜索 0.8.3 确保全部替换
- 若后端编译失败 → 检查 v1_api.rs 的 score 计算逻辑
- 回滚方案：`git checkout HEAD~1 -- desktop/src-tauri/tauri.conf.json Cargo.toml static/index.html static/app.js src/v1_api.rs CHANGELOG.md`

---