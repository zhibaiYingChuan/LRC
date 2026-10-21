// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
// 不参与检索、不参与记忆存储、不参与索引。
// 只做一件事：把用户的模糊自然语言，翻译成精确的代码关键词。
// 然后由 Fast Match 去执行真正的检索。

use crate::errors::{LrcError, LrcResult};
use serde::{Deserialize, Serialize};

/// LLM API 配置
#[derive(Debug, Clone, Default)]
pub enum LlmApiConfig {
    /// 未配置 LLM API
    #[default]
    None,
    /// OpenAI 兼容 API
    OpenAI {
        api_key: String,
        model: String,
        /// 自定义 API 端点（默认 https://api.openai.com/v1）
        endpoint: String,
    },
    /// Ollama 本地模型
    Ollama { host: String, model: String },
}

impl LlmApiConfig {
    /// 从字符串解析 LLM API 配置
    ///
    /// 支持两种分隔符格式：
    /// - `||` 分隔符（桌面端优先使用，避免 API Key 中包含 `:` 时解析错误）：
    ///   - `openai||sk-xxx||gpt-4o-mini||https://custom.api.com/v1`
    ///   - `ollama||localhost||llama3`
    /// - `:` 分隔符（向后兼容旧格式）：
    ///   - `openai:sk-xxx:gpt-4o-mini`
    ///   - `openai:sk-xxx:gpt-4o-mini:https://custom.api.com/v1`
    ///   - `ollama:localhost:llama3`
    pub fn parse(input: &str) -> LrcResult<Self> {
        // 自动检测分隔符：优先使用 || 分隔符（更安全），回退到 : 分隔符（向后兼容）
        let parts: Vec<&str> = if input.contains("||") {
            input.splitn(4, "||").collect()
        } else {
            input.splitn(4, ':').collect()
        };

        match parts[0] {
            "openai" => {
                let api_key = parts
                    .get(1)
                    .filter(|s| !s.is_empty())
                    .ok_or(LrcError::invalid_input(
                        "openai 模式需要 API Key: openai:sk-xxx:model",
                    ))?
                    .to_string();
                let model = parts
                    .get(2)
                    .filter(|s| !s.is_empty())
                    .ok_or(LrcError::invalid_input(
                        "openai 模式需要模型名: openai:sk-xxx:gpt-4o-mini",
                    ))?
                    .to_string();
                let endpoint = parts
                    .get(3)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

                // SSRF 防护：配置入口统一字面量校验（拒绝 metadata/链路本地/未指定等）
                if let Err(e) = crate::url_safety::validate_http_url(&endpoint) {
                    return Err(LrcError::invalid_input(format!(
                        "openai endpoint 地址校验失败: {}",
                        e
                    )));
                }

                Ok(LlmApiConfig::OpenAI {
                    api_key,
                    model,
                    endpoint,
                })
            }
            "ollama" => {
                // v0.9.14 修复：桌面端旧版本按 `ollama||model||host` 顺序拼接，
                // 与本模块约定的 `ollama||host||model` 相反，导致模型名被当成主机地址。
                // 这里做一次窄归一化：仅当第二段是带协议的地址、第一段不是时交换，
                // 不影响任何既有正确配置。
                let raw_host = parts.get(1).map(|s| s.trim()).unwrap_or("");
                let raw_model = parts.get(2).map(|s| s.trim()).unwrap_or("");
                let swapped = is_http_url(raw_model) && !is_http_url(raw_host);
                let (host, model) = if swapped {
                    (raw_model, raw_host)
                } else {
                    (raw_host, raw_model)
                };
                if host.is_empty() {
                    return Err(LrcError::invalid_input(
                        "ollama 模式需要主机地址: ollama||localhost:11434||llama3",
                    ));
                }
                if model.is_empty() {
                    return Err(LrcError::invalid_input(
                        "ollama 模式需要模型名: ollama||localhost:11434||llama3",
                    ));
                }
                let host = host.to_string();
                let model = model.to_string();

                // SSRF 防护：Ollama host 构造 http://host 统一校验
                // host 可能为 "localhost"、"192.168.1.109"、"192.168.1.109:11434"
                let target = if host.starts_with("http://") || host.starts_with("https://") {
                    host.clone()
                } else {
                    format!("http://{}", host)
                };
                if let Err(e) = crate::url_safety::validate_http_url(&target) {
                    return Err(LrcError::invalid_input(format!(
                        "ollama host 地址校验失败: {}",
                        e
                    )));
                }

                Ok(LlmApiConfig::Ollama { host, model })
            }
            other => Err(LrcError::invalid_input(format!(
                "不支持的 LLM API 类型: {}。支持的类型: openai, ollama",
                other
            ))),
        }
    }

    /// 是否已配置 LLM API
    pub fn is_configured(&self) -> bool {
        !matches!(self, LlmApiConfig::None)
    }

    /// v0.5.18 新增：调用 LLM embedding API，返回文本的语义向量
    ///
    /// 用于结晶流水线在合成时计算高质量的语义相似度。
    /// 返回高维向量（OpenAI 通常 1536 维，Ollama 取决于模型）。
    ///
    /// 失败时返回 Err，调用方应降级到洛书向量。
    pub async fn embed_text(&self, text: &str) -> LrcResult<Vec<f32>> {
        match self {
            LlmApiConfig::OpenAI {
                api_key,
                model,
                endpoint,
            } => embed_openai(endpoint, api_key, model, text).await,
            LlmApiConfig::Ollama { host, model } => embed_ollama(host, model, text).await,
            LlmApiConfig::None => Err(LrcError::config("LLM 未配置，无法调用 embedding API")),
        }
    }

    /// v0.5.18 新增：批量 embedding（减少 API 调用次数）
    ///
    /// OpenAI 支持 input 数组批量请求，Ollama 逐条调用。
    /// 返回顺序与输入一致的向量列表。
    pub async fn embed_texts(&self, texts: &[&str]) -> LrcResult<Vec<Vec<f32>>> {
        match self {
            LlmApiConfig::OpenAI {
                api_key,
                model,
                endpoint,
            } => embed_openai_batch(endpoint, api_key, model, texts).await,
            LlmApiConfig::Ollama { host, model } => {
                // Ollama 不支持批量，逐条调用
                let mut results = Vec::with_capacity(texts.len());
                for text in texts {
                    results.push(embed_ollama(host, model, text).await?);
                }
                Ok(results)
            }
            LlmApiConfig::None => Err(LrcError::config("LLM 未配置，无法调用 embedding API")),
        }
    }

    /// v0.5.18 新增：调用 LLM chat API 将多条同类记忆合成为一条总结
    ///
    /// 用于结晶流水线在聚类通过信息增量阈值后，生成合成记忆的内容。
    /// 返回合成后的文本（已去除解释性前缀）。
    ///
    /// 失败时返回 Err，调用方应跳过该簇的合成。
    pub async fn summarize_memories(&self, memories: &[String]) -> LrcResult<String> {
        if memories.is_empty() {
            return Err(LrcError::invalid_input("记忆列表为空，无法合成"));
        }
        if memories.len() == 1 {
            return Ok(memories[0].clone());
        }

        let prompt = build_synthesis_prompt(memories);

        match self {
            LlmApiConfig::OpenAI {
                api_key,
                model,
                endpoint,
            } => summarize_openai(endpoint, api_key, model, &prompt).await,
            LlmApiConfig::Ollama { host, model } => summarize_ollama(host, model, &prompt).await,
            LlmApiConfig::None => Err(LrcError::config("LLM 未配置，无法调用合成 API")),
        }
    }
}

// ==================== v0.5.18 LLM 合成函数 ====================

/// 通过 OpenAI 兼容 API 合成记忆
///
/// 复用 chat/completions 接口，使用更大的 max_tokens（512）以容纳总结内容。
async fn summarize_openai(
    endpoint: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
) -> LrcResult<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| LrcError::network(format!("创建 HTTP 客户端失败: {}", e)))?;

    let request = OpenAiChatRequest {
        model: model.to_string(),
        messages: vec![OpenAiMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
        temperature: 0.1, // 低温度确保总结稳定
        max_tokens: 512,
    };

    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| LrcError::network(format!("合成请求失败: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        eprintln!(
            "[LRC·合成] OpenAI API 返回错误状态: {} (响应: {:.200})",
            status, body
        );
        return Err(LrcError::network(format!("合成 API 返回错误: {}", status)));
    }

    let raw_body = response
        .text()
        .await
        .map_err(|e| LrcError::network(format!("读取合成响应体失败: {}", e)))?;

    let body: OpenAiChatResponse = serde_json::from_str(&raw_body).map_err(|e| {
        eprintln!(
            "[LRC·合成] OpenAI 响应 JSON 解析失败: {} (原始响应: {:.200})",
            e, raw_body
        );
        LrcError::parse(format!("解析合成响应失败: {}", e))
    })?;

    if body.choices.is_empty() {
        eprintln!("[LRC·合成] OpenAI 返回空 choices 数组");
        return Err(LrcError::network("合成 API 返回空 choices"));
    }

    let content = body
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| LrcError::network("合成 API 返回空内容"))?;

    Ok(content)
}

/// 通过 Ollama 合成记忆
async fn summarize_ollama(host: &str, model: &str, prompt: &str) -> LrcResult<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| LrcError::network(format!("创建 HTTP 客户端失败: {}", e)))?;

    let request = OllamaChatRequest {
        model: model.to_string(),
        messages: vec![OllamaMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
        stream: false,
    };

    let url = format!("{}/api/chat", ollama_base_url(host));

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| LrcError::network(format!("Ollama 合成请求失败: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        eprintln!(
            "[LRC·合成] Ollama API 返回错误状态: {} (响应: {:.200})",
            status, body
        );
        return Err(LrcError::network(format!(
            "Ollama 合成 API 返回错误: {}",
            status
        )));
    }

    let raw_body = response
        .text()
        .await
        .map_err(|e| LrcError::network(format!("读取 Ollama 合成响应体失败: {}", e)))?;

    let body: OllamaChatResponse = serde_json::from_str(&raw_body).map_err(|e| {
        eprintln!(
            "[LRC·合成] Ollama 响应 JSON 解析失败: {} ({})",
            e,
            response_log_summary(&raw_body)
        );
        LrcError::parse(format!("解析 Ollama 合成响应失败: {}", e))
    })?;

    let content = body.message.content.trim().to_string();

    if content.is_empty() {
        return Err(LrcError::network("Ollama 合成返回空内容"));
    }

    Ok(content)
}

// ==================== OpenAI API 类型 ====================

#[derive(Debug, Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Debug, Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    content: String,
}

// ==================== Ollama API 类型 ====================

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

// ==================== 翻译 Prompt ====================

/// 代码搜索翻译 Prompt — 将自然语言翻译为代码符号
const CODE_TRANSLATION_PROMPT: &str = "你是一个代码搜索助手。将用户的自然语言查询翻译成可能出现在代码中的函数名、变量名、结构体名或关键词。只返回逗号分隔的关键词列表，不要解释。";

/// 记忆检索翻译 Prompt — 将问题翻译为答案可能包含的关键词
/// 用于桥接 LongMemEval 等场景中"问题词与答案词不重叠"的语义鸿沟
const MEMORY_TRANSLATION_PROMPT: &str = "你是一个记忆检索助手。用户会提出关于他们过往对话的问题，你需要将问题翻译为答案可能包含的关键词。\n\
例如：\n\
- 问「What degree did I graduate with?」→ Business, Administration, major, bachelor, degree, graduation, college, university\n\
- 问「Where did I travel last year?」→ Japan, Tokyo, travel, trip, vacation, visited, destination\n\
- 问「What is my dog's name?」→ dog, pet, name, Max, Buddy, puppy, animal\n\
- 问「What company do I work for?」→ Google, Microsoft, company, employer, job, work, career\n\
只返回逗号分隔的关键词列表，不要解释。关键词应包含可能的答案内容（如具体名称、地点、专业名、公司名等）以及相关概念词。";

/// v0.5.18 新增：记忆合成 Prompt — 将多条同类记忆融合为一条高密度总结
const MEMORY_SYNTHESIS_PROMPT: &str = "你是一个记忆合成助手。下面给出多条相关的记忆条目，请将它们融合成一条简洁、高密度的总结记忆。\n\
要求：\n\
1. 保留所有关键信息（事实、偏好、决策、技术细节）\n\
2. 去除重复内容，合并同类项\n\
3. 用清晰的陈述句表达，不要使用列表或 bullet points\n\
4. 不要添加任何解释性前缀（如「总结：」「合成记忆：」），直接输出合成内容\n\
5. 长度控制在 200 字以内\n\
6. 如果记忆之间存在矛盾，保留最新信息并标注「（已更新）」";

/// 构建翻译 Prompt（通用）
fn build_prompt(system_prompt: &str, query: &str) -> String {
    format!("{}\n\n用户查询：{}", system_prompt, query)
}

/// v0.5.18 新增：构建记忆合成 Prompt
fn build_synthesis_prompt(memories: &[String]) -> String {
    let formatted: Vec<String> = memories
        .iter()
        .enumerate()
        .map(|(i, m)| format!("记忆 #{}: {}", i + 1, m))
        .collect();
    format!(
        "{}\n\n以下是需要合成的 {} 条记忆：\n\n{}",
        MEMORY_SYNTHESIS_PROMPT,
        memories.len(),
        formatted.join("\n")
    )
}

/// 将 LLM 响应解析为关键词列表
fn parse_keywords(response: &str) -> Vec<String> {
    response
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn response_log_summary(body: &str) -> String {
    format!("响应体已脱敏（{} 字节）", body.len())
}

#[cfg(test)]
mod response_log_tests {
    use super::response_log_summary;

    #[test]
    fn response_log_summary_does_not_include_response_body() {
        let summary = response_log_summary("secret-api-error");
        assert!(!summary.contains("secret-api-error"));
        assert!(summary.contains("脱敏"));
    }
}

// ==================== 核心翻译函数 ====================

/// 将自然语言查询翻译为代码关键词
///
/// 如果配置了 LLM API，则调用 LLM 进行翻译；
/// 如果未配置，则返回原始查询作为关键词。
///
/// 翻译失败时自动回退到原始查询，确保搜索功能不受影响。
pub async fn translate_query(config: &LlmApiConfig, query: &str) -> Vec<String> {
    match config {
        LlmApiConfig::None => {
            vec![query.to_string()]
        }
        LlmApiConfig::OpenAI {
            api_key,
            model,
            endpoint,
        } => match translate_openai(endpoint, api_key, model, query, CODE_TRANSLATION_PROMPT).await
        {
            Ok(keywords) if !keywords.is_empty() => keywords,
            _ => {
                eprintln!("[LRC] LLM 翻译失败，回退到原始查询");
                vec![query.to_string()]
            }
        },
        LlmApiConfig::Ollama { host, model } => {
            match translate_ollama(host, model, query, CODE_TRANSLATION_PROMPT).await {
                Ok(keywords) if !keywords.is_empty() => keywords,
                _ => {
                    eprintln!("[LRC] LLM 翻译失败，回退到原始查询");
                    vec![query.to_string()]
                }
            }
        }
    }
}

/// 将自然语言问题翻译为记忆检索关键词（用于 LongMemEval 等语义检索场景）
///
/// 与 translate_query（代码搜索）不同，此函数使用记忆场景 Prompt，
/// 引导 LLM 生成答案可能包含的关键词，桥接"问 X 答 Y"的语义鸿沟。
/// 例如："What degree did I graduate with?" →
///   "Business, Administration, major, degree, graduation, college"
///
/// 翻译失败时自动回退到原始查询，确保检索不受影响。
pub async fn translate_memory_query(config: &LlmApiConfig, query: &str) -> Vec<String> {
    match config {
        LlmApiConfig::None => {
            vec![query.to_string()]
        }
        LlmApiConfig::OpenAI {
            api_key,
            model,
            endpoint,
        } => {
            match translate_openai(endpoint, api_key, model, query, MEMORY_TRANSLATION_PROMPT).await
            {
                Ok(keywords) if !keywords.is_empty() => keywords,
                _ => {
                    eprintln!("[LRC] 记忆翻译失败，回退到原始查询");
                    vec![query.to_string()]
                }
            }
        }
        LlmApiConfig::Ollama { host, model } => {
            match translate_ollama(host, model, query, MEMORY_TRANSLATION_PROMPT).await {
                Ok(keywords) if !keywords.is_empty() => keywords,
                _ => {
                    eprintln!("[LRC] 记忆翻译失败，回退到原始查询");
                    vec![query.to_string()]
                }
            }
        }
    }
}

/// 把「LLM 记忆查询翻译 + 关键词并入检索查询」收敛为**单一真源**。
///
/// ## 策略（与既有 MCP 调用点逐字一致）
/// - LLM 未配置 / 翻译失败 / 结果为空 / 结果与原文相同 ⇒ **原样返回 `query`**（零行为变更）；
/// - 否则返回 `"{关键词…} {原查询}"`（关键词在前，用于桥接"问 X 答 Y"的词汇鸿沟）。
///
/// ## 为什么必须抽成一处
/// 该策略原先在 `server.rs` 的**两处** MCP handler 里各写一遍，而
/// REST `POST /v1/memories/enrich` —— **AML 与桌面端实际使用的检索入口** ——
/// **完全没有**这一步 ⇒ 同一个引擎、同一条查询，经 MCP 与经 REST 的检索质量不一致。
/// 收敛为单一真源后三条入口共用同一策略；并以 `LlmApiConfig::None` 做**确定性**单测
/// （未配置时必须逐字返回原查询）。
///
/// ## 时延与安全
/// 底层 `translate_openai` / `translate_ollama` 各自带 5s / 10s **客户端超时**，
/// 故本函数不会无限等待；任何失败都回退原查询，**绝不阻断检索**。
/// 调用方须在**取存储锁之前**调用（网络等待不得发生在持锁期）。
pub async fn enrich_query_for_retrieval(config: &LlmApiConfig, query: &str) -> String {
    // ★ `LRC_LLM_TRANSLATE=0` ⇒ **只关"检索侧查询翻译"，保留合成的 LLM 能力**。
    //
    //   动机（2026-10-21 线上实测）：一旦配置 `LRC_LLM_API`，REST `enrich` 与 MCP
    //   两条入口都会先做一次查询翻译（`translate_memory_query`，客户端 5s 超时）。
    //   16 并发 Search 实测 **中位 1.15s → 14.0s、最坏 28.3s**（每条 1.2~1.5s，
    //   上游串行返回）—— 而 AML 申报的 Search 并发正是 16，这个延迟有撞平台
    //   超时的风险。而**结晶（合成）走的是另一条链路，不受本开关影响**。
    //
    //   默认**开**：不设该环境变量时行为与改造前逐字一致。
    if !llm_translate_enabled() {
        return query.to_string();
    }
    if !config.is_configured() {
        return query.to_string();
    }
    let keywords = translate_memory_query(config, query).await;
    let translated = keywords.join(" ");
    if translated.is_empty() || translated.trim() == query {
        query.to_string()
    } else {
        format!("{} {}", translated, query)
    }
}

/// 查询翻译开关的**纯函数**部分（不读环境变量，便于确定性的单测）。
fn llm_translate_enabled_from(raw: Option<&str>) -> bool {
    match raw {
        None => true,
        Some(v) => {
            let s = v.trim().to_ascii_lowercase();
            !(s == "0" || s == "false" || s == "off" || s == "no")
        }
    }
}

/// 是否启用"检索侧 LLM 查询翻译"（`LRC_LLM_TRANSLATE=0/false/off/no` 关闭）。
fn llm_translate_enabled() -> bool {
    llm_translate_enabled_from(std::env::var("LRC_LLM_TRANSLATE").ok().as_deref())
}

#[cfg(test)]
mod translate_gate_tests {
    use super::llm_translate_enabled_from;

    #[test]
    fn unset_means_enabled() {
        assert!(llm_translate_enabled_from(None));
    }

    #[test]
    fn explicit_off_values_disable() {
        for v in ["0", "false", "FALSE", " off ", "no"] {
            assert!(!llm_translate_enabled_from(Some(v)), "value={v:?}");
        }
    }

    #[test]
    fn other_values_keep_enabled() {
        for v in ["1", "true", "on", "yes", ""] {
            assert!(llm_translate_enabled_from(Some(v)), "value={v:?}");
        }
    }
}

/// 通过 OpenAI 兼容 API 翻译查询
///
/// v0.5.4 P2-11 修复：LLM 响应解析安全化
/// - 所有解析失败路径都有详细日志
/// - 空 choices 数组时记录警告并返回错误
/// - 响应解析失败时记录原始响应（截断前 200 字符）便于调试
async fn translate_openai(
    endpoint: &str,
    api_key: &str,
    model: &str,
    query: &str,
    system_prompt: &str,
) -> LrcResult<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| LrcError::network(format!("创建 HTTP 客户端失败: {}", e)))?;

    let request = OpenAiChatRequest {
        model: model.to_string(),
        messages: vec![OpenAiMessage {
            role: "user".to_string(),
            content: build_prompt(system_prompt, query),
        }],
        temperature: 0.0,
        max_tokens: 100,
    };

    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));

    // ★ v0.9.12（2026-10-10）：**失败必须可见**（传输层错误此前完全不打日志）。
    //
    // 实测后果：端点不可达/限流时，查询扩展被**静默降级为空操作** ——
    // 18/18 实例日志只剩调用方那句"记忆翻译失败，回退到原始查询"，**连状态码都没有**；
    // 而配与不配 LLM，探针台 R_sim **逐位相同（0.7514）**，外部完全看不出该杠杆从未生效。
    // 而这恰是命中"词汇鸿沟"（94.3% 召回缺口）的唯一手段 ⇒ 失败必须可见。
    //
    // ★ 同处做过的**有界重试（429/5xx）已实测无效果并回退**：本机 LLM 账户是
    //   **持续性**限流（加重试后 llm 臂仍 11/18 失败，与加重试前逐位相同），退避重试
    //   只能救"瞬时抖动"、救不了持续限流；而其代价是失败路径上多 ~400ms~5.4s 延迟。
    //   ⇒ 按"实测无效者不合入"回退。**若线上真出现瞬时 429/5xx，再按需加回并配
    //   mock（先 429 后 200）做一次可证伪验证。**
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| {
            eprintln!(
                "[LRC·LLM] 请求失败（传输层）: {} | endpoint={} | model={}",
                e, url, model
            );
            LrcError::network(format!("LLM 请求失败: {}", e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        eprintln!(
            "[LRC·LLM] OpenAI API 返回错误状态: {} (响应: {:.200})",
            status, body
        );
        return Err(LrcError::network(format!("LLM API 返回错误: {}", status)));
    }

    // v0.5.4 P2-11 修复：先获取原始文本，解析失败时可以记录用于调试
    let raw_body = response.text().await.map_err(|e| {
        // ★ v0.9.12（2026-10-10）：**读响应体失败原也无日志** ⇒ 一并补齐。
        // 只打错误与端点，**不打响应体**（沿用本文件既有的"响应体脱敏"策略）。
        eprintln!("[LRC·LLM] 读取响应体失败: {} | endpoint={}", e, url);
        LrcError::network(format!("读取 LLM 响应体失败: {}", e))
    })?;

    let body: OpenAiChatResponse = serde_json::from_str(&raw_body).map_err(|e| {
        eprintln!(
            "[LRC·LLM] OpenAI 响应 JSON 解析失败: {} (原始响应: {:.200})",
            e, raw_body
        );
        LrcError::parse(format!("解析 LLM 响应失败: {}", e))
    })?;

    // v0.5.4 P2-11 修复：防御性检查空 choices 数组
    if body.choices.is_empty() {
        eprintln!("[LRC·LLM] OpenAI 返回空 choices 数组，可能模型拒绝回答或发生内部错误");
        return Err(LrcError::network("LLM 返回空 choices 数组"));
    }

    let content = body
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default();

    // v0.5.4 P2-11 修复：防御性检查空内容
    if content.trim().is_empty() {
        eprintln!("[LRC·LLM] OpenAI 返回空内容，使用原始查询回退");
        return Err(LrcError::network("LLM 返回空内容"));
    }

    // ★ v0.9.12（2026-10-10）：**解析为空也必须可见**。
    // 响应非空却切不出关键词（内容只有逗号/空白）时，`parse_keywords` 返回空表，
    // 调用方只打一句兜底文案 ⇒ 整条"查询扩展"链路会**静默空转**。
    // 此处只打响应**字节数**，不打内容（沿用本文件"响应体脱敏"策略）。
    let keywords = parse_keywords(&content);
    if keywords.is_empty() {
        eprintln!(
            "[LRC·LLM] 关键词解析为空（响应 {} 字节）| endpoint={} | model={}",
            content.len(),
            url,
            model
        );
    }
    Ok(keywords)
}

/// 通过 Ollama 本地模型翻译查询
///
/// v0.5.4 P2-11 修复：LLM 响应解析安全化
/// - 所有解析失败路径都有详细日志
/// - 空内容时记录警告并返回错误
/// - 响应解析失败时记录原始响应（截断前 200 字符）便于调试
async fn translate_ollama(
    host: &str,
    model: &str,
    query: &str,
    system_prompt: &str,
) -> LrcResult<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| LrcError::network(format!("创建 HTTP 客户端失败: {}", e)))?;

    let request = OllamaChatRequest {
        model: model.to_string(),
        messages: vec![OllamaMessage {
            role: "user".to_string(),
            content: build_prompt(system_prompt, query),
        }],
        stream: false,
    };

    let url = format!("{}/api/chat", ollama_base_url(host));

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| LrcError::network(format!("Ollama 请求失败: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        eprintln!(
            "[LRC·LLM] Ollama API 返回错误状态: {} (响应: {:.200})",
            status, body
        );
        return Err(LrcError::network(format!(
            "Ollama API 返回错误: {}",
            status
        )));
    }

    // v0.5.4 P2-11 修复：先获取原始文本，解析失败时可以记录用于调试
    let raw_body = response
        .text()
        .await
        .map_err(|e| LrcError::network(format!("读取 Ollama 响应体失败: {}", e)))?;

    let body: OllamaChatResponse = serde_json::from_str(&raw_body).map_err(|e| {
        eprintln!(
            "[LRC·LLM] Ollama 响应 JSON 解析失败: {} ({})",
            e,
            response_log_summary(&raw_body)
        );
        LrcError::parse(format!("解析 Ollama 响应失败: {}", e))
    })?;

    // v0.5.4 P2-11 修复：防御性检查空内容
    if body.message.content.trim().is_empty() {
        eprintln!("[LRC·LLM] Ollama 返回空内容，使用原始查询回退");
        return Err(LrcError::network("Ollama 返回空内容"));
    }

    Ok(parse_keywords(&body.message.content))
}

// ==================== v0.5.18 Embedding API ====================

/// OpenAI embedding 请求体
#[derive(Debug, Serialize)]
struct OpenAiEmbeddingRequest {
    model: String,
    input: String,
}

/// OpenAI embedding 批量请求体
#[derive(Debug, Serialize)]
struct OpenAiEmbeddingBatchRequest {
    model: String,
    input: Vec<String>,
}

/// OpenAI embedding 响应体
#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
}

/// Ollama embedding 请求体
#[derive(Debug, Serialize)]
struct OllamaEmbeddingRequest {
    model: String,
    prompt: String,
}

/// Ollama embedding 响应体
#[derive(Debug, Deserialize)]
struct OllamaEmbeddingResponse {
    embedding: Vec<f32>,
}

/// 通过 OpenAI 兼容 API 获取单条文本的 embedding
async fn embed_openai(
    endpoint: &str,
    api_key: &str,
    model: &str,
    text: &str,
) -> LrcResult<Vec<f32>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| LrcError::network(format!("创建 HTTP 客户端失败: {}", e)))?;

    let request = OpenAiEmbeddingRequest {
        model: model.to_string(),
        input: text.to_string(),
    };

    let url = format!("{}/embeddings", endpoint.trim_end_matches('/'));

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| LrcError::network(format!("Embedding 请求失败: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        eprintln!(
            "[LRC·Embedding] OpenAI API 返回错误状态: {} ({})",
            status,
            response_log_summary(&body)
        );
        return Err(LrcError::network(format!(
            "Embedding API 返回错误: {}",
            status
        )));
    }

    let raw_body = response
        .text()
        .await
        .map_err(|e| LrcError::network(format!("读取 Embedding 响应体失败: {}", e)))?;

    let body: OpenAiEmbeddingResponse = serde_json::from_str(&raw_body).map_err(|e| {
        eprintln!(
            "[LRC·Embedding] OpenAI 响应 JSON 解析失败: {} ({})",
            e,
            response_log_summary(&raw_body)
        );
        LrcError::parse(format!("解析 Embedding 响应失败: {}", e))
    })?;

    body.data
        .into_iter()
        .next()
        .map(|d| d.embedding)
        .ok_or_else(|| LrcError::network("Embedding 响应为空"))
}

/// 通过 OpenAI 兼容 API 批量获取 embedding
///
/// OpenAI /embeddings 接口支持 input 数组，一次请求获取多条文本的向量。
/// 单次最多 2048 条（API 限制），超出自动分批。
async fn embed_openai_batch(
    endpoint: &str,
    api_key: &str,
    model: &str,
    texts: &[&str],
) -> LrcResult<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| LrcError::network(format!("创建 HTTP 客户端失败: {}", e)))?;

    let url = format!("{}/embeddings", endpoint.trim_end_matches('/'));
    let mut all_embeddings = Vec::with_capacity(texts.len());

    // 分批处理，每批最多 100 条（避免请求体过大）
    for chunk in texts.chunks(100) {
        let request = OpenAiEmbeddingBatchRequest {
            model: model.to_string(),
            input: chunk.iter().map(|s| s.to_string()).collect(),
        };

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| LrcError::network(format!("批量 Embedding 请求失败: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            eprintln!(
                "[LRC·Embedding] 批量 API 返回错误状态: {} (响应: {:.200})",
                status, body
            );
            return Err(LrcError::network(format!(
                "批量 Embedding API 返回错误: {}",
                status
            )));
        }

        let raw_body = response
            .text()
            .await
            .map_err(|e| LrcError::network(format!("读取批量 Embedding 响应体失败: {}", e)))?;

        let body: OpenAiEmbeddingResponse = serde_json::from_str(&raw_body).map_err(|e| {
            eprintln!(
                "[LRC·Embedding] 批量响应 JSON 解析失败: {} ({})",
                e,
                response_log_summary(&raw_body)
            );
            LrcError::parse(format!("解析批量 Embedding 响应失败: {}", e))
        })?;

        for data in body.data {
            all_embeddings.push(data.embedding);
        }
    }

    Ok(all_embeddings)
}

/// 是否为带协议的 HTTP(S) 地址
fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

/// 归一化 Ollama 基础地址（协议 + 主机 + 端口）
///
/// v0.9.14 修复：此前三处调用点直接拼接 `http://{host}:11434`，
/// 当 host 已带端口（如 `127.0.0.1:11434`）或已带协议（如 `http://localhost:11434`）时，
/// 会生成 `http://127.0.0.1:11434:11434/...` 这类非法 URL，
/// reqwest 直接返回 `builder error`（本地日志累计 1831 次，结晶流水线退化为洛书合成）。
///
/// 规则：缺少协议补 `http://`；authority 未显式带端口时补 `:11434`；去掉尾部斜杠。
fn ollama_base_url(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('/');
    let base = if is_http_url(trimmed) {
        trimmed.to_string()
    } else {
        format!("http://{}", trimmed)
    };

    // authority = 协议之后、第一个斜杠之前的部分
    let authority = base
        .split("://")
        .nth(1)
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default();
    // IPv6 字面量（[::1]）的方括号内冒号不算端口分隔符
    let host_part = match authority.rfind(']') {
        Some(idx) => &authority[idx + 1..],
        None => authority,
    };

    if host_part.contains(':') {
        base
    } else {
        format!("{}:11434", base)
    }
}

/// 通过 Ollama 获取单条文本的 embedding
async fn embed_ollama(host: &str, model: &str, text: &str) -> LrcResult<Vec<f32>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| LrcError::network(format!("创建 HTTP 客户端失败: {}", e)))?;

    let request = OllamaEmbeddingRequest {
        model: model.to_string(),
        prompt: text.to_string(),
    };

    let url = format!("{}/api/embeddings", ollama_base_url(host));

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| LrcError::network(format!("Ollama Embedding 请求失败: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        eprintln!(
            "[LRC·Embedding] Ollama API 返回错误状态: {} (响应: {:.200})",
            status, body
        );
        return Err(LrcError::network(format!(
            "Ollama Embedding API 返回错误: {}",
            status
        )));
    }

    let raw_body = response
        .text()
        .await
        .map_err(|e| LrcError::network(format!("读取 Ollama Embedding 响应体失败: {}", e)))?;

    let body: OllamaEmbeddingResponse = serde_json::from_str(&raw_body).map_err(|e| {
        eprintln!(
            "[LRC·Embedding] Ollama 响应 JSON 解析失败: {} ({})",
            e,
            response_log_summary(&raw_body)
        );
        LrcError::parse(format!("解析 Ollama Embedding 响应失败: {}", e))
    })?;

    Ok(body.embedding)
}

/// 计算两个高维向量的余弦相似度
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a > 0.0 && norm_b > 0.0 {
        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

// ==================== 测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_keywords() {
        let result = parse_keywords("authenticate_user, login, handle_login, auth");
        assert_eq!(
            result,
            vec!["authenticate_user", "login", "handle_login", "auth"]
        );
    }

    #[test]
    fn test_parse_keywords_with_spaces() {
        let result = parse_keywords("  authenticate_user ,  login , handle_login  ");
        assert_eq!(result, vec!["authenticate_user", "login", "handle_login"]);
    }

    #[test]
    fn test_parse_keywords_empty() {
        let result = parse_keywords("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_config_openai() {
        let config = LlmApiConfig::parse("openai:sk-test123:gpt-4o-mini").unwrap();
        match config {
            LlmApiConfig::OpenAI {
                api_key,
                model,
                endpoint,
            } => {
                assert_eq!(api_key, "sk-test123");
                assert_eq!(model, "gpt-4o-mini");
                assert_eq!(endpoint, "https://api.openai.com/v1");
            }
            _ => panic!("应为 OpenAI 配置"),
        }
    }

    #[test]
    fn test_parse_config_openai_custom_endpoint() {
        let config = LlmApiConfig::parse("openai:sk-test:gpt-4:https://custom.api.com/v1").unwrap();
        match config {
            LlmApiConfig::OpenAI { endpoint, .. } => {
                assert_eq!(endpoint, "https://custom.api.com/v1");
            }
            _ => panic!("应为 OpenAI 配置"),
        }
    }

    #[test]
    fn test_parse_config_openai_double_pipe() {
        // 测试 || 分隔符（桌面端 to_llm_api_string 使用此格式）
        let config =
            LlmApiConfig::parse("openai||sk-test123||gpt-4o-mini||https://api.openai.com/v1")
                .unwrap();
        match config {
            LlmApiConfig::OpenAI {
                api_key,
                model,
                endpoint,
            } => {
                assert_eq!(api_key, "sk-test123");
                assert_eq!(model, "gpt-4o-mini");
                assert_eq!(endpoint, "https://api.openai.com/v1");
            }
            _ => panic!("应为 OpenAI 配置"),
        }
    }

    #[test]
    fn test_parse_config_openai_double_pipe_custom_endpoint() {
        // 测试 || 分隔符 + 自定义端点（通义千问等兼容 API）
        let config = LlmApiConfig::parse(
            "openai||sk-test||qwen-plus||https://dashscope.aliyuncs.com/compatible-mode/v1",
        )
        .unwrap();
        match config {
            LlmApiConfig::OpenAI {
                api_key,
                model,
                endpoint,
            } => {
                assert_eq!(api_key, "sk-test");
                assert_eq!(model, "qwen-plus");
                assert_eq!(
                    endpoint,
                    "https://dashscope.aliyuncs.com/compatible-mode/v1"
                );
            }
            _ => panic!("应为 OpenAI 配置"),
        }
    }

    #[test]
    fn test_parse_config_ollama_double_pipe() {
        // 测试 || 分隔符的 Ollama 格式
        let config = LlmApiConfig::parse("ollama||localhost||llama3").unwrap();
        match config {
            LlmApiConfig::Ollama { host, model } => {
                assert_eq!(host, "localhost");
                assert_eq!(model, "llama3");
            }
            _ => panic!("应为 Ollama 配置"),
        }
    }

    #[test]
    fn test_parse_config_ollama() {
        let config = LlmApiConfig::parse("ollama:localhost:llama3").unwrap();
        match config {
            LlmApiConfig::Ollama { host, model } => {
                assert_eq!(host, "localhost");
                assert_eq!(model, "llama3");
            }
            _ => panic!("应为 Ollama 配置"),
        }
    }

    #[test]
    fn test_parse_config_ollama_accepts_legacy_inverted_order() {
        // 桌面端旧版本按 `ollama||model||host` 拼接，解析侧必须归一化为 host 在前
        let config = LlmApiConfig::parse("ollama||llama3||http://localhost:11434").unwrap();
        match config {
            LlmApiConfig::Ollama { host, model } => {
                assert_eq!(host, "http://localhost:11434");
                assert_eq!(model, "llama3");
            }
            _ => panic!("应为 Ollama 配置"),
        }
    }

    #[test]
    fn test_parse_config_ollama_host_first_unchanged() {
        let config = LlmApiConfig::parse("ollama||127.0.0.1:11434||bge-m3").unwrap();
        match config {
            LlmApiConfig::Ollama { host, model } => {
                assert_eq!(host, "127.0.0.1:11434");
                assert_eq!(model, "bge-m3");
            }
            _ => panic!("应为 Ollama 配置"),
        }
    }

    #[test]
    fn test_ollama_base_url_normalization() {
        // 裸主机：补协议 + 默认端口
        assert_eq!(ollama_base_url("localhost"), "http://localhost:11434");
        // 已带端口：不重复追加（此前会拼成 http://127.0.0.1:11434:11434 ⇒ builder error）
        assert_eq!(ollama_base_url("127.0.0.1:11434"), "http://127.0.0.1:11434");
        // 已带协议：不重复补协议、不重复补端口
        assert_eq!(
            ollama_base_url("http://localhost:11434/"),
            "http://localhost:11434"
        );
        // 自定义端口保留
        assert_eq!(
            ollama_base_url("http://0.0.0.0:8080"),
            "http://0.0.0.0:8080"
        );
        // IPv6 字面量：方括号内冒号不算端口
        assert_eq!(ollama_base_url("[::1]"), "http://[::1]:11434");
        // 拼接后的完整路径必须是合法 URL
        let full = format!("{}/api/embeddings", ollama_base_url("127.0.0.1:11434"));
        assert!(reqwest::Url::parse(&full).is_ok(), "URL 非法: {full}");
    }

    #[test]
    fn test_parse_config_unknown() {
        let result = LlmApiConfig::parse("unknown:something");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_configured() {
        assert!(!LlmApiConfig::None.is_configured());
        let config = LlmApiConfig::parse("openai:sk-test:gpt-4o-mini").unwrap();
        assert!(config.is_configured());
    }

    #[test]
    fn test_build_prompt() {
        let prompt = build_prompt(CODE_TRANSLATION_PROMPT, "处理用户登录");
        assert!(prompt.contains("处理用户登录"));
        assert!(prompt.contains("代码搜索助手"));
    }

    /// ★v0.9.11（能力对齐）确定性单测：**未配置 LLM 时，检索富化必须逐字返回原查询**。
    ///
    /// 这条是整个"能力对齐"的**零行为变更**保证：REST `enrich` 从"完全不翻译"
    /// 变为"走同一富化函数"，只要 LLM 未配置，它就必须与改造前**逐字一致**。
    /// （配置了 LLM 时才会追加关键词；那条路径依赖真实网络，故不在此做单测。）
    #[tokio::test]
    async fn enrich_query_is_identity_when_llm_not_configured() {
        let cfg = LlmApiConfig::None;
        let q = "What degree did I graduate with?";
        assert_eq!(enrich_query_for_retrieval(&cfg, q).await, q);
        assert_eq!(enrich_query_for_retrieval(&cfg, "").await, "");
    }
}
