// ============================================================
// 许可证: Apache 2.0
// 本文件实现记忆数据迁移与合并工具，属于公开层 (Layer 1)。
// ============================================================
//
// v0.8.0 "归一" 专项：记忆数据迁移工具
//
// 功能：
//   1. 扫描已知老路径（项目指纹目录、老版本路径）
//   2. 按 memory.id 去重合并到 global 目录
//   3. 原文件重命名 .bak，不删除
//   4. 生成迁移报告
//
// 设计原则：
//   - 按 JSON 层面操作，兼容老版本格式差异
//   - 按 id 去重，保留最新 updated_at 的版本
//   - 原文件保留 .bak 备份，确保数据安全
//   - 龙老版本（G:\loong\data\memory\）格式不兼容，跳过

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 迁移源描述
#[derive(Debug, Clone)]
pub struct MigrationSource {
    /// 数据源路径（memories.json 所在目录）
    pub data_dir: PathBuf,
    /// 源类型描述
    pub source_type: String,
    /// 是否为 global 目录（global 不迁移，仅作为合并目标）
    pub is_global: bool,
}

/// 迁移结果报告
#[derive(Debug, Clone, serde::Serialize)]
pub struct MigrationReport {
    /// 扫描到的数据源数量
    pub sources_scanned: usize,
    /// 各源详情
    pub sources: Vec<SourceReport>,
    /// 合并前 global 记忆数
    pub global_before: usize,
    /// 合并后 global 记忆数
    pub global_after: usize,
    /// 新增记忆数（去重后）
    pub memories_added: usize,
    /// 跳过的重复记忆数
    pub duplicates_skipped: usize,
    /// 备份的文件数
    pub files_backed_up: usize,
    /// 是否成功
    pub success: bool,
    /// 错误信息（如有）
    pub error: Option<String>,
}

/// 单个数据源的迁移报告
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceReport {
    /// 数据目录路径
    pub data_dir: String,
    /// 源类型
    pub source_type: String,
    /// 该源的記憶数
    pub memory_count: usize,
    /// 该源新增的记忆数（去重后）
    pub added: usize,
    /// 该源重复的记忆数
    pub duplicates: usize,
    /// 是否已备份
    pub backed_up: bool,
    /// 处理状态
    pub status: String,
}

/// 扫描已知老路径，返回所有可能含记忆数据的目录
///
/// 扫描路径：
/// 1. ~/.loong-recall/projects/*/data/（项目指纹目录）
/// 2. G:\data\code-memory\（老版本路径）
/// 3. ~/.loong-recall/global/data/（global 目录，仅作为合并目标）
pub fn scan_legacy_sources() -> Vec<MigrationSource> {
    let mut sources = Vec::new();
    let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("."));

    // 1. 扫描项目指纹目录
    let projects_dir = home.join(".loong-recall").join("projects");
    if projects_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&projects_dir) {
            for entry in entries.flatten() {
                let fingerprint_dir = entry.path();
                let data_dir = fingerprint_dir.join("data");
                let memory_file = data_dir.join("memories.json");
                if memory_file.exists() {
                    sources.push(MigrationSource {
                        data_dir: data_dir.clone(),
                        source_type: format!("项目指纹({})", fingerprint_dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                        is_global: false,
                    });
                }
            }
        }
    }

    // 2. 扫描老版本路径 G:\data\code-memory\
    let legacy_path = PathBuf::from(r"G:\data\code-memory");
    if legacy_path.join("memories.json").exists() {
        sources.push(MigrationSource {
            data_dir: legacy_path,
            source_type: "老版本路径(G:\\data\\code-memory)".to_string(),
            is_global: false,
        });
    }

    // 3. global 目录（仅作为合并目标，不迁移）
    let global_dir = home.join(".loong-recall").join("global").join("data");
    if global_dir.join("memories.json").exists() {
        sources.push(MigrationSource {
            data_dir: global_dir,
            source_type: "全局目录(global)".to_string(),
            is_global: true,
        });
    }

    sources
}

/// 从 memories.json 文件读取记忆列表（JSON 层面，兼容老版本格式）
fn read_memories(data_dir: &Path) -> Result<Vec<serde_json::Value>, String> {
    let memory_file = data_dir.join("memories.json");
    if !memory_file.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&memory_file)
        .map_err(|e| format!("读取 {} 失败: {}", memory_file.display(), e))?;
    // 兼容两种格式：Vec<Memory> 或单个 Memory 对象
    let memories: Vec<serde_json::Value> = if content.trim_start().starts_with('[') {
        serde_json::from_str(&content)
            .map_err(|e| format!("解析 {} 失败: {}", memory_file.display(), e))?
    } else {
        // 单对象格式，包装为数组
        let single: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("解析 {} 失败: {}", memory_file.display(), e))?;
        vec![single]
    };
    Ok(memories)
}

/// 获取记忆的 id 和 updated_at（用于去重）
fn get_id_and_updated(mem: &serde_json::Value) -> (String, String) {
    let id = mem.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let updated = mem.get("updated_at")
        .and_then(|v| v.as_str())
        .or_else(|| mem.get("created_at").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    (id, updated)
}

/// 执行迁移与合并
///
/// 流程：
/// 1. 扫描所有数据源
/// 2. 读取 global 目录现有记忆作为基准
/// 3. 逐个读取其他源，按 id 去重合并
/// 4. 写入 global 目录
/// 5. 非 global 源文件重命名 .bak
pub fn execute_migration() -> MigrationReport {
    let mut report = MigrationReport {
        sources_scanned: 0,
        sources: Vec::new(),
        global_before: 0,
        global_after: 0,
        memories_added: 0,
        duplicates_skipped: 0,
        files_backed_up: 0,
        success: false,
        error: None,
    };

    let sources = scan_legacy_sources();
    report.sources_scanned = sources.len();

    // 按 id 去重的记忆池：id -> (记忆, 来源, updated_at)
    let mut memory_pool: HashMap<String, (serde_json::Value, String, String)> = HashMap::new();

    // 逐个处理数据源
    for source in &sources {
        match read_memories(&source.data_dir) {
            Ok(memories) => {
                let count = memories.len();
                let mut added = 0usize;
                let mut duplicates = 0usize;

                for mem in &memories {
                    let (id, updated) = get_id_and_updated(mem);
                    if id.is_empty() {
                        // 无 id 的记忆直接保留
                        let fake_id = format!("no-id-{}", uuid::Uuid::new_v4());
                        memory_pool.insert(fake_id, (mem.clone(), source.source_type.clone(), updated.clone()));
                        added += 1;
                        continue;
                    }
                    match memory_pool.get(&id) {
                        Some(existing) => {
                            // 比较 updated_at，保留最新的
                            if updated > existing.2 {
                                memory_pool.insert(id, (mem.clone(), source.source_type.clone(), updated));
                            }
                            duplicates += 1;
                        }
                        None => {
                            memory_pool.insert(id, (mem.clone(), source.source_type.clone(), updated));
                            added += 1;
                        }
                    }
                }

                if source.is_global {
                    report.global_before = count;
                }

                report.sources.push(SourceReport {
                    data_dir: source.data_dir.to_string_lossy().to_string(),
                    source_type: source.source_type.clone(),
                    memory_count: count,
                    added,
                    duplicates,
                    backed_up: false,
                    status: "已读取".to_string(),
                });
            }
            Err(e) => {
                report.sources.push(SourceReport {
                    data_dir: source.data_dir.to_string_lossy().to_string(),
                    source_type: source.source_type.clone(),
                    memory_count: 0,
                    added: 0,
                    duplicates: 0,
                    backed_up: false,
                    status: format!("读取失败: {}", e),
                });
            }
        }
    }

    // 合并后的记忆列表（按 created_at 排序）
    let mut merged: Vec<serde_json::Value> = memory_pool.into_values().map(|(m, _, _)| m).collect();
    merged.sort_by(|a, b| {
        let a_time = a.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        let b_time = b.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        a_time.cmp(&b_time)
    });

    report.global_after = merged.len();
    report.memories_added = report.global_after.saturating_sub(report.global_before);

    // 写入 global 目录
    let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let global_dir = home.join(".loong-recall").join("global").join("data");
    if let Err(e) = std::fs::create_dir_all(&global_dir) {
        report.error = Some(format!("创建 global 目录失败: {}", e));
        return report;
    }

    let global_file = global_dir.join("memories.json");
    let merged_json = match serde_json::to_string_pretty(&merged) {
        Ok(s) => s,
        Err(e) => {
            report.error = Some(format!("序列化合并记忆失败: {}", e));
            return report;
        }
    };
    if let Err(e) = std::fs::write(&global_file, merged_json) {
        report.error = Some(format!("写入 global memories.json 失败: {}", e));
        return report;
    }

    // 非 global 源文件重命名 .bak
    for source in &sources {
        if source.is_global {
            continue;
        }
        let memory_file = source.data_dir.join("memories.json");
        if !memory_file.exists() {
            continue;
        }
        let bak_file = memory_file.with_extension("json.bak");
        match std::fs::rename(&memory_file, &bak_file) {
            Ok(()) => {
                report.files_backed_up += 1;
                // 更新对应 source 的 backed_up 状态
                for s in &mut report.sources {
                    if s.data_dir == source.data_dir.to_string_lossy().to_string() {
                        s.backed_up = true;
                        s.status = "已备份(.bak)".to_string();
                    }
                }
            }
            Err(e) => {
                for s in &mut report.sources {
                    if s.data_dir == source.data_dir.to_string_lossy().to_string() {
                        s.status = format!("备份失败: {}", e);
                    }
                }
            }
        }
    }

    // 清理空项目目录（只有 .lrc.lock，无 memories.json）
    let projects_dir = home.join(".loong-recall").join("projects");
    if projects_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&projects_dir) {
            for entry in entries.flatten() {
                let fp_dir = entry.path();
                let data_dir = fp_dir.join("data");
                let mem_file = data_dir.join("memories.json");
                let bak_file = data_dir.join("memories.json.bak");
                // 既无 memories.json 也无 .bak，且只有 .lrc.lock → 空目录
                if !mem_file.exists() && !bak_file.exists() {
                    let lock_file = data_dir.join(".lrc.lock");
                    if lock_file.exists() {
                        let _ = std::fs::remove_dir_all(&fp_dir);
                    }
                }
            }
        }
    }

    report.success = report.error.is_none();

    // v0.8.0 "归一"：记录数据操作日志
    if report.success {
        let details = format!(
            "扫描 {} 处源，合并 {} 条记忆至 global（新增 {}，备份 {} 文件）",
            report.sources_scanned,
            report.global_after,
            report.memories_added,
            report.files_backed_up
        );
        crate::data_log::log_operation(
            crate::data_log::OperationType::Migrate,
            &details,
        );
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_id_and_updated() {
        let mem = serde_json::json!({
            "id": "test-001",
            "updated_at": "2026-07-29T10:00:00Z",
            "content": "测试"
        });
        let (id, updated) = get_id_and_updated(&mem);
        assert_eq!(id, "test-001");
        assert_eq!(updated, "2026-07-29T10:00:00Z");
    }

    #[test]
    fn test_get_id_fallback_to_created_at() {
        // 无 updated_at 时回退到 created_at
        let mem = serde_json::json!({
            "id": "test-002",
            "created_at": "2026-07-28T00:00:00Z"
        });
        let (id, updated) = get_id_and_updated(&mem);
        assert_eq!(id, "test-002");
        assert_eq!(updated, "2026-07-28T00:00:00Z");
    }

    #[test]
    fn test_get_id_empty() {
        let mem = serde_json::json!({"content": "无 id"});
        let (id, _) = get_id_and_updated(&mem);
        assert_eq!(id, "");
    }

    #[test]
    fn test_read_memories_array_format() {
        // 标准数组格式
        let dir = std::env::temp_dir().join("lrc_migration_test_array");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("memories.json"),
            r#"[{"id":"a","content":"A"},{"id":"b","content":"B"}]"#,
        ).unwrap();
        let memories = read_memories(&dir).unwrap();
        assert_eq!(memories.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_read_memories_single_object_format() {
        // 龙老版本单对象格式
        let dir = std::env::temp_dir().join("lrc_migration_test_single");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("memories.json"),
            r#"{"id":"a","content":"A","long_term":true}"#,
        ).unwrap();
        let memories = read_memories(&dir).unwrap();
        assert_eq!(memories.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_read_memories_nonexistent() {
        let dir = std::env::temp_dir().join("lrc_migration_test_nonexist");
        let memories = read_memories(&dir).unwrap();
        assert_eq!(memories.len(), 0);
    }
}
