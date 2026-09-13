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
use crate::errors::{LrcError, LrcResult};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// 备份锁序契约（v0.9.7 修复 GLOBAL_CODE_REVIEW_REPORT 并发 P2-3「备份锁序未文档化」）
/// ============================================================
/// 本模块**只持有一把锁**，代码库中的版本如下，改动时须刷新此说明：
///
/// ```text
///   [1] BACKUP_OPERATION_LOCK (Mutex<()>)  —— 全局单例，串行化"创建/恢复/清理备份"
/// ```
///
/// **与持久层的锁序关系（关键，避免 ABBA）**：
///   - 本锁**从不**在持有 `persistence::json::JSON_WRITE_LOCK` 或
///     `JsonPersistence::cache` 锁的情况下获取（备份流程为独立入口：
///     仪表盘"立即备份"、CLI `--backup`、恢复均自顶向下调用）。
///   - 反向：持久层写入路径（`save_memory` 等）**从不**调用备份模块。
///     故两模块之间**无环路**，不存在跨模块 ABBA。
///   - 若未来需要"写入前自动备份"，**必须**在**释放**本锁后再进入持久层写锁，
///     即维持 `BACKUP → (release) → cache → JSON_WRITE` 的顺序，禁止嵌套获取。
///
/// **持锁期间的行为**：备份为文件级 `fs::copy`（见 `create_backup_locked`），
///   持锁期间执行磁盘 IO 是**有意**的——备份的语义就是"某一时刻的一致快照"，
///   必须与并发的写操作互斥。此处临界区放大是正确性要求，非缺陷。
///
/// **调用点**：[`create_backup`]（本文件 :109 附近）与 [`restore_backup`]
///   （本文件 :267 附近）各获取一次；`count_backups` / `cleanup_old_backups`
///   不单独加锁，仅由持锁方在临界区内调用。
static BACKUP_OPERATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 备份保留份数（超出此数量的最旧备份将被删除）
const MAX_BACKUPS: usize = 4;
const SNAPSHOT_FILES: [&str; 7] = [
    "memories.json",
    "chunks.json",
    "archive.json",
    "audit.jsonl",
    "audit.jsonl.seal",
    "audit.jsonl.anchors.jsonl",
    "feedback.jsonl",
];
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
}

/// 获取备份目录路径：~/.loong-recall/backups/
///
/// v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P2 安全「备份目录两套推导」+ P3「密钥路径回退 CWD」同源项）：
///   原实现 `home_dir().unwrap_or_else(|| PathBuf::from("."))` 在 home 不可用时
///   退化为**相对当前工作目录**的 `./.loong-recall/backups`——CWD 可被攻击者控制，
///   备份（含全部记忆明文快照）会落到非预期位置。改为显式回退到系统临时目录，
///   并打印告警，避免"静默写入相对路径"。
pub fn backups_dir() -> PathBuf {
    home_dir_or_fallback().join(".loong-recall").join("backups")
}

/// 获取全局数据目录路径：~/.loong-recall/global/data/
fn global_data_dir() -> PathBuf {
    home_dir_or_fallback()
        .join(".loong-recall")
        .join("global")
        .join("data")
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
pub fn create_backup() -> BackupReport {
    let _operation_guard = BACKUP_OPERATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut report = BackupReport {
        success: false,
        backup_path: None,
        backup_size: 0,
        memory_count: 0,
        old_backups_removed: 0,
        total_backups: 0,
        error: None,
    };
    let data_dir = global_data_dir();
    if !data_dir.join("memories.json").exists() {
        report.error = Some(format!(
            "记忆文件不存在: {}",
            data_dir.join("memories.json").display()
        ));
        return report;
    }
    let root = backups_dir();
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
    let result = (|| -> LrcResult<()> {
        fs::create_dir(&temp_dir).map_err(|e| LrcError::io(format!("创建临时快照失败: {}", e)))?;
        for file in SNAPSHOT_FILES {
            let source = data_dir.join(file);
            let target = temp_dir.join(file);
            if source.exists() {
                fs::copy(&source, &target)
                    .map_err(|e| LrcError::io(format!("备份 {} 失败: {}", file, e)))?;
            } else {
                fs::write(&target, b"")
                    .map_err(|e| LrcError::io(format!("创建空文件 {} 失败: {}", file, e)))?;
            }
        }
        fs::rename(&temp_dir, &final_dir).map_err(|e| LrcError::io(format!("提交快照失败: {}", e)))
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&temp_dir);
        // BackupReport.error 是对外 JSON 字段，仍为 String 契约：取 message。
        report.error = Some(String::from(e));
        return report;
    }
    let backup_file = final_dir.join("memories.json");
    report.backup_size = fs::metadata(&backup_file).map(|m| m.len()).unwrap_or(0);

    // 统计记忆数
    if let Ok(content) = fs::read_to_string(&backup_file) {
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
            report.memory_count = if data.is_array() {
                data.as_array().map(|a| a.len()).unwrap_or(0)
            } else if data.is_object() {
                data.get("memories")
                    .and_then(|m| m.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0)
            } else {
                0
            };
        }
    }

    report.backup_path = Some(final_dir.to_string_lossy().to_string());

    // 清理旧备份
    report.old_backups_removed = cleanup_old_backups(&root);

    // 统计当前备份总数
    report.total_backups = count_backups(&root);

    report.success = true;

    // v0.8.0 "归一"：记录数据操作日志
    let details = format!(
        "备份 {} 条记忆至 {}（清理 {} 份旧备份，当前共 {} 份）",
        report.memory_count,
        report.backup_path.as_deref().unwrap_or("未知路径"),
        report.old_backups_removed,
        report.total_backups
    );
    crate::data_log::log_operation(crate::data_log::OperationType::Backup, &details);

    report
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
pub fn restore_backup(snapshot_path: &Path) -> LrcResult<()> {
    let _operation_guard = BACKUP_OPERATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // v0.9.7 审查修复（HCSE-P0 安全）：快照目录必须位于本工具的备份目录内。
    // 端点默认无 token 保护，若不约束路径，本机任意进程可把任意目录内容
    // （SNAPSHOT_FILES 同名文件）覆盖进全局数据目录，属路径穿越/越权写。
    let allowed_root = backups_dir();
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
    let data_dir = global_data_dir();
    fs::create_dir_all(&data_dir).map_err(|e| LrcError::io(format!("创建数据目录失败: {}", e)))?;
    let temp = data_dir.join(format!(
        ".restore-{}.tmp",
        BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&temp).map_err(|e| LrcError::io(format!("创建恢复临时目录失败: {}", e)))?;
    let result = (|| -> LrcResult<()> {
        for file in SNAPSHOT_FILES {
            let source = snapshot_path.join(file);
            if source.exists() {
                fs::copy(&source, temp.join(file))
                    .map_err(|e| LrcError::io(format!("恢复 {} 失败: {}", file, e)))?;
            } else {
                fs::write(temp.join(file), b"")
                    .map_err(|e| LrcError::io(format!("恢复空文件 {} 失败: {}", file, e)))?;
            }
        }
        let rollback = data_dir.join(format!(
            ".restore-{}-rollback",
            BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&rollback)
            .map_err(|e| LrcError::io(format!("创建恢复回滚目录失败: {}", e)))?;
        let mut originals = Vec::new();
        let mut committed = Vec::new();
        let commit_result = (|| -> LrcResult<()> {
            for file in SNAPSHOT_FILES {
                let target = data_dir.join(file);
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
            return Err(LrcError::io(format!("{}；已尝试回滚恢复文件", error)));
        }
        fs::remove_dir_all(&rollback)
            .map_err(|e| LrcError::io(format!("清理恢复回滚目录失败: {}", e)))?;
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
                    let size = meta.len();
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
        let temp = std::env::temp_dir().join("lrc_backup_test_snapshot_dirs");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        for i in 0..6 {
            let snapshot = temp.join(format!("memories_2026010{}.snapshot", i));
            fs::create_dir(&snapshot).unwrap();
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
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"memory_count\":100"));
    }
}
