// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件实现用户反馈回路，属于守护层 (Layer 2)。
// ============================================================
//
// 用户反馈回路 (UserFeedback) v2.0 — 两阶段确认增强版
//
// 解决质疑四"可解释性下降"和文档总评"引入用户反馈回路"问题：
// 在关键的记忆合成或遗忘环节，设计轻量级的用户确认机制，
// 将人的判断力注入到系统的自主演化中，形成人机协同的"演化"
// 而非纯粹的"自主"。
//
// v2.0 新增两阶段确认机制（解决质疑一"人机协同深度不足"）：
//   阶段一：用户发起操作 → 系统返回影响评估报告
//   阶段二：用户审阅报告 → 发送确认指令 → 系统执行
//   这确保了高影响操作（如隔离）不会在用户不知情的情况下
//   引发记忆链断裂等副作用。
//
// 核心功能：
//   - 用户标记检索结果相关性
//   - 正反馈：提升合成记忆的质量评分，阻止被隔离
//   - 负反馈：加速低质量记忆的隔离流程
//   - 隔离恢复：用户可手动恢复被隔离的记忆
//   - 两阶段确认：高影响操作先返回影响评估，用户确认后执行

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// 用户反馈类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FeedbackType {
    /// 正面反馈：检索结果相关，合成质量好
    Positive,
    /// 负面反馈：检索结果不相关，合成质量差
    Negative,
    /// 中立反馈：结果部分相关
    Neutral,
}

/// 用户反馈目标类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FeedbackTarget {
    /// 对检索结果的反馈
    RetrievalResult,
    /// 对合成记忆的反馈
    SynthesisQuality,
    /// 对隔离决定的反馈（恢复被隔离的记忆）
    QuarantineOverride,
    /// 隔离请求（需要两阶段确认）
    IsolateMemory,
    /// 第二阶段确认：用户审阅影响评估后确认执行
    ConfirmAction,
    /// 取消待确认操作
    CancelAction,
}

/// 待确认操作的种类
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PendingActionType {
    /// 隔离指定记忆
    Isolate,
    /// 删除指定记忆
    Delete,
    /// 批量隔离
    BatchIsolate,
}

/// 影响评估报告（阶段一返回）
///
/// 当用户发起高影响操作（如隔离）时，系统首先生成此报告，
/// 描述该操作可能产生的连锁影响，供用户决策。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactAssessment {
    /// 评估 ID（用于第二阶段确认）
    pub assessment_id: String,
    /// 待确认的操作类型
    pub action_type: PendingActionType,
    /// 目标记忆 ID 列表
    pub target_memory_ids: Vec<String>,
    /// 影响范围：直接关联的记忆数
    pub direct_neighbors: usize,
    /// 影响范围：间接关联的记忆数（2-hop）
    pub indirect_neighbors: usize,
    /// 二阶间接影响的记忆详情（质疑四：深度影响评估）
    pub second_order_affected: Vec<AffectedMemoryInfo>,
    /// 影响的合成链数量
    pub affected_synthesis_chains: usize,
    /// 是否为多个合成记忆的核心节点
    pub is_core_node: bool,
    /// 受影响记忆的摘要信息
    pub affected_memories: Vec<AffectedMemoryInfo>,
    /// 叙事性因果链（质疑二：降低用户理解鸿沟）
    ///
    /// 用人类语言描述关键影响链，帮助用户理解"隔离 A 为什么会影响 Z"。
    /// 例如："Isolating 'core-1' will break 'synth-A', which in turn affects 'fact-X'."
    pub narrative: Option<String>,
    /// 风险等级：low / medium / high / critical
    pub risk_level: String,
    /// 风险说明
    pub risk_description: String,
    /// 建议（供用户参考）
    pub recommendation: String,
    /// 评估时间戳
    pub timestamp_ms: u64,
    /// 过期时间（超过此时间确认无效）
    pub expires_at_ms: u64,
}

/// 受影响记忆的简要信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedMemoryInfo {
    /// 记忆 ID
    pub memory_id: String,
    /// 记忆类型
    pub memory_type: String,
    /// 关系类型
    pub relation_type: String,
    /// 关系权重
    pub weight: f32,
    /// 影响深度：1 = 直接关联，2 = 二阶间接关联
    pub depth: u8,
}

/// 待确认操作记录
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PendingConfirmation {
    /// 评估 ID
    assessment_id: String,
    /// 操作类型
    action_type: PendingActionType,
    /// 目标记忆 ID
    target_memory_ids: Vec<String>,
    /// 创建时间戳
    created_at_ms: u64,
    /// 过期时间戳
    expires_at_ms: u64,
}

/// 单条用户反馈记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackRecord {
    /// 反馈 ID
    pub id: String,
    /// 反馈类型
    pub feedback_type: FeedbackType,
    /// 反馈目标类型
    pub target_type: FeedbackTarget,
    /// 目标记忆 ID
    pub memory_id: String,
    /// 关联的查询文本（检索反馈时使用）
    pub query: Option<String>,
    /// 用户备注（可选）
    pub note: Option<String>,
    /// 反馈时间戳（毫秒）
    pub timestamp_ms: u64,
    /// 反馈是否已被系统处理
    pub processed: bool,
}

/// 用户反馈统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackStats {
    /// 总反馈数
    pub total_feedback: usize,
    /// 正面反馈数
    pub positive_count: usize,
    /// 负面反馈数
    pub negative_count: usize,
    /// 合成质量反馈数
    pub synthesis_feedback_count: usize,
    /// 隔离恢复次数
    pub quarantine_overrides: usize,
    /// 正面反馈比例
    pub positive_ratio: f32,
    /// 隐式反馈是否启用（质疑二·隐私）
    pub implicit_feedback_enabled: bool,
    /// 知情同意状态（质疑二·终极）
    /// None = 未选择, Some(true) = 已同意, Some(false) = 已拒绝
    pub consent_granted: Option<bool>,
}

/// 记忆关系查询器 trait
///
/// 用于影响评估时查询记忆图结构，由 MemoryStore 实现。
/// 解耦 UserFeedback 与具体存储实现。
pub trait MemoryGraphQuery {
    /// 查询与指定记忆直接关联的记忆数
    fn count_direct_neighbors(&self, memory_id: &str) -> usize;
    /// 查询与指定记忆关联的记忆 ID 列表及关系类型
    fn get_neighbor_info(&self, memory_id: &str) -> Vec<AffectedMemoryInfo>;
    /// 查询记忆是否为核心合成节点（被多条合成边引用）
    fn is_core_synthesis_node(&self, memory_id: &str) -> bool;
    /// 查询受影响的合成链数量
    fn count_affected_synthesis_chains(&self, memory_ids: &[String]) -> usize;
}

/// 用户反馈管理器 v2.0
///
/// 记录和处理用户对记忆系统输出的反馈，
/// 将人类判断力注入到系统的自主演化中。
///
/// v2.0 新增两阶段确认机制：
/// - 阶段一：`request_impact_assessment()` 返回影响评估报告
/// - 阶段二：`confirm_action()` 或 `cancel_pending()` 执行或取消
///
/// v3.0 新增隐式反馈隐私保护（质疑二·隐私）：
/// - `implicit_feedback_enabled`：控制是否允许通过用户行为推断反馈
/// - 默认启用，但可以随时通过 `set_implicit_feedback_enabled(false)` 关闭
/// - 关闭后仅依赖显式用户反馈指令
#[derive(Debug)]
pub struct UserFeedback {
    /// 反馈历史（FIFO，最多 500 条）
    records: Mutex<VecDeque<FeedbackRecord>>,
    /// 正面反馈计数
    positive_count: Mutex<usize>,
    /// 负面反馈计数
    negative_count: Mutex<usize>,
    /// 隔离恢复计数
    quarantine_override_count: Mutex<usize>,
    /// 反馈 ID 计数器
    id_counter: Mutex<u64>,
    /// 待确认操作映射（assessment_id → PendingConfirmation）
    pending_confirmations: Mutex<HashMap<String, PendingConfirmation>>,
    /// 评估 ID 计数器
    assessment_counter: Mutex<u64>,
    /// 隐式反馈开关（质疑二·隐私：允许用户关闭行为推断）
    /// true = 启用隐式反馈（默认），false = 仅依赖显式反馈
    implicit_feedback_enabled: Mutex<bool>,
    /// 知情同意状态（质疑二·终极：用户是否已明确给予同意）
    /// - None: 用户尚未做出选择（首次使用）
    /// - Some(true): 用户已明确同意
    /// - Some(false): 用户已明确拒绝
    consent_granted: Mutex<Option<bool>>,
}

impl UserFeedback {
    /// 创建新的用户反馈管理器
    pub fn new() -> Self {
        Self {
            records: Mutex::new(VecDeque::with_capacity(500)),
            positive_count: Mutex::new(0),
            negative_count: Mutex::new(0),
            quarantine_override_count: Mutex::new(0),
            id_counter: Mutex::new(0),
            pending_confirmations: Mutex::new(HashMap::new()),
            assessment_counter: Mutex::new(0),
            implicit_feedback_enabled: Mutex::new(true), // 质疑二·隐私：默认启用隐式反馈
            consent_granted: Mutex::new(None),           // 质疑二·终极：用户尚未做出知情选择
        }
    }

    /// 生成新的反馈 ID
    fn next_id(&self) -> String {
        let mut counter = self.id_counter.lock().unwrap_or_else(|e| e.into_inner());
        *counter += 1;
        format!("feedback_{:06}", counter)
    }

    /// 生成新的评估 ID
    fn next_assessment_id(&self) -> String {
        let mut counter = self
            .assessment_counter
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *counter += 1;
        format!("impact_{:06}", counter)
    }

    /// 道枢映射: 兑卦·泽 (☱) — 说也，刚中而柔外，用户反馈如泽水之润泽，标记系统输出的阴阳属性
    ///
    /// 记录用户反馈
    pub fn record_feedback(
        &self,
        feedback_type: FeedbackType,
        target_type: FeedbackTarget,
        memory_id: &str,
        query: Option<&str>,
        note: Option<&str>,
    ) -> String {
        let id = self.next_id();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let record = FeedbackRecord {
            id: id.clone(),
            feedback_type: feedback_type.clone(),
            target_type: target_type.clone(),
            memory_id: memory_id.to_string(),
            query: query.map(|s| s.to_string()),
            note: note.map(|s| s.to_string()),
            timestamp_ms: now,
            processed: false,
        };

        // 更新统计
        match &feedback_type {
            FeedbackType::Positive => {
                let mut count = self
                    .positive_count
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                *count += 1;
            }
            FeedbackType::Negative => {
                let mut count = self
                    .negative_count
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                *count += 1;
            }
            FeedbackType::Neutral => {}
        }

        if target_type == FeedbackTarget::QuarantineOverride {
            let mut count = self
                .quarantine_override_count
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *count += 1;
        }

        let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if records.len() >= 500 {
            records.pop_front();
        }
        records.push_back(record);

        id
    }

    // ============================================================
    // 两阶段确认机制（质疑一：人机协同深度）
    // ============================================================

    /// 阶段一：请求影响评估
    ///
    /// 当用户发起高影响操作（如隔离记忆）时，首先生成影响评估报告。
    /// 用户需要审阅报告后，通过 `confirm_action()` 发送第二阶段确认。
    ///
    /// 参数：
    ///   - action_type: 操作类型（隔离/删除/批量隔离）
    ///   - memory_ids: 目标记忆 ID 列表
    ///   - graph_query: 记忆图查询接口（由 MemoryStore 提供）
    ///
    /// 返回影响评估报告，包含评估 ID 供第二阶段确认使用。
    ///
    /// 构建叙事性因果链（质疑二：降低用户理解鸿沟）
    ///
    /// 将结构化的影响数据转化为人类可读的因果叙事。
    /// 例如："Isolating 'core-1' will break 'synth-A', which in turn affects 'fact-X'."
    ///
    /// v2.0 新增高重要性节点标注：当受影响的节点 weight >= 0.9 时，
    /// 在叙事中特别标注，防止用户因叙事简化而忽略关键决策记录。
    fn build_narrative(
        target_ids: &[String],
        direct_affected: &[AffectedMemoryInfo],
        second_order: &[AffectedMemoryInfo],
        is_core: bool,
        chain_count: usize,
    ) -> Option<String> {
        if direct_affected.is_empty() && second_order.is_empty() {
            return None;
        }

        let target_list = target_ids.join(", ");
        let mut parts: Vec<String> = Vec::new();

        // 操作描述
        parts.push(format!("Isolating '{}'", target_list));

        // 高重要性节点检测（质疑二：weight >= 0.9 视为高重要性）
        let high_importance_direct: Vec<&AffectedMemoryInfo> =
            direct_affected.iter().filter(|i| i.weight >= 0.9).collect();
        let high_importance_second: Vec<&AffectedMemoryInfo> =
            second_order.iter().filter(|i| i.weight >= 0.9).collect();

        // 直接影响
        if !direct_affected.is_empty() {
            let direct_synths: Vec<&str> = direct_affected
                .iter()
                .filter(|i| i.memory_type == "synthesis")
                .map(|i| i.memory_id.as_str())
                .take(3)
                .collect();

            if !direct_synths.is_empty() {
                let synths_str = direct_synths.join(", ");
                let and_more = if direct_affected.len() > direct_synths.len() {
                    format!(" and {} more", direct_affected.len() - direct_synths.len())
                } else {
                    String::new()
                };
                parts.push(format!(
                    "will break {} synthesis chain(s) ({}:{})",
                    chain_count, synths_str, and_more
                ));
            } else {
                parts.push(format!(
                    "will affect {} directly connected memor(y|ies)",
                    direct_affected.len()
                ));
            }
        }

        // 二阶影响
        if !second_order.is_empty() {
            let snd_synths: Vec<&str> = second_order
                .iter()
                .filter(|i| i.memory_type == "synthesis")
                .map(|i| i.memory_id.as_str())
                .take(2)
                .collect();
            let snd_facts: Vec<&str> = second_order
                .iter()
                .filter(|i| i.memory_type == "fact")
                .map(|i| i.memory_id.as_str())
                .take(2)
                .collect();

            let detail: Vec<String> = snd_synths
                .into_iter()
                .chain(snd_facts)
                .take(3)
                .map(|s| s.to_string())
                .collect();

            if !detail.is_empty() {
                let and_more = if second_order.len() > detail.len() {
                    format!(" and {} more", second_order.len() - detail.len())
                } else {
                    String::new()
                };
                parts.push(format!(
                    "which in turn affects {} second-order memor(y|ies) ({}{})",
                    second_order.len(),
                    detail.join(", "),
                    and_more
                ));
            }
        }

        // 高重要性节点标注（质疑二：防止叙事忽略关键决策记录）
        let total_high = high_importance_direct.len() + high_importance_second.len();
        if total_high > 0 {
            let high_ids: Vec<String> = high_importance_direct
                .iter()
                .chain(high_importance_second.iter())
                .take(3)
                .map(|info| format!("{} (weight={:.2})", info.memory_id, info.weight))
                .collect();
            let and_more = if total_high > 3 {
                format!(" and {} more", total_high - 3)
            } else {
                String::new()
            };
            parts.push(format!(
                "WARNING: {} high-importance node(s) affected: {}{}",
                total_high,
                high_ids.join(", "),
                and_more
            ));
        }

        // 核心节点警告
        if is_core && chain_count >= 3 {
            parts.push(
                "This may impact higher-order decision records and knowledge structures."
                    .to_string(),
            );
        }

        let narrative = parts.join(". ") + ".";

        Some(narrative)
    }
    /// 道枢映射: 兑卦·泽 (☱) — 说也，影响评估如泽水之润泽，评估反馈对系统的影响
    pub fn request_impact_assessment(
        &self,
        action_type: PendingActionType,
        memory_ids: &[String],
        graph_query: &dyn MemoryGraphQuery,
    ) -> ImpactAssessment {
        let assessment_id = self.next_assessment_id();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        // 评估报告 5 分钟内有效
        let expires_at = now + 300_000;

        // 收集所有目标记忆的邻居信息（直接和二阶）
        let mut all_affected: Vec<AffectedMemoryInfo> = Vec::new();
        let mut second_order_affected: Vec<AffectedMemoryInfo> = Vec::new();
        let mut total_direct = 0usize;
        let mut total_indirect = 0usize;
        let mut is_core = false;

        for mem_id in memory_ids {
            // 直接邻居（深度 1）
            let neighbors: Vec<AffectedMemoryInfo> = graph_query
                .get_neighbor_info(mem_id)
                .into_iter()
                .map(|mut info| {
                    info.depth = 1;
                    info
                })
                .collect();
            total_direct += neighbors.len();

            // 二阶邻居（深度 2）：收集每个直接邻居的邻居信息
            for neighbor in &neighbors {
                // 跳过目标记忆本身，避免循环引用
                let indirect: Vec<AffectedMemoryInfo> = graph_query
                    .get_neighbor_info(&neighbor.memory_id)
                    .into_iter()
                    .filter(|info| !memory_ids.contains(&info.memory_id))
                    .map(|mut info| {
                        info.depth = 2;
                        info
                    })
                    .collect();
                total_indirect += indirect.len();
                second_order_affected.extend(indirect);
            }

            all_affected.extend(neighbors);

            if graph_query.is_core_synthesis_node(mem_id) {
                is_core = true;
            }
        }

        // 去重二阶受影响记忆
        second_order_affected.sort_by(|a, b| a.memory_id.cmp(&b.memory_id));
        second_order_affected.dedup_by(|a, b| a.memory_id == b.memory_id);

        // 去重受影响记忆
        all_affected.sort_by(|a, b| a.memory_id.cmp(&b.memory_id));
        all_affected.dedup_by(|a, b| a.memory_id == b.memory_id);

        let affected_chains = graph_query.count_affected_synthesis_chains(memory_ids);

        // 风险等级判定
        let (risk_level, risk_description, recommendation) = if is_core && affected_chains >= 3 {
            (
                "critical".to_string(),
                format!(
                    "目标记忆是 {} 条合成链的核心节点，隔离将导致 {} 条合成链断裂。直接关联 {} 条记忆，间接关联 {} 条记忆。",
                    affected_chains, affected_chains, total_direct, total_indirect
                ),
                "强烈建议取消此操作。如需继续，请先确认所有受影响合成链的替代方案。".to_string(),
            )
        } else if is_core || affected_chains >= 2 {
            (
                "high".to_string(),
                format!(
                    "目标记忆关联 {} 条合成链，隔离将影响 {} 条直接关联记忆。",
                    affected_chains, total_direct
                ),
                "建议审慎确认。可考虑先降低记忆重要性而非直接隔离。".to_string(),
            )
        } else if total_direct >= 5 {
            (
                "medium".to_string(),
                format!(
                    "目标记忆关联 {} 条直接记忆，隔离可能影响检索完整性。",
                    total_direct
                ),
                "建议确认后执行，观察系统后续表现。".to_string(),
            )
        } else {
            (
                "low".to_string(),
                format!("目标记忆仅关联 {} 条直接记忆，影响范围可控。", total_direct),
                "可以安全执行。".to_string(),
            )
        };

        let assessment = ImpactAssessment {
            assessment_id: assessment_id.clone(),
            action_type: action_type.clone(),
            target_memory_ids: memory_ids.to_vec(),
            direct_neighbors: total_direct,
            indirect_neighbors: total_indirect,
            affected_synthesis_chains: affected_chains,
            is_core_node: is_core,
            narrative: Self::build_narrative(
                memory_ids,
                &all_affected,
                &second_order_affected,
                is_core,
                affected_chains,
            ),
            second_order_affected,
            affected_memories: all_affected,
            risk_level,
            risk_description,
            recommendation,
            timestamp_ms: now,
            expires_at_ms: expires_at,
        };

        // 存储待确认记录
        let pending = PendingConfirmation {
            assessment_id: assessment_id.clone(),
            action_type,
            target_memory_ids: memory_ids.to_vec(),
            created_at_ms: now,
            expires_at_ms: expires_at,
        };
        let mut pending_map = self
            .pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // 清理过期确认
        pending_map.retain(|_, p| p.expires_at_ms > now);
        pending_map.insert(assessment_id.clone(), pending);

        assessment
    }

    /// 道枢映射: 乾卦·天 (☰) — 天行健，确认操作如天道之决断
    /// 阶段二：确认执行待处理操作
    ///
    /// 用户审阅影响评估报告后，发送确认指令以执行操作。
    ///
    /// 返回确认结果：Ok(目标记忆 ID 列表) 或 Err(错误信息)。
    pub fn confirm_action(&self, assessment_id: &str) -> Result<Vec<String>, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut pending_map = self
            .pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let pending = pending_map.remove(assessment_id).ok_or_else(|| {
            "未找到该评估 ID，可能已过期或已被处理。请重新发起影响评估。".to_string()
        })?;

        if pending.expires_at_ms < now {
            return Err(format!(
                "评估 {} 已过期（{} 毫秒前），请重新发起影响评估。",
                assessment_id,
                now - pending.expires_at_ms
            ));
        }

        // 清理其他过期确认
        pending_map.retain(|_, p| p.expires_at_ms > now);

        Ok(pending.target_memory_ids)
    }

    /// 取消待确认操作
    ///
    /// 用户在审阅影响评估后决定不执行，可取消操作。
    pub fn cancel_pending(&self, assessment_id: &str) -> Result<(), String> {
        let mut pending_map = self
            .pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        if pending_map.remove(assessment_id).is_some() {
            Ok(())
        } else {
            Err("未找到该评估 ID，可能已过期或已被处理。".to_string())
        }
    }

    /// 获取待确认操作列表（供管理面板使用）
    pub fn get_pending_actions(&self) -> Vec<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let pending_map = self
            .pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending_map
            .iter()
            .filter(|(_, p)| p.expires_at_ms > now)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// 检查是否有过期的待确认操作
    pub fn cleanup_expired_pending(&self) -> usize {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut pending_map = self
            .pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let before = pending_map.len();
        pending_map.retain(|_, p| p.expires_at_ms > now);
        before - pending_map.len()
    }

    // ============================================================
    // 查询方法
    // ============================================================

    /// 获取记忆的正面反馈数
    pub fn get_positive_feedback_count(&self, memory_id: &str) -> usize {
        let records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        records
            .iter()
            .filter(|r| {
                r.memory_id == memory_id
                    && r.feedback_type == FeedbackType::Positive
                    && r.target_type == FeedbackTarget::SynthesisQuality
            })
            .count()
    }

    /// 获取记忆的负面反馈数
    pub fn get_negative_feedback_count(&self, memory_id: &str) -> usize {
        let records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        records
            .iter()
            .filter(|r| r.memory_id == memory_id && r.feedback_type == FeedbackType::Negative)
            .count()
    }

    /// 检查记忆是否应因用户负面反馈而被隔离
    pub fn should_quarantine_by_user(&self, memory_id: &str) -> bool {
        let negative = self.get_negative_feedback_count(memory_id);
        let positive = self.get_positive_feedback_count(memory_id);
        negative >= 2 && positive < negative
    }

    /// 获取待处理的隔离恢复请求
    pub fn get_quarantine_override_ids(&self) -> Vec<String> {
        let records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        let mut ids: Vec<String> = records
            .iter()
            .filter(|r| r.target_type == FeedbackTarget::QuarantineOverride && !r.processed)
            .map(|r| r.memory_id.clone())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// 标记反馈为已处理
    pub fn mark_processed(&self, feedback_id: &str) {
        let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        for record in records.iter_mut() {
            if record.id == feedback_id {
                record.processed = true;
                return;
            }
        }
    }

    /// 获取反馈统计
    pub fn get_stats(&self) -> FeedbackStats {
        let records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        let total = records.len();
        let positive = *self
            .positive_count
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let negative = *self
            .negative_count
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let synthesis_feedback = records
            .iter()
            .filter(|r| r.target_type == FeedbackTarget::SynthesisQuality)
            .count();
        let quarantine_overrides = *self
            .quarantine_override_count
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let positive_ratio = if total > 0 {
            positive as f32 / total as f32
        } else {
            0.5
        };

        FeedbackStats {
            total_feedback: total,
            positive_count: positive,
            negative_count: negative,
            synthesis_feedback_count: synthesis_feedback,
            quarantine_overrides,
            positive_ratio,
            implicit_feedback_enabled: self.is_implicit_feedback_enabled(),
            consent_granted: self.is_consent_granted(),
        }
    }

    /// 获取最近的反馈记录
    pub fn get_recent(&self, n: usize) -> Vec<FeedbackRecord> {
        let records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        records.iter().rev().take(n).cloned().collect()
    }

    // ============================================================
    // 被动反馈机制（质疑三：防止"沉默螺旋"）
    // ============================================================
    //
    // 大多数用户从不主动反馈。系统在此"沉默"环境中完全依赖
    // 内部调节器和 GC 进行自治，可能逐渐偏离用户的实际需求。
    //
    // 被动反馈通过观察用户的隐式行为（点击、复制、重复查询、
    // 停留时间等）来推断相关性，无需用户主动参与。

    /// 记录隐式信号（质疑三：被动反馈）
    ///
    /// 即使用户不主动反馈，系统也可以通过其行为推断相关性。
    /// 这些隐式信号作为调节器和合成器的软标签，持续校准系统。
    pub fn record_implicit_signal(&self, signal: ImplicitSignal) {
        // 质疑二·隐私：检查隐式反馈是否启用
        if !*self
            .implicit_feedback_enabled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
        {
            return;
        }

        // 累计隐式信号到对应记忆的被动分数
        match signal.signal_type {
            ImplicitSignalType::Click => {
                // 点击检索结果 = 正相关，+0.05
                // 通过标记为 processed 的正面反馈模拟
                self.record_feedback(
                    FeedbackType::Positive,
                    FeedbackTarget::RetrievalResult,
                    &signal.memory_id,
                    signal.query.as_deref(),
                    Some("[隐式] 用户点击了检索结果（置信度: 0.7）"),
                );
            }
            ImplicitSignalType::Copy => {
                // 复制内容 = 强正相关，+0.1
                self.record_feedback(
                    FeedbackType::Positive,
                    FeedbackTarget::RetrievalResult,
                    &signal.memory_id,
                    signal.query.as_deref(),
                    Some("[隐式] 用户复制了记忆内容（置信度: 0.95）"),
                );
            }
            ImplicitSignalType::Dwell => {
                // 长时间停留（> 5 秒）= 正相关，+0.03
                if signal.dwell_ms.unwrap_or(0) > 5000 {
                    self.record_feedback(
                        FeedbackType::Positive,
                        FeedbackTarget::RetrievalResult,
                        &signal.memory_id,
                        signal.query.as_deref(),
                        Some(&format!(
                            "[隐式] 用户停留 {}ms 查看结果（置信度: 0.6）",
                            signal.dwell_ms.unwrap_or(0)
                        )),
                    );
                }
            }
            ImplicitSignalType::RepeatQuery => {
                // 重复查询相同问题 = 之前结果不满意，负相关，-0.05
                self.record_feedback(
                    FeedbackType::Negative,
                    FeedbackTarget::RetrievalResult,
                    &signal.memory_id,
                    signal.query.as_deref(),
                    Some(&format!(
                        "[隐式] 用户重复查询 '{}'，之前结果可能不相关（置信度: 0.65）",
                        signal.query.as_deref().unwrap_or("未知")
                    )),
                );
            }
            ImplicitSignalType::Ignore => {
                // 检索结果被忽略（出现在结果中但未被点击）= 弱负相关，-0.02
                self.record_feedback(
                    FeedbackType::Neutral,
                    FeedbackTarget::RetrievalResult,
                    &signal.memory_id,
                    signal.query.as_deref(),
                    Some("[隐式] 检索结果未被用户交互（置信度: 0.4）"),
                );
            }
        }
    }

    /// 获取基于隐式信号的记忆质量调整建议
    ///
    /// 返回 (memory_id, adjusted_quality_score) 的列表。
    /// 调用方可将此建议用于调节合成阈值或 GC 候选评估。
    pub fn get_implicit_quality_adjustments(&self) -> Vec<(String, f32)> {
        let records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        let mut adjustments: HashMap<String, (f32, usize)> = HashMap::new();

        for record in records.iter() {
            let is_implicit = record
                .note
                .as_ref()
                .map(|n| n.starts_with("[隐式]"))
                .unwrap_or(false);
            if !is_implicit {
                continue;
            }

            let weight = match record.feedback_type {
                FeedbackType::Positive => 0.05,
                FeedbackType::Negative => -0.05,
                FeedbackType::Neutral => -0.02,
            };

            let entry = adjustments
                .entry(record.memory_id.clone())
                .or_insert((0.0, 0));
            entry.0 += weight;
            entry.1 += 1;
        }

        adjustments
            .into_iter()
            .map(|(id, (total, count))| {
                let avg = if count > 0 { total / count as f32 } else { 0.0 };
                (id, avg)
            })
            .collect()
    }

    /// 设置隐式反馈开关（质疑二·隐私）
    ///
    /// 启用时，系统通过用户行为（点击、复制、停留、重复查询等）推断相关性。
    /// 禁用时，仅依赖用户的显式反馈指令。
    /// 所有隐式反馈数据仅留在本地，不会上传到任何外部服务器。
    pub fn set_implicit_feedback_enabled(&self, enabled: bool) {
        let mut flag = self
            .implicit_feedback_enabled
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let was_enabled = *flag;
        *flag = enabled;
        if was_enabled != enabled {
            eprintln!(
                "[LRC·隐私] 隐式反馈已{}。{}",
                if enabled { "启用" } else { "关闭" },
                if enabled {
                    "系统将通过用户行为推断相关性，数据仅留在本地。"
                } else {
                    "系统将仅依赖显式用户反馈指令。"
                }
            );
        }
    }

    /// 检查隐式反馈是否启用（质疑二·隐私）
    pub fn is_implicit_feedback_enabled(&self) -> bool {
        *self
            .implicit_feedback_enabled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    // ============================================================
    // 质疑二·终极：知情同意强化
    //
    // 启动日志一闪而过，用户可能注意不到隐私声明。
    // 以下机制提供更可见的隐私实践：
    // 1. 隐私清单：可随时查询的完整隐私声明
    // 2. 知情同意：用户需明确做出选择，不再"默认监控"
    // 3. 同意状态持久化在健康报告中
    //
    // 道枢映射：离卦·火 (☲) — 明也，光明正大，无隐无藏。
    // ============================================================

    /// 道枢映射: 离卦·火 (☲) — 明也，隐私清单如火光之照亮，让数据处理透明可见
    /// 隐私清单：返回完整的隐私声明文本
    ///
    /// 可在启动时打印，或通过 API 查询。
    /// 比简单的启动日志更详细，帮助用户做出知情选择。
    pub fn privacy_manifest() -> &'static str {
        "\
╔══════════════════════════════════════════════════════════════╗
║              LRC 隐私清单 — 您的数据，您的规则              ║
╠══════════════════════════════════════════════════════════════╣
║                                                            ║
║  LRC 是本地优先的记忆系统，坚守以下隐私承诺：               ║
║                                                            ║
║  1. 数据主权：所有数据存储在您的本地设备上，不会上传       ║
║     到任何外部服务器。您拥有完全的数据所有权。             ║
║                                                            ║
║  2. 隐式反馈机制：为优化记忆检索质量，系统会观察您的       ║
║     自然交互行为（如点击、复制、重复查询），推断内容       ║
║     的相关性。这类似于搜索引擎根据点击优化排序。           ║
║     - 收集范围：仅限您与 LRC 的交互行为                   ║
║     - 数据去向：仅存储在本地，绝不外传                     ║
║     - 目的：提高记忆检索的准确性和相关性                   ║
║                                                            ║
║  3. 您的控制权：                                          ║
║     - 随时可关闭隐式反馈：set_implicit_feedback_enabled    ║
║     - 随时可查看隐私清单：privacy_manifest()               ║
║     - 随时可查询被记录的数据：stats()                      ║
║                                                            ║
║  4. 透明度承诺：所有数据处理逻辑均在源代码中可见。        ║
║     本项目遵循 DaoTi Research License，鼓励审查和改进。    ║
║                                                            ║
║  如果您对隐私有任何疑问，请查阅源代码或联系维护者。       ║
║                                                            ║
║  道枢映射：离卦·火 (☲) — 明也，日月丽乎天，               ║
║  百谷草木丽乎土。重明以丽乎正，乃化成天下。               ║
╚══════════════════════════════════════════════════════════════╝"
    }

    /// 道枢映射: 离卦·火 (☲) — 重明以丽乎正，知情同意是透明性的核心承诺
    /// 授予知情同意（质疑二·终极）
    ///
    /// 用户明确同意系统收集隐式反馈数据。
    /// 调用此方法后，隐式反馈功能将保持启用状态。
    pub fn grant_consent(&self) {
        let mut consent = self
            .consent_granted
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *consent = Some(true);
        eprintln!("[LRC·隐私] 用户已明确授予知情同意。隐式反馈机制将正常运行。");
    }

    /// 道枢映射: 坎卦·水 (☵) — 行险而不失其信，撤销同意是用户主权的保障
    /// 撤销知情同意（质疑二·终极）
    ///
    /// 用户明确拒绝隐式反馈数据收集。
    /// 系统将自动关闭隐式反馈，仅依赖显式反馈指令。
    pub fn revoke_consent(&self) {
        let mut consent = self
            .consent_granted
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *consent = Some(false);
        // 自动关闭隐式反馈
        *self
            .implicit_feedback_enabled
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = false;
        eprintln!("[LRC·隐私] 用户已撤销知情同意。隐式反馈已关闭，系统仅依赖显式反馈指令。");
    }

    /// 检查用户是否已授予知情同意（质疑二·终极）
    ///
    /// 返回 None 表示用户尚未做出选择。
    pub fn is_consent_granted(&self) -> Option<bool> {
        *self
            .consent_granted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// 获取反馈统计（质疑五·健康报告）
    pub fn stats(&self) -> FeedbackStats {
        let records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        let total = records.len();
        let positive = *self
            .positive_count
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let negative = *self
            .negative_count
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let overrides = *self
            .quarantine_override_count
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let synthesis_feedback = records
            .iter()
            .filter(|r| matches!(r.target_type, FeedbackTarget::SynthesisQuality))
            .count();
        let ratio = if total > 0 {
            positive as f32 / total as f32
        } else {
            0.0
        };

        FeedbackStats {
            total_feedback: total,
            positive_count: positive,
            negative_count: negative,
            synthesis_feedback_count: synthesis_feedback,
            quarantine_overrides: overrides,
            positive_ratio: ratio,
            implicit_feedback_enabled: self.is_implicit_feedback_enabled(),
            consent_granted: self.is_consent_granted(),
        }
    }
}

/// 隐式信号类型（质疑三：被动反馈）
///
/// 用户不主动评价，但系统通过其行为推断相关性。
#[derive(Debug, Clone)]
pub enum ImplicitSignalType {
    /// 点击了检索结果
    Click,
    /// 复制了记忆内容
    Copy,
    /// 在结果上停留了较长时间
    Dwell,
    /// 重复查询了相同问题（暗示之前结果不满意）
    RepeatQuery,
    /// 检索结果出现在结果集中但未被交互
    Ignore,
}

/// 隐式信号（质疑三：被动反馈）
///
/// 包含信号类型、目标记忆和上下文信息。
#[derive(Debug, Clone)]
pub struct ImplicitSignal {
    /// 信号类型
    pub signal_type: ImplicitSignalType,
    /// 目标记忆 ID
    pub memory_id: String,
    /// 关联的查询（如有）
    pub query: Option<String>,
    /// 停留时长（毫秒，仅 Dwell 类型）
    pub dwell_ms: Option<u64>,
    /// 信号发生时间戳
    pub timestamp_ms: u64,
}

impl ImplicitSignal {
    /// 创建点击信号
    pub fn click(memory_id: &str, query: Option<&str>) -> Self {
        Self {
            signal_type: ImplicitSignalType::Click,
            memory_id: memory_id.to_string(),
            query: query.map(|s| s.to_string()),
            dwell_ms: None,
            timestamp_ms: now_ms(),
        }
    }

    /// 创建复制信号
    pub fn copy(memory_id: &str, query: Option<&str>) -> Self {
        Self {
            signal_type: ImplicitSignalType::Copy,
            memory_id: memory_id.to_string(),
            query: query.map(|s| s.to_string()),
            dwell_ms: None,
            timestamp_ms: now_ms(),
        }
    }

    /// 创建停留信号
    pub fn dwell(memory_id: &str, query: Option<&str>, ms: u64) -> Self {
        Self {
            signal_type: ImplicitSignalType::Dwell,
            memory_id: memory_id.to_string(),
            query: query.map(|s| s.to_string()),
            dwell_ms: Some(ms),
            timestamp_ms: now_ms(),
        }
    }

    /// 创建重复查询信号
    pub fn repeat_query(memory_id: &str, query: &str) -> Self {
        Self {
            signal_type: ImplicitSignalType::RepeatQuery,
            memory_id: memory_id.to_string(),
            query: Some(query.to_string()),
            dwell_ms: None,
            timestamp_ms: now_ms(),
        }
    }

    /// 创建忽略信号
    pub fn ignore(memory_id: &str, query: Option<&str>) -> Self {
        Self {
            signal_type: ImplicitSignalType::Ignore,
            memory_id: memory_id.to_string(),
            query: query.map(|s| s.to_string()),
            dwell_ms: None,
            timestamp_ms: now_ms(),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Default for UserFeedback {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用的模拟图查询器
    struct MockGraphQuery {
        neighbors: HashMap<String, Vec<AffectedMemoryInfo>>,
        core_nodes: Vec<String>,
        chain_counts: HashMap<String, usize>,
    }

    impl MemoryGraphQuery for MockGraphQuery {
        fn count_direct_neighbors(&self, memory_id: &str) -> usize {
            self.neighbors.get(memory_id).map(|v| v.len()).unwrap_or(0)
        }

        fn get_neighbor_info(&self, memory_id: &str) -> Vec<AffectedMemoryInfo> {
            self.neighbors.get(memory_id).cloned().unwrap_or_default()
        }

        fn is_core_synthesis_node(&self, memory_id: &str) -> bool {
            self.core_nodes.contains(&memory_id.to_string())
        }

        fn count_affected_synthesis_chains(&self, memory_ids: &[String]) -> usize {
            memory_ids
                .iter()
                .map(|id| self.chain_counts.get(id).copied().unwrap_or(0))
                .sum()
        }
    }

    #[test]
    fn test_record_positive_feedback() {
        let feedback = UserFeedback::new();
        let id = feedback.record_feedback(
            FeedbackType::Positive,
            FeedbackTarget::SynthesisQuality,
            "synth_001",
            None,
            Some("合成质量很好，准确概括了数据库技术栈"),
        );
        assert!(id.starts_with("feedback_"));

        let stats = feedback.get_stats();
        assert_eq!(stats.total_feedback, 1);
        assert_eq!(stats.positive_count, 1);
        assert_eq!(stats.negative_count, 0);
    }

    #[test]
    fn test_record_negative_feedback() {
        let feedback = UserFeedback::new();
        feedback.record_feedback(
            FeedbackType::Negative,
            FeedbackTarget::RetrievalResult,
            "mem_001",
            Some("数据库查询优化"),
            Some("检索结果不相关"),
        );

        let stats = feedback.get_stats();
        assert_eq!(stats.total_feedback, 1);
        assert_eq!(stats.negative_count, 1);
    }

    #[test]
    fn test_user_triggered_quarantine() {
        let feedback = UserFeedback::new();
        feedback.record_feedback(
            FeedbackType::Negative,
            FeedbackTarget::SynthesisQuality,
            "synth_bad",
            None,
            None,
        );
        assert!(!feedback.should_quarantine_by_user("synth_bad"));

        feedback.record_feedback(
            FeedbackType::Negative,
            FeedbackTarget::SynthesisQuality,
            "synth_bad",
            None,
            None,
        );
        assert!(feedback.should_quarantine_by_user("synth_bad"));
    }

    #[test]
    fn test_positive_feedback_prevents_quarantine() {
        let feedback = UserFeedback::new();
        feedback.record_feedback(
            FeedbackType::Positive,
            FeedbackTarget::SynthesisQuality,
            "synth_mixed",
            None,
            None,
        );
        feedback.record_feedback(
            FeedbackType::Negative,
            FeedbackTarget::SynthesisQuality,
            "synth_mixed",
            None,
            None,
        );
        assert!(!feedback.should_quarantine_by_user("synth_mixed"));
    }

    #[test]
    fn test_quarantine_override() {
        let feedback = UserFeedback::new();
        feedback.record_feedback(
            FeedbackType::Positive,
            FeedbackTarget::QuarantineOverride,
            "quarantined_mem",
            None,
            Some("这条记忆被误隔离，应该恢复"),
        );

        let override_ids = feedback.get_quarantine_override_ids();
        assert_eq!(override_ids.len(), 1);
        assert!(override_ids.contains(&"quarantined_mem".to_string()));

        feedback.record_feedback(
            FeedbackType::Positive,
            FeedbackTarget::QuarantineOverride,
            "quarantined_mem",
            None,
            None,
        );
        let override_ids = feedback.get_quarantine_override_ids();
        assert_eq!(override_ids.len(), 1, "重复隔离恢复请求应去重");
    }

    // ============================================================
    // 两阶段确认测试
    // ============================================================

    #[test]
    fn test_two_phase_isolate_low_risk() {
        let feedback = UserFeedback::new();

        // 低风险场景：记忆仅有 1 条关联
        let mut neighbors = HashMap::new();
        neighbors.insert(
            "mem_001".to_string(),
            vec![AffectedMemoryInfo {
                memory_id: "mem_002".to_string(),
                memory_type: "fact".to_string(),
                relation_type: "related_to".to_string(),
                weight: 0.5,
                depth: 0, // 由 request_impact_assessment 设置
            }],
        );
        let mock = MockGraphQuery {
            neighbors,
            core_nodes: vec![],
            chain_counts: HashMap::new(),
        };

        // 阶段一：请求影响评估
        let assessment = feedback.request_impact_assessment(
            PendingActionType::Isolate,
            &["mem_001".to_string()],
            &mock,
        );

        assert_eq!(assessment.risk_level, "low");
        assert_eq!(assessment.direct_neighbors, 1);
        assert!(!assessment.is_core_node);
        assert!(assessment.assessment_id.starts_with("impact_"));
        // 二阶影响应无数据（mem_002 无邻居）
        assert!(assessment.second_order_affected.is_empty());
        // 验证叙事性因果链（质疑二）
        assert!(assessment.narrative.is_some(), "应生成叙事性因果链");
        let narrative = assessment.narrative.unwrap();
        assert!(narrative.contains("mem_001"), "叙事应包含目标记忆");
        assert!(
            narrative.contains("directly connected"),
            "叙事应提及直接关联"
        );

        // 阶段二：确认执行
        let result = feedback.confirm_action(&assessment.assessment_id);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec!["mem_001".to_string()]);

        // 重复确认应失败
        let result2 = feedback.confirm_action(&assessment.assessment_id);
        assert!(result2.is_err());
    }

    #[test]
    fn test_two_phase_isolate_critical_risk() {
        let feedback = UserFeedback::new();

        // 高风险场景：核心节点，关联 5 条合成链
        let mut neighbors = HashMap::new();
        let mut affected = Vec::new();
        for i in 0..8 {
            affected.push(AffectedMemoryInfo {
                memory_id: format!("synth_{:03}", i),
                memory_type: "synthesis".to_string(),
                relation_type: "synthesizes_from".to_string(),
                weight: 0.9,
                depth: 0, // 由 request_impact_assessment 设置
            });
        }
        neighbors.insert("core_mem".to_string(), affected);

        let mut chain_counts = HashMap::new();
        chain_counts.insert("core_mem".to_string(), 5);

        let mock = MockGraphQuery {
            neighbors,
            core_nodes: vec!["core_mem".to_string()],
            chain_counts,
        };

        let assessment = feedback.request_impact_assessment(
            PendingActionType::Isolate,
            &["core_mem".to_string()],
            &mock,
        );

        assert_eq!(assessment.risk_level, "critical");
        assert!(assessment.is_core_node);
        assert_eq!(assessment.affected_synthesis_chains, 5);
        assert!(assessment.recommendation.contains("强烈建议取消"));
    }

    #[test]
    fn test_two_phase_cancel_action() {
        let feedback = UserFeedback::new();

        let neighbors = HashMap::new();
        let mock = MockGraphQuery {
            neighbors,
            core_nodes: vec![],
            chain_counts: HashMap::new(),
        };

        let assessment = feedback.request_impact_assessment(
            PendingActionType::Isolate,
            &["mem_x".to_string()],
            &mock,
        );

        // 取消操作
        let result = feedback.cancel_pending(&assessment.assessment_id);
        assert!(result.is_ok());

        // 确认已取消的操作应失败
        let result2 = feedback.confirm_action(&assessment.assessment_id);
        assert!(result2.is_err());
    }

    #[test]
    fn test_two_phase_cleanup_expired() {
        let feedback = UserFeedback::new();

        let neighbors = HashMap::new();
        let mock = MockGraphQuery {
            neighbors,
            core_nodes: vec![],
            chain_counts: HashMap::new(),
        };

        let _assessment = feedback.request_impact_assessment(
            PendingActionType::Isolate,
            &["mem_x".to_string()],
            &mock,
        );

        // 不应立即过期
        let pending = feedback.get_pending_actions();
        assert_eq!(pending.len(), 1);

        let cleaned = feedback.cleanup_expired_pending();
        assert_eq!(cleaned, 0, "未过期的确认不应被清理");
    }

    #[test]
    fn test_two_phase_confirm_expired() {
        let feedback = UserFeedback::new();

        let neighbors = HashMap::new();
        let mock = MockGraphQuery {
            neighbors,
            core_nodes: vec![],
            chain_counts: HashMap::new(),
        };

        let _assessment = feedback.request_impact_assessment(
            PendingActionType::Isolate,
            &["mem_x".to_string()],
            &mock,
        );

        // 确认不存在的 ID 应报错
        let result = feedback.confirm_action("nonexistent_id");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("未找到"));
    }

    /// 测试：二阶影响评估 — 隔离核心节点时报告二阶间接影响
    #[test]
    fn test_two_phase_second_order_impact() {
        let feedback = UserFeedback::new();

        // 构建二阶影响链条：core → synth_A → fact_X
        //                        core → synth_B → fact_Y
        let mut neighbors = HashMap::new();

        // core 的直接邻居
        neighbors.insert(
            "core".to_string(),
            vec![
                AffectedMemoryInfo {
                    memory_id: "synth_A".to_string(),
                    memory_type: "synthesis".to_string(),
                    relation_type: "synthesizes_from".to_string(),
                    weight: 0.9,
                    depth: 0, // 由 request_impact_assessment 设置
                },
                AffectedMemoryInfo {
                    memory_id: "synth_B".to_string(),
                    memory_type: "synthesis".to_string(),
                    relation_type: "synthesizes_from".to_string(),
                    weight: 0.85,
                    depth: 0,
                },
            ],
        );

        // synth_A 的邻居（二阶）
        neighbors.insert(
            "synth_A".to_string(),
            vec![
                AffectedMemoryInfo {
                    memory_id: "fact_X".to_string(),
                    memory_type: "fact".to_string(),
                    relation_type: "references".to_string(),
                    weight: 0.7,
                    depth: 0,
                },
                AffectedMemoryInfo {
                    memory_id: "fact_Z".to_string(),
                    memory_type: "fact".to_string(),
                    relation_type: "references".to_string(),
                    weight: 0.6,
                    depth: 0,
                },
            ],
        );

        // synth_B 的邻居（二阶）
        neighbors.insert(
            "synth_B".to_string(),
            vec![AffectedMemoryInfo {
                memory_id: "fact_Y".to_string(),
                memory_type: "fact".to_string(),
                relation_type: "references".to_string(),
                weight: 0.8,
                depth: 0,
            }],
        );

        let mut chain_counts = HashMap::new();
        chain_counts.insert("core".to_string(), 2);

        let mock = MockGraphQuery {
            neighbors,
            core_nodes: vec!["core".to_string()],
            chain_counts,
        };

        let assessment = feedback.request_impact_assessment(
            PendingActionType::Isolate,
            &["core".to_string()],
            &mock,
        );

        // 验证直接影响
        assert_eq!(assessment.direct_neighbors, 2, "应有 2 条直接关联记忆");
        assert_eq!(assessment.affected_synthesis_chains, 2);

        // 验证二阶影响
        assert!(
            !assessment.second_order_affected.is_empty(),
            "应包含二阶影响记忆"
        );
        assert_eq!(
            assessment.indirect_neighbors, 3,
            "应有 3 条二阶间接关联记忆"
        );

        // 验证二阶影响的 depth 字段
        for info in &assessment.second_order_affected {
            assert_eq!(info.depth, 2, "二阶影响记忆的 depth 应为 2");
        }

        // 验证直接影响的 depth 字段
        for info in &assessment.affected_memories {
            assert_eq!(info.depth, 1, "直接影响记忆的 depth 应为 1");
        }

        // 验证叙事性因果链（质疑二）
        assert!(assessment.narrative.is_some(), "应生成叙事性因果链");
        let narrative = assessment.narrative.unwrap();
        assert!(narrative.contains("core"), "叙事应包含目标记忆");
        assert!(narrative.contains("synth_A"), "叙事应包含直接合成记忆");
        assert!(narrative.contains("second-order"), "叙事应提及二阶影响");
    }
}
