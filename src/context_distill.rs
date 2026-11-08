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
    /// 单次最多留存几条"整轮报错日志"（默认 10）
    pub max_error_blocks: usize,
}

impl Default for DistillConfig {
    fn default() -> Self {
        Self {
            threshold: 2.0,
            max_items: 40,
            min_chars: 6,
            max_chars: 300,
            max_error_blocks: 10,
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
    "决策",
    "采用",
    "改用",
    "统一用",
    "decided to",
    "let's use",
    "switch to",
    "go with",
    "we'll use",
];

/// 约定词：陈述"我们这里就是这么定的"，不带决策动词也不带否定。
///
/// 为什么需要单独一张表：真实对话里大量约定是**陈述句**——
/// 「LRC 的端口纪律是 3099 正式、3111 测试」既没有"决定/采用"，
/// 也没有"禁止/不要"，按旧打分只有专有名词 1.0 分，被 2.0 阈值挡掉。
/// 这类句子恰恰是跨会话最该记住的东西，故与决策词同权重。
///
/// 刻意不收裸 `标准` / `规则`：`标准库`、`规则引擎` 会把普通技术名词拉进来。
const CONVENTION_WORDS: &[&str] = &[
    "纪律",
    "约定",
    "规范",
    "口径",
    "原则",
    "一律",
    "规定",
    "默认是",
    "默认用",
    "默认值",
    "默认端口",
    "统一为",
    "固定为",
    "要求是",
    "convention",
    "by default",
];

/// 弱约定词：语义上确实表达"约定"，但在技术文本里出现频率极高
/// （"必须注意"、"always check"），单独命中不足以证明这是一条可复用约定。
///
/// 故与强约定词分开计权（+1.0 而非 +2.0）：单独出现时仍需专有名词等
/// 其它信号才能过阈值，避免把普通技术句大量收进记忆库。
const WEAK_CONVENTION_WORDS: &[&str] = &["必须", "应当", "must", "always"];

/// 否定约束词：出现即说明"这条路被排除了"。
///
/// 代码场景里 `Don't` / `shouldn't` / `never` 常单独出现（不带 `use`），
/// 早期词表只收 `don't use`，导致 "Don't commit secrets" 这类高危约束被漏掉。
const NEGATION_WORDS: &[&str] = &[
    "不能用",
    "不要用",
    "不要",
    "不能",
    "禁止",
    "严禁",
    "切勿",
    "不得",
    "避免",
    "别用",
    "放弃",
    "不允许",
    "不再",
    "杜绝",
    "don't",
    "do not",
    "shouldn't",
    "should not",
    "mustn't",
    "must not",
    "never",
    "avoid",
    "not allowed",
    "forbidden",
    "prohibited",
    "deprecated",
];

/// 强报错标记：出现即说明这一轮里有**真实的错误日志**，必须留存。
///
/// 刻意只收"无歧义"的标记（`panic` / `traceback` / `error:` 等），
/// 不收 `error` / `失败` 这类泛词 —— 它们在开发对话里出现频率太高，
/// 收进来会把记忆库刷成噪声。
const ERROR_MARKERS: &[&str] = &[
    // 刻意用 `panic:` / `panicked` / `panic at` 而不是裸 `panic`：
    // 裸词会把 "Avoid panics inside the handler" 这类**约束句**误判成报错日志。
    "panic:",
    "panicked",
    "panic at",
    "traceback",
    "stack trace",
    "stacktrace",
    "segmentation fault",
    "segfault",
    "error:",
    "exception:",
    "assertion failed",
    "assertion `left",
    "exit code",
    "exit status",
    "报错",
    "崩溃",
    "异常退出",
    "编译错误",
    "运行时错误",
    "错误信息",
    "堆栈",
    "调用栈",
];

/// 结论词：出现即说明"这是本轮得出的判断"。
const CONCLUSION_WORDS: &[&str] = &[
    "结论是",
    "结论",
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

    // ① 轮次级：整块报错日志 → **整块**留存。
    //
    // 为什么不在句级做：报错日志天然跨多行（`panic: ...` 后面跟调用栈），
    // 逐句切会把上下文切碎，留下的每一行都不可读。整块留存才有回溯价值。
    for (line_idx, block) in extract_error_blocks(text, config.max_error_blocks, config.max_chars) {
        let key = normalize_key(&block);
        if seen.iter().any(|k| k == &key) {
            continue;
        }
        seen.push(key);
        scored.push((
            4.0,
            line_idx,
            DistilledItem {
                content: format!("[{}] {}", Kind::Error.label(), block),
                memory_type: Kind::Error.memory_type().to_string(),
                tags: vec![Kind::Error.tag().to_string(), "distilled".to_string()],
                // 报错是排障的第一手证据，固定给高分
                importance: 8,
                source_turns: (line_idx, line_idx),
            },
        ));
    }

    // ② 句级：决策 / 约束 / 结论 / 代码坐标
    for (turn_idx, turn) in turns.iter().enumerate() {
        // 显式声明（`结论：`/`决策：`/`约束：`）整行作为一个单元，**不切句**。
        //
        // 切句会把「结论：A？B」拆成「结论：A？」+「B」，后者因无关键词被丢弃；
        // 「结论：A。B」同理。显式声明是作者主动打的标签，整行都应保留，
        // 否则正文兜底路径会静默丢失后半段（实测 DSH 损失 40-55% 字符）。
        if let Some(kind) = explicit_kind(turn) {
            let trimmed = turn.trim();
            let char_count = trimmed.chars().count();
            if char_count >= config.min_chars && !is_markdown_noise(trimmed) {
                let key = normalize_key(trimmed);
                if !seen.iter().any(|k| k == &key) {
                    seen.push(key);
                    let body = truncate_chars(trimmed, config.max_chars);
                    let mut tags = vec![kind.tag().to_string(), "distilled".to_string()];
                    if contains_any(trimmed, ANCHOR_WORDS) {
                        tags.push("anchor".to_string());
                    }
                    scored.push((
                        3.0,
                        turn_idx,
                        DistilledItem {
                            content: format!("[{}] {}", kind.label(), body),
                            memory_type: kind.memory_type().to_string(),
                            tags,
                            importance: importance_from_score(3.0),
                            source_turns: (turn_idx, turn_idx),
                        },
                    ));
                }
            }
            continue;
        }
        for sentence in split_sentences(turn) {
            let trimmed = sentence.trim();
            let char_count = trimmed.chars().count();
            if char_count < config.min_chars {
                continue;
            }
            // 排版噪声先剔掉：表格行 / 标题 / 引用块 / 分隔线 / 纯链接行。
            // 规则式提炼分不清「结论」和「报告里的引用块」，实测一次沉淀
            // 40 条里大半是 markdown 表格行和链接。
            if is_markdown_noise(trimmed) {
                continue;
            }
            // 编号列表项（`1. …` / `2) …`）默认视为待办/选项清单，不是结论。
            // 例外：含强否定词或约定词的编号项仍保留
            //（"1. 禁止把密钥写进代码" / "1. 必须用 pnpm"）。
            if is_numbered_list_item(trimmed)
                && !has_strong_negation(trimmed)
                && count_hits(&trimmed.to_lowercase(), CONVENTION_WORDS) == 0
                && count_hits(&trimmed.to_lowercase(), WEAK_CONVENTION_WORDS) == 0
            {
                continue;
            }
            // 打分前先剥掉 markdown 链接：链接里的路径会被算成"专有名词"，
            // 让 `见 [index.ts](G:/...)` 这类行凭空多 1 分。
            let mut scoring_text = strip_markdown_links(trimmed);
            if scoring_text.trim().is_empty() {
                scoring_text = trimmed.to_string();
            }
            let (score, kind) = score_sentence(&scoring_text);
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
    /// 客观事实（含「过程叙述冒充结论」被降级的条目）
    Fact,
    /// 整轮报错日志（由 `ERROR_MARKERS` 在轮次级识别，不走逐句打分）
    Error,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Decision => "决策",
            Kind::Constraint => "约束",
            Kind::Conclusion => "结论",
            Kind::CodeContext => "代码",
            Kind::Fact => "事实",
            Kind::Error => "报错",
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Kind::Decision => "decision",
            Kind::Constraint => "constraint",
            Kind::Conclusion => "conclusion",
            Kind::CodeContext => "code",
            Kind::Fact => "fact",
            Kind::Error => "error",
        }
    }

    fn memory_type(self) -> &'static str {
        match self {
            Kind::Decision => "decision",
            Kind::Constraint => "preference",
            Kind::Conclusion => "fact",
            Kind::CodeContext => "code_context",
            Kind::Fact => "fact",
            // 报错是客观事实，且需要能被"上次那个报错"检索到
            Kind::Error => "fact",
        }
    }
}

/// Markdown 排版噪声：表格行、标题、引用块、分隔线、代码围栏、纯链接行。
///
/// 为什么需要：真实对话里 AI 会输出大量报告式排版，规则式提炼分不清
/// 「结论」和「报告里的引用块」—— 实测一次沉淀 40 条里大半是
/// `| 1 | LRC 服务连通 | ✅ |` 这类表格行和 `[index.ts](D:/...)` 这类链接。
///
/// 刻意**不**过滤列表项（`- xxx` / `1. xxx`）：列表里经常装着真正的约束。
fn is_markdown_noise(sentence: &str) -> bool {
    let t = sentence.trim();
    if t.is_empty() {
        return true;
    }
    // 标题 / 引用块 / 代码围栏
    if t.starts_with('#') || t.starts_with('>') || t.starts_with("```") {
        return true;
    }
    // 表格行：至少两个竖线，且**行首或行尾**也是竖线。
    //
    // 只数竖线会把 `cat a | grep b | wc -l` 这类 shell 管道句误判成表格，
    // 而真正的 markdown 表格行几乎都以 `|` 开头或结尾。
    if t.matches('|').count() >= 2 && (t.starts_with('|') || t.ends_with('|')) {
        return true;
    }
    // 分隔线：只由 - * _ 和空格组成，且长度 >= 3
    if t.chars().count() >= 3 && t.chars().all(|c| matches!(c, '-' | '*' | '_' | ' ')) {
        return true;
    }
    // 纯链接行：去掉 markdown 链接后几乎没有文字
    if t.contains("](") {
        let stripped = strip_markdown_links(t);
        if stripped.chars().filter(|c| c.is_alphanumeric()).count() < 4 {
            return true;
        }
    }
    false
}

/// 编号列表项：`1. xxx` / `2) xxx` / `3、xxx`。
///
/// 报告里的编号列表绝大多数是"待办 / 选项 / 步骤"，不是可复用结论。
/// 实测一次沉淀 40 条里 12 条是这类片段。
fn is_numbered_list_item(s: &str) -> bool {
    let t = s.trim_start();
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return false;
    }
    let rest = &t[digits.len()..];
    let mut it = rest.chars();
    match it.next() {
        Some('.') | Some(')') | Some('、') => it.next().map_or(true, |c| c.is_whitespace()),
        _ => false,
    }
}

/// 强否定词：编号列表项里出现这些词时仍按约束保留。
fn has_strong_negation(s: &str) -> bool {
    const WORDS: &[&str] = &[
        "禁止",
        "严禁",
        "不得",
        "不要",
        "不能",
        "切勿",
        "杜绝",
        "不允许",
    ];
    WORDS.iter().any(|w| s.contains(w))
}

/// 去掉 `[text](url)` 形式的链接，只留链接文字之外的正文。
fn strip_markdown_links(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            if let Some(close) = (i..chars.len()).find(|&j| chars[j] == ']') {
                if close + 1 < chars.len() && chars[close + 1] == '(' {
                    if let Some(end) = (close + 2..chars.len()).find(|&j| chars[j] == ')') {
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 过程叙述词：出现这些词说明句子在描述「接下来要做什么」，不是已完成的结论。
///
/// 实测 DSH 会话里 AI 会把「接下来我要确认工具参数存放在哪里」写成 `- 结论：…`，
/// 抽取器按行首标签直接挂 `[结论]`，导致结论计数约 10% 虚高。
const PROCESS_NARRATION_WORDS: &[&str] = &[
    "接下来",
    "下一步",
    "我要",
    "待确认",
    "待验证",
    "还需要",
    "准备",
    "打算",
    "将要",
];

/// 完成时态词：出现这些词说明句子已经得出结论，即使含过程词也不降级。
const COMPLETION_WORDS: &[&str] = &[
    "已完成",
    "已确认",
    "已验证",
    "已修复",
    "结论是",
    "因此",
    "所以",
    "说明",
    "表明",
];

/// 是否是「过程叙述冒充结论」：含过程词且不含完成词。
fn is_process_narration(body: &str) -> bool {
    let lower = body.to_lowercase();
    if !PROCESS_NARRATION_WORDS.iter().any(|w| lower.contains(w)) {
        return false;
    }
    !COMPLETION_WORDS.iter().any(|w| lower.contains(w))
}

/// 行首显式声明：`决策：…` / `约束：…` / `结论：…`。
///
/// 这是作者主动打的标签，比关键词猜测可靠得多 —— 一句话里出现"约束"二字
/// 不代表它是约束，但行首写"约束："就是明确声明。
/// 支持列表前缀（`- ` / `* ` / `+ `）与全角/半角冒号。
fn explicit_kind(sentence: &str) -> Option<Kind> {
    let t = sentence
        .trim_start()
        .trim_start_matches(['-', '*', '+'])
        .trim_start();
    for (prefix, kind) in [
        ("决策", Kind::Decision),
        ("决定", Kind::Decision),
        ("约束", Kind::Constraint),
        ("结论是", Kind::Conclusion),
        ("结论", Kind::Conclusion),
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let rest = rest.trim_start();
            if rest.starts_with('：') || rest.starts_with(':') {
                let body = rest.trim_start_matches(['：', ':']).trim();
                // 「结论：接下来我要…」是过程叙述，不是结论 → 降级为事实
                if kind == Kind::Conclusion && is_process_narration(body) {
                    return Some(Kind::Fact);
                }
                return Some(kind);
            }
        }
    }
    None
}

/// 打分并判定类别。
///
/// 类别优先级：决策 > 约束 > 结论 > 代码。
/// 理由：一句话同时含"决定"和"所以"时，它首先是决策，结论只是修饰；
/// 含"结论是"又带文件路径时，它是结论（路径只是细节）。
fn score_sentence(sentence: &str) -> (f32, Kind) {
    // 行首显式声明优先：作者已经标了类别，不再靠关键词猜。
    // 固定给 3.0（高于阈值 2.0），避免"结论：…"因为只有一个结论词被挡掉。
    if let Some(kind) = explicit_kind(sentence) {
        return (3.0, kind);
    }

    let lower = sentence.to_lowercase();
    let decision_hits = count_hits(&lower, DECISION_WORDS);
    let convention_hits = count_hits(&lower, CONVENTION_WORDS);
    let weak_convention_hits = count_hits(&lower, WEAK_CONVENTION_WORDS);
    let negation_hits = count_hits(&lower, NEGATION_WORDS);
    let conclusion_hits = count_hits(&lower, CONCLUSION_WORDS);
    let proper_hits = count_proper_nouns(sentence);
    let is_question = is_pure_question(sentence);

    let mut score = 0.0_f32;
    score += 2.0 * decision_hits as f32;
    score += 2.0 * convention_hits as f32;
    // 弱约定词降权：单独出现不足以过阈值（见 WEAK_CONVENTION_WORDS 文档）
    score += 1.0 * weak_convention_hits as f32;
    score += 2.0 * negation_hits as f32;
    score += 1.0 * conclusion_hits as f32;
    score += 1.0 * proper_hits as f32;
    if is_question {
        score -= 1.0;
    }

    let kind = if decision_hits > 0 {
        Kind::Decision
    } else if negation_hits > 0 || convention_hits > 0 || weak_convention_hits > 0 {
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
                // 只有「以疑问词开头」的段才在问号处切分。
                //
                // 陈述句里出现的问号（如「结论：A？B」）**不切**：切了会把后半段
                // 当成独立句，因无关键词被丢弃，导致显式声明的后半段静默丢失
                // （实测 DSH 正文兜底路径损失 40-55% 字符）。
                // 纯疑问句（「为什么 X？」）仍会被 `is_pure_question` 在打分时识别并降分。
                let head = buf.trim_end_matches(['？', '?']).trim().to_lowercase();
                if QUESTION_PREFIXES.iter().any(|p| head.starts_with(p)) {
                    out.push(buf.trim().to_string());
                    buf.clear();
                }
            }
            _ => buf.push(ch),
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

/// 从原文中抽取"报错块"：命中标记的行 + 其后最多 11 行（遇到空行即止）。
///
/// 返回 `(起始行号, 块文本)`，行号用于 `source_turns` 回溯。
///
/// 为什么按行而不是按"轮"：纯文本输入下 `split_turns` 是逐行切的，
/// 一条 `panic:` 加三行调用栈会被切成四个"轮"，按轮取只能拿到第一行。
fn extract_error_blocks(text: &str, max_blocks: usize, max_chars: usize) -> Vec<(usize, String)> {
    const MAX_BLOCK_LINES: usize = 12;
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < lines.len() && out.len() < max_blocks {
        let lower = lines[i].to_lowercase();
        if !contains_any(&lower, ERROR_MARKERS) {
            i += 1;
            continue;
        }
        let start = i;
        let mut block = String::new();
        let mut j = i;
        while j < lines.len() && j < start + MAX_BLOCK_LINES {
            if j > start && lines[j].trim().is_empty() {
                break;
            }
            if !block.is_empty() {
                block.push('\n');
            }
            block.push_str(lines[j]);
            j += 1;
        }
        out.push((start, truncate_chars(block.trim(), max_chars)));
        i = j.max(start + 1);
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

    /// 陈述式约定必须能过阈值。
    ///
    /// 回归背景：真实 DSH 对话里「LRC 的端口纪律是 3099 正式、3111 测试」
    /// 既无决策动词也无否定词，旧打分只有专有名词 1.0 分 ⇒ 被 2.0 阈值挡掉，
    /// 用户必须把话说成「决策 / 约束」结构才能被记住。
    #[test]
    fn declarative_convention_passes_threshold() {
        for text in [
            "LRC 的端口纪律是 3099 正式、3111 测试。",
            "默认端口是 3099。",
            "提交信息一律用中文。",
            "By default we keep the cache under ~/.loong-recall.",
        ] {
            let items = distill_transcript(text, &cfg());
            assert_eq!(
                items.len(),
                1,
                "陈述式约定应产出：{:?} -> {:?}",
                text,
                items
            );
            assert_eq!(items[0].memory_type, "preference", "输入 {:?}", text);
        }
    }

    /// Markdown 排版噪声不得被当成结论。
    ///
    /// 回归背景：真实 DSH 会话里 AI 输出报告式排版，规则式提炼把
    /// `| 1 | LRC 服务连通 | ✅ |` 判成「代码」、把 `[index.ts](D:/...)` 判成「代码」，
    /// 一次沉淀 40 条里大半是这类噪声。
    #[test]
    fn drops_markdown_noise() {
        for text in [
            "| 1 | LRC 服务连通 | ✅ | `3099/health` → 200 |",
            "## 四、要不要动手",
            "> 引用块里的结论",
            "---",
            "[index.ts](D:/Users/.dsh/profiles/desktop/node_modules/dsh-plugin-lrc/src/index.ts#L63)",
        ] {
            let items = distill_transcript(text, &cfg());
            assert!(items.is_empty(), "排版噪声不应产出：{:?} -> {:?}", text, items);
        }
    }

    /// 列表项里的真实约束不能被误杀。
    #[test]
    fn keeps_list_item_constraints() {
        let text = "- 禁止在 3099 上跑测试数据。";
        let items = distill_transcript(text, &cfg());
        assert_eq!(items.len(), 1, "列表项里的约束应保留：{:?}", items);
    }

    /// 编号列表项默认丢弃，但含强否定词的保留。
    ///
    /// 回归背景：真实 DSH 会话一次沉淀 40 条里 12 条是 `1. …` `2. …` 这类
    /// 待办/选项片段，写进库后完全不可复用。
    #[test]
    fn drops_numbered_list_items_except_constraints() {
        for text in [
            "3. `index` 那处建议先在 LRC 侧确认语义再动",
            "2. 顺手把 `snapshotItems`、提取粒度调一调，避免一次写满 40 条",
            "1. 把 `config.baseUrl` 接入 `LrcRuntime`，让端口切换真正生效",
        ] {
            let items = distill_transcript(text, &cfg());
            assert!(
                items.is_empty(),
                "编号待办不应产出：{:?} -> {:?}",
                text,
                items
            );
        }
        let kept = distill_transcript("1. 禁止把密钥写进代码。", &cfg());
        assert_eq!(kept.len(), 1, "编号项里的强约束应保留：{:?}", kept);
    }

    /// 行首显式声明必须直接采信，不能被正文关键词劫持。
    ///
    /// 回归背景：`结论：itemsFromTag 按行分类，约束→preference` 因为正文里
    /// 出现「约束」二字被误判成约束；`决策：…` 因为词表里只有「决定」被整条丢弃。
    #[test]
    fn explicit_prefix_wins_over_keywords() {
        let items = distill_transcript(
            "结论：itemsFromTag 按行分类，约束→preference、决策→decision",
            &cfg(),
        );
        assert_eq!(items.len(), 1, "行首「结论：」应保留：{:?}", items);
        assert!(items[0].content.starts_with("[结论]"), "{:?}", items);
        assert_eq!(items[0].memory_type, "fact");

        let items = distill_transcript("决策：本次仅为连通性测试", &cfg());
        assert_eq!(items.len(), 1, "行首「决策：」应保留：{:?}", items);
        assert!(items[0].content.starts_with("[决策]"), "{:?}", items);

        let items = distill_transcript("- 约束：禁止连 3099 做测试", &cfg());
        assert_eq!(
            items.len(),
            1,
            "列表前缀 + 行首「约束：」应保留：{:?}",
            items
        );
        assert!(items[0].content.starts_with("[约束]"), "{:?}", items);
    }

    /// 回归：AI 把「接下来我要…」写成 `- 结论：…` 时，必须降级为事实，
    /// 否则结论计数虚高（实测约 10%）。
    #[test]
    fn process_narration_under_conclusion_prefix_downgrades_to_fact() {
        let items = distill_transcript(
            "- 结论：接下来我要确认工具参数和推理内容究竟存放在哪里",
            &cfg(),
        );
        assert_eq!(items.len(), 1, "{:?}", items);
        assert!(items[0].content.starts_with("[事实]"), "{:?}", items);
        assert_eq!(items[0].memory_type, "fact");

        // 含完成时态词时不降级
        let items = distill_transcript("- 结论：已完成迁移，因此必须分块", &cfg());
        assert_eq!(items.len(), 1, "{:?}", items);
        assert!(items[0].content.starts_with("[结论]"), "{:?}", items);

        // 过程词 + 完成词同时出现：以完成词为准，不降级
        let items = distill_transcript("- 结论：接下来我要验证，已验证通过", &cfg());
        assert_eq!(items.len(), 1, "{:?}", items);
        assert!(items[0].content.starts_with("[结论]"), "{:?}", items);
    }

    /// 回归：显式声明整行保留，不因句末标点被切掉后半段。
    ///
    /// 实测 DSH 正文兜底路径：`结论：A？B` 被切成 `结论：A？` + `B`，
    /// `B` 因无关键词被丢弃，真实句式下损失 40-55% 字符。
    #[test]
    fn explicit_prefix_preserves_whole_line_across_punctuation() {
        // 问号：后半段必须保留
        let items = distill_transcript("结论：为什么选 A？因为 B 更稳定 TAILQ1", &cfg());
        assert_eq!(items.len(), 1, "{:?}", items);
        assert!(
            items[0].content.contains("TAILQ1"),
            "问号后内容丢失：{:?}",
            items
        );

        // 句号：后半段必须保留
        let items = distill_transcript("结论：方案 A 可行。补充说明 TAILP2", &cfg());
        assert_eq!(items.len(), 1, "{:?}", items);
        assert!(
            items[0].content.contains("TAILP2"),
            "句号后内容丢失：{:?}",
            items
        );

        // 分号：后半段必须保留
        let items = distill_transcript("决策：用 pnpm；理由 TAILS3", &cfg());
        assert_eq!(items.len(), 1, "{:?}", items);
        assert!(
            items[0].content.contains("TAILS3"),
            "分号后内容丢失：{:?}",
            items
        );
    }

    /// 回归：陈述句里的问号不应把句子切开（无显式前缀时）。
    #[test]
    fn question_mark_mid_statement_does_not_split() {
        // 无显式前缀，但含「决定」关键词，整句应作为一个单元保留
        let items = distill_transcript("我们决定用 A？因为 B 更稳定 TAILQ9", &cfg());
        assert_eq!(items.len(), 1, "{:?}", items);
        assert!(
            items[0].content.contains("TAILQ9"),
            "问号后内容丢失：{:?}",
            items
        );
    }

    /// 行内 markdown 链接不应把普通句子抬过阈值。
    #[test]
    fn inline_link_does_not_inflate_score() {
        let text = "见 [index.ts](G:/dsh-plugin-lrc/src/index.ts#L63)";
        let items = distill_transcript(text, &cfg());
        assert!(items.is_empty(), "纯链接引用不应产出：{:?}", items);
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

    /// 代码场景的短否定（不带 `use`）必须被捕获。
    ///
    /// 回归背景：词表原只有 `don't use`，导致 "Don't commit secrets" 这类
    /// 高危约束整条漏掉。
    #[test]
    fn short_negations_are_captured() {
        for text in [
            "Don't commit secrets to the repo.",
            "You shouldn't log the API key.",
            "Never hardcode the token.",
            "严禁把密钥写进代码。",
            "不得跳过鉴权中间件。",
        ] {
            let items = distill_transcript(text, &cfg());
            assert_eq!(items.len(), 1, "短否定应产出：{:?} -> {:?}", text, items);
            assert_eq!(items[0].memory_type, "preference", "输入 {:?}", text);
        }
    }

    /// 报错日志：整轮留存，且固定高重要性。
    #[test]
    fn keeps_error_log_as_whole_turn() {
        let text =
            "panic: index out of bounds\nnote: run with RUST_BACKTRACE=1\n  at src/main.rs:12";
        let items = distill_transcript(text, &cfg());
        assert_eq!(items.len(), 1, "整轮报错应产出 1 条，实际 {:?}", items);
        assert_eq!(items[0].memory_type, "fact");
        assert!(items[0].content.starts_with("[报错]"));
        assert_eq!(items[0].importance, 8);
        assert!(items[0].tags.contains(&"error".to_string()));
        // 整轮留存：三行都在
        assert!(items[0].content.contains("RUST_BACKTRACE"));
        assert!(items[0].content.contains("src/main.rs:12"));
    }

    /// 中英文报错标记都要能识别。
    #[test]
    fn recognizes_error_markers_both_languages() {
        for text in [
            "Traceback (most recent call last):\n  File \"a.py\", line 3",
            "报错：E0382 use of moved value",
            "编译错误：cannot find value `x` in this scope",
            "Error: connection refused",
        ] {
            let items = distill_transcript(text, &cfg());
            assert!(
                items.iter().any(|i| i.tags.contains(&"error".to_string())),
                "应识别为报错：{:?} -> {:?}",
                text,
                items
            );
        }
    }

    /// 报错块数量受 `max_error_blocks` 限制，防止刷屏。
    ///
    /// 注意：**连续**的报错行会被合并成一块（调用栈就是这样），
    /// 所以这里用空行把 20 条报错隔开，才能验证"块数上限"。
    #[test]
    fn caps_error_blocks() {
        let mut text = String::new();
        for i in 0..20 {
            text.push_str(&format!("panic: boom {}\n\n", i));
        }
        let config = DistillConfig {
            max_error_blocks: 3,
            ..DistillConfig::default()
        };
        let items = distill_transcript(&text, &config);
        let errors = items
            .iter()
            .filter(|i| i.tags.contains(&"error".to_string()))
            .count();
        assert_eq!(errors, 3, "报错块应被限制为 3 条，实际 {:?}", items);
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
