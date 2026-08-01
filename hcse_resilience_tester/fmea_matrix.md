# FMEA 正式矩阵 — LRC Desktop v0.8.20 韧性验证

> 生成时间: 2026-07-31 | HCSE 范式 | 基于 git diff 动态分析 + 代码静态扫描

## 评分标准

| 维度 | 范围 | 含义 |
|------|------|------|
| 严重度 (S) | 1-10 | 10=数据丢失/死锁, 7=功能不可用, 4=体验受损, 1=无影响 |
| 发生度 (O) | 1-10 | 10=必现, 7=高频, 4=偶发, 1=极罕见 |
| 探测度 (D) | 1-10 | 10=CDP 无法捕获, 7=需复杂注入, 4=简单注入可见, 1=日志即可 |
| RPN | S×O×D | ≥200 需立即修复, ≥100 需排期, <100 可接受 |

## 失败模式矩阵

### FM-01: /v1/health/detailed 持续超时 10s（已确认 P0）

| 维度 | 值 | 说明 |
|------|-----|------|
| 失败模式 | /v1/health/detailed 端点使用 `store.lock().await` 而非 `try_lock`，后台合成持锁期间请求阻塞 10s 超时 |
| 代码位置 | [src/v1_api.rs:692](file:///g:/code-memory/src/v1_api.rs#L692) |
| 严重度 S | 9 | 仪表盘加载链路核心端点，超时导致整个 loadDashboard 卡顿 |
| 发生度 O | 9 | 后台合成周期 300s，每次持锁 100-200ms，但 detailed 端点请求会排队等待 |
| 探测度 D | 3 | CDP Network.responseReceived 可直接捕获 timing.waitingTime |
| RPN | **243** | ≥200 立即修复 |
| 现有屏障 | 前端 loadDashboard 使用 Promise.allSettled + 10s 超时（[app.js:682-780](file:///g:/code-memory/static/app.js#L682)） |
| 屏障缺陷 | detailed 端点本身无服务端超时；前端超时后仅显示降级，不阻止后端继续阻塞 Tokio worker |
| HCSE 策略 | **Fail-fast + Bulkhead**：服务端改用 try_lock，503 时返回降级数据；前端 8s 超时单独控制 |
| 关联不变式 | INV-LOCK-001, INV-TIMEOUT-004 |

### FM-02: 503 lock_busy 被误报为"LRC 服务未启动"（已确认 P0）

| 维度 | 值 | 说明 |
|------|-----|------|
| 失败模式 | loadDaoMetrics 在 catch 中将 `SidecarUnreachableError` 直接映射为"LRC 服务未启动"，但 503 lock_busy 实际是服务在线但繁忙 |
| 代码位置 | [static/app.js:5261-5262](file:///g:/code-memory/static/app.js#L5261) |
| 严重度 S | 8 | 用户看到"服务未启动"会误以为需要重启，实际服务正常运行 |
| 发生度 O | 8 | 后台合成期间每次 dao_metrics 请求都可能触发 |
| 探测度 D | 4 | CDP DOM 查询 .dao-fallback-banner 文本即可检测 |
| RPN | **256** | ≥200 立即修复 |
| 现有屏障 | handleHttpError 有 503 专用分支（[app.js:276-292](file:///g:/code-memory/static/app.js#L276)），1 次自动重试 |
| 屏障缺陷 | loadDaoMetrics 未使用 handleHttpError，直接用 fetchWithTimeout；fetchWithTimeout 在 503 时不抛错但 data.ok=false 走"数据格式异常"分支；若 fetch 网络层失败则误判为"服务未启动" |
| HCSE 策略 | **状态区分**：前端应检查 response.status === 503 + body.error === "lock_busy"，显示"后台合成中" |
| 关联不变式 | INV-STATE-002 |

### FM-03: 仪表盘数据始终为空（已确认 P0）

| 维度 | 值 | 说明 |
|------|-----|------|
| 失败模式 | /v1/health/system 持续返回 503 lock_busy，loadDashboard 无法获取系统统计数据，仪表盘 40s 后仍为 "--" |
| 代码位置 | [static/app.js:682-780](file:///g:/code-memory/static/app.js#L682) loadDashboard；[src/v1_api.rs:651-684](file:///g:/code-memory/src/v1_api.rs#L651) /v1/health/system |
| 严重度 S | 8 | 仪表盘核心功能完全不可用 |
| 发生度 O | 7 | 取决于后台合成频率，合成期间必现 |
| 探测度 D | 4 | CDP DOM 查询统计卡片文本 |
| RPN | **224** | ≥200 立即修复 |
| 现有屏障 | /v1/health/system 已用 try_lock（v0.8.19 修复）；前端 loadDashboard 有 503 lock_busy 检测（[app.js:738](file:///g:/code-memory/static/app.js#L738)） |
| 屏障缺陷 | 503 检测仅在 Promise.allSettled 结果中，但 system 端点 503 时 loadDashboard 抛 LOCK_BUSY 进入 catch，显示"后台合成中"而非降级数据；用户仍看不到任何数据 |
| HCSE 策略 | **Graceful Degradation**：503 时应显示上次缓存的统计数据 + "数据刷新中"提示，而非清空 |
| 关联不变式 | INV-STATE-002, INV-LOCK-001 |

### FM-04: 后台合成持锁导致 /v1/health/detailed 阻塞 Tokio worker

| 维度 | 值 | 说明 |
|------|-----|------|
| 失败模式 | consolidation.rs 合成流水线持有 store.lock().await 期间，/v1/health/detailed 的 lock().await 排队等待，占用 Tokio worker 线程 |
| 代码位置 | [src/consolidation.rs:291,357,419,534](file:///g:/code-memory/src/consolidation.rs#L291)；[src/v1_api.rs:692](file:///g:/code-memory/src/v1_api.rs#L692) |
| 严重度 S | 7 | 多个 detailed 请求并发可耗尽 Tokio worker 线程池，影响其他端点 |
| 发生度 O | 6 | 需要前端多次刷新 + 后台合成同时运行 |
| 探测度 D | 5 | CDP 可观察，但需多请求并发注入才能复现 |
| RPN | **210** | ≥200 立即修复 |
| 现有屏障 | Tokio 默认 256 worker 线程；合成周期 300s 持锁 100-200ms |
| 屏障缺陷 | 无端点级超时；无 worker 线程耗尽告警 |
| HCSE 策略 | **Bulkhead 隔离**：健康检查端点用 try_lock；合成流水线用独立 Tokio runtime |
| 关联不变式 | INV-LOCK-001 |

### FM-05: sidecar 启动超时 10s 后未清理子进程（潜在）

| 维度 | 值 | 说明 |
|------|-----|------|
| 失败模式 | wait_for_health_static 超时返回 HealthCheckTimeout，但若 kill+wait 失败可能残留孤儿进程 |
| 代码位置 | [desktop/src-tauri/src/sidecar_manager.rs:733-755](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L733) |
| 严重度 S | 6 | 孤儿进程占用端口，下次启动端口冲突 |
| 发生度 O | 4 | 仅在 kill 失败时发生（权限不足/进程僵死） |
| 探测度 D | 6 | 需 ps 查询进程列表，CDP 无法直接探测 |
| RPN | **144** | ≥100 排期修复 |
| 现有屏障 | spawn_and_wait 健康检查失败时显式 kill+wait（[sidecar_manager.rs:743-752](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L743)）；Drop impl 3s 超时 wait |
| 屏障缺陷 | kill 失败仅 log error，无重试；Drop 3s 超时后 break 可能残留僵尸 |
| HCSE 策略 | **Fail-fast + 重试**：kill 失败时重试 3 次，仍失败则记录 PID 供下次启动清理 |
| 关联不变式 | INV-PROC-003, INV-CANCEL-005 |

### FM-06: 取消 sidecar 启动后 cancel_flag 未在所有入口重置

| 维度 | 值 | 说明 |
|------|-----|------|
| 失败模式 | 用户取消启动后 cancel_flag=true，若新启动入口未重置则永久失效 |
| 代码位置 | [desktop/src-tauri/src/commands.rs:472-473](file:///g:/code-memory/desktop/src-tauri/src/commands.rs#L472) start_sidecar（已重置）；[commands.rs:590-591](file:///g:/code-memory/desktop/src-tauri/src/commands.rs#L590) start_sidecar_for_project（已重置）；[commands.rs:1462-1464](file:///g:/code-memory/desktop/src-tauri/src/commands.rs#L1462) switch_project（v0.8.14 修复） |
| 严重度 S | 7 | 用户取消后无法再次启动 sidecar |
| 发生度 O | 3 | v0.8.14 已修复 switch_project，start_sidecar 已重置，仅未来新增入口可能遗漏 |
| 探测度 D | 5 | 需 CDP 触发取消后再次启动 |
| RPN | **105** | ≥100 排期修复 |
| 现有屏障 | 3 个入口均已重置；cancel_start_sidecar 仅设 true |
| 屏障缺陷 | 无集中式入口校验；未来新增启动入口可能遗漏 |
| HCSE 策略 | **集中式守卫**：将 cancel_flag 重置提取到 spawn_and_wait 入口 |
| 关联不变式 | INV-CANCEL-005 |

### FM-07: 配置向导未显示（wizard.json 不存在时）

| 维度 | 值 | 说明 |
|------|-----|------|
| 失败模式 | wizard.json 不存在 → setup_complete=false，但前端直接显示仪表盘未显示配置向导，用户无法完成首次配置 |
| 代码位置 | [desktop/src-tauri/src/commands.rs:1302-1378](file:///g:/code-memory/desktop/src-tauri/src/commands.rs#L1302) get_wizard_state；前端初始化逻辑 |
| 严重度 S | 6 | 首次用户无法配置 LLM，llm_configured=false 永久 |
| 发生度 O | 5 | 首次安装必现，但仅一次 |
| 探测度 D | 4 | CDP DOM 查询向导元素可见性 |
| RPN | **120** | ≥100 排期修复 |
| 现有屏障 | get_wizard_state 返回 setup_complete=false；前端应据此显示向导 |
| 屏障缺陷 | 前端初始化逻辑未正确响应 setup_complete=false，直接显示仪表盘 |
| HCSE 策略 | **状态驱动渲染**：前端必须根据 setup_complete 决定显示向导还是仪表盘 |
| 关联不变式 | INV-STATE-002 |

### FM-08: WebView2 渲染卡死无检测机制

| 维度 | 值 | 说明 |
|------|-----|------|
| 失败模式 | WebView2 主线程被长任务阻塞（如大量 DOM 操作），前端无响应，但 sidecar 正常 |
| 代码位置 | 前端无渲染卡死检测；SidecarHealthMonitor 仅检测 sidecar 可达性 |
| 严重度 S | 7 | 用户看到"未响应"，无法操作 |
| 发生度 O | 3 | 仅在极端 DOM 操作时发生 |
| 探测度 D | 8 | CDP 本身依赖 WebView2 主线程，卡死时 CDP 也无响应 |
| RPN | **168** | ≥100 排期修复 |
| 现有屏障 | 无 |
| 屏障缺陷 | 无 heartbeat 机制检测前端主线程活性 |
| HCSE 策略 | **Watchdog**：Tauri 主进程定期 eval('1+1')，超时 5s 判定渲染卡死，重启 WebView |
| 关联不变式 | INV-PROC-003 |
| 盲点说明 | CDP 无法捕获 WebView2 内核级卡死，需 eBPF/Wireshark 辅助 |

### FM-09: 多个 API 并发请求时 Tokio worker 线程耗尽

| 维度 | 值 | 说明 |
|------|-----|------|
| 失败模式 | 前端 Promise.allSettled 并发请求 system/detailed/dao_metrics，若 detailed 阻塞 10s，3 个 worker 被占用；多窗口并发可耗尽 256 worker |
| 代码位置 | [static/app.js:682-780](file:///g:/code-memory/static/app.js#L682) loadDashboard 并发请求 |
| 严重度 S | 6 | 其他端点响应变慢 |
| 发生度 O | 4 | 需多窗口 + 合成同时运行 |
| 探测度 D | 6 | 需并发注入 + 资源监控 |
| RPN | **144** | ≥100 排期修复 |
| 现有屏障 | Tokio 256 worker；/health 用 try_lock |
| 屏障缺陷 | detailed 端点 lock().await 不释放 worker |
| HCSE 策略 | **Bulkhead**：detailed 改 try_lock；前端并发请求限制 3 个 |
| 关联不变式 | INV-LOCK-001, INV-RESOURCE-007 |

### FM-10: sidecar 崩溃后心跳恢复失败但前端无感知

| 维度 | 值 | 说明 |
|------|-----|------|
| 失败模式 | recover_dead_instances 重启失败时仅 log error，前端 SidecarHealthMonitor 10s 后才检测到不可达 |
| 代码位置 | [desktop/src-tauri/src/sidecar_manager.rs:1301-1304](file:///g:/code-memory/desktop/src-tauri/src/sidecar_manager.rs#L1301) |
| 严重度 S | 6 | 用户 10s 内看到"运行中"但实际已崩溃 |
| 发生度 O | 4 | 崩溃恢复失败概率低 |
| 探测度 D | 5 | 需注入 sidecar 崩溃 + 端口占用 |
| RPN | **120** | ≥100 排期修复 |
| 现有屏障 | 心跳 10s 轮询；连续 2 次失败才判定不可达 |
| 屏障缺陷 | 恢复失败无事件通知前端；心跳间隔 10s 太长 |
| HCSE 策略 | **事件驱动**：recover_dead_instances 失败时 emit 事件，前端立即显示错误 |
| 关联不变式 | INV-PROC-003 |

## RPN 排序与修复优先级

| 排名 | 失败模式 | RPN | 优先级 | 关联不变式 |
|------|----------|-----|--------|------------|
| 1 | FM-02 503 误报为"服务未启动" | 256 | P0 立即 | INV-STATE-002 |
| 2 | FM-01 /v1/health/detailed 超时 10s | 243 | P0 立即 | INV-LOCK-001 |
| 3 | FM-03 仪表盘数据始终为空 | 224 | P0 立即 | INV-STATE-002 |
| 4 | FM-04 合成持锁阻塞 worker | 210 | P0 立即 | INV-LOCK-001 |
| 5 | FM-08 WebView2 渲染卡死无检测 | 168 | P1 排期 | INV-PROC-003 |
| 6 | FM-05 启动超时未清理子进程 | 144 | P1 排期 | INV-PROC-003 |
| 7 | FM-09 并发请求 worker 耗尽 | 144 | P1 排期 | INV-RESOURCE-007 |
| 8 | FM-07 配置向导未显示 | 120 | P1 排期 | INV-STATE-002 |
| 9 | FM-10 崩溃恢复失败前端无感知 | 120 | P1 排期 | INV-PROC-003 |
| 10 | FM-06 cancel_flag 未集中重置 | 105 | P1 排期 | INV-CANCEL-005 |

## 异常路径覆盖矩阵

| 失败模式 | 超时路径 | 卡死路径 | 错误路径 | 取消路径 | 竞态路径 |
|----------|----------|----------|----------|----------|----------|
| FM-01 | ✓ | ✓ | - | - | ✓ |
| FM-02 | - | - | ✓ | - | - |
| FM-03 | ✓ | - | ✓ | - | - |
| FM-04 | ✓ | ✓ | - | - | ✓ |
| FM-05 | ✓ | - | - | ✓ | - |
| FM-06 | - | - | - | ✓ | - |
| FM-07 | - | - | ✓ | - | - |
| FM-08 | - | ✓ | - | - | - |
| FM-09 | - | ✓ | - | - | ✓ |
| FM-10 | - | - | ✓ | - | - |

**覆盖统计**: 超时路径 4/10, 卡死路径 4/10, 错误路径 4/10, 取消路径 2/10, 竞态路径 3/10
