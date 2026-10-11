# LRC 桌面端 v0.8.6 修复优化计划

> **版本**：v0.8.6（基于 v0.8.5 HCSE 验证报告与 CDP 测试报告）
> **制定日期**：2026-07-30
> **目标版本**：v0.8.6
> **基线版本**：v0.8.5（CDP 通过率 81.1%，交互韧性 72.4 分，安全不变量合规率 70%）
> **输入文档**：
> - [CDP_TEST_REPORT_v0.8.5.md](./CDP_TEST_REPORT_v0.8.5.md)
> - [HCSE_VERIFICATION_REPORT_v0.8.5.md](./HCSE_VERIFICATION_REPORT_v0.8.5.md)
> - [INTERACTION_AUDIT_REPORT_v0.8.4.md](./INTERACTION_AUDIT_REPORT_v0.8.4.md)
> - [FIX_PLAN_v0.8.5.md](./FIX_PLAN_v0.8.5.md)（18 Step，仅 Step 2 完成）

---

## 一、概述

### 1.1 版本目标

v0.8.6 的核心目标是**修复 v0.8.5 遗留的 10 个问题**，使 LRC 桌面端达到生产可用标准：

| 目标维度 | v0.8.5 基线 | v0.8.6 目标 | 提升幅度 |
|:---|:---:|:---:|:---:|
| CDP 测试通过率 | 81.1% (30/37) | **≥97% (36/37)** | +15.9% |
| 交互韧性综合评分 | 72.4 | **≥88** | +15.6 |
| 安全不变量合规率 | 70% (7/10) | **100% (10/10)** | +30% |
| P0 问题数 | 3 | **0** | -3 |
| P1 问题数 | 5 | **0** | -5 |
| 用户反馈问题修复率 | 2/3 | **3/3** | +1 |

### 1.2 RICE 优先级矩阵

对 10 个问题进行 RICE 评分（Reach×Impact×Confidence÷Effort），确定修复顺序：

| Step | 问题 ID | 问题描述 | R | I | C | E | RICE | 优先级 |
|:---:|:---|:---|:---:|:---:|:---:|:---:|:---:|:---:|
| 1 | N002/G076 | CSP 完全未配置 | 10 | 5 | 10 | 2 | **250** | P0 |
| 2 | N003/G058 | 取消按钮无 AbortController | 8 | 4 | 10 | 2 | **160** | P0 |
| 3 | N001/G052 | handleHttpError 死代码 | 10 | 5 | 10 | 4 | **125** | P0 |
| 4 | N008 | 启动服务模态框 CSS 显示问题 | 8 | 4 | 10 | 1 | **320** | P1 |
| 5 | N004 | showToast 未暴露到 window | 7 | 3 | 10 | 1 | **210** | P1 |
| 6 | N005 | validateInput 未暴露到 window | 7 | 3 | 10 | 1 | **210** | P1 |
| 7 | N007 | AbortController 变量未暴露到 window | 7 | 3 | 10 | 1 | **210** | P1 |
| 8 | N006 | SidecarHealthMonitor intervalId 属性名不匹配 | 6 | 2 | 8 | 1 | **96** | P1 |
| 9 | N009 | 15 个输入框无 maxlength 限制 | 6 | 2 | 10 | 2 | **60** | P2 |
| 10 | N010 | 35 次 404 资源加载错误 | 10 | 2 | 8 | 3 | **53** | P2 |

**RICE 评分说明**：
- **Reach（影响范围，1-10）**：受影响的用户数或场景数
- **Impact（影响程度，1-5）**：对用户体验/安全的影响程度
- **Confidence（信心度，1-10）**：对修复方案正确性的信心
- **Effort（工作量，1-10）**：所需人天工作量（数值越小工作量越大）

### 1.3 修复顺序总览

按 RICE 评分降序排列，但遵循 P0→P1→P2 的优先级分组原则：

| 阶段 | Step | 问题 | RICE | 预计工作量 |
|:---:|:---:|:---|:---:|:---:|
| **P0 修复** | 1 | CSP 配置 | 250 | 0.5 人天 |
| | 2 | AbortController 取消按钮 | 160 | 1 人天 |
| | 3 | handleHttpError 死代码 | 125 | 2 人天 |
| **P1 修复** | 4 | 模态框 CSS | 320 | 0.3 人天 |
| | 5 | showToast 暴露 | 210 | 0.2 人天 |
| | 6 | validateInput 暴露 | 210 | 0.2 人天 |
| | 7 | AbortController 暴露 | 210 | 0.2 人天 |
| | 8 | intervalId 属性名 | 96 | 0.2 人天 |
| **P2 修复** | 9 | maxlength 限制 | 60 | 0.5 人天 |
| | 10 | 404 资源修复 | 53 | 0.8 人天 |
| **收尾** | 11 | 版本号升级 | — | 0.2 人天 |
| | 12 | 文档同步 | — | 0.3 人天 |
| | 13 | 验收测试 | — | 0.5 人天 |
| **合计** | | | | **7.9 人天** |

---

## 二、P0 修复步骤（Step 1-3）

### Step 1：配置 Content-Security-Policy（N002/G076）

#### 问题描述

index.html 的 `<head>` 中**完全没有 Content-Security-Policy meta 标签**，导致 XSS 攻击和数据注入防御层完全缺失。HCSE 验证报告显示 `hasCspMeta=false`，违反安全不变量 INV-01。

#### 用户故事

> **As a** LRC 桌面端用户
> **I want to** 应用在所有页面加载时启用内容安全策略（CSP）
> **So that** 恶意脚本无法通过 XSS 注入执行，保护我的代码记忆库数据安全

#### 根因分析

1. v0.8.2 修复时移除了 6 处内联事件（onclick/onchange/oninput），改为 `data-action` 绑定
2. 但未添加 CSP meta 标签作为纵深防御层
3. Tauri 配置中的 CSP 可能未生效（需确认 `tauri.conf.json` 的 `csp` 字段）
4. FMEA 分析 FM-05 显示 RPN=160，属于高风险

#### 修复方案

在 `index.html` 的 `<head>` 第一行添加 CSP meta 标签：

```html
<meta http-equiv="Content-Security-Policy" content="
  default-src 'self';
  script-src 'self' 'unsafe-inline';
  style-src 'self' 'unsafe-inline' https://fonts.googleapis.com;
  font-src 'self' https://fonts.gstatic.com;
  img-src 'self' data: blob:;
  connect-src 'self' http://127.0.0.1:* http://localhost:* https://api.openai.com;
  frame-src 'self';
  object-src 'none';
  base-uri 'self';
  form-action 'self';
">
```

**CSP 策略说明**：
- `default-src 'self'`：默认只允许同源资源
- `script-src 'self' 'unsafe-inline'`：允许内联脚本（app.js 使用 IIFE 模式，需逐步迁移）
- `style-src`：允许内联样式和 Google Fonts
- `connect-src`：允许本地 sidecar（3099-3198 端口范围）和 OpenAI API
- `object-src 'none'`：禁止 Flash/Java 插件
- `base-uri 'self'`：防止 base 标签注入

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\static\index.html` | 新增 | 在 `<head>` 第一行添加 CSP meta |
| `G:\code-memory\desktop\src-tauri\tauri.conf.json` | 检查 | 确认 `app.security.csp` 字段是否与 meta 一致 |

#### 验证脚本

`G:\code-memory\scripts\verify_step1_csp_v086.ps1`

#### 验收标准

```gherkin
Given LRC 桌面端已启动并加载仪表盘页面
When 检查 index.html 的 <head> 标签
Then 存在 <meta http-equiv="Content-Security-Policy"> 标签
And CSP 策略包含 default-src 'self'
And CSP 策略包含 connect-src http://127.0.0.1:*
And 控制台无 CSP 违规错误
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 功能风险 | 低 | CSP 可能阻止某些资源加载 | 移除 meta 标签，恢复无 CSP 状态 |
| 兼容性风险 | 中 | `unsafe-inline` 可能被未来浏览器收紧 | 逐步迁移到 nonce-based CSP |
| 回滚难度 | 低 | 删除一行 HTML 即可 | Git revert 单文件 |

#### 回滚方案

1. 删除 index.html 中新增的 CSP meta 标签
2. 执行 `git checkout static/index.html`
3. 重新编译 sidecar：`cargo build --bin code-memory-server`

---

### Step 2：为取消按钮添加 AbortController（N003/G058）

#### 问题描述

启动服务弹窗的"取消"按钮**无法中断进行中的请求**。用户点击取消后，模态框关闭但后台的 Tauri invoke 调用继续执行（最长 60 秒），导致"取消按钮无反应"的用户反馈。HCSE 验证报告显示违反安全不变量 INV-05。

#### 用户故事

> **As a** LRC 桌面端用户
> **I want to** 点击"取消"按钮时立即中断启动服务的后台请求
> **So that** 我可以快速取消误触发的启动操作，无需等待 60 秒超时

#### 根因分析

1. `postMessageToParent` 函数（app.js L894-943）仅有 `setTimeout` 超时机制
2. Tauri 环境下直接调用 `await invokeFn(cmdName, extra)`，无 AbortController
3. 取消按钮的 `data-action="closeStartServiceModal"` 仅调用 `hideStartServiceModal()`
4. 60 秒内的 Tauri invoke 调用无法中断
5. FMEA 分析 FM-07 显示 RPN=160，属于高风险

#### 修复方案

修改 `postMessageToParent` 函数，添加 AbortController 支持：

```javascript
// 新增模块级变量
let startServiceAbortController = null;

function postMessageToParent(type, extra = {}, timeoutMs = 30000, externalSignal) {
  return new Promise(async (resolve, reject) => {
    // 创建内部 AbortController
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);

    // 支持外部信号联动（如取消按钮）
    if (externalSignal) {
      if (externalSignal.aborted) {
        clearTimeout(timer);
        reject(new DOMException('Aborted', 'AbortError'));
        return;
      }
      externalSignal.addEventListener('abort', () => controller.abort(), { once: true });
    }

    try {
      if (isTauriEnv) {
        // Tauri 环境：监听 abort 信号，虽无法真正中断 invoke，但可拒绝 Promise
        const invokePromise = invokeFn(cmdName, extra);
        const abortPromise = new Promise((_, reject) => {
          controller.signal.addEventListener('abort', () => {
            reject(new DOMException('Aborted', 'AbortError'));
          }, { once: true });
        });
        const result = await Promise.race([invokePromise, abortPromise]);
        clearTimeout(timer);
        resolve(result);
      } else {
        // iframe 模式：使用 fetch + signal
        const res = await fetch(url, { signal: controller.signal, ... });
        clearTimeout(timer);
        resolve(res);
      }
    } catch (err) {
      clearTimeout(timer);
      if (err.name === 'AbortError') {
        reject(new DOMException('用户取消操作', 'AbortError'));
      } else {
        reject(err);
      }
    }
  });
}
```

修改取消按钮处理逻辑：

```javascript
// closeStartServiceModal 增强版
function closeStartServiceModal() {
  if (startServiceAbortController) {
    startServiceAbortController.abort();
    startServiceAbortController = null;
  }
  hideStartServiceModal();
}

// 启动服务调用处
async function handleStartServiceClick() {
  startServiceAbortController = new AbortController();
  try {
    const result = await postMessageToParent(
      'lrc-start-service', {}, 60000, startServiceAbortController.signal
    );
    // 处理成功
  } catch (err) {
    if (err.name === 'AbortError') {
      showToast('已取消启动服务', 'info');
    } else {
      showToast(`启动失败：${err.message}`, 'error');
    }
  } finally {
    startServiceAbortController = null;
  }
}
```

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\static\app.js` | 修改 | postMessageToParent 添加 AbortController 参数 |
| `G:\code-memory\static\app.js` | 修改 | 新增 startServiceAbortController 变量 |
| `G:\code-memory\static\app.js` | 修改 | closeStartServiceModal 调用 abort() |
| `G:\code-memory\static\app.js` | 修改 | handleStartServiceClick 传入 signal |

#### 验证脚本

`G:\code-memory\scripts\verify_step2_abort_controller_v086.ps1`

#### 验收标准

```gherkin
Given 用户已打开"启动服务"弹窗并点击"启动服务"按钮
When 启动过程中用户点击"取消"按钮
Then 后台 invoke 请求被中断（Promise 被 reject）
And 弹窗立即关闭
And 显示 Toast 提示"已取消启动服务"
And startServiceAbortController 被置为 null
And 控制台无未捕获的 Promise 错误
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 功能风险 | 中 | Tauri invoke 实际无法真正中断，仅前端拒绝 | 可接受，用户感知已取消 |
| 并发风险 | 低 | abort 后 controller 被置 null | finally 块确保清理 |
| 回滚难度 | 低 | 恢复 postMessageToParent 原签名 | Git revert app.js |

#### 回滚方案

1. 移除 postMessageToParent 的 externalSignal 参数
2. 移除 startServiceAbortController 变量
3. 恢复 closeStartServiceModal 原实现
4. 执行 `git checkout static/app.js`

---

### Step 3：修复 handleHttpError 死代码（N001/G052）

#### 问题描述

`handleHttpError` 函数已定义（app.js L177）但**40+ catch 块无一调用**，导致整个错误恢复层完全失效。HCSE 验证报告显示违反安全不变量 INV-06，FMEA 分析 FM-08 显示 RPN=180（最高风险）。

影响范围：
- 500 错误仅显示 Toast，无重试 Modal
- 429 限流无 Toast 提示
- `_retryCounters` Map 永不填充
- `_retryModalActive` 永远 false
- `switchTab` 的 `_retryModalActive` 检查永远通过

#### 用户故事

> **As a** LRC 桌面端用户
> **I want to** 当 sidecar 返回 500/429 错误时，应用显示友好的重试提示而非通用错误
> **So that** 我可以选择重试或取消，而非面对无法理解的错误信息

#### 根因分析

1. v0.8.3/v0.8.4 修复了 G007（500 重试 Modal）、G009（503 降级）、G029（重试次数上限）、G047（重试时禁止切换标签）
2. 这些修复全部基于 `handleHttpError` 函数
3. 但实际所有 `fetchWithTimeout` 调用方都使用 try/catch，**无一调用 handleHttpError**
4. handleHttpError 成为死代码，错误恢复层完全失效
5. 重点修复位置：
   - `loadDashboard` 中的 /v1/dao-metrics 请求
   - `loadMemories` 中的 /v1/memories 请求
   - `searchMemories` 中的 /v1/memories/enrich 请求
   - `loadTrustCenterData` 中的 /v1/trust/* 请求

#### 修复方案

**分阶段修复**（禁止批量修改，逐个 catch 块替换）：

**阶段 3.1**：在 `fetchWithTimeout` 内部集成 handleHttpError

```javascript
async function fetchWithTimeout(url, options = {}, timeoutMs = 60000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, { ...options, signal: controller.signal });
    // 集成 handleHttpError 检查
    if (!response.ok) {
      const handled = await handleHttpError(response, { url, context: 'fetchWithTimeout' });
      if (handled.action === 'retry') {
        clearTimeout(timer);
        return fetchWithTimeout(url, options, timeoutMs); // 递归重试
      }
      throw new Error(`HTTP ${response.status}: ${handled.message}`);
    }
    return response;
  } catch (err) {
    if (err.name === 'AbortError') {
      throw new SidecarTimeoutError(`请求超时: ${url}`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}
```

**阶段 3.2**：逐个替换关键 catch 块（4 个核心位置）

```javascript
// loadDashboard 示例
async function loadDashboard() {
  try {
    const data = await fetchWithTimeout('/v1/health/dao_metrics');
    // ...
  } catch (err) {
    // 旧代码：showToast(err.message, 'error');
    // 新代码：交由 handleHttpError 处理
    if (err.name !== 'AbortError') {
      const result = await handleHttpError(
        { status: err.statusCode || 500 },
        { url: '/v1/health/dao_metrics', context: 'loadDashboard' }
      );
      if (result.action === 'retry') {
        return loadDashboard();
      }
    }
  }
}
```

**阶段 3.3**：验证 `_retryCounters` 和 `_retryModalActive` 正确工作

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\static\app.js` | 修改 | fetchWithTimeout 集成 handleHttpError |
| `G:\code-memory\static\app.js` | 修改 | loadDashboard catch 块 |
| `G:\code-memory\static\app.js` | 修改 | loadMemories catch 块 |
| `G:\code-memory\static\app.js` | 修改 | searchMemories catch 块 |
| `G:\code-memory\static\app.js` | 修改 | loadTrustCenterData catch 块 |

#### 验证脚本

`G:\code-memory\scripts\verify_step3_handle_http_error_v086.ps1`

#### 验收标准

```gherkin
Given sidecar 返回 500 内部错误
When fetchWithTimeout 调用 /v1/health/dao_metrics
Then handleHttpError 被调用
And _retryCounters Map 填充对应 URL 的计数
And 重试次数 < 3 时显示重试 Modal
And 重试次数 >= 3 时显示 Toast "已达最大重试次数"
And 控制台无 "handleHttpError is not defined" 错误

Given sidecar 返回 429 限流
When fetchWithTimeout 调用任意 API
Then handleHttpError 被调用
And 显示 Toast "请求过于频繁，请稍后重试"
And _retryCounters 对应 URL 计数 +1
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 功能风险 | 高 | 修改核心 fetch 函数，可能引入回归 | 分阶段修复，每阶段独立测试 |
| 递归风险 | 中 | 重试可能导致无限递归 | handleHttpError 内部限制最大重试 3 次 |
| 性能风险 | 低 | 重试增加请求次数 | 指数退避（2s/4s/8s） |
| 回滚难度 | 中 | 涉及 5+ 处修改 | Git revert app.js，但需逐 Step 回滚 |

#### 回滚方案

1. 恢复 fetchWithTimeout 原实现（移除 handleHttpError 集成）
2. 恢复 4 个 catch 块为 `showToast(err.message, 'error')`
3. 执行 `git checkout static/app.js`
4. 重新编译 sidecar 并测试

---

## 三、P1 修复步骤（Step 4-8）

### Step 4：修复启动服务模态框 CSS 显示问题（N008）

#### 问题描述

启动服务模态框 `.modal-overlay` 使用 `display:flex`，覆盖了 HTML `[hidden]` 属性的 `display:none`，导致模态框无法正确隐藏。HCSE 验证报告显示控制台报错"[openStartServiceModal] 模态框仍不可见，检查 CSS"。

#### 根因分析

1. `.modal-overlay` CSS 设置为 `display:flex` 用于居中布局
2. HTML `[hidden]` 属性的默认 `display:none` 被 `display:flex` 覆盖
3. 模态框设置 `hidden` 属性后仍然可见
4. 这违反了项目记忆中记录的已知问题："CSS .modal-overlay 使用 display:flex 会覆盖 HTML [hidden] 属性的 display:none"

#### 修复方案

在 CSS 中添加显式隐藏规则：

```css
/* 修复 .modal-overlay 覆盖 [hidden] 属性的问题 */
.modal-overlay[hidden] {
  display: none !important;
}

/* 确保所有使用 hidden 属性的元素都能正确隐藏 */
[hidden] {
  display: none !important;
}
```

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\static\index.html` | 修改 | 在 `<style>` 中添加 .modal-overlay[hidden] 规则 |

#### 验证脚本

`G:\code-memory\scripts\verify_step4_modal_css_v086.ps1`

#### 验收标准

```gherkin
Given 启动服务模态框具有 hidden 属性
When 检查 .modal-overlay 的计算样式
Then display 为 none
And 模态框不可见
And ESC 键可以关闭模态框
And 控制台无 "[openStartServiceModal] 模态框仍不可见" 错误
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 功能风险 | 低 | 仅添加 CSS 规则，不影响逻辑 | 删除新增 CSS 规则 |
| 兼容性风险 | 低 | `!important` 优先级高，符合预期 | 移除 `!important` |
| 回滚难度 | 低 | 删除几行 CSS | Git checkout index.html |

#### 回滚方案

1. 移除 `.modal-overlay[hidden]` 和 `[hidden]` CSS 规则
2. 执行 `git checkout static/index.html`

---

### Step 5：暴露 showToast 到 window（N004）

#### 问题描述

`showToast` 函数在 IIFE 作用域内，未通过 `window.showToast = showToast` 暴露，导致 CDP 测试无法执行 Toast 队列管理测试（G011）。

#### 根因分析

1. app.js 使用 IIFE 模式封装所有函数
2. `showToast` 未暴露到 window 对象
3. CDP 测试通过 `window.showToast` 访问失败
4. Toast 队列上限测试（G011）和优先级测试无法执行

#### 修复方案

在 IIFE 末尾添加 window 暴露：

```javascript
// 在 IIFE 末尾添加
window.showToast = showToast;
window._toastQueue = _toastQueue; // 用于测试检查队列状态
window._toastMaxVisible = _toastMaxVisible; // 暴露配置便于测试
```

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\static\app.js` | 修改 | IIFE 末尾添加 window.showToast 暴露 |

#### 验证脚本

`G:\code-memory\scripts\verify_step5_showToast_window_v086.ps1`

#### 验收标准

```gherkin
Given app.js 已加载完成
When 在浏览器控制台执行 window.showToast
Then 返回值类型为 function
And 调用 window.showToast('测试', 'info') 显示 Toast
And window._toastQueue 返回当前队列数组
And 队列上限为 3 个
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 安全风险 | 低 | 暴露到 window 不影响 CSP（已配置 unsafe-inline） | 移除暴露代码 |
| 功能风险 | 低 | 仅添加暴露，不修改逻辑 | Git checkout app.js |
| 回滚难度 | 低 | 删除一行代码 | 移除 window.showToast 赋值 |

#### 回滚方案

1. 移除 `window.showToast = showToast` 及相关暴露
2. 执行 `git checkout static/app.js`

---

### Step 6：暴露 validateInput 到 window（N005）

#### 问题描述

`validateInput` 函数在 IIFE 作用域内，未暴露到 window，导致 CDP 测试无法执行输入校验测试（G015）。

#### 根因分析

1. `validateInput` 函数定义在 IIFE 内
2. `setupInputValidation` 已定义但 `validateInput` 不可访问
3. CDP 测试通过 `window.validateInput` 访问失败
4. 输入校验功能可能正常工作（通过 MutationObserver），但无法测试

#### 修复方案

```javascript
// 在 IIFE 末尾添加
window.validateInput = validateInput;
window.setupInputValidation = setupInputValidation;
```

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\static\app.js` | 修改 | IIFE 末尾添加 window.validateInput 暴露 |

#### 验证脚本

`G:\code-memory\scripts\verify_step6_validateInput_window_v086.ps1`

#### 验收标准

```gherkin
Given app.js 已加载完成
When 在浏览器控制台执行 window.validateInput
Then 返回值类型为 function
And 调用 window.validateInput(invalidInput) 返回 false
And 调用 window.validateInput(validInput) 返回 true
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 功能风险 | 低 | 仅添加暴露，不修改逻辑 | Git checkout app.js |
| 回滚难度 | 低 | 删除一行代码 | 移除 window.validateInput 赋值 |

#### 回滚方案

1. 移除 `window.validateInput = validateInput`
2. 执行 `git checkout static/app.js`

---

### Step 7：暴露 AbortController 变量到 window（N007）

#### 问题描述

`dashboardAbortController`、`searchAbortController`、`_tabAbortControllers` 等变量未暴露到 window，导致 CDP 并发控制检测失败（G017）。

#### 根因分析

1. 这些变量定义在 IIFE 作用域内
2. CDP 测试通过 `window.dashboardAbortController` 等访问失败
3. 无法验证标签切换时是否正确 abort 请求

#### 修复方案

```javascript
// 在 IIFE 末尾添加（仅用于测试，生产环境可移除）
window.__testHooks = {
  get dashboardAbortController() { return dashboardAbortController; },
  get searchAbortController() { return searchAbortController; },
  get tabAbortControllers() { return _tabAbortControllers; },
  get retryCounters() { return _retryCounters; },
  get retryModalActive() { return _retryModalActive; },
  _abortActiveTabRequests: _abortActiveTabRequests
};
```

**说明**：使用 `__testHooks` 命名空间而非直接暴露，避免污染 window 对象，且可通过 `Object.defineProperty` 设置为不可枚举。

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\static\app.js` | 修改 | IIFE 末尾添加 __testHooks 命名空间 |

#### 验证脚本

`G:\code-memory\scripts\verify_step7_abort_controller_window_v086.ps1`

#### 验收标准

```gherkin
Given app.js 已加载完成
When 在浏览器控制台执行 window.__testHooks
Then 返回对象包含 dashboardAbortController 属性
And 返回对象包含 searchAbortController 属性
And 返回对象包含 tabAbortControllers 属性
And _abortActiveTabRequests 类型为 function
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 安全风险 | 低 | __testHooks 不含敏感数据 | 移除命名空间 |
| 功能风险 | 低 | 仅添加暴露，不修改逻辑 | Git checkout app.js |
| 回滚难度 | 低 | 删除测试钩子 | 移除 __testHooks 赋值 |

#### 回滚方案

1. 移除 `window.__testHooks` 赋值
2. 执行 `git checkout static/app.js`

---

### Step 8：修复 SidecarHealthMonitor intervalId 属性名（N006）

#### 问题描述

`SidecarHealthMonitor.intervalId` 属性名不匹配，CDP 测试检查 `window.SidecarHealthMonitor.intervalId` 返回 null，但实际已启动（控制台日志确认）。

#### 根因分析

1. `SidecarHealthMonitor` 实际已启动（控制台日志"Sidecar 健康监测器已启动，轮询间隔: 10000ms"）
2. 但 `intervalId` 属性名可能是 `_intervalId` 或未暴露到 window
3. CDP 测试通过 `window.SidecarHealthMonitor.intervalId` 访问失败
4. 属于测试方法问题，但需统一属性名以便测试

#### 修复方案

```javascript
// SidecarHealthMonitor 定义处
const SidecarHealthMonitor = {
  intervalId: null,  // 确保使用 intervalId 而非 _intervalId
  isRunning: false,
  
  start(intervalMs = 10000) {
    if (this.intervalId) {
      clearInterval(this.intervalId);
    }
    this.intervalId = setInterval(() => this.check(), intervalMs);
    this.isRunning = true;
    console.log(`Sidecar 健康监测器已启动，轮询间隔: ${intervalMs}ms`);
  },
  
  stop() {
    if (this.intervalId) {
      clearInterval(this.intervalId);
      this.intervalId = null;
    }
    this.isRunning = false;
  }
};

// 暴露到 window
window.SidecarHealthMonitor = SidecarHealthMonitor;
```

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\static\app.js` | 修改 | SidecarHealthMonitor 确保使用 intervalId 属性名 |

#### 验证脚本

`G:\code-memory\scripts\verify_step8_interval_id_v086.ps1`

#### 验收标准

```gherkin
Given app.js 已加载完成且 SidecarHealthMonitor 已启动
When 在浏览器控制台执行 window.SidecarHealthMonitor.intervalId
Then 返回非 null 的数字（setInterval ID）
And window.SidecarHealthMonitor.isRunning 为 true
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 功能风险 | 低 | 仅统一属性名，不修改逻辑 | Git checkout app.js |
| 回滚难度 | 低 | 恢复原属性名 | 恢复 _intervalId |

#### 回滚方案

1. 恢复 `intervalId` 为 `_intervalId`
2. 执行 `git checkout static/app.js`

---

## 四、P2 修复步骤（Step 9-10）

### Step 9：为输入框添加 maxlength 限制（N009）

#### 问题描述

15 个输入框无 `maxlength` 属性限制，用户可能输入超长文本导致 sidecar 处理异常或内存占用过高。

#### 根因分析

1. HTML 输入框未设置 `maxlength` 属性
2. `validateInput` 函数未强制长度校验
3. 涉及输入框：
   - LLM API 配置输入框（app.js L1200）
   - 记忆搜索输入框（app.js L4430）
   - 向导路径输入框（app.js L1450）
   - 其他 12 个输入框

#### 修复方案

**阶段 9.1**：为 HTML 静态输入框添加 maxlength

```html
<!-- 记忆搜索输入框 -->
<input type="text" id="memory-search-input" maxlength="200" ...>

<!-- LLM API 配置输入框 -->
<input type="text" id="llm-api-key" maxlength="500" ...>
<textarea id="llm-api-config" maxlength="2000" ...></textarea>

<!-- 向导路径输入框 -->
<input type="text" id="wizard-project-dir" maxlength="500" ...>
```

**阶段 9.2**：在 validateInput 中添加长度校验

```javascript
function validateInput(input) {
  const value = input.value.trim();
  const maxLength = input.getAttribute('maxlength');
  
  if (maxLength && value.length > parseInt(maxLength)) {
    input.classList.add('input-error');
    showToast(`输入超过最大长度 ${maxLength} 字符`, 'error');
    return false;
  }
  // ... 其他校验
  return true;
}
```

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\static\index.html` | 修改 | 15 个输入框添加 maxlength 属性 |
| `G:\code-memory\static\app.js` | 修改 | validateInput 添加长度校验逻辑 |

#### 验证脚本

`G:\code-memory\scripts\verify_step9_maxlength_v086.ps1`

#### 验收标准

```gherkin
Given LRC 桌面端已加载
When 检查所有 input 和 textarea 元素
Then 每个元素都有 maxlength 属性
And maxlength 值合理（搜索 200，配置 500-2000，路径 500）
And 输入超过 maxlength 时显示错误提示
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 功能风险 | 低 | maxlength 是浏览器原生属性 | 移除属性 |
| 用户体验风险 | 中 | 可能限制合法长输入 | 设置合理的 maxlength 值 |
| 回滚难度 | 低 | 移除 maxlength 属性 | Git checkout |

#### 回滚方案

1. 移除所有新增的 maxlength 属性
2. 恢复 validateInput 原实现
3. 执行 `git checkout static/index.html static/app.js`

---

### Step 10：修复 35 次 404 资源加载错误（N010）

#### 问题描述

CDP 测试发现 35 次 404 资源加载错误，虽不影响核心功能，但增加网络开销和控制台噪音。

#### 根因分析

1. HTML 中引用了不存在的资源（图片、字体、脚本）
2. 可能是 v0.8.x 修复过程中遗留的无效引用
3. 需通过 CDP 网络日志识别具体 404 URL

#### 修复方案

**阶段 10.1**：通过 CDP 收集 404 URL 列表

```powershell
# 使用 CDP 收集 404 错误
$cdpUrl = "http://127.0.0.1:9222"
# 执行 CDP Network.responseReceived 事件监听
# 过滤 status === 404 的响应
```

**阶段 10.2**：逐个修复 404 引用

```html
<!-- 修复前 -->
<link rel="stylesheet" href="/static/css/nonexistent.css">
<img src="/static/images/missing-icon.png">

<!-- 修复后 -->
<link rel="stylesheet" href="/static/css/correct.css">
<img src="/static/images/icon.svg">
<!-- 或移除无效引用 -->
```

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\static\index.html` | 修改 | 修复或移除 404 资源引用 |
| `G:\code-memory\static\app.js` | 修改 | 修复动态加载的 404 资源 |

#### 验证脚本

`G:\code-memory\scripts\verify_step10_404_resources_v086.ps1`

#### 验收标准

```gherkin
Given LRC 桌面端已加载完成
When 监听 CDP Network 事件 5 秒
Then 404 响应数量为 0
And 控制台无 "Failed to load resource: 404" 警告
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 功能风险 | 低 | 移除无效引用不影响功能 | 恢复原引用 |
| 回滚难度 | 低 | 逐个恢复 | Git checkout |

#### 回滚方案

1. 恢复 index.html 和 app.js 原资源引用
2. 执行 `git checkout static/index.html static/app.js`

---

## 五、版本号升级（Step 11）

### Step 11：升级版本号 v0.8.5 → v0.8.6

#### 问题描述

需将所有版本标识从 v0.8.5 升级到 v0.8.6，确保桌面端、sidecar、前端版本一致。

#### 修复方案

**逐文件修改**（禁止批量替换）：

1. **根 Cargo.toml**（sidecar 版本）
```toml
[package]
version = "0.8.6"
```

2. **desktop/src-tauri/Cargo.toml**（桌面端版本）
```toml
[package]
version = "0.8.6"
```

3. **desktop/src-tauri/tauri.conf.json**（应用版本）
```json
{
  "version": "0.8.6"
}
```

4. **static/index.html**（前端显示版本）
```html
<span id="app-version">v0.8.6</span>
```

5. **static/app.js**（JS 版本常量）
```javascript
const APP_VERSION = '0.8.6';
```

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\Cargo.toml` | 修改 | version = "0.8.6" |
| `G:\code-memory\desktop\src-tauri\Cargo.toml` | 修改 | version = "0.8.6" |
| `G:\code-memory\desktop\src-tauri\tauri.conf.json` | 修改 | "version": "0.8.6" |
| `G:\code-memory\static\index.html` | 修改 | v0.8.6 |
| `G:\code-memory\static\app.js` | 修改 | APP_VERSION = '0.8.6' |

#### 验证脚本

`G:\code-memory\scripts\verify_step11_version_v086.ps1`

#### 验收标准

```gherkin
Given 所有文件已修改完成
When 执行 cargo build --release
Then 编译成功无错误
And sidecar /v1/version API 返回 0.8.6
And 桌面端关于页面显示 v0.8.6
And 前端 footer 显示 v0.8.6
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 功能风险 | 低 | 仅版本号变更 | 恢复 0.8.5 |
| 回滚难度 | 低 | 逐文件恢复 | Git checkout |

#### 回滚方案

1. 恢复所有文件版本号为 0.8.5
2. 执行 `git checkout Cargo.toml desktop/src-tauri/Cargo.toml desktop/src-tauri/tauri.conf.json static/index.html static/app.js`

---

## 六、文档同步（Step 12）

### Step 12：同步项目文档

#### 问题描述

需更新项目文档以反映 v0.8.6 的修复内容和测试结果。

#### 修复方案

**阶段 12.1**：更新 CHANGELOG.md（如不存在则创建）

```markdown
# Changelog

## v0.8.6 (2026-07-30)

### 修复
- **P0**: 配置 Content-Security-Policy（N002/G076）
- **P0**: 启动服务取消按钮添加 AbortController（N003/G058）
- **P0**: 修复 handleHttpError 死代码（N001/G052）
- **P1**: 修复启动服务模态框 CSS 显示问题（N008）
- **P1**: 暴露 showToast 到 window（N004）
- **P1**: 暴露 validateInput 到 window（N005）
- **P1**: 暴露 AbortController 变量到 window（N007）
- **P1**: 修复 SidecarHealthMonitor intervalId 属性名（N006）
- **P2**: 为 15 个输入框添加 maxlength 限制（N009）
- **P2**: 修复 35 次 404 资源加载错误（N010）

### 改进
- CDP 测试通过率：81.1% → ≥97%
- 交互韧性评分：72.4 → ≥88
- 安全不变量合规率：70% → 100%
```

**阶段 12.2**：更新项目记忆库

通过 LRC 记忆系统记录 v0.8.6 修复完成状态：
- 更新架构总览记忆
- 记录新增的 CSP 配置
- 记录 handleHttpError 调用链路

**阶段 12.3**：更新 README.md（如需）

在 README 中更新版本号和测试结果链接。

#### 影响文件

| 文件路径 | 修改类型 | 说明 |
|:---|:---|:---|
| `G:\code-memory\docs\CHANGELOG.md` | 新增/修改 | 添加 v0.8.6 变更日志 |
| `G:\code-memory\README.md` | 修改 | 更新版本号（如需） |
| LRC 记忆库 | 更新 | 通过 remember 工具同步 |

#### 验证脚本

`G:\code-memory\scripts\verify_step12_docs_sync_v086.ps1`

#### 验收标准

```gherkin
Given v0.8.6 所有修复已完成
When 检查 docs/CHANGELOG.md
Then 存在 v0.8.6 条目
And 条目包含所有 10 个修复项
And 日期为 2026-07-30
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 文档风险 | 低 | 仅文档更新 | Git checkout |
| 回滚难度 | 低 | 恢复文档 | Git revert |

#### 回滚方案

1. 恢复 CHANGELOG.md 原内容
2. 执行 `git checkout docs/CHANGELOG.md`

---

## 七、验收测试（Step 13）

### Step 13：执行 v0.8.6 完整验收测试

#### 问题描述

需执行完整的 HCSE 验证和 CDP 测试，确认 v0.8.6 达到生产可用标准。

#### 修复方案

**阶段 13.1**：编译验证

```powershell
# 编译 sidecar
cargo build --release --bin code-memory-server

# 编译桌面端
cd desktop/src-tauri
cargo build --release
```

**阶段 13.2**：CDP 37 个测试用例回归

```powershell
# 启动 sidecar
.\scripts\check_sidecar_health_v085.ps1

# 执行 CDP 测试
python scripts/test_desktop_cdp_v083.js
```

**阶段 13.3**：HCSE 安全不变量验证

逐项验证 10 个安全不变量：
- INV-01: CSP 内容安全策略 ✅（Step 1 修复）
- INV-02: XSS 防御-无内联事件 ✅（v0.8.2 已合规）
- INV-03: sidecar 健康检查 ✅（Step 8 修复）
- INV-04: 模态框队列单例 ✅（v0.8.5 已合规）
- INV-05: AbortController 请求取消 ✅（Step 2/7 修复）
- INV-06: handleHttpError 错误恢复 ✅（Step 3 修复）
- INV-07: 预设场景持久化 ✅（v0.8.5 已合规）
- INV-08: 道同构度数据完整性 ✅（v0.8.5 已合规）
- INV-09: 标签页切换请求取消 ✅（v0.8.5 已合规）
- INV-10: Z-index 层级正确 ✅（v0.8.5 已合规）

**阶段 13.4**：用户反馈三大问题验证

1. ✅ 启动服务取消按钮可中断请求（Step 2 修复）
2. ✅ 道同构度 4 指标正常显示（v0.8.5 已修复）
3. ✅ 预设场景刷新后恢复（v0.8.5 已修复）

#### 影响文件

无代码修改，仅执行测试。

#### 验证脚本

`G:\code-memory\scripts\verify_step13_acceptance_test_v086.ps1`

#### 验收标准

```gherkin
Given v0.8.6 所有修复已完成并编译成功
When 执行 CDP 37 个测试用例
Then 通过率 ≥ 97% (36/37)
And 交互韧性评分 ≥ 88
And 安全不变量合规率 100% (10/10)
And P0 问题数 = 0
And P1 问题数 = 0
And 用户反馈三大问题全部修复
And 控制台无 critical 错误
```

#### 风险评估

| 风险维度 | 等级 | 说明 | 回滚策略 |
|:---|:---:|:---|:---|
| 测试风险 | 中 | 可能发现新的回归问题 | 逐个分析失败项 |
| 回滚难度 | 中 | 可能需回滚特定 Step | 按 Step 独立回滚 |

#### 回滚方案

1. 如验收测试失败，分析具体失败的 Step
2. 按 Step 的回滚方案独立回滚
3. 修复后重新执行验收测试
4. 必要时降级到 v0.8.5

---

## 八、附录

### 8.1 验证脚本清单

| Step | 脚本路径 | 用途 |
|:---:|:---|:---|
| 1 | `scripts\verify_step1_csp_v086.ps1` | 验证 CSP meta 标签存在且策略正确 |
| 2 | `scripts\verify_step2_abort_controller_v086.ps1` | 验证取消按钮可中断请求 |
| 3 | `scripts\verify_step3_handle_http_error_v086.ps1` | 验证 handleHttpError 被调用 |
| 4 | `scripts\verify_step4_modal_css_v086.ps1` | 验证模态框 CSS 隐藏正确 |
| 5 | `scripts\verify_step5_showToast_window_v086.ps1` | 验证 showToast 暴露到 window |
| 6 | `scripts\verify_step6_validateInput_window_v086.ps1` | 验证 validateInput 暴露到 window |
| 7 | `scripts\verify_step7_abort_controller_window_v086.ps1` | 验证 AbortController 变量暴露 |
| 8 | `scripts\verify_step8_interval_id_v086.ps1` | 验证 intervalId 属性名正确 |
| 9 | `scripts\verify_step9_maxlength_v086.ps1` | 验证输入框 maxlength 限制 |
| 10 | `scripts\verify_step10_404_resources_v086.ps1` | 验证无 404 资源加载错误 |
| 11 | `scripts\verify_step11_version_v086.ps1` | 验证版本号统一为 0.8.6 |
| 12 | `scripts\verify_step12_docs_sync_v086.ps1` | 验证文档同步完成 |
| 13 | `scripts\verify_step13_acceptance_test_v086.ps1` | 执行完整验收测试 |

### 8.2 执行顺序

```
Step 1 (CSP) → Step 2 (AbortController) → Step 3 (handleHttpError)
    ↓
Step 4 (模态框 CSS) → Step 5 (showToast) → Step 6 (validateInput)
    ↓
Step 7 (AbortController 暴露) → Step 8 (intervalId)
    ↓
Step 9 (maxlength) → Step 10 (404 资源)
    ↓
Step 11 (版本升级) → Step 12 (文档同步) → Step 13 (验收测试)
```

### 8.3 PowerShell 脚本执行规范

所有验证脚本必须遵循 PowerShell 专家规范：

1. **脚本头部**：包含 `#Requires -Version 5.1` 和 UTF-8 BOM
2. **错误处理**：使用 `$ErrorActionPreference = "Stop"`
3. **输出格式**：使用 `Write-Host` 输出彩色结果（绿色 PASS，红色 FAIL）
4. **退出码**：成功返回 0，失败返回 1
5. **中文编码**：确保控制台使用 UTF-8 编码（`[Console]::OutputEncoding = [System.Text.Encoding]::UTF8`）
6. **参数化**：支持 `-CdpPort`、`-SidecarPort` 等参数，默认值 9222/3099

### 8.4 风险总览

| Step | 风险等级 | 关键风险点 | 回滚难度 |
|:---:|:---:|:---|:---:|
| 1 | 低 | CSP 可能阻止资源 | 低 |
| 2 | 中 | Tauri invoke 无法真正中断 | 低 |
| 3 | 高 | 修改核心 fetch 函数 | 中 |
| 4 | 低 | CSS 规则冲突 | 低 |
| 5 | 低 | 暴露到 window | 低 |
| 6 | 低 | 暴露到 window | 低 |
| 7 | 低 | 暴露到 window | 低 |
| 8 | 低 | 属性名变更 | 低 |
| 9 | 低 | maxlength 限制 | 低 |
| 10 | 低 | 资源引用变更 | 低 |
| 11 | 低 | 版本号变更 | 低 |
| 12 | 低 | 文档更新 | 低 |
| 13 | 中 | 验收测试可能失败 | 中 |

### 8.5 成功标准

v0.8.6 发布成功的标准：

1. ✅ 所有 13 个 Step 的验证脚本通过
2. ✅ CDP 测试通过率 ≥ 97%
3. ✅ 交互韧性评分 ≥ 88
4. ✅ 安全不变量合规率 100%
5. ✅ 用户反馈三大问题全部修复
6. ✅ 编译成功无错误
7. ✅ 控制台无 critical 错误
8. ✅ 文档同步完成

---

## 九、实施原则

### 9.1 分步实施规范

1. **每个 Step 独立修改**：禁止批量修改，每个 Step 只修改必要的文件
2. **逐个验证**：每个 Step 完成后立即运行验证脚本
3. **失败回滚**：验证失败时立即回滚，不继续后续 Step
4. **记忆同步**：每完成 3 个 Step 同步一次 LRC 记忆库

### 9.2 PowerShell 操作规范

1. **脚本先写后执行**：复杂脚本先写入 .ps1 文件再执行
2. **引号转义**：遵循 PowerShell 专家规范处理引号转义
3. **中文编码**：所有脚本使用 UTF-8 编码
4. **错误处理**：所有外部命令调用检查 `$LASTEXITCODE`

### 9.3 安全规范

1. **不修改核心算法**：本次修复不涉及 src/engine/ 算法文件
2. **不引入新依赖**：仅修改现有代码
3. **保守修改**：优先最小化改动，避免重构

---

**文档结束**

> 本计划严格遵循分步实施原则，每个 Step 独立可验证、可回滚。执行过程中如遇阻塞性问题，应立即停止并报告，禁止强行推进。
