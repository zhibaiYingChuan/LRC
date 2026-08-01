# HCSE FMEA 形式化矩阵 — LRC v0.8.23 韧性验证

> 生成时间: 2026-08-01
> 范围: v0.8.23 修复 P2-01(E4)/P2-02(D6)/P2-03/OBS-01/A-02 + 回归验证
> 方法: 失败模式与影响分析（FMEA）+ 当前屏障 + HCSE 策略
> 范式: 运行时验证优先（CDP WebSocket 实时断言）

## 评分标准

| 维度 | 范围 | 含义 |
|------|------|------|
| 严重度 (S) | 1-10 | 对业务的影响（10=数据丢失/死锁） |
| 发生度 (O) | 1-10 | 生产环境出现概率（10=几乎必然） |
| 可检测度 (D) | 1-10 | CDP 捕获难度（10=极难检测） |
| RPN | S×O×D | 风险优先数（>200 需强制缓解） |

## FMEA 矩阵

| 编号 | 失败模式 | S | O | D | RPN | 当前屏障 | 推荐 HCSE 策略 | 对应不变式 | 状态 |
|------|---------|---|---|---|-----|---------|--------------|-----------|------|
| FM-E4 | sidecar 不可达时用户不知代理问题，反复重试 | 5 | 5 | 3 | 75 | detectProxyConfiguration() 异步检测 + _detectProxyAndUpdateBanner 更新 banner 文案 | 用户告知（代理信息显示在 banner） | INV-V0823-P201 | 已实现 |
| FM-D6 | 向导输入框 Enter 键无响应，用户需手动点击按钮 | 4 | 6 | 2 | 48 | 3 个输入框绑定 keydown Enter → preventDefault + 对应函数调用 | 直接导航（Enter 触发操作） | INV-V0823-P202 | 已实现 |
| FM-502 | 502 Bad Gateway 仅显示 toast，无自动重试 | 6 | 5 | 3 | 90 | handleHttpError 502/504 分支：自动重试 3 次，指数退避（1s/2s/4s），首次 toast 提示 | 自动恢复（指数退避重试） | INV-V0823-P203 | 已实现 |
| FM-504 | 504 Gateway Timeout 仅显示 toast，无自动重试 | 6 | 4 | 3 | 72 | 同 502 分支，自动重试 3 次，指数退避 | 自动恢复（指数退避重试） | INV-V0823-P203 | 已实现 |
| FM-OBS | 快速切换标签页时信任中心旧请求未取消，产生竞态 | 5 | 6 | 4 | 120 | trustAbortController 每次调用前 abort 上一次请求；AbortError 静默处理 | 竞态防护（AbortController 取消旧请求） | INV-V0823-OBS01 | 已实现 |
| FM-A02-500 | 500 错误退避延迟在标签页切换后仍继续执行 | 5 | 5 | 4 | 100 | retryContext.signal 传递到 handleHttpError；500 退避监听 signal.abort | 信号传播（signal 驱动退避取消） | INV-V0823-A02 | 已实现 |
| FM-A02-502 | 502/504 退避延迟在标签页切换后仍继续执行 | 5 | 5 | 4 | 100 | 同 A-02，502/504 退避监听 signal.abort | 信号传播（signal 驱动退避取消） | INV-V0823-A02 | 已实现 |
| FM-503-回归 | 503 lock_busy 30s 冷却期回归失效 | 7 | 3 | 2 | 42 | now - lastToastTime > 30000 判断逻辑 | 优雅降级（节流提示） | INV-V0823-REGR-02 | 已回归确认 |
| FM-DAO-回归 | loadDaoMetrics AbortController 回归失效 | 7 | 3 | 3 | 63 | daoAbortController 入口 abort 逻辑 | 竞态防护（AbortController） | INV-V0823-REGR-03 | 已回归确认 |
| FM-GLOBAL-回归 | 全局错误处理回归失效 | 7 | 2 | 2 | 28 | _lrcGlobalErrorRegistered 标志 + onerror/unhandledrejection 监听 | 防御纵深（全局错误兜底） | INV-V0823-REGR-04 | 已回归确认 |
| FM-503CANCEL-回归 | 503 仍自动重试，双重重试回归 | 6 | 3 | 2 | 36 | 503 返回 { action: 'cancel' } | Fail-fast（单层重试） | INV-V0823-REGR-06 | 已回归确认 |
| FM-COUNT-回归 | pendingRequestCount 计数器泄漏回归 | 5 | 3 | 4 | 60 | finally 块统一管理计数器生命周期 | 资源完整性（单一减少点） | INV-V0823-REGR-07 | 已回归确认 |

## 组合故障分析（Phase 4 状态爆破维度）

| 组合编号 | 网络层 | 时序层 | 异常叠加 | 覆盖状态 | 说明 |
|---------|--------|--------|---------|---------|------|
| C-01 | 502 + 正常 | 空闲态 | 无 | 已覆盖 | FM-502 自动重试，CDP evaluate handler 验证 |
| C-02 | 504 + 正常 | 空闲态 | 无 | 已覆盖 | FM-504 自动重试，CDP evaluate handler 验证 |
| C-03 | 503 lock_busy | 信任中心加载中 | 标签页快速切换 | 已覆盖 | FM-OBS trustAbortController 取消旧请求 |
| C-04 | 500 错误 | 退避延迟中 | 标签页切换 | 已覆盖 | FM-A02-500 signal 取消退避 |
| C-05 | 502 错误 | 退避延迟中 | 标签页切换 | 已覆盖 | FM-A02-502 signal 取消退避 |
| C-06 | 不可达 | 空闲态 | 代理检测 | 已覆盖 | FM-E4 detectProxyConfiguration 调用 |
| C-07 | Enter 键 | 向导输入框聚焦 | 无 | 已覆盖 | FM-D6 Enter 键触发对应操作 |
| C-08 | 503 × 5 | 30s 窗口内 | toast 风暴 | 回归覆盖 | FM-503-回归 冷却期验证 |

## 等价划分（处理状态爆炸）

当组合超过 1000 时，按以下维度降维：
1. **网络层等价类**: {200, 4xx, 502, 504, 503, 超时, 不可达} → 7 类
2. **时序等价类**: {空闲, 加载中, 退避中, 切换中} → 4 类
3. **异常叠加等价类**: {无, 标签页切换, 快速连续触发, 代理检测} → 4 类

实际覆盖: 8/8 组合（全部已覆盖，无 CDP 限制豁免项）

## 结论

- **高 RPN（>100）失败模式**: FM-OBS(120), FM-A02-500(100), FM-A02-502(100) — 均为 P2 级，已通过 AbortController + signal 传播缓解
- **运行时验证方法**: 所有 12 个失败模式均可通过 CDP Runtime.evaluate 直接验证
- **回归风险**: 7 个回归验证项全部为 v0.8.22 已修复点，需确认未回归
- **CDP 可检测性**: 12/12 失败模式可被 CDP 直接捕获（D<=4）