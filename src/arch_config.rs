// ============================================================
// 许可证: Apache 2.0
// 本文件实现架构记忆存储，属于公开层 (Layer 1)。
// ============================================================
//
// 架构记忆配置（Architecture Memory Config）
//
// 架构记忆存储：
//   存储洛书算子参数、衰减曲线、权限策略等系统级配置，
//   支持系统自举与演化（服务重启后自动恢复上次运行参数）。
//
// 持久化格式：JSON 文件，存储在数据目录下的 `arch_config.json`

use crate::memory_types::DecayConfig;
use serde::{Deserialize, Serialize};

/// 架构级持久化配置
///
/// 包含所有可调的系统参数，这些参数决定 Loong Recall 的运行行为。
/// 服务重启后自动从 `arch_config.json` 加载，实现系统自举。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchConfig {
    /// 配置版本号（用于向后兼容的迁移）
    pub version: String,
    /// 衰减曲线配置
    pub decay: DecayConfig,
    /// 合成参数配置
    pub synthesis: SynthesisArchConfig,
    /// 检索参数配置
    pub retrieval: RetrievalArchConfig,
    /// 隐私策略配置
    pub privacy: PrivacyArchConfig,
    /// 洛书编码器参数
    pub luoshu: LuoshuArchConfig,
    /// 最后一次更新时间
    pub updated_at: String,
}

/// 合成参数配置（属于架构记忆）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisArchConfig {
    /// 最小聚类大小（达到此数量才触发合成，默认 3）
    pub min_cluster: usize,
    /// 合成相似度阈值（Jaccard ≥ 此值纳入同一簇，默认 0.4）
    pub similarity_threshold: f32,
    /// 合成置信度阈值（低于此值的合成结果被丢弃，默认 0.3）
    pub confidence_threshold: f32,
    /// 是否启用洛书驱动合成（八卦分类 + 门控融合）
    pub use_luoshu_synthesis: bool,
    /// 是否启用 Jaccard 驱动合成（词集相似度聚类）
    pub use_jaccard_synthesis: bool,
}

/// 检索参数配置（属于架构记忆）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalArchConfig {
    /// 默认返回数量
    pub default_top_k: usize,
    /// 梯形聚焦检索深度（0=全量，1=4分，2=16分）
    pub trapezoid_depth: u32,
    /// RRF 融合参数 k（倒数排名融合的常数，默认 60）
    pub rrf_k: f32,
    /// 快速通路返回倍数（快速结果 = top_k × fast_path_multiplier）
    pub fast_path_multiplier: usize,
}

/// 隐私策略配置（属于架构记忆）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyArchConfig {
    /// 默认隐私级别（session / user / global）
    pub default_level: String,
    /// 是否启用 Session 级隔离
    pub enable_session_isolation: bool,
    /// 是否启用 User 级隔离
    pub enable_user_isolation: bool,
    /// Session 级记忆的默认 TTL 天数（None = 永久）
    pub session_ttl_days: Option<u32>,
}

/// 洛书编码器参数配置（属于架构记忆）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuoshuArchConfig {
    /// 是否使用洛书先验权重
    pub use_prior: bool,
    /// 迭代投影轮数（幻和收敛迭代次数，默认 5）
    pub projection_iterations: u32,
    /// 特征提取权重：字符密度
    pub feature_density_weight: f32,
    /// 特征提取权重：信息熵
    pub feature_entropy_weight: f32,
    /// 特征提取权重：位置衰减
    pub feature_position_weight: f32,
}

impl Default for SynthesisArchConfig {
    fn default() -> Self {
        Self {
            min_cluster: 3,
            similarity_threshold: 0.4,
            confidence_threshold: 0.3,
            use_luoshu_synthesis: true,
            use_jaccard_synthesis: true,
        }
    }
}

impl Default for RetrievalArchConfig {
    fn default() -> Self {
        Self {
            default_top_k: 5,
            trapezoid_depth: 1,
            rrf_k: 60.0,
            fast_path_multiplier: 2,
        }
    }
}

impl Default for PrivacyArchConfig {
    fn default() -> Self {
        Self {
            default_level: "user".to_string(),
            enable_session_isolation: true,
            enable_user_isolation: true,
            session_ttl_days: Some(7),
        }
    }
}

impl Default for LuoshuArchConfig {
    fn default() -> Self {
        Self {
            use_prior: true,
            projection_iterations: 5,
            feature_density_weight: 0.4,
            feature_entropy_weight: 0.3,
            feature_position_weight: 0.3,
        }
    }
}

impl Default for ArchConfig {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            decay: DecayConfig::default(),
            synthesis: SynthesisArchConfig::default(),
            retrieval: RetrievalArchConfig::default(),
            privacy: PrivacyArchConfig::default(),
            luoshu: LuoshuArchConfig::default(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

impl ArchConfig {
    /// 从数据目录加载架构配置
    ///
    /// 如果配置文件不存在，返回默认配置并自动保存。
    pub fn load_or_default(data_dir: &str) -> Self {
        let path = format!("{}/arch_config.json", data_dir);
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<ArchConfig>(&content) {
                Ok(config) => {
                    eprintln!("[LRC·架构] 已加载架构配置 v{}", config.version);
                    config
                }
                Err(e) => {
                    eprintln!("[LRC·架构] 配置解析失败 ({}), 使用默认配置", e);
                    let default = Self::default();
                    let _ = default.save(data_dir);
                    default
                }
            },
            Err(_) => {
                let default = Self::default();
                eprintln!("[LRC·架构] 未找到配置文件, 已创建默认架构配置");
                let _ = default.save(data_dir);
                default
            }
        }
    }

    /// 保存架构配置到数据目录
    pub fn save(&self, data_dir: &str) -> Result<(), std::io::Error> {
        let path = format!("{}/arch_config.json", data_dir);
        // 确保目录存在
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut config = self.clone();
        config.updated_at = chrono::Utc::now().to_rfc3339();
        let json = serde_json::to_string_pretty(&config).map_err(std::io::Error::other)?;
        std::fs::write(&path, json)
    }

    /// 更新衰减配置并保存
    pub fn update_decay(
        &mut self,
        decay: DecayConfig,
        data_dir: &str,
    ) -> Result<(), std::io::Error> {
        self.decay = decay;
        self.save(data_dir)
    }

    /// 应用配置到 MemoryStore（通过回调方式）
    ///
    /// 将所有架构级参数同步到运行中的 MemoryStore 实例。
    pub fn apply_to_store<P: crate::persistence::Persistence>(
        &self,
        store: &mut crate::memory_store::MemoryStore<P>,
    ) {
        // 衰减曲线
        store.decay_config = self.decay.clone();

        // 合成参数
        store.synthesis_min_cluster = self.synthesis.min_cluster;
        store.synthesis_similarity = self.synthesis.similarity_threshold;
    }
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = ArchConfig::default();
        assert_eq!(config.decay.decay_rate, 0.05);
        assert_eq!(config.synthesis.min_cluster, 3);
        assert_eq!(config.retrieval.default_top_k, 5);
        assert_eq!(config.privacy.default_level, "user");
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();

        // 保存默认配置
        let config = ArchConfig {
            decay: DecayConfig::aggressive(),
            ..ArchConfig::default()
        };
        config.save(&data_dir).expect("应成功保存");

        // 重新加载
        let loaded = ArchConfig::load_or_default(&data_dir);
        assert_eq!(loaded.decay.decay_rate, 0.15);
        assert_eq!(loaded.decay.topo_weight, 0.5);
    }

    #[test]
    fn test_load_default_when_missing() {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();

        // 目录中无配置文件
        let config = ArchConfig::load_or_default(&data_dir);
        assert_eq!(config.decay.decay_rate, 0.05);

        // 应自动创建了配置文件
        let path = format!("{}/arch_config.json", data_dir);
        assert!(std::path::Path::new(&path).exists(), "应自动创建配置文件");
    }

    #[test]
    fn test_apply_to_store() {
        use crate::memory_store::MemoryStore;
        use crate::persistence::create_json_persistence;

        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = create_json_persistence(&data_dir).expect("应成功创建");
        let mut store = MemoryStore::new(p);

        let config = ArchConfig {
            decay: DecayConfig::aggressive(),
            synthesis: SynthesisArchConfig {
                min_cluster: 5,
                similarity_threshold: 0.6,
                ..Default::default()
            },
            ..Default::default()
        };

        config.apply_to_store(&mut store);
        assert_eq!(store.decay_config.decay_rate, 0.15);
        assert_eq!(store.synthesis_min_cluster, 5);
        assert_eq!(store.synthesis_similarity, 0.6);
    }
}
