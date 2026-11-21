//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现宿主钩子载荷解析，属于公开层 (Layer 1)。
//! ============================================================
//!
//! # 为什么需要本模块
//!
//! 任务结束自动沉淀（`lrc absorb`）的输入来自各宿主的钩子载荷：
//!
//! | 宿主 | 字段风格 | 会话字段 | 文本字段 |
//! |---|---|---|---|
//! | Trae Stop | snake_case | `session_id` | `last_assistant_message` |
//! | Codex notify | kebab-case | `thread-id` | `last-assistant-message` |
//! | Cursor stop | snake_case | `conversation_id` | `last_assistant_message` / `text` |
//!
//! 字段名不一致是**契约差异**，不是错误；若在每个入口各写一份兼容逻辑，
//! 新增宿主时必然漏改其中一处。故收敛到本模块，作为唯一真源。
//!
//! # 边界
//!
//! - 纯解析：不读 stdin、不访问网络、不写库。
//! - 不是 JSON（或不是对象）时返回 `None`，由调用方按纯文本处理。

/// 宿主钩子 JSON 中与沉淀相关的字段。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HostPayload {
    /// 本轮最终回复文本
    pub text: Option<String>,
    /// 会话标识
    pub session_id: Option<String>,
    /// 宿主标识（载荷自带时优先于命令行）
    pub host: Option<String>,
    /// 归属项目（载荷自带时优先于当前目录推断）
    pub project: Option<String>,
    /// 宿主工作目录（Codex notify 的 `cwd`；仅用于推断项目归属）
    pub cwd: Option<String>,
    /// Stop 钩子自身触发的轮次（Cursor / Claude Code 的 `stop_hook_active`）。
    ///
    /// 为 `true` 时必须视为空轮次：否则钩子会把自己刚写入的沉淀再次沉淀，
    /// 形成自激循环。
    pub stop_hook_active: bool,
}

/// 从宿主钩子的 JSON 载荷中提取本轮文本与会话信息。
///
/// 兼容字段：
/// - 文本：`last_assistant_message` / `last-assistant-message` / `text` / `response`
/// - 会话：`session_id` / `sessionId` / `thread-id` / `thread_id` / `conversation_id`
/// - 工作目录：`cwd`，或 Cursor 的 `workspace_roots[0]`
///
/// 返回 `None` 表示"不是 JSON 对象"，调用方应按纯文本处理。
/// 返回 `Some` 但 `text` 为 `None` 表示"是 JSON 但没有文本字段"——
/// 这是空轮次，**不得**把整段 JSON 当结论沉淀。
pub fn parse_host_payload(raw: &str) -> Option<HostPayload> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let object = value.as_object()?;
    let field = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
            .map(str::to_string)
    };
    // Cursor 的 stop 载荷不带 `cwd`，而是 `workspace_roots: ["<path>"]`。
    let cwd = field(&["cwd"]).or_else(|| {
        object
            .get("workspace_roots")
            .and_then(serde_json::Value::as_array)
            .and_then(|roots| roots.first())
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    });
    Some(HostPayload {
        text: field(&[
            "last_assistant_message",
            "last-assistant-message",
            "text",
            "response",
        ]),
        session_id: field(&[
            "session_id",
            "sessionId",
            "thread-id",
            "thread_id",
            "conversation_id",
        ]),
        host: field(&["host"]),
        project: field(&["project"]),
        cwd,
        stop_hook_active: object
            .get("stop_hook_active")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

/// 把原始输入收敛为"本轮文本"（`lrc absorb` 的唯一入口）。
///
/// 三种情况：
/// - 合法 JSON 对象：取文本字段；`stop_hook_active=true` 或无文本字段 ⇒ 空轮次
/// - 以 `{` 开头、**疑似宿主载荷**但不是合法 JSON：视为空轮次
///   （绝不把畸形 JSON 当散文提炼——实测会写出 `[代码] \n[LRC]\n- decision: ...}`
///   这类垃圾记忆）
/// - 其它（含以 `{` 开头的正常回复/代码块）：按纯文本原样返回
pub fn resolve_turn_text(raw: &str) -> String {
    match parse_host_payload(raw) {
        Some(payload) if payload.stop_hook_active => String::new(),
        Some(payload) => payload.text.unwrap_or_default(),
        None if looks_like_host_payload(raw) => String::new(),
        None => raw.trim().to_string(),
    }
}

/// 畸形 JSON 是否"疑似宿主载荷"。
///
/// 只看**宿主特征字段名**，不因"以 `{` 开头"就判空：
/// 模型完全可能在回复里贴一段 JSON 代码块（以 `{` 开头），那是正常内容。
/// 实测的畸形载荷（PowerShell 吞掉引号）仍包含 `last-assistant-message`，
/// 因此能被识别。
fn looks_like_host_payload(raw: &str) -> bool {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with('{') {
        return false;
    }
    // 只认**宿主载荷独有**的字段：`session_id` / `thread-id` 这类通用字段
    // 也常出现在正常回复的 JSON 代码块里，不能作为判据。
    const HOST_FIELDS: &[&str] = &[
        "last-assistant-message",
        "last_assistant_message",
        "stop_hook_active",
        "hook_event_name",
    ];
    // 字段名后必须紧跟冒号（兼容 JSON 的 `"field":` 与畸形载荷的 `field:`），
    // 进一步避免正文里"只是提到字段名"被误判为空轮次。
    HOST_FIELDS.iter().any(|field| {
        trimmed.contains(&format!("{field}:")) || trimmed.contains(&format!("{field}\":"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Codex notify 的真实载荷（字段为 kebab-case，作为最后一个 argv 传入）。
    #[test]
    fn parse_codex_notify_payload() {
        let raw = r#"{
            "type": "agent-turn-complete",
            "thread-id": "thread-abc123",
            "turn-id": "turn-1",
            "cwd": "G:\\code-memory",
            "last-assistant-message": "已完成。\n[LRC]\n- decision: 采用 RwLock"
        }"#;
        let payload = parse_host_payload(raw).expect("Codex 载荷必须可解析");
        assert_eq!(
            payload.text.as_deref(),
            Some("已完成。\n[LRC]\n- decision: 采用 RwLock")
        );
        assert_eq!(payload.session_id.as_deref(), Some("thread-abc123"));
        assert_eq!(payload.cwd.as_deref(), Some("G:\\code-memory"));
        assert!(payload.host.is_none(), "Codex 载荷不带 host，由命令行补齐");
    }

    /// Trae Stop 的真实载荷（snake_case）。
    #[test]
    fn parse_trae_stop_payload() {
        let raw = r#"{
            "session_id": "s-1",
            "cwd": "G:\\code-memory",
            "stop_hook_active": false,
            "last_assistant_message": "结论在末尾。"
        }"#;
        let payload = parse_host_payload(raw).expect("Trae 载荷必须可解析");
        assert_eq!(payload.text.as_deref(), Some("结论在末尾。"));
        assert_eq!(payload.session_id.as_deref(), Some("s-1"));
    }

    /// Cursor stop / afterAgentResponse 的候选字段。
    #[test]
    fn parse_cursor_stop_payload() {
        let raw = r#"{
            "conversation_id": "conv-9",
            "workspace_roots": ["G:\\code-memory"],
            "text": "本轮回复正文"
        }"#;
        let payload = parse_host_payload(raw).expect("Cursor 载荷必须可解析");
        assert_eq!(payload.text.as_deref(), Some("本轮回复正文"));
        assert_eq!(payload.session_id.as_deref(), Some("conv-9"));
        assert_eq!(
            payload.cwd.as_deref(),
            Some("G:\\code-memory"),
            "Cursor 用 workspace_roots[0] 代替 cwd"
        );
        assert!(!payload.stop_hook_active);
    }

    /// Stop 钩子自身触发的轮次必须被识别：调用方据此视为空轮次，避免自激循环。
    #[test]
    fn parse_stop_hook_active() {
        let raw = r#"{"stop_hook_active": true, "last_assistant_message": "钩子自身的输出"}"#;
        let payload = parse_host_payload(raw).expect("载荷必须可解析");
        assert!(payload.stop_hook_active);
    }

    /// JSON 但没有文本字段 ⇒ `Some` + `text=None`（空轮次，禁止整段 JSON 入库）。
    #[test]
    fn parse_json_without_text_is_empty_turn() {
        let payload = parse_host_payload(r#"{"thread-id":"t-1"}"#).expect("是 JSON 对象");
        assert!(payload.text.is_none());
        assert_eq!(payload.session_id.as_deref(), Some("t-1"));
    }

    /// 非 JSON / 数组 / 非法 JSON ⇒ `None`，调用方按纯文本处理。
    #[test]
    fn parse_non_object_returns_none() {
        assert!(parse_host_payload("普通文本回复").is_none());
        assert!(parse_host_payload("[1,2,3]").is_none());
        assert!(parse_host_payload("{not json").is_none());
        assert!(parse_host_payload("").is_none());
    }

    /// `resolve_turn_text`：合法 JSON 取文本；stop_hook_active 为空；纯文本原样。
    #[test]
    fn resolve_turn_text_variants() {
        assert_eq!(
            resolve_turn_text(r#"{"last_assistant_message":"结论正文"}"#),
            "结论正文"
        );
        assert_eq!(
            resolve_turn_text(r#"{"stop_hook_active":true,"last_assistant_message":"钩子输出"}"#),
            "",
            "钩子自身触发的轮次必须为空"
        );
        assert_eq!(resolve_turn_text("  普通文本  "), "普通文本");
    }

    /// ★实测修复：以 `{` 开头但**畸形**的 JSON 必须视为空轮次。
    ///
    /// 修前它会落到"纯文本"分支，被规则提炼器写成
    /// `[代码] \n[LRC]\n- decision: ...}` 这类垃圾记忆
    /// （真实触发场景：PowerShell 调用原生程序时吞掉 JSON 的引号）。
    #[test]
    fn resolve_turn_text_rejects_malformed_json() {
        let malformed = "{type:agent-turn-complete,last-assistant-message:结论}";
        assert!(parse_host_payload(malformed).is_none());
        assert_eq!(
            resolve_turn_text(malformed),
            "",
            "畸形 JSON 不得被当作散文提炼"
        );
    }

    /// 回归审查 P2-4：以 `{` 开头的**正常回复**（如 JSON 代码块）不得被误判为空。
    #[test]
    fn resolve_turn_text_keeps_plain_text_starting_with_brace() {
        let reply = "{\"code\": \"示例\"} 这段 JSON 说明了配置格式";
        assert!(parse_host_payload(reply).is_none());
        assert_eq!(
            resolve_turn_text(reply),
            reply,
            "没有宿主特征字段的 `{{` 开头文本必须保留"
        );
    }

    /// 最终审查 P2-1：正文里"提到 session_id 但没有赋值"不得被误判为宿主载荷。
    #[test]
    fn resolve_turn_text_keeps_prose_mentioning_host_field() {
        let reply = "{说明：session_id 字段用于会话隔离，本轮没有赋值}";
        assert!(parse_host_payload(reply).is_none());
        assert_eq!(
            resolve_turn_text(reply),
            reply,
            "字段名后没有冒号的正文必须保留"
        );
    }
}
