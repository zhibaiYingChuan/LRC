//! ============================================================
//! 许可证: Apache 2.0
//! 本文件提供侧车观测工具（时间戳 + 锁统计 + 写优先闸门），属于公开层 (Layer 1)。
//! ============================================================
//!
//! 观测工具（Observability）
//!
//! ## 为什么需要（2026-10-31 实测根因）
//!
//! **① 日志没有时间戳**：实测到"连续 81 秒 `lock_busy`、期间 MCP 写入 3/12 成功"，
//! 但侧车日志行不带时间，无法把日志与写入失败时刻对齐，只能靠外部采样反推。
//! 没有时间戳的日志在"偶发突发"类问题里几乎不可用（判不出先后、算不出持续时长）。
//!
//! **② 锁只有现象、没有账本**：全进程只有一把存储锁（改造前 `Mutex`，现 `RwLock`），
//! 读（每路在锁内跑 30~40s）与写（毫秒级）互相饿死，但此前只能看到
//! `/v1/health/system` 的 `lock_busy` 布尔值，看不到**谁在等、等了多久、超时几次**。
//!
//! ⇒ 本模块提供进程级原子计数，经 `/v1/health/system` 的 `lock_stats` 暴露。
//! 计数全是 `Relaxed` 原子操作（无锁、无内存分配），可安全放在取锁热路径上。
//!
//! ## 写优先闸门（`WriteWaitGuard`）
//!
//! **现状（2026-11-05 读并发改造后）**：`RwLock` 让读者并行，写者只需等
//! 当前写者，不再需要读者让行。实测（16R+16W）恢复让行反而降低吞吐并触发
//! 503，故 v1 通道已**不再登记**；本类型保留供 `consolidation` 的
//! `write_waiting()` 检查使用（当前恒为 false，即结晶任务不再为写入让行）。

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// 统一日志时间戳（本地时区，精确到毫秒）。
///
/// 格式 `MM-DD HH:MM:SS.mmm`：日志文件通常按天滚动，年份/日期前缀反而占宽，
/// 故只保留到月日；需要完整日期时用时间戳反查日志文件本身即可。
pub fn ts() -> String {
    chrono::Local::now()
        .format("%m-%d %H:%M:%S%.3f")
        .to_string()
}

// ============================================================
// 写优先闸门
// ============================================================

/// 当前**正在等待存储锁**的写入请求数（进程级）。
///
/// # 为什么必须是进程级静态量，而不是 `MemoryStore` 的字段
///
/// 检索路径要在**取锁之前**判断"是否有写入在等"；而那一刻它还没拿到存储锁，
/// 根本读不到锁内的字段。sidecar 进程内只有一个 `MemoryStore`，
/// 因此进程级计数与"该存储的写入等待者"是一一对应的。
///
/// # 它解决的是什么（线上实测）
///
/// 检索阶段维持 2~4 路并发题流，每路在**锁内**跑 30~40 秒。任何时刻只要有一路
/// 检索在跑，锁就几乎无空窗：写入（只需毫秒级）在 45s 内仍可能一次都抢不到，
/// 实测 Add 成功率 0%（`LRC budget exhausted before`）。
/// 让"新到的检索"在有写入等待时**主动让行 10ms**，写入即可在≤一个轮询周期内拿到锁，
/// 而检索只是晚 10~几十 ms 拿到锁 —— 不改变任何检索结果。
static WRITE_WAITERS: AtomicUsize = AtomicUsize::new(0);

/// 写入等待标记的 RAII 守卫：构造即登记，析构即注销（panic 也不会泄漏计数）。
///
/// 作用域应当**只覆盖"等待锁"这一段**（取到锁即可释放）：这样检索只需在
/// 写入真正排队时让行，而不会在写入漫长的锁外编码阶段被白白压制。
pub struct WriteWaitGuard;

impl WriteWaitGuard {
    pub fn new() -> Self {
        WRITE_WAITERS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Default for WriteWaitGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WriteWaitGuard {
    fn drop(&mut self) {
        WRITE_WAITERS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 是否有写入正在等待存储锁（供长读路径让行）。
pub fn write_waiting() -> bool {
    WRITE_WAITERS.load(Ordering::SeqCst) > 0
}

// ============================================================
// 锁统计
// ============================================================

/// 统计维度（**固定数组**而非 `HashMap`）
///
/// 取锁是热路径，`HashMap` 需要额外一把锁与堆分配 —— 为观测引入新的锁竞争
/// 是本末倒置。固定 6 个标签用线性查找（≤6 次字符串比较）即可。
const LOCK_LABELS: [&str; 6] = [
    // v1 写入路径（/v1/memories/consolidate 两阶段取锁）
    "store.write",
    // MCP 通道（remember / batch_remember / forget / update / archive / correct）
    "mcp.store",
    // 健康检查（try_lock 一次，失败即降级）
    "store.health",
    // 后台结晶（try_lock 轮询至 LOCK_WAIT）
    "consolidate",
    // 图落盘（后台 flush 持锁取快照）
    "graph.flush",
    // 长读路径（enrich / recall 取锁）
    "store.read",
];

/// 单个标签的累加器
struct LockStat {
    /// 成功取锁次数
    waits: AtomicU64,
    /// 全部等待时长累计（毫秒）
    wait_ms_total: AtomicU64,
    /// 单次最长等待（毫秒）
    wait_ms_max: AtomicU64,
    /// 等待超时次数（未取到锁即放弃）
    timeouts: AtomicU64,
    /// 单次最长持锁时长（毫秒，仅少数关键段落埋点）
    hold_ms_max: AtomicU64,
}

impl LockStat {
    const fn new() -> Self {
        Self {
            waits: AtomicU64::new(0),
            wait_ms_total: AtomicU64::new(0),
            wait_ms_max: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            hold_ms_max: AtomicU64::new(0),
        }
    }
}

/// 6 个标签的累加器（顺序与 [`LOCK_LABELS`] 一一对应）
///
/// 用内联 `const` 块展开：原子类型不实现 `Copy`，普通数组重复在这里不成立
/// （MSRV 1.80 已支持内联 const）。
static LOCK_STATS: [LockStat; 6] = [const { LockStat::new() }; 6];

fn label_index(label: &str) -> Option<usize> {
    LOCK_LABELS.iter().position(|l| *l == label)
}

/// 无锁更新最大值（CAS 循环，冲突概率极低）
fn bump_max(target: &AtomicU64, value: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

/// 一次取锁的计时器：`start` 在**开始等待之前**调用，取到锁后调用 [`Self::ok`]，
/// 放弃时调用 [`Self::timeout`]。
///
/// 标签不在 [`LOCK_LABELS`] 中时静默忽略（`idx = None`），
/// 这样新增埋点时写错标签不会 panic，也不会污染统计。
pub struct LockTimer {
    idx: Option<usize>,
    started: Instant,
}

impl LockTimer {
    pub fn start(label: &str) -> Self {
        Self {
            idx: label_index(label),
            started: Instant::now(),
        }
    }

    /// 记录一次**成功**取锁（含等待时长）。
    pub fn ok(&self) {
        if let Some(i) = self.idx {
            let ms = self.started.elapsed().as_millis() as u64;
            let stat = &LOCK_STATS[i];
            stat.waits.fetch_add(1, Ordering::Relaxed);
            stat.wait_ms_total.fetch_add(ms, Ordering::Relaxed);
            bump_max(&stat.wait_ms_max, ms);
        }
    }

    /// 记录一次**等待超时**（同样计入等待时长与最长等待）。
    pub fn timeout(&self) {
        if let Some(i) = self.idx {
            let ms = self.started.elapsed().as_millis() as u64;
            let stat = &LOCK_STATS[i];
            stat.timeouts.fetch_add(1, Ordering::Relaxed);
            stat.wait_ms_total.fetch_add(ms, Ordering::Relaxed);
            bump_max(&stat.wait_ms_max, ms);
        }
    }
}

/// 记录一次**持锁时长**（仅关键段落埋点，避免每个锁位置都加计时开销）。
pub fn record_hold(label: &str, held: Duration) {
    if let Some(i) = label_index(label) {
        bump_max(&LOCK_STATS[i].hold_ms_max, held.as_millis() as u64);
    }
}

/// RAII 持锁计时器：构造即开始，`drop` 时把时长记入 `hold_ms_max`。
///
/// # 为什么需要（S3 前置测量，2026-11-01）
///
/// 推进计划 S3 要把"取快照"与"重计算"分开，但改动前必须先知道
/// **单次检索到底在锁内待了多久**。`LockTimer` 只能测"等锁"，
/// 而读路径的持锁区间跨越多个函数调用，手写 `record_hold` 容易漏掉提前返回。
/// 用 RAII 把它绑到 guard 的作用域上：无论正常返回、`?` 还是 `return`，
/// 时长都会被记下（唯一例外是 panic，而 panic 时本进程本就不再服务）。
pub struct HoldTimer {
    label: &'static str,
    started: Instant,
}

impl HoldTimer {
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            started: Instant::now(),
        }
    }
}

impl Drop for HoldTimer {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        record_hold(self.label, elapsed);
        // S0.2（2026-11-03）：持锁超过 2s 立即打点。
        // 背景：`graph.flush` 曾出现 339s 的**等锁**，但所有已埋点的持锁都 <5s
        // ⇒ 长持锁者落在未埋点的 `lock().await` 上。把阈值日志绑到 RAII 上，
        // 下一次长持锁会直接以"标签 + 时长"出现在 sidecar 日志里，无需再靠外部采样反推。
        if elapsed >= Duration::from_secs(2) {
            eprintln!(
                "{} [LRC·锁] ⚠ {} 持锁 {}ms（>2s 阈值）",
                ts(),
                self.label,
                elapsed.as_millis()
            );
        }
    }
}

/// 锁统计快照（供 `/v1/health/system` 的 `lock_stats` 字段）。
pub fn lock_stats_json() -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (i, label) in LOCK_LABELS.iter().enumerate() {
        let stat = &LOCK_STATS[i];
        map.insert(
            (*label).to_string(),
            serde_json::json!({
                "waits": stat.waits.load(Ordering::Relaxed),
                "wait_ms_total": stat.wait_ms_total.load(Ordering::Relaxed),
                "wait_ms_max": stat.wait_ms_max.load(Ordering::Relaxed),
                "timeouts": stat.timeouts.load(Ordering::Relaxed),
                "hold_ms_max": stat.hold_ms_max.load(Ordering::Relaxed),
            }),
        );
    }
    // 当前写入等待者数：用来回答"检索让行机制是否真的在工作"
    map.insert(
        "write_waiters".to_string(),
        serde_json::json!(WRITE_WAITERS.load(Ordering::Relaxed)),
    );
    serde_json::Value::Object(map)
}
