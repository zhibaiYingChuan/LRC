// ============================================================
// 许可证: DaoTi Research License v1.0
// 受保护核心引擎 — 包含模型底层架构衍生的编码/检索/编排算法。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================

pub mod encoder;
pub mod encoder_registry;
pub mod hnsw;
pub mod retriever;
pub mod manager;

#[cfg(feature = "ml")]
pub mod encoder_codebert;

pub use encoder::{CodeEncoder, EmbeddingVector, FastEncoder};
pub use encoder_registry::EncoderRegistry;
pub use hnsw::HnswRetriever;
pub use manager::{ChunkStats, CoreManager};
pub use retriever::{CodeRetriever, LocalRetriever, RetrievalResult, ScoredChunk};

#[cfg(feature = "ml")]
pub use encoder_codebert::{CodeBertEncoder, PoolingStrategy};