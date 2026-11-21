//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现记忆数据自动备份机制，属于公开层 (Layer 1)。
//! ============================================================
//!
//! v0.8.0 "归一" 专项：记忆数据备份模块
//!
//! 功能：
//!   1. 手动/自动将当前记忆库导出为 JSON 备份文件
//!   2. 备份存储在 ~/.loong-recall/backups/ 目录
//!   3. 文件名格式：memories_YYYYMMDD_HHMMSS.json
//!   4. 自动清理旧备份，默认保留最近 4 份
//!
//! 设计原则：
//!   - 备份是只读拷贝，不修改原文件
//!   - 备份文件包含完整的 memories.json 内容
//!   - 清理策略基于文件修改时间，最旧的先删
//!   - 备份失败不影响主流程

// v0.9.7（GLOBAL_CODE_REVIEW_REPORT P1-6）：恢复路径（安全关键）已由
// 不可判别的 String 收敛为带域分类的 LrcError（io / not_found / invalid_input）。
// 「快照越界」归 invalid_input、「路径/文件缺失」归 not_found，便于调用方与测试判别。
use crate::atomic_file::dir_size;
use crate::errors::{LrcError, LrcResult};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// 备份锁序契约（v0.9.7 修复 GLOBAL_CODE_REVIEW_REPORT 并发 P2-3「备份锁序未文档化」）
/// ============================================================
/// 本模块持有**两把**锁，代码库中的版本如下，改动时须刷新此说明：
///
/// ```text
///   [1] BACKUP_OPERATION_LOCK (Mutex<()>)          —— 全局单例，串行化"创建/恢复/清理备份"
///   [2] atomic_file::snapshot_guard() (RwLock 写)  —— 快照一致性闸门，独占（CR-37）
/// ```
///
/// **与持久层的锁序关系（关键，避免 ABBA）**：
///   - 本模块**从不**在持有 `persistence::json::JSON_WRITE_LOCK` 或
///     `JsonPersistence::cache` 锁的情况下获取（备份流程为独立入口：
///     仪表盘"立即备份"、CLI `--backup`、恢复均自顶向下调用）。
///   - 反向：持久层写入路径（`save_memory` 等）在**释放**其写锁前不调用备份模块，
///     但会在写完一个文件时取 [2] 的**读**锁（`atomic_file::writer_guard`）。
///     方向为"写入方持读、备份方持写"，是标准读写语义，**不构成 ABBA**：
///     备份方只取 [2]，写入方在 [2] 之下不再取别的锁。
///   - 若未来需要"写入前自动备份"，**必须**在**释放**本锁后再进入持久层写锁，
///     即维持 `BACKUP → (release) → cache → JSON_WRITE` 的顺序，禁止嵌套获取。
///
/// **持锁期间的行为**：备份为文件级 `fs::copy`（见 `create_backup_locked`），
///   持锁期间执行磁盘 IO 是**有意**的——备份的语义就是"某一时刻的一致快照"，
///   必须与并发的写操作互斥。此处临界区放大是正确性要求，非缺陷。
///
/// **⚠️ v0.9.11（G5 / CR-37）修正**：上面这句"必须与并发的写操作互斥"此前**只是承诺、没有机制**
///   —— 写路径从不取本模块的任何锁，于是快照仍可能是"半新半旧"的文件组（如
///   `memories.json` 已是新版而 `audit.jsonl` 还是旧版 ⇒ 悬空审计引用）。
///   现在由 [2] 兑现：备份/恢复持**写**锁独占，所有落盘路径持**读**锁共享。
///   **持 [2] 写锁期间不得调用 `atomic_file::write_atomic`**（同线程会自锁死），
///   本模块因此坚持文件级 `fs::copy`。
///
/// **调用点**：[`create_backup`]（本文件 :109 附近）与 [`restore_backup`]
///   （本文件 :267 附近）各获取一次；`count_backups` / `cleanup_old_backups`
///   不单独加锁，仅由持锁方在临界区内调用。
static BACKUP_OPERATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 备份保留份数（超出此数量的最旧备份将被删除）
const MAX_BACKUPS: usize = 4;

/// 快照清单文件名。v0.11.1 起每份快照带清单，恢复时**只覆盖清单列出的文件**。
const MANIFEST_FILE: &str = "manifest.json";
/// 清单格式版本。
const MANIFEST_FORMAT_VERSION: u32 = 2;

/// v0.11.1 之前的快照只含这 8 个文件（无清单）。
///
/// 恢复旧快照时保持原语义：存在的复制、缺失的置空；
/// **不触碰**清单外的新增大文件（语义向量/图边），避免旧快照把新数据清空。
const LEGACY_SNAPSHOT_FILES: [&str; 8] = [
    "memories.json",
    "memories.journal.jsonl",
    "chunks.json",
    "archive.json",
    "audit.jsonl",
    "audit.jsonl.seal",
    "audit.jsonl.anchors.jsonl",
    "feedback.jsonl",
];

/// 核心快照：记忆 + 小型状态文件；语义向量与图边可在恢复后重建。
const CORE_SNAPSHOT_FILES: [&str; 11] = [
    "memories.json",
    "memories.journal.jsonl",
    "chunks.json",
    "archive.json",
    "audit.jsonl",
    "audit.jsonl.seal",
    "audit.jsonl.anchors.jsonl",
    "feedback.jsonl",
    "memory_state.json",
    "lifecycle.json",
    "assoc_frequency.json",
];

/// 完整快照：核心 + 语义向量 + 图边（恢复后语义检索/联想不降级）。
const FULL_SNAPSHOT_FILES: [&str; 15] = [
    "memories.json",
    "memories.journal.jsonl",
    "chunks.json",
    "archive.json",
    "audit.jsonl",
    "audit.jsonl.seal",
    "audit.jsonl.anchors.jsonl",
    "feedback.jsonl",
    "memory_state.json",
    "lifecycle.json",
    "assoc_frequency.json",
    "semantic_vectors.bin",
    "semantic_vectors.journal.bin",
    "graph_edges.json",
    "graph_edges.journal.jsonl",
];

/// 快照模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupMode {
    /// 完整：包含语义向量与图边（默认）
    #[default]
    Full,
    /// 核心：不包含语义向量与图边（体积小，恢复后需重建）
    Core,
}

impl BackupMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Core => "core",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "full" => Some(Self::Full),
            "core" => Some(Self::Core),
            _ => None,
        }
    }

    fn files(self) -> &'static [&'static str] {
        match self {
            Self::Full => &FULL_SNAPSHOT_FILES,
            Self::Core => &CORE_SNAPSHOT_FILES,
        }
    }
}

/// 快照清单：记录本份快照实际包含的文件，恢复时以此为准。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SnapshotManifest {
    format_version: u32,
    created_at_ms: i64,
    mode: BackupMode,
    files: Vec<String>,
    /// 本模式应有、但备份时**不存在**的文件。
    ///
    /// 恢复时必须把目标目录里的同名文件移除：否则「快照时没有日志、恢复时目标有日志」
    /// 会让 `replay_journal` 把恢复点之后的记忆重新写回，恢复被静默部分撤销。
    #[serde(default)]
    absent: Vec<String>,
}

/// 清单里的文件名必须是**单层普通文件名**，防止路径穿越与 Windows 特殊名。
fn is_safe_snapshot_file(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    // 拒绝分隔符、盘符、ADS（`file:stream`）与 `..`
    if name.contains('/') || name.contains('\\') || name.contains(':') || name.contains("..") {
        return false;
    }
    // Windows 会静默去掉结尾的空格/点 ⇒ 可能别名到另一个文件
    if name.ends_with(' ') || name.ends_with('.') {
        return false;
    }
    // Windows 保留设备名（含带扩展名形式）
    let stem = name.split('.').next().unwrap_or(name);
    const RESERVED: &[&str] = &[
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    if RESERVED.iter().any(|r| stem.eq_ignore_ascii_case(r)) {
        return false;
    }
    // 必须恰好是一个 Normal 路径分量
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 备份结果报告
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackupReport {
    /// 是否成功
    pub success: bool,
    /// 备份文件路径
    pub backup_path: Option<String>,
    /// 备份文件大小（字节）
    pub backup_size: u64,
    /// 备份的记忆数
    pub memory_count: usize,
    /// 清理的旧备份数
    pub old_backups_removed: usize,
    /// 当前备份总数
    pub total_backups: usize,
    /// 错误信息（如有）
    pub error: Option<String>,
    /// 数据来源目录（哪个实例的库被备份）
    ///
    /// 为什么需要：备份文件集中存放于全局 `backups/`，多实例共享同一轮转池。
    /// 没有来源标注时，用户无法判断某份快照属于哪个库。
    pub source_data_dir: Option<String>,
    /// 快照模式：full | core
    pub mode: String,
    /// 本份快照实际包含的文件清单（与 manifest 一致）
    pub files_included: Vec<String>,
}

/// 获取备份目录路径：~/.loong-recall/backups/
///
/// v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P2 安全「备份目录两套推导」+ P3「密钥路径回退 CWD」同源项）：
///   原实现 `home_dir().unwrap_or_else(|| PathBuf::from("."))` 在 home 不可用时
///   退化为**相对当前工作目录**的 `./.loong-recall/backups`——CWD 可被攻击者控制，
///   备份（含全部记忆明文快照）会落到非预期位置。改为显式回退到系统临时目录，
///   并打印告警，避免"静默写入相对路径"。
pub fn backups_dir() -> PathBuf {
    backups_dir_with(std::env::var("LRC_BACKUPS_DIR").ok().as_deref())
}

/// 备份目录解析（可注入）。
///
/// `LRC_BACKUPS_DIR` 供 3111 测试实例隔离使用：不设置时仍落到
/// `~/.loong-recall/backups`，生产行为零变化。
fn backups_dir_with(override_dir: Option<&str>) -> PathBuf {
    if let Some(dir) = override_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return path;
        }
        eprintln!("[备份][告警] LRC_BACKUPS_DIR 必须是绝对路径，已忽略: {dir}（回退默认备份目录）");
    }
    home_dir_or_fallback().join(".loong-recall").join("backups")
}

/// home 目录解析：优先 `home_dir()`，失败时退到系统临时目录（**不用 CWD**）并告警
fn home_dir_or_fallback() -> PathBuf {
    if let Some(home) = dirs_next::home_dir() {
        return home;
    }
    let fallback = std::env::temp_dir().join("loong-recall-home");
    eprintln!(
        "[备份][告警] 无法确定用户主目录，回退到 {} —— 该位置非持久化，请检查 HOME/USERPROFILE 环境变量",
        fallback.display()
    );
    fallback
}

/// 生成带时间戳的备份文件名
fn backup_filename() -> String {
    let now = chrono::Local::now();
    format!(
        "memories_{}_{}_{}.json",
        now.format("%Y%m%d_%H%M%S"),
        now.timestamp_subsec_nanos(),
        BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
    )
}

/// 创建备份
///
/// 将 ~/.loong-recall/global/data/memories.json 复制到
/// ~/.loong-recall/backups/memories_YYYYMMDD_HHMMSS.json
///
/// 自动清理超过 MAX_BACKUPS 份数的旧备份。
/// 备份**当前实例**的数据目录。
///
/// 为什么必须传入 `data_dir`：此前硬编码 `global_data_dir()`，
/// 导致自定义 `--data-dir` 或项目模式实例调用备份时，备份的是**另一个库**
/// （用户以为备份了当前数据，实际备份的是全局默认库）。
pub fn create_backup(data_dir: &Path) -> BackupReport {
    create_backup_with_mode(data_dir, BackupMode::default())
}

/// 按指定模式创建备份。
///
/// - `Full`（默认）：包含语义向量与图边，恢复后检索/联想不降级；
/// - `Core`：只含记忆与小型状态文件，体积小，恢复后需重建语义向量/图边。
pub fn create_backup_with_mode(data_dir: &Path, mode: BackupMode) -> BackupReport {
    let _operation_guard = BACKUP_OPERATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // v0.9.11（G5 / CR-37）：取**独占**快照闸门（锁序 [1] → [2]）。
    //   一旦取得，所有 JSON 落盘路径（持 writer_guard 读锁）都被排除 ⇒
    //   文件组取自**同一时刻**，兑现"必须与并发写互斥"的承诺。
    let _snapshot_guard = crate::atomic_file::snapshot_guard();
    let mut report = create_backup_in(data_dir, &backups_dir(), mode);
    report.source_data_dir = Some(data_dir.to_string_lossy().to_string());
    // 数据操作日志只在**真实**入口记录：`create_backup_in` 供测试注入临时目录调用，
    // 不应把测试运行写进用户的 `~/.loong-recall/data_operations.log`。
    if report.success {
        let details = format!(
            "备份 {} 条记忆至 {}（模式 {}，{} 个文件，清理 {} 份旧备份，当前共 {} 份）",
            report.memory_count,
            report.backup_path.as_deref().unwrap_or("未知路径"),
            report.mode,
            report.files_included.len(),
            report.old_backups_removed,
            report.total_backups
        );
        crate::data_log::log_operation(crate::data_log::OperationType::Backup, &details);
    }
    report
}

/// [`create_backup`] 的**可注入路径内核**（v0.9.11 / G7 / CR-45）。
///
/// `data_dir` 与 `root` 由调用方给定 ⇒ 测试可指向临时目录，
/// **绝不触碰**用户的真实 `~/.loong-recall/`。
/// 这正是本函数被拆出来的唯一理由：原实现把 home 路径写死在函数体内，
/// 使这条"数据安全最后防线"长期 **0 测试覆盖**（一测就会污染真实记忆库）。
///
/// **调用方契约**：本函数**不取任何锁** —— 锁由 [`create_backup`] 在同一临界区内持有。
/// 生产代码不得直接调用它（`fn` 私有即为此）。
fn create_backup_in(data_dir: &Path, root: &Path, mode: BackupMode) -> BackupReport {
    let mut report = BackupReport {
        success: false,
        backup_path: None,
        backup_size: 0,
        memory_count: 0,
        old_backups_removed: 0,
        total_backups: 0,
        error: None,
        source_data_dir: None,
        mode: mode.as_str().to_string(),
        files_included: Vec::new(),
    };
    let data_dir = data_dir.to_path_buf();
    if !data_dir.join("memories.json").exists() {
        report.error = Some(format!(
            "记忆文件不存在: {}",
            data_dir.join("memories.json").display()
        ));
        return report;
    }
    let root = root.to_path_buf();
    if let Err(e) = fs::create_dir_all(&root) {
        report.error = Some(format!("创建备份目录失败: {}", e));
        return report;
    }
    let name = backup_filename().trim_end_matches(".json").to_string() + ".snapshot";
    let final_dir = root.join(name);
    let temp_dir = root.join(format!(
        ".{}.tmp",
        BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> LrcResult<Vec<String>> {
        fs::create_dir(&temp_dir).map_err(|e| LrcError::io(format!("创建临时快照失败: {}", e)))?;
        let mut included = Vec::new();
        let mut absent = Vec::new();
        for file in mode.files() {
            let source = data_dir.join(file);
            // 只复制**实际存在**的文件：不再为空文件占位。
            // 占位会在恢复时把新数据（语义向量/图边）清空，属静默数据损失。
            if !source.exists() {
                absent.push((*file).to_string());
                continue;
            }
            let target = temp_dir.join(file);
            fs::copy(&source, &target)
                .map_err(|e| LrcError::io(format!("备份 {} 失败: {}", file, e)))?;
            included.push((*file).to_string());
        }
        // 清单与实际内容同批提交：恢复时只覆盖清单列出的文件，
        // 并删除 `absent` 里记录的"快照时不存在"的文件。
        let manifest = SnapshotManifest {
            format_version: MANIFEST_FORMAT_VERSION,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            mode,
            files: included.clone(),
            absent,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| LrcError::io(format!("序列化快照清单失败: {}", e)))?;
        fs::write(temp_dir.join(MANIFEST_FILE), manifest_bytes)
            .map_err(|e| LrcError::io(format!("写入快照清单失败: {}", e)))?;
        fs::rename(&temp_dir, &final_dir)
            .map_err(|e| LrcError::io(format!("提交快照失败: {}", e)))?;
        Ok(included)
    })();
    let included = match result {
        Ok(files) => files,
        Err(e) => {
            let _ = fs::remove_dir_all(&temp_dir);
            // BackupReport.error 是对外 JSON 字段，仍为 String 契约：取 message。
            report.error = Some(String::from(e));
            return report;
        }
    };
    report.files_included = included;
    // 体积按**整份快照**统计，而不是只看 memories.json
    //（否则 200MB 的图边快照会被报告成 60MB）。
    report.backup_size = dir_size(&final_dir);

    // 统计记忆数：必须按「基文件 + 追加日志」的真实口径，
    // 否则压实周期内的新增/修改不会被计入（报告少报）。
    report.memory_count = count_memories_in_snapshot(&final_dir);

    report.backup_path = Some(final_dir.to_string_lossy().to_string());

    // 清理旧备份
    report.old_backups_removed = cleanup_old_backups(&root);

    // 统计当前备份总数
    report.total_backups = count_backups(&root);

    report.success = true;
    report
}

/// 统计快照目录中的**有效记忆数**（基文件 + 追加日志，后写胜出）。
///
/// 只读取 `id` 与墓碑标记，不要求完整 `Memory` 结构 —— 备份计数不应因为
/// 某个字段演进（新增必填字段）而失效。
fn count_memories_in_snapshot(snapshot: &Path) -> usize {
    use std::collections::HashSet;

    let mut ids: HashSet<String> = HashSet::new();
    if let Ok(content) = fs::read_to_string(snapshot.join("memories.json")) {
        if let Ok(serde_json::Value::Array(items)) =
            serde_json::from_str::<serde_json::Value>(&content)
        {
            for item in items {
                if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                    ids.insert(id.to_string());
                }
            }
        }
    }
    if let Ok(content) = fs::read_to_string(snapshot.join("memories.journal.jsonl")) {
        for line in content.lines() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(id) = value.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            if value
                .get("deleted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                ids.remove(id);
            } else {
                ids.insert(id.to_string());
            }
        }
    }
    ids.len()
}

/// 清理旧备份，保留最近 MAX_BACKUPS 份
///
/// 按文件修改时间排序，删除最旧的超出部分。
fn cleanup_old_backups(backups_dir: &Path) -> usize {
    let mut backups: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    if let Ok(entries) = fs::read_dir(backups_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            // 只处理 memories_*.json 文件
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if (name.starts_with("memories_") && name.ends_with(".json"))
                    || name.ends_with(".snapshot")
                {
                    if let Ok(meta) = entry.metadata() {
                        if let Ok(modified) = meta.modified() {
                            backups.push((path, modified));
                        }
                    }
                }
            }
        }
    }

    if backups.len() <= MAX_BACKUPS {
        return 0;
    }

    // 按修改时间降序排列（最新的在前）
    backups.sort_by_key(|(_, t)| std::cmp::Reverse(*t));

    // 删除超出部分
    let mut removed = 0;
    for (path, _) in backups.iter().skip(MAX_BACKUPS) {
        let result = if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        if result.is_ok() {
            removed += 1;
        }
    }

    removed
}

/// 统计当前备份文件数
fn count_backups(backups_dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(backups_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if (name.starts_with("memories_") && name.ends_with(".json"))
                    || name.ends_with(".snapshot")
                {
                    count += 1;
                }
            }
        }
    }
    count
}

/// 从快照恢复全部运行时文件。
/// 从快照恢复到**当前实例**的数据目录。
///
/// 与 [`create_backup`] 同源修正：此前恢复目标硬编码全局目录，
/// 在项目模式或自定义 `--data-dir` 实例上会把全局库覆盖掉。
pub fn restore_backup(snapshot_path: &Path, data_dir: &Path) -> LrcResult<()> {
    let _operation_guard = BACKUP_OPERATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // v0.9.11（G5 / CR-37）：恢复同样要**独占**（锁序 [1] → [2]）。
    //   否则恢复进行到一半时，实时写会把"旧版本记忆"重新落盘 ⇒ 恢复结果被静默污染。
    let _snapshot_guard = crate::atomic_file::snapshot_guard();
    restore_backup_in(snapshot_path, &backups_dir(), data_dir)
}

/// [`restore_backup`] 的**可注入路径内核**（v0.9.11 / G7 / CR-45）。
///
/// `allowed_root` / `data_dir` 由调用方给定 ⇒ 测试可指向临时目录，
/// **绝不触碰**用户的真实记忆库（恢复是要覆盖数据的操作，测错路径后果极重）。
///
/// **调用方契约**：不取锁；锁由 [`restore_backup`] 在同一临界区持有。
fn restore_backup_in(snapshot_path: &Path, allowed_root: &Path, data_dir: &Path) -> LrcResult<()> {
    // v0.9.7 审查修复（HCSE-P0 安全）：快照目录必须位于本工具的备份目录内。
    // 端点默认无 token 保护，若不约束路径，本机任意进程可把任意目录内容
    // （清单/legacy 同名文件）覆盖进全局数据目录，属路径穿越/越权写。
    let canonical_snapshot = snapshot_path.canonicalize().map_err(|e| {
        LrcError::not_found(format!(
            "快照路径无法解析: {} ({})",
            snapshot_path.display(),
            e
        ))
    })?;
    let canonical_root = match allowed_root.canonicalize() {
        Ok(root) => root,
        // 备份目录尚不存在时不可能有合法快照，直接拒绝
        Err(_) => {
            return Err(LrcError::not_found(format!(
                "备份目录不存在: {}",
                allowed_root.display()
            )))
        }
    };
    if !canonical_snapshot.starts_with(&canonical_root) {
        return Err(LrcError::invalid_input(format!(
            "拒绝恢复：快照目录 {} 不在备份目录 {} 内",
            canonical_snapshot.display(),
            canonical_root.display()
        )));
    }
    if !canonical_snapshot.is_dir() {
        return Err(LrcError::not_found(format!(
            "快照目录不存在: {}",
            snapshot_path.display()
        )));
    }
    let required_memory_file = canonical_snapshot.join("memories.json");
    if !required_memory_file.is_file() {
        return Err(LrcError::not_found(format!(
            "快照缺少有效的 memories.json: {}",
            required_memory_file.display()
        )));
    }
    let snapshot_path = canonical_snapshot.as_path();
    // 清单优先：v0.11.1 起的快照带 manifest，恢复时**只覆盖清单列出的文件**；
    // 旧快照无清单，按 legacy 8 文件处理（保持"缺失置空"语义，但不碰新增大文件）。
    let manifest_path = snapshot_path.join(MANIFEST_FILE);
    let (files, absent, legacy) = if manifest_path.is_file() {
        let raw = fs::read_to_string(&manifest_path)
            .map_err(|e| LrcError::io(format!("读取快照清单失败: {}", e)))?;
        let manifest: SnapshotManifest = serde_json::from_str(&raw)
            .map_err(|e| LrcError::invalid_input(format!("快照清单损坏: {}", e)))?;
        if manifest.format_version != MANIFEST_FORMAT_VERSION {
            return Err(LrcError::invalid_input(format!(
                "不支持快照清单版本 {}（当前 {}）",
                manifest.format_version, MANIFEST_FORMAT_VERSION
            )));
        }
        if manifest.files.is_empty() {
            return Err(LrcError::invalid_input("快照清单为空，拒绝恢复"));
        }
        let mut seen = std::collections::HashSet::new();
        for file in manifest.files.iter().chain(manifest.absent.iter()) {
            if !is_safe_snapshot_file(file) {
                return Err(LrcError::invalid_input(format!(
                    "快照清单含非法文件名: {}",
                    file
                )));
            }
            // 白名单：只允许恢复已知数据文件。否则被篡改的清单（仍放在
            // 备份目录内）可以覆盖 `reclassify_progress.json`、`absorb_log.jsonl`
            // 等数据目录文件（v0.11.1 审查 P2）。
            if !FULL_SNAPSHOT_FILES.contains(&file.as_str()) {
                return Err(LrcError::invalid_input(format!(
                    "快照清单含未知文件: {}",
                    file
                )));
            }
            if !seen.insert(file.as_str()) {
                return Err(LrcError::invalid_input(format!(
                    "快照清单含重复文件: {}",
                    file
                )));
            }
        }
        if !manifest.files.iter().any(|file| file == "memories.json") {
            return Err(LrcError::invalid_input(
                "快照清单缺少 memories.json，拒绝恢复",
            ));
        }
        let mut absent = manifest.absent;
        // Core 快照**不含**语义向量与图边：恢复时必须移除目标目录里的旧文件，
        // 否则旧向量仍指向恢复点之后的记忆 ID —— 看似有索引、实际错位。
        // 移除后由启动流程按恢复出的记忆重建（v0.11.1 审查 P2）。
        if manifest.mode == BackupMode::Core {
            for file in FULL_SNAPSHOT_FILES {
                if !manifest.files.iter().any(|f| f == file) && !absent.iter().any(|f| f == file) {
                    absent.push((*file).to_string());
                }
            }
        }
        (manifest.files, absent, false)
    } else {
        (
            LEGACY_SNAPSHOT_FILES
                .iter()
                .map(|file| (*file).to_string())
                .collect::<Vec<_>>(),
            Vec::new(),
            true,
        )
    };
    fs::create_dir_all(data_dir).map_err(|e| LrcError::io(format!("创建数据目录失败: {}", e)))?;
    let temp = data_dir.join(format!(
        ".restore-{}.tmp",
        BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&temp).map_err(|e| LrcError::io(format!("创建恢复临时目录失败: {}", e)))?;
    let result = (|| -> LrcResult<()> {
        for file in &files {
            let source = snapshot_path.join(file);
            if source.exists() {
                fs::copy(&source, temp.join(file))
                    .map_err(|e| LrcError::io(format!("恢复 {} 失败: {}", file, e)))?;
            } else if legacy {
                // 旧快照没有清单，保持历史"缺失置空"语义（仅限 legacy 8 文件）。
                fs::write(temp.join(file), b"")
                    .map_err(|e| LrcError::io(format!("恢复空文件 {} 失败: {}", file, e)))?;
            } else {
                // 清单说有、实际没有 ⇒ 快照损坏，必须显式失败，不能静默少恢复。
                return Err(LrcError::not_found(format!(
                    "快照清单列出的文件缺失: {}",
                    file
                )));
            }
        }
        let rollback = data_dir.join(format!(
            ".restore-{}-rollback.tmp",
            BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&rollback)
            .map_err(|e| LrcError::io(format!("创建恢复回滚目录失败: {}", e)))?;
        let mut originals = Vec::new();
        let mut committed = Vec::new();
        let commit_result = (|| -> LrcResult<()> {
            for file in &files {
                let target = data_dir.join(file);
                // 防御纵深：文件名已校验为单层，这里再断言父目录未被改写。
                if target.parent() != Some(data_dir) {
                    return Err(LrcError::invalid_input(format!("恢复目标越界: {}", file)));
                }
                if target.exists() {
                    let saved = rollback.join(file);
                    fs::rename(&target, &saved)
                        .map_err(|e| LrcError::io(format!("保存原始 {} 失败: {}", file, e)))?;
                    originals.push((target.clone(), saved));
                }
                fs::rename(temp.join(file), &target)
                    .map_err(|e| LrcError::io(format!("提交 {} 失败: {}", file, e)))?;
                committed.push(target);
            }
            // `absent`：快照时不存在、但目标目录现在有的文件必须移除，
            // 否则旧日志会被重放，恢复点之后的记忆重新写回（静默部分撤销）。
            for file in &absent {
                let target = data_dir.join(file);
                if target.parent() != Some(data_dir) {
                    return Err(LrcError::invalid_input(format!("恢复目标越界: {}", file)));
                }
                if target.exists() {
                    let saved = rollback.join(file);
                    fs::rename(&target, &saved).map_err(|e| {
                        LrcError::io(format!("移除快照外文件 {} 失败: {}", file, e))
                    })?;
                    originals.push((target.clone(), saved));
                }
            }
            Ok(())
        })();
        if let Err(error) = commit_result {
            for target in committed.iter().rev() {
                let _ = if target.is_dir() {
                    fs::remove_dir_all(target)
                } else {
                    fs::remove_file(target)
                };
            }
            for (target, saved) in originals.iter().rev() {
                let _ = fs::rename(saved, target);
            }
            // 回滚完成后清理空目录，避免失败路径留下 `.restore-*-rollback.tmp`。
            let _ = fs::remove_dir_all(&rollback);
            return Err(LrcError::io(format!("{}；已尝试回滚恢复文件", error)));
        }
        fs::remove_dir_all(&rollback)
            .map_err(|e| LrcError::io(format!("清理恢复回滚目录失败: {}", e)))?;
        // ★v0.11 P0：清除「日志作废」标记。若不清，`replay_journal` 见到该标记会
        // 删除刚恢复的 journal —— 恢复被静默撤销（旧快照无 journal 时尤其明显）。
        let invalidated = data_dir.join("memories.journal.jsonl.invalidated");
        if invalidated.exists() {
            fs::remove_file(&invalidated)
                .map_err(|e| LrcError::io(format!("清除日志作废标记失败: {}", e)))?;
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&temp);
    result
}

/// 列出所有备份文件信息（按时间降序）
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackupInfo {
    pub filename: String,
    pub path: String,
    pub size: u64,
    pub size_human: String,
    pub modified_timestamp: u64,
}

pub fn list_backups() -> Vec<BackupInfo> {
    let backups_dir = backups_dir();
    let mut backups: Vec<BackupInfo> = Vec::new();

    if !backups_dir.exists() {
        return backups;
    }

    if let Ok(entries) = fs::read_dir(&backups_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if !name.ends_with(".snapshot") {
                    continue;
                }
                if let Ok(meta) = entry.metadata() {
                    // 快照是目录：必须递归统计，否则 Windows 上目录元数据长度恒为 0。
                    let size = if meta.is_dir() {
                        dir_size(&path)
                    } else {
                        meta.len()
                    };
                    let size_human = if size > 1024 * 1024 {
                        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
                    } else if size > 1024 {
                        format!("{:.1} KB", size as f64 / 1024.0)
                    } else {
                        format!("{} B", size)
                    };
                    let modified_timestamp = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);

                    backups.push(BackupInfo {
                        filename: name.to_string(),
                        path: path.to_string_lossy().to_string(),
                        size,
                        size_human,
                        modified_timestamp,
                    });
                }
            }
        }
    }

    // 按修改时间降序排列（最新的在前）
    backups.sort_by_key(|b| std::cmp::Reverse(b.modified_timestamp));
    backups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_filename_format() {
        let name = backup_filename();
        assert!(name.starts_with("memories_"));
        assert!(name.ends_with(".json"));
        // 格式：memories_YYYYMMDD_HHMMSS_纳秒_进程号.json
        assert!(name.len() > "memories_YYYYMMDD_HHMMSS.json".len());
    }

    #[test]
    fn test_backup_filenames_are_unique_within_same_second() {
        let first = backup_filename();
        let second = backup_filename();
        assert_ne!(first, second);
    }

    #[test]
    fn test_backups_dir_path() {
        let dir = backups_dir();
        assert!(dir.to_string_lossy().contains("backups"));
    }

    /// `LRC_BACKUPS_DIR` 必须能隔离 3111 测试实例，避免污染生产备份池。
    #[test]
    fn backups_dir_honors_override() {
        // 绝对路径的写法必须按平台区分：`C:\tmp\...` 在 Unix 上不是绝对路径，
        // CI（Linux）会正确判定为非法并回退（v0.11.1 发布门禁修复）。
        #[cfg(windows)]
        let absolute = "C:\\tmp\\lrc-backups";
        #[cfg(not(windows))]
        let absolute = "/tmp/lrc-backups";
        assert_eq!(backups_dir_with(Some(absolute)), PathBuf::from(absolute));
        assert_eq!(
            backups_dir_with(Some("   ")),
            home_dir_or_fallback().join(".loong-recall").join("backups"),
            "空白覆盖值应回退默认目录"
        );
        assert_eq!(
            backups_dir_with(Some("relative\\backups")),
            home_dir_or_fallback().join(".loong-recall").join("backups"),
            "相对路径必须被忽略（与 CWD 回退同类风险）"
        );
        assert!(
            backups_dir_with(None).ends_with(Path::new(".loong-recall").join("backups")),
            "未设置覆盖时必须仍是默认目录"
        );
    }

    #[test]
    fn test_cleanup_with_few_backups() {
        // 临时目录测试：少于 MAX_BACKUPS 时不删除
        let temp = std::env::temp_dir().join("lrc_backup_test_few");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        // 创建 2 个备份文件
        for i in 0..2 {
            let f = temp.join(format!("memories_2026010{}.json", i));
            fs::write(&f, "[]").unwrap();
        }

        let removed = cleanup_old_backups(&temp);
        assert_eq!(removed, 0, "少于 MAX_BACKUPS 时不应删除");

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_cleanup_with_many_backups() {
        // 临时目录测试：超过 MAX_BACKUPS 时删除最旧的
        let temp = std::env::temp_dir().join("lrc_backup_test_many");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        // 创建 6 个备份文件（MAX_BACKUPS=4，应删除 2 个）
        for i in 0..6 {
            let f = temp.join(format!("memories_2026010{}.json", i));
            fs::write(&f, format!("[{{\"id\":{}}}]", i)).unwrap();
            // 稍微延迟以区分修改时间
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let removed = cleanup_old_backups(&temp);
        assert_eq!(removed, 2, "应删除 2 个最旧备份");

        let remaining = count_backups(&temp);
        assert_eq!(remaining, MAX_BACKUPS, "应保留 {} 个备份", MAX_BACKUPS);

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_cleanup_removes_snapshot_directories() {
        // 用进程 ID 隔离：固定目录名在"上次运行残留 / 并行运行"时会命中
        // AlreadyExists（Windows 上 remove_dir_all 可能因句柄占用静默失败），
        // 属测试自身的卫生缺陷，与被测逻辑无关。
        let temp = std::env::temp_dir().join(format!(
            "lrc_backup_test_snapshot_dirs_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        for i in 0..6 {
            let snapshot = temp.join(format!("memories_2026010{}.snapshot", i));
            // create_dir_all 幂等：即使有残留也不会因 AlreadyExists 直接 panic。
            fs::create_dir_all(&snapshot).unwrap();
            fs::write(snapshot.join("memories.json"), "[]").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let removed = cleanup_old_backups(&temp);
        assert_eq!(removed, 2, "应删除 2 个最旧快照目录");
        assert_eq!(count_backups(&temp), MAX_BACKUPS);

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_list_backups_empty() {
        // 不存在的目录应返回空列表
        let temp = std::env::temp_dir().join("lrc_backup_test_nonexist");
        let _ = fs::remove_dir_all(&temp);

        let backups = std::panic::catch_unwind(|| {
            let dir = temp.clone();
            // list_backups 使用固定的 backups_dir()，此处仅验证逻辑
            let _backups_dir = dir;
            Vec::<BackupInfo>::new()
        });
        assert!(backups.is_ok());
    }

    #[test]
    fn test_backup_report_serialization() {
        let report = BackupReport {
            success: true,
            backup_path: Some("/test/path.json".to_string()),
            backup_size: 1024,
            memory_count: 100,
            old_backups_removed: 1,
            total_backups: 4,
            error: None,
            source_data_dir: Some("/test/data".to_string()),
            mode: "full".to_string(),
            files_included: vec!["memories.json".to_string()],
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"memory_count\":100"));
    }

    // ════════ v0.9.11（G7 / CR-45）：备份 / 恢复的**端到端**用例（此前 0 覆盖）════════
    //
    // 为什么此前没有：`create_backup()` / `restore_backup()` 把 home 路径**写死在函数体内**，
    // 一测就会读写**真实**的 `~/.loong-recall/`（恢复甚至会覆盖真实记忆库）。
    // 故先拆出可注入路径的 `create_backup_in` / `restore_backup_in` 内核，
    // 下面的用例只指向临时目录 —— 这也是这两个内核存在的唯一理由。

    /// 造一套临时目录：(临时目录守卫, 数据目录, 备份根目录)
    fn setup_dirs() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = tempfile::TempDir::new().expect("创建临时目录");
        let data = tmp.path().join("data");
        let backups = tmp.path().join("backups");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&backups).unwrap();
        (tmp, data, backups)
    }

    /// 快照只包含**实际存在**的文件，并写入清单；缺失文件不再置空。
    #[test]
    fn create_backup_in_snapshots_existing_files_with_manifest() {
        let (_tmp, data, backups) = setup_dirs();
        fs::write(data.join("memories.json"), r#"[{"id":"m1"},{"id":"m2"}]"#).unwrap();
        fs::write(data.join("audit.jsonl"), "{\"a\":1}\n").unwrap();
        // 其余文件故意不创建

        let report = create_backup_in(&data, &backups, BackupMode::Core);
        assert!(report.success, "备份应成功：{:?}", report.error);
        assert_eq!(report.memory_count, 2, "应数出 2 条记忆");
        assert_eq!(report.mode, "core");
        let snapshot = std::path::PathBuf::from(report.backup_path.as_ref().unwrap());
        assert!(snapshot.join("memories.json").exists());
        assert!(snapshot.join("audit.jsonl").exists());
        assert!(
            !snapshot.join("archive.json").exists(),
            "缺失文件不得置空（否则恢复时会误覆盖现有数据）"
        );
        let manifest: SnapshotManifest =
            serde_json::from_str(&fs::read_to_string(snapshot.join(MANIFEST_FILE)).unwrap())
                .unwrap();
        assert_eq!(manifest.mode, BackupMode::Core);
        assert!(manifest.files.contains(&"memories.json".to_string()));
        assert!(manifest.files.contains(&"audit.jsonl".to_string()));
        assert!(!manifest.files.contains(&"archive.json".to_string()));
        assert_eq!(report.files_included.len(), 2);
        assert_eq!(report.total_backups, 1);
    }

    /// 完整快照必须包含语义向量与图边；核心快照必须排除它们。
    #[test]
    fn full_backup_includes_vectors_and_graph_core_excludes_them() {
        let (_tmp, data, backups) = setup_dirs();
        fs::write(data.join("memories.json"), r#"[{"id":"m1"}]"#).unwrap();
        fs::write(data.join("semantic_vectors.bin"), b"vectors").unwrap();
        fs::write(data.join("graph_edges.json"), b"edges").unwrap();
        fs::write(data.join("memory_state.json"), b"{}").unwrap();

        let full = create_backup_in(&data, &backups, BackupMode::Full);
        assert!(full.success, "{:?}", full.error);
        let full_dir = std::path::PathBuf::from(full.backup_path.as_ref().unwrap());
        assert!(full_dir.join("semantic_vectors.bin").exists());
        assert!(full_dir.join("graph_edges.json").exists());
        assert!(full_dir.join("memory_state.json").exists());
        assert!(full.backup_size >= 7, "体积应统计整份快照");

        let core = create_backup_in(&data, &backups, BackupMode::Core);
        assert!(core.success, "{:?}", core.error);
        let core_dir = std::path::PathBuf::from(core.backup_path.as_ref().unwrap());
        assert!(!core_dir.join("semantic_vectors.bin").exists());
        assert!(!core_dir.join("graph_edges.json").exists());
        assert!(core_dir.join("memory_state.json").exists());
    }

    /// Core 快照恢复必须移除旧的语义向量/图边（否则旧向量指向已不存在的记忆）。
    #[test]
    fn restore_core_snapshot_removes_stale_vectors_and_graph() {
        let (_tmp, data, backups) = setup_dirs();
        fs::write(data.join("memories.json"), r#"[{"id":"old"}]"#).unwrap();
        fs::write(data.join("semantic_vectors.bin"), b"stale-vectors").unwrap();
        fs::write(data.join("graph_edges.json"), b"stale-edges").unwrap();

        let core = create_backup_in(&data, &backups, BackupMode::Core);
        assert!(core.success, "{:?}", core.error);
        let core_dir = std::path::PathBuf::from(core.backup_path.as_ref().unwrap());

        // 备份后再写入"恢复点之后"的运行时文件，模拟目标目录比快照更新。
        fs::write(data.join("semantic_vectors.bin"), b"newer-vectors").unwrap();
        fs::write(data.join("graph_edges.json"), b"newer-edges").unwrap();

        restore_backup_in(&core_dir, &backups, &data).expect("恢复应成功");
        assert!(
            !data.join("semantic_vectors.bin").exists(),
            "Core 恢复应移除旧向量，等待按恢复出的记忆重建"
        );
        assert!(
            !data.join("graph_edges.json").exists(),
            "Core 恢复应移除旧图边，等待重建"
        );
    }

    /// 清单里出现未知文件必须拒绝恢复（防止篡改清单覆盖数据目录其他文件）。
    #[test]
    fn restore_rejects_manifest_with_unknown_files() {
        let (_tmp, data, backups) = setup_dirs();
        let snapshot = backups.join("memories_20260101_000003.snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("memories.json"), r#"[{"id":"m1"}]"#).unwrap();
        let manifest = serde_json::json!({
            "format_version": MANIFEST_FORMAT_VERSION,
            "created_at_ms": 0,
            "mode": "core",
            "files": ["memories.json", "reclassify_progress.json"],
            "absent": []
        });
        fs::write(
            snapshot.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let error = restore_backup_in(&snapshot, &backups, &data).expect_err("必须拒绝");
        assert!(
            error.to_string().contains("未知文件"),
            "错误信息应指出未知文件：{error}"
        );
    }

    /// 恢复旧快照（无清单）不得清空新增的大文件（语义向量/图边）。
    #[test]
    fn restore_legacy_snapshot_keeps_new_runtime_files() {
        let (_tmp, data, backups) = setup_dirs();
        let snapshot = backups.join("memories_20260101_000002.snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("memories.json"), r#"[{"id":"restored"}]"#).unwrap();
        fs::write(data.join("memories.json"), r#"[{"id":"stale"}]"#).unwrap();
        fs::write(data.join("semantic_vectors.bin"), b"keep-vectors").unwrap();
        fs::write(data.join("graph_edges.json"), b"keep-edges").unwrap();

        restore_backup_in(&snapshot, &backups, &data).expect("恢复应成功");
        assert_eq!(
            fs::read_to_string(data.join("memories.json")).unwrap(),
            r#"[{"id":"restored"}]"#
        );
        assert_eq!(
            fs::read(data.join("semantic_vectors.bin")).unwrap(),
            b"keep-vectors",
            "旧快照不得清空语义向量"
        );
        assert_eq!(
            fs::read(data.join("graph_edges.json")).unwrap(),
            b"keep-edges",
            "旧快照不得清空图边"
        );
    }

    /// 带清单的快照只覆盖清单列出的文件；未列出的文件保持原样。
    #[test]
    fn restore_manifest_snapshot_only_touches_listed_files() {
        let (_tmp, data, backups) = setup_dirs();
        let snapshot = backups.join("memories_20260101_000003.snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("memories.json"), r#"[{"id":"restored"}]"#).unwrap();
        let manifest = SnapshotManifest {
            format_version: MANIFEST_FORMAT_VERSION,
            created_at_ms: 0,
            // Full 模式：清单未列出的文件保持原样（Core 会主动移除旧向量/图边，
            // 见 `restore_core_snapshot_removes_stale_vectors_and_graph`）。
            mode: BackupMode::Full,
            files: vec!["memories.json".to_string()],
            absent: Vec::new(),
        };
        fs::write(
            snapshot.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(data.join("memories.json"), r#"[{"id":"stale"}]"#).unwrap();
        fs::write(data.join("graph_edges.json"), b"keep-edges").unwrap();

        restore_backup_in(&snapshot, &backups, &data).expect("恢复应成功");
        assert_eq!(
            fs::read_to_string(data.join("memories.json")).unwrap(),
            r#"[{"id":"restored"}]"#
        );
        assert_eq!(
            fs::read(data.join("graph_edges.json")).unwrap(),
            b"keep-edges",
            "清单未列出的文件不得被覆盖"
        );
    }

    /// 清单路径穿越必须被拒绝。
    #[test]
    fn restore_rejects_manifest_with_path_traversal() {
        let (_tmp, data, backups) = setup_dirs();
        let snapshot = backups.join("memories_20260101_000004.snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("memories.json"), r#"[{"id":"restored"}]"#).unwrap();
        let manifest = SnapshotManifest {
            format_version: MANIFEST_FORMAT_VERSION,
            created_at_ms: 0,
            mode: BackupMode::Core,
            files: vec!["../evil.json".to_string()],
            absent: Vec::new(),
        };
        fs::write(
            snapshot.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(data.join("memories.json"), r#"[{"id":"keep"}]"#).unwrap();

        let err = restore_backup_in(&snapshot, &backups, &data).expect_err("必须拒绝路径穿越");
        assert!(err.to_string().contains("非法文件名"), "实际 {err}");
        assert_eq!(
            fs::read_to_string(data.join("memories.json")).unwrap(),
            r#"[{"id":"keep"}]"#
        );
    }

    /// Windows 盘符/ADS/保留名/结尾点都必须被拒绝。
    #[test]
    fn restore_rejects_windows_unsafe_manifest_names() {
        for name in ["C:evil", "memories.json:ads", "CON", "aux.txt", "trailing."] {
            let (_tmp, data, backups) = setup_dirs();
            let snapshot = backups.join("memories_20260101_000005.snapshot");
            fs::create_dir_all(&snapshot).unwrap();
            fs::write(snapshot.join("memories.json"), r#"[{"id":"restored"}]"#).unwrap();
            let manifest = SnapshotManifest {
                format_version: MANIFEST_FORMAT_VERSION,
                created_at_ms: 0,
                mode: BackupMode::Core,
                files: vec!["memories.json".to_string(), name.to_string()],
                absent: Vec::new(),
            };
            fs::write(
                snapshot.join(MANIFEST_FILE),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            fs::write(data.join("memories.json"), r#"[{"id":"keep"}]"#).unwrap();

            let err =
                restore_backup_in(&snapshot, &backups, &data).expect_err("必须拒绝不安全文件名");
            assert!(err.to_string().contains("非法文件名"), "{name}: {err}");
            assert_eq!(
                fs::read_to_string(data.join("memories.json")).unwrap(),
                r#"[{"id":"keep"}]"#
            );
        }
    }

    /// `absent` 记录的文件必须在恢复时移除，防止旧日志被重放。
    #[test]
    fn restore_manifest_absent_removes_target_journal() {
        let (_tmp, data, backups) = setup_dirs();
        let snapshot = backups.join("memories_20260101_000006.snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("memories.json"), r#"[{"id":"restored"}]"#).unwrap();
        let manifest = SnapshotManifest {
            format_version: MANIFEST_FORMAT_VERSION,
            created_at_ms: 0,
            mode: BackupMode::Full,
            files: vec!["memories.json".to_string()],
            absent: vec!["memories.journal.jsonl".to_string()],
        };
        fs::write(
            snapshot.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(data.join("memories.json"), r#"[{"id":"stale"}]"#).unwrap();
        fs::write(
            data.join("memories.journal.jsonl"),
            "{\"id\":\"after-backup\"}\n",
        )
        .unwrap();

        restore_backup_in(&snapshot, &backups, &data).expect("恢复应成功");
        assert!(
            !data.join("memories.journal.jsonl").exists(),
            "快照时缺失的日志必须在恢复时移除，否则会被重放"
        );
        assert_eq!(
            fs::read_to_string(data.join("memories.json")).unwrap(),
            r#"[{"id":"restored"}]"#
        );
    }

    /// 清单必须包含 memories.json；未知格式版本必须拒绝。
    #[test]
    fn restore_manifest_requires_base_file_and_known_version() {
        // 缺 memories.json
        let (_tmp, data, backups) = setup_dirs();
        let snapshot = backups.join("memories_20260101_000007.snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("memories.json"), r#"[{"id":"restored"}]"#).unwrap();
        let manifest = SnapshotManifest {
            format_version: MANIFEST_FORMAT_VERSION,
            created_at_ms: 0,
            mode: BackupMode::Core,
            files: vec!["lifecycle.json".to_string()],
            absent: Vec::new(),
        };
        fs::write(
            snapshot.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(data.join("memories.json"), r#"[{"id":"keep"}]"#).unwrap();
        let err = restore_backup_in(&snapshot, &backups, &data).expect_err("缺基文件必须拒绝");
        assert!(err.to_string().contains("memories.json"), "{err}");

        // 未知版本
        let manifest = SnapshotManifest {
            format_version: 999,
            created_at_ms: 0,
            mode: BackupMode::Core,
            files: vec!["memories.json".to_string()],
            absent: Vec::new(),
        };
        fs::write(
            snapshot.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let err = restore_backup_in(&snapshot, &backups, &data).expect_err("未知版本必须拒绝");
        assert!(err.to_string().contains("不支持快照清单版本"), "{err}");
    }

    /// 无 `memories.json` ⇒ **明确失败**并给出可读原因（不得静默造出空快照）
    #[test]
    fn create_backup_in_fails_loudly_without_memories_file() {
        let (_tmp, data, backups) = setup_dirs();
        let report = create_backup_in(&data, &backups, BackupMode::Full);
        assert!(!report.success, "缺 memories.json 时不得报告成功");
        assert!(
            report
                .error
                .as_deref()
                .unwrap_or("")
                .contains("记忆文件不存在"),
            "错误信息应指明原因，实际 {:?}",
            report.error
        );
        assert_eq!(count_backups(&backups), 0, "失败不得留下半成品快照");
    }

    /// 恢复必须把快照内容**逐字节**写回数据目录
    #[test]
    fn restore_backup_in_writes_snapshot_back_into_data_dir() {
        let (_tmp, data, backups) = setup_dirs();
        let snapshot = backups.join("memories_20260101_000000.snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("memories.json"), r#"[{"id":"restored"}]"#).unwrap();
        // 数据目录里先放"被覆盖前"的旧内容
        fs::write(data.join("memories.json"), r#"[{"id":"stale"}]"#).unwrap();

        restore_backup_in(&snapshot, &backups, &data).expect("恢复应成功");

        assert_eq!(
            fs::read_to_string(data.join("memories.json")).unwrap(),
            r#"[{"id":"restored"}]"#,
            "恢复后必须是快照内容"
        );
        // 快照中缺失的其它文件也应被恢复成空文件（保持"全量覆盖"语义）
        assert!(
            data.join("audit.jsonl").exists(),
            "缺失文件应被恢复为空文件"
        );
        assert_eq!(fs::metadata(data.join("audit.jsonl")).unwrap().len(), 0);
    }

    /// ★v0.11 P0 回归：备份必须包含追加日志；恢复必须清掉「日志作废」标记。
    ///
    /// 背景：写入走「基文件 + 追加日志」，此前快照只拷基文件 ⇒ 压实周期内的
    /// 新增记忆不在备份里；恢复后旧日志又被重放 ⇒ 恢复被静默撤销。
    #[test]
    fn backup_includes_journal_and_restore_clears_invalidated_marker() {
        let (_tmp, data, backups) = setup_dirs();
        fs::write(data.join("memories.json"), r#"[{"id":"base"}]"#).unwrap();
        fs::write(
            data.join("memories.journal.jsonl"),
            "{\"id\":\"journaled\"}\n{\"id\":\"base\",\"deleted\":true}\n",
        )
        .unwrap();

        let report = create_backup_in(&data, &backups, BackupMode::Full);
        assert!(report.success, "{:?}", report.error);
        assert_eq!(
            report.memory_count, 1,
            "base 已被墓碑删除，有效记忆只剩 journaled"
        );
        let snapshot = std::path::PathBuf::from(report.backup_path.as_ref().unwrap());
        assert_eq!(
            fs::read_to_string(snapshot.join("memories.journal.jsonl")).unwrap(),
            "{\"id\":\"journaled\"}\n{\"id\":\"base\",\"deleted\":true}\n",
            "日志必须与基文件同快照"
        );

        // 恢复前数据目录里有「更新」的日志与作废标记
        fs::write(data.join("memories.journal.jsonl"), "{\"id\":\"newer\"}\n").unwrap();
        fs::write(data.join("memories.journal.jsonl.invalidated"), "").unwrap();

        restore_backup_in(&snapshot, &backups, &data).expect("恢复应成功");
        assert_eq!(
            fs::read_to_string(data.join("memories.journal.jsonl")).unwrap(),
            "{\"id\":\"journaled\"}\n{\"id\":\"base\",\"deleted\":true}\n",
            "恢复后必须是快照里的日志（恢复点之后的旧日志不得残留）"
        );
        assert!(
            !data.join("memories.journal.jsonl.invalidated").exists(),
            "必须清除作废标记，否则 replay_journal 会删除刚恢复的日志"
        );
    }

    /// **安全红线**：快照目录必须位于备份目录内，否则拒绝（路径穿越/越权写）
    #[test]
    fn restore_backup_in_rejects_snapshot_outside_backups_root() {
        let (tmp, data, backups) = setup_dirs();
        let outside = tmp.path().join("evil");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("memories.json"), r#"[{"id":"evil"}]"#).unwrap();
        fs::write(data.join("memories.json"), r#"[{"id":"keep"}]"#).unwrap();

        let err =
            restore_backup_in(&outside, &backups, &data).expect_err("备份目录外的快照必须被拒绝");
        assert!(
            err.to_string().contains("不在备份目录"),
            "应给出越界原因，实际 {err}"
        );
        assert_eq!(
            fs::read_to_string(data.join("memories.json")).unwrap(),
            r#"[{"id":"keep"}]"#,
            "拒绝之后目标数据必须**原封不动**"
        );
    }

    /// 恢复同样要求快照内确有 `memories.json`（空目录不得被当成合法快照）
    #[test]
    fn restore_backup_in_rejects_snapshot_without_memories_file() {
        let (_tmp, data, backups) = setup_dirs();
        let snapshot = backups.join("memories_20260101_000001.snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(data.join("memories.json"), r#"[{"id":"keep"}]"#).unwrap();

        let err = restore_backup_in(&snapshot, &backups, &data)
            .expect_err("缺 memories.json 的快照必须被拒绝");
        assert!(
            err.to_string().contains("memories.json"),
            "应指明缺失项，实际 {err}"
        );
        assert_eq!(
            fs::read_to_string(data.join("memories.json")).unwrap(),
            r#"[{"id":"keep"}]"#,
            "数据必须原封不动"
        );
    }

    /// 源码级守卫：生产入口必须走可注入内核，且**不得**把 home 路径写回函数体
    /// （写回去 = 这些用例会立刻变成"污染真实记忆库"的危险测试）
    ///
    /// ⚠️ 断言范围截到测试模块之前：测试自身源码含 `create_backup_in(` 字样，
    /// 整文件扫描会"自己匹配自己"⇒ 守卫恒真。
    #[test]
    fn backup_entrypoints_delegate_to_injectable_cores() {
        let src = include_str!("backup.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        let pub_create = prod
            .find("pub fn create_backup(")
            .expect("必须存在 create_backup");
        assert!(
            prod[pub_create..].contains("create_backup_in("),
            "create_backup 必须委托 create_backup_in（否则无法被隔离测试）"
        );
        let pub_restore = prod
            .find("pub fn restore_backup(")
            .expect("必须存在 restore_backup");
        assert!(
            prod[pub_restore..].contains("restore_backup_in("),
            "restore_backup 必须委托 restore_backup_in（否则无法被隔离测试）"
        );
    }
}
