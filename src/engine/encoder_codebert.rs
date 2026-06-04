// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含外部模型适配接口，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 外部编码器适配
// 集成第三方语义模型，需启用 `ml` feature。
//
// 默认模型: GraphCodeBERT (microsoft/graphcodebert-base)
//   同架构、同尺寸，代码检索精度比 CodeBERT 高 12.3%
// 可通过环境变量覆盖:
//   LRC_MODEL_ID=microsoft/codebert-base  (回退到 CodeBERT)
//   HF_ENDPOINT=https://hf-mirror.com     (国内镜像，默认已设置)

use super::encoder::{CodeEncoder, EmbeddingVector};
use crate::chunker::CodeChunk;
use candle_core::{Device, Tensor};
use std::fs::File;
use std::io::BufReader;

pub enum PoolingStrategy {
    Cls,
    Mean,
}

pub struct CodeBertEncoder {
    model: candle_transformers::models::bert::BertModel,
    tokenizer: tokenizers::Tokenizer,
    device: Device,
    pooling: PoolingStrategy,
}

impl CodeBertEncoder {
    pub fn load() -> Result<Self, String> {
        let device = Device::Cpu;

        // 默认使用 GraphCodeBERT（比 CodeBERT 代码检索精度高 12.3%，同架构同尺寸）
        let model_id = std::env::var("LRC_MODEL_ID")
            .unwrap_or_else(|_| "microsoft/graphcodebert-base".to_string());

        // ============================================================
        // 智能加载策略：本地 models/ 文件夹 → 缓存 → 远程下载
        // ============================================================
        // 1. 检查项目根目录 models/ 文件夹（用户手动放置 · 完全离线）
        // 2. 检查本地文件缓存（~/.cache/huggingface/hub/blobs/）
        // 3. 从 HF_ENDPOINT 镜像下载（默认 hf-mirror.com · 纯 ureq 请求）

        // 将 model_id 中的 / 替换为 -- 作为本地文件夹名
        // 例如: microsoft/graphcodebert-base → microsoft--graphcodebert-base
        let local_model_name = model_id.replace('/', "--");

        // 查找项目根目录的 models/ 文件夹
        let project_root = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        let local_model_dir = project_root.join("models").join(&local_model_name);

        // 也检查可执行文件所在目录的 models/ 文件夹
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| project_root.clone());
        let exe_model_dir = exe_dir.join("models").join(&local_model_name);

        let mut use_local = false;
        let mut model_dir = std::path::PathBuf::new();

        // 依次检查各个本地路径
        for dir in [&local_model_dir, &exe_model_dir] {
            let dir = dir.clone();
            if dir.join("config.json").exists()
                && (dir.join("model.safetensors").exists()
                    || dir.join("pytorch_model.bin").exists())
            {
                use_local = true;
                model_dir = dir.clone();
                println!("  ✓ 使用本地模型: {}", model_dir.display());
                break;
            }
        }

        if use_local {
            // 从本地 models/ 文件夹加载，完全不走网络
            let config_path = model_dir.join("config.json");

            // 智能选择模型格式：safetensors 原生加载，.bin 使用 PthTensors
            let is_safetensors = model_dir.join("model.safetensors").exists();
            let model_path = if is_safetensors {
                model_dir.join("model.safetensors")
            } else if model_dir.join("pytorch_model.bin").exists() {
                model_dir.join("pytorch_model.bin")
            } else {
                return Err(format!(
                    "本地模型目录缺少模型文件: {}\n\
                     提示: 需要 model.safetensors 或 pytorch_model.bin",
                    model_dir.display()
                ));
            };

            let config_file = File::open(&config_path)
                .map_err(|e| format!("打开 config.json 失败: {e}\n路径: {}", config_path.display()))?;
            let config: candle_transformers::models::bert::Config =
                serde_json::from_reader(BufReader::new(config_file))
                    .map_err(|e| format!("解析 config.json 失败: {e}"))?;

            // 智能加载 tokenizer：优先 tokenizer.json，回退 vocab.json + merges.txt
            let tokenizer = load_tokenizer_local(&model_dir, &local_model_name)?;

            let tensors: std::collections::HashMap<String, Tensor> = if is_safetensors {
                candle_core::safetensors::load(&model_path, &device)
                    .map_err(|e| format!("safetensors 加载失败: {e}\n路径: {}", model_path.display()))?
            } else {
                // pytorch_model.bin 使用 PthTensors 懒加载器
                let pth = candle_core::pickle::PthTensors::new(&model_path, None)
                    .map_err(|e| format!(
                        "pickle 加载 pytorch_model.bin 失败: {e}\n\
                         提示: 如果持续失败，请尝试转换为 safetensors 格式:\n\
                         pip install safetensors torch && python scripts/convert_model.py"
                    ))?;
                let mut tensors = std::collections::HashMap::new();
                for (name, _info) in pth.tensor_infos() {
                    if let Some(tensor) = pth.get(name)
                        .map_err(|e| format!("加载 tensor '{}' 失败: {}", name, e))?
                    {
                        tensors.insert(name.to_string(), tensor);
                    }
                }
                if tensors.is_empty() {
                    return Err(format!(
                        "pytorch_model.bin 中未找到任何 tensor\n\
                         提示: 文件可能已损坏，请尝试重新下载"
                    ));
                }
                tensors
            };

            let vb = candle_nn::VarBuilder::from_tensors(
                tensors,
                candle_transformers::models::bert::DTYPE,
                &device,
            );

            let model = candle_transformers::models::bert::BertModel::load(vb, &config)
                .map_err(|e| format!("构建模型失败: {e}"))?;

            println!(
                "  本地模型加载成功 (hidden_size={}, device=CPU)",
                config.hidden_size
            );

            return Ok(Self {
                model,
                tokenizer,
                device,
                pooling: PoolingStrategy::Cls,
            });
        }

        // ============================================================
        // 本地没有模型，从远程下载
        // ============================================================

        // 始终使用 HF_ENDPOINT 指定的镜像站点（默认 hf-mirror.com）
        // 注意：绝不硬编码 huggingface.co，绝不通过 hf-hub 库发起网络请求
        let endpoint = std::env::var("HF_ENDPOINT")
            .unwrap_or_else(|_| "https://hf-mirror.com".to_string());

        println!("  ↓ 下载模型: {} (来源: {})", model_id, endpoint);
        println!("  提示: 如果下载慢，可将模型文件放到 models/{} 目录", local_model_name);

        // 获取 hf-hub 标准缓存目录（仅用于路径计算，绝不做任何网络请求）
        let cache = hf_hub::Cache::default();
        // 缓存目录结构: ~/.cache/huggingface/hub/models--{org}--{repo}/blobs/
        let folder_name = format!("models--{}", model_id.replace('/', "--"));
        let cache_dir = cache.path().join(folder_name).join("blobs");

        // 纯本地缓存检测 + 自定义下载函数
        // 彻底绕过 hf-hub 的网络层，所有 HTTP 请求由 ureq 直接发起
        // 原因：hf-hub 内部可能忽略 HF_ENDPOINT 设置，直接访问 huggingface.co
        // 注意：ureq 2.x 对非 2xx 状态码返回 Err，需用 match 而非 ? 运算符
        fn manual_download(
            filename: &str,
            endpoint: &str,
            model_id: &str,
            cache_dir: &std::path::Path,
        ) -> Result<std::path::PathBuf, String> {
            // 1. 纯本地文件系统检测缓存（绝不做任何网络请求）
            let cached_path = cache_dir.join(filename);
            if cached_path.exists() {
                let size = std::fs::metadata(&cached_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                println!("    ✓ 缓存命中: {} ({} MB)", filename, size / 1024 / 1024);
                return Ok(cached_path);
            }

            // 2. 缓存未命中，使用 ureq 直接下载（URL 由 endpoint 变量控制）
            let url = format!(
                "{}/{}/resolve/main/{}",
                endpoint.trim_end_matches('/'),
                model_id,
                filename
            );

            // 最多重试 3 次（处理网络不稳定导致的大文件下载中断）
            let max_retries = 3;
            let mut last_error = String::new();

            for attempt in 1..=max_retries {
                if attempt > 1 {
                    println!("    ↻ 重试 {}/{}: {}", attempt, max_retries, filename);
                } else {
                    println!("    ↓ 下载: {}", url);
                }

                let response = match ureq::get(&url).call() {
                    Ok(r) => r,
                    Err(ureq::Error::Status(code, _response)) => {
                        last_error = format!(
                            "HTTP {} (URL: {})\n\
                             提示: 模型文件可能不存在，请确认模型 ID 正确: {}",
                            code, url, model_id
                        );
                        break; // 非网络错误，不重试
                    }
                    Err(e) => {
                        last_error = format!(
                            "下载失败: {} (URL: {})\n\
                             提示: 请检查网络连接，或手动将模型文件放到 models/ 目录",
                            e, url
                        );
                        if attempt < max_retries {
                            std::thread::sleep(std::time::Duration::from_secs(2));
                        }
                        continue;
                    }
                };

                // 读取 Content-Length 用于后续完整性校验
                let expected_size: Option<u64> = response
                    .header("Content-Length")
                    .and_then(|v| v.parse().ok());

                // 3. 写入缓存目录
                if let Err(e) = std::fs::create_dir_all(cache_dir) {
                    return Err(format!("创建缓存目录失败: {}", e));
                }

                let mut file = match std::fs::File::create(&cached_path) {
                    Ok(f) => f,
                    Err(e) => return Err(format!("创建文件失败: {}", e)),
                };

                let mut reader = response.into_reader();
                match std::io::copy(&mut reader, &mut file) {
                    Ok(bytes_written) => {
                        // 4. 完整性校验：对比 Content-Length 与实际写入字节数
                        // 如果大小不匹配，说明下载中断，文件损坏
                        if let Some(expected) = expected_size {
                            if bytes_written != expected {
                                let _ = std::fs::remove_file(&cached_path);
                                last_error = format!(
                                    "文件大小校验失败: 期望 {} MB, 实际 {} MB ({} 字节)\n\
                                     提示: 网络连接不稳定，下载中断。已自动重试 {}/{} 次",
                                    expected / 1024 / 1024,
                                    bytes_written / 1024 / 1024,
                                    bytes_written,
                                    attempt,
                                    max_retries
                                );
                                // 练习 1 秒后重试，给网络恢复时间
                                std::thread::sleep(std::time::Duration::from_secs(
                                    (attempt as u64).min(5),
                                ));
                                continue;
                            }
                        }

                        let size_mb = bytes_written / 1024 / 1024;
                        println!("    ✓ 下载完成: {} ({} MB)", filename, size_mb);
                        return Ok(cached_path);
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&cached_path);
                        last_error = format!("写入文件失败: {}", e);
                        if attempt < max_retries {
                            std::thread::sleep(std::time::Duration::from_secs(2));
                        }
                        continue;
                    }
                }
            }

            Err(last_error)
        }

        // 尝试下载文件，404 返回 None 而非错误（用于可选文件下载）
        // 注意：ureq 2.x 对非 2xx 状态码返回 Err，需用 match 而非 ? 运算符
        fn try_download(
            filename: &str,
            endpoint: &str,
            model_id: &str,
            cache_dir: &std::path::Path,
        ) -> Result<Option<std::path::PathBuf>, String> {
            let cached_path = cache_dir.join(filename);
            if cached_path.exists() {
                let size = std::fs::metadata(&cached_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                println!("    ✓ 缓存命中: {} ({} KB)", filename, size / 1024);
                return Ok(Some(cached_path));
            }

            let url = format!(
                "{}/{}/resolve/main/{}",
                endpoint.trim_end_matches('/'),
                model_id,
                filename
            );
            println!("    ↓ 尝试: {}", url);

            // ureq 2.x 对非 2xx 返回 Err(Error::Status(code, response))
            let response = match ureq::get(&url).call() {
                Ok(r) => r,
                Err(ureq::Error::Status(404, _response)) => {
                    println!("    ✗ 不存在 (404): {}", filename);
                    return Ok(None);
                }
                Err(ureq::Error::Status(code, _response)) => {
                    return Err(format!("HTTP {} (URL: {})", code, url));
                }
                Err(e) => {
                    return Err(format!(
                        "下载失败: {} (URL: {})\n提示: 请检查网络连接",
                        e, url
                    ));
                }
            };

            // 读取 Content-Length 用于完整性校验
            let expected_size: Option<u64> = response
                .header("Content-Length")
                .and_then(|v| v.parse().ok());

            std::fs::create_dir_all(cache_dir)
                .map_err(|e| format!("创建缓存目录失败: {}", e))?;
            let mut file = std::fs::File::create(&cached_path)
                .map_err(|e| format!("创建文件失败: {}", e))?;
            let mut reader = response.into_reader();
            let bytes_written = std::io::copy(&mut reader, &mut file)
                .map_err(|e| format!("写入文件失败: {}", e))?;

            // 完整性校验
            if let Some(expected) = expected_size {
                if bytes_written != expected {
                    let _ = std::fs::remove_file(&cached_path);
                    return Err(format!(
                        "文件大小校验失败: {} (期望 {} MB, 实际 {} MB)",
                        filename,
                        expected / 1024 / 1024,
                        bytes_written / 1024 / 1024
                    ));
                }
            }

            let size_kb = bytes_written / 1024;
            println!("    ✓ 下载完成: {} ({} KB)", filename, size_kb);
            Ok(Some(cached_path))
        }

        // 本地 tokenizer 加载：优先 tokenizer.json，回退到 vocab.json + merges.txt
        fn load_tokenizer_local(
            model_dir: &std::path::Path,
            _model_name: &str,
        ) -> Result<tokenizers::Tokenizer, String> {
            // 1. 尝试 tokenizer.json
            let tokenizer_json = model_dir.join("tokenizer.json");
            if tokenizer_json.exists() {
                return tokenizers::Tokenizer::from_file(&tokenizer_json)
                    .map_err(|e| format!("加载 tokenizer.json 失败: {}", e));
            }

            // 2. 回退到 vocab.json + merges.txt
            let vocab_path = model_dir.join("vocab.json");
            let merges_path = model_dir.join("merges.txt");

            if !vocab_path.exists() {
                return Err(format!(
                    "本地模型缺少 tokenizer.json: {}\n\
                     提示: 请确保 models/{} 目录包含完整的模型文件（vocab.json + merges.txt 或 tokenizer.json）",
                    tokenizer_json.display(),
                    model_dir.file_name().unwrap_or_default().to_string_lossy()
                ));
            }

            if !merges_path.exists() {
                return Err(format!(
                    "本地模型缺少 merges.txt: {}\n\
                     提示: 请确保 models/{} 目录包含完整的模型文件",
                    merges_path.display(),
                    model_dir.file_name().unwrap_or_default().to_string_lossy()
                ));
            }

            // 使用 BPE 模型从 vocab.json + merges.txt 构建 tokenizer
            let bpe = tokenizers::models::bpe::BPE::from_file(
                vocab_path.to_str().unwrap(),
                merges_path.to_str().unwrap(),
            )
            .build()
            .map_err(|e| format!("构建 BPE 模型失败: {}", e))?;

            let mut tokenizer = tokenizers::Tokenizer::new(bpe);

            // 配置 RoBERTa 风格的 tokenizer 组件
            let normalizer = tokenizers::normalizers::bert::BertNormalizer::default();
            tokenizer.with_normalizer(Some(normalizer));

            let pre_tokenizer = tokenizers::pre_tokenizers::byte_level::ByteLevel::default();
            tokenizer.with_pre_tokenizer(Some(pre_tokenizer));

            let decoder = tokenizers::decoders::byte_level::ByteLevel::default();
            tokenizer.with_decoder(Some(decoder));

            let post_processor = tokenizers::processors::roberta::RobertaProcessing::default();
            tokenizer.with_post_processor(Some(post_processor));

            // 添加 RoBERTa 特殊 tokens
            tokenizer.add_special_tokens(&[
                tokenizers::AddedToken::from("<s>", true),     // bos_token_id=0
                tokenizers::AddedToken::from("<pad>", true),   // pad_token_id=1
                tokenizers::AddedToken::from("</s>", true),    // eos_token_id=2
                tokenizers::AddedToken::from("<unk>", true),   // unk_token_id=3
                tokenizers::AddedToken::from("<mask>", true),  // mask_token_id=50264
            ]);

            println!("    ✓ 使用 vocab.json + merges.txt 格式加载本地 tokenizer");
            Ok(tokenizer)
        }

        // 智能加载 tokenizer：优先 tokenizer.json，回退到 vocab.json + merges.txt
        // GraphCodeBERT 使用 RoBERTa 格式（vocab.json + merges.txt），没有 tokenizer.json
        fn load_tokenizer(
            endpoint: &str,
            model_id: &str,
            cache_dir: &std::path::Path,
        ) -> Result<tokenizers::Tokenizer, String> {
            // 1. 尝试 tokenizer.json（HuggingFace tokenizers 统一格式）
            if let Some(path) = try_download("tokenizer.json", endpoint, model_id, cache_dir)? {
                return tokenizers::Tokenizer::from_file(&path)
                    .map_err(|e| format!("加载 tokenizer.json 失败: {}", e));
            }

            // 2. 回退到 vocab.json + merges.txt（RoBERTa/GPT-2 等旧格式）
            println!("    tokenizer.json 不存在，使用 vocab.json + merges.txt 格式...");

            let vocab_path = manual_download("vocab.json", endpoint, model_id, cache_dir)
                .map_err(|e| format!("vocab.json: {}", e))?;
            let merges_path = manual_download("merges.txt", endpoint, model_id, cache_dir)
                .map_err(|e| format!("merges.txt: {}", e))?;

            // 下载辅助配置文件（不影响核心功能，但建议下载）
            let _ = try_download("special_tokens_map.json", endpoint, model_id, cache_dir);
            let _ = try_download("tokenizer_config.json", endpoint, model_id, cache_dir);

            // 使用 BPE 模型从 vocab.json + merges.txt 构建 tokenizer
            let bpe = tokenizers::models::bpe::BPE::from_file(
                vocab_path.to_str().unwrap(),
                merges_path.to_str().unwrap(),
            )
            .build()
            .map_err(|e| format!("构建 BPE 模型失败: {}", e))?;

            let mut tokenizer = tokenizers::Tokenizer::new(bpe);

            // 配置 RoBERTa 风格的 tokenizer 组件
            let normalizer = tokenizers::normalizers::bert::BertNormalizer::default();
            tokenizer.with_normalizer(Some(normalizer));

            let pre_tokenizer = tokenizers::pre_tokenizers::byte_level::ByteLevel::default();
            tokenizer.with_pre_tokenizer(Some(pre_tokenizer));

            let decoder = tokenizers::decoders::byte_level::ByteLevel::default();
            tokenizer.with_decoder(Some(decoder));

            let post_processor = tokenizers::processors::roberta::RobertaProcessing::default();
            tokenizer.with_post_processor(Some(post_processor));

            // 添加 RoBERTa 特殊 tokens
            tokenizer.add_special_tokens(&[
                tokenizers::AddedToken::from("<s>", true),     // bos_token_id=0
                tokenizers::AddedToken::from("<pad>", true),   // pad_token_id=1
                tokenizers::AddedToken::from("</s>", true),    // eos_token_id=2
                tokenizers::AddedToken::from("<unk>", true),   // unk_token_id=3
                tokenizers::AddedToken::from("<mask>", true),  // mask_token_id=50264
            ]);

            println!("    ✓ 使用 vocab.json + merges.txt 格式加载 tokenizer");
            Ok(tokenizer)
        }

        let config_path = manual_download("config.json", &endpoint, &model_id, &cache_dir)
            .map_err(|e| format!("config.json: {}", e))?;
        let tokenizer = load_tokenizer(&endpoint, &model_id, &cache_dir)
            .map_err(|e| format!("tokenizer 加载失败: {}", e))?;

        // 模型格式降级：safetensors → pytorch_model.bin
        // GraphCodeBERT 只有 pytorch_model.bin，没有 safetensors 格式
        let model_path = manual_download("model.safetensors", &endpoint, &model_id, &cache_dir)
            .or_else(|_| manual_download("pytorch_model.bin", &endpoint, &model_id, &cache_dir))
            .map_err(|e| format!(
                "模型文件下载失败: {}\n\
                 提示: 1) 检查网络连接 2) 确认模型 ID 正确: '{}'\n\
                 3) 若只有 pytorch_model.bin 格式，请运行:\n\
                 pip install safetensors torch && python scripts/convert_to_safetensors.py",
                e, model_id
            ))?;

        let config_file = File::open(&config_path)
            .map_err(|e| format!("open config: {e}"))?;
        let config: candle_transformers::models::bert::Config =
            serde_json::from_reader(BufReader::new(config_file))
                .map_err(|e| format!("parse config: {e}"))?;

        // 根据文件格式选择加载器：.safetensors 用原生加载，.bin 用 pickle 加载 PyTorch 格式
        let is_pytorch_bin = model_path
            .to_str()
            .map_or(false, |s| s.ends_with(".bin"));
        let tensors: std::collections::HashMap<String, Tensor> = if is_pytorch_bin {
            // 使用 PthTensors 懒加载器读取 PyTorch checkpoint
            // 注意：graphcodebert-base 是旧格式 raw pickle（非 zip），
            // candle_core::pickle::read_all 只支持 zip 格式，需用 PthTensors
            let pth = candle_core::pickle::PthTensors::new(&model_path, None)
                .map_err(|e| format!("pickle 加载 pytorch_model.bin 失败: {e}\n\
                    提示: 如果持续失败，请尝试转换为 safetensors 格式:\n\
                    pip install safetensors torch && python scripts/convert_to_safetensors.py"))?;
            let mut tensors = std::collections::HashMap::new();
            for (name, _info) in pth.tensor_infos() {
                if let Some(tensor) = pth.get(name)
                    .map_err(|e| format!("加载 tensor '{}' 失败: {}", name, e))?
                {
                    tensors.insert(name.to_string(), tensor);
                }
            }
            if tensors.is_empty() {
                return Err(format!(
                    "pytorch_model.bin 中未找到任何 tensor\n\
                     提示: 文件可能已损坏，请尝试重新下载\n\
                     cargo run --release --features ml 会自动重新下载"
                ));
            }
            tensors
        } else {
            candle_core::safetensors::load(&model_path, &device)
                .map_err(|e| format!("safetensors 加载失败: {e}"))?
        };
        let vb = candle_nn::VarBuilder::from_tensors(
            tensors,
            candle_transformers::models::bert::DTYPE,
            &device,
        );

        let model = candle_transformers::models::bert::BertModel::load(vb, &config)
            .map_err(|e| format!("model: {e}"))?;

        println!(
            "external encoder loaded (hidden_size={}, device=CPU)",
            config.hidden_size
        );

        Ok(Self {
            model,
            tokenizer,
            device,
            pooling: PoolingStrategy::Cls,
        })
    }

    pub fn with_pooling(mut self, strategy: PoolingStrategy) -> Self {
        self.pooling = strategy;
        self
    }

    fn encode_text(&self, text: &str) -> Result<EmbeddingVector, String> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| format!("tokenize: {e}"))?;

        let token_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let attention_mask: Vec<f32> =
            encoding.get_attention_mask().iter().map(|&m| m as f32).collect();
        let seq_len = token_ids.len();

        if seq_len > 512 {
            return Err(format!("text too long: {} tokens (max 512)", seq_len));
        }

        let input_ids = Tensor::new(token_ids.as_slice(), &self.device)
            .map_err(|e| format!("input_ids: {e}"))?
            .unsqueeze(0)
            .map_err(|e| format!("unsqueeze: {e}"))?;

        let token_type_ids = input_ids
            .zeros_like()
            .map_err(|e| format!("type_ids: {e}"))?;

        let attention_tensor = Tensor::new(attention_mask.as_slice(), &self.device)
            .map_err(|e| format!("attention: {e}"))?
            .unsqueeze(0)
            .map_err(|e| format!("unsqueeze: {e}"))?;

        let output = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_tensor))
            .map_err(|e| format!("forward: {e}"))?;

        let values = match self.pooling {
            PoolingStrategy::Cls => {
                let cls = output
                    .get(0)
                    .map_err(|e| format!("batch: {e}"))?
                    .get(0)
                    .map_err(|e| format!("cls: {e}"))?;
                cls.to_vec1().map_err(|e| format!("to_vec: {e}"))?
            }
            PoolingStrategy::Mean => {
                let mask = attention_tensor
                    .unsqueeze(2)
                    .map_err(|e| format!("mask unsqueeze: {e}"))?;
                let masked = output
                    .broadcast_mul(&mask)
                    .map_err(|e| format!("masked mul: {e}"))?;
                let sum = masked
                    .sum_keepdim(1)
                    .map_err(|e| format!("sum: {e}"))?;
                let count = mask
                    .sum_keepdim(1)
                    .map_err(|e| format!("count: {e}"))?;
                let pooled = sum
                    .broadcast_div(&count)
                    .map_err(|e| format!("div: {e}"))?;
                pooled
                    .squeeze(0)
                    .map_err(|e| format!("squeeze: {e}"))?
                    .squeeze(0)
                    .map_err(|e| format!("squeeze: {e}"))?
                    .to_vec1()
                    .map_err(|e| format!("to_vec: {e}"))?
            }
        };

        Ok(EmbeddingVector {
            dim: values.len(),
            values,
        })
    }
}

impl CodeEncoder for CodeBertEncoder {
    fn encode(&self, chunk: &CodeChunk) -> EmbeddingVector {
        let text = format!(
            "{} {} {}",
            chunk.signature,
            chunk.doc_comment.as_deref().unwrap_or(""),
            chunk.content
        );

        match self.encode_text(&text) {
            Ok(vec) => vec,
            Err(e) => {
                eprintln!("encode failed ({}): {}", chunk.name, e);
                EmbeddingVector::zeros(self.dimension())
            }
        }
    }

    fn dimension(&self) -> usize {
        768
    }
}

// ==================== 测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::encoder::FastEncoder;
    use crate::engine::model_resolver;

    fn should_skip() -> bool {
        // P0-3: 使用模型文件检测代替环境变量，确保有模型时真实执行测试
        let model_ready =
            model_resolver::check_model_ready("microsoft/graphcodebert-base");
        if !model_ready {
            println!(
                "[跳过] ML 测试：模型文件未在本地就绪。启动服务时将自动下载。"
            );
        }
        !model_ready
    }

    fn make_chunk(name: &str, content: &str) -> CodeChunk {
        CodeChunk {
            id: format!("test.rs:L1-L{}", content.lines().count()),
            file_path: "test.rs".to_string(),
            start_line: 1,
            end_line: content.lines().count(),
            chunk_type: "fn".to_string(),
            name: name.to_string(),
            signature: format!("fn {}()", name),
            content: content.to_string(),
            doc_comment: None,
            language: "rust".to_string(),
        }
    }

    #[test]
    fn test_load() {
        if should_skip() { return; }
        let encoder = CodeBertEncoder::load().expect("load");
        assert_eq!(encoder.dimension(), 768);
    }

    #[test]
    fn test_encode_dim() {
        if should_skip() { return; }
        let encoder = CodeBertEncoder::load().expect("load");
        let chunk = make_chunk("hello", "fn hello() { println!(\"world\"); }");
        let vec = encoder.encode(&chunk);
        assert_eq!(vec.dim, 768);
        assert_eq!(vec.values.len(), 768);
    }

    #[test]
    fn test_similarity() {
        if should_skip() { return; }
        let encoder = CodeBertEncoder::load().expect("load");

        let c1 = make_chunk("store", "fn store(item: &Item) { db.insert(item); }");
        let c2 = make_chunk("save", "fn save(rec: Record) { db.write(rec); }");
        let c3 = make_chunk("render", "fn render(tpl: &str) -> String { tpl.replace(\"{{\", \"<\") }");

        let v1 = encoder.encode(&c1);
        let v2 = encoder.encode(&c2);
        let v3 = encoder.encode(&c3);

        let sim_related = v1.cosine_similarity(&v2);
        let sim_unrelated = v1.cosine_similarity(&v3);

        println!("similarity: related={:.4}, unrelated={:.4}", sim_related, sim_unrelated);
        assert!(sim_related > sim_unrelated);
    }

    #[test]
    fn test_self_similarity() {
        if should_skip() { return; }
        let encoder = CodeBertEncoder::load().expect("load");
        let chunk = make_chunk("fetch", "fn fetch(key: &str) -> Option<Item> { cache.get(key) }");

        let v1 = encoder.encode(&chunk);
        let v2 = encoder.encode(&chunk);

        let sim = v1.cosine_similarity(&v2);
        assert!(sim > 0.999, "self-sim={:.6}", sim);
    }

    #[test]
    fn test_batch() {
        if should_skip() { return; }
        let encoder = CodeBertEncoder::load().expect("load");
        let chunks = vec![make_chunk("a", "fn a() {}"), make_chunk("b", "fn b() {}")];
        let vectors = encoder.encode_batch(&chunks);
        assert_eq!(vectors.len(), 2);
    }

    use super::super::retriever::{CodeRetriever, LocalRetriever};
    use std::sync::Arc;

    #[test]
    fn test_comparison_dim() {
        if should_skip() { return; }
        let cb = CodeBertEncoder::load().expect("load");
        let fast = FastEncoder::new(vec!["fn".into(), "struct".into(), "alpha".into(), "beta".into(), "gamma".into()]);

        let chunk = make_chunk("hello", "fn hello() {}");
        let cb_vec = cb.encode(&chunk);
        let fast_vec = fast.encode(&chunk);

        assert_eq!(cb_vec.dim, 768);
        assert_eq!(fast_vec.dim, 5);
    }

    #[test]
    fn test_comparison_gap() {
        if should_skip() { return; }
        let cb = CodeBertEncoder::load().expect("load");
        let fast = FastEncoder::new(vec![
            "fn".into(), "db".into(), "insert".into(), "save".into(), "render".into(),
            "html".into(), "template".into(), "user".into(), "record".into(),
        ]);

        let c1 = make_chunk("save_user", "fn save_user(u: &User) -> Result { db.insert(\"users\", u)?; Ok(()) }");
        let c2 = make_chunk("save_rec", "fn save_rec(r: &Record) -> Result { db.insert(\"recs\", r)?; Ok(()) }");
        let c3 = make_chunk("render_page", "fn render_page(t: &Template) -> Html { t.render(&ctx).unwrap() }");

        let cb_related = cb.encode(&c1).cosine_similarity(&cb.encode(&c2));
        let cb_unrelated = cb.encode(&c1).cosine_similarity(&cb.encode(&c3));
        let fast_related = fast.encode(&c1).cosine_similarity(&fast.encode(&c2));
        let fast_unrelated = fast.encode(&c1).cosine_similarity(&fast.encode(&c3));

        println!("gap: cb=related={:.4} unrelated={:.4}, fast=related={:.4} unrelated={:.4}",
            cb_related, cb_unrelated, fast_related, fast_unrelated);

        assert!(cb_related > cb_unrelated);
        assert!(fast_related > fast_unrelated);
        assert!(cb_related - cb_unrelated >= (fast_related - fast_unrelated) * 0.8);
    }

    #[test]
    fn test_comparison_retriever() {
        if should_skip() { return; }
        let cb = CodeBertEncoder::load().expect("load");
        let fast = Arc::new(FastEncoder::new(vec![
            "fn".into(), "alpha".into(), "beta".into(), "gamma".into(), "delta".into(),
            "epsilon".into(), "zeta".into(), "eta".into(), "theta".into(), "iota".into(),
        ]));

        let mut fast_ret = LocalRetriever::new(fast, 0.01);
        let mut cb_ret = LocalRetriever::new(Arc::new(cb), 0.01);

        let chunks = vec![
            make_chunk("fetch", "fn fetch(key: &str) -> Option<Item> { cache.get(key).cloned() }"),
            make_chunk("store", "fn store(key: &str, val: Item) { cache.insert(key.to_string(), val); }"),
            make_chunk("load_cfg", "fn load_cfg(path: &str) -> Config { toml::from_str(&fs::read_to_string(path).unwrap()).unwrap() }"),
            make_chunk("render_tpl", "fn render_tpl(name: &str, ctx: &Ctx) -> String { tmpl.render(name, ctx).unwrap() }"),
        ];

        for c in &chunks {
            fast_ret.index_chunk(c.clone());
            cb_ret.index_chunk(c.clone());
        }

        let fast_res = fast_ret.search("fetch cache", 3);
        let cb_res = cb_ret.search("fetch cache", 3);

        assert!(fast_res.returned > 0);
        assert!(cb_res.returned > 0);
        assert_eq!(fast_res.total_indexed, cb_res.total_indexed);
    }

    #[test]
    fn test_cross_type() {
        if should_skip() { return; }
        let cb = CodeBertEncoder::load().expect("load");

        let c1 = make_chunk("get", "fn get(id: u64) -> Option<Item> { db.query(\"SELECT * FROM t WHERE id = ?\", id) }");
        let c2 = make_chunk("Repo_get", "impl Repo { fn get(&self, id: u64) -> Option<Item> { self.db.query(\"SELECT * FROM t WHERE id = ?\", id) } }");

        let sim = cb.encode(&c1).cosine_similarity(&cb.encode(&c2));
        println!("cross-type sim: {:.4}", sim);
        assert!(sim > 0.7);
    }
}