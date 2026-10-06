// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件实现道同构度监控仪表，属于守护层 (Layer 2)。
// ============================================================
//
// 道同构度（DAO Isomorphism）监控仪表
//
// 洛书记忆系统的健康度指标收集与暴露。
//
// 核心指标：
//   - 道同构度 (dao_isomorphism): 洛书幻和约束的整体满足度
//   - 编码吞吐量 (encodings_total): 累计编码次数
//   - 合成次数 (compositions_total): 递归合成触发次数
//   - 检索次数 (recalls_total): 累计检索调用次数
//   - 修正次数 (corrections_total): 用户修正次数
//   - 活跃记忆数 (active_memories): 当前活跃记忆数
//   - 结晶记忆数 (crystallized_memories): 已合成的抽象记忆数
//   - 八卦分布 (bagua_distribution): 各卦象的记忆分布熵

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::engine::luoshu_encoder::LUOSHU_DEVIATION_MAX;

/// 道同构度指标聚合器
///
/// 线程安全的计数器集合，使用 AtomicU64 实现无锁并发更新。
/// 所有指标均可通过 MCP 工具 `/health/dao_metrics` 暴露。
#[derive(Debug)]
pub struct DaoMetrics {
    /// 累计编码次数（洛书向量生成次数）
    encodings_total: AtomicU64,
    /// 递归合成触发次数
    compositions_total: AtomicU64,
    /// 累计检索调用次数
    recalls_total: AtomicU64,
    /// 用户修正次数
    corrections_total: AtomicU64,
    /// 最后采集时间戳（Unix 毫秒）
    last_collected: AtomicU64,
}

impl DaoMetrics {
    /// 创建新的道同构度指标实例
    pub fn new() -> Self {
        Self {
            encodings_total: AtomicU64::new(0),
            compositions_total: AtomicU64::new(0),
            recalls_total: AtomicU64::new(0),
            corrections_total: AtomicU64::new(0),
            last_collected: AtomicU64::new(0),
        }
    }

    /// 记录一次编码操作
    pub fn record_encoding(&self) {
        self.encodings_total.fetch_add(1, Ordering::Relaxed);
        self.touch();
    }

    /// 记录一次递归合成操作
    pub fn record_composition(&self) {
        self.compositions_total.fetch_add(1, Ordering::Relaxed);
        self.touch();
    }

    /// 记录一次检索操作
    pub fn record_recall(&self) {
        self.recalls_total.fetch_add(1, Ordering::Relaxed);
        self.touch();
    }

    /// 记录一次用户修正操作
    pub fn record_correction(&self) {
        self.corrections_total.fetch_add(1, Ordering::Relaxed);
        self.touch();
    }

    /// 更新最后活跃时间戳
    fn touch(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.last_collected.store(now, Ordering::Relaxed);
    }

    /// 道枢映射: 坤卦·地 (☷) — 厚德载物，编码总数如大地承载万物
    /// 获取编码总数
    pub fn encodings_total(&self) -> u64 {
        self.encodings_total.load(Ordering::Relaxed)
    }

    /// 道枢映射: 震卦·雷 (☳) — 万物出乎震，合成总数如春雷唤醒生机
    /// 获取合成总数
    pub fn compositions_total(&self) -> u64 {
        self.compositions_total.load(Ordering::Relaxed)
    }

    /// 道枢映射: 离卦·火 (☲) — 明两作，召回总数如火光之普照
    /// 获取检索总数
    pub fn recalls_total(&self) -> u64 {
        self.recalls_total.load(Ordering::Relaxed)
    }

    /// 道枢映射: 兑卦·泽 (☱) — 说万物者莫说乎泽，修正如泽水之润物无声
    /// 获取修正总数
    pub fn corrections_total(&self) -> u64 {
        self.corrections_total.load(Ordering::Relaxed)
    }
}

/// 道同构度快照（用于 API 响应序列化）
///
/// 一次性采集所有指标，生成可序列化的快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaoMetricsSnapshot {
    /// 累计编码次数
    pub encodings_total: u64,
    /// 递归合成触发次数
    pub compositions_total: u64,
    /// 累计检索调用次数
    pub recalls_total: u64,
    /// 用户修正次数
    pub corrections_total: u64,
    /// 当前活跃记忆数
    pub active_memories: usize,
    /// 已合成的抽象记忆（Synthesis 类型）数量
    pub crystallized_memories: usize,
    /// 已归档记忆数量
    pub archived_memories: usize,
    /// 道同构度评分（0.0 ~ 1.0）
    ///
    /// 基于洛书幻和约束的平均偏离度计算。
    /// 1.0 表示所有记忆的洛书向量完美满足幻和约束，
    /// 0.0 表示完全偏离。
    pub dao_isomorphism_score: f32,
    /// 八卦分布熵（0.0 ~ 3.0）
    ///
    /// 基于香农熵计算，反映记忆在八卦类别上的分布均匀度。
    /// 越大表示分布越均匀，0 表示所有记忆集中在同一卦象。
    pub bagua_entropy: f32,
    /// 合成/原始记忆比率
    pub synthesis_ratio: f32,
    /// 最后采集时间戳（Unix 毫秒）
    pub last_collected_ms: u64,
}

impl DaoMetrics {
    /// 道枢映射: 洛书·幻和 — 九宫格幻和偏离度是道同构度的核心度量，快照如镜面反映系统健康
    ///
    /// 采集当前指标快照
    ///
    /// 需要传入外部数据（记忆总数、八卦分布等），
    /// 因为这些数据由 MemoryStore 管理。
    pub fn snapshot(
        &self,
        total_memories: usize,
        crystallized_count: usize,
        archived_count: usize,
        avg_luoshu_deviation: f32,
        bagua_counts: &[usize; 8],
    ) -> DaoMetricsSnapshot {
        // 计算道同构度：按 luoshu_deviation 的**结构最大值**归一化，回到 [0,1]（偏离度越低越好）。
        // v0.9.10 修正：luoshu_deviation 归一化契约变更后，其可行域已扩展为
        // [0, LUOSHU_DEVIATION_MAX]（≈2.0456），而旧实现 `1.0 - x.min(1.0)` 中的
        // `.min(1.0)` 是旧尺度遗留 —— 实测均值偏离度≈1.62 会被饱和钳到 0.0，
        // 指标失去区分度，并因此误判系统处于异常。改为除以结构最大值。
        // NaN 安全：非有限偏离度 → 0.0，与旧实现语义一致。
        let dao_score = if avg_luoshu_deviation.is_finite() {
            (1.0 - avg_luoshu_deviation / LUOSHU_DEVIATION_MAX).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // 计算八卦分布熵（香农熵）
        let bagua_entropy = compute_bagua_entropy(bagua_counts);

        // 合成比率
        let synthesis_ratio = if total_memories > 0 {
            crystallized_count as f32 / total_memories as f32
        } else {
            0.0
        };

        let last_collected = self.last_collected.load(Ordering::Relaxed);

        DaoMetricsSnapshot {
            encodings_total: self.encodings_total(),
            compositions_total: self.compositions_total(),
            recalls_total: self.recalls_total(),
            corrections_total: self.corrections_total(),
            active_memories: total_memories,
            crystallized_memories: crystallized_count,
            archived_memories: archived_count,
            dao_isomorphism_score: dao_score,
            bagua_entropy,
            synthesis_ratio,
            last_collected_ms: last_collected,
        }
    }
}

impl Default for DaoMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// 计算八卦分布的香农熵
///
/// 熵值范围 [0, log2(8)] = [0, 3.0]。
/// 熵越大表示记忆在八卦类别上的分布越均匀。
///
/// v0.9.10：可见性由 `fn` 放宽为 `pub(crate)`，供
/// [`crate::memory_store::MemoryStore::reclassify_bagua`] 复用同一公式
/// （历史重分类需在重分类前后各算一次熵以校验分布是否被打散，
/// 若重复实现会导致两处口径漂移）。
pub(crate) fn compute_bagua_entropy(counts: &[usize; 8]) -> f32 {
    let total: usize = counts.iter().sum();
    if total == 0 {
        return 0.0;
    }

    let mut entropy = 0.0f32;
    for &count in counts.iter() {
        if count > 0 {
            let p = count as f32 / total as f32;
            entropy -= p * p.log2();
        }
    }
    entropy
}

/// 计算洛书幻和平均偏离度
///
/// 对所有记忆的洛书向量，计算行/列/对角线之和与 1.0 的偏离程度。
/// 返回值越小越好（0.0 = 完美满足幻和约束）。
pub fn compute_avg_luoshu_deviation(vectors: &[[f32; 9]]) -> f32 {
    if vectors.is_empty() {
        return 0.0;
    }

    let mut total_deviation = 0.0f32;
    for vec in vectors {
        total_deviation +=
            crate::engine::luoshu_encoder::LuoShuVector::new(*vec).luoshu_deviation();
    }
    total_deviation / vectors.len() as f32
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dao_metrics_recording() {
        let metrics = DaoMetrics::new();
        assert_eq!(metrics.encodings_total(), 0);
        assert_eq!(metrics.recalls_total(), 0);

        metrics.record_encoding();
        metrics.record_encoding();
        metrics.record_recall();
        metrics.record_composition();
        metrics.record_correction();

        assert_eq!(metrics.encodings_total(), 2);
        assert_eq!(metrics.recalls_total(), 1);
        assert_eq!(metrics.compositions_total(), 1);
        assert_eq!(metrics.corrections_total(), 1);
    }

    #[test]
    fn test_bagua_entropy_uniform() {
        // 均匀分布：每个卦象各 1 条，熵应为 log2(8) = 3.0
        let counts = [1usize; 8];
        let entropy = compute_bagua_entropy(&counts);
        assert!(
            (entropy - 3.0).abs() < 0.01,
            "均匀分布熵应为 3.0，实际: {}",
            entropy
        );
    }

    #[test]
    fn test_bagua_entropy_single() {
        // 全部集中在第一个卦象，熵应为 0.0
        let mut counts = [0usize; 8];
        counts[0] = 10;
        let entropy = compute_bagua_entropy(&counts);
        assert!(
            (entropy - 0.0).abs() < 0.01,
            "单类熵应为 0.0，实际: {}",
            entropy
        );
    }

    #[test]
    fn test_bagua_entropy_empty() {
        let counts = [0usize; 8];
        let entropy = compute_bagua_entropy(&counts);
        assert_eq!(entropy, 0.0);
    }

    #[test]
    fn test_dao_snapshot() {
        let metrics = DaoMetrics::new();
        metrics.record_encoding();
        metrics.record_recall();

        let bagua_counts = [1, 2, 0, 0, 1, 0, 0, 0];
        let snapshot = metrics.snapshot(10, 2, 1, 0.15, &bagua_counts);

        assert_eq!(snapshot.encodings_total, 1);
        assert_eq!(snapshot.recalls_total, 1);
        assert_eq!(snapshot.active_memories, 10);
        assert_eq!(snapshot.crystallized_memories, 2);
        assert_eq!(snapshot.archived_memories, 1);
        assert!(
            snapshot.dao_isomorphism_score > 0.8,
            "偏离度 0.15 → 道同构度 > 0.8"
        );
        assert!(snapshot.bagua_entropy > 0.0, "非空分布应有正熵");
        assert!((snapshot.synthesis_ratio - 0.2).abs() < 0.01);
    }

    #[test]
    fn test_dao_perfect_isomorphism() {
        let metrics = DaoMetrics::new();
        let snapshot = metrics.snapshot(5, 0, 0, 0.0, &[0; 8]);
        assert_eq!(snapshot.dao_isomorphism_score, 1.0, "零偏离度 → 完美道同构");
    }

    /// ★★v0.9.10：道同构度必须按**结构最大值**归一化，而非旧尺度的 `.min(1.0)`。
    ///
    /// # 这条测的是实测发现的指标失效
    ///
    /// v0.9.10 改了 `luoshu_deviation` 的尺度（归一化契约变更后，理想每线和
    /// 目标改为 `15/√285 ≈ 0.8885`，偏离度可行域扩展为 `[0, 2.0456]`），
    /// 但 [`DaoMetrics::snapshot`] 的 score 映射仍停留在旧尺度：
    ///
    /// ```text
    /// dao_score = (1.0 - avg_deviation.min(1.0)).max(0.0)
    /// ```
    ///
    /// 于是实测 `avg_deviation ≈ 1.62` 被 `.min(1.0)` 饱和 → `dao_score` 恒为 0.0，
    /// 指标**彻底失去区分度**，还连带把健康的系统误判为「道同构度异常」。
    ///
    /// 修复契约：除以结构最大值 `LUOSHU_DEVIATION_MAX`，使 score 回到 [0,1] 且单调。
    #[test]
    fn test_dao_score_uses_structural_max() {
        let metrics = DaoMetrics::new();

        // 实测场景：avg_deviation ≈ 1.62（dev 服务 regulator 报「洛书幻和平齐偏离度 1.62」）
        let snapshot = metrics.snapshot(3189, 47, 0, 1.62, &[400; 8]);
        let expected = 1.0 - 1.62 / 2.0456; // ≈ 0.208
        assert!(
            (snapshot.dao_isomorphism_score - expected).abs() < 0.005,
            "avg_deviation=1.62 → score 应≈{:.3}（按结构最大值归一化），实际 {:.4}",
            expected,
            snapshot.dao_isomorphism_score
        );
        assert!(
            snapshot.dao_isomorphism_score > 0.0,
            "旧实现因 .min(1.0) 饱和把 score 钳为 0.0，失去区分度"
        );

        // 单调性：偏离度越大 score 越小
        let low = metrics
            .snapshot(10, 0, 0, 0.5, &[1; 8])
            .dao_isomorphism_score;
        let high = metrics
            .snapshot(10, 0, 0, 1.5, &[1; 8])
            .dao_isomorphism_score;
        assert!(
            low > high,
            "偏离度越大，道同构度应越小（{} > {}）",
            low,
            high
        );

        // 边界：超出结构最大值 → 钳为 0.0；零偏离 → 1.0
        assert_eq!(
            metrics
                .snapshot(10, 0, 0, 5.0, &[1; 8])
                .dao_isomorphism_score,
            0.0,
            "偏离度超出结构最大值应钳为 0.0"
        );
        assert_eq!(
            metrics
                .snapshot(10, 0, 0, 0.0, &[1; 8])
                .dao_isomorphism_score,
            1.0
        );
    }
}
