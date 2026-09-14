// ============================================================
// 许可证: Apache 2.0
// 本文件实现状态驱动发现通道，属于公开层 (Layer 1)。
// ============================================================
//
// 状态驱动发现 — 用道体状态**直接**匹配记忆（v2.0，不经文本翻译层）
//
// 与既有 `discovery.rs`（P7 文本触发）的区别（**类别差别**）：
//   - `discovery.rs`：把卦象翻译成**文本**（语义词 + 方向短语），走 TF-IDF 检索。
//     实测否证（PREREG §3.8）：抽象状态词与具体记忆语料的词面交集近乎为零，
//     触发源在检索层等价于空。
//   - 本模块：**跳过文本层**，用道体 8 维母卦分布 ↔ 记忆的 `bagua_index`
//     （同为 0-7 先天八卦索引）做**同体系向量匹配**。
//     这是对 §3.8.6「必须换中介」结论的直接落实。
//
// 三条硬约束（继承 principles，违反即为方向回退）：
//   原则一：**不参与排序** —— 本模块绝不写回任何排序状态，也不调用
//           `recall`（连只读检索都不调，纯遍历 + 向量点积）。
//   原则二：提示必须可忽略 —— 输出仅为候选列表，由前端以非阻塞卡片展示。
//   原则三：结果必须有依据 —— 每条候选携带 reason（状态卦 + 匹配度 + 时间跨度）。
//
// License 边界：产品侧只消费不计算 —— 本模块只**消费** daemon 产出的
// `gua_distribution`（8 维已聚合分布），不内置任何引擎或词典。
// daemon 不可达 → 全链路静默降级（不产生候选），行为与未引入本模块时一致。
//
// ---------------------------------------------------------------------------
// **实测边界声明（2026-09-14，v2.0 接入后实测，必须如实保留）**
// ---------------------------------------------------------------------------
// 本通道的匹配是「道体 8 维分布 ↔ 记忆 `bagua_index`」。链路本身已验证可用
// （temp/v2-probe5.py：换状态 → 候选确实改变；候选内容与卦象无词面交集，
// 证明未走文本检索）。**但实测同时暴露一个决定效果上限的既有约束**：
//
//   · 记忆侧 `bagua_index` 由 `mirror_project(luoshu_encoder.encode_text())` 得出。
//   · 默认构建（未开 `ml` feature）使用的是**统计编码器**
//     （`luoshu_encoder.rs::extract_9_features`），其 9 维特征仅含
//     「字符密度 / 字符熵 / 位置权重」——**与语义无关**。
//   · 后果：长度相近的中文短句产出几乎相同的洛书向量 → 落入同一母卦。
//     实测（temp/v2-probe4.py）：语义差异极大的 14 条文本（火锅/量子/婚礼/
//     猫/法律/诗歌/架构约束）**只落到 2 个母卦**；用户真实库 4450 条中
//     **96.8% 是同一个母卦**。
//
// 这意味着：本通道在**默认构建**下的实际区分度，受限于右侧标签的固有塌缩 ——
// 状态再怎么变，也只能在"有记忆的那 1~2 个卦"之间选择。
// 该约束**不在本模块内**，而是既有洛书编码器的特性；本模块不改动它
// （避免为凑效果去调编码器，那会污染既有检索行为）。
//
// 因此：本通道的定位是**已接通、可分阶段观察**的能力。其区分度上限来自
// 右侧标签的聚合粒度，**不在本模块内**，也**不得在本模块内通过调参"修好"**
// （那会污染既有检索行为，且属于"为凑效果改判据"）。
// 观察期若要判断"候选少"是否等于"机制无效"，必须先核对：
//   ① 记忆库中实际覆盖了几个母卦（用户真实库实测仅 1~2 个）；
//   ② 该卦下是否存在满足"≥7 天未访问"的记忆。
// 两个前提任一不成立，候选少是**数据分布**的结果，不是机制失效。
//
// **注**：`/v1/encode` 端点用的是 `HybridLuoShuEncoder::default()`，
// 它**不加载 ML 模型**（只有 `new_with_ml` 才加载）；而 store 内的编码器
// 在 ml feature + `--mode smart` 下才可能加载 ML。
// 故用 `/v1/encode` 探针测"ML 是否改善分散度"是**无效的**（两侧不同源）。
// ===========================================================================

use serde::{Deserialize, Serialize};

use crate::memory_store::MemoryStore;
use crate::memory_types::Memory;
use std::path::Path;

/// 状态驱动发现门控（**默认关**，与 LRC_ACTIVE_DISCOVERY 独立）。
///
/// 关闭时，本模块的全部入口立即返回跳过态，LRC 行为与 v0.9.7 完全一致。
/// 运行期实时读取（与 `active_discovery_enabled` 同款约定）。
pub fn state_driven_enabled() -> bool {
    std::env::var("LRC_STATE_DRIVEN_DISCOVERY")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// 匹配度阈值默认值（v2.0 §步骤二建议 0.6）。
pub const STATE_MATCH_MIN_DEFAULT: f32 = 0.6;

/// 未访问天数下限默认值（与 `discovery.rs` 的 stale_days 语义一致）。
pub const STATE_STALE_DAYS_DEFAULT: i64 = 7;

/// 单次最多产出的候选数（防轰炸，与 D4 的上限默认值一致）。
pub const STATE_MAX_CANDIDATES: usize = 3;

/// 读取匹配度阈值（环境变量可覆盖，运行期实时读取）。
pub fn state_match_min() -> f32 {
    std::env::var("LRC_STATE_MATCH_MIN")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0)
        .unwrap_or(STATE_MATCH_MIN_DEFAULT)
}

/// 读取未访问天数下限。
pub fn state_stale_days() -> i64 {
    std::env::var("LRC_STATE_STALE_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(STATE_STALE_DAYS_DEFAULT)
}

/// 道体状态快照（由 daemon `GET /state/snapshot` 返回，`snapshot` 字段）。
///
/// 契约与 daoti 研究资产侧 `make_state_snapshot` 一一对应。
/// 字段全部 `#[serde(default)]`：daemon 版本差异时缺字段不导致解析失败
/// （缺 `gua_distribution` 时匹配自然产出为空 —— 保守降级，不误报）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StateSnapshot {
    /// 快照时间戳（毫秒，daemon 侧生成）
    #[serde(default)]
    pub timestamp: i64,
    /// 道体原生主导卦（64 卦体系，如 "大有"）——仅用于展示缘由
    #[serde(default)]
    pub dominant_gua: Option<String>,
    /// 8 母卦主导卦（LRC 顺序，如 "坎"）——匹配的主信号
    #[serde(default)]
    pub dominant_bagua: Option<String>,
    /// **8 维母卦分布**（已按 LRC 八卦顺序：乾兑离震巽坎艮坤）
    #[serde(default)]
    pub gua_distribution: Vec<f32>,
    /// 探索场总质量（未归一化）——冷启动的辅助判据
    #[serde(default)]
    pub gua_mass: f32,
    /// 最近漂移量
    #[serde(default)]
    pub drift_magnitude: f32,
    /// 探索节拍累计
    #[serde(default)]
    pub explore_beats: i64,
    /// 状态积累天数（daemon 侧按首条快照推算）
    #[serde(default)]
    pub state_age_days: f32,
    /// 冷启动标记（true = 状态未积累够，不应生成提示）
    #[serde(default)]
    pub warmup: bool,
}

/// daemon `GET /state/snapshot` 的响应体。
#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotResponse {
    #[serde(default)]
    pub snapshot: Option<StateSnapshot>,
}

/// 一条状态驱动候选（步骤三的输出单元）。
#[derive(Debug, Clone, Serialize)]
pub struct StateMatchCandidate {
    pub memory_id: String,
    /// 内容摘要（按 char 边界截断）
    pub content_preview: String,
    /// 匹配度（0-1；8 维分布与该记忆母卦的对齐强度）
    pub match_score: f32,
    /// 距上次访问天数
    pub days_since_last_access: i64,
    /// 该记忆的母卦索引（0-7）与名称，便于用户核对依据
    pub bagua_index: u8,
    pub bagua_name: String,
    /// 人类可读依据（原则三）
    pub reason: StateMatchReason,
}

/// 候选的可解释依据（原则三：结果必须有依据）。
#[derive(Debug, Clone, Serialize)]
pub struct StateMatchReason {
    /// 状态主导母卦（LRC 名称，如 "坎·水"）
    pub trigger_bagua: String,
    /// 该记忆所属母卦
    pub memory_bagua: String,
    /// 道体原生主导卦（64 卦，如 "大有"）
    pub daoti_gua: Option<String>,
    pub days_since_last_access: i64,
    /// 中文缘由（前端直接展示）
    pub human_readable: String,
}

/// 一次状态驱动发现的完整产出。
#[derive(Debug, Clone, Serialize)]
pub struct StateDrivenOutcome {
    /// 是否真的执行（门控关 / daemon 不可达 / 冷启动 → false）
    pub executed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    pub candidates: Vec<StateMatchCandidate>,
    /// 被判据过滤掉的候选数（可观测性）
    pub filtered_out: usize,
    pub match_min_used: f32,
    pub stale_days_used: i64,
    /// 本日已展示条数（复用 discovery 账本，D4 同口径）
    pub shown_today: usize,
    pub daily_cap: usize,
    /// 状态是否处于冷启动期（前端可据此隐藏卡片）
    pub warmup: bool,
    /// 本次使用的状态主导卦（可观测性）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_bagua: Option<String>,
}

/// 内容摘要截断长度（字符数，与 discovery.rs 保持一致的观感）。
const PREVIEW_MAX_CHARS: usize = 80;

/// LRC 侧先天八卦名称（顺序必须与 `mirror_trapezoid::BAGUA_NAMES` 一致）。
///
/// **顺序即契约**：daemon 侧 `LRC_BAGUA_ORDER` 与本数组一一对应。
/// 若两侧顺序不同，8 维分布会整体错位（`bagua_name_to_index` 的注释同源风险）。
pub const LRC_BAGUA_NAMES: [&str; 8] = [
    "乾·天", "兑·泽", "离·火", "震·雷", "巽·风", "坎·水", "艮·山", "坤·地",
];

/// 构造基础结果（统一填默认字段）。
fn base_outcome(executed: bool, skip_reason: Option<&str>) -> StateDrivenOutcome {
    StateDrivenOutcome {
        executed,
        skip_reason: skip_reason.map(|s| s.to_string()),
        candidates: Vec::new(),
        filtered_out: 0,
        match_min_used: state_match_min(),
        stale_days_used: state_stale_days(),
        shown_today: 0,
        daily_cap: crate::discovery::discovery_daily_cap(),
        warmup: false,
        trigger_bagua: None,
    }
}

/// 标记未执行（门控关 / daemon 不可达 / 冷启动）。
pub fn skipped_outcome(reason: &str) -> StateDrivenOutcome {
    base_outcome(false, Some(reason))
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

/// 计算一条记忆与状态分布的**匹配度**（0-1）。
///
/// 口径（**为什么用"该记忆母卦上的分布质量"而非余弦**）：
///   记忆侧只有**单个离散标签** `bagua_index`（无分布向量），
///   余弦需要两向量，而 here 只有一边是向量 —— 强行构造 one-hot 再算余弦
///   等价于直接取该维分量，徒增复杂度。
///   故口径取 `dist[mem_bagua]`：**状态在该卦上的归一化质量**。
///   语义直白："当前状态有多大比例落在该记忆所属的卦上"。
///   阈值 0.6 的含义即"状态有 60% 以上集中在该卦"（8 卦均分时为 0.125，
///   故 0.6 是显著聚焦，不会轻易满足 —— 避免"处处都匹配"的退化为恒真）。
pub fn match_score_of(distribution: &[f32], mem_bagua: u8) -> f32 {
    if distribution.len() != 8 {
        return 0.0;
    }
    let idx = mem_bagua as usize;
    if idx >= distribution.len() {
        return 0.0;
    }
    distribution[idx].clamp(0.0, 1.0)
}

/// 判断记忆是否可作为候选（PREREG 同款判据：未访问天数）。
///
/// 与 `discovery.rs::is_valid_candidate` 的差异：相关度判据在此处已由
/// `match_score_of` 承担（匹配度阈值），故本函数只管时间跨度。
fn passes_stale(days_since_access: i64, stale_days: i64) -> bool {
    days_since_access >= stale_days
}

/// **核心匹配逻辑**：对一组记忆做状态匹配（不做任何 I/O，纯函数）。
///
/// 抽出本函数的原因（可测试性 + 契约清晰）：
///   - 调用方（`match_by_state`）只负责"取记忆"，匹配规则完全在此；
///   - 测试可直接注入**任意形态**的记忆（包括 `bagua_index = None`、
///     指定时间的记忆），无需绕开 `remember` 对卦标签的重算。
///
/// **不用 recall、不做文本检索**（原则一 + §3.8.6「换中介」）——
/// 只遍历 + 读 `bagua_index` 字段。
pub fn match_memories_core(memories: &[Memory], snapshot: &StateSnapshot) -> StateDrivenOutcome {
    // 冷启动保护（v2.0 §6.4）：状态未积累够 → 不生成提示。
    // 依据两个信号（任一成立即跳过）：daemon 的 warmup 标记、状态质量过低。
    if snapshot.warmup {
        let mut out = base_outcome(true, Some("warmup"));
        out.warmup = true;
        return out;
    }
    if snapshot.gua_distribution.len() != 8 {
        return base_outcome(true, Some("no_state_distribution"));
    }
    if snapshot.gua_mass <= 1e-9 {
        let mut out = base_outcome(true, Some("state_mass_zero"));
        out.warmup = true;
        return out;
    }

    let match_min = state_match_min();
    let stale_days = state_stale_days();
    let now = chrono::Utc::now();

    // 状态主导母卦（供展示与缘由；与分布的 argmax 一致）
    let trigger_idx = snapshot
        .gua_distribution
        .iter()
        .enumerate()
        .fold(
            (0usize, f32::MIN),
            |acc, (i, v)| {
                if *v > acc.1 {
                    (i, *v)
                } else {
                    acc
                }
            },
        )
        .0;
    let trigger_name = LRC_BAGUA_NAMES
        .get(trigger_idx)
        .copied()
        .unwrap_or("未知")
        .to_string();

    let mut scored: Vec<(f32, i64, u8, &Memory)> = Vec::new();
    let mut filtered_out = 0usize;
    for m in memories {
        if m.is_expired() {
            continue;
        }
        // **必须已有卦象标签**：无标签的记忆无法在不经文本检索的前提下匹配
        // （这正是本通道的设计边界 —— 宁可漏，不可退化回文本匹配）。
        let Some(mem_bagua) = m.bagua_index else {
            filtered_out += 1;
            continue;
        };
        let days = (now - m.last_accessed).num_days();
        let score = match_score_of(&snapshot.gua_distribution, mem_bagua);
        if score < match_min || !passes_stale(days, stale_days) {
            filtered_out += 1;
            continue;
        }
        scored.push((score, days, mem_bagua, m));
    }

    // 按匹配度降序；同分时优先"更久未访问"（对用户信息增益更大）
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))
    });

    let mut candidates = Vec::new();
    for (score, days, mem_bagua, m) in scored.into_iter().take(STATE_MAX_CANDIDATES) {
        let mem_name = LRC_BAGUA_NAMES
            .get(mem_bagua as usize)
            .copied()
            .unwrap_or("未知")
            .to_string();
        let human = format!(
            "你当前的状态集中在「{}」，这条 {} 天前的记忆也属于「{}」，故想起它",
            trigger_name, days, mem_name
        );
        candidates.push(StateMatchCandidate {
            memory_id: m.id.clone(),
            content_preview: preview_of(&m.content),
            match_score: score,
            days_since_last_access: days,
            bagua_index: mem_bagua,
            bagua_name: mem_name.clone(),
            reason: StateMatchReason {
                trigger_bagua: trigger_name.clone(),
                memory_bagua: mem_name,
                daoti_gua: snapshot.dominant_gua.clone(),
                days_since_last_access: days,
                human_readable: human,
            },
        });
    }

    StateDrivenOutcome {
        executed: true,
        skip_reason: None,
        candidates,
        filtered_out,
        match_min_used: match_min,
        stale_days_used: stale_days,
        shown_today: 0,
        daily_cap: crate::discovery::discovery_daily_cap(),
        warmup: false,
        trigger_bagua: Some(trigger_name),
    }
}

/// 用道体状态快照匹配记忆（从记忆库取数据后委托给 `match_memories_core`）。
///
/// 参数：
///   - `store`：记忆库（**只读**使用；仅 `list_memories` 遍历 + 字段读取）
///   - `snapshot`：道体状态快照
pub fn match_by_state<P>(store: &MemoryStore<P>, snapshot: &StateSnapshot) -> StateDrivenOutcome
where
    P: crate::persistence::Persistence,
{
    // 门控前置：关闭时连遍历都不做（行为与未引入本模块完全一致）
    if !state_driven_enabled() {
        return skipped_outcome("gate_off");
    }
    // 遍历全部记忆（只读）。**不用 recall**：本通道刻意不经文本检索
    // （原则一 + §3.8.6「换中介」的直接落实）。
    let filter = crate::memory_store_types::ListFilter::new();
    let Ok((all, _total)) = store.list_memories(&filter) else {
        return base_outcome(true, Some("list_failed"));
    };
    match_memories_core(&all, snapshot)
}

/// 从 daemon 拉取状态快照（超时 2s，失败返回 None —— 静默降级）。
///
/// 复用既有 daemon 通信模式（与 `discovery::fetch_drift_state` 同款客户端配置）。
pub async fn fetch_state_snapshot() -> Option<StateSnapshot> {
    let base =
        std::env::var("DAOTI_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3222".to_string());
    let url = format!("{}/state/snapshot", base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<SnapshotResponse>().await.ok()?.snapshot
}

/// 步骤三 + 步骤四的**编排入口**（HTTP 层唯一调用点）。
///
/// 顺序（都不触碰排序路径）：
///   1. 门控关闭 → 跳过
///   2. 匹配（步骤二）
///   3. 复用 discovery 账本：每日额度裁剪 + 降频跳过（步骤三/四）
pub fn run_state_driven_cycle<P>(
    store: &MemoryStore<P>,
    snapshot: &StateSnapshot,
    data_dir: &Path,
) -> StateDrivenOutcome
where
    P: crate::persistence::Persistence,
{
    if !state_driven_enabled() {
        return skipped_outcome("gate_off");
    }

    let mut outcome = match_by_state(store, snapshot);
    if !outcome.executed {
        return outcome;
    }

    // 步骤四：若状态主导卦因"连续忽略"被降频 → 本轮不展示
    // （复用既有账本，**不新增第二套反馈机制**）
    let mut ledger = crate::discovery::load_ledger(data_dir);
    ledger.roll_day_if_needed();
    let cap = crate::discovery::discovery_daily_cap();

    let trigger_gua = outcome.trigger_bagua.clone().unwrap_or_default();
    if ledger.is_suppressed(&trigger_gua) {
        let mut out = skipped_outcome("suppressed_by_feedback");
        out.shown_today = ledger.shown_today;
        out.daily_cap = cap;
        return out;
    }

    // 步骤三：按剩余额度裁剪（D4 同口径：每日上限，跨通道共享计数）
    let remaining = ledger.remaining_today();
    if outcome.candidates.len() > remaining {
        outcome.candidates.truncate(remaining);
    }
    for c in &outcome.candidates {
        ledger.record_shown(&c.memory_id, &trigger_gua);
    }
    crate::discovery::save_ledger(data_dir, &ledger);

    outcome.shown_today = ledger.shown_today;
    outcome.daily_cap = cap;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_store::MemoryStore;
    use crate::memory_types::{Importance, Memory, MemoryType};
    use tempfile::TempDir;

    fn make_store() -> (TempDir, MemoryStore<crate::JsonPersistence>) {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = crate::persistence::create_json_persistence(&data_dir).expect("应成功创建");
        (dir, MemoryStore::new(p))
    }

    /// 环境变量类测试的串行锁。
    ///
    /// **为什么需要**：`cargo test` 默认多线程并行跑用例，而读门控/阈值的
    /// 用例都要改**进程级**环境变量 —— 并发时会互相覆盖（实测：
    /// `gate_off_skips_without_touching_store` 因另一个用例设了 gate=1 而假失败）。
    /// 故所有改环境变量的用例先取本锁，把它们串行化。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 构造一条指定母卦、指定未访问天数的记忆。
    ///
    /// **为什么不走 `store.remember`**：`remember` 会按内容重算
    /// `bagua_index`/`luoshu_vector`（生产上正确的行为），会把测试预设的
    /// 卦标签覆盖掉，导致"想测坎卦却拿到随机卦"的假失败。
    /// 故测试直接构造 `Memory` 并走纯函数 `match_memories_core`。
    fn mem_with_bagua(id: &str, content: &str, bagua: Option<u8>, days: i64) -> Memory {
        let mut m = Memory::new(
            content.to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::default(),
            None,
        );
        m.id = id.to_string();
        m.bagua_index = bagua;
        m.luoshu_vector = Some([0.1; 9]);
        m.last_accessed = chrono::Utc::now() - chrono::Duration::days(days);
        m
    }

    /// 构造以某母卦为主导的分布（该卦占比 `share`，其余均分）。
    fn dist_focused(idx: usize, share: f32) -> Vec<f32> {
        let rest = (1.0 - share) / 7.0;
        (0..8)
            .map(|i| if i == idx { share } else { rest })
            .collect()
    }

    fn snapshot_with(dist: Vec<f32>, warmup: bool) -> StateSnapshot {
        StateSnapshot {
            timestamp: 0,
            dominant_gua: Some("大有".to_string()),
            dominant_bagua: Some("乾".to_string()),
            gua_distribution: dist,
            gua_mass: 1.0,
            drift_magnitude: 0.4,
            explore_beats: 100,
            state_age_days: 10.0,
            warmup,
        }
    }

    // ---- 门控 ----

    #[test]
    fn gate_off_skips_without_touching_store() {
        let _g = env_guard();
        std::env::remove_var("LRC_STATE_DRIVEN_DISCOVERY");
        let (dir, store) = make_store();
        let snap = snapshot_with(dist_focused(0, 0.9), false);
        let out = run_state_driven_cycle(&store, &snap, dir.path());
        assert!(!out.executed);
        assert_eq!(out.skip_reason.as_deref(), Some("gate_off"));
        assert!(out.candidates.is_empty());
    }

    // ---- 冷启动保护（v2.0 §6.4）----

    #[test]
    fn warmup_produces_no_candidates() {
        let mems = vec![mem_with_bagua("m1", "状态卦相关记忆", Some(5), 30)];
        let snap = snapshot_with(dist_focused(5, 0.9), true);
        let out = match_memories_core(&mems, &snap);
        assert!(out.executed, "应执行（但冷启动应跳过产出）");
        assert_eq!(out.skip_reason.as_deref(), Some("warmup"));
        assert!(out.candidates.is_empty(), "冷启动期不得产出候选");
        assert!(out.warmup);
    }

    #[test]
    fn zero_state_mass_produces_no_candidates() {
        let mems = vec![mem_with_bagua("m1", "内容", Some(5), 30)];
        let mut snap = snapshot_with(vec![0.0; 8], false);
        snap.gua_mass = 0.0;
        let out = match_memories_core(&mems, &snap);
        assert_eq!(out.skip_reason.as_deref(), Some("state_mass_zero"));
        assert!(out.candidates.is_empty());
    }

    #[test]
    fn wrong_dimension_produces_no_candidates() {
        let mems = vec![mem_with_bagua("m1", "内容", Some(5), 30)];
        let snap = snapshot_with(vec![0.5, 0.5], false);
        let out = match_memories_core(&mems, &snap);
        assert_eq!(out.skip_reason.as_deref(), Some("no_state_distribution"));
        assert!(out.candidates.is_empty());
    }

    // ---- 核心匹配（M9：门禁必须能失败）----

    /// **状态驱动发现的核心门禁**：换状态分布必须换候选集。
    ///
    /// 若换成另一个母卦主导，候选**必须**改变 —— 否则说明匹配根本没生效
    /// （与 P7 实验的教训一致：验证"触发源确实进入了匹配"）。
    #[test]
    fn changing_state_must_change_candidates() {
        let mems = vec![
            mem_with_bagua("kan-mem", "坎域的记忆", Some(5), 30),
            mem_with_bagua("dui-mem", "兑域的记忆", Some(1), 30),
        ];
        let snap_kan = snapshot_with(dist_focused(5, 0.9), false);
        let snap_dui = snapshot_with(dist_focused(1, 0.9), false);
        let out_kan = match_memories_core(&mems, &snap_kan);
        let out_dui = match_memories_core(&mems, &snap_dui);

        let ids_kan: Vec<&str> = out_kan
            .candidates
            .iter()
            .map(|c| c.memory_id.as_str())
            .collect();
        let ids_dui: Vec<&str> = out_dui
            .candidates
            .iter()
            .map(|c| c.memory_id.as_str())
            .collect();
        assert!(
            !ids_kan.is_empty(),
            "坎状态应产出坎域候选（构造有效性前提）"
        );
        assert!(
            !ids_dui.is_empty(),
            "兑状态应产出兑域候选（构造有效性前提）"
        );
        assert_ne!(
            ids_kan, ids_dui,
            "状态驱动匹配失效：换状态分布后候选未改变 → 匹配未生效。\
             kan={ids_kan:?} dui={ids_dui:?}"
        );
        assert_eq!(ids_kan, vec!["kan-mem"], "坎状态应只命中坎域记忆");
        assert_eq!(ids_dui, vec!["dui-mem"], "兑状态应只命中兑域记忆");
    }

    /// 阈值必须真的能过滤（M9：防"阈值恒被满足"）。
    ///
    /// 用一个"几乎均匀"的分布（各卦 0.125），在阈值 0.6 下**必须**全被过滤；
    /// 再把阈值降到 0.1，同样的分布**必须**产出候选 —— 证明阈值确实生效。
    #[test]
    fn match_threshold_can_actually_filter() {
        let _g = env_guard();
        let mems = vec![mem_with_bagua("m1", "某域记忆", Some(5), 30)];
        let uniform = vec![0.125f32; 8];

        std::env::set_var("LRC_STATE_MATCH_MIN", "0.6");
        let out_hi = match_memories_core(&mems, &snapshot_with(uniform.clone(), false));
        assert!(
            out_hi.candidates.is_empty(),
            "均匀分布（各 0.125）在阈值 0.6 下必须全被过滤，实际 {} 条",
            out_hi.candidates.len()
        );
        assert!(out_hi.filtered_out >= 1, "应记录被过滤数");

        std::env::set_var("LRC_STATE_MATCH_MIN", "0.1");
        let out_lo = match_memories_core(&mems, &snapshot_with(uniform, false));
        assert!(
            !out_lo.candidates.is_empty(),
            "阈值降到 0.1 后必须产出候选（证明阈值真的在起作用）"
        );
        std::env::remove_var("LRC_STATE_MATCH_MIN");
    }

    #[test]
    fn days_since_access_gate_blocks_recent_memory() {
        // 1 天前访问过 → 不满足 ≥7 天
        let mems = vec![mem_with_bagua("recent", "刚看过的记忆", Some(5), 1)];
        let out = match_memories_core(&mems, &snapshot_with(dist_focused(5, 0.9), false));
        assert!(out.candidates.is_empty(), "近期访问过的记忆不得作为候选");
        assert!(out.filtered_out >= 1);
    }

    #[test]
    fn memory_without_bagua_is_skipped_not_text_matched() {
        let mems = vec![
            mem_with_bagua("tagged", "有卦标签的记忆", Some(5), 30),
            mem_with_bagua("untagged", "无卦标签但内容含关键词的记忆", None, 30),
        ];
        let out = match_memories_core(&mems, &snapshot_with(dist_focused(5, 0.9), false));
        let ids: Vec<&str> = out
            .candidates
            .iter()
            .map(|c| c.memory_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["tagged"],
            "无卦标签的记忆应被跳过（不得退化为文本匹配）"
        );
    }

    #[test]
    fn candidate_carries_human_readable_reason() {
        let mems = vec![mem_with_bagua("m1", "坎域记忆", Some(5), 12)];
        let out = match_memories_core(&mems, &snapshot_with(dist_focused(5, 0.9), false));
        let c = out.candidates.first().expect("应有候选");
        assert!(
            !c.reason.human_readable.is_empty(),
            "原则三：候选必须携带可读依据"
        );
        assert_eq!(c.reason.memory_bagua, "坎·水");
        assert_eq!(c.reason.trigger_bagua, "坎·水");
        assert_eq!(c.days_since_last_access, 12);
        assert_eq!(c.reason.daoti_gua.as_deref(), Some("大有"));
    }

    #[test]
    fn candidates_sorted_by_match_then_staleness() {
        // 同卦（坎）三条：分布只在该卦上有质量，故匹配度相同 → 应按"更久未访问"排。
        // 三条的未访问天数都须 ≥ 门槛（默认 7 天），否则会被 stale 判据先行过滤。
        let mems = vec![
            mem_with_bagua("a", "坎域记忆", Some(5), 8),
            mem_with_bagua("b", "坎域记忆", Some(5), 40),
            mem_with_bagua("c", "坎域记忆", Some(5), 20),
        ];
        let out = match_memories_core(&mems, &snapshot_with(dist_focused(5, 0.9), false));
        let days: Vec<i64> = out
            .candidates
            .iter()
            .map(|c| c.days_since_last_access)
            .collect();
        assert_eq!(
            days,
            vec![40, 20, 8],
            "同分时应优先更久未访问（信息增益更大）"
        );
    }

    #[test]
    fn max_candidates_is_capped() {
        let mems: Vec<Memory> = (0..6)
            .map(|i| mem_with_bagua(&format!("m{i}"), "坎域记忆", Some(5), 30))
            .collect();
        let out = match_memories_core(&mems, &snapshot_with(dist_focused(5, 0.9), false));
        assert_eq!(
            out.candidates.len(),
            STATE_MAX_CANDIDATES,
            "单次产出不得超过上限（防轰炸）"
        );
    }

    #[test]
    fn match_score_reads_correct_dimension() {
        let dist = dist_focused(3, 0.8);
        assert!((match_score_of(&dist, 3) - 0.8).abs() < 1e-6);
        // 非聚焦维 = 其余均分 (1-0.8)/7
        assert!((match_score_of(&dist, 0) - (0.2 / 7.0)).abs() < 1e-6);
        // 维度不符 → 0（保守降级）
        assert_eq!(match_score_of(&[0.5, 0.5], 0), 0.0);
        assert_eq!(match_score_of(&dist, 99), 0.0);
    }

    /// 原则一验证：本通道**不调用 recall**，故不会写回状态机活跃锚点。
    ///
    /// 对照实验：先记录状态机活跃集合，跑一次状态驱动匹配，再核对活跃集合
    /// **逐字节未变** —— 这与 `discovery::read_only` 的目标一致，
    /// 但本模块连 recall 都不调用，故是更强的保证。
    #[test]
    fn does_not_touch_state_machine_contrast() {
        let _g = env_guard();
        std::env::set_var("LRC_STATE_DRIVEN_DISCOVERY", "1");
        let (dir, mut store) = make_store();
        store
            .remember(mem_with_bagua("m1", "坎域记忆", None, 30))
            .expect("应成功记住");
        // 先用一次正常 recall 建立活跃上下文（否则活跃集本就为空，对照无意义）
        let _ = store.recall("坎域记忆", &crate::memory_store_types::RecallFilter::new());
        let before = store.memory_state_machine.active_ids(64);

        let snap = snapshot_with(dist_focused(5, 0.9), false);
        let _ = run_state_driven_cycle(&store, &snap, dir.path());

        let after = store.memory_state_machine.active_ids(64);
        assert_eq!(
            before, after,
            "原则一：状态驱动匹配绝不写回状态机（本模块不调用 recall）"
        );
        std::env::remove_var("LRC_STATE_DRIVEN_DISCOVERY");
    }

    #[test]
    fn legacy_daemon_without_distribution_degrades_silently() {
        let mems = vec![mem_with_bagua("m1", "内容", Some(5), 30)];
        // 模拟旧 daemon：无 gua_distribution 字段（serde default → 空 vec）
        let snap = StateSnapshot {
            gua_distribution: Vec::new(),
            gua_mass: 0.0,
            ..Default::default()
        };
        let out = match_memories_core(&mems, &snap);
        assert!(out.candidates.is_empty(), "缺字段应静默降级，不误报候选");
    }
}
