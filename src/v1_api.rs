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
use crate::persistence::Persistence;
use crate::server::IndexedCodebase;
use crate::{LlmApiConfig, RecallResult};
use axum::{
    extract::Query,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::{Mutex, RwLock};

/// 基准测试报告缓存（避免每次请求都重新运行耗时的基准测试）
/// v0.5.6：添加缓存时间戳，支持 1 小时过期机制
static BENCHMARK_CACHE: std::sync::LazyLock<
    StdMutex<Option<(serde_json::Value, std::time::Instant)>>,
> = std::sync::LazyLock::new(|| StdMutex::new(None));

/// 基准测试缓存有效期：1 小时
const BENCHMARK_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

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

/// /v1/health/dao_metrics 响应体（v0.8.1：契约对齐，包装为 {ok, data, raw} 结构）
///
/// 字段说明：
///   - yin_yang_balance: 阴阳守恒度（0-100），派生自 dao_isomorphism_score * 100
///   - luoshu_deviation: 洛书偏差（0-100），派生自 (1 - dao_isomorphism_score) * 100
///   - bagua_balance: 八卦均衡度（0-100），派生自 (1 - bagua_entropy) * 100
///   - synthesis_ratio: 合成比率（0-100 百分比），原始值 * 100
#[derive(Debug, Serialize)]
pub struct DaoMetricsData {
    pub yin_yang_balance: f32,
    pub luoshu_deviation: f32,
    pub bagua_balance: f32,
    pub synthesis_ratio: f32,
    // 保留原始诊断字段（前端展示用）
    pub dao_isomorphism_score: f32,
    pub active_memories: usize,
    pub crystallized_memories: usize,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct DaoMetricsRaw {
    pub bagua_entropy: f32,
    pub archived_memories: usize,
    pub encodings_total: u64,
    pub compositions_total: u64,
    pub recalls_total: u64,
    pub corrections_total: u64,
}

#[derive(Debug, Serialize)]
pub struct DaoMetricsResponse {
    pub ok: bool,
    pub data: DaoMetricsData,
    pub raw: DaoMetricsRaw,
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

/// v0.6.0 新增：/v1/memories/list 请求体
///
/// 用于备份导出时获取全量记忆列表。
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryListRequest {
    /// 返回的记忆数量（默认 10000，最大 50000）
    pub limit: Option<usize>,
}

/// v0.6.0 新增：/v1/memories/remember 请求体
///
/// 用于导入备份时逐条写入记忆。字段与前端 app.js 调用对齐。
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryRememberRequest {
    /// 记忆内容（必填）
    pub content: String,
    /// 记忆类型（如 fact, decision, preference 等）
    pub memory_type: String,
    /// 重要性 1-10（默认 5）
    pub importance: Option<u8>,
}

/// v0.8.1 新增：/v1/config/llm/test 请求体
///
/// 前端 testLlmConfig 通过 sidecar 转发 LLM 测试请求，
/// 绕过浏览器 CSP connect-src 限制。
#[derive(Debug, Clone, Deserialize)]
pub struct LlmTestRequest {
    /// LLM API 端点（如 https://api.deepseek.com）
    pub endpoint: String,
    /// API Key
    pub api_key: String,
    /// 供应商名称（可选，用于日志）
    #[serde(default)]
    pub provider: Option<String>,
}

/// v0.8.1 新增：/v1/config/llm/test 响应体
#[derive(Debug, Serialize)]
pub struct LlmTestResponse {
    pub ok: bool,
    pub status: u16,
    pub message: String,
    pub latency_ms: u64,
}

// ==================== 路由构建 ====================

/// 创建 v1 REST API 路由（状态类型为 ()，以便与主路由合并）
///
/// 通过闭包捕获 memory_store 和 codebase_manager，无需使用 axum State。
pub fn build_v1_router(
    store: SharedStore,
    codebase_manager: Arc<Mutex<Box<dyn IndexedCodebase>>>,
    llm_api: Arc<RwLock<LlmApiConfig>>,
    // v0.8.22 P0-1 修复：传入 LLM 配置状态的无锁缓存，便于 /v1/config/llm 更新时同步
    llm_configured_atomic: Arc<std::sync::atomic::AtomicBool>,
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
                    // v0.7.1 P1-2 修复：用 spawn_blocking 包裹同步编码调用，
                    // 避免 ML feature 下阻塞 Tokio worker 线程
                    let text = req.text;
                    let luoshu_vec = tokio::task::spawn_blocking(move || {
                        encoder.encode_text(&text)
                    })
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": format!("编码任务执行失败: {}", e)
                            })),
                        )
                    })?;

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
                    // v0.8.22 P2-NEW-03 修复（interaction-resilience-auditor Round4）：
                    //   根因：consolidate handler 在 tokio worker 线程上持锁执行
                    //         luoshu_synthesize()（CPU 密集），与 P0-3 修复前的问题一致
                    //   修复：三阶段锁安全模式
                    //     Phase 1：持锁写入记忆（快速操作，<1ms）
                    //     Phase 2：释放锁，spawn_blocking 执行 luoshu_synthesize（CPU 密集）
                    //     Phase 3：重新持锁，列出记忆和获取总数（快速操作，<1ms）

                    // Phase 1：持锁写入记忆
                    let mut stored = 0usize;
                    {
                        let mut store = store.lock().await;
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
                    } // 锁释放

                    // Phase 2：spawn_blocking 执行 luoshu_synthesize（CPU 密集，不占 tokio worker）
                    let store_arc = store.clone();
                    let synthesized = match tokio::task::spawn_blocking(move || {
                        let mut store = store_arc.blocking_lock();
                        store.luoshu_synthesize()
                    }).await {
                        Ok(Ok(n)) => n,
                        Ok(Err(e)) => {
                            eprintln!("[v1/consolidate] 合成失败: {}", e);
                            0
                        }
                        Err(e) => {
                            eprintln!("[v1/consolidate] spawn_blocking panic: {}", e);
                            0
                        }
                    };

                    // Phase 3：重新持锁，列出记忆和获取总数（快速操作）
                    let (synthesis_summaries, total) = {
                        let store = store.lock().await;
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
                        (synthesis_summaries, total)
                    }; // 锁释放

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
        // v0.8.22 P1-02 修复：lock_busy 时返回降级数据而非 503
        .route("/health/dao_metrics", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    // v0.8.19 P0-1b 修复：改用 try_lock，避免结晶流水线持锁时卡死
                    // v0.8.22 P1-02：lock_busy 时返回降级数据而非 503
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            // v0.8.22 P1-02：lock_busy 时返回降级数据而非 503
                            // v0.8.22 P3-NEW-06 修复：active_memories/crystallized_memories 改为 null，
                            //   避免外部 API 消费者将 0 误认为"系统无记忆"。
                            //   前端已有 hasLockBusy200 检查（P1-NEW-01），不会渲染降级数据。
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "ok": true,
                                "data": {
                                    "yin_yang_balance": 0.0,
                                    "luoshu_deviation": 100.0,
                                    "bagua_balance": 0.0,
                                    "synthesis_ratio": 0.0,
                                    "dao_isomorphism_score": 0.0,
                                    "active_memories": null,
                                    "crystallized_memories": null,
                                    "status": "loading"
                                },
                                "raw": {
                                    "bagua_entropy": 1.0,
                                    "archived_memories": 0,
                                    "encodings_total": 0,
                                    "compositions_total": 0,
                                    "recalls_total": 0,
                                    "corrections_total": 0
                                },
                                "lock_busy": true,
                                "degraded": true,
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                            })));
                        }
                    };
                    match store.dao_metrics_snapshot() {
                        Ok(snapshot) => {
                            let status = if snapshot.dao_isomorphism_score < 0.3 {
                                "critical"
                            } else if snapshot.dao_isomorphism_score < 0.5 {
                                "warning"
                            } else {
                                "healthy"
                            };
                            // v0.8.1：派生前端友好字段（0-100 区间）
                            let yin_yang_balance = snapshot.dao_isomorphism_score * 100.0;
                            let luoshu_deviation = (1.0 - snapshot.dao_isomorphism_score) * 100.0;
                            let bagua_balance = (1.0 - snapshot.bagua_entropy) * 100.0;
                            let synthesis_ratio_pct = snapshot.synthesis_ratio * 100.0;

                            // v0.8.22 P1-02 类型修复：lock_busy 路径返回 serde_json::Value，
                            // 正常路径也需保持类型一致（DaoMetricsResponse → Value）
                            let response = DaoMetricsResponse {
                                ok: true,
                                data: DaoMetricsData {
                                    yin_yang_balance,
                                    luoshu_deviation,
                                    bagua_balance,
                                    synthesis_ratio: synthesis_ratio_pct,
                                    dao_isomorphism_score: snapshot.dao_isomorphism_score,
                                    active_memories: snapshot.active_memories,
                                    crystallized_memories: snapshot.crystallized_memories,
                                    status: status.to_string(),
                                },
                                raw: DaoMetricsRaw {
                                    bagua_entropy: snapshot.bagua_entropy,
                                    archived_memories: snapshot.archived_memories,
                                    encodings_total: snapshot.encodings_total,
                                    compositions_total: snapshot.compositions_total,
                                    recalls_total: snapshot.recalls_total,
                                    corrections_total: snapshot.corrections_total,
                                },
                            };
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(
                                serde_json::to_value(&response).unwrap_or_else(|_| serde_json::json!({
                                    "ok": false,
                                    "error": "serialize_failed",
                                    "message": "道同构度数据序列化失败"
                                }))
                            ))
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
        // v0.8.22 P1-02 修复：lock_busy 时返回降级数据而非 503
        .route("/health/system", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    // v0.8.19 P0-1b 修复：改用 try_lock，避免结晶流水线持锁时卡死
                    // v0.8.22 P1-02：lock_busy 时返回降级数据而非 503
                    let mut store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            // v0.8.48 修复：lock_busy 降级时返回完整的系统状态框架
                            //   前端会在 loadDashboard 中检测 lock_busy 并渲染降级数据
                            //   系统浮窗需要 encoder / system_mode / memory_stats 字段，
                            //   不能返回 null，否则前端显示 "--"
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "ok": true,
                                "lock_busy": true,
                                "degraded": true,
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载",
                                "system_mode": "healthy",
                                "system_mode_description": "后台合成中，数据稍后刷新",
                                "encoder": {
                                    "mode": "statistical",
                                    "model_name": null,
                                    "hidden_size": null,
                                    "degradation_reason": "系统合成中",
                                    "total_encodings": 0,
                                    "last_encoding_ms": 0,
                                    "capability_description": "系统合成中",
                                    "quality_score": 0.0
                                },
                                "memory_stats": {
                                    "total_memories": 0,
                                    "active_memories": 0,
                                    "synthesis_memories": 0,
                                    "expired_memories": 0,
                                    "low_quality_synthesis": 0,
                                    "bagua_distribution": [0, 0, 0, 0, 0, 0, 0, 0]
                                },
                                "dao_metrics": {
                                    "active_memories": 0,
                                    "crystallized_memories": 0,
                                    "archived_memories": 0,
                                    "encodings_total": 0,
                                    "compositions_total": 0,
                                    "recalls_total": 0,
                                    "corrections_total": 0,
                                    "yin_yang_balance": 0.0,
                                    "luoshu_deviation": 0.0,
                                    "bagua_balance": 0.0,
                                    "synthesis_ratio": 0.0,
                                    "dao_isomorphism_score": 0.0
                                }
                            })));
                        }
                    };
                    match store.health_report() {
                        Ok(report) => {
                            // v0.8.23 CI 修复：始终添加 lock_busy 和 degraded 字段，
                            //   满足 CI E2E smoke test 的正常路径和降级路径统一校验
                            let mut json = serde_json::json!(report);
                            if let Some(obj) = json.as_object_mut() {
                                obj.insert("lock_busy".to_string(), serde_json::Value::Bool(false));
                                let is_degraded = obj.get("system_mode")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s == "degraded")
                                    .unwrap_or(false);
                                obj.insert("degraded".to_string(), serde_json::Value::Bool(is_degraded));
                                // v0.8.25：新增 version 字段，从 Cargo.toml 编译期注入
                                obj.insert("version".to_string(), serde_json::Value::String(
                                    env!("CARGO_PKG_VERSION").to_string()
                                ));
                            }
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(json))
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
        //
        // v0.8.21 P0-01 修复（GAP-P0-01 / interaction-resilience-auditor）：
        //   原实现使用 lock().await，后台合成持锁时请求挂起 10s 直到前端超时，
        //   导致 loadDashboard 的 Promise.allSettled 被拖死 10s。
        //   修复：改用 try_lock，锁被持有时返回 503 lock_busy，与 /v1/health/system 一致。
        //
        // v0.8.22 P1-02 修复（interaction-resilience-auditor Round5）：
        //   根因：try_lock 失败时返回 503，前端间歇性收到 503 lock_busy，
        //         虽有 30s 冷却期但用户体验仍受影响
        //   修复：lock_busy 时返回 200 + 降级数据（空字段 + lock_busy 标记），
        //         前端正常渲染部分数据，不触发 503 处理逻辑
        .route("/health/detailed", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    // v0.8.21 P0-01：try_lock 避免 lock().await 阻塞 10s
                    // v0.8.22 P1-02：lock_busy 时返回降级数据而非 503
                    let mut store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            // 返回 200 + 降级数据，避免前端 503 处理
                            // v0.8.22 P3-NEW-06：添加 degraded 标记，与 dao_metrics/system 保持一致
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "health": null,
                                "coupling_trend": [],
                                "catastrophic_events": [],
                                "gc_candidates": [],
                                "pending_user_actions": [],
                                "lock_busy": true,
                                "degraded": true,
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                            })));
                        }
                    };
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
                    // v0.8.22 HCSE 修复：改用 try_lock，避免 lock_busy 期间超时
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Err::<_, (StatusCode, Json<serde_json::Value>)>((
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(serde_json::json!({
                                    "ok": false,
                                    "error": "lock_busy",
                                    "lock_busy": true,
                                    "message": "记忆系统正在执行后台合成，请稍后重试"
                                })),
                            ));
                        }
                    };

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
                    // v0.8.19 P0-1b 修复：改用 try_lock，避免结晶流水线持锁时卡死
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Err::<_, (StatusCode, Json<serde_json::Value>)>((
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(serde_json::json!({
                                    "ok": false,
                                    "error": "lock_busy",
                                    "lock_busy": true,
                                    "message": "记忆系统正在执行后台合成，请稍后重试"
                                })),
                            ));
                        }
                    };
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
                    // v0.8.45 修复：改用 try_read，避免 lock_busy 期间挂起超时（与 /memories/stats 一致）
                    //   根因：原实现 lock().await 在结晶持锁时阻塞等待，前端 fetchWithTimeout 8s 超时
                    //         显示"加载失败"，而非 v0.8.45 前端预期的"后台合成中"降级提示
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Err::<_, (StatusCode, Json<serde_json::Value>)>((
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(serde_json::json!({
                                    "ok": false,
                                    "error": "lock_busy",
                                    "lock_busy": true,
                                    "message": "记忆系统正在执行后台合成，请稍后重试"
                                })),
                            ));
                        }
                    };

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
        // 记忆备份与恢复 API（审计 P0-1 修复）
        // ============================================================
        //
        // v0.6.0 新增：POST /v1/memories/list — 获取记忆列表（备份导出用）
        //
        // 返回全量记忆列表（不截断内容），供前端 backupMemories 导出 JSON 备份。
        // 与 /memories/recent 的区别：recent 返回摘要且限制 20 条，list 返回完整内容。
        .route("/memories/list", post({
            let store = metrics_store.clone();
            move |Json(params): Json<MemoryListRequest>| {
                let store = store.clone();
                async move {
                    // 限制最大返回数量，防止内存溢出
                    let limit = params.limit.unwrap_or(10000).clamp(1, 50000);
                    // v0.8.45 修复：改用 try_read，避免 lock_busy 期间挂起超时（与 /memories/recent 一致）
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Err::<_, (StatusCode, Json<serde_json::Value>)>((
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(serde_json::json!({
                                    "ok": false,
                                    "error": "lock_busy",
                                    "lock_busy": true,
                                    "message": "记忆系统正在执行后台合成，请稍后重试"
                                })),
                            ));
                        }
                    };

                    let filter = crate::memory_store::ListFilter {
                        limit,
                        offset: 0,
                        sort_by: crate::memory_store::SortBy::CreatedAt,
                        order: crate::memory_store::SortOrder::Desc,
                        ..Default::default()
                    };

                    match store.list_memories(&filter) {
                        Ok((memories, total)) => {
                            let memories_json: Vec<serde_json::Value> = memories
                                .iter()
                                .map(|m| {
                                    serde_json::json!({
                                        "id": m.id,
                                        "content": m.content,
                                        "memory_type": m.memory_type.as_str(),
                                        "project": m.project,
                                        "created_at_ms": m.created_at.timestamp_millis(),
                                        "importance": m.importance.value(),
                                        "tags": m.tags,
                                    })
                                })
                                .collect();

                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "memories": memories_json,
                                "total": total,
                            })))
                        }
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "list_memories_failed",
                                "message": format!("记忆列表获取失败: {}", e)
                            })),
                        )),
                    }
                }
            }
        }))
        //
        // v0.6.0 新增：POST /v1/memories/archive — 获取归档记忆列表（备份导出用）
        //
        // 返回已归档的记忆列表，供前端 backupMemories 导出完整备份。
        .route("/memories/archive", post({
            let store = metrics_store.clone();
            move |_body: Json<serde_json::Value>| {
                let store = store.clone();
                async move {
                    let store = store.lock().await;

                    // 通过持久层加载归档记忆
                    match store.persistence().load_archived_memories() {
                        Ok(archived_memories) => {
                            let archived: Vec<serde_json::Value> = archived_memories
                                .iter()
                                .map(|m| {
                                    serde_json::json!({
                                        "id": m.id,
                                        "content": m.content,
                                        "memory_type": m.memory_type.as_str(),
                                        "project": m.project,
                                        "created_at_ms": m.created_at.timestamp_millis(),
                                        "importance": m.importance.value(),
                                        "tags": m.tags,
                                    })
                                })
                                .collect();

                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "archive": archived,
                                "total": archived.len(),
                            })))
                        }
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "archive_list_failed",
                                "message": format!("归档记忆获取失败: {}", e)
                            })),
                        )),
                    }
                }
            }
        }))
        //
        // v0.6.0 新增：POST /v1/memories/remember — 写入单条记忆（导入恢复用）
        //
        // 接收前端导入备份时逐条写入的记忆数据，字段与前端 JSON.stringify 对齐。
        .route("/memories/remember", post({
            let store = metrics_store.clone();
            move |Json(params): Json<MemoryRememberRequest>| {
                let store = store.clone();
                async move {
                    // 输入校验：content 不能为空
                    if params.content.trim().is_empty() {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "error": "invalid_input",
                                "message": "content 字段不能为空"
                            })),
                        ));
                    }

                    // 解析记忆类型，无效时回退到 Fact
                    let memory_type = MemoryType::from_str(&params.memory_type)
                        .unwrap_or(MemoryType::Fact);

                    // 解析重要性，限制 1-10
                    let importance = Importance::new(params.importance.unwrap_or(5));

                    let mut store = store.lock().await;

                    // 构造新记忆（project 和 tags 暂为空，ttl 永久）
                    let memory = Memory::new(
                        params.content,
                        memory_type,
                        None,
                        Vec::new(),
                        importance,
                        None,
                    );

                    match store.remember(memory) {
                        Ok(id) => Ok::<_, (StatusCode, Json<serde_json::Value>)>(
                            Json(serde_json::json!({
                                "success": true,
                                "memory_id": id,
                            }))
                        ),
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "remember_failed",
                                "message": format!("记忆写入失败: {}", e)
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
                    // v0.8.22 HCSE 修复：改用 try_read，避免 lock_busy 期间超时
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Err::<_, (StatusCode, Json<serde_json::Value>)>((
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(serde_json::json!({
                                    "ok": false,
                                    "error": "lock_busy",
                                    "lock_busy": true,
                                    "message": "记忆系统正在执行后台合成，请稍后重试"
                                })),
                            ));
                        }
                    };
                    // 获取记忆文件实际路径
                    let data_dir = store.persistence().data_dir().to_path_buf();
                    let memory_file = data_dir.join("memories.json");
                    // v0.7.1 P2-1 修复：用 spawn_blocking 包裹同步文件 I/O
                    let memory_file_clone = memory_file.clone();
                    let data_dir_clone = data_dir.clone();
                    let (file_exists, file_size, memory_count, last_backup_time) = tokio::task::spawn_blocking(move || {
                        let exists = memory_file_clone.exists();
                        let size = if exists {
                            std::fs::metadata(&memory_file_clone).map(|m| m.len()).unwrap_or_else(|e| {
                                eprintln!("[v1/trust] 读取文件大小失败: {}", e);
                                0
                            })
                        } else {
                            0
                        };

                        // v0.8.0 "归一"：读取记忆数量（直接解析 JSON 文件，避免在 spawn_blocking 中获取锁）
                        let count = if exists {
                            std::fs::read_to_string(&memory_file_clone)
                                .ok()
                                .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
                                .and_then(|v| {
                                    if v.is_array() {
                                        Some(v.as_array().unwrap().len())
                                    } else if v.is_object() {
                                        v.get("memories")
                                            .and_then(|m| m.as_array())
                                            .map(|a| a.len())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(0)
                        } else {
                            0
                        };

                        // v0.8.0 "归一"：检查 backups 目录获取最后备份时间
                        let backups_dir = data_dir_clone
                            .parent() // data/ 的父目录
                            .map(|p| p.join("backups"))
                            .unwrap_or_else(|| std::path::PathBuf::from(".loong-recall/backups"));
                        let last_backup = if backups_dir.exists() {
                            std::fs::read_dir(&backups_dir)
                                .ok()
                                .and_then(|entries| {
                                    entries
                                        .filter_map(|e| e.ok())
                                        .filter_map(|e| {
                                            e.metadata()
                                                .ok()
                                                .and_then(|m| m.modified().ok())
                                        })
                                        .max()
                                })
                                .and_then(|t| {
                                    t.duration_since(std::time::UNIX_EPOCH)
                                        .ok()
                                        .map(|d| d.as_secs())
                                })
                        } else {
                            None
                        };

                        (exists, size, count, last_backup)
                    })
                    .await
                    .unwrap_or((false, 0, 0, None));

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
                        "memory_count": memory_count,
                        "last_backup_time": last_backup_time,
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
                    // v0.8.22 HCSE 修复：改用 try_read，避免 lock_busy 期间超时
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Err::<_, (StatusCode, Json<serde_json::Value>)>((
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(serde_json::json!({
                                    "ok": false,
                                    "error": "lock_busy",
                                    "lock_busy": true,
                                    "message": "记忆系统正在执行后台合成，请稍后重试"
                                })),
                            ));
                        }
                    };
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
                    // v0.8.19 P0-1b 修复：改用 try_lock，避免结晶流水线持锁时卡死
                    let mut store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Err::<_, (StatusCode, Json<serde_json::Value>)>((
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(serde_json::json!({
                                    "ok": false,
                                    "error": "lock_busy",
                                    "lock_busy": true,
                                    "message": "记忆系统正在执行后台合成，请稍后重试"
                                })),
                            ));
                        }
                    };

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
                        // v0.6.1 P0-2 修复: query 和 keywords 均为空时,回退返回最近 top_k 条
                        // 修复前: 返回空结果,导致导出代码片段功能在无查询参数时失效
                        // 修复后: 返回最近索引的 top_k 条代码片段,确保导出功能可用
                        manager.recent_chunks(top_k)
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
        // v0.8.25 新增：POST /v1/model/test — 测试模型编码器连通性
        // 发送一段测试文本到编码器，验证模型是否正常响应
        // 区别于 /v1/encode（常规编码），此端点仅用于连通性验证
        .route("/model/test", post({
            let encoder = encode_encoder.clone();
            move || {
                let encoder = encoder.clone();
                async move {
                    let test_text = "这是一个模型连通性测试。";
                    let start = std::time::Instant::now();

                    // v0.8.25 R-12：添加 15s 硬超时保护，防止编码器卡死导致请求挂起
                    // v0.8.25 GAP-17 修复：添加取消标志，超时后通知任务放弃执行
                    // 注意：spawn_blocking 提交后，即使 JoinHandle 被 drop，
                    // 底层的 blocking 线程仍会继续运行已启动的任务（Rust 异步运行时限制）。
                    // 取消标志可确保：超时后任务即使尚未启动也立即返回，不浪费线程池资源。
                    let cancel_flag = Arc::new(AtomicBool::new(false));
                    let cancel_flag_inner = cancel_flag.clone(); // 预留给 spawn_blocking 内部使用

                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        tokio::task::spawn_blocking(move || {
                            if cancel_flag_inner.load(Ordering::SeqCst) {
                                // 取消标志已设置（超时触发），返回空值表示已取消
                                // 编码器不会被占用，线程池资源立即释放
                                return None;
                            }
                            Some(encoder.encode_text(test_text))
                        })
                    )
                    .await
                    .map_err(|_| {
                        // 超时路径：15s 内未完成编码，返回 504 Gateway Timeout
                        // 设置取消标志，通知 spawn_blocking 任务（如果尚未启动）放弃执行
                        cancel_flag.store(true, Ordering::SeqCst);
                        eprintln!(
                            "[v1/model/test] 超时（15s），编码任务已通知取消。\
                             如果编码器当前被长时间占用，请检查模型状态或增大超时时间"
                        );
                        (
                            StatusCode::GATEWAY_TIMEOUT,
                            Json(serde_json::json!({
                                "ok": false,
                                "error": "model_test_timeout",
                                "message": "模型测试超时（15s），请确认模型已下载并应用".to_string()
                            })),
                        )
                    })?
                    .map_err(|e| {
                        // spawn_blocking 内部 panic 处理
                        eprintln!("[v1/model/test] spawn_blocking panic: {}", e);
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "ok": false,
                                "error": "model_test_crashed",
                                "message": format!("模型测试执行失败: {}", e)
                            })),
                        )
                    })?;
                    // 检查是否因取消标志导致返回 None
                    let luoshu_vec = match result {
                        Some(v) => v,
                        None => {
                            eprintln!("[v1/model/test] 编码任务已因取消标志提前终止");
                            return Err((
                                StatusCode::GATEWAY_TIMEOUT,
                                Json(serde_json::json!({
                                    "ok": false,
                                    "error": "model_test_cancelled",
                                    "message": "模型测试任务已被取消".to_string()
                                })),
                            ));
                        }
                    };
                    let elapsed_ms = start.elapsed().as_millis() as u64;
                    Ok::<_, (StatusCode, Json<serde_json::value::Value>)>(Json(serde_json::json!({
                        "ok": true,
                        "message": "模型响应正常",
                        "vector_dim": luoshu_vec.values.len(),
                        "elapsed_ms": elapsed_ms,
                        "center_value": luoshu_vec.center_value(),
                        "bagua_category": crate::engine::mirror_trapezoid::mirror_project(&luoshu_vec).best_category.to_string(),
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
                // 优先返回缓存结果（v0.5.6：检查缓存是否过期）
                if let Some((cached, cached_at)) = BENCHMARK_CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    if cached_at.elapsed() < BENCHMARK_CACHE_TTL {
                        return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(cached.clone()));
                    }
                    // 缓存已过期，清空缓存
                    eprintln!("[基准报告] 缓存已过期（超过 {} 秒），重新运行基准测试", BENCHMARK_CACHE_TTL.as_secs());
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

                        // 缓存结果（v0.5.6：记录缓存时间，支持 TTL 过期）
                        if let Ok(mut cache) = BENCHMARK_CACHE.lock() {
                            *cache = Some((result.clone(), std::time::Instant::now()));
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
        // POST /v1/migrate — v0.8.0 "归一"：数据迁移与合并
        // 扫描所有已知老路径，按 memory.id 去重合并到 global 目录
        .route("/migrate", post(|| async move {
            // v0.8.0：迁移是同步文件 I/O 操作，用 spawn_blocking 避免阻塞 Tokio
            let report = tokio::task::spawn_blocking(|| {
                crate::migration::execute_migration()
            })
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "success": false,
                        "error": format!("迁移任务执行失败: {}", e)
                    })),
                )
            })?;
            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::to_value(&report).unwrap_or_else(|_| {
                serde_json::json!({"success": false, "error": "序列化迁移报告失败"})
            })))
        }))
        // POST /v1/backup — v0.8.0 "归一"：手动创建记忆备份
        // 将 global/data/memories.json 复制到 ~/.loong-recall/backups/
        .route("/backup", post(|| async move {
            let report = tokio::task::spawn_blocking(|| {
                crate::backup::create_backup()
            })
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "success": false,
                        "error": format!("备份任务执行失败: {}", e)
                    })),
                )
            })?;
            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::to_value(&report).unwrap_or_else(|_| {
                serde_json::json!({"success": false, "error": "序列化备份报告失败"})
            })))
        }))
        // GET /v1/backups — v0.8.0 "归一"：列出所有备份文件
        .route("/backups", get(|| async move {
            let backups = tokio::task::spawn_blocking(|| {
                crate::backup::list_backups()
            })
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "success": false,
                        "error": format!("列出备份失败: {}", e)
                    })),
                )
            })?;
            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                "success": true,
                "total": backups.len(),
                "backups": backups,
            })))
        }))
        // GET /v1/data-logs — v0.8.0 "归一"：数据操作日志
        // 返回最近 10 条数据操作记录（迁移、备份、导入等）
        .route("/data-logs", get(|| async move {
            let entries = tokio::task::spawn_blocking(|| {
                crate::data_log::read_recent_operations(10)
            })
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "success": false,
                        "error": format!("读取操作日志失败: {}", e)
                    })),
                )
            })?;
            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                "success": true,
                "total": entries.len(),
                "entries": entries,
            })))
        }))
        // v0.8.1 新增：POST /v1/config/llm/test — LLM 连接测试转发
        //
        // 由 sidecar 服务端发起对外部 LLM API 的测试请求，
        // 绕过浏览器 CSP connect-src 限制。
        // 安全说明：API Key 仅在 sidecar 进程内传输，不经过浏览器网络层。
        .route("/config/llm/test", post({
            move |Json(req): Json<LlmTestRequest>| {
                async move {
                    // 输入校验
                    if req.endpoint.trim().is_empty() || req.api_key.trim().is_empty() {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "ok": false,
                                "status": 0,
                                "message": "endpoint 和 api_key 不能为空",
                                "latency_ms": 0
                            })),
                        ));
                    }

                    // 校验 endpoint 是合法 HTTP/HTTPS URL，并拼接 /models 路径（OpenAI 兼容）
                    let test_url = format!("{}/models", req.endpoint.trim_end_matches('/'));
                    if !test_url.starts_with("https://") && !test_url.starts_with("http://") {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "ok": false,
                                "status": 0,
                                "message": "endpoint 必须以 http:// 或 https:// 开头",
                                "latency_ms": 0
                            })),
                        ));
                    }

                    let start = std::time::Instant::now();

                    // 构造 HTTP 客户端（带 10 秒超时）
                    let client = match reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(10))
                        .user_agent("loong-recall-llm-test")
                        .build()
                    {
                        Ok(c) => c,
                        Err(e) => {
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(
                                Json(serde_json::json!({
                                    "ok": false,
                                    "status": 0,
                                    "message": format!("HTTP 客户端创建失败: {}", e),
                                    "latency_ms": start.elapsed().as_millis() as u64
                                }))
                            );
                        }
                    };

                    // 发起测试请求（GET /models，OpenAI 兼容端点）
                    let resp_result = client
                        .get(&test_url)
                        .header("Authorization", format!("Bearer {}", req.api_key))
                        .send()
                        .await;

                    let latency_ms = start.elapsed().as_millis() as u64;

                    match resp_result {
                        Ok(resp) => {
                            let status = resp.status().as_u16();
                            if resp.status().is_success() {
                                Ok(Json(serde_json::json!({
                                    "ok": true,
                                    "status": status,
                                    "message": "连接成功，API Key 有效",
                                    "latency_ms": latency_ms
                                })))
                            } else {
                                let err_msg = match status {
                                    401 => "API Key 无效或已过期",
                                    403 => "无访问权限",
                                    404 => "端点不存在，请检查 endpoint 配置",
                                    429 => "请求频率超限",
                                    _ => "连接失败",
                                };
                                Ok(Json(serde_json::json!({
                                    "ok": false,
                                    "status": status,
                                    "message": err_msg,
                                    "latency_ms": latency_ms
                                })))
                            }
                        }
                        Err(e) => {
                            let err_msg = if e.is_timeout() {
                                "连接超时（10秒），请检查网络或 endpoint 可达性"
                            } else if e.is_connect() {
                                "无法连接到 endpoint，请检查 URL 是否正确"
                            } else {
                                "网络请求失败"
                            };
                            Ok(Json(serde_json::json!({
                                "ok": false,
                                "status": 0,
                                "message": format!("{}: {}", err_msg, e),
                                "latency_ms": latency_ms
                            })))
                        }
                    }
                }
            }
        }))
        // v0.8.1 新增：GET /v1/config — 获取当前 LLM 配置状态（统一前缀，与 /api/config 兼容）
        .route("/config", get({
            let llm_api = llm_api.clone();
            move || {
                let llm_api = llm_api.clone();
                async move {
                    Json(crate::server::get_llm_config_state(&llm_api).await)
                }
            }
        }))
        // v0.8.1 新增：POST /v1/config/llm — 更新 LLM API Key 配置（统一前缀，与 /api/config/llm 兼容）
        .route("/config/llm", post({
            let memory_store = store.clone();
            let llm_api = llm_api.clone();
            let llm_configured_atomic = llm_configured_atomic.clone();
            move |Json(body): Json<serde_json::Value>| {
                let memory_store = memory_store.clone();
                let llm_api = llm_api.clone();
                let llm_configured_atomic = llm_configured_atomic.clone();
                async move {
                    crate::server::update_llm_config(&memory_store, &llm_api, &llm_configured_atomic, body).await
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

    // v0.7.1 P4-1 补充：compare_versions 边界场景
    #[test]
    fn test_compare_versions_edge_cases() {
        // 空字符串应安全降级为 false（不升级）
        assert!(!compare_versions("", "0.1.0"));
        assert!(!compare_versions("0.1.0", ""));
        // 非法格式（含非数字段）应过滤后返回 false
        assert!(!compare_versions("x.y.z", "0.1.0"));
        // 多段版本号：补 0 对齐比较
        assert!(compare_versions("0.2.0.1", "0.2.0"));
        assert!(!compare_versions("0.2.0", "0.2.0.1"));
        // 大版本号跨越
        assert!(compare_versions("2.0.0", "1.9.9.9"));
    }
}

// ──────────────────────────────────────────────────────────────
// v0.7.1 P4-1：v1_api.rs 核心端点单元测试
// ──────────────────────────────────────────────────────────────
// 说明：22 个端点均以闭包形式注册到 Router，handler 未暴露为命名函数，
//       因此无法直接调用 handler 做纯单元测试。此处采用三层测试策略：
//   1. 纯函数测试（default_* 系列）
//   2. 请求体 serde 默认值测试（验证 API 契约的向后兼容性）
//   3. 响应体序列化字段名测试（确保前端可正确解析 JSON）
// 完整端点级集成测试由 server.rs 中的 axum integration tests 覆盖。
#[cfg(test)]
mod api_contracts_tests {
    use super::*;

    // ===== 1. 纯函数测试：default_* 系列确保默认值稳定 =====

    #[test]
    fn test_default_synthesis_similarity() {
        assert_eq!(default_synthesis_similarity(), 0.4);
    }

    #[test]
    fn test_default_min_cluster() {
        assert_eq!(default_min_cluster(), 3);
    }

    #[test]
    fn test_default_memory_type() {
        assert_eq!(default_memory_type(), "fact");
    }

    #[test]
    fn test_default_importance() {
        assert_eq!(default_importance(), 5);
    }

    #[test]
    fn test_default_privacy() {
        assert_eq!(default_privacy(), "user");
    }

    #[test]
    fn test_default_top_k() {
        assert_eq!(default_top_k(), 5);
    }

    #[test]
    fn test_default_min_activation() {
        assert_eq!(default_min_activation(), 0.1);
    }

    // ===== 2. 请求体 serde 默认值测试（API 契约向后兼容性） =====

    #[test]
    fn test_consolidate_request_serde_defaults() {
        // 最小请求体：仅提供 memories 字段，其余字段应使用 serde default
        let json = r#"{"memories":[{"content":"测试记忆"}]}"#;
        let req: ConsolidateRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.memories.len(), 1);
        assert_eq!(
            req.synthesis_similarity, 0.4,
            "synthesis_similarity 默认值应为 0.4"
        );
        assert_eq!(req.min_cluster, 3, "min_cluster 默认值应为 3");
        // ConsolidateMemory 的默认值
        assert_eq!(req.memories[0].memory_type, "fact");
        assert_eq!(req.memories[0].importance, 5);
        assert_eq!(req.memories[0].privacy_level, "user");
        assert!(req.memories[0].tags.is_empty());
        assert!(req.memories[0].project.is_none());
        assert!(req.memories[0].session_id.is_none());
        assert!(req.memories[0].user_id.is_none());
    }

    #[test]
    fn test_consolidate_request_explicit_values() {
        // 显式提供所有字段，确保不被默认值覆盖
        let json = r#"{
            "memories":[{"content":"显式","memory_type":"decision","importance":9,"privacy_level":"team"}],
            "synthesis_similarity":0.6,
            "min_cluster":5
        }"#;
        let req: ConsolidateRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.synthesis_similarity, 0.6);
        assert_eq!(req.min_cluster, 5);
        assert_eq!(req.memories[0].memory_type, "decision");
        assert_eq!(req.memories[0].importance, 9);
        assert_eq!(req.memories[0].privacy_level, "team");
    }

    #[test]
    fn test_enrich_request_serde_defaults() {
        let json = r#"{"query":"Rust 开发"}"#;
        let req: EnrichRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.query, "Rust 开发");
        assert_eq!(req.top_k, 5, "top_k 默认值应为 5");
        assert!(req.session_id.is_none());
        assert!(req.user_id.is_none());
    }

    #[test]
    fn test_unfold_request_serde_defaults() {
        let json = r#"{"memory_id":"mem-001"}"#;
        let req: UnfoldRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.memory_id, "mem-001");
        assert_eq!(req.min_activation, 0.1, "min_activation 默认值应为 0.1");
    }

    #[test]
    fn test_correct_request_serde_defaults() {
        // reason 字段 #[serde(default)]，缺失时应为 None
        let json = r#"{"memory_id":"mem-001","content":"修正内容"}"#;
        let req: CorrectRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.memory_id, "mem-001");
        assert_eq!(req.content, "修正内容");
        assert!(req.reason.is_none(), "reason 默认应为 None");
    }

    #[test]
    fn test_correct_request_with_reason() {
        let json = r#"{"memory_id":"mem-001","content":"修正","reason":"用户指出错误"}"#;
        let req: CorrectRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.reason, Some("用户指出错误".to_string()));
    }

    #[test]
    fn test_encode_request_required_fields() {
        // text 是必填字段，缺失应反序列化失败
        let json = r#"{}"#;
        let result: Result<EncodeRequest, _> = serde_json::from_str(json);
        assert!(result.is_err(), "缺失 text 字段应反序列化失败");
    }

    // ===== 3. 响应体序列化字段名测试（前端契约稳定性） =====

    #[test]
    fn test_encode_response_field_names() {
        let resp = EncodeResponse {
            luoshu_vector: [0.5; 9],
            bagua_index: 3,
            bagua_category: "震".to_string(),
            center_value: 0.5,
            topological_depth: 0.5,
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        // 验证字段名与前端预期一致（snake_case）
        assert!(
            json.get("luoshu_vector").is_some(),
            "字段名应为 luoshu_vector"
        );
        assert!(json.get("bagua_index").is_some());
        assert!(json.get("bagua_category").is_some());
        assert!(json.get("center_value").is_some());
        assert!(json.get("topological_depth").is_some());
        // 验证数组长度为 9
        assert_eq!(json["luoshu_vector"].as_array().unwrap().len(), 9);
    }

    #[test]
    fn test_consolidate_response_field_names() {
        let resp = ConsolidateResponse {
            stored: 3,
            synthesized: 1,
            total_memories: 4,
            synthesis_summaries: vec!["合成摘要".to_string()],
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        assert_eq!(json["stored"], 3);
        assert_eq!(json["synthesized"], 1);
        assert_eq!(json["total_memories"], 4);
        assert!(json["synthesis_summaries"].is_array());
    }

    #[test]
    fn test_dao_metrics_response_field_names() {
        // v0.8.1：契约对齐后，响应包装为 {ok, data, raw} 嵌套结构
        let resp = DaoMetricsResponse {
            ok: true,
            data: DaoMetricsData {
                yin_yang_balance: 85.5,
                luoshu_deviation: 14.5,
                bagua_balance: 97.9,
                synthesis_ratio: 30.0,
                dao_isomorphism_score: 0.855,
                active_memories: 100,
                crystallized_memories: 30,
                status: "healthy".to_string(),
            },
            raw: DaoMetricsRaw {
                bagua_entropy: 2.1,
                archived_memories: 5,
                encodings_total: 1000,
                compositions_total: 50,
                recalls_total: 200,
                corrections_total: 10,
            },
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        // 验证顶层嵌套结构（前端 loadDaoMetrics 依赖 ok/data）
        assert_eq!(json["ok"], true);
        // f32 → JSON 存在精度损失，浮点字段用容差比较
        let approx_eq = |a: f64, b: f64| (a - b).abs() < 1e-3;
        // 验证 data 字段（前端 dashboard 依赖这些名称）
        let data = &json["data"];
        assert!(approx_eq(data["yin_yang_balance"].as_f64().unwrap(), 85.5));
        assert!(approx_eq(data["luoshu_deviation"].as_f64().unwrap(), 14.5));
        assert!(approx_eq(data["bagua_balance"].as_f64().unwrap(), 97.9));
        assert!(approx_eq(data["synthesis_ratio"].as_f64().unwrap(), 30.0));
        assert!(approx_eq(
            data["dao_isomorphism_score"].as_f64().unwrap(),
            0.855
        ));
        assert_eq!(data["active_memories"], 100);
        assert_eq!(data["crystallized_memories"], 30);
        assert_eq!(data["status"], "healthy");
        // 验证 raw 字段（保留原始诊断信息）
        let raw = &json["raw"];
        assert!(approx_eq(raw["bagua_entropy"].as_f64().unwrap(), 2.1));
        assert_eq!(raw["archived_memories"], 5);
        assert_eq!(raw["encodings_total"], 1000);
        assert_eq!(raw["compositions_total"], 50);
        assert_eq!(raw["recalls_total"], 200);
        assert_eq!(raw["corrections_total"], 10);
    }

    #[test]
    fn test_unfold_response_field_names() {
        let resp = UnfoldResponse {
            success: true,
            source_memory_id: "mem-001".to_string(),
            sub_vectors_count: 3,
            fidelity: 0.95,
            sub_memories: vec![],
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        assert_eq!(json["success"], true);
        assert_eq!(json["source_memory_id"], "mem-001");
        assert_eq!(json["sub_vectors_count"], 3);
        // f32 → JSON 精度损失，用容差比较
        assert!(
            (json["fidelity"].as_f64().unwrap() - 0.95).abs() < 1e-5,
            "fidelity 字段值异常"
        );
        assert!(json["sub_memories"].is_array());
    }

    #[test]
    fn test_correct_response_field_names() {
        let resp = CorrectResponse {
            success: true,
            memory_id: "mem-001".to_string(),
            new_version: 2,
            history_versions: 1,
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        assert_eq!(json["success"], true);
        assert_eq!(json["memory_id"], "mem-001");
        assert_eq!(json["new_version"], 2);
        assert_eq!(json["history_versions"], 1);
    }

    #[test]
    fn test_enriched_memory_field_names() {
        let mem = EnrichedMemory {
            id: "mem-001".to_string(),
            content: "内容".to_string(),
            memory_type: "fact".to_string(),
            score: 0.85,
            bagua_category: Some("震".to_string()),
            importance: 7,
            topological_depth: 0.5,
            version: 1,
            created_at: "2026-07-29T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&mem).expect("序列化失败");
        assert_eq!(json["id"], "mem-001");
        assert_eq!(json["memory_type"], "fact");
        // f32 → JSON 精度损失，用容差比较
        assert!(
            (json["score"].as_f64().unwrap() - 0.85).abs() < 1e-5,
            "score 字段值异常"
        );
        assert_eq!(json["bagua_category"], "震");
        assert_eq!(json["importance"], 7);
        assert_eq!(json["version"], 1);
    }
}
