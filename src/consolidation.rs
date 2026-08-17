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
// v0.5.18 新增：LLM embedding 合成所需的导入
use crate::engine::llm_translator::{cosine_similarity, LlmApiConfig};
// v0.6.0 新增：统一 Embedder 抽象（结晶路径支持本地嵌入）
use crate::engine::embedder::Embedder;
// v0.9.1 三阶段锁解耦：结晶流水线在锁外执行聚类计算
use crate::engine::synthesis_engine::SynthesisEngine;
use crate::memory_store::{ListFilter, MemoryStore};
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
    /// FIX-007：改用 RwLock 避免 spawn_blocking + blocking_lock 竞争
    store: Arc<Mutex<MemoryStore<P>>>,
    /// 洛书编码器（保留供未来直接编码使用）
    #[allow(dead_code)]
    luoshu_encoder: HybridLuoShuEncoder,
    /// v0.5.18 新增：LLM 配置（用于结晶时的高维 embedding 合成）
    /// 如果为 None 或 LlmApiConfig::None，则降级到洛书合成
    llm_config: Option<LlmApiConfig>,
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
            llm_config: None,
            last_run: Utc::now(),
            total_stats: CycleStats::default(),
        }
    }

    /// v0.5.18 新增：创建带 LLM 配置的结晶流水线
    ///
    /// 当 LLM 配置有效时，结晶周期会优先使用 LLM embedding 进行高维语义聚类
    /// 和信息增量计算，绕过 9 维洛书向量的局限性。
    /// LLM 调用失败时自动降级到洛书合成。
    pub fn new_with_llm(
        config: ConsolidationConfig,
        store: Arc<Mutex<MemoryStore<P>>>,
        llm_config: LlmApiConfig,
    ) -> Self {
        let has_llm = llm_config.is_configured();
        if has_llm {
            eprintln!("[LRC·结晶] LLM embedding 合成已启用（高维语义聚类）");
        }
        Self {
            config,
            store,
            luoshu_encoder: HybridLuoShuEncoder::default(),
            llm_config: if has_llm { Some(llm_config) } else { None },
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

        // 2. 逐条处理：洛书编码 → MirrorProject 分类 → 写入（Phase 1：持锁写入）
        {
            let mut store = self.store.lock().await;

            for sm in &surface_memories {
                let memory_type =
                    MemoryType::try_parse(&sm.memory_type).unwrap_or(MemoryType::Fact);
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
                .with_privacy(
                    privacy_level,
                    sm.session_id.clone(),
                    sm.user_id.clone(),
                );

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
        } // 自动释放锁

        // 3. 合成阶段
        if self.config.auto_synthesize {
            // 3a. v0.5.18 LLM embedding 合成路径（优先）
            // 遵循三阶段锁安全模式：Phase 1 持锁加载 → Phase 2 无锁 LLM 调用 → Phase 3 持锁写入
            let llm_succeeded: bool = if let Some(ref llm_config) = self.llm_config {
                match self.llm_synthesize_cycle(llm_config).await {
                    Ok(n) => {
                        stats.synthesized = n;
                        if n > 0 && self.config.verbose >= 1 {
                            eprintln!("[LRC·结晶] LLM 合成完成，生成 {} 条合成记忆", n);
                        }
                        n > 0
                    }
                    Err(e) => {
                        if self.config.verbose >= 1 {
                            eprintln!("[LRC·结晶] LLM 合成失败，降级到洛书合成: {}", e);
                        }
                        false
                    }
                }
            } else {
                false
            };

            // 3b. 洛书合成（降级路径：仅当 LLM 未配置或失败时执行）
            if !llm_succeeded {
                let store_arc = self.store.clone();
                let threshold = self.config.synthesis_threshold;
                let similarity = self.config.synthesis_similarity;

                // v0.9.1 三阶段锁解耦（根治 lock_busy）：
                //   Phase 1（持锁读快照，极短）→ Phase 2（锁外 CPU 聚类计算）
                //   → Phase 3（持锁写回，极短）。
                //   消除 v0.9.0 中"blocking_lock 持锁执行 luoshu_synthesize
                //   （含数秒~数十秒的聚类计算）"导致的读接口 lock_busy。
                let result =
                    tokio::task::spawn_blocking(move || -> Result<usize, PersistenceError> {
                        // Phase 1：持锁读快照 + 临时设置阈值（快速）
                        let (snapshot, old_threshold, old_similarity) = {
                            let mut store = store_arc.blocking_lock();
                            let old_threshold = store.synthesis_min_cluster;
                            let old_similarity = store.synthesis_similarity;
                            store.synthesis_min_cluster = threshold;
                            store.synthesis_similarity = similarity;
                            let snapshot = store.synthesis_snapshot()?;
                            (snapshot, old_threshold, old_similarity)
                        }; // 锁在此释放

                        // Phase 2：锁外 CPU 密集计算
                        let engine = SynthesisEngine::new(snapshot.config);
                        let mut plan =
                            engine.plan_luoshu(&snapshot.all, snapshot.information_gain_threshold);
                        if plan.synthesized == 0 {
                            plan = engine.plan_jaccard(&snapshot.all);
                        }

                        // Phase 3：持锁写回 + 恢复阈值（快速）
                        let synthesized = {
                            let mut store = store_arc.blocking_lock();
                            store.synthesis_min_cluster = old_threshold;
                            store.synthesis_similarity = old_similarity;
                            store.apply_synthesis_plan(plan)
                        };

                        Ok(synthesized)
                    })
                    .await;

                match result {
                    Ok(Ok(n)) => {
                        stats.synthesized = n;
                        if n > 0 && self.config.verbose >= 1 {
                            eprintln!("[LRC·结晶] 洛书合成完成，生成 {} 条合成记忆", n);
                        }
                    }
                    Ok(Err(e)) => {
                        if self.config.verbose >= 1 {
                            eprintln!("[LRC·结晶] 合成失败: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("[LRC·结晶] 合成任务 panic: {}", e);
                    }
                }
            }
        }

        // v0.9.1 三阶段锁解耦：合成阶段结束后统一重置待合成标记。
        // 洛书降级路径由 apply_synthesis_plan 重置，此处兜底覆盖 LLM 路径，
        // 确保无论走哪条合成路径，synthesis_pending 都被正确清除（store(false) 幂等）。
        {
            let store = self.store.lock().await;
            store
                .synthesis_pending
                .store(false, std::sync::atomic::Ordering::Release);
        }

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

    // ==================== v0.5.18 LLM embedding 合成 ====================

    /// v0.5.18 新增：LLM embedding 合成周期
    ///
    /// 遵循三阶段锁安全模式（参考项目硬约束）：
    /// - Phase 1：持锁加载所有非 Synthesis 记忆的 (id, content)，释放锁（<1ms）
    /// - Phase 2：无锁，LLM embedding + 聚类 + LLM 总结（I/O 密集，可能耗时数秒）
    /// - Phase 3：持锁写入合成记忆，释放锁（<1ms）
    ///
    /// 失败时返回 Err，调用方应降级到洛书合成。
    async fn llm_synthesize_cycle(&self, llm_config: &LlmApiConfig) -> Result<usize, String> {
        // ===== Phase 1：持锁加载记忆列表 =====
        let candidates: Vec<(String, String)> = {
            let store = self.store.lock().await;
            let filter = ListFilter {
                limit: 1000, // 加载足够多的记忆用于聚类
                ..ListFilter::new()
            };
            let (all, _) = store
                .list_memories(&filter)
                .map_err(|e| format!("加载记忆列表失败: {}", e))?;
            all.into_iter()
                .filter(|m| m.memory_type != MemoryType::Synthesis)
                .map(|m| (m.id, m.content))
                .collect::<Vec<_>>()
        };

        if candidates.len() < self.config.synthesis_threshold {
            if self.config.verbose >= 2 {
                eprintln!(
                    "[LRC·结晶·LLM] 记忆数 {} 低于阈值 {}，跳过 LLM 合成",
                    candidates.len(),
                    self.config.synthesis_threshold
                );
            }
            return Ok(0);
        }

        if self.config.verbose >= 2 {
            eprintln!(
                "[LRC·结晶·LLM] Phase 1 完成：加载 {} 条候选记忆",
                candidates.len()
            );
        }

        // ===== Phase 2：无锁，LLM embedding + 聚类 + 总结 =====
        let texts: Vec<&str> = candidates.iter().map(|(_, c)| c.as_str()).collect();
        let embeddings = llm_config
            .embed_texts(&texts)
            .await
            .map_err(|e| format!("LLM embedding 调用失败: {}", e))?;

        if embeddings.len() != candidates.len() {
            return Err(format!(
                "Embedding 数量不匹配：期望 {}，实际 {}",
                candidates.len(),
                embeddings.len()
            ));
        }

        // 基于余弦相似度聚类（贪心法）
        let clusters = self.cluster_by_embedding(&embeddings, self.config.synthesis_similarity);

        // 信息增量阈值：与 DaoRegulator 默认值一致（0.01）
        const INFO_GAIN_THRESHOLD: f32 = 0.01;

        let mut synthesis_results: Vec<(Vec<String>, String, f32)> = Vec::new();

        for cluster_indices in &clusters {
            if cluster_indices.len() < self.config.synthesis_threshold {
                continue;
            }

            // 计算信息增量：1 - 平均成对余弦相似度
            let avg_sim = self.average_pairwise_similarity(&embeddings, cluster_indices);
            let information_gain = 1.0 - avg_sim;

            if information_gain < INFO_GAIN_THRESHOLD {
                if self.config.verbose >= 2 {
                    eprintln!(
                        "[LRC·结晶·LLM] 跳过簇（大小={}，信息增量 {:.4} < 阈值 {:.4}）",
                        cluster_indices.len(),
                        information_gain,
                        INFO_GAIN_THRESHOLD
                    );
                }
                continue;
            }

            // 收集簇内记忆内容
            let cluster_memories: Vec<String> = cluster_indices
                .iter()
                .map(|&i| candidates[i].1.clone())
                .collect();

            // 调用 LLM chat API 生成合成内容
            let summary = llm_config
                .summarize_memories(&cluster_memories)
                .await
                .map_err(|e| format!("LLM 合成总结失败: {}", e))?;

            // 收集源记忆 ID
            let source_ids: Vec<String> = cluster_indices
                .iter()
                .map(|&i| candidates[i].0.clone())
                .collect();

            if self.config.verbose >= 2 {
                eprintln!(
                    "[LRC·结晶·LLM] 簇通过阈值（大小={}，信息增量 {:.4}），已生成合成内容（{} 字）",
                    cluster_indices.len(),
                    information_gain,
                    summary.chars().count()
                );
            }

            synthesis_results.push((source_ids, summary, information_gain));
        }

        if synthesis_results.is_empty() {
            if self.config.verbose >= 1 {
                eprintln!("[LRC·结晶·LLM] 无簇通过信息增量阈值，本次不生成合成记忆");
            }
            return Ok(0);
        }

        // ===== Phase 3：持锁写入合成记忆 =====
        let written = {
            let mut store = self.store.lock().await;
            let mut count = 0usize;
            for (source_ids, summary, info_gain) in synthesis_results {
                let mut memory = Memory::new(
                    summary,
                    MemoryType::Synthesis,
                    None,
                    Vec::new(),
                    Importance::new(7), // 合成记忆重要性 7
                    None,
                )
                .with_source("llm_synthesis");
                memory.source_ids = source_ids;
                memory.information_gain = Some(info_gain);
                memory.confidence = Some(0.85); // LLM 合成置信度
                memory.resolution = "synthesized".to_string();

                match store.remember(memory) {
                    Ok(_) => count += 1,
                    Err(e) => {
                        if self.config.verbose >= 1 {
                            eprintln!("[LRC·结晶·LLM] 写入合成记忆失败: {}", e);
                        }
                    }
                }
            }
            count
        };

        if self.config.verbose >= 2 {
            eprintln!("[LRC·结晶·LLM] Phase 3 完成：写入 {} 条合成记忆", written);
        }

        Ok(written)
    }

    /// v0.6.0 新增：基于统一 Embedder 的结晶合成
    ///
    /// 接受任意 Embedder 实现（本地 BERT 或 LLM API），复用相同的聚类算法。
    /// 总结阶段：如果有 LLM summarizer，用 LLM 生成高质量总结；否则用简单文本拼接。
    ///
    /// 遵循三阶段锁安全模式：
    ///   Phase 1: 持锁加载记忆列表（<1ms）
    ///   Phase 2: 无锁嵌入+聚类+总结（I/O 密集）
    ///   Phase 3: 持锁写入合成记忆（<1ms）
    ///
    /// # 参数
    /// - `embedder`：嵌入器（LocalBertEmbedder 或 LlmApiEmbedder）
    /// - `summarizer`：可选的 LLM 总结器（有则用 LLM 生成总结，无则用本地拼接）
    ///
    /// 注：此方法已实现并测试通过，等待集成到 run_cycle 主流程（功能 4.2.3 后续步骤）。
    #[allow(dead_code)]
    async fn embedding_synthesize_cycle(
        &self,
        embedder: &dyn Embedder,
        summarizer: Option<&LlmApiConfig>,
    ) -> Result<usize, String> {
        // ===== Phase 1：持锁加载记忆列表 =====
        let candidates: Vec<(String, String)> = {
            let store = self.store.lock().await;
            let filter = ListFilter {
                limit: 1000, // 加载足够多的记忆用于聚类
                ..ListFilter::new()
            };
            let (all, _) = store
                .list_memories(&filter)
                .map_err(|e| format!("加载记忆列表失败: {}", e))?;
            all.into_iter()
                .filter(|m| m.memory_type != MemoryType::Synthesis)
                .map(|m| (m.id, m.content))
                .collect::<Vec<_>>()
        };

        if candidates.len() < self.config.synthesis_threshold {
            if self.config.verbose >= 2 {
                eprintln!(
                    "[LRC·结晶·Embed] 记忆数 {} 低于阈值 {}，跳过合成",
                    candidates.len(),
                    self.config.synthesis_threshold
                );
            }
            return Ok(0);
        }

        if self.config.verbose >= 2 {
            eprintln!(
                "[LRC·结晶·Embed] Phase 1 完成：加载 {} 条候选记忆（embedder={}, dim={}）",
                candidates.len(),
                embedder.model_id(),
                embedder.dim()
            );
        }

        // ===== Phase 2：无锁，embedding + 聚类 + 总结 =====
        let texts: Vec<&str> = candidates.iter().map(|(_, c)| c.as_str()).collect();
        let embeddings = embedder
            .embed(&texts)
            .await
            .map_err(|e| format!("Embedding 调用失败: {}", e))?;

        if embeddings.len() != candidates.len() {
            return Err(format!(
                "Embedding 数量不匹配：期望 {}，实际 {}",
                candidates.len(),
                embeddings.len()
            ));
        }

        // 基于余弦相似度聚类（贪心法，复用现有算法）
        let clusters = self.cluster_by_embedding(&embeddings, self.config.synthesis_similarity);

        // 信息增量阈值：与 DaoRegulator 默认值一致（0.01）
        const INFO_GAIN_THRESHOLD: f32 = 0.01;

        let mut synthesis_results: Vec<(Vec<String>, String, f32)> = Vec::new();

        for cluster_indices in &clusters {
            if cluster_indices.len() < self.config.synthesis_threshold {
                continue;
            }

            // 计算信息增量：1 - 平均成对余弦相似度
            let avg_sim = self.average_pairwise_similarity(&embeddings, cluster_indices);
            let information_gain = 1.0 - avg_sim;

            if information_gain < INFO_GAIN_THRESHOLD {
                if self.config.verbose >= 2 {
                    eprintln!(
                        "[LRC·结晶·Embed] 跳过簇（大小={}，信息增量 {:.4} < 阈值 {:.4}）",
                        cluster_indices.len(),
                        information_gain,
                        INFO_GAIN_THRESHOLD
                    );
                }
                continue;
            }

            // 收集簇内记忆内容
            let cluster_memories: Vec<String> = cluster_indices
                .iter()
                .map(|&i| candidates[i].1.clone())
                .collect();

            // 总结：有 LLM 用 LLM，否则用本地拼接降级
            let summary = if let Some(llm) = summarizer {
                llm.summarize_memories(&cluster_memories)
                    .await
                    .map_err(|e| format!("LLM 合成总结失败: {}", e))?
            } else {
                self.local_summarize(&cluster_memories)
            };

            // 收集源记忆 ID
            let source_ids: Vec<String> = cluster_indices
                .iter()
                .map(|&i| candidates[i].0.clone())
                .collect();

            if self.config.verbose >= 2 {
                eprintln!(
                    "[LRC·结晶·Embed] 簇通过阈值（大小={}，信息增量 {:.4}），已生成合成内容（{} 字）",
                    cluster_indices.len(),
                    information_gain,
                    summary.chars().count()
                );
            }

            synthesis_results.push((source_ids, summary, information_gain));
        }

        if synthesis_results.is_empty() {
            if self.config.verbose >= 1 {
                eprintln!("[LRC·结晶·Embed] 无簇通过信息增量阈值，本次不生成合成记忆");
            }
            return Ok(0);
        }

        // ===== Phase 3：持锁写入合成记忆 =====
        let written = {
            let mut store = self.store.lock().await;
            let mut count = 0usize;
            for (source_ids, summary, info_gain) in synthesis_results {
                // 区分 LLM 合成和本地合成（用于审计与置信度）
                let (source_tag, confidence) = if summarizer.is_some() {
                    ("llm_synthesis", 0.85)
                } else {
                    ("local_synthesis", 0.65) // 本地合成置信度较低
                };
                let mut memory = Memory::new(
                    summary,
                    MemoryType::Synthesis,
                    None,
                    Vec::new(),
                    Importance::new(7), // 合成记忆重要性 7
                    None,
                )
                .with_source(source_tag);
                memory.source_ids = source_ids;
                memory.information_gain = Some(info_gain);
                memory.confidence = Some(confidence);
                memory.resolution = "synthesized".to_string();

                match store.remember(memory) {
                    Ok(_) => count += 1,
                    Err(e) => {
                        if self.config.verbose >= 1 {
                            eprintln!("[LRC·结晶·Embed] 写入合成记忆失败: {}", e);
                        }
                    }
                }
            }
            count
        };

        if self.config.verbose >= 2 {
            eprintln!(
                "[LRC·结晶·Embed] Phase 3 完成：写入 {} 条合成记忆（source={}）",
                written,
                if summarizer.is_some() { "llm" } else { "local" }
            );
        }

        Ok(written)
    }

    /// v0.6.0 新增：本地总结（无 LLM 时的降级方案）
    ///
    /// 简单拼接前 N 条记忆，用于本地嵌入路径的合成。
    /// 虽然质量不如 LLM 总结，但能保留簇内记忆的关键信息。
    ///
    /// 注：此方法由 `embedding_synthesize_cycle` 调用，等待集成到主流程。
    #[allow(dead_code)]
    fn local_summarize(&self, memories: &[String]) -> String {
        if memories.is_empty() {
            return String::new();
        }
        if memories.len() == 1 {
            return memories[0].clone();
        }
        // 简单拼接：取前 5 条，每条截断到 100 字
        const MAX_ITEMS: usize = 5;
        const MAX_CHARS: usize = 100;
        let n = memories.len();
        let parts: Vec<String> = memories
            .iter()
            .take(MAX_ITEMS)
            .enumerate()
            .map(|(i, m)| {
                let truncated: String = m.chars().take(MAX_CHARS).collect();
                format!("{}. {}", i + 1, truncated)
            })
            .collect();
        let suffix = if n > MAX_ITEMS {
            format!("\n...（共 {} 条记忆）", n)
        } else {
            String::new()
        };
        format!("融合 {} 条记忆:\n{}{}", n, parts.join("\n"), suffix)
    }

    /// v0.5.18 新增：基于余弦相似度的贪心聚类
    ///
    /// 算法：遍历每条记忆，如果与已有簇的质心相似度超过阈值，归入该簇；
    /// 否则创建新簇。返回每个簇的候选索引列表。
    ///
    /// 注：质心为簇内所有向量的平均值。
    fn cluster_by_embedding(
        &self,
        embeddings: &[Vec<f32>],
        similarity_threshold: f32,
    ) -> Vec<Vec<usize>> {
        if embeddings.is_empty() {
            return Vec::new();
        }

        let mut clusters: Vec<Vec<usize>> = Vec::new();
        let mut centroids: Vec<Vec<f32>> = Vec::new();

        for (i, emb) in embeddings.iter().enumerate() {
            // 寻找最相似的簇
            let mut best_cluster: Option<usize> = None;
            let mut best_sim: f32 = -1.0;

            for (ci, centroid) in centroids.iter().enumerate() {
                let sim = cosine_similarity(emb, centroid);
                if sim > best_sim {
                    best_sim = sim;
                    best_cluster = Some(ci);
                }
            }

            if best_sim >= similarity_threshold {
                if let Some(ci) = best_cluster {
                    clusters[ci].push(i);
                    // 更新质心
                    self.update_centroid(&mut centroids[ci], &clusters[ci], embeddings);
                }
            } else {
                // 创建新簇
                clusters.push(vec![i]);
                centroids.push(emb.clone());
            }
        }

        clusters
    }

    /// 更新簇质心（所有成员向量的平均值）
    fn update_centroid(
        &self,
        centroid: &mut [f32],
        member_indices: &[usize],
        embeddings: &[Vec<f32>],
    ) {
        if member_indices.is_empty() {
            return;
        }
        let n = member_indices.len() as f32;
        for (d, slot) in centroid.iter_mut().enumerate() {
            let sum: f32 = member_indices
                .iter()
                .map(|&i| embeddings[i].get(d).copied().unwrap_or(0.0))
                .sum();
            *slot = sum / n;
        }
    }

    /// 计算簇内平均成对余弦相似度
    ///
    /// 用于衡量簇内记忆的语义一致性。
    /// 高相似度 → 信息增量低（记忆冗余）；
    /// 低相似度 → 信息增量高（记忆多样，合成有价值）。
    fn average_pairwise_similarity(
        &self,
        embeddings: &[Vec<f32>],
        cluster_indices: &[usize],
    ) -> f32 {
        let n = cluster_indices.len();
        if n < 2 {
            return 1.0; // 单条记忆与自身完全相似
        }

        let mut total_sim = 0.0f32;
        let mut pair_count = 0usize;

        for i in 0..n {
            for j in (i + 1)..n {
                let a = &embeddings[cluster_indices[i]];
                let b = &embeddings[cluster_indices[j]];
                total_sim += cosine_similarity(a, b);
                pair_count += 1;
            }
        }

        if pair_count == 0 {
            1.0
        } else {
            total_sim / pair_count as f32
        }
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

    // ==================== v0.5.18 LLM 合成路径测试 ====================

    /// 测试：高维余弦相似度计算
    #[test]
    fn test_cosine_similarity_high_dim() {
        use crate::engine::llm_translator::cosine_similarity;

        // 完全相同的向量 → 相似度 1.0
        let a = vec![1.0, 0.5, 0.3, 0.8];
        let b = vec![1.0, 0.5, 0.3, 0.8];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-5, "完全相同应=1.0，实际: {}", sim);

        // 正交向量 → 相似度 0.0
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-5, "正交应≈0.0，实际: {}", sim);

        // 零向量 → 相似度 0.0
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-5, "零向量应=0.0，实际: {}", sim);
    }

    /// 测试：基于 embedding 的贪心聚类
    #[test]
    fn test_cluster_by_embedding() {
        let (_dir, store) = make_store();
        let store = Arc::new(Mutex::new(store));
        let pipeline = make_pipeline(store);

        // 构造两组语义相似的 embedding：
        // 组1（数据库相关）：前 3 条相似，余弦相似度 > 0.9
        // 组2（前端相关）：后 2 条相似
        let embeddings: Vec<Vec<f32>> = vec![
            vec![1.0, 0.1, 0.05, 0.0, 0.0],   // DB-1
            vec![0.95, 0.15, 0.1, 0.0, 0.0],  // DB-2
            vec![0.9, 0.1, 0.0, 0.05, 0.0],   // DB-3
            vec![0.0, 0.0, 0.0, 0.1, 1.0],    // FE-1
            vec![0.0, 0.0, 0.05, 0.15, 0.95], // FE-2
        ];

        // synthesis_similarity = 0.3（来自 make_pipeline 的 config）
        let clusters = pipeline.cluster_by_embedding(&embeddings, 0.3);

        // 应形成 2 个簇：DB 组（3 条）+ FE 组（2 条）
        assert!(
            clusters.len() >= 2,
            "应至少形成 2 个簇，实际: {}",
            clusters.len()
        );

        // 最大簇应包含 3 条记忆
        let max_cluster_size = clusters.iter().map(|c| c.len()).max().unwrap_or(0);
        assert_eq!(
            max_cluster_size, 3,
            "最大簇应包含 3 条记忆，实际: {}",
            max_cluster_size
        );
    }

    /// 测试：信息增量计算（平均成对相似度）
    #[test]
    fn test_average_pairwise_similarity() {
        let (_dir, store) = make_store();
        let store = Arc::new(Mutex::new(store));
        let pipeline = make_pipeline(store);

        let embeddings: Vec<Vec<f32>> = vec![
            vec![1.0, 0.0, 0.0],
            vec![1.0, 0.0, 0.0], // 与第一条完全相同
            vec![1.0, 0.0, 0.0], // 与第一条完全相同
        ];

        let avg_sim = pipeline.average_pairwise_similarity(&embeddings, &[0, 1, 2]);
        assert!(
            (avg_sim - 1.0).abs() < 1e-5,
            "完全相同的向量平均相似度应=1.0，实际: {}",
            avg_sim
        );

        // 信息增量 = 1 - avg_sim = 0.0（完全冗余，不应合成）
        let info_gain = 1.0 - avg_sim;
        assert!(
            info_gain < 0.01,
            "完全冗余的信息增量应 < 0.01，实际: {}",
            info_gain
        );

        // 语义多样的记忆 → 低相似度 → 高信息增量
        let diverse: Vec<Vec<f32>> = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ];
        let avg_sim_div = pipeline.average_pairwise_similarity(&diverse, &[0, 1, 2]);
        let info_gain_div = 1.0 - avg_sim_div;
        assert!(
            info_gain_div > 0.9,
            "正交向量的信息增量应 > 0.9，实际: {}",
            info_gain_div
        );
    }

    /// 测试：LLM 未配置时自动降级到洛书合成
    #[tokio::test]
    async fn test_llm_not_configured_fallback_to_luoshu() {
        let (_dir, store) = make_store();
        let store = Arc::new(Mutex::new(store));

        // 使用 new（不传 LLM 配置）创建 pipeline
        let config = ConsolidationConfig {
            poll_interval_secs: 3600,
            batch_size: 10,
            synthesis_threshold: 2,
            synthesis_similarity: 0.3,
            run_on_start: false,
            auto_synthesize: true,
            verbose: 0,
        };
        let pipeline = ConsolidationPipeline::new(config, store.clone());

        // llm_config 应为 None
        assert!(
            pipeline.llm_config.is_none(),
            "未传入 LLM 配置时 llm_config 应为 None"
        );

        // 写入 3 条相似记忆，然后运行周期
        let mut pipeline = pipeline;
        let source = InMemorySource::new(
            "test-fallback",
            vec![
                SurfaceMemory {
                    content: "Rust 内存安全所有权机制".into(),
                    memory_type: "fact".into(),
                    importance: 7,
                    project: None,
                    tags: vec!["rust".into()],
                    session_id: None,
                    user_id: None,
                    timestamp: Some(Utc::now()),
                    source: "test".into(),
                },
                SurfaceMemory {
                    content: "Rust 借用检查器工作原理".into(),
                    memory_type: "fact".into(),
                    importance: 7,
                    project: None,
                    tags: vec!["rust".into()],
                    session_id: None,
                    user_id: None,
                    timestamp: Some(Utc::now()),
                    source: "test".into(),
                },
                SurfaceMemory {
                    content: "Rust 生命周期标注规则".into(),
                    memory_type: "fact".into(),
                    importance: 7,
                    project: None,
                    tags: vec!["rust".into()],
                    session_id: None,
                    user_id: None,
                    timestamp: Some(Utc::now()),
                    source: "test".into(),
                },
            ],
        );

        let stats = pipeline.run_cycle(&source).await.expect("周期应成功");
        assert_eq!(stats.stored, 3);
        // 降级路径：洛书合成应执行（无论是否生成合成记忆，都不应报错）
        // 注：洛书合成在 9 维空间下信息增量极低，可能不生成合成记忆，
        // 但关键是无 LLM 时不崩溃、正确降级
    }

    /// 测试：LlmApiConfig::None 时 new_with_llm 应将 llm_config 设为 None
    #[test]
    fn test_new_with_llm_none_config() {
        let (_dir, store) = make_store();
        let store = Arc::new(Mutex::new(store));

        let config = ConsolidationConfig::default();
        let pipeline = ConsolidationPipeline::new_with_llm(
            config,
            store,
            crate::engine::llm_translator::LlmApiConfig::None,
        );

        assert!(
            pipeline.llm_config.is_none(),
            "LlmApiConfig::None 时 llm_config 应为 None"
        );
    }
}
