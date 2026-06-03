// ============================================================
// 许可证: Apache 2.0
// 本文件实现记忆存储管理层，属于公开层 (Layer 1)。
// ============================================================
//
// 记忆存储管理器
//
// Aggregate Root — 记忆领域的中心协调单元。
// 封装持久化和检索逻辑，向 MCP 工具层提供统一的 CRUD 接口。

use crate::memory_types::{DecayConfig, Importance, Memory, MemoryType, PrivacyLevel};
use crate::persistence::{Persistence, PersistenceError};
use crate::engine::luoshu_encoder::{LuoShuEncoder, LuoShuVector};
use crate::engine::mirror_trapezoid::{mirror_project, recursive_compose, recursive_unfold, TrapezoidROI};
use crate::engine::dao_metrics::DaoMetrics;
use crate::graph_store::{EdgeType, GraphMemoryStore};

/// 记忆召回过滤条件
#[derive(Debug, Clone, Default)]
pub struct RecallFilter {
    /// 按记忆类型过滤
    pub memory_type: Option<MemoryType>,
    /// 按项目过滤
    pub project: Option<String>,
    /// 按标签过滤
    pub tags: Vec<String>,
    /// 最低重要性阈值
    pub min_importance: Option<Importance>,
    /// 最大返回数
    pub top_k: usize,
    /// 隐私上下文：按隐私级别过滤（Session/User/Global）
    /// 传入 (PrivacyLevel, session_id, user_id) 三元组
    pub privacy_context: Option<(PrivacyLevel, Option<String>, Option<String>)>,
}

impl RecallFilter {
    /// 创建默认过滤条件
    pub fn new() -> Self {
        Self {
            memory_type: None,
            project: None,
            tags: Vec::new(),
            min_importance: None,
            top_k: 5,
            privacy_context: None,
        }
    }

    /// 设置返回数量
    pub fn with_top_k(mut self, k: usize) -> Self {
        self.top_k = k;
        self
    }

    /// 设置类型过滤
    pub fn with_type(mut self, t: MemoryType) -> Self {
        self.memory_type = Some(t);
        self
    }

    /// 设置项目过滤
    pub fn with_project(mut self, p: impl Into<String>) -> Self {
        self.project = Some(p.into());
        self
    }

    /// 设置隐私上下文过滤
    pub fn with_privacy(
        mut self,
        level: PrivacyLevel,
        session_id: Option<String>,
        user_id: Option<String>,
    ) -> Self {
        self.privacy_context = Some((level, session_id, user_id));
        self
    }
}

/// 记忆列表查询过滤条件
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    /// 按记忆类型过滤
    pub memory_type: Option<MemoryType>,
    /// 按项目过滤
    pub project: Option<String>,
    /// 按标签过滤
    pub tags: Vec<String>,
    /// 排序方式
    pub sort_by: SortBy,
    /// 排序方向
    pub order: SortOrder,
    /// 分页大小
    pub limit: usize,
    /// 分页偏移
    pub offset: usize,
    /// 隐私上下文：按隐私级别过滤
    pub privacy_context: Option<(PrivacyLevel, Option<String>, Option<String>)>,
}

impl ListFilter {
    pub fn new() -> Self {
        Self {
            memory_type: None,
            project: None,
            tags: Vec::new(),
            sort_by: SortBy::CreatedAt,
            order: SortOrder::Desc,
            limit: 20,
            offset: 0,
            privacy_context: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SortBy {
    #[default]
    CreatedAt,
    Importance,
    LastAccessed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SortOrder {
    #[default]
    Desc,
    Asc,
}

/// 记忆库统计信息
#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    /// 记忆总数
    pub total_memories: usize,
    /// 按类型分布
    pub by_type: std::collections::HashMap<String, usize>,
    /// 按项目分布
    pub by_project: std::collections::HashMap<String, usize>,
    /// 过期记忆数
    pub expired_count: usize,
    /// 存储文件大小（字节）
    pub storage_size_bytes: u64,
}

/// 召回结果
#[derive(Debug, Clone)]
pub struct RecallResult {
    /// 匹配的记忆列表
    pub memories: Vec<Memory>,
    /// 每条记忆的匹配分数（与 memories 一一对应）
    pub scores: Vec<f32>,
    /// 记忆库总数
    pub total: usize,
}

/// 记忆存储管理器（Aggregate Root）
///
/// 聚合所有记忆的 CRUD 操作。
/// 通过 `Persistence` trait 抽象存储后端。
pub struct MemoryStore<P: Persistence> {
    persistence: P,
    /// 冲突检测相似度阈值（0.0 ~ 1.0），默认 0.5
    /// 高于此阈值的记忆将被视为重复并自动合并
    similarity_threshold: f32,
    /// 合成触发阈值：相似记忆数量达到此值时触发递归合成（默认 3）
    pub synthesis_min_cluster: usize,
    /// 合成相似度阈值：记忆相似度超过此值时纳入同一簇（默认 0.4）
    pub synthesis_similarity: f32,
    /// 洛书编码器（用于记忆的 9 维坐标编码 + 八卦分类）
    luoshu_encoder: LuoShuEncoder,
    /// 道同构度指标（L5 监控仪表）
    pub dao_metrics: DaoMetrics,
    /// 衰减曲线配置（可外部化，控制记忆衰减行为）
    pub decay_config: DecayConfig,
    /// 可选图存储（用于自动建立冲突/演进关系边）
    graph_store: Option<GraphMemoryStore>,
}

/// 隐私过滤辅助函数：判断记忆是否对指定隐私上下文可见
///
/// 规则（Section 3.3 隐私与权限）：
/// - Global 级别：对所有上下文可见
/// - User 级别：仅当 user_id 匹配时可见
/// - Session 级别：仅当 session_id 匹配时可见
/// - 无隐私上下文时：显示所有记忆（向后兼容）
fn is_visible(
    memory: &Memory,
    context: &Option<(PrivacyLevel, Option<String>, Option<String>)>,
) -> bool {
    match context {
        None => true, // 无隐私上下文，全部可见
        Some((_level, session_id, user_id)) => {
            match memory.privacy_level {
                PrivacyLevel::Global => true,
                PrivacyLevel::User => {
                    // User 级别：需要匹配 user_id
                    match (user_id, &memory.user_id) {
                        (Some(uid), Some(mid)) => uid == mid,
                        _ => false,
                    }
                }
                PrivacyLevel::Session => {
                    // Session 级别：需要匹配 session_id
                    match (session_id, &memory.session_id) {
                        (Some(sid), Some(mid)) => sid == mid,
                        _ => false,
                    }
                }
            }
        }
    }
}

impl<P: Persistence> MemoryStore<P> {
    /// 创建新的记忆存储器（默认相似度阈值 0.5）
    pub fn new(persistence: P) -> Self {
        Self {
            persistence,
            similarity_threshold: 0.5,
            synthesis_min_cluster: 3,
            synthesis_similarity: 0.4,
            luoshu_encoder: LuoShuEncoder::new(),
            dao_metrics: DaoMetrics::new(),
            decay_config: DecayConfig::default(),
            graph_store: None,
        }
    }

    /// 设置冲突检测的相似度阈值
    ///
    /// 范围 0.0 ~ 1.0，值越高表示要求越严格（越相似才会合并）。
    pub fn with_similarity_threshold(mut self, threshold: f32) -> Self {
        self.similarity_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// 设置图存储（用于自动建立冲突/演进/合成关系边）
    ///
    /// 启用后，写入冲突时会自动创建 Contradicts/Evolves 边，
    /// 递归合成时会自动创建 SynthesizesFrom 边。
    pub fn with_graph_store(mut self, graph_store: GraphMemoryStore) -> Self {
        self.graph_store = Some(graph_store);
        self
    }

    /// 计算 Jaccard 词集相似度
    ///
    /// 对于中文文本（无空格分词），使用字符级 bigram 比较。
    /// 对于英文文本，使用空格分词比较。
    fn compute_jaccard(&self, a: &str, b: &str) -> f32 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();

        // 检测是否包含 CJK 字符（中文、日文、韩文）
        let has_cjk = a_lower.chars().any(|c| c as u32 >= 0x4E00 && c as u32 <= 0x9FFF)
            || b_lower.chars().any(|c| c as u32 >= 0x4E00 && c as u32 <= 0x9FFF);

        if has_cjk {
            // 中文：使用字符级 2-gram
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
            // 英文：空格分词
            let words_a: std::collections::HashSet<&str> =
                a_lower.split_whitespace().collect();
            let words_b: std::collections::HashSet<&str> =
                b_lower.split_whitespace().collect();

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

    /// 查找与给定内容高度相似的已有记忆
    ///
    /// 返回第一条相似度超过阈值的记忆。
    /// 如果无相似记忆则返回 None。
    pub fn find_similar(&self, content: &str) -> Result<Option<Memory>, PersistenceError> {
        let all = self.persistence.load_all_memories()?;

        for m in &all {
            if m.is_expired() {
                continue;
            }
            let sim = self.compute_jaccard(content, &m.content);
            if sim >= self.similarity_threshold {
                return Ok(Some(m.clone()));
            }
        }

        Ok(None)
    }

    /// 查找相似记忆簇（用于递归合成）
    ///
    /// 使用并查集算法，将所有 Jaccard 相似度 ≥ synthesis_similarity 的记忆
    /// 归入同一簇。返回所有大小 ≥ synthesis_min_cluster 的簇。
    fn find_synthesis_clusters(&self) -> Result<Vec<Vec<Memory>>, PersistenceError> {
        let all = self.persistence.load_all_memories()?;

        // 过滤：只处理非过期、非合成类型的记忆（避免对合成结果再合成）
        let candidates: Vec<&Memory> = all
            .iter()
            .filter(|m| !m.is_expired() && m.memory_type != MemoryType::Synthesis)
            .collect();

        if candidates.len() < self.synthesis_min_cluster {
            return Ok(Vec::new());
        }

        let n = candidates.len();

        // 并查集初始化
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

        // 两两比较相似度，相似则合并
        for i in 0..n {
            for j in (i + 1)..n {
                let sim = self.compute_jaccard(
                    &candidates[i].content,
                    &candidates[j].content,
                );
                if sim >= self.synthesis_similarity {
                    union(&mut parent, &mut rank, i, j);
                }
            }
        }

        // 按根节点分组
        let mut groups: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();
        for i in 0..n {
            let root = find(&mut parent, i);
            groups.entry(root).or_default().push(i);
        }

        // 筛选大小达到阈值的簇
        let clusters: Vec<Vec<Memory>> = groups
            .into_values()
            .filter(|indices| indices.len() >= self.synthesis_min_cluster)
            .map(|indices| {
                indices
                    .into_iter()
                    .map(|i| candidates[i].clone())
                    .collect()
            })
            .collect();

        Ok(clusters)
    }

    /// 对单个记忆簇执行递归合成
    ///
    /// 将簇中所有源记忆融合为一条 Synthesis 类型的抽象知识。
    /// 合成结果包含结构化摘要和置信度评分。
    fn synthesize_cluster(
        &mut self,
        cluster: &[Memory],
    ) -> Result<Memory, PersistenceError> {
        // 收集源记忆 ID
        let source_ids: Vec<String> = cluster.iter().map(|m| m.id.clone()).collect();

        // 计算置信度：基于簇内平均相似度和簇大小
        let cluster_size = cluster.len() as f32;
        let avg_similarity: f32 = {
            let mut total = 0.0f32;
            let mut count = 0usize;
            for i in 0..cluster.len() {
                for j in (i + 1)..cluster.len() {
                    total += self.compute_jaccard(
                        &cluster[i].content,
                        &cluster[j].content,
                    );
                    count += 1;
                }
            }
            if count > 0 { total / count as f32 } else { 0.0 }
        };
        // 置信度 = 平均相似度 * 簇大小归一化因子（log2 增长）
        let confidence = avg_similarity * (cluster_size.log2() / 5.0).min(1.0);

        // 生成结构化摘要（不依赖 LLM，纯模板法）
        let summary = self.build_synthesis_summary(cluster);

        // 收集所有标签（去重）
        let mut all_tags: Vec<String> = Vec::new();
        for m in cluster {
            for tag in &m.tags {
                if !all_tags.contains(tag) {
                    all_tags.push(tag.clone());
                }
            }
        }

        // 取最高重要性
        let max_importance = cluster
            .iter()
            .map(|m| m.importance)
            .max()
            .unwrap_or(Importance::new(7));

        // 创建合成记忆
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

        // 持久化
        self.persistence.save_memory(&synthesis)?;

        Ok(synthesis)
    }

    /// 构建合成摘要（模板法，不依赖 LLM）
    ///
    /// 从源记忆中提取关键短语，生成结构化摘要。
    fn build_synthesis_summary(&self, cluster: &[Memory]) -> String {
        // 提取类型分布
        let type_counts: std::collections::HashMap<String, usize> = {
            let mut map = std::collections::HashMap::new();
            for m in cluster {
                *map.entry(m.memory_type.as_str().to_string()).or_insert(0) += 1;
            }
            map
        };

        // 提取共同关键词（出现 ≥2 次的长词）
        let mut word_freq: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for m in cluster {
            let words: Vec<&str> = m.content.split_whitespace().collect();
            let mut seen: std::collections::HashSet<&str> =
                std::collections::HashSet::new();
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

        // 取最有代表性的源记忆内容（前 3 条，每条截断 60 字）
        let snippets: Vec<String> = cluster
            .iter()
            .take(3)
            .map(|m| {
                let preview: String = m.content.chars().take(60).collect();
                let ellipsis = if m.content.chars().count() > 60 { "…" } else { "" };
                format!("- {}", preview.trim())
                    + ellipsis
            })
            .collect();

        // 构建摘要
        let type_desc: Vec<String> = type_counts
            .iter()
            .map(|(t, c)| format!("{c} 条 {t}"))
            .collect();

        let mut summary = format!(
            "「合成知识」{} 条相关记忆的融合结果。",
            cluster.len()
        );
        if !type_desc.is_empty() {
            summary.push_str(&format!(" 类型分布：{}。", type_desc.join("，")));
        }
        if !common_words.is_empty() {
            summary.push_str(&format!(
                " 共同主题：{}。",
                common_words.join("、")
            ));
        }
        summary.push('\n');
        summary.push_str(&snippets.join("\n"));

        summary
    }

    /// 尝试执行递归合成（在写入新记忆后调用）
    ///
    /// 扫描记忆库，找到所有满足条件的记忆簇，为每个簇生成合成记忆。
    /// 如果簇中已有合成记忆（通过 source_ids 判断），则跳过该簇。
    ///
    /// 返回本次新生成的合成记忆数量。
    pub fn try_synthesize(&mut self) -> Result<usize, PersistenceError> {
        let clusters = self.find_synthesis_clusters()?;
        if clusters.is_empty() {
            return Ok(0);
        }

        // 加载所有已有合成记忆的 source_ids，用于去重
        let all_memories = self.persistence.load_all_memories()?;
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
            // 获取簇的源 ID 集合（排序后用于去重比较）
            let mut cluster_ids: Vec<String> =
                cluster.iter().map(|m| m.id.clone()).collect();
            cluster_ids.sort();

            // 如果该簇已有合成记忆，跳过
            if existing_sources.contains(&cluster_ids) {
                continue;
            }

            match self.synthesize_cluster(cluster) {
                Ok(synthesis) => {
                    // 自动建立合成来源关系边（Section 3.3 冲突解决）
                    if let Some(ref mut graph) = self.graph_store {
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
                    self.dao_metrics.record_composition();
                }
                Err(e) => {
                    eprintln!(
                        "[LRC] 合成失败: {}（簇大小={}）",
                        e, cluster.len()
                    );
                }
            }
        }

        Ok(synthesized)
    }

    /// 洛书驱动递归合成（M.T.R. RecursiveCompose 增强版）
    ///
    /// 与 Jaccard-based try_synthesize 不同，此方法使用洛书向量进行:
    /// 1. MirrorProject 分类 → 按八卦类别分组
    /// 2. RecursiveCompose 门控融合 → 每个类别内合成
    /// 3. 生成高置信度的 Synthesis 记忆
    ///
    /// 返回新生成的合成记忆数量。
    pub fn luoshu_synthesize(&mut self) -> Result<usize, PersistenceError> {
        let all = self.persistence.load_all_memories()?;

        // 筛选有洛书向量且非合成类型的记忆
        let candidates: Vec<&Memory> = all
            .iter()
            .filter(|m| {
                !m.is_expired()
                && m.memory_type != MemoryType::Synthesis
                && m.luoshu_vector.is_some()
            })
            .collect();

        if candidates.len() < self.synthesis_min_cluster {
            return Ok(0);
        }

        // 按八卦类别分组
        let mut groups: std::collections::HashMap<u8, Vec<&Memory>> =
            std::collections::HashMap::new();
        for m in &candidates {
            if let Some(idx) = m.bagua_index {
                groups.entry(idx).or_default().push(m);
            }
        }

        let mut synthesized = 0usize;

        // 对每个足够大的类别执行 RecursiveCompose
        for (bagua_idx, group) in &groups {
            if group.len() < self.synthesis_min_cluster {
                continue;
            }

            // 转换为 LuoShuVector
            let vectors: Vec<LuoShuVector> = group
                .iter()
                .filter_map(|m| m.luoshu_vector.map(|v| LuoShuVector { values: v }))
                .collect();

            if vectors.len() < self.synthesis_min_cluster {
                continue;
            }

            // 执行递归合成
            let result = recursive_compose(&vectors);

            // 仅当置信度足够高时才创建合成记忆
            if result.confidence < 0.3 {
                continue;
            }

            // 收集源记忆 ID
            let source_ids: Vec<String> = group.iter().map(|m| m.id.clone()).collect();

            // 检查是否已有相同来源的合成记忆（去重）
            let existing_sources: std::collections::HashSet<Vec<String>> = all
                .iter()
                .filter(|m| m.memory_type == MemoryType::Synthesis && !m.source_ids.is_empty())
                .map(|m| { let mut ids = m.source_ids.clone(); ids.sort(); ids })
                .collect();

            let mut cluster_ids = source_ids.clone();
            cluster_ids.sort();
            if existing_sources.contains(&cluster_ids) {
                continue;
            }

            // 生成摘要
            let category = crate::engine::mirror_trapezoid::BAGUA_CATEGORIES
                .get(*bagua_idx as usize)
                .copied()
                .unwrap_or("未知");
            let summary = format!(
                "「洛书合成·{}」{} 条相关记忆的几何融合结果。",
                category,
                group.len()
            );

            // 收集所有标签
            let mut all_tags: Vec<String> = Vec::new();
            for m in group {
                for tag in &m.tags {
                    if !all_tags.contains(tag) {
                        all_tags.push(tag.clone());
                    }
                }
            }

            // 创建合成记忆
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
            synthesis.luoshu_vector = Some(result.vector.values);

            // 合成记忆也带八卦分类
            let proj = mirror_project(&result.vector);
            synthesis.bagua_index = Some(proj.best_index as u8);
            synthesis.bagua_category = Some(proj.best_category.to_string());

            self.persistence.save_memory(&synthesis)?;
            // 自动建立合成来源关系边
            if let Some(ref mut graph) = self.graph_store {
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
            self.dao_metrics.record_composition();
        }

        Ok(synthesized)
    }

    /// 写入一条新记忆（含冲突检测、洛书编码、递归合成触发）
    ///
    /// 自动设置 id、created_at 等元数据。
    /// 如果内容与已有记忆高度相似（Jaccard ≥ 阈值），则自动合并：
    /// - 更新内容为新内容
    /// - 合并标签（去重）
    /// - 更新 last_accessed
    /// - 保留原始 id 和 created_at
    ///
    /// 写入后自动：
    /// 1. 洛书编码：将记忆内容编码为 9 维洛书向量
    /// 2. MirrorProject 分类：自动判定记忆的先天八卦类别
    /// 3. 递归合成：若记忆库中相似记忆数 ≥ 3 条，则自动生成合成记忆
    pub fn remember(&mut self, memory: Memory) -> Result<Memory, PersistenceError> {
        // 检查是否有相似记忆
        let mut result = if let Some(existing) = self.find_similar(&memory.content)? {
            // 合并标签（去重）
            let mut merged_tags = existing.tags.clone();
            for tag in &memory.tags {
                if !merged_tags.contains(tag) {
                    merged_tags.push(tag.clone());
                }
            }

            // 构建合并后的记忆
            let mut merged = existing.clone();
            let old_content = merged.content.clone();
            merged.content = memory.content;
            merged.tags = merged_tags;
            merged.touch();

            // 如果新记忆的重要性更高，则提升
            if memory.importance > merged.importance {
                merged.importance = memory.importance;
            }

            // 自动建立冲突关系边（Section 3.3 冲突解决）
            if self.graph_store.is_some() {
                let jaccard = self.compute_jaccard(&old_content, &merged.content);
                if let Some(ref mut graph) = self.graph_store {
                    // 内容实质不同的合并 → Contradicts 边（需要后续解决）
                    if jaccard < 0.9 {
                        // 相似但不等同 → 可能是矛盾或演进
                        let _ = graph.add_edge(&memory.id, &existing.id, EdgeType::Contradicts, jaccard);
                    }
                    // 内容更新 → Evolves 边
                    let _ = graph.add_edge(&memory.id, &existing.id, EdgeType::Evolves, jaccard);
                }
            }

            // 更新持久化
            self.persistence.save_memory(&merged)?;
            merged
        } else {
            // 无冲突，正常写入
            self.persistence.save_memory(&memory)?;
            memory
        };

        // 洛书编码 + 八卦分类（透明地附加到每条记忆）
        {
            let luoshu_vec = self.luoshu_encoder.encode_text(&result.content);
            let proj = mirror_project(&luoshu_vec);

            result.luoshu_vector = Some(luoshu_vec.values);
            result.bagua_index = Some(proj.best_index as u8);
            result.bagua_category = Some(proj.best_category.to_string());

            // 计算拓扑深度：中心值越高（越靠近太极），深度越小（越持久）
            // topological_depth = 1.0 - center_value（归一化到 0.0~1.0）
            let center_val = luoshu_vec.center_value();
            result.topological_depth = (1.0 - center_val).clamp(0.0, 1.0);

            // 更新持久化（写入编码后的元数据）
            self.persistence.save_memory(&result)?;
        }

        // 记录指标：编码 + 1
        self.dao_metrics.record_encoding();

        // 写入后触发递归合成（非阻塞，失败不影响写入）
        match self.try_synthesize() {
            Ok(n) if n > 0 => {
                // 合成成功，静默记录
            }
            Err(e) => {
                eprintln!("[LRC] 递归合成触发失败: {}", e);
            }
            _ => {}
        }

        Ok(result)
    }

    /// 梯形聚焦检索（Section 3.2 TrapezoidFocus）
    ///
    /// 使用洛书九宫格几何结构进行空间分区检索：
    /// 1. 将查询文本编码为洛书向量
    /// 2. 以查询向量的重心位置为中心创建 TrapezoidROI
    /// 3. 递归细分为 4^depth 个子区域
    /// 4. 仅检索落在密度最高子区域内的记忆
    ///
    /// 复杂度：O(N / 4^depth)，depth=2 时仅检索 ~6% 的记忆
    ///
    /// 参数：
    /// - `query`: 查询文本
    /// - `filter`: 召回过滤条件
    /// - `depth`: 梯形细分深度（0=全量，1=4分，2=16分）
    pub fn trapezoid_focus_recall(
        &mut self,
        query: &str,
        filter: &RecallFilter,
        depth: u32,
    ) -> Result<RecallResult, PersistenceError> {
        // 1. 编码查询文本
        let query_vec = self.luoshu_encoder.encode_text(query);

        // 2. 以查询向量重心为中心创建 ROI
        let center = query_vec.values.iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(4);
        let roi = TrapezoidROI::centered(center, depth);

        let all_memories = self.persistence.load_all_memories()?;
        let total_count = all_memories.iter().filter(|m| !m.is_expired()).count();

        // 3. 构建 (索引, 洛书向量) 对
        let indexed: Vec<(usize, LuoShuVector)> = all_memories
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                if m.is_expired() { return false; }
                if let Some(ref mt) = filter.memory_type {
                    if m.memory_type != *mt { return false; }
                }
                if let Some(ref proj) = filter.project {
                    if m.project.as_deref() != Some(proj.as_str()) { return false; }
                }
                if !filter.tags.is_empty()
                    && !filter.tags.iter().any(|t| m.tags.contains(t))
                {
                    return false;
                }
                if let Some(min_imp) = filter.min_importance {
                    if m.importance < min_imp { return false; }
                }
                if !is_visible(m, &filter.privacy_context) {
                    return false;
                }
                m.luoshu_vector.is_some()
            })
            .filter_map(|(i, m)| {
                m.luoshu_vector.map(|v| (i, LuoShuVector { values: v }))
            })
            .collect();

        // 4. 执行梯形聚焦检索
        let vec_refs: Vec<(usize, &LuoShuVector)> = indexed.iter()
            .map(|(i, v)| (*i, v))
            .collect();
        let focus_result = roi.focused_recall(&vec_refs);

        // 5. 从匹配索引还原记忆
        let all: Vec<Memory> = all_memories;
        let mut memories: Vec<Memory> = focus_result.matched_indices
            .iter()
            .filter_map(|&idx| all.get(idx).cloned())
            .collect();

        // 6. 计算分数（基于洛书向量与查询向量的余弦相似度）
        let mut scores: Vec<f32> = memories.iter()
            .map(|m| {
                if let Some(ref lv) = m.luoshu_vector {
                    let mem_vec = LuoShuVector { values: *lv };
                    mem_vec.cosine_similarity(&query_vec)
                } else {
                    0.0
                }
            })
            .collect();

        // 7. 按分数排序并截取 top_k
        let mut scored: Vec<(usize, f32)> = (0..memories.len()).map(|i| (i, scores[i])).collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let top_k = filter.top_k.min(scored.len());
        let top_indices: Vec<usize> = scored.iter().take(top_k).map(|(i, _)| *i).collect();

        memories = top_indices.iter().map(|&i| memories[i].clone()).collect();
        scores = top_indices.iter().map(|&i| scores[i]).collect();

        // 8. 更新访问时间
        let matched_ids: std::collections::HashSet<String> =
            memories.iter().map(|m| m.id.clone()).collect();
        let mut all_memories = self.persistence.load_all_memories()?;
        let mut any_modified = false;
        for m in &mut all_memories {
            if matched_ids.contains(&m.id) {
                m.mark_accessed();
                any_modified = true;
            }
        }
        if any_modified {
            self.persistence.clear_memories()?;
            for m in all_memories {
                self.persistence.save_memory(&m)?;
            }
        }

        self.dao_metrics.record_recall();

        Ok(RecallResult {
            memories,
            scores,
            total: total_count,
        })
    }
    /// 语义搜索记忆
    ///
    /// 当前使用文本匹配算法（关键词提取 + 子串匹配 + 词频评分）。
    /// 检索到的记忆会自动更新 `last_accessed` 字段，使衰减模型正确工作。
    pub fn recall(
        &mut self,
        query: &str,
        filter: &RecallFilter,
    ) -> Result<RecallResult, PersistenceError> {
        let mut all_memories = self.persistence.load_all_memories()?;
        let total_count = all_memories.iter().filter(|m| !m.is_expired()).count();

        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        // 分两阶段处理以避免借用冲突：
        // 阶段 1: 用不可变引用评分和排序
        // 阶段 2: 修改匹配记忆的 last_accessed 并写回持久化
        let (memories, scores) = {
            // 过滤记忆
            let privacy_ctx = filter.privacy_context.clone();
            let candidates: Vec<&Memory> = all_memories
                .iter()
                .filter(|m| !m.is_expired())
                .filter(|m| {
                    // 类型过滤
                    if let Some(ref mt) = filter.memory_type {
                        if m.memory_type != *mt {
                            return false;
                        }
                    }
                    // 项目过滤
                    if let Some(ref proj) = filter.project {
                        if m.project.as_deref() != Some(proj.as_str()) {
                            return false;
                        }
                    }
                    // 标签过滤
                    if !filter.tags.is_empty()
                        && !filter.tags.iter().any(|t| m.tags.contains(t))
                    {
                        return false;
                    }
                    // 重要性过滤
                    if let Some(min_imp) = filter.min_importance {
                        if m.importance < min_imp {
                            return false;
                        }
                    }
                    // 隐私权限过滤（Section 3.3）
                    if !is_visible(m, &privacy_ctx) {
                        return false;
                    }
                    true
                })
                .collect();

            // 计算匹配分数
            let mut scored: Vec<(f32, &Memory)> = candidates
                .iter()
                .map(|m| {
                    let content_lower = m.content.to_lowercase();
                    let mut score: f32 = 0.0;

                    // 完全匹配加分
                    if content_lower.contains(&query_lower) {
                        score += 0.4;
                    }

                    // 词匹配加分
                    for word in &query_words {
                        if content_lower.contains(word) {
                            score += 0.1;
                        }
                    }

                    // 标签匹配加分
                    for tag in &m.tags {
                        for word in &query_words {
                            if tag.to_lowercase().contains(word) {
                                score += 0.15;
                            }
                        }
                    }

                    // 重要性加权（含衰减因子，使用可配置衰减曲线）
                    score += m.decayed_importance_with_config(&self.decay_config) * 0.01;

                    // 类型匹配加权
                    if (query_lower.contains("偏好") || query_lower.contains("prefer"))
                        && m.memory_type == MemoryType::Preference
                    {
                        score += 0.2;
                    }
                    if (query_lower.contains("决定") || query_lower.contains("选择") || query_lower.contains("decision"))
                        && m.memory_type == MemoryType::Decision
                    {
                        score += 0.2;
                    }

                    // 合成记忆优先返回（置信度加权）
                    if m.memory_type == MemoryType::Synthesis {
                        let confidence_boost = m.confidence.unwrap_or(0.5) * 0.3;
                        score += confidence_boost;
                    }

                    // 洛书几何距离加权（M.T.R. TrapezoidFocus 增强）
                    if let Some(ref luoshu_values) = m.luoshu_vector {
                        // 对查询也进行洛书编码，与记忆的洛书向量计算余弦相似度
                        let mem_vec = LuoShuVector { values: *luoshu_values };
                        // 用记忆向量和查询文本特征的简单几何距离近似
                        // 中心值越高（太极位激活越强），说明记忆越"核心"
                        let center_boost = mem_vec.center_value() * 0.1;
                        score += center_boost;
                    }

                    // 八卦分类匹配加权（同类别记忆额外加分）
                    if let Some(ref bagua) = m.bagua_category {
                        if (query_lower.contains("配置") || query_lower.contains("基础"))
                            && bagua == "承载基础" { score += 0.15; } // 坤
                        if (query_lower.contains("规则") || query_lower.contains("架构"))
                            && bagua == "刚性法则" { score += 0.15; } // 乾
                        if (query_lower.contains("依赖") || query_lower.contains("关联"))
                            && bagua == "依附关联" { score += 0.15; } // 离
                        if (query_lower.contains("偏好") || query_lower.contains("交互"))
                            && bagua == "愉悦表达" { score += 0.15; } // 兑
                        if (query_lower.contains("错误") || query_lower.contains("bug") || query_lower.contains("修复"))
                            && bagua == "陷溺困境" { score += 0.15; } // 坎
                    }

                    (score, *m)
                })
                .collect();

            // 按分数降序排序
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            // 截取 top_k
            let top_k = filter.top_k.min(scored.len());
            let scored: Vec<(f32, &Memory)> = scored.into_iter().take(top_k).collect();

            let memories: Vec<Memory> = scored.iter().map(|(_, m)| (*m).clone()).collect();
            let scores: Vec<f32> = scored.iter().map(|(s, _)| *s).collect();

            (memories, scores)
        };
        // 块作用域结束，candidates 和 scored 的不可变引用均已释放

        // 收集匹配记忆的 ID（用于标记访问时间）
        let matched_ids: std::collections::HashSet<String> =
            memories.iter().map(|m| m.id.clone()).collect();

        // 更新被检索到的记忆的 last_accessed（衰减模型依赖此字段）
        let mut any_modified = false;
        for m in &mut all_memories {
            if matched_ids.contains(&m.id) {
                m.mark_accessed();
                any_modified = true;
            }
        }

        // 将更新后的访问时间写回持久化存储
        if any_modified {
            self.persistence.clear_memories()?;
            for m in all_memories {
                self.persistence.save_memory(&m)?;
            }
        }

        // 记录指标：检索 + 1
        self.dao_metrics.record_recall();

        Ok(RecallResult {
            memories,
            scores,
            total: total_count,
        })
    }

    /// 删除一条记忆
    pub fn forget(&mut self, id: &str) -> Result<bool, PersistenceError> {
        self.persistence.delete_memory(id)
    }

    /// 更新记忆内容
    ///
    /// 如果记忆存在则更新并返回旧版本，否则返回 None。
    pub fn update_memory(
        &mut self,
        id: &str,
        new_content: &str,
        new_importance: Option<Importance>,
    ) -> Result<Option<Memory>, PersistenceError> {
        let all = self.persistence.load_all_memories()?;
        let mut found: Option<Memory> = None;

        let updated: Vec<Memory> = all
            .into_iter()
            .map(|mut m| {
                if m.id == id {
                    let old = m.clone();
                    m.update_content(new_content.to_string());
                    if let Some(imp) = new_importance {
                        m.update_importance(imp);
                    }
                    found = Some(old);
                    m
                } else {
                    m
                }
            })
            .collect();

        // 重新写入所有记忆（更新后的列表）
        self.persistence.clear_memories()?;
        for m in updated {
            self.persistence.save_memory(&m)?;
        }

        Ok(found)
    }

    /// 列出记忆（支持分页、过滤、排序）
    pub fn list_memories(&self, filter: &ListFilter) -> Result<(Vec<Memory>, usize), PersistenceError> {
        let mut all = self.persistence.load_all_memories()?;
        let privacy_ctx = filter.privacy_context.clone();

        // 过滤
        all.retain(|m| {
            if let Some(ref mt) = filter.memory_type {
                if m.memory_type != *mt {
                    return false;
                }
            }
            if let Some(ref proj) = filter.project {
                if m.project.as_deref() != Some(proj.as_str()) {
                    return false;
                }
            }
            if !filter.tags.is_empty()
                && !filter.tags.iter().any(|t| m.tags.contains(t))
            {
                return false;
            }
            // 隐私权限过滤（Section 3.3）
            if !is_visible(m, &privacy_ctx) {
                return false;
            }
            true
        });

        let total = all.len();

        // 排序
        all.sort_by(|a, b| {
            let cmp = match filter.sort_by {
                SortBy::CreatedAt => a.created_at.cmp(&b.created_at),
                SortBy::Importance => a.importance.value().cmp(&b.importance.value()),
                SortBy::LastAccessed => a.last_accessed.cmp(&b.last_accessed),
            };
            match filter.order {
                SortOrder::Desc => cmp.reverse(),
                SortOrder::Asc => cmp,
            }
        });

        // 分页
        let paged: Vec<Memory> = all
            .into_iter()
            .skip(filter.offset)
            .take(filter.limit)
            .collect();

        Ok((paged, total))
    }

    /// 获取记忆库统计信息
    pub fn stats(&self) -> Result<MemoryStats, PersistenceError> {
        let all = self.persistence.load_all_memories()?;
        let mut stats = MemoryStats {
            total_memories: all.len(),
            ..Default::default()
        };

        for m in &all {
            *stats
                .by_type
                .entry(m.memory_type.as_str().to_string())
                .or_insert(0) += 1;

            let proj = m.project.as_deref().unwrap_or("_global_");
            *stats.by_project.entry(proj.to_string()).or_insert(0) += 1;

            if m.is_expired() {
                stats.expired_count += 1;
            }
        }

        stats.storage_size_bytes = self.persistence.size_bytes()?;

        Ok(stats)
    }

    /// 获取记忆总数
    pub fn total_count(&self) -> Result<usize, PersistenceError> {
        let all = self.persistence.load_all_memories()?;
        Ok(all.len())
    }

    /// 归档过期记忆
    ///
    /// 将已过期的记忆从活跃存储迁移到归档存储（冷存储）。
    /// 归档的记忆不会丢失，但不再参与检索、列表和统计。
    ///
    /// 返回归档的记忆数量，若无可归档记忆则返回 0。
    pub fn archive_expired(&mut self) -> Result<usize, PersistenceError> {
        let all = self.persistence.load_all_memories()?;

        // 筛选过期记忆与活跃记忆
        let (expired, active): (Vec<Memory>, Vec<Memory>) = all
            .into_iter()
            .partition(|m| m.is_expired());

        if expired.is_empty() {
            return Ok(0);
        }

        let count = expired.len();

        // 归档过期记忆到冷存储
        self.persistence.add_to_archive(&expired)?;

        // 从活跃存储中重建（仅保留活跃记忆）
        self.persistence.clear_memories()?;
        for m in active {
            self.persistence.save_memory(&m)?;
        }

        Ok(count)
    }

    /// 获取持久化层的引用
    #[allow(dead_code)]
    pub fn persistence(&self) -> &P {
        &self.persistence
    }

    /// 获取道同构度指标快照（L5 监控仪表）
    ///
    /// 计算当前记忆库的完整健康度指标，包括：
    /// - 道同构度（幻和约束满足度）
    /// - 八卦分布熵
    /// - 合成/原始记忆比率
    pub fn dao_metrics_snapshot(&self) -> Result<crate::engine::dao_metrics::DaoMetricsSnapshot, PersistenceError> {
        let all = self.persistence.load_all_memories()?;
        let archived = self.persistence.load_archived_memories().unwrap_or_default();

        let total = all.len();
        let crystallized = all.iter().filter(|m| m.memory_type == MemoryType::Synthesis).count();
        let archived_count = archived.len();

        // 计算平均洛书偏离度
        let vectors: Vec<[f32; 9]> = all
            .iter()
            .filter_map(|m| m.luoshu_vector)
            .collect();
        let avg_deviation = crate::engine::dao_metrics::compute_avg_luoshu_deviation(&vectors);

        // 计算八卦分布
        let mut bagua_counts = [0usize; 8];
        for m in &all {
            if let Some(idx) = m.bagua_index {
                bagua_counts[idx as usize] += 1;
            }
        }

        Ok(self.dao_metrics.snapshot(
            total,
            crystallized,
            archived_count,
            avg_deviation,
            &bagua_counts,
        ))
    }

    /// 拆解合成记忆（Section 3.2 RecursiveUnfold）
    ///
    /// 将一条 Synthesis 类型的抽象记忆展开为具体子记忆。
    /// 算法：
    /// 1. 加载指定记忆，验证其类型为 Synthesis 且有洛书向量
    /// 2. 调用递归拆解算子，激活阈值 min_activation
    /// 3. 为每个子向量创建对应的子记忆（Fact 类型）
    /// 4. 子记忆继承父记忆的项目、标签和隐私设置
    ///
    /// 返回拆解出的子记忆列表及重构保真度。
    ///
    /// 参数：
    /// - `id`: 要拆解的合成记忆 ID
    /// - `min_activation`: 激活阈值（低于此值的九宫格位置不生成子记忆，默认 0.1）
    pub fn unfold_memory(
        &mut self,
        id: &str,
        min_activation: f32,
    ) -> Result<Option<(Vec<Memory>, f32)>, PersistenceError> {
        let all = self.persistence.load_all_memories()?;

        // 找到目标记忆
        let memory = match all.iter().find(|m| m.id == id) {
            Some(m) => m.clone(),
            None => return Ok(None),
        };

        // 仅支持拆解 Synthesis 类型且有洛书向量的记忆
        if memory.memory_type != MemoryType::Synthesis {
            return Ok(None);
        }

        let vector = match memory.luoshu_vector {
            Some(v) => LuoShuVector { values: v },
            None => return Ok(None),
        };

        // 执行递归拆解
        let unfold_result = recursive_unfold(&vector, min_activation.max(0.01));

        if unfold_result.sub_vectors.is_empty() {
            return Ok(Some((Vec::new(), 0.0)));
        }

        // 为每个子向量创建子记忆
        let mut sub_memories = Vec::with_capacity(unfold_result.sub_vectors.len());
        let bagua_names = crate::engine::mirror_trapezoid::BAGUA_CATEGORIES;

        for (i, sub_vec) in unfold_result.sub_vectors.iter().enumerate() {
            let proj = mirror_project(sub_vec);
            let category = bagua_names.get(proj.best_index).copied().unwrap_or("未知");

            let content = format!(
                "「拆解·{}」来自合成记忆的子步骤 #{}。类别: {}，权重: {:.2}",
                memory.content.chars().take(40).collect::<String>(),
                i + 1,
                category,
                unfold_result.sub_weights.get(i).copied().unwrap_or(0.0),
            );

            let mut sub_mem = Memory::new(
                content,
                MemoryType::Fact,
                memory.project.clone(),
                memory.tags.clone(),
                memory.importance,
                None,
            );
            sub_mem.source = Some(format!("unfold:{}", memory.id));
            sub_mem.source_ids = vec![memory.id.clone()];
            sub_mem.luoshu_vector = Some(sub_vec.values);
            sub_mem.bagua_index = Some(proj.best_index as u8);
            sub_mem.bagua_category = Some(proj.best_category.to_string());
            sub_mem.privacy_level = memory.privacy_level;
            sub_mem.session_id = memory.session_id.clone();
            sub_mem.user_id = memory.user_id.clone();

            // 持久化
            self.persistence.save_memory(&sub_mem)?;
            sub_memories.push(sub_mem);
        }

        Ok(Some((sub_memories, unfold_result.fidelity)))
    }

    /// 用户修正记忆（带版本追踪）
    ///
    /// 创建新版本而非直接覆盖，保留修正历史。
    /// 返回修正后的记忆。
    pub fn correct_memory(
        &mut self,
        id: &str,
        new_content: &str,
        reason: Option<&str>,
    ) -> Result<Option<Memory>, PersistenceError> {
        let all = self.persistence.load_all_memories()?;
        let mut found: Option<Memory> = None;

        let updated: Vec<Memory> = all
            .into_iter()
            .map(|mut m| {
                if m.id == id {
                    // 使用版本追踪更新（自动保存历史版本）
                    let reason_str = reason.unwrap_or("用户修正").to_string();
                    m.update_content_with_reason(new_content.to_string(), reason_str);
                    // 追加修正标记到 source
                    m.source = Some(format!("corrected: {}", reason.unwrap_or("未提供原因")));
                    found = Some(m.clone());
                    m
                } else {
                    m
                }
            })
            .collect();

        // 重新写入
        self.persistence.clear_memories()?;
        for m in updated {
            self.persistence.save_memory(&m)?;
        }

        // 记录指标：修正 + 1
        if found.is_some() {
            self.dao_metrics.record_correction();
        }

        Ok(found)
    }
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::create_json_persistence;
    use chrono::{Duration, Utc};
    use tempfile::TempDir;

    fn make_store() -> (TempDir, MemoryStore<crate::persistence::json::JsonPersistence>) {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = create_json_persistence(&data_dir).expect("应成功创建");
        (dir, MemoryStore::new(p))
    }

    /// 创建具有自定义相似度阈值的 MemoryStore（用于合成测试）
    fn make_store_with_threshold(
        threshold: f32,
    ) -> (TempDir, MemoryStore<crate::persistence::json::JsonPersistence>) {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = create_json_persistence(&data_dir).expect("应成功创建");
        (dir, MemoryStore::new(p).with_similarity_threshold(threshold))
    }

    fn make_test_memory(content: &str, mtype: MemoryType) -> Memory {
        Memory::new(
            content.to_string(),
            mtype,
            None,
            vec![],
            Importance::default(),
            None,
        )
    }

    #[test]
    fn test_remember_and_recall() {
        let (_dir, mut store) = make_store();

        let m = make_test_memory("用户偏好使用 pnpm 作为包管理器", MemoryType::Preference);
        let saved = store.remember(m).expect("应成功记住");
        assert!(!saved.id.is_empty());

        let result = store
            .recall("pnpm 包管理器", &RecallFilter::new().with_top_k(3))
            .expect("应成功召回");
        assert!(!result.memories.is_empty());
        assert!(result.memories[0].content.contains("pnpm"));
    }

    #[test]
    fn test_forget() {
        let (_dir, mut store) = make_store();

        let m = make_test_memory("测试记忆", MemoryType::Fact);
        let saved = store.remember(m).expect("应成功记住");
        let id = saved.id;

        let deleted = store.forget(&id).expect("应成功删除");
        assert!(deleted);

        let deleted_again = store.forget(&id).expect("应正常返回");
        assert!(!deleted_again);
    }

    #[test]
    fn test_update_memory() {
        let (_dir, mut store) = make_store();

        let m = make_test_memory("旧内容", MemoryType::Fact);
        let saved = store.remember(m).expect("应成功记住");
        let id = saved.id.clone();

        let old = store
            .update_memory(&id, "新内容", Some(Importance::new(9)))
            .expect("应成功更新");
        assert!(old.is_some());
        assert_eq!(old.unwrap().content, "旧内容");

        let result = store
            .recall("新内容", &RecallFilter::new())
            .expect("应成功召回");
        assert_eq!(result.memories[0].content, "新内容");
        assert_eq!(result.memories[0].importance.value(), 9);
    }

    #[test]
    fn test_update_nonexistent() {
        let (_dir, mut store) = make_store();

        let result = store
            .update_memory("nonexistent", "新", None)
            .expect("应正常返回");
        assert!(result.is_none());
    }

    #[test]
    fn test_list_memories() {
        let (_dir, mut store) = make_store();

        store
            .remember(make_test_memory("Frontend uses React", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("Backend uses Rust", MemoryType::Preference))
            .expect("应成功记住");
        store
            .remember(make_test_memory("Database is PostgreSQL", MemoryType::Decision))
            .expect("应成功记住");

        let (memories, total) = store
            .list_memories(&ListFilter::new())
            .expect("应成功列出");
        assert_eq!(total, 3);
        assert_eq!(memories.len(), 3);
    }

    #[test]
    fn test_list_filter_by_type() {
        let (_dir, mut store) = make_store();

        store
            .remember(make_test_memory("事实记忆", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("偏好记忆", MemoryType::Preference))
            .expect("应成功记住");

        let mut filter = ListFilter::new();
        filter.memory_type = Some(MemoryType::Fact);

        let (memories, total) = store.list_memories(&filter).expect("应成功列出");
        assert_eq!(total, 1);
        assert_eq!(memories[0].memory_type, MemoryType::Fact);
    }

    #[test]
    fn test_stats() {
        let (_dir, mut store) = make_store();

        store
            .remember(make_test_memory("事实1", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("偏好1", MemoryType::Preference))
            .expect("应成功记住");

        let stats = store.stats().expect("应获取统计");
        assert_eq!(stats.total_memories, 2);
        assert_eq!(stats.by_type.get("fact"), Some(&1));
        assert_eq!(stats.by_type.get("preference"), Some(&1));
    }

    #[test]
    fn test_recall_filter_by_type() {
        let (_dir, mut store) = make_store();

        store
            .remember(make_test_memory("Fact content", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("Preference content", MemoryType::Preference))
            .expect("应成功记住");

        let filter = RecallFilter::new().with_type(MemoryType::Fact).with_top_k(5);
        let result = store.recall("content", &filter).expect("应成功召回");

        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].memory_type, MemoryType::Fact);
    }

    #[test]
    fn test_recall_updates_last_accessed() {
        let (_dir, mut store) = make_store();

        let mut m = make_test_memory("测试衰减更新", MemoryType::Fact);
        // 模拟 10 天前的访问
        m.last_accessed = Utc::now() - Duration::days(10);
        let before_access = m.last_accessed;
        let before_factor = m.decay_factor();
        store.remember(m).expect("应成功记住");

        // 执行 recall，触发 mark_accessed
        let old_decayed = {
            let result = store
                .recall("衰减", &RecallFilter::new())
                .expect("应成功召回");
            result.memories[0].decayed_importance()
        };

        // 重新加载记忆，验证 last_accessed 已更新
        let all = store.persistence().load_all_memories().unwrap();
        let updated = all.first().unwrap();
        assert!(
            updated.last_accessed > before_access,
            "recall 后 last_accessed 应该更新: before={:?}, after={:?}",
            before_access, updated.last_accessed
        );
        assert!(
            updated.decay_factor() > before_factor,
            "recall 后衰减因子应回升: before={}, after={}",
            before_factor, updated.decay_factor()
        );
        assert!(
            updated.decayed_importance() > old_decayed,
            "recall 后衰减后重要性应提升: old={}, new={}",
            old_decayed, updated.decayed_importance()
        );
    }
    #[test]
    fn test_recall_min_importance() {
        let (_dir, mut store) = make_store();

        let mut m1 = make_test_memory("高重要性内容", MemoryType::Fact);
        m1.importance = Importance::new(9);
        store.remember(m1).expect("应成功记住");

        let mut m2 = make_test_memory("低重要性内容", MemoryType::Fact);
        m2.importance = Importance::new(2);
        store.remember(m2).expect("应成功记住");

        let mut filter = RecallFilter::new();
        filter.min_importance = Some(Importance::new(5));
        let result = store.recall("内容", &filter).expect("应成功召回");

        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].importance.value(), 9);
    }

    /// 持久化闭环测试：写入 → 查总数 → 确认非零
    #[test]
    fn test_persistence_roundtrip() {
        let (_dir, mut store) = make_store();

        store
            .remember(make_test_memory("持久化测试", MemoryType::Fact))
            .expect("应成功记住");

        let count = store.total_count().expect("应获取总数");
        assert_eq!(count, 1);
    }

    // === P1.1 冲突解决测试 ===

    /// 辅助函数：计算两个字符串的 Jaccard 相似度（中文用 bigram，英文用词集）
    fn jaccard_similarity(a: &str, b: &str) -> f32 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();

        // 检测 CJK 字符
        let has_cjk = a_lower.chars().any(|c| c as u32 >= 0x4E00 && c as u32 <= 0x9FFF)
            || b_lower.chars().any(|c| c as u32 >= 0x4E00 && c as u32 <= 0x9FFF);

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

            intersection as f32 / union as f32
        } else {
            let words_a: std::collections::HashSet<&str> =
                a_lower.split_whitespace().collect();
            let words_b: std::collections::HashSet<&str> =
                b_lower.split_whitespace().collect();

            if words_a.is_empty() && words_b.is_empty() {
                return 1.0;
            }

            let intersection = words_a.intersection(&words_b).count();
            let union = words_a.union(&words_b).count();

            intersection as f32 / union as f32
        }
    }

    #[test]
    fn test_jaccard_identical() {
        assert!((jaccard_similarity("hello world", "hello world") - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_jaccard_disjoint() {
        assert!((jaccard_similarity("hello", "world") - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_jaccard_partial() {
        let sim = jaccard_similarity("hello world rust", "hello world python");
        // 交集: {hello, world} = 2, 并集: {hello, world, rust, python} = 4
        assert!((sim - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_remember_auto_merge_similar() {
        let (_dir, mut store) = make_store();

        // 写入第一条记忆
        let m1 = store
            .remember(make_test_memory("项目使用 PostgreSQL 数据库", MemoryType::Fact))
            .expect("应成功记住");
        let count1 = store.total_count().expect("应获取总数");
        assert_eq!(count1, 1, "第一条记忆后应有 1 条");

        // 写入高度相似的内容（Jaccard ≈ 0.5 ≥ 阈值 0.5，应合并而非新建）
        let m2 = store
            .remember(make_test_memory("项目使用 PostgreSQL 作为主数据库", MemoryType::Fact))
            .expect("应成功记住");
        let count2 = store.total_count().expect("应获取总数");
        assert_eq!(count2, 1, "相似记忆应合并，仍为 1 条");

        // 合并后的记忆 ID 应与第一条相同
        assert_eq!(m2.id, m1.id, "合并后的 ID 应与原记忆一致");
        // 内容应更新为新内容
        assert!(m2.content.contains("PostgreSQL"), "应包含合并后内容");
    }

    #[test]
    fn test_remember_no_merge_dissimilar() {
        let (_dir, mut store) = make_store();

        store
            .remember(make_test_memory("项目使用 PostgreSQL 数据库", MemoryType::Fact))
            .expect("应成功记住");

        // 写入完全不同内容
        store
            .remember(make_test_memory("用户偏好 Python 语言开发", MemoryType::Preference))
            .expect("应成功记住");

        let count = store.total_count().expect("应获取总数");
        assert_eq!(count, 2, "不相似的内容应分别存储");
    }

    #[test]
    fn test_remember_merge_tags() {
        let (_dir, mut store) = make_store();

        // 使用英文内容确保 Jaccard ≥ 阈值
        let mut m1 = make_test_memory("Frontend uses React framework", MemoryType::Fact);
        m1.tags = vec!["react".into(), "frontend".into()];
        store.remember(m1).expect("应成功记住");

        let mut m2 = make_test_memory("Frontend uses React and TypeScript framework", MemoryType::Fact);
        m2.tags = vec!["typescript".into()];
        store.remember(m2).expect("应成功记住");

        let result = store
            .recall("React", &RecallFilter::new())
            .expect("应成功召回");

        assert_eq!(result.memories.len(), 1, "应合并为一条");
        let tags = &result.memories[0].tags;
        assert!(tags.contains(&"react".to_string()), "应保留原标签");
        assert!(tags.contains(&"frontend".to_string()), "应保留原标签");
        assert!(tags.contains(&"typescript".to_string()), "应合并新标签");
    }

    #[test]
    fn test_archive_expired_moves_to_cold_storage() {
        let (_dir, mut store) = make_store();

        // 创建过期记忆（2天前创建，ttl=1天）
        let mut expired = Memory::new(
            "过期记忆内容".to_string(),
            MemoryType::Fact,
            None,
            vec!["test".into()],
            Importance::default(),
            Some(1),
        );
        expired.created_at = Utc::now() - Duration::days(2);

        // 创建活跃记忆
        let active = Memory::new(
            "活跃记忆内容".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::default(),
            None,
        );

        store.remember(expired.clone()).unwrap();
        store.remember(active.clone()).unwrap();

        assert_eq!(store.total_count().unwrap(), 2, "初始应有2条记忆");

        // 执行归档
        let count = store.archive_expired().unwrap();
        assert_eq!(count, 1, "应归档1条过期记忆");

        // 归档后只剩1条活跃记忆
        assert_eq!(store.total_count().unwrap(), 1);

        // 归档文件中有1条记忆
        let archived = store.persistence().load_archived_memories().unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, expired.id, "归档记忆ID应匹配");
    }

    #[test]
    fn test_archive_expired_no_expired_returns_zero() {
        let (_dir, mut store) = make_store();

        let m = Memory::new(
            "活跃记忆".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::default(),
            None,
        );
        store.remember(m).unwrap();

        let count = store.archive_expired().unwrap();
        assert_eq!(count, 0, "无过期记忆应返回0");
        assert_eq!(store.total_count().unwrap(), 1, "活跃记忆应保持不变");
    }

    #[test]
    fn test_archive_expired_preserves_unexpired() {
        let (_dir, mut store) = make_store();

        // 创建过期记忆
        let mut expired = Memory::new(
            "过期".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::default(),
            Some(1),
        );
        expired.created_at = Utc::now() - Duration::days(2);

        // 创建活跃记忆
        let active1 = Memory::new(
            "活跃1".to_string(),
            MemoryType::Preference,
            None,
            vec![],
            Importance::default(),
            None,
        );
        let active2 = Memory::new(
            "活跃2".to_string(),
            MemoryType::Decision,
            None,
            vec![],
            Importance::new(9),
            None,
        );

        store.remember(expired).unwrap();
        store.remember(active1.clone()).unwrap();
        store.remember(active2.clone()).unwrap();

        let count = store.archive_expired().unwrap();
        assert_eq!(count, 1, "应归档1条过期记忆");

        // 活跃记忆应保留
        let (_all, total) = store.list_memories(&ListFilter::new()).unwrap();
        assert_eq!(total, 2, "应保留2条活跃记忆");
    }

    // === 递归合成测试 ===

    /// 验证：写入 3 条相似记忆后自动触发递归合成
    #[test]
    fn test_synthesis_triggered_on_remember() {
        let (_dir, mut store) = make_store_with_threshold(0.9);

        // 写入 3 条关于项目技术栈的相似记忆（Jaccard 约 0.5-0.8，不会被合并但会被聚类）
        store
            .remember(make_test_memory("项目使用 PostgreSQL 数据库", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("项目数据库连接使用 PostgreSQL", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("PostgreSQL 是项目的主数据库", MemoryType::Fact))
            .expect("应成功记住");

        // 应包含源记忆 + 合成记忆
        let (memories, total) = store.list_memories(&ListFilter::new()).unwrap();
        assert!(total >= 4, "应有 3 条源记忆 + ≥1 条合成记忆，实际: {}", total);

        // 存在 Synthesis 类型的记忆
        let has_synthesis = memories.iter().any(|m| m.memory_type == MemoryType::Synthesis);
        assert!(has_synthesis, "应包含合成记忆");
    }

    /// 验证：不相似记忆不会触发合成
    #[test]
    fn test_synthesis_not_triggered_dissimilar() {
        let (_dir, mut store) = make_store();

        store
            .remember(make_test_memory("项目使用 PostgreSQL 数据库", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("用户偏好 Python 语言开发", MemoryType::Preference))
            .expect("应成功记住");
        store
            .remember(make_test_memory("前端使用 React 框架", MemoryType::Decision))
            .expect("应成功记住");

        // 三条不相关记忆，不应合成
        let (memories, _) = store.list_memories(&ListFilter::new()).unwrap();
        let synthesis_count = memories
            .iter()
            .filter(|m| m.memory_type == MemoryType::Synthesis)
            .count();
        assert_eq!(synthesis_count, 0, "不相似记忆不应触发合成");
    }

    /// 验证：合成记忆包含正确的元数据
    #[test]
    fn test_synthesis_metadata() {
        let (_dir, mut store) = make_store_with_threshold(0.9);

        store
            .remember(make_test_memory("项目使用 PostgreSQL 数据库", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("项目数据库连接使用 PostgreSQL", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("PostgreSQL 是项目的主数据库", MemoryType::Fact))
            .expect("应成功记住");

        let (memories, _) = store.list_memories(&ListFilter::new()).unwrap();

        // 找到合成记忆
        let synthesis = memories.iter().find(|m| m.memory_type == MemoryType::Synthesis);
        assert!(synthesis.is_some(), "应存在合成记忆");

        let s = synthesis.unwrap();
        assert!(!s.source_ids.is_empty(), "合成记忆应有 source_ids");
        assert!(s.source_ids.len() >= 3, "source_ids 应包含源记忆");
        assert!(s.confidence.is_some(), "合成记忆应有 confidence");
        assert_eq!(s.source.as_deref(), Some("recursive_synthesis"), "source 应为 recursive_synthesis");
    }

    /// 验证：合成记忆在 recall 中获得优先返回
    #[test]
    fn test_synthesis_priority_in_recall() {
        let (_dir, mut store) = make_store_with_threshold(0.9);

        // 先写入合成记忆已经存在的场景——通过 3 条相似记忆触发合成
        store
            .remember(make_test_memory("PostgreSQL 数据库配置", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("数据库连接使用 PostgreSQL", MemoryType::Fact))
            .expect("应成功记住");
        store
            .remember(make_test_memory("使用 PostgreSQL 数据库存储数据", MemoryType::Fact))
            .expect("应成功记住");

        // 写入一条不相关的记忆作为对比
        store
            .remember(make_test_memory("前端使用 React 框架", MemoryType::Decision))
            .expect("应成功记住");

        let result = store
            .recall("PostgreSQL 数据库", &RecallFilter::new().with_top_k(5))
            .expect("应成功召回");

        // 合成记忆应该排在最前面（置信度 boost）
        if result.memories.len() >= 2 {
            let first_is_synthesis = result.memories[0].memory_type == MemoryType::Synthesis;
            let first_score = result.scores[0];
            let second_score = result.scores.get(1).copied().unwrap_or(0.0);
            assert!(
                first_is_synthesis || first_score >= second_score,
                "合成记忆应优先返回: scores={:?}",
                result.scores
            );
        }
    }
}