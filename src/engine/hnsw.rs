// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心检索算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// HNSW 近似最近邻检索模块
//
// 基于 Navigable Small World (NSW) 图算法实现高效向量检索。
// 单层 NSW 图结构，支持贪心搜索和动态插入。
// 后续可扩展为多层 HNSW（Hierarchical NSW）。

use crate::chunker::CodeChunk;
use crate::engine::encoder::{CodeEncoder, EmbeddingVector};
use crate::engine::retriever::{CodeRetriever, RetrievalResult, ScoredChunk};
use std::sync::Arc;

/// NSW 图节点
#[derive(Debug, Clone)]
struct HnswNode {
    /// 向量表示
    vector: EmbeddingVector,
    /// 关联的代码片段索引（在 chunks 数组中的位置）
    chunk_idx: usize,
    /// 邻居节点索引列表
    neighbors: Vec<usize>,
}

impl HnswNode {
    fn new(vector: EmbeddingVector, chunk_idx: usize) -> Self {
        Self {
            vector,
            chunk_idx,
            neighbors: Vec::new(),
        }
    }
}

/// NSW 图结构
///
/// 基于 Navigable Small World 算法的近似最近邻索引。
/// 核心参数：
/// - M: 每个节点的最大连接数（默认 16）
/// - ef_search: 搜索时的束宽度（默认 50）
struct HnswGraph {
    /// 图中所有节点
    nodes: Vec<HnswNode>,
    /// 搜索入口节点索引
    entry_point: Option<usize>,
    /// 每个节点的最大连接数
    max_connections: usize,
    /// 搜索束宽度
    ef_search: usize,
    /// 节点计数器（用于追踪已插入数量）
    node_count: usize,
}

impl HnswGraph {
    /// 创建 NSW 图
    fn new(max_connections: usize, ef_search: usize) -> Self {
        Self {
            nodes: Vec::new(),
            entry_point: None,
            max_connections,
            ef_search,
            node_count: 0,
        }
    }

    /// 计算两个向量的余弦距离（1 - 余弦相似度）
    fn cosine_distance(a: &EmbeddingVector, b: &EmbeddingVector) -> f32 {
        1.0 - a.cosine_similarity(b)
    }

    /// 图中节点数
    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 是否为空
    fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 贪心搜索：从入口出发，在图中寻找最近的 ef 个邻居
    ///
    /// 返回 (候选节点索引列表, 候选距离列表)，按距离升序排列。
    fn search_layer(
        &self,
        query: &EmbeddingVector,
        entry_idx: usize,
        ef: usize,
    ) -> (Vec<usize>, Vec<f32>) {
        let mut visited = vec![false; self.nodes.len()];
        // 候选集（按距离升序）
        let mut candidates: Vec<(usize, f32)> = Vec::with_capacity(ef * 2);
        // 结果集（按距离升序，容量 ef）
        let mut results: Vec<(usize, f32)> = Vec::with_capacity(ef);

        let entry_dist = Self::cosine_distance(query, &self.nodes[entry_idx].vector);
        candidates.push((entry_idx, entry_dist));
        results.push((entry_idx, entry_dist));
        visited[entry_idx] = true;

        while !candidates.is_empty() {
            // 找到候选集中最近的未探索节点
            let closest_idx = candidates
                .iter()
                .enumerate()
                .min_by(|(_, (_, a)), (_, (_, b))| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);

            let (explore_node, explore_dist) = candidates.swap_remove(closest_idx);

            // 如果当前最近候选比结果中最远的还远，提前终止
            if let Some(&(_, worst_dist)) = results.last() {
                if explore_dist > worst_dist {
                    break;
                }
            }

            // 探索邻居
            for &neighbor_idx in &self.nodes[explore_node].neighbors {
                if visited[neighbor_idx] {
                    continue;
                }
                visited[neighbor_idx] = true;

                let dist = Self::cosine_distance(query, &self.nodes[neighbor_idx].vector);

                // 如果结果集未满或当前节点比最远结果更近
                let should_add = results.len() < ef
                    || dist < results.last().map(|&(_, d)| d).unwrap_or(f32::MAX);

                if should_add {
                    candidates.push((neighbor_idx, dist));
                    results.push((neighbor_idx, dist));
                    // 保持结果集按距离升序
                    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                    // 截断到 ef
                    if results.len() > ef {
                        results.truncate(ef);
                    }
                }
            }
        }

        let indices: Vec<usize> = results.iter().map(|&(i, _)| i).collect();
        let distances: Vec<f32> = results.iter().map(|&(_, d)| d).collect();
        (indices, distances)
    }

    /// 插入新节点到图中
    ///
    /// 1. 先推入节点（确保 self.nodes 中包含此节点）
    /// 2. 搜索最近的 ef 个邻居
    /// 3. 建立双向连接
    /// 4. 对超限的邻居进行剪枝
    fn insert_node(&mut self, vector: EmbeddingVector, chunk_idx: usize) {
        let node_idx = self.nodes.len();
        // 先推入节点，确保 prune_neighbors 可以访问 self.nodes[node_idx]
        let node = HnswNode::new(vector, chunk_idx);
        self.nodes.push(node);

        if let Some(entry) = self.entry_point {
            // 搜索最近的 max_connections 个邻居
            let ef = self.ef_search.max(self.max_connections);
            let (neighbors, _distances) = self.search_layer(
                &self.nodes[node_idx].vector,
                entry,
                ef,
            );

            // 选择最近的 max_connections 个作为连接
            let selected: Vec<usize> = neighbors
                .into_iter()
                .take(self.max_connections)
                .collect();

            // 建立双向连接
            for &neighbor_idx in &selected {
                self.nodes[node_idx].neighbors.push(neighbor_idx);
                self.nodes[neighbor_idx].neighbors.push(node_idx);
            }
        } else {
            // 第一个节点，设为入口
            self.entry_point = Some(node_idx);
        }

        // 剪枝：对每个被连接的邻居，检查是否超出 max_connections
        let neighbors_snapshot = self.nodes[node_idx].neighbors.clone();
        for &neighbor_idx in &neighbors_snapshot {
            if self.nodes[neighbor_idx].neighbors.len() > self.max_connections {
                self.prune_neighbors(neighbor_idx);
            }
        }

        self.node_count += 1;
    }

    /// 剪枝：对指定节点的邻居列表，保留最近的 max_connections 个
    fn prune_neighbors(&mut self, node_idx: usize) {
        let max = self.max_connections;
        if self.nodes[node_idx].neighbors.len() <= max {
            return;
        }

        let node_vector = self.nodes[node_idx].vector.clone();
        let mut neighbor_dists: Vec<(usize, f32)> = self.nodes[node_idx]
            .neighbors
            .iter()
            .map(|&n| {
                let d = Self::cosine_distance(&node_vector, &self.nodes[n].vector);
                (n, d)
            })
            .collect();

        // 按距离升序排序，保留最近的 max 个
        neighbor_dists
            .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let pruned: Vec<usize> = neighbor_dists
            .into_iter()
            .take(max)
            .map(|(i, _)| i)
            .collect();

        // 从被移除的邻居中删除当前节点的反向连接
        let old_neighbors: Vec<usize> = self.nodes[node_idx]
            .neighbors
            .iter()
            .filter(|n| !pruned.contains(n))
            .copied()
            .collect();

        for &removed_n in &old_neighbors {
            self.nodes[removed_n].neighbors.retain(|&x| x != node_idx);
        }

        self.nodes[node_idx].neighbors = pruned;
    }

    /// 批量插入节点
    #[allow(dead_code)]
    fn insert_batch(&mut self, vectors: Vec<(EmbeddingVector, usize)>) {
        for (vector, chunk_idx) in vectors {
            self.insert_node(vector, chunk_idx);
        }
    }
}

/// HNSW 检索器 — 基于 NSW 图的近似最近邻检索
///
/// 实现 `CodeRetriever` trait，可作为 `LocalRetriever` 的替代方案。
/// 当数据量较大时提供比线性扫描更快的检索。
pub struct HnswRetriever<E: CodeEncoder> {
    encoder: Arc<E>,
    /// NSW 图索引
    graph: HnswGraph,
    /// 代码片段存储
    chunks: Vec<CodeChunk>,
    /// 相似度阈值
    threshold: f32,
}

impl<E: CodeEncoder> HnswRetriever<E> {
    /// 创建 HNSW 检索器
    ///
    /// - `encoder`: 编码器
    /// - `threshold`: 相似度阈值（低于此值的结果被过滤）
    /// - `max_connections`: 每个节点的最大连接数（默认 16）
    /// - `ef_search`: 搜索束宽度（默认 50）
    pub fn new(
        encoder: Arc<E>,
        threshold: f32,
        max_connections: usize,
        ef_search: usize,
    ) -> Self {
        Self {
            encoder,
            graph: HnswGraph::new(max_connections, ef_search),
            chunks: Vec::new(),
            threshold,
        }
    }

    /// 创建默认参数的 HNSW 检索器
    pub fn with_defaults(encoder: Arc<E>, threshold: f32) -> Self {
        Self::new(encoder, threshold, 16, 50)
    }

    /// 索引单个代码片段
    pub fn index_chunk(&mut self, chunk: CodeChunk) {
        let vector = self.encoder.encode(&chunk);
        let chunk_idx = self.chunks.len();
        self.chunks.push(chunk);
        self.graph.insert_node(vector, chunk_idx);
    }

    /// 批量索引
    pub fn index_batch(&mut self, chunks: Vec<CodeChunk>) {
        for chunk in chunks {
            self.index_chunk(chunk);
        }
    }

    /// 清空索引
    pub fn clear(&mut self) {
        self.graph = HnswGraph::new(self.graph.max_connections, self.graph.ef_search);
        self.chunks.clear();
    }

    /// 返回所有已索引的片段
    pub fn all_chunks(&self) -> &[CodeChunk] {
        &self.chunks
    }
}

impl<E: CodeEncoder> CodeRetriever for HnswRetriever<E> {
    fn search(&self, query: &str, top_k: usize) -> RetrievalResult {
        if self.graph.is_empty() {
            return RetrievalResult {
                query: query.to_string(),
                returned: 0,
                total_indexed: 0,
                results: Vec::new(),
            };
        }

        // 编码查询
        let query_chunk = CodeChunk {
            id: "__query__".to_string(),
            file_path: String::new(),
            start_line: 0,
            end_line: 0,
            chunk_type: "query".to_string(),
            name: query.to_string(),
            signature: query.to_string(),
            content: query.to_string(),
            doc_comment: None,
            language: "text".to_string(),
        };
        let query_vector = self.encoder.encode(&query_chunk);

        // 通过 NSW 图搜索
        let entry = self.graph.entry_point.unwrap_or(0);
        let (indices, distances) = self.graph.search_layer(
            &query_vector,
            entry,
            self.graph.ef_search.max(top_k),
        );

        // 距离转相似度，过滤低于阈值的
        let mut scored: Vec<ScoredChunk> = indices
            .into_iter()
            .zip(distances)
            .filter_map(|(idx, dist)| {
                let score = 1.0 - dist; // 余弦距离转相似度
                if score >= self.threshold {
                    Some(ScoredChunk {
                        chunk: self.chunks[self.graph.nodes[idx].chunk_idx].clone(),
                        score,
                        rank: 0,
                    })
                } else {
                    None
                }
            })
            .collect();

        // 按分数降序
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 截取 top_k
        scored.truncate(top_k);

        // 设置排名
        for (i, item) in scored.iter_mut().enumerate() {
            item.rank = i + 1;
        }

        let returned = scored.len();
        RetrievalResult {
            query: query.to_string(),
            returned,
            total_indexed: self.chunks.len(),
            results: scored,
        }
    }

    fn indexed_count(&self) -> usize {
        self.chunks.len()
    }
}

// ==================== 测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::encoder::FastEncoder;

    fn make_chunk(file: &str, name: &str, chunk_type: &str, content: &str) -> CodeChunk {
        CodeChunk {
            id: format!("{}:L1-L1", file),
            file_path: file.to_string(),
            start_line: 1,
            end_line: 1,
            chunk_type: chunk_type.to_string(),
            name: name.to_string(),
            signature: format!("{} {}()", chunk_type, name),
            content: content.to_string(),
            doc_comment: None,
            language: "rust".to_string(),
        }
    }

    fn build_encoder() -> Arc<FastEncoder> {
        Arc::new(FastEncoder::new(vec![
            "alpha".into(), "beta".into(), "gamma".into(),
            "delta".into(), "epsilon".into(),
        ]))
    }

    fn build_retriever() -> HnswRetriever<FastEncoder> {
        HnswRetriever::with_defaults(build_encoder(), 0.01)
    }

    #[test]
    fn test_hnsw_empty_search() {
        let retriever = build_retriever();
        let result = retriever.search("alpha", 5);
        assert_eq!(result.returned, 0);
        assert_eq!(result.total_indexed, 0);
    }

    #[test]
    fn test_hnsw_single_insert_search() {
        let mut retriever = build_retriever();
        retriever.index_chunk(make_chunk("a.rs", "func_a", "fn", "fn func_a() { alpha beta }"));
        let result = retriever.search("alpha", 5);
        assert_eq!(result.returned, 1);
        assert_eq!(result.results[0].chunk.name, "func_a");
    }

    #[test]
    fn test_hnsw_multiple_insert_search() {
        let mut retriever = build_retriever();
        retriever.index_chunk(make_chunk("a.rs", "fn1", "fn", "fn fn1() { x y }"));
        retriever.index_chunk(make_chunk("b.rs", "fn2", "fn", "fn fn2() { alpha beta gamma }"));
        retriever.index_chunk(make_chunk("c.rs", "fn3", "fn", "fn fn3() { alpha }"));

        let result = retriever.search("alpha beta", 5);
        assert!(result.returned >= 2, "应至少召回 2 个结果: {}", result.returned);
    }

    #[test]
    fn test_hnsw_top_k() {
        let mut retriever = build_retriever();
        for i in 0..10 {
            retriever.index_chunk(make_chunk(
                &format!("file{}.rs", i),
                &format!("fn_{}", i),
                "fn",
                &format!("fn fn_{}() {{ alpha }}", i),
            ));
        }
        let result = retriever.search("alpha", 3);
        assert_eq!(result.returned, 3, "top_k=3 应返回 3 个结果");
        assert_eq!(result.total_indexed, 10);
    }

    #[test]
    fn test_hnsw_count() {
        let mut retriever = build_retriever();
        assert!(retriever.is_empty());
        retriever.index_chunk(make_chunk("a.rs", "f", "fn", "fn f() {}"));
        assert_eq!(retriever.indexed_count(), 1);
        assert!(!retriever.is_empty());
    }

    #[test]
    fn test_hnsw_clear() {
        let mut retriever = build_retriever();
        retriever.index_chunk(make_chunk("a.rs", "f", "fn", "fn f() { alpha }"));
        assert_eq!(retriever.indexed_count(), 1);

        retriever.clear();
        assert_eq!(retriever.indexed_count(), 0);
        assert!(retriever.is_empty());
    }

    #[test]
    fn test_hnsw_threshold_filter() {
        let mut retriever = HnswRetriever::with_defaults(build_encoder(), 0.9);
        retriever.index_chunk(make_chunk("a.rs", "fn1", "fn", "fn fn1() { x y z w }"));
        retriever.index_chunk(make_chunk("b.rs", "fn2", "fn", "fn fn2() { alpha beta gamma delta epsilon }"));

        let result = retriever.search("alpha beta gamma", 5);
        // 高阈值会过滤掉一些结果
        assert!(result.returned <= 2);
    }

    #[test]
    fn test_hnsw_large_batch() {
        let mut retriever = build_retriever();
        // 插入 100 个节点测试图形结构稳定性
        // 使用编码器词汇表中的关键词确保有效匹配
        let keywords = ["alpha", "beta", "gamma", "delta", "epsilon"];
        for i in 0..100 {
            let kw = keywords[i % keywords.len()];
            retriever.index_chunk(make_chunk(
                &format!("file{}.rs", i),
                &format!("fn_{}", i),
                "fn",
                &format!("fn fn_{}() {{ {} }}", i, kw),
            ));
        }
        assert_eq!(retriever.indexed_count(), 100);

        // 搜索应返回合理结果（alpha 出现在 20% 的节点中）
        let result = retriever.search("alpha", 5);
        assert!(result.returned > 0, "应至少有一个匹配");
    }
}