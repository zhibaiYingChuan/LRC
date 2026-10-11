//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现「记忆写入的单一真源」，属于公开层 (Layer 1)。
//! ============================================================
//!
//! ## 为什么必须有这个模块（结构性修复，不是抽象洁癖）
//!
//! 修复前，「写一条记忆」这件事**没有单一真源**，由此产生两类真实缺陷：
//!
//! 1. **入参字段集按端点各自定义**：`/v1/memories/consolidate` 有 `user_id` 但没有
//!    `event_id`/`entities`，而 `/v1/memories/remember` 反过来 ⇒ 调用方被迫**二选一**；
//!    并且两者对「源时间」用了**不同的字段名与类型**（毫秒整数 vs RFC3339 字符串）。
//!    实测后果：记忆库里 `event_id`/`entities` 全为 0。
//! 2. **合并语义散落在 `MemoryStore::remember` 的内联实现里**：`created_at`（源时间回填）
//!    与归属字段的合并规则**没有明确定义** ⇒ 新字段接入时极易在"命中相似记忆合并"
//!    这条路径上被**静默丢弃**（而 `event_id`/`entities` 却有合并逻辑，形成内部不一致）。
//!
//! 本模块把这两件事各收敛到**唯一一处**：所有写入入口共用 [`MemoryWriteInput`]，
//! 所有合并路径共用 [`apply_merge`]。新增可写字段时只需改这里
//! ⇒ 结构上不可能再"漏一路径"。

use crate::memory_types::{EventEntity, Importance, Memory, MemoryType, PrivacyLevel};
use chrono::{DateTime, Utc};

/// 所有记忆写入入口共用的**统一输入**。
///
/// 字段与 [`Memory`] 的可写入子集一一对应。**新增可写字段时只需在此处加一个字段**，
/// 并在 [`MemoryWriteInput::into_memory`] 与 [`apply_merge`] 中各自处理一次。
#[derive(Debug, Clone, Default)]
pub struct MemoryWriteInput {
    /// 记忆内容
    pub content: String,
    /// 记忆类型（缺省 `Fact`）
    pub memory_type: Option<MemoryType>,
    /// 重要性 1-10（缺省 5）
    pub importance: Option<Importance>,
    /// 存活天数（`None` = 永久，不参与过期回收）
    pub ttl_days: Option<u32>,
    /// 关联项目
    pub project: Option<String>,
    /// 标签
    pub tags: Vec<String>,
    /// 隐私级别（缺省 `User`）
    pub privacy_level: Option<PrivacyLevel>,
    /// 来源会话 ID
    pub session_id: Option<String>,
    /// 来源用户 ID
    pub user_id: Option<String>,
    /// 事件 ID（「共同经历」关联的唯一载体）
    pub event_id: Option<String>,
    /// 事件实体（人/地/时/物）
    pub entities: Vec<EventEntity>,
    /// **源时间**：该记忆在来源系统中**真实发生**的时间（缺省 = 摄入时刻）
    pub source_timestamp: Option<DateTime<Utc>>,
    /// 道体预判：卦（由上游编码器提供时透传；缺省 `None`）
    pub daoti_preview_gua: Option<String>,
    /// 道体预判：八卦
    pub daoti_preview_bagua: Option<String>,
    /// 道体预判：版本
    pub daoti_preview_version: Option<String>,
}

impl MemoryWriteInput {
    /// 创建仅带内容的输入（其余走缺省）
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            ..Default::default()
        }
    }

    /// 转为一条新 [`Memory`]（**写入单一真源**）
    ///
    /// **v0.9.11（G6 / CR-19）**：`user_id` 在此处经
    /// [`crate::privacy::namespace_owner`] 规范化 —— 多租户域（已设 `LRC_API_TOKEN`）
    /// 下自动写成 `"{租户}::{user}"`，使**凭据**成为隔离边界；
    /// 本地可信域（桌面端 / 比赛口径）**逐字节原样**，零行为变更。
    /// 放在此处而非各调用点：所有写入入口（HTTP ×2 + MCP ×2）共用本转换
    /// ⇒ 不可能漏掉某条写入路径。
    pub fn into_memory(self) -> Memory {
        let owner =
            crate::privacy::namespace_owner(crate::privacy::from_env(), self.user_id.as_deref());
        let mut m = Memory::new(
            self.content,
            self.memory_type.unwrap_or(MemoryType::Fact),
            self.project,
            self.tags,
            self.importance.unwrap_or_else(|| Importance::new(5)),
            self.ttl_days,
        )
        .with_privacy(
            self.privacy_level.unwrap_or_default(),
            self.session_id,
            owner,
        )
        .with_event(self.event_id)
        .with_entities(self.entities);
        // 道体预判：上游编码器提供时透传（缺省 `None` ⇒ 零行为变更）
        m.daoti_preview_gua = self.daoti_preview_gua;
        m.daoti_preview_bagua = self.daoti_preview_bagua;
        m.daoti_preview_version = self.daoti_preview_version;
        if let Some(ts) = self.source_timestamp {
            m = m.with_created_at(ts);
        }
        m
    }
}

/// Unix 毫秒 → `DateTime<Utc>`（**源时间的统一解析入口**，所有写入端点共用）。
///
/// 返回 `None` 表示"未提供 / 无法解析"，调用方应保持"摄入时刻"
/// ⇒ **缺省零行为变更**。
pub fn ts_from_millis(ms: i64) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp_millis(ms)
}

/// 合并结果的附加信息（供调用方做图边判定等；不参与字段合并本身）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeOutcome {
    /// 内容是否被**真实替换**（用于 `Contradicts` / `Evolves` 边判定）
    pub content_changed: bool,
}

/// 隐私级别的**严格度秩**（数值越大越严格）
fn privacy_rank(level: PrivacyLevel) -> u8 {
    match level {
        PrivacyLevel::Session => 2,
        PrivacyLevel::User => 1,
        PrivacyLevel::Global => 0,
    }
}

/// 把 `incoming` 的字段按**统一规则**合并进已存在的 `existing`（**合并单一真源**）。
///
/// ## 规则表（三分法：取新值 / 取并集 / 取更严格或更早）
///
/// | 字段 | 规则 | 理由 |
/// |---|---|---|
/// | `content` | 内容**真变**才 `update_content`（存档旧版本） | 同一条被重记时不得虚增版本号 |
/// | `tags` / `entities` / `source_ids` | **并集去重** | 新旧都指向真实存在的事实，不应因"这次没提"而丢弃 |
/// | `event_id` | 入参有值 → **取新值** | 内容已更新为最新表述，事件归属随之更新 |
/// | `importance` | **取较大者** | 重要性只升不降（降级属显式操作） |
/// | `daoti_preview_*` | **取新值** | 编码派生字段，跟随最新一次编码 |
/// | `session_id` / `user_id` | **只补全，不改写** | 归属变更应由显式操作完成，不能作为"相似合并"的副作用 |
/// | `privacy_level` | **取更严格者** | 合并**不得扩大**可见范围（fail-closed 方向） |
/// | `created_at` | **取更早者**（`min`） | 见下 |
///
/// ## 为什么 `created_at` 取更早者（决策 1）
/// `created_at` 承载**事件发生时间**，`updated_at` 承载**处理时间**，二者职责正交
/// （既有 `update_content` 也只动 `updated_at`，不动 `created_at`）。
/// 取更早者同时满足三种情形，且**幂等**：
/// - 旧值是"摄入时刻"、新值是真源时间 ⇒ **修正**；
/// - 两者都是真源时间 ⇒ **保留首次发生**；
/// - 入参未提供源时间（其 `created_at` 即"此刻"，必不早于旧值）⇒ `min` 自然保留旧值 ⇒ **不变**。
pub fn apply_merge(incoming: &Memory, existing: &mut Memory) -> MergeOutcome {
    // ① 内容：真变才存档版本（与既有行为一致）
    let content_changed = existing.content != incoming.content;
    if content_changed {
        existing.update_content(incoming.content.clone());
    }
    // ②/③/④ 并集去重
    push_unique(&mut existing.tags, &incoming.tags);
    push_unique(&mut existing.entities, &incoming.entities);
    push_unique(&mut existing.source_ids, &incoming.source_ids);
    // ⑤ 事件 ID：取新值（仅当入参确有值）
    if incoming.event_id.is_some() {
        existing.event_id = incoming.event_id.clone();
    }
    // ⑥ 重要性：取较大者
    if incoming.importance > existing.importance {
        existing.importance = incoming.importance;
    }
    // ⑦ 编码派生字段：取新值
    existing.daoti_preview_gua = incoming.daoti_preview_gua.clone();
    existing.daoti_preview_bagua = incoming.daoti_preview_bagua.clone();
    existing.daoti_preview_version = incoming.daoti_preview_version.clone();
    // ⑧ 归属：只补全，不改写
    if existing.session_id.is_none() {
        existing.session_id = incoming.session_id.clone();
    }
    if existing.user_id.is_none() {
        existing.user_id = incoming.user_id.clone();
    }
    // ⑨ 隐私：取更严格者（合并不得扩权）
    if privacy_rank(incoming.privacy_level) > privacy_rank(existing.privacy_level) {
        existing.privacy_level = incoming.privacy_level;
    }
    // ⑩ 源时间：取更早者（决策 1）
    if incoming.created_at < existing.created_at {
        existing.created_at = incoming.created_at;
    }
    MergeOutcome { content_changed }
}

/// 把 `src` 中**尚不存在**的元素追加到 `dst`（并集去重，保持既有顺序）
fn push_unique<T: PartialEq + Clone>(dst: &mut Vec<T>, src: &[T]) {
    for item in src {
        if !dst.contains(item) {
            dst.push(item.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_types::EntityKind;

    fn mem(content: &str) -> Memory {
        Memory::new(
            content.to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        )
    }

    /// 写入单一真源：`into_memory()` 必须把**每一个**字段都落到位
    #[test]
    fn into_memory_applies_all_fields() {
        let ts = ts_from_millis(1_780_000_000_000).expect("合法毫秒");
        let input = MemoryWriteInput {
            content: "和爸妈去西湖".into(),
            memory_type: Some(MemoryType::Experience),
            importance: Some(Importance::new(8)),
            ttl_days: Some(30),
            project: Some("aml:aaa".into()),
            tags: vec!["trip".into()],
            privacy_level: Some(PrivacyLevel::User),
            session_id: Some("s1".into()),
            user_id: Some("u1".into()),
            event_id: Some("req-1".into()),
            entities: vec![EventEntity {
                name: "西湖".into(),
                kind: EntityKind::Place,
            }],
            source_timestamp: Some(ts),
            daoti_preview_gua: Some("乾".into()),
            daoti_preview_bagua: Some("乾为天".into()),
            daoti_preview_version: Some("pv1".into()),
        };
        let m = input.into_memory();
        assert_eq!(m.content, "和爸妈去西湖");
        assert_eq!(m.memory_type, MemoryType::Experience);
        assert_eq!(m.importance.value(), 8);
        assert_eq!(m.ttl_days, Some(30), "ttl_days 必须透传");
        assert_eq!(m.project.as_deref(), Some("aml:aaa"));
        assert_eq!(m.tags, vec!["trip".to_string()]);
        assert_eq!(m.session_id.as_deref(), Some("s1"));
        assert_eq!(m.user_id.as_deref(), Some("u1"));
        assert_eq!(m.event_id.as_deref(), Some("req-1"));
        assert_eq!(m.entities.len(), 1);
        assert_eq!(m.entities[0].kind, EntityKind::Place);
        assert_eq!(m.daoti_preview_gua.as_deref(), Some("乾"));
        assert_eq!(m.daoti_preview_bagua.as_deref(), Some("乾为天"));
        assert_eq!(m.daoti_preview_version.as_deref(), Some("pv1"));
        assert_eq!(m.created_at, ts, "源时间必须回填到 created_at");
    }

    /// 缺省路径零行为变更：不给字段时的默认值必须与历史一致
    #[test]
    fn into_memory_defaults_when_absent() {
        let m = MemoryWriteInput::new("x").into_memory();
        assert_eq!(m.memory_type, MemoryType::Fact);
        assert_eq!(m.importance.value(), 5);
        assert_eq!(m.privacy_level, PrivacyLevel::User);
        assert!(m.event_id.is_none());
        assert!(m.entities.is_empty());
        assert!(m.user_id.is_none());
    }

    // ════ 决策 1：created_at 取更早者（三情形 + 幂等）════

    /// 情形①：旧值是"摄入时刻"、新值是真源时间 ⇒ 应**修正**
    #[test]
    fn merge_created_at_corrects_ingest_time() {
        let mut existing = mem("a"); // created_at = 此刻（摄入时刻）
        let mut incoming = mem("a");
        incoming.created_at = Utc::now() - chrono::Duration::days(365);
        apply_merge(&incoming, &mut existing);
        assert!(
            existing.created_at < Utc::now() - chrono::Duration::days(300),
            "应被修正为真实源时间，实际 {:?}",
            existing.created_at
        );
    }

    /// 情形②：两者都是真源时间 ⇒ 应**保留首次发生**（后来的不得改写）
    #[test]
    fn merge_created_at_keeps_earliest() {
        let base = Utc::now() - chrono::Duration::days(100);
        let mut existing = mem("a");
        existing.created_at = base;
        let mut incoming = mem("a");
        incoming.created_at = base + chrono::Duration::days(10);
        apply_merge(&incoming, &mut existing);
        assert_eq!(
            existing.created_at, base,
            "后来的源时间不得改写首次发生时间"
        );
    }

    /// 情形③：入参未提供源时间 ⇒ 应**不变**
    #[test]
    fn merge_created_at_unchanged_without_source_time() {
        let base = Utc::now() - chrono::Duration::days(100);
        let mut existing = mem("a");
        existing.created_at = base;
        let incoming = mem("a"); // created_at = 此刻（无源时间）
        apply_merge(&incoming, &mut existing);
        assert_eq!(existing.created_at, base);
    }

    /// 幂等：反复合并同一输入不得让 created_at 漂移
    #[test]
    fn merge_created_at_is_idempotent() {
        let mut existing = mem("a");
        let mut incoming = mem("a");
        incoming.created_at = Utc::now() - chrono::Duration::days(365);
        apply_merge(&incoming, &mut existing);
        let after_first = existing.created_at;
        apply_merge(&incoming, &mut existing);
        assert_eq!(existing.created_at, after_first, "反复合并不得漂移");
    }

    // ════ 归属与隐私：不扩权 ════

    /// 归属**不得**被合并副作用改写（那属显式操作）
    #[test]
    fn merge_does_not_rewrite_ownership() {
        let mut existing = mem("a");
        existing.session_id = Some("s-old".into());
        existing.user_id = Some("u-old".into());
        let mut incoming = mem("a");
        incoming.session_id = Some("s-new".into());
        incoming.user_id = Some("u-new".into());
        apply_merge(&incoming, &mut existing);
        assert_eq!(
            existing.session_id.as_deref(),
            Some("s-old"),
            "归属不得被合并改写"
        );
        assert_eq!(existing.user_id.as_deref(), Some("u-old"));
    }

    /// 但允许**补全**缺失的归属
    #[test]
    fn merge_fills_missing_ownership() {
        let mut existing = mem("a");
        let mut incoming = mem("a");
        incoming.session_id = Some("s1".into());
        incoming.user_id = Some("u1".into());
        apply_merge(&incoming, &mut existing);
        assert_eq!(existing.session_id.as_deref(), Some("s1"));
        assert_eq!(existing.user_id.as_deref(), Some("u1"));
    }

    /// 隐私级别取更严格者 —— 合并不得**扩大**可见范围，但更严格者应生效
    #[test]
    fn merge_privacy_takes_stricter_and_never_widens() {
        let mut existing = mem("a");
        existing.privacy_level = PrivacyLevel::Session;
        let mut incoming = mem("a");
        incoming.privacy_level = PrivacyLevel::Global;
        apply_merge(&incoming, &mut existing);
        assert_eq!(
            existing.privacy_level,
            PrivacyLevel::Session,
            "合并不得扩大可见范围"
        );

        let mut existing2 = mem("a");
        existing2.privacy_level = PrivacyLevel::Global;
        let mut incoming2 = mem("a");
        incoming2.privacy_level = PrivacyLevel::User;
        apply_merge(&incoming2, &mut existing2);
        assert_eq!(
            existing2.privacy_level,
            PrivacyLevel::User,
            "更严格者应生效"
        );
    }

    // ════ 并集与版本 ════

    /// 标签 / 实体 / 来源关系取并集去重；内容未变时**不得**虚增版本号
    #[test]
    fn merge_unions_collections_and_does_not_inflate_version() {
        let mut existing = mem("a");
        existing.tags = vec!["t1".into()];
        existing.source_ids = vec!["m1".into()];
        let mut incoming = mem("a");
        incoming.tags = vec!["t1".into(), "t2".into()];
        incoming.source_ids = vec!["m2".into()];
        incoming.entities = vec![EventEntity {
            name: "西湖".into(),
            kind: EntityKind::Place,
        }];
        let outcome = apply_merge(&incoming, &mut existing);
        assert_eq!(existing.tags, vec!["t1".to_string(), "t2".to_string()]);
        assert_eq!(
            existing.source_ids,
            vec!["m1".to_string(), "m2".to_string()]
        );
        assert_eq!(existing.entities.len(), 1);
        assert!(!outcome.content_changed, "内容相同不应判定为变化");
        assert_eq!(existing.version, 1, "内容未变不得虚增版本号");
    }

    /// 内容真变时：替换内容 + 存档旧版本 + 版本号 +1
    #[test]
    fn merge_content_change_archives_version() {
        let mut existing = mem("旧内容");
        let incoming = mem("新内容");
        let outcome = apply_merge(&incoming, &mut existing);
        assert!(outcome.content_changed);
        assert_eq!(existing.content, "新内容");
        assert_eq!(existing.version, 2);
        assert_eq!(existing.version_history.len(), 1);
        assert_eq!(existing.version_history[0].content, "旧内容");
    }
}
