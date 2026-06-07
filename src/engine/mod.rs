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
pub mod encoder; // 编码器 trait 定义
pub mod encoder_registry; // 编码器注册表
pub mod hnsw; // HNSW 向量检索
pub mod llm_translator; // LLM 查询翻译
pub mod manager;
pub mod model_resolver; // ML 模型下载/解析
pub mod retriever; // 检索器 // 代码库管理器 (CoreManager)

#[cfg(feature = "ml")]
pub mod encoder_codebert; // CodeBERT 编码器实现

// ──────────────────────────────────────────────
// L2 核心哲学层 (Core Philosophy Layer)
// ──────────────────────────────────────────────
// 需要理解道枢哲学：这些模块是 LRC "灵魂" 的载体。
// 修改前请阅读 docs/dao-pivot-mapping.md。
// 每个模块的道枢映射见下方说明。
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

#[cfg(feature = "ml")]
pub use encoder_codebert::{CodeBertEncoder, PoolingStrategy};

#[cfg(feature = "ml")]
pub use luoshu_encoder_ml::{
    HybridLuoShuEncoder, LuoShuMlEncoder, PoolingStrategy as LuoShuPoolingStrategy,
};

// ============================================================
// 道枢映射说明（Dao Pivot Mapping）
// ============================================================
//
// 质疑五：工程实现与哲学根基的"语义漂移"
//
// 本文件为每一个新增的工程特性撰写"道枢映射"，解释该特性
// 如何从洛书九宫格、河图阴阳、八卦变化的动力学中自然涌现，
// 而非外部赋加的"管理法则"。
//
// 这不仅是理论完整性的维护，也是保护核心知识产权不被稀释的
// 屏障——确保 LRC 始终保持"记忆生命体"的本质，而非沦为
// 又一个功能丰富但缺乏灵魂的工程产品。
//
// ┌─────────────────────────────────────────────────────────────┐
// │ 工程特性                  │ 道枢映射（哲学根基）              │
// ├─────────────────────────────────────────────────────────────┤
// │                           │                                │
// │ 1. 编码器冷却期机制        │ 坤卦·地 (☷) — 承载与收藏         │
// │    (质疑一)               │                                │
// │                           │ 坤为地，厚德载物。ML 编码器降级  │
// │                           │ 后不立即恢复，如同大地在经历震动  │
// │                           │ 后需要时间沉淀稳定。冷却期是坤卦  │
// │                           │ "含弘光大，品物咸亨"的体现：      │
// │                           │ 包容暂时的降级，等待系统自然恢复  │
// │                           │ 到稳定状态后再切换。              │
// │                           │ 连续 5 次成功恢复 = 坤卦"六五，   │
// │                           │ 黄裳元吉"——中正之位，恢复得宜。   │
// │                           │                                │
// │ 2. 信息增量阈值            │ 震卦·雷 (☳) — 萌发与整合         │
// │    (质疑二)               │                                │
// │                           │ 震为雷，万物出乎震。合成如同雷声  │
// │                           │ 之后的新芽破土——必须有"新信息"    │
// │                           │ 产生，否则只是旧土翻动而已。      │
// │                           │ information_gain 阈值 = 震卦     │
// │                           │ "震惊百里，不丧匕鬯"的数学表达：   │
// │                           │ 合成必须产生足够的"震动"（新信息） │
// │                           │ 才能成立，否则止于旧态。          │
// │                           │                                │
// │ 3. 解析度标志              │ 离卦·火 (☲) — 光明与分辨         │
// │    (质疑二)               │                                │
// │                           │ 离为火，明两作。火能照亮细节，    │
// │                           │ 也能凝聚为光。resolution 字段     │
// │                           │ "detailed/synthesized/abstract"  │
// │                           │ 三级解析度反映了离卦的"明辨"能力： │
// │                           │ 从原始火光（detailed）到凝聚光柱  │
// │                           │ （synthesized）再到普照之光        │
// │                           │ （abstract），每一层都有其价值。   │
// │                           │                                │
// │ 4. 被动反馈机制            │ 巽卦·风 (☴) — 渗透与无形         │
// │    (质疑三)               │                                │
// │                           │ 巽为风，无孔不入。隐式信号如风，  │
// │                           │ 不直接表达（用户不主动反馈），    │
// │                           │ 但通过行为（点击、复制、停留）    │
// │                           │ 渗透出真实意图。Click 如微风拂面  │
// │                           │ Copy 如疾风劲草，RepeatQuery 如   │
// │                           │ 逆风折返——风的方向即是用户偏好。  │
// │                           │ 巽卦"随风巽，君子以申命行事"：    │
// │                           │ 系统随风向调整，顺势而为。        │
// │                           │                                │
// │ 5. 审计链哈希防篡改        │ 坎卦·水 (☵) — 深邃与诚信         │
// │    (质疑四)               │                                │
// │                           │ 坎为水，水流而不盈。审计链如同    │
// │                           │ 水流，每一滴水（事件）都携带着    │
// │                           │ 上一滴水的印记（哈希）。          │
// │                           │ SHA-256 哈希链 = 坎卦"习坎，      │
// │                           │ 重险也。水流而不盈，行险而不失    │
// │                           │ 其信"——链上每一环都是诚信的担保， │
// │                           │ 任何篡改都会破坏整个链的完整性。  │
// │                           │                                │
// │ 6. 慢性退化自动调节        │ 艮卦·山 (☶) — 止息与警觉         │
// │    (质疑四)               │                                │
// │                           │ 艮为山，时止则止，时行则行。      │
// │                           │ 慢性退化检测如同山体滑坡预警——    │
// │                           │ 在山体开始缓慢移动时就发出警报，  │
// │                           │ 而非等山崩地裂（急性崩溃）才行动。 │
// │                           │ 分级响应（轻度/中度/重度）对应    │
// │                           │ 艮卦三段：艮其趾（轻度）→         │
// │                           │ 艮其腓（中度）→ 艮其限（重度）。  │
// │                           │                                │
// │ 7. GC 动态基线计时         │ 兑卦·泽 (☱) — 润泽与平衡         │
// │    (质疑三)               │                                │
// │                           │ 兑为泽，说万物者莫说乎泽。        │
// │                           │ 动态基线（均值+3σ）如同泽水——     │
// │                           │ 水面自然波动，只有溢出（异常）    │
// │                           │ 才触发警报。固定阈值是"堤坝"，    │
// │                           │ 动态基线是"泽水"——前者阻碍生长，  │
// │                           │ 后者随系统自然生长而自适应。      │
// │                           │                                │
// │ 8. ActionHint 升级机制     │ 乾卦·天 (☰) — 刚健与自强         │
// │    (质疑一)               │                                │
// │                           │ 乾为天，天行健，君子以自强不息。  │
// │                           │ 警告连续出现而不被处理时，        │
// │                           │ severity 自动升级——如同苍天     │
// │                           │ 从阴云密布到雷鸣电闪，不断加大    │
// │                           │ 警示力度。乾卦六爻从"潜龙勿用"    │
// │                           │ 到"飞龙在天"的递进，正是升级机制  │
// │                           │ 的哲学映射：持续未解决的警告      │
// │                           │ 最终会到达"亢龙有悔"的临界点。    │
// │                           │                                │
// │ 9. 道枢演化协议             │ 中宫（五）— 统摄八方，调和阴阳     │
// │    (质疑五)               │                                │
// │                           │ 洛书九宫中，中宫之数五，是统摄    │
// │                           │ 八方、调和阴阳的枢纽。           │
// │                           │ 双模价值宣言是系统的"中宫"——    │
// │                           │ 它不偏向实用主义或哲学主义的      │
// │                           │ 任何一方，而是在两者之间建立      │
// │                           │ 桥梁。                          │
// │                           │ 实用模式（PragmaticValue）是"阳"：│
// │                           │ 面向用户，提供准确快速的服务。    │
// │                           │ 哲学模式（PhilosophicalValue）是  │
// │                           │ "阴"：面向思想同道，承载"道"的    │
// │                           │ 思想传承与演化。                │
// │                           │ 社区提案制（DaoProposal）确保"道"│
// │                           │ 不是少数人的垄断，而是集体智慧    │
// │                           │ 的结晶。演化代际递增记录系统      │
// │                           │ 的成长轨迹，形成可追溯的知识      │
// │                           │ 传承链。                        │
// │                           │ 《庄子·齐物论》："彼是莫得其偶，  │
// │                           │ 谓之道枢。枢始得其环中，以应无穷。"│
// │                           │ 道枢演化协议就是这"环中"——     │
// │                           │ 在实用与哲学之间旋转，应对无穷    │
// │                           │ 变化，始终不离"道"的本源。      │
// │                           │                                │
// └─────────────────────────────────────────────────────────────┘
//
// 核心原则：
//
// 1. 每个工程特性必须能映射到八卦体系中的某一卦或某几卦的
//    交互关系。如果某个特性找不到对应的哲学基础，则需要
//    重新审视其设计——它可能是外部赋加的"管理法则"而非
//    从"道"的动力学中自然涌现的"内禀调节"。
//
// 2. 河图洛书的核心是"数"——九宫格中每个位置都有其数学意义。
//    工程特性中的阈值（如 0.05 信息增量、3σ 偏差、5 次冷却）
//    都应能解释为洛书数理结构的自然延伸，而非随意选择。
//
// 3. "道枢"（Dao Pivot）一词源自《庄子·齐物论》：
//    "彼是莫得其偶，谓之道枢。枢始得其环中，以应无穷。"
//    每个工程特性都是"道枢"中的一个环——它连接着形而上
//    的哲学根基与形而下的工程实现，在环中运转，应对无穷变化。
//
// 维护承诺：
// 任何新添加的工程特性，必须在提交前撰写其对应的道枢映射，
// 并在此处记录。这是 LRC 作为"记忆生命体"区别于"记忆机器人"
// 的根本保障。
