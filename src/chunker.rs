// 代码切分器
// ==========
// 将 Rust 源码按函数/结构体/trait/impl 边界切分为 CodeChunk。
// Phase 1 使用正则表达式（行走骨架），Phase 2 替换为 tree-sitter AST。

use serde::{Deserialize, Serialize};

/// 代码片段 — 与 Python Phase 0 脚本的 CodeChunk 对齐
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
    /// 类型：fn | struct | trait | impl | enum | mod
    pub chunk_type: String,
    /// 函数名/结构体名/trait 名
    pub name: String,
    /// 签名行文本
    pub signature: String,
    /// 完整代码文本（含大括号体）
    pub content: String,
    /// /// 或 /** */ 文档注释（可选）
    pub doc_comment: Option<String>,
}

/// 代码切分器 trait — 契约定义
///
/// 实现此 trait 即可替换切分策略：
/// - 正则版：Phase 1 行走骨架，快速验证
/// - tree-sitter 版：Phase 2，AST 级精准切分
pub trait CodeChunker: Send + Sync {
    /// 切分单个 Rust 源文件为代码片段列表
    fn chunk_file(&self, file_path: &str, content: &str) -> Vec<CodeChunk>;

    /// 批量切分（目录扫描由 Manager 层负责）
    fn chunk_batch(&self, files: &[(String, String)]) -> Vec<CodeChunk> {
        files
            .iter()
            .flat_map(|(path, content)| self.chunk_file(path, content))
            .collect()
    }
}

// ==================== 正则版实现（行走骨架） ====================

/// 基于正则表达式的 Rust 代码切分器
///
/// 作为 Phase 1 行走骨架，后续替换为 tree-sitter 实现。
pub struct RegexChunker;

impl RegexChunker {
    /// 从代码行列表中提取函数/结构体前的文档注释
    fn extract_doc_comment(lines: &[&str], start_idx: usize) -> Option<String> {
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
}

impl CodeChunker for RegexChunker {
    fn chunk_file(&self, file_path: &str, content: &str) -> Vec<CodeChunk> {
        let lines: Vec<&str> = content.lines().collect();
        let mut chunks = Vec::new();

        let mut i = 0;
        while i < lines.len() {
            let line = lines[i].trim();

            // 匹配 Rust 顶层定义：fn struct trait enum impl mod
            let (keyword, name) = match Self::parse_definition(line) {
                Some(result) => result,
                None => {
                    i += 1;
                    continue;
                }
            };

            let start_line = i + 1; // 1-based
            let signature = line.to_string();

            // 大括号深度匹配，找到定义体的结束位置
            let mut depth = 0i32;
            let mut found_open = false;
            let mut end_idx = i;

            for j in i..lines.len() {
                for ch in lines[j].chars() {
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

            if !found_open || end_idx == i {
                end_idx = i; // 单行定义或未找到体
            }

            let end_line = end_idx + 1; // 1-based
            let chunk_content = lines[i..=end_idx].join("\n");
            let doc = Self::extract_doc_comment(&lines, i);

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
            });

            i = end_idx + 1;
        }

        chunks
    }
}

impl RegexChunker {
    /// 解析一行 Rust 代码，识别是否为定义行
    ///
    /// 返回 Some((关键字, 名称))，如 ("fn", "retrieve")。
    fn parse_definition(line: &str) -> Option<(&'static str, String)> {
        let trimmed = line.trim();

        // 跳过属性行和宏调用（但不跳过 #[cfg] 等，它们后面才跟着真正的定义）
        if trimmed.starts_with("//") || trimmed.starts_with("/*") {
            return None;
        }

        // 匹配模式：移除可见性/异步/非安全等修饰符后的关键字+名称
        let after_modifiers = Self::strip_modifiers(trimmed);

        for keyword in &["fn", "struct", "trait", "enum", "impl", "mod"] {
            if let Some(rest) = after_modifiers.strip_prefix(&format!("{} ", keyword)) {
                // 提取名称：直到遇到 < > ( { 或空格
                let name = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>();
                if !name.is_empty() && !matches!(name.as_str(), "where" | "for" | "dyn" | "type")
                {
                    return Some((keyword, name));
                }
            }
        }

        None
    }

    /// 剥离 Rust 可见性/修饰符前缀
    ///
    /// pub(crate) async unsafe extern "C" fn ... → fn ...
    fn strip_modifiers(line: &str) -> String {
        let mut s = line.to_string();

        // 移除 pub / pub(crate) / pub(super) / pub(self)
        let pub_patterns = ["pub(crate) ", "pub(super) ", "pub(self) ", "pub(in ", "pub "];
        for pat in &pub_patterns {
            if let Some(rest) = s.strip_prefix(pat) {
                s = rest.to_string();
                break;
            }
        }
        // 有可见性限制的 pub(in path) — 匹配到 )
        if s.starts_with("pub(in ") {
            if let Some(pos) = s.find(") ") {
                s = s[pos + 2..].to_string();
            }
        }

        // 移除 async / unsafe / extern "C" / const / default
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

// ==================== 单元测试（TDD 红→绿→重构） ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试空文件
    #[test]
    fn test_chunk_empty_file() {
        let chunker = RegexChunker;
        let chunks = chunker.chunk_file("empty.rs", "");
        assert!(chunks.is_empty());
    }

    /// 测试简单函数
    #[test]
    fn test_chunk_simple_fn() {
        let chunker = RegexChunker;
        let code = "fn hello() {\n    println!(\"world\");\n}\n";
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "hello");
        assert_eq!(chunks[0].chunk_type, "fn");
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
    }

    /// 测试 pub async fn
    #[test]
    fn test_chunk_pub_async_fn() {
        let chunker = RegexChunker;
        let code = "pub async fn retrieve(&self, query: &str) -> Vec<String> {\n    vec![]\n}\n";
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "retrieve");
        assert_eq!(chunks[0].chunk_type, "fn");
    }

    /// 测试 struct 定义
    #[test]
    fn test_chunk_struct() {
        let chunker = RegexChunker;
        let code = "pub struct MemoryItem {\n    pub id: String,\n    pub content: String,\n}\n";
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "MemoryItem");
        assert_eq!(chunks[0].chunk_type, "struct");
        assert_eq!(chunks[0].end_line, 4);
    }

    /// 测试 impl 块
    #[test]
    fn test_chunk_impl_block() {
        let chunker = RegexChunker;
        let code = "impl MemoryManager {\n    fn new() -> Self {\n        Self {}\n    }\n}\n";
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "MemoryManager");
        assert_eq!(chunks[0].chunk_type, "impl");
    }

    /// 测试 impl Trait for Type
    #[test]
    fn test_chunk_impl_trait_for_type() {
        let chunker = RegexChunker;
        let code =
            "impl CodeChunker for RegexChunker {\n    fn chunk_file(&self) -> Vec<CodeChunk> {\n        vec![]\n    }\n}\n";
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "impl");
    }

    /// 测试多个定义混合
    #[test]
    fn test_chunk_multiple_definitions() {
        let chunker = RegexChunker;
        let code = concat!(
            "fn a() {}\n",
            "struct B {}\n",
            "fn c() {}\n",
            "impl D {}\n",
            "trait E {}\n",
            "enum F {}\n",
            "mod G {}\n",
        );
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 7);
        assert_eq!(chunks[0].name, "a");
        assert_eq!(chunks[1].name, "B");
        assert_eq!(chunks[2].name, "c");
        assert_eq!(chunks[6].name, "G");
    }

    /// 测试嵌套大括号不干扰边界检测
    #[test]
    fn test_chunk_nested_braces() {
        let chunker = RegexChunker;
        let code = concat!(
            "fn outer() {\n",
            "    if true {\n",
            "        let x = { 1 + 2 };\n",
            "    }\n",
            "}\n",
            "\n",
            "fn next_fn() {}\n",
        );
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].name, "outer");
        assert_eq!(chunks[0].end_line, 5);
        assert_eq!(chunks[1].name, "next_fn");
    }

    /// 测试文档注释提取
    #[test]
    fn test_chunk_doc_comment() {
        let chunker = RegexChunker;
        let code = concat!(
            "/// 这是文档注释\n",
            "/// 第二行\n",
            "pub fn documented() {}\n",
        );
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].doc_comment.as_deref(),
            Some("这是文档注释\n第二行")
        );
    }

    /// 测试 pub(crate) 修饰符
    #[test]
    fn test_chunk_pub_crate_fn() {
        let chunker = RegexChunker;
        let code = "pub(crate) fn internal() {}\n";
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "internal");
        assert_eq!(chunks[0].chunk_type, "fn");
    }

    /// 测试 const fn
    #[test]
    fn test_chunk_const_fn() {
        let chunker = RegexChunker;
        let code = "pub const fn constant_fn() -> u32 { 42 }\n";
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "constant_fn");
        assert_eq!(chunks[0].chunk_type, "fn");
    }

    /// 测试签名中包含泛型参数的函数
    #[test]
    fn test_chunk_generic_fn() {
        let chunker = RegexChunker;
        let code = "pub fn get_memories_paginated<T>(offset: usize) -> T {}\n";
        let chunks = chunker.chunk_file("test.rs", code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "get_memories_paginated");
    }

    /// 测试代码 ID 格式
    #[test]
    fn test_chunk_id_format() {
        let chunker = RegexChunker;
        let code = "fn foo() {}\n";
        let chunks = chunker.chunk_file("src/memory.rs", code);
        assert_eq!(chunks[0].id, "src/memory.rs:L1-L1");
    }
}