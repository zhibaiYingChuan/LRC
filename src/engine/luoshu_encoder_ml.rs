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
use super::mirror_trapezoid::{BAGUA_PALACE_POS, LUOSHU_CENTER_POS};
use super::model_resolver::{EncodeRole, ModelFamilyProfile};
use super::pooling::PoolingStrategy;
use crate::errors::{ErrorKind, LrcError, LrcResult};
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
///
/// v0.9.7 核实：移除实验证明该 allow 非冗余（仍报 `never used`）。
#[allow(dead_code)]
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
/// v0.9.7 修复（模型 ID 常量重复）：改为委托 Layer 1 中立模块
/// [`crate::model_ids::detect_default_model_by_lang`]，消除本文件与
/// `model_resolver.rs` 的同名重复实现（两处此前各自硬编码同一对模型 ID）。
///
/// v0.9.7 核实：移除实验证明该 allow 非冗余（库目标下仍报 `never used`；
/// 当前仅有测试模块引用此包装函数）。
#[allow(dead_code)]
fn detect_default_model_by_lang(lang: &str) -> &'static str {
    crate::model_ids::detect_default_model_by_lang(lang)
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
    ///
    /// v0.9.10：不再硬编码，改由**模型家族的输入约定**决定
    /// （见 `model_resolver::profile_for_model`：e5 → Mean、bge → CLS）。
    pooling: PoolingStrategy,
    /// 模型家族的输入约定（前缀 / 池化）
    ///
    /// v0.9.10 新增：把"与模型强绑定"的输入格式约定集中到一处，
    /// 避免散落硬编码（此前是硬编码 `PoolingStrategy::Mean` + 全仓库无前缀）。
    profile: ModelFamilyProfile,
    /// 结构投影的 8 个「卦义原型」单位方向（8 × hidden）
    ///
    /// v0.9.10 取代原先的伪随机投影矩阵 `W ∈ R^(hidden×9)`。
    ///
    /// 原实现的问题（与产品的价值判据直接冲突，见 `memory_store.rs` §3.43.9）：
    /// 伪随机线性投影得到的 9 维只是 768 维语义向量的线性组合，即"BGE 的
    /// **粗粒度版本**"——而产品的价值判据是「关联图中**有多少条是 BGE 给不出
    /// 的**」，粗粒度版本注定给不出增量（该陷阱在 §3.47 已被明确否证）。
    ///
    /// 现改为「结构投影」：`value[宫] = cos(emb − μ, 原型方向[宫])`，
    /// 原型取自 `BAGUA_CATEGORIES`（与道体 GUA_LEXICON 同属一套语义坐标系）。
    /// 这是一次**有损的结构化量化**——两条主题不同但"性质"相同的记忆会因
    /// 落在同一宫位而彼此关联，正是 BGE 给不出的那类关联。
    archetype_dirs: Vec<Vec<f32>>,
    /// 结构投影的公共方向 μ（= 8 个原型嵌入的均值）
    ///
    /// v0.9.10 新增，同时解决两件事：
    ///   1. 去掉嵌入里的共模分量（实测其占嵌入能量约 73%，会淹没差异）；
    ///   2. 为什么用"8 个原型嵌入的均值"而不是样本均值——**μ 必须固定**。
    ///      若 μ 随使用漂移，第 1 天写入的向量与第 30 天写入的将不可比，
    ///      那比不修更糟。原型均值是模型自带、零新数据、零硬编码且时间恒定的。
    archetype_mu: Vec<f32>,
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
    pub fn load() -> LrcResult<Self> {
        // ════════════════════════════════════════════════════════════
        // 本地镜像守卫 — 确保 hf-hub 库的下载请求走国内镜像
        // v0.5.4 修复：使用 ApiBuilder::with_endpoint 替代 set_var，避免多线程数据竞争
        let hf_endpoint =
            std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://hf-mirror.com".to_string());

        let device = Device::Cpu;

        // v0.6.0 默认模型选择：环境变量 > 语言检测默认值
        // 优先级：LRC_LUOSHU_MODEL_ID（向后兼容）> 语言检测（中文→BGE，其他→MiniLM）
        let model_id = crate::engine::model_resolver::selected_model_id();

        let local_model_name = model_id.replace('/', "--");

        // v0.9.0 修复：统一模型目录查找，支持多个候选路径
        // （cwd/models + exe_dir/models + ~/.loong-recall/models），
        // 避免 sidecar 运行时 cwd 与模型下载时 cwd 不一致导致找不到模型。
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("models").join(&local_model_name));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                candidates.push(exe_dir.join("models").join(&local_model_name));
            }
        }
        if let Some(home) = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(std::path::PathBuf::from)
        {
            candidates.push(
                home.join(".loong-recall")
                    .join("models")
                    .join(&local_model_name),
            );
        }

        let mut use_local = false;
        let mut model_dir = std::path::PathBuf::new();

        for dir in candidates {
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
                    return Err(LrcError::network(
                        "HuggingFace 不可达（3s+6s 双层检测均超时），自动降级为统计编码器",
                    ));
                }
                eprintln!("[LRC·洛书ML] 6s 宽容检测通过，网络较慢但可用");
            }
        }

        // 加载分词器
        let tokenizer = if use_local {
            let tokenizer_path = model_dir.join("tokenizer.json");
            tokenizers::Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| LrcError::parse(format!("加载本地分词器失败: {}", e)))?
        } else {
            let api = hf_hub::api::sync::ApiBuilder::new()
                .with_endpoint(hf_endpoint.clone())
                .build()
                .map_err(|e| LrcError::network(format!("连接 HF Hub 失败: {}", e)))?;
            let repo = api.model(model_id.clone());
            let tokenizer_path = repo
                .get("tokenizer.json")
                .map_err(|e| LrcError::network(format!("下载分词器失败: {}", e)))?;
            tokenizers::Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| LrcError::parse(format!("解析分词器失败: {}", e)))?
        };

        // 加载模型
        let (model, hidden_size) = if use_local {
            let config_path = model_dir.join("config.json");
            // 从 config.json 解析真实的 BERT 配置（与 CodeBertEncoder 一致）
            // 修复：不能使用 Default::default()，因为不同模型的层数/维度不同
            // 例如 all-MiniLM-L6-v2 是 6 层 384 维，而 default 是 12 层 768 维
            let config_file = std::fs::File::open(&config_path).map_err(|e| {
                LrcError::io(format!(
                    "打开 config.json 失败: {}\n路径: {}",
                    e,
                    config_path.display()
                ))
            })?;
            let config: candle_transformers::models::bert::Config =
                serde_json::from_reader(std::io::BufReader::new(config_file))
                    .map_err(|e| LrcError::parse(format!("解析 config.json 失败: {}", e)))?;
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
                    LrcError::io(format!(
                        "safetensors 加载失败: {}\n路径: {}",
                        e,
                        weights_path.display()
                    ))
                })?
            } else {
                // pytorch_model.bin 使用 PthTensors 懒加载器（与 CodeBertEncoder 一致）
                let pth =
                    candle_core::pickle::PthTensors::new(&weights_path, None).map_err(|e| {
                        LrcError::io(format!(
                            "pickle 加载 pytorch_model.bin 失败: {}\n\
                         提示: 文件可能已损坏，请尝试转换为 safetensors 格式后再试",
                            e
                        ))
                    })?;
                let mut tensors = HashMap::new();
                for name in pth.tensor_infos().keys() {
                    if let Some(tensor) = pth
                        .get(name)
                        .map_err(|e| LrcError::io(format!("加载 tensor '{}' 失败: {}", name, e)))?
                    {
                        tensors.insert(name.to_string(), tensor);
                    }
                }
                if tensors.is_empty() {
                    return Err(LrcError::not_found(
                        "pytorch_model.bin 中未找到任何 tensor\n\
                         提示: 文件可能已损坏，请尝试重新下载",
                    ));
                }
                tensors
            };

            let vb = candle_nn::VarBuilder::from_tensors(tensors, candle_core::DType::F32, &device);

            let model = candle_transformers::models::bert::BertModel::load(vb, &config)
                .map_err(|e| LrcError::internal(format!("构建 BERT 模型失败: {}", e)))?;

            (model, hidden_size)
        } else {
            let api = hf_hub::api::sync::ApiBuilder::new()
                .with_endpoint(hf_endpoint.clone())
                .build()
                .map_err(|e| LrcError::network(format!("连接 HF Hub 失败: {}", e)))?;
            let repo = api.model(model_id);

            let config_path = repo
                .get("config.json")
                .map_err(|e| LrcError::network(format!("下载配置失败: {}", e)))?;
            // 从 config.json 解析真实的 BERT 配置（与 CodeBertEncoder 一致）
            let config_file = std::fs::File::open(&config_path)
                .map_err(|e| LrcError::io(format!("打开 config.json 失败: {}", e)))?;
            let config: candle_transformers::models::bert::Config =
                serde_json::from_reader(std::io::BufReader::new(config_file))
                    .map_err(|e| LrcError::parse(format!("解析 config.json 失败: {}", e)))?;
            let hidden_size = config.hidden_size;

            // 模型格式降级：safetensors → pytorch_model.bin（与 CodeBertEncoder 一致）
            let (weights_path, is_safetensors) = match repo.get("model.safetensors") {
                Ok(path) => (path, true),
                Err(_) => {
                    let path = repo.get("pytorch_model.bin").map_err(|e| {
                        LrcError::network(format!(
                            "下载模型文件失败（safetensors 和 pytorch_model.bin 均不可用）: {}\n\
                             提示: 请检查网络连接，或手动将模型文件放到 models/{} 目录",
                            e, local_model_name
                        ))
                    })?;
                    (path, false)
                }
            };

            let tensors: HashMap<String, Tensor> = if is_safetensors {
                candle_core::safetensors::load(&weights_path, &device)
                    .map_err(|e| LrcError::io(format!("safetensors 加载失败: {}", e)))?
            } else {
                let pth =
                    candle_core::pickle::PthTensors::new(&weights_path, None).map_err(|e| {
                        LrcError::io(format!(
                            "pickle 加载 pytorch_model.bin 失败: {}\n\
                         提示: 如果持续失败，请尝试转换为 safetensors 格式",
                            e
                        ))
                    })?;
                let mut tensors = HashMap::new();
                for name in pth.tensor_infos().keys() {
                    if let Some(tensor) = pth
                        .get(name)
                        .map_err(|e| LrcError::io(format!("加载 tensor '{}' 失败: {}", name, e)))?
                    {
                        tensors.insert(name.to_string(), tensor);
                    }
                }
                if tensors.is_empty() {
                    return Err(LrcError::not_found(
                        "pytorch_model.bin 中未找到任何 tensor\n\
                         提示: 文件可能已损坏，请尝试重新下载",
                    ));
                }
                tensors
            };

            let vb = candle_nn::VarBuilder::from_tensors(tensors, candle_core::DType::F32, &device);

            let model = candle_transformers::models::bert::BertModel::load(vb, &config)
                .map_err(|e| LrcError::internal(format!("构建 BERT 模型失败: {}", e)))?;

            (model, hidden_size)
        };

        // 模型完整性校验：hidden_size 必须合理（BERT 系模型常见 384/768/1024）
        if !(128..=2048).contains(&hidden_size) {
            return Err(LrcError::invalid_input(format!(
                "模型 config.json 中 hidden_size={} 异常，疑似文件损坏或版本不匹配。\
                 请检查 models/{} 目录下的模型文件是否完整",
                hidden_size, local_model_name
            )));
        }

        // 初始化投影矩阵
        // v0.9.10：原先在此构造伪随机投影矩阵（`init_projection`），现已由
        // 「结构投影」取代 —— 见 `build_archetype_projection`。

        // v0.9.10：池化与输入前缀由**模型家族**决定，不再硬编码 Mean。
        // 修的是"换模型时约定没跟着换"这一上游成因：
        //   e5 系列需要 `query:` / `passage:` 前缀，官方池化 = Mean；
        //   bge 系列不需前缀，官方检索用法 = CLS。
        let profile =
            crate::engine::model_resolver::profile_for_model(&local_model_name.to_string());
        let pooling_name = match profile.pooling {
            PoolingStrategy::Cls => "CLS",
            PoolingStrategy::Mean => "Mean",
        };
        let prefix_name = if profile.prefix_passage.is_empty() {
            "无"
        } else {
            profile.prefix_passage.trim()
        };

        eprintln!(
            "[LRC·洛书ML] 模型加载完成: {} (hidden_size={}, 池化={}, 前缀={})",
            if use_local { "本地" } else { "远程" },
            hidden_size,
            pooling_name,
            prefix_name
        );

        // 构建编码器实例（结构投影方向在模型就绪后再构建）
        let mut encoder = Self {
            model,
            tokenizer,
            device,
            pooling: profile.pooling,
            profile,
            archetype_dirs: Vec::new(),
            archetype_mu: Vec::new(),
            hidden_size,
        };

        // v0.9.10 构建结构投影：8 个卦义原型方向 + 公共方向 μ
        encoder.build_archetype_projection()?;

        // 加载后验证（v0.9.11：与表示层温度旋钮解耦）
        //
        // 历史缺陷：原实现以 `encode_text("Hello")` 的「幻和偏离度 < 2.0」为门槛。
        // 但该偏离度随 `LRC_LUOSHU_CONTRAST_TEMP`（对比度温度）**单调漂移**——
        // 它实际在测"当前温度"，而非"模型是否健康"：为提升判别分辨率而调低温度，
        // 会让健康模型被误判为"损坏/分词器不匹配"，进而整体降级为统计编码器
        // （实测：temp=0.40 → 偏离度 2.033 ≥ 2.0 → 加载失败）。这是 v0.9.10 引入
        // 表示层旋钮时遗留的耦合。
        //
        // 新判据与温度无关，只检验"编码器是否真的在工作"：
        //   (1) 输出有限、非退化（非 NaN/Inf、非全零）；
        //   (2) 不同输入可区分（不会塌缩为同一向量）。
        // 权重损坏或分词器不匹配都会使输出退化或不可区分，故仍被捕获；
        // 同时不再随表示层参数漂移。
        let probe_texts: [&str; 2] = ["Hello", "数据库连接池的最大连接数配置为 20"];
        let mut probes: Vec<LuoShuVector> = Vec::with_capacity(probe_texts.len());
        for t in probe_texts {
            match encoder.encode_text(t) {
                Ok(vec) => probes.push(vec),
                Err(e) => {
                    return Err(LrcError::internal(format!(
                        "模型加载后验证失败：测试编码出错: {}。\
                         模型可能已损坏，请尝试重新下载模型文件到 models/{} 目录",
                        e, local_model_name
                    )));
                }
            }
        }

        // (1) 有限且非退化
        for (k, v) in probes.iter().enumerate() {
            if v.values.iter().any(|x| !x.is_finite()) {
                return Err(LrcError::internal(format!(
                    "模型加载后验证失败：测试编码[{}] 含非有限值（NaN/Inf）。\
                     模型可能已损坏或与分词器不匹配",
                    k
                )));
            }
            if !(v.values.iter().sum::<f32>() > 0.0) {
                return Err(LrcError::internal(
                    "模型加载后验证失败：测试编码退化（全零）。\
                     模型可能已损坏或与分词器不匹配"
                        .to_string(),
                ));
            }
        }

        // (2) 不同输入可区分。1e-3 仅为浮点噪声下限（非标定阈值），
        //     健康状态下两路语义不同的探测文本差异远大于此（含低温区间）。
        let max_abs_diff = probes[0]
            .values
            .iter()
            .zip(probes[1].values.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        if max_abs_diff <= 1e-3 {
            return Err(LrcError::internal(format!(
                "模型加载后验证失败：不同输入编码不可区分（最大分量差 {:.2e}）。\
                 模型可能已损坏或与分词器不匹配",
                max_abs_diff
            )));
        }

        eprintln!(
            "[LRC·洛书ML] 加载后验证通过（温度无关判据）：两路探测最大分量差 {:.4}",
            max_abs_diff
        );

        Ok(encoder)
    }

    /// 构建结构投影：8 个「卦义原型」单位方向 + 公共方向 μ
    ///
    /// ## 为什么是原型投影，而不是随机投影
    ///
    /// 产品对联想层的价值判据（`memory_store.rs` §3.43.9 逐字）：
    /// > 「道体的价值判据不是'能否产出关联图'，而是'**产出的关联图中，
    /// >   有多少条是 BGE 给不出的**'」
    ///
    /// 伪随机线性投影产出的 9 维是 768 维语义向量的线性组合，即"BGE 的粗粒度
    /// 版本"——这类投影注定给不出增量（该陷阱在 §3.47 已被实测否证）。
    ///
    /// 改为把嵌入投影到 8 个**卦义原型**方向后，得到的是"这条记忆在八卦这套
    /// 语义坐标系里的**性质画像**"：两条主题不同但性质相同的记忆会因落在同一
    /// 宫位而彼此关联 —— 这正是 BGE 给不出的那类关联。
    ///
    /// ## μ 为什么用「原型均值」
    ///
    /// μ 必须**固定**：若随使用漂移，第 1 天写入的向量与第 30 天写入的将不可比，
    /// 那比不修更糟。原型均值满足全部约束：模型自带、零新数据、零硬编码、
    /// 时间恒定，且能抵掉嵌入里约 73% 能量的共模分量。
    ///
    /// ## 原型文本
    ///
    /// 默认取 `BAGUA_CATEGORIES`（LRC 既有卦义表）；可用
    /// `LRC_BAGUA_ARCHETYPE_TEXTS`（JSON 字符串数组，必须 8 项）覆盖，
    /// 以便与道体 `GUA_LEXICON` 的词表对齐。
    fn build_archetype_projection(&mut self) -> LrcResult<()> {
        use crate::engine::mirror_trapezoid::BAGUA_CATEGORIES;

        let from_env = std::env::var("LRC_BAGUA_ARCHETYPE_TEXTS").ok();
        let texts: Vec<String> = from_env
            .as_ref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .filter(|v| v.len() == 8)
            .unwrap_or_else(|| BAGUA_CATEGORIES.iter().map(|s| s.to_string()).collect());

        let mut embs: Vec<Vec<f32>> = Vec::with_capacity(8);
        for t in &texts {
            embs.push(self.encode_embedding_role(t, EncodeRole::Passage)?);
        }

        let dim = embs[0].len().min(self.hidden_size);
        if dim == 0 {
            return Err(LrcError::internal("结构投影构建失败：原型嵌入维度为 0"));
        }

        // μ = 8 个原型嵌入的均值（确定性的公共方向）
        let mut mu = vec![0.0f32; dim];
        for e in &embs {
            for i in 0..dim {
                mu[i] += e[i];
            }
        }
        for m in mu.iter_mut() {
            *m /= embs.len() as f32;
        }

        // 原型方向 = 单位化(原型嵌入 − μ)
        let mut dirs: Vec<Vec<f32>> = Vec::with_capacity(embs.len());
        for e in &embs {
            let mut v: Vec<f32> = (0..dim).map(|i| e[i] - mu[i]).collect();
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 1e-6 {
                for x in v.iter_mut() {
                    *x /= norm;
                }
            }
            dirs.push(v);
        }

        eprintln!(
            "[LRC·结构投影] 已构建 {} 个卦义原型方向（dim={}），原型来源: {}",
            dirs.len(),
            dim,
            if from_env.is_some() {
                "环境变量覆盖"
            } else {
                "BAGUA_CATEGORIES"
            }
        );

        self.archetype_mu = mu;
        self.archetype_dirs = dirs;
        Ok(())
    }

    /// v0.9.2 对比度增强（纯函数，可单测）
    ///
    /// 对 9 维投影特征做中心化 + 温度调制的 softmax，放大维度间差异，
    /// 使不同输入的编码向量保持可区分性，防止全部塌缩到同一八卦类别。
    ///
    /// 温度越低 softmax 越尖锐，最高维度的权重越突出（增强区分度）；
    /// 当所有特征相等时退化为均匀分布（1/9），避免数值问题。
    fn contrast_normalize(features: &[f32; 9], temperature: f32) -> [f32; 9] {
        let mean = features.iter().sum::<f32>() / 9.0;
        let mut centered = [0.0f32; 9];
        for (i, v) in features.iter().enumerate() {
            centered[i] = v - mean;
        }

        let max_feat = centered.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let temp = temperature.max(1e-4); // 防除零
        let exp_sum: f32 = centered.iter().map(|v| ((v - max_feat) / temp).exp()).sum();

        if exp_sum <= 1e-12 || !exp_sum.is_finite() {
            return [1.0 / 9.0; 9]; // 数值退化时返回均匀分布
        }

        let mut out = [0.0f32; 9];
        for (i, v) in centered.iter().enumerate() {
            out[i] = ((v - max_feat) / temp).exp() / exp_sum;
        }
        out
    }

    /// 使用 ML 模型将文本编码为洛书 9 维向量（默认按「文档 / 记忆」角色）
    ///
    /// v0.9.10：改为委托 [`Self::encode_text_role`]，默认 `EncodeRole::Passage`
    /// （记忆写入路径占绝大多数）；检索侧请显式用 `EncodeRole::Query`。
    pub fn encode_text(&self, text: &str) -> LrcResult<LuoShuVector> {
        self.encode_text_role(text, EncodeRole::Passage)
    }

    /// 按指定角色把文本编码为洛书 9 维向量
    ///
    /// 角色只影响**输入前缀**：e5 系列要求查询用 `query: `、文档用 `passage: `；
    /// 其余家族前缀为空串，行为与改动前完全一致。
    pub fn encode_text_role(&self, text: &str, role: EncodeRole) -> LrcResult<LuoShuVector> {
        // 0. 按模型家族补输入前缀（e5 必需；其余家族为空前缀，无行为变化）
        let prefixed = self.profile.apply_prefix(text, role);

        // 1. Tokenize
        let encoding = self
            .tokenizer
            .encode(prefixed, true)
            .map_err(|e| LrcError::parse(format!("分词失败: {}", e)))?;

        let token_ids: Vec<u32> = encoding.get_ids().to_vec();
        let attention_mask: Vec<f32> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as f32)
            .collect();

        let seq_len = token_ids.len().min(512);

        // 2. 创建输入张量（与 CodeBertEncoder 相同的模式）
        let input_ids = Tensor::new(&token_ids[..seq_len], &self.device)
            .map_err(|e| LrcError::internal(format!("创建 input_ids: {}", e)))?
            .unsqueeze(0)
            .map_err(|e| LrcError::internal(format!("unsqueeze: {}", e)))?;

        let token_type_ids = input_ids
            .zeros_like()
            .map_err(|e| LrcError::internal(format!("type_ids: {}", e)))?;

        let attention_tensor = Tensor::new(&attention_mask[..seq_len], &self.device)
            .map_err(|e| LrcError::internal(format!("attention: {}", e)))?
            .unsqueeze(0)
            .map_err(|e| LrcError::internal(format!("unsqueeze: {}", e)))?;

        // 3. BERT 前向传播
        let output = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_tensor))
            .map_err(|e| LrcError::internal(format!("BERT 前向: {}", e)))?;

        // 4. 池化
        let embedding = match self.pooling {
            PoolingStrategy::Cls => {
                // [CLS] token 是第 0 个位置
                output
                    .get(0)
                    .map_err(|e| LrcError::internal(format!("batch: {}", e)))?
                    .get(0)
                    .map_err(|e| LrcError::internal(format!("cls: {}", e)))?
            }
            PoolingStrategy::Mean => {
                let mask = attention_tensor
                    .unsqueeze(2)
                    .map_err(|e| LrcError::internal(format!("mask unsqueeze: {}", e)))?;
                let masked = output
                    .broadcast_mul(&mask)
                    .map_err(|e| LrcError::internal(format!("masked mul: {}", e)))?;
                let sum = masked
                    .sum(1)
                    .map_err(|e| LrcError::internal(format!("sum: {}", e)))?;
                let mask_sum = mask
                    .sum(1)
                    .map_err(|e| LrcError::internal(format!("mask_sum: {}", e)))?;
                sum.broadcast_div(&mask_sum)
                    .map_err(|e| LrcError::internal(format!("div: {}", e)))?
            }
        };

        // 展平为 1D 向量
        let emb_vec: Vec<f32> = embedding
            .flatten_all()
            .map_err(|e| LrcError::internal(format!("flatten: {}", e)))?
            .to_vec1()
            .map_err(|e| LrcError::internal(format!("to_vec1: {}", e)))?;

        // 步骤 5~7 抽为 project_embedding（v0.9.10 批量编码复用，行为逐字不变）
        Ok(self.project_embedding(&emb_vec))
    }

    /// 把单条 BERT 句嵌入（hidden 维）投影为洛书 9 维向量
    ///
    /// v0.9.10 从 [`Self::encode_text_role`] 的步骤 5~7 原样抽出，供单条与
    /// 批量编码共用同一条投影链路。**保证批量与逐条结果逐位一致**——批量
    /// 编码只改变 BERT 前向的并行度，不改变这里的任何数学。
    fn project_embedding(&self, emb_vec: &[f32]) -> LuoShuVector {
        let actual_hidden = self.hidden_size.min(emb_vec.len());

        // 5. 结构投影（v0.9.10）：hidden → 8 宫义亲和度 + 1 中心
        //
        // value[宫] = cos(emb − μ, 原型方向[宫])：给出"这条记忆在八卦语义坐标系
        // 里的性质画像"。原型方向在模型加载时构建（见 build_archetype_projection）。
        let dim = actual_hidden.min(self.archetype_mu.len());
        let mut raw_features = [0.0f32; 9];
        {
            let mut outer = [0.0f32; 8];
            for (j, dir) in self.archetype_dirs.iter().enumerate().take(8) {
                let mut dot = 0.0f32;
                for i in 0..dim {
                    dot += (emb_vec[i] - self.archetype_mu[i]) * dir[i];
                }
                outer[j] = dot;
            }
            // 卦索引 → 洛书九宫位置（与 BAGUA_BASES 严格同序）
            for (j, &pos) in BAGUA_PALACE_POS.iter().enumerate() {
                raw_features[pos] = outer[j];
            }
            // 中心（位置 4）= 外圈 8 宫的均值。
            // 理据：理想洛书 (4+9+2+3+7+8+1+6)/8 = 5 = 中心数，即"中心 = 外圈均值"
            // 是洛书自带的平衡性。中心因此天然"始终不变"（它是外圈的确定函数），
            // 且永远不是最大值，不破坏依赖 argmax 的几何。
            let sum: f32 = outer.iter().sum();
            raw_features[LUOSHU_CENTER_POS] = sum / 8.0;
        }

        // 5.5 v0.9.2 对比度增强：中心化 + softmax 放大维度差异，防止编码塌缩
        // 根因：ML 投影在贝叶斯融合前被 LUOSHU_WEIGHTS 先验主导，不同输入的 9 维
        // 向量高度相似 → mirror_project 全部落入同一八卦类别 -> 信息增量守卫拦截合成。
        // 修复：先中心化消除公共偏置，再用温度调制的 softmax 放大输入相关的差异。
        //
        // v0.9.10：温度去硬编码，便于按模型家族标定；
        //   可用 `LRC_LUOSHU_CONTRAST_TEMP`（> 0）覆盖做对照实验。
        // v0.9.11：默认值由 0.7 重标定为 **0.4**（基于温度扫描 + 端到端基准复测）。
        //   依据：0.4 处于判别分辨率最优区（近义/无关分离度最高），且 30 实例
        //   基准下 ML 双维优于统计回退（Session 0.8000 vs 0.6333，Turn 0.5000
        //   vs 0.4333），而 0.7 时 Turn(0.4138) 反低于统计回退。此项为单变量标定，
        //   非为迁就评测而改语义能力。
        let contrast_temp: f32 = std::env::var("LRC_LUOSHU_CONTRAST_TEMP")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(0.4);
        let enhanced = Self::contrast_normalize(&raw_features, contrast_temp);

        // 6. 输入特征与洛书先验的加权融合
        //
        // 历史根因：原实现 posterior = LUOSHU_WEIGHTS * (1 + likelihood)，
        //   likelihood 微弱时后验 ≈ LUOSHU_WEIGHTS ⇒ mirror_project 全部映射到
        //   同一八卦类别（离·火）——因为 LUOSHU_WEIGHTS 的 argmax 恒在位置 1
        //   （9/45 最大）。v0.9.2 把先验权重降到 25%，但仍**保留了一个固定偏置**：
        //   只要 enhanced 接近均匀，25% 的先验就足以把 argmax 拉回位置 1。
        //
        // v0.9.10：默认权重降为 **0** —— 先验不再充当偏置源。
        //   与 normalize_to_luoshu 的修正同一原则：**洛书结构是骨架，
        //   语义在激活值里**；把"理想洛书形状"强行叠加到激活值上，等于把
        //   固定的 argmax 预埋进每一个向量。
        //   可用 `LRC_LUOSHU_PRIOR_WEIGHT`（0.0~1.0）覆盖，做对照实验。
        let prior_weight: f32 = std::env::var("LRC_LUOSHU_PRIOR_WEIGHT")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| (0.0..=1.0).contains(v))
            .unwrap_or(0.0);
        let mut posterior = [0.0f32; 9];
        for i in 0..9 {
            posterior[i] = (1.0 - prior_weight) * enhanced[i] + prior_weight * LUOSHU_WEIGHTS[i];
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
        vec
    }

    /// 批量把文本编码为洛书 9 维向量（v0.9.10 吞吐优化）
    ///
    /// 与 [`Self::encode_text_role`] 语义完全一致，仅把逐条的 BERT 前向合并为
    /// **单次批量前向**（candle `BertModel::forward` 原生支持 `[batch, seq]`），
    /// 以摊薄 CPU 前向的固定开销。输出与逐条调用逐位一致（同一池化 + 同一投影）。
    ///
    /// 约定：
    ///   - 空入参返回空 `Vec`（不触发任何前向）；
    ///   - 输出条数与输入条数严格相等，顺序一一对应；
    ///   - padding 到批内最长（上限 512），`[PAD]` 的 attention_mask=0，
    ///     CLS 池化取位置 0、Mean 池化按 mask 加权，均不受 padding 影响。
    pub fn encode_text_batch(
        &self,
        texts: &[&str],
        role: EncodeRole,
    ) -> LrcResult<Vec<LuoShuVector>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // 1. 逐条补前缀 + 分词（中文注释：与单条路径 apply_prefix 完全一致）
        let mut seqs: Vec<Vec<u32>> = Vec::with_capacity(texts.len());
        for text in texts {
            let prefixed = self.profile.apply_prefix(text, role);
            let encoding = self
                .tokenizer
                .encode(prefixed, true)
                .map_err(|e| LrcError::parse(format!("分词失败: {}", e)))?;
            let ids: Vec<u32> = encoding.get_ids().iter().take(512).copied().collect();
            seqs.push(ids);
        }

        // 2. 批内 padding：pad_id 取分词表的 [PAD]（缺省 0），max_len 上限 512
        let pad_id = self.tokenizer.token_to_id("[PAD]").unwrap_or(0);
        let max_len = seqs
            .iter()
            .map(|s| s.len())
            .max()
            .unwrap_or(0)
            .clamp(1, 512);
        let batch = seqs.len();

        // 构造扁平 [batch * max_len] 的 input_ids 与 attention_mask（0/1）
        let mut flat_ids: Vec<u32> = Vec::with_capacity(batch * max_len);
        let mut flat_mask: Vec<f32> = Vec::with_capacity(batch * max_len);
        for seq in &seqs {
            for i in 0..max_len {
                if i < seq.len() {
                    flat_ids.push(seq[i]);
                    flat_mask.push(1.0);
                } else {
                    flat_ids.push(pad_id);
                    flat_mask.push(0.0);
                }
            }
        }

        // 3. 建张量：[batch, max_len]
        let input_ids = Tensor::new(flat_ids.as_slice(), &self.device)
            .map_err(|e| LrcError::internal(format!("创建 input_ids: {}", e)))?
            .reshape((batch, max_len))
            .map_err(|e| LrcError::internal(format!("reshape input_ids: {}", e)))?;
        let attention_tensor = Tensor::new(flat_mask.as_slice(), &self.device)
            .map_err(|e| LrcError::internal(format!("创建 attention: {}", e)))?
            .reshape((batch, max_len))
            .map_err(|e| LrcError::internal(format!("reshape attention: {}", e)))?;
        let token_type_ids = input_ids
            .zeros_like()
            .map_err(|e| LrcError::internal(format!("type_ids: {}", e)))?;

        // 4. 单次 BERT 前向：[batch, max_len] → [batch, max_len, hidden]
        let output = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_tensor))
            .map_err(|e| LrcError::internal(format!("BERT 前向: {}", e)))?;

        // 5. 池化（与单条路径同策略，仅多保留 batch 维）
        let pooled = match self.pooling {
            PoolingStrategy::Cls => output
                // 取位置 0 的 [CLS]：narrow(1,0,1) → [batch,1,hidden] → squeeze(1)
                .narrow(1, 0, 1)
                .map_err(|e| LrcError::internal(format!("cls narrow: {}", e)))?
                .squeeze(1)
                .map_err(|e| LrcError::internal(format!("cls squeeze: {}", e)))?,
            PoolingStrategy::Mean => {
                let mask = attention_tensor
                    .unsqueeze(2)
                    .map_err(|e| LrcError::internal(format!("mask unsqueeze: {}", e)))?;
                let masked = output
                    .broadcast_mul(&mask)
                    .map_err(|e| LrcError::internal(format!("masked mul: {}", e)))?;
                let sum = masked
                    .sum(1)
                    .map_err(|e| LrcError::internal(format!("sum: {}", e)))?;
                let mask_sum = mask
                    .sum(1)
                    .map_err(|e| LrcError::internal(format!("mask_sum: {}", e)))?;
                sum.broadcast_div(&mask_sum)
                    .map_err(|e| LrcError::internal(format!("div: {}", e)))?
            }
        };

        // 6. [batch, hidden] → Vec<Vec<f32>>，逐行复用同一投影链路
        let rows: Vec<Vec<f32>> = pooled
            .to_vec2()
            .map_err(|e| LrcError::internal(format!("to_vec2: {}", e)))?;
        Ok(rows.iter().map(|row| self.project_embedding(row)).collect())
    }

    /// 获取底层 BERT 编码器的句嵌入（未经投影，用于其他语义场景）
    ///
    /// 默认按「文档 / 记忆」角色补前缀；检索侧请用
    /// [`Self::encode_embedding_role`] 传 `EncodeRole::Query`。
    pub fn encode_embedding(&self, text: &str) -> LrcResult<Vec<f32>> {
        self.encode_embedding_role(text, EncodeRole::Passage)
    }

    /// 按指定角色获取底层 BERT 编码器的句嵌入（未经投影）
    ///
    /// v0.9.10 修正此处的两处硬编码：
    ///   1. **池化改为随模型家族**（`self.pooling`）。此前硬编码 CLS，而注释
    ///      以 bge 论证——当实际模型换成 e5（官方池化 = Mean）时，约定与模型
    ///      不匹配。现在由 `model_resolver::profile_for_model` 决定。
    ///   2. **补输入前缀**。e5 系列要求 `query: ` / `passage: `；此前全仓库
    ///      无任何前缀注入。
    pub fn encode_embedding_role(&self, text: &str, role: EncodeRole) -> LrcResult<Vec<f32>> {
        let prefixed = self.profile.apply_prefix(text, role);
        let encoding = self
            .tokenizer
            .encode(prefixed, true)
            .map_err(|e| LrcError::parse(format!("分词失败: {}", e)))?;

        let token_ids: Vec<u32> = encoding.get_ids().to_vec();
        let attention_mask: Vec<f32> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as f32)
            .collect();
        let seq_len = token_ids.len().min(512);

        let input_ids = Tensor::new(&token_ids[..seq_len], &self.device)
            .map_err(|e| LrcError::internal(format!("input_ids: {}", e)))?
            .unsqueeze(0)
            .map_err(|e| LrcError::internal(format!("unsqueeze: {}", e)))?;

        let token_type_ids = input_ids
            .zeros_like()
            .map_err(|e| LrcError::internal(format!("type_ids: {}", e)))?;

        let attention_tensor = Tensor::new(&attention_mask[..seq_len], &self.device)
            .map_err(|e| LrcError::internal(format!("attention: {}", e)))?
            .unsqueeze(0)
            .map_err(|e| LrcError::internal(format!("unsqueeze: {}", e)))?;

        let output = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_tensor))
            .map_err(|e| LrcError::internal(format!("forward: {}", e)))?;

        // 池化：随模型家族选择（v0.9.10 起不再硬编码 CLS）
        let vec: Vec<f32> = match self.pooling {
            PoolingStrategy::Cls => {
                // 取序列首位 token 的隐层向量（[batch, seq, hidden] → [hidden]）
                let cls = output
                    .narrow(1, 0, 1)
                    .map_err(|e| LrcError::internal(format!("cls narrow: {}", e)))?
                    .squeeze(1)
                    .map_err(|e| LrcError::internal(format!("cls squeeze: {}", e)))?;
                cls.flatten_all()
                    .map_err(|e| LrcError::internal(format!("cls flatten: {}", e)))?
                    .to_vec1()
                    .map_err(|e| LrcError::internal(format!("cls to_vec1: {}", e)))?
            }
            PoolingStrategy::Mean => {
                // 掩码平均池化（与 encode_text 同一实现）
                let mask = attention_tensor
                    .unsqueeze(2)
                    .map_err(|e| LrcError::internal(format!("mask unsqueeze: {}", e)))?;
                let masked = output
                    .broadcast_mul(&mask)
                    .map_err(|e| LrcError::internal(format!("masked mul: {}", e)))?;
                let sum = masked
                    .sum(1)
                    .map_err(|e| LrcError::internal(format!("sum: {}", e)))?;
                let mask_sum = mask
                    .sum(1)
                    .map_err(|e| LrcError::internal(format!("mask_sum: {}", e)))?;
                let pooled = sum
                    .broadcast_div(&mask_sum)
                    .map_err(|e| LrcError::internal(format!("div: {}", e)))?;
                pooled
                    .flatten_all()
                    .map_err(|e| LrcError::internal(format!("mean flatten: {}", e)))?
                    .to_vec1()
                    .map_err(|e| LrcError::internal(format!("mean to_vec1: {}", e)))?
            }
        };
        Ok(vec)
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
                    // 签名迁移：record_degradation 接收 &str，LrcError 的 Display 即 message，
                    // 故 to_string() 与迁移前文案逐字一致。
                    self.record_degradation(&e.to_string());
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

    /// 批量编码文本为洛书向量（v0.9.10 吞吐优化）
    ///
    /// 语义与 [`Self::encode_text`] 完全一致（同样的降级/恢复状态机），差别只在
    /// 于有 ML 编码器时调用 `encode_text_batch` 做**单次批量前向**。约定：
    ///   - 空入参返回空 `Vec`；
    ///   - 输出条数与输入条数严格相等、顺序一一对应；
    ///   - **一次批量调用只算一次恢复探测**（冷却期内累计 1 次成功），避免
    ///     批量路径瞬间刷满 `recovery_threshold` 造成误恢复；
    ///   - 整批 ML 失败时，整批回退到统计编码器逐条编码。
    pub fn encode_text_batch(&self, texts: &[&str]) -> Vec<LuoShuVector> {
        if texts.is_empty() {
            return Vec::new();
        }

        // 与 encode_text 一致：先读降级状态（锁顺序 status → recovery）
        let is_degraded = {
            let recovery = self
                .recovery_state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            recovery.is_degraded
        };

        if let Some(ref ml) = self.ml_encoder {
            // v0.9.10：跨条目分片并行——瓶颈在单线程 elementwise，故按条目并行
            // 才能吃到多核；内部保序，返回顺序与输入严格一致。
            match encode_text_batch_parallel(ml, texts) {
                Ok(vecs) => {
                    // 长度契约防御：正常情况下二者的长度严格相等
                    if vecs.len() != texts.len() {
                        eprintln!(
                            "[LRC·洛书] 批量编码返回 {} 条，期望 {} 条；整批回退统计编码器",
                            vecs.len(),
                            texts.len()
                        );
                    } else {
                        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
                        status.total_encodings += vecs.len() as u64;
                        status.last_encoding_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;

                        // 延迟恢复：整批只算一次探测成功
                        if is_degraded {
                            let mut recovery = self
                                .recovery_state
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            recovery.consecutive_successes += 1;
                            eprintln!(
                                "[LRC·编码器] ML 批量探测成功 {}/{}（冷却中...）",
                                recovery.consecutive_successes, recovery.recovery_threshold
                            );

                            if recovery.consecutive_successes >= recovery.recovery_threshold {
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
                        }
                        return vecs;
                    }
                }
                Err(e) => {
                    eprintln!("[LRC·洛书] ML 批量编码失败 ({}), 整批回退到统计编码器", e);
                    self.record_degradation(&e.to_string());
                }
            }
        }

        // 统计模式编码（逐条）
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        status.total_encodings += texts.len() as u64;
        status.last_encoding_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        drop(status);

        // ML 存在但降级且本次 ML 失败 → 中断恢复过程
        if self.ml_encoder.is_some() {
            let mut recovery = self
                .recovery_state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if recovery.is_degraded && recovery.consecutive_successes > 0 {
                eprintln!(
                    "[LRC·编码器] ML 批量探测失败，重置冷却计数（之前: {} 次成功）",
                    recovery.consecutive_successes
                );
                recovery.consecutive_successes = 0;
            }
        }

        texts.iter().map(|t| self.fallback.encode_text(t)).collect()
    }

    /// 检查是否使用 ML 模式
    pub fn is_ml_mode(&self) -> bool {
        self.ml_encoder.is_some()
    }

    /// 获取底层 ML 编码器的完整语义句向量（bge 隐层均值池化，未经洛书投影）。
    ///
    /// 9 维洛书投影粒度太粗，承担不了"重要日子 ↔ 结婚纪念日"这类
    /// 语义强相关、词面零重叠的判断；完整句向量（384/768 维）才能。
    /// ML 编码器未加载或编码失败时返回 None——调用方据此退回词面
    /// 通路，绝不放宽标准。
    pub fn encode_embedding(&self, text: &str) -> Option<Vec<f32>> {
        self.ml_encoder.as_ref()?.encode_embedding(text).ok()
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

// ============================================================
// 跨条目并行编码（v0.9.10 吞吐优化）
// ============================================================
// 背景（实测驱动的自我评估结论）：
//   candle CPU 后端的矩阵乘已由 gemm+rayon 多核并行，故"批内并行"没有收益；
//   真正的瓶颈是单线程的 LayerNorm / GELU / softmax 等 elementwise 算子——
//   实测 reclassify 稳态进程 CPU 占用仅 ~1.5/6 核。
//   因此把并行度提到**条目维度**：把一批文本切成若干片，各片独立做前向，
//   让多个核同时推进 elementwise 计算，从而重叠彼此的单线程停顿。

/// 环境变量：跨条目并行编码的工作线程数覆盖项。
///
/// 不设置时按逻辑核数（`std::thread::available_parallelism`）自动决定；
/// 设为 1 可强制串行（便于对照实验或规避线程开销）。
const ENV_ENCODE_WORKERS: &str = "LRC_ENCODE_WORKERS";

/// 解析批量编码应使用的**跨条目**工作线程数（纯函数，便于无环境竞争地测试）。
///
/// 规则：
///   - `text_count ≤ 1` 直接返回 `text_count`（0/1 条不启线程）；
///   - 显式 `override_n` 且 >0 时采用之，否则取逻辑核数；
///   - 结果夹紧到 `[1, text_count]`，绝不超出待编码条目数（避免空分片）。
fn resolve_encode_workers(text_count: usize, override_n: Option<usize>) -> usize {
    if text_count <= 1 {
        return text_count;
    }
    let desired = match override_n {
        Some(n) if n > 0 => n,
        _ => std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1),
    };
    desired.clamp(1, text_count)
}

/// 读取 `LRC_ENCODE_WORKERS` 后解析工作线程数（环境变量是唯一的可配置入口）。
fn resolve_encode_workers_from_env(text_count: usize) -> usize {
    let override_n = std::env::var(ENV_ENCODE_WORKERS)
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok());
    resolve_encode_workers(text_count, override_n)
}

/// 将条目按 `workers` 分片并行执行 `f`，并按输入顺序拼接结果（保序契约）。
///
/// 设计要点：
///   - **保序**：先按分片顺序收集句柄，再顺序 `join`，输出与输入严格一一对应；
///   - **异常隔离**：任一分片返回 `Err` → 整体 `Err`；分片线程 `panic` →
///     转成 `LrcError`（绝不让调用方线程崩溃，也绝不静默丢条目）；
///   - `workers ≤ 1` 或条目 ≤1 时直接串行，避免无谓线程开销。
fn map_chunks_ordered<T, F>(texts: &[&str], workers: usize, f: F) -> LrcResult<Vec<T>>
where
    T: Send,
    F: Fn(&[&str]) -> LrcResult<Vec<T>> + Sync,
{
    if workers <= 1 || texts.len() <= 1 {
        return f(texts);
    }

    // 向上取整分片：保证片数 ≤ workers 且不产生空片
    let chunk_size = texts.len().div_ceil(workers);
    let mut out: Vec<T> = Vec::with_capacity(texts.len());

    std::thread::scope(|scope| -> LrcResult<()> {
        let mut handles = Vec::new();
        for slice in texts.chunks(chunk_size) {
            let f_ref = &f;
            handles.push(scope.spawn(move || f_ref(slice)));
        }
        // 顺序 join：这是"保序"的唯一保证，不得改为乱序收集
        for handle in handles {
            match handle.join() {
                Ok(Ok(part)) => out.extend(part),
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(LrcError::new(
                        ErrorKind::Internal,
                        "并行编码线程 panic，已隔离为错误",
                    ))
                }
            }
        }
        Ok(())
    })?;

    Ok(out)
}

/// 跨条目并行批量编码（工作线程数由 `LRC_ENCODE_WORKERS` 或逻辑核数决定）。
fn encode_text_batch_parallel(
    ml: &LuoShuMlEncoder,
    texts: &[&str],
) -> LrcResult<Vec<LuoShuVector>> {
    let workers = resolve_encode_workers_from_env(texts.len());
    encode_text_batch_parallel_with(ml, texts, workers)
}

/// 跨条目并行批量编码（显式指定工作线程数，供对照实验与测试使用）。
fn encode_text_batch_parallel_with(
    ml: &LuoShuMlEncoder,
    texts: &[&str],
    workers: usize,
) -> LrcResult<Vec<LuoShuVector>> {
    map_chunks_ordered(texts, workers, |chunk| {
        ml.encode_text_batch(chunk, EncodeRole::Passage)
    })
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

    /// 测试：八卦索引 → 洛书九宫位置映射的结构契约
    ///
    /// v0.9.10 取代原 `test_projection_initialization`（伪随机投影矩阵已删除，
    /// 改为结构投影）。这条契约必须成立，否则「卦」与「九宫位置」会静默错位：
    ///   1. 外圈 8 个位置各被占用一次（卦与位置一一对应）；
    ///   2. **绝不包含中心位置** —— 中心不属于任何梯形，即"中心点始终不变"。
    #[test]
    fn test_bagua_palace_pos_contract() {
        let mut seen = [false; 9];
        for &pos in BAGUA_PALACE_POS.iter() {
            assert!(pos < 9, "九宫位置越界: {}", pos);
            assert!(
                !seen[pos],
                "位置 {} 被重复占用（卦与位置必须一一对应）",
                pos
            );
            seen[pos] = true;
        }
        assert!(
            !seen[LUOSHU_CENTER_POS],
            "中心位置 {} 不得被任何卦占用（中心点始终不变）",
            LUOSHU_CENTER_POS
        );
        assert_eq!(
            seen.iter().filter(|&&s| s).count(),
            8,
            "外圈应恰好被占用 8 个位置"
        );
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

    /// v0.9.2 测试：对比度增强使不同输入产生不同 dominant 维度（防止八卦类别塌缩）
    #[test]
    fn test_contrast_normalize_amplifies_differences() {
        // 两个 dominant 维度不同的投影特征（模拟编码塌缩修复后的场景）
        let a = [0.3f32, 0.3, 0.9, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3]; // dominant at 2
        let b = [0.3f32, 0.3, 0.3, 0.3, 0.3, 0.3, 0.9, 0.3, 0.3]; // dominant at 6

        let na = LuoShuMlEncoder::contrast_normalize(&a, 0.7);
        let nb = LuoShuMlEncoder::contrast_normalize(&b, 0.7);

        // 输出应是概率分布（和为 1）
        let sum_a: f32 = na.iter().sum();
        let sum_b: f32 = nb.iter().sum();
        assert!((sum_a - 1.0).abs() < 1e-3, "a 之和应接近 1，实际 {sum_a}");
        assert!((sum_b - 1.0).abs() < 1e-3, "b 之和应接近 1，实际 {sum_b}");

        // a 的 dominant 维度应保持为 2，b 的 dominant 维度应保持为 6
        let argmax_a = na
            .iter()
            .enumerate()
            .max_by(|(_, x), (_, y)| x.partial_cmp(y).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        let argmax_b = nb
            .iter()
            .enumerate()
            .max_by(|(_, x), (_, y)| x.partial_cmp(y).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(argmax_a, 2, "a 的 dominant 维度应为 2，实际 {argmax_a}");
        assert_eq!(argmax_b, 6, "b 的 dominant 维度应为 6，实际 {argmax_b}");
    }

    /// v0.9.2 测试：混合融合后不同输入保持可区分（后验不被 LUOSHU_WEIGHTS 主导）
    #[test]
    fn test_mixture_fusion_keeps_posterior_derived_from_input() {
        // 模拟 encode_text 的融合步骤：enhanced 分布（来自对比度增强）与 LUOSHU_WEIGHTS 混合
        let enhanced_a = [0.05f32, 0.05, 0.45, 0.05, 0.05, 0.05, 0.2, 0.05, 0.05];
        let enhanced_b = [0.05f32, 0.45, 0.05, 0.05, 0.05, 0.05, 0.05, 0.2, 0.05];

        const PRIOR_WEIGHT: f32 = 0.25;
        let mut a = [0.0f32; 9];
        let mut b = [0.0f32; 9];
        for i in 0..9 {
            a[i] = (1.0 - PRIOR_WEIGHT) * enhanced_a[i] + PRIOR_WEIGHT * LUOSHU_WEIGHTS[i];
            b[i] = (1.0 - PRIOR_WEIGHT) * enhanced_b[i] + PRIOR_WEIGHT * LUOSHU_WEIGHTS[i];
        }

        // 后验的 dominant 维度应由输入（enhanced）决定，而非先验
        let argmax_a = a
            .iter()
            .enumerate()
            .max_by(|(_, x), (_, y)| x.partial_cmp(y).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        let argmax_b = b
            .iter()
            .enumerate()
            .max_by(|(_, x), (_, y)| x.partial_cmp(y).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(argmax_a, 2, "a 的后验 dominant 维度应为 2，实际 {argmax_a}");
        assert_eq!(argmax_b, 1, "b 的后验 dominant 维度应为 1，实际 {argmax_b}");
        assert_ne!(argmax_a, argmax_b, "不同输入应映射到不同八卦类别（防塌缩）");
    }

    /// v0.9.2 测试：特征全相等时退化为均匀分布（数值安全）
    #[test]
    fn test_contrast_normalize_uniform_fallback() {
        let uniform = [0.5f32; 9];
        let out = LuoShuMlEncoder::contrast_normalize(&uniform, 0.7);
        for v in out.iter() {
            assert!((v - 1.0 / 9.0).abs() < 1e-3, "应退化为均匀分布，实际 {v}");
        }
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

    /// 诊断：**去共模分量的 oracle 上界**（决定去共模这条路要不要走）
    ///
    /// ## 为什么先做这个
    ///
    /// 去共模需要知道公共方向 μ。而**任何 μ 估计都有误差**。若连"完美 μ"
    /// （直接用样本均值，即 oracle）都无法改善表示质量，那么换任何 μ 估计
    /// 都注定无效，这条路应当直接放弃——而不是先花力气去实现 μ 的估计。
    ///
    /// ## 判据（两个指标必须同时看）
    ///
    /// 1. **argmax 分散度**：命中位置数、最大单类占比（越大越差）
    /// 2. **语义分离度**：近义对余弦 − 无关对余弦（越大越好）
    ///
    /// 只看指标 1 会被误导：温度实验里 temp=0.3 时散布最大，但语义顺序
    /// 反而被打乱（近义分离度变负）。
    #[cfg(feature = "ml")]
    #[test]
    fn diagnostic_oracle_centering_upper_bound_ml() {
        let enc = match LuoShuMlEncoder::load() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[Oracle 中心化] 跳过：ML 模型不可用（{}）", e);
                return;
            }
        };

        // 近义对索引：(0,1) (2,3) (4,5)；其余为无关对
        let texts: [&str; 12] = [
            "数据库连接池的最大连接数配置为 20",
            "数据库连接池最大连接数设置成 20 个",
            "查询性能优化",
            "查询性能很差需要优化",
            "我喜欢喝美式咖啡，不加糖",
            "偏好美式咖啡不加糖",
            "项目使用 PostgreSQL 数据库存储用户数据",
            "前端使用 React 框架构建组件界面",
            "医生建议每天服用两次降压药",
            "周末去西湖散步顺便吃片儿川",
            "服务器内存不足被 OOM Killer 杀掉",
            "会议定在周三下午三点",
        ];
        let near_pairs: [(usize, usize); 3] = [(0, 1), (2, 3), (4, 5)];

        let embs: Vec<Vec<f32>> = texts
            .iter()
            .filter_map(|t| enc.encode_embedding(t).ok())
            .collect();
        if embs.len() != texts.len() {
            eprintln!(
                "[Oracle 中心化] 跳过：嵌入数量 {} != {}",
                embs.len(),
                texts.len()
            );
            return;
        }

        let dim = embs[0].len();
        let n = embs.len() as f32;

        // 公共方向 μ（样本均值 —— 即 oracle，此处仅用于对照打印）
        let mut mu = vec![0.0f32; dim];
        for e in &embs {
            for (i, x) in e.iter().enumerate() {
                mu[i] += x;
            }
        }
        for m in mu.iter_mut() {
            *m /= n;
        }

        // v0.9.10：结构投影已内建"减 μ"，本诊断改为比较「减 / 不减」的端到端效果。
        // 用的是 **encoder 自带的确定性 μ**（= 8 个原型嵌入的均值），而不是样本
        // 均值 —— 因为产品里不能有会随使用漂移的 μ（否则新旧记忆不可比）。
        let enc_mu: Vec<f32> = enc.archetype_mu.clone();
        {
            let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
            for i in 0..mu.len().min(enc_mu.len()) {
                dot += mu[i] * enc_mu[i];
                na += mu[i] * mu[i];
                nb += enc_mu[i] * enc_mu[i];
            }
            let c = if na > 1e-9 && nb > 1e-9 {
                dot / (na.sqrt() * nb.sqrt())
            } else {
                0.0
            };
            eprintln!(
                "[Oracle 中心化] 样本 μ 与 encoder μ 的余弦 = {:.4}（越接近 1，说明「原型均值」越能代表真实公共方向）",
                c
            );
        }

        for &centered in &[false, true] {
            let vecs: Vec<LuoShuVector> = embs
                .iter()
                .map(|e| {
                    let mut raw = [0.0f32; 9];
                    let d = e.len().min(enc.hidden_size).min(enc_mu.len());
                    let mut outer = [0.0f32; 8];
                    for (j, dir) in enc.archetype_dirs.iter().enumerate().take(8) {
                        let mut s = 0.0f32;
                        for i in 0..d {
                            let x = if centered { e[i] - enc_mu[i] } else { e[i] };
                            s += x * dir[i];
                        }
                        outer[j] = s;
                    }
                    for (j, &pos) in BAGUA_PALACE_POS.iter().enumerate() {
                        raw[pos] = outer[j];
                    }
                    raw[LUOSHU_CENTER_POS] = outer.iter().sum::<f32>() / 8.0;
                    let enhanced = LuoShuMlEncoder::contrast_normalize(&raw, 0.7);
                    // 复刻 encode_text_role 的归一化收口
                    let total: f32 = enhanced.iter().sum();
                    let posterior = if total > 1e-6 {
                        let mut p = [0.0f32; 9];
                        for i in 0..9 {
                            p[i] = enhanced[i] / total;
                        }
                        p
                    } else {
                        [1.0 / 9.0; 9]
                    };
                    let mut v = LuoShuVector { values: posterior };
                    v.normalize_to_luoshu();
                    v
                })
                .collect();

            // 指标 1：argmax 分散度
            let mut dist = [0usize; 9];
            for v in &vecs {
                let (mut bi, mut bv) = (0usize, f32::NEG_INFINITY);
                for (i, &x) in v.values.iter().enumerate() {
                    if x > bv {
                        bv = x;
                        bi = i;
                    }
                }
                dist[bi] += 1;
            }
            let hit = dist.iter().filter(|&&c| c > 0).count();
            let maxc = *dist.iter().max().unwrap_or(&0);

            // 指标 2：余弦分布 + 语义分离度
            let mut cos: Vec<f32> = Vec::new();
            for i in 0..vecs.len() {
                for j in (i + 1)..vecs.len() {
                    cos.push(vecs[i].cosine_similarity(&vecs[j]));
                }
            }
            cos.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |p: f32| cos[(((cos.len() - 1) as f32) * p) as usize];

            let near_mean: f32 = near_pairs
                .iter()
                .map(|&(i, j)| vecs[i].cosine_similarity(&vecs[j]))
                .sum::<f32>()
                / near_pairs.len() as f32;
            let mut far_sum = 0.0f32;
            let mut far_cnt = 0usize;
            for i in 0..vecs.len() {
                for j in (i + 1)..vecs.len() {
                    if !near_pairs.contains(&(i, j)) {
                        far_sum += vecs[i].cosine_similarity(&vecs[j]);
                        far_cnt += 1;
                    }
                }
            }
            let far_mean = far_sum / far_cnt.max(1) as f32;

            eprintln!(
                "[Oracle 中心化] centered={:<5} 分布={:?} 命中={}/9 最大单类={}",
                centered, dist, hit, maxc
            );
            eprintln!(
                "[Oracle 中心化] centered={:<5} 余弦 P10/P50/P90={:.4}/{:.4}/{:.4}  近义={:.4} 无关={:.4} 分离度={:+.4}",
                centered,
                q(0.10),
                q(0.50),
                q(0.90),
                near_mean,
                far_mean,
                near_mean - far_mean
            );
        }
    }

    /// 诊断（enc-1c）：**对比度温度扫描** —— 找出让结构空间分离度最佳的温度。
    ///
    /// ## 背景
    /// 新编码（BGE-base + 结构投影 + 减 μ）的中位两两余弦被压缩到 ~0.66，
    /// 远高于统计编码器 ~0.50，致 deep 排序分辨率下降（turn 级命中坍塌）。
    /// 本诊断固定 centered=true（= 产品路线），扫描 `contrast_temp` 观察：
    ///   - 余弦 P10/P50/P90（动态范围是否被拉开）
    ///   - 近义/无关分离度（判别力是否提升）
    ///   - argmax 分布命中度（表征是否仍分散，未塌缩回单类）
    /// 结论用于选 `LRC_LUOSHU_CONTRAST_TEMP`（env 可覆盖，**不改产品源码**）。
    #[cfg(feature = "ml")]
    #[test]
    fn diagnostic_contrast_temp_sweep_ml() {
        let enc = match LuoShuMlEncoder::load() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[温度扫描] 跳过：ML 模型不可用（{}）", e);
                return;
            }
        };

        // 与 oracle 诊断同批文本，保证可比
        let texts: [&str; 12] = [
            "数据库连接池的最大连接数配置为 20",
            "数据库连接池最大连接数设置成 20 个",
            "查询性能优化",
            "查询性能很差需要优化",
            "我喜欢喝美式咖啡，不加糖",
            "偏好美式咖啡不加糖",
            "项目使用 PostgreSQL 数据库存储用户数据",
            "前端使用 React 框架构建组件界面",
            "医生建议每天服用两次降压药",
            "周末去西湖散步顺便吃片儿川",
            "服务器内存不足被 OOM Killer 杀掉",
            "会议定在周三下午三点",
        ];
        let near_pairs: [(usize, usize); 3] = [(0, 1), (2, 3), (4, 5)];

        let embs: Vec<Vec<f32>> = texts
            .iter()
            .filter_map(|t| enc.encode_embedding(t).ok())
            .collect();
        if embs.len() != texts.len() {
            eprintln!(
                "[温度扫描] 跳过：嵌入数量 {} != {}",
                embs.len(),
                texts.len()
            );
            return;
        }

        // 构造闭包：给定温度，复刻 encode_text_role 的 centered 投影 + 对比度归一化 + 收口
        let build_one = |e: &Vec<f32>, temp: f32| -> LuoShuVector {
            let mut raw = [0.0f32; 9];
            let d = e.len().min(enc.hidden_size).min(enc.archetype_mu.len());
            let mut outer = [0.0f32; 8];
            for (j, dir) in enc.archetype_dirs.iter().enumerate().take(8) {
                let mut s = 0.0f32;
                for i in 0..d {
                    let x = e[i] - enc.archetype_mu[i];
                    s += x * dir[i];
                }
                outer[j] = s;
            }
            for (j, &pos) in BAGUA_PALACE_POS.iter().enumerate() {
                raw[pos] = outer[j];
            }
            raw[LUOSHU_CENTER_POS] = outer.iter().sum::<f32>() / 8.0;
            let enhanced = LuoShuMlEncoder::contrast_normalize(&raw, temp);
            let total: f32 = enhanced.iter().sum();
            let posterior = if total > 1e-6 {
                let mut p = [0.0f32; 9];
                for i in 0..9 {
                    p[i] = enhanced[i] / total;
                }
                p
            } else {
                [1.0 / 9.0; 9]
            };
            let mut v = LuoShuVector { values: posterior };
            v.normalize_to_luoshu();
            v
        };
        let build_vecs =
            |temp: f32| -> Vec<LuoShuVector> { embs.iter().map(|e| build_one(e, temp)).collect() };

        // enc-1c：自检门槛关乎「模型是否可用」，而它经 encode_text 间接用到 contrast_temp。
        // 同时打印自检文本 "Hello" 在各温度下的幻和偏离度，用于找出 temp 的安全区间。
        let hello_emb: Option<Vec<f32>> = enc.encode_embedding("Hello").ok();

        // 对给定向量序列打印分布 + 余弦分位 + 分离度
        let report = |tag: &str, vecs: &[LuoShuVector]| {
            let mut dist = [0usize; 9];
            for v in vecs {
                let (mut bi, mut bv) = (0usize, f32::NEG_INFINITY);
                for (i, &x) in v.values.iter().enumerate() {
                    if x > bv {
                        bv = x;
                        bi = i;
                    }
                }
                dist[bi] += 1;
            }
            let hit = dist.iter().filter(|&&c| c > 0).count();
            let maxc = *dist.iter().max().unwrap_or(&0);
            let mut cos: Vec<f32> = Vec::new();
            for i in 0..vecs.len() {
                for j in (i + 1)..vecs.len() {
                    cos.push(vecs[i].cosine_similarity(&vecs[j]));
                }
            }
            cos.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |p: f32| cos[(((cos.len() - 1) as f32) * p) as usize];
            let near_mean: f32 = near_pairs
                .iter()
                .map(|&(i, j)| vecs[i].cosine_similarity(&vecs[j]))
                .sum::<f32>()
                / near_pairs.len() as f32;
            let mut far_sum = 0.0f32;
            let mut far_cnt = 0usize;
            for i in 0..vecs.len() {
                for j in (i + 1)..vecs.len() {
                    if !near_pairs.contains(&(i, j)) {
                        far_sum += vecs[i].cosine_similarity(&vecs[j]);
                        far_cnt += 1;
                    }
                }
            }
            let far_mean = far_sum / far_cnt.max(1) as f32;
            eprintln!(
                "[温度扫描] {} 分布={:?} 命中={}/9 最大单类={}",
                tag, dist, hit, maxc
            );
            eprintln!(
                "[温度扫描] {} 余弦 P10/P50/P90={:.4}/{:.4}/{:.4}  近义={:.4} 无关={:.4} 分离度={:+.4}",
                tag,
                q(0.10),
                q(0.50),
                q(0.90),
                near_mean,
                far_mean,
                near_mean - far_mean
            );
        };

        for &temp in &[0.20f32, 0.30, 0.40, 0.50, 0.60, 0.70, 0.80, 1.00] {
            let vecs = build_vecs(temp);
            report(&format!("temp={:.2}", temp), &vecs);
            // 自检文本的幻和偏离度：load() 内以该值是否 < 2.0 决定 ML 是否可用
            if let Some(he) = &hello_emb {
                let dev = build_one(he, temp).luoshu_deviation();
                eprintln!(
                    "[温度扫描] temp={:.2} 自检文本\"Hello\" 幻和偏离度={:.3}（自检门槛 < 2.0：{}）",
                    temp,
                    dev,
                    if dev < 2.0 { "通过" } else { "拒绝" }
                );
            }
        }
    }

    /// 诊断：**联想对的「BGE 给不出」程度**（产品价值判据的直接检验）
    ///
    /// ## 判据出处
    ///
    /// `memory_store.rs` §3.43.9 逐字：
    /// > 「道体的价值判据不是'能否产出关联图'，而是'**产出的关联图中，
    /// >   有多少条是 BGE 给不出的**'」
    ///
    /// 且明确警告过陷阱：若本层只是 BGE 的粗粒度版本，则它与 BGE 等价、无独立价值。
    ///
    /// ## 测什么
    ///
    /// 对每条记忆：
    ///   - 在 **9 维结构空间**里取 top-3 邻居（= 结构视角给出的"联想对"）
    ///   - 记录这些对在 **768 维 BGE 全库排名**中的分位
    ///   - 同时记录 BGE **自身** top-3 邻居的分位，作为"零增量"基线
    ///
    /// **分位越高 = BGE 越排不出来 = 增量越大。**
    /// 若两者接近，说明结构视图只是复刻 BGE —— 那这条路就没有价值，应停手。
    #[cfg(feature = "ml")]
    #[test]
    fn diagnostic_bge_giveup_rate_ml() {
        let enc = match LuoShuMlEncoder::load() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[BGE 给不出率] 跳过：ML 模型不可用（{}）", e);
                return;
            }
        };

        // 模拟真实记忆库：同一项目内主题分散、任意两条都可能相邻
        let texts: [&str; 24] = [
            "数据库连接池的最大连接数配置为 20",
            "数据库连接池最大连接数设置成 20 个",
            "查询性能优化",
            "查询性能很差需要优化",
            "我喜欢喝美式咖啡，不加糖",
            "偏好美式咖啡不加糖",
            "项目使用 PostgreSQL 数据库存储用户数据",
            "前端使用 React 框架构建组件界面",
            "医生建议每天服用两次降压药",
            "周末去西湖散步顺便吃片儿川",
            "服务器内存不足被 OOM Killer 杀掉",
            "会议定在周三下午三点",
            "把日志级别从 debug 调回 info",
            "日志里出现了大量重复的 warning",
            "用户反馈登录按钮点了没反应",
            "登录成功后需要跳转到首页",
            "这个接口的响应时间超过了 2 秒",
            "接口返回 500 说明后端异常",
            "缓存失效导致每次都查数据库",
            "加一层 Redis 缓存能减轻数据库压力",
            "部署脚本里写错了环境变量名",
            "上线前必须先在预发环境验证一遍",
            "这个名字取得太随意了，后面会看不懂",
            "变量命名要能表达它的用途",
        ];

        let sem: Vec<Vec<f32>> = texts
            .iter()
            .filter_map(|t| enc.encode_embedding(t).ok())
            .collect();
        let st: Vec<LuoShuVector> = texts
            .iter()
            .filter_map(|t| enc.encode_text(t).ok())
            .collect();
        if sem.len() != texts.len() || st.len() != texts.len() {
            eprintln!("[BGE 给不出率] 跳过：编码数量不足");
            return;
        }

        let n = sem.len();
        let cos = |a: &[f32], b: &[f32]| -> f32 {
            let (mut d, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
            for i in 0..a.len().min(b.len()) {
                d += a[i] * b[i];
                na += a[i] * a[i];
                nb += b[i] * b[i];
            }
            if na > 1e-9 && nb > 1e-9 {
                d / (na.sqrt() * nb.sqrt())
            } else {
                0.0
            }
        };

        const TOP_K: usize = 3;
        let mut struct_ranks: Vec<f32> = Vec::new();
        let mut bge_ranks: Vec<f32> = Vec::new();

        for i in 0..n {
            // BGE 全库排名（降序），换算为分位（0 = 最相似，1 = 最不相似）
            let mut by_sem: Vec<(usize, f32)> = (0..n)
                .filter(|&j| j != i)
                .map(|j| (j, cos(&sem[i], &sem[j])))
                .collect();
            by_sem.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            let denom = (n - 2).max(1) as f32;
            let mut rank_of = std::collections::HashMap::new();
            for (r, (j, _)) in by_sem.iter().enumerate() {
                rank_of.insert(*j, r as f32 / denom);
            }

            // 9 维结构空间的 top-K 邻居 —— 这就是"结构视角给出的联想对"
            let mut by_struct: Vec<(usize, f32)> = (0..n)
                .filter(|&j| j != i)
                .map(|j| (j, st[i].cosine_similarity(&st[j])))
                .collect();
            by_struct.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            for k in 0..TOP_K.min(by_struct.len()) {
                if let Some(&r) = rank_of.get(&by_struct[k].0) {
                    struct_ranks.push(r);
                }
            }
            // 基线：BGE 自身 top-K（它们必然排在最前，分位≈0，"零增量"）
            for k in 0..TOP_K.min(by_sem.len()) {
                bge_ranks.push(rank_of[&by_sem[k].0]);
            }
        }

        let median = |mut v: Vec<f32>| -> f32 {
            if v.is_empty() {
                return -1.0;
            }
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        };
        let mean = |v: &[f32]| -> f32 {
            if v.is_empty() {
                -1.0
            } else {
                v.iter().sum::<f32>() / v.len() as f32
            }
        };

        let sm = median(struct_ranks.clone());
        let bm = median(bge_ranks.clone());
        eprintln!(
            "[BGE 给不出率] 结构联想对（9 维 top{}) 的 BGE 排名分位: 中位={:.4} 均值={:.4} 样本={}",
            TOP_K,
            sm,
            mean(&struct_ranks),
            struct_ranks.len()
        );
        eprintln!(
            "[BGE 给不出率] BGE 自身 top{}        的 BGE 排名分位: 中位={:.4} 均值={:.4} 样本={}",
            TOP_K,
            bm,
            mean(&bge_ranks),
            bge_ranks.len()
        );
        eprintln!(
            "[BGE 给不出率] 分位差 = {:.4}（越高说明结构联想越能给出 BGE 排不出来的关联；接近 0 则本层无独立价值）",
            sm - bm
        );
    }

    /// 测试：批量编码接口的**长度契约**（统计回退路径，无需 ML 模型）
    ///
    /// 契约：`encode_text_batch` 恒返回与输入等长的向量，空输入返回空。
    /// 这是 `MemoryStore` 分块批量重算做 `zip` 对齐的前提——长度不等会静默丢条目。
    #[test]
    fn test_hybrid_encode_text_batch_length_contract() {
        let encoder = HybridLuoShuEncoder::new_statistical();

        // 空输入 → 空输出（不得 panic）
        assert!(encoder.encode_text_batch(&[]).is_empty());

        // 含空串：回退路径也必须逐条给出 9 维向量
        let texts = ["数据库", "数据库配置", ""];
        let out = encoder.encode_text_batch(&texts);
        assert_eq!(out.len(), texts.len(), "批量输出条数必须与输入一致");
        for v in &out {
            assert_eq!(v.values.len(), 9);
        }
    }

    /// 测试：批量编码与逐条编码的**一致性契约**（批量吞吐优化的正确性护栏）
    ///
    /// 判据：① 条数一致；② 每条向量的 argmax（决定 `mirror_project` 的八卦归属）
    /// 与逐条编码一致；③ 数值差异在浮点容差内——批处理改变 GEMM 归约顺序，
    /// 允许 ~1e-3 级差异，但**不得改变离散分类**（否则历史重分类会漂移）。
    #[cfg(feature = "ml")]
    #[test]
    fn test_encode_text_batch_matches_single() {
        let enc = match LuoShuMlEncoder::load() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[批量编码一致性] 跳过：ML 模型不可用（{}）", e);
                return;
            }
        };

        let texts: [&str; 5] = [
            "数据库连接池的最大连接数配置为 20",
            "前端使用 React 框架构建组件界面",
            "周末去西湖散步顺便吃片儿川",
            "把日志级别从 debug 调回 info",
            "会议定在周三下午三点",
        ];
        // 本地 argmax（避免为测试引入额外依赖）
        let argmax = |v: &[f32; 9]| -> usize {
            let mut best = 0usize;
            for i in 1..9 {
                if v[i] > v[best] {
                    best = i;
                }
            }
            best
        };

        let batch = enc
            .encode_text_batch(&texts, EncodeRole::Passage)
            .expect("批量编码失败");
        assert_eq!(batch.len(), texts.len());

        for (i, t) in texts.iter().enumerate() {
            let single = enc
                .encode_text_role(t, EncodeRole::Passage)
                .expect("单条编码失败");
            assert_eq!(
                argmax(&batch[i].values),
                argmax(&single.values),
                "第 {} 条（{}）批量/逐条 argmax 不一致",
                i,
                t
            );
            for k in 0..9 {
                assert!(
                    (batch[i].values[k] - single.values[k]).abs() < 1e-3,
                    "第 {} 条维度 {} 差异过大: 批量={} 逐条={}",
                    i,
                    k,
                    batch[i].values[k],
                    single.values[k]
                );
            }
        }
    }

    /// 测试：批量编码的边界输入（空串 / 超长文本）不 panic 且条数正确
    ///
    /// 覆盖异常路径：空串只有特殊 token、超长需截断到 512、批内长度不齐需 padding。
    #[cfg(feature = "ml")]
    #[test]
    fn test_encode_text_batch_edge_inputs() {
        let enc = match LuoShuMlEncoder::load() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[批量编码边界] 跳过：ML 模型不可用（{}）", e);
                return;
            }
        };

        let long = "长".repeat(2000);
        let texts: Vec<&str> = vec!["", "短", long.as_str()];
        let out = enc
            .encode_text_batch(&texts, EncodeRole::Passage)
            .expect("边界批量编码失败");
        assert_eq!(out.len(), texts.len());
        for v in &out {
            assert_eq!(v.values.len(), 9);
            assert!(v.values.iter().all(|x| x.is_finite()), "向量含非有限值");
        }
    }

    /// 测试：跨线程并行编码的**工作线程数解析契约**（纯函数，无副作用）
    ///
    /// 判据：① 条目数 ≤1 时直接返回条目数（空/单条不启线程）；
    /// ② 显式覆盖值生效且被 `[1, text_count]` 夹紧；
    /// ③ 覆盖值非法（0）时回退到逻辑核数，同样被夹紧；
    /// ④ 结果恒 ≤ text_count，绝不超出待编码条目数。
    #[test]
    fn test_resolve_encode_workers_bounds() {
        // ① 退化输入：不启线程
        assert_eq!(resolve_encode_workers(0, None), 0);
        assert_eq!(resolve_encode_workers(1, None), 1);
        assert_eq!(resolve_encode_workers(1, Some(8)), 1);

        // ② 显式覆盖生效并夹紧
        assert_eq!(resolve_encode_workers(100, Some(1)), 1);
        assert_eq!(resolve_encode_workers(2, Some(4)), 2);
        assert_eq!(resolve_encode_workers(12, Some(3)), 3);

        // ③ 非法覆盖（0）→ 回退逻辑核数，仍夹紧到条目数以内
        let auto_zero = resolve_encode_workers(64, Some(0));
        assert!(
            (1..=64).contains(&auto_zero),
            "非法覆盖应回退到逻辑核数，实际 {}",
            auto_zero
        );

        // ④ 不设覆盖时按逻辑核数，恒 ≤ 条目数
        let auto = resolve_encode_workers(64, None);
        assert!(
            (1..=64).contains(&auto),
            "自动解析应落在 [1, text_count]，实际 {}",
            auto
        );
    }

    /// 测试：保序分片执行器的**顺序与结果一致性契约**（无 ML 依赖，纯逻辑）
    ///
    /// 覆盖异常路径（HCSE L5）：
    ///   - 正常路径：多线程结果与串行逐条结果完全一致且严格保序；
    ///   - 错误路径：任一分片返回 `Err` 时整体返回 `Err`（不静默丢条目）；
    ///   - 卡死路径：分片内 `panic` 被隔离为 `Err`，不炸掉调用方。
    #[test]
    fn test_map_chunks_ordered_contract() {
        // 12 条 ≥ workers(4) ⇒ 必然走到多线程分支
        let texts: Vec<&str> = vec![
            "a",
            "bb",
            "ccc",
            "dddd",
            "eeeee",
            "ffffff",
            "ggggggg",
            "hhhhhhhh",
            "iiiiiiiii",
            "jjjjjjjjjj",
            "kkkkkkkkkkk",
            "llllllllllll",
        ];

        // 正常路径：每片按元素自身长度产出 ⇒ 与分片方式无关，可验证保序
        let serial: Vec<usize> = texts.iter().map(|s| s.len()).collect();
        let parallel = map_chunks_ordered(&texts, 4, |chunk| {
            Ok(chunk.iter().map(|s| s.len()).collect())
        })
        .expect("并行执行失败");
        assert_eq!(parallel, serial, "并行结果必须与串行严格一致且保序");

        // workers=1 → 串行等价
        let single = map_chunks_ordered(&texts, 1, |chunk| {
            Ok(chunk.iter().map(|s| s.len()).collect())
        })
        .expect("串行执行失败");
        assert_eq!(single, serial, "workers=1 应与逐条串行一致");

        // 空输入 → 空输出
        let empty = map_chunks_ordered(&[] as &[&str], 4, |chunk| {
            Ok(chunk.iter().map(|s| s.len()).collect::<Vec<usize>>())
        })
        .expect("空输入应返回空");
        assert!(empty.is_empty());

        // 错误路径：分片返回 Err → 整体 Err
        let err = map_chunks_ordered(&texts, 4, |chunk| {
            if chunk.iter().any(|s| s.starts_with('g')) {
                Err(LrcError::new(ErrorKind::Timeout, "模拟分片超时"))
            } else {
                Ok(chunk.iter().map(|s| s.len()).collect())
            }
        });
        assert!(err.is_err(), "分片失败必须向上传播，不得静默丢条目");

        // 卡死路径：分片 panic → 隔离为 Err
        let panicked = map_chunks_ordered(&texts, 4, |chunk| {
            if chunk.iter().any(|s| s.starts_with('g')) {
                panic!("模拟分片线程崩溃");
            }
            Ok(chunk.iter().map(|s| s.len()).collect())
        });
        assert!(panicked.is_err(), "分片 panic 必须被隔离为 Err");
    }

    /// 测试：跨线程并行批量编码与**单次批量前向**的一致性 + 保序契约
    ///
    /// 12 条 ≥ workers(4) ⇒ 真正走多线程；逐维差异须 <1e-3、argmax 须相等，
    /// 否则历史重分类会因并行度不同而漂移（幂等性被破坏）。
    #[cfg(feature = "ml")]
    #[test]
    fn test_encode_batch_parallel_matches_serial() {
        let enc = match LuoShuMlEncoder::load() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[并行编码一致性] 跳过：ML 模型不可用（{}）", e);
                return;
            }
        };

        let texts: Vec<&str> = vec![
            "数据库连接池的最大连接数配置为 20",
            "前端使用 React 框架构建组件界面",
            "周末去西湖散步顺便吃片儿川",
            "把日志级别从 debug 调回 info",
            "会议定在周三下午三点",
            "用户登录接口需要校验 JWT 令牌",
            "缓存穿透的兜底策略是布隆过滤器",
            "把 CI 的构建产物缓存到 G 盘",
            "洛书编码器使用幻和归一化约束",
            "晚餐吃西红柿鸡蛋面",
            "把提交历史压缩成单个 commit",
            "雨天出门记得带伞",
        ];

        let argmax = |v: &[f32; 9]| -> usize {
            let mut best = 0usize;
            for i in 1..9 {
                if v[i] > v[best] {
                    best = i;
                }
            }
            best
        };

        // 串行基线：一次性批量前向
        let serial = enc
            .encode_text_batch(&texts, EncodeRole::Passage)
            .expect("串行批量编码失败");
        // 并行：显式 4 线程
        let parallel = encode_text_batch_parallel_with(&enc, &texts, 4).expect("并行批量编码失败");

        assert_eq!(parallel.len(), texts.len());
        for i in 0..texts.len() {
            assert_eq!(
                argmax(&parallel[i].values),
                argmax(&serial[i].values),
                "第 {} 条（{}）并行/串行 argmax 不一致",
                i,
                texts[i]
            );
            for k in 0..9 {
                assert!(
                    (parallel[i].values[k] - serial[i].values[k]).abs() < 1e-3,
                    "第 {} 条维度 {} 差异过大: 并行={} 串行={}",
                    i,
                    k,
                    parallel[i].values[k],
                    serial[i].values[k]
                );
            }
        }
    }
}
