// Loong Recall (L-RC / 忆) — 通用语义记忆引擎
// ================================================
//
// 源于道体（DaoTi）道枢层（Core Layer）的语义编码与检索技术，
// 作为 Loong Agent OS 的记忆子系统独立开源。
//
// 支持多语言代码（Rust / Python / TypeScript / JavaScript / Go）
// 及通用文档（Markdown / Text / YAML / TOML / JSON）的自动切分与检索，
// 为 AI 助手提供跨项目、跨语言的永久记忆能力。
//
// 分层开源架构（遵循 DaoTi 分层协议）:
//   Layer 1 (Public)  — chunker.rs, server.rs, bin/server.rs → Apache 2.0
//   Layer 2 (Protected) — engine/ 子模块 → DaoTi Research License v1.0
//   Layer 3 (Binary)  — 编译产物 → 二进制分发，Rust 编译天然混淆
//
// 架构：Chunker（多语言切分）→ Encoder（语义编码）→ Retriever（向量检索）→ Manager（编排）
// 可选：Server（MCP HTTP/Stdio 服务）→ IDE 直接调用
// 可选：CodeBERT（ml feature）→ 高精度语义编码

// === Layer 1: 公开层 (Apache 2.0) ===
pub mod chunker;
pub mod memory_store;
pub mod memory_types;
pub mod persistence;

/// 运行时防护模块：反调试、完整性校验、防篡改
pub mod guard;

// === Layer 2: 受保护核心引擎 (DaoTi Research License v1.0) ===
pub mod engine;

// === Layer 1: MCP 服务层 (Apache 2.0) ===
#[cfg(feature = "server")]
pub mod server;

// === 公开重导出 ===
pub use chunker::{
    chunk_by_language, detect_language, is_supported_file, CodeChunk, CodeChunker,
    ConversationChunker, GenericChunker, GoChunker, PythonChunker, RustChunker, TsJsChunker,
};

pub use memory_store::{
    ListFilter, MemoryStats, MemoryStore, RecallFilter, RecallResult, SortBy, SortOrder,
};

pub use memory_types::{Importance, Memory, MemoryType};

pub use persistence::{
    create_json_persistence, json::JsonPersistence, Persistence, PersistenceError,
};

// === 受保护核心重导出（仅导出接口类型，实现细节在 engine/ 中） ===
pub use engine::encoder::{CodeEncoder, EmbeddingVector};
pub use engine::hnsw::HnswRetriever;
pub use engine::manager::{ChunkStats, CoreManager as CodeMemoryManager};
pub use engine::retriever::{CodeRetriever, RetrievalResult, ScoredChunk};

#[cfg(feature = "ml")]
pub use engine::encoder_codebert::{CodeBertEncoder, PoolingStrategy};