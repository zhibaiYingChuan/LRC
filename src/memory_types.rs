// ============================================================
// 许可证: Apache 2.0
// 本文件定义通用记忆类型，属于公开层 (Layer 1)。
// ============================================================
//
// 记忆类型定义
//
// 面向 AI 助手的通用永久记忆数据类型。
// 不局限于代码——支持事实、偏好、决策、对话等多种记忆类型。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// 衰减曲线配置（Section 3.3 衰减与遗忘 — 可配置参数）
///
/// 控制记忆衰减行为的两大参数：
/// - `decay_rate`: 时间衰减速度（默认 0.05），越大衰减越快
/// - `topo_weight`: 拓扑深度影响权重（默认 0.3），越大拓扑位置影响越大
///
/// 最终衰减因子 = 时间衰减(e^(-decay_rate × days / importance)) × 拓扑保护(1.0 - topological_depth × topo_weight)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayConfig {
    /// 时间衰减速率（0.0 ~ 1.0），默认 0.05
    /// 值越大，记忆随时间衰减越快
    pub decay_rate: f32,
    /// 拓扑深度衰减权重（0.0 ~ 1.0），默认 0.3
    /// 值越大，九宫格边缘位置的记忆衰减越快
    pub topo_weight: f32,
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self {
            decay_rate: 0.05,
            topo_weight: 0.3,
        }
    }
}

impl DecayConfig {
    /// 创建衰减配置（自动 clamp 到合法范围）
    pub fn new(decay_rate: f32, topo_weight: f32) -> Self {
        Self {
            decay_rate: decay_rate.clamp(0.0, 1.0),
            topo_weight: topo_weight.clamp(0.0, 1.0),
        }
    }

    /// 激进衰减模式：记忆快速衰减，适合存储空间有限的场景
    /// decay_rate=0.15, topo_weight=0.5
    pub fn aggressive() -> Self {
        Self {
            decay_rate: 0.15,
            topo_weight: 0.5,
        }
    }

    /// 保守衰减模式：记忆长期保留，适合知识管理场景
    /// decay_rate=0.01, topo_weight=0.1
    pub fn conservative() -> Self {
        Self {
            decay_rate: 0.01,
            topo_weight: 0.1,
        }
    }
}

/// 记忆类型枚举
///
/// 不同记忆类型影响存储策略和检索优先级。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// 事实 — 客观信息，如 "项目 X 使用 PostgreSQL 作为数据库"
    Fact,
    /// 偏好 — 用户/项目的配置偏好，如 "用户偏好使用 pnpm 而非 npm"
    Preference,
    /// 决策 — 重要的架构/设计决策记录，如 "选择 Actix Web 因为其高性能"
    Decision,
    /// 代码上下文 — 索引后的代码片段记忆
    CodeContext,
    /// 对话 — 对话轮次中提炼的关键信息
    Conversation,
    /// 合成 — 递归合成产生的抽象知识（多条相关记忆的融合结果）
    Synthesis,
}

impl MemoryType {
    /// 从字符串解析记忆类型（返回 Option，None 表示无效类型）
    ///
    /// 此方法委托给 `FromStr` trait 实现，但返回 `Option` 以兼容现有调用方。
    pub fn try_parse(s: &str) -> Option<Self> {
        s.parse().ok()
    }

    /// 转为小写字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Decision => "decision",
            Self::CodeContext => "code_context",
            Self::Conversation => "conversation",
            Self::Synthesis => "synthesis",
        }
    }

    /// 列出所有有效的类型字符串
    pub fn valid_values() -> &'static [&'static str] {
        &["fact", "preference", "decision", "code_context", "conversation", "synthesis"]
    }
}

impl FromStr for MemoryType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "fact" => Ok(Self::Fact),
            "preference" => Ok(Self::Preference),
            "decision" => Ok(Self::Decision),
            "code_context" | "codecontext" => Ok(Self::CodeContext),
            "conversation" => Ok(Self::Conversation),
            "synthesis" => Ok(Self::Synthesis),
            _ => Err(format!("无效的记忆类型: '{}'，有效值: {:?}", s, Self::valid_values())),
        }
    }
}

/// 重要性等级
///
/// 新类型包装 u8，确保值在 1-10 之间。
/// 重要性影响记忆衰减速度和检索优先级。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Importance(u8);

impl Importance {
    /// 默认重要性
    pub const DEFAULT: Self = Self(5);
    /// 最低重要性
    pub const MIN: Self = Self(1);
    /// 最高重要性
    pub const MAX: Self = Self(10);

    /// 创建重要性等级，自动 clamp 到 [1, 10]
    pub fn new(value: u8) -> Self {
        Self(value.clamp(1, 10))
    }

    /// 获取内部值
    pub fn value(&self) -> u8 {
        self.0
    }

    /// 数值越接近 10 越重要
    pub fn is_high(&self) -> bool {
        self.0 >= 8
    }

    /// 数值越接近 1 越不重要
    pub fn is_low(&self) -> bool {
        self.0 <= 3
    }
}

impl Default for Importance {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl From<u8> for Importance {
    fn from(v: u8) -> Self {
        Self::new(v)
    }
}

/// 隐私权限级别（Section 3.3 隐私与权限）
///
/// 支持三级隔离：Session 级、User 级、Global 级。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyLevel {
    /// Session 级：仅当前会话可见，会话结束后自动隔离
    Session,
    /// User 级：同一用户的所有会话共享
    #[default]
    User,
    /// Global 级：所有用户可见的全局知识
    Global,
}

impl PrivacyLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::User => "user",
            Self::Global => "global",
        }
    }

    pub fn try_parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "session" => Some(Self::Session),
            "user" => Some(Self::User),
            "global" => Some(Self::Global),
            _ => None,
        }
    }
}

/// 通用记忆项
///
/// 这是面向 AI 助手的通用永久记忆单元，不局限于代码。
/// 通过 MCP 协议暴露给 AI 助手进行读写操作。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// 唯一标识符（UUID v4）
    pub id: String,
    /// 记忆内容（自然语言或代码文本）
    pub content: String,
    /// 记忆类型
    pub memory_type: MemoryType,
    /// 关联项目名称（None 表示全局记忆）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// 标签列表（检索时可按标签过滤）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// 重要性（1-10，默认 5）
    pub importance: Importance,
    /// 存活天数（None 或 0 = 永久）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_days: Option<u32>,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 最后更新时间
    pub updated_at: DateTime<Utc>,
    /// 最后访问时间（用于衰减计算）
    pub last_accessed: DateTime<Utc>,
    /// 记忆来源（如 "mcp_client", "code_indexer"）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// 合成来源记忆 ID 列表（仅 Synthesis 类型，递归合成时记录源记忆）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_ids: Vec<String>,
    /// 合成置信度（0.0 ~ 1.0，仅 Synthesis 类型）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// 洛书 9 维坐标向量（序列化为 JSON 数组）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub luoshu_vector: Option<[f32; 9]>,
    /// 先天八卦分类索引（0-7，由 MirrorProject 自动判定）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bagua_index: Option<u8>,
    /// 先天八卦分类名称
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bagua_category: Option<String>,
    /// 隐私权限级别（Session/User/Global，默认 User）
    #[serde(default)]
    pub privacy_level: PrivacyLevel,
    /// 会话 ID（privacy_level=Session 时必填，用于隔离作用域）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 用户 ID（privacy_level=User/Global 时用于用户级隔离）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// 拓扑深度（0.0 ~ 1.0，0.0=九宫格中心/永久，1.0=边缘/快速衰减）
    /// 由洛书编码器自动计算：九宫格位置越靠近中心（太极），深度越小，衰减越慢
    #[serde(default = "default_topological_depth")]
    pub topological_depth: f32,
    /// 版本号（用于冲突解决追踪，每次修正 +1）
    #[serde(default)]
    pub version: u32,
    /// 历史版本内容列表（仅保留最近 5 个版本，用于版本回溯）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub version_history: Vec<MemoryVersion>,
}

/// 记忆版本快照（用于版本回溯）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryVersion {
    /// 版本号
    pub version: u32,
    /// 该版本的内容
    pub content: String,
    /// 更新时间
    pub updated_at: DateTime<Utc>,
    /// 变更原因（如 "用户修正: 信息已过时"）
    pub reason: Option<String>,
}

fn default_topological_depth() -> f32 {
    0.5 // 默认中等深度
}

impl Memory {
    /// 创建新的记忆项
    pub fn new(
        content: String,
        memory_type: MemoryType,
        project: Option<String>,
        tags: Vec<String>,
        importance: Importance,
        ttl_days: Option<u32>,
    ) -> Self {
        let now = Utc::now();
        let id = uuid::Uuid::new_v4().to_string();
        Self {
            id,
            content,
            memory_type,
            project,
            tags,
            importance,
            ttl_days,
            created_at: now,
            updated_at: now,
            last_accessed: now,
            source: None,
            source_ids: Vec::new(),
            confidence: None,
            luoshu_vector: None,
            bagua_index: None,
            bagua_category: None,
            privacy_level: PrivacyLevel::default(),
            session_id: None,
            user_id: None,
            topological_depth: default_topological_depth(),
            version: 1,
            version_history: Vec::new(),
        }
    }

    /// 设置记忆来源
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// 设置隐私权限
    pub fn with_privacy(
        mut self,
        level: PrivacyLevel,
        session_id: Option<String>,
        user_id: Option<String>,
    ) -> Self {
        self.privacy_level = level;
        self.session_id = session_id;
        self.user_id = user_id;
        self
    }

    /// 设置拓扑深度（由洛书编码器在 remember 时自动计算）
    pub fn with_topological_depth(mut self, depth: f32) -> Self {
        self.topological_depth = depth.clamp(0.0, 1.0);
        self
    }

    /// 是否已过期（基于 ttl_days）
    pub fn is_expired(&self) -> bool {
        if let Some(days) = self.ttl_days {
            if days == 0 {
                return false;
            }
            let deadline = self.created_at + chrono::Duration::days(days as i64);
            Utc::now() > deadline
        } else {
            false
        }
    }

    /// 记录一次访问（更新 last_accessed）
    pub fn mark_accessed(&mut self) {
        self.last_accessed = Utc::now();
    }

    /// 更新内容（自动增加版本号并保存历史版本）
    pub fn update_content(&mut self, new_content: String) {
        // 保存当前版本到历史
        if self.version_history.len() >= 5 {
            self.version_history.remove(0); // 只保留最近 5 个版本
        }
        self.version_history.push(MemoryVersion {
            version: self.version,
            content: std::mem::replace(&mut self.content, new_content),
            updated_at: self.updated_at,
            reason: None,
        });
        self.version += 1;
        self.updated_at = Utc::now();
    }

    /// 更新内容并记录原因（用于用户修正场景）
    pub fn update_content_with_reason(&mut self, new_content: String, reason: String) {
        if self.version_history.len() >= 5 {
            self.version_history.remove(0);
        }
        self.version_history.push(MemoryVersion {
            version: self.version,
            content: std::mem::replace(&mut self.content, new_content),
            updated_at: self.updated_at,
            reason: Some(reason),
        });
        self.version += 1;
        self.updated_at = Utc::now();
    }

    /// 更新重要性
    pub fn update_importance(&mut self, importance: Importance) {
        self.importance = importance;
        self.updated_at = Utc::now();
    }

    /// 刷新更新时间和访问时间（用于合并等场景）
    pub fn touch(&mut self) {
        let now = Utc::now();
        self.updated_at = now;
        self.last_accessed = now;
    }

    /// 计算记忆衰减因子（0.0 ~ 1.0），使用默认衰减配置
    ///
    /// 基于双重衰减模型，详见 `decay_factor_with_config`。
    pub fn decay_factor(&self) -> f32 {
        self.decay_factor_with_config(&DecayConfig::default())
    }

    /// 计算记忆衰减因子（0.0 ~ 1.0），使用自定义衰减配置
    ///
    /// 基于双重衰减模型：
    ///
    /// 1. **时间衰减**: `e^(-decay_rate * days_since_access / importance)`
    ///    - 重要性越高，衰减越慢（importance=10 几乎不衰减）
    ///    - 距上次访问越久，衰减越严重
    ///
    /// 2. **拓扑深度衰减** (Section 3.3 衰减与遗忘):
    ///    - 九宫格中心（太极位，topological_depth ≈ 0）为永久记忆，几乎不衰减
    ///    - 九宫格边缘（topological_depth ≈ 1）快速衰减
    ///    - 合成因子: `1.0 - topological_depth * topo_weight`
    ///
    /// 最终衰减因子 = 时间衰减 × 拓扑保护
    ///
    /// 参数可通过 `DecayConfig` 配置：
    /// - `decay_rate`: 时间衰减速度（默认 0.05），越大衰减越快
    /// - `topo_weight`: 拓扑深度影响程度（默认 0.3），越大拓扑影响越大
    ///   * 中心节点(depth=0): 保护因子=1.0，完全不受拓扑影响
    ///   * 边缘节点(depth=1): 保护因子=1.0-topo_weight，受拓扑衰减
    pub fn decay_factor_with_config(&self, config: &DecayConfig) -> f32 {
        // 时间衰减分量
        let now = Utc::now();
        let hours_since = (now - self.last_accessed).num_hours() as f64;
        let days_since = hours_since / 24.0;

        let time_factor = if days_since <= 0.0 {
            1.0
        } else {
            let importance = self.importance.value() as f64;
            let decay_rate = config.decay_rate as f64;
            let exponent = -decay_rate * days_since / importance;
            exponent.exp() as f32
        };

        // 拓扑保护因子：中心节点受保护，边缘节点加速衰减
        let topo_protection = 1.0 - self.topological_depth * config.topo_weight;

        // 合成衰减因子
        let factor = time_factor * topo_protection;
        factor.clamp(0.0, 1.0)
    }

    /// 计算衰减后的重要性分值（使用默认衰减配置）
    ///
    /// `importance * decay_factor`，用于检索排序。
    pub fn decayed_importance(&self) -> f32 {
        self.decayed_importance_with_config(&DecayConfig::default())
    }

    /// 计算衰减后的重要性分值（使用自定义衰减配置）
    pub fn decayed_importance_with_config(&self, config: &DecayConfig) -> f32 {
        self.importance.value() as f32 * self.decay_factor_with_config(config)
    }

    /// 生成简洁的描述行
    pub fn summary(&self) -> String {
        let prefix = match self.memory_type {
            MemoryType::Fact => "[事实]",
            MemoryType::Preference => "[偏好]",
            MemoryType::Decision => "[决策]",
            MemoryType::CodeContext => "[代码]",
            MemoryType::Conversation => "[对话]",
            MemoryType::Synthesis => "[合成]",
        };
        let content_preview: String = self
            .content
            .chars()
            .take(80)
            .collect();
        let ellipsis = if self.content.chars().count() > 80 {
            "..."
        } else {
            ""
        };
        format!("{} {}{}", prefix, content_preview, ellipsis)
    }
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_type_from_str() {
        assert_eq!(MemoryType::try_parse("fact"), Some(MemoryType::Fact));
        assert_eq!(MemoryType::try_parse("FACT"), Some(MemoryType::Fact));
        assert_eq!(MemoryType::try_parse("preference"), Some(MemoryType::Preference));
        assert_eq!(MemoryType::try_parse("code_context"), Some(MemoryType::CodeContext));
        assert_eq!(MemoryType::try_parse("codecontext"), Some(MemoryType::CodeContext));
        assert_eq!(MemoryType::try_parse("unknown"), None);
    }

    #[test]
    fn test_memory_type_as_str() {
        assert_eq!(MemoryType::Fact.as_str(), "fact");
        assert_eq!(MemoryType::CodeContext.as_str(), "code_context");
    }

    #[test]
    fn test_importance_clamp() {
        let imp = Importance::new(15);
        assert_eq!(imp.value(), 10);

        let imp = Importance::new(0);
        assert_eq!(imp.value(), 1);

        let imp = Importance::new(5);
        assert_eq!(imp.value(), 5);
    }

    #[test]
    fn test_importance_is_high_low() {
        assert!(Importance::new(9).is_high());
        assert!(!Importance::new(5).is_high());
        assert!(Importance::new(2).is_low());
        assert!(!Importance::new(5).is_low());
    }

    #[test]
    fn test_memory_new() {
        let m = Memory::new(
            "用户偏好使用 pnpm 作为包管理器".to_string(),
            MemoryType::Preference,
            Some("loong".into()),
            vec!["pnpm".into(), "tooling".into()],
            Importance::new(8),
            None,
        );

        assert_eq!(m.memory_type, MemoryType::Preference);
        assert_eq!(m.project, Some("loong".into()));
        assert_eq!(m.tags.len(), 2);
        assert_eq!(m.importance.value(), 8);
        assert!(m.ttl_days.is_none());
        assert!(!m.is_expired());
        assert_eq!(m.source, None);
    }

    #[test]
    fn test_memory_expiry() {
        let ttl_days = Some(0u32);
        let m = Memory::new(
            "测试".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::default(),
            ttl_days,
        );
        // ttl_days=0 表示永不过期（语义同 None）
        assert!(!m.is_expired());
    }

    #[test]
    fn test_memory_summary() {
        let m = Memory::new(
            "用户偏好使用 Rust 作为主要开发语言，用于系统编程和 Web 服务开发".to_string(),
            MemoryType::Preference,
            None,
            vec![],
            Importance::default(),
            None,
        );
        let s = m.summary();
        assert!(s.starts_with("[偏好]"));
    }

    #[test]
    fn test_memory_update_content() {
        let mut m = Memory::new(
            "旧内容".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::default(),
            None,
        );
        let old_updated = m.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(10));
        m.update_content("新内容".to_string());
        assert_eq!(m.content, "新内容");
        assert!(m.updated_at > old_updated);
    }

    #[test]
    fn test_memory_with_source() {
        let m = Memory::new(
            "测试".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::default(),
            None,
        )
        .with_source("mcp_client");

        assert_eq!(m.source, Some("mcp_client".into()));
    }

    #[test]
    fn test_serde_roundtrip() {
        let m = Memory::new(
            "用户偏好 pnpm".to_string(),
            MemoryType::Preference,
            Some("loong".into()),
            vec!["pnpm".into()],
            Importance::new(7),
            Some(365),
        );

        let json = serde_json::to_string(&m).expect("序列化失败");
        let restored: Memory = serde_json::from_str(&json).expect("反序列化失败");

        assert_eq!(restored.id, m.id);
        assert_eq!(restored.content, m.content);
        assert_eq!(restored.memory_type, m.memory_type);
        assert_eq!(restored.importance, m.importance);
        assert_eq!(restored.ttl_days, m.ttl_days);
    }

    #[test]
    fn test_decay_factor_fresh() {
        // 刚创建的记忆，拓扑深度默认 0.5
        // 衰减因子 = 时间衰减(1.0) × 拓扑保护(1.0 - 0.5*0.3 = 0.85) = 0.85
        let m = Memory::new(
            "测试".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        let factor = m.decay_factor();
        // 新鲜记忆因拓扑深度默认为 0.5，衰减因子应为 0.85
        assert!((factor - 0.85).abs() < 0.01, "新鲜记忆默认拓扑深度 0.5，衰减因子 ≈ 0.85: {}", factor);
    }

    #[test]
    fn test_decay_factor_high_importance_resists_decay() {
        // 高重要性记忆（10）衰减更慢
        let mut m = Memory::new(
            "高重要性".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(10),
            None,
        );
        // 模拟 100 天前的访问
        m.last_accessed = Utc::now() - chrono::Duration::days(100);

        let factor = m.decay_factor();
        // importance=10, 100天后: e^(-0.05 * 100 / 10) = e^(-0.5) ≈ 0.607
        assert!(factor > 0.5, "高重要性应抵抗衰减: {}", factor);
    }

    #[test]
    fn test_decay_factor_low_importance_decays_fast() {
        // 低重要性记忆（1）衰减更快
        let mut m = Memory::new(
            "低重要性".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(1),
            None,
        );
        // 模拟 30 天前的访问
        m.last_accessed = Utc::now() - chrono::Duration::days(30);

        let factor = m.decay_factor();
        // importance=1, 30天后: e^(-0.05 * 30 / 1) = e^(-1.5) ≈ 0.223
        assert!(factor < 0.5, "低重要性应快速衰减: {}", factor);
    }

    #[test]
    fn test_decayed_importance() {
        let mut m = Memory::new(
            "测试".to_string(),
            MemoryType::Fact,
            None,
            vec![],
            Importance::new(5),
            None,
        );
        m.last_accessed = Utc::now() - chrono::Duration::days(20);

        let decayed = m.decayed_importance();
        // importance=5, 20天后: 5 * e^(-0.05 * 20 / 5) = 5 * e^(-0.2) ≈ 5 * 0.819 = 4.09
        assert!(decayed < 5.0, "衰减后应低于原始重要性: {}", decayed);
        assert!(decayed > 3.0, "不应衰减过猛: {}", decayed);
    }
}