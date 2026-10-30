//! 记忆生命周期状态（取代 / 衰减 / 抑制）的**独立**持久层。
//!
//! ## 为什么必须独立于 memories.json
//!
//! 1. **回滚安全**：`Memory` 结构没有 `deny_unknown_fields`，旧版 exe 读到新增字段
//!    会静默忽略，并在下一次全量写入时把它们抹掉 ⇒ 一旦把寿命字段塞进
//!    `memories.json`，回滚旧版就变成单向门。
//! 2. **写放大**：`memories.json` 的整库压实是全量序列化（632MB 级）。
//!    检索命中要更新 `last_hit_at`，若落到同一个文件，每次检索都会触发全库写，
//!    线上已有因此产生 503 风暴的历史。
//!
//! 因此寿命状态单独存 `lifecycle.json`：体积小、只含变更过的条目、原子写。
//!
//! ## 确定性约束
//!
//! 所有与时间有关的方法都**显式接收 `now_ms`**，不读系统时钟 —— 同一
//! `(状态表, now_ms)` 必须给出逐位相同的结果，否则检索结果无法 A/B 比对
//! （违反"检索结果确定性"这条既有不变量）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 生命周期表的格式版本（独立于 `memories.json`）。
pub const LIFECYCLE_VERSION: u32 = 1;

/// 检索命中记录的默认合并间隔：内存累积，最多每 60s 落一次盘。
pub const DEFAULT_HIT_FLUSH_INTERVAL_MS: u64 = 60_000;

/// 衰减半衰期（天）：30 天未命中衰减到 0.5。
pub const DEFAULT_DECAY_HALF_LIFE_DAYS: f64 = 30.0;

/// 衰减下限：无论多久没命中，都不低于该系数（避免"永久埋没"，保留可发现性）。
pub const MIN_DECAY_FACTOR: f32 = 0.2;

/// 检索命中是否写入寿命表。**默认关闭**，开关 `LRC_LIFECYCLE_HITS=1`。
///
/// 为什么默认关：这是唯一落在**检索热路径**上的新增逻辑。默认关闭时，
/// 检索路径与改造前完全一致，便于把"可靠性修复"与"寿命能力"分开上线。
pub fn hits_enabled() -> bool {
    std::env::var("LRC_LIFECYCLE_HITS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// 衰减是否启用。**默认关闭**，开关 `LRC_LIFECYCLE_DECAY=1`。
///
/// 为什么默认关：衰减会改变检索排序 ⇒ 必须先与冻结基线做 A/B（集合 + 位次），
/// 确认 MRR 不下降才能默认开启。默认关闭时，检索行为与改造前**逐字节一致**。
pub fn decay_enabled() -> bool {
    std::env::var("LRC_LIFECYCLE_DECAY")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// 当前毫秒时间戳。
///
/// 只在一次检索的入口调用一次，之后所有 `decay_factor` 共用同一 `now_ms` ——
/// 保证同一请求内排序确定（不同请求之间按时间推进属预期）。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn default_version() -> u32 {
    LIFECYCLE_VERSION
}

/// 一条记忆的生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    /// 正常可检索（缺省）。
    #[default]
    Active,
    /// 已被更新的事实取代：默认从结果中排除，显式请求时才返回。
    Superseded,
    /// 被用户/规则抑制：默认降权但保留可发现性。
    Suppressed,
}

/// 单条记忆的寿命记录。所有字段带 `#[serde(default)]`，旧文件缺字段可直接读。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LifecycleEntry {
    #[serde(default)]
    pub state: LifecycleState,
    /// 取代它的新记忆 ID（仅 `Superseded` 时非空）。
    #[serde(default)]
    pub superseded_by: Option<String>,
    /// 最近一次被检索命中的时间（毫秒时间戳，0 表示从未命中）。
    #[serde(default)]
    pub last_hit_at_ms: u64,
    /// 累计命中次数（用于后续"越常用越稳"的策略，当前仅记录）。
    #[serde(default)]
    pub hit_count: u64,
    /// 本条记录最后一次变更时间。
    #[serde(default)]
    pub updated_at_ms: u64,
}

/// 生命周期表：`记忆ID → 寿命记录`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleTable {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub entries: HashMap<String, LifecycleEntry>,
}

impl Default for LifecycleTable {
    fn default() -> Self {
        Self {
            version: LIFECYCLE_VERSION,
            entries: HashMap::new(),
        }
    }
}

impl LifecycleTable {
    /// 读取一条记录（不存在返回 `None`，语义为"从未记录 = Active 且不衰减"）。
    pub fn get(&self, memory_id: &str) -> Option<&LifecycleEntry> {
        self.entries.get(memory_id)
    }

    /// 记录一次检索命中；返回 `true` 表示状态确有变化。
    pub fn touch(&mut self, memory_id: &str, now_ms: u64) -> bool {
        let entry = self.entries.entry(memory_id.to_string()).or_default();
        entry.last_hit_at_ms = now_ms;
        entry.hit_count = entry.hit_count.saturating_add(1);
        entry.updated_at_ms = now_ms;
        true
    }

    /// 标记旧记忆被新记忆取代。
    pub fn mark_superseded(&mut self, old_id: &str, new_id: &str, now_ms: u64) -> bool {
        let entry = self.entries.entry(old_id.to_string()).or_default();
        if entry.state == LifecycleState::Superseded
            && entry.superseded_by.as_deref() == Some(new_id)
        {
            return false;
        }
        entry.state = LifecycleState::Superseded;
        entry.superseded_by = Some(new_id.to_string());
        entry.updated_at_ms = now_ms;
        true
    }

    /// 标记为被抑制。
    pub fn mark_suppressed(&mut self, memory_id: &str, now_ms: u64) -> bool {
        let entry = self.entries.entry(memory_id.to_string()).or_default();
        if entry.state == LifecycleState::Suppressed {
            return false;
        }
        entry.state = LifecycleState::Suppressed;
        entry.updated_at_ms = now_ms;
        true
    }

    /// 清除记录（恢复为 Active 且不衰减）。
    pub fn clear_entry(&mut self, memory_id: &str) -> bool {
        self.entries.remove(memory_id).is_some()
    }

    /// 是否应从默认结果中排除（仅 `Superseded`）。
    pub fn is_excluded_by_default(&self, memory_id: &str) -> bool {
        matches!(
            self.entries.get(memory_id).map(|e| e.state),
            Some(LifecycleState::Superseded)
        )
    }

    /// 排序乘子，取值 `[MIN_DECAY_FACTOR, 1.0]`。
    ///
    /// - 无记录：返回 `1.0`（不惩罚未知，保持与改造前一致）；
    /// - 从未命中（`last_hit_at_ms == 0`）：返回 `1.0`（无信息即不判罚）；
    /// - 其余：按"距上次命中的天数 / 半衰期"做指数衰减，并夹到下限；
    /// - `Suppressed`：额外乘 0.5。
    ///
    /// 纯函数：只依赖 `(entry, now_ms)`，不读系统时钟。
    pub fn decay_factor(&self, memory_id: &str, now_ms: u64) -> f32 {
        let Some(entry) = self.entries.get(memory_id) else {
            return 1.0;
        };
        let mut factor = 1.0f32;
        if entry.last_hit_at_ms > 0 && now_ms > entry.last_hit_at_ms {
            let days = (now_ms - entry.last_hit_at_ms) as f64 / 86_400_000.0;
            let half_life = DEFAULT_DECAY_HALF_LIFE_DAYS.max(1.0);
            factor = 0.5f32.powf((days / half_life) as f32);
        }
        if entry.state == LifecycleState::Suppressed {
            factor *= 0.5;
        }
        factor.clamp(MIN_DECAY_FACTOR, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: u64 = 86_400_000;

    #[test]
    fn unknown_memory_is_not_decayed() {
        let table = LifecycleTable::default();
        assert_eq!(table.decay_factor("never-seen", 1_000), 1.0);
        assert!(!table.is_excluded_by_default("never-seen"));
    }

    #[test]
    fn never_hit_entry_is_not_decayed() {
        let mut table = LifecycleTable::default();
        table.mark_suppressed("m1", 0);
        table.entries.get_mut("m1").unwrap().state = LifecycleState::Active;
        assert_eq!(table.decay_factor("m1", 10 * DAY_MS), 1.0);
    }

    #[test]
    fn half_life_decay_is_deterministic() {
        let mut table = LifecycleTable::default();
        table.touch("m1", 0);
        table.entries.get_mut("m1").unwrap().last_hit_at_ms = 1;
        let now = 1 + (30 * DAY_MS);
        let a = table.decay_factor("m1", now);
        let b = table.decay_factor("m1", now);
        assert!((a - 0.5).abs() < 0.01, "30 天应衰减到约 0.5，实际 {}", a);
        assert_eq!(a, b, "同一 (状态, now) 必须给出逐位相同的结果");
    }

    #[test]
    fn decay_has_floor() {
        let mut table = LifecycleTable::default();
        table.touch("m1", 0);
        table.entries.get_mut("m1").unwrap().last_hit_at_ms = 1;
        let far = 1 + 3650 * DAY_MS;
        assert_eq!(table.decay_factor("m1", far), MIN_DECAY_FACTOR);
    }

    #[test]
    fn superseded_is_excluded_and_idempotent() {
        let mut table = LifecycleTable::default();
        assert!(table.mark_superseded("old", "new", 100));
        assert!(table.is_excluded_by_default("old"));
        assert_eq!(
            table.get("old").unwrap().superseded_by.as_deref(),
            Some("new")
        );
        assert!(
            !table.mark_superseded("old", "new", 200),
            "相同取代关系应幂等"
        );
    }

    #[test]
    fn old_file_without_fields_is_readable() {
        let json = r#"{"entries":{"m1":{}}}"#;
        let table: LifecycleTable = serde_json::from_str(json).expect("缺 version 与字段应可读");
        assert_eq!(table.version, LIFECYCLE_VERSION);
        assert_eq!(table.get("m1").unwrap().state, LifecycleState::Active);
    }
}
