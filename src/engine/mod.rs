// ============================================================
// 许可证: DaoTi Research License v1.0
// 受保护核心引擎 — 包含模型底层架构衍生的编码/检索/编排算法。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================

pub mod encoder;
pub mod encoder_registry;
pub mod hnsw;
pub mod llm_translator;
pub mod luoshu_encoder;
pub mod mirror_trapezoid;
pub mod retriever;
pub mod manager;
pub mod dao_metrics;

#[cfg(feature = "ml")]
pub mod encoder_codebert;

/// 洛书编码器 ML 模式（真实 BERT Embedding，需 ml feature）
#[cfg(feature = "ml")]
pub mod luoshu_encoder_ml;

pub use encoder::{CodeEncoder, EmbeddingVector, FastEncoder};
pub use encoder_registry::EncoderRegistry;
pub use hnsw::HnswRetriever;
pub use llm_translator::LlmApiConfig;
pub use luoshu_encoder::{LuoShuEncoder, LuoShuVector};
pub use manager::{ChunkStats, CoreManager};
pub use mirror_trapezoid::{
    mirror_project, recursive_compose, recursive_unfold, evolution_cycle,
    BaguaProjection, ComposeResult, TrapezoidFocusResult, TrapezoidROI, UnfoldResult,
    BAGUA_CATEGORIES, BAGUA_NAMES,
};
pub use retriever::{CodeRetriever, LocalRetriever, RetrievalResult, ScoredChunk};
pub use dao_metrics::{DaoMetrics, DaoMetricsSnapshot, compute_avg_luoshu_deviation};

#[cfg(feature = "ml")]
pub use encoder_codebert::{CodeBertEncoder, PoolingStrategy};

#[cfg(feature = "ml")]
pub use luoshu_encoder_ml::{HybridLuoShuEncoder, LuoShuMlEncoder, PoolingStrategy as LuoShuPoolingStrategy};