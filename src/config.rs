// ============================================================
// 许可证: Apache 2.0
// 本文件实现配置持久化，属于公开层 (Layer 1)。
// ============================================================
//
// 配置持久化模块 — 支持桌面端agent配置保存与加载
//
// 核心能力:
//   1. LrcConfig — 完整配置结构，包含端口、LLM API、源码目录等
//   2. 自动保存到用户配置目录 (%APPDATA%\LoongRecall\config.json)
//   3. 自动加载已有配置，支持增量修改
//   4. 支持全局配置vs项目级配置分离

use crate::engine::llm_translator::LlmApiConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::{fs, io};

/// LRC 完整配置结构（可持久化）
///
/// 保存用户所有可配置参数，支持JSON序列化/反序列化。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LrcConfig {
    /// 默认端口（HTTP服务）
    pub default_port: u16,
    /// 默认绑定主机
    pub default_host: String,
    /// LLM API 配置（如果启用）
    pub llm_api: Option<String>,
    /// 解析后的LLM API配置（内存中，不持久化）
    #[serde(skip_serializing, skip_deserializing)]
    pub parsed_llm_api: Option<LlmApiConfig>,
    /// 多窗口最大数量（默认1）
    pub max_multi_window: u8,
    /// 是否开机自启动（桌面端）
    pub auto_start_on_boot: bool,
    /// 是否最小化到系统托盘（桌面端）
    pub minimize_to_tray: bool,
    /// 是否启动后自动打开仪表盘
    pub auto_open_dashboard: bool,
    /// 全局配置文件版本，用于迁移
    pub config_version: u32,
}

impl Default for LrcConfig {
    fn default() -> Self {
        Self {
            default_port: 3099,
            default_host: "127.0.0.1".to_string(),
            llm_api: None,
            parsed_llm_api: None,
            max_multi_window: 1,
            auto_start_on_boot: false,
            minimize_to_tray: true,
            auto_open_dashboard: true,
            config_version: 1,
        }
    }
}

impl LrcConfig {
    /// 创建新配置（默认值）
    pub fn new() -> Self {
        Self::default()
    }

    /// 从已保存的文件加载配置
    ///
    /// 如果文件不存在，返回默认配置。
    /// 如果文件损坏/解析失败，返回默认配置。
    pub fn load() -> Self {
        match Self::get_config_path() {
            Ok(path) => {
                if !path.exists() {
                    return Self::default();
                }
                match fs::read_to_string(&path) {
                    Ok(content) => match serde_json::from_str(&content) {
                        Ok(config) => config,
                        Err(e) => {
                            eprintln!("[配置] 解析配置文件失败，使用默认配置: {}", e);
                            Self::default()
                        }
                    },
                    Err(e) => {
                        eprintln!("[配置] 读取配置文件失败，使用默认配置: {}", e);
                        Self::default()
                    }
                }
            }
            Err(_) => Self::default(),
        }
    }

    /// 保存当前配置到文件
    ///
    /// 自动创建父目录，如果创建失败则返回错误。
    pub fn save(&self) -> Result<(), String> {
        let path = Self::get_config_path()
            .map_err(|e| format!("获取配置路径失败: {}", e))?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建配置目录失败: {}", e))?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("序列化配置失败: {}", e))?;

        fs::write(&path, json)
            .map_err(|e| format!("写入配置文件失败: {}", e))?;

        Ok(())
    }

    /// 获取全局配置文件路径
    ///
    /// Windows: `%APPDATA%\LoongRecall\config.json`
    /// Linux: `~/.config/LoongRecall/config.json`
    /// macOS: `~/Library/Application Support/LoongRecall/config.json`
    pub fn get_config_path() -> io::Result<PathBuf> {
        #[cfg(windows)]
        {
            let app_data = std::env::var("APPDATA")
                .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "APPDATA not found"))?;
            Ok(Path::new(&app_data)
                .join("LoongRecall")
                .join("config.json"))
        }

        #[cfg(target_os = "linux")]
        {
            let home = std::env::var("HOME")
                .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "HOME not found"))?;
            Ok(Path::new(&home)
                .join(".config")
                .join("LoongRecall")
                .join("config.json"))
        }

        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME")
                .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "HOME not found"))?;
            Ok(Path::new(&home)
                .join("Library")
                .join("Application Support")
                .join("LoongRecall")
                .join("config.json"))
        }

        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            Ok(Path::new(".loongrecall-config.json").to_path_buf())
        }
    }

    /// 解析LLM API配置（从保存的字符串解析为LlmApiConfig）
    ///
    /// 调用此方法后，结果存入 `parsed_llm_api` 字段。
    pub fn parse_llm_api(&mut self) -> Result<(), String> {
        match &self.llm_api {
            Some(raw) => {
                if raw.trim().is_empty() {
                    self.parsed_llm_api = None;
                    return Ok(());
                }
                let parsed = LlmApiConfig::parse(raw.trim())?;
                self.parsed_llm_api = Some(parsed);
                Ok(())
            }
            None => {
                self.parsed_llm_api = None;
                Ok(())
            }
        }
    }
}
