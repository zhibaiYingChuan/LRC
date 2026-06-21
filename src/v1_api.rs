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
//   GET  /v1/health/system         — 系统健康报告（可解释性面板）
//   GET  /v1/health/detailed       — 详细系统健康报告（运维级，含 GC / 反馈 / 调节器耦合信息）
//   POST /v1/feedback              — 用户反馈回路（标记检索/合成质量，恢复隔离记忆）
//   GET  /v1/audit-trail            — 审计追踪（查询系统自主行为日志，质疑五）
//   GET  /v1/code/search            — 代码库搜索（查询参数: query, top_k, keywords）

use crate::config::DEFAULT_PORT;
use crate::engine::audit_trail::{AuditEventType, AuditQuery};
#[cfg(not(feature = "ml"))]
use crate::engine::luoshu_encoder::LuoShuEncoder as HybridLuoShuEncoder;
#[cfg(feature = "ml")]
use crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder;
use crate::engine::mirror_trapezoid::mirror_project;
use crate::engine::user_feedback::{FeedbackTarget, FeedbackType};
use crate::memory_store::{ListFilter, MemoryStore, RecallFilter};
use crate::memory_types::{Importance, Memory, MemoryType, PrivacyLevel};
use crate::persistence::json::JsonPersistence;
use crate::server::IndexedCodebase;
use crate::RecallResult;
use axum::{
    extract::Query,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;

/// 基准测试报告缓存（避免每次请求都重新运行耗时的基准测试）
static BENCHMARK_CACHE: std::sync::LazyLock<StdMutex<Option<serde_json::Value>>> =
    std::sync::LazyLock::new(|| StdMutex::new(None));

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

fn default_synthesis_similarity() -> f32 {
    0.4
}
fn default_min_cluster() -> usize {
    3
}

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

fn default_memory_type() -> String {
    "fact".into()
}
fn default_importance() -> u8 {
    5
}
fn default_privacy() -> String {
    "user".into()
}

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

fn default_top_k() -> usize {
    5
}

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

fn default_min_activation() -> f32 {
    0.1
}

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

/// v0.5.4 P1-7 新增：/v1/memories/recent 查询参数
///
/// 用于控制最近记忆端点的返回数量。
#[derive(Debug, Clone, Deserialize)]
pub struct RecentMemoriesParams {
    /// 返回的记忆数量（默认 5，最大 20）
    pub limit: Option<usize>,
}

// ==================== 路由构建 ====================

/// 创建 v1 REST API 路由（状态类型为 ()，以便与主路由合并）
///
/// 通过闭包捕获 memory_store 和 codebase_manager，无需使用 axum State。
pub fn build_v1_router(
    store: SharedStore,
    codebase_manager: Arc<Mutex<Box<dyn IndexedCodebase>>>,
) -> Router {
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

                    let synthesized = store.luoshu_synthesize().unwrap_or_else(|e| {
                        eprintln!("[v1/consolidate] 合成失败: {}", e);
                        0
                    });

                    let filter = ListFilter::new();
                    let all_memories = store.list_memories(&filter).unwrap_or_else(|e| {
                        eprintln!("[v1/consolidate] 列出记忆失败: {}", e);
                        Default::default()
                    });
                    let synthesis_summaries: Vec<String> = all_memories.0
                        .iter()
                        .filter(|m| m.memory_type == MemoryType::Synthesis)
                        .map(|m| m.summary())
                        .collect();

                    let total = store.total_count().unwrap_or_else(|e| {
                        eprintln!("[v1/consolidate] 获取总数失败: {}", e);
                        0
                    });

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
                    // v0.5.4 桌面端测试修复：当请求未携带 user_id 时，不设置隐私上下文
                    // 本地单用户应用场景下，仪表盘检索不应因缺少 user_id 而过滤掉 User 级记忆
                    // is_visible() 在 privacy_context 为 None 时返回 true（全部可见）
                    let privacy_ctx = if req.user_id.is_some() {
                        Some((PrivacyLevel::User, req.session_id.clone(), req.user_id.clone()))
                    } else {
                        None
                    };

                    let fast_filter = RecallFilter {
                        memory_type: None,
                        project: None,
                        tags: vec![],
                        min_importance: None,
                        top_k: req.top_k * 2,
                        privacy_context: privacy_ctx.clone(),
                    };
                    let fast_result = store.recall(&req.query, &fast_filter).unwrap_or_else(|e| {
                        eprintln!("[v1/enrich] 快速路径检索失败: {}", e);
                        RecallResult { memories: vec![], scores: vec![], total: 0 }
                    });

                    let deep_filter = RecallFilter {
                        memory_type: None,
                        project: None,
                        tags: vec![],
                        min_importance: None,
                        top_k: req.top_k * 2,
                        privacy_context: privacy_ctx,
                    };
                    let deep_result = store.trapezoid_focus_recall(&req.query, &deep_filter, 1).unwrap_or_else(|e| {
                        eprintln!("[v1/enrich] 深度路径检索失败: {}", e);
                        RecallResult { memories: vec![], scores: vec![], total: 0 }
                    });

                    // RRF 融合 — 使用共享 rrf_fuse
                    let fused = crate::engine::rrf::rrf_fuse(
                        &fast_result,
                        &deep_result,
                        req.top_k,
                        crate::engine::rrf::RRF_DEFAULT_K,
                    );

                    let total = fused.total_candidates;
                    let memories: Vec<EnrichedMemory> = fused
                        .memories
                        .iter()
                        .zip(fused.scores.iter())
                        .map(|(m, &score)| EnrichedMemory {
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
            let store = metrics_store.clone();
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
        // GET /v1/health/system — 系统健康报告（可解释性面板）
        .route("/health/system", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    let mut store = store.lock().await;
                    match store.health_report() {
                        Ok(report) => {
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!(report)))
                        }
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "health_report_failed",
                                "message": format!("系统健康报告生成失败: {}", e)
                            })),
                        )),
                    }
                }
            }
        }))
        // GET /v1/health/detailed — 详细系统健康报告（运维级）
        // 质疑五核心端点：提供 GC 状态、反馈统计、调节器耦合信息等运维级数据
        .route("/health/detailed", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    let mut store = store.lock().await;
                    match store.health_report() {
                        Ok(report) => {
                            // 补充调节器耦合趋势分析（仅详细端点提供）
                            let coupling_trend = store.dao_regulator.analyze_coupling_trend();
                            let catastrophic_events = store.dao_regulator.get_catastrophic_events();
                            let gc_candidates = store.memory_gc.get_candidates();
                            let pending_actions = store.user_feedback.get_pending_actions();

                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "health": report,
                                "coupling_trend": coupling_trend,
                                "catastrophic_events": catastrophic_events,
                                "gc_candidates": gc_candidates,
                                "pending_user_actions": pending_actions,
                            })))
                        }
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "detailed_health_failed",
                                "message": format!("详细健康报告生成失败: {}", e)
                            })),
                        )),
                    }
                }
            }
        }))
        // POST /v1/feedback — 用户反馈回路（支持两阶段确认）
        //
        // 支持以下目标类型：
        //   - retrieval: 对检索结果的反馈
        //   - synthesis: 对合成质量的反馈
        //   - quarantine_override: 恢复被隔离的记忆
        //   - isolate: 请求隔离记忆（两阶段确认，阶段一）
        //   - confirm_action: 确认执行待处理操作（两阶段确认，阶段二）
        //   - cancel_action: 取消待处理操作
        .route("/feedback", post({
            let store = metrics_store.clone();
            move |Json(body): Json<serde_json::Value>| {
                let store = store.clone();
                async move {
                    // 解析反馈类型和目标类型
                    let feedback_type = match body.get("type").and_then(|v| v.as_str()).unwrap_or("neutral") {
                        "positive" => FeedbackType::Positive,
                        "negative" => FeedbackType::Negative,
                        _ => FeedbackType::Neutral,
                    };

                    let target_type = match body.get("target").and_then(|v| v.as_str()) {
                        Some("synthesis") => FeedbackTarget::SynthesisQuality,
                        Some("quarantine_override") => FeedbackTarget::QuarantineOverride,
                        Some("isolate") => FeedbackTarget::IsolateMemory,
                        Some("confirm_action") => FeedbackTarget::ConfirmAction,
                        Some("cancel_action") => FeedbackTarget::CancelAction,
                        _ => FeedbackTarget::RetrievalResult,
                    };

                    // 处理两阶段确认的特殊目标类型
                    match target_type {
                        FeedbackTarget::ConfirmAction => {
                            // 阶段二：确认执行待处理操作
                            let assessment_id = match body.get("assessment_id").and_then(|v| v.as_str()) {
                                Some(id) => id,
                                None => {
                                    return Err((
                                        StatusCode::BAD_REQUEST,
                                        Json(serde_json::json!({
                                            "error": "missing_assessment_id",
                                            "message": "缺少 assessment_id 参数"
                                        })),
                                    ));
                                }
                            };

                            let store = store.lock().await;
                            match store.user_feedback.confirm_action(assessment_id) {
                                Ok(memory_ids) => {
                                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                        "success": true,
                                        "action": "confirm",
                                        "assessment_id": assessment_id,
                                        "memory_ids": memory_ids,
                                        "message": "操作已确认执行，请等待下一个调节周期处理隔离"
                                    })))
                                }
                                Err(e) => Err((
                                    StatusCode::BAD_REQUEST,
                                    Json(serde_json::json!({
                                        "error": "confirmation_failed",
                                        "message": e
                                    })),
                                ))
                            }
                        }
                        FeedbackTarget::CancelAction => {
                            // 取消待处理操作
                            let assessment_id = match body.get("assessment_id").and_then(|v| v.as_str()) {
                                Some(id) => id,
                                None => {
                                    return Err((
                                        StatusCode::BAD_REQUEST,
                                        Json(serde_json::json!({
                                            "error": "missing_assessment_id",
                                            "message": "缺少 assessment_id 参数"
                                        })),
                                    ));
                                }
                            };

                            let store = store.lock().await;
                            match store.user_feedback.cancel_pending(assessment_id) {
                                Ok(_) => Ok(Json(serde_json::json!({
                                    "success": true,
                                    "action": "cancel",
                                    "assessment_id": assessment_id,
                                    "message": "操作已取消"
                                }))),
                                Err(e) => Err((
                                    StatusCode::BAD_REQUEST,
                                    Json(serde_json::json!({
                                        "error": "cancellation_failed",
                                        "message": e
                                    })),
                                ))
                            }
                        }
                        FeedbackTarget::IsolateMemory => {
                            // 阶段一：请求隔离记忆，生成影响评估报告
                            let memory_ids = match body.get("memory_ids") {
                                Some(serde_json::Value::Array(arr)) => {
                                    arr.iter()
                                        .filter_map(|v| v.as_str())
                                        .map(|s| s.to_string())
                                        .collect::<Vec<_>>()
                                }
                                _ => {
                                    // 兼容单记忆ID格式
                                    match body.get("memory_id").and_then(|v| v.as_str()) {
                                        Some(id) => vec![id.to_string()],
                                        None => {
                                            return Err((
                                                StatusCode::BAD_REQUEST,
                                                Json(serde_json::json!({
                                                    "error": "missing_memory_ids",
                                                    "message": "缺少 memory_ids 或 memory_id 参数"
                                                })),
                                            ));
                                        }
                                    }
                                }
                            };

                            if memory_ids.is_empty() {
                                return Err((
                                    StatusCode::BAD_REQUEST,
                                    Json(serde_json::json!({
                                        "error": "empty_memory_ids",
                                        "message": "memory_ids 不能为空"
                                    })),
                                ));
                            }

                            let store = store.lock().await;
                            // MemoryStore 需要实现 MemoryGraphQuery trait
                            let assessment = store.user_feedback.request_impact_assessment(
                                crate::engine::user_feedback::PendingActionType::Isolate,
                                &memory_ids,
                                &*store,
                            );

                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "action": "request_isolate",
                                "impact_assessment": assessment,
                                "message": "影响评估已生成，请审阅后确认执行或取消"
                            })))
                        }
                        _ => {
                            // 普通反馈记录（retrieval, synthesis, quarantine_override）
                            let memory_id = match body.get("memory_id").and_then(|v| v.as_str()) {
                                Some(id) => id.to_string(),
                                None => {
                                    return Err((
                                        StatusCode::BAD_REQUEST,
                                        Json(serde_json::json!({
                                            "error": "missing_memory_id",
                                            "message": "缺少 memory_id 参数"
                                        })),
                                    ));
                                }
                            };

                            let query = body.get("query").and_then(|v| v.as_str());
                            let note = body.get("note").and_then(|v| v.as_str());

                            let store = store.lock().await;
                            let feedback_id = store.user_feedback.record_feedback(
                                feedback_type.clone(),
                                target_type.clone(),
                                &memory_id,
                                query,
                                note,
                            );

                            let stats = store.user_feedback.get_stats();

                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "feedback_id": feedback_id,
                                "type": match feedback_type {
                                    FeedbackType::Positive => "positive",
                                    FeedbackType::Negative => "negative",
                                    FeedbackType::Neutral => "neutral",
                                },
                                "target": match target_type {
                                    FeedbackTarget::RetrievalResult => "retrieval",
                                    FeedbackTarget::SynthesisQuality => "synthesis",
                                    FeedbackTarget::QuarantineOverride => "quarantine_override",
                                    FeedbackTarget::IsolateMemory => "isolate",
                                    FeedbackTarget::ConfirmAction => "confirm_action",
                                    FeedbackTarget::CancelAction => "cancel_action",
                                },
                                "memory_id": memory_id,
                                "stats": {
                                    "total_feedback": stats.total_feedback,
                                    "positive_ratio": stats.positive_ratio,
                                },
                                "message": if target_type == FeedbackTarget::QuarantineOverride {
                                    "隔离恢复请求已记录，将在下一个调节周期中处理"
                                } else {
                                    "反馈已记录，感谢您的参与"
                                },
                            })))
                        }
                    }
                }
            }
        }))
        // GET /v1/audit-trail — 审计追踪（质疑五：透明度与信任）
        //
        // 提供完整的、可回溯的系统自主行为日志。
        // 支持按时间范围、事件类型、记忆 ID 过滤查询。
        // 查询参数：
        //   - from_ms: 起始时间戳（毫秒）
        //   - to_ms: 结束时间戳（毫秒）
        //   - event_types: 事件类型，逗号分隔（如 "synthesis_created,gc_cleanup"）
        //   - memory_id: 受影响的记忆 ID
        //   - limit: 最大返回条数，默认 100
        .route("/audit-trail", get({
            let store = metrics_store.clone();
            move |Query(params): Query<HashMap<String, String>>| {
                let store = store.clone();
                async move {
                    let store = store.lock().await;

                    let from_ms = params.get("from_ms").and_then(|v| v.parse::<u64>().ok());
                    let to_ms = params.get("to_ms").and_then(|v| v.parse::<u64>().ok());
                    let memory_id = params.get("memory_id").cloned();
                    let limit = params.get("limit").and_then(|v| v.parse::<usize>().ok());

                    let event_types: Option<Vec<AuditEventType>> = params.get("event_types").map(|s| {
                        s.split(',')
                            .filter_map(|t| match t.trim() {
                                "synthesis_created" => Some(AuditEventType::SynthesisCreated),
                                "memory_deleted" => Some(AuditEventType::MemoryDeleted),
                                "memory_isolated" => Some(AuditEventType::MemoryIsolated),
                                "decay_rate_changed" => Some(AuditEventType::DecayRateChanged),
                                "synthesis_threshold_changed" => Some(AuditEventType::SynthesisThresholdChanged),
                                "retrieval_weights_adjusted" => Some(AuditEventType::RetrievalWeightsAdjusted),
                                "reencoding_suggested" => Some(AuditEventType::ReencodingSuggested),
                                "gc_cleanup" => Some(AuditEventType::GcCleanup),
                                "regulation_applied" => Some(AuditEventType::RegulationApplied),
                                "feedback_processed" => Some(AuditEventType::FeedbackProcessed),
                                "comprehensive_rebalance" => Some(AuditEventType::ComprehensiveRebalance),
                                "catastrophic_event" => Some(AuditEventType::CatastrophicEvent),
                                "chronic_degradation" => Some(AuditEventType::ChronicDegradation),
                                "regulator_frozen" => Some(AuditEventType::RegulatorFrozen),
                                "regulator_unfrozen" => Some(AuditEventType::RegulatorUnfrozen),
                                _ => None,
                            })
                            .collect()
                    });

                    let query = AuditQuery {
                        from_ms,
                        to_ms,
                        event_types,
                        memory_id,
                        limit,
                    };

                    let events = store.audit_trail.query(&query);
                    let stats = store.audit_trail.type_statistics();

                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "total": events.len(),
                        "total_all": store.audit_trail.total_count(),
                        "type_statistics": stats,
                        "events": events,
                    })))
                }
            }
        }))
        // GET /v1/memories/stats — 记忆统计信息（仪表盘用）
        .route("/memories/stats", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    let store = store.lock().await;
                    match store.stats() {
                        Ok(stats) => {
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "total_memories": stats.total_memories,
                                "expired_count": stats.expired_count,
                                "storage_size_bytes": stats.storage_size_bytes,
                                "by_type": stats.by_type,
                                "by_project": stats.by_project,
                            })))
                        }
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "stats_failed",
                                "message": format!("记忆统计获取失败: {}", e)
                            })),
                        )),
                    }
                }
            }
        }))
        // v0.5.4 P1-7 新增：GET /v1/memories/recent — 获取最近记忆摘要（仪表盘用）
        //
        // 返回最近 N 条记忆的摘要信息（id、内容前 100 字符、类型、项目、创建时间、重要性），
        // 供仪表盘"最近记忆"区域展示。默认返回 5 条，可通过 ?limit 参数调整（最大 20）。
        .route("/memories/recent", get({
            let store = metrics_store.clone();
            move |Query(params): Query<RecentMemoriesParams>| {
                let store = store.clone();
                async move {
                    // 限制最大返回数量，防止滥用
                    let limit = params.limit.unwrap_or(5).clamp(1, 20);
                    let store = store.lock().await;

                    // 使用 ListFilter 按创建时间降序获取最近记忆
                    let filter = crate::memory_store::ListFilter {
                        limit,
                        offset: 0,
                        sort_by: crate::memory_store::SortBy::CreatedAt,
                        order: crate::memory_store::SortOrder::Desc,
                        ..Default::default()
                    };

                    match store.list_memories(&filter) {
                        Ok((memories, total)) => {
                            // 转换为摘要格式，避免泄露完整内容
                            let summaries: Vec<serde_json::Value> = memories
                                .iter()
                                .map(|m| {
                                    // 内容截断：超过 100 字符显示省略号
                                    let content_preview = if m.content.chars().count() > 100 {
                                        let truncated: String = m.content.chars().take(100).collect();
                                        format!("{}...", truncated)
                                    } else {
                                        m.content.clone()
                                    };

                                    serde_json::json!({
                                        "id": m.id,
                                        "content_preview": content_preview,
                                        "memory_type": m.memory_type.as_str(),
                                        "project": m.project.as_deref().unwrap_or("全局"),
                                        "created_at_ms": m.created_at.timestamp_millis(),
                                        "importance": m.importance.value(),
                                        "tags": m.tags,
                                    })
                                })
                                .collect();

                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "memories": summaries,
                                "total": total,
                                "returned": summaries.len(),
                            })))
                        }
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "recent_memories_failed",
                                "message": format!("最近记忆获取失败: {}", e)
                            })),
                        )),
                    }
                }
            }
        }))
        // ============================================================
        // 信任中心可验证性 API（质疑四：完美闭环悖论）
        // ============================================================

        // GET /v1/trust/data-location — 数据存储位置信息
        .route("/trust/data-location", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    let store = store.lock().await;
                    // 获取记忆文件实际路径
                    let data_dir = store.persistence().data_dir().to_path_buf();
                    let memory_file = data_dir.join("memories.json");
                    let file_exists = memory_file.exists();
                    let file_size = if file_exists {
                        std::fs::metadata(&memory_file).map(|m| m.len()).unwrap_or_else(|e| {
                            eprintln!("[v1/trust] 读取文件大小失败: {}", e);
                            0
                        })
                    } else {
                        0
                    };

                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "data_directory": data_dir.to_string_lossy(),
                        "memory_file": memory_file.to_string_lossy(),
                        "file_exists": file_exists,
                        "file_size_bytes": file_size,
                        "file_size_human": if file_size > 1024*1024 {
                            format!("{:.1} MB", file_size as f64 / (1024.0 * 1024.0))
                        } else if file_size > 1024 {
                            format!("{:.1} KB", file_size as f64 / 1024.0)
                        } else {
                            format!("{} B", file_size)
                        },
                        "storage_backend": "JSON 文件（本地存储）",
                        "is_local": true,
                        "network_required": false,
                    })))
                }
            }
        }))
        // GET /v1/trust/network-audit — 网络活动记录
        .route("/trust/network-audit", get({
            move || async move {
                // 检查是否有网络请求记录（通过环境变量追踪）
                let has_network = std::env::var("LRC_NETWORK_REQUESTS").unwrap_or_default();
                let requests: Vec<String> = if has_network.is_empty() {
                    vec![]
                } else {
                    has_network.split('|').map(|s| s.to_string()).collect()
                };

                Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                    "total_network_requests": requests.len(),
                    "requests": requests,
                    "network_policy": "本地优先 — 无网络也能正常工作",
                    "network_used_for": [
                        "首次下载 ML 模型（可选，使用 --mode fast 跳过）",
                        "LLM 查询翻译（可选，需配置 --llm-api）",
                        "检查更新（仅在用户主动触发时）"
                    ],
                    "no_telemetry": true,
                    "no_analytics": true,
                    "verification_note": "以下网络请求记录由系统运行时自动追踪，未经任何修改"
                })))
            }
        }))
        // GET /v1/trust/audit-integrity — 审计日志完整性验证
        .route("/trust/audit-integrity", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    let store = store.lock().await;
                    let total_events = store.audit_trail.total_count();
                    let integrity = store.audit_trail.verify_integrity();
                    let anchors = store.audit_trail.get_anchors();
                    let anchor_chain_valid = store.audit_trail.verify_anchor_chain();

                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "total_events": total_events,
                        "hash_chain_valid": integrity.is_valid,
                        "hash_chain_status": if integrity.is_valid { "完整 — 哈希链未被篡改" } else { "警告 — 检测到哈希链断裂" },
                        "hash_chain_details": integrity.details,
                        "anchor_count": anchors.len(),
                        "anchor_chain_valid": anchor_chain_valid,
                        "anchor_chain_status": if anchor_chain_valid { "完整 — 锚点链未被篡改" } else { "警告 — 检测到锚点链异常" },
                        "last_anchor_at": anchors.last().map(|a| a.created_at_ms),
                        "tamper_proof": integrity.is_valid && anchor_chain_valid,
                        "verification_note": "每次启动时自动验证哈希链完整性，任何篡改都会被检测到"
                    })))
                }
            }
        }))
        // GET /v1/captains-log — 船长日志一键生成演示
        // 产品化核心端点：输入项目路径，一键生成项目记忆全景报告
        .route("/captains-log", get({
            let store = metrics_store.clone();
            move |Query(params): Query<std::collections::HashMap<String, String>>| {
                let store = store.clone();
                async move {
                    let project_path = params.get("path").cloned().unwrap_or_else(|| ".".to_string());
                    let mut store = store.lock().await;

                    // 收集系统健康数据
                    let health = store.health_report().ok();
                    let dao_snapshot = store.dao_metrics_snapshot().ok();
                    let stats = store.stats().ok();
                    let audit_stats = store.audit_trail.type_statistics();

                    // 生成船长日志报告
                    let mut report = String::new();
                    report.push_str("═══════════════════════════════════════════\n");
                    report.push_str("  🏴‍☠️  Loong Recall 船长日志\n");
                    report.push_str("═══════════════════════════════════════════\n\n");

                    report.push_str(&format!("📂 项目路径: {}\n", project_path));
                    report.push_str(&format!("🕐 生成时间: {}\n\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")));

                    // 记忆统计
                    report.push_str("━━━ 📊 记忆统计 ━━━\n");
                    if let Some(ref s) = stats {
                        report.push_str(&format!("  记忆总数: {} 条\n", s.total_memories));
                        report.push_str(&format!("  已过期: {} 条\n", s.expired_count));
                        report.push_str("  类型分布:\n");
                        let mut types: Vec<_> = s.by_type.iter().collect();
                        types.sort_by(|a, b| b.1.cmp(a.1));
                        for (t, c) in types {
                            report.push_str(&format!("    - {}: {} 条\n", t, c));
                        }
                    } else {
                        report.push_str("  （暂无记忆数据）\n");
                    }

                    report.push_str("\n━━━ 🧘 道同构度 ━━━\n");
                    if let Some(ref dao) = dao_snapshot {
                        report.push_str(&format!("  道同构度: {:.1}%\n", dao.dao_isomorphism_score * 100.0));
                        report.push_str(&format!("  八卦分布熵: {:.3}\n", dao.bagua_entropy));
                        report.push_str(&format!("  合成比率: {:.1}%\n", dao.synthesis_ratio * 100.0));
                        report.push_str(&format!("  活跃记忆: {} 条\n", dao.active_memories));
                        report.push_str(&format!("  结晶记忆: {} 条\n", dao.crystallized_memories));
                    } else {
                        report.push_str("  （暂无道同构度数据）\n");
                    }

                    report.push_str("\n━━━ 🏥 系统健康 ━━━\n");
                    if let Some(ref h) = health {
                        report.push_str(&format!("  运行模式: {}\n", h.system_mode.as_str()));
                        report.push_str(&format!("  状态描述: {}\n", h.system_mode_description));
                        if !h.action_hints.is_empty() {
                            report.push_str("  行动建议:\n");
                            for hint in &h.action_hints {
                                report.push_str(&format!("    [{}] {} — {}\n",
                                    hint.severity, hint.message, hint.suggested_action));
                            }
                        }
                        // 道同构度摘要
                        if h.dao_metrics.dao_isomorphism_score < 0.5 {
                            report.push_str("  ⚠ 道同构度偏低，建议检查编码器状态\n");
                        }
                    } else {
                        report.push_str("  （暂无健康数据）\n");
                    }

                    report.push_str("\n━━━ 🔒 审计追踪 ━━━\n");
                    report.push_str(&format!("  审计事件总数: {} 条\n", audit_stats.values().sum::<usize>()));
                    report.push_str("  事件类型分布:\n");
                    let mut audit_types: Vec<_> = audit_stats.iter().collect();
                    audit_types.sort_by(|a, b| b.1.cmp(a.1));
                    for (t, c) in audit_types {
                        report.push_str(&format!("    - {}: {} 条\n", t, c));
                    }

                    report.push_str("\n━━━ 🎯 状态摘要 ━━━\n");
                    let status_emoji = if let Some(ref h) = health {
                        match h.system_mode {
                            crate::engine::health_report::SystemMode::Healthy => "✅ 系统运行健康",
                            crate::engine::health_report::SystemMode::Degraded => "⚠️ 编码器已降级，语义能力降低",
                            crate::engine::health_report::SystemMode::Oscillating => "🔄 系统参数正在自我调整中",
                            crate::engine::health_report::SystemMode::Drifting => "📉 检测到参数持续漂移，建议检查",
                            crate::engine::health_report::SystemMode::Frozen => "🧊 调节器已冻结，需要手动干预",
                            crate::engine::health_report::SystemMode::Overloaded => "📊 记忆库接近容量上限",
                        }
                    } else {
                        "🔧 系统正在初始化中"
                    };
                    report.push_str(&format!("  {}\n", status_emoji));

                    report.push_str("\n═══════════════════════════════════════════\n");
                    report.push_str("  💡 提示：使用 code-memory-server 启动服务后\n");
                    report.push_str(&format!("  访问 http://localhost:{}/dashboard 查看可视化仪表盘\n", DEFAULT_PORT));
                    report.push_str("═══════════════════════════════════════════\n");

                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "project_path": project_path,
                        "report": report,
                        "raw": {
                            "health": health,
                            "dao_snapshot": dao_snapshot,
                            "stats": stats,
                        }
                    })))
                }
            }
        }))
        // GET /v1/code/search — 代码库搜索（查询参数: query, top_k, keywords）
        // 返回与查询相关的代码片段，支持关键词和语义搜索
        .route("/code/search", get({
            let manager = codebase_manager.clone();
            move |Query(params): Query<std::collections::HashMap<String, String>>| {
                let manager = manager.clone();
                async move {
                    let query = params.get("query").cloned().unwrap_or_default();
                    let top_k = params.get("top_k")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(5)
                        .clamp(1, 100);
                    let keywords_str = params.get("keywords").cloned().unwrap_or_default();

                    let manager = manager.lock().await;

                    // 如果提供了 keywords 参数，则使用多关键词搜索
                    let result = if !keywords_str.is_empty() {
                        let keywords: Vec<String> = keywords_str
                            .split(',')
                            .map(|k| k.trim().to_string())
                            .filter(|k| !k.is_empty())
                            .collect();
                        if keywords.is_empty() {
                            manager.search(&query, top_k)
                        } else {
                            manager.multi_keyword_search(&keywords, top_k)
                        }
                    } else if !query.is_empty() {
                        manager.search(&query, top_k)
                    } else {
                        // 没有查询参数时返回空结果
                        crate::engine::retriever::RetrievalResult {
                            query: String::new(),
                            returned: 0,
                            total_indexed: manager.get_stats().total_chunks,
                            results: vec![],
                        }
                    };

                    let stats = manager.get_stats();

                    // 格式化为前端友好的 JSON 结构
                    let results: Vec<serde_json::Value> = result.results.iter().map(|r| {
                        serde_json::json!({
                            "rank": r.rank,
                            "score": r.score,
                            "file_path": r.chunk.file_path,
                            "name": r.chunk.name,
                            "language": r.chunk.language,
                            "start_line": r.chunk.start_line,
                            "end_line": r.chunk.end_line,
                            "content": r.chunk.content,
                            "doc_comment": r.chunk.doc_comment,
                        })
                    }).collect();

                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "query": result.query,
                        "returned": result.returned,
                        "total_indexed": result.total_indexed,
                        "results": results,
                        "stats": {
                            "file_count": stats.file_count,
                            "total_chunks": stats.total_chunks,
                            "avg_lines": stats.avg_lines,
                            "type_counts": stats.type_counts,
                        }
                    })))
                }
            }
        }))
        // GET /v1/version/check — 自动更新检测
        // 查询 GitHub Releases 获取最新版本号，与当前版本对比
        .route("/version/check", get({
            move || async move {
                let current_version = env!("CARGO_PKG_VERSION");
                let mut latest_version = "未知".to_string();
                let mut update_available = false;
                let mut update_url = String::new();
                let mut check_error = Option::<String>::None;

                // 尝试从 GitHub API 获取最新版本
                // 注意：此请求仅在用户主动触发时发起，不会自动上报任何数据
                if let Ok(client) = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .user_agent("loong-recall-version-check")
                    .build()
                {
                    match client
                        .get("https://api.github.com/repos/zhibaiYingChuan/LRC/releases/latest")
                        .send()
                        .await
                    {
                        Ok(resp) => {
                            if let Ok(json) = resp.json::<serde_json::Value>().await {
                                if let Some(tag) = json.get("tag_name").and_then(|v| v.as_str()) {
                                    latest_version = tag.trim_start_matches('v').to_string();
                                    update_url = json.get("html_url")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("https://github.com/zhibaiYingChuan/LRC/releases")
                                        .to_string();

                                    // 比较版本号
                                    update_available = compare_versions(&latest_version, current_version);
                                }
                            }
                        }
                        Err(e) => {
                            check_error = Some(format!("无法连接到 GitHub API: {}", e));
                        }
                    }
                } else {
                    check_error = Some("无法创建 HTTP 客户端".to_string());
                }

                Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                    "current_version": current_version,
                    "latest_version": latest_version,
                    "update_available": update_available,
                    "update_url": update_url,
                    "check_error": check_error,
                    "check_note": "版本检查仅在用户主动触发时发起，不会自动上报任何数据",
                    "download_url": format!("https://github.com/zhibaiYingChuan/LRC/releases/tag/v{}", latest_version),
                })))
            }
        }))
        // GET /v1/benchmarks/report — 三层基准测试报告
        // 使用缓存避免每次请求都重新运行耗时的基准测试
        .route("/benchmarks/report", get({
            || async move {
                // 优先返回缓存结果
                if let Some(cached) = BENCHMARK_CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(cached.clone()));
                }

                // 在独立线程中运行基准测试，添加 90 秒超时
                let report = tokio::time::timeout(
                    std::time::Duration::from_secs(90),
                    tokio::task::spawn_blocking(|| {
                        crate::benchmark::run_all_benchmarks(None)
                    })
                ).await;

                match report {
                    Ok(Ok(report)) => {
                        // 构建用户故事映射
                        let user_stories: std::collections::HashMap<&str, &str> = [
                            ("benchmark_retrieval_latency_scalability", "无论记忆库有多大，检索都能在眨眼间完成"),
                            ("benchmark_retrieval_recall_precision", "你搜索的内容，总能准确找到"),
                            ("benchmark_session_recall_accuracy", "你说过的话，它都记得"),
                            ("benchmark_memory_decay_effectiveness", "重要的约定历久弥新，临时的琐事自然淡忘"),
                            ("benchmark_synthesis_trigger_and_quality", "它会自己'悟'出规律：多次修复同类问题后，自动记住标准方案"),
                            ("benchmark_yin_yang_balance_stability", "系统有'内禀健康指标'，像生命体一样自我监控"),
                            ("benchmark_anti_pollution_capability", "在混乱中保持清醒：矛盾信息不会污染你的核心记忆"),
                            ("benchmark_data_localization", "你的记忆，只属于你。所有数据绝不会离开你的电脑"),
                            ("benchmark_audit_tamper_proof", "它对你绝对诚实：任何修改都有防篡改日志，可以被验证"),
                            ("benchmark_privacy_level_isolation", "不同隐私级别的记忆严格隔离，会话私密数据不会泄露给其他上下文"),
                            ("benchmark_complexity_red_line_self_check", "系统有自己的'健康红线'，不会让技术债务悄悄累积"),
                        ].iter().cloned().collect();

                        let layer_descriptions: std::collections::HashMap<u8, &str> = [
                            (1, "对标业界标准，证明 LRC 在基础检索能力上不输于人"),
                            (2, "只有 LRC 能做到的事——记忆演化、健康监控、抗污染"),
                            (3, "数据本地化、审计防篡改、隐私隔离——承诺可以被验证"),
                        ].iter().cloned().collect();

                        let layers: Vec<serde_json::Value> = report.layers.iter().enumerate().map(|(idx, l)| {
                            let layer_num = (idx + 1) as u8; // 层级编号 1/2/3，替代脆弱字符串匹配
                            serde_json::json!({
                                "name": l.name,
                                "description": layer_descriptions.get(&layer_num).unwrap_or(&""),
                                "total": l.total,
                                "passed": l.passed,
                                "status": l.status,
                                "tests": report.results.iter()
                                    .filter(|r| r.layer == layer_num)
                                    .map(|r| serde_json::json!({
                                        "name": r.name,
                                        "function": format!("benchmark_{}", r.name),
                                        "status": if r.passed { "PASS" } else { "FAIL" },
                                        "description": r.description,
                                        "user_story": user_stories.get(r.name.as_str()).unwrap_or(&""),
                                        "metric": r.details,
                                        "score": r.score,
                                        "duration_ms": r.duration_ms,
                                    }))
                                    .collect::<Vec<_>>(),
                            })
                        }).collect();

                        let result = serde_json::json!({
                            "report_version": report.version,
                            "generated_at": report.generated_at,
                            "summary": {
                                "total_tests": report.total,
                                "passed": report.passed,
                                "failed": report.failed,
                                "status": if report.failed == 0 { "PASS" } else { "FAIL" },
                            },
                            "layers": layers,
                            "radar_chart": report.radar_scores,
                            "note": "本报告通过实际运行基准测试生成，反映系统当前能力水平"
                        });

                        // 缓存结果
                        if let Ok(mut cache) = BENCHMARK_CACHE.lock() {
                            *cache = Some(result.clone());
                        }

                        Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(result))
                    }
                    Ok(Err(e)) => Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": "benchmark_failed",
                            "message": format!("基准测试运行失败: {}", 
                                if e.is_panic() { "测试过程发生内部错误".to_string() } 
                                else { e.to_string() })
                        })),
                    )),
                    Err(_timeout) => {
                        // 超时：返回降级提示
                        Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(serde_json::json!({
                                "error": "benchmark_timeout",
                                "message": "基准测试运行超时（90秒），请稍后重试。首次运行需加载模型，可能需要较长时间。",
                                "hint": "刷新页面重试，后续请求将使用缓存结果"
                            })),
                        ))
                    }
                }
            }
        }))
}

/// 比较版本号：latest > current 返回 true
///
/// 支持语义化版本号（semver）比较，如 "0.2.0" > "0.1.0"
/// 版本号格式：major.minor.patch
fn compare_versions(latest: &str, current: &str) -> bool {
    let parse =
        |v: &str| -> Vec<u32> { v.split('.').filter_map(|s| s.parse::<u32>().ok()).collect() };

    let latest_parts = parse(latest);
    let current_parts = parse(current);

    if latest_parts.is_empty() || current_parts.is_empty() {
        return false;
    }

    let max_len = latest_parts.len().max(current_parts.len());
    for i in 0..max_len {
        let l = latest_parts.get(i).copied().unwrap_or(0);
        let c = current_parts.get(i).copied().unwrap_or(0);
        if l > c {
            return true;
        } else if l < c {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn test_compare_versions() {
        assert!(compare_versions("0.2.0", "0.1.0"));
        assert!(!compare_versions("0.1.0", "0.2.0"));
        assert!(!compare_versions("0.2.0", "0.2.0"));
        assert!(compare_versions("1.0.0", "0.9.9"));
        assert!(!compare_versions("0.1.0", "1.0.0"));
        assert!(compare_versions("0.2.1", "0.2.0"));
    }
}
