// ============================================================
// 许可证: Apache 2.0
// 本文件实现后台结晶流水线，属于公开层 (Layer 1)。
// ============================================================
//
// 后台结晶流水线（Background Consolidation Pipeline）
//
// 后台结晶流水线：
//   定时从表层记忆系统拉取新记忆，
//   经由洛书编码 → 八卦分类 → 递归合成，将表层记忆结晶为永久记忆。
//
// 核心组件：
//   1. ConsolidationPipeline — 主流水线，协调编码→分类→合成全流程
//   2. ConsolidationConfig — 可配置的流水线参数（轮询间隔、合成阈值等）
//   3. SurfaceMemorySource — 表层记忆数据源 trait（可对接任意表层记忆系统）
//   4. run_consolidation_loop — 后台 tokio 任务入口

#[cfg(not(feature = "ml"))]
use crate::engine::luoshu_encoder::LuoShuEncoder as HybridLuoShuEncoder;
#[cfg(feature = "ml")]
use crate::engine::luoshu_encoder_ml::HybridLuoShuEncoder;
use crate::memory_store::MemoryStore;
use crate::memory_types::{Importance, Memory, MemoryType, PrivacyLevel};
use crate::persistence::Persistence;
use crate::persistence::PersistenceError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};

// ==================== 配置类型 ====================

/// 后台结晶流水线配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    /// 轮询间隔（秒），默认 300 秒（5 分钟）
    pub poll_interval_secs: u64,
    /// 每轮最大处理记忆数，默认 100
    pub batch_size: usize,
    /// 合成触发阈值：同类记忆达到此数量时触发递归合成，默认 5
    pub synthesis_threshold: usize,
    /// 合成相似度阈值：记忆相似度超过此值时纳入同一簇，默认 0.4
    pub synthesis_similarity: f32,
    /// 是否在启动时立即运行一次，默认 true
    pub run_on_start: bool,
    /// 是否启用自动合成，默认 true
    pub auto_synthesize: bool,
    /// 日志详细程度：0=静默, 1=摘要, 2=详细
    pub verbose: u8,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 300,
            batch_size: 100,
            synthesis_threshold: 5,
            synthesis_similarity: 0.4,
            run_on_start: true,
            auto_synthesize: true,
            verbose: 1,
        }
    }
}

// ==================== 表层记忆数据源 trait ====================

/// 从表层记忆系统拉取的原始记忆条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceMemory {
    /// 记忆内容（自然语言文本）
    pub content: String,
    /// 记忆类型标识（如 "fact", "preference", "decision"）
    #[serde(default = "default_surface_type")]
    pub memory_type: String,
    /// 重要性（1-10）
    #[serde(default = "default_surface_importance")]
    pub importance: u8,
    /// 关联项目
    pub project: Option<String>,
    /// 标签
    #[serde(default)]
    pub tags: Vec<String>,
    /// 来源会话 ID
    pub session_id: Option<String>,
    /// 来源用户 ID
    pub user_id: Option<String>,
    /// 来源时间戳
    pub timestamp: Option<DateTime<Utc>>,
    /// 源系统标识（如 "in_memory", "api"）
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_surface_type() -> String {
    "fact".into()
}
fn default_surface_importance() -> u8 {
    5
}
fn default_source() -> String {
    "surface_memory".into()
}

/// 表层记忆数据源抽象（可接入任意表层记忆系统或 HTTP API）
///
/// 实现此 trait 即可将任意表层记忆系统接入结晶流水线。
#[async_trait::async_trait]
pub trait SurfaceMemorySource: Send + Sync {
    /// 获取自指定时间以来的新记忆
    async fn get_memories_since(
        &self,
        since: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<SurfaceMemory>, String>;

    /// 获取数据源名称（用于日志和指标）
    fn source_name(&self) -> &str;
}

// ==================== 静态内存数据源（测试用） ====================

/// 基于内存列表的静态数据源，适用于测试和批量导入
pub struct InMemorySource {
    name: String,
    memories: Vec<SurfaceMemory>,
    #[allow(dead_code)]
    cursor: usize,
}

impl InMemorySource {
    /// 创建静态数据源
    pub fn new(name: impl Into<String>, memories: Vec<SurfaceMemory>) -> Self {
        Self {
            name: name.into(),
            memories,
            cursor: 0,
        }
    }
}

#[async_trait::async_trait]
impl SurfaceMemorySource for InMemorySource {
    async fn get_memories_since(
        &self,
        since: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<SurfaceMemory>, String> {
        let filtered: Vec<SurfaceMemory> = self
            .memories
            .iter()
            .filter(|m| {
                m.timestamp.map(|t| t > since).unwrap_or(true) // 无时间戳的视为新记忆
            })
            .take(limit)
            .cloned()
            .collect();
        Ok(filtered)
    }

    fn source_name(&self) -> &str {
        &self.name
    }
}

// ==================== 合并统计信息 ====================

/// 单轮结晶周期的运行统计
#[derive(Debug, Clone, Default, Serialize)]
pub struct CycleStats {
    /// 本轮拉取的原始记忆数
    pub fetched: usize,
    /// 成功编码的记忆数
    pub encoded: usize,
    /// 成功写入的记忆数
    pub stored: usize,
    /// 触发合成的簇数
    pub clusters_found: usize,
    /// 新生成的合成记忆数
    pub synthesized: usize,
    /// 失败的记忆数
    pub failed: usize,
    /// 本轮耗时（毫秒）
    pub elapsed_ms: u64,
    /// 最近一次运行时间
    pub last_run: Option<DateTime<Utc>>,
}

// ==================== 结晶流水线 ====================

/// 后台结晶流水线
///
/// 协调从表层记忆拉取到永久记忆结晶的全流程。
pub struct ConsolidationPipeline<P: Persistence> {
    /// 配置参数
    config: ConsolidationConfig,
    /// 记忆存储
    store: Arc<Mutex<MemoryStore<P>>>,
    /// 洛书编码器（保留供未来直接编码使用）
    #[allow(dead_code)]
    luoshu_encoder: HybridLuoShuEncoder,
    /// 上次运行时间（用于增量拉取）
    last_run: DateTime<Utc>,
    /// 累积统计
    pub total_stats: CycleStats,
}

impl<P: Persistence + Send + 'static> ConsolidationPipeline<P> {
    /// 创建新的结晶流水线
    pub fn new(config: ConsolidationConfig, store: Arc<Mutex<MemoryStore<P>>>) -> Self {
        Self {
            config,
            store,
            luoshu_encoder: HybridLuoShuEncoder::default(),
            last_run: Utc::now(),
            total_stats: CycleStats::default(),
        }
    }

    /// 单轮结晶周期
    ///
    /// 从数据源拉取新记忆 → 洛书编码 → 八卦分类 → 写入 → 触发合成。
    /// 返回本轮统计信息。
    pub async fn run_cycle(
        &mut self,
        source: &dyn SurfaceMemorySource,
    ) -> Result<CycleStats, PersistenceError> {
        let cycle_start = std::time::Instant::now();
        let mut stats = CycleStats::default();

        // 1. 拉取新记忆
        let surface_memories = source
            .get_memories_since(self.last_run, self.config.batch_size)
            .await
            .map_err(|e| {
                PersistenceError::Io(std::io::Error::other(format!("拉取表层记忆失败: {}", e)))
            })?;

        stats.fetched = surface_memories.len();

        // v0.5.4 P2-10 修复：即使没有新的表层记忆，也继续执行合成步骤
        // 原因：用户通过 HTTP API 直接写入的记忆可能存在重复，需要定期合并
        // 仅跳过存储循环，但合成检查必须始终运行
        if surface_memories.is_empty() {
            if self.config.verbose >= 2 {
                eprintln!("[LRC·结晶] 无新表层记忆，仅执行合成检查");
            }
        } else if self.config.verbose >= 2 {
            eprintln!(
                "[LRC·结晶] 拉取到 {} 条表层记忆 (来源: {})",
                stats.fetched,
                source.source_name()
            );
        }

        // 2. 逐条处理：洛书编码 → MirrorProject 分类 → 写入
        let mut store = self.store.lock().await;

        for sm in &surface_memories {
            let memory_type = MemoryType::try_parse(&sm.memory_type).unwrap_or(MemoryType::Fact);
            let privacy_level = PrivacyLevel::try_parse(
                sm.session_id.as_ref().map(|_| "session").unwrap_or("user"),
            )
            .unwrap_or_default();

            let memory = Memory::new(
                sm.content.clone(),
                memory_type,
                sm.project.clone(),
                sm.tags.clone(),
                Importance::new(sm.importance),
                None, // 由洛书编码器决定拓扑深度，而非 TTL
            )
            .with_source(format!("consolidation:{}", sm.source))
            .with_privacy(privacy_level, sm.session_id.clone(), sm.user_id.clone());

            match store.remember(memory) {
                Ok(_) => {
                    stats.stored += 1;
                    stats.encoded += 1; // remember 内部自动完成洛书编码
                }
                Err(e) => {
                    stats.failed += 1;
                    if self.config.verbose >= 1 {
                        eprintln!("[LRC·结晶] 写入失败: {} (内容: {:.40}...)", e, sm.content);
                    }
                }
            }
        }

        // 3. 洛书驱动递归合成（按八卦类别分组融合）
        if self.config.auto_synthesize {
            // 临时提高合成阈值
            let old_threshold = store.synthesis_min_cluster;
            let old_similarity = store.synthesis_similarity;
            store.synthesis_min_cluster = self.config.synthesis_threshold;
            store.synthesis_similarity = self.config.synthesis_similarity;

            match store.luoshu_synthesize() {
                Ok(n) => {
                    stats.synthesized = n;
                    if n > 0 && self.config.verbose >= 1 {
                        eprintln!("[LRC·结晶] 洛书合成完成，生成 {} 条合成记忆", n);
                    }
                }
                Err(e) => {
                    if self.config.verbose >= 1 {
                        eprintln!("[LRC·结晶] 合成失败: {}", e);
                    }
                }
            }

            // 恢复原始阈值
            store.synthesis_min_cluster = old_threshold;
            store.synthesis_similarity = old_similarity;
        }

        drop(store); // 释放锁

        // 4. 更新统计
        stats.last_run = Some(Utc::now());
        stats.elapsed_ms = cycle_start.elapsed().as_millis() as u64;

        // 累积全局统计
        self.total_stats.fetched += stats.fetched;
        self.total_stats.encoded += stats.encoded;
        self.total_stats.stored += stats.stored;
        self.total_stats.synthesized += stats.synthesized;
        self.total_stats.failed += stats.failed;
        self.total_stats.last_run = stats.last_run;

        self.last_run = Utc::now();

        if self.config.verbose >= 1 {
            eprintln!(
                "[LRC·结晶] 周期完成: 拉取={}, 写入={}, 合成={}, 失败={}, 耗时={}ms",
                stats.fetched, stats.stored, stats.synthesized, stats.failed, stats.elapsed_ms
            );
        }

        Ok(stats)
    }
}

// ==================== 后台循环入口 ====================

/// 启动后台结晶循环
///
/// 这是一个异步任务，每 `config.poll_interval_secs` 秒执行一次结晶周期。
/// 循环会在 `shutdown_signal` 被触发时优雅停止。
///
/// # 参数
/// - `pipeline`: 结晶流水线实例
/// - `source`: 表层记忆数据源
/// - `shutdown_signal`: 停止信号接收器（`tokio::sync::watch::Receiver<bool>`）
pub async fn run_consolidation_loop<P: Persistence + Send + 'static>(
    mut pipeline: ConsolidationPipeline<P>,
    source: Arc<dyn SurfaceMemorySource>,
    mut shutdown_signal: tokio::sync::watch::Receiver<bool>,
) {
    let poll_duration = Duration::from_secs(pipeline.config.poll_interval_secs);
    let run_on_start = pipeline.config.run_on_start;

    eprintln!(
        "[LRC·结晶] 后台流水线已启动 (间隔={}s, 批大小={}, 阈值={})",
        pipeline.config.poll_interval_secs,
        pipeline.config.batch_size,
        pipeline.config.synthesis_threshold
    );

    // 启动时立即运行一次
    if run_on_start {
        match pipeline.run_cycle(source.as_ref()).await {
            Ok(stats) => {
                eprintln!(
                    "[LRC·结晶] 初始周期完成: 处理 {} 条, 合成 {} 条",
                    stats.stored, stats.synthesized
                );
            }
            Err(e) => {
                eprintln!("[LRC·结晶] 初始周期失败: {}", e);
            }
        }
    }

    let mut ticker = interval(poll_duration);

    loop {
        tokio::select! {
            // 检查关闭信号
            _ = shutdown_signal.changed() => {
                if *shutdown_signal.borrow() {
                    eprintln!("[LRC·结晶] 收到关闭信号，停止流水线");
                    break;
                }
            }
            // 定时触发
            _ = ticker.tick() => {
                match pipeline.run_cycle(source.as_ref()).await {
                    Ok(stats) => {
                        if pipeline.config.verbose >= 1 && stats.fetched > 0 {
                            eprintln!(
                                "[LRC·结晶] 定时周期完成: 处理 {} 条",
                                stats.stored
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("[LRC·结晶] 定时周期失败: {}", e);
                    }
                }
            }
        }
    }

    // 停止前输出累积统计
    eprintln!(
        "[LRC·结晶] 流水线已停止。累积统计: 拉取={}, 存储={}, 合成={}",
        pipeline.total_stats.fetched, pipeline.total_stats.stored, pipeline.total_stats.synthesized
    );
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::create_json_persistence;
    use crate::persistence::json::JsonPersistence;
    use tempfile::TempDir;

    /// 创建测试用 MemoryStore
    fn make_store() -> (TempDir, MemoryStore<JsonPersistence>) {
        let dir = TempDir::new().expect("应创建临时目录");
        let data_dir = dir.path().to_string_lossy().to_string();
        let p = create_json_persistence(&data_dir).expect("持久化创建失败");
        (dir, MemoryStore::new(p))
    }

    /// 创建测试用结晶流水线
    fn make_pipeline(
        store: Arc<Mutex<MemoryStore<JsonPersistence>>>,
    ) -> ConsolidationPipeline<JsonPersistence> {
        let config = ConsolidationConfig {
            poll_interval_secs: 3600, // 测试中用不到
            batch_size: 10,
            synthesis_threshold: 2, // 降低阈值便于触发合成
            synthesis_similarity: 0.3,
            run_on_start: false,
            auto_synthesize: true,
            verbose: 0,
        };
        ConsolidationPipeline::new(config, store)
    }

    /// 测试：单轮结晶周期基本流程
    #[tokio::test]
    async fn test_consolidation_cycle_basic() {
        let (_dir, store) = make_store();
        let store = Arc::new(Mutex::new(store));
        let mut pipeline = make_pipeline(store.clone());

        let source = InMemorySource::new(
            "test",
            vec![
                SurfaceMemory {
                    content: "用户偏好使用 Rust 编程语言".into(),
                    memory_type: "preference".into(),
                    importance: 8,
                    project: None,
                    tags: vec!["rust".into()],
                    session_id: Some("sess-1".into()),
                    user_id: Some("user-1".into()),
                    timestamp: Some(Utc::now()),
                    source: "test".into(),
                },
                SurfaceMemory {
                    content: "项目使用 PostgreSQL 数据库".into(),
                    memory_type: "fact".into(),
                    importance: 7,
                    project: Some("loong".into()),
                    tags: vec!["database".into(), "postgresql".into()],
                    session_id: None,
                    user_id: Some("user-1".into()),
                    timestamp: Some(Utc::now()),
                    source: "test".into(),
                },
            ],
        );

        let stats = pipeline.run_cycle(&source).await.expect("周期应成功");
        assert_eq!(stats.fetched, 2);
        assert_eq!(stats.stored, 2);
        assert_eq!(stats.failed, 0);

        // 验证记忆已存储（不含合成记忆）
        let store = store.lock().await;
        let all = store
            .list_memories(&crate::memory_store::ListFilter::new())
            .unwrap();
        let non_synthesis: Vec<_> = all
            .0
            .iter()
            .filter(|m| m.memory_type != MemoryType::Synthesis)
            .collect();
        assert_eq!(
            non_synthesis.len(),
            2,
            "应有 2 条非合成记忆，实际: {:?}",
            all.0.iter().map(|m| &m.content).collect::<Vec<_>>()
        );
    }

    /// 测试：自动洛书合成触发
    #[tokio::test]
    async fn test_consolidation_triggers_synthesis() {
        let (_dir, store) = make_store();
        let store = Arc::new(Mutex::new(store));
        let mut pipeline = make_pipeline(store.clone());

        // 准备 3 条关于数据库的相似记忆（应触发合成）
        let memories: Vec<SurfaceMemory> = vec![
            "PostgreSQL 数据库配置优化",
            "PostgreSQL 连接池管理策略",
            "PostgreSQL 索引优化最佳实践",
        ]
        .into_iter()
        .map(|content| SurfaceMemory {
            content: content.into(),
            memory_type: "fact".into(),
            importance: 6,
            project: Some("loong".into()),
            tags: vec!["postgresql".into(), "database".into()],
            session_id: None,
            user_id: Some("user-1".into()),
            timestamp: Some(Utc::now()),
            source: "test-synthesis".into(),
        })
        .collect();

        let source = InMemorySource::new("test-synthesis", memories);
        let stats = pipeline.run_cycle(&source).await.expect("周期应成功");
        assert_eq!(stats.fetched, 3);
        assert_eq!(stats.stored, 3);

        // 应触发了合成（3 条相似 PostgresSQL 记忆）
        let store = store.lock().await;
        let all = store
            .list_memories(&crate::memory_store::ListFilter::new())
            .unwrap();
        let synthesis_count = all
            .0
            .iter()
            .filter(|m| m.memory_type == MemoryType::Synthesis)
            .count();
        assert!(
            synthesis_count >= 1,
            "应有至少 1 条合成记忆，实际: {}",
            synthesis_count
        );
    }

    /// 测试：静态数据源时间过滤
    #[tokio::test]
    async fn test_source_time_filtering() {
        let old_time = Utc::now() - chrono::Duration::hours(2);
        let new_time = Utc::now();

        let source = InMemorySource::new(
            "test-filter",
            vec![
                SurfaceMemory {
                    content: "旧记忆".into(),
                    memory_type: "fact".into(),
                    importance: 5,
                    project: None,
                    tags: vec![],
                    session_id: None,
                    user_id: None,
                    timestamp: Some(old_time),
                    source: "test".into(),
                },
                SurfaceMemory {
                    content: "新记忆".into(),
                    memory_type: "fact".into(),
                    importance: 5,
                    project: None,
                    tags: vec![],
                    session_id: None,
                    user_id: None,
                    timestamp: Some(new_time),
                    source: "test".into(),
                },
            ],
        );

        // since = 1 小时前，应只返回新记忆
        let since = Utc::now() - chrono::Duration::hours(1);
        let result = source
            .get_memories_since(since, 10)
            .await
            .expect("应成功拉取");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "新记忆");
    }
}
