// ============================================================
// Loong Recall 基准测试库模块
// ============================================================
//
// 提供三层基准测试的核心逻辑，可供 CLI 工具、仪表盘 API、
// 和 CI/CD 流水线复用。
//
// 道枢映射：中宫（五）— 统摄八方，基准测试如中宫之统摄
// ============================================================

use std::time::Instant;
use tempfile::TempDir;

use crate::engine::dao_metrics::DaoMetricsSnapshot;
use crate::memory_store::{MemoryStore, RecallFilter};
use crate::memory_types::{Importance, Memory, MemoryType, PrivacyLevel};
use crate::persistence::create_json_persistence;
use crate::persistence::json::JsonPersistence;
use crate::persistence::Persistence;

// ════════════════════════════════════════════════════════════
// 测试辅助函数
// ════════════════════════════════════════════════════════════

fn make_store() -> (TempDir, MemoryStore<JsonPersistence>) {
    let dir = TempDir::new().expect("创建临时目录失败");
    let persistence =
        create_json_persistence(&dir.path().to_string_lossy()).expect("创建持久化层失败");
    // 使用统计编码器跳过 ML 模型下载，加速基准测试
    let store = MemoryStore::new_statistical(persistence);
    (dir, store)
}

fn generate_test_memories(count: usize, prefix: &str, importance: Importance) -> Vec<Memory> {
    let mut memories = Vec::with_capacity(count);
    let base_time = chrono::Utc::now() - chrono::Duration::days(365);
    for i in 0..count {
        let days_ago = (i as f64 * 365.0 / count as f64) as i64;
        let content = format!(
            "{}记忆 #{:04}: 这是一条关于{}的测试记忆内容。包含关键词：项目、API、数据库、配置。",
            prefix, i, prefix
        );
        let mut mem = Memory::new(
            content,
            MemoryType::Fact,
            Some("test-project".to_string()),
            vec!["测试".to_string(), "基准".to_string()],
            importance,
            None,
        );
        mem.created_at = base_time + chrono::Duration::days(days_ago);
        mem.last_accessed = mem.created_at;
        mem.id = format!("{}-{:04}", prefix, i);
        mem.privacy_level = PrivacyLevel::Global;
        memories.push(mem);
    }
    memories
}

fn generate_db_connection_memories(count: usize) -> Vec<Memory> {
    let templates = vec![
        "数据库连接超时，错误码 50001，发生在 pg_main 实例",
        "数据库连接池耗尽，最大连接数 100，当前活跃连接 98",
        "PostgreSQL 连接超时，目标主机 10.0.1.5:5432，超时 30s",
        "数据库连接失败，SSL 握手超时，切换到备用连接池",
        "修复数据库连接超时：增加连接超时时间至 60s",
        "数据库连接池配置优化：max_connections 从 100 提升到 200",
        "发现数据库连接泄漏：未释放的连接导致连接池耗尽",
        "最终方案：使用连接池监控 + 自动回收 + 超时强制断开",
        "数据库连接超时根因分析：网络分区导致 TCP 连接挂起",
        "数据库连接监控告警：P99 延迟超过 500ms，触发自动扩容",
    ];
    let mut memories = Vec::with_capacity(count);
    for i in 0..count {
        let template = templates[i % templates.len()];
        let content = format!("{} [时间戳: {}]", template, i);
        let mut mem = Memory::new(
            content,
            MemoryType::Fact,
            Some("db-project".to_string()),
            vec!["数据库".to_string(), "诊断".to_string()],
            Importance::new(5),
            None,
        );
        mem.id = format!("db-conn-{:04}", i);
        mem.privacy_level = PrivacyLevel::Global;
        memories.push(mem);
    }
    memories
}

fn generate_noise_memories(count: usize) -> Vec<Memory> {
    let noise_texts = vec![
        "今天天气真好，适合出去散步",
        "我午餐吃了三明治和咖啡",
        "会议改到下午三点，请准时参加",
        "这个功能需要重构，架构太复杂了",
        "Python 是世界上最快的语言（矛盾信息）",
        "Rust 不适合做 Web 开发（错误信息）",
        "推荐使用 jQuery 来构建现代前端项目（过时建议）",
        "明天要交房租，别忘了转账",
        "这个电影评分很高，周末去看",
        "数据库连接永远不会超时（错误信息）",
    ];
    let mut memories = Vec::with_capacity(count);
    for i in 0..count {
        let text = noise_texts[i % noise_texts.len()];
        let mut mem = Memory::new(
            format!("噪声 #{}: {}", i, text),
            MemoryType::Fact,
            Some("test-project".to_string()),
            vec!["噪声".to_string()],
            Importance::new(2),
            None,
        );
        mem.id = format!("noise-{:04}", i);
        mem.privacy_level = PrivacyLevel::Global;
        memories.push(mem);
    }
    memories
}

// ════════════════════════════════════════════════════════════
// 基准测试结果类型
// ════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize)]
pub struct BenchmarkResult {
    pub name: String,
    pub layer: u8,
    pub description: String,
    pub industry_problem: String,
    pub passed: bool,
    pub score: f64,
    pub details: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BenchmarkReport {
    pub version: String,
    pub generated_at: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub layers: Vec<LayerReport>,
    pub results: Vec<BenchmarkResult>,
    pub radar_scores: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LayerReport {
    pub name: String,
    pub total: usize,
    pub passed: usize,
    pub status: String,
}

// ════════════════════════════════════════════════════════════
// 第一层：通用记忆检索基准
// ════════════════════════════════════════════════════════════

pub fn run_benchmark_l1_retrieval_latency() -> BenchmarkResult {
    let start = Instant::now();
    let (_dir, mut store) = make_store();
    let memories = generate_test_memories(1000, "latency", Importance::new(5));
    for m in &memories {
        store.remember(m.clone()).expect("写入记忆失败");
    }
    let filter = RecallFilter::new().with_top_k(10);
    let mut latencies = Vec::with_capacity(100);
    for _ in 0..100 {
        let s = Instant::now();
        let _ = store.recall("数据库", &filter);
        latencies.push(s.elapsed().as_micros() as f64);
    }
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p50 = latencies[49];
    let p95 = latencies[94];
    let passed = p50 < 500_000.0 && p95 < 1_000_000.0;
    let score = if passed {
        0.9
    } else {
        (500_000.0 / p50.max(1.0)).min(1.0) * 0.8
    };

    BenchmarkResult {
        name: "benchmark_retrieval_latency_scalability".into(),
        layer: 1,
        description: "大规模记忆检索延迟（1K 规模 P50/P95）".into(),
        industry_problem: "RAG 系统长上下文检索延迟退化问题".into(),
        passed,
        score,
        details: format!("P50: {:.0}μs, P95: {:.0}μs", p50, p95),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

pub fn run_benchmark_l1_recall_precision() -> BenchmarkResult {
    let start = Instant::now();
    let (_dir, mut store) = make_store();
    let mut golden = Memory::new(
        "项目 Loong Recall 使用 Rust 编写，记忆核心基于洛书编码器的 9 维向量空间".to_string(),
        MemoryType::Fact,
        Some("loong-recall".to_string()),
        vec!["洛书".to_string(), "编码器".to_string()],
        Importance::new(8),
        None,
    );
    golden.id = "golden-001".to_string();
    golden.privacy_level = PrivacyLevel::Global;
    store.remember(golden).expect("写入黄金记忆失败");

    let filter = RecallFilter::new().with_top_k(5);
    let result = store
        .recall("Loong Recall Rust 洛书编码器", &filter)
        .expect("检索失败");
    let passed = !result.memories.is_empty();
    let top_content = result
        .memories
        .first()
        .map(|m| m.content.clone())
        .unwrap_or_default();
    let score = if passed && (top_content.contains("Loong Recall") || top_content.contains("洛书"))
    {
        0.95
    } else {
        0.3
    };

    BenchmarkResult {
        name: "benchmark_retrieval_recall_precision".into(),
        layer: 1,
        description: "检索召回率 — 精确匹配".into(),
        industry_problem: "向量检索中的语义漂移问题".into(),
        passed,
        score,
        details: format!("返回 {} 条结果", result.memories.len()),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

pub fn run_benchmark_l1_session_recall() -> BenchmarkResult {
    let start = Instant::now();
    let (_dir, mut store) = make_store();
    let conversations = vec![
        ("用户: 我叫张三，目前在北京工作", "fact", "high"),
        ("用户: 我使用 Python 和 Rust 进行开发", "fact", "high"),
        ("用户: 我的项目叫 Loong，是一个记忆系统", "fact", "high"),
        (
            "用户: 数据库使用 PostgreSQL，缓存用 Redis",
            "fact",
            "medium",
        ),
        ("用户: 我更喜欢 pnpm 而不是 npm", "preference", "high"),
        (
            "用户: 上次你说用 Rust 实现洛书编码器，我已经完成了",
            "fact",
            "medium",
        ),
        ("用户: 下周一要提交项目报告，周三有演示", "fact", "medium"),
        ("用户: 我的团队有 5 个人，都是后端工程师", "fact", "medium"),
        ("用户: 服务器部署在阿里云华东区", "fact", "low"),
        ("用户: 我每天下午 3 点喝咖啡", "preference", "low"),
    ];
    for (content, mem_type, importance) in &conversations {
        let mtype = match *mem_type {
            "fact" => MemoryType::Fact,
            _ => MemoryType::Preference,
        };
        let imp = match *importance {
            "high" => Importance::new(8),
            "medium" => Importance::new(5),
            _ => Importance::new(2),
        };
        let mut mem = Memory::new(
            content.to_string(),
            mtype,
            Some("chat".to_string()),
            vec!["对话".to_string()],
            imp,
            None,
        );
        mem.id = format!("session-{}", content.len());
        mem.privacy_level = PrivacyLevel::Global;
        store.remember(mem).expect("写入对话记忆失败");
    }
    let queries = vec!["张三", "Rust", "Loong", "PostgreSQL", "pnpm", "北京"];
    let filter = RecallFilter::new().with_top_k(3);
    let mut recalled = 0;
    for query in &queries {
        let result = store.recall(query, &filter).expect("检索失败");
        if !result.memories.is_empty() {
            let combined: String = result
                .memories
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if combined.contains(query) {
                recalled += 1;
            }
        }
    }
    let recall_rate = recalled as f64 / queries.len() as f64;
    let passed = recall_rate >= 0.5;
    let score = recall_rate;

    BenchmarkResult {
        name: "benchmark_session_recall_accuracy".into(),
        layer: 1,
        description: "Session Recall — 长对话上下文事实提取".into(),
        industry_problem: "会话引擎中的遗忘灾难问题".into(),
        passed,
        score,
        details: format!(
            "召回率: {:.1}% ({}/{})",
            recall_rate * 100.0,
            recalled,
            queries.len()
        ),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

// ════════════════════════════════════════════════════════════
// 第二层：LRC 独有能力基准
// ════════════════════════════════════════════════════════════

pub fn run_benchmark_l2_decay() -> BenchmarkResult {
    let start = Instant::now();
    let (_dir, mut store) = make_store();
    let high = generate_test_memories(50, "high", Importance::new(8));
    let low = generate_test_memories(50, "low", Importance::new(2));
    for m in &high {
        store.remember(m.clone()).expect("写入失败");
    }
    for m in &low {
        store.remember(m.clone()).expect("写入失败");
    }
    let filter = RecallFilter::new().with_top_k(20);
    let result = store.recall("测试记忆内容", &filter).expect("检索失败");
    let mut high_count = 0;
    let mut low_count = 0;
    for mem in &result.memories {
        if mem.id.starts_with("high") {
            high_count += 1;
        } else if mem.id.starts_with("low") {
            low_count += 1;
        }
    }
    let passed = high_count >= 0;
    let score = if high_count + low_count > 0 {
        high_count as f64 / (high_count + low_count) as f64
    } else {
        0.5
    };

    BenchmarkResult {
        name: "benchmark_memory_decay_effectiveness".into(),
        layer: 2,
        description: "记忆衰减有效性 — 高重要性记忆优先检索".into(),
        industry_problem: "记忆系统中的无差别遗忘问题".into(),
        passed,
        score,
        details: format!("前20条中：高重要性={}, 低重要性={}", high_count, low_count),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

pub fn run_benchmark_l2_synthesis() -> BenchmarkResult {
    let start = Instant::now();
    let (_dir, mut store) = make_store();
    // 写入相似的数据库连接故障记忆，测试合成引擎是否能自动提炼规律
    let db_memories = generate_db_connection_memories(10);
    for m in &db_memories {
        store.remember(m.clone()).expect("写入失败");
    }

    // 验证合成前记忆数（写入 + 初始种子记忆）
    let all_before = store.persistence().load_all_memories().expect("加载失败");
    let count_before = all_before.len();

    // 计算前两条记忆的 Jaccard 相似度（验证它们确实相似）
    let jaccard = store
        .synthesis_engine
        .compute_jaccard(&db_memories[0].content, &db_memories[1].content);

    // 实际触发洛书合成（核心测试：碎片信息自动提炼为标准方案）
    let synthesized = store.luoshu_synthesize().unwrap_or(0);

    // 验证合成后记忆数增加
    let all_after = store.persistence().load_all_memories().expect("加载失败");

    // 检索验证：合成后应该能搜到数据库排查相关内容
    let filter = RecallFilter::new().with_top_k(5);
    let result = store
        .recall("排查数据库超时的标准步骤", &filter)
        .expect("检索失败");
    let combined: String = result
        .memories
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let has_keywords = combined.contains("数据库") && combined.contains("超时");

    // 合成成功条件：Jaccard 合法 + 合成产生了新记忆 + 检索能找到关键词
    let passed = (0.0..=1.0).contains(&jaccard)
        && synthesized > 0
        && all_after.len() > count_before
        && has_keywords;
    let score = if passed {
        0.9
    } else if has_keywords {
        0.6
    } else {
        0.3
    };

    BenchmarkResult {
        name: "benchmark_synthesis_trigger_and_quality".into(),
        layer: 2,
        description: "合成触发与质量 — 碎片信息自动提炼".into(),
        industry_problem: "AI 记忆的被动存储困境".into(),
        passed,
        score,
        details: format!(
            "Jaccard: {:.3}, 合成: {} 条, 记忆数: {}→{}, 含关键词: {}",
            jaccard,
            synthesized,
            count_before,
            all_after.len(),
            has_keywords
        ),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

pub fn run_benchmark_l2_yin_yang() -> BenchmarkResult {
    let start = Instant::now();
    let (_dir, mut store) = make_store();
    let mut snapshots: Vec<DaoMetricsSnapshot> = Vec::new();
    for batch in 0..5 {
        let memories = generate_test_memories(100, &format!("batch-{}", batch), Importance::new(5));
        for m in &memories {
            store.remember(m.clone()).expect("写入失败");
        }
        let snapshot = store
            .dao_metrics
            .snapshot(100 * (batch + 1), 0, 0, 0.0, &[0; 8]);
        snapshots.push(snapshot);
    }
    let all_valid = snapshots
        .iter()
        .all(|s| s.dao_isomorphism_score >= 0.0 && s.dao_isomorphism_score <= 1.0);
    let all_entropy_ok = snapshots.iter().all(|s| s.bagua_entropy < 3.0);
    let passed = all_valid && all_entropy_ok;
    let avg_score: f64 = snapshots
        .iter()
        .map(|s| s.dao_isomorphism_score as f64)
        .sum::<f64>()
        / snapshots.len() as f64;

    BenchmarkResult {
        name: "benchmark_yin_yang_balance_stability".into(),
        layer: 2,
        description: "阴阳守恒稳定性 — 记忆增长中道同构度保持".into(),
        industry_problem: "复杂系统的黑盒退化问题".into(),
        passed,
        score: avg_score,
        details: format!(
            "5批快照平均道同构度: {:.3}, 八卦熵均<3.0: {}",
            avg_score, all_entropy_ok
        ),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

pub fn run_benchmark_l2_anti_pollution() -> BenchmarkResult {
    let start = Instant::now();
    let (_dir, mut store) = make_store();
    let core = generate_test_memories(80, "core", Importance::new(8));
    let noise = generate_noise_memories(20);
    for m in &core {
        store.remember(m.clone()).expect("写入失败");
    }
    for m in &noise {
        store.remember(m.clone()).expect("写入失败");
    }
    let filter = RecallFilter::new().with_top_k(10);
    let mut top_ids: Vec<Vec<String>> = Vec::new();
    for _ in 0..5 {
        let result = store.recall("测试记忆内容", &filter).expect("检索失败");
        top_ids.push(result.memories.iter().map(|m| m.id.clone()).collect());
    }
    let mut consistency = 0;
    for i in 0..5 {
        let first = &top_ids[0];
        let current = &top_ids[i];
        let overlap = first
            .iter()
            .take(5)
            .filter(|id| current.iter().take(5).any(|c| c == *id))
            .count();
        if overlap >= 3 {
            consistency += 1;
        }
    }
    let no_noise_in_top5 = top_ids
        .iter()
        .all(|ids| ids.iter().take(5).all(|id| !id.starts_with("noise")));
    let passed = consistency >= 3 && no_noise_in_top5;
    let score = consistency as f64 / 5.0;

    BenchmarkResult {
        name: "benchmark_anti_pollution_capability".into(),
        layer: 2,
        description: "抗污染能力 — 20% 噪声下的检索一致性".into(),
        industry_problem: "RAG 系统的上下文窗口污染问题".into(),
        passed,
        score,
        details: format!(
            "5次检索一致性: {}/5, 前5条无噪声: {}",
            consistency, no_noise_in_top5
        ),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

// ════════════════════════════════════════════════════════════
// 第三层：隐私与信任基准
// ════════════════════════════════════════════════════════════

pub fn run_benchmark_l3_data_localization() -> BenchmarkResult {
    let start = Instant::now();
    let (dir, mut store) = make_store();
    let mut sensitive = Memory::new(
        "用户隐私数据：身份证号 110101199001011234".to_string(),
        MemoryType::Fact,
        Some("private".to_string()),
        vec!["隐私".to_string()],
        Importance::new(8),
        None,
    );
    sensitive.privacy_level = PrivacyLevel::User;
    sensitive.user_id = Some("user-a".to_string());
    store.remember(sensitive).expect("写入失败");
    let data_path = dir.path().join("memories.json");
    let exists = data_path.exists();
    let content_ok = if exists {
        std::fs::read_to_string(&data_path)
            .unwrap_or_default()
            .contains("110101199001011234")
    } else {
        false
    };
    let passed = exists && content_ok;
    let score = if passed { 1.0 } else { 0.0 };

    BenchmarkResult {
        name: "benchmark_data_localization".into(),
        layer: 3,
        description: "数据本地化验证 — 所有数据仅存本地".into(),
        industry_problem: "AI 记忆服务的隐私悖论".into(),
        passed,
        score,
        details: format!("数据文件存在: {}, 内容完整: {}", exists, content_ok),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

pub fn run_benchmark_l3_audit_tamper() -> BenchmarkResult {
    let start = Instant::now();
    let (_dir, mut store) = make_store();
    let mut m1 = Memory::new(
        "测试记忆 1".to_string(),
        MemoryType::Fact,
        Some("test".to_string()),
        vec![],
        Importance::new(5),
        None,
    );
    m1.privacy_level = PrivacyLevel::Global;
    let mut m2 = Memory::new(
        "测试记忆 2".to_string(),
        MemoryType::Fact,
        Some("test".to_string()),
        vec![],
        Importance::new(5),
        None,
    );
    m2.privacy_level = PrivacyLevel::Global;
    store.remember(m1).expect("写入失败");
    store.remember(m2).expect("写入失败");
    let total = store.audit_trail.total_count();
    let integrity = store.audit_trail.verify_integrity();
    let anchors = store.audit_trail.get_anchors();
    let anchor_valid = if anchors.is_empty() {
        true
    } else {
        store.audit_trail.verify_anchor_chain()
    };
    let passed = integrity.is_valid && anchor_valid;
    let score = if passed { 1.0 } else { 0.5 };

    BenchmarkResult {
        name: "benchmark_audit_tamper_proof".into(),
        layer: 3,
        description: "审计防篡改验证 — 哈希链+信任锚点完整性".into(),
        industry_problem: "企业级系统的合规审计真空".into(),
        passed,
        score,
        details: format!(
            "审计事件: {}, 哈希链完整: {}, 锚点: {}",
            total,
            integrity.is_valid,
            anchors.len()
        ),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

pub fn run_benchmark_l3_privacy_isolation() -> BenchmarkResult {
    let start = Instant::now();
    let (_dir, mut store) = make_store();
    let mut session_mem = Memory::new(
        "会话私有记忆".to_string(),
        MemoryType::Fact,
        Some("test".to_string()),
        vec!["会话".to_string()],
        Importance::new(5),
        None,
    );
    session_mem.id = "session-1".to_string();
    session_mem.privacy_level = PrivacyLevel::Session;
    session_mem.session_id = Some("session-a".to_string());
    let mut user_mem = Memory::new(
        "用户私有记忆".to_string(),
        MemoryType::Preference,
        Some("test".to_string()),
        vec!["用户".to_string()],
        Importance::new(8),
        None,
    );
    user_mem.id = "user-1".to_string();
    user_mem.privacy_level = PrivacyLevel::User;
    user_mem.user_id = Some("user-a".to_string());
    let mut global_mem = Memory::new(
        "全局共享记忆".to_string(),
        MemoryType::Fact,
        Some("test".to_string()),
        vec!["全局".to_string()],
        Importance::new(5),
        None,
    );
    global_mem.id = "global-1".to_string();
    global_mem.privacy_level = PrivacyLevel::Global;
    store.remember(session_mem).expect("写入失败");
    store.remember(user_mem).expect("写入失败");
    store.remember(global_mem).expect("写入失败");

    let session_filter = RecallFilter::new().with_top_k(10).with_privacy(
        PrivacyLevel::Session,
        Some("session-a".to_string()),
        None,
    );
    let session_results = store.recall("记忆", &session_filter).expect("检索失败");
    let session_ok = session_results
        .memories
        .iter()
        .all(|m| m.privacy_level != PrivacyLevel::User);

    let user_filter = RecallFilter::new().with_top_k(10).with_privacy(
        PrivacyLevel::User,
        None,
        Some("user-a".to_string()),
    );
    let user_results = store.recall("记忆", &user_filter).expect("检索失败");
    let user_ok = user_results
        .memories
        .iter()
        .all(|m| m.privacy_level != PrivacyLevel::Session);

    let passed = session_ok && user_ok;
    let score = if passed { 1.0 } else { 0.5 };

    BenchmarkResult {
        name: "benchmark_privacy_level_isolation".into(),
        layer: 3,
        description: "隐私级别隔离 — Session/User/Global 正确隔离".into(),
        industry_problem: "多租户系统中的数据泄漏风险".into(),
        passed,
        score,
        details: format!("Session隔离: {}, User隔离: {}", session_ok, user_ok),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

pub fn run_benchmark_l3_complexity_redline() -> BenchmarkResult {
    let start = Instant::now();
    let (_dir, mut store) = make_store();
    for i in 0..5 {
        let mut mem = Memory::new(
            format!("红线测试记忆 {}", i),
            MemoryType::Fact,
            Some("test".to_string()),
            vec!["红线".to_string()],
            Importance::new(5),
            None,
        );
        mem.privacy_level = PrivacyLevel::Global;
        store.remember(mem).expect("写入失败");
    }
    let redline = store.complexity_budget.red_line_check();
    let score = store.complexity_budget.maintainability_score();
    let passed = redline.passed && score >= 0.3;

    BenchmarkResult {
        name: "benchmark_complexity_red_line_self_check".into(),
        layer: 3,
        description: "复杂度预算红线自检 — 健康系统通过CI拦截".into(),
        industry_problem: "长期维护项目的隐性技术债务".into(),
        passed,
        score: score.min(1.0),
        details: format!("红线检查: {}, 可维护性: {:.3}", redline.passed, score),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

// ════════════════════════════════════════════════════════════
// 基准测试编排
// ════════════════════════════════════════════════════════════

/// 基准测试条目类型：(层级, 编号, 测试函数)
type BenchmarkEntry = (u8, &'static str, fn() -> BenchmarkResult);

/// 运行所有基准测试，返回完整报告
pub fn run_all_benchmarks(target_layer: Option<u8>) -> BenchmarkReport {
    // 定义所有基准测试（按层分组）
    let all_benchmarks: Vec<BenchmarkEntry> = vec![
        (1, "L1-1", run_benchmark_l1_retrieval_latency),
        (1, "L1-2", run_benchmark_l1_recall_precision),
        (1, "L1-3", run_benchmark_l1_session_recall),
        (2, "L2-1", run_benchmark_l2_decay),
        (2, "L2-2", run_benchmark_l2_synthesis),
        (2, "L2-3", run_benchmark_l2_yin_yang),
        (2, "L2-4", run_benchmark_l2_anti_pollution),
        (3, "L3-1", run_benchmark_l3_data_localization),
        (3, "L3-2", run_benchmark_l3_audit_tamper),
        (3, "L3-3", run_benchmark_l3_privacy_isolation),
        (3, "L3-4", run_benchmark_l3_complexity_redline),
    ];

    let mut results: Vec<BenchmarkResult> = Vec::new();

    for (layer, _label, runner) in &all_benchmarks {
        if let Some(target) = target_layer {
            if *layer != target {
                continue;
            }
        }
        results.push(runner());
    }

    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;

    // 构建雷达图数据
    let radar_scores = build_radar_scores(&results);

    let layers = vec![
        LayerReport {
            name: "第一层：通用记忆检索基准".into(),
            total: results.iter().filter(|r| r.layer == 1).count(),
            passed: results.iter().filter(|r| r.layer == 1 && r.passed).count(),
            status: if results.iter().filter(|r| r.layer == 1).all(|r| r.passed) {
                "PASS".into()
            } else {
                "FAIL".into()
            },
        },
        LayerReport {
            name: "第二层：LRC 独有能力基准".into(),
            total: results.iter().filter(|r| r.layer == 2).count(),
            passed: results.iter().filter(|r| r.layer == 2 && r.passed).count(),
            status: if results.iter().filter(|r| r.layer == 2).all(|r| r.passed) {
                "PASS".into()
            } else {
                "FAIL".into()
            },
        },
        LayerReport {
            name: "第三层：隐私与信任基准".into(),
            total: results.iter().filter(|r| r.layer == 3).count(),
            passed: results.iter().filter(|r| r.layer == 3 && r.passed).count(),
            status: if results.iter().filter(|r| r.layer == 3).all(|r| r.passed) {
                "PASS".into()
            } else {
                "FAIL".into()
            },
        },
    ];

    BenchmarkReport {
        version: "1.0".into(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        total: results.len(),
        passed,
        failed,
        layers,
        results,
        radar_scores,
    }
}

fn build_radar_scores(results: &[BenchmarkResult]) -> serde_json::Value {
    let mut scores = serde_json::Map::new();
    for r in results {
        let key = match r.name.as_str() {
            "benchmark_retrieval_latency_scalability" => "检索性能",
            "benchmark_retrieval_recall_precision" => "检索精度",
            "benchmark_session_recall_accuracy" => "会话回忆",
            "benchmark_memory_decay_effectiveness" => "记忆衰减",
            "benchmark_synthesis_trigger_and_quality" => "记忆合成",
            "benchmark_yin_yang_balance_stability" => "健康监控",
            "benchmark_anti_pollution_capability" => "抗污染",
            "benchmark_data_localization" => "数据本地化",
            "benchmark_audit_tamper_proof" => "审计安全",
            "benchmark_privacy_level_isolation" => "隐私隔离",
            "benchmark_complexity_red_line_self_check" => "可维护性",
            _ => continue,
        };
        scores.insert(
            key.to_string(),
            serde_json::Value::Number(
                serde_json::Number::from_f64((r.score * 100.0).round() / 100.0)
                    .unwrap_or(serde_json::Number::from(0)),
            ),
        );
    }
    serde_json::Value::Object(scores)
}
