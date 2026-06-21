// ============================================================
// 许可证: Apache 2.0
// 本文件实现 PostgreSQL 持久化后端，属于公开层 (Layer 1)。
// ============================================================
//
// PostgreSQL 持久化后端
//
// PostgreSQL 持久化后端：
//   存储原始对话日志、时间戳、会话 ID 等，
//   支持审计、回溯原始证据。
//
// 使用 `sqlx` 进行异步 PostgreSQL 连接。
// 需要 `postgres` feature flag + 运行中的 PostgreSQL 服务。

use crate::chunker::CodeChunk;
use crate::memory_types::{Importance, Memory, MemoryType, PrivacyLevel};
use crate::persistence::{Persistence, PersistenceError};
use chrono::{DateTime, Utc};

/// PostgreSQL 持久化配置
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    /// 数据库连接 URL
    /// 格式: postgres://user:password@host:port/database
    pub database_url: String,
    /// 连接池最大连接数
    pub max_connections: u32,
    /// 表名前缀（默认 "lrc_"）
    pub table_prefix: String,
    /// 是否在启动时自动创建表
    pub auto_migrate: bool,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            database_url: String::new(),
            max_connections: 5,
            table_prefix: "lrc_".to_string(),
            auto_migrate: true,
        }
    }
}

impl PostgresConfig {
    /// 从环境变量创建配置
    ///
    /// 环境变量：
    /// - `DATABASE_URL` 或 `LRC_PG_URL`：数据库连接 URL
    pub fn from_env() -> Result<Self, String> {
        let database_url = std::env::var("LRC_PG_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .map_err(|_| {
                "未设置数据库 URL，请设置 LRC_PG_URL 或 DATABASE_URL 环境变量".to_string()
            })?;
        Ok(Self {
            database_url,
            ..Default::default()
        })
    }
}

/// PostgreSQL 持久化后端
///
/// 实现 `Persistence` trait，使用 PostgreSQL 存储记忆和代码片段。
///
/// 表结构：
/// ```sql
/// CREATE TABLE lrc_memories (
///     id TEXT PRIMARY KEY,
///     content TEXT NOT NULL,
///     memory_type TEXT NOT NULL,
///     project TEXT,
///     tags JSONB DEFAULT '[]',
///     importance INTEGER DEFAULT 5,
///     version INTEGER DEFAULT 1,
///     created_at TIMESTAMPTZ NOT NULL,
///     last_accessed TIMESTAMPTZ NOT NULL,
///     ttl_days INTEGER,
///     expires_at TIMESTAMPTZ,
///     luoshu_vector JSONB,
///     bagua_index SMALLINT,
///     bagua_category TEXT,
///     topological_depth REAL DEFAULT 0.5,
///     privacy_level TEXT DEFAULT 'user',
///     session_id TEXT,
///     user_id TEXT,
///     source TEXT,
///     source_ids JSONB DEFAULT '[]',
///     correction_history JSONB DEFAULT '[]',
///     metadata JSONB DEFAULT '{}'
/// );
/// ```
#[cfg(feature = "postgres")]
pub struct PostgresPersistence {
    pool: sqlx::PgPool,
    table_prefix: String,
}

#[cfg(feature = "postgres")]
impl PostgresPersistence {
    /// 创建新的 PostgreSQL 持久化后端
    pub async fn new(config: PostgresConfig) -> Result<Self, PersistenceError> {
        // v0.5.4 修复：table_prefix 白名单校验，防止 SQL 注入
        // 仅允许字母、数字和下划线，长度 1-30 字符
        if config.table_prefix.is_empty() || config.table_prefix.len() > 30 {
            return Err(PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "table_prefix 长度必须在 1-30 之间，当前值: '{}'",
                    config.table_prefix
                ),
            )));
        }
        if !config
            .table_prefix
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_')
        {
            return Err(PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "table_prefix 仅允许字母、数字和下划线，当前值: '{}'",
                    config.table_prefix
                ),
            )));
        }

        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(config.max_connections)
            .connect(&config.database_url)
            .await
            .map_err(|e| {
                PersistenceError::Io(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("PostgreSQL 连接失败: {}", e),
                ))
            })?;

        let this = Self {
            pool,
            table_prefix: config.table_prefix,
        };

        if config.auto_migrate {
            this.migrate().await?;
        }

        Ok(this)
    }

    /// 自动创建表（幂等操作）
    async fn migrate(&self) -> Result<(), PersistenceError> {
        let memories_table = format!(
            "CREATE TABLE IF NOT EXISTS {}memories (\
                id TEXT PRIMARY KEY, \
                content TEXT NOT NULL, \
                memory_type TEXT NOT NULL DEFAULT 'fact', \
                project TEXT, \
                tags JSONB DEFAULT '[]', \
                importance SMALLINT DEFAULT 5, \
                version INTEGER DEFAULT 1, \
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), \
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), \
                last_accessed TIMESTAMPTZ NOT NULL DEFAULT NOW(), \
                ttl_days INTEGER, \
                luoshu_vector JSONB, \
                bagua_index SMALLINT, \
                bagua_category TEXT, \
                topological_depth REAL DEFAULT 0.5, \
                privacy_level TEXT DEFAULT 'user', \
                session_id TEXT, \
                user_id TEXT, \
                source TEXT, \
                source_ids JSONB DEFAULT '{{}}', \
                confidence REAL, \
                version_history JSONB DEFAULT '{{}}'\
            )",
            self.table_prefix
        );

        let chunks_table = format!(
            "CREATE TABLE IF NOT EXISTS {}chunks (\
                id TEXT PRIMARY KEY, \
                name TEXT NOT NULL, \
                content TEXT NOT NULL, \
                language TEXT NOT NULL, \
                file_path TEXT NOT NULL, \
                start_line INTEGER NOT NULL, \
                end_line INTEGER NOT NULL, \
                chunk_type TEXT NOT NULL\
            )",
            self.table_prefix
        );

        sqlx::query(&memories_table)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                PersistenceError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("创建 memories 表失败: {}", e),
                ))
            })?;

        sqlx::query(&chunks_table)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                PersistenceError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("创建 chunks 表失败: {}", e),
                ))
            })?;

        Ok(())
    }

    /// 将 Memory 序列化为数据库行
    fn memory_to_row(memory: &Memory) -> serde_json::Value {
        serde_json::json!({
            "id": memory.id,
            "content": memory.content,
            "memory_type": memory.memory_type.as_str(),
            "project": memory.project,
            "tags": memory.tags,
            "importance": memory.importance.value(),
            "version": memory.version,
            "created_at": memory.created_at.to_rfc3339(),
            "updated_at": memory.updated_at.to_rfc3339(),
            "last_accessed": memory.last_accessed.to_rfc3339(),
            "ttl_days": memory.ttl_days,
            "luoshu_vector": memory.luoshu_vector,
            "bagua_index": memory.bagua_index,
            "bagua_category": memory.bagua_category,
            "topological_depth": memory.topological_depth,
            "privacy_level": memory.privacy_level.as_str(),
            "session_id": memory.session_id,
            "user_id": memory.user_id,
            "source": memory.source,
            "source_ids": memory.source_ids,
            "confidence": memory.confidence,
            "version_history": memory.version_history.iter().map(|v| {
                serde_json::json!({
                    "version": v.version,
                    "content": v.content,
                    "updated_at": v.updated_at.to_rfc3339(),
                    "reason": v.reason,
                })
            }).collect::<Vec<_>>(),
        })
    }

    /// 从数据库行反序列化为 Memory
    fn row_to_memory(row: &sqlx::postgres::PgRow) -> Result<Memory, PersistenceError> {
        use sqlx::Row;

        let id: String = row.try_get("id").unwrap_or_default();
        let content: String = row.try_get("content").unwrap_or_default();
        let memory_type_str: String = row.try_get("memory_type").unwrap_or_else(|_| "fact".into());
        let memory_type = MemoryType::try_parse(&memory_type_str).unwrap_or(MemoryType::Fact);
        let project: Option<String> = row.try_get("project").ok();
        let tags_json: Option<serde_json::Value> = row.try_get("tags").ok();
        let tags: Vec<String> = tags_json
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let importance_val: Option<i16> = row.try_get("importance").ok();
        let importance = Importance::new(importance_val.unwrap_or(5) as u8);
        let version: Option<i32> = row.try_get("version").ok();
        let created_at: Option<DateTime<Utc>> = row.try_get("created_at").ok();
        let updated_at: Option<DateTime<Utc>> = row.try_get("updated_at").ok();
        let last_accessed: Option<DateTime<Utc>> = row.try_get("last_accessed").ok();
        let ttl_days: Option<i32> = row.try_get("ttl_days").ok();
        let luoshu_vector: Option<serde_json::Value> = row.try_get("luoshu_vector").ok();
        let luoshu_vec: Option<[f32; 9]> =
            luoshu_vector.and_then(|v| serde_json::from_value(v).ok());
        let bagua_index: Option<i16> = row.try_get("bagua_index").ok();
        let bagua_category: Option<String> = row.try_get("bagua_category").ok();
        let topological_depth: Option<f32> = row.try_get("topological_depth").ok();
        let privacy_str: Option<String> = row.try_get("privacy_level").ok();
        let privacy_level = privacy_str
            .as_deref()
            .and_then(PrivacyLevel::try_parse)
            .unwrap_or_default();
        let session_id: Option<String> = row.try_get("session_id").ok();
        let user_id: Option<String> = row.try_get("user_id").ok();
        let source: Option<String> = row.try_get("source").ok();
        let source_ids_json: Option<serde_json::Value> = row.try_get("source_ids").ok();
        let source_ids: Vec<String> = source_ids_json
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let confidence: Option<f32> = row.try_get("confidence").ok();
        let version_history_json: Option<serde_json::Value> = row.try_get("version_history").ok();
        let version_history: Vec<crate::memory_types::MemoryVersion> = version_history_json
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let mut memory = Memory::new(
            content,
            memory_type,
            project,
            tags,
            importance,
            ttl_days.map(|d| d as u32),
        );
        memory.id = id;
        memory.version = version.unwrap_or(1) as u32;
        memory.created_at = created_at.unwrap_or_else(Utc::now);
        memory.updated_at = updated_at.unwrap_or_else(Utc::now);
        memory.last_accessed = last_accessed.unwrap_or_else(Utc::now);
        memory.luoshu_vector = luoshu_vec;
        memory.bagua_index = bagua_index.map(|i| i as u8);
        memory.bagua_category = bagua_category;
        memory.topological_depth = topological_depth.unwrap_or(0.5);
        memory.privacy_level = privacy_level;
        memory.session_id = session_id;
        memory.user_id = user_id;
        memory.source = source;
        memory.source_ids = source_ids;
        memory.confidence = confidence;
        memory.version_history = version_history;

        Ok(memory)
    }
}

#[cfg(feature = "postgres")]
impl PostgresPersistence {
    /// 在当前 tokio 运行时中执行异步操作
    ///
    /// PostgreSQL 后端需要异步执行，但 `Persistence` trait 是同步接口。
    /// 此方法桥接同步与异步：获取当前 tokio 运行时句柄并阻塞执行 future。
    ///
    /// v0.5.4 修复：使用 `block_in_place` 包裹 `block_on`，
    /// 通知 tokio 运行时当前线程即将阻塞，允许运行时将其他任务迁移到备用线程，
    /// 避免阻塞整个异步运行时。
    ///
    /// # Errors
    /// 如果不在 tokio 运行时上下文中调用，返回错误。
    fn block_on_async<F, T>(&self, future: F) -> Result<T, PersistenceError>
    where
        F: std::future::Future<Output = Result<T, PersistenceError>> + Send,
        T: Send,
    {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "PostgresPersistence 需要在 tokio 运行时上下文中使用。\
                 请确保在 #[tokio::main] 或 tokio::runtime::Runtime 中调用。",
            ))
        })?;
        // v0.5.4 修复：block_in_place 通知 tokio 当前线程将阻塞
        tokio::task::block_in_place(|| handle.block_on(future))
    }
}

#[cfg(feature = "postgres")]
impl Persistence for PostgresPersistence {
    fn save_memory(&self, memory: &Memory) -> Result<(), PersistenceError> {
        let pool = self.pool.clone();
        let row = Self::memory_to_row(memory);
        let table = format!("{}memories", self.table_prefix);

        self.block_on_async(async move {
            sqlx::query(&format!(
                "INSERT INTO {} (id, content, memory_type, project, tags, importance, version, \
                 created_at, updated_at, last_accessed, ttl_days, luoshu_vector, bagua_index, \
                 bagua_category, topological_depth, privacy_level, session_id, user_id, \
                 source, source_ids, confidence, version_history) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22) \
                 ON CONFLICT (id) DO UPDATE SET \
                 content = EXCLUDED.content, \
                 memory_type = EXCLUDED.memory_type, \
                 importance = EXCLUDED.importance, \
                 version = EXCLUDED.version, \
                 updated_at = EXCLUDED.updated_at, \
                 last_accessed = EXCLUDED.last_accessed, \
                 luoshu_vector = EXCLUDED.luoshu_vector, \
                 topological_depth = EXCLUDED.topological_depth",
                table
            ))
            .bind(row["id"].as_str().unwrap_or(""))
            .bind(row["content"].as_str().unwrap_or(""))
            .bind(row["memory_type"].as_str().unwrap_or("fact"))
            .bind(row["project"].as_str())
            .bind(serde_json::to_string(&row["tags"]).unwrap_or_default())
            .bind(row["importance"].as_i64().unwrap_or(5) as i16)
            .bind(row["version"].as_i64().unwrap_or(1) as i32)
            .bind(row["created_at"].as_str().map(|s| s.to_string()))
            .bind(row["updated_at"].as_str().map(|s| s.to_string()))
            .bind(row["last_accessed"].as_str().map(|s| s.to_string()))
            .bind(row["ttl_days"].as_i64().map(|v| v as i32))
            .bind(serde_json::to_string(&row["luoshu_vector"]).unwrap_or_default())
            .bind(row["bagua_index"].as_i64().map(|v| v as i16))
            .bind(row["bagua_category"].as_str().map(|s| s.to_string()))
            .bind(row["topological_depth"].as_f64().unwrap_or(0.5) as f32)
            .bind(row["privacy_level"].as_str().unwrap_or("user"))
            .bind(row["session_id"].as_str().map(|s| s.to_string()))
            .bind(row["user_id"].as_str().map(|s| s.to_string()))
            .bind(row["source"].as_str().map(|s| s.to_string()))
            .bind(serde_json::to_string(&row["source_ids"]).unwrap_or_default())
            .bind(row["confidence"].as_f64().map(|v| v as f32))
            .bind(serde_json::to_string(&row["version_history"]).unwrap_or_default())
            .execute(&pool)
            .await
            .map_err(|e| PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("保存记忆失败: {}", e),
            )))?;
            Ok(())
        })
    }

    fn load_all_memories(&self) -> Result<Vec<Memory>, PersistenceError> {
        let pool = self.pool.clone();
        let table = format!("{}memories", self.table_prefix);

        self.block_on_async(async move {
            let rows = sqlx::query(&format!("SELECT * FROM {}", table))
                .fetch_all(&pool)
                .await
                .map_err(|e| {
                    PersistenceError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("加载记忆失败: {}", e),
                    ))
                })?;

            let mut memories = Vec::with_capacity(rows.len());
            for row in &rows {
                match Self::row_to_memory(row) {
                    Ok(m) => memories.push(m),
                    Err(e) => {
                        eprintln!("[LRC·PG] 反序列化记忆失败: {}", e);
                    }
                }
            }
            Ok(memories)
        })
    }

    fn delete_memory(&self, id: &str) -> Result<bool, PersistenceError> {
        let pool = self.pool.clone();
        let id_owned = id.to_string();
        let table = format!("{}memories", self.table_prefix);

        self.block_on_async(async move {
            let result = sqlx::query(&format!("DELETE FROM {} WHERE id = $1", table))
                .bind(&id_owned)
                .execute(&pool)
                .await
                .map_err(|e| {
                    PersistenceError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("删除记忆失败: {}", e),
                    ))
                })?;
            Ok(result.rows_affected() > 0)
        })
    }

    fn clear_memories(&self) -> Result<(), PersistenceError> {
        let pool = self.pool.clone();
        let table = format!("{}memories", self.table_prefix);

        self.block_on_async(async move {
            sqlx::query(&format!("DELETE FROM {}", table))
                .execute(&pool)
                .await
                .map_err(|e| {
                    PersistenceError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("清空记忆失败: {}", e),
                    ))
                })?;
            Ok(())
        })
    }

    fn save_chunks(&self, _chunks: &[CodeChunk]) -> Result<(), PersistenceError> {
        // v0.5.4 修复：不再静默返回 Ok，明确告知调用方操作不支持
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "PostgreSQL 后端暂不支持代码片段存储，请使用 JSON 文件方案",
        )))
    }

    fn load_chunks(&self) -> Result<Vec<CodeChunk>, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "PostgreSQL 后端暂不支持代码片段加载，请使用 JSON 文件方案",
        )))
    }

    fn clear_chunks(&self) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "PostgreSQL 后端暂不支持代码片段清除，请使用 JSON 文件方案",
        )))
    }

    fn size_bytes(&self) -> Result<u64, PersistenceError> {
        Ok(0) // PG 存储大小需要单独查询
    }

    fn load_archived_memories(&self) -> Result<Vec<Memory>, PersistenceError> {
        Ok(Vec::new()) // 归档通过 expires_at 字段在查询中过滤
    }

    fn save_archived_memories(&self, _memories: &[Memory]) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn add_to_archive(&self, _memories: &[Memory]) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn delete_from_archive(&self, _id: &str) -> Result<bool, PersistenceError> {
        Ok(false)
    }
}

// === 无 postgres feature 时的占位实现 ===
// 确保在未启用 postgres feature 时类型仍然可见（用于 MultiPersistence 枚举）

/// 当 `postgres` feature 未启用时的占位类型
#[cfg(not(feature = "postgres"))]
#[derive(Debug, Clone)]
pub struct PostgresPersistence;

#[cfg(not(feature = "postgres"))]
impl PostgresPersistence {
    /// 占位构造（永远返回错误）
    pub async fn new(_config: PostgresConfig) -> Result<Self, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "PostgreSQL 后端未启用，请在编译时启用 `postgres` feature",
        )))
    }
}

#[cfg(test)]
#[cfg(feature = "postgres")]
mod tests {
    use super::*;

    /// 测试：Memory ↔ 数据库行 序列化往返
    /// 注意：此测试需要运行中的 PostgreSQL，仅在 CI 中启用
    #[tokio::test]
    #[ignore = "需要 PostgreSQL 服务"]
    async fn test_postgres_roundtrip() {
        let config = PostgresConfig {
            database_url: std::env::var("TEST_PG_URL")
                .unwrap_or_else(|_| "postgres://localhost:5432/lrc_test".into()),
            max_connections: 1,
            table_prefix: "test_".to_string(),
            auto_migrate: true,
        };

        let pg = PostgresPersistence::new(config).await.expect("连接应成功");

        let memory = Memory::new(
            "测试记忆".to_string(),
            MemoryType::Fact,
            Some("test_project".into()),
            vec!["test".into()],
            Importance::new(7),
            None,
        );

        pg.save_memory(&memory).expect("保存应成功");

        let all = pg.load_all_memories().expect("加载应成功");
        assert!(!all.is_empty());
        assert!(all.iter().any(|m| m.content.contains("测试记忆")));

        // 清理
        pg.clear_memories().expect("清理应成功");
    }
}
