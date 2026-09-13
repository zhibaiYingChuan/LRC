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
// v0.9.7（P0-2 依赖倒置修复）：原为 `crate::engine::memory_state_machine::MemoryState`——
// Layer 1 反向依赖 Layer 2。该类型已上提至 Layer 1 顶层（`crate::memory_state_machine`），
// 本处改为指向中立层，消除 Layer 1 → Layer 2 的依赖倒置。
use crate::memory_state_machine::MemoryState;
use crate::memory_types::Memory;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub mod json;

/// PostgreSQL 持久化后端（需 `postgres` feature）
#[cfg(feature = "postgres")]
pub mod postgres;

/// Qdrant 向量存储后端（需 `qdrant` feature）
#[cfg(feature = "qdrant")]
pub mod qdrant;

/// Neo4j 图存储后端（需 `neo4j` feature）
#[cfg(feature = "neo4j")]
pub mod neo4j;

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

impl Error for PersistenceError {
    // v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P1-6「错误处理不统一」）：
    //   此前为空的 `impl Error`，包裹的 io::Error / serde_json::Error 无法沿错误链
    //   回溯（`Error::source()` 恒为 None）。补齐后调用方可逐层打印根因。
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Serialization(e) => Some(e),
            Self::NotFound(_) | Self::Other(_) => None,
        }
    }
}

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

/// 关联召回的跨查询命中频次统计（P8.2o 状态化方案的持久化载体）。
///
/// 语义：`hit_counts[content]` 记录该内容在历史查询的根候选池中**出现过多少次**
/// （按"出现的查询数"计数，同一查询的池内不重复）；`total_queries` 为已累计的
/// 查询总数。二者共同支撑"留一命中率"：`l1_rate(c) = df(c) / max(total, 1)`。
///
/// 该口径与 P8.2o 离线探针逐字等价：探针写为
/// `(df_all − 1_{c ∈ q 池}) / (n_queries − 1)`，其中 `df_all = df + 1`、
/// `n_queries = total + 1`，两者恒等；而在线增量累计天然使当前查询不计入自身
/// 统计，从机制上排除了循环论证。
///
/// 以独立文件承载，`Memory` 无需新增字段；Postgres/Qdrant 后端通过 trait 默认
/// 方法自动忽略。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssocFrequency {
    /// 内容 → 跨查询文档频率（df，按出现的查询数计，池内不重复）
    pub hit_counts: BTreeMap<String, u32>,
    /// 已累计的查询总数
    pub total_queries: u64,
    /// 结构版本号，便于后续格式演进
    pub version: u32,
}

impl Default for AssocFrequency {
    fn default() -> Self {
        Self {
            hit_counts: BTreeMap::new(),
            total_queries: 0,
            version: 1,
        }
    }
}

impl AssocFrequency {
    /// 留一命中率 `l1_rate(c) = df(c) / max(total_queries, 1)`，值域 `[0, 1]`。
    ///
    /// 在线增量口径下当前查询尚未计入自身统计，故无需再减 1（与 P8.2o 探针
    /// 的 `(df_all − 1)/(n_queries − 1)` 恒等）。分母用 `max(…, 1)` 防除零。
    pub fn leave_one_rate(&self, content: &str) -> f32 {
        let df = self.hit_counts.get(content).copied().unwrap_or(0);
        if df == 0 {
            return 0.0;
        }
        let denom = self.total_queries.max(1) as f32;
        (df as f32 / denom).clamp(0.0, 1.0)
    }

    /// 累计一次查询的命中：对池内去重后的每个内容 `+1`，并将查询总数 `+1`。
    ///
    /// 去重语义与 P8.2o 探针的 df 构造一致（同一查询的池内不重复计数）。
    pub fn record_query<I, S>(&mut self, pool_contents: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        // 先物化为 owned 字符串：`seen` 需要跨迭代持有借用，若直接借用迭代器
        // 产出的临时值会触发 E0597（borrowed value does not live long enough）。
        let pool: Vec<String> = pool_contents
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .collect();
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for content in &pool {
            if !seen.insert(content.as_str()) {
                continue;
            }
            *self.hit_counts.entry(content.clone()).or_insert(0) += 1;
        }
        self.total_queries = self.total_queries.saturating_add(1);
    }
}

/// 持久化存储抽象 trait
///
/// 定义记忆和代码片段的 CRUD 操作接口。
/// 实现此 trait 即可接入不同的存储后端（JSON、SQLite、Redis 等）。
pub trait Persistence: Send + Sync {
    /// 保存一条记忆（新增或更新）
    fn save_memory(&self, memory: &Memory) -> Result<(), PersistenceError>;

    /// 加载 LRC 内置道体状态机的持久化快照。
    /// 未实现的后端返回空状态，保证旧后端兼容。
    fn load_memory_state(&self) -> Result<MemoryState, PersistenceError> {
        Ok(MemoryState::default())
    }

    /// 保存 LRC 内置道体状态机的持久化快照。
    /// 未实现的后端默认忽略，具体后端可提供原子持久化。
    fn save_memory_state(&self, _state: &MemoryState) -> Result<(), PersistenceError> {
        Ok(())
    }

    /// 加载跨查询命中频次统计（P8.2o 状态化方案）。
    /// 未实现的后端返回空统计，保证旧后端兼容。
    fn load_assoc_frequency(&self) -> Result<AssocFrequency, PersistenceError> {
        Ok(AssocFrequency::default())
    }

    /// 保存跨查询命中频次统计（P8.2o 状态化方案）。
    /// 未实现的后端默认忽略，具体后端可提供原子持久化。
    fn save_assoc_frequency(&self, _state: &AssocFrequency) -> Result<(), PersistenceError> {
        Ok(())
    }

    /// 批量更新记忆（仅更新指定的记忆，不触碰其他记忆）
    ///
    /// 默认实现：循环调用 `save_memory`（每条都会触发一次全量序列化+磁盘写入）。
    /// 推荐在具体后端中重写为单次序列化+单次磁盘写入，以获得 O(1) I/O 性能。
    ///
    /// 此方法用于 recall 后仅更新被检索到的记忆的 `last_accessed` 字段，
    /// 避免对 N 条记忆全量重写（原 O(N²) 序列化降为 O(N) 序列化）。
    fn update_memories(&self, memories: &[Memory]) -> Result<(), PersistenceError> {
        for m in memories {
            self.save_memory(m)?;
        }
        Ok(())
    }

    /// 批量保存记忆（新增或更新），单次序列化 + 单次磁盘写入
    ///
    /// 相比循环调用 `save_memory`（每条触发一次全量序列化+磁盘写入 O(n)），
    /// 此方法将 N 次 O(n) 磁盘 I/O 降为 1 次 O(n)，适用于批量合成/批量写入场景。
    ///
    /// 默认实现：循环调用 `save_memory`（向后兼容）。
    /// 推荐在具体后端重写为单次序列化+单次磁盘写入。
    fn save_memories(&self, memories: &[Memory]) -> Result<(), PersistenceError> {
        for m in memories {
            self.save_memory(m)?;
        }
        Ok(())
    }

    /// 全量替换所有记忆（原子语义）。
    ///
    /// 用于归档/修正等需要"仅保留指定集合"的场景——旧实现用
    /// `clear_memories` + 循环 `save_memory` 两端点，clear 成功后任一写入
    /// 失败/崩溃都会导致磁盘上的活跃记忆丢失（C05 全库丢失窗口）。
    ///
    /// 默认实现：明确拒绝全量替换，避免用非原子 clear+save 冒充原子语义。
    /// 后端必须自行实现事务/原子替换后才能支持此操作。
    fn replace_all_memories(&self, _memories: &[Memory]) -> Result<(), PersistenceError> {
        Err(PersistenceError::Other(
            "当前持久化后端不支持原子全量替换".to_string(),
        ))
    }

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

    /// 清空归档存储（默认实现：覆盖写入空列表）
    fn clear_archive(&self) -> Result<(), PersistenceError> {
        self.save_archived_memories(&[])
    }
}

/// 创建默认的 JSON 文件持久化后端
///
/// 使用指定的数据目录。文件结构：
/// - `{data_dir}/memories.json` — 记忆存储
/// - `{data_dir}/chunks.json`    — 代码片段存储
pub fn create_json_persistence(data_dir: &str) -> Result<json::JsonPersistence, PersistenceError> {
    json::JsonPersistence::new(data_dir)
}
