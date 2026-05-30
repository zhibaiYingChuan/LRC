// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心编排逻辑，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 编排模块
// 整合切分、编码、检索三阶段流水线。
// 按文件扩展名自动选择多语言切分策略。

use crate::chunker::{chunk_by_language, is_supported_file, CodeChunk};
use crate::engine::encoder::{CodeEncoder, FastEncoder};
use crate::engine::retriever::{CodeRetriever, LocalRetriever, RetrievalResult};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// 索引统计
#[derive(Debug, Clone, Serialize)]
pub struct ChunkStats {
    pub file_count: usize,
    pub total_chunks: usize,
    pub type_counts: HashMap<String, usize>,
    pub language_counts: HashMap<String, usize>,
    pub avg_lines: f32,
}

/// 核心编排器
pub struct CoreManager<E: CodeEncoder = FastEncoder> {
    retriever: LocalRetriever<E>,
    file_count: usize,
}

impl CoreManager {
    pub fn new() -> Self {
        let terms = vec![
            "fn",
            "struct",
            "impl",
            "trait",
            "enum",
            "mod",
            "pub",
            "async",
            "await",
            "def",
            "class",
            "self",
            "function",
            "export",
            "import",
            "const",
            "let",
            "var",
            "interface",
            "type",
            "func",
            "return",
            "memory",
            "retrieve",
            "encode",
            "decode",
            "search",
            "index",
            "session",
            "config",
            "user",
            "task",
            "workflow",
            "engine",
            "handler",
            "manager",
            "store",
            "cache",
            "query",
            "fetch",
            "build",
            "init",
            "setup",
            "load",
            "save",
        ];

        let encoder = Arc::new(FastEncoder::new(
            terms.into_iter().map(String::from).collect(),
        ));
        let retriever = LocalRetriever::new(encoder, 0.01);

        Self {
            retriever,
            file_count: 0,
        }
    }
}

impl<E: CodeEncoder> CoreManager<E> {
    pub fn with_encoder(encoder: Arc<E>) -> Self {
        Self {
            retriever: LocalRetriever::new(encoder, 0.01),
            file_count: 0,
        }
    }

    /// 索引整个项目目录，自动识别所有支持的文本文件格式
    pub fn index_project(&mut self, src_dir: &str) -> std::io::Result<usize> {
        let src_path = Path::new(src_dir);
        if !src_path.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("目录不存在: {}", src_dir),
            ));
        }

        let mut total_chunks = 0usize;
        let mut files = 0usize;

        for entry in walkdir::WalkDir::new(src_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file() && is_supported_file(e.path()))
        {
            let content = match std::fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let file_path = entry
                .path()
                .strip_prefix(src_path.parent().unwrap_or(src_path))
                .unwrap_or(entry.path())
                .to_string_lossy()
                .to_string();

            let chunks = chunk_by_language(&file_path, &content);
            let count = chunks.len();
            self.retriever.index_batch(chunks);
            total_chunks += count;
            files += 1;
        }

        self.file_count += files;
        Ok(total_chunks)
    }

    pub fn search(&self, query: &str, top_k: usize) -> RetrievalResult {
        self.retriever.search(query, top_k)
    }

    pub fn indexed_count(&self) -> usize {
        self.retriever.indexed_count()
    }

    /// 索引单个文件，自动按扩展名选择切分策略
    pub fn index_file(&mut self, file_path: &str, content: &str) -> usize {
        let chunks = chunk_by_language(file_path, content);
        let count = chunks.len();
        self.retriever.index_batch(chunks);
        self.file_count += 1;
        count
    }

    pub fn get_stats(&self) -> ChunkStats {
        let chunks = self.retriever.all_chunks();
        let mut type_counts: HashMap<String, usize> = HashMap::new();
        let mut language_counts: HashMap<String, usize> = HashMap::new();
        let mut total_lines = 0usize;

        for chunk in chunks {
            *type_counts.entry(chunk.chunk_type.clone()).or_insert(0) += 1;
            *language_counts.entry(chunk.language.clone()).or_insert(0) += 1;
            total_lines += chunk.end_line.saturating_sub(chunk.start_line) + 1;
        }

        let avg_lines = if chunks.is_empty() {
            0.0
        } else {
            total_lines as f32 / chunks.len() as f32
        };

        ChunkStats {
            file_count: self.file_count,
            total_chunks: chunks.len(),
            type_counts,
            language_counts,
            avg_lines,
        }
    }

    pub fn clear(&mut self) {
        self.retriever.clear();
        self.file_count = 0;
    }

    pub fn export_chunks_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self.retriever.all_chunks())
    }

    /// 保存代码片段到 JSON 字符串（持久化用）
    pub fn save_chunks(&self) -> serde_json::Result<String> {
        self.export_chunks_json()
    }

    /// 从 JSON 字符串加载代码片段并重建索引
    ///
    /// 会清空现有索引，然后从 JSON 数据重新构建。
    pub fn load_chunks(&mut self, json: &str) -> serde_json::Result<usize> {
        let chunks: Vec<CodeChunk> = serde_json::from_str(json)?;
        self.clear();
        let count = chunks.len();
        self.retriever.index_batch(chunks);
        self.file_count = count;
        Ok(count)
    }
}

impl Default for CoreManager {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== 测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_project() -> (TempDir, String) {
        let dir = TempDir::new().expect("应创建临时目录");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).expect("应创建 src 目录");

        let f1 = src_dir.join("module_a.rs");
        let mut f = std::fs::File::create(&f1).expect("应创建文件");
        writeln!(
            f,
            r#"
pub struct Container {{
    path: String,
}}

impl Container {{
    pub async fn build(path: &str) -> Self {{
        Self {{ path: path.to_string() }}
    }}

    pub async fn fetch(&self, key: &str) -> Vec<String> {{
        vec![]
    }}
}}
"#
        )
        .expect("应写入");

        let f2 = src_dir.join("settings.rs");
        let mut f = std::fs::File::create(&f2).expect("应创建文件");
        writeln!(
            f,
            r#"
pub struct Settings {{
    pub port: u16,
    pub host: String,
}}

fn defaults() -> Settings {{
    Settings {{ port: 8080, host: "localhost".into() }}
}}
"#
        )
        .expect("应写入");

        // 添加一个 Python 文件
        let f3 = src_dir.join("utils.py");
        let mut f = std::fs::File::create(&f3).expect("应创建文件");
        writeln!(
            f,
            r#"
def get_config(key: str) -> str:
    """获取配置"""
    return "default"

class AppConfig:
    def __init__(self):
        self.port = 8080
"#
        )
        .expect("应写入");

        (dir, src_dir.to_string_lossy().to_string())
    }

    #[test]
    fn test_new() {
        let mgr = CoreManager::new();
        assert_eq!(mgr.indexed_count(), 0);
    }

    #[test]
    fn test_index() {
        let (_guard, src_dir) = create_test_project();
        let mut mgr = CoreManager::new();
        let count = mgr.index_project(&src_dir).expect("应成功索引");
        assert!(count > 0);
        assert_eq!(mgr.indexed_count(), count);
        // 应包含 Rust 和 Python 文件
        let stats = mgr.get_stats();
        assert!(stats.file_count >= 2);
        assert!(stats.language_counts.contains_key("rust"));
        assert!(stats.language_counts.contains_key("python"));
    }

    #[test]
    fn test_search() {
        let (_guard, src_dir) = create_test_project();
        let mut mgr = CoreManager::new();
        mgr.index_project(&src_dir).expect("应成功索引");
        let result = mgr.search("async build", 5);
        assert!(result.returned > 0);
    }

    #[test]
    fn test_stats() {
        let mut mgr = CoreManager::new();
        mgr.index_file("test.rs", "fn a() {}\nstruct B {}\nimpl B {}\n");
        let stats = mgr.get_stats();
        assert_eq!(stats.file_count, 1);
        assert_eq!(stats.total_chunks, 3);
        assert_eq!(stats.type_counts.get("fn"), Some(&1));
        assert_eq!(stats.type_counts.get("struct"), Some(&1));
        assert_eq!(stats.type_counts.get("impl"), Some(&1));
        // language 字段也应正确
        assert_eq!(stats.language_counts.get("rust"), Some(&3));
    }

    #[test]
    fn test_python_index() {
        let mut mgr = CoreManager::new();
        mgr.index_file("test.py", "def a():\n    pass\n\nclass B:\n    pass\n");
        let stats = mgr.get_stats();
        assert_eq!(stats.file_count, 1);
        assert!(stats.total_chunks >= 2);
        assert_eq!(stats.language_counts.get("python"), Some(&2));
    }

    #[test]
    fn test_clear() {
        let mut mgr = CoreManager::new();
        mgr.index_file("test.rs", "fn a() {}\n");
        assert_eq!(mgr.indexed_count(), 1);
        mgr.clear();
        assert_eq!(mgr.indexed_count(), 0);
    }

    #[test]
    fn test_export() {
        let mut mgr = CoreManager::new();
        mgr.index_file("test.rs", "fn hello() {}\n");
        let json = mgr.export_chunks_json().expect("应导出");
        assert!(json.contains("hello"));
        assert!(json.contains("test.rs"));
        assert!(json.contains("\"language\""));
    }

    #[test]
    fn test_default() {
        let mgr = CoreManager::default();
        assert_eq!(mgr.indexed_count(), 0);
    }

    #[test]
    fn test_nonexistent_dir() {
        let mut mgr = CoreManager::new();
        assert!(mgr.index_project("/nonexistent/12345").is_err());
    }

    #[test]
    fn test_save_load_chunks() {
        let mut mgr = CoreManager::new();
        mgr.index_file("test.rs", "fn hello() {}\nstruct Foo {}\n");

        let json = mgr.save_chunks().expect("应成功保存");
        assert!(json.contains("hello"));

        let mut mgr2 = CoreManager::new();
        let count = mgr2.load_chunks(&json).expect("应成功加载");
        assert_eq!(count, 2);
        assert_eq!(mgr2.indexed_count(), 2);

        // 搜索应能命中（使用编码器关键词中的术语）
        let result = mgr2.search("struct", 3);
        assert!(result.returned > 0, "加载后的片段应能搜索到");
    }
}