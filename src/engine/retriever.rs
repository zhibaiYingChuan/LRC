// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心检索算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 检索模块
// 基于向量空间模型的代码片段检索。

use crate::chunker::CodeChunk;
use crate::engine::encoder::{CodeEncoder, EmbeddingVector};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 检索结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredChunk {
    pub chunk: CodeChunk,
    pub score: f32,
    pub rank: usize,
}

/// 检索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalResult {
    pub query: String,
    pub returned: usize,
    pub total_indexed: usize,
    pub results: Vec<ScoredChunk>,
}

/// 检索器契约
pub trait CodeRetriever: Send + Sync {
    fn search(&self, query: &str, top_k: usize) -> RetrievalResult;
    fn indexed_count(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.indexed_count() == 0
    }
}

/// 本地检索器
pub struct LocalRetriever<E: CodeEncoder> {
    encoder: Arc<E>,
    vectors: Vec<EmbeddingVector>,
    chunks: Vec<CodeChunk>,
    threshold: f32,
}

impl<E: CodeEncoder> LocalRetriever<E> {
    pub fn new(encoder: Arc<E>, threshold: f32) -> Self {
        Self {
            encoder,
            vectors: Vec::new(),
            chunks: Vec::new(),
            threshold,
        }
    }

    pub fn index_chunk(&mut self, chunk: CodeChunk) {
        let vector = self.encoder.encode(&chunk);
        self.vectors.push(vector);
        self.chunks.push(chunk);
    }

    pub fn index_batch(&mut self, chunks: Vec<CodeChunk>) {
        for chunk in chunks {
            self.index_chunk(chunk);
        }
    }

    pub fn clear(&mut self) {
        self.vectors.clear();
        self.chunks.clear();
    }

    /// 获取所有嵌入向量（用于缓存序列化）
    pub fn get_vectors(&self) -> &[EmbeddingVector] {
        &self.vectors
    }

    /// 直接从向量+片段重建索引（从缓存加载，跳过重新编码）
    pub fn load_from_vectors(&mut self, vectors: Vec<EmbeddingVector>, chunks: Vec<CodeChunk>) {
        self.vectors = vectors;
        self.chunks = chunks;
    }

    pub fn all_chunks(&self) -> &[CodeChunk] {
        &self.chunks
    }
}

impl<E: CodeEncoder> CodeRetriever for LocalRetriever<E> {
    fn search(&self, query: &str, top_k: usize) -> RetrievalResult {
        if self.vectors.is_empty() {
            return RetrievalResult {
                query: query.to_string(),
                returned: 0,
                total_indexed: 0,
                results: Vec::new(),
            };
        }

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

        let mut scored: Vec<ScoredChunk> = self
            .vectors
            .iter()
            .zip(self.chunks.iter())
            .map(|(vec, chunk)| {
                let score = query_vector.cosine_similarity(vec);
                ScoredChunk {
                    chunk: chunk.clone(),
                    score,
                    rank: 0,
                }
            })
            .filter(|s| s.score >= self.threshold)
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let results: Vec<ScoredChunk> = scored
            .into_iter()
            .take(top_k)
            .enumerate()
            .map(|(i, mut s)| {
                s.rank = i + 1;
                s
            })
            .collect();

        let returned = results.len();
        RetrievalResult {
            query: query.to_string(),
            returned,
            total_indexed: self.chunks.len(),
            results,
        }
    }

    fn indexed_count(&self) -> usize {
        self.chunks.len()
    }
}

// ==================== 测试 ====================

#[cfg(test)]
mod tests {
    use crate::engine::encoder::FastEncoder;
    use super::*;

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

    fn build_retriever() -> LocalRetriever<FastEncoder> {
        let encoder = Arc::new(FastEncoder::new(vec![
            "alpha".into(), "beta".into(), "gamma".into(),
            "delta".into(), "epsilon".into(),
        ]));
        LocalRetriever::new(encoder, 0.01)
    }

    #[test]
    fn test_empty_search() {
        let retriever = build_retriever();
        let result = retriever.search("alpha", 5);
        assert_eq!(result.returned, 0);
        assert_eq!(result.total_indexed, 0);
    }

    #[test]
    fn test_single_match() {
        let mut retriever = build_retriever();
        retriever.index_chunk(make_chunk("a.rs", "func_a", "fn", "fn func_a() { alpha beta }"));
        let result = retriever.search("alpha", 5);
        assert_eq!(result.returned, 1);
        assert_eq!(result.results[0].rank, 1);
    }

    #[test]
    fn test_ranking() {
        let mut retriever = build_retriever();
        retriever.index_chunk(make_chunk("a.rs", "fn1", "fn", "fn fn1() { x y }"));
        retriever.index_chunk(make_chunk("b.rs", "fn2", "fn", "fn fn2() { alpha beta gamma }"));
        retriever.index_chunk(make_chunk("c.rs", "fn3", "fn", "fn fn3() { alpha }"));

        let result = retriever.search("alpha beta", 5);
        assert!(result.returned >= 2);
        assert_eq!(result.results[0].chunk.name, "fn2");
    }

    #[test]
    fn test_top_k() {
        let mut retriever = build_retriever();
        for i in 0..10 {
            retriever.index_chunk(make_chunk(
                &format!("file{}.rs", i), &format!("fn_{}", i), "fn",
                &format!("fn fn_{}() {{ alpha }}", i),
            ));
        }
        let result = retriever.search("alpha", 3);
        assert_eq!(result.returned, 3);
        assert_eq!(result.total_indexed, 10);
    }

    #[test]
    fn test_count() {
        let mut retriever = build_retriever();
        assert!(retriever.is_empty());
        retriever.index_chunk(make_chunk("a.rs", "f", "fn", "fn f() {}"));
        assert_eq!(retriever.indexed_count(), 1);
    }

    #[test]
    fn test_serialization() {
        let mut retriever = build_retriever();
        retriever.index_chunk(make_chunk("a.rs", "f", "fn", "fn f() { alpha }"));
        let result = retriever.search("alpha", 5);
        let json = serde_json::to_string(&result).expect("应序列化");
        let restored: RetrievalResult = serde_json::from_str(&json).expect("应反序列化");
        assert_eq!(restored.query, "alpha");
    }
}