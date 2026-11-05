//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现记忆存储的内存缓存子系统，属于公开层 (Layer 1)。
//! ============================================================
//!
//! 记忆存储缓存子系统（`MemoryStoreCache`）
//!
//! v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P2-2「MemoryStore God Object」）：
//!   本模块以**零行为变更**的切片，把 `MemoryStore` 原持有的 6 个缓存字段
//!   （`memory_cache` / `cache_dirty` / `bigram_index` / `bigram_index_dirty`
//!   / `recall_documents` / `assoc_frequency`）与随之而来的全部纯缓存逻辑
//!   外提为独立子系统，使 `MemoryStore` 只保留一个 `cache` 字段。
//!
//! **并发模型（2026-11-05 改造）**：
//!   本模块原用 `RefCell`/`Cell` 做内部可变性，依赖外层 `Mutex<MemoryStore>`
//!   提供互斥。压测显示单一全局写锁使 16 并发读的 P95 达 6s（锁等待超时 503），
//!   故改为**内部线程安全**：数据字段用 `RwLock`（读多写少，允许并发读），
//!   标量标记用 `AtomicBool`/`AtomicU64`。这样 `MemoryStoreCache: Sync`，
//!   外层即可用 `Arc<RwLock<MemoryStore>>` 让多个读请求真正并行。
//!
//! 不负责：
//!   - 持久化读写（由 `MemoryStore` 持有的 `Persistence` 负责）
//!   - 检索评分（由 `MemoryStore` 的召回路径负责）

use crate::engine::synthesis_engine::JaccardTerms;
use crate::memory_types::Memory;
use crate::persistence::AssocFrequency;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// 记忆文档特征（recall 文本缓存项）
///
/// 缓存「规范化正文 + 文档长度」，避免同一记忆在同一/多次查询中被反复
/// 做 `to_lowercase` 与分词——这两步是检索热路径上最常复用的纯计算。
pub(crate) struct RecallDocument {
    /// 小写规范化后的正文
    ///
    /// ★ 2026-10-30（性能回归修复）：由 `String` 改为 `Arc<str>`。
    ///
    /// 修前每次 `recall_document()` 命中缓存都要**深拷贝整条正文**，而快路
    /// 的文档频率统计是「查询词数 × 全库候选数」次调用（46.7 万条库、20 个
    /// 查询词 ⇒ 约 930 万次克隆），单次检索因此多付数 GB 的内存拷贝与分配，
    /// 实测把快路从 1.6s 拖到 12s。改为共享 `Arc<str>` 后取用只增引用计数。
    pub(crate) normalized_content: Arc<str>,
    /// 文档长度（token 数，与评分口径一致）
    pub(crate) token_count: usize,
}

/// 候选索引类型：term → 记忆 ID 集合
pub(crate) type BigramIndex = HashMap<String, HashSet<String>>;

/// 八卦桶索引：`(data_version, 8 个桶的 position 列表)`
///
/// 抽成别名是为了让 `bagua_buckets` 字段的类型可读（clippy::type_complexity）。
pub(crate) type BaguaBuckets = (u64, Arc<[Vec<usize>; 8]>);

/// `MemoryStore` 的内存缓存子系统
///
/// 持有叠加在持久层之上的四类缓存：
///   1. 全量记忆快照（`memory` + `dirty`）
///   2. 候选 term 倒排索引（`index` + `index_dirty`）
///   3. recall 文本特征（`documents`）
///   4. 跨查询命中频次统计（`assoc`）
pub(crate) struct MemoryStoreCache {
    /// 全量记忆增量缓存（避免每次操作都 O(N) 全量加载）
    ///
    /// ★ v0.9.13（建议二）：由 `Vec<Memory>` 改为 `Arc<Vec<Memory>>`。
    /// 修前 `snapshot()` 每次都 `clone()` 整库（`Memory` 含正文/实体/卦象字符串），
    /// 而一次 enrich 里 `recall`（快路 + 深路 + 联想扩散的每个节点）与
    /// `expand_associations` 都会各取一次快照 ⇒ 同一请求内把整库深拷贝十几遍，
    /// 全部发生在存储锁内，是"单次 enrich 持锁 20~25s"的主要结构性成本。
    /// 改为 `Arc` 后，同版本的多次快照共享同一份不可变数据（只增引用计数），
    /// **读取路径零拷贝**；写入走 `Arc::make_mut`（写时复制）。
    /// 外层 `RwLock` 允许读并发，故快照可能被多个读者同时持有 ⇒ 写时复制
    /// 只复制指针（见下），不复制 `Memory` 本体。
    ///
    /// ★ 2026-11-05（读并发改造）：内层元素由 `Memory` 改为 `Arc<Memory>`。
    ///
    /// **为什么**：读并发后，读者会**长期持有**快照 `Arc`（整个检索期间），
    /// 写者 `Arc::make_mut` 必然触发整库克隆。实测 7149 条（25MB）单次克隆
    /// **424ms**（`commit` P50 从 19.8ms 暴增到 424ms），混合负载写 P95 达 20.6s。
    /// 改为 `Vec<Arc<Memory>>` 后，克隆只复制 **7149 个指针（57KB）**，
    /// 单次 ~0.1ms，且 `Memory` 本体仍共享（读者与写者看到同一份不可变数据）。
    memory: RwLock<Arc<Vec<Arc<Memory>>>>,
    /// 缓存脏标记：任何写操作后置 true，读操作前检查
    dirty: AtomicBool,
    /// 加载本缓存时数据源的指纹（v0.9.11 / CR-42）：
    /// 与持久层当前指纹不符 ⇒ 说明**别的进程**改过数据，必须重载。
    /// `None` 表示后端不提供指纹（则只看 `dirty`，行为与修前一致）。
    source_fingerprint: Mutex<Option<u64>>,
    /// 候选 term 倒排索引（默认启用域候选索引；最终仍由 Jaccard 判定）
    index: RwLock<BigramIndex>,
    /// 索引脏标记
    index_dirty: AtomicBool,
    /// 数据版本号（v0.9.13）：**任何**数据变更（写入/删除/外部重载）后单调递增。
    ///
    /// 用途：让"从全库派生的统计表"（hub 实体表、自动事件表、artifact 表）
    /// 能以**版本相等**判定缓存是否仍与当前数据同源，从而跳过每次检索都
    /// 重建 O(N) 表的代价（实测 449k 条库上建表需 17s）。
    ///
    /// 为什么放在本模块而不是 `MemoryStore`：所有写入路径最终都会经过本模块的
    /// `invalidate` / `mark_dirty_preserving_index` / `append_new` / `store`
    /// 四个入口之一 ⇒ 在此递增是**唯一不会漏**的位置。
    data_version: AtomicU64,
    /// recall 文本特征缓存，按 Memory ID 复用规范化正文和文档长度
    documents: RwLock<HashMap<String, RecallDocument>>,
    /// 跨查询命中频次统计（独立文件持久化，`Memory` 零改动）
    assoc: RwLock<AssocFrequency>,
    /// 八卦桶索引（P3 / M.T.R. 传输层）：`bagua_index → 快照内 position 列表`。
    ///
    /// 用途：`find_similar` 的全库扫描按"查询桶 → 相邻桶 → 其余桶"的顺序路由，
    /// 让 M.T.R. 只负责**传输顺序**（不判断语义），召回仍由桶内 Jaccard + 回退保证。
    ///
    /// 以 `data_version` 失效：任何写入/重载后版本递增，下次取桶时重建。
    bagua_buckets: RwLock<Option<BaguaBuckets>>,
    /// 每条记忆的 **Jaccard 预计算集合**（2026-11-05）。
    ///
    /// **为什么需要**：相似检查的剩余全库扫描里，单次比较的成本几乎全在
    /// `JaccardTerms::new(&m.content)`——它每次都要 `to_lowercase` + 收集字符 +
    /// 建 `HashSet`（实测 ~3µs/条）。预计算后单次比较只剩集合交集（~0.3µs）。
    ///
    /// **维护方式**：与 `documents` 同生命周期——
    ///   · `store()`（冷加载/外部重载）一次性为全库预计算；
    ///   · `append_new()` / `upsert()` 增量补算新增或改写的条目；
    ///   · `invalidate()` / `mark_dirty_preserving_index()` 清空（正文可能已变）。
    ///
    /// 用 `Arc<HashMap>` 而非 `HashMap`：`prepare_remember` 需要把一份**只读快照**
    /// 交给锁外的 `analyze`（`RememberPrepData` 必须可跨线程），克隆 `Arc` 是 O(1)。
    /// 写路径用 `Arc::make_mut`：正常时序下（analyze 在 commit 前结束）引用计数为 1，
    /// 原地修改；仅当确有并发快照在外时才整表克隆一次（安全方向）。
    jaccard_terms: RwLock<Arc<HashMap<String, Arc<JaccardTerms>>>>,
    /// 记忆 ID → 快照内 position 的映射（2026-11-05）。
    ///
    /// **为什么需要**：`build_candidates` 每次查询都要把倒排索引命中的 id 换算成
    /// 快照下标。修前每次都 `all.iter().enumerate().collect::<HashMap>()`——
    /// 即**每次查询 O(N) 建表**（7048 条约 0.5ms），把索引剪枝省下的时间又还回去。
    ///
    /// **维护方式**（与快照严格对齐）：
    ///   · `store()` 全量重建；
    ///   · `append_new()` 追加（既有 position 不变）；
    ///   · `upsert()` 原地替换（position 不变）或追加（position = len-1）；
    ///   · `invalidate()` / `mark_dirty_preserving_index()` 清空（下次 `store()` 重建）。
    ///
    /// 删除路径走 `mark_dirty_preserving_index` ⇒ 下次读整库重载并重建，
    /// 因此**不存在**"删除后 position 前移"的失效问题。
    id_positions: RwLock<Arc<HashMap<String, usize>>>,
}

impl MemoryStoreCache {
    /// 构造空缓存（全部标记为脏，首次读取时惰性加载/重建）
    pub(crate) fn new(assoc_frequency: AssocFrequency) -> Self {
        Self {
            memory: RwLock::new(Arc::new(Vec::new())),
            dirty: AtomicBool::new(true), // 初始为脏，首次读取时加载
            source_fingerprint: Mutex::new(None),
            index: RwLock::new(HashMap::new()),
            index_dirty: AtomicBool::new(true),
            data_version: AtomicU64::new(0),
            documents: RwLock::new(HashMap::new()),
            assoc: RwLock::new(assoc_frequency),
            bagua_buckets: RwLock::new(None),
            jaccard_terms: RwLock::new(Arc::new(HashMap::new())),
            id_positions: RwLock::new(Arc::new(HashMap::new())),
        }
    }

    /// 当前数据版本号（每次数据变更后递增；语义见字段文档）
    pub(crate) fn data_version(&self) -> u64 {
        self.data_version.load(Ordering::Acquire)
    }

    /// 递增数据版本号（所有失效入口都必须调用）
    fn bump_data_version(&self) {
        self.data_version.fetch_add(1, Ordering::AcqRel);
    }

    // ---------- 全量记忆快照 ----------

    /// 是否需要从持久层重载
    ///
    /// 条件（v0.9.11 / CR-42 新增第 2 条）：
    ///   1. 自身被标记为脏（本进程写过）；
    ///   2. **数据源指纹变了**（别的进程写过）—— 这是修前缺失的一环，
    ///      导致多窗口/多进程并存时可能**无限期读陈旧数据**。
    ///
    /// 后端不提供指纹（`current == None`）时只按 `dirty` 判定 ⇒ 与修前行为一致。
    pub(crate) fn needs_reload(&self, current: Option<u64>) -> bool {
        if self.dirty.load(Ordering::Acquire) {
            return true;
        }
        let loaded = *self.source_fingerprint.lock().unwrap();
        match (current, loaded) {
            (Some(now), Some(loaded)) => now != loaded,
            (Some(_), None) => true, // 首次拿到指纹（此前未记录）⇒ 保守重载一次
            _ => false,
        }
    }

    /// 用新加载的全量记忆替换缓存并清除脏标记，同时记录数据源指纹
    pub(crate) fn store(&self, memories: Vec<Memory>, source_fingerprint: Option<u64>) {
        // 冷加载/外部重载：一次性为全库预计算 Jaccard 集合（只发生一次，不在写热路径）。
        // 先整体替换为空表，避免旧库已删除条目的条目残留。
        *self.jaccard_terms.write().unwrap() = Arc::new(HashMap::new());
        self.fill_jaccard_terms(&memories);
        // ID → position 全量重建（与快照严格对齐）。
        let positions: HashMap<String, usize> = memories
            .iter()
            .enumerate()
            .map(|(position, memory)| (memory.id.clone(), position))
            .collect();
        *self.id_positions.write().unwrap() = Arc::new(positions);
        // ★ 2026-11-05（读并发改造）：元素包 `Arc`，使后续写时复制只复制指针。
        *self.memory.write().unwrap() = Arc::new(memories.into_iter().map(Arc::new).collect());
        *self.source_fingerprint.lock().unwrap() = source_fingerprint;
        self.dirty.store(false, Ordering::Release);
        self.bump_data_version();
        // P1-6（2026-10-30）：整库替换后，旧索引与文本特征不再覆盖新数据，理论上
        // 应同步失效（否则别的进程写入的新条目会静默漏召回）。
        //
        // **但默认关闭**：线上实测（46.7 万条 + ML 编码器）发现，重载后清空文本特征
        // 会把快路 1.65s→12.2s、深路 1.7s→4.8s、关联 0.7s→4.1s（三段同时变慢），
        // 因为每次检索都要重新小写化整库正文并重建关联表。
        // 需要修复"跨进程新条目漏召回"时显式开启：
        //   set LRC_CACHE_INVALIDATE_ON_RELOAD=1
        // 默认关闭时行为与改造前**逐字节一致**。
        if Self::invalidate_on_reload_enabled() {
            self.index_dirty.store(true, Ordering::Release);
            self.documents.write().unwrap().clear();
        }
    }

    /// P1-6 开关：整库替换时是否同步失效索引与文本特征缓存（默认关闭）。
    pub(crate) fn invalidate_on_reload_enabled() -> bool {
        std::env::var("LRC_CACHE_INVALIDATE_ON_RELOAD")
            .map(|v| v == "1")
            .unwrap_or(false)
    }

    /// 返回缓存快照（`Arc` 共享，**不做整库深拷贝**）
    ///
    /// ★ v0.9.13（建议二）：修前返回 `Vec<Memory>` 的深拷贝，每取一次快照就
    /// 付一次全库克隆；调用方都是"只读扫描"（`recall` 过滤打分、
    /// `expand_associations` 建索引表），从不修改快照内容 ⇒ 共享 `Arc` 即可，
    /// 语义（"调用方拿到一份独立于后续写入的稳定视图"）完全不变：
    /// 后续任何写入都会 `Arc::make_mut` 造出新副本，旧快照照样看到写入前的内容。
    ///
    /// # 为什么调用方不能拿到 `Ref`
    ///
    /// 调用期间可能**再次**取快照/取表（如 `recall` 内部取活跃记忆、
    /// `expand_associations` 内部建三张全库统计表），返回 `Ref` 会自锁。
    pub(crate) fn snapshot(&self) -> Arc<Vec<Arc<Memory>>> {
        Arc::clone(&self.memory.read().unwrap())
    }

    /// 取八卦桶索引（P3 / M.T.R. 传输层，按 `data_version` 缓存）。
    ///
    /// 返回 `Arc<[Vec<usize>; 8]>`：每个桶存快照内的 position 列表。
    /// 桶号 = `bagua_index % 8`；无 `bagua_index` 的记忆归入桶 0（保守）。
    ///
    /// **只负责路由，不判断语义**：调用方按"查询桶 → 相邻桶 → 其余桶"的顺序
    /// 扫描，召回仍由桶内 Jaccard + 回退保证。
    pub(crate) fn bagua_buckets(&self) -> Arc<[Vec<usize>; 8]> {
        let version = self.data_version.load(Ordering::Acquire);
        {
            let cached = self.bagua_buckets.read().unwrap();
            if let Some((v, buckets)) = cached.as_ref() {
                if *v == version {
                    return Arc::clone(buckets);
                }
            }
        }
        let mut buckets: [Vec<usize>; 8] = std::array::from_fn(|_| Vec::new());
        {
            let memories = self.memory.read().unwrap();
            for (pos, m) in memories.iter().enumerate() {
                let b = m.bagua_index.map(|x| x as usize % 8).unwrap_or(0);
                buckets[b].push(pos);
            }
        }
        let arc = Arc::new(buckets);
        *self.bagua_buckets.write().unwrap() = Some((version, Arc::clone(&arc)));
        arc
    }

    /// 取 Jaccard 预计算集合的只读快照（`Arc` 共享，O(1)）
    ///
    /// 供 `find_similar`（锁内）与 `prepare_remember`（交给锁外 `analyze`）使用。
    /// 未命中的 id 由调用方按需现算（见 `scan_candidates`），不影响正确性。
    pub(crate) fn jaccard_terms_snapshot(&self) -> Arc<HashMap<String, Arc<JaccardTerms>>> {
        Arc::clone(&self.jaccard_terms.read().unwrap())
    }

    /// 取 ID → position 映射的只读快照（`Arc` 共享，O(1)）
    ///
    /// 供 `build_candidates` 把倒排索引命中的 id 换算成快照下标，
    /// 避免每次查询都 O(N) 重建映射表。
    pub(crate) fn id_positions_snapshot(&self) -> Arc<HashMap<String, usize>> {
        Arc::clone(&self.id_positions.read().unwrap())
    }

    /// 为一批记忆补算 Jaccard 预计算集合（幂等）
    fn fill_jaccard_terms(&self, memories: &[Memory]) {
        if memories.is_empty() {
            return;
        }
        let mut guard = self.jaccard_terms.write().unwrap();
        let map = Arc::make_mut(&mut guard);
        for memory in memories {
            map.insert(
                memory.id.clone(),
                Arc::new(JaccardTerms::new(&memory.content)),
            );
        }
    }

    /// 仅判断指定 ID 是否存在（**不克隆**任何 `Memory`）
    ///
    /// 与 `snapshot().iter().any(|m| m.id == id)` 语义完全相同，但后者会先
    /// 深拷贝整库（`Memory` 含正文字符串、实体列表、卦象等），在**逐条**执行的
    /// 存在性校验路径上是纯浪费的内存抖动与锁持有时间。
    pub(crate) fn contains_id(&self, id: &str) -> bool {
        self.memory.read().unwrap().iter().any(|m| m.id == id)
    }

    /// 缓存中的记忆条数（O(1)，**不克隆**）
    pub(crate) fn len(&self) -> usize {
        self.memory.read().unwrap().len()
    }

    /// 收集合成类记忆的摘要文本（**不克隆整库**）
    ///
    /// 为什么需要：`/v1/memories/consolidate` 的 Phase 3 原先先
    /// `list_memories()`（整库深拷贝）再 `total_count()`（又一次整库深拷贝），
    /// 在 28 万条库上实测白付 11~13 秒并长时间持锁。这里只按类型过滤后
    /// 克隆命中的摘要字符串（通常为 0 条）。
    pub(crate) fn synthesis_summaries(&self) -> Vec<String> {
        self.memory
            .read()
            .unwrap()
            .iter()
            .filter(|m| m.memory_type == crate::memory_types::MemoryType::Synthesis)
            .map(|m| m.summary())
            .collect()
    }

    /// 增量同步：把**刚落盘的新记忆**并入缓存并保持"不脏"
    ///
    /// 前置条件（由 [`crate::memory_store::MemoryStore::remember_batch_preencoded`] 保证）：
    /// `new` 中的每条记忆都带**新生成的 UUID**（`Memory::new`），因此并入即等价于
    /// upsert；不需要逐条查重（逐条查重是 O(N·k)，正是本改动要消除的成本）。
    ///
    /// `source_fingerprint` 必须传入**写入之后**的数据源指纹 —— 否则下一次
    /// `needs_reload()` 会因"磁盘指纹变了"而把整个库重新解析一遍（等于没优化）。
    ///
    /// 为什么必须这样做：写完 `mark_dirty_preserving_index()` 会让下一次
    /// `ensure_cache_loaded()` 重新**从磁盘解析 400MB+ JSON**——这曾是单次 Add
    /// 白付 4~6 秒的直接原因（见 v1_api Phase 3 注释）。
    pub(crate) fn append_new(&self, new: &[Memory], source_fingerprint: Option<u64>) {
        if new.is_empty() {
            return;
        }
        // ★ v0.9.13（建议二）：写时复制。读并发下快照可能被多个读者持有，
        //   `make_mut` 会克隆一次（只复制指针，见下）；否则原地追加。
        let base_len = {
            let mut guard = self.memory.write().unwrap();
            let base_len = guard.len();
            // ★ 2026-11-05（读并发改造）：元素是 `Arc<Memory>`，`make_mut` 只复制指针。
            Arc::make_mut(&mut guard).extend(new.iter().map(|m| Arc::new(m.clone())));
            base_len
        };
        // 新增条目补算 Jaccard 预计算集合（既有条目正文未变，无需重算）。
        self.fill_jaccard_terms(new);
        // 追加条目的 position 从 base_len 起（既有条目 position 不变）。
        {
            let mut guard = self.id_positions.write().unwrap();
            let map = Arc::make_mut(&mut guard);
            for (offset, memory) in new.iter().enumerate() {
                map.insert(memory.id.clone(), base_len + offset);
            }
        }
        // ★ 2026-10-28（实测修正）：**不再清空**全库文本特征。
        //
        // 修前此处 `documents.clear()`，理由是"新增条目的文本特征尚未计算"。但：
        //   · 本方法的前置条件已保证 `new` 全带**新 UUID** ⇒ 缓存里本就不存在
        //     它们的条目，`recall_document()` 会按需惰性补算（幂等）；
        //   · 追加**不会改动**任何既有记忆的正文 ⇒ 既有条目的特征仍然有效。
        // 因此清空的唯一效果是"让下一次检索把 449k 条全部重新 lowercase + 计数"——
        // 线上实测该重建与「快路 4.8s + 深路 7.8s」中的主要部分重合，并使写入在
        // 每次检索后都被迫重付一次。改为**不清空**后，缓存只增不减地覆盖到全库，
        // 检索侧首次全量计算后即可长期复用。
        *self.source_fingerprint.lock().unwrap() = source_fingerprint;
        self.dirty.store(false, Ordering::Release);
        self.bump_data_version();
    }

    // ---------- 失效 ----------

    /// 单条写入后的**增量**缓存同步（upsert）。
    ///
    /// 与 [`Self::append_new`] 的区别：本方法支持**同 ID 替换**（`remember`
    /// 的合并路径会改写已有记忆），并只失效该条的 recall 文本特征。
    ///
    /// **为什么需要**：单条 `remember` 原用 [`Self::mark_dirty_preserving_index`]，
    /// 会让下一次读**从磁盘重载整库**（7048 条实测每次约 50ms），且清空**全部**
    /// 文本特征缓存（下次检索要重新 lowercase 整库）。改为 upsert 后：
    ///   · 快照原地并入/替换，`dirty=false` ⇒ 下一次读零重载；
    ///   · 只移除被改写那条的文本特征，其余条目继续复用。
    ///
    /// 前置条件：调用方已完成持久化，并传入**写入之后**的数据源指纹
    /// （否则下一次 `needs_reload` 会因指纹变化而重载整库）。
    pub(crate) fn upsert(&self, memory: &Memory, source_fingerprint: Option<u64>) {
        let appended_position = {
            let mut guard = self.memory.write().unwrap();
            // 写时复制：无并发读者时引用计数为 1，原地修改；否则只复制指针。
            let memories = Arc::make_mut(&mut guard);
            if let Some(slot) = memories.iter_mut().find(|m| m.id == memory.id) {
                *slot = Arc::new(memory.clone());
                None
            } else {
                let position = memories.len();
                memories.push(Arc::new(memory.clone()));
                Some(position)
            }
        };
        // 新增条目才需要补 position（原地替换的 position 不变）。
        if let Some(position) = appended_position {
            Arc::make_mut(&mut self.id_positions.write().unwrap())
                .insert(memory.id.clone(), position);
        }
        // 正文可能已变 ⇒ 只失效该条的文本特征（其余条目不受影响）
        self.documents.write().unwrap().remove(&memory.id);
        // 同步刷新该条的 Jaccard 预计算集合（合并路径会改写正文）。
        self.fill_jaccard_terms(std::slice::from_ref(memory));
        *self.source_fingerprint.lock().unwrap() = source_fingerprint;
        self.dirty.store(false, Ordering::Release);
        self.bump_data_version();
    }

    /// 单条删除后的**增量**缓存同步（2026-11-05）。
    ///
    /// **为什么需要**：`forget` 原用 [`Self::mark_dirty_preserving_index`]，
    /// 会把缓存标脏 ⇒ 下一次 `load_cached()` **整库重解析**（7137 条实测 ~100ms）。
    /// 16 并发删除时每次删除都触发一次全库重载，是压测中删除 P50 撞满 6s 锁等待的根因。
    ///
    /// 持久化侧早已是 O(1) 追加墓碑（见 `json.rs::delete_memory`），
    /// 本方法把**内存缓存侧**也对齐为增量删除：从快照移除该条、重建 id→position、
    /// 清掉该条的文本特征与 Jaccard 集合，并保持 `dirty=false`。
    ///
    /// 前置条件：调用方已完成持久化，并传入**删除之后**的数据源指纹。
    pub(crate) fn remove(&self, id: &str, source_fingerprint: Option<u64>) {
        let snapshot = {
            let mut guard = self.memory.write().unwrap();
            let memories = Arc::make_mut(&mut guard);
            let before = memories.len();
            memories.retain(|m| m.id != id);
            if memories.len() == before {
                return; // 缓存里本就没有该条
            }
            Arc::clone(&guard)
        };
        // 删除会使后续 position 前移 ⇒ 全量重建映射（O(N)，但只在本条删除时发生一次）。
        let positions: HashMap<String, usize> = snapshot
            .iter()
            .enumerate()
            .map(|(position, memory)| (memory.id.clone(), position))
            .collect();
        *self.id_positions.write().unwrap() = Arc::new(positions);
        self.documents.write().unwrap().remove(id);
        Arc::make_mut(&mut self.jaccard_terms.write().unwrap()).remove(id);
        *self.source_fingerprint.lock().unwrap() = source_fingerprint;
        self.dirty.store(false, Ordering::Release);
        self.bump_data_version();
    }

    /// 全量失效：任何写操作（保存/删除/修改）后调用
    ///
    /// 同时清空 recall 文本特征（正文可能已变）。
    pub(crate) fn invalidate(&self) {
        self.dirty.store(true, Ordering::Release);
        self.index_dirty.store(true, Ordering::Release);
        self.documents.write().unwrap().clear();
        *self.jaccard_terms.write().unwrap() = Arc::new(HashMap::new());
        *self.id_positions.write().unwrap() = Arc::new(HashMap::new());
        self.bump_data_version();
    }

    /// 部分失效：仅失效全量快照与文本特征，**保留**倒排索引
    ///
    /// 适用于"增删单条记忆且索引已增量同步"的路径——此时索引仍正确，
    /// 重建代价（O(N) 全量分词）可省。
    pub(crate) fn mark_dirty_preserving_index(&self) {
        self.dirty.store(true, Ordering::Release);
        self.documents.write().unwrap().clear();
        *self.jaccard_terms.write().unwrap() = Arc::new(HashMap::new());
        *self.id_positions.write().unwrap() = Arc::new(HashMap::new());
        self.bump_data_version();
    }

    // ---------- recall 文本特征 ----------

    /// 取（或惰性计算并缓存）指定记忆的文本特征
    ///
    /// 缓存命中时**只复制 `Arc` 引用**，不再深拷贝正文（见 `RecallDocument`
    /// 的字段文档：修前此处每次命中都克隆整条正文，是检索热路径的主要成本）。
    pub(crate) fn recall_document(&self, memory: &Memory) -> RecallDocument {
        // ★ 2026-11-05（P1 并发）：命中路径只取**读锁**。
        //   修前整个函数持写锁 ⇒ 16 并发检索时该锁成为全局串行点
        //   （每次检索对每个候选都要取一次），实测只读 P50 仍近 1s。
        if let Some(document) = self.documents.read().unwrap().get(&memory.id) {
            return RecallDocument {
                normalized_content: Arc::clone(&document.normalized_content),
                token_count: document.token_count,
            };
        }
        let normalized = memory.content.to_lowercase();
        let token_count = crate::memory_store::doc_token_count(&normalized);
        let normalized_content: Arc<str> = Arc::from(normalized);
        let mut documents = self.documents.write().unwrap();
        // 双检：并发下可能已被其它线程填入（幂等，取先到者即可）。
        if let Some(document) = documents.get(&memory.id) {
            return RecallDocument {
                normalized_content: Arc::clone(&document.normalized_content),
                token_count: document.token_count,
            };
        }
        documents.insert(
            memory.id.clone(),
            RecallDocument {
                normalized_content: Arc::clone(&normalized_content),
                token_count,
            },
        );
        RecallDocument {
            normalized_content,
            token_count,
        }
    }

    // ---------- 倒排索引 ----------

    /// 索引是否待重建
    pub(crate) fn index_is_dirty(&self) -> bool {
        self.index_dirty.load(Ordering::Acquire)
    }

    /// 借用倒排索引（只读）
    pub(crate) fn borrow_index(&self) -> RwLockReadGuard<'_, BigramIndex> {
        self.index.read().unwrap()
    }

    /// 增量加入一条记忆的 term
    pub(crate) fn add_to_index(&self, memory: &Memory) {
        if self.index_dirty.load(Ordering::Acquire) {
            return;
        }
        let mut index = self.index.write().unwrap();
        for term in Self::index_terms_for_memory(memory) {
            index.entry(term).or_default().insert(memory.id.clone());
        }
    }

    /// 增量移除一条记忆的 term
    pub(crate) fn remove_from_index(&self, memory: &Memory) {
        if self.index_dirty.load(Ordering::Acquire) {
            return;
        }
        let mut index = self.index.write().unwrap();
        for term in Self::index_terms_for_memory(memory) {
            if let Some(ids) = index.get_mut(&term) {
                ids.remove(&memory.id);
                if ids.is_empty() {
                    index.remove(&term);
                }
            }
        }
        // 同步移除该条的 Jaccard 预计算集合（删除后不再参与相似检查）。
        Arc::make_mut(&mut self.jaccard_terms.write().unwrap()).remove(&memory.id);
    }

    /// 增量替换一条记忆在索引中的 term
    pub(crate) fn replace_in_index(&self, old: &Memory, new: &Memory) {
        if self.index_dirty.load(Ordering::Acquire) {
            return;
        }
        self.remove_from_index(old);
        self.add_to_index(new);
    }

    /// **仅在索引脏时**重建（并发安全：拿到写锁后二次检查）
    ///
    /// ## 为什么需要二次检查（2026-11-05 压测归因）
    ///
    /// 调用方 `build_candidates` 在**锁外**判断 `index_is_dirty()`，随后才进入本函数
    /// 排队等 `index.write()`。16 并发纯写实测：16 个写者同时看到 `dirty=true`
    /// （启动初值），全部进入重建并串行排队 ⇒ **重复全量重建 16 次**，
    /// 单次耗时从 814ms 累积到 15208ms，直接构成写路径 P99 的 15s 尾部。
    ///
    /// 修复：先到者重建完成后置 `dirty=false`，后到者拿到锁**重新检查**后立即返回。
    /// 重建次数 16 → 1，语义不变（索引内容与"重建一次"逐位一致）。
    pub(crate) fn rebuild_index_if_dirty(&self, all: &[Arc<Memory>]) {
        let index_start = std::time::Instant::now();
        let mut index = self.index.write().unwrap();
        if !self.index_dirty.load(Ordering::Acquire) {
            return;
        }
        index.clear();
        Self::fill_index(&mut index, all, &[]);
        self.index_dirty.store(false, Ordering::Release);
        if std::env::var_os("LRC_PROFILE_REMEMBER").is_some() {
            crate::memory_store::emit_remember_profile(format!(
                "[LRC_PROFILING] rebuild_index_ms={:.3} index_terms={} memory_count={}",
                index_start.elapsed().as_secs_f64() * 1000.0,
                index.len(),
                all.len()
            ));
        }
    }

    /// 把一批记忆的 term 写入倒排索引（调用方须已持有 `index` 写锁）
    fn fill_index(index: &mut BigramIndex, base: &[Arc<Memory>], extra: &[Memory]) {
        for memory in base.iter().map(|m| m.as_ref()).chain(extra.iter()) {
            for term in Self::index_terms_for_memory(memory) {
                index.entry(term).or_default().insert(memory.id.clone());
            }
        }
    }

    /// 用「已加载快照 + 额外一批记忆」重建倒排索引
    ///
    /// 为什么需要"额外一批"：批量写入路径（`remember_batch_preencoded`）在
    /// `append_new` 之前就要把索引修好——否则 `add_to_index` 会因索引脏而直接
    /// no-op，读侧的候选裁剪将**永远拿不到可用索引**。而为了建索引触发一次
    /// 整库 JSON 重解析（400MB 级，4~6 秒）正是 `append_new` 要消除的成本，
    /// 故这里直接用内存快照 + 本批新增条目，不读盘。
    pub(crate) fn rebuild_index_with_extra(&self, base: &[Arc<Memory>], extra: &[Memory]) {
        let index_start = std::time::Instant::now();
        let mut index = self.index.write().unwrap();
        index.clear();
        // ★ S6（2026-11-02）：term 提取统一走 `index_terms_for_memory`。
        //   修前这里内联了一份同样的判断，与增量维护（add/remove）各写一遍
        //   ⇒ 一旦只改一处，全量重建与增量更新就会产出不同的索引（静默漏召回）。
        Self::fill_index(&mut index, base, extra);
        self.index_dirty.store(false, Ordering::Release);
        if std::env::var_os("LRC_PROFILE_REMEMBER").is_some() {
            crate::memory_store::emit_remember_profile(format!(
                "[LRC_PROFILING] rebuild_index_ms={:.3} index_terms={} memory_count={}",
                index_start.elapsed().as_secs_f64() * 1000.0,
                index.len(),
                base.len() + extra.len()
            ));
        }
    }

    /// 全量记忆是否已在内存中（`false` = 尚未加载，或刚被整体失效）
    ///
    /// 供写路径判断"能否用内存快照建索引而不读盘"。
    pub(crate) fn is_loaded(&self) -> bool {
        !self.dirty.load(Ordering::Acquire)
    }

    // ---------- 跨查询频次 ----------

    /// 借用跨查询统计（只读）
    ///
    /// 仅供 `ml` feature 下的语义压制路径使用，故同步门控以避免
    /// 默认 feature 构建出现 dead_code 告警（`-D warnings` 下为错误）。
    #[cfg(feature = "ml")]
    pub(crate) fn borrow_assoc(&self) -> RwLockReadGuard<'_, AssocFrequency> {
        self.assoc.read().unwrap()
    }

    /// 借用跨查询统计（可写）
    pub(crate) fn borrow_assoc_mut(&self) -> RwLockWriteGuard<'_, AssocFrequency> {
        self.assoc.write().unwrap()
    }

    /// 取跨查询统计快照（用于持久化）
    pub(crate) fn assoc_snapshot(&self) -> AssocFrequency {
        self.assoc.read().unwrap().clone()
    }

    // ---------- 纯函数辅助 ----------

    /// 字符级 bigram 集合（中文无空格分词时的兜底比较口径）
    pub(crate) fn content_bigrams(content: &str) -> HashSet<String> {
        content
            .to_lowercase()
            .chars()
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| format!("{}{}", w[0], w[1]))
            .collect()
    }

    /// 域候选索引的 term 提取：与评分侧 `tokenize_query` 保持一致。
    ///
    /// v8 检索质量修复：原先"含任一 CJK 即整体 bigram 切分"会把混合文本中的
    /// 英文标识符切碎（try_read → 2 字符碎片），导致索引 term 与评分 token
    /// 分叉、候选剪枝漏召回正确记忆。现直接复用 `tokenize_query`（混合语言下
    /// 保留英文/数字整词 + 中文 bigram），保证候选索引与 TF-IDF 评分口径统一。
    pub(crate) fn content_index_terms(content: &str) -> HashSet<String> {
        crate::memory_store::tokenize_query(content)
            .into_iter()
            .collect()
    }

    /// 域候选索引开关：默认启用 NOLANG（项目优先、无语言剪枝），
    /// 对应决策记忆 914a95dd（§5.11 方向 a 线上 A/B 验收通过后默认启用）。
    ///
    /// - `LRC_DOMAIN_CANDIDATE_INDEX_OFF=1`：逃生开关，完全关闭域候选索引，
    ///   回退到旧的全量/大词索引路径（与默认启用前行为一致）。
    /// - `LRC_DOMAIN_CANDIDATE_INDEX=1`（未设 NOLANG）：显式退回"项目+语言域"
    ///   带语言剪枝的普通 domain 模式。
    /// - `LRC_DOMAIN_CANDIDATE_INDEX_NOLANG=1`：显式启用 NOLANG（与默认同语义）。
    pub(crate) fn domain_index_enabled() -> bool {
        !std::env::var_os("LRC_DOMAIN_CANDIDATE_INDEX_OFF").is_some()
    }

    /// NOLANG 子模式：默认启用；仅当显式设置 LRC_DOMAIN_CANDIDATE_INDEX
    /// 而未设置 LRC_DOMAIN_CANDIDATE_INDEX_NOLANG 时，退回带语言剪枝的
    /// 普通 domain 模式（历史兼容语义）。
    pub(crate) fn domain_index_nolang_enabled() -> bool {
        if !Self::domain_index_enabled() {
            return false;
        }
        if std::env::var_os("LRC_DOMAIN_CANDIDATE_INDEX").is_some()
            && std::env::var_os("LRC_DOMAIN_CANDIDATE_INDEX_NOLANG").is_none()
        {
            return false;
        }
        true
    }

    /// 单条记忆的索引 term 集合
    ///
    /// 三种模式（与修前一致的部分保持不变）：
    /// - 域候选索引开启（默认）：`tokenize_query` 口径 + **检索可达 term 补齐**（见
    ///   [`Self::recall_reachable_terms`]）。补齐不删任何既有 term，故
    ///   `find_similar` 的候选集只增不减，最终仍由 Jaccard 判定，行为不变。
    /// - 域候选索引关闭且正文含 CJK：字符级 bigram（旧口径）。
    /// - 域候选索引关闭且正文不含 CJK：空集（旧口径，跳过英文记忆）。
    pub(crate) fn index_terms_for_memory(memory: &Memory) -> HashSet<String> {
        if !Self::domain_index_enabled() {
            return if memory.content.chars().any(|c| is_cjk_char(&c)) {
                Self::content_bigrams(&memory.content)
            } else {
                HashSet::new()
            };
        }
        let lower = memory.content.to_lowercase();
        let mut terms: HashSet<String> = crate::memory_store::tokenize_query(&lower)
            .into_iter()
            .collect();
        Self::recall_reachable_terms(&lower, &mut terms);
        terms
    }

    /// 补齐"评分侧可达、但 `tokenize_query` 未产出"的索引 term（S6）
    ///
    /// ## 为什么必须补齐
    ///
    /// 快路候选裁剪的等价性前提是「索引 term 命中」⊇「`count_word_occurrences > 0`」。
    /// 但两套口径并不天然同源，实测有两类漏召回：
    ///
    /// 1. **低 CJK 比例的长文本里的中文段**：`tokenize_query` 对 CJK 比例 <30% 的
    ///    文本走空格分词（`技术文档` 不产 bigram），而评分对 CJK token 用**子串**匹配
    ///    ⇒ 查询 bigram `技术` 能在正文里命中，索引里却不存在该 term。
    /// 2. **字母数字连写**：评分对长度 ≥3 的 ASCII 词用**词边界**判定，而
    ///    `is_alpha_byte` 只把 ASCII 字母视为"词内字符"⇒ `abc123` 中的 `abc`
    ///    会被判为命中，但索引里只有整词 `abc123`。
    ///
    /// 本函数只**加** term（幂等），不改变任何既有 term，因此：
    /// - 候选裁剪不会再漏掉打分 >0 的记忆；
    /// - 索引变成打分口径的**超集**，交集操作只会多算不会少算。
    fn recall_reachable_terms(lower: &str, terms: &mut HashSet<String>) {
        // 1) ASCII 词的字母/数字分段（数字在 `is_alpha_byte` 中被视为词边界）
        let mut pieces: Vec<String> = Vec::new();
        for term in terms.iter() {
            let is_ascii_run = !term.is_empty()
                && term
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !is_ascii_run {
                continue;
            }
            let has_digit = term.chars().any(|c| c.is_ascii_digit());
            let has_alpha = term.chars().any(|c| c.is_ascii_alphabetic());
            if !(has_digit && has_alpha) {
                continue;
            }
            let mut piece = String::new();
            let mut prev_digit: Option<bool> = None;
            for c in term.chars() {
                let is_digit = c.is_ascii_digit();
                if prev_digit.is_some_and(|pd| pd != is_digit) && !piece.is_empty() {
                    pieces.push(std::mem::take(&mut piece));
                }
                piece.push(c);
                prev_digit = Some(is_digit);
            }
            if !piece.is_empty() {
                pieces.push(piece);
            }
        }
        terms.extend(pieces);

        // 2) 低 CJK 比例文本：补 CJK 相邻字符 bigram。
        //    比例 ≥30% 时 `tokenize_query` 已走 bigram 分支、这些 term 已存在，
        //    再全量扫一遍只会重复分配 ⇒ 直接跳过。
        if crate::memory_store::cjk_ratio(lower) >= 0.3 {
            return;
        }
        let chars: Vec<char> = lower.chars().collect();
        for window in chars.windows(2) {
            if is_cjk_char(&window[0]) && is_cjk_char(&window[1]) {
                terms.insert(format!("{}{}", window[0], window[1]));
            }
        }
    }
}

/// 是否 CJK 字符（与 `cjk_ratio` / `contains_word` 的判定范围一致）
fn is_cjk_char(c: &char) -> bool {
    let cp = *c as u32;
    (0x4E00..=0x9FFF).contains(&cp)
        || (0x3400..=0x4DBF).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_types::{Importance, MemoryType};

    /// 构造一条最小可用的测试记忆
    fn make_memory(content: &str) -> Memory {
        Memory::new(
            content.to_string(),
            MemoryType::Fact,
            None,
            Vec::new(),
            Importance::new(3),
            None,
        )
    }

    /// 建议二（v0.9.13）：同版本多次取快照必须共享同一份分配（零拷贝）。
    ///
    /// 修前 `snapshot()` 每次都 `clone()` 整库；一次 enrich 内会取十余次，
    /// 全在存储锁内。这里用 `Arc::ptr_eq` 锁定"不再逐次深拷贝"这一性质，
    /// 一旦有人把 `snapshot()` 改回克隆，本测试立刻失败。
    #[test]
    fn 同版本快照共享同一份分配() {
        let cache = MemoryStoreCache::new(AssocFrequency::default());
        cache.store(vec![make_memory("甲"), make_memory("乙")], None);

        let first = cache.snapshot();
        let second = cache.snapshot();

        assert_eq!(first.len(), 2, "快照应包含全部两条记忆");
        assert!(
            Arc::ptr_eq(&first, &second),
            "同版本快照必须共享同一份分配（回归：又变回逐次整库深拷贝）"
        );
        assert_eq!(first[0].content, second[0].content, "共享分配内容必须一致");
    }

    /// 建议二（v0.9.13）：写入必须走写时复制——
    /// 旧快照保持写入前的内容（快照隔离语义与修前完全一致），
    /// 新快照能看到新增条目，且与旧快照不共享分配。
    #[test]
    fn 写入后旧快照不受影响且新快照可见新条目() {
        let cache = MemoryStoreCache::new(AssocFrequency::default());
        cache.store(vec![make_memory("甲")], None);

        let before = cache.snapshot();
        let version_before = cache.data_version();
        cache.append_new(&[make_memory("乙")], None);

        assert!(
            cache.data_version() != version_before,
            "追加新记忆必须推进 data_version（关联三表缓存依赖它失效）"
        );

        let after = cache.snapshot();
        assert_eq!(before.len(), 1, "旧快照不应看到写入后的新条目");
        assert_eq!(after.len(), 2, "新快照必须包含新增条目");
        assert_eq!(after[1].content, "乙");
        assert!(
            !Arc::ptr_eq(&before, &after),
            "写入后应产生新分配（写时复制），否则旧快照会被写入污染"
        );
    }

    /// 2026-11-05（压测根因）：单条删除必须**增量**更新缓存，不得标脏。
    ///
    /// 修前 `forget` 走 `mark_dirty_preserving_index()` ⇒ 下一次 `load_cached()`
    /// 整库重解析；16 并发删除时每次删除都触发一次全库重载（P50 撞满 6s）。
    /// 本测试锁定"删除后缓存仍为已加载状态、且该条已从快照/映射/特征中移除"。
    #[test]
    fn 单条删除后缓存保持已加载且条目被移除() {
        let cache = MemoryStoreCache::new(AssocFrequency::default());
        let a = make_memory("甲");
        let b = make_memory("乙");
        let id_a = a.id.clone();
        let id_b = b.id.clone();
        cache.store(vec![a, b], None);
        // 触发一次惰性特征计算，确保删除要清理它
        let all = cache.snapshot();
        for m in all.iter() {
            let _ = cache.recall_document(m);
        }
        assert!(cache.is_loaded(), "store 后应为已加载");

        cache.remove(&id_a, None);

        assert!(
            cache.is_loaded(),
            "增量删除后缓存必须仍为已加载（回归：又变回标脏 ⇒ 下次整库重载）"
        );
        let after = cache.snapshot();
        assert_eq!(after.len(), 1, "快照应只剩一条");
        assert_eq!(after[0].id, id_b, "应保留未被删除的那条");
        assert!(!cache.contains_id(&id_a), "被删条目不应再存在");
        assert!(
            !cache.documents.read().unwrap().contains_key(&id_a),
            "被删条目的文本特征应被清理"
        );
        assert!(
            !cache.id_positions_snapshot().contains_key(&id_a),
            "被删条目不应残留在 id→position 映射中"
        );
        assert_eq!(
            cache.id_positions_snapshot().get(&id_b).copied(),
            Some(0),
            "删除后剩余条目的 position 必须前移并重建"
        );
    }

    /// 2026-10-28：追加新记忆**不得**清空既有文本特征缓存。
    ///
    /// 修前 `append_new` 会 `documents.clear()`，使下一次检索把全库重新
    /// lowercase + 计数（449k 条库上是「快路/深路」的主要耗时之一）。
    /// 新条目带新 UUID，缺失特征由 `recall_document()` 惰性补算即可。
    #[test]
    fn 追加新记忆不清空既有文本特征() {
        let cache = MemoryStoreCache::new(AssocFrequency::default());
        let a = make_memory("甲");
        let b = make_memory("乙");
        let id_a = a.id.clone();
        let id_b = b.id.clone();
        cache.store(vec![a, b], None);

        // 触发一次惰性计算，填充两条的特征
        let all = cache.snapshot();
        for m in all.iter() {
            let doc = cache.recall_document(m);
            assert!(!doc.normalized_content.is_empty(), "特征不应为空");
        }
        let cached_before = cache.documents.read().unwrap().len();
        assert_eq!(cached_before, 2, "两条记忆的特征都应已缓存");

        let c = make_memory("丙");
        let id_c = c.id.clone();
        cache.append_new(&[c], None);

        assert_eq!(
            cache.documents.read().unwrap().len(),
            cached_before,
            "追加新记忆不得清空既有特征缓存（回归：又变回每写一次就全库重算）"
        );
        assert!(
            cache.documents.read().unwrap().contains_key(&id_a)
                && cache.documents.read().unwrap().contains_key(&id_b),
            "既有记忆的特征必须仍在缓存中"
        );
        assert!(
            !cache.documents.read().unwrap().contains_key(&id_c),
            "新条目特征应留待惰性计算，而不是被写入路径清空整表"
        );

        // 惰性补算：新条目取特征时自动填入，且与独立计算的语义一致
        let all_after = cache.snapshot();
        let new_mem = all_after
            .iter()
            .find(|m| m.id == id_c)
            .expect("新条目应存在");
        let doc = cache.recall_document(new_mem);
        assert_eq!(&*doc.normalized_content, "丙", "新条目特征应按正文计算");
        assert!(
            cache.documents.read().unwrap().contains_key(&id_c),
            "惰性计算后新条目特征应写入缓存"
        );
    }
}
