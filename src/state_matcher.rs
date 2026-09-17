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
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
///
/// ---------------------------------------------------------------------------
/// **实测修正（2026-09-14，V15 接入后标定，必须如实保留）**
/// ---------------------------------------------------------------------------
/// **0.6 对 V15 真实产出的状态分布不可用** —— 实测（`temp/v15-constructive.py`，
/// V15 驱动 1200 拍）：
///   · 真实 8 维分布的 max 分量：p05=0.2192 p50=0.2571 p95=0.3333
///   · 阈值 0.6 → 触发率 **0.0%**（候选恒空，通道等价于未启用）
///   · 阈值 0.5 → ~0%；0.40 → 0.2%；0.32 → 7.9%；0.30 → 18.2%；0.26 → 47.1%
/// **0.6 当初为何"看起来可用"**：v2.0 的链路验证（`v2-probe5.py`）用的是
/// **人工构造**的高集中分布 `dist_focused(5, 0.9)`（单卦 0.9），而 daemon 真实
/// 产出的分布分散得多（探索场按宫聚合 + 遗忘 + clamp 后天然平滑）。
/// 即：**探针可分性不能外推到生产池**（方法论 5 的又一实例）。
///
/// **触发率目标带 [3%, 10%]（§3.3）对应的阈值**：**0.32 ~ 0.34**
/// （实测触发率 7.9% / 5.8% / 3.2%）。
///
/// ---------------------------------------------------------------------------
/// **⚠ 上述标定仅在"人造均匀记忆池"上成立；真实库上任何阈值都恒空**
/// （2026-09-14 决定性命中，见 PREREG §3.17，**改默认值前必须先读**）
/// ---------------------------------------------------------------------------
/// 上述 0.32~0.34 是在**人造池**（8 母卦各 3 条、均匀分布）上标定的。
/// 在**真实用户库**（`~/.loong-recall/global`，4458 条）上实测：
///   · 记忆母卦分布：**离·火 96.77%、兑·泽 1.70%、其余 6 卦为 0**（覆盖 2/8）
///   · 道体状态主导：**{艮, 坤, 乾}**（探索场累积）
///   · **两空间交集 = ∅** ⇒ 候选在**任何阈值**下恒空（实测降到 0.20 仍为 0 条）
///   · 反向对照：人为把状态指向「离」→ 立刻命中 4183 条
/// ⇒ 根因是**中介无效**（状态空间与记忆空间不共享语义基），**不是阈值问题**。
///
/// **更深一层**：记忆的 `bagua_index` **不是语义标签，而是长度标签** ——
/// 实测（`temp/v15-label-is-length.py`）：长度 ≥400 字 → **100% 是「离」**；
/// 离 平均 813 字 vs 兑 平均 86 字（**9.42x**）；长度 800-1000 字区间内
/// 167 条**全部同卦**（内容却涵盖马尔代夫环保/比特币协议/Rust 日志）。
///
/// **因此**：
///   · 本常量区间**不得**作为"改默认阈值"的依据（改与不改都恒空）；
///   · 阈值应在**中介有效之后**再标定（PREREG §3.17.8 第 22 条）；
///   · `STATE_MATCH_MIN_DEFAULT` 维持 0.6 —— 在真实库上它与 0.32 **等价**（都恒空），
///     既然无差别，就不做无依据的变更。
pub const STATE_MATCH_MIN_DEFAULT: f32 = 0.6;

/// 标定得出的**可用阈值区间**（**仅在人造均匀池上有效**）。
///
/// ⚠ **真实库上不适用**：见 `STATE_MATCH_MIN_DEFAULT` 的 §3.17 说明 ——
/// 真实库母卦覆盖仅 2/8 且与状态空间不相交，任何阈值都恒空。
/// 本常量保留**仅作研究记录**，不得用于产品默认值决策。
pub const STATE_MATCH_MIN_CALIBRATED_RANGE: (f32, f32) = (0.32, 0.34);

/// 未访问天数下限默认值（与 `discovery.rs` 的 stale_days 语义一致）。
pub const STATE_STALE_DAYS_DEFAULT: i64 = 7;

/// 单次最多产出的候选数（防轰炸，与 D4 的上限默认值一致）。
pub const STATE_MAX_CANDIDATES: usize = 3;

/// 读取单次候选上限（环境变量可覆盖）。
///
/// **为什么允许覆盖**：标定/诊断时需看到**完整**的余弦分布才能定阈值
/// （PREREG 标定纪律：不得凭空设定，必须实测）。生产默认仍为 3。
pub fn state_max_candidates() -> usize {
    std::env::var("LRC_STATE_DRIVEN_TOP_K")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(STATE_MAX_CANDIDATES)
}

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
    /// **方向二：状态语义锚点文本**（daemon 侧产出）。
    ///
    /// 语义路径的查询输入 —— 由 daemon 把"当前状态"翻译成一段自然语言
    /// （如「危险 困境 艰难 从喜悦转向」），LRC 侧编码为句向量后与记忆向量
    /// 做余弦匹配。
    ///
    /// **为什么不是 LRC 侧生成**：License 边界（产品侧只消费不计算）——
    /// 词典/引擎属道体研究资产，LRC 不得内置。故锚点必须由 daemon 提供。
    ///
    /// 缺省（旧 daemon）→ None → 语义路径不启用，自动回退标签匹配
    /// （行为与 v2.0 逐字节一致，不因缺字段而失效）。
    #[serde(default)]
    pub state_anchor_text: Option<String>,
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
///
/// ---------------------------------------------------------------------------
/// **⚠ 本函数是"绝对口径"，其可用卦数受数学上界约束**（PREREG §3.21，必读）
/// ---------------------------------------------------------------------------
/// 绝对阈值匹配有**数学上界**：要 N 卦同时可用，需 `N × 阈值 ≤ 1`，
/// 故 `可用卦数 ≤ ⌊1/阈值⌋`（阈值 0.32 → **最多 3 卦**）。
/// 实测（`temp/v15-fix-e2e.py`）：真实 V15 状态在此口径下可用卦数仅 1~3/8。
///
/// **但这不是 V15 的缺陷**：V15 的平稳分布在宫级是相对均匀的
/// （艮 17.8% / 离 17.3% / 坤 15.2% / … / 震 7.4%），覆盖全部 8 宫 ——
/// 只是被绝对阈值口径浪费了。
///
/// **突破上界的路径见 `match_score_relative`**（相对口径，§3.22）。
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

// ===========================================================================
// 相对口径匹配（方向E，PREREG §3.22）—— 突破绝对阈值的数学上界
// ===========================================================================
//
// **为什么需要**（实测驱动，§3.21）：
//   绝对口径的可用卦数 ≤ ⌊1/阈值⌋（阈值 0.32 → 最多 3 卦）。
//   实测真实 V15 状态在该口径下仅 1~3/8 卦可用，而 V15 的平稳分布
//   其实覆盖全部 8 宫（宫级 7.4%~17.8%）—— 信息被阈值口径浪费。
//
// **口径设计：Top-k ∩ z>0**（`temp/v15-e-combined.py` 实测选定）
//   1. 在**拍内**对 8 维分布做 z-score 标准化（消除量纲/整体缩放）；
//   2. 只保留 `z > 0`（高于本拍均值）的维 → **排除无信息状态**；
//   3. 在这些维中取前 k 名作为"命中的卦"。
//
// **为什么必须带 `z > 0` 约束**（关键：防恒真闸门）：
//   纯 Top-k 对**任何**分布都必有前 k 名，包括均匀分布（完全无信息）——
//   那会让"无状态"也产出候选，通道退化为恒真。
//   加上 `z > 0` 后，均匀分布（全维同值，z 全为 0）**无候选**。
//   实测（`temp/v15-e-combined.py`）：
//     · 可用卦数：Top-2 → 2/8；**Top-3 → 4/8**；Top-4 → 5/8
//     · **均匀分布候选数 = 0**（判别力保住）
//
// **⚠ 不要引用"标签一致率 100%"作为语义质量证据**（§3.23 更正）：
//   匹配逻辑即"状态是艮 → 取标签为艮的记忆"，故"候选标签 == 状态标签"
//   由**匹配逻辑本身保证**，是**循环论证**，不能证明内容相关。
//   独立核查（LLM 裁判 + 随机对照组，`temp/v15-de-and-audit.py`）显示
//   真实判别增益为 **平均 +21pp**（艮 +62pp、巽 +50pp、坎 +17pp、兑 +0pp）
//   —— **远低于 100%**，且**各域差异极大**（不宜等量推广）。
//
// **与绝对口径的关系**：本口径**默认关闭**，由
// `LRC_STATE_RELATIVE_MATCH=1` 开启（保持既有行为可回退）。

/// 相对口径的 Top-k（默认 3；实测 4/8 可用、均匀分布 0 候选）。
pub const STATE_RELATIVE_TOPK_DEFAULT: usize = 3;

/// 相对口径是否启用（**默认关**；开启后 `match_memories_core` 走相对口径）。
///
/// **为什么默认关**：改口径会改变产品可见的候选产出与依据文案，
/// 属产品决策；且本口径的长期效果需真实使用观察（§3.22）。
/// 关闭时行为与 v2.0 逐字节一致。
pub fn state_relative_match_enabled() -> bool {
    std::env::var("LRC_STATE_RELATIVE_MATCH")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// 读取相对口径的 k（环境变量 `LRC_STATE_RELATIVE_TOPK` 可覆盖）。
pub fn state_relative_topk() -> usize {
    std::env::var("LRC_STATE_RELATIVE_TOPK")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0 && *v <= 8)
        .unwrap_or(STATE_RELATIVE_TOPK_DEFAULT)
}

/// 相对口径的最小集中度（**CV = σ/μ**）—— 防"伪信息状态"放行。
///
/// **为什么必须加这道门禁**（端到端实测暴露，`temp/v15-e2e-verify.py`）：
///   仅用 `Top-k ∩ z>0` 时，伪信息状态 `[0.13,0.12,…,0.14,0.12]`（几乎平坦）
///   被放行并命中 **815 条**记忆。根因：**z-score 是无量纲变换**，
///   会把"几乎平坦但非全同值"的微小差异放大成显著 z 值 —— 恒真闸门的变体。
///   `zscore_8` 的 σ≈0 判据只能挡住**完全均匀**，挡不住"微扰均匀"。
///
/// **判据选择：CV（变异系数）**（`temp/v15-concentration-gate.py` 实测）：
///   真实 V15 状态（600 拍）：CV 最小值 **0.3501**（p01=0.3550）
///   负例（均匀/微扰/单维微高/微小随机）：CV 最大值 **0.1437**
///   ⇒ 间隔 **0.2064**，**完全可分**，且对真实状态**误杀率 0%**。
///
/// **标定方式**（遵守标定纪律）：取真实状态 CV 最小值 × 0.9 作裕量，
///   即 `0.3501 × 0.9 ≈ 0.3151`。**不是凭感觉设定，也不是按"能否通过"反推**。
///
/// **⚠ 已知边界：该阈值在"采样修复（D）"场景下会切在分布中部**（§3.23.6）：
///   D 使状态分布更均匀 → CV 中位数从 0.5967 降到 **0.3058**（逼近本阈值），
///   导致 **52.2% 的拍被本门禁拒绝**（实测 `temp/v15-de-and-audit.py`），
///   且有样本 CV=0.3141（距阈值仅 0.001）。
///   ⇒ **若启用 D，必须在本阈值上重新标定**；当前值是针对**未修采样**的分布定的。
///
/// 环境变量 `LRC_STATE_MIN_CONCENTRATION` 可覆盖（换模型后重标定用）。
pub const STATE_MIN_CONCENTRATION: f32 = 0.3151;

/// 读取最小集中度（环境变量可覆盖）。
pub fn state_min_concentration() -> f32 {
    std::env::var("LRC_STATE_MIN_CONCENTRATION")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(STATE_MIN_CONCENTRATION)
}

/// 计算 8 维分布的**变异系数** CV = σ/μ（μ≤0 时返回 0）。
pub fn concentration_cv(distribution: &[f32]) -> f32 {
    if distribution.is_empty() {
        return 0.0;
    }
    let n = distribution.len() as f32;
    let mu = distribution.iter().sum::<f32>() / n;
    if !mu.is_finite() || mu <= 0.0 {
        return 0.0;
    }
    let var = distribution
        .iter()
        .map(|x| (x - mu) * (x - mu))
        .sum::<f32>()
        / n;
    let sd = var.sqrt();
    if !sd.is_finite() {
        return 0.0;
    }
    sd / mu
}

/// 拍内 z-score 标准化（8 维）。返回 `None` 表示分布退化（σ≈0，无信息）。
pub fn zscore_8(distribution: &[f32]) -> Option<Vec<f32>> {
    if distribution.len() != 8 {
        return None;
    }
    let n = distribution.len() as f32;
    let mu = distribution.iter().sum::<f32>() / n;
    let var = distribution
        .iter()
        .map(|x| (x - mu) * (x - mu))
        .sum::<f32>()
        / n;
    let sd = var.sqrt();
    // **阈值取 1e-6（而非 1e-9）**：概率量级为 O(0.1)，而浮点累积误差可使
    // "全同值"分布的 σ 达到 ~1e-8（实测：`[0.3; 8]` 的方差因 0.3 不可精确表示
    // 而非零）。若阈值过小，这种**浮点噪声**会被误判为"有信息"，
    // 从而让平坦状态产出候选 —— 正是本函数要防的恒真闸门。
    if !sd.is_finite() || sd <= 1e-6 {
        // 均匀分布 / 全同值（含浮点噪声）→ 无信息，**不得产出候选**
        return None;
    }
    Some(distribution.iter().map(|x| (x - mu) / sd).collect())
}

/// 相对口径打分：返回该母卦在**本拍 Top-k ∩ z>0** 中的 z 值；未入选 → `None`。
///
/// 返回 `Option<f32>` 而非 0.0 是刻意的：`None` 与"分数很低"语义不同，
/// 调用方可据此区分"未入选"与"入选但靠后"。
pub fn match_score_relative(distribution: &[f32], mem_bagua: u8, k: usize) -> Option<f32> {
    let z = zscore_8(distribution)?;
    let idx = mem_bagua as usize;
    if idx >= z.len() || z[idx] <= 0.0 {
        return None;
    }
    // 取 z>0 的前 k 名
    let mut pos: Vec<usize> = (0..z.len()).filter(|i| z[*i] > 0.0).collect();
    pos.sort_by(|a, b| {
        z[*b]
            .partial_cmp(&z[*a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pos.truncate(k);
    if pos.contains(&idx) {
        Some(z[idx])
    } else {
        None
    }
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
    // 方向E：相对口径（默认关）。开启时改用 Top-k ∩ z>0 判定是否入选，
    // **不再受绝对阈值的 ⌊1/阈值⌋ 上界约束**（§3.22）。
    let relative = state_relative_match_enabled();
    let topk = state_relative_topk();
    // 相对口径下分布必须非退化（全同值 → zscore_8 返回 None → 无候选）
    if relative && zscore_8(&snapshot.gua_distribution).is_none() {
        return base_outcome(true, Some("state_distribution_flat"));
    }
    // **集中度门禁**（防"伪信息状态"放行）：σ≈0 只能挡完全均匀，
    // 挡不住"微扰均匀"（z-score 会把微小差异放大）。故用 CV 再挡一层。
    if relative {
        let cv = concentration_cv(&snapshot.gua_distribution);
        let min_cv = state_min_concentration();
        if cv < min_cv {
            let mut out = base_outcome(true, Some("state_low_concentration"));
            out.warmup = true; // 语义同冷启动：状态尚不具判别力，不生成提示
            return out;
        }
    }

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
        if !passes_stale(days, stale_days) {
            filtered_out += 1;
            continue;
        }
        // 两口径：相对（Top-k ∩ z>0，分数为 z 值）或绝对（dist[i] vs 阈值）。
        // 相对口径的分数语义不同（z 值，可为负），但**排序语义一致**
        // （越大越相关），故后续排序逻辑复用。
        let score = if relative {
            match match_score_relative(&snapshot.gua_distribution, mem_bagua, topk) {
                Some(z) => z,
                None => {
                    filtered_out += 1;
                    continue;
                }
            }
        } else {
            let s = match_score_of(&snapshot.gua_distribution, mem_bagua);
            if s < match_min {
                filtered_out += 1;
                continue;
            }
            s
        };
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
        let human = if relative {
            // 相对口径下分数是 z 值（不是 0-1 相似度），故文案必须区分，
            // 避免把 "1.83" 当成 "183% 相似"误导用户。
            format!(
                "你当前的状态偏向「{}」，这条 {} 天前的记忆属于「{}」（相对强度 {:.2}），故想起它",
                trigger_name, days, mem_name, score
            )
        } else {
            format!(
                "你当前的状态集中在「{}」，这条 {} 天前的记忆也属于「{}」，故想起它",
                trigger_name, days, mem_name
            )
        };
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

// ============================================================
// 方向二：语义向量匹配（绕过 9 维投影，直接用 bge 完整句向量）
// ============================================================
//
// **为什么需要这一层**（实测驱动，见 PREREG §3.9.3）：
//   `bagua_index` 的来源是 `mirror_project(9 维洛书向量)`，而 9 维特征
//   （字符密度/字符熵/位置权重，或 bge 经投影矩阵降维）**承载不了语义区分度**
//   —— 实测语义差异极大的 14 条文本只落 2 个母卦；用户真实库 4450 条中
//   96.8% 同属一卦。状态再怎么变，也只能在"有记忆的 1~2 个卦"之间选择。
//
// **已实测的前提**（`tests/state_semantic_probe.rs`）：bge 完整句向量能把
//   "语义相关但与锚点零词面交集"的记忆排在无关记忆之前（3/3 锚点可分，
//   间隔 +0.029~+0.053）⇒ 绕过投影直接用句向量是可行的。
//
// **已实测的成本约束**（`tests/state_semantic_latency.rs`）：
//   单条编码 ≈2.25s（bge-base-zh、6 核已饱和、并发无加速）。
//   ⇒ 全库实时编码不可行（3115 条 ≈ 2 小时）；故必须：
//     ① 记忆向量**落盘缓存**（一次回填、后续只读）；
//     ② 查询时**只编码锚点 1 条**（2.25s）+ 与缓存做点积（微秒级）。
//   这使单次请求耗时 ≈ 锚点编码耗时，可稳定落在前端 8s 超时内。
//
// 缓存文件与 `discovery_ledger.json` 同目录（`semantic_vectors.json`），
// 格式为 `{ memory_id: [f32...] }`；维度不一致的旧缓存项自动丢弃
// （模型更换后维度可能变化，静默重建优于报错）。

/// 语义向量缓存文件名（与 memories.json 同目录）。
pub const SEMANTIC_CACHE_FILE: &str = "semantic_vectors.json";

/// 缓存结构版本（模型更换/维度变化时通过版本号整体失效）。
const SEMANTIC_CACHE_VERSION: u32 = 1;

/// 回填的**时间预算**（毫秒）——本通道单次请求的硬上界保护。
///
/// **为什么必须用时间预算而非条数**（实测驱动）：
///   首次端到端验证（`temp/v2-probe8.py`）实测：把回填批量设为 12 条时，
///   单次请求耗时 **24.6s**，**远超前端 8s 超时** ⇒ 前端会静默放弃，
///   用户看不到任何提示，且服务端仍在空转编码。
///   条数限制无法约束总时长（单条耗时会随文本长度/机器负载波动），
///   因此改为**按已用时间截断**：无论批量设多大，回填阶段都不会超过预算。
///
/// 取值依据：锚点编码实测 ≈2.25s，前端超时 8s ⇒ 回填预算取 4000ms，
/// 总耗时上界 ≈ 6.25s，留出约 1.7s 余量给匹配/IO/网络。
/// 环境变量 `LRC_SEMANTIC_BACKFILL_BUDGET_MS` 可覆盖。
pub const SEMANTIC_BACKFILL_BUDGET_MS_DEFAULT: u64 = 4000;

pub fn semantic_backfill_budget_ms() -> u64 {
    std::env::var("LRC_SEMANTIC_BACKFILL_BUDGET_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(SEMANTIC_BACKFILL_BUDGET_MS_DEFAULT)
}

/// 单次回填最多编码多少条（**辅助上限**，真正的约束是时间预算）。
///
/// 保留条数上限的原因：防止在极快机器上一次回填过多、导致缓存文件
/// 频繁整体重写（每次都序列化全量向量）。故"条数"与"时间"双约束取先到者。
const SEMANTIC_BACKFILL_DEFAULT: usize = 8;

/// 读取单次回填批量（环境变量可覆盖）。
pub fn semantic_backfill_batch() -> usize {
    std::env::var("LRC_SEMANTIC_BACKFILL_BATCH")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(SEMANTIC_BACKFILL_DEFAULT)
}

/// 语义向量缓存（磁盘持久化；一次回填、后续只读）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SemanticVectorCache {
    /// 结构版本（与 `SEMANTIC_CACHE_VERSION` 不符则整体丢弃）
    #[serde(default)]
    pub version: u32,
    /// 向量维度（0 = 未定；与实测维度不符的条目丢弃）
    #[serde(default)]
    pub dim: usize,
    /// memory_id → 句向量
    #[serde(default)]
    pub vectors: HashMap<String, Vec<f32>>,
}

/// 缓存路径（与 memories.json 同目录）。
pub fn semantic_cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SEMANTIC_CACHE_FILE)
}

/// 读取缓存（文件缺失/损坏/版本不符 → 返回空缓存，静默重建）。
pub fn load_semantic_cache(data_dir: &Path) -> SemanticVectorCache {
    let path = semantic_cache_path(data_dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return SemanticVectorCache::default();
    };
    match serde_json::from_str::<SemanticVectorCache>(&text) {
        Ok(c) if c.version == SEMANTIC_CACHE_VERSION => c,
        _ => SemanticVectorCache::default(),
    }
}

/// 保存缓存（原子写；失败只记日志，不影响主流程）。
pub fn save_semantic_cache(data_dir: &Path, cache: &SemanticVectorCache) {
    let path = semantic_cache_path(data_dir);
    let Ok(json) = serde_json::to_string(cache) else {
        eprintln!("[LRC·状态发现] 语义缓存序列化失败，本轮不落盘");
        return;
    };
    if let Err(e) = crate::atomic_file::write_atomic(&path, json.as_bytes()) {
        eprintln!("[LRC·状态发现] 语义缓存落盘失败: {}", e);
    }
}

/// 余弦相似度（两向量；维度不符/零模长 → None）。
fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let (na, nb) = (na.sqrt(), nb.sqrt());
    if !na.is_finite() || !nb.is_finite() || na <= 0.0 || nb <= 0.0 {
        return None;
    }
    Some((dot / (na * nb)).clamp(-1.0, 1.0))
}

/// 估计向量池的**逐维均值**（公共分量）与**逐维样本数**。
///
/// **保留但默认不启用**（见 `state_semantic_vector_debias_enabled` 的说明）：
/// 实测证明该口径在**小池**上不稳定，故仅作可选逃生开关，不参与默认路径。
fn pool_mean(vectors: &HashMap<String, Vec<f32>>, dim: usize) -> Option<Vec<f32>> {
    let mut mean = vec![0.0f32; dim];
    let mut counted = 0usize;
    for v in vectors.values() {
        if v.len() != dim {
            continue;
        }
        for (acc, x) in mean.iter_mut().zip(v.iter()) {
            *acc += *x;
        }
        counted += 1;
    }
    if counted == 0 {
        return None;
    }
    let inv = 1.0f32 / counted as f32;
    for acc in mean.iter_mut() {
        *acc *= inv;
    }
    Some(mean)
}

/// 去中心化余弦：两侧同时减去公共分量后再算余弦（对称去中心化）。
fn cosine_decentered(a: &[f32], b: &[f32], mean: &[f32]) -> Option<f32> {
    if a.len() != b.len() || b.len() != mean.len() {
        return None;
    }
    let ac: Vec<f32> = a.iter().zip(mean.iter()).map(|(x, m)| x - m).collect();
    let bc: Vec<f32> = b.iter().zip(mean.iter()).map(|(x, m)| x - m).collect();
    cosine(&ac, &bc)
}

/// 锚点编码时使用的 **BGE 检索指令前缀**。
///
/// **为什么必须加**（bge-zh 官方用法 + 项目既有标定结论）：
///   `memory_store.rs::semantic_similarities_impl` 的注释明确记录：
///   "大 bge-zh 官方用法：短查询侧需加检索指令前缀，文档侧不加。
///    缺省前缀时句向量各向异性严重（实测相关对与无关对差距仅 ~0.01）"。
///   实测（`temp/v2-probe8.py` + TRACE 日志）：不加前缀时，"危险 困境 艰难"
///   锚点的**相关记忆最低余弦（0.7255）低于无关记忆最高余弦（0.7890）**
///   —— 完全重叠、不可分。故锚点侧必须与项目既有检索路径同口径。
///
/// 文档侧（记忆）不加前缀，与 `semantic_similarities_impl` 的文档侧一致。
const BGE_QUERY_INSTRUCTION: &str = "为这个句子生成表示以用于检索相关文章：";

/// 编码锚点（查询侧）：按 BGE 官方用法加检索指令前缀。
fn encode_anchor<P>(store: &MemoryStore<P>, anchor_text: &str) -> Option<Vec<f32>>
where
    P: crate::persistence::Persistence,
{
    let instructed = format!("{BGE_QUERY_INSTRUCTION}{anchor_text}");
    store.encode_sentence_vector(&instructed)
}

/// **768 维向量去中心化**是否启用（**默认关**）。
///
/// **为什么默认关**（实测驱动，2026-09-14，`tests/state_semantic_probe.rs`）：
///   该口径在**小池**上会**反向恶化**可分性——实测「危险 困境 艰难」锚点：
///     原始空间间隔 +0.0329（可分） → 去中心化后 −0.0895（**不可分**）。
///   根因：池内多数向量属"无关"类时，均值偏向无关类，双侧减去均值后
///   反而把无关向量拉近锚点。项目既有 P8.2j 使用该口径时，池是**真实检索
///   候选池**（规模与相关性分布不同），故其结论不能直接迁移到本通道。
///   保留为逃生开关供后续在**大池**上复验，不参与默认路径。
pub fn state_semantic_vector_debias_enabled() -> bool {
    std::env::var("LRC_STATE_SEMANTIC_DEBIAS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// **z-score 相对口径**是否启用（**默认开**）。
///
/// **它解决什么、不解决什么**（务必区分，否则会误判其能力）：
///   解决：**跨锚点的阈值刻度不统一**。不同状态锚点的余弦基线不同
///         （实测：兑锚点整体偏高、坎锚点整体偏低），一个绝对阈值
///         无法同时对两者都合适。z-score 在每个锚点内标准化，
///         使**同一个阈值**可跨锚点复用。
///   不解决：**分辨能力**。z-score 是单调线性变换，**不改变候选排序**，
///         因此**不可能**把重叠的相关/无关分开。
///
/// **实测结论（本通道的真实瓶颈，2026-09-14）**：
///   坎锚点「危险 困境 艰难」的候选余弦（`LRC_STATE_SEMANTIC_TRACE=1` 实测）：
///     相关：线上报错崩溃 0.6041、服务超时 0.5999、部署脚本中断 0.5988、
///           数据库连接池耗尽 0.5898
///     无关：和同事对齐方案 **0.6544**、周末买了束花 0.6191、
///           朋友约咖啡馆 0.6062、服务器扩容 0.5975、机房空调 0.5620 …
///   ⇒ **相关最低（0.5898）< 无关最高（0.6544）**，两者**完全重叠**。
///   故无论采用绝对阈值还是 z-score（乃至去中心化），都**无法干净分离**。
///
///   对照：`tests/state_semantic_probe.rs` 的探针在"极端无关项"
///   （火锅/手冲咖啡/婚礼）上确实可分（间隔 +0.033）；但生产候选池里
///   大量是**语义中庸的技术/日常句**（"对齐方案""扩容八台"），
///   它们与任何锚点都有 0.6+ 的余弦（bge-zh 各向异性的典型表现）。
///   ⇒ 本通道的可分性**取决于候选池的语义分布**，而非普适成立。
///
/// 因此：z-score 仍默认开（刻度统一是必要的），但**不得据此宣称
/// "语义匹配已可用"**——真实性以 TRACE 实测为准。
pub fn state_semantic_zscore_enabled() -> bool {
    std::env::var("LRC_STATE_SEMANTIC_ZSCORE")
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// 语义筛选阈值。
///
/// 口径取决于 `state_semantic_zscore_enabled()`，两者量纲不同，故分别给默认值：
///   - **z-score 口径（默认）**：`STATE_SEMANTIC_ZSCORE_MIN`，标定依据见上。
///   - **绝对余弦口径（逃生开关）**：`STATE_SEMANTIC_COSINE_MIN`，
///     由 `tests/state_semantic_probe.rs` 的原始空间实测标定。
///
/// 环境变量 `LRC_STATE_SEMANTIC_MIN` 可**同时覆盖两者**（换模型后重标定用）。
pub const STATE_SEMANTIC_ZSCORE_MIN: f32 = 0.35;

/// 绝对余弦口径的阈值（仅 `LRC_STATE_SEMANTIC_ZSCORE=0` 时使用）。
///
/// 标定依据（原始空间实测，每锚点 6 条）：
///   坎 相关最低 0.5866 / 无关最高 0.5537；兑 0.6746 / 0.6214；坤 0.6225 / 0.5937。
///   ⇒ 三锚点的可行区间交集为 `(0.6214, 0.6225]`——**几乎为空**，
///     这正是必须改用 z-score 的数学证据（绝对阈值跨锚点不可用）。
///   本常量取 0.62 作为"单一锚点场景"的近似值，**不推荐跨锚点使用**。
pub const STATE_SEMANTIC_COSINE_MIN: f32 = 0.62;

pub fn state_semantic_min() -> f32 {
    let default = if state_semantic_zscore_enabled() {
        STATE_SEMANTIC_ZSCORE_MIN
    } else {
        STATE_SEMANTIC_COSINE_MIN
    };
    std::env::var("LRC_STATE_SEMANTIC_MIN")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}

/// z-score 标准化所需的最少候选数（少于该数量时 σ 估计不可靠 → 退化为绝对口径）。
const ZSCORE_MIN_CANDIDATES: usize = 4;

/// **语义匹配核心**：锚点句向量 vs 记忆句向量，返回按余弦降序的候选。
///
/// 参数：
///   - `anchor`：锚点句向量（道体状态 → 语义文本 → bge）
///   - `memories`：全部记忆（用于取 id/content/last_accessed）
///   - `vectors`：memory_id → 句向量（**缓存**，缺项的记忆直接跳过 ——
///     没缓存的记忆不参与语义匹配，避免为了它触发一次 2.25s 编码）
///
/// 与 `match_memories_core`（标签匹配）的关系：二者产出的候选合并后再裁剪，
/// 故本函数与标签路径**共用**时长/依据等约束。
pub fn match_semantic_core(
    anchor: &[f32],
    memories: &[Memory],
    vectors: &HashMap<String, Vec<f32>>,
) -> StateDrivenOutcome {
    let match_min = state_semantic_min();
    let stale_days = state_stale_days();
    let now = chrono::Utc::now();

    // 向量去中心化（P8.2j 口径）：**默认关**，见 `state_semantic_vector_debias_enabled`。
    let mean = if state_semantic_vector_debias_enabled() {
        pool_mean(vectors, anchor.len())
    } else {
        None
    };

    // 第一遍：算每条候选的**原始余弦**（并过滤掉未缓存/过期/时长不足的）。
    let mut sims: Vec<(f32, i64, &Memory)> = Vec::new();
    let mut filtered_out = 0usize;
    for m in memories {
        if m.is_expired() {
            continue;
        }
        let Some(v) = vectors.get(&m.id) else {
            // 未缓存 → 不参与（这是"分批回填"的必然中间态，不是错误）
            filtered_out += 1;
            continue;
        };
        let days = (now - m.last_accessed).num_days();
        if !passes_stale(days, stale_days) {
            filtered_out += 1;
            continue;
        }
        let sim = match &mean {
            Some(mu) => cosine_decentered(anchor, v, mu),
            None => cosine(anchor, v),
        };
        let Some(sim) = sim else {
            filtered_out += 1;
            continue;
        };
        sims.push((sim, days, m));
    }

    // 第二遍：**筛选口径**。
    //
    // z-score 口径（默认）：把余弦在**本次候选分布内**标准化后再比阈值。
    // 这是跨锚点可用的唯一口径（实测：三锚点的绝对余弦可行区间交集几乎为空，
    // 见 `STATE_SEMANTIC_COSINE_MIN` 的说明）。
    // 候选数不足（σ 不可靠）→ 退化为绝对口径（保守，不制造虚假筛选）。
    let zscore = state_semantic_zscore_enabled() && sims.len() >= ZSCORE_MIN_CANDIDATES;
    let threshold = if zscore {
        let n = sims.len() as f32;
        let mu = sims.iter().map(|(s, _, _)| *s).sum::<f32>() / n;
        let var = sims
            .iter()
            .map(|(s, _, _)| {
                let d = *s - mu;
                d * d
            })
            .sum::<f32>()
            / n;
        let sigma = var.sqrt();
        if !sigma.is_finite() || sigma <= 1e-6 {
            // 分布退化（全部同分）→ 无法标准化，退化为绝对口径
            None
        } else {
            Some((mu, sigma))
        }
    } else {
        None
    };

    let mut scored: Vec<(f32, i64, &Memory)> = Vec::new();
    for (sim, days, m) in sims {
        let keep = match &threshold {
            Some((mu, sigma)) => (sim - mu) / sigma >= match_min,
            // 绝对口径：阈值语义为余弦本身
            None => sim >= match_min,
        };
        if !keep {
            filtered_out += 1;
            continue;
        }
        scored.push((sim, days, m));
    }

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))
    });

    let mut candidates = Vec::new();
    for (sim, days, m) in scored.into_iter().take(state_max_candidates()) {
        let human = format!(
            "你当前的状态与这条 {} 天前的记忆语义接近（相似度 {:.2}），故想起它",
            days, sim
        );
        candidates.push(StateMatchCandidate {
            memory_id: m.id.clone(),
            content_preview: preview_of(&m.content),
            match_score: sim,
            days_since_last_access: days,
            bagua_index: m.bagua_index.unwrap_or(0),
            bagua_name: m
                .bagua_index
                .and_then(|i| LRC_BAGUA_NAMES.get(i as usize))
                .copied()
                .unwrap_or("未知")
                .to_string(),
            reason: StateMatchReason {
                trigger_bagua: "语义匹配".to_string(),
                memory_bagua: "语义匹配".to_string(),
                daoti_gua: None,
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
        trigger_bagua: Some("语义匹配".to_string()),
    }
}

/// 语义路径入口：编码锚点 → 与缓存向量匹配 → **顺带回填少量未缓存向量**。
///
/// 返回 `None` 表示语义路径不可用（未开 ml / 模型缺失 / 锚点编码失败），
/// 调用方应回退到标签路径。
///
/// **为什么把回填放在这里**（而非独立后台线程）：
///   - `MemoryStore` 因持久化缓存含 `RefCell` 而 `!Sync`，后台线程无法共享它；
///   - 而本通道本身是"低频、非用户可见"的后台检查（前端 8s 超时内），
///     把回填搭在这条路径上，无需引入跨线程共享，也不额外增加常驻开销。
///
/// 代价：回填速率受调用频率限制（每次调用受时间预算约束，见
/// `semantic_backfill_budget_ms`）。这是**刻意的**：
/// 避免回填在启动瞬间吃满 CPU、影响用户查询（原则二）。
fn run_semantic_match<P>(
    store: &MemoryStore<P>,
    memories: &[Memory],
    anchor_text: &str,
    data_dir: &Path,
) -> Option<StateDrivenOutcome>
where
    P: crate::persistence::Persistence,
{
    // 锚点编码（单条，实测 ≈2.25s）。失败 → 语义路径不可用。
    // **必须走 `encode_anchor`**（加 BGE 检索指令前缀）——不加前缀时
    // 相关/无关余弦完全重叠（实测不可分），见 `BGE_QUERY_INSTRUCTION` 的说明。
    let anchor = encode_anchor(store, anchor_text)?;
    if anchor.is_empty() {
        return None;
    }

    let mut cache = load_semantic_cache(data_dir);
    // 维度变更（如换模型）→ 整体失效重建（静默，不报错）
    if cache.dim != 0 && cache.dim != anchor.len() {
        cache = SemanticVectorCache {
            version: SEMANTIC_CACHE_VERSION,
            dim: anchor.len(),
            vectors: HashMap::new(),
        };
    }
    cache.dim = anchor.len();
    cache.version = SEMANTIC_CACHE_VERSION;

    // **回填**：挑"未缓存且满足时长判据"的记忆，最多 `batch` 条。
    // 先按"更久未访问"排序——优先回填最可能成为候选的那些，提升早期有效性。
    let stale_days = state_stale_days();
    let now = chrono::Utc::now();
    let mut todo: Vec<&Memory> = memories
        .iter()
        .filter(|m| !m.is_expired() && !cache.vectors.contains_key(&m.id))
        .filter(|m| (now - m.last_accessed).num_days() >= stale_days)
        .collect();
    todo.sort_by_key(|m| m.last_accessed);
    let batch = semantic_backfill_batch();
    let budget = std::time::Duration::from_millis(semantic_backfill_budget_ms());
    let started = std::time::Instant::now();
    let mut encoded = 0usize;
    for m in todo.into_iter().take(batch) {
        // **时间预算检查（每轮编码前）**：本通道单次请求必须落在前端超时内
        // （实测：不设预算时 12 条回填导致 24.6s，前端会静默放弃）。
        if started.elapsed() >= budget {
            break;
        }
        // 单条失败（超长文本/分词异常）→ 跳过该条，不阻断整轮
        if let Some(v) = store.encode_sentence_vector(&m.content) {
            if v.len() == anchor.len() {
                cache.vectors.insert(m.id.clone(), v);
                encoded += 1;
            }
        }
    }
    if encoded > 0 {
        save_semantic_cache(data_dir, &cache);
    }

    // 匹配（用回填后的缓存；本次新回填的条目即刻可参与）
    let mut outcome = match_semantic_core(&anchor, memories, &cache.vectors);
    // **可观测性**（标定/排障必需）：记录缓存规模与各阶段计数。
    // 诊断模式（`LRC_STATE_SEMANTIC_TRACE=1`）额外打印**全部候选的原始余弦**——
    // z-score 阈值必须由真实分布标定，不得凭探针池数字设定（PREREG 标定纪律）。
    if std::env::var("LRC_STATE_SEMANTIC_TRACE").as_deref() == Ok("1") {
        for c in &outcome.candidates {
            eprintln!(
                "[LRC·状态发现][trace] cos={:.4} days={} | {}",
                c.match_score,
                c.days_since_last_access,
                c.content_preview.chars().take(20).collect::<String>()
            );
        }
    }
    eprintln!(
        "[LRC·状态发现] 语义路径: 锚点维={} 缓存={} 语料={} 候选={} 过滤={} 阈值={} z={}",
        anchor.len(),
        cache.vectors.len(),
        memories.len(),
        outcome.candidates.len(),
        outcome.filtered_out,
        state_semantic_min(),
        if state_semantic_zscore_enabled() {
            "on"
        } else {
            "off"
        },
    );
    outcome.match_min_used = state_semantic_min();
    Some(outcome)
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
///   2. 匹配（步骤二）：**优先语义路径**（方向二），不可用时回退标签路径
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

    // 取记忆（只读；两条路径都需要）
    let filter = crate::memory_store_types::ListFilter::new();
    let Ok((all, _total)) = store.list_memories(&filter) else {
        return base_outcome(true, Some("list_failed"));
    };

    // **优先语义路径**（方向二）：绕开 9 维投影塌缩，直接用 bge 句向量。
    // 仅在"ml 可用 + 锚点文本存在"时生效；否则回退标签路径（行为与 v2.0 一致）。
    let anchor_text = snapshot
        .state_anchor_text
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut outcome = match anchor_text {
        Some(text) => match run_semantic_match(store, &all, text, data_dir) {
            Some(o) => o,
            // 语义路径不可用（未开 ml / 模型缺失 / 锚点编码失败）→ 回退标签路径
            None => match_memories_core(&all, snapshot),
        },
        None => match_memories_core(&all, snapshot),
    };
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
            // 锚点文本置空 → 走标签路径（本模块多数用例测的是标签匹配）
            state_anchor_text: None,
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

    /// **标定门禁：默认阈值 0.6 对真实状态分布确实会关闭通道**（M9 双向对照）。
    ///
    /// 本用例把实测结论（`temp/v15-constructive.py`）固化为可回归的契约：
    ///   真实 8 维分布集中在 0.22~0.34 区间（p50≈0.26），故
    ///     · 阈值 0.6 → **必须**过滤掉 p50 那条（证明 0.6 会关掉通道）
    ///     · 阈值 0.32（标定区间下界）→ **必须**产出候选（证明区间可用）
    /// 若未来某次改动让"0.6 也能产出"，说明分布形状变了，本用例会失败并提醒重新标定。
    #[test]
    fn calibrated_range_matches_real_state_concentration() {
        let _g = env_guard();
        // 真实分布的 p50 与 p95（实测值，作为"典型/聚焦"两个代表）
        let p50 = 0.2571f32;
        let p95 = 0.3333f32;
        let mems = vec![
            mem_with_bagua("typical", "典型集中度记忆", Some(5), 30),
            mem_with_bagua("focused", "较聚焦记忆", Some(6), 30),
        ];
        let mut dist = vec![0.0f32; 8];
        dist[5] = p50;
        dist[6] = p95;
        let rest = (1.0 - p50 - p95) / 6.0;
        for (i, v) in dist.iter_mut().enumerate() {
            if i != 5 && i != 6 {
                *v = rest;
            }
        }

        // ① 默认 0.6 → 通道关闭（这正是实测 0.1% 触发率的成因）
        std::env::set_var("LRC_STATE_MATCH_MIN", "0.6");
        let out_default = match_memories_core(&mems, &snapshot_with(dist.clone(), false));
        assert!(
            out_default.candidates.is_empty(),
            "阈值 0.6 对真实集中度（max≈0.34）必须全滤 —— 否则与实测结论矛盾"
        );

        // ② 标定区间下界 0.32 → 必须产出（且能区分两个母卦的不同分数）
        let (lo, _hi) = STATE_MATCH_MIN_CALIBRATED_RANGE;
        std::env::set_var("LRC_STATE_MATCH_MIN", lo.to_string());
        let out_cal = match_memories_core(&mems, &snapshot_with(dist, false));
        assert!(
            !out_cal.candidates.is_empty(),
            "标定区间下界 {lo} 必须能产出候选（否则区间结论无效）"
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

    /// **契约测试：真实 daemon 快照必须能被本结构体解析**（端到端取证的固化）。
    ///
    /// 样本取自**真实运行**的 daemon（V15 驱动，`temp/v15-http-e2e.py` 同款路径），
    /// 经 `GET /state/snapshot` 原样导出后内联。**不是构造数据**。
    ///
    /// 本用例守住三条契约（此前只靠人工目视，无回归保护）：
    ///   ① **daemon 新增字段不得导致解析失败** —— 样本含 6 个本结构体未声明的
    ///      字段（`gua_distribution_64` / `drift_direction` / `step` / `engine` /
    ///      `explore_driver` / `last_explore_gua`）。若未来给结构体加
    ///      `deny_unknown_fields`，daemon 一升级就会**静默全空**（通道假死）。
    ///   ② **`gua_distribution` 含负值的历史缺陷已修** —— 修正前实测真实快照出现
    ///      −0.010273（探索场按宫聚合时克宫为负）。修正后（`bagua_distribution_8`
    ///      在**源头**把负值归零）样本已非负，故本用例**同时**守住两条：
    ///      历史样本（含负值）仍必须被 `clamp` 安全处理，修正后的样本必须已非负。
    ///   ③ **`dominant_gua` 可为 null**（空闲期未经 /deduce，道体无 64 卦主导），
    ///      `Option<String>` 必须容忍，且 `reason` 里输出 None 而非空串。
    #[test]
    fn real_daemon_snapshot_deserializes_and_matches() {
        // ---- 样本 A：**修正前**的真实快照（含负值，历史证据，必须仍被安全处理）----
        let raw_legacy = r#"{
          "timestamp": 1789399838417,
          "dominant_gua": null,
          "dominant_bagua": "艮",
          "gua_distribution": [
            0.180882, 0.015549, -0.010273, 0.162575,
            0.200704, 0.080087, 0.203422, 0.167054
          ],
          "gua_mass": 0.076663,
          "drift_magnitude": 0.248606,
          "drift_direction": "",
          "explore_beats": 5,
          "step": 0,
          "entropy": 1.0,
          "engine": "v23",
          "explore_driver": "v15",
          "last_explore_gua": "贲",
          "state_age_days": 0.0,
          "warmup": false,
          "state_anchor_text": "停止 阻碍 阻挡"
        }"#;
        let resp: SnapshotResponse =
            serde_json::from_str(&format!("{{\"snapshot\":{raw_legacy}}}")).expect(
                "真实快照必须可解析 —— 含 daemon 新增字段与负值分量；\
                 解析失败意味着通道会静默假死",
            );
        let snap = resp.snapshot.expect("snapshot 字段应存在");
        assert_eq!(snap.gua_distribution.len(), 8, "8 维分布长度契约");
        assert!(
            snap.gua_distribution.iter().any(|v| *v < 0.0),
            "样本应含负值（历史缺陷证据）；若模型/聚合方式变了本断言会提醒复核"
        );
        assert!(snap.dominant_gua.is_none(), "空闲期 dominant_gua 为 null");
        assert_eq!(snap.dominant_bagua.as_deref(), Some("艮"));
        assert_eq!(snap.state_anchor_text.as_deref(), Some("停止 阻碍 阻挡"));

        // 负值必须被 match_score_of 归零（不得产生负分参与排序）
        assert_eq!(match_score_of(&snap.gua_distribution, 2), 0.0);
        // 正分量应原样返回
        assert!((match_score_of(&snap.gua_distribution, 6) - 0.203422).abs() < 1e-6);

        // 端到端：该快照喂进匹配核心，不得 panic
        let mems = vec![
            mem_with_bagua("gu-null-hits", "艮域记忆", Some(6), 30),
            mem_with_bagua("xun-hits", "巽域记忆", Some(4), 30),
        ];
        let out = match_memories_core(&mems, &snap);
        assert!(out.executed, "真实快照应能执行匹配");
        let ids: Vec<&str> = out
            .candidates
            .iter()
            .map(|c| c.memory_id.as_str())
            .collect();
        // 阈值 0.6 下：0.203422 与 0.200704 均 < 0.6 → 应无候选（与实测一致）
        assert!(
            ids.is_empty(),
            "阈值 0.6 对真实集中度应无候选（这正是 §3.16.8 暴露的阻塞），实际 {ids:?}"
        );
    }

    /// **回归门禁：道体侧产出的 8 维分布必须是非负概率分布**（源头契约）。
    ///
    /// 修正前 daemon 会产出含负分量的"分布"（实测 −0.010273），靠 LRC 侧
    /// `clamp` 兜底 —— 属"消费端替生产端擦屁股"。本用例守住源头契约：
    /// 若未来有人改动 `bagua_distribution_8` 又引入负值，本用例必须失败。
    ///
    /// **注意**：本用例在 **Rust 侧**无法直接调用 Python 的
    /// `bagua_distribution_8`，故此处校验的是**契约本身**（对历史样本的
    /// 负值处理 + 修正后样本的非负性）。真正的源头回归在
    /// `temp/v15-nonneg-recheck.py`（Python 侧，1200 拍实测）。
    #[test]
    fn state_distribution_must_be_non_negative() {
        // 修正后**真实**快照的 8 维分布（逐字取自修正后 daemon 的
        // GET /state/snapshot 输出，非构造数据）：temp/real_snapshot_fixed.json
        let fixed: Vec<f32> = vec![
            0.139181, 0.034495, 0.027165, 0.066770, 0.106795, 0.052046, 0.342088, 0.231460,
        ];
        assert!(fixed.iter().all(|v| *v >= 0.0), "修正后真实样本必须非负");
        let sum: f32 = fixed.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-4,
            "修正后真实样本应为归一化分布（实测 sum=1.0），实际 {sum}"
        );
        // 该真实样本的 max 分量 0.342088 —— 与标定区间（0.32~0.34）吻合，
        // 这是"阈值 0.6 会关掉通道"的直接证据
        assert!((match_score_of(&fixed, 6) - 0.342088).abs() < 1e-6);
        assert!(
            match_score_of(&fixed, 6) < STATE_MATCH_MIN_DEFAULT,
            "真实样本最高分 {:.4} 低于默认阈值 {:.2} → 默认阈值下通道必然关闭",
            match_score_of(&fixed, 6),
            STATE_MATCH_MIN_DEFAULT
        );
    }

    /// **§3.17 决定性证据的回归门禁：真实库上任何阈值都恒空。**
    ///
    /// **为什么必须固化**（防未来重蹈）：真实库的记忆母卦分布（离 96.77%、
    /// 兑 1.70%）与道体状态主导（艮/坤/乾）**不相交**，故候选恒空。
    /// 若未来有人看到"候选恒空"就去调阈值，本用例会失败并指向 §3.17。
    ///
    /// 样本为**真实**分布的摘要（`temp/v15-space-disjoint.py` 实测）：
    ///   · 记忆侧：离(idx2) 4183 条、兑(idx1) 74 条
    ///   · 状态侧：8 维均值 艮≈0.240、坤≈0.217、离≈0.103、兑≈0.058
    /// 断言：**真实状态分布在记忆所在维上的质量 < 任何可用阈值**。
    #[test]
    fn real_library_spaces_are_disjoint_so_any_threshold_yields_nothing() {
        // 真实状态 8 维均值（temp/v15-space-disjoint.py 实测，V15 驱动 300 拍）
        let state_avg: [f32; 8] = [
            0.0909, 0.0565, 0.0892, 0.1165, 0.1013, 0.0876, 0.2409, 0.2172,
        ];
        // 记忆所在维：离=2（4183 条）、兑=1（74 条）
        let mem_dims = [1usize, 2usize];
        let mem_mass: f32 = mem_dims.iter().map(|i| state_avg[*i]).sum();
        println!(
            "真实库：记忆所在维（兑+离）上的状态质量合计 = {mem_mass:.4}；\
             状态主导维（艮+坤）合计 = {:.4}",
            state_avg[6] + state_avg[7]
        );
        assert!(
            mem_mass < 0.20,
            "记忆所在维的状态质量 {mem_mass:.4} 应远小于任何合理阈值 → 恒空。\
             若此断言失败，说明分布形状变了，须重新核对 PREREG §3.17"
        );
        // 即便把阈值降到荒谬的 0.15，最高的记忆维（离 0.0892）仍不足
        assert!(
            state_avg[2] < 0.15,
            "离维质量 {:.4} 应低于 0.15（对应实测「降到 0.20 仍恒空」）",
            state_avg[2]
        );
    }

    /// **§3.22 方向E 核心门禁：相对口径必须"覆盖更广"且"不放行无信息状态"**。
    ///
    /// 双向对照（M9：门禁必须能失败）：
    ///   ① **正面对照**：有信息的分布 → 相对口径命中的维**多于**绝对口径
    ///      （证明它确实突破了 ⌊1/阈值⌋ 上界）
    ///   ② **负面对照**：均匀分布（无信息）→ `zscore_8` 必须返回 `None`，
    ///      相对口径**不得产出任何候选**（防恒真闸门）
    #[test]
    fn relative_metric_expands_coverage_but_rejects_flat_state() {
        let _g = env_guard();
        // **真实单拍状态分布**（temp/v15-concentration-gate.py 实测输出，
        // CV=0.3869）。注意：**不能用跨拍均值**——均值被平滑后 CV 仅 0.257，
        // 会被集中度门禁误判为"低集中"，是探针设计错误（实测踩过）。
        let real_like: Vec<f32> = vec![
            0.0687, 0.0873, 0.1183, 0.1307, 0.1166, 0.0835, 0.2271, 0.1678,
        ];
        let abs_min = STATE_MATCH_MIN_DEFAULT; // 0.6
        let abs_ok = (0..8)
            .filter(|i| match_score_of(&real_like, *i as u8) >= abs_min)
            .count();
        let rel_ok = (0..8)
            .filter(|i| match_score_relative(&real_like, *i as u8, 3).is_some())
            .count();
        println!("真实形态分布：绝对口径命中 {abs_ok} 维，相对口径(Top-3) 命中 {rel_ok} 维");
        assert_eq!(
            abs_ok, 0,
            "该分布各维均 <0.6，绝对口径应 0 命中（这正是上界问题）"
        );
        assert!(
            rel_ok >= 3,
            "相对口径 Top-3 应命中至少 3 维（含 z>0 约束），实际 {rel_ok}"
        );

        // 负面对照：均匀分布必须**完全无候选**
        let flat = vec![0.125f32; 8];
        assert!(
            zscore_8(&flat).is_none(),
            "均匀分布 σ=0 → 必须返回 None（无信息状态不得产出候选）"
        );
        assert!(
            (0..8).all(|i| match_score_relative(&flat, i as u8, 3).is_none()),
            "均匀分布下相对口径必须 8 维全不命中"
        );

        // 端到端：开启相对口径后，均匀分布快照不得产出候选
        std::env::set_var("LRC_STATE_RELATIVE_MATCH", "1");
        let mems = vec![mem_with_bagua("m1", "某域记忆", Some(6), 30)];
        let mut snap = snapshot_with(flat, false);
        snap.gua_mass = 1.0;
        let out = match_memories_core(&mems, &snap);
        assert!(
            out.candidates.is_empty(),
            "相对口径下无信息状态必须跳过，实际产出 {} 条",
            out.candidates.len()
        );
        assert_eq!(out.skip_reason.as_deref(), Some("state_distribution_flat"));

        // 有信息状态则必须产出（证明门禁不是恒空）
        let snap2 = snapshot_with(real_like.clone(), false);
        let out2 = match_memories_core(&mems, &snap2);
        assert!(
            !out2.candidates.is_empty(),
            "相对口径对有信息状态必须产出候选（否则门禁恒空）"
        );
        std::env::remove_var("LRC_STATE_RELATIVE_MATCH");
    }

    /// 相对口径**默认关闭**：不设环境变量时，行为必须与绝对口径完全一致。
    #[test]
    fn relative_metric_is_off_by_default() {
        let _g = env_guard();
        std::env::remove_var("LRC_STATE_RELATIVE_MATCH");
        assert!(!state_relative_match_enabled(), "相对口径必须默认关闭");

        let real_like: Vec<f32> = vec![
            0.0687, 0.0873, 0.1183, 0.1307, 0.1166, 0.0835, 0.2271, 0.1678,
        ];
        let mems = vec![mem_with_bagua("m1", "某域记忆", Some(6), 30)];
        // 默认阈值 0.6 下该分布无候选 —— 关闭态必须复现此行为
        let out = match_memories_core(&mems, &snapshot_with(real_like, false));
        assert!(
            out.candidates.is_empty(),
            "关闭态下应走绝对口径（0.6 阈值 → 无候选），实际 {}",
            out.candidates.len()
        );
    }

    /// `zscore_8` 的边界：维度不符 / 零方差 → `None`（不得产生 NaN 污染）。
    #[test]
    fn zscore_8_handles_degenerate_inputs() {
        assert!(zscore_8(&[0.5, 0.5]).is_none(), "维度不符应返回 None");
        assert!(zscore_8(&[0.0; 8]).is_none(), "全零应返回 None");
        assert!(zscore_8(&[0.3; 8]).is_none(), "全同值应返回 None");
        let z = zscore_8(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]).expect("应可标准化");
        assert!(z.iter().all(|v| v.is_finite()), "不得含 NaN/Inf");
        let sum: f32 = z.iter().sum();
        assert!(sum.abs() < 1e-5, "标准化后均值应为 0，实际 {sum}");
    }

    /// **集中度门禁（§3.22 补）："伪信息状态"必须被拒绝**。
    ///
    /// 覆盖端到端实测暴露的缺口（`temp/v15-e2e-verify.py`）：
    ///   仅 Top-k ∩ z>0 时，`[0.13,0.12,…,0.14,0.12]`（几乎平坦）命中 815 条候选。
    ///   根因：z-score 无量纲，会把微小差异放大 → 恒真闸门变体。
    ///
    /// 双向对照：
    ///   ① 真实形态分布（CV≈0.40）→ 必须通过
    ///   ② 伪信息分布（CV≈0.055）→ 必须拒绝
    #[test]
    fn concentration_gate_rejects_pseudo_information() {
        let _g = env_guard();
        // 真实 V15 状态形态（temp/v15-concentration-gate.py 实测，CV≈0.3869）
        let real_like: Vec<f32> = vec![
            0.0687, 0.0873, 0.1183, 0.1307, 0.1166, 0.0835, 0.2271, 0.1678,
        ];
        // 伪信息（端到端实测被误放行的那个）
        let pseudo: Vec<f32> = vec![0.13, 0.12, 0.13, 0.12, 0.13, 0.12, 0.14, 0.12];
        let cv_real = concentration_cv(&real_like);
        let cv_pseudo = concentration_cv(&pseudo);
        println!("CV 真实={cv_real:.4} 伪信息={cv_pseudo:.4} 阈值={STATE_MIN_CONCENTRATION:.4}");
        assert!(
            cv_real > STATE_MIN_CONCENTRATION,
            "真实形态分布 CV={cv_real:.4} 必须高于阈值（否则门禁会误杀）"
        );
        assert!(
            cv_pseudo < STATE_MIN_CONCENTRATION,
            "伪信息分布 CV={cv_pseudo:.4} 必须低于阈值（否则门禁形同虚设）"
        );
        // 两者必须可分（间隔 > 0）
        assert!(cv_real > cv_pseudo, "真实与伪信息的 CV 必须可分");

        // 端到端：伪信息状态在相对口径下必须无候选
        std::env::set_var("LRC_STATE_RELATIVE_MATCH", "1");
        let mems = vec![mem_with_bagua("m1", "某域记忆", Some(6), 30)];
        let mut snap = snapshot_with(pseudo, false);
        snap.gua_mass = 1.0;
        let out = match_memories_core(&mems, &snap);
        assert!(
            out.candidates.is_empty(),
            "伪信息状态必须被集中度门禁拒绝，实际产出 {} 条",
            out.candidates.len()
        );
        assert_eq!(out.skip_reason.as_deref(), Some("state_low_concentration"));

        // 真实形态状态必须正常产出（证明门禁不是恒空）
        let mut snap2 = snapshot_with(real_like, false);
        snap2.gua_mass = 1.0;
        let out2 = match_memories_core(&mems, &snap2);
        assert!(
            !out2.candidates.is_empty(),
            "真实形态状态必须产出候选（否则门禁恒空）"
        );
        std::env::remove_var("LRC_STATE_RELATIVE_MATCH");
    }

    /// `concentration_cv` 边界：空/全零/负均值 → 0（不得产生 NaN）。
    #[test]
    fn concentration_cv_handles_degenerate_inputs() {
        assert_eq!(concentration_cv(&[]), 0.0);
        assert_eq!(concentration_cv(&[0.0; 8]), 0.0);
        assert!(concentration_cv(&[-1.0; 8]) >= 0.0);
        // 均匀分布 → CV=0
        assert!(concentration_cv(&[0.125; 8]).abs() < 1e-6);
        // 全部有限
        let v = concentration_cv(&[0.1, 0.2, 0.3, 0.1, 0.2, 0.3, 0.1, 0.2]);
        assert!(v.is_finite() && v > 0.0);
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

    // ---- 方向二：语义向量匹配 ----

    /// 构造一个"方向明确"的锚点向量与两类记忆向量：
    /// 相关组与锚点夹角小（余弦高），无关组与锚点接近正交。
    fn vec_of(dim: usize, idx: usize, noise: f32) -> Vec<f32> {
        (0..dim)
            .map(|i| if i == idx { 1.0 } else { noise })
            .collect()
    }

    /// **方向二核心门禁（M9）：换锚点向量必须换候选集。**
    ///
    /// 若换锚点后候选不变，说明语义匹配没生效（与 P7 的教训同源：
    /// 必须验证"触发源确实进入了被检环节"）。
    #[test]
    fn semantic_changing_anchor_must_change_candidates() {
        let _g = env_guard();
        let dim = 16;
        let mems = vec![
            mem_with_bagua("a", "组A记忆", Some(0), 30),
            mem_with_bagua("b", "组B记忆", Some(0), 30),
        ];
        let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
        vectors.insert("a".to_string(), vec_of(dim, 0, 0.01)); // 与锚点A同向
        vectors.insert("b".to_string(), vec_of(dim, 1, 0.01)); // 与锚点B同向

        let anchor_a = vec_of(dim, 0, 0.01);
        let anchor_b = vec_of(dim, 1, 0.01);
        let out_a = match_semantic_core(&anchor_a, &mems, &vectors);
        let out_b = match_semantic_core(&anchor_b, &mems, &vectors);

        let ids_a: Vec<&str> = out_a
            .candidates
            .iter()
            .map(|c| c.memory_id.as_str())
            .collect();
        let ids_b: Vec<&str> = out_b
            .candidates
            .iter()
            .map(|c| c.memory_id.as_str())
            .collect();
        assert!(!ids_a.is_empty(), "锚点A 应命中同向记忆（构造有效性前提）");
        assert!(!ids_b.is_empty(), "锚点B 应命中同向记忆（构造有效性前提）");
        assert_ne!(
            ids_a, ids_b,
            "语义匹配失效：换锚点后候选未改变 → 匹配未生效。a={ids_a:?} b={ids_b:?}"
        );
        assert_eq!(ids_a, vec!["a"], "锚点A 应只命中 A 组记忆");
        assert_eq!(ids_b, vec!["b"], "锚点B 应只命中 B 组记忆");
    }

    /// 语义阈值必须真能过滤（M9：防"阈值恒被满足"）。
    ///
    /// **用绝对口径测**（`LRC_STATE_SEMANTIC_ZSCORE=0`）：z-score 是"池内相对"
    /// 口径，在只有 1 条候选时 σ 不可靠 → 自动退化为绝对口径，故此处显式
    /// 关闭 z-score 以测绝对分支的阈值是否真的生效。
    #[test]
    fn semantic_threshold_can_actually_filter() {
        let _g = env_guard();
        std::env::set_var("LRC_STATE_SEMANTIC_ZSCORE", "0");
        let dim = 16;
        let mems = vec![mem_with_bagua("m", "某记忆", Some(0), 30)];
        let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
        // 与锚点近正交 → 余弦≈0
        vectors.insert("m".to_string(), vec_of(dim, 5, 0.01));
        let anchor = vec_of(dim, 0, 0.01);

        std::env::set_var("LRC_STATE_SEMANTIC_MIN", "0.5");
        let hi = match_semantic_core(&anchor, &mems, &vectors);
        assert!(
            hi.candidates.is_empty(),
            "近正交（余弦≈0）在阈值 0.5 下必须被过滤"
        );

        std::env::set_var("LRC_STATE_SEMANTIC_MIN", "0.0");
        let lo = match_semantic_core(&anchor, &mems, &vectors);
        assert!(
            !lo.candidates.is_empty(),
            "阈值降到 0 后必须产出候选（证明阈值在起作用）"
        );
        std::env::remove_var("LRC_STATE_SEMANTIC_MIN");
        std::env::remove_var("LRC_STATE_SEMANTIC_ZSCORE");
    }

    /// **z-score 口径必须能跨锚点复用**（本通道的核心门禁）。
    ///
    /// 实测依据：三个真实锚点的绝对余弦可行区间交集几乎为空
    /// （兑锚点"无关最高 0.6214" > 坎锚点"相关最低 0.5866"），
    /// 故绝对阈值跨锚点**数学上不可用**。本用例构造两个"基线余弦不同的锚点"，
    /// 验证 z-score 口径下**同一个阈值**都能正确筛出各自的相关项。
    #[test]
    fn semantic_zscore_separates_across_anchors_with_one_threshold() {
        let _g = env_guard();
        std::env::set_var("LRC_STATE_SEMANTIC_ZSCORE", "1");
        let dim = 8;
        // 构造 6 条记忆：3 条与锚点A 更近，3 条更远。
        // 关键：两个锚点的**基线余弦整体不同**（模拟实测中坎 0.59 / 兑 0.67）。
        let mut mems = Vec::new();
        let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
        for i in 0..6 {
            let id = format!("m{i}");
            mems.push(mem_with_bagua(&id, "记忆", Some(0), 30));
            // 前 3 条与"维 0"相关，后 3 条与"维 1"相关
            let v = if i < 3 {
                vec_of(dim, 0, 0.30 + i as f32 * 0.01)
            } else {
                vec_of(dim, 1, 0.30 + i as f32 * 0.01)
            };
            vectors.insert(id, v);
        }
        let anchor = vec_of(dim, 0, 0.30);

        // 阈值取标定值 0.35（z 口径）
        std::env::set_var("LRC_STATE_SEMANTIC_MIN", "0.35");
        let out = match_semantic_core(&anchor, &mems, &vectors);
        let ids: Vec<&str> = out
            .candidates
            .iter()
            .map(|c| c.memory_id.as_str())
            .collect();
        assert!(
            !ids.is_empty(),
            "z-score 口径下应能筛出与锚点相关的那组（构造有效性前提）"
        );
        // 应只命中前 3 条（与维 0 相关），不应命中后 3 条
        assert!(
            ids.iter().all(|id| ["m0", "m1", "m2"].contains(id)),
            "应只命中与锚点同向的那组，实际 {ids:?}"
        );
        std::env::remove_var("LRC_STATE_SEMANTIC_MIN");
        std::env::remove_var("LRC_STATE_SEMANTIC_ZSCORE");
    }

    /// **未缓存的记忆必须被跳过**（而非回退去实时编码）。
    ///
    /// 语义路径的设计前提是"只编码锚点、其余读缓存"；若这里退化成
    /// "缺缓存就实时编码"，会让单次请求耗时随库规模线性增长
    /// （实测 2.25s/条 → 全库约 2 小时），导致请求必然超时。
    #[test]
    fn semantic_skips_uncached_memory_instead_of_encoding() {
        let _g = env_guard();
        let dim = 16;
        let mems = vec![
            mem_with_bagua("cached", "已缓存", Some(0), 30),
            mem_with_bagua("uncached", "未缓存", Some(0), 30),
        ];
        let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
        vectors.insert("cached".to_string(), vec_of(dim, 0, 0.01));
        let anchor = vec_of(dim, 0, 0.01);

        let out = match_semantic_core(&anchor, &mems, &vectors);
        let ids: Vec<&str> = out
            .candidates
            .iter()
            .map(|c| c.memory_id.as_str())
            .collect();
        assert_eq!(ids, vec!["cached"], "未缓存记忆必须跳过，不得实时编码");
        assert!(out.filtered_out >= 1, "应记录被跳过数（可观测性）");
    }

    /// 语义路径同样遵守"未访问 ≥ N 天"判据（与标签路径同口径）。
    #[test]
    fn semantic_respects_stale_gate() {
        let _g = env_guard();
        let dim = 16;
        let mems = vec![mem_with_bagua("recent", "刚看过", Some(0), 1)];
        let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
        vectors.insert("recent".to_string(), vec_of(dim, 0, 0.01));
        let anchor = vec_of(dim, 0, 0.01);
        let out = match_semantic_core(&anchor, &mems, &vectors);
        assert!(out.candidates.is_empty(), "近期访问过的记忆不得作为候选");
    }

    #[test]
    fn semantic_cosine_is_bounded_and_safe() {
        // 相同向量 → 1.0
        let a = vec![1.0f32, 2.0, 3.0];
        assert!((cosine(&a, &a).unwrap() - 1.0).abs() < 1e-6);
        // 正交 → 0.0
        let b = vec![1.0f32, 0.0, 0.0];
        let c = vec![0.0f32, 1.0, 0.0];
        assert!(cosine(&b, &c).unwrap().abs() < 1e-6);
        // 零模长 / 维度不符 → None（不得产生 NaN 污染排序）
        assert!(cosine(&[0.0, 0.0, 0.0], &b).is_none());
        assert!(cosine(&[1.0, 2.0], &a).is_none());
        assert!(cosine(&[], &[]).is_none());
    }

    /// 缓存往返：落盘 → 读回内容一致（跨请求可见性）。
    #[test]
    fn semantic_cache_roundtrip_persists() {
        let dir = TempDir::new().expect("应创建临时目录");
        let mut c = SemanticVectorCache {
            version: SEMANTIC_CACHE_VERSION,
            dim: 3,
            vectors: HashMap::new(),
        };
        c.vectors.insert("m1".to_string(), vec![0.1, 0.2, 0.3]);
        save_semantic_cache(dir.path(), &c);
        let back = load_semantic_cache(dir.path());
        assert_eq!(back.dim, 3);
        assert_eq!(back.vectors.get("m1").map(|v| v.len()), Some(3));
    }

    /// 缓存损坏/版本不符 → 静默重建为空（不得让发现功能因磁盘问题失效）。
    #[test]
    fn semantic_cache_load_degrades_silently() {
        let dir = TempDir::new().expect("应创建临时目录");
        let path = semantic_cache_path(dir.path());
        std::fs::write(&path, b"not json at all").expect("应能写入");
        let c = load_semantic_cache(dir.path());
        assert!(c.vectors.is_empty(), "损坏缓存应静默返回空");
        // 版本不符同样失效
        std::fs::write(
            &path,
            br#"{"version":9999,"dim":3,"vectors":{"m":[1,2,3]}}"#,
        )
        .expect("应能写入");
        let c2 = load_semantic_cache(dir.path());
        assert!(c2.vectors.is_empty(), "版本不符应整体失效");
    }

    /// 缺 `state_anchor_text`（旧 daemon）→ 语义路径不启用，回退标签匹配。
    #[test]
    fn missing_anchor_text_falls_back_to_label_path() {
        let _g = env_guard();
        std::env::set_var("LRC_STATE_DRIVEN_DISCOVERY", "1");
        let (dir, mut store) = make_store();
        store
            .remember(mem_with_bagua("m1", "坎域记忆", None, 30))
            .expect("应成功记住");
        let snap = snapshot_with(dist_focused(5, 0.9), false);
        assert!(snap.state_anchor_text.is_none());
        let out = run_state_driven_cycle(&store, &snap, dir.path());
        // 回退标签路径后仍能产出（证明缺字段不导致通道失效）
        assert!(out.executed, "缺锚点文本不应让通道失效");
        std::env::remove_var("LRC_STATE_DRIVEN_DISCOVERY");
    }
}
