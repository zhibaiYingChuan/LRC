// ============================================================
// 许可证: Apache 2.0
// 本文件实现主动发现通道，属于公开层 (Layer 1)。
// ============================================================
//
// 主动发现通道 — 常驻推演进程的"触发源"角色（P7）
//
// 架构定位（2026-09-14 预注册，见 daoti 研究资产侧 PREREG_ACTIVE_DISCOVERY.md）：
//
//   **该进程不是排序增强器，是触发源。**
//
// 历史先验（诚实引用，不得选择性忽略）：
//   - PREREG_NAV（2026-09-05）：三种导航信号源召回增益 NO-GO（+1~2pp）
//   - PREREG_ASSOC_NAV（P3.5）：静态快照信号注入 NO-GO（G1 5%、G2 −4.7pp）
//   - PREREG_CLOSED_LOOP（P6/CL4）：完全体闭环 NO-GO（L 与 L-off 逐字节一致）
//
// 上述三轮否证的对象都是「该信号**参与检索排序**」。它们从未检验
// 「该信号作为**触发源**驱动主动发现」—— 二者的差别是**类别差别**而非程度差别：
// 前者要求信号"比 BGE 更准"，后者只要求信号"能在 BGE 不触发时触发一次检查"。
//
// 三条硬约束（PREREG 二节，违反即为方向回退）：
//   原则一：主动发现**不参与排序** —— 本模块的输出绝不写回任何排序状态
//           （不碰 gua_sims、不注入 navigated_deep_recall）。
//   原则二：提示必须可忽略 —— 不弹模态框、不自动聚焦、不阻塞任何用户操作。
//   原则三：发现结果必须有依据 —— 每条候选必须携带 reason（触发卦象 + 缘由 + 时间跨度）。
//
// License 边界：产品侧只消费不计算 —— 本模块只**消费** daemon 产出的 JSON 信号
// （漂移事件 + probe 词），不内置任何引擎或词典（DaoTi License）。
// daemon 不可达时，全链路静默降级（不产生任何候选），行为与未引入本模块时一致。
// ============================================================

use serde::{Deserialize, Serialize};

use crate::memory_store::MemoryStore;
use crate::memory_store_types::{RecallFilter, RecallResult};
use crate::JsonPersistence;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 主动发现门控（**默认关**，与 LRC_DAOTI_NAVIGATE / LRC_DAOTI_REFLECT 独立）。
///
/// 三个门控互相独立，本实验只开启本门控，前两个保持关闭——
/// 这保证了 D3（零伤害承诺）在**机制上**成立：排序路径根本没有该信号。
///
/// **运行期实时读取，不缓存**（与 assoc_debias_enabled 同款约定，
/// 便于实验期间动态开关而不必重启 sidecar）。
pub fn active_discovery_enabled() -> bool {
    std::env::var("LRC_ACTIVE_DISCOVERY")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// 相关度阈值下限（PREREG §3.3 `RELEVANCE_MIN` 的保守初值，标定后回填）。
/// 环境变量 `LRC_DISCOVERY_RELEVANCE_MIN` 可覆盖（标定/消融用）。
pub const DISCOVERY_RELEVANCE_MIN_DEFAULT: f32 = 0.35;

/// 未访问天数下限（PREREG §3.1「有效发现候选」定义第 2 条，硬编码）。
pub const DISCOVERY_STALE_DAYS_DEFAULT: i64 = 7;

/// 每日提示上限（PREREG D4「无轰炸承诺」，硬编码）。
pub const DISCOVERY_DAILY_CAP_DEFAULT: usize = 3;

/// 连续忽略降频阈值（PREREG §3.3，保守值）。
pub const DISCOVERY_IGNORE_DECAY_DEFAULT: u32 = 3;

/// 单次检查最多产出的候选数（防止一次漂移刷出大量提示）。
pub const DISCOVERY_MAX_CANDIDATES: usize = 3;

/// 探索查询里并入"最近用户交互摘要"时，取多少条活跃记忆（PREREG §步骤二）。
const DISCOVERY_CONTEXT_ACTIVE_LIMIT: usize = 4;

/// 读取相关度阈值（环境变量可覆盖，运行期实时读取）。
pub fn discovery_relevance_min() -> f32 {
    std::env::var("LRC_DISCOVERY_RELEVANCE_MIN")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0)
        .unwrap_or(DISCOVERY_RELEVANCE_MIN_DEFAULT)
}

/// 读取未访问天数下限（环境变量可覆盖，便于实验加速）。
pub fn discovery_stale_days() -> i64 {
    std::env::var("LRC_DISCOVERY_STALE_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DISCOVERY_STALE_DAYS_DEFAULT)
}

/// 读取每日上限（环境变量可覆盖；D4 判据在默认值 3 上成立）。
pub fn discovery_daily_cap() -> usize {
    std::env::var("LRC_DISCOVERY_DAILY_CAP")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DISCOVERY_DAILY_CAP_DEFAULT)
}

/// 读取连续忽略降频阈值。
pub fn discovery_ignore_decay() -> u32 {
    std::env::var("LRC_DISCOVERY_IGNORE_DECAY")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DISCOVERY_IGNORE_DECAY_DEFAULT)
}

/// 状态漂移事件（由 daemon `GET /drift` 的 `pending_events` 元素反序列化）。
///
/// 契约与 daoti 研究资产侧的 `make_drift_event` 一一对应：
///   StateDriftEvent { timestamp, drift_magnitude, dominant_gua_before,
///                     dominant_gua_after, query_text }
#[derive(Debug, Clone, Deserialize)]
pub struct StateDriftEvent {
    /// 事件时间戳（毫秒，daemon 侧生成）
    #[serde(default)]
    pub timestamp: i64,
    /// 漂移量（0-1）；daemon 侧已按阈值过滤
    #[serde(default)]
    pub drift_magnitude: f32,
    /// 漂移前的主导卦（可能为 None —— 冷启动期）
    #[serde(default)]
    pub dominant_gua_before: Option<String>,
    /// 漂移后的主导卦
    #[serde(default)]
    pub dominant_gua_after: Option<String>,
    /// **方案C：卦象→语义文本 的翻译结果**（daemon 侧用离线词典产出）。
    ///
    /// **为什么需要它**（实测驱动，见 PREREG §3.5.5）：卦名是 1-2 字的符号，
    /// 与数百字的用户上下文**并置拼接**后交给加性 TF-IDF 检索时会被完全淹没——
    /// 实测"换任何卦象候选都不变"（探索侧 B≡C 逐字节相同）。
    /// 根因是"64 维符号信号 → 文本检索"之间缺少翻译层；`query_text` 即该层。
    ///
    /// 缺省（旧 daemon / 字段缺失）→ 空串 → 回退到卦名，行为与修正前一致。
    #[serde(default)]
    pub query_text: Option<String>,
}

/// daemon `GET /drift` 的响应体。
#[derive(Debug, Clone, Deserialize)]
pub struct DriftStateResponse {
    #[serde(default)]
    pub last_drift: f32,
    #[serde(default)]
    pub drift_total: f32,
    #[serde(default)]
    pub threshold: f32,
    #[serde(default)]
    pub explore_beats: u64,
    #[serde(default)]
    pub active_gua: Option<String>,
    #[serde(default)]
    pub active_palace: Option<String>,
    #[serde(default)]
    pub pending_events: Vec<StateDriftEvent>,
}

/// 发现候选（PREREG §步骤二 的输出契约 `DiscoveryCandidate`）。
///
/// 字段 `reason` 承载**原则三**（发现结果必须有依据）：必须能说明"为什么提示这条"。
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryCandidate {
    /// 被发现的记忆 ID
    pub memory_id: String,
    /// 内容摘要（截断，避免超长文本进 UI）
    pub content_preview: String,
    /// 相关度分数（0-1）
    pub relevance_score: f32,
    /// 距上次访问的天数
    pub days_since_last_access: i64,
    /// 发现依据（原则三：必须可解释）
    pub reason: DiscoveryReason,
}

/// 发现依据：解释"为什么提示这条记忆"（PREREG 原则三）。
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryReason {
    /// 触发本次检查的漂移卦象（before → after）
    pub drift_from_gua: Option<String>,
    pub drift_to_gua: Option<String>,
    /// 触发本检查的漂移量
    pub drift_magnitude: f32,
    /// 人类可读的缘由（面向 UI 展示，中文）
    pub human_readable: String,
}

/// 主动发现结果（一次检查的完整产出，供 API 与实验统计）。
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryOutcome {
    /// 本次检查是否真的执行（门控关闭 / daemon 不可达 → false）
    pub executed: bool,
    /// 未执行的原因（可观测性；executed=true 时为 None）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    /// 消费到的漂移事件数（触发源强度）
    pub drift_events_consumed: usize,
    /// 通过全部判据的候选
    pub candidates: Vec<DiscoveryCandidate>,
    /// 被过滤掉的候选数（可观测性：噪声率）
    pub filtered_out: usize,
    /// 使用的相关度阈值（标定取证）
    pub relevance_min_used: f32,
    /// 未访问天数判据（标定取证）
    pub stale_days_used: i64,
    /// 步骤四：本触发卦是否因"连续忽略"被降频而直接跳过
    pub suppressed_by_feedback: bool,
    /// 步骤三 D4：本日已展示条数（含本次）
    pub shown_today: usize,
    /// 步骤三 D4：本日额度（判据上限）
    pub daily_cap: usize,
    /// **展示额度裁剪前**的完整产出（判据 D1 的计量对象）。
    ///
    /// **为什么必须与 `candidates` 分开**：`candidates` 是"今日还能展示几条"的
    /// 结果，受 D4 每日上限约束（默认 3）。而 D1 判据（PREREG §3.1）的定义是
    /// "每 100 次漂移事件中**产生**有效发现候选的比例"——它衡量**发现机制的能力**，
    /// 不是展示纪律。若用 `candidates` 计 D1，当日额度用尽后所有检查都返回 0，
    /// D1 会被 D4 人为压低（两个独立判据被耦合，且会误判为 NO-GO）。
    ///
    /// 这是 PREREG §步骤二/D1 与 §步骤三/D4 的**职责切分**：
    ///   产出（本字段）= 步骤二的输出，D1 在此计量；
    ///   展示（`candidates`）= 步骤三经 D4 裁剪后的输出，前端只用它。
    pub produced: Vec<DiscoveryCandidate>,
}

/// 内容摘要截断长度（字符数，非字节；按 char 边界安全截断）。
const PREVIEW_MAX_CHARS: usize = 80;

/// 探索查询中"最近用户交互摘要"的最大字符数（方案A 分路专用的上下文上限）。
const QUERY_CONTEXT_MAX_CHARS: usize = 120;

/// 方案A 的 RRF 融合常数（与既有检索一致，便于行为可比）。
const DISCOVERY_RRF_K: f32 = 60.0;

/// 构造**触发源查询**（方案C：卦象的语义文本）。
///
/// 为什么不再拼接上下文（v1.0 修正，实测驱动，见 PREREG §3.5.5）：
/// 原实现把"卦名 + 数百字上下文摘要"**并置**成一条查询，交给加性 TF-IDF。
/// 实测证明：短卦名被长摘要完全淹没——换任何卦象、甚至换成语料实词，
/// 候选都**逐字节相同**（B≡C）。即触发源从未真正进入检索，实验等于没测。
///
/// 修正后分工（方案C + 方案A）：
///   - **本函数**只产出"触发源语义文本"（daemon 侧已把卦象翻译为多个实义词），
///     作为**独立查询**走一路检索 —— 触发源不再与上下文竞争；
///   - 上下文摘要另走一路（`build_context_query`），两路结果 RRF 融合。
///     这既解决了淹没问题，又保留"用户最近在想什么"的语义贡献。
///
/// 回退：`query_text` 缺失（旧 daemon）→ 用卦名，行为与修正前一致。
fn build_trigger_query(event: &StateDriftEvent) -> String {
    if let Some(q) = event.query_text.as_deref() {
        let q = q.trim();
        if !q.is_empty() {
            return q.to_string();
        }
    }
    event
        .dominant_gua_after
        .clone()
        .or_else(|| event.dominant_gua_before.clone())
        .unwrap_or_default()
}

/// 构造**上下文查询**（方案A 的第二路：用户最近在想什么）。
///
/// 只读保证（D3）：`active_ids` 只读取快照，不激活、不转移、不持久化。
fn build_context_query<P>(store: &MemoryStore<P>) -> Option<String>
where
    P: crate::persistence::Persistence,
{
    let active_ids = store
        .memory_state_machine
        .active_ids(DISCOVERY_CONTEXT_ACTIVE_LIMIT);
    if active_ids.is_empty() {
        return None;
    }
    let mut context = String::new();
    for m in store.memories_by_ids(&active_ids) {
        if !context.is_empty() {
            context.push(' ');
        }
        context.push_str(&m.content);
        if context.chars().count() >= QUERY_CONTEXT_MAX_CHARS {
            break;
        }
    }
    let context: String = context.chars().take(QUERY_CONTEXT_MAX_CHARS).collect();
    if context.trim().is_empty() {
        None
    } else {
        Some(context)
    }
}

/// 生成内容摘要（按 char 边界截断，避免 UTF-8 字节切裂）。
fn preview_of(content: &str) -> String {
    let trimmed = content.trim();
    let mut out: String = trimmed.chars().take(PREVIEW_MAX_CHARS).collect();
    if trimmed.chars().count() > PREVIEW_MAX_CHARS {
        out.push('…');
    }
    out
}

/// 从 daemon 拉取漂移状态（超时 2s，失败返回 None —— 静默降级）。
///
/// 复用既有 daemon 通信模式（server::fetch_daoti_navigation 同款客户端配置）：
/// 2s 连接+读超时、`DAOTI_SERVICE_URL` 可覆盖、缺省 127.0.0.1:3222。
pub async fn fetch_drift_state() -> Option<DriftStateResponse> {
    let base =
        std::env::var("DAOTI_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3222".to_string());
    let url = format!("{}/drift", base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        // 主动发现是后台通道，超时必须严格——绝不拖累任何用户可见请求
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<DriftStateResponse>().await.ok()
}

/// 消费 daemon 的漂移事件（`POST /drift/consume`）。
///
/// 语义：LRC 侧**成功把事件转为主动检查之后**才调用，避免同一事件被重复消费。
/// 失败静默返回 false（消费失败不会造成错误候选，只会导致下次重复检查——
/// 这是"宁可重复检查也不丢事件"的保守取向，与 PREREG 原则二一致）。
pub async fn consume_drift_events() -> bool {
    let base =
        std::env::var("DAOTI_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3222".to_string());
    let url = format!("{}/drift/consume", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    matches!(client.post(&url).send().await, Ok(r) if r.status().is_success())
}

/// 对一条候选做"有效发现"判据校验（PREREG §3.1 的精确定义，防止事后放宽）。
///
/// 三条同时满足才算有效：
///   1. relevance_score ≥ RELEVANCE_MIN
///   2. days_since_last_access ≥ 7
///   3. 通过既有质量防线（由调用方在检索阶段已保证）
///
/// 返回 true 表示有效。
fn is_valid_candidate(
    relevance: f32,
    days_since_access: i64,
    relevance_min: f32,
    stale_days: i64,
) -> bool {
    relevance >= relevance_min && days_since_access >= stale_days
}

/// 构造基础结果（统一填默认的可观测字段，避免各处字面量散落）。
fn base_outcome(
    executed: bool,
    skip_reason: Option<&str>,
    drift_events_consumed: usize,
) -> DiscoveryOutcome {
    DiscoveryOutcome {
        executed,
        skip_reason: skip_reason.map(|s| s.to_string()),
        drift_events_consumed,
        candidates: Vec::new(),
        filtered_out: 0,
        relevance_min_used: discovery_relevance_min(),
        stale_days_used: discovery_stale_days(),
        suppressed_by_feedback: false,
        shown_today: 0,
        daily_cap: 0,
        produced: Vec::new(),
    }
}

/// 主动发现的**核心检查**：用漂移事件的卦象作为查询，检索"用户未主动查询
/// 但可能相关"的旧记忆，产出候选。
///
/// **独立代码路径**（原则一）：本函数只读取记忆与原查询无关的检索结果，
/// 不写入任何排序状态。调用方保证它不在用户查询的请求路径上。
///
/// 参数：
///   - `store`：记忆库（只读使用；调用方以 `try_lock` 获取，锁忙则跳过本轮）
///   - `drift`：触发源信号
///
/// 返回：DiscoveryOutcome（含跳过原因，供可观测性）
pub fn run_discovery_check<P>(
    store: &mut MemoryStore<P>,
    drift: &DriftStateResponse,
) -> DiscoveryOutcome
where
    P: crate::persistence::Persistence,
{
    let relevance_min = discovery_relevance_min();
    let stale_days = discovery_stale_days();
    let now = chrono::Utc::now();

    // 触发源强度：无待消费事件 → 无事可做（不制造空检查）
    if drift.pending_events.is_empty() {
        return base_outcome(true, Some("no_drift_events"), 0);
    }

    // 取最新事件（队尾）——它代表当前状态的最新方向。
    let latest = match drift.pending_events.last() {
        Some(e) => e,
        None => unreachable!("pending_events 非空已在上方保证"),
    };

    // 构造**触发源查询**（方案C）：卦象的语义文本，**独立成一路**，不与上下文拼接。
    // 这是 §3.5.5 暴露缺陷的直接修正——详见 `build_trigger_query` 的文档。
    let trigger_query = build_trigger_query(latest);
    // **M9 门禁对照开关**（`LRC_M9_LEGACY_CONCAT=1`，默认关闭，生产零影响）：
    // 临时恢复"卦名+上下文并置拼接"的旧实现，用于证明
    // `trigger_source_must_change_candidates` 门禁**确实能失败**（M9 要求），
    // 而不是空洞通过。实测结果（2026-09-14）：
    //   拼接模式下两个不同触发源产出的候选**是无序集合相同的同一批记忆、
    //   仅顺序不同** → 门禁（用无序集合比较）**确实失败**并给出准确诊断。
    // 这同时实证了"仅顺序/分数变化不算触发源生效"这一口径升级的必要性。
    let (trigger_query, m9_legacy_concat) = {
        let mut q = trigger_query;
        let mut concat = false;
        if std::env::var("LRC_M9_LEGACY_CONCAT").as_deref() == Ok("1") {
            if let Some(ctx) = build_context_query(store) {
                q = format!("{} {}", q, ctx);
                concat = true;
            }
        }
        (q, concat)
    };
    if trigger_query.trim().is_empty() {
        return base_outcome(true, Some("no_trigger_query"), drift.pending_events.len());
    }

    // 检索参数：top_k 取宽松值（候选池大一些，后续由 stale/relevance 判据筛选）。
    //
    // **read_only = true 是硬要求**（PREREG §3.1 D3）：见 `RecallFilter::read_only`
    // 的文档——若此处照常写回状态机，发现功能一开，用户查询的活性偏置与联想桥词
    // 就会随之改变，D3 在机制上不可能成立。
    let mut filter = RecallFilter::new();
    filter.top_k = DISCOVERY_MAX_CANDIDATES * 4;
    filter.read_only = true;

    // 第一路：触发源语义文本（方案C）—— 不与上下文竞争，故其词面信号必被计入。
    let trigger_result = match store.recall(&trigger_query, &filter) {
        Ok(r) => r,
        Err(_) => {
            return base_outcome(true, Some("recall_failed"), drift.pending_events.len());
        }
    };

    // 第二路：用户上下文（方案A）—— 保留"最近在想什么"的语义贡献。
    // 两路各自独立检索后再 RRF 融合：这既让触发源"可被听见"，
    // 又不丢掉上下文对相关性的贡献（原设计想要的是两者兼得）。
    let result = if m9_legacy_concat {
        // M9 对照：拼接模式下用单路结果（模拟旧实现，不做分路融合）
        trigger_result
    } else {
        match build_context_query(store) {
            Some(ctx) => match store.recall(&ctx, &filter) {
                Ok(ctx_result) => {
                    let fused = crate::engine::rrf::rrf_fuse(
                        &trigger_result,
                        &ctx_result,
                        DISCOVERY_MAX_CANDIDATES * 4,
                        DISCOVERY_RRF_K,
                    );
                    RecallResult::basic(fused.memories, fused.scores, trigger_result.total)
                }
                // 第二路失败 → 退化为纯触发源检索（不影响主结果）
                Err(_) => trigger_result,
            },
            None => trigger_result,
        }
    };

    let mut candidates = Vec::new();
    let mut filtered_out = 0usize;

    // **相关度归一化（v1.0 实现修正，实测驱动）**：
    // `MemoryStore::recall` 返回的是**原始 TF-IDF 加权分**（实测 64~75 量级，
    // 见 temp/p7-smoke.py 标定探针），**不是 0-1**。而 PREREG §3.1 的 D1 定义
    // 是 `relevance_score ≥ RELEVANCE_MIN`，其中 RELEVANCE_MIN 初值 0.35 —— 若
    // 直接拿原始分比较，0.35 这个门槛**恒被满足**（75 > 0.35），判据退化为
    // "永远通过"，D1 的"有效发现"定义失效（这是一个真实的判据缺陷，非假说否证）。
    //
    // 归一化口径：**相对本次探索检索最高分**（`score / top_score` ∈ (0,1]）。
    // 选择理由：
    //   1. 尺度无关 —— 不依赖语料规模/文档长度带来的绝对分数漂移，避免"用
    //      判据实验数据调参"（PREREG §3.3 标定纪律）；
    //   2. 语义清晰 —— "至少达到最佳命中 35% 的相关度"，是可解释的准入线；
    //   3. 与 D1 定义兼容 —— 输出确定为 0-1，可直接与 RELEVANCE_MIN 比较。
    let top_score: f32 = result
        .scores
        .iter()
        .copied()
        .fold(0.0f32, f32::max)
        .max(f32::MIN_POSITIVE);

    for (mem, raw_score) in result.memories.iter().zip(result.scores.iter()) {
        // 判据 2：未访问天数（D 定义）
        let days = (now - mem.last_accessed).num_days();
        let relevance = (*raw_score / top_score).clamp(0.0, 1.0);
        // 判据 1 + 2 合并校验
        if !is_valid_candidate(relevance, days, relevance_min, stale_days) {
            filtered_out += 1;
            continue;
        }
        if candidates.len() >= DISCOVERY_MAX_CANDIDATES {
            break;
        }
        // 判据 3（原则三）：必须携带可解释的依据
        let human = format!(
            "你最近的状态从「{}」漂移到「{}」，这条 {} 天前的记忆与之相关",
            latest.dominant_gua_before.as_deref().unwrap_or("未知"),
            latest.dominant_gua_after.as_deref().unwrap_or("未知"),
            days
        );
        candidates.push(DiscoveryCandidate {
            memory_id: mem.id.clone(),
            content_preview: preview_of(&mem.content),
            relevance_score: relevance,
            days_since_last_access: days,
            reason: DiscoveryReason {
                drift_from_gua: latest.dominant_gua_before.clone(),
                drift_to_gua: latest.dominant_gua_after.clone(),
                drift_magnitude: latest.drift_magnitude,
                human_readable: human,
            },
        });
    }

    DiscoveryOutcome {
        // produced 是**裁剪前**的完整产出（D1 的计量对象）；candidates 在
        // run_discovery_cycle 里按 D4 剩余额度裁剪后才填。此处两者相同。
        produced: candidates.clone(),
        candidates,
        filtered_out,
        ..base_outcome(true, None, drift.pending_events.len())
    }
}

/// 标记未执行（门控关闭 / daemon 不可达）的结果（可观测性用）。
pub fn skipped_outcome(reason: &str) -> DiscoveryOutcome {
    base_outcome(false, Some(reason), 0)
}

/// 供 `JsonPersistence` 特化使用的便捷入口（生产路径的类型别名收敛点）。
///
/// 说明：`MemoryStore<P>` 对 `P` 泛型，而 HTTP 层只用 `JsonPersistence`。
/// 单独提供一个非泛型入口可避免调用点到处写泛型参数。
pub fn run_discovery_check_json(
    store: &mut MemoryStore<JsonPersistence>,
    drift: &DriftStateResponse,
) -> DiscoveryOutcome {
    run_discovery_check(store, drift)
}

// ============================================================
// 步骤三 / 步骤四：提示账本（每日额度 D4）+ 反馈回流
// ============================================================

/// 用户对一次主动提示的响应（PREREG §3.2 的四类埋点事件）。
///
/// 语义分工（PREREG §步骤四）：
///   - `Shown`：提示已展示（D2 的分母，也是 D4 的计数单位）
///   - `Clicked`：用户点开查看 → **正向**信号
///   - `Ignored`：用户看到但未响应 → **中性**信号（连续累计才降频）
///   - `NotInterested`：用户显式标记不感兴趣 → **负向**信号
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryFeedbackKind {
    Shown,
    Clicked,
    Ignored,
    NotInterested,
}

impl DiscoveryFeedbackKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "shown" => Some(Self::Shown),
            "clicked" => Some(Self::Clicked),
            "ignored" => Some(Self::Ignored),
            "not_interested" => Some(Self::NotInterested),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Shown => "shown",
            Self::Clicked => "clicked",
            Self::Ignored => "ignored",
            Self::NotInterested => "not_interested",
        }
    }
}

/// 单个触发卦的反馈累积（步骤四的降频依据）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuaFeedback {
    /// 连续忽略次数（点击/标记不感兴趣会重置）
    #[serde(default)]
    pub ignored_streak: u32,
    /// 累计点击次数（正向信号）
    #[serde(default)]
    pub clicked_total: u32,
    /// 累计"不感兴趣"次数（负向信号）
    #[serde(default)]
    pub not_interested_total: u32,
}

/// 主动发现账本（跨请求存活；JSON 原子落盘）。
///
/// 承载两类状态，**都不参与检索排序**（原则一）：
///   1. 每日已展示条数（D4 无轰炸承诺的计数依据）
///   2. 按触发卦聚合的反馈（步骤四的降频依据）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryLedger {
    /// 记账日期（本地日期，YYYY-MM-DD）；跨日自动清零 `shown_today`
    #[serde(default)]
    pub date: String,
    /// 本日已展示条数（D4：≤ 每日上限）
    #[serde(default)]
    pub shown_today: usize,
    /// 待用户响应的提示（memory_id → 触发卦）；响应后移除
    #[serde(default)]
    pub pending_gua: HashMap<String, String>,
    /// 按触发卦聚合的反馈
    #[serde(default)]
    pub by_gua: HashMap<String, GuaFeedback>,
    /// 结构版本号（便于后续格式演进）
    #[serde(default = "ledger_version")]
    pub version: u32,
}

fn ledger_version() -> u32 {
    1
}

impl Default for DiscoveryLedger {
    fn default() -> Self {
        Self {
            date: String::new(),
            shown_today: 0,
            pending_gua: HashMap::new(),
            by_gua: HashMap::new(),
            version: ledger_version(),
        }
    }
}

impl DiscoveryLedger {
    /// 当前本地日期（与账本比较用；跨日则重置计数）。
    fn today() -> String {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }

    /// 跨日重置：日期不同则清零本日计数（D4 是"每日"上限）。
    pub fn roll_day_if_needed(&mut self) {
        let today = Self::today();
        if self.date != today {
            self.date = today;
            self.shown_today = 0;
            // 跨日的未响应提示不再计入本日额度，也不再阻塞新提示
            self.pending_gua.clear();
        }
    }

    /// 本日剩余额度。
    pub fn remaining_today(&self) -> usize {
        discovery_daily_cap().saturating_sub(self.shown_today)
    }

    /// 该触发卦是否因"连续忽略"达到降频阈值（步骤四）。
    ///
    /// 返回 true 表示**本轮不再触发**该卦的主动发现。
    /// 注意：降频只影响"是否触发"与"展示优先级"，**不影响检索排序**（原则一）。
    pub fn is_suppressed(&self, gua: &str) -> bool {
        if gua.is_empty() {
            return false;
        }
        self.by_gua
            .get(gua)
            .map(|f| f.ignored_streak >= discovery_ignore_decay())
            .unwrap_or(false)
    }

    /// 登记一次展示（D4 计数 + 待响应登记）。
    pub fn record_shown(&mut self, memory_id: &str, gua: &str) {
        self.shown_today = self.shown_today.saturating_add(1);
        if !memory_id.is_empty() {
            self.pending_gua
                .insert(memory_id.to_string(), gua.to_string());
        }
    }

    /// 登记一次用户响应（步骤四：反馈回流）。
    ///
    /// 若 `memory_id` 不在待响应表中，则从传入的 `gua` 兜底——保证
    /// "报了什么就记什么"，不因账本丢失而静默丢弃用户反馈。
    pub fn record_feedback(
        &mut self,
        memory_id: &str,
        gua_fallback: &str,
        kind: DiscoveryFeedbackKind,
    ) {
        if kind == DiscoveryFeedbackKind::Shown {
            // 展示走 record_shown（避免重复计数）
            return;
        }
        let gua = self
            .pending_gua
            .remove(memory_id)
            .unwrap_or_else(|| gua_fallback.to_string());
        let entry = self.by_gua.entry(gua).or_default();
        match kind {
            DiscoveryFeedbackKind::Clicked => {
                entry.clicked_total = entry.clicked_total.saturating_add(1);
                entry.ignored_streak = 0; // 正向信号重置降频
            }
            DiscoveryFeedbackKind::NotInterested => {
                entry.not_interested_total = entry.not_interested_total.saturating_add(1);
                // 显式负向信号：直接计入忽略强度（用户已明确表态）
                entry.ignored_streak = entry.ignored_streak.saturating_add(1);
            }
            DiscoveryFeedbackKind::Ignored => {
                entry.ignored_streak = entry.ignored_streak.saturating_add(1);
            }
            DiscoveryFeedbackKind::Shown => {}
        }
    }
}

/// 账本文件名（与 memories.json 同目录，便于随数据目录一起迁移/备份）。
pub const DISCOVERY_LEDGER_FILE: &str = "discovery_ledger.json";

/// 账本落盘路径。
pub fn ledger_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DISCOVERY_LEDGER_FILE)
}

/// 读取账本（不存在/损坏 → 空账本，静默降级为"无历史"）。
pub fn load_ledger(data_dir: &Path) -> DiscoveryLedger {
    let path = ledger_path(data_dir);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return DiscoveryLedger::default();
    };
    if content.trim().is_empty() {
        return DiscoveryLedger::default();
    }
    serde_json::from_str(&content).unwrap_or_default()
}

/// 保存账本（原子写：tmp + rename）。
///
/// 失败只记录日志、不上抛——账本属于增强项，不得让发现功能因磁盘问题影响主服务。
pub fn save_ledger(data_dir: &Path, ledger: &DiscoveryLedger) {
    let path = ledger_path(data_dir);
    let Ok(json) = serde_json::to_string_pretty(ledger) else {
        eprintln!("[LRC·发现] 账本序列化失败，本轮不落盘");
        return;
    };
    if let Err(e) = crate::atomic_file::write_atomic(&path, json.as_bytes()) {
        eprintln!("[LRC·发现] 账本落盘失败: {}", e);
    }
}

/// 步骤二 + 步骤三 + 步骤四的**编排入口**（HTTP 层唯一调用点）。
///
/// 顺序（都不触碰排序路径）：
///   1. 门控关闭 → 跳过
///   2. 步骤四：触发卦因连续忽略被降频 → 跳过
///   3. 步骤二：执行发现检查（**不因 D4 额度耗尽而跳过**，见下文注释）
///   4. 步骤三：按剩余额度裁剪展示，并把展示记入账本
pub fn run_discovery_cycle(
    store: &mut MemoryStore<JsonPersistence>,
    drift: &DriftStateResponse,
    data_dir: &Path,
) -> DiscoveryOutcome {
    if !active_discovery_enabled() {
        return skipped_outcome("gate_off");
    }

    let mut ledger = load_ledger(data_dir);
    ledger.roll_day_if_needed();
    let cap = discovery_daily_cap();

    // 步骤四：降频判定（用最新漂移事件的卦作为触发卦）
    let trigger_gua = drift
        .pending_events
        .last()
        .and_then(|e| e.dominant_gua_after.clone())
        .or_else(|| drift.active_gua.clone())
        .unwrap_or_default();
    if ledger.is_suppressed(&trigger_gua) {
        let mut out = skipped_outcome("suppressed_by_feedback");
        out.suppressed_by_feedback = true;
        out.shown_today = ledger.shown_today;
        out.daily_cap = cap;
        return out;
    }

    // 步骤三 D4：本日额度（不在此提前返回——见下方说明）
    let remaining = ledger.remaining_today();

    // 步骤二：执行发现检查。
    //
    // **为什么不因额度耗尽而提前返回**（判据解耦，v1.0 实现修正）：
    //   D4（每日展示 ≤3 条）是**展示纪律**，D1（发现率）是**机制能力**。
    //   若额度耗尽就跳过检查，则当日后续漂移事件全部不产生 `produced`，
    //   D1 会被 D4 人为压低（实测会把 B 臂发现率从 ~40% 压到个位数），
    //   进而把"机制有效"误判为 NO-GO。两个判据必须各自独立可测。
    //   代价：额度已满时仍做一次只读检索——这是实验正确性换来的必要开销，
    //   且该检索不写回任何状态（read_only），不产生用户可见副作用。
    let mut outcome = run_discovery_check(store, drift);

    // 步骤三：按剩余额度裁剪（判据 D4 在**展示层**兜底，不依赖上游候选数）
    if outcome.candidates.len() > remaining {
        outcome.candidates.truncate(remaining);
    }
    for c in &outcome.candidates {
        let gua = c
            .reason
            .drift_to_gua
            .clone()
            .unwrap_or_else(|| trigger_gua.clone());
        ledger.record_shown(&c.memory_id, &gua);
    }
    save_ledger(data_dir, &ledger);

    outcome.shown_today = ledger.shown_today;
    outcome.daily_cap = cap;
    outcome
}

/// 步骤四：登记一次用户反馈（HTTP 反馈端点的唯一调用点）。
///
/// 返回登记后的账本（供调用方回显可观测状态）。
pub fn record_feedback_and_save(
    data_dir: &Path,
    memory_id: &str,
    gua_fallback: &str,
    kind: DiscoveryFeedbackKind,
) -> DiscoveryLedger {
    let mut ledger = load_ledger(data_dir);
    ledger.roll_day_if_needed();
    ledger.record_feedback(memory_id, gua_fallback, kind);
    save_ledger(data_dir, &ledger);
    ledger
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_store::MemoryStore;
    use crate::memory_types::{Importance, Memory, MemoryType};
    use tempfile::TempDir;

    fn make_store() -> (TempDir, MemoryStore<JsonPersistence>) {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = crate::persistence::create_json_persistence(&data_dir).expect("应成功创建");
        (dir, MemoryStore::new(p))
    }

    /// 构造测试用漂移响应。
    ///
    /// `query_text` 显式置 None → 走**向后兼容回退**（用卦名），
    /// 用于验证"旧 daemon 无翻译字段时行为不退化"。
    /// 需要测方案C 语义文本的用例请单独构造事件。
    fn drift_with(gua: &str, before: &str, mag: f32) -> DriftStateResponse {
        DriftStateResponse {
            last_drift: mag,
            drift_total: mag,
            threshold: 0.35,
            explore_beats: 1,
            active_gua: Some(gua.to_string()),
            active_palace: None,
            pending_events: vec![StateDriftEvent {
                timestamp: 0,
                drift_magnitude: mag,
                dominant_gua_before: Some(before.to_string()),
                dominant_gua_after: Some(gua.to_string()),
                query_text: None,
            }],
        }
    }

    /// 无漂移事件 → 不制造空检查（触发源强度为零时不该有任何动作）。
    #[test]
    fn no_drift_events_produces_no_candidates() {
        let (_d, mut store) = make_store();
        let drift = DriftStateResponse {
            last_drift: 0.0,
            drift_total: 0.0,
            threshold: 0.35,
            explore_beats: 0,
            active_gua: None,
            active_palace: None,
            pending_events: vec![],
        };
        let out = run_discovery_check(&mut store, &drift);
        assert!(out.executed);
        assert_eq!(out.skip_reason.as_deref(), Some("no_drift_events"));
        assert!(out.candidates.is_empty());
    }

    /// 判据校验：相关度不足 / 未访问天数不足 都必须被过滤（防止事后放宽门槛）。
    #[test]
    fn is_valid_candidate_enforces_both_thresholds() {
        // 相关度达标 + 天数达标 → 有效
        assert!(is_valid_candidate(0.5, 10, 0.35, 7));
        // 相关度不足 → 无效
        assert!(!is_valid_candidate(0.34, 10, 0.35, 7));
        // 天数不足 → 无效
        assert!(!is_valid_candidate(0.5, 6, 0.35, 7));
        // 恰好等于门槛（含等号，与 PREREG "≥" 一致）→ 有效
        assert!(is_valid_candidate(0.35, 7, 0.35, 7));
    }

    /// M9 门禁有效性：相关度归一化后，`RELEVANCE_MIN` 必须**真的能筛掉**低分候选。
    ///
    /// 背景（实测驱动）：`recall` 返回原始 TF-IDF 分（64~75 量级），若直接用
    /// 0.35 比较则恒通过、门槛失效。归一化到 0-1 后才有意义。
    /// 本测试构造"池内分数差异大"的场景，断言低分候选被过滤。
    #[test]
    fn relevance_gate_can_actually_filter_after_normalization() {
        // 模拟 recall 输出：top=75（归一化 1.0），尾部=10（归一化 0.133）
        let top = 75.0f32;
        let tail = 10.0f32;
        let rel_tail = (tail / top).clamp(0.0, 1.0);
        assert!(
            rel_tail < 0.35,
            "尾部候选归一化后应低于门槛（实际 {}）",
            rel_tail
        );
        assert!(
            !is_valid_candidate(rel_tail, 30, 0.35, 7),
            "归一化前用原始分 10.0 也会通过 0.35 —— 必须归一化才使门槛有效"
        );
        // 对照：同一个尾部候选，用**原始分**比较会误判为通过（这就是修正前的缺陷）
        assert!(
            is_valid_candidate(tail, 30, 0.35, 7),
            "对照前提：原始分尺度下 0.35 门槛恒通过（修正前的真实缺陷）"
        );
    }

    /// 只读检索（D3 零伤害的核心机制）：read_only 不写回状态机快照。
    ///
    /// 判据：连续两次 read_only recall 后，活跃状态快照与检索前逐字节一致。
    /// 若这里失败，说明发现通道会污染用户查询路径的活性偏置——D3 必然不成立。
    #[test]
    fn read_only_recall_does_not_touch_state_machine() {
        let (_d, mut store) = make_store();
        store
            .remember(Memory::new(
                "离".to_string(),
                MemoryType::Fact,
                None,
                vec![],
                Importance::default(),
                None,
            ))
            .expect("应成功记住");

        let before = store.memory_state_machine.snapshot();
        let mut filter = RecallFilter::new();
        filter.read_only = true;
        let _ = store.recall("离", &filter).expect("只读检索应成功");
        let after = store.memory_state_machine.snapshot();

        assert_eq!(
            format!("{:?}", before),
            format!("{:?}", after),
            "read_only 检索不得写入状态机快照（否则 D3 零伤害承诺不成立）"
        );
    }

    /// M9 门禁有效性对照：证明 `read_only_recall_does_not_touch_state_machine`
    /// **不是空洞通过**——同一个 store、同一个查询，**非** read_only 的检索
    /// 确实会改变状态机。若本测试失败，则上一条测试的"无变化"结论不可信
    /// （说明该场景下本来就不会写状态，测不出 read_only 的作用）。
    #[test]
    fn normal_recall_does_touch_state_machine_contrast() {
        let (_d, mut store) = make_store();
        store
            .remember(Memory::new(
                "离".to_string(),
                MemoryType::Fact,
                None,
                vec![],
                Importance::default(),
                None,
            ))
            .expect("应成功记住");

        let before = store.memory_state_machine.snapshot();
        let _ = store
            .recall("离", &RecallFilter::new())
            .expect("检索应成功");
        let after = store.memory_state_machine.snapshot();

        assert_ne!(
            format!("{:?}", before),
            format!("{:?}", after),
            "对照前提：普通检索本应改变状态机；否则 read_only 门禁测不出差异"
        );
    }

    /// D4 无轰炸承诺：账本每日额度用尽后拒绝新提示。
    #[test]
    fn ledger_daily_cap_blocks_further_shows() {
        let mut ledger = DiscoveryLedger::default();
        ledger.roll_day_if_needed();
        let cap = discovery_daily_cap();
        for i in 0..cap {
            assert!(ledger.remaining_today() > 0, "第 {} 条应仍有余量", i + 1);
            ledger.record_shown(&format!("m{}", i), "离");
        }
        assert_eq!(ledger.remaining_today(), 0, "额度用尽后余量必须为 0");
        assert_eq!(ledger.shown_today, cap);
    }

    /// 步骤四：连续忽略达阈值 → 该卦被降频；点击一次即重置。
    #[test]
    fn feedback_backflow_suppresses_after_repeated_ignores() {
        let mut ledger = DiscoveryLedger::default();
        ledger.roll_day_if_needed();
        let decay = discovery_ignore_decay();

        // 连续忽略至阈值
        for i in 0..decay {
            ledger.record_shown(&format!("m{}", i), "离");
            ledger.record_feedback(&format!("m{}", i), "", DiscoveryFeedbackKind::Ignored);
        }
        assert!(
            ledger.is_suppressed("离"),
            "连续忽略 {} 次后该卦应被降频",
            decay
        );

        // 一次点击 → 正向信号重置降频
        ledger.record_shown("m-last", "离");
        ledger.record_feedback("m-last", "", DiscoveryFeedbackKind::Clicked);
        assert!(!ledger.is_suppressed("离"), "点击后降频状态应被重置");
    }

    /// 步骤四：待响应表缺失时用传入的 gua 兜底（不静默丢弃用户反馈）。
    #[test]
    fn feedback_uses_gua_fallback_when_pending_missing() {
        let mut ledger = DiscoveryLedger::default();
        ledger.roll_day_if_needed();
        ledger.record_feedback("unknown-id", "坎", DiscoveryFeedbackKind::NotInterested);
        let entry = ledger.by_gua.get("坎").expect("应由 fallback 卦记入账本");
        assert_eq!(entry.not_interested_total, 1);
        assert_eq!(entry.ignored_streak, 1, "显式负向信号应计入忽略强度");
    }

    /// 门控关闭时 run_discovery_cycle 必须直接跳过（默认关，绝不误开）。
    #[test]
    fn cycle_skips_when_gate_off() {
        let (dir, mut store) = make_store();
        let drift = drift_with("离", "坎", 0.5);
        // 门控默认关闭（除非外部显式设置环境变量）
        assert!(!active_discovery_enabled(), "测试前提：门控默认关");
        let out = run_discovery_cycle(&mut store, &drift, dir.path());
        assert!(!out.executed);
        assert_eq!(out.skip_reason.as_deref(), Some("gate_off"));
        assert!(out.candidates.is_empty());
    }

    /// 判据解耦（v1.0 实现修正）：D4 额度耗尽时**仍必须**产出 `produced`，
    /// 否则 D1（发现率）会被 D4（展示纪律）人为压低，两个判据耦合。
    ///
    /// 构造：把账本预置为"今日额度已满"，然后跑一次 cycle——
    /// 断言 `candidates` 被 D4 裁到 0（展示纪律生效），
    /// 但 `produced` 与"额度充足时"同口径（机制能力不受展示纪律影响）。
    #[test]
    fn d4_cap_does_not_suppress_d1_production() {
        let (dir, mut store) = make_store();
        // 预置一条"很久没看"的记忆，使之有机会成为候选
        let mut m = Memory::new(
            "离离状态相关的旧记忆内容".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::default(),
            None,
        );
        m.last_accessed = chrono::Utc::now() - chrono::Duration::days(30);
        store.remember(m).expect("应成功记住");

        let drift = drift_with("离", "坎", 0.9);

        // 额度充足时：记录基准产出
        let baseline = run_discovery_check(&mut store, &drift);
        // M9 门禁有效性：若基准产出为 0，则"额度耗尽不影响 produced"是**空洞成立**
        // （0 == 0），测不出耦合。必须先证明本构造确实能产出候选。
        assert!(
            !baseline.candidates.is_empty(),
            "M9 前提：本构造必须能产出至少 1 条候选，否则 D1/D4 解耦断言无意义"
        );

        // 额度吃满：账本预置 shown_today = cap
        let mut ledger = DiscoveryLedger::default();
        ledger.roll_day_if_needed();
        ledger.shown_today = discovery_daily_cap();
        save_ledger(dir.path(), &ledger);

        // 门控在本测试内显式打开（避免依赖外部环境）
        std::env::set_var("LRC_ACTIVE_DISCOVERY", "1");
        let capped = run_discovery_cycle(&mut store, &drift, dir.path());
        std::env::remove_var("LRC_ACTIVE_DISCOVERY");

        assert!(
            capped.candidates.is_empty(),
            "D4：额度耗尽后不得再展示新提示"
        );
        assert_eq!(
            capped.produced.len(),
            baseline.candidates.len(),
            "D1：额度耗尽不得影响 produced 口径（否则 D1 被 D4 人为压低）"
        );
    }

    /// **D3 零伤害承诺的直接验证**（PREREG §3.1 最强形式）：
    /// 开关主动发现（并真实执行一次发现检查）后，用户查询的返回结果
    /// 与从未开启时**逐字节一致**。
    ///
    /// 这是端到端的判据验证，而非机制验证——即使 read_only 写对了，
    /// 若发现路径以别的方式（全局缓存、指标、日志）间接影响了排序，
    /// 本测试也会失败。
    ///
    /// 构造：两个**完全独立**的 store（各自临时目录），加载同一份语料。
    ///   组 A：只跑用户查询（模拟主动发现关闭）
    ///   组 B：先跑主动发现检查（read_only），再跑同一用户查询
    /// 断言两组用户查询的 (id, score) 序列完全相同。
    #[test]
    fn d3_user_query_results_identical_with_and_without_discovery() {
        let corpus = [
            "今晚想吃火锅，上次念叨的那家海底捞一直还没去",
            "冰箱里有半盒鸡蛋两个西红柿，实在不行就下碗面",
            "楼下新开那家日料周三前会员打八折",
            "上回吃太辣第二天胃不舒服，买了盒达喜",
            "从小家里晚饭都固定一荤一汤，习惯了",
        ];
        let query = "今晚吃什么好呢";

        let build = || {
            let (dir, mut store) = make_store();
            for c in corpus {
                store
                    .remember(Memory::new(
                        c.to_string(),
                        MemoryType::Fact,
                        None,
                        vec![],
                        Importance::default(),
                        None,
                    ))
                    .expect("应成功记住");
            }
            (dir, store)
        };

        let (_da, mut store_a) = build();
        let (_db, mut store_b) = build();

        // 签名以**内容 + 分数位模式**为键，不用 memory_id——
        // 两个独立 store 各自生成随机 UUID，id 天然不同，用 id 比较是构造错误
        // （会恒失败且测不出真正的排序/分数差异）。
        let run_user_queries = |s: &mut MemoryStore<JsonPersistence>| {
            let mut out = Vec::new();
            for q in [query, "晚饭怎么解决", query] {
                let r = s.recall(q, &RecallFilter::new()).expect("检索应成功");
                let sig: Vec<(String, u32)> = r
                    .memories
                    .iter()
                    .zip(r.scores.iter())
                    .map(|(m, sc)| (m.content.clone(), sc.to_bits()))
                    .collect();
                out.push(sig);
            }
            out
        };

        let sig_a = run_user_queries(&mut store_a);

        // 组 B：先做一次真实主动发现检查（read_only），再跑同样的用户查询
        let drift = drift_with("离", "坎", 0.9);
        let _ = run_discovery_check(&mut store_b, &drift);
        let sig_b = run_user_queries(&mut store_b);

        assert_eq!(
            sig_a, sig_b,
            "D3 零伤害承诺失败：开启主动发现后用户查询结果发生变化（含分数位模式）"
        );
    }

    /// **构造有效性检验**（PREREG §3.7，从 §3.5.5 事故中新增的**前置门禁**）：
    /// 换触发源**必须**改变候选集——否则说明触发源根本没进入检索链路，
    /// 实验无效（而非"机制无贡献"）。
    ///
    /// 背景：v1.0 首轮实验把"卦名 + 长上下文"并置成一条查询，短卦名被淹没，
    /// 导致换任何触发源候选都不变（B≡C），直到 NO-GO 之后才被发现。
    /// 本测试把该检验**前移为门禁**：任何后续改动若再次让触发源失效，此处会红。
    ///
    /// 断言：同一上下文、同一记忆库下，两个**不同**触发源查询
    /// （方案C 产出的语义文本）必须给出不同的候选集。
    #[test]
    fn trigger_source_must_change_candidates() {
        let (_d, mut store) = make_store();
        // 两类记忆分别呼应两个不同的触发源语义（取自方案C 的词典词）
        for c in [
            "排查线上报错：服务异常崩溃，需要看 traceback 定位失败原因",
            "危险与困境：这条记录讲的是陷入低谷时的情绪",
            "准备汇报材料：需要做数据分析与统计报告",
            "沟通交流：会议讨论中如何表达与谈判",
        ] {
            let mut m = Memory::new(
                c.to_string(),
                MemoryType::Fact,
                None,
                vec![],
                Importance::default(),
                None,
            );
            m.last_accessed = chrono::Utc::now() - chrono::Duration::days(30);
            store.remember(m).expect("应成功记住");
        }

        // 两个触发源：方案C 翻译出的语义文本（坎→报错域；兑→交流域）
        let drift_a = DriftStateResponse {
            last_drift: 0.9,
            drift_total: 0.9,
            threshold: 0.32,
            explore_beats: 1,
            active_gua: Some("坎".to_string()),
            active_palace: None,
            pending_events: vec![StateDriftEvent {
                timestamp: 0,
                drift_magnitude: 0.9,
                dominant_gua_before: Some("兑".to_string()),
                dominant_gua_after: Some("坎".to_string()),
                query_text: Some("危险 困境 艰难".to_string()),
            }],
        };
        let drift_b = DriftStateResponse {
            active_gua: Some("兑".to_string()),
            pending_events: vec![StateDriftEvent {
                timestamp: 0,
                drift_magnitude: 0.9,
                dominant_gua_before: Some("坎".to_string()),
                dominant_gua_after: Some("兑".to_string()),
                query_text: Some("喜悦 交流 沟通".to_string()),
            }],
            ..drift_a.clone()
        };

        // 建立活跃上下文（模拟"用户最近在想什么"），使两路检索都真实生效。
        // 这也是 M9 对照（LRC_M9_LEGACY_CONCAT）能起作用的前提——否则
        // build_context_query 返回 None，拼接退化，测不出淹没问题。
        let _ = store.recall("沟通交流 会议讨论", &RecallFilter::new());

        let out_a = run_discovery_check(&mut store, &drift_a);
        let out_b = run_discovery_check(&mut store, &drift_b);
        assert!(
            !out_a.produced.is_empty() && !out_b.produced.is_empty(),
            "构造有效性前提：两个触发源都必须产出候选（否则差异断言无意义）"
        );
        let ids_a: Vec<&str> = out_a
            .produced
            .iter()
            .map(|c| c.memory_id.as_str())
            .collect();
        let ids_b: Vec<&str> = out_b
            .produced
            .iter()
            .map(|c| c.memory_id.as_str())
            .collect();

        // **必须比较无序集合**（PREREG §3.8.7 教训，口径升级）：
        // 初版门禁比较有序序列，而实测（temp/p7-probe13.py）证明
        // **分数/顺序变化会伪装成"集合变化"**——签名里含 relevance_score 时，
        // 8 个语义完全不同的触发源被判为"3 种不同候选"，从而误判"构造有效"。
        // 实际上那些触发源产出的是**同一批记忆，只是 RRF 重标定让分数不同**。
        // 只有当**集合成员**本身改变，才说明触发源真的进入了检索链路。
        let mut set_a = ids_a.clone();
        let mut set_b = ids_b.clone();
        set_a.sort_unstable();
        set_b.sort_unstable();
        assert_ne!(
            set_a, set_b,
            "构造有效性检验失败：换触发源后候选**无序集合**未改变 → 触发源未进入检索链路，\
             实验无效。注意：仅顺序/分数变化不算有效（见 PREREG §3.8.7）。\
             有序对比：a={ids_a:?} b={ids_b:?}"
        );
    }

    /// 旧缺陷回归守卫（§3.5.5）：**并置拼接**会让短触发源信号被长上下文淹没。
    ///
    /// 本测试用底层 `recall` 直接复现该现象（不经过 discovery 的新实现），
    /// 作为"为什么必须分路检索"的**证据留档**，防止有人把新实现改回拼接式。
    #[test]
    fn legacy_concatenated_query_drowns_short_signal() {
        let (_d, mut store) = make_store();
        for c in ["离相关的内容：今晚想吃火锅", "坎相关的内容：明天要去爬山"]
        {
            let mut m = Memory::new(
                c.to_string(),
                MemoryType::Fact,
                None,
                vec![],
                Importance::default(),
                None,
            );
            m.last_accessed = chrono::Utc::now() - chrono::Duration::days(30);
            store.remember(m).expect("应成功记住");
        }
        let mut f = RecallFilter::new();
        f.read_only = true;
        f.top_k = 2;

        // 短信号单独成查询 → 能区分
        let solo_li = store.recall("离", &f).expect("检索应成功");
        let solo_kan = store.recall("坎", &f).expect("检索应成功");
        assert_ne!(
            solo_li.memories.first().map(|m| m.content.clone()),
            solo_kan.memories.first().map(|m| m.content.clone()),
            "前提：短信号单独成查询时应能区分候选"
        );

        // 并置长上下文 → 信号被淹没，两者趋同（这正是首轮实验的失效机制）
        let ctx = "今晚想吃火锅 冰箱里有半盒鸡蛋 楼下新开那家日料 外卖起送三十 \
                   上回吃太辣胃不舒服 商场停车两小时 她收藏了居酒屋 周末包了饺子";
        let c_li = store
            .recall(&format!("离 {}", ctx), &f)
            .expect("检索应成功");
        let c_kan = store
            .recall(&format!("坎 {}", ctx), &f)
            .expect("检索应成功");
        let sig_li: Vec<String> = c_li.memories.iter().map(|m| m.content.clone()).collect();
        let sig_kan: Vec<String> = c_kan.memories.iter().map(|m| m.content.clone()).collect();
        assert_eq!(
            sig_li, sig_kan,
            "并置长上下文后短信号应被淹没（若不再淹没，可考虑简化探索查询实现）"
        );
    }

    /// 账本落盘/读取往返（含跨进程可见性：同目录重读内容一致）。
    #[test]
    fn ledger_roundtrip_persists() {
        let dir = TempDir::new().expect("应创建临时目录");
        let mut ledger = DiscoveryLedger::default();
        ledger.roll_day_if_needed();
        ledger.record_shown("m1", "震");
        save_ledger(dir.path(), &ledger);

        let loaded = load_ledger(dir.path());
        assert_eq!(loaded.shown_today, 1);
        assert_eq!(loaded.pending_gua.get("m1").map(|s| s.as_str()), Some("震"));
    }

    /// 账本文件缺失/损坏 → 静默降级为空账本（不得让发现功能因磁盘问题报错）。
    #[test]
    fn ledger_load_degrades_silently() {
        let dir = TempDir::new().expect("应创建临时目录");
        // 完全不存在的文件
        assert_eq!(load_ledger(dir.path()).shown_today, 0);
        // 损坏内容
        std::fs::write(ledger_path(dir.path()), "{ not json").expect("应写入");
        assert_eq!(load_ledger(dir.path()).shown_today, 0);
    }
}
