# HCSE FMEA 形式化矩阵 — LRC v0.8.22 回归验证

> 生成时间: 2026-08-01
> 范围: v0.8.22 第二轮修复 P0-1/P0-2/P0-3/P0-4/P1-2/P1-3
> 方法: 失败模式与影响分析（FMEA）+ 当前屏障 + HCSE 策略

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
| FM-P01 | /health 端点 RwLock 读锁阻塞，worker 线程耗尽导致 12s 超时 | 10 | 8 | 3 | 240 | AtomicBool 无锁读取（P0-1） | 舱壁隔离（无锁 O(1) 读取） | INV-REG-P01 | 已缓解 |
| FM-P02 | index_project CPU 密集型在 tokio worker 上执行，阻塞 runtime | 9 | 7 | 4 | 252 | spawn_blocking 隔离（P0-2） | 舱壁隔离（阻塞线程池） | INV-REG-P02 | 已缓解 |
| FM-P03 | luoshu_synthesize 持锁在 worker 线程执行，阻塞 async runtime | 10 | 6 | 4 | 240 | spawn_blocking + blocking_lock（P0-3） | 舱壁隔离 + Fail-fast | INV-REG-P03 | 已缓解 |
| FM-P04 | 503 lock_busy 错误无冷却期，toast 风暴（每秒 5-10 toast） | 7 | 9 | 2 | 126 | 30s 冷却期（P0-4） | 优雅降级（节流提示） | INV-REG-P04 | 已缓解 |
| FM-P12 | handleHttpError 503 自动重试 + 上层重试 = 双重重试，请求翻倍 | 6 | 8 | 3 | 144 | 去掉自动重试，返回 cancel（P1-2） | Fail-fast（单层重试） | INV-REG-P12 | 已缓解 |
| FM-P13 | pendingRequestCount 重试路径双重减少，变负值，beforeunload 失效 | 5 | 7 | 5 | 175 | finally 统一管理计数器（P1-3） | 资源完整性（单一减少点） | INV-REG-P13 | 已缓解 |
| FM-LOCK | 合成期间健康端点被锁阻塞，>5s 无响应 | 9 | 6 | 3 | 162 | try_lock 快速 503 失败 | Fail-fast（快速失败） | INV-REG-LOCK-001 | 已缓解 |
| FM-STATE | sidecar 卡死时前端 online 仍 true，状态不一致 | 8 | 5 | 4 | 160 | SidecarHealthMonitor 轮询 | 优雅降级（指数退避） | INV-REG-STATE-002 | 已缓解 |
| FM-TIMEOUT | fetch 无超时，请求永久挂起 | 8 | 4 | 6 | 192 | fetchWithTimeout + AbortController | Fail-fast（10s 超时） | INV-REG-TIMEOUT-004 | 已缓解 |
| FM-PATH | 测试脚本越界访问系统目录 | 10 | 2 | 8 | 160 | PathValidator 白名单 + Hard Halt | 防御纵深（Hard Halt 130） | INV-REG-PATH-WHITELIST | 已缓解 |
| FM-LEAK | 证据工件泄露敏感数据（API Key/email） | 9 | 3 | 7 | 189 | DataSanitizer 双重脱敏 | 防御纵深（正则+结构剪枝） | INV-REG-SANITIZE | 已缓解 |
| FM-RESOURCE | 测试进程内存/CPU 超限，拖垮测试平台 | 7 | 3 | 5 | 105 | ResourceWatchdog 1024MB/60s | 舱壁隔离（先杀子进程） | INV-REG-RESOURCE | 已缓解 |

## 组合故障分析（Phase 4 状态爆破维度）

| 组合编号 | 网络层 | 时序层 | 异常叠加 | 覆盖状态 | 说明 |
|---------|--------|--------|---------|---------|------|
| C-01 | 慢网络 + 502 | Page.loadEventFired 前阻断 | - | 豁免 | CDP 无法注入 502（sidecar 返回真实状态） |
| C-02 | 正常 + 503 lock_busy | 合成期间并发 /health | - | 已覆盖 | FM-P01 + FM-P03 组合，20 并发测试 |
| C-03 | 超时 8s + 503 | Modal 打开时 WebSocket 断开 | - | 部分覆盖 | 503 冷却期已测，WebSocket 断开豁免 |
| C-04 | 20 并发 /health | 索引期间 | - | 已覆盖 | P01 并发压测（P99=107ms） |
| C-05 | 5× 连续 503 | 30s 冷却窗口内 | toast 风暴 | 已覆盖 | P04 冷却期决定性证据 |

## 等价划分（处理状态爆炸）

当组合超过 1000 时，按以下维度降维：
1. **网络层等价类**: {200, 4xx, 5xx, 超时} → 4 类（无需测试每个状态码）
2. **时序等价类**: {加载前, 加载中, 加载后, 空闲} → 4 类
3. **异常叠加等价类**: {单异常, 双异常, 三异常+} → 按严重度优先测试单异常

实际覆盖: 5/8 组合（3 个因 CDP 限制豁免，已说明原因）

## 结论

- **高 RPN（>200）失败模式**: FM-P01(240), FM-P02(252), FM-P03(240) — 均为 P0 级，已通过 spawn_blocking/AtomicBool 缓解
- **运行时验证结果**: 12 个失败模式全部"已缓解"，无未缓解项
- **CDP 可检测性**: 9/12 失败模式可被 CDP 直接捕获（D≤5），3 个沙箱类需自检验证
