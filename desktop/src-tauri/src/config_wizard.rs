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
    /// 格式（推荐，使用 || 分隔符，支持 API Key 中包含冒号）：
    ///   "openai||sk-xxx||gpt-4o||https://api.openai.com/v1"
    ///   "ollama||llama3||http://localhost:11434"
    ///
    /// 向后兼容：若输入不含 ||，则按旧格式（冒号分隔）解析并记录警告日志：
    ///   "openai:sk-xxx:gpt-4o:https://api.openai.com/v1"
    ///   "ollama:llama3:http://localhost:11434"
    ///
    /// API Key 使用 AES-256-GCM 加密后存储（L1-02），
    /// 原始 Key 不会以明文形式写入磁盘。
    pub fn parse_llm_config(&mut self, llm_api: &str) -> Result<(), String> {
        // M-5 修复：优先使用 || 分隔符（支持 API Key 中包含冒号）
        // 向后兼容：若输入不含 ||，回退到旧分隔符 : 并记录警告
        let parts: Vec<&str> = if llm_api.contains("||") {
            llm_api.splitn(4, "||").collect()
        } else {
            tracing::warn!(
                "LLM 配置使用旧格式（冒号分隔），建议迁移到新格式（|| 分隔）。\
                 当 API Key 包含冒号时旧格式会解析错误。输入: {}",
                llm_api
            );
            llm_api.splitn(4, ':').collect()
        };

        match parts.first() {
            Some(&"openai") => {
                self.llm_type = "openai".into();
                self.llm_configured = true;
                // parts: [openai, api_key, model, base_url]
                if let Some(api_key) = parts.get(1) {
                    if !api_key.is_empty() {
                        // v0.5.4 修复：API Key 清洗 — trim + 过滤 \r\n 等控制字符
                        let cleaned_key: String = api_key
                            .trim()
                            .chars()
                            .filter(|c| !c.is_control() || *c == ' ')
                            .collect();
                        if !cleaned_key.is_empty() {
                            self.encrypted_api_key = crypto::encrypt_api_key(&cleaned_key)?;
                        }
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
    /// 格式与 parse_llm_config 推荐格式一致（使用 || 分隔符）：
    ///   "openai||sk-xxx||gpt-4o||https://api.openai.com/v1"
    ///   "ollama||llama3||http://localhost:11434"
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
                let base_url = self
                    .llm_base_url
                    .as_deref()
                    .unwrap_or("https://api.openai.com/v1");
                // M-5 修复：使用 || 分隔符，避免 API Key 中包含冒号时解析错误
                Some(format!("openai||{}||{}||{}", api_key, model, base_url))
            }
            "ollama" => {
                let model = self.llm_model.as_deref().unwrap_or("llama3");
                // 使用用户配置的实际 Ollama host
                let host = self
                    .llm_base_url
                    .as_deref()
                    .unwrap_or("http://localhost:11434");
                Some(format!("ollama||{}||{}", model, host))
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
    /// v0.5.4 新增：配置是否从损坏状态恢复
    /// 当配置文件读取或解析失败时设为 true，前端可据此提示用户
    pub corrupted_on_load: bool,
    /// v0.8.21 P0-01 新增：wizard.json 文件在 load() 时是否已存在
    /// false 表示文件不存在（首次安装或文件意外丢失），
    /// main.rs 据此在自动启动判断中兜底（避免 wizard.json 丢失导致 sidecar 永不自动启动）
    pub file_existed: bool,
}

// v0.6.0 P3-1 修复：实现 Default trait，避免 load() 失败时 panic
impl Default for WizardState {
    fn default() -> Self {
        Self {
            config: WizardConfig::default(),
            config_path: PathBuf::new(),
            corrupted_on_load: false,
            file_existed: false,
        }
    }
}

impl WizardState {
    /// 加载或创建向导状态
    ///
    /// 自动迁移：
    ///   1. 版本不匹配 → 重置配置，重新引导用户完成向导
    ///   2. 已有有效配置（project_dir + llm_configured）但 setup_complete 为 false → 自动完成
    ///      这解决了从旧版本升级或配置已存在但向导未完成标记的问题。
    ///
    /// v0.5.4 修复：配置损坏时记录日志，设置 corrupted_on_load 标记供前端检测
    pub fn load() -> Result<Self, String> {
        let config_path = Self::config_path()?;
        let mut corrupted = false;
        // v0.8.21 P0-01：记录 wizard.json 是否存在，供 main.rs 自动启动兜底判断
        let file_existed = config_path.exists();
        let mut config = if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(json) => {
                    match serde_json::from_str::<WizardConfig>(&json) {
                        Ok(cfg) => cfg,
                        Err(e) => {
                            // v0.5.4 修复：JSON 解析失败时记录详细日志
                            tracing::error!(
                                "配置文件 {} 解析失败，将使用默认配置。错误：{e}。原始内容（前200字符）：{}",
                                config_path.display(),
                                &json[..json.len().min(200)]
                            );
                            corrupted = true;
                            WizardConfig::default()
                        }
                    }
                }
                Err(e) => {
                    // v0.5.4 修复：文件读取失败时记录日志
                    tracing::error!(
                        "无法读取配置文件 {}，将使用默认配置。错误：{e}",
                        config_path.display()
                    );
                    corrupted = true;
                    WizardConfig::default()
                }
            }
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
            tracing::info!("检测到已有有效配置但向导未完成，自动设置 setup_complete=true");
            config.setup_complete = true;
            // 立即持久化，避免下次启动仍需手动操作
            if let Ok(json) = serde_json::to_string_pretty(&config) {
                if let Some(parent) = config_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&config_path, json);
            }
        }

        Ok(Self {
            config,
            config_path,
            corrupted_on_load: corrupted,
            file_existed,
        })
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

    /// v0.5.3 新增：重置向导状态，让用户重新进入配置向导
    ///
    /// v0.5.4 修复：改用 save() 而非删除文件，确保 API Key 在重置后仍然保留。
    /// 原逻辑：删除 wizard.json → 下次 load() 创建全新默认配置 → API Key 丢失。
    /// 新逻辑：save() 写入重置后的配置 → setup_complete=false → API Key 保留。
    ///
    /// v0.5.4 P2-22 修复：重置时保留 LLM 配置（llm_configured/llm_type/llm_model/llm_base_url/encrypted_api_key）
    /// 修复前：reset() 将 llm_configured 重置为 false，但保留了 encrypted_api_key，
    ///         导致 to_llm_api_string() 返回 None，sidecar 的 state.llm_api 为 None，
    ///         仪表盘显示"LLM 未配置"，与桌面端实际配置状态不一致。
    /// 修复后：reset() 只重置 setup_complete 和 configured_agents，保留 LLM 配置，
    ///         用户重新配置时不需要重新输入 LLM API Key。
    pub fn reset(&mut self) -> Result<(), String> {
        let saved_llm_configured = self.config.llm_configured;
        let saved_llm_type = self.config.llm_type.clone();
        let saved_llm_model = self.config.llm_model.clone();
        let saved_llm_base_url = self.config.llm_base_url.clone();
        let saved_key = self.config.encrypted_api_key.clone();

        self.config = WizardConfig::default();

        // 恢复 LLM 配置（P2-22 修复）
        self.config.llm_configured = saved_llm_configured;
        self.config.llm_type = saved_llm_type;
        self.config.llm_model = saved_llm_model;
        self.config.llm_base_url = saved_llm_base_url;
        self.config.encrypted_api_key = saved_key;

        // 保存重置后的配置（而非删除文件），确保 API Key 不丢失
        self.save()?;
        tracing::info!(
            "向导状态已重置（LLM 配置和 API Key 已保留），用户下次打开应用时将看到配置向导"
        );
        Ok(())
    }

    /// 配置存储路径
    ///
    /// M-14 修复：APPDATA 未设置时使用 dirs crate 回退，而非当前目录（安全风险）。
    /// 优先级：APPDATA 环境变量 → dirs::config_dir() → dirs::data_dir() → 错误
    fn config_path() -> Result<PathBuf, String> {
        // 优先使用 APPDATA 环境变量（保持向后兼容）
        let base_dir = if let Ok(appdata) = std::env::var("APPDATA") {
            if !appdata.is_empty() {
                PathBuf::from(appdata)
            } else {
                // APPDATA 为空字符串，回退到 dirs crate
                tracing::warn!("APPDATA 环境变量为空，使用 dirs::config_dir() 作为回退");
                dirs::config_dir().or_else(dirs::data_dir).ok_or_else(|| {
                    "无法确定配置目录：APPDATA 为空且 dirs::config_dir()/data_dir() 均返回 None"
                        .to_string()
                })?
            }
        } else {
            // APPDATA 未设置，回退到 dirs crate
            tracing::warn!("APPDATA 环境变量未设置，使用 dirs::config_dir() 作为回退");
            dirs::config_dir().or_else(dirs::data_dir).ok_or_else(|| {
                "无法确定配置目录：APPDATA 未设置且 dirs::config_dir()/data_dir() 均返回 None"
                    .to_string()
            })?
        };

        Ok(base_dir.join("LoongRecall").join("wizard.json"))
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
        tracing::debug!(
            "配置已保存 (encrypted_api_key={}B)",
            self.config.encrypted_api_key.len()
        );
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

    /// TDD：测试 LLM 配置解析（旧格式冒号分隔，向后兼容）
    #[test]
    fn test_llm_config_parsing_openai() {
        let mut config = WizardConfig::default();
        config
            .parse_llm_config("openai:sk-test:gpt-4o:https://api.openai.com/v1")
            .expect("解析失败");
        assert!(config.llm_configured);
        assert_eq!(config.llm_type, "openai");
        assert_eq!(config.llm_model, Some("gpt-4o".into()));
        // API Key 应已加密存储
        assert!(!config.encrypted_api_key.is_empty(), "API Key 应已加密");
        // 解密后应恢复原始 Key
        let decrypted = config.get_api_key().expect("解密失败");
        assert_eq!(decrypted, "sk-test");
    }

    /// TDD：测试 LLM 配置解析（新格式 || 分隔符）
    #[test]
    fn test_llm_config_parsing_openai_new_format() {
        let mut config = WizardConfig::default();
        config
            .parse_llm_config("openai||sk-test||gpt-4o||https://api.openai.com/v1")
            .expect("解析失败");
        assert!(config.llm_configured);
        assert_eq!(config.llm_type, "openai");
        assert_eq!(config.llm_model, Some("gpt-4o".into()));
        assert!(!config.encrypted_api_key.is_empty(), "API Key 应已加密");
        let decrypted = config.get_api_key().expect("解密失败");
        assert_eq!(decrypted, "sk-test");
    }

    /// TDD：M-5 修复验证 — API Key 中包含冒号，只有 || 格式能正确解析
    #[test]
    fn test_llm_config_parsing_openai_colon_in_key() {
        let mut config = WizardConfig::default();
        // API Key "sk-abc:def" 包含冒号，旧格式会解析错误
        config
            .parse_llm_config("openai||sk-abc:def||gpt-4o||https://api.openai.com/v1")
            .expect("解析失败");
        assert!(config.llm_configured);
        assert_eq!(config.llm_type, "openai");
        assert_eq!(config.llm_model, Some("gpt-4o".into()));
        let decrypted = config.get_api_key().expect("解密失败");
        // 关键断言：API Key 中的冒号应被完整保留
        assert_eq!(decrypted, "sk-abc:def");
    }

    /// TDD：测试 to_llm_api_string 使用 || 分隔符输出
    #[test]
    fn test_to_llm_api_string_uses_new_separator() {
        let mut config = WizardConfig::default();
        config
            .parse_llm_config("openai||sk-test123||gpt-4o||https://api.deepseek.com/v1")
            .expect("解析失败");
        let api_string = config.to_llm_api_string().expect("应返回 LLM API 字符串");
        // 应使用 || 分隔符（而非冒号作为字段分隔符）
        assert!(api_string.starts_with("openai||"), "应以 openai|| 开头");
        // 应能被 parse_llm_config 正确解析回来（往返一致性）
        let mut config2 = WizardConfig::default();
        config2.parse_llm_config(&api_string).expect("往返解析失败");
        assert_eq!(config2.llm_type, "openai");
        assert_eq!(config2.llm_model, Some("gpt-4o".into()));
        assert_eq!(config2.get_api_key().expect("解密失败"), "sk-test123");
    }

    #[test]
    fn test_llm_config_parsing_ollama() {
        let mut config = WizardConfig::default();
        config
            .parse_llm_config("ollama:llama3:http://localhost:11434")
            .expect("解析失败");
        assert!(config.llm_configured);
        assert_eq!(config.llm_type, "ollama");
        assert_eq!(config.llm_model, Some("llama3".into()));
        // Ollama 不需要 API Key
        assert!(config.encrypted_api_key.is_empty());
        assert!(config.get_api_key().is_none());
    }

    /// TDD：测试 Ollama 新格式 || 分隔符解析
    #[test]
    fn test_llm_config_parsing_ollama_new_format() {
        let mut config = WizardConfig::default();
        config
            .parse_llm_config("ollama||llama3||http://localhost:11434")
            .expect("解析失败");
        assert!(config.llm_configured);
        assert_eq!(config.llm_type, "ollama");
        assert_eq!(config.llm_model, Some("llama3".into()));
        assert!(config.encrypted_api_key.is_empty());
        assert!(config.get_api_key().is_none());
    }
}
