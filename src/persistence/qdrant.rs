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

/// Qdrant 滚动请求
#[derive(Debug, Serialize)]
struct QdrantScrollRequest {
    /// 每页返回的点数上限
    limit: u32,
    /// 是否返回向量数据
    #[serde(default)]
    with_vector: bool,
    /// 是否返回 payload
    #[serde(default)]
    with_payload: bool,
    /// 分页偏移量（用于下一页）
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<serde_json::Value>,
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

    /// 从 Qdrant 滚动查询所有点（分页获取）
    ///
    /// 使用 Qdrant Scroll API 逐页获取所有存储的记忆点，
    /// 然后将 payload 转换回 Memory 对象。
    /// 每页最多 100 条，支持分页遍历。
    async fn scroll_all_points(&self) -> Result<Vec<Memory>, PersistenceError> {
        let url = format!(
            "{}/collections/{}/points/scroll",
            self.config.endpoint, self.config.collection
        );

        let mut all_memories: Vec<Memory> = Vec::new();
        let mut next_offset: Option<serde_json::Value> = None;

        loop {
            let request_body = QdrantScrollRequest {
                limit: 100,
                with_vector: false,
                with_payload: true,
                offset: next_offset.take(),
            };

            let response = self
                .client
                .post(&url)
                .json(&request_body)
                .send()
                .await
                .map_err(|e| {
                    PersistenceError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        format!("Qdrant 滚动查询失败: {}", e),
                    ))
                })?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                eprintln!(
                    "[LRC·Qdrant] 滚动查询返回 HTTP {}: {}",
                    status, body
                );
                break;
            }

            let scroll_result: QdrantScrollResult = response.json().await.map_err(|e| {
                PersistenceError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("解析 Qdrant 滚动结果失败: {}", e),
                ))
            })?;

            let points = scroll_result.result.points;
            let has_more = scroll_result.result.next_page_offset.is_some();

            // 转换每个点为 Memory 对象
            for point in &points {
                if let Some(ref payload) = point.payload {
                    if let Ok(memory) = Self::payload_to_memory(point, payload) {
                        all_memories.push(memory);
                    }
                }
            }

            if !has_more || points.is_empty() {
                break;
            }

            next_offset = scroll_result.result.next_page_offset;
        }

        eprintln!(
            "[LRC·Qdrant] 从 Qdrant 加载了 {} 条记忆",
            all_memories.len()
        );
        Ok(all_memories)
    }

    /// 将 Qdrant 点的 payload 转换为 Memory 对象
    fn payload_to_memory(
        point: &QdrantScoredPoint,
        payload: &serde_json::Value,
    ) -> Result<Memory, String> {
        let id = match &point.id {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => point.id.to_string(),
        };

        let content = payload
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let memory_type = payload
            .get("memory_type")
            .and_then(|v| v.as_str())
            .and_then(MemoryType::try_parse)
            .unwrap_or(MemoryType::Fact);

        let importance = payload
            .get("importance")
            .and_then(|v| v.as_u64())
            .map(|v| Importance::new(v as u8))
            .unwrap_or(Importance::DEFAULT);

        let project = payload
            .get("project")
            .and_then(|v| v.as_str())
            .map(String::from);

        let tags = payload
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let privacy_level = payload
            .get("privacy_level")
            .and_then(|v| v.as_str())
            .and_then(PrivacyLevel::try_parse)
            .unwrap_or_default();

        let now = Utc::now();

        Ok(Memory {
            id,
            content,
            memory_type,
            project,
            tags,
            importance,
            ttl_days: None,
            created_at: now,
            updated_at: now,
            last_accessed: now,
            source: None,
            source_ids: Vec::new(),
            confidence: None,
            information_gain: None,
            resolution: "detailed".to_string(),
            luoshu_vector: None, // 不加载向量（save 时重新计算）
            bagua_index: payload
                .get("bagua_index")
                .and_then(|v| v.as_u64())
                .map(|n| n as u8),
            bagua_category: payload
                .get("bagua_category")
                .and_then(|v| v.as_str())
                .map(String::from),
            privacy_level,
            session_id: None,
            user_id: None,
            topological_depth: payload
                .get("topological_depth")
                .and_then(|v| v.as_f64())
                .map(|n| n as f32)
                .unwrap_or(0.5),
            version: 1,
            version_history: Vec::new(),
        })
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
            // v0.5.4 修复 C05：Qdrant 写入错误不再静默丢弃，改为传播错误
            // 用户需要知道数据是否真正写入成功，而非被"假成功"误导
            let response = handle
                .block_on(async move { client.put(&url).json(&body).send().await })
                .map_err(|e| PersistenceError::Other(format!("Qdrant 网络请求失败: {e}")))?;

            if !response.status().is_success() {
                return Err(PersistenceError::Other(format!(
                    "Qdrant 写入失败: HTTP {} {}",
                    response.status().as_u16(),
                    response.status().canonical_reason().unwrap_or("未知错误")
                )));
            }
        }

        Ok(())
    }

    fn load_all_memories(&self) -> Result<Vec<Memory>, PersistenceError> {
        // 从 Qdrant 滚动查询所有记忆（修复：重启后数据可恢复）
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "QdrantPersistence 需要在 tokio 运行时上下文中使用",
            ))
        })?;

        let qdrant_result = handle.block_on(async { self.scroll_all_points().await });

        match qdrant_result {
            Ok(memories) if !memories.is_empty() => {
                // Qdrant 数据加载成功，同步到本地兜底缓存
                if let Ok(mut guard) = self.fallback_memories.lock() {
                    *guard = memories.clone();
                }
                return Ok(memories);
            }
            Ok(_) => {
                // Qdrant 中没有数据，回退到本地兜底
                eprintln!("[LRC·Qdrant] Qdrant 中无数据，尝试本地兜底加载");
            }
            Err(e) => {
                // Qdrant 查询失败，回退到本地兜底
                eprintln!("[LRC·Qdrant] Qdrant 查询失败: {}，回退到本地兜底", e);
            }
        }

        // 本地兜底：从内存缓存加载
        if let Ok(guard) = self.fallback_memories.lock() {
            if !guard.is_empty() {
                eprintln!(
                    "[LRC·Qdrant] 从本地兜底加载 {} 条记忆",
                    guard.len()
                );
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
        // v0.5.4 修复 C06：同步清除 Qdrant 远程向量数据
        // 此前仅清除本地兜底缓存，远程 Qdrant 数据未删除，导致数据不一致
        // 用户调用"清除记忆"后期望所有数据都被删除
        
        // 1. 清除本地兜底缓存
        if let Ok(mut guard) = self.fallback_memories.lock() {
            guard.clear();
        }

        // 2. 清除 Qdrant 远程集合中的所有向量点
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
        let response = handle
            .block_on(async move {
                // 使用空 filter 匹配所有点，一次性删除全部
                let body = serde_json::json!({
                    "filter": {}
                });
                client.post(&url).json(&body).send().await
            })
            .map_err(|e| PersistenceError::Other(format!("Qdrant 清除远程数据失败: {e}")))?;

        if !response.status().is_success() {
            return Err(PersistenceError::Other(format!(
                "Qdrant 清除远程数据失败: HTTP {} {}",
                response.status().as_u16(),
                response.status().canonical_reason().unwrap_or("未知错误")
            )));
        }

        Ok(())
    }

    /// v0.5.4 修复：不再静默返回 Ok，Qdrant 后端不支持代码片段存储
    /// 调用方应使用 JSON 文件方案处理代码片段
    fn save_chunks(&self, _chunks: &[CodeChunk]) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Qdrant 后端暂不支持代码片段存储，请使用 JSON 文件方案",
        )))
    }

    /// v0.5.4 修复：不再静默返回空 Vec，Qdrant 后端不支持代码片段加载
    fn load_chunks(&self) -> Result<Vec<CodeChunk>, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Qdrant 后端暂不支持代码片段加载，请使用 JSON 文件方案",
        )))
    }

    /// v0.5.4 修复：不再静默返回 Ok，Qdrant 后端不支持代码片段清除
    fn clear_chunks(&self) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Qdrant 后端暂不支持代码片段清除，请使用 JSON 文件方案",
        )))
    }

    /// v0.5.4 修复：不再静默返回 0，Qdrant 后端暂不支持文件大小查询
    fn size_bytes(&self) -> Result<u64, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Qdrant 后端暂不支持文件大小查询，请使用 JSON 文件方案",
        )))
    }

    /// v0.5.4 修复：不再静默返回空 Vec，Qdrant 后端暂不支持归档记忆加载
    fn load_archived_memories(&self) -> Result<Vec<Memory>, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Qdrant 后端暂不支持归档记忆加载，请使用 JSON 文件方案",
        )))
    }

    /// v0.5.4 修复：不再静默返回 Ok，Qdrant 后端暂不支持归档记忆存储
    fn save_archived_memories(&self, _memories: &[Memory]) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Qdrant 后端暂不支持归档记忆存储，请使用 JSON 文件方案",
        )))
    }

    /// v0.5.4 修复：不再静默返回 Ok，Qdrant 后端暂不支持归档添加
    fn add_to_archive(&self, _memories: &[Memory]) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Qdrant 后端暂不支持归档添加，请使用 JSON 文件方案",
        )))
    }

    /// v0.5.4 修复：不再静默返回 false，Qdrant 后端暂不支持归档删除
    fn delete_from_archive(&self, _id: &str) -> Result<bool, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Qdrant 后端暂不支持归档删除，请使用 JSON 文件方案",
        )))
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
