// ============================================================
// 许可证: Apache 2.0
// LRC 内置记忆联想状态机。
// ============================================================

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

/// 活性偏置开关（LRC_STATE_BIAS=0 关闭，默认开启）。
/// 实时读取环境变量而非 OnceLock 缓存：逃生开关必须在运行期可切换，
/// 否则一旦首次调用缓存为 true，用户设置 LRC_STATE_BIAS=0 将永远失效。
pub fn state_bias_enabled() -> bool {
    std::env::var("LRC_STATE_BIAS")
        .map(|v| v != "0")
        .unwrap_or(true)
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
/// 3. **标签共鸣**：候选标签与原查询词共享 → 用户主动标注的语义证据。
///
/// 若三种证据皆无 → 判定为"发散噪声"，剔除。
/// 返回保留/剔除判定及证据标签（供联想链输出可观测）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegressionVerdict {
    pub keep: bool,
    pub evidence: &'static str,
}

pub fn regression_recheck(
    original_overlap: usize,
    bridge_hits: usize,
    tag_hits: usize,
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

    #[test]
    fn 道体再次校验原查询直接命中保留() {
        let verdict = regression_recheck(2, 0, 0);
        assert!(verdict.keep);
        assert_eq!(verdict.evidence, "原查询词面命中");
    }

    #[test]
    fn 道体再次校验联想桥强关联保留() {
        let verdict = regression_recheck(0, 3, 0);
        assert!(verdict.keep);
        assert_eq!(verdict.evidence, "联想桥强关联");
    }

    #[test]
    fn 道体再次校验单泛词不通过() {
        // 仅 1 个联想桥词命中、原查询零重叠、无标签 → 发散噪声，剔除
        let verdict = regression_recheck(0, 1, 0);
        assert!(!verdict.keep);
        assert_eq!(verdict.evidence, "无共鸣信号·发散噪声");
    }

    #[test]
    fn 道体再次校验标签共鸣兜底() {
        let verdict = regression_recheck(0, 0, 1);
        assert!(verdict.keep);
        assert_eq!(verdict.evidence, "标签共鸣");
    }
}
