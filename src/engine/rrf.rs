// ============================================================
// RRF 融合 — 倒数排名融合 (Reciprocal Rank Fusion)
//
// 从 server.rs 和 v1_api.rs 中提取的公共 RRF 融合逻辑。
// 用于将多路检索结果（快速通路 + 深度通路）合并为统一排序。
// ============================================================

use crate::memory_types::Memory;
use crate::memory_store::RecallResult;
use std::collections::HashMap;

/// 默认 RRF 常数 k（控制排名对分数的敏感度，k 越大排名差异越小）
pub const RRF_DEFAULT_K: f32 = 60.0;

/// RRF 融合结果：按分数排序的记忆列表及对应的分数
pub struct RrfFusedResult {
    pub memories: Vec<Memory>,
    pub scores: Vec<f32>,
    pub total_candidates: usize,
}

/// 倒数排名融合 (RRF, Reciprocal Rank Fusion)
///
/// 将快速通路和深度通路的结果合并，使用 RRF 公式计算融合分数。
/// 公式: score = sum(1 / (k + rank_i))，其中 k = 60
///
/// 返回按融合分数降序排列的结果，最多取 top_k 条。
pub fn rrf_fuse(
    fast: &RecallResult,
    deep: &RecallResult,
    top_k: usize,
    rrf_k: f32,
) -> RrfFusedResult {
    let mut fused_scores: HashMap<String, f32> = HashMap::new();
    let mut id_to_memory: HashMap<String, Memory> = HashMap::new();

    // 快速通路排名
    for (rank, m) in fast.memories.iter().enumerate() {
        let score = 1.0 / (rrf_k + (rank + 1) as f32);
        *fused_scores.entry(m.id.clone()).or_insert(0.0) += score;
        id_to_memory
            .entry(m.id.clone())
            .or_insert_with(|| m.clone());
    }

    // 深度通路排名
    for (rank, m) in deep.memories.iter().enumerate() {
        let score = 1.0 / (rrf_k + (rank + 1) as f32);
        *fused_scores.entry(m.id.clone()).or_insert(0.0) += score;
        id_to_memory
            .entry(m.id.clone())
            .or_insert_with(|| m.clone());
    }

    // 按融合分数排序
    let mut scored: Vec<(f32, String)> = fused_scores
        .into_iter()
        .map(|(id, score)| (score, id))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // 截取 top_k
    let total = id_to_memory.len();
    let result_memories: Vec<Memory> = scored
        .into_iter()
        .take(top_k)
        .filter_map(|(_, id)| id_to_memory.remove(&id))
        .collect();
    let result_scores: Vec<f32> = (0..result_memories.len())
        .map(|i| {
            let rank = (i + 1) as f32;
            1.0 / (1.0 + rank.log10())
        })
        .collect();

    RrfFusedResult {
        memories: result_memories,
        scores: result_scores,
        total_candidates: total,
    }
}