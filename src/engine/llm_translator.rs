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
    Ollama {
        host: String,
        model: String,
    },
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

/// 硬编码的查询翻译 Prompt — 极度精简，只做一件事
const TRANSLATION_PROMPT: &str = "你是一个代码搜索助手。将用户的自然语言查询翻译成可能出现在代码中的函数名、变量名、结构体名或关键词。只返回逗号分隔的关键词列表，不要解释。";

/// 构建翻译 Prompt
fn build_prompt(query: &str) -> String {
    format!("{}\n\n用户查询：{}", TRANSLATION_PROMPT, query)
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
pub async fn translate_query(
    config: &LlmApiConfig,
    query: &str,
) -> Vec<String> {
    match config {
        LlmApiConfig::None => {
            vec![query.to_string()]
        }
        LlmApiConfig::OpenAI {
            api_key,
            model,
            endpoint,
        } => {
            match translate_openai(endpoint, api_key, model, query).await {
                Ok(keywords) if !keywords.is_empty() => keywords,
                _ => {
                    eprintln!("[LRC] LLM 翻译失败，回退到原始查询");
                    vec![query.to_string()]
                }
            }
        }
        LlmApiConfig::Ollama { host, model } => {
            match translate_ollama(host, model, query).await {
                Ok(keywords) if !keywords.is_empty() => keywords,
                _ => {
                    eprintln!("[LRC] LLM 翻译失败，回退到原始查询");
                    vec![query.to_string()]
                }
            }
        }
    }
}

/// 通过 OpenAI 兼容 API 翻译查询
async fn translate_openai(
    endpoint: &str,
    api_key: &str,
    model: &str,
    query: &str,
) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let request = OpenAiChatRequest {
        model: model.to_string(),
        messages: vec![OpenAiMessage {
            role: "user".to_string(),
            content: build_prompt(query),
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
        return Err(format!("LLM API 返回错误: {}", response.status()));
    }

    let body: OpenAiChatResponse = response
        .json()
        .await
        .map_err(|e| format!("解析 LLM 响应失败: {}", e))?;

    let content = body
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default();

    Ok(parse_keywords(&content))
}

/// 通过 Ollama 本地模型翻译查询
async fn translate_ollama(
    host: &str,
    model: &str,
    query: &str,
) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let request = OllamaChatRequest {
        model: model.to_string(),
        messages: vec![OllamaMessage {
            role: "user".to_string(),
            content: build_prompt(query),
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
        return Err(format!("Ollama API 返回错误: {}", response.status()));
    }

    let body: OllamaChatResponse = response
        .json()
        .await
        .map_err(|e| format!("解析 Ollama 响应失败: {}", e))?;

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
        assert_eq!(
            result,
            vec!["authenticate_user", "login", "handle_login"]
        );
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
        let config =
            LlmApiConfig::parse("openai:sk-test:gpt-4:https://custom.api.com/v1").unwrap();
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
        let prompt = build_prompt("处理用户登录");
        assert!(prompt.contains("处理用户登录"));
        assert!(prompt.contains("代码搜索助手"));
    }
}