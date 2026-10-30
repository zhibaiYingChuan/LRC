//! Loong Recall (LRC) — 通用语义记忆引擎
//! ================================================
//!
//! 为 AI 助手提供跨项目、跨语言的永久记忆能力。
//! 支持多语言代码和通用文档的自动切分与检索。
//!
//! 分层开源架构（v0.9.11 / CR-52：改为**目录级**定义，与 LICENSE §2.2 口径一致）:
//!   Layer 1 (Public)  — `src/engine/` 目录**之外**的全部源码 → Apache 2.0
//!                        （含 chunker.rs、server.rs、bin/server.rs、v1_api.rs、
//!                          memory_store.rs、persistence/、privacy.rs 等）
//!   Layer 2 (Protected) — `src/engine/` 目录（及经 `pub use` 的再导出）→ 专有研究许可
//!   Layer 3 (Binary)  — 编译产物 → 二进制分发，Rust 编译天然混淆
//!
//! 注：`scripts/check_algorithm_leak.py` 的 `PUBLIC_FILES`（4 个文件）是**泄露扫描面**，
//!     是公开层的显式子集，**不等于**公开层全集。
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
#[cfg(test)]
mod build_profile_tests {
    //! 构建配置的**不变量守卫**（v0.9.11 / G8 / CR-06）

    /// ★守卫：release 必须 `panic = "unwind"`。
    ///
    /// ## 为什么这是"不变量"而不只是配置偏好
    /// 代码中大量隔离分支只在 unwind 下成立，例如
    /// `catch_unwind`、`tokio::spawn_blocking` 的 `Err(join_error) =>
    /// "…内部错误，服务已保持运行"`。若改回 `abort`：
    /// ① 这些分支成为**死逻辑**（承诺失效但无人察觉）；
    /// ② 一处 panic 即 abort 整个 sidecar，`Drop` 不执行 ⇒ 单例锁文件残留；
    /// ③ `overflow-checks = true` 的整数溢出也会升级为"整进程退出"。
    #[test]
    fn release_profile_must_use_unwind() {
        let manifest = include_str!("../Cargo.toml");
        let after = manifest
            .split("[profile.release]")
            .nth(1)
            .expect("Cargo.toml 必须包含 [profile.release] 段");
        // 截取到下一个段落头为止，避免误读其它 profile
        let section = after.split("\n[").next().unwrap_or(after);
        assert!(
            section.contains("panic = \"unwind\""),
            "release profile 必须为 panic = \"unwind\"（否则全部 panic 隔离失效，CR-06）。\n实际：{section}"
        );
        assert!(
            !section.contains("panic = \"abort\""),
            "release profile 不得使用 panic = \"abort\"（会让隔离分支成为死逻辑）"
        );
    }
}

/// 超时阶梯的**单一真源**（v0.9.11 / G4）：各层超时集中于此，并用守卫测试断言
/// "锁等待 < 内层 < 中层 < 外层"，防止"内层 ≥ 外层 ⇒ 取消失效"的不可达代码重现
pub mod timeouts;

/// **全 crate 唯一的测试环境变量锁**（v0.9.11 / G6 / CR-20）。
///
/// ## 为什么必须"唯一"
/// Rust 测试默认**并行**运行，而 `std::env::set_var` 改的是**进程级共享**状态。
/// 修前各模块各持一把自己的 `static LOCK: Mutex<()>`（`memory_state_machine`、
/// `state_matcher`、`dao_regulator`）—— **它们互不排斥**，于是"跨模块改同一变量"的竞态
/// 依然存在，且只表现为**偶发失败**（本会话实测到 `活性偏置门控必须默认关闭` 偶发失败）。
/// 局部修只是把竞态缩小到模块边界内，**没有消除它**；本模块提供唯一那把锁。
#[cfg(test)]
pub mod test_env {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static GLOBAL_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    /// 取得**全 crate 唯一**的 env 锁。所有读写进程环境变量的用例都必须先取它。
    ///
    /// 用法：`let _guard = crate::test_env::lock();`（守卫存活期间互斥）
    ///
    /// 中毒（某用例 panic）时**不传播 panic**：否则一个失败会连带炸掉其余所有用 env 的用例，
    /// 反而放大噪声、掩盖真实故障面。
    pub fn lock() -> MutexGuard<'static, ()> {
        GLOBAL_ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// ★守卫（v0.9.11 / G6 / CR-20）：**env 锁必须全 crate 唯一**。
    ///
    /// ## 为什么必须
    /// 修前 `memory_state_machine` / `state_matcher` / `dao_regulator` /
    /// `model_downloader` 各持一把**模块私锁**。私锁只在**模块内**互斥，
    /// **模块之间互不排斥** —— 而同一个变量会被多个模块的用例改写
    /// （`LRC_STATE_BIAS` 既被 `memory_state_machine` 又被 `memory_store_tests` 改写）。
    /// 于是"跨模块竞态"依旧存在，只从**必现**退化为**偶发**（更难定位）。
    /// 单一真源要求：只允许一把锁，所有改进程环境变量的用例都取它。
    ///
    /// 本守卫以**源码文本**断言防止有人再引入模块私锁（回归防线）。
    #[test]
    fn env_lock_must_be_unique_across_crate() {
        // 会改写**进程级**环境变量的全部测试源文件（相对 src/）。
        let sources: &[(&str, &str)] = &[
            ("state_matcher.rs", include_str!("state_matcher.rs")),
            (
                "memory_state_machine.rs",
                include_str!("memory_state_machine.rs"),
            ),
            (
                "engine/model_resolver.rs",
                include_str!("engine/model_resolver.rs"),
            ),
            (
                "engine/dao_regulator.rs",
                include_str!("engine/dao_regulator.rs"),
            ),
            (
                "engine/model_downloader.rs",
                include_str!("engine/model_downloader.rs"),
            ),
            (
                "memory_store_tests.rs",
                include_str!("memory_store_tests.rs"),
            ),
            ("v1_api_tests.rs", include_str!("v1_api_tests.rs")),
            ("process_guard.rs", include_str!("process_guard.rs")),
            ("discovery.rs", include_str!("discovery.rs")),
        ];
        for (name, src) in sources {
            assert!(
                src.contains("crate::test_env::lock()"),
                "{name} 会改进程环境变量，必须使用全 crate 唯一 env 锁 \
                 `crate::test_env::lock()`（CR-20）"
            );
            // `Mutex::new(())` 是模块私锁（`static X: Mutex<()> = Mutex::new(())`）的
            // 特征写法；一旦出现即说明私锁被重新引入 —— 模块间互不排斥，竞态回归。
            assert!(
                !src.contains("Mutex::new(())"),
                "{name} 不得再声明模块私锁（`static ...: Mutex<()> = Mutex::new(())`）——\
                 私锁模块间互不排斥，跨模块环境变量竞态仍在（CR-20）"
            );
        }
    }
}
/// 记忆写入的**单一真源**（v0.9.11：统一入参 `MemoryWriteInput` 与合并语义 `apply_merge`，
/// 消除"字段集按端点各自定义"与"合并规则散落内联"造成的静默丢字段）
pub mod memory_write;
pub mod persistence;
/// 可见性作用域的**单一真源**（v0.9.11 / G3）：`scoped` / `scope_from_args` 统一构造隐私上下文，
/// `TrustMode` + `require_scope` 实现"本地可信域保留、对外域强制"的分层隔离
pub mod privacy;

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

/// 记忆生命周期（取代 / 衰减 / 抑制）的独立持久状态（Layer 1）。
///
/// 刻意**不放进** `Memory` 结构：旧版 exe 读到未知字段会静默丢弃，并在下一次
/// 全量写入时抹掉（回滚变单向门）；且 `memories.json` 的整库压实是全量重写，
/// 检索命中不应触发它（线上已有 503 风暴前例）。因此单独存 `lifecycle.json`。
pub mod lifecycle;

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

// v0.9.10 塔缩修复·第 2 层：历史重分类的异步任务与进度状态机（依赖 v1_api 的 SharedStore）。
// 命名说明：模块与文件采用中性命名 `reclassify_job`，使**公开层**（本文件）
// 不出现受保护术语，与 `engine` 同属"公开层用中性名"的既有约定。
#[cfg(feature = "server")]
pub mod reclassify_job;

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
