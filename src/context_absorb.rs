//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现「上下文沉淀」的语义过滤与模型声明解析，属于公开层 (Layer 1)。
//! ============================================================
//!
//! # 为什么需要本模块
//!
//! 宿主（Trae / Codex / Cursor / Claude Code）在任务结束时只能提供
//! **最终回复**或模型主动声明的结论块。LRC 不能从碎片里"猜"语义，
//! 但可以做三件确定的事：
//!
//! 1. **解析**模型按规则输出的 `[LRC]` 结论块（JSON 或行格式）；
//! 2. **过滤**明显不是结论的内容（过程叙述、纯代码、过短、删除语义）；
//! 3. **批内去重**（同一轮里重复表述只留一条）。
//!
//! 跨轮去重与落库由 `/v1/context/absorb` 调用存储层的
//! `prepare_remember_with_privacy` 完成（阈值 0.85，避免自动摘要
//! 合并进用户显式写入的高价值记忆；匿名轮次只允许合并 Global）。
//!
//! # 边界
//!
//! - 本模块**不写库**、不访问网络、不读文件。
//! - 规则引擎只做"保守过滤"：宁可漏，不可滥。

use crate::context_distill::DistilledItem;
use serde::{Deserialize, Serialize};

/// 自动沉淀允许的记忆类型（与 `MemoryType::try_parse` 对齐）。
const ALLOWED_TYPES: &[&str] = &["decision", "preference", "fact", "code_context"];

/// 模型声明块的起始标记。
pub const MODEL_BLOCK_MARKER: &str = "[LRC]";

/// 单条内容的最大字符数（与 `DistillConfig::max_chars` 默认值一致）。
const MAX_CONTENT_CHARS: usize = 300;

/// 单条内容的最小字符数（过滤"好的""收到"这类）。
const MIN_CONTENT_CHARS: usize = 8;

/// 批内去重的相似度阈值（字符二元组 Jaccard）。
const DEDUPE_THRESHOLD: f32 = 0.85;

/// 模型声明的单条结论（`[LRC]` 块中的一项）。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DeclaredItem {
    /// 类型：decision | constraint | conclusion | error | preference | fact | code_context
    #[serde(rename = "type", alias = "kind")]
    pub kind: String,
    /// 结论正文
    pub content: String,
    /// 可选：被本条修正/替代的旧内容关键词（仅打标签，不自动删除）
    #[serde(default)]
    pub supersedes: Option<String>,
}

/// 解析结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedBlock {
    /// 成功解析出的条目
    pub items: Vec<DistilledItem>,
    /// 被过滤的原因（用于 dry-run 诊断）
    pub dropped: Vec<String>,
    /// 是否命中了 `[LRC]` 标记（或调用方直接给了 items）。
    ///
    /// **为什么必须有**：模型明确输出了 `[LRC]` 但条目全被过滤时，
    /// 不能回退到全文规则提炼——那会把模型没有声明的内容写进库。
    pub found: bool,
}

/// 把调用方直接给出的声明条目规范化（供 `/v1/context/absorb` 的 `items` 字段使用）。
pub fn parse_declared_items(items: Vec<DeclaredItem>) -> ParsedBlock {
    let found = !items.is_empty();
    let mut parsed = ParsedBlock {
        found,
        ..Default::default()
    };
    for declared in items {
        push_declared(&mut parsed, declared);
    }
    parsed
}

/// 从最终回复中解析 `[LRC]` 结论块。
///
/// 支持两种格式（二选一）：
///
/// ```text
/// [LRC]
/// - decision: 采用 RwLock
/// - constraint: 不要提交 secrets
/// ```
///
/// ```text
/// [LRC]
/// {"items":[{"type":"decision","content":"采用 RwLock"}]}
/// ```
///
/// 找不到标记时返回空结果（调用方回退到规则提炼）。
pub fn parse_model_block(text: &str) -> ParsedBlock {
    // 只认"独立成行"的标记：正文里引用 `[LRC]` 字样（如解释规则、行内代码）
    // 不应吞掉整轮沉淀。取最后一个标记行，兼容"先举例、后给真结论"。
    let Some(pos) = text
        .match_indices(MODEL_BLOCK_MARKER)
        .filter(|(pos, _)| is_marker_line(text, *pos))
        .map(|(pos, _)| pos)
        .last()
    else {
        return ParsedBlock::default();
    };
    let block = &text[pos + MODEL_BLOCK_MARKER.len()..];
    // 只取到下一个一级标题 / 分隔线 / 另一个 `[LRC]` 标记行，避免把后续正文吃进来。
    let block = trim_at_block_boundary(block).trim();
    if block.is_empty() {
        // 命中标记但块为空：模型明确表示"本轮无结论"，禁止回退规则提炼。
        return ParsedBlock {
            found: true,
            ..Default::default()
        };
    }

    // ① 先尝试 JSON（模型偶尔会输出结构化块）
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(block) {
        let arr = value
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .or_else(|| value.as_array().cloned());
        if let Some(arr) = arr {
            let mut parsed = ParsedBlock {
                found: true,
                ..Default::default()
            };
            for item in arr {
                match serde_json::from_value::<DeclaredItem>(item) {
                    Ok(declared) => push_declared(&mut parsed, declared),
                    Err(e) => parsed.dropped.push(format!("json_item_invalid: {e}")),
                }
            }
            return parsed;
        }
    }

    // ② 行格式：`- decision: ...` / `decision: ...`
    let mut parsed = ParsedBlock {
        found: true,
        ..Default::default()
    };
    for raw_line in block.lines() {
        let line = raw_line.trim().trim_start_matches(['-', '*', '•']).trim();
        if line.is_empty() {
            continue;
        }
        let Some((kind, content)) = line.split_once(':').or_else(|| line.split_once('：')) else {
            parsed.dropped.push(format!("line_without_kind: {line}"));
            continue;
        };
        push_declared(
            &mut parsed,
            DeclaredItem {
                kind: kind.trim().to_string(),
                content: content.trim().to_string(),
                supersedes: None,
            },
        );
    }
    parsed
}

/// 把结论块截断到下一个**真正的块边界**：Markdown 标题、分隔线、另一个 `[LRC]` 行。
///
/// 不能按 `\n[` 粗暴截断：模型常直接给 JSON 数组（`[LRC]` 后紧跟 `[{...}]`），
/// 那样会把合法结论整块截空，且 `found=true` 会禁止回退规则提炼 ——
/// 表现为"模型声明了结论，但一条都没写，也没有任何诊断"（v0.11.1 审查 P1）。
fn trim_at_block_boundary(block: &str) -> &str {
    let mut offset = 0usize;
    let mut seen_content = false;
    for line in block.split_inclusive('\n') {
        let trimmed = line.trim();
        if !seen_content && trimmed.is_empty() {
            offset += line.len();
            continue;
        }
        if is_heading_line(trimmed) || is_rule_line(trimmed) || is_second_marker(trimmed) {
            return &block[..offset];
        }
        seen_content = true;
        offset += line.len();
    }
    block
}

/// ATX 标题（`#` ~ `######`，后跟空白或行尾）。
fn is_heading_line(trimmed: &str) -> bool {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes)
        && trimmed
            .chars()
            .nth(hashes)
            .map(|c| c.is_whitespace())
            .unwrap_or(true)
}

/// 分隔线（`---` / `***` / `___`，至少 3 个相同字符）。
fn is_rule_line(trimmed: &str) -> bool {
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    matches!(first, '-' | '*' | '_') && trimmed.chars().count() >= 3 && chars.all(|c| c == first)
}

/// 另一个独立成行的 `[LRC]` 标记。
fn is_second_marker(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix(MODEL_BLOCK_MARKER) else {
        return false;
    };
    match rest.chars().next() {
        None => true,
        Some(ch) => ch.is_whitespace() || matches!(ch, '：' | ':' | '{' | '['),
    }
}

/// 标记是否独立成行（允许行首缩进，标记后可跟空白/冒号/JSON 起始符）。
///
/// 行内出现（如 `` `[LRC]` `` 或「讨论 [LRC] 标记」）不算声明块。
fn is_marker_line(text: &str, pos: usize) -> bool {
    let line_start = text[..pos].rfind('\n').map(|index| index + 1).unwrap_or(0);
    let line_end = text[pos..]
        .find('\n')
        .map(|index| pos + index)
        .unwrap_or(text.len());
    let line = text[line_start..line_end].trim();
    let Some(rest) = line.strip_prefix(MODEL_BLOCK_MARKER) else {
        return false;
    };
    match rest.chars().next() {
        None => true,
        Some(ch) => ch.is_whitespace() || matches!(ch, '：' | ':' | '{' | '['),
    }
}

/// 把一条声明规范化后放入结果；非法类型/空内容/过程叙述在这里丢弃。
fn push_declared(parsed: &mut ParsedBlock, declared: DeclaredItem) {
    let kind = declared.kind.trim().to_ascii_lowercase();
    let (memory_type, tag) = match kind.as_str() {
        "decision" => ("decision", "decision"),
        "constraint" | "约束" => ("preference", "constraint"),
        "conclusion" | "结论" => ("fact", "conclusion"),
        "error" | "报错" => ("code_context", "error"),
        "preference" | "偏好" => ("preference", "preference"),
        "fact" | "事实" => ("fact", "fact"),
        "code_context" => ("code_context", "code_context"),
        other => {
            parsed.dropped.push(format!("type_not_allowed: {other}"));
            return;
        }
    };
    let content = normalize_content(&declared.content);
    if content.is_empty() {
        parsed.dropped.push("empty_content".to_string());
        return;
    }
    if is_process_narration(&content) {
        parsed
            .dropped
            .push(format!("process_narration: {}", preview(&content)));
        return;
    }
    let mut tags = vec![
        "distilled".to_string(),
        "model_declared".to_string(),
        tag.to_string(),
    ];
    if let Some(supersedes) = declared.supersedes.as_deref().map(str::trim) {
        if !supersedes.is_empty() {
            tags.push(format!("supersedes:{}", preview(supersedes)));
        }
    }
    parsed.items.push(DistilledItem {
        content,
        memory_type: memory_type.to_string(),
        tags,
        // 模型声明属于高置信来源，但自动沉淀仍有上限（调用方再 cap）。
        importance: 6,
        source_turns: (0, 0),
    });
}

/// 规范化内容：压空白、去列表符号、截断到上限。
pub fn normalize_content(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim().trim_start_matches(['-', '*', '•']).trim();
    if trimmed.chars().count() <= MAX_CONTENT_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_CONTENT_CHARS).collect::<String>() + "…"
}

/// 判断一段文本是否包含"删除/修正记忆"的强语义。
///
/// 自动沉淀**绝不复活**用户显式删除的内容；命中时调用方应跳过本轮。
/// 只匹配强标记，避免把"删除了临时文件"误判成删除记忆。
pub fn has_deletion_intent(text: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "已删除该记忆",
        "已从记忆库删除",
        "已忘记",
        "已移除该记忆",
        "已修正该记忆",
        "删除记忆成功",
        "forget memory",
        "lrc forget",
        "correct_memory",
        "已撤销记忆",
    ];
    let lower = text.to_ascii_lowercase();
    PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_ascii_lowercase()))
}

/// 判断一条内容是否是过程叙述（不是结论）。
pub fn is_process_narration(content: &str) -> bool {
    let text = content.trim();
    if text.is_empty() {
        return true;
    }
    // 纯代码块 / 纯路径 / 纯命令
    if text.starts_with("```") || text.starts_with('$') || text.starts_with('>') {
        return true;
    }
    if text.starts_with('/') && !text.contains(' ') && text.len() < 120 {
        return true;
    }
    // 过短
    if text.chars().count() < MIN_CONTENT_CHARS {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    let process_prefixes = [
        "我先",
        "我先来",
        "接下来",
        "然后",
        "下面",
        "现在",
        "让我",
        "我来",
        "我会",
        "首先",
        "接着",
        "最后",
        "let me",
        "i'll",
        "i will",
        "next,",
        "next ",
        "first,",
        "now ",
        "we need to",
        "i'm going to",
    ];
    let strong_markers = [
        "决定",
        "采用",
        "必须",
        "不要",
        "禁止",
        "结论",
        "建议",
        "根因",
        "修复",
        "因为",
        "decision",
        "constraint",
        "must",
        "should",
        "conclusion",
        "root cause",
    ];
    let has_strong = strong_markers.iter().any(|m| lower.contains(m));
    if !has_strong && process_prefixes.iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    false
}

/// 批内去重：保留相似度 < 阈值的条目；重复时保留 importance 更高者。
pub fn dedupe_within(items: Vec<DistilledItem>) -> (Vec<DistilledItem>, usize) {
    let mut kept: Vec<DistilledItem> = Vec::with_capacity(items.len());
    let mut duplicates = 0usize;
    for item in items {
        if let Some(existing) = kept
            .iter_mut()
            .find(|k| char_bigram_jaccard(&k.content, &item.content) >= DEDUPE_THRESHOLD)
        {
            duplicates += 1;
            if item.importance > existing.importance {
                existing.importance = item.importance;
            }
            for tag in item.tags {
                if !existing.tags.contains(&tag) {
                    existing.tags.push(tag);
                }
            }
            continue;
        }
        kept.push(item);
    }
    (kept, duplicates)
}

/// 过滤规则兜底产出的条目（类型白名单 + 过程叙述 + 删除语义）。
pub fn filter_distilled(
    items: Vec<DistilledItem>,
    source_text: &str,
) -> (Vec<DistilledItem>, Vec<String>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    if has_deletion_intent(source_text) {
        dropped.push("deletion_guard".to_string());
        return (kept, dropped);
    }
    for mut item in items {
        let ty = item.memory_type.trim().to_ascii_lowercase();
        if !ALLOWED_TYPES.contains(&ty.as_str()) {
            dropped.push(format!("type_not_allowed: {ty}"));
            continue;
        }
        item.content = normalize_content(&item.content);
        if is_process_narration(&item.content) {
            dropped.push(format!("process_narration: {}", preview(&item.content)));
            continue;
        }
        // 自动摘要一律打标，便于后续过滤/归档。
        if !item.tags.iter().any(|t| t == "auto-summary") {
            item.tags.push("auto-summary".to_string());
        }
        kept.push(item);
    }
    (kept, dropped)
}

/// 字符二元组 Jaccard 相似度（中文无需分词）。
pub fn char_bigram_jaccard(a: &str, b: &str) -> f32 {
    let sa = bigrams(a);
    let sb = bigrams(b);
    if sa.is_empty() || sb.is_empty() {
        return if a.trim() == b.trim() { 1.0 } else { 0.0 };
    }
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn bigrams(text: &str) -> std::collections::HashSet<(char, char)> {
    let chars: Vec<char> = text
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
        .collect();
    if chars.len() < 2 {
        return chars.iter().map(|c| (*c, '\0')).collect();
    }
    chars.windows(2).map(|w| (w[0], w[1])).collect()
}

fn preview(text: &str) -> String {
    text.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_block() {
        let text = "做完了。\n[LRC]\n- decision: 采用 RwLock，读并发优先\n- constraint: 不要提交 secrets\n";
        let parsed = parse_model_block(text);
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].memory_type, "decision");
        assert_eq!(parsed.items[1].memory_type, "preference");
        assert!(parsed.items[1].tags.iter().any(|t| t == "constraint"));
    }

    #[test]
    fn parse_json_block() {
        let text =
            "[LRC]\n{\"items\":[{\"type\":\"conclusion\",\"content\":\"健康评分修复在 3ae886a\"}]}";
        let parsed = parse_model_block(text);
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].memory_type, "fact");
    }

    #[test]
    fn parse_json_array_directly_after_marker() {
        // v0.11.1 审查 P1：`[LRC]` 后紧跟 JSON 数组时，旧实现按 `\n[` 截断会整块清空。
        let text = "[LRC]\n[{\"type\":\"decision\",\"content\":\"采用 RwLock，读并发优先\"}]";
        let parsed = parse_model_block(text);
        assert!(parsed.found);
        assert_eq!(
            parsed.items.len(),
            1,
            "JSON 数组不得被截空：{:?}",
            parsed.dropped
        );
        assert_eq!(parsed.items[0].memory_type, "decision");
    }

    #[test]
    fn parse_block_stops_at_heading_or_rule_only() {
        // 方括号行不是块边界：后续合法条目仍必须解析。
        let bracket = parse_model_block(
            "[LRC]\n[约束] 不要提交 secrets\n- decision: 采用 RwLock，读并发优先",
        );
        assert_eq!(bracket.items.len(), 1, "{:?}", bracket.dropped);
        assert!(bracket.items[0].content.contains("RwLock"));
        assert!(
            bracket
                .dropped
                .iter()
                .any(|r| r.starts_with("line_without_kind")),
            "方括号行应作为无效行记录：{:?}",
            bracket.dropped
        );

        // 一级标题 / 分隔线是块边界：后续正文不得被吃进结论块。
        for boundary in [
            "\n## 说明\n- decision: 这是正文不是结论",
            "\n---\n- decision: 这是正文",
        ] {
            let text = format!("[LRC]\n- conclusion: 采用 RwLock 保证读并发优先{boundary}");
            let parsed = parse_model_block(&text);
            assert_eq!(parsed.items.len(), 1, "边界后内容不应进入块：{text}");
            assert!(parsed.items[0].content.contains("RwLock"));
        }
    }

    #[test]
    fn parse_missing_marker_returns_empty() {
        let parsed = parse_model_block("普通回复，没有结论块");
        assert!(parsed.items.is_empty());
        assert!(!parsed.found, "没有标记时 found 必须为 false");
    }

    #[test]
    fn parse_block_present_sets_found_even_if_items_dropped() {
        // 模型明确输出了 `[LRC]`，但条目类型非法 ⇒ items 为空、dropped 非空。
        // 调用方必须据此禁止回退全文提炼。
        let parsed = parse_model_block("[LRC]\n- 非法类型: 随便写点什么");
        assert!(parsed.items.is_empty());
        assert!(parsed.found, "命中标记后 found 必须为 true");
        assert!(
            parsed
                .dropped
                .iter()
                .any(|r| r.starts_with("type_not_allowed")),
            "应记录丢弃原因：{:?}",
            parsed.dropped
        );

        // 空块同样是"模型声明了本轮无结论"。
        let empty = parse_model_block("[LRC]");
        assert!(empty.items.is_empty());
        assert!(empty.found, "空 `[LRC]` 也必须视为已声明");
    }

    #[test]
    fn parse_ignores_inline_marker_mention() {
        // 行内引用（反引号 / 正文中出现）不算声明块，应允许回退规则提炼。
        assert!(!parse_model_block("规则要求回复末尾输出 `[LRC]` 结论块。").found);
        assert!(!parse_model_block("我们讨论一下 [LRC] 标记的含义。").found);

        // 独立成行的标记仍然命中；多个标记取最后一个。
        let parsed = parse_model_block(
            "示例：\n[LRC]\n- decision: 示例内容\n\n正式结论：\n[LRC]\n- decision: 采用 RwLock，读并发优先",
        );
        assert!(parsed.found);
        assert_eq!(parsed.items.len(), 1);
        assert!(parsed.items[0].content.contains("RwLock"));
    }

    #[test]
    fn parse_declared_items_sets_found() {
        assert!(!parse_declared_items(Vec::new()).found);
        let parsed = parse_declared_items(vec![DeclaredItem {
            kind: "decision".into(),
            content: "采用 RwLock，读并发优先".into(),
            supersedes: None,
        }]);
        assert!(parsed.found);
        assert_eq!(parsed.items.len(), 1);
    }

    #[test]
    fn process_narration_filtered() {
        assert!(is_process_narration("我先看看代码结构"));
        assert!(!is_process_narration("决定采用 RwLock，因为读多写少"));
    }

    #[test]
    fn deletion_guard_only_strong_markers() {
        assert!(has_deletion_intent("已删除该记忆，不会再出现"));
        assert!(!has_deletion_intent("已删除临时文件，继续下一步"));
    }

    #[test]
    fn dedupe_within_keeps_higher_importance() {
        let items = vec![
            DistilledItem {
                content: "采用 RwLock 提升读并发".into(),
                memory_type: "decision".into(),
                tags: vec!["distilled".into()],
                importance: 3,
                source_turns: (0, 0),
            },
            DistilledItem {
                content: "采用 RwLock 提升读并发。".into(),
                memory_type: "decision".into(),
                tags: vec!["auto-summary".into()],
                importance: 5,
                source_turns: (0, 0),
            },
        ];
        let (kept, dup) = dedupe_within(items);
        assert_eq!(kept.len(), 1);
        assert_eq!(dup, 1);
        assert_eq!(kept[0].importance, 5);
    }

    #[test]
    fn filter_marks_auto_summary_and_drops_unknown_type() {
        let items = vec![
            DistilledItem {
                content: "决定采用 pnpm 作为包管理器".into(),
                memory_type: "decision".into(),
                tags: vec!["distilled".into()],
                importance: 6,
                source_turns: (1, 1),
            },
            DistilledItem {
                content: "随便聊聊".into(),
                memory_type: "chat".into(),
                tags: vec![],
                importance: 1,
                source_turns: (1, 1),
            },
        ];
        let (kept, dropped) = filter_distilled(items, "正常文本");
        assert_eq!(kept.len(), 1);
        assert!(kept[0].tags.iter().any(|t| t == "auto-summary"));
        assert_eq!(dropped.len(), 1);
    }

    #[test]
    fn filter_aborts_on_deletion_intent() {
        let items = vec![DistilledItem {
            content: "决定采用 pnpm".into(),
            memory_type: "decision".into(),
            tags: vec![],
            importance: 5,
            source_turns: (1, 1),
        }];
        let (kept, dropped) = filter_distilled(items, "已删除该记忆");
        assert!(kept.is_empty());
        assert_eq!(dropped, vec!["deletion_guard".to_string()]);
    }
}
