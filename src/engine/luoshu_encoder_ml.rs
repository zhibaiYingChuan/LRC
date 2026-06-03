// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 洛书坐标编码器 — ML 模式（真实 BERT Embedding）
//
// 洛书编码器 ML 增强模式：
//   使用轻量级嵌入模型（如 BAAI/bge-small-zh）将文本转为高维向量，
//   通过投影矩阵降维至 9 维，施加幻和正则化约束。
//
// 与统计版 `LuoShuEncoder` 的区别：
//   统计版：词频 + 字符熵 + 位置权重 → 9 维
//   ML 版：  BERT 768/384 维 → 投影矩阵 W(hidden×9) → 9 维 → 幻和归一化
//
// 默认模型: sentence-transformers/all-MiniLM-L6-v2 (384维, 轻量, 多语言)
// 可通过环境变量覆盖:
//   LRC_LUOSHU_MODEL_ID=BAAI/bge-small-zh  (中文专用)
//   LRC_LUOSHU_MODEL_ID=sentence-transformers/all-MiniLM-L6-v2  (默认)

use super::luoshu_encoder::{LuoShuEncoder, LuoShuVector, LUOSHU_WEIGHTS};
use candle_core::{Device, Tensor};
use std::sync::Arc;

/// 洛书编码器 ML 增强器
///
/// 使用真实的 BERT 语义模型进行文本编码，提供比统计特征
/// 更精准的语义区分能力。
pub struct LuoShuMlEncoder {
    /// 底层 BERT 模型（通过 candle 加载）
    model: candle_transformers::models::bert::BertModel,
    /// 分词器
    tokenizer: tokenizers::Tokenizer,
    /// 计算设备（CPU/CUDA）
    device: Device,
    /// 池化策略：CLS 或 Mean
    pooling: PoolingStrategy,
    /// 投影矩阵 W ∈ R^(hidden_size × 9)，将 BERT 隐藏层映射到洛书 9 维
    projection: Vec<Vec<f32>>,
    /// 实际隐藏层维度
    hidden_size: usize,
}

/// ML 编码器池化策略
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PoolingStrategy {
    Cls,
    Mean,
}

impl LuoShuMlEncoder {
    /// 加载默认的轻量级多语言模型
    ///
    /// 加载策略（与 CodeBertEncoder 一致）：
    /// 1. 检查 `models/` 本地文件夹
    /// 2. 检查 HuggingFace 缓存
    /// 3. 从 HF_ENDPOINT 镜像下载
    pub fn load() -> Result<Self, String> {
        let device = Device::Cpu;

        let model_id = std::env::var("LRC_LUOSHU_MODEL_ID")
            .unwrap_or_else(|_| "sentence-transformers/all-MiniLM-L6-v2".to_string());

        let local_model_name = model_id.replace('/', "--");

        let project_root = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        let local_model_dir = project_root.join("models").join(&local_model_name);

        let mut use_local = false;
        let mut model_dir = std::path::PathBuf::new();

        for dir in [&local_model_dir] {
            let dir = dir.clone();
            if dir.join("config.json").exists()
                && (dir.join("model.safetensors").exists()
                    || dir.join("pytorch_model.bin").exists())
            {
                use_local = true;
                model_dir = dir.clone();
                eprintln!("[LRC·洛书ML] 使用本地模型: {}", model_dir.display());
                break;
            }
        }

        // 加载分词器
        let tokenizer = if use_local {
            let tokenizer_path = model_dir.join("tokenizer.json");
            tokenizers::Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| format!("加载本地分词器失败: {}", e))?
        } else {
            let api = hf_hub::api::sync::Api::new()
                .map_err(|e| format!("连接 HF Hub 失败: {}", e))?;
            let repo = api.model(model_id.clone());
            let tokenizer_path = repo.get("tokenizer.json")
                .map_err(|e| format!("下载分词器失败: {}", e))?;
            tokenizers::Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| format!("解析分词器失败: {}", e))?
        };

        // 加载模型
        let (model, hidden_size) = if use_local {
            let config_path = model_dir.join("config.json");
            let config_content = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("读取配置失败: {}", e))?;
            let config: serde_json::Value = serde_json::from_str(&config_content)
                .map_err(|e| format!("解析配置失败: {}", e))?;
            let hidden_size = config["hidden_size"]
                .as_u64()
                .unwrap_or(384) as usize;

            let weights_path = if model_dir.join("model.safetensors").exists() {
                model_dir.join("model.safetensors")
            } else {
                model_dir.join("pytorch_model.bin")
            };

            let vb = unsafe {
                candle_nn::VarBuilder::from_mmaped_safetensors(
                    &[&weights_path],
                    candle_core::DType::F32,
                    &device,
                )
                .map_err(|e| format!("加载本地模型失败: {}", e))?
            };

            let model = candle_transformers::models::bert::BertModel::load(vb, &Default::default())
                .map_err(|e| format!("构建 BERT 模型失败: {}", e))?;

            (model, hidden_size)
        } else {
            let api = hf_hub::api::sync::Api::new()
                .map_err(|e| format!("连接 HF Hub 失败: {}", e))?;
            let repo = api.model(model_id);

            let config_path = repo.get("config.json")
                .map_err(|e| format!("下载配置失败: {}", e))?;
            let config_content = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("读取配置失败: {}", e))?;
            let config: serde_json::Value = serde_json::from_str(&config_content)
                .map_err(|e| format!("解析配置失败: {}", e))?;
            let hidden_size = config["hidden_size"]
                .as_u64()
                .unwrap_or(384) as usize;

            let weights_path = repo.get("model.safetensors")
                .map_err(|e| format!("下载模型失败: {}", e))?;
            let vb = unsafe {
                candle_nn::VarBuilder::from_mmaped_safetensors(
                    &[&weights_path],
                    candle_core::DType::F32,
                    &device,
                )
                .map_err(|e| format!("加载模型权重失败: {}", e))?
            };

            let model = candle_transformers::models::bert::BertModel::load(vb, &Default::default())
                .map_err(|e| format!("构建 BERT 模型失败: {}", e))?;

            (model, hidden_size)
        };

        // 初始化投影矩阵
        let projection = Self::init_projection(hidden_size);

        eprintln!(
            "[LRC·洛书ML] 模型加载完成: {} (hidden_size={}, 池化=Mean)",
            if use_local { "本地" } else { "远程" },
            hidden_size
        );

        Ok(Self {
            model,
            tokenizer,
            device,
            pooling: PoolingStrategy::Mean,
            projection,
            hidden_size,
        })
    }

    /// 初始化投影矩阵 W ∈ R^(hidden_size × 9)
    fn init_projection(hidden_size: usize) -> Vec<Vec<f32>> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let bound = (6.0_f32 / (hidden_size as f32 + 9.0)).sqrt();
        let mut proj = vec![vec![0.0f32; 9]; hidden_size];

        for i in 0..hidden_size {
            for j in 0..9 {
                let mut hasher = DefaultHasher::new();
                (i * 9 + j).hash(&mut hasher);
                let seed = hasher.finish() as f32 / u64::MAX as f32;
                proj[i][j] = (seed - 0.5) * 2.0 * bound;
            }
        }
        proj
    }

    /// 使用 ML 模型将文本编码为洛书 9 维向量
    pub fn encode_text(&self, text: &str) -> Result<LuoShuVector, String> {
        // 1. Tokenize
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| format!("分词失败: {}", e))?;

        let token_ids: Vec<u32> = encoding.get_ids().iter().map(|&id| id).collect();
        let attention_mask: Vec<f32> = encoding.get_attention_mask()
            .iter()
            .map(|&m| m as f32)
            .collect();

        let seq_len = token_ids.len().min(512);

        // 2. 创建输入张量（与 CodeBertEncoder 相同的模式）
        let input_ids = Tensor::new(
            &token_ids[..seq_len],
            &self.device,
        )
        .map_err(|e| format!("创建 input_ids: {}", e))?
        .unsqueeze(0)
        .map_err(|e| format!("unsqueeze: {}", e))?;

        let token_type_ids = input_ids
            .zeros_like()
            .map_err(|e| format!("type_ids: {}", e))?;

        let attention_tensor = Tensor::new(
            &attention_mask[..seq_len],
            &self.device,
        )
        .map_err(|e| format!("attention: {}", e))?
        .unsqueeze(0)
        .map_err(|e| format!("unsqueeze: {}", e))?;

        // 3. BERT 前向传播
        let output = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_tensor))
            .map_err(|e| format!("BERT 前向: {}", e))?;

        // 4. 池化
        let embedding = match self.pooling {
            PoolingStrategy::Cls => {
                // [CLS] token 是第 0 个位置
                output
                    .get(0)
                    .map_err(|e| format!("batch: {}", e))?
                    .get(0)
                    .map_err(|e| format!("cls: {}", e))?
            }
            PoolingStrategy::Mean => {
                let mask = attention_tensor
                    .unsqueeze(2)
                    .map_err(|e| format!("mask unsqueeze: {}", e))?;
                let masked = output
                    .broadcast_mul(&mask)
                    .map_err(|e| format!("masked mul: {}", e))?;
                let sum = masked.sum(1).map_err(|e| format!("sum: {}", e))?;
                let mask_sum = mask.sum(1).map_err(|e| format!("mask_sum: {}", e))?;
                sum.broadcast_div(&mask_sum)
                    .map_err(|e| format!("div: {}", e))?
            }
        };

        // 展平为 1D 向量
        let emb_vec: Vec<f32> = embedding
            .flatten_all()
            .map_err(|e| format!("flatten: {}", e))?
            .to_vec1()
            .map_err(|e| format!("to_vec1: {}", e))?;

        let actual_hidden = self.hidden_size.min(emb_vec.len());

        // 5. 投影：hidden_size → 9
        let mut raw_features = [0.0f32; 9];
        for j in 0..9 {
            let mut sum = 0.0f32;
            for i in 0..actual_hidden {
                sum += emb_vec[i] * self.projection[i][j];
            }
            raw_features[j] = sum;
        }

        // 6. 贝叶斯融合：先验（洛书标准权重）× 似然（ML 投影）
        let mut posterior = [0.0f32; 9];
        for i in 0..9 {
            let likelihood = raw_features[i].max(0.0);
            posterior[i] = LUOSHU_WEIGHTS[i] * (1.0 + likelihood);
        }

        // 7. 归一化
        let total: f32 = posterior.iter().sum();
        if total > 1e-6 {
            for v in posterior.iter_mut() {
                *v /= total;
            }
        } else {
            posterior = [1.0 / 9.0; 9];
        }

        let mut vec = LuoShuVector { values: posterior };
        vec.normalize_to_luoshu();
        Ok(vec)
    }

    /// 获取底层 BERT 编码器的句嵌入（未经投影，用于其他语义场景）
    pub fn encode_embedding(&self, text: &str) -> Result<Vec<f32>, String> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| format!("分词失败: {}", e))?;

        let token_ids: Vec<u32> = encoding.get_ids().iter().map(|&id| id).collect();
        let attention_mask: Vec<f32> = encoding.get_attention_mask()
            .iter()
            .map(|&m| m as f32)
            .collect();
        let seq_len = token_ids.len().min(512);

        let input_ids = Tensor::new(&token_ids[..seq_len], &self.device)
            .map_err(|e| format!("input_ids: {}", e))?
            .unsqueeze(0)
            .map_err(|e| format!("unsqueeze: {}", e))?;

        let token_type_ids = input_ids.zeros_like()
            .map_err(|e| format!("type_ids: {}", e))?;

        let attention_tensor = Tensor::new(&attention_mask[..seq_len], &self.device)
            .map_err(|e| format!("attention: {}", e))?
            .unsqueeze(0)
            .map_err(|e| format!("unsqueeze: {}", e))?;

        let output = self.model
            .forward(&input_ids, &token_type_ids, Some(&attention_tensor))
            .map_err(|e| format!("forward: {}", e))?;

        let mask = attention_tensor
            .unsqueeze(2)
            .map_err(|e| format!("mask unsqueeze: {}", e))?;
        let masked = output.broadcast_mul(&mask)
            .map_err(|e| format!("masked mul: {}", e))?;
        let sum = masked.sum(1).map_err(|e| format!("sum: {}", e))?;
        let mask_sum = mask.sum(1).map_err(|e| format!("mask_sum: {}", e))?;
        let pooled = sum.broadcast_div(&mask_sum)
            .map_err(|e| format!("div: {}", e))?;

        pooled.flatten_all()
            .map_err(|e| format!("flatten: {}", e))?
            .to_vec1()
            .map_err(|e| format!("to_vec1: {}", e))
    }
}

/// 创建带 ML 编码器的洛书编码器组合
///
/// 当 `ml` feature 启用时，优先使用 ML 编码器；
/// 如果 ML 编码器不可用（模型未加载），自动回退到统计编码器。
pub struct HybridLuoShuEncoder {
    /// ML 编码器（可选，模型未加载时为 None）
    ml_encoder: Option<Arc<LuoShuMlEncoder>>,
    /// 统计编码器（始终可用，用作回退）
    fallback: LuoShuEncoder,
}

impl HybridLuoShuEncoder {
    /// 创建混合编码器（仅统计模式）
    pub fn new_statistical() -> Self {
        Self {
            ml_encoder: None,
            fallback: LuoShuEncoder::new(),
        }
    }

    /// 创建混合编码器（尝试加载 ML 模型）
    pub fn new_with_ml(ml_encoder: LuoShuMlEncoder) -> Self {
        Self {
            ml_encoder: Some(Arc::new(ml_encoder)),
            fallback: LuoShuEncoder::new(),
        }
    }

    /// 编码文本为洛书向量
    ///
    /// 优先使用 ML 编码器，失败时自动回退到统计编码器。
    pub fn encode_text(&self, text: &str) -> LuoShuVector {
        if let Some(ref ml) = self.ml_encoder {
            match ml.encode_text(text) {
                Ok(vec) => return vec,
                Err(e) => {
                    eprintln!("[LRC·洛书] ML 编码失败 ({}), 回退到统计编码器", e);
                }
            }
        }
        self.fallback.encode_text(text)
    }

    /// 检查是否使用 ML 模式
    pub fn is_ml_mode(&self) -> bool {
        self.ml_encoder.is_some()
    }

    /// 获取幻和偏离度（监控用）
    pub fn deviation_of(&self, text: &str) -> f32 {
        let vec = self.encode_text(text);
        vec.luoshu_deviation()
    }
}

impl Default for HybridLuoShuEncoder {
    fn default() -> Self {
        Self::new_statistical()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试：统计编码器始终可用
    #[test]
    fn test_hybrid_fallback_works() {
        let encoder = HybridLuoShuEncoder::new_statistical();
        assert!(!encoder.is_ml_mode());

        let vec = encoder.encode_text("PostgreSQL 数据库优化");
        assert_eq!(vec.values.len(), 9);
        let dev = vec.luoshu_deviation();
        assert!(dev < 1.0, "幻和偏离度 {} 过高", dev);
    }

    /// 测试：投影矩阵初始化有效性
    #[test]
    fn test_projection_initialization() {
        let proj = LuoShuMlEncoder::init_projection(384);

        for j in 0..9 {
            let col_sum: f32 = (0..384).map(|i| proj[i][j].abs()).sum();
            assert!(col_sum > 0.0, "第 {} 列投影权重全为零", j);
        }
    }

    /// 测试：混合编码器在统计模式下也能工作
    #[test]
    fn test_hybrid_statistical_mode() {
        let encoder = HybridLuoShuEncoder::new_statistical();
        let v1 = encoder.encode_text("数据库");
        let v2 = encoder.encode_text("数据库配置");
        assert_eq!(v1.values.len(), 9);
        assert_eq!(v2.values.len(), 9);
    }
}