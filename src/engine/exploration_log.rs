// ============================================================
// 结构化探索日志模块（Exploration Log）
// ============================================================
//
// 用途：为"开放探索赛题"提供科学探索过程的结构化日志记录。
// 输出 JSON Lines 格式，每行一个事件，便于后续分析与可视化。
//
// 集成方式：
//   1. 将本文件复制到 src/engine/exploration_log.rs
//   2. 在 src/engine/mod.rs 添加: pub mod exploration_log;
//   3. 在 MemoryStore/SynthesisEngine 等关键方法中调用 ExplorationLogger
//
// 许可证：Apache 2.0（参赛公开层）

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// 探索日志事件类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExplorationEventType {
    /// 实验配置（启动时记录一次）
    ExperimentConfig,
    /// Sidecar 启动完成
    SidecarStarted,
    /// Sidecar 停止
    SidecarStopped,
    /// 写入单条记忆
    Remember,
    /// 批量写入记忆
    BatchRemember,
    /// 检索记忆
    Recall,
    /// 合成触发
    Synthesize,
    /// 拆解触发
    Unfold,
    /// 道同构度调节
    Regulate,
    /// 记忆回收
    Gc,
    /// 定期状态快照
    Snapshot,
    /// 自定义事件
    Custom(String),
}

impl ExplorationEventType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::ExperimentConfig => "experiment_config",
            Self::SidecarStarted => "sidecar_started",
            Self::SidecarStopped => "sidecar_stopped",
            Self::Remember => "remember",
            Self::BatchRemember => "batch_remember",
            Self::Recall => "recall",
            Self::Synthesize => "synthesize",
            Self::Unfold => "unfold",
            Self::Regulate => "regulate",
            Self::Gc => "gc",
            Self::Snapshot => "snapshot",
            Self::Custom(s) => s.as_str(),
        }
    }
}

/// 深度分布快照（九宫格位置分布）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthDistribution {
    /// 中心位置（太极位）记忆数
    pub center: usize,
    /// 近中心位置记忆数
    pub near_center: usize,
    /// 边缘位置记忆数
    pub edge: usize,
}

/// 八卦分布快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaguaDistribution {
    pub qian: usize,  // 乾卦·天
    pub kun: usize,   // 坤卦·地
    pub zhen: usize,  // 震卦·雷
    pub xun: usize,   // 巽卦·风
    pub kan: usize,   // 坎卦·水
    pub li: usize,    // 离卦·火
    pub gen: usize,   // 艮卦·山
    pub dui: usize,   // 兑卦·泽
}

/// 状态快照 payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotPayload {
    /// 记忆总数
    pub memory_count: usize,
    /// 深度分布
    pub depth_distribution: DepthDistribution,
    /// 八卦分布
    pub bagua_distribution: BaguaDistribution,
}

/// 性能指标
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Metrics {
    /// 延迟（毫秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// 结果数量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_count: Option<usize>,
    /// 合成置信度
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// 信息增量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub information_gain: Option<f32>,
    /// 道同构度偏离
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_deviation: Option<f32>,
    /// 几何中心偏移（9维向量）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center_offset: Option<Vec<f32>>,
    /// 合成比率
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synthesis_ratio: Option<f32>,
    /// 平均衰减因子
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decay_factor_avg: Option<f32>,
}

/// 探索日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplorationLogEntry {
    /// Unix 时间戳（秒，含小数毫秒）
    pub timestamp: f64,
    /// 事件类型
    pub event_type: String,
    /// 实验 ID
    pub experiment_id: String,
    /// 会话 ID（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 事件 payload（任意 JSON）
    pub payload: serde_json::Value,
    /// 性能指标
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Metrics>,
}

/// 探索日志记录器（线程安全）
pub struct ExplorationLogger {
    /// 输出文件路径
    log_path: PathBuf,
    /// 实验 ID
    experiment_id: String,
    /// 会话 ID
    session_id: Option<String>,
    /// 文件句柄（Mutex 保护）
    file: Mutex<Option<File>>,
    /// 是否启用
    enabled: bool,
}

impl ExplorationLogger {
    /// 创建新的日志记录器
    pub fn new(log_path: PathBuf, experiment_id: String) -> Self {
        Self {
            log_path,
            experiment_id,
            session_id: None,
            file: Mutex::new(None),
            enabled: true,
        }
    }

    /// 设置会话 ID
    pub fn with_session_id(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// 禁用日志（用于生产模式）
    pub fn disabled() -> Self {
        Self {
            log_path: PathBuf::new(),
            experiment_id: String::new(),
            session_id: None,
            file: Mutex::new(None),
            enabled: false,
        }
    }

    /// 初始化文件句柄（懒加载）
    fn ensure_file(&self) -> std::io::Result<()> {
        let mut guard = self.file.lock().map_err(|e| {
            std::io::Error::other(format!("锁失败: {}", e))
        })?;
        if guard.is_none() {
            // 确保父目录存在
            if let Some(parent) = self.log_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)?;
            *guard = Some(file);
        }
        Ok(())
    }

    /// 记录一条日志
    pub fn log(
        &self,
        event_type: ExplorationEventType,
        payload: serde_json::Value,
        metrics: Option<Metrics>,
    ) {
        if !self.enabled {
            return;
        }
        if let Err(e) = self._log_internal(event_type, payload, metrics) {
            eprintln!("[探索日志] 写入失败: {}", e);
        }
    }

    fn _log_internal(
        &self,
        event_type: ExplorationEventType,
        payload: serde_json::Value,
        metrics: Option<Metrics>,
    ) -> std::io::Result<()> {
        self.ensure_file()?;

        let entry = ExplorationLogEntry {
            timestamp: Utc::now().timestamp_millis() as f64 / 1000.0,
            event_type: event_type.as_str().to_string(),
            experiment_id: self.experiment_id.clone(),
            session_id: self.session_id.clone(),
            payload,
            metrics,
        };

        let line = serde_json::to_string(&entry)
            .map_err(std::io::Error::other)?;
        let line = line + "\n";

        let mut guard = self.file.lock().map_err(|e| {
            std::io::Error::other(format!("锁失败: {}", e))
        })?;
        if let Some(ref mut file) = *guard {
            file.write_all(line.as_bytes())?;
            file.flush()?;
        }
        Ok(())
    }

    /// 便捷方法：记录 remember 事件
    pub fn log_remember(
        &self,
        memory_type: &str,
        importance: u8,
        tags: &[String],
        latency_ms: u64,
    ) {
        self.log(
            ExplorationEventType::Remember,
            serde_json::json!({
                "memory_type": memory_type,
                "importance": importance,
                "tags": tags,
            }),
            Some(Metrics {
                latency_ms: Some(latency_ms),
                ..Default::default()
            }),
        );
    }

    /// 便捷方法：记录 recall 事件
    pub fn log_recall(
        &self,
        query: &str,
        top_k: usize,
        result_count: usize,
        latency_ms: u64,
    ) {
        // 截断 query 防止日志膨胀
        let truncated_query = if query.len() > 200 { &query[..200] } else { query };
        self.log(
            ExplorationEventType::Recall,
            serde_json::json!({
                "query": truncated_query,
                "top_k": top_k,
            }),
            Some(Metrics {
                latency_ms: Some(latency_ms),
                result_count: Some(result_count),
                ..Default::default()
            }),
        );
    }

    /// 便捷方法：记录 synthesize 事件
    pub fn log_synthesize(
        &self,
        source_ids: &[String],
        result_id: &str,
        synthesis_time_ms: u64,
        confidence: f32,
        information_gain: f32,
    ) {
        self.log(
            ExplorationEventType::Synthesize,
            serde_json::json!({
                "source_ids": source_ids,
                "result_id": result_id,
            }),
            Some(Metrics {
                latency_ms: Some(synthesis_time_ms),
                confidence: Some(confidence),
                information_gain: Some(information_gain),
                ..Default::default()
            }),
        );
    }

    /// 便捷方法：记录 snapshot 事件
    pub fn log_snapshot(&self, payload: SnapshotPayload, metrics: Metrics) {
        self.log(
            ExplorationEventType::Snapshot,
            serde_json::to_value(payload).unwrap_or(serde_json::Value::Null),
            Some(metrics),
        );
    }

    /// 便捷方法：记录实验配置
    pub fn log_experiment_config(&self, config: serde_json::Value) {
        self.log(
            ExplorationEventType::ExperimentConfig,
            config,
            None,
        );
    }

    /// 便捷方法：记录 sidecar 启动
    pub fn log_sidecar_started(&self, startup_time_ms: u64, cmd: &str) {
        self.log(
            ExplorationEventType::SidecarStarted,
            serde_json::json!({
                "startup_time_ms": startup_time_ms,
                "cmd": cmd,
            }),
            None,
        );
    }

    /// 便捷方法：记录 sidecar 停止
    pub fn log_sidecar_stopped(&self, uptime_ms: u64) {
        self.log(
            ExplorationEventType::SidecarStopped,
            serde_json::json!({
                "uptime_ms": uptime_ms,
            }),
            None,
        );
    }

    /// 便捷方法：记录 regulate 事件（道同构度调节）
    ///
    /// 在 DaoRegulator::regulate() 调用前后埋点，记录调节动作和系统状态。
    pub fn log_regulate(
        &self,
        dao_score: f32,
        bagua_entropy: f32,
        synthesis_ratio: f32,
        avg_deviation: f32,
        action: &str,
        latency_ms: u64,
    ) {
        self.log(
            ExplorationEventType::Regulate,
            serde_json::json!({
                "dao_score": dao_score,
                "bagua_entropy": bagua_entropy,
                "synthesis_ratio": synthesis_ratio,
                "avg_deviation": avg_deviation,
                "action": action,
            }),
            Some(Metrics {
                latency_ms: Some(latency_ms),
                avg_deviation: Some(avg_deviation),
                synthesis_ratio: Some(synthesis_ratio),
                ..Default::default()
            }),
        );
    }

    /// 便捷方法：记录 gc 事件（记忆回收）
    pub fn log_gc(&self, reclaimed: usize, remaining: usize, latency_ms: u64) {
        self.log(
            ExplorationEventType::Gc,
            serde_json::json!({
                "reclaimed": reclaimed,
                "remaining": remaining,
            }),
            Some(Metrics {
                latency_ms: Some(latency_ms),
                result_count: Some(reclaimed),
                ..Default::default()
            }),
        );
    }
}

impl Default for ExplorationLogger {
    fn default() -> Self {
        Self::disabled()
    }
}

// ============================================================
// 单元测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_log_remember_writes_jsonl() {
        // 准备临时目录
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("exploration.jsonl");
        let logger = ExplorationLogger::new(
            log_path.clone(),
            "test_experiment".to_string(),
        );

        // 写入一条 remember 事件
        logger.log_remember("fact", 8, &["test".to_string()], 42);

        // 验证文件内容
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("remember"));
        assert!(content.contains("test_experiment"));
        assert!(content.contains("\"latency_ms\":42"));
        assert!(content.ends_with('\n'));
    }

    #[test]
    fn test_log_recall_includes_query_and_count() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("recall.jsonl");
        let logger = ExplorationLogger::new(
            log_path.clone(),
            "test_recall".to_string(),
        );

        logger.log_recall("如何实现用户认证", 10, 5, 120);

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("recall"));
        assert!(content.contains("如何实现用户认证"));
        assert!(content.contains("\"result_count\":5"));
    }

    #[test]
    fn test_log_snapshot_serializes_distribution() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("snapshot.jsonl");
        let logger = ExplorationLogger::new(
            log_path.clone(),
            "test_snapshot".to_string(),
        );

        let payload = SnapshotPayload {
            memory_count: 1000,
            depth_distribution: DepthDistribution {
                center: 200,
                near_center: 400,
                edge: 400,
            },
            bagua_distribution: BaguaDistribution {
                qian: 125, kun: 125, zhen: 125, xun: 125,
                kan: 125, li: 125, gen: 125, dui: 125,
            },
        };

        let metrics = Metrics {
            avg_deviation: Some(0.42),
            synthesis_ratio: Some(0.18),
            ..Default::default()
        };

        logger.log_snapshot(payload, metrics);

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("snapshot"));
        assert!(content.contains("\"memory_count\":1000"));
        assert!(content.contains("\"center\":200"));
        assert!(content.contains("\"qian\":125"));
        assert!(content.contains("\"avg_deviation\":0.42"));
    }

    #[test]
    fn test_disabled_logger_writes_nothing() {
        let logger = ExplorationLogger::disabled();
        logger.log_remember("fact", 5, &[], 10);
        // 禁用模式下不应写入任何文件
        // （无法验证文件不存在，但不应 panic）
    }

    #[test]
    fn test_log_synthesize_includes_confidence() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("synth.jsonl");
        let logger = ExplorationLogger::new(
            log_path.clone(),
            "test_synth".to_string(),
        );

        logger.log_synthesize(
            &["id-1".to_string(), "id-2".to_string()],
            "result-id",
            350,
            0.87,
            0.42,
        );

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("synthesize"));
        assert!(content.contains("\"confidence\":0.87"));
        assert!(content.contains("\"information_gain\":0.42"));
        assert!(content.contains("id-1"));
    }

    #[test]
    fn test_long_query_is_truncated() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("trunc.jsonl");
        let logger = ExplorationLogger::new(
            log_path.clone(),
            "test_trunc".to_string(),
        );

        // 构造超长 query
        let long_query = "a".repeat(500);
        logger.log_recall(&long_query, 5, 1, 50);

        let content = std::fs::read_to_string(&log_path).unwrap();
        // query 应被截断到 200 字符
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        let query = parsed["payload"]["query"].as_str().unwrap();
        assert!(query.len() <= 200);
    }
}
