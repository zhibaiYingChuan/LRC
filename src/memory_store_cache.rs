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
//! **关于"RefCell → Sync 前置条件"的更正（v0.9.7 实测）**：
//!   报告原判「字段级拆分需先完成 `RefCell → Sync` 并发模型改造」。实测全仓
//!   `MemoryStore` 的共享方式**全部 8 处**均为 `Arc<Mutex<MemoryStore<P>>>`；
//!   `Mutex<T>: Sync` 只要求 `T: Send`（`RefCell`/`Cell` 均为 `Send`），
//!   **并不要求 `T: Sync`**。故该"前置条件"不成立，字段级拆分可独立完成。
//!   本模块因此**保留** `RefCell`/`Cell` 的内部可变性设计——它正是让 `&self`
//!   方法（如 `load_cached`）能惰性刷新缓存的手段，外层 `Mutex` 已提供互斥。
//!
//! 不负责：
//!   - 持久化读写（由 `MemoryStore` 持有的 `Persistence` 负责）
//!   - 检索评分（由 `MemoryStore` 的召回路径负责）

use crate::memory_types::Memory;
use crate::persistence::AssocFrequency;
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// 记忆文档特征（recall 文本缓存项）
///
/// 缓存「规范化正文 + 文档长度」，避免同一记忆在同一/多次查询中被反复
/// 做 `to_lowercase` 与分词——这两步是检索热路径上最常复用的纯计算。
pub(crate) struct RecallDocument {
    /// 小写规范化后的正文
    pub(crate) normalized_content: String,
    /// 文档长度（token 数，与评分口径一致）
    pub(crate) token_count: usize,
}

/// 候选索引类型：term → 记忆 ID 集合
pub(crate) type BigramIndex = HashMap<String, HashSet<String>>;

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
    /// **读取路径零拷贝**；写入走 `Arc::make_mut`（写时复制），
    /// 因写路径与读路径同受外层 `Mutex` 串行化，快照不会跨锁存活 ⇒ 实际不发生拷贝。
    memory: RefCell<Arc<Vec<Memory>>>,
    /// 缓存脏标记：任何写操作后置 true，读操作前检查
    dirty: Cell<bool>,
    /// 加载本缓存时数据源的指纹（v0.9.11 / CR-42）：
    /// 与持久层当前指纹不符 ⇒ 说明**别的进程**改过数据，必须重载。
    /// `None` 表示后端不提供指纹（则只看 `dirty`，行为与修前一致）。
    source_fingerprint: Cell<Option<u64>>,
    /// 候选 term 倒排索引（默认启用域候选索引；最终仍由 Jaccard 判定）
    index: RefCell<BigramIndex>,
    /// 索引脏标记
    index_dirty: Cell<bool>,
    /// 数据版本号（v0.9.13）：**任何**数据变更（写入/删除/外部重载）后单调递增。
    ///
    /// 用途：让"从全库派生的统计表"（hub 实体表、自动事件表、artifact 表）
    /// 能以**版本相等**判定缓存是否仍与当前数据同源，从而跳过每次检索都
    /// 重建 O(N) 表的代价（实测 449k 条库上建表需 17s）。
    ///
    /// 为什么放在本模块而不是 `MemoryStore`：所有写入路径最终都会经过本模块的
    /// `invalidate` / `mark_dirty_preserving_index` / `append_new` / `store`
    /// 四个入口之一 ⇒ 在此递增是**唯一不会漏**的位置。
    data_version: Cell<u64>,
    /// recall 文本特征缓存，按 Memory ID 复用规范化正文和文档长度
    documents: RefCell<HashMap<String, RecallDocument>>,
    /// 跨查询命中频次统计（独立文件持久化，`Memory` 零改动）
    assoc: RefCell<AssocFrequency>,
}

impl MemoryStoreCache {
    /// 构造空缓存（全部标记为脏，首次读取时惰性加载/重建）
    pub(crate) fn new(assoc_frequency: AssocFrequency) -> Self {
        Self {
            memory: RefCell::new(Arc::new(Vec::new())),
            dirty: Cell::new(true), // 初始为脏，首次读取时加载
            source_fingerprint: Cell::new(None),
            index: RefCell::new(HashMap::new()),
            index_dirty: Cell::new(true),
            data_version: Cell::new(0),
            documents: RefCell::new(HashMap::new()),
            assoc: RefCell::new(assoc_frequency),
        }
    }

    /// 当前数据版本号（每次数据变更后递增；语义见字段文档）
    pub(crate) fn data_version(&self) -> u64 {
        self.data_version.get()
    }

    /// 递增数据版本号（所有失效入口都必须调用）
    fn bump_data_version(&self) {
        self.data_version
            .set(self.data_version.get().wrapping_add(1));
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
        if self.dirty.get() {
            return true;
        }
        match (current, self.source_fingerprint.get()) {
            (Some(now), Some(loaded)) => now != loaded,
            (Some(_), None) => true, // 首次拿到指纹（此前未记录）⇒ 保守重载一次
            _ => false,
        }
    }

    /// 用新加载的全量记忆替换缓存并清除脏标记，同时记录数据源指纹
    pub(crate) fn store(&self, memories: Vec<Memory>, source_fingerprint: Option<u64>) {
        *self.memory.borrow_mut() = Arc::new(memories);
        self.source_fingerprint.set(source_fingerprint);
        self.dirty.set(false);
        self.bump_data_version();
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
    pub(crate) fn snapshot(&self) -> Arc<Vec<Memory>> {
        Arc::clone(&self.memory.borrow())
    }

    /// 仅判断指定 ID 是否存在（**不克隆**任何 `Memory`）
    ///
    /// 与 `snapshot().iter().any(|m| m.id == id)` 语义完全相同，但后者会先
    /// 深拷贝整库（`Memory` 含正文字符串、实体列表、卦象等），在**逐条**执行的
    /// 存在性校验路径上是纯浪费的内存抖动与锁持有时间。
    pub(crate) fn contains_id(&self, id: &str) -> bool {
        self.memory.borrow().iter().any(|m| m.id == id)
    }

    /// 缓存中的记忆条数（O(1)，**不克隆**）
    pub(crate) fn len(&self) -> usize {
        self.memory.borrow().len()
    }

    /// 收集合成类记忆的摘要文本（**不克隆整库**）
    ///
    /// 为什么需要：`/v1/memories/consolidate` 的 Phase 3 原先先
    /// `list_memories()`（整库深拷贝）再 `total_count()`（又一次整库深拷贝），
    /// 在 28 万条库上实测白付 11~13 秒并长时间持锁。这里只按类型过滤后
    /// 克隆命中的摘要字符串（通常为 0 条）。
    pub(crate) fn synthesis_summaries(&self) -> Vec<String> {
        self.memory
            .borrow()
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
        // ★ v0.9.13（建议二）：写时复制。若此刻恰有快照在外（正常不会——
        //   读写同受外层 Mutex 串行化），`make_mut` 才会整库克隆一次；
        //   否则原地追加，代价与修前一致。
        Arc::make_mut(&mut self.memory.borrow_mut()).extend_from_slice(new);
        // 新增条目的文本特征尚未计算；清空后按需惰性重建（不影响正确性）。
        self.documents.borrow_mut().clear();
        self.source_fingerprint.set(source_fingerprint);
        self.dirty.set(false);
        self.bump_data_version();
    }

    // ---------- 失效 ----------

    /// 全量失效：任何写操作（保存/删除/修改）后调用
    ///
    /// 同时清空 recall 文本特征（正文可能已变）。
    pub(crate) fn invalidate(&self) {
        self.dirty.set(true);
        self.index_dirty.set(true);
        self.documents.borrow_mut().clear();
        self.bump_data_version();
    }

    /// 部分失效：仅失效全量快照与文本特征，**保留**倒排索引
    ///
    /// 适用于"增删单条记忆且索引已增量同步"的路径——此时索引仍正确，
    /// 重建代价（O(N) 全量分词）可省。
    pub(crate) fn mark_dirty_preserving_index(&self) {
        self.dirty.set(true);
        self.documents.borrow_mut().clear();
        self.bump_data_version();
    }

    // ---------- recall 文本特征 ----------

    /// 取（或惰性计算并缓存）指定记忆的文本特征
    pub(crate) fn recall_document(&self, memory: &Memory) -> RecallDocument {
        let mut documents = self.documents.borrow_mut();
        if let Some(document) = documents.get(&memory.id) {
            return RecallDocument {
                normalized_content: document.normalized_content.clone(),
                token_count: document.token_count,
            };
        }
        let normalized_content = memory.content.to_lowercase();
        let token_count = crate::memory_store::doc_token_count(&normalized_content);
        let document = RecallDocument {
            normalized_content,
            token_count,
        };
        documents.insert(
            memory.id.clone(),
            RecallDocument {
                normalized_content: document.normalized_content.clone(),
                token_count: document.token_count,
            },
        );
        document
    }

    // ---------- 倒排索引 ----------

    /// 索引是否待重建
    pub(crate) fn index_is_dirty(&self) -> bool {
        self.index_dirty.get()
    }

    /// 借用倒排索引（只读）
    pub(crate) fn borrow_index(&self) -> Ref<'_, BigramIndex> {
        self.index.borrow()
    }

    /// 增量加入一条记忆的 term
    pub(crate) fn add_to_index(&self, memory: &Memory) {
        if self.index_dirty.get() {
            return;
        }
        let mut index = self.index.borrow_mut();
        for term in Self::index_terms_for_memory(memory) {
            index.entry(term).or_default().insert(memory.id.clone());
        }
    }

    /// 增量移除一条记忆的 term
    pub(crate) fn remove_from_index(&self, memory: &Memory) {
        if self.index_dirty.get() {
            return;
        }
        let mut index = self.index.borrow_mut();
        for term in Self::index_terms_for_memory(memory) {
            if let Some(ids) = index.get_mut(&term) {
                ids.remove(&memory.id);
                if ids.is_empty() {
                    index.remove(&term);
                }
            }
        }
    }

    /// 增量替换一条记忆在索引中的 term
    pub(crate) fn replace_in_index(&self, old: &Memory, new: &Memory) {
        if self.index_dirty.get() {
            return;
        }
        self.remove_from_index(old);
        self.add_to_index(new);
    }

    /// 全量重建倒排索引
    pub(crate) fn rebuild_index(&self, all: &[Memory]) {
        let index_start = std::time::Instant::now();
        let mut index = self.index.borrow_mut();
        index.clear();
        let use_domain_index = Self::domain_index_enabled();
        for memory in all {
            let terms = if use_domain_index {
                Self::content_index_terms(&memory.content)
            } else {
                if !memory.content.chars().any(|c| {
                    let code = c as u32;
                    (0x4E00..=0x9FFF).contains(&code)
                }) {
                    continue;
                }
                Self::content_bigrams(&memory.content)
            };
            for term in terms {
                index.entry(term).or_default().insert(memory.id.clone());
            }
        }
        self.index_dirty.set(false);
        if std::env::var_os("LRC_PROFILE_REMEMBER").is_some() {
            crate::memory_store::emit_remember_profile(format!(
                "[LRC_PROFILING] rebuild_index_ms={:.3} index_terms={} memory_count={}",
                index_start.elapsed().as_secs_f64() * 1000.0,
                index.len(),
                all.len()
            ));
        }
    }

    // ---------- 跨查询频次 ----------

    /// 借用跨查询统计（只读）
    ///
    /// 仅供 `ml` feature 下的语义压制路径使用，故同步门控以避免
    /// 默认 feature 构建出现 dead_code 告警（`-D warnings` 下为错误）。
    #[cfg(feature = "ml")]
    pub(crate) fn borrow_assoc(&self) -> Ref<'_, AssocFrequency> {
        self.assoc.borrow()
    }

    /// 借用跨查询统计（可写）
    pub(crate) fn borrow_assoc_mut(&self) -> RefMut<'_, AssocFrequency> {
        self.assoc.borrow_mut()
    }

    /// 取跨查询统计快照（用于持久化）
    pub(crate) fn assoc_snapshot(&self) -> AssocFrequency {
        self.assoc.borrow().clone()
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
    pub(crate) fn index_terms_for_memory(memory: &Memory) -> HashSet<String> {
        if Self::domain_index_enabled() {
            Self::content_index_terms(&memory.content)
        } else if memory.content.chars().any(|c| {
            let code = c as u32;
            (0x4E00..=0x9FFF).contains(&code)
        }) {
            Self::content_bigrams(&memory.content)
        } else {
            HashSet::new()
        }
    }
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
}
