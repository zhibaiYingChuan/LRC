# LRC 桌面端 v0.8.7 修复优化计划

> **版本**：v0.8.7（基于 v0.8.6 HCSE 验证报告与新发现 3 个问题）
> **制定日期**：2026-07-30
> **目标版本**：v0.8.7
> **基线版本**：v0.8.6（HCSE 验证通过率 90.9%，安全不变量合规率 100%，P0 全部消除）
> **输入文档**：
> - [HCSE_VERIFICATION_REPORT_v0.8.6.md](./HCSE_VERIFICATION_REPORT_v0.8.6.md)
> - [FIX_PLAN_v0.8.6.md](./FIX_PLAN_v0.8.6.md)
> - [CDP_TEST_REPORT_v0.8.5.md](./CDP_TEST_REPORT_v0.8.5.md)
> **制定者**：创始者智能体（creator-product-designer）
> **文档版本**：v1.0

---

## 一、概述

### 1.1 版本目标

v0.8.7 的核心目标是**修复 v0.8.6 HCSE 验证发现的 3 个新问题**，使 LRC 桌面端达到生产就绪（Production-Ready）标准，并为 v0.9.x 系列的 Tauri invoke 真正中断、500 错误自动化测试等中期改进奠定基础。

| 目标维度 | v0.8.6 基线 | v0.8.7 目标 | 提升幅度 |
|:---|:---:|:---:|:---:|
| HCSE 验证通过率 | 90.9% (10/11) | **100% (11/11)** | +9.1% |
| Step 10 (404 资源) 完成度 | 部分修复 | **完全修复** | +50% |
| 图标 HTTP 404 数 | 33 次 | **0 次** | -33 |
| 图片加载失败率 | 34.5% (39/113) | **0%** | -34.5% |
| 版本号一致性（5 处配置 + 状态页 + 日志） | 部分一致 | **全部一致 v0.8.7** | +3 处 |
| 生产就绪度 | 接近就绪 | **就绪** | — |

### 1.2 RICE 优先级矩阵

对 3 个新发现问题进行 RICE 评分（Reach × Impact × Confidence ÷ Effort），确定修复顺序：

| Step | 问题 ID | 问题描述 | R | I | C | E | RICE | 优先级 |
|:---:|:---|:---|:---:|:---:|:---:|:---:|:---:|:---:|
| 1 | HCSE-P1 | sidecar 静态资源嵌入不完整（21 个图标 404） | 10 | 4 | 10 | 3 | **133.3** | P1 |
| 2 | HCSE-P2 | 系统状态页 sys-version 硬编码 v0.8.2 | 8 | 2 | 10 | 1 | **160** | P2 |
| 3 | HCSE-P3 | 日志前缀硬编码版本号（20 处） | 6 | 1 | 10 | 2 | **30** | P3 |

**RICE 评分说明**：
- **Reach（影响范围，1-10）**：受影响的用户数或场景数
- **Impact（影响程度，1-5）**：对用户体验/可信度的影响程度
- **Confidence（信心度，1-10）**：对修复方案正确性的信心
- **Effort（工作量，1-10）**：所需人天工作量（数值越小工作量越大）

**特殊说明**：
- Step 1 的 RICE 评分低于 Step 2，但**优先级 P1 高于 P2**。MoSCoW 优先级原则覆盖 RICE 排序：Must have > Should have > Could have。因此实施顺序仍为 Step 1 → Step 2 → Step 3。
- Step 1（图标嵌入）虽 Effort=3（涉及 21 个文件嵌入 + 路由扩展 + 编译验证），但其 Reach=10（影响全部用户）+ Impact=4（碎图标严重影响可信度），是阻碍生产就绪的最大短板。

### 1.3 MoSCoW 优先级分类

| 类别 | 问题 | 说明 |
|:---|:---|:---|
| **Must have (P1)** | sidecar 静态资源嵌入不完整 | 阻碍生产就绪，必须修复 |
| **Should have (P2)** | sys-version 硬编码 | 影响版本一致性，应该修复 |
| **Could have (P3)** | 日志前缀硬编码 | 不影响功能，可修复 |
| **Won't have** | CSP nonce 化、Tauri invoke 真正中断 | 延后至 v0.9.x |

### 1.4 修复顺序总览

遵循 **P1 → P2 → P3 → 收尾** 的优先级分组原则，禁止批量修改，所有步骤分步实施：

| 阶段 | Step | 问题 | RICE | 预计工作量 | 验证方式 |
|:---:|:---:|:---|:---:|:---:|:---|
| **P1 修复** | 1 | sidecar 图标嵌入（18 个 icon-*.svg） | 133.3 | 0.5 人天 | cargo build + curl 验证 |
| | 2 | sidecar power-*.svg 嵌入（新增 handler） | — | 0.3 人天 | cargo build + curl 验证 |
| **P2 修复** | 3 | sys-version 动态填充 | 160 | 0.2 人天 | CDP DOM 检查 |
| **P3 修复** | 4a | 日志前缀修复（v0.8.2 → 模板） | 30 | 0.2 人天 | 控制台日志验证 |
| | 4b | 日志前缀修复（v0.6.0 → 模板） | — | 0.2 人天 | 控制台日志验证 |
| | 4c | 日志前缀修复（v0.8.3 → 模板） | — | 0.1 人天 | 控制台日志验证 |
| **收尾** | 5 | 版本号升级到 v0.8.7（6 处配置） | — | 0.2 人天 | 版本一致性脚本 |
| | 6 | 编译验证与端到端测试 | — | 0.5 人天 | HCSE 验证脚本 |
| | 7 | 文档同步与 HCSE 报告 | — | 0.3 人天 | 文档审查 |
| **合计** | | | | **2.2 人天** | |

---

## 二、问题陈述

### 2.1 问题 1：sidecar 静态资源嵌入不完整 [P1]

#### 现象
- 22 个图标文件通过 HTTP 返回 404，但本地文件存在于 `G:\code-memory\static\assets\icons\`
- 33 次网络 404 请求，39 个图片加载失败（占 34.5%）
- 用户在仪表盘看到碎图标，严重影响产品可信度

#### 根因分析（基于实际代码调查）

**调查位置**：[`g:\code-memory\src\server.rs:1797-1848`](file:///G:/code-memory/src/server.rs#L1797-1848) `icon_asset_handler` 函数

**嵌入方式**：使用 `include_str!` 宏逐个声明 const 常量，**不是** `include_dir!` glob 模式。

```rust
// src/server.rs:1800-1815 当前实现（仅嵌入 15 个图标）
const ICON_DASHBOARD: &str = include_str!("../static/assets/icons/icon-dashboard.svg");
const ICON_SEARCH: &str = include_str!("../static/assets/icons/icon-search-lrc.svg");
// ... 共 15 个常量
const ICON_INTEGRITY: &str = include_str!("../static/assets/icons/icon-integrity.svg");
```

**根因**：
1. `icon_asset_handler` 函数中 `include_str!` 仅声明了 15 个图标常量，遗漏了 18 个 `index.html` 实际引用的图标
2. 完全没有 `power-*.svg` 的处理逻辑（3 个文件）
3. 路由匹配 `_ => None`，导致缺失图标返回 404

#### 缺失图标清单（基于 index.html 实际引用 + LS 目录对比）

**18 个缺失的 icon-*.svg**（被 index.html 引用但未嵌入）：

| 序号 | 文件名 | 引用位置（index.html 行号） | 用途 |
|:---:|:---|:---|:---|
| 1 | icon-project.svg | L50, L108, L1417, L1617 | MCP 配置入口 |
| 2 | icon-save.svg | L657, L661, L1985 | 数据备份与恢复 |
| 3 | icon-export.svg | L664, L2037 | 导出记忆 |
| 4 | icon-import.svg | L667, L2047 | 导入记忆 |
| 5 | icon-lightning.svg | L743, L1445 | 快速上手 |
| 6 | icon-config.svg | L784 | 配置指南 |
| 7 | icon-info.svg | L852, L1003, L1459, L1799 | FAQ/更多资源 |
| 8 | icon-smile.svg | L938, L1570 | 使用技巧 |
| 9 | icon-settings.svg | L1429, L1874 | 系统设置 |
| 10 | icon-search.svg | L1495, L1989 | 通用检索 |
| 11 | icon-folder.svg | L1507, L1515 | 选择文件夹 |
| 12 | icon-warning.svg | L1590 | 警告提示 |
| 13 | icon-embed.svg | L1732 | 嵌入配置 |
| 14 | icon-download.svg | L1823 | 下载并安装 |
| 15 | icon-llm.svg | L1853 | LLM 配置 |
| 16 | icon-cloud.svg | L1871 | 云端 API |
| 17 | icon-delete.svg | L1993 | 删除操作 |
| 18 | icon-chart.svg | L2008 | 图表展示 |

**3 个缺失的 power-*.svg**（被 index.html 引用但无任何 handler）：

| 序号 | 文件名 | 引用位置 | 用途 |
|:---:|:---|:---|:---|
| 19 | power-growth.svg | L1072 | 三才宝典-成长 |
| 20 | power-balance.svg | L1083 | 三才宝典-取舍 |
| 21 | power-shield.svg | L1094 | 三才宝典-托付 |

#### 证据链
- `Test-Path G:\code-memory\static\assets\icons\icon-project.svg` → True（文件存在）
- `Invoke-WebRequest http://127.0.0.1:3099/assets/icons/icon-project.svg` → 404
- `Invoke-WebRequest http://127.0.0.1:3099/assets/icons/icon-dashboard.svg` → 200（已嵌入）

### 2.2 问题 2：系统状态页版本号硬编码 [P2]

#### 现象
- 系统状态页显示 `v0.8.2`，与状态栏 `v0.8.6` 不一致
- 用户对版本真实性产生怀疑

#### 根因分析（基于实际代码调查）

**调查位置**：
- [`g:\code-memory\static\index.html:1314`](file:///G:/code-memory/static/index.html#L1314) 系统状态页元素
- [`g:\code-memory\static\index.html:2103`](file:///G:/code-memory/static/index.html#L2103) 状态栏元素
- [`g:\code-memory\static\app.js:850`](file:///G:/code-memory/static/app.js#L850) `updateStatusBar` 函数

**关键代码对比**：

```html
<!-- index.html:1314 系统状态页（硬编码，无 JS 填充） -->
<div style="font-size: 1.2em; font-weight: 600; color: var(--lrc-墨韵-700);"
     id="sys-version">v0.8.2</div>

<!-- index.html:2103 状态栏（硬编码 v0.8.6，会被 JS 覆盖） -->
<span>版本 <strong id="status-version">v0.8.6</strong></span>
```

```javascript
// app.js:850 只填充 status-version，未填充 sys-version
const version = $('status-version');  // 注意：是 status-version，不是 sys-version

// app.js:868 只更新 status-version
if (version) version.textContent = 'v' + APP_VERSION;
```

**根因**：
1. `sys-version` 元素在 HTML 中硬编码为 `v0.8.2`，从未被任何 JS 逻辑更新
2. `updateStatusBar` 函数仅填充 `status-version`（状态栏），未覆盖 `sys-version`（系统状态页）
3. 版本号升级时遗漏了 `sys-version` 元素

### 2.3 问题 3：日志前缀版本号硬编码 [P3]

#### 现象
- 控制台日志显示 `[LRC v0.8.2]`、`[LRC v0.6.0]`、`[LRC v0.8.3]` 等旧版本号
- 不影响功能，但破坏版本一致性观感

#### 根因分析（基于实际代码调查）

**调查位置**：[`g:\code-memory\static\app.js`](file:///G:/code-memory/static/app.js) 多处 console.log/error/warn

**硬编码位置清单（共 20 处）**：

| 版本号 | 出现次数 | 行号位置 | 函数/场景 |
|:---|:---:|:---|:---|
| `[LRC v0.8.2]` | 9 处 | L316, L377, L396, L5895, L6469, L6494, L6528, L6539, L6544 | SidecarHealthMonitor、selectEmbedderModel、集中事件绑定 |
| `[LRC v0.6.0]` | 9 处 | L4211, L4220, L4262, L4391, L4398, L4520, L4524, L4663, L4686 | 隐私检查、道同构度、演化时间线、记忆搜索 |
| `[LRC v0.8.3]` | 2 处 | L6566, L6574 | 网络在线/离线监听 |

**根因**：
1. 日志字符串中硬编码版本号，未引用 `APP_VERSION` 常量
2. `APP_VERSION` 在 [app.js:6](file:///G:/code-memory/static/app.js#L6) 已定义（`const APP_VERSION = '0.8.6';`），但日志未使用
3. 历史版本升级时未同步更新日志前缀

**关键代码示例**：

```javascript
// app.js:6 已有版本常量
const APP_VERSION = '0.8.6';
window.__LRC_VERSION__ = APP_VERSION;

// app.js:316 硬编码（应改为模板字符串）
console.log('[LRC v0.8.2] Sidecar 健康监测器已启动，轮询间隔:', this._pollInterval + 'ms');
// 期望：console.log(`[LRC v${APP_VERSION}] Sidecar 健康监测器已启动，轮询间隔:`, this._pollInterval + 'ms');
```

**作用域分析**：
- `APP_VERSION` 定义在 IIFE 外部（第 6 行，全局作用域）
- IIFE 从第 9 行开始：`(function() { 'use strict';`
- 所有硬编码日志均在 IIFE 内部（行号 > 9），可通过闭包访问 `APP_VERSION`
- 注意：第 6566/6574 行 `window.addEventListener` 若在 IIFE 外，需用 `window.__LRC_VERSION__` 兜底

---

## 三、目标与成功指标

### 3.1 业务目标

| 目标 | 衡量指标 | 验收标准 |
|:---|:---|:---|
| **G1 生产就绪** | 图标加载完整性 | 21 个缺失图标全部 HTTP 200 |
| **G2 版本一致性** | 版本号显示统一 | 5 处配置 + 状态页 + 日志前缀全部显示 v0.8.7 |
| **G3 HCSE 全通过** | HCSE 验证通过率 | 11/11 = 100% |
| **G4 用户可信度** | 碎图标率 | 0%（当前 34.5%） |

### 3.2 成功指标（SMART）

- **S**pecific：修复 21 个图标 404 + 1 处 sys-version + 20 处日志前缀
- **M**easurable：curl HTTP 状态码 + CDP DOM 检查 + 控制台日志
- **A**chievable：2.5 人天内完成（基于代码调查已确认修复方案）
- **R**elevant：直接服务于"Loong 投入生产环境"项目目标
- **T**ime-bound：2026-07-30 完成，与 v0.8.6 同日迭代

### 3.3 安全不变量（v0.8.7 增量）

| ID | 不变量名称 | 验证方式 | 优先级 |
|:---|:---|:---|:---:|
| INV-11 | 静态资源完整性 | 21 个图标 curl 返回 200 + Content-Type: image/svg+xml | P1 |
| INV-12 | 版本号全局一致性 | 8 处版本号全部为 v0.8.7（5 配置 + status-version + sys-version + 日志前缀） | P2 |

---

## 四、用户故事

### 4.1 用户故事 1：终端用户查看仪表盘

```
As a 终端用户（开发者）
I want to 在仪表盘看到完整的图标显示
So that 我能快速识别功能入口，不怀疑产品成熟度
Acceptance Criteria:
- Given 用户访问 http://127.0.0.1:3099/dashboard
  When 页面加载完成
  Then 所有 21 个缺失图标（icon-project, icon-save, power-growth 等）HTTP 200 且正确渲染
  And 控制台无 404 错误日志
  And 碎图标率从 34.5% 降至 0%
```

### 4.2 用户故事 2：运维人员核对版本

```
As a 运维人员
I want to 系统状态页显示的版本号与状态栏、日志前缀一致
So that 我能准确判断当前运行版本，避免版本混淆
Acceptance Criteria:
- Given sidecar 已启动 v0.8.7
  When 用户打开"系统状态"页查看版本号
  Then sys-version 元素显示 "v0.8.7"
  And status-version 状态栏显示 "v0.8.7"
  And 控制台日志前缀为 "[LRC v0.8.7]"
  And Cargo.toml / tauri.conf.json / app.js APP_VERSION 全部为 0.8.7
```

### 4.3 用户故事 3：开发者调试日志

```
As a 前端开发者
I want to 日志前缀动态使用 APP_VERSION
So that 后续版本升级时日志前缀自动同步，无需手动修改 20 处字符串
Acceptance Criteria:
- Given 开发者升级版本号（修改 app.js APP_VERSION）
  When 重新加载页面触发任意日志
  Then 日志前缀自动显示新版本号（如 "[LRC v0.8.8]"）
  And 无任何硬编码版本号残留
```

### 4.4 用户故事 4：HCSE 验证架构师

```
As a HCSE 验证架构师
I want to 通过自动化脚本验证 21 个图标完整嵌入
So that 能快速回归验证，确保 sidecar 编译流程未遗漏资源
Acceptance Criteria:
- Given sidecar v0.8.7 已编译并启动（端口 3099）
  When 执行验证脚本 Verify-Icons.ps1
  Then 21 个图标全部返回 HTTP 200
  And Content-Type 头为 "image/svg+xml; charset=utf-8"
  And 响应体以 "<svg" 或 "<?xml" 开头
```

---

## 五、功能需求

### 5.1 P1 修复（Must have）

#### FR-P1-01：扩展 icon_asset_handler 嵌入 18 个缺失 icon-*.svg

**影响文件**：
- [`g:\code-memory\src\server.rs`](file:///G:/code-memory/src/server.rs)（第 1801-1832 行扩展）

**修改内容**：

在 `icon_asset_handler` 函数中，于现有 15 个 `const` 声明之后（[第 1815 行后](file:///G:/code-memory/src/server.rs#L1815)），新增 18 个 `include_str!` 常量：

```rust
// v0.8.7 Step 1：补全 index.html 实际引用但未嵌入的 18 个图标
const ICON_PROJECT: &str = include_str!("../static/assets/icons/icon-project.svg");
const ICON_SAVE: &str = include_str!("../static/assets/icons/icon-save.svg");
const ICON_EXPORT: &str = include_str!("../static/assets/icons/icon-export.svg");
const ICON_IMPORT: &str = include_str!("../static/assets/icons/icon-import.svg");
const ICON_LIGHTNING: &str = include_str!("../static/assets/icons/icon-lightning.svg");
const ICON_CONFIG: &str = include_str!("../static/assets/icons/icon-config.svg");
const ICON_INFO: &str = include_str!("../static/assets/icons/icon-info.svg");
const ICON_SMILE: &str = include_str!("../static/assets/icons/icon-smile.svg");
const ICON_SETTINGS: &str = include_str!("../static/assets/icons/icon-settings.svg");
const ICON_SEARCH: &str = include_str!("../static/assets/icons/icon-search.svg");
const ICON_FOLDER: &str = include_str!("../static/assets/icons/icon-folder.svg");
const ICON_WARNING: &str = include_str!("../static/assets/icons/icon-warning.svg");
const ICON_EMBED: &str = include_str!("../static/assets/icons/icon-embed.svg");
const ICON_DOWNLOAD: &str = include_str!("../static/assets/icons/icon-download.svg");
const ICON_LLM: &str = include_str!("../static/assets/icons/icon-llm.svg");
const ICON_CLOUD: &str = include_str!("../static/assets/icons/icon-cloud.svg");
const ICON_DELETE: &str = include_str!("../static/assets/icons/icon-delete.svg");
const ICON_CHART: &str = include_str!("../static/assets/icons/icon-chart.svg");
```

在 `match filename.as_str()` 分支中（[第 1817-1834 行](file:///G:/code-memory/src/server.rs#L1817-1834)），于 `"icon-integrity.svg"` 分支之后新增 18 个映射：

```rust
"icon-project.svg" => Some(ICON_PROJECT),
"icon-save.svg" => Some(ICON_SAVE),
"icon-export.svg" => Some(ICON_EXPORT),
"icon-import.svg" => Some(ICON_IMPORT),
"icon-lightning.svg" => Some(ICON_LIGHTNING),
"icon-config.svg" => Some(ICON_CONFIG),
"icon-info.svg" => Some(ICON_INFO),
"icon-smile.svg" => Some(ICON_SMILE),
"icon-settings.svg" => Some(ICON_SETTINGS),
"icon-search.svg" => Some(ICON_SEARCH),
"icon-folder.svg" => Some(ICON_FOLDER),
"icon-warning.svg" => Some(ICON_WARNING),
"icon-embed.svg" => Some(ICON_EMBED),
"icon-download.svg" => Some(ICON_DOWNLOAD),
"icon-llm.svg" => Some(ICON_LLM),
"icon-cloud.svg" => Some(ICON_CLOUD),
"icon-delete.svg" => Some(ICON_DELETE),
"icon-chart.svg" => Some(ICON_CHART),
```

同步更新函数文档注释（[第 1795-1796 行](file:///G:/code-memory/src/server.rs#L1795-1796)）：

```rust
/// 支持的图标：dashboard/search-lrc/captain-log/trust/benchmark/audit/
/// baga/health/decay/luoshu/memory/crystallization/privacy/network/integrity
/// v0.8.7 补全：project/save/export/import/lightning/config/info/smile/
/// settings/search/folder/warning/embed/download/llm/cloud/delete/chart
```

#### FR-P1-02：新增 power_asset_handler 处理 3 个 power-*.svg

**影响文件**：
- [`g:\code-memory\src\server.rs`](file:///G:/code-memory/src/server.rs)（新增函数 + 路由注册）

**修改内容**：

在 `icon_asset_handler` 函数之后（[第 1848 行后](file:///G:/code-memory/src/server.rs#L1848)），新增 `power_asset_handler` 函数：

```rust
/// 龙忆设计系统 v1.0 — 三才宝典图标端点
///
/// v0.8.7 新增：仪表盘"三才宝典"模块引用 /assets/icons/power-*.svg，
/// 此端点将编译时嵌入的 SVG 内容以 `image/svg+xml` MIME 类型返回。
/// 支持的图标：power-growth（成长）/power-balance（取舍）/power-shield（托付）
async fn power_asset_handler(
    axum::extract::Path(filename): axum::extract::Path<String>,
) -> axum::response::Response<String> {
    // 编译时嵌入三才宝典图标 SVG 文件
    const POWER_GROWTH: &str = include_str!("../static/assets/icons/power-growth.svg");
    const POWER_BALANCE: &str = include_str!("../static/assets/icons/power-balance.svg");
    const POWER_SHIELD: &str = include_str!("../static/assets/icons/power-shield.svg");

    let content = match filename.as_str() {
        "power-growth.svg" => Some(POWER_GROWTH),
        "power-balance.svg" => Some(POWER_BALANCE),
        "power-shield.svg" => Some(POWER_SHIELD),
        _ => None,
    };
    match content {
        Some(svg) => axum::response::Response::builder()
            .header("Content-Type", "image/svg+xml; charset=utf-8")
            .body(svg.to_string())
            .unwrap_or_else(|e| {
                eprintln!("[server] 三才宝典 SVG 响应构建失败: {}", e);
                axum::response::Response::new(String::new())
            }),
        None => axum::response::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(format!("<!-- Power icon not found: {} -->", filename))
            .unwrap_or_else(|_| axum::response::Response::new(String::new())),
    }
}
```

在路由注册处（搜索 `icon_asset_handler` 路由注册位置），新增 `power_asset_handler` 路由。**需先调查路由注册代码**：

```rust
// 假设路由注册形如（需在实施时确认实际位置）：
.route("/assets/icons/:filename", get(icon_asset_handler))
// v0.8.7 新增：三才宝典图标路由
.route("/assets/icons/power-:filename", get(power_asset_handler))
```

**注意**：由于 `icon_asset_handler` 已匹配 `/assets/icons/:filename`，`power-*.svg` 也会进入该 handler。两种方案可选：
- **方案 A（推荐）**：在 `icon_asset_handler` 的 match 分支中扩展 power-*.svg（合并到一个 handler，更简单）
- **方案 B**：新增独立 `power_asset_handler` 并配置优先路由

**实施决策**：采用**方案 A**，将 3 个 power-*.svg 合并到 `icon_asset_handler`，减少路由复杂度。FR-P1-02 调整为在 FR-P1-01 的修改中追加 3 个 power 常量：

```rust
// v0.8.7 Step 1（续）：三才宝典图标
const POWER_GROWTH: &str = include_str!("../static/assets/icons/power-growth.svg");
const POWER_BALANCE: &str = include_str!("../static/assets/icons/power-balance.svg");
const POWER_SHIELD: &str = include_str!("../static/assets/icons/power-shield.svg");
```

```rust
"power-growth.svg" => Some(POWER_GROWTH),
"power-balance.svg" => Some(POWER_BALANCE),
"power-shield.svg" => Some(POWER_SHIELD),
```

**修订**：FR-P1-01 与 FR-P1-02 合并为单个 Step 1，共嵌入 21 个图标。

### 5.2 P2 修复（Should have）

#### FR-P2-01：修复 sys-version 版本号硬编码

**影响文件**：
- [`g:\code-memory\static\app.js`](file:///G:/code-memory/static/app.js)（`updateStatusBar` 函数扩展）
- [`g:\code-memory\static\index.html`](file:///G:/code-memory/static/index.html)（第 1314 行硬编码兜底）

**修改方案**：采用双保险策略——HTML 兜底 + JS 动态填充。

**Step 3a：HTML 兜底值更新**

将 [index.html:1314](file:///G:/code-memory/static/index.html#L1314) 的硬编码从 `v0.8.2` 改为 `v0.8.7`，作为 JS 未执行时的兜底：

```html
<!-- 修改前 -->
<div style="font-size: 1.2em; font-weight: 600; color: var(--lrc-墨韵-700);"
     id="sys-version">v0.8.2</div>

<!-- 修改后 -->
<div style="font-size: 1.2em; font-weight: 600; color: var(--lrc-墨韵-700);"
     id="sys-version">v0.8.7</div>
```

**Step 3b：JS 动态填充逻辑**

在 [app.js:850](file:///G:/code-memory/static/app.js#L850) `updateStatusBar` 函数中，于现有 `const version = $('status-version');` 之后新增 `sysVersion` 变量：

```javascript
// 修改前（app.js:850-851）
const version = $('status-version');
const dataDir = $('status-data-dir');

// 修改后
const version = $('status-version');
const sysVersion = $('sys-version');  // v0.8.7 新增：系统状态页版本号
const dataDir = $('status-data-dir');
```

在 [app.js:868](file:///G:/code-memory/static/app.js#L868) 现有填充逻辑之后，新增 `sysVersion` 填充：

```javascript
// 修改前（app.js:868）
if (version) version.textContent = 'v' + APP_VERSION;

// 修改后
if (version) version.textContent = 'v' + APP_VERSION;
if (sysVersion) sysVersion.textContent = 'v' + APP_VERSION;  // v0.8.7 新增
```

**注意**：`$('sys-version')` 假设 `$` 是 `document.getElementById` 的别名（需在实施时确认 app.js 顶部 `$` 的定义）。

### 5.3 P3 修复（Could have）

#### FR-P3-01：修复日志前缀版本号硬编码（v0.8.2 → 模板字符串）

**影响文件**：
- [`g:\code-memory\static\app.js`](file:///G:/code-memory/static/app.js)（9 处修改）

**修改方案**：将 `[LRC v0.8.2]` 全部替换为 `[LRC v${APP_VERSION}]`，使用模板字符串（反引号包裹）。

**修改清单**：

| 行号 | 修改前 | 修改后 |
|:---|:---|:---|
| L316 | `console.log('[LRC v0.8.2] Sidecar 健康监测器已启动，轮询间隔:', this._pollInterval + 'ms');` | ``console.log(`[LRC v${APP_VERSION}] Sidecar 健康监测器已启动，轮询间隔:`, this._pollInterval + 'ms');`` |
| L377 | `console.log('[LRC v0.8.2] Sidecar 已恢复可达');` | ``console.log(`[LRC v${APP_VERSION}] Sidecar 已恢复可达`);`` |
| L396 | `console.log('[LRC v0.8.2] Sidecar 不可达，已禁用 API 按钮');` | ``console.log(`[LRC v${APP_VERSION}] Sidecar 不可达，已禁用 API 按钮`);`` |
| L5895 | `console.warn('[LRC v0.8.2] selectEmbedderModel 未找到匹配卡片:', modelId);` | ``console.warn(`[LRC v${APP_VERSION}] selectEmbedderModel 未找到匹配卡片:`, modelId);`` |
| L6469 | `console.warn('[LRC v0.8.2] 未找到 data-action 对应函数:', action);` | ``console.warn(`[LRC v${APP_VERSION}] 未找到 data-action 对应函数:`, action);`` |
| L6494 | `console.error('[LRC v0.8.2] data-action 执行异常:', action, e);` | ``console.error(`[LRC v${APP_VERSION}] data-action 执行异常:`, action, e);`` |
| L6528 | `console.warn('[LRC v0.8.2] 未找到 data-input-action 对应函数:', action);` | ``console.warn(`[LRC v${APP_VERSION}] 未找到 data-input-action 对应函数:`, action);`` |
| L6539 | `console.error('[LRC v0.8.2] data-input-action 执行异常:', action, e);` | ``console.error(`[LRC v${APP_VERSION}] data-input-action 执行异常:`, action, e);`` |
| L6544 | `console.log('[LRC v0.8.2] 集中事件绑定完成，共绑定', actionEls.length, '个元素');` | ``console.log(`[LRC v${APP_VERSION}] 集中事件绑定完成，共绑定`, actionEls.length, '个元素');`` |

**实施约束**：禁止使用全局查找替换（Find & Replace All），必须逐行 Edit 工具调用修改，确保每处变更可独立回滚。

#### FR-P3-02：修复日志前缀版本号硬编码（v0.6.0 → 模板字符串）

**影响文件**：
- [`g:\code-memory\static\app.js`](file:///G:/code-memory/static/app.js)（9 处修改）

**修改清单**：

| 行号 | 修改前 | 修改后 |
|:---|:---|:---|
| L4211 | `console.log('[LRC v0.6.0] 隐私检查完成，耗时 ${elapsed}ms，信任等级: ${trustLevel}');` | ``console.log(`[LRC v${APP_VERSION}] 隐私检查完成，耗时 ${elapsed}ms，信任等级: ${trustLevel}`);`` |
| L4220 | `console.error('[LRC v0.6.0] 隐私检查失败:', err);` | ``console.error(`[LRC v${APP_VERSION}] 隐私检查失败:`, err);`` |
| L4262 | `console.warn('[LRC v0.6.0] 加载结晶历史失败，使用预览数据:', err.message);` | ``console.warn(`[LRC v${APP_VERSION}] 加载结晶历史失败，使用预览数据:`, err.message);`` |
| L4391 | `console.log('[LRC v0.6.0] 道同构度加载完成，健康评分: ${score}');` | ``console.log(`[LRC v${APP_VERSION}] 道同构度加载完成，健康评分: ${score}`);`` |
| L4398 | `console.warn('[LRC v0.6.0] 道同构度加载失败，使用默认值:', err.message);` | ``console.warn(`[LRC v${APP_VERSION}] 道同构度加载失败，使用默认值:`, err.message);`` |
| L4520 | `console.log('[LRC v0.6.0] 演化时间线加载了 ${data.events.length} 条事件');` | ``console.log(`[LRC v${APP_VERSION}] 演化时间线加载了 ${data.events.length} 条事件`);`` |
| L4524 | `console.warn('[LRC v0.6.0] 演化时间线加载失败，使用示例数据:', err.message);` | ``console.warn(`[LRC v${APP_VERSION}] 演化时间线加载失败，使用示例数据:`, err.message);`` |
| L4663 | `console.log('[LRC v0.6.0] 记忆搜索完成，返回 ${filtered.length} 条结果');` | ``console.log(`[LRC v${APP_VERSION}] 记忆搜索完成，返回 ${filtered.length} 条结果`);`` |
| L4686 | `console.error('[LRC v0.6.0] 记忆搜索失败:', err);` | ``console.error(`[LRC v${APP_VERSION}] 记忆搜索失败:`, err);`` |

**注意**：L4211/L4391/L4520/L4663 原本已使用模板字符串，但版本号是硬编码字面量。修改时需保留外层反引号，仅替换 `v0.6.0` 为 `v${APP_VERSION}`。

#### FR-P3-03：修复日志前缀版本号硬编码（v0.8.3 → 模板字符串）

**影响文件**：
- [`g:\code-memory\static\app.js`](file:///G:/code-memory/static/app.js)（2 处修改）

**修改清单**：

| 行号 | 修改前 | 修改后 |
|:---|:---|:---|
| L6566 | `console.warn('[LRC v0.8.3] 网络已断开');` | ``console.warn(`[LRC v${APP_VERSION}] 网络已断开`);`` |
| L6574 | `console.log('[LRC v0.8.3] 网络已恢复');` | ``console.log(`[LRC v${APP_VERSION}] 网络已恢复`);`` |

**作用域注意**：L6566/L6574 是 `window.addEventListener` 全局监听器。需确认这两行是否在 IIFE 内部：
- 若在 IIFE 内部：直接使用 `APP_VERSION`（闭包可访问）
- 若在 IIFE 外部：使用 `window.__LRC_VERSION__` 兜底

实施时需先读取 app.js 末尾确认 IIFE 闭合位置，再决定使用哪个变量。

### 5.4 收尾需求

#### FR-FINAL-01：版本号升级到 v0.8.7

**影响文件清单（5 处配置 + 1 处兜底，共 6 处需修改）**：

| 序号 | 文件 | 行号 | 修改前 | 修改后 |
|:---:|:---|:---:|:---|:---|
| 1 | [`g:\code-memory\Cargo.toml`](file:///G:/code-memory/Cargo.toml) | L7 | `version = "0.8.6"` | `version = "0.8.7"` |
| 2 | [`g:\code-memory\desktop\src-tauri\Cargo.toml`](file:///G:/code-memory/desktop/src-tauri/Cargo.toml) | L3 | `version = "0.8.6"` | `version = "0.8.7"` |
| 3 | [`g:\code-memory\desktop\src-tauri\tauri.conf.json`](file:///G:/code-memory/desktop/src-tauri/tauri.conf.json) | L4 | `"version": "0.8.6"` | `"version": "0.8.7"` |
| 4 | [`g:\code-memory\static\app.js`](file:///G:/code-memory/static/app.js) | L6 | `const APP_VERSION = '0.8.6';` | `const APP_VERSION = '0.8.7';` |
| 5 | [`g:\code-memory\static\index.html`](file:///G:/code-memory/static/index.html) | L2103 | `<strong id="status-version">v0.8.6</strong>` | `<strong id="status-version">v0.8.7</strong>` |

**额外同步**：
- [index.html:1314](file:///G:/code-memory/static/index.html#L1314) `sys-version` 兜底值同步为 `v0.8.7`（FR-P2-01 Step 3a 已覆盖）

**验证脚本对应**：[`scripts/Verify-Version.ps1`](file:///G:/code-memory/scripts/Verify-Version.ps1) 需在配置项清单中加入 `desktop/src-tauri/Cargo.toml` 校验（共 6 处配置 + 1 处 sys-version + 1 处日志前缀 = 8 项验证）

---

## 六、非功能需求

### 6.1 性能需求

| 指标 | 目标 | 验证方式 |
|:---|:---|:---|
| 图标 HTTP 响应时间 | < 50ms（本地 sidecar） | curl `-w %{time_total}` |
| sidecar 二进制大小增长 | < 100KB（21 个 SVG ≈ 30-50KB） | 编译前后对比 |
| 图标 SVG 平均大小 | < 3KB | 文件系统检查 |
| 前端无新增运行时开销 | 日志前缀模板字符串无性能损耗 | V8 优化（模板字符串单次解析） |

### 6.2 安全需求

| 指标 | 目标 | 验证方式 |
|:---|:---|:---|
| CSP 不变式 INV-01 | 保持合规（不引入新违规） | CDP `cspViolations: []` |
| SVG 内容安全 | 无 `<script>` 标签注入 | 静态扫描 21 个 SVG 文件 |
| 路径遍历防护 | `:filename` 参数不突破 `assets/icons/` 边界 | axum `Path` 提取器默认防护 |

### 6.3 可维护性需求

| 指标 | 目标 | 验证方式 |
|:---|:---|:---|
| 日志前缀去硬编码 | 0 处硬编码版本号（APP_VERSION 唯一来源） | grep `\[LRC v0\.` 应无匹配 |
| 版本号单点维护 | 升级版本时仅需修改 5 处配置 + 0 处日志 | grep 验证 |
| 代码注释完整 | 新增 handler 包含文档注释 | 代码审查 |

### 6.4 兼容性需求

| 指标 | 目标 |
|:---|:---|
| Rust MSRV | 1.70+（不变） |
| Tauri 版本 | 2.x（不变） |
| 浏览器兼容性 | Chrome/Edge 90+（模板字符串 ES6 已广泛支持） |
| WebView2 兼容性 | Windows 10/11 默认 WebView2 支持 |

---

## 七、技术考量

### 7.1 sidecar 静态资源嵌入方式评估

**当前方案**：`include_str!` 逐个常量声明（21 + 15 = 36 个常量）

**优点**：
- 编译时嵌入，无运行时文件系统依赖
- 二进制自包含，支持离线运行
- 类型安全，编译期检查文件存在性

**缺点**：
- 每新增图标需修改源码（维护成本）
- 36 个常量略显冗长

**替代方案评估**：

| 方案 | 优点 | 缺点 | 决策 |
|:---|:---|:---|:---:|
| `include_dir!` 宏（glob） | 一次嵌入整个目录 | 需新增依赖 `include_dir = "0.7"` | **延后**（v0.9.x） |
| 运行时 `tokio::fs::read` | 无需重新编译 | 破坏离线运行 + 二进制自包含 | **拒绝** |
| `include_str!` 逐个（当前） | 零依赖，编译期检查 | 代码冗长 | **采用**（v0.8.7） |

**决策**：v0.8.7 保持 `include_str!` 方案，避免引入新依赖（生产就绪优先）。v0.9.x 评估迁移到 `include_dir!`。

### 7.2 路由匹配优先级

**潜在问题**：`/assets/icons/:filename` 路由会匹配所有 `power-*.svg` 请求。若 FR-P1-02 采用独立 handler（方案 B），需配置路由优先级。

**采用方案 A**：将 3 个 power-*.svg 合并到 `icon_asset_handler`，避免路由冲突。函数语义可接受（power-*.svg 也是图标资源）。

### 7.3 模板字符串作用域

**问题**：`APP_VERSION` 定义在 [app.js:6](file:///G:/code-memory/static/app.js#L6)（IIFE 外部全局作用域），IIFE 内部日志能否访问？

**验证**：
- `const APP_VERSION = '0.8.6';` 在第 6 行（全局作用域）
- IIFE 从第 9 行开始：`(function() { 'use strict';`
- IIFE 内部可通过闭包访问外部 `const` 变量（JavaScript 作用域规则）
- 模板字符串 `${APP_VERSION}` 在 IIFE 内部求值时，沿作用域链查找，可访问全局 `APP_VERSION`

**结论**：方案可行。但 L6566/L6574 的 `window.addEventListener` 若在 IIFE 外部，需确认 IIFE 闭合位置。

### 7.4 编译流程影响

**sidecar 编译命令**（需在实施时确认）：
```powershell
cargo build --release --features server
```

**编译时间影响**：新增 21 个 `include_str!` 不会显著增加编译时间（每个宏在编译期读取一次文件）。

**二进制大小影响**：21 个 SVG 文件预计增加 30-50KB（每个 SVG 平均 1.5-2.5KB），对总二进制大小（约 20MB）可忽略。

### 7.5 核心算法文件保护

**硬约束**：核心算法文件 `src/engine/` 必须通过泄漏检测。

**v0.8.7 修改范围**：仅 `src/server.rs` + `static/index.html` + `static/app.js` + 配置文件，**不涉及 `src/engine/`**。

**结论**：无需泄漏检测，但建议在编译后运行一次 `cargo test` 确保引擎单元测试通过。

---

## 八、时间线与里程碑

### 8.1 分步实施计划

| 阶段 | Step | 任务 | 工作量 | 验证方式 | 可回滚性 |
|:---:|:---:|:---|:---:|:---|:---|
| **P1** | 1 | 修改 src/server.rs：扩展 icon_asset_handler 嵌入 21 个图标 | 0.5 人天 | cargo build + Verify-Icons.ps1 | git revert 单文件 |
| **P2** | 2 | 修改 static/index.html L1314：sys-version 兜底值 v0.8.2 → v0.8.7 | 0.1 人天 | 浏览器查看 | git revert 单文件 |
| | 3 | 修改 static/app.js L850/L868：新增 sysVersion 填充逻辑 | 0.1 人天 | CDP DOM 检查 | git revert 单文件 |
| **P3** | 4a | 修改 static/app.js：9 处 `[LRC v0.8.2]` → 模板字符串 | 0.2 人天 | 控制台日志 | 逐处 git revert |
| | 4b | 修改 static/app.js：9 处 `[LRC v0.6.0]` → 模板字符串 | 0.2 人天 | 控制台日志 | 逐处 git revert |
| | 4c | 修改 static/app.js：2 处 `[LRC v0.8.3]` → 模板字符串 | 0.1 人天 | 控制台日志 | 逐处 git revert |
| **收尾** | 5 | 升级版本号到 v0.8.7（5 处配置） | 0.2 人天 | 版本一致性脚本 | git revert |
| | 6 | 编译 sidecar + 端到端验证 | 0.5 人天 | HCSE 验证脚本 | — |
| | 7 | 编写 HCSE 验证报告 + 文档同步 | 0.3 人天 | 文档审查 | — |
| **合计** | | | **2.2 人天** | | |

### 8.2 关键里程碑

| 里程碑 | 完成标志 | 预计时间 |
|:---|:---|:---|
| M1：P1 修复完成 | 21 个图标 HTTP 200 | T+0.5 人天 |
| M2：P2 修复完成 | sys-version 显示 v0.8.7 | T+0.7 人天 |
| M3：P3 修复完成 | 日志前缀全部 `[LRC v0.8.7]` | T+1.2 人天 |
| M4：版本号升级 | 5 处配置全部 v0.8.7 | T+1.4 人天 |
| M5：端到端验证 | HCSE 验证 11/11 通过 | T+1.9 人天 |
| M6：文档同步 | HCSE 报告 v0.8.7 发布 | T+2.2 人天 |

### 8.3 关键路径

```
Step 1 (P1 图标嵌入) ──┐
                       ├──> Step 5 (版本号升级) ──> Step 6 (编译验证) ──> Step 7 (文档)
Step 2-3 (P2 sys-version) ─┘                        │
Step 4a-c (P3 日志前缀) ────────────────────────────┘
```

**关键路径**：Step 1 → Step 5 → Step 6（编译验证依赖 Step 1 的源码修改 + Step 5 的版本号同步）。

---

## 九、风险评估与回滚方案

### 9.1 风险矩阵

| 风险 ID | 风险描述 | 概率 | 影响 | RPN | 缓解措施 |
|:---:|:---|:---:|:---:|:---:|:---|
| R1 | `include_str!` 文件路径错误导致编译失败 | 低 | 中 | 6 | 编译前用 Test-Path 验证 21 个文件存在 |
| R2 | 路由冲突导致 power-*.svg 仍 404 | 低 | 高 | 9 | 采用方案 A（合并 handler），编译后 curl 验证 3 个 power-*.svg |
| R3 | 模板字符串语法错误导致 app.js 加载失败 | 低 | 高 | 9 | 每次修改后用 `node -c app.js` 语法检查 + 浏览器加载验证 |
| R4 | `$` 别名未定义导致 sysVersion 填充失败 | 中 | 中 | 9 | 实施前确认 app.js 顶部 `$` 定义；回退用 `document.getElementById` |
| R5 | L6566/L6574 在 IIFE 外部，APP_VERSION 不可访问 | 中 | 低 | 3 | 实施前读取 app.js 末尾确认 IIFE 闭合位置；回退用 `window.__LRC_VERSION__` |
| R6 | sidecar 编译失败（依赖问题） | 低 | 高 | 6 | 编译前 `cargo check --features server` 预检 |
| R7 | 图标 SVG 内容含恶意脚本 | 极低 | 高 | 5 | 静态扫描 21 个 SVG（grep `<script>`） |
| R8 | 版本号遗漏某处配置 | 中 | 中 | 9 | 升级后 grep `0\.8\.6` 全项目扫描确认无残留 |

### 9.2 回滚方案

#### 9.2.1 单步骤回滚（推荐）

每个 Step 完成后立即 git commit，便于精准回滚：

```powershell
# PowerShell 编码自保护块
$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$PSDefaultParameterValues['Out-File:Encoding'] = 'utf8'

# 每个 Step 单独 commit
git add src/server.rs
git commit -m "v0.8.7 Step 1: extend icon_asset_handler with 21 missing icons"

# 回滚单个 Step
git revert <commit-hash>
```

#### 9.2.2 全版本回滚（紧急）

若 v0.8.7 整体失败，回滚到 v0.8.6：

```powershell
# PowerShell 编码自保护块
$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$PSDefaultParameterValues['Out-File:Encoding'] = 'utf8'

# 确认当前状态
git log --oneline -10

# 回滚到 v0.8.6 最后一次 commit
git reset --hard <v0.8.6-last-commit>

# 重新编译 sidecar
cargo build --release --features server
```

#### 9.2.3 紧急止血（生产环境）

若生产环境出现严重问题（如 sidecar 无法启动），无需回滚代码，直接重启 v0.8.6 sidecar：

```powershell
# 停止 v0.8.7 sidecar
Stop-Process -Name "code-memory-server" -Force -ErrorAction SilentlyContinue

# 启动 v0.8.6 备份 sidecar（需提前备份）
Start-Process "G:\rust-target\release-v0.8.6\code-memory-server.exe"
```

---

## 十、验证脚本与验收标准

### 10.1 验证脚本清单

所有验证脚本存放于 [`g:\code-memory\scripts\`](file:///G:/code-memory/scripts/) 目录：

| 脚本名称 | 用途 | 依赖 |
|:---|:---|:---|
| `Verify-Icons.ps1` | 验证 21 个图标 HTTP 200 | sidecar 已启动 |
| `Verify-Version.ps1` | 验证 8 处版本号一致性 | sidecar + 前端 |
| `Verify-LogPrefix.ps1` | 验证日志前缀动态填充 | CDP MCP |
| `Verify-Compile.ps1` | 验证 sidecar 编译成功 | cargo |
| `HCSE-Verify-v0.8.7.ps1` | 端到端 HCSE 验证 | 全部 |

### 10.2 Verify-Icons.ps1（图标完整性验证）

**路径**：`g:\code-memory\scripts\Verify-Icons.ps1`

```powershell
# ============================================================
# LRC v0.8.7 图标完整性验证脚本
# 验证 21 个新增图标 + 15 个原有图标全部 HTTP 200
# ============================================================

# PowerShell 编码自保护块（必需）
$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$PSDefaultParameterValues['Out-File:Encoding'] = 'utf8'

$ErrorActionPreference = 'Stop'

# sidecar 端点
$baseUrl = 'http://127.0.0.1:3099/assets/icons'

# 待验证的 36 个图标清单
$icons = @(
    # 原有 15 个
    'icon-dashboard', 'icon-search-lrc', 'icon-captain-log', 'icon-trust',
    'icon-benchmark', 'icon-audit', 'icon-bagua', 'icon-health', 'icon-decay',
    'icon-luoshu', 'icon-memory', 'icon-crystallization', 'icon-privacy',
    'icon-network', 'icon-integrity',
    # v0.8.7 新增 18 个 icon-*.svg
    'icon-project', 'icon-save', 'icon-export', 'icon-import',
    'icon-lightning', 'icon-config', 'icon-info', 'icon-smile',
    'icon-settings', 'icon-search', 'icon-folder', 'icon-warning',
    'icon-embed', 'icon-download', 'icon-llm', 'icon-cloud',
    'icon-delete', 'icon-chart',
    # v0.8.7 新增 3 个 power-*.svg
    'power-growth', 'power-balance', 'power-shield'
)

$passed = 0
$failed = 0
$failedList = @()

foreach ($icon in $icons) {
    $url = "$baseUrl/$icon.svg"
    try {
        $response = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 5
        $contentType = $response.Headers['Content-Type']
        $body = $response.Content

        if ($response.StatusCode -eq 200 -and
            $contentType -like 'image/svg+xml*' -and
            ($body -match '<svg' -or $body -match '<\?xml')) {
            Write-Host "[PASS] $icon.svg (HTTP 200, SVG valid)" -ForegroundColor Green
            $passed++
        } else {
            Write-Host "[FAIL] $icon.svg (Status: $($response.StatusCode), CT: $contentType)" -ForegroundColor Red
            $failed++
            $failedList += $icon
        }
    } catch {
        $status = $_.Exception.Response.StatusCode.value__
        Write-Host "[FAIL] $icon.svg (HTTP $status)" -ForegroundColor Red
        $failed++
        $failedList += $icon
    }
}

Write-Host ""
Write-Host "=========================================="
Write-Host "  图标完整性验证结果"
Write-Host "=========================================="
Write-Host "  通过: $passed / $($icons.Count)"
Write-Host "  失败: $failed"
if ($failed -gt 0) {
    Write-Host "  失败列表: $($failedList -join ', ')"
    exit 1
} else {
    Write-Host "  全部通过" -ForegroundColor Green
    exit 0
}
```

### 10.3 Verify-Version.ps1（版本号一致性验证）

**路径**：`g:\code-memory\scripts\Verify-Version.ps1`

```powershell
# ============================================================
# LRC v0.8.7 版本号一致性验证脚本
# 验证 8 处版本号全部为 v0.8.7
# ============================================================

# PowerShell 编码自保护块（必需）
$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$PSDefaultParameterValues['Out-File:Encoding'] = 'utf8'

$ErrorActionPreference = 'Stop'
$expectedVersion = '0.8.7'

$results = @()

# 1. Cargo.toml
$cargo = Get-Content 'G:\code-memory\Cargo.toml' -Raw
if ($cargo -match "version\s*=\s*`"$expectedVersion`"") {
    $results += [PSCustomObject]@{File='Cargo.toml'; Status='PASS'}
} else {
    $results += [PSCustomObject]@{File='Cargo.toml'; Status='FAIL'}
}

# 2. tauri.conf.json
$tauri = Get-Content 'G:\code-memory\desktop\src-tauri\tauri.conf.json' -Raw
if ($tauri -match "`"version`"\s*:\s*`"$expectedVersion`"") {
    $results += [PSCustomObject]@{File='tauri.conf.json'; Status='PASS'}
} else {
    $results += [PSCustomObject]@{File='tauri.conf.json'; Status='FAIL'}
}

# 3. app.js APP_VERSION
$appJs = Get-Content 'G:\code-memory\static\app.js' -Raw
if ($appJs -match "const APP_VERSION\s*=\s*'$expectedVersion'") {
    $results += [PSCustomObject]@{File='app.js APP_VERSION'; Status='PASS'}
} else {
    $results += [PSCustomObject]@{File='app.js APP_VERSION'; Status='FAIL'}
}

# 4. index.html status-version
$indexHtml = Get-Content 'G:\code-memory\static\index.html' -Raw
if ($indexHtml -match "id=`"status-version`">v$expectedVersion<") {
    $results += [PSCustomObject]@{File='index.html status-version'; Status='PASS'}
} else {
    $results += [PSCustomObject]@{File='index.html status-version'; Status='FAIL'}
}

# 5. index.html sys-version
if ($indexHtml -match "id=`"sys-version`">v$expectedVersion<") {
    $results += [PSCustomObject]@{File='index.html sys-version'; Status='PASS'}
} else {
    $results += [PSCustomObject]@{File='index.html sys-version'; Status='FAIL'}
}

# 6. 日志前缀无硬编码（应无 [LRC v0.x.x] 字面量）
$hardcodedPatterns = @('\[LRC v0\.8\.2\]', '\[LRC v0\.6\.0\]', '\[LRC v0\.8\.3\]', '\[LRC v0\.8\.6\]')
$hasHardcoded = $false
foreach ($pattern in $hardcodedPatterns) {
    if ($appJs -match $pattern) {
        $hasHardcoded = $true
        Write-Host "[FAIL] 发现硬编码日志前缀: $pattern" -ForegroundColor Red
    }
}
if (-not $hasHardcoded) {
    $results += [PSCustomObject]@{File='app.js 日志前缀'; Status='PASS'}
} else {
    $results += [PSCustomObject]@{File='app.js 日志前缀'; Status='FAIL'}
}

# 输出结果
Write-Host ""
Write-Host "=========================================="
Write-Host "  版本号一致性验证结果"
Write-Host "=========================================="
$results | Format-Table -AutoSize

$failedCount = ($results | Where-Object { $_.Status -eq 'FAIL' }).Count
if ($failedCount -gt 0) {
    Write-Host "  失败: $failedCount 项" -ForegroundColor Red
    exit 1
} else {
    Write-Host "  全部通过" -ForegroundColor Green
    exit 0
}
```

### 10.4 验收标准（Gherkin 格式）

#### Feature 1：图标完整性

```gherkin
Feature: sidecar 静态资源完整性
  作为终端用户
  我希望仪表盘所有图标正确加载
  以便快速识别功能入口

  Scenario Outline: 21 个新增图标返回 HTTP 200
    Given sidecar v0.8.7 已启动并监听端口 3099
    When 客户端请求 /assets/icons/<icon>.svg
    Then 响应状态码为 200
    And 响应头 Content-Type 为 "image/svg+xml; charset=utf-8"
    And 响应体包含 "<svg" 或 "<?xml"

    Examples:
      | icon |
      | icon-project |
      | icon-save |
      | icon-export |
      | icon-import |
      | icon-lightning |
      | icon-config |
      | icon-info |
      | icon-smile |
      | icon-settings |
      | icon-search |
      | icon-folder |
      | icon-warning |
      | icon-embed |
      | icon-download |
      | icon-llm |
      | icon-cloud |
      | icon-delete |
      | icon-chart |
      | power-growth |
      | power-balance |
      | power-shield |

  Scenario: 仪表盘加载无 404 错误
    Given 用户访问 http://127.0.0.1:3099/dashboard
    When 页面完成加载
    Then 浏览器控制台无 "404" 网络错误
    And 碎图标数量为 0
```

#### Feature 2：版本号一致性

```gherkin
Feature: 版本号全局一致性
  作为运维人员
  我希望所有版本号显示统一
  以便准确判断当前运行版本

  Scenario: 系统状态页显示 v0.8.7
    Given sidecar v0.8.7 已启动
    And 用户访问 http://127.0.0.1:3099/dashboard
    When 用户切换到"系统状态"标签页
    Then id="sys-version" 元素的 textContent 为 "v0.8.7"

  Scenario: 状态栏显示 v0.8.7
    Given sidecar v0.8.7 已启动
    And 用户访问 http://127.0.0.1:3099/dashboard
    When 页面加载完成
    Then id="status-version" 元素的 textContent 为 "v0.8.7"

  Scenario: 日志前缀动态填充
    Given sidecar v0.8.7 已启动
    And 用户访问 http://127.0.0.1:3099/dashboard
    When 控制台输出任意 [LRC ...] 日志
    Then 日志前缀为 "[LRC v0.8.7]"
    And 不存在 "[LRC v0.8.2]" 或 "[LRC v0.6.0]" 字面量

  Scenario: 版本升级后日志自动同步
    Given 开发者将 app.js APP_VERSION 修改为 "0.8.8"
    When 重新加载页面触发任意日志
    Then 日志前缀自动变为 "[LRC v0.8.8]"
    And 无需手动修改任何日志字符串
```

#### Feature 3：HCSE 验证全通过

```gherkin
Feature: HCSE 验证 100% 通过
  作为 HCSE 验证架构师
  我希望 v0.8.7 所有不变量合规
  以便确认生产就绪

  Scenario: INV-11 静态资源完整性合规
    Given sidecar v0.8.7 已启动
    When 执行 Verify-Icons.ps1 脚本
    Then 36 个图标全部返回 HTTP 200
    And 脚本退出码为 0

  Scenario: INV-12 版本号一致性合规
    Given sidecar v0.8.7 已启动
    When 执行 Verify-Version.ps1 脚本
    Then 8 处版本号全部为 v0.8.7
    And 脚本退出码为 0

  Scenario: HCSE 验证通过率 100%
    Given 所有 P1/P2/P3 修复已完成
    When 执行 HCSE-Verify-v0.8.7.ps1 端到端验证
    Then 验证通过率为 11/11 (100%)
    And 安全不变量合规率为 12/12 (100%，含新增 INV-11/INV-12)
```

---

## 十一、HCSE-Verify-v0.8.7.ps1（端到端验证主脚本）

**路径**：`g:\code-memory\scripts\HCSE-Verify-v0.8.7.ps1`

```powershell
# ============================================================
# LRC v0.8.7 HCSE 端到端验证主脚本
# 验证 3 个修复项 + 2 个新增不变量
# ============================================================

# PowerShell 编码自保护块（必需）
$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$PSDefaultParameterValues['Out-File:Encoding'] = 'utf8'

$ErrorActionPreference = 'Continue'

Write-Host "=========================================="
Write-Host "  LRC v0.8.7 HCSE 端到端验证"
Write-Host "=========================================="
Write-Host ""

$totalChecks = 0
$passedChecks = 0

# ---------- 阶段 1：编译验证 ----------
Write-Host "[阶段 1] 编译验证" -ForegroundColor Cyan
$totalChecks++
try {
    Push-Location 'G:\code-memory'
    cargo build --release --features server 2>&1 | Tee-Object -Variable buildLog
    if ($LASTEXITCODE -eq 0) {
        Write-Host "[PASS] sidecar 编译成功" -ForegroundColor Green
        $passedChecks++
    } else {
        Write-Host "[FAIL] sidecar 编译失败" -ForegroundColor Red
    }
    Pop-Location
} catch {
    Write-Host "[FAIL] 编译异常: $_" -ForegroundColor Red
}

# ---------- 阶段 2：sidecar 启动 ----------
Write-Host ""
Write-Host "[阶段 2] sidecar 启动验证" -ForegroundColor Cyan
$totalChecks++
$sidecarProc = Get-Process -Name "code-memory-server" -ErrorAction SilentlyContinue
if (-not $sidecarProc) {
    Write-Host "  启动 sidecar..."
    Start-Process "G:\rust-target\release\code-memory-server.exe"
    Start-Sleep -Seconds 3
    $sidecarProc = Get-Process -Name "code-memory-server" -ErrorAction SilentlyContinue
}
if ($sidecarProc) {
    Write-Host "[PASS] sidecar 已启动 (PID: $($sidecarProc.Id))" -ForegroundColor Green
    $passedChecks++
} else {
    Write-Host "[FAIL] sidecar 启动失败" -ForegroundColor Red
}

# ---------- 阶段 3：图标完整性（INV-11）----------
Write-Host ""
Write-Host "[阶段 3] INV-11 静态资源完整性" -ForegroundColor Cyan
$totalChecks++
$iconScript = 'G:\code-memory\scripts\Verify-Icons.ps1'
if (Test-Path $iconScript) {
    & $iconScript
    if ($LASTEXITCODE -eq 0) {
        Write-Host "[PASS] INV-11 图标完整性合规" -ForegroundColor Green
        $passedChecks++
    } else {
        Write-Host "[FAIL] INV-11 图标完整性不合规" -ForegroundColor Red
    }
} else {
    Write-Host "[SKIP] 验证脚本不存在: $iconScript" -ForegroundColor Yellow
}

# ---------- 阶段 4：版本号一致性（INV-12）----------
Write-Host ""
Write-Host "[阶段 4] INV-12 版本号一致性" -ForegroundColor Cyan
$totalChecks++
$versionScript = 'G:\code-memory\scripts\Verify-Version.ps1'
if (Test-Path $versionScript) {
    & $versionScript
    if ($LASTEXITCODE -eq 0) {
        Write-Host "[PASS] INV-12 版本号一致性合规" -ForegroundColor Green
        $passedChecks++
    } else {
        Write-Host "[FAIL] INV-12 版本号一致性不合规" -ForegroundColor Red
    }
} else {
    Write-Host "[SKIP] 验证脚本不存在: $versionScript" -ForegroundColor Yellow
}

# ---------- 阶段 5：sidecar API 健康检查 ----------
Write-Host ""
Write-Host "[阶段 5] sidecar /health API" -ForegroundColor Cyan
$totalChecks++
try {
    $health = Invoke-RestMethod -Uri 'http://127.0.0.1:3099/health' -TimeoutSec 5
    if ($health.version -eq '0.8.7') {
        Write-Host "[PASS] /health 返回 v0.8.7" -ForegroundColor Green
        $passedChecks++
    } else {
        Write-Host "[FAIL] /health 返回版本 $($health.version)（期望 0.8.7）" -ForegroundColor Red
    }
} catch {
    Write-Host "[FAIL] /health 请求失败: $_" -ForegroundColor Red
}

# ---------- 汇总 ----------
Write-Host ""
Write-Host "=========================================="
Write-Host "  HCSE 验证汇总"
Write-Host "=========================================="
Write-Host "  通过: $passedChecks / $totalChecks"
$passRate = [math]::Round($passedChecks / $totalChecks * 100, 1)
Write-Host "  通过率: $passRate%"
if ($passedChecks -eq $totalChecks) {
    Write-Host "  结论: 全部通过，v0.8.7 生产就绪" -ForegroundColor Green
    exit 0
} else {
    Write-Host "  结论: 存在失败项，需修复后重新验证" -ForegroundColor Red
    exit 1
}
```

---

## 十二、实施约束与合规要求

### 12.1 分步实施原则（强制）

**禁止批量修改**：所有代码变更必须分步实施，每个 Step 独立 commit。

| Step | 修改文件数 | 修改行数 | commit 粒度 |
|:---:|:---:|:---:|:---|
| 1 | 1（server.rs） | ~45 行新增 | 单次 commit |
| 2 | 1（index.html） | 1 行 | 单次 commit |
| 3 | 1（app.js） | 2 行新增 | 单次 commit |
| 4a | 1（app.js） | 9 行修改 | 单次 commit |
| 4b | 1（app.js） | 9 行修改 | 单次 commit |
| 4c | 1（app.js） | 2 行修改 | 单次 commit |
| 5 | 4-5（配置） | 5 行 | 单次 commit |

**commit message 规范**：
```
v0.8.7 Step <N>: <简要描述>

- 修改文件: <file>
- 修改内容: <detail>
- 验证方式: <verification>
- 关联问题: HCSE-P<N>
```

### 12.2 PowerShell 编码自保护（强制）

所有 PowerShell 脚本必须包含以下编码自保护块（置于脚本首行后）：

```powershell
$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$PSDefaultParameterValues['Out-File:Encoding'] = 'utf8'
```

### 12.3 核心算法文件保护

- **不修改** `src/engine/` 目录任何文件
- 编译后运行 `cargo test` 确认引擎单元测试通过
- 若 `cargo test` 失败，立即回滚

### 12.4 AI 工具规则配置

- 所有 AI 工具（Trae、Claude Code 等）必须配置 LRC 规则
- v0.8.7 修改涉及 sidecar + 前端，需在 AI 工具中同步规则

### 12.5 验证脚本前置条件

执行验证脚本前需确认：
1. sidecar 已停止（编译前）或已启动（验证时）
2. 端口 3099 未被占用
3. 工作目录为 `G:\code-memory`
4. PowerShell 执行策略允许运行脚本（`Set-ExecutionPolicy -Scope Process Bypass`）

---

## 十三、文档同步清单

### 13.1 需更新文档

| 文档 | 更新内容 | 负责人 |
|:---|:---|:---|
| `docs/HCSE_VERIFICATION_REPORT_v0.8.7.md` | 新建，记录 v0.8.7 验证结果 | HCSE 架构师 |
| `docs/FIX_PLAN_v0.8.7.md` | 本文档（已完成） | 创始者智能体 |
| `README.md` | 版本号同步到 v0.8.7 | 开发者 |
| `CHANGELOG.md` | 新增 v0.8.7 变更记录 | 开发者 |
| `static/index.html` | 版本号 + sys-version 兜底值 | Step 2/5 |

### 13.2 CHANGELOG.md 模板

```markdown
## v0.8.7 (2026-07-30)

### 修复
- **P1**: 修复 sidecar 静态资源嵌入不完整，21 个图标（18 icon-*.svg + 3 power-*.svg）HTTP 404 问题
- **P2**: 修复系统状态页 sys-version 硬编码 v0.8.2 问题，改为 JS 动态填充 APP_VERSION
- **P3**: 修复 20 处日志前缀硬编码版本号（v0.8.2/v0.6.0/v0.8.3），改为模板字符串 `${APP_VERSION}`

### 新增
- INV-11 静态资源完整性不变量
- INV-12 版本号全局一致性不变量
- `scripts/Verify-Icons.ps1` 图标完整性验证脚本
- `scripts/Verify-Version.ps1` 版本号一致性验证脚本
- `scripts/HCSE-Verify-v0.8.7.ps1` 端到端验证主脚本

### 改进
- sidecar icon_asset_handler 文档注释补全
- 版本号单点维护：升级时仅需修改 5 处配置 + 0 处日志

### 已知问题（延后至 v0.9.x）
- CSP nonce 化（当前仍使用 'unsafe-inline'）
- Tauri invoke 真正中断机制
- 500 错误 Modal 交互自动化测试
```

---

## 十四、附录

### 14.1 代码调查证据索引

| 调查项 | 文件路径 | 行号 | 证据 |
|:---|:---|:---:|:---|
| sidecar 图标嵌入实现 | `src/server.rs` | 1797-1848 | `icon_asset_handler` 函数，15 个 `include_str!` 常量 |
| sys-version 硬编码 | `static/index.html` | 1314 | `id="sys-version">v0.8.2</div>` |
| status-version 填充逻辑 | `static/app.js` | 850, 868 | `const version = $('status-version');` + `if (version) version.textContent = 'v' + APP_VERSION;` |
| APP_VERSION 定义 | `static/app.js` | 6 | `const APP_VERSION = '0.8.6';` |
| 日志前缀 v0.8.2 硬编码 | `static/app.js` | 316,377,396,5895,6469,6494,6528,6539,6544 | 9 处 `[LRC v0.8.2]` |
| 日志前缀 v0.6.0 硬编码 | `static/app.js` | 4211,4220,4262,4391,4398,4520,4524,4663,4686 | 9 处 `[LRC v0.6.0]` |
| 日志前缀 v0.8.3 硬编码 | `static/app.js` | 6566, 6574 | 2 处 `[LRC v0.8.3]` |
| 图标目录清单 | `static/assets/icons/` | — | 36 个 icon-*.svg + 3 个 power-*.svg = 39 个文件 |
| index.html 图标引用 | `static/index.html` | 50-2008 | 21 个缺失图标被引用 |
| Cargo.toml 版本号 | `Cargo.toml` | 7 | `version = "0.8.6"` |
| tauri.conf.json 版本号 | `desktop/src-tauri/tauri.conf.json` | 4 | `"version": "0.8.6"` |

### 14.2 缺失图标详细引用位置

| 图标文件 | index.html 引用行号 | 用途模块 |
|:---|:---|:---|
| icon-project.svg | L50, L108, L1417, L1617 | 侧边栏 / MCP 配置入口 |
| icon-save.svg | L657, L661, L1985 | 信任中心 / 数据备份 |
| icon-export.svg | L664, L2037 | 信任中心 / 导出记忆 |
| icon-import.svg | L667, L2047 | 信任中心 / 导入记忆 |
| icon-lightning.svg | L743, L1445 | 用户文档 / 快速上手 |
| icon-config.svg | L784 | 用户文档 / 配置指南 |
| icon-info.svg | L852, L1003, L1459, L1799 | 用户文档 / FAQ / MCP 配置提示 |
| icon-smile.svg | L938, L1570 | 用户文档 / 使用技巧 |
| icon-settings.svg | L1429, L1874 | MCP 配置 / 系统设置 |
| icon-search.svg | L1495, L1989 | MCP 配置 / 通用检索 |
| icon-folder.svg | L1507, L1515 | MCP 配置 / 选择文件夹 |
| icon-warning.svg | L1590 | MCP 配置 / 警告提示 |
| icon-embed.svg | L1732 | 系统设置 / 嵌入配置 |
| icon-download.svg | L1823 | 系统设置 / 下载并安装 |
| icon-llm.svg | L1853 | 系统设置 / LLM 配置 |
| icon-cloud.svg | L1871 | 系统设置 / 云端 API |
| icon-delete.svg | L1993 | 系统设置 / 删除操作 |
| icon-chart.svg | L2008 | 系统设置 / 图表展示 |
| power-growth.svg | L1072 | 三才宝典 / 成长 |
| power-balance.svg | L1083 | 三才宝典 / 取舍 |
| power-shield.svg | L1094 | 三才宝典 / 托付 |

### 14.3 v0.8.6 → v0.8.7 版本号变更对照表

| 配置项 | v0.8.6 | v0.8.7 | 文件位置 |
|:---|:---:|:---:|:---|
| Cargo.toml version | 0.8.6 | 0.8.7 | `Cargo.toml:7` |
| tauri.conf.json version | 0.8.6 | 0.8.7 | `desktop/src-tauri/tauri.conf.json:4` |
| app.js APP_VERSION | 0.8.6 | 0.8.7 | `static/app.js:6` |
| index.html status-version | v0.8.6 | v0.8.7 | `static/index.html:2103` |
| index.html sys-version | v0.8.2 | v0.8.7 | `static/index.html:1314` |
| 日志前缀 [LRC v0.8.2] | v0.8.2 | ${APP_VERSION} | `static/app.js` 9 处 |
| 日志前缀 [LRC v0.6.0] | v0.6.0 | ${APP_VERSION} | `static/app.js` 9 处 |
| 日志前缀 [LRC v0.8.3] | v0.8.3 | ${APP_VERSION} | `static/app.js` 2 处 |

### 14.4 验证脚本依赖关系

```
HCSE-Verify-v0.8.7.ps1
  ├── Verify-Compile.ps1 (cargo build)
  ├── Verify-Icons.ps1 (21 + 15 图标 HTTP 200)
  ├── Verify-Version.ps1 (8 处版本号一致性)
  └── /health API 验证 (sidecar 自报版本)
```

---

## 十五、计划评审与批准

### 15.1 评审检查清单

- [x] 已基于实际代码调查（非假设）
- [x] 每个问题有根因分析 + 代码证据
- [x] 每个修复方案有详细代码示例
- [x] 影响文件清单完整
- [x] 验证脚本路径明确
- [x] 验收标准使用 Gherkin 格式
- [x] 风险评估含 RPN 评分
- [x] 回滚方案分单步骤 + 全版本 + 紧急止血三层
- [x] RICE 评分完整
- [x] MoSCoW 优先级分类
- [x] 用户故事符合 As a / I want / So that 格式
- [x] 分步实施计划禁止批量修改
- [x] PowerShell 脚本含编码自保护块
- [x] 核心算法文件保护约束
- [x] 生产就绪性考量

### 15.2 待确认事项（已全部确认 ✓）

| 序号 | 确认项 | 确认结果 | 影响 Step | 结论 |
|:---:|:---|:---|:---:|:---|
| 1 | `desktop/src-tauri/Cargo.toml` version 字段 | **存在**，[L3](file:///G:/code-memory/desktop/src-tauri/Cargo.toml#L3) `version = "0.8.6"` | Step 5 | 需同步升级到 0.8.7（FR-FINAL-01 新增第 6 处配置） |
| 2 | app.js `$` 别名定义 | **存在**，[app.js:446](file:///G:/code-memory/static/app.js#L446) `function $(id) { ... document.getElementById(id); }`，且 [app.js:4001](file:///G:/code-memory/static/app.js#L4001) 暴露到 window | Step 3 | `$('sys-version')` 可行，方案无需调整 |
| 3 | app.js L6566/L6574 IIFE 作用域 | **在 IIFE 内部**，IIFE 闭合在 [app.js:6628](file:///G:/code-memory/static/app.js#L6628) `})();` | Step 4c | 可直接使用 `APP_VERSION`（闭包可访问），无需 `window.__LRC_VERSION__` 兜底 |
| 4 | icon_asset_handler 路由注册位置 | **已确认**，[server.rs:2834](file:///G:/code-memory/src/server.rs#L2834) `.route("/assets/icons/{filename}", get(icon_asset_handler))`（axum 0.7+ `{filename}` 语法） | Step 1 | 采用方案 A（合并 handler），无需新增路由 |

### 15.3 批准签字

| 角色 | 姓名 | 日期 | 签字 |
|:---|:---|:---|:---|
| 产品经理（创始者智能体） | 创世者设计 | 2026-07-30 | _自动批准_ |
| HCSE 验证架构师 | — | — | 待评审 |
| 开发负责人 | — | — | 待评审 |

---

**计划结束**

> 本计划由创始者智能体（creator-product-designer）基于 v0.8.6 HCSE 验证报告与实际代码调查编写。所有修复方案均有静态代码证据支撑，禁止假设。实施时遵循分步实施原则，每个 Step 独立可验证、可回滚。计划目标：使 LRC 桌面端 v0.8.7 达到生产就绪标准，HCSE 验证通过率 100%。
