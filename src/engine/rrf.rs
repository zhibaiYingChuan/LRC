// ============================================================
// RRF 融合 — 倒数排名融合 (Reciprocal Rank Fusion)
//
// 从 server.rs 和 v1_api.rs 中提取的公共 RRF 融合逻辑。
// 用于将多路检索结果（快速通路 + 深度通路）合并为统一排序。
// ============================================================

use crate::memory_store::RecallResult;
use crate::memory_types::Memory;
use std::collections::{HashMap, HashSet};

/// 默认 RRF 常数 k（控制排名对分数的敏感度，k 越大排名差异越小）
pub const RRF_DEFAULT_K: f32 = 60.0;

/// 根据查询是否包含具体错误或组件线索调整检索通路权重。
/// 具体问题优先 Deep，泛化问题保持等权，避免泛化词污染结果。
pub fn query_path_weights(query: &str) -> (f32, f32) {
    let specific_terms = [
        "traceback",
        "module",
        "moduleNotFoundError",
        "borrow checker",
        "cargo",
        "500",
        "404",
        "超时",
        "错误",
        "报错",
        "异常",
        "依赖",
        "组件",
        "接口",
    ];
    let specific = specific_terms
        .iter()
        .filter(|term| query.to_lowercase().contains(&term.to_lowercase()))
        .count();
    if specific >= 2 {
        (0.8, 1.8)
    } else if specific == 1 {
        (1.0, 1.4)
    } else {
        (1.0, 1.0)
    }
}

/// 单条记忆的通路贡献明细（阶段D 联想解释用，只观测不排序）。
///
/// 与 `RrfFusedResult` 的 memories/scores 平行，记录每个聚合桶在快速
/// 路径与深度路径上的 RRF 贡献分及各自排名，供面向用户的"为什么联想
/// 这条"解释；该数据不参与排序决策，仅为观测信息。
#[derive(Debug, Clone, Copy, Default)]
pub struct RrfContribution {
    /// 快速路径贡献分 = fast_weight / (k + fast_rank)
    pub fast_contrib: f32,
    /// 深度路径贡献分 = deep_weight / (k + deep_rank)
    pub deep_contrib: f32,
    /// 在快速路径结果中的排名（1 起，未命中为 None）
    pub fast_rank: Option<usize>,
    /// 在深度路径结果中的排名（1 起，未命中为 None）
    pub deep_rank: Option<usize>,
}

impl RrfContribution {
    /// 真实融合贡献分（两路贡献之和，与排序无关，仅供观测/解释）
    pub fn fused_contrib(&self) -> f32 {
        self.fast_contrib + self.deep_contrib
    }

    /// 命中的检索通路列表（"fast"/"deep"）
    pub fn hit_paths(&self) -> Vec<&'static str> {
        let mut paths = Vec::with_capacity(2);
        if self.fast_rank.is_some() {
            paths.push("fast");
        }
        if self.deep_rank.is_some() {
            paths.push("deep");
        }
        paths
    }
}

/// RRF 融合结果：按分数排序的记忆列表及对应的分数，含每条的路径贡献明细
pub struct RrfFusedResult {
    pub memories: Vec<Memory>,
    pub scores: Vec<f32>,
    pub total_candidates: usize,
    /// 与 memories 平行的每桶通路贡献（阶段D 联想解释，只观测）
    pub contributions: Vec<RrfContribution>,
}

/// 记忆内容指纹：对规范化内容（trim 去首尾空白）计算 FNV-1a 64 位稳定哈希。
/// 同内容跨 id/project 分裂时指纹一致，用于聚合键；不参与排序。
fn content_fingerprint(content: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in content.trim().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// RRF 融合聚合键：(source, 内容指纹)。
///
/// v0.9.6 t3 修复：同一物理内容（如 USER_GUIDE.md 片段）在库中按 project 分裂
/// 为不同 id 的记忆时，按 `m.id` 聚合无法合并 fast/deep 两路的 RRF 贡献
/// （各仅 1/(k+rank)），导致正确目标记忆跌出 fused top1。改用
/// (source, 内容指纹) 后，同源同内容的记忆聚到同一桶累加融合分；
/// 跨来源的同文本（source 不同）仍保持独立桶，不会误并。
fn fusion_key(memory: &Memory) -> String {
    let source = memory.source.as_deref().unwrap_or("");
    let fingerprint = content_fingerprint(&memory.content);
    format!("{}::{}", source, fingerprint)
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
    rrf_fuse_weighted(fast, deep, top_k, rrf_k, 1.0, 1.0)
}

/// 带检索通路权重的 RRF 融合。
///
/// 权重用于表达查询意图：具体错误/组件查询提高深度路径权重，
/// 泛化查询保持快速路径和深度路径等权，避免破坏默认行为。
pub fn rrf_fuse_weighted(
    fast: &RecallResult,
    deep: &RecallResult,
    top_k: usize,
    rrf_k: f32,
    fast_weight: f32,
    deep_weight: f32,
) -> RrfFusedResult {
    // 参数无效时返回空结果，避免 NaN/无穷分数污染排序。
    if !rrf_k.is_finite()
        || rrf_k < 0.0
        || !fast_weight.is_finite()
        || fast_weight < 0.0
        || !deep_weight.is_finite()
        || deep_weight < 0.0
    {
        // 2026-09-01 修复(P2)：非法参数给出诊断日志，避免被误读为"无召回"
        eprintln!(
            "[rrf] 非法参数：rrf_k={} fast_weight={} deep_weight={}，已返回空融合结果",
            rrf_k, fast_weight, deep_weight
        );
        return RrfFusedResult {
            memories: Vec::new(),
            scores: Vec::new(),
            total_candidates: 0,
            contributions: Vec::new(),
        };
    }
    // scores 是召回器的平行元数据；RRF 的排名以 memories 顺序为准，
    // 不因 scores 长度不一致而制造虚假条目，输出 scores 始终与 memories 对齐。
    // 聚合键升级：(source, 内容指纹)。同源同内容的记忆（即使因 project
    // 分裂为不同 id）合并为一桶累加两路贡献；跨来源同文本不误并。
    let mut fused_scores: HashMap<String, f32> = HashMap::new();
    let mut key_to_memory: HashMap<String, Memory> = HashMap::new();
    // 阶段D：每桶的两路贡献明细（联想解释用，只观测不参与排序）
    let mut key_contributions: HashMap<String, RrfContribution> = HashMap::new();

    // 快速通路排名
    // 同一路径内同一聚合桶只计首次出现（rank 与贡献一致）：
    // 重复记录会导致分数被多次累加而 rank 仅保留末次，解释层无法还原。
    let mut fast_seen: HashSet<String> = HashSet::new();
    for (rank, m) in fast.memories.iter().enumerate() {
        let key = fusion_key(m);
        if !fast_seen.insert(key.clone()) {
            continue; // 同路径重复桶：跳过，避免贡献与排名不一致
        }
        // RRF ranks are one-based; the enumeration index is zero-based.
        let score = fast_weight / (rrf_k + (rank + 1) as f32);
        *fused_scores.entry(key.clone()).or_insert(0.0) += score;
        let contrib = key_contributions.entry(key.clone()).or_default();
        contrib.fast_contrib += score;
        contrib.fast_rank = Some(rank + 1);
        key_to_memory.entry(key).or_insert_with(|| m.clone());
    }

    // 深度通路排名
    let mut deep_seen: HashSet<String> = HashSet::new();
    for (rank, m) in deep.memories.iter().enumerate() {
        let key = fusion_key(m);
        if !deep_seen.insert(key.clone()) {
            continue; // 同路径重复桶：跳过，避免贡献与排名不一致
        }
        let score = deep_weight / (rrf_k + (rank + 1) as f32);
        *fused_scores.entry(key.clone()).or_insert(0.0) += score;
        let contrib = key_contributions.entry(key.clone()).or_default();
        contrib.deep_contrib += score;
        contrib.deep_rank = Some(rank + 1);
        key_to_memory.entry(key).or_insert_with(|| m.clone());
    }

    // 按融合分数排序；只保留有限结果，避免异常输入传播。
    let mut scored: Vec<(f32, String)> = fused_scores
        .into_iter()
        .filter(|(_, score)| score.is_finite())
        .map(|(key, score)| (score, key))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // 截取 top_k
    let total = key_to_memory.len();
    let top_scored = scored.iter().take(top_k);
    let mut result_memories = Vec::new();
    let mut result_scores = Vec::new();
    for (score, key) in top_scored {
        if let Some(memory) = key_to_memory.remove(key) {
            result_memories.push(memory);
            result_scores.push(*score);
        }
    }
    // 按 result_memories 相同顺序取出对应桶的贡献明细
    let result_contributions: Vec<RrfContribution> = scored
        .iter()
        .take(top_k)
        .filter_map(|(_, key)| key_contributions.remove(key))
        .collect();

    RrfFusedResult {
        memories: result_memories,
        scores: result_scores,
        total_candidates: total,
        contributions: result_contributions,
    }
}

#[cfg(test)]
mod tests {
    use super::{fusion_key, query_path_weights, rrf_fuse_weighted, RRF_DEFAULT_K};
    use crate::memory_store::RecallResult;
    use crate::memory_types::{Importance, Memory, MemoryType};

    fn make_memory(id: &str, content: &str, source: Option<&str>, project: Option<&str>) -> Memory {
        let mut m = Memory::new(
            content.to_string(),
            MemoryType::Fact,
            project.map(str::to_string),
            vec![],
            Importance::default(),
            None,
        );
        m.id = id.to_string();
        m.source = source.map(str::to_string);
        m
    }

    #[test]
    fn 具体错误查询提高深度权重() {
        let (fast, deep) = query_path_weights("Python ModuleNotFoundError traceback 怎么排查");
        assert!(deep > fast);
    }

    #[test]
    fn 泛化查询保持等权() {
        assert_eq!(query_path_weights("如何改善代码质量"), (1.0, 1.0));
    }

    /// t3 根因场景：同一物理内容（USER_GUIDE.md 片段）按 project 分裂为两条
    /// 不同 id 的记忆（fast 命中 v8、deep 命中 v7）。按 id 聚合时两路各得
    /// 1/(k+rank) 无法合并，正确目标跌出 fused top1；按 (source, 内容指纹)
    /// 聚合后双路贡献累加，应回到 top1（输出记忆保留 fast 路径那条）。
    #[test]
    fn 同源同内容跨id分裂的记忆在融合时聚为一桶() {
        let fast = RecallResult {
            memories: vec![
                make_memory(
                    "fast-id-a",
                    "Rust 编译失败 服务模块缺失 依赖版本过旧 建议更新",
                    Some("USER_GUIDE.md"),
                    Some("proj-x"),
                ),
                make_memory(
                    "fast-id-b",
                    "完全无关的干扰记忆内容",
                    Some("OTHER.md"),
                    Some("proj-x"),
                ),
            ],
            scores: vec![1.0, 0.8],
            total: 2,
            regression_evidence: std::collections::HashMap::new(),
        };
        let deep = RecallResult {
            memories: vec![
                // 与 fast-id-a 同内容同来源、但不同 id（跨 project 分裂）
                make_memory(
                    "deep-id-a",
                    "Rust 编译失败 服务模块缺失 依赖版本过旧 建议更新",
                    Some("USER_GUIDE.md"),
                    Some("proj-y"),
                ),
                make_memory("deep-id-b", "另一条干扰记忆", None, None),
            ],
            scores: vec![0.95, 0.7],
            total: 2,
            regression_evidence: std::collections::HashMap::new(),
        };
        let fused = rrf_fuse_weighted(&fast, &deep, 10, RRF_DEFAULT_K, 1.0, 1.8);
        assert_eq!(
            fused.total_candidates, 3,
            "应聚为 3 个唯一 (source, 指纹) 桶"
        );
        let top = &fused.memories[0];
        assert!(
            top.content.contains("Rust 编译失败"),
            "双路贡献合并后目标记忆应回到 fused top1，实际 top1: {}",
            top.content
        );
        assert_eq!(top.id, "fast-id-a", "同桶保留先到（fast 路径）的记忆对象");
        assert_eq!(
            fusion_key(top),
            fusion_key(&fast.memories[0]),
            "top1 应为 a 记忆所在桶"
        );
    }

    /// 保守性：不同来源的同文本不得误并（各保留独立桶）。
    #[test]
    fn 不同来源的同文本不被误并() {
        let fast = RecallResult {
            memories: vec![make_memory(
                "id1",
                "同一段完全相同的文本内容",
                Some("doc-a.md"),
                None,
            )],
            scores: vec![1.0],
            total: 1,
            regression_evidence: std::collections::HashMap::new(),
        };
        let deep = RecallResult {
            memories: vec![make_memory(
                "id2",
                "同一段完全相同的文本内容",
                Some("doc-b.md"),
                None,
            )],
            scores: vec![0.9],
            total: 1,
            regression_evidence: std::collections::HashMap::new(),
        };
        let fused = rrf_fuse_weighted(&fast, &deep, 10, RRF_DEFAULT_K, 1.0, 1.0);
        assert_eq!(fused.total_candidates, 2, "跨来源同文本应保持两条独立记忆");
        assert_eq!(fused.memories.len(), 2);
    }

    /// 同一路径内重复桶：只计首次出现的贡献与排名，避免分数累加但 rank 覆盖。
    #[test]
    fn 同路径重复桶只计一次贡献() {
        // fast 路径中同一 (source, 指纹) 桶出现两次（第 1、3 位）
        let fast = RecallResult {
            memories: vec![
                make_memory("dup-1", "Rust 编译失败 full", Some("doc.md"), None),
                make_memory("other", "无关记忆", Some("other.md"), None),
                make_memory("dup-2", "Rust 编译失败 full", Some("doc.md"), None),
            ],
            scores: vec![1.0, 0.5, 0.4],
            total: 3,
            regression_evidence: std::collections::HashMap::new(),
        };
        let deep = RecallResult {
            memories: vec![make_memory("deep-x", "无关深度记忆", Some("deep.md"), None)],
            scores: vec![0.8],
            total: 1,
            regression_evidence: std::collections::HashMap::new(),
        };
        let fused = rrf_fuse_weighted(&fast, &deep, 10, RRF_DEFAULT_K, 1.0, 1.0);
        // 只应出现 3 个唯一桶：dup、other、deep-x
        assert_eq!(fused.total_candidates, 3, "同路径重复桶应只聚为一个桶");
        // dup 桶（Rust 编译失败 full/doc.md）的融合分数应等于仅一次 fast 贡献
        let dup_key = fusion_key(&fast.memories[0]);
        let expected_dup_score = 1.0 / (RRF_DEFAULT_K + 1.0); // 第 1 位，未累积第 3 位
        let dup_score = fused
            .memories
            .iter()
            .zip(fused.scores.iter())
            .find(|(m, _)| fusion_key(m) == dup_key)
            .map(|(_, s)| *s);
        assert!(dup_score.is_some(), "dup 桶应出现在融合结果中");
        let dup_score = dup_score.unwrap();
        assert!(
            (dup_score - expected_dup_score).abs() < 1e-4,
            "重复桶分数应为单次贡献 {expected_dup_score}，实际 {dup_score}"
        );
        // rank 为 1 的贡献存在（深度路径的 deep-x 有 fast_rank=None，dup 桶 fast_rank=1）
        assert!(
            fused.contributions.iter().any(|c| c.fast_rank == Some(1)),
            "fast 路径第 1 位桶应有 rank=1 的贡献明细"
        );
    }

    /// 非法 RRF 参数应返回空结果而非 NaN 污染。
    #[test]
    fn 非法参数返回空结果() {
        let fast = RecallResult {
            memories: vec![make_memory("id", "内容", Some("a.md"), None)],
            scores: vec![1.0],
            total: 1,
            regression_evidence: std::collections::HashMap::new(),
        };
        let deep = RecallResult {
            memories: vec![],
            scores: vec![],
            total: 0,
            regression_evidence: std::collections::HashMap::new(),
        };
        let fused = rrf_fuse_weighted(&fast, &deep, 10, f32::NAN, 1.0, 1.0);
        assert!(fused.memories.is_empty(), "NaN rrf_k 应返回空结果");
        for score in fused.scores {
            assert!(score.is_finite(), "输出分数必须有限");
        }
    }

    /// 阶段D：贡献明细与 memories 平行，双路命中桶同时含两路排名与贡献分。
    #[test]
    fn 双路命中的桶贡献明细可观测() {
        let fast = RecallResult {
            memories: vec![
                make_memory(
                    "fast-a",
                    "Rust 编译失败 依赖缺失 更新建议",
                    Some("GUIDE.md"),
                    None,
                ),
                make_memory("fast-b", "干扰内容 B", Some("OTHER.md"), None),
            ],
            scores: vec![1.0, 0.8],
            total: 2,
            regression_evidence: std::collections::HashMap::new(),
        };
        let deep = RecallResult {
            memories: vec![
                make_memory(
                    "deep-a",
                    "Rust 编译失败 依赖缺失 更新建议",
                    Some("GUIDE.md"),
                    None,
                ),
                make_memory("deep-c", "深度命中的另一条", Some("DEEP.md"), None),
            ],
            scores: vec![0.95, 0.6],
            total: 2,
            regression_evidence: std::collections::HashMap::new(),
        };
        let (fw, dw) = (1.0, 1.8);
        let fused = rrf_fuse_weighted(&fast, &deep, 10, RRF_DEFAULT_K, fw, dw);
        assert_eq!(
            fused.memories.len(),
            fused.contributions.len(),
            "贡献明细必须与结果平行"
        );

        // 找到双路命中的桶（a 记忆所在桶）
        let idx = fused
            .memories
            .iter()
            .position(|m| m.content.contains("Rust 编译失败"))
            .expect("a 记忆应在融合结果中");
        let c = fused.contributions[idx];
        assert_eq!(c.fast_rank, Some(1), "a 在 fast 路径排第 1");
        assert_eq!(c.deep_rank, Some(1), "a 在 deep 路径排第 1");
        assert_eq!(c.hit_paths(), vec!["fast", "deep"], "双路均应命中");

        let expected_fast = fw / (RRF_DEFAULT_K + 1.0);
        let expected_deep = dw / (RRF_DEFAULT_K + 1.0);
        assert!(
            (c.fast_contrib - expected_fast).abs() < 1e-5,
            "fast 贡献分应等于 fw/(k+1)，实际 {}",
            c.fast_contrib
        );
        assert!(
            (c.deep_contrib - expected_deep).abs() < 1e-5,
            "deep 贡献分应等于 dw/(k+1)，实际 {}",
            c.deep_contrib
        );
        assert!(
            (c.fused_contrib() - (expected_fast + expected_deep)).abs() < 1e-5,
            "真实融合贡献分应为两路之和"
        );
    }

    /// 阶段D：单路命中的桶，另一路排名与贡献应为空/0。
    #[test]
    fn 无效参数返回空结果且不产生非有限分数() {
        let empty = RecallResult {
            memories: vec![],
            scores: vec![],
            total: 0,
            regression_evidence: std::collections::HashMap::new(),
        };
        for (k, fw, dw) in [
            (f32::NAN, 1.0, 1.0),
            (-1.0, 1.0, 1.0),
            (1.0, f32::INFINITY, 1.0),
            (1.0, 1.0, -1.0),
        ] {
            let fused = rrf_fuse_weighted(&empty, &empty, 10, k, fw, dw);
            assert!(fused.memories.is_empty());
            assert!(fused.scores.iter().all(|score| score.is_finite()));
        }
    }

    #[test]
    fn 分数长度不一致时结果仍保持对齐() {
        let fast = RecallResult {
            memories: vec![make_memory("a", "内容", None, None)],
            scores: vec![],
            total: 1,
            regression_evidence: std::collections::HashMap::new(),
        };
        let empty = RecallResult {
            memories: vec![],
            scores: vec![],
            total: 0,
            regression_evidence: std::collections::HashMap::new(),
        };
        let fused = rrf_fuse_weighted(&fast, &empty, 10, 60.0, 1.0, 1.0);
        assert_eq!(fused.memories.len(), fused.scores.len());
    }

    #[test]
    fn 单路命中的桶仅一路有贡献() {
        let fast = RecallResult {
            memories: vec![make_memory(
                "only-fast",
                "仅快速命中内容",
                Some("FAST.md"),
                None,
            )],
            scores: vec![1.0],
            total: 1,
            regression_evidence: std::collections::HashMap::new(),
        };
        let deep = RecallResult {
            memories: vec![],
            scores: vec![],
            total: 0,
            regression_evidence: std::collections::HashMap::new(),
        };
        let fused = rrf_fuse_weighted(&fast, &deep, 10, RRF_DEFAULT_K, 1.0, 1.8);
        let c = &fused.contributions[0];
        assert_eq!(c.fast_rank, Some(1));
        assert_eq!(c.deep_rank, None, "deep 未命中该桶");
        assert_eq!(c.deep_contrib, 0.0);
        assert_eq!(c.hit_paths(), vec!["fast"]);
    }
}
