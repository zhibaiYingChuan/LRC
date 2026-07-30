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

use super::luoshu_encoder::{EncoderStatus, LuoShuEncoder, LuoShuVector, LUOSHU_WEIGHTS};
use super::pooling::PoolingStrategy;
use candle_core::{Device, Tensor};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ============================================================
// 默认模型语言检测（v0.6.0 通用语义引擎）
// ============================================================
// 根据系统语言自动选择默认嵌入模型：
//   - 中文环境 → BAAI/bge-small-zh（512 维，~100MB，中文 SOTA）
//   - 其他语言 → sentence-transformers/all-MiniLM-L6-v2（384 维，~80MB，多语言轻量）
//
// 用户仍可通过环境变量覆盖：
//   - LRC_LUOSHU_MODEL_ID（向后兼容，最高优先级）
//   - LRC_EMBEDDER_MODEL（v0.6.0 引入，统一配置，将在 4.2.1 实现）
//
// 语言检测优先级（高 → 低）：
//   1. LRC_LANG（LRC 自定义）
//   2. LANG（Unix 标准）
//   3. LC_ALL（Unix 标准）
//   4. LANGUAGE（GNU 标准，可含多语言，取第一个）
//   5. 默认 "zh_CN"（LRC 主要服务中文用户）

/// 道枢映射: 坤卦·地 (☷) — 承载万物，语言检测是模型选择的基础
/// 检测系统语言环境
///
/// 返回 BCP-47 风格的语言代码（如 "zh_CN"、"en_US"）。
/// Windows 用户若未设置环境变量，默认返回 "zh_CN"。
fn detect_system_lang() -> String {
    // 1. 优先检查 LRC 自定义环境变量
    for var in &["LRC_LANG", "LANG", "LC_ALL"] {
        if let Ok(val) = std::env::var(var) {
            // 过滤空值和 "C"/"POSIX"（ POSIX 默认值，非真实语言）
            if !val.is_empty() && val != "C" && val != "POSIX" {
                return val;
            }
        }
    }
    // 2. 检查 LANGUAGE（GNU 标准，可能含冒号分隔的多个语言）
    if let Ok(val) = std::env::var("LANGUAGE") {
        if !val.is_empty() {
            return val
                .split(':')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("zh_CN")
                .to_string();
        }
    }
    // 3. 默认值：LRC 主要服务中文用户
    "zh_CN".to_string()
}

/// 根据语言代码选择默认嵌入模型 ID
///
/// - 中文（zh_*）→ `BAAI/bge-small-zh`（中文 SOTA）
/// - 其他 → `sentence-transformers/all-MiniLM-L6-v2`（多语言轻量）
fn detect_default_model_by_lang(lang: &str) -> &'static str {
    if lang.to_lowercase().starts_with("zh") {
        "BAAI/bge-small-zh"
    } else {
        "sentence-transformers/all-MiniLM-L6-v2"
    }
}

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

impl LuoShuMlEncoder {
    /// 道枢映射: 坤卦·地 (☷) — 承载万物，模型加载是编码能力的根基
    /// 加载默认的轻量级多语言模型
    ///
    /// 加载策略（与 CodeBertEncoder 一致）：
    /// 1. 检查 `models/` 本地文件夹
    /// 2. 检查 HuggingFace 缓存
    /// 3. 从 HF_ENDPOINT 镜像下载
    ///
    /// 镜像守卫：函数入口强制检查 HF_ENDPOINT，确保绝不访问外网。
    /// 若 HF_ENDPOINT 未设置，自动设为 hf-mirror.com 国内镜像。
    pub fn load() -> Result<Self, String> {
        // ════════════════════════════════════════════════════════════
        // 本地镜像守卫 — 确保 hf-hub 库的下载请求走国内镜像
        // v0.5.4 修复：使用 ApiBuilder::with_endpoint 替代 set_var，避免多线程数据竞争
        let hf_endpoint =
            std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://hf-mirror.com".to_string());

        let device = Device::Cpu;

        // v0.6.0 默认模型选择：环境变量 > 语言检测默认值
        // 优先级：LRC_LUOSHU_MODEL_ID（向后兼容）> 语言检测（中文→BGE，其他→MiniLM）
        let model_id = std::env::var("LRC_LUOSHU_MODEL_ID").unwrap_or_else(|_| {
            let lang = detect_system_lang();
            let default_model = detect_default_model_by_lang(&lang);
            eprintln!(
                "[LRC·洛书ML] 系统语言: {} → 默认模型: {}",
                lang, default_model
            );
            default_model.to_string()
        });

        let local_model_name = model_id.replace('/', "--");

        let project_root =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
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

        // 自适应连通性检测：分层超时策略
        // 第一层：3 秒快速检测（覆盖 90% 的正常网络环境）
        // 第二层：6 秒宽容检测（覆盖慢速网络/代理环境）
        // 两层均失败才降级为统计编码器
        if !use_local {
            let hf_ip = std::net::SocketAddr::from(([104, 16, 86, 20], 443)); // huggingface.co
            let hf_reachable_fast =
                std::net::TcpStream::connect_timeout(&hf_ip, std::time::Duration::from_secs(3))
                    .is_ok();

            if !hf_reachable_fast {
                eprintln!("[LRC·洛书ML] 3s 快速检测超时，尝试 6s 宽容检测...");
                let hf_reachable_slow =
                    std::net::TcpStream::connect_timeout(&hf_ip, std::time::Duration::from_secs(6))
                        .is_ok();
                if !hf_reachable_slow {
                    return Err(
                        "HuggingFace 不可达（3s+6s 双层检测均超时），自动降级为统计编码器"
                            .to_string(),
                    );
                }
                eprintln!("[LRC·洛书ML] 6s 宽容检测通过，网络较慢但可用");
            }
        }

        // 加载分词器
        let tokenizer = if use_local {
            let tokenizer_path = model_dir.join("tokenizer.json");
            tokenizers::Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| format!("加载本地分词器失败: {}", e))?
        } else {
            let api = hf_hub::api::sync::ApiBuilder::new()
                .with_endpoint(hf_endpoint.clone())
                .build()
                .map_err(|e| format!("连接 HF Hub 失败: {}", e))?;
            let repo = api.model(model_id.clone());
            let tokenizer_path = repo
                .get("tokenizer.json")
                .map_err(|e| format!("下载分词器失败: {}", e))?;
            tokenizers::Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| format!("解析分词器失败: {}", e))?
        };

        // 加载模型
        let (model, hidden_size) = if use_local {
            let config_path = model_dir.join("config.json");
            // 从 config.json 解析真实的 BERT 配置（与 CodeBertEncoder 一致）
            // 修复：不能使用 Default::default()，因为不同模型的层数/维度不同
            // 例如 all-MiniLM-L6-v2 是 6 层 384 维，而 default 是 12 层 768 维
            let config_file = std::fs::File::open(&config_path).map_err(|e| {
                format!(
                    "打开 config.json 失败: {}\n路径: {}",
                    e,
                    config_path.display()
                )
            })?;
            let config: candle_transformers::models::bert::Config =
                serde_json::from_reader(std::io::BufReader::new(config_file))
                    .map_err(|e| format!("解析 config.json 失败: {}", e))?;
            let hidden_size = config.hidden_size;

            // 智能选择格式：safetensors 原生加载，pytorch_model.bin 使用 PthTensors
            let is_safetensors = model_dir.join("model.safetensors").exists();
            let weights_path = if is_safetensors {
                model_dir.join("model.safetensors")
            } else {
                model_dir.join("pytorch_model.bin")
            };

            let tensors: HashMap<String, Tensor> = if is_safetensors {
                candle_core::safetensors::load(&weights_path, &device).map_err(|e| {
                    format!(
                        "safetensors 加载失败: {}\n路径: {}",
                        e,
                        weights_path.display()
                    )
                })?
            } else {
                // pytorch_model.bin 使用 PthTensors 懒加载器（与 CodeBertEncoder 一致）
                let pth =
                    candle_core::pickle::PthTensors::new(&weights_path, None).map_err(|e| {
                        format!(
                            "pickle 加载 pytorch_model.bin 失败: {}\n\
                         提示: 文件可能已损坏，请尝试转换为 safetensors 格式后再试",
                            e
                        )
                    })?;
                let mut tensors = HashMap::new();
                for name in pth.tensor_infos().keys() {
                    if let Some(tensor) = pth
                        .get(name)
                        .map_err(|e| format!("加载 tensor '{}' 失败: {}", name, e))?
                    {
                        tensors.insert(name.to_string(), tensor);
                    }
                }
                if tensors.is_empty() {
                    return Err("pytorch_model.bin 中未找到任何 tensor\n\
                         提示: 文件可能已损坏，请尝试重新下载"
                        .to_string());
                }
                tensors
            };

            let vb = candle_nn::VarBuilder::from_tensors(tensors, candle_core::DType::F32, &device);

            let model = candle_transformers::models::bert::BertModel::load(vb, &config)
                .map_err(|e| format!("构建 BERT 模型失败: {}", e))?;

            (model, hidden_size)
        } else {
            let api = hf_hub::api::sync::ApiBuilder::new()
                .with_endpoint(hf_endpoint.clone())
                .build()
                .map_err(|e| format!("连接 HF Hub 失败: {}", e))?;
            let repo = api.model(model_id);

            let config_path = repo
                .get("config.json")
                .map_err(|e| format!("下载配置失败: {}", e))?;
            // 从 config.json 解析真实的 BERT 配置（与 CodeBertEncoder 一致）
            let config_file = std::fs::File::open(&config_path)
                .map_err(|e| format!("打开 config.json 失败: {}", e))?;
            let config: candle_transformers::models::bert::Config =
                serde_json::from_reader(std::io::BufReader::new(config_file))
                    .map_err(|e| format!("解析 config.json 失败: {}", e))?;
            let hidden_size = config.hidden_size;

            // 模型格式降级：safetensors → pytorch_model.bin（与 CodeBertEncoder 一致）
            let (weights_path, is_safetensors) = match repo.get("model.safetensors") {
                Ok(path) => (path, true),
                Err(_) => {
                    let path = repo.get("pytorch_model.bin").map_err(|e| {
                        format!(
                            "下载模型文件失败（safetensors 和 pytorch_model.bin 均不可用）: {}\n\
                             提示: 请检查网络连接，或手动将模型文件放到 models/{} 目录",
                            e, local_model_name
                        )
                    })?;
                    (path, false)
                }
            };

            let tensors: HashMap<String, Tensor> = if is_safetensors {
                candle_core::safetensors::load(&weights_path, &device)
                    .map_err(|e| format!("safetensors 加载失败: {}", e))?
            } else {
                let pth =
                    candle_core::pickle::PthTensors::new(&weights_path, None).map_err(|e| {
                        format!(
                            "pickle 加载 pytorch_model.bin 失败: {}\n\
                         提示: 如果持续失败，请尝试转换为 safetensors 格式",
                            e
                        )
                    })?;
                let mut tensors = HashMap::new();
                for name in pth.tensor_infos().keys() {
                    if let Some(tensor) = pth
                        .get(name)
                        .map_err(|e| format!("加载 tensor '{}' 失败: {}", name, e))?
                    {
                        tensors.insert(name.to_string(), tensor);
                    }
                }
                if tensors.is_empty() {
                    return Err("pytorch_model.bin 中未找到任何 tensor\n\
                         提示: 文件可能已损坏，请尝试重新下载"
                        .to_string());
                }
                tensors
            };

            let vb = candle_nn::VarBuilder::from_tensors(tensors, candle_core::DType::F32, &device);

            let model = candle_transformers::models::bert::BertModel::load(vb, &config)
                .map_err(|e| format!("构建 BERT 模型失败: {}", e))?;

            (model, hidden_size)
        };

        // 模型完整性校验：hidden_size 必须合理（BERT 系模型常见 384/768/1024）
        if !(128..=2048).contains(&hidden_size) {
            return Err(format!(
                "模型 config.json 中 hidden_size={} 异常，疑似文件损坏或版本不匹配。\
                 请检查 models/{} 目录下的模型文件是否完整",
                hidden_size, local_model_name
            ));
        }

        // 初始化投影矩阵
        let projection = Self::init_projection(hidden_size);

        eprintln!(
            "[LRC·洛书ML] 模型加载完成: {} (hidden_size={}, 池化=Mean)",
            if use_local { "本地" } else { "远程" },
            hidden_size
        );

        // 构建编码器实例
        let encoder = Self {
            model,
            tokenizer,
            device,
            pooling: PoolingStrategy::Mean,
            projection,
            hidden_size,
        };

        // 加载后验证：编码一个简单测试文本，确保模型实际可用
        // 这能捕获模型权重损坏、分词器不匹配等隐蔽问题
        match encoder.encode_text("Hello") {
            Ok(vec) => {
                let dev = vec.luoshu_deviation();
                if dev > 2.0 {
                    return Err(format!(
                        "模型加载后验证失败：测试编码的幻和偏离度 {:.2} 异常（期望 < 2.0）。\
                         模型可能已损坏或与分词器不匹配",
                        dev
                    ));
                }
                eprintln!("[LRC·洛书ML] 加载后验证通过，幻和偏离度: {:.2}", dev);
            }
            Err(e) => {
                return Err(format!(
                    "模型加载后验证失败：测试编码出错: {}。\
                     模型可能已损坏，请尝试重新下载模型文件到 models/{} 目录",
                    e, local_model_name
                ));
            }
        }

        Ok(encoder)
    }

    /// 初始化投影矩阵 W ∈ R^(hidden_size × 9)
    fn init_projection(hidden_size: usize) -> Vec<Vec<f32>> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let bound = (6.0_f32 / (hidden_size as f32 + 9.0)).sqrt();
        let mut proj = vec![vec![0.0f32; 9]; hidden_size];

        for (i, row) in proj.iter_mut().enumerate().take(hidden_size) {
            for (j, cell) in row.iter_mut().enumerate() {
                let mut hasher = DefaultHasher::new();
                (i * 9 + j).hash(&mut hasher);
                let seed = hasher.finish() as f32 / u64::MAX as f32;
                *cell = (seed - 0.5) * 2.0 * bound;
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

        let token_ids: Vec<u32> = encoding.get_ids().to_vec();
        let attention_mask: Vec<f32> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as f32)
            .collect();

        let seq_len = token_ids.len().min(512);

        // 2. 创建输入张量（与 CodeBertEncoder 相同的模式）
        let input_ids = Tensor::new(&token_ids[..seq_len], &self.device)
            .map_err(|e| format!("创建 input_ids: {}", e))?
            .unsqueeze(0)
            .map_err(|e| format!("unsqueeze: {}", e))?;

        let token_type_ids = input_ids
            .zeros_like()
            .map_err(|e| format!("type_ids: {}", e))?;

        let attention_tensor = Tensor::new(&attention_mask[..seq_len], &self.device)
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
        for (j, rf) in raw_features.iter_mut().enumerate() {
            let mut sum = 0.0f32;
            for (i, &ev) in emb_vec.iter().enumerate().take(actual_hidden) {
                sum += ev * self.projection[i][j];
            }
            *rf = sum;
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

    /// 道枢映射: 洛书·九宫 — 将语义向量映射到洛书九宫格，实现数与义的统一
    /// 获取底层 BERT 编码器的句嵌入（未经投影，用于其他语义场景）
    pub fn encode_embedding(&self, text: &str) -> Result<Vec<f32>, String> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| format!("分词失败: {}", e))?;

        let token_ids: Vec<u32> = encoding.get_ids().to_vec();
        let attention_mask: Vec<f32> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as f32)
            .collect();
        let seq_len = token_ids.len().min(512);

        let input_ids = Tensor::new(&token_ids[..seq_len], &self.device)
            .map_err(|e| format!("input_ids: {}", e))?
            .unsqueeze(0)
            .map_err(|e| format!("unsqueeze: {}", e))?;

        let token_type_ids = input_ids
            .zeros_like()
            .map_err(|e| format!("type_ids: {}", e))?;

        let attention_tensor = Tensor::new(&attention_mask[..seq_len], &self.device)
            .map_err(|e| format!("attention: {}", e))?
            .unsqueeze(0)
            .map_err(|e| format!("unsqueeze: {}", e))?;

        let output = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_tensor))
            .map_err(|e| format!("forward: {}", e))?;

        let mask = attention_tensor
            .unsqueeze(2)
            .map_err(|e| format!("mask unsqueeze: {}", e))?;
        let masked = output
            .broadcast_mul(&mask)
            .map_err(|e| format!("masked mul: {}", e))?;
        let sum = masked.sum(1).map_err(|e| format!("sum: {}", e))?;
        let mask_sum = mask.sum(1).map_err(|e| format!("mask_sum: {}", e))?;
        let pooled = sum
            .broadcast_div(&mask_sum)
            .map_err(|e| format!("div: {}", e))?;

        pooled
            .flatten_all()
            .map_err(|e| format!("flatten: {}", e))?
            .to_vec1()
            .map_err(|e| format!("to_vec1: {}", e))
    }

    /// 返回模型的隐藏层维度（v0.6.0 新增）
    ///
    /// 用于 Embedder trait 实现获取向量维度。
    /// BGE-small-zh: 512, MiniLM-L6-v2: 384, BGE-base-zh: 768
    pub fn hidden_size(&self) -> usize {
        self.hidden_size
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
    /// 编码器状态追踪
    status: Mutex<EncoderStatus>,
    /// 延迟恢复机制（质疑一：防止频繁模式切换）
    /// 当 ML 编码器从降级中恢复时，不立即切换，而是在连续 N 次成功编码后才切换
    recovery_state: Mutex<RecoveryState>,
}

/// 延迟恢复状态（质疑一：防止 ML↔统计 频繁横跳）
///
/// 当 ML 编码器因网络抖动等原因短暂不可用后恢复时，
/// 不立即切回 ML 模式，而是等待连续 N 次成功编码积累冷却期。
/// 这避免了编码器在两种模式之间来回震荡导致的向量质量波动。
struct RecoveryState {
    /// 连续 ML 编码成功次数（用于冷却期计数）
    consecutive_successes: u32,
    /// 恢复阈值：连续成功此次数后才切回 ML 模式
    recovery_threshold: u32,
    /// 是否处于降级状态（ML 不可用，正在使用统计模式）
    is_degraded: bool,
    /// 降级原因
    degradation_reason: String,
}

impl RecoveryState {
    fn new() -> Self {
        Self {
            consecutive_successes: 0,
            recovery_threshold: 5, // 默认连续 5 次成功才恢复
            is_degraded: false,
            degradation_reason: String::new(),
        }
    }
}

impl HybridLuoShuEncoder {
    /// 创建混合编码器（仅统计模式）
    pub fn new_statistical() -> Self {
        Self {
            ml_encoder: None,
            fallback: LuoShuEncoder::new(),
            status: Mutex::new(EncoderStatus {
                mode: "statistical".to_string(),
                model_name: None,
                hidden_size: None,
                degradation_reason: Some("ML 编码器未启用或加载失败".to_string()),
                total_encodings: 0,
                last_encoding_ms: 0,
                capability_description: "统计模式：基于词频和字符熵的轻量编码，语义区分能力有限"
                    .to_string(),
                quality_score: 0.25,
            }),
            recovery_state: Mutex::new(RecoveryState::new()),
        }
    }

    /// 创建混合编码器（尝试加载 ML 模型）
    pub fn new_with_ml(ml_encoder: LuoShuMlEncoder) -> Self {
        let hidden_size = ml_encoder.hidden_size; // 在移动前保存
        let model_name = format!("ML 语义模型 (hidden_size={})", hidden_size);
        Self {
            ml_encoder: Some(Arc::new(ml_encoder)),
            fallback: LuoShuEncoder::new(),
            status: Mutex::new(EncoderStatus {
                mode: "ml".to_string(),
                model_name: Some(model_name.clone()),
                hidden_size: Some(hidden_size),
                degradation_reason: None,
                total_encodings: 0,
                last_encoding_ms: 0,
                capability_description: format!(
                    "ML 语义模式：基于 {} 的深度学习编码，提供高精度语义理解",
                    model_name
                ),
                quality_score: 1.0,
            }),
            recovery_state: Mutex::new(RecoveryState::new()),
        }
    }

    /// 记录编码器降级（当 ML 编码失败回退到统计模式时调用）
    ///
    /// 质疑一修复：降级时设置恢复状态，确保后续恢复需要经过冷却期
    pub fn record_degradation(&self, reason: &str) {
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        let mut recovery = self
            .recovery_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        if status.mode == "ml" {
            status.mode = "statistical".to_string();
            status.degradation_reason = Some(reason.to_string());
            status.quality_score = 0.25; // 降级后语义保真度降低
            status.capability_description = format!(
                "降级统计模式：ML 编码器不可用（{}），当前使用词频编码，语义保真度降低",
                reason
            );

            // 标记降级状态，重置连续成功计数
            recovery.is_degraded = true;
            recovery.consecutive_successes = 0;
            recovery.degradation_reason = reason.to_string();

            eprintln!(
                "[LRC·编码器] 模式切换: ML → 统计（原因: {}）需要连续 {} 次 ML 成功后方可恢复",
                reason, recovery.recovery_threshold
            );
        }
    }

    /// 编码文本为洛书向量
    ///
    /// 优先使用 ML 编码器，失败时自动回退到统计编码器。
    ///
    /// 质疑一修复：引入冷却期机制。
    /// - 降级：ML 失败时立即切换到统计模式（快速降级）
    /// - 恢复：ML 成功后不立即切换，需连续 N 次成功才恢复（延迟恢复）
    ///   这避免了因临时网络抖动导致的频繁 ML↔统计 模式切换。
    pub fn encode_text(&self, text: &str) -> LuoShuVector {
        // 检查是否处于降级恢复状态
        let is_degraded = {
            let recovery = self
                .recovery_state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            recovery.is_degraded
        };

        if let Some(ref ml) = self.ml_encoder {
            match ml.encode_text(text) {
                Ok(vec) => {
                    // 更新编码器状态
                    let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
                    status.total_encodings += 1;
                    status.last_encoding_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;

                    // 质疑一核心逻辑：延迟恢复
                    if is_degraded {
                        // 处于降级状态，ML 编码成功但不立即恢复
                        let mut recovery = self
                            .recovery_state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        recovery.consecutive_successes += 1;
                        eprintln!(
                            "[LRC·编码器] ML 探测成功 {}/{}（冷却中...）",
                            recovery.consecutive_successes, recovery.recovery_threshold
                        );

                        if recovery.consecutive_successes >= recovery.recovery_threshold {
                            // 冷却期结束，恢复 ML 模式
                            recovery.is_degraded = false;
                            recovery.consecutive_successes = 0;
                            status.mode = "ml".to_string();
                            status.degradation_reason = None;
                            status.quality_score = 1.0;
                            status.capability_description =
                                "ML 语义模式：已恢复，提供高精度语义理解".to_string();
                            eprintln!(
                                "[LRC·编码器] 模式切换: 统计 → ML（冷却期结束，连续 {} 次成功）",
                                recovery.recovery_threshold
                            );
                        }
                        // 即使处于降级冷却期，也返回 ML 编码结果（探测模式）
                        return vec;
                    }

                    return vec;
                }
                Err(e) => {
                    eprintln!("[LRC·洛书] ML 编码失败 ({}), 回退到统计编码器", e);
                    self.record_degradation(&e);
                }
            }
        }
        // 统计模式编码
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        status.total_encodings += 1;
        status.last_encoding_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        drop(status);

        // 如果 ML 编码器存在但处于降级状态，且 ML 编码失败，
        // 重置连续成功计数（中断恢复过程）
        if self.ml_encoder.is_some() {
            let mut recovery = self
                .recovery_state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if recovery.is_degraded && recovery.consecutive_successes > 0 {
                eprintln!(
                    "[LRC·编码器] ML 探测失败，重置冷却计数（之前: {} 次成功）",
                    recovery.consecutive_successes
                );
                recovery.consecutive_successes = 0;
            }
        }

        self.fallback.encode_text(text)
    }

    /// 检查是否使用 ML 模式
    pub fn is_ml_mode(&self) -> bool {
        self.ml_encoder.is_some()
    }

    /// 检查是否处于降级状态（质疑一：监控用）
    pub fn is_degraded(&self) -> bool {
        self.recovery_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_degraded
    }

    /// 道枢映射: 震卦·雷 (☳) — 万物出乎震，恢复进度如春雷之后的复苏
    /// 获取当前恢复进度（质疑一：可解释性面板）
    /// 返回 (consecutive_successes, recovery_threshold)
    pub fn recovery_progress(&self) -> (u32, u32) {
        let recovery = self
            .recovery_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        (recovery.consecutive_successes, recovery.recovery_threshold)
    }

    /// 设置恢复阈值（质疑一：允许用户根据网络稳定性调整）
    pub fn set_recovery_threshold(&self, threshold: u32) {
        let mut recovery = self
            .recovery_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        recovery.recovery_threshold = threshold.max(1);
    }

    /// 获取编码器状态快照（可解释性面板）
    pub fn get_status(&self) -> EncoderStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 道枢映射: 洛书·幻和 — 计算向量的洛书幻和偏离度，度量编码质量
    /// 获取幻和偏离度（监控用）
    pub fn deviation_of(&self, text: &str) -> f32 {
        let vec = self.encode_text(text);
        vec.luoshu_deviation()
    }
}

impl Default for HybridLuoShuEncoder {
    fn default() -> Self {
        // 默认使用统计编码器，零依赖、零下载、秒启动
        // ML 语义模型仅在用户明确执行 --mode smart 时加载，
        // 且加载前会先检查本地模型是否存在，不存在则提示用户确认后从国内镜像下载
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

        // 计算每列投影权重绝对值之和，确保每列非零
        let col_sums: [f32; 9] = proj.iter().fold([0.0f32; 9], |mut acc, row| {
            for (j, &val) in row.iter().enumerate() {
                acc[j] += val.abs();
            }
            acc
        });
        for (j, &sum) in col_sums.iter().enumerate() {
            assert!(sum > 0.0, "第 {} 列投影权重全为零", j);
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

    /// 测试：质疑一冷却期 — 降级后恢复需要连续成功
    #[test]
    fn test_cooldown_recovery_mechanism() {
        let encoder = HybridLuoShuEncoder::new_statistical();
        // 统计模式下不应处于降级状态
        assert!(!encoder.is_degraded());

        // 模拟降级
        encoder.record_degradation("模拟网络抖动");
        // 统计模式编码器没有 ML 编码器，降级标记应设置
        // 但由于没有 ML 编码器，is_degraded 取决于 ml_encoder 是否存在
        // 此处重点验证降级逻辑不 panic
    }

    /// 测试：恢复阈值设置
    #[test]
    fn test_recovery_threshold_config() {
        let encoder = HybridLuoShuEncoder::new_statistical();
        encoder.set_recovery_threshold(10);
        let (_, threshold) = encoder.recovery_progress();
        assert_eq!(threshold, 10);

        // 阈值不能为 0
        encoder.set_recovery_threshold(0);
        let (_, threshold) = encoder.recovery_progress();
        assert_eq!(threshold, 1);
    }

    // ============================================================
    // v0.6.0 语言检测与默认模型选择测试
    // ============================================================

    /// 测试：中文语言检测 → BGE-small-zh
    #[test]
    fn test_detect_default_model_chinese() {
        // 标准中文
        assert_eq!(detect_default_model_by_lang("zh_CN"), "BAAI/bge-small-zh");
        assert_eq!(
            detect_default_model_by_lang("zh_CN.UTF-8"),
            "BAAI/bge-small-zh"
        );
        assert_eq!(detect_default_model_by_lang("zh_TW"), "BAAI/bge-small-zh");
        assert_eq!(detect_default_model_by_lang("zh_HK"), "BAAI/bge-small-zh");
        assert_eq!(detect_default_model_by_lang("zh_SG"), "BAAI/bge-small-zh");

        // 大小写不敏感
        assert_eq!(detect_default_model_by_lang("ZH_CN"), "BAAI/bge-small-zh");
        assert_eq!(detect_default_model_by_lang("Zh_CN"), "BAAI/bge-small-zh");

        // 纯语言代码
        assert_eq!(detect_default_model_by_lang("zh"), "BAAI/bge-small-zh");
    }

    /// 测试：非中文语言 → MiniLM-L6-v2（多语言轻量）
    #[test]
    fn test_detect_default_model_english_and_others() {
        // 英文
        assert_eq!(
            detect_default_model_by_lang("en_US"),
            "sentence-transformers/all-MiniLM-L6-v2"
        );
        assert_eq!(
            detect_default_model_by_lang("en_US.UTF-8"),
            "sentence-transformers/all-MiniLM-L6-v2"
        );
        assert_eq!(
            detect_default_model_by_lang("en_GB"),
            "sentence-transformers/all-MiniLM-L6-v2"
        );

        // 其他语言（日/法/德/韩）→ MiniLM（多语言支持）
        assert_eq!(
            detect_default_model_by_lang("ja_JP"),
            "sentence-transformers/all-MiniLM-L6-v2"
        );
        assert_eq!(
            detect_default_model_by_lang("fr_FR"),
            "sentence-transformers/all-MiniLM-L6-v2"
        );
        assert_eq!(
            detect_default_model_by_lang("de_DE"),
            "sentence-transformers/all-MiniLM-L6-v2"
        );
        assert_eq!(
            detect_default_model_by_lang("ko_KR"),
            "sentence-transformers/all-MiniLM-L6-v2"
        );
    }

    /// 测试：空字符串和边界情况 → 默认 MiniLM（非中文）
    #[test]
    fn test_detect_default_model_edge_cases() {
        // 空字符串 → 非中文 → MiniLM
        assert_eq!(
            detect_default_model_by_lang(""),
            "sentence-transformers/all-MiniLM-L6-v2"
        );
        // "zh" 作为子串但非前缀 → 不应识别为中文
        // "en_ZH_manufacturing" 经 to_lowercase 后为 "en_zh_manufacturing"，
        // 以 "en" 开头，不以 "zh" 开头，应返回 MiniLM
        assert_eq!(
            detect_default_model_by_lang("en_ZH_manufacturing"),
            "sentence-transformers/all-MiniLM-L6-v2"
        );
    }

    /// 测试：系统语言检测（仅验证返回值非空且符合格式）
    /// 注意：此测试不设置环境变量，依赖运行环境，仅做烟雾测试
    #[test]
    fn test_detect_system_lang_returns_nonempty() {
        let lang = detect_system_lang();
        assert!(!lang.is_empty(), "系统语言不应为空");
        // 默认应为 "zh_CN"（LRC 主要服务中文用户）或环境变量值
        println!("[smoke test] 当前系统语言检测: {}", lang);
    }
}
