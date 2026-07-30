// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心编排逻辑，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
/// 编排模块
/// 整合切分、编码、检索三阶段流水线。
/// 按文件扩展名自动选择多语言切分策略。
use crate::chunker::{chunk_by_language, is_supported_file, CodeChunk};
use crate::engine::encoder::{CodeEncoder, EmbeddingVector, FastEncoder};
use crate::engine::retriever::{CodeRetriever, LocalRetriever, RetrievalResult, ScoredChunk};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// v0.5.12 新增：判断目录是否应被忽略（不索引）
///
/// 排除依赖目录、构建产物、版本控制目录等，减少内存占用和索引时间。
/// 这些目录通常包含大量自动生成的文件，对代码记忆无意义。
fn is_ignored_dir(path: &Path) -> bool {
    // 只检查目录名（最后一级）
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let name_lower = name.to_lowercase();
        return matches!(
            name_lower.as_str(),
            // 依赖目录
            "node_modules"
            | ".cargo"
            | "vendor"
            | "bower_components"
            // 构建产物
            | "target"
            | "dist"
            | "build"
            | ".next"
            | ".nuxt"
            | ".output"
            | "__pycache__"
            | ".pytest_cache"
            | ".mypy_cache"
            | "bin"
            | "obj"
            // 版本控制
            | ".git"
            | ".svn"
            | ".hg"
            // IDE/工具缓存
            | ".idea"
            | ".vscode"
            | ".vs"
            // 其他
            | ".cache"
            | "coverage"
            | ".nyc_output"
        );
    }
    false
}

/// 索引统计
#[derive(Debug, Clone, Serialize)]
pub struct ChunkStats {
    pub file_count: usize,
    pub total_chunks: usize,
    pub type_counts: HashMap<String, usize>,
    pub language_counts: HashMap<String, usize>,
    pub avg_lines: f32,
}

/// 嵌入向量缓存（保存到磁盘，重启时秒加载）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EmbeddingCache {
    /// 代码片段（chunker 输出是确定性的，同一源码产生相同 chunk）
    chunks: Vec<CodeChunk>,
    /// 对应的嵌入向量
    vectors: Vec<Vec<f32>>,
}

impl EmbeddingCache {
    /// 缓存文件路径：<data_dir>/../cache/embedding_cache.json
    fn cache_path(data_dir: &str) -> PathBuf {
        Path::new(data_dir)
            .parent()
            .unwrap_or(Path::new("."))
            .join("cache")
            .join("embedding_cache.json")
    }

    fn save(&self, data_dir: &str) -> Result<(), String> {
        let path = Self::cache_path(data_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建缓存目录失败: {}", e))?;
        }
        let json = serde_json::to_string(self).map_err(|e| format!("缓存序列化失败: {}", e))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("写入缓存文件失败: {} → {}", e, path.display()))?;
        Ok(())
    }

    fn load(data_dir: &str) -> Option<Self> {
        let path = Self::cache_path(data_dir);
        if !path.exists() {
            return None;
        }
        let json = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&json).ok()
    }
}

/// 核心编排器
pub struct CoreManager<E: CodeEncoder = FastEncoder> {
    retriever: LocalRetriever<E>,
    file_count: usize,
}

impl CoreManager {
    pub fn new() -> Self {
        let terms = vec![
            // ==================== Rust 语言关键字 ====================
            "fn",
            "struct",
            "impl",
            "trait",
            "enum",
            "mod",
            "pub",
            "async",
            "await",
            "match",
            "loop",
            "for",
            "while",
            "if",
            "else",
            "mut",
            "ref",
            "move",
            "clone",
            "copy",
            "drop",
            "deref",
            "borrow",
            "crate",
            "super",
            "dyn",
            "where",
            "into",
            "from",
            "macro",
            "derive",
            "new",
            "static",
            "unsafe",
            "union",
            "use",
            "let",
            "const",
            "type",
            "self",
            "return",
            "true",
            "false",
            "some",
            "none",
            "ok",
            "err",
            "result",
            "option",
            "box",
            "arc",
            "rc",
            "cell",
            "mutex",
            "rwlock",
            "channel",
            "thread",
            "spawn",
            "tokio",
            "serde",
            "ownership",
            "lifetime",
            "concurrent",
            "parallel",
            // ==================== 通用编程关键词 ====================
            "def",
            "class",
            "function",
            "export",
            "import",
            "var",
            "interface",
            "abstract",
            "virtual",
            "override",
            "extends",
            "implements",
            "throw",
            "try",
            "catch",
            "finally",
            "yield",
            "pass",
            "break",
            "continue",
            "null",
            "nil",
            "void",
            "string",
            "number",
            "integer",
            "float",
            "boolean",
            "array",
            "list",
            "map",
            "hash",
            "set",
            "vector",
            "iterator",
            "collection",
            "input",
            "output",
            "file",
            "read",
            "write",
            "open",
            "close",
            "stream",
            "buffer",
            "parse",
            "serialize",
            "deserialize",
            "convert",
            "transform",
            "validate",
            "process",
            "execute",
            "invoke",
            "call",
            "send",
            "receive",
            "connect",
            "disconnect",
            "register",
            "unregister",
            "subscribe",
            "publish",
            "notify",
            "trigger",
            "listen",
            "watch",
            "observe",
            "monitor",
            "track",
            "create",
            "update",
            "delete",
            "remove",
            "insert",
            "find",
            "replace",
            "merge",
            "split",
            "sort",
            "filter",
            "group",
            "aggregate",
            "calculate",
            "compute",
            "generate",
            "apply",
            "resolve",
            // ==================== 机器学习 / AI ====================
            "loss",
            "gradient",
            "model",
            "training",
            "train",
            "inference",
            "evaluate",
            "accuracy",
            "epoch",
            "batch",
            "dataset",
            "optimizer",
            "learning_rate",
            "weight",
            "bias",
            "activation",
            "layer",
            "neural",
            "deep",
            "embedding",
            "token",
            "predict",
            "classify",
            "classification",
            "regression",
            "validate",
            "validation",
            "overfit",
            "underfit",
            "normalize",
            "softmax",
            "relu",
            "sigmoid",
            "tanh",
            "dropout",
            "backprop",
            "forward",
            "backward",
            "tensor",
            "feature",
            "label",
            "target",
            "metric",
            "f1",
            "precision",
            "recall",
            "attention",
            "transformer",
            "encoder",
            "decoder",
            "llama",
            "gpt",
            "bert",
            "diffusion",
            "generator",
            "discriminator",
            "latent",
            "sample",
            "reward",
            "policy",
            "agent",
            "state",
            "action",
            "value",
            "entropy",
            "log_prob",
            "likelihood",
            "prior",
            "posterior",
            "bayesian",
            "gaussian",
            "categorical",
            "vocabulary",
            "corpus",
            "similarity",
            "distance",
            "cosine",
            "euclidean",
            "cluster",
            "centroid",
            "pipeline",
            "preprocessing",
            "scaling",
            "augmentation",
            "prompt",
            "completion",
            "temperature",
            "top_p",
            "top_k",
            "sampling",
            "context",
            "reasoning",
            "chain",
            "thought",
            "alignment",
            "rlhf",
            "dpo",
            "ppo",
            "sft",
            "lora",
            "quantization",
            "distillation",
            "pruning",
            "gpu",
            "cuda",
            "fp16",
            "bf16",
            "int8",
            "int4",
            "torch",
            "tensorflow",
            "pytorch",
            "numpy",
            "pandas",
            "huggingface",
            // ==================== Web / HTTP ====================
            "request",
            "response",
            "header",
            "body",
            "status",
            "route",
            "router",
            "middleware",
            "cookie",
            "auth",
            "cors",
            "json",
            "xml",
            "get",
            "post",
            "put",
            "delete",
            "patch",
            "rest",
            "graphql",
            "websocket",
            "http",
            "api",
            "url",
            "uri",
            "endpoint",
            "payload",
            "ssl",
            "tls",
            "redirect",
            "proxy",
            "gateway",
            "rate_limit",
            "throttle",
            "timeout",
            "retry",
            "backoff",
            "health_check",
            "metric",
            "alert",
            "log",
            "span",
            // ==================== 数据库 / 存储 ====================
            "database",
            "db",
            "table",
            "column",
            "row",
            "query",
            "select",
            "join",
            "index",
            "primary",
            "foreign",
            "transaction",
            "commit",
            "rollback",
            "migrate",
            "schema",
            "sql",
            "redis",
            "postgres",
            "mysql",
            "mongodb",
            "sqlite",
            "pool",
            "connection",
            "cursor",
            "backup",
            "restore",
            "replication",
            "partition",
            "lock",
            "deadlock",
            // ==================== 测试 / 调试 ====================
            "test",
            "assert",
            "expect",
            "mock",
            "fixture",
            "stub",
            "integration",
            "unit",
            "coverage",
            "benchmark",
            "profile",
            "error",
            "warn",
            "info",
            "debug",
            "trace",
            "panic",
            "exception",
            "stack",
            // ==================== DevOps / 工具链 ====================
            "cargo",
            "npm",
            "pnpm",
            "yarn",
            "git",
            "commit",
            "push",
            "pull",
            "branch",
            "merge",
            "rebase",
            "deploy",
            "docker",
            "container",
            "ci",
            "cd",
            "pipeline",
            "yaml",
            "toml",
            "env",
            "environment",
            "build",
            "init",
            "setup",
            "cli",
            "release",
            "version",
            "package",
            "publish",
            "registry",
            "module",
            "plugin",
            "extension",
            "framework",
            "library",
            "workspace",
            // ==================== Loong 内部术语 ====================
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
            "load",
            "save",
            "recall",
            "remember",
            "forget",
            "luoshu",
            "bagua",
            "topology",
            "crystallize",
            "compose",
            "unfold",
            "reflect",
            "mirror",
            "evolution",
            "governance",
            "constitution",
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

        // v0.5.12 修复：排除大目录和二进制文件目录，减少内存占用
        // 根因：旧逻辑遍历所有目录（包括 node_modules、target、.git 等），
        //       导致大量文件被读取到内存，sidecar 内存占用过大
        for entry in walkdir::WalkDir::new(src_dir)
            .into_iter()
            .filter_entry(|e| !is_ignored_dir(e.path()))
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file() && is_supported_file(e.path()))
        {
            // v0.5.12 新增：跳过过大的文件（> 1MB），避免内存浪费
            if let Ok(metadata) = entry.metadata() {
                if metadata.len() > 1_048_576 {
                    continue;
                }
            }

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

    /// 多关键词合并检索：对每个关键词独立检索，合并去重，按相似度排序
    pub fn multi_keyword_search(&self, keywords: &[String], top_k: usize) -> RetrievalResult {
        if keywords.is_empty() {
            return RetrievalResult {
                query: String::new(),
                results: vec![],
                returned: 0,
                total_indexed: self.retriever.indexed_count(),
            };
        }

        if keywords.len() == 1 {
            return self.search(&keywords[0], top_k);
        }

        let query_str = keywords.join(", ");
        let mut all_results = Vec::new();
        // 按关键词分段检索，每个关键词取 top_k 条
        for keyword in keywords {
            let result = self.search(keyword, top_k);
            all_results.extend(result.results);
        }

        // 去重：按 (file_path, start_line, end_line) 唯一标识
        let mut seen = std::collections::HashSet::new();
        let mut unique: Vec<_> = all_results
            .into_iter()
            .filter(|r| {
                let key = (
                    r.chunk.file_path.clone(),
                    r.chunk.start_line,
                    r.chunk.end_line,
                );
                seen.insert(key)
            })
            .collect();

        // 按相似度降序排序
        unique.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 截取 top_k
        let returned = unique.len().min(top_k);
        unique.truncate(returned);

        // 重新编号
        let results: Vec<_> = unique
            .into_iter()
            .enumerate()
            .map(|(i, mut r)| {
                r.rank = i + 1;
                r
            })
            .collect();

        let total_indexed = self.retriever.indexed_count();

        RetrievalResult {
            query: query_str,
            returned: results.len(),
            total_indexed,
            results,
        }
    }

    pub fn indexed_count(&self) -> usize {
        self.retriever.indexed_count()
    }

    /// v0.6.1 P0-2 修复: 获取最近索引的 N 条代码片段
    ///
    /// 用于 /v1/code/search 的 query 为空时的回退逻辑,
    /// 避免导出功能在无查询参数时返回空结果导致功能失效。
    ///
    /// 返回最近索引的 top_k 条 chunks(按索引顺序逆序),score=1.0 表示"推荐"而非相似度。
    pub fn recent_chunks(&self, top_k: usize) -> RetrievalResult {
        let all = self.retriever.all_chunks();
        let total_indexed = all.len();

        // 取最后 top_k 条(最近索引的),并逆序使最新的在前
        let start = total_indexed.saturating_sub(top_k);
        let recent: Vec<ScoredChunk> = all[start..]
            .iter()
            .rev()
            .enumerate()
            .map(|(i, chunk)| ScoredChunk {
                chunk: chunk.clone(),
                score: 1.0, // 回退结果无相似度评分,用 1.0 标记为"推荐"
                rank: i + 1,
            })
            .collect();

        let returned = recent.len();
        RetrievalResult {
            query: String::new(), // 空查询,与调用方一致
            returned,
            total_indexed,
            results: recent,
        }
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

    /// 保存嵌入向量缓存到磁盘（跳过后续启动的重复编码）
    ///
    /// 缓存文件位于 <data_dir>/../cache/embedding_cache.json
    pub fn save_embedding_cache(&self, data_dir: &str) -> Result<(), String> {
        let chunks = self.retriever.all_chunks().to_vec();
        let vectors = self
            .retriever
            .get_vectors()
            .iter()
            .map(|v| v.values.clone())
            .collect();

        let cache = EmbeddingCache { chunks, vectors };
        cache.save(data_dir)
    }

    /// 从磁盘加载嵌入向量缓存（跳过编码，秒级恢复索引）
    ///
    /// 返回加载的片段数量，或 None 表示缓存不存在/无效。
    pub fn load_embedding_cache(&mut self, data_dir: &str) -> Option<usize> {
        let cache = EmbeddingCache::load(data_dir)?;
        if cache.chunks.is_empty() || cache.vectors.len() != cache.chunks.len() {
            return None;
        }

        let dim = self
            .retriever
            .get_vectors()
            .first()
            .map(|v| v.dim)
            .unwrap_or(0);
        let vectors: Vec<EmbeddingVector> = cache
            .vectors
            .into_iter()
            .map(|values| EmbeddingVector {
                dim: values.len(),
                values,
            })
            .collect();

        // 校验维度一致性
        if let Some(first) = vectors.first() {
            if dim > 0 && first.dim != dim {
                return None; // 维度不匹配，放弃缓存
            }
        }

        let count = vectors.len();
        self.retriever.load_from_vectors(vectors, cache.chunks);
        self.file_count = count;
        Some(count)
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
