// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件实现审计追踪，属于守护层 (Layer 2)。
// ============================================================
//
// 审计追踪 (AuditTrail)
//
// 解决质疑五"自主演化与用户信任之间的临界点"问题：
// 提供完整的、可回溯的系统自主行为日志，包括合成、删除、
// 衰减加速、GC 清理等。每一条日志都包含明确的理由和时间戳，
// 让用户即使在系统"自主"运行时，也能保持完全的知情权。
//
// 核心功能：
//   - 记录所有系统自主行为（合成、删除、隔离、GC、调节等）
//   - 按时间范围、事件类型、受影响的记忆 ID 查询
//   - FIFO 环形缓冲区，自动淘汰旧事件
//   - 通过 /v1/audit-trail 端点暴露

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 审计事件类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    /// 合成记忆创建
    SynthesisCreated,
    /// 记忆被系统删除（GC 或衰减过期）
    MemoryDeleted,
    /// 记忆被用户/系统隔离
    MemoryIsolated,
    /// 衰减速率被调节器调整
    DecayRateChanged,
    /// 合成阈值被调节器调整
    SynthesisThresholdChanged,
    /// 检索权重被调整
    RetrievalWeightsAdjusted,
    /// 重新编码建议
    ReencodingSuggested,
    /// GC 垃圾回收执行
    GcCleanup,
    /// 调节动作被应用
    RegulationApplied,
    /// 用户反馈被处理
    FeedbackProcessed,
    /// 综合再平衡
    ComprehensiveRebalance,
    /// 灾难性事件检测
    CatastrophicEvent,
    /// 慢性恶化检测
    ChronicDegradation,
    /// 调节器冻结
    RegulatorFrozen,
    /// 调节器解冻
    RegulatorUnfrozen,
    /// 信任锚点创建（质疑四：分布式信任锚点系统）
    TrustAnchorCreated,
    /// 信任锚点发布到外部
    TrustAnchorPublished,
    /// 双人确认请求
    DualConfirmationRequested,
    /// 双人确认通过
    DualConfirmationGranted,
    /// 双人确认拒绝
    DualConfirmationDenied,
    /// 联想检索执行（阶段D：检索链路结构化审计，只观测不排序）
    ///
    /// 由 enrich 联想检索触发，记录两路权重、候选规模与每条结果的
    /// 通路贡献分解。不参与默认排序决策，仅提供可审计的联想解释数据。
    RetrievalExecuted,
    /// 联想确认（v0.9.7：用户在联想探索中点击"就是这个"）
    ///
    /// 记录用户确认某条联想记忆与查询意图相关的行为，
    /// 该确认会以最高激活强度写回道体状态机活跃锚点。
    AssociationConfirmed,
}

impl AuditEventType {
    pub fn as_str(&self) -> &str {
        match self {
            AuditEventType::SynthesisCreated => "synthesis_created",
            AuditEventType::MemoryDeleted => "memory_deleted",
            AuditEventType::MemoryIsolated => "memory_isolated",
            AuditEventType::DecayRateChanged => "decay_rate_changed",
            AuditEventType::SynthesisThresholdChanged => "synthesis_threshold_changed",
            AuditEventType::RetrievalWeightsAdjusted => "retrieval_weights_adjusted",
            AuditEventType::ReencodingSuggested => "reencoding_suggested",
            AuditEventType::GcCleanup => "gc_cleanup",
            AuditEventType::RegulationApplied => "regulation_applied",
            AuditEventType::FeedbackProcessed => "feedback_processed",
            AuditEventType::ComprehensiveRebalance => "comprehensive_rebalance",
            AuditEventType::CatastrophicEvent => "catastrophic_event",
            AuditEventType::ChronicDegradation => "chronic_degradation",
            AuditEventType::RegulatorFrozen => "regulator_frozen",
            AuditEventType::RegulatorUnfrozen => "regulator_unfrozen",
            AuditEventType::TrustAnchorCreated => "trust_anchor_created",
            AuditEventType::TrustAnchorPublished => "trust_anchor_published",
            AuditEventType::DualConfirmationRequested => "dual_confirmation_requested",
            AuditEventType::DualConfirmationGranted => "dual_confirmation_granted",
            AuditEventType::DualConfirmationDenied => "dual_confirmation_denied",
            AuditEventType::RetrievalExecuted => "retrieval_executed",
            AuditEventType::AssociationConfirmed => "association_confirmed",
        }
    }
}

/// 单条审计事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// 事件唯一 ID
    pub id: String,
    /// 事件时间戳（毫秒）
    pub timestamp_ms: u64,
    /// 事件类型
    pub event_type: AuditEventType,
    /// 人类可读的描述
    pub description: String,
    /// 执行原因（系统为什么这么做）
    pub reason: String,
    /// 受影响的记忆 ID 列表
    pub affected_memory_ids: Vec<String>,
    /// 额外元数据
    pub metadata: HashMap<String, String>,
    /// 前一条事件的哈希（质疑四：哈希链防篡改）
    /// 空字符串表示创世事件（链上第一条）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub previous_hash: String,
    /// 本条事件的哈希（质疑四：哈希链防篡改）
    /// 由 previous_hash + 事件内容计算得出
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub event_hash: String,
    /// 内容哈希格式版本（用于区分"真篡改"与"旧格式不可验证"）
    ///
    /// - "canonical_v2"：规范确定性编码（排序键 + 长度前缀单射），verify 时 canonical
    ///   失配即判为真篡改（硬失败）。
    /// - 空字符串：旧版 `{:?}` 序列化（HashMap 迭代顺序跨进程不稳定），verify 时
    ///   canonical 与 legacy 双双重算失配只记为"旧格式不可验证"（软信号，不判断裂）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash_format: String,
}

/// 审计查询参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditQuery {
    /// 起始时间戳（毫秒），可选
    pub from_ms: Option<u64>,
    /// 结束时间戳（毫秒），可选
    pub to_ms: Option<u64>,
    /// 事件类型过滤，可选
    pub event_types: Option<Vec<AuditEventType>>,
    /// 受影响的记忆 ID 过滤，可选
    pub memory_id: Option<String>,
    /// 最大返回条数，默认 100
    pub limit: Option<usize>,
}

/// 完整性验证结果（质疑四：哈希链防篡改）
#[derive(Debug, Clone)]
pub struct IntegrityVerification {
    /// 审计链是否完整
    pub is_valid: bool,
    /// 第一条不匹配的事件索引（None 表示全部有效）
    pub first_mismatch: Option<usize>,
    /// 验证详情
    pub details: String,
    /// 无法用当前规范格式验证的旧格式事件数（0 = 无）
    ///
    /// 旧版（升级前）用 `{:?}` 序列化 HashMap，其迭代顺序跨进程不稳定，
    /// 无法在新进程重算哈希。此类事件不等于被篡改——只要哈希链
    /// （previous_hash 链接）完整，就只标记为"旧格式不可验证"，而非判为断裂。
    pub legacy_unverifiable: usize,
}

impl IntegrityVerification {
    /// 兼容旧构造点：未指定 legacy_unverifiable 时视为 0（无旧格式事件）。
    pub fn ok(details: String) -> Self {
        Self {
            is_valid: true,
            first_mismatch: None,
            details,
            legacy_unverifiable: 0,
        }
    }

    pub fn fail(idx: Option<usize>, details: String) -> Self {
        Self {
            is_valid: false,
            first_mismatch: idx,
            details,
            legacy_unverifiable: 0,
        }
    }
}

// ============================================================
// 质疑四"完美闭环悖论"：分布式信任锚点系统
//
// 道枢映射：离卦·火 (☲) — "明两作，离。大人以继明照于四方。"
// 信任锚点如同离卦的双重光明——第一重是审计日志，第二重是外部锚定。
// 双重确认如同离卦的双日并照，任何单一光源的熄灭都不会导致黑暗。
// ============================================================

/// 信任锚点（质疑四"完美闭环悖论"：分布式信任锚点系统）
///
/// 每个锚点将当前哈希链状态封装为不可篡改的快照。
/// 通过定期创建锚点并发布到外部见证系统，打破"用户是唯一不受监控的神"
/// 这一完美闭环悖论——即使管理员账号被盗，已发布的锚点也无法被修改。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustAnchor {
    /// 锚点唯一标识
    pub anchor_id: String,
    /// 创建时间戳（毫秒）
    pub created_at_ms: u64,
    /// 封装时的最后一条事件哈希
    pub last_event_hash: String,
    /// 封装时的总事件数
    pub total_events_at_anchor: u64,
    /// 外部见证哈希（可选，由外部系统提供）
    pub external_witness_hash: Option<String>,
    /// 审计链的 Merkle 根哈希
    pub anchor_merkle_root: String,
    /// 是否已发布到外部
    pub is_published: bool,
    /// 发布时间戳（毫秒）
    pub published_at_ms: Option<u64>,
    /// 发布位置描述
    pub publish_location: Option<String>,
}

/// 信任锚点配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustAnchorConfig {
    /// 自动锚定间隔（毫秒），默认 3600000 = 1 小时
    pub auto_anchor_interval_ms: u64,
    /// 是否要求关键操作双人确认
    pub require_dual_confirmation: bool,
    /// 外部见证服务 URL（可选）
    pub external_witness_url: Option<String>,
    /// 锚点持久化路径
    pub anchor_persistence_path: Option<String>,
}

impl Default for TrustAnchorConfig {
    fn default() -> Self {
        Self {
            auto_anchor_interval_ms: 3600000, // 默认每小时自动锚定一次
            require_dual_confirmation: false,
            external_witness_url: None,
            anchor_persistence_path: None,
        }
    }
}

/// 双人确认状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationStatus {
    /// 等待确认
    Pending,
    /// 已通过
    Granted,
    /// 已拒绝
    Denied,
}

/// 待双人确认的操作
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingConfirmation {
    /// 请求唯一标识
    pub request_id: String,
    /// 操作描述
    pub operation: String,
    /// 请求者标识
    pub requested_by: String,
    /// 请求时间戳（毫秒）
    pub requested_at_ms: u64,
    /// 确认状态
    pub status: ConfirmationStatus,
}

/// 获取当前时间戳（毫秒），供信任锚点方法和测试使用
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 计算 Merkle 根哈希（独立函数，用于信任锚点）
///
/// 将所有事件哈希两两配对，逐层向上计算，最终得到根哈希。
/// 每层使用 SipHash 对配对字符串进行哈希运算。
fn compute_merkle_root(hashes: &[String]) -> String {
    if hashes.is_empty() {
        return String::new();
    }
    let mut level: Vec<String> = hashes.to_vec();
    while level.len() > 1 {
        let mut next_level = Vec::new();
        for chunk in level.chunks(2) {
            let combined = if chunk.len() == 2 {
                format!("{}{}", chunk[0], chunk[1])
            } else {
                // 奇数个时，最后一个自己和自己配对
                chunk[0].clone()
            };
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            combined.hash(&mut hasher);
            next_level.push(format!("{:016x}", hasher.finish()));
        }
        level = next_level;
    }
    level.into_iter().next().unwrap_or_default()
}

/// 异步持久化消息（质疑三·性能 + 质疑三·终极）
///
/// - `Event`: 追加一条审计事件行到 JSONL
/// - `Seal`: 将完整性封印（哈希链根）写入独立封印文件
enum AuditWriteMsg {
    Event(String),
    Seal(String),
}

/// 计算 SipHash-1-3（DefaultHasher）的 16 进制编码。
///
/// 独立自由函数：供验证路径在持有事件可变借用（迁移重算）时使用，
/// 避免 `&mut self` 与 `&self` 借用冲突。
fn compute_siphash(input: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 将事件的非确定性字段序列化为规范（确定性）哈希输入
///
/// 修复：旧实现用 `{:?}` 序列化 `HashMap<String, String>`，其迭代顺序依赖
/// 每进程随机的 SipHash 种子，跨进程重启后重算的哈希会与落盘时不一致，
/// 导致未篡改的审计日志被误报为"哈希链断裂"。
/// 改为排序键名的规范形式，保证任意进程/任意时刻重算结果一致。
fn canonical_hash_fields(
    affected: &[String],
    metadata: &HashMap<String, String>,
) -> (String, String) {
    // 单射编码：为每个元素加"长度:值"前缀，并转义分隔符，
    // 避免 ["a,b","c"] 与 ["a","b,c"] 产生相同 canonical 串（哈希碰撞风险）。
    fn enc(v: &str) -> String {
        let escaped = v.replace('\\', "\\\\").replace(':', "\\:");
        format!("{}:{}", escaped.len(), escaped)
    }
    let affected_str = affected
        .iter()
        .map(|s| enc(s))
        .collect::<Vec<_>>()
        .join(",");
    let mut keys: Vec<&String> = metadata.keys().collect();
    keys.sort();
    let meta_str = keys
        .iter()
        .map(|k| format!("{}={}", enc(k), enc(&metadata[*k])))
        .collect::<Vec<_>>()
        .join("&");
    (affected_str, meta_str)
}

/// 审计追踪器
///
/// 维护一个 FIFO 环形缓冲区，记录系统所有自主行为。
/// 默认保留最近 10000 条事件，超出后自动淘汰最旧的事件。
///
/// v2.0 新增可选的 JSONL 持久化后端（质疑五）：
/// 当设置 persist_path 后，所有事件自动追加写入 JSONL 文件。
/// JSONL 文件为只追加（append-only），任何事件都不会从中删除，
/// 因此内存窗口（max_events）溢出不影响磁盘上的审计完整性——
/// 溢出的事件仍可随时从文件重新加载并检索。
///
/// v3.0 新增哈希链防篡改（质疑四）：
/// 每条事件包含 previous_hash 和 event_hash，形成不可篡改的
/// 哈希链。验证函数 verify_integrity() 可检测任何篡改。
///
/// v4.0 新增异步持久化（质疑三·性能）：
/// JSONL 持久化由独立后台线程处理，不阻塞主业务流程。
/// 使用 mpsc channel 解耦事件记录与磁盘写入。
#[derive(Debug)]
pub struct AuditTrail {
    /// 事件列表（按时间倒序，最新的在前）
    events: Vec<AuditEvent>,
    /// 事件计数器（用于生成自增 ID）
    counter: u64,
    /// 最大保留事件数
    max_events: usize,
    /// JSONL 持久化路径（可选，质疑五）
    persist_path: Option<String>,
    /// 总写入事件数（含已溢出的，用于统计）
    total_written: u64,
    /// 上一条事件的哈希（质疑四：哈希链防篡改）
    last_hash: String,
    /// 异步持久化发送端（质疑三·性能：解耦事件记录与磁盘写入）
    /// None 表示同步模式（无持久化或未启用异步）
    async_writer: Option<std::sync::mpsc::SyncSender<AuditWriteMsg>>,
    /// 后台写入线程句柄（质疑三·性能）
    writer_thread: Option<std::thread::JoinHandle<()>>,
    /// 完整性封印（质疑三·终极：防篡改硬化）
    ///
    /// 存储哈希链根（即最后一条事件的哈希），保存在独立文件中。
    /// 即使攻击者修改了审计日志 JSONL 文件并重新计算哈希链，
    /// 只要封印文件是独立的，就能检测到篡改。
    /// 道枢映射：乾卦·天 (☰) — 天行健，君子以自强不息；
    ///   封印如同天道，独立于人事，不可更改。
    integrity_seal: String,
    /// 封印持久化路径（独立于审计日志文件）
    seal_path: Option<String>,
    /// 封印是否已验证（启动时验证一次）
    seal_verified: bool,
    /// 信任锚点列表（质疑四：分布式信任锚点系统）
    ///
    /// 每个锚点将当前哈希链状态封装为不可篡改的快照。
    /// 通过定期创建锚点并发布到外部见证系统，打破
    /// "用户是唯一不受监控的神"这一完美闭环悖论。
    trust_anchors: Vec<TrustAnchor>,
    /// 锚点配置
    anchor_config: TrustAnchorConfig,
    /// 上次锚定时间戳（毫秒），用于自动锚定检查
    last_anchor_ms: u64,
    /// 待双人确认的操作列表（质疑四：双人确认机制）
    ///
    /// 关键操作需要第二人确认才能执行，防止单个恶意
    /// 内部人员或被盗账号进行隐蔽的数据污染。
    pending_dual_confirmations: Vec<PendingConfirmation>,
    /// 被用户**隐藏**的事件 ID 集合（v0.9.8）
    ///
    /// # 为什么用"隐藏集合"而不是"真删除"
    ///
    /// 用户点「清除联想足迹」的诉求是"不想再看到这些记录"，而审计日志是
    /// **防篡改哈希链**（`event_hash` 由 `previous_hash + 事件内容` 计算，
    /// 见 [`canonical_hash_fields`]）——**从 JSONL 中删行会让链条断裂**，
    /// `verify_integrity` 将报篡改。
    ///
    /// 因此采用**独立隐藏集合**：
    /// - 不动事件内容 ⇒ 哈希链完好，完整性校验照常通过
    /// - [`Self::query`] 过滤掉隐藏项 ⇒ 用户视角"已清除"
    /// - 隐藏集合**独立落盘**（`.hidden` 文件，JSONL 每行一个 ID）⇒ 重启后仍隐藏
    ///
    /// # 为什么不能用 `metadata` 打标记
    ///
    /// `metadata` 的**全部键值**都参与 `canonical_hash_fields` ⇒ 往里加
    /// `hidden=true` 会改变 `metadata_canon`，导致该事件的
    /// `event_hash` 与重算值不符 ⇒ **立即被判定为篡改**（硬失败）。
    /// 故标记必须存放在事件**之外**。
    hidden_ids: std::collections::HashSet<String>,
    /// 隐藏集合的持久化路径（派生自审计日志路径：`<audit>.hidden`）
    hidden_path: Option<String>,
}

impl Drop for AuditTrail {
    fn drop(&mut self) {
        // 质疑三·性能：优雅关闭后台写入线程
        // 丢弃 sender 会关闭 channel，后台线程检测到 channel 关闭后自动退出
        // JoinHandle 在 AuditTrail 被 drop 时也会被 drop，
        // 但如果线程尚未完成，我们需要等待它完成
        if let Some(handle) = self.writer_thread.take() {
            // 先丢弃 sender 通知线程退出
            self.async_writer.take();
            // 等待线程完成（最多等待 5 秒）
            let _ = handle.join();
        }
    }
}

impl AuditTrail {
    /// 创建新的审计追踪器
    pub fn new() -> Self {
        Self {
            events: Vec::with_capacity(10000),
            counter: 0,
            max_events: 10000,
            persist_path: None,
            total_written: 0,
            last_hash: String::new(), // 创世事件，previous_hash 为空
            async_writer: None,
            writer_thread: None,
            integrity_seal: String::new(), // 质疑三·终极：初始为空，首次封印时生成
            seal_path: None,
            seal_verified: false,
            trust_anchors: Vec::new(), // 质疑四：信任锚点列表
            anchor_config: TrustAnchorConfig::default(), // 质疑四：锚点配置
            last_anchor_ms: 0,         // 质疑四：尚未锚定
            pending_dual_confirmations: Vec::new(), // 质疑四：待确认列表
            hidden_ids: std::collections::HashSet::new(), // v0.9.8：初始无隐藏事件
            hidden_path: None,
        }
    }

    /// 设置 JSONL 持久化路径（质疑五：永久审计）
    ///
    /// 设置后，所有新事件将自动追加写入指定 JSONL 文件。
    /// 如果文件已存在，将从中加载历史事件到内存缓冲区。
    ///
    /// 质疑三·性能：启动独立后台线程处理 JSONL 写入，
    /// 使用 mpsc channel 解耦事件记录与磁盘 I/O，
    /// 确保高负载下主业务流程不受阻塞。
    ///
    /// 质疑三·终极：自动设置并验证完整性封印。
    /// 封印文件独立于审计日志，提供第二层防篡改保护。
    pub fn set_persist_path(&mut self, path: &str) -> std::io::Result<()> {
        self.persist_path = Some(path.to_string());

        // 质疑三·终极：设置封印路径（.lrc_audit_seal）
        let seal_path = format!("{}.seal", path);
        self.seal_path = Some(seal_path.clone());

        // v0.9.8：「清除联想足迹」的隐藏集合——独立于审计日志的文件，
        // 避免"删除记录"与"防篡改哈希链"的根本冲突（见 hidden_ids 字段文档）
        let hidden_path = format!("{}.hidden", path);
        self.hidden_path = Some(hidden_path.clone());
        self.load_hidden_from_disk(&hidden_path);

        // 质疑四·锚点跨重启持久化：若未显式配置锚点文件，
        // 默认派生 `<audit>.anchors.jsonl`，并从磁盘恢复历史锚点。
        let anchor_path = self
            .anchor_config
            .anchor_persistence_path
            .clone()
            .unwrap_or_else(|| format!("{}.anchors.jsonl", path));
        self.anchor_config.anchor_persistence_path = Some(anchor_path.clone());
        // 先探测审计日志是否存在，用于区分"首次运行"与"锚点证据丢失"
        let audit_log_exists = std::path::Path::new(path).exists();
        let restored_anchors = self.load_anchors_from_disk(&anchor_path);
        if audit_log_exists && restored_anchors == 0 {
            eprintln!(
                "[LRC·审计·锚点·告警] 审计日志存在但未能从 {} 恢复任何历史锚点：锚点证据可能丢失或文件损坏，建议核查",
                anchor_path
            );
        }

        // 尝试从已有文件加载历史事件
        if audit_log_exists {
            self.load_from_file(path)?;
            // P0 修复（2026-09-01）：加载后把旧格式事件一次性迁移为
            // canonical_v2（重算哈希并回写），保证验证路径不依赖事件自述
            // 格式、无软信号后门。迁移是可信的加载期操作。
            self.migrate_legacy_events(path)?;
            // 质疑三·终极：加载后立即验证封印
            self.verify_integrity_with_seal();
        }

        // 质疑三·性能：启动异步持久化后台线程
        self.start_async_writer(path)?;

        Ok(())
    }

    /// 从磁盘恢复历史信任锚点（跨重启持久化）。
    ///
    /// 2026-09-01 修复：此前锚点仅存内存，重启后 `trust_anchors` 恒为空，
    /// `verify_anchor_chain` 因"无锚点视为有效"而恒真，悬空锚点防护整体失效。
    /// 现在从 JSONL 逐行反序列化恢复，并在异常时告警而非静默放行。
    ///
    /// 返回恢复的锚点数量；若审计日志存在但锚点文件缺失/完全损坏，
    /// 会输出告警（与封印缺失处理对称），避免"锚点丢失→校验恒真"再次出现。
    fn load_anchors_from_disk(&mut self, path: &str) -> usize {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => {
                // 区分"首次运行（无日志）"与"有日志但锚点文件缺失"——
                // 后者属于锚点证据丢失，需显式告警（在 set_persist_path 中判断）。
                return 0;
            }
        };
        let mut restored = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(anchor) = serde_json::from_str::<TrustAnchor>(line) {
                restored.push(anchor);
            } else {
                eprintln!(
                    "[LRC·审计·锚点] 锚点文件 {} 中存在无法解析的行，已跳过（可能损坏）",
                    path
                );
            }
        }
        // 文件存在但全部行都无法解析（损坏/截断）→ 告警而非静默清空
        if content.trim().is_empty() {
            eprintln!(
                "[LRC·审计·锚点·告警] 锚点文件 {} 为空（历史锚点可能已丢失）",
                path
            );
        }
        let restored_count = restored.len();
        if restored_count > 0 {
            self.trust_anchors = restored;
            eprintln!(
                "[LRC·审计·锚点] 从磁盘恢复 {} 条历史信任锚点",
                self.trust_anchors.len()
            );
        }
        restored_count
    }

    /// 启动异步持久化后台线程（质疑三·性能）
    ///
    /// 创建 mpsc channel 和后台线程，将 JSONL 文件写入
    /// 从主业务流程中解耦。channel 缓冲区大小 4096，
    /// 满时 send 会阻塞，防止内存无限增长。
    fn start_async_writer(&mut self, path: &str) -> std::io::Result<()> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<AuditWriteMsg>(4096);
        let path_clone = path.to_string();
        let seal_path_clone = self.seal_path.clone();

        let handle = std::thread::Builder::new()
            .name("lrc-audit-writer".to_string())
            .spawn(move || {
                use std::io::Write;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        AuditWriteMsg::Event(line) => {
                            // 追加写入 JSONL 文件，忽略单条写入失败
                            if let Ok(mut file) = std::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&path_clone)
                            {
                                let _ = writeln!(file, "{}", line);
                            }
                        }
                        AuditWriteMsg::Seal(seal) => {
                            // 写独立封印文件（覆盖写，始终反映最新哈希链根）
                            if let Some(ref seal_path) = seal_path_clone {
                                let _ = std::fs::write(seal_path, seal);
                            }
                        }
                    }
                }
                // channel 关闭，线程正常退出
                eprintln!("[LRC·审计] 异步持久化线程已退出");
            })?;

        self.async_writer = Some(tx);
        self.writer_thread = Some(handle);

        eprintln!("[LRC·审计] 异步持久化已启动，后台线程: lrc-audit-writer");
        Ok(())
    }

    /// 刷新异步持久化缓冲区（质疑三·性能）
    ///
    /// 关闭当前 channel 等待后台线程处理完所有待处理消息，
    /// 然后重新启动异步写入器。用于测试中确保事件已落盘，
    /// 或优雅关闭前确保数据完整性。
    pub fn flush(&mut self) {
        // 关闭 sender，后台线程将处理完缓冲区中剩余消息后退出
        if let Some(tx) = self.async_writer.take() {
            drop(tx);
        }
        // 等待后台线程退出
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
        // 重新启动异步写入器，确保后续事件可继续写入
        if let Some(ref path) = self.persist_path {
            let path = path.clone();
            if let Err(e) = self.start_async_writer(&path) {
                eprintln!("[LRC·审计·错误] 刷新后重启异步写入器失败: {}", e);
            }
        }
    }

    // ============================================================
    // 质疑三·终极：完整性封印 — 防篡改硬化
    //
    // 哈希链保证了"检测"能力，但无法阻止本地有 root 权限的
    // 攻击者同时修改日志文件和哈希验证逻辑。
    //
    // 完整性封印将哈希链根写入独立文件（.lrc_audit_seal），
    // 提供了第二层防护：
    // 1. 独立存储：封印文件与日志文件分离，攻击者需同时修改两处
    // 2. 定期验证：系统启动时和运行中定期交叉验证
    // 3. 变更告警：封印与日志不匹配时立即告警
    //
    // 道枢映射：乾卦·天 (☰) — 万物资始，乃统天。
    //   封印如同天道之印，独立于一地一事，见证一切变迁。
    // ============================================================

    /// 道枢映射: 坎卦·水 (☵) — 水流而不盈，封印如水源之标记，记录完整性的根
    /// 封印当前哈希链的完整性状态
    ///
    /// 将当前 last_hash（哈希链根）写入独立的封印文件。
    /// 每次调用覆盖之前的封印，确保封印始终反映最新状态。
    pub fn seal_integrity(&mut self) {
        let seal = &self.last_hash;
        self.integrity_seal = seal.clone();

        if let Some(ref seal_path) = self.seal_path {
            if let Err(e) = std::fs::write(seal_path, seal) {
                eprintln!("[LRC·审计·封印] 写入封印文件失败: {}", e);
            } else {
                self.seal_verified = true;
            }
        }
    }

    /// 判断给定事件哈希是否存在于历史中（先内存链，再 JSONL 全量）。
    ///
    /// 2026-09-01 修复：封印刷新与锚点悬空判定此前仅查内存链
    /// （≤10000 条窗口），若被校验的哈希已被 FIFO 淘汰会误报篡改/悬空。
    /// 现统一为"内存 + 磁盘全量"双路检索，不遗漏窗口外历史。
    fn hash_exists_in_history(&self, target: &str) -> bool {
        if self.events.iter().any(|e| e.event_hash == *target) {
            return true;
        }
        // 磁盘全量检索（仅在内存未命中时执行，避免正常路径的 O(N) 开销）
        if let Some(ref persist_path) = self.persist_path {
            if let Ok(content) = std::fs::read_to_string(persist_path) {
                return content
                    .lines()
                    .filter_map(|line| serde_json::from_str::<AuditEvent>(line.trim()).ok())
                    .any(|e| e.event_hash == *target);
            }
        }
        false
    }

    /// 道枢映射: 坎卦·水 (☵) — 行险而不失其信，封印验证是双重诚信保障
    /// 使用封印文件验证审计链完整性
    ///
    /// 将内存中的哈希链根与封印文件中存储的值对比。
    /// 不匹配说明审计日志被篡改过。
    ///
    /// 返回 true 表示验证通过，false 表示封印缺失或不匹配。
    pub fn verify_integrity_with_seal(&mut self) -> bool {
        // 如果没有封印文件路径，无法验证
        let seal_path = match &self.seal_path {
            Some(p) => p.clone(),
            None => return true, // 无封印时假定通过（未启用持久化）
        };

        // 读取封印文件
        let stored_seal = match std::fs::read_to_string(&seal_path) {
            Ok(s) => s.trim().to_string(),
            Err(_) => {
                // 分发：有历史日志但封印缺失 → 异常（可能被删除/篡改场景）；
                // 无日志（首次使用）→ 创建初始封印。
                let has_logs = self
                    .persist_path
                    .as_ref()
                    .map(|p| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false))
                    .unwrap_or(false);
                if has_logs {
                    eprintln!(
                        "[LRC·审计·封印·告警] 审计日志存在但封印文件缺失：{} 封印可能被外部删除，建议人工核查",
                        seal_path
                    );
                    self.seal_verified = false;
                    return false;
                }
                // 首次使用：创建初始封印
                self.seal_integrity();
                return true;
            }
        };

        // 如果封印文件为空，写入当前封印
        if stored_seal.is_empty() {
            self.seal_integrity();
            return true;
        }

        let current_seal = &self.integrity_seal;
        if current_seal.is_empty() {
            // 内存中尚无封印（可能从文件加载但未计算），使用 last_hash
            self.integrity_seal = self.last_hash.clone();
        }

        let current = &self.integrity_seal;

        if stored_seal != *current {
            // 修复：封印落后于当前链根并不一定是篡改——
            // 事件记录与封印通过异步通道落盘，若进程在封印写入前退出/重启，
            // 存储的封印会落后若干事件。此时只要 stored_seal 仍对应链上
            // 某条已知事件的 event_hash，就属于"封印后链正常增长"，刷新封印即可；
            // 只有 stored_seal 在链上完全找不到时才判定为篡改。
            let is_chain_prefix = self.hash_exists_in_history(&stored_seal);
            if is_chain_prefix {
                // 链在封印之后继续增长，刷新封印到最新状态
                self.integrity_seal = self.last_hash.clone();
                self.seal_integrity();
                self.seal_verified = true;
                return true;
            }
            eprintln!(
                "[LRC·审计·封印·告警] 完整性封印验证失败！\n\
                  封印值: {}\n\
                  当前值: {}\n\
                  审计链可能已被篡改。建议立即审查审计日志文件。",
                stored_seal, current
            );
            self.seal_verified = false;
            return false;
        }

        self.seal_verified = true;
        true
    }

    /// 道枢映射: 兑卦·泽 (☱) — 说以利贞，自检如泽水之自净，系统自我审视
    /// 自检审计链完整性（质疑三·终极：定期自检）
    ///
    /// 可在后台定时调用，或通过 API 手动触发。
    /// 同时检查哈希链连续性和封印一致性。
    pub fn self_check_integrity(&mut self) -> bool {
        // 检查一：哈希链连续性
        let chain_valid = self.verify_hash_chain();

        // 检查二：封印一致性
        let seal_valid = self.verify_integrity_with_seal();

        // 检查三：刷新封印（如果通过且在持久化模式下）
        if chain_valid && seal_valid && self.persist_path.is_some() {
            self.seal_integrity();
        }

        let overall = chain_valid && seal_valid;

        if !overall {
            eprintln!(
                "[LRC·审计·自检] 完整性自检失败: 哈希链={}, 封印={}",
                if chain_valid { "通过" } else { "失败" },
                if seal_valid { "通过" } else { "失败" }
            );
        }

        overall
    }

    /// 验证哈希链的连续性（内部方法）
    fn verify_hash_chain(&self) -> bool {
        if self.events.is_empty() {
            return true; // 空链视为有效
        }

        // 从最旧到最新验证哈希链
        // events 按最新在前排列，所以需要反向迭代
        for i in (1..self.events.len()).rev() {
            let current = &self.events[i]; // 较旧的事件
            let previous = &self.events[i - 1]; // 较新的事件

            // previous 的 previous_hash 应该等于 current 的 event_hash
            if previous.previous_hash != current.event_hash {
                eprintln!(
                    "[LRC·审计·哈希链] 在事件 {} 处检测到链断裂:\n\
                      期望的 previous_hash: {}\n\
                      实际的 previous_hash: {}",
                    previous.id, current.event_hash, previous.previous_hash
                );
                return false;
            }
        }

        true
    }

    /// 道枢映射: 离卦·火 (☲) — 明也，封印验证状态如火光之可见
    /// 获取封印状态（用于健康报告）
    pub fn seal_verified(&self) -> bool {
        self.seal_verified
    }

    /// 获取当前封印值（用于健康报告）
    pub fn current_seal(&self) -> &str {
        &self.integrity_seal
    }

    /// 从 JSONL 文件加载历史事件
    fn load_from_file(&mut self, path: &str) -> std::io::Result<()> {
        let content = std::fs::read_to_string(path)?;
        let mut loaded = 0usize;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<AuditEvent>(line) {
                // 恢复计数器
                if let Some(num) = event.id.strip_prefix("audit_") {
                    if let Ok(n) = num.parse::<u64>() {
                        self.counter = self.counter.max(n);
                    }
                }
                loaded += 1;
                // 插入到开头保持时间倒序
                self.events.insert(0, event);
            }
        }

        // 限制内存中保留的数量
        if self.events.len() > self.max_events {
            self.events.truncate(self.max_events);
        }

        if loaded > 0 {
            eprintln!(
                "[LRC·审计] 从文件加载了 {} 条历史审计事件（内存保留 {} 条）",
                loaded,
                self.events.len()
            );
        }

        self.total_written = loaded as u64;

        // 质疑四：从文件加载后恢复哈希链状态
        self.recover_last_hash();

        Ok(())
    }

    /// 一次性迁移旧格式事件为 canonical_v2（P0 修复）。
    ///
    /// 旧版事件（hash_format 为空，用 `{:?}` 序列化 HashMap）的 event_hash
    /// 无法在新进程复算。为保证验证路径不依赖事件自述格式、无软信号后门，
    /// 在加载期用 canonical 确定性编码重算整条哈希链，统一标记为
    /// canonical_v2 并回写 JSONL。
    ///
    /// 注意：这会使旧事件的 event_hash/previous_hash 全部变化（内容不变，
    /// 仅哈希口径升级）；引用旧哈希的历史锚点将失效，需要重新锚定。
    fn migrate_legacy_events(&mut self, path: &str) -> std::io::Result<()> {
        // 必须从磁盘读取完整事件流，不能使用已按 max_events 截断的内存窗口。
        let content = std::fs::read_to_string(path)?;
        let mut all_events: Vec<AuditEvent> = content
            .lines()
            .filter_map(|line| serde_json::from_str::<AuditEvent>(line.trim()).ok())
            .collect();
        let has_legacy = all_events.iter().any(|e| e.hash_format != "canonical_v2");
        if !has_legacy {
            return Ok(());
        }
        let n = all_events.len();
        // 文件按最旧到最新存储；从最旧到最新重建完整哈希链。
        let mut prev_hash = String::new();
        for event in all_events.iter_mut() {
            let (affected_canon, metadata_canon) =
                canonical_hash_fields(&event.affected_memory_ids, &event.metadata);
            let hash_input = format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                prev_hash,
                event.id,
                event.timestamp_ms,
                event.event_type.as_str(),
                event.description,
                event.reason,
                affected_canon,
                metadata_canon
            );
            let new_hash = compute_siphash(&hash_input);
            event.previous_hash = prev_hash.clone();
            event.event_hash = new_hash.clone();
            event.hash_format = "canonical_v2".to_string();
            prev_hash = new_hash;
        }
        self.last_hash = prev_hash.clone();
        self.integrity_seal = prev_hash.clone();

        // 按原始顺序（最旧在前）写入唯一临时文件，再原子替换原日志。
        //
        // v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P3-3「原子写入逻辑重复 4 处」）：
        //   原为固定临时名 `format!("{}.migration.tmp", path)` 且失败不清理——
        //   并发迁移/写入同一审计日志时会争用同一临时文件。现统一委托 Layer 1
        //   公共设施 [`crate::atomic_file::write_atomic`]（UUID 唯一名 + 失败清理）。
        let mut out = String::new();
        for event in &all_events {
            if let Ok(json) = serde_json::to_string(event) {
                out.push_str(&json);
                out.push('\n');
            }
        }
        crate::atomic_file::write_atomic(std::path::Path::new(path), out.as_bytes())?;

        // 内存仍只保留最新窗口，但磁盘已保留完整事件流。
        self.events = all_events.into_iter().rev().take(self.max_events).collect();
        self.total_written = n as u64;
        self.recover_last_hash();
        self.seal_integrity();
        eprintln!(
            "[LRC·审计] 已将 {} 条旧格式事件迁移为 canonical_v2（历史锚点需重新锚定）",
            n
        );
        Ok(())
    }

    /// 道枢映射: 坎卦·水 (☵) — 水流而不息，事件记录如水流之连续
    /// 记录一条审计事件
    pub fn record(
        &mut self,
        event_type: AuditEventType,
        description: String,
        reason: String,
        affected_memory_ids: Vec<String>,
        metadata: HashMap<String, String>,
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        self.counter += 1;
        let id = format!("audit_{:016}", self.counter);

        // 质疑四：计算哈希链
        let previous_hash = self.last_hash.clone();
        // 哈希计算：previous_hash + 事件关键字段（规范确定性序列化）
        // 注意：必须与 verify_integrity() 中重算哈希的字段完全一致，
        // 否则记录后的正常事件在完整性校验时会判定为哈希不匹配。
        let (affected_canon, metadata_canon) =
            canonical_hash_fields(&affected_memory_ids, &metadata);
        let hash_input = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            previous_hash,
            id,
            now,
            event_type.as_str(),
            description,
            reason,
            affected_canon,
            metadata_canon
        );
        let event_hash = self.compute_hash(&hash_input);

        let event = AuditEvent {
            id,
            timestamp_ms: now,
            event_type,
            description,
            reason,
            affected_memory_ids,
            metadata,
            previous_hash,
            event_hash: event_hash.clone(),
            hash_format: "canonical_v2".to_string(),
        };

        // 更新链上最后哈希
        self.last_hash = event_hash;

        // 质疑三·终极：更新完整性封印（如果已启用持久化）
        if self.persist_path.is_some() {
            self.integrity_seal = self.last_hash.clone();
        }

        // 持久化到 JSONL 文件与封印文件（质疑三·性能：异步非阻塞）
        if let Some(ref tx) = self.async_writer {
            if let Ok(json) = serde_json::to_string(&event) {
                // 通过 channel 发送到后台线程，不阻塞主流程
                // sync_channel 缓冲区满时会阻塞，防止内存无限增长
                if let Err(e) = tx.send(AuditWriteMsg::Event(json)) {
                    eprintln!("[LRC·审计·警告] 异步持久化通道已关闭: {}", e);
                }
            }
            // 质疑三·终极：封印文件随哈希链同步推进（独立文件，二次防篡改）
            if let Err(e) = tx.send(AuditWriteMsg::Seal(self.last_hash.clone())) {
                eprintln!("[LRC·审计·封印] 封印异步写入通道已关闭: {}", e);
            }
        }

        // 插入到开头（最新在前）
        self.events.insert(0, event);
        self.total_written += 1;

        // 超出容量限制时移除最旧的
        if self.events.len() > self.max_events {
            self.events.truncate(self.max_events);
        }
    }

    /// 计算 SipHash 哈希（质疑四：哈希链防篡改）
    ///
    /// 使用 Rust 标准库的 DefaultHasher（SipHash-1-3），
    /// 生成 64 位哈希值并编码为 16 进制字符串。
    /// 对于审计链完整性验证而言，SipHash 的抗碰撞性足够，
    /// 且无需额外依赖。
    fn compute_hash(&self, input: &str) -> String {
        compute_siphash(input)
    }

    /// 道枢映射: 坎卦·水 (☵) — 行险而不失其信，哈希链验证如水流之诚信不可断
    /// 验证审计链的完整性（质疑四：哈希链防篡改）
    ///
    /// 遍历内存中的所有事件，验证每条事件的 previous_hash 是否
    /// 与前一条（更旧的）事件的 event_hash 一致。
    ///
    /// 事件按时间倒序存储（events[0] = 最新，events[n-1] = 最旧）。
    /// 哈希链方向：旧 → 新，即 events[i+1].event_hash 应等于
    /// events[i].previous_hash（新事件的 previous_hash 引用旧事件）。
    ///
    /// 返回完整性验证结果。
    pub fn verify_integrity(&self) -> IntegrityVerification {
        if self.events.is_empty() {
            return IntegrityVerification::ok("审计链为空，无需验证".to_string());
        }

        // 事件按时间倒序存储：[最新, ..., 最旧]
        // 验证方向：从旧到新，为每对相邻事件验证哈希链
        for i in (0..self.events.len() - 1).rev() {
            let newer = &self.events[i]; // events[i] = 较新的事件
            let older = &self.events[i + 1]; // events[i+1] = 较旧的事件

            // 较新事件的 previous_hash 应等于较旧事件的 event_hash
            if newer.previous_hash != older.event_hash {
                return IntegrityVerification::fail(
                    Some(i + 1),
                    format!(
                        "事件 #{}→#{} 哈希链断裂：id={} 的 previous_hash 与 id={} 的 event_hash 不匹配",
                        i + 1, i,
                        newer.id, older.id
                    ),
                );
            }
        }

        // 验证每条事件的内容哈希（包括创世事件）
        // P0 修复（2026-09-01）：验证路径不信任事件自述的 hash_format——
        // 任何 canonical 失配一律硬失败。旧格式事件在 set_persist_path 加载时
        // 已被统一迁移为 canonical_v2（见 migrate_legacy_events），
        // 因此运行期不存在"旧格式软信号"分支，攻击者无法靠清空 hash_format
        // 把真篡改降级为"不可验证"而规避检测。
        let mut legacy_unverifiable = 0usize;
        for event in self.events.iter() {
            let (affected_canon, metadata_canon) =
                canonical_hash_fields(&event.affected_memory_ids, &event.metadata);
            let hash_input = format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                event.previous_hash,
                event.id,
                event.timestamp_ms,
                event.event_type.as_str(),
                event.description,
                event.reason,
                affected_canon,
                metadata_canon
            );
            let recomputed = self.compute_hash(&hash_input);
            if recomputed != event.event_hash {
                // canonical 失配即判篡改（硬失败）。旧格式事件若未迁移
                // 也会在此失败——这正是期望行为：未迁移的旧日志不可信，
                // 必须显式触发迁移后才通过验证。
                if event.hash_format != "canonical_v2" {
                    legacy_unverifiable += 1;
                }
                return IntegrityVerification {
                    is_valid: false,
                    first_mismatch: None,
                    details: format!(
                        "事件 (id={}) 的内容哈希不匹配（canonical_v2）{}",
                        event.id,
                        if event.hash_format != "canonical_v2" {
                            "；该事件为未迁移的旧格式，请触发一次迁移"
                        } else {
                            ""
                        }
                    ),
                    legacy_unverifiable,
                };
            }
        }

        let mut result =
            IntegrityVerification::ok(format!("审计链完整，共 {} 条事件", self.events.len()));
        result.legacy_unverifiable = legacy_unverifiable;
        if legacy_unverifiable > 0 {
            result.is_valid = true;
            result.details = format!(
                "审计链完整，共 {} 条事件；其中 {} 条为旧格式（内容哈希无法在新进程复算，建议触发一次迁移/重新封印）",
                self.events.len(),
                legacy_unverifiable
            );
        }
        result
    }

    /// 从 JSONL 文件加载时恢复 last_hash（质疑四）
    ///
    /// 在 load_from_file 之后调用，确保后续新事件的哈希链连续。
    fn recover_last_hash(&mut self) {
        if let Some(first) = self.events.first() {
            // 最新的在列表开头
            self.last_hash = first.event_hash.clone();
        }
    }

    /// 获取审计事件总数（含已溢出的，质疑五·健康报告）
    pub fn total_events(&self) -> u64 {
        self.total_written
    }

    /// 检查是否启用了持久化（质疑五·健康报告）
    pub fn has_persistence(&self) -> bool {
        self.persist_path.is_some()
    }

    /// 道枢映射: 离卦·火 (☲) — 明也，查询如火光之照亮审计历史
    /// 按查询条件筛选事件
    ///
    /// v0.9.8：**过滤被用户隐藏的事件**（「清除联想足迹」的实现载体）。
    /// 过滤发生在本层而非删除层——事件在存储中仍完整保留以维持哈希链，
    /// 只是不再对用户可见（见 [`Self::hide_matching`] 的设计说明）。
    pub fn query(&self, query: &AuditQuery) -> Vec<&AuditEvent> {
        let limit = query.limit.unwrap_or(100).min(1000);

        self.events
            .iter()
            .filter(|event| {
                // v0.9.8：用户隐藏的事件不返回（用户视角"已清除"）
                if self.hidden_ids.contains(&event.id) {
                    return false;
                }
                // 时间范围过滤
                if let Some(from) = query.from_ms {
                    if event.timestamp_ms < from {
                        return false;
                    }
                }
                if let Some(to) = query.to_ms {
                    if event.timestamp_ms > to {
                        return false;
                    }
                }
                // 事件类型过滤
                if let Some(ref types) = query.event_types {
                    if !types.contains(&event.event_type) {
                        return false;
                    }
                }
                // 记忆 ID 过滤
                if let Some(ref mem_id) = query.memory_id {
                    if !event.affected_memory_ids.contains(mem_id) {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .collect()
    }

    /// 获取事件总数
    pub fn total_count(&self) -> usize {
        self.events.len()
    }

    /// 道枢映射: 坤卦·地 (☷) — 地势坤，类型统计如大地之分类承载
    /// 获取按事件类型的统计
    pub fn type_statistics(&self) -> HashMap<String, usize> {
        let mut stats = HashMap::new();
        for event in &self.events {
            *stats
                .entry(event.event_type.as_str().to_string())
                .or_insert(0) += 1;
        }
        stats
    }

    /// **隐藏**符合条件的事件，返回隐藏数量（v0.9.8，替代原 `clear_matching`）。
    ///
    /// # 与"删除"的区别（这是本轮修复的核心）
    ///
    /// 原名 `clear_matching` 只做 `self.events.retain(...)` —— **仅清内存**。
    /// 但审计日志是 append-only 的 JSONL（见结构体文档），且
    /// [`Self::load_from_file`] 在每次启动时会把磁盘记录**重新灌回内存**
    /// ⇒ 用户点「清除联想足迹」后，**重启 sidecar 记录全部复活**，功能实际无效。
    ///
    /// 实测（v0.9.8，dev 库）：
    /// ```text
    /// 清除前   内存 141 条 / 磁盘 141 行
    /// 清除后   内存   0 条 / 磁盘 141 行   ← 用户看到"已清除"，磁盘未变
    /// 重启后   内存 141 条 / 磁盘 141 行   ← 全部复活
    /// ```
    ///
    /// 本方法改为把命中事件的 **ID 记入隐藏集合**：
    /// - 事件本身**不删**（哈希链完好，`verify_integrity` 照常通过）
    /// - [`Self::query`] 过滤隐藏项 ⇒ 用户视角已清除
    /// - 隐藏集合落盘（`<audit>.hidden`）⇒ **重启后仍隐藏**
    ///
    /// # 为什么"隐藏"而不是"真删+重建链"
    ///
    /// 真删需要重算整条哈希链并重写封印与信任锚点——那会**摧毁
    /// "审计不可篡改"这一承诺本身**（用户将无法区分"系统重建了链"与
    /// "攻击者改了链"）。隐藏则把"用户隐私诉求"与"审计完整性"解耦：
    /// 前者是**视图层**的事，后者是**存储层**的事。
    pub fn hide_matching<F>(&mut self, mut predicate: F) -> usize
    where
        F: FnMut(&AuditEvent) -> bool,
    {
        // 先收集 ID（不可在遍历中借用 self.events 的可变引用）
        let ids: Vec<String> = self
            .events
            .iter()
            .filter(|e| predicate(e))
            .map(|e| e.id.clone())
            .collect();
        let n = ids.len();
        for id in ids {
            self.hidden_ids.insert(id);
        }
        if n > 0 {
            self.persist_hidden();
        }
        n
    }

    /// 取消隐藏（恢复可见），返回恢复数量。用于误操作回退。
    pub fn unhide_all(&mut self) -> usize {
        let n = self.hidden_ids.len();
        self.hidden_ids.clear();
        if n > 0 {
            self.persist_hidden();
        }
        n
    }

    /// 当前隐藏的事件数量
    pub fn hidden_count(&self) -> usize {
        self.hidden_ids.len()
    }

    /// 把隐藏集合写入磁盘（JSONL：每行一个事件 ID）
    ///
    /// **失败不静默**：写盘失败会打印错误（隐藏集合丢失会导致"重启后复活"
    /// 这一用户可见缺陷重现，必须可诊断）。
    fn persist_hidden(&self) {
        let Some(path) = self.hidden_path.as_ref() else {
            return;
        };
        let mut buf = String::new();
        for id in &self.hidden_ids {
            buf.push_str(id);
            buf.push('\n');
        }
        if let Err(e) = std::fs::write(path, buf) {
            eprintln!(
                "[LRC·审计] 隐藏集合写盘失败（重启后隐藏将失效）: {} - {}",
                path, e
            );
        }
    }

    /// 从磁盘加载隐藏集合（启动时调用）
    fn load_hidden_from_disk(&mut self, path: &str) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            // 文件不存在 = 从未隐藏过任何记录，属正常情况（不告警）
            Err(_) => return,
        };
        for line in content.lines() {
            let id = line.trim();
            if !id.is_empty() {
                self.hidden_ids.insert(id.to_string());
            }
        }
        if !self.hidden_ids.is_empty() {
            eprintln!(
                "[LRC·审计] 从文件加载了 {} 条隐藏记录（这些记录不会出现在联想足迹中）",
                self.hidden_ids.len()
            );
        }
    }

    /// 清理所有事件（慎用）
    ///
    /// **注意**：本方法只清内存，不影响磁盘 JSONL（append-only）。
    /// 若要"用户视角清除"，请用 [`Self::hide_matching`]。
    pub fn clear(&mut self) {
        self.events.clear();
    }

    // ============================================================
    // 质疑四"完美闭环悖论"：分布式信任锚点系统
    //
    // 道枢映射：离卦·火 (☲) — "明两作，离。大人以继明照于四方。"
    // 信任锚点如同离卦的双重光明——第一重是审计日志，第二重是外部锚定。
    // 双重确认如同离卦的双日并照，任何单一光源的熄灭都不会导致黑暗。
    // ============================================================

    /// 道枢映射: 离卦·火 (☲) — 明两作，锚点创建如第二重光明照亮审计链
    /// 创建新的信任锚点
    ///
    /// 将当前哈希链状态（最后一条事件哈希、总事件数、Merkle 根）封装为
    /// 不可篡改的锚点。锚点创建后可通过 publish_anchor() 发布到外部见证系统，
    /// 打破"用户是唯一不受监控的神"这一完美闭环悖论。
    ///
    /// 返回创建的锚点。
    ///
    /// 2026-09-01 修复(P2)：空链（无任何审计事件）时不创建锚点——
    /// 空哈希锚点会被 `verify_anchor_chain` 判为非法。`auto_anchor_check`
    /// 在首次调用时无条件锚定，若此时链为空会产生必然失败的锚点。
    pub fn create_anchor(&mut self) -> TrustAnchor {
        if self.last_hash.is_empty() {
            eprintln!("[LRC·审计·锚点] 审计链为空，跳过锚点创建（锚点必须封装真实链状态）");
            // 返回一个占位锚点（不写入 trust_anchors、不持久化）保持签名兼容
            return TrustAnchor {
                anchor_id: format!("anchor_{:016}", self.trust_anchors.len() + 1),
                created_at_ms: current_time_ms(),
                last_event_hash: String::new(),
                total_events_at_anchor: self.total_written,
                external_witness_hash: None,
                anchor_merkle_root: String::new(),
                is_published: false,
                published_at_ms: None,
                publish_location: None,
            };
        }
        let now = current_time_ms();
        let anchor_id = format!("anchor_{:016}", self.trust_anchors.len() + 1);
        let last_event_hash = self.last_hash.clone();
        let total_events = self.total_written;

        // 计算 Merkle 根：将当前所有事件哈希构建 Merkle 树
        let event_hashes: Vec<String> = self.events.iter().map(|e| e.event_hash.clone()).collect();
        let merkle_root = compute_merkle_root(&event_hashes);

        let anchor = TrustAnchor {
            anchor_id,
            created_at_ms: now,
            last_event_hash,
            total_events_at_anchor: total_events,
            external_witness_hash: None,
            anchor_merkle_root: merkle_root,
            is_published: false,
            published_at_ms: None,
            publish_location: None,
        };

        self.trust_anchors.push(anchor.clone());
        self.last_anchor_ms = now;

        // 记录锚点创建审计事件
        let mut metadata = HashMap::new();
        metadata.insert("anchor_id".to_string(), anchor.anchor_id.clone());
        metadata.insert("merkle_root".to_string(), anchor.anchor_merkle_root.clone());
        self.record(
            AuditEventType::TrustAnchorCreated,
            format!("创建信任锚点 {}", anchor.anchor_id),
            format!(
                "定期锚定：将当前审计链状态封装为不可篡改的信任锚点，共 {} 条事件",
                total_events
            ),
            vec![],
            metadata,
        );

        // 持久化锚点到文件（如果配置了路径）——追加式 JSONL，保留完整锚点历史
        if let Some(ref path) = self.anchor_config.anchor_persistence_path {
            if let Ok(json) = serde_json::to_string(&anchor) {
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(file, "{}", json);
                }
            }
        }

        anchor
    }

    /// 道枢映射: 离卦·火 (☲) — 继明照于四方，锚点链验证如双日并照之互证
    /// 验证锚点链的完整性
    ///
    /// 检查所有锚点是否按时间顺序排列，以及每个锚点的事件计数
    /// 是否单调递增（即后续锚点的事件数不应少于前一个锚点）。
    ///
    /// 返回 true 表示锚点链完整，false 表示检测到异常。
    pub fn verify_anchor_chain(&self) -> bool {
        if self.trust_anchors.is_empty() {
            return true; // 无锚点视为有效
        }

        // 修复（锚点验证闭环）：每个锚点封装的 last_event_hash 必须真正存在于
        // 审计链中——锚点必须对应一个真实可验证的链状态，
        // 防止"锚点引用了并不存在的哈希"（伪造/悬空锚点）逃过检测。
        // 2026-09-01 修复(P1)：与封印统一使用"内存 + JSONL 全量"检索，
        // 避免事件超过内存窗口（10000 条）后早期锚点被误判为悬空。
        for anchor in &self.trust_anchors {
            // 空哈希锚点为非法（create_anchor 只在链非空时才应产生有效哈希）
            if anchor.last_event_hash.is_empty() {
                eprintln!(
                    "[LRC·审计·锚点] 锚点 {} 的 last_event_hash 为空，非法锚点",
                    anchor.anchor_id
                );
                return false;
            }
            if !self.hash_exists_in_history(&anchor.last_event_hash) {
                eprintln!(
                    "[LRC·审计·锚点] 锚点 {} 的 last_event_hash {} 不在审计事件链中，锚点悬空",
                    anchor.anchor_id, anchor.last_event_hash
                );
                return false;
            }
        }

        // 验证锚点按时间顺序排列且事件计数单调递增
        for i in 1..self.trust_anchors.len() {
            let prev = &self.trust_anchors[i - 1];
            let curr = &self.trust_anchors[i];

            // 时间必须递增
            if curr.created_at_ms < prev.created_at_ms {
                eprintln!(
                    "[LRC·审计·锚点] 锚点链时间异常: {} 的时间戳 ({}) 早于 {} ({})",
                    curr.anchor_id, curr.created_at_ms, prev.anchor_id, prev.created_at_ms
                );
                return false;
            }

            // 事件计数必须单调递增（后续锚点不能比之前少）
            if curr.total_events_at_anchor < prev.total_events_at_anchor {
                eprintln!(
                    "[LRC·审计·锚点] 锚点链事件计数异常: {} 的事件数 ({}) 少于 {} ({})",
                    curr.anchor_id,
                    curr.total_events_at_anchor,
                    prev.anchor_id,
                    prev.total_events_at_anchor
                );
                return false;
            }
        }

        true
    }

    /// 道枢映射: 离卦·火 (☲) — 大人以继明照于四方，锚点发布如光明照耀外部
    /// 将锚点发布到外部（模拟外部见证）
    ///
    /// 在真实场景中，此方法会将锚点信息发送到外部见证服务
    /// （如区块链、公证服务等）。当前为模拟实现，仅标记锚点为已发布。
    ///
    /// 返回 true 表示发布成功，false 表示未找到指定锚点。
    pub fn publish_anchor(&mut self, anchor_id: &str, location: &str) -> bool {
        let now = current_time_ms();

        if let Some(anchor) = self
            .trust_anchors
            .iter_mut()
            .find(|a| a.anchor_id == anchor_id)
        {
            anchor.is_published = true;
            anchor.published_at_ms = Some(now);
            anchor.publish_location = Some(location.to_string());

            // 记录发布审计事件
            let mut metadata = HashMap::new();
            metadata.insert("anchor_id".to_string(), anchor_id.to_string());
            metadata.insert("publish_location".to_string(), location.to_string());
            self.record(
                AuditEventType::TrustAnchorPublished,
                format!("发布信任锚点 {} 到 {}", anchor_id, location),
                "将锚点发布到外部见证系统，确保审计链不可篡改".to_string(),
                vec![],
                metadata,
            );

            true
        } else {
            eprintln!("[LRC·审计·锚点] 未找到锚点: {}", anchor_id);
            false
        }
    }

    /// 获取所有信任锚点
    pub fn get_anchors(&self) -> &[TrustAnchor] {
        &self.trust_anchors
    }

    /// 获取锚点配置的不可变引用
    pub fn anchor_config(&self) -> &TrustAnchorConfig {
        &self.anchor_config
    }

    /// 获取锚点配置的可变引用（用于运行时调整）
    pub fn anchor_config_mut(&mut self) -> &mut TrustAnchorConfig {
        &mut self.anchor_config
    }

    /// 道枢映射: 离卦·火 (☲) — 双日并照，双人确认如双日之互证
    /// 请求关键操作的双人确认
    ///
    /// 对于关键操作（如批量删除记忆、修改衰减参数等），
    /// 需要第二人确认后才能执行，防止单个恶意内部人员或被盗账号
    /// 进行隐蔽的数据污染。
    ///
    /// 返回创建的待确认请求。
    pub fn request_dual_confirmation(
        &mut self,
        operation: &str,
        requested_by: &str,
    ) -> PendingConfirmation {
        let now = current_time_ms();
        let request_id = format!("dc_{:016}", self.pending_dual_confirmations.len() + 1);

        let pending = PendingConfirmation {
            request_id: request_id.clone(),
            operation: operation.to_string(),
            requested_by: requested_by.to_string(),
            requested_at_ms: now,
            status: ConfirmationStatus::Pending,
        };

        self.pending_dual_confirmations.push(pending.clone());

        // 记录双人确认请求审计事件
        let mut metadata = HashMap::new();
        metadata.insert("request_id".to_string(), request_id);
        metadata.insert("requested_by".to_string(), requested_by.to_string());
        self.record(
            AuditEventType::DualConfirmationRequested,
            format!("请求双人确认: {}", operation),
            format!(
                "关键操作「{}」需要第二人确认，由 {} 发起",
                operation, requested_by
            ),
            vec![],
            metadata,
        );

        pending
    }

    /// 道枢映射: 离卦·火 (☲) — 明两作，离，确认操作如双日之明照
    /// 第二人确认（或拒绝）操作
    ///
    /// 对指定的待确认请求进行确认或拒绝。只有状态为 Pending 的请求
    /// 才能被确认。确认后不可更改。
    ///
    /// 返回 true 表示操作成功，false 表示请求未找到或已处理。
    pub fn confirm_operation(
        &mut self,
        request_id: &str,
        granted: bool,
        confirmed_by: &str,
    ) -> bool {
        // 先查找并更新确认状态，在独立作用域内完成以避免借用冲突
        let (status_str, operation, event_type) = {
            if let Some(pending) = self
                .pending_dual_confirmations
                .iter_mut()
                .find(|p| p.request_id == request_id)
            {
                // 只能确认待处理状态的请求
                if pending.status != ConfirmationStatus::Pending {
                    eprintln!(
                        "[LRC·审计·双人确认] 请求 {} 已处理，当前状态: {:?}",
                        request_id, pending.status
                    );
                    return false;
                }

                pending.status = if granted {
                    ConfirmationStatus::Granted
                } else {
                    ConfirmationStatus::Denied
                };

                let status_str = if granted { "通过" } else { "拒绝" };
                let operation = pending.operation.clone();
                let event_type = if granted {
                    AuditEventType::DualConfirmationGranted
                } else {
                    AuditEventType::DualConfirmationDenied
                };

                (status_str, operation, event_type)
            } else {
                eprintln!("[LRC·审计·双人确认] 未找到请求: {}", request_id);
                return false;
            }
        }; // 借用在此结束

        // 记录确认审计事件（此时 self 不再被 pending 借用）
        let mut metadata = HashMap::new();
        metadata.insert("request_id".to_string(), request_id.to_string());
        metadata.insert("confirmed_by".to_string(), confirmed_by.to_string());
        metadata.insert("granted".to_string(), granted.to_string());

        self.record(
            event_type,
            format!("双人确认{}: {} → {}", status_str, operation, confirmed_by),
            format!(
                "关键操作「{}」由 {} 确认{}",
                operation, confirmed_by, status_str
            ),
            vec![],
            metadata,
        );

        true
    }

    /// 道枢映射: 离卦·火 (☲) — 明两作，自动锚定如定时之火照亮审计链
    /// 检查是否需要自动锚定
    ///
    /// 根据 anchor_config.auto_anchor_interval_ms 判断是否到了
    /// 下一次自动锚定的时间。如果距离上次锚定已超过设定间隔，
    /// 则自动创建新的信任锚点。
    ///
    /// 返回 true 表示本次创建了锚点，false 表示无需锚定。
    pub fn auto_anchor_check(&mut self) -> bool {
        // 2026-09-01 修复(P2)：审计链为空时不锚定（锚点必须封装真实链状态），
        // 且不消耗 last_anchor_ms——等链非空后再按间隔正常锚定。
        if self.last_hash.is_empty() {
            return false;
        }

        let now = current_time_ms();

        // 首次锚定：无条件创建
        if self.last_anchor_ms == 0 {
            self.create_anchor();
            return true;
        }

        // 检查是否超过自动锚定间隔
        if now - self.last_anchor_ms >= self.anchor_config.auto_anchor_interval_ms {
            self.create_anchor();
            return true;
        }

        false
    }

    /// 获取待双人确认的请求列表
    pub fn get_pending_confirmations(&self) -> &[PendingConfirmation] {
        &self.pending_dual_confirmations
    }

    /// 清理已处理的确认请求（保留最近 N 条）
    pub fn cleanup_confirmations(&mut self, keep_recent: usize) {
        let processed: Vec<_> = self
            .pending_dual_confirmations
            .iter()
            .filter(|p| p.status != ConfirmationStatus::Pending)
            .cloned()
            .collect();

        if processed.len() > keep_recent {
            // 保留最近的 keep_recent 条已处理请求
            let to_remove = processed.len() - keep_recent;
            self.pending_dual_confirmations
                .retain(|p| p.status == ConfirmationStatus::Pending);
            // 重新添加最近的 keep_recent 条
            for item in processed.into_iter().skip(to_remove) {
                self.pending_dual_confirmations.push(item);
            }
        }
    }
}

impl Default for AuditTrail {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(trail: &mut AuditTrail, event_type: AuditEventType, desc: &str) {
        trail.record(
            event_type,
            desc.to_string(),
            "test reason".to_string(),
            vec!["mem_001".to_string()],
            HashMap::new(),
        );
    }

    #[test]
    fn test_record_and_query_all() {
        let mut trail = AuditTrail::new();

        make_event(&mut trail, AuditEventType::SynthesisCreated, "合成记忆 A");
        make_event(&mut trail, AuditEventType::MemoryDeleted, "删除记忆 B");
        make_event(&mut trail, AuditEventType::GcCleanup, "GC 清理 3 条记忆");

        // 最新事件应在前
        let all = trail.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: None,
            memory_id: None,
            limit: None,
        });
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].event_type, AuditEventType::GcCleanup);
        assert_eq!(all[1].event_type, AuditEventType::MemoryDeleted);
        assert_eq!(all[2].event_type, AuditEventType::SynthesisCreated);
    }

    #[test]
    fn test_query_by_type() {
        let mut trail = AuditTrail::new();

        make_event(&mut trail, AuditEventType::SynthesisCreated, "合成 A");
        make_event(&mut trail, AuditEventType::GcCleanup, "GC 清理");
        make_event(&mut trail, AuditEventType::GcCleanup, "GC 清理 2");

        let gc_only = trail.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: Some(vec![AuditEventType::GcCleanup]),
            memory_id: None,
            limit: None,
        });
        assert_eq!(gc_only.len(), 2);
    }

    #[test]
    fn test_query_by_memory_id() {
        let mut trail = AuditTrail::new();

        trail.record(
            AuditEventType::MemoryDeleted,
            "删除 A".to_string(),
            "reason".to_string(),
            vec!["mem_A".to_string()],
            HashMap::new(),
        );
        trail.record(
            AuditEventType::SynthesisCreated,
            "合成 B".to_string(),
            "reason".to_string(),
            vec!["mem_B".to_string()],
            HashMap::new(),
        );

        let a_only = trail.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: None,
            memory_id: Some("mem_A".to_string()),
            limit: None,
        });
        assert_eq!(a_only.len(), 1);
        assert_eq!(a_only[0].event_type, AuditEventType::MemoryDeleted);
    }

    #[test]
    fn test_limit() {
        let mut trail = AuditTrail::new();
        for i in 0..10 {
            make_event(
                &mut trail,
                AuditEventType::SynthesisCreated,
                &format!("合成 {}", i),
            );
        }

        let limited = trail.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: None,
            memory_id: None,
            limit: Some(3),
        });
        assert_eq!(limited.len(), 3);
    }

    #[test]
    fn test_max_events_cap() {
        let mut trail = AuditTrail::new();
        trail.max_events = 5;

        for i in 0..10 {
            make_event(
                &mut trail,
                AuditEventType::SynthesisCreated,
                &format!("合成 {}", i),
            );
        }

        assert_eq!(trail.total_count(), 5, "应只保留最近 5 条");
    }

    #[test]
    fn test_type_statistics() {
        let mut trail = AuditTrail::new();
        make_event(&mut trail, AuditEventType::SynthesisCreated, "合成 A");
        make_event(&mut trail, AuditEventType::SynthesisCreated, "合成 B");
        make_event(&mut trail, AuditEventType::GcCleanup, "GC 清理");

        let stats = trail.type_statistics();
        assert_eq!(stats.get("synthesis_created").unwrap(), &2);
        assert_eq!(stats.get("gc_cleanup").unwrap(), &1);
    }

    #[test]
    fn test_empty_query() {
        let trail = AuditTrail::new();
        let results = trail.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: None,
            memory_id: None,
            limit: None,
        });
        assert!(results.is_empty());
    }

    /// 测试：JSONL 持久化 — 事件写入文件后可重新加载
    #[test]
    fn test_jsonl_persistence() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir
            .join("lrc_audit_test.jsonl")
            .to_string_lossy()
            .to_string();

        // 清理旧测试文件
        let _ = std::fs::remove_file(&file_path);

        // 创建带持久化的审计追踪器
        let mut trail = AuditTrail::new();
        trail.set_persist_path(&file_path).unwrap();

        // 记录事件
        make_event(&mut trail, AuditEventType::SynthesisCreated, "合成 A");
        make_event(&mut trail, AuditEventType::MemoryDeleted, "删除 B");
        make_event(&mut trail, AuditEventType::GcCleanup, "GC 清理");

        // 质疑三·性能：刷新异步缓冲区，确保事件已落盘
        trail.flush();

        // 验证文件存在
        assert!(
            std::path::Path::new(&file_path).exists(),
            "JSONL 文件应存在"
        );

        // 从文件重新加载
        let mut trail2 = AuditTrail::new();
        trail2.set_persist_path(&file_path).unwrap();

        assert_eq!(trail2.total_count(), 3, "应从文件加载 3 条事件");

        // 验证事件内容
        let all = trail2.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: None,
            memory_id: None,
            limit: None,
        });
        assert_eq!(all[2].event_type, AuditEventType::SynthesisCreated);
        assert_eq!(all[1].event_type, AuditEventType::MemoryDeleted);
        assert_eq!(all[0].event_type, AuditEventType::GcCleanup);

        // 清理测试文件
        let _ = std::fs::remove_file(&file_path);
    }

    /// 测试：JSONL 持久化 — 无路径时仅内存操作
    #[test]
    fn test_no_persist_path() {
        let mut trail = AuditTrail::new();

        make_event(&mut trail, AuditEventType::SynthesisCreated, "合成 A");
        assert_eq!(trail.total_count(), 1);
        // 无持久化路径时不应创建文件
    }

    /// 测试：JSONL 持久化 — 内存缓冲区溢出后文件仍保留完整历史
    #[test]
    fn test_persist_after_overflow() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir
            .join("lrc_audit_overflow_test.jsonl")
            .to_string_lossy()
            .to_string();

        let _ = std::fs::remove_file(&file_path);

        let mut trail = AuditTrail::new();
        trail.max_events = 3; // 小缓冲区，强制溢出
        trail.set_persist_path(&file_path).unwrap();

        // 记录 10 条事件（超出缓冲区）
        for i in 0..10 {
            make_event(
                &mut trail,
                AuditEventType::SynthesisCreated,
                &format!("合成 {}", i),
            );
        }

        // 质疑三·性能：刷新异步缓冲区，确保事件已落盘
        trail.flush();

        // 内存中只保留 3 条
        assert_eq!(trail.total_count(), 3);

        // 从文件重新加载：设置小缓冲区，验证截断行为
        let mut trail2 = AuditTrail::new();
        trail2.max_events = 3;
        trail2.set_persist_path(&file_path).unwrap();

        // 内存缓冲区仅保留 max_events 条
        assert_eq!(
            trail2.total_count(),
            3,
            "内存缓冲区仅保留 max_events 条，但文件中保留了全部 10 条历史"
        );

        // 加载到足够大的缓冲区，验证文件保留了全部 10 条
        let mut trail3 = AuditTrail::new();
        trail3.set_persist_path(&file_path).unwrap();
        assert_eq!(
            trail3.total_count(),
            10,
            "文件中应保留全部 10 条事件，即使内存缓冲区已溢出"
        );

        let _ = std::fs::remove_file(&file_path);
    }

    /// 测试：质疑四哈希链 — 完整性验证
    #[test]
    fn test_hash_chain_integrity() {
        let mut trail = AuditTrail::new();

        // 记录 5 条事件
        for i in 0..5 {
            trail.record(
                AuditEventType::SynthesisCreated,
                format!("合成 {}", i),
                "测试".to_string(),
                vec![format!("mem_{}", i)],
                HashMap::new(),
            );
        }

        // 验证哈希链完整性
        let result = trail.verify_integrity();
        assert!(result.is_valid, "哈希链应完整，但: {}", result.details);

        // 验证每个事件都有哈希
        for event in &trail.events {
            assert!(
                !event.event_hash.is_empty(),
                "事件 {} 缺少 event_hash",
                event.id
            );
        }

        // 验证第一条事件的 previous_hash 为空（创世事件）
        if let Some(first) = trail.events.last() {
            assert!(
                first.previous_hash.is_empty(),
                "创世事件应有空的 previous_hash"
            );
        }
    }

    /// 测试：质疑四哈希链 — 篡改检测
    #[test]
    fn test_hash_chain_tamper_detection() {
        let mut trail = AuditTrail::new();

        for i in 0..3 {
            trail.record(
                AuditEventType::SynthesisCreated,
                format!("合成 {}", i),
                "测试".to_string(),
                vec![format!("mem_{}", i)],
                HashMap::new(),
            );
        }

        // 验证初始完整性
        assert!(trail.verify_integrity().is_valid);

        // 模拟篡改：修改事件描述
        trail.events[0].description = "被篡改的描述".to_string();

        // 验证检测到篡改
        let result = trail.verify_integrity();
        assert!(!result.is_valid, "应检测到篡改");
        assert!(result.details.contains("哈希不匹配"), "应报告哈希不匹配");
    }

    // ============================================================
    // 质疑四"完美闭环悖论"：分布式信任锚点系统测试
    // ============================================================

    /// 测试：创建信任锚点并验证其字段
    #[test]
    fn test_create_anchor() {
        let mut trail = AuditTrail::new();

        // 先记录一些事件，确保有内容可锚定
        for i in 0..5 {
            make_event(
                &mut trail,
                AuditEventType::SynthesisCreated,
                &format!("合成 {}", i),
            );
        }

        let total_before = trail.total_written;

        // 创建锚点
        let anchor = trail.create_anchor();

        // 验证锚点字段
        assert_eq!(anchor.anchor_id, "anchor_0000000000000001");
        assert!(anchor.created_at_ms > 0, "锚点应包含创建时间戳");
        assert!(!anchor.last_event_hash.is_empty(), "锚点应包含最后事件哈希");
        assert_eq!(
            anchor.total_events_at_anchor, total_before,
            "锚点事件数应等于当前总事件数"
        );
        assert!(
            !anchor.anchor_merkle_root.is_empty(),
            "锚点应包含 Merkle 根"
        );
        assert!(!anchor.is_published, "新锚点不应已发布");
        assert!(anchor.published_at_ms.is_none(), "新锚点不应有发布时间");
        assert!(anchor.publish_location.is_none(), "新锚点不应有发布位置");

        // 验证锚点已被加入列表
        assert_eq!(trail.get_anchors().len(), 1);
        assert_eq!(trail.get_anchors()[0].anchor_id, "anchor_0000000000000001");

        // 验证锚点创建事件已被记录
        let anchor_events = trail.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: Some(vec![AuditEventType::TrustAnchorCreated]),
            memory_id: None,
            limit: None,
        });
        assert_eq!(anchor_events.len(), 1, "应记录一条锚点创建事件");
    }

    /// 测试：验证锚点链完整性
    #[test]
    fn test_verify_anchor_chain() {
        let mut trail = AuditTrail::new();

        // 创建多个锚点
        make_event(&mut trail, AuditEventType::SynthesisCreated, "事件 1");
        trail.create_anchor();

        make_event(&mut trail, AuditEventType::SynthesisCreated, "事件 2");
        trail.create_anchor();

        make_event(&mut trail, AuditEventType::SynthesisCreated, "事件 3");
        trail.create_anchor();

        // 验证锚点链完整
        assert!(trail.verify_anchor_chain(), "正常锚点链应通过验证");

        // 验证锚点数量
        assert_eq!(trail.get_anchors().len(), 3);

        // 验证事件计数单调递增
        let anchors = trail.get_anchors();
        for i in 1..anchors.len() {
            assert!(
                anchors[i].total_events_at_anchor >= anchors[i - 1].total_events_at_anchor,
                "锚点事件计数应单调递增"
            );
        }
    }

    /// 测试：空锚点链验证
    #[test]
    fn test_verify_anchor_chain_empty() {
        let trail = AuditTrail::new();
        // 空锚点链应视为有效
        assert!(trail.verify_anchor_chain(), "空锚点链应通过验证");
    }

    /// 测试：双人确认流程
    #[test]
    fn test_dual_confirmation_flow() {
        let mut trail = AuditTrail::new();

        // 启用双人确认
        trail.anchor_config_mut().require_dual_confirmation = true;

        // 请求双人确认
        let pending = trail.request_dual_confirmation("批量删除 100 条记忆", "user_001");
        assert_eq!(pending.status, ConfirmationStatus::Pending);
        assert_eq!(pending.requested_by, "user_001");
        assert!(pending.request_id.starts_with("dc_"));
        assert!(pending.requested_at_ms > 0);

        // 验证待确认列表
        assert_eq!(trail.get_pending_confirmations().len(), 1);

        // 第二人确认通过
        let result = trail.confirm_operation(&pending.request_id, true, "admin_001");
        assert!(result, "确认操作应成功");

        // 验证确认后状态
        let confirmations = trail.get_pending_confirmations();
        assert_eq!(confirmations[0].status, ConfirmationStatus::Granted);

        // 验证审计事件已记录
        let granted_events = trail.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: Some(vec![AuditEventType::DualConfirmationGranted]),
            memory_id: None,
            limit: None,
        });
        assert_eq!(granted_events.len(), 1, "应记录一条确认通过事件");

        // 验证重复确认被拒绝
        let dup_result = trail.confirm_operation(&pending.request_id, true, "admin_002");
        assert!(!dup_result, "已处理的请求不应再次确认");
    }

    /// 测试：双人确认拒绝流程
    #[test]
    fn test_dual_confirmation_denied() {
        let mut trail = AuditTrail::new();

        let pending = trail.request_dual_confirmation("修改衰减速率", "user_001");

        // 第二人拒绝
        let result = trail.confirm_operation(&pending.request_id, false, "admin_001");
        assert!(result, "拒绝操作应成功");

        let confirmations = trail.get_pending_confirmations();
        assert_eq!(confirmations[0].status, ConfirmationStatus::Denied);

        // 验证拒绝事件已记录
        let denied_events = trail.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: Some(vec![AuditEventType::DualConfirmationDenied]),
            memory_id: None,
            limit: None,
        });
        assert_eq!(denied_events.len(), 1, "应记录一条拒绝事件");
    }

    /// 测试：自动锚定机制
    #[test]
    fn test_auto_anchor() {
        let mut trail = AuditTrail::new();

        // 2026-09-01 修复(P2)：审计链为空时不应锚定（锚点必须封装真实链状态）
        assert!(
            !trail.auto_anchor_check(),
            "空审计链不应创建锚点（否则产生空哈希非法锚点）"
        );
        assert_eq!(trail.get_anchors().len(), 0, "空链不应产生锚点");

        // 记录若干事件后，审计链非空，锚定可用
        make_event(&mut trail, AuditEventType::SynthesisCreated, "测试事件一");
        make_event(&mut trail, AuditEventType::SynthesisCreated, "测试事件二");

        // 首次调用 auto_anchor_check 应创建锚点（last_anchor_ms == 0）
        assert!(trail.auto_anchor_check(), "链非空后首次应触发自动锚定");
        assert_eq!(trail.get_anchors().len(), 1, "应创建第一个锚点");

        // 立即再次调用不应创建锚点（间隔未到）
        assert!(!trail.auto_anchor_check(), "间隔未到不应触发锚定");
        assert_eq!(trail.get_anchors().len(), 1, "锚点数量不应增加");

        // 设置极短的锚定间隔（1 毫秒），模拟时间流逝
        trail.anchor_config_mut().auto_anchor_interval_ms = 0;
        // 重置 last_anchor_ms 以模拟时间已过
        trail.last_anchor_ms = 0;

        assert!(trail.auto_anchor_check(), "间隔满足后应触发锚定");
        assert_eq!(trail.get_anchors().len(), 2, "应创建第二个锚点");
    }

    /// 测试：锚点发布
    #[test]
    fn test_publish_anchor() {
        let mut trail = AuditTrail::new();

        make_event(&mut trail, AuditEventType::SynthesisCreated, "测试事件");
        let anchor = trail.create_anchor();

        // 发布锚点
        let result = trail.publish_anchor(&anchor.anchor_id, "区块链公证服务");
        assert!(result, "发布应成功");

        // 验证锚点状态
        let anchors = trail.get_anchors();
        assert!(anchors[0].is_published, "锚点应标记为已发布");
        assert!(anchors[0].published_at_ms.is_some(), "应有发布时间");
        assert_eq!(
            anchors[0].publish_location.as_deref(),
            Some("区块链公证服务"),
            "应有发布位置"
        );

        // 验证发布事件已记录
        let published_events = trail.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: Some(vec![AuditEventType::TrustAnchorPublished]),
            memory_id: None,
            limit: None,
        });
        assert_eq!(published_events.len(), 1, "应记录一条发布事件");

        // 测试发布不存在的锚点
        let bad_result = trail.publish_anchor("nonexistent", "某处");
        assert!(!bad_result, "发布不存在的锚点应失败");
    }

    /// 测试：清理已处理的确认请求
    #[test]
    fn test_cleanup_confirmations() {
        let mut trail = AuditTrail::new();

        // 创建多个确认请求
        let p1 = trail.request_dual_confirmation("操作 A", "user_001");
        let p2 = trail.request_dual_confirmation("操作 B", "user_001");

        // 确认第一个
        trail.confirm_operation(&p1.request_id, true, "admin_001");

        // 清理前应有 2 条（1 待处理 + 1 已处理）
        assert_eq!(trail.get_pending_confirmations().len(), 2);

        // 清理已处理请求，保留 0 条
        trail.cleanup_confirmations(0);

        // 清理后应只剩 1 条待处理
        assert_eq!(trail.get_pending_confirmations().len(), 1);
        assert_eq!(
            trail.get_pending_confirmations()[0].request_id,
            p2.request_id,
            "应保留待处理的请求"
        );
    }

    /// 回归：信任锚点必须跨重启持久化——`set_persist_path` 时从磁盘恢复历史锚点，
    /// 重启后 `verify_anchor_chain` 不得因锚点丢失而恒真。
    #[test]
    fn test_anchors_restored_across_restart() {
        let tmp_dir = std::env::temp_dir();
        let audit_path = tmp_dir
            .join("lrc_audit_anchor_restart_test.jsonl")
            .to_string_lossy()
            .to_string();
        // 清理可能存在的残留文件
        let _ = std::fs::remove_file(&audit_path);
        let anchor_path = format!("{}.anchors.jsonl", audit_path);
        let _ = std::fs::remove_file(&anchor_path);
        let seal_path = format!("{}.seal", audit_path);
        let _ = std::fs::remove_file(&seal_path);

        // 首次进程：记录事件 + 创建锚点
        {
            let mut trail = AuditTrail::new();
            trail.set_persist_path(&audit_path).expect("设置审计路径");
            for i in 0..3 {
                trail.record(
                    AuditEventType::GcCleanup,
                    format!("事件 {}", i),
                    "测试".to_string(),
                    vec![],
                    HashMap::new(),
                );
            }
            let anchor = trail.create_anchor();
            assert!(!anchor.last_event_hash.is_empty(), "锚点哈希不应为空");
            trail.flush();
        }

        // 模拟重启：新实例从同一路径加载
        {
            let mut trail2 = AuditTrail::new();
            trail2.set_persist_path(&audit_path).expect("重新设置路径");
            // 锚点必须被恢复（而不是空列表导致恒真）
            assert!(
                !trail2.get_anchors().is_empty(),
                "重启后应恢复历史锚点（P1：锚点跨重启持久化）"
            );
            // 恢复的锚点必须真实存在于审计链中
            assert!(trail2.verify_anchor_chain(), "恢复后的锚点链应通过验证");
        }

        // 清理
        let _ = std::fs::remove_file(&audit_path);
        let _ = std::fs::remove_file(&anchor_path);
        let _ = std::fs::remove_file(&seal_path);
    }

    /// 回归：canonical_hash_fields 的编码必须是单射——不同输入不得产生相同 canonical 串。
    #[test]
    fn test_canonical_hash_fields_injective() {
        // 使用 metadata 的 key/value 组合验证：值含分隔符时不得碰撞
        let mut m1 = HashMap::new();
        m1.insert("k".to_string(), "a=b".to_string());
        m1.insert("v".to_string(), "c".to_string());

        let mut m2 = HashMap::new();
        m2.insert("k".to_string(), "a".to_string());
        m2.insert("v".to_string(), "b=c".to_string());

        let (_a1, s1) = canonical_hash_fields(&[], &m1);
        let (_a2, s2) = canonical_hash_fields(&[], &m2);
        assert_ne!(s1, s2, "不同 metadata 不应产生相同 canonical 串");

        // affected 含分隔符时不得碰撞
        let (_b1, _) =
            canonical_hash_fields(&["a,b".to_string(), "c".to_string()], &HashMap::new());
        let (_b2, _) =
            canonical_hash_fields(&["a".to_string(), "b,c".to_string()], &HashMap::new());
        assert_ne!(_b1, _b2, "不同 affected 列表不应产生相同 canonical 串");
    }

    /// 回归：旧格式（hash_format 为空）事件在磁盘往返后仅计为"不可验证"软信号，
    /// 不判为哈希链断裂（修复 P1：旧格式兼容误报）。
    #[test]
    fn test_legacy_event_soft_signal_after_serialization() {
        let mut trail = AuditTrail::new();
        // 手工构造一个旧格式事件（hash_format 为空，模拟升级前落盘的事件）
        let legacy_input = format!(
            "{}|{}|{}|{}|{}|{}|{:?}|{:?}",
            "",
            "audit_legacy_1",
            1000u64,
            AuditEventType::RetrievalExecuted.as_str(),
            "desc",
            "reason",
            vec!["a".to_string()],
            {
                let mut meta = HashMap::new();
                meta.insert("k".to_string(), "v".to_string());
                meta
            },
        );
        let legacy_hash = trail.compute_hash(&legacy_input);
        trail.events.push(AuditEvent {
            id: "audit_legacy_1".to_string(),
            timestamp_ms: 1000,
            event_type: AuditEventType::RetrievalExecuted,
            description: "desc".to_string(),
            reason: "reason".to_string(),
            affected_memory_ids: vec!["a".to_string()],
            metadata: HashMap::new(), // 序列化往返后为空（模拟重建），顺序已不可复现
            previous_hash: String::new(),
            event_hash: legacy_hash,
            hash_format: String::new(), // 旧格式
        });
        // 旧格式事件未迁移：内容哈希用 canonical 重算必然失配 → 硬失败。
        // 这是 P0 修复的期望行为——旧格式日志不被信任，攻击者无法靠
        // 清空 hash_format 把真篡改降级为软信号绕过检测。
        let result = trail.verify_integrity();
        assert!(
            !result.is_valid,
            "未迁移的旧格式事件应判为哈希不匹配（硬失败），而非软信号放行"
        );
        assert_eq!(result.legacy_unverifiable, 1, "应计为旧格式事件");

        // 走迁移路径后，事件统一为 canonical_v2，canonical 重算应通过。
        let tmp_dir = std::env::temp_dir();
        let legacy_file = tmp_dir
            .join("lrc_audit_legacy_migrate_test.jsonl")
            .to_string_lossy()
            .to_string();
        {
            let mut file = std::fs::File::create(&legacy_file).expect("创建测试文件");
            use std::io::Write;
            let _ = writeln!(
                file,
                "{}",
                serde_json::to_string(&trail.events[0]).expect("序列化旧事件")
            );
        }
        let mut trail2 = AuditTrail::new();
        trail2
            .set_persist_path(&legacy_file)
            .expect("设置路径触发迁移");
        // 迁移后验证通过
        let migrated = trail2.verify_integrity();
        assert!(
            migrated.is_valid,
            "迁移后旧格式事件应可通过 canonical_v2 验证，detail: {}",
            migrated.details
        );
        let _ = std::fs::remove_file(&legacy_file);
        let _ = std::fs::remove_file(format!("{}.seal", legacy_file));
        let _ = std::fs::remove_file(format!("{}.anchors.jsonl", legacy_file));
    }

    /// 阶段D：检索执行审计事件记录与按类型查询。
    #[test]
    fn test_retrieval_executed_event() {
        let mut trail = AuditTrail::new();
        let mut meta = HashMap::new();
        meta.insert("query".to_string(), "Rust 编译失败 怎么排查".to_string());
        meta.insert("fast_weight".to_string(), "1.0".to_string());
        meta.insert("deep_weight".to_string(), "1.8".to_string());
        meta.insert("fast_hits".to_string(), "5".to_string());
        meta.insert("deep_hits".to_string(), "3".to_string());
        meta.insert("total_candidates".to_string(), "4".to_string());
        trail.record(
            AuditEventType::RetrievalExecuted,
            "联想检索执行：查询「Rust 编译失败」，融合 4 条候选".to_string(),
            "阶段D 联想解释观测".to_string(),
            vec!["mem_a".to_string(), "mem_b".to_string()],
            meta,
        );

        // 事件类型字符串
        assert_eq!(
            AuditEventType::RetrievalExecuted.as_str(),
            "retrieval_executed"
        );

        // 按类型查询
        let results = trail.query(&AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: Some(vec![AuditEventType::RetrievalExecuted]),
            memory_id: None,
            limit: None,
        });
        assert_eq!(results.len(), 1);
        let ev = &results[0];
        assert_eq!(ev.event_type, AuditEventType::RetrievalExecuted);
        assert_eq!(ev.metadata["query"], "Rust 编译失败 怎么排查");
        assert_eq!(ev.metadata["fast_weight"], "1.0");
        assert_eq!(ev.metadata["deep_weight"], "1.8");
        assert_eq!(
            ev.affected_memory_ids,
            vec!["mem_a".to_string(), "mem_b".to_string()]
        );

        // 哈希链保持完整
        assert!(trail.verify_integrity().is_valid);
    }

    // ============================================================
    // 回归修复：确定性序列化 / 完整性封印接线 / 锚点验证闭环
    // ============================================================

    /// 回归：带多键 metadata 的事件落盘后重新加载（反序列化重建 HashMap），
    /// 哈希重算必须一致——旧实现用 `{:?}` 依赖 HashMap 迭代顺序，
    /// 反序列化后的顺序与写入时不同，导致未篡改的日志被误判为哈希链断裂。
    #[test]
    fn test_hash_deterministic_across_reload() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir
            .join("lrc_audit_canonical_test.jsonl")
            .to_string_lossy()
            .to_string();
        let _ = std::fs::remove_file(&file_path);

        {
            let mut trail = AuditTrail::new();
            let mut meta = HashMap::new();
            meta.insert("query".to_string(), "Q1".to_string());
            meta.insert("fast_weight".to_string(), "1.0".to_string());
            meta.insert("deep_weight".to_string(), "1.8".to_string());
            meta.insert("total_candidates".to_string(), "4".to_string());
            trail.record(
                AuditEventType::RetrievalExecuted,
                "联想检索执行".to_string(),
                "阶段D 解释观测".to_string(),
                vec!["mem_a".to_string(), "mem_b".to_string()],
                meta,
            );
            // 落盘（手动写行，模拟既有 JSONL 文件）
            let line = serde_json::to_string(&trail.events[0]).unwrap();
            std::fs::write(&file_path, format!("{}\n", line)).unwrap();
        }

        // 模拟跨进程重启：新实例从文件加载并验证
        let mut trail2 = AuditTrail::new();
        trail2.load_from_file(&file_path).unwrap();
        assert!(
            trail2.verify_integrity().is_valid,
            "canonical 哈希输入应保证跨加载重算一致，详情: {}",
            trail2.verify_integrity().details
        );

        let _ = std::fs::remove_file(&file_path);
    }

    /// 兼容回退：升级前落盘的历史文件（旧 `{:?}` 哈希格式）在升级后
    /// 不应被直接判定为篡改，而是由一次性迁移重算为 canonical_v2。
    #[test]
    fn test_hash_legacy_backward_compat() {
        let mut trail = AuditTrail::new();
        let mut meta = HashMap::new();
        meta.insert("k1".to_string(), "v1".to_string());
        meta.insert("k2".to_string(), "v2".to_string());
        let previous_hash = String::new();
        let id = "audit_0000000000000001".to_string();
        let ts = 12345u64;
        // 按旧格式计算哈希（升级前代码的 hash_input 构造方式）
        let legacy_input = format!(
            "{}|{}|{}|{}|{}|{}|{:?}|{:?}",
            previous_hash,
            id,
            ts,
            "retrieval_executed",
            "desc",
            "reason",
            vec!["a".to_string()],
            meta
        );
        let legacy_hash = trail.compute_hash(&legacy_input);

        let event = AuditEvent {
            id,
            timestamp_ms: ts,
            event_type: AuditEventType::RetrievalExecuted,
            description: "desc".to_string(),
            reason: "reason".to_string(),
            affected_memory_ids: vec!["a".to_string()],
            metadata: meta,
            previous_hash,
            event_hash: legacy_hash,
            hash_format: String::new(), // 旧格式事件：无版本标记
        };
        trail.events.clear();
        trail.events.push(event);
        trail.recover_last_hash();

        // P0 修复语义：旧格式事件在未迁移前，canonical 重算失配 → 硬失败，
        // 不因"可能是旧格式"而静默放行（攻击者无法靠清空 hash_format 绕过）。
        let before = trail.verify_integrity();
        assert!(
            !before.is_valid,
            "未迁移的旧格式事件不应直接通过内容哈希验证（P0：不得软信号放行）"
        );

        // 一次性迁移：写入 JSONL 后经 set_persist_path 触发迁移，验证转为通过
        let tmp_dir = std::env::temp_dir();
        let legacy_file = tmp_dir
            .join("lrc_audit_legacy_compat_test.jsonl")
            .to_string_lossy()
            .to_string();
        {
            let mut file = std::fs::File::create(&legacy_file).expect("创建测试文件");
            use std::io::Write;
            let _ = writeln!(
                file,
                "{}",
                serde_json::to_string(&trail.events[0]).expect("序列化旧事件")
            );
        }
        let mut trail2 = AuditTrail::new();
        trail2
            .set_persist_path(&legacy_file)
            .expect("设置路径触发迁移");
        let after = trail2.verify_integrity();
        assert!(
            after.is_valid,
            "迁移后旧格式事件应改判为 canonical_v2 并通过验证，detail: {}",
            after.details
        );
        let _ = std::fs::remove_file(&legacy_file);
        let _ = std::fs::remove_file(format!("{}.seal", legacy_file));
        let _ = std::fs::remove_file(format!("{}.anchors.jsonl", legacy_file));
    }

    /// 回归：完整性封印必须随哈希链推进落盘（seal 文件 = 最新链根），
    /// 重启加载后封印验证通过。
    #[test]
    fn test_integrity_seal_roundtrip() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir
            .join("lrc_audit_seal_test.jsonl")
            .to_string_lossy()
            .to_string();
        let seal_path = format!("{}.seal", file_path);
        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_file(&seal_path);

        {
            let mut trail = AuditTrail::new();
            trail.set_persist_path(&file_path).unwrap();
            make_event(&mut trail, AuditEventType::SynthesisCreated, "A");
            make_event(&mut trail, AuditEventType::MemoryDeleted, "B");
            trail.flush();

            assert!(
                std::path::Path::new(&seal_path).exists(),
                "封印文件应已生成"
            );
            let stored = std::fs::read_to_string(&seal_path)
                .unwrap()
                .trim()
                .to_string();
            assert_eq!(stored, trail.last_hash, "封印文件应始终反映最新哈希链根");
            assert!(trail.verify_integrity_with_seal(), "封印应验证通过");
        }

        // 重启：加载事件 + 封印，验证通过
        {
            let mut trail2 = AuditTrail::new();
            trail2.set_persist_path(&file_path).unwrap();
            assert_eq!(trail2.total_count(), 2);
            assert!(trail2.seal_verified(), "重启后封印应有验证结果");
            assert!(trail2.verify_integrity_with_seal(), "重启后封印验证应通过");
        }

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_file(&seal_path);
    }

    /// 回归：封印落后于最新链根（封印指向链上较早事件哈希）时，
    /// 属于"封印后链正常增长"，不应误报篡改，且应刷新封印。
    #[test]
    fn test_seal_chain_growth_no_false_alarm() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir
            .join("lrc_audit_seal_growth_test.jsonl")
            .to_string_lossy()
            .to_string();
        let seal_path = format!("{}.seal", file_path);
        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_file(&seal_path);

        let mut trail = AuditTrail::new();
        trail.set_persist_path(&file_path).unwrap();
        make_event(&mut trail, AuditEventType::SynthesisCreated, "A");
        make_event(&mut trail, AuditEventType::MemoryDeleted, "B");
        trail.flush();

        let latest = trail.last_hash.clone();
        // events[1] = 较早的 A 事件（events[0] 为最新 B）
        let older_hash = trail.events[1].event_hash.clone();
        assert_ne!(older_hash, latest);
        // 手工制造封印滞后
        std::fs::write(&seal_path, &older_hash).unwrap();

        assert!(
            trail.verify_integrity_with_seal(),
            "封印指向链上较早事件属于链增长，不应误报篡改"
        );
        // 封印应被刷新到最新链根
        let stored = std::fs::read_to_string(&seal_path)
            .unwrap()
            .trim()
            .to_string();
        assert_eq!(stored, latest, "链增长场景下封印应刷新到最新哈希链根");

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_file(&seal_path);
    }

    /// 回归：封印不指向链上任何事件哈希（悬空值）时应判定篡改。
    #[test]
    fn test_seal_tamper_detected() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir
            .join("lrc_audit_seal_tamper_test.jsonl")
            .to_string_lossy()
            .to_string();
        let seal_path = format!("{}.seal", file_path);
        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_file(&seal_path);

        let mut trail = AuditTrail::new();
        trail.set_persist_path(&file_path).unwrap();
        make_event(&mut trail, AuditEventType::SynthesisCreated, "A");
        trail.flush();
        // 伪造封印：不存在的哈希
        std::fs::write(&seal_path, "deadbeefdeadbeef").unwrap();

        assert!(
            !trail.verify_integrity_with_seal(),
            "封印值不在链上应判定为篡改"
        );

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_file(&seal_path);
    }

    /// 回归：锚点验证闭环——锚点封装的 last_event_hash 必须是链上真实
    /// 哈希；被篡改为悬空哈希时应被检测。
    #[test]
    fn test_verify_anchor_chain_tamper_detection() {
        let mut trail = AuditTrail::new();
        make_event(&mut trail, AuditEventType::SynthesisCreated, "事件 1");
        trail.create_anchor();
        assert!(trail.verify_anchor_chain(), "正常锚点链应通过验证");

        // 篡改锚点封装的 last_event_hash（悬空引用，不在链上）
        trail.trust_anchors[0].last_event_hash = "deadbeefdeadbeef".to_string();
        assert!(
            !trail.verify_anchor_chain(),
            "悬空锚点（引用链上不存在的哈希）应被检测"
        );
    }

    // ============================================================
    // v0.9.8：「清除联想足迹」修复——隐藏集合机制
    // ============================================================

    /// 「清除联想足迹」：隐藏后 query 不再返回，但**哈希链必须完好**。
    ///
    /// 这是修复的核心判据：用户的隐私诉求（不再看到）与审计的完整性
    /// （不可篡改）必须**同时满足**——原实现只清内存，破坏了后者没保住前者。
    #[test]
    fn test_hide_matching_hides_from_query_but_keeps_chain() {
        let mut trail = AuditTrail::new();
        make_event(&mut trail, AuditEventType::RetrievalExecuted, "检索 A");
        make_event(&mut trail, AuditEventType::RetrievalExecuted, "检索 B");
        make_event(&mut trail, AuditEventType::SynthesisCreated, "合成 C");

        let q = AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: Some(vec![AuditEventType::RetrievalExecuted]),
            memory_id: None,
            limit: None,
        };
        assert_eq!(trail.query(&q).len(), 2, "隐藏前应能查到 2 条检索记录");

        let n = trail.hide_matching(|e| e.event_type == AuditEventType::RetrievalExecuted);
        assert_eq!(n, 2, "应隐藏 2 条");
        assert_eq!(trail.hidden_count(), 2);

        assert_eq!(trail.query(&q).len(), 0, "隐藏后不应再查到检索记录");

        // ★关键：其他类型不受影响，且完整性校验仍通过
        let q_all = AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: None,
            memory_id: None,
            limit: None,
        };
        assert_eq!(trail.query(&q_all).len(), 1, "未隐藏的合成事件应仍然可见");
        assert!(
            trail.verify_integrity().is_valid,
            "隐藏不得破坏哈希链（事件本体未被修改）：{}",
            trail.verify_integrity().details
        );
    }

    /// ★★ 决定性：隐藏集合**落盘**，模拟跨进程重启后**不复活**。
    ///
    /// 这正是用户报告的缺陷场景：原实现下重启会 load_from_file 把记录灌回内存，
    /// 用户"清除了"但重启后全部回来。本测试锁定修复后的行为。
    #[test]
    fn test_hidden_survives_restart() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir
            .join("lrc_audit_hidden_restart_test.jsonl")
            .to_string_lossy()
            .to_string();
        let hidden_path = format!("{}.hidden", file_path);
        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_file(&hidden_path);
        let _ = std::fs::remove_file(format!("{}.seal", file_path));
        let _ = std::fs::remove_file(format!("{}.anchors.jsonl", file_path));

        let q = AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: Some(vec![AuditEventType::RetrievalExecuted]),
            memory_id: None,
            limit: None,
        };

        // 第一次运行：写 3 条检索记录并隐藏
        {
            let mut trail = AuditTrail::new();
            trail.set_persist_path(&file_path).unwrap();
            make_event(&mut trail, AuditEventType::RetrievalExecuted, "检索 1");
            make_event(&mut trail, AuditEventType::RetrievalExecuted, "检索 2");
            make_event(&mut trail, AuditEventType::RetrievalExecuted, "检索 3");
            trail.flush();
            assert_eq!(trail.query(&q).len(), 3);

            let n = trail.hide_matching(|e| e.event_type == AuditEventType::RetrievalExecuted);
            assert_eq!(n, 3);
            assert_eq!(trail.query(&q).len(), 0, "隐藏后当前进程应为 0 条");
        }

        // 模拟重启：新实例从磁盘加载
        {
            let mut trail2 = AuditTrail::new();
            trail2.set_persist_path(&file_path).unwrap();
            assert_eq!(
                trail2.total_count(),
                3,
                "磁盘事件本体应仍完整（append-only，未删除）"
            );
            assert_eq!(trail2.hidden_count(), 3, "隐藏集合应从 <audit>.hidden 恢复");
            assert_eq!(
                trail2.query(&q).len(),
                0,
                "★重启后仍应为 0 条（修复前此处会复活为 3 条）"
            );
            assert!(
                trail2.verify_integrity().is_valid,
                "重启加载后哈希链仍应完好：{}",
                trail2.verify_integrity().details
            );
        }

        for p in [
            &file_path,
            &hidden_path,
            &format!("{}.seal", file_path),
            &format!("{}.anchors.jsonl", file_path),
        ] {
            let _ = std::fs::remove_file(p);
        }
    }

    /// 取消隐藏可恢复可见（误操作回退），且同样落盘。
    #[test]
    fn test_unhide_all_restores_visibility() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir
            .join("lrc_audit_unhide_test.jsonl")
            .to_string_lossy()
            .to_string();
        let hidden_path = format!("{}.hidden", file_path);
        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_file(&hidden_path);
        let _ = std::fs::remove_file(format!("{}.seal", file_path));
        let _ = std::fs::remove_file(format!("{}.anchors.jsonl", file_path));

        let q = AuditQuery {
            from_ms: None,
            to_ms: None,
            event_types: Some(vec![AuditEventType::RetrievalExecuted]),
            memory_id: None,
            limit: None,
        };

        let mut trail = AuditTrail::new();
        trail.set_persist_path(&file_path).unwrap();
        make_event(&mut trail, AuditEventType::RetrievalExecuted, "检索 1");
        trail.flush();
        trail.hide_matching(|e| e.event_type == AuditEventType::RetrievalExecuted);
        assert_eq!(trail.query(&q).len(), 0);

        let restored = trail.unhide_all();
        assert_eq!(restored, 1);
        assert_eq!(trail.query(&q).len(), 1, "取消隐藏后应恢复可见");
        assert_eq!(trail.hidden_count(), 0);

        for p in [
            &file_path,
            &hidden_path,
            &format!("{}.seal", file_path),
            &format!("{}.anchors.jsonl", file_path),
        ] {
            let _ = std::fs::remove_file(p);
        }
    }
}
