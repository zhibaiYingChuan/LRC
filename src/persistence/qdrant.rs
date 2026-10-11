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
// v0.9.7（GLOBAL_CODE_REVIEW_REPORT P1-6）：payload 反序列化路径的签名由
// 不可判别的 `Result<_, String>` 收敛为 [`crate::errors::LrcResult`]。
// 该函数当前无错误构造点（失败项静默跳过），故无 ErrorKind 归属。
use crate::errors::LrcResult;
use crate::memory_types::{Importance, Memory, MemoryType, PrivacyLevel};
use crate::persistence::{Persistence, PersistenceError};
use chrono::Utc;
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════
// v0.9.11（G5 / CR-18）：Qdrant 兜底存储的**落盘 WAL**
// ═══════════════════════════════════════════════════════════════════
//
// ## 修的是什么
// 修前 `fallback_memories` 是**纯进程内 `Mutex<Vec<Memory>>`，从不落盘**。
// 于是"Qdrant 不可用 / 集合为空"期间写入的记忆**只存在于内存**，重启即全部丢失 ——
// 而"兜底"的全部意义正是"主存储不可用时也不丢数据"。这是**功能承诺与实现不符**，
// 不是性能问题。
//
// ## 设计：追加式 WAL + 加载期重放/压实
// - 每次变更**追加一行**（O(1)），不重写整个文件（否则兜底会变成热路径瓶颈）
// - 加载时按顺序**重放**：`Memory`（同 id 后写者胜）/ `Delete`（墓碑）/ `Clear`
// - 重放后若行数 ≠ 结果数（有重复/墓碑/损坏行），则**原子压实**一次，使文件有界
//
// ## 为什么需要墓碑（`Delete`）
// 只追加 `Memory` 记录的话，**删除会丢失**：重启后重放会把已删记忆**复活**。
// 因此删除也必须进 WAL（墓碑），`Clear` 同理。
//
// 纯函数 [`replay_fallback_wal`] / [`compact_fallback_wal`] 与磁盘无关，
// 可在**默认 feature**（无需 Qdrant 服务）下直接单测。

/// 兜底 WAL 的默认文件名（可用 `LRC_QDRANT_FALLBACK_PATH` 覆盖）
pub const DEFAULT_FALLBACK_PATH: &str = "lrc_qdrant_fallback.jsonl";

/// 兜底 WAL 的一条记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FallbackRecord {
    /// 写入或更新一条记忆（同一 id **后写者胜**）
    Memory(Box<Memory>),
    /// 删除一条记忆（**墓碑**：缺它则重启后被删记忆会复活）
    Delete { id: String },
    /// 清空全部（其前的记录全部作废）
    Clear,
}

/// 纯函数：把 WAL 文本**重放**为记忆集合。
///
/// 语义（顺序敏感）：
/// - `Memory` ⇒ 插入或覆盖（同 id 后写者胜）
/// - `Delete` ⇒ 移除该 id
/// - `Clear`   ⇒ 清空此前累计的全部
/// - **损坏行 ⇒ 跳过**（与"锚点恢复失败告警而非整体作废"同策略：
///   不因单行损坏丢掉整个兜底库）
///
/// 返回顺序 = **首次出现的插入顺序**（不依赖 `HashMap` 迭代序，保证结果确定可比）。
pub fn replay_fallback_wal(text: &str) -> Vec<Memory> {
    let mut order: Vec<String> = Vec::new();
    let mut by_id: std::collections::HashMap<String, Memory> = std::collections::HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<FallbackRecord>(line) {
            Ok(FallbackRecord::Memory(m)) => {
                let m = *m;
                if !by_id.contains_key(&m.id) {
                    order.push(m.id.clone());
                }
                by_id.insert(m.id.clone(), m);
            }
            Ok(FallbackRecord::Delete { id }) => {
                by_id.remove(&id);
            }
            Ok(FallbackRecord::Clear) => {
                by_id.clear();
                order.clear();
            }
            Err(_) => {} // 损坏行：跳过
        }
    }

    order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect()
}

/// 纯函数：把记忆集合序列化为 WAL 文本（**压实**用：每条一行 `Memory` 记录）。
///
/// 压实后**不再含墓碑**，故重放结果与输入集合等价（见单测）。
pub fn compact_fallback_wal(memories: &[Memory]) -> String {
    let mut out = String::new();
    for m in memories {
        if let Ok(json) = serde_json::to_string(&FallbackRecord::Memory(Box::new(m.clone()))) {
            out.push_str(&json);
            out.push('\n');
        }
    }
    out
}

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
    /// 兜底记忆的**落盘 WAL 路径**（v0.9.11 / G5 / CR-18）。
    ///
    /// - `Some(path)`：兜底写入会**追加落盘**，启动时重放恢复（默认为
    ///   [`DEFAULT_FALLBACK_PATH`]）—— 这才是"兜底"应有的语义
    /// - `None`：退回旧行为（仅内存，重启即丢）；可用 `LRC_QDRANT_FALLBACK_PATH=""`
    ///   显式关闭
    pub fallback_path: Option<String>,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:6333".to_string(),
            collection: "lrc_memories".to_string(),
            vector_size: 9,
            timeout_secs: 10,
            auto_create: true,
            // v0.9.11（G5 / CR-18）：默认**开启**落盘兜底 —— 否则"兜底不落盘"
            // 这个缺陷会继续以默认配置存在。
            fallback_path: Some(DEFAULT_FALLBACK_PATH.to_string()),
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

        // v0.9.11（G5 / CR-18）：兜底 WAL 路径。
        // 显式设为**空字符串**即退回"仅内存"模式（旧行为）；未设置则用默认文件名。
        let fallback_path = match std::env::var("LRC_QDRANT_FALLBACK_PATH") {
            Ok(v) if v.trim().is_empty() => None,
            Ok(v) => Some(v),
            Err(_) => Some(DEFAULT_FALLBACK_PATH.to_string()),
        };

        Self {
            endpoint,
            collection,
            fallback_path,
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

#[derive(Debug, Deserialize)]
struct QdrantScoredPoint {
    id: serde_json::Value,
    /// 相似度分值（Qdrant 返回，用于排序参考）
    ///
    /// v0.9.7 核实：移除实验（`cargo check --features qdrant`）证明该 allow 非冗余
    /// （仍报 `field 'score' is never read`），予以保留。
    #[allow(dead_code)]
    #[serde(default)]
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
                PersistenceError::Io(std::io::Error::other(format!(
                    "创建 HTTP 客户端失败: {}",
                    e
                )))
            })?;

        let this = Self {
            config,
            client,
            fallback_memories: std::sync::Mutex::new(Vec::new()),
        };

        // v0.9.11（G5 / CR-18）：**启动即从落盘 WAL 恢复兜底库**。
        // 修前兜底只在内存 ⇒ 重启即丢，"兜底"名不副实。
        this.restore_fallback_from_wal();

        if this.config.auto_create {
            this.ensure_collection().await?;
        }

        Ok(this)
    }

    /// 兜底 WAL 的路径（`None` ⇒ 保持旧的内存-only 行为）
    fn fallback_file(&self) -> Option<&std::path::Path> {
        self.config
            .fallback_path
            .as_deref()
            .map(std::path::Path::new)
    }

    /// 从 WAL 恢复兜底记忆（v0.9.11 / G5 / CR-18）。
    ///
    /// 若 WAL 行数与重放结果数不等（存在重复行／墓碑／损坏行），则**原子压实**一次 ——
    /// 否则追加式 WAL 会无界增长（每次更新都追加一行）。
    fn restore_fallback_from_wal(&self) {
        let Some(path) = self.fallback_file() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return; // 文件不存在 = 首次运行
        };

        let restored = replay_fallback_wal(&text);
        let line_count = text.lines().filter(|l| !l.trim().is_empty()).count();
        if line_count != restored.len() {
            let compacted = compact_fallback_wal(&restored);
            if let Err(e) = crate::atomic_file::write_atomic(path, compacted.as_bytes()) {
                // 压实失败不影响数据可用性（内存中已恢复），故只告警
                eprintln!("[LRC·Qdrant·兜底] 压实 WAL 失败（数据仍可用）: {e}");
            }
        }
        if !restored.is_empty() {
            eprintln!(
                "[LRC·Qdrant·兜底] 从 WAL 恢复 {} 条记忆（{}）",
                restored.len(),
                path.display()
            );
        }
        if let Ok(mut guard) = self.fallback_memories.lock() {
            *guard = restored;
        }
    }

    /// 追加一条 WAL 记录（O(1)）。
    ///
    /// **失败只告警不返回错误**：兜底是主存储的旁路，写兜底失败不应使业务写入失败。
    fn append_fallback_wal(&self, record: &FallbackRecord) {
        let Some(path) = self.fallback_file() else {
            return;
        };
        let Ok(line) = serde_json::to_string(record) else {
            return;
        };
        use std::io::Write;
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            Ok(mut f) => {
                if let Err(e) = writeln!(f, "{line}") {
                    eprintln!("[LRC·Qdrant·兜底] 写 WAL 失败: {e}");
                }
            }
            Err(e) => eprintln!("[LRC·Qdrant·兜底] 打开 WAL 失败（{}）: {e}", path.display()),
        }
    }

    /// 用给定集合**原子重写** WAL（Qdrant 成功加载后对齐镜像时使用）
    fn rewrite_fallback_wal(&self, memories: &[Memory]) {
        let Some(path) = self.fallback_file() else {
            return;
        };
        let payload = compact_fallback_wal(memories);
        if let Err(e) = crate::atomic_file::write_atomic(path, payload.as_bytes()) {
            eprintln!("[LRC·Qdrant·兜底] 重写 WAL 失败: {e}");
        }
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
                eprintln!("[LRC·Qdrant] 滚动查询返回 HTTP {}: {}", status, body);
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
    ) -> LrcResult<Memory> {
        let id = match &point.id {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => point.id.to_string(),
        };

        // ═══ v0.9.11（G2）：**优先用完整 JSON 还原**（结构性修复 CR-17）═══
        //
        // 新写入的点都带 `memory_json`（整条记忆的序列化）。用它还原
        // ⇒ 时间戳/归属/版本/来源等**一律原样保留**，不可能再"每次重载记忆变新"。
        // 读不到 `memory_json` 时（v0.9.11 之前的旧点）才回退到下面的逐字段重建。
        if let Some(full) = payload.get("memory_json") {
            if let Ok(mut restored) = serde_json::from_value::<Memory>(full.clone()) {
                // 向量列不随 payload 走，保持既有约定「载入时不带向量，由 save 时重算」。
                restored.luoshu_vector = None;
                return Ok(restored);
            }
        }

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
            daoti_preview_gua: payload
                .get("daoti_preview_gua")
                .and_then(|v| v.as_str())
                .map(String::from),
            daoti_preview_bagua: payload
                .get("daoti_preview_bagua")
                .and_then(|v| v.as_str())
                .map(String::from),
            daoti_preview_version: payload
                .get("daoti_preview_version")
                .and_then(|v| v.as_str())
                .map(String::from),
            privacy_level,
            session_id: None,
            user_id: None,
            // 事件维度：缺失（旧数据）时退化为默认值，不影响加载
            event_id: payload
                .get("event_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            entities: payload
                .get("entities")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
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
        // v0.9.11（G5 / CR-18）：**同时追加落盘** —— 否则兜底只活在内存里，重启即丢
        self.append_fallback_wal(&FallbackRecord::Memory(Box::new(memory.clone())));

        // 如果有洛书向量，写入 Qdrant
        if let Some(lv) = memory.luoshu_vector.as_ref() {
            let handle = tokio::runtime::Handle::try_current().map_err(|_| {
                PersistenceError::Io(std::io::Error::other(
                    "QdrantPersistence 需要在 tokio 运行时上下文中使用",
                ))
            })?;

            let point = QdrantPoint {
                id: memory.id.clone(),
                vector: lv.to_vec(),
                payload: serde_json::json!({
                    // ═══ v0.9.11（G2）：**整条记忆的 JSON**（结构性修复 CR-17）═══
                    //
                    // 修复前这里**只存 10 个字段**，读回时把 `created_at`/`updated_at`/
                    // `last_accessed` **无条件重置为 `now()`**，并丢弃 `ttl_days`/`source`/
                    // `source_ids`/`confidence`/`information_gain`/`resolution`/`session_id`/
                    // `user_id`/`version`/`version_history` ⇒ 每次重载"记忆变新"、
                    // 衰减与事件顺序推理全部失真。
                    //
                    // 现在把**整条记忆**序列化进 payload，读回时用 serde 还原
                    // ⇒ 与 JSON 后端获得同一性质：**字段不可能因白名单不全而丢失**。
                    "memory_json": memory,
                    // ═══ 以下为 Qdrant 侧**索引/过滤**用的冗余字段（保持向后兼容）═══
                    // 它们与 memory_json 冗余是**有意**的：Qdrant 需要用它们做原生过滤，
                    // 而重建完整对象一律以 memory_json 为准。
                    "content": memory.content,
                    "memory_type": memory.memory_type.as_str(),
                    "importance": memory.importance.value(),
                    "project": memory.project,
                    "tags": memory.tags,
                    "bagua_index": memory.bagua_index,
                    "bagua_category": memory.bagua_category,
                    "topological_depth": memory.topological_depth,
                    "privacy_level": memory.privacy_level.as_str(),
                    // 事件维度：共同经历（event_id）与实体关联（entities）
                    // 缺此二者会导致重载后"同源/共享实体"关联全部丢失
                    "event_id": memory.event_id,
                    "entities": memory.entities,
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
            PersistenceError::Io(std::io::Error::other(
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
                // v0.9.11（G5 / CR-18）：镜像与 WAL **对齐**（一次原子重写），
                // 避免 WAL 保留与 Qdrant 不一致的陈旧内容
                self.rewrite_fallback_wal(&memories);
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
                eprintln!("[LRC·Qdrant] 从本地兜底加载 {} 条记忆", guard.len());
                return Ok(guard.clone());
            }
        }
        Ok(Vec::new())
    }

    fn delete_memory(&self, id: &str) -> Result<bool, PersistenceError> {
        // 先从 Qdrant 删除，成功后再更新本地兜底
        let found = self
            .fallback_memories
            .lock()
            .map(|guard| guard.iter().any(|m| m.id == id))
            .unwrap_or(false);

        // 从 Qdrant 删除
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            PersistenceError::Io(std::io::Error::other(
                "QdrantPersistence 需要在 tokio 运行时上下文中使用",
            ))
        })?;

        let url = format!(
            "{}/collections/{}/points/delete",
            self.config.endpoint, self.config.collection
        );

        let client = self.client.clone();
        let id_owned = id.to_string();
        let response = handle
            .block_on(async move {
                let body = serde_json::json!({ "points": [id_owned] });
                client.post(&url).json(&body).send().await
            })
            .map_err(|e| PersistenceError::Other(format!("Qdrant 删除请求失败: {e}")))?;
        if !response.status().is_success() {
            return Err(PersistenceError::Other(format!(
                "Qdrant 删除失败: HTTP {}",
                response.status().as_u16()
            )));
        }
        if let Ok(mut guard) = self.fallback_memories.lock() {
            guard.retain(|m| m.id != id);
        }
        // v0.9.11（G5 / CR-18）：删除必须写入**墓碑**。
        // 若只追加 Memory 记录，重启重放会把已删记忆**复活**。
        self.append_fallback_wal(&FallbackRecord::Delete { id: id.to_string() });
        Ok(found)
    }

    fn replace_all_memories(&self, _memories: &[Memory]) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Qdrant 后端无法保证全量替换的原子语义",
        )))
    }

    fn clear_memories(&self) -> Result<(), PersistenceError> {
        // v0.5.4 修复 C06：同步清除 Qdrant 远程向量数据
        // 此前仅清除本地兜底缓存，远程 Qdrant 数据未删除，导致数据不一致
        // 用户调用"清除记忆"后期望所有数据都被删除

        // 远程删除成功后再清除本地兜底缓存，失败时保留可恢复数据。
        // 清除 Qdrant 远程集合中的所有向量点
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            PersistenceError::Io(std::io::Error::other(
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
        if let Ok(mut guard) = self.fallback_memories.lock() {
            guard.clear();
        }
        // v0.9.11（G5 / CR-18）：清空同样必须进 WAL，否则重启后兜底会把已清空的记忆**带回来**
        self.append_fallback_wal(&FallbackRecord::Clear);

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

// ═══════════════════════════════════════════════════════════════════
// v0.9.11（G5 / CR-18）：兜底 WAL 的**纯函数单测**
// ═══════════════════════════════════════════════════════════════════
//
// 这些测试只依赖**与磁盘/网络无关的纯函数**（无需 Qdrant 服务即可运行）。
//
// ⚠️ 注意：整个 `qdrant` 模块受 `#[cfg(feature = "qdrant")]` 门控，故本模块需以
// `cargo test --features server,qdrant --lib` 运行。**这本身是一处门禁盲区**：
// 默认 CI 不覆盖该后端（见报告 G7 / CR-47），因此这里的验证依赖手动执行。
#[cfg(test)]
mod fallback_wal_tests {
    use super::*;

    fn mem(id: &str, content: &str) -> Memory {
        let mut m = Memory::new(
            content.to_string(),
            MemoryType::Fact,
            None,
            Vec::new(),
            Importance::new(5),
            None,
        );
        m.id = id.to_string();
        m
    }

    fn line(rec: &FallbackRecord) -> String {
        serde_json::to_string(rec).expect("WAL 记录应可序列化")
    }

    /// 同一 id **后写者胜**（更新语义）
    #[test]
    fn replay_last_write_wins() {
        let text = format!(
            "{}\n{}\n",
            line(&FallbackRecord::Memory(Box::new(mem("a", "旧")))),
            line(&FallbackRecord::Memory(Box::new(mem("a", "新")))),
        );
        let got = replay_fallback_wal(&text);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].content, "新", "同一 id 应以最后写入为准");
    }

    /// ★核心：**墓碑必须生效** —— 否则重启后已删记忆会**复活**
    #[test]
    fn replay_delete_tombstone_removes_memory() {
        let text = format!(
            "{}\n{}\n{}\n",
            line(&FallbackRecord::Memory(Box::new(mem("a", "甲")))),
            line(&FallbackRecord::Memory(Box::new(mem("b", "乙")))),
            line(&FallbackRecord::Delete {
                id: "a".to_string()
            }),
        );
        let got = replay_fallback_wal(&text);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "b", "被墓碑删除的 id 不得复活");
    }

    /// `Clear` 使其**前**的记录全部作废；其**后**的记录仍生效
    #[test]
    fn replay_clear_then_new_records() {
        let text = format!(
            "{}\n{}\n{}\n{}\n",
            line(&FallbackRecord::Memory(Box::new(mem("a", "甲")))),
            line(&FallbackRecord::Clear),
            line(&FallbackRecord::Memory(Box::new(mem("b", "乙")))),
            line(&FallbackRecord::Memory(Box::new(mem("c", "丙")))),
        );
        let ids: Vec<String> = replay_fallback_wal(&text)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec!["b".to_string(), "c".to_string()]);
    }

    /// 损坏行被跳过，且**不影响其余记录**（不因单行损坏丢掉整个兜底库）
    #[test]
    fn replay_skips_corrupted_lines() {
        let text = format!(
            "{}\n这不是合法 JSON\n{}\n",
            line(&FallbackRecord::Memory(Box::new(mem("a", "甲")))),
            line(&FallbackRecord::Memory(Box::new(mem("b", "乙")))),
        );
        let got = replay_fallback_wal(&text);
        assert_eq!(got.len(), 2, "单行损坏不应丢掉整库");
    }

    /// 压实是**无损**的：压实后行数 = 记录数，且重放结果与原集合等价
    #[test]
    fn compact_then_replay_is_equivalent() {
        let memories = vec![mem("a", "甲"), mem("b", "乙")];
        let compacted = compact_fallback_wal(&memories);
        assert_eq!(
            compacted.lines().count(),
            memories.len(),
            "压实后行数应为记录数（墓碑与重复行被消除）"
        );
        let mut ids: Vec<String> = replay_fallback_wal(&compacted)
            .into_iter()
            .map(|m| m.id)
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }

    /// 空文本 / 空集合 ⇒ 空结果（首次运行的路径，不得 panic）
    #[test]
    fn replay_empty_input() {
        assert!(replay_fallback_wal("").is_empty());
        assert!(replay_fallback_wal("\n\n  \n").is_empty());
        assert!(compact_fallback_wal(&[]).is_empty());
    }
}
