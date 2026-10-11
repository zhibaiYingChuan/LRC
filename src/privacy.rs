//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现「可见性作用域」的单一真源，属于公开层 (Layer 1)。
//! ============================================================
//!
//! ## 背景：隐私隔离曾有两条主通道**根本没接入**
//!
//! `MemoryStore` 的 `is_visible` 语义是：`privacy_context = None` ⇒ **全部可见**。
//! 这是为**本地单用户桌面场景**设计的（不传归属即不做过滤）。
//! 但对外通道把它当成了"默认值"，于是：
//!   - **MCP**：`recall` / `recall_enhanced` / `list_memories` 共 5 处**硬编码**
//!     `privacy_context: None` ⇒ 写进去的 `user_id`/`privacy_level` **读时被完全忽略**；
//!   - **HTTP 只读端点**：`list`/`recent`/`stored-edges` 连 `user_id` 参数都没有。
//!
//! ## 本模块提供什么
//! ① [`scoped`] / [`scope_from_args`]：由归属字段构造隐私上下文的**唯一入口**；
//! ② [`TrustMode`]（信任域）+ [`require_scope`]：**分层**策略 ——
//!    "本地可信域保留既有全可见行为、对外域强制要求 scope"，从而在不破坏桌面端的前提下堵住对外通道。
//!
//! ## 为什么用"信任域"而不是直接改成 fail-closed
//! 若把 `None` 直接改成"仅 `Global` 可见"，桌面端（不传 scope）会**看不到自己的记忆**
//! ⇒ 破坏产品。而 `LRC_API_TOKEN` 是运维**已有的**"本服务对外提供"开关，
//! 用它作为判据既不新增配置，也与既有安全模型一致（见 `server.rs` 的安全告警文档）。

use crate::memory_types::PrivacyLevel;

/// 隐私上下文三元组（与 `ListFilter` / `RecallFilter` 的字段类型一致，直接可赋值）
///
/// **v0.9.11（G3）收炼**：本别名改为 [`crate::memory_store::PrivacyScope`] 的同义词 ——
/// 此前两处各写一遍同样的元组（G3 分两步实施留下），属**同义真源重复**；
/// 未来给上下文加字段（如 tenant）时只需改一处。
pub type ScopeTuple = crate::memory_store::PrivacyScope;

/// 多租户命名空间分隔符。
///
/// 对齐业界 memory 平台约定（如 Zep 的 `tenant_id::user_id::uuid`）：
/// 归属键统一写成「租户::用户」，使**凭据**成为真正的隔离边界。
pub const TENANT_NAMESPACE_SEP: &str = "::";

/// 从凭据（`LRC_API_TOKEN`）**派生**租户标识 —— 不可逆、稳定。
///
/// ## 为什么租户必须由凭据派生（CR-19：这是本函数的全部理由）
/// 修前：请求体里的 `user_id`（以及隐含的"我是哪个租户"）**完全由客户端自报**
/// ⇒ 任何持有 token 的调用方都能声明成别人的 `user_id`，读走别人的记忆。
/// 业界共识（anysearch 佐证）：**客户端不得在请求体中声明租户**；
/// 租户身份必须在认证层从凭据派生，构造**不可冒充**的 principal。
///
/// 实现取 token 的 **SHA-256 前 16 字节（32 位十六进制）**：
/// - **不可逆** ⇒ 不会把 token 明文写进记忆库的 `user_id` 字段；
/// - **稳定** ⇒ 同一 token 恒等，重放/重启后归属一致。
pub fn tenant_from_credential(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.trim().as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(32);
    for byte in &digest[..16] {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 当前部署的租户标识：**仅多租户域**（已设非空 `LRC_API_TOKEN`）才有值。
///
/// 单 token ⇒ 单租户（业界一致结论：共享一个 key 跨多租户等于没有隔离，
/// 真正的多租户需每租户独立凭据）。
pub fn current_tenant(mode: TrustMode) -> Option<String> {
    if mode != TrustMode::MultiTenant {
        return None;
    }
    std::env::var("LRC_API_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .map(|t| tenant_from_credential(&t))
}

/// 把客户端自报的 `user_id` 规范化为**租户内**归属（纯函数，便于免环境测试）。
///
/// | 输入 | 返回 |
/// |---|---|
/// | `user_id` 缺失/空白 | `None`（调用方随后按信任域决定放行或拒绝） |
/// | 有租户（多租户域且能派生） | `Some("{tenant}::{user}")` |
/// | 无租户（本地可信域，或显式传入 `None`） | `Some(user)` —— **逐字节原样**返回 |
///
/// "本地可信域原样返回"是**兼容红线**：桌面端与比赛口径都跑在无 token 的本地域，
/// 必须零变更。
pub fn namespace_owner_with(tenant: Option<&str>, user_id: Option<&str>) -> Option<String> {
    let user = user_id.map(str::trim).filter(|s| !s.is_empty())?;
    match tenant.map(str::trim).filter(|s| !s.is_empty()) {
        Some(tenant) => Some(format!("{tenant}{TENANT_NAMESPACE_SEP}{user}")),
        None => Some(user.to_string()),
    }
}

/// [`namespace_owner_with`] 的环境封装：自动取当前租户。
pub fn namespace_owner(mode: TrustMode, user_id: Option<&str>) -> Option<String> {
    let tenant = current_tenant(mode);
    namespace_owner_with(tenant.as_deref(), user_id)
}

/// 信任域
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustMode {
    /// 本地可信域（单用户桌面）：无 scope ⇒ 保持**既有**"全可见"行为
    LocalTrusted,
    /// 多租户/对外域：读取入口**必须**显式给出 scope
    MultiTenant,
}

/// 判据：**是否启用 `LRC_API_TOKEN`**。
///
/// ## 为什么以 token 为唯一判据（而不看回环地址）
/// HTTP handler 层**无法可靠获取对端是否为回环**（可能经反代），
/// 而"启用 token"是运维**显式声明对外提供服务**的既有开关（`server.rs` 的安全文档即以此为准）。
/// 用它作判据 ⇒ 不新增配置、不与既有模型冲突，且失败方向是**保守**的
/// （宁可多要 scope，不可少要）。
pub fn judge(token_configured: bool) -> TrustMode {
    if token_configured {
        TrustMode::MultiTenant
    } else {
        TrustMode::LocalTrusted
    }
}

/// 从环境读取信任域（`LRC_API_TOKEN` 非空 ⇒ 多租户）
pub fn from_env() -> TrustMode {
    let token_configured = std::env::var("LRC_API_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    judge(token_configured)
}

/// 由归属字段构造隐私上下文（**唯一入口**）。
///
/// 规则：
/// - 两者皆缺 ⇒ `None`（**保持既有"全可见"行为**，不改变本地场景）；
/// - 给了 `session_id` ⇒ `Session` 级（最严格，会话内可见）；
/// - 只给 `user_id` ⇒ `User` 级。
pub fn scoped(user_id: Option<&str>, session_id: Option<&str>) -> Option<ScopeTuple> {
    let user = user_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let session = session_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if user.is_none() && session.is_none() {
        return None;
    }
    let level = if session.is_some() {
        PrivacyLevel::Session
    } else {
        PrivacyLevel::User
    };
    Some((level, session, user))
}

/// 从 MCP 工具参数构造隐私上下文（供各 MCP handler 复用）。
///
/// **这是 CR-02 的修法**：此前 MCP 各处**硬编码 `None`**，
/// 使 `remember` 写入的 `user_id`/`privacy_level` 在读取时被完全忽略。
///
/// **v0.9.11（CR-19）**：`user_id` 先经 [`namespace_owner`] 规范化 ——
/// 多租户域下自动加「租户::」前缀，客户端无法通过自报 `user_id` 跨出租户。
pub fn scope_from_args(args: &serde_json::Value) -> Option<ScopeTuple> {
    let owner = namespace_owner(from_env(), args.get("user_id").and_then(|v| v.as_str()));
    scoped(
        owner.as_deref(),
        args.get("session_id").and_then(|v| v.as_str()),
    )
}

/// 多租户域下**强制**要求 scope；本地可信域直接通过。
///
/// 返回 `Err(错误码)` 时调用方应回 `400`（契约要求"负载非法"用 400/422）。
pub fn require_scope(mode: TrustMode, scope: &Option<ScopeTuple>) -> Result<(), &'static str> {
    match (mode, scope.is_some()) {
        (TrustMode::MultiTenant, false) => Err("scope_required"),
        _ => Ok(()),
    }
}

/// 读取入口的**统一 scope 解析 + 强制**（供各 HTTP/MCP 读取入口复用）。
///
/// 语义：先按归属构造上下文，再按信任域校验 ——
/// - 本地可信域：无 scope 也放行（保持既有全可见行为）；
/// - 多租户域：无 scope ⇒ `Err("scope_required")`，调用方应回 `400`。
///
/// **v0.9.11（CR-19）**：归属先经 [`namespace_owner`] 规范化（多租户域加租户前缀），
/// 使"隔离键由凭据派生"，而不是由客户端自报。
pub fn resolve(
    mode: TrustMode,
    user_id: Option<&str>,
    session_id: Option<&str>,
) -> Result<Option<ScopeTuple>, &'static str> {
    let owner = namespace_owner(mode, user_id);
    let scope = scoped(owner.as_deref(), session_id);
    require_scope(mode, &scope)?;
    Ok(scope)
}

/// 派生写入的**隐私继承**（v0.9.11 / G3 / CR-04）。
///
/// ## 背景（这是一处真实的泄漏面，非比赛特有）
/// 结晶 / 发现 / 合成等**派生写入**（`consolidation.rs`、`synthesis_engine.rs`、
/// `discovery.rs`、`memory_store.rs` 的自动产物）此前**不继承来源隐私**。于是
/// "由私有记忆归纳出的摘要"可能以更宽的可见级别落库 —— 等于把私有内容**间接**暴露
/// 给其他 user。本函数是这条规则的**唯一真源**。
///
/// ## 规则：派生记忆的可见性 = 各来源可见性的**交集**（取最严）
/// | 来源集合 | 返回 |
/// |---|---|
/// | 全部 `Global`（无约束） | `Some((Global, None, None))` —— **既有行为逐位不变** |
/// | 同一 user，最严级别为 `User` | `Some((User, None, Some(user)))` |
/// | 同一 user 且 `Session` 级来源同属一个 session | `Some((Session, Some(sid), Some(user)))` |
/// | 跨 user ／ 受约束但缺 `user_id` ／ `Session` 级来源分属不同 session | **`None`** |
///
/// ## 为什么"跨域 ⇒ 返回 `None`（放弃派生）"而不是"设法隐藏"
/// ① 隐私级别里**没有"谁都不可见"**这一档，用永不匹配的 id 伪装属 hack；
/// ② 更本质：**跨用户摘要本身即不合法** —— 任一 user 都不该看到"把别人的私有记忆
///    归纳进来"的内容。**跳过并记日志**才是诚实做法。
///
/// ## 调用方契约
/// 返回 `None` ⇒ 调用方**必须放弃该派生写入**（并可选记日志），
/// **不得**回退到"无约束"落库。
pub fn inherit_privacy(sources: &[crate::memory_types::Memory]) -> Option<ScopeTuple> {
    use crate::memory_types::PrivacyLevel as L;
    // 无来源：无可继承的约束（极端情况，保持既有行为）
    if sources.is_empty() {
        return Some((L::Global, None, None));
    }
    // 只看**有约束**的来源：Global 级来源对派生结果不构成约束
    let constrained: Vec<&crate::memory_types::Memory> = sources
        .iter()
        .filter(|m| m.privacy_level != L::Global)
        .collect();
    if constrained.is_empty() {
        return Some((L::Global, None, None));
    }

    // ① 归属必须唯一：跨 user 或归属不明 ⇒ 放弃派生
    let mut user: Option<&str> = None;
    for m in &constrained {
        match m.user_id.as_deref() {
            Some(u) if user.is_none() => user = Some(u),
            Some(u) if user == Some(u) => {}
            _ => return None, // 跨 user，或"有约束却无 user_id"（归属不明）
        }
    }
    let user = user?;

    // ② 级别取**最严**：只要有 Session 级来源，派生也必须是 Session 级
    let has_session_scoped = constrained.iter().any(|m| m.privacy_level == L::Session);
    if has_session_scoped {
        // Session 级来源必须同属一个 session，否则交集为空 ⇒ 放弃派生
        let mut sid: Option<&str> = None;
        for m in constrained.iter().filter(|m| m.privacy_level == L::Session) {
            match m.session_id.as_deref() {
                Some(s) if sid.is_none() => sid = Some(s),
                Some(s) if sid == Some(s) => {}
                _ => return None, // 分属不同 session
            }
        }
        return Some((L::Session, sid.map(str::to_string), Some(user.to_string())));
    }

    // ③ 仅 User 级约束
    Some((L::User, None, Some(user.to_string())))
}

#[cfg(test)]
mod tests {
    /// 构造测试记忆（仅供继承规则测试使用）
    fn make_priv_memory(
        content: &str,
        level: crate::memory_types::PrivacyLevel,
        session: Option<&str>,
        user: Option<&str>,
    ) -> crate::memory_types::Memory {
        use crate::memory_types::{Importance, Memory, MemoryType};
        Memory::new(
            content.to_string(),
            MemoryType::Fact,
            None,
            Vec::new(),
            Importance::new(5),
            None,
        )
        .with_privacy(level, session.map(str::to_string), user.map(str::to_string))
    }

    /// 全部 Global（无约束）⇒ 派生保持无约束：**既有行为逐位不变**
    #[test]
    fn inherit_privacy_all_global_stays_unconstrained() {
        use crate::memory_types::PrivacyLevel as L;
        let srcs = vec![
            make_priv_memory("a", L::Global, None, None),
            make_priv_memory("b", L::Global, None, None),
        ];
        assert_eq!(inherit_privacy(&srcs), Some((L::Global, None, None)));
    }

    /// 同一 user（User 级）⇒ 派生也必须是 User 级并携带该 user
    #[test]
    fn inherit_privacy_same_user_becomes_user_scoped() {
        use crate::memory_types::PrivacyLevel as L;
        let srcs = vec![
            make_priv_memory("a", L::User, None, Some("alice")),
            make_priv_memory("b", L::Global, None, None), // Global 不构成约束
        ];
        assert_eq!(
            inherit_privacy(&srcs),
            Some((L::User, None, Some("alice".to_string())))
        );
    }

    /// 全部 Session 级且同属一个 session ⇒ 派生同为 Session 级（携带 session 与 user）
    #[test]
    fn inherit_privacy_session_scoped_when_same_session() {
        use crate::memory_types::PrivacyLevel as L;
        let srcs = vec![
            make_priv_memory("a", L::Session, Some("s1"), Some("alice")),
            make_priv_memory("b", L::Session, Some("s1"), Some("alice")),
        ];
        assert_eq!(
            inherit_privacy(&srcs),
            Some((
                L::Session,
                Some("s1".to_string()),
                Some("alice".to_string())
            ))
        );
    }

    /// ★核心：**跨 user 的来源必须拒绝派生** —— 否则摘要会把私有内容带给其他 user
    #[test]
    fn inherit_privacy_cross_user_is_rejected() {
        use crate::memory_types::PrivacyLevel as L;
        let srcs = vec![
            make_priv_memory("alice 的事", L::User, None, Some("alice")),
            make_priv_memory("bob 的事", L::User, None, Some("bob")),
        ];
        assert_eq!(
            inherit_privacy(&srcs),
            None,
            "跨 user 的来源不得派生出任何记忆（跨用户摘要本身即泄漏）"
        );
    }

    /// Session 级来源分属不同 session ⇒ 交集为空 ⇒ 拒绝派生
    #[test]
    fn inherit_privacy_different_sessions_is_rejected() {
        use crate::memory_types::PrivacyLevel as L;
        let srcs = vec![
            make_priv_memory("a", L::Session, Some("s1"), Some("alice")),
            make_priv_memory("b", L::Session, Some("s2"), Some("alice")),
        ];
        assert_eq!(inherit_privacy(&srcs), None);
    }

    /// 受约束却**缺 user_id** ⇒ 归属不明，保守拒绝（不得当无约束放行）
    #[test]
    fn inherit_privacy_constrained_without_user_is_rejected() {
        use crate::memory_types::PrivacyLevel as L;
        let srcs = vec![make_priv_memory("a", L::User, None, None)];
        assert_eq!(
            inherit_privacy(&srcs),
            None,
            "有约束却无归属 ⇒ 必须拒绝，而非回退为全可见"
        );
    }

    use super::*;

    #[test]
    fn judge_maps_token_to_trust_mode() {
        assert_eq!(judge(true), TrustMode::MultiTenant);
        assert_eq!(judge(false), TrustMode::LocalTrusted);
    }

    #[test]
    fn scoped_returns_none_when_both_absent() {
        assert!(scoped(None, None).is_none());
        // 空白串等同于"未提供"（不得被当成一个用户）
        assert!(scoped(Some("   "), Some("")).is_none());
    }

    #[test]
    fn scoped_prefers_strictest_level() {
        let (level, session, user) = scoped(Some("u1"), Some("s1")).expect("应构造成功");
        assert_eq!(level, PrivacyLevel::Session, "给了会话即按会话级隔离");
        assert_eq!(session.as_deref(), Some("s1"));
        assert_eq!(user.as_deref(), Some("u1"));

        let (level2, session2, user2) = scoped(Some("u1"), None).expect("应构造成功");
        assert_eq!(level2, PrivacyLevel::User);
        assert!(session2.is_none());
        assert_eq!(user2.as_deref(), Some("u1"));
    }

    #[test]
    fn scope_from_args_reads_mcp_arguments() {
        let args = serde_json::json!({"user_id": "u1", "session_id": "s1"});
        let (level, _, _) = scope_from_args(&args).expect("应从参数构造");
        assert_eq!(level, PrivacyLevel::Session);

        let empty = serde_json::json!({});
        assert!(
            scope_from_args(&empty).is_none(),
            "缺省不得改变既有全可见行为"
        );
    }

    #[test]
    fn require_scope_enforces_only_in_multi_tenant() {
        let none = None;
        let some = scoped(Some("u1"), None);
        assert!(
            require_scope(TrustMode::LocalTrusted, &none).is_ok(),
            "本地域不得拦截"
        );
        assert_eq!(
            require_scope(TrustMode::MultiTenant, &none),
            Err("scope_required"),
            "多租户域下缺 scope 必须拒绝"
        );
        assert!(require_scope(TrustMode::MultiTenant, &some).is_ok());
    }

    /// 租户标识必须**稳定**（同一 token 恒等）且**不可逆**（不得把 token 明文写进库）
    #[test]
    fn tenant_from_credential_is_stable_and_does_not_leak_token() {
        let token = "s3cret-token-abc";
        let a = tenant_from_credential(token);
        let b = tenant_from_credential(token);
        assert_eq!(a, b, "同一凭据派生结果必须稳定");
        assert_eq!(a.len(), 32, "前 16 字节 ⇒ 32 位十六进制");
        assert!(
            a.chars().all(|c| c.is_ascii_hexdigit()),
            "必须是十六进制摘要，实际 {a}"
        );
        assert!(
            !a.contains(token) && a != token,
            "派生值不得包含/等于 token 明文（否则会把凭据写进记忆库）"
        );
        assert_ne!(
            a,
            tenant_from_credential("s3cret-token-abd"),
            "不同凭据必须得到不同租户"
        );
        assert_eq!(
            a,
            tenant_from_credential("  s3cret-token-abc  "),
            "两侧空白应被规整（前后端裁剪口径一致）"
        );
    }

    /// 命名空间规则：**有租户才加前缀**；本地可信域逐字节原样（兼容红线）
    #[test]
    fn namespace_owner_namespaces_only_when_tenant_present() {
        assert_eq!(
            namespace_owner_with(None, Some("alice")).as_deref(),
            Some("alice"),
            "本地可信域必须逐字节原样（桌面端 / 比赛口径零变更）"
        );
        assert_eq!(
            namespace_owner_with(Some("t1"), Some("alice")).as_deref(),
            Some("t1::alice"),
            "多租户域必须加租户前缀（对齐业界 tenant::user 约定）"
        );
        assert_eq!(
            namespace_owner_with(Some("t1"), None),
            None,
            "缺 user_id 不得凭租户凭空造一个用户"
        );
        assert_eq!(
            namespace_owner_with(Some("t1"), Some("   ")),
            None,
            "空白 user_id 等同缺失"
        );
        assert_eq!(
            namespace_owner_with(Some("   "), Some("alice")).as_deref(),
            Some("alice"),
            "空租户等同无租户"
        );
    }

    /// 本地可信域下 `resolve` 的返回值必须与历史**逐字节一致**
    #[test]
    fn resolve_local_trusted_unchanged_by_namespacing() {
        let scope = resolve(TrustMode::LocalTrusted, Some("alice"), Some("s1"))
            .unwrap()
            .expect("本地域应构造出上下文");
        assert_eq!(scope.0, PrivacyLevel::Session);
        assert_eq!(scope.1.as_deref(), Some("s1"));
        assert_eq!(
            scope.2.as_deref(),
            Some("alice"),
            "本地域不得加任何前缀（否则桌面端/比赛数据会突然互相看不见）"
        );
    }

    #[test]
    fn resolve_enforces_and_returns_scope() {
        assert!(resolve(TrustMode::LocalTrusted, None, None)
            .unwrap()
            .is_none());
        assert_eq!(
            resolve(TrustMode::MultiTenant, None, None),
            Err("scope_required")
        );
        let scope = resolve(TrustMode::MultiTenant, Some("u1"), None)
            .expect("有 scope 必须放行")
            .expect("应构造出上下文");
        assert_eq!(scope.0, PrivacyLevel::User);
    }
}
