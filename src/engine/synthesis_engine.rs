// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心算法，受研究许可证保护。
// ============================================================
//
// 合成引擎 (SynthesisEngine)
//
// 从 memory_store.rs 中提取的合成逻辑，降低单文件复杂度。
//
// 职责：
//   - 记忆簇发现（并查集聚类）
//   - 合成摘要生成（模板法）
//   - 洛书驱动递归合成（RecursiveCompose）
//   - 合成去重
//
// 不负责：
//   - 记忆存储（由 Persistence trait 负责）
//   - 检索（由 trapped_focus_recall / recall 负责）
//   - 调节（由 DaoRegulator 负责）
// ============================================================

use crate::engine::dao_metrics::DaoMetrics;
use crate::engine::luoshu_encoder::LuoShuVector;
use crate::engine::mirror_trapezoid::{mirror_project, recursive_compose};
use crate::engine::synthesis_journal::SynthesisJournal;
use crate::graph_store::{EdgeType, GraphMemoryStore};
use crate::memory_types::{Importance, Memory, MemoryType};
use crate::persistence::Persistence;

/// 合成引擎配置
#[derive(Debug, Clone)]
pub struct SynthesisConfig {
    /// 合成触发阈值：相似记忆数量达到此值时才触发合成（默认 3）
    pub min_cluster: usize,
    /// 合成相似度阈值：Jaccard 相似度超过此值归入同一簇（默认 0.4）
    pub similarity: f32,
}

impl Default for SynthesisConfig {
    fn default() -> Self {
        Self {
            min_cluster: 3,
            similarity: 0.4,
        }
    }
}

/// 合成引擎
///
/// 负责记忆的自动合成，支持两种模式：
/// 1. Jaccard 文本相似度聚类（try_synthesize）
/// 2. 洛书几何分类合成（luoshu_synthesize）
pub struct SynthesisEngine {
    config: SynthesisConfig,
}

impl SynthesisEngine {
    pub fn new(config: SynthesisConfig) -> Self {
        Self { config }
    }

    /// 计算 Jaccard 词集相似度（与 MemoryStore 中的实现一致）
    pub fn compute_jaccard(&self, a: &str, b: &str) -> f32 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();

        let has_cjk = a_lower
            .chars()
            .any(|c| (c as u32) >= 0x4E00 && (c as u32) <= 0x9FFF)
            || b_lower
                .chars()
                .any(|c| (c as u32) >= 0x4E00 && (c as u32) <= 0x9FFF);

        if has_cjk {
            let bigrams_a: std::collections::HashSet<String> = a_lower
                .chars()
                .collect::<Vec<_>>()
                .windows(2)
                .map(|w| format!("{}{}", w[0], w[1]))
                .collect();
            let bigrams_b: std::collections::HashSet<String> = b_lower
                .chars()
                .collect::<Vec<_>>()
                .windows(2)
                .map(|w| format!("{}{}", w[0], w[1]))
                .collect();

            if bigrams_a.is_empty() && bigrams_b.is_empty() {
                return 1.0;
            }
            let intersection = bigrams_a.intersection(&bigrams_b).count();
            let union = bigrams_a.union(&bigrams_b).count();
            if union == 0 {
                return 0.0;
            }
            intersection as f32 / union as f32
        } else {
            let words_a: std::collections::HashSet<&str> = a_lower.split_whitespace().collect();
            let words_b: std::collections::HashSet<&str> = b_lower.split_whitespace().collect();

            if words_a.is_empty() && words_b.is_empty() {
                return 1.0;
            }
            let intersection = words_a.intersection(&words_b).count();
            let union = words_a.union(&words_b).count();
            if union == 0 {
                return 0.0;
            }
            intersection as f32 / union as f32
        }
    }

    /// 查找相似记忆簇（用于递归合成）
    ///
    /// 使用并查集算法，将所有 Jaccard 相似度 ≥ config.similarity 的记忆
    /// 归入同一簇。返回所有大小 ≥ config.min_cluster 的簇。
    pub fn find_synthesis_clusters<P: Persistence>(
        &self,
        persistence: &P,
    ) -> Result<Vec<Vec<Memory>>, crate::persistence::PersistenceError> {
        let all = persistence.load_all_memories()?;

        let candidates: Vec<&Memory> = all
            .iter()
            .filter(|m| !m.is_expired() && m.memory_type != MemoryType::Synthesis)
            .collect();

        if candidates.len() < self.config.min_cluster {
            return Ok(Vec::new());
        }

        let n = candidates.len();
        let mut parent: Vec<usize> = (0..n).collect();
        let mut rank = vec![0usize; n];

        fn find(parent: &mut [usize], x: usize) -> usize {
            if parent[x] != x {
                parent[x] = find(parent, parent[x]);
            }
            parent[x]
        }

        fn union(parent: &mut [usize], rank: &mut [usize], x: usize, y: usize) {
            let rx = find(parent, x);
            let ry = find(parent, y);
            if rx == ry {
                return;
            }
            if rank[rx] < rank[ry] {
                parent[rx] = ry;
            } else if rank[rx] > rank[ry] {
                parent[ry] = rx;
            } else {
                parent[ry] = rx;
                rank[rx] += 1;
            }
        }

        for i in 0..n {
            for j in (i + 1)..n {
                let sim = self.compute_jaccard(&candidates[i].content, &candidates[j].content);
                if sim >= self.config.similarity {
                    union(&mut parent, &mut rank, i, j);
                }
            }
        }

        let mut groups: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();
        for i in 0..n {
            let root = find(&mut parent, i);
            groups.entry(root).or_default().push(i);
        }

        let clusters: Vec<Vec<Memory>> = groups
            .into_values()
            .filter(|indices| indices.len() >= self.config.min_cluster)
            .map(|indices| indices.into_iter().map(|i| candidates[i].clone()).collect())
            .collect();

        Ok(clusters)
    }

    /// 道枢映射: 坤卦·地 (☷) — 厚德载物，合成摘要是记忆凝练的土壤
    /// 构建合成摘要（模板法，不依赖 LLM）
    pub fn build_synthesis_summary(&self, cluster: &[Memory]) -> String {
        let type_counts: std::collections::HashMap<String, usize> = {
            let mut map = std::collections::HashMap::new();
            for m in cluster {
                *map.entry(m.memory_type.as_str().to_string()).or_insert(0) += 1;
            }
            map
        };

        let mut word_freq: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for m in cluster {
            let words: Vec<&str> = m.content.split_whitespace().collect();
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for w in words {
                if w.len() >= 3 && seen.insert(w) {
                    *word_freq.entry(w.to_lowercase()).or_insert(0) += 1;
                }
            }
        }
        let mut common_words: Vec<String> = word_freq
            .into_iter()
            .filter(|(_, c)| *c >= 2)
            .map(|(w, _)| w)
            .collect();
        common_words.sort();

        let snippets: Vec<String> = cluster
            .iter()
            .take(3)
            .map(|m| {
                let preview: String = m.content.chars().take(60).collect();
                let ellipsis = if m.content.chars().count() > 60 {
                    "…"
                } else {
                    ""
                };
                format!("- {}", preview.trim()) + ellipsis
            })
            .collect();

        let type_desc: Vec<String> = type_counts
            .iter()
            .map(|(t, c)| format!("{c} 条 {t}"))
            .collect();

        let mut summary = format!("「合成知识」{} 条相关记忆的融合结果。", cluster.len());
        if !type_desc.is_empty() {
            summary.push_str(&format!(" 类型分布：{}。", type_desc.join("，")));
        }
        if !common_words.is_empty() {
            summary.push_str(&format!(" 共同主题：{}。", common_words.join("、")));
        }
        summary.push('\n');
        summary.push_str(&snippets.join("\n"));

        summary
    }

    /// 道枢映射: 震卦·雷 (☳) — 震惊百里，合成簇如雷霆之后的新生
    /// 对单个记忆簇执行递归合成
    pub fn synthesize_cluster<P: Persistence>(
        &self,
        cluster: &[Memory],
        persistence: &P,
    ) -> Result<Memory, crate::persistence::PersistenceError> {
        let source_ids: Vec<String> = cluster.iter().map(|m| m.id.clone()).collect();

        let cluster_size = cluster.len() as f32;
        let avg_similarity: f32 = {
            let mut total = 0.0f32;
            let mut count = 0usize;
            for i in 0..cluster.len() {
                for j in (i + 1)..cluster.len() {
                    total += self.compute_jaccard(&cluster[i].content, &cluster[j].content);
                    count += 1;
                }
            }
            if count > 0 {
                total / count as f32
            } else {
                0.0
            }
        };
        let confidence = avg_similarity * (cluster_size.log2() / 5.0).min(1.0);

        let summary = self.build_synthesis_summary(cluster);

        let mut all_tags: Vec<String> = Vec::new();
        for m in cluster {
            for tag in &m.tags {
                if !all_tags.contains(tag) {
                    all_tags.push(tag.clone());
                }
            }
        }

        let max_importance = cluster
            .iter()
            .map(|m| m.importance)
            .max()
            .unwrap_or(Importance::new(7));

        let mut synthesis = Memory::new(
            summary,
            MemoryType::Synthesis,
            cluster.first().and_then(|m| m.project.clone()),
            all_tags,
            max_importance,
            None,
        );
        synthesis.source = Some("recursive_synthesis".into());
        synthesis.source_ids = source_ids;
        synthesis.confidence = Some(confidence);
        synthesis.information_gain = Some(avg_similarity); // Jaccard 合成以平均相似度作为信息增量
        synthesis.resolution = "synthesized".to_string(); // 质疑二：标记为合成记忆

        persistence.save_memory(&synthesis)?;
        Ok(synthesis)
    }

    /// 道枢映射: 震卦·雷 (☳) — 万物出乎震，合成如春雷唤醒新生，信息增益阈值是萌发的门槛
    ///
    /// 尝试执行 Jaccard 递归合成
    ///
    /// 返回本次新生成的合成记忆数量。
    pub fn try_synthesize<P: Persistence>(
        &self,
        persistence: &P,
        graph_store: &mut Option<GraphMemoryStore>,
        dao_metrics: &mut DaoMetrics,
    ) -> Result<usize, crate::persistence::PersistenceError> {
        let clusters = self.find_synthesis_clusters(persistence)?;
        if clusters.is_empty() {
            return Ok(0);
        }

        let all_memories = persistence.load_all_memories()?;
        let existing_sources: std::collections::HashSet<Vec<String>> = all_memories
            .iter()
            .filter(|m| m.memory_type == MemoryType::Synthesis && !m.source_ids.is_empty())
            .map(|m| {
                let mut ids = m.source_ids.clone();
                ids.sort();
                ids
            })
            .collect();

        let mut synthesized = 0usize;

        for cluster in &clusters {
            let mut cluster_ids: Vec<String> = cluster.iter().map(|m| m.id.clone()).collect();
            cluster_ids.sort();

            if existing_sources.contains(&cluster_ids) {
                continue;
            }

            match self.synthesize_cluster(cluster, persistence) {
                Ok(synthesis) => {
                    if let Some(ref mut graph) = graph_store {
                        for source_id in &synthesis.source_ids {
                            let _ = graph.add_edge(
                                &synthesis.id,
                                source_id,
                                EdgeType::SynthesizesFrom,
                                synthesis.confidence.unwrap_or(0.5),
                            );
                        }
                    }
                    synthesized += 1;
                    dao_metrics.record_composition();
                }
                Err(e) => {
                    eprintln!("[LRC] 合成失败: {}（簇大小={}）", e, cluster.len());
                }
            }
        }

        Ok(synthesized)
    }

    /// 洛书驱动递归合成（M.T.R. RecursiveCompose 增强版）
    ///
    /// 使用 MirrorProject 分类 + RecursiveCompose 门控融合。
    /// 返回新生成的合成记忆数量。
    ///
    /// `information_gain_threshold`：由 DaoRegulator 动态管理的防坍塌阈值（质疑一·活性），
    /// 替代之前的硬编码常量。当合成产物的信息增量低于此阈值时，阻止合成。
    pub fn luoshu_synthesize<P: Persistence>(
        &self,
        persistence: &P,
        graph_store: &mut Option<GraphMemoryStore>,
        dao_metrics: &mut DaoMetrics,
        synthesis_journal: &SynthesisJournal,
        information_gain_threshold: f32,
    ) -> Result<usize, crate::persistence::PersistenceError> {
        let all = persistence.load_all_memories()?;

        let candidates: Vec<&Memory> = all
            .iter()
            .filter(|m| {
                !m.is_expired()
                    && m.memory_type != MemoryType::Synthesis
                    && m.luoshu_vector.is_some()
            })
            .collect();

        if candidates.len() < self.config.min_cluster {
            return Ok(0);
        }

        let mut groups: std::collections::HashMap<u8, Vec<&Memory>> =
            std::collections::HashMap::new();
        for m in &candidates {
            if let Some(idx) = m.bagua_index {
                groups.entry(idx).or_default().push(m);
            }
        }

        let mut synthesized = 0usize;

        // 去重集合：提前构建，避免循环内重复构建（性能优化 P1-4）
        let existing_sources: std::collections::HashSet<Vec<String>> = all
            .iter()
            .filter(|m| m.memory_type == MemoryType::Synthesis && !m.source_ids.is_empty())
            .map(|m| {
                let mut ids = m.source_ids.clone();
                ids.sort();
                ids
            })
            .collect();

        for (bagua_idx, group) in &groups {
            if group.len() < self.config.min_cluster {
                continue;
            }

            let vectors: Vec<LuoShuVector> = group
                .iter()
                .filter_map(|m| m.luoshu_vector.map(|v| LuoShuVector { values: v }))
                .collect();

            if vectors.len() < self.config.min_cluster {
                continue;
            }

            let result = recursive_compose(&vectors);

            if result.confidence < 0.3 {
                continue;
            }

            // 质疑二：信息增量阈值检查（防止模式坍塌）
            // 信息增量过低表示合成只是"压缩冗余"而非"产生新知识"
            // 此时阻止合成，保持记忆空间的细节多样性
            //
            // 质疑一·活性：阈值由 DaoRegulator 动态管理，不再硬编码。
            // 调节器根据合成产物质量和系统健康指标自动微调此值。
            if result.information_gain < information_gain_threshold {
                eprintln!(
                    "[LRC·合成·守卫] 阻止八卦类别 {} 的合成：信息增量 {:.4} 低于阈值 {:.4}（疑似空洞抽象）",
                    bagua_idx, result.information_gain, information_gain_threshold
                );
                continue;
            }

            let source_ids: Vec<String> = group.iter().map(|m| m.id.clone()).collect();

            let mut cluster_ids = source_ids.clone();
            cluster_ids.sort();
            if existing_sources.contains(&cluster_ids) {
                continue;
            }

            let category = crate::engine::mirror_trapezoid::BAGUA_CATEGORIES
                .get(*bagua_idx as usize)
                .copied()
                .unwrap_or("未知");
            let summary = format!(
                "「洛书合成·{}」{} 条相关记忆的几何融合结果。",
                category,
                group.len()
            );

            let mut all_tags: Vec<String> = Vec::new();
            for m in group {
                for tag in &m.tags {
                    if !all_tags.contains(tag) {
                        all_tags.push(tag.clone());
                    }
                }
            }

            let mut synthesis = Memory::new(
                summary,
                MemoryType::Synthesis,
                group.first().and_then(|m| m.project.clone()),
                all_tags,
                Importance::new(8),
                None,
            );
            synthesis.source = Some("luoshu_recursive_compose".into());
            synthesis.source_ids = source_ids;
            synthesis.confidence = Some(result.confidence);
            synthesis.information_gain = Some(result.information_gain);
            synthesis.resolution = "synthesized".to_string(); // 质疑二：标记为合成记忆
            synthesis.luoshu_vector = Some(result.vector.values);

            let proj = mirror_project(&result.vector);
            synthesis.bagua_index = Some(proj.best_index as u8);
            synthesis.bagua_category = Some(proj.best_category.to_string());

            persistence.save_memory(&synthesis)?;

            if let Some(ref mut graph) = graph_store {
                for sid in &synthesis.source_ids {
                    let _ = graph.add_edge(
                        &synthesis.id,
                        sid,
                        EdgeType::SynthesizesFrom,
                        synthesis.confidence.unwrap_or(0.5),
                    );
                }
            }
            synthesized += 1;
            dao_metrics.record_composition();

            synthesis_journal.record_synthesis(
                synthesis.id.clone(),
                "luoshu_auto",
                category,
                *bagua_idx,
                cluster_ids.clone(),
                result.confidence,
                group.len(),
            );
        }

        Ok(synthesized)
    }
}

impl Default for SynthesisEngine {
    fn default() -> Self {
        Self::new(SynthesisConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_types::{Importance, Memory, MemoryType};
    use crate::persistence::json::JsonPersistence;
    use tempfile::TempDir;

    /// 创建测试用记忆
    fn make_memory(content: &str, tags: Vec<&str>) -> Memory {
        Memory::new(
            content.to_string(),
            MemoryType::Fact,
            Some("test".to_string()),
            tags.iter().map(|s| s.to_string()).collect(),
            Importance::new(5),
            None,
        )
    }

    /// 创建合成类型测试用记忆
    #[allow(dead_code)]
    fn make_synthesis_memory(source_ids: Vec<&str>) -> Memory {
        let mut m = Memory::new(
            "合成测试".to_string(),
            MemoryType::Synthesis,
            Some("test".to_string()),
            vec![],
            Importance::new(7),
            None,
        );
        m.source_ids = source_ids.iter().map(|s| s.to_string()).collect();
        m
    }

    #[test]
    fn test_config_defaults() {
        let config = SynthesisConfig::default();
        assert_eq!(config.min_cluster, 3);
        assert!(config.similarity >= 0.3 && config.similarity <= 0.5);
    }

    #[test]
    fn test_compute_jaccard_identical() {
        let engine = SynthesisEngine::default();
        let sim = engine.compute_jaccard("hello world", "hello world");
        assert!(
            (sim - 1.0).abs() < 0.001,
            "相同文本应返回 1.0，实际: {}",
            sim
        );
    }

    #[test]
    fn test_compute_jaccard_completely_different() {
        let engine = SynthesisEngine::default();
        let sim = engine.compute_jaccard("hello world", "foo bar baz");
        assert!(sim < 0.01, "完全不同的文本应接近 0，实际: {}", sim);
    }

    #[test]
    fn test_compute_jaccard_partial_overlap() {
        let engine = SynthesisEngine::default();
        let sim = engine.compute_jaccard("hello world foo", "hello world bar");
        assert!(
            sim > 0.4 && sim < 0.8,
            "部分重叠应在 0.4-0.8 之间，实际: {}",
            sim
        );
    }

    #[test]
    fn test_compute_jaccard_cjk() {
        let engine = SynthesisEngine::default();
        let sim = engine.compute_jaccard("你好世界", "你好世界");
        assert!(
            (sim - 1.0).abs() < 0.001,
            "相同 CJK 文本应返回 1.0，实际: {}",
            sim
        );

        let sim_diff = engine.compute_jaccard("你好世界", "再见朋友");
        assert!(
            sim_diff < 0.5,
            "不同 CJK 文本相似度应较低，实际: {}",
            sim_diff
        );
    }

    #[test]
    fn test_compute_jaccard_empty() {
        let engine = SynthesisEngine::default();
        let sim = engine.compute_jaccard("", "");
        assert!((sim - 1.0).abs() < 0.001, "两个空字符串应返回 1.0");
    }

    #[test]
    fn test_build_synthesis_summary_single_memory() {
        let engine = SynthesisEngine::default();
        let cluster = vec![make_memory("这是一条测试记忆", vec!["test"])];
        let summary = engine.build_synthesis_summary(&cluster);
        assert!(summary.contains("合成知识"));
        assert!(summary.contains("测试记忆"));
    }

    #[test]
    fn test_build_synthesis_summary_multi_memory() {
        let engine = SynthesisEngine::default();
        let cluster = vec![
            make_memory("项目上线需要准备文档", vec!["deploy"]),
            make_memory("部署文档已完成编写", vec!["deploy"]),
            make_memory("文档需要经过审核", vec!["review"]),
        ];
        let summary = engine.build_synthesis_summary(&cluster);
        assert!(summary.contains("合成知识"));
        assert!(summary.contains("3 条相关记忆"));
        // 共同主题应包含"文档"（出现 3 次）
        assert!(
            summary.contains("文档"),
            "应包含共同主题词，实际: {}",
            summary
        );
    }

    #[test]
    fn test_find_synthesis_clusters_empty() {
        let engine = SynthesisEngine::default();
        let dir = TempDir::new().expect("创建临时目录失败");
        let persistence =
            JsonPersistence::new(dir.path().to_str().unwrap()).expect("创建 JSON 持久化失败");
        let clusters = engine
            .find_synthesis_clusters(&persistence)
            .expect("查找簇失败");
        assert!(clusters.is_empty(), "空记忆库应返回空簇");
    }

    #[test]
    fn test_find_synthesis_clusters_below_min_cluster() {
        let engine = SynthesisEngine::default();
        let dir = TempDir::new().expect("创建临时目录失败");
        let persistence =
            JsonPersistence::new(dir.path().to_str().unwrap()).expect("创建 JSON 持久化失败");

        // 只添加 2 条记忆，低于 min_cluster=3
        persistence
            .save_memory(&make_memory("记忆 A", vec!["test"]))
            .expect("保存记忆 A 失败");
        persistence
            .save_memory(&make_memory("记忆 B", vec!["test"]))
            .expect("保存记忆 B 失败");

        let clusters = engine
            .find_synthesis_clusters(&persistence)
            .expect("查找簇失败");
        assert!(
            clusters.is_empty(),
            "低于最小簇大小时应返回空，实际: {} 簇",
            clusters.len()
        );
    }

    #[test]
    fn test_find_synthesis_clusters_similar() {
        let engine = SynthesisEngine::default();
        let dir = TempDir::new().expect("创建临时目录失败");
        let persistence =
            JsonPersistence::new(dir.path().to_str().unwrap()).expect("创建 JSON 持久化失败");

        // 添加 3 条相似记忆
        persistence
            .save_memory(&make_memory("部署到生产环境", vec!["deploy"]))
            .expect("保存失败");
        persistence
            .save_memory(&make_memory("生产环境部署步骤", vec!["deploy"]))
            .expect("保存失败");
        persistence
            .save_memory(&make_memory("部署到生产的方法", vec!["deploy"]))
            .expect("保存失败");

        let clusters = engine
            .find_synthesis_clusters(&persistence)
            .expect("查找簇失败");
        // 3 条相似记忆应形成至少 1 个簇
        assert!(!clusters.is_empty(), "相似记忆应形成簇");
    }

    #[test]
    fn test_try_synthesize_empty() {
        let engine = SynthesisEngine::default();
        let dir = TempDir::new().expect("创建临时目录失败");
        let persistence =
            JsonPersistence::new(dir.path().to_str().unwrap()).expect("创建 JSON 持久化失败");

        let mut graph_store: Option<GraphMemoryStore> = None;
        let mut dao_metrics = DaoMetrics::default();
        let count = engine
            .try_synthesize(&persistence, &mut graph_store, &mut dao_metrics)
            .expect("合成失败");
        assert_eq!(count, 0, "空记忆库应返回 0 条合成");
    }

    #[test]
    fn test_try_synthesize_with_similar() {
        let engine = SynthesisEngine::default();
        let dir = TempDir::new().expect("创建临时目录失败");
        let persistence =
            JsonPersistence::new(dir.path().to_str().unwrap()).expect("创建 JSON 持久化失败");

        // 添加 3 条相似记忆
        persistence
            .save_memory(&make_memory("Python 性能优化技巧", vec!["python"]))
            .expect("保存失败");
        persistence
            .save_memory(&make_memory("Python 代码优化方法", vec!["python"]))
            .expect("保存失败");
        persistence
            .save_memory(&make_memory("优化 Python 程序性能", vec!["python"]))
            .expect("保存失败");

        let mut graph_store: Option<GraphMemoryStore> = None;
        let mut dao_metrics = DaoMetrics::default();
        let count = engine
            .try_synthesize(&persistence, &mut graph_store, &mut dao_metrics)
            .expect("合成失败");
        // 相似记忆应触发合成
        assert!(count > 0, "相似记忆应触发合成，实际: {} 条", count);

        // 验证合成记忆已保存
        let all = persistence.load_all_memories().expect("加载失败");
        let synthesis_count = all
            .iter()
            .filter(|m| m.memory_type == MemoryType::Synthesis)
            .count();
        assert!(
            synthesis_count > 0,
            "应存在合成记忆，实际: {} 条",
            synthesis_count
        );
    }

    #[test]
    fn test_synthesize_is_idempotent() {
        let engine = SynthesisEngine::default();
        let dir = TempDir::new().expect("创建临时目录失败");
        let persistence =
            JsonPersistence::new(dir.path().to_str().unwrap()).expect("创建 JSON 持久化失败");

        // 添加 3 条相似记忆
        persistence
            .save_memory(&make_memory("Rust 所有权规则", vec!["rust"]))
            .expect("保存失败");
        persistence
            .save_memory(&make_memory("Rust 所有权和借用", vec!["rust"]))
            .expect("保存失败");
        persistence
            .save_memory(&make_memory("理解 Rust 所有权", vec!["rust"]))
            .expect("保存失败");

        let mut graph_store: Option<GraphMemoryStore> = None;
        let mut dao_metrics = DaoMetrics::default();

        // 第一次合成
        let count1 = engine
            .try_synthesize(&persistence, &mut graph_store, &mut dao_metrics)
            .expect("第一次合成失败");

        // 第二次合成 — 应不再产生新合成记忆（幂等）
        let count2 = engine
            .try_synthesize(&persistence, &mut graph_store, &mut dao_metrics)
            .expect("第二次合成失败");

        assert!(count1 > 0, "第一次应产生合成记忆");
        assert_eq!(
            count2, 0,
            "第二次合成应为幂等（无新簇），实际: {} 条",
            count2
        );
    }
}
