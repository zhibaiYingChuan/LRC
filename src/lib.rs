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
pub mod graph_store;
pub mod memory_store;
pub mod memory_types;
pub mod persistence;

/// 运行时防护模块：反调试、完整性校验、防篡改
pub mod guard;

/// 项目指纹模块：跨IDE项目身份标准化（SHA256哈希）
pub mod project_id;

/// 统一数据目录管理：V2 标准化数据目录结构
pub mod data_dir;

/// 数据迁移模块：旧版 V1 数据自动迁移到 V2 结构
pub mod migration;

/// 数据导出/导入模块：记忆备份与恢复
pub mod export;

/// 进程守护模块：单例锁、端口自适应、优雅关闭
pub mod process_guard;

/// 桌面仪表盘模块：双击 exe 打开浏览器查看记忆管理面板
pub mod dashboard;

/// 配置持久化模块：保存端口、LLM API、源码目录等
pub mod config;

/// L1 数据加密模块：API Key 安全存储（与桌面端共享密钥文件）
pub mod crypto;

/// 系统托盘模块：桌面端后台运行，托盘图标+菜单
#[cfg(feature = "webbrowser")]
pub mod tray;

/// 后台结晶流水线（定时 consolidation job）
pub mod consolidation;

/// A/B 测试框架与 MRR 评估（Phase 1 影子验证）
pub mod ab_test;

/// 架构记忆配置（算子参数、衰减曲线、权限策略持久化）
pub mod arch_config;

/// 三层基准测试框架（可被 CLI、仪表盘 API、CI/CD 复用）
pub mod benchmark;

// === Layer 2: 受保护核心引擎 (DaoTi Research License v1.0) ===
pub mod engine;

// === Layer 1: MCP 服务层 (Apache 2.0) ===
#[cfg(feature = "server")]
pub mod server;

// === Layer 1: v1 REST API (Apache 2.0) ===
#[cfg(feature = "server")]
pub mod v1_api;

// === 公开重导出 ===
pub use chunker::{
    chunk_by_language, detect_language, is_supported_file, CodeChunk, CodeChunker,
    ConversationChunker, GenericChunker, GoChunker, PythonChunker, RustChunker, TsJsChunker,
};

pub use memory_store::{
    ListFilter, MemoryStats, MemoryStore, RecallFilter, RecallResult, SortBy, SortOrder,
};

pub use memory_types::{DecayConfig, Importance, Memory, MemoryType, MemoryVersion, PrivacyLevel};

pub use persistence::{
    create_json_persistence, json::JsonPersistence, Persistence, PersistenceError,
};

pub use graph_store::{EdgeType, GraphMemoryStore, GraphQueryResult, MemoryEdge};

// === 架构记忆配置重导出 ===
pub use arch_config::{
    ArchConfig, LuoshuArchConfig, PrivacyArchConfig, RetrievalArchConfig, SynthesisArchConfig,
};

// === 受保护核心重导出（仅导出接口类型，实现细节在 engine/ 中） ===
pub use engine::encoder::{CodeEncoder, EmbeddingVector};
pub use engine::hnsw::HnswRetriever;
pub use engine::llm_translator::LlmApiConfig;
pub use engine::manager::{ChunkStats, CoreManager as CodeMemoryManager};
pub use engine::retriever::{CodeRetriever, RetrievalResult, ScoredChunk};

#[cfg(feature = "ml")]
pub use engine::encoder_codebert::{CodeBertEncoder, PoolingStrategy};

#[cfg(feature = "ml")]
pub use engine::luoshu_encoder_ml::{HybridLuoShuEncoder, LuoShuMlEncoder};

// === 持久化后端重导出 ===
