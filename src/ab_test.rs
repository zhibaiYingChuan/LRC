// ============================================================
// 许可证: Apache 2.0
// 本文件实现 A/B 测试框架与 MRR 评估，属于公开层 (Layer 1)。
// ============================================================
//
// A/B 测试框架（AB Test Framework）
//
// 对比"带幻和约束的九宫格检索"与"纯向量检索"的 MRR（平均倒数排名）。
//
// 核心组件：
//   1. SearchEngine trait — 检索引擎抽象
//   2. LuoshuSearchEngine — 基于洛书幻和约束的检索（实验组）
//   3. PlainVectorSearchEngine — 纯向量检索（对照组）
//   4. ABTestRunner — 协调实验运行和结果统计
//   5. mrr_evaluate — MRR 计算函数

#[cfg(feature = "ml")]
use crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder;
#[cfg(not(feature = "ml"))]
use crate::engine::luoshu_encoder::LuoShuEncoder as HybridLuoShuEncoder;
use crate::engine::mirror_trapezoid::mirror_project;
use crate::memory_types::Memory;
use serde::{Deserialize, Serialize};

// ==================== 检索引擎抽象 ====================

/// 检索引擎 trait
///
/// 不同的检索策略（洛书约束 vs 纯向量）实现此 trait，
/// 由 A/B 测试框架统一调度。
pub trait SearchEngine {
    /// 引擎名称
    fn name(&self) -> &str;

    /// 执行检索
    ///
    /// 返回按相关度排序的记忆 ID 列表（排名越靠前越相关）。
    fn search(&self, query: &str, top_k: usize) -> Vec<String>;
}

// ==================== 测试查询 ====================

/// 单个测试查询（包含查询文本和期望的正确答案 ID）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestQuery {
    /// 查询文本
    pub query: String,
    /// 期望排在首位的记忆 ID（正确答案）
    pub expected_id: String,
    /// 期望的相关记忆 ID 列表（用于计算 Recall@K）
    pub relevant_ids: Vec<String>,
    /// 查询类别（便于分组分析）
    pub category: Option<String>,
}

// ==================== 实验结果 ====================

/// 单个引擎的实验结果
#[derive(Debug, Clone, Serialize)]
pub struct EngineResult {
    /// 引擎名称
    pub engine_name: String,
    /// MRR（Mean Reciprocal Rank，平均倒数排名）
    pub mrr: f64,
    /// Recall@1（首位命中率）
    pub recall_at_1: f64,
    /// Recall@3
    pub recall_at_3: f64,
    /// Recall@5
    pub recall_at_5: f64,
    /// 平均排名（期望答案在结果中的平均位置，越低越好）
    pub avg_rank: f64,
    /// 查询总数
    pub total_queries: usize,
    /// 至少命中一条相关记忆的查询数
    pub queries_with_hits: usize,
    /// 每个查询的倒数排名详情
    pub per_query: Vec<PerQueryResult>,
}

/// 单个查询的详细结果
#[derive(Debug, Clone, Serialize)]
pub struct PerQueryResult {
    /// 查询文本
    pub query: String,
    /// 期望答案的排名（1-based，0 表示未找到）
    pub rank: usize,
    /// 倒数排名（1/rank，未找到为 0.0）
    pub reciprocal_rank: f64,
    /// 返回结果中命中的相关记忆数
    pub hits: usize,
}

/// A/B 测试比较结果
#[derive(Debug, Clone, Serialize)]
pub struct ABTestReport {
    /// 实验组（洛书约束）结果
    pub experiment: EngineResult,
    /// 对照组（纯向量）结果
    pub baseline: EngineResult,
    /// MRR 提升百分比
    pub mrr_improvement_pct: f64,
    /// Recall@1 提升百分比
    pub recall1_improvement_pct: f64,
    /// 是否达到统计显著性（简化判断：提升 > 0%）
    pub is_significant: bool,
    /// 测试时间戳
    pub timestamp: String,
    /// 测试配置摘要
    pub config: ABTestConfig,
}

/// A/B 测试配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ABTestConfig {
    /// 检索结果数
    pub top_k: usize,
    /// 测试查询总数
    pub query_count: usize,
    /// 实验组描述
    pub experiment_desc: String,
    /// 对照组描述
    pub baseline_desc: String,
}

// ==================== MRR 计算函数 ====================

/// 计算 MRR（Mean Reciprocal Rank，平均倒数排名）
///
/// MRR = (1/N) * Σ(1/rank_i)
/// 其中 rank_i 是第 i 个查询的正确答案在结果中的排名。
/// 如果正确答案不在结果中，贡献为 0。
///
/// # 参数
/// - `query_results`: 每个查询的检索结果（按排名排序的记忆 ID 列表）
/// - `expected_ids`: 每个查询的期望正确答案 ID
///
/// # 返回
/// - `mrr`: 平均倒数排名
/// - `per_query_rr`: 每个查询的倒数排名
pub fn mrr_evaluate(
    query_results: &[Vec<String>],
    expected_ids: &[String],
) -> (f64, Vec<f64>) {
    assert_eq!(
        query_results.len(),
        expected_ids.len(),
        "查询结果数与期望 ID 数必须一致"
    );

    let mut reciprocal_ranks = Vec::with_capacity(query_results.len());

    for (i, results) in query_results.iter().enumerate() {
        let expected = &expected_ids[i];
        let rank = results
            .iter()
            .position(|id| id == expected)
            .map(|pos| pos + 1) // 转为 1-based 排名
            .unwrap_or(0);

        let rr = if rank > 0 {
            1.0 / rank as f64
        } else {
            0.0
        };
        reciprocal_ranks.push(rr);
    }

    let mrr = if reciprocal_ranks.is_empty() {
        0.0
    } else {
        reciprocal_ranks.iter().sum::<f64>() / reciprocal_ranks.len() as f64
    };

    (mrr, reciprocal_ranks)
}

/// 计算 Recall@K
///
/// Recall@K = (在 top-K 结果中命中的相关记忆数) / (总相关记忆数)
pub fn recall_at_k(
    query_results: &[Vec<String>],
    relevant_ids: &[Vec<String>],
    k: usize,
) -> f64 {
    assert_eq!(
        query_results.len(),
        relevant_ids.len(),
        "查询结果数与相关 ID 数必须一致"
    );

    if query_results.is_empty() {
        return 0.0;
    }

    let total_recall: f64 = query_results
        .iter()
        .zip(relevant_ids.iter())
        .map(|(results, relevant)| {
            if relevant.is_empty() {
                return 1.0; // 无相关记忆视为完美命中
            }
            let hits = results
                .iter()
                .take(k)
                .filter(|id| relevant.contains(id))
                .count();
            hits as f64 / relevant.len() as f64
        })
        .sum();

    total_recall / query_results.len() as f64
}

// ==================== A/B 测试运行器 ====================

/// A/B 测试运行器
///
/// 协调实验组和对照组的检索评估，生成比较报告。
pub struct ABTestRunner {
    config: ABTestConfig,
}

impl ABTestRunner {
    /// 创建新的 A/B 测试运行器
    pub fn new(config: ABTestConfig) -> Self {
        Self { config }
    }

    /// 运行 A/B 测试
    ///
    /// # 参数
    /// - `experiment`: 实验组检索引擎（洛书约束）
    /// - `baseline`: 对照组检索引擎（纯向量）
    /// - `queries`: 测试查询列表
    ///
    /// # 返回
    /// A/B 测试比较报告
    pub fn run(
        &self,
        experiment: &dyn SearchEngine,
        baseline: &dyn SearchEngine,
        queries: &[TestQuery],
    ) -> ABTestReport {
        let top_k = self.config.top_k;

        // 收集期望 ID 和相关 ID
        let expected_ids: Vec<String> =
            queries.iter().map(|q| q.expected_id.clone()).collect();
        let relevant_ids: Vec<Vec<String>> =
            queries.iter().map(|q| q.relevant_ids.clone()).collect();

        // 实验组检索
        let exp_results: Vec<Vec<String>> = queries
            .iter()
            .map(|q| experiment.search(&q.query, top_k))
            .collect();

        // 对照组检索
        let base_results: Vec<Vec<String>> = queries
            .iter()
            .map(|q| baseline.search(&q.query, top_k))
            .collect();

        // 计算实验组指标
        let (exp_mrr, exp_rr) = mrr_evaluate(&exp_results, &expected_ids);
        let exp_recall_1 = recall_at_k(&exp_results, &relevant_ids, 1);
        let exp_recall_3 = recall_at_k(&exp_results, &relevant_ids, 3);
        let exp_recall_5 = recall_at_k(&exp_results, &relevant_ids, 5);

        let exp_avg_rank = expected_ids
            .iter()
            .zip(exp_results.iter())
            .map(|(expected, results)| {
                results
                    .iter()
                    .position(|id| id == expected)
                    .map(|pos| (pos + 1) as f64)
                    .unwrap_or(top_k as f64 + 1.0) // 未找到使用 top_k+1
            })
            .sum::<f64>()
            / expected_ids.len().max(1) as f64;

        let exp_hits = expected_ids
            .iter()
            .zip(exp_results.iter())
            .filter(|(expected, results)| results.contains(expected))
            .count();

        // 计算对照组指标
        let (base_mrr, base_rr) = mrr_evaluate(&base_results, &expected_ids);
        let base_recall_1 = recall_at_k(&base_results, &relevant_ids, 1);
        let base_recall_3 = recall_at_k(&base_results, &relevant_ids, 3);
        let base_recall_5 = recall_at_k(&base_results, &relevant_ids, 5);

        let base_avg_rank = expected_ids
            .iter()
            .zip(base_results.iter())
            .map(|(expected, results)| {
                results
                    .iter()
                    .position(|id| id == expected)
                    .map(|pos| (pos + 1) as f64)
                    .unwrap_or(top_k as f64 + 1.0)
            })
            .sum::<f64>()
            / expected_ids.len().max(1) as f64;

        let base_hits = expected_ids
            .iter()
            .zip(base_results.iter())
            .filter(|(expected, results)| results.contains(expected))
            .count();

        // 构建详细结果
        let exp_per_query: Vec<PerQueryResult> = queries
            .iter()
            .zip(exp_results.iter())
            .zip(exp_rr.iter())
            .zip(relevant_ids.iter())
            .map(|(((q, results), &rr), relevant)| {
                let rank = results
                    .iter()
                    .position(|id| id == &q.expected_id)
                    .map(|pos| pos + 1)
                    .unwrap_or(0);
                let hits = results
                    .iter()
                    .filter(|id| relevant.contains(id))
                    .count();
                PerQueryResult {
                    query: q.query.clone(),
                    rank,
                    reciprocal_rank: rr,
                    hits,
                }
            })
            .collect();

        let base_per_query: Vec<PerQueryResult> = queries
            .iter()
            .zip(base_results.iter())
            .zip(base_rr.iter())
            .zip(relevant_ids.iter())
            .map(|(((q, results), &rr), relevant)| {
                let rank = results
                    .iter()
                    .position(|id| id == &q.expected_id)
                    .map(|pos| pos + 1)
                    .unwrap_or(0);
                let hits = results
                    .iter()
                    .filter(|id| relevant.contains(id))
                    .count();
                PerQueryResult {
                    query: q.query.clone(),
                    rank,
                    reciprocal_rank: rr,
                    hits,
                }
            })
            .collect();

        // 计算提升百分比
        let mrr_improvement = if base_mrr > 0.0 {
            ((exp_mrr - base_mrr) / base_mrr) * 100.0
        } else if exp_mrr > 0.0 {
            100.0
        } else {
            0.0
        };

        let recall1_improvement = if base_recall_1 > 0.0 {
            ((exp_recall_1 - base_recall_1) / base_recall_1) * 100.0
        } else if exp_recall_1 > 0.0 {
            100.0
        } else {
            0.0
        };

        ABTestReport {
            experiment: EngineResult {
                engine_name: experiment.name().to_string(),
                mrr: exp_mrr,
                recall_at_1: exp_recall_1,
                recall_at_3: exp_recall_3,
                recall_at_5: exp_recall_5,
                avg_rank: exp_avg_rank,
                total_queries: queries.len(),
                queries_with_hits: exp_hits,
                per_query: exp_per_query,
            },
            baseline: EngineResult {
                engine_name: baseline.name().to_string(),
                mrr: base_mrr,
                recall_at_1: base_recall_1,
                recall_at_3: base_recall_3,
                recall_at_5: base_recall_5,
                avg_rank: base_avg_rank,
                total_queries: queries.len(),
                queries_with_hits: base_hits,
                per_query: base_per_query,
            },
            mrr_improvement_pct: mrr_improvement,
            recall1_improvement_pct: recall1_improvement,
            is_significant: mrr_improvement > 0.0,
            timestamp: chrono::Utc::now().to_rfc3339(),
            config: self.config.clone(),
        }
    }
}

// ==================== 洛书约束检索引擎（实验组） ====================

/// 基于洛书幻和约束的检索引擎
///
/// 使用洛书 9 维向量 + 八卦分类实现几何检索。
pub struct LuoshuSearchEngine {
    /// 记忆存储引用（通过传入所有记忆实现）
    memories: Vec<Memory>,
    /// 洛书编码器
    encoder: HybridLuoShuEncoder,
}

impl LuoshuSearchEngine {
    /// 创建洛书检索引擎
    pub fn new(memories: Vec<Memory>) -> Self {
        Self {
            memories,
            encoder: HybridLuoShuEncoder::default(),
        }
    }
}

impl SearchEngine for LuoshuSearchEngine {
    fn name(&self) -> &str {
        "洛书约束检索 (Luoshu-Constrained)"
    }

    fn search(&self, query: &str, top_k: usize) -> Vec<String> {
        // 对查询进行洛书编码
        let query_vec = self.encoder.encode_text(query);
        let query_proj = mirror_project(&query_vec);

        let mut scored: Vec<(f64, &Memory)> = self
            .memories
            .iter()
            .filter(|m| !m.is_expired())
            .map(|m| {
                let mut score: f64 = 0.0;

                // 1. 洛书向量余弦相似度（几何距离）
                if let Some(ref mem_vec) = m.luoshu_vector {
                    let dot: f64 = query_vec
                        .values
                        .iter()
                        .zip(mem_vec.iter())
                        .map(|(a, b)| *a as f64 * *b as f64)
                        .sum();
                    let q_norm: f64 = query_vec
                        .values
                        .iter()
                        .map(|v| (*v as f64).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    let m_norm: f64 = mem_vec
                        .iter()
                        .map(|v| (*v as f64).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    let cos_sim = if q_norm > 0.0 && m_norm > 0.0 {
                        dot / (q_norm * m_norm)
                    } else {
                        0.0
                    };
                    score += cos_sim * 0.5; // 几何相似度权重 50%
                }

                // 2. 八卦分类匹配（同类别加 30%）
                if let Some(mem_bagua) = m.bagua_index {
                    if mem_bagua == query_proj.best_index as u8 {
                        score += 0.3;
                    }
                }

                // 3. 中心值加权（核心记忆优先）
                if let Some(ref mem_vec) = m.luoshu_vector {
                    let center_val = query_vec.center_value();
                    let mem_center = mem_vec[4]; // 中心位置（索引 4）
                    // 中心值越接近，奖励越高
                    let center_sim = 1.0 - (center_val as f64 - mem_center as f64).abs();
                    score += center_sim * 0.1;
                }

                // 4. 文本关键词匹配（辅助信号）
                let query_lower = query.to_lowercase();
                let content_lower = m.content.to_lowercase();
                if content_lower.contains(&query_lower) {
                    score += 0.1;
                }
                for word in query_lower.split_whitespace() {
                    if content_lower.contains(word) {
                        score += 0.05;
                    }
                }

                (score, m)
            })
            .collect();

        // 按分数降序排序
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(top_k)
            .map(|(_, m)| m.id.clone())
            .collect()
    }
}

// ==================== 纯向量检索引擎（对照组） ====================

/// 纯向量检索（无洛书约束）
///
/// 仅使用文本关键词匹配，不依赖洛书几何约束。
/// 这是对照组（baseline），代表传统的向量检索方法。
pub struct PlainVectorSearchEngine {
    /// 记忆列表
    memories: Vec<Memory>,
}

impl PlainVectorSearchEngine {
    /// 创建纯向量检索引擎
    pub fn new(memories: Vec<Memory>) -> Self {
        Self { memories }
    }
}

impl SearchEngine for PlainVectorSearchEngine {
    fn name(&self) -> &str {
        "纯向量检索 (Plain Vector)"
    }

    fn search(&self, query: &str, top_k: usize) -> Vec<String> {
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(f64, &Memory)> = self
            .memories
            .iter()
            .filter(|m| !m.is_expired())
            .map(|m| {
                let content_lower = m.content.to_lowercase();
                let mut score: f64 = 0.0;

                // 完全匹配
                if content_lower.contains(&query_lower) {
                    score += 0.4;
                }

                // 词匹配
                for word in &query_words {
                    if content_lower.contains(word) {
                        score += 0.1;
                    }
                    for tag in &m.tags {
                        if tag.to_lowercase().contains(word) {
                            score += 0.15;
                        }
                    }
                }

                // 重要性加权
                score += m.importance.value() as f64 * 0.01;

                (score, m)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(top_k)
            .map(|(_, m)| m.id.clone())
            .collect()
    }
}

// ==================== 报告生成 ====================

/// 生成 A/B 测试的 Markdown 报告
pub fn format_report_markdown(report: &ABTestReport) -> String {
    let mut md = String::new();

    md.push_str("# A/B 测试报告：洛书约束检索 vs 纯向量检索\n\n");
    md.push_str(&format!(
        "**测试时间**: {}  \n**查询总数**: {}  \n**Top-K**: {}\n\n",
        report.timestamp,
        report.config.query_count,
        report.config.top_k,
    ));

    md.push_str("## 核心指标对比\n\n");
    md.push_str("| 指标 | 实验组 (洛书约束) | 对照组 (纯向量) | 提升 |\n");
    md.push_str("|------|:------:|:------:|:----:|\n");

    md.push_str(&format!(
        "| MRR | {:.4} | {:.4} | {:.1}% |\n",
        report.experiment.mrr,
        report.baseline.mrr,
        report.mrr_improvement_pct
    ));

    md.push_str(&format!(
        "| Recall@1 | {:.1}% | {:.1}% | {:.1}% |\n",
        report.experiment.recall_at_1 * 100.0,
        report.baseline.recall_at_1 * 100.0,
        report.recall1_improvement_pct
    ));

    md.push_str(&format!(
        "| Recall@3 | {:.1}% | {:.1}% | — |\n",
        report.experiment.recall_at_3 * 100.0,
        report.baseline.recall_at_3 * 100.0,
    ));

    md.push_str(&format!(
        "| Recall@5 | {:.1}% | {:.1}% | — |\n",
        report.experiment.recall_at_5 * 100.0,
        report.baseline.recall_at_5 * 100.0,
    ));

    md.push_str(&format!(
        "| 平均排名 | {:.2} | {:.2} | — |\n",
        report.experiment.avg_rank,
        report.baseline.avg_rank,
    ));

    md.push_str(&format!(
        "| 命中查询数 | {}/{} | {}/{} | — |\n\n",
        report.experiment.queries_with_hits,
        report.experiment.total_queries,
        report.baseline.queries_with_hits,
        report.baseline.total_queries,
    ));

    // 显著性判断
    if report.is_significant {
        md.push_str(&format!(
            "**结论**: 实验组 MRR 提升 {:.1}%，洛书约束检索在本次测试中表现优于纯向量检索。\n\n",
            report.mrr_improvement_pct
        ));
    } else {
        md.push_str("**结论**: 实验组 MRR 未显著提升，需要进一步优化洛书编码器或增加训练数据。\n\n");
    }

    md.push_str("## 逐查询详情\n\n");
    md.push_str("| 查询 | 实验排名 | 对照排名 | 实验 RR | 对照 RR |\n");
    md.push_str("|------|:------:|:------:|:------:|:------:|\n");

    for (exp, base) in report
        .experiment
        .per_query
        .iter()
        .zip(report.baseline.per_query.iter())
    {
        md.push_str(&format!(
            "| {} | {} | {} | {:.4} | {:.4} |\n",
            exp.query,
            if exp.rank > 0 {
                exp.rank.to_string()
            } else {
                "未找到".into()
            },
            if base.rank > 0 {
                base.rank.to_string()
            } else {
                "未找到".into()
            },
            exp.reciprocal_rank,
            base.reciprocal_rank,
        ));
    }

    md
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::luoshu_encoder::LuoShuEncoder;
    use crate::memory_types::{Importance, MemoryType};

    /// 创建测试用记忆
    fn make_test_memory(id: &str, content: &str) -> Memory {
        let mut m = Memory::new(
            content.to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        m.id = id.to_string();
        m
    }

    /// 创建带洛书编码的测试记忆
    fn make_encoded_memory(id: &str, content: &str, encoder: &LuoShuEncoder) -> Memory {
        let mut m = make_test_memory(id, content);
        let luoshu_vec = encoder.encode_text(content);
        let proj = mirror_project(&luoshu_vec);
        m.luoshu_vector = Some(luoshu_vec.values);
        m.bagua_index = Some(proj.best_index as u8);
        m.bagua_category = Some(proj.best_category.to_string());
        m
    }

    #[test]
    fn test_mrr_perfect() {
        let results = vec![
            vec!["a".into(), "b".into(), "c".into()],
            vec!["d".into(), "e".into(), "f".into()],
        ];
        let expected = vec!["a".into(), "d".into()];
        let (mrr, rr) = mrr_evaluate(&results, &expected);
        assert!((mrr - 1.0).abs() < 0.001, "完美 MRR 应为 1.0: {}", mrr);
        assert_eq!(rr, vec![1.0, 1.0]);
    }

    #[test]
    fn test_mrr_partial() {
        let results = vec![
            vec!["b".into(), "a".into(), "c".into()], // a 在位置 2
            vec!["d".into(), "e".into(), "f".into()], // d 在位置 1
            vec!["x".into(), "y".into(), "z".into()], // g 未找到
        ];
        let expected = vec!["a".into(), "d".into(), "g".into()];
        let (mrr, rr) = mrr_evaluate(&results, &expected);
        // RR: 1/2=0.5, 1/1=1.0, 0.0 → avg = 1.5/3 = 0.5
        assert!((mrr - 0.5).abs() < 0.001, "MRR 应为 0.5: {}", mrr);
        assert!((rr[0] - 0.5).abs() < 0.001);
        assert!((rr[1] - 1.0).abs() < 0.001);
        assert!((rr[2] - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_recall_at_k() {
        let results = vec![
            vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            vec!["x".into(), "y".into(), "a".into(), "z".into(), "w".into()],
        ];
        let relevant = vec![
            vec!["a".into(), "b".into()], // 2 个相关
            vec!["a".into(), "b".into(), "c".into()], // 3 个相关
        ];

        let r1 = recall_at_k(&results, &relevant, 1);
        // Q1: 1/2=0.5, Q2: 0/3=0.0 → avg = 0.25
        assert!((r1 - 0.25).abs() < 0.001, "Recall@1 应为 0.25: {}", r1);

        let r3 = recall_at_k(&results, &relevant, 3);
        // Q1: 2/2=1.0, Q2: 1/3=0.333 → avg = 0.667
        assert!((r3 - 0.666).abs() < 0.01, "Recall@3 应为 0.667: {}", r3);
    }

    #[test]
    fn test_luoshu_search_engine() {
        let encoder = LuoShuEncoder::new();
        let memories = vec![
            make_encoded_memory("m1", "用户偏好使用 Rust 编程语言", &encoder),
            make_encoded_memory("m2", "项目使用 PostgreSQL 数据库", &encoder),
            make_encoded_memory("m3", "前端使用 React 框架开发", &encoder),
        ];

        let engine = LuoshuSearchEngine::new(memories);
        let results = engine.search("数据库", 3);
        assert!(!results.is_empty(), "应返回结果");
        // 最相关的结果应该是 m2（PostgreSQL）
        assert_eq!(results[0], "m2", "第一条应为数据库相关记忆");
    }

    #[test]
    fn test_plain_vector_search_engine() {
        let memories = vec![
            make_test_memory("m1", "用户偏好使用 Rust 编程语言"),
            make_test_memory("m2", "项目使用 PostgreSQL 数据库"),
            make_test_memory("m3", "前端使用 React 框架开发"),
        ];

        let engine = PlainVectorSearchEngine::new(memories);
        let results = engine.search("数据库", 3);
        assert!(!results.is_empty(), "应返回结果");
        assert_eq!(results[0], "m2", "第一条应为数据库相关记忆");
    }

    #[test]
    fn test_ab_test_runner() {
        let encoder = LuoShuEncoder::new();
        let memories = vec![
            make_encoded_memory("m1", "用户偏好使用 Rust 编程语言", &encoder),
            make_encoded_memory("m2", "项目使用 PostgreSQL 数据库", &encoder),
            make_encoded_memory("m3", "前端使用 React 框架开发", &encoder),
            make_encoded_memory("m4", "API 使用 Actix Web 框架", &encoder),
            make_encoded_memory("m5", "部署使用 Docker 容器化", &encoder),
        ];

        let queries = vec![
            TestQuery {
                query: "数据库技术".into(),
                expected_id: "m2".into(),
                relevant_ids: vec!["m2".into()],
                category: Some("技术栈".into()),
            },
            TestQuery {
                query: "前端框架".into(),
                expected_id: "m3".into(),
                relevant_ids: vec!["m3".into()],
                category: Some("技术栈".into()),
            },
            TestQuery {
                query: "部署方式".into(),
                expected_id: "m5".into(),
                relevant_ids: vec!["m5".into()],
                category: Some("运维".into()),
            },
        ];

        let config = ABTestConfig {
            top_k: 3,
            query_count: queries.len(),
            experiment_desc: "洛书约束检索".into(),
            baseline_desc: "纯向量检索".into(),
        };

        let runner = ABTestRunner::new(config);

        let experiment = LuoshuSearchEngine::new(memories.clone());
        let baseline = PlainVectorSearchEngine::new(memories);

        let report = runner.run(&experiment, &baseline, &queries);

        assert_eq!(report.experiment.total_queries, 3);
        assert_eq!(report.baseline.total_queries, 3);
        assert!(report.experiment.mrr >= 0.0, "MRR 应 >= 0");
        assert!(report.experiment.mrr <= 1.0, "MRR 应 <= 1");

        // 生成 Markdown 报告
        let md = format_report_markdown(&report);
        assert!(md.contains("A/B 测试报告"));
        assert!(md.contains("洛书约束"));
        assert!(md.contains("纯向量"));
    }
}