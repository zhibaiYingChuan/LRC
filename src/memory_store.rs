//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现记忆存储管理层，属于公开层 (Layer 1)。
//! ============================================================
//!
//! 记忆存储管理器 (MemoryStore) — 架构概览
//!
//! MemoryStore 是记忆领域的 Aggregate Root，协调以下子系统：
//!
//! ┌─────────────────────────────────────────────────────────┐
//! │                    MemoryStore                         │
//! │  (协调层 — 薄封装，委托给专业引擎)                      │
//! ├─────────────────────────────────────────────────────────┤
//! │  • 写入/更新/删除 (remember, forget, update_memory)     │
//! │  • 检索 (recall, trapezoid_focus_recall)               │
//! │  • 列表/统计 (list_memories, stats)                    │
//! │  • 归档 (archive_expired)                             │
//! │  • 修正 (correct_memory, unfold_memory)               │
//! ├─────────────────────────────────────────────────────────┤
//! │  委托子系统:                                           │
//! │  ┌──────────────────┐  ┌──────────────────────────────┐ │
//! │  │ SynthesisEngine  │  │ DaoRegulator                 │ │
//! │  │ (合成引擎)        │  │ (道同构度调节器)              │ │
//! │  │ • 簇发现          │  │ • 健康检测                   │ │
//! │  │ • 摘要生成        │  │ • 自适应调节                 │ │
//! │  │ • 洛书合成        │  │ • 振荡防护                   │ │
//! │  └──────────────────┘  └──────────────────────────────┘ │
//! │  ┌──────────────────┐  ┌──────────────────────────────┐ │
//! │  │ SynthesisJournal │  │ DaoMetrics                   │ │
//! │  │ (合成日志)        │  │ (道同构度指标)                │ │
//! │  │ • 事件记录        │  │ • 幻和偏离度                 │ │
//! │  │ • 质量反馈        │  │ • 八卦熵                     │ │
//! │  │ • 命中追踪        │  │ • 合成比率                   │ │
//! │  └──────────────────┘  └──────────────────────────────┘ │
//! └─────────────────────────────────────────────────────────┘
//!
//! 调试入口：
//!   - 合成行为异常 → 查看 SynthesisJournal 日志 + SynthesisEngine 配置
//!   - 检索结果异常 → 查看 trapezoid_focus_recall 的 ROI 参数
//!   - 系统健康异常 → 查看 DaoRegulator 的调节历史 + DaoMetrics 快照
//!
//! ============================================================

use crate::engine::audit_trail::{AuditEventType, AuditTrail};
use crate::engine::complexity_budget::ComplexityBudget;
use crate::engine::dao_metrics::DaoMetrics;
use crate::engine::dao_regulator::{DaoRegulator, RegulationAction};
use crate::engine::health_report::HintEscalationTracker;
use crate::engine::health_report::{generate_health_report, SystemHealthReport};
#[cfg(not(feature = "ml"))]
use crate::engine::luoshu_encoder::LuoShuEncoder as HybridLuoShuEncoder;
use crate::engine::luoshu_encoder::LuoShuVector;
#[cfg(feature = "ml")]
use crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder;
use crate::engine::memory_gc::{GcStats, MemoryGarbageCollector, MemoryInfoQuery, MemorySnapshot};
use crate::engine::memory_state_machine::MemoryStateMachine;
use crate::engine::mirror_trapezoid::{
    bagua_name_to_index, mirror_project, recursive_unfold, TrapezoidROI,
};
use crate::engine::synthesis_engine::{SynthesisConfig, SynthesisEngine, SynthesisPlan};
use crate::engine::synthesis_journal::SynthesisJournal;
use crate::engine::user_feedback::{
    AffectedMemoryInfo, ImplicitSignal, MemoryGraphQuery, UserFeedback,
};
use crate::graph_store::{EdgeType, GraphMemoryStore};
use crate::memory_types::{DecayConfig, Importance, Memory, MemoryType, PrivacyLevel};
use crate::persistence::{AssocFrequency, Persistence, PersistenceError};

/// 记忆写入性能剖析输出
///
/// v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P2-1「use 语句被函数截断」）：
///   该函数原被插在 `use` 块中间（`complexity_budget` 与 `dao_metrics` 两行 use 之间），
///   把连续的导入声明切成两段，损害可读性。现移至全部 `use` 声明之后。
pub(crate) fn emit_remember_profile(line: String) {
    eprintln!("{line}");
    if let Some(path) = std::env::var_os("LRC_PROFILE_REMEMBER_FILE") {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes());
        }
    }
}

/// BM25 检索评分参数（v0.8.50 检索质量修复）：
/// 替代朴素 TF-IDF 的线性文档长度归一，抑制长文档（如大段源码/文档节选种子记忆）
/// 被过度稀释的问题。k1 控制词频饱和、b 控制长度归一强度，取 Lucene/ES 常用默认值。
const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;

/// 语义向量域过滤权重（v0.8.52）：
/// deep 路洛书向量是纯字符位置统计特征（density/entropy/position_weight），不含词义，
/// 短 query 与无关长记忆的余弦区分度低，实测 dev 库 32 组 query 中 deep top1 与 query
/// 零词面重合 23/32。在余弦之外叠加词面域重合分（候选池内平滑 IDF 加权），
/// final = 余弦 + LEX_DOMAIN_WEIGHT * (词面域分 / 域内最大值)，
/// 域外高余弦记忆无词面加分自然沉底。词面域是内容锚，不涉及八卦元数据，遵循方案 §3.5。
/// 权重可通过环境变量 LRC_DEEP_LEX_DOMAIN_WEIGHT 覆盖（阶段 B scale 消融用），默认 0.25。
const LEX_DOMAIN_WEIGHT: f32 = 0.25;

/// 数据契约类型重导出（v0.9.7，GLOBAL_CODE_REVIEW_REPORT P2-2「MemoryStore God Object」）
///
/// 以下类型的**本体**已外提至 [`crate::memory_store_types`]（纯数据契约、零状态依赖）。
/// 在此重导出以保持 `crate::memory_store::Xxx` 既有路径与
/// `use crate::memory_store::*` 调用方**零改动**。
pub use crate::memory_store_types::{
    ListFilter, MemoryStats, RecallFilter, RecallResult, RegulatorHeartbeat, SortBy, SortOrder,
    SynthesisSnapshot,
};

pub struct MemoryStore<P: Persistence> {
    persistence: P,
    /// 冲突检测相似度阈值（0.0 ~ 1.0），默认 0.5
    /// 高于此阈值的记忆将被视为重复并自动合并
    similarity_threshold: f32,
    /// 合成触发阈值：相似记忆数量达到此值时触发递归合成（默认 3）
    pub synthesis_min_cluster: usize,
    /// 合成相似度阈值：记忆相似度超过此值时纳入同一簇（默认 0.4）
    pub synthesis_similarity: f32,
    /// 洛书编码器（用于记忆的 9 维坐标编码 + 八卦分类）
    luoshu_encoder: HybridLuoShuEncoder,
    /// 道同构度指标（L5 监控仪表）
    pub dao_metrics: DaoMetrics,
    /// 合成日志：记录每次合成事件，支持质量反馈闭环
    pub synthesis_journal: SynthesisJournal,
    /// 道同构度调节器：从感知到行动的闭环
    pub dao_regulator: DaoRegulator,
    /// P1 调节器心跳：记录最近一次 regulate() 的执行状态（供 /v1/health/system 与前端展示）
    pub regulator_heartbeat: RegulatorHeartbeat,
    /// 合成引擎：记忆簇发现与递归合成
    pub synthesis_engine: SynthesisEngine,
    /// 衰减曲线配置（可外部化，控制记忆衰减行为）
    pub decay_config: DecayConfig,
    /// 可选图存储（用于自动建立冲突/演进关系边）
    graph_store: Option<GraphMemoryStore>,
    /// LRC 内置道体状态机：跟踪当前活跃记忆与联想转移
    pub memory_state_machine: MemoryStateMachine,
    /// 用户反馈回路：将人类判断力注入系统演化（解决质疑四）
    pub user_feedback: UserFeedback,
    /// 自主记忆垃圾回收器：定期清理低质量、长期未用的记忆
    pub memory_gc: MemoryGarbageCollector,
    /// GC 延迟执行标记（质疑三：避免 GC 阻塞用户请求关键路径）
    /// v0.8.48 P0 修复：改为 AtomicBool 解决竞态条件
    /// 多个后台任务并发检查时，使用原子 CAS 操作确保 GC 恰好执行一次
    pub gc_pending: std::sync::atomic::AtomicBool,
    /// v0.5.4 合成延迟执行标记：避免合成阻塞用户请求关键路径
    /// 写入记忆后设为 true，由后台健康检查或定时任务触发执行
    /// v0.8.48 P0 修复：改为 AtomicBool 解决竞态条件
    /// 多个后台任务并发检查时，使用原子 CAS 操作确保合成恰好执行一次
    pub synthesis_pending: std::sync::atomic::AtomicBool,
    /// 审计追踪：记录系统所有自主行为（质疑五：透明度与信任）
    pub audit_trail: AuditTrail,
    /// 提示升级追踪器：防止 ActionHint 重复警告的"狼来了"效应（质疑一）
    pub hint_escalation: HintEscalationTracker,
    /// 复杂度预算（质疑五·终极：防止系统超出人类可理解范围）
    pub complexity_budget: ComplexityBudget,
    /// v0.5.4 增量缓存 + v0.9.7 结构化拆分（GLOBAL_CODE_REVIEW_REPORT P2-2）：
    /// 本地缓存子系统，原 6 个缓存字段（memory_cache / cache_dirty / bigram_index /
    /// bigram_index_dirty / recall_documents / assoc_frequency）及其全部纯缓存逻辑
    /// 已外提至 [`crate::memory_store_cache::MemoryStoreCache`]，此处只保留单一字段。
    cache: crate::memory_store_cache::MemoryStoreCache,
    /// v0.5.5 P1-1：LLM 是否已配置
    /// LLM 配置后替代本地 ML 模型提供语义理解能力，编码器不再视为"降级"
    /// 通过 set_llm_configured() 在 sidecar 启动后设置
    llm_configured: std::sync::atomic::AtomicBool,
    /// v0.6.0+ 参赛扩展：探索日志记录器（默认禁用，由 sidecar 启动时注入）
    /// 用于记录科学探索过程的结构化日志（JSON Lines 格式）
    exploration_logger: crate::engine::exploration_log::ExplorationLogger,
}

// ============================================================
// MemoryGraphQuery trait 实现（供两阶段确认的影响评估使用）
// ============================================================

impl<P: Persistence> MemoryGraphQuery for MemoryStore<P> {
    /// 查询与指定记忆直接关联的记忆数
    fn count_direct_neighbors(&self, memory_id: &str) -> usize {
        // 通过 source_ids 反向查找：哪些记忆引用了该记忆
        let all = match self.load_cached() {
            Ok(memories) => memories,
            Err(_) => return 0,
        };
        // 统计有多少其他记忆的 source_ids 中包含此记忆 ID
        all.iter()
            .filter(|m| m.source_ids.contains(&memory_id.to_string()))
            .count()
    }

    /// 查询与指定记忆关联的记忆 ID 列表及关系类型
    fn get_neighbor_info(&self, memory_id: &str) -> Vec<AffectedMemoryInfo> {
        let all = match self.load_cached() {
            Ok(memories) => memories,
            Err(_) => return Vec::new(),
        };

        // 收集所有引用此记忆的其他记忆（source_ids 反向查找）
        let mut neighbors = Vec::new();
        for m in &all {
            if m.source_ids.contains(&memory_id.to_string()) {
                neighbors.push(AffectedMemoryInfo {
                    memory_id: m.id.clone(),
                    memory_type: m.memory_type.as_str().to_string(),
                    relation_type: "synthesizes_from".to_string(),
                    weight: m.confidence.unwrap_or(0.5),
                    depth: 0, // 由调用方（request_impact_assessment）设置
                });
            }
        }
        // 如果图存储可用，也查询图中的边关系
        if let Some(ref graph) = self.graph_store {
            for edge in graph.query_edges(memory_id) {
                let other = if edge.source_id == memory_id {
                    &edge.target_id
                } else {
                    &edge.source_id
                };
                // 避免重复添加已在 source_ids 中的记忆
                if !neighbors.iter().any(|n| n.memory_id == *other) {
                    neighbors.push(AffectedMemoryInfo {
                        memory_id: other.clone(),
                        memory_type: "fact".to_string(),
                        relation_type: format!("{:?}", edge.edge_type).to_lowercase(),
                        weight: edge.weight,
                        depth: 0, // 由调用方（request_impact_assessment）设置
                    });
                }
            }
        }
        neighbors
    }

    /// 查询记忆是否为核心合成节点（被多条合成边引用）
    fn is_core_synthesis_node(&self, memory_id: &str) -> bool {
        // 核心节点判定：被 ≥ 3 条其他记忆的 source_ids 引用
        let all = match self.load_cached() {
            Ok(memories) => memories,
            Err(_) => return false,
        };
        let ref_count = all
            .iter()
            .filter(|m| m.source_ids.contains(&memory_id.to_string()))
            .count();
        ref_count >= 3
    }

    /// 查询受影响的合成链数量
    fn count_affected_synthesis_chains(&self, memory_ids: &[String]) -> usize {
        let all = match self.load_cached() {
            Ok(memories) => memories,
            Err(_) => return 0,
        };

        // 收集所有受影响记忆的 source_ids，去重后统计合成链数
        let mut affected_chains = std::collections::HashSet::new();
        for target_id in memory_ids {
            for m in &all {
                if m.source_ids.contains(target_id) {
                    // 每条合成边代表一条链
                    affected_chains.insert(format!("{}->{}", target_id, m.id));
                }
            }
        }
        affected_chains.len()
    }
}

// ============================================================
// MemoryInfoQuery trait 实现（供自主内存垃圾回收器使用）
// ============================================================

impl<P: Persistence> MemoryInfoQuery for MemoryStore<P> {
    fn get_last_accessed_ms(&self, memory_id: &str) -> Option<u64> {
        let all = self.load_cached().ok()?;
        all.iter()
            .find(|m| m.id == memory_id)
            .map(|m| m.last_accessed.timestamp_millis() as u64)
    }

    fn get_importance(&self, memory_id: &str) -> Option<u8> {
        let all = self.load_cached().ok()?;
        all.iter()
            .find(|m| m.id == memory_id)
            .map(|m| m.importance.value())
    }

    fn get_memory_type(&self, memory_id: &str) -> Option<String> {
        let all = self.load_cached().ok()?;
        all.iter()
            .find(|m| m.id == memory_id)
            .map(|m| m.memory_type.as_str().to_string())
    }

    fn get_reference_count(&self, memory_id: &str) -> usize {
        let all = match self.load_cached() {
            Ok(memories) => memories,
            Err(_) => return 0,
        };
        all.iter()
            .filter(|m| m.source_ids.contains(&memory_id.to_string()))
            .count()
    }

    fn is_core_synthesis_node(&self, memory_id: &str) -> bool {
        // 复用 MemoryGraphQuery 的实现
        MemoryGraphQuery::is_core_synthesis_node(self, memory_id)
    }

    fn is_low_quality_synthesis(&self, memory_id: &str) -> bool {
        self.synthesis_journal
            .get_low_quality_ids()
            .contains(&memory_id.to_string())
    }

    fn get_quality_score(&self, memory_id: &str) -> f32 {
        // 从合成日志中获取质量评分（通过命中记录计算）
        let events = self.synthesis_journal.get_events();
        for event in &events {
            if event.synthesis_id == memory_id {
                return event.avg_relevance;
            }
        }
        // 未在合成日志中 → 默认中等质量
        0.5
    }

    fn get_negative_feedback_count(&self, memory_id: &str) -> usize {
        self.user_feedback.get_negative_feedback_count(memory_id)
    }

    fn get_all_memory_ids(&self) -> Vec<String> {
        match self.load_cached() {
            Ok(memories) => memories.iter().map(|m| m.id.clone()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn delete_memory(&mut self, memory_id: &str) -> bool {
        let result = self
            .persistence
            .delete_memory(memory_id)
            .unwrap_or_else(|e| {
                eprintln!("[memory_store] 删除记忆失败 ({}): {}", memory_id, e);
                false
            });
        // P2-1 修复：GC 删除记忆时记录审计事件（MemoryDeleted），补全审计追踪覆盖
        if result {
            self.record_audit(
                AuditEventType::MemoryDeleted,
                format!("GC 删除记忆 {}", memory_id),
                "自主垃圾回收器删除过期/低质量记忆",
                vec![memory_id.to_string()],
            );
        }
        // v0.5.4 写操作后标记缓存为脏
        self.invalidate_cache();
        result
    }
}

// ============================================================
// v0.5.4 P1-9 修复：中文检索精度 — CJK 分词辅助函数
// ============================================================

/// 计算 CJK 字符在文本中的比例（0.0 ~ 1.0）
///
/// 当 CJK 字符比例超过 30% 时，应使用 bigram 分词策略。
fn cjk_ratio(text: &str) -> f32 {
    let total = text.chars().filter(|c| !c.is_whitespace()).count();
    if total == 0 {
        return 0.0;
    }
    let cjk_count = text
        .chars()
        .filter(|c| {
            let cp = *c as u32;
            (0x4E00..=0x9FFF).contains(&cp)
                || (0x3400..=0x4DBF).contains(&cp)
                || (0xF900..=0xFAFF).contains(&cp)
        })
        .count();
    cjk_count as f32 / total as f32
}

/// v0.5.4 P1-9 修复：智能分词函数
///
/// 根据文本的 CJK 字符比例自动选择分词策略：
/// - CJK 比例 ≥ 30%：使用字符级 bigram 分词（中文友好）
/// - CJK 比例 < 30%：使用空格分词（英文/混合文本）
///
/// 返回分词后的 token 列表（已转为小写）。
///
/// # 示例
///
/// ```ignore
/// // 中文文本 → bigram 分词
/// let tokens = tokenize_query("数据库连接");
/// assert!(tokens.contains(&"数据".to_string()));
/// assert!(tokens.contains(&"据库".to_string()));
/// assert!(tokens.contains(&"库连".to_string()));
/// assert!(tokens.contains(&"连接".to_string()));
///
/// // 英文文本 → 空格分词
/// let tokens = tokenize_query("database connection");
/// assert!(tokens.contains(&"database".to_string()));
/// ```
///
/// v0.9.7：开放为 pub——联想探索 API 层（v1_api）需要用同一套分词
/// 口径实现"根节点实质共鸣门禁"，避免分词逻辑被复制产生漂移。
pub fn tokenize_query(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();

    // CJK 比例 ≥ 30% 使用 bigram 分词
    if cjk_ratio(&lower) >= 0.3 {
        tokenize_cjk(&lower)
    } else {
        // 英文/混合文本：空格分词 + 过滤空 token
        lower
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

/// 判断一个分词 token 是否为"泛指 bigram"——两个字符都属于问句
/// 功能字/高频虚词的组合（如「是什」「什么」「怎么」「这个」）。
///
/// v0.9.7 联想根门禁专用：泛指 bigram 的命中不代表主题相关——
/// "量子物理是什么"会因「是什」「什么」两个泛指 bigram，与恰好
/// 含测试文本"是什么"的代码 chunk 虚假共鸣并抢走起点。过滤后
/// 门禁只统计实义 token（量子/物理/今晚/吃什 等）的重叠。
pub fn is_generic_bigram(token: &str) -> bool {
    const GENERIC_CJK_CHARS: &[char] = &[
        '是', '什', '么', '吗', '呢', '吧', '啊', '怎', '样', '办', '的', '了', '这', '那', '个',
    ];
    let mut chars = token.chars();
    let (Some(first), Some(second), None) = (chars.next(), chars.next(), chars.next()) else {
        return false;
    };
    GENERIC_CJK_CHARS.contains(&first) && GENERIC_CJK_CHARS.contains(&second)
}

/// CJK 字符级 bigram 分词（含混合语言兜底）
///
/// 将中文文本拆分为相邻字符对（bigram），例如 "数据库" → ["数据", "据库"]。
/// 对于长度 < 2 的文本，返回单字符 token。
///
/// v8 检索质量修复（混合语言兜底）：当文本为 "CJK + 英文/数字" 混合时（如
/// "所有try_lock()读操作改为try_read()"），连续 ASCII 字母/数字/下划线/连字符段
/// 被保留为独立 token（如 try_read、try_lock），而非按 bigram 切成 2 字符碎片，
/// 使 `contains_word` 能对这类标识符做整词边界匹配（词边界判定对长度 ≥ 3 的
/// ASCII 词生效）。中文部分仍按 bigram 切分，标点/符号作为分隔符。
///
/// 修复前引入的缺陷：混合文本被整体按字符切 bigram，英文标识符被切碎
/// （try_read → tr/ry/y_/_r/re/ea/ad），`contains_word` 对 2 字符英文仅做
/// 子串匹配，DF 高、IDF 低，正确记忆与大量无关文档同分甚至落榜。
fn tokenize_cjk(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();

    if chars.is_empty() {
        return Vec::new();
    }

    if chars.len() == 1 {
        return vec![chars[0].to_string()];
    }

    let mut result = Vec::new();
    // 连续 ASCII 字母/数字/下划线/连字符段（保留整词）
    let mut ascii_run = String::new();
    // CJK 等非 ASCII 字母段（按 bigram 切分）
    let mut cjk_run: Vec<char> = Vec::new();

    // 将 CJK 段按字符 bigram 产出 token
    let flush_cjk = |run: &mut Vec<char>, out: &mut Vec<String>| {
        if run.is_empty() {
            return;
        }
        if run.len() == 1 {
            out.push(run[0].to_string());
        } else {
            for pair in run.windows(2) {
                out.push(format!("{}{}", pair[0], pair[1]));
            }
        }
        run.clear();
    };

    for &c in &chars {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            // ASCII 字母/数字/下划线/连字符：刷新 CJK 段后累积到整词段
            flush_cjk(&mut cjk_run, &mut result);
            ascii_run.push(c);
        } else if c.is_alphabetic() && !c.is_ascii() {
            // CJK 等非 ASCII 字母：刷新英文段后累积到 CJK 段
            if !ascii_run.is_empty() {
                result.push(std::mem::take(&mut ascii_run));
            }
            cjk_run.push(c);
        } else {
            // 标点/符号：作为分隔符，刷新两侧段
            if !ascii_run.is_empty() {
                result.push(std::mem::take(&mut ascii_run));
            }
            flush_cjk(&mut cjk_run, &mut result);
        }
    }
    if !ascii_run.is_empty() {
        result.push(std::mem::take(&mut ascii_run));
    }
    flush_cjk(&mut cjk_run, &mut result);

    result
}

/// v0.5.4 P1-9 修复：计算文档长度（token 数量）
///
/// 与 `tokenize_query` 配合使用，确保 CJK 文本的文档长度基于 bigram 数量，
/// 而非 `split_whitespace().count()`（对中文无效）。
pub(crate) fn doc_token_count(text: &str) -> usize {
    tokenize_query(text).len().max(1)
}

/// v0.5.5 修复二：词边界感知的子串匹配
///
/// 解决 TF-IDF 检索中 "cat" 错误匹配 "category" 的问题。
///
/// 匹配规则：
/// - **CJK bigram**（包含 CJK 字符的 token）：保留 `contains()` 子串匹配
///   理由：CJK 文本无空格分词，bigram 本身就是 2 字符子串，子串匹配是合理的
/// - **英文单词**（纯 ASCII 字母）：要求词边界匹配
///   即匹配位置前后字符不能是字母（或位于字符串开头/结尾）
///
/// # 示例
///
/// ```ignore
/// // 英文单词：词边界匹配
/// assert!(contains_word("the cat sat", "cat"));
/// assert!(!contains_word("the category sat", "cat"));  // 不再误匹配
/// assert!(contains_word("cat", "cat"));               // 整词匹配
///
/// // CJK bigram：保留子串匹配
/// assert!(contains_word("数据库连接", "据库"));
/// ```
fn contains_word(content: &str, word: &str) -> bool {
    // 空 word 直接返回 false（防御性）
    if word.is_empty() {
        return false;
    }

    // 判断 word 是否为 CJK token（包含 CJK 字符）
    let is_cjk_token = word.chars().any(|c| {
        let cp = c as u32;
        (0x4E00..=0x9FFF).contains(&cp)
            || (0x3400..=0x4DBF).contains(&cp)
            || (0xF900..=0xFAFF).contains(&cp)
    });

    // CJK token：保留子串匹配（bigram 本身就是子串）
    if is_cjk_token {
        return content.contains(word);
    }

    // v0.5.5 修复：仅对长度 ≥ 3 的纯 ASCII 字母单词做词边界检测
    // 理由：CJK bigram 分词会把英文单词拆成 2 字符 bigram（如 "Rust" → "ru", "us", "st"），
    // 这些 2 字符 bigram 是纯 ASCII 字母，但应保留子串匹配（否则 "ru" 无法匹配 "rust"）。
    // 长度 ≥ 3 的英文单词（如 "cat", "category"）才做词边界检测，避免 "cat" 匹配 "category"。
    let char_count = word.chars().count();
    if char_count < 3 {
        return content.contains(word);
    }

    // 英文单词（长度 ≥ 3）：词边界匹配
    // 使用 char_indices 遍历所有匹配位置，检查前后字符是否为词边界
    let word_bytes = word.as_bytes();
    let content_bytes = content.as_bytes();
    let word_len = word_bytes.len();

    if word_len == 0 || word_len > content_bytes.len() {
        return false;
    }

    // 遍历所有可能的起始位置
    let mut start = 0;
    while start + word_len <= content_bytes.len() {
        // 查找下一个匹配位置
        if let Some(pos) = content[start..].find(word) {
            let match_start = start + pos;
            let match_end = match_start + word_len;

            // 检查前一个字符是否为词边界（非字母）
            let prefix_ok = match_start == 0 || !is_alpha_byte(content_bytes[match_start - 1]);

            // 检查后一个字符是否为词边界（非字母）
            let suffix_ok =
                match_end == content_bytes.len() || !is_alpha_byte(content_bytes[match_end]);

            if prefix_ok && suffix_ok {
                return true;
            }

            // 继续查找下一个匹配位置
            // 按完整字符推进，避免多字节词的下一次切片落在 UTF-8 字符内部
            let next_char_len = content[match_start..]
                .chars()
                .next()
                .map_or(word_len, char::len_utf8);
            start = match_start + next_char_len;
        } else {
            break;
        }
    }

    false
}

/// 判断字节是否为 ASCII 字母（用于词边界检测）
fn is_alpha_byte(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

/// v0.5.5 修复二：词边界感知的词频统计
///
/// 与 `contains_word` 配套，统计 word 在 content 中以整词形式出现的次数。
/// CJK token 使用 `matches().count()` 子串计数，英文单词使用词边界计数。
fn count_word_occurrences(content: &str, word: &str) -> usize {
    if word.is_empty() {
        return 0;
    }

    // 判断 word 是否为 CJK token
    let is_cjk_token = word.chars().any(|c| {
        let cp = c as u32;
        (0x4E00..=0x9FFF).contains(&cp)
            || (0x3400..=0x4DBF).contains(&cp)
            || (0xF900..=0xFAFF).contains(&cp)
    });

    // CJK token：子串计数
    if is_cjk_token {
        return content.matches(word).count();
    }

    // v0.5.5 修复：仅对长度 ≥ 3 的纯 ASCII 字母单词做词边界计数
    // 与 contains_word 的判断逻辑保持一致
    let char_count = word.chars().count();
    if char_count < 3 {
        return content.matches(word).count();
    }

    // 英文单词（长度 ≥ 3）：词边界计数
    let content_bytes = content.as_bytes();
    let word_bytes = word.as_bytes();
    let word_len = word_bytes.len();
    let mut count = 0;
    let mut start = 0;

    while start + word_len <= content_bytes.len() {
        if let Some(pos) = content[start..].find(word) {
            let match_start = start + pos;
            let match_end = match_start + word_len;

            let prefix_ok = match_start == 0 || !is_alpha_byte(content_bytes[match_start - 1]);
            let suffix_ok =
                match_end == content_bytes.len() || !is_alpha_byte(content_bytes[match_end]);

            if prefix_ok && suffix_ok {
                count += 1;
            }

            // 按完整字符推进，避免多字节词的下一次切片落在 UTF-8 字符内部
            let next_char_len = content[match_start..]
                .chars()
                .next()
                .map_or(word_len, char::len_utf8);
            start = match_start + next_char_len;
        } else {
            break;
        }
    }

    count
}

/// 隐私过滤辅助函数：判断记忆是否对指定隐私上下文可见
///
/// 规则（Section 3.3 隐私与权限）：
/// - Global 级别：对所有上下文可见
/// - User 级别：仅当 user_id 匹配时可见
/// - Session 级别：仅当 session_id 匹配时可见
/// - 无隐私上下文时：显示所有记忆（向后兼容）
fn is_visible(
    memory: &Memory,
    context: &Option<(PrivacyLevel, Option<String>, Option<String>)>,
) -> bool {
    match context {
        None => true, // 无隐私上下文，全部可见
        Some((_level, session_id, user_id)) => {
            match memory.privacy_level {
                PrivacyLevel::Global => true,
                PrivacyLevel::User => {
                    // User 级别：需要匹配 user_id
                    match (user_id, &memory.user_id) {
                        (Some(uid), Some(mid)) => uid == mid,
                        _ => false,
                    }
                }
                PrivacyLevel::Session => {
                    // Session 级别：需要匹配 session_id
                    match (session_id, &memory.session_id) {
                        (Some(sid), Some(mid)) => sid == mid,
                        _ => false,
                    }
                }
            }
        }
    }
}

impl<P: Persistence> MemoryStore<P> {
    /// 创建新的记忆存储器（默认相似度阈值 0.5）
    pub fn new(persistence: P) -> Self {
        let memory_state = persistence.load_memory_state().unwrap_or_default();
        let assoc_frequency = persistence.load_assoc_frequency().unwrap_or_default();
        // 质疑二·终极：启动时打印完整的隐私清单，而非一闪而过的日志
        eprintln!(
            "{}",
            crate::engine::user_feedback::UserFeedback::privacy_manifest()
        );

        Self {
            persistence,
            similarity_threshold: 0.5,
            synthesis_min_cluster: 3,
            synthesis_similarity: 0.4,
            luoshu_encoder: HybridLuoShuEncoder::default(),
            dao_metrics: DaoMetrics::new(),
            synthesis_journal: SynthesisJournal::new(),
            dao_regulator: DaoRegulator::new(),
            regulator_heartbeat: RegulatorHeartbeat::default(),
            synthesis_engine: SynthesisEngine::new(SynthesisConfig {
                min_cluster: 3,
                similarity: 0.4,
            }),
            decay_config: DecayConfig::default(),
            graph_store: None,
            memory_state_machine: MemoryStateMachine::from_state(memory_state),
            user_feedback: UserFeedback::new(),
            memory_gc: MemoryGarbageCollector::default(),
            gc_pending: std::sync::atomic::AtomicBool::new(false),
            synthesis_pending: std::sync::atomic::AtomicBool::new(false), // v0.5.4 初始无待合成任务
            audit_trail: AuditTrail::new(),
            hint_escalation: HintEscalationTracker::new(),
            // 质疑五·终极：初始化复杂度预算
            // 当前系统: ~20 个核心模块, ~200 个 pub fn, ~40 个跨模块依赖, 最深因果链 5 层
            complexity_budget: {
                let mut budget = ComplexityBudget::new();
                budget.update(20, 200, 40, 5);
                budget
            },
            // v0.5.4 增量缓存初始化（v0.9.7：整体委托 MemoryStoreCache）
            cache: crate::memory_store_cache::MemoryStoreCache::new(assoc_frequency),
            // v0.5.5 P1-1：LLM 默认未配置，由 sidecar 启动后通过 set_llm_configured() 设置
            llm_configured: std::sync::atomic::AtomicBool::new(false),
            // v0.6.0+ 参赛扩展：探索日志默认禁用
            exploration_logger: crate::engine::exploration_log::ExplorationLogger::disabled(),
        }
    }

    /// 创建使用统计编码器的 MemoryStore（跳过 ML 模型下载，适合基准测试）
    pub fn new_statistical(persistence: P) -> Self {
        Self {
            persistence,
            similarity_threshold: 0.5,
            synthesis_min_cluster: 3,
            synthesis_similarity: 0.4,
            #[cfg(feature = "ml")]
            luoshu_encoder: HybridLuoShuEncoder::new_statistical(),
            #[cfg(not(feature = "ml"))]
            luoshu_encoder: HybridLuoShuEncoder::new(),
            dao_metrics: DaoMetrics::new(),
            synthesis_journal: SynthesisJournal::new(),
            dao_regulator: DaoRegulator::new(),
            regulator_heartbeat: RegulatorHeartbeat::default(),
            synthesis_engine: SynthesisEngine::new(SynthesisConfig {
                min_cluster: 3,
                similarity: 0.4,
            }),
            decay_config: DecayConfig::default(),
            graph_store: None,
            memory_state_machine: MemoryStateMachine::new(),
            user_feedback: UserFeedback::new(),
            memory_gc: MemoryGarbageCollector::default(),
            gc_pending: std::sync::atomic::AtomicBool::new(false),
            synthesis_pending: std::sync::atomic::AtomicBool::new(false), // v0.5.4 初始无待合成任务
            audit_trail: AuditTrail::new(),
            hint_escalation: HintEscalationTracker::new(),
            complexity_budget: {
                let mut budget = ComplexityBudget::new();
                budget.update(20, 200, 40, 5);
                budget
            },
            // v0.5.4 增量缓存初始化（v0.9.7：整体委托 MemoryStoreCache）
            // P8.2o 状态化方案：统计模式构造点不加载磁盘统计，以空统计起步
            cache: crate::memory_store_cache::MemoryStoreCache::new(AssocFrequency::default()),
            // v0.5.5 P1-1：LLM 默认未配置
            llm_configured: std::sync::atomic::AtomicBool::new(false),
            // v0.6.0+ 参赛扩展：探索日志默认禁用
            exploration_logger: crate::engine::exploration_log::ExplorationLogger::disabled(),
        }
    }

    /// v0.9.0 新增：创建带指定编码器的 MemoryStore
    ///
    /// 用于 sidecar 启动时根据本地模型检测结果注入 ML 语义编码器。
    /// 复用 `new()` 的全部初始化逻辑，仅替换洛书编码器，
    /// 避免复制构造函数导致字段遗漏。
    pub fn new_with_encoder(persistence: P, luoshu_encoder: HybridLuoShuEncoder) -> Self {
        let mut store = Self::new(persistence);
        store.luoshu_encoder = luoshu_encoder;
        store
    }

    /// v0.9.0 新增：判断当前编码器是否为 ML 语义模式（而非统计降级模式）
    ///
    /// 统计模式下洛书 9 维向量对语义的区分度低，深度检索路径（trapezoid_focus_recall）
    /// 会把不相关记忆排到前面，稀释字面匹配结果。调用方据此决定是否跳过深度路径。
    pub fn is_ml_encoder(&self) -> bool {
        self.luoshu_encoder.get_status().mode.as_str() == "ml"
    }

    /// v0.9.7 联想探索·真实语义相似度（bge 完整句向量余弦，0-1）。
    ///
    /// 9 维洛书投影粒度太粗，承担不了"重要日子 ↔ 结婚纪念日"这类
    /// 语义强相关、词面零重叠的判断；本方法用底层 BERT 句向量直接
    /// 计算余弦。ML 编码器不可用（未加载/未启用 ml feature）时全部
    /// 返回 None，调用方退回纯词面通路——绝不放宽标准。
    ///
    /// 查询向量只编码一次，候选并行编码（见下）。调用方应惰性使用：
    /// 仅在词面门禁无人通过时调用，避免给常规检索热路径增加 ML 开销。
    #[cfg(feature = "ml")]
    pub fn semantic_similarities(&self, query: &str, memories: &[&Memory]) -> Vec<Option<f32>> {
        self.semantic_similarities_impl(query, memories, false, None)
    }

    /// v0.9.7 联想探索·去中心化语义相似度（P8.2j 实验通路，默认关）。
    ///
    /// 与 [`Self::semantic_similarities`] 的唯一差别：算余弦前，以**候选池自身的
    /// 均值向量**估计句向量的公共分量，并从查询/文档两侧同时减去（mean-centering）。
    /// 动机：bge-zh 句向量各向异性严重——全库向量挤在一个公共方向附近，使无关内容
    /// 的绝对余弦也被抬到 0.6+，任何绝对/相对标量阈值都难分离（P8.2h/P8.2i 已实测）。
    ///
    /// 用候选池均值而非全库均值，是因为全库均值需常驻全库向量（P8.2j 行动前侦查已证
    /// 仓库既有缓存为 2026-06-03 旧语料 208 维、不可复用，全库重编码叠加 10.8 的编码
    /// 吞吐风险）；而候选池向量本就要编码，属**零额外编码开销**，可满足旁路 6s 硬时限。
    ///
    /// 本方法仅在门控 `LRC_ASSOC_DEBIAS` 开启时被调用；关闭时生产路径仍走
    /// [`Self::semantic_similarities`]，与 P8.2h 逐字节一致（零影响承诺）。
    ///
    /// `suppress`（P8.2n）为可选的**池内中心度压制强度 λ**：`None` = 不压制，
    /// 与 P8.2m 逐字节一致；`Some(λ)` = 对显著超出池内中位中心度的候选按
    /// `s' = s − λ·max(0, κ − median(κ))` 扣减余弦（κ = 候选与同池其他候选的
    /// 平均去中心化余弦）。压制**以去中心化池内口径为界**——调用方须同时开启
    /// `LRC_ASSOC_DEBIAS`，否则本参数不生效（原始空间无中心度语义）。
    #[cfg(feature = "ml")]
    pub fn semantic_similarities_debiased(
        &self,
        query: &str,
        memories: &[&Memory],
        suppress: Option<f32>,
    ) -> Vec<Option<f32>> {
        self.semantic_similarities_impl(query, memories, true, suppress)
    }

    /// 语义相似度公共实现。`decenter=false` 时与历史实现逐字节一致；
    /// `decenter=true` 时追加"候选池均值去中心化"步骤（P8.2j）；
    /// `suppress=Some(λ)` 时在去中心化余弦上追加"池内中心度去偏"（P8.2n）。
    #[cfg(feature = "ml")]
    fn semantic_similarities_impl(
        &self,
        query: &str,
        memories: &[&Memory],
        decenter: bool,
        suppress: Option<f32>,
    ) -> Vec<Option<f32>> {
        fn l2_norm(v: &[f32]) -> f32 {
            v.iter().map(|x| x * x).sum::<f32>().sqrt()
        }
        fn dot(a: &[f32], b: &[f32]) -> f32 {
            a.iter().zip(b).map(|(x, y)| x * y).sum()
        }
        // bge-zh 官方用法：短查询侧需加检索指令前缀，文档侧不加。
        // 缺省前缀时句向量各向异性严重（实测相关对与无关对差距仅 ~0.01）。
        const BGE_QUERY_INSTRUCTION: &str = "为这个句子生成表示以用于检索相关文章：";
        let instructed_query = format!("{BGE_QUERY_INSTRUCTION}{query}");
        let Some(q) = self.luoshu_encoder.encode_embedding(&instructed_query) else {
            return vec![None; memories.len()];
        };
        let mut q = q;
        let q_norm_pre = l2_norm(&q);
        if !q_norm_pre.is_finite() || q_norm_pre <= 0.0 {
            return vec![None; memories.len()];
        }
        // v0.9.7 性能修复：候选并行编码。语义旁路只在词面零命中时触发，
        // 候选逐条串行前向（每条 ~2s）会在 root 召回阶段耗尽外层 15s
        // 超时，把"诚实空态"变成 503 错误。std::thread::scope 并发前向，
        // 墙钟时间压缩到单条耗时量级。
        // 注意：MemoryStore 因持久化缓存字段含 RefCell 而 !Sync，跨线程
        // 只共享编码器字段（HybridLuoShuEncoder 本身 Sync）。
        let encoder = &self.luoshu_encoder;
        let vectors: Vec<Option<Vec<f32>>> = std::thread::scope(|scope| {
            let handles: Vec<_> = memories
                .iter()
                .map(|m| {
                    let content = m.content.clone();
                    scope.spawn(move || encoder.encode_embedding(&content))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or(None))
                .collect()
        });
        if !decenter {
            // 历史路径（逐字节保留）：直接以原始句向量算余弦。
            return memories
                .iter()
                .zip(vectors)
                .map(|(_, v)| {
                    let v = v?;
                    let n = l2_norm(&v);
                    if !n.is_finite() || n <= 0.0 || v.len() != q.len() {
                        return None;
                    }
                    Some((dot(&q, &v) / (q_norm_pre * n)).clamp(0.0, 1.0))
                })
                .collect();
        }
        // P8.2j 去中心化：以候选池内"维度合法"的向量估计公共分量（均值向量），
        // 查询与文档两侧同时减去后再算余弦。池内向量本就要编码，零额外编码开销。
        let mut mean = vec![0.0f32; q.len()];
        let mut counted = 0usize;
        for v in vectors.iter().flatten() {
            if v.len() != q.len() {
                continue;
            }
            for (acc, x) in mean.iter_mut().zip(v.iter()) {
                *acc += *x;
            }
            counted += 1;
        }
        if counted == 0 {
            return vec![None; memories.len()];
        }
        let inv = 1.0f32 / counted as f32;
        for acc in mean.iter_mut() {
            *acc *= inv;
        }
        // 公共分量同时从查询侧与文档侧减去（对称去中心化），保持余弦可比性。
        for (x, m) in q.iter_mut().zip(mean.iter()) {
            *x -= *m;
        }
        let q_norm = l2_norm(&q);
        if !q_norm.is_finite() || q_norm <= 0.0 {
            return vec![None; memories.len()];
        }
        // 第一遍：算各候选的去中心化余弦。`suppress=None` 时下面的返回值与
        // P8.2m 逐字节一致（同一表达式、同一顺序）。
        let mut sims: Vec<Option<f32>> = Vec::with_capacity(memories.len());
        for v in &vectors {
            let Some(v) = v.as_ref() else {
                sims.push(None);
                continue;
            };
            if v.len() != q.len() {
                sims.push(None);
                continue;
            }
            let centered: Vec<f32> = v.iter().zip(mean.iter()).map(|(x, m)| *x - *m).collect();
            let n = l2_norm(&centered);
            if !n.is_finite() || n <= 0.0 {
                sims.push(None);
                continue;
            }
            sims.push(Some((dot(&q, &centered) / (q_norm * n)).clamp(0.0, 1.0)));
        }
        let Some(lambda) = suppress else {
            return sims;
        };
        if !lambda.is_finite() || lambda <= 0.0 {
            return sims;
        }
        // P8.2o 状态化压制（跨查询命中频次折扣）：以**独立统计文件**持久化的
        // 跨查询文档频率 df(content) 计算留一命中率
        //   l1_rate(c) = df(c) / max(total_queries, 1)
        // 作为"语义吸铁石"的真实度量，替代 P8.2n 的池内中心度**无状态代理**
        // （该代理在 §10.17 实测证伪：κ 仅随去中心化模长单调递减，霸榜者恰是
        // 模长最大者 ⇒ excess=0 ⇒ 永不被压，反杀平庸正确召回）。折扣形式
        //   s' = s − λ·l1_rate(c)，**不额外 clamp**
        // 与 §10.19 探针口径逐字一致（对 H2 不施加人为偏利）。
        //
        // 留一法保证：调用方（`run_association_explore`）必须在**完成本次压制判定
        // 之后**才调用 `record_assoc_query` 累计本查询的池，故当前查询不计入自身
        // 贡献——在线增量口径 (df+1)/(n+1) 与离线探针 (df_all−1)/(n_all−1) 恒等
        // （df_all = df+1、n_all = n+1），满足 §10.19.1 的防自证要求。
        //
        // 该量以"跨查询出现次数"为统计口径，是**跨查询原生可比的状态量**——正落在
        // §10.18 否证边界（查询内、无状态标量阈值族）之外（见 §10.19.4）。
        let freq = self.cache.borrow_assoc();
        for (memory, s) in memories.iter().zip(sims.iter_mut()) {
            let Some(s) = s else { continue };
            let rate = freq.leave_one_rate(&memory.content);
            if rate > 0.0 {
                *s -= lambda * rate;
            }
        }
        drop(freq);
        sims
    }

    /// 非 ml 构建：语义相似度恒不可用（返回全 None），词面通路独自生效。
    #[cfg(not(feature = "ml"))]
    pub fn semantic_similarities(&self, _query: &str, memories: &[&Memory]) -> Vec<Option<f32>> {
        vec![None; memories.len()]
    }

    /// 非 ml 构建：去中心化语义相似度同样恒不可用（门控开启也不改变行为）。
    /// P8.2n/P8.2o 压制参数在非 ml 下无编码器可依，同样静默忽略。
    #[cfg(not(feature = "ml"))]
    pub fn semantic_similarities_debiased(
        &self,
        _query: &str,
        memories: &[&Memory],
        _suppress: Option<f32>,
    ) -> Vec<Option<f32>> {
        vec![None; memories.len()]
    }

    /// P8.2o 状态化方案：累计一次查询的根候选池命中频次，并持久化统计快照。
    ///
    /// **留一法调用约束（强）**：调用方必须在**完成本次压制判定之后**才调用本方法。
    /// 判定读取的是"本次查询尚未计入"的历史统计，故当前查询不计入自身贡献；
    /// 在线增量口径 `df(c)/max(total,1)` 与离线探针
    /// `(df_all − 1_{c∈q池})/(n_all − 1)` 恒等（df_all = df+1、n_all = n+1），
    /// 满足防自证要求（口径推导见 [`Self::semantic_similarities_impl`] 内注释）。
    ///
    /// 持久化失败不阻塞检索/探索——跨查询统计是增强项，与 `bake_activation`
    /// 对 `save_memory_state` 的容错同一范式（`let _ =`）。
    pub fn record_assoc_query(&mut self, pool_contents: &[String]) {
        if pool_contents.is_empty() {
            // 与离线探针口径一致：空池不计数（探针见空池直接 continue）
            return;
        }
        {
            let mut freq = self.cache.borrow_assoc_mut();
            freq.record_query(pool_contents.iter().map(|s| s.as_str()));
        }
        let snapshot = self.cache.assoc_snapshot();
        let _ = self.persistence.save_assoc_frequency(&snapshot);
    }

    /// v0.6.0+ 参赛扩展：设置探索日志记录器
    /// 由 sidecar 启动时根据 `--exploration-log <path>` 参数注入
    /// 未调用此方法时，所有日志调用为空操作（disabled 模式）
    pub fn set_exploration_logger(
        &mut self,
        logger: crate::engine::exploration_log::ExplorationLogger,
    ) {
        self.exploration_logger = logger;
    }

    /// v0.5.5 P1-1：设置 LLM 配置状态
    /// LLM 配置后替代本地 ML 模型提供语义理解能力，编码器不再视为"降级"
    /// 由 sidecar 启动后根据 LLM 配置情况调用
    pub fn set_llm_configured(&self, configured: bool) {
        self.llm_configured
            .store(configured, std::sync::atomic::Ordering::Relaxed);
    }

    /// v0.5.5 P1-1：获取 LLM 配置状态
    pub fn is_llm_configured(&self) -> bool {
        self.llm_configured
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 检查编码器是否处于降级状态（条件编译：仅 ml feature 时检查）
    #[cfg(feature = "ml")]
    fn check_encoder_degraded(encoder: &HybridLuoShuEncoder) -> bool {
        encoder.is_degraded()
    }

    #[cfg(not(feature = "ml"))]
    fn check_encoder_degraded(_encoder: &HybridLuoShuEncoder) -> bool {
        false // 纯统计编码器永不降级
    }

    /// 获取编码器恢复进度（条件编译：仅 ml feature 时获取）
    #[cfg(feature = "ml")]
    fn get_encoder_recovery_progress(encoder: &HybridLuoShuEncoder) -> (u32, u32) {
        encoder.recovery_progress()
    }

    #[cfg(not(feature = "ml"))]
    fn get_encoder_recovery_progress(_encoder: &HybridLuoShuEncoder) -> (u32, u32) {
        (0, 0) // 纯统计编码器无恢复进度
    }

    // ============================================================
    // v0.5.4 增量缓存辅助方法
    // ============================================================

    /// 确保缓存有效并返回记忆列表的克隆
    ///
    /// v0.9.7（GLOBAL_CODE_REVIEW_REPORT P2-2）：缓存的存储与惰性加载已外提至
    /// [`crate::memory_store_cache::MemoryStoreCache`]，本方法仅保留"脏则先从持久层
    /// 加载，再取副本"的编排与性能剖析埋点，行为与拆分前逐字节一致。
    fn load_cached(&self) -> Result<Vec<Memory>, PersistenceError> {
        let load_start = std::time::Instant::now();
        let cache_was_dirty = self.cache.is_dirty();
        if cache_was_dirty {
            let loaded = self.persistence.load_all_memories()?;
            self.cache.store(loaded);
        }
        let result = self.cache.snapshot();
        if std::env::var_os("LRC_PROFILE_REMEMBER").is_some() {
            eprintln!(
                "[LRC_PROFILING] load_cached_ms={:.3} cache_reload={} memory_count={}",
                load_start.elapsed().as_secs_f64() * 1000.0,
                cache_was_dirty,
                result.len()
            );
        }
        Ok(result)
    }

    /// 标记缓存为脏：任何写操作（保存/删除/修改）后调用
    /// 下次 load_cached 时会重新从持久层加载
    fn invalidate_cache(&self) {
        self.cache.invalidate();
    }

    /// v0.9.7 审查修复：外部替换数据文件（如备份恢复）后强制失效内存缓存，
    /// 否则后续读取仍返回恢复前的旧缓存数据
    pub fn invalidate_cache_after_external_restore(&self) {
        self.cache.invalidate();
    }

    fn mark_cache_dirty_preserving_index(&self) {
        self.cache.mark_dirty_preserving_index();
    }

    fn recall_document(&self, memory: &Memory) -> crate::memory_store_cache::RecallDocument {
        self.cache.recall_document(memory)
    }

    /// 统计一条记忆与查询的词面实质重叠 token 数（v0.9.7 精确度门禁信号源）。
    ///
    /// 复用 recall 内部的规范化文档缓存与词边界匹配逻辑，保证与评分口径
    /// 完全一致。调用方（联想探索 API）用它实现"根节点必须与查询实质
    /// 共鸣"的门禁：仅凭单个泛指词（如"什么"）重叠的无关记忆不得上位。
    ///
    /// # 参数
    /// - `memory`：待统计的候选记忆
    /// - `query_tokens`：调用方预先用 [`tokenize_query`] 分好的查询 token
    ///   （避免对同一查询的 N 条候选重复分词）
    pub fn query_overlap_count(&self, memory: &Memory, query_tokens: &[String]) -> usize {
        let content_lower = self.recall_document(memory).normalized_content;
        query_tokens
            .iter()
            .filter(|token| contains_word(&content_lower, token))
            .count()
    }

    /// 设置冲突检测的相似度阈值
    ///
    /// 范围 0.0 ~ 1.0，值越高表示要求越严格（越相似才会合并）。
    pub fn with_similarity_threshold(mut self, threshold: f32) -> Self {
        self.similarity_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// 设置隐式反馈开关（质疑二·隐私）
    ///
    /// 启用时，系统通过用户行为（点击、复制、停留、重复查询等）推断相关性。
    /// 禁用时，仅依赖用户的显式反馈指令。
    /// 数据仅留在本地，不会上传到任何外部服务器。
    pub fn set_implicit_feedback_enabled(&self, enabled: bool) {
        self.user_feedback.set_implicit_feedback_enabled(enabled);
    }

    /// 检查隐式反馈是否启用（质疑二·隐私）
    pub fn is_implicit_feedback_enabled(&self) -> bool {
        self.user_feedback.is_implicit_feedback_enabled()
    }

    /// 设置图存储（用于自动建立冲突/演进/合成关系边）
    ///
    /// 启用后，写入冲突时会自动创建 Contradicts/Evolves 边，
    /// 递归合成时会自动创建 SynthesizesFrom 边。
    pub fn with_graph_store(mut self, graph_store: GraphMemoryStore) -> Self {
        self.graph_store = Some(graph_store);
        self
    }

    /// 计算 Jaccard 词集相似度
    ///
    /// 对于中文文本（无空格分词），使用字符级 bigram 比较。
    /// 对于英文文本，使用空格分词比较。
    fn compute_jaccard(&self, a: &str, b: &str) -> f32 {
        self.synthesis_engine.compute_jaccard(a, b)
    }

    fn content_bigrams(content: &str) -> std::collections::HashSet<String> {
        crate::memory_store_cache::MemoryStoreCache::content_bigrams(content)
    }

    /// 域候选索引的 term 提取：与评分侧 `tokenize_query` 保持一致。
    ///
    /// v8 检索质量修复：原先"含任一 CJK 即整体 bigram 切分"会把混合文本中的
    /// 英文标识符切碎（try_read → 2 字符碎片），导致索引 term 与评分 token
    /// 分叉、候选剪枝漏召回正确记忆。现直接复用 `tokenize_query`（混合语言下
    /// 保留英文/数字整词 + 中文 bigram），保证候选索引与 TF-IDF 评分口径统一。
    fn content_index_terms(content: &str) -> std::collections::HashSet<String> {
        crate::memory_store_cache::MemoryStoreCache::content_index_terms(content)
    }

    /// 域候选索引开关：默认启用 NOLANG（项目优先、无语言剪枝），
    /// 对应决策记忆 914a95dd（§5.11 方向 a 线上 A/B 验收通过后默认启用）。
    fn domain_index_enabled() -> bool {
        crate::memory_store_cache::MemoryStoreCache::domain_index_enabled()
    }

    /// NOLANG 子模式：默认启用；仅当显式设置 LRC_DOMAIN_CANDIDATE_INDEX
    /// 而未设置 LRC_DOMAIN_CANDIDATE_INDEX_NOLANG 时，退回带语言剪枝的
    /// 普通 domain 模式（历史兼容语义）。
    fn domain_index_nolang_enabled() -> bool {
        crate::memory_store_cache::MemoryStoreCache::domain_index_nolang_enabled()
    }

    fn add_memory_to_index(&self, memory: &Memory) {
        self.cache.add_to_index(memory);
    }

    fn remove_memory_from_index(&self, memory: &Memory) {
        self.cache.remove_from_index(memory);
    }

    fn replace_memory_in_index(&self, old: &Memory, new: &Memory) {
        self.cache.replace_in_index(old, new);
    }

    fn verify_candidate_equivalence(
        &self,
        all: &[Memory],
        candidate_positions: &[usize],
        content: &str,
    ) -> (usize, usize) {
        let exact_matches = all
            .iter()
            .filter(|memory| {
                self.compute_jaccard(content, &memory.content) >= self.similarity_threshold
            })
            .map(|memory| memory.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let candidate_ids = candidate_positions
            .iter()
            .map(|position| all[*position].id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let indexed_exact_matches = candidate_positions
            .iter()
            .filter(|position| {
                self.compute_jaccard(content, &all[**position].content) >= self.similarity_threshold
            })
            .map(|position| all[*position].id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let missing = exact_matches.difference(&indexed_exact_matches).count();
        let extra = candidate_ids.difference(&exact_matches).count();
        (missing, extra)
    }

    fn rebuild_bigram_index(&self, all: &[Memory]) {
        self.cache.rebuild_index(all);
    }

    /// 查找与给定内容高度相似的已有记忆。
    pub fn find_similar(&self, content: &str) -> Result<Option<Memory>, PersistenceError> {
        self.find_similar_scoped_with_privacy(content, None, &None)
    }

    /// 按项目和语言域优先查找相似记忆，未命中时受控扩大候选范围。
    /// 仅测试使用（生产路径经 [Self::find_similar_scoped_with_privacy]）。
    #[cfg(test)]
    fn find_similar_scoped(
        &self,
        content: &str,
        scope: Option<&Memory>,
    ) -> Result<Option<Memory>, PersistenceError> {
        self.find_similar_scoped_with_privacy(content, scope, &None)
    }

    /// 按项目和语言域优先查找相似记忆（带隐私可见性过滤）。
    ///
    /// 2026-09-01 隐私隔离修复：评估循环中对每个候选调用 `is_visible`
    /// 过滤——只有对调用方隐私上下文可见的记忆才参与相似合并，避免
    /// `remember` 把其他 session/user 的私有记忆误合并进当前上下文。
    fn find_similar_scoped_with_privacy(
        &self,
        content: &str,
        scope: Option<&Memory>,
        privacy_context: &Option<(PrivacyLevel, Option<String>, Option<String>)>,
    ) -> Result<Option<Memory>, PersistenceError> {
        let all = self.load_cached()?;
        let candidate_start = std::time::Instant::now();
        let use_index = std::env::var_os("LRC_BIGRAM_CANDIDATE_INDEX").is_some();
        let use_rare_index = std::env::var_os("LRC_RARE_BIGRAM_CANDIDATE_INDEX").is_some();
        let use_domain_index = Self::domain_index_enabled();
        let use_domain_index_nolang = Self::domain_index_nolang_enabled();
        let index_mode = if use_domain_index && use_domain_index_nolang {
            "domain-nolang"
        } else if use_domain_index {
            "domain"
        } else if use_rare_index {
            "rare"
        } else if use_index {
            "all"
        } else {
            "off"
        };
        let mut primary_candidates = Vec::new();
        let mut same_language_candidates = Vec::new();
        let fallback_candidates: Vec<usize> = (0..all.len()).collect();
        let mut candidate_positions = Vec::new();
        let mut indexed_position_count = 0usize;
        let mut fallback_triggered = false;
        let mut same_project_count = 0usize;

        let content_is_cjk = content.chars().any(|c| {
            let code = c as u32;
            (0x4E00..=0x9FFF).contains(&code)
        });
        let index_enabled = use_index || use_rare_index || (use_domain_index && scope.is_some());
        let query_terms = if use_domain_index {
            Self::content_index_terms(content)
        } else {
            Self::content_bigrams(content)
        };
        if index_enabled && (!query_terms.is_empty()) && (content_is_cjk || use_domain_index) {
            if self.cache.index_is_dirty() {
                self.rebuild_bigram_index(&all);
            }
            let index = self.cache.borrow_index();
            let selected_bigrams = if use_rare_index && content_is_cjk {
                let mut ranked = query_terms
                    .iter()
                    .map(|bigram| {
                        (
                            bigram,
                            index.get(bigram).map_or(0, std::collections::HashSet::len),
                        )
                    })
                    .collect::<Vec<_>>();
                ranked.sort_unstable_by_key(|(_, frequency)| *frequency);
                ranked
                    .into_iter()
                    .take(4)
                    .map(|(bigram, _)| bigram.as_str())
                    .collect::<Vec<_>>()
            } else {
                query_terms.iter().map(String::as_str).collect()
            };
            let mut candidate_ids = std::collections::HashSet::new();
            for bigram in selected_bigrams {
                if let Some(found) = index.get(bigram) {
                    candidate_ids.extend(found.iter().cloned());
                }
            }
            let positions_by_id = all
                .iter()
                .enumerate()
                .map(|(position, memory)| (memory.id.as_str(), position))
                .collect::<std::collections::HashMap<_, _>>();
            let positions = candidate_ids
                .iter()
                .filter_map(|id| positions_by_id.get(id.as_str()).copied())
                .collect::<std::collections::HashSet<_>>();
            indexed_position_count = positions.len();
            let scope_is_cjk = scope
                .map(|memory| {
                    memory.content.chars().any(|c| {
                        let code = c as u32;
                        (0x4E00..=0x9FFF).contains(&code)
                    })
                })
                .unwrap_or(content_is_cjk);
            let scope_project = scope.and_then(|memory| memory.project.as_deref());
            let same_project = |memory: &Memory| memory.project.as_deref() == scope_project;
            for position in positions {
                let memory = &all[position];
                if use_domain_index && same_project(memory) {
                    // 项目优先桶：同项目即入（NOLANG 模式不按语言剪枝，
                    // 普通 domain 模式仍要求语言一致，见下方同语言并入条件）。
                    primary_candidates.push(position);
                } else if !use_domain_index || use_domain_index_nolang || {
                    let memory_is_cjk = memory.content.chars().any(|c| {
                        let code = c as u32;
                        (0x4E00..=0x9FFF).contains(&code)
                    });
                    memory_is_cjk == scope_is_cjk
                } {
                    same_language_candidates.push(position);
                }
            }
            primary_candidates.sort_unstable();
            same_language_candidates.sort_unstable();
            same_project_count = primary_candidates.len();
            candidate_positions.extend(&primary_candidates);
            candidate_positions.extend(&same_language_candidates);
            candidate_positions.sort_unstable();
            candidate_positions.dedup();
        }
        if !index_enabled {
            candidate_positions = fallback_candidates.clone();
        } else if candidate_positions.is_empty() {
            fallback_triggered = true;
            candidate_positions = fallback_candidates.clone();
        }

        if std::env::var_os("LRC_CANDIDATE_INDEX_VERIFY").is_some() && index_enabled {
            let (missing, extra) =
                self.verify_candidate_equivalence(&all, &candidate_positions, content);
            eprintln!(
                "[LRC_VERIFY] index_mode={} candidates={} missing_exact_matches={} extra_candidates={}",
                index_mode,
                candidate_positions.len(),
                missing,
                extra
            );
        }

        let candidate_count = candidate_positions.len();
        let candidate_build_ms = candidate_start.elapsed().as_secs_f64() * 1000.0;
        let primary_candidate_count = primary_candidates.len();
        let same_language_count = same_language_candidates.len();
        let fallback_candidate_count = if fallback_triggered {
            fallback_candidates.len()
        } else {
            0
        };
        let mut comparisons = 0usize;
        let similarity_start = std::time::Instant::now();
        for position in &candidate_positions {
            let m = &all[*position];
            if m.is_expired() {
                continue;
            }
            // 隐私隔离：跳过对调用方不可见的记忆（不参与相似合并）
            if !is_visible(m, privacy_context) {
                continue;
            }
            comparisons += 1;
            let sim = self.compute_jaccard(content, &m.content);
            if sim >= self.similarity_threshold {
                if std::env::var_os("LRC_PROFILE_REMEMBER").is_some() {
                    emit_remember_profile(format!(
                        "[LRC_PROFILING] remember candidates={} indexed_positions={} primary_candidates={} same_language_candidates={} fallback_candidates={} fallback_triggered={} fallback_comparisons={} same_project={} jaccard_comparisons={} index_mode={} query_is_cjk={} scope_project={:?} candidate_build_ms={:.3} similarity_ms={:.3} hit=true",
                        candidate_count,
                        indexed_position_count,
                        primary_candidate_count,
                        same_language_count,
                        fallback_candidate_count,
                        fallback_triggered,
                        if fallback_triggered { comparisons } else { 0 },
                        same_project_count,
                        comparisons,
                        index_mode,
                        content_is_cjk,
                        scope.and_then(|memory| memory.project.as_deref()),
                        candidate_build_ms,
                        similarity_start.elapsed().as_secs_f64() * 1000.0
                    ));
                }
                return Ok(Some(m.clone()));
            }
        }

        if std::env::var_os("LRC_PROFILE_REMEMBER").is_some() {
            emit_remember_profile(format!(
                "[LRC_PROFILING] remember candidates={} indexed_positions={} primary_candidates={} same_language_candidates={} fallback_candidates={} fallback_triggered={} fallback_comparisons={} same_project={} jaccard_comparisons={} index_mode={} query_is_cjk={} scope_project={:?} candidate_build_ms={:.3} similarity_ms={:.3}",
                candidate_count,
                indexed_position_count,
                primary_candidate_count,
                same_language_count,
                fallback_candidate_count,
                fallback_triggered,
                if fallback_triggered { comparisons } else { 0 },
                same_project_count,
                comparisons,
                index_mode,
                content_is_cjk,
                scope.and_then(|memory| memory.project.as_deref()),
                candidate_build_ms,
                similarity_start.elapsed().as_secs_f64() * 1000.0
            ));
        }
        Ok(None)
    }

    /// 尝试执行递归合成（在写入新记忆后调用）
    ///
    /// 扫描记忆库，找到所有满足条件的记忆簇，为每个簇生成合成记忆。
    /// 如果簇中已有合成记忆（通过 source_ids 判断），则跳过该簇。
    ///
    /// 返回本次新生成的合成记忆数量。
    pub fn try_synthesize(&mut self) -> Result<usize, PersistenceError> {
        // v0.6.0+ 参赛扩展：探索日志埋点（synthesize 事件）
        let synthesize_start = std::time::Instant::now();

        let result = self.synthesis_engine.try_synthesize(
            &self.persistence,
            &mut self.graph_store,
            &mut self.dao_metrics,
        );

        // v0.6.0+ 参赛扩展：探索日志记录（synthesize 事件）
        if let Ok(synthesized_count) = &result {
            self.exploration_logger.log(
                crate::engine::exploration_log::ExplorationEventType::Synthesize,
                serde_json::json!({
                    "engine": "jaccard",
                    "synthesized_count": synthesized_count,
                }),
                Some(crate::engine::exploration_log::Metrics {
                    latency_ms: Some(synthesize_start.elapsed().as_millis() as u64),
                    result_count: Some(*synthesized_count),
                    ..Default::default()
                }),
            );
        }

        result
    }

    /// 合成快照（三阶段锁解耦·Phase 1）：持锁下快速读取全量记忆 + 配置
    ///
    /// 仅做磁盘读取，不执行 CPU 密集的聚类计算，锁持有时间极短。
    pub fn synthesis_snapshot(&self) -> Result<SynthesisSnapshot, PersistenceError> {
        let all = self.persistence.load_all_memories()?;
        Ok(SynthesisSnapshot {
            all,
            config: SynthesisConfig {
                min_cluster: self.synthesis_min_cluster,
                similarity: self.synthesis_similarity,
            },
            information_gain_threshold: self.dao_regulator.information_gain_threshold,
        })
    }

    /// 应用合成计划（三阶段锁解耦·Phase 3）：持锁下批量写回
    ///
    /// 将 Phase 2 无锁计算产出的 SynthesisPlan 写回：磁盘 + 图 + 日志 + 指标 + 审计。
    /// 返回实际写入的合成记忆数量。
    pub fn apply_synthesis_plan(&mut self, plan: SynthesisPlan) -> usize {
        let synthesized = plan.synthesized;
        if plan.batch.is_empty() {
            return 0;
        }

        // 批量写入磁盘（单次序列化）
        if let Err(e) = self.persistence.save_memories(&plan.batch) {
            eprintln!("[LRC·合成] 批量写入合成记忆失败: {}", e);
            return 0;
        }

        // 图边
        for (synthesis_id, source_ids, confidence) in &plan.graph_edges {
            if let Some(ref mut graph) = self.graph_store {
                for sid in source_ids {
                    let _ =
                        graph.add_edge(synthesis_id, sid, EdgeType::SynthesizesFrom, *confidence);
                }
            }
        }

        // 合成日志
        for entry in &plan.journal_entries {
            self.synthesis_journal.record_synthesis(
                entry.synthesis_id.clone(),
                &entry.trigger_source,
                &entry.bagua_category,
                entry.bagua_index,
                entry.source_ids.clone(),
                entry.confidence,
                entry.member_count,
            );
        }

        // 指标
        for _ in 0..synthesized {
            self.dao_metrics.record_composition();
        }

        // 审计（系统自主行为，需可回溯）
        self.record_audit(
            AuditEventType::SynthesisCreated,
            format!("合成创建 {} 条合成记忆", synthesized),
            "三阶段锁解耦：聚类计算在锁外执行，仅写回阶段持锁",
            plan.batch.iter().map(|m| m.id.clone()).collect(),
        );

        // v0.5.4 写操作后标记缓存为脏
        self.invalidate_cache();
        // v0.9.1 三阶段锁解耦：合成写回完成后清除待合成标记，
        // 由后台结晶流水线的三阶段合成（而非健康检查）负责消费此标记。
        self.synthesis_pending
            .store(false, std::sync::atomic::Ordering::Release);
        synthesized
    }

    /// 洛书驱动递归合成（M.T.R. RecursiveCompose 增强版）
    ///
    /// 与 Jaccard-based try_synthesize 不同，此方法使用洛书向量进行:
    /// 1. MirrorProject 分类 → 按八卦类别分组
    /// 2. RecursiveCompose 门控融合 → 每个类别内合成
    /// 3. 生成高置信度的 Synthesis 记忆
    ///
    /// 返回新生成的合成记忆数量。
    ///
    /// 三阶段锁解耦（v0.9.1）：此兼容接口内部已拆分为 snapshot → plan → apply，
    /// 真正的锁外计算由 consolidation / v1_api 的三阶段调用实现。
    pub fn luoshu_synthesize(&mut self) -> Result<usize, PersistenceError> {
        let synthesize_start = std::time::Instant::now();

        // 同步引擎配置（确保 consolidation 等外部调用者设置的阈值生效）
        self.synthesis_engine = SynthesisEngine::new(SynthesisConfig {
            min_cluster: self.synthesis_min_cluster,
            similarity: self.synthesis_similarity,
        });

        // Phase 1：读快照
        let snapshot = self.synthesis_snapshot()?;

        // Phase 2：纯计算（洛书优先，失败降级 Jaccard）
        let engine = SynthesisEngine::new(snapshot.config);
        let mut plan = engine.plan_luoshu(&snapshot.all, snapshot.information_gain_threshold);
        let engine_kind = if plan.synthesized > 0 {
            "luoshu"
        } else {
            plan = engine.plan_jaccard(&snapshot.all);
            "jaccard_fallback"
        };

        // Phase 3：写回
        let result = self.apply_synthesis_plan(plan);

        // v0.6.0+ 参赛扩展：探索日志记录
        self.exploration_logger.log(
            crate::engine::exploration_log::ExplorationEventType::Synthesize,
            serde_json::json!({
                "engine": engine_kind,
                "synthesized_count": result,
            }),
            Some(crate::engine::exploration_log::Metrics {
                latency_ms: Some(synthesize_start.elapsed().as_millis() as u64),
                result_count: Some(result),
                ..Default::default()
            }),
        );

        Ok(result)
    }

    /// v0.5.4 运行待处理的合成任务（从关键路径移出，由后台调用）
    ///
    /// 检查 `synthesis_pending` 标记，如果为 true 则执行合成并清除标记。
    /// 此方法设计为从健康检查、定时任务或后台线程中调用，
    /// 避免合成操作阻塞用户的记忆写入/检索请求。
    ///
    /// v0.8.48 P0 修复：使用 AtomicBool + compare_exchange 确保
    /// 多个后台任务并发调用时，合成恰好执行一次（Leader Election 模式）。
    ///
    /// 返回合成的记忆数量，无待合成任务时返回 0。
    pub fn run_pending_synthesis(&mut self) -> Result<usize, PersistenceError> {
        // 原子 CAS：如果当前值为 true，设为 false 并返回 Ok(true) 表示"本线程获取执行权"
        // 如果当前值已为 false，返回 Err(...) 表示"已有其他线程在执行或无需执行"
        use std::sync::atomic::Ordering;
        if self
            .synthesis_pending
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            // 其他线程已获取执行权，或当前无待合成任务
            return Ok(0);
        }
        self.luoshu_synthesize()
    }

    /// 道枢映射: 道枢·中枢 — 道枢调节的对外接口，连接哲学根基与工程实践
    /// 道同构度自适应调节（感知→行动闭环）
    ///
    /// 基于 DaoMetrics + SynthesisJournal 的数据，
    /// 自动检测系统健康状态并生成调节动作。
    /// 返回调节动作的描述，供上层决策使用。
    pub fn regulate(&mut self) -> Option<RegulationAction> {
        if !self.dao_regulator.should_regulate() {
            return None;
        }

        // 采集当前系统状态
        let all = match self.load_cached() {
            Ok(memories) => memories,
            Err(_) => return None,
        };

        let total = all.iter().filter(|m| !m.is_expired()).count();
        let crystallized = all
            .iter()
            .filter(|m| m.memory_type == MemoryType::Synthesis)
            .count();
        // 归档记忆 = 已过期但未删除的记忆
        let archived = all.iter().filter(|m| m.is_expired()).count();

        // 计算八卦分布
        let mut bagua_counts = [0usize; 8];
        for m in &all {
            if let Some(idx) = m.bagua_index {
                if (idx as usize) < 8 {
                    bagua_counts[idx as usize] += 1;
                }
            }
        }

        // 计算平均洛书偏离度
        let vectors: Vec<[f32; 9]> = all.iter().filter_map(|m| m.luoshu_vector).collect();
        let avg_deviation = crate::engine::dao_metrics::compute_avg_luoshu_deviation(&vectors);

        // 采集道同构度快照
        let snapshot =
            self.dao_metrics
                .snapshot(total, crystallized, archived, avg_deviation, &bagua_counts);
        let journal_snapshot = self.synthesis_journal.snapshot();

        // v0.9.1 深度接线：记录调节前的灾难事件数与冻结状态，用于事后对比审计
        let catastrophic_before = self.dao_regulator.get_catastrophic_events().len();
        let was_frozen = self.dao_regulator.is_frozen();

        // v0.6.0+ 参赛扩展：探索日志埋点（regulate 事件）
        let regulate_start = std::time::Instant::now();

        let action = self.dao_regulator.regulate(
            snapshot.dao_isomorphism_score,
            snapshot.bagua_entropy,
            snapshot.synthesis_ratio,
            avg_deviation,
            journal_snapshot.synthesis_rate_per_minute,
            self.decay_config.decay_rate,
            self.synthesis_min_cluster,
        );

        // v0.6.0+ 参赛扩展：探索日志记录（regulate 事件）
        let action_str = match &action {
            RegulationAction::NoAction => "no_action",
            RegulationAction::AdjustDecayRate { .. } => "adjust_decay_rate",
            RegulationAction::AdjustSynthesisThreshold { .. } => "adjust_synthesis_threshold",
            RegulationAction::SuggestReencoding { .. } => "suggest_reencoding",
            RegulationAction::AdjustRetrievalWeights { .. } => "adjust_retrieval_weights",
            RegulationAction::AdjustInformationGainThreshold { .. } => {
                "adjust_information_gain_threshold"
            }
            RegulationAction::SuggestComprehensiveRebalance { .. } => {
                "suggest_comprehensive_rebalance"
            }
        };
        self.exploration_logger.log_regulate(
            snapshot.dao_isomorphism_score,
            snapshot.bagua_entropy,
            snapshot.synthesis_ratio,
            avg_deviation,
            action_str,
            regulate_start.elapsed().as_millis() as u64,
        );

        // 执行调节动作（并记录审计：系统自主调节需可回溯）
        match &action {
            RegulationAction::AdjustDecayRate { new_rate, reason } => {
                let old_rate = self.decay_config.decay_rate;
                self.decay_config.decay_rate = *new_rate;
                eprintln!(
                    "[LRC·调节] 衰减速率已调整: {:.2} → {:.2}（{}）",
                    old_rate, new_rate, reason
                );
                self.record_audit(
                    AuditEventType::DecayRateChanged,
                    format!("衰减速率 {:.2} → {:.2}", old_rate, new_rate),
                    reason.clone(),
                    Vec::new(),
                );
            }
            RegulationAction::AdjustSynthesisThreshold {
                new_min_cluster,
                reason,
                ..
            } => {
                let old_cluster = self.synthesis_min_cluster;
                self.synthesis_min_cluster = *new_min_cluster;
                // 同步更新合成引擎配置
                self.synthesis_engine = SynthesisEngine::new(SynthesisConfig {
                    min_cluster: *new_min_cluster,
                    similarity: self.synthesis_similarity,
                });
                eprintln!(
                    "[LRC·调节] 合成最小聚类已调整: {} → {}（{}）",
                    old_cluster, new_min_cluster, reason
                );
                self.record_audit(
                    AuditEventType::SynthesisThresholdChanged,
                    format!("合成最小聚类 {} → {}", old_cluster, new_min_cluster),
                    reason.clone(),
                    Vec::new(),
                );
            }
            RegulationAction::SuggestReencoding { reason, .. } => {
                eprintln!("[LRC·调节] 建议重新编码: {}", reason);
                self.record_audit(
                    AuditEventType::ReencodingSuggested,
                    "建议重新编码以恢复洛书几何约束",
                    reason.clone(),
                    Vec::new(),
                );
            }
            RegulationAction::AdjustRetrievalWeights { reason, .. } => {
                eprintln!("[LRC·调节] 建议调整检索权重: {}", reason);
                self.record_audit(
                    AuditEventType::RetrievalWeightsAdjusted,
                    "调整检索权重以平衡八卦分布",
                    reason.clone(),
                    Vec::new(),
                );
            }
            RegulationAction::NoAction => {}
            RegulationAction::AdjustInformationGainThreshold {
                new_threshold,
                reason,
            } => {
                let old = self.dao_regulator.information_gain_threshold;
                self.dao_regulator.information_gain_threshold = *new_threshold;
                eprintln!(
                    "[LRC·调节] 信息增量阈值已调整: {:.4} → {:.4}，原因: {}",
                    old, new_threshold, reason
                );
                self.record_audit(
                    AuditEventType::RegulationApplied,
                    format!("信息增量阈值 {:.4} → {:.4}", old, new_threshold),
                    reason.clone(),
                    Vec::new(),
                );
            }
            RegulationAction::SuggestComprehensiveRebalance {
                anomaly_description,
                coupling_score,
                ..
            } => {
                eprintln!(
                    "[LRC·调节] 综合再平衡建议（耦合指数 {:.2}）: {}",
                    coupling_score, anomaly_description
                );
                self.record_audit(
                    AuditEventType::ComprehensiveRebalance,
                    format!("综合再平衡建议（耦合指数 {:.2}）", coupling_score),
                    anomaly_description.clone(),
                    Vec::new(),
                );
            }
        }

        // 灾难性/慢性恶化事件审计（系统自主守护行为的可回溯记录）
        let catastrophic_events = self.dao_regulator.get_catastrophic_events();
        for event in catastrophic_events.iter().skip(catastrophic_before) {
            let is_chronic = event.last_action_before_crash == "chronic_degradation";
            let event_type = if is_chronic {
                AuditEventType::ChronicDegradation
            } else {
                AuditEventType::CatastrophicEvent
            };
            self.record_audit(
                event_type,
                format!("[{}] {}", event.severity, event.diagnosis),
                format!(
                    "健康评分 {:.2} → {:.2}（下降 {:.2}）",
                    event.health_before, event.health_after, event.drop_magnitude
                ),
                Vec::new(),
            );
        }

        // 调节器冻结审计（连续无效调节触发冻结保护）
        if !was_frozen && self.dao_regulator.is_frozen() {
            self.record_audit(
                AuditEventType::RegulatorFrozen,
                "调节器已冻结（连续无效调节达到阈值）",
                "防止振荡/漂移进一步恶化，暂停自动调节等待人工介入",
                Vec::new(),
            );
        }

        // 垃圾回收：在每次调节时顺带清理低质量合成记忆
        // 使用隔离 + 渐进式淘汰替代直接删除，防止污染扩散
        if let Ok(quarantined) = self.clean_low_quality_synthesis() {
            if quarantined > 0 {
                eprintln!(
                    "[LRC·调节] 调节过程中隔离了 {} 条低质量合成记忆",
                    quarantined
                );
            }
        }
        // 清除隔离期满的低质量记忆
        if let Ok(purged) = self.purge_quarantine() {
            if purged > 0 {
                eprintln!("[LRC·调节] 隔离区淘汰了 {} 条过期低质量记忆", purged);
            }
        }

        // 用户反馈回路：处理用户的隔离恢复请求和负面反馈
        self.process_user_feedback();

        // 自主记忆垃圾回收：标记为待执行，避免阻塞用户请求关键路径
        // 质疑三核心修复：GC 不在 regulate 中同步执行，而是设置延迟标记。
        // 实际的 GC 工作在 run_gc_if_pending() 中由外部调度触发。
        if self.memory_gc.should_run() {
            // v0.8.48 P0 修复：原子写入，Release 语义确保此前的写操作对执行 GC 的线程可见
            self.gc_pending
                .store(true, std::sync::atomic::Ordering::Release);
        }

        // P1 调节器心跳：记录本次调节执行状态（类型 + 原因 + 时间戳 + 计数）
        // 无论 NoAction 还是真实动作都计入心跳，便于前端展示"调节器在运转"
        self.regulator_heartbeat.record(Some(&action));

        Some(action)
    }

    /// 处理用户反馈（调节周期中的反馈回路）
    ///
    /// 在每次调节周期中处理用户反馈：
    /// 1. 隔离恢复：用户标记被误隔离的记忆 → 恢复到活跃存储
    /// 2. 负面反馈加速：用户多次负面反馈 → 主动标记为低质量触发隔离
    /// 3. 正面反馈保护：用户正面反馈 → 提升合成质量评分，阻止被隔离
    ///
    /// 这是"文档总评 3. 引入用户反馈回路"的实现：
    /// 将人的判断力注入到系统的自主演化中，形成人机协同。
    fn process_user_feedback(&mut self) {
        // 1. 处理隔离恢复请求（用户标记被误隔离的记忆）
        // TOCTOU 防护（2026-09-01 改进）：基于"快照 record_id 精确标记"，
        // 不依赖墙上时钟（系统时钟回拨不会导致已恢复反馈永久无法标记）。
        // 先取 (memory_id, record_id) 精确快照，恢复成功后只标记快照内的记录；
        // 快照后新到的恢复请求不在快照内，保持未处理，下一周期再处理。
        let override_snapshot = self.user_feedback.get_quarantine_override_snapshot();
        if !override_snapshot.is_empty() {
            let override_memory_ids: Vec<String> = override_snapshot
                .iter()
                .map(|(memory_id, _)| memory_id.clone())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            match self.recover_from_quarantine(&override_memory_ids) {
                Ok(recovered) if recovered > 0 => {
                    eprintln!(
                        "[LRC·反馈] 用户反馈回路：恢复了 {} 条被误隔离的记忆",
                        recovered
                    );
                    // 按快照 record_id 精确标记已处理（含持久化），
                    // 避免下一调节周期重复恢复；快照后新到的记录不受影响。
                    let record_ids: Vec<String> =
                        override_snapshot.into_iter().map(|(_, id)| id).collect();
                    self.user_feedback
                        .mark_override_processed_by_records(&record_ids);
                }
                Err(e) => {
                    eprintln!("[LRC·反馈] 隔离恢复失败: {}", e);
                }
                _ => {}
            }
        }

        // 2. 处理用户负面反馈 → 主动标记低质量合成
        let stats = self.user_feedback.get_stats();
        if stats.negative_count > 0 {
            let all = match self.load_cached() {
                Ok(m) => m,
                Err(_) => return,
            };
            // 找出所有合成记忆，检查是否有用户负面反馈
            let synth_memories: Vec<&Memory> = all
                .iter()
                .filter(|m| m.memory_type == MemoryType::Synthesis)
                .collect();

            let mut flagged_ids: Vec<String> = Vec::new();
            for mem in synth_memories {
                if self.user_feedback.should_quarantine_by_user(&mem.id) {
                    // 用户多次负面反馈 → 主动标记为低质量
                    eprintln!(
                        "[LRC·反馈] 用户负面反馈触发：合成记忆 {} 将被标记为低质量",
                        &mem.id[..16.min(mem.id.len())]
                    );
                    // 显式标记低质量（不伪造检索命中，避免污染检索统计）
                    self.synthesis_journal.mark_low_quality(&mem.id);
                    flagged_ids.push(mem.id.clone());
                }
            }
            if !flagged_ids.is_empty() {
                // 记录审计：用户负面反馈驱动低质量标记（人机协同回路）
                self.record_audit(
                    AuditEventType::FeedbackProcessed,
                    format!("用户负面反馈标记 {} 条合成记忆为低质量", flagged_ids.len()),
                    "用户多次负面反馈，主动标记低质量以触发隔离",
                    flagged_ids,
                );
            }
        }

        // 3. 正面反馈保护：清除低质量标记
        // 如果合成记忆获得了足够的正面反馈，撤销低质量标记
        let all = match self.load_cached() {
            Ok(m) => m,
            Err(_) => return,
        };
        for mem in &all {
            if mem.memory_type == MemoryType::Synthesis {
                let positive = self.user_feedback.get_positive_feedback_count(&mem.id);
                if positive >= 2 {
                    // 用户确认合成质量好 → 提升合成日志中的质量评分
                    if self
                        .synthesis_journal
                        .get_events()
                        .iter()
                        .any(|e| e.synthesis_id == mem.id && e.low_quality)
                    {
                        eprintln!(
                            "[LRC·反馈] 用户正面反馈保护：合成记忆 {} 的低质量标记已撤销",
                            &mem.id[..16.min(mem.id.len())]
                        );
                        // 显式撤销低质量标记（不伪造检索命中，避免污染检索统计）
                        self.synthesis_journal.clear_low_quality(&mem.id);
                    }
                }
            }
        }
    }

    /// 记录隐式反馈信号（质疑三：被动反馈，防止"沉默螺旋"）
    ///
    /// 即使用户不主动反馈，系统也可以通过其行为推断相关性。
    /// 支持的信号类型：Click（点击）、Copy（复制）、Dwell（停留）、
    /// RepeatQuery（重复查询）、Ignore（忽略）。
    ///
    /// 这些隐式信号作为调节器和合成器的软标签，持续校准系统。
    pub fn record_implicit_signal(&self, signal: ImplicitSignal) {
        self.user_feedback.record_implicit_signal(signal);
    }

    /// 获取基于隐式信号的记忆质量调整建议
    ///
    /// 返回 (memory_id, quality_adjustment) 的列表。
    /// 正值表示用户隐式认可该记忆，负值表示隐式否定。
    pub fn get_implicit_quality_adjustments(&self) -> Vec<(String, f32)> {
        self.user_feedback.get_implicit_quality_adjustments()
    }

    /// 记录审计事件（封装 audit_trail.record()，降低各调用点的重复代码）
    ///
    /// 用于系统自主行为（GC 清理、合成、调节等）的可回溯审计。
    /// 事件通过哈希链防篡改，并异步持久化到审计日志文件。
    pub fn record_audit(
        &mut self,
        event_type: AuditEventType,
        description: impl Into<String>,
        reason: impl Into<String>,
        affected_memory_ids: Vec<String>,
    ) {
        self.audit_trail.record(
            event_type,
            description.into(),
            reason.into(),
            affected_memory_ids,
            std::collections::HashMap::new(),
        );
    }

    /// 道枢映射: 兑卦·泽 (☱) — 说以利贞，GC调度如泽水之自然净化
    /// 执行延迟的垃圾回收（质疑三：异步 GC）
    ///
    /// 质疑三核心方法：将 GC 工作从用户请求的关键路径中解耦。
    /// 当 `gc_pending` 为 true 时执行实际的垃圾回收周期。
    ///
    /// v0.8.48 P0 修复：使用 AtomicBool + compare_exchange 确保
    /// 多个后台任务并发调用时，GC 恰好执行一次。
    ///
    /// 此方法设计为可从以下场景调用：
    ///   - 后台定时任务（低优先级周期调用）
    ///   - 系统空闲时主动调用
    ///   - 下次 remember/recall 操作前调用（非关键路径）
    ///
    /// 返回 Some(stats) 表示本次执行了 GC，None 表示无需执行。
    pub fn run_gc_if_pending(&mut self) -> Option<GcStats> {
        // 原子 CAS：如果当前值为 true，设为 false 并返回 Ok(true) 表示"本线程获取执行权"
        // 如果当前值已为 false，返回 Err(...) 表示"已有其他线程在执行或无需执行"
        use std::sync::atomic::Ordering;
        if self
            .gc_pending
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            // 其他线程已获取 GC 执行权，或当前无待 GC 任务
            return None;
        }

        // 阶段一：收集记忆快照（不可变借用 self）
        let start = std::time::Instant::now();
        let snapshots = MemorySnapshot::collect_all(self);
        let elapsed_ms = start.elapsed().as_millis() as f64;
        // 阶段二：GC 计算候选和待删除列表（仅可变借用 self.memory_gc）
        let (gc_stats, to_delete) = self.memory_gc.collect_garbage(&snapshots);
        // 记录性能基线（质疑三：动态警告阈值，替代固定 500ms）
        self.memory_gc.record_timing(elapsed_ms, snapshots.len());
        // 阶段三：执行删除（可变借用 self.persistence）
        for id in &to_delete {
            let _ = self.persistence.delete_memory(id);
        }
        // 记录审计：GC 垃圾回收（系统自主行为，需可回溯）
        if !to_delete.is_empty() {
            self.record_audit(
                AuditEventType::GcCleanup,
                format!("GC 垃圾回收清理 {} 条记忆", to_delete.len()),
                format!(
                    "累计回收 {} 条，最近移除 {} 条",
                    gc_stats.total_freed, gc_stats.last_removed_count
                ),
                to_delete.clone(),
            );
        }
        // v0.5.4 写操作后标记缓存为脏
        if !to_delete.is_empty() {
            self.invalidate_cache();
        }

        if gc_stats.last_removed_count > 0 {
            eprintln!(
                "[LRC·GC] 异步垃圾回收完成: 删除 {} 条记忆，累计回收 {}",
                gc_stats.last_removed_count, gc_stats.total_freed
            );
        }

        Some(gc_stats)
    }

    /// 道枢映射: 震卦·雷 (☳) — 万物出乎震，隔离恢复如春雷唤醒沉睡
    /// 从隔离区恢复记忆（用户反馈驱动）
    ///
    /// 将隔离区中的指定记忆恢复到活跃存储。
    /// 这是用户反馈回路的关键环节——用户可以对系统的自动隔离决定
    /// 进行人工干预。
    ///
    /// 返回恢复的记忆数量。
    pub fn recover_from_quarantine(
        &mut self,
        memory_ids: &[String],
    ) -> Result<usize, PersistenceError> {
        let archived = self
            .persistence
            .load_archived_memories()
            .unwrap_or_default();
        if archived.is_empty() {
            return Ok(0);
        }

        let mut recovered = 0usize;
        let mut recovered_ids: Vec<String> = Vec::new();
        let mut remaining: Vec<Memory> = Vec::new();

        for mem in archived {
            if memory_ids.contains(&mem.id) {
                // 恢复到活跃存储
                let mut restored = mem.clone();
                restored.last_accessed = chrono::Utc::now();
                self.persistence.save_memory(&restored)?;
                recovered += 1;
                recovered_ids.push(mem.id.clone());
                eprintln!(
                    "[LRC·恢复] 用户反馈驱动：记忆 {} 已从隔离区恢复到活跃存储",
                    &mem.id[..16.min(mem.id.len())]
                );
            } else {
                remaining.push(mem);
            }
        }

        // 重建归档区
        if recovered > 0 {
            self.persistence.clear_archive()?;
            if !remaining.is_empty() {
                self.persistence.add_to_archive(&remaining)?;
            }
            // v0.5.4 写操作后标记缓存为脏
            self.invalidate_cache();
            // 记录审计：用户反馈驱动隔离恢复（人机协同回路）
            self.record_audit(
                AuditEventType::FeedbackProcessed,
                format!("用户反馈驱动恢复 {} 条被隔离记忆", recovered),
                "用户标记被误隔离的记忆，系统恢复到活跃存储",
                recovered_ids,
            );
        }

        Ok(recovered)
    }

    /// 道枢映射: 兑卦·泽 (☱) — 润泽也，清理低质量合成如泽水之洗涤
    /// 清理低质量合成记忆（隔离 + 渐进式淘汰）
    ///
    /// 解决质疑三"垃圾堆积"问题：SynthesisJournal 标记的低质量合成记忆
    /// 不会立即删除，而是经过"隔离→观察→淘汰"三阶段处理：
    ///
    /// 阶段 1（隔离）：首次发现低质量记忆时，将其移入归档区（隔离区），
    ///   从活跃检索中排除，但保留观察机会。
    /// 阶段 2（观察）：在隔离区中保留 N 个调节周期，等待质量改善。
    ///   如果在此期间被外部修正（如用户反馈），可恢复。
    /// 阶段 3（淘汰）：隔离期满后，永久删除。
    ///
    /// 这种渐进式淘汰避免了"标记后立即遗忘"的粗暴处理，
    /// 给系统留出自我纠错和外部干预的窗口。
    ///
    /// 返回被隔离的记忆数量。
    pub fn clean_low_quality_synthesis(&mut self) -> Result<usize, PersistenceError> {
        let low_quality_ids = self.synthesis_journal.get_low_quality_ids();
        if low_quality_ids.is_empty() {
            return Ok(0);
        }

        let count = low_quality_ids.len();
        eprintln!("[LRC·清理] 发现 {} 条低质量合成记忆，开始隔离...", count);

        // 加载所有记忆，找出低质量合成记忆
        let all = self.load_cached()?;
        let mut quarantined = 0usize;
        let mut quarantined_ids: Vec<String> = Vec::new();
        let mut failed = 0usize;

        for id in &low_quality_ids {
            if let Some(memory) = all.iter().find(|m| m.id == *id) {
                // 阶段 1：移入隔离区（归档），而非直接删除
                match self
                    .persistence
                    .add_to_archive(std::slice::from_ref(memory))
                {
                    Ok(()) => {
                        // 从活跃存储中删除
                        let _ = self.persistence.delete_memory(id);
                        self.synthesis_journal.remove_event(id);
                        quarantined += 1;
                        quarantined_ids.push(id.clone());
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("[LRC·清理] 隔离低质量合成记忆 {} 失败: {}", id, e);
                    }
                }
            } else {
                // 记忆已不存在，仅清理日志
                self.synthesis_journal.remove_event(id);
            }
        }

        // v0.5.4 写操作后标记缓存为脏
        if quarantined > 0 {
            self.invalidate_cache();
            // 记录审计：系统自主隔离低质量合成记忆（三阶段渐进式淘汰·阶段 1）
            self.record_audit(
                AuditEventType::MemoryIsolated,
                format!("隔离 {} 条低质量合成记忆", quarantined),
                "SynthesisJournal 标记为低质量，移入隔离区观察（三阶段渐进式淘汰）",
                quarantined_ids,
            );
            eprintln!(
                "[LRC·清理] 隔离完成: {} 条低质量合成记忆已移入隔离区，{} 条失败",
                quarantined, failed
            );
        }

        Ok(quarantined)
    }

    /// 道枢映射: 离卦·火 (☲) — 明两作，隔离清除如火光之净化
    /// 清除隔离区中的过期记忆（渐进式淘汰的最终阶段）
    ///
    /// 隔离区中的记忆在超过保留期限后被永久删除。
    /// 默认保留期限：3 个调节周期（约 15 分钟），给系统留出观察窗口。
    ///
    /// 返回被永久删除的记忆数量。
    pub fn purge_quarantine(&mut self) -> Result<usize, PersistenceError> {
        let archived = self
            .persistence
            .load_archived_memories()
            .unwrap_or_default();
        if archived.is_empty() {
            return Ok(0);
        }

        let now = chrono::Utc::now();
        // 隔离保留期限：3 个调节周期（默认 5 分钟/周期 = 15 分钟）
        let retention = chrono::Duration::minutes(15);
        let mut purged = 0usize;
        let mut purged_ids: Vec<String> = Vec::new();

        // 筛选需要保留的归档记忆
        let retained: Vec<Memory> = archived
            .into_iter()
            .filter(|m| {
                if m.memory_type == MemoryType::Synthesis {
                    // 合成类型隔离记忆：检查是否过期
                    let age = now - m.last_accessed;
                    if age > retention {
                        purged += 1;
                        purged_ids.push(m.id.clone());
                        false // 过期，淘汰
                    } else {
                        true // 未过期，保留
                    }
                } else {
                    true // 非合成类型（正常过期归档），保留
                }
            })
            .collect();

        if purged > 0 {
            // 重建归档：逐个删除旧归档并重新添加保留的记忆
            // 由于 Persistence trait 没有 clear_archive，我们通过删除+重建来模拟
            // 注意：当前实现仅支持 JSON 持久化，归档文件会被整体重写
            // 实际清理通过只保留未过期记忆来实现
            self.persistence.clear_archive()?;
            if !retained.is_empty() {
                self.persistence.add_to_archive(&retained)?;
            }
            // v0.5.4 写操作后标记缓存为脏
            self.invalidate_cache();
            // 记录审计：隔离期满后永久删除（三阶段渐进式淘汰·阶段 3）
            self.record_audit(
                AuditEventType::MemoryDeleted,
                format!("隔离区淘汰 {} 条过期低质量合成记忆", purged),
                "隔离保留期（15 分钟）届满，永久删除",
                purged_ids,
            );

            eprintln!(
                "[LRC·清理] 隔离区淘汰: {} 条低质量合成记忆已永久删除",
                purged
            );
        }

        Ok(purged)
    }

    /// 写入一条新记忆（含冲突检测、洛书编码、递归合成触发）
    ///
    /// 自动设置 id、created_at 等元数据。
    /// 如果内容与已有记忆高度相似（Jaccard ≥ 阈值），则自动合并：
    /// - 更新内容为新内容
    /// - 合并标签（去重）
    /// - 更新 last_accessed
    /// - 保留原始 id 和 created_at
    ///
    /// 写入后自动：
    /// 1. 洛书编码：将记忆内容编码为 9 维洛书向量
    /// 2. MirrorProject 分类：自动判定记忆的先天八卦类别
    /// 3. 可选道体预判：仅在 pilot 开关开启时附加独立的卦类元数据
    /// 4. 递归合成：若记忆库中相似记忆数 ≥ 3 条，则自动生成合成记忆
    pub fn remember(&mut self, memory: Memory) -> Result<Memory, PersistenceError> {
        // v0.6.0+ 参赛扩展：探索日志埋点（remember 事件）
        let remember_start = std::time::Instant::now();
        let mem_type_str = memory.memory_type.as_str().to_string();
        let mem_importance = memory.importance.value();
        let mem_tags = memory.tags.clone();

        // v0.6.0+ 参赛扩展：基线 A（zero_memory）支持
        // 当 --disable-memory 启用时，remember 直接返回原始记忆但不持久化
        // 用于对照实验：验证"记忆系统"本身的价值
        if std::env::var("LRC_DISABLE_MEMORY").is_ok() {
            let mut no_op_memory = memory.clone();
            no_op_memory.id = format!("zero_memory_{}", chrono::Utc::now().timestamp_millis());
            // 记录探索日志但不实际存储
            self.exploration_logger.log_remember(
                &mem_type_str,
                mem_importance,
                &mem_tags,
                remember_start.elapsed().as_millis() as u64,
            );
            return Ok(no_op_memory);
        }

        // 检查是否有相似记忆
        let similarity_start = std::time::Instant::now();
        // 隐私隔离：从待写入记忆自身派生隐私上下文，相似检查只合并
        // 对当前上下文可见的记忆，避免跨 session/user 污染。
        // 注意：仅当记忆携带真实归属标识（session_id/user_id）时才启用过滤——
        // 无标识的匿名记忆（含默认 User 级但未设 user_id）保持既有合并行为，
        // 避免默认配置下自动合并功能被隐私过滤意外禁用（回归）。
        let remember_privacy = if memory.session_id.is_none() && memory.user_id.is_none() {
            None
        } else {
            Some((
                memory.privacy_level,
                memory.session_id.clone(),
                memory.user_id.clone(),
            ))
        };
        let similar = if Self::domain_index_enabled() {
            self.find_similar_scoped_with_privacy(
                &memory.content,
                Some(&memory),
                &remember_privacy,
            )?
        } else {
            self.find_similar_scoped_with_privacy(&memory.content, None, &remember_privacy)?
        };
        if std::env::var_os("LRC_PROFILE_REMEMBER").is_some() {
            eprintln!(
                "[LRC_PROFILING] remember similarity_lookup_ms={:.3} found={}",
                similarity_start.elapsed().as_secs_f64() * 1000.0,
                similar.is_some()
            );
        }
        let mut result = if let Some(existing) = similar.as_ref() {
            // 合并标签（去重）
            let mut merged_tags = existing.tags.clone();
            for tag in &memory.tags {
                if !merged_tags.contains(tag) {
                    merged_tags.push(tag.clone());
                }
            }

            // 构建合并后的记忆
            let mut merged = existing.clone();
            let old_content = merged.content.clone();
            merged.content = memory.content;
            merged.tags = merged_tags;
            merged.daoti_preview_gua = memory.daoti_preview_gua;
            merged.daoti_preview_bagua = memory.daoti_preview_bagua;
            merged.daoti_preview_version = memory.daoti_preview_version;
            merged.touch();

            // 如果新记忆的重要性更高，则提升
            if memory.importance > merged.importance {
                merged.importance = memory.importance;
            }

            // 自动建立冲突关系边（Section 3.3 冲突解决）
            if self.graph_store.is_some() {
                let jaccard = self.compute_jaccard(&old_content, &merged.content);
                if let Some(ref mut graph) = self.graph_store {
                    // 内容实质不同的合并 → Contradicts 边（需要后续解决）
                    if jaccard < 0.9 {
                        // 相似但不等同 → 可能是矛盾或演进
                        let _ = graph.add_edge(
                            &memory.id,
                            &existing.id,
                            EdgeType::Contradicts,
                            jaccard,
                        );
                    }
                    // 内容更新 → Evolves 边
                    let _ = graph.add_edge(&memory.id, &existing.id, EdgeType::Evolves, jaccard);
                }
            }

            merged
        } else {
            // 无冲突，正常写入
            memory
        };

        // 洛书编码 + 八卦分类（透明地附加到每条记忆）
        {
            let luoshu_vec = self.luoshu_encoder.encode_text(&result.content);
            let proj = mirror_project(&luoshu_vec);

            result.luoshu_vector = Some(luoshu_vec.values);
            result.bagua_index = Some(proj.best_index as u8);
            result.bagua_category = Some(proj.best_category.to_string());

            // 计算拓扑深度：中心值越高（越靠近太极），深度越小（越持久）
            // topological_depth = 1.0 - center_value（归一化到 0.0~1.0）
            let center_val = luoshu_vec.center_value();
            result.topological_depth = (1.0 - center_val).clamp(0.0, 1.0);
        }

        // 统一在编码和分类完成后持久化一次，避免单条写入重复重写整个 JSON。
        self.persistence.save_memory(&result)?;
        if let Some(existing) = similar.as_ref() {
            self.replace_memory_in_index(existing, &result);
        } else {
            self.add_memory_to_index(&result);
        }

        // 记录指标：编码 + 1
        self.dao_metrics.record_encoding();

        // v0.5.4 合成触发移出关键路径：标记待合成，由后台运行
        // 洛书合成基于几何分类和门控融合，替代 Jaccard 文本相似度聚类
        // v0.8.48 P0 修复：原子写入，Release 语义确保此前的写操作对执行合成的线程可见
        self.synthesis_pending
            .store(true, std::sync::atomic::Ordering::Release);

        // 写入后仅刷新记忆快照，保留已完成的增量索引。
        self.mark_cache_dirty_preserving_index();

        // v0.6.0+ 参赛扩展：探索日志记录（remember 事件）
        self.exploration_logger.log_remember(
            &mem_type_str,
            mem_importance,
            &mem_tags,
            remember_start.elapsed().as_millis() as u64,
        );

        // LRC 内置道体状态机：新写入的记忆代表"当前语境"，以重要性激活，
        // 让后续检索能感知"用户刚刚在记什么"，形成跨调用的话题延续。
        self.memory_state_machine.activate(
            &result.id,
            (result.importance.value() as f32 / 10.0).clamp(0.0, 1.0),
        );
        let _ = self
            .persistence
            .save_memory_state(&self.memory_state_machine.snapshot());

        Ok(result)
    }

    /// 批量记忆注入（快速路径，LongMemEval 优化版）
    ///
    /// 一次性注入多条记忆，比逐条调用 remember 快 10-30 倍。
    ///
    /// 优化策略：
    /// 1. 跳过相似性检查（适用于每条记忆独立的场景，如 LongMemEval）
    /// 2. 直接追加写入（不触发 clear+re-save 全量重写）
    /// 3. 跳过洛书合成（合成对检索无直接帮助，且在大批量下是 O(N^2) 瓶颈）
    /// 4. 保留洛书编码（L2 层 trapezoid_focus_recall 几何检索仍可使用）
    ///
    /// 适用于 LongMemEval 等需要大量注入独立会话历史的场景。
    pub fn remember_batch(
        &mut self,
        memories: Vec<Memory>,
    ) -> Result<Vec<Memory>, PersistenceError> {
        if memories.is_empty() {
            return Ok(vec![]);
        }

        // 快速批量注入路径（LongMemEval 优化）：
        // 跳过相似性检查（每条会话独立唯一），直接编码并追加写入，
        // 避免 O(N*M) 的相似度比较和 clear+re-save 的昂贵全量重写。
        let mut results = Vec::with_capacity(memories.len());

        for memory in memories {
            let mut result = memory;

            // 洛书编码 + 八卦分类（保留 L2 层检索能力）
            let luoshu_vec = self.luoshu_encoder.encode_text(&result.content);
            let proj = mirror_project(&luoshu_vec);
            result.luoshu_vector = Some(luoshu_vec.values);
            result.bagua_index = Some(proj.best_index as u8);
            result.bagua_category = Some(proj.best_category.to_string());
            let center_val = luoshu_vec.center_value();
            result.topological_depth = (1.0 - center_val).clamp(0.0, 1.0);

            // 先完成整批编码，持久化统一在循环结束后执行。
            results.push(result);
            self.dao_metrics.record_encoding();
        }

        // 批量写入只执行一次全量序列化和磁盘写入。
        self.persistence.save_memories(&results)?;
        for result in &results {
            self.add_memory_to_index(result);
        }

        // 注意：此处不在关键路径内执行合成（luoshu_synthesize），因为：
        // 1. 合成操作（簇发现、摘要生成）在大批量数据下耗时巨大（O(N^2) 级别）
        // 2. 合成会延后到健康检查（system_health/memory_stats）时通过 run_pending_synthesis 执行
        //
        // v0.6.x 参赛修复：与单条 remember 路径保持一致，批量注入后同样标记待合成，
        // 使"完整 LRC"实验组（full_lrc）能真实运行演化（合成）机制，而非静默跳过。
        // 该标记仅置位，实际合成仍由后台健康检查执行，不阻塞本调用。
        // v0.8.48 P0 修复：原子写入，Release 语义确保此前的写操作对执行合成的线程可见
        self.synthesis_pending
            .store(true, std::sync::atomic::Ordering::Release);

        // 批量写入后仅刷新记忆快照，保留已完成的增量索引。
        self.mark_cache_dirty_preserving_index();

        Ok(results)
    }

    /// 梯形聚焦检索（Section 3.2 TrapezoidFocus）
    ///
    /// 使用洛书九宫格几何结构进行空间分区检索：
    /// 1. 将查询文本编码为洛书向量
    /// 2. 以查询向量的重心位置为中心创建 TrapezoidROI
    /// 3. 递归细分为 4^depth 个子区域
    /// 4. 仅检索落在密度最高子区域内的记忆
    ///
    /// 复杂度：O(N / 4^depth)，depth=2 时仅检索 ~6% 的记忆
    ///
    /// 参数：
    /// - `query`: 查询文本
    /// - `filter`: 召回过滤条件
    /// - `depth`: 梯形细分深度（0=全量，1=4分，2=16分）
    pub fn trapezoid_focus_recall(
        &mut self,
        query: &str,
        filter: &RecallFilter,
        depth: u32,
    ) -> Result<RecallResult, PersistenceError> {
        self.trapezoid_focus_recall_with_cancel(query, filter, depth, None)
    }

    pub fn trapezoid_focus_recall_with_cancel(
        &mut self,
        query: &str,
        filter: &RecallFilter,
        depth: u32,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<RecallResult, PersistenceError> {
        // 阶段D 可观测性：LRC_DEEP_TRACE=1 时输出审计行（候选/八卦剪除/词面域/top1 来源；默认关闭零开销）
        let deep_trace = std::env::var("LRC_DEEP_TRACE")
            .map(|v| v == "1")
            .unwrap_or(false);
        let mut trace_lex_max = 0.0f32;
        let mut trace_lex_fallback = false;

        // 1. 编码查询文本
        let query_vec = self.luoshu_encoder.encode_text(query);

        // 2. MirrorProject 分类查询向量（用于八卦预过滤）
        let query_proj = mirror_project(&query_vec);
        let query_bagua = query_proj.best_index as u8;

        // 阶段三 b2：预判元数据参与候选剪枝（默认开启）。
        // LRC_DAOTI_PREVIEW_PRUNE=0 / =false 时关闭，退化为仅 LRC 自分类卦硬剪除
        // （保持 v0.8.50 回滚后的行为）。
        let daoti_prune_enabled = !matches!(
            std::env::var("LRC_DAOTI_PREVIEW_PRUNE").as_deref(),
            Ok("0") | Ok("false")
        );

        // 3. 以查询向量重心为中心创建 ROI
        let center = query_vec
            .values
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(4);
        let roi = TrapezoidROI::centered(center, depth);

        let all_memories = self.load_cached()?;
        let total_count = all_memories.iter().filter(|m| !m.is_expired()).count();
        if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(PersistenceError::Other("enrich_cancelled".to_string()));
        }

        // LRC 内置道体状态机·联想导航（deep 候选白名单）：
        // 活跃记忆无论卦象一律进入候选——它们代表"近期在想什么"，
        // 是联想扩散的锚点，不能被八卦硬剪除误丢（记忆丢失的深层原因）。
        let active_whitelist: std::collections::HashSet<String> =
            if crate::engine::memory_state_machine::state_bias_enabled() {
                self.memory_state_machine
                    .active_ids(16)
                    .into_iter()
                    .collect()
            } else {
                std::collections::HashSet::new()
            };
        // 道体再次校验·回归证据收集（deep）：索引 → 证据标签，
        // 由下方校验段填充，随 RecallResult 返回供联想链输出可观测。
        let mut deep_evidence: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        // 4. 构建 (索引, 洛书向量) 对 — 增加八卦预过滤
        let indexed: Vec<(usize, LuoShuVector)> = all_memories
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                if m.is_expired() {
                    return false;
                }
                if let Some(ref mt) = filter.memory_type {
                    if m.memory_type != *mt {
                        return false;
                    }
                }
                if let Some(ref proj) = filter.project {
                    if m.project.as_deref() != Some(proj.as_str()) {
                        return false;
                    }
                }
                if !filter.tags.is_empty() && !filter.tags.iter().any(|t| m.tags.contains(t)) {
                    return false;
                }
                if let Some(min_imp) = filter.min_importance {
                    if m.importance < min_imp {
                        return false;
                    }
                }
                if !is_visible(m, &filter.privacy_context) {
                    return false;
                }
                // 活跃记忆白名单：无论卦象一律保留（联想导航锚点，防记忆丢失）
                if active_whitelist.contains(&m.id) {
                    return m.luoshu_vector.is_some();
                }
                // v0.8.50 检索质量修复 A/B（3111 旧 vs 3122 BM25+八卦降权）后回滚：
                // 八卦降权（0.6~1.0 惩罚）对 deep 命中零改进（top1 2/32 持平），
                // 且把卦近/跨卦的无关全局记忆顶上 top1、污染 RRF top1 命中
                // （model_file_missing/orig、port_binding/rewrite 各退回 1 处）。
                // 按方案 §3.5「预判元数据只做观测、不接入默认排序」，恢复原硬剪除：
                // 仅保留同卦或相邻卦候选，其余剪除。
                // 阶段三 b2：接入道体预判卦（daoti_preview_bagua）作为候选保留的
                // 第二证据——LRC 自分类与道体预判不一致（跨域）时，任一证据命中
                // 即保留，修正跨域污染误剪；仅影响召回候选、不改 RRF 评分权重。
                m.luoshu_vector.is_some() && {
                    // 证据1：LRC 自分类卦（环形距离 ≤1 保留）
                    let lrc_keep = match m.bagua_index {
                        Some(mem_bagua) => {
                            let diff = (mem_bagua as i8 - query_bagua as i8).abs();
                            // 八卦环形距离：diff 与 8-diff 取小者；≤1（同卦/相邻卦）保留
                            let ring_dist = diff.min(8 - diff);
                            ring_dist <= 1
                        }
                        None => true,
                    };
                    if lrc_keep {
                        true
                    } else if daoti_prune_enabled {
                        // 证据2：道体预判卦（按名称映射，修正跨域污染）
                        match m
                            .daoti_preview_bagua
                            .as_deref()
                            .and_then(bagua_name_to_index)
                        {
                            Some(daoti_bagua) => {
                                let diff = (daoti_bagua as i8 - query_bagua as i8).abs();
                                let ring_dist = diff.min(8 - diff);
                                ring_dist <= 1
                            }
                            None => false,
                        }
                    } else {
                        false
                    }
                }
            })
            .filter_map(|(i, m)| m.luoshu_vector.map(|v| (i, LuoShuVector { values: v })))
            .collect();
        // 阶段D 审计：八卦硬剪除后的候选规模
        let trace_candidates = indexed.len();

        // 4. 执行梯形聚焦检索
        let vec_refs: Vec<(usize, &LuoShuVector)> = indexed.iter().map(|(i, v)| (*i, v)).collect();
        let focus_result = roi.focused_recall(&vec_refs);

        // 5. 从匹配索引还原记忆
        let all: Vec<Memory> = all_memories;
        let mut memories: Vec<Memory> = focus_result
            .matched_indices
            .iter()
            .filter_map(|&idx| all.get(idx).cloned())
            .collect();
        // 阶段D 审计：ROI 聚焦原始召回数（词面域过滤前）
        let trace_roi = memories.len();

        // 6. 计算分数（纯洛书向量余弦相似度；八卦已在候选阶段硬剪除，评分不再降权）
        let mut scores: Vec<f32> = memories
            .iter()
            .map(|m| {
                if let Some(ref lv) = m.luoshu_vector {
                    let mem_vec = LuoShuVector { values: *lv };
                    mem_vec.cosine_similarity(&query_vec)
                } else {
                    0.0
                }
            })
            .collect();

        // ========== 语义向量域过滤（词面域融合评分，v0.8.52） ==========
        // 洛书向量无词义：短 query 的余弦前排被与查询无关的长记忆抢占。
        // 以查询词面为语义域锚点：对候选池统计查询 token 的平滑 IDF，
        // 词面域重合分 = 命中 token 的 IDF 加权和，融合 final = 余弦 + 权重×(分/域内最大)；
        // 域外记忆（零重合）不加分，域内记忆按词面相关度提升，域排序由余弦主导。
        // 环境变量 LRC_DEEP_LEX_DOMAIN=0 可关闭本过滤（A/B 对照与回归用）；
        // LRC_DEEP_LEX_DOMAIN_WEIGHT 可覆盖权重（阶段 B scale 消融，默认 0.25）。
        let lex_domain_enabled = std::env::var("LRC_DEEP_LEX_DOMAIN")
            .map(|v| v != "0")
            .unwrap_or(true);
        let lex_domain_weight: f32 = std::env::var("LRC_DEEP_LEX_DOMAIN_WEIGHT")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(LEX_DOMAIN_WEIGHT);
        if lex_domain_enabled {
            let mut query_tokens = tokenize_query(query);
            // LRC 内置道体状态机·联想导航（deep 词面域锚点扩展）：
            // 把活跃记忆内容中的联想桥词并入词面域锚点，让"近期在想什么"
            // 的内容也能获得词面域加分——与 fast 路径的查询扩展对齐。
            if crate::engine::memory_state_machine::state_bias_enabled()
                && !active_whitelist.is_empty()
            {
                let all = self.load_cached().unwrap_or_default();
                let mut bridge_text = String::new();
                for m in &all {
                    if active_whitelist.contains(&m.id) {
                        bridge_text.push_str(&m.content);
                        bridge_text.push(' ');
                    }
                }
                let mut seen: std::collections::HashSet<String> =
                    query_tokens.iter().cloned().collect();
                for w in tokenize_query(&bridge_text) {
                    if seen.insert(w.clone()) {
                        query_tokens.push(w);
                    }
                }
                if query_tokens.len() > 32 {
                    query_tokens.truncate(32);
                }
            }
            if !query_tokens.is_empty() && !memories.is_empty() {
                // 候选池内 DF：查询 token 在多少候选 content 中出现（词边界匹配，与 fast 路一致）
                let mut doc_freq: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                for w in &query_tokens {
                    // v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P1 并发「取消标志内存序不一致」）：
                    //   根因：同一 cancel 标志的其他 4 处检查点（:3047/:3510/:3631/:3917）均用 Acquire，
                    //         写入端为 Release（v1_api.rs:1812/:1953），唯此处用 Relaxed——
                    //         不构成 Release/Acquire 配对，理论上无法保证看到取消前的写入可见性，
                    //         在弱内存序平台（ARM）存在读到过期值的风险，导致取消响应延迟或失效。
                    //   修复：统一为 Acquire，与全仓其余检查点及写入端配对。
                    if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
                        return Err(PersistenceError::Other("enrich_cancelled".to_string()));
                    }
                    for m in &memories {
                        if contains_word(&m.content.to_lowercase(), w) {
                            *doc_freq.entry(w.as_str()).or_insert(0) += 1;
                        }
                    }
                }
                let n_docs = memories.len() as f32;
                // 词面域重合分：命中 token 的平滑 IDF 累加
                let lex_domain: Vec<f32> = memories
                    .iter()
                    .map(|m| {
                        let lower = m.content.to_lowercase();
                        query_tokens
                            .iter()
                            .map(|w| {
                                if contains_word(&lower, w) {
                                    let df = *doc_freq.get(w.as_str()).unwrap_or(&0) as f32;
                                    ((n_docs + 1.0) / (df + 1.0)).ln()
                                } else {
                                    0.0
                                }
                            })
                            .sum()
                    })
                    .collect();
                let lex_max = lex_domain.iter().copied().fold(0.0f32, f32::max);
                // 阶段D 审计：词面域是否回退纯余弦
                trace_lex_max = lex_max;
                // 融合：仅在有词面重合时生效；候选池与查询零重合（如纯英文 query 对中文库）回退纯余弦
                if lex_max > 0.0 {
                    for (s, lex) in scores.iter_mut().zip(lex_domain.iter()) {
                        *s += lex_domain_weight * (lex / lex_max);
                    }
                } else {
                    trace_lex_fallback = true;
                }
            }
        }

        // LRC 内置道体状态机·活性偏置（与 fast 路径一致）：
        // 即便查询措辞与活跃记忆无词面重叠，也按激活强度给加分，
        // 让"近期在想什么"对 deep 检索同样生效，保持双路径行为一致。
        if crate::engine::memory_state_machine::state_bias_enabled() && !memories.is_empty() {
            let active_map: std::collections::HashMap<String, f32> = self
                .memory_state_machine
                .active_context(usize::MAX)
                .into_iter()
                .collect();
            for (m, s) in memories.iter().zip(scores.iter_mut()) {
                let activation = active_map.get(&m.id).copied().unwrap_or(0.0);
                if activation > 0.0 {
                    *s += (activation * 0.25).min(0.25);
                }
            }
        }

        // 7. 按分数排序并截取 top_k
        let mut scored: Vec<(usize, f32)> = (0..memories.len()).map(|i| (i, scores[i])).collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // LRC 内置道体状态机·道体再次校验（deep 回归验证层）：
        // 与 fast 路径同语义——联想扩散（词面域锚点扩展）拉进候选的记忆，
        // 必须在输出前验证与原查询的共鸣。活跃记忆白名单是联想锚点本身，
        // 不参与校验（它们代表近期语境，保留是设计意图）。
        if !active_whitelist.is_empty() {
            let original_tokens = tokenize_query(query);
            let bridge_words: Vec<String> = {
                let all = self.load_cached().unwrap_or_default();
                let mut bridge_text = String::new();
                for m in &all {
                    if active_whitelist.contains(&m.id) {
                        bridge_text.push_str(&m.content);
                        bridge_text.push(' ');
                    }
                }
                tokenize_query(&bridge_text)
            };
            let mut rejected: usize = 0;
            // 收集回归证据：索引 → 证据标签（供联想链输出可观测）
            let mut evidence_by_idx: std::collections::HashMap<usize, String> =
                std::collections::HashMap::new();
            let kept_indices: Vec<usize> = scored
                .iter()
                .map(|(i, _)| *i)
                .filter(|&i| {
                    use crate::engine::memory_state_machine::regression_recheck;
                    let m = &memories[i];
                    // 白名单活跃记忆：联想锚点，直接保留
                    if active_whitelist.contains(&m.id) {
                        evidence_by_idx.insert(i, "活跃锚点".to_string());
                        return true;
                    }
                    let content_lower = m.content.to_lowercase();
                    let original_overlap = original_tokens
                        .iter()
                        .filter(|w| contains_word(&content_lower, w))
                        .count();
                    let bridge_hits = bridge_words
                        .iter()
                        .filter(|w| contains_word(&content_lower, w))
                        .count();
                    let tag_hits = m
                        .tags
                        .iter()
                        .filter(|t| original_tokens.iter().any(|w| t.to_lowercase().contains(w)))
                        .count();
                    let verdict = regression_recheck(original_overlap, bridge_hits, tag_hits, None);
                    if verdict.keep {
                        evidence_by_idx.insert(i, verdict.evidence.to_string());
                    } else {
                        rejected += 1;
                    }
                    verdict.keep
                })
                .collect();
            if rejected > 0 {
                eprintln!("[LRC-STATE] deep 道体再次校验剔除 {} 条发散噪声", rejected);
            }
            // 用校验后的索引重建 scored
            let kept: std::collections::HashSet<usize> = kept_indices.iter().copied().collect();
            scored.retain(|(i, _)| kept.contains(i));
            // 将证据映射到最终输出的记忆 id
            deep_evidence = kept_indices
                .iter()
                .filter_map(|&i| {
                    evidence_by_idx
                        .get(&i)
                        .map(|e| (memories[i].id.clone(), e.clone()))
                })
                .collect();
        }

        // v0.5.4 P2-12 修复：按 content 哈希去重，保留匹配度最高的那条
        // 在排序后、截取 top_k 前进行去重，确保深度检索结果中不会出现内容相同的记忆
        let mut seen_content: std::collections::HashSet<String> = std::collections::HashSet::new();
        let top_k = filter.top_k.min(scored.len());
        let top_indices: Vec<usize> = scored
            .iter()
            .filter(|(i, _)| {
                let content_key = memories[*i].content.trim().to_lowercase();
                seen_content.insert(content_key)
            })
            .take(top_k)
            .map(|(i, _)| *i)
            .collect();

        memories = top_indices.iter().map(|&i| memories[i].clone()).collect();
        scores = top_indices.iter().map(|&i| scores[i]).collect();

        // 8. 深度检索保持只读，不在搜索请求中更新访问时间或写回磁盘。
        // 这样可避免 ML 模式下全量清空并逐条重写 memories.json 导致请求超时。

        // 阶段D 可观测性：LRC_DEEP_TRACE=1 输出审计行（默认关闭，生产零开销）
        if deep_trace {
            let top1 = memories.first();
            eprintln!(
                "[LRC-DEEP-TRACE] {}",
                serde_json::json!({
                    "query": query.chars().take(60).collect::<String>(),
                    "query_bagua": query_bagua,
                    "depth": depth,
                    "top_k": filter.top_k,
                    "recall": {
                        "bagua_pruned_candidates": trace_candidates,
                        "roi_matched": trace_roi,
                        "lex_max": trace_lex_max,
                        "lex_fallback": trace_lex_fallback,
                        "lex_weight": lex_domain_weight,
                    },
                    "top1": top1.map(|m| {
                        serde_json::json!({
                            "id": m.id,
                            "source": m.source,
                            "project": m.project,
                            "bagua": m.bagua_index,
                            "preview_gua": m.daoti_preview_gua,
                            "score": scores.first().copied().unwrap_or(0.0),
                            "preview": m.content.chars().take(80).collect::<String>(),
                        })
                    }),
                })
            );
        }

        self.dao_metrics.record_recall();

        // 质量反馈闭环：记录合成记忆被检索命中的相关性
        for (mem, score) in memories.iter().zip(scores.iter()) {
            if mem.memory_type == MemoryType::Synthesis {
                self.synthesis_journal.record_hit(&mem.id, *score);
            }
        }

        // LRC 内置道体状态机：激活本次召回的记忆、记录联想轨迹并持久化。
        // 这样下一次检索可感知"近期在想什么"，让信息从"查询依赖"变为"上下文依赖"。
        self.bake_activation(&memories, &scores);

        // v0.5.4 检索后合成标记移出关键路径：由后台运行
        if memories.len() >= self.synthesis_min_cluster {
            // v0.8.48 P0 修复：原子写入，Release 语义确保此前的写操作对执行合成的线程可见
            self.synthesis_pending
                .store(true, std::sync::atomic::Ordering::Release);
        }

        Ok(RecallResult {
            memories,
            scores,
            total: total_count,
            regression_evidence: deep_evidence,
        })
    }
    /// 将召回结果写入内置道体状态机的活性与联想轨迹。
    fn populate_state(
        &mut self,
        memories: &[Memory],
        scores: &[f32],
    ) -> Result<(), PersistenceError> {
        let mut previous = None;
        for (memory, score) in memories.iter().zip(scores.iter()) {
            self.memory_state_machine
                .transition(previous, &memory.id, *score, 1);
            previous = Some(memory.id.as_str());
        }
        self.persistence
            .save_memory_state(&self.memory_state_machine.snapshot())
    }

    /// LRC 内置道体状态机·激活快照：
    /// 激活本次召回的记忆、记录联想轨迹并持久化。
    /// 快照写入失败不阻塞检索："记忆活性"是增强项，不应影响主路径。
    fn bake_activation(&mut self, memories: &[Memory], scores: &[f32]) {
        for (mem, score) in memories.iter().zip(scores.iter()) {
            self.memory_state_machine.activate(&mem.id, *score);
        }
        if !memories.is_empty() {
            let _ = self.populate_state(memories, scores);
        }
    }

    /// 联想确认（v0.9.7：用户在联想探索中点击"就是这个"）。
    ///
    /// 用户确认某条记忆与当前意图相关：以最高激活强度（1.0）写入道体
    /// 状态机活跃锚点并持久化，下次联想/检索时该记忆优先呈现。
    /// 返回 Ok(true) 表示记忆存在且已确认；Ok(false) 表示记忆不存在。
    pub fn confirm_memory(&mut self, memory_id: &str) -> Result<bool, PersistenceError> {
        let exists = self
            .load_cached()?
            .iter()
            .any(|m| m.id == memory_id && !m.is_expired());
        if !exists {
            return Ok(false);
        }
        // 用户显式确认 = 最高激活强度；轨迹记录一次确认转移并持久化快照
        self.memory_state_machine.activate(memory_id, 1.0);
        self.memory_state_machine
            .transition(None, memory_id, 1.0, 1);
        let _ = self
            .persistence
            .save_memory_state(&self.memory_state_machine.snapshot());
        Ok(true)
    }

    /// 道枢映射: 道枢·检索 — 记忆召回是系统的核心能力，如道枢之"环中"应对无穷
    /// 语义搜索记忆
    ///
    /// 当前使用文本匹配算法（关键词提取 + 子串匹配 + 词频评分）。
    /// 检索到的记忆会自动更新 `last_accessed` 字段，使衰减模型正确工作。
    pub fn recall(
        &mut self,
        query: &str,
        filter: &RecallFilter,
    ) -> Result<RecallResult, PersistenceError> {
        self.recall_with_cancel(query, filter, None)
    }

    pub fn recall_with_cancel(
        &mut self,
        query: &str,
        filter: &RecallFilter,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<RecallResult, PersistenceError> {
        // v0.6.0+ 参赛扩展：探索日志埋点（recall 事件）
        let recall_start = std::time::Instant::now();
        let recall_top_k = filter.top_k;

        let all_memories = self.load_cached()?;
        let total_count = all_memories.iter().filter(|m| !m.is_expired()).count();
        if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(PersistenceError::Other("enrich_cancelled".to_string()));
        }

        // v0.5.4 P1-9 修复：使用智能分词替代 split_whitespace()
        // 对中文文本使用 bigram 分词，解决中文检索精度问题
        let query_lower = query.to_lowercase();
        let query_words: Vec<String> = tokenize_query(query);

        // LRC 内置道体状态机·联想导航（查询扩展）：
        // 把"近期活跃记忆"的内容作为联想桥并入查询词。这样即使本次查询与
        // 目标记忆无词面重叠（如"今天晚饭吃什么" vs "粤菜餐厅牛肉丸"），
        // 活跃记忆中的高频实词也能把相关记忆拉回候选——解决"记忆丢失"。
        // 回归约束：扩展词仅在候选与原查询零词重叠时以封顶加分生效，
        // 且权重低于原查询词，避免发散跑偏。开关 LRC_STATE_BIAS=0 关闭。
        // v0.9.7 精确度修复：explore_pure（联想探索）模式下不做查询扩展，
        // 探索语义完全由原查询主导。
        let (query_words, expansion_boost) =
            if crate::engine::memory_state_machine::state_bias_enabled() && !filter.explore_pure {
                let active_ids = self.memory_state_machine.active_ids(8);
                if active_ids.is_empty() {
                    (query_words, Vec::<(String, f32)>::new())
                } else {
                    // 从活跃记忆内容抽取联想桥词（复用全库缓存，避免额外 I/O）
                    let all = self.load_cached().unwrap_or_default();
                    let id_set: std::collections::HashSet<&str> =
                        active_ids.iter().map(|s| s.as_str()).collect();
                    let mut bridge_text = String::new();
                    for m in &all {
                        if id_set.contains(m.id.as_str()) {
                            bridge_text.push_str(&m.content);
                            bridge_text.push(' ');
                        }
                    }
                    let bridge_words: Vec<String> = tokenize_query(&bridge_text);
                    // 扩展词去重（排除已属于原查询的词）
                    let mut seen: std::collections::HashSet<String> =
                        query_words.iter().cloned().collect();
                    let mut expansion_boost: Vec<(String, f32)> = Vec::new();
                    for w in bridge_words {
                        if seen.insert(w.clone()) {
                            expansion_boost.push((w, 0.20));
                        }
                    }
                    // 联想桥词上限：防长活跃记忆稀释原查询主导地位
                    if expansion_boost.len() > 16 {
                        expansion_boost.truncate(16);
                    }
                    let mut merged = query_words;
                    merged.extend(expansion_boost.iter().map(|(w, _)| w.clone()));
                    (merged, expansion_boost)
                }
            } else {
                (query_words, Vec::<(String, f32)>::new())
            };
        let query_word_refs: Vec<&str> = query_words.iter().map(|s| s.as_str()).collect();
        // 原查询词（不含联想桥扩展词）——用于"零词重叠"判断：只有与原查询
        // 完全无重叠的记忆才有资格获得联想桥扩展分，否则仍以原查询分主导。
        // v0.9.7：联想探索设置 regression_query 后，回归校验锚定到该查询
        //（起点记忆主题）而非当前 recall 查询（父记忆内容），
        // 防止多跳扩散的语义随父内容漂移到无关领域。
        let recheck_query: &str = filter.regression_query.as_deref().unwrap_or(query);
        let original_query_words: Vec<String> = tokenize_query(recheck_query);
        let original_query_refs: Vec<&str> =
            original_query_words.iter().map(|s| s.as_str()).collect();

        // 道体再次校验·回归证据收集（memory_id → 证据标签）：
        // 函数级声明，由 scored 块内校验段填充，随 RecallResult 返回
        // 供联想链输出可观测。
        let mut regression_evidence: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        // 用不可变引用评分和排序（只读：不在热路径更新 last_accessed 或同步重写文件）
        let (memories, scores) = {
            // 过滤记忆
            let privacy_ctx = filter.privacy_context.clone();
            let candidates: Vec<&Memory> = all_memories
                .iter()
                .filter(|m| !m.is_expired())
                .filter(|m| {
                    // 类型过滤
                    if let Some(ref mt) = filter.memory_type {
                        if m.memory_type != *mt {
                            return false;
                        }
                    }
                    // 项目过滤
                    if let Some(ref proj) = filter.project {
                        if m.project.as_deref() != Some(proj.as_str()) {
                            return false;
                        }
                    }
                    // 标签过滤
                    if !filter.tags.is_empty() && !filter.tags.iter().any(|t| m.tags.contains(t)) {
                        return false;
                    }
                    // 重要性过滤
                    if let Some(min_imp) = filter.min_importance {
                        if m.importance < min_imp {
                            return false;
                        }
                    }
                    // 隐私权限过滤（Section 3.3）
                    if !is_visible(m, &privacy_ctx) {
                        return false;
                    }
                    true
                })
                .collect();

            // 计算匹配分数（使用 TF-IDF 加权，替代简单的关键词匹配）
            // TF-IDF 能更好地区分相关和无关记忆，尤其是对于长文本记忆
            // 参考: LongMemEval 基准测试验证了 TF-IDF 在长对话记忆检索中的有效性
            // v0.5.4 P1-9 修复：使用 query_word_refs 替代 query_words，支持 CJK bigram
            // v0.5.5 修复二：使用 contains_word 替代 contains，避免 "cat" 匹配 "category"
            let mut scored: Vec<(f32, &Memory)> = {
                // ========== TF-IDF 预处理 ==========
                // 计算文档频率（DF）: 每个查询词在多少条候选记忆中出现
                let mut doc_freq: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                for word in &query_word_refs {
                    if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
                        return Err(PersistenceError::Other("enrich_cancelled".to_string()));
                    }
                    for m in &candidates {
                        let content_lower = self.recall_document(m).normalized_content;
                        // v0.5.5 修复二：词边界匹配替代子串匹配
                        if contains_word(&content_lower, word) {
                            *doc_freq.entry(word).or_insert(0) += 1;
                        }
                    }
                }

                // 计算 IDF（逆文档频率）: 稀有词获得更高权重
                let n_docs = candidates.len().max(1) as f32;
                let idf: std::collections::HashMap<&str, f32> = query_word_refs
                    .iter()
                    .map(|word| {
                        let df = *doc_freq.get(word).unwrap_or(&0) as f32;
                        // 使用平滑 IDF: log((N + 1) / (df + 1)) + 1，避免除零和负值
                        let idf_val = ((n_docs + 1.0) / (df + 1.0)).ln() + 1.0;
                        (*word, idf_val)
                    })
                    .collect();

                // v0.8.50 BM25 检索质量修复：计算候选集合平均文档长度（token 数），
                // 供 BM25 饱和归一使用。候选集即过滤后的全部未过期记忆。
                let avgdl: f32 = {
                    let mut total_tokens: usize = 0;
                    for m in &candidates {
                        total_tokens += self.recall_document(m).token_count;
                    }
                    total_tokens as f32 / candidates.len().max(1) as f32
                };

                // LRC 内置道体状态机·活性偏置：
                // 将"近期活跃记忆"预构建为 id->激活强度 映射，供后续逐候选 O(1) 查询，
                // 避免在 N×M 打分循环中反复构造活跃列表。
                let active_map: std::collections::HashMap<String, f32> = self
                    .memory_state_machine
                    .active_context(usize::MAX)
                    .into_iter()
                    .collect();

                candidates
                    .iter()
                    .map(|m| {
                        if cancel
                            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
                        {
                            return (0.0, *m);
                        }
                        let document = self.recall_document(m);
                        let content_lower = &document.normalized_content;
                        let mut score: f32 = 0.0;

                        // 完全匹配加分（精确匹配整句查询时额外加分）
                        if content_lower.contains(&query_lower) {
                            score += 0.4;
                        }
                        // TF-IDF 词匹配加分（替代原 0.1/词的固定权重）
                        // 对每个查询词，计算其在当前记忆中的词频（TF），乘以 IDF
                        // 归一化 TF：除以文档长度（token 数），避免长文本获得不合理的
                        // 高分。短文档中关键词密度更高，应获得加权。
                        // v0.5.4 P1-9 修复：使用 doc_token_count 替代 split_whitespace().count()
                        // 对 CJK 文本基于 bigram 数量计算文档长度
                        // v0.5.5 修复二：使用 contains_word + count_word_occurrences
                        // 替代 contains + matches().count()，避免子串误匹配
                        let doc_len = document.token_count as f32;
                        // 原查询词重叠计数：>0 表示该记忆本就与查询直接相关，
                        // 此时扩展词不应加分（原查询主导，防发散稀释）。
                        let original_overlap: usize = original_query_refs
                            .iter()
                            .filter(|word| contains_word(content_lower, word))
                            .count();
                        for word in &query_word_refs {
                            if contains_word(content_lower, word) {
                                // 联想桥扩展词的权重（0.20）；原查询词权重为 1.0。
                                // 回归约束：仅当该记忆与原查询零重叠时才给扩展分——
                                // 扩展词只负责把"孤立但相关"的记忆拉进候选，
                                // 已经与原查询直接匹配的记忆不需要联想桥。
                                let expansion_weight = expansion_boost
                                    .iter()
                                    .find(|(w, _)| w.as_str() == *word)
                                    .map(|(_, w)| *w)
                                    .unwrap_or(1.0);
                                if expansion_weight < 1.0 && original_overlap > 0 {
                                    continue;
                                }
                                // 计算词频（TF）: 该词在记忆内容中以整词形式出现的次数
                                let tf = count_word_occurrences(content_lower, word) as f32;
                                let idf_val = idf.get(word).copied().unwrap_or(1.0);
                                // v0.8.50 BM25 检索质量修复：以 BM25 TF 饱和项替代线性
                                // (词频/文档长度) 归一。长文档不再因 doc_len 线性放大而稀释
                                // 精确短语命中，短文档关键词密度收益保留；参数取 Lucene/ES
                                // 默认 k1=1.2、b=0.75。
                                score += expansion_weight * idf_val * tf * (BM25_K1 + 1.0)
                                    / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * doc_len / avgdl));
                            }
                        }

                        // 标签匹配加分（标签是用户主动标注的元数据，具有高信息量）
                        // v0.5.4 P1-9 修复：使用 query_word_refs 支持中文标签匹配
                        for tag in &m.tags {
                            for word in &query_word_refs {
                                if tag.to_lowercase().contains(word) {
                                    score += 0.15;
                                }
                            }
                        }

                        // 重要性加权（含衰减因子，使用可配置衰减曲线）
                        score += m.decayed_importance_with_config(&self.decay_config) * 0.01;

                        // LRC 内置道体状态机·活性偏置：
                        // 仅当该记忆当前处于活跃状态（近期被召回）时，才依据其
                        // 激活强度加一小分。这样"近期在想什么"能温和地影响下一次
                        // 检索，但不会把不相关的高活性记忆顶到前面（因为基础分
                        // 仍是内容匹配主导）。默认开关 LRC_STATE_BIAS=1 启用，
                        // 设 0 可精确回退到旧行为。
                        // v0.9.7：explore_pure（联想探索）模式下跳过，探索排序
                        // 不受"近期语境"牵引。
                        if crate::engine::memory_state_machine::state_bias_enabled()
                            && !filter.explore_pure
                        {
                            let activation = active_map.get(&m.id).copied().unwrap_or(0.0);
                            if activation > 0.0 {
                                // 活性偏置上限 0.25，避免盖过内容匹配主导地位。
                                score += (activation * 0.25).min(0.25);
                            }
                        }

                        // 类型匹配加权
                        if (query_lower.contains("偏好") || query_lower.contains("prefer"))
                            && m.memory_type == MemoryType::Preference
                        {
                            score += 0.2;
                        }
                        if (query_lower.contains("决定")
                            || query_lower.contains("选择")
                            || query_lower.contains("decision"))
                            && m.memory_type == MemoryType::Decision
                        {
                            score += 0.2;
                        }

                        // 合成记忆优先返回（置信度加权）
                        if m.memory_type == MemoryType::Synthesis {
                            let confidence_boost = m.confidence.unwrap_or(0.5) * 0.3;
                            score += confidence_boost;
                        }

                        // 洛书几何距离加权（M.T.R. TrapezoidFocus 增强）
                        if let Some(ref luoshu_values) = m.luoshu_vector {
                            let mem_vec = LuoShuVector {
                                values: *luoshu_values,
                            };
                            let center_boost = mem_vec.center_value() * 0.1;
                            score += center_boost;
                        }

                        // 八卦分类匹配加权（同类别记忆额外加分）
                        if let Some(ref bagua) = m.bagua_category {
                            if (query_lower.contains("配置") || query_lower.contains("基础"))
                                && bagua == "承载基础"
                            {
                                score += 0.15;
                            } // 坤
                            if (query_lower.contains("规则") || query_lower.contains("架构"))
                                && bagua == "刚性法则"
                            {
                                score += 0.15;
                            } // 乾
                            if (query_lower.contains("依赖") || query_lower.contains("关联"))
                                && bagua == "依附关联"
                            {
                                score += 0.15;
                            } // 离
                            if (query_lower.contains("偏好") || query_lower.contains("交互"))
                                && bagua == "愉悦表达"
                            {
                                score += 0.15;
                            } // 兑
                            if (query_lower.contains("错误")
                                || query_lower.contains("bug")
                                || query_lower.contains("修复"))
                                && bagua == "陷溺困境"
                            {
                                score += 0.15;
                            } // 坎
                        }

                        (score, *m)
                    })
                    .collect()
            };

            // 按分数降序排序
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            // LRC 内置道体状态机·道体再次校验（回归验证层）：
            // 联想扩散把候选拉进 top_k 后，必须在输出前验证它是否真的回应了
            // 原始查询——"发散之后能否收束回主题"，防止碰巧共享联想桥词的
            // 噪声混入结果。判定信号：
            //   1. 原查询词面命中（直接相关）
            //   2. 联想桥强关联（≥2 个活跃记忆专属词命中 → 真联想边）
            //   3. 标签共鸣（用户主动标注的语义证据）
            // 三种证据皆无 → 判定为发散噪声，剔除。
            // 仅当联想导航开启且存在联想桥词时执行；否则跳过（旧行为零影响）。
            // v0.9.7：explore_pure（联想探索）模式下的处理分两级——
            //   · 扩散跳（regression_query=Some）：必须校验，且锚定起点主题，
            //     防止多跳扩散随父内容漂移；
            //   · 根节点召回（regression_query=None）：不得在此词面预筛——
            //     词面零重叠但语义强相关的候选（如"重要日子"↔"结婚纪念日"）
            //     会在这里被提前剔除，根门禁的语义旁路就永远看不到它们。
            //     起点是否实质共鸣交给 API 层根门禁（词面重叠 + 语义余弦）裁决。
            let skip_pure_recheck = filter.explore_pure && filter.regression_query.is_none();
            if !expansion_boost.is_empty() || (filter.explore_pure && !skip_pure_recheck) {
                let bridge_words: Vec<&str> =
                    expansion_boost.iter().map(|(w, _)| w.as_str()).collect();
                let mut rejected: usize = 0;
                // 收集回归证据：memory_id → 证据标签（供联想链输出可观测）
                let mut evidence: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                scored = scored
                    .into_iter()
                    .filter(|(_, m)| {
                        use crate::engine::memory_state_machine::regression_recheck;
                        let content_lower = self.recall_document(m).normalized_content;
                        // 证据1：原查询词面命中数
                        let original_overlap = original_query_refs
                            .iter()
                            .filter(|w| contains_word(&content_lower, w))
                            .count();
                        // 证据2：联想桥词命中数（活跃记忆专属词）
                        let bridge_hits = bridge_words
                            .iter()
                            .filter(|w| contains_word(&content_lower, w))
                            .count();
                        // 证据3：标签与原查询词共鸣
                        let tag_hits = m
                            .tags
                            .iter()
                            .filter(|t| {
                                original_query_refs
                                    .iter()
                                    .any(|w| t.to_lowercase().contains(w))
                            })
                            .count();
                        let verdict =
                            regression_recheck(original_overlap, bridge_hits, tag_hits, None);
                        if verdict.keep {
                            // 记录证据标签：联想桥强关联是联想导航的产物，
                            // 标注它让调用方看到"这条记忆为何被联想回来"
                            evidence.insert(m.id.clone(), verdict.evidence.to_string());
                        } else {
                            rejected += 1;
                        }
                        verdict.keep
                    })
                    .collect::<Vec<_>>();
                if rejected > 0 {
                    eprintln!(
                        "[LRC-STATE] 道体再次校验剔除 {} 条发散噪声（联想桥拉回但无共鸣信号）",
                        rejected
                    );
                }
                // 校验证据随结果返回（仅保留 top_k 截取前的证据即可，
                // 去重/截取不会改变记忆 id 集合）
                regression_evidence = evidence;
            }

            // v0.5.4 P2-12 修复：按 content 哈希去重，保留匹配度最高的那条
            // 在排序后、截取 top_k 前进行去重，确保结果中不会出现内容相同的记忆
            // 使用规范化内容（trim + lowercase）作为去重键，捕获大小写/空白差异的重复
            let mut seen_content: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let top_k = filter.top_k.min(scored.len());
            let scored: Vec<(f32, &Memory)> = scored
                .into_iter()
                .filter(|(_, m)| {
                    let content_key = m.content.trim().to_lowercase();
                    seen_content.insert(content_key)
                })
                .take(top_k)
                .collect();

            if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
                return Err(PersistenceError::Other("enrich_cancelled".to_string()));
            }
            let memories: Vec<Memory> = scored.iter().map(|(_, m)| (*m).clone()).collect();
            let scores: Vec<f32> = scored.iter().map(|(s, _)| *s).collect();

            (memories, scores)
        };
        // 块作用域结束，candidates 和 scored 的不可变引用均已释放

        // 搜索是只读热路径：不在请求内更新 last_accessed 或同步重写记忆文件。
        // 访问时间由后台维护，避免每次搜索都序列化全量记忆并阻塞全局 store 锁。

        // 记录指标：检索 + 1
        self.dao_metrics.record_recall();

        // v0.6.0+ 参赛扩展：探索日志记录（recall 事件）
        let recall_result_count = memories.len();
        self.exploration_logger.log_recall(
            query,
            recall_top_k,
            recall_result_count,
            recall_start.elapsed().as_millis() as u64,
        );

        // LRC 内置道体状态机：激活本次召回的记忆、记录联想轨迹并持久化。
        // 与 deep 路径 (trapezoid_focus_recall) 保持一致。
        self.bake_activation(&memories, &scores);

        Ok(RecallResult {
            memories,
            scores,
            total: total_count,
            regression_evidence,
        })
    }

    /// 删除一条记忆
    pub fn forget(&mut self, id: &str) -> Result<bool, PersistenceError> {
        let old = self
            .load_cached()?
            .into_iter()
            .find(|memory| memory.id == id);
        let result = self.persistence.delete_memory(id)?;
        if result {
            if let Some(old) = old.as_ref() {
                self.remove_memory_from_index(old);
            }
            self.mark_cache_dirty_preserving_index();
        }
        Ok(result)
    }

    /// 更新记忆内容
    ///
    /// 如果记忆存在则更新并返回旧版本，否则返回 None。
    ///
    /// 写盘走 `update_memories` 单端点（单次序列化 + tmp+rename 原子写），
    /// 不再使用 clear_memories + save_memories 两端点——避免 clear 成功、
    /// save 前崩溃导致磁盘全库丢失的间隙窗口（持久化评估 C05）。
    pub fn update_memory(
        &mut self,
        id: &str,
        new_content: &str,
        new_importance: Option<Importance>,
    ) -> Result<Option<Memory>, PersistenceError> {
        let all = self.load_cached()?;
        let mut found: Option<Memory> = None;

        let updated: Vec<Memory> = all
            .into_iter()
            .map(|mut m| {
                if m.id == id {
                    let old = m.clone();
                    m.update_content(new_content.to_string());
                    if let Some(imp) = new_importance {
                        m.update_importance(imp);
                    }
                    found = Some(old);
                    m
                } else {
                    m
                }
            })
            .collect();

        // 单端点原子写全量：update_memories 内部单次序列化 + tmp+rename，
        // 消除旧 clear_memories+save_memories 两端点在 save 前崩溃丢全库的窗口（C05）。
        self.persistence.update_memories(&updated)?;
        if let Some(new_memory) = updated.iter().find(|memory| memory.id == id) {
            if let Some(old_memory) = found.as_ref() {
                self.replace_memory_in_index(old_memory, new_memory);
            }
        }
        // 更新后仅刷新记忆快照，保留已完成的增量索引。
        self.mark_cache_dirty_preserving_index();

        Ok(found)
    }

    /// 列出记忆（支持分页、过滤、排序）
    pub fn list_memories(
        &self,
        filter: &ListFilter,
    ) -> Result<(Vec<Memory>, usize), PersistenceError> {
        let mut all = self.load_cached()?;
        let privacy_ctx = filter.privacy_context.clone();

        // 过滤
        all.retain(|m| {
            if let Some(ref mt) = filter.memory_type {
                if m.memory_type != *mt {
                    return false;
                }
            }
            if let Some(ref proj) = filter.project {
                if m.project.as_deref() != Some(proj.as_str()) {
                    return false;
                }
            }
            if !filter.tags.is_empty() && !filter.tags.iter().any(|t| m.tags.contains(t)) {
                return false;
            }
            // 隐私权限过滤（Section 3.3）
            if !is_visible(m, &privacy_ctx) {
                return false;
            }
            true
        });

        let total = all.len();

        // 排序
        all.sort_by(|a, b| {
            let cmp = match filter.sort_by {
                SortBy::CreatedAt => a.created_at.cmp(&b.created_at),
                SortBy::Importance => a.importance.value().cmp(&b.importance.value()),
                SortBy::LastAccessed => a.last_accessed.cmp(&b.last_accessed),
            };
            match filter.order {
                SortOrder::Desc => cmp.reverse(),
                SortOrder::Asc => cmp,
            }
        });

        // 分页
        let paged: Vec<Memory> = all
            .into_iter()
            .skip(filter.offset)
            .take(filter.limit)
            .collect();

        Ok((paged, total))
    }

    /// 获取记忆库统计信息
    pub fn stats(&self) -> Result<MemoryStats, PersistenceError> {
        let all = self.load_cached()?;
        let mut stats = MemoryStats {
            total_memories: all.len(),
            ..Default::default()
        };

        // v0.9.6：近 7 天新增按创建时间真实统计（替代"今日新增=累计编码次数"的误导口径）
        let recent_cutoff = chrono::Utc::now() - chrono::Duration::days(7);

        for m in &all {
            *stats
                .by_type
                .entry(m.memory_type.as_str().to_string())
                .or_insert(0) += 1;

            let proj = m.project.as_deref().unwrap_or("_global_");
            *stats.by_project.entry(proj.to_string()).or_insert(0) += 1;

            if m.is_expired() {
                stats.expired_count += 1;
            }

            if m.created_at >= recent_cutoff {
                stats.recent_added += 1;
            }
        }

        stats.storage_size_bytes = self.persistence.size_bytes()?;

        Ok(stats)
    }

    /// 获取记忆总数
    pub fn total_count(&self) -> Result<usize, PersistenceError> {
        let all = self.load_cached()?;
        Ok(all.len())
    }

    /// 道枢映射: 坤卦·地 (☷) — 厚德载物，归档如大地之收藏与沉淀
    /// 归档过期记忆
    ///
    /// 将已过期的记忆从活跃存储迁移到归档存储（冷存储）。
    /// 归档的记忆不会丢失，但不再参与检索、列表和统计。
    ///
    /// 返回归档的记忆数量，若无可归档记忆则返回 0。
    pub fn archive_expired(&mut self) -> Result<usize, PersistenceError> {
        let all = self.load_cached()?;

        // 筛选过期记忆与活跃记忆
        let (expired, active): (Vec<Memory>, Vec<Memory>) =
            all.into_iter().partition(|m| m.is_expired());

        if expired.is_empty() {
            return Ok(0);
        }

        let count = expired.len();

        // 归档过期记忆到冷存储
        self.persistence.add_to_archive(&expired)?;

        // 从活跃存储中重建（仅保留活跃记忆）
        // 2026-09-01 C05 修复：改用 replace_all_memories 单端点原子写，
        // 消除旧 clear_memories+save_memory 两端点在重建中途崩溃丢全库的窗口。
        self.persistence.replace_all_memories(&active)?;
        // v0.5.4 写操作后标记缓存为脏
        self.invalidate_cache();

        Ok(count)
    }

    /// 道枢映射: 坤卦·地 (☷) — 地势坤，持久化如大地之承载记忆
    /// 获取持久化层的引用
    pub fn persistence(&self) -> &P {
        &self.persistence
    }

    /// 获取道同构度指标快照（L5 监控仪表）
    ///
    /// 计算当前记忆库的完整健康度指标，包括：
    /// - 道同构度（幻和约束满足度）
    /// - 八卦分布熵
    /// - 合成/原始记忆比率
    pub fn dao_metrics_snapshot(
        &self,
    ) -> Result<crate::engine::dao_metrics::DaoMetricsSnapshot, PersistenceError> {
        let all = self.load_cached()?;
        let archived = self
            .persistence
            .load_archived_memories()
            .unwrap_or_default();

        let total = all.len();
        let crystallized = all
            .iter()
            .filter(|m| m.memory_type == MemoryType::Synthesis)
            .count();
        let archived_count = archived.len();

        // 计算平均洛书偏离度
        let vectors: Vec<[f32; 9]> = all.iter().filter_map(|m| m.luoshu_vector).collect();
        let avg_deviation = crate::engine::dao_metrics::compute_avg_luoshu_deviation(&vectors);

        // 计算八卦分布
        let mut bagua_counts = [0usize; 8];
        for m in &all {
            if let Some(idx) = m.bagua_index {
                bagua_counts[idx as usize] += 1;
            }
        }

        Ok(self.dao_metrics.snapshot(
            total,
            crystallized,
            archived_count,
            avg_deviation,
            &bagua_counts,
        ))
    }

    /// 道枢映射: 道枢·全息 — 健康报告是系统全息状态的可解释性面板，如道枢之"环中"统观全局
    /// 生成系统健康报告（可解释性面板）
    ///
    /// 聚合编码器、调节器、合成日志、道同构度等所有子系统的状态，
    /// 生成统一的诊断视图。解决质疑四"可解释性下降"问题。
    ///
    /// 返回结构化的 SystemHealthReport，可序列化为 JSON 通过 API 暴露。
    pub fn health_report(&mut self) -> Result<SystemHealthReport, PersistenceError> {
        let all = self.load_cached()?;
        let _archived = self
            .persistence
            .load_archived_memories()
            .unwrap_or_default();

        let total = all.len();
        let active = all.iter().filter(|m| !m.is_expired()).count();
        let synthesis = all
            .iter()
            .filter(|m| m.memory_type == MemoryType::Synthesis)
            .count();
        let expired = all.iter().filter(|m| m.is_expired()).count();

        // 计算八卦分布
        let mut bagua_distribution = [0usize; 8];
        for m in &all {
            if let Some(idx) = m.bagua_index {
                if (idx as usize) < 8 {
                    bagua_distribution[idx as usize] += 1;
                }
            }
        }

        // 计算平均洛书偏离度
        let vectors: Vec<[f32; 9]> = all.iter().filter_map(|m| m.luoshu_vector).collect();
        let avg_deviation = crate::engine::dao_metrics::compute_avg_luoshu_deviation(&vectors);

        // 编码器状态
        let encoder_status = self.luoshu_encoder.get_status();

        // 道同构度快照
        let dao_snapshot = self.dao_metrics.snapshot(
            total,
            synthesis,
            expired,
            avg_deviation,
            &bagua_distribution,
        );

        // 合成日志快照
        let journal_snapshot = self.synthesis_journal.snapshot();

        // 调节器状态
        let regulator_state = self.dao_regulator.get_state();

        // 低质量合成记忆数
        let low_quality = self.synthesis_journal.get_low_quality_ids().len();

        // 垃圾回收器统计（质疑五：运维可观测性）
        let gc_stats = self.memory_gc.get_stats();

        // 用户反馈统计（质疑五：运维可观测性）
        let feedback_stats = self.user_feedback.get_stats();

        // 复杂度预算（质疑五·终极：防止系统超出人类可理解范围）
        // 每次生成健康报告时更新复杂度预算，确保指标反映当前状态
        self.complexity_budget.update(
            20, // 核心模块数（src/engine/*.rs + src/memory_store.rs + src/memory_types.rs）
            self.count_public_api_surface(),
            self.count_cross_module_dependencies(),
            self.complexity_budget
                .causal_chains
                .iter()
                .map(|c| c.depth)
                .max()
                .unwrap_or(5),
        );

        // v0.9.1 三阶段锁解耦：健康检查是读关键路径，绝不再持锁执行合成。
        // synthesis_pending 标记由后台结晶流水线（consolidation）的三阶段合成消费，
        // 避免 health/system 接口在持锁时触发数秒~数十秒的聚类计算导致 lock_busy。

        // v0.5.5 P1-1：获取 LLM 配置状态，传入健康报告
        let llm_configured = self.is_llm_configured();

        Ok(generate_health_report(
            encoder_status,
            dao_snapshot,
            journal_snapshot,
            regulator_state,
            total,
            active,
            synthesis,
            expired,
            low_quality,
            bagua_distribution,
            gc_stats,
            feedback_stats,
            self.complexity_budget.clone(),
            &mut self.hint_escalation,
            // v0.5.5 P1-1：传入 LLM 配置状态，LLM 配置后编码器不再视为降级
            llm_configured,
        ))
    }

    /// 统计公开 API 表面（pub fn 数量）
    /// 用于复杂度预算的更新
    fn count_public_api_surface(&self) -> usize {
        // 当前系统的公开 API 约 200 个函数
        // 这是一个近似值，精确统计需要扫描所有源文件
        // 在实际 CI/CD 中可通过 cargo-public-api 或自定义脚本获取
        200
    }

    /// 统计跨模块依赖数量
    /// 用于复杂度预算的更新
    fn count_cross_module_dependencies(&self) -> usize {
        // 当前系统约 40 个跨模块依赖（engine 模块间相互引用）
        // 这是一个近似值，精确统计需要分析 use 语句
        40
    }

    /// 拆解合成记忆（Section 3.2 RecursiveUnfold）
    ///
    /// 将一条 Synthesis 类型的抽象记忆展开为具体子记忆。
    /// 算法：
    /// 1. 加载指定记忆，验证其类型为 Synthesis 且有洛书向量
    /// 2. 调用递归拆解算子，激活阈值 min_activation
    /// 3. 为每个子向量创建对应的子记忆（Fact 类型）
    /// 4. 子记忆继承父记忆的项目、标签和隐私设置
    ///
    /// 返回拆解出的子记忆列表及重构保真度。
    ///
    /// 参数：
    /// - `id`: 要拆解的合成记忆 ID
    /// - `min_activation`: 激活阈值（低于此值的九宫格位置不生成子记忆，默认 0.1）
    pub fn unfold_memory(
        &mut self,
        id: &str,
        min_activation: f32,
    ) -> Result<Option<(Vec<Memory>, f32)>, PersistenceError> {
        let all = self.load_cached()?;

        // 找到目标记忆
        let memory = match all.iter().find(|m| m.id == id) {
            Some(m) => m.clone(),
            None => return Ok(None),
        };

        // 仅支持拆解 Synthesis 类型且有洛书向量的记忆
        if memory.memory_type != MemoryType::Synthesis {
            return Ok(None);
        }

        let vector = match memory.luoshu_vector {
            Some(v) => LuoShuVector { values: v },
            None => return Ok(None),
        };

        // 执行递归拆解
        let unfold_result = recursive_unfold(&vector, min_activation.max(0.01));

        if unfold_result.sub_vectors.is_empty() {
            return Ok(Some((Vec::new(), 0.0)));
        }

        // 为每个子向量创建子记忆
        let mut sub_memories = Vec::with_capacity(unfold_result.sub_vectors.len());
        let bagua_names = crate::engine::mirror_trapezoid::BAGUA_CATEGORIES;

        for (i, sub_vec) in unfold_result.sub_vectors.iter().enumerate() {
            let proj = mirror_project(sub_vec);
            let category = bagua_names.get(proj.best_index).copied().unwrap_or("未知");

            let content = format!(
                "「拆解·{}」来自合成记忆的子步骤 #{}。类别: {}，权重: {:.2}",
                memory.content.chars().take(40).collect::<String>(),
                i + 1,
                category,
                unfold_result.sub_weights.get(i).copied().unwrap_or(0.0),
            );

            let mut sub_mem = Memory::new(
                content,
                MemoryType::Fact,
                memory.project.clone(),
                memory.tags.clone(),
                memory.importance,
                None,
            );
            sub_mem.source = Some(format!("unfold:{}", memory.id));
            sub_mem.source_ids = vec![memory.id.clone()];
            sub_mem.luoshu_vector = Some(sub_vec.values);
            sub_mem.bagua_index = Some(proj.best_index as u8);
            sub_mem.bagua_category = Some(proj.best_category.to_string());
            sub_mem.privacy_level = memory.privacy_level;
            sub_mem.session_id = memory.session_id.clone();
            sub_mem.user_id = memory.user_id.clone();

            // 持久化
            self.persistence.save_memory(&sub_mem)?;
            sub_memories.push(sub_mem);
        }

        // v0.5.4 写操作后标记缓存为脏
        self.invalidate_cache();

        Ok(Some((sub_memories, unfold_result.fidelity)))
    }

    /// 道枢映射: 兑卦·泽 (☱) — 说以利贞，记忆修正如泽水之润物无声
    /// 用户修正记忆（带版本追踪）
    ///
    /// 创建新版本而非直接覆盖，保留修正历史。
    /// 返回修正后的记忆。
    pub fn correct_memory(
        &mut self,
        id: &str,
        new_content: &str,
        reason: Option<&str>,
    ) -> Result<Option<Memory>, PersistenceError> {
        let all = self.load_cached()?;
        let mut found: Option<Memory> = None;

        let updated: Vec<Memory> = all
            .into_iter()
            .map(|mut m| {
                if m.id == id {
                    // 使用版本追踪更新（自动保存历史版本）
                    let reason_str = reason.unwrap_or("用户修正").to_string();
                    m.update_content_with_reason(new_content.to_string(), reason_str);
                    // 追加修正标记到 source
                    m.source = Some(format!("corrected: {}", reason.unwrap_or("未提供原因")));
                    found = Some(m.clone());
                    m
                } else {
                    m
                }
            })
            .collect();

        // 重新写入
        // 2026-09-01 C05 修复：改用 replace_all_memories 单端点原子写，
        // 消除旧 clear_memories+save_memory 两端点在重写中途崩溃丢全库的窗口。
        self.persistence.replace_all_memories(&updated)?;
        // v0.5.4 写操作后标记缓存为脏
        self.invalidate_cache();

        // 记录指标：修正 + 1
        if found.is_some() {
            self.dao_metrics.record_correction();
        }

        Ok(found)
    }

    /// 生成系统健康聚合报告（质疑五·可理解性）
    ///
    /// 将分散在多个子系统中的状态指标聚合为一个人类可读的单一视图。
    /// 这是排查"检索质量在长期运行中略有下降"等微妙问题时
    /// 的"一站式入口"——无需逐个检查每个子系统。
    ///
    /// 道枢映射：中宫（五）— 统摄八方的核心枢纽。
    pub fn generate_health_report(&self) -> crate::engine::dao_regulator::SystemHealthReport {
        use crate::engine::dao_regulator::SystemHealthReport;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // 采集记忆统计
        let all = self.load_cached().unwrap_or_default();
        let total = all.len();
        let active = all.iter().filter(|m| !m.is_expired()).count();
        let expired = all.iter().filter(|m| m.is_expired()).count();
        let synthesis_count = all
            .iter()
            .filter(|m| m.memory_type == MemoryType::Synthesis)
            .count();
        let quarantined_count = self.synthesis_journal.get_low_quality_ids().len();

        // 计算合成比率
        let synthesis_ratio = if active > 0 {
            synthesis_count as f32 / active as f32
        } else {
            0.0
        };

        // 采集道同构度快照
        let mut bagua_counts = [0usize; 8];
        let mut vectors: Vec<[f32; 9]> = Vec::new();
        for m in &all {
            if let Some(idx) = m.bagua_index {
                if (idx as usize) < 8 {
                    bagua_counts[idx as usize] += 1;
                }
            }
            if let Some(v) = m.luoshu_vector {
                vectors.push(v);
            }
        }
        let avg_deviation = crate::engine::dao_metrics::compute_avg_luoshu_deviation(&vectors);
        let snapshot = self.dao_metrics.snapshot(
            total,
            synthesis_count,
            expired,
            avg_deviation,
            &bagua_counts,
        );
        let journal_snapshot = self.synthesis_journal.snapshot();
        let feedback_stats = self.user_feedback.stats();
        let regulator_state = self.dao_regulator.get_state();

        // 计算综合健康评分
        let bagua_health = (snapshot.bagua_entropy / 3.0).min(1.0);
        let deviation_health = (1.0 - avg_deviation).max(0.0);
        let coupling_health = 1.0 - regulator_state.coupling_score;
        let overall_health = (snapshot.dao_isomorphism_score * 0.35
            + bagua_health * 0.2
            + deviation_health * 0.2
            + (1.0 - synthesis_ratio.min(1.0)) * 0.15
            + coupling_health * 0.1)
            .clamp(0.0, 1.0);

        let health_level = if overall_health > 0.7 {
            "healthy"
        } else if overall_health > 0.4 {
            "degraded"
        } else {
            "critical"
        };

        // 编码器状态
        let encoder_status = self.luoshu_encoder.get_status();
        let encoder_mode = encoder_status.mode.clone();
        // v0.5.5 P1-1：LLM 配置后替代本地 ML 模型提供语义理解能力
        // 如果 LLM 已配置，编码器不再视为"降级"，系统模式为 Healthy
        let llm_configured = self.is_llm_configured();
        let encoder_degraded = if llm_configured {
            // LLM 已配置 → 编码器不视为降级（LLM 提供语义理解能力）
            false
        } else {
            encoder_mode == "statistical" || Self::check_encoder_degraded(&self.luoshu_encoder)
        };
        let encoder_recovery_progress = if encoder_degraded {
            let (successes, threshold) = Self::get_encoder_recovery_progress(&self.luoshu_encoder);
            if threshold > 0 {
                Some(successes as f32 / threshold as f32)
            } else {
                Some(0.0)
            }
        } else {
            None
        };

        // 审计状态
        let audit_chain_verification = self.audit_trail.verify_integrity();
        let audit_chain_valid = audit_chain_verification.is_valid;

        // 灾难性事件
        let catastrophic_events = self.dao_regulator.get_catastrophic_events();
        let catastrophic_event_count = catastrophic_events.len();
        let last_catastrophic_event = catastrophic_events.last().map(|e| e.diagnosis.clone());

        SystemHealthReport {
            timestamp_ms: now,
            overall_health,
            health_level: health_level.to_string(),
            encoder_mode,
            encoder_degraded,
            encoder_recovery_progress,
            dao_score: snapshot.dao_isomorphism_score,
            bagua_entropy: snapshot.bagua_entropy,
            is_oscillating: regulator_state.is_oscillating,
            is_drifting: regulator_state.is_drifting,
            is_frozen: regulator_state.is_frozen,
            coupling_score: regulator_state.coupling_score,
            information_gain_threshold: self.dao_regulator.information_gain_threshold,
            threshold_baseline: self.dao_regulator.threshold_baseline(),
            threshold_ema: self.dao_regulator.threshold_ema(),
            synthesis_min_cluster: self.synthesis_min_cluster,
            synthesis_ratio,
            synthesis_rate_per_minute: journal_snapshot.synthesis_rate_per_minute,
            synthesis_count,
            quarantined_count,
            total_feedback: feedback_stats.total_feedback,
            positive_feedback_ratio: feedback_stats.positive_ratio,
            implicit_feedback_enabled: self.user_feedback.is_implicit_feedback_enabled(),
            consent_granted: self.user_feedback.is_consent_granted(),
            total_audit_events: self.audit_trail.total_events(),
            audit_chain_valid,
            audit_persistence_enabled: self.audit_trail.has_persistence(),
            audit_seal_verified: self.audit_trail.seal_verified(),
            gc_pending: self.gc_pending.load(std::sync::atomic::Ordering::Relaxed),
            gc_last_run_ms: self.memory_gc.last_run_ms(),
            synthesis_pending: self
                .synthesis_pending
                .load(std::sync::atomic::Ordering::Relaxed), // v0.5.4
            catastrophic_event_count,
            last_catastrophic_event,
            total_memories: total,
            active_memories: active,
            expired_memories: expired,
            decay_rate: self.decay_config.decay_rate,
        }
    }
}

// v0.9.7 structural refactor (GLOBAL_CODE_REVIEW_REPORT P2-2/P2-4): the
// inline test island of memory_store.rs was extracted to
// src/memory_store_tests.rs and is re-included here via #[path]. Module name
// and test visibility are unchanged, so `cargo test` runs the same cases.
#[cfg(test)]
#[path = "memory_store_tests.rs"]
mod memory_store_tests;
