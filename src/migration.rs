// ============================================================
// 许可证: Apache 2.0
// 本文件实现旧数据迁移，属于公开层 (Layer 1)。
// ============================================================
//
// 数据迁移模块 — 将旧版 V1 数据目录迁移到 V2 统一数据目录
//
// 迁移策略：
//   1. 启动时检查旧版 {src_dir}/.loong-recall/data/ 是否存在
//   2. 若存在且新版 ~/.loong-recall/projects/{fp}/data/ 为空 → 自动迁移
//   3. 迁移完成后在旧目录创建 .migrated_to_v2 标记文件
//   4. 若迁移失败，保留旧数据不动，报告错误
//
// 迁移文件：
//   - memories.json
//   - chunks.json
//   - archive.json
//   - 其他 .json 文件
//
// 安全原则：
//   - 迁移采用复制（copy）而非移动（move），旧数据不丢失
//   - 迁移前检查新版目录是否为空，非空则跳过避免覆盖
//   - 每个文件独立迁移，失败不影响其他文件

use std::path::Path;
use std::{fs, io};

/// 迁移结果统计
#[derive(Debug, Clone, Default)]
pub struct MigrationResult {
    /// 已迁移的文件数
    pub migrated_files: usize,
    /// 跳过的文件数（新版目录已有）
    pub skipped_files: usize,
    /// 失败的文件数
    pub failed_files: usize,
    /// 操作详情（用于日志和调试）
    pub details: Vec<String>,
}

impl MigrationResult {
    /// 是否完全成功（无失败文件）
    pub fn is_success(&self) -> bool {
        self.failed_files == 0
    }

    /// 是否有文件被迁移
    pub fn has_migrations(&self) -> bool {
        self.migrated_files > 0
    }
}

/// 将旧版数据从 src_dir 迁移到新版数据目录
///
/// # 参数
/// - `src_dir`: 项目源码目录（旧版数据位于 src_dir/.loong-recall/data/）
/// - `new_data_dir`: 新版数据目录（~/.loong-recall/projects/{fp}/data/）
/// - `dry_run`: 如果为 true，仅报告将迁移什么，不实际执行
///
/// # 返回
/// - `Ok(MigrationResult)`: 迁移结果统计
/// - `Err(io::Error)`: 严重错误（如无法读取旧版目录）
pub fn migrate_legacy_to_v2(
    src_dir: &Path,
    new_data_dir: &Path,
    dry_run: bool,
) -> io::Result<MigrationResult> {
    let legacy_data_dir = crate::data_dir::DataDir::legacy_data_path(src_dir);
    let mut result = MigrationResult::default();

    // 检查迁移标记：如果已迁移过，直接跳过
    if crate::data_dir::DataDir::is_migrated(src_dir) {
        result
            .details
            .push("已迁移过（检测到 .migrated_to_v2 标记），跳过".to_string());
        return Ok(result);
    }

    // 检查旧版数据目录是否存在
    if !legacy_data_dir.exists() {
        result
            .details
            .push("旧版数据目录不存在，无需迁移".to_string());
        return Ok(result);
    }

    result
        .details
        .push(format!("检测到旧版数据: {}", legacy_data_dir.display()));

    // 确保新版数据目录存在
    if !dry_run {
        fs::create_dir_all(new_data_dir)?;
    }

    // 列出旧版数据目录中的所有 JSON 文件
    let entries = match fs::read_dir(&legacy_data_dir) {
        Ok(entries) => entries,
        Err(e) => {
            result.details.push(format!("无法读取旧版数据目录: {}", e));
            return Err(e);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // 只迁移数据文件（.json），跳过锁文件和标记文件
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !file_name.ends_with(".json") {
            result.details.push(format!("跳过非数据文件: {file_name}"));
            continue;
        }

        let dest = new_data_dir.join(file_name);

        // 如果新版目录已有同名文件，跳过（避免覆盖已有数据）
        if dest.exists() {
            result
                .details
                .push(format!("跳过已存在文件: {file_name}（新版目录已有）"));
            result.skipped_files += 1;
            continue;
        }

        if dry_run {
            result.details.push(format!(
                "[DRY RUN] 将迁移: {file_name} → {}",
                new_data_dir.display()
            ));
            result.migrated_files += 1;
        } else {
            // 复制文件（保留旧数据不丢失）
            match fs::copy(&path, &dest) {
                Ok(bytes) => {
                    result
                        .details
                        .push(format!("已迁移: {file_name} ({} bytes)", bytes));
                    result.migrated_files += 1;
                }
                Err(e) => {
                    result
                        .details
                        .push(format!("迁移失败: {file_name} — {}", e));
                    result.failed_files += 1;
                }
            }
        }
    }

    // 迁移完成后写入标记文件
    if !dry_run && result.is_success() {
        let marker = crate::data_dir::DataDir::migration_marker_path(src_dir);
        if let Some(parent) = marker.parent() {
            fs::create_dir_all(parent)?;
        }
        match fs::write(
            &marker,
            format!(
                "v2\nmigrated_at: {}\nnew_path: {}",
                {
                    // 使用简单的 Unix 时间戳，避免依赖 chrono
                    use std::time::SystemTime;
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map(|d| d.as_secs().to_string())
                        .unwrap_or_else(|_| "unknown".to_string())
                },
                new_data_dir.display()
            ),
        ) {
            Ok(()) => {
                result
                    .details
                    .push("迁移标记已写入 .migrated_to_v2".to_string());
            }
            Err(e) => {
                result.details.push(format!("写入迁移标记失败: {}", e));
            }
        }
    }

    if result.migrated_files == 0 && result.skipped_files == 0 {
        result
            .details
            .push("旧版数据目录为空，无需迁移".to_string());
    }

    Ok(result)
}

/// 检查是否需要进行迁移
///
/// 返回 true 表示需要迁移：
/// - 旧版数据目录存在
/// - 尚未迁移（无 .migrated_to_v2 标记）
pub fn needs_migration(src_dir: &Path) -> bool {
    crate::data_dir::DataDir::has_legacy_data(src_dir)
        && !crate::data_dir::DataDir::is_migrated(src_dir)
}

/// 生成迁移报告字符串（用于日志输出）
pub fn format_migration_report(result: &MigrationResult) -> String {
    let mut report = String::new();
    report.push_str(&format!(
        "迁移完成: {} 个文件已迁移, {} 跳过, {} 失败",
        result.migrated_files, result.skipped_files, result.failed_files
    ));
    if !result.details.is_empty() {
        report.push('\n');
        for detail in &result.details {
            report.push_str(&format!("  - {detail}\n"));
        }
    }
    report
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_dir::DataDir;

    /// 测试: 旧版数据不存在时跳过迁移
    #[test]
    fn test_no_legacy_data_skips() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("empty_project");
        let new_data = tmp.path().join("new_data");

        // 创建空的新版目录
        fs::create_dir_all(&new_data).unwrap();

        let result = migrate_legacy_to_v2(&src, &new_data, false).expect("迁移不应报错");
        assert_eq!(result.migrated_files, 0);
        assert_eq!(result.failed_files, 0);
        assert!(result.is_success());
    }

    /// 测试: 已迁移过则跳过
    #[test]
    fn test_already_migrated_skips() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("migrated_project");
        let new_data = tmp.path().join("new_data");

        // 创建迁移标记
        let marker = DataDir::migration_marker_path(&src);
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, "v2").unwrap();

        // 也创建旧版数据目录（但应被跳过）
        let legacy = DataDir::legacy_data_path(&src);
        fs::create_dir_all(&legacy).unwrap();

        let result = migrate_legacy_to_v2(&src, &new_data, false).expect("迁移不应报错");
        assert_eq!(result.migrated_files, 0);
        assert!(result.details.iter().any(|d| d.contains("已迁移过")));
    }

    /// 测试: 正常迁移流程
    #[test]
    fn test_successful_migration() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("project");
        let new_data = tmp.path().join("new_data");

        // 创建旧版数据目录和文件
        let legacy = DataDir::legacy_data_path(&src);
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("memories.json"), r#"{"test": "memory"}"#).unwrap();
        fs::write(legacy.join("chunks.json"), r#"{"test": "chunk"}"#).unwrap();
        // 创建非 JSON 文件（应被跳过）
        fs::write(legacy.join(".lrc.lock"), "12345").unwrap();

        let result = migrate_legacy_to_v2(&src, &new_data, false).expect("迁移不应报错");

        assert_eq!(result.migrated_files, 2, "应迁移 2 个 JSON 文件");
        assert_eq!(result.failed_files, 0, "不应有失败文件");
        assert!(result.is_success());

        // 验证文件已复制到新版目录
        assert!(new_data.join("memories.json").exists());
        assert!(new_data.join("chunks.json").exists());
        // 锁文件不应被迁移
        assert!(!new_data.join(".lrc.lock").exists());

        // 验证迁移标记
        assert!(DataDir::is_migrated(&src));

        // 验证旧数据仍然存在（复制非移动）
        assert!(legacy.join("memories.json").exists());
    }

    /// 测试: 新版目录已有文件时跳过
    #[test]
    fn test_skip_existing_files() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("project");
        let new_data = tmp.path().join("new_data");

        // 创建旧版数据
        let legacy = DataDir::legacy_data_path(&src);
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("memories.json"), "old data").unwrap();

        // 新版目录已有同名文件
        fs::create_dir_all(&new_data).unwrap();
        fs::write(new_data.join("memories.json"), "new data").unwrap();

        let result = migrate_legacy_to_v2(&src, &new_data, false).expect("迁移不应报错");

        assert_eq!(result.migrated_files, 0);
        assert_eq!(result.skipped_files, 1, "已有文件应被跳过");
        // 新数据不应被覆盖
        let content = fs::read_to_string(new_data.join("memories.json")).unwrap();
        assert_eq!(content, "new data", "新版数据不应被覆盖");
    }

    /// 测试: dry_run 模式不实际执行
    #[test]
    fn test_dry_run() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("project");
        let new_data = tmp.path().join("new_data");

        // 创建旧版数据
        let legacy = DataDir::legacy_data_path(&src);
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("memories.json"), "test").unwrap();

        let result = migrate_legacy_to_v2(&src, &new_data, true).expect("dry_run 不应报错");

        assert!(result.details.iter().any(|d| d.contains("DRY RUN")));
        // 实际文件不应被创建
        assert!(!new_data.join("memories.json").exists());
        // 迁移标记不应被写入
        assert!(!DataDir::is_migrated(&src));
    }

    /// 测试: needs_migration 检测
    #[test]
    fn test_needs_migration() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");

        // 空目录不需要迁移
        let empty = tmp.path().join("empty");
        assert!(!needs_migration(&empty));

        // 有旧数据但无迁移标记 → 需要迁移
        let unmirated = tmp.path().join("unmirated");
        let legacy = DataDir::legacy_data_path(&unmirated);
        fs::create_dir_all(&legacy).unwrap();
        assert!(needs_migration(&unmirated));

        // 写入迁移标记 → 不再需要迁移
        let marker = DataDir::migration_marker_path(&unmirated);
        fs::write(&marker, "v2").unwrap();
        assert!(!needs_migration(&unmirated));
    }

    /// 测试: 空旧版目录不迁移
    #[test]
    fn test_empty_legacy_dir() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let src = tmp.path().join("empty_legacy");
        let new_data = tmp.path().join("new_data");

        // 创建空的旧版数据目录
        let legacy = DataDir::legacy_data_path(&src);
        fs::create_dir_all(&legacy).unwrap();

        let result = migrate_legacy_to_v2(&src, &new_data, false).expect("迁移不应报错");
        assert_eq!(result.migrated_files, 0);
        assert!(result.details.iter().any(|d| d.contains("为空")));
    }

    /// 测试: format_migration_report 生成报告
    #[test]
    fn test_format_migration_report() {
        let result = MigrationResult {
            migrated_files: 3,
            skipped_files: 1,
            failed_files: 0,
            details: vec!["已迁移: memories.json (100 bytes)".to_string()],
        };
        let report = format_migration_report(&result);
        assert!(report.contains("3"));
        assert!(report.contains("1"));
        assert!(report.contains("memories.json"));
    }
}
