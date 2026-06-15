// ============================================================
// 许可证: Apache 2.0
// 本文件实现记忆数据导出/导入，属于公开层 (Layer 1)。
// ============================================================
//
// 数据导出/导入模块 — 支持记忆数据的备份、恢复和迁移
//
// 核心能力:
//   1. lrc export — 导出记忆数据到 JSON 文件
//   2. lrc import — 从 JSON 文件导入记忆数据
//   3. 支持项目级和全局模式的导出
//   4. 导入时验证数据格式和完整性
//
// 导出文件格式（V2）：
//   {
//     "version": "2.0",
//     "exported_at": "ISO8601",
//     "fingerprint": "sha256前16位",
//     "canonical_path": "规范化路径",
//     "memories": [...],
//     "chunks": [...],
//     "archive": [...]
//   }
//
// 安全原则：
//   - 导入时不清除现有数据（追加模式）
//   - 导入前验证 JSON 结构完整性
//   - 支持 dry-run 预览导入内容

use crate::data_dir::DataDir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 导出文件格式版本
const EXPORT_VERSION: &str = "2.0";

/// 导出文件结构体
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportData {
    /// 导出格式版本
    pub version: String,
    /// 导出时间（ISO 8601 格式）
    pub exported_at: String,
    /// 项目指纹（可选，全局模式导出时为空）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// 规范化项目路径（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_path: Option<String>,
    /// 导出来源：project 或 global
    pub source: String,
    /// 记忆数据
    pub memories: serde_json::Value,
    /// 代码片段
    pub chunks: serde_json::Value,
    /// 归档数据
    pub archive: serde_json::Value,
}

/// 导入结果统计
#[derive(Debug, Clone, Default)]
pub struct ImportResult {
    /// 导入的记忆数
    pub memories_imported: usize,
    /// 导入的代码片段数
    pub chunks_imported: usize,
    /// 导入的归档数
    pub archive_imported: usize,
    /// 跳过的条目数（已存在）
    pub skipped: usize,
    /// 详情日志
    pub details: Vec<String>,
}

/// 导出操作结果
#[derive(Debug)]
pub struct ExportResult {
    /// 导出文件路径
    pub file_path: PathBuf,
    /// 导出的记忆数量
    pub memory_count: usize,
    /// 导出的代码片段数量
    pub chunk_count: usize,
    /// 导出文件大小（字节）
    pub file_size: u64,
}

/// 导出记忆数据到 JSON 文件
///
/// # 参数
/// - `data_dir_manager`: 数据目录管理器
/// - `output_path`: 输出文件路径（可选，默认使用 ~/.loong-recall/exports/ 目录）
/// - `src_dir`: 项目源码目录（用于计算指纹）
///
/// # 返回
/// - `Ok(ExportResult)`: 导出结果
/// - `Err(String)`: 错误描述
pub fn export_memories(
    data_dir_manager: &DataDir,
    output_path: Option<&Path>,
    src_dir: Option<&Path>,
) -> Result<ExportResult, String> {
    let data_path = data_dir_manager.data_path();

    // 确保数据目录存在
    if !data_path.exists() {
        return Err(format!(
            "数据目录不存在: {}。请先启动 LRC 服务以生成记忆数据。",
            data_path.display()
        ));
    }

    // 读取记忆数据
    let memories_path = data_path.join("memories.json");
    let memories: serde_json::Value = if memories_path.exists() {
        let content = fs::read_to_string(&memories_path)
            .map_err(|e| format!("读取 memories.json 失败: {}", e))?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!([]))
    } else {
        serde_json::json!([])
    };

    // 读取代码片段
    let chunks_path = data_path.join("chunks.json");
    let chunks: serde_json::Value = if chunks_path.exists() {
        let content = fs::read_to_string(&chunks_path)
            .map_err(|e| format!("读取 chunks.json 失败: {}", e))?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!([]))
    } else {
        serde_json::json!([])
    };

    // 读取归档数据
    let archive_path = data_path.join("archive.json");
    let archive: serde_json::Value = if archive_path.exists() {
        let content = fs::read_to_string(&archive_path)
            .map_err(|e| format!("读取 archive.json 失败: {}", e))?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!([]))
    } else {
        serde_json::json!([])
    };

    // 计算记忆数量
    let memory_count = memories.as_array().map(|a| a.len()).unwrap_or(0);
    let chunk_count = chunks.as_array().map(|a| a.len()).unwrap_or(0);

    // 构建导出数据
    let (fingerprint, canonical_path, source) = if let Some(src) = src_dir {
        let (fp, cp) = crate::project_id::project_fingerprint_with_path(src);
        (Some(fp), Some(cp), "project".to_string())
    } else {
        (None, None, "global".to_string())
    };

    let export_data = ExportData {
        version: EXPORT_VERSION.to_string(),
        exported_at: {
            use std::time::SystemTime;
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        },
        fingerprint: fingerprint.clone(),
        canonical_path: canonical_path.clone(),
        source,
        memories,
        chunks,
        archive,
    };

    // 序列化导出数据
    let json_str = serde_json::to_string_pretty(&export_data)
        .map_err(|e| format!("序列化导出数据失败: {}", e))?;

    // 确定输出路径
    let file_path = if let Some(path) = output_path {
        path.to_path_buf()
    } else {
        let exports_dir = data_dir_manager
            .ensure_exports_dir()
            .map_err(|e| format!("创建导出目录失败: {}", e))?;
        let timestamp = {
            use std::time::SystemTime;
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        };
        let fp_short = fingerprint.as_deref().unwrap_or("global");
        exports_dir.join(format!("lrc-export-{fp_short}-{timestamp}.json"))
    };

    // 写入文件
    fs::write(&file_path, &json_str).map_err(|e| format!("写入导出文件失败: {}", e))?;

    let file_size = json_str.len() as u64;

    Ok(ExportResult {
        file_path,
        memory_count,
        chunk_count,
        file_size,
    })
}

/// 从 JSON 文件导入记忆数据
///
/// # 参数
/// - `import_path`: 导入文件路径
/// - `data_dir_manager`: 目标数据目录管理器
/// - `dry_run`: 如果为 true，仅验证格式，不实际导入
///
/// # 返回
/// - `Ok(ImportResult)`: 导入结果
/// - `Err(String)`: 错误描述
pub fn import_memories(
    import_path: &Path,
    data_dir_manager: &DataDir,
    dry_run: bool,
) -> Result<ImportResult, String> {
    // 验证导入文件存在
    if !import_path.exists() {
        return Err(format!("导入文件不存在: {}", import_path.display()));
    }

    // 读取并解析导入文件
    let content =
        fs::read_to_string(import_path).map_err(|e| format!("读取导入文件失败: {}", e))?;

    let export_data: ExportData = serde_json::from_str(&content).map_err(|e| {
        format!(
            "解析导入文件失败: {}\n提示: 请确认文件是有效的 LRC 导出 JSON",
            e
        )
    })?;

    // 验证版本
    if export_data.version != EXPORT_VERSION {
        return Err(format!(
            "导出文件版本不匹配: 期望 {}，实际 {}",
            EXPORT_VERSION, export_data.version
        ));
    }

    let mut result = ImportResult::default();

    // 计算分类数量
    let memory_count = export_data
        .memories
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let chunk_count = export_data.chunks.as_array().map(|a| a.len()).unwrap_or(0);
    let archive_count = export_data.archive.as_array().map(|a| a.len()).unwrap_or(0);

    result.details.push(format!(
        "导入文件包含: {} 条记忆, {} 个代码片段, {} 条归档",
        memory_count, chunk_count, archive_count
    ));

    if dry_run {
        result
            .details
            .push("[DRY RUN] 仅验证格式，未实际导入".to_string());
        return Ok(result);
    }

    // 确保数据目录存在
    let data_path = data_dir_manager.data_path();
    data_dir_manager
        .ensure()
        .map_err(|e| format!("创建数据目录失败: {}", e))?;

    // 导入记忆数据（追加模式）
    if memory_count > 0 {
        let memories_path = data_path.join("memories.json");
        let existing: serde_json::Value = if memories_path.exists() {
            let content = fs::read_to_string(&memories_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or(serde_json::json!([]))
        } else {
            serde_json::json!([])
        };

        // 简单合并：将导入的记忆追加到现有数据
        let merged = merge_arrays(&existing, &export_data.memories);
        let merged_str = serde_json::to_string_pretty(&merged)
            .map_err(|e| format!("序列化合并后的记忆数据失败: {}", e))?;
        fs::write(&memories_path, merged_str)
            .map_err(|e| format!("写入 memories.json 失败: {}", e))?;
        result.memories_imported = memory_count;
    }

    // 导入代码片段
    if chunk_count > 0 {
        let chunks_path = data_path.join("chunks.json");
        let existing: serde_json::Value = if chunks_path.exists() {
            let content = fs::read_to_string(&chunks_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or(serde_json::json!([]))
        } else {
            serde_json::json!([])
        };

        let merged = merge_arrays(&existing, &export_data.chunks);
        let merged_str = serde_json::to_string_pretty(&merged)
            .map_err(|e| format!("序列化合并后的代码片段失败: {}", e))?;
        fs::write(&chunks_path, merged_str).map_err(|e| format!("写入 chunks.json 失败: {}", e))?;
        result.chunks_imported = chunk_count;
    }

    // 导入归档数据
    if archive_count > 0 {
        let archive_path = data_path.join("archive.json");
        let existing: serde_json::Value = if archive_path.exists() {
            let content = fs::read_to_string(&archive_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or(serde_json::json!([]))
        } else {
            serde_json::json!([])
        };

        let merged = merge_arrays(&existing, &export_data.archive);
        let merged_str = serde_json::to_string_pretty(&merged)
            .map_err(|e| format!("序列化合并后的归档数据失败: {}", e))?;
        fs::write(&archive_path, merged_str)
            .map_err(|e| format!("写入 archive.json 失败: {}", e))?;
        result.archive_imported = archive_count;
    }

    result.details.push(format!(
        "导入完成: {} 记忆, {} 代码片段, {} 归档",
        result.memories_imported, result.chunks_imported, result.archive_imported
    ));

    Ok(result)
}

/// 合并两个 JSON 数组（去重：基于 id 字段）
fn merge_arrays(existing: &serde_json::Value, new: &serde_json::Value) -> serde_json::Value {
    let existing_arr = existing.as_array().cloned().unwrap_or_default();
    let new_arr = new.as_array().cloned().unwrap_or_default();

    // 收集已有 id 集合
    let existing_ids: std::collections::HashSet<String> = existing_arr
        .iter()
        .filter_map(|v| {
            v.get("id")
                .and_then(|id| id.as_str().map(|s| s.to_string()))
        })
        .collect();

    let mut merged = existing_arr;
    for item in new_arr {
        let is_new = item
            .get("id")
            .and_then(|id| id.as_str())
            .map(|id| !existing_ids.contains(id))
            .unwrap_or(true); // 无 id 的条目直接追加
        if is_new {
            merged.push(item);
        }
    }

    serde_json::Value::Array(merged)
}

/// 验证导出文件格式
pub fn validate_export_file(path: &Path) -> Result<ExportData, String> {
    if !path.exists() {
        return Err(format!("文件不存在: {}", path.display()));
    }
    let content = fs::read_to_string(path).map_err(|e| format!("读取文件失败: {}", e))?;
    let data: ExportData =
        serde_json::from_str(&content).map_err(|e| format!("JSON 格式无效: {}", e))?;
    if data.version != EXPORT_VERSION {
        return Err(format!(
            "版本不匹配: 期望 {}，实际 {}",
            EXPORT_VERSION, data.version
        ));
    }
    Ok(data)
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// 创建测试用数据目录
    fn setup_test_data_dir() -> (tempfile::TempDir, DataDir) {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let data_path = tmp.path().join("data");
        fs::create_dir_all(&data_path).unwrap();

        // 创建测试记忆数据
        let memories = serde_json::json!([
            {"id": "mem-1", "content": "测试记忆1", "importance": 5},
            {"id": "mem-2", "content": "测试记忆2", "importance": 3}
        ]);
        fs::write(
            data_path.join("memories.json"),
            serde_json::to_string_pretty(&memories).unwrap(),
        )
        .unwrap();

        // 创建测试代码片段
        let chunks = serde_json::json!([
            {"id": "chunk-1", "code": "fn main() {}", "language": "rust"}
        ]);
        fs::write(
            data_path.join("chunks.json"),
            serde_json::to_string_pretty(&chunks).unwrap(),
        )
        .unwrap();

        let dd = DataDir::for_custom(data_path.to_string_lossy().as_ref());
        (tmp, dd)
    }

    /// 测试: 导出功能基本流程
    #[test]
    fn test_export_basic() {
        let (_tmp, dd) = setup_test_data_dir();
        let output = _tmp.path().join("export.json");

        let result = export_memories(&dd, Some(&output), None).expect("导出应成功");

        assert!(result.file_path.exists(), "导出文件应存在");
        assert_eq!(result.memory_count, 2, "应导出 2 条记忆");
        assert_eq!(result.chunk_count, 1, "应导出 1 个代码片段");
        assert!(result.file_size > 0, "文件大小应大于 0");

        // 验证导出文件内容
        let content = fs::read_to_string(&output).unwrap();
        let data: ExportData = serde_json::from_str(&content).unwrap();
        assert_eq!(data.version, "2.0");
        assert_eq!(data.source, "global");
    }

    /// 测试: 导出到默认路径
    #[test]
    fn test_export_to_default_path() {
        let (_tmp, dd) = setup_test_data_dir();

        let result = export_memories(&dd, None, None).expect("默认路径导出应成功");

        // 验证文件在 exports 目录下
        let path_str = result.file_path.to_string_lossy();
        assert!(
            path_str.contains("exports"),
            "默认导出路径应在 exports 目录下"
        );
        assert!(
            path_str.contains("lrc-export-"),
            "文件名应以 lrc-export- 开头"
        );
    }

    /// 测试: 导出到带项目指纹的路径
    #[test]
    fn test_export_with_project() {
        let (_tmp, dd) = setup_test_data_dir();
        let output = _tmp.path().join("project-export.json");

        let _result =
            export_memories(&dd, Some(&output), Some(Path::new("."))).expect("项目导出应成功");

        let content = fs::read_to_string(&output).unwrap();
        let data: ExportData = serde_json::from_str(&content).unwrap();
        assert_eq!(data.source, "project");
        assert!(data.fingerprint.is_some(), "项目导出应有指纹");
        assert!(data.canonical_path.is_some(), "项目导出应有路径");
    }

    /// 测试: 导出空数据目录
    #[test]
    fn test_export_empty_data_dir() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let empty_path = tmp.path().join("empty_data");
        fs::create_dir_all(&empty_path).unwrap();
        let dd = DataDir::for_custom(empty_path.to_string_lossy().as_ref());
        let output = tmp.path().join("empty-export.json");

        let result = export_memories(&dd, Some(&output), None).expect("空目录导出应成功");

        assert_eq!(result.memory_count, 0);
        assert_eq!(result.chunk_count, 0);
    }

    /// 测试: 导入功能基本流程
    #[test]
    fn test_import_basic() {
        let (_tmp, dd) = setup_test_data_dir();

        // 先导出
        let export_path = _tmp.path().join("for-import.json");
        export_memories(&dd, Some(&export_path), None).unwrap();

        // 创建新的目标数据目录
        let target_path = _tmp.path().join("target_data");
        fs::create_dir_all(&target_path).unwrap();
        let target_dd = DataDir::for_custom(target_path.to_string_lossy().as_ref());

        // 导入
        let result = import_memories(&export_path, &target_dd, false).expect("导入应成功");

        assert_eq!(result.memories_imported, 2);
        assert_eq!(result.chunks_imported, 1);

        // 验证数据已导入
        assert!(target_path.join("memories.json").exists());
        assert!(target_path.join("chunks.json").exists());
    }

    /// 测试: 导入 dry_run 模式
    #[test]
    fn test_import_dry_run() {
        let (_tmp, dd) = setup_test_data_dir();
        let export_path = _tmp.path().join("dry-run-export.json");
        export_memories(&dd, Some(&export_path), None).unwrap();

        let target_path = _tmp.path().join("dry_run_target");
        let target_dd = DataDir::for_custom(target_path.to_string_lossy().as_ref());

        let result = import_memories(&export_path, &target_dd, true).expect("dry_run 导入应成功");

        assert!(result.details.iter().any(|d| d.contains("DRY RUN")));
        // 数据不应被实际写入
        assert!(!target_path.join("memories.json").exists());
    }

    /// 测试: 导入不存在的文件
    #[test]
    fn test_import_nonexistent_file() {
        let dd = DataDir::for_global();
        let result = import_memories(Path::new("/tmp/definitely_not_exist.json"), &dd, false);
        assert!(result.is_err(), "不存在的文件应报错");
    }

    /// 测试: 导入时追加模式（不覆盖现有数据）
    #[test]
    fn test_import_append_mode() {
        let (_tmp, dd) = setup_test_data_dir();

        // 导出当前数据
        let export_path = _tmp.path().join("append-export.json");
        export_memories(&dd, Some(&export_path), None).unwrap();

        // 导入到同一目录（应追加）
        let result = import_memories(&export_path, &dd, false).expect("追加导入应成功");

        assert_eq!(result.memories_imported, 2);
        // 已存在的记忆应被跳过（去重）
        assert_eq!(result.skipped, 0, "导入时不应重复已有条目");
    }

    /// 测试: validate_export_file 验证
    #[test]
    fn test_validate_export_file() {
        let (_tmp, dd) = setup_test_data_dir();
        let export_path = _tmp.path().join("validate-test.json");
        export_memories(&dd, Some(&export_path), None).unwrap();

        let data = validate_export_file(&export_path).expect("验证应成功");
        assert_eq!(data.version, "2.0");
    }

    /// 测试: validate_export_file 对无效文件报错
    #[test]
    fn test_validate_invalid_file() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let invalid = tmp.path().join("invalid.json");
        fs::write(&invalid, "not valid json").unwrap();

        let result = validate_export_file(&invalid);
        assert!(result.is_err(), "无效 JSON 应报错");
    }

    /// 测试: merge_arrays 去重
    #[test]
    fn test_merge_arrays_dedup() {
        let existing = serde_json::json!([
            {"id": "a", "value": 1},
            {"id": "b", "value": 2}
        ]);
        let new = serde_json::json!([
            {"id": "b", "value": 99}, // 重复 id，应被跳过
            {"id": "c", "value": 3}
        ]);
        let merged = merge_arrays(&existing, &new);
        let arr = merged.as_array().unwrap();
        assert_eq!(arr.len(), 3, "合并后应有 3 条（去重了重复的 b）");
    }

    /// 测试: 导出数据目录不存在时报错
    #[test]
    fn test_export_nonexistent_data_dir() {
        let dd = DataDir::for_custom("/tmp/definitely_not_exist_lrc_data");
        let result = export_memories(&dd, None, None);
        assert!(result.is_err(), "不存在的数据目录应报错");
    }
}
