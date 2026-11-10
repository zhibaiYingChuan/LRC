//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现语义向量索引与后台编码服务，属于公开层 (Layer 1)。
//! ============================================================
//!
//! # 为什么需要本模块（全局视角，不是补丁）
//!
//! 检索排序长期缺少**可靠的语义信号**：
//!   - fast 路（BM25）有效，但只认词面；
//!   - deep 路（洛书 9 维）**不读语义**——实测打乱字符顺序后分类 100% 不变；
//!   - 既有的 bge 句向量能力只用于"联想根门禁旁路"，且侧车缓存
//!     `semantic_vectors.json` 从未落盘（回填速率结构性追不上库规模）。
//!
//! 本模块把"语义向量"从**旁路**提升为**一等基础设施**，解决三件事：
//!
//! 1. **存储格式**：JSON 侧车在 7k 库上约 150MB，每次检索解析一遍不可行。
//!    改为紧凑二进制（`semantic_vectors.bin`），启动加载一次常驻内存，
//!    单次全库余弦检索 ≈ 数毫秒。
//! 2. **写入时编码**：新记忆写入后**异步入队**编码，缓存随库增长，
//!    不再依赖"每次请求 4s 预算"的追赶式回填。
//! 3. **一次性回填**：启动时把存量记忆交给后台工作线程批量编码
//!    （`encode_embedding_batch`，分片 32），可断点续跑、原子落盘。
//!
//! # 不变量
//!
//! - **不参与写入语义**：本模块只读记忆内容、只写自己的侧车文件，
//!   绝不修改记忆、不写回任何排序状态。
//! - **诚实降级**：ML 编码器不可用 / 索引为空 ⇒ 检索侧返回 `None`，
//!   调用方回退既有两路，行为与未引入本模块时一致。
//! - **维度治理**：换模型后维度变化 ⇒ 旧向量整体失效（按维度过滤），
//!   绝不混用不同向量空间的分值。
//! - **原子落盘**：任何时刻崩溃都不会留下半个文件（`write_atomic`）。

use crate::lock_ext::LockExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use crate::memory_types::Memory;

/// 二进制索引文件名（与 memories.json 同目录）。
pub const SEMANTIC_INDEX_FILE: &str = "semantic_vectors.bin";

/// 增量日志文件名（追加写；压实后清空）。
pub const SEMANTIC_JOURNAL_FILE: &str = "semantic_vectors.journal.bin";

/// 文件魔数（8 字节，用于快速识别与拒绝损坏文件）。
const MAGIC: &[u8; 8] = b"LRCSEM01";

/// 增量日志魔数（与基文件区分，防止误读）。
const MAGIC_JOURNAL: &[u8; 8] = b"LRCSEMJ1";

/// 增量日志的字节上限（超过即触发一次整份压实）。
///
/// 默认 64MB，与主库 / 图日志同口径；可用 `LRC_SEMANTIC_JOURNAL_MAX_BYTES` 覆盖。
fn journal_max_bytes() -> u64 {
    std::env::var("LRC_SEMANTIC_JOURNAL_MAX_BYTES")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 1024 * 1024)
        .unwrap_or(64 * 1024 * 1024)
}

/// 结构版本（格式变更时递增，旧文件整体丢弃并重建）。
const FORMAT_VERSION: u32 = 1;

/// 后台编码的批大小（与 `EMBEDDING_BATCH_CHUNK` 对齐，避免二次分片）。
const ENCODE_BATCH: usize = 32;

/// 每写入多少条向量落盘一次（降低 IO 频率，同时限制崩溃丢失窗口）。
///
/// # 取值依据（阶段 0 核对后修正）
///
/// 索引是**缓存**：检索读内存，落盘只为重启后免于重新编码。因此落盘频率
/// 只需兼顾"崩溃后重编码量"与"IO 放大"。
///
/// 原值 64 在 7k 库上意味着每 64 条就整份重写 57MB（回填期间约每 30 秒一次），
/// 是本次新增代码的 IO 瓶颈。改为 **512**：重写频率降 8 倍，
/// 崩溃最多丢 512 条（重编码约 1~2 分钟），可接受。
///
/// 短生命周期进程（stdio 评测）在查询后即被杀，但查询读的是内存索引，
/// 不依赖落盘；空闲 10 秒的定时 flush 仍会兜底。
const FLUSH_EVERY: usize = 512;

/// 工作线程空闲轮询间隔（用于把脏数据定期落盘）。
const IDLE_TICK: Duration = Duration::from_secs(10);

/// bge-zh 官方检索指令前缀（查询侧加、文档侧不加）。
///
/// 与 `memory_store::BGE_QUERY_INSTRUCTION` 同源同值：两处必须一致，
/// 否则"查询向量"与"文档向量"来自不同约定，余弦不可比。
const BGE_QUERY_INSTRUCTION: &str = "为这个句子生成表示以用于检索相关文章：";

/// 语义向量索引（内存常驻 + 二进制持久化）。
///
/// # 多向量（分窗）设计
///
/// 一条记忆可能很长（实测平均 3.7KB），而 bge 的注意力成本随长度**平方**增长。
/// 若整条编码，单条 4.4s、全库 8.5 小时；若只截首段，尾部信息丢失。
/// 折中：每条记忆取**首/中/尾**三个窗口（各 [`CHUNK_CHARS`] 字），分别编码，
/// 检索时对同一记忆的多个向量取**最大余弦**（max-pooling）。
///
/// 这样既覆盖了长文本的两端与中段，又把单条成本压到 3 个短窗口
/// （实测短文本批量 ≈37ms/条 ⇒ 单条约 0.1s，全库 7k ≈ 13 分钟）。
///
/// 向量在**插入时归一化**，因此检索时余弦相似度退化为点积。
#[derive(Debug, Default)]
pub struct SemanticIndex {
    /// 向量维度（0 = 尚未确定）
    dim: usize,
    /// 记忆 id（与 `vectors` 平行；同一 id 可出现多次 = 多个窗口）
    ids: Vec<String>,
    /// 归一化后的句向量（与 `ids` 平行）
    vectors: Vec<Vec<f32>>,
    /// id → 该记忆全部窗口的下标
    pos: HashMap<String, Vec<usize>>,
}

/// 单个窗口的字符数。
///
/// # 取值依据（实测定标，非拍脑袋）
///
/// bge 的注意力成本随长度**平方**增长。本机实测（bge-base-zh / CPU）：
/// 40 字 ≈ 0.25s/条、200 字 ≈ 4.4s/条（25×，与平方项一致）。
/// 取 **160 字**：既覆盖一段完整语义（约 1~2 句），又把单窗口成本压到
/// 200 字的约 0.64 倍。配合"首/中/尾"三窗口，长记忆仍能覆盖两端与中段。
pub const CHUNK_CHARS: usize = 160;

impl SemanticIndex {
    /// 向量维度（0 表示空索引）。
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// 已索引的**记忆条数**（不是窗口数）。
    pub fn len(&self) -> usize {
        self.pos.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    /// 取某条记忆的首个窗口向量（未索引返回 None）。
    pub fn get(&self, id: &str) -> Option<&[f32]> {
        self.pos
            .get(id)
            .and_then(|indices| indices.first())
            .map(|&i| self.vectors[i].as_slice())
    }

    /// 追加一条向量（同一 id 可多次调用 = 多个窗口）。
    ///
    /// 维度不一致的向量**直接拒绝**（换模型后旧向量不可复用，
    /// 由调用方先 `clear` 再重建，绝不混用两个向量空间）。
    pub fn insert(&mut self, id: String, vector: Vec<f32>) {
        if vector.is_empty() {
            return;
        }
        if self.dim == 0 {
            self.dim = vector.len();
        }
        if vector.len() != self.dim {
            return;
        }
        let normalized = normalize(vector);
        let index = self.vectors.len();
        self.pos.entry(id.clone()).or_default().push(index);
        self.ids.push(id);
        self.vectors.push(normalized);
    }

    /// 清空（维度变更时整体重建）。
    pub fn clear(&mut self) {
        self.dim = 0;
        self.ids.clear();
        self.vectors.clear();
        self.pos.clear();
    }

    /// 全库余弦检索：返回 `(id, 相似度)`，按相似度降序，最多 `top_k` 条。
    ///
    /// 同一记忆的多个窗口取**最大余弦**（max-pooling）——只要任一窗口与查询
    /// 语义接近，该记忆就应被召回。
    ///
    /// `query` 必须与索引同维度；不同维度返回空（诚实降级）。
    pub fn search(&self, query: &[f32], top_k: usize) -> Vec<(String, f32)> {
        if self.dim == 0 || query.len() != self.dim || top_k == 0 {
            return Vec::new();
        }
        let q = normalize(query.to_vec());
        let mut best: HashMap<&str, f32> = HashMap::with_capacity(self.pos.len());
        for (i, v) in self.vectors.iter().enumerate() {
            let score = dot(&q, v);
            let id = self.ids[i].as_str();
            let entry = best.entry(id).or_insert(f32::NEG_INFINITY);
            if score > *entry {
                *entry = score;
            }
        }
        let mut scored: Vec<(String, f32)> = best
            .into_iter()
            .map(|(id, score)| (id.to_string(), score))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    /// 从二进制文件加载（缺失/损坏/版本不符 ⇒ 空索引）。
    ///
    /// ★v3：基文件 + 增量日志重放。日志是自上次压实以来的新增向量，
    /// 两者合并才是完整索引（与主库 / 图日志同纪律）。
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(SEMANTIC_INDEX_FILE);
        let mut index = match std::fs::read(&path) {
            Ok(bytes) => Self::decode(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        index.replay_journal(data_dir);
        index
    }

    /// 重放增量日志（把日志里的向量合入索引）。
    ///
    /// 日志头含维度；与基文件维度不一致（换模型）⇒ 整份跳过，
    /// 由调用方按"未索引"重新编码，绝不混用两个向量空间。
    fn replay_journal(&mut self, data_dir: &Path) {
        let path = data_dir.join(SEMANTIC_JOURNAL_FILE);
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        if bytes.len() < 16 || &bytes[..8] != MAGIC_JOURNAL {
            return;
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap_or([0; 4]));
        if version != FORMAT_VERSION {
            return;
        }
        let dim = u32::from_le_bytes(bytes[12..16].try_into().unwrap_or([0; 4])) as usize;
        if dim == 0 || dim > 8192 {
            return;
        }
        if self.dim != 0 && self.dim != dim {
            return;
        }
        let mut cursor = 16usize;
        while cursor + 2 <= bytes.len() {
            let id_len = u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap()) as usize;
            cursor += 2;
            if cursor + id_len + dim * 4 > bytes.len() {
                break; // 截断的尾记录：跳过（日志是缓存，不因半条记录整体失败）
            }
            let Ok(id) = String::from_utf8(bytes[cursor..cursor + id_len].to_vec()) else {
                break;
            };
            cursor += id_len;
            let mut vector = Vec::with_capacity(dim);
            for _ in 0..dim {
                vector.push(f32::from_le_bytes(
                    bytes[cursor..cursor + 4].try_into().unwrap(),
                ));
                cursor += 4;
            }
            self.insert(id, vector);
        }
    }

    /// 原子落盘（失败只记日志，不影响主流程）。
    ///
    /// ★v3：整份落盘后清空增量日志（日志里的向量已被吸收）。
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = data_dir.join(SEMANTIC_INDEX_FILE);
        let bytes = self.encode();
        crate::atomic_file::write_atomic(&path, &bytes)?;
        let _ = std::fs::remove_file(data_dir.join(SEMANTIC_JOURNAL_FILE));
        Ok(())
    }

    /// 追加一批向量到增量日志（O(新增条数)，不随索引规模增长）。
    ///
    /// 日志格式：`MAGIC_JOURNAL(8) + version(4) + dim(4)` 头，
    /// 之后每条记录 `[u16 id_len][id][dim*4 vector]`。
    fn append_journal(data_dir: &Path, items: &[(String, Vec<f32>)]) -> std::io::Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let dim = items[0].1.len();
        if dim == 0 {
            return Ok(());
        }
        let path = data_dir.join(SEMANTIC_JOURNAL_FILE);
        let exists = path.exists();
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        if !exists {
            let mut header = Vec::with_capacity(16);
            header.extend_from_slice(MAGIC_JOURNAL);
            header.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
            header.extend_from_slice(&(dim as u32).to_le_bytes());
            file.write_all(&header)?;
        }
        let mut buf: Vec<u8> = Vec::with_capacity(items.len() * (2 + 36 + dim * 4));
        for (id, vector) in items {
            if vector.len() != dim {
                continue; // 维度不一致：跳过（绝不混用两个向量空间）
            }
            let id_bytes = id.as_bytes();
            let id_len = id_bytes.len().min(u16::MAX as usize);
            buf.extend_from_slice(&(id_len as u16).to_le_bytes());
            buf.extend_from_slice(&id_bytes[..id_len]);
            for x in vector {
                buf.extend_from_slice(&x.to_le_bytes());
            }
        }
        file.write_all(&buf)?;
        file.flush()?;
        file.sync_data()?;
        Ok(())
    }

    /// 日志当前字节数（用于压实判据）。
    fn journal_len(data_dir: &Path) -> u64 {
        std::fs::metadata(data_dir.join(SEMANTIC_JOURNAL_FILE))
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// 序列化为二进制。
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + self.ids.len() * (2 + 32 + self.dim * 4));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&(self.dim as u32).to_le_bytes());
        out.extend_from_slice(&(self.ids.len() as u32).to_le_bytes());
        for (id, vector) in self.ids.iter().zip(self.vectors.iter()) {
            let id_bytes = id.as_bytes();
            let id_len = id_bytes.len().min(u16::MAX as usize);
            out.extend_from_slice(&(id_len as u16).to_le_bytes());
            out.extend_from_slice(&id_bytes[..id_len]);
            for x in vector {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        out
    }

    /// 反序列化（任何不一致都返回 None，由调用方按空索引处理）。
    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 20 || &bytes[..8] != MAGIC {
            return None;
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
        if version != FORMAT_VERSION {
            return None;
        }
        let dim = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
        let count = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
        if dim == 0 || dim > 8192 {
            return None;
        }
        let mut index = SemanticIndex {
            dim,
            ids: Vec::with_capacity(count),
            vectors: Vec::with_capacity(count),
            pos: HashMap::with_capacity(count),
        };
        let mut cursor = 20usize;
        for _ in 0..count {
            if cursor + 2 > bytes.len() {
                return None;
            }
            let id_len = u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().ok()?) as usize;
            cursor += 2;
            if cursor + id_len + dim * 4 > bytes.len() {
                return None;
            }
            let id = String::from_utf8(bytes[cursor..cursor + id_len].to_vec()).ok()?;
            cursor += id_len;
            let mut vector = Vec::with_capacity(dim);
            for _ in 0..dim {
                vector.push(f32::from_le_bytes(
                    bytes[cursor..cursor + 4].try_into().ok()?,
                ));
                cursor += 4;
            }
            index
                .pos
                .entry(id.clone())
                .or_default()
                .push(index.vectors.len());
            index.ids.push(id);
            index.vectors.push(vector);
        }
        Some(index)
    }
}

/// L2 归一化（零向量原样返回，检索时自然得 0 分）。
fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm.is_finite() && norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    v
}

/// 点积（两侧均已归一化 ⇒ 余弦相似度）。
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// 后台任务。
enum Job {
    /// 写入时编码（新记忆）
    Encode(Vec<(String, String)>),
    /// 写入时**已带向量**（一次前向双产出的产物），直接插入，不再编码
    Insert(Vec<(String, Vec<f32>)>),
    /// 存量回填（启动时一次性投递）
    Backfill(Vec<(String, String)>),
}

/// 语义索引服务（全局单例）。
pub struct SemanticService {
    index: RwLock<SemanticIndex>,
    encoder: Arc<crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder>,
    data_dir: PathBuf,
    tx: Sender<Job>,
    /// 库内记忆总数（覆盖率分母）
    total: AtomicUsize,
    /// 编码失败累计条数（**必须从 pending 中扣除**，否则失败项会让
    /// `wait_for_index` 永远等不到 0，屏障退化成固定超时）
    failed: AtomicUsize,
    /// 是否有未落盘的增量
    dirty: AtomicBool,
    /// 自上次落盘以来**新增**的向量（flush 时只追加这些，不整份重写）
    pending: std::sync::Mutex<Vec<(String, Vec<f32>)>>,
    /// 是否具备 ML 语义能力
    enabled: bool,
}

impl SemanticService {
    /// 已索引条数。
    pub fn indexed(&self) -> usize {
        self.index.read().map(|i| i.len()).unwrap_or(0)
    }

    /// 覆盖率（已索引 / 库内总数），0.0~1.0。
    pub fn coverage(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        (self.indexed() as f32 / total as f32).clamp(0.0, 1.0)
    }

    /// 是否可用（ML 编码器就绪且索引非空）。
    pub fn is_ready(&self) -> bool {
        self.enabled && self.indexed() > 0
    }

    /// 待索引条数（已入队但尚未落索引，且未失败）。
    ///
    /// 口径：`total - indexed - failed`，下限 0。失败项必须扣除，
    /// 否则一条编码失败就会让 `wait_for_index` 永远等不到 0。
    pub fn pending_count(&self) -> usize {
        let total = self.total.load(Ordering::Relaxed);
        let failed = self.failed.load(Ordering::Relaxed);
        total.saturating_sub(self.indexed()).saturating_sub(failed)
    }

    /// 编码失败累计条数。
    pub fn failed_count(&self) -> usize {
        self.failed.load(Ordering::Relaxed)
    }

    /// 同步写入路径记账：新记忆计入库内总数，失败项计入 `failed`。
    ///
    /// **为什么必须在这里记 `total`**：同步路径（`insert_precomputed_sync` /
    /// `encode_batch_sync`）写入的是**此前未入队**的新记忆，`enqueue` 的
    /// `total.fetch_add` 不会执行。漏记会让覆盖率分母偏小、`pending_count`
    /// 提前归零（屏障形同虚设）。
    fn record_sync(&self, inserted: usize, failed: usize) {
        self.total.fetch_add(inserted + failed, Ordering::Relaxed);
        self.failed.fetch_add(failed, Ordering::Relaxed);
    }

    /// 异步/回填路径记账：条目已由 `enqueue` 计入 `total`，这里**只记失败**。
    fn record_failed(&self, failed: usize) {
        self.failed.fetch_add(failed, Ordering::Relaxed);
    }

    /// 阻塞等待索引追平（覆盖率 ≥ 1.0），最多等 `timeout`。
    ///
    /// 返回 `true` 表示已追平；`false` 表示超时或本通路不可用。
    ///
    /// **为什么需要它**：写入与索引是异步的，调用方若「写完立刻按语义检索」，
    /// 未索引的记忆不会参与打分。需要读己之写的调用方（基准测试、迁移脚本、
    /// 用户手动触发的一致性检查）必须先过这道屏障。
    pub fn wait_for_index(&self, timeout: std::time::Duration) -> bool {
        if !self.enabled {
            return false;
        }
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.pending_count() == 0 {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// 取某条记忆的语义向量（未索引返回 None）。
    ///
    /// 向量在入库时已归一化，因此两向量点积即余弦相似度。
    /// 供「召回时近重复折叠」使用：只读、不触发编码，成本可忽略。
    pub fn vector_of(&self, id: &str) -> Option<Vec<f32>> {
        self.index.read().ok()?.get(id).map(|v| v.to_vec())
    }

    /// 全库语义检索。
    ///
    /// 返回 `None` 表示**本通路不可用**（编码器不可用 / 索引为空 / 查询编码失败），
    /// 调用方应回退既有通路，而不是把空结果当作"没有相关记忆"。
    ///
    /// `candidates` 是已通过基础过滤（隐私/项目/类型等）的记忆；本方法只对
    /// **已索引**的候选打分，未索引者不参与（不填 0 分，避免静默沉底）。
    pub fn search(
        &self,
        query: &str,
        candidates: &[&Memory],
        top_k: usize,
    ) -> Option<Vec<(String, f32)>> {
        if !self.is_ready() || top_k == 0 {
            return None;
        }
        let instructed = format!("{BGE_QUERY_INSTRUCTION}{query}");
        let query_vector = self.encoder.encode_embedding(&instructed)?;
        let index = self.index.read().ok()?;
        if index.dim() == 0 || query_vector.len() != index.dim() {
            return None;
        }
        let q = normalize(query_vector);
        let mut scored: Vec<(String, f32)> = candidates
            .iter()
            .filter_map(|m| {
                let v = index.get(&m.id)?;
                Some((m.id.clone(), dot(&q, v)))
            })
            .collect();
        if scored.is_empty() {
            return None;
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        Some(scored)
    }

    /// 新记忆入队（非阻塞；服务未初始化时静默忽略）。
    pub fn enqueue(&self, id: &str, content: &str) {
        if !self.enabled || content.trim().is_empty() {
            return;
        }
        // 已索引则跳过（合并写入时内容可能变化，但向量差异通常很小；
        // 内容真变由 `remember` 的合并路径决定，这里以"未索引"为准，
        // 避免每次合并都重复编码）。
        if self
            .index
            .read()
            .map(|i| i.get(id).is_some())
            .unwrap_or(false)
        {
            return;
        }
        self.total.fetch_add(1, Ordering::Relaxed);
        let _ = self
            .tx
            .send(Job::Encode(vec![(id.to_string(), content.to_string())]));
    }

    /// 新记忆入队（**已带高维句向量**，跳过后台编码）。
    ///
    /// 供写入路径「一次前向双产出」使用：洛书编码时已顺带算出高维句向量，
    /// 直接插入索引即可，避免后台对同一段文本再跑一遍前向。
    ///
    /// **调用方契约**：传入的向量必须与 [`Self::enqueue`] 后台编码的结果同源
    /// （同一 `forward_pooled_batch` + 同一 `EncodeRole::Passage`），且文本长度
    /// 不超过 [`CHUNK_CHARS`]（单窗口）。长文本仍走 [`Self::enqueue`] 的分窗路径。
    pub fn enqueue_with_vector(&self, id: &str, vector: Vec<f32>) {
        if !self.enabled || vector.is_empty() {
            return;
        }
        if self
            .index
            .read()
            .map(|i| i.get(id).is_some())
            .unwrap_or(false)
        {
            return;
        }
        self.total.fetch_add(1, Ordering::Relaxed);
        let _ = self
            .tx
            .send(Job::Insert(vec![(id.to_string(), normalize(vector))]));
    }

    /// 立即落盘（供测试/关闭时调用）。
    ///
    /// ★v3：只追加**新增向量**到日志（O(新增条数)）；仅当日志达到阈值
    /// 或基文件缺失时才整份压实。不再每次 flush 都重写 57MB。
    pub fn flush(&self) {
        let pending = {
            let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *p)
        };
        if pending.is_empty() {
            self.dirty.store(false, Ordering::Release);
            return;
        }
        let base_missing = !self.data_dir.join(SEMANTIC_INDEX_FILE).exists();
        let need_compact =
            base_missing || SemanticIndex::journal_len(&self.data_dir) >= journal_max_bytes();
        let result = if need_compact {
            // 整份压实：写基文件（内部会清空日志）
            match self.index.read() {
                Ok(index) => index.save(&self.data_dir),
                Err(_) => Ok(()),
            }
        } else {
            SemanticIndex::append_journal(&self.data_dir, &pending)
        };
        match result {
            Ok(()) => {
                self.dirty.store(false, Ordering::Release);
            }
            Err(e) => {
                eprintln!(
                    "[LRC·语义索引] ⚠ 落盘失败（已恢复待写向量，下轮重试）: {}",
                    e
                );
                // 放回 pending，避免这批向量永不落盘
                let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
                p.splice(0..0, pending);
                self.dirty.store(true, Ordering::Release);
            }
        }
    }
}

/// 全局单例。
static SERVICE: OnceLock<Arc<SemanticService>> = OnceLock::new();

/// 取全局服务（未初始化返回 None）。
pub fn service() -> Option<&'static Arc<SemanticService>> {
    SERVICE.get()
}

/// 新记忆入队编码（供写入路径调用；未初始化时静默忽略）。
pub fn enqueue(id: &str, content: &str) {
    if let Some(service) = SERVICE.get() {
        service.enqueue(id, content);
    }
}

/// 索引状态快照（供 `/v1/index/status` 与 MCP `lrc_status` 使用）。
///
/// 未初始化时返回 `enabled=false` 的全零快照，而不是 `None` ——
/// 调用方需要区分「没开语义索引」与「服务没起来」，前者是正常配置。
pub fn status() -> serde_json::Value {
    match SERVICE.get() {
        Some(s) => serde_json::json!({
            "enabled": s.enabled,
            "indexed": s.indexed(),
            "total": s.total.load(Ordering::Relaxed),
            "pending": s.pending_count(),
            "coverage": s.coverage(),
            "ready": s.is_ready(),
        }),
        None => serde_json::json!({
            "enabled": false,
            "indexed": 0,
            "total": 0,
            "pending": 0,
            "coverage": 0.0,
            "ready": false,
        }),
    }
}

/// 阻塞等待索引追平（供 `/v1/index/wait` 与基准测试使用）。
///
/// 返回 `(ready, coverage, pending)`；未初始化时 `ready=false`。
pub fn wait_for_index(timeout: std::time::Duration) -> (bool, f32, usize) {
    match SERVICE.get() {
        Some(s) => {
            let ready = s.wait_for_index(timeout);
            (ready, s.coverage(), s.pending_count())
        }
        None => (false, 0.0, 0),
    }
}

/// 同步插入一批**已编码**的向量（写入路径「一次前向双产出」的产物）。
///
/// 与 [`encode_batch_sync`] 对称：同样受时间预算约束，超预算的部分交回异步队列。
/// 供 `remember_batch` 在已拿到句向量时跳过第二次前向。
pub fn insert_precomputed_sync(items: &[(String, Vec<f32>)]) {
    let Some(service) = SERVICE.get() else {
        return;
    };
    if !service.enabled || items.is_empty() {
        return;
    }
    const SYNC_BUDGET: Duration = Duration::from_secs(15);
    let started = std::time::Instant::now();
    let mut done = 0usize;
    for chunk in items.chunks(ENCODE_BATCH) {
        if started.elapsed() >= SYNC_BUDGET {
            break;
        }
        let (inserted, failed) = insert_precomputed(service, chunk);
        service.record_sync(inserted, failed);
        done += chunk.len();
    }
    if done > 0 {
        service.flush();
    }
    for (id, vector) in &items[done..] {
        service.enqueue_with_vector(id, vector.clone());
    }
}

/// **同步**编码一批记忆并立即落盘（供批量写入路径调用）。
///
/// # 为什么批量写入要同步
///
/// 评测/导入的典型流程是「批量注入 → 短暂等待 → 立即查询」。若批量写入只做
/// 异步入队，编码可能**尚未完成**（实测 143 条注入后 30s 内未编完），
/// 语义路在查询时几乎为空 ⇒ 新写入的记忆（往往正是答案）无法被语义召回。
///
/// 故对**中小批量**（≤ [`SYNC_ENCODE_MAX`]）在写入路径内同步编码：
/// 批量写入本就是长耗时操作（调用方超时 300s），多花数十秒换取
/// "写入返回即索引可用"的一致性。超大批量仍走异步，避免长时间占用写路径。
pub fn encode_batch_sync(items: &[(String, String)]) {
    let Some(service) = SERVICE.get() else {
        return;
    };
    if !service.enabled || items.is_empty() {
        return;
    }
    // ★ 时间预算：同步编码**不得**无限占用写路径。
    //
    // 实测教训：143 条长记忆同步编码耗时 >10 分钟，直接撞满 MCP 调用超时
    // （300s），调用方拿不到写入结果。故只同步编码预算内的部分，
    // 其余交回异步队列（后台线程继续编码，不阻塞写入返回）。
    const SYNC_BUDGET: Duration = Duration::from_secs(15);
    let started = std::time::Instant::now();
    let mut done = 0usize;
    for chunk in items.chunks(ENCODE_BATCH) {
        if started.elapsed() >= SYNC_BUDGET {
            break;
        }
        let (inserted, failed) = encode_and_insert(service, chunk);
        service.record_sync(inserted, failed);
        done += chunk.len();
    }
    if done > 0 {
        service.flush();
    }
    for (id, content) in &items[done..] {
        service.enqueue(id, content);
    }
}

/// 同步编码的批量上限（超过则退回异步，避免写路径被超大批量长时间占用）。
pub const SYNC_ENCODE_MAX: usize = 256;

/// 初始化全局服务（幂等：重复调用返回既有实例）。
///
/// `snapshot` 是启动时的全量记忆 `(id, content)`；其中**尚未索引**的部分
/// 会作为回填任务投递给后台线程。回填在独立线程执行，不持有存储锁。
pub fn init(
    encoder: Arc<crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder>,
    data_dir: PathBuf,
    snapshot: Vec<(String, String)>,
) -> Arc<SemanticService> {
    if let Some(existing) = SERVICE.get() {
        return existing.clone();
    }
    let enabled = encoder.is_ml_mode();
    let mut index = SemanticIndex::load(&data_dir);
    // 旧 JSON 侧车迁移：二进制不存在时尝试导入一次（导入后由二进制接管）。
    // `state_matcher` 按 server feature 门控，故迁移逻辑同样门控。
    #[cfg(feature = "server")]
    if index.is_empty() {
        let legacy = crate::state_matcher::load_semantic_cache(&data_dir);
        if !legacy.vectors.is_empty() {
            for (id, vector) in legacy.vectors {
                index.insert(id, vector);
            }
            eprintln!("[LRC·语义索引] 已从旧 JSON 侧车导入 {} 条向量", index.len());
        }
    }
    let total = snapshot.len().max(index.len());
    let indexed_before = index.len();
    let (tx, rx) = std::sync::mpsc::channel::<Job>();
    let service = Arc::new(SemanticService {
        index: RwLock::new(index),
        encoder: encoder.clone(),
        data_dir: data_dir.clone(),
        tx,
        total: AtomicUsize::new(total),
        failed: AtomicUsize::new(0),
        dirty: AtomicBool::new(false),
        pending: std::sync::Mutex::new(Vec::new()),
        enabled,
    });
    if SERVICE.set(service.clone()).is_err() {
        // 并发初始化：以先设置者为准
        return SERVICE.get().cloned().unwrap_or(service);
    }
    if !enabled {
        eprintln!("[LRC·语义索引] ML 编码器不可用，语义通路保持关闭（诚实降级）");
        return service;
    }
    // 待回填 = 快照中尚未索引的记忆
    let pending: Vec<(String, String)> = {
        let index = service.index.read_recover();
        snapshot
            .into_iter()
            .filter(|(id, _)| index.get(id).is_none())
            .collect()
    };
    eprintln!(
        "[LRC·语义索引] 启动：已索引 {} 条，待回填 {} 条（维度 {}）",
        indexed_before,
        pending.len(),
        service.index.read().map(|i| i.dim()).unwrap_or(0)
    );
    if !pending.is_empty() {
        let _ = service.tx.send(Job::Backfill(pending));
    }
    spawn_worker(service.clone(), rx);
    service
}

/// 后台工作线程：批量编码 → 插入索引 → 定期落盘。
fn spawn_worker(service: Arc<SemanticService>, rx: Receiver<Job>) {
    std::thread::Builder::new()
        .name("lrc-semantic-index".to_string())
        .spawn(move || {
            let mut since_flush = 0usize;
            loop {
                match rx.recv_timeout(IDLE_TICK) {
                    Ok(Job::Encode(mut items)) => {
                        // 批量注入时会有大量单条 Encode 任务；这里**非阻塞地合并**
                        // 到一批再编码，避免"每条一次前向"的固定开销。
                        while items.len() < ENCODE_BATCH {
                            match rx.try_recv() {
                                Ok(Job::Encode(more)) => items.extend(more),
                                Ok(Job::Insert(more)) => {
                                    // 已编码向量：直接插入，不参与本批编码
                                    let (inserted, failed) = insert_precomputed(&service, &more);
                                    service.record_failed(failed);
                                    since_flush += inserted;
                                }
                                Ok(Job::Backfill(backfill)) => {
                                    // 回填任务不插队：放回队列，稍后处理
                                    let _ = service.tx.send(Job::Backfill(backfill));
                                    break;
                                }
                                Err(_) => break,
                            }
                        }
                        let (inserted, failed) = encode_and_insert(&service, &items);
                        service.record_failed(failed);
                        since_flush += inserted;
                    }
                    Ok(Job::Insert(items)) => {
                        let (inserted, failed) = insert_precomputed(&service, &items);
                        service.record_failed(failed);
                        since_flush += inserted;
                    }
                    Ok(Job::Backfill(items)) => {
                        let total = items.len();
                        let started = std::time::Instant::now();
                        let mut done = 0usize;
                        for chunk in items.chunks(ENCODE_BATCH) {
                            let (_, failed) = encode_and_insert(&service, chunk);
                            service.record_failed(failed);
                            done += chunk.len();
                            since_flush += chunk.len();
                            if since_flush >= FLUSH_EVERY {
                                service.flush();
                                since_flush = 0;
                            }
                            if done % (ENCODE_BATCH * 8) == 0 || done == total {
                                eprintln!(
                                    "[LRC·语义索引] 回填进度 {}/{}（{:.1} 条/s，覆盖率 {:.1}%）",
                                    done,
                                    total,
                                    done as f64 / started.elapsed().as_secs_f64().max(1e-3),
                                    service.coverage() * 100.0
                                );
                            }
                        }
                        service.flush();
                        since_flush = 0;
                        eprintln!(
                            "[LRC·语义索引] 回填完成：{} 条，耗时 {:.1}s，覆盖率 {:.1}%",
                            done,
                            started.elapsed().as_secs_f64(),
                            service.coverage() * 100.0
                        );
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if since_flush > 0 {
                            service.flush();
                            since_flush = 0;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                if since_flush >= FLUSH_EVERY {
                    service.flush();
                    since_flush = 0;
                }
            }
            service.flush();
        })
        .expect("创建语义索引工作线程失败");
}

/// 把一条记忆切成 1~3 个窗口（首 / 中 / 尾）。
///
/// 短文本（≤ [`CHUNK_CHARS`] 字）整条编码；长文本取首、中、尾三段，
/// 去重后返回。这样既覆盖长文本两端与中段，又把单条成本压到 3 个短窗口。
fn chunk_text(content: &str) -> Vec<String> {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= CHUNK_CHARS {
        return vec![content.to_string()];
    }
    let mut windows: Vec<String> = Vec::with_capacity(3);
    let head: String = chars[..CHUNK_CHARS].iter().collect();
    windows.push(head);
    let mid_start = (chars.len() - CHUNK_CHARS) / 2;
    let mid: String = chars[mid_start..mid_start + CHUNK_CHARS].iter().collect();
    if !windows.contains(&mid) {
        windows.push(mid);
    }
    let tail: String = chars[chars.len() - CHUNK_CHARS..].iter().collect();
    if !windows.contains(&tail) {
        windows.push(tail);
    }
    windows
}

/// 并行批量编码窗口。
///
/// # 为什么需要并行
///
/// 实测（本机 bge-base-zh / CPU）：单线程批量编码约 **1.0 条记忆/s**
/// （每条 1~3 个窗口），全库 7k 需约 2 小时。而 candle 的单次前向只用到
/// 约 1.3 个核 ⇒ 外部按窗口分片并行可把墙钟时间压到 1/3~1/4。
///
/// 编码器本身 `Sync`（既有 `semantic_similarities_impl` 已用同一范式跨线程
/// 共享 `&encoder`），故此处直接 `thread::scope` 分片，输出顺序与输入严格一致。
///
/// # 并行度上限（v0.10.4：由硬编码 4 改为可配，默认仍为 4）
///
/// 原为 `.clamp(1, 4)`。之所以不能简单放到"等于核数"：
/// candle 的 `forward` 内部还会用 Rayon 再并行（调用方常设
/// `RAYON_NUM_THREADS=2`），于是 `外部片数 × 内部线程数` 很容易超过物理核数
/// ⇒ 线程超订，实测反而更慢。
///
/// 因此改为环境变量 `LRC_ENCODE_PARALLELISM` 可覆盖，**默认仍为 4**（零行为变更），
/// 便于在不改代码的前提下按机器实测调优。
fn encode_parallelism() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    std::env::var("LRC_ENCODE_PARALLELISM")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        // 上限取核数：再多也只是纯超订
        .map(|v| v.min(cores))
        .unwrap_or_else(|| cores.clamp(1, 4))
}

fn encode_windows_parallel(
    encoder: &Arc<crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder>,
    texts: &[String],
) -> Option<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Some(Vec::new());
    }
    let parallelism = encode_parallelism();
    if parallelism == 1 || texts.len() < 8 {
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        return encoder.encode_embedding_batch(&refs);
    }
    // ★ 按长度排序后再分片：批内 padding 到最长样本，长短混排会让短文本
    //   被迫按最长样本计算（实测长记忆与短记忆混批时吞吐掉一个数量级）。
    //   排序后同批长度接近，padding 浪费最小；编码完再按原下标还原顺序。
    let mut order: Vec<usize> = (0..texts.len()).collect();
    order.sort_by_key(|&i| texts[i].chars().count());
    let sorted: Vec<&str> = order.iter().map(|&i| texts[i].as_str()).collect();
    let chunk_size = sorted.len().div_ceil(parallelism);
    let results: Vec<Option<Vec<Vec<f32>>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = sorted
            .chunks(chunk_size)
            .map(|chunk| {
                let refs: Vec<&str> = chunk.to_vec();
                scope.spawn(move || encoder.encode_embedding_batch(&refs))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().ok().flatten())
            .collect()
    });
    let mut sorted_out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for result in results {
        sorted_out.extend(result?);
    }
    if sorted_out.len() != texts.len() {
        return None;
    }
    // 还原原始顺序
    let mut out: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
    for (sorted_idx, &original_idx) in order.iter().enumerate() {
        out[original_idx] = Some(std::mem::take(&mut sorted_out[sorted_idx]));
    }
    out.into_iter().collect()
}

/// 编码一批并插入索引，返回成功插入的**记忆条数**（不是窗口数）。
fn encode_and_insert(service: &SemanticService, items: &[(String, String)]) -> (usize, usize) {
    if items.is_empty() {
        return (0, 0);
    }
    // 1. 展开为窗口序列，并记录每个窗口属于哪条记忆
    let mut texts: Vec<String> = Vec::new();
    let mut owners: Vec<usize> = Vec::new();
    for (i, (_, content)) in items.iter().enumerate() {
        for chunk in chunk_text(content) {
            texts.push(chunk);
            owners.push(i);
        }
    }
    if texts.is_empty() {
        return (0, items.len());
    }
    // 2. 并行批量编码（按窗口分片，输出顺序与输入一致）
    let Some(vectors) = encode_windows_parallel(&service.encoder, &texts) else {
        return (0, items.len());
    };
    if vectors.len() != texts.len() {
        return (0, items.len());
    }
    let first_dim = vectors.first().map(|v| v.len()).unwrap_or(0);
    if first_dim == 0 {
        return (0, items.len());
    }
    let Ok(mut index) = service.index.write() else {
        return (0, items.len());
    };
    // 维度变更（换模型）⇒ 旧索引整体失效，先清空再重建
    if index.dim() != 0 && index.dim() != first_dim {
        eprintln!(
            "[LRC·语义索引] ⚠ 向量维度变化（{} → {}），清空旧索引重建",
            index.dim(),
            first_dim
        );
        index.clear();
        // ★v3：维度变更时基文件与日志都失效 ⇒ 立即写空基文件并清空日志，
        //   否则新维度的日志会与旧维度的基文件混在一起（重放时被整份跳过）。
        let _ = index.save(&service.data_dir);
    }
    let mut inserted_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    // 已索引的 id 直接跳过：写入时编码与存量回填可能对同一条记忆**并发**编码，
    // 先完成者胜出，后完成者不得重复追加窗口（否则索引膨胀、内存翻倍）。
    let existing: std::collections::HashSet<String> = items
        .iter()
        .map(|(id, _)| id.clone())
        .filter(|id| index.get(id).is_some())
        .collect();
    // ★v3：记录本次新增的向量，供 flush 追加到增量日志（不整份重写）。
    let mut new_vectors: Vec<(String, Vec<f32>)> = Vec::new();
    for (j, vector) in vectors.into_iter().enumerate() {
        let (id, _) = &items[owners[j]];
        if existing.contains(id) {
            continue;
        }
        let normalized = normalize(vector);
        index.insert(id.clone(), normalized.clone());
        new_vectors.push((id.clone(), normalized));
        inserted_ids.insert(id.clone());
    }
    drop(index);
    if !new_vectors.is_empty() {
        let mut p = service.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.extend(new_vectors);
        service.dirty.store(true, Ordering::Release);
    }
    let inserted = inserted_ids.len();
    let failed = items
        .len()
        .saturating_sub(inserted)
        .saturating_sub(existing.len());
    (inserted, failed)
}

/// 插入一批**已编码**的向量（写入路径「一次前向双产出」的产物）。
///
/// 与 [`encode_and_insert`] 的差别：不展开窗口、不调用编码器，直接落索引。
/// 维度变更清空、已索引跳过、增量日志记录三条契约与编码路径逐位一致。
fn insert_precomputed(service: &SemanticService, items: &[(String, Vec<f32>)]) -> (usize, usize) {
    if items.is_empty() {
        return (0, 0);
    }
    let first_dim = items.first().map(|(_, v)| v.len()).unwrap_or(0);
    if first_dim == 0 {
        return (0, items.len());
    }
    let Ok(mut index) = service.index.write() else {
        return (0, items.len());
    };
    // 维度变更（换模型）⇒ 旧索引整体失效，先清空再重建（与编码路径同口径）
    if index.dim() != 0 && index.dim() != first_dim {
        eprintln!(
            "[LRC·语义索引] ⚠ 向量维度变化（{} → {}），清空旧索引重建",
            index.dim(),
            first_dim
        );
        index.clear();
        let _ = index.save(&service.data_dir);
    }
    let mut new_vectors: Vec<(String, Vec<f32>)> = Vec::new();
    let mut inserted = 0usize;
    let mut failed = 0usize;
    for (id, vector) in items {
        // 维度不一致：该条**无法**进入本向量空间，计为失败（不是"已索引跳过"），
        // 否则 `pending_count` 会把它当作已完成，屏障提前放行。
        if vector.len() != first_dim {
            failed += 1;
            continue;
        }
        // 已索引：并发编码/回填先完成者胜出，跳过且不计失败。
        if index.get(id).is_some() {
            continue;
        }
        // ═══ v0.10.4 P0 修复：本路径必须与其它入库路径一样做 L2 归一化 ═══
        //
        // `encode_both_batch` 产出的是**原始 pooled 向量**（BERT CLS 池化，范数
        // 常在 10~30），不是单位向量。而检索侧 `search()` 用的是裸点积
        // `dot(&q, v)`、`vector_of` 的文档也写明"向量在入库时已归一化"。
        //
        // 漏归一化的后果（两条，都是静默的）：
        //   ① 语义分被 `|v|` 放大 ⇒ 同一查询下，走本路径写入的记忆（短文本）
        //      分数远高于走 `encode_and_insert` 的长文本，排序失真；
        //   ② 更严重：`dedup_keep_indices_with`（默认阈值 0.90）用裸点积判近重复，
        //      `dot(u_un, v_un) = |u||v|cos ≈ 100~900 × cos` ⇒ 任意两条都会被判
        //      为"重复"而折叠掉，召回集被压成 1 条。
        //
        // 对照：`encode_and_insert`（1108 行）与 `enqueue_with_vector`（616 行）
        // 都已 normalize，唯独本函数遗漏 —— 这是 P3「一次前向双产出」接入时
        // 新增的第三条入库路径，未同步该契约。
        let vector = normalize(vector.clone());
        index.insert(id.clone(), vector.clone());
        new_vectors.push((id.clone(), vector));
        inserted += 1;
    }
    drop(index);
    if !new_vectors.is_empty() {
        let mut p = service.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.extend(new_vectors);
        service.dirty.store(true, Ordering::Release);
    }
    (inserted, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_of(values: &[f32]) -> Vec<f32> {
        values.to_vec()
    }

    /// ★v0.10.4 回归：`LRC_ENCODE_PARALLELISM` 可覆盖并行度，且必须被核数钳住。
    ///
    /// # 锁住的是什么
    ///
    /// 修前并行度是硬编码 `.clamp(1, 4)`，6 核机器只能用 4 线程，无法按机器调优。
    /// 现改为环境变量可覆盖，但**不允许超过物理核数**（超订只会更慢）。
    ///
    /// 注意：环境变量是进程级全局，本测试与其它并发测试共享进程，
    /// 故对 `LRC_ENCODE_PARALLELISM` 的写入必须在本测试结束后还原。
    /// 为免与其它用例互扰，这里只断言**函数自身的不变量**（返回值 ≥1 且 ≤ 核数），
    /// 不去断言具体等于某个值。
    #[test]
    fn 并行度受环境变量控制且不超过核数() {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        // 未设置：默认路径，必须落在 [1, min(4, cores)]
        let default_p = encode_parallelism();
        assert!(default_p >= 1, "并行度至少为 1");
        assert!(
            default_p <= cores.min(4),
            "默认并行度不得超过 min(4, 核数)，实际 {default_p}，核数 {cores}"
        );

        // 显式设置为一个超大值：必须被核数钳住（不允许超订）
        // 用 RAII 还原，避免污染同进程的其它用例
        struct EnvGuard(Option<std::ffi::OsString>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("LRC_ENCODE_PARALLELISM", v),
                    None => std::env::remove_var("LRC_ENCODE_PARALLELISM"),
                }
            }
        }
        let _guard = EnvGuard(std::env::var_os("LRC_ENCODE_PARALLELISM"));

        std::env::set_var("LRC_ENCODE_PARALLELISM", "9999");
        let clamped = encode_parallelism();
        assert_eq!(
            clamped, cores,
            "★超大并行度必须被钳到物理核数（{cores}），否则线程超订反而更慢"
        );

        // 非法值（0 / 非数字）应回退到默认路径，不得 panic
        std::env::set_var("LRC_ENCODE_PARALLELISM", "0");
        let zero = encode_parallelism();
        assert!(zero >= 1, "非法值 0 必须回退到默认并行度，而不是 0 线程");
        std::env::set_var("LRC_ENCODE_PARALLELISM", "not-a-number");
        let bad = encode_parallelism();
        assert!(bad >= 1, "非数字必须回退到默认并行度");
    }

    /// 构造一个**不启动后台线程**的服务实例，用于验证记账逻辑。
    ///
    /// 编码器用 `new_statistical()`（无需模型），`insert_precomputed` 不调用编码器，
    /// 故本实例只用于测试 `total` / `failed` / `pending_count` 的算术不变量。
    fn test_service(dir: &Path) -> Arc<SemanticService> {
        let (tx, _rx) = std::sync::mpsc::channel();
        Arc::new(SemanticService {
            index: RwLock::new(SemanticIndex::default()),
            encoder: Arc::new(
                crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder::new_statistical(),
            ),
            data_dir: dir.to_path_buf(),
            tx,
            total: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            dirty: AtomicBool::new(false),
            pending: std::sync::Mutex::new(Vec::new()),
            enabled: true,
        })
    }

    /// ★v0.10.4 P0 回归：`insert_precomputed` 必须对向量做 L2 归一化。
    ///
    /// # 锁住的是什么
    ///
    /// `encode_both_batch` 产出的是**原始 pooled 向量**（BERT CLS 池化，范数
    /// 常在 10~30），不是单位向量。全库检索与近重复折叠都用**裸点积**，
    /// 契约是"入库即归一化"。本函数是第三条入库路径（P3「一次前向双产出」
    /// 引入），修前唯独它漏了 normalize。
    ///
    /// 后果 ①：语义分被 `|v|` 放大，同查询下短文本记忆分数虚高，排序失真。
    /// 后果 ②（更严重）：`dedup_keep_indices_with`（阈值 0.90）用裸点积判近重复，
    ///   `dot(u,v) = |u||v|cos ≈ 100~900 × cos` ⇒ 任意两条都被判"重复"而折叠，
    ///   召回集被压成 1 条。
    ///
    /// **鉴别力**：删掉 `insert_precomputed` 里的 `normalize(vector)`
    /// ⇒ 本测试断言 `norm ≈ 1.0` 失败。
    #[test]
    fn 预编码向量入库时必须归一化() {
        let dir = tempfile::TempDir::new().expect("临时目录");
        let svc = test_service(dir.path());

        // 构造一个"类 BERT 原始输出"的向量：方向 (3,4)，范数 5
        let raw = vec![3.0f32, 4.0];
        let (inserted, failed) = insert_precomputed(&svc, &[("m1".into(), raw)]);
        assert_eq!(inserted, 1);
        assert_eq!(failed, 0);

        let v = {
            let idx = svc.index.read().expect("读索引");
            idx.get("m1").map(|s| s.to_vec()).expect("应已入库")
        };
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "★入库向量必须是单位向量（L2 范数 1.0），实际 {norm}。\
             未归一化会让裸点积语义分虚高、并让 0.90 的近重复折叠误杀全部结果"
        );
        // 方向不变：(3,4)/5 = (0.6, 0.8)
        assert!(
            (v[0] - 0.6).abs() < 1e-5 && (v[1] - 0.8).abs() < 1e-5,
            "归一化不得改变方向，实际 {v:?}"
        );
    }

    /// 回归：维度不一致的预编码向量必须计为**失败**，不能当作「已索引跳过」。
    ///
    /// 若误判为跳过，`pending_count` 会把它当作已完成 ⇒ 屏障提前放行。
    #[test]
    fn 维度不一致的预编码向量计为失败而非跳过() {
        let dir = tempfile::TempDir::new().expect("临时目录");
        let svc = test_service(dir.path());
        let (inserted, failed) = insert_precomputed(
            &svc,
            &[
                ("ok".into(), vec![1.0, 0.0]),
                ("bad".into(), vec![1.0, 0.0, 0.0]),
            ],
        );
        assert_eq!(inserted, 1);
        assert_eq!(failed, 1, "维度不一致必须计为失败");
    }

    /// 回归：同步写入路径必须把新记忆计入 `total`，且失败项从 `pending` 扣除。
    ///
    /// 漏记 `total` ⇒ 覆盖率分母偏小；不扣 `failed` ⇒ `wait_for_index` 永不归零。
    #[test]
    fn 同步记账后失败项从pending扣除() {
        let dir = tempfile::TempDir::new().expect("临时目录");
        let svc = test_service(dir.path());
        let (inserted, failed) = insert_precomputed(
            &svc,
            &[
                ("ok".into(), vec![1.0, 0.0]),
                ("bad".into(), vec![1.0, 0.0, 0.0]),
            ],
        );
        svc.record_sync(inserted, failed);
        assert_eq!(
            svc.total.load(Ordering::Relaxed),
            2,
            "同步写入必须计入库内总数"
        );
        assert_eq!(svc.pending_count(), 0, "失败项必须从 pending 扣除");
        assert!(
            svc.wait_for_index(Duration::from_millis(10)),
            "pending=0 时屏障应立即放行"
        );
    }

    /// 回归：异步/回填路径只记失败，**不得**重复累加 `total`（已由 enqueue 记过）。
    #[test]
    fn 异步记账只记失败不改总数() {
        let dir = tempfile::TempDir::new().expect("临时目录");
        let svc = test_service(dir.path());
        // 模拟 enqueue 已为 3 条记忆记过 total
        svc.total.fetch_add(3, Ordering::Relaxed);
        let (inserted, failed) = insert_precomputed(
            &svc,
            &[
                ("a".into(), vec![1.0, 0.0]),
                ("b".into(), vec![0.0, 1.0]),
                ("bad".into(), vec![1.0, 0.0, 0.0]),
            ],
        );
        svc.record_failed(failed);
        assert_eq!(inserted, 2);
        assert_eq!(failed, 1);
        assert_eq!(
            svc.total.load(Ordering::Relaxed),
            3,
            "异步路径不得重复累加 total"
        );
        assert_eq!(svc.pending_count(), 0);
    }

    #[test]
    fn 插入后按余弦降序检索() {
        let mut index = SemanticIndex::default();
        index.insert("a".into(), vec_of(&[1.0, 0.0]));
        index.insert("b".into(), vec_of(&[0.0, 1.0]));
        index.insert("c".into(), vec_of(&[0.7, 0.7]));
        let hits = index.search(&[1.0, 0.0], 3);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].0, "a");
        assert!(hits[0].1 > hits[1].1);
    }

    #[test]
    fn 维度不一致的向量被拒绝() {
        let mut index = SemanticIndex::default();
        index.insert("a".into(), vec_of(&[1.0, 0.0]));
        index.insert("b".into(), vec_of(&[1.0, 0.0, 0.0]));
        assert_eq!(index.len(), 1);
        assert!(index.get("b").is_none());
    }

    #[test]
    fn 二进制编解码往返一致() {
        let mut index = SemanticIndex::default();
        index.insert("记忆-1".into(), vec_of(&[0.6, 0.8]));
        index.insert("记忆-2".into(), vec_of(&[0.0, 1.0]));
        let bytes = index.encode();
        let decoded = SemanticIndex::decode(&bytes).expect("应可解码");
        assert_eq!(decoded.dim(), 2);
        assert_eq!(decoded.len(), 2);
        assert!(decoded.get("记忆-1").is_some());
    }

    #[test]
    fn 损坏文件返回空索引() {
        assert!(SemanticIndex::decode(b"not-a-valid-index").is_none());
        assert!(SemanticIndex::decode(&[]).is_none());
    }

    /// ★v3：基文件 + 增量日志重放 = 完整索引。
    #[test]
    fn 增量日志往返一致() {
        let dir = tempfile::TempDir::new().expect("应创建临时目录");
        let mut index = SemanticIndex::default();
        index.insert("base-1".into(), vec_of(&[1.0, 0.0]));
        index.save(dir.path()).expect("基文件应落盘");

        // 追加两条到日志（模拟 flush 的增量路径）
        SemanticIndex::append_journal(
            dir.path(),
            &[
                ("j-1".into(), vec_of(&[0.0, 1.0])),
                ("j-2".into(), vec_of(&[0.7, 0.7])),
            ],
        )
        .expect("日志应追加成功");

        let loaded = SemanticIndex::load(dir.path());
        assert_eq!(loaded.len(), 3, "★基文件 1 条 + 日志 2 条");
        assert!(loaded.get("base-1").is_some());
        assert!(loaded.get("j-1").is_some());
        assert!(loaded.get("j-2").is_some());
    }

    /// ★v3：日志维度与基文件不一致（换模型）⇒ 整份跳过，绝不混用向量空间。
    #[test]
    fn 日志维度不一致时整份跳过() {
        let dir = tempfile::TempDir::new().expect("应创建临时目录");
        let mut index = SemanticIndex::default();
        index.insert("base-1".into(), vec_of(&[1.0, 0.0]));
        index.save(dir.path()).expect("基文件应落盘");

        SemanticIndex::append_journal(dir.path(), &[("j-1".into(), vec_of(&[1.0, 0.0, 0.0]))])
            .expect("日志应追加成功");

        let loaded = SemanticIndex::load(dir.path());
        assert_eq!(loaded.len(), 1, "★维度不一致的日志必须整份跳过");
        assert!(loaded.get("j-1").is_none());
    }

    /// ★v3 规模判据：追加日志的耗时**不随索引规模增长**（整份重写则线性增长）。
    ///
    /// 运行：`cargo test --release --lib --features ml bench_journal_scaling -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_journal_scaling() {
        let dim = 128usize;
        let mut results = Vec::new();
        for n in [10_000usize, 100_000usize] {
            let dir = tempfile::TempDir::new().expect("应创建临时目录");
            let mut index = SemanticIndex::default();
            for i in 0..n {
                index.insert(format!("id{}", i), vec![0.1f32; dim]);
            }
            // 整份重写（旧路径）
            let t0 = std::time::Instant::now();
            index.save(dir.path()).expect("整份落盘应成功");
            let full_ms = t0.elapsed().as_secs_f64() * 1000.0;

            // 追加 100 条（新路径）
            let items: Vec<(String, Vec<f32>)> = (0..100)
                .map(|i| (format!("new{}", i), vec![0.1f32; dim]))
                .collect();
            let t0 = std::time::Instant::now();
            SemanticIndex::append_journal(dir.path(), &items).expect("追加应成功");
            let append_ms = t0.elapsed().as_secs_f64() * 1000.0;
            println!(
                "[bench] n={} 整份重写={:.1}ms 追加100条={:.1}ms",
                n, full_ms, append_ms
            );
            results.push((n, full_ms, append_ms));
        }
        // 判据：100k 的追加耗时不应超过 10k 的 3 倍（整份重写会接近 10 倍）
        let (_, _, a10k) = results[0];
        let (_, _, a100k) = results[1];
        assert!(
            a100k < a10k * 3.0 + 5.0,
            "★追加耗时随规模增长：10k={:.1}ms 100k={:.1}ms",
            a10k,
            a100k
        );
    }

    #[test]
    fn 空索引检索返回空() {
        let index = SemanticIndex::default();
        assert!(index.search(&[1.0, 0.0], 5).is_empty());
    }

    #[test]
    fn 长文本切分为首中尾三窗口() {
        // 三段内容互不相同，避免去重把窗口合并
        let long = format!(
            "{}{}{}",
            "甲".repeat(CHUNK_CHARS),
            "乙".repeat(CHUNK_CHARS),
            "丙".repeat(CHUNK_CHARS)
        );
        let windows = chunk_text(&long);
        assert_eq!(windows.len(), 3, "长文本应切出首/中/尾三个窗口");
        assert!(windows.iter().all(|w| w.chars().count() == CHUNK_CHARS));
        assert!(windows[0].starts_with('甲'));
        assert!(windows[2].starts_with('丙'));
    }

    #[test]
    fn 短文本不切分() {
        let short = "短记忆内容";
        assert_eq!(chunk_text(short), vec![short.to_string()]);
    }

    #[test]
    fn 同一记忆多窗口取最大余弦() {
        let mut index = SemanticIndex::default();
        // 同一记忆两个窗口：一个与查询正交，一个与查询同向
        index.insert("m1".into(), vec_of(&[0.0, 1.0]));
        index.insert("m1".into(), vec_of(&[1.0, 0.0]));
        index.insert("m2".into(), vec_of(&[0.7, 0.7]));
        let hits = index.search(&[1.0, 0.0], 2);
        assert_eq!(hits.len(), 2, "同一记忆的多个窗口应聚合为一条");
        assert_eq!(hits[0].0, "m1");
        assert!((hits[0].1 - 1.0).abs() < 1e-5, "应取最大余弦");
    }
}
