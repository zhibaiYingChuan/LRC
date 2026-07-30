// 通用代码/文档切分器
// =========================
// 支持多语言代码和通用文档切分。
// Phase 1 使用正则/缩进（行走骨架），Phase 2 替换为 tree-sitter AST。

use serde::{Deserialize, Serialize};
use std::path::Path;

/// 代码/文档片段
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeChunk {
    /// 唯一 ID（格式：文件路径:L起始行-L结束行）
    pub id: String,
    /// 相对于项目根目录的源文件路径
    pub file_path: String,
    /// 起始行号（1-based）
    pub start_line: usize,
    /// 结束行号（1-based）
    pub end_line: usize,
    /// 类型：fn | struct | class | def | section | paragraph ...
    pub chunk_type: String,
    /// 函数名/结构体名/类名/段落首行
    pub name: String,
    /// 签名行文本
    pub signature: String,
    /// 完整代码/文档文本
    pub content: String,
    /// 文档注释（可选）
    pub doc_comment: Option<String>,
    /// 源语言：rust | python | typescript | javascript | go | markdown | text ...
    pub language: String,
}

/// 代码切分器 trait
pub trait CodeChunker: Send + Sync {
    fn chunk_file(&self, file_path: &str, content: &str) -> Vec<CodeChunk>;

    fn chunk_batch(&self, files: &[(String, String)]) -> Vec<CodeChunk> {
        files
            .iter()
            .flat_map(|(path, content)| self.chunk_file(path, content))
            .collect()
    }
}

// ==================== 语言检测 ====================

/// 按文件扩展名检测语言类型
pub fn detect_language(file_path: &str) -> String {
    let path = Path::new(file_path);
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "javascript",
        Some("go") => "go",
        Some("md") | Some("mdx") => "markdown",
        Some("txt") | Some("rst") => "text",
        Some("yaml") | Some("yml") => "yaml",
        Some("toml") => "toml",
        Some("json") => "json",
        Some("html") | Some("htm") => "html",
        Some("css") | Some("scss") | Some("less") => "css",
        Some("xml") | Some("svg") => "xml",
        Some("sh") | Some("bash") | Some("zsh") => "shell",
        Some("sql") => "sql",
        _ => "text",
    }
    .to_string()
}

/// 检查文件是否支持索引（文本文件而非二进制）
pub fn is_supported_file(file_path: &Path) -> bool {
    matches!(
        file_path.extension().and_then(|e| e.to_str()),
        Some(
            "rs" | "py"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "mjs"
                | "cjs"
                | "go"
                | "md"
                | "mdx"
                | "txt"
                | "rst"
                | "yaml"
                | "yml"
                | "toml"
                | "json"
                | "html"
                | "htm"
                | "css"
                | "scss"
                | "less"
                | "xml"
                | "svg"
                | "sh"
                | "bash"
                | "zsh"
                | "sql"
                | "java"
                | "kt"
                | "swift"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "cc"
                | "cxx"
                | "hxx"
        )
    )
}

// ==================== 共享工具函数 ====================

/// 大括号深度匹配：从 start_idx 开始，找到匹配的闭合 }
fn find_brace_body_end(lines: &[&str], start_idx: usize) -> usize {
    let mut depth = 0i32;
    let mut found_open = false;
    let mut end_idx = start_idx;

    for (j, line) in lines.iter().enumerate().skip(start_idx) {
        for ch in line.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    found_open = true;
                }
                '}' => {
                    depth -= 1;
                }
                _ => {}
            }
        }
        if found_open && depth == 0 {
            end_idx = j;
            break;
        }
    }

    if !found_open {
        start_idx // 无大括号体，返回起始行
    } else {
        end_idx
    }
}

/// 提取 /// 或 /** */ 文档注释
fn extract_triple_slash_doc(lines: &[&str], start_idx: usize) -> Option<String> {
    if start_idx == 0 {
        return None;
    }
    let mut docs: Vec<String> = Vec::new();
    let mut i = start_idx.saturating_sub(1);
    loop {
        let line = lines.get(i)?.trim();
        if let Some(doc) = line.strip_prefix("///") {
            docs.push(doc.trim().to_string());
        } else if let Some(doc) = line.strip_prefix("//!") {
            docs.push(doc.trim().to_string());
        } else if line.is_empty() {
            // 跳过空行
        } else {
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    docs.reverse();
    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

/// 提取 # 或 /** 文档注释（Python/JS 风格）
fn extract_hash_or_jsdoc(lines: &[&str], start_idx: usize) -> Option<String> {
    if start_idx == 0 {
        return None;
    }
    let mut docs: Vec<String> = Vec::new();
    let mut i = start_idx.saturating_sub(1);

    // 先尝试 JSDoc /** ... */
    let line = lines.get(i)?.trim();
    if line.starts_with("/**") || line.starts_with(" * ") || line.starts_with(" */") {
        // 往上找 /** 开头
        while i > 0 {
            let l = lines[i].trim();
            if l.starts_with("/**") {
                break;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
        // 往下收集直到 */
        let mut j = i;
        while j < lines.len() {
            let l = lines[j].trim();
            if l.starts_with("/**") || l == "*/" {
                // skip markers
            } else if l.starts_with(" * ") {
                docs.push(l.strip_prefix(" * ").unwrap_or("").to_string());
            } else if l == "*" {
                // 跳过 JSDoc 中的空注释行
            } else {
                break;
            }
            j += 1;
        }
        if !docs.is_empty() {
            return Some(docs.join("\n"));
        }
    }

    // 然后尝试 # 注释（Python）
    i = start_idx.saturating_sub(1);
    docs.clear();
    loop {
        let line = lines.get(i)?.trim();
        if let Some(doc) = line.strip_prefix("# ") {
            docs.push(doc.to_string());
        } else if let Some(doc) = line.strip_prefix("#") {
            docs.push(doc.to_string());
        } else if line.is_empty() {
            // 跳过空行
        } else {
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    docs.reverse();
    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

// ==================== Rust 切分器 ====================

pub struct RustChunker;

impl RustChunker {
    fn parse_definition(line: &str) -> Option<(&'static str, String)> {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("/*") {
            return None;
        }
        let after_modifiers = Self::strip_modifiers(trimmed);
        for keyword in &["fn", "struct", "trait", "enum", "impl", "mod"] {
            if let Some(rest) = after_modifiers.strip_prefix(&format!("{} ", keyword)) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '!')
                    .collect();
                if !name.is_empty() && !matches!(name.as_str(), "where" | "for" | "dyn" | "type") {
                    return Some((keyword, name));
                }
            }
        }
        None
    }

    fn strip_modifiers(line: &str) -> String {
        let mut s = line.to_string();
        let pub_patterns = [
            "pub(crate) ",
            "pub(super) ",
            "pub(self) ",
            "pub(in ",
            "pub ",
        ];
        for pat in &pub_patterns {
            if let Some(rest) = s.strip_prefix(pat) {
                s = rest.to_string();
                break;
            }
        }
        if s.starts_with("pub(in ") {
            if let Some(pos) = s.find(") ") {
                s = s[pos + 2..].to_string();
            }
        }
        loop {
            let trimmed = s.trim_start().to_string();
            if let Some(rest) = trimmed.strip_prefix("async ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("unsafe ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("const ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("default ") {
                s = rest.to_string();
            } else if trimmed.starts_with("extern ") {
                if let Some(pos) = trimmed.find("\" ") {
                    s = trimmed[pos + 2..].to_string();
                } else if let Some(pos) = trimmed.find("] ") {
                    s = trimmed[pos + 2..].to_string();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        s
    }
}

impl CodeChunker for RustChunker {
    fn chunk_file(&self, file_path: &str, content: &str) -> Vec<CodeChunk> {
        let lines: Vec<&str> = content.lines().collect();
        let language = detect_language(file_path);
        let mut chunks = Vec::new();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i].trim();
            let (keyword, name) = match Self::parse_definition(line) {
                Some(r) => r,
                None => {
                    i += 1;
                    continue;
                }
            };

            let start_line = i + 1;
            let signature = line.to_string();
            let end_idx = find_brace_body_end(&lines, i);
            let end_idx = end_idx.min(lines.len().saturating_sub(1)); // 边界安全检查
            let end_line = end_idx + 1;
            let chunk_content = lines[i..=end_idx].join("\n");
            let doc = extract_triple_slash_doc(&lines, i);

            chunks.push(CodeChunk {
                id: format!("{}:L{}-L{}", file_path, start_line, end_line),
                file_path: file_path.to_string(),
                start_line,
                end_line,
                chunk_type: keyword.to_string(),
                name,
                signature,
                content: chunk_content,
                doc_comment: doc,
                language: language.clone(),
            });

            i = end_idx + 1;
        }

        chunks
    }
}

// ==================== Python 切分器 ====================

pub struct PythonChunker;

impl PythonChunker {
    fn parse_definition(line: &str) -> Option<(&'static str, String)> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            return None;
        }
        if trimmed.starts_with("from ") || trimmed.starts_with("import ") {
            return None;
        }

        // 处理 async def
        let after_async = trimmed.strip_prefix("async def ").map(|r| ("def", r));
        #[allow(clippy::question_mark)]
        let (keyword, rest) = if let Some((kw, r)) = after_async {
            (kw, r.to_string())
        } else if let Some(r) = trimmed.strip_prefix("def ") {
            ("def", r.to_string())
        } else if let Some(r) = trimmed.strip_prefix("class ") {
            ("class", r.to_string())
        } else {
            return None;
        };

        let name = rest
            .split(['(', ':'])
            .next()
            .unwrap_or("")
            .trim()
            .to_string();

        if name.is_empty() || name == "self" {
            None
        } else {
            Some((keyword, name))
        }
    }
}

impl CodeChunker for PythonChunker {
    fn chunk_file(&self, file_path: &str, content: &str) -> Vec<CodeChunk> {
        let lines: Vec<&str> = content.lines().collect();
        let language = detect_language(file_path);
        let mut chunks = Vec::new();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            let (keyword, name) = match Self::parse_definition(line) {
                Some(r) => r,
                None => {
                    i += 1;
                    continue;
                }
            };

            let start_line = i + 1;
            let signature = line.trim().to_string();
            let base_indent = line.len() - line.trim_start().len();
            let trimmed = line.trim();

            // 缩进体检测
            let mut end_idx = i;
            if trimmed.ends_with(':') {
                for (j, body_line) in lines.iter().enumerate().skip(i + 1) {
                    let body_trimmed = body_line.trim();
                    if body_trimmed.is_empty() {
                        end_idx = j;
                        continue;
                    }
                    let body_indent = body_line.len() - body_trimmed.len();
                    if body_indent > base_indent {
                        end_idx = j;
                    } else {
                        break;
                    }
                }
            }

            let end_line = end_idx + 1;
            let chunk_content = lines[i..=end_idx.min(lines.len().saturating_sub(1))].join("\n");
            let doc = extract_hash_or_jsdoc(&lines, i);

            chunks.push(CodeChunk {
                id: format!("{}:L{}-L{}", file_path, start_line, end_line),
                file_path: file_path.to_string(),
                start_line,
                end_line,
                chunk_type: keyword.to_string(),
                name,
                signature,
                content: chunk_content,
                doc_comment: doc,
                language: language.clone(),
            });

            i = end_idx + 1;
        }

        chunks
    }
}

// ==================== TypeScript / JavaScript 切分器 ====================

pub struct TsJsChunker;

impl TsJsChunker {
    fn parse_definition(line: &str, lang: &str) -> Option<(&'static str, String)> {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed == "*"
        {
            return None;
        }

        let after = Self::strip_modifiers(trimmed);

        // TS 特有关键字在前
        for keyword in &["interface", "type", "enum"] {
            if lang == "typescript" {
                if let Some(rest) = after.strip_prefix(&format!("{} ", keyword)) {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        return Some((keyword, name));
                    }
                }
            }
        }

        // function / class（TS 和 JS 都支持）
        for keyword in &["function", "class"] {
            if let Some(rest) = after.strip_prefix(&format!("{} ", keyword)) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    return Some((keyword, name));
                }
            }
        }

        // 箭头函数变量：const fnName = (...) => {
        if let Some(rest) = after
            .strip_prefix("const ")
            .or_else(|| after.strip_prefix("let "))
            .or_else(|| after.strip_prefix("var "))
        {
            if let Some(eq_pos) = rest.find('=') {
                let name = rest[..eq_pos].trim().to_string();
                // 检查等号后面是否是 (...) => 或 async (...) =>
                let after_eq = rest[eq_pos + 1..].trim();
                let is_arrow = after_eq.starts_with('(')
                    || after_eq.starts_with("async (")
                    || after_eq.starts_with('<');
                if !name.is_empty() && is_arrow {
                    return Some(("fn", name));
                }
            }
        }

        None
    }

    fn strip_modifiers(line: &str) -> String {
        let mut s = line.trim().to_string();
        loop {
            let trimmed = s.trim_start().to_string();
            if let Some(rest) = trimmed.strip_prefix("export default ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("export ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("public ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("private ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("protected ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("static ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("abstract ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("async ") {
                s = rest.to_string();
            } else if let Some(rest) = trimmed.strip_prefix("readonly ") {
                s = rest.to_string();
            } else {
                break;
            }
        }
        s
    }
}

impl CodeChunker for TsJsChunker {
    fn chunk_file(&self, file_path: &str, content: &str) -> Vec<CodeChunk> {
        let lines: Vec<&str> = content.lines().collect();
        let language = detect_language(file_path);
        let mut chunks = Vec::new();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i].trim();
            let (keyword, name) = match Self::parse_definition(line, &language) {
                Some(r) => r,
                None => {
                    i += 1;
                    continue;
                }
            };

            let start_line = i + 1;
            let signature = line.to_string();
            let end_idx = find_brace_body_end(&lines, i);
            let end_idx = end_idx.min(lines.len().saturating_sub(1)); // 边界安全检查
            let end_line = end_idx + 1;
            let chunk_content = lines[i..=end_idx].join("\n");
            let doc = extract_hash_or_jsdoc(&lines, i);

            chunks.push(CodeChunk {
                id: format!("{}:L{}-L{}", file_path, start_line, end_line),
                file_path: file_path.to_string(),
                start_line,
                end_line,
                chunk_type: keyword.to_string(),
                name,
                signature,
                content: chunk_content,
                doc_comment: doc,
                language: language.clone(),
            });

            i = end_idx + 1;
        }

        chunks
    }
}

// ==================== Go 切分器 ====================

pub struct GoChunker;

impl GoChunker {
    fn parse_definition(line: &str) -> Option<(&'static str, String)> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
            return None;
        }

        for keyword in &["func", "type"] {
            if let Some(rest) = trimmed.strip_prefix(&format!("{} ", keyword)) {
                let rest = rest.trim();
                // Go 方法: func (r *Receiver) MethodName(...)
                let clean_name = if rest.starts_with('(') {
                    // 提取接收者后的方法名
                    rest.split(')')
                        .nth(1)
                        .unwrap_or("")
                        .trim()
                        .split(['(', '{'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string()
                } else {
                    // 普通函数/类型: func Name(...) 或 type Name ...
                    rest.split(['(', '{', ' '])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string()
                };
                if !clean_name.is_empty() && clean_name != "(" {
                    return Some((keyword, clean_name));
                }
            }
        }

        None
    }
}

impl CodeChunker for GoChunker {
    fn chunk_file(&self, file_path: &str, content: &str) -> Vec<CodeChunk> {
        let lines: Vec<&str> = content.lines().collect();
        let language = detect_language(file_path);
        let mut chunks = Vec::new();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i].trim();
            let (keyword, name) = match Self::parse_definition(line) {
                Some(r) => r,
                None => {
                    i += 1;
                    continue;
                }
            };

            let start_line = i + 1;
            let signature = line.to_string();
            let end_idx = find_brace_body_end(&lines, i);
            let end_idx = end_idx.min(lines.len().saturating_sub(1)); // 边界安全检查
            let end_line = end_idx + 1;
            let chunk_content = lines[i..=end_idx].join("\n");

            chunks.push(CodeChunk {
                id: format!("{}:L{}-L{}", file_path, start_line, end_line),
                file_path: file_path.to_string(),
                start_line,
                end_line,
                chunk_type: keyword.to_string(),
                name,
                signature,
                content: chunk_content,
                doc_comment: None,
                language: language.clone(),
            });

            i = end_idx + 1;
        }

        chunks
    }
}

// ==================== 对话切分器 ====================

/// 对话切分器 — 按角色轮次对对话文本进行结构化切分
///
/// 支持中英文角色前缀识别（如 "用户:", "助手:", "User:", "Assistant:"），
/// 每轮对话作为一个独立片段，保留角色标识和内容。
pub struct ConversationChunker;

impl ConversationChunker {
    /// 检测行首是否为已知的角色前缀
    ///
    /// 返回 `Some(角色名)` 如果匹配，否则返回 `None`。
    /// 支持中文全角冒号（：）和英文半角冒号（:）。
    fn detect_role(line: &str) -> Option<String> {
        let trimmed = line.trim_start();

        let role_patterns: &[(&[&str], &str)] = &[
            (&["用户:", "用户："], "用户"),
            (&["助手:", "助手："], "助手"),
            (&["系统:", "系统："], "系统"),
            (&["User:", "USER:"], "User"),
            (&["Assistant:", "ASSISTANT:"], "Assistant"),
            (&["System:", "SYSTEM:"], "System"),
            (&["AI:", "ai:"], "AI"),
            (&["Human:", "HUMAN:", "human:"], "Human"),
        ];

        for (prefixes, role_name) in role_patterns {
            for prefix in *prefixes {
                if trimmed.starts_with(prefix) {
                    return Some(role_name.to_string());
                }
            }
        }

        None
    }

    /// 提取角色前缀后的内容
    fn extract_content(line: &str) -> &str {
        let trimmed = line.trim_start();
        // 先检查半角冒号 ':'
        if let Some(colon_pos) = trimmed.find(':') {
            if colon_pos > 0 {
                let colon_end = colon_pos + 1; // ':' 占 1 字节
                let prefix = &trimmed[..colon_end];
                let remaining = trimmed[colon_end..].trim();
                if Self::detect_role(prefix).is_some() {
                    return remaining;
                }
            }
        }
        // 再检查全角冒号 '：'
        if let Some(colon_pos) = trimmed.find('：') {
            if colon_pos > 0 {
                let colon_end = colon_pos + '：'.len_utf8(); // '：' 占 3 字节
                let prefix = &trimmed[..colon_end];
                let remaining = trimmed[colon_end..].trim();
                if Self::detect_role(prefix).is_some() {
                    return remaining;
                }
            }
        }
        trimmed
    }
}

impl CodeChunker for ConversationChunker {
    fn chunk_file(&self, file_path: &str, content: &str) -> Vec<CodeChunk> {
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() || (lines.len() == 1 && lines[0].trim().is_empty()) {
            return vec![];
        }

        let mut chunks = Vec::new();
        let mut turn_start: Option<usize> = None;
        let mut current_role: Option<String> = None;

        for (idx, line) in lines.iter().enumerate() {
            let detected_role = Self::detect_role(line);

            if let Some(role) = detected_role {
                // 保存上一轮对话
                if let Some(start) = turn_start {
                    if start < idx {
                        let end = idx
                            .saturating_sub(1)
                            .max(start)
                            .min(lines.len().saturating_sub(1)); // 边界安全检查
                        let turn_lines = &lines[start..=end];
                        let content = turn_lines.join("\n");
                        let role_name = current_role.unwrap_or_else(|| "未知".to_string());

                        let first_line = Self::extract_content(lines[start]);
                        let name = format!(
                            "{}: {}",
                            role_name,
                            first_line.chars().take(40).collect::<String>()
                        );

                        chunks.push(CodeChunk {
                            id: format!("{}:L{}-L{}", file_path, start + 1, end + 1),
                            file_path: file_path.to_string(),
                            start_line: start + 1,
                            end_line: end + 1,
                            chunk_type: "conversation_turn".to_string(),
                            name,
                            signature: role_name.clone(),
                            content,
                            doc_comment: None,
                            language: "conversation".to_string(),
                        });
                    }
                }
                turn_start = Some(idx);
                current_role = Some(role);
            }
        }

        // 保存最后一轮对话
        if let Some(start) = turn_start {
            if start < lines.len() {
                let start = start.min(lines.len()); // 边界安全检查
                let turn_lines = &lines[start..];
                let content = turn_lines.join("\n");
                let role_name = current_role.unwrap_or_else(|| "未知".to_string());

                let first_line = Self::extract_content(lines[start]);
                let name = format!(
                    "{}: {}",
                    role_name,
                    first_line.chars().take(40).collect::<String>()
                );

                chunks.push(CodeChunk {
                    id: format!("{}:L{}-L{}", file_path, start + 1, lines.len()),
                    file_path: file_path.to_string(),
                    start_line: start + 1,
                    end_line: lines.len(),
                    chunk_type: "conversation_turn".to_string(),
                    name,
                    signature: role_name.clone(),
                    content,
                    doc_comment: None,
                    language: "conversation".to_string(),
                });
            }
        }

        chunks
    }
}

// ==================== 通用文档切分器 ====================

pub struct GenericChunker;

impl GenericChunker {
    fn chunk_markdown(file_path: &str, content: &str, language: &str) -> Vec<CodeChunk> {
        let lines: Vec<&str> = content.lines().collect();
        let mut chunks = Vec::new();
        let mut section_start: Option<usize> = None;
        let mut section_title = String::new();

        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            // 检测 Markdown 标题行
            let is_heading = trimmed.starts_with("## ") || trimmed.starts_with("# ");

            if is_heading || idx == 0 {
                // 保存上一个 section
                if let Some(start) = section_start {
                    if start < idx {
                        let end = (idx - 1).max(start).min(lines.len().saturating_sub(1)); // 边界安全检查
                        let chunk_content = lines[start..=end].join("\n");
                        let name = if section_title.is_empty() {
                            lines[start].chars().take(80).collect()
                        } else {
                            section_title.clone()
                        };

                        chunks.push(CodeChunk {
                            id: format!("{}:L{}-L{}", file_path, start + 1, end + 1),
                            file_path: file_path.to_string(),
                            start_line: start + 1,
                            end_line: end + 1,
                            chunk_type: "section".to_string(),
                            name,
                            signature: section_title.clone(),
                            content: chunk_content,
                            doc_comment: None,
                            language: language.to_string(),
                        });
                    }
                }

                section_start = Some(idx);
                section_title = if is_heading {
                    trimmed.trim_start_matches('#').trim().to_string()
                } else {
                    String::new()
                };
            }
        }

        // 最后一个 section
        if let Some(start) = section_start {
            if start < lines.len() {
                let start = start.min(lines.len()); // 边界安全检查
                let chunk_content = lines[start..].join("\n");
                let name = if section_title.is_empty() {
                    lines[start].chars().take(80).collect()
                } else {
                    section_title.clone()
                };

                chunks.push(CodeChunk {
                    id: format!("{}:L{}-L{}", file_path, start + 1, lines.len()),
                    file_path: file_path.to_string(),
                    start_line: start + 1,
                    end_line: lines.len(),
                    chunk_type: "section".to_string(),
                    name,
                    signature: section_title.clone(),
                    content: chunk_content,
                    doc_comment: None,
                    language: language.to_string(),
                });
            }
        }

        chunks
    }

    fn chunk_generic(file_path: &str, content: &str, language: &str) -> Vec<CodeChunk> {
        if content.trim().is_empty() {
            return vec![];
        }

        // 按双换行（段落）分割
        let paragraphs: Vec<&str> = content.split("\n\n").collect();
        let mut chunks = Vec::new();
        let mut line_cursor = 0usize;

        for para in paragraphs {
            if para.trim().is_empty() {
                // 推进行号光标
                line_cursor += para.lines().count();
                if para.ends_with('\n') {
                    // 实际是 \n\n 分割，加回一个空行
                }
                continue;
            }

            let para_lines: Vec<&str> = para.lines().collect();
            let start_line = line_cursor + 1;
            let end_line = line_cursor + para_lines.len();
            let first_line = para_lines.first().map_or("", |s| s.trim());
            let name = first_line.chars().take(80).collect::<String>();

            chunks.push(CodeChunk {
                id: format!("{}:L{}-L{}", file_path, start_line, end_line),
                file_path: file_path.to_string(),
                start_line,
                end_line,
                chunk_type: "paragraph".to_string(),
                name,
                signature: first_line.to_string(),
                content: para.to_string(),
                doc_comment: None,
                language: language.to_string(),
            });

            line_cursor += para_lines.len() + 1; // +1 for the blank line separator
        }

        chunks
    }
}

impl CodeChunker for GenericChunker {
    fn chunk_file(&self, file_path: &str, content: &str) -> Vec<CodeChunk> {
        let language = detect_language(file_path);

        match language.as_str() {
            "markdown" => Self::chunk_markdown(file_path, content, &language),
            _ => Self::chunk_generic(file_path, content, &language),
        }
    }
}

// ==================== 统一分发 ====================

/// 根据文件路径自动选择切分器并执行切分
pub fn chunk_by_language(file_path: &str, content: &str) -> Vec<CodeChunk> {
    let lang = detect_language(file_path);

    match lang.as_str() {
        "rust" => RustChunker.chunk_file(file_path, content),
        "python" => PythonChunker.chunk_file(file_path, content),
        "typescript" | "javascript" => TsJsChunker.chunk_file(file_path, content),
        "go" => GoChunker.chunk_file(file_path, content),
        _ => GenericChunker.chunk_file(file_path, content),
    }
}

// ==================== 测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    // === Rust 切分测试 ===

    #[test]
    fn test_rust_empty_file() {
        let chunks = RustChunker.chunk_file("empty.rs", "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_rust_simple_fn() {
        let code = "fn hello() {\n    println!(\"world\");\n}\n";
        let chunks = RustChunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "hello");
        assert_eq!(chunks[0].chunk_type, "fn");
        assert_eq!(chunks[0].language, "rust");
        assert_eq!(chunks[0].start_line, 1);
    }

    #[test]
    fn test_rust_pub_async_fn() {
        let code = "pub async fn retrieve(&self, query: &str) -> Vec<String> {\n    vec![]\n}\n";
        let chunks = RustChunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "retrieve");
    }

    #[test]
    fn test_rust_struct() {
        let code = "pub struct MemoryItem {\n    pub id: String,\n    pub content: String,\n}\n";
        let chunks = RustChunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "MemoryItem");
        assert_eq!(chunks[0].chunk_type, "struct");
    }

    #[test]
    fn test_rust_multiple_definitions() {
        let code = concat!("fn a() {}\n", "struct B {}\n", "fn c() {}\n",);
        let chunks = RustChunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 3);
    }

    #[test]
    fn test_rust_nested_braces() {
        let code = concat!(
            "fn outer() {\n",
            "    if true {\n",
            "        let x = { 1 + 2 };\n",
            "    }\n",
            "}\n",
            "\n",
            "fn next_fn() {}\n",
        );
        let chunks = RustChunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].end_line, 5);
        assert_eq!(chunks[1].name, "next_fn");
    }

    #[test]
    fn test_rust_doc_comment() {
        let code = concat!(
            "/// 这是文档注释\n",
            "/// 第二行\n",
            "pub fn documented() {}\n",
        );
        let chunks = RustChunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].doc_comment.as_deref(),
            Some("这是文档注释\n第二行")
        );
    }

    #[test]
    fn test_rust_const_fn() {
        let code = "pub const fn constant_fn() -> u32 { 42 }\n";
        let chunks = RustChunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "constant_fn");
    }

    // === Python 切分测试 ===

    #[test]
    fn test_python_empty() {
        let chunks = PythonChunker.chunk_file("empty.py", "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_python_simple_def() {
        let code = "def hello():\n    return 'world'\n";
        let chunks = PythonChunker.chunk_file("test.py", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "hello");
        assert_eq!(chunks[0].chunk_type, "def");
        assert_eq!(chunks[0].language, "python");
    }

    #[test]
    fn test_python_async_def() {
        let code = "async def fetch(url):\n    return await get(url)\n";
        let chunks = PythonChunker.chunk_file("test.py", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "fetch");
    }

    #[test]
    fn test_python_class() {
        let code = "class MemoryManager:\n    def __init__(self):\n        self.items = []\n\n    def add(self, item):\n        self.items.append(item)\n";
        let chunks = PythonChunker.chunk_file("test.py", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "MemoryManager");
        assert_eq!(chunks[0].chunk_type, "class");
        // 整个类体作为一个 chunk（文件共 6 行，末尾 \n 被 lines() 自动去除）
        assert_eq!(chunks[0].end_line, 6);
    }

    #[test]
    fn test_python_multiple_defs() {
        let code = "def a():\n    pass\n\n\ndef b():\n    pass\n";
        let chunks = PythonChunker.chunk_file("test.py", code);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].name, "a");
        assert_eq!(chunks[1].name, "b");
    }

    // === TypeScript 切分测试 ===

    #[test]
    fn test_ts_empty() {
        let chunks = TsJsChunker.chunk_file("empty.ts", "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_ts_function() {
        let code = "export async function fetchUser(id: string): Promise<User> {\n    return {} as User;\n}\n";
        let chunks = TsJsChunker.chunk_file("test.ts", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "fetchUser");
        assert_eq!(chunks[0].chunk_type, "function");
        assert_eq!(chunks[0].language, "typescript");
    }

    #[test]
    fn test_ts_interface() {
        let code = "export interface MemoryItem {\n    id: string;\n    content: string;\n}\n";
        let chunks = TsJsChunker.chunk_file("test.ts", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "MemoryItem");
        assert_eq!(chunks[0].chunk_type, "interface");
    }

    #[test]
    fn test_ts_type_alias() {
        let code = "type MemoryKey = string;\n";
        let chunks = TsJsChunker.chunk_file("test.ts", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "MemoryKey");
        assert_eq!(chunks[0].chunk_type, "type");
    }

    #[test]
    fn test_ts_class() {
        let code = "class MemoryStore {\n    private items: Map<string, any> = new Map();\n}\n";
        let chunks = TsJsChunker.chunk_file("test.ts", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "MemoryStore");
        assert_eq!(chunks[0].chunk_type, "class");
    }

    #[test]
    fn test_js_function() {
        let code = "function hello() {\n    return 'world';\n}\n";
        let chunks = TsJsChunker.chunk_file("test.js", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "hello");
        assert_eq!(chunks[0].language, "javascript");
    }

    // === Go 切分测试 ===

    #[test]
    fn test_go_empty() {
        let chunks = GoChunker.chunk_file("empty.go", "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_go_func() {
        let code = "func Hello() string {\n    return \"world\"\n}\n";
        let chunks = GoChunker.chunk_file("test.go", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "Hello");
        assert_eq!(chunks[0].chunk_type, "func");
        assert_eq!(chunks[0].language, "go");
    }

    #[test]
    fn test_go_method() {
        let code = "func (m *Manager) Search(query string) []Result {\n    return nil\n}\n";
        let chunks = GoChunker.chunk_file("test.go", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "Search");
    }

    // === 通用切分测试 ===

    #[test]
    fn test_generic_text() {
        let code = "这是第一段内容。\n包含多行文字。\n\n这是第二段内容。\n也是多行。\n\n第三段。\n";
        let chunks = GenericChunker.chunk_file("notes.txt", code);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].chunk_type, "paragraph");
        assert_eq!(chunks[0].language, "text");
    }

    #[test]
    fn test_markdown_heading_split() {
        let code =
            "# 标题\n\n一些介绍文字。\n\n## 第一节\n\n第一节内容。\n\n## 第二节\n\n第二节内容。\n";
        let chunks = GenericChunker.chunk_file("doc.md", code);
        assert!(chunks.len() >= 3);
        assert_eq!(chunks[0].language, "markdown");
        assert_eq!(chunks[0].chunk_type, "section");
    }

    // === 语言检测测试 ===

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language("main.rs"), "rust");
        assert_eq!(detect_language("app.py"), "python");
        assert_eq!(detect_language("index.ts"), "typescript");
        assert_eq!(detect_language("index.tsx"), "typescript");
        assert_eq!(detect_language("app.js"), "javascript");
        assert_eq!(detect_language("main.go"), "go");
        assert_eq!(detect_language("README.md"), "markdown");
        assert_eq!(detect_language("notes.txt"), "text");
        assert_eq!(detect_language("unknown.xyz"), "text");
    }

    // === 统一分发测试 ===

    #[test]
    fn test_chunk_by_language_routing() {
        let rs = chunk_by_language("test.rs", "fn a() {}\n");
        assert_eq!(rs[0].language, "rust");

        let py = chunk_by_language("test.py", "def a():\n    pass\n");
        assert_eq!(py[0].language, "python");

        let ts = chunk_by_language("test.ts", "function a() {}\n");
        assert_eq!(ts[0].language, "typescript");

        let txt = chunk_by_language("readme.md", "# Hello\n\nWorld\n");
        assert_eq!(txt[0].language, "markdown");
    }

    // === 对话切分测试 ===

    #[test]
    fn test_conversation_empty() {
        let chunks = ConversationChunker.chunk_file("chat.txt", "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_conversation_chinese_simple() {
        let text = "用户: 你好\n助手: 你好！有什么可以帮你的？\n用户: 我想了解 Rust\n";
        let chunks = ConversationChunker.chunk_file("chat.txt", text);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].chunk_type, "conversation_turn");
        assert_eq!(chunks[0].signature, "用户");
        assert!(chunks[0].content.contains("你好"));
        assert_eq!(chunks[0].language, "conversation");

        assert_eq!(chunks[1].signature, "助手");
        assert!(chunks[1].content.contains("有什么可以帮你的"));

        assert_eq!(chunks[2].signature, "用户");
        assert!(chunks[2].content.contains("Rust"));
    }

    #[test]
    fn test_conversation_english_simple() {
        let text = "User: Hello\nAssistant: Hi there!\nUser: How are you?\n";
        let chunks = ConversationChunker.chunk_file("chat.txt", text);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].signature, "User");
        assert!(chunks[0].content.contains("Hello"));
        assert_eq!(chunks[1].signature, "Assistant");
        assert!(chunks[1].content.contains("Hi there"));
        assert_eq!(chunks[2].signature, "User");
        assert!(chunks[2].content.contains("How are you"));
    }

    #[test]
    fn test_conversation_multiline_turns() {
        let text = concat!(
            "用户: 我有一个问题\n",
            "关于 Rust 的所有权系统\n",
            "能帮我理解一下吗？\n",
            "助手: 当然可以！\n",
            "Rust 的所有权系统是它最独特的特性\n",
            "它确保内存安全而无需垃圾回收\n",
        );
        let chunks = ConversationChunker.chunk_file("chat.txt", text);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].signature, "用户");
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
        assert!(chunks[0].content.contains("所有权系统"));

        assert_eq!(chunks[1].signature, "助手");
        assert_eq!(chunks[1].start_line, 4);
        assert_eq!(chunks[1].end_line, 6);
        assert!(chunks[1].content.contains("内存安全"));
    }

    #[test]
    fn test_conversation_single_turn() {
        let text = "用户: 只有一条消息\n";
        let chunks = ConversationChunker.chunk_file("chat.txt", text);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].signature, "用户");
        assert!(chunks[0].content.contains("只有一条消息"));
    }

    #[test]
    fn test_conversation_fullwidth_colon() {
        let text = "用户：你好\n助手：你好！\n";
        let chunks = ConversationChunker.chunk_file("chat.txt", text);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].signature, "用户");
        assert_eq!(chunks[1].signature, "助手");
    }

    #[test]
    fn test_conversation_system_role() {
        let text = "系统: 你是一个有用的助手\n用户: 你好\n";
        let chunks = ConversationChunker.chunk_file("chat.txt", text);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].signature, "系统");
        assert_eq!(chunks[1].signature, "用户");
    }

    #[test]
    fn test_conversation_no_role_prefix() {
        let text = "这是第一行\n这是第二行\n这是第三行\n";
        let chunks = ConversationChunker.chunk_file("chat.txt", text);

        assert_eq!(chunks.len(), 0, "无角色前缀的文本不应产生对话切分");
    }
}
