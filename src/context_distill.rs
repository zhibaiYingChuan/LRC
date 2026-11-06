//! 对话提炼规则引擎（P1-1，Layer 1，零外部依赖）。
//!
//! ## 职责
//!
//! 把一段对话文本（Claude Code 的 JSONL transcript，或纯文本）切成候选片段，
//! 判定哪些片段值得长期留存，产出 [`DistilledItem`] 供 `/v1/context/migrate` 写入。
//!
//! ## 两阶段设计（与推进文档 P1-1 一致）
//!
//! 1. **强规则切分**：按句末标点切句，纯规则、零成本；
//! 2. **规则打分**：命中决策 / 否定 / 结论词加分，含专有名词加分，纯疑问减分。
//!    未启用模型时这就是最终判定 —— 保证离线可用、不引入新依赖。
//!
//! ## 打分公式（推进文档 P1-1 的公式，三处细化已回写文档）
//!
//! ```text
//! score = 2.0 * 决策词命中数 + 2.0 * 否定词命中数 + 1.0 * 结论词命中数
//!       + 1.0 * 专有名词命中数（反引号 / 文件路径 / 大写标识符，上限 2）
//!       - 1.0 * 纯疑问句
//! 阈值：score >= 2.0 才产出
//! ```
//!
//! **细化 1：按"命中数"而非"是否命中"计分。**
//! 若按布尔计分，否定词与结论词单独出现永远过不了 2.0 阈值，
//! `preference` / `fact` 两类记忆将永远产不出来 —— 实测确认过这个死代码问题。
//!
//! **细化 2：专有名词上限 2。**
//! `src/server.rs` 的 `build_mcp_router` 同时命中路径与标识符（1.0 + 1.0 = 2.0），
//! 这类"代码坐标"句应能独立过阈值；但不设上限会让长句靠堆标识符刷分。
//!
//! **细化 3：否定词权重 1.5 → 2.0。**
//! 真实对话抽检发现"禁止连 3099 做测试"这类**完整禁令**只有 1.5 分，被阈值挡掉。
//! 一条禁令本身就是完整约束，强度与"决定用 X"同级，故提到 2.0。
//!
//! **重叠抑制**：词表里"决定用"与"决定"并存，按词长降序匹配并抹除已命中片段，
//! 保证同一处文本只计一次（否则"决定用 pnpm"会被算成 4.0 分）。
//!
//! **指代锚点词**（"刚才那个""上次说的"）**不计分**，只打 `anchor` 标签。
//! 理由：锚点句单独看没有信息量（"刚才那个方案"），它的价值是提示"这里需要回溯"，
//! 给它加分会让大量无内容的指代句混进记忆库。
//!
//! ## 边界
//!
//! - 本模块**不写库**、不访问网络、不读文件。输入是 `&str`，输出是 `Vec<DistilledItem>`。
//! - 误判的代价由调用方控制：`importance` 随分数递增，低分条目可被人工清理。

use serde::{Deserialize, Serialize};

/// 一条提炼结果（尚未写入 LRC）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistilledItem {
    /// 结构化文本，如 `[决策] 用 pnpm（理由：解决兼容）`
    pub content: String,
    /// `decision` | `preference` | `fact` | `code_context`
    pub memory_type: String,
    /// 标签：类别 + `distilled`，命中指代锚点时追加 `anchor`
    pub tags: Vec<String>,
    /// 重要性 1-10，随分数递增
    pub importance: u8,
    /// 原文轮次区间 `(起, 止)`，便于回溯
    pub source_turns: (usize, usize),
}

/// 提炼配置。
#[derive(Debug, Clone)]
pub struct DistillConfig {
    /// 产出阈值（默认 2.0，与推进文档一致）
    pub threshold: f32,
    /// 单次最多产出条数（默认 40）
    pub max_items: usize,
    /// 片段最短字符数（默认 6，过滤"好的""嗯"这类）
    pub min_chars: usize,
    /// 片段最长字符数（默认 300，超长截断）
    pub max_chars: usize,
}

impl Default for DistillConfig {
    fn default() -> Self {
        Self {
            threshold: 2.0,
            max_items: 40,
            min_chars: 6,
            max_chars: 300,
        }
    }
}

// ============================================================
// 规则词表（`const` 数组，可整体替换；不做正则，避免引入依赖）
// ============================================================

/// 决策收束词：出现即说明"已经定下来了"。
const DECISION_WORDS: &[&str] = &[
    "决定用",
    "就用",
    "改成",
    "换成",
    "定为",
    "选型为",
    "最终采用",
    "决定",
    "采用",
    "改用",
    "统一用",
    "decided to",
    "let's use",
    "switch to",
    "go with",
    "we'll use",
];

/// 否定约束词：出现即说明"这条路被排除了"。
const NEGATION_WORDS: &[&str] = &[
    "不能用",
    "不要用",
    "禁止",
    "避免",
    "别用",
    "放弃",
    "不允许",
    "不再",
    "don't use",
    "do not use",
    "avoid",
    "must not",
    "never use",
];

/// 结论词：出现即说明"这是本轮得出的判断"。
const CONCLUSION_WORDS: &[&str] = &[
    "结论是",
    "所以",
    "因此",
    "最终",
    "确认",
    "总结",
    "综上",
    "therefore",
    "in conclusion",
    "so we",
];

/// 指代锚点词：只打标签，不计分（见模块文档）。
const ANCHOR_WORDS: &[&str] = &[
    "刚才那个",
    "上次说的",
    "前面提到的",
    "之前定的",
    "上次",
    "之前",
    "earlier",
    "last time",
];

/// 疑问句起始词：用于"纯疑问"判定。
///
/// **刻意不含 `is ` / `are ` / `can ` / `do ` 这类英文助动词**：
/// 它们会把祈使句误判成疑问句 —— "Do not use unwrap" 以 `do ` 开头，
/// 一旦被判为疑问就减 1.0 分，整条禁令被丢弃（实测踩过）。
/// 英文疑问句几乎都以 `?` 结尾，靠标点判定足够。
const QUESTION_PREFIXES: &[&str] = &[
    "什么",
    "为什么",
    "怎么",
    "如何",
    "是否",
    "能不能",
    "有没有",
    "哪",
    "多少",
    "何时",
    "谁",
    "what",
    "why",
    "how",
];

// ============================================================
// 对外入口
// ============================================================

/// 从对话文本中提炼结构化记忆。
///
/// 输入可以是：
/// - Claude Code 的 JSONL transcript（每行一个 JSON 对象）；
/// - 任意纯文本（每行视为一轮）。
///
/// 输出按原文顺序排列，最多 `config.max_items` 条。
pub fn distill_transcript(text: &str, config: &DistillConfig) -> Vec<DistilledItem> {
    let turns = split_turns(text);
    let mut scored: Vec<(f32, usize, DistilledItem)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for (turn_idx, turn) in turns.iter().enumerate() {
        for sentence in split_sentences(turn) {
            let trimmed = sentence.trim();
            let char_count = trimmed.chars().count();
            if char_count < config.min_chars {
                continue;
            }
            let (score, kind) = score_sentence(trimmed);
            if score < config.threshold {
                continue;
            }

            // 去重：同一结论在对话里常被复述多次，只留第一次出现的位置。
            let key = normalize_key(trimmed);
            if seen.iter().any(|k| k == &key) {
                continue;
            }
            seen.push(key);

            let body = truncate_chars(trimmed, config.max_chars);
            let mut tags = vec![kind.tag().to_string(), "distilled".to_string()];
            if contains_any(trimmed, ANCHOR_WORDS) {
                tags.push("anchor".to_string());
            }

            scored.push((
                score,
                turn_idx,
                DistilledItem {
                    content: format!("[{}] {}", kind.label(), body),
                    memory_type: kind.memory_type().to_string(),
                    tags,
                    importance: importance_from_score(score),
                    source_turns: (turn_idx, turn_idx),
                },
            ));
        }
    }

    // 超量时保留分数最高的 N 条，再按原文顺序输出（便于人工按对话顺序抽检）。
    if scored.len() > config.max_items {
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(config.max_items);
        scored.sort_by_key(|(_, idx, _)| *idx);
    }

    scored.into_iter().map(|(_, _, item)| item).collect()
}

/// 生成一行极简索引，用于替换宿主的原生摘要。
pub fn build_index_line(items: &[DistilledItem], project: &str) -> String {
    if items.is_empty() {
        return format!("前序对话未提炼出可留存结论（项目：{}）", project);
    }
    format!(
        "前序对话已提炼 {} 条至 LRC（项目：{}）",
        items.len(),
        project
    )
}

// ============================================================
// 内部实现
// ============================================================

/// 片段类别，决定 `memory_type` 与内容前缀。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Decision,
    Constraint,
    Conclusion,
    CodeContext,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Decision => "决策",
            Kind::Constraint => "约束",
            Kind::Conclusion => "结论",
            Kind::CodeContext => "代码",
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Kind::Decision => "decision",
            Kind::Constraint => "constraint",
            Kind::Conclusion => "conclusion",
            Kind::CodeContext => "code",
        }
    }

    fn memory_type(self) -> &'static str {
        match self {
            Kind::Decision => "decision",
            Kind::Constraint => "preference",
            Kind::Conclusion => "fact",
            Kind::CodeContext => "code_context",
        }
    }
}

/// 打分并判定类别。
///
/// 类别优先级：决策 > 约束 > 结论 > 代码。
/// 理由：一句话同时含"决定"和"所以"时，它首先是决策，结论只是修饰；
/// 含"结论是"又带文件路径时，它是结论（路径只是细节）。
fn score_sentence(sentence: &str) -> (f32, Kind) {
    let lower = sentence.to_lowercase();
    let decision_hits = count_hits(&lower, DECISION_WORDS);
    let negation_hits = count_hits(&lower, NEGATION_WORDS);
    let conclusion_hits = count_hits(&lower, CONCLUSION_WORDS);
    let proper_hits = count_proper_nouns(sentence);
    let is_question = is_pure_question(sentence);

    let mut score = 0.0_f32;
    score += 2.0 * decision_hits as f32;
    score += 2.0 * negation_hits as f32;
    score += 1.0 * conclusion_hits as f32;
    score += 1.0 * proper_hits as f32;
    if is_question {
        score -= 1.0;
    }

    let kind = if decision_hits > 0 {
        Kind::Decision
    } else if negation_hits > 0 {
        Kind::Constraint
    } else if conclusion_hits > 0 {
        Kind::Conclusion
    } else {
        Kind::CodeContext
    };

    (score, kind)
}

/// 统计词表命中数（**重叠抑制**）。
///
/// 为什么需要重叠抑制：`DECISION_WORDS` 里同时有"决定用"和"决定"，
/// 朴素计数会把"决定用 pnpm"算成 2 次命中（4.0 分），虚高。
/// 这里按词长降序匹配，命中后把该词从工作串里抹掉，保证同一处文本只计一次。
fn count_hits(haystack: &str, words: &[&str]) -> usize {
    let mut work = haystack.to_string();
    let mut sorted: Vec<&str> = words.to_vec();
    sorted.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));

    let mut count = 0;
    for word in sorted {
        if work.contains(word) {
            count += 1;
            // 用 NUL 占位，避免被后续更短的词再次匹配。
            work = work.replace(word, "\u{0}");
        }
    }
    count
}

/// 统计专有名词命中数，上限 2。
///
/// 三类各计 1：反引号包裹、文件路径、大写开头的 ASCII 标识符。
/// 上限 2 的理由：`src/server.rs` 的 `build_mcp_router` 同时命中路径与标识符，
/// 这类"代码坐标"句应当能独立过阈值（1.0 + 1.0 = 2.0），但不应无限累加。
fn count_proper_nouns(sentence: &str) -> usize {
    let mut hits = 0;

    if sentence.contains('`') {
        hits += 1;
    }
    if (sentence.contains('/') || sentence.contains('\\')) && sentence.contains('.') {
        hits += 1;
    }
    let mut prev_is_boundary = true;
    let mut has_upper_ident = false;
    for ch in sentence.chars() {
        if ch.is_ascii_uppercase() && prev_is_boundary {
            has_upper_ident = true;
            break;
        }
        prev_is_boundary = !ch.is_alphanumeric();
    }
    if has_upper_ident {
        hits += 1;
    }

    hits.min(2)
}

/// 纯疑问句：以问号结尾，或以疑问词开头。
fn is_pure_question(sentence: &str) -> bool {
    let trimmed = sentence.trim();
    if trimmed.ends_with('?') || trimmed.ends_with('？') {
        return true;
    }
    let lower = trimmed.to_lowercase();
    QUESTION_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// 按句末标点切句。
///
/// 刻意**不**按英文句点 `.` 切分 —— 那会把 `src/server.rs` 切成两半。
///
/// 问号**保留在句尾**：`is_pure_question` 靠"以问号结尾"判定纯疑问，
/// 若切句时把问号丢掉，"我们为什么要决定用这个方案？"会被误判成陈述句
/// （实测踩过：该句曾以 2.0 分混进记忆库）。
fn split_sentences(turn: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for ch in turn.chars() {
        match ch {
            '。' | '！' | '!' | '；' | ';' | '\n' => {
                if !buf.trim().is_empty() {
                    out.push(buf.trim().to_string());
                }
                buf.clear();
            }
            '？' | '?' => {
                buf.push(ch);
                if !buf.trim().is_empty() {
                    out.push(buf.trim().to_string());
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

/// 切轮次：JSONL 逐行解析，纯文本逐行。
fn split_turns(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let looks_like_jsonl = lines
        .first()
        .map(|l| l.trim_start().starts_with('{'))
        .unwrap_or(false);

    if !looks_like_jsonl {
        return lines.iter().map(|l| l.to_string()).collect();
    }

    let mut turns = Vec::new();
    for line in lines {
        match serde_json::from_str::<serde_json::Value>(line.trim()) {
            Ok(value) => {
                if let Some(text) = extract_text_from_json(&value) {
                    if !text.trim().is_empty() {
                        turns.push(text);
                    }
                }
            }
            // 单行解析失败不致命：按纯文本处理，避免整段 transcript 丢失。
            Err(_) => turns.push(line.to_string()),
        }
    }
    turns
}

/// 从一行 transcript JSON 里取出正文。
///
/// 兼容三种常见形状：
/// - `{"type":"user","message":{"content":"..."}}`
/// - `{"type":"assistant","message":{"content":[{"type":"text","text":"..."}]}}`
/// - `{"role":"user","content":"..."}`
fn extract_text_from_json(value: &serde_json::Value) -> Option<String> {
    let message = value.get("message").unwrap_or(value);
    let content = message.get("content").or_else(|| value.get("content"))?;

    match content {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                    parts.push(t.to_string());
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

/// 去重键：去空白、转小写。
fn normalize_key(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// 按字符数截断（避免按字节切导致 UTF-8 边界 panic）。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// 分数 → 重要性。2.0 分给 6，5.0 分给 9，封顶 9。
fn importance_from_score(score: f32) -> u8 {
    let raw = 4.0 + score;
    raw.clamp(3.0, 9.0).round() as u8
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

// ============================================================
// 单测
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> DistillConfig {
        DistillConfig::default()
    }

    /// 中文决策句：应产出 decision，且内容带 `[决策]` 前缀。
    #[test]
    fn distills_chinese_decision() {
        let text = "我们讨论了一下，决定用 pnpm 替代 npm，因为依赖解析更快。";
        let items = distill_transcript(text, &cfg());
        assert_eq!(items.len(), 1, "应产出 1 条，实际 {:?}", items);
        assert_eq!(items[0].memory_type, "decision");
        assert!(items[0].content.starts_with("[决策]"));
        assert!(items[0].content.contains("pnpm"));
        assert!(items[0].importance >= 6);
    }

    /// 英文决策句：大小写不敏感。
    #[test]
    fn distills_english_decision() {
        let text = "We decided to switch to PostgreSQL for the transaction support.";
        let items = distill_transcript(text, &cfg());
        assert_eq!(items.len(), 1, "应产出 1 条，实际 {:?}", items);
        assert_eq!(items[0].memory_type, "decision");
    }

    /// 闲聊：不含任何规则词，应全部丢弃。
    #[test]
    fn drops_small_talk() {
        let text = "今天天气不错\n我先去吃个饭\n好的收到";
        let items = distill_transcript(text, &cfg());
        assert!(items.is_empty(), "闲聊不应产出记忆，实际 {:?}", items);
    }

    /// 纯疑问：即使含决策词，也应因减分被丢弃。
    #[test]
    fn drops_pure_question() {
        let text = "我们为什么要决定用这个方案？";
        let items = distill_transcript(text, &cfg());
        assert!(items.is_empty(), "纯疑问不应产出记忆，实际 {:?}", items);
    }

    /// 含文件路径的结论：应识别为 fact（结论优先于代码坐标），并保留路径原文。
    #[test]
    fn keeps_conclusion_with_file_path() {
        let text = "结论是：路由注册在 src/server.rs 的 build_mcp_router 里。";
        let items = distill_transcript(text, &cfg());
        assert_eq!(items.len(), 1, "应产出 1 条，实际 {:?}", items);
        assert_eq!(items[0].memory_type, "fact");
        assert!(items[0].content.starts_with("[结论]"));
        assert!(items[0].content.contains("src/server.rs"));
    }

    /// 纯代码坐标句（无决策/结论词）：靠"路径 + 标识符"两个专有名词过阈值。
    #[test]
    fn distills_code_coordinate() {
        let text = "src/server.rs 的 build_mcp_router 负责注册 MCP 路由。";
        let items = distill_transcript(text, &cfg());
        assert_eq!(items.len(), 1, "应产出 1 条，实际 {:?}", items);
        assert_eq!(items[0].memory_type, "code_context");
        assert!(items[0].content.starts_with("[代码]"));
    }

    /// 否定约束：应识别为 preference（约定/禁止）。
    #[test]
    fn distills_negation_constraint() {
        let text = "不要用 unwrap，禁止在请求路径上 panic。";
        let items = distill_transcript(text, &cfg());
        assert_eq!(items.len(), 1, "应产出 1 条，实际 {:?}", items);
        assert_eq!(items[0].memory_type, "preference");
        assert!(items[0].content.starts_with("[约束]"));
    }

    /// 单条完整禁令（无其他信号）必须能独立过阈值。
    ///
    /// 回归背景：否定词权重原为 1.5，真实对话抽检时"禁止连 3099 做测试"
    /// 这类完整禁令被 2.0 阈值挡掉 —— 禁令本身就是完整约束，不该被丢弃。
    #[test]
    fn standalone_prohibition_passes_threshold() {
        let text = "禁止连 3099 做测试。";
        let items = distill_transcript(text, &cfg());
        assert_eq!(items.len(), 1, "单条禁令应产出，实际 {:?}", items);
        assert_eq!(items[0].memory_type, "preference");
    }

    /// 英文祈使禁令不得被误判为疑问句。
    ///
    /// 回归背景：`QUESTION_PREFIXES` 原含 `do ` / `is ` / `can ` 等助动词，
    /// "Do not use unwrap" 以 `do ` 开头 ⇒ 判为疑问 ⇒ 减 1.0 分 ⇒ 整条禁令被丢弃。
    #[test]
    fn english_imperative_is_not_a_question() {
        for text in [
            "Do not use unwrap in request paths.",
            "Never use blocking IO on the async path.",
            "Avoid panics inside the request handler.",
        ] {
            let items = distill_transcript(text, &cfg());
            assert_eq!(items.len(), 1, "英文禁令应产出：{:?} -> {:?}", text, items);
            assert_eq!(items[0].memory_type, "preference", "输入 {:?}", text);
        }
    }

    /// JSONL transcript：应能取出 message.content 的文本。
    #[test]
    fn parses_jsonl_transcript() {
        let text = concat!(
            r#"{"type":"user","message":{"content":"我们决定用 RwLock 替换 Mutex。"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"因此读并发提升 67%，见 src/memory_store.rs。"}]}}"#
        );
        let items = distill_transcript(text, &cfg());
        assert_eq!(items.len(), 2, "应产出 2 条，实际 {:?}", items);
        assert_eq!(items[0].memory_type, "decision");
        assert_eq!(items[1].memory_type, "fact");
    }

    /// 去重：同一结论复述多次只留一条。
    #[test]
    fn dedups_repeated_conclusion() {
        let text = "决定用 pnpm。\n决定用 pnpm。\n决定用 pnpm。";
        let items = distill_transcript(text, &cfg());
        assert_eq!(items.len(), 1, "重复结论应去重，实际 {:?}", items);
    }

    /// 超量截断：保留分数最高的 N 条。
    #[test]
    fn caps_to_max_items() {
        let mut text = String::new();
        for i in 0..10 {
            text.push_str(&format!("决定用方案{}。\n", i));
        }
        let config = DistillConfig {
            max_items: 3,
            ..DistillConfig::default()
        };
        let items = distill_transcript(&text, &config);
        assert_eq!(items.len(), 3);
    }

    /// 指代锚点：只打标签，不因锚点词单独产出。
    #[test]
    fn anchor_word_tags_but_does_not_score() {
        let text = "刚才那个方案";
        let items = distill_transcript(text, &cfg());
        assert!(items.is_empty(), "锚点句单独不应产出，实际 {:?}", items);

        let text2 = "决定用刚才那个方案。";
        let items2 = distill_transcript(text2, &cfg());
        assert_eq!(items2.len(), 1);
        assert!(items2[0].tags.contains(&"anchor".to_string()));
    }

    /// 索引行：空结果与非空结果都要有可读文案。
    #[test]
    fn builds_index_line() {
        assert!(build_index_line(&[], "demo").contains("未提炼出"));
        let items = distill_transcript("决定用 pnpm。", &cfg());
        let line = build_index_line(&items, "demo");
        assert!(line.contains("1 条"));
        assert!(line.contains("demo"));
    }

    /// 超长片段按字符截断，不 panic（UTF-8 边界安全）。
    #[test]
    fn truncates_long_sentence_by_chars() {
        let long = format!("决定用{}。", "很长的理由".repeat(100));
        let config = DistillConfig {
            max_chars: 20,
            ..DistillConfig::default()
        };
        let items = distill_transcript(&long, &config);
        assert_eq!(items.len(), 1);
        // 正文截到 20 字符 + 省略号，再加 `[决策] ` 前缀（5 字符）。
        assert!(
            items[0].content.chars().count() <= 26,
            "实际 {} 字符：{}",
            items[0].content.chars().count(),
            items[0].content
        );
        assert!(items[0].content.ends_with('…'));
    }
}
