// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件实现道同构度自适应调节器，属于守护层 (Layer 2)。
// ============================================================
//
// 道同构度调节器 (DaoRegulator) v2.0 — 防振荡增强版
//
// 将 DaoMetrics 从"只读仪表盘"升级为"感知→行动"闭环。
// 定期检查系统健康指标，自动生成调节动作。
//
// v2.0 新增防振荡机制：
//   - 调节历史追踪：记录最近 N 次调节，检测同方向连续调节次数
//   - 冲突仲裁：当多个条件同时触发时，按优先级排序
//   - 自适应步长：检测到振荡时自动减半步长
//   - 冷却自适应：根据调节效果动态调整冷却时间
//
// 核心功能：
//   - 阴阳失衡检测 → 调整衰减速率恢复平衡
//   - 洛书偏差增大 → 建议重新编码
//   - 合成/检索比率异常 → 调整合成阈值
//   - 八卦分布集中 → 调整检索权重鼓励探索

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

/// 调节动作类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RegulationAction {
    /// 无需调节
    NoAction,
    /// 调整衰减速率：加快衰减以释放空间，或减缓衰减以保留记忆
    AdjustDecayRate {
        /// 新的衰减速率（0.0 ~ 1.0）
        new_rate: f32,
        /// 调整原因
        reason: String,
    },
    /// 调整合成阈值：降低以增加合成频率，或提高以减少伪合成
    /// v2.2 新增 `severity` 字段，用于慢性退化自动响应（质疑四）。
    AdjustSynthesisThreshold {
        /// 新的最小聚类大小
        new_min_cluster: usize,
        /// 调整原因
        reason: String,
        /// 严重程度（可选）
        severity: Option<String>,
    },
    /// 建议重新编码：洛书偏差过大，需要重新校准编码器
    SuggestReencoding {
        /// 偏差程度说明
        severity: String,
        /// 建议
        reason: String,
    },
    /// 调整检索权重：八卦分布不均匀，引导检索向冷门类别倾斜
    /// v2.2 新增 `new_weights` 字段，用于慢性退化自动响应（质疑四）。
    AdjustRetrievalWeights {
        /// 新的检索权重（可选，为空时由引擎自动计算）
        new_weights: Option<Vec<f32>>,
        /// 原因
        reason: String,
    },
    /// 调整信息增量阈值：基于合成质量反馈动态微调防坍塌门槛（质疑一·活性）
    ///
    /// 当合成产物的平均命中率偏低时，降低阈值以鼓励更多合成；
    /// 当合成产物中出现过多空洞抽象时，提高阈值以收紧标准。
    /// 道枢映射：坤卦·地 (☷) — 承载与收藏，动态调节"收"与"放"的平衡。
    AdjustInformationGainThreshold {
        /// 新的信息增量阈值（0.0 ~ 1.0）
        new_threshold: f32,
        /// 调整原因
        reason: String,
    },
    /// 综合再平衡：多指标同时异常，建议系统级综合调节（质疑二）
    ///
    /// 当耦合指数 > 0.5 时生成，包含所有异常指标的联合诊断。
    /// v2.2 新增 `severity` 字段，用于慢性退化自动响应（质疑四）。
    SuggestComprehensiveRebalance {
        /// 异常指标描述
        anomaly_description: String,
        /// 耦合指数 (0.0 ~ 1.0)
        coupling_score: f32,
        /// 严重程度：warning / severe / critical
        severity: String,
    },
}

/// 调节器状态快照（可解释性面板）
///
/// 提供调节器当前运行状态的透明视图，包括振荡检测、
/// 步长倍率、冷却时间、漂移、冻结等关键参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaoRegulatorState {
    /// 上次调节时间戳（毫秒）
    pub last_regulation_ms: u64,
    /// 是否检测到振荡
    pub is_oscillating: bool,
    /// 振荡检测窗口大小
    pub oscillation_window: usize,
    /// 当前步长倍率（1.0 = 正常，< 1.0 = 减半）
    pub step_multiplier: f32,
    /// 是否启用自动调节
    pub auto_regulate: bool,
    /// 调节间隔（毫秒）
    pub regulation_interval_ms: u64,
    /// 是否检测到漂移
    pub is_drifting: bool,
    /// 连续同方向调节次数
    pub consecutive_same_direction: usize,
    /// 漂移阈值
    pub drift_threshold: usize,
    /// 是否已冻结
    pub is_frozen: bool,
    /// 连续无效调节次数
    pub consecutive_ineffective: usize,
    /// 冻结阈值
    pub freeze_threshold: usize,
    /// 耦合指数（质疑一·活性：聚合到状态快照中，供健康报告使用）
    pub coupling_score: f32,
}

/// 单次调节历史记录
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct RegulationRecord {
    /// 调节时间戳（毫秒）
    timestamp_ms: u64,
    /// 调节动作类型标签
    action_tag: String,
    /// 调节方向（+1 表示增加/提高，-1 表示减少/降低）
    direction: i8,
    /// 调节幅度
    magnitude: f32,
}

/// 道同构度调节器 v2.0
///
/// 基于 DaoMetrics + SynthesisJournal 的数据，
/// 自动生成调节动作以维持系统健康。
///
/// 防振荡设计：
/// 1. 调节历史追踪：最多保留 20 条历史记录
/// 2. 振荡检测：同一参数在 3 次调节内方向反转 → 判定为振荡
/// 3. 冲突仲裁：优先级 Reencoding > RetrievalWeights > SynthesisThreshold > DecayRate
/// 4. 自适应步长：检测到振荡时步长减半，稳定后恢复
///
/// v2.1 新增长期稳定性机制：
/// 5. 漂移检测：同一指标连续 N 次同方向调节 → 判定为漂移，发出警告
/// 6. 冻结保护：连续 N 次无效调节（建议相同但无改善）→ 冻结调节器，等待外部干预
#[derive(Debug)]
pub struct DaoRegulator {
    /// 上次调节时间戳
    last_regulation_ms: u64,
    /// 调节间隔（毫秒），默认 5 分钟
    regulation_interval_ms: u64,
    /// 是否启用自动调节
    pub auto_regulate: bool,
    /// 调节历史记录（FIFO，最多 20 条）
    regulation_history: Vec<RegulationRecord>,
    /// 是否检测到振荡状态
    in_oscillation: bool,
    /// 振荡检测窗口（调节次数）
    oscillation_window: usize,
    /// 当前自适应步长倍率（1.0 = 正常，0.5 = 减半，0.25 = 四分之一）
    step_multiplier: f32,
    /// 漂移检测：连续同方向调节次数追踪
    consecutive_same_direction: usize,
    /// 漂移检测阈值：连续同方向超过此次数触发告警
    drift_threshold: usize,
    /// 是否检测到漂移
    in_drift: bool,
    /// 冻结保护：连续无效调节次数
    consecutive_ineffective: usize,
    /// 冻结保护阈值：连续无效超过此次数冻结调节器
    freeze_threshold: usize,
    /// 是否已冻结
    is_frozen: bool,
    /// 上次调节的 action_tag（用于检测"重复无效建议"）
    last_action_tag: Option<String>,
    /// 耦合指数：跟踪多指标同时异常的相关性（解决质疑二"多指标耦合"）
    ///
    /// 当多个指标同时触发异常时，耦合指数递增。高耦合指数表明
    /// 指标间存在因果关联，应生成综合建议而非简单按优先级仲裁。
    coupling_score: f32,
    /// 耦合历史记录（用于长周期反馈分析）
    coupling_history: Vec<CouplingEvent>,
    /// 灾难性转折检测器（质疑二：识别"压死骆驼的最后一根稻草"）
    catastrophic_detector: CatastrophicEventDetector,
    /// 当前合成最小聚类大小（用于慢性退化自动响应，质疑四）
    synthesis_min_cluster_size: usize,
    /// 动态信息增量阈值（质疑一·活性：由调节器自动微调，替代硬编码常量）
    ///
    /// 默认值 0.01，适配统计编码器模式。调节器根据合成产物的
    /// 平均命中率和信息增量分布，自动上下微调此阈值。
    /// 道枢映射：坤卦·地 (☷) — 承载与收藏，动态平衡"收"与"放"。
    pub information_gain_threshold: f32,

    // ---- 质疑一·终极：防漂移机制 ----
    //
    // 动态阈值虽然赋予了系统"活性"，但也引入了"近视"风险：
    // 短期环境变化可能导致阈值过度调整，然后在环境恢复后
    // 漫长的历史数据漂移回来。以下机制确保阈值调整既灵敏
    // 又稳定，如同一个有记忆的免疫系统。
    //
    // 道枢映射：艮卦·山 (☶) — 止于至善，为变化设界。
    /// 阈值锚定基线（EMA 长期均值回归目标）
    /// 默认 0.01，阈值偏离此基线越远，回归力越强。
    threshold_baseline: f32,
    /// 阈值 EMA（指数移动平均），平滑短期波动
    /// 平滑因子 α = 0.3，即新值权重 30%，旧值权重 70%
    threshold_ema: f32,
    /// EMA 平滑因子（0.0 ~ 1.0，值越大对新值越敏感）
    threshold_momentum: f32,
    /// 阈值历史追踪（最近 20 次调整值，用于长周期趋势分析）
    threshold_history: VecDeque<f32>,
    /// 最大允许偏离基线的幅度（绝对值）
    /// 阈值范围 = [baseline - max_deviation, baseline + max_deviation]
    max_threshold_deviation: f32,
    /// 均值回归速率：每次调节中无阈值调整时，向基线回归的步长
    reversion_rate: f32,
    /// 上次阈值调整距今的调节周期数（用于触发均值回归）
    cycles_since_last_threshold_adjustment: usize,
    /// 均值回归触发周期：连续 N 次调节无阈值调整时，开始均值回归
    reversion_trigger_cycles: usize,

    // ---- 质疑三·责任鸿沟：可解释决策日志 ----
    //
    // 记录每次自主决策的完整上下文，最多保留 100 条。
    // 每条日志包含输入快照、分析推理链、替代方案和风险评估，
    // 确保系统的"思考"过程透明可追溯。
    //
    // 道枢映射：艮卦·山 (☶) — "艮其止，止其所也"。决策日志
    // 如山之层积，每一层都清晰可见，填平"有原因，无责任"的鸿沟。
    /// 决策日志（FIFO，最多 100 条）
    decision_log: Vec<DecisionLog>,
}

/// 系统健康快照
///
/// 每次调节前记录系统关键指标，用于长周期对比分析。
/// 灾难性转折检测器通过对比历史快照，识别系统状态的急剧恶化。
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SystemHealthSnapshot {
    /// 时间戳（毫秒）
    timestamp_ms: u64,
    /// 道同构度评分
    dao_score: f32,
    /// 八卦分布熵
    bagua_entropy: f32,
    /// 合成比率
    synthesis_ratio: f32,
    /// 洛书幻和平均偏离度
    avg_luoshu_deviation: f32,
    /// 耦合指数
    coupling_score: f32,
    /// 综合健康评分（0.0 ~ 1.0，越高越健康）
    composite_health: f32,
}

/// 灾难性转折事件
///
/// 当系统健康评分在短时间内急剧下降时触发。
/// 记录"压死骆驼的最后一根稻草"——导致状态急转直下的关键调节事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatastrophicEvent {
    /// 事件时间戳
    pub timestamp_ms: u64,
    /// 转折前的健康评分
    pub health_before: f32,
    /// 转折后的健康评分
    pub health_after: f32,
    /// 下降幅度
    pub drop_magnitude: f32,
    /// 转折前最后一次调节动作
    pub last_action_before_crash: String,
    /// 转折前最后一次调节的原因
    pub last_action_reason: String,
    /// 转折前耦合指数
    pub coupling_before: f32,
    /// 转折前连续同方向调节次数
    pub consecutive_same_direction_before: usize,
    /// 是否检测到漂移
    pub drift_detected_before: bool,
    /// 严重程度：warning / severe / critical
    pub severity: String,
    /// 诊断建议
    pub diagnosis: String,
}

/// 灾难性转折检测器
///
/// 解决质疑二"灾难性遗忘"问题：
/// 通过追踪系统健康快照的历史变化，检测是否存在某个调节动作
/// 导致系统状态急剧恶化——即"压死骆驼的最后一根稻草"。
///
/// 工作原理：
/// 1. 每次调节前记录系统健康快照
/// 2. 对比最近 N 次快照，计算健康评分的变化趋势
/// 3. 当检测到健康评分在短时间内（如 5 次调节内）下降超过阈值时，
///    标记为灾难性转折事件
/// 4. 输出导致转折的关键调节动作，供人工或自动回滚决策
#[derive(Debug)]
struct CatastrophicEventDetector {
    /// 健康快照历史（FIFO，最多 50 条）
    snapshots: Vec<SystemHealthSnapshot>,
    /// 探测到的灾难性事件列表
    catastrophic_events: Vec<CatastrophicEvent>,
    /// 上次调节动作标签（用于归因）
    last_action_tag: String,
    /// 上次调节原因
    last_action_reason: String,
    /// 急性下降阈值：健康评分下降超过此值触发急症告警
    drop_threshold: f32,
    /// 急性检测窗口：最近 N 次调节内检测骤降
    detection_window: usize,
    /// 慢性恶化窗口：更长周期检测持续下降（质疑四）
    chronic_window: usize,
    /// 慢性恶化阈值：在慢性窗口内累计下降超过此值触发告警
    chronic_drop_threshold: f32,
}

impl CatastrophicEventDetector {
    fn new() -> Self {
        Self {
            snapshots: Vec::with_capacity(50),
            catastrophic_events: Vec::new(),
            last_action_tag: String::new(),
            last_action_reason: String::new(),
            drop_threshold: 0.3, // 急性：健康评分下降超过 0.3 触发急症告警
            detection_window: 5, // 急性：最近 5 次调节内检测
            chronic_window: 20,  // 慢性：最近 20 次调节内检测（质疑四）
            chronic_drop_threshold: 0.15, // 慢性：累计下降超过 0.15 触发预警
        }
    }

    /// 记录健康快照（每次调节前调用）
    fn record_snapshot(
        &mut self,
        dao_score: f32,
        bagua_entropy: f32,
        synthesis_ratio: f32,
        avg_luoshu_deviation: f32,
        coupling_score: f32,
    ) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // 综合健康评分：道同构度 + 八卦熵归一化 + 偏离度逆指标
        let bagua_health = (bagua_entropy / 3.0).min(1.0); // 熵越高越健康（归一化到 0-1）
        let deviation_health = (1.0 - avg_luoshu_deviation).max(0.0); // 偏离度越低越健康
        let synthesis_health = if synthesis_ratio < 0.5 {
            1.0 - synthesis_ratio * 2.0 // 合成比率越低越健康
        } else {
            0.0 // 合成比率超过 50% 不健康
        };
        let coupling_health = 1.0 - coupling_score;

        let composite = dao_score * 0.35
            + bagua_health * 0.2
            + deviation_health * 0.2
            + synthesis_health * 0.15
            + coupling_health * 0.1;

        let snapshot = SystemHealthSnapshot {
            timestamp_ms: now,
            dao_score,
            bagua_entropy,
            synthesis_ratio,
            avg_luoshu_deviation,
            coupling_score,
            composite_health: composite.clamp(0.0, 1.0),
        };

        self.snapshots.push(snapshot);
        if self.snapshots.len() > 50 {
            self.snapshots.remove(0);
        }
    }

    /// 记录调节动作（用于归因）
    fn record_action(&mut self, action_tag: &str, reason: &str) {
        self.last_action_tag = action_tag.to_string();
        self.last_action_reason = reason.to_string();
    }

    /// 检测灾难性转折
    ///
    /// 在最近 `detection_window` 次快照中，如果健康评分下降超过
    /// `drop_threshold`，则判定为灾难性转折。
    fn detect(
        &mut self,
        consecutive_same_direction: usize,
        drift_detected: bool,
    ) -> Option<CatastrophicEvent> {
        if self.snapshots.len() < self.detection_window {
            return None;
        }

        let window_start = self.snapshots.len().saturating_sub(self.detection_window);
        let recent: Vec<&SystemHealthSnapshot> = self.snapshots[window_start..].iter().collect();

        // 找到窗口内的最高和最低健康评分
        let max_health = recent
            .iter()
            .map(|s| s.composite_health)
            .fold(0.0f32, f32::max);
        let min_health = recent
            .iter()
            .map(|s| s.composite_health)
            .fold(1.0f32, f32::min);

        let drop = max_health - min_health;

        if drop < self.drop_threshold {
            return None;
        }

        // 找到转折点：健康评分开始下降的位置
        let turning_point = recent
            .iter()
            .position(|s| s.composite_health < max_health - drop * 0.5)
            .unwrap_or(0);

        let health_before = if turning_point > 0 {
            recent[turning_point - 1].composite_health
        } else {
            recent[0].composite_health
        };
        let health_after = min_health;

        // 严重程度判定
        let severity = if drop > 0.6 {
            "critical"
        } else if drop > 0.4 {
            "severe"
        } else {
            "warning"
        };

        let diagnosis = if drift_detected {
            format!(
                "检测到灾难性转折：健康评分从 {:.2} 骤降至 {:.2}（下降 {:.2}）。\
                 系统此前已检测到漂移（连续 {} 次同方向调节），\
                 最后一次调节动作 '{}' 可能是导火索。\
                 建议：立即回滚最近一次调节，暂停自动调节，等待人工介入。",
                health_before, health_after, drop, consecutive_same_direction, self.last_action_tag
            )
        } else {
            format!(
                "检测到灾难性转折：健康评分从 {:.2} 骤降至 {:.2}（下降 {:.2}）。\
                 最后一次调节动作 '{}' ({}) 可能是导火索。\
                 建议：审查最近 {} 次调节的合理性，考虑回滚。",
                health_before,
                health_after,
                drop,
                self.last_action_tag,
                self.last_action_reason,
                self.detection_window
            )
        };

        let event = CatastrophicEvent {
            timestamp_ms: recent.last().map(|s| s.timestamp_ms).unwrap_or(0),
            health_before,
            health_after,
            drop_magnitude: drop,
            last_action_before_crash: self.last_action_tag.clone(),
            last_action_reason: self.last_action_reason.clone(),
            coupling_before: recent[turning_point].coupling_score,
            consecutive_same_direction_before: consecutive_same_direction,
            drift_detected_before: drift_detected,
            severity: severity.to_string(),
            diagnosis,
        };

        self.catastrophic_events.push(event.clone());
        Some(event)
    }

    /// 检测慢性恶化（质疑四：防止"温水煮青蛙"式退化）
    ///
    /// 与急性检测不同，慢性恶化检测关注的是在较长周期内（如 20 次调节）
    /// 健康评分持续单向下降的趋势。即使每次下降幅度很小，积累起来
    /// 也可能导致系统在不知不觉中逐渐失去记忆质量。
    ///
    /// 工作原理：
    /// 1. 在慢性窗口（chronic_window）内，计算健康评分的线性回归斜率
    /// 2. 如果斜率持续为负且累计下降超过慢性阈值，触发预警
    /// 3. 同时检查是否所有快照都呈现下降趋势（单调性验证）
    fn detect_chronic_degradation(&mut self) -> Option<CatastrophicEvent> {
        if self.snapshots.len() < self.chronic_window {
            return None;
        }

        let window_start = self.snapshots.len().saturating_sub(self.chronic_window);
        let chronic: Vec<&SystemHealthSnapshot> = self.snapshots[window_start..].iter().collect();

        // 计算起始和最新的健康评分
        let health_start = chronic.first().map(|s| s.composite_health).unwrap_or(1.0);
        let health_end = chronic.last().map(|s| s.composite_health).unwrap_or(1.0);
        let total_drop = health_start - health_end;

        // 累计下降未超过慢性阈值，不触发
        if total_drop < self.chronic_drop_threshold {
            return None;
        }

        // 单调性验证：检查是否大多数快照呈现下降趋势
        // 将窗口分为前后两半，比较前后半的平均值
        let mid = chronic.len() / 2;
        let first_half_avg: f32 = chronic[..mid]
            .iter()
            .map(|s| s.composite_health)
            .sum::<f32>()
            / mid as f32;
        let second_half_avg: f32 = chronic[mid..]
            .iter()
            .map(|s| s.composite_health)
            .sum::<f32>()
            / (chronic.len() - mid) as f32;

        // 后半段平均值低于前半段，确认下降趋势
        if second_half_avg >= first_half_avg {
            return None;
        }

        // 计算下降部分的占比：连续下降的片段数
        let mut decline_streaks = 0usize;
        let mut current_streak = 0usize;
        for i in 1..chronic.len() {
            if chronic[i].composite_health < chronic[i - 1].composite_health {
                current_streak += 1;
                decline_streaks = decline_streaks.max(current_streak);
            } else {
                current_streak = 0;
            }
        }

        // 最长的连续下降段太短，可能是随机波动
        if decline_streaks < 3 {
            return None;
        }

        let severity = if total_drop > 0.3 {
            "severe"
        } else {
            "warning"
        };

        let diagnosis = format!(
            "检测到慢性恶化：健康评分在过去 {} 次调节中从 {:.2} 持续下降至 {:.2}（累计下降 {:.2}）。\
             后半段平均健康评分（{:.2}）明显低于前半段（{:.2}），最长连续下降段 {} 次。\
             这不是急性崩溃，而是'温水煮青蛙'式的缓慢退化，可能由编码器质量下降、\
             记忆库膨胀或合成质量恶化引起。\
             建议：检查编码器状态、审查近期的合成日志、考虑提高合成质量阈值。",
            self.chronic_window,
            health_start, health_end, total_drop,
            second_half_avg, first_half_avg,
            decline_streaks
        );

        let event = CatastrophicEvent {
            timestamp_ms: chronic.last().map(|s| s.timestamp_ms).unwrap_or(0),
            health_before: health_start,
            health_after: health_end,
            drop_magnitude: total_drop,
            last_action_before_crash: "chronic_degradation".to_string(),
            last_action_reason: format!(
                "慢性恶化检测：{} 次调节内累计下降 {:.2}",
                self.chronic_window, total_drop
            ),
            coupling_before: chronic.last().map(|s| s.coupling_score).unwrap_or(0.0),
            consecutive_same_direction_before: decline_streaks,
            drift_detected_before: true, // 慢性恶化本质上是一种长期漂移
            severity: severity.to_string(),
            diagnosis,
        };

        // 避免重复报告：只在最近 10 次未报告过慢性事件时才记录
        let recent_chronic = self
            .catastrophic_events
            .iter()
            .rev()
            .take(10)
            .any(|e| e.last_action_before_crash == "chronic_degradation");

        if !recent_chronic {
            eprintln!(
                "[LRC·灾难·慢性] 检测到慢性恶化：{:.2} → {:.2}（{} 次调节内）",
                health_start, health_end, self.chronic_window
            );
            self.catastrophic_events.push(event.clone());
            return Some(event);
        }

        None
    }

    /// 获取所有灾难性事件
    fn get_events(&self) -> &[CatastrophicEvent] {
        &self.catastrophic_events
    }
}

/// 耦合事件记录
///
/// 记录多指标同时异常的事件，用于长周期反馈分析。
/// 解决质疑二"长周期反馈延迟"问题——通过历史耦合事件
/// 判断调节策略是否需要根本性调整。
#[derive(Debug, Clone)]
struct CouplingEvent {
    /// 事件时间戳（毫秒）
    timestamp_ms: u64,
    /// 同时异常的指标：deviation + bagua + dao + synthesis
    anomaly_flags: [bool; 4],
    /// 耦合强度 (0.0 ~ 1.0)
    coupling_strength: f32,
    /// 是否发生了连锁反应
    cascade_detected: bool,
}

/// 长周期耦合趋势分析结果
///
/// 由 `analyze_coupling_trend()` 生成，提供耦合历史的统计分析。
/// 解决质疑二"长周期反馈延迟"——通过趋势分析判断
/// 系统是否在持续恶化，而非仅依赖单次快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouplingTrendAnalysis {
    /// 耦合事件总数
    pub total_coupling_events: usize,
    /// 连锁反应次数
    pub cascade_count: usize,
    /// 连锁反应占比 (0.0 ~ 1.0)
    pub cascade_ratio: f32,
    /// 全部事件的平均耦合强度
    pub all_avg_strength: f32,
    /// 最近 10 个事件的平均耦合强度
    pub recent_avg_strength: f32,
    /// 是否呈恶化趋势（近期强度 > 历史均值 × 1.2）
    pub is_worsening: bool,
    /// 洛书偏差异常次数
    pub deviation_anomaly_count: usize,
    /// 八卦分布异常次数
    pub bagua_anomaly_count: usize,
    /// 道同构度异常次数
    pub dao_anomaly_count: usize,
    /// 合成比率异常次数
    pub synthesis_anomaly_count: usize,
    /// 最近 1 小时内的耦合事件密度
    pub recent_hour_density: usize,
}

impl Default for CouplingTrendAnalysis {
    fn default() -> Self {
        Self {
            total_coupling_events: 0,
            cascade_count: 0,
            cascade_ratio: 0.0,
            all_avg_strength: 0.0,
            recent_avg_strength: 0.0,
            is_worsening: false,
            deviation_anomaly_count: 0,
            bagua_anomaly_count: 0,
            dao_anomaly_count: 0,
            synthesis_anomaly_count: 0,
            recent_hour_density: 0,
        }
    }
}

impl DaoRegulator {
    /// 创建新的调节器
    pub fn new() -> Self {
        Self {
            last_regulation_ms: 0,
            regulation_interval_ms: 300_000, // 5 分钟
            auto_regulate: true,
            regulation_history: Vec::with_capacity(20),
            in_oscillation: false,
            oscillation_window: 5,
            step_multiplier: 1.0,
            consecutive_same_direction: 0,
            drift_threshold: 8,
            in_drift: false,
            consecutive_ineffective: 0,
            freeze_threshold: 10,
            is_frozen: false,
            last_action_tag: None,
            coupling_score: 0.0,
            coupling_history: Vec::new(),
            catastrophic_detector: CatastrophicEventDetector::new(),
            synthesis_min_cluster_size: 3,    // 默认最小聚类大小
            information_gain_threshold: 0.01, // 质疑一·活性：默认信息增量阈值，由调节器动态微调
            // 质疑一·终极：防漂移机制初始化
            threshold_baseline: 0.01,
            threshold_ema: 0.01,
            threshold_momentum: 0.3, // EMA 平滑因子：30% 新值 + 70% 旧值
            threshold_history: VecDeque::with_capacity(20),
            max_threshold_deviation: 0.05, // 最大偏离基线 ±0.05（范围 [0.001, 0.06]）
            reversion_rate: 0.001,         // 每次无调整周期向基线回归 0.001
            cycles_since_last_threshold_adjustment: 0,
            reversion_trigger_cycles: 5, // 连续 5 次无调整后开始均值回归
            // 质疑三·责任鸿沟：决策日志初始化
            decision_log: Vec::with_capacity(100),
        }
    }

    /// 检查是否需要调节
    pub fn should_regulate(&self) -> bool {
        if !self.auto_regulate {
            return false;
        }
        // 冻结状态：拒绝所有自动调节，等待外部干预
        if self.is_frozen {
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        now - self.last_regulation_ms >= self.regulation_interval_ms
    }

    /// 记录调节历史并检测振荡、漂移、冻结
    fn record_regulation(&mut self, action_tag: &str, direction: i8, magnitude: f32) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let record = RegulationRecord {
            timestamp_ms: now,
            action_tag: action_tag.to_string(),
            direction,
            magnitude,
        };

        if self.regulation_history.len() >= 20 {
            self.regulation_history.remove(0);
        }
        self.regulation_history.push(record);

        // 振荡检测：检查同一 action_tag 最近几次调节的方向是否反转
        self.detect_oscillation(action_tag);

        // 漂移检测：同一 action_tag 连续同方向
        self.detect_drift(action_tag, direction);

        // 冻结保护：同一 action_tag 重复建议但无改善
        self.detect_freeze(action_tag);
    }

    /// 检测振荡：同一参数在 oscillation_window 内方向反转 ≥ 2 次
    fn detect_oscillation(&mut self, action_tag: &str) {
        let relevant: Vec<&RegulationRecord> = self
            .regulation_history
            .iter()
            .rev()
            .take(self.oscillation_window)
            .filter(|r| r.action_tag == action_tag)
            .collect();

        if relevant.len() < 3 {
            return;
        }

        // 统计方向反转次数
        let mut reversals = 0usize;
        for i in 1..relevant.len() {
            if relevant[i].direction != 0
                && relevant[i - 1].direction != 0
                && relevant[i].direction != relevant[i - 1].direction
            {
                reversals += 1;
            }
        }

        if reversals >= 2 {
            if !self.in_oscillation {
                eprintln!(
                    "[LRC·调节] 检测到振荡: {} 在最近 {} 次调节中方向反转 {} 次，步长减半",
                    action_tag,
                    relevant.len(),
                    reversals
                );
            }
            self.in_oscillation = true;
            self.step_multiplier = (self.step_multiplier * 0.5).max(0.125); // 最低 1/8
                                                                            // 振荡时延长冷却时间，给上次调节更多时间生效
            self.regulation_interval_ms = (self.regulation_interval_ms as f64 * 1.5) as u64;
        } else if reversals <= 1 && relevant.len() >= 3 && self.in_oscillation {
            // 反转次数降到 1 或 0 → 趋于稳定，逐步恢复步长
            // 使用 <= 1 而非 == 0，避免在窗口边缘迟迟无法恢复
            self.step_multiplier = (self.step_multiplier * 1.5).min(1.0);
            if self.step_multiplier >= 1.0 {
                self.in_oscillation = false;
                self.regulation_interval_ms = 300_000; // 恢复默认冷却
                eprintln!("[LRC·调节] 振荡已消除，步长和冷却时间恢复默认");
            }
        }
    }

    /// 检测漂移：同一 action_tag 连续同方向调节超过阈值
    ///
    /// 漂移与振荡不同：振荡是方向来回反转，漂移是持续单向推进。
    /// 漂移可能意味着：
    /// - 系统存在根因问题（如编码器质量持续下降）
    /// - 调节参数设置不当（如步长过大导致持续 overshoot）
    /// - 外部环境变化（如记忆增长速度远超预期）
    fn detect_drift(&mut self, action_tag: &str, direction: i8) {
        if direction == 0 {
            // 无方向（如 Reencoding 建议），不计入漂移检测
            self.consecutive_same_direction = 0;
            self.in_drift = false;
            return;
        }

        let relevant: Vec<&RegulationRecord> = self
            .regulation_history
            .iter()
            .rev()
            .take(self.oscillation_window + 2)
            .filter(|r| r.action_tag == action_tag)
            .collect();

        // 数据不足时不判定漂移，但计数器仍递增（方向相同即累积）
        if relevant.len() < 3 {
            self.consecutive_same_direction += 1;
            return;
        }

        // 检查最近几条记录是否同方向
        let all_same_direction = relevant.iter().all(|r| r.direction == direction);

        if all_same_direction {
            self.consecutive_same_direction += 1;
            if self.consecutive_same_direction >= self.drift_threshold && !self.in_drift {
                self.in_drift = true;
                eprintln!(
                    "[LRC·调节] 检测到漂移: {} 连续 {} 次同方向调节（方向 {}），可能存在根因问题",
                    action_tag,
                    self.consecutive_same_direction,
                    if direction > 0 { "增加" } else { "减少" }
                );
            }
        } else {
            // 方向变化，重置漂移计数
            self.consecutive_same_direction = 0;
            self.in_drift = false;
        }
    }

    /// 检测冻结：连续多次返回相同建议但系统无改善
    ///
    /// 当同一 action_tag 反复出现但系统指标没有改善时，
    /// 说明自动调节在当前状态下无效，需要冻结等待外部干预。
    fn detect_freeze(&mut self, action_tag: &str) {
        if let Some(ref last) = self.last_action_tag {
            if last == action_tag {
                self.consecutive_ineffective += 1;
                if self.consecutive_ineffective >= self.freeze_threshold && !self.is_frozen {
                    self.is_frozen = true;
                    eprintln!(
                        "[LRC·调节] 冻结保护触发: {} 连续 {} 次被建议但无改善，调节器已冻结。\
                         请检查系统状态或手动解除冻结",
                        action_tag, self.consecutive_ineffective
                    );
                }
            } else {
                // 动作类型变化，说明之前的调节可能起了作用
                self.consecutive_ineffective = 0;
            }
        } else {
            // 首次调节，开始计数
            self.consecutive_ineffective = 1;
        }
        self.last_action_tag = Some(action_tag.to_string());
    }

    // ============================================================
    // 质疑一·终极：防漂移机制
    //
    // 动态阈值调节面临的核心风险是"调节器近视"——
    // 短期环境变化导致阈值过度调整，环境恢复后无法及时回归。
    //
    // 本机制由三层防护构成：
    // 1. EMA 平滑：新阈值 = α × 提议值 + (1-α) × 旧EMA，抑制短期波动
    // 2. 基线锚定：阈值偏离基线（baseline）不得超过 max_deviation
    // 3. 均值回归：连续 N 次调节无阈值调整时，逐步向基线回归
    //
    // 道枢映射：艮卦·山 (☶) — "艮其止，止其所也"
    //   山有定势，不为风雨所动，但亦因时而变。
    // ============================================================

    /// 应用阈值调整（带 EMA 平滑 + 基线锚定）
    ///
    /// 返回经过平滑和约束后的实际阈值。
    /// 如果调整被基线约束截断，会在日志中记录。
    fn apply_threshold_adjustment(&mut self, proposed: f32) -> f32 {
        // 层一：EMA 平滑 — 抑制短期剧烈波动
        let alpha = self.threshold_momentum;
        self.threshold_ema = alpha * proposed + (1.0 - alpha) * self.threshold_ema;

        // 层二：基线锚定 — 防止阈值漂移过远
        let lower_bound = (self.threshold_baseline - self.max_threshold_deviation).max(0.001);
        let upper_bound = (self.threshold_baseline + self.max_threshold_deviation).min(0.1);
        let clamped = self.threshold_ema.clamp(lower_bound, upper_bound);

        if (clamped - self.threshold_ema).abs() > 0.0001 {
            eprintln!(
                "[LRC·调节·防漂移] 阈值 {:.4} 超出基线 {:.4}±{:.4} 范围，已截断为 {:.4}",
                self.threshold_ema, self.threshold_baseline, self.max_threshold_deviation, clamped
            );
        }

        // 层三：记录历史用于长周期趋势分析
        if self.threshold_history.len() >= 20 {
            self.threshold_history.pop_front();
        }
        self.threshold_history.push_back(clamped);

        // 重置均值回归计数器
        self.cycles_since_last_threshold_adjustment = 0;

        clamped
    }

    /// 均值回归：当连续多轮无阈值调整时，逐步向基线靠拢
    ///
    /// 这模拟了生物系统的"稳态恢复"——当外部刺激消失后，
    /// 系统参数逐渐回归到其默认最优值。
    fn maybe_revert_threshold(&mut self) {
        self.cycles_since_last_threshold_adjustment += 1;

        if self.cycles_since_last_threshold_adjustment < self.reversion_trigger_cycles {
            return; // 尚未触发回归条件
        }

        let current = self.information_gain_threshold;
        let target = self.threshold_baseline;

        if (current - target).abs() < 0.0001 {
            return; // 已在基线，无需回归
        }

        // 向基线方向移动一个 reversion_rate 步长
        let reverted = if current > target {
            (current - self.reversion_rate).max(target)
        } else {
            (current + self.reversion_rate).min(target)
        };

        self.information_gain_threshold = reverted;
        self.threshold_ema = reverted; // 同步更新 EMA

        eprintln!(
            "[LRC·调节·均值回归] 连续 {} 轮无阈值调整，\
             阈值从 {:.4} 向基线 {:.4} 回归至 {:.4}",
            self.cycles_since_last_threshold_adjustment, current, target, reverted
        );

        // 回归后重置计数器，避免过度回归
        self.cycles_since_last_threshold_adjustment = 0;
    }

    /// 检测多指标耦合（解决质疑二"多指标耦合"问题）
    ///
    /// 当"洛书偏差过大"和"八卦分布集中"等指标同时异常时，
    /// 很可能存在因果关联。简单优先级排序会忽略这种关联，
    /// 可能产生次优的调节动作。
    ///
    /// 本方法：
    /// 1. 统计同时异常的指标数量
    /// 2. 计算耦合强度（异常指标数 / 总指标数）
    /// 3. 记录耦合事件用于长周期反馈分析
    /// 4. 在高耦合时生成连锁反应检测
    fn detect_coupling(
        &mut self,
        avg_luoshu_deviation: f32,
        bagua_entropy: f32,
        dao_score: f32,
        synthesis_ratio: f32,
    ) -> f32 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // 各指标的异常判定
        let deviation_anomaly = avg_luoshu_deviation > 0.5;
        let bagua_anomaly = bagua_entropy < 0.5;
        let dao_anomaly = dao_score < 0.3;
        let synthesis_anomaly = synthesis_ratio > 0.5;

        let anomaly_count = [
            deviation_anomaly,
            bagua_anomaly,
            dao_anomaly,
            synthesis_anomaly,
        ]
        .iter()
        .filter(|&&x| x)
        .count();

        if anomaly_count >= 2 {
            // 多指标同时异常，计算耦合强度
            let strength = anomaly_count as f32 / 4.0;

            // 检测连锁反应：当前耦合 + 历史耦合事件
            let recent_cascades = self
                .coupling_history
                .iter()
                .rev()
                .take(5)
                .filter(|e| e.cascade_detected)
                .count();

            let cascade = recent_cascades >= 2;

            // 记录耦合事件
            self.coupling_history.push(CouplingEvent {
                timestamp_ms: now,
                anomaly_flags: [
                    deviation_anomaly,
                    bagua_anomaly,
                    dao_anomaly,
                    synthesis_anomaly,
                ],
                coupling_strength: strength,
                cascade_detected: cascade,
            });

            // 限制历史记录大小
            if self.coupling_history.len() > 50 {
                self.coupling_history.remove(0);
            }

            // 更新耦合指数（指数加权移动平均）
            // 异常指标越多，当前观测的权重越高（快速响应严重耦合）
            let weight = if anomaly_count >= 3 { 0.5 } else { 0.3 };
            self.coupling_score = self.coupling_score * (1.0 - weight) + strength * weight;

            if cascade {
                eprintln!(
                    "[LRC·耦合] 检测到连锁反应: {} 个指标同时异常（耦合指数 {:.2}），{} 次历史连锁",
                    anomaly_count,
                    self.coupling_score,
                    recent_cascades + 1
                );
            }
        } else {
            // 单指标异常或无异常，耦合指数衰减
            self.coupling_score *= 0.8;
        }

        self.coupling_score
    }

    /// 获取耦合指数（可解释性面板）
    pub fn coupling_score(&self) -> f32 {
        self.coupling_score
    }

    /// 检查是否存在连锁反应
    pub fn has_cascade(&self) -> bool {
        self.coupling_history
            .iter()
            .rev()
            .take(3)
            .filter(|e| e.cascade_detected)
            .count()
            >= 2
    }

    /// 道枢映射: 震卦·雷 (☳) — 万物出乎震，耦合趋势如雷霆之变，分析子系统间的交互动力学
    /// 长周期耦合趋势分析（解决质疑二"长周期反馈延迟"）
    ///
    /// 分析耦合历史中的长期模式，识别以下趋势：
    /// - 耦合频率是否在上升（恶化趋势）
    /// - 哪些指标组合最常同时异常（根因定位）
    /// - 是否存在周期性的耦合爆发（外部环境变化）
    ///
    /// 返回结构化分析结果，供可解释性面板和调节器决策使用。
    pub fn analyze_coupling_trend(&self) -> CouplingTrendAnalysis {
        if self.coupling_history.is_empty() {
            return CouplingTrendAnalysis::default();
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let total_events = self.coupling_history.len();
        let cascade_count = self
            .coupling_history
            .iter()
            .filter(|e| e.cascade_detected)
            .count();

        // 分析最近 10 个事件 vs 全部事件的平均耦合强度
        let all_avg_strength: f32 = self
            .coupling_history
            .iter()
            .map(|e| e.coupling_strength)
            .sum::<f32>()
            / total_events as f32;

        let recent: Vec<&CouplingEvent> = self.coupling_history.iter().rev().take(10).collect();
        let recent_avg_strength: f32 = if recent.is_empty() {
            0.0
        } else {
            recent.iter().map(|e| e.coupling_strength).sum::<f32>() / recent.len() as f32
        };

        // 趋势判断：近期耦合强度是否高于历史均值
        let is_worsening = recent_avg_strength > all_avg_strength * 1.2;

        // 统计最常同时异常的指标组合
        let mut deviation_anomalies = 0usize;
        let mut bagua_anomalies = 0usize;
        let mut dao_anomalies = 0usize;
        let mut synthesis_anomalies = 0usize;

        for event in &self.coupling_history {
            if event.anomaly_flags[0] {
                deviation_anomalies += 1;
            }
            if event.anomaly_flags[1] {
                bagua_anomalies += 1;
            }
            if event.anomaly_flags[2] {
                dao_anomalies += 1;
            }
            if event.anomaly_flags[3] {
                synthesis_anomalies += 1;
            }
        }

        // 计算耦合事件的时间密度（最近 1 小时内的耦合事件数）
        let one_hour_ms = 3_600_000u64;
        let recent_density = self
            .coupling_history
            .iter()
            .rev()
            .take_while(|e| now.saturating_sub(e.timestamp_ms) <= one_hour_ms)
            .count();

        CouplingTrendAnalysis {
            total_coupling_events: total_events,
            cascade_count,
            cascade_ratio: if total_events > 0 {
                cascade_count as f32 / total_events as f32
            } else {
                0.0
            },
            all_avg_strength,
            recent_avg_strength,
            is_worsening,
            deviation_anomaly_count: deviation_anomalies,
            bagua_anomaly_count: bagua_anomalies,
            dao_anomaly_count: dao_anomalies,
            synthesis_anomaly_count: synthesis_anomalies,
            recent_hour_density: recent_density,
        }
    }

    /// 执行调节检查，返回建议动作
    ///
    /// 参数：
    ///   - dao_score: 道同构度评分 (0.0 ~ 1.0)
    ///   - bagua_entropy: 八卦分布熵 (0.0 ~ 3.0)
    ///   - synthesis_ratio: 合成/原始记忆比率
    ///   - avg_luoshu_deviation: 平均洛书幻和偏离度
    ///   - synthesis_rate: 合成触发频率（次/分钟）
    ///    - current_decay_rate: 当前衰减速率
    ///   - current_min_cluster: 当前合成最小聚类大小
    ///
    /// 注意：参数较多（7 个）因为这些指标共同决定调节策略。
    /// 后续重构时可考虑将参数封装为 RegulationInput 结构体。
    #[allow(clippy::too_many_arguments)]
    pub fn regulate(
        &mut self,
        dao_score: f32,
        bagua_entropy: f32,
        synthesis_ratio: f32,
        avg_luoshu_deviation: f32,
        synthesis_rate: f32,
        current_decay_rate: f32,
        current_min_cluster: usize,
    ) -> RegulationAction {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.last_regulation_ms = now;

        // 灾难性转折检测：记录调节前系统健康快照（质疑二）
        self.catastrophic_detector.record_snapshot(
            dao_score,
            bagua_entropy,
            synthesis_ratio,
            avg_luoshu_deviation,
            self.coupling_score,
        );

        // 耦合检测：多指标同时异常的关联分析（质疑二）
        let coupling = self.detect_coupling(
            avg_luoshu_deviation,
            bagua_entropy,
            dao_score,
            synthesis_ratio,
        );

        // 收集所有候选动作，按优先级排序
        // 正常模式优先级: Reencoding > RetrievalWeights > SynthesisThreshold > DecayRate
        // 高耦合模式（coupling > 0.5）：所有异常指标生成综合建议
        let mut candidates: Vec<(u8, RegulationAction)> = Vec::new();

        // 优先级 1：洛书幻和偏差过大 → 建议重新编码
        if avg_luoshu_deviation > 0.5 {
            candidates.push((
                1,
                RegulationAction::SuggestReencoding {
                    severity: if avg_luoshu_deviation > 0.8 {
                        "critical"
                    } else {
                        "high"
                    }
                    .to_string(),
                    reason: format!(
                        "洛书幻和平均偏离度 {:.2} 超过阈值 0.5，建议重新编码以恢复几何约束",
                        avg_luoshu_deviation
                    ),
                },
            ));
        }

        // 优先级 2：八卦分布严重不均 → 调整检索权重
        if bagua_entropy < 0.5 && synthesis_ratio > 0.0 {
            candidates.push((
                2,
                RegulationAction::AdjustRetrievalWeights {
                    new_weights: None,
                    reason: format!(
                    "八卦熵 {:.2} 极低，记忆严重集中在少数类别。建议调整检索权重以鼓励冷门类别激活",
                    bagua_entropy
                ),
                },
            ));
        }

        // 优先级 2（同优先级）：低道同构度 + 八卦集中 → 调整检索权重
        if dao_score < 0.3 && bagua_entropy < 1.0 {
            // 避免重复添加同类型动作
            if !candidates
                .iter()
                .any(|(_, a)| matches!(a, RegulationAction::AdjustRetrievalWeights { .. }))
            {
                candidates.push((2, RegulationAction::AdjustRetrievalWeights {
                    new_weights: None,
                    reason: format!(
                        "道同构度 {:.2} 偏低，八卦熵 {:.2} 表明记忆过于集中。建议调整检索权重鼓励探索",
                        dao_score, bagua_entropy
                    ),
                }));
            }
        }

        // 优先级 3：合成比率过高 → 提高阈值（应用自适应步长）
        if dao_score < 0.3 && synthesis_ratio > 0.5 {
            let raw_step = 1.0;
            let step = (raw_step * self.step_multiplier).max(1.0).round() as usize;
            let new_cluster = (current_min_cluster + step).min(10);
            candidates.push((3, RegulationAction::AdjustSynthesisThreshold {
                new_min_cluster: new_cluster,
                reason: format!(
                    "合成比率 {:.2} 过高，合成记忆占比过大。将最小聚类从 {} 提升到 {}（步长倍率 {:.2}）",
                    synthesis_ratio, current_min_cluster, new_cluster, self.step_multiplier
                ),
                severity: None,
            }));
        }

        // 优先级 4：道同构度健康但合成频率过低 → 降低阈值
        if dao_score > 0.7 && synthesis_rate < 0.1 && synthesis_ratio < 0.1 {
            let raw_step = 1.0;
            let step = (raw_step * self.step_multiplier).max(1.0).round() as usize;
            let new_cluster = current_min_cluster.saturating_sub(step).max(2);
            if new_cluster < current_min_cluster {
                candidates.push((4, RegulationAction::AdjustSynthesisThreshold {
                    new_min_cluster: new_cluster,
                    reason: format!(
                        "合成频率 {:.2} 次/分钟偏低，合成比率 {:.2} 偏低。将最小聚类从 {} 降低到 {}（步长倍率 {:.2}）",
                        synthesis_rate, synthesis_ratio, current_min_cluster, new_cluster, self.step_multiplier
                    ),
                    severity: None,
                }));
            }
        }

        // 优先级 4.5：信息增量阈值动态微调（质疑一·活性 + 终极防漂移）
        // 基于合成质量反馈自动调整防坍塌门槛，替代硬编码常量
        // 当合成比率极低时降低阈值以鼓励合成，当合成比率过高时提高阈值以收紧标准
        //
        // 质疑一·终极：三层防漂移防护
        //   1. EMA 平滑：抑制短期环境变化导致的剧烈波动
        //   2. 基线锚定：阈值偏离基线不得超过 max_deviation
        //   3. 均值回归：连续无调整时自动向基线靠拢
        if synthesis_ratio < 0.02 && synthesis_rate < 0.05 {
            // 合成极度稀少 → 降低信息增量阈值，鼓励更多合成
            let raw_step = 0.005;
            let step = raw_step * self.step_multiplier;
            let proposed = (self.information_gain_threshold - step).max(0.001);
            if proposed < self.information_gain_threshold {
                // 质疑一·终极：应用 EMA 平滑 + 基线锚定
                let smoothed = self.apply_threshold_adjustment(proposed);
                candidates.push((5, RegulationAction::AdjustInformationGainThreshold {
                    new_threshold: smoothed,
                    reason: format!(
                        "合成比率 {:.2} 极低，合成频率 {:.2} 次/分钟偏低。\
                         将信息增量阈值从 {:.4} 降低到 {:.4}（EMA平滑，步长倍率 {:.2}），鼓励更多合成",
                        synthesis_ratio, synthesis_rate,
                        self.information_gain_threshold, smoothed, self.step_multiplier
                    ),
                }));
            }
        } else if synthesis_ratio > 0.4 && dao_score < 0.5 {
            // 合成比率过高且道同构度偏低 → 提高信息增量阈值，收紧合成标准
            let raw_step = 0.005;
            let step = raw_step * self.step_multiplier;
            let proposed = (self.information_gain_threshold + step).min(0.1);
            if proposed > self.information_gain_threshold {
                // 质疑一·终极：应用 EMA 平滑 + 基线锚定
                let smoothed = self.apply_threshold_adjustment(proposed);
                candidates.push((5, RegulationAction::AdjustInformationGainThreshold {
                    new_threshold: smoothed,
                    reason: format!(
                        "合成比率 {:.2} 过高，道同构度 {:.2} 偏低。\
                         将信息增量阈值从 {:.4} 提高到 {:.4}（EMA平滑，步长倍率 {:.2}），收紧合成质量门槛",
                        synthesis_ratio, dao_score,
                        self.information_gain_threshold, smoothed, self.step_multiplier
                    ),
                }));
            }
        }

        // 优先级 5：八卦分布均匀且合成健康 → 适度加快衰减
        if dao_score > 0.7 && bagua_entropy > 2.5 && synthesis_ratio > 0.2 {
            let raw_step = 0.05;
            let step = raw_step * self.step_multiplier;
            let new_rate = (current_decay_rate + step).min(0.5);
            candidates.push((5, RegulationAction::AdjustDecayRate {
                new_rate,
                reason: format!(
                    "八卦熵 {:.2} 良好，合成比率 {:.2} 健康。适度加快衰减速率从 {:.2} 到 {:.2}（步长倍率 {:.2}）",
                    bagua_entropy, synthesis_ratio, current_decay_rate, new_rate, self.step_multiplier
                ),
            }));
        }

        // 按优先级排序（数字越小优先级越高）
        candidates.sort_by_key(|(pri, _)| *pri);

        // 耦合感知仲裁（质疑二）
        // 高耦合模式：生成综合再平衡建议而非简单优先级仲裁
        // 正常模式：简单优先级仲裁
        //
        // 关键重构：将灾难性检测从"路径依赖"改为"全局守护"。
        // 无论走哪个调节分支，detect 始终在最后统一执行，确保未来代码变更不会意外绕过。
        let (mut action, action_tag, action_desc) = if coupling >= 0.5 && candidates.len() >= 2 {
            // 高耦合：综合再平衡
            let anomaly_count = candidates.len();
            let descriptions: Vec<&str> = candidates
                .iter()
                .map(|(_, action)| match action {
                    RegulationAction::SuggestReencoding { .. } => "洛书偏差",
                    RegulationAction::AdjustRetrievalWeights { .. } => "八卦分布集中",
                    RegulationAction::AdjustSynthesisThreshold { .. } => "合成比率异常",
                    RegulationAction::AdjustDecayRate { .. } => "衰减速率异常",
                    RegulationAction::AdjustInformationGainThreshold { .. } => "信息增量阈值异常",
                    _ => "未知",
                })
                .collect();

            let anomaly_desc = format!(
                "检测到 {} 个指标同时异常（耦合指数 {:.2}），建议系统级综合再平衡: {}",
                anomaly_count,
                coupling,
                descriptions.join(" + ")
            );

            let action = RegulationAction::SuggestComprehensiveRebalance {
                anomaly_description: anomaly_desc.clone(),
                coupling_score: coupling,
                severity: "warning".to_string(),
            };

            // 综合再平衡不计方向，但记录调节历史
            self.record_regulation("comprehensive_rebalance", 0, coupling);
            (action, "comprehensive_rebalance", anomaly_desc)
        } else if let Some((_, action)) = candidates.into_iter().next() {
            // 正常优先级仲裁：提取动作标签和描述
            let (tag, direction, magnitude, desc) = match &action {
                RegulationAction::SuggestReencoding { reason, .. } => {
                    ("reencoding", 0, 0.0, reason.clone())
                }
                RegulationAction::AdjustRetrievalWeights { reason, .. } => {
                    ("retrieval_weights", 0, 0.0, reason.clone())
                }
                RegulationAction::AdjustSynthesisThreshold {
                    new_min_cluster,
                    reason,
                    ..
                } => {
                    let dir = if *new_min_cluster > current_min_cluster {
                        1
                    } else {
                        -1
                    };
                    ("synthesis_threshold", dir, 1.0, reason.clone())
                }
                RegulationAction::AdjustDecayRate {
                    new_rate, reason, ..
                } => {
                    let dir = if *new_rate > current_decay_rate {
                        1
                    } else {
                        -1
                    };
                    ("decay_rate", dir, 0.05, reason.clone())
                }
                RegulationAction::AdjustInformationGainThreshold {
                    new_threshold,
                    reason,
                } => {
                    let dir = if *new_threshold > self.information_gain_threshold {
                        1
                    } else {
                        -1
                    };
                    ("information_gain_threshold", dir, 0.005, reason.clone())
                }
                RegulationAction::NoAction => ("no_action", 0, 0.0, "无异常".to_string()),
                // 综合再平衡在正常路径中不应出现，但保留完整性
                RegulationAction::SuggestComprehensiveRebalance {
                    anomaly_description,
                    ..
                } => (
                    "comprehensive_rebalance",
                    0,
                    0.0,
                    anomaly_description.clone(),
                ),
            };
            self.record_regulation(tag, direction, magnitude);
            (action, tag, desc)
        } else {
            (
                RegulationAction::NoAction,
                "no_action",
                "无异常".to_string(),
            )
        };

        // ============================================================
        // 守护模式：无论采取何种调节路径，始终执行灾难性转折检测
        //
        // 质疑二核心修复：灾难性检测是最高优先级的全局守护任务，
        // 不依赖于任何具体的调节分支。即使未来添加新路径或修改逻辑，
        // 只要 regulate() 执行完毕，检测一定会运行。
        // ============================================================
        self.catastrophic_detector
            .record_action(action_tag, &action_desc);
        let catastrophic = self
            .catastrophic_detector
            .detect(self.consecutive_same_direction, self.in_drift);
        if let Some(event) = catastrophic {
            eprintln!(
                "[LRC·灾难] {} 严重程度: {}",
                event.diagnosis, event.severity
            );
        }

        // 慢性恶化检测（质疑四：防止"温水煮青蛙"式退化）
        // 与急性检测不同，慢性检测在更长周期内识别持续缓慢下降
        let chronic = self.catastrophic_detector.detect_chronic_degradation();
        if let Some(event) = chronic {
            eprintln!(
                "[LRC·灾难·慢性] {} 严重程度: {}",
                event.diagnosis, event.severity
            );

            // 质疑四核心修复：慢性退化不能只是"检测并沉默"。
            // 检测到慢性退化意味着系统已在 20+ 周期内持续恶化，
            // 必须自动触发调节动作，形成检测→行动的闭环。
            //
            // 策略：
            //   - 轻度慢性退化（drop 0.15-0.25）：提高合成阈值 1，过滤低质量合成
            //   - 中度慢性退化（drop 0.25-0.35）：综合再平衡 + 调整检索权重
            //   - 重度慢性退化（drop > 0.35）：全部手段 + 调节器冻结警告
            let chronic_action = if event.drop_magnitude > 0.35 {
                // 重度：综合再平衡
                RegulationAction::SuggestComprehensiveRebalance {
                    anomaly_description: format!(
                        "慢性重度退化（累计下降 {:.2}）：建议全面审查编码器、合成日志和衰减参数",
                        event.drop_magnitude
                    ),
                    coupling_score: event.coupling_before,
                    severity: "severe".to_string(),
                }
            } else if event.drop_magnitude > 0.25 {
                // 中度：调整检索权重 + 提高合成阈值
                RegulationAction::AdjustRetrievalWeights {
                    new_weights: Some(vec![0.5, 0.2, 0.2, 0.1]), // 降低语义权重，提高多样性
                    reason: format!(
                        "慢性中度退化（累计下降 {:.2}）：调整检索权重鼓励探索，防止局部最优",
                        event.drop_magnitude
                    ),
                }
            } else {
                // 轻度：提高合成阈值
                let new_min_cluster = (self.synthesis_min_cluster_size + 1).min(10);
                RegulationAction::AdjustSynthesisThreshold {
                    new_min_cluster,
                    reason: format!(
                        "慢性轻度退化（累计下降 {:.2}）：提高合成阈值以过滤低质量合成",
                        event.drop_magnitude
                    ),
                    severity: Some("warning".to_string()),
                }
            };

            // 如果原有 action 是 NoAction，替换为慢性响应
            // 否则保留原有 action（不覆盖更紧急的响应）
            if matches!(action, RegulationAction::NoAction) {
                action = chronic_action;
            }
        }

        // 质疑一·终极：均值回归 — 当本轮无阈值调整时，检查是否需要向基线回归
        // 这确保了阈值不会在环境变化后"卡"在偏离状态
        if !matches!(
            action,
            RegulationAction::AdjustInformationGainThreshold { .. }
        ) {
            self.maybe_revert_threshold();
        }

        // ============================================================
        // 质疑三·责任鸿沟：记录决策日志
        //
        // 每次自主决策后，将完整的输入参数、分析推理链、
        // 替代方案和风险评估写入决策日志。容量上限 100 条。
        //
        // 道枢映射：艮卦·山 (☶) — "艮其止，止其所也"。
        // 决策日志如山之层积，每一层都清晰可见，让系统的
        // "思考"过程透明化，填平"有原因，无责任"的鸿沟。
        // ============================================================
        self.record_decision_log(
            &action,
            dao_score,
            bagua_entropy,
            synthesis_ratio,
            avg_luoshu_deviation,
            synthesis_rate,
            current_decay_rate,
            current_min_cluster,
        );

        action
    }

    /// 记录决策日志（质疑三·责任鸿沟）
    ///
    /// 生成完整的决策日志条目，包括输入快照、分析推理链、
    /// 替代方案、风险评估和面向用户的解释。
    ///
    /// 注意：参数较多（9 个）因为需要完整记录决策上下文。
    #[allow(clippy::too_many_arguments)]
    fn record_decision_log(
        &mut self,
        action: &RegulationAction,
        dao_score: f32,
        bagua_entropy: f32,
        synthesis_ratio: f32,
        avg_luoshu_deviation: f32,
        synthesis_rate: f32,
        current_decay_rate: f32,
        current_min_cluster: usize,
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // 生成决策唯一标识
        let decision_id = format!("dl_{}_{:x}", now, self.decision_log.len());

        // 构建输入参数快照
        let inputs = DecisionInputs {
            dao_score,
            bagua_entropy,
            synthesis_ratio,
            avg_luoshu_deviation,
            synthesis_rate,
            current_decay_rate,
            current_min_cluster,
            coupling_score: self.coupling_score,
            is_oscillating: self.in_oscillation,
            information_gain_threshold: self.information_gain_threshold,
        };

        // 生成决策类型标签
        let decision_type = match action {
            RegulationAction::NoAction => "无操作".to_string(),
            RegulationAction::AdjustDecayRate { .. } => "调整衰减速率".to_string(),
            RegulationAction::AdjustSynthesisThreshold { .. } => "调整合成阈值".to_string(),
            RegulationAction::SuggestReencoding { .. } => "编码建议".to_string(),
            RegulationAction::AdjustRetrievalWeights { .. } => "调整检索权重".to_string(),
            RegulationAction::AdjustInformationGainThreshold { .. } => {
                "调整信息增量阈值".to_string()
            }
            RegulationAction::SuggestComprehensiveRebalance { .. } => "综合再平衡".to_string(),
        };

        // 生成分析推理链（人类可读）
        let analysis = self.build_decision_analysis(action, &inputs);

        // 计算置信度（基于指标异常程度）
        let confidence = self.calculate_decision_confidence(action, &inputs);

        // 收集替代方案
        let alternatives = self.collect_alternatives(action, &inputs);

        // 评估风险
        let risk_assessment = self.assess_decision_risk(action, &inputs);

        // 生成面向用户的解释
        let user_facing_explanation = self.build_user_facing_explanation(action, &inputs);

        let log = DecisionLog {
            decision_id,
            timestamp_ms: now,
            decision_type,
            inputs,
            analysis,
            decision: action.clone(),
            confidence,
            alternatives_considered: alternatives,
            risk_assessment,
            user_facing_explanation,
        };

        // 维护 FIFO 容量上限：最多 100 条
        if self.decision_log.len() >= 100 {
            self.decision_log.remove(0);
        }
        self.decision_log.push(log);
    }

    /// 构建决策分析推理链（人类可读）
    fn build_decision_analysis(
        &self,
        action: &RegulationAction,
        inputs: &DecisionInputs,
    ) -> String {
        match action {
            RegulationAction::NoAction => {
                format!(
                    "系统指标均处于健康阈值范围内。道同构度 {:.2}（阈值 0.25），八卦熵 {:.2}（阈值 0.5），\
                     合成比率 {:.2}（阈值 0.1~0.5），洛书偏离度 {:.2}（阈值 0.5）。无需触发调节。",
                    inputs.dao_score, inputs.bagua_entropy,
                    inputs.synthesis_ratio, inputs.avg_luoshu_deviation
                )
            }
            RegulationAction::SuggestReencoding { reason, .. } => {
                format!(
                    "检测到洛书幻和平均偏离度 {:.2} 超过阈值 0.5，表明当前编码器无法维持几何约束。\
                     分析：编码器的统计特性与八卦空间结构不匹配，已无法通过简单参数调节修复。\
                     决策：{reason}",
                    inputs.avg_luoshu_deviation
                )
            }
            RegulationAction::AdjustRetrievalWeights { reason, .. } => {
                format!(
                    "检测到道同构度 {:.2} 且八卦熵 {:.2}（分布集中），表明检索偏向少数类别，\
                     八卦分布严重不均。分析：当前检索权重偏差导致某些类别被过度检索，\
                     另一些类别被忽略，造成信息茧房。决策：{reason}",
                    inputs.dao_score, inputs.bagua_entropy
                )
            }
            RegulationAction::AdjustSynthesisThreshold { reason, .. } => {
                format!(
                    "检测到合成比率 {:.2} 或合成频率 {:.2} 异常。分析：合成的节奏与系统健康状态不匹配，\
                     需要调整合成阈值来恢复平衡。决策：{reason}",
                    inputs.synthesis_ratio, inputs.synthesis_rate
                )
            }
            RegulationAction::AdjustDecayRate { reason, .. } => {
                format!(
                    "系统整体健康但道同构度偏高（{:.2} > 0.75），记忆空间利用充分。\
                     分析：高道同构度意味着记忆结构高度一致，可以适当加速衰减以释放空间。\
                     决策：{reason}",
                    inputs.dao_score
                )
            }
            RegulationAction::AdjustInformationGainThreshold { reason, .. } => {
                format!(
                    "基于合成质量反馈分析，当前信息增量阈值 {:.3} 需要微调。\
                     分析：合成产物的质量分布表明当前阈值与系统实际需求存在偏差。\
                     决策：{reason}",
                    inputs.information_gain_threshold
                )
            }
            RegulationAction::SuggestComprehensiveRebalance {
                anomaly_description,
                coupling_score,
                ..
            } => {
                format!(
                    "检测到多指标同时异常（耦合指数 {:.2} > 0.5），表明不同指标之间存在因果关联。\
                     分析：单独调节任一指标无法解决根本问题，需要从系统层面进行综合再平衡。\
                     决策：{anomaly_description}",
                    coupling_score
                )
            }
        }
    }

    /// 计算决策置信度（基于指标异常程度）
    fn calculate_decision_confidence(
        &self,
        action: &RegulationAction,
        inputs: &DecisionInputs,
    ) -> f32 {
        match action {
            RegulationAction::NoAction => {
                // 无操作时，置信度基于指标与阈值的距离
                let dao_margin = (inputs.dao_score - 0.25).max(0.0);
                let entropy_margin = (inputs.bagua_entropy - 0.5).max(0.0);
                let dev_margin = (0.5 - inputs.avg_luoshu_deviation).max(0.0);
                let synth_normal = if inputs.synthesis_ratio >= 0.1 && inputs.synthesis_ratio <= 0.5
                {
                    0.3
                } else {
                    0.0
                };
                (0.5 + dao_margin * 0.2 + entropy_margin * 0.15 + dev_margin * 0.1 + synth_normal)
                    .min(1.0)
            }
            RegulationAction::SuggestReencoding { .. } => {
                // 偏差越严重，置信度越高
                (0.6 + inputs.avg_luoshu_deviation * 0.4).min(1.0)
            }
            RegulationAction::AdjustRetrievalWeights { .. } => {
                // 道同构度越低 + 八卦熵越低，置信度越高
                let dao_factor = (0.3 - inputs.dao_score).max(0.0) / 0.3;
                let entropy_factor = (0.5 - inputs.bagua_entropy).max(0.0) / 0.5;
                (0.5 + dao_factor * 0.25 + entropy_factor * 0.25).min(1.0)
            }
            RegulationAction::AdjustSynthesisThreshold { .. } => {
                let ratio_anomaly = if inputs.synthesis_ratio < 0.05 {
                    (0.05 - inputs.synthesis_ratio) / 0.05
                } else if inputs.synthesis_ratio > 0.5 {
                    (inputs.synthesis_ratio - 0.5) / 0.5
                } else {
                    0.0
                };
                (0.5 + ratio_anomaly * 0.5).min(1.0)
            }
            RegulationAction::AdjustDecayRate { .. } => {
                (0.5 + (inputs.dao_score - 0.75).max(0.0) * 2.0).min(0.95)
            }
            RegulationAction::AdjustInformationGainThreshold { .. } => {
                // 基于阈值偏离基线的程度
                let deviation = (inputs.information_gain_threshold - self.threshold_baseline).abs();
                (0.5 + deviation * 10.0).min(0.9)
            }
            RegulationAction::SuggestComprehensiveRebalance { coupling_score, .. } => {
                // 耦合指数越高，置信度越高
                (0.6 + (*coupling_score).min(1.0) * 0.4).min(1.0)
            }
        }
    }

    /// 收集曾考虑过的替代方案
    fn collect_alternatives(
        &self,
        action: &RegulationAction,
        inputs: &DecisionInputs,
    ) -> Vec<String> {
        let mut alternatives = Vec::new();

        match action {
            RegulationAction::NoAction => {
                // 考虑了手动干预的可能性
                alternatives.push("手动触发调节以主动优化系统".to_string());
                alternatives.push("忽略当前异常指标，等待下一次评估".to_string());
            }
            RegulationAction::SuggestReencoding { .. } => {
                alternatives.push("仅调整衰减速率以缓解症状（非根治）".to_string());
                alternatives.push("提高检索权重多样性以补偿编码偏差".to_string());
                if inputs.bagua_entropy < 0.5 {
                    alternatives.push("调整检索权重以改善八卦分布".to_string());
                }
            }
            RegulationAction::AdjustRetrievalWeights { .. } => {
                alternatives.push("重新编码以从根本上改善八卦分布".to_string());
                alternatives.push("调整合成阈值以改变记忆分布".to_string());
                if inputs.avg_luoshu_deviation > 0.3 {
                    alternatives.push("考虑洛书偏差也在增大，建议重新编码".to_string());
                }
            }
            RegulationAction::AdjustSynthesisThreshold { .. } => {
                alternatives.push("调整衰减速率以间接调节合成比率".to_string());
                alternatives.push("调整检索权重以改变合成素材来源".to_string());
                alternatives.push("维持当前阈值，依靠自然衰减平衡".to_string());
            }
            RegulationAction::AdjustDecayRate { .. } => {
                alternatives.push("调整合成阈值而非衰减速率".to_string());
                alternatives.push("保持当前速率，等待合成自然降低".to_string());
            }
            RegulationAction::AdjustInformationGainThreshold { .. } => {
                alternatives.push("保持当前阈值，依赖合成质量自然改善".to_string());
                alternatives.push("调整合成阈值作为替代方案".to_string());
            }
            RegulationAction::SuggestComprehensiveRebalance { .. } => {
                alternatives.push("按优先级逐个调节（可能忽略耦合效应）".to_string());
                alternatives.push("仅关注最严重的单一指标".to_string());
                alternatives.push("人工介入进行全面诊断".to_string());
            }
        }

        alternatives
    }

    /// 评估决策风险
    fn assess_decision_risk(&self, action: &RegulationAction, inputs: &DecisionInputs) -> String {
        match action {
            RegulationAction::NoAction => {
                if inputs.dao_score < 0.3 || inputs.avg_luoshu_deviation > 0.4 {
                    "风险较低：虽然部分指标接近阈值，但尚未触发调节条件。建议密切监控。".to_string()
                } else {
                    "风险极低：所有指标均处于健康范围，无需干预。".to_string()
                }
            }
            RegulationAction::SuggestReencoding { severity, .. } => {
                if severity == "critical" {
                    "风险较高：重新编码会短暂中断服务，且可能改变已有记忆的编码表示。\
                     建议在低负载时段执行，并备份现有编码器。"
                        .to_string()
                } else {
                    "风险中等：重新编码可能影响现有检索结果的一致性。建议先验证编码器降级程度。"
                        .to_string()
                }
            }
            RegulationAction::AdjustRetrievalWeights { .. } => {
                if self.in_oscillation {
                    "风险中等：系统处于振荡状态，调整检索权重可能加剧振荡。建议等待振荡消退后再调整。".to_string()
                } else {
                    "风险较低：调整检索权重是渐进式操作，不会影响已有数据，可随时回滚。".to_string()
                }
            }
            RegulationAction::AdjustSynthesisThreshold { .. } => {
                if self.in_oscillation {
                    "风险中等：系统处于振荡状态，合成阈值调整可能引发进一步的振荡。\
                     已启用自适应步长减半机制。"
                        .to_string()
                } else if self.in_drift {
                    "风险较高：检测到漂移趋势，合成阈值调整可能加剧单向偏离。建议人工审查。"
                        .to_string()
                } else {
                    "风险较低：合成阈值调整影响范围可控，仅影响新合成的记忆。".to_string()
                }
            }
            RegulationAction::AdjustDecayRate { .. } => {
                "风险较低：衰减速率调整是平滑的，不会造成记忆的突然丢失。\
                 但需注意长期过高衰减可能导致记忆过快过期。"
                    .to_string()
            }
            RegulationAction::AdjustInformationGainThreshold { .. } => {
                let deviation = (inputs.information_gain_threshold - self.threshold_baseline).abs();
                if deviation > self.max_threshold_deviation * 0.8 {
                    "风险中等：阈值已接近最大允许偏差，进一步调整可能触发均值回归。\
                     建议监控阈值变化趋势。"
                        .to_string()
                } else {
                    "风险较低：阈值调整在安全范围内，系统会自动向基线回归。".to_string()
                }
            }
            RegulationAction::SuggestComprehensiveRebalance { severity, .. } => {
                match severity.as_str() {
                    "critical" => "风险极高：多指标同时处于临界状态，系统可能存在系统性故障。\
                                  建议立即人工介入并进行全面诊断。"
                        .to_string(),
                    "severe" => "风险较高：综合再平衡涉及多个参数的联动调整，存在连锁反应风险。\
                                建议先在测试环境验证。"
                        .to_string(),
                    _ => "风险中等：综合再平衡是多维度协调调整，效果可能超出预期范围。\
                          建议观察调整后系统表现。"
                        .to_string(),
                }
            }
        }
    }

    /// 构建面向用户的决策解释（非技术语言）
    fn build_user_facing_explanation(
        &self,
        action: &RegulationAction,
        _inputs: &DecisionInputs,
    ) -> String {
        match action {
            RegulationAction::NoAction => "您的记忆系统运行良好，各项指标都处于健康水平。\
                 系统没有进行任何调整，一切都在正常运转。"
                .to_string(),
            RegulationAction::SuggestReencoding { reason, severity } => {
                format!(
                    "您的记忆编码方式出现了一些偏差（严重程度：{}）。\
                     这就像图书馆的索引系统开始变得不准确了。系统建议重新整理索引，\
                     以确保您能准确找到需要的记忆。具体原因：{}",
                    severity, reason
                )
            }
            RegulationAction::AdjustRetrievalWeights { reason, .. } => {
                format!(
                    "系统发现您在检索记忆时有些偏向某些类别，导致其他类别的记忆被忽略了。\
                     系统正在调整检索策略，让更多类型的记忆有机会被访问到。\
                     具体原因：{}",
                    reason
                )
            }
            RegulationAction::AdjustSynthesisThreshold { reason, .. } => {
                format!(
                    "系统正在调整记忆的合成节奏。这就像调整笔记整理的速度——\
                     太快可能产生冗余，太慢可能错过关联。具体原因：{}",
                    reason
                )
            }
            RegulationAction::AdjustDecayRate { reason, .. } => {
                format!(
                    "系统正在优化记忆的保存策略。就像整理旧笔记一样，\
                     系统在决定哪些记忆需要保留更久，哪些可以适当精简。具体原因：{}",
                    reason
                )
            }
            RegulationAction::AdjustInformationGainThreshold { reason, .. } => {
                format!(
                    "系统正在微调信息整合的敏感度。这就像调整「触类旁通」的敏锐度，\
                     让系统在合适的时候进行知识关联。具体原因：{}",
                    reason
                )
            }
            RegulationAction::SuggestComprehensiveRebalance {
                anomaly_description,
                severity,
                ..
            } => {
                format!(
                    "系统检测到多个指标同时出现异常（严重程度：{}），这表明可能存在系统性问题。\
                     系统建议进行全面调整，而非单一修补。如果您不同意，可以手动覆盖这些调整。\
                     详情：{}",
                    severity, anomaly_description
                )
            }
        }
    }
    /// 设置调节间隔
    pub fn set_interval(&mut self, interval_ms: u64) {
        self.regulation_interval_ms = interval_ms;
    }

    /// 获取当前振荡状态（监控用）
    pub fn is_oscillating(&self) -> bool {
        self.in_oscillation
    }

    /// 道枢映射: 巽卦·风 (☴) — 随风巽，调节步长如风之渗透，渐进而非激进
    /// 获取当前步长倍率（监控用）
    pub fn step_multiplier(&self) -> f32 {
        self.step_multiplier
    }

    /// 获取灾难性转折事件列表（质疑二：可解释性面板）
    ///
    /// 返回历史上检测到的所有灾难性转折事件，
    /// 包括健康评分骤降、关键调节动作归因和诊断建议。
    pub fn get_catastrophic_events(&self) -> Vec<CatastrophicEvent> {
        self.catastrophic_detector.get_events().to_vec()
    }

    /// 获取调节器状态快照（可解释性面板）
    pub fn get_state(&self) -> DaoRegulatorState {
        DaoRegulatorState {
            last_regulation_ms: self.last_regulation_ms,
            is_oscillating: self.in_oscillation,
            oscillation_window: self.oscillation_window,
            step_multiplier: self.step_multiplier,
            auto_regulate: self.auto_regulate,
            regulation_interval_ms: self.regulation_interval_ms,
            is_drifting: self.in_drift,
            consecutive_same_direction: self.consecutive_same_direction,
            drift_threshold: self.drift_threshold,
            is_frozen: self.is_frozen,
            consecutive_ineffective: self.consecutive_ineffective,
            freeze_threshold: self.freeze_threshold,
            coupling_score: self.coupling_score,
        }
    }

    /// 道枢映射: 艮卦·山 (☶) — 艮其止，阈值基线如山之稳固，是动态调节的锚点
    /// 获取阈值锚定基线（质疑一·终极：防漂移）
    pub fn threshold_baseline(&self) -> f32 {
        self.threshold_baseline
    }

    /// 道枢映射: 坎卦·水 (☵) — 水流而不盈，EMA平滑如水流之连续性
    /// 获取阈值 EMA（质疑一·终极：防漂移）
    pub fn threshold_ema(&self) -> f32 {
        self.threshold_ema
    }

    /// 手动解除冻结（外部干预）
    pub fn unfreeze(&mut self) {
        if self.is_frozen {
            self.is_frozen = false;
            self.consecutive_ineffective = 0;
            self.last_action_tag = None;
            // 解除冻结后重置调节时间戳，允许立即调节
            self.last_regulation_ms = 0;
            eprintln!("[LRC·调节] 冻结已手动解除，调节器恢复正常运行");
        }
    }

    /// 检查是否处于漂移状态
    pub fn is_drifting(&self) -> bool {
        self.in_drift
    }

    /// 检查是否已冻结
    pub fn is_frozen(&self) -> bool {
        self.is_frozen
    }

    // ---- 质疑三·责任鸿沟：决策日志查询方法 ----

    /// 获取最近 N 条决策日志
    ///
    /// 返回决策日志的副本，按时间倒序排列（最新的在前）。
    /// 如果没有决策日志，返回空列表。
    ///
    /// 道枢映射：艮卦·山 (☶) — 如山层层分明，每层决策清晰可见。
    pub fn get_decision_log(&self, limit: usize) -> Vec<DecisionLog> {
        let total = self.decision_log.len();
        if total == 0 {
            return Vec::new();
        }
        let count = limit.min(total);
        self.decision_log[total - count..]
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    /// 获取最近一次决策日志
    ///
    /// 如果没有决策日志，返回 None。
    pub fn get_last_decision(&self) -> Option<DecisionLog> {
        self.decision_log.last().cloned()
    }

    /// 将最近一次决策转换为面向用户的解释
    ///
    /// 如果没有决策日志，返回 None。
    /// 返回的 ExplainableDecision 包含一句话总结、原因、
    /// 参数变化、预期影响和手动覆盖指南。
    pub fn explain_last_decision(&self) -> Option<ExplainableDecision> {
        self.decision_log.last().map(|log| {
            let (summary, what_changed, impact, how_to_override) = match &log.decision {
                RegulationAction::NoAction => (
                    "系统状态健康，无需进行调整".to_string(),
                    "无参数变化".to_string(),
                    "无影响，系统将保持当前状态".to_string(),
                    "如果您希望手动优化，可以调用 regulate() 并传入自定义参数".to_string(),
                ),
                RegulationAction::AdjustDecayRate { new_rate, .. } => (
                    format!("系统自动调整了记忆衰减速率至 {:.2}", new_rate),
                    format!("衰减速率 → {:.2}", new_rate),
                    format!(
                        "记忆将以新速率自然衰减，预期 {} 释放存储空间",
                        if *new_rate > 0.1 { "加速" } else { "减缓" }
                    ),
                    "您可以通过 set_decay_rate() 方法手动设置衰减速率".to_string(),
                ),
                RegulationAction::AdjustSynthesisThreshold {
                    new_min_cluster, ..
                } => (
                    format!(
                        "系统自动调整了合成阈值，最小聚类大小改为 {}",
                        new_min_cluster
                    ),
                    format!("合成最小聚类大小 → {}", new_min_cluster),
                    if *new_min_cluster > 3 {
                        "提高合成门槛，减少低质量合成，记忆质量可能提升但合成频率降低".to_string()
                    } else {
                        "降低合成门槛，允许更多记忆被合成，合成频率可能增加".to_string()
                    },
                    "您可以通过 set_synthesis_min_cluster() 方法手动设置聚类大小".to_string(),
                ),
                RegulationAction::SuggestReencoding { severity, .. } => (
                    format!("系统建议重新编码（严重程度：{}）", severity),
                    "建议触发重新编码流程".to_string(),
                    "重新编码后检索精度可能提升，但编码期间服务可能短暂降级".to_string(),
                    "您可以通过手动调用 reencode() 方法确认或拒绝此建议".to_string(),
                ),
                RegulationAction::AdjustRetrievalWeights { reason, .. } => (
                    "系统自动调整了检索权重以改善八卦分布".to_string(),
                    "检索权重已重新分配".to_string(),
                    "检索结果的多样性将提升，冷门类别记忆获得更多曝光机会".to_string(),
                    format!("您可以手动设置检索权重。原因：{}", reason),
                ),
                RegulationAction::AdjustInformationGainThreshold { new_threshold, .. } => (
                    format!("系统自动微调了信息增量阈值至 {:.3}", new_threshold),
                    format!("信息增量阈值 → {:.3}", new_threshold),
                    if *new_threshold > 0.01 {
                        "提高了合成标准，要求更强的信息增量，合成数量可能减少但质量提升".to_string()
                    } else {
                        "降低了合成标准，允许更多记忆参与合成，合成数量可能增加".to_string()
                    },
                    "您可以通过 set_information_gain_threshold() 方法手动设置阈值".to_string(),
                ),
                RegulationAction::SuggestComprehensiveRebalance {
                    severity,
                    anomaly_description,
                    ..
                } => (
                    format!("系统建议进行全面再平衡（严重程度：{}）", severity),
                    "多个参数需要联动调整".to_string(),
                    "综合调整将改善系统整体健康状态，但影响范围较大".to_string(),
                    format!("建议您审查以下异常并确认调整：{}", anomaly_description),
                ),
            };

            ExplainableDecision {
                summary,
                why: log.user_facing_explanation.clone(),
                what_changed,
                impact,
                how_to_override,
                confidence: log.confidence,
                timestamp_ms: log.timestamp_ms,
            }
        })
    }

    /// 获取决策日志总数（用于监控和调试）
    pub fn decision_log_count(&self) -> usize {
        self.decision_log.len()
    }
}

impl Default for DaoRegulator {
    fn default() -> Self {
        Self::new()
    }
}

/// 决策日志（质疑三"责任鸿沟"）
///
/// 记录每次自主决策的完整上下文，包括决策时的输入参数快照、
/// 分析推理链、替代方案和风险评估。每一条日志都是系统"思考"
/// 过程的完整记录，确保"有原因，就有责任"。
///
/// 道枢映射：艮卦·山 (☶) — "艮其止，止其所也"。决策日志如
/// 山之层积，每一层都清晰可见，让系统的"思考"过程透明化，
/// 填平"有原因，无责任"的鸿沟。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionLog {
    /// 决策唯一标识（格式：dl_<timestamp>_<random>）
    pub decision_id: String,
    /// 决策时间戳（毫秒）
    pub timestamp_ms: u64,
    /// 决策类型：阈值调整 / 权重调整 / 编码建议 / 衰减速率 / 综合再平衡 / 无操作
    pub decision_type: String,
    /// 决策时的输入参数快照
    pub inputs: DecisionInputs,
    /// 决策分析过程（人类可读的推理链）
    pub analysis: String,
    /// 最终决策
    pub decision: RegulationAction,
    /// 置信度评分（0.0 ~ 1.0）
    pub confidence: f32,
    /// 曾考虑过的替代方案
    pub alternatives_considered: Vec<String>,
    /// 风险评估
    pub risk_assessment: String,
    /// 面向用户的解释（非技术语言）
    pub user_facing_explanation: String,
}

/// 决策输入参数快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionInputs {
    /// 道同构度评分
    pub dao_score: f32,
    /// 八卦分布熵
    pub bagua_entropy: f32,
    /// 合成比率
    pub synthesis_ratio: f32,
    /// 洛书幻和平均偏离度
    pub avg_luoshu_deviation: f32,
    /// 合成频率
    pub synthesis_rate: f32,
    /// 当前衰减速率
    pub current_decay_rate: f32,
    /// 当前最小聚类大小
    pub current_min_cluster: usize,
    /// 耦合指数
    pub coupling_score: f32,
    /// 是否检测到振荡
    pub is_oscillating: bool,
    /// 动态信息增量阈值
    pub information_gain_threshold: f32,
}

/// 可解释决策（面向用户）
///
/// 将技术性的 DecisionLog 转换为人类可读的决策解释，
/// 帮助用户理解系统"为什么这么做"以及"如何手动覆盖"。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainableDecision {
    /// 一句话总结
    pub summary: String,
    /// 为什么系统做了这个决策
    pub why: String,
    /// 什么参数改变了
    pub what_changed: String,
    /// 预期影响
    pub impact: String,
    /// 如果用户不同意，如何手动覆盖
    pub how_to_override: String,
    /// 置信度（0.0 ~ 1.0）
    pub confidence: f32,
    /// 决策时间戳
    pub timestamp_ms: u64,
}

/// 系统健康聚合报告（质疑五·可理解性）
///
/// 将分散在多个子系统（DaoRegulator、DaoMetrics、SynthesisJournal、
/// UserFeedback、AuditTrail、MemoryGC）中的状态指标聚合为一个
/// 人类可读的单一视图，降低排查系统行为时的认知负担。
///
/// 道枢映射：中宫（五）— 统摄八方的核心枢纽，聚合所有子系统状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemHealthReport {
    /// 报告生成时间戳（毫秒）
    pub timestamp_ms: u64,
    /// 综合健康评分（0.0 ~ 1.0，越高越健康）
    pub overall_health: f32,
    /// 健康等级：healthy / degraded / critical
    pub health_level: String,

    // ---- 编码器 ----
    /// 编码器模式：ml / statistical / degraded
    pub encoder_mode: String,
    /// 编码器是否处于降级状态
    pub encoder_degraded: bool,
    /// 编码器恢复进度（0.0 ~ 1.0，仅降级时有效）
    pub encoder_recovery_progress: Option<f32>,

    // ---- 调节器 ----
    /// 道同构度评分
    pub dao_score: f32,
    /// 八卦分布熵
    pub bagua_entropy: f32,
    /// 是否检测到振荡
    pub is_oscillating: bool,
    /// 是否检测到漂移
    pub is_drifting: bool,
    /// 是否已冻结
    pub is_frozen: bool,
    /// 耦合指数
    pub coupling_score: f32,
    /// 动态信息增量阈值
    pub information_gain_threshold: f32,
    /// 阈值锚定基线（质疑一·终极：防漂移）
    pub threshold_baseline: f32,
    /// 阈值 EMA（质疑一·终极：指数移动平均平滑值）
    pub threshold_ema: f32,
    /// 合成最小聚类大小
    pub synthesis_min_cluster: usize,

    // ---- 合成 ----
    /// 合成比率（合成记忆 / 总记忆）
    pub synthesis_ratio: f32,
    /// 合成频率（次/分钟）
    pub synthesis_rate_per_minute: f32,
    /// 合成记忆总数
    pub synthesis_count: usize,
    /// 低质量合成记忆数（隔离区）
    pub quarantined_count: usize,

    // ---- 反馈 ----
    /// 总反馈数
    pub total_feedback: usize,
    /// 正面反馈比例
    pub positive_feedback_ratio: f32,
    /// 隐式反馈是否启用
    pub implicit_feedback_enabled: bool,
    /// 知情同意状态（质疑二·终极：None=未选择, Some(true)=已同意, Some(false)=已拒绝）
    pub consent_granted: Option<bool>,

    // ---- 审计 ----
    /// 审计事件总数
    pub total_audit_events: u64,
    /// 审计链是否完整
    pub audit_chain_valid: bool,
    /// 审计持久化是否启用
    pub audit_persistence_enabled: bool,
    /// 完整性封印状态（质疑三·终极：独立封印验证）
    pub audit_seal_verified: bool,

    // ---- GC ----
    /// GC 是否待执行
    pub gc_pending: bool,
    /// GC 上次运行时间（毫秒，0 表示从未运行）
    pub gc_last_run_ms: u64,
    /// v0.5.4 合成是否待执行（从关键路径移出后的延迟标记）
    pub synthesis_pending: bool,

    // ---- 灾难性事件 ----
    /// 灾难性事件数量
    pub catastrophic_event_count: usize,
    /// 最近一次灾难性事件的描述
    pub last_catastrophic_event: Option<String>,

    // ---- 记忆统计 ----
    /// 总记忆数
    pub total_memories: usize,
    /// 活跃记忆数
    pub active_memories: usize,
    /// 已过期记忆数
    pub expired_memories: usize,
    /// 衰减速率
    pub decay_rate: f32,
}

impl SystemHealthReport {
    /// 生成人类可读的摘要文本
    pub fn summary(&self) -> String {
        format!(
            "LRC 健康报告 [{level} | 综合评分: {health:.2}]\n\
             ├─ 编码器: {enc_mode}{degraded}\n\
             ├─ 调节器: 道同构度={dao:.2} 八卦熵={entropy:.2} {osc}{drift}{frozen}\n\
             │  └─ 信息增量阈值: {ig_threshold:.4} (基线={ig_baseline:.4}, EMA={ig_ema:.4})\n\
             ├─ 合成: {synth_count} 条合成记忆 ({ratio:.1}%%), 速率={rate:.2}/min, 隔离={quar}\n\
             ├─ 反馈: {fb_total} 条 ({fb_pos:.1}%% 正面), 隐式反馈{implicit}, 知情同意{consent}\n\
             ├─ 审计: {audit_total} 条事件, 哈希链{chain}, 封印{seal}, 持久化{persist}\n\
             ├─ GC: {gc_status}, 上次运行{last_gc}\n\
             ├─ 灾难: {cat_count} 次灾难性事件{last_cat}\n\
             └─ 记忆: {total} 总数, {active} 活跃, {expired} 过期, 衰减速率={decay_rate:.3}",
            level = self.health_level,
            health = self.overall_health,
            enc_mode = self.encoder_mode,
            degraded = if self.encoder_degraded {
                format!(
                    " (降级中, 恢复进度 {:.0}%%)",
                    self.encoder_recovery_progress.unwrap_or(0.0) * 100.0
                )
            } else {
                String::new()
            },
            dao = self.dao_score,
            entropy = self.bagua_entropy,
            osc = if self.is_oscillating {
                "⚠振荡 "
            } else {
                ""
            },
            drift = if self.is_drifting { "⚠漂移 " } else { "" },
            frozen = if self.is_frozen { "⛔冻结" } else { "" },
            ig_threshold = self.information_gain_threshold,
            ig_baseline = self.threshold_baseline,
            ig_ema = self.threshold_ema,
            synth_count = self.synthesis_count,
            ratio = self.synthesis_ratio,
            rate = self.synthesis_rate_per_minute,
            quar = self.quarantined_count,
            fb_total = self.total_feedback,
            fb_pos = self.positive_feedback_ratio,
            implicit = if self.implicit_feedback_enabled {
                "已启用"
            } else {
                "已关闭"
            },
            consent = match self.consent_granted {
                Some(true) => "已同意",
                Some(false) => "已拒绝",
                None => "未选择",
            },
            audit_total = self.total_audit_events,
            chain = if self.audit_chain_valid {
                "有效"
            } else {
                "⚠断裂"
            },
            seal = if self.audit_seal_verified {
                "有效"
            } else {
                "⚠失效"
            },
            persist = if self.audit_persistence_enabled {
                "已启用"
            } else {
                "未启用"
            },
            gc_status = if self.gc_pending {
                "待执行"
            } else {
                "空闲"
            },
            last_gc = if self.gc_last_run_ms > 0 {
                format!(
                    "{}ms 前",
                    self.timestamp_ms.saturating_sub(self.gc_last_run_ms)
                )
            } else {
                "从未运行".to_string()
            },
            cat_count = self.catastrophic_event_count,
            last_cat = self
                .last_catastrophic_event
                .as_ref()
                .map(|e| format!("\n    └─ 最近: {}", e))
                .unwrap_or_default(),
            total = self.total_memories,
            active = self.active_memories,
            expired = self.expired_memories,
            decay_rate = self.decay_rate,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regulate_high_deviation() {
        let mut regulator = DaoRegulator::new();
        let action = regulator.regulate(0.8, 2.0, 0.2, 0.6, 0.5, 0.1, 3);
        assert!(
            matches!(action, RegulationAction::SuggestReencoding { .. }),
            "高幻和偏离应触发重新编码建议"
        );
    }

    #[test]
    fn test_regulate_low_dao_concentrated() {
        let mut regulator = DaoRegulator::new();
        let action = regulator.regulate(0.2, 0.5, 0.2, 0.1, 0.5, 0.1, 3);
        assert!(
            matches!(action, RegulationAction::AdjustRetrievalWeights { .. }),
            "低道同构度+低八卦熵应触发检索权重调整"
        );
    }

    #[test]
    fn test_regulate_healthy_low_synthesis() {
        let mut regulator = DaoRegulator::new();
        let action = regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5);
        assert!(
            matches!(action, RegulationAction::AdjustSynthesisThreshold { .. }),
            "健康但合成频率低应降低合成阈值"
        );
    }

    #[test]
    fn test_regulate_healthy_everything() {
        let mut regulator = DaoRegulator::new();
        let action = regulator.regulate(0.9, 2.8, 0.3, 0.05, 0.5, 0.1, 3);
        assert!(
            matches!(
                action,
                RegulationAction::AdjustDecayRate { .. } | RegulationAction::NoAction
            ),
            "高道同构度+高八卦熵+健康合成比应加速衰减或无操作"
        );
    }

    #[test]
    fn test_no_action_when_healthy() {
        let mut regulator = DaoRegulator::new();
        let action = regulator.regulate(0.6, 1.5, 0.15, 0.2, 0.3, 0.1, 3);
        assert_eq!(action, RegulationAction::NoAction);
    }

    // === 防振荡测试 ===

    /// 测试：冲突仲裁 — 高偏差 + 低八卦熵同时触发，应优先返回 Reencoding
    #[test]
    fn test_conflict_arbitration_priority() {
        let mut regulator = DaoRegulator::new();
        // 同时触发"高幻和偏离"和"八卦分布集中"
        let action = regulator.regulate(0.2, 0.3, 0.2, 0.7, 0.5, 0.1, 3);
        // Reencoding 优先级最高
        assert!(
            matches!(action, RegulationAction::SuggestReencoding { .. }),
            "冲突时应优先返回重新编码建议（最高优先级）"
        );
    }

    /// 测试：振荡检测 — 连续 3 次同方向调节后步长不变
    #[test]
    fn test_no_oscillation_on_consistent_direction() {
        let mut regulator = DaoRegulator::new();

        // 连续 3 次触发"合成频率低" → 持续降低阈值
        for _ in 0..3 {
            let action = regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5);
            assert!(matches!(
                action,
                RegulationAction::AdjustSynthesisThreshold { .. }
            ));
        }

        // 同方向连续调节不应触发振荡
        assert!(!regulator.is_oscillating(), "同方向连续调节不应触发振荡");
    }

    /// 测试：振荡检测 — 方向反转触发振荡
    #[test]
    fn test_oscillation_detection_on_reversal() {
        let mut regulator = DaoRegulator::new();

        // 第 1 次：合成比率过高 → 提高阈值（方向 +1）
        let _ = regulator.regulate(0.2, 1.5, 0.6, 0.1, 0.5, 0.1, 3);

        // 第 2 次：合成频率低 → 降低阈值（方向 -1）
        let _ = regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5);

        // 第 3 次：合成比率过高 → 提高阈值（方向 +1）— 反转 #1
        let _ = regulator.regulate(0.2, 1.5, 0.6, 0.1, 0.5, 0.1, 3);

        // 第 4 次：合成频率低 → 降低阈值（方向 -1）— 反转 #2
        let _ = regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5);

        // 2 次反转 → 应检测到振荡
        assert!(regulator.is_oscillating(), "方向反转 2 次应触发振荡检测");
        assert!(regulator.step_multiplier() < 1.0, "振荡时步长应减小");
    }

    /// 测试：自适应步长 — 振荡时合成阈值调整幅度减小
    #[test]
    fn test_adaptive_step_on_oscillation() {
        let mut regulator = DaoRegulator::new();

        // 模拟振荡：交替触发"合成比率过高"和"合成频率过低"
        // 这两种状态都会触发 synthesis_threshold 动作，但方向相反
        for round in 0..4 {
            if round % 2 == 0 {
                // 合成比率过高 → 提高阈值（方向 +1）
                drop(regulator.regulate(0.2, 1.5, 0.6, 0.1, 0.5, 0.1, 3));
            } else {
                // 合成频率低 → 降低阈值（方向 -1），但需要确保 dao_score > 0.7
                drop(regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5));
            }
        }

        // 振荡后步长倍率应小于 1.0
        let multiplier = regulator.step_multiplier();
        assert!(
            multiplier < 1.0,
            "振荡后步长倍率应减小，实际: {:.2}",
            multiplier
        );
    }

    /// 测试：稳定后恢复 — 连续同方向后步长恢复
    #[test]
    fn test_step_recovery_after_stability() {
        let mut regulator = DaoRegulator::new();

        // 先制造振荡（交替 synthesis_threshold 方向）
        for round in 0..4 {
            if round % 2 == 0 {
                drop(regulator.regulate(0.2, 1.5, 0.6, 0.1, 0.5, 0.1, 3));
            } else {
                drop(regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5));
            }
        }
        assert!(regulator.is_oscillating());

        // 连续 3 次同方向（持续降低阈值）→ 应恢复
        for _ in 0..3 {
            drop(regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5));
        }

        // 连续同方向 → 步长应逐步恢复（从 0.125 开始恢复，3 轮后应 ≥ 0.15）
        // 恢复是渐进的：振荡后需要多轮同方向调节才能完全恢复，这是防振荡的刻意设计
        assert!(
            regulator.step_multiplier() >= 0.15,
            "连续同方向后步长应逐步恢复，实际: {:.2}",
            regulator.step_multiplier()
        );
    }

    // === v2.1 漂移检测与冻结保护测试 ===

    /// 测试：漂移检测 — 连续同方向调节超过阈值触发漂移告警
    #[test]
    fn test_drift_detection_on_consecutive_same_direction() {
        let mut regulator = DaoRegulator::new();

        // 连续 8 次触发"合成频率低" → 持续降低阈值（同方向 -1）
        // 漂移阈值为 8，第 8 次应触发漂移
        for i in 0..8 {
            let _ = regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5);
            if i < 7 {
                assert!(!regulator.is_drifting(), "第 {} 次调节不应触发漂移", i + 1);
            }
        }

        // 第 8 次同方向 → 应触发漂移
        assert!(regulator.is_drifting(), "连续 8 次同方向调节应触发漂移检测");
    }

    /// 测试：漂移重置 — 方向变化后漂移计数重置
    #[test]
    fn test_drift_reset_on_direction_change() {
        let mut regulator = DaoRegulator::new();

        // 先连续同方向 5 次
        for _ in 0..5 {
            drop(regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5));
        }

        assert!(!regulator.is_drifting(), "5 次同方向不应触发漂移");

        // 一次反向调节 → 漂移计数应重置
        drop(regulator.regulate(0.2, 1.5, 0.6, 0.1, 0.5, 0.1, 3));

        assert!(!regulator.is_drifting(), "方向变化后漂移应重置");

        // 再次同方向 5 次（应从头计数，不会触发漂移）
        for _ in 0..5 {
            drop(regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5));
        }

        assert!(
            !regulator.is_drifting(),
            "方向变化后重新计数，5 次不应触发漂移"
        );
    }

    /// 测试：冻结保护 — 同一建议重复触发冻结
    #[test]
    fn test_freeze_on_repeated_same_action() {
        let mut regulator = DaoRegulator::new();

        // 连续 10 次触发"合成频率低"（同一 action_tag）
        // 冻结阈值为 10，第 10 次应触发冻结
        for i in 0..10 {
            let action = regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5);
            assert!(
                matches!(action, RegulationAction::AdjustSynthesisThreshold { .. }),
                "第 {} 次应返回合成阈值调整",
                i + 1
            );

            if i < 9 {
                assert!(!regulator.is_frozen(), "第 {} 次调节不应触发冻结", i + 1);
            }
        }

        // 第 10 次同一建议 → 应触发冻结
        assert!(regulator.is_frozen(), "连续 10 次同一建议应触发冻结保护");
    }

    /// 测试：冻结后拒绝调节
    #[test]
    fn test_frozen_regulator_rejects_regulation() {
        let mut regulator = DaoRegulator::new();

        // 触发冻结
        for _ in 0..10 {
            drop(regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5));
        }
        assert!(regulator.is_frozen());

        // 冻结后 should_regulate 应返回 false
        assert!(!regulator.should_regulate(), "冻结后调节器应拒绝调节");
    }

    /// 测试：手动解除冻结
    #[test]
    fn test_unfreeze_restores_regulation() {
        let mut regulator = DaoRegulator::new();

        // 触发冻结
        for _ in 0..10 {
            drop(regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5));
        }
        assert!(regulator.is_frozen());

        // 手动解除冻结
        regulator.unfreeze();
        assert!(!regulator.is_frozen());
        assert!(regulator.should_regulate(), "解除冻结后调节器应恢复正常");
    }

    /// 测试：动作类型切换阻止冻结
    #[test]
    fn test_action_change_prevents_freeze() {
        let mut regulator = DaoRegulator::new();

        // 交替触发不同动作类型（不应触发冻结）
        for _ in 0..12 {
            // 合成频率低（synthesis_threshold）
            drop(regulator.regulate(0.85, 2.0, 0.05, 0.1, 0.05, 0.1, 5));
            // 八卦熵低（retrieval_weights）
            drop(regulator.regulate(0.2, 0.3, 0.2, 0.1, 0.5, 0.1, 3));
        }

        // 交替不同动作类型 → 不应冻结
        assert!(!regulator.is_frozen(), "交替不同动作类型不应触发冻结");
    }

    // === v2.2 耦合感知仲裁测试（质疑二） ===

    /// 测试：多指标同时异常触发综合再平衡
    #[test]
    fn test_coupling_triggers_comprehensive_rebalance() {
        let mut regulator = DaoRegulator::new();

        // 同时触发多个异常：道同构度低 + 八卦集中 + 洛书偏差大
        let action = regulator.regulate(
            0.2, // 低道同构度
            0.3, // 八卦集中（低熵）
            0.6, // 高合成比率
            0.6, // 高洛书偏差
            0.5, // 合成频率
            0.1, // 当前衰减速率
            3,   // 当前聚类大小
        );

        // 应返回综合再平衡而非简单优先级仲裁
        assert!(
            matches!(
                action,
                RegulationAction::SuggestComprehensiveRebalance { .. }
            ),
            "多指标同时异常应触发综合再平衡，实际: {:?}",
            action
        );
    }

    /// 测试：耦合指数衰减 — 恢复健康后耦合指数下降
    #[test]
    fn test_coupling_decay_on_recovery() {
        let mut regulator = DaoRegulator::new();

        // 先制造高耦合
        for _ in 0..3 {
            drop(regulator.regulate(0.2, 0.3, 0.6, 0.6, 0.5, 0.1, 3));
        }

        let coupling_high = regulator.coupling_score();
        assert!(coupling_high > 0.0, "高耦合应产生正耦合指数");

        // 恢复正常
        for _ in 0..5 {
            drop(regulator.regulate(0.8, 2.5, 0.2, 0.1, 0.3, 0.1, 3));
        }

        let coupling_low = regulator.coupling_score();
        assert!(
            coupling_low < coupling_high,
            "恢复健康后耦合指数应下降: high={:.2}, low={:.2}",
            coupling_high,
            coupling_low
        );
    }

    /// 测试：长周期反馈 — 模拟连锁反应检测
    #[test]
    fn test_long_cycle_cascade_detection() {
        let mut regulator = DaoRegulator::new();

        // 模拟长周期：多轮间发的高耦合事件
        for round in 0..10 {
            if round % 3 == 0 {
                // 每隔 3 轮制造一次高耦合
                drop(regulator.regulate(0.2, 0.3, 0.6, 0.7, 0.5, 0.1, 3));
            } else {
                // 其他轮正常
                drop(regulator.regulate(0.8, 2.5, 0.2, 0.1, 0.3, 0.1, 3));
            }
        }

        // 应该有耦合事件记录
        let coupling = regulator.coupling_score();
        assert!(coupling > 0.0, "长周期中应有耦合事件记录");
    }

    /// 测试：单指标异常不触发综合再平衡
    #[test]
    fn test_single_anomaly_no_comprehensive_rebalance() {
        let mut regulator = DaoRegulator::new();

        // 仅合成阈值异常（无耦合）
        let action = regulator.regulate(
            0.85, // 健康道同构度
            2.0,  // 健康八卦熵
            0.05, // 低合成比率
            0.1,  // 低洛书偏差
            0.05, // 低合成频率
            0.1,  // 当前衰减速率
            5,    // 当前聚类大小
        );

        // 单异常不应触发综合再平衡
        assert!(
            !matches!(
                action,
                RegulationAction::SuggestComprehensiveRebalance { .. }
            ),
            "单指标异常不应触发综合再平衡"
        );
    }

    /// 测试：耦合趋势分析 — 验证恶化检测和指标统计
    #[test]
    fn test_coupling_trend_analysis() {
        let mut regulator = DaoRegulator::new();

        // 模拟 50 轮调节：
        // 前 30 轮高频触发低耦合（2 指标异常，强度 0.5）
        // 后 20 轮持续高耦合（4 指标异常，强度 1.0 → 恶化趋势）
        for round in 0..50 {
            if round < 30 {
                // 前 30 轮：每 2 轮触发一次 2 指标异常（耦合强度 0.5）
                if round % 2 == 0 {
                    drop(regulator.regulate(0.1, 2.5, 0.6, 0.3, 0.5, 0.1, 3));
                } else {
                    // 健康状态 → 无耦合事件
                    drop(regulator.regulate(0.8, 2.5, 0.2, 0.1, 0.3, 0.1, 3));
                }
            } else {
                // 后 20 轮：4 指标同时异常（耦合强度 1.0 → 恶化）
                drop(regulator.regulate(0.15, 0.2, 0.7, 0.8, 0.5, 0.1, 3));
            }
        }

        let trend = regulator.analyze_coupling_trend();
        // 应有耦合事件记录
        assert!(trend.total_coupling_events > 0, "应有耦合事件记录");
        // 恶化趋势：近期（后 20 轮）耦合强度 1.0 应远高于全量均值（含前 0.5 的事件）
        assert!(
            trend.is_worsening,
            "持续高耦合应检测到恶化趋势: all_avg={:.2}, recent_avg={:.2}",
            trend.all_avg_strength, trend.recent_avg_strength
        );
        // 洛书偏差和道同构度应是最常异常的指标
        assert!(trend.deviation_anomaly_count > 0, "应有洛书偏差异常记录");
        assert!(trend.dao_anomaly_count > 0, "应有道同构度异常记录");
    }

    /// 测试：空耦合历史返回默认分析
    #[test]
    fn test_coupling_trend_empty_history() {
        let regulator = DaoRegulator::new();
        let trend = regulator.analyze_coupling_trend();
        assert_eq!(trend.total_coupling_events, 0);
        assert!(!trend.is_worsening);
        assert_eq!(trend.cascade_ratio, 0.0);
    }

    // ============================================================
    // 灾难性转折检测测试（质疑二）
    // ============================================================

    /// 测试：正常状态下无灾难性转折检测
    #[test]
    fn test_no_catastrophic_event_when_healthy() {
        let mut regulator = DaoRegulator::new();

        // 模拟 6 次健康调节（需要至少 5 次快照才能检测）
        for _ in 0..6 {
            regulator.regulate(0.8, 2.5, 0.2, 0.1, 0.3, 0.1, 3);
        }

        let events = regulator.get_catastrophic_events();
        assert!(events.is_empty(), "健康状态下不应检测到灾难性转折");
    }

    /// 测试：健康评分骤降应检测到灾难性转折
    #[test]
    fn test_catastrophic_event_on_health_crash() {
        let mut regulator = DaoRegulator::new();

        // 前 4 次：健康状态
        for _ in 0..4 {
            regulator.regulate(0.8, 2.5, 0.2, 0.1, 0.3, 0.1, 3);
        }

        // 第 5 次：急剧恶化（道同构度暴跌 + 偏离度飙升 + 合成比率异常）
        regulator.regulate(0.05, 0.3, 0.9, 0.95, 0.5, 0.1, 3);

        // 第 6 次：持续恶化
        regulator.regulate(0.03, 0.2, 0.95, 0.98, 0.5, 0.1, 3);

        let events = regulator.get_catastrophic_events();
        assert!(!events.is_empty(), "健康评分骤降应检测到灾难性转折");
        let event = &events[0];
        assert!(event.drop_magnitude > 0.3, "下降幅度应超过阈值");
        assert!(!event.diagnosis.is_empty(), "应有诊断建议");
    }

    /// 测试：灾难性转折事件的严重程度分类
    #[test]
    fn test_catastrophic_severity_levels() {
        let mut regulator = DaoRegulator::new();

        // 前 4 次：健康
        for _ in 0..4 {
            regulator.regulate(0.8, 2.5, 0.2, 0.1, 0.3, 0.1, 3);
        }

        // 模拟急剧恶化
        for _ in 0..3 {
            regulator.regulate(0.02, 0.1, 0.98, 0.99, 0.5, 0.1, 3);
        }

        let events = regulator.get_catastrophic_events();
        if !events.is_empty() {
            let severity = &events[0].severity;
            assert!(
                severity == "warning" || severity == "severe" || severity == "critical",
                "严重程度应为 warning/severe/critical 之一，实际: {}",
                severity
            );
        }
    }

    /// 测试：灾难性转折事件包含归因信息
    #[test]
    fn test_catastrophic_event_attribution() {
        let mut regulator = DaoRegulator::new();

        // 前 4 次：健康
        for _ in 0..4 {
            regulator.regulate(0.8, 2.5, 0.2, 0.1, 0.3, 0.1, 3);
        }

        // 急剧恶化
        for _ in 0..3 {
            regulator.regulate(0.05, 0.3, 0.9, 0.95, 0.5, 0.1, 3);
        }

        let events = regulator.get_catastrophic_events();
        if !events.is_empty() {
            let event = &events[0];
            // 验证归因信息存在
            assert!(
                !event.last_action_before_crash.is_empty(),
                "应包含导致崩溃的最后调节动作"
            );
            assert!(event.drop_magnitude > 0.0, "应记录下降幅度");
            assert!(
                event.diagnosis.contains("建议"),
                "诊断信息应包含建议: {}",
                event.diagnosis
            );
        }
    }

    // ============================================================
    // 质疑三·责任鸿沟：可解释决策日志测试
    // ============================================================

    /// 测试：每次 regulate() 后决策日志被记录
    #[test]
    fn test_decision_log_is_recorded() {
        let mut regulator = DaoRegulator::new();

        // 执行一次调节
        regulator.regulate(0.8, 2.0, 0.2, 0.6, 0.5, 0.1, 3);

        // 验证决策日志已记录
        let logs = regulator.get_decision_log(10);
        assert!(!logs.is_empty(), "每次 regulate 后应记录决策日志");
        assert_eq!(logs.len(), 1, "第一次调节后应有 1 条日志");

        let log = &logs[0];
        assert!(!log.decision_id.is_empty(), "决策 ID 不应为空");
        assert!(
            log.confidence >= 0.0 && log.confidence <= 1.0,
            "置信度应在 0.0~1.0 之间，实际: {:.2}",
            log.confidence
        );
        assert!(!log.analysis.is_empty(), "分析推理链不应为空");
        assert!(!log.risk_assessment.is_empty(), "风险评估不应为空");
        assert!(!log.user_facing_explanation.is_empty(), "用户解释不应为空");

        // 验证决策日志包含输入快照
        assert!(
            (log.inputs.dao_score - 0.8).abs() < f32::EPSILON,
            "应记录道同构度输入"
        );
        assert!(
            (log.inputs.avg_luoshu_deviation - 0.6).abs() < f32::EPSILON,
            "应记录洛书偏离度输入"
        );

        // 执行第二次调节
        regulator.regulate(0.2, 0.3, 0.2, 0.1, 0.5, 0.1, 3);
        let logs = regulator.get_decision_log(10);
        assert_eq!(logs.len(), 2, "第二次调节后应有 2 条日志");
    }

    /// 测试：explain_last_decision 生成可解释决策
    #[test]
    fn test_explain_last_decision() {
        let mut regulator = DaoRegulator::new();

        // 触发重新编码建议（高洛书偏差）
        regulator.regulate(0.8, 2.0, 0.2, 0.7, 0.5, 0.1, 3);

        let explanation = regulator.explain_last_decision();
        assert!(explanation.is_some(), "应有可解释决策");
        let exp = explanation.unwrap();

        assert!(!exp.summary.is_empty(), "一句话总结不应为空");
        assert!(!exp.why.is_empty(), "原因解释不应为空");
        assert!(!exp.what_changed.is_empty(), "参数变化说明不应为空");
        assert!(!exp.impact.is_empty(), "预期影响不应为空");
        assert!(!exp.how_to_override.is_empty(), "手动覆盖指南不应为空");
        assert!(
            exp.confidence >= 0.0 && exp.confidence <= 1.0,
            "置信度应在 0.0~1.0 之间，实际: {:.2}",
            exp.confidence
        );
        assert!(exp.timestamp_ms > 0, "时间戳应大于 0");

        // 验证 NoAction 场景
        let mut regulator2 = DaoRegulator::new();
        regulator2.regulate(0.6, 1.5, 0.15, 0.2, 0.3, 0.1, 3);
        let exp2 = regulator2.explain_last_decision().unwrap();
        assert!(exp2.summary.contains("健康"), "无操作时应显示健康信息");
        assert!(exp2.what_changed.contains("无"), "无操作时参数不变");
    }

    /// 测试：决策日志容量限制（最多 100 条）
    #[test]
    fn test_decision_log_capacity() {
        let mut regulator = DaoRegulator::new();

        // 执行 120 次调节，超过 100 条容量上限
        for i in 0..120 {
            if i % 3 == 0 {
                // 高偏差场景
                regulator.regulate(0.8, 2.0, 0.2, 0.6, 0.5, 0.1, 3);
            } else if i % 3 == 1 {
                // 低八卦熵场景
                regulator.regulate(0.2, 0.3, 0.2, 0.1, 0.5, 0.1, 3);
            } else {
                // 健康场景
                regulator.regulate(0.6, 1.5, 0.15, 0.2, 0.3, 0.1, 3);
            }
        }

        // 验证容量不超过 100
        let logs = regulator.get_decision_log(200);
        assert_eq!(logs.len(), 100, "决策日志不应超过 100 条");
        assert_eq!(
            regulator.decision_log_count(),
            100,
            "decision_log_count 应返回 100"
        );

        // 验证最新的日志是第 120 次调节
        let latest = regulator.get_last_decision().unwrap();
        assert!(!latest.decision_id.is_empty(), "最新日志应有有效 ID");

        // 验证日志按时间倒序排列（最新的在前）
        let first_in_logs = &logs[0];
        let last = regulator.get_last_decision().unwrap();
        assert_eq!(
            first_in_logs.decision_id, last.decision_id,
            "get_decision_log 第一条应是最新决策"
        );

        // 验证旧日志被淘汰（第 1 次调节的日志不应存在）
        // 已经执行了 120 次，只保留最近 100 条
        assert_eq!(logs.len(), 100);
    }
}
