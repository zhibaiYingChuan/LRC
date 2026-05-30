// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的编码器注册与路由算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 编码器注册表模块
// 统一管理多个编码器，按语言类型自动路由编码请求。
// 核心模式：Strategy + Registry 组合，实现编码策略的热插拔。

use crate::chunker::CodeChunk;
use crate::engine::encoder::{CodeEncoder, EmbeddingVector, FastEncoder};
use std::collections::HashMap;
use std::sync::Arc;

/// 编码器注册表 — 按语言类型路由编码策略
///
/// 支持多语言编码器注册与统一调用接口。
/// 未注册的语言自动回退到默认编码器。
pub struct EncoderRegistry {
    /// 语言 → 编码器映射（如 "rust" → RustEncoder）
    encoders: HashMap<String, Arc<dyn CodeEncoder>>,
    /// 默认回退编码器（处理未注册的语言）
    default_encoder: Arc<dyn CodeEncoder>,
}

impl EncoderRegistry {
    /// 创建编码器注册表
    ///
    /// `default_terms` — 默认编码器的关键词列表，用于构建 FastEncoder
    pub fn new(default_terms: Vec<String>) -> Self {
        let default = Arc::new(FastEncoder::new(default_terms));
        Self {
            encoders: HashMap::new(),
            default_encoder: default,
        }
    }

    /// 注册一个编码器到指定语言
    ///
    /// 同一语言多次注册会覆盖之前的编码器。
    pub fn register(&mut self, language: &str, encoder: Arc<dyn CodeEncoder>) {
        self.encoders.insert(language.to_lowercase(), encoder);
    }

    /// 获取指定语言对应的编码器（含回退逻辑）
    fn get_encoder(&self, language: &str) -> &Arc<dyn CodeEncoder> {
        self.encoders
            .get(&language.to_lowercase())
            .unwrap_or(&self.default_encoder)
    }

    /// 编码单个代码片段（自动按语言路由）
    pub fn encode(&self, chunk: &CodeChunk) -> EmbeddingVector {
        let encoder = self.get_encoder(&chunk.language);
        encoder.encode(chunk)
    }

    /// 批量编码（自动按语言路由每个片段）
    pub fn encode_batch(&self, chunks: &[CodeChunk]) -> Vec<EmbeddingVector> {
        chunks.iter().map(|c| self.encode(c)).collect()
    }

    /// 返回已注册的语言数量
    pub fn registered_languages(&self) -> Vec<String> {
        let mut langs: Vec<String> = self.encoders.keys().cloned().collect();
        langs.sort();
        langs
    }

    /// 返回已注册的编码器总数
    pub fn encoder_count(&self) -> usize {
        self.encoders.len()
    }
}

// ==================== 测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk(language: &str, content: &str) -> CodeChunk {
        CodeChunk {
            id: format!("test:L1-L{}", content.lines().count()),
            file_path: "test.ext".to_string(),
            start_line: 1,
            end_line: content.lines().count(),
            chunk_type: "fn".to_string(),
            name: "test_fn".to_string(),
            signature: "fn test_fn()".to_string(),
            content: content.to_string(),
            doc_comment: None,
            language: language.to_string(),
        }
    }

    fn default_terms() -> Vec<String> {
        vec![
            "fn".into(), "struct".into(), "impl".into(), "pub".into(),
            "use".into(), "mod".into(), "let".into(), "mut".into(),
        ]
    }

    #[test]
    fn test_new_registry_has_default() {
        let registry = EncoderRegistry::new(default_terms());
        assert_eq!(registry.encoder_count(), 0, "初始无自定义编码器");

        // 使用默认编码器编码任意语言
        let chunk = make_chunk("rust", "fn main() {}");
        let vec = registry.encode(&chunk);
        assert!(vec.dim > 0, "默认编码器应产生向量");
    }

    #[test]
    fn test_register_and_route() {
        let mut registry = EncoderRegistry::new(default_terms());

        // 注册一个专用的 rust 编码器（关键词更少）
        let rust_encoder = Arc::new(FastEncoder::new(vec!["struct".into(), "impl".into()]));
        registry.register("rust", rust_encoder);

        assert_eq!(registry.encoder_count(), 1);
        assert_eq!(registry.registered_languages(), vec!["rust"]);

        // rust 应路由到专用编码器（dim=2）
        let rust_chunk = make_chunk("rust", "struct Foo { x: i32 }");
        let rust_vec = registry.encode(&rust_chunk);
        assert_eq!(rust_vec.dim, 2, "rust 应使用专用编码器");

        // python 应回退到默认编码器（dim=8）
        let py_chunk = make_chunk("python", "def foo(): pass");
        let py_vec = registry.encode(&py_chunk);
        assert_eq!(py_vec.dim, 8, "python 应回退到默认编码器");
    }

    #[test]
    fn test_case_insensitive_language() {
        let mut registry = EncoderRegistry::new(vec!["fn".into()]);
        let encoder = Arc::new(FastEncoder::new(vec!["alpha".into()]));
        registry.register("Rust", encoder);

        // 大小写不敏感的注册查找
        let chunk = make_chunk("rust", "fn main() { alpha }");
        let vec = registry.encode(&chunk);
        assert_eq!(vec.dim, 1, "大小写不敏感匹配");
    }

    #[test]
    fn test_override_registration() {
        let mut registry = EncoderRegistry::new(vec!["fn".into()]);

        let encoder_a = Arc::new(FastEncoder::new(vec!["a".into()]));
        let encoder_b = Arc::new(FastEncoder::new(vec!["b".into(), "c".into()]));
        registry.register("typescript", encoder_a.clone());
        registry.register("typescript", encoder_b.clone());

        // 覆盖后应只有一个 typescript 编码器
        assert_eq!(registry.encoder_count(), 1);
        let chunk = make_chunk("typescript", "const x = b;");
        let vec = registry.encode(&chunk);
        assert_eq!(vec.dim, 2, "应使用最后注册的编码器");
    }

    #[test]
    fn test_batch_encode_routing() {
        let mut registry = EncoderRegistry::new(vec!["fn".into()]);
        let rust_enc = Arc::new(FastEncoder::new(vec!["struct".into(), "impl".into()]));
        registry.register("rust", rust_enc);

        let chunks = vec![
            make_chunk("rust", "struct A;"),
            make_chunk("rust", "impl A {}"),
            make_chunk("python", "def foo(): pass"),
            make_chunk("go", "func main() {}"),
        ];

        let vectors = registry.encode_batch(&chunks);
        assert_eq!(vectors.len(), 4);

        // rust 片段用专用编码器（dim=2），其他用默认编码器（dim=1）
        assert_eq!(vectors[0].dim, 2, "rust chunk 1 用专用编码器");
        assert_eq!(vectors[1].dim, 2, "rust chunk 2 用专用编码器");
        assert_eq!(vectors[2].dim, 1, "python 回退到默认");
        assert_eq!(vectors[3].dim, 1, "go 回退到默认");
    }

    #[test]
    fn test_registered_languages_sorted() {
        let mut registry = EncoderRegistry::new(vec!["fn".into()]);
        let enc = Arc::new(FastEncoder::new(vec!["x".into()]));

        registry.register("rust", enc.clone());
        registry.register("python", enc.clone());
        registry.register("typescript", enc.clone());

        // 应按字母排序
        assert_eq!(
            registry.registered_languages(),
            vec!["python", "rust", "typescript"]
        );
    }
}