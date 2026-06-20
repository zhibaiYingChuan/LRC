// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件实现系统健康报告，属于守护层 (Layer 2)。
// ============================================================
//
// 系统健康报告 (SystemHealthReport)
//
// 解决质疑四"可解释性下降"问题：提供统一的系统健康面板，
// 让用户和开发者能一眼看清当前系统的运行模式、编码器状态、
// 调节历史及关键参数。
//
// 核心功能：
//   - 聚合编码器、调节器、合成日志、道同构度等子系统状态
//   - 生成面向用户的系统能力摘要
//   - 提供调试用的详细诊断信息

use super::complexity_budget::ComplexityBudget;
use super::dao_metrics::DaoMetricsSnapshot;
use super::dao_regulator::DaoRegulatorState;
use super::luoshu_encoder::EncoderStatus;
use super::memory_gc::GcStats;
use super::synthesis_journal::SynthesisJournalSnapshot;
use super::user_feedback::FeedbackStats;
use serde::{Deserialize, Serialize};

/// 系统运行模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SystemMode {
    /// 正常运行：ML 编码器 + 洛书合成 + 调节器全部在线
    Healthy,
    /// 部分降级：ML 编码器降级为统计模式，但核心功能正常
    Degraded,
    /// 调节器振荡：检测到调节参数频繁反转，系统正在自我稳定
    Oscillating,
    /// 调节器漂移：检测到参数持续单向漂移，可能存在根因问题
    Drifting,
    /// 调节器冻结：连续无效调节导致调节器暂停，需要外部干预
    Frozen,
    /// 系统过载：记忆数量或合成频率过高，需要关注
    Overloaded,
}

impl SystemMode {
    pub fn as_str(&self) -> &str {
        match self {
            SystemMode::Healthy => "healthy",
            SystemMode::Degraded => "degraded",
            SystemMode::Oscillating => "oscillating",
            SystemMode::Drifting => "drifting",
            SystemMode::Frozen => "frozen",
            SystemMode::Overloaded => "overloaded",
        }
    }

    /// 面向用户的描述
    pub fn user_description(&self) -> &str {
        match self {
            SystemMode::Healthy => "系统运行正常，所有功能在线，语义理解能力完整",
            SystemMode::Degraded => "编码器已降级为统计模式，语义理解能力降低。建议检查网络连接或安装本地 ML 模型",
            SystemMode::Oscillating => "系统参数正在自我调整中，调节器检测到振荡并已启动稳定机制。这是正常现象，无需干预",
            SystemMode::Drifting => "检测到系统参数持续单向漂移，可能存在根因问题（如编码器质量下降、记忆增长速度异常）。建议检查系统日志",
            SystemMode::Frozen => "调节器已冻结——连续多次建议未改善系统状态。建议手动检查并调整参数后解除冻结",
            SystemMode::Overloaded => "记忆库接近容量上限，建议清理过期记忆或调整合成阈值",
        }
    }
}

/// 系统健康报告（可解释性面板）v4.0
///
/// 聚合所有子系统的健康状态，提供统一的诊断视图。
/// v2.0 新增 GC 状态、用户反馈统计、调节器关键参数等运维级信息。
/// v3.0 新增 action_hints 可操作建议，解决"有数据无方法"的运维焦虑。
/// v4.0 新增 complexity_budget 复杂度预算，解决"人类无法驾驭"的终极挑战。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemHealthReport {
    /// 系统运行模式
    pub system_mode: SystemMode,
    /// 系统模式描述（面向用户）
    pub system_mode_description: String,
    /// 编码器状态
    pub encoder: EncoderStatus,
    /// 道同构度指标
    pub dao_metrics: DaoMetricsSnapshot,
    /// 合成日志统计
    pub synthesis_journal: SynthesisJournalSnapshot,
    /// 调节器状态
    pub regulator: DaoRegulatorState,
    /// 记忆库统计
    pub memory_stats: MemoryHealthStats,
    /// 垃圾回收器状态（质疑五：运维可观测性）
    pub gc_stats: GcStats,
    /// 用户反馈统计（质疑五：运维可观测性）
    pub feedback_stats: FeedbackStats,
    /// 可操作建议（质疑一：面向运维的行动指引）
    pub action_hints: Vec<ActionHint>,
    /// 复杂度预算（质疑五·终极：防止系统超出人类可理解范围）
    pub complexity_budget: ComplexityBudget,
    /// 生成时间戳（毫秒）
    pub generated_at_ms: u64,
}

/// 可操作建议（质疑一：降低仪表盘解读门槛）
///
/// 为每个关键指标提供面向运维的行动指引，而非仅展示原始数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionHint {
    /// 提示所属类别：gc / coupling / quality / degradation / feedback
    pub category: String,
    /// 紧急程度：info / warning / action_required
    pub severity: String,
    /// 人类可读的提示信息
    pub message: String,
    /// 建议的具体操作（可为空，表示仅需观察）
    pub suggested_action: String,
}

/// 提示升级追踪器（质疑一：防止"狼来了"效应）
///
/// 追踪每种警告类型连续出现的次数。当同一类警告在连续
/// 多次健康检查中重复出现时，自动提升 severity 级别，
/// 防止用户对重复警告产生麻木。
#[derive(Debug, Clone)]
pub struct HintEscalationTracker {
    /// 指纹 → 连续出现次数
    /// 指纹格式："{category}:{message_key}"
    fingerprints: std::collections::HashMap<String, u32>,
    /// 升级阈值：连续出现超过此次数后升级 severity
    escalation_threshold: u32,
    /// 升级后的 severity 级别
    escalated_severity: String,
}

impl HintEscalationTracker {
    pub fn new() -> Self {
        Self {
            fingerprints: std::collections::HashMap::new(),
            escalation_threshold: 3, // 连续 3 次后升级
            escalated_severity: "action_required".to_string(),
        }
    }

    /// 记录本轮提示的指纹，返回升级后的 Hint 列表
    ///
    /// 工作原理：
    /// 1. 清除上一轮不再出现的指纹（ = 该问题已解决）
    /// 2. 对当前轮的每个 Hint，计算指纹并递增计数
    /// 3. 超过阈值的 Hint 升级 severity
    pub fn process_hints(&mut self, hints: &[ActionHint]) -> Vec<ActionHint> {
        // 收集本轮所有指纹
        let current_fingerprints: std::collections::HashSet<String> =
            hints.iter().map(|h| self.make_fingerprint(h)).collect();

        // 清除不再出现的指纹（问题已解决，重置计数）
        self.fingerprints
            .retain(|k, _| current_fingerprints.contains(k));

        // 处理当前轮的每个 Hint
        hints
            .iter()
            .map(|h| {
                let fp = self.make_fingerprint(h);
                let count = self
                    .fingerprints
                    .entry(fp)
                    .and_modify(|c| *c += 1)
                    .or_insert(1);

                if *count >= self.escalation_threshold && h.severity != "action_required" {
                    // 升级 severity 并追加升级说明
                    let escalated_message =
                        format!("{} [已连续 {} 次出现此警告，级别提升]", h.message, count);
                    ActionHint {
                        category: h.category.clone(),
                        severity: self.escalated_severity.clone(),
                        message: escalated_message,
                        suggested_action: format!(
                            "{} 此问题已持续 {} 次未解决，请优先处理。",
                            h.suggested_action, count
                        ),
                    }
                } else {
                    h.clone()
                }
            })
            .collect()
    }

    /// 生成提示指纹（用于追踪相同类型的警告）
    fn make_fingerprint(&self, hint: &ActionHint) -> String {
        // 使用 category + 可操作建议的前 20 个字符作为指纹
        // 注意：使用 char_indices 安全截断 UTF-8 字符串，避免在多字节字符中间截断
        let action_preview: String = hint
            .suggested_action
            .char_indices()
            .take_while(|(idx, _)| *idx < 20)
            .map(|(_, c)| c)
            .collect();
        let preview = if action_preview.len() < hint.suggested_action.len() {
            action_preview
        } else {
            hint.suggested_action.clone()
        };
        format!("{}:{}", hint.category, preview)
    }
}

impl Default for HintEscalationTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// 记忆库健康统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHealthStats {
    /// 记忆总数
    pub total_memories: usize,
    /// 活跃记忆数（未过期）
    pub active_memories: usize,
    /// 合成记忆数
    pub synthesis_memories: usize,
    /// 过期记忆数
    pub expired_memories: usize,
    /// 低质量合成记忆数（待清理）
    pub low_quality_synthesis: usize,
    /// 八卦分布
    pub bagua_distribution: [usize; 8],
}

/// 生成系统健康报告 v4.0
///
/// 聚合所有子系统的状态，判断系统运行模式，生成统一的诊断视图。
/// v2.0 新增 GC 统计和用户反馈统计，提供运维级可观测性。
/// v3.0 新增 hint_escalation 参数，支持有状态升级的可操作建议。
/// v4.0 新增 complexity_budget 参数，纳入复杂度预算追踪。
/// v0.5.5 新增 llm_configured 参数，LLM 配置后编码器不再视为降级。
///
/// 注意：参数较多（15 个）因为需要聚合所有子系统的状态。
/// 后续重构时可考虑将参数封装为 HealthReportInput 结构体。
#[allow(clippy::too_many_arguments)]
pub fn generate_health_report(
    encoder_status: EncoderStatus,
    dao_snapshot: DaoMetricsSnapshot,
    journal_snapshot: SynthesisJournalSnapshot,
    regulator_state: DaoRegulatorState,
    total_memories: usize,
    active_memories: usize,
    synthesis_memories: usize,
    expired_memories: usize,
    low_quality_count: usize,
    bagua_distribution: [usize; 8],
    gc_stats: GcStats,
    feedback_stats: FeedbackStats,
    complexity_budget: ComplexityBudget,
    hint_escalation: &mut HintEscalationTracker,
    llm_configured: bool,
) -> SystemHealthReport {
    // 判断系统运行模式
    let system_mode = determine_system_mode(
        &encoder_status,
        &dao_snapshot,
        &journal_snapshot,
        &regulator_state,
        total_memories,
        llm_configured,
    );

    let generated_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    SystemHealthReport {
        system_mode: system_mode.clone(),
        system_mode_description: system_mode.user_description().to_string(),
        encoder: encoder_status,
        dao_metrics: dao_snapshot,
        synthesis_journal: journal_snapshot,
        regulator: regulator_state.clone(),
        memory_stats: MemoryHealthStats {
            total_memories,
            active_memories,
            synthesis_memories,
            expired_memories,
            low_quality_synthesis: low_quality_count,
            bagua_distribution,
        },
        action_hints: hint_escalation.process_hints(&generate_action_hints(
            &system_mode,
            &gc_stats,
            &feedback_stats,
            low_quality_count,
            &regulator_state,
        )),
        gc_stats,
        feedback_stats,
        complexity_budget,
        generated_at_ms,
    }
}

/// 生成可操作建议（质疑一：降低仪表盘解读门槛）
///
/// 分析当前系统状态，为每个关键指标生成面向运维的行动指引。
/// 解决"有数据无方法"的运维焦虑——不仅告诉用户"发生了什么"，
/// 还告诉用户"应该做什么"。
fn generate_action_hints(
    mode: &SystemMode,
    gc_stats: &GcStats,
    feedback_stats: &FeedbackStats,
    low_quality_count: usize,
    regulator: &DaoRegulatorState,
) -> Vec<ActionHint> {
    let mut hints = Vec::new();

    // 1. GC 相关建议
    if gc_stats.observing_count > 100 {
        hints.push(ActionHint {
            category: "gc".to_string(),
            severity: "warning".to_string(),
            message: format!(
                "GC 观察队列中有 {} 条候选记忆，建议检查是否需要加速回收周期",
                gc_stats.observing_count
            ),
            suggested_action: "考虑缩短 GC 间隔或手动触发一次 GC 清理".to_string(),
        });
    } else if gc_stats.observing_count > 20 {
        hints.push(ActionHint {
            category: "gc".to_string(),
            severity: "info".to_string(),
            message: format!(
                "GC 观察队列中有 {} 条候选记忆，处于正常范围",
                gc_stats.observing_count
            ),
            suggested_action: "无需操作，系统将在下一个回收周期自动处理".to_string(),
        });
    }

    if gc_stats.total_removed > 0 {
        hints.push(ActionHint {
            category: "gc".to_string(),
            severity: "info".to_string(),
            message: format!(
                "GC 已累计回收 {} 条记忆，释放存储空间",
                gc_stats.total_removed
            ),
            suggested_action: "".to_string(),
        });
    }

    // 2. 低质量记忆相关建议
    if low_quality_count > 50 {
        hints.push(ActionHint {
            category: "quality".to_string(),
            severity: "warning".to_string(),
            message: format!(
                "有 {} 条低质量合成记忆，可能影响检索和合成质量",
                low_quality_count
            ),
            suggested_action:
                "建议检查编码器状态，确认是否为降级模式导致。考虑提高合成阈值以减少低质量产出"
                    .to_string(),
        });
    } else if low_quality_count > 10 {
        hints.push(ActionHint {
            category: "quality".to_string(),
            severity: "info".to_string(),
            message: format!(
                "有 {} 条低质量合成记忆，GC 将在下次周期中自动清理",
                low_quality_count
            ),
            suggested_action: "保持观察，无需手动干预".to_string(),
        });
    }

    // 3. 用户反馈相关建议
    if feedback_stats.negative_count > 0 && feedback_stats.positive_ratio < 0.3 {
        hints.push(ActionHint {
            category: "feedback".to_string(),
            severity: "warning".to_string(),
            message: format!(
                "用户负面反馈占比偏高（正面率 {:.1}），共 {} 条负面反馈",
                feedback_stats.positive_ratio * 100.0,
                feedback_stats.negative_count
            ),
            suggested_action:
                "建议检查最近被负面标记的记忆，排查是否存在系统性的检索或合成质量下降".to_string(),
        });
    }

    if feedback_stats.total_feedback > 0 && feedback_stats.positive_ratio > 0.8 {
        hints.push(ActionHint {
            category: "feedback".to_string(),
            severity: "info".to_string(),
            message: format!(
                "用户反馈正面率 {:.1}，系统输出质量良好",
                feedback_stats.positive_ratio * 100.0
            ),
            suggested_action: "".to_string(),
        });
    }

    // 4. 调节器状态相关建议
    if regulator.is_frozen {
        hints.push(ActionHint {
            category: "degradation".to_string(),
            severity: "action_required".to_string(),
            message: "调节器已冻结——连续多次无效调节，系统停止自动调节。需要人工介入".to_string(),
            suggested_action: "建议手动检查道同构度指标、八卦分布和合成比率，调整参数后解除冻结"
                .to_string(),
        });
    }

    if regulator.is_drifting {
        hints.push(ActionHint {
            category: "degradation".to_string(),
            severity: "warning".to_string(),
            message: format!(
                "检测到调节器持续单向漂移（连续 {} 次同方向调节），可能存在根因问题",
                regulator.consecutive_same_direction
            ),
            suggested_action: "建议检查编码器质量是否下降、记忆增长速度是否异常".to_string(),
        });
    }

    if regulator.is_oscillating {
        hints.push(ActionHint {
            category: "degradation".to_string(),
            severity: "info".to_string(),
            message: "调节器处于振荡状态，系统正在自我稳定。这是正常现象，无需干预".to_string(),
            suggested_action: "".to_string(),
        });
    }

    // 5. 系统模式相关建议
    match mode {
        SystemMode::Degraded => {
            hints.push(ActionHint {
                category: "degradation".to_string(),
                severity: "warning".to_string(),
                message: "系统已降级运行，语义理解能力减弱。当前依赖统计编码器兜底".to_string(),
                suggested_action: "建议检查 ML 模型是否可用、网络连接是否正常。如长期处于降级模式，建议安装本地 ML 模型".to_string(),
            });
        }
        SystemMode::Overloaded => {
            hints.push(ActionHint {
                category: "degradation".to_string(),
                severity: "warning".to_string(),
                message: "系统记忆库接近容量上限或合成频率过高，建议清理或限流".to_string(),
                suggested_action: "建议清理过期记忆、提高合成阈值或降低合成频率".to_string(),
            });
        }
        SystemMode::Healthy => {
            hints.push(ActionHint {
                category: "degradation".to_string(),
                severity: "info".to_string(),
                message: "系统运行正常，所有子系统健康。无需干预".to_string(),
                suggested_action: "".to_string(),
            });
        }
        _ => {}
    }

    hints
}

/// 判断系统运行模式
fn determine_system_mode(
    encoder: &EncoderStatus,
    dao: &DaoMetricsSnapshot,
    journal: &SynthesisJournalSnapshot,
    regulator: &DaoRegulatorState,
    total_memories: usize,
    llm_configured: bool,
) -> SystemMode {
    // 1. 调节器冻结（最高优先级）
    if regulator.is_frozen {
        return SystemMode::Frozen;
    }

    // 2. 调节器漂移
    if regulator.is_drifting {
        return SystemMode::Drifting;
    }

    // 3. 编码器降级检测
    // v0.5.5 P1-1：LLM 配置后替代本地 ML 模型提供语义理解能力
    // 如果 LLM 已配置，编码器不再视为"降级"，系统模式为 Healthy
    if !llm_configured && encoder.mode == "statistical" && encoder.degradation_reason.is_some() {
        return SystemMode::Degraded;
    }

    // 4. 调节器振荡检测
    if regulator.is_oscillating {
        return SystemMode::Oscillating;
    }

    // 5. 系统过载检测
    if total_memories > 100_000 {
        return SystemMode::Overloaded;
    }
    if journal.synthesis_rate_per_minute > 10.0 {
        return SystemMode::Overloaded;
    }

    // 6. 道同构度异常检测
    if dao.dao_isomorphism_score < 0.2 {
        return SystemMode::Degraded;
    }

    SystemMode::Healthy
}

#[cfg(test)]
mod tests {
    use super::super::luoshu_encoder::EncoderStatus;
    use super::super::memory_gc::GcStats;
    use super::super::user_feedback::FeedbackStats;
    use super::*;

    fn make_gc_stats() -> GcStats {
        GcStats {
            total_cycles: 0,
            total_removed: 0,
            observing_count: 0,
            last_gc_ms: 0,
            last_removed_count: 0,
            total_freed: 0,
        }
    }

    fn make_feedback_stats() -> FeedbackStats {
        FeedbackStats {
            total_feedback: 0,
            positive_count: 0,
            negative_count: 0,
            synthesis_feedback_count: 0,
            quarantine_overrides: 0,
            positive_ratio: 0.5,
            implicit_feedback_enabled: false,
            consent_granted: None,
        }
    }

    fn make_escalation() -> HintEscalationTracker {
        HintEscalationTracker::new()
    }

    fn make_complexity_budget() -> ComplexityBudget {
        let mut budget = ComplexityBudget::new();
        budget.update(20, 200, 40, 4);
        budget
    }

    fn make_healthy_state() -> (
        EncoderStatus,
        DaoMetricsSnapshot,
        SynthesisJournalSnapshot,
        DaoRegulatorState,
    ) {
        let encoder = EncoderStatus {
            mode: "ml".to_string(),
            model_name: Some("test-model".to_string()),
            hidden_size: Some(384),
            degradation_reason: None,
            total_encodings: 100,
            last_encoding_ms: 0,
            capability_description: "ML 语义模式".to_string(),
            quality_score: 1.0,
        };

        let dao = DaoMetricsSnapshot {
            encodings_total: 100,
            compositions_total: 20,
            recalls_total: 200,
            corrections_total: 5,
            active_memories: 100,
            crystallized_memories: 20,
            archived_memories: 5,
            dao_isomorphism_score: 0.85,
            bagua_entropy: 2.5,
            synthesis_ratio: 0.2,
            last_collected_ms: 0,
        };

        let journal = SynthesisJournalSnapshot {
            total_synthesis: 20,
            low_quality_count: 0,
            synthesis_rate_per_minute: 0.5,
            recent_events: vec![],
            success_rate: 1.0,
        };

        let regulator = DaoRegulatorState {
            last_regulation_ms: 0,
            is_oscillating: false,
            oscillation_window: 5,
            step_multiplier: 1.0,
            auto_regulate: true,
            regulation_interval_ms: 300_000,
            is_drifting: false,
            consecutive_same_direction: 0,
            drift_threshold: 8,
            is_frozen: false,
            consecutive_ineffective: 0,
            freeze_threshold: 10,
            coupling_score: 0.0,
        };

        (encoder, dao, journal, regulator)
    }

    #[test]
    fn test_healthy_mode() {
        let (encoder, dao, journal, regulator) = make_healthy_state();
        let report = generate_health_report(
            encoder,
            dao,
            journal,
            regulator,
            100,
            95,
            20,
            5,
            0,
            [12, 13, 12, 13, 12, 13, 12, 13],
            make_gc_stats(),
            make_feedback_stats(),
            make_complexity_budget(),
            &mut make_escalation(),
            false,
        );

        assert_eq!(report.system_mode, SystemMode::Healthy);
        assert_eq!(report.memory_stats.total_memories, 100);
        assert_eq!(report.memory_stats.low_quality_synthesis, 0);
        // 验证 GC 和反馈统计已包含
        assert_eq!(report.gc_stats.total_cycles, 0);
        assert_eq!(report.feedback_stats.total_feedback, 0);
        // 验证 action_hints 已生成（质疑一）
        assert!(
            !report.action_hints.is_empty(),
            "健康模式下应生成 action_hints"
        );
        let healthy_hint = report
            .action_hints
            .iter()
            .find(|h| h.category == "degradation" && h.severity == "info")
            .expect("应包含健康模式提示");
        assert!(healthy_hint.message.contains("运行正常"));
    }

    #[test]
    fn test_degraded_mode_on_encoder_fallback() {
        let (mut encoder, dao, journal, regulator) = make_healthy_state();
        encoder.mode = "statistical".to_string();
        encoder.degradation_reason = Some("ML 模型加载失败".to_string());

        let report = generate_health_report(
            encoder,
            dao,
            journal,
            regulator,
            100,
            95,
            20,
            5,
            0,
            [12; 8],
            make_gc_stats(),
            make_feedback_stats(),
            make_complexity_budget(),
            &mut make_escalation(),
            false,
        );

        assert_eq!(report.system_mode, SystemMode::Degraded);
        assert!(report.system_mode_description.contains("降级"));
    }

    #[test]
    fn test_oscillating_mode() {
        let (encoder, dao, journal, mut regulator) = make_healthy_state();
        regulator.is_oscillating = true;

        let report = generate_health_report(
            encoder,
            dao,
            journal,
            regulator,
            100,
            95,
            20,
            5,
            0,
            [12; 8],
            make_gc_stats(),
            make_feedback_stats(),
            make_complexity_budget(),
            &mut make_escalation(),
            false,
        );

        assert_eq!(report.system_mode, SystemMode::Oscillating);
    }

    #[test]
    fn test_overloaded_mode() {
        let (encoder, dao, journal, regulator) = make_healthy_state();

        let report = generate_health_report(
            encoder,
            dao,
            journal,
            regulator,
            200_000,
            150_000,
            50_000,
            10_000,
            100,
            [25000; 8],
            make_gc_stats(),
            make_feedback_stats(),
            make_complexity_budget(),
            &mut make_escalation(),
            false,
        );

        assert_eq!(report.system_mode, SystemMode::Overloaded);
    }

    #[test]
    fn test_report_serialization() {
        let (encoder, dao, journal, regulator) = make_healthy_state();
        let report = generate_health_report(
            encoder,
            dao,
            journal,
            regulator,
            100,
            95,
            20,
            5,
            0,
            [12, 13, 12, 13, 12, 13, 12, 13],
            make_gc_stats(),
            make_feedback_stats(),
            make_complexity_budget(),
            &mut make_escalation(),
            false,
        );

        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("healthy"));
        assert!(json.contains("ml"));
        assert!(json.contains("total_memories"));
        // 验证新增字段存在
        assert!(json.contains("gc_stats"));
        assert!(json.contains("feedback_stats"));
        assert!(json.contains("action_hints"));
    }

    /// 测试：提示升级追踪器 — 连续出现 3 次后升级 severity
    #[test]
    fn test_hint_escalation_tracker() {
        let mut tracker = HintEscalationTracker::new();

        // 第 1 次：正常 warning
        let hints1 = vec![ActionHint {
            category: "gc".to_string(),
            severity: "warning".to_string(),
            message: "GC 队列偏高".to_string(),
            suggested_action: "考虑缩短 GC 间隔".to_string(),
        }];
        let result1 = tracker.process_hints(&hints1);
        assert_eq!(result1[0].severity, "warning");

        // 第 2 次：仍然是 warning
        let result2 = tracker.process_hints(&hints1);
        assert_eq!(result2[0].severity, "warning");

        // 第 3 次：触发升级
        let result3 = tracker.process_hints(&hints1);
        assert_eq!(result3[0].severity, "action_required");
        assert!(result3[0].message.contains("已连续 3 次出现"));

        // 第 4 次：继续是 action_required
        let result4 = tracker.process_hints(&hints1);
        assert_eq!(result4[0].severity, "action_required");
        assert!(result4[0].message.contains("已连续 4 次出现"));
    }

    /// 测试：提示升级追踪器 — 警告消失后重置计数
    #[test]
    fn test_hint_escalation_reset() {
        let mut tracker = HintEscalationTracker::new();

        let gc_hint = vec![ActionHint {
            category: "gc".to_string(),
            severity: "warning".to_string(),
            message: "GC 队列偏高".to_string(),
            suggested_action: "考虑缩短 GC 间隔".to_string(),
        }];

        // 连续 3 次触发升级
        tracker.process_hints(&gc_hint);
        tracker.process_hints(&gc_hint);
        let result3 = tracker.process_hints(&gc_hint);
        assert_eq!(result3[0].severity, "action_required");

        // 警告消失（本轮无 gc 提示）
        let no_gc_hint: Vec<ActionHint> = vec![];
        let result4 = tracker.process_hints(&no_gc_hint);
        assert!(result4.is_empty());

        // 重新出现：应重置为 warning
        let result5 = tracker.process_hints(&gc_hint);
        assert_eq!(result5[0].severity, "warning", "警告消失后应重置计数");
    }
}
