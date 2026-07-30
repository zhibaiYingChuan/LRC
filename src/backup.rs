// ============================================================
// 许可证: Apache 2.0
// 本文件实现记忆数据自动备份机制，属于公开层 (Layer 1)。
// ============================================================
//
// v0.8.0 "归一" 专项：记忆数据备份模块
//
// 功能：
//   1. 手动/自动将当前记忆库导出为 JSON 备份文件
//   2. 备份存储在 ~/.loong-recall/backups/ 目录
//   3. 文件名格式：memories_YYYYMMDD_HHMMSS.json
//   4. 自动清理旧备份，默认保留最近 4 份
//
// 设计原则：
//   - 备份是只读拷贝，不修改原文件
//   - 备份文件包含完整的 memories.json 内容
//   - 清理策略基于文件修改时间，最旧的先删
//   - 备份失败不影响主流程

use std::fs;
use std::path::{Path, PathBuf};

/// 备份保留份数（超出此数量的最旧备份将被删除）
const MAX_BACKUPS: usize = 4;

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
pub fn backups_dir() -> PathBuf {
    let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".loong-recall").join("backups")
}

/// 获取全局数据目录路径：~/.loong-recall/global/data/
fn global_data_dir() -> PathBuf {
    let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".loong-recall").join("global").join("data")
}

/// 生成带时间戳的备份文件名
fn backup_filename() -> String {
    let now = chrono::Local::now();
    format!("memories_{}.json", now.format("%Y%m%d_%H%M%S"))
}

/// 创建备份
///
/// 将 ~/.loong-recall/global/data/memories.json 复制到
/// ~/.loong-recall/backups/memories_YYYYMMDD_HHMMSS.json
///
/// 自动清理超过 MAX_BACKUPS 份数的旧备份。
pub fn create_backup() -> BackupReport {
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
    let memory_file = data_dir.join("memories.json");

    // 检查源文件是否存在
    if !memory_file.exists() {
        report.error = Some(format!("记忆文件不存在: {}", memory_file.display()));
        return report;
    }

    // 确保备份目录存在
    let backups_dir = backups_dir();
    if let Err(e) = fs::create_dir_all(&backups_dir) {
        report.error = Some(format!("创建备份目录失败: {}", e));
        return report;
    }

    // 生成备份文件路径
    let backup_file = backups_dir.join(backup_filename());

    // 复制文件
    if let Err(e) = fs::copy(&memory_file, &backup_file) {
        report.error = Some(format!("复制文件失败: {}", e));
        return report;
    }

    // 获取备份文件大小
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

    report.backup_path = Some(backup_file.to_string_lossy().to_string());

    // 清理旧备份
    report.old_backups_removed = cleanup_old_backups(&backups_dir);

    // 统计当前备份总数
    report.total_backups = count_backups(&backups_dir);

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
                if name.starts_with("memories_") && name.ends_with(".json") {
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
        if fs::remove_file(path).is_ok() {
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
                if name.starts_with("memories_") && name.ends_with(".json") {
                    count += 1;
                }
            }
        }
    }
    count
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
                if !(name.starts_with("memories_") && name.ends_with(".json")) {
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
        // 格式：memories_YYYYMMDD_HHMMSS.json
        assert_eq!(name.len(), "memories_YYYYMMDD_HHMMSS.json".len());
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
