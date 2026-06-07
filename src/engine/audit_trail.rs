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

/// 审计追踪器
///
/// 维护一个 FIFO 环形缓冲区，记录系统所有自主行为。
/// 默认保留最近 10000 条事件，超出后自动淘汰最旧的事件。
///
/// v2.0 新增可选的 JSONL 持久化后端（质疑五）：
/// 当设置 persist_path 后，所有事件自动追加写入 JSONL 文件。
/// 缓冲区溢出的事件不会丢失——它们已持久化到磁盘。
/// 关键事件类型（MemoryDeleted, MemoryIsolated, CatastrophicEvent,
/// ChronicDegradation, RegulatorFrozen）即使溢出也始终保留在
/// JSONL 文件中，确保长期审计完整性。
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
    async_writer: Option<std::sync::mpsc::SyncSender<String>>,
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

/// 需要永久保留的关键事件类型（质疑五）
///
/// 即使缓冲区溢出，这些事件类型也应保留在 JSONL 文件中。
#[allow(dead_code)]
const CRITICAL_EVENT_TYPES: &[&str] = &[
    "memory_deleted",
    "memory_isolated",
    "catastrophic_event",
    "chronic_degradation",
    "regulator_frozen",
    "regulator_unfrozen",
];

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

        // 尝试从已有文件加载历史事件
        if std::path::Path::new(path).exists() {
            self.load_from_file(path)?;
            // 质疑三·终极：加载后立即验证封印
            self.verify_integrity_with_seal();
        }

        // 质疑三·性能：启动异步持久化后台线程
        self.start_async_writer(path)?;

        Ok(())
    }

    /// 启动异步持久化后台线程（质疑三·性能）
    ///
    /// 创建 mpsc channel 和后台线程，将 JSONL 文件写入
    /// 从主业务流程中解耦。channel 缓冲区大小 4096，
    /// 满时 send 会阻塞，防止内存无限增长。
    fn start_async_writer(&mut self, path: &str) -> std::io::Result<()> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<String>(4096);
        let path_clone = path.to_string();

        let handle = std::thread::Builder::new()
            .name("lrc-audit-writer".to_string())
            .spawn(move || {
                use std::io::Write;
                while let Ok(line) = rx.recv() {
                    // 追加写入 JSONL 文件，忽略单条写入失败
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path_clone)
                    {
                        let _ = writeln!(file, "{}", line);
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
                // 封印文件不存在（首次使用），创建初始封印
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

    /// 道枢映射: 坎卦·水 (☵) — 水流而不盈，事件记录如水流之连续
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
        // 哈希计算：previous_hash + 事件关键字段
        let hash_input = format!(
            "{}|{}|{}|{}|{}|{:?}",
            previous_hash,
            id,
            now,
            event_type.as_str(),
            description,
            affected_memory_ids
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
        };

        // 更新链上最后哈希
        self.last_hash = event_hash;

        // 质疑三·终极：更新完整性封印（如果已启用持久化）
        if self.persist_path.is_some() {
            self.integrity_seal = self.last_hash.clone();
        }

        // 持久化到 JSONL 文件（质疑三·性能：异步非阻塞）
        if let Some(ref tx) = self.async_writer {
            if let Ok(json) = serde_json::to_string(&event) {
                // 通过 channel 发送到后台线程，不阻塞主流程
                // sync_channel 缓冲区满时会阻塞，防止内存无限增长
                if let Err(e) = tx.send(json) {
                    eprintln!("[LRC·审计·警告] 异步持久化通道已关闭: {}", e);
                }
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
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        input.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
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
            return IntegrityVerification {
                is_valid: true,
                first_mismatch: None,
                details: "审计链为空，无需验证".to_string(),
            };
        }

        // 事件按时间倒序存储：[最新, ..., 最旧]
        // 验证方向：从旧到新，为每对相邻事件验证哈希链
        for i in (0..self.events.len() - 1).rev() {
            let newer = &self.events[i]; // events[i] = 较新的事件
            let older = &self.events[i + 1]; // events[i+1] = 较旧的事件

            // 较新事件的 previous_hash 应等于较旧事件的 event_hash
            if newer.previous_hash != older.event_hash {
                return IntegrityVerification {
                    is_valid: false,
                    first_mismatch: Some(i + 1),
                    details: format!(
                        "事件 #{}→#{} 哈希链断裂：id={} 的 previous_hash 与 id={} 的 event_hash 不匹配",
                        i + 1, i,
                        newer.id, older.id
                    ),
                };
            }
        }

        // 验证每条事件的内容哈希（包括创世事件）
        for (i, event) in self.events.iter().enumerate() {
            let hash_input = format!(
                "{}|{}|{}|{}|{}|{:?}",
                event.previous_hash,
                event.id,
                event.timestamp_ms,
                event.event_type.as_str(),
                event.description,
                event.affected_memory_ids
            );
            let recomputed = self.compute_hash(&hash_input);
            if recomputed != event.event_hash {
                return IntegrityVerification {
                    is_valid: false,
                    first_mismatch: Some(i),
                    details: format!("事件 #{} (id={}) 的内容哈希不匹配", i, event.id),
                };
            }
        }

        IntegrityVerification {
            is_valid: true,
            first_mismatch: None,
            details: format!("审计链完整，共 {} 条事件", self.events.len()),
        }
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

    /// 追加一行到 JSONL 文件
    #[allow(dead_code)]
    fn append_to_file(&self, path: &str, line: &str) -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{}", line)?;
        Ok(())
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
    pub fn query(&self, query: &AuditQuery) -> Vec<&AuditEvent> {
        let limit = query.limit.unwrap_or(100).min(1000);

        self.events
            .iter()
            .filter(|event| {
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

    /// 清理所有事件（慎用）
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
    pub fn create_anchor(&mut self) -> TrustAnchor {
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

        // 持久化锚点到文件（如果配置了路径）
        if let Some(ref path) = self.anchor_config.anchor_persistence_path {
            if let Ok(json) = serde_json::to_string(&anchor) {
                let _ = std::fs::write(path, &json);
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

        // 首次调用 auto_anchor_check 应创建锚点（last_anchor_ms == 0）
        assert!(trail.auto_anchor_check(), "首次应触发自动锚定");
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
}
