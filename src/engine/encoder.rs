// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 语义编码模块
// 将代码文本转换为结构化向量表示，用于后续相似度计算。

use crate::chunker::CodeChunk;
use serde::{Deserialize, Serialize};

/// 向量表示
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingVector {
    pub dim: usize,
    pub values: Vec<f32>,
}

impl EmbeddingVector {
    pub fn zeros(dim: usize) -> Self {
        Self {
            dim,
            values: vec![0.0; dim],
        }
    }

    /// 相似度计算
    pub fn cosine_similarity(&self, other: &EmbeddingVector) -> f32 {
        if self.dim != other.dim || self.dim == 0 {
            return 0.0;
        }

        let dot: f32 = self
            .values
            .iter()
            .zip(other.values.iter())
            .map(|(a, b)| a * b)
            .sum();
        let norm_a: f32 = self.values.iter().map(|v| v * v).sum::<f32>().sqrt();
        let norm_b: f32 = other.values.iter().map(|v| v * v).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
    }

    /// v0.5.4 检查向量中是否包含 NaN 值
    /// NaN 值会破坏 HNSW 图的结构和检索精度，必须在插入前检测并拒绝
    pub fn has_nan(&self) -> bool {
        self.values.iter().any(|v| v.is_nan())
    }
}

/// 编码器契约
pub trait CodeEncoder: Send + Sync {
    fn encode(&self, chunk: &CodeChunk) -> Result<EmbeddingVector, String>;

    fn encode_batch(&self, chunks: &[CodeChunk]) -> Result<Vec<EmbeddingVector>, String> {
        chunks.iter().map(|c| self.encode(c)).collect()
    }

    fn dimension(&self) -> usize;
}

/// 快速编码器实现
pub struct FastEncoder {
    dim: usize,
    terms: Vec<String>,
}

impl FastEncoder {
    pub fn new(terms: Vec<String>) -> Self {
        let dim = terms.len();
        Self { dim, terms }
    }

    fn signal_map(&self, text: &str) -> Vec<f32> {
        let lower = text.to_lowercase();
        let count = lower.split_whitespace().count().max(1) as f32;
        self.terms
            .iter()
            .map(|t| lower.matches(&t.to_lowercase()).count() as f32 / count)
            .collect()
    }
}

impl CodeEncoder for FastEncoder {
    fn encode(&self, chunk: &CodeChunk) -> Result<EmbeddingVector, String> {
        let combined = format!(
            "{} {} {}",
            chunk.signature,
            chunk.doc_comment.as_deref().unwrap_or(""),
            chunk.content
        );
        let values = self.signal_map(&combined);
        Ok(EmbeddingVector {
            dim: self.dim,
            values,
        })
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

// ==================== 测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk(name: &str, content: &str) -> CodeChunk {
        CodeChunk {
            id: format!("test.rs:L1-L{}", content.lines().count()),
            file_path: "test.rs".to_string(),
            start_line: 1,
            end_line: content.lines().count(),
            chunk_type: "fn".to_string(),
            name: name.to_string(),
            signature: format!("fn {}()", name),
            content: content.to_string(),
            doc_comment: None,
            language: "rust".to_string(),
        }
    }

    #[test]
    fn test_zeros() {
        let v = EmbeddingVector::zeros(4);
        assert_eq!(v.dim, 4);
        assert_eq!(v.values, vec![0.0; 4]);
    }

    #[test]
    fn test_similarity_identical() {
        let v1 = EmbeddingVector {
            dim: 3,
            values: vec![1.0, 0.0, 0.0],
        };
        let v2 = v1.clone();
        assert!((v1.cosine_similarity(&v2) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_similarity_orthogonal() {
        let v1 = EmbeddingVector {
            dim: 3,
            values: vec![1.0, 0.0, 0.0],
        };
        let v2 = EmbeddingVector {
            dim: 3,
            values: vec![0.0, 1.0, 0.0],
        };
        assert!((v1.cosine_similarity(&v2) - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_encoder_dimension() {
        let encoder = FastEncoder::new(vec![
            "a".into(),
            "b".into(),
            "c".into(),
            "d".into(),
            "e".into(),
        ]);
        assert_eq!(encoder.dimension(), 5);
    }

    #[test]
    fn test_encoder_output() {
        let encoder = FastEncoder::new(vec!["alpha".into(), "beta".into()]);
        let chunk = make_chunk("test", "fn test() { alpha beta gamma }");
        let vec = encoder.encode(&chunk).unwrap();
        assert_eq!(vec.dim, 2);
        assert!(vec.values[0] > 0.0);
        assert!(vec.values[1] > 0.0);
    }

    #[test]
    fn test_batch_encode() {
        let encoder = FastEncoder::new(vec!["fn".into()]);
        let chunks = vec![
            make_chunk("a", "fn a() {}"),
            make_chunk("b", "fn b() {}"),
            make_chunk("c", "fn c() {}"),
        ];
        let vectors = encoder.encode_batch(&chunks).unwrap();
        assert_eq!(vectors.len(), 3);
    }
}
