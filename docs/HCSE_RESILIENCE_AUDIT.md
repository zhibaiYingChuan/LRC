# HCSE 交互韧性审计清单

> 本文档是 hcse-resilience-validator 智能体的项目级检查清单。
> 全局框架见 `~/.trae-cn/user_rules/hcse-framework.md`。
> 核心原则：**不只测功能正确性，必须测异常路径韧性**。

---

## 一、调用智能体时的标准 query 模板

调用 `hcse-resilience-validator` 智能体时，必须按以下结构组织 query：

```
请对桌面端应用执行 HCSE 交互韧性审计。

## 审计范围
- 目标版本：<版本号>
- 审计层级：L1 一级页面 / L2 二级弹窗 / L3 三级卡片 / L4 四级嵌套
- 重点场景：<如：启动服务模态框、状态栏点击、项目切换等>

## 异常路径要求
对每个场景，必须覆盖以下异常路径（不仅测正常路径）：
1. 超时路径：操作长时间无响应时 UI 是否有兜底反馈
2. 卡死路径：底层 invoke 永不返回时 UI 是否能恢复
3. 错误路径：操作失败时是否有明确错误提示 + 状态恢复
4. 取消路径：用户取消操作时是否能正确中断 + 清理

## 超时机制验证
对所有 Tauri invoke 调用，验证：
- 前端是否有 setTimeout 硬超时（不只是 AbortController）
- 超时是否真正触发 reject（而非永久 pending）
- 超时后 UI 状态是否恢复（按钮可重新点击）

## 项目审计清单
参考 docs/HCSE_RESILIENCE_AUDIT.md 的场景清单和已知 bug
```

---

## 二、五层交互韧性审计模型

### L1 一级页面：仪表盘（dashboard）

| 场景 | 正常路径 | 异常路径 | 验证状态 |
|------|---------|---------|---------|
| 页面加载 | 数据正常显示 | sidecar 未启动时加载失败 → 是否有重试/提示 | ✓ v0.8.2 |
| 状态栏显示 | 绿色"运行中" | 红色"不可达"时点击 → 是否弹出启动弹窗 | ✓ v0.6.0 |
| 数据目录点击 | 打开文件夹 | 文件夹不存在 → 是否有错误提示 | ✓ v0.6.0 |
| 版本号显示 | 正常显示 | sys-version 动态填充失败 → 是否有兜底 | ✓ v0.8.6 |

### L2 二级弹窗：启动服务模态框

| 场景 | 正常路径 | 异常路径 | 验证状态 |
|------|---------|---------|---------|
| 打开模态框 | 正常显示 | modal 元素不存在/CSS display 未生效 → 是否有 toast | ✓ v0.8.4 |
| 点击启动 | 60s 内启动成功 | **invoke 永不返回 → UI 是否有硬超时兜底** | ✓ v0.8.9 修复（postMessageToParent Tauri 分支添加 setTimeout + Promise.race） |
| 启动失败 | 显示错误 toast | 错误信息是否人性化（端口占用/超时分类） | ✓ v0.5.4 + v0.8.9 G-002 端口冲突友好提示 |
| 用户取消 | abort 中断 invoke | **abort 后 invoke 是否真正中断？UI 按钮是否恢复** | ✓ v0.8.9 修复（G-001 cancel_start_sidecar 命令 + AtomicBool 标志 + 健康检查循环检测取消） |
| Tab 焦点陷阱 | Tab 循环 | Shift+Tab 反向循环 → 是否正确 | ✓ v0.8.4 |
| 遮罩点击关闭 | 关闭模态框 | 启动进行中点击遮罩 → 是否触发 abort | ✓ v0.8.6 |

### L3 三级卡片：弹窗内卡片/折叠面板

| 场景 | 正常路径 | 异常路径 | 验证状态 |
|------|---------|---------|---------|
| API 配置卡片 | 保存成功 | API 地址不可达 → 是否有超时+错误分类 | ✓ v0.5.4 |
| 项目切换卡片 | 切换成功 | 目标目录不存在 → 是否有错误提示 | ⚠ 需验证 |
| Agent 配置卡片 | 配置保存 | agent 路径无效 → 是否有验证 | ⚠ 需验证 |

### L4 四级嵌套：卡片内嵌套操作

| 场景 | 正常路径 | 异常路径 | 验证状态 |
|------|---------|---------|---------|
| API 测试按钮 | 返回成功 | API 超时 → 是否有 10s 硬超时 + 错误提示 | ✓ v0.5.4 |
| 模型下载按钮 | 下载成功 | 网络中断 → 是否有重试 + 回退 | ✓ v0.5.x |
| 导入/导出按钮 | 操作成功 | 文件损坏 → 是否有验证 + 错误提示 | ⚠ 需验证 |

### L5 异常全局：跨层级异常

| 场景 | 正常路径 | 异常路径 | 验证状态 |
|------|---------|---------|---------|
| 网络断开 | 所有请求正常 | sidecar 崩溃后请求 → 是否有全局错误处理 | ⚠ 需验证 |
| 进程崩溃 | sidecar 正常 | sidecar 崩溃 → UI 是否检测到并提示重启 | ⚠ 需验证 |
| 端口冲突 | 端口空闲 | 端口被占用 → 启动时是否检测并提示 | ✓ v0.5.4 + v0.8.9 G-002 端口预检+复用 |
| 多窗口 | 单窗口 | 多窗口同时操作 sidecar → 是否有锁保护 | ✓ v0.5.x |

---

## 三、超时机制验证清单

> **核心教训（v0.8.9）**：代码中有超时常量 ≠ 超时真正生效。
> postMessageToParent 的 Tauri 分支有 timeoutMs 参数但完全未使用，导致 UI 卡死 10 分钟。

| 调用点 | 超时机制 | 是否真正触发 reject | UI 状态是否恢复 | 验证状态 |
|--------|---------|-------------------|----------------|---------|
| start_sidecar (Tauri invoke) | timeoutMs=60000 | ✓ 触发（v0.8.9 修复） | ✓ 恢复 | ✓ v0.8.9 修复 |
| start_sidecar (iframe postMessage) | setTimeout 60000 | ✓ 触发 | ✓ 恢复 | ✓ v0.8.6 |
| open_data_dir (Tauri invoke) | timeoutMs=10000 | ✓ 触发（v0.8.9 修复） | ✓ 恢复 | ✓ v0.8.9 修复 |
| API 测试 (Tauri invoke) | 后端 reqwest 10s | ✓ 后端超时 | ⚠ 前端需验证 | ⚠ 需验证 |
| 模型下载 | 后端指数退避 | ✓ 后端处理 | ✓ 前端进度 | ✓ v0.5.x |
| cancel_start_sidecar (Tauri invoke) | AtomicBool 标志检测 | ✓ 后端健康检查循环检测取消标志 | ✓ 前端 abort + 后端 kill 子进程 | ✓ v0.8.9 G-001 |

### 超时机制验证方法

对每个 Tauri invoke 调用：

1. **代码审查**：前端是否有 `setTimeout` + `Promise.race` 硬超时（不只是 AbortController）
2. **运行时验证**：模拟后端永不返回（如 kill sidecar 进程），观察前端是否在超时后 reject
3. **UI 状态验证**：超时后按钮是否恢复可点击、模态框是否可关闭、是否有错误提示

---

## 四、已知 bug 与待验证项

### 已知 bug（v0.8.9 修复）

**postMessageToParent Tauri 分支缺少 setTimeout 超时** — ✓ 已修复

- 位置：`static/app.js`（Tauri 环境分支）
- 根因：`timeoutMs` 参数在 Tauri 分支被完全忽略，只有 iframe 模式才有 `setTimeout`
- 影响：所有 Tauri invoke 调用（start_sidecar/open_data_dir 等）如果后端永不返回，UI 永久卡死
- 修复：Tauri 分支添加 `setTimeout` + `Promise.race`，与 iframe 模式一致

**G-001 假取消（前端 abort 后后端继续执行）** — ✓ 已修复

- 根因：前端 AbortController 仅中断前端 Promise，后端 `spawn_and_wait` 的健康检查循环无取消机制
- 修复：新增 `cancel_start_sidecar` IPC 命令 + `AtomicBool` 标志，健康检查循环每次迭代检测取消标志
- 清理：取消时显式 `child.kill()` + `child.wait()`，防止孤儿进程

**G-002/G-009 孤儿进程（桌面端崩溃后重启导致重复 sidecar）** — ✓ 已修复

- 根因：桌面端崩溃后重启，旧 sidecar 仍在端口上运行，新 spawn 的 sidecar 绑定到其他端口
- 修复：`spawn_and_wait` 中添加 200ms 超时的端口预检；`start_sidecar`/`start_sidecar_for_project` 中添加 Phase 1.5 端口冲突检测，复用已有 sidecar

**G-003 启动期间无进度反馈** — ✓ 已修复

- 根因：sidecar 启动最多 40 秒，期间前端无任何可见性，用户以为卡死
- 修复：`spawn_and_wait` 在 4 个阶段发送 `StartProgress` 事件（port_check/spawn/health_check/ready）
- 通道：命令层创建 `mpsc` channel + `tokio::spawn` 转发任务，通过 `app.emit("sidecar-start-progress", ...)` 推送到前端
- 前端接入：`listen('sidecar-start-progress', cb)` 接收 `{stage, progress, message}` 结构

**G-004 错误为字符串，无结构化分类** — ✓ 已修复

- 根因：`spawn_and_wait` 返回 `Result<_, String>`，`user_friendly_error` 依赖字符串 pattern matching，措辞变化会导致漏匹配
- 修复：新增 `SidecarStartError` 枚举（7 变体 + 错误码 E001-E007），`spawn_and_wait` 返回类型改为 `Result<_, SidecarStartError>`
- 类型安全：`sidecar_error_to_user_message` 直接匹配枚举变体，不依赖字符串匹配
- 前端可扩展：`SidecarStartError` 实现 `serde::Serialize`，未来可直接返回结构化错误给前端

### 待验证项

- [x] ~~abort 后 Tauri invoke 是否真正中断~~ → ✓ v0.8.9 G-001 修复（cancel_start_sidecar + AtomicBool）
- [x] ~~启动期间无进度反馈~~ → ✓ v0.8.9 G-003 修复（StartProgress + Tauri event）
- [x] ~~错误无结构化分类~~ → ✓ v0.8.9 G-004 修复（SidecarStartError 枚举 + 错误码）
- [x] ~~sidecar 崩溃后 UI 是否能检测到~~ → ✓ v0.8.10 L5-01 修复（新增 sidecar-crash 事件监听器）
- [x] ~~前端接入 sidecar-start-progress 事件监听器~~ → ✓ v0.8.9 已接入，v0.8.10 扩展至 sidecar-detected/recovered/crash
- [x] ~~_setReachable 仅刷新仪表盘导致其他页面状态不同步~~ → ✓ v0.8.10 L4-02 修复（_broadcastSidecarStateChange 全局广播）
- [x] ~~startSidecarForProject/switchProject 超时 60s 不足~~ → ✓ v0.8.10 L3-01/L4-01 修复（统一 120s）
- [x] ~~child.kill() 错误静默吞掉~~ → ✓ v0.8.10 L5-03 修复（tracing 日志记录）
- [ ] 多窗口同时启动 sidecar 的竞态条件
- [ ] 项目切换时旧 sidecar 是否正确停止

---

## 五、审计执行流程

### 5.1 自动化审计（CDP 协议测试）

使用 `cdp-protocol-tester` 或 `cdp-robust-tester` 智能体：

1. 启动桌面端应用
2. 拦截 Tauri invoke 响应（模拟超时/不返回）
3. 验证 UI 是否在预期时间内有兜底反馈
4. 验证 UI 状态是否可恢复

### 5.2 人工审计

按五层模型逐层验证，重点验证异常路径（超时/卡死/错误/取消）。

### 5.3 审计报告格式

```
## 审计结果
- 审计版本：v0.8.x
- 审计层级：L1-L5
- 通过项：X
- 失败项：Y
- 待验证项：Z

## 失败项详情
| 场景 | 异常路径 | 预期行为 | 实际行为 | 严重级别 |
|------|---------|---------|---------|---------|
```

---

## 六、历史交互 bug 案例

| 版本 | bug | 根因 | 修复 | 审计项更新 |
|------|-----|------|------|-----------|
| v0.8.2 | API 按钮在服务未启动时无反馈 | 未禁用按钮 | 添加 btn-disabled + title | L1 状态栏 |
| v0.8.3 | 模态框打开失败无提示 | CSS display 问题 | removeAttribute + offsetHeight 强制重排 | L2 打开模态框 |
| v0.8.4 | alert 阻塞 JS 线程 | Tauri WebView alert 行为 | 替换为 showToast | L2 启动失败 |
| v0.8.6 | 取消按钮不中断 invoke | 无 AbortController | Promise.race + abort | L2 用户取消 |
| v0.8.9 | 启动服务 10 分钟无响应 | Tauri 分支无 setTimeout 超时 | 待修复 | L2 点击启动（超时路径） |
