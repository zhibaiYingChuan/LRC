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
                 // v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P1-4「行尾注释错位」）：
                 //   原第 22 行把三条注释挤在同一行尾部（`pub mod rrf; // RRF ... // 检索器 // 代码库管理器`），
                 //   后两条注释实际描述的是上一行的 `retriever` 与更早的 `manager`，属排版错位。
                 //   现按各自归属拆分到对应模块旁。
pub mod retriever; // 检索器
pub mod rrf; // RRF 倒数排名融合（server + v1_api 共享）

#[cfg(feature = "ml")]
pub mod encoder_codebert; // CodeBERT 编码器实现

// v0.6.0+：model_downloader 仅依赖 ureq + std，无需 ml 重依赖。
// 在 server feature 下也可用，供仪表盘嵌入模型管理 API 调用。
#[cfg(any(feature = "ml", feature = "server"))]
pub mod model_downloader; // v0.6.0 模型下载器（进度回调 + 重试 + 镜像源）

// v0.6.0+ 参赛扩展：结构化探索日志模块
// 为"开放探索赛题"提供科学探索过程的结构化日志记录（JSON Lines 格式）
// 位置在 L1 应用层，因为它只是日志记录工具，不涉及核心算法
pub mod exploration_log;

// ──────────────────────────────────────────────
// L2 核心引擎层 (Core Engine Layer)
// ──────────────────────────────────────────────
// 受 DaoTi Research License v1.0 保护。
// 仅限已接受许可证的贡献者开发。
// ──────────────────────────────────────────────
pub mod luoshu_encoder; // 洛书编码器 — 乾卦·天 (☰)
pub mod mirror_trapezoid; // 镜像梯形 — 中宫 (五)
pub mod navigation; // 道体导航信号 — 检索前方向注入（多视图 deep + N 路 RRF）

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
                   // v0.9.7（P0-2 依赖倒置修复）：memory_state_machine 已上提至 Layer 1（`crate::memory_state_machine`），
                   // 因其文件头声明为 Apache 2.0 且零内部依赖，不应留在 DaoTi 许可的 engine/ 下。
                   // 此处保留再导出，使既有 `crate::engine::memory_state_machine::*` 路径（36 处引用）继续可用。
pub use crate::memory_state_machine;
pub mod synthesis_engine; // 合成引擎 — 离卦·火 (☲)
pub mod synthesis_journal; // 合成日志 — 兑卦·泽 (☱)
                           // v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P1-4「行尾注释错位」）：
                           //   原第 68 行把「道枢演化 — 中宫 (五)」误挂在 user_feedback 行尾——该注释
                           //   实际描述的是 `dao_evolution`（见上方 `pub mod dao_evolution;`），属排版错位。
pub mod user_feedback; // 用户反馈 — 坤卦·地 (☷)

// `engine/archive/` 为**有意保留**的归档位，非常规源码模块（故此处无 `pub mod`）：
//   CHANGELOG「引擎代码归档流程规范」约定——`src/engine/` 核心文件发生重大变更时，
//   旧版本文件必须先移入 `src/engine/archive/` 再删除，以便同一提交内保留可回溯实现。
//   目录当前仅含 `.gitkeep`（Git 不跟踪空目录，占位文件用于在克隆后保留该目录）。
//   审计提示：GLOBAL_CODE_REVIEW_REPORT 曾将其登记为 P3-1「空目录」，该定性过重——
//   它是流程占位而非残留，与 P0-1（未识别 Junction 而误判残留）同属
//   「未核对项目既有约定即判定为垃圾」的方法论问题。核对此项前请先查 CHANGELOG。

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
#[cfg(feature = "server")]
pub use embedder::LlmApiEmbedder;
#[cfg(feature = "ml")]
pub use embedder::LocalBertEmbedder; // v0.6.0 本地 BERT 嵌入器
pub use embedder::{EmbedError, Embedder}; // v0.6.0 统一嵌入器抽象
pub use encoder::{CodeEncoder, EmbeddingVector, FastEncoder};
pub use encoder_registry::EncoderRegistry; // v0.6.0 LLM API 嵌入器
                                           // v0.6.0+：model_downloader 在 server 或 ml feature 下均导出
pub use health_report::{
    generate_health_report, MemoryHealthStats, SystemHealthReport, SystemMode,
};
pub use hnsw::HnswRetriever;
pub use llm_translator::LlmApiConfig;
pub use luoshu_encoder::{EncoderStatus, LuoShuEncoder, LuoShuVector, LUOSHU_WEIGHTS};
pub use manager::{ChunkStats, CoreManager};
pub use memory_gc::{
    GcCandidate, GcConfig, GcStats, MemoryGarbageCollector, MemoryInfoQuery, MemorySnapshot,
};
pub use mirror_trapezoid::{
    evolution_cycle, mirror_project, recursive_compose, recursive_unfold, BaguaProjection,
    ComposeResult, TrapezoidFocusResult, TrapezoidROI, UnfoldResult, BAGUA_CATEGORIES, BAGUA_NAMES,
};
#[cfg(any(feature = "ml", feature = "server"))]
pub use model_downloader::{
    build_download_url, manual_download_guide, ConsoleProgress, DownloadConfig, DownloadError,
    DownloadProgress, MirrorSource, ModelDownloader,
}; // v0.6.0 模型下载器
pub use retriever::{CodeRetriever, LocalRetriever, RetrievalResult, ScoredChunk};
pub use synthesis_engine::{SynthesisConfig, SynthesisEngine};
pub use synthesis_journal::{SynthesisEvent, SynthesisJournal, SynthesisJournalSnapshot};
pub use user_feedback::{
    AffectedMemoryInfo, FeedbackRecord, FeedbackStats, FeedbackTarget, FeedbackType,
    ImpactAssessment, ImplicitSignal, ImplicitSignalType, MemoryGraphQuery, PendingActionType,
    UserFeedback,
};

pub use pooling::PoolingStrategy;

// v0.6.0+ 参赛扩展：探索日志公开导出
pub use exploration_log::{
    BaguaDistribution, DepthDistribution, ExplorationEventType, ExplorationLogEntry,
    ExplorationLogger, Metrics, SnapshotPayload,
};

#[cfg(feature = "ml")]
pub use encoder_codebert::CodeBertEncoder;

#[cfg(feature = "ml")]
pub use luoshu_encoder_ml::{HybridLuoShuEncoder, LuoShuMlEncoder};

// 签名迁移：本函数为 ml feature 专属，故错误别名同样按 feature 门控引入，
// 避免默认 feature（server）下产生 unused import 告警。
#[cfg(feature = "ml")]
use crate::errors::LrcResult;

/// v0.9.1 算法泄露合规：公开层（bin/server.rs）通过此工厂函数创建编码器，
/// 避免在公开层文件中直接引用受保护的算法类型名。
#[cfg(feature = "ml")]
pub fn create_smart_encoder() -> LrcResult<(HybridLuoShuEncoder, bool)> {
    match LuoShuMlEncoder::load() {
        Ok(ml) => Ok((HybridLuoShuEncoder::new_with_ml(ml), true)),
        Err(e) => Err(e),
    }
}

/// v0.9.1 算法泄露合规：统计模式编码器工厂函数
#[cfg(feature = "ml")]
pub fn create_statistical_encoder() -> HybridLuoShuEncoder {
    HybridLuoShuEncoder::new_statistical()
}

// 核心引擎设计原则：
// 1. 每个模块设计遵循"内禀调节"原则，从系统动力学自然涌现
// 2. 所有阈值参数均有严格的数学推导依据
// 3. 架构设计在实用性与理论完整性之间保持平衡
