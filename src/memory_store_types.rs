//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现记忆存储层的数据类型，属于公开层 (Layer 1)。
//! ============================================================
//!
//! 记忆存储数据类型（MemoryStore 的纯数据契约）
//!
//! v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P2-2「MemoryStore God Object」）：
//!   `memory_store.rs` 曾承载 27 字段 / 18 pub 方法 / 约 3600 行主体 impl。
//!   本模块以**零行为变更**的切片先行外提其中的**纯数据类型**——
//!   它们不持有 `MemoryStore` 状态、不访问持久层，仅描述输入/输出契约，
//!   因此可独立演进（新增查询条件/统计字段不再触碰存储主体）。
//!
//! 兼容性：本模块全部类型在 `memory_store` 中以 `pub use` 重导出，
//!   既有 `crate::memory_store::Xxx` 路径与 `use crate::memory_store::*`
//!   调用方均不受影响。

use crate::engine::dao_regulator::RegulationAction;
use crate::engine::synthesis_engine::SynthesisConfig;
use crate::memory_types::{Importance, Memory, MemoryType, PrivacyLevel};
use serde::{Deserialize, Serialize};

/// 记忆召回过滤条件
#[derive(Debug, Clone, Default)]
pub struct RecallFilter {
    /// 按记忆类型过滤
    pub memory_type: Option<MemoryType>,
    /// 按项目过滤
    pub project: Option<String>,
    /// 按标签过滤
    pub tags: Vec<String>,
    /// 最低重要性阈值
    pub min_importance: Option<Importance>,
    /// 最大返回数
    pub top_k: usize,
    /// 隐私上下文：按隐私级别过滤（Session/User/Global）
    /// 传入 (PrivacyLevel, session_id, user_id) 三元组
    pub privacy_context: Option<(PrivacyLevel, Option<String>, Option<String>)>,
    /// 纯净查询模式（联想探索专用，v0.9.7 精确度修复）：
    /// 跳过联想导航的查询扩展与活性偏置加分——用户主动发起探索时，
    /// 语义必须完全由查询本身主导，不得被"近期活跃记忆"（可能全是
    /// 某个领域的旧内容）牵引。回归校验退化为纯原查询词面/标签共鸣。
    pub explore_pure: bool,
    /// 回归校验锚定查询（联想探索专用，v0.9.7 精确度修复）：
    /// 设置后，道体再次校验以此查询（而非当前 recall 查询）判定候选
    /// 是否与主题相关。探索的多跳 BFS 以父记忆内容为查询逐层扩散，
    /// 若每跳独立校验，语义会随深度漂移（父内容 → 无关领域噪声）。
    /// 锚定到起点记忆主题可保证每一跳结果都必须"收束回联想主题"。
    pub regression_query: Option<String>,
}

impl RecallFilter {
    /// 创建默认过滤条件
    pub fn new() -> Self {
        Self {
            memory_type: None,
            project: None,
            tags: Vec::new(),
            min_importance: None,
            top_k: 5,
            privacy_context: None,
            explore_pure: false,
            regression_query: None,
        }
    }

    /// 设置返回数量
    pub fn with_top_k(mut self, k: usize) -> Self {
        self.top_k = k;
        self
    }

    /// 设置类型过滤
    pub fn with_type(mut self, t: MemoryType) -> Self {
        self.memory_type = Some(t);
        self
    }

    /// 设置项目过滤
    pub fn with_project(mut self, p: impl Into<String>) -> Self {
        self.project = Some(p.into());
        self
    }

    /// 设置隐私上下文过滤
    pub fn with_privacy(
        mut self,
        level: PrivacyLevel,
        session_id: Option<String>,
        user_id: Option<String>,
    ) -> Self {
        self.privacy_context = Some((level, session_id, user_id));
        self
    }
}

/// 记忆列表查询过滤条件
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    /// 按记忆类型过滤
    pub memory_type: Option<MemoryType>,
    /// 按项目过滤
    pub project: Option<String>,
    /// 按标签过滤
    pub tags: Vec<String>,
    /// 排序方式
    pub sort_by: SortBy,
    /// 排序方向
    pub order: SortOrder,
    /// 分页大小
    pub limit: usize,
    /// 分页偏移
    pub offset: usize,
    /// 隐私上下文：按隐私级别过滤
    pub privacy_context: Option<(PrivacyLevel, Option<String>, Option<String>)>,
}

impl ListFilter {
    pub fn new() -> Self {
        Self {
            memory_type: None,
            project: None,
            tags: Vec::new(),
            sort_by: SortBy::CreatedAt,
            order: SortOrder::Desc,
            limit: 20,
            offset: 0,
            privacy_context: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SortBy {
    #[default]
    CreatedAt,
    Importance,
    LastAccessed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SortOrder {
    #[default]
    Desc,
    Asc,
}

/// 记忆库统计信息
#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryStats {
    /// 记忆总数
    pub total_memories: usize,
    /// 按类型分布
    pub by_type: std::collections::HashMap<String, usize>,
    /// 按项目分布
    pub by_project: std::collections::HashMap<String, usize>,
    /// 过期记忆数
    pub expired_count: usize,
    /// 近 7 天新增记忆数（v0.9.6：仪表盘"最近新增"真实时间口径，替代累计编码次数近似）
    pub recent_added: usize,
    /// 存储文件大小（字节）
    pub storage_size_bytes: u64,
}

/// 召回结果
#[derive(Debug, Clone)]
pub struct RecallResult {
    /// 匹配的记忆列表
    pub memories: Vec<Memory>,
    /// 每条记忆的匹配分数（与 memories 一一对应）
    pub scores: Vec<f32>,
    /// 记忆库总数
    pub total: usize,
    /// 道体再次校验·回归证据标签（memory_id → 证据，如"联想桥强关联"）。
    /// 联想导航未开启或未执行校验时为空，旧行为零影响。
    pub regression_evidence: std::collections::HashMap<String, String>,
}

impl RecallResult {
    /// 构造无回归证据的结果（兼容旧调用点）
    pub fn basic(memories: Vec<Memory>, scores: Vec<f32>, total: usize) -> Self {
        Self {
            memories,
            scores,
            total,
            regression_evidence: std::collections::HashMap::new(),
        }
    }
}

/// 合成快照（三阶段锁解耦·Phase 1 读）
///
/// 在持锁下快速读取全量记忆 + 合成配置，随后在锁外执行 CPU 密集的聚类计算。
#[derive(Debug)]
pub struct SynthesisSnapshot {
    /// 全量记忆快照（供纯计算使用）
    pub all: Vec<Memory>,
    /// 合成引擎配置
    pub config: SynthesisConfig,
    /// 信息增量阈值（由 DaoRegulator 动态管理）
    pub information_gain_threshold: f32,
}

/// 调节器心跳状态（P1：sidecar 后台周期任务调用 regulate() 的可观测状态）
///
/// 记录最近一次调节动作的类型、时间与执行计数，
/// 供 `/v1/health/system` 与前端系统状态卡展示"调节器心跳"。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegulatorHeartbeat {
    /// 最近一次调节执行的时间戳（毫秒，UNIX 纪元）
    pub last_run_ms: u64,
    /// 最近一次调节动作（如 "adjust_decay_rate" / "no_action"）
    pub last_action: String,
    /// 累计执行次数
    pub run_count: u64,
    /// 最近一次调节的原因摘要（NoAction 时为 None）
    pub last_reason: Option<String>,
}

impl Default for RegulatorHeartbeat {
    fn default() -> Self {
        Self {
            last_run_ms: 0,
            last_action: "never".to_string(),
            run_count: 0,
            last_reason: None,
        }
    }
}

impl RegulatorHeartbeat {
    /// 记录一次调节心跳。
    ///
    /// action 为 None 表示调节器判定"无需调节"（NoAction）或未到间隔；
    /// 有动作时记录动作类型与原因，便于前端展示"上次调节做了什么"。
    pub fn record(&mut self, action: Option<&RegulationAction>) {
        self.last_run_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.run_count = self.run_count.saturating_add(1);
        match action {
            Some(RegulationAction::AdjustDecayRate { new_rate, reason }) => {
                self.last_action = format!("adjust_decay_rate → {new_rate:.2}");
                self.last_reason = Some(reason.clone());
            }
            Some(RegulationAction::AdjustSynthesisThreshold {
                new_min_cluster,
                reason,
                ..
            }) => {
                self.last_action = format!("adjust_synthesis_threshold → {new_min_cluster}");
                self.last_reason = Some(reason.clone());
            }
            Some(RegulationAction::SuggestReencoding { reason, .. }) => {
                self.last_action = "suggest_reencoding".to_string();
                self.last_reason = Some(reason.clone());
            }
            Some(RegulationAction::AdjustRetrievalWeights { reason, .. }) => {
                self.last_action = "adjust_retrieval_weights".to_string();
                self.last_reason = Some(reason.clone());
            }
            Some(RegulationAction::AdjustInformationGainThreshold {
                new_threshold,
                reason,
            }) => {
                self.last_action =
                    format!("adjust_information_gain_threshold → {new_threshold:.4}");
                self.last_reason = Some(reason.clone());
            }
            Some(RegulationAction::SuggestComprehensiveRebalance {
                anomaly_description,
                ..
            }) => {
                self.last_action = "suggest_comprehensive_rebalance".to_string();
                self.last_reason = Some(anomaly_description.clone());
            }
            None | Some(RegulationAction::NoAction) => {
                self.last_action = "no_action".to_string();
                self.last_reason = None;
            }
        }
    }
}
