//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现记忆图存储，属于公开层 (Layer 1)。
//! ============================================================
//!
//! 记忆图存储（Graph Memory Store）
//!
//! 图数据库的轻量替代方案 — 在 JSON 持久化层之上实现
//! 记忆之间的关系边（contradicts / evolves / synthesizes_from / related_to）。
//!
//! 后续可平滑迁移到 Neo4j 或其它图数据库。

use crate::persistence::PersistenceError;
use serde::{Deserialize, Serialize};

/// 记忆关系类型
///
/// 洛书图结构中的边类型。
///
/// # 两组关系的来源不同（不可混同）
///
/// **第一组：图存储内生的（v0.6.0 起的合成/冲突链路）**
/// 由 `synthesis_engine` 与写入冲突检查产出：`Contradicts` / `Evolves` /
/// `SynthesizesFrom` / `RelatedTo`。
///
/// **第二组：记录层关系（v0.9.8 起接入）**
/// 由 `MemoryStore::associations_in` 的确定性规则产出，**全部来自已有记录字段**，
/// 不依赖任何语义计算。它们的语义与第一组**证据性质不同**——
/// 前者是系统推断（可能错），后者是记录事实（不会错），
/// 故必须是独立类型，不得合并（承「证据要可区分」纪律）。
///
/// **为什么要有这一组**：`associations_in` 的产出此前只存在于内存（一次检索的
/// 返回值），检索结束即消失；图化后关系**跨会话累积**，才可能做多跳与关系统计。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    /// 矛盾关系：两条记忆内容冲突
    Contradicts,
    /// 演进关系：新记忆是旧记忆的更新/演进版本
    Evolves,
    /// 合成来源：合成记忆来源于多条源记忆
    SynthesizesFrom,
    /// 一般关联：语义相关但非直接衍生
    RelatedTo,

    // ===== 记录层关系（v0.9.8 接入；与 `MemoryStore::associations_in` 一一对应）=====
    /// 同一次经历：`event_id` 相同（**知情者断言**，证据最强）
    SameEvent,
    /// 同期工作记录：同项目 + 同时间窗（**系统统计推断**，证据弱）
    ///
    /// 与 `SameEvent` 必须分开：前者是事实断言，后者是统计推断，
    /// 混用会让用户把推断当断言（承 `same_event_auto` 的拆分理由）。
    SameEventAuto,
    /// 共享实体：`entities` 中同名同类型的实体（跳过 hub 实体）
    SharedEntity,
    /// 共享具体产物：正文中出现的同一标识符（如文件名），**形态检出**
    SharedArtifact,
    /// 结晶来源：本记忆由目标记忆衍生（有向）
    DerivedFrom,
    /// 被结晶为：目标记忆由本记忆衍生而来（有向，`DerivedFrom` 的反向）
    CrystallizedInto,
    /// 被更新过：本记忆存在历史版本（自指）
    EvolvedFrom,

    // ===== 逻辑关系层（v0.9.8 接入；承《记忆联想系统设计文档》§5.3）=====
    //
    // **与上面两组的本质区别**（三方来源不可混同）：
    //   · 第一组（图存储内生）：由合成/冲突链路产出，语义是"系统推断"
    //   · 第二组（记录层）：由 `associations_in` 产出，语义是"记录事实"
    //   · 本组（逻辑关系）：由**结构算子候选**产出（§4.3 互/错/综/变），
    //     语义是"从当前框架**变换**出的关联视角"——它是**生成性**的，
    //     前两组是"检索已存在的关系"，本组是"推导可能的关系"。
    //
    // **为什么单列一组而非复用**：`Contradicts` 与 `CONSTRAINT`、
    //   `Evolves` 与 `TEMPORAL` 表面相近，实则不同——
    //   前者是"两条内容冲突/演进"（已发生的事实），
    //   后者是"一条限制/先于另一条"（结构性关系）。
    //   合并会让前端无法区分"已知冲突"与"结构约束"，承 §5.3 的设计意图。
    /// 因果：A 引发 B（有向）
    Cause,
    /// 时序：A 先于 B（有向）
    Temporal,
    /// 约束：A 限制 B 的取值域（有向）
    Constraint,
    /// 促进：A 提高 B 的概率/程度（有向）
    Facilitate,
    /// 并列同源：A、B 同属一个框架（**无向**）
    Coordinate,
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Contradicts => "contradicts",
            Self::Evolves => "evolves",
            Self::SynthesizesFrom => "synthesizes_from",
            Self::RelatedTo => "related_to",
            Self::SameEvent => "same_event",
            Self::SameEventAuto => "same_event_auto",
            Self::SharedEntity => "shared_entity",
            Self::SharedArtifact => "shared_artifact",
            Self::DerivedFrom => "derived_from",
            Self::CrystallizedInto => "crystallized_into",
            Self::EvolvedFrom => "evolved_from",
            Self::Cause => "cause",
            Self::Temporal => "temporal",
            Self::Constraint => "constraint",
            Self::Facilitate => "facilitate",
            Self::Coordinate => "coordinate",
        }
    }

    /// 从**外部推导入口**（`/v1/memories/external-edge`）解析允许的关系名。
    ///
    /// ## ★为什么必须与 `from_relation_str` 分开（2026-09-18 修 S3）
    ///
    /// `from_relation_str` 接受**全部 12 类**关系（它是通用解析器）。但外部
    /// 端点的**安全边界**只允许 §5.3 的 5 类逻辑关系：
    ///
    /// | 组 | 类型 | 外部可否写入 | 理由 |
    /// |---|---|---|---|
    /// | 图存储内生 | `contradicts` `evolves` `synthesizes_from` `related_to` | ❌ | 由合成/冲突链路产出，外部伪造会污染系统推断 |
    /// | **记录层** | `same_event` `shared_entity` `derived_from` … | ❌ | ★**记录事实**（`same_event` 语义是"知情者断言"，证据最强）——外部进程无权断言 |
    /// | 逻辑关系（§5.3） | `cause` `temporal` `constraint` `facilitate` `coordinate` | ✅ | **推导性**关系，本就由结构算子产出 |
    ///
    /// ## 不加白名单的实测后果
    ///
    /// 此前实现直接调 `from_relation_str` ⇒ 外部进程可写入 `same_event`，
    /// 其权重按 `relation_priority` 为 **1.0**（最强），且会经
    /// `expand_associations` 并入 recall 输出、被渲染为"**由记录推导**、必然成立"。
    /// 即：任意本机进程都能把两条真实记忆伪造成"同一次经历"，
    /// 而用户看到的是最高证据等级的记录事实 ⇒ 击穿「证据要可区分」的整个设计。
    /// （注释原本也写着"只接受 5 类"，属**文档与实现不一致**。）
    ///
    /// ⇒ 返回 `None` 表示"该类型不被本端点接受"，调用方跳过（宁缺勿错）。
    pub fn from_external_rel_str(s: &str) -> Option<Self> {
        match Self::from_relation_str(s) {
            Some(
                e @ (Self::Cause
                | Self::Temporal
                | Self::Constraint
                | Self::Facilitate
                | Self::Coordinate),
            ) => Some(e),
            // 其余（图存储内生 4 类 + 记录层 7 类）一律不接受
            _ => None,
        }
    }

    /// 从记录层关系名（`MemoryStore` 的 `relation` 字符串）解析
    ///
    /// 返回 `None` 表示该关系名不属于记录层（或尚未支持）。
    /// **不做兜底映射**：把未知关系名硬塞进某个已知类型会让用户读到错误的
    /// 关系语义，返回 `None` 让调用方显式跳过更安全。
    pub fn from_relation_str(s: &str) -> Option<Self> {
        match s {
            "same_event" => Some(Self::SameEvent),
            "same_event_auto" => Some(Self::SameEventAuto),
            "shared_entity" => Some(Self::SharedEntity),
            "shared_artifact" => Some(Self::SharedArtifact),
            "derived_from" => Some(Self::DerivedFrom),
            "crystallized_into" => Some(Self::CrystallizedInto),
            "evolved_from" => Some(Self::EvolvedFrom),
            // 逻辑关系（§5.3）：接受**大写**（道体服务输出）与**小写**两种写法。
            // 为什么要兼容大写：Python 侧 `assoc_service` 的 rel_type 常量是
            // 大写（CAUSE/TEMPORAL/…），与 §5.3 表格逐字一致；而 Rust 侧
            // 惯例是小写。兼容两者可省去中间转换层（少一层即少一处漂移）。
            "cause" | "CAUSE" => Some(Self::Cause),
            "temporal" | "TEMPORAL" => Some(Self::Temporal),
            "constraint" | "CONSTRAINT" => Some(Self::Constraint),
            "facilitate" | "FACILITATE" => Some(Self::Facilitate),
            "coordinate" | "COORDINATE" => Some(Self::Coordinate),
            _ => None,
        }
    }

    /// 是否为**对称关系**（方向仅为书写顺序，不表示因果或先后）
    ///
    /// 采用**白名单**口径（与 `memory_store_types.rs` 的 `GraphEdge.symmetric`
    /// 同源）：未来新增关系类型若忘记分类，默认按**非对称**处理——
    /// 多画一个箭头是可见的，方向信息丢失是静默的，后者更危险。
    pub fn is_symmetric(&self) -> bool {
        matches!(
            self,
            Self::SameEvent
                | Self::SameEventAuto
                | Self::SharedEntity
                | Self::SharedArtifact
                | Self::RelatedTo
                // §5.3：并列同源（A、B 同属一个框架）**无向**——
                // 「在哪吃 ↔ 和谁吃」交换两端语义不变。
                // 其余四类逻辑关系（Cause/Temporal/Constraint/Facilitate）
                // 均为**有向**，方向即语义（承 §5.3 表格的"方向性"列）。
                | Self::Coordinate
        )
    }
}

/// 记忆图边
///
/// 连接两条记忆的有向关系边。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEdge {
    /// 边唯一标识
    pub id: String,
    /// 源记忆 ID
    pub source_id: String,
    /// 目标记忆 ID
    pub target_id: String,
    /// 关系类型
    pub edge_type: EdgeType,
    /// 关系权重（0.0 ~ 1.0，表示关联强度）
    pub weight: f32,
    /// 创建时间戳
    pub created_at: String,
}

impl MemoryEdge {
    /// 创建新的记忆边
    pub fn new(source_id: String, target_id: String, edge_type: EdgeType, weight: f32) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        Self {
            id,
            source_id,
            target_id,
            edge_type,
            weight: weight.clamp(0.0, 1.0),
            created_at,
        }
    }
}

/// 图查询结果
#[derive(Debug, Clone, Default)]
pub struct GraphQueryResult {
    /// 直接关联的记忆 ID 列表
    pub related_ids: Vec<String>,
    /// 演进链（从最旧到最新）
    pub evolution_chain: Vec<String>,
    /// 合成来源（Synthesis → 源记忆）
    pub synthesis_sources: Vec<String>,
    /// 子图大小（关联的记忆总数）
    pub subgraph_size: usize,
}

/// 记忆图存储
///
/// 在持久化层之上管理记忆之间的关系边。
/// 使用 JSON 文件持久化边数据。
///
/// # 落盘策略（v0.9.16：脏标记 + 后台 flush）
///
/// 图文件此前**每加一条边就整体重写一次**（`add_edge` / `add_edges_batch`），
/// 而 `expand_associations` 会在**每次检索**里写入记录层边 ⇒ 读路径每轮都要
/// 序列化并重写 52MB 的 `graph_edges.json`，且**全程持有全局存储锁**。
/// 实测这是"连续 81 秒 lock_busy、写入 3/12 成功"的最大突发源。
///
/// 现在分两条通道：
/// * **读路径**（`expand_associations`）走 [`Self::add_edges_batch_deferred`]：
///   只改内存 + 置脏标记，**不碰磁盘**；后台任务每 [`crate::graph_store::GRAPH_FLUSH_INTERVAL_SECS`]
///   秒把内存图整体落盘一次。
/// * **写路径**（`remember` / `forget` / `/external-edge`）仍走
///   [`Self::add_edges_batch`] 等**立即落盘**的方法，保持"写成功即持久化"的既有语义。
///
/// 代价：读路径产生的边最多延迟一个 flush 周期落盘；进程若在此期间被杀，
/// 这批边会丢失——但它们是**可从记忆记录重新推导**的派生数据（下次检索会再写入），
/// 故可接受。
pub struct GraphMemoryStore {
    /// 所有关系边
    edges: Vec<MemoryEdge>,
    /// 边持久化文件路径（相对于数据目录）
    edges_file: String,
    /// 增量日志文件路径（追加写，压实后清空）
    journal_file: String,
    /// 是否有**尚未落盘**的内存变更（仅由 deferred 通道置位）
    dirty: bool,
    /// 自上次落盘以来**新增**的边（deferred 通道用；flush 时只追加这些）
    pending: Vec<MemoryEdge>,
    /// 去重索引：`(source, target, edge_type)`。
    ///
    /// 为什么需要：`merge_edges` 原实现逐条线性扫描 `edges`（O(E)）。
    /// 图规模到 10 万条时，单次检索写入 10 条边就是 100 万次比较；
    /// 到百万条时单次检索会被拖到百毫秒级。索引把去重降到 O(1)。
    edge_keys: std::collections::HashSet<(String, String, EdgeType)>,
}

impl GraphMemoryStore {
    /// 创建新的图存储实例
    pub fn new(data_dir: &str) -> Self {
        Self {
            edges: Vec::new(),
            edges_file: format!("{}/graph_edges.json", data_dir),
            journal_file: format!("{}/graph_edges.journal.jsonl", data_dir),
            dirty: false,
            pending: Vec::new(),
            edge_keys: std::collections::HashSet::new(),
        }
    }

    /// 重建去重索引（加载 / 删除 / 清空之后调用）。
    fn rebuild_edge_keys(&mut self) {
        self.edge_keys.clear();
        self.edge_keys.reserve(self.edges.len());
        for e in &self.edges {
            self.edge_keys.insert((
                e.source_id.clone(),
                e.target_id.clone(),
                e.edge_type.clone(),
            ));
        }
    }

    /// 增量日志的字节上限（超过即触发一次整图压实）。
    ///
    /// 默认 64MB，与主库 `LRC_JOURNAL_MAX_BYTES` 同口径；可用
    /// `LRC_GRAPH_JOURNAL_MAX_BYTES` 覆盖（下限 1MB）。
    fn journal_max_bytes() -> u64 {
        std::env::var("LRC_GRAPH_JOURNAL_MAX_BYTES")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v >= 1024 * 1024)
            .unwrap_or(64 * 1024 * 1024)
    }

    fn journal_len(&self) -> u64 {
        std::fs::metadata(&self.journal_file)
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// 本次落盘是否必须"整图压实"（写 `graph_edges.json` 并清空日志）。
    ///
    /// 与主库同纪律：基文件缺失时一律压实（首次写入即建基文件），
    /// 之后按日志阈值触发。
    fn needs_compact(&self) -> bool {
        !std::path::Path::new(&self.edges_file).exists()
            || self.journal_len() >= Self::journal_max_bytes()
    }

    /// 把一批边追加到日志（每条一行紧凑 JSON）。
    ///
    /// 追加是 O(1)，不随图规模增长——这是本模块要解决的核心成本。
    fn append_journal(&self, edges: &[MemoryEdge]) -> Result<(), PersistenceError> {
        if edges.is_empty() {
            return Ok(());
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_file)
            .map_err(PersistenceError::Io)?;
        let mut buf = String::new();
        for edge in edges {
            buf.push_str(&serde_json::to_string(edge).map_err(PersistenceError::Serialization)?);
            buf.push('\n');
        }
        file.write_all(buf.as_bytes())
            .map_err(PersistenceError::Io)?;
        file.flush().map_err(PersistenceError::Io)?;
        // 追加日志是"已确认写入"的唯一副本，必须真正落盘。
        file.sync_data().map_err(PersistenceError::Io)?;
        Ok(())
    }

    /// 清空日志（整图压实 / 删除 / 清空之后调用）。
    fn truncate_journal(&self) {
        let _ = std::fs::remove_file(&self.journal_file);
    }

    /// 重放日志：把日志里的边合入内存（去重）。
    ///
    /// 日志行损坏时**跳过该行**而非整体失败——日志是派生数据，
    /// 单行损坏不应让整张图不可用（基文件仍是事实源）。
    fn replay_journal(&mut self) {
        let Ok(content) = std::fs::read_to_string(&self.journal_file) else {
            return;
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(edge) = serde_json::from_str::<MemoryEdge>(line) else {
                continue;
            };
            let key = (
                edge.source_id.clone(),
                edge.target_id.clone(),
                edge.edge_type.clone(),
            );
            if self.edge_keys.insert(key) {
                self.edges.push(edge);
            }
        }
    }

    /// 从文件加载已有边
    pub fn load(&mut self) -> Result<(), PersistenceError> {
        // 文件不存在（首次运行）→ 正常空状态
        // 文件存在但读取/解析失败 → 必须返回错误，防止后续 add_edge 用空状态覆盖原图数据
        match std::fs::read_to_string(&self.edges_file) {
            Ok(content) => {
                if !content.trim().is_empty() {
                    self.edges =
                        serde_json::from_str(&content).map_err(PersistenceError::Serialization)?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(PersistenceError::Io(e)),
        }
        // ★先为基文件建索引，再重放日志（重放时靠索引去重）。
        self.rebuild_edge_keys();
        // ★增量日志重放：基文件是快照，日志是自上次压实以来的增量。
        //   两者合并才是完整状态（与主库 `memories.json` + journal 同纪律）。
        self.replay_journal();
        Ok(())
    }

    /// 持久化边到文件
    ///
    /// `&mut self` 是因为成功落盘后要清掉 [`Self::dirty`]（v0.9.16）。
    pub fn save(&mut self) -> Result<(), PersistenceError> {
        let json =
            serde_json::to_string_pretty(&self.edges).map_err(PersistenceError::Serialization)?;
        // ★原子写（2026-09-18 审查修复）
        //
        // **为什么不能直接 `fs::write`**：图文件处在写入热路径上——
        // `expand_associations` 每次检索都可能触发 `save()`。直接覆盖写时若
        // 进程崩溃/断电，会留下**截断的 JSON** ⇒ 下次启动 `load()` 解析失败
        // （`load` 已正确地返回 Err 而非静默空图，但图仍不可用）。
        //
        // ★★为什么必须复用 `crate::atomic_file::write_atomic`（而非本文件自写）：
        //   初版在此手写了 `format!("{}.tmp", ...)` + write + rename，注释还写着
        //   "对齐同一纪律"——但那恰是 `atomic_file.rs` 模块文档**点名要消除**的写法：
        //     · 固定临时名 ⇒ 并发写同一路径时两方争用同一临时文件，
        //       可能一方 rename 到另一方写了一半的内容；
        //     · 失败时不清理 ⇒ 残留 `.tmp` 垃圾。
        //   `write_atomic` 用 UUID 临时名 + 失败清理，且已是本仓 6 处 JSON
        //   落盘点的统一实现（config/arch_config/data_dir/discovery/
        //   state_matcher/audit_trail）——graph_store 是**唯一遗漏**的一处。
        //   ⇒ 收敛到唯一实现，避免"同一个仓库两套写法"。
        crate::atomic_file::write_atomic(std::path::Path::new(&self.edges_file), json.as_bytes())
            .map_err(PersistenceError::Io)?;
        // ★整图已落盘 ⇒ 日志里的增量已被吸收，清空日志（否则下次启动会重复重放）。
        self.truncate_journal();
        // 已整体落盘 ⇒ 内存与磁盘一致
        self.dirty = false;
        self.pending.clear();
        Ok(())
    }

    /// 是否有尚未落盘的变更。
    pub fn has_pending_writes(&self) -> bool {
        self.dirty
    }

    /// 重新置脏（后台 flush 写盘失败时补回）。
    ///
    /// [`Self::take_dirty_snapshot`] 在返回快照时就清了脏标记；若随后的写盘
    /// 失败，必须由调用方把它补回来，否则这批边会一直留在内存、等下一次
    /// 检索重新产出才可能落盘（而那次可能永不发生）。
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// 若存在未落盘变更则落盘一次；返回是否真的写了盘。
    ///
    /// 供后台 flush 任务（`bin/server.rs`）与关停路径调用。
    /// 无变更时**不产生任何 I/O**（这是本模块要解决的核心成本）。
    pub fn flush_if_dirty(&mut self) -> Result<bool, PersistenceError> {
        if !self.dirty {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// 取一份**脏快照**并清掉脏标记（由调用方在**锁外**序列化 + 写盘）。
    ///
    /// # 为什么不在锁内直接 `save()`
    ///
    /// 图文件 52MB，序列化 + 写盘实测需数百毫秒。后台 flush 若全程持存储锁，
    /// 就只是把"每次检索写一次盘"换成"每 30 秒卡住一次"，写入仍会被阻塞。
    /// 故这里只做一次内存克隆（边是普通结构体，克隆在毫秒级），
    /// 真正的 I/O 交给 [`GraphSnapshot::write`] 在锁外完成。
    ///
    /// # 竞态说明
    ///
    /// 快照期间若有新的 deferred 写入，它会重新置脏 ⇒ 下一轮 flush 会再写一次，
    /// 不会丢边。若期间恰好有**写路径的立即落盘**（`add_edge` 等），
    /// 可能被这份较旧的快照覆盖——但两者都是派生数据，下一轮检索会重新写入，
    /// 属可接受的自愈代价（不为它引入全局序号 / 二次锁）。
    pub fn take_dirty_snapshot(&mut self) -> Option<GraphSnapshot> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(GraphSnapshot {
            path: std::path::PathBuf::from(&self.edges_file),
            edges: self.edges.clone(),
        })
    }

    /// 取一份**增量快照**并清掉脏标记（由调用方在**锁外**追加日志）。
    ///
    /// 与 [`Self::take_dirty_snapshot`] 的区别：本方法只带走**自上次落盘以来
    /// 新增的边**（`pending`），flush 时追加到日志即可，**不整图重写**。
    /// 只有当日志达到阈值（或基文件缺失）时，才附带一份整图快照用于压实。
    ///
    /// # 失败恢复
    ///
    /// 若调用方写盘失败，必须调用 [`Self::restore_pending`] 把边放回，
    /// 否则这批边会留在内存却永不落盘（直到下一次压实）。
    pub fn take_pending_snapshot(&mut self) -> Option<GraphPending> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        let pending = std::mem::take(&mut self.pending);
        let compact = if self.needs_compact() {
            Some((
                std::path::PathBuf::from(&self.edges_file),
                self.edges.clone(),
            ))
        } else {
            None
        };
        Some(GraphPending {
            journal_path: std::path::PathBuf::from(&self.journal_file),
            pending,
            compact,
        })
    }

    /// 写盘失败时把增量边放回（配合 [`Self::take_pending_snapshot`]）。
    pub fn restore_pending(&mut self, edges: Vec<MemoryEdge>) {
        if !edges.is_empty() {
            self.pending.extend(edges);
            self.dirty = true;
        }
    }

    /// 添加一条关系边
    ///
    /// 自动去重：相同 source_id + target_id + edge_type 的边不会重复添加。
    pub fn add_edge(
        &mut self,
        source_id: &str,
        target_id: &str,
        edge_type: EdgeType,
        weight: f32,
    ) -> Result<(), PersistenceError> {
        // 去重检查（O(1) 索引）
        let key = (
            source_id.to_string(),
            target_id.to_string(),
            edge_type.clone(),
        );
        if !self.edge_keys.contains(&key) {
            let edge = MemoryEdge::new(
                source_id.to_string(),
                target_id.to_string(),
                edge_type,
                weight,
            );
            self.edge_keys.insert(key);
            self.edges.push(edge.clone());
            // ★增量写：追加日志（O(1)），不再整图重写。
            //   基文件缺失或日志超阈值时改为整图压实（与主库同纪律）。
            //   失败时回滚内存，保证"返回 Err ⇔ 未写入"。
            let result = if self.needs_compact() {
                self.save()
            } else {
                self.append_journal(std::slice::from_ref(&edge))
            };
            if let Err(e) = result {
                self.edges.pop();
                self.edge_keys.remove(&(
                    edge.source_id.clone(),
                    edge.target_id.clone(),
                    edge.edge_type.clone(),
                ));
                return Err(e);
            }
        }

        Ok(())
    }

    /// 删除一条边
    pub fn remove_edge(&mut self, edge_id: &str) -> Result<bool, PersistenceError> {
        let len_before = self.edges.len();
        self.edges.retain(|e| e.id != edge_id);
        let removed = self.edges.len() < len_before;
        if removed {
            self.rebuild_edge_keys();
            self.save()?;
        }
        Ok(removed)
    }

    /// 删除与某条记忆**相关的一切边**（任一端命中），返回删除条数。
    ///
    /// ## ★为什么需要它（2026-09-18 审查 G3 修复）
    ///
    /// 图此前**只增不减**：`remove_edge` / `clear` 定义存在但**零生产调用**，
    /// 而 `forget` 删除记忆时**不触碰图** ⇒ 已删除的记忆其边**永久残留**，
    /// 形成"悬空边"。消费侧只能在读到边时靠 `by_id.get()` 查不到而跳过
    /// （见 `expand_associations` 与 `query_stored_edges`），
    /// 即**每轮检索都要为这些死边付出一次遍历与判空**，且成本随生命期单调增长。
    ///
    /// ## 为什么按"任一端命中"删除（而非只删出边）
    ///
    /// 边的语义是**两端记忆之间的关系**。任一端被删除，这条关系就**不再成立**
    /// （不存在"A 与已不存在的 B 相关"）。只删一端会留下一条指向空洞的边。
    ///
    /// ## 为什么批量（而非逐条 `remove_edge`）
    ///
    /// `remove_edge` 每次调用都 `save()`（全量序列化），逐条删是 O(E²) 写盘。
    /// 本方法只保存一次，与 `add_edges_batch` 的纪律一致。
    pub fn remove_edges_of_memory(&mut self, memory_id: &str) -> Result<usize, PersistenceError> {
        let len_before = self.edges.len();
        self.edges
            .retain(|e| e.source_id != memory_id && e.target_id != memory_id);
        let removed = len_before - self.edges.len();
        if removed > 0 {
            self.rebuild_edge_keys();
            self.save()?;
        }
        Ok(removed)
    }

    /// 把一批边合入内存（去重 + 对称规范化的**唯一实现**）。
    ///
    /// 返回**新增**的边（已存在的边不返回）。**不落盘、不置脏**——两条通道
    /// （立即落盘 / deferred）各自在调用后处理持久化，避免把两种策略写两份。
    fn merge_edges(&mut self, edges: &[(String, String, EdgeType, f32)]) -> Vec<MemoryEdge> {
        let mut added: Vec<MemoryEdge> = Vec::new();
        for (source, target, etype, weight) in edges {
            // 对称关系规范化：让 (A,B) 与 (B,A) 落到同一个去重键
            let (s, t) = if etype.is_symmetric() && source.as_str() > target.as_str() {
                (target, source)
            } else {
                (source, target)
            };
            // 自环无信息量（`evolved_from` 是自指关系，需排除）
            if s == t {
                continue;
            }
            let exists = self
                .edge_keys
                .contains(&(s.clone(), t.clone(), etype.clone()));
            if !exists {
                let edge = MemoryEdge::new(s.clone(), t.clone(), etype.clone(), *weight);
                self.edge_keys.insert((s.clone(), t.clone(), etype.clone()));
                self.edges.push(edge.clone());
                added.push(edge);
            }
        }
        added
    }

    /// 批量添加边（**立即落盘**，只在末尾保存一次）
    ///
    /// # 为什么需要它
    ///
    /// [`Self::add_edge`] 每条边都调用 `save()`（全量序列化整个边集）。
    /// 全库回填记录层关系时若逐条调用，就是 **O(E²) 写盘**——边数上千时会
    /// 卡住服务。本方法先把所有边入内存，最后统一保存一次。
    ///
    /// # 对称关系规范化（关键）
    ///
    /// 对**对称关系**（`same_event` / `shared_artifact` 等），
    /// 关联推导会**双向产出**（A 的关联含 B，B 的关联也含 A）。
    /// 若按原样写入，A→B 与 B→A 会因 `(source,target,edge_type)` 不同
    /// 而被存成**两条边** ⇒ 边数虚增一倍，且 `query_edges` 返回重复邻居。
    ///
    /// 故对称关系按 **ID 字典序**规范化到同一键。有向关系（`derived_from` 等）
    /// **不做规范化**——方向是记录的语义，交换两端会丢失"谁衍生自谁"。
    ///
    /// 返回**新增**边数（已存在的边不重复计数）。
    pub fn add_edges_batch(
        &mut self,
        edges: &[(String, String, EdgeType, f32)],
    ) -> Result<usize, PersistenceError> {
        let added = self.merge_edges(edges);
        if !added.is_empty() {
            // ★★2026-09-18 审查修复：保存失败时**回滚内存**，保证
            //   "返回 Err ⇔ 未写入"这一语义成立。
            //
            // 原实现先 `push` 再 `save()?`：若 save 失败（磁盘满/权限），
            //   边已留在 self.edges 里，而函数返回 Err ⇒ 调用方（
            //   `/v1/memories/external-edge` 返回 500；Python 侧记 `skipped`）
            //   认为"没写成"，**进程内却已生效**，并会在下次任意成功的
            //   `save()` 时被一起落盘 —— 形成"报失败却最终落盘"的时序反转。
            //
            // 回滚方式：记录本次批次的起始长度，失败时截断回去。
            // 这比"先写盘再入内存"简单，且不引入临时副本。
            let before = self.edges.len() - added.len();
            // ★增量写：追加日志（O(1)），不再整图重写。
            //   基文件缺失或日志超阈值时改为整图压实（与主库同纪律）。
            let result = if self.needs_compact() {
                self.save()
            } else {
                self.append_journal(&added)
            };
            if let Err(e) = result {
                self.edges.truncate(before);
                // ★ 回滚 edge_keys：`merge_edges` 已把本批新增键插入索引，
                //   若不回滚，重试同一批边会被判"已存在"而返回 Ok(0) 且不写盘
                //   ⇒ 边永久丢失（审查 P1-2）。
                self.rebuild_edge_keys();
                return Err(e);
            }
        }
        Ok(added.len())
    }

    /// 批量添加边（**只改内存**，置脏标记，由后台 flush 统一落盘）。
    ///
    /// # 为什么读路径必须用它（v0.9.16）
    ///
    /// `expand_associations` 在**每次检索**里写记录层边。走 [`Self::add_edges_batch`]
    /// 就意味着每次检索都要序列化并重写整个图文件（线上 52MB），
    /// 而且发生在**全局存储锁内** ⇒ 单次检索的锁占用被拉长到秒级，
    /// 写入被饿死（实测 81 秒 busy / 写入 3/12 成功）。
    ///
    /// 本方法只做内存合入（微秒级），磁盘交给后台任务。
    /// 返回**新增**边数。
    pub fn add_edges_batch_deferred(&mut self, edges: &[(String, String, EdgeType, f32)]) -> usize {
        let added = self.merge_edges(edges);
        if !added.is_empty() {
            // ★只记录"新增的边"，flush 时只追加这些（不再整图重写）。
            self.pending.extend(added.iter().cloned());
            self.dirty = true;
        }
        added.len()
    }

    /// 查询与指定记忆相关的所有边
    pub fn query_edges(&self, memory_id: &str) -> Vec<&MemoryEdge> {
        self.edges
            .iter()
            .filter(|e| e.source_id == memory_id || e.target_id == memory_id)
            .collect()
    }

    /// 查询完整子图（指定记忆的 1-hop 邻居 + 边类型分布）
    pub fn query_subgraph(&self, memory_id: &str) -> GraphQueryResult {
        let edges = self.query_edges(memory_id);

        let mut related_ids: Vec<String> = Vec::new();
        let mut evolution_chain: Vec<String> = Vec::new();
        let mut synthesis_sources: Vec<String> = Vec::new();

        for e in &edges {
            let other = if e.source_id == memory_id {
                &e.target_id
            } else {
                &e.source_id
            };

            if !related_ids.contains(other) {
                related_ids.push(other.clone());
            }

            match e.edge_type {
                EdgeType::Evolves if !evolution_chain.contains(other) => {
                    evolution_chain.push(other.clone());
                }
                EdgeType::SynthesizesFrom if !synthesis_sources.contains(other) => {
                    synthesis_sources.push(other.clone());
                }
                _ => {}
            }
        }

        // BFS 获取子图大小
        let mut visited = std::collections::HashSet::new();
        let mut queue: Vec<String> = vec![memory_id.to_string()];
        visited.insert(memory_id.to_string());

        while let Some(current) = queue.pop() {
            for e in &self.edges {
                let neighbor = if e.source_id == current {
                    &e.target_id
                } else if e.target_id == current {
                    &e.source_id
                } else {
                    continue;
                };

                if visited.insert(neighbor.clone()) {
                    queue.push(neighbor.clone());
                }
            }
        }

        GraphQueryResult {
            related_ids,
            evolution_chain,
            synthesis_sources,
            subgraph_size: visited.len(),
        }
    }

    /// 获取所有边（用于调试和导出）
    pub fn all_edges(&self) -> &[MemoryEdge] {
        &self.edges
    }

    /// 获取边总数
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// 清空所有边
    pub fn clear(&mut self) -> Result<(), PersistenceError> {
        self.edges.clear();
        self.edge_keys.clear();
        self.pending.clear();
        self.save()
    }
}

/// 图落盘的默认周期（秒）。
///
/// 取值理由：图是**派生数据**，延迟一个周期落盘只影响"进程被杀后重算"的成本；
/// 而周期太短会让 52MB 的整图重写重新变成常态写入。30 秒是两条约束的交点。
/// 环境变量 `LRC_GRAPH_FLUSH_SECS` 可覆盖（下限 5 秒，防止把磁盘写满）。
pub const GRAPH_FLUSH_INTERVAL_SECS: u64 = 30;

/// 待落盘的图快照（在**锁外**完成序列化与写盘）。
///
/// 见 [`GraphMemoryStore::take_dirty_snapshot`] 的"为什么不在锁内 save"。
pub struct GraphSnapshot {
    path: std::path::PathBuf,
    edges: Vec<MemoryEdge>,
}

impl GraphSnapshot {
    /// 快照中的边数（供日志）。
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// 序列化并原子写盘。
    pub fn write(&self) -> Result<(), PersistenceError> {
        let json =
            serde_json::to_string_pretty(&self.edges).map_err(PersistenceError::Serialization)?;
        crate::atomic_file::write_atomic(&self.path, json.as_bytes()).map_err(PersistenceError::Io)
    }
}

/// 待落盘的**增量**图快照（在锁外追加日志；必要时整图压实）。
///
/// 见 [`GraphMemoryStore::take_pending_snapshot`]。
pub struct GraphPending {
    journal_path: std::path::PathBuf,
    pending: Vec<MemoryEdge>,
    /// `Some((基文件路径, 全量边))` 表示本轮需要整图压实。
    compact: Option<(std::path::PathBuf, Vec<MemoryEdge>)>,
}

impl GraphPending {
    /// 本轮待追加的边数（供日志）。
    pub fn edge_count(&self) -> usize {
        self.pending.len()
    }

    /// 本轮是否触发整图压实。
    pub fn is_compaction(&self) -> bool {
        self.compact.is_some()
    }

    /// 取回待追加的边（写盘失败时用于 [`GraphMemoryStore::restore_pending`]）。
    pub fn pending_edges(&self) -> &[MemoryEdge] {
        &self.pending
    }

    /// 落盘：需要压实时整图重写 + 清空日志；否则只追加日志。
    pub fn write(&self) -> Result<(), PersistenceError> {
        if let Some((path, edges)) = &self.compact {
            let json =
                serde_json::to_string_pretty(edges).map_err(PersistenceError::Serialization)?;
            crate::atomic_file::write_atomic(path, json.as_bytes())
                .map_err(PersistenceError::Io)?;
            // 整图已吸收全部增量 ⇒ 清空日志。
            let _ = std::fs::remove_file(&self.journal_path);
            return Ok(());
        }
        if self.pending.is_empty() {
            return Ok(());
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_path)
            .map_err(PersistenceError::Io)?;
        let mut buf = String::new();
        for edge in &self.pending {
            buf.push_str(&serde_json::to_string(edge).map_err(PersistenceError::Serialization)?);
            buf.push('\n');
        }
        file.write_all(buf.as_bytes())
            .map_err(PersistenceError::Io)?;
        file.flush().map_err(PersistenceError::Io)?;
        file.sync_data().map_err(PersistenceError::Io)?;
        Ok(())
    }
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_graph_store() -> (TempDir, GraphMemoryStore) {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let store = GraphMemoryStore::new(&data_dir);
        (dir, store)
    }

    #[test]
    fn test_add_edge() {
        let (_dir, mut store) = make_graph_store();
        store
            .add_edge("mem-1", "mem-2", EdgeType::RelatedTo, 0.5)
            .expect("应成功添加边");
        assert_eq!(store.edge_count(), 1);
    }

    #[test]
    fn test_deduplicate_edges() {
        let (_dir, mut store) = make_graph_store();
        store.add_edge("a", "b", EdgeType::Evolves, 0.8).unwrap();
        store.add_edge("a", "b", EdgeType::Evolves, 0.9).unwrap(); // 重复
        assert_eq!(store.edge_count(), 1, "不应添加重复边");
    }

    /// ★★审查发现 6：`save()` 必须用 UUID 唯一临时名，且**不残留** `.tmp`。
    ///
    /// # 为什么必须有这条
    ///
    /// 修复前此处手写 `format!("{}.tmp", edges_file)`（固定名）且失败不清理，
    /// 恰是 `atomic_file.rs` 模块文档点名要消除的写法。
    /// 现复用 `write_atomic`（UUID 临时名 + 失败清理）。
    ///
    /// 本测试锁定两件事：① 落盘后目录里**没有**残留 `.tmp`；
    /// ② 目标文件内容完整可被 `load()` 解析回来（原子替换语义）。
    #[test]
    fn test_save_is_atomic_and_leaves_no_tmp() {
        let (dir, mut store) = make_graph_store();
        store
            .add_edge("mem-1", "mem-2", EdgeType::RelatedTo, 0.5)
            .expect("应成功添加边");

        // ① 目录里不得残留任何 .tmp
        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .expect("应能读目录")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "★原子写后不得残留临时文件（修复前固定名且失败不清理）: {:?}",
            leftovers
        );

        // ② 内容可被重新加载（原子替换后是完整 JSON）
        let mut reloaded = GraphMemoryStore::new(&dir.path().to_string_lossy());
        reloaded.load().expect("应能加载回图");
        assert_eq!(
            reloaded.edge_count(),
            1,
            "★落盘内容必须完整可解析（原子替换语义）"
        );
    }

    /// ★★审查发现 7：`add_edges_batch` 保存失败时必须**回滚内存**，
    /// 保证"返回 Err ⇔ 未写入"。
    ///
    /// # 构造手法
    ///
    /// 把**日志路径**做成一个**目录**（而非文件）⇒ 追加必失败，
    /// 从而在**不依赖"磁盘满"这种不可造条件**的前提下触发写盘失败。
    ///
    /// ★v3 更新：持久化从"整图重写"改为"追加日志"，故失败注入点从
    /// `graph_edges.json` 移到 `graph_edges.journal.jsonl`。不变量不变：
    /// 返回 Err ⇔ 内存已回滚。
    #[test]
    fn test_add_edges_batch_rolls_back_on_save_failure() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let mut store = GraphMemoryStore::new(&data_dir);

        // 先正常写一条并落盘（确认基础路径可用；首次写入会建基文件）
        store
            .add_edge("base-a", "base-b", EdgeType::RelatedTo, 0.5)
            .expect("应成功");
        let before = store.edge_count();

        // 把**日志**路径替换成一个**目录** ⇒ 后续追加必失败
        let journal_file = dir.path().join("graph_edges.journal.jsonl");
        if journal_file.exists() {
            std::fs::remove_file(&journal_file).expect("应能删除日志");
        }
        std::fs::create_dir(&journal_file).expect("应能建同名目录");

        // 批量添加：append_journal 会因目标是目录而失败
        let batch = vec![(
            "x-1".to_string(),
            "x-2".to_string(),
            EdgeType::RelatedTo,
            0.4f32,
        )];
        let res = store.add_edges_batch(&batch);

        assert!(res.is_err(), "★写盘失败时 add_edges_batch 必须返回 Err");
        assert_eq!(
            store.edge_count(),
            before,
            "★★返回 Err 时内存必须已回滚（不得留下'报失败却已生效'的边）"
        );
        // 确认那条边**确实没进内存**
        assert!(
            !store
                .query_edges("x-1")
                .iter()
                .any(|e| e.target_id == "x-2"),
            "★失败的边不得残留在内存图中"
        );
    }

    /// ★v3：增量日志的往返一致性——基文件 + 日志重放 = 完整状态。
    #[test]
    fn test_journal_roundtrip() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let mut store = GraphMemoryStore::new(&data_dir);

        // 首次写入建基文件（needs_compact 为真）
        store
            .add_edge("a", "b", EdgeType::RelatedTo, 0.5)
            .expect("应成功");
        // 后续写入走日志追加
        store
            .add_edge("c", "d", EdgeType::RelatedTo, 0.5)
            .expect("应成功");
        store
            .add_edges_batch(&[("e".to_string(), "f".to_string(), EdgeType::Cause, 0.9f32)])
            .expect("应成功");
        assert_eq!(store.edge_count(), 3);

        // 重新加载：基文件 + 日志重放
        let mut reloaded = GraphMemoryStore::new(&data_dir);
        reloaded.load().expect("应能加载");
        assert_eq!(
            reloaded.edge_count(),
            3,
            "★基文件 + 日志重放必须还原完整边集"
        );
    }

    /// ★v3：deferred 通道只把**新增边**放进 pending，flush 后追加日志。
    #[test]
    fn test_deferred_pending_appends_only_new_edges() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let mut store = GraphMemoryStore::new(&data_dir);
        // 先建基文件
        store
            .add_edge("base-a", "base-b", EdgeType::RelatedTo, 0.5)
            .expect("应成功");

        let added = store.add_edges_batch_deferred(&[
            (
                "x".to_string(),
                "y".to_string(),
                EdgeType::SameEvent,
                0.9f32,
            ),
            (
                "x".to_string(),
                "z".to_string(),
                EdgeType::SameEvent,
                0.9f32,
            ),
        ]);
        assert_eq!(added, 2);
        assert!(store.has_pending_writes());

        let pending = store.take_pending_snapshot().expect("应有增量快照");
        assert_eq!(pending.edge_count(), 2, "只应带走新增的 2 条边");
        assert!(!pending.is_compaction(), "日志未超阈值时不应压实");
        pending.write().expect("应能追加日志");

        // 重新加载：基文件 1 条 + 日志 2 条
        let mut reloaded = GraphMemoryStore::new(&data_dir);
        reloaded.load().expect("应能加载");
        assert_eq!(reloaded.edge_count(), 3);
    }

    #[test]
    fn test_query_edges() {
        let (_dir, mut store) = make_graph_store();
        store.add_edge("a", "b", EdgeType::Evolves, 0.8).unwrap();
        store.add_edge("a", "c", EdgeType::RelatedTo, 0.3).unwrap();
        store
            .add_edge("d", "a", EdgeType::Contradicts, 0.1)
            .unwrap();

        let edges = store.query_edges("a");
        assert_eq!(edges.len(), 3, "a 应有 3 条关联边");
    }

    #[test]
    fn test_subgraph() {
        let (_dir, mut store) = make_graph_store();
        store.add_edge("a", "b", EdgeType::Evolves, 0.8).unwrap();
        store.add_edge("b", "c", EdgeType::Evolves, 0.7).unwrap();
        store.add_edge("a", "d", EdgeType::RelatedTo, 0.3).unwrap();

        let result = store.query_subgraph("a");
        assert_eq!(result.subgraph_size, 4, "子图应包含 a,b,c,d");
        assert_eq!(result.related_ids.len(), 2, "a 直接关联 b 和 d");
    }

    #[test]
    fn test_remove_edge() {
        let (_dir, mut store) = make_graph_store();
        store.add_edge("a", "b", EdgeType::Evolves, 0.8).unwrap();
        let edge_id = store.all_edges()[0].id.clone();

        let removed = store.remove_edge(&edge_id).unwrap();
        assert!(removed);
        assert_eq!(store.edge_count(), 0);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let (dir, mut store) = make_graph_store();
        store
            .add_edge("mem-1", "mem-2", EdgeType::Evolves, 0.85)
            .unwrap();
        store
            .add_edge("mem-2", "mem-3", EdgeType::SynthesizesFrom, 0.92)
            .unwrap();
        store.save().unwrap();

        // 重新加载
        let data_dir = dir.path().to_string_lossy().to_string();
        let mut store2 = GraphMemoryStore::new(&data_dir);
        store2.load().unwrap();
        assert_eq!(store2.edge_count(), 2);
    }

    /// ★v0.9.16：deferred 通道**不得**在加边时碰磁盘，flush 之后才落盘。
    ///
    /// 这条断言锁定的是 P1 的**全部收益来源**：`expand_associations` 在每次检索里
    /// 调它，若这里仍然写盘，就还是"每轮检索在锁内重写 52MB 图文件"。
    #[test]
    fn test_deferred_batch_does_not_write_disk_until_flush() {
        let (dir, mut store) = make_graph_store();
        let added = store.add_edges_batch_deferred(&[(
            "a".to_string(),
            "b".to_string(),
            EdgeType::RelatedTo,
            0.5,
        )]);
        assert_eq!(added, 1);
        assert!(store.has_pending_writes(), "deferred 后应置脏");

        // ① 未 flush：文件都不存在（更谈不上被重写）
        assert!(
            !dir.path().join("graph_edges.json").exists(),
            "★deferred 通道不得产生任何磁盘写入"
        );

        // ② flush 一次 ⇒ 落盘且清脏
        assert!(store.flush_if_dirty().unwrap(), "有脏标记时应真的写盘");
        assert!(!store.has_pending_writes(), "落盘后脏标记必须清掉");
        assert!(
            !store.flush_if_dirty().unwrap(),
            "无变更时不得再写盘（否则定时 flush 会变成常态写入）"
        );

        // ③ 内容完整可解析
        let mut reloaded = GraphMemoryStore::new(&dir.path().to_string_lossy());
        reloaded.load().expect("应能加载回图");
        assert_eq!(reloaded.edge_count(), 1);
    }

    /// ★v0.9.16：快照取走后脏标记清空；若写盘失败可经 `mark_dirty` 补回。
    #[test]
    fn test_take_snapshot_then_restore_dirty() {
        let (_dir, mut store) = make_graph_store();
        store.add_edges_batch_deferred(&[("x".to_string(), "y".to_string(), EdgeType::Cause, 0.6)]);
        let snapshot = store.take_dirty_snapshot().expect("应有快照");
        assert_eq!(snapshot.edge_count(), 1);
        assert!(!store.has_pending_writes(), "取快照即清脏");
        assert!(
            store.take_dirty_snapshot().is_none(),
            "无新变更时不得重复取快照"
        );

        // 模拟写盘失败后的补回
        store.mark_dirty();
        assert!(store.has_pending_writes(), "mark_dirty 后必须重新置脏");
    }
}
