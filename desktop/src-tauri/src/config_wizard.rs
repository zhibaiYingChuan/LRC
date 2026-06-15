/// 配置向导模块
///
/// 管理首次配置流程的状态和持久化。
/// 存储路径：%APPDATA%\LoongRecall\wizard.json
/// 
/// 安全：API Key 使用 AES-256-GCM 加密存储（L1-02），
/// 配置文件仅当前用户可读写（L1-03）。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::crypto; // L1 数据加密模块

/// 向导配置版本号
/// 当配置结构发生变化时递增此值，旧版本配置将被自动迁移或重置。
const CURRENT_CONFIG_VERSION: u32 = 1;

/// 向导配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardConfig {
    /// 配置版本号（用于自动迁移旧版本配置）
    #[serde(default = "default_config_version")]
    pub config_version: u32,
    /// 是否已完成首次配置
    pub setup_complete: bool,
    /// 项目目录路径
    pub project_dir: Option<String>,
    /// LLM 是否已配置
    pub llm_configured: bool,
    /// LLM 提供商类型
    pub llm_type: String,
    /// LLM 模型名称
    pub llm_model: Option<String>,
    /// LLM API 基础 URL（OpenAI 兼容格式时使用，如 https://api.deepseek.com/v1）
    #[serde(default)]
    pub llm_base_url: Option<String>,
    /// 已配置的 Agent 列表
    pub configured_agents: Vec<String>,
    /// API Key 加密存储（Base64 编码的 AES-256-GCM 密文）
    /// 空字符串表示未配置 API Key（Ollama 等不需要 Key 的场景）
    #[serde(default)]
    pub encrypted_api_key: String,
}

/// serde default 函数：新版本默认版本号
fn default_config_version() -> u32 {
    CURRENT_CONFIG_VERSION
}

impl WizardConfig {
    /// 解析 LLM API 配置字符串并加密 API Key
    /// 
    /// 格式：
    ///   "openai:sk-xxx:gpt-4o:https://api.openai.com/v1"
    ///   "ollama:llama3:http://localhost:11434"
    /// 
    /// API Key 使用 AES-256-GCM 加密后存储（L1-02），
    /// 原始 Key 不会以明文形式写入磁盘。
    pub fn parse_llm_config(&mut self, llm_api: &str) -> Result<(), String> {
        let parts: Vec<&str> = llm_api.splitn(4, ':').collect();

        match parts.first() {
            Some(&"openai") => {
                self.llm_type = "openai".into();
                self.llm_configured = true;
                // parts: [openai, api_key, model, base_url]
                if let Some(api_key) = parts.get(1) {
                    if !api_key.is_empty() {
                        self.encrypted_api_key = crypto::encrypt_api_key(api_key)?;
                    }
                }
                if let Some(model) = parts.get(2) {
                    if !model.is_empty() {
                        self.llm_model = Some(model.to_string());
                    }
                }
                // 存储 base_url（国产模型如 DeepSeek/通义千问有不同端点）
                if let Some(base_url) = parts.get(3) {
                    if !base_url.is_empty() {
                        self.llm_base_url = Some(base_url.to_string());
                    }
                }
            }
            Some(&"ollama") => {
                self.llm_type = "ollama".into();
                self.llm_configured = true;
                // Ollama 不需要 API Key
                self.encrypted_api_key = String::new();
                // parts: [ollama, model, host]
                if let Some(model) = parts.get(1) {
                    if !model.is_empty() {
                        self.llm_model = Some(model.to_string());
                    }
                }
                // 存储 Ollama host（用户可能自定义地址）
                if let Some(host) = parts.get(2) {
                    if !host.is_empty() {
                        self.llm_base_url = Some(host.to_string());
                    }
                }
            }
            _ => {
                self.llm_configured = false;
                self.llm_type = "none".into();
                self.encrypted_api_key = String::new();
            }
        }
        Ok(())
    }

    /// 获取解密后的 API Key
    /// 
    /// 返回 None 表示未配置 API Key（如 Ollama 场景）。
    pub fn get_api_key(&self) -> Option<String> {
        if self.encrypted_api_key.is_empty() {
            return None;
        }
        crypto::decrypt_api_key(&self.encrypted_api_key).ok()
    }

    /// 检查 API Key 是否已配置（是否有加密存储）
    pub fn has_api_key(&self) -> bool {
        !self.encrypted_api_key.is_empty()
    }

    /// 将存储的配置重建为 LLM API 字符串（用于传递给 Sidecar）
    ///
    /// 格式与 parse_llm_config 输入格式一致：
    ///   "openai:sk-xxx:gpt-4o:https://api.openai.com/v1"
    ///   "ollama:llama3:http://localhost:11434"
    ///
    /// 返回 None 表示 LLM 未配置。
    pub fn to_llm_api_string(&self) -> Option<String> {
        if !self.llm_configured {
            return None;
        }

        match self.llm_type.as_str() {
            "openai" => {
                let api_key = self.get_api_key()?;
                let model = self.llm_model.as_deref().unwrap_or("gpt-4o-mini");
                // 使用用户配置的实际 base_url，而非硬编码 OpenAI 地址
                let base_url = self.llm_base_url.as_deref().unwrap_or("https://api.openai.com/v1");
                Some(format!("openai:{}:{}:{}", api_key, model, base_url))
            }
            "ollama" => {
                let model = self.llm_model.as_deref().unwrap_or("llama3");
                // 使用用户配置的实际 Ollama host
                let host = self.llm_base_url.as_deref().unwrap_or("http://localhost:11434");
                Some(format!("ollama:{}:{}", model, host))
            }
            _ => None,
        }
    }
}

impl Default for WizardConfig {
    fn default() -> Self {
        Self {
            config_version: CURRENT_CONFIG_VERSION,
            setup_complete: false,
            project_dir: None,
            llm_configured: false,
            llm_type: "none".into(),
            llm_model: None,
            llm_base_url: None,
            configured_agents: Vec::new(),
            encrypted_api_key: String::new(),
        }
    }
}

/// 向导状态管理器
pub struct WizardState {
    config: WizardConfig,
    config_path: PathBuf,
}

impl WizardState {
    /// 加载或创建向导状态
    /// 
    /// 自动迁移：
    ///   1. 版本不匹配 → 重置配置，重新引导用户完成向导
    ///   2. 已有有效配置（project_dir + llm_configured）但 setup_complete 为 false → 自动完成
    /// 这解决了从旧版本升级或配置已存在但向导未完成标记的问题。
    pub fn load() -> Self {
        let config_path = Self::config_path();
        let mut config = if config_path.exists() {
            let json = std::fs::read_to_string(&config_path).unwrap_or_default();
            serde_json::from_str(&json).unwrap_or_default()
        } else {
            WizardConfig::default()
        };

        // ── 版本迁移：旧版本配置 → 重置向导 ──
        if config.config_version < CURRENT_CONFIG_VERSION {
            tracing::info!(
                "检测到旧版本配置 (v{})，当前版本 v{}，将重置向导",
                config.config_version,
                CURRENT_CONFIG_VERSION
            );
            // 保留 encrypted_api_key（避免用户重新输入 Key）
            let saved_key = config.encrypted_api_key.clone();
            config = WizardConfig::default();
            config.encrypted_api_key = saved_key;
            // 立即持久化新版本配置
            if let Ok(json) = serde_json::to_string_pretty(&config) {
                if let Some(parent) = config_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&config_path, json);
            }
        }

        // ── 自动迁移：已有有效配置但未标记完成 → 自动完成 ──
        if !config.setup_complete && config.project_dir.is_some() && config.llm_configured {
            tracing::info!(
                "检测到已有有效配置但向导未完成，自动设置 setup_complete=true"
            );
            config.setup_complete = true;
            // 立即持久化，避免下次启动仍需手动操作
            if let Ok(json) = serde_json::to_string_pretty(&config) {
                if let Some(parent) = config_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&config_path, json);
            }
        }

        Self {
            config,
            config_path,
        }
    }

    /// 获取配置的只读引用
    pub fn config(&self) -> &WizardConfig {
        &self.config
    }

    /// 设置项目目录
    pub fn set_project_dir(&mut self, dir: &str) -> Result<(), String> {
        self.config.project_dir = Some(dir.into());
        self.save()
    }

    /// 保存 LLM API 配置（含 API Key 加密）
    pub fn save_llm_config(&mut self, llm_api: &str) -> Result<(), String> {
        self.config.parse_llm_config(llm_api)?;
        self.save()
    }

    /// 标记配置完成
    pub fn mark_complete(&mut self) -> Result<(), String> {
        self.config.setup_complete = true;
        self.save()
    }

    /// 保存已配置的 Agent 列表（P2-05 修复：确保 configured_agents 持久化）
    pub fn save_configured_agents(&mut self, agents: Vec<String>) -> Result<(), String> {
        self.config.configured_agents = agents;
        self.save()
    }

    /// 配置存储路径
    fn config_path() -> PathBuf {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        PathBuf::from(appdata)
            .join("LoongRecall")
            .join("wizard.json")
    }

    /// 持久化到磁盘
    /// 
    /// 安全：配置文件存储在 %APPDATA%\LoongRecall\ 下，
    /// Windows 默认 ACL 仅允许当前用户 + SYSTEM + Administrators 访问（L1-03）。
    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&self.config).map_err(|e| e.to_string())?;
        std::fs::write(&self.config_path, json).map_err(|e| e.to_string())?;
        tracing::debug!("配置已保存 (encrypted_api_key={}B)", self.config.encrypted_api_key.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD：测试默认配置
    #[test]
    fn test_default_config() {
        let config = WizardConfig::default();
        assert!(!config.setup_complete);
        assert!(config.project_dir.is_none());
        assert!(!config.llm_configured);
        assert_eq!(config.llm_type, "none");
    }

    /// TDD：测试 LLM 配置解析
    #[test]
    fn test_llm_config_parsing_openai() {
        let mut config = WizardConfig::default();
        config.parse_llm_config("openai:sk-test:gpt-4o:https://api.openai.com/v1").expect("解析失败");
        assert!(config.llm_configured);
        assert_eq!(config.llm_type, "openai");
        assert_eq!(config.llm_model, Some("gpt-4o".into()));
        // API Key 应已加密存储
        assert!(!config.encrypted_api_key.is_empty(), "API Key 应已加密");
        // 解密后应恢复原始 Key
        let decrypted = config.get_api_key().expect("解密失败");
        assert_eq!(decrypted, "sk-test");
    }

    #[test]
    fn test_llm_config_parsing_ollama() {
        let mut config = WizardConfig::default();
        config.parse_llm_config("ollama:llama3:http://localhost:11434").expect("解析失败");
        assert!(config.llm_configured);
        assert_eq!(config.llm_type, "ollama");
        assert_eq!(config.llm_model, Some("llama3".into()));
        // Ollama 不需要 API Key
        assert!(config.encrypted_api_key.is_empty());
        assert!(config.get_api_key().is_none());
    }
}