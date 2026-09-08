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

use crate::engine::audit_trail::{AuditEvent, AuditEventType, AuditQuery};
#[cfg(not(feature = "ml"))]
use crate::engine::luoshu_encoder::LuoShuEncoder as HybridLuoShuEncoder;
#[cfg(feature = "ml")]
use crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder;
use crate::engine::mirror_trapezoid::mirror_project;
use std::time::Duration;
// v0.9.1 三阶段锁解耦：consolidate handler 在锁外执行聚类计算
use crate::engine::synthesis_engine::SynthesisEngine;
use crate::engine::user_feedback::{FeedbackTarget, FeedbackType};
use crate::memory_store::{ListFilter, MemoryStore, RecallFilter};
use crate::memory_types::{Importance, Memory, MemoryType, PrivacyLevel};

type EnrichBlockingResult = (
    Vec<EnrichedMemory>,
    Vec<EnrichExplanationItem>,
    f32,
    f32,
    usize,
    usize,
    usize,
    usize,
    Vec<crate::engine::memory_state_machine::AssociationStep>,
    HashMap<String, String>,
);

struct CancellationFlag(Arc<AtomicBool>);

impl Drop for CancellationFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
use crate::persistence::json::JsonPersistence;
use crate::persistence::Persistence;
use crate::server::{safe_code_search, safe_recent_code_search, IndexedCodebase, SearchError};
use crate::{LlmApiConfig, RecallResult};
use axum::{
    extract::Query,
    http::StatusCode,
    response::Json,
    routing::{delete, get, post},
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

/// 统一 API 错误响应类型
type ApiError = (StatusCode, Json<serde_json::Value>);

/// 全局 memory_store 锁获取超时。v0.9.3 修复：任何 handler 卡死都不能无限持有
/// 全局锁阻塞所有其它请求（搜索/健康检查），超时后返回 503。
async fn lock_store_with_timeout<T>(
    store: &Arc<Mutex<T>>,
) -> Result<tokio::sync::MutexGuard<'_, T>, ApiError> {
    tokio::time::timeout(std::time::Duration::from_secs(2), store.lock())
        .await
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "store_busy",
                    "message": "记忆服务繁忙，请稍后重试"
                })),
            )
        })
}

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
    pub project: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// 联想中心探索请求：从查询或指定记忆开始，执行有界多跳扩散。
#[derive(Debug, Deserialize)]
pub struct AssociationExploreRequest {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub memory_id: Option<String>,
    #[serde(default = "default_association_depth")]
    pub depth: u8,
    #[serde(default = "default_association_width")]
    pub width: usize,
    pub project: Option<String>,
}

fn default_association_depth() -> u8 {
    4
}

fn default_association_width() -> usize {
    3
}

/// 联想确认请求（v0.9.7：用户在联想探索中点击"就是这个"）。
/// 确认后该记忆以最高激活强度写回道体状态机活跃锚点。
#[derive(Debug, Deserialize)]
pub struct ConfirmAssociationRequest {
    pub memory_id: String,
    #[serde(default)]
    pub query: Option<String>,
}

/// 联想探索响应：从起点开始的多跳联想树（有界 BFS）。
#[derive(Debug, Serialize)]
pub struct AssociationExploreResponse {
    /// 起点记忆 ID（无则从查询召回第一条）
    pub root: Option<String>,
    /// 实际执行的联想层数
    pub depth: u8,
    /// 每层方向数上限
    pub width: usize,
    /// 探索到的节点（按发现顺序）
    pub nodes: Vec<ExploreNode>,
    /// 节点间的联想边
    pub edges: Vec<ExploreEdge>,
    /// 状态机联想链（与 /v1/memories/enrich 同源）
    pub trail: Vec<crate::engine::memory_state_machine::AssociationStep>,
    /// 总共展开的召回次数（含起点）
    pub total_expanded: usize,
    /// 是否因超时/取消而提前中断
    pub interrupted: bool,
    /// 弱匹配标记（v0.9.7 精确度修复）：
    /// true 表示探索没有找到任何与查询实质共鸣的记忆（连起点都没有）——
    /// 记忆库里没有与查询真正相关的内容。前端应显示诚实空态，
    /// 而不是把无关记忆硬凑成"联想结果"。
    /// false 时 nodes 至少包含起点（起点已通过实质共鸣门禁）。
    pub weak_match: bool,
}

/// 联想探索节点
#[derive(Debug, Serialize)]
pub struct ExploreNode {
    pub id: String,
    pub content: String,
    /// 所处的联想层（起点为 0）
    pub depth: u8,
    pub score: f32,
    /// 节点来源：root（起点）/ expanded（发散）
    pub source: String,
    /// 道体再次校验·保留证据（无则为空）
    pub evidence: Option<String>,
}

/// 联想探索边
#[derive(Debug, Serialize)]
pub struct ExploreEdge {
    pub from: String,
    pub to: String,
    pub score: f32,
    /// 从父节点联想到子节点的回归证据
    pub evidence: Option<String>,
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
    /// 本次联想的状态机轨迹。
    pub trail: Vec<crate::engine::memory_state_machine::AssociationStep>,
    /// 每条结果通过道体再次校验的证据。
    pub regression_evidence: HashMap<String, String>,
    /// 被回归校验剔除的候选数量。
    pub filtered_count: usize,
    /// 联想模式，供桌面端解释当前过程。
    pub association_mode: String,
    /// 阶段D 联想解释块（只观测，不参与排序决策）
    pub explanation: EnrichExplanation,
}

/// 联想解释块：面向用户解释"为什么联想这条"（阶段D 可观测性）。
///
/// 约定：该块仅暴露观测数据（两路权重、候选规模、每条的通路贡献），
/// 不接入默认排序决策，符合"预判元数据只做观测"的产品约束。
#[derive(Debug, Serialize)]
pub struct EnrichExplanation {
    /// 触发联想的查询原文
    pub query: String,
    /// RRF 两路检索权重
    pub weights: ExplanationWeights,
    /// RRF 常数 k
    pub rrf_k: f32,
    /// 快速路径候选数
    pub fast_path_hits: usize,
    /// 深度路径候选数
    pub deep_path_hits: usize,
    /// 融合后唯一候选桶数（与响应体 total 一致）
    pub total_candidates: usize,
    /// 每条结果的通路贡献明细（与 memories 平行）
    pub items: Vec<EnrichExplanationItem>,
}

/// RRF 两路检索权重
#[derive(Debug, Serialize)]
pub struct ExplanationWeights {
    pub fast: f32,
    pub deep: f32,
}

/// 单条联想结果的通路贡献明细
#[derive(Debug, Serialize)]
pub struct EnrichExplanationItem {
    pub id: String,
    /// 融合结果中的排名（1 起）
    pub rank: usize,
    /// 展示分（与 EnrichedMemory.score 一致）
    pub score: f32,
    /// 真实融合贡献分 = fast_contrib + deep_contrib
    pub fused_contrib: f32,
    pub fast_contrib: f32,
    pub deep_contrib: f32,
    /// 在快速路径结果中的排名（未命中为 None）
    pub fast_rank: Option<usize>,
    /// 在深度路径结果中的排名（未命中为 None）
    pub deep_rank: Option<usize>,
    /// 命中的检索通路，如 ["fast"] / ["deep"] / ["fast","deep"]
    pub hit_paths: Vec<&'static str>,
}

/// 增强记忆条目
#[derive(Debug, Serialize)]
pub struct EnrichedMemory {
    pub id: String,
    pub content: String,
    pub memory_type: String,
    pub score: f32,
    pub bagua_category: Option<String>,
    pub daoti_preview_gua: Option<String>,
    pub daoti_preview_bagua: Option<String>,
    pub daoti_preview_version: Option<String>,
    pub importance: u8,
    pub topological_depth: f32,
    pub version: u32,
    pub created_at: String,
}

/// 联想探索·根节点最小实质重叠 token 数（v0.9.7 精确度门禁）。
///
/// 起点记忆必须与查询至少共享这么多个分词 token（bigram），防止只靠
/// 单个泛指词（如"什么"）重叠的无关记忆（如恰好提到"什么"的代码笔记）
/// 上位当起点。上限按查询长度收缩：短查询（如"失眠"只有 1 个 bigram）
/// 不会因凑不满 2 个 token 被误杀——门禁要求是 min(本值, 查询 token 数)。
const ASSOCIATION_ROOT_MIN_OVERLAP: usize = 2;

/// 联想探索·根节点候选池大小。根门禁要从候选中"挑"出实质共鸣的起点，
/// 池子必须比扩散宽度宽——只看 top-2/3 时，真正相关的记忆可能排在
/// 噪声之后而根本没被门禁看到。
const ASSOCIATION_ROOT_POOL_TOPK: usize = 8;

/// 联想探索·根节点语义旁路阈值（bge 完整句向量余弦，0-1）。
///
/// 词面重叠门禁挡得住"泛指词偶然命中"，但挡不住语义强相关、词面
/// 零重叠的真实召回（如查询"我以前记过什么重要日子" ↔ 记忆
/// "和爸妈去了杭州西湖"——分词后几乎无共享 token）。语义旁路仅在
/// 词面门禁无人通过时惰性启用（避免常规热路径的 ML 编码开销），
/// 用 bge 句向量余弦判断实质相关；ML 编码器不可用时相似度恒为
/// None，旁路自动失效，门禁退化为纯词面通路，绝不放宽标准。
const ASSOCIATION_ROOT_MIN_SEMANTIC_SIM: f32 = 0.55;

/// 执行一次有界联想探索。
///
/// 探索本身复用 LRC 的 recall 入口，因此每一跳都会经过内置道体状态机、
/// 联想导航和回归校验，而不是在 API 层另造一套排序逻辑。
fn run_association_explore(
    store: &mut MemoryStore<JsonPersistence>,
    query: Option<&str>,
    memory_id: Option<&str>,
    max_depth: u8,
    width: usize,
    cancel: &AtomicBool,
) -> AssociationExploreResponse {
    use std::collections::{HashSet, VecDeque};

    let max_depth = max_depth.clamp(1, 4);
    let width = width.clamp(1, 3);
    // v0.9.7 精确度修复：探索是用户主动发起的联想，语义必须由查询本身主导。
    // explore_pure 关闭联想导航的查询扩展与活性偏置，避免"近期活跃记忆"
    //（可能是某个领域的旧内容）把任意查询的结果牵引到固定的一批记忆上。
    let mut filter = RecallFilter {
        memory_type: None,
        project: None,
        tags: Vec::new(),
        min_importance: None,
        top_k: width,
        privacy_context: None,
        explore_pure: true,
        regression_query: None,
    };
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    let mut total_expanded = 0usize;
    let mut interrupted = false;
    // v0.9.7 韧性修复：探索耗时预算从函数入口起算——root 召回（含语义
    // 旁路的并行编码）与 BFS 扩散共享同一预算，确保任何路径下探索都
    // 在外层 15s 超时之前优雅收敛，用户看到部分结果或诚实空态而非 503。
    const ASSOCIATION_EXPLORE_TIME_BUDGET: Duration = Duration::from_secs(10);
    const ASSOCIATION_SEMANTIC_BYPASS_BUDGET: Duration = Duration::from_secs(6);
    let explore_started = std::time::Instant::now();

    let root_memory = if let Some(id) = memory_id {
        store
            .list_memories(&ListFilter {
                limit: usize::MAX,
                ..ListFilter::new()
            })
            .ok()
            .and_then(|(all, _)| all.into_iter().find(|memory| memory.id == id))
            .map(|memory| (memory, 1.0f32, "root".to_string(), None))
    } else {
        query.and_then(|text| {
            // v0.9.7 精确度修复（根节点主导性门禁）：起点是整条联想链
            // 的锚，"词面命中过一次"不够——含泛指词（如"什么"）的查询
            // 会让恰好提到该词的无关记忆靠单 token 重叠上位。起点必须
            // 与查询实质共鸣：至少命中 min(ASSOCIATION_ROOT_MIN_OVERLAP,
            // 查询 token 数) 个 token；没有合格候选则诚实标记弱匹配，
            // 绝不硬凑一个错误起点把整条联想链带偏。
            //
            // 门禁是"从候选池里挑"，池子必须够宽（ROOT_POOL_TOPK）：
            // 只看 top-2/3 时，真正相关的记忆可能排在噪声之后没被看到。
            let mut root_filter = filter.clone();
            root_filter.top_k = ASSOCIATION_ROOT_POOL_TOPK;
            let result = store.recall(text, &root_filter).ok()?;
            let query_tokens = crate::memory_store::tokenize_query(text);
            // v0.9.7 泛指 bigram 过滤：「是什/什么/怎么」等问句功能组合
            // 不计入实质共鸣——"量子物理是什么"曾因「是什」「什么」两个
            // 泛指 bigram 与含测试文本"是什么"的代码 chunk 虚假共鸣，
            // 让代码 chunk 抢走生活查询的起点，进而把整条联想链拖进
            // 代码 chunk 邻域造成 15s 扩散超时。过滤后门禁只看实义
            // token（量子/物理/今晚/吃什 等）的重叠。
            let substantive_tokens: Vec<String> = query_tokens
                .iter()
                .filter(|t| !crate::memory_store::is_generic_bigram(t))
                .cloned()
                .collect();
            let min_required = substantive_tokens
                .len()
                .clamp(1, ASSOCIATION_ROOT_MIN_OVERLAP);
            // 双通路门禁（先词面后语义，惰性）：词面通路要求实质重叠
            // ≥ min_required 个 token；无人通过时才启用语义旁路——用
            // bge 句向量余弦（阈值 ASSOCIATION_ROOT_MIN_SEMANTIC_SIM）
            // 挽救"重要日子 ↔ 结婚纪念日"类语义强相关但词面零重叠的
            // 真实召回。ML 未加载时旁路自动失效，诚实弱匹配。
            let candidates: Vec<(crate::memory_types::Memory, f32)> =
                result.memories.into_iter().zip(result.scores).collect();
            let mut passing: Vec<(crate::memory_types::Memory, f32)> = candidates
                .iter()
                .filter(|(memory, _)| {
                    store.query_overlap_count(memory, &substantive_tokens) >= min_required
                })
                .cloned()
                .collect();
            if passing.is_empty() && explore_started.elapsed() < ASSOCIATION_SEMANTIC_BYPASS_BUDGET
            {
                // 旁路硬时限：并行编码仍超预算时放弃旁路（诚实空态），
                // 不允许 root 阶段吃掉 BFS 扩散的全部预算
                let mem_refs: Vec<&crate::memory_types::Memory> =
                    candidates.iter().map(|(m, _)| m).collect();
                let sims = store.semantic_similarities(text, &mem_refs);
                for ((memory, score), sim) in candidates.iter().zip(sims) {
                    // v0.9.7 精确度修复：语义旁路不放行代码记忆。旁路只在
                    // 词面零命中时触发，此时若救回的是代码 chunk，几乎都
                    // 是 bge 对长文本的向量居中假象（"量子物理是什么"曾
                    // 因此被 memory_store.rs 的代码块当起点，整条联想链
                    // 全是代码）。代码查询词面命中率高（标识符/函数名
                    // 天然是实义 token），不需要旁路。
                    if memory.memory_type == crate::memory_types::MemoryType::CodeContext {
                        continue;
                    }
                    if sim.is_some_and(|s| s >= ASSOCIATION_ROOT_MIN_SEMANTIC_SIM) {
                        passing.push((memory.clone(), *score));
                    }
                }
            }
            // 开发上下文块（6000+ 条代码 chunk 在语义空间里无处不在）
            // 只在没有更贴合的生活记忆通过门禁时才允许充当起点，防止
            // 代码记忆抢走生活查询的锚。代码查询下所有通过门禁的候选
            // 都是代码块时，照常以代码为起点，联想链保持在开发语境。
            let picked = passing
                .iter()
                .find(|(memory, _)| {
                    memory.memory_type != crate::memory_types::MemoryType::CodeContext
                })
                .or_else(|| passing.first())
                .cloned()?;
            let (memory, score) = picked;
            let evidence = result.regression_evidence.get(&memory.id).cloned();
            Some((memory, score, "root".to_string(), evidence))
        })
    };

    let Some((root_memory, root_score, source, evidence)) = root_memory else {
        return AssociationExploreResponse {
            root: None,
            depth: max_depth,
            width,
            nodes,
            edges,
            trail: store.memory_state_machine.snapshot().trail,
            total_expanded,
            interrupted,
            // 连起点都召不回（召回为空，或没有候选通过根节点实质共鸣门禁），
            // 必然是弱匹配：记忆库里没有与查询实质相关的内容
            weak_match: true,
        };
    };

    let root_id = root_memory.id.clone();
    let root: Option<String> = Some(root_id.clone());
    visited.insert(root_id.clone());
    nodes.push(ExploreNode {
        id: root_id.clone(),
        content: root_memory.content.clone(),
        depth: 0,
        score: root_score,
        source,
        evidence,
    });
    // v0.9.7 精确度修复：多跳扩散的回归校验锚定到起点记忆主题。
    // 每一跳以父记忆内容为查询逐层扩散，但校验一律对齐起点——
    // 保证展示给用户的每个节点都"收束回联想主题"，杜绝深层
    // 语义漂移（食物 → 代码噪声）。
    filter.regression_query = Some(root_memory.content.clone());
    // 非代码起点的联想链不混入开发上下文块：6000+ 代码 chunk 在
    // 语义空间里无处不在，会把"周末去哪儿玩"的发散拉向 src/
    // benchmark 之类的实现细节。代码起点不受影响——开发查询的
    // 联想链本来就应该是代码。
    let root_is_code = root_memory.memory_type == crate::memory_types::MemoryType::CodeContext;
    // BFS 扩散与 root 召回共享 explore_started 预算（见函数入口），
    // 超预算即优雅收敛：返回已找到的部分 + interrupted=true。
    queue.push_back((root_id, root_memory.content, 0u8));

    while let Some((from_id, from_content, current_depth)) = queue.pop_front() {
        if current_depth >= max_depth {
            continue;
        }
        if cancel.load(Ordering::Acquire) {
            interrupted = true;
            break;
        }
        if explore_started.elapsed() >= ASSOCIATION_EXPLORE_TIME_BUDGET {
            interrupted = true;
            break;
        }
        total_expanded += 1;
        let result = match store.recall(
            &from_content,
            &RecallFilter {
                top_k: width.saturating_add(2),
                ..filter.clone()
            },
        ) {
            Ok(result) => result,
            Err(_) => continue,
        };
        let next_depth = current_depth + 1;
        let mut added = 0usize;
        for (memory, score) in result.memories.into_iter().zip(result.scores) {
            if !root_is_code && memory.memory_type == crate::memory_types::MemoryType::CodeContext {
                continue;
            }
            if added >= width || visited.contains(&memory.id) {
                continue;
            }
            let evidence = result.regression_evidence.get(&memory.id).cloned();
            let child_id = memory.id.clone();
            visited.insert(child_id.clone());
            edges.push(ExploreEdge {
                from: from_id.clone(),
                to: child_id.clone(),
                score,
                evidence: evidence.clone(),
            });
            nodes.push(ExploreNode {
                id: child_id.clone(),
                content: memory.content.clone(),
                depth: next_depth,
                score,
                source: "expanded".to_string(),
                evidence,
            });
            queue.push_back((child_id, memory.content, next_depth));
            added += 1;
        }
    }

    // v0.9.7 精确度修复：弱匹配 = 没有任何与查询实质相关的内容可展示。
    // 起点都没找到（nodes 为空）时必然弱匹配，前端显示诚实空态；
    // 找到起点时，起点本身就是实质共鸣的相关内容——"没有更多发散"
    // 不算弱匹配，照常展示起点与确认按钮，绝不把有效结果整个隐藏。
    let weak_match = nodes.is_empty();

    AssociationExploreResponse {
        root,
        depth: max_depth,
        width,
        nodes,
        edges,
        trail: store.memory_state_machine.snapshot().trail,
        total_expanded,
        interrupted,
        weak_match,
    }
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
///   - bagua_balance: 八卦均衡度（0-100），派生自 (bagua_entropy / 3.0) * 100（熵越大越均匀）
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
    /// P1 调节器心跳：最近一次 regulate() 的执行状态（None 表示尚未触发）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regulator_heartbeat: Option<crate::memory_store::RegulatorHeartbeat>,
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
    /// 按项目过滤
    pub project: Option<String>,
    /// 按标签过滤（任一标签匹配）
    #[serde(default)]
    pub tags: Vec<String>,
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
    /// 关联项目名称
    pub project: Option<String>,
    /// 按标签过滤
    #[serde(default)]
    pub tags: Vec<String>,
}

/// /v1/memories/forget 请求体
///
/// 前端删除单条记忆时调用，memory_id 必填。
#[derive(Debug, Clone, Deserialize)]
pub struct ForgetRequest {
    /// 待删除的记忆 ID（必填）
    pub memory_id: String,
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

/// 构建结晶历史时间线（v0.9.6 修复：数据源从审计事件改为合成记忆本身）。
///
/// 结晶的持久化产物就是 Synthesis 类型记忆，其 created_at 即结晶完成时间。
/// 原实现从审计事件（synthesis_created）提取，但历史合成未落审计事件
/// （如批量导入/早期版本），导致时间线永远显示"暂无结晶记录"。
/// 修复：直接过滤 Synthesis 记忆并按创建时间倒序返回，即"每次结晶被持久化记录"。
/// 抽为纯函数以便单元测试覆盖字段名与排序逻辑。
fn build_synthesis_timeline(memories: &[Memory], limit: usize) -> serde_json::Value {
    let limit = limit.clamp(1, 50);
    let mut sorted: Vec<&Memory> = memories
        .iter()
        .filter(|m| m.memory_type == MemoryType::Synthesis)
        .collect();
    // 防御性排序：不依赖调用方传入顺序，保证时间线倒序契约
    sorted.sort_by_key(|m| std::cmp::Reverse(m.created_at));
    let items: Vec<serde_json::Value> = sorted
        .iter()
        .take(limit)
        .map(|m| {
            // v0.9.6 P1-1：补充来源记忆数/置信度/信息增量，支撑"结晶成长链路"展示
            serde_json::json!({
                "id": m.id,
                "content": m.content,
                "memory_type": m.memory_type.as_str(),
                "project": m.project,
                "created_at_ms": m.created_at.timestamp_millis(),
                "importance": m.importance.value(),
                "source_count": m.source_ids.len(),
                "confidence": m.confidence,
                "information_gain": m.information_gain,
            })
        })
        .collect();
    serde_json::json!({
        "items": items,
        "total": sorted.len(),
    })
}

/// 聚合 RetrievalExecuted 审计事件为联想执行活动指标（v0.9.6 首屏仪表盘）。
///
/// 抽为纯函数以便单元测试覆盖字段名与计算逻辑，防止前后端契约漂移
/// （与 audit-trail/trust 端点字段名错误同源的历史教训）。
/// 输入为 `audit_trail.query` 返回的引用切片（最新在前）。
fn aggregate_association_activity(events: &[&AuditEvent]) -> serde_json::Value {
    let mut fast_only = 0u64;
    let mut deep_only = 0u64;
    let mut both = 0u64;
    let mut cand_sum: u64 = 0;
    let mut cand_n: u64 = 0;
    let mut last_ms: Option<u64> = None;

    for ev in events.iter() {
        let md = &ev.metadata;
        let fh = md.get("fast_hits").and_then(|v| v.parse::<u64>().ok());
        let dh = md.get("deep_hits").and_then(|v| v.parse::<u64>().ok());
        if let (Some(f), Some(d)) = (fh, dh) {
            if f > 0 && d > 0 {
                both += 1;
            } else if f > 0 {
                fast_only += 1;
            } else if d > 0 {
                deep_only += 1;
            }
        }
        if let Some(c) = md
            .get("total_candidates")
            .and_then(|v| v.parse::<u64>().ok())
        {
            cand_sum += c;
            cand_n += 1;
        }
        if ev.timestamp_ms > last_ms.unwrap_or(0) {
            last_ms = Some(ev.timestamp_ms);
        }
    }

    // 最近 10 次执行明细（audit_trail 事件为最新在前，直接 take）
    let recent: Vec<serde_json::Value> = events
        .iter()
        .take(10)
        .map(|ev| {
            let md = &ev.metadata;
            serde_json::json!({
                "timestamp_ms": ev.timestamp_ms,
                "fast_hits": md.get("fast_hits").and_then(|v| v.parse::<u64>().ok()),
                "deep_hits": md.get("deep_hits").and_then(|v| v.parse::<u64>().ok()),
                "total_candidates": md.get("total_candidates").and_then(|v| v.parse::<u64>().ok()),
                "query_length": md.get("query_length").and_then(|v| v.parse::<u64>().ok()),
            })
        })
        .collect();

    let avg_candidates = if cand_n > 0 {
        cand_sum as f64 / cand_n as f64
    } else {
        0.0
    };

    serde_json::json!({
        "total_executions": events.len(),
        "avg_candidates": avg_candidates,
        "fast_only_count": fast_only,
        "deep_only_count": deep_only,
        "both_count": both,
        "last_execution_ms": last_ms,
        "recent": recent,
    })
}

/// 创建 v1 REST API 路由（状态类型为 ()，以便与主路由合并）
///
/// 通过闭包捕获 memory_store 和 codebase_manager，无需使用 axum State。
pub fn build_v1_router(
    store: SharedStore,
    codebase_manager: Arc<Mutex<Box<dyn IndexedCodebase>>>,
    llm_api: Arc<RwLock<LlmApiConfig>>,
    // v0.8.22 P0-1 修复：传入 LLM 配置状态的无锁缓存，便于 /v1/config/llm 更新时同步
    llm_configured_atomic: Arc<std::sync::atomic::AtomicBool>,
    // v0.9.0: 开发模式标志，注入到 /health/system 响应中
    dev_mode: bool,
) -> Router {
    let consolidate_store = store.clone();
    let enrich_store = store.clone();
    let correct_store = store.clone();
    let explore_store = store.clone();
    let confirm_store = store.clone();
    let metrics_store = store.clone();
    let unfold_store = store.clone();
    let regulator_store = store.clone();
    // v0.9.7 审查修复：备份恢复需持 store 锁防止并发写竞态，并失效缓存
    let restore_store = store.clone();

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

                    // Phase 1：持锁写入记忆。
                    // v0.9.6 P1 修复：整体移入 spawn_blocking，避免守卫自旋占死 worker。
                    let mem_inputs = req.memories;
                    let phase1_store = store.clone();
                    let stored = tokio::task::spawn_blocking(move || -> Option<usize> {
                        // 有界 try_lock 轮询（2s），与 remember 一致：
                        // 在阻塞线程中自旋，不饥饿 HTTP worker，也不无限占用阻塞线程
                        let lock_deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(2);
                        let mut store = loop {
                            match phase1_store.try_lock() {
                                Ok(guard) => break guard,
                                Err(_) if std::time::Instant::now() < lock_deadline => {
                                    std::thread::sleep(std::time::Duration::from_millis(10));
                                }
                                Err(_) => return None,
                            }
                        };
                        let mut stored = 0usize;
                        for mem in &mem_inputs {
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
                        Some(stored)
                    })
                    .await
                    .unwrap_or(None)
                    .ok_or((
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(serde_json::json!({
                            "error": "store_busy",
                            "message": "记忆服务繁忙，请稍后重试"
                        })),
                    ))?;

                    // Phase 2：异步超时获取快照，锁外执行 CPU 聚类计算。
                    let snapshot = {
                        let store = lock_store_with_timeout(&store).await?;
                        store
                            .synthesis_snapshot()
                            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                                "error": "consolidation_failed",
                                "message": format!("合成快照失败: {}", e)
                            }))))?
                    };
                    let synthesized_plan = tokio::task::spawn_blocking(move || {
                        let engine = SynthesisEngine::new(snapshot.config);
                        let mut plan =
                            engine.plan_luoshu(&snapshot.all, snapshot.information_gain_threshold);
                        if plan.synthesized == 0 {
                            plan = engine.plan_jaccard(&snapshot.all);
                        }
                        plan
                    }).await;
                    let synthesized = match synthesized_plan {
                        Ok(plan) => {
                            let mut store = lock_store_with_timeout(&store).await?;
                            store.apply_synthesis_plan(plan)
                        }
                        Err(e) => {
                            eprintln!("[v1/consolidate] spawn_blocking panic: {}", e);
                            return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                                "error": "consolidation_internal_error",
                                "message": "合成任务执行失败，服务已保持运行"
                            }))));
                        }
                    };

                    // Phase 3：重新持锁，列出记忆和获取总数（快速操作）
                    let (synthesis_summaries, total) = {
                        let store = lock_store_with_timeout(&store).await?;
                        let filter = ListFilter::new();
                        let all_memories = store.list_memories(&filter).map_err(|e| {
                            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                                "error": "list_memories_failed",
                                "message": format!("列出记忆失败: {}", e)
                            })))
                        })?;
                        let synthesis_summaries: Vec<String> = all_memories.0
                            .iter()
                            .filter(|m| m.memory_type == MemoryType::Synthesis)
                            .map(|m| m.summary())
                            .collect();

                        let total = store.total_count().map_err(|e| {
                            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                                "error": "total_count_failed",
                                "message": format!("获取记忆总数失败: {}", e)
                            })))
                        })?;
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
        // POST /v1/memories/enrich — 双路检索增强（v0.9.3 修复：增加超时与 panic 隔离）
        // v0.9.3 修复：锁获取 + CPU 检索统一放入 spawn_blocking（blocking_lock），
        //       外层 15s timeout 兜底（覆盖锁等待与检索执行两个阶段）。
        // 根因：recall() / trapezoid_focus_recall() 在 tokio 异步上下文中同步阻塞，
        //       无超时保护时若检索卡死，锁永不释放，后续请求全部超时
        .route("/memories/enrich", post({
            let store = enrich_store;
            move |Json(req): Json<EnrichRequest>| {
                let store = store.clone();
                async move {
                    let query = req.query;
                    // 联想解释块需要保留查询原文（query 将被 move 进 spawn_blocking 闭包）
                    let query_text = query.clone();
                    let top_k = req.top_k.clamp(1, 100);
                    let privacy_ctx = if req.user_id.is_some() {
                        Some((PrivacyLevel::User, req.session_id.clone(), req.user_id.clone()))
                    } else {
                        None
                    };

                    // Phase 1+2：锁获取与 CPU 密集检索统一放入 spawn_blocking（阻塞上下文）。
                    // 锁获取采用"有界 try_lock 轮询"（2 秒截止）——若锁被长期占用，
                    // 阻塞线程自行放弃并返回 search_busy，不会无限阻塞线程池。
                    // 外部 15s timeout 对"检索执行"阶段兜底（请求等待超时）。
                    let cancellation = Arc::new(AtomicBool::new(false));
                    let cancellation_for_task = cancellation.clone();
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        tokio::task::spawn_blocking(move || -> Result<EnrichBlockingResult, &'static str> {
                            let _cancellation_guard = CancellationFlag(cancellation_for_task.clone());
                            // 有界锁获取：轮询 try_lock，2 秒未获得则放弃
                            let lock_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                            let mut store = loop {
                                match store.try_lock() {
                                    Ok(guard) => break guard,
                                    Err(_) if std::time::Instant::now() < lock_deadline => {
                                        if cancellation_for_task.load(Ordering::Acquire) {
                                            return Err("cancelled");
                                        }
                                        std::thread::sleep(std::time::Duration::from_millis(10));
                                    }
                                    Err(_) => {
                                        eprintln!("[v1/enrich] 锁获取超时（2s），返回 search_busy");
                                        return Err("search_busy");
                                    }
                                }
                            };

                            let fast_filter = RecallFilter {
                                memory_type: None,
                                project: req.project.clone(),
                                tags: req.tags.clone(),
                                min_importance: None,
                                top_k: top_k * 2,
                                privacy_context: privacy_ctx.clone(),
                                explore_pure: false,
                                regression_query: None,
                            };
                            let mut internal_error = false;
                            let fast_result = match store.recall_with_cancel(
                                &query,
                                &fast_filter,
                                Some(&cancellation_for_task),
                            ) {
                                Ok(result) => result,
                                Err(e) if e.to_string() == "enrich_cancelled" => {
                                    return Err("cancelled");
                                }
                                Err(_e) => {
                                    eprintln!("[v1/enrich] 快速路径检索失败，已降级为空结果");
                                    internal_error = true;
                                    RecallResult::basic(vec![], vec![], 0)
                                }
                            };

                            let deep_filter = RecallFilter {
                                memory_type: None,
                                project: req.project.clone(),
                                tags: req.tags.clone(),
                                min_importance: None,
                                top_k: top_k * 2,
                                privacy_context: privacy_ctx,
                                explore_pure: false,
                                regression_query: None,
                            };
                            // 深度路径始终执行：查询意图权重会在 RRF 阶段抑制泛化词污染，
                            // 同时保留无 ML 模式下的真实结果，便于评估与后续升级编码器。
                            let deep_result = match store.trapezoid_focus_recall_with_cancel(
                                &query,
                                &deep_filter,
                                1,
                                Some(&cancellation_for_task),
                            ) {
                                Ok(result) => result,
                                Err(e) if e.to_string() == "enrich_cancelled" => {
                                    return Err("cancelled");
                                }
                                Err(_e) => {
                                    eprintln!("[v1/enrich] 深度路径检索失败，已降级为空结果");
                                    internal_error = true;
                                    RecallResult::basic(vec![], vec![], 0)
                                }
                            };

                            // RRF 融合：具体错误查询提高 Deep 路径权重。
                            let (fast_weight, deep_weight) =
                                crate::engine::rrf::query_path_weights(&query);
                            let fused = crate::engine::rrf::rrf_fuse_weighted(
                                &fast_result,
                                &deep_result,
                                top_k,
                                crate::engine::rrf::RRF_DEFAULT_K,
                                fast_weight,
                                deep_weight,
                            );

                            // 阶段D 可观测性：LRC_RRF_TRACE=1 输出融合审计行（默认关闭）
                            if std::env::var("LRC_RRF_TRACE").map(|v| v == "1").unwrap_or(false) {
                                let ft = fast_result.memories.first();
                                let dt = deep_result.memories.first();
                                let mt = fused.memories.first();
                                eprintln!("[LRC-RRF-TRACE] {}", serde_json::json!({
                                    "query_length": query.chars().count(),
                                    "weights": { "fast": fast_weight, "deep": deep_weight },
                                    "fast_top1": ft.map(|m| serde_json::json!({
                                        "source": m.source, "project": m.project,
                                        "bagua": m.bagua_index, "score": fast_result.scores.first().copied().unwrap_or(0.0),
                                        "preview": m.content.chars().take(60).collect::<String>(),
                                    })),
                                    "deep_top1": dt.map(|m| serde_json::json!({
                                        "source": m.source, "project": m.project,
                                        "bagua": m.bagua_index, "score": deep_result.scores.first().copied().unwrap_or(0.0),
                                        "preview": m.content.chars().take(60).collect::<String>(),
                                    })),
                                    "fused_top1": mt.map(|m| serde_json::json!({
                                        "source": m.source, "project": m.project,
                                        "bagua": m.bagua_index, "score": fused.scores.first().copied().unwrap_or(0.0),
                                        "preview": m.content.chars().take(60).collect::<String>(),
                                    })),
                                }));
                            }

                            if internal_error && fused.memories.is_empty() {
                                return Err("search_internal_error");
                            }
                            let total = fused.total_candidates;
                            let fast_hits = fast_result.memories.len();
                            let deep_hits = deep_result.memories.len();
                            let mut memories: Vec<EnrichedMemory> = Vec::with_capacity(fused.memories.len());
                            let mut explanation_items: Vec<EnrichExplanationItem> =
                                Vec::with_capacity(fused.memories.len());
                            for (i, (m, (&score, contrib))) in fused
                                .memories
                                .iter()
                                .zip(fused.scores.iter().zip(fused.contributions.iter()))
                                .enumerate()
                            {
                                memories.push(EnrichedMemory {
                                    id: m.id.clone(),
                                    content: m.content.clone(),
                                    memory_type: m.memory_type.as_str().to_string(),
                                    score,
                                    bagua_category: m.bagua_category.clone(),
                                    daoti_preview_gua: m.daoti_preview_gua.clone(),
                                    daoti_preview_bagua: m.daoti_preview_bagua.clone(),
                                    daoti_preview_version: m.daoti_preview_version.clone(),
                                    importance: m.importance.value(),
                                    topological_depth: m.topological_depth,
                                    version: m.version,
                                    created_at: m.created_at.to_rfc3339(),
                                });
                                // 阶段D：每条的路径贡献明细（只观测，不参与排序）
                                explanation_items.push(EnrichExplanationItem {
                                    id: m.id.clone(),
                                    rank: i + 1,
                                    score,
                                    fused_contrib: contrib.fused_contrib(),
                                    fast_contrib: contrib.fast_contrib,
                                    deep_contrib: contrib.deep_contrib,
                                    fast_rank: contrib.fast_rank,
                                    deep_rank: contrib.deep_rank,
                                    hit_paths: contrib.hit_paths(),
                                });
                            }

                            // 阶段D：检索链路结构化审计（只观测，不参与排序决策）
                            // 记录两路权重、候选规模与每条结果的通路贡献分解，
                            // 供 /v1/audit-trail 查询与后续反馈回流观测使用。
                            let mut audit_meta = std::collections::HashMap::new();
                            audit_meta.insert(
                                "query_length".to_string(),
                                query.chars().count().to_string(),
                            );
                            audit_meta.insert("fast_weight".to_string(), fast_weight.to_string());
                            audit_meta.insert("deep_weight".to_string(), deep_weight.to_string());
                            audit_meta.insert("fast_hits".to_string(), fast_hits.to_string());
                            audit_meta.insert("deep_hits".to_string(), deep_hits.to_string());
                            audit_meta.insert("total_candidates".to_string(), total.to_string());
                            let items_json = serde_json::json!(explanation_items
                                .iter()
                                .map(|item| {
                                    serde_json::json!({
                                        "rank": item.rank,
                                        "id": item.id,
                                        "fused_contrib": item.fused_contrib,
                                        "fast_contrib": item.fast_contrib,
                                        "deep_contrib": item.deep_contrib,
                                        "fast_rank": item.fast_rank,
                                        "deep_rank": item.deep_rank,
                                        "hit_paths": item.hit_paths,
                                    })
                                })
                                .collect::<Vec<_>>());
                            audit_meta.insert("items".to_string(), items_json.to_string());
                            let state_snapshot = store.memory_state_machine.snapshot();
                            let mut regression_evidence = fast_result.regression_evidence.clone();
                            for (id, evidence) in &deep_result.regression_evidence {
                                regression_evidence.insert(id.clone(), evidence.clone());
                            }
                            let filtered_count = fast_hits
                                .saturating_add(deep_hits)
                                .saturating_sub(total);
                            let affected_ids =
                                explanation_items.iter().map(|item| item.id.clone()).collect();
                            let evidence_json = serde_json::to_string(&regression_evidence)
                                .unwrap_or_else(|_| "{}".to_string());
                            audit_meta.insert("filtered_count".to_string(), filtered_count.to_string());
                            audit_meta.insert("regression_evidence".to_string(), evidence_json);
                            audit_meta.insert(
                                "trail".to_string(),
                                serde_json::to_string(&state_snapshot.trail)
                                    .unwrap_or_else(|_| "[]".to_string()),
                            );
                            store.audit_trail.record(
                                AuditEventType::RetrievalExecuted,
                                format!(
                                    "联想检索执行：查询「{}」，融合 {} 条候选（fast={}，deep={}）",
                                    query.chars().take(40).collect::<String>(),
                                    total,
                                    fast_hits,
                                    deep_hits
                                ),
                                "阶段D 联想解释观测：记录检索链路权重与每条结果的通路贡献分解，供结构化审计，不参与排序".to_string(),
                                affected_ids,
                                audit_meta,
                            );

                            Ok((
                                memories,
                                explanation_items,
                                fast_weight,
                                deep_weight,
                                fast_hits,
                                deep_hits,
                                total,
                                filtered_count,
                                state_snapshot.trail,
                                regression_evidence,
                            ))
                        })
                    ).await;

                    match result {
                        Ok(Ok(Ok((memories, explanation_items, fast_weight, deep_weight, fast_hits, deep_hits, total, filtered_count, trail, regression_evidence)))) => {
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(EnrichResponse {
                                memories,
                                fast_path_hits: fast_hits,
                                deep_path_hits: deep_hits,
                                total,
                                trail,
                                regression_evidence,
                                filtered_count,
                                association_mode: "state_machine_navigation".to_string(),
                                explanation: EnrichExplanation {
                                    query: query_text,
                                    weights: ExplanationWeights {
                                        fast: fast_weight,
                                        deep: deep_weight,
                                    },
                                    rrf_k: crate::engine::rrf::RRF_DEFAULT_K,
                                    fast_path_hits: fast_hits,
                                    deep_path_hits: deep_hits,
                                    total_candidates: total,
                                    items: explanation_items,
                                },
                            }))
                        }
                        Ok(Ok(Err("search_internal_error"))) => {
                            Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                                "error": "search_internal_error",
                                "message": "搜索内部错误，请稍后重试"
                            }))))
                        }
                        Ok(Ok(Err("cancelled"))) => {
                            eprintln!("[v1/enrich] 检索任务已取消，后台 blocking 任务已退出");
                            Err((StatusCode::REQUEST_TIMEOUT, Json(serde_json::json!({
                                "error": "search_cancelled",
                                "message": "搜索已取消"
                            }))))
                        }
                        Ok(Ok(Err("search_busy"))) => {
                            // 有界锁获取超时（2s）：返回与既有 lock_busy 一致的口径
                            Err((StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                                "error": "search_busy",
                                "message": "搜索服务繁忙，请稍后重试"
                            }))))
                        }
                        Ok(Err(join_error)) => {
                            // spawn_blocking 内部 panic 被捕获
                            eprintln!("[v1/enrich] spawn_blocking 内部 panic: {}", join_error);
                            Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                                "error": "search_internal_error",
                                "message": "搜索内部错误，服务已保持运行"
                            }))))
                        }
                        Err(_timeout) => {
                            // 15 秒超时触发
                            // v0.9.7 审查修复（HCSE-P1）：CancellationFlag 的 Drop
                            // 只在闭包结束时触发，超时分支若不显式置位，失控任务会
                            // 继续持锁跑完整轮检索，让后续请求在窗口期内全部 busy。
                            // 置位后任务会在下一个 token 检查点退出并释放锁。
                            cancellation.store(true, Ordering::Release);
                            eprintln!("[v1/enrich] 搜索超时（15s），已置取消标志令后台任务提前退出");
                            Err((StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                                "error": "search_timeout",
                                "message": "搜索超时，请稍后重试"
                            }))))
                        }
                        Ok(Ok(Err(other))) => {
                            // 兜底：未知锁错误分支（当前仅 search_busy 一种）
                            eprintln!("[v1/enrich] 未知检索错误: {}", other);
                            Err((StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                                "error": "search_unavailable",
                                "message": "搜索暂不可用，请稍后重试"
                            }))))
                        }
                    }
                }
            }
        }))
        // POST /v1/associations/explore — 联想中心多跳探索（有界 BFS）
        .route("/associations/explore", post({
            let store = explore_store;
            move |Json(req): Json<AssociationExploreRequest>| {
                let store = store.clone();
                async move {
                    let depth = req.depth.clamp(1, 4);
                    let width = req.width.clamp(1, 3);
                    if req.query.as_deref().map(|q| q.trim()).unwrap_or("").is_empty()
                        && req.memory_id.as_deref().map(|id| id.trim()).unwrap_or("").is_empty()
                    {
                        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
                            "error": "missing_start",
                            "message": "必须提供 query 或 memory_id 作为联想起点"
                        }))));
                    }
                    let cancellation = Arc::new(AtomicBool::new(false));
                    let cancellation_for_task = cancellation.clone();
                    let query = req.query.clone();
                    let memory_id = req.memory_id.clone();
                    let result = tokio::time::timeout(
                        Duration::from_secs(15),
                        tokio::task::spawn_blocking(move || -> Result<AssociationExploreResponse, &'static str> {
                            let _guard = CancellationFlag(cancellation_for_task.clone());
                            let mut store = loop {
                                match store.try_lock() {
                                    Ok(guard) => break guard,
                                    Err(_) => {
                                        if cancellation_for_task.load(Ordering::Acquire) {
                                            return Err("search_busy");
                                        }
                                        std::thread::sleep(Duration::from_millis(50));
                                    }
                                }
                            };
                            Ok(run_association_explore(
                                &mut store,
                                query.as_deref().and_then(|q| {
                                    let t = q.trim();
                                    if t.is_empty() { None } else { Some(t) }
                                }),
                                memory_id.as_deref().and_then(|id| {
                                    let t = id.trim();
                                    if t.is_empty() { None } else { Some(t) }
                                }),
                                depth,
                                width,
                                &cancellation_for_task,
                            ))
                        }),
                    ).await;

                    match result {
                        Ok(Ok(Ok(response))) => Ok(Json(response)),
                        Ok(Ok(Err(_))) => Err((StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                            "error": "search_busy",
                            "message": "记忆系统正在执行后台任务，请稍后重试"
                        })))),
                        Ok(Err(join_error)) => {
                            eprintln!("[v1/associations/explore] spawn_blocking panic: {}", join_error);
                            Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                                "error": "explore_internal_error",
                                "message": "探索内部错误，服务已保持运行"
                            }))))
                        }
                        Err(_timeout) => {
                            cancellation.store(true, Ordering::Release);
                            eprintln!("[v1/associations/explore] 探索超时（15s），已置取消标志");
                            Err((StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                                "error": "explore_timeout",
                                "message": "联想探索超时，请降低联想层数后重试"
                            }))))
                        }
                    }
                }
            }
        }))
        // POST /v1/associations/confirm — 用户确认一条联想（"就是这个"）。
        // 确认 = 该记忆以最高激活强度写回道体状态机活跃锚点并持久化。
        .route("/associations/confirm", post({
            let store = confirm_store;
            move |Json(req): Json<ConfirmAssociationRequest>| {
                let store = store.clone();
                async move {
                    let memory_id = req.memory_id.trim().to_string();
                    if memory_id.is_empty() {
                        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
                            "error": "missing_memory_id",
                            "message": "必须提供 memory_id"
                        }))));
                    }
                    let query = req.query.clone();
                    // 克隆后移入阻塞任务，避免外层后续审计与响应无法使用
                    let blocking_store = store.clone();
                    let confirm_id = memory_id.clone();
                    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
                        let mut guard = loop {
                            match blocking_store.try_lock() {
                                Ok(g) => break g,
                                Err(_) => std::thread::sleep(Duration::from_millis(50)),
                            }
                        };
                        guard.confirm_memory(&confirm_id).map_err(|e| e.to_string())
                    })
                    .await;

                    match result {
                        Ok(Ok(true)) => {
                            // 结构化审计：记录用户确认行为（可观测，不参与排序）
                            if let Ok(mut guard) = store.try_lock() {
                                let mut meta = std::collections::HashMap::new();
                                meta.insert(
                                    "query".to_string(),
                                    query.unwrap_or_default(),
                                );
                                guard.audit_trail.record(
                                    AuditEventType::AssociationConfirmed,
                                    format!("用户确认联想：「{}」", memory_id),
                                    "用户在联想探索中确认该记忆与意图相关，写入活跃锚点".to_string(),
                                    vec![memory_id.clone()],
                                    meta,
                                );
                            }
                            Ok(Json(serde_json::json!({
                                "confirmed": true,
                                "memory_id": memory_id
                            })))
                        }
                        Ok(Ok(false)) => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({
                            "error": "memory_not_found",
                            "message": "该记忆不存在或已过期"
                        })))),
                        Ok(Err(e)) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                            "error": "confirm_failed",
                            "message": e
                        })))),
                        Err(join_error) => {
                            eprintln!("[v1/associations/confirm] spawn_blocking panic: {}", join_error);
                            Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                                "error": "confirm_internal_error",
                                "message": "确认操作内部错误，服务已保持运行"
                            }))))
                        }
                    }
                }
            }
        }))
        // POST /v1/memories/correct — 用户修正记忆
        .route("/memories/correct", post({
            let store = correct_store;
            move |Json(req): Json<CorrectRequest>| {
                let store = store.clone();
                async move {
                    // v0.9.6 P1 修复：correct_memory 走跨进程写守卫，移入 spawn_blocking
                    // 避免守卫自旋占死 HTTP worker。
                    let memory_id = req.memory_id.clone();
                    let memory_id_for_task = memory_id.clone();
                    let outcome = tokio::task::spawn_blocking(move || match store.try_lock() {
                        Ok(mut store) => {
                            match store.correct_memory(&memory_id_for_task, &req.content, req.reason.as_deref()) {
                                Ok(Some(memory)) => Ok((memory.id, memory.version, memory.version_history.len())),
                                Ok(None) => Err("not_found"),
                                Err(e) => {
                                    eprintln!("[v1/correct] 修正失败: {}", e);
                                    Err("write_failed")
                                }
                            }
                        }
                        Err(_) => Err("store_busy"),
                    })
                    .await;

                    match outcome {
                        Ok(Ok((id, version, history))) => {
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(CorrectResponse {
                                success: true,
                                memory_id: id,
                                new_version: version,
                                history_versions: history,
                            }))
                        }
                        Ok(Err("not_found")) => Err((
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({
                                "error": "memory_not_found",
                                "message": format!("未找到记忆: {}", memory_id)
                            })),
                        )),
                        Ok(Err("store_busy")) => Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(serde_json::json!({
                                "error": "store_busy",
                                "message": "记忆服务繁忙，请稍后重试"
                            })),
                        )),
                        Ok(Err(_)) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "correction_failed",
                                "message": "修正失败，请稍后重试"
                            })),
                        )),
                        Err(join_error) => {
                            eprintln!("[v1/correct] spawn_blocking panic: {}", join_error);
                            Err((
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({
                                    "error": "correction_failed",
                                    "message": "修正任务异常终止"
                                })),
                            ))
                        }
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
                    // v0.9.6 P1 修复：unfold_memory 走跨进程写守卫，移入 spawn_blocking
                    let memory_id = req.memory_id;
                    let memory_id_for_task = memory_id.clone();
                    let outcome = tokio::task::spawn_blocking(move || match store.try_lock() {
                        Ok(mut store) => match store.unfold_memory(&memory_id_for_task, req.min_activation) {
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
                                Ok((unfolded, fidelity, sub_count))
                            }
                            Ok(None) => Err("not_found"),
                            Err(e) => {
                                eprintln!("[v1/unfold] 拆解失败: {}", e);
                                Err("write_failed")
                            }
                        },
                        Err(_) => Err("store_busy"),
                    })
                    .await;

                    match outcome {
                        Ok(Ok((unfolded, fidelity, sub_count))) => {
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(UnfoldResponse {
                                success: true,
                                source_memory_id: memory_id,
                                sub_vectors_count: sub_count,
                                fidelity,
                                sub_memories: unfolded,
                            }))
                        }
                        Ok(Err("not_found")) => Err((
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({
                                "error": "unfold_failed",
                                "message": format!("无法拆解记忆: {} (可能不是合成类型或无洛书向量)", memory_id)
                            })),
                        )),
                        Ok(Err("store_busy")) => Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(serde_json::json!({
                                "error": "store_busy",
                                "message": "记忆服务繁忙，请稍后重试"
                            })),
                        )),
                        Ok(Err(_)) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "unfold_error",
                                "message": "拆解失败，请稍后重试"
                            })),
                        )),
                        Err(join_error) => {
                            eprintln!("[v1/unfold] spawn_blocking panic: {}", join_error);
                            Err((
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({
                                    "error": "unfold_error",
                                    "message": "拆解任务异常终止"
                                })),
                            ))
                        }
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
                            let bagua_balance = (snapshot.bagua_entropy / 3.0) * 100.0;
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
                                    // P1 调节器心跳：随 dao 指标一起暴露（前端 loadDaoMetrics 直接读取）
                                    regulator_heartbeat: Some(store.regulator_heartbeat.clone()),
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
                let dv = dev_mode;
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
                                "dev_mode": dv,
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
                                // v0.9.0：新增 dev_mode 标志（前端据此显示开发模式 UI）
                                obj.insert("dev_mode".to_string(), serde_json::Value::Bool(dv));
                                // v0.9.0：新增 consolidation 结晶状态（前端可观测结晶引擎）
                                obj.insert("consolidation".to_string(), serde_json::json!({
                                    "status": "active",
                                    "supported": true
                                }));
                                // v0.9.6：新增 data_directory（前端仪表盘据此显示真实数据目录，
                                // 替换此前硬编码的全局路径，避免项目指纹模式下误导用户）
                                obj.insert("data_directory".to_string(), serde_json::Value::String(
                                    store.persistence().data_dir().to_string_lossy().to_string()
                                ));
                                // P1 调节器心跳：暴露最近一次 regulate() 的执行状态，
                                // 前端系统状态卡据此展示"调节器心跳"（最近运行时间/最近动作）
                                obj.insert(
                                    "regulator_heartbeat".to_string(),
                                    serde_json::to_value(&store.regulator_heartbeat)
                                        .unwrap_or_else(|_| serde_json::json!({})),
                                );
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
                        Some("association") => FeedbackTarget::AssociationRelevance,
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

                            let store = lock_store_with_timeout(&store).await?;
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

                            let store = lock_store_with_timeout(&store).await?;
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

                            let store = lock_store_with_timeout(&store).await?;
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
                            // 普通反馈记录（retrieval, synthesis, quarantine_override, association）
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

                            let store = lock_store_with_timeout(&store).await?;

                            // 阶段D：联想级反馈——解析联想上下文（排名、命中的检索通路）
                            let feedback_id = if target_type == FeedbackTarget::AssociationRelevance {
                                let rank = body.get("rank").and_then(|v| v.as_u64()).map(|r| r as usize);
                                let hit_paths: Vec<String> = body
                                    .get("hit_paths")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .map(|s| s.to_string())
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                store.user_feedback.record_association_feedback(
                                    feedback_type.clone(),
                                    &memory_id,
                                    query,
                                    rank,
                                    hit_paths,
                                    note,
                                )
                            } else {
                                store.user_feedback.record_feedback(
                                    feedback_type.clone(),
                                    target_type.clone(),
                                    &memory_id,
                                    query,
                                    note,
                                )
                            };

                            let stats = store.user_feedback.get_stats();

                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "success": true,
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
                                    FeedbackTarget::AssociationRelevance => "association",
                                },
                                "memory_id": memory_id,
                                "stats": {
                                    "total_feedback": stats.total_feedback,
                                    "positive_ratio": stats.positive_ratio,
                                },
                                "message": if target_type == FeedbackTarget::QuarantineOverride {
                                    "隔离恢复请求已记录，将在下一个调节周期中处理"
                                } else if target_type == FeedbackTarget::AssociationRelevance {
                                    "联想结果反馈已记录（仅观测，不影响当前排序）"
                                } else {
                                    "反馈已记录，感谢您的参与"
                                },
                            })))
                        }
                    }
                }
            }
        }))
        // GET /v1/feedback/association-stats — 联想级反馈聚合（阶段D：反馈回流闭环）
        //
        // 按记忆聚合联想级反馈（AssociationRelevance），返回建议性质量分。
        // 约束：仅观测/建议，不接入默认排序决策。
        .route("/feedback/association-stats", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "total": 0,
                                "stats": [],
                                "lock_busy": true,
                                "degraded": true,
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                            })));
                        }
                    };

                    let stats = store.user_feedback.get_association_stats();
                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "total": stats.len(),
                        "stats": stats,
                    })))
                }
            }
        }))
        // GET /v1/feedback/association-activity — 联想执行活动聚合（v0.9.6 首屏仪表盘）
        //
        // 聚合 RetrievalExecuted 审计事件，返回真实的联想检索执行情况：
        // 执行次数、平均候选规模、通路命中分布、最近执行时间。
        // 与 association-stats（反馈统计）互补：反馈需用户评价才有数据，
        // 而执行活动只要发生过 enrich 就有数据，确保首屏"记忆联想执行"
        // 仪表盘不再长期显示空白。仅观测，不参与检索排序。
        .route("/feedback/association-activity", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "total_executions": 0,
                                "lock_busy": true,
                                "degraded": true,
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                            })));
                        }
                    };

                    let q = AuditQuery {
                        from_ms: None,
                        to_ms: None,
                        event_types: Some(vec![AuditEventType::RetrievalExecuted]),
                        memory_id: None,
                        limit: Some(1000),
                    };
                    let events = store.audit_trail.query(&q);
                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(
                        aggregate_association_activity(&events),
                    ))
                }
            }
        }))
        // GET /v1/associations/records — 联想中心本地过程记录。
        // 仅读取本机审计链，不上传查询原文；客户端可选择隐藏原文。
        .route("/associations/records", get({
            let store = metrics_store.clone();
            move |Query(params): Query<HashMap<String, String>>| {
                let store = store.clone();
                async move {
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "records": [], "total": 0, "lock_busy": true, "degraded": true,
                                "message": "记忆系统正在执行后台任务，记录稍后自动加载"
                            })));
                        }
                    };
                    let from_ms = params.get("from_ms").and_then(|v| v.parse::<u64>().ok());
                    let to_ms = params.get("to_ms").and_then(|v| v.parse::<u64>().ok());
                    let limit = params.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(50).clamp(1, 100);
                    let offset = params.get("offset").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                    let multi_hop = params.get("multi_hop").map(|v| v == "1" || v == "true").unwrap_or(false);
                    let evidence_only = params.get("evidence_only").map(|v| v == "1" || v == "true").unwrap_or(false);
                    let query = AuditQuery {
                        from_ms, to_ms,
                        event_types: Some(vec![AuditEventType::RetrievalExecuted]),
                        memory_id: None, limit: Some(1000),
                    };
                    let events = store.audit_trail.query(&query);
                    let filtered: Vec<&AuditEvent> = events.into_iter().filter(|event| {
                        let trail_len = event.metadata.get("trail").and_then(|v| serde_json::from_str::<Vec<serde_json::Value>>(v).ok()).map(|v| v.len()).unwrap_or(0);
                        let has_evidence = event.metadata.get("regression_evidence").map(|v| v != "{}" && v != "null").unwrap_or(false);
                        (!multi_hop || trail_len > 1) && (!evidence_only || has_evidence)
                    }).collect();
                    let total = filtered.len();
                    let records: Vec<serde_json::Value> = filtered.into_iter().skip(offset).take(limit).map(|event| {
                        let md = &event.metadata;
                        serde_json::json!({
                            "id": event.id,
                            "timestamp_ms": event.timestamp_ms,
                            "query_length": md.get("query_length").and_then(|v| v.parse::<u64>().ok()),
                            "fast_hits": md.get("fast_hits").and_then(|v| v.parse::<u64>().ok()),
                            "deep_hits": md.get("deep_hits").and_then(|v| v.parse::<u64>().ok()),
                            "total_candidates": md.get("total_candidates").and_then(|v| v.parse::<u64>().ok()),
                            "filtered_count": md.get("filtered_count").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0),
                            "trail": md.get("trail").and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok()).unwrap_or_else(|| serde_json::json!([])),
                            "regression_evidence": md.get("regression_evidence").and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok()).unwrap_or_else(|| serde_json::json!({})),
                            "affected_memory_ids": event.affected_memory_ids,
                            "description": event.description,
                        })
                    }).collect();
                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "records": records, "total": total, "offset": offset, "limit": limit
                    })))
                }
            }
        }))
        // DELETE /v1/associations/records — 清除本机联想过程记录。
        .route("/associations/records", delete({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    let mut store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => return Err((StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                            "error": "store_busy", "message": "记忆服务繁忙，请稍后重试"
                        })))),
                    };
                    let cleared = store.audit_trail.clear_matching(|event| event.event_type == AuditEventType::RetrievalExecuted);
                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "success": true, "cleared": cleared, "message": "联想记录已在本机清除"
                    })))
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
                    // v0.9.1 修复：lock_busy 时返回 200 + 降级数据而非 503（与 /health/system 一致）
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "total": 0,
                                "total_all": 0,
                                "type_statistics": {},
                                "events": [],
                                "lock_busy": true,
                                "degraded": true,
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                            })));
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
                                "consolidation_completed" => Some(AuditEventType::SynthesisCreated),
                                "retrieval_executed" => Some(AuditEventType::RetrievalExecuted),
                                "association_confirmed" => Some(AuditEventType::AssociationConfirmed),
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
                    // v0.9.1 修复：lock_busy 时返回 200 + 降级数据而非 503（与 /health/system 一致）
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "total_memories": null,
                                "expired_count": null,
                                "storage_size_bytes": null,
                                "by_type": {},
                                "by_project": {},
                                "lock_busy": true,
                                "degraded": true,
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                            })));
                        }
                    };
                    match store.stats() {
                        Ok(stats) => {
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "total_memories": stats.total_memories,
                                "expired_count": stats.expired_count,
                                "recent_added": stats.recent_added,
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
                    // v0.9.1 修复：lock_busy 时返回 200 + 降级数据而非 503（与 /health/system 一致）
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "memories": [],
                                "total": null,
                                "returned": 0,
                                "lock_busy": true,
                                "degraded": true,
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                            })));
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
                                        "bagua_category": m.bagua_category,
                                        "daoti_preview_gua": m.daoti_preview_gua,
                                        "daoti_preview_bagua": m.daoti_preview_bagua,
                                        "daoti_preview_version": m.daoti_preview_version,
                                        "luoshu_vector": m.luoshu_vector,
                                        "topological_depth": m.topological_depth,
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
                    // v0.9.1 修复：lock_busy 时返回 200 + 降级数据而非 503（与 /health/system 一致）
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "memories": [],
                                "total": null,
                                "lock_busy": true,
                                "degraded": true,
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                            })));
                        }
                    };

                    let filter = crate::memory_store::ListFilter {
                        project: params.project.clone(),
                        tags: params.tags.clone(),
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
                                        "bagua_category": m.bagua_category,
                                        "daoti_preview_gua": m.daoti_preview_gua,
                                        "daoti_preview_bagua": m.daoti_preview_bagua,
                                        "daoti_preview_version": m.daoti_preview_version,
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
        // v0.9.6 修复：GET /v1/memories/synthesis-timeline — 结晶历史时间线
        //
        // 结晶的持久化产物是 Synthesis 类型记忆。原实现从审计事件
        // （synthesis_created）提取，但历史合成未落审计事件导致时间线永远空白。
        // 修复：直接按创建时间倒序返回合成记忆，即"每次结晶被持久化记录"。
        // 查询参数：limit（默认 10，最大 50）
        .route("/memories/synthesis-timeline", get({
            let store = metrics_store.clone();
            move |Query(params): Query<HashMap<String, String>>| {
                let store = store.clone();
                async move {
                    let limit = params
                        .get("limit")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(10);

                    // v0.9.1 修复：lock_busy 时返回 200 + 降级数据而非 503（与其他端点一致）
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "items": [],
                                "total": null,
                                "lock_busy": true,
                                "degraded": true,
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                            })));
                        }
                    };

                    // 只在存储层过滤 Synthesis，减少传输量；纯函数内二次过滤保证契约
                    let filter = crate::memory_store::ListFilter {
                        memory_type: Some(MemoryType::Synthesis),
                        limit: 50,
                        sort_by: crate::memory_store::SortBy::CreatedAt,
                        order: crate::memory_store::SortOrder::Desc,
                        ..Default::default()
                    };

                    match store.list_memories(&filter) {
                        Ok((memories, _total)) => {
                            Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(
                                build_synthesis_timeline(&memories, limit),
                            ))
                        }
                        Err(e) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "synthesis_timeline_failed",
                                "message": format!("结晶时间线获取失败: {}", e)
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
                    // v0.9.3 修复：锁获取超时保护
                    let store = lock_store_with_timeout(&store).await?;

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
                                        "bagua_category": m.bagua_category,
                                        "daoti_preview_gua": m.daoti_preview_gua,
                                        "daoti_preview_bagua": m.daoti_preview_bagua,
                                        "daoti_preview_version": m.daoti_preview_version,
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

                    // v0.9.6 P1 修复：锁获取 + 写入统一放入 spawn_blocking（blocking_lock）。
                    // 根因：store.remember() 内部会走跨进程写守卫 acquire_process_write_guard，
                    //   该守卫在被占用时用 std::thread::sleep 自旋最长 5s。若在 async handler
                    //   中同步调用，会占死 tokio worker 线程，连带拖慢 /health（实测 5.4s），
                    //   并使 2s async 锁超时失效（被阻塞 worker 无法被轮询）。
                    // 修复：移入 spawn_blocking 阻塞线程，用 blocking_lock 有界等待，
                    //   守卫自旋只占用阻塞线程池，不再饥饿 HTTP worker。
                    let blocking = tokio::task::spawn_blocking(move || {
                        let lock_deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(2);
                        let mut store = loop {
                            match store.try_lock() {
                                Ok(guard) => break guard,
                                Err(_) if std::time::Instant::now() < lock_deadline => {
                                    std::thread::sleep(std::time::Duration::from_millis(10));
                                }
                                Err(_) => return Err("store_busy"),
                            }
                        };
                        let memory = Memory::new(
                            params.content,
                            memory_type,
                            params.project,
                            params.tags,
                            importance,
                            None,
                        );
                        match store.remember(memory) {
                            Ok(saved) => Ok(saved.id),
                            Err(e) => {
                                eprintln!("[v1/remember] 记忆写入失败: {}", e);
                                Err("remember_failed")
                            }
                        }
                    })
                    .await;

                    match blocking {
                        Ok(Ok(memory_id)) => Ok::<_, (StatusCode, Json<serde_json::Value>)>(
                            Json(serde_json::json!({
                                "success": true,
                                "memory_id": memory_id,
                            }))
                        ),
                        Ok(Err("store_busy")) => Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(serde_json::json!({
                                "error": "store_busy",
                                "message": "记忆服务繁忙，请稍后重试"
                            })),
                        )),
                        Ok(Err(_)) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "remember_failed",
                                "message": "记忆写入失败，请稍后重试"
                            })),
                        )),
                        Err(join_error) => {
                            eprintln!("[v1/remember] spawn_blocking panic: {}", join_error);
                            Err((
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({
                                    "error": "remember_failed",
                                    "message": "记忆写入任务异常终止"
                                })),
                            ))
                        }
                    }
                }
            }
        }))
        // POST /v1/memories/forget — 删除单条记忆（前端删除按钮调用）
        .route("/memories/forget", post({
            let store = metrics_store.clone();
            move |Json(req): Json<ForgetRequest>| {
                let store = store.clone();
                async move {
                    // v0.9.6 P1 修复：forget 走跨进程写守卫（acquire_process_write_guard），
                    // 移入 spawn_blocking 避免守卫自旋占死 HTTP worker（与 remember/correct 同模式）。
                    let memory_id = req.memory_id;
                    let memory_id_for_task = memory_id.clone();
                    let outcome = tokio::task::spawn_blocking(move || {
                        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                        let mut store = loop {
                            match store.try_lock() {
                                Ok(guard) => break guard,
                                Err(_) if std::time::Instant::now() < deadline => {
                                    std::thread::sleep(std::time::Duration::from_millis(10));
                                }
                                Err(_) => return Err("store_busy"),
                            }
                        };
                        match store.forget(&memory_id_for_task) {
                            Ok(true) => Ok(()),
                            Ok(false) => Err("not_found"),
                            Err(e) => {
                                eprintln!("[v1/forget] 删除记忆失败: {}", e);
                                Err("forget_failed")
                            }
                        }
                    })
                    .await;

                    match outcome {
                        Ok(Ok(())) => Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(
                            serde_json::json!({
                                "success": true,
                                "memory_id": memory_id,
                            })
                        )),
                        Ok(Err("not_found")) => Err((
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({
                                "error": "memory_not_found",
                                "message": format!("未找到记忆: {}", memory_id)
                            })),
                        )),
                        Ok(Err("store_busy")) => Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(serde_json::json!({
                                "error": "store_busy",
                                "message": "记忆服务繁忙，请稍后重试"
                            })),
                        )),
                        Ok(Err(_)) => Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "forget_failed",
                                "message": "删除失败，请稍后重试"
                            })),
                        )),
                        Err(join_error) => {
                            eprintln!("[v1/forget] spawn_blocking panic: {}", join_error);
                            Err((
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({
                                    "error": "forget_failed",
                                    "message": "删除任务异常终止"
                                })),
                            ))
                        }
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
                    // v0.9.1 修复：data-location 只读磁盘文件，不依赖内存 store 状态。
                    // lock_busy 时用全局路径兜底（for_global），不再返回 503。
                    let data_dir = match store.try_lock() {
                        Ok(guard) => guard.persistence().data_dir().to_path_buf(),
                        Err(_) => {
                            // 锁被持有时，用全局数据目录兜底（不持锁，仅读静态路径）
                            crate::data_dir::DataDir::for_global().data_path().to_path_buf()
                        }
                    }; // 锁在此释放，后续不再持锁
                    // 获取记忆文件实际路径
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
                    // v0.9.1 修复：lock_busy 时返回 200 降级数据而非 503（与 /health/system 一致）
                    let store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                            match store.try_lock() {
                                Ok(guard) => guard,
                                Err(_) => {
                                    return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                        "total_events": null,
                                        "hash_chain_valid": null,
                                        "hash_chain_status": "验证延迟 — 后台合成中，稍后自动加载",
                                        "hash_chain_details": "",
                                        "anchor_count": null,
                                        "anchor_chain_valid": null,
                                        "anchor_chain_status": "验证延迟 — 后台合成中，稍后自动加载",
                                        "last_anchor_at": null,
                                        "tamper_proof": null,
                                        "verification_note": "后台合成中，完整性验证稍后自动加载",
                                        "lock_busy": true,
                                        "degraded": true,
                                        "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                                    })));
                                }
                            }
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
        // GET /v1/captains-log — 生成当前服务项目的船长日志
        // 当前指标存储绑定到服务启动时的项目，接口不支持按请求参数切换或过滤项目。
        .route("/captains-log", get({
            let store = metrics_store.clone();
            move || {
                let store = store.clone();
                async move {
                    // v0.9.1 修复：lock_busy 时返回 200 降级数据而非 503（与 /health/system 一致）
                    let mut store = match store.try_lock() {
                        Ok(guard) => guard,
                        Err(_) => {
                            return Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                                "ok": true,
                                "lock_busy": true,
                                "degraded": true,
                                "report": "[等待] 记忆系统正在执行后台合成，船长日志稍后自动加载...",
                                "message": "记忆系统正在执行后台合成，数据稍后自动加载"
                            })));
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
                    report.push_str("  Loong Recall 船长日志\n");
                    report.push_str("═══════════════════════════════════════════\n\n");

                    report.push_str("项目范围: 当前服务绑定的项目\n");
                    report.push_str(&format!("生成时间: {}\n\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")));

                    // 记忆统计
                    report.push_str("━━━ 记忆统计 ━━━\n");
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

                    report.push_str("\n━━━ 道同构度 ━━━\n");
                    if let Some(ref dao) = dao_snapshot {
                        report.push_str(&format!("  道同构度: {:.1}%\n", dao.dao_isomorphism_score * 100.0));
                        report.push_str(&format!("  八卦分布熵: {:.3}\n", dao.bagua_entropy));
                        report.push_str(&format!("  合成比率: {:.1}%\n", dao.synthesis_ratio * 100.0));
                        report.push_str(&format!("  活跃记忆: {} 条\n", dao.active_memories));
                        report.push_str(&format!("  结晶记忆: {} 条\n", dao.crystallized_memories));
                    } else {
                        report.push_str("  （暂无道同构度数据）\n");
                    }

                    report.push_str("\n━━━ 系统健康 ━━━\n");
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
                            report.push_str("  [警告] 道同构度偏低，建议检查编码器状态\n");
                        }
                    } else {
                        report.push_str("  （暂无健康数据）\n");
                    }

                    report.push_str("\n━━━ 审计追踪 ━━━\n");
                    report.push_str(&format!("  审计事件总数: {} 条\n", audit_stats.values().sum::<usize>()));
                    report.push_str("  事件类型分布:\n");
                    let mut audit_types: Vec<_> = audit_stats.iter().collect();
                    audit_types.sort_by(|a, b| b.1.cmp(a.1));
                    for (t, c) in audit_types {
                        report.push_str(&format!("    - {}: {} 条\n", t, c));
                    }

                    report.push_str("\n━━━ 状态摘要 ━━━\n");
                    let status_text = if let Some(ref h) = health {
                        match h.system_mode {
                            crate::engine::health_report::SystemMode::Healthy => "系统运行健康",
                            crate::engine::health_report::SystemMode::Degraded => "编码器已降级，语义能力降低",
                            crate::engine::health_report::SystemMode::Oscillating => "系统参数正在自我调整中",
                            crate::engine::health_report::SystemMode::Drifting => "检测到参数持续漂移，建议检查",
                            crate::engine::health_report::SystemMode::Frozen => "调节器已冻结，需要手动干预",
                            crate::engine::health_report::SystemMode::Overloaded => "记忆库接近容量上限",
                        }
                    } else {
                        "系统正在初始化中"
                    };
                    report.push_str(&format!("  {}\n", status_text));

                    report.push_str("\n═══════════════════════════════════════════\n");
                    report.push('\n');
                    report.push_str("═══════════════════════════════════════════\n");

                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "project_scope": "current_service_project",
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

                    // 如果提供了 keywords 参数，则使用多关键词搜索
                    let keywords: Vec<String> = if !keywords_str.is_empty() {
                        keywords_str
                            .split(',')
                            .map(|k| k.trim().to_string())
                            .filter(|k| !k.is_empty())
                            .collect()
                    } else if !query.is_empty() {
                        vec![query.clone()]
                    } else {
                        Vec::new()
                    };

                    let result = if keywords.is_empty() {
                        safe_recent_code_search(manager.clone(), top_k)
                            .await
                            .map_err(|error| {
                                let (code, message) = match error {
                                    SearchError::LockTimeout => ("search_busy", "搜索服务繁忙，请稍后重试"),
                                    SearchError::ExecutionTimeout => ("search_timeout", "搜索超时，请稍后重试"),
                                    SearchError::Panic => ("search_internal_error", "搜索内部错误，服务已保持运行"),
                                };
                                (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                                    "error": code,
                                    "message": message,
                                })))
                            })?
                    } else {
                        safe_code_search(manager.clone(), keywords, top_k)
                            .await
                            .map_err(|error| {
                                let (code, message) = match error {
                                    SearchError::LockTimeout => ("search_busy", "搜索服务繁忙，请稍后重试"),
                                    SearchError::ExecutionTimeout => ("search_timeout", "搜索超时，请缩小查询范围后重试"),
                                    SearchError::Panic => ("search_internal_error", "搜索内部错误，服务已保持运行"),
                                };
                                (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                                    "error": code,
                                    "message": message,
                                })))
                            })?
                    };

                    let stats = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        manager.clone().lock_owned(),
                    )
                    .await
                    .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                        "error": "search_busy",
                        "message": "搜索服务繁忙，请稍后重试"
                    }))))?;
                    let stats = stats.get_stats();

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
        // POST /v1/regulator/unfreeze — 手动解冻调节器（补全 RegulatorUnfrozen 审计闭环）
        //
        // 当调节器因连续无效调节被冻结后，用户可通过此端点手动解冻。
        // 解冻后记录 RegulatorUnfrozen 审计事件，恢复自动调节能力。
        .route("/regulator/unfreeze", post({
            let store = regulator_store;
            move || {
                let store = store.clone();
                async move {
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

                    let was_frozen = store.dao_regulator.is_frozen();
                    if was_frozen {
                        store.dao_regulator.unfreeze();
                        // 记录审计：调节器手动解冻（用户干预行为的可回溯记录）
                        store.record_audit(
                            AuditEventType::RegulatorUnfrozen,
                            "调节器已手动解冻",
                            "用户通过 /v1/regulator/unfreeze 端点手动恢复自动调节",
                            Vec::new(),
                        );
                    }

                    let state = store.dao_regulator.get_state();
                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "ok": true,
                        "was_frozen": was_frozen,
                        "is_frozen": state.is_frozen,
                        "message": if was_frozen {
                            "调节器已成功解冻，自动调节已恢复"
                        } else {
                            "调节器未处于冻结状态，无需解冻"
                        }
                    })))
                }
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
        // POST /v1/backup/restore — 从快照恢复全部运行时文件
        // v0.9.7 审查修复（HCSE-P0）：
        //   1) 恢复前以有界轮询获取 store 锁并全程持有，防止与并发 remember/save
        //      互写数据文件（半新半旧混合状态）；拿不到锁返回 409 而非硬闯。
        //   2) 恢复成功后立即失效内存缓存，否则后续读取仍返回恢复前数据。
        //   3) 失败返回 500（此前返回 200 + success:false，前端按状态码会误判）。
        //   4) 快照路径必须位于 backups_dir 内（backup::restore_backup 内强制校验）。
        .route("/backup/restore", post({
            let restore_store = restore_store.clone();
            move |body: Json<serde_json::Value>| async move {
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            // 有界获取 store 锁（最多等待 5s）
            let lock_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let guard = loop {
                match restore_store.try_lock() {
                    Ok(g) => break g,
                    Err(_) if std::time::Instant::now() < lock_deadline => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    Err(_) => {
                        return Err((StatusCode::CONFLICT, Json(serde_json::json!({
                            "success": false,
                            "error": "restore_busy",
                            "message": "记忆系统正忙（合成/写入中），请稍后重试恢复"
                        }))));
                    }
                }
            };
            let result = tokio::task::spawn_blocking(move || crate::backup::restore_backup(std::path::Path::new(&path))).await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": e.to_string()}))))?;
            match result {
                Ok(()) => {
                    // 失效缓存并立即从磁盘重载，保证后续读取看到恢复后的数据
                    guard.invalidate_cache_after_external_restore();
                    let restored_count = guard.total_count().unwrap_or(0);
                    drop(guard);
                    Ok::<_, (StatusCode, Json<serde_json::Value>)>(Json(serde_json::json!({
                        "success": true,
                        "restored_memories": restored_count
                    })))
                }
                Err(e) => {
                    drop(guard);
                    Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"success": false, "error": e}))))
                }
            }
        }}))
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

                    // SSRF 防护：字面量校验（scheme/userinfo/host 网段检查）
                    if let Err(e) = crate::url_safety::validate_http_url(&req.endpoint) {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "ok": false,
                                "status": 0,
                                "message": format!("endpoint 校验失败: {}", e),
                                "latency_ms": 0
                            })),
                        ));
                    }
                    // SSRF 防护：解析并固定本次连接目标，保留原 URL 的 Host/SNI。
                    let resolved_ips = match crate::url_safety::resolve_and_check_dns(&req.endpoint).await {
                        Ok(ips) => ips,
                        Err(e) => {
                            return Err((
                                StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({
                                    "ok": false,
                                    "status": 0,
                                    "message": format!("endpoint 校验失败: {}", e),
                                    "latency_ms": 0
                                })),
                            ));
                        }
                    };

                    // 校验 endpoint 是合法 HTTP/HTTPS URL，并拼接 /models 路径（OpenAI 兼容）
                    let test_url = format!("{}/models", req.endpoint.trim_end_matches('/'));

                    let start = std::time::Instant::now();

                    // 构造 HTTP 客户端（带 10 秒超时）。resolve 将连接固定到刚才校验的 IP，
                    // URL 仍使用原域名，因此 HTTPS 的 Host 与 SNI 不变。
                    let endpoint_url = url::Url::parse(&req.endpoint).expect("已完成 URL 校验");
                    let endpoint_host = endpoint_url.host_str().expect("已完成主机校验");
                    let endpoint_port = endpoint_url.port_or_known_default().expect("HTTP(S) 默认端口");
                    let client = match reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(10))
                        .resolve(endpoint_host, std::net::SocketAddr::new(resolved_ips[0], endpoint_port))
                        .user_agent("loong-recall-llm-test")
                        // SSRF 防护：禁止自动重定向（防 302 跳转至内网/metadata）
                        .redirect(reqwest::redirect::Policy::none())
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
                                "message": err_msg,
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
    fn test_forget_request_serde() {
        // 最小请求体：仅提供 memory_id
        let json = r#"{"memory_id":"mem-001"}"#;
        let req: ForgetRequest = serde_json::from_str(json).expect("反序列化失败");
        assert_eq!(req.memory_id, "mem-001");
    }

    #[test]
    fn test_forget_request_requires_memory_id() {
        // memory_id 是必填字段，缺失应反序列化失败
        let json = r#"{}"#;
        let result: Result<ForgetRequest, _> = serde_json::from_str(json);
        assert!(result.is_err(), "缺失 memory_id 字段应反序列化失败");
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
                regulator_heartbeat: None,
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
            daoti_preview_gua: None,
            daoti_preview_bagua: None,
            daoti_preview_version: None,
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

    /// 阶段D：EnrichResponse 联想解释块字段契约（只观测，不参与排序）。
    #[test]
    fn test_enrich_explanation_block_fields() {
        let resp = EnrichResponse {
            memories: vec![],
            fast_path_hits: 6,
            deep_path_hits: 4,
            total: 3,
            trail: vec![],
            regression_evidence: HashMap::new(),
            filtered_count: 0,
            association_mode: "state_machine_navigation".to_string(),
            explanation: EnrichExplanation {
                query: "Rust 编译失败 ModuleNotFoundError 怎么排查".to_string(),
                weights: ExplanationWeights {
                    fast: 1.0,
                    deep: 1.8,
                },
                rrf_k: 60.0,
                fast_path_hits: 6,
                deep_path_hits: 4,
                total_candidates: 3,
                items: vec![EnrichExplanationItem {
                    id: "mem-001".to_string(),
                    rank: 1,
                    score: 1.0,
                    fused_contrib: 0.0311,
                    fast_contrib: 0.0164,
                    deep_contrib: 0.0147,
                    fast_rank: Some(1),
                    deep_rank: Some(3),
                    hit_paths: vec!["fast", "deep"],
                }],
            },
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        let exp = &json["explanation"];
        assert_eq!(exp["query"], "Rust 编译失败 ModuleNotFoundError 怎么排查");
        assert!((exp["weights"]["fast"].as_f64().unwrap() - 1.0).abs() < 1e-5);
        assert!((exp["weights"]["deep"].as_f64().unwrap() - 1.8).abs() < 1e-5);
        assert!((exp["rrf_k"].as_f64().unwrap() - 60.0).abs() < 1e-5);
        assert_eq!(exp["fast_path_hits"], 6);
        assert_eq!(exp["deep_path_hits"], 4);
        assert_eq!(exp["total_candidates"], 3);
        let item = &exp["items"][0];
        assert_eq!(item["id"], "mem-001");
        assert_eq!(item["rank"], 1);
        assert_eq!(item["fast_rank"], 1);
        assert_eq!(item["deep_rank"], 3);
        assert_eq!(item["hit_paths"], serde_json::json!(["fast", "deep"]));
        assert!((item["fast_contrib"].as_f64().unwrap() - 0.0164).abs() < 1e-5);
        assert!((item["deep_contrib"].as_f64().unwrap() - 0.0147).abs() < 1e-5);
        // 顶层与解释块保持一致
        assert_eq!(json["total"], 3);
    }

    /// 阶段D：单路命中的解释条目 hit_paths 只含命中的通路。
    #[test]
    fn test_enrich_explanation_item_single_path() {
        let resp = EnrichResponse {
            memories: vec![],
            fast_path_hits: 1,
            deep_path_hits: 0,
            total: 1,
            trail: vec![],
            regression_evidence: HashMap::new(),
            filtered_count: 0,
            association_mode: "state_machine_navigation".to_string(),
            explanation: EnrichExplanation {
                query: "仅快速命中".to_string(),
                weights: ExplanationWeights {
                    fast: 1.0,
                    deep: 1.0,
                },
                rrf_k: 60.0,
                fast_path_hits: 1,
                deep_path_hits: 0,
                total_candidates: 1,
                items: vec![EnrichExplanationItem {
                    id: "mem-002".to_string(),
                    rank: 1,
                    score: 1.0,
                    fused_contrib: 0.0164,
                    fast_contrib: 0.0164,
                    deep_contrib: 0.0,
                    fast_rank: Some(1),
                    deep_rank: None,
                    hit_paths: vec!["fast"],
                }],
            },
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        let item = &json["explanation"]["items"][0];
        assert_eq!(item["deep_rank"], serde_json::Value::Null);
        assert_eq!(item["hit_paths"], serde_json::json!(["fast"]));
    }

    // ===== 4. 联想执行活动聚合纯函数测试（v0.9.6 首屏仪表盘契约） =====

    /// 构造一条 RetrievalExecuted 审计事件（metadata 为字符串映射，模拟真实落盘格式）
    fn retrieval_event(ts: u64, fast: &str, deep: &str, total: &str) -> AuditEvent {
        let mut md = std::collections::HashMap::new();
        md.insert("fast_hits".to_string(), fast.to_string());
        md.insert("deep_hits".to_string(), deep.to_string());
        md.insert("total_candidates".to_string(), total.to_string());
        AuditEvent {
            id: format!("audit-{ts}"),
            timestamp_ms: ts,
            event_type: AuditEventType::RetrievalExecuted,
            description: String::new(),
            reason: String::new(),
            affected_memory_ids: vec![],
            metadata: md,
            previous_hash: String::new(),
            event_hash: String::new(),
            hash_format: String::new(),
        }
    }

    #[test]
    fn test_association_activity_field_names_and_aggregation() {
        // 双通路命中 2 条 + 仅快速 1 条 + 仅深度 1 条，候选 10/20/30/40
        let e1 = retrieval_event(100, "5", "5", "10");
        let e2 = retrieval_event(300, "3", "3", "20");
        let e3 = retrieval_event(200, "4", "0", "30");
        let e4 = retrieval_event(50, "0", "2", "40");
        let refs: Vec<&AuditEvent> = vec![&e1, &e2, &e3, &e4];
        let agg = aggregate_association_activity(&refs);

        // 字段名契约（前端消费依赖这些 snake_case 键）
        for key in [
            "total_executions",
            "avg_candidates",
            "fast_only_count",
            "deep_only_count",
            "both_count",
            "last_execution_ms",
            "recent",
        ] {
            assert!(agg.get(key).is_some(), "缺少字段: {key}");
        }

        assert_eq!(agg["total_executions"], 4);
        assert_eq!(agg["both_count"], 2);
        assert_eq!(agg["fast_only_count"], 1);
        assert_eq!(agg["deep_only_count"], 1);
        // 平均候选 = (10+20+30+40)/4 = 25
        assert_eq!(agg["avg_candidates"], 25.0);
        // last_execution_ms 取最大时间戳（非数组末位），验证乱序鲁棒
        assert_eq!(agg["last_execution_ms"], 300);
        // recent 最多 10 条，此处 4 条全含
        assert_eq!(agg["recent"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn test_association_activity_empty_is_safe() {
        // 空事件集不得 panic，且各计数为 0、avg 为 0、last 为 null
        let agg = aggregate_association_activity(&[]);
        assert_eq!(agg["total_executions"], 0);
        assert_eq!(agg["avg_candidates"], 0.0);
        assert!(agg["last_execution_ms"].is_null());
        assert!(agg["recent"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_association_activity_ignores_unparseable_metadata() {
        // metadata 缺字段或非数字时按未命中处理，不污染统计
        let mut md = std::collections::HashMap::new();
        md.insert("fast_hits".to_string(), "abc".to_string()); // 不可解析
        let ev = AuditEvent {
            id: "x".into(),
            timestamp_ms: 10,
            event_type: AuditEventType::RetrievalExecuted,
            description: String::new(),
            reason: String::new(),
            affected_memory_ids: vec![],
            metadata: md,
            previous_hash: String::new(),
            event_hash: String::new(),
            hash_format: String::new(),
        };
        let refs: Vec<&AuditEvent> = vec![&ev];
        let agg = aggregate_association_activity(&refs);
        assert_eq!(agg["total_executions"], 1);
        assert_eq!(agg["both_count"], 0);
        assert_eq!(agg["fast_only_count"], 0);
        assert_eq!(agg["avg_candidates"], 0.0);
    }

    // ===== 5. 结晶历史时间线纯函数测试（v0.9.6 首屏时间线契约） =====

    /// 构造一条指定创建时间的记忆（Synthesis 或对照类型），覆盖默认随机时间
    fn timed_memory(id: &str, ts_ms: u64, content: &str, mtype: MemoryType) -> Memory {
        let mut m = Memory::new(
            content.to_string(),
            mtype,
            Some("test".to_string()),
            vec![],
            Importance::default(),
            None,
        );
        m.id = id.to_string();
        let ts = chrono::DateTime::from_timestamp_millis(ts_ms as i64).unwrap();
        m.created_at = ts;
        m.updated_at = ts;
        m.last_accessed = ts;
        m
    }

    #[test]
    fn test_synthesis_timeline_field_names_and_order() {
        let m1 = timed_memory("a", 100, "第一次结晶", MemoryType::Synthesis);
        let m2 = timed_memory("b", 300, "第二次结晶", MemoryType::Synthesis);
        // 非合成记忆必须被过滤，即使创建时间更晚
        let fact = timed_memory("c", 999, "普通记忆", MemoryType::Fact);
        // 故意乱序传入，验证防御性倒序排序
        let agg = build_synthesis_timeline(&[m1, fact, m2], 10);

        // 顶层字段契约（前端消费依赖这些 snake_case 键）
        for key in ["items", "total"] {
            assert!(agg.get(key).is_some(), "缺少字段: {key}");
        }
        assert_eq!(agg["total"], 2, "应只统计 Synthesis 记忆");
        let items = agg["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        // 倒序契约：b(300) 在 a(100) 前
        assert_eq!(items[0]["id"], "b");
        assert_eq!(items[1]["id"], "a");
        // 条目字段契约
        for key in [
            "id",
            "content",
            "memory_type",
            "project",
            "created_at_ms",
            "importance",
            // v0.9.6 P1-1：结晶成长链路字段
            "source_count",
            "confidence",
            "information_gain",
        ] {
            assert!(items[0].get(key).is_some(), "条目缺少字段: {key}");
        }
        assert_eq!(items[0]["source_count"], 0, "默认记忆无来源应记 0");
        assert_eq!(items[0]["memory_type"], "synthesis");
        assert_eq!(items[0]["created_at_ms"], 300);
    }

    #[test]
    fn test_synthesis_timeline_limit_and_empty() {
        // limit 截断契约
        let mems: Vec<Memory> = (0..20)
            .map(|i| {
                timed_memory(
                    &format!("m{i}"),
                    i as u64,
                    &format!("结晶{i}"),
                    MemoryType::Synthesis,
                )
            })
            .collect();
        let agg = build_synthesis_timeline(&mems, 5);
        assert_eq!(agg["total"], 20);
        assert_eq!(agg["items"].as_array().unwrap().len(), 5);
        // 空输入安全
        let empty = build_synthesis_timeline(&[], 10);
        assert_eq!(empty["total"], 0);
        assert!(empty["items"].as_array().unwrap().is_empty());
    }

    // ===== 6. 联想中心探索契约测试（v0.9.7 产品化） =====

    #[test]
    fn test_association_explore_request_defaults() {
        // 不传 depth/width 时使用默认：4 层、3 分支
        let req: AssociationExploreRequest =
            serde_json::from_value(serde_json::json!({ "query": "今晚吃什么" })).unwrap();
        assert_eq!(req.depth, 4);
        assert_eq!(req.width, 3);
        assert_eq!(req.query.as_deref(), Some("今晚吃什么"));

        // query 与 memory_id 均可作为起点
        let req2: AssociationExploreRequest = serde_json::from_value(
            serde_json::json!({ "memory_id": "mem-001", "depth": 2, "width": 1 }),
        )
        .unwrap();
        assert_eq!(req2.query, None);
        assert_eq!(req2.memory_id.as_deref(), Some("mem-001"));
        assert_eq!(req2.depth, 2);
        assert_eq!(req2.width, 1);

        // depth 越界时在 handler 入口被 clamp 到 1..=4、width clamp 到 1..=3
        let req3: AssociationExploreRequest =
            serde_json::from_value(serde_json::json!({ "query": "x", "depth": 99, "width": 99 }))
                .unwrap();
        assert_eq!(req3.depth.clamp(1, 4), 4);
        assert_eq!(req3.width.clamp(1, 3), 3);
    }

    #[test]
    fn test_association_explore_response_field_names() {
        use crate::engine::memory_state_machine::AssociationStep;
        let resp = AssociationExploreResponse {
            root: Some("mem-root".to_string()),
            depth: 2,
            width: 2,
            nodes: vec![
                ExploreNode {
                    id: "mem-root".into(),
                    content: "起点记忆".into(),
                    depth: 0,
                    score: 1.0,
                    source: "root".into(),
                    evidence: None,
                },
                ExploreNode {
                    id: "mem-child".into(),
                    content: "联想记忆".into(),
                    depth: 1,
                    score: 0.7,
                    source: "expanded".into(),
                    evidence: Some("联想桥强关联".into()),
                },
            ],
            edges: vec![ExploreEdge {
                from: "mem-root".into(),
                to: "mem-child".into(),
                score: 0.7,
                evidence: Some("联想桥强关联".into()),
            }],
            trail: vec![AssociationStep {
                from_id: Some("mem-root".into()),
                to_id: "mem-child".into(),
                score: 0.7,
                depth: 1,
            }],
            total_expanded: 2,
            interrupted: false,
            // 存在通过校验的扩展节点 → 非弱匹配
            weak_match: false,
        };
        let json = serde_json::to_value(&resp).expect("序列化失败");
        assert_eq!(json["root"], "mem-root");
        assert_eq!(json["depth"], 2);
        assert_eq!(json["width"], 2);
        assert_eq!(json["interrupted"], false);
        // v0.9.7：weak_match 契约——有扩展节点时必须为 false
        assert_eq!(json["weak_match"], false);
        for key in ["nodes", "edges", "trail"] {
            assert!(json[key].is_array(), "缺少数组字段: {key}");
        }
        let node = &json["nodes"][1];
        for key in ["id", "content", "depth", "score", "source", "evidence"] {
            assert!(node.get(key).is_some(), "节点缺少字段: {key}");
        }
        assert_eq!(node["source"], "expanded");
        assert_eq!(node["evidence"], "联想桥强关联");
        let edge = &json["edges"][0];
        for key in ["from", "to", "score", "evidence"] {
            assert!(edge.get(key).is_some(), "边缺少字段: {key}");
        }
    }

    /// 探索集成测试：BFS 有界性、visited 去重、节点深度合法。
    #[test]
    fn test_run_association_explore_bfs_bounds() {
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_bfs_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());

        // 写入一组生活联想链相关记忆：食物、餐馆、和谁吃、在哪吃
        for content in [
            "今晚吃什么好呢",
            "潮汕牛肉火锅和粤式烧腊都是不错的选项",
            "楼下新开的粤菜餐馆珠江新城店环境很好",
            "和小王一起去吃火锅很开心",
            "中山路的老字号肠粉店排队很久",
            "周末和老婆去郊外野餐带了三明治",
            "完全无关的记忆：绿萝每周换一次盆土",
        ] {
            let memory = Memory::new(
                content.to_string(),
                MemoryType::Conversation,
                None,
                vec![],
                Importance::new(5),
                None,
            );
            store.remember(memory).unwrap();
        }

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(
            &mut store,
            Some("今晚吃什么"),
            None,
            3, // 最多 3 层
            2, // 每层最多 2 个方向
            &cancel,
        );

        assert!(resp.root.is_some(), "应找到起点记忆");
        assert!(!resp.nodes.is_empty(), "应至少返回起点节点");
        // 深度合法性：节点 depth 不超过 3
        for node in &resp.nodes {
            assert!(node.depth <= 3, "节点深度越界: {}", node.depth);
            if node.depth == 0 {
                assert_eq!(node.source, "root", "起点节点来源应为 root");
            } else {
                assert_eq!(node.source, "expanded", "发散节点来源应为 expanded");
            }
        }
        // visited 去重：节点 id 不得重复
        let mut ids = std::collections::HashSet::new();
        for node in &resp.nodes {
            assert!(
                ids.insert(node.id.clone()),
                "探索出现重复节点 id: {}",
                node.id
            );
        }
        // 边端点必须都出现在节点集合中
        for edge in &resp.edges {
            assert!(
                ids.contains(&edge.from),
                "边起点不在节点集合: {}",
                edge.from
            );
            assert!(ids.contains(&edge.to), "边终点不在节点集合: {}", edge.to);
        }
        // 有界性：每层分支 <= width（2），总节点数 <= 1 + 2 + 4 + 8 = 15
        assert!(
            resp.nodes.len() <= 15,
            "探索节点数量超过有界上限: {}",
            resp.nodes.len()
        );
        assert!(!resp.interrupted, "小规模探索不应中断");
    }

    /// 探索集成测试：从记忆 ID 出发且目标不存在时安全返回空树。
    #[test]
    fn test_run_association_explore_missing_memory_id_safe() {
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_missing_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());
        let cancel = AtomicBool::new(false);
        let resp =
            run_association_explore(&mut store, None, Some("mem-does-not-exist"), 2, 1, &cancel);
        assert!(resp.root.is_none(), "不存在的记忆 ID 不应产生根节点");
        assert!(resp.nodes.is_empty());
        assert!(resp.edges.is_empty());
    }

    /// v0.9.7 精确度修复：多跳扩散的回归校验必须锚定起点主题。
    /// 深层记忆与"漂移陷阱"（与 hop1 共享词、但与起点主题无关）不得出现。
    #[test]
    fn test_run_association_explore_anchored_regression_no_drift() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_anchor_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());

        // 食物主题链 + 漂移陷阱：
        // 陷阱与 hop1 记忆共享"浇水"，但与起点"今晚吃番茄炒蛋"主题无关。
        // 若回归校验随父内容漂移（旧行为），深层会把陷阱拉进结果。
        for content in [
            "今晚吃番茄炒蛋，简单好吃",
            "番茄苗要每日浇水补光才能长好",
            "浇水的计算机程序实现细节与源码",
        ] {
            let memory = Memory::new(
                content.to_string(),
                MemoryType::Conversation,
                None,
                vec![],
                Importance::new(5),
                None,
            );
            store.remember(memory).unwrap();
        }

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(
            &mut store,
            Some("今晚吃什么"),
            None,
            3, // 允许 3 层，确保会走到深层扩散
            2,
            &cancel,
        );

        assert!(resp.root.is_some(), "应找到起点记忆");
        // 锚定校验：任何层级的节点都不得包含漂移陷阱内容
        for node in &resp.nodes {
            assert!(
                !node.content.contains("计算机"),
                "深层联想漂移：无关记忆混入联想结果: {}",
                node.content
            );
        }
        // hop1 的主题内记忆（番茄苗）应被保留——锚定只杀漂移，不杀主题
        assert!(
            resp.nodes
                .iter()
                .any(|node| node.content.contains("番茄苗")),
            "锚定校验不应误杀主题内记忆"
        );
        assert!(!resp.weak_match, "存在主题内扩展节点时不应标记弱匹配");
    }

    /// v0.9.7 精确度修复：根节点主导性门禁。
    /// 仅凭单个泛指词（"什么"）重叠的代码记忆不得上位当起点，
    /// 实质共鸣（≥2 个 token）的主题记忆必须胜出。
    #[test]
    fn test_run_association_explore_root_dominance_gate() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_root_gate_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = MemoryStore::new(JsonPersistence::new(&dir_str).unwrap());

        // 干扰记忆：与"今晚吃什么"只共享泛指 bigram「什么」1 个 token；
        // 主题记忆：命中「今晚」「晚吃」2 个 token，实质共鸣；
        // 扩展记忆：与起点"今晚吃火锅还是点外卖"共鸣（火锅），供发散层使用。
        // 旧行为（无条件取 top1）在分数接近时会把代码记忆当起点。
        for content in [
            "这个报错是什么意思",
            "今晚吃火锅还是点外卖",
            "周末和朋友也想吃火锅，虾滑必点",
        ] {
            let memory = Memory::new(
                content.to_string(),
                MemoryType::Conversation,
                None,
                vec![],
                Importance::new(5),
                None,
            );
            store.remember(memory).unwrap();
        }

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(&mut store, Some("今晚吃什么"), None, 2, 2, &cancel);

        assert!(resp.root.is_some(), "应存在实质共鸣的起点记忆");
        let root_node = resp
            .nodes
            .iter()
            .find(|node| node.depth == 0)
            .expect("起点节点必须存在");
        assert!(
            root_node.content.contains("火锅"),
            "起点应为食物主题记忆，而不是泛指词重叠的无关记忆: {}",
            root_node.content
        );
        // 干扰记忆不得以任何身份（起点或发散）出现在联想结果里
        assert!(
            resp.nodes.iter().all(|node| !node.content.contains("报错")),
            "泛指词重叠的无关记忆混入联想结果"
        );
        assert!(!resp.weak_match, "存在主题记忆与扩展节点时不应标记弱匹配");
    }

    /// 根节点门禁的诚实空态：记忆库里只有泛指词重叠的无关记忆时，
    /// 不硬凑起点，直接标记 weak_match。
    #[test]
    fn test_run_association_explore_root_gate_weak_match() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_root_weak_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        // 统计模式确定性构造：语义旁路在本用例中必须保持失效，
        // 泛指词重叠的无关记忆无论是否装有 ML 模型都不应上位
        let mut store = new_statistical_store(&dir_str);

        // 只有与查询共享单个泛指词的记忆——旧行为会把它硬凑成起点
        let memory = Memory::new(
            "这个报错是什么意思".to_string(),
            MemoryType::Conversation,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(memory).unwrap();

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(&mut store, Some("今晚吃什么"), None, 2, 2, &cancel);

        assert!(resp.root.is_none(), "泛指词重叠的无关记忆不应成为起点");
        assert!(resp.nodes.is_empty(), "弱匹配时不应产生任何节点");
        assert!(resp.weak_match, "无实质共鸣候选时应标记弱匹配");
    }

    /// 泛指 bigram 组合不得虚假共鸣（v0.9.7 CDP 回归发现的真实缺陷）：
    /// "量子物理是什么"与含测试文本"是什么"的代码 chunk 共享「是什」
    /// 「什么」两个泛指 bigram——修复前刚好凑满 min_required=2，代码
    /// chunk 抢走起点并让整条联想链陷入代码邻域（depth=4 扩散超时 503）。
    /// 修复后：泛指 bigram 不计入门禁，实义 token（量子/物理）零重叠
    /// → 诚实空态。对照查询验证实义 bigram 不被误伤。
    #[test]
    fn test_run_association_explore_generic_bigram_gate() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_generic_gate_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        // 统计模式（确定性）：不依赖 ML 是否可用
        let mut store = new_statistical_store(&dir_str);

        // 干扰记忆：含"是什么"字样的代码记忆（模拟代码 chunk 里的测试文本）
        let noisy = Memory::new(
            "来源：src/lib.rs:1-10；主题：rs。项目的数据库和缓存架构是什么".to_string(),
            MemoryType::CodeContext,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        store.remember(noisy).unwrap();

        // 查询 1：库里没有量子物理相关内容 → 不硬凑代码起点，诚实空态
        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(&mut store, Some("量子物理是什么"), None, 2, 2, &cancel);
        assert!(
            resp.root.is_none() && resp.weak_match,
            "泛指 bigram 组合不应让代码记忆虚假上位: root={:?}",
            resp.root
        );

        // 查询 2（对照）：实义 bigram（数据库/据库/缓存）命中时正常上位，
        // 证明过滤只剔除泛指组合，不伤害实义共鸣。
        let resp2 =
            run_association_explore(&mut store, Some("数据库和缓存架构"), None, 2, 2, &cancel);
        assert!(
            resp2.root.is_some() && !resp2.weak_match,
            "实义 token 重叠的代码记忆应正常成为起点"
        );
    }

    /// 构造统计模式（确定性）记忆库：ML 模型存在与否不影响测试预期。
    /// 本机装有 bge 模型时，`MemoryStore::new` 会加载 ML 编码器并激活
    /// 语义旁路；需要验证"纯词面门禁"的测试必须显式固定为统计模式。
    fn new_statistical_store(dir_str: &str) -> MemoryStore<JsonPersistence> {
        #[cfg(feature = "ml")]
        {
            MemoryStore::new_with_encoder(
                JsonPersistence::new(dir_str).unwrap(),
                crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder::new_statistical(),
            )
        }
        #[cfg(not(feature = "ml"))]
        {
            // 非 ml 构建本身就是统计编码器
            MemoryStore::new(JsonPersistence::new(dir_str).unwrap())
        }
    }

    /// 构造 ML 能力记忆库（镜像 bin/server.rs 的启动链路：
    /// LuoShuMlEncoder::load → new_with_ml）。模型不可用时返回 None，
    /// 调用方跳过用例——语义旁路只能在真实 ML 环境验证。
    #[cfg(feature = "ml")]
    fn new_ml_capable_store(dir_str: &str) -> Option<MemoryStore<JsonPersistence>> {
        let ml = crate::engine::luoshu_encoder_ml::LuoShuMlEncoder::load().ok()?;
        Some(MemoryStore::new_with_encoder(
            JsonPersistence::new(dir_str).unwrap(),
            crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder::new_with_ml(ml),
        ))
    }

    /// 语义旁路阈值标定（仅 ml 构建且模型可用时执行）：
    /// 断言"语义强相关对"的余弦显著高于"泛指结构相似对"，
    /// 保证 ASSOCIATION_ROOT_MIN_SEMANTIC_SIM 存在可分离的取值区间，
    /// 并打印具体数值供阈值调整参考。
    #[test]
    #[cfg(feature = "ml")]
    fn test_semantic_similarity_threshold_calibration() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_sem_calib_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        // 显式加载 ML 编码器（默认构造恒为统计模式，不会加载模型）
        let Some(store) = new_ml_capable_store(&dir_str) else {
            eprintln!("[标定] ML 编码器不可用，跳过语义阈值标定");
            return;
        };

        let anniversary = Memory::new(
            "结婚纪念日是 10 月 20 号，每年都要一起吃顿好的。".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(9),
            None,
        );
        let decoy = Memory::new(
            "这个报错是什么意思".to_string(),
            MemoryType::Conversation,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        let hiking = Memory::new(
            "周末去白云山徒步，山顶风景很好。".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(6),
            None,
        );

        let pairs = [
            ("我以前记过什么重要日子？", &anniversary),
            ("今晚吃什么", &decoy),
            ("周末去哪儿玩", &hiking),
        ];
        let sims = pairs
            .iter()
            .map(|(q, m)| store.semantic_similarities(q, &[m])[0].unwrap_or(f32::NAN));
        let (sim_days, sim_decoy, sim_hiking) = {
            let v: Vec<f32> = sims.collect();
            (v[0], v[1], v[2])
        };
        eprintln!(
            "[标定] 重要日子↔结婚纪念日 = {sim_days:.4}；今晚吃什么↔报错 = {sim_decoy:.4}；周末去哪儿玩↔白云山 = {sim_hiking:.4}；当前阈值 = {ASSOCIATION_ROOT_MIN_SEMANTIC_SIM}"
        );
        // 相关对必须与泛指结构对可分离，阈值才有存在意义
        assert!(
            sim_days > sim_decoy,
            "语义强相关对的余弦必须高于泛指结构对：{sim_days} vs {sim_decoy}"
        );
        assert!(
            sim_hiking > sim_decoy,
            "语义强相关对的余弦必须高于泛指结构对：{sim_hiking} vs {sim_decoy}"
        );
    }

    /// 语义旁路行为：查询"重要日子"与"结婚纪念日"记忆词面零重叠，
    /// 纯词面门禁下必然弱匹配；ML 编码器可用时语义旁路应挽救该起点。
    #[test]
    #[cfg(feature = "ml")]
    fn test_run_association_explore_semantic_bypass_life_days() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_sem_bypass_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        // 显式加载 ML 编码器（默认构造恒为统计模式，不会加载模型）
        let Some(mut store) = new_ml_capable_store(&dir_str) else {
            eprintln!("[语义旁路] ML 编码器不可用，本用例仅在 ml 环境执行");
            return;
        };

        let memory = Memory::new(
            "结婚纪念日是 10 月 20 号，每年都要一起吃顿好的。".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(9),
            None,
        );
        store.remember(memory).unwrap();

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(
            &mut store,
            Some("我以前记过什么重要日子？"),
            None,
            2,
            2,
            &cancel,
        );

        assert!(!resp.weak_match, "语义强相关的纪念日记忆不应被误判为弱匹配");
        let root_node = resp
            .nodes
            .iter()
            .find(|node| node.depth == 0)
            .expect("起点节点必须存在");
        assert!(
            root_node.content.contains("结婚纪念日"),
            "语义旁路应召回结婚纪念日记忆: {}",
            root_node.content
        );
    }

    /// 开发上下文噪声过滤：非代码起点的联想链不得混入 code_context
    /// 代码块（用户视角的"src/benchmark"噪声）。词面构造保证起点
    /// 稳定通过门禁，两种 feature 配置下行为一致。
    #[test]
    fn test_run_association_explore_skips_code_children_for_life_root() {
        use crate::memory_types::{Importance, Memory, MemoryType};
        use std::sync::atomic::AtomicBool;
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let dir = std::env::temp_dir().join(format!("lrc_explore_code_noise_{ts}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_str().unwrap().to_string();
        let mut store = new_statistical_store(&dir_str);

        // 起点：与查询共享 周末/去哪/哪儿 多个 token，稳定通过词面门禁
        let root = Memory::new(
            "周末去哪儿玩都可以，我去白云山徒步看风景。".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(6),
            None,
        );
        // 真实发散：与起点共享 白云山/徒步 token
        let real_child = Memory::new(
            "白云山徒步记得带水，南门上山比较轻松。".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        // 代码噪声：内容刻意包含"白云山"以挤进扩散候选，但类型是代码块
        let code_noise = Memory::new(
            "来源：src/benchmark.rs；主题：rs。白云山景区人流量数据统计实现。".to_string(),
            MemoryType::CodeContext,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        for memory in [root, real_child, code_noise] {
            store.remember(memory).unwrap();
        }

        let cancel = AtomicBool::new(false);
        let resp = run_association_explore(&mut store, Some("周末去哪儿玩"), None, 2, 2, &cancel);

        assert!(resp.root.is_some(), "起点应通过词面实质共鸣门禁");
        assert!(
            resp.nodes
                .iter()
                .all(|node| !node.content.contains("src/benchmark")),
            "代码块不得混入生活起点的联想链: {:?}",
            resp.nodes.iter().map(|n| &n.content).collect::<Vec<_>>()
        );
        assert!(
            resp.nodes
                .iter()
                .any(|node| node.content.contains("白云山徒步记得带水")),
            "真实发散记忆应正常出现"
        );
    }
}
