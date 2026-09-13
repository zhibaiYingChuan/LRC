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
    memory: RefCell<Vec<Memory>>,
    /// 缓存脏标记：任何写操作后置 true，读操作前检查
    dirty: Cell<bool>,
    /// 候选 term 倒排索引（默认启用域候选索引；最终仍由 Jaccard 判定）
    index: RefCell<BigramIndex>,
    /// 索引脏标记
    index_dirty: Cell<bool>,
    /// recall 文本特征缓存，按 Memory ID 复用规范化正文和文档长度
    documents: RefCell<HashMap<String, RecallDocument>>,
    /// 跨查询命中频次统计（独立文件持久化，`Memory` 零改动）
    assoc: RefCell<AssocFrequency>,
}

impl MemoryStoreCache {
    /// 构造空缓存（全部标记为脏，首次读取时惰性加载/重建）
    pub(crate) fn new(assoc_frequency: AssocFrequency) -> Self {
        Self {
            memory: RefCell::new(Vec::new()),
            dirty: Cell::new(true), // 初始为脏，首次读取时加载
            index: RefCell::new(HashMap::new()),
            index_dirty: Cell::new(true),
            documents: RefCell::new(HashMap::new()),
            assoc: RefCell::new(assoc_frequency),
        }
    }

    // ---------- 全量记忆快照 ----------

    /// 全量记忆缓存是否已失效
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    /// 用新加载的全量记忆替换缓存并清除脏标记
    pub(crate) fn store(&self, memories: Vec<Memory>) {
        *self.memory.borrow_mut() = memories;
        self.dirty.set(false);
    }

    /// 返回缓存副本（调用方持有独立所有权，不占用 borrow）
    pub(crate) fn snapshot(&self) -> Vec<Memory> {
        self.memory.borrow().clone()
    }

    // ---------- 失效 ----------

    /// 全量失效：任何写操作（保存/删除/修改）后调用
    ///
    /// 同时清空 recall 文本特征（正文可能已变）。
    pub(crate) fn invalidate(&self) {
        self.dirty.set(true);
        self.index_dirty.set(true);
        self.documents.borrow_mut().clear();
    }

    /// 部分失效：仅失效全量快照与文本特征，**保留**倒排索引
    ///
    /// 适用于"增删单条记忆且索引已增量同步"的路径——此时索引仍正确，
    /// 重建代价（O(N) 全量分词）可省。
    pub(crate) fn mark_dirty_preserving_index(&self) {
        self.dirty.set(true);
        self.documents.borrow_mut().clear();
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
