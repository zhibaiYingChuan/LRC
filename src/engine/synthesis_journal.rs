// ============================================================
// 许可证: DaoTi Research License v1.0
// 本文件实现合成日志与质量反馈闭环，属于守护层 (Layer 2)。
// ============================================================
//
// 合成日志 (SynthesisJournal)
//
// 记录每次递归合成的触发条件、参与记忆、合成结果，
// 并在后续检索中验证合成质量，形成"合成→验证→校正"的闭环。
//
// 核心功能：
//   - 记录合成事件（触发源、参与记忆、置信度）
//   - 检索后验证合成记忆的相关性
//   - 统计合成触发频率和成功率
//   - 通过 MCP/v1 API 暴露日志数据

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

/// 单次合成事件记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisEvent {
    /// 合成产生的记忆 ID
    pub synthesis_id: String,
    /// 触发源：remember（写入后）或 recall（检索后）
    pub trigger_source: String,
    /// 八卦类别
    pub bagua_category: String,
    /// 八卦类别索引（0-7）
    pub bagua_index: u8,
    /// 参与合成的原始记忆 ID 列表
    pub source_ids: Vec<String>,
    /// 合成置信度 (0.0 ~ 1.0)
    pub confidence: f32,
    /// 参与记忆数量
    pub member_count: usize,
    /// 合成时间戳（Unix 毫秒）
    pub timestamp_ms: u64,
    /// 质量反馈：后续检索中该合成记忆被命中的次数
    pub hit_count: u64,
    /// 质量反馈：被命中时的平均相关性评分
    pub avg_relevance: f32,
    /// 是否被标记为低质量合成
    pub low_quality: bool,
}

/// 合成日志管理器
///
/// 线程安全的事件记录器，支持最多保留 1000 条历史记录。
/// 提供质量反馈闭环：检索命中合成记忆时自动更新相关性评分。
#[derive(Debug)]
pub struct SynthesisJournal {
    /// 合成事件历史（FIFO 队列，最多 1000 条）
    events: Mutex<VecDeque<SynthesisEvent>>,
    /// 累计合成次数
    total_synthesis: Mutex<u64>,
    /// 被标记为低质量的合成次数
    low_quality_count: Mutex<u64>,
}

impl SynthesisJournal {
    /// 创建新的合成日志
    pub fn new() -> Self {
        Self {
            events: Mutex::new(VecDeque::with_capacity(1000)),
            total_synthesis: Mutex::new(0),
            low_quality_count: Mutex::new(0),
        }
    }

    /// 记录一次合成事件
    ///
    /// 注意：参数较多（8 个）因为需要完整记录合成上下文。
    /// 后续重构时可考虑将参数封装为 SynthesisRecord 结构体。
    #[allow(clippy::too_many_arguments)]
    pub fn record_synthesis(
        &self,
        synthesis_id: String,
        trigger_source: &str,
        bagua_category: &str,
        bagua_index: u8,
        source_ids: Vec<String>,
        confidence: f32,
        member_count: usize,
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = SynthesisEvent {
            synthesis_id,
            trigger_source: trigger_source.to_string(),
            bagua_category: bagua_category.to_string(),
            bagua_index,
            source_ids,
            confidence,
            member_count,
            timestamp_ms: now,
            hit_count: 0,
            avg_relevance: 0.0,
            low_quality: false,
        };

        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if events.len() >= 1000 {
            events.pop_front();
        }
        events.push_back(event);

        let mut total = self
            .total_synthesis
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *total += 1;
    }

    /// 记录合成记忆被检索命中（质量反馈）
    ///
    /// 当检索结果中包含合成记忆时调用，更新其命中次数和相关性评分。
    pub fn record_hit(&self, synthesis_id: &str, relevance: f32) {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        for event in events.iter_mut().rev() {
            if event.synthesis_id == synthesis_id {
                // 增量更新平均相关性
                let total = event.hit_count as f32 * event.avg_relevance;
                event.hit_count += 1;
                event.avg_relevance = (total + relevance) / event.hit_count as f32;

                // 连续多次命中但相关性低 → 标记为低质量
                if event.hit_count >= 3 && event.avg_relevance < 0.3 {
                    event.low_quality = true;
                    let mut lq = self
                        .low_quality_count
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    *lq += 1;
                }
                return;
            }
        }
    }

    /// 获取合成触发频率（每分钟合成次数）
    pub fn synthesis_rate_per_minute(&self) -> f32 {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if events.len() < 2 {
            return 0.0;
        }
        let first_ts = events.front().map(|e| e.timestamp_ms).unwrap_or(0);
        let last_ts = events.back().map(|e| e.timestamp_ms).unwrap_or(0);
        let duration_minutes = (last_ts - first_ts) as f32 / 60_000.0;
        if duration_minutes <= 0.0 {
            return 0.0;
        }
        events.len() as f32 / duration_minutes
    }

    /// 获取所有合成事件
    pub fn get_events(&self) -> Vec<SynthesisEvent> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.iter().cloned().collect()
    }

    /// 获取最近 N 条合成事件
    pub fn get_recent_events(&self, n: usize) -> Vec<SynthesisEvent> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.iter().rev().take(n).cloned().collect()
    }

    /// 获取所有被标记为低质量的合成记忆 ID 列表
    ///
    /// 用于垃圾清理：找出需要被清理/降级的合成产物。
    pub fn get_low_quality_ids(&self) -> Vec<String> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events
            .iter()
            .filter(|e| e.low_quality)
            .map(|e| e.synthesis_id.clone())
            .collect()
    }

    /// 清除指定合成事件的跟踪记录（记忆被清理后调用）
    pub fn remove_event(&self, synthesis_id: &str) {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.retain(|e| e.synthesis_id != synthesis_id);
    }
}

impl Default for SynthesisJournal {
    fn default() -> Self {
        Self::new()
    }
}

/// 合成日志统计快照（用于 API 响应）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisJournalSnapshot {
    /// 累计合成总次数
    pub total_synthesis: u64,
    /// 低质量合成次数
    pub low_quality_count: u64,
    /// 合成触发频率（次/分钟）
    pub synthesis_rate_per_minute: f32,
    /// 最近 10 条合成事件
    pub recent_events: Vec<SynthesisEvent>,
    /// 合成成功率（1.0 - 低质量比例）
    pub success_rate: f32,
}

impl SynthesisJournal {
    /// 采集日志快照
    pub fn snapshot(&self) -> SynthesisJournalSnapshot {
        let total = *self
            .total_synthesis
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let low_quality = *self
            .low_quality_count
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let success_rate = if total > 0 {
            1.0 - (low_quality as f32 / total as f32)
        } else {
            1.0
        };

        SynthesisJournalSnapshot {
            total_synthesis: total,
            low_quality_count: low_quality,
            synthesis_rate_per_minute: self.synthesis_rate_per_minute(),
            recent_events: self.get_recent_events(10),
            success_rate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_hit() {
        let journal = SynthesisJournal::new();

        journal.record_synthesis(
            "synth_001".into(),
            "remember",
            "离",
            1,
            vec!["mem_1".into(), "mem_2".into(), "mem_3".into()],
            0.75,
            3,
        );

        let events = journal.get_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].synthesis_id, "synth_001");
        assert_eq!(events[0].trigger_source, "remember");
        assert_eq!(events[0].confidence, 0.75);
        assert_eq!(events[0].member_count, 3);

        // 记录命中
        journal.record_hit("synth_001", 0.9);
        journal.record_hit("synth_001", 0.85);

        let events = journal.get_events();
        assert_eq!(events[0].hit_count, 2);
        assert!((events[0].avg_relevance - 0.875).abs() < 0.01);
        assert!(!events[0].low_quality);
    }

    #[test]
    fn test_low_quality_detection() {
        let journal = SynthesisJournal::new();

        journal.record_synthesis(
            "synth_bad".into(),
            "recall",
            "坎",
            7,
            vec!["m1".into()],
            0.2,
            1,
        );

        // 连续 3 次低相关性命中 → 标记为低质量
        journal.record_hit("synth_bad", 0.1);
        journal.record_hit("synth_bad", 0.15);
        journal.record_hit("synth_bad", 0.2);

        let events = journal.get_events();
        assert!(events[0].low_quality, "低相关性合成应被标记为低质量");
        assert_eq!(events[0].hit_count, 3);
    }

    #[test]
    fn test_snapshot() {
        let journal = SynthesisJournal::new();

        journal.record_synthesis("s1".into(), "remember", "震", 3, vec!["a".into()], 0.8, 2);
        journal.record_synthesis("s2".into(), "recall", "兑", 5, vec!["b".into()], 0.6, 3);

        let snapshot = journal.snapshot();
        assert_eq!(snapshot.total_synthesis, 2);
        assert_eq!(snapshot.recent_events.len(), 2);
        assert_eq!(snapshot.success_rate, 1.0);
    }
}
