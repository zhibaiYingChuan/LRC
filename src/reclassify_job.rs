//! ============================================================
//! 许可证: Apache 2.0
//! 洛书八卦历史重分类的**异步任务与进度状态机**（v0.9.10 塔缩修复·第 2 层）。
//! ============================================================
//!
//! 背景：全库重分类（`MemoryStore::reclassify_bagua`）在大盘库上单次持锁可长达
//! 数十分钟，会撞上网关 30s 超时（504）并阻塞用户查询路径。本模块把重分类改造成
//! **分批推进 + 进度可查**的后台任务：
//!
//!   - 触发：`POST /v1/memories/reclassify-bagua` 携带 `"async": true` → 立即 202
//!   - 进度：`GET /v1/memories/reclassify-bagua/progress` → 返回快照
//!
//! 关键设计：
//!   1. **按 offset 分批释放锁**：每批处理完立即 drop 锁，用户请求可穿插其间，
//!      避免"一次持锁数十分钟"饿死正常读写。
//!   2. **状态机可实例化**：`ReclassifyJobHandle` 不是硬编码全局，测试可各自
//!      `new()` 一份互不干扰；生产通过 `global_job()` 取单例。
//!   3. **增量分布累加**：启动瞬间抓取 `before` 基线，之后用每批的
//!      `old_counts/new_counts` 增量推进 `after`，无需二次全量扫描。

use crate::memory_store::BaguaReclassifyBatch;
use crate::v1_api::SharedStore;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 默认每批处理条数：在"持锁时长"与"锁切换次数"之间取折中。
pub const DEFAULT_RECLASSIFY_BATCH_SIZE: usize = 50;
/// 默认锁忙重试次数上限（配合退避间隔，约等于最长等待时长）。
pub const DEFAULT_RECLASSIFY_BUSY_RETRIES: usize = 900;
/// 锁忙时的退避间隔（毫秒）：锁被用户请求占用时，让出后稍后再试。
pub const RECLASSIFY_BUSY_BACKOFF_MS: u64 = 200;

/// 异步重分类任务的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    /// 无任务（从未启动，或启动前的初始态）
    Idle,
    /// 任务运行中
    Running,
    /// 任务正常完成
    Completed,
    /// 任务失败（见快照的 error 字段）
    Failed,
}

impl JobStatus {
    /// 序列化为 API 字符串（与前端/测试约定的字面量一致）
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Idle => "idle",
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
        }
    }
}

/// 异步重分类任务的进度快照（不可变拷贝，供 API 直接序列化）。
#[derive(Debug, Clone)]
pub struct ReclassifyJobSnapshot {
    /// 任务标识（UUID，供前端轮询对齐）
    pub task_id: String,
    /// 当前状态
    pub status: JobStatus,
    /// 全库记忆总数
    pub total: usize,
    /// 已处理条数（跨批累加）
    pub processed: usize,
    /// 已发生分类变化的条数（跨批累加）
    pub changed: usize,
    /// 启动瞬间的全库分布基线
    pub before: [usize; 8],
    /// 当前累计分布（以 before 为基底，按批增量推进）
    pub after: [usize; 8],
    /// 启动时间（Unix 毫秒）
    pub started_ms: u64,
    /// 结束时间（Unix 毫秒；未结束时为 0）
    pub finished_ms: u64,
    /// 失败原因（仅 Failed 时有值）
    pub error: Option<String>,
}

impl ReclassifyJobSnapshot {
    /// 空快照（Idle 初始态）
    fn idle() -> Self {
        Self {
            task_id: String::new(),
            status: JobStatus::Idle,
            total: 0,
            processed: 0,
            changed: 0,
            before: [0; 8],
            after: [0; 8],
            started_ms: 0,
            finished_ms: 0,
            error: None,
        }
    }
}

/// 当前 Unix 毫秒时间戳
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 异步重分类任务的句柄：内部以互斥锁保护一份进度快照。
///
/// 之所以"可实例化"而非直接用全局静态：单测需要并行跑多份互不干扰的任务状态；
/// 生产环境通过 [`global_job`] 取进程内单例（同一时刻只允许一个重分类任务）。
pub struct ReclassifyJobHandle {
    state: Mutex<ReclassifyJobSnapshot>,
}

impl Default for ReclassifyJobHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl ReclassifyJobHandle {
    /// 创建空闲任务句柄
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ReclassifyJobSnapshot::idle()),
        }
    }

    /// 尝试启动一个新任务。
    ///
    /// - 成功：写入初始快照（`after` 以 `before` 为基底），返回 `Ok(())`
    /// - 已有任务运行中：返回 `Err(现有 task_id)`，供上层回 409 冲突
    pub fn try_start(
        &self,
        task_id: String,
        total: usize,
        before: [usize; 8],
    ) -> Result<(), String> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.status == JobStatus::Running {
            return Err(st.task_id.clone());
        }
        *st = ReclassifyJobSnapshot {
            task_id,
            status: JobStatus::Running,
            total,
            processed: 0,
            changed: 0,
            before,
            // 起始累计分布 == 基线（尚未处理任何条目）
            after: before,
            started_ms: now_ms(),
            finished_ms: 0,
            error: None,
        };
        Ok(())
    }

    /// 取当前快照拷贝
    pub fn snapshot(&self) -> ReclassifyJobSnapshot {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 累加一批的处理结果：进度 + 分布增量。
    ///
    /// 分布以 `after[i] += new_counts[i] − old_counts[i]` 推进；用 `isize` 中间量
    /// 防止下溢 panic（理论上旧桶计数 ≥ 本批 old_counts，但防御性处理更稳）。
    pub fn apply_batch(&self, batch: &BaguaReclassifyBatch) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.processed = st.processed.saturating_add(batch.processed);
        st.changed = st.changed.saturating_add(batch.changed);
        for i in 0..8 {
            let delta = batch.new_counts[i] as isize - batch.old_counts[i] as isize;
            let cur = st.after[i] as isize + delta;
            st.after[i] = cur.max(0) as usize;
        }
    }

    /// 标记任务正常完成
    pub fn finish_ok(&self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.status = JobStatus::Completed;
        st.finished_ms = now_ms();
    }

    /// 标记任务失败（携带原因）
    pub fn finish_err(&self, msg: &str) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.status = JobStatus::Failed;
        st.error = Some(msg.to_string());
        st.finished_ms = now_ms();
    }
}

/// 进程内全局任务单例（同一时刻只允许一个重分类任务）。
static GLOBAL_RECLASSIFY_JOB: OnceLock<ReclassifyJobHandle> = OnceLock::new();

/// 取全局重分类任务句柄（首次调用时惰性初始化）。
pub fn global_job() -> &'static ReclassifyJobHandle {
    GLOBAL_RECLASSIFY_JOB.get_or_init(ReclassifyJobHandle::new)
}

/// 同步执行一次分批重分类任务（在 `spawn_blocking` 线程中调用）。
///
/// 循环推进：每批"取锁 → 处理 `[offset, offset+batch_size)` → 释放锁 → 累加进度"。
/// 取锁失败（锁被用户请求占用）时退避重试；连续失败超过 `max_busy_retries`
/// 则判为 `store_busy_timeout` 失败，避免任务因长尾独占而无限挂起。
///
/// - `job`：任务句柄（进度写入目标）
/// - `store`：共享记忆库
/// - `batch_size`：每批条数；`0` 时回落到 [`DEFAULT_RECLASSIFY_BATCH_SIZE`]
/// - `max_busy_retries`：连续锁忙重试上限
pub fn run_reclassify_job(
    job: &ReclassifyJobHandle,
    store: &SharedStore,
    batch_size: usize,
    max_busy_retries: usize,
) {
    let batch_size = if batch_size == 0 {
        DEFAULT_RECLASSIFY_BATCH_SIZE
    } else {
        batch_size
    };
    let mut offset = 0usize;
    let mut busy_retries = 0usize;

    loop {
        // 取锁：成功即重置退避计数；失败则退避后重试，超限判失败
        let guard = match store.try_lock() {
            Ok(g) => {
                busy_retries = 0;
                g
            }
            Err(_) => {
                busy_retries += 1;
                if busy_retries > max_busy_retries {
                    job.finish_err("store_busy_timeout");
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(RECLASSIFY_BUSY_BACKOFF_MS));
                continue;
            }
        };

        // 处理一批后立即释放锁（显式 drop，保证用户请求可穿插）
        let batch_result = guard.reclassify_bagua_batch(offset, batch_size);
        drop(guard);

        match batch_result {
            Ok(batch) => {
                let processed = batch.processed;
                let total = batch.total;
                job.apply_batch(&batch);
                offset = offset.saturating_add(processed);
                // 已处理到末尾，或本批无条目可处理（空库/offset 越界）⇒ 收尾
                if processed == 0 || offset >= total {
                    job.finish_ok();
                    return;
                }
            }
            Err(_) => {
                job.finish_err("reclassify_failed");
                return;
            }
        }
    }
}
