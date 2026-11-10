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
const CURRENT_RULES_VERSION: u32 = 1;

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
    /// 已写入规则的版本
    #[serde(default)]
    pub rules_version: u32,
    /// 已写入规则的工具快照
    #[serde(default)]
    pub rules_agents: Vec<String>,
    /// API Key 加密存储（Base64 编码的 AES-256-GCM 密文）
    /// 空字符串表示未配置 API Key（Ollama 等不需要 Key 的场景）
    #[serde(default)]
    pub encrypted_api_key: String,
    /// 运行时状态：已配置的 API Key 无法解密时为 true，不写入磁盘。
    #[serde(skip)]
    pub api_key_invalid: bool,
    /// v0.8.31 S-03：AI 工具手动修正（用户点击齿轮时设置）
    /// key = agent_id，value = true 表示强制设为已安装，false 表示强制设为未安装
    /// 优先级：最高（即使自动检测结果相反也以这里为准）
    #[serde(default)]
    pub manual_agent_overrides: std::collections::HashMap<String, bool>,
    /// v0.10.4：LLM 重排开关（sidecar 的 `/v1/config/rerank` 写入本字段）。
    ///
    /// # 为什么桌面端也必须有这个字段（否则是静默丢配置）
    ///
    /// 本结构体是**强类型且整份序列化回盘**（见 `save()`）。此前它不含该字段，
    /// 于是 `load()` 反序列化时静默丢弃 sidecar 写入的 `rerank_enabled`，
    /// 下一次 `save()`（启动时 `rules_need_update → save_rules_state` 就会触发）
    /// 用默认值整份覆盖 ⇒ 用户开启的重排开关**重启后凭空失效**。
    ///
    /// 桌面端不修改该值，只负责**读入并原样写回**（round-trip 保真）。
    #[serde(default)]
    pub rerank_enabled: bool,
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
    ///   "ollama||http://localhost:11434||llama3"
    ///
    /// 向后兼容：若输入不含 ||，则按旧格式（冒号分隔）解析并记录警告日志：
    ///   "openai:sk-xxx:gpt-4o:https://api.openai.com/v1"
    ///   "ollama:localhost:llama3"
    ///
    /// API Key 使用 AES-256-GCM 加密后存储（L1-02），
    /// 原始 Key 不会以明文形式写入磁盘。
    pub fn parse_llm_config(&mut self, llm_api: &str) -> Result<(), String> {
        self.api_key_invalid = false;
        // M-5 修复：优先使用 || 分隔符（支持 API Key 中包含冒号）
        // 向后兼容：若输入不含 ||，回退到旧分隔符 : 并记录警告
        let parts: Vec<&str> = if llm_api.contains("||") {
            llm_api.splitn(4, "||").collect()
        } else {
            tracing::warn!(
                "LLM 配置使用旧格式（冒号分隔），建议迁移到新格式（|| 分隔）；当 API Key 包含冒号时旧格式会解析错误"
            );
            llm_api.splitn(4, ':').collect()
        };

        match parts.first() {
            Some(&"openai") => {
                self.llm_type = "openai".into();
                self.llm_configured = false;
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
                self.llm_configured = !self.encrypted_api_key.is_empty();
            }
            Some(&"ollama") => {
                self.llm_type = "ollama".into();
                self.llm_configured = true;
                // Ollama 不需要 API Key
                self.encrypted_api_key = String::new();
                // v0.9.14 修复：统一为 `ollama||host||model`（与 sidecar 的
                // LlmApiConfig::parse 一致）。旧顺序 `model||host` 会让 sidecar
                // 把模型名当主机地址，请求 URL 变成 http://<model>:11434 ⇒ builder error。
                let first = parts.get(1).map(|s| s.trim()).unwrap_or("");
                let second = parts.get(2).map(|s| s.trim()).unwrap_or("");
                // 兼容历史输入：第二段是带协议地址、第一段不是 ⇒ 仍是 model 在前
                let legacy_order = (second.starts_with("http://")
                    || second.starts_with("https://"))
                    && !(first.starts_with("http://") || first.starts_with("https://"));
                let (host, model) = if legacy_order {
                    (second, first)
                } else {
                    (first, second)
                };
                if !host.is_empty() {
                    self.llm_base_url = Some(host.to_string());
                }
                if !model.is_empty() {
                    self.llm_model = Some(model.to_string());
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

    /// 返回 API Key 是否存在且可解密。
    pub fn api_key_status(&self) -> &'static str {
        if self.encrypted_api_key.is_empty() {
            "not_configured"
        } else if self.api_key_invalid {
            "invalid"
        } else {
            "configured"
        }
    }

    /// 运行时校验加密 API Key，并记录不可用状态。
    pub fn refresh_api_key_status(&mut self) {
        self.api_key_invalid = !self.encrypted_api_key.is_empty()
            && crypto::decrypt_api_key(&self.encrypted_api_key).is_err();
        if self.api_key_invalid {
            self.llm_configured = false;
        }
    }

    /// 检查 API Key 是否已配置（是否有加密存储）
    pub fn has_api_key(&self) -> bool {
        !self.encrypted_api_key.is_empty()
    }

    /// 将存储的配置重建为 LLM API 字符串（用于传递给 Sidecar）
    ///
    /// 格式与 parse_llm_config 推荐格式一致（使用 || 分隔符）：
    ///   "openai||sk-xxx||gpt-4o||https://api.openai.com/v1"
    ///   "ollama||http://localhost:11434||llama3"
    ///
    /// 返回 None 表示 LLM 未配置。
    pub fn to_llm_api_string(&self) -> Option<String> {
        if !self.llm_configured || self.api_key_invalid {
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
                // v0.9.14 修复：host 在前，与 sidecar 解析约定一致
                Some(format!("ollama||{}||{}", host, model))
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
            rules_version: 0,
            rules_agents: Vec::new(),
            encrypted_api_key: String::new(),
            api_key_invalid: false,
            manual_agent_overrides: std::collections::HashMap::new(),
            rerank_enabled: false,
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
        let mut config = match std::fs::read_to_string(&config_path) {
            Ok(json) => match serde_json::from_str::<WizardConfig>(&json) {
                Ok(cfg) => cfg,
                Err(e) => {
                    // v0.5.4 修复：JSON 解析失败时记录详细日志
                    let preview: String = json.chars().take(200).collect();
                    tracing::error!(
                        "配置文件 {} 解析失败，将使用默认配置。错误：{e}。原始内容（前200字符）：{}",
                        config_path.display(),
                        preview
                    );
                    corrupted = true;
                    WizardConfig::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => WizardConfig::default(),
            Err(e) => {
                // v0.5.4 修复：文件读取失败时记录日志
                tracing::error!(
                    "无法读取配置文件 {}，将使用默认配置。错误：{e}",
                    config_path.display()
                );
                corrupted = true;
                WizardConfig::default()
            }
        };

        config.refresh_api_key_status();
        if config.api_key_invalid {
            tracing::error!("向导配置中的 API Key 无法解密，已标记为无效");
        }

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
                let _ = atomic_save(&config_path, &json);
            }
        }

        // ── 自动迁移：已有有效配置但未标记完成 → 自动完成 ──
        if !config.setup_complete && config.project_dir.is_some() && config.llm_configured {
            tracing::info!("检测到已有有效配置但向导未完成，自动设置 setup_complete=true");
            config.setup_complete = true;
            // 立即持久化，避免下次启动仍需手动操作
            if let Ok(json) = serde_json::to_string_pretty(&config) {
                let _ = atomic_save(&config_path, &json);
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

    /// v0.9.9：是否**确有使用痕迹**（用户已实际用过这个应用）。
    ///
    /// # 为什么需要它（修「每次打开都要手动点启动服务」）
    ///
    /// 自动启动原先只看 `setup_complete`。但 `setup_complete` 只有在**走完向导**
    /// 时才会置位，而本项目的两条正常使用路径都不走向导：
    ///   · **全局模式**（不选项目目录）⇒ `project_dir == None`
    ///   · **不用 LLM**（纯本地检索）⇒ `llm_configured == false`
    /// 而上面的自动迁移（`project_dir.is_some() && llm_configured`）恰好
    /// **要求这两个都成立** ⇒ 该路径下 `setup_complete` 永远为 false
    /// ⇒ 自动启动被永久跳过。实测实锤：`tauri-dev-run.log` 第 38 行
    /// 「wizard 未完成配置（setup_complete=false, file_existed=true），跳过自动启动」，
    /// 而同一台机器上记忆库已有 3175 条、规则已写入 4 个 AI 工具
    /// ——用户明显在正常使用，配置**实际可用**。
    ///
    /// # 判据：以「能否正常工作」为准，而非「是否走过向导」
    ///
    /// 只要满足任一条，就说明用户已经用起来了：
    ///   · 走过向导（`setup_complete`）
    ///   · 有项目目录（项目模式已被使用）
    ///   · 配过 LLM
    ///   · **写过 AI 规则**（`rules_agents` 非空）—— 这是最可靠的痕迹：
    ///     规则写入发生在启动流程里，能写成功说明应用已在正常提供服务
    pub fn has_usage_history(&self) -> bool {
        self.config.setup_complete
            || self.config.project_dir.is_some()
            || self.config.llm_configured
            || !self.config.rules_agents.is_empty()
            || !self.config.configured_agents.is_empty()
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

    /// 返回当前规则状态是否需要重新写入。
    pub fn rules_need_update(&self, agents: &[String]) -> bool {
        let mut current = agents.to_vec();
        current.sort();
        let mut saved = self.config.rules_agents.clone();
        saved.sort();
        self.config.rules_version != CURRENT_RULES_VERSION || saved != current
    }

    /// 保存规则写入状态。
    pub fn save_rules_state(&mut self, agents: Vec<String>) -> Result<(), String> {
        let mut sorted = agents;
        sorted.sort();
        self.config.rules_version = CURRENT_RULES_VERSION;
        self.config.rules_agents = sorted;
        self.save()
    }

    /// v0.8.31 S-03：设置/清除 AI 工具的手动修正
    /// agent_id: 工具 ID（如 "trae"、"codebuddy"）
    /// override_installed:
    ///   - Some(true)  = 用户判定为已安装（覆盖自动检测的 installed=false）
    ///   - Some(false) = 用户判定为未安装（覆盖自动检测的 installed=true）
    ///   - None        = 清除该工具的手动修正（恢复为自动检测结果）
    pub fn set_agent_manual_override(
        &mut self,
        agent_id: &str,
        override_installed: Option<bool>,
    ) -> Result<(), String> {
        match override_installed {
            Some(val) => {
                self.config
                    .manual_agent_overrides
                    .insert(agent_id.to_string(), val);
            }
            None => {
                self.config.manual_agent_overrides.remove(agent_id);
            }
        }
        self.save()
    }

    /// v0.8.31 S-03：获取所有手动修正的拷贝（用于 discover_all_agents 覆盖结果）
    pub fn get_manual_agent_overrides(&self) -> std::collections::HashMap<String, bool> {
        self.config.manual_agent_overrides.clone()
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
    ///
    /// v0.8.31 S-03：重置时也保留 manual_agent_overrides（用户纠正过的误检不应因重置而丢失）
    pub fn reset(&mut self) -> Result<(), String> {
        let saved_llm_configured = self.config.llm_configured;
        let saved_llm_type = self.config.llm_type.clone();
        let saved_llm_model = self.config.llm_model.clone();
        let saved_llm_base_url = self.config.llm_base_url.clone();
        let saved_key = self.config.encrypted_api_key.clone();
        // S-03：保留用户对工具检测结果的手动修正
        let saved_overrides = self.config.manual_agent_overrides.clone();

        self.config = WizardConfig::default();

        // 恢复 LLM 配置（P2-22 修复）
        self.config.llm_configured = saved_llm_configured;
        self.config.llm_type = saved_llm_type;
        self.config.llm_model = saved_llm_model;
        self.config.llm_base_url = saved_llm_base_url;
        self.config.encrypted_api_key = saved_key;
        // S-03：恢复手动修正（用户明确纠正过的误检/漏检优先级最高）
        self.config.manual_agent_overrides = saved_overrides;

        // 保存重置后的配置（而非删除文件），确保 API Key 不丢失
        self.save()?;
        tracing::info!(
            "向导状态已重置（LLM 配置/API Key/工具手动修正 已保留），用户下次打开应用时将看到配置向导"
        );
        Ok(())
    }

    /// 检测是否处于开发模式
    ///
    /// 开发模式下使用独立的配置文件路径，与稳定版完全隔离。
    fn is_dev_mode() -> bool {
        cfg!(debug_assertions)
    }

    /// 配置存储路径
    ///
    /// M-14 修复：APPDATA 未设置时使用 dirs crate 回退，而非当前目录（安全风险）。
    /// 优先级：APPDATA 环境变量 → dirs::config_dir() → dirs::data_dir() → 错误
    /// v0.9.0：开发模式使用独立路径 %APPDATA%\LoongRecall\dev\wizard.json，与稳定版完全隔离
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

        // v0.9.0 开发模式隔离：使用独立配置文件，不与稳定版共享
        let loong_dir = base_dir.join("LoongRecall");
        let path = if Self::is_dev_mode() {
            let dev_path = loong_dir.join("dev").join("wizard.json");
            tracing::info!("[开发模式] 向导配置路径: {}", dev_path.display());
            dev_path
        } else {
            loong_dir.join("wizard.json")
        };
        Ok(path)
    }

    /// 持久化到磁盘
    ///
    /// 安全：配置文件存储在 %APPDATA%\LoongRecall\ 下，
    /// Windows 默认 ACL 仅允许当前用户 + SYSTEM + Administrators 访问（L1-03）。
    ///
    /// # 为什么不是"整份结构体直接覆盖写"（v0.10.4 修复）
    ///
    /// wizard.json 是**桌面端与 sidecar 双写**的共享文件：
    /// 桌面端拥有向导/规则/LLM 凭据，sidecar 拥有 `rerank_enabled` 等运行时开关。
    /// 此前本函数把 `WizardConfig` 整份序列化覆盖 ⇒ sidecar 写入的字段被抹掉。
    ///
    /// 现在改为**字段级所有权**：先读磁盘最新内容，用本进程持有的字段覆盖，
    /// 磁盘上其余字段（属 sidecar 所有）原样保留。即便未来 sidecar 再新增开关，
    /// 也无需同步修改本结构体。
    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        // 我方字段的 JSON 表示（权威来源是本进程内存中的 config）
        let owned = serde_json::to_value(&self.config).map_err(|e| e.to_string())?;

        // 磁盘现有内容：存在且可解析则作为基底。
        //
        // ★v0.10.4：解析失败时**先备份损坏文件**，再以空对象为基底重建。
        //
        // 修前的注释声称"解析失败时不回退到空对象覆盖，避免把损坏文件上的残留
        // 字段一并丢掉"，但代码实际是 `unwrap_or_else(|_| json!({}))` —— 直接
        // 以空对象为基底，随后被本进程字段写满并落盘 ⇒ 损坏文件里 sidecar 拥有的
        // 字段（`rerank_enabled` 等）**永久消失**，且原始内容无从追溯。
        //
        // 现在：损坏 ⇒ 复制一份带时间戳的 `.corrupted-<ts>` 备份（尽力而为，
        // 失败不阻断写入），再重建。这样既能让保存继续完成，又保留取证/恢复能力。
        let mut merged = match std::fs::read_to_string(&self.config_path) {
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) if v.is_object() => v,
                _ => {
                    backup_corrupted(&self.config_path);
                    serde_json::json!({})
                }
            },
            // 文件不存在是正常首次保存；其它 IO 错误同样按空基底处理
            Err(_) => serde_json::json!({}),
        };

        // 用我方字段覆盖基底（我方字段优先，其余字段保留）
        // `rerank_enabled` 属 sidecar 所有，此处**不回写**内存中的旧值，
        // 因为磁盘可能比内存新（用户在仪表盘刚改过）。
        if let (Some(base), Some(own)) = (merged.as_object_mut(), owned.as_object()) {
            for (key, value) in own {
                if key == "rerank_enabled" {
                    continue;
                }
                base.insert(key.clone(), value.clone());
            }
        } else {
            merged = owned;
        }

        let json = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
        atomic_save(&self.config_path, &json)
    }
}

/// 临时文件名唯一性计数器（`atomic_save` 用）。
static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 把损坏的配置文件复制为带时间戳的备份（尽力而为，失败不阻断调用方）。
///
/// 供 `save()` 在"磁盘内容无法解析"时调用：既避免静默丢弃 sidecar 拥有的字段，
/// 又给用户/支持人员留下可追溯的原始内容。
fn backup_corrupted(path: &std::path::Path) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = path.with_extension(format!("corrupted-{ts}.json"));
    match std::fs::copy(path, &backup) {
        Ok(_) => tracing::warn!(
            "[配置向导] wizard.json 解析失败，已备份原文件到 {}",
            backup.display()
        ),
        Err(e) => tracing::warn!(
            "[配置向导] wizard.json 解析失败，且备份失败（{}）；将以已知字段重建",
            e
        ),
    }
}

/// 在任意路径上以原子方式写入 JSON 配置。
///
/// # 原子性（v0.10.4 修复）
///
/// 此前 Windows 分支是「写临时文件 → `fs::copy` 覆盖目标」：`copy` 不是原子的，
/// 并发读者（sidecar）可能读到**写了一半**的 JSON；一旦此时进程崩溃，
/// wizard.json 就永久损坏，下次 `load()` 回退默认值 ⇒ 用户的 API Key 密文丢失。
///
/// 现在统一为「唯一临时名 + `sync_all` + `rename`」：
/// Rust 的 `fs::rename` 在 Windows 上走 `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`，
/// 在 Unix 上是同目录 `rename`，**两者都是原子的目录项替换**。
fn atomic_save(path: &std::path::Path, json: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {}", e))?;
    }
    // 唯一临时名：并发写同一路径时互不争用（固定 `.json.tmp` 会互相踩踏）
    // 不引入 uuid 依赖：进程号 + 单调计数器 + 纳秒时间戳已足够唯一
    //（同一进程内计数器保证唯一，跨进程由 pid 区分）。
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("wizard");
    let seq = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_path = path.with_file_name(format!(
        "{}.{}.{}.{}.tmp",
        file_name,
        std::process::id(),
        seq,
        nanos
    ));

    let result = (|| -> Result<(), String> {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp_path)
            .map_err(|e| format!("写入临时配置文件失败: {}", e))?;
        file.write_all(json.as_bytes())
            .map_err(|e| format!("写入临时配置文件失败: {}", e))?;
        // 先落盘再 rename：否则崩溃后可能"目录项指向新文件、内容仍在页缓存"⇒ 0 字节配置
        file.sync_all()
            .map_err(|e| format!("同步临时配置文件失败: {}", e))?;
        drop(file);
        std::fs::rename(&temp_path, path).map_err(|e| format!("提交配置文件失败: {}", e))
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result?;

    tracing::debug!("配置文件已写入: {}", path.display());
    Ok(())
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
        assert!(!config.rerank_enabled, "重排开关默认必须是关闭");
    }

    /// ★v0.10.4 回归：桌面端 `save()` **不得**抹掉 sidecar 写入的字段。
    ///
    /// # 这条锁住的是一个真实缺陷
    ///
    /// wizard.json 是桌面端与 sidecar **双写**的共享文件。修复前 `save()` 用
    /// `WizardConfig` 整份序列化覆盖 ⇒ 用户在仪表盘开启的 `rerank_enabled`
    /// 在下一次桌面端保存（启动时 `save_rules_state` 就会触发）后**凭空丢失**，
    /// 表现为"重排开关重启后自动关闭"。
    ///
    /// 本测试模拟该时序：先由 sidecar 侧写入 `rerank_enabled=true` 与一个
    /// 桌面端不认识的未来字段，再让桌面端执行一次 `save()`，断言两者都还在。
    #[test]
    fn test_save_preserves_sidecar_owned_fields() {
        // 桌面端 crate 无 tempfile dev-dependency，用标准库建唯一临时目录
        let dir = std::env::temp_dir().join(format!(
            "lrc_wizard_test_{}_{}",
            std::process::id(),
            TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let path = dir.join("wizard.json");

        // 1) sidecar 侧写入（含桌面端不认识的未来字段）
        //    必填字段（llm_configured / configured_agents）也一并给出，模拟真实文件
        let initial = serde_json::json!({
            "setup_complete": true,
            "llm_configured": true,
            "llm_type": "openai",
            "configured_agents": ["trae"],
            "rerank_enabled": true,
            "some_future_switch": "keep-me"
        });
        std::fs::write(&path, serde_json::to_string_pretty(&initial).unwrap()).unwrap();

        // 2) 桌面端 load → save（模拟启动时规则状态持久化）
        let mut state = WizardState {
            // 与 WizardState::load() 的解析分支等价
            config: serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
                .expect("应能解析 sidecar 写入的配置"),
            config_path: path.clone(),
            corrupted_on_load: false,
            file_existed: true,
        };
        assert!(state.config.rerank_enabled, "load 必须能读到 sidecar 写入的开关");
        state.config.rules_version = 1;
        state.save().expect("保存应成功");

        // 3) 断言：sidecar 拥有的字段在桌面端保存后仍然存在
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            after.get("rerank_enabled").and_then(|v| v.as_bool()),
            Some(true),
            "★桌面端保存不得抹掉 sidecar 写入的 rerank_enabled"
        );
        assert_eq!(
            after.get("some_future_switch").and_then(|v| v.as_str()),
            Some("keep-me"),
            "★桌面端不认识的字段也必须原样保留（否则每加一个 sidecar 开关就要改桌面端）"
        );
        // 桌面端自己的字段照常写入
        assert_eq!(after.get("rules_version").and_then(|v| v.as_u64()), Some(1));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★★v0.9.9：全新安装**不得**被判为"有使用痕迹"。
    ///
    /// 这是自动启动判据的安全边界：若全新安装被误判为"老用户"，
    /// 首次打开就会跳过向导直接起服务，用户失去配置入口。
    #[test]
    fn test_has_usage_history_false_for_fresh_install() {
        let state = WizardState {
            config: WizardConfig::default(),
            config_path: PathBuf::new(),
            corrupted_on_load: false,
            file_existed: true,
        };
        assert!(
            !state.has_usage_history(),
            "★全新安装（无任何痕迹）不得被判为有使用痕迹"
        );
    }

    /// ★★v0.9.9：**全局模式 + 不用 LLM** 的正常使用路径必须被判为"有使用痕迹"。
    ///
    /// # 这条测的就是用户报的那个 bug
    ///
    /// 实测现场：`setup_complete=false` + `project_dir=null` + `llm_configured=false`
    /// + `rules_agents` 有 4 个工具 ⇒ 原判据（只看 `setup_complete`）判为"未配置"
    /// ⇒ 自动启动被跳过 ⇒ 用户每次都要手点。
    /// 修法后凭 `rules_agents` 非空即可判定 —— 本测试锁住这个判定。
    #[test]
    fn test_has_usage_history_true_for_global_mode_with_rules() {
        let mut config = WizardConfig::default();
        // 复刻实测现场：不走向导、全局模式、不用 LLM，但已写过规则
        config.setup_complete = false;
        config.project_dir = None;
        config.llm_configured = false;
        config.rules_agents = vec![
            "codebuddy".into(),
            "minimax-code".into(),
            "tongyi-lingma".into(),
            "trae-cn".into(),
        ];
        let state = WizardState {
            config,
            config_path: PathBuf::new(),
            corrupted_on_load: false,
            file_existed: true, // ★文件存在（不是丢失），故不走 file_existed 兜底
        };
        assert!(
            state.has_usage_history(),
            "★★全局模式 + 不用 LLM + 已写规则，必须判为有使用痕迹\
             （否则自动启动被永久跳过，用户每次都要手点）"
        );
    }

    /// v0.9.9：单一痕迹各自也能成立（判据取「或」，不是「且」）。
    #[test]
    fn test_has_usage_history_each_signal_alone_suffices() {
        // ① 只走过向导
        let mut c = WizardConfig::default();
        c.setup_complete = true;
        assert!(WizardState {
            config: c,
            config_path: PathBuf::new(),
            corrupted_on_load: false,
            file_existed: true
        }
        .has_usage_history());

        // ② 只有项目目录
        let mut c = WizardConfig::default();
        c.project_dir = Some("G:/proj".into());
        assert!(WizardState {
            config: c,
            config_path: PathBuf::new(),
            corrupted_on_load: false,
            file_existed: true
        }
        .has_usage_history());

        // ③ 只配过 LLM
        let mut c = WizardConfig::default();
        c.llm_configured = true;
        assert!(WizardState {
            config: c,
            config_path: PathBuf::new(),
            corrupted_on_load: false,
            file_existed: true
        }
        .has_usage_history());

        // ④ 只有 configured_agents
        let mut c = WizardConfig::default();
        c.configured_agents = vec!["trae-cn".into()];
        assert!(WizardState {
            config: c,
            config_path: PathBuf::new(),
            corrupted_on_load: false,
            file_existed: true
        }
        .has_usage_history());
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
    fn test_invalid_api_key_is_not_reported_as_configured() {
        let mut config = WizardConfig {
            llm_type: "openai".into(),
            llm_configured: true,
            encrypted_api_key: "不是有效密文".into(),
            ..WizardConfig::default()
        };

        config.refresh_api_key_status();

        assert!(config.api_key_invalid);
        assert!(!config.llm_configured);
        assert_eq!(config.api_key_status(), "invalid");
        assert!(config.to_llm_api_string().is_none());
    }

    #[test]
    fn test_llm_config_parsing_ollama() {
        let mut config = WizardConfig::default();
        config
            .parse_llm_config("ollama:localhost:llama3")
            .expect("解析失败");
        assert!(config.llm_configured);
        assert_eq!(config.llm_type, "ollama");
        assert_eq!(config.llm_model, Some("llama3".into()));
        assert_eq!(config.llm_base_url, Some("localhost".into()));
        // Ollama 不需要 API Key
        assert!(config.encrypted_api_key.is_empty());
        assert!(config.get_api_key().is_none());
    }

    /// TDD：测试 Ollama 新格式 || 分隔符解析（host 在前）
    #[test]
    fn test_llm_config_parsing_ollama_new_format() {
        let mut config = WizardConfig::default();
        config
            .parse_llm_config("ollama||http://localhost:11434||llama3")
            .expect("解析失败");
        assert!(config.llm_configured);
        assert_eq!(config.llm_type, "ollama");
        assert_eq!(config.llm_model, Some("llama3".into()));
        assert_eq!(config.llm_base_url, Some("http://localhost:11434".into()));
        assert!(config.encrypted_api_key.is_empty());
        assert!(config.get_api_key().is_none());
    }

    /// TDD：Ollama 往返一致性 —— 输出格式必须能被自身与 sidecar 正确解析
    #[test]
    fn test_ollama_roundtrip_keeps_host_and_model() {
        let mut config = WizardConfig::default();
        config
            .parse_llm_config("ollama||127.0.0.1:11434||bge-m3")
            .expect("解析失败");
        let api_string = config.to_llm_api_string().expect("应返回 LLM API 字符串");
        assert_eq!(api_string, "ollama||127.0.0.1:11434||bge-m3");

        let mut config2 = WizardConfig::default();
        config2.parse_llm_config(&api_string).expect("往返解析失败");
        assert_eq!(config2.llm_model, Some("bge-m3".into()));
        assert_eq!(config2.llm_base_url, Some("127.0.0.1:11434".into()));
    }

    /// TDD：兼容历史 `ollama||model||host` 输入（旧桌面端顺序）
    #[test]
    fn test_llm_config_parsing_ollama_legacy_order() {
        let mut config = WizardConfig::default();
        config
            .parse_llm_config("ollama||llama3||http://localhost:11434")
            .expect("解析失败");
        assert_eq!(config.llm_model, Some("llama3".into()));
        assert_eq!(config.llm_base_url, Some("http://localhost:11434".into()));
    }
}
