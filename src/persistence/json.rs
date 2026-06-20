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

use super::{Persistence, PersistenceError};
use crate::chunker::CodeChunk;
use crate::memory_types::Memory;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

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
    /// 代码片段文件路径
    chunks_file: PathBuf,
    /// 归档记忆文件路径
    archive_file: PathBuf,
    /// 记忆缓存：避免每次写操作都全量读取 JSON 文件
    /// - `None`：缓存未初始化或已失效
    /// - `Some(vec)`：已加载的记忆列表
    cache: RwLock<Option<Vec<Memory>>>,
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
            chunks_file: dir.join("chunks.json"),
            archive_file: dir.join("archive.json"),
            data_dir: dir,
            cache: RwLock::new(None),
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
    #[allow(dead_code)]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 直接从磁盘加载记忆（不经过缓存，供 load_all_memories 使用）
    fn load_all_memories_from_disk(&self) -> Result<Vec<Memory>, PersistenceError> {
        if !self.memories_file.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.memories_file)?;
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }
        let memories: Vec<Memory> = serde_json::from_str(&content)?;
        Ok(memories)
    }

    /// 确保缓存已加载（懒加载）
    /// 首次调用时从磁盘读取，后续直接使用缓存
    fn ensure_cache_loaded(&self) -> Result<(), PersistenceError> {
        {
            // v0.5.4 修复 C04：RwLock 毒化恢复，避免一个 panic 导致持久化层瘫痪
            let cache = self.cache.read().unwrap_or_else(|e| e.into_inner());
            if cache.is_some() {
                return Ok(());
            }
        }
        // 写锁：加载数据
        let memories = self.load_all_memories_from_disk()?;
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = Some(memories);
        Ok(())
    }

    /// 使缓存失效（当外部修改文件时调用）
    #[allow(dead_code)]
    pub fn invalidate_cache(&self) {
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

impl Persistence for JsonPersistence {
    fn save_memory(&self, memory: &Memory) -> Result<(), PersistenceError> {
        // 防御性检查：确保数据目录存在（应对临时目录被清理等场景）
        self.ensure_data_dir()?;

        // 使用缓存优化：避免每次全量读取+反序列化 JSON 文件
        self.ensure_cache_loaded()?;
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let memories = cache.as_mut().expect("缓存已通过 ensure_cache_loaded 初始化");

        // 按 ID 查找并更新，或追加新记忆
        if let Some(existing) = memories.iter_mut().find(|m| m.id == memory.id) {
            *existing = memory.clone();
        } else {
            memories.push(memory.clone());
        }

        let json = serde_json::to_string_pretty(memories)?;
        drop(cache); // 释放写锁
        atomic_write(&self.memories_file, &json)?;
        Ok(())
    }

    fn load_all_memories(&self) -> Result<Vec<Memory>, PersistenceError> {
        // 优先从缓存读取（O(1)），缓存失效时从磁盘加载（O(n)）
        {
            let cache = self.cache.read().unwrap_or_else(|e| e.into_inner());
            if let Some(ref cached) = *cache {
                return Ok(cached.clone());
            }
        }
        let memories = self.load_all_memories_from_disk()?;
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = Some(memories.clone());
        Ok(memories)
    }

    fn delete_memory(&self, id: &str) -> Result<bool, PersistenceError> {
        self.ensure_data_dir()?;

        // 使用缓存优化：避免全量读取
        self.ensure_cache_loaded()?;
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let memories = cache.as_mut().expect("缓存已通过 ensure_cache_loaded 初始化");
        let original_len = memories.len();
        memories.retain(|m| m.id != id);

        if memories.len() == original_len {
            return Ok(false); // 未找到
        }

        let json = serde_json::to_string_pretty(memories)?;
        drop(cache);
        atomic_write(&self.memories_file, &json)?;
        Ok(true)
    }

    fn clear_memories(&self) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        // 清空缓存
        *self.cache.write().unwrap_or_else(|e| e.into_inner()) = Some(Vec::new());
        let empty: Vec<Memory> = Vec::new();
        let json = serde_json::to_string_pretty(&empty)?;
        atomic_write(&self.memories_file, &json)?;
        Ok(())
    }

    fn save_chunks(&self, chunks: &[CodeChunk]) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
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
        let json = serde_json::to_string_pretty(memories)?;
        atomic_write(&self.archive_file, &json)?;
        Ok(())
    }

    fn add_to_archive(&self, memories: &[Memory]) -> Result<(), PersistenceError> {
        self.ensure_data_dir()?;
        let mut existing = self.load_archived_memories()?;

        // 按 ID 去重，避免重复归档
        let existing_ids: std::collections::HashSet<String> =
            existing.iter().map(|m| m.id.clone()).collect();
        for m in memories {
            if !existing_ids.contains(&m.id) {
                existing.push(m.clone());
            }
        }

        self.save_archived_memories(&existing)
    }

    fn delete_from_archive(&self, id: &str) -> Result<bool, PersistenceError> {
        self.ensure_data_dir()?;
        let mut archived = self.load_archived_memories()?;
        let original_len = archived.len();
        archived.retain(|m| m.id != id);

        if archived.len() == original_len {
            return Ok(false);
        }

        self.save_archived_memories(&archived)?;
        Ok(true)
    }
}

// === 原子写入辅助函数 ===

/// 原子写入：先写临时文件，再重命名（同文件系统内是原子操作）
/// 防止崩溃时产生损坏的 JSON 文件
fn atomic_write(path: &Path, content: &str) -> Result<(), PersistenceError> {
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
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

    #[test]
    fn test_load_empty() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = JsonPersistence::new(&data_dir).expect("应成功创建");

        let loaded = p.load_all_memories().expect("应成功加载");
        assert!(loaded.is_empty());
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
}
