// ============================================================
// 许可证: Apache 2.0
// 本文件定义持久化抽象层，属于公开层 (Layer 1)。
// ============================================================
//
// 持久化层
//
// 定义记忆和代码片段的持久化 trait。
// 默认实现为 JSON 文件存储，后续可扩展 SQLite/Redis 等后端。

use crate::chunker::CodeChunk;
use crate::memory_types::Memory;
use std::error::Error;
use std::fmt;

pub mod json;

/// 持久化错误类型
#[derive(Debug)]
pub enum PersistenceError {
    /// IO 错误
    Io(std::io::Error),
    /// 序列化/反序列化错误
    Serialization(serde_json::Error),
    /// 记忆不存在
    NotFound(String),
    /// 其他错误
    Other(String),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO 错误: {}", e),
            Self::Serialization(e) => write!(f, "序列化错误: {}", e),
            Self::NotFound(id) => write!(f, "记忆不存在: {}", id),
            Self::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl Error for PersistenceError {}

impl From<std::io::Error> for PersistenceError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for PersistenceError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e)
    }
}

/// 持久化存储抽象 trait
///
/// 定义记忆和代码片段的 CRUD 操作接口。
/// 实现此 trait 即可接入不同的存储后端（JSON、SQLite、Redis 等）。
pub trait Persistence: Send + Sync {
    /// 保存一条记忆（新增或更新）
    fn save_memory(&self, memory: &Memory) -> Result<(), PersistenceError>;

    /// 加载所有记忆
    fn load_all_memories(&self) -> Result<Vec<Memory>, PersistenceError>;

    /// 按 ID 删除记忆，返回是否成功删除
    fn delete_memory(&self, id: &str) -> Result<bool, PersistenceError>;

    /// 清空所有记忆
    fn clear_memories(&self) -> Result<(), PersistenceError>;

    /// 保存代码片段
    fn save_chunks(&self, chunks: &[CodeChunk]) -> Result<(), PersistenceError>;

    /// 加载所有代码片段
    fn load_chunks(&self) -> Result<Vec<CodeChunk>, PersistenceError>;

    /// 清空代码片段
    fn clear_chunks(&self) -> Result<(), PersistenceError>;

    /// 获取存储的字节大小（用于统计）
    fn size_bytes(&self) -> Result<u64, PersistenceError>;

    /// 加载归档记忆（冷存储）
    fn load_archived_memories(&self) -> Result<Vec<Memory>, PersistenceError>;

    /// 保存归档记忆（覆盖写入）
    fn save_archived_memories(&self, memories: &[Memory]) -> Result<(), PersistenceError>;

    /// 追加记忆到归档（合并已有归档）
    fn add_to_archive(&self, memories: &[Memory]) -> Result<(), PersistenceError>;

    /// 从归档中删除指定记忆
    fn delete_from_archive(&self, id: &str) -> Result<bool, PersistenceError>;
}

/// 创建默认的 JSON 文件持久化后端
///
/// 使用指定的数据目录。文件结构：
/// - `{data_dir}/memories.json` — 记忆存储
/// - `{data_dir}/chunks.json`    — 代码片段存储
pub fn create_json_persistence(data_dir: &str) -> Result<json::JsonPersistence, PersistenceError> {
    json::JsonPersistence::new(data_dir)
}