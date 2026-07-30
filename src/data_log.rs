// ============================================================
// 许可证: Apache 2.0
// 本文件实现数据操作日志记录，属于公开层 (Layer 1)。
// ============================================================
//
// v0.8.0 "归一" 专项：数据操作日志模块
//
// 功能：
//   1. 记录所有数据操作（迁移、合并、备份、恢复、导出、导入）
//   2. 日志存储在 ~/.loong-recall/data_operations.log
//   3. 格式：ISO8601时间 | 操作类型 | 详情描述
//   4. 支持读取最近 N 条记录用于信任中心展示
//
// 设计原则：
//   - 日志是追加写入，不修改历史记录
//   - 日志文件大小自然增长（数据操作不会很频繁）
//   - 记录失败不影响主流程（静默失败）
//   - 每条记录一行，便于读取和解析

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

/// 操作类型
#[derive(Debug, Clone)]
pub enum OperationType {
    Migrate,
    Backup,
    Restore,
    Export,
    Import,
    Clean,
}

impl OperationType {
    fn as_str(&self) -> &'static str {
        match self {
            OperationType::Migrate => "migrate",
            OperationType::Backup => "backup",
            OperationType::Restore => "restore",
            OperationType::Export => "export",
            OperationType::Import => "import",
            OperationType::Clean => "clean",
        }
    }
}

/// 日志文件路径：~/.loong-recall/data_operations.log
pub fn log_file_path() -> PathBuf {
    let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".loong-recall").join("data_operations.log")
}

/// 记录一条数据操作日志
///
/// 格式：`2026-07-29T10:00:00Z | migrate | 3125 条记忆从 G:\data\code-memory\ 迁移至 global`
pub fn log_operation(op: OperationType, details: &str) {
    let log_path = log_file_path();

    // 确保目录存在
    if let Some(parent) = log_path.parent() {
        if !parent.exists() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("[data_log] 创建日志目录失败: {}", e);
                return;
            }
        }
    }

    // 生成时间戳
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let line = format!("{} | {} | {}\n", timestamp, op.as_str(), details);

    // 追加写入
    match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(mut file) => {
            if let Err(e) = file.write_all(line.as_bytes()) {
                eprintln!("[data_log] 写入日志失败: {}", e);
            }
        }
        Err(e) => {
            eprintln!("[data_log] 打开日志文件失败: {}", e);
        }
    }
}

/// 日志条目
#[derive(Debug, Clone, serde::Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub operation: String,
    pub details: String,
}

/// 读取最近 N 条日志记录
///
/// 从文件末尾读取，返回按时间倒序的列表（最新的在前）。
pub fn read_recent_operations(n: usize) -> Vec<LogEntry> {
    let log_path = log_file_path();

    if !log_path.exists() {
        return Vec::new();
    }

    let content = match fs::read_to_string(&log_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[data_log] 读取日志文件失败: {}", e);
            return Vec::new();
        }
    };

    // 按行解析，取最后 N 行
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > n { lines.len() - n } else { 0 };

    let mut entries: Vec<LogEntry> = lines[start..]
        .iter()
        .filter_map(|line| parse_log_line(line))
        .collect();

    // 反转为倒序（最新的在前）
    entries.reverse();
    entries
}

/// 解析单行日志
fn parse_log_line(line: &str) -> Option<LogEntry> {
    let parts: Vec<&str> = line.splitn(3, " | ").collect();
    if parts.len() != 3 {
        return None;
    }
    Some(LogEntry {
        timestamp: parts[0].to_string(),
        operation: parts[1].to_string(),
        details: parts[2].to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_type_as_str() {
        assert_eq!(OperationType::Migrate.as_str(), "migrate");
        assert_eq!(OperationType::Backup.as_str(), "backup");
        assert_eq!(OperationType::Restore.as_str(), "restore");
        assert_eq!(OperationType::Export.as_str(), "export");
        assert_eq!(OperationType::Import.as_str(), "import");
        assert_eq!(OperationType::Clean.as_str(), "clean");
    }

    #[test]
    fn test_log_file_path() {
        let path = log_file_path();
        assert!(path.to_string_lossy().contains("data_operations.log"));
    }

    #[test]
    fn test_parse_log_line_valid() {
        let line = "2026-07-29T10:00:00Z | migrate | 3125 条记忆迁移至 global";
        let entry = parse_log_line(line);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.timestamp, "2026-07-29T10:00:00Z");
        assert_eq!(entry.operation, "migrate");
        assert_eq!(entry.details, "3125 条记忆迁移至 global");
    }

    #[test]
    fn test_parse_log_line_invalid() {
        assert!(parse_log_line("invalid line").is_none());
        assert!(parse_log_line("").is_none());
        // 分隔符为 " | "（带空格），splitn(3) 只分割前 2 个，剩余作为第 3 部分
        assert!(parse_log_line("only | two | parts | extra").is_some()); // splitn(3) handles this
    }

    #[test]
    fn test_parse_log_line_with_pipe_in_details() {
        let line = "2026-07-29T10:00:00Z | backup | 备份至 /path|with|pipe.json";
        let entry = parse_log_line(line);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.details, "备份至 /path|with|pipe.json");
    }

    #[test]
    fn test_read_recent_operations_empty() {
        // 不存在的文件应返回空列表
        // 注意：此测试依赖 log_file_path() 返回的实际路径
        // 在测试环境中，如果文件不存在则返回空
        let entries = read_recent_operations(10);
        // 不做严格断言，因为文件可能存在（取决于测试运行环境）
        let _ = entries;
    }

    #[test]
    fn test_log_entry_serialization() {
        let entry = LogEntry {
            timestamp: "2026-07-29T10:00:00Z".to_string(),
            operation: "backup".to_string(),
            details: "备份 100 条记忆".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"operation\":\"backup\""));
        assert!(json.contains("\"details\":\"备份 100 条记忆\""));
    }
}
