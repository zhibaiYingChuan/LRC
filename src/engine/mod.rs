// ============================================================
// 许可证: DaoTi Research License v1.0
// 受保护核心引擎 — 包含模型底层架构衍生的编码/检索/编排算法。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================

// ──────────────────────────────────────────────
// L1 应用层 (Application Layer)
// ──────────────────────────────────────────────
// 贡献者友好：修改这些模块无需理解道枢哲学。
// 专注于实用功能：编码、检索、管理、持久化。
// ──────────────────────────────────────────────
pub mod embedder; // v0.6.0 统一 Embedder 抽象层
pub mod encoder; // 编码器 trait 定义
pub mod encoder_registry; // 编码器注册表
pub mod hnsw; // HNSW 向量检索
pub mod llm_translator; // LLM 查询翻译
pub mod manager;
pub mod model_resolver; // ML 模型下载/解析
pub mod pooling; // 池化策略（CodeBERT + 洛书 ML 共享）
pub mod retriever;
pub mod rrf; // RRF 倒数排名融合（server + v1_api 共享） // 检索器 // 代码库管理器 (CoreManager)

#[cfg(feature = "ml")]
pub mod encoder_codebert; // CodeBERT 编码器实现

// v0.6.0+：model_downloader 仅依赖 ureq + std，无需 ml 重依赖。
// 在 server feature 下也可用，供仪表盘嵌入模型管理 API 调用。
#[cfg(any(feature = "ml", feature = "server"))]
pub mod model_downloader; // v0.6.0 模型下载器（进度回调 + 重试 + 镜像源）

// ──────────────────────────────────────────────
// L2 核心引擎层 (Core Engine Layer)
// ──────────────────────────────────────────────
// 受 DaoTi Research License v1.0 保护。
// 仅限已接受许可证的贡献者开发。
// ──────────────────────────────────────────────
pub mod luoshu_encoder; // 洛书编码器 — 乾卦·天 (☰)
pub mod mirror_trapezoid; // 镜像梯形 — 中宫 (五)

#[cfg(feature = "ml")]
pub mod luoshu_encoder_ml; // 洛书编码器 ML 模式 — 艮卦·山 (☶)

// L2 自愈系统：记忆生命体的自我调节能力
pub mod audit_trail; // 审计追踪 — 离卦·火 (☲)
pub mod complexity_budget; // 复杂度预算 — 艮卦·山 (☶)
pub mod dao_evolution;
pub mod dao_metrics; // 道同构度 — 巽卦·风 (☴)
pub mod dao_regulator; // 道调节器 — 震卦·雷 (☳)
pub mod health_report; // 健康报告 — 乾卦·天 (☰)
pub mod memory_gc; // 记忆回收 — 坎卦·水 (☵)
pub mod synthesis_engine; // 合成引擎 — 离卦·火 (☲)
pub mod synthesis_journal; // 合成日志 — 兑卦·泽 (☱)
pub mod user_feedback; // 用户反馈 — 坤卦·地 (☷) // 道枢演化 — 中宫 (五)

pub use audit_trail::{AuditEvent, AuditEventType, AuditQuery, AuditTrail, IntegrityVerification};
pub use complexity_budget::{
    CausalChain, CausalLink, ComplexityBudget, ComplexityLimit, ImpactType, RedLineResult,
    RedLineSeverity, RedLineViolation, RiskLevel,
};
pub use dao_evolution::{
    AcceptedEvolution, DaoEvolutionProtocol, DaoProposal, DualModeValueDeclaration,
    PhilosophicalValue, PragmaticValue, ProposalStatus,
};
pub use dao_metrics::{compute_avg_luoshu_deviation, DaoMetrics, DaoMetricsSnapshot};
pub use dao_regulator::{
    CatastrophicEvent, CouplingTrendAnalysis, DaoRegulator, DaoRegulatorState, RegulationAction,
};
pub use encoder::{CodeEncoder, EmbeddingVector, FastEncoder};
pub use encoder_registry::EncoderRegistry;
pub use embedder::{EmbedError, Embedder}; // v0.6.0 统一嵌入器抽象
#[cfg(feature = "ml")]
pub use embedder::LocalBertEmbedder; // v0.6.0 本地 BERT 嵌入器
#[cfg(feature = "server")]
pub use embedder::LlmApiEmbedder; // v0.6.0 LLM API 嵌入器
// v0.6.0+：model_downloader 在 server 或 ml feature 下均导出
#[cfg(any(feature = "ml", feature = "server"))]
pub use model_downloader::{
    build_download_url, manual_download_guide, ConsoleProgress, DownloadConfig, DownloadError,
    DownloadProgress, MirrorSource, ModelDownloader,
}; // v0.6.0 模型下载器
pub use health_report::{
    generate_health_report, MemoryHealthStats, SystemHealthReport, SystemMode,
};
pub use hnsw::HnswRetriever;
pub use llm_translator::LlmApiConfig;
pub use luoshu_encoder::{EncoderStatus, LuoShuEncoder, LuoShuVector};
pub use manager::{ChunkStats, CoreManager};
pub use memory_gc::{
    GcCandidate, GcConfig, GcStats, MemoryGarbageCollector, MemoryInfoQuery, MemorySnapshot,
};
pub use mirror_trapezoid::{
    evolution_cycle, mirror_project, recursive_compose, recursive_unfold, BaguaProjection,
    ComposeResult, TrapezoidFocusResult, TrapezoidROI, UnfoldResult, BAGUA_CATEGORIES, BAGUA_NAMES,
};
pub use retriever::{CodeRetriever, LocalRetriever, RetrievalResult, ScoredChunk};
pub use synthesis_engine::{SynthesisConfig, SynthesisEngine};
pub use synthesis_journal::{SynthesisEvent, SynthesisJournal, SynthesisJournalSnapshot};
pub use user_feedback::{
    AffectedMemoryInfo, FeedbackRecord, FeedbackStats, FeedbackTarget, FeedbackType,
    ImpactAssessment, ImplicitSignal, ImplicitSignalType, MemoryGraphQuery, PendingActionType,
    UserFeedback,
};

pub use pooling::PoolingStrategy;

#[cfg(feature = "ml")]
pub use encoder_codebert::CodeBertEncoder;

#[cfg(feature = "ml")]
pub use luoshu_encoder_ml::{HybridLuoShuEncoder, LuoShuMlEncoder};

// 核心引擎设计原则：
// 1. 每个模块设计遵循"内禀调节"原则，从系统动力学自然涌现
// 2. 所有阈值参数均有严格的数学推导依据
// 3. 架构设计在实用性与理论完整性之间保持平衡
