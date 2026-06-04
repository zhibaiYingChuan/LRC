// ============================================================
// 许可证: Apache 2.0
// 本文件实现 REST v1 API 端点（Section 4.3），属于公开层 (Layer 1)。
// ============================================================
//
// Loong Recall v1 REST API
//
// REST v1 API 端点实现。提供以下端点：
//   POST /v1/encode               — 将文本转为洛书 9 维向量
//   POST /v1/memories/consolidate  — 接收表层记忆，触发结晶流程
//   POST /v1/memories/enrich       — 根据查询返回结构化长期记忆
//   POST /v1/memories/correct      — 用户手动修正一个已结晶的事实
//   POST /v1/memories/unfold       — 拆解合成记忆为子记忆（RecursiveUnfold）
//   GET  /v1/health/dao_metrics    — 返回道同构度仪表数据

#[cfg(feature = "ml")]
use crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder;
#[cfg(not(feature = "ml"))]
use crate::engine::luoshu_encoder::LuoShuEncoder as HybridLuoShuEncoder;
use crate::engine::mirror_trapezoid::mirror_project;
use crate::memory_store::{ListFilter, MemoryStore, RecallFilter};
use crate::memory_types::{Importance, Memory, MemoryType, PrivacyLevel};
use crate::persistence::json::JsonPersistence;
use crate::RecallResult;
use axum::{
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

// ==================== 请求/响应类型 ====================

/// /v1/encode 请求体
#[derive(Debug, Deserialize)]
pub struct EncodeRequest {
    pub text: String,
}

/// /v1/encode 响应体
#[derive(Debug, Serialize)]
pub struct EncodeResponse {
    pub luoshu_vector: [f32; 9],
    pub bagua_index: u8,
    pub bagua_category: String,
    pub center_value: f32,
    pub topological_depth: f32,
}

/// /v1/memories/consolidate 请求体
#[derive(Debug, Deserialize)]
pub struct ConsolidateRequest {
    pub memories: Vec<ConsolidateMemory>,
    #[serde(default = "default_synthesis_similarity")]
    pub synthesis_similarity: f32,
    #[serde(default = "default_min_cluster")]
    pub min_cluster: usize,
}

fn default_synthesis_similarity() -> f32 { 0.4 }
fn default_min_cluster() -> usize { 3 }

/// 结晶输入记忆
#[derive(Debug, Deserialize)]
pub struct ConsolidateMemory {
    pub content: String,
    #[serde(default = "default_memory_type")]
    pub memory_type: String,
    #[serde(default = "default_importance")]
    pub importance: u8,
    pub project: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_privacy")]
    pub privacy_level: String,
    pub session_id: Option<String>,
    pub user_id: Option<String>,
}

fn default_memory_type() -> String { "fact".into() }
fn default_importance() -> u8 { 5 }
fn default_privacy() -> String { "user".into() }

/// /v1/memories/consolidate 响应体
#[derive(Debug, Serialize)]
pub struct ConsolidateResponse {
    pub stored: usize,
    pub synthesized: usize,
    pub total_memories: usize,
    pub synthesis_summaries: Vec<String>,
}

/// /v1/memories/enrich 请求体
#[derive(Debug, Deserialize)]
pub struct EnrichRequest {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    pub session_id: Option<String>,
    pub user_id: Option<String>,
}

fn default_top_k() -> usize { 5 }

/// /v1/memories/enrich 响应体
#[derive(Debug, Serialize)]
pub struct EnrichResponse {
    pub memories: Vec<EnrichedMemory>,
    pub fast_path_hits: usize,
    pub deep_path_hits: usize,
    pub total: usize,
}

/// 增强记忆条目
#[derive(Debug, Serialize)]
pub struct EnrichedMemory {
    pub id: String,
    pub content: String,
    pub memory_type: String,
    pub score: f32,
    pub bagua_category: Option<String>,
    pub importance: u8,
    pub topological_depth: f32,
    pub version: u32,
    pub created_at: String,
}

/// /v1/memories/correct 请求体
#[derive(Debug, Deserialize)]
pub struct CorrectRequest {
    pub memory_id: String,
    pub content: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// /v1/memories/correct 响应体
#[derive(Debug, Serialize)]
pub struct CorrectResponse {
    pub success: bool,
    pub memory_id: String,
    pub new_version: u32,
    pub history_versions: usize,
}

/// /v1/health/dao_metrics 响应体
#[derive(Debug, Serialize)]
pub struct DaoMetricsResponse {
    pub dao_isomorphism_score: f32,
    pub bagua_entropy: f32,
    pub synthesis_ratio: f32,
    pub active_memories: usize,
    pub crystallized_memories: usize,
    pub archived_memories: usize,
    pub encodings_total: u64,
    pub compositions_total: u64,
    pub recalls_total: u64,
    pub corrections_total: u64,
    pub status: String,
}

/// /v1/memories/unfold 请求体（Section 3.2 RecursiveUnfold）
#[derive(Debug, Deserialize)]
pub struct UnfoldRequest {
    pub memory_id: String,
    #[serde(default = "default_min_activation")]
    pub min_activation: f32,
}

fn default_min_activation() -> f32 { 0.1 }

/// /v1/memories/unfold 响应体
#[derive(Debug, Serialize)]
pub struct UnfoldResponse {
    pub success: bool,
    pub source_memory_id: String,
    pub sub_vectors_count: usize,
    pub fidelity: f32,
    pub sub_memories: Vec<UnfoldedSubMemory>,
}

/// 拆解出的子记忆
#[derive(Debug, Serialize)]
pub struct UnfoldedSubMemory {
    pub id: String,
    pub content: String,
    pub bagua_category: String,
    pub weight: f32,
}

/// 共享状态类型别名（避免过长的类型签名）
pub type SharedStore = Arc<Mutex<MemoryStore<JsonPersistence>>>;

// ==================== 路由构建 ====================

/// 创建 v1 REST API 路由（状态类型为 ()，以便与主路由合并）
///
/// 通过闭包捕获 memory_store，无需使用 axum State。
pub fn build_v1_router(store: SharedStore) -> Router {
    let consolidate_store = store.clone();
    let enrich_store = store.clone();
    let correct_store = store.clone();
    let metrics_store = store.clone();
    let unfold_store = store.clone();

    // P0-1: 编码器创建一次，所有请求复用（避免每次请求都加载 ML 模型）
    let encode_encoder = std::sync::Arc::new(HybridLuoShuEncoder::default());

    Router::new()
        // POST /v1/encode — 将文本编码为洛书 9 维向量
        .route("/encode", post({
            let encoder = encode_encoder.clone();
            move |Json(req): Json<EncodeRequest>| {
                async move {
                    let luoshu_vec = encoder.encode_text(&req.text);
                let proj = mirror_project(&luoshu_vec);
                let center_val = luoshu_vec.center_value();
                let topological_depth: f32 = (1.0 - center_val).clamp(0.0, 1.0);

                Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(EncodeResponse {
                    luoshu_vector: luoshu_vec.values,
                    bagua_index: proj.best_index as u8,
                    bagua_category: proj.best_category.to_string(),
                    center_value: center_val,
                    topological_depth,
                }))
            }
        }
    }))
        // POST /v1/memories/consolidate — 接收表层记忆，触发结晶流程
        .route("/memories/consolidate", post({
            let store = consolidate_store;
            move |Json(req): Json<ConsolidateRequest>| {
                let store = store.clone();
                async move {
                    let mut store = store.lock().await;
                    let mut stored = 0usize;

                    for mem in &req.memories {
                        let memory_type = MemoryType::try_parse(&mem.memory_type)
                            .unwrap_or(MemoryType::Fact);
                        let privacy_level = PrivacyLevel::try_parse(&mem.privacy_level)
                            .unwrap_or_default();

                        let memory = Memory::new(
                            mem.content.clone(),
                            memory_type,
                            mem.project.clone(),
                            mem.tags.clone(),
                            Importance::new(mem.importance),
                            None,
                        )
                        .with_privacy(privacy_level, mem.session_id.clone(), mem.user_id.clone());

                        match store.remember(memory) {
                            Ok(_) => stored += 1,
                            Err(e) => eprintln!("[v1/consolidate] 写入失败: {}", e),
                        }
                    }

                    let synthesized = store.luoshu_synthesize().unwrap_or(0);

                    let filter = ListFilter::new();
                    let all_memories = store.list_memories(&filter).unwrap_or_default();
                    let synthesis_summaries: Vec<String> = all_memories.0
                        .iter()
                        .filter(|m| m.memory_type == MemoryType::Synthesis)
                        .map(|m| m.summary())
                        .collect();

                    let total = store.total_count().unwrap_or(0);

                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(ConsolidateResponse {
                        stored,
                        synthesized,
                        total_memories: total,
                        synthesis_summaries,
                    }))
                }
            }
        }))
        // POST /v1/memories/enrich — 双路检索增强
        .route("/memories/enrich", post({
            let store = enrich_store;
            move |Json(req): Json<EnrichRequest>| {
                let store = store.clone();
                async move {
                    let mut store = store.lock().await;
                    let privacy_ctx = (PrivacyLevel::User, req.session_id.clone(), req.user_id.clone());

                    let fast_filter = RecallFilter {
                        memory_type: None,
                        project: None,
                        tags: vec![],
                        min_importance: None,
                        top_k: req.top_k * 2,
                        privacy_context: Some(privacy_ctx.clone()),
                    };
                    let fast_result = store.recall(&req.query, &fast_filter).unwrap_or(RecallResult {
                        memories: vec![], scores: vec![], total: 0,
                    });

                    let deep_filter = RecallFilter {
                        memory_type: None,
                        project: None,
                        tags: vec![],
                        min_importance: None,
                        top_k: req.top_k * 2,
                        privacy_context: Some(privacy_ctx),
                    };
                    let deep_result = store.recall(&req.query, &deep_filter).unwrap_or(RecallResult {
                        memories: vec![], scores: vec![], total: 0,
                    });

                    // RRF 融合
                    let rrf_k: f32 = 60.0;
                    let mut fused: std::collections::HashMap<String, (f32, &Memory)> =
                        std::collections::HashMap::new();

                    for (rank, m) in fast_result.memories.iter().enumerate() {
                        let score = 1.0 / (rrf_k + (rank + 1) as f32);
                        fused.entry(m.id.clone()).or_insert((0.0, m)).0 += score;
                    }
                    for (rank, m) in deep_result.memories.iter().enumerate() {
                        let score = 1.0 / (rrf_k + (rank + 1) as f32);
                        fused.entry(m.id.clone()).or_insert((0.0, m)).0 += score;
                    }

                    let mut scored: Vec<(f32, &Memory)> = fused.values().cloned().collect();
                    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

                    let total = scored.len();
                    let memories: Vec<EnrichedMemory> = scored
                        .into_iter()
                        .take(req.top_k)
                        .map(|(score, m)| EnrichedMemory {
                            id: m.id.clone(),
                            content: m.content.clone(),
                            memory_type: m.memory_type.as_str().to_string(),
                            score,
                            bagua_category: m.bagua_category.clone(),
                            importance: m.importance.value(),
                            topological_depth: m.topological_depth,
                            version: m.version,
                            created_at: m.created_at.to_rfc3339(),
                        })
                        .collect();

                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(EnrichResponse {
                        memories,
                        fast_path_hits: fast_result.memories.len(),
                        deep_path_hits: deep_result.memories.len(),
                        total,
                    }))
                }
            }
        }))
        // POST /v1/memories/correct — 用户修正记忆
        .route("/memories/correct", post({
            let store = correct_store;
            move |Json(req): Json<CorrectRequest>| {
                let store = store.clone();
                async move {
                    let mut store = store.lock().await;
                    match store.correct_memory(&req.memory_id, &req.content, req.reason.as_deref()) {
                        Ok(Some(memory)) => {
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(CorrectResponse {
                                success: true,
                                memory_id: memory.id,
                                new_version: memory.version,
                                history_versions: memory.version_history.len(),
                            }))
                        }
                        Ok(None) => Err((
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({
                                "error": "memory_not_found",
                                "message": format!("未找到记忆: {}", req.memory_id)
                            })),
                        )),
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "correction_failed",
                                "message": format!("修正失败: {}", e)
                            })),
                        )),
                    }
                }
            }
        }))
        // POST /v1/memories/unfold — 拆解合成记忆
        .route("/memories/unfold", post({
            let store = unfold_store;
            move |Json(req): Json<UnfoldRequest>| {
                let store = store.clone();
                async move {
                    let mut store = store.lock().await;
                    match store.unfold_memory(&req.memory_id, req.min_activation) {
                        Ok(Some((sub_memories, fidelity))) => {
                            let sub_count = sub_memories.len();
                            let unfolded: Vec<UnfoldedSubMemory> = sub_memories
                                .into_iter()
                                .map(|m| UnfoldedSubMemory {
                                    id: m.id,
                                    content: m.content,
                                    bagua_category: m.bagua_category.unwrap_or_else(|| "未知".into()),
                                    weight: 1.0 / sub_count.max(1) as f32,
                                })
                                .collect();
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(UnfoldResponse {
                                success: true,
                                source_memory_id: req.memory_id,
                                sub_vectors_count: sub_count,
                                fidelity,
                                sub_memories: unfolded,
                            }))
                        }
                        Ok(None) => Err((
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({
                                "error": "unfold_failed",
                                "message": format!("无法拆解记忆: {} (可能不是合成类型或无洛书向量)", req.memory_id)
                            })),
                        )),
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "unfold_error",
                                "message": format!("拆解失败: {}", e)
                            })),
                        )),
                    }
                }
            }
        }))
        // GET /v1/health/dao_metrics — 道同构度仪表
        .route("/health/dao_metrics", get({
            let store = metrics_store;
            move || {
                let store = store.clone();
                async move {
                    let store = store.lock().await;
                    match store.dao_metrics_snapshot() {
                        Ok(snapshot) => {
                            let status = if snapshot.dao_isomorphism_score < 0.3 {
                                "critical"
                            } else if snapshot.dao_isomorphism_score < 0.5 {
                                "warning"
                            } else {
                                "healthy"
                            };
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(DaoMetricsResponse {
                                dao_isomorphism_score: snapshot.dao_isomorphism_score,
                                bagua_entropy: snapshot.bagua_entropy,
                                synthesis_ratio: snapshot.synthesis_ratio,
                                active_memories: snapshot.active_memories,
                                crystallized_memories: snapshot.crystallized_memories,
                                archived_memories: snapshot.archived_memories,
                                encodings_total: snapshot.encodings_total,
                                compositions_total: snapshot.compositions_total,
                                recalls_total: snapshot.recalls_total,
                                corrections_total: snapshot.corrections_total,
                                status: status.to_string(),
                            }))
                        }
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "metrics_failed",
                                "message": format!("道同构度采集失败: {}", e)
                            })),
                        )),
                    }
                }
            }
        }))
}