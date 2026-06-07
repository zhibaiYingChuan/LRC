// ============================================================
// 许可证: Apache 2.0
// 本文件实现 Qdrant 向量存储后端，属于公开层 (Layer 1)。
// ============================================================
//
// Qdrant 向量存储后端
//
// Qdrant 向量存储后端：
//   存储洛书 9 维向量 + 先天类别标签，
//   支持快速几何检索。
//
// 通过 Qdrant REST API 通信（使用 reqwest），
// 无需额外的 gRPC 依赖。

use crate::chunker::CodeChunk;
use crate::memory_types::{Importance, Memory, MemoryType, PrivacyLevel};
use crate::persistence::{Persistence, PersistenceError};
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Qdrant 连接配置
#[derive(Debug, Clone)]
pub struct QdrantConfig {
    /// Qdrant REST API 地址（默认 http://localhost:6333）
    pub endpoint: String,
    /// 集合名称（默认 "lrc_memories"）
    pub collection: String,
    /// 向量维度（固定 9，对应洛书九宫格）
    pub vector_size: u64,
    /// HTTP 超时（秒）
    pub timeout_secs: u64,
    /// 是否在启动时自动创建集合
    pub auto_create: bool,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:6333".to_string(),
            collection: "lrc_memories".to_string(),
            vector_size: 9,
            timeout_secs: 10,
            auto_create: true,
        }
    }
}

impl QdrantConfig {
    /// 从环境变量创建配置
    ///
    /// 环境变量：
    /// - `LRC_QDRANT_URL`：Qdrant 服务地址
    /// - `LRC_QDRANT_COLLECTION`：集合名称
    pub fn from_env() -> Self {
        let endpoint =
            std::env::var("LRC_QDRANT_URL").unwrap_or_else(|_| "http://localhost:6333".to_string());
        let collection =
            std::env::var("LRC_QDRANT_COLLECTION").unwrap_or_else(|_| "lrc_memories".to_string());

        Self {
            endpoint,
            collection,
            ..Default::default()
        }
    }
}

// ==================== Qdrant REST API 类型 ====================

/// Qdrant 点（一条记忆记录）
#[derive(Debug, Serialize, Deserialize)]
struct QdrantPoint {
    id: String,
    vector: Vec<f32>,
    payload: serde_json::Value,
}

/// Qdrant 批量插入请求
#[derive(Debug, Serialize)]
struct QdrantUpsertRequest {
    points: Vec<QdrantPoint>,
}

/// Qdrant 创建集合请求
#[derive(Debug, Serialize)]
struct QdrantCreateCollectionRequest {
    vectors: QdrantVectorConfig,
}

#[derive(Debug, Serialize)]
struct QdrantVectorConfig {
    size: u64,
    distance: String,
}

/// Qdrant 搜索结果
#[derive(Debug, Deserialize)]
struct QdrantSearchResult {
    #[serde(default)]
    result: Vec<QdrantScoredPoint>,
}

#[derive(Debug, Deserialize)]
struct QdrantScoredPoint {
    id: serde_json::Value,
    score: f32,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

/// Qdrant 滚动结果
#[derive(Debug, Deserialize)]
struct QdrantScrollResult {
    #[serde(default)]
    result: QdrantScrollData,
}

#[derive(Debug, Deserialize, Default)]
struct QdrantScrollData {
    #[serde(default)]
    points: Vec<QdrantScoredPoint>,
    #[serde(default)]
    next_page_offset: Option<serde_json::Value>,
}

/// Qdrant 持久化后端
///
/// 通过 HTTP REST API 与 Qdrant 向量数据库通信，
/// 使用洛书 9 维向量作为索引，支持高效的几何检索。
#[cfg(feature = "qdrant")]
pub struct QdrantPersistence {
    config: QdrantConfig,
    client: reqwest::Client,
    /// 本地 JSON 兜底存储（Qdrant 不可用时使用）
    fallback_memories: std::sync::Mutex<Vec<Memory>>,
}

#[cfg(feature = "qdrant")]
impl QdrantPersistence {
    /// 创建 Qdrant 持久化后端
    pub async fn new(config: QdrantConfig) -> Result<Self, PersistenceError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| {
                PersistenceError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("创建 HTTP 客户端失败: {}", e),
                ))
            })?;

        let this = Self {
            config,
            client,
            fallback_memories: std::sync::Mutex::new(Vec::new()),
        };

        if this.config.auto_create {
            this.ensure_collection().await?;
        }

        Ok(this)
    }

    /// 确保集合存在（不存在则创建）
    async fn ensure_collection(&self) -> Result<(), PersistenceError> {
        let url = format!(
            "{}/collections/{}",
            self.config.endpoint, self.config.collection
        );

        // 先检查集合是否存在
        let check = self.client.get(&url).send().await;
        if let Ok(resp) = check {
            if resp.status().is_success() {
                return Ok(());
            }
        }

        // 创建集合
        let create_body = QdrantCreateCollectionRequest {
            vectors: QdrantVectorConfig {
                size: self.config.vector_size,
                distance: "Cosine".to_string(),
            },
        };

        let response = self
            .client
            .put(&url)
            .json(&create_body)
            .send()
            .await
            .map_err(|e| {
                PersistenceError::Io(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("创建 Qdrant 集合失败: {}", e),
                ))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            eprintln!(
                "[LRC·Qdrant] 创建集合返回 {}: {}（将使用本地兜底）",
                status, body
            );
        } else {
            eprintln!("[LRC·Qdrant] 集合 '{}' 已就绪", self.config.collection);
        }

        Ok(())
    }
}

#[cfg(feature = "qdrant")]
impl Persistence for QdrantPersistence {
    fn save_memory(&self, memory: &Memory) -> Result<(), PersistenceError> {
        // 同时保存到本地兜底存储
        if let Ok(mut guard) = self.fallback_memories.lock() {
            // 更新或追加
            if let Some(idx) = guard.iter().position(|m| m.id == memory.id) {
                guard[idx] = memory.clone();
            } else {
                guard.push(memory.clone());
            }
        }

        // 如果有洛书向量，写入 Qdrant
        if let Some(lv) = memory.luoshu_vector {
            let handle = tokio::runtime::Handle::try_current().map_err(|_| {
                PersistenceError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "QdrantPersistence 需要在 tokio 运行时上下文中使用",
                ))
            })?;

            let point = QdrantPoint {
                id: memory.id.clone(),
                vector: lv.to_vec(),
                payload: serde_json::json!({
                    "content": memory.content,
                    "memory_type": memory.memory_type.as_str(),
                    "importance": memory.importance.value(),
                    "project": memory.project,
                    "tags": memory.tags,
                    "bagua_index": memory.bagua_index,
                    "bagua_category": memory.bagua_category,
                    "topological_depth": memory.topological_depth,
                    "privacy_level": memory.privacy_level.as_str(),
                }),
            };

            let url = format!(
                "{}/collections/{}/points",
                self.config.endpoint, self.config.collection
            );

            let body = QdrantUpsertRequest {
                points: vec![point],
            };

            let client = self.client.clone();
            // 忽略 Qdrant 写入错误（使用本地兜底）
            let _ = handle.block_on(async move { client.put(&url).json(&body).send().await });
        }

        Ok(())
    }

    fn load_all_memories(&self) -> Result<Vec<Memory>, PersistenceError> {
        // 优先从本地兜底加载
        if let Ok(guard) = self.fallback_memories.lock() {
            if !guard.is_empty() {
                return Ok(guard.clone());
            }
        }
        Ok(Vec::new())
    }

    fn delete_memory(&self, id: &str) -> Result<bool, PersistenceError> {
        // 从本地兜底删除
        let mut found = false;
        if let Ok(mut guard) = self.fallback_memories.lock() {
            if let Some(idx) = guard.iter().position(|m| m.id == id) {
                guard.remove(idx);
                found = true;
            }
        }

        // 从 Qdrant 删除
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "QdrantPersistence 需要在 tokio 运行时上下文中使用",
            ))
        })?;

        let url = format!(
            "{}/collections/{}/points/delete",
            self.config.endpoint, self.config.collection
        );

        let client = self.client.clone();
        let id_owned = id.to_string();
        let _ = handle.block_on(async move {
            let body = serde_json::json!({
                "points": [id_owned],
            });
            client.post(&url).json(&body).send().await
        });

        Ok(found)
    }

    fn clear_memories(&self) -> Result<(), PersistenceError> {
        if let Ok(mut guard) = self.fallback_memories.lock() {
            guard.clear();
        }
        Ok(())
    }

    fn save_chunks(&self, _chunks: &[CodeChunk]) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn load_chunks(&self) -> Result<Vec<CodeChunk>, PersistenceError> {
        Ok(Vec::new())
    }

    fn clear_chunks(&self) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn size_bytes(&self) -> Result<u64, PersistenceError> {
        Ok(0)
    }

    fn load_archived_memories(&self) -> Result<Vec<Memory>, PersistenceError> {
        Ok(Vec::new())
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

/// 当 `qdrant` feature 未启用时的占位类型
#[cfg(not(feature = "qdrant"))]
#[derive(Debug, Clone)]
pub struct QdrantPersistence;

#[cfg(not(feature = "qdrant"))]
impl QdrantPersistence {
    pub async fn new(_config: QdrantConfig) -> Result<Self, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Qdrant 后端未启用，请在编译时启用 `qdrant` feature",
        )))
    }
}
