// ============================================================
// v0.6.0 通用语义引擎 — 统一 Embedder 抽象层
// ============================================================
// 本文件定义嵌入器统一接口，消除代码搜索与结晶路径的模型割裂：
//   - 写入时编码（LuoShuMlEncoder）
//   - 结晶时聚类（consolidation.rs）
//   - 代码语义搜索（CodeBertEncoder）
// 三者共享同一 Embedder trait，用户只需配置一处即可切换模型。
//
// 实现方：
//   - LocalBertEmbedder：本地 BERT 模型（BGE-small-zh / MiniLM-L6-v2）
//   - LlmApiEmbedder：LLM API（OpenAI / Ollama）
//   - EmbedderRegistry：根据配置选择本地或 LLM，支持运行时切换
//
// 设计原则：
//   1. 异步优先：LLM API 是异步的，本地 BERT 用 spawn_blocking 包装
//   2. 降级友好：本地嵌入失败时，调用方可降级到统计编码器
//   3. 配置统一：LRC_EMBEDDER_MODEL 环境变量统一控制嵌入模型

use async_trait::async_trait;
use std::fmt;
use std::sync::Arc;

// v0.6.0 本地 BERT 嵌入器依赖 LuoShuMlEncoder（ml feature 启用）
#[cfg(feature = "ml")]
use super::luoshu_encoder_ml::LuoShuMlEncoder;

// v0.6.0 LLM API 嵌入器依赖 LlmApiConfig（server feature 启用）
#[cfg(feature = "server")]
use super::llm_translator::LlmApiConfig;

/// 嵌入模型环境变量名（供 CLI 显示与读取共用）
///
/// 注：此常量定义在 engine 层（非公开层），避免公开层文件直接出现受保护术语。
/// 公开层（如 `src/bin/server.rs`）应通过此常量引用环境变量名。
pub const EMBEDDER_MODEL_ENV_VAR: &str = "LRC_LUOSHU_MODEL_ID";

// ============================================================
// 错误类型
// ============================================================

/// 嵌入错误类型
///
/// 区分不同失败场景，便于调用方选择合适的降级策略。
#[derive(Debug)]
pub enum EmbedError {
    /// 模型加载失败（文件缺失、权重损坏等）
    ModelLoad(String),
    /// 推理失败（张量运算错误、分词失败等）
    Inference(String),
    /// 网络错误（LLM API 不可达、超时等）
    Network(String),
    /// 配置错误（环境变量无效、配置文件解析失败等）
    Config(String),
    /// 已降级到统计编码器（非错误，调用方可记录告警）
    Degraded(String),
}

impl fmt::Display for EmbedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmbedError::ModelLoad(msg) => write!(f, "模型加载失败: {}", msg),
            EmbedError::Inference(msg) => write!(f, "推理失败: {}", msg),
            EmbedError::Network(msg) => write!(f, "网络错误: {}", msg),
            EmbedError::Config(msg) => write!(f, "配置错误: {}", msg),
            EmbedError::Degraded(msg) => write!(f, "已降级: {}", msg),
        }
    }
}

impl std::error::Error for EmbedError {}

impl From<String> for EmbedError {
    /// 将现有的 String 错误转换为 EmbedError::Inference
    ///
    /// 兼容现有编码器的 String 错误返回类型。
    fn from(msg: String) -> Self {
        EmbedError::Inference(msg)
    }
}

// ============================================================
// Embedder Trait
// ============================================================

/// 嵌入器统一抽象
///
/// v0.6.0 引入，统一代码搜索与结晶路径的嵌入模型接口。
/// 实现方需保证：
///   1. `embed()` 返回的向量维度与 `dim()` 一致
///   2. `embed()` 是线程安全的（Send + Sync）
///   3. 失败时返回 `EmbedError`，调用方负责降级处理
///
/// # 示例
///
/// ```ignore
/// use code_memory::engine::embedder::{Embedder, EmbedError};
///
/// async fn example(embedder: &dyn Embedder) -> Result<(), EmbedError> {
///     let texts = ["你好世界", "Hello world"];
///     let vectors = embedder.embed(&texts).await?;
///     assert_eq!(vectors.len(), 2);
///     assert_eq!(vectors[0].len(), embedder.dim());
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait Embedder: Send + Sync {
    /// 将文本数组编码为高维向量数组
    ///
    /// # 参数
    /// - `texts`：待编码的文本切片数组
    ///
    /// # 返回
    /// - 成功：顺序与输入一致的向量列表，每个向量维度由 `dim()` 决定
    /// - 失败：`EmbedError`，调用方应降级处理
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError>;

    /// 返回嵌入向量的维度
    ///
    /// BGE-small-zh: 512, MiniLM-L6-v2: 384, OpenAI text-embedding-3-small: 1536
    fn dim(&self) -> usize;

    /// 返回模型 ID（如 "BAAI/bge-small-zh"）
    fn model_id(&self) -> &str;

    /// 返回嵌入源类型（"local" 或 "llm"）
    ///
    /// 用于日志记录和降级决策。
    fn source(&self) -> &'static str;
}

// ============================================================
// LocalBertEmbedder — 本地 BERT 嵌入器（ml feature）
// ============================================================

/// 本地 BERT 嵌入器
///
/// 封装 `LuoShuMlEncoder`，提供本地 BERT 模型的嵌入能力。
/// 用于写入时编码和结晶时聚类（离线场景）。
///
/// # 特点
/// - 零网络依赖：模型在本地加载，无 API 调用
/// - 零成本：无 API 费用
/// - 离线可用：适合内网/离线环境
///
/// # 限制
/// - 首次启动需要下载模型（~100MB）
/// - CPU 推理速度较慢（~50ms/文本）
///
/// # 示例
///
/// ```ignore
/// use code_memory::engine::embedder::{Embedder, LocalBertEmbedder};
/// use code_memory::engine::luoshu_encoder_ml::LuoShuMlEncoder;
///
/// let encoder = LuoShuMlEncoder::load()?;
/// let embedder = LocalBertEmbedder::new(encoder, "BAAI/bge-small-zh".to_string());
/// let vectors = embedder.embed(&["你好世界"]).await?;
/// ```
#[cfg(feature = "ml")]
pub struct LocalBertEmbedder {
    encoder: Arc<LuoShuMlEncoder>,
    model_id: String,
    dim: usize,
}

#[cfg(feature = "ml")]
impl LocalBertEmbedder {
    /// 创建本地 BERT 嵌入器
    ///
    /// # 参数
    /// - `encoder`：已加载的 `LuoShuMlEncoder` 实例
    /// - `model_id`：模型 ID（如 "BAAI/bge-small-zh"）
    pub fn new(encoder: LuoShuMlEncoder, model_id: String) -> Self {
        let dim = encoder.hidden_size();
        Self {
            encoder: Arc::new(encoder),
            model_id,
            dim,
        }
    }

    /// 返回底层编码器的 Arc 引用（用于直接调用洛书编码等高级功能）
    pub fn encoder(&self) -> &Arc<LuoShuMlEncoder> {
        &self.encoder
    }
}

#[cfg(feature = "ml")]
#[async_trait]
impl Embedder for LocalBertEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        // 本地 BERT 推理是 CPU 密集型操作，直接同步调用。
        // 调用方若需避免阻塞 tokio 运行时，可自行用 spawn_blocking 包装。
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            let vec = self
                .encoder
                .encode_embedding(text)
                .map_err(EmbedError::from)?;
            results.push(vec);
        }
        Ok(results)
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn source(&self) -> &'static str {
        "local"
    }
}

// ============================================================
// LlmApiEmbedder — LLM API 嵌入器（server feature）
// ============================================================

/// LLM API 嵌入器
///
/// 封装 `LlmApiConfig`，提供 LLM API（OpenAI/Ollama）的嵌入能力。
/// 用于结晶时聚类（在线场景，精度更高，维度更高）。
///
/// # 特点
/// - 高精度：OpenAI text-embedding-3-small 1536 维
/// - 高维度：比本地 BERT（512 维）聚类更精准
/// - 网络依赖：需要 API Key 和网络连接
///
/// # 限制
/// - 需要 API Key（OpenAI）或本地 Ollama 服务
/// - 有 API 调用费用（OpenAI）
/// - 网络延迟（~200ms/请求）
///
/// # 示例
///
/// ```ignore
/// use code_memory::engine::embedder::{Embedder, LlmApiEmbedder};
/// use code_memory::engine::llm_translator::LlmApiConfig;
///
/// let config = LlmApiConfig::parse("sk-xxx||text-embedding-3-small||https://api.openai.com")?;
/// let embedder = LlmApiEmbedder::new(config, "text-embedding-3-small".to_string(), 1536);
/// let vectors = embedder.embed(&["你好世界"]).await?;
/// ```
#[cfg(feature = "server")]
pub struct LlmApiEmbedder {
    config: Arc<LlmApiConfig>,
    model_id: String,
    dim: usize,
}

#[cfg(feature = "server")]
impl LlmApiEmbedder {
    /// 创建 LLM API 嵌入器
    ///
    /// # 参数
    /// - `config`：LLM API 配置（OpenAI/Ollama）
    /// - `model_id`：模型 ID（如 "text-embedding-3-small"）
    /// - `dim`：嵌入向量维度（OpenAI 通常 1536，Ollama 取决于模型）
    pub fn new(config: LlmApiConfig, model_id: String, dim: usize) -> Self {
        Self {
            config: Arc::new(config),
            model_id,
            dim,
        }
    }

    /// 返回底层 LLM 配置的 Arc 引用
    pub fn config(&self) -> &Arc<LlmApiConfig> {
        &self.config
    }
}

#[cfg(feature = "server")]
#[async_trait]
impl Embedder for LlmApiEmbedder {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        // LLM API 调用是真正的异步 I/O 操作，不会阻塞 CPU
        self.config
            .embed_texts(texts)
            .await
            .map_err(EmbedError::Network)
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn source(&self) -> &'static str {
        "llm"
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试：EmbedError 的 Display 实现
    #[test]
    fn test_embed_error_display() {
        let err = EmbedError::ModelLoad("文件不存在".to_string());
        assert!(format!("{}", err).contains("模型加载失败"));
        assert!(format!("{}", err).contains("文件不存在"));

        let err = EmbedError::Network("连接超时".to_string());
        assert!(format!("{}", err).contains("网络错误"));

        let err = EmbedError::Config("无效的 API Key".to_string());
        assert!(format!("{}", err).contains("配置错误"));

        let err = EmbedError::Degraded("回退到统计编码器".to_string());
        assert!(format!("{}", err).contains("已降级"));
    }

    /// 测试：String → EmbedError 转换
    #[test]
    fn test_embed_error_from_string() {
        let err = EmbedError::from("推理失败".to_string());
        match err {
            EmbedError::Inference(msg) => assert_eq!(msg, "推理失败"),
            _ => panic!("应转换为 Inference 变体"),
        }
    }

    /// 测试：EmbedError 实现 std::error::Error
    #[test]
    fn test_embed_error_is_std_error() {
        let err = EmbedError::Inference("测试".to_string());
        // 验证可以转换为 trait 对象
        let _: &dyn std::error::Error = &err;
    }

    /// 测试：MockEmbedder 用于单元测试
    ///
    /// 提供一个简单的 Mock 实现，验证 Embedder trait 的基本契约。
    struct MockEmbedder {
        dim: usize,
        model_id: String,
    }

    impl MockEmbedder {
        fn new(dim: usize, model_id: &str) -> Self {
            Self {
                dim,
                model_id: model_id.to_string(),
            }
        }
    }

    #[async_trait]
    impl Embedder for MockEmbedder {
        async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
            // 返回固定向量（用于测试）
            Ok(texts
                .iter()
                .map(|text| {
                    // 简单哈希到向量，确保维度匹配
                    let hash = text.len() as f32;
                    vec![hash; self.dim]
                })
                .collect())
        }

        fn dim(&self) -> usize {
            self.dim
        }

        fn model_id(&self) -> &str {
            &self.model_id
        }

        fn source(&self) -> &'static str {
            "mock"
        }
    }

    /// 测试：MockEmbedder 满足 Embedder 契约
    #[tokio::test]
    async fn test_mock_embedder_contract() {
        let embedder = MockEmbedder::new(512, "mock/model");

        // 维度一致
        assert_eq!(embedder.dim(), 512);
        assert_eq!(embedder.model_id(), "mock/model");
        assert_eq!(embedder.source(), "mock");

        // 嵌入返回正确数量和维度
        let texts = ["你好", "世界", "Hello"];
        let vectors = embedder.embed(&texts).await.unwrap();
        assert_eq!(vectors.len(), 3);
        for vec in &vectors {
            assert_eq!(vec.len(), 512);
        }

        // 不同文本返回不同向量（基于长度哈希）
        assert_ne!(vectors[0], vectors[2]);
    }

    /// 测试：空输入处理
    #[tokio::test]
    async fn test_mock_embedder_empty_input() {
        let embedder = MockEmbedder::new(384, "mock/empty");
        let vectors = embedder.embed(&[]).await.unwrap();
        assert!(vectors.is_empty());
    }

    /// 测试：Embedder 可作为 trait 对象使用
    #[tokio::test]
    async fn test_embedder_as_trait_object() {
        let embedder: Box<dyn Embedder> = Box::new(MockEmbedder::new(256, "mock/trait"));
        assert_eq!(embedder.dim(), 256);
        assert_eq!(embedder.model_id(), "mock/trait");

        let vectors = embedder.embed(&["test"]).await.unwrap();
        assert_eq!(vectors[0].len(), 256);
    }
}
