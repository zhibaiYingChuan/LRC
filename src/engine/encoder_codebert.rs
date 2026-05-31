// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含外部模型适配接口，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
//
// 外部编码器适配
// 集成第三方语义模型，需启用 `ml` feature。

use super::encoder::{CodeEncoder, EmbeddingVector, FastEncoder};
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

        let api =
            hf_hub::api::sync::Api::new().map_err(|e| format!("hf-hub init: {e}"))?;
        let repo = api.model("microsoft/codebert-base".to_string());

        let config_path = repo
            .get("config.json")
            .map_err(|e| format!("config.json: {e}"))?;
        let tokenizer_path = repo
            .get("tokenizer.json")
            .map_err(|e| format!("tokenizer.json: {e}"))?;
        let model_path = repo
            .get("model.safetensors")
            .map_err(|e| format!("model.safetensors: {e}"))?;

        let config_file = File::open(&config_path)
            .map_err(|e| format!("open config: {e}"))?;
        let config: candle_transformers::models::bert::Config =
            serde_json::from_reader(BufReader::new(config_file))
                .map_err(|e| format!("parse config: {e}"))?;

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("tokenizer: {e}"))?;

        let tensors = candle_core::safetensors::load(&model_path, &device)
            .map_err(|e| format!("safetensors: {e}"))?;
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

    fn should_skip() -> bool {
        std::env::var("SKIP_ML_TESTS").is_ok()
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