// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件实现自主记忆垃圾回收器，属于守护层 (Layer 2)。
// ============================================================
//
// 自主记忆垃圾回收器 (MemoryGarbageCollector)
//
// 解决质疑三"记忆垃圾回收机制缺失"问题：
// 设计一套"记忆垃圾回收"机制，作为一个低优先级的后台任务，
// 根据质量评分、被访问频率、关联记忆的存活状态等指标，
// 自动识别并清理那些不再被系统活跃使用的记忆。
//
// 质疑一核心修复：使用 MemorySnapshot 数据模式替代 trait object 传参，
// 消除调用方 std::mem::replace 的临时借用绕道，使 GC 状态管理更为清晰。

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// 垃圾回收配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcConfig {
    /// 是否启用自动回收
    pub enabled: bool,
    /// 回收间隔（秒）
    pub interval_secs: u64,
    /// 低质量分数阈值（低于此分数的记忆被标记为候选）
    pub quality_threshold: f32,
    /// 未访问天数阈值（超过此天数未访问的记忆被标记为候选）
    pub stale_days: u64,
    /// 观察期（秒）：候选记忆在观察期后才能被删除
    pub observation_period_secs: u64,
    /// 单次回收最大数量（防止一次性删除过多）
    pub max_per_cycle: usize,
    /// 是否允许回收核心节点
    pub allow_core_node_removal: bool,
    /// 最低重要性保护（低于此重要性的记忆才可被回收）
    pub max_importance_for_removal: u8,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 3600,            // 每小时检查一次
            quality_threshold: 0.2,         // 质量评分低于 0.2 的候选
            stale_days: 30,                 // 30 天未访问的候选
            observation_period_secs: 86400, // 24 小时观察期
            max_per_cycle: 50,              // 每次最多回收 50 条
            allow_core_node_removal: false, // 不允许回收核心节点
            max_importance_for_removal: 3,  // 重要性 <= 3 才可回收
        }
    }
}

/// 垃圾回收候选记忆
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcCandidate {
    /// 记忆 ID
    pub memory_id: String,
    /// 综合垃圾评分（0.0 ~ 1.0，越高越应该被回收）
    pub garbage_score: f32,
    /// 质量评分（来自 SynthesisJournal）
    pub quality_score: f32,
    /// 上次访问距今的天数
    pub days_since_access: f64,
    /// 被引用的次数（图边数量）
    pub reference_count: usize,
    /// 是否为合成记忆
    pub is_synthesis: bool,
    /// 是否为低质量合成（被 SynthesisJournal 标记）
    pub is_low_quality: bool,
    /// 用户负面反馈数
    pub negative_feedback_count: usize,
    /// 标记为候选的时间戳
    pub marked_at_ms: u64,
    /// 当前状态：observed / pending_delete
    pub status: String,
}

/// 垃圾回收统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcStats {
    /// 总回收次数
    pub total_cycles: usize,
    /// 总回收记忆数
    pub total_removed: usize,
    /// 当前观察中的候选数
    pub observing_count: usize,
    /// 上次回收时间戳
    pub last_gc_ms: u64,
    /// 上次回收删除数
    pub last_removed_count: usize,
    /// 累计节省的内存估算（条数）
    pub total_freed: usize,
}

/// 记忆快照 — GC 查询的纯数据载体
///
/// 质疑一核心设计：将 GC 所需的记忆信息从 trait object 查询
/// 转换为一次性快照。调用方在 GC 前收集所有快照，GC 仅基于快照
/// 进行计算，不再需要 &mut dyn MemoryInfoQuery 传递。
///
/// 这消除了 `std::mem::replace` 的绕道借用，让 GC 的状态管理
/// 回归 Rust 的所有权模型自然表达。
#[derive(Debug, Clone)]
pub struct MemorySnapshot {
    pub memory_id: String,
    pub last_accessed_ms: Option<u64>,
    pub importance: Option<u8>,
    pub memory_type: Option<String>,
    pub quality_score: f32,
    pub reference_count: usize,
    pub is_core_node: bool,
    pub is_low_quality: bool,
    pub negative_feedback_count: usize,
}

impl MemorySnapshot {
    /// 从查询器构建单个记忆快照
    pub fn from_query(query: &dyn MemoryInfoQuery, memory_id: &str) -> Self {
        Self {
            memory_id: memory_id.to_string(),
            last_accessed_ms: query.get_last_accessed_ms(memory_id),
            importance: query.get_importance(memory_id),
            memory_type: query.get_memory_type(memory_id),
            quality_score: query.get_quality_score(memory_id),
            reference_count: query.get_reference_count(memory_id),
            is_core_node: query.is_core_synthesis_node(memory_id),
            is_low_quality: query.is_low_quality_synthesis(memory_id),
            negative_feedback_count: query.get_negative_feedback_count(memory_id),
        }
    }

    /// 从查询器收集所有记忆的快照
    ///
    /// 注意：性能计时已迁移至 MemoryGarbageCollector::collect_snapshots_with_timing，
    /// 该方法使用动态基线替代固定阈值（质疑三）。
    pub fn collect_all(query: &dyn MemoryInfoQuery) -> Vec<Self> {
        let all_ids = query.get_all_memory_ids();

        all_ids
            .iter()
            .map(|id| Self::from_query(query, id))
            .collect()
    }
}

/// 记忆信息查询器 trait
///
/// 保留用于监控面板的单条评估和外部诊断，主 GC 流程已迁移至快照模式。
pub trait MemoryInfoQuery {
    /// 获取记忆的上次访问时间戳（毫秒）
    fn get_last_accessed_ms(&self, memory_id: &str) -> Option<u64>;
    /// 获取记忆的重要性（1-10）
    fn get_importance(&self, memory_id: &str) -> Option<u8>;
    /// 获取记忆的类型
    fn get_memory_type(&self, memory_id: &str) -> Option<String>;
    /// 获取记忆的引用计数（图边数）
    fn get_reference_count(&self, memory_id: &str) -> usize;
    /// 检查记忆是否为核心合成节点
    fn is_core_synthesis_node(&self, memory_id: &str) -> bool;
    /// 检查记忆是否被 SynthesisJournal 标记为低质量
    fn is_low_quality_synthesis(&self, memory_id: &str) -> bool;
    /// 获取记忆的质量评分（0.0 ~ 1.0）
    fn get_quality_score(&self, memory_id: &str) -> f32;
    /// 获取用户负面反馈数
    fn get_negative_feedback_count(&self, memory_id: &str) -> usize;
    /// 获取所有记忆 ID 列表
    fn get_all_memory_ids(&self) -> Vec<String>;
    /// 删除指定记忆（调用方负责执行）
    fn delete_memory(&mut self, memory_id: &str) -> bool;
}

/// 垃圾回收器内部状态
#[derive(Debug)]
struct GcState {
    config: GcConfig,
    stats: GcStats,
    /// 当前候选列表（观察中 + 待删除）
    candidates: Vec<GcCandidate>,
    /// 上次 GC 运行时间戳
    last_gc_ms: u64,
}

/// 自主记忆垃圾回收器
///
/// 定期扫描记忆库，识别低质量、长期未使用、孤立的记忆，
/// 经过观察期后自动清理，防止记忆垃圾堆积。
#[derive(Debug)]
pub struct MemoryGarbageCollector {
    state: GcState,
    /// 性能计时基线（质疑三：动态警告阈值）
    timing_baseline: GcTimingBaseline,
}

/// GC 性能计时基线（质疑三：防止固定阈值产生噪音）
///
/// 记录最近 N 次快照收集的耗时，建立动态基线。
/// 仅当单次耗时显著偏离基线（> 均值 + 3σ）时才触发警告，
/// 避免在记忆库自然增长过程中产生大量无效警告。
#[derive(Debug)]
struct GcTimingBaseline {
    /// 最近 N 次快照耗时（毫秒），FIFO
    samples: Vec<f64>,
    /// 最大样本数
    max_samples: usize,
    /// 当前均值
    mean: f64,
    /// 当前标准差
    stddev: f64,
}

impl GcTimingBaseline {
    fn new() -> Self {
        Self {
            samples: Vec::with_capacity(20),
            max_samples: 20,
            mean: 0.0,
            stddev: 0.0,
        }
    }

    /// 添加新样本，判断是否需要警告
    ///
    /// 返回 (should_warn, mean, stddev, deviation_factor)
    /// deviation_factor = (current - mean) / stddev，> 3.0 时触发警告
    fn record(&mut self, elapsed_ms: f64) -> (bool, f64, f64, f64) {
        self.samples.push(elapsed_ms);
        if self.samples.len() > self.max_samples {
            self.samples.remove(0);
        }

        // 需要足够样本才能建立基线
        if self.samples.len() < 5 {
            return (false, 0.0, 0.0, 0.0);
        }

        // 计算均值
        let n = self.samples.len() as f64;
        self.mean = self.samples.iter().sum::<f64>() / n;

        // 计算标准差
        let variance = self
            .samples
            .iter()
            .map(|s| (s - self.mean).powi(2))
            .sum::<f64>()
            / n;
        self.stddev = variance.sqrt();

        // 计算偏离因子
        let deviation = if self.stddev > 0.0 {
            (elapsed_ms - self.mean) / self.stddev
        } else {
            0.0
        };

        // 偏离超过 3 个标准差触发警告
        let should_warn = deviation > 3.0 && self.mean > 0.0;

        (should_warn, self.mean, self.stddev, deviation)
    }
}

impl MemoryGarbageCollector {
    /// 创建新的垃圾回收器
    pub fn new(config: GcConfig) -> Self {
        Self {
            state: GcState {
                config,
                stats: GcStats {
                    total_cycles: 0,
                    total_removed: 0,
                    observing_count: 0,
                    last_gc_ms: 0,
                    last_removed_count: 0,
                    total_freed: 0,
                },
                candidates: Vec::new(),
                last_gc_ms: 0,
            },
            timing_baseline: GcTimingBaseline::new(),
        }
    }

    /// 执行一次垃圾回收周期（快照模式）
    ///
    /// 质疑一核心方法：接受预收集的 MemorySnapshot 列表，
    /// 道枢映射: 兑卦·泽 (☱) — 说万物者莫说乎泽，GC如泽水之自然净化，回收过期记忆维持生态平衡
    ///
    /// 不再需要 &mut dyn MemoryInfoQuery，从而消除调用方的借用冲突。
    ///
    /// 返回 (统计信息, 待删除的记忆 ID 列表)。
    /// 调用方负责执行实际的删除操作。
    pub fn collect_garbage(&mut self, snapshots: &[MemorySnapshot]) -> (GcStats, Vec<String>) {
        let now = now_ms();

        if !self.state.config.enabled {
            return (self.state.stats.clone(), Vec::new());
        }

        // 检查是否到了回收间隔
        if self.state.last_gc_ms > 0
            && now - self.state.last_gc_ms < self.state.config.interval_secs * 1000
        {
            return (self.state.stats.clone(), Vec::new());
        }

        self.state.last_gc_ms = now;
        self.state.stats.total_cycles += 1;

        // ============================================================
        // 阶段一：标记候选
        // ============================================================
        let mut new_candidates: Vec<GcCandidate> = Vec::new();

        for snapshot in snapshots {
            let garbage_score = self.compute_garbage_score(snapshot);

            if garbage_score >= 0.5 {
                let days_since = self.compute_days_since_access(snapshot);

                new_candidates.push(GcCandidate {
                    memory_id: snapshot.memory_id.clone(),
                    garbage_score,
                    quality_score: snapshot.quality_score,
                    days_since_access: days_since,
                    reference_count: snapshot.reference_count,
                    is_synthesis: snapshot.memory_type.as_deref() == Some("synthesis"),
                    is_low_quality: snapshot.is_low_quality,
                    negative_feedback_count: snapshot.negative_feedback_count,
                    marked_at_ms: now,
                    status: "observed".to_string(),
                });
            }
        }

        // ============================================================
        // 阶段二：观察期检查 — 将过观察期的候选转为待删除
        // ============================================================
        let observation_period = self.state.config.observation_period_secs * 1000;
        let mut to_delete: Vec<String> = Vec::new();
        let mut to_keep: Vec<GcCandidate> = Vec::new();

        // 检查已有候选：过观察期的转为待删除，仍在观察期的保留
        let existing = std::mem::take(&mut self.state.candidates);
        for candidate in existing {
            if now - candidate.marked_at_ms >= observation_period {
                // 过观察期，在快照中查找该记忆确认仍可删除
                if self.can_delete_from_snapshots(&candidate, snapshots) {
                    to_delete.push(candidate.memory_id.clone());
                }
            } else {
                // 仍在观察期，保留
                to_keep.push(candidate);
            }
        }

        // 新候选加回列表
        to_keep.append(&mut new_candidates);
        self.state.candidates = to_keep;

        // ============================================================
        // 阶段三：返回待删除列表（调用方负责执行删除）
        // ============================================================
        let max_per_cycle = self.state.config.max_per_cycle;
        let to_delete: Vec<String> = to_delete.into_iter().take(max_per_cycle).collect();
        let removed_count = to_delete.len();

        // 更新统计（调用方负责实际删除，这里只记录预期删除数）
        self.state.stats.total_removed += removed_count;
        self.state.stats.total_freed += removed_count;
        self.state.stats.observing_count = self.state.candidates.len();
        self.state.stats.last_gc_ms = now;
        self.state.stats.last_removed_count = removed_count;

        (self.state.stats.clone(), to_delete)
    }

    /// 计算记忆的综合垃圾评分（快照版本）
    fn compute_garbage_score(&self, snapshot: &MemorySnapshot) -> f32 {
        let quality = snapshot.quality_score;
        let days_stale = self.compute_days_since_access(snapshot);
        let ref_count = snapshot.reference_count;
        let neg_feedback = snapshot.negative_feedback_count;
        let importance = snapshot.importance.unwrap_or(5) as f32;

        // 重要性保护：高重要性记忆降低垃圾评分
        if importance > self.state.config.max_importance_for_removal as f32 {
            return 0.0;
        }

        // 核心节点保护
        if !self.state.config.allow_core_node_removal && snapshot.is_core_node {
            return 0.0;
        }

        // 各维度权重（总和 1.0）
        let quality_weight = 0.35;
        let staleness_weight = 0.25;
        let isolation_weight = 0.20;
        let feedback_weight = 0.20;

        // 质量评分：低质量 → 高垃圾分
        let quality_score = (1.0 - quality).clamp(0.0, 1.0);

        // 陈旧度：超过阈值天数线性增长
        let stale_threshold = self.state.config.stale_days as f64;
        let staleness_score = if days_stale > stale_threshold {
            (days_stale / (stale_threshold * 2.0)).min(1.0)
        } else {
            0.0
        };

        // 孤立度：无引用 → 高垃圾分
        let isolation_score = if ref_count == 0 {
            0.8
        } else if ref_count <= 2 {
            0.3
        } else {
            0.0
        };

        // 用户反馈：负面反馈多 → 高垃圾分
        let feedback_score = if neg_feedback >= 3 {
            1.0
        } else if neg_feedback >= 1 {
            0.5
        } else {
            0.0
        };

        quality_score * quality_weight
            + staleness_score as f32 * staleness_weight
            + isolation_score * isolation_weight
            + feedback_score * feedback_weight
    }

    /// 计算记忆距上次访问的天数（快照版本）
    fn compute_days_since_access(&self, snapshot: &MemorySnapshot) -> f64 {
        let now = now_ms();
        match snapshot.last_accessed_ms {
            Some(last) => {
                let ms_since = now.saturating_sub(last);
                ms_since as f64 / (1000.0 * 3600.0 * 24.0)
            }
            None => 365.0, // 无访问记录，视为一年未访问
        }
    }

    /// 在快照中确认候选记忆仍可删除（观察期结束时的重新评估）
    fn can_delete_from_snapshots(
        &self,
        candidate: &GcCandidate,
        snapshots: &[MemorySnapshot],
    ) -> bool {
        // 在快照中查找该记忆
        let snapshot = match snapshots
            .iter()
            .find(|s| s.memory_id == candidate.memory_id)
        {
            Some(s) => s,
            None => {
                // 快照中不存在（可能已被其他流程删除），跳过
                return false;
            }
        };

        // 重新计算当前垃圾评分
        let current_score = self.compute_garbage_score(snapshot);
        if current_score < 0.5 {
            return false;
        }

        // 检查重要性是否在此期间被提升
        if let Some(importance) = snapshot.importance {
            if importance > self.state.config.max_importance_for_removal {
                return false;
            }
        }

        // 检查是否在此期间被重新访问
        if let Some(last_ms) = snapshot.last_accessed_ms {
            if last_ms > candidate.marked_at_ms {
                return false;
            }
        }

        true
    }

    /// 获取当前候选列表（供监控面板使用）
    pub fn get_candidates(&self) -> Vec<GcCandidate> {
        self.state.candidates.clone()
    }

    /// 获取垃圾回收统计
    pub fn get_stats(&self) -> GcStats {
        self.state.stats.clone()
    }

    /// 收集快照并记录性能基线（质疑三：动态警告阈值）
    ///
    /// 包装 MemorySnapshot::collect_all，使用动态基线判断是否触发警告。
    /// 仅当当前耗时 > 基线均值 + 3σ 时才输出警告，避免固定阈值产生噪音。
    pub fn collect_snapshots_with_timing(
        &mut self,
        query: &dyn MemoryInfoQuery,
    ) -> Vec<MemorySnapshot> {
        let start = std::time::Instant::now();
        let snapshots = MemorySnapshot::collect_all(query);
        let elapsed_ms = start.elapsed().as_millis() as f64;

        self.log_timing(elapsed_ms, snapshots.len());
        snapshots
    }

    /// 记录并评估快照收集耗时（质疑三：供调用方传入预计算的耗时）
    ///
    /// 当调用方已经完成了快照收集（因借用限制无法使用 collect_snapshots_with_timing）
    /// 时，可通过此方法单独记录耗时并评估。
    pub fn record_timing(&mut self, elapsed_ms: f64, total: usize) {
        self.log_timing(elapsed_ms, total);
    }

    /// 内部计时日志方法
    fn log_timing(&mut self, elapsed_ms: f64, total: usize) {
        let (should_warn, mean, stddev, deviation) = self.timing_baseline.record(elapsed_ms);

        // 基础日志
        if total > 1000 {
            eprintln!(
                "[LRC·GC·性能] 快照收集完成: {} 条记忆，耗时 {:.0}ms（{:.2}μs/条）",
                total,
                elapsed_ms,
                if total > 0 {
                    elapsed_ms * 1000.0 / total as f64
                } else {
                    0.0
                }
            );
        } else if total > 100 {
            eprintln!(
                "[LRC·GC·性能] 快照收集: {} 条记忆，耗时 {:.0}ms",
                total, elapsed_ms
            );
        }

        // 动态基线警告（质疑三：替代固定 500ms 阈值）
        if should_warn {
            eprintln!(
                "[LRC·GC·性能·警告] 快照耗时 {:.0}ms 显著偏离基线（均值 {:.0}ms，σ={:.0}ms，偏离 {:.1}σ）。\
                 可能原因：记忆库突增、系统负载升高或存储介质性能下降",
                elapsed_ms, mean, stddev, deviation
            );
        }
    }

    /// 道枢映射: 兑卦·泽 (☱) — 说万物者莫说乎泽，单条评估如泽水之滋润甄别
    /// 手动触发对特定记忆的回收评估（快照版本）
    pub fn evaluate_single(&self, snapshot: &MemorySnapshot) -> GcCandidate {
        let now = now_ms();
        let garbage_score = self.compute_garbage_score(snapshot);
        let days_since = self.compute_days_since_access(snapshot);

        GcCandidate {
            memory_id: snapshot.memory_id.clone(),
            garbage_score,
            quality_score: snapshot.quality_score,
            days_since_access: days_since,
            reference_count: snapshot.reference_count,
            is_synthesis: snapshot.memory_type.as_deref() == Some("synthesis"),
            is_low_quality: snapshot.is_low_quality,
            negative_feedback_count: snapshot.negative_feedback_count,
            marked_at_ms: now,
            status: if garbage_score >= 0.5 {
                "candidate"
            } else {
                "healthy"
            }
            .to_string(),
        }
    }

    /// 更新配置
    pub fn update_config(&mut self, config: GcConfig) {
        self.state.config = config;
    }

    /// 检查是否到了回收时间
    pub fn should_run(&self) -> bool {
        if !self.state.config.enabled {
            return false;
        }

        let now = now_ms();
        self.state.last_gc_ms == 0
            || now - self.state.last_gc_ms >= self.state.config.interval_secs * 1000
    }

    /// 获取上次 GC 运行时间戳（毫秒，0 表示从未运行，质疑五·健康报告）
    pub fn last_run_ms(&self) -> u64 {
        self.state.last_gc_ms
    }
}

impl Default for MemoryGarbageCollector {
    fn default() -> Self {
        Self::new(GcConfig::default())
    }
}

/// 获取当前毫秒时间戳
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用的模拟记忆查询器（保留用于评估和诊断）
    struct MockMemoryQuery {
        memories: Vec<MockMemoryInfo>,
        core_nodes: Vec<String>,
        low_quality: Vec<String>,
        deleted: Vec<String>,
    }

    struct MockMemoryInfo {
        id: String,
        last_accessed_ms: u64,
        importance: u8,
        memory_type: String,
        quality_score: f32,
        ref_count: usize,
        neg_feedback: usize,
    }

    impl MemoryInfoQuery for MockMemoryQuery {
        fn get_last_accessed_ms(&self, memory_id: &str) -> Option<u64> {
            self.memories
                .iter()
                .find(|m| m.id == memory_id)
                .map(|m| m.last_accessed_ms)
        }

        fn get_importance(&self, memory_id: &str) -> Option<u8> {
            self.memories
                .iter()
                .find(|m| m.id == memory_id)
                .map(|m| m.importance)
        }

        fn get_memory_type(&self, memory_id: &str) -> Option<String> {
            self.memories
                .iter()
                .find(|m| m.id == memory_id)
                .map(|m| m.memory_type.clone())
        }

        fn get_reference_count(&self, memory_id: &str) -> usize {
            self.memories
                .iter()
                .find(|m| m.id == memory_id)
                .map(|m| m.ref_count)
                .unwrap_or(0)
        }

        fn is_core_synthesis_node(&self, memory_id: &str) -> bool {
            self.core_nodes.contains(&memory_id.to_string())
        }

        fn is_low_quality_synthesis(&self, memory_id: &str) -> bool {
            self.low_quality.contains(&memory_id.to_string())
        }

        fn get_quality_score(&self, memory_id: &str) -> f32 {
            self.memories
                .iter()
                .find(|m| m.id == memory_id)
                .map(|m| m.quality_score)
                .unwrap_or(0.5)
        }

        fn get_negative_feedback_count(&self, memory_id: &str) -> usize {
            self.memories
                .iter()
                .find(|m| m.id == memory_id)
                .map(|m| m.neg_feedback)
                .unwrap_or(0)
        }

        fn get_all_memory_ids(&self) -> Vec<String> {
            self.memories.iter().map(|m| m.id.clone()).collect()
        }

        fn delete_memory(&mut self, memory_id: &str) -> bool {
            if self.memories.iter().any(|m| m.id == memory_id) {
                self.deleted.push(memory_id.to_string());
                self.memories.retain(|m| m.id != memory_id);
                true
            } else {
                false
            }
        }
    }

    fn make_mock_query() -> MockMemoryQuery {
        let now = now_ms();

        // 60 天前
        let sixty_days_ago = now - 60 * 24 * 3600 * 1000;
        // 1 天前
        let one_day_ago = now - 24 * 3600 * 1000;

        MockMemoryQuery {
            memories: vec![
                MockMemoryInfo {
                    id: "healthy_mem".to_string(),
                    last_accessed_ms: one_day_ago,
                    importance: 7,
                    memory_type: "fact".to_string(),
                    quality_score: 0.8,
                    ref_count: 5,
                    neg_feedback: 0,
                },
                MockMemoryInfo {
                    id: "stale_low_quality".to_string(),
                    last_accessed_ms: sixty_days_ago,
                    importance: 2,
                    memory_type: "synthesis".to_string(),
                    quality_score: 0.1,
                    ref_count: 0,
                    neg_feedback: 3,
                },
                MockMemoryInfo {
                    id: "isolated_mem".to_string(),
                    last_accessed_ms: sixty_days_ago,
                    importance: 1,
                    memory_type: "fact".to_string(),
                    quality_score: 0.5,
                    ref_count: 0,
                    neg_feedback: 0,
                },
                MockMemoryInfo {
                    id: "core_node".to_string(),
                    last_accessed_ms: one_day_ago,
                    importance: 8,
                    memory_type: "synthesis".to_string(),
                    quality_score: 0.9,
                    ref_count: 10,
                    neg_feedback: 0,
                },
                MockMemoryInfo {
                    id: "high_importance_stale".to_string(),
                    last_accessed_ms: sixty_days_ago,
                    importance: 9,
                    memory_type: "decision".to_string(),
                    quality_score: 0.6,
                    ref_count: 3,
                    neg_feedback: 0,
                },
            ],
            core_nodes: vec!["core_node".to_string()],
            low_quality: vec!["stale_low_quality".to_string()],
            deleted: Vec::new(),
        }
    }

    /// 辅助函数：从查询器构建快照
    fn make_snapshots(query: &dyn MemoryInfoQuery) -> Vec<MemorySnapshot> {
        MemorySnapshot::collect_all(query)
    }

    #[test]
    fn test_gc_identifies_low_quality_candidates() {
        let gc = MemoryGarbageCollector::default();
        let query = make_mock_query();
        let snapshots = make_snapshots(&query);

        let stats = gc.get_stats();
        assert_eq!(stats.total_cycles, 0);

        // 评估单个记忆
        let stale_snapshot = snapshots
            .iter()
            .find(|s| s.memory_id == "stale_low_quality")
            .unwrap();
        let candidate = gc.evaluate_single(stale_snapshot);
        // 低质量 + 长期未访问 + 无引用 + 负面反馈 → 高垃圾评分
        assert!(
            candidate.garbage_score > 0.5,
            "低质量+长期未访问+无引用+负面反馈的记忆应有高垃圾评分: {}",
            candidate.garbage_score
        );
        assert!(candidate.is_low_quality);
        assert_eq!(candidate.negative_feedback_count, 3);
    }

    #[test]
    fn test_gc_protects_high_importance() {
        let gc = MemoryGarbageCollector::default();
        let query = make_mock_query();
        let snapshots = make_snapshots(&query);

        let hi_snapshot = snapshots
            .iter()
            .find(|s| s.memory_id == "high_importance_stale")
            .unwrap();
        let candidate = gc.evaluate_single(hi_snapshot);
        // 高重要性记忆即使长期未访问也不应被回收
        assert_eq!(
            candidate.garbage_score, 0.0,
            "高重要性记忆应受保护: {}",
            candidate.garbage_score
        );
    }

    #[test]
    fn test_gc_protects_core_nodes() {
        let gc = MemoryGarbageCollector::default();
        let query = make_mock_query();
        let snapshots = make_snapshots(&query);

        let core_snapshot = snapshots
            .iter()
            .find(|s| s.memory_id == "core_node")
            .unwrap();
        let candidate = gc.evaluate_single(core_snapshot);
        // 核心节点不应被回收
        assert_eq!(
            candidate.garbage_score, 0.0,
            "核心节点应受保护: {}",
            candidate.garbage_score
        );
    }

    #[test]
    fn test_gc_healthy_memory_low_score() {
        let gc = MemoryGarbageCollector::default();
        let query = make_mock_query();
        let snapshots = make_snapshots(&query);

        let healthy_snapshot = snapshots
            .iter()
            .find(|s| s.memory_id == "healthy_mem")
            .unwrap();
        let candidate = gc.evaluate_single(healthy_snapshot);
        // 健康记忆应有低垃圾评分
        assert!(
            candidate.garbage_score < 0.5,
            "健康记忆应有低垃圾评分: {}",
            candidate.garbage_score
        );
    }

    #[test]
    fn test_gc_isolated_memory_medium_score() {
        let gc = MemoryGarbageCollector::default();
        let query = make_mock_query();
        let snapshots = make_snapshots(&query);

        let iso_snapshot = snapshots
            .iter()
            .find(|s| s.memory_id == "isolated_mem")
            .unwrap();
        let candidate = gc.evaluate_single(iso_snapshot);
        // 孤立记忆应有中等垃圾评分
        assert!(
            candidate.garbage_score > 0.3,
            "孤立记忆应有中等垃圾评分: {}",
            candidate.garbage_score
        );
    }

    #[test]
    fn test_gc_collect_cycle() {
        let mut gc = MemoryGarbageCollector::new(GcConfig {
            observation_period_secs: 0, // 跳过观察期，立即删除
            interval_secs: 0,           // 允许连续调用，不等待间隔
            max_per_cycle: 10,
            ..GcConfig::default()
        });
        let mut query = make_mock_query();

        let before_count = query.get_all_memory_ids().len();
        // 第一次调用：收集快照，标记候选（新候选加入观察列表）
        let snapshots1 = make_snapshots(&query);
        let (_stats, to_delete1) = gc.collect_garbage(&snapshots1);
        // 第一次调用不应有删除（候选刚标记，观察期 0 但候选在 drain 前为空）
        // 手动执行删除（即使为空也无副作用）
        for id in &to_delete1 {
            query.delete_memory(id);
        }

        // 第二次调用：观察期已过（observation_period_secs=0），删除候选
        let snapshots2 = make_snapshots(&query);
        let (stats, to_delete2) = gc.collect_garbage(&snapshots2);
        for id in &to_delete2 {
            query.delete_memory(id);
        }
        let after_count = query.get_all_memory_ids().len();

        // 至少应删除 stale_low_quality
        assert!(stats.last_removed_count > 0, "GC 应删除至少一条低质量记忆");
        assert!(after_count < before_count, "GC 后记忆数应减少");
    }

    #[test]
    fn test_gc_disabled() {
        let mut gc = MemoryGarbageCollector::new(GcConfig {
            enabled: false,
            ..GcConfig::default()
        });
        let mut query = make_mock_query();
        let snapshots = make_snapshots(&query);

        let (stats, to_delete) = gc.collect_garbage(&snapshots);
        for id in &to_delete {
            query.delete_memory(id);
        }
        assert_eq!(stats.last_removed_count, 0, "GC 禁用时不应删除任何记忆");
    }

    #[test]
    fn test_gc_default_config() {
        let config = GcConfig::default();
        assert!(config.enabled);
        assert_eq!(config.interval_secs, 3600);
        assert_eq!(config.max_per_cycle, 50);
        assert!(!config.allow_core_node_removal);
    }
}
