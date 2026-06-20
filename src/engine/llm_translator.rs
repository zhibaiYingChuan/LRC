// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件包含模型底层架构衍生的核心算法，受研究许可证保护。
// 禁止逆向工程、禁止商业再分发、禁止用于训练竞争模型。
// ============================================================
// 不参与检索、不参与记忆存储、不参与索引。
// 只做一件事：把用户的模糊自然语言，翻译成精确的代码关键词。
// 然后由 Fast Match 去执行真正的检索。

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
    /// 支持的格式：
    /// - `openai:sk-xxx:gpt-4o-mini` → OpenAI API
    /// - `openai:sk-xxx:gpt-4o-mini:https://custom.api.com/v1` → 自定义 OpenAI 端点
    /// - `ollama:localhost:llama3` → Ollama 本地模型
    pub fn parse(input: &str) -> Result<Self, String> {
        let parts: Vec<&str> = input.splitn(4, ':').collect();

        match parts[0] {
            "openai" => {
                let api_key = parts
                    .get(1)
                    .filter(|s| !s.is_empty())
                    .ok_or("openai 模式需要 API Key: openai:sk-xxx:model")?
                    .to_string();
                let model = parts
                    .get(2)
                    .filter(|s| !s.is_empty())
                    .ok_or("openai 模式需要模型名: openai:sk-xxx:gpt-4o-mini")?
                    .to_string();
                let endpoint = parts
                    .get(3)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

                Ok(LlmApiConfig::OpenAI {
                    api_key,
                    model,
                    endpoint,
                })
            }
            "ollama" => {
                let host = parts
                    .get(1)
                    .filter(|s| !s.is_empty())
                    .ok_or("ollama 模式需要主机地址: ollama:localhost:model")?
                    .to_string();
                let model = parts
                    .get(2)
                    .filter(|s| !s.is_empty())
                    .ok_or("ollama 模式需要模型名: ollama:localhost:llama3")?
                    .to_string();

                Ok(LlmApiConfig::Ollama { host, model })
            }
            other => Err(format!(
                "不支持的 LLM API 类型: {}。支持的类型: openai, ollama",
                other
            )),
        }
    }

    /// 是否已配置 LLM API
    pub fn is_configured(&self) -> bool {
        !matches!(self, LlmApiConfig::None)
    }
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

/// 构建翻译 Prompt（通用）
fn build_prompt(system_prompt: &str, query: &str) -> String {
    format!("{}\n\n用户查询：{}", system_prompt, query)
}

/// 将 LLM 响应解析为关键词列表
fn parse_keywords(response: &str) -> Vec<String> {
    response
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
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
) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

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

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("LLM 请求失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        eprintln!("[LRC·LLM] OpenAI API 返回错误状态: {} (响应: {:.200})", status, body);
        return Err(format!("LLM API 返回错误: {}", status));
    }

    // v0.5.4 P2-11 修复：先获取原始文本，解析失败时可以记录用于调试
    let raw_body = response
        .text()
        .await
        .map_err(|e| format!("读取 LLM 响应体失败: {}", e))?;

    let body: OpenAiChatResponse = serde_json::from_str(&raw_body).map_err(|e| {
        eprintln!(
            "[LRC·LLM] OpenAI 响应 JSON 解析失败: {} (原始响应: {:.200})",
            e, raw_body
        );
        format!("解析 LLM 响应失败: {}", e)
    })?;

    // v0.5.4 P2-11 修复：防御性检查空 choices 数组
    if body.choices.is_empty() {
        eprintln!("[LRC·LLM] OpenAI 返回空 choices 数组，可能模型拒绝回答或发生内部错误");
        return Err("LLM 返回空 choices 数组".to_string());
    }

    let content = body
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default();

    // v0.5.4 P2-11 修复：防御性检查空内容
    if content.trim().is_empty() {
        eprintln!("[LRC·LLM] OpenAI 返回空内容，使用原始查询回退");
        return Err("LLM 返回空内容".to_string());
    }

    Ok(parse_keywords(&content))
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
) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let request = OllamaChatRequest {
        model: model.to_string(),
        messages: vec![OllamaMessage {
            role: "user".to_string(),
            content: build_prompt(system_prompt, query),
        }],
        stream: false,
    };

    let url = format!("http://{}:11434/api/chat", host);

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("Ollama 请求失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        eprintln!("[LRC·LLM] Ollama API 返回错误状态: {} (响应: {:.200})", status, body);
        return Err(format!("Ollama API 返回错误: {}", status));
    }

    // v0.5.4 P2-11 修复：先获取原始文本，解析失败时可以记录用于调试
    let raw_body = response
        .text()
        .await
        .map_err(|e| format!("读取 Ollama 响应体失败: {}", e))?;

    let body: OllamaChatResponse = serde_json::from_str(&raw_body).map_err(|e| {
        eprintln!(
            "[LRC·LLM] Ollama 响应 JSON 解析失败: {} (原始响应: {:.200})",
            e, raw_body
        );
        format!("解析 Ollama 响应失败: {}", e)
    })?;

    // v0.5.4 P2-11 修复：防御性检查空内容
    if body.message.content.trim().is_empty() {
        eprintln!("[LRC·LLM] Ollama 返回空内容，使用原始查询回退");
        return Err("Ollama 返回空内容".to_string());
    }

    Ok(parse_keywords(&body.message.content))
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
}
