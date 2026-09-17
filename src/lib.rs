//! Loong Recall (LRC) — 通用语义记忆引擎
//! ================================================
//!
//! 为 AI 助手提供跨项目、跨语言的永久记忆能力。
//! 支持多语言代码和通用文档的自动切分与检索。
//!
//! 分层开源架构:
//!   Layer 1 (Public)  — chunker.rs, server.rs, bin/server.rs → Apache 2.0
//!   Layer 2 (Protected) — engine/ 子模块 → 专有研究许可
//!   Layer 3 (Binary)  — 编译产物 → 二进制分发，Rust 编译天然混淆
//!
//! 架构: Chunker（多语言切分）→ Encoder（语义编码）→ Retriever（向量检索）→ Manager（编排）
//! 可选: Server（MCP HTTP/Stdio 服务）→ IDE 直接调用
//! 可选: CodeBERT（ml feature）→ 高精度语义编码

// === Layer 1: 公开层 (Apache 2.0) ===
pub mod chunker;
/// 统一错误契约（v0.9.7，GLOBAL_CODE_REVIEW_REPORT P1-6）
pub mod errors;
pub mod graph_store;
pub mod memory_store;
/// 记忆存储缓存子系统（v0.9.7：从 memory_store.rs 外提，P2-2 God Object 拆分）
pub mod memory_store_cache;
/// 记忆存储数据契约类型（v0.9.7：从 memory_store.rs 外提，P2-2 God Object 拆分）
pub mod memory_store_types;
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

/// v0.8.0 "归一"：记忆数据备份模块
pub mod backup;

/// v0.8.0 "归一"：数据操作日志模块
pub mod data_log;

/// 数据导出/导入模块：记忆备份与恢复
pub mod export;

/// 进程守护模块：单例锁、端口自适应、优雅关闭
pub mod process_guard;

/// 桌面仪表盘模块：双击 exe 打开浏览器查看记忆管理面板
pub mod dashboard;

/// 配置持久化模块：保存端口、LLM API、源码目录等
pub mod config;

/// URL 安全校验模块：集中式 SSRF 防护（云 metadata/链路本地/未指定地址拒绝）
pub mod url_safety;

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

/// 嵌入模型 ID 单一真源（Layer 1 中立，无 feature 门控，供 Layer 1/Layer 2 共用）
pub mod model_ids;

/// 原子文件写入工具（Layer 1 公共设施）
///
/// v0.9.7 新增（GLOBAL_CODE_REVIEW_REPORT P3-3「原子写入逻辑重复 4 处」）：
///   收敛 `arch_config.rs` / `config.rs` / `data_dir.rs` / `engine/audit_trail.rs`
///   四处各自手写的"临时文件 + rename"实现为唯一实现。
pub mod atomic_file;

/// 记忆联想状态机（Layer 1）
///
/// v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P0-2「依赖倒置 Layer 1 → Layer 2」）：
///   根因：本模块原位于 `engine/memory_state_machine.rs`（Layer 2，DaoTi 研究许可），
///         但其自身**文件头声明为 Apache 2.0 且零 crate 内部依赖**（仅 serde/std）——
///         属**许可层错位**。由于 Layer 1 的 `persistence/` 需要其中 `MemoryState`
///         作为持久化载体，形成 Layer 1 → Layer 2 的反向依赖。
///   修复：上提至 Layer 1 顶层。为不破坏既有 `crate::engine::memory_state_machine::*`
///         路径（36 处引用），`engine/mod.rs` 保留 `pub use` 再导出。
pub mod memory_state_machine;

/// 三层基准测试框架（可被 CLI、仪表盘 API、CI/CD 复用）
pub mod benchmark;

/// P7 主动发现通道（产品侧只消费不计算）：把状态机的漂移信号当作"触发源"，
/// 在独立只读路径上发起一次检查，结果以可忽略的提示呈现（判据见
/// daoti 研究资产目录下的 PREREG_ACTIVE_DISCOVERY.md）。
/// 许可边界：只消费 daemon 的 JSON 信号，不内置引擎/词典。
/// 门控 `LRC_ACTIVE_DISCOVERY` 默认关。
#[cfg(feature = "server")]
pub mod discovery;
// state_matcher 依赖 discovery（复用其账本/门控辅助），故同样按 server feature 门控。
#[cfg(feature = "server")]
pub mod state_matcher;

// === Layer 2: 受保护核心引擎（专有研究许可） ===
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

pub use memory_types::{
    DecayConfig, EntityKind, EventEntity, Importance, Memory, MemoryType, MemoryVersion,
    PrivacyLevel,
};

pub use persistence::{
    create_json_persistence, json::JsonPersistence, Persistence, PersistenceError,
};

pub use graph_store::{EdgeType, GraphMemoryStore, GraphQueryResult, MemoryEdge};

// === 架构记忆配置重导出 ===
pub use arch_config::{
    ArchConfig, EncoderArchConfig, PrivacyArchConfig, RetrievalArchConfig, SynthesisArchConfig,
};

// === 受保护核心重导出（仅导出接口类型，实现细节在 engine/ 中） ===
pub use engine::encoder::{CodeEncoder, EmbeddingVector};
pub use engine::hnsw::HnswRetriever;
pub use engine::llm_translator::LlmApiConfig;
pub use engine::manager::{ChunkStats, CoreManager as CodeMemoryManager};
pub use engine::retriever::{CodeRetriever, RetrievalResult, ScoredChunk};

#[cfg(feature = "ml")]
pub use engine::encoder_codebert::CodeBertEncoder;
#[cfg(feature = "ml")]
pub use engine::pooling::PoolingStrategy;

#[cfg(feature = "ml")]
pub use engine::luoshu_encoder_ml::{HybridLuoShuEncoder, LuoShuMlEncoder};

// === 持久化后端重导出 ===
