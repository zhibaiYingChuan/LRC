# LRC 修复计划 v0.6.1 - v0.7.0

> **创建时间**: 2026-07-28
> **制定方**: Shannon 六钥匙 + 创作者产品经理 + 高级工程文化教练
> **依据**: END_TO_END_AUDIT_PLAN.md 审计报告

## 修复依赖链

```
v0.6.1 脚本(探照灯) → 所有修复的验证基础
    ↓
P1-2(枚举统一) ──┬─→ P1-3(switch_project)
                 └─→ P0-3(命令接入)
P1-1(配置链) ──→ P1-3(switch_project)
P0-2(导出) ──→ 独立,可随时做
    ↓
v0.7.0 E2E 测试套件(守护层)
```

## v0.6.1 — 底座重建(紧急版本)

### Step 1: 创建 audit_api_contract.py
- **修复**: 自动化 API 契约审计脚本
- **文件**: 新增 `scripts/audit_api_contract.py`
- **功能**: 对比前端 fetch 调用与 sidecar Rust route 定义,输出差异表
- **验证**: 脚本能输出 JSON 报告

### Step 2: 创建 audit_config_chain.py
- **修复**: 自动化配置链路审计脚本
- **文件**: 新增 `scripts/audit_config_chain.py`
- **功能**: 追踪 wizard.json 每个字段的消费路径,标记断点
- **验证**: 脚本能识别 P1-1(wizard.project_dir 未使用)

### Step 3: P1-2 memory_type 枚举统一
- **修复**: 前端 'general' 后端无法识别
- **文件**: `static/app.js` (导入逻辑)
- **方案**: 前端 'general' 映射为后端 'fact'(或后端新增 'general')
- **验证**: audit_api_contract.py 报告 memory_type 枚举零差异

### Step 4: P1-1 wizard.project_dir 配置链路
- **修复**: wizard.json 的 project_dir 未被 start_sidecar 使用
- **文件**: `desktop/src-tauri/src/commands.rs` (start_sidecar 函数)
- **方案**: src_dir 为 None 时,回退读取 wizard.json 的 project_dir
- **优先级链路**: 显式 src_dir > wizard.project_dir > --global
- **验证**: audit_config_chain.py 报告 wizard.project_dir 已被消费

## v0.6.2 — 功能补全

### Step 5: P0-2 导出代码片段空 query 回退
- **修复**: 空 query 返回空结果,功能失效
- **文件**: `src/v1_api.rs` (code/search handler)
- **方案**: query 为空时返回最近 N 条
- **验证**: 记忆库非空时导出得到非空结果

### Step 6: P1-3 switch_project 前端入口
- **修复**: switch_project 命令前端无调用入口
- **文件**: `static/app.js` (添加项目切换 UI)
- **依赖**: Step 3 + Step 4
- **验证**: 切换项目后 sidecar 参数变化

### Step 7: P0-3 前端补齐 Tauri 命令调用
- **修复**: 前端只调用 2/28 个 Tauri 命令
- **文件**: `static/app.js` (多处), `static/index.html` (高级管理面板)
- **依赖**: Step 3 (memory_type 统一)
- **分批**: 第一批核心 CRUD,第二批用户功能,第三批低频
- **验证**: audit_api_contract.py 报告前端调用数 ≥20
- **状态**: ✅ 已完成 (2026-07-28)
- **实现详情**:
  - 第一批 5 个核心 CRUD: stopSidecarService/listSidecarProjects/pickProjectDirectory/getWizardState/resetWizardState
  - 第二批 6 个用户功能: getLlmConfig/testLlmConnection/detectAgents/detectInstalledAgents/setProjectDir/getProjectDir
  - 第三批 10 个低频管理: startSidecarForProject/stopSidecarForProject/getAgentConfigGuide/discoverAllAgents/configureAgents/saveConfiguredAgents/scanIdeProjects/openSettings/markComplete/verifySetup
  - index.html 设置页新增"桌面端服务管理"卡片(第一批按钮) + "高级管理"折叠面板(第二/三批按钮)
  - toggleAdvancedManagement 函数实现折叠/展开切换
- **验证结果**:
  - Tauri 命令覆盖率: 27/30 = 90% (目标 ≥20,达成)
  - memory_type 枚举一致性: 无差异
  - HTTP 方法不匹配: 0 个
  - 未覆盖的 3 个命令(open_dashboard_window/navigate_main_to_dashboard/update_tray_tooltip)为系统内部调用,无需前端触发

## v0.6.1 完成总结

| Step | 内容 | 状态 |
|------|------|------|
| Step 1 | 创建 audit_api_contract.py | ✅ 已完成 |
| Step 2 | 创建 audit_config_chain.py | ✅ 已完成 |
| Step 3 | P1-2 memory_type 枚举统一 | ✅ 已完成 |
| Step 4 | P1-1 wizard.project_dir 配置链路 | ✅ 已完成 |
| Step 5 | P0-2 导出代码片段空 query 回退 | ✅ 已完成 |
| Step 6 | P1-3 switch_project 前端入口 | ✅ 已完成 |
| Step 7 | P0-3 前端补齐 Tauri 命令调用 | ✅ 已完成 |

**v0.6.1 所有计划任务已完成,可进入 v0.7.0 阶段。**

## v0.7.0 — 体验打磨 + E2E 测试

### Step 8: 审计脚本修复（audit_api_contract.py）
- **修复**: 审计脚本未处理 nest_service 前缀 + 静态资源误报 + 跨调用 method 误判
- **文件**: `scripts/audit_api_contract.py`
- **状态**: ✅ 已完成 (2026-07-28)
- **实现详情**:
  - `extract_rust_routes` 新增 `prefix` 参数,处理 `nest_service("/v1", ...)` 嵌套路由
  - `extract_frontend_api_calls` 修复 nearby method 检测逻辑,截断到下一个 `fetchWithTimeout` 调用之前
  - `audit_api_contract` 新增 `STATIC_ROUTE_PATTERNS` 排除静态资源路由(/app.js /app.css /assets/* 等)
  - 新增无效路径过滤(正则 `^/[a-zA-Z0-9/_-]+$`),排除 "/)" 等误匹配
- **验证结果**:
  - 修复前: 17 个幽灵调用 + 32 个孤儿路由 + 1 个方法不匹配
  - 修复后: 0 个幽灵调用 + 6 个孤儿路由(均为可选功能) + 0 个方法不匹配
  - 审计退出码: 0 (通过)

### Step 9: 真实孤儿路由评估
- **评估**: 6 个孤儿路由均为可选功能或内部 API,无需添加前端入口
  - `POST /v1/encode` — 内部 API,编码文本为洛书向量
  - `POST /v1/memories/correct` — 用户修正记忆,可选功能
  - `POST /v1/memories/unfold` — 拆解合成记忆,可选功能
  - `POST /v1/feedback` — 用户反馈回路,可选功能
  - `GET /v1/captains-log` — 船长日志演示端点,前端通过 `/v1/health/system` + `/v1/health/dao_metrics` 组合实现
  - `GET /v1/version/check` — 版本检查,可选功能
- **状态**: ✅ 已完成 (2026-07-28)

### Step 3-A ~ 3-F: 孤儿路由前端入口补全（用户反馈驱动）
> **背景**: 用户反馈"6 孤儿路由（可选功能）就不完善了吗？产品不好用"，决定为所有孤儿路由补全前端入口。

#### Step 3-A: /v1/captains-log 前端入口
- **修复**: generateCaptainLog 函数优先调用 /v1/captains-log 端点,失败回退到手动拼接
- **文件**: `static/app.js` (generateCaptainLog 函数)
- **状态**: ✅ 已完成 (2026-07-29)

#### Step 3-B: /v1/memories/correct 前端入口
- **修复**: 记忆详情面板新增"修正记忆"按钮,实现 correctMemory 函数
- **文件**: `static/app.js` (openMemoryDetail, correctMemory 函数)
- **状态**: ✅ 已完成 (2026-07-29)

#### Step 3-C: /v1/feedback 前端入口
- **修复**: 记忆详情面板新增"反馈"按钮,实现 submitMemoryFeedback 函数,支持 5 种反馈类型
- **文件**: `static/app.js` (openMemoryDetail, submitMemoryFeedback 函数)
- **状态**: ✅ 已完成 (2026-07-29)

#### Step 3-D: /v1/encode 前端入口
- **修复**: 仪表盘新增"洛书向量编码器"卡片,实现 encodeTextToLuoshu 函数,渲染 9 维向量九宫格
- **文件**: `static/index.html` (洛书向量编码器卡片), `static/app.js` (encodeTextToLuoshu 函数)
- **状态**: ✅ 已完成 (2026-07-29)
- **实现详情**:
  - HTML: 3x3 九宫格显示 9 维向量分量,元数据区显示八卦索引/类别/中心值/拓扑深度
  - JS: 调用 POST /v1/encode,按洛书九宫格传统排列(4|9|2/3|5|7/8|1|6)渲染向量
  - 修复: 同步修复 correctMemory/submitMemoryFeedback 的 window 导出遗漏 bug

#### Step 3-E: /v1/memories/unfold 前端入口
- **修复**: 记忆详情面板"拆解合成"按钮(仅合成记忆显示),实现 unfoldMemory 函数
- **文件**: `static/app.js` (unfoldMemory 函数)
- **状态**: ✅ 已完成 (2026-07-29)
- **实现详情**:
  - 调用 POST /v1/memories/unfold,请求体 {memory_id, min_activation: 0.1}
  - 在记忆详情面板动态追加拆解结果区域,显示子记忆列表(内容+八卦类别+权重)、保真度
  - 错误处理: 404(非合成类型)、500(拆解失败)均显示具体错误信息

#### Step 3-F: /v1/version/check 前端入口
- **修复**: 设置页新增"关于与更新"卡片,实现 checkVersionUpdate 函数
- **文件**: `static/index.html` (关于与更新卡片), `static/app.js` (checkVersionUpdate 函数)
- **状态**: ✅ 已完成 (2026-07-29)
- **实现详情**:
  - 调用 GET /v1/version/check,展示当前版本/最新版本/更新链接/下载链接
  - 三种状态 UI: 有新版本(金色提示)、已是最新(玉色提示)、检查失败(朱砂色错误)
  - 隐私说明: 显示"仅在点击时发起请求,不会自动上报"

#### 审计验证
- **审计结果**: 0 个幽灵调用 + 0 个孤儿路由 + 0 个方法不匹配
- **Tauri 覆盖率**: 90% (27/30)
- **memory_type 一致性**: 无差异
- **审计退出码**: 0 (通过)

### Step 10: Clippy 代码质量修复
- **修复**: 6 个 Clippy warning（redundant closure / io_other_error / vec_init_then_push / match_result_ok）
- **文件**: `src/engine/embedder.rs`, `src/engine/exploration_log.rs`, `src/server.rs`
- **状态**: ✅ 已完成 (2026-07-28)
- **实现详情**:
  - `embedder.rs:286` - `.map_err(|e| EmbedError::Network(e))` → `.map_err(EmbedError::Network)`
  - `exploration_log.rs:200` - `std::io::Error::new(ErrorKind::Other, ...)` → `std::io::Error::other(...)`
  - `exploration_log.rs:249` - `|e| std::io::Error::other(e)` → `std::io::Error::other`（二次优化）
  - `exploration_log.rs:253` - `std::io::Error::new(ErrorKind::Other, ...)` → `std::io::Error::other(...)`
  - `server.rs:2504` - `Vec::new()` + 6 个 `push()` → `vec![]` 宏初始化
  - `server.rs:2783` - `if let Some(s) = origin.to_str().ok()` → `if let Ok(s) = origin.to_str()`
- **验证结果**: Clippy 0 warning ✅

### Step 11: E2E 测试套件
- **修复**: 防止修复回归
- **文件**: 新增 `e2e/` 目录
- **验证**: CI 中每次 PR 自动跑 E2E
- **状态**: ⏳ 待实施

### Step 12: P2-P3 体验类问题
- 集中清理边界场景、UI 细节
- **状态**: ⏳ 待实施

## 执行规范

1. **分步实施**: 禁止批量修改,每个 Step 独立提交
2. **PowerShell 验证**: 涉及 PowerShell 命令必须先用 PowerShell 专家技能验证
3. **文档同步**: 完成后更新相关技术文档
4. **代码审查**: 每个 Step 完成后调用 code-review 技能
