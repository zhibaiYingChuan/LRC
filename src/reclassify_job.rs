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
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// 任务被用户取消（可从 `processed` 处续跑）
    Cancelled,
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
            JobStatus::Cancelled => "cancelled",
            JobStatus::Failed => "failed",
        }
    }

    /// 是否可从进度点续跑（取消或失败，且已处理过至少一批）。
    pub fn is_resumable(&self) -> bool {
        matches!(self, JobStatus::Cancelled | JobStatus::Failed)
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

    /// 是否可从当前进度点续跑。
    ///
    /// - `cancelled` / `failed` 且**还有剩余条目**：可续跑；
    ///   已处理完全部条目（`processed >= total`）时无事可做，不报可续跑；
    /// - `completed` 且 `processed < total`：**配额完成**（`--limit` 分块），
    ///   也可续跑；否则全量完成不可续跑。
    ///
    /// 口径必须与落盘形态 `PersistedReclassifyProgress::resumable` 一致，
    /// 否则同一进度在内存态报 true、重启后报 false（v0.11.1 审查 P2）。
    pub fn resumable(&self) -> bool {
        match self.status {
            JobStatus::Cancelled | JobStatus::Failed => {
                self.processed > 0 && self.processed < self.total
            }
            JobStatus::Completed => self.processed < self.total,
            _ => false,
        }
    }
}

/// 重编码进度的**落盘形态**（进程重启后仍可续跑）。
///
/// 该文件位于数据目录（`reclassify_progress.json`），属于**可重建的诊断状态**：
/// 删除后只是失去"重启后续跑"能力，不会影响记忆数据，因此不参与备份。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedReclassifyProgress {
    pub task_id: String,
    pub status: String,
    pub total: usize,
    pub processed: usize,
    pub changed: usize,
    pub before: [usize; 8],
    pub after: [usize; 8],
    pub started_ms: u64,
    pub updated_ms: u64,
}

impl PersistedReclassifyProgress {
    /// 是否可从落盘进度续跑。
    ///
    /// `running` 也视为可续跑：它代表"上个进程在处理中被终止"，
    /// 而进度只在每批处理完后落盘，`processed` 始终是安全锚点。
    pub fn resumable(&self) -> bool {
        match self.status.as_str() {
            "cancelled" | "failed" | "running" => self.processed > 0 && self.processed < self.total,
            "completed" => self.processed < self.total,
            _ => false,
        }
    }
}

/// 进度文件名（位于数据目录）。
pub const RECLASSIFY_PROGRESS_FILE: &str = "reclassify_progress.json";

impl ReclassifyJobSnapshot {
    /// 转为可落盘的进度记录。
    fn to_persisted(&self) -> PersistedReclassifyProgress {
        PersistedReclassifyProgress {
            task_id: self.task_id.clone(),
            status: self.status.as_str().to_string(),
            total: self.total,
            processed: self.processed,
            changed: self.changed,
            before: self.before,
            after: self.after,
            started_ms: self.started_ms,
            updated_ms: now_ms(),
        }
    }
}

/// 读取落盘进度（文件缺失/损坏 ⇒ `None`，按"无历史"处理）。
pub fn load_progress(data_dir: &Path) -> Option<PersistedReclassifyProgress> {
    let text = std::fs::read_to_string(data_dir.join(RECLASSIFY_PROGRESS_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// 原子写入进度；失败只告警（进度丢失不应影响重编码本身）。
pub fn save_progress(data_dir: &Path, snapshot: &ReclassifyJobSnapshot) {
    let path = data_dir.join(RECLASSIFY_PROGRESS_FILE);
    let bytes = match serde_json::to_vec_pretty(&snapshot.to_persisted()) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("[LRC·重编码] 进度序列化失败（不影响任务）: {}", e);
            return;
        }
    };
    if let Err(e) = crate::atomic_file::write_atomic(&path, &bytes) {
        eprintln!("[LRC·重编码] 进度落盘失败（不影响任务）: {}", e);
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
    /// 取消请求标志：由 HTTP 线程置位，后台循环在每个批次边界检查。
    cancel_requested: AtomicBool,
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
            cancel_requested: AtomicBool::new(false),
        }
    }

    /// 尝试启动一个新任务（从 0 开始）。
    ///
    /// - 成功：写入初始快照（`after` 以 `before` 为基底），返回 `Ok(())`
    /// - 已有任务运行中：返回 `Err(现有 task_id)`，供上层回 409 冲突
    pub fn try_start(
        &self,
        task_id: String,
        total: usize,
        before: [usize; 8],
    ) -> Result<(), String> {
        self.try_start_from(task_id, total, before, 0)
    }

    /// 尝试启动一个任务，并从 `start_offset` 处继续。
    ///
    /// `processed` 直接置为 `start_offset`：它同时是**进度分母的起点**与
    /// **下次续跑的锚点**。`start_offset > total` 时夹取到 `total`（防御越界）。
    pub fn try_start_from(
        &self,
        task_id: String,
        total: usize,
        before: [usize; 8],
        start_offset: usize,
    ) -> Result<(), String> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.status == JobStatus::Running {
            return Err(st.task_id.clone());
        }
        let start_offset = start_offset.min(total);
        // 新一轮任务必须清除上一轮的取消标志，否则会立刻自我取消。
        self.cancel_requested.store(false, Ordering::Release);
        *st = ReclassifyJobSnapshot {
            task_id,
            status: JobStatus::Running,
            total,
            processed: start_offset,
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
    ///
    /// `processed` 夹取到 `total`：运行中删除记忆时批次可能多报，
    /// 进度不应显示超过 100%（续跑锚点另有 clamp）。
    pub fn apply_batch(&self, batch: &BaguaReclassifyBatch) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // 运行中新增记忆时，批次看到的总数会变大：同步上调分母，
        // 否则 processed 会被旧的 total 夹小、续跑锚点偏小（回归审查 P2-1）。
        st.total = st.total.max(batch.total);
        st.processed = st.processed.saturating_add(batch.processed).min(st.total);
        st.changed = st.changed.saturating_add(batch.changed);
        for i in 0..8 {
            let delta = batch.new_counts[i] as isize - batch.old_counts[i] as isize;
            let cur = st.after[i] as isize + delta;
            st.after[i] = cur.max(0) as usize;
        }
    }

    /// 标记任务正常完成。
    ///
    /// ★取消优先：若取消标志已在置位（例如最后一刻收到 cancel），
    /// 状态记为 `Cancelled` 而不是 `Completed`，避免"取消成功但显示完成"的
    /// 语义矛盾。`finish_ok` 与 `request_cancel_if_task` 共用同一把状态锁，
    /// 因此不存在"置位后又被 Completed 覆盖"的窗口。
    pub fn finish_ok(&self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.status = if self.cancel_requested.load(Ordering::Acquire) {
            JobStatus::Cancelled
        } else {
            JobStatus::Completed
        };
        st.finished_ms = now_ms();
    }

    /// 标记任务失败（携带原因）
    pub fn finish_err(&self, msg: &str) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.status = JobStatus::Failed;
        st.error = Some(msg.to_string());
        st.finished_ms = now_ms();
    }

    /// 请求取消当前任务（幂等）。
    ///
    /// 只置位标志，不改变状态：后台循环在**下一个批次边界**观察到后
    /// 才会把状态置为 `Cancelled`。这样进度快照里的 `processed`
    /// 始终是"已安全落盘"的条数，可直接作为续跑锚点。
    pub fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::Release);
    }

    /// 是否已请求取消
    pub fn is_cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::Acquire)
    }

    /// 仅当 `task_id` 仍是当前运行中的任务时，登记取消。
    ///
    /// 这是取消端点的**原子判定**：先读状态再置位会被"旧任务刚完成、新任务
    /// 已启动"的窗口穿透，误取消新任务（审查 P2-E）。本方法在同一把状态锁内
    /// 完成"校验 + 置位"。
    pub fn request_cancel_if_task(&self, task_id: &str) -> bool {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.status != JobStatus::Running || st.task_id != task_id {
            return false;
        }
        self.cancel_requested.store(true, Ordering::Release);
        true
    }

    /// 标记任务已取消（保留 `processed` 供续跑）
    pub fn finish_cancelled(&self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.status = JobStatus::Cancelled;
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
/// - `limit`：本次最多处理条数（`0` = 不限；续跑时相对 `start_offset` 计算）
/// - `progress_dir`：进度落盘目录（`None` = 不落盘，供单测使用）
pub fn run_reclassify_job(
    job: &ReclassifyJobHandle,
    store: &SharedStore,
    batch_size: usize,
    max_busy_retries: usize,
    limit: usize,
    progress_dir: Option<&Path>,
) {
    let batch_size = if batch_size == 0 {
        DEFAULT_RECLASSIFY_BATCH_SIZE
    } else {
        batch_size
    };
    let start_offset = job.snapshot().processed;
    // `limit` 是"本次运行的处理量"，不是绝对偏移：续跑 120 后再 --limit 50
    // 应处理 [120, 170)，而不是直接跳到 50 之前。
    let target = if limit == 0 {
        usize::MAX
    } else {
        start_offset.saturating_add(limit)
    };
    let mut offset = start_offset;
    let mut busy_retries = 0usize;
    // 每批结束后落盘一次：进程被杀后 `--resume` 可从安全锚点继续。
    let persist = || {
        if let Some(dir) = progress_dir {
            save_progress(dir, &job.snapshot());
        }
    };

    loop {
        // 取消请求在**批次边界**生效：不打断正在处理的批次，
        // 因此 `processed` 始终是已落盘的安全锚点，可直接续跑。
        if job.is_cancel_requested() {
            job.finish_cancelled();
            persist();
            return;
        }
        if offset >= target {
            job.finish_ok();
            persist();
            return;
        }
        // 取锁：成功即重置退避计数；失败则退避后重试，超限判失败
        let guard = match store.try_write() {
            Ok(g) => {
                busy_retries = 0;
                g
            }
            Err(_) => {
                busy_retries += 1;
                if busy_retries > max_busy_retries {
                    job.finish_err("store_busy_timeout");
                    persist();
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(RECLASSIFY_BUSY_BACKOFF_MS));
                continue;
            }
        };

        // 处理一批后立即释放锁（显式 drop，保证用户请求可穿插）。
        // 本批不得超过剩余配额，否则 --limit 会多处理。
        let this_batch = batch_size.min(target.saturating_sub(offset)).max(1);
        let batch_result = guard.reclassify_bagua_batch(offset, this_batch);
        drop(guard);

        match batch_result {
            Ok(batch) => {
                let processed = batch.processed;
                let total = batch.total;
                job.apply_batch(&batch);
                persist();
                offset = offset.saturating_add(processed);
                // 已处理到末尾、达到本次配额，或本批无条目可处理（空库/offset 越界）⇒ 收尾
                if processed == 0 || offset >= total || offset >= target {
                    job.finish_ok();
                    persist();
                    return;
                }
            }
            Err(_) => {
                job.finish_err("reclassify_failed");
                persist();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_store::MemoryStore;
    use crate::persistence::create_json_persistence;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn shared_store() -> (SharedStore, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("临时目录");
        let persistence =
            create_json_persistence(dir.path().to_str().expect("路径可转字符串")).expect("持久化");
        let store: SharedStore = Arc::new(RwLock::new(MemoryStore::new(persistence)));
        (store, dir)
    }

    #[test]
    fn cancel_flag_lifecycle() {
        let job = ReclassifyJobHandle::new();
        job.try_start("t1".into(), 100, [0; 8]).expect("启动");
        assert!(!job.is_cancel_requested(), "新任务不得自带取消标志");
        job.request_cancel();
        assert!(job.is_cancel_requested());
        job.finish_cancelled();
        let snap = job.snapshot();
        assert_eq!(snap.status, JobStatus::Cancelled);
        assert!(snap.status.is_resumable(), "取消后必须允许续跑");
    }

    #[test]
    fn resume_from_offset_and_block_while_running() {
        let job = ReclassifyJobHandle::new();
        job.try_start("t1".into(), 100, [0; 8]).expect("启动");
        assert!(
            job.try_start_from("t2".into(), 100, [0; 8], 40).is_err(),
            "运行中不得重复启动"
        );
        job.request_cancel();
        job.finish_cancelled();
        job.try_start_from("t2".into(), 100, [0; 8], 40)
            .expect("取消后应允许从进度点续跑");
        let snap = job.snapshot();
        assert_eq!(snap.processed, 40, "续跑必须从锚点开始");
        assert_eq!(snap.status, JobStatus::Running);
        assert!(!job.is_cancel_requested(), "新一轮必须清除上一轮的取消标志");
    }

    /// 审查 P2-E：取消必须按 task_id 原子判定，不能误取消新任务。
    #[test]
    fn cancel_is_scoped_to_task_id() {
        let job = ReclassifyJobHandle::new();
        job.try_start("t1".into(), 100, [0; 8]).expect("启动");
        assert!(
            !job.request_cancel_if_task("stale-task"),
            "task_id 不匹配时不得登记取消"
        );
        assert!(!job.is_cancel_requested(), "不匹配的取消不得置位");
        assert!(job.request_cancel_if_task("t1"), "当前任务应可取消");
        assert!(job.is_cancel_requested());
    }

    /// 审查 P2-E：最后一刻的取消不得被 finish_ok 覆盖成 Completed。
    #[test]
    fn finish_ok_respects_pending_cancel() {
        let job = ReclassifyJobHandle::new();
        job.try_start("t1".into(), 10, [0; 8]).expect("启动");
        job.request_cancel_if_task("t1");
        job.finish_ok();
        assert_eq!(
            job.snapshot().status,
            JobStatus::Cancelled,
            "已登记取消的任务不得显示为完成"
        );
    }

    /// 回归审查 P2-2：`--limit` 分块完成（processed < total）必须可续跑。
    #[test]
    fn completed_with_remaining_quota_is_resumable() {
        let job = ReclassifyJobHandle::new();
        job.try_start_from("t".into(), 100, [0; 8], 40)
            .expect("启动");
        job.finish_ok();
        let snap = job.snapshot();
        assert_eq!(snap.status, JobStatus::Completed);
        assert!(snap.resumable(), "配额完成（processed<total）应可续跑");
        assert_eq!(snap.processed, 40);

        // 全量完成后不可续跑
        job.try_start_from("t2".into(), 40, [0; 8], 40)
            .expect("启动");
        job.finish_ok();
        assert!(!job.snapshot().resumable(), "全量完成不得标记可续跑");
    }

    /// v0.11.1 审查 P2：内存态与落盘态的 `resumable` 口径必须一致。
    #[test]
    fn resumable_matches_persisted_definition() {
        let job = ReclassifyJobHandle::new();
        job.try_start_from("t1".into(), 100, [0; 8], 40)
            .expect("启动");
        job.request_cancel();
        job.finish_cancelled();
        let snap = job.snapshot();
        assert!(snap.resumable(), "取消且还有剩余应可续跑");
        assert!(snap.to_persisted().resumable(), "内存态与落盘态必须一致");

        // 最后一批处理完才取消：已无剩余，不得再报可续跑。
        job.try_start_from("t2".into(), 100, [0; 8], 100)
            .expect("启动");
        job.request_cancel();
        job.finish_cancelled();
        let snap = job.snapshot();
        assert_eq!(snap.processed, 100);
        assert!(!snap.resumable(), "已处理完全部条目不得报可续跑");
        assert!(!snap.to_persisted().resumable(), "落盘态口径必须一致");
    }

    #[test]
    fn start_offset_is_clamped_to_total() {
        let job = ReclassifyJobHandle::new();
        job.try_start_from("t".into(), 10, [0; 8], 999)
            .expect("启动");
        assert_eq!(job.snapshot().processed, 10, "越界锚点必须夹取到 total");
    }

    #[test]
    fn pre_cancelled_job_stops_before_touching_store() {
        let (store, _dir) = shared_store();
        let job = ReclassifyJobHandle::new();
        job.try_start_from("t".into(), 100, [0; 8], 30)
            .expect("启动");
        job.request_cancel();
        run_reclassify_job(&job, &store, 10, 1, 0, None);
        let snap = job.snapshot();
        assert_eq!(snap.status, JobStatus::Cancelled);
        assert_eq!(snap.processed, 30, "取消必须保留续跑锚点");
    }

    /// 进度落盘：进程重启后可从文件恢复锚点；全量完成不可续跑。
    #[test]
    fn persisted_progress_roundtrip_and_resumable() {
        let dir = tempfile::TempDir::new().expect("临时目录");
        let job = ReclassifyJobHandle::new();
        job.try_start_from("t".into(), 100, [1; 8], 40)
            .expect("启动");
        save_progress(dir.path(), &job.snapshot());
        let loaded = load_progress(dir.path()).expect("应能读回");
        assert_eq!(loaded.processed, 40);
        assert_eq!(loaded.total, 100);
        assert!(loaded.resumable(), "中断进度应可续跑");

        job.finish_cancelled();
        job.try_start_from("t2".into(), 40, [1; 8], 40)
            .expect("启动");
        job.finish_ok();
        save_progress(dir.path(), &job.snapshot());
        let done = load_progress(dir.path()).expect("应能读回");
        assert!(!done.resumable(), "全量完成不得标记可续跑");
    }

    /// 进度文件缺失/损坏 ⇒ 按"无历史"处理，不得让启动失败。
    #[test]
    fn missing_or_corrupt_progress_is_none() {
        let dir = tempfile::TempDir::new().expect("临时目录");
        assert!(load_progress(dir.path()).is_none());
        std::fs::write(dir.path().join(RECLASSIFY_PROGRESS_FILE), "not-json").expect("写入");
        assert!(load_progress(dir.path()).is_none());
    }
}
