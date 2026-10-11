//! ============================================================
//! 许可证: Apache 2.0
//! LRC 内置记忆联想状态机。
//! ============================================================

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;

const MAX_ACTIVE_MEMORIES: usize = 32;
const MAX_ASSOCIATION_DEPTH: u8 = 4;
const DECAY_PER_STEP: f32 = 0.85;

/// 当前记忆联想的运行状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryState {
    /// 当前活跃记忆及其激活强度，按强度降序维护。
    pub active: Vec<ActiveMemory>,
    /// 当前联想链，记录本轮由哪条记忆走到哪条记忆。
    pub trail: Vec<AssociationStep>,
    /// 已经连续多少步没有发现新的高相关记忆。
    pub stable_steps: u8,
    /// 状态机版本，便于未来迁移持久化格式。
    pub version: u32,
}

impl Default for MemoryState {
    fn default() -> Self {
        Self {
            active: Vec::new(),
            trail: Vec::new(),
            stable_steps: 0,
            version: 1,
        }
    }
}

/// 一个被状态机激活的记忆。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveMemory {
    pub memory_id: String,
    pub activation: f32,
    pub hits: u32,
    pub last_step: u32,
}

/// 联想链中的一步。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssociationStep {
    pub from_id: Option<String>,
    pub to_id: String,
    pub score: f32,
    pub depth: u8,
}

/// 状态机决定本轮的联想策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssociationDecision {
    /// 当前方向足够明确，停止扩散并收缩到当前候选。
    Contract,
    /// 有一个明显方向，只沿一条边继续。
    Single { depth: u8 },
    /// 存在多个近似方向，保留多个分支。
    Branch { width: usize, depth: u8 },
}

/// 由候选分数、不确定性和当前深度决定是否继续联想。
pub fn decide_association(scores: &[f32], current_depth: u8, max_depth: u8) -> AssociationDecision {
    if scores.is_empty() || current_depth >= max_depth {
        return AssociationDecision::Contract;
    }
    let mut ordered = scores.to_vec();
    ordered.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
    let top = ordered[0];
    let second = ordered.get(1).copied().unwrap_or(0.0);
    let margin = top - second;
    if top >= 0.85 && margin >= 0.20 {
        AssociationDecision::Contract
    } else if margin >= 0.15 {
        AssociationDecision::Single {
            depth: current_depth + 1,
        }
    } else {
        let width = ordered
            .iter()
            .take(3)
            .filter(|score| **score >= top * 0.80)
            .count()
            .max(2);
        AssociationDecision::Branch {
            width,
            depth: current_depth + 1,
        }
    }
}

/// 内置道体状态机：维护活跃记忆、决定扩散形态并生成联想链。
#[derive(Debug, Clone)]
pub struct MemoryStateMachine {
    pub state: MemoryState,
    step: u32,
}

impl Default for MemoryStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStateMachine {
    pub fn new() -> Self {
        Self {
            state: MemoryState::default(),
            step: 0,
        }
    }

    pub fn from_state(state: MemoryState) -> Self {
        let step = state
            .active
            .iter()
            .map(|entry| entry.last_step)
            .max()
            .unwrap_or(0);
        Self { state, step }
    }

    /// 衰减旧活性，并激活本轮召回的记忆。
    pub fn activate(&mut self, memory_id: &str, relevance: f32) {
        self.step += 1;
        for entry in &mut self.state.active {
            entry.activation *= DECAY_PER_STEP;
        }
        if let Some(entry) = self
            .state
            .active
            .iter_mut()
            .find(|entry| entry.memory_id == memory_id)
        {
            entry.activation = (entry.activation + relevance.clamp(0.0, 1.0)).min(1.0);
            entry.hits += 1;
            entry.last_step = self.step;
        } else {
            self.state.active.push(ActiveMemory {
                memory_id: memory_id.to_string(),
                activation: relevance.clamp(0.0, 1.0),
                hits: 1,
                last_step: self.step,
            });
        }
        self.state.active.sort_by(|a, b| {
            b.activation
                .partial_cmp(&a.activation)
                .unwrap_or(Ordering::Equal)
        });
        self.state.active.truncate(MAX_ACTIVE_MEMORIES);
    }

    /// 记录一次从当前记忆到下一条记忆的状态转移。
    pub fn transition(
        &mut self,
        from_id: Option<&str>,
        to_id: &str,
        score: f32,
        depth: u8,
    ) -> bool {
        let depth = depth.min(MAX_ASSOCIATION_DEPTH);
        let changed = self
            .state
            .trail
            .last()
            .map(|step| step.to_id != to_id || step.depth != depth)
            .unwrap_or(true);
        self.state.trail.push(AssociationStep {
            from_id: from_id.map(str::to_string),
            to_id: to_id.to_string(),
            score: score.clamp(0.0, 1.0),
            depth,
        });
        if changed {
            self.state.stable_steps = 0;
        } else {
            self.state.stable_steps = self.state.stable_steps.saturating_add(1);
        }
        changed
    }

    /// 取得当前活跃记忆的强度，用于将跨会话上下文带回新查询。
    pub fn active_context(&self, limit: usize) -> Vec<(String, f32)> {
        self.state
            .active
            .iter()
            .take(limit)
            .map(|entry| (entry.memory_id.clone(), entry.activation))
            .collect()
    }

    /// 取得当前活跃记忆的 id 列表（导航源）。
    /// 用于联想扩散：把"近期在想什么"的内容作为联想桥带入本次检索。
    pub fn active_ids(&self, limit: usize) -> Vec<String> {
        self.state
            .active
            .iter()
            .take(limit)
            .map(|entry| entry.memory_id.clone())
            .collect()
    }

    pub fn should_continue(&self, decision: AssociationDecision) -> bool {
        !matches!(decision, AssociationDecision::Contract)
            && self.state.stable_steps < 2
            && self.state.trail.len() < MAX_ASSOCIATION_DEPTH as usize
    }

    pub fn snapshot(&self) -> MemoryState {
        self.state.clone()
    }
}

/// 活性偏置开关（**LRC_STATE_BIAS=1 显式开启，默认关闭**）。
///
/// ---------------------------------------------------------------------------
/// **默认值翻转（v0.9.8）——原为默认开启，现改为默认关闭**
/// ---------------------------------------------------------------------------
/// 本开关门控的是 v0.9.7 遗留的「道体状态机·联想导航」整条通路，共 4 处注入点：
///   ① `trapezoid_focus_recall` 活跃记忆**候选白名单**（召回层）
///   ② `trapezoid_focus_recall` 联想桥词**词面域锚点扩展**（召回层）
///   ③ `trapezoid_focus_recall` 活性偏置**加分**（`*s += activation * 0.25`，**排序层**）
///   ④ `recall` 联想桥词**查询扩展**（召回层）
///
/// **为什么改为默认关闭**：③ 是直接改写排序分数的道体信号，而
/// `daoti/PREREG_FAIR_STATE_MACHINE.md` §判据 G2 已实测**道体信号参与排序
/// 无净增量**（补齐 +8pp 门槛未达，三臂齐平）⇒ 该通路**已被否证**。
/// 此外 §3.37 进一步实测：记忆侧的 `bagua_index`（`bagua_index` 及其
/// `daoti_preview_*` 同源）**不读语义**——打乱字符顺序后分类 100% 不变，
/// 根因是洛书编码器 9 维特征仅含字符密度/字符熵/位置权重。
///
/// **边界（必须如实保留）**：本次仅**翻转默认值**，不删除代码、不改算法。
/// 开 `LRC_STATE_BIAS=1` 可完整复现 v0.9.7 行为（用于对照实验与回归取证）。
/// 被否证的是「该信号参与排序」，**不是**「状态机本身无用」——
/// 状态机作为记录层/观测层的能力不受影响。
///
/// 实时读取环境变量而非 OnceLock 缓存：开关必须在运行期可切换，
/// 否则一旦首次调用缓存为 true，用户设置 LRC_STATE_BIAS=1 将永远失效。
pub fn state_bias_enabled() -> bool {
    std::env::var("LRC_STATE_BIAS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// 对候选记忆分数执行回归校验：不能因为扩散而完全偏离原始查询。
pub fn regression_filter(
    candidates: &[(String, f32)],
    original_scores: &HashMap<String, f32>,
    threshold: f32,
) -> Vec<(String, f32)> {
    candidates
        .iter()
        .filter(|(id, score)| {
            let original = original_scores.get(id).copied().unwrap_or(0.0);
            *score >= threshold && (original >= threshold * 0.5 || *score >= threshold * 1.25)
        })
        .cloned()
        .collect()
}

/// 道体再次校验（回归验证层）。
///
/// 联想扩散把候选拉进 top_k 后，必须在输出前验证它是否真的回应了原始查询
/// ——"发散之后能否收束回主题"，防止碰巧共享联想桥词的噪声混入结果。
///
/// 判定信号（满足任一即保留）：
/// 1. **原查询词面命中**：候选直接包含原查询词 → 与查询直接相关；
/// 2. **联想桥强关联**：候选命中 ≥2 个活跃记忆专属联想桥词 → 真联想边，
///    非单个泛词误撞；
/// 3. **标签共鸣**：候选标签与原查询词共享 → 用户主动标注的语义证据；
/// 4. **原意图回应度**（P4 第四证据位）：daemon /reflect 的意图相关分 ≥0.5 →
///    词面零重叠但语义回应原意图的候选凭此保留（道体状态级校验）。
///
/// `intent_score` 语义：
///   - `Some(score)` 且 `score >= 0.5` → 判定"回应原意图"，保留（第四证据生效）
///   - `Some(score)` 且 `score < 0.5` → 第四证据不通过，回退三证据裁决
///   - `None`（daemon 离线/未接入）→ 第四证据缺省跳过，行为与三证据现状逐字节一致
///
/// 若三种词面证据皆无且第四证据缺席/未过 → 判定为"发散噪声"，剔除。
/// 返回保留/剔除判定及证据标签（供联想链输出可观测）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegressionVerdict {
    pub keep: bool,
    pub evidence: &'static str,
}

/// 第四证据位（原意图回应度）的判定阈值：daemon /reflect 意图相关分 ≥ 0.5
/// 视为"回应原意图"（与计划文档 P4.1 一致）。
pub const ASSOCIATION_INTENT_RESPONSE_THRESHOLD: f32 = 0.5;

/// 第五证据位（**本地语义同源度**）的默认阈值。
///
/// ## 为什么需要这一位（根因与设计意图）
///
/// 深度检索的**召回阶段用语义**（洛书向量余弦 + 卦象/ROI），
/// 而**验收阶段此前只有词面判据**（证据 1~3 全是 token 重叠）。
/// 两阶段判据不同源 ⇒ **"语义相关但词面零重叠"的候选被系统性否决**。
///
/// 实测（开发端 3189 条库，`LRC_DEEP_TRACE=1`）：
///   ROI 命中 311 → 深度路仅存活 72（23.15%）；另一查询 385 → 12（3.12%）。
/// 关掉词面门后存活率升到 63.67% / 51.95% ⇒ 上限被这道门锁死。
///
/// 修法：把**召回阶段已算出的语义余弦**作为第五证据喂回来 —— 判据同源。
///
/// ## 关于默认值：**是标定出来的，不是拍脑袋**
///
/// 方法：用**金丝雀**（`test_record_layer_end_to_end_persist_and_associate` ——
/// 它断言"语义无关的记忆不得进主结果"）做门槛扫描，找它转绿的拐点：
///
/// ```text
///  threshold=0.60 → FAILED      （漏噪：把「充电宝」放进了「苏堤」的主结果）
///  threshold=0.70 → FAILED
///  threshold=0.75 → ok          ← 拐点（本默认值）
///  0.80/0.85/0.90/0.95 → ok
/// ```
///
/// 即该负例的余弦约 0.70~0.75。取拐点值 0.75 作为默认；
/// 可用 `LRC_DEEP_SEMANTIC_KEEP` 覆盖（标定/消融），
/// `LRC_DEEP_SEMANTIC_GATE=0` 可整体关闭。
///
/// ## 已知残余风险（诚实标注）
///
/// 绝对阈值对**查询长度**敏感（本例查询仅 2 字，短查询的向量余弦普遍偏高）。
/// 更稳的形态是"池内相对分位 + 绝对底线"双条件，留待用真实库标定后再切换；
/// 当前以金丝雀守住下限，先确保**不引入噪声**。
pub const DEEP_SEMANTIC_KEEP_DEFAULT: f32 = 0.75;

/// 读取第五证据位的阈值（运行期实时读取，便于标定与消融，不必重启）。
pub fn deep_semantic_keep_threshold() -> f32 {
    std::env::var("LRC_DEEP_SEMANTIC_KEEP")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| (0.0..=1.0).contains(v))
        .unwrap_or(DEEP_SEMANTIC_KEEP_DEFAULT)
}

/// 第五证据位的**总开关**（消融 / 归因 / 回滚用）。
///
/// - 缺省 **开启**（这是能力修复的默认形态）
/// - `LRC_DEEP_SEMANTIC_GATE=0` 显式关闭 ⇒ 行为与"未接入 P5 前"逐字节一致
///
/// 为什么要有它：定位回归时必须能做 A/B 归因（"这个失败是我引入的还是本来就有的"），
/// 而不必重新编译。（与 `LRC_STATE_BIAS` / `LRC_DEEP_LEX_DOMAIN` 同款约定。）
pub fn deep_semantic_gate_enabled() -> bool {
    !matches!(
        std::env::var("LRC_DEEP_SEMANTIC_GATE").ok().as_deref(),
        Some("0") | Some("false") | Some("off")
    )
}

pub fn regression_recheck(
    original_overlap: usize,
    bridge_hits: usize,
    tag_hits: usize,
    // P4 第四证据位：daemon 返回的意图相关分（None = daemon 离线，跳过）
    intent_score: Option<f32>,
    // P5 第五证据位：**本地语义同源度**——召回阶段已算出的"候选 vs 原查询"余弦相似度。
    // None = 编码器不可用/未提供 → 缺省跳过，行为与"未接入 P5 前"逐字节一致。
    semantic_sim: Option<f32>,
) -> RegressionVerdict {
    if original_overlap > 0 {
        RegressionVerdict {
            keep: true,
            evidence: "原查询词面命中",
        }
    } else if bridge_hits >= 2 {
        RegressionVerdict {
            keep: true,
            evidence: "联想桥强关联",
        }
    } else if tag_hits > 0 {
        RegressionVerdict {
            keep: true,
            evidence: "标签共鸣",
        }
    } else if intent_score.is_some_and(|s| s >= ASSOCIATION_INTENT_RESPONSE_THRESHOLD) {
        // P4 第四证据：道体状态级——词面全空但语义回应原意图
        RegressionVerdict {
            keep: true,
            evidence: "原意图回应度",
        }
    } else if deep_semantic_gate_enabled()
        && semantic_sim.is_some_and(|s| s >= deep_semantic_keep_threshold())
    {
        // P5 第五证据：**判据同源**——凭召回阶段自己的语义分通过验收。
        // 位置放在词面证据之后，保证既有语义（词面优先）不被掩盖。
        RegressionVerdict {
            keep: true,
            evidence: "语义同源命中",
        }
    } else {
        RegressionVerdict {
            keep: false,
            evidence: "无共鸣信号·发散噪声",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 高置信度候选收缩() {
        assert_eq!(
            decide_association(&[0.95, 0.60], 0, 4),
            AssociationDecision::Contract
        );
    }

    #[test]
    fn 明确单方向继续一层() {
        assert_eq!(
            decide_association(&[0.70, 0.50], 0, 4),
            AssociationDecision::Single { depth: 1 }
        );
    }

    #[test]
    fn 多方向保留分支() {
        assert_eq!(
            decide_association(&[0.70, 0.68, 0.60], 0, 4),
            AssociationDecision::Branch { width: 3, depth: 1 }
        );
    }

    #[test]
    fn 活性衰减并强化命中记忆() {
        let mut machine = MemoryStateMachine::new();
        machine.activate("a", 0.8);
        machine.activate("b", 0.7);
        machine.activate("a", 0.6);
        assert_eq!(machine.state.active[0].memory_id, "a");
        assert_eq!(machine.state.active[0].hits, 2);
        assert!(machine.state.active[0].activation > machine.state.active[1].activation);
    }

    #[test]
    fn 状态转移生成联想链并限制深度() {
        let mut machine = MemoryStateMachine::new();
        assert!(machine.transition(None, "a", 0.8, 1));
        assert!(machine.transition(Some("a"), "b", 0.6, 2));
        assert_eq!(machine.state.trail.len(), 2);
        assert_eq!(machine.state.trail[1].depth, 2);
    }

    #[test]
    fn 回归过滤排除无关扩散结果() {
        let candidates = vec![("a".to_string(), 0.8), ("x".to_string(), 0.7)];
        let original = HashMap::from([(String::from("a"), 0.7), (String::from("x"), 0.0)]);
        let result = regression_filter(&candidates, &original, 0.6);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "a");
    }

    // 串行化对**进程级**环境变量 `LRC_STATE_BIAS` 的读写（v0.9.11 修竞态）。
    //
    // ## 为什么必须（这是一处真实的测试竞态，不是形式主义）
    // Rust 测试默认**并行**，而环境变量是**进程级共享**的。下面两条配对用例：
    //   - `活性偏置门控必须默认关闭`：清除变量 → 断言默认关
    //   - `活性偏置门控显式开启仍生效`：设为 `"1"` → 断言开启
    //
    // 二者并发时存在真实竞态——若"设为 1"恰好插在"清除"与"读取"之间，
    // 前者会读到 `"1"` 而**偶发失败**（实测：全量并行跑时复现过，
    // 单独跑该用例则始终通过）。
    //
    // 原注释声称"不对变量的具体取值做假设，即可不与其它测试互相干扰"**不成立**：
    // `state_bias_enabled()` 每次调用都**实时读全局变量**，读到什么完全取决于调度时机。
    // 修法与 `dao_regulator::info_gain_threshold_tests` 同款——统一使用
    // **全 crate 唯一**的 env 锁（`crate::test_env::lock()`，v0.9.11 / G6 / CR-20）。
    // 曾用模块私锁，但模块间互不排斥 ⇒ 跨模块改同一变量（如 `LRC_STATE_BIAS`
    // 亦被 `memory_store_tests` 修改）的竞态仍在，只是从必现退化为偶发。

    /// v0.9.8 门控默认值契约：`LRC_STATE_BIAS` **必须默认关闭**。
    ///
    /// **为什么需要这条测试**（不是形式主义）：该开关门控的是 v0.9.7 遗留的
    /// 「道体状态机·联想导航」通路，其中「活性偏置加分」**直接改写检索排序分数**，
    /// 而 `daoti/PREREG_FAIR_STATE_MACHINE.md` 判据 G2 已实测该信号
    /// **无净增量**（三臂齐平，+8pp 门槛未达）⇒ 已被否证。
    ///
    /// 若未来有人把默认值改回开启，本测试会立即失败——**防止已被否证的
    /// 信号在无人察觉的情况下重新进入生产排序**（这类"静默回退"正是
    /// 本项目 §3.55 实测抓到的真实故障模式：测试前提曾依赖该通道偶然生效）。
    ///
    /// 负向验证（已实测）：把实现改回 `.unwrap_or(true)` ⇒ 本测试立即失败，
    /// 证明断言具备鉴别力而非恒真。
    #[test]
    fn 活性偏置门控必须默认关闭() {
        // v0.9.11（G6 / CR-20）：改用**全 crate 唯一**的 env 锁
        //（模块私锁互不排斥 ⇒ 跨模块竞态仍在，如 `memory_store_tests` 亦改 `LRC_STATE_BIAS`）
        let _guard = crate::test_env::lock();
        // 保存并清空环境变量，还原"用户未设置"的默认态。
        // 串行化：持全 crate 唯一 env 锁（见其文档），避免与
        // `活性偏置门控显式开启仍生效` 并发时读到对方写入的 `"1"`。
        let saved = std::env::var_os("LRC_STATE_BIAS");
        std::env::remove_var("LRC_STATE_BIAS");
        let default_state = state_bias_enabled();
        // 先还原现场，再做断言——避免断言 panic 时把变量泄漏给其它测试。
        match saved {
            Some(v) => std::env::set_var("LRC_STATE_BIAS", v),
            None => std::env::remove_var("LRC_STATE_BIAS"),
        }
        assert!(
            !default_state,
            "LRC_STATE_BIAS 必须默认关闭：该通路（活性偏置加分）直接改写排序分数，\
             且已被 PREREG_FAIR_STATE_MACHINE 判据 G2 否证（无净增量）。\
             如需复现 v0.9.7 行为，请显式设置 LRC_STATE_BIAS=1。"
        );
    }

    /// v0.9.8 门控语义契约：显式 `LRC_STATE_BIAS=1` 时必须开启。
    /// 与上一条配对，确保"默认关"没有被实现成"永远关"（逃生开关仍可用）。
    #[test]
    fn 活性偏置门控显式开启仍生效() {
        let _guard = crate::test_env::lock();
        let saved = std::env::var_os("LRC_STATE_BIAS");
        std::env::set_var("LRC_STATE_BIAS", "1");
        let enabled = state_bias_enabled();
        match saved {
            Some(v) => std::env::set_var("LRC_STATE_BIAS", v),
            None => std::env::remove_var("LRC_STATE_BIAS"),
        }
        assert!(
            enabled,
            "LRC_STATE_BIAS=1 必须能开启该通路（对照实验与回归取证依赖它）"
        );
    }

    #[test]
    fn 道体再次校验原查询直接命中保留() {
        let verdict = regression_recheck(2, 0, 0, None, None);
        assert!(verdict.keep);
        assert_eq!(verdict.evidence, "原查询词面命中");
    }

    #[test]
    fn 道体再次校验联想桥强关联保留() {
        let verdict = regression_recheck(0, 3, 0, None, None);
        assert!(verdict.keep);
        assert_eq!(verdict.evidence, "联想桥强关联");
    }

    #[test]
    fn 道体再次校验单泛词不通过() {
        // 仅 1 个联想桥词命中、原查询零重叠、无标签 → 发散噪声，剔除
        let verdict = regression_recheck(0, 1, 0, None, None);
        assert!(!verdict.keep);
        assert_eq!(verdict.evidence, "无共鸣信号·发散噪声");
    }

    #[test]
    fn 道体再次校验标签共鸣兜底() {
        let verdict = regression_recheck(0, 0, 1, None, None);
        assert!(verdict.keep);
        assert_eq!(verdict.evidence, "标签共鸣");
    }

    // === P4：第四证据位（原意图回应度）契约测试 ===

    /// P4.1-1：词面全空但意图相关分 ≥0.5 → 保留，证据"原意图回应度"
    #[test]
    fn 第四证据意图回应度高分保留() {
        let verdict = regression_recheck(0, 0, 0, Some(0.7), None);
        assert!(verdict.keep, "intent_score ≥0.5 时应凭第四证据保留");
        assert_eq!(verdict.evidence, "原意图回应度");
    }

    /// P4.1-2：意图相关分 <0.5 → 第四证据不通过，回退三证据裁决（此处全空 → 剔除）
    #[test]
    fn 第四证据低分不通过回退三证据() {
        let verdict = regression_recheck(0, 0, 0, Some(0.3), None);
        assert!(!verdict.keep, "intent_score <0.5 且词面全空时应仍剔除");
        assert_eq!(verdict.evidence, "无共鸣信号·发散噪声");
    }

    /// P4.1-3：意图分数在场且词面证据存在时，词面证据优先（不掩盖原证据）
    #[test]
    fn 第四证据不与词面证据冲突() {
        let verdict = regression_recheck(2, 0, 0, Some(0.1), None);
        assert!(verdict.keep);
        assert_eq!(verdict.evidence, "原查询词面命中", "词面证据优先于第四证据");
    }

    /// P4.1-4：daemon 离线（None）时第四证据缺省跳过，行为与三证据现状逐字节一致
    #[test]
    fn 第四证据离线缺省跳过() {
        // 词面全空 + intent=None → 与"未接入 P4 前"的现状一致（剔除）
        let verdict = regression_recheck(0, 0, 0, None, None);
        assert!(!verdict.keep);
        assert_eq!(verdict.evidence, "无共鸣信号·发散噪声");
    }

    // ══════════════════════════════════════════════════════════════
    // P5：判据同源 —— 本地语义同源度作为第五证据
    //
    // 根因：召回用语义、验收用词面 ⇒ "语义相关但词面零重叠"的候选被系统性否决。
    // 实测开发端：ROI 命中 311 → 深度路存活 72（23.15%）。
    // ══════════════════════════════════════════════════════════════

    /// P5-1（红→绿）：词面全空但语义余弦达标 → **必须保留**（这是本次修复的目标行为）
    #[test]
    fn 第五证据语义同源命中保留() {
        let _g = crate::test_env::lock();
        std::env::remove_var("LRC_DEEP_SEMANTIC_KEEP");
        let verdict = regression_recheck(0, 0, 0, None, Some(0.8));
        assert!(verdict.keep, "语义余弦 ≥ 阈值时应凭第五证据保留");
        assert_eq!(verdict.evidence, "语义同源命中");
    }

    /// P5-2（负向对照，**不可省**）：语义余弦不达标 → **仍须剔除**。
    /// 缺了这条，等于把验收门开成全通，只是把噪声换了个来源。
    #[test]
    fn 第五证据语义不足仍剔除() {
        let _g = crate::test_env::lock();
        std::env::remove_var("LRC_DEEP_SEMANTIC_KEEP");
        let verdict = regression_recheck(0, 0, 0, None, Some(0.2));
        assert!(!verdict.keep, "语义余弦低于阈值且词面全空时应仍判发散噪声");
        assert_eq!(verdict.evidence, "无共鸣信号·发散噪声");
    }

    /// P5-3：第五证据缺省（None，编码器不可用）时，行为与"未接入 P5 前"逐字节一致
    #[test]
    fn 第五证据缺省时行为不变() {
        let _g = crate::test_env::lock();
        std::env::remove_var("LRC_DEEP_SEMANTIC_KEEP");
        let verdict = regression_recheck(0, 0, 0, None, None);
        assert!(!verdict.keep);
        assert_eq!(verdict.evidence, "无共鸣信号·发散噪声");
    }

    /// P5-4：**词面证据优先**——第五证据不得掩盖更强的词面证据（保持既有语义可读性）
    #[test]
    fn 第五证据不与词面证据冲突() {
        let _g = crate::test_env::lock();
        let verdict = regression_recheck(2, 0, 0, None, Some(0.99));
        assert!(verdict.keep);
        assert_eq!(
            verdict.evidence, "原查询词面命中",
            "词面证据应优先于第五证据"
        );
    }

    /// P5-5：阈值可经环境变量覆盖（标定/消融用），且非法值回退默认
    #[test]
    fn 第五证据阈值可配置且非法值回退() {
        let _g = crate::test_env::lock();
        std::env::set_var("LRC_DEEP_SEMANTIC_KEEP", "0.3");
        assert!((deep_semantic_keep_threshold() - 0.3).abs() < 1e-6);
        // 0.5 在 0.3 阈值下应通过
        assert!(regression_recheck(0, 0, 0, None, Some(0.5)).keep);
        // 非法值（>1）必须回退默认，而不是被当成 1.0 用（否则等于全通）
        std::env::set_var("LRC_DEEP_SEMANTIC_KEEP", "9");
        assert!((deep_semantic_keep_threshold() - DEEP_SEMANTIC_KEEP_DEFAULT).abs() < 1e-6);
        std::env::remove_var("LRC_DEEP_SEMANTIC_KEEP");
    }

    /// P5-6：总开关可关闭 ⇒ 用于**回归归因**（"这个失败是不是我引入的"）
    /// 关闭后行为必须与"未接入 P5 前"逐字节一致（高语义分也不放行）。
    #[test]
    fn 第五证据总开关关闭后行为不变() {
        let _g = crate::test_env::lock();
        std::env::remove_var("LRC_DEEP_SEMANTIC_KEEP");
        std::env::set_var("LRC_DEEP_SEMANTIC_GATE", "0");
        assert!(!deep_semantic_gate_enabled());
        let verdict = regression_recheck(0, 0, 0, None, Some(0.99));
        assert!(!verdict.keep, "总开关关闭时，第五证据必须完全失效");
        assert_eq!(verdict.evidence, "无共鸣信号·发散噪声");
        std::env::remove_var("LRC_DEEP_SEMANTIC_GATE");
        assert!(deep_semantic_gate_enabled(), "移除开关后应恢复默认开启");
    }
}
