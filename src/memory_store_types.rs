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

/// 由检索结果**联想补全**出的一条记忆（v0.9.8）
///
/// 与 [`MemoryAssociation`] 的区别：后者描述"某条记忆的关联"（详情页用），
/// 本类型描述"**本次检索之外、但由记录层必然关联**的记忆"——
/// 它带 `via_*` 字段说明"从哪条已召回的记忆联想过来"，让"为什么它会出现"
/// 可追溯到具体起点。
///
/// **典型形态**（用户原始设想）：用户查「游西湖」，
/// 结果里补出同一次杭州之行的「吃楼外楼」——两句话义毫不相似，
/// 靠 `event_id`（共同经历）连上，这是任何相似度算法都给不出的。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociatedMemory {
    /// 被联想出来的记忆 ID（**不在本次召回结果中**）
    pub memory_id: String,
    /// 内容预览
    pub content_preview: String,
    /// 记忆类型字符串（fact / experience / decision ...）
    pub memory_type: String,
    /// 关联类型：same_event | shared_entity | derived_from | indirect
    pub relation: String,
    /// 人类可读的关联依据（如 "同一次经历（event_id=trip-hangzhou-2026-09）"）
    pub why: String,
    /// **从哪条已召回记忆**联想过来（起点，可追溯）
    pub via_memory_id: String,
    /// 起点记忆的内容预览
    pub via_preview: String,
    /// 距联想起点的**跳数**：1 = 直接记录关联；2 = 经中间记忆的**间接关联**
    ///
    /// **为什么要它**（2026-09-17）：用户对联想的价值判据是
    /// 「告诉我这是第几层能想到的」——层次是**寻路**的坐标，
    /// 而非分类的标签。1 跳是"记录直接成立"，2 跳是"图上的路径合成"，
    /// 二者证据强度不同，必须可区分。
    #[serde(default = "default_hops_one")]
    pub hops: usize,
    /// 完整路径（记忆 ID 序列）
    ///
    /// - 1 跳：`[起点, 此记忆]`
    /// - 2 跳：`[起点, 中间记忆, 此记忆]`
    ///
    /// **为什么要有它**：2 跳的"意外性"来自**路径本身**——
    /// 用户看到 `游西湖 → 同一项目 → 某次会议记录` 才能判断这个跳跃是否合理。
    /// 只给终点而不给路径，间接关联就变成了无解释的推测。
    #[serde(default)]
    pub path: Vec<String>,
}

/// 兼容旧序列化数据的 `hops` 默认值（1 = 直接关联）
fn default_hops_one() -> usize {
    1
}

/// 记忆间的结构化关联（联想的结果形态）
///
/// 与"排序后的检索结果"不同：每条关联都带**类型**与**依据说明**，
/// 使"为什么关联"可被人类检验（对应"人类可解释"判据）。
/// 不同类型的关联**并存**，而非被压成单一相似度分数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryAssociation {
    /// 关联到的目标记忆 ID
    pub memory_id: String,
    /// 关系类型：same_event | same_event_auto | shared_entity | derived_from | crystallized_into | evolved_from
    pub relation: String,
    /// 关联依据（人类可读，如 "同一次经历（event_id=e1）"）
    pub why: String,
    /// 目标记忆内容预览
    pub content_preview: String,
}

/// 关联图中的一个节点（一条记忆）
///
/// 与 [`MemoryAssociation`] 的区别：后者只描述"边"，
/// 本类型让图**自洽**——前端拿到 nodes + edges 即可独立渲染，
/// 无需再逐条查询节点内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub memory_id: String,
    pub content_preview: String,
    /// 记忆类型字符串（fact / experience / decision ...）
    pub memory_type: String,
    /// 该记忆所属的事件 ID（若有）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// 实体描述（如 `person:小美`），供前端展示"为什么连到这里"
    pub entities: Vec<String>,
}

/// 关联图中的一条边（有类型、有方向、有解释）
///
/// **方向语义**：`from` → `to` 是路径书写顺序（从根节点向外）。
/// 但**并非所有关系都有方向**——`same_event` / `shared_entity` 是**对称关系**
/// （"A 与 B 同一次经历"等价于"B 与 A 同一次经历"），此时 `symmetric = true`，
/// 方向仅为书写顺序，**不表示因果或先后**。
/// 只有 `derived_from` 等**非对称**关系，方向才有实质含义。
/// 前端据此决定是否显示箭头——避免把对称关系误画成有向因果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    /// 关系类型：same_event | same_event_auto | shared_entity |
    /// derived_from | crystallized_into | evolved_from | indirect
    pub relation: String,
    /// 人类可读依据（间接边会写出完整路径）
    pub why: String,
    /// 跳数：1 = 由记录直接推导；2 = 经中间记忆间接推导
    pub hops: usize,
    /// 是否为对称关系（true 时方向无语义，仅表示书写顺序）
    pub symmetric: bool,
    /// 完整路径（节点 ID 序列）：直接边为 [from, to]；间接边为 [root, mid, to]
    pub path: Vec<String>,
    /// 间接边的路径类型（如 `same_event → shared_entity`），直接边为 None
    ///
    /// **为什么需要它**：2 跳路径由两段关系合成，而**两段的证据强度不同**
    /// （PREREG §3.48.4 实测：`same_event → same_event` 因传递性恒为 0，
    /// 故实际产出**全部**含 `shared_entity` 段）。
    /// 前端/调用方据此可区分"共同经历传递"与"经由同一实体桥接"——
    /// 二者含义不同，不应混为一谈。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

/// 关联图 —— 联想的**结构化**形态（非排序列表）
///
/// 这是"记录层推导 + 多跳传递推理"的结果：节点是记忆，边是**有类型、有方向、有解释**
/// 的关系。与检索结果的根本区别在于：它可以包含**用户没直接问、但由图结构必然成立**
/// 的间接关联——这正是"意外性"的来源。
///
/// 设计原则（PREREG §3.48）：
/// 1. **不做语义匹配**：边全部来自记录（event_id / entities / source_ids），
///    不引入任何向量相似度——语义匹配交给 BGE。
/// 2. **间接关联只做结构传递**：多跳是图上的路径合成，
///    不是"猜"两条记忆在语义上相关。路径本身即是解释。
/// 3. **裁剪必须可见**：若因规模上限截断，`truncated` 会标记
///    （承方法论 100：过滤不得静默）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociationGraph {
    /// 根节点（推理起点）的记忆 ID
    pub root: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// 直接边数量（hops == 1）
    pub direct_count: usize,
    /// 间接边数量（hops >= 2）
    pub indirect_count: usize,
    /// 是否因规模上限而被截断（true 时用户不应把结果视为完整图）
    pub truncated: bool,
}

/// 从图存储**直读**的关系边（v0.9.8）
///
/// 与 [`GraphEdge`] 的区别（**不可混同**）：
///
/// | | `GraphEdge` | 本类型 |
/// |---|---|---|
/// | 来源 | `associations_in` 当场推导 | `graph_edges.json` 落盘边 |
/// | 内容 | 记录层关系 + 2 跳路径合成 | **全部**落盘边（含 §5.3 逻辑关系） |
/// | 解释 | 带 `why` / `via`（可读路径） | 带 `weight` / `created_at`（可核证据） |
///
/// **为什么必须有它**（实测缺口，2026-09-17）：
/// `graph_store` 的边此前**没有任何 HTTP 读出口**——唯一的读端点
/// `/memories/association-graph` 走的是 `associations_in`（内存记录层推导），
/// **完全不读 `graph_store`**。实测证据：写入 `coordinate` 边后，
/// `graph_edges.json` 里确有此边，但该端点的返回中看不到它。
/// ⇒ 文档 §6 #4 的 `engram.query(node_id, rel_type?, hops≤3)` 契约此前**不可满足**。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEdge {
    /// 边的源记忆 ID（**落盘边的真实方向**，非查询根）
    pub from: String,
    /// 边的目标记忆 ID（同上）
    pub to: String,
    /// 关系类型（`EdgeType::as_str()` 的取值，如 `coordinate` / `cause`）
    pub relation: String,
    /// 边权（0~1）；多跳时已按 `γ^hop` 衰减（γ=0.7，承 §4.5）
    pub weight: f32,
    /// 是否为对称关系（true 时方向仅表示书写顺序，不表示因果）
    pub symmetric: bool,
    /// 距查询根的跳数：1 = 直接边；≥2 = 经中间节点
    pub hops: usize,
    /// 遍历路径（节点 ID 序列）：恒以查询根开头、以本次边的一端结尾。
    ///
    /// **与 `from`/`to` 的关系**：`path` 描述**怎么走到的**，`from`/`to`
    /// 描述**这条落盘边的真实两端**。对称关系在图里是按 ID 字典序规范化的，
    /// 故 `path` 的末节点可能与 `to` 相同也可能与 `from` 相同——
    /// 用 `hops`/`path` 判断可达性，用 `from`/`to` 判断边的原始方向。
    pub path: Vec<String>,
    /// 边创建时间（RFC3339）
    pub created_at: String,
}

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
    /// 只读检索（P7 主动发现专用，判据见研究资产侧 PREREG_ACTIVE_DISCOVERY.md §3.1 D3）：
    /// 置 true 时本次 recall **不写回任何状态**——不更新状态机活跃锚点
    /// （`bake_activation`）、不记录探索日志、不累加检索指标。
    ///
    /// **为什么必须有这个开关**：主动发现是"第二通道"，它在后台自主发起检索。
    /// 若该检索照常写入状态机，则用户查询路径上的两个排序输入会随发现功能的
    /// 开关而改变——① 活性偏置（`active_map` 按激活强度加分）；
    /// ② 联想桥词扩展（`active_ids` 的内容抽词并入查询）。届时 D3「零伤害
    /// 承诺」（开关开启与关闭时用户查询返回结果逐字节一致）**在机制上不可能
    /// 成立**，无论发现逻辑写得多干净。
    pub read_only: bool,
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
            read_only: false,
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
    /// 带 event_id 的记忆数（v0.9.7 记录层：让"共同经历"的积累可观测）
    ///
    /// **为什么必须暴露**：`event_id` 是联想的前提（PREREG §3.42/§3.43），
    /// 但它是**可选字段**，若无人填写则覆盖率恒为 0，关联推导永远为空。
    /// 不暴露该指标，产品侧无法判断"记录层是否真的在积累"，
    /// 会把"无人填写"误读为"机制无效"（承接方法论 92：先查记录层）。
    pub with_event_count: usize,
    /// 带实体的记忆数（`entities` 非空）
    pub with_entity_count: usize,
    /// 已形成的事件簇数（不同 event_id 的个数）
    pub event_cluster_count: usize,
    /// 记忆类型为 Experience 的条数
    pub experience_count: usize,
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
