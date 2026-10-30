// ============================================================
// 许可证: Apache 2.0
// 本文件实现 JSON 文件持久化，属于公开层 (Layer 1)。
// ============================================================
//
// JSON 持久化实现
//
// 将记忆和代码片段存储为 JSON 文件。
// 适用于轻量级部署（记忆数量 < 10,000 条）。
// 大数据量场景建议使用 SQLite 后端（P1 计划）。
//
// ## ⚠️ 已知规模边界（v0.9.11 / G9 收尾 / CR-41：把隐患变成**显式量化声明**）
//
// **单条写入是"全量重写 JSON"** ⇒ 写放大为 `O(N)`/条、批量写 `O(N²)`。
// 实测（仓库内既有剖析记录，**本轮未重跑**）：4,485 条规模下单次序列化
// mean ≈ 308ms、p95 ≈ 324ms。
//
// - **这是被接受的设计取舍**，不是缺陷：本后端的定位就是**< 10,000 条**的轻量部署
//   （语料规模上限已由上面两行声明）；
// - **缓解措施已存在**：批量路径（`update_memories`）把 N 次磁盘 I/O 降为 1 次；
//   查询路径不再逐条写（见 `last_accessed` 批量更新）；
// - **超出该规模的正确做法是换后端**：`postgres` / `qdrant` / `neo4j` 已是内置
//   feature，`sqlx` 路径按"字段清单单一真源"生成 SQL（见 `persistence::fields`）。
//   **不要**在此文件上继续叠加"分片 JSON"这类自研方案 —— 那是在重造一个数据库。

use super::{AssocFrequency, Persistence, PersistenceError};
use crate::chunker::CodeChunk;
// v0.9.7（P0-2 依赖倒置修复）：原为 `crate::engine::memory_state_machine::MemoryState`，
// 现指向 Layer 1 中立层，消除 Layer 1 → Layer 2 反向依赖。
use crate::lifecycle::LifecycleTable;
use crate::memory_state_machine::MemoryState;
use crate::memory_types::Memory;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

/// 锁序契约（v0.9.7 修复 GLOBAL_CODE_REVIEW_REPORT 并发 P1-5「嵌套锁」）
/// ============================================================
/// 本模块存在两把锁，**必须**按下述固定顺序获取，否则会构成 ABBA 死锁：
///
/// ```text
///   获取顺序（外层 → 内层）：
///     ① self.cache  (RwLock<Option<Vec<Memory>>>)   —— 进程内缓存一致性
///     ② JSON_WRITE_LOCK (Mutex<()>)                 —— 跨进程/跨实例文件写入串行化
///   另有独立于上述两把的进程级文件锁：
///     ③ self.acquire_process_write_guard()          —— 跨进程 flock（实现见下）
///   以及一把**共享**的快照一致性读锁（顺序 ④，最后取）：
///     ④ crate::atomic_file::writer_guard()          —— 与备份/恢复的独占快照区间互斥（CR-37）
/// ```
///
/// **为什么 JSON_WRITE_LOCK 是全局单例而非 per-instance**：
///   同一个数据目录可能被**多个 `JsonPersistence` 实例**持有（测试并行、多窗口
///   桌面端、CLI 与 sidecar 同时运行）。若锁放在实例字段上，两个实例各自持有
///   不同的锁去写**同一个** `memories.json`，`fs::rename` 前的 tmp 文件虽已加
///   UUID（见 `atomic_write`），但重命名仍会互相覆盖。全局单例保证同一进程内
///   所有实例的文件写入串行。
///
/// **当前调用点**：`atomic_write()` 内部获取 ②（见本文件 `atomic_write`）。
///   调用方在 `save_memory` / `batch_update` 等路径上**先**持有 ① 的写锁，
///   再通过 `atomic_write` 间接获取 ②，符合上文顺序，无逆向获取点。
///
/// **已知代价（有意接受，非缺陷）**：持 ① 期间执行磁盘 IO 会放大临界区
///   （见报告并发 P2-1）。选择接受的原因：JSON 后端定位为轻量部署
///   （<10,000 条），且放弃该顺序会引入"缓存与磁盘不一致"的正确性风险——
///   正确性优先于吞吐。若未来切换 SQLite/Qdrant 后端，应改用后端自身的
///   事务语义替代本锁序。
///
/// **改动须知**：任何新增的写路径都**不得**在持有 ② 时去获取 ①（即禁止
///   在 `atomic_write` 内部或其后回调中调用 `self.cache.write()`）。
static JSON_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// JSON 文件持久化后端
///
/// 内存缓存优化：使用 `RwLock<Option<Vec<Memory>>>` 缓存已加载的记忆，
/// 避免每次写操作（save/delete）都全量读取+反序列化 JSON 文件。
/// 对于 10,000+ 条记忆的场景，可将写操作延迟从 O(n) 读取+O(n) 写入
/// 降低为 O(1) 缓存查找+O(n) 写入，约 2x 性能提升。
pub struct JsonPersistence {
    /// 数据目录路径
    data_dir: PathBuf,
    /// 记忆文件路径
    memories_file: PathBuf,
    /// ★ B-1（2026-10-27）：**增量追加日志**（每条记忆一行紧凑 JSON）。
    ///
    /// 为什么需要：`save_memories` 此前每次都把**整库**序列化并重写（实测 432MB 库
    /// 单次 5.6~10.9 秒），`update_memories`（检索后回写 `last_accessed`）亦然。
    /// 现改为：**热路径只追加本批**，整库重写只在日志超过阈值时做一次"压实"。
    ///
    /// 读写契约：
    ///   · 追加：`append_journal()`，每条一行、紧凑 JSON、`\n` 分隔；
    ///   · 读取：`load_all_memories_from_disk()` = 基文件 + 日志重放（同 id 后写胜出）；
    ///   · 压实：日志超过 `LRC_JOURNAL_MAX_BYTES`（默认 64MB）⇒ 整库重写 + 删日志；
    ///   · 删除/清空/整体替换：一律整库重写 + 删日志（否则已删条目会被日志"复活"）；
    ///   · 崩溃安全：追加日志被截断时只丢弃不可解析的行，基文件不受影响。
    journal_file: PathBuf,
    /// 代码片段文件路径
    chunks_file: PathBuf,
    /// 归档记忆文件路径
    archive_file: PathBuf,
    /// 记忆缓存：避免每次写操作都全量读取 JSON 文件
    /// - `None`：缓存未初始化或已失效
    /// - `Some(vec)`：已加载的记忆列表
    cache: RwLock<Option<Vec<Memory>>>,
    /// ★ B1（2026-10-25）：缓存对应的**磁盘文件指纹**。
    ///
    /// 写路径此前无条件 `refresh_cache_from_disk()`（整库重读），线上库到 400MB 时
    /// 单次 Add 因此多付一次全量读+反序列化，耗时冲过 120s 外层超时 ⇒ 504。
    /// 现在只在"指纹与已知值不一致"（即持锁前确有外部写入）或缓存未初始化时才重读。
    /// 指纹算法与 `source_fingerprint()` 完全一致（mtime 纳秒 rotate_left(16) ^ 长度）。
    loaded_fp: RwLock<Option<u64>>,
}

impl JsonPersistence {
    /// 创建新的 JSON 持久化后端
    ///
    /// 自动创建数据目录（如果不存在）。
    /// 文件命名：
    /// - `memories.json` — 记忆数据
    /// - `chunks.json`    — 代码片段数据
    /// - `archive.json`   — 归档记忆（冷存储）
    pub fn new(data_dir: &str) -> Result<Self, PersistenceError> {
        let dir = PathBuf::from(data_dir);
        if !dir.exists() {
            fs::create_dir_all(&dir).map_err(|e| {
                PersistenceError::Other(format!("无法创建数据目录 '{}': {}", data_dir, e))
            })?;
        }

        Ok(Self {
            memories_file: dir.join("memories.json"),
            journal_file: dir.join("memories.journal.jsonl"),
            chunks_file: dir.join("chunks.json"),
            archive_file: dir.join("archive.json"),
            data_dir: dir,
            cache: RwLock::new(None),
            loaded_fp: RwLock::new(None),
        })
    }

    /// 确保数据目录存在（防御性检查，应对测试中被清理或运行时被删除的情况）
    fn ensure_data_dir(&self) -> Result<(), PersistenceError> {
        if !self.data_dir.exists() {
            fs::create_dir_all(&self.data_dir).map_err(|e| {
                PersistenceError::Other(format!(
                    "无法重建数据目录 '{}': {}",
                    self.data_dir.display(),
                    e
                ))
            })?;
        }
        Ok(())
    }

    /// 获取数据目录路径（供外部读取）
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 直接从磁盘加载记忆（不经过缓存，供 load_all_memories 使用）
    ///
    /// ★ B-1：基文件 + **追加日志重放**。同 id 后写胜出（日志里的更新覆盖基文件）。
    fn load_all_memories_from_disk(&self) -> Result<Vec<Memory>, PersistenceError> {
        let mut memories: Vec<Memory> = if !self.memories_file.exists() {
            Vec::new()
        } else {
            let content = fs::read_to_string(&self.memories_file)?;
            if content.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&content)?
            }
        };
        self.replay_journal(&mut memories);
        Ok(memories)
    }

    /// 追加日志的字节上限（超过即触发一次全量压实）
    ///
    /// 64MB ≈ 约 4 万条记忆的增量；按线上 ~180 次 Add/小时估算约 12 小时压实一次，
    /// 把"每次 O(N) 重写"摊薄成"每天一两次"。
    fn journal_max_bytes() -> u64 {
        std::env::var("LRC_JOURNAL_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v >= 1024 * 1024)
            .unwrap_or(64 * 1024 * 1024)
    }

    fn journal_len(&self) -> u64 {
        fs::metadata(&self.journal_file)
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// 本次写入是否必须"整库压实"（写 `memories.json` 并清空日志）
    ///
    /// ★ 2026-10-28 修复（本轮 full 评测发现的缺陷 13）：
    /// 增量持久化（B-1）落地后，全新数据目录里的**第一次写入只落日志**，
    /// `memories.json` 这个"事实基文件"根本不会被创建。后果有二：
    /// ① 崩溃安全退化——基文件缺失时日志是唯一副本，日志损坏/误删即整库丢失；
    /// ② 任何按契约读 `memories.json` 的路径读不到文件（v1_api 的两个
    ///    §3.42.5 负向对照测试因此失败，属真实可观测的契约破坏）。
    /// 故：**基文件缺失时一律压实**，首次写入即建立基文件并清空日志；
    /// 之后照常走增量（阈值 `LRC_JOURNAL_MAX_BYTES`，默认 64MB）。
    fn needs_compact(&self) -> bool {
        (!self.memories_file.exists()) || self.journal_len() >= Self::journal_max_bytes()
    }

    /// 把一批记忆追加到日志（每条一行紧凑 JSON）
    fn append_journal(&self, memories: &[Memory]) -> Result<(), PersistenceError> {
        if memories.is_empty() {
            return Ok(());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_file)?;
        let mut buf = String::new();
        for memory in memories {
            buf.push_str(&serde_json::to_string(memory)?);
            buf.push('\n');
        }
        file.write_all(buf.as_bytes())?;
        file.flush()?;
        // P0-3：`flush()` 只把数据交给内核，掉电/强杀仍可能丢最后一批；
        // 追加日志是"已确认写入"的唯一副本，必须真正落盘。
        file.sync_data()?;
        Ok(())
    }

    /// 日志作废标记：`truncate_journal` 删除失败时写入，加载侧见到即跳过重放。
    fn journal_invalidated_marker(&self) -> PathBuf {
        self.journal_file.with_extension("jsonl.invalidated")
    }

    /// 清空日志（整库重写 / 删除 / 清空之后调用）
    ///
    /// ★ P0-3（2026-10-30）：`fs::remove_file` 失败时**不能静默忽略**。
    /// 删除路径先原子重写基文件、再清日志；若清日志失败（Windows 上文件被
    /// 其它进程持有、杀毒扫描等），残留日志会在下次启动被 `replay_journal`
    /// 重放 ⇒ **已删除的记忆复活**，而接口当时已返回成功。
    /// 兜底：删除失败则写"日志作废"标记，加载侧见标记即跳过重放并清理残留。
    fn truncate_journal(&self) {
        if fs::remove_file(&self.journal_file).is_ok() {
            let _ = fs::remove_file(self.journal_invalidated_marker());
            return;
        }
        if self.journal_file.exists() {
            let marker = self.journal_invalidated_marker();
            if let Err(error) = atomic_write(&marker, "1") {
                eprintln!(
                    "[LRC·持久化] ⚠ 日志清空失败且作废标记写入失败：{}（残留日志可能使已删记忆复活，下次启动务必核查）",
                    error
                );
            }
        }
    }

    /// 把日志重放到 `base`：同 id 覆盖，未知 id 追加（后写胜出）
    fn replay_journal(&self, base: &mut Vec<Memory>) {
        // P0-3：日志已被判定作废（truncate 失败兜底）⇒ 以基文件为唯一事实源，
        // 绝不重放，并清理残留，避免已删记忆复活。
        let marker = self.journal_invalidated_marker();
        if marker.exists() {
            let _ = fs::remove_file(&self.journal_file);
            let _ = fs::remove_file(&marker);
            return;
        }
        if !self.journal_file.exists() {
            return;
        }
        let content = match fs::read_to_string(&self.journal_file) {
            Ok(content) => content,
            Err(_) => return,
        };
        let mut index: std::collections::HashMap<String, usize> = base
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.clone(), i))
            .collect();
        let mut applied = 0usize;
        let mut corrupted = 0usize;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let memory: Memory = match serde_json::from_str(line) {
                Ok(memory) => memory,
                Err(_) => {
                    // 进程被杀可能留下截断的尾行：跳过，不希望整库加载失败
                    corrupted += 1;
                    continue;
                }
            };
            match index.get(&memory.id).copied() {
                Some(i) => base[i] = memory,
                None => {
                    index.insert(memory.id.clone(), base.len());
                    base.push(memory);
                }
            }
            applied += 1;
        }
        if corrupted > 0 {
            eprintln!(
                "[json] 追加日志有 {} 行不可解析（崩溃残留），已跳过；有效重放 {} 条",
                corrupted, applied
            );
        }
    }

    /// 使缓存失效（当外部修改文件时调用）
    pub fn invalidate_cache(&self) {
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = None;
        *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// 数据源指纹（基文件 + **追加日志** 的 mtime/长度混合）。
    ///
    /// ★ B-1：日志也是事实源的一部分 ⇒ 指纹必须覆盖它，否则"别的进程只写了日志"
    /// 会被误判为"无变化"（与 CR-42 修掉的陈旧缓存问题同类）。
    fn disk_fingerprint(&self) -> Option<u64> {
        let base_meta = fs::metadata(&self.memories_file).ok();
        let journal_meta = fs::metadata(&self.journal_file).ok();
        // 两者都不存在 ⇒ 尚无任何数据（首次写入前），交由调用方按"未初始化"处理。
        if base_meta.is_none() && journal_meta.is_none() {
            return None;
        }
        let base_fp = match base_meta {
            Some(meta) => {
                let nanos = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                nanos.rotate_left(16) ^ meta.len()
            }
            None => 0,
        };
        let journal_fp = match journal_meta {
            Some(meta) => {
                let nanos = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                nanos.rotate_left(32) ^ meta.len()
            }
            None => 0,
        };
        Some(base_fp ^ journal_fp)
    }

    fn refresh_cache_from_disk(&self) -> Result<(), PersistenceError> {
        // 先取指纹再读：若读的过程中有外部写入，指纹会停留在"读之前"的值，
        // 从而使下一次 `refresh_cache_if_changed` 判定为过期并重读（安全方向）。
        let fp_before = self.disk_fingerprint();
        let memories = self.load_all_memories_from_disk()?;
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = Some(memories);
        *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = fp_before;
        Ok(())
    }

    /// ★ B1：仅当磁盘文件相对已知指纹发生变化（或其他进程写过 / 缓存未初始化）时才重读。
    ///
    /// 为什么安全：调用方都先持有 `acquire_process_write_guard()`（跨进程写锁），
    /// 持锁期间不可能有其他进程在写；指纹只用来判定"**拿锁之前**"是否发生过外部写入。
    /// 这正是原先无条件 `refresh_cache_from_disk()` 想保证的语义，只是把代价从
    /// "每次写都整库重读"降成"只有真发生外部写入才重读"。
    fn refresh_cache_if_changed(&self) -> Result<(), PersistenceError> {
        let initialized = self
            .cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        let known = *self.loaded_fp.read().unwrap_or_else(|e| e.into_inner());
        if !initialized || known.is_none() || self.disk_fingerprint() != known {
            self.refresh_cache_from_disk()?;
        }
        Ok(())
    }

    fn acquire_process_write_guard(&self) -> Result<ProcessWriteGuard, PersistenceError> {
        let guard_path = self.data_dir.join(".memories.write.guard");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&guard_path)
            {
                Ok(mut file) => {
                    if let Err(error) = writeln!(file, "{}", std::process::id()) {
                        let _ = fs::remove_file(&guard_path);
                        return Err(error.into());
                    }
                    return Ok(ProcessWriteGuard { path: guard_path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let guard_pid = fs::read_to_string(&guard_path)
                        .ok()
                        .and_then(|value| value.trim().parse::<u32>().ok());
                    let stale = guard_pid
                        .map(|pid| !crate::process_guard::is_pid_alive(pid))
                        .unwrap_or(false);
                    if stale {
                        let _ = fs::remove_file(&guard_path);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        if guard_pid.is_none() {
                            let _ = fs::remove_file(&guard_path);
                            continue;
                        }
                        return Err(PersistenceError::Other(
                            "等待跨进程 JSON 写入锁超时".to_string(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

struct ProcessWriteGuard {
    path: PathBuf,
}

impl Drop for ProcessWriteGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_archived_memories(path: &Path, memories: &[Memory]) -> Result<(), PersistenceError> {
    let json = serde_json::to_string_pretty(memories)?;
    atomic_write(path, &json)?;
    Ok(())
}

impl Persistence for JsonPersistence {
    /// 侧车文件定位用（`semantic_vectors.json` 等）。
    ///
    /// ★ 必须放在**本 trait impl 块内**：泛型上下文（`impl<P: Persistence>`）只认
    /// trait 方法，放在固有块里会静默落到 trait 的默认实现（`None`）⇒ 功能不生效。
    fn data_dir_path(&self) -> Option<&Path> {
        Some(self.data_dir())
    }

    fn load_memory_state(&self) -> Result<MemoryState, PersistenceError> {
        let path = self.data_dir.join("memory_state.json");
        if !path.exists() {
            return Ok(MemoryState::default());
        }
        let content = fs::read_to_string(path)?;
        if content.trim().is_empty() {
            return Ok(MemoryState::default());
        }
        Ok(serde_json::from_str(&content)?)
    }

    fn save_memory_state(&self, state: &MemoryState) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        let path = self.data_dir.join("memory_state.json");
        let json = serde_json::to_string_pretty(state)?;
        atomic_write(&path, &json)?;
        Ok(())
    }

    fn load_assoc_frequency(&self) -> Result<AssocFrequency, PersistenceError> {
        let path = self.data_dir.join("assoc_frequency.json");
        if !path.exists() {
            return Ok(AssocFrequency::default());
        }
        let content = fs::read_to_string(path)?;
        if content.trim().is_empty() {
            return Ok(AssocFrequency::default());
        }
        Ok(serde_json::from_str(&content)?)
    }

    fn save_assoc_frequency(&self, state: &AssocFrequency) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        let path = self.data_dir.join("assoc_frequency.json");
        let json = serde_json::to_string_pretty(state)?;
        atomic_write(&path, &json)?;
        Ok(())
    }

    /// 生命周期表加载（独立文件 `lifecycle.json`）。
    ///
    /// 文件不存在或为空 ⇒ 返回空表（等价于"全部 Active 且不衰减"），
    /// 与改造前行为一致，不需要迁移。
    fn load_lifecycle(&self) -> Result<LifecycleTable, PersistenceError> {
        let path = self.data_dir.join("lifecycle.json");
        if !path.exists() {
            return Ok(LifecycleTable::default());
        }
        let content = fs::read_to_string(path)?;
        if content.trim().is_empty() {
            return Ok(LifecycleTable::default());
        }
        Ok(serde_json::from_str(&content)?)
    }

    /// 生命周期表保存（原子写，独立于 `memories.json`）。
    fn save_lifecycle(&self, table: &LifecycleTable) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        let path = self.data_dir.join("lifecycle.json");
        let json = serde_json::to_string_pretty(table)?;
        atomic_write(&path, &json)?;
        Ok(())
    }

    /// JSON 后端完整实现状态持久化（memory_state.json / assoc_frequency.json）
    fn supports_state_persistence(&self) -> bool {
        true
    }

    fn save_memory(&self, memory: &Memory) -> Result<(), PersistenceError> {
        // 防御性检查：确保数据目录存在（应对临时目录被清理等场景）
        self.ensure_data_dir()?;

        let _process_guard = self.acquire_process_write_guard()?;
        // ★ B1：只在"确有外部写入"时整库重读（原为无条件重读，400MB 库下单次多付一次全量读）。
        self.refresh_cache_if_changed()?;
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        // v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT 第五节第 5 项「收敛生产路径 expect」）：
        //   原为 .expect("缓存已通过 ensure_cache_loaded 初始化")——该不变量由上一行
        //   refresh_cache_from_disk() 的成功返回保证（失败时已 `?` 提前返回），
        //   但一旦未来重构漏掉刷新调用，此处会 panic 而非返回可恢复错误。
        //   改为 map_err 转 PersistenceError，消除库路径 panic 面。
        let memories = cache.as_mut().ok_or_else(|| {
            PersistenceError::Other(
                "内存缓存未初始化（refresh_cache_from_disk 未生效）".to_string(),
            )
        })?;

        // 按 ID 查找并更新，或追加新记忆
        if let Some(existing) = memories.iter_mut().find(|m| m.id == memory.id) {
            *existing = memory.clone();
        } else {
            memories.push(memory.clone());
        }

        let serialize_start = std::time::Instant::now();
        let disk_start = std::time::Instant::now();
        // ★ B-1（增量持久化）：单条写入同样只追加日志，除非日志已到阈值需压实。
        // ★ 2026-10-28：基文件缺失时也压实（见 `needs_compact`），保证首次写入即建基文件。
        let compact = self.needs_compact();
        let write_result = if compact {
            // ★ B2：紧凑 JSON（不再 pretty）。
            match serde_json::to_string(&*memories) {
                Ok(json) => {
                    let result = atomic_write(&self.memories_file, &json);
                    if result.is_ok() {
                        self.truncate_journal();
                    }
                    result
                }
                Err(error) => Err(error.into()),
            }
        } else {
            self.append_journal(std::slice::from_ref(memory))
        };
        let serialize_ms = serialize_start.elapsed().as_secs_f64() * 1000.0;
        if write_result.is_err() {
            // v0.9.11（G9 收尾 / CR-35）：失败时**丢弃缓存**，而不是恢复一份全量深拷贝。
            // 依据：`atomic_write` 是原子的 —— 失败时磁盘仍是旧状态，
            // 下次读取从磁盘重载即得到与"恢复 previous"**完全等价**的结果；
            // 而 `previous = memories.clone()` 会在每次写路径上多持有一份整库副本，
            // 正是"峰值内存 3–4× 库大小"的主要来源之一。
            *cache = None;
        }
        drop(cache); // 释放写锁
                     // ★ B1：维护指纹 —— 写成功则记录新指纹（下次写无需重读）；
                     //   写失败则置 None（下次写强制重读，回到"以磁盘为事实源"的安全侧）。
        *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = if write_result.is_err() {
            None
        } else {
            self.disk_fingerprint()
        };
        write_result?;
        if std::env::var_os("LRC_PROFILE_REMEMBER").is_some() {
            eprintln!(
                "[LRC·profiling] save_memory serialize_ms={:.3} journal_write_ms={:.3}",
                serialize_ms,
                disk_start.elapsed().as_secs_f64() * 1000.0,
            );
        }
        Ok(())
    }

    /// 批量更新记忆（单次序列化 + 单次磁盘写入）
    ///
    /// 性能优化：相比循环调用 `save_memory`（每条触发一次全量序列化+磁盘写入），
    /// 此方法将 N 次磁盘 I/O 降为 1 次，适用于 recall 后批量更新 `last_accessed`。
    /// 仅更新传入的记忆，其他记忆保持不变。
    fn update_memories(&self, updated: &[Memory]) -> Result<(), PersistenceError> {
        if updated.is_empty() {
            return Ok(());
        }

        // 防御性检查：确保数据目录存在
        self.ensure_data_dir()?;

        let _process_guard = self.acquire_process_write_guard()?;
        // ★ B1：recall 后更新 last_accessed 也走这里 —— 原每次检索都要整库重读 400MB，
        //   是 enrich 长耗时的组成部分之一。改为指纹门控。
        self.refresh_cache_if_changed()?;
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        // v0.9.7 修复：同上，消除库路径 panic 面（原为 .expect("缓存已通过磁盘刷新初始化")）
        let memories = cache.as_mut().ok_or_else(|| {
            PersistenceError::Other(
                "内存缓存未初始化（refresh_cache_from_disk 未生效）".to_string(),
            )
        })?;

        // 构建待更新记忆的 ID → Memory 映射，O(M) 查找
        let mut update_map: std::collections::HashMap<&str, &Memory> =
            std::collections::HashMap::with_capacity(updated.len());
        for m in updated {
            update_map.insert(m.id.as_str(), m);
        }

        // 单次遍历：按 ID 更新匹配的记忆
        let mut touched: Vec<Memory> = Vec::with_capacity(updated.len());
        for existing in memories.iter_mut() {
            if let Some(new_mem) = update_map.get(existing.id.as_str()) {
                *existing = (*new_mem).clone();
                touched.push((*new_mem).clone());
            }
        }

        // ★ B-1（2026-10-27）：检索后的 `last_accessed` 回写此前会**整库重写**
        //   （432MB 库实测数秒，且每次检索都付一次），现改为只把变更条目追加进日志。
        //   同时去掉原来的 `previous = memories.clone()`（整库深拷贝）——失败时
        //   直接丢弃缓存即可（原子写保证磁盘仍是旧状态）。
        // ★ 2026-10-28：基文件缺失时也压实（见 `needs_compact`）。
        let compact = self.needs_compact();
        let write_result = if compact {
            match serde_json::to_string(&*memories) {
                Ok(json) => {
                    let result = atomic_write(&self.memories_file, &json);
                    if result.is_ok() {
                        self.truncate_journal();
                    }
                    result
                }
                Err(error) => Err(error.into()),
            }
        } else {
            self.append_journal(&touched)
        };
        if let Err(error) = write_result {
            *cache = None;
            drop(cache);
            *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = None;
            return Err(error);
        }
        drop(cache); // 释放写锁
        *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = self.disk_fingerprint();
        Ok(())
    }

    /// 批量保存记忆（新增或更新）：单次序列化 + 单次磁盘写入
    ///
    /// 相比循环 save_memory（每条 O(n) 序列化 + 重写整个文件），
    /// 此方法将 N 次磁盘 I/O 降为 1 次，用于结晶合成的批量写入。
    fn save_memories(&self, memories: &[Memory]) -> Result<(), PersistenceError> {
        if memories.is_empty() {
            return Ok(());
        }

        self.ensure_data_dir()?;
        let _process_guard = self.acquire_process_write_guard()?;
        // ★ B1：这是 /add 的热路径 —— 原无条件整库重读在 400MB 库上单次多付一次全量读，
        //   直接导致请求越过 120s 外层超时。改为指纹门控。
        // ★ 2026-10-28 分段计时（`LRC_TIMING=1`）：量出"指纹门控是否失效 → 整库重读"、
        //   "450k 条 ID 索引表构建"、"追加日志/压实"三段各占多少。
        let __st_t0 = std::time::Instant::now();
        self.refresh_cache_if_changed()?;
        let __st_refresh = __st_t0.elapsed().as_secs_f64();
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        // v0.9.7 修复：同上，消除库路径 panic 面（原为 .expect("缓存已通过磁盘刷新初始化")）
        let existing = cache.as_mut().ok_or_else(|| {
            PersistenceError::Other(
                "内存缓存未初始化（refresh_cache_from_disk 未生效）".to_string(),
            )
        })?;

        // 构建 ID → 索引 映射，支持"已存在则更新，不存在则追加"
        // 使用 String 作为 key（拥有所有权），避免 &str 借用 existing 导致可变借用冲突
        let __st_t1 = std::time::Instant::now();
        let mut index_map: std::collections::HashMap<String, usize> =
            std::collections::HashMap::with_capacity(existing.len() + memories.len());
        for (i, m) in existing.iter().enumerate() {
            index_map.insert(m.id.clone(), i);
        }
        let __st_idmap = __st_t1.elapsed().as_secs_f64();

        for m in memories {
            match index_map.get(&m.id) {
                Some(&idx) => {
                    existing[idx] = m.clone();
                }
                None => {
                    existing.push(m.clone());
                    index_map.insert(m.id.clone(), existing.len() - 1);
                }
            }
        }

        // ★ B-1（2026-10-27，增量持久化）：热路径**只追加本批**，不再整库重写。
        //
        // 依据（线上实测，432MB / 28 万条库）：整库序列化+写盘单次 5.6~10.9 秒，
        // 是 Add 长尾与取锁竞争的主因之一。现改为：
        //   · 增量：把本批写进 `memories.journal.jsonl`（O(本批)）；
        //   · 压实：日志超过 `LRC_JOURNAL_MAX_BYTES` 时整库重写一次并清空日志
        //     （把 O(N) 摊薄到约每 12 小时一次，见 `journal_max_bytes`）。
        // 崩溃安全：追加是"整行 + flush"，进程被杀最多留下截断尾行，重放时跳过。
        // ★ 2026-10-28：基文件缺失时也压实（见 `needs_compact`）。
        let compact = self.needs_compact();
        let __st_t2 = std::time::Instant::now();
        let write_result = if compact {
            // ★ B2：紧凑 JSON（省约 12% 字节与序列化时间；读路径不依赖缩进）。
            match serde_json::to_string(existing) {
                Ok(json) => {
                    let result = atomic_write(&self.memories_file, &json);
                    if result.is_ok() {
                        self.truncate_journal();
                    }
                    result
                }
                Err(error) => {
                    // v0.9.11（G9 / CR-35）：丢弃缓存而非深拷贝回滚（磁盘未被触碰，
                    // 重载即得旧状态）。
                    *cache = None;
                    return Err(error.into());
                }
            }
        } else {
            self.append_journal(memories)
        };
        if let Err(error) = write_result {
            *cache = None;
            drop(cache);
            // ★ B1：写失败 ⇒ 指纹置空，下次写强制以磁盘为事实源重读。
            *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = None;
            return Err(error);
        }
        drop(cache); // 释放写锁
                     // ★ B1：记录写盘后的新指纹（含日志），下一次写操作可跳过整库重读。
        *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = self.disk_fingerprint();
        if std::env::var("LRC_TIMING")
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            eprintln!(
                "[LRC·计时] write 落盘 n={} refresh={:.3}s idmap={:.3}s write={:.3}s compact={}",
                memories.len(),
                __st_refresh,
                __st_idmap,
                __st_t2.elapsed().as_secs_f64(),
                compact
            );
        }
        Ok(())
    }

    /// 全量替换所有记忆：单次序列化 + tmp+rename 原子写。
    ///
    /// 覆盖默认实现（clear+save 两端点存在 C05 全库丢失窗口）：
    /// 无论空列表还是非空列表都以一次原子写落盘，进程崩溃也不会丢库。
    fn replace_all_memories(&self, memories: &[Memory]) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        let _process_guard = self.acquire_process_write_guard()?;
        let json = serde_json::to_string_pretty(memories)?;
        atomic_write(&self.memories_file, &json)?;
        // ★ B-1：整体替换后必须清空追加日志，否则旧日志会把已删除条目"复活"。
        self.truncate_journal();
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = Some(memories.to_vec());
        // ★ B1：整体替换后同步指纹，避免下一次写操作误判为"外部写入"而白读 400MB。
        *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = self.disk_fingerprint();
        Ok(())
    }

    fn load_all_memories(&self) -> Result<Vec<Memory>, PersistenceError> {
        // 读取路径必须以磁盘为事实源，避免其他进程写入后继续返回旧缓存。
        let fp_before = self.disk_fingerprint();
        let memories = self.load_all_memories_from_disk()?;
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = Some(memories.clone());
        *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = fp_before;
        Ok(memories)
    }

    /// 数据源指纹 = 基文件与**追加日志**的 `mtime`（纳秒）**长度混合**（v0.9.11 / G9 / CR-42）。
    ///
    /// 用途：让 `MemoryStore` 的外层缓存能发现"**别的进程**写了新数据"
    /// （见 `Persistence::source_fingerprint` 文档的契约与已知边界）。
    /// 取 `mtime` 而非内容哈希：本方法的**全部价值**就在于代价远低于全量读。
    /// ★ B-1：与 `disk_fingerprint` 同口径（含日志），避免"只写日志"被判为无变化。
    fn source_fingerprint(&self) -> Option<u64> {
        self.disk_fingerprint()
    }

    fn delete_memory(&self, id: &str) -> Result<bool, PersistenceError> {
        self.ensure_data_dir()?;

        let _process_guard = self.acquire_process_write_guard()?;
        // ★ B1：同上 —— 只在确有外部写入时整库重读。
        self.refresh_cache_if_changed()?;
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        // v0.9.7 修复：同上，消除库路径 panic 面（原为 .expect("缓存已通过磁盘刷新初始化")）
        let memories = cache.as_mut().ok_or_else(|| {
            PersistenceError::Other(
                "内存缓存未初始化（refresh_cache_from_disk 未生效）".to_string(),
            )
        })?;
        let original_len = memories.len();
        memories.retain(|m| m.id != id);

        if memories.len() == original_len {
            return Ok(false); // 未找到
        }

        // ★ B-1：删除必须整库重写并清空日志（否则被删条目会被日志重放"复活"）。
        //   同时去掉原来的整库深拷贝回滚——失败时丢弃缓存即可（原子写保证磁盘未变）。
        let json = serde_json::to_string_pretty(&*memories)?;
        if let Err(error) = atomic_write(&self.memories_file, &json) {
            *cache = None;
            drop(cache);
            *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = None;
            return Err(error);
        }
        self.truncate_journal();
        drop(cache);
        *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = self.disk_fingerprint();
        Ok(true)
    }

    fn clear_memories(&self) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        let _process_guard = self.acquire_process_write_guard()?;
        self.refresh_cache_if_changed()?;
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let empty: Vec<Memory> = Vec::new();
        let json = serde_json::to_string_pretty(&empty)?;
        if let Err(error) = atomic_write(&self.memories_file, &json) {
            // ★ B-1：失败时直接丢弃缓存（不再深拷贝整库做回滚）。
            *cache = None;
            drop(cache);
            *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = None;
            return Err(error);
        }
        // ★ B-1：清空后必须清空日志，否则日志重放会把整库"复活"。
        self.truncate_journal();
        *cache = Some(empty);
        drop(cache);
        *self.loaded_fp.write().unwrap_or_else(|e| e.into_inner()) = self.disk_fingerprint();
        Ok(())
    }

    fn save_chunks(&self, chunks: &[CodeChunk]) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        let _process_guard = self.acquire_process_write_guard()?;
        let all_chunks: Vec<&CodeChunk> = chunks.iter().collect();
        let json = serde_json::to_string_pretty(&all_chunks)?;
        atomic_write(&self.chunks_file, &json)?;
        Ok(())
    }

    fn load_chunks(&self) -> Result<Vec<CodeChunk>, PersistenceError> {
        if !self.chunks_file.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.chunks_file)?;
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        let chunks: Vec<CodeChunk> = serde_json::from_str(&content)?;
        Ok(chunks)
    }

    fn clear_chunks(&self) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        let _process_guard = self.acquire_process_write_guard()?;
        let empty: Vec<CodeChunk> = Vec::new();
        let json = serde_json::to_string_pretty(&empty)?;
        atomic_write(&self.chunks_file, &json)?;
        Ok(())
    }

    fn size_bytes(&self) -> Result<u64, PersistenceError> {
        let mut total: u64 = 0;

        if self.memories_file.exists() {
            total += fs::metadata(&self.memories_file)?.len();
        }

        if self.chunks_file.exists() {
            total += fs::metadata(&self.chunks_file)?.len();
        }

        if self.archive_file.exists() {
            total += fs::metadata(&self.archive_file)?.len();
        }

        // ★ B-1：追加日志同样是库体积的一部分（否则磁盘占用统计会偏低）。
        if self.journal_file.exists() {
            total += fs::metadata(&self.journal_file)?.len();
        }

        Ok(total)
    }

    fn load_archived_memories(&self) -> Result<Vec<Memory>, PersistenceError> {
        if !self.archive_file.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.archive_file)?;
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        let memories: Vec<Memory> = serde_json::from_str(&content)?;
        Ok(memories)
    }

    fn save_archived_memories(&self, memories: &[Memory]) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        let _process_guard = self.acquire_process_write_guard()?;
        write_archived_memories(&self.archive_file, memories)
    }

    fn add_to_archive(&self, memories: &[Memory]) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        let _process_guard = self.acquire_process_write_guard()?;
        let mut existing = self.load_archived_memories()?;

        // 按 ID 去重，避免重复归档
        let existing_ids: std::collections::HashSet<String> =
            existing.iter().map(|m| m.id.clone()).collect();
        for m in memories {
            if !existing_ids.contains(&m.id) {
                existing.push(m.clone());
            }
        }

        write_archived_memories(&self.archive_file, &existing)
    }

    fn delete_from_archive(&self, id: &str) -> Result<bool, PersistenceError> {
        self.ensure_data_dir()?;
        let _process_guard = self.acquire_process_write_guard()?;
        let mut archived = self.load_archived_memories()?;
        let original_len = archived.len();
        archived.retain(|m| m.id != id);

        if archived.len() == original_len {
            return Ok(false);
        }

        write_archived_memories(&self.archive_file, &archived)?;
        Ok(true)
    }
}

// === 原子写入辅助函数 ===

/// 原子**且持久**写入（v0.9.11 / G5 / CR-40：改为**委托**唯一实现）。
///
/// ## 为什么委托而不是各写一遍
/// 修前本文件自带一份"临时文件 + rename"实现，与 `atomic_file::write_atomic` 重复；
/// 两份实现意味着**加固只会做一半**（例如本轮补 fsync 时，只改一处就会漏掉主记忆写路径）。
/// 现统一委托 Layer 1 公共设施：临时名带 UUID、失败清理、**写入 fsync 后再 rename**、
/// rename 后同步父目录（Unix）—— 全部收敛到一处。
///
/// ## 锁序
/// - 锁② `JSON_WRITE_LOCK`：此处获取（调用方须已按序持有锁① `cache` 写锁）；
/// - 锁④ 快照一致性闸门：由 `atomic_file::write_atomic` 内部获取（**不在本函数重复取**，
///   否则同线程递归取读锁在"有写者在等待"时可能死锁）。
fn atomic_write(path: &Path, content: &str) -> Result<(), PersistenceError> {
    // 锁序②：见文件顶部「锁序契约」。调用方须已按序持有锁①（cache 写锁）。
    let _guard = JSON_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::atomic_file::write_atomic(path, content.as_bytes()).map_err(PersistenceError::from)
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_types::{Importance, Memory, MemoryType};
    use tempfile::TempDir;

    fn make_test_memory(id: &str, content: &str) -> Memory {
        let mut m = Memory::new(
            content.to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::default(),
            None,
        );
        // 覆盖 ID 以便测试验证
        m.id = id.to_string();
        m
    }

    #[test]
    fn test_new_creates_directory() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().join("data").to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        assert!(Path::new(&data_dir).exists());
        assert!(p
            .data_dir()
            .join("memories.json")
            .to_string_lossy()
            .contains("memories"));
    }

    // ===== P0-1 / P0-2：生命周期表必须独立于 memories.json =====

    #[test]
    fn lifecycle_roundtrip_is_separate_from_memories_json() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        let mut table = LifecycleTable::default();
        table.touch("m1", 1_000);
        p.save_lifecycle(&table).expect("应成功保存生命周期表");

        assert!(dir.path().join("lifecycle.json").exists(), "必须写独立文件");
        assert!(
            !dir.path().join("memories.json").exists(),
            "寿命状态不得顺带创建/重写基文件（回滚安全 + 防全库写）"
        );

        let p2 = JsonPersistence::new(&data_dir).expect("应成功创建第二个实例");
        let loaded = p2.load_lifecycle().expect("应成功加载");
        assert_eq!(loaded.get("m1").map(|e| e.hit_count), Some(1));
    }

    #[test]
    fn lifecycle_missing_file_is_empty_and_harmless() {
        let dir = TempDir::new().expect("应创建临时目录");
        let p = JsonPersistence::new(dir.path().to_string_lossy().as_ref()).expect("应成功创建");
        let table = p.load_lifecycle().expect("缺失文件应返回空表而不是报错");
        assert!(table.entries.is_empty(), "缺失即等价于全部 Active 且不衰减");
    }

    // ===== P0-3：日志清空失败时的作废标记必须阻止重放 =====

    #[test]
    fn invalidated_journal_is_not_replayed() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");
        p.save_memory(&make_test_memory("keep", "保留的记忆"))
            .expect("应成功保存");

        // 模拟"删除后清日志失败"：日志里残留一条本应被删掉的记忆
        let ghost = make_test_memory("ghost", "本应被删除的记忆");
        p.append_journal(std::slice::from_ref(&ghost))
            .expect("应成功追加日志");

        // 前置（负向对照）：没有标记时，日志确实会被重放
        let p2 = JsonPersistence::new(&data_dir).expect("应成功创建读实例");
        let with_replay = p2.load_all_memories().expect("应成功加载");
        assert!(
            with_replay.iter().any(|m| m.id == "ghost"),
            "前置：未作废时日志重放应带出 ghost（证明对照有效）"
        );

        // 放置作废标记后：必须跳过重放，并清理残留
        atomic_write(&p.journal_invalidated_marker(), "1").expect("应成功写标记");
        let p3 = JsonPersistence::new(&data_dir).expect("应成功创建读实例");
        let after = p3.load_all_memories().expect("应成功加载");
        assert!(
            !after.iter().any(|m| m.id == "ghost"),
            "标记存在时不得重放 —— 否则已删除的记忆会复活"
        );
        assert!(
            !p.journal_invalidated_marker().exists(),
            "清理完成后作废标记应被移除"
        );
        assert!(
            after.iter().any(|m| m.id == "keep"),
            "基文件里的记忆必须保留"
        );
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        let m = make_test_memory("mem-1", "测试记忆内容");
        p.save_memory(&m).expect("应成功保存");

        let loaded = p.load_all_memories().expect("应成功加载");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "mem-1");
        assert_eq!(loaded[0].content, "测试记忆内容");
    }

    // ── v0.9.8：状态持久化能力声明 ──
    //
    // 背景：这两个状态方法有 trait 默认实现（返回空 + 忽略保存），
    // 未实现的后端会导致"联想活性/语义吸铁石压制重启归零"且**无提示**。
    // 故此组测试既验证 JSON 后端**做到**了，也验证它**声明**了。

    /// JSON 后端必须声明支持状态持久化（否则启动时会打印降级告警）
    #[test]
    fn test_json_backend_declares_state_persistence() {
        let dir = TempDir::new().expect("应创建临时目录");
        let p = JsonPersistence::new(dir.path().to_string_lossy().as_ref()).expect("应成功创建");
        assert!(
            p.supports_state_persistence(),
            "JSON 后端已实现 load/save_memory_state 与 load/save_assoc_frequency，\
必须如实声明 true——否则用户会看到不该出现的降级告警"
        );
    }

    /// 负向对照：默认实现（未声明）必须为 false
    ///
    /// **为什么需要**：若 `supports_state_persistence` 的默认值被误改为 true，
    /// 所有未实现的后端都会**静默**降级（告警永不触发）——本测试固化默认值。
    ///
    /// 实现方式：复用已有的 `StubPersistence`（只实现必需方法，不实现任何
    /// 状态方法），从而**真实地**走 trait 默认实现，而不是断言常量。
    #[test]
    fn test_default_supports_state_persistence_is_false() {
        let p = crate::persistence::test_stub::StubPersistence;
        assert!(
            !p.supports_state_persistence(),
            "默认实现必须为 false：未实现状态持久化的后端不该声称支持，\
否则降级告警永不触发（静默功能降级会重新出现）"
        );
        // 同时确认默认实现的行为确实是"空 + 忽略"
        assert!(p
            .load_memory_state()
            .expect("默认实现不应报错")
            .active
            .is_empty());
        assert_eq!(
            p.load_assoc_frequency()
                .expect("默认实现不应报错")
                .total_queries,
            0
        );
    }

    /// 状态确实能跨实例读回（写 → 新实例 → 读到，非内存缓存假绿）
    #[test]
    fn test_memory_state_persists_across_instances() {
        use crate::memory_state_machine::{ActiveMemory, MemoryState};
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let first = JsonPersistence::new(&data_dir).expect("应创建第一个实例");

        let mut state = MemoryState::default();
        state.active.push(ActiveMemory {
            memory_id: "mem-x".to_string(),
            activation: 0.75,
            hits: 3,
            last_step: 5,
        });
        first.save_memory_state(&state).expect("应成功保存状态");

        // 新实例（不共享内存）读取
        let second = JsonPersistence::new(&data_dir).expect("应创建第二个实例");
        let loaded = second.load_memory_state().expect("应成功加载状态");
        assert_eq!(loaded.active.len(), 1, "状态必须跨实例可读回");
        assert_eq!(loaded.active[0].memory_id, "mem-x");
        assert!((loaded.active[0].activation - 0.75).abs() < 1e-6);
    }

    #[test]
    fn test_two_instances_observe_each_others_writes() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let first = JsonPersistence::new(&data_dir).expect("应创建第一个实例");
        let second = JsonPersistence::new(&data_dir).expect("应创建第二个实例");

        first
            .save_memory(&make_test_memory("first", "第一个实例写入"))
            .expect("第一个实例应写入");
        assert_eq!(second.load_all_memories().unwrap().len(), 1);

        second
            .save_memory(&make_test_memory("second", "第二个实例写入"))
            .expect("第二个实例应写入");
        let loaded = first.load_all_memories().expect("第一个实例应看到外部写入");
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().any(|memory| memory.id == "first"));
        assert!(loaded.iter().any(|memory| memory.id == "second"));
    }

    #[test]
    fn test_two_instances_archive_read_modify_write_is_serialized() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let first = JsonPersistence::new(&data_dir).expect("应创建第一个实例");
        let second = JsonPersistence::new(&data_dir).expect("应创建第二个实例");

        first
            .add_to_archive(&[make_test_memory("archive-first", "第一个归档")])
            .expect("第一个实例应归档");
        second
            .add_to_archive(&[make_test_memory("archive-second", "第二个归档")])
            .expect("第二个实例应归档");

        let archived = first.load_archived_memories().expect("应读取归档文件");
        assert_eq!(archived.len(), 2);
        assert!(archived.iter().any(|memory| memory.id == "archive-first"));
        assert!(archived.iter().any(|memory| memory.id == "archive-second"));
    }

    #[test]
    fn test_load_empty() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        let loaded = p.load_all_memories().expect("应成功加载");
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_save_failure_does_not_update_cache() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");
        let original = make_test_memory("mem-1", "原始内容");
        // 先用整体替换写出一份基文件（首次 save_memory 只会写日志）。
        p.replace_all_memories(std::slice::from_ref(&original))
            .expect("应写出基文件");

        // ★ B-1 后热路径的写目标是**追加日志** ⇒ 失败注入点随之改到日志路径：
        //   把它换成同名目录，追加必然失败。
        std::fs::create_dir(dir.path().join("memories.journal.jsonl")).expect("应创建同名目录");
        let failed = p.save_memory(&make_test_memory("mem-1", "不应进入缓存"));
        assert!(failed.is_err(), "日志追加失败必须向上报错");

        // 缓存已被丢弃；重新加载只应看到磁盘上真实存在的那一条。
        let loaded = p.load_all_memories().expect("基文件未损坏，应可加载");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].content, "原始内容");
    }

    #[test]
    fn test_corrupted_base_file_is_reported() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");
        // 用整体替换确保**基文件**存在（首次 save_memory 现在也会建基文件，
        // 但这里刻意走整体替换，避免测试依赖压实路径的具体行为）。
        p.replace_all_memories(&[make_test_memory("mem-1", "原始内容")])
            .expect("应成功写出基文件");
        // 破坏基文件：即使追加日志完好，也不应返回"看似正常"的结果。
        std::fs::remove_file(dir.path().join("memories.json")).expect("应删除文件");
        std::fs::create_dir(dir.path().join("memories.json")).expect("应创建同名目录");
        assert!(
            p.load_all_memories().is_err(),
            "磁盘损坏时不应伪造旧缓存结果"
        );
    }

    /// ★ B-1 守卫：基文件 + 追加日志必须合并加载，且**同 id 后写胜出**。
    #[test]
    fn test_journal_replay_merges_and_overrides() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");
        p.replace_all_memories(&[make_test_memory("mem-1", "基文件内容")])
            .expect("应写出基文件");
        p.save_memory(&make_test_memory("mem-2", "增量新增"))
            .expect("应追加新增");
        p.save_memory(&make_test_memory("mem-1", "增量覆盖"))
            .expect("应追加覆盖");

        // 新实例 = 模拟进程重启，只能依赖磁盘（基文件 + 日志）
        let p2 = JsonPersistence::new(&data_dir).expect("应成功创建");
        let mut loaded = p2.load_all_memories().expect("应可加载");
        loaded.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].content, "增量覆盖");
        assert_eq!(loaded[1].content, "增量新增");
    }

    /// ★ 2026-10-28 守卫：全新数据目录的首次写入必须建立基文件（`memories.json`）
    /// 并清空日志，之后才回到增量追加。
    ///
    /// 修前：首次写入只落 `memories.journal.jsonl`，基文件迟迟不存在——
    /// 既让日志成为唯一副本（崩溃安全退化），也让按契约读 `memories.json` 的路径
    /// 读不到文件（`v1_api` 两个 §3.42.5 负向对照测试因此失败）。
    #[test]
    fn test_first_write_creates_base_file_then_appends_journal() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");
        let base = dir.path().join("memories.json");
        assert!(!base.exists(), "全新目录不应存在基文件");

        p.save_memory(&make_test_memory("mem-1", "首条"))
            .expect("应成功保存");
        assert!(
            base.exists(),
            "首次写入必须建立基文件（否则日志是唯一副本）"
        );
        assert_eq!(p.journal_len(), 0, "首次压实后追加日志应为空");
        let text = std::fs::read_to_string(&base).expect("应能读取基文件");
        assert!(text.contains("首条"), "基文件必须包含刚写入的内容");

        p.save_memory(&make_test_memory("mem-2", "第二条"))
            .expect("应成功保存");
        assert!(
            p.journal_len() > 0,
            "基文件已存在时，后续写入应回到增量追加"
        );
    }

    /// ★ B-1 守卫：删除必须清空日志，否则已删条目会被重放"复活"。
    #[test]
    fn test_delete_clears_journal_without_resurrection() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");
        p.save_memory(&make_test_memory("mem-1", "待删除"))
            .expect("应成功保存");
        assert!(p.delete_memory("mem-1").expect("应成功删除"));

        let p2 = JsonPersistence::new(&data_dir).expect("应成功创建");
        assert!(p2.load_all_memories().expect("应可加载").is_empty());
    }

    #[test]
    fn test_save_update_existing() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        let m1 = make_test_memory("mem-1", "原始内容");
        p.save_memory(&m1).expect("应成功保存");

        let mut m2 = make_test_memory("mem-1", "更新后的内容");
        m2.memory_type = MemoryType::Preference;
        p.save_memory(&m2).expect("应更新成功");

        let loaded = p.load_all_memories().expect("应成功加载");
        assert_eq!(loaded.len(), 1, "不应产生重复项");
        assert_eq!(loaded[0].content, "更新后的内容");
        assert_eq!(loaded[0].memory_type, MemoryType::Preference);
    }

    #[test]
    fn test_delete_memory() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        let m = make_test_memory("mem-1", "待删除");
        p.save_memory(&m).expect("应成功保存");

        let deleted = p.delete_memory("mem-1").expect("应成功删除");
        assert!(deleted);

        let loaded = p.load_all_memories().expect("应成功加载");
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_delete_nonexistent() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        let deleted = p.delete_memory("nonexistent").expect("应正常返回");
        assert!(!deleted);
    }

    #[test]
    fn test_clear_memories() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        p.save_memory(&make_test_memory("m1", "内容1"))
            .expect("应成功保存");
        p.save_memory(&make_test_memory("m2", "内容2"))
            .expect("应成功保存");

        p.clear_memories().expect("应成功清空");

        let loaded = p.load_all_memories().expect("应成功加载");
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_chunk_persistence() {
        use crate::chunker::CodeChunk;

        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        let chunk = CodeChunk {
            id: "test.rs:L1-L5".to_string(),
            file_path: "test.rs".to_string(),
            start_line: 1,
            end_line: 5,
            chunk_type: "fn".to_string(),
            name: "test_fn".to_string(),
            signature: "fn test_fn()".to_string(),
            content: "fn test_fn() {\n    // test\n}".to_string(),
            doc_comment: None,
            language: "rust".to_string(),
        };

        p.save_chunks(&[chunk]).expect("应成功保存片段");

        let loaded = p.load_chunks().expect("应成功加载片段");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "test_fn");
        assert_eq!(loaded[0].language, "rust");
    }

    #[test]
    fn test_size_bytes() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        let m = make_test_memory("mem-1", "测试记忆");
        p.save_memory(&m).expect("应成功保存");

        let size = p.size_bytes().expect("应获取文件大小");
        assert!(size > 0);
    }

    /// v0.5.5 修复一：验证批量更新记忆的正确性
    /// 确保 update_memories 仅更新指定记忆，不触碰其他记忆
    #[test]
    fn test_update_memories_partial_update() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        // 准备 3 条记忆
        p.save_memory(&make_test_memory("m1", "内容1-原始"))
            .expect("应成功保存");
        p.save_memory(&make_test_memory("m2", "内容2-原始"))
            .expect("应成功保存");
        p.save_memory(&make_test_memory("m3", "内容3-原始"))
            .expect("应成功保存");

        // 仅更新 m1 和 m3，不触碰 m2
        let mut updated_m1 = make_test_memory("m1", "内容1-已更新");
        updated_m1.importance = crate::memory_types::Importance::new(8);
        let mut updated_m3 = make_test_memory("m3", "内容3-已更新");
        updated_m3.importance = crate::memory_types::Importance::new(9);

        p.update_memories(&[updated_m1, updated_m3])
            .expect("应成功批量更新");

        // 验证：m1 和 m3 已更新，m2 保持不变
        let loaded = p.load_all_memories().expect("应成功加载");
        assert_eq!(loaded.len(), 3, "记忆总数应保持 3 条");

        let m1 = loaded.iter().find(|m| m.id == "m1").expect("应找到 m1");
        assert_eq!(m1.content, "内容1-已更新");
        assert_eq!(m1.importance.value(), 8);

        let m2 = loaded.iter().find(|m| m.id == "m2").expect("应找到 m2");
        assert_eq!(m2.content, "内容2-原始", "m2 应保持不变");

        let m3 = loaded.iter().find(|m| m.id == "m3").expect("应找到 m3");
        assert_eq!(m3.content, "内容3-已更新");
        assert_eq!(m3.importance.value(), 9);
    }

    /// v0.5.5 修复一：验证空列表批量更新是 no-op
    #[test]
    fn test_update_memories_empty() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        p.save_memory(&make_test_memory("m1", "内容1"))
            .expect("应成功保存");

        // 空列表批量更新应成功且不修改任何记忆
        p.update_memories(&[]).expect("空列表应成功");

        let loaded = p.load_all_memories().expect("应成功加载");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].content, "内容1");
    }

    /// v0.9.0 批量保存：验证批量追加新记忆（合成场景）
    #[test]
    fn test_save_memories_batch_append() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        // 批量追加 3 条新记忆
        let batch = vec![
            make_test_memory("s1", "合成记忆1"),
            make_test_memory("s2", "合成记忆2"),
            make_test_memory("s3", "合成记忆3"),
        ];
        p.save_memories(&batch).expect("应成功批量保存");

        let loaded = p.load_all_memories().expect("应成功加载");
        assert_eq!(loaded.len(), 3, "应批量追加 3 条记忆");
        assert!(loaded.iter().any(|m| m.id == "s1"));
        assert!(loaded.iter().any(|m| m.id == "s2"));
        assert!(loaded.iter().any(|m| m.id == "s3"));
    }

    /// v0.9.0 批量保存：验证混合更新已有 + 追加新记忆
    #[test]
    fn test_save_memories_batch_mixed() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        // 预置 2 条记忆
        p.save_memory(&make_test_memory("m1", "旧内容1"))
            .expect("应成功保存");
        p.save_memory(&make_test_memory("m2", "旧内容2"))
            .expect("应成功保存");

        // 批量：更新 m1 + 追加 m3
        let mut updated_m1 = make_test_memory("m1", "新内容1");
        updated_m1.importance = crate::memory_types::Importance::new(9);
        let new_m3 = make_test_memory("m3", "新记忆3");
        p.save_memories(&[updated_m1, new_m3])
            .expect("应成功批量保存");

        let loaded = p.load_all_memories().expect("应成功加载");
        assert_eq!(loaded.len(), 3, "应更新 m1 + 追加 m3，共 3 条");

        let m1 = loaded.iter().find(|m| m.id == "m1").expect("应找到 m1");
        assert_eq!(m1.content, "新内容1");
        assert_eq!(m1.importance.value(), 9);

        let m2 = loaded.iter().find(|m| m.id == "m2").expect("应找到 m2");
        assert_eq!(m2.content, "旧内容2", "m2 应保持不变");

        assert!(loaded.iter().any(|m| m.id == "m3"), "应追加 m3");
    }

    /// v0.9.0 批量保存：验证空列表是 no-op
    #[test]
    fn test_save_memories_empty() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        p.save_memories(&[]).expect("空列表应成功");

        let loaded = p.load_all_memories().expect("应成功加载");
        assert!(loaded.is_empty());
    }
}
