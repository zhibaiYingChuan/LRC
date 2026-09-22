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
use crate::graph_store::{EdgeType, GraphMemoryStore, MemoryEdge};
use crate::memory_types::{DecayConfig, EntityKind, Importance, Memory, MemoryType, PrivacyLevel};
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

/// hub 实体判定·文档频率占比阈值（v0.9.7，PREREG §3.45.5）
///
/// **问题**：过于泛化的实体会产生海量零区分度的假关联。
/// 实测（§3.45.5）一个真实样例库中，`shared_entity` 关联的 **95.7%** 来自
/// 两个项目名型实体（`玄盾` 出现于 70% 记忆、`LRC` 出现于 30%），
/// 它们"共享"近乎恒真 ⇒ 不是关联，是噪声。**假关联比无关联更糟**——污染用户信任。
///
/// **阈值取值依据（实测定标，非拍脑袋）**：在 53 条真实记忆上实测实体 df 分布，
/// 存在**极宽的断层**：
///
/// | 实体类型 | df / 总数 | 占比 |
/// |---|---|---|
/// | 项目名型（hub） | 37 / 16 | **69.8% / 30.2%** |
/// | 具体产物型（有效） | 1 ~ 7 | **1.9% ~ 13.2%** |
///
/// ⇒ 断层位于 13.2% 与 30.2% 之间，取 **0.20** 居中，
/// 两侧各留 ≥6.8pp 余量 ⇒ **对样本波动鲁棒**（详见 `temp/hub-calib.py` 输出）。
///
/// **为什么不需要语义判断**（承 §3.45.8 方法论 99）：
/// 该判定只用频次统计，**不涉及"两个实体是否同一对象"的语义问题**，
/// 因此可以自动执行——这与"判断两条记忆是否真的同一次经历"（需知情者）
/// 是完全不同的两类问题。
///
/// **⚠ 分母必须按项目内计算，不能用全库（§3.49 修正）**：
/// 初版用**全库占比**判定，在多项目混合库中会**系统性漏判**——
/// 实测：`app.js` 在 LRC 项目内占 **43.8%**（典型前端主文件，应判为 hub），
/// 但全库占比仅 **13.2%**（因分母混入 37 条玄盾记忆而被**稀释**）⇒ 未被拦下，
/// 结果它单独贡献了 **43.2% 的间接边**（95/220 条），成为最大的假关联源。
/// ⇒ 现改为**按记忆所属 project 分组统计 df**（见 [`MemoryStore::hub_entities`]），
/// 无 project 的记忆归入 `_global_` 组。同一实体在**任一**项目内达阈值即判为 hub。
const HUB_ENTITY_DF_RATIO: f32 = 0.20;

/// hub 实体判定·文档频率绝对下限
///
/// **用途**：小库（记忆数很少）时占比法会误伤——例如全库仅 5 条记忆时，
/// 出现 2 次的实体占比 40% 会被误判为 hub，但它显然是具体产物。
/// 要求 df **同时** 达到此绝对下限才可能是 hub。
///
/// 取值 5：低于 5 条记忆共享的实体，其关联对最多 C(4,2)=6 对，
/// 规模上不构成"噪声淹没信号"的问题，无需过滤。
const HUB_ENTITY_MIN_DF: usize = 5;

/// 间接关联（2 跳）的**中间节点数上限**
///
/// **用途**：`expand_associations` 的第二层从"已补入的 1 跳结果"里挑中间节点
/// 再走一步。若不限制中间节点数，每个 1 跳结果都要做一次
/// `associations_in`（每次含全库遍历 + 查表），会被放大成
/// `max_out × O(全库)`。
///
/// **取值 3**：与 `max_out` 的默认量级（server 侧 `ASSOCIATION_EXPAND_MAX = 3`）
/// 对齐 —— 中间节点数不超过"起点数"，避免第二层的成本超过第一层。
/// 这是**成本约束**而非质量阈值：中间节点再多也只是产出更多候选，
/// 而配额（`max_out`）决定了最终留下几条。
const INDIRECT_MID_MAX: usize = 3;

/// 判定某实体是否为 hub（过于泛化、关联无区分度）
///
/// 判据 = **占比高（≥[`HUB_ENTITY_DF_RATIO`]）且绝对频次足够（≥[`HUB_ENTITY_MIN_DF`]）**。
/// 双条件设计的原因见两常量各自的文档。
fn is_hub_entity(df: usize, total: usize) -> bool {
    if total == 0 {
        return false;
    }
    df >= HUB_ENTITY_MIN_DF && (df as f32) / (total as f32) >= HUB_ENTITY_DF_RATIO
}

/// 自动事件推断·时间窗口（秒）= 1 小时（v0.9.8，PREREG §3.54）
///
/// **为什么需要自动事件**：`event_id` 是"共同经历"的唯一载体，但实测
/// 真实库填写率 **0.00%**（§3.51）——记录层联想因此**产出恒为 0**。
/// 而"同项目 + 同一小时写入"是**客观事实**（不需要任何用户判断），
/// 实测覆盖 **70.7%（global）/ 90.4%（dev）**，且抽样证实桶内确实是
/// **同一次连续工作**（如 CSCD 08:01→08:59 的重构→Phase2→Phase4→审查）。
///
/// **窗口取值依据（实测定标，非拍脑袋）**：
///
/// | 窗口 | 覆盖率(global) | 关联对 | 大桶纯度 |
/// |---|---|---|---|
/// | **1h** | **70.7%** | **627** | 0.0474（稳定） |
/// | 2h | 79.1% | 945 | 0.0486 |
/// | 4h | 84.9% | 1305 | 0.0438 |
/// | 8h | 89.6% | **1908** | **0.0365**（明显下降） |
///
/// ⇒ 1h 用 3 倍少的关联对换取足够覆盖，且纯度不随窗口放大而退化。
const AUTO_EVENT_WINDOW_SECS: i64 = 3600;

/// 自动事件推断·**批量写入排斥**阈值（秒/条）（v0.9.8，实测定标）
///
/// **为什么必须排斥**：实测发现"同一小时内"存在两类**截然不同**的桶：
///
/// | 类别 | 时间戳特征 | 实例 |
/// |---|---|---|
/// | **一次经历**（应关联） | 跨度/条数 **≥ 106.6 秒** | CSCD 08:01→08:59 共 14 条 |
/// | **批量写入**（应排斥） | 跨度/条数 **≤ 17.3 秒** | XuanDun 11 条**全在同一秒** |
///
/// 后者是脚本批量导入/测试语料注入（dev 库 32 条在 **3 秒内**写完），
/// 桶内内容互相无关（纯度 0.0027 ≈ 随机）——把它们当作"同一次经历"
/// 会产出**海量假关联**（比不做更糟，污染用户信任）。
///
/// **判据 = 时间跨度 / 条数 < 60 秒**（平均每条不足 1 分钟 ⇒ 非人工可产出）。
/// **断层实测**：坏桶最大 17.3s ↔ 好桶最小 106.6s，**89 秒空白带**，
/// 取 60 居中，两侧各留 >40s 余量 ⇒ 对样本波动鲁棒。
/// **为什么用"跨度/条数"而非"固定条数上限"**：实测两库坏桶规模分布
/// （global 最坏 11 条而好桶有 14 条）⇒ **规模无法区分**，但时间密度可以。
const AUTO_EVENT_MIN_SECS_PER_MEMORY: i64 = 60;

/// 产物标识符（artifact）的**项目内占比** hub 阈值（v0.9.8，实测定标）
///
/// # 为什么需要这一维度（承 PREREG_MEMORY_ASSOCIATION.md §三）
///
/// 实测：`entities` 填写率 **0.00%** ⇒ `shared_entity` 在真实库上**产出恒为 0**；
/// 且各维度覆盖的**记忆子集几乎不重叠**（`same_event_auto` 覆盖日常写入，
/// 谱系维度只覆盖 `synthesis` 类型）⇒ 实测**多维并存率仅 1.02%**
/// （从单条记忆出发能看到 ≥2 种关系类型的比例）。
///
/// §3.54.8 方法论 110 已确立规范：**依赖人工填写的通路，必须同时提供
/// 一条零填写负担的客观替代路径**，判据是覆盖率 <5% 即视为**未激活**。
/// 本维度即 `shared_entity` 的那条替代路径——输入从「人工填 entities」
/// 换成「从正文**形态**检出具体产物标识符」。
///
/// # 为什么这是"形态检出"而非"语义推断"（承 §3.44.5 方法론 97 的边界）
///
/// §3.44.5 禁止"自动推断"，其判据是：**该判断是否需要「对世界做一次判断」**。
/// 本维度只做前一件事、不做后一件：
/// - ✅ **可自动**：「`app.js` 是文件名」——只由这 6 个字符决定，不引用其他记忆、
///   不引用外部世界（属 §3.44.5 明文允许的「格式化/校验」类）
/// - ❌ **留知情者**：「这两条是不是同一次经历」——需理解语义
///
/// 且**不回写 `entities` 字段**、**独立命名 `shared_artifact`**、
/// `why` 显式标注检出方式——与 `same_event_auto` 对 `same_event` 的关系同构
/// （§3.54 已确立的"不越界的自动推断"三约束）。
///
/// # 阈值取值依据（实测定标，非拍脑袋）
///
/// 在 **1283 条真实记忆**（注入语料已实测覆盖率仅 1.0%，不参与标定）上
/// 统计 artifact 的**项目内占比**（承 §3.49：分母必须按项目分层，
/// 用全库分母会稀释而系统性漏判）：
///
/// | 层 | 占比区间 | 实例 | 性质 |
/// |---|---|---|---|
/// | **恒真层（应拦）** | **42.86% ~ 100%** | `Cargo.toml`(100%) / `README.md`(75%) / `app.py`(66.7%) | 出现于多数记忆 ⇒ "共享"近乎恒真 ⇒ 零区分度 |
/// | 断层 | **42.86% → 28.57%（14.29pp）** | — | 最大有效断层 |
/// | 具体产物层（保留） | ≤28.57% | `app.js`(18.2%) / `v1_api.rs`(14.5%) / `commands.rs`(14.3%) | 具体产物 ⇒ 关联有信息量 |
///
/// ⇒ 取 **0.35**：位于断层中点（35.71%）且**两侧各留 ≥6.4pp 余量**。
///
/// # ⚠ 已知未决项（必须如实保留，不得静默）
///
/// **`app.js`（占比 18.24%）会逃过本阈值**，而它单独贡献 **16.04%** 的全部关联对
/// （df=69 ⇒ C(69,2)=2346 对），是全库**最大的单点来源**。
/// 两个选项各有代价，当前**择优保留现状并标注**：
/// - 若降到 0.18 以拦下它，则该阈值**落在连续分布内部**（18.24% / 14.47% /
///   14.29% / …相邻差仅 3~4pp）⇒ 属"拍脑袋"，违反 §3.49.5「无断层则不设阈值」；
/// - 保留 0.35 ⇒ 如实承认 `app.js` 未被拦下。
///
/// **为什么不删除该维度**：即使含 `app.js` 噪声，它仍把多维并存率从 1.02%
/// 提升到 17.35%（**合并口径**，见下），且样例经人工核验为
/// "同一具体函数在不同时间被碰过"的**有效跨会话关联**（`fetchWithTimeout`
/// ↔ v0.8.2 修复 / v0.8.4 审计）——这是 BGE 给不出的类型。
const HUB_ARTIFACT_DF_RATIO: f32 = 0.35;

/// hub artifact 判定·文档频率绝对下限
///
/// 与 [`HUB_ENTITY_MIN_DF`] 同理由：小库时占比法会误伤具体产物
/// （全库 3 条时出现 2 次的占比 66% 但显然是具体文件）。
/// 低于 5 条记忆共享的 artifact 最多 C(4,2)=6 对，不构成噪声淹没。
const HUB_ARTIFACT_MIN_DF: usize = 5;

/// 产物标识符（artifact）词法抽取：后缀长度上限
///
/// **为什么用"形态类"而非后缀白名单**：用户要求「不能是死的映射」——
/// 白名单（50 个后缀的静态表）会让新语言后缀（`.zig`/`.proto`）**静默漏检**。
/// 实测对比（1283 条真实记忆）：
///
/// | 方案 | 覆盖率 | 问题 |
/// |---|---|---|
/// | 固定白名单（50 后缀） | 75.60% | 漏检 48 条（含 `tauri.conf`/`daoti.onnx`/`hello_libc.elf`） |
/// | **形态类（本方案）** | **79.35%** | 多抓 48 条，且正确排除 `v0.9`/`127.0`/`2.35`（版本号/IP，共 2268 次） |
///
/// 形态类判据：**后缀以 ASCII 字母开头、长度 1~8、其余为字母数字**——
/// 该约束天然排除「纯数字后缀」（版本号 `v0.9.8`、浮点 `2.35`、IP `127.0`）。
const ARTIFACT_MAX_SUFFIX_LEN: usize = 8;

/// 产物标识符最小长度（过短的多为短语缩写，非具体产物）
const ARTIFACT_MIN_TOKEN_LEN: usize = 4;

/// 判定某 artifact 是否为 hub（过于泛化、关联无区分度）
///
/// 判据 = **占比高（≥[`HUB_ARTIFACT_DF_RATIO`]）且绝对频次足够（≥[`HUB_ARTIFACT_MIN_DF`]）**。
/// 与 [`is_hub_entity`] 同构——复用同一条纪律（占比型判定的分母必须按项目分层，§3.49）。
fn is_hub_artifact(df: usize, total: usize) -> bool {
    if total == 0 {
        return false;
    }
    df >= HUB_ARTIFACT_MIN_DF && (df as f32) / (total as f32) >= HUB_ARTIFACT_DF_RATIO
}

/// 从正文抽取**形态可识别的产物标识符**（纯形态，无语义判断）
///
/// 扫描规则：把正文按"是否属于标识符字符集"（ASCII 字母/数字/`_`/`-`/`.`）
/// 切成候选 token，再对每个候选套一道形态判据：
///
/// 1. 含至少一个 `.`（点号是"文件名/限定名"的形态特征）；
/// 2. 长度 ≥ [`ARTIFACT_MIN_TOKEN_LEN`]；
/// 3. 以**最后一个点号**为界取后缀，后缀长度 1~[`ARTIFACT_MAX_SUFFIX_LEN`]
///    且**首字符为 ASCII 字母**、其余为字母数字
///    （该条排除版本号 `v0.9.8` / 浮点 `2.35` / IP `127.0.0.1` 的尾段）；
/// 4. 前缀部分至少含一个 ASCII 字母（排除 `1.5` 这类纯数字）；
/// 5. 只取 ASCII（CJK 文本中不会出现"文件名"形态，且避免 CJK 被误切）。
///
/// **为什么不用正则库**：本项目未引入 `regex` 依赖，且本抽取是**手写状态机**
/// 即可完成的线性扫描；引入依赖会为单一用途增加编译期与二进制体积成本。
/// 手写实现的行为已由单元测试逐案锁定（含负向对照）。
///
/// **为什么取"最后一个点号"**：`tauri.conf.json` 的后缀应是 `json` 而非 `conf`；
/// `delivery_audit_v1.3.2.md` 的后缀应是 `md`。取最后一个点号同时让
/// 版本号形态（`v1.3.2.md` 中 `v1.3.2` 被跳过、整串作为前缀）自然成立。
///
/// 返回**去重后**的列表（同一条记忆内同一产物只计一次），保持首次出现顺序。
fn extract_artifacts(text: &str) -> Vec<String> {
    /// 标识符允许的字符：ASCII 字母/数字/下划线/连字符/点号。
    #[inline]
    fn is_ident_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'
    }

    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();

    // 收尾逻辑抽成闭包，供"遇到非标识符字符"与"文本结束"两处复用
    // （避免两处实现漂移——历史上这类收尾漏写导致的静默丢 token 很难察觉）
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            if let Some(tok) = normalize_artifact(cur) {
                if !out.contains(&tok) {
                    out.push(tok);
                }
            }
            cur.clear();
        }
    };

    for ch in text.chars() {
        if is_ident_char(ch) {
            cur.push(ch);
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// 对单个候选 token 套形态判据，通过则归一化为小写返回
///
/// 归一化为小写的原因：`App.js` 与 `app.js` 指同一产物，
/// 大小写差异不应产生两条互不相连的关联。
fn normalize_artifact(tok: &str) -> Option<String> {
    let lower = tok.to_lowercase();
    if lower.chars().count() < ARTIFACT_MIN_TOKEN_LEN {
        return None;
    }
    // 必须以最后一个点号分隔，且两侧都非空
    let idx = lower.rfind('.')?;
    if idx == 0 || idx + 1 >= lower.len() {
        return None;
    }
    let (prefix, suffix) = (&lower[..idx], &lower[idx + 1..]);
    if suffix.chars().count() > ARTIFACT_MAX_SUFFIX_LEN {
        return None;
    }
    // 后缀：首字符必须是 ASCII 字母（排除纯数字后缀 ⇒ 版本号/浮点/IP）
    let mut sfx = suffix.chars();
    let first = sfx.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !suffix.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    // 前缀至少含一个 ASCII 字母（排除 `1.5`、`2026.09` 这类纯数字）
    if !prefix.chars().any(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(lower)
}

/// 计算 artifact 的**项目内占比**表：`artifact -> 最大项目内占比`
///
/// **为什么要抽成独立函数**：hub 判定依赖"分母按项目分层"（§3.49 的教训），
/// 而占比需遍历全库统计。检索主路径要复用该表（避免每起点重算 O(N)）。
fn artifact_project_ratio(all: &[Memory]) -> std::collections::HashMap<String, (usize, usize)> {
    use std::collections::{HashMap, HashSet};

    // 各项目的记忆总数（分母）
    let mut proj_total: HashMap<String, usize> = HashMap::new();
    for m in all {
        *proj_total
            .entry(m.project.as_deref().unwrap_or("_global_").to_string())
            .or_insert(0) += 1;
    }

    // (项目, artifact) -> 该 artifact 在该项目内出现的记忆 id 集合
    let mut per_proj: HashMap<(String, String), HashSet<String>> = HashMap::new();
    for m in all {
        let proj = m.project.as_deref().unwrap_or("_global_").to_string();
        for a in extract_artifacts(&m.content) {
            per_proj
                .entry((proj.clone(), a))
                .or_default()
                .insert(m.id.clone());
        }
    }

    // 取"任一项目内占比最大"的那次（与 hub_entity_set 同口径）
    let mut out: HashMap<String, (usize, usize)> = HashMap::new();
    for ((proj, a), ids) in per_proj {
        let total = proj_total.get(&proj).copied().unwrap_or(0);
        let df = ids.len();
        let ratio = if total == 0 {
            0.0
        } else {
            df as f32 / total as f32
        };
        let entry = out.entry(a).or_insert((df, total));
        let cur_ratio = if entry.1 == 0 {
            0.0
        } else {
            entry.0 as f32 / entry.1 as f32
        };
        if ratio > cur_ratio {
            *entry = (df, total);
        }
    }
    out
}

/// artifact → 出现过它的记忆 ID 集合（供关联展开查表）
fn artifact_members(all: &[Memory]) -> std::collections::HashMap<String, Vec<String>> {
    use std::collections::HashMap;
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for m in all {
        for a in extract_artifacts(&m.content) {
            out.entry(a).or_default().push(m.id.clone());
        }
    }
    out
}

/// artifact 维度的两张全库统计表（一次构建，多次复用）
///
/// **为什么把两张表打包**：它们必须**同源构建**（都来自同一次
/// `extract_artifacts` 遍历）——若分别构建，两处抽取实现漂移时
/// 「被 hub 判定的名字」与「实际连接的成员」会不一致，
/// 产生"拦了 A 却仍由 A 连出边"这类静默错误。
/// 打包后**结构上不可能**出现该不一致。
struct ArtifactTables {
    /// artifact → (df, 该项目内记忆数)：用于 hub 判定（占比按项目分层，§3.49）
    ratio: std::collections::HashMap<String, (usize, usize)>,
    /// artifact → 出现过它的记忆 ID 列表：用于展开关联
    members: std::collections::HashMap<String, Vec<String>>,
}

impl ArtifactTables {
    /// 一次遍历构建两张表（供检索主路径与详情页复用）
    fn build(all: &[Memory]) -> Self {
        Self {
            ratio: artifact_project_ratio(all),
            members: artifact_members(all),
        }
    }

    /// 该 artifact 是否应被跳过（过泛化、或成员不足 2 条而无关联可言）
    fn is_skipped(&self, artifact: &str) -> bool {
        let (df, total) = self.ratio.get(artifact).copied().unwrap_or((0, 0));
        if is_hub_artifact(df, total) {
            return true;
        }
        // 只有一条记忆提到它 ⇒ 无关联对可言（但仍可能是有效产物，
        // 只是本维度对它不产出边）
        // 注：此处不用 `is_none_or`（Rust 1.82+），项目 MSRV 为 1.80
        match self.members.get(artifact) {
            None => true,
            Some(ids) => ids.len() < 2,
        }
    }
}

/// 判定某个"同项目 + 同窗口"分组是否构成**自动事件**
///
/// 判据：条数 ≥ 2（否则无关联可言）且**不是批量写入**
/// （时间跨度/条数 ≥ [`AUTO_EVENT_MIN_SECS_PER_MEMORY`]）。
///
/// **为什么条数 ≥ 2 才能成为事件**：单条记忆没有"共同经历"可言——
/// 它与谁都不是"同一次"。（但它仍可作 shared_entity 关联的来源。）
fn is_auto_event_cluster(count: usize, span_secs: i64) -> bool {
    if count < 2 {
        return false;
    }
    // 跨度/条数：整数除法在 count 很大时足够（判据本身是量级判断）
    span_secs / (count as i64) >= AUTO_EVENT_MIN_SECS_PER_MEMORY
}

/// 实词集 Jaccard 相似度——**「意外性」的廉价代理**（v0.9.8）
///
/// # 它是做什么的（这是本轮的核心设计）
///
/// 用户对"联想"的价值判据（§3.43.9 逐字）：
/// > 「道体的价值判据不是'能否产出关联图'，而是'**产出的关联图中，
/// >   有多少条是 BGE 给不出的**'」
///
/// 实测（`temp/assoc-bge-giveup.py`，真实库 1285 条 + BGE 全库排名）：
/// 联想产出的对，**BGE 排名中位仅 0.0685**（= BGE top 6.9%）——
/// 即**大部分联想对 BGE 本来就能找到**，那部分没有增量。
///
/// 因此需要"该对是否 BGE 给不出"的判据。精确做法要编码全库（热路径不可接受），
/// 故用本函数做**廉价代理**。实测有效性（`temp/assoc-proxy-test.py`）：
///
/// | 代理 | Spearman（vs BGE 排名） | AUC（判"给不出"） |
/// | --- | --- | --- |
/// | **实词 Jaccard（本函数）** | **−0.6855** | **0.9275** |
/// | 3-gram Jaccard | −0.6678 | 0.9099 |
/// | 字符集 Jaccard | −0.5964 | 0.8748 |
///
/// # 为什么它是"代理"而不是"又一层相似度打分"
///
/// **陷阱检验**（同上脚本）：若代理只是 BGE 的粗粒度版本，那用它筛低相似对
/// 与用 BGE 筛等价 ⇒ 无独立价值。实测**否证了该陷阱**：
/// 代理筛出的低分对与 BGE 低相似对**重叠 Jaccard 仅 0.081**（θ_p=0.02）；
/// 且代理选出的对里 **BGE 给不出率 98.4%**，而随机同量对照仅 65.3%
/// （**+33.1pp**）⇒ 代理确实在**识别意外关联**，不是在复刻 BGE。
///
/// # 词表口径
/// 复用生产既有的 CJK bigram + ASCII 词抽取口径，并过滤泛指虚词
/// （「什么」「怎么」「可以」等）——虚词命中不代表主题相关，
/// 留着会让"两条都在讲废话"的记忆产生虚假高相似（承 §3.47 的泛指 bigram 教训）。
fn content_words(text: &str) -> std::collections::HashSet<String> {
    use std::collections::HashSet;
    // 泛指虚词（单字）：这些字参与组成的 bigram 不携带主题信息
    const GENERIC_CHARS: &[char] = &[
        '的', '是', '了', '在', '和', '有', '就', '不', '人', '都', '一', '上', '也', '很', '到',
        '说', '要', '去', '你', '会', '着', '没', '看', '好', '自', '己', '这', '那', '么', '些',
        '什', '怎',
    ];
    // 泛指 bigram（双字词）：整体是功能短语，非主题

    let lower = text.to_lowercase();
    let mut out: HashSet<String> = HashSet::new();

    // ASCII 词（长度 ≥2，含 `_`/`-`），保留整词以便与 CJK 区分
    let mut buf = String::new();
    let flush_ascii = |buf: &mut String, out: &mut HashSet<String>| {
        if buf.chars().count() >= 2 {
            out.insert(buf.clone());
        }
        buf.clear();
    };
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            buf.push(ch);
        } else {
            flush_ascii(&mut buf, &mut out);
        }
    }
    flush_ascii(&mut buf, &mut out);

    // CJK bigram（跳过含泛指虚词或跨 ASCII 边界的组合）
    let cjk: Vec<char> = lower
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_ascii())
        .collect();
    for w in cjk.windows(2) {
        let (a, b) = (w[0], w[1]);
        if GENERIC_CHARS.contains(&a) || GENERIC_CHARS.contains(&b) {
            continue;
        }
        out.insert(format!("{a}{b}"));
    }
    out
}

/// 实词 Jaccard 相似度（0.0 ~ 1.0）。两文本均无实词时返回 0.0。
fn word_jaccard(
    a: &std::collections::HashSet<String>,
    b: &std::collections::HashSet<String>,
) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// 门控：道体预判卦是否参与候选剪枝（**v0.9.8 起默认关闭**）
///
/// `LRC_DAOTI_PREVIEW_PRUNE=1` / `=true` 时开启；未设或其它值 ⇒ 关闭。
///
/// **为什么默认关闭**：该剪枝的第二证据 `daoti_preview_bagua` 与
/// `bagua_index` 同源（均出自洛书编码器 + `mirror_project`），
/// 而 `daoti/PREREG_ACTIVE_DISCOVERY.md` §3.37 已实测该编码**不读语义**——
/// 打乱字符顺序后分类 100% 不变（根因：9 维特征仅含字符密度/字符熵/位置权重），
/// 且真实语料上最大单类占比 99~100%（`data_beir_eval` 3633 条全落同一卦）。
///
/// 抽成独立函数（而非内联 `matches!`）的原因：门控判据是**默认值契约**
/// 的单一事实来源，测试须直接断言本函数而非复刻判据（承方法论 79：
/// 两处实现必然漂移，而漂移后的测试会失去鉴别力）。
pub(crate) fn daoti_preview_prune_enabled() -> bool {
    matches!(
        std::env::var("LRC_DAOTI_PREVIEW_PRUNE").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// 关系类型的中文标签（用于把路径写成人可读的解释）
///
/// 存在的原因：`why` 字段的判据是"**人类可解释**"（用户指引 §六）。
/// 直接输出 `same_event` 这类标识符对用户无意义，必须给出中文说明。
///
/// **注意（§3.49.2）**：间接边的解释**不使用**本函数——
/// 它直接拼接两段的真实 `why`（含具体实体名，如"共享 thing 实体「app.js」"），
/// 因为只写"共享实体"而不写共享的是什么，恰好违背可解释判据。
/// 本函数保留供 `why` 的生成路径与未来按类型聚合展示复用。
pub(crate) fn relation_label(relation: &str) -> &'static str {
    match relation {
        "same_event" => "同一次经历",
        "same_event_auto" => "同期工作记录",
        "shared_entity" => "共享实体",
        "shared_artifact" => "共享具体产物",
        "derived_from" => "结晶来源",
        "crystallized_into" => "被结晶为",
        "evolved_from" => "被更新过",
        "indirect" => "间接关联",
        // ═══ ★符号层（§5.3 逻辑关系）标签（2026-09-18 修 S5）═══
        //
        // ## 此前的问题
        //
        // 符号层边经 `/external-edge` 落图后，会在 `expand_associations`
        // 的并入段被读回并渲染。但那段用的是**本表**，而本表当时没有
        // 这 5 个键 ⇒ 全部落到兜底 `"相关联"` ⇒ 用户拿到一条**无法分辨
        // 是因果、时序还是约束**的联想。
        //
        // 更矛盾的是：同一轮里作者在 `server.rs` 明确写过
        //   「不能复用 relation_label：那是**记录层**标签表，
        //     符号层的 CONSTRAINT/COORDINATE 撞进去会全部落到兜底值
        //     『相关联』，把类型信息抹平。两张表语义不同，必须分开。」
        // 并为此新写了 `structural_rel_label` —— 但它只用在了 MCP 的
        // `/cycle`、`/build_edges` 两个**写入端回显**分区上，
        // **真正落图并进入 recall 的那条路没接上**。
        //
        // ## 修法：把符号层键并入本表，而非让调用方换函数
        //
        // 为什么并入而不是"让调用方改用 structural_rel_label"：
        //   · `relation` 字段存的是**小写**（`EdgeType::as_str()`），
        //     而 `structural_rel_label` 的键是**大写**（COORDINATE/…）
        //     ⇒ 直接换函数会**全部失配**（这是上一轮已有的隐患）。
        //   · 本表已是"关系名 → 中文标签"的**唯一**通用入口，
        //     分散成两表就还会再有第三处漏接。
        //   ⇒ 保留 `structural_rel_label` 供大写输入（道体响应直接回显），
        //     本表覆盖小写输入（图存储读出），两者标签文案**逐字一致**。
        "cause" => "因果",
        "temporal" => "时序",
        "constraint" => "相错（约束）",
        "facilitate" => "变爻（促成）",
        "coordinate" => "相综/互卦（协同）",
        _ => "相关联",
    }
}

/// 该图边类型是否属于**符号层（§5.3 逻辑关系）**。
///
/// 用途：`expand_associations` 的落盘边并入段据此**分区**（2026-09-18 修 S4）。
/// 为什么需要：图里有三类来源不同的边，混在一起会混淆证据性质：
///   · 图存储内生（系统推断）：`contradicts` / `evolves` / `synthesizes_from` / `related_to`
///   · 记录层（记录事实）：`same_event` 等 7 类
///   · 符号层（结构推导）：`cause` / `temporal` / `constraint` / `facilitate` / `coordinate`
pub(crate) fn is_symbolic_edge_type(rel: &str) -> bool {
    matches!(
        rel,
        "cause" | "temporal" | "constraint" | "facilitate" | "coordinate"
    )
}

/// 该图边类型是否属于**记录层**（记录事实）。
///
/// # 生产调用方
///
/// `expand_associations` 的**落图段**用它做白名单：只把记录层关系写进图。
/// （语义上那段就是"记录层关系图化"；符号层边有自己的写入口
/// `/v1/memories/external-edge`，且是**从图里读出来**的，不该再写回。）
///
/// # 与 [`is_symbolic_edge_type`] 一起构成「图边三来源」的完整分类
///
///   · 图存储内生（系统推断）：`contradicts` / `evolves` / `synthesizes_from` / `related_to`
///     —— 上述两个判定都返回 `false`（**不属于**记录层/符号层）
///   · 记录层（记录事实）：本函数返回 `true`
///   · 符号层（结构推导）：[`is_symbolic_edge_type`] 返回 `true`
///
/// 该分类由 `test_edge_type_three_source_taxonomy_is_exhaustive` 断言
/// **穷尽且互斥**（新增 `EdgeType` 变体若忘记归类会当场变红）。
pub(crate) fn is_record_edge_type(rel: &str) -> bool {
    matches!(
        rel,
        "same_event"
            | "same_event_auto"
            | "shared_entity"
            | "shared_artifact"
            | "derived_from"
            | "crystallized_into"
            | "evolved_from"
    )
}

/// 数据契约类型重导出（v0.9.7，GLOBAL_CODE_REVIEW_REPORT P2-2「MemoryStore God Object」）
///
/// 以下类型的**本体**已外提至 [`crate::memory_store_types`]（纯数据契约、零状态依赖）。
/// 在此重导出以保持 `crate::memory_store::Xxx` 既有路径与
/// `use crate::memory_store::*` 调用方**零改动**。
pub use crate::memory_store_types::{
    AssociatedMemory, AssociationGraph, GraphEdge, GraphNode, ListFilter, MemoryAssociation,
    MemoryStats, RecallFilter, RecallResult, RegulatorHeartbeat, SortBy, SortOrder, StoredEdge,
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
        // v0.9.8：状态持久化能力**显式可见**。
        //
        // 背景：`load/save_memory_state` 与 `load/save_assoc_frequency` 有
        // trait 默认实现（返回空 + 忽略保存）。若后端未实现且**无人声明**，
        // 用户会看到"联想活性/语义吸铁石压制每次重启归零"却**毫无提示**，
        // 只感觉"联想时好时坏"——这是**静默功能降级**。
        //
        // 注意：此处用 `supports_state_persistence()` 自报开关，而不是
        // 靠"加载到的状态是否为空"推断——首次运行（正常的空）与
        // 后端不持久化（降级的空）在返回值上**完全一样**，无法区分。
        if !persistence.supports_state_persistence() {
            eprintln!(
                "[LRC·持久化] ⚠ 当前后端未实现状态持久化：\
联想活性（近期在想什么）与「语义吸铁石」压制统计**仅在本进程内有效，重启后归零**。\
这是已知限制，不是故障；若需跨重启保留，请使用 JSON 后端（默认）。"
            );
        }
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

    /// 状态驱动发现（方向二）：编码**单条文本**为完整句向量（bge 原始维度）。
    ///
    /// **为什么需要这个入口**（实测驱动）：
    /// 状态驱动发现的语义匹配需要"道体状态锚点 → 句向量"，再与记忆缓存向量
    /// 做点积。9 维洛书投影承载不了语义区分度（实测：语义差异极大的文本
    /// 只落 2 个母卦、真实库 4450 条中 96.8% 同属一卦），故必须用**未投影的
    /// 完整句向量**。
    ///
    /// 契约：
    ///   - ML 编码器不可用（未开 ml feature / 模型缺失）→ 返回 None，调用方
    ///     回退到标签匹配路径（绝不放宽标准，也不报错）；
    ///   - 单条耗时实测 ≈2.25s（bge-base-zh, 6 核已饱和、并发无加速），
    ///     **故调用方只能编码极少量文本**（如单个锚点），不得逐条编码全库。
    #[cfg(feature = "ml")]
    pub fn encode_sentence_vector(&self, text: &str) -> Option<Vec<f32>> {
        self.luoshu_encoder.encode_embedding(text)
    }

    /// 非 ml 构建下的同名入口：恒返回 None（调用方回退标签匹配）。
    #[cfg(not(feature = "ml"))]
    pub fn encode_sentence_vector(&self, _text: &str) -> Option<Vec<f32>> {
        None
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

    /// 确保缓存有效（脏则从持久层重载），**不做任何克隆**
    ///
    /// 供只读元信息查询使用（如"某 ID 是否存在"）——这类调用不需要
    /// `Memory` 本体，只需缓存处于可用状态。
    fn ensure_cache_loaded(&self) -> Result<(), PersistenceError> {
        if self.cache.is_dirty() {
            let loaded = self.persistence.load_all_memories()?;
            self.cache.store(loaded);
        }
        Ok(())
    }

    /// 判断指定 ID 的记忆是否存在（v0.9.8 审查 G6 修复引入）
    ///
    /// 与 `memories_by_ids(&[id]).len() == 1` 等价，但**不克隆记忆本体**：
    /// 后者会走 `load_cached()` → `snapshot()`（整库深拷贝）再过滤，
    /// 在逐条调用的存在性校验路径上是纯浪费。
    ///
    /// # 为什么返回 `Result` 而非 `bool`
    ///
    /// 初版写成"出错即 `false`"，但那是**静默降级**：磁盘故障与"该 ID 确实
    /// 不存在"会得到同一个结果，而两者对调用方的含义完全不同——前者是
    /// 系统故障（应上报），后者是**正常的业务过滤**（宁缺勿错，静默跳过）。
    /// 把故障伪装成业务拒绝，排查时看到的是"边被规则拒了"，而无从知道
    /// 持久层不可读。故错误**必须**向上传播（与原来的 `load_cached()?` 一致）。
    pub fn has_memory_id(&self, id: &str) -> Result<bool, PersistenceError> {
        self.ensure_cache_loaded()?;
        Ok(self.cache.contains_id(id))
    }

    /// 按 ID 集合只读取出记忆（P7 主动发现构造探索查询用）。
    ///
    /// 语义：纯读，不做过滤/排序/写回。`ids` 为空时返回空列表。
    /// 抽出本方法的原因：`load_cached` 是私有实现细节，而调用方（discovery）
    /// 需要一个"只读、不触发任何状态写入"的公开入口——若改用 `list_memories`
    /// 则要先全量排序再按 limit 截断，既多算又可能截不到目标 ID。
    pub fn memories_by_ids(&self, ids: &[String]) -> Vec<Memory> {
        if ids.is_empty() {
            return Vec::new();
        }
        let wanted: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
        self.load_cached()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| wanted.contains(m.id.as_str()))
            .collect()
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

    /// 图存储的只读访问（v0.9.8：测试与诊断用）
    ///
    /// 生产路径不需要它——图由 `expand_associations` 内部写入。
    /// 暴露只读引用是为了让测试能**直接断言图的真实内容**，
    /// 而不是只断言返回值（返回值写图失败时仍可能正确，
    /// 只断言返回值会让"图化失效"静默通过）。
    pub fn graph_store_ref(&self) -> Option<&GraphMemoryStore> {
        self.graph_store.as_ref()
    }

    /// 测试用：可变访问图存储，以便**直接写入特定类型的边**。
    ///
    /// **为什么必须可变访问**（与 `graph_store_ref` 同理由，2026-09-18 补）：
    /// 有些边类型**无法经生产路径构造**——例如 `Evolves` /
    /// `SynthesizesFrom` / `Contradicts` / `RelatedTo` 由图存储内生的
    /// 合成/冲突链路产出，而 `add_external_edge` 又（正确地）拒绝它们
    /// ⇒ 若不给测试直写图的口子，就无法验证"这些边不得冒充记录型关联"。
    ///
    /// **为什么只给测试**：生产代码**不应**绕过 `add_external_edge` 的白名单
    /// 直写图（那正是 S3 要防的）。故此处显式标注 `_for_test` 并仅由单测调用。
    #[cfg(test)]
    pub fn graph_store_mut_for_test(&mut self) -> Option<&mut GraphMemoryStore> {
        self.graph_store.as_mut()
    }

    /// 写入一条**外部推导的**关系边（v0.9.8，承《记忆联想系统设计文档》§5.4）
    ///
    /// # 用途
    ///
    /// 道体联想服务（`temp/daoti_assoc`）由结构算子（互/错/综/变）产出候选关系，
    /// 经 HTTP 回传后由本方法落入图。这是 §5.4「候选命中 → 生成边」的写入端。
    ///
    /// # 与 `expand_associations` 内建写入的区别（不可混同）
    ///
    /// | | `expand_associations` 内建 | 本方法 |
    /// |---|---|---|
    /// | 边来源 | **记录层事实**（event_id/entities/source_ids） | **结构算子推导**（道体） |
    /// | 证据性质 | 记录必然成立 | 推导，可能不成立 |
    /// | rel_type | 7 类记录层关系 | 5 类逻辑关系（§5.3） |
    ///
    /// 两者写入同一张图，但 `rel_type` 不同 ⇒ 消费方可按类型区分证据强度。
    ///
    /// # 参数
    /// - `from_id` / `to_id`：两端记忆 ID。**必须都已存在于记忆库**，
    ///   否则边指向不存在的记忆（悬空边），消费时会查到空节点。
    /// - `rel_type`：§5.3 的关系名（大小写皆可，见 `EdgeType::from_relation_str`）
    /// - `weight`：置信度 0~1（超出会被 clamp）
    ///
    /// # 返回
    /// `Ok(true)` 表示新写入；`Ok(false)` 表示已存在（去重）或校验不通过被跳过。
    /// 校验不通过**不报错**——外部推导的候选本就允许被过滤（宁缺勿错）。
    pub fn add_external_edge(
        &mut self,
        from_id: &str,
        to_id: &str,
        rel_type: &str,
        weight: f32,
    ) -> Result<bool, PersistenceError> {
        // 解析关系类型：★走**外部白名单**（只接受 §5.3 五类逻辑关系）。
        //
        // 为什么不能用通用的 `from_relation_str`（2026-09-18 修 S3）：
        //   通用解析器接受**全部 12 类**，包括记录层的 `same_event`——
        //   那是"知情者断言"、证据最强（relation_priority=0、权重 1.0），
        //   且会经 expand_associations 进入 recall 并被渲染为"由记录推导、
        //   必然成立"。若外部可写，任意本机进程都能把两条真实记忆伪造成
        //   "同一次经历"，用户看到的是最高证据等级的**假事实**。
        //   本方法文档上方也明写"5 类逻辑关系"，故这是**实现向注释对齐**。
        let Some(etype) = EdgeType::from_external_rel_str(rel_type) else {
            return Ok(false);
        };
        // 自环无信息量
        if from_id == to_id {
            return Ok(false);
        }
        // ★两端必须都是**已知记忆**：防悬空边（图里出现指向不存在记忆的边，
        //   消费方按图取节点会得到空，用户看到"关联到空"）
        //
        // ★★ 2026-09-18 审查 G6 修复：不再用 `load_cached()` ★★
        //
        // ## 此前的问题
        //
        // 原实现 `let all = self.load_cached()?` 会**深拷贝整库**
        // （`MemoryStoreCache::snapshot()` 是 `Vec<Memory>::clone()`），
        // 然后再 `all.iter().any(...)` 线性查两个 ID。
        //
        // 而本端点的设计用途是**批量回传候选边**
        // （`max_out_per_seed × max_seeds` 可达 48 条），
        // 且调用方是逐条 HTTP 回传 ⇒ 每条边一次全库克隆 = **O(N×M)**。
        // 在真实库（4500+ 条）上会放大成明显的内存抖动与锁持有时间。
        //
        // ## 修法：用只查存在性的 `has_memory_id`（无克隆）
        //
        // 语义完全等价（都只判断 ID 是否存在），但不复制任何 `Memory`。
        // 实现见 `MemoryStoreCache::contains_id`（直接遍历缓存借用，不取副本）。
        // ★返回 `Result`：持久层不可读时**照旧向上报错**（不降级成 false——
        //   那会把"磁盘故障"伪装成"该边被规则拒绝"，两者处置完全不同）。
        if !self.has_memory_id(from_id)? || !self.has_memory_id(to_id)? {
            return Ok(false);
        }
        let Some(ref mut graph) = self.graph_store else {
            return Ok(false); // 未启用图存储：静默跳过（图是增强能力）
        };
        let before = graph.edge_count();
        graph.add_edges_batch(&[(
            from_id.to_string(),
            to_id.to_string(),
            etype,
            weight.clamp(0.0, 1.0),
        )])?;
        Ok(graph.edge_count() > before)
    }

    /// 从图存储**直读**关系边（v0.9.8，补文档 §6 #4 的读通路）
    ///
    /// # 为什么需要它（实测缺口，2026-09-17）
    ///
    /// `graph_store` 此前**只有写入方、没有读出口**：
    ///   · `/memories/association-graph` → 走 `associations_in`（内存记录层推导），
    ///     **完全不读 `graph_store`**
    ///   · `/memories/associations` → 同上
    /// ⇒ `add_external_edge` 写进去的 §5.3 逻辑关系边**无人能读出**。
    /// 实测证据：写入 `coordinate` 边后，`graph_edges.json` 里确有此边，
    /// 但 `association-graph` 的返回中看不到它（见 `temp/daoti_assoc/probe_read_gap.py`）。
    /// 本方法即补这个读出口，使 §5.4「候选命中 → 生成边 → 可检索」闭环成立。
    ///
    /// # 与 `association_graph` 的分工（互补，非替代）
    ///
    /// | | `association_graph` | 本方法 |
    /// |---|---|---|
    /// | 读什么 | 记录层**当场推导**（event_id/entities/source_ids） | 图里**已落盘**的边 |
    /// | 含 2 跳路径合成 | 是（带 `why` 可读路径） | 否（只走真实边，每跳有据） |
    /// | 含 §5.3 逻辑关系 | 否 | **是** |
    ///
    /// 两者回答不同问题："为什么这两条相关"（前者）vs "图里有哪些已确立的关系"（后者）。
    ///
    /// # 参数
    /// - `memory_id`：起点。**必须已存在**，否则返回空（不构造悬空边）。
    /// - `rel_type`：按关系名过滤（大小写皆可，走 `EdgeType::from_relation_str`）；
    ///   `None` = 不过滤。**未知类型返回空**（不兜底为"全部"——
    ///   调用方打错字时若拿到全部边，会误以为过滤生效）。
    /// - `hops`：最大跳数，**上限 3**（承 §6 #4 契约）。传 0 按 1 处理。
    ///
    /// # 返回值
    /// 按 `weight` 降序排列（同权按 `hops` 升序）。多跳边的 `weight` 已按
    /// `γ^hop` 衰减（γ=0.7，承 §4.5），故跨跳比较权重是有意义的。
    ///
    /// # 可见性
    /// 边**两端都必须是当前可见的记忆**才返回——否则图里会漏出
    /// 用户无权看到的记忆 ID（隐私红线，与 `associations_in` 的 `visible`
    /// 过滤同等严重）。任一端不可见 ⇒ 整条边不返回。
    pub fn query_stored_edges(
        &self,
        memory_id: &str,
        rel_type: Option<&str>,
        hops: usize,
        privacy: &Option<(PrivacyLevel, Option<String>, Option<String>)>,
    ) -> Result<Vec<StoredEdge>, PersistenceError> {
        // §6 #4 契约：hops ≤ 3。0 视为 1（"至少一跳"才有意义）
        let max_hops = hops.clamp(1, 3);
        let all = self.load_cached()?;
        let Some(root) = all.iter().find(|m| m.id == memory_id) else {
            return Ok(Vec::new()); // 根不存在：无图可言
        };
        // ★可见性判定必须与检索路径**同口径**（2026-09-18 审查 G1 修复）
        //
        // ## 此前的问题
        //
        // 本函数只调 `is_visible`（**仅隐私三级**），**不查 `is_expired`**。
        // 而 `Memory::is_expired` 的 TTL 语义是"过期即应消失"——
        // recall / list / stats 都已把它排除，本端点却仍能读出其 ID、
        // 关系、权重与创建时间 ⇒ **同一份数据两种可见性口径**。
        //
        // 而本函数自己的文档还写着"边**两端都必须是当前可见的记忆**才算
        // 可见"——实现与文档不一致（同 S7 的一类问题）。
        //
        // ## 为什么抽成闭包（而非两处各写一遍）
        //
        // 下面是"根 + 每个对端"两处判定。若各写一遍，将来加过滤项
        // （如新增某种可见性维度）会漏改其一 —— 承「同一套规则复用谓词，
        // 不另写一份」的既有纪律（`expand_associations` 的 `visible` 闭包
        // 就是这么写的）。
        let visible = |m: &Memory| -> bool {
            if m.is_expired() {
                return false;
            }
            is_visible(m, privacy)
        };
        if !visible(root) {
            return Ok(Vec::new());
        }
        let by_id: std::collections::HashMap<&str, &Memory> =
            all.iter().map(|m| (m.id.as_str(), m)).collect();
        let Some(graph) = self.graph_store.as_ref() else {
            return Ok(Vec::new()); // 未启用图存储：降态为空（图是增强能力）
        };

        // 过滤函数：None = 不过滤；Some(未知名) = 空（不回退为"全部"）
        let want: Option<EdgeType> = match rel_type {
            None => None,
            Some(s) => match EdgeType::from_relation_str(s) {
                Some(t) => Some(t),
                // 类型名无法识别 ⇒ 直接空结果，避免"打错字却拿到全部边"
                None => return Ok(Vec::new()),
            },
        };

        // ---- BFS（沿无向邻接遍历，但保留每条边的原始方向）----
        //
        // 邻接表按**无向**建：`query_edges` 的语义是"与此记忆相关的边"，
        // 方向不参与"能不能走到"。边的原始方向在产出时由 `edge.source_id`
        // 还原（见下），故遍历无向不会丢失方向信息。
        let mut adj: std::collections::HashMap<&str, Vec<&MemoryEdge>> =
            std::collections::HashMap::new();
        for e in graph.all_edges() {
            adj.entry(e.source_id.as_str()).or_default().push(e);
            adj.entry(e.target_id.as_str()).or_default().push(e);
        }

        let mut out: Vec<StoredEdge> = Vec::new();
        let mut seen_edges: std::collections::HashSet<&str> = std::collections::HashSet::new();
        // 已访问节点：防止在多跳里绕回，也避免同一节点被两条路径重复展开
        let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
        visited.insert(root.id.as_str());
        // 本层待展开：(节点ID, 迄今为止的路径)
        let mut frontier: Vec<(String, Vec<String>)> =
            vec![(root.id.clone(), vec![root.id.clone()])];

        // §4.5 多跳衰减系数 γ = 0.7
        const GAMMA: f32 = 0.7;

        for hop in 1..=max_hops {
            if frontier.is_empty() {
                break;
            }
            let mut next: Vec<(String, Vec<String>)> = Vec::new();
            for (cur_id, path) in &frontier {
                let Some(edges) = adj.get(cur_id.as_str()) else {
                    continue;
                };
                for e in edges.iter() {
                    // 同一条边在一轮里只产出一次（避免 A→B 与 B→A 重复）
                    if !seen_edges.insert(e.id.as_str()) {
                        continue;
                    }
                    let Some(other) = by_id.get(if e.source_id == *cur_id {
                        e.target_id.as_str()
                    } else {
                        e.source_id.as_str()
                    }) else {
                        continue; // 悬空边（对端记忆已删除）：跳过
                    };
                    // ★可见性：任一端不可见 ⇒ 整条边不返回（隐私红线）
                    //   与根同一套规则（含 is_expired，见上方闭包说明）
                    if !visible(other) {
                        continue;
                    }
                    let etype_str = e.edge_type.as_str();
                    // ★过滤只作用于**输出**，不阻断遍历：
                    //   `query(node, rel_type=coordinate, hops=2)` 的语义是
                    //   "两跳内可达的 coordinate 边"，而非"只经由 coordinate 边走"。
                    //   若在此 `continue`，长跳的匹配边会因中间边类型不符而不可达。
                    // 注：此处用 `map_or` 而非 `is_none_or`（Rust 1.82+），
                    // 项目 MSRV 为 1.80（与 memory_store.rs 内既有注释同纪律）。
                    let matched = want.as_ref().map_or(true, |t| e.edge_type == *t);
                    let mut p = path.clone();
                    p.push(other.id.clone());
                    if matched {
                        out.push(StoredEdge {
                            // 原始方向取自落盘边，与遍历方向无关
                            from: e.source_id.clone(),
                            to: e.target_id.clone(),
                            relation: etype_str.to_string(),
                            weight: e.weight * GAMMA.powi(hop as i32 - 1),
                            symmetric: e.edge_type.is_symmetric(),
                            hops: hop,
                            path: p,
                            created_at: e.created_at.clone(),
                        });
                    }
                    if visited.insert(other.id.as_str()) {
                        let mut np = path.clone();
                        np.push(other.id.clone());
                        next.push((other.id.clone(), np));
                    }
                }
            }
            frontier = next;
        }

        // 权重降序；同权按跳数升序（近的更可信）
        out.sort_by(|a, b| {
            b.weight
                .partial_cmp(&a.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.hops.cmp(&b.hops))
        });
        Ok(out)
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
    ///
    /// ## ★当前接线状态（2026-09-18 审查核实，修正过期注释）
    ///
    /// 本方法**当前无生产调用方**（仅单测调用）。原注释称"由健康检查调用"，
    /// 但实际 `server.rs` 的 `memory_stats` / `system_health` 两个 handler
    /// **都不调用它**——原因是 v0.9.1 三阶段锁解耦后，`synthesis_pending`
    /// 标记改由**后台结晶流水线**消费（见 `consolidation.rs` 的三阶段合成：
    /// 成功后置 `false`、取消/失败则保留以便重试）。
    ///
    /// ⇒ 这是**遗留兼容入口**，不是死代码：它封装了"CAS + 委托 luoshu_synthesize"
    ///   这一在同进程内触发合成的语义，`benchmark.rs` 走的 `luoshu_synthesize`
    ///   是它的无 CAS 版本。**是否删除待定**（删除会连带影响 5 个单测），
    ///   故本轮**只修正注释**，不改行为。
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
        let result = if let Some(existing) = similar.as_ref() {
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
            // ⭐ 内容被**真实替换**时必须存档旧版本（v0.9.8 §3.55 根因修复）
            //
            // **修的是什么**：此前这里直接 `merged.content = memory.content`，
            // 旧内容**当场被丢弃**——`version_history` 与 `version` 都不留痕。
            // 实测（`temp/multidim-input-measure.py`，global 4485 条）：
            // `updated_at` 变过的有 463 条（10.32%），但其中 **448 条（96.8%）
            // 没有任何版本历史** ⇒ "同一件事被更新过"这一事实**被销毁**，
            // 演进维度因此几乎无法产出（全库仅 15 条有版本历史）。
            //
            // **为什么用 `update_content` 而非手动 push**：该函数已实现
            // "存档旧内容 + 版本号 +1 + 只保留最近 5 版"，是单一事实来源
            //（承方法论 79）。手写一份会与它漂移。
            //
            // **为什么先比内容**：相似记忆合并很常见（同一条被重记一次），
            // 内容没变时不该虚增版本号——否则版本历史会被无意义的重复填满，
            // 真正有信息量的旧版本反而被挤出最近 5 版之外。
            let content_changed = merged.content != memory.content;
            if content_changed {
                merged.update_content(memory.content);
            }
            merged.tags = merged_tags;
            merged.daoti_preview_gua = memory.daoti_preview_gua;
            merged.daoti_preview_bagua = memory.daoti_preview_bagua;
            merged.daoti_preview_version = memory.daoti_preview_version;
            // **事件维度必须合并而非丢弃**（否则"共同经历"信息被静默吃掉）：
            // 新记忆带来 event_id 时以新值为准（内容已更新为最新表述）；
            // entities 取并集去重（同一次经历的两条记忆可能各提到不同实体）。
            if memory.event_id.is_some() {
                merged.event_id = memory.event_id.clone();
            }
            for e in &memory.entities {
                if !merged.entities.contains(e) {
                    merged.entities.push(e.clone());
                }
            }
            // ⭐ 来源关系同样**必须合并而非丢弃**（v0.9.8 §3.55）
            //
            // **为什么必须**：`source_ids` 是"这条记忆从哪几条衍生"的**唯一载体**，
            // 也是记录层唯一不需要用户填写的依据（实测覆盖 8.54%，全库最高）。
            // 合并时丢弃它 ⇒ 记忆一旦被合并就**永久失去来源关系**，
            // 且该损失不可恢复（原始 source_ids 无处可查）。
            //
            // **触发场景是常态而非边缘**：本次实测就是这样发现的——
            // 测试给记忆加上 `source_ids` 后 `remember` 一次，因与既有记忆
            // 相似而走了合并路径，`source_ids` 当场丢失，关联为空。
            //
            // 取**并集去重**（与 entities 同款）：新旧来源都指向真实存在的
            // 记忆，任一条都是有效证据，不应因为"新的一次写入没提"而丢弃。
            for s in &memory.source_ids {
                if !merged.source_ids.contains(s) {
                    merged.source_ids.push(s.clone());
                }
            }
            merged.touch();

            // 如果新记忆的重要性更高，则提升
            if memory.importance > merged.importance {
                merged.importance = memory.importance;
            }

            // 自动建立冲突/演进关系边（Section 3.3 冲突解决）
            //
            // ★★ 2026-09-18 审查 G4 修复：两处改动 ★★
            //
            // ## 1. 从 `?` 传播改为「显式告警 + 不阻断」
            //
            // 此前这里用 `?` 传播（`graph.add_edge(...)?`），与
            // `expand_associations` 里写图用的**静默**策略**互相矛盾**：
            // 同一种失败（图写盘失败）在一条路径上让用户操作失败、
            // 在另一条上静默 ⇒ 同仓库两套纪律，后来者无法判断哪条是规范。
            //
            // ⇒ 统一为「图是增强能力，写图失败不阻断主操作，但**必须发声**」
            //   （与 `forget` 清理边、`expand_associations` 落图的处置一致）。
            //
            // ## 2. ★写图移到**记忆落盘之后**（修一个真实的悬空边风险）
            //
            // 原顺序是「先写图 → 再 `save_memory`」。而 `Contradicts` 边先写成功后，
            // 若 `Evolves` 边失败并 `?` 返回，**`save_memory` 根本不会执行**
            // ⇒ 图里留下一条**指向不存在记忆的边**（因为合并后的记忆从未落盘）。
            // 这正是 `add_external_edge` 花大力气用 `by_id` 防御的悬空边，
            // 却被写路径**自己制造**出来。
            //
            // ⇒ 顺序改为「先落盘记忆，再写图」：记忆一定存在，边不会悬空。
            //   实现上把这批边**收集起来**，在 `save_memory` 之后统一写。
            let mut pending_edges: Vec<(EdgeType, f32)> = Vec::new();
            if self.graph_store.is_some() {
                let jaccard = self.compute_jaccard(&old_content, &merged.content);
                // 内容实质不同的合并 → Contradicts 边（需要后续解决）
                if jaccard < 0.9 {
                    // 相似但不等同 → 可能是矛盾或演进
                    pending_edges.push((EdgeType::Contradicts, jaccard));
                }
                // 内容更新 → Evolves 边
                pending_edges.push((EdgeType::Evolves, jaccard));
            }

            (merged, pending_edges)
        } else {
            // 无冲突，正常写入（无待写边）
            (memory, Vec::new())
        };
        let (mut result, pending_edges) = result;

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

        // ★★ 图边写入放在**记忆落盘之后**（2026-09-18 审查 G4 修复）
        //
        // **为什么不放在前面**：若先写边再落盘，一旦落盘失败（或中间的
        // 边写入失败并中断），图里就会留下**指向不存在记忆的边**——正是
        // `add_external_edge` 用 `by_id` 防御的悬空边。现在记忆一定已落盘，
        // 边不可能悬空。
        //
        // **失败策略**：显式告警、**不阻断**（图是增强能力；且此处记忆
        // 已经保存成功，再返回错误会让用户误以为"没记住"——那是更严重的误导）。
        if !pending_edges.is_empty() {
            if let Some(existing) = similar.as_ref() {
                if let Some(ref mut graph) = self.graph_store {
                    for (etype, weight) in &pending_edges {
                        if let Err(e) =
                            graph.add_edge(&result.id, &existing.id, etype.clone(), *weight)
                        {
                            eprintln!(
                                "[LRC-GRAPH] ⚠ 记忆已保存，但写入 {:?} 边失败（该关系将缺失）：{}",
                                etype, e
                            );
                        }
                    }
                }
            }
        }
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

        // 阶段三 b2：预判元数据参与候选剪枝（**v0.9.8 起默认关闭**，
        // LRC_DAOTI_PREVIEW_PRUNE=1 显式开启）。
        //
        // **为什么改为默认关闭**：本剪枝的第二证据是 `daoti_preview_bagua`，
        // 其取值与 `bagua_index` 同源（均来自洛书编码器 + mirror_project）。
        // §3.37 已实测该编码**不读语义**——打乱字符顺序后分类 100% 不变
        // （根因：9 维特征仅含字符密度/字符熵/位置权重），且真实语料上
        // 最大单类占比 99~100%（`data_beir_eval` 3633 条全部落入同一卦）。
        // 用一个无语义判别力的标签去"修正跨域误剪"，实质是用噪声换噪声。
        //
        // 关闭后退化为 v0.8.50 回滚后行为：仅按 LRC 自分类卦做环形距离硬剪除。
        // 注意 LRC 自分类卦本身同样不读语义（同一根因），故该硬剪除的
        // 语义有效性同样存疑——但那属于**更上游**的编码器问题（须单独立项），
        // 不在本次门控翻转的范围内，此处不擅自改动既有剪除行为。
        // LRC_DAOTI_PREVIEW_PRUNE=1 可复现 v0.9.7 行为（对照实验用）。
        let daoti_prune_enabled = daoti_preview_prune_enabled();

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

        // P7 主动发现（PREREG §3.1 D3）：只读检索不得写回任何状态，与快路径同款。
        // 深路径的写入面更广（指标、合成命中日志、状态机激活、合成预标记），
        // 被主动发现调用时会同时污染排序输入与后台任务调度，故必须同样提前返回。
        if filter.read_only {
            return Ok(RecallResult {
                memories,
                scores,
                total: total_count,
                regression_evidence: deep_evidence,
            });
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

        // P7 主动发现（PREREG §3.1 D3 零伤害承诺）：read_only 检索是"第二通道"，
        // 必须在**任何**状态写回之前返回——不写状态机、不写探索日志、不写指标。
        // 这三个写入都会改变用户查询路径上的后续排序输入（活性偏置 + 联想桥词），
        // 一旦发生，D3 的"逐字节一致"即在机制上不可能成立。
        if filter.read_only {
            return Ok(RecallResult {
                memories,
                scores,
                total: total_count,
                regression_evidence,
            });
        }

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
            // ═══ ★清理该记忆在图上的边（2026-09-18 审查 G3 修复）═══
            //
            // **修的是什么**：图此前**只增不减**——`remove_edge` / `clear`
            // 定义存在但**零生产调用**，而 forget 也不触碰图 ⇒ 已删除记忆的边
            // **永久残留**为"悬空边"。消费侧只能靠 `by_id.get()` 查不到而跳过，
            // 即每轮检索都为死边付出一次遍历+判空，成本随生命期单调增长。
            //
            // **为什么放这里（而非让调用方自己清）**：删除记忆是**唯一**使边
            // 失效的事件，把它与"清理其边"放在同一处，才不会漏（承
            // 「生命周期事件与其清理必须同处」）。
            //
            // **失败策略：显式告警但不阻断**（与 `expand_associations` 写图的
            // 静默策略一致——图是增强能力，删记忆这个主操作不该因图失败而失败）。
            // 但**不能静默**：残留边会持续占用遍历成本，用户需要知道。
            if let Some(ref mut graph) = self.graph_store {
                match graph.remove_edges_of_memory(id) {
                    Ok(n) if n > 0 => {
                        eprintln!("[LRC-GRAPH] 已随记忆删除清理 {} 条关联边（id={}）", n, id);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        // 不阻断：记忆本体已删除成功；但必须发声
                        eprintln!(
                            "[LRC-GRAPH] ⚠ 记忆已删除，但其关联边清理失败（将残留为悬空边）: {}",
                            e
                        );
                    }
                }
            }
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

    /// 按事件 ID 反查「同一次经历」产生的其他记忆
    ///
    /// **记录层 → 关联的推导**：不冗余存储"共同经历"列表，
    /// 而是以 `event_id` 为键即时分组。这样避免了冗余字段与主体不一致的风险。
    ///
    /// 返回该 event_id 下的全部记忆（含起点自身，按 created_at 升序），
    /// 便于上层判断"先后顺序"。`exclude_id` 用于排除起点。
    pub fn memories_by_event(
        &self,
        event_id: &str,
        exclude_id: Option<&str>,
    ) -> Result<Vec<Memory>, PersistenceError> {
        let all = self.load_cached()?;
        let mut hits: Vec<Memory> = all
            .into_iter()
            .filter(|m| m.event_id.as_deref() == Some(event_id))
            .filter(|m| exclude_id != Some(m.id.as_str()))
            .collect();
        hits.sort_by_key(|m| m.created_at);
        Ok(hits)
    }

    /// 列出全部事件及其记忆数（按记忆数降序）
    ///
    /// 用于观测"经历"维度的覆盖情况：有多少条记忆带 event_id、
    /// 形成了多少个经历簇、簇的规模分布如何。
    pub fn event_index(&self) -> Result<Vec<(String, usize)>, PersistenceError> {
        let all = self.load_cached()?;
        let mut cnt: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for m in &all {
            if let Some(ref e) = m.event_id {
                *cnt.entry(e.clone()).or_insert(0) += 1;
            }
        }
        let mut out: Vec<(String, usize)> = cnt.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(out)
    }

    /// 按实体名反查共享该实体的记忆
    ///
    /// 与 `memories_by_event` 互补：event_id 管"同一次经历"，
    /// entities 管"跨经历的同一对象"（如两条不同记忆都提到"爸爸"）。
    pub fn memories_by_entity(
        &self,
        name: &str,
        kind: Option<EntityKind>,
        exclude_id: Option<&str>,
    ) -> Result<Vec<Memory>, PersistenceError> {
        let all = self.load_cached()?;
        let mut hits: Vec<Memory> = all
            .into_iter()
            .filter(|m| {
                m.entities
                    .iter()
                    .any(|e| e.name == name && kind.map(|k| e.kind == k).unwrap_or(true))
            })
            .filter(|m| exclude_id != Some(m.id.as_str()))
            .collect();
        hits.sort_by_key(|m| std::cmp::Reverse(m.created_at));
        Ok(hits)
    }

    /// 统计实体的文档频率（df = 出现在多少条记忆里），**按项目分组**
    ///
    /// 键为 `(项目, 实体名, 实体类型)`；项目取自 `Memory.project`，
    /// 无 project 者归入 `_global_`（与 `list`/`stats` 的项目口径一致）。
    ///
    /// **为什么必须按项目分组**（§3.49 修正）：全库占比会被**其他项目稀释**，
    /// 导致项目内高度泛化的实体（如 LRC 的 `app.js`，项目内 43.8%）被判为"不泛化"。
    ///
    /// 与 `associations` 的匹配口径（name + kind 均相同）**严格一致**。
    /// 同一条记忆内重复出现的同一实体只计 1 次（df 语义是"文档数"而非"出现次数"）。
    fn entity_df_map(
        &self,
        all: &[Memory],
    ) -> std::collections::HashMap<((String, String), EntityKind), usize> {
        let mut df: std::collections::HashMap<((String, String), EntityKind), usize> =
            std::collections::HashMap::new();
        for m in all {
            let proj = m.project.as_deref().unwrap_or("_global_").to_string();
            // 去重：同一条记忆里同一实体只算一次
            let mut seen_in_doc: std::collections::HashSet<(String, EntityKind)> =
                std::collections::HashSet::new();
            for e in &m.entities {
                if seen_in_doc.insert((e.name.clone(), e.kind)) {
                    *df.entry(((proj.clone(), e.name.clone()), e.kind))
                        .or_insert(0) += 1;
                }
            }
        }
        df
    }

    /// 计算 hub 实体集合（按项目口径判定）
    ///
    /// 返回 `(实体名, 类型) -> (df, 该项目总记忆数)`——
    /// 保留命中时的**项目分母**，使调用方能如实展示"在哪个项目里泛化"。
    ///
    /// 判定规则：**任一项目内**满足 `df >= HUB_ENTITY_MIN_DF` 且
    /// `df / 该项目记忆数 >= HUB_ENTITY_DF_RATIO` 即视为 hub。
    fn hub_entity_set(
        &self,
        all: &[Memory],
    ) -> std::collections::HashMap<(String, EntityKind), (usize, usize)> {
        use std::collections::HashMap;
        // 各项目的记忆总数（分母）
        let mut proj_total: HashMap<String, usize> = HashMap::new();
        for m in all {
            *proj_total
                .entry(m.project.as_deref().unwrap_or("_global_").to_string())
                .or_insert(0) += 1;
        }
        let df = self.entity_df_map(all);

        let mut out: HashMap<(String, EntityKind), (usize, usize)> = HashMap::new();
        for (((proj, name), kind), c) in df {
            let total = proj_total.get(&proj).copied().unwrap_or(0);
            if is_hub_entity(c, total) {
                // 同一实体可能命中多个项目：保留 df 最大（最泛化）的那次
                let entry = out.entry((name.clone(), kind)).or_insert((c, total));
                if c > entry.0 {
                    *entry = (c, total);
                }
            }
        }
        out
    }

    /// 列出被判为 hub 的实体及其频次（**让过滤可见，而非静默**）
    ///
    /// **为什么必须提供此查询**：hub 过滤会**丢弃**部分关联。若用户不知道
    /// "哪些关联被丢了、为什么丢"，就会把过滤后的稀疏结果误读为"没有关联"——
    /// 这与此前 §3.42.5 批评过的"静默丢字段"是同一类失败。
    /// 因此过滤规则必须**可查询、可解释**。
    ///
    /// 返回 `(实体名, 类型, df, 所属项目内的记忆数)`，按 df 降序。
    /// **注意第 4 项分母是"该项目内的记忆数"**，不是全库总数（§3.49 修正）。
    pub fn hub_entities(
        &self,
    ) -> Result<Vec<(String, EntityKind, usize, usize)>, PersistenceError> {
        let all = self.load_cached()?;
        let mut out: Vec<(String, EntityKind, usize, usize)> = self
            .hub_entity_set(&all)
            .into_iter()
            .map(|((name, kind), (c, total))| (name, kind, c, total))
            .collect();
        out.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        Ok(out)
    }

    /// 记忆的关联图谱：以一条记忆为起点，推导它的多类型关联
    ///
    /// 返回 **结构化关联**（非排序列表）：每条关联带 `relation` 类型与 `why` 解释。
    /// 这是"联想"的载体——不同类型的关联并存，而非单一相似度排序。
    ///
    /// 关系类型：
    /// - `same_event` — 来自同一次经历（依据：相同 event_id）
    /// - `shared_entity` — 共享实体（依据：实体名+类型相同）
    /// - `derived_from` — 谱系衍生（依据：source_ids 互指）
    ///
    /// **hub 实体过滤（v0.9.7，PREREG §3.45.5）**：
    /// 过于泛化的实体（如项目名，出现在 70% 的记忆里）其"共享"近乎恒真，
    /// 会产生海量**零区分度的假关联**（实测 95.7% 的 shared_entity 来自两个项目名）。
    /// 此类实体在本方法中被**跳过**，其判定规则见 [`Self::hub_entities`]。
    ///
    /// **为什么过滤而非降权**：假关联比无关联更糟——它会污染用户对系统的信任
    /// （§3.45.5）。而降权只是把噪声排后，用户仍会看到它们。
    /// 被过滤的实体可通过 [`Self::hub_entities`] 显式查询（保证过滤**可见**，非静默）。
    pub fn associations(
        &self,
        memory_id: &str,
    ) -> Result<Vec<MemoryAssociation>, PersistenceError> {
        let all = self.load_cached()?;

        // 一次性统计 hub 实体（按**项目内**占比判定；见 `hub_entity_set` 与
        // `HUB_ENTITY_DF_RATIO` 的文档：全库分母会被其他项目稀释而漏判）
        let hub_set = self.hub_entity_set(&all);

        Ok(Self::associations_in(
            &all,
            memory_id,
            &hub_set,
            &Self::auto_event_map(&all),
            &ArtifactTables::build(&all),
        ))
    }

    /// 计算**自动事件**分组（v0.9.8）：`memory_id -> 同一次自动经历的其他记忆`
    ///
    /// **为什么需要它**：`event_id` 实测填写率 0%（§3.51），记录层联想因此
    /// 产出恒为 0。本函数用**客观事实**（同项目 + 同小时写入）推断"同一次经历"，
    /// 零填写负担，实测覆盖 70.7%（global）/ 90.4%（dev）。
    ///
    /// **必须做成"一次算全库"而非"逐条算"**：分组需要先按 (项目, 小时桶)
    /// 聚合并算时间跨度——逐条计算会退化为 O(N²)。故本函数一次遍历建表，
    /// 供 [`Self::associations_in`] 以 O(1) 查表。
    ///
    /// ★2026-09-18 修正悬空引用：原文指向 `Self::associations_in_with_auto`，
    /// 但该函数**全仓不存在**（已合并进 `associations_in` 的 `auto_events` 参数）
    /// ⇒ 读者按图索骥会找不到实现。改为实际消费方 `associations_in`。
    ///
    /// **⭐ 批量写入必须排斥**（见 [`AUTO_EVENT_MIN_SECS_PER_MEMORY`]）：
    /// 实测坏桶（脚本导入/测试注入）的时间戳**集中在几秒内**，桶内内容
    /// 互相无关；若纳入会产生海量假关联。
    ///
    /// **仅对 `memory_type == Experience` 生效**吗？——不。
    /// 实测候选以 `decision`(69%) / `fact`(30%) 为主（§3.51.3 抽样显示
    /// 它们确像"某次工作会话产出的结论"），故**不按类型限制**，
    /// 否则会把绝大多数真实候选排除在外。类型由调用方的 filter 决定。
    fn auto_event_map(
        all: &[Memory],
    ) -> std::collections::HashMap<String, Vec<(String, i64, String)>> {
        use std::collections::HashMap;
        /// 自动事件 ID 前缀：与手填 `event_id` **显式区分**
        ///（用户可能在 `why` 中看到，必须能分辨这是系统推断而非人工填写）
        const AUTO_PREFIX: &str = "auto:";

        // 分桶键 = (项目, 小时窗口序号)。用时间戳整除窗口得到**稳定序号**，
        // 避免依赖字符串格式化（跨时区/格式差异会导致同组被拆散）。
        let mut buckets: HashMap<(String, i64), Vec<(&Memory, i64)>> = HashMap::new();
        for m in all {
            let Some(ts) = m.created_at.timestamp_nanos_opt() else {
                continue;
            };
            let secs = ts / 1_000_000_000;
            let proj = m.project.as_deref().unwrap_or("_global_").to_string();
            let slot = secs.div_euclid(AUTO_EVENT_WINDOW_SECS);
            buckets.entry((proj, slot)).or_default().push((m, secs));
        }

        let mut out: HashMap<String, Vec<(String, i64, String)>> = HashMap::new();
        for ((proj, slot), members) in buckets {
            let count = members.len();
            if count < 2 {
                continue; // 单条构不成"共同经历"
            }
            let min_t = members.iter().map(|(_, t)| *t).min().unwrap_or(0);
            let max_t = members.iter().map(|(_, t)| *t).max().unwrap_or(0);
            let span = max_t - min_t;
            if !is_auto_event_cluster(count, span) {
                // 批量写入（脚本导入/测试注入）：时间过度集中 ⇒ 不是一次经历
                continue;
            }
            // 事件 ID 含项目与窗口序号：稳定、可复现、与真实 event_id 不冲突
            let auto_id = format!("{}{}-{}", AUTO_PREFIX, proj, slot);
            for (m, _t) in &members {
                // 记录 (对方 id, 时间戳, 自动事件 ID)，供关联推导与 why 生成
                out.entry(m.id.clone()).or_default().extend(
                    members
                        .iter()
                        .filter(|(o, _)| o.id != m.id)
                        .map(|(o, ot)| (o.id.clone(), *ot, auto_id.clone())),
                );
            }
        }
        out
    }

    /// 在**已加载的全量记忆**上推导关联（纯函数：不触 I/O、不重算 hub）
    ///
    /// **为什么要抽出来**（v0.9.8）：检索主路径需要对若干条已入选记忆批量展开
    /// 关联。若逐条调用 [`Self::associations`]，每条都会 `load_cached()` +
    /// 全库重算 hub 实体 + 全库重建自动事件表（3 次 O(N) 扫描），
    /// 在检索热路径上不可接受。抽出后主路径**只加载/统计一次**，
    /// 再对 k 条记忆复用。
    ///
    /// **为什么不是"在主路径里重新实现一遍规则"**：检索时用的关联
    /// 必须与详情页看到的关联**完全同源**，否则两处漂移——
    /// 用户会发现"检索说有关联、点进去却没有"。故主路径复用本函数。
    ///
    /// **为什么 `auto_events` 必须由调用方传入**（而非本函数内自建）：
    /// 自动事件分组是**全库统计量**（需先按 (项目,窗口) 聚合再算跨度），
    /// 放在本函数内会把 O(N) 建表塞进"每起点一次"的循环里。
    ///
    /// **为什么 `artifact_tabs` 也必须由调用方传入**（v0.9.8 同款理由）：
    /// artifact 的占比表与倒排表都是**全库统计量**（需遍历全库抽取 + 按项目聚合），
    /// 同理不能放进"每起点一次"的循环。
    fn associations_in(
        all: &[Memory],
        memory_id: &str,
        hub_set: &std::collections::HashMap<(String, EntityKind), (usize, usize)>,
        auto_events: &std::collections::HashMap<String, Vec<(String, i64, String)>>,
        artifact_tabs: &ArtifactTables,
    ) -> Vec<MemoryAssociation> {
        let Some(anchor) = all.iter().find(|m| m.id == memory_id) else {
            return Vec::new();
        };

        let mut out: Vec<MemoryAssociation> = Vec::new();

        // ① 共同经历（event_id 相同）
        if let Some(ref ev) = anchor.event_id {
            for m in all.iter() {
                if m.id != anchor.id && m.event_id.as_deref() == Some(ev.as_str()) {
                    out.push(MemoryAssociation {
                        memory_id: m.id.clone(),
                        relation: "same_event".to_string(),
                        why: format!("同一次经历（event_id={}）", ev),
                        content_preview: m.content.chars().take(120).collect(),
                    });
                }
            }
        }

        // ② 共享实体（名称+类型均相同），跳过 hub 实体
        for e in &anchor.entities {
            if hub_set.contains_key(&(e.name.clone(), e.kind)) {
                continue; // 过于泛化：共享近乎恒真，关联无区分度
            }
            for m in all.iter() {
                if m.id == anchor.id {
                    continue;
                }
                if m.entities
                    .iter()
                    .any(|x| x.name == e.name && x.kind == e.kind)
                {
                    // 去重：同一目标记忆若已由"共同经历"关联，则不再重复加入
                    if out
                        .iter()
                        .any(|a| a.memory_id == m.id && a.relation == "shared_entity")
                    {
                        continue;
                    }
                    out.push(MemoryAssociation {
                        memory_id: m.id.clone(),
                        relation: "shared_entity".to_string(),
                        why: format!(
                            "{}「{}」（{}）",
                            relation_label("shared_entity"),
                            e.name,
                            e.kind.as_str()
                        ),
                        content_preview: m.content.chars().take(120).collect(),
                    });
                }
            }
        }

        // ③ 谱系关联（source_ids）—— **双向**（v0.9.8 §3.55）
        //
        // 此前只做**单向**：仅当锚点自己有 `source_ids` 时才产出关联
        //（即只有"结晶产物"能联想到"它的来源"）。
        // 但记录本身是**双向可读**的：来源记忆同样能联想到"我被结晶成了什么"。
        // **实测定标**（`temp/multidim-input-measure.py`，global 4485 条）：
        // 单向覆盖 93 条（2.07%）→ 双向 383 条（**8.54%**，**4.1×**）。
        //
        // **为什么不复用同一个 relation 名**：方向语义不同。
        // `derived_from` = "我由它衍生"；反向 = "它被结晶成了我"。
        // 用同一个名字会让前端画成同向边（`GraphEdge.symmetric` 判定依赖
        // relation），用户会误读"谁来自谁"——承 §3.53「证据要可区分」的原则。
        if !anchor.source_ids.is_empty() {
            for m in all.iter() {
                if anchor.source_ids.contains(&m.id) {
                    out.push(MemoryAssociation {
                        memory_id: m.id.clone(),
                        relation: "derived_from".to_string(),
                        why: format!("由来源「{}」衍生", m.id),
                        content_preview: m.content.chars().take(120).collect(),
                    });
                }
            }
        }
        // 反向：哪些记忆把**锚点**当作了来源（即锚点被结晶成了它们）
        for m in all.iter() {
            if m.id == anchor.id || !m.source_ids.contains(&anchor.id) {
                continue;
            }
            // 去重：同一目标若已由其他规则关联，则不重复加入
            //（否则用户会看到同一对记忆出现两条不同依据的边）
            if out.iter().any(|a| a.memory_id == m.id) {
                continue;
            }
            out.push(MemoryAssociation {
                memory_id: m.id.clone(),
                relation: "crystallized_into".to_string(),
                why: format!(
                    "被结晶为合成记忆（该记忆由 {} 条来源融合而成）",
                    m.source_ids.len()
                ),
                content_preview: m.content.chars().take(120).collect(),
            });
        }

        // ④ 自动事件（v0.9.8）：系统按「同项目 + 同窗口」推断的「同一次经历」
        //
        // 与 ① 的**关键区别**：① 依据 `event_id` 是**知情者的断言**，
        // 本规则依据写入时间是**系统的事实统计**。二者可信度不同，
        // 故**必须分类型标注**，让用户自己判断依据强度（承 §3.53 原则）。
        if let Some(peers) = auto_events.get(&anchor.id) {
            let anchor_ts = anchor
                .created_at
                .timestamp_nanos_opt()
                .map(|n| n / 1_000_000_000)
                .unwrap_or(0);
            for (peer_id, peer_ts, auto_id) in peers {
                // 去重：同一目标若已由 ①（更强依据）关联，则不再重复加入
                if out
                    .iter()
                    .any(|a| &a.memory_id == peer_id && a.relation == "same_event")
                {
                    continue;
                }
                let Some(peer) = all.iter().find(|m| &m.id == peer_id) else {
                    continue;
                };
                // 时间间隔写进 why：这是用户**唯一能自行复核**的客观量
                //（"相隔 12 分钟"比"同一小时"信息量大得多）
                //
                // **必须区分分钟/秒**：实测一个桶内可能既有间隔数十分钟的
                // 记录，也有**同一秒**写入的两条。若统一按分钟取整，
                // 后者会显示"相隔约 0 分钟"——不仅无信息量，还掩盖了
                // "这两条可能是批量写入"这一用户本该看到的线索。
                let gap_secs = (anchor_ts - *peer_ts).abs();
                let gap_desc = if gap_secs >= 60 {
                    format!("相隔约 {} 分钟", gap_secs / 60)
                } else {
                    format!("相隔 {} 秒", gap_secs)
                };
                out.push(MemoryAssociation {
                    memory_id: peer_id.clone(),
                    relation: "same_event_auto".to_string(),
                    why: format!(
                        "{}（{}，{}）",
                        relation_label("same_event_auto"),
                        auto_id,
                        gap_desc
                    ),
                    content_preview: peer.content.chars().take(120).collect(),
                });
            }
        }

        // ⑤ 演进（v0.9.8 §3.55）：依据 `version_history`——**同一件事被更新过**
        //
        // **为什么这条规则不需要新的输入**：`version_history` 是每次修正
        // 记忆时**系统自动**存档的旧内容，不依赖用户填写、不依赖相似度猜测。
        // 它是"这条记忆确实被替换过"的**直接证据**。
        //
        // ⚠ **本轮同时修了一个根因**：写入路径合并相似记忆时，
        // 旧内容被直接覆盖、从不存档（实测 96.8% 的更新未留痕）。
        // 若不修，本条规则只在极少数"手动修正"的记忆上生效（全库 15 条）。
        //
        // **为什么不用"新旧内容相似度"判演进**：相似 ≠ 同一件事被更新。
        // "我爱吃苹果"与"我爱吃苹果派"高度相似，但并非演进关系。
        // 相似度只能产生**猜测**，而版本历史是**事实**（承 §3.53 的原则）。
        //
        // **为什么只在 `why` 中给出旧版本、不产出独立节点**：
        // 旧版本内容**不是另一条记忆**（没有自己的 ID，也不该被检索到）。
        // 若伪造一个 ID 塞进 `memory_id`，下游的"排除已在结果中""可见性过滤"
        // 都会因为查不到该 ID 而静默丢弃它——产出一条用不了的关系。
        // 故演进信息**附在锚点自身的 why 里**，由前端在展开时展示。
        if !anchor.version_history.is_empty() {
            let mut versions: Vec<String> = anchor
                .version_history
                .iter()
                .map(|v| {
                    format!(
                        "第 {} 版（{}）",
                        v.version,
                        v.updated_at.format("%Y-%m-%d %H:%M")
                    )
                })
                .collect();
            versions.sort();
            out.push(MemoryAssociation {
                memory_id: anchor.id.clone(), // 自指：表示"这是关于自身的演变线索"
                relation: "evolved_from".to_string(),
                why: format!(
                    "这条记忆被更新过 {} 次：{}",
                    versions.len(),
                    versions.join("、")
                ),
                content_preview: anchor
                    .version_history
                    .last()
                    .map(|v| v.content.chars().take(120).collect())
                    .unwrap_or_default(),
            });
        }

        // ⑥ 共享产物标识符（v0.9.8，PREREG_MEMORY_ASSOCIATION.md）
        //
        // **为什么需要这条规则**：② `shared_entity` 依赖人工填 `entities`，
        // 实测填写率 **0.00%** ⇒ 在真实库上**产出恒为 0**。本规则是它的
        // **零填写负担客观替代路径**（§3.54.8 方法论 110 的规范要求）。
        //
        // **与 ② 的关系**：**同一关系语义**（跨经历引用同一具体对象），
        // 但**证据来源不同** —— ② 是**知情者断言**（人填），
        // 本条是**系统形态检出**（从正文识别产物标识符）。
        // 依 §3.53「证据要可区分」原则，**必须独立命名**，让用户判断依据强度
        //（与 `same_event` / `same_event_auto` 的拆分同构）。
        //
        // **为什么它是"形态检出"而非"语义推断"**：见 [`HUB_ARTIFACT_DF_RATIO`]
        // 的文档——「`app.js` 是文件名」只由字符本身决定，不需要"对世界做判断"，
        // 属 §3.44.5 明文允许的「格式化/校验」类。
        //
        // **不回写 `entities` 字段**：本维度**只产出关联、不修改记忆**——
        // 一旦回写，用户就无法分辨"哪些实体是 AI 填的、哪些是系统检出的"，
        // 知情者断言的证据强度会被系统检出污染（这是本轮刻意守住的红线）。
        for art in extract_artifacts(&anchor.content) {
            if artifact_tabs.is_skipped(&art) {
                // hub（过泛化 ⇒ 共享近乎恒真 ⇒ 零区分度）
                // 或成员不足 2 条（无关联对可言）
                continue;
            }
            let Some(ids) = artifact_tabs.members.get(&art) else {
                continue;
            };
            let (df, total) = artifact_tabs.ratio.get(&art).copied().unwrap_or((0, 0));
            for other in ids {
                if other == &anchor.id {
                    continue;
                }
                let Some(peer) = all.iter().find(|m| &m.id == other) else {
                    continue;
                };
                // 去重：同一目标若已由更强的"人工实体"关联（②），不再重复加入
                if out
                    .iter()
                    .any(|a| a.memory_id == *other && a.relation == "shared_entity")
                {
                    continue;
                }
                out.push(MemoryAssociation {
                    memory_id: other.clone(),
                    relation: "shared_artifact".to_string(),
                    // `why` 必须含**具体产物名**（承方法论 105：解释要解释到
                    // 可核验的具体对象），并按 §3.53 标注**证据来源**
                    //（"系统识别" 而非"你填的"），让用户能自行判断可信度。
                    // 括号内给出该产物在库中的覆盖度（df/项目内总数），
                    // 这是用户**唯一能自行复核"它是否过于泛化"**的客观量。
                    why: format!(
                        "{}（系统识别「{}」，库内 {} 条出现过 / 该项目共 {} 条）",
                        relation_label("shared_artifact"),
                        art,
                        df,
                        total
                    ),
                    content_preview: peer.content.chars().take(120).collect(),
                });
            }
        }

        out
    }

    /// 为**一批检索结果**计算「为什么这条会出现」的可复核理由（v0.9.8）
    ///
    /// # 为什么需要它（用户裁定 + 实测）
    ///
    /// 用户对道体的定位裁定（逐字）：「道体不是检索器、不是排序器、不是分类器，
    /// 是**关系与规则的推理引擎**」「道体要做的不是"匹配"，是**解释关联**」。
    /// 实测（`temp/assoc-gua-necessity7.py`）：元数据作**召回器**时相对 BGE 冗余
    /// （关系对落 BGE top-20 的比例 16.0~44.0%，随机仅 1.6%），作**解释器**时
    /// 覆盖率 60.8% vs 随机 8.1%（**7.5×**）⇒ 价值在解释，不在召回。
    ///
    /// 但生产上**主检索结果从不带理由**：搜索结果卡片只显示通路标签
    /// （"快速 + 深度 · 贡献 0.0164"），而 [`Self::associations_in`] 里
    /// 精心写好的 `why`（含具体产物名、覆盖度、时间间隔）只在详情页/联想分区出现。
    /// 本方法把记录层理由接到**检索出口**。
    ///
    /// # 判据来源（已实测，避免造出低价值机制）
    ///
    /// **不用「对查询的理由」**：实测（`temp/assoc-why-hitrate.py`，真实库 1294 条）
    /// 真实短查询的实词 token 中位仅 **2 个**，87% 的结果都是"词面命中查询词"
    /// ⇒ 显示"这条含你搜的词"是**同义反复**（用户本来就知道），零重合（纯语义）
    /// 仅占 0~2.6% ⇒ 该口径**不足以支撑功能**。
    ///
    /// **用「与结果集内兄弟的关系」**：实测（`temp/assoc-sibling-coverage.py`）
    /// 结果集内至少有一条兄弟关系的结果占 **68.1%（关键词型）/ 59.3%（短查询）**
    /// ⇒ 可挂在多数卡片上。且这正是用户举例的原话
    /// （「"同项目"、"共享 memory_store.rs"、"同标签 daoti 且间隔3小时"」）。
    ///
    /// # 边界（刻意守住的红线）
    ///
    /// 1. **只产出理由，不改排序**：返回 `id -> why` 映射供展示，
    ///    调用方不得据此调分（与 §3.48「联想不并入排序」同纪律）。
    /// 2. **不做自动推断**：理由**全部来自已有记录**（`project` / 正文形态 /
    ///    时间 / `event_id` / `source_ids`），不引入任何新的语义判断
    ///    （承 §3.44.5 与六钥匙判据：形态问题可自动，语义问题留知情者）。
    /// 3. **证据强度必须可区分**：不同来源的理由前缀不同
    ///    （「同项目」vs「系统识别」vs「同一次经历」），用户据此判断可信度
    ///    （承 §3.53）。
    /// 4. **不静默**：无理由的记忆不出现在返回映射中，调用方据此展示空态；
    ///    绝不编造一个"弱理由"填满每个位置。
    ///
    /// # 参数
    /// - `results`：本次检索的结果集（**顺序即展示顺序**，理由按此顺序取最优）
    /// - 返回：`记忆 ID -> 人类可读理由`（仅含**确有理由**的条目）
    pub fn result_reasons(
        &self,
        results: &[Memory],
    ) -> Result<std::collections::HashMap<String, String>, PersistenceError> {
        if results.len() < 2 {
            // 单条结果没有"兄弟关系"可言（同一次经历也需要至少两条才算共同经历）
            return Ok(std::collections::HashMap::new());
        }
        let mut out: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let artifacts: Vec<Vec<String>> = results
            .iter()
            .map(|m| extract_artifacts(&m.content))
            .collect();

        for (i, anchor) in results.iter().enumerate() {
            let mut best: Option<(u8, String)> = None;
            for (j, peer) in results.iter().enumerate() {
                if i == j {
                    continue;
                }
                if let Some((prio, why)) =
                    Self::pair_reason(anchor, peer, &artifacts[i], &artifacts[j])
                {
                    // 取**证据最强**（prio 最小）的一条作为卡片理由
                    match &best {
                        Some((bp, _)) if *bp <= prio => {}
                        _ => best = Some((prio, why)),
                    }
                }
            }
            if let Some((_, why)) = best {
                out.insert(anchor.id.clone(), why);
            }
        }
        Ok(out)
    }

    /// 判定**两条记忆之间**的记录层理由（供 [`Self::result_reasons`] 调用）
    ///
    /// 返回 `(优先级, 人类可读理由)`；无理由返回 `None`。
    /// 优先级口径与 [`Self::relation_priority`] 同源（数字越小证据越强），
    /// **不新造一套分级**——否则两处漂移后用户看到的强弱顺序会自相矛盾。
    fn pair_reason(
        a: &Memory,
        b: &Memory,
        art_a: &[String],
        art_b: &[String],
    ) -> Option<(u8, String)> {
        // ① 同一次经历（知情者断言，证据最强）
        if let (Some(ea), Some(eb)) = (a.event_id.as_deref(), b.event_id.as_deref()) {
            if !ea.is_empty() && ea == eb {
                return Some((0, format!("与结果内另一条同属一次经历（event_id={ea}）")));
            }
        }
        // ② 谱系互指（source_ids 是直接证据，非推断）
        if a.source_ids.contains(&b.id) {
            return Some((1, "结果内另一条是它的结晶来源".to_string()));
        }
        if b.source_ids.contains(&a.id) {
            return Some((1, "结果内另一条由它结晶而来".to_string()));
        }
        // ③ 共享具体产物（形态检出：由字符本身决定，不涉及语义判断）
        //    必须给出**具体产物名**——只写"共享产物"不可复核（承方法论 105）
        if let Some(shared) = art_a.iter().find(|x| art_b.contains(x)) {
            return Some((
                3,
                format!("与结果内另一条共享「{shared}」（系统从正文识别）"),
            ));
        }
        // ④ 同项目 + 同一时段（统计推断，证据最弱 ⇒ 必须标明这是系统推断）
        if let (Some(pa), Some(pb)) = (a.project.as_deref(), b.project.as_deref()) {
            if !pa.is_empty() && pa == pb {
                if let (Some(ta), Some(tb)) = (
                    a.created_at.timestamp_nanos_opt(),
                    b.created_at.timestamp_nanos_opt(),
                ) {
                    let gap = (ta - tb).abs() / 1_000_000_000;
                    if gap < AUTO_EVENT_WINDOW_SECS {
                        let desc = if gap >= 60 {
                            format!("相隔约 {} 分钟", gap / 60)
                        } else {
                            format!("相隔 {gap} 秒")
                        };
                        return Some((
                            4,
                            format!("与结果内另一条同属「{pa}」的同一时段（{desc}）"),
                        ));
                    }
                }
                return Some((5, format!("与结果内另一条同属项目「{pa}」")));
            }
        }
        None
    }

    /// 由检索结果**联想补全**：找出「本次没召回、但由记录层必然关联」的记忆（v0.9.8）
    ///
    /// # 这一步补的是什么（为什么它是"真正的联想"）
    ///
    /// 在此之前，本项目所有检索通路（fast / deep / RRF）**全部是相似度驱动的**——
    /// 它们只能在"语义相近"的记忆里找。而用户要的能力是：
    /// 查「游西湖」时，把同一次杭州之行的「吃楼外楼」也带出来，
    /// 哪怕两句话**没有一个共同词、语义也不相似**。
    /// 这类连接**不可能**由相似度产生，只能由**记录**（`event_id` / `entities`）
    /// 推导——这就是记录层存在的意义，也是本方法存在的意义。
    ///
    /// **在此之前关联推导已经实现，但只挂在详情页接口上**：
    /// 用户必须"先点开某条记忆"才看得到关联，检索结果本身从不带联想。
    /// 本方法把同一套记录层规则（复用 [`Self::associations_in`]，不重新实现）
    /// 接入检索出口。
    ///
    /// # 设计约束（每条都有理由）
    ///
    /// 1. **不并入排序，单独分区返回**：联想结果的证据性质不同
    ///    （记录必然成立 vs 相似度打分），混排会让调用方误以为二者可比。
    ///    且现有 fast/deep/RRF 的排序质量有大量 A/B 证据支撑，
    ///    不应被一个新通道改变（PREREG §3.48 同理）。
    /// 2. **★必须复用候选可见性规则**：联想是"绕过查询词"直接取记忆，
    ///    若不做隐私/项目/类型过滤，就会成为**绕过权限的后门**——
    ///    这是安全红线，不是体验问题。故本方法显式复用与检索
    ///    相同的过滤谓词（`is_visible` + 类型/项目/标签/重要性）。
    /// 3. **只对已召回的记忆展开**（不扩张检索根集）：联想的价值是
    ///    "把没召回到的补上"，而非"再检一遍"。以已召回的 top-k 为起点，
    ///    既保证联想与本次查询**语义相关**（起点回应了查询），
    ///    又把展开成本限定在 k 次关联查询。
    /// 4. **`read_only` 检索不展开**：P7 主动发现的"零伤害承诺"
    ///    （PREREG §3.1 D3）要求只读检索与基线逐字节一致，
    ///    任何额外产出都必须跳过。
    /// 5. **每条补入记忆附带 `via_*` 溯源**：用户必须能看到
    ///    "它是因为哪条记忆被带上来的"，否则无法判断这个联想是否合理
    ///    （承方法论 105：解释必须解释到具体对象）。
    ///
    /// # 参数
    /// - `seed_ids`：已召回记忆的 ID（联想起点）
    /// - `exclude`：需要排除的"已在结果中"的 ID 集合
    /// - `filter`：**复用检索的过滤条件**（隐私/项目/类型/标签/重要性）
    /// - `max_out`：补入上限（防大簇把输出撑爆）
    ///
    /// # 为什么是 `&mut self`（v0.9.8 改）
    ///
    /// 本方法现在会把产出的关系**写入图存储**（见函数尾部），故需可变借用。
    /// 三个生产调用点均已持有 `&mut MemoryStore`（server 的 recall 在
    /// `spawn_blocking` 内持锁、`run_association_explore` 形参即 `&mut`），
    /// 故此次签名变更不引入新的锁竞争。
    pub fn expand_associations(
        &mut self,
        seed_ids: &[String],
        filter: &RecallFilter,
        max_out: usize,
    ) -> Result<Vec<AssociatedMemory>, PersistenceError> {
        if seed_ids.is_empty() || max_out == 0 {
            return Ok(Vec::new());
        }
        // P7 只读检索必须与基线逐字节一致 ⇒ 不做任何联想补全
        if filter.read_only {
            return Ok(Vec::new());
        }

        let all = self.load_cached()?;

        // 已在结果中的记忆：不再作为联想补入（否则是重复，不是联想）
        let exclude: std::collections::HashSet<&str> =
            seed_ids.iter().map(|s| s.as_str()).collect();

        // ═══ ★★ 为符号层边**预留席位**（2026-09-18 审查 G5a 修复）★★ ═══
        //
        // ## 实测结论（2026-09-18 真实库副本测量，11 组样本；结论已内联于下）
        //
        // 配额（`max_out`，默认 3）原先的分配顺序是：
        //   记录层 1 跳 → 记录层 2 跳 → **符号层落盘边**（最后）。
        //
        // 真实库副本实测（11 组样本）：
        //
        //   | 记录层 1 跳产出 | 样本 | 符号层读回 | 读回率 |
        //   |---|---|---|---|
        //   | ≥3 条（配额吃满） | 10 | 0 | **0%** |
        //   | <3 条（配额有余） | 1 | 1 | **100%** |
        //
        // ⇒ **完美分离**（排除"边没加载/可见性/种子不匹配"等替代解释），
        //   且**记录层 ≥3 条的概率 = 10/11 = 91%**
        // ⇒ 符号层边在真实 recall 中**可读回率 ≈ 0%**，即"接了等于没接"。
        //
        // ## 原注释的假设被实测否定
        //
        // `memory_store.rs` 原注释称「只有配额有余（**真实 recall 的常见情形**）
        // 才出现 2 跳」——实测"有余"仅 **9%**（1/11），与"常见"相反。
        //
        // ## 修法：条件式预留（关键在"条件式"）
        //
        // 若种子节点在图上**确有**可读的边（记录层或符号层），
        // 则把**记录层**可用配额压到 `max_out - 1`，给落盘边留 ≥1 席。
        //
        // ## 修法：为符号层边预留席位（关键在"条件式"）
        //
        // 若种子节点在图上**确有符号层边**，则把**记录层**可用配额压到
        // `max_out - 1`，给那些边留 ≥1 席。
        //
        // **为什么必须"确有边"才预留**：若无收益而仍压缩记录层配额，
        // 就是**白白少产出 1 条**——拿既有能力换一个空的承诺。
        // 故只在真有待读的边时才压缩，其余情形与改动前**逐字节一致**
        // ⇒ 既有配额/轮转/2 跳测试不受影响。
        //
        // ## ★★ 为什么只看**符号层**边（而非"记录层 ∪ 符号层"）
        //
        // 这一点由 `test_symbolic_edges_get_reserved_seat_under_quota_pressure`
        // 实测逼出：初版把"记录层类型"也算作"有待读的边"，结果
        // **并入段把 `same_event` 从图里又读回来一条**，反而挤掉了符号层边。
        //
        // 根因是一个**冗余**：
        //   · 图里的**记录层边**全部是 `expand_associations` 自己写的
        //     （S3 修复后，外部写入已只允许符号层类型）
        //   · 而 `associations_in` **每次都从记忆数据重新算出**同样的关系
        //   ⇒ 把记录层边从图里读回来，是**纯冗余**（同一关系出现两次来源）
        //
        // 而符号层边是**唯一**记录层产出不了的（`cause`/`temporal`/
        // `constraint`/`facilitate`/`coordinate` 由结构算子推导，
        // 由图外的道体服务经 `/external-edge` 写入）
        // ⇒ 并入段只需负责读这一类。
        // ═══ ★预留判据执行位置：见下方 `visible` 闭包**之后** ═══
        //
        // （2026-09-18 审查修复：原实现在此处计算 `has_symbolic_edges`，
        //  但此处 `by_id` / `visible` 尚未就绪 ⇒ 判据只能看出"一端是种子
        //  且是符号层边"，比并入段实际的门槛**宽**，会造成"预留了席位却
        //  并入 0 条"⇒ 净损失 1 条记录层联想。现移到判据可复用的位置。）

        // hub 实体一次性统计（与详情页关联同口径，见 `hub_entity_set`）
        let hub_set = self.hub_entity_set(&all);
        // 自动事件表一次性统计（同口径）：**必须提到循环外**，
        // 否则每条起点都要重建一次全库分组（O(N) × k）
        let auto_events = Self::auto_event_map(&all);
        // artifact 两张表一次性统计（同口径，v0.9.8）：同上，
        // 建表需遍历全库做形态抽取 + 按项目聚合，逐起点重建是 O(N × k)
        let artifact_tabs = ArtifactTables::build(&all);
        // 起点记忆索引（用于填 via_* 溯源字段）
        let by_id: std::collections::HashMap<&str, &Memory> =
            all.iter().map(|m| (m.id.as_str(), m)).collect();

        // ★ 可见性判定：与检索路径**同一套规则**（复用谓词，非另写一份）
        // 顺序与 RecallFilter 字段一一对应，便于核对是否漏项。
        let visible = |m: &Memory| -> bool {
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
            // ★隐私：联想不得成为绕过权限的后门
            if !is_visible(m, &filter.privacy_context) {
                return false;
            }
            true
        };

        // ═══ ★★ 符号层落盘边的席位：**后置让位**（2026-09-18 审查修复）★★ ═══
        //
        // ## 为什么不预压缩记录层配额（前两版都错在这里）
        //
        // v1（原始）：判据只要求"一端是种子 ∧ 符号层类型" ⇒ 比并入段实际门槛宽
        //   ⇒ 会出现"预留了席位却并入 0 条"⇒ 净损失 1 条记录层联想。
        // v2（本审查初版）：试图让预留判据复用并入段的四道门槛
        //   （`exclude` / `seen` / `by_id` / `visible`）。
        //   **实测失败**（由 `test_unreadable_symbolic_edge_must_not_reserve_seat`
        //   当场抓出）：其中 **`seen` 在预留决策时根本不存在**——它是记录层
        //   边跑边填的集合，而预留发生在记录层**之前** ⇒ 逻辑上不可能复用
        //   （循环依赖）。于是"对端会被记录层先取走"这一破绽无法预先排除。
        //
        // ## 修法：把决策**推到事后**（后置让位）
        //
        // 不再预先压缩配额，改为：
        //   ① 记录层按 `max_out` 正常产出（行为与改动前**逐字节一致**）
        //   ② 并入段把可读的符号层边收进**独立缓冲** `sym_merged`
        //   ③ 全部并入完成后，才按 `max_out` 上限让位：每并入 1 条，
        //      从 `out` **尾部**弹掉 1 条（此时 `out` 只含记录层条目
        //      ⇒ 弹掉的一定是记录层条目）
        //
        // ## 为什么这版三条不变量同时成立
        //
        //   · **无收益必无损失**：并入 0 条 ⇒ 让位 0 次 ⇒ 记录层拿满配额
        //     （不再有"空的承诺"）
        //   · **有收益才让位**：`seen`/`by_id`/`visible` 的真实结果在并入时
        //     已知，无需预估 ⇒ 不会空占席位
        //   · **不饿死记录层**：`max_out == 1` 时记录层照常产出 1 条
        //     （v1 的 `record_cap = 0` 饿死问题连带消失）
        //
        // ## 附带收益：去掉一次 O(E) 全图扫描
        //
        // 旧实现在函数开头无条件扫全图判断 `has_symbolic_edges`，
        // 而探索路径会按节点重复调用本方法（深度≤4 × 宽度≤3）
        // ⇒ 放大为 O(深度 × 宽度 × E)。后置让位不需要这次预扫描。

        let mut out: Vec<AssociatedMemory> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        // ★ 多维度交错取用（v0.9.8 §3.55）
        //
        // # 解决两个**不同**的"某一来源刷满"问题
        //
        // **问题一（上一轮已修）**：单个**起点**吃光配额。
        // 真实库实测：联想输出 100% 是自动事件、单桶占比 88%——
        // 第一个种子的同小时桶把 max_out 全占了，其余种子一条不露。
        //
        // **问题二（本轮修）**：单个**关系类型**吃光配额。
        // 即使按起点均分，若某起点只有 `same_event_auto` 一种候选
        //（实测这正是常态：该类型覆盖 70%，其余类型覆盖个位数百分比），
        // 那么输出仍然是"一种关系的堆砌"，用户看不到"联想可以从不同
        // 角度发生"——而多维度正是本次要交付的能力。
        //
        // # 做法：把"通道"定义为 (起点, 关系类型)，在通道间轮转
        //
        // 这样**同时**保证两件事：
        // - 每轮每个起点都会推进 ⇒ per-seed 公平（上一轮的保证不退化）
        // - 每轮每个关系的每种类型都会推进 ⇒ **多维度覆盖**
        //
        // **通道内按「意外性优先」排序**（v0.9.8，本轮核心）——
        // 不是按证据强度。实测（`temp/assoc-bge-giveup.py`）发现：
        // 联想产出的对，**BGE 排名中位仅 0.0685（top 6.9%）**，
        // 即**大部分联想对 BGE 本来就能找到**，那部分没有增量。
        // 而用户对联想的价值判据（§3.43.9 逐字）是
        // 「**产出的关联图中，有多少条是 BGE 给不出的**」。
        //
        // ⇒ 把「BGE 给不出」的候选排在通道前面。判据用实词 Jaccard 代理
        //（见 [`content_words`] 文档：AUC 0.9275 识别"给不出"，且与 BGE
        // 低相似对的**重叠仅 0.081** ⇒ 是独立信息，非 BGE 的粗粒度版）。
        //
        // **为什么不是"过滤掉 BGE 能给的"**：那会丢弃 85% 产出
        //（实测 proxy<0.02 只留 386/2615 对）。本设计只改**顺序**，配额仍由
        // 轮转决定 ⇒ "先给意外、后给常规"，不减少总量，也不引入新阈值
        //（承 §3.49.5：无断层则不设阈值）。
        //
        // **轮转的代价已实测**（`temp/assoc-quota-calibrate.py`，真实库 251 桶）：
        // 每起点取 K 条时，联想总量保留 K=1→43% / K=2→68% / **K=3→81%**。
        // 本实现用"轮转"而非固定 K：等价于 K = max_out/通道数（动态），
        // 因此**不需要新阈值**——配额天然按通道数均分。
        let per_seed: Vec<(String, String, Vec<Vec<MemoryAssociation>>)> = seed_ids
            .iter()
            .filter_map(|seed| {
                let seed_mem = by_id.get(seed.as_str())?;
                let assocs =
                    Self::associations_in(&all, seed, &hub_set, &auto_events, &artifact_tabs);
                // 按关系类型分组成通道
                let seed_words = content_words(&seed_mem.content);
                let mut by_rel: Vec<(u8, String, Vec<MemoryAssociation>)> = Vec::new();
                for a in assocs {
                    let prio = Self::relation_priority(&a.relation);
                    match by_rel.iter_mut().find(|(_, r, _)| *r == a.relation) {
                        Some((_, _, v)) => v.push(a),
                        None => by_rel.push((prio, a.relation.clone(), vec![a])),
                    }
                }
                // 组内按"意外性"降序（代理相似度越低 ⇒ 越意外 ⇒ 越靠前）。
                // 用 `total_cmp` 保证全序（`partial_cmp` 对 NaN 会退化为 Equal，
                // 使排序结果依赖输入顺序 ⇒ 不可复现）。
                for (_, _, v) in by_rel.iter_mut() {
                    v.sort_by(|x, y| {
                        let wx = content_words(&x.content_preview);
                        let wy = content_words(&y.content_preview);
                        let sx = word_jaccard(&seed_words, &wx);
                        let sy = word_jaccard(&seed_words, &wy);
                        sx.total_cmp(&sy)
                    });
                }
                by_rel.sort_by(|x, y| x.0.cmp(&y.0).then_with(|| x.1.cmp(&y.1)));
                Some((
                    seed.clone(),
                    seed_mem.content.chars().take(60).collect::<String>(),
                    by_rel.into_iter().map(|(_, _, v)| v).collect(),
                ))
            })
            .collect();

        // 最大轮数 = 任一通道的最长长度（通道长度参差，短的跳过即可）
        let max_rounds = per_seed
            .iter()
            .flat_map(|(_, _, channels)| channels.iter().map(|c| c.len()))
            .max()
            .unwrap_or(0);
        'rounds: for round in 0..max_rounds {
            for (seed, seed_preview, channels) in &per_seed {
                for channel in channels {
                    // ★记录层按 `max_out` **正常**产出（不预压缩配额）：
                    //   预压缩会造成"预留了席位却并入 0 条" ⇒ 净损失 1 条。
                    //   符号层边的席位改为**后置让位**（见函数尾部并入段），
                    //   行为与改动前逐字节一致（见上方说明）。
                    if out.len() >= max_out {
                        break 'rounds;
                    }
                    let Some(a) = channel.get(round) else {
                        continue;
                    };
                    // 排除：已在结果中的、已补入过的、自身
                    if exclude.contains(a.memory_id.as_str())
                        || a.memory_id == *seed
                        || !seen.insert(a.memory_id.clone())
                    {
                        continue;
                    }
                    // ★可见性过滤：不可见的记忆绝不因"有关联"而被带出
                    let Some(target) = by_id.get(a.memory_id.as_str()) else {
                        continue;
                    };
                    if !visible(target) {
                        continue;
                    }
                    out.push(AssociatedMemory {
                        memory_id: a.memory_id.clone(),
                        content_preview: a.content_preview.clone(),
                        memory_type: target.memory_type.as_str().to_string(),
                        relation: a.relation.clone(),
                        why: a.why.clone(),
                        via_memory_id: seed.clone(),
                        via_preview: seed_preview.clone(),
                        // ★1 跳 = 直接记录关联（起点 → 此记忆）
                        hops: 1,
                        path: vec![seed.clone(), a.memory_id.clone()],
                    });
                }
            }
        }

        // ═══ 第二层：间接关联（2 跳，v0.9.8 §5.4 / 用户指引"第几层"）═══
        //
        // # 为什么必须补这一层（2026-09-17）
        //
        // 用户对联想的价值判据是「**告诉我这是第几层能想到的**」。
        // 但 `expand_associations` 此前**只有 1 跳**，`hops` 恒为 1，
        // 而 recall 是联想最常用的出口 ⇒ 用户永远看不到"层次"。
        //
        // 同一件事在别处已实现（口径不一致）：
        //   · `association_graph`（联想中心）：2 跳，带 `path`/`hops`/`via`
        //   · `query_stored_edges`（stored-edges 读端）：1~3 跳，带 `hops`/`path`
        //   · **本方法（recall 出口）：1 跳，无层次** ← 补的就是这个缺口
        // 且 `association_graph` 的文档明确写着「多跳推理是"**推理**"而非"匹配"」
        // ⇒ recall 出口缺这一层，等于最常用的路径上没有"推理"。
        //
        // # 为什么只做 2 跳（与 `association_graph` 同口径，非新阈值）
        //
        // 承 `association_graph` 的既有纪律：3 跳及以上会迅速膨胀，且
        // 「A 与 B 同经历、B 与 C 同经历」**推不出**「A 与 C 同经历」
        // ⇒ 2 跳是"信息量与可靠性"的平衡点。此处**不引入新参数**。
        //
        // # 为什么放在 1 跳轮转**之后**（保守优先）
        //
        // 既有 1 跳轮转有 12 个单测锁定（配额公平、多维覆盖、意外性优先……）。
        // 若把 2 跳混进轮转，会改变这些测试的输出顺序与构成。
        // ⇒ 设计为「1 跳先用配额，**配额有余时**才追加 2 跳」：
        //    · 配额被 1 跳吃满时，行为与改动前**逐字节一致**
        //    · 只有配额有余（真实 recall 的常见情形）才出现 2 跳
        //
        // # 依据的来源
        //
        // 中间节点取自**本次已补入的 1 跳结果**（`out` 的前若干条），
        // 而不是全库任意节点 ⇒ 路径的每一段都由记录保证成立，
        // 且中间节点必然**可见**（已在 `out` 里过了一道 `visible`）。
        //
        // 每个中间节点连**它自己的起点与首跳关系**一起取出，
        // 这样 `why` 与 `path` 都能写出完整两段，不留空占位。
        if out.len() < max_out {
            let mid_candidates: Vec<(String, String, String, String, String)> = out
                .iter()
                .take(INDIRECT_MID_MAX)
                .map(|a| {
                    (
                        a.memory_id.clone(),
                        a.content_preview.clone(),
                        a.via_memory_id.clone(),
                        a.via_preview.clone(),
                        a.relation.clone(),
                    )
                })
                .collect();

            'outer: for (mid_id, mid_preview, seed_id, seed_preview, first_rel) in &mid_candidates {
                if out.len() >= max_out {
                    break;
                }
                let Some(mid_mem) = by_id.get(mid_id.as_str()) else {
                    continue;
                };
                // 从中间节点再走一步（复用同一套记录层规则，不重写）
                let step2 =
                    Self::associations_in(&all, mid_id, &hub_set, &auto_events, &artifact_tabs);
                // 组内按「意外性优先」排序（与 1 跳同口径：BGE 给不出的先给）
                let mid_words = content_words(&mid_mem.content);
                let mut step2_sorted = step2;
                step2_sorted.sort_by(|x, y| {
                    let sx = word_jaccard(&mid_words, &content_words(&x.content_preview));
                    let sy = word_jaccard(&mid_words, &content_words(&y.content_preview));
                    sx.total_cmp(&sy)
                });
                for b in step2_sorted {
                    if out.len() >= max_out {
                        break 'outer;
                    }
                    // 排除：已在召回结果中 / 已补入 / 回到中间节点自身
                    if exclude.contains(b.memory_id.as_str())
                        || b.memory_id == *mid_id
                        || !seen.insert(b.memory_id.clone())
                    {
                        continue;
                    }
                    let Some(target) = by_id.get(b.memory_id.as_str()) else {
                        continue;
                    };
                    if !visible(target) {
                        continue;
                    }
                    out.push(AssociatedMemory {
                        memory_id: b.memory_id.clone(),
                        content_preview: b.content_preview.clone(),
                        memory_type: target.memory_type.as_str().to_string(),
                        // 关系名标为 `indirect`：与 1 跳的"记录直接成立"**显式区分**
                        relation: "indirect".to_string(),
                        // why 写出**两段的真实依据**（承 §3.49.2：必须含具体对象名，
                        // 只写"间接关联"而不写经由什么 => 不可解释）
                        why: format!(
                            "间接关联（2 跳）：由「{}」—{}→ 「{}」—{}→ 此记忆",
                            seed_preview.chars().take(30).collect::<String>(),
                            relation_label(first_rel),
                            mid_preview.chars().take(30).collect::<String>(),
                            relation_label(&b.relation),
                        ),
                        via_memory_id: mid_id.clone(),
                        via_preview: mid_preview.clone(),
                        hops: 2,
                        // 路径：起点 → 中间 → 终点（用户据此判断这个跳跃是否合理）
                        path: vec![seed_id.clone(), mid_id.clone(), b.memory_id.clone()],
                    });
                }
            }
        }

        // ═══ 记录层关系图化（v0.9.8）═══
        //
        // **为什么在这里写**：`out` 是本次检索**实际采用**的关联（已过可见性
        // 过滤、已按配额截断），正是用户能看到的那部分关系。把它落图，
        // 图的内容与用户看到的内容一致；若改用 `associations_in` 的全量产出，
        // 图里会含**用户无权看到**的边（隐私红线，与 `visible` 过滤同等严重）。
        //
        // **为什么失败静默**：图是增强能力，写图失败（磁盘满/权限）不应
        // 让检索本身失败——联想是附加价值，主结果必须照常返回。
        //
        // **注意对称关系去重**：`expand_associations` 从每个 seed 出发产出，
        // A→B 与 B→A 都会出现；`add_edges_batch` 内按 ID 字典序规范化，
        // 故这里直接传原样即可，无需在此判重。
        //
        // ═══ ★★ 2026-09-18 审查断链 #2 修复：只写**记录层**边 ★★ ═══
        //
        // ## 此前的问题（两处，都不报错）
        //
        // ① **符号层边被冗余写回**：`out` 现在也含符号层边（并入段从图里读出来的），
        //    而它们**本来就在图里** ⇒ 写回是无用功。后果虽轻（`add_edges_batch`
        //    按 (source,target,type) 去重，不会重复计数、也不覆盖权重），
        //    但让"图化"的来源统计含混：读进来的边又被当成"本次产出"写一遍。
        //
        // ② **`evolved_from` 落图永不可达**：它是**自指**关系
        //    （`memory_id == anchor.id`，见 `associations_in` ⑤），
        //    故此处构造出的边两端相同 ⇒ 在 `add_edges_batch` 里撞上
        //    `if s == t { continue }` 被**静默丢弃**。
        //    即：每次 recall 都白构造一条边、传下去、无声丢掉。
        //
        // ## 修法
        //
        // ① 用**白名单** `is_record_edge_type` 而非黑名单 `!is_symbolic`：
        //    本段的语义定义就是"**记录层**关系图化"（见上方小节标题），
        //    白名单让实现与注释对齐（承 S3 的修法原则）。
        //    新增符号层类型时忘加白名单不会漏写记录层边——
        //    因为符号层边**本就不该**由本段写入（它有自己的写入口
        //    `/v1/memories/external-edge`）。
        // ② 显式跳过自指，并**说明为什么**——避免下一位读者以为漏了 `evolved_from`。
        if let Some(ref mut graph) = self.graph_store {
            let mut pending: Vec<(String, String, EdgeType, f32)> = Vec::with_capacity(out.len());
            for a in &out {
                // ★只写记录层：符号层边已在图中（本段是从图里读出来的）
                if !is_record_edge_type(&a.relation) {
                    continue;
                }
                // ★自指关系（`evolved_from`）无法在图里表达：图的边是"两端之间的关系"，
                //   而它是"这条记忆自身被更新过"——信息在 `version_history` 里，
                //   已由 `associations_in` 直接渲染进 `why`，不依赖图。
                //   ⇒ 显式跳过（而非靠 `add_edges_batch` 静默丢弃）。
                if a.via_memory_id == a.memory_id {
                    continue;
                }
                let Some(etype) = EdgeType::from_relation_str(&a.relation) else {
                    // 未知关系名不落图：宁缺勿错（把未知关系硬塞进已知类型
                    // 会让用户读到错误的关系语义）
                    continue;
                };
                // 权重取自证据强度：priority 越小证据越强 ⇒ 权重越大。
                // 与 `relation_priority` 同源，避免两处各定一套强度口径。
                let prio = Self::relation_priority(&a.relation) as f32;
                pending.push((
                    a.via_memory_id.clone(),
                    a.memory_id.clone(),
                    etype,
                    1.0 / (1.0 + prio),
                ));
            }
            if !pending.is_empty() {
                let _ = graph.add_edges_batch(&pending);
            }
        }

        // ═══ ★符号层边并入（v0.9.8，2026-09-18）═══
        //
        // **修的是什么**：上面那段只把记录层关系**写**进图，但本次检索
        // **从不读**图。实测取证（`grep query_stored_edges src/server.rs`
        // → 零匹配）：符号层（§5.3 逻辑关系 cause/temporal/constraint/
        // facilitate/coordinate）经 `/v1/memories/external-edge` 落图后，
        // **再也没有任何路径能把它们带回检索结果**——写进去就沉底了。
        //
        // ⇒ 这使 §5.4 的闭环（候选命中 → 生成边 → **可检索**）缺最后一环。
        //   本节补上：把种子节点在图上的既有边读出来并入 `out`。
        //
        // **为什么放在写图之后**：先写后读，本次产出的记录层边也能
        // 在**同一次检索**里被读到（否则新边要等下次检索才可见）。
        //
        // **为什么必须过滤可见性**：图里可能有用户无权看到的记忆 ID
        //（边由外部服务写入，不经过检索的 visible 过滤）。任一端不可见
        // ⇒ 整条边不并入，与 `query_stored_edges` 内部的隐私红线同口径。
        //
        // ═══ ★★ 为符号层边**预留席位**（2026-09-18 审查 G5a 修复）★★ ═══
        //
        // ## 实测结论（2026-09-18 真实库副本测量，11 组样本；结论已内联于下）
        //
        // 配额（`max_out`，默认 3）的分配顺序是：记录层 1 跳 → 2 跳 → 符号层。
        // 实测（真实库副本，11 组样本）：
        //
        //   | 记录层 1 跳产出 | 样本 | 符号层读回 | 读回率 |
        //   |---|---|---|---|
        //   | ≥3 条（配额吃满） | 10 | 0 | **0%** |
        //   | <3 条（配额有余） | 1 | 1 | **100%** |
        //
        // ⇒ **完美分离**，且**记录层 ≥3 条的概率 = 10/11 = 91%**
        // ⇒ 符号层边在真实 recall 中**可读回率 ≈ 0%**，即"接了等于没接"。
        //
        // 而代码原来的假设（`memory_store.rs:5553` 注释）
        // 「只有配额有余（**真实 recall 的常见情形**）才出现」**被实测否定**：
        // 配额有余仅 **9%**（1/11），与"常见"相反。
        //
        // ## 修法：为符号层预留席位（v1 的预留版本已被 v3 替换）
        //
        // ★v3（2026-09-18 审查修复）：**后置让位**。本段不再受"记录层已压缩
        //   配额"的恩惠（记录层现在拿满 `max_out`），而是把可读的符号层边先收进
        //   `sym_merged` 缓冲，全部并入完成后，再按 `max_out` 上限从 `out`
        //   **尾部**（纯记录层条目）等量让位。
        //   ⇒ 并入 0 条则让位 0 次（无收益必无损失）；有并入才让位（不空占席位）。
        let mut sym_merged: Vec<AssociatedMemory> = Vec::new();
        if let Some(graph) = self.graph_store.as_ref() {
            // 只取种子节点**直接相邻**的边：多跳留给下次检索，
            // 避免一次检索把所有可达节点都拉进来（CP 成本 + 淹没主结果）。
            let seed_set: std::collections::HashSet<&str> =
                seed_ids.iter().map(|s| s.as_str()).collect();
            for e in graph.all_edges() {
                // ★v3：本段不再受 `out` 剩余配额限制（席位由**后置让位**保证）。
                //   但仍设上限 `max_out`，防一次并入过多把输出撑爆。
                if sym_merged.len() >= max_out {
                    break;
                }
                // 边必须**一端是种子**、另一端是**其他记忆**
                let (from_id, to_id) = if seed_set.contains(e.source_id.as_str()) {
                    (e.source_id.as_str(), e.target_id.as_str())
                } else if seed_set.contains(e.target_id.as_str()) {
                    (e.target_id.as_str(), e.source_id.as_str())
                } else {
                    continue;
                };
                let rel = e.edge_type.as_str();
                // ═══ ★证据性质过滤（2026-09-18 修 S4）═══
                //
                // ## 此前的问题
                //
                // 本段此前**只按"一端是种子"筛选，不看边类型** ⇒ 把图里
                // **系统推断**的边（`contradicts` / `evolves` /
                // `synthesizes_from` / `related_to`）也当作"记录型关联"
                // 输出。而渲染分区冠名是「**由记录推导**、非语义相似」，
                // 用户会把推断当成**记录必然成立的事实**。
                //
                // 实测证据：生产样本 `graph_edges.json` 里 14 条边**全部**是
                // `evolves`(12) + `synthesizes_from`(2)，无一条外部/记录层边
                // ⇒ 该分区在实际数据上会被纯推断边占满。
                //
                // ## ★★ 顺序至关重要：类型过滤必须**先于** `seen` 去重
                //
                // 初版把本过滤放在 `seen.insert` **之后**，结果实测失败：
                //   第一条是 `evolves`（应被过滤）→ 但它已把 `to_id` 插进
                //   `seen` ⇒ 随后同一目标的 `cause`（应被并入）撞上
                //   `!seen.insert(...)` 被 **误丢** ⇒ 联想结果全空。
                //
                // ⇒ 这类"被过滤的边**占用**了去重名额"是隐蔽的顺序 bug：
                //   它只在"同一目标上既有应过滤边、又有应保留边"时暴露，
                //   单看代码不易发现（本次由 `test_expand_associations_
                //   excludes_system_inferred_edges` 实测抓出）。
                //
                // ## 为什么必须排除（而非"标注一下就行"）
                //
                // `graph_store.rs` 已按来源把边分三组，并注明第一组
                // 「语义是**系统推断**（可能错）」。把可能错的推断放进
                // "由记录必然关联"的分区，正是该文件明令禁止的混同
                //（承「证据要可区分」）。
                //
                // ⇒ 只并入**符号层**（结构推导）边；记录层边由
                //   `associations_in` 当场算出，从图里再读一遍是纯冗余
                //   （且会挤占并入选段留给符号层的席位）。
                //   图存储内生四类（`evolves`/`synthesizes_from`/…）
                //   语义是"系统推断（可能错）"，也不得并入
                //   （`graph_store.rs` 明令禁止混同）。
                if !is_symbolic_edge_type(rel) {
                    continue;
                }
                // 已在召回结果中 / 已由记录层补入 ⇒ 不重复
                //
                // ★注意：本检查必须在类型过滤**之后**（见上），否则被过滤的
                //   边会占用 `seen` 名额，把同目标的合法边挤掉。
                if exclude.contains(to_id) || !seen.insert(to_id.to_string()) {
                    continue;
                }
                let Some(target) = by_id.get(to_id) else {
                    continue; // 悬空边（另一端已删除）：跳过，不构造假记忆
                };
                if !visible(target) {
                    continue; // 隐私红线：任一端不可见 ⇒ 整条边不并入
                }
                let from_preview = by_id
                    .get(from_id)
                    .map(|m| m.content.chars().take(120).collect::<String>())
                    .unwrap_or_default();
                // ★来源标注（承「证据要可区分」）：
                //   本段只并入符号层边 ⇒ 恒为"结构推导"（可能不成立），
                //   与记录层的"记录事实"性质不同，必须在 why 里写清，
                //   否则用户无法判断该不该采信。
                let origin = "符号层推导边";
                // ★v3：收进**独立缓冲**而非直接进 `out`。
                //   理由见本段开头：席位由"后置让位"保证，
                //   若直接进 `out` 则需先腾位，又会退回"预压缩"的老问题。
                sym_merged.push(AssociatedMemory {
                    memory_id: to_id.to_string(),
                    content_preview: target.content.chars().take(120).collect(),
                    memory_type: target.memory_type.as_str().to_string(),
                    relation: rel.to_string(),
                    // why 必须含**具体证据**（承 §3.49.2）：写清这是
                    // 图上的**落盘边**及其权重，而非"看起来相关"
                    why: format!(
                        "图存储既有边（{}，{}，权重 {:.2}）：由「{}」经此关系连到本记忆",
                        origin,
                        relation_label(rel),
                        e.weight,
                        from_preview.chars().take(30).collect::<String>(),
                    ),
                    via_memory_id: from_id.to_string(),
                    via_preview: from_preview,
                    hops: 1,
                    path: vec![from_id.to_string(), to_id.to_string()],
                });
            }
        }

        // ═══ ★★ 后置让位（2026-09-18 审查修复，v3）★★ ═══
        //
        // ## 位置至关重要：必须在并入段**全部完成之后**
        //
        // 此时 `out` 只含**记录层**条目（符号层边都收在 `sym_merged` 里），
        // 故从尾部弹掉的**一定是记录层条目**——不会误弹符号层。
        //
        // ## 让位规则
        //
        //   `out` 已有 L 条，并入 S 条，上限 M：
        //     · S == 0 ⇒ 不让位（**无收益必无损失**：记录层拿满，与改动前一致）
        //     · S > 0  ⇒ 从尾部弹 `min(S, L)` 条，再把 S 条符号层边接上，
        //                 总量仍 ≤ M
        //
        // ## 为什么不担心"让位后记录层变少"
        //
        // 让位**只在符号层确有产出时**发生，即"拿 1 条记录层换取 1 条符号层"，
        // 是**等价交换**而非净损失。反之并入 0 条时一次都不让——
        // 这恰是 v1/v2 反复出错的地方（预先压缩无法知道并入能否成功）。
        //
        // ## 顺序
        //
        // 让位后符号层条目接在 `out` 尾部：记录层在前、符号层在后。
        // 排序理由与 `association_evidence_rank` 无关——两者证据性质不同，
        // 详情由渲染层按来源分区呈现（见 `append_associated_memories`）。
        if !sym_merged.is_empty() {
            let keep = max_out.saturating_sub(sym_merged.len());
            out.truncate(keep);
            out.extend(sym_merged);
        }

        Ok(out)
    }

    /// 关系类型的**证据强度**排序键（越小越强），用于同类型内/通道间的出场顺序
    ///
    /// **为什么需要它**：配额有限时，"先出哪一条"直接影响用户对系统可信度的
    /// 判断。若让最弱的一类（统计推断）先占位，用户第一眼看到的联想就最可疑。
    ///
    /// 强度分层的依据（承 §3.53「证据要可区分」的原则）：
    /// - **知情者断言**（人明确说过）：`same_event` — 最强
    /// - **系统确定性记录**（产生于真实操作，不是猜测）：
    ///   `derived_from` / `crystallized_into` / `evolved_from`
    /// - **系统实体匹配**（客观共现，但 hub 过滤后仍有噪声）：`shared_entity`
    /// - **系统形态检出**（v0.9.8 新增）：`shared_artifact` —— 与 `shared_entity`
    ///   同属"共享具体对象"，证据强度略低于人工填写（人填时可确认"这是同一个对象"，
    ///   形态检出只能确认"两处字符串相同"）
    /// - **统计推断**（只看时间，同小时可能含两段无关工作）：`same_event_auto`
    fn relation_priority(relation: &str) -> u8 {
        match relation {
            "same_event" => 0,
            "derived_from" | "crystallized_into" | "evolved_from" => 1,
            "shared_entity" => 2,
            // 形态检出：与 shared_entity 同关系、证据强度略低 ⇒ 紧随其后
            "shared_artifact" => 3,
            "same_event_auto" => 4,
            _ => 5,
        }
    }

    /// 构造以某条记忆为起点的**关联图**（联想的结构化形态）
    ///
    /// 与 [`Self::associations`] 的区别：
    /// - `associations` 返回**一列边**（扁平）
    /// - 本方法返回**图**（节点 + 边 + 路径），并做 **2 跳结构传递**
    ///
    /// **多跳推理是"推理"而非"匹配"**（用户指引 §七）：
    /// 用户给的例子是「A 因果 B，B 与 C 共享情境 ⇒ A 与 C 间接关联」。
    /// 这里的多跳**不是**"猜两条记忆语义相关"，而是**图上的路径合成**——
    /// 每一步都由记录（event_id / entities）保证成立，路径本身即是解释。
    /// 因此它产出的关联是**必然成立**的，不是相似度推测。
    ///
    /// **为什么只做 2 跳**：3 跳及以上会迅速膨胀（实测典型同经历簇规模 2~8），
    /// 且路径越长，"同一次经历"的传递越弱（A 与 B 同经历、B 与 C 同经历
    /// 不能推出 A 与 C 同经历）。故 2 跳是**信息量与可靠性**的平衡点，
    /// 并显式标注 `hops`，让用户知道这是间接关联。
    ///
    /// **一个必须知道的结构事实（§3.48.5，已构造性验证）**：
    /// `same_event` 是**等价关系**（自反/对称/传递），因此"2 跳同为 same_event"
    /// 必然退化为"1 跳 same_event"，其目标**必已在直接邻居中而被排除**
    /// ⇒ `same_event → same_event` 型间接边**恒为 0**（数学必然，与数据无关）。
    /// **推论**：2 跳的产出**只能来自交叉路径**（含 `shared_entity` / `derived_from`）。
    /// 换言之，当前记录层能推出的"意外关联"，实质是
    /// "经由同一实体（如某文件）把两次不同经历连起来"，
    /// **而非**用户设想的"因果传递"——后者需要记录**因果/时序**关系，
    /// 属记录层的后续扩展（§3.48.6）。
    ///
    /// # 参数
    /// - `memory_id`：根节点
    /// - `max_nodes`：节点数上限（防大簇爆图）；超出时 `truncated = true`
    pub fn association_graph(
        &self,
        memory_id: &str,
        max_nodes: usize,
    ) -> Result<AssociationGraph, PersistenceError> {
        let all = self.load_cached()?;
        let by_id: std::collections::HashMap<&str, &Memory> =
            all.iter().map(|m| (m.id.as_str(), m)).collect();

        let empty = |root: &str| AssociationGraph {
            root: root.to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
            direct_count: 0,
            indirect_count: 0,
            truncated: false,
        };

        if !by_id.contains_key(memory_id) {
            return Ok(empty(memory_id));
        }

        // 复用关联推导的**纯函数**：它的边语义（含 hub 过滤）已在本方法之外
        // 被验证过，此处不复刻规则，避免两处实现漂移（承方法论 79：单一事实来源）。
        //
        // **为什么不再逐节点调 `self.associations()`**（v0.9.8 修正）：
        // 该路径第二层要对**每个直接邻居**取一次关联，而 `associations()`
        // 每次都做 `load_cached()` + 全库 hub 统计 + 全库自动事件分组。
        // 加入自动事件后，这里从"2 次全库扫描/邻居"恶化到"3 次"，
        // 大簇场景（实测同经历簇可达 20+ 节点）会明显变慢。
        // 本方法已持有 `all`，故两张表**在全图构建期间只算一次**。
        let hub_set = self.hub_entity_set(&all);
        let auto_events = Self::auto_event_map(&all);
        // artifact 两张表：全图构建期间只算一次（同 hub / auto_events 的理由）
        let artifact_tabs = ArtifactTables::build(&all);
        let anchor_edges =
            Self::associations_in(&all, memory_id, &hub_set, &auto_events, &artifact_tabs);

        let mut edges: Vec<GraphEdge> = Vec::new();
        let mut node_ids: Vec<String> = vec![memory_id.to_string()];
        let mut seen_nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
        seen_nodes.insert(memory_id.to_string());
        // 已加入的直接边目标（避免重复的间接边指向同一节点）
        let mut direct_targets: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // 去重键：(from, to, relation)
        let mut edge_keys: std::collections::HashSet<(String, String, String)> =
            std::collections::HashSet::new();

        let mut truncated = false;

        // ---- 第一层：直接边（hops = 1）----
        for a in &anchor_edges {
            if !seen_nodes.insert(a.memory_id.clone()) {
                // 节点已在图中（可能由不同类型的边重复指向）：边仍要加
            } else {
                if node_ids.len() >= max_nodes {
                    // 节点预算耗尽：**明确标记截断**，不静默丢弃（方法论 100）
                    truncated = true;
                    break;
                }
                node_ids.push(a.memory_id.clone());
            }
            direct_targets.insert(a.memory_id.clone());
            let key = (
                memory_id.to_string(),
                a.memory_id.clone(),
                a.relation.clone(),
            );
            if edge_keys.insert(key) {
                edges.push(GraphEdge {
                    from: memory_id.to_string(),
                    to: a.memory_id.clone(),
                    relation: a.relation.clone(),
                    why: a.why.clone(),
                    hops: 1,
                    // **对称白名单**（而非"非 derived_from 即对称"的黑名单）：
                    // 方向语义明确的关系（derived_from / crystallized_into）
                    // 必须画成有向，否则用户误读"谁来自谁"。
                    // 用白名单的原因：未来新增关系类型若忘记分类，默认按
                    // **非对称**处理（多画一个箭头）比默认对称（丢掉方向信息）
                    // 更安全——方向信息丢失是静默错误，箭头多余是可见的。
                    symmetric: matches!(
                        a.relation.as_str(),
                        "same_event" | "same_event_auto" | "shared_entity" | "shared_artifact"
                    ),
                    path: vec![memory_id.to_string(), a.memory_id.clone()],
                    via: None,
                });
            }
        }

        // ---- 第二层：间接边（hops = 2，图上的路径合成）----
        // 对每个直接邻居，取其关联；若指向的节点尚未与根相连，则形成间接边。
        // **排除回到根自身**（否则会产出"A→B→A"这种零信息的自环）。
        let direct_targets_snapshot: Vec<String> = direct_targets.iter().cloned().collect();
        for mid in &direct_targets_snapshot {
            if truncated {
                break;
            }
            let mid_edges =
                Self::associations_in(&all, mid, &hub_set, &auto_events, &artifact_tabs);
            for a2 in mid_edges {
                if a2.memory_id == memory_id || a2.memory_id == *mid {
                    continue; // 回到根 or 自环：无信息量
                }
                // 已达直接关联的节点，不再作为间接目标（直接边更有解释力）
                if direct_targets.contains(&a2.memory_id) {
                    continue;
                }
                if !seen_nodes.contains(&a2.memory_id) {
                    if node_ids.len() >= max_nodes {
                        truncated = true;
                        break;
                    }
                    seen_nodes.insert(a2.memory_id.clone());
                    node_ids.push(a2.memory_id.clone());
                }
                let key = (
                    memory_id.to_string(),
                    a2.memory_id.clone(),
                    "indirect".to_string(),
                );
                if !edge_keys.insert(key) {
                    continue; // 已有指向同一节点的间接边
                }
                // 找出中间节点与根的关系，用于写出可读路径。
                // **必须带上两段的 `why`（含具体实体名），而非只写关系类型**：
                // 间接边是最需要解释的一类（用户看不出两条无关记忆为何相连），
                // 若只写"共享实体"而不写"共享的是 app.js"，
                // 恰好违背「人类可解释」判据（PREREG §3.49.2）。
                let (first_leg, first_why) = edges
                    .iter()
                    .find(|e| e.to == *mid && e.hops == 1)
                    .map(|e| (e.relation.clone(), e.why.clone()))
                    .unwrap_or_else(|| ("related".to_string(), "相关联".to_string()));
                edges.push(GraphEdge {
                    from: memory_id.to_string(),
                    to: a2.memory_id.clone(),
                    relation: "indirect".to_string(),
                    why: format!("间接关联：根 —{}→ 中间记忆 —{}→ 此记忆", first_why, a2.why),
                    hops: 2,
                    // 间接边是路径合成，方向仅表示书写顺序，不表示因果
                    symmetric: true,
                    path: vec![memory_id.to_string(), mid.clone(), a2.memory_id.clone()],
                    // 标注本段的关系类型，便于前端按强弱路径分组展示
                    via: Some(format!("{} → {}", first_leg, a2.relation)),
                });
            }
        }

        // ---- 组装节点 ----
        let nodes: Vec<GraphNode> = node_ids
            .iter()
            .filter_map(|nid| by_id.get(nid.as_str()).map(|m| (*m).clone()))
            .map(|m| GraphNode {
                memory_id: m.id.clone(),
                content_preview: m.content.chars().take(120).collect(),
                memory_type: m.memory_type.as_str().to_string(),
                event_id: m.event_id.clone(),
                entities: m
                    .entities
                    .iter()
                    .map(|e| format!("{}:{}", e.kind.as_str(), e.name))
                    .collect(),
            })
            .collect();

        let direct_count = edges.iter().filter(|e| e.hops == 1).count();
        let indirect_count = edges.iter().filter(|e| e.hops >= 2).count();

        Ok(AssociationGraph {
            root: memory_id.to_string(),
            nodes,
            edges,
            direct_count,
            indirect_count,
            truncated,
        })
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

            // 记录层覆盖度（v0.9.7）：让「共同经历」的积累可观测
            if m.event_id.is_some() {
                stats.with_event_count += 1;
            }
            if !m.entities.is_empty() {
                stats.with_entity_count += 1;
            }
            if m.memory_type == crate::memory_types::MemoryType::Experience {
                stats.experience_count += 1;
            }
        }

        // 事件簇数：不同 event_id 的个数（与 event_index 的分组口径一致）
        {
            let mut seen = std::collections::HashSet::new();
            for m in &all {
                if let Some(ref e) = m.event_id {
                    seen.insert(e.as_str());
                }
            }
            stats.event_cluster_count = seen.len();
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
