//! ============================================================
//! 许可证: Apache 2.0
//! 本文件实现「上下文沉淀」的共享执行层，属于公开层 (Layer 1)。
//! ============================================================
//!
//! # 为什么需要本模块
//!
//! `POST /v1/context/absorb` 与 MCP `absorb` 工具必须**逐位一致**：
//! 解析 → 过滤 → 批内去重 → 跨轮去重 → 写入。若各写一份，
//! 新增一个过滤规则时必然漏改其中一处（历史缺陷的典型形态）。
//!
//! 本模块把这条链路收敛为单一实现，REST 与 MCP 只做参数/错误码适配。
//!
//! # 关键约束（与 v0.11.1 上下文沉淀设计一致）
//!
//! - 自动摘要永不用 `Global`；有 session 用 `Session`，否则 `User`；
//! - 跨轮去重阈值 **0.85**（高于默认 0.5），不吞并用户显式写入的记忆；
//! - 删除语义守卫命中时整轮跳过，绝不复活已删除内容；
//! - `dry_run` 只返回判定，不写库。
//!
//! # 写入性能（C1）
//!
//! 逐条写入时，每条都要跑一次 ML 前向（实测约 0.55s/条）。
//! 本模块先把所有条目的洛书向量**一次批量编码**（同编码器、同角色），
//! 再逐条 `analyze_with_luoshu`，把 106 条的写入从 ~58.6s 压到 ≤30s。
//! 单条语义不变：批量编码与逐条编码的输出逐位一致（见编码器文档）。
//!
//! # 可观测性（B4）
//!
//! 每次沉淀结果追加一行 JSON 到 `{data_dir}/absorb_log.jsonl`，
//! 供 `GET /v1/context/absorb/log` 与 `lrc doctor --capture` 查询。
//! 该文件是**可重建的诊断日志**（不参与备份）：删除后只是历史记录消失。

use crate::context_absorb as ca;
use crate::context_absorb::DeclaredItem;
use crate::memory_types::{Importance, MemoryType, PrivacyLevel};
use crate::persistence::Persistence;
use crate::privacy::TrustMode;
use crate::v1_api::SharedStore;
use serde::Serialize;
use std::path::Path;

/// 自动沉淀的跨轮去重阈值（高于默认 0.5，避免吞并用户显式记忆）。
const ABSORB_DEDUPE_THRESHOLD: f32 = 0.85;

/// 诊断日志文件名（位于数据目录，**不参与备份**，可随时删除）。
pub const ABSORB_LOG_FILE: &str = "absorb_log.jsonl";

/// 诊断日志轮转阈值：超过后把旧文件改名为 `.1`（只保留一份旧档）。
const ABSORB_LOG_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// 一次沉淀的输入（REST / MCP 共用）。
#[derive(Debug, Clone, Default)]
pub struct AbsorbInput {
    /// 最终回复（可含 `[LRC]` 结论块）
    pub text: Option<String>,
    /// 模型直接声明的结论条目（优先于 text）
    pub items: Vec<DeclaredItem>,
    /// 宿主会话 ID
    pub session_id: Option<String>,
    /// 用户 ID（多租户域必填）
    pub user_id: Option<String>,
    /// 归属项目
    pub project: Option<String>,
    /// 宿主标识（trae / codex / cursor / claude-code / generic）
    pub host: Option<String>,
    /// 来源：model_declared | auto_distill（缺省按解析结果判定）
    pub source: Option<String>,
    /// 自动摘要的重要性上限（默认 4，最大 6）
    pub importance_cap: Option<u8>,
    /// 单次最多处理条数（默认 10，最大 50）
    pub max_items: Option<usize>,
    /// 只返回判定结果，不写库
    pub dry_run: bool,
}

/// 单条沉淀结果。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AbsorbItemOutcome {
    /// 内容预览（最多 120 字符）
    pub content: String,
    /// written | would_write | duplicate | failed
    pub reason: String,
}

/// 一次沉淀的结果（与 REST 响应字段一一对应）。
#[derive(Debug, Clone, Serialize)]
pub struct AbsorbOutcome {
    pub ok: bool,
    pub dry_run: bool,
    pub source: String,
    pub host: String,
    pub candidates: usize,
    pub written: usize,
    pub skipped: usize,
    pub failed: usize,
    pub dropped: Vec<String>,
    pub items: Vec<AbsorbItemOutcome>,
}

/// 沉淀失败分类（由调用方映射为 HTTP / JSON-RPC 错误码）。
#[derive(Debug)]
pub enum AbsorbError {
    /// 输入非法（text 与 items 都为空）→ 400 / -32602
    InvalidInput(String),
    /// 多租户域缺少 scope → 400 / -32602
    ScopeRequired(&'static str),
    /// 内部执行失败 → 500 / -32603
    Internal(String),
}

/// 执行一次沉淀。
///
/// `trust_mode` 由调用方注入（REST 用路由构建时的值，MCP 用 `privacy::from_env()`），
/// 避免本模块在测试中依赖进程级环境变量。
pub async fn absorb(
    store: SharedStore,
    trust_mode: TrustMode,
    input: AbsorbInput,
) -> Result<AbsorbOutcome, AbsorbError> {
    let started = std::time::Instant::now();
    let AbsorbInput {
        text,
        items,
        session_id,
        user_id,
        project,
        host,
        source,
        importance_cap,
        max_items,
        dry_run,
    } = input;

    let text = text.unwrap_or_default();
    let has_text = !text.trim().is_empty();
    if items.is_empty() && !has_text {
        return Err(AbsorbError::InvalidInput(
            "text 与 items 至少提供一个".to_string(),
        ));
    }

    // 隐私 scope：本地可信域无 scope 也放行；多租户域必须显式。
    let scope = crate::privacy::resolve(trust_mode, user_id.as_deref(), session_id.as_deref())
        .map_err(AbsorbError::ScopeRequired)?;
    let privacy_level = scope
        .as_ref()
        .map(|(level, _, _)| *level)
        .unwrap_or(PrivacyLevel::User);
    // 自动沉淀永不使用 Global（避免跨用户泄漏）。
    let privacy_level = if privacy_level == PrivacyLevel::Global {
        PrivacyLevel::User
    } else {
        privacy_level
    };
    // 相似合并的隐私上下文：有 scope 用 scope；匿名轮次收紧为
    // `User(None, None)`（只可见 Global），避免自动摘要跨用户/会话合并。
    let absorb_privacy: Option<crate::memory_store::PrivacyScope> =
        scope.clone().or(Some((PrivacyLevel::User, None, None)));

    let session_id = session_id.filter(|s| !s.trim().is_empty());
    let user_id = user_id.filter(|s| !s.trim().is_empty());
    let host = host.unwrap_or_else(|| "generic".to_string());
    let source_override = source;
    let has_items = !items.is_empty();
    let importance_cap = importance_cap.unwrap_or(4).clamp(1, 6);
    let max_items = max_items.unwrap_or(10).clamp(1, 50);

    // ① 解析 + 过滤 + 批内去重（纯 CPU，锁外）。
    let parsed = tokio::task::spawn_blocking(move || {
        let full_text = text;
        let (raw_items, mut dropped, from_block, guard_text) = if has_items {
            // items 路径：删除守卫要同时覆盖 text 与条目正文，避免只传
            // items 时绕过守卫、或同时传 text 时漏判 text 里的删除语义。
            let mut guard_text = full_text;
            if !guard_text.trim().is_empty() {
                guard_text.push('\n');
            }
            guard_text.push_str(
                &items
                    .iter()
                    .map(|item| item.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            let parsed = ca::parse_declared_items(items);
            (parsed.items, parsed.dropped, true, guard_text)
        } else {
            let block = ca::parse_model_block(&full_text);
            // 命中 `[LRC]` 后即使条目全被过滤也**不得**回退规则提炼：
            // 模型明确声明了什么，就只按声明处理。
            if !block.found {
                let cfg = crate::context_distill::DistillConfig {
                    max_items,
                    ..Default::default()
                };
                let distilled = crate::context_distill::distill_transcript(&full_text, &cfg);
                (distilled, Vec::new(), false, full_text)
            } else {
                (block.items, block.dropped, true, full_text)
            }
        };
        // 两条来源都走同一套过滤：类型白名单、过程叙述、删除守卫、auto-summary 标签。
        let (kept, more_dropped) = ca::filter_distilled(raw_items, &guard_text);
        dropped.extend(more_dropped);
        let (mut kept, duplicates) = ca::dedupe_within(kept);
        if kept.len() > max_items {
            // 截断必须显式记账：否则调用方只看到 candidates 变小，
            // 无法区分"本来就这么少"与"被 max_items 截掉了"（v0.11.1 审查 P2）。
            let truncated = kept.len() - max_items;
            kept.truncate(max_items);
            dropped.push(format!("truncated_by_max_items: {truncated}"));
        }
        (kept, dropped, duplicates, from_block)
    })
    .await
    .map_err(|e| AbsorbError::Internal(format!("解析任务异常终止: {e}")))?;

    let (items, dropped, within_duplicates, from_block) = parsed;
    let candidates = items.len();
    // 解析后确定来源：模型声明块（items 或 `[LRC]`）标 model_declared，
    // 规则提炼标 auto_distill。标签用于后续过滤与统计，不能标错。
    let source = source_override.unwrap_or_else(|| {
        if has_items || from_block {
            "model_declared".to_string()
        } else {
            "auto_distill".to_string()
        }
    });

    // ② 逐条 prepare（读锁）→ 批量编码（锁外）→ analyze + commit（写锁）。
    let source_for_tag = source.clone();
    let host_for_tag = host.clone();
    // 闭包要求 'static：为写入阶段克隆句柄，外层保留原值供诊断日志使用。
    let outcome = {
        let store = store.clone();
        let session_id = session_id.clone();
        let project = project.clone();
        tokio::task::spawn_blocking(move || {
            let mut slots: Vec<Option<AbsorbItemOutcome>> = Vec::with_capacity(items.len());
            // 第四项 = 是否需要编码（NoOp 条目跳过批量编码，见 LRC_DISABLE_MEMORY）。
            let mut pending: Vec<(usize, crate::memory_store::RememberPrep, String, bool)> =
                Vec::with_capacity(items.len());
            let mut written = 0usize;
            let mut skipped = within_duplicates;
            let mut failed = 0usize;

            for (index, item) in items.into_iter().enumerate() {
                let preview: String = item.content.chars().take(120).collect();
                let mut tags = item.tags.clone();
                for tag in [
                    "auto-summary".to_string(),
                    format!("host:{host_for_tag}"),
                    format!("source:{source_for_tag}"),
                ] {
                    if !tags.contains(&tag) {
                        tags.push(tag);
                    }
                }
                if let Some(sid) = session_id.as_deref() {
                    let tag = format!("session:{sid}");
                    if !tags.contains(&tag) {
                        tags.push(tag);
                    }
                }
                let event_id = session_id.as_deref().map(|sid| format!("session:{sid}"));
                let importance = item.importance.min(importance_cap);
                let content = item.content.clone();
                let memory = crate::memory_write::MemoryWriteInput {
                    content: item.content,
                    memory_type: MemoryType::try_parse(&item.memory_type),
                    importance: Some(Importance::new(importance)),
                    ttl_days: None,
                    project: project.clone(),
                    tags,
                    privacy_level: Some(privacy_level),
                    session_id: session_id.clone(),
                    user_id: user_id.clone(),
                    event_id,
                    entities: Vec::new(),
                    source_timestamp: None,
                    daoti_preview_gua: None,
                    daoti_preview_bagua: None,
                    daoti_preview_version: None,
                }
                .into_memory();

                // 读锁只包住 prepare（取快照/候选），analyze 在锁外。
                // 隐私上下文显式传入：匿名轮次只允许合并 Global 记忆。
                let prep = {
                    let guard = store.blocking_read();
                    guard.prepare_remember_with_privacy(
                        memory,
                        ABSORB_DEDUPE_THRESHOLD,
                        absorb_privacy.clone(),
                    )
                };
                match prep {
                    Ok(prep) => {
                        let needs_encode = !prep.is_noop();
                        pending.push((index, prep, content, needs_encode));
                        slots.push(None);
                    }
                    Err(e) => {
                        eprintln!("[v1/context/absorb] 准备失败: {}", e);
                        failed += 1;
                        slots.push(Some(AbsorbItemOutcome {
                            content: preview,
                            reason: "failed".to_string(),
                        }));
                    }
                }
            }

            // 批量编码：只编码需要落盘的条目（NoOp 跳过，避免实验模式白跑前向）。
            // 长度契约防御：批量结果与待编码数不一致时退化为逐条 analyze，
            // 绝不静默错位（错位会把 A 的向量写到 B 上，且无法事后发现）。
            let encode_texts: Vec<&str> = pending
                .iter()
                .filter(|(_, _, _, needs_encode)| *needs_encode)
                .map(|(_, _, text, _)| text.as_str())
                .collect();
            let vectors = if encode_texts.is_empty() {
                Vec::new()
            } else {
                let encoder = store.blocking_read().luoshu_encoder_handle();
                encoder.encode_text_batch(&encode_texts)
            };
            let vectors_ok = vectors.len() == encode_texts.len();
            if !vectors_ok {
                eprintln!(
                    "[v1/context/absorb] 批量编码返回 {} 条，期望 {} 条；回退逐条编码",
                    vectors.len(),
                    encode_texts.len()
                );
            }

            let mut vector_cursor = 0usize;
            for (index, prep, preview, needs_encode) in pending.into_iter() {
                let analysis = if needs_encode && vectors_ok {
                    let vector = vectors[vector_cursor].clone();
                    vector_cursor += 1;
                    prep.analyze_with_luoshu(vector)
                } else {
                    prep.analyze()
                };
                // `LRC_DISABLE_MEMORY` 对照实验：NoOp 不落盘，绝不能计成 written。
                if analysis.is_noop() {
                    skipped += 1;
                    slots[index] = Some(AbsorbItemOutcome {
                        content: preview,
                        reason: "no_op".to_string(),
                    });
                    continue;
                }
                if analysis.is_merge() {
                    skipped += 1;
                    slots[index] = Some(AbsorbItemOutcome {
                        content: preview,
                        reason: "duplicate".to_string(),
                    });
                    continue;
                }
                if dry_run {
                    slots[index] = Some(AbsorbItemOutcome {
                        content: preview,
                        reason: "would_write".to_string(),
                    });
                    continue;
                }
                match store.blocking_write().commit_remember(analysis) {
                    Ok(_) => {
                        written += 1;
                        slots[index] = Some(AbsorbItemOutcome {
                            content: preview,
                            reason: "written".to_string(),
                        });
                    }
                    Err(e) => {
                        eprintln!("[v1/context/absorb] 提交失败: {}", e);
                        failed += 1;
                        slots[index] = Some(AbsorbItemOutcome {
                            content: preview,
                            reason: "failed".to_string(),
                        });
                    }
                }
            }

            let results = slots.into_iter().flatten().collect::<Vec<_>>();
            (written, skipped, failed, results)
        })
        .await
        .map_err(|e| AbsorbError::Internal(format!("写入任务异常终止: {e}")))?
    };

    let (written, skipped, failed, items) = outcome;
    let result = AbsorbOutcome {
        // 部分写入失败必须如实反映在 ok 上（与 CLI 的 `ok = failed == 0` 同口径），
        // 否则调用方看到 200 + ok:true 会误以为全部落库。
        ok: failed == 0,
        dry_run,
        source,
        host,
        candidates,
        written,
        skipped,
        failed,
        dropped,
        items,
    };

    // ③ 诊断日志（B4）：写入失败不影响沉淀结果，但必须显式告警。
    record_capture(
        &store,
        trust_mode,
        &result,
        session_id.as_deref(),
        project.as_deref(),
        started,
    )
    .await;
    Ok(result)
}

/// 追加一行诊断日志；失败只告警，不影响调用方。
///
/// 注意：本函数在 async 上下文调用，**不得**使用 `blocking_read`
/// （tokio 会在运行时线程上 panic）；文件写入移入 `spawn_blocking`。
async fn record_capture(
    store: &SharedStore,
    trust_mode: TrustMode,
    outcome: &AbsorbOutcome,
    session_id: Option<&str>,
    project: Option<&str>,
    started: std::time::Instant,
) {
    let data_dir = {
        let guard = store.read().await;
        guard
            .persistence()
            .data_dir_path()
            .map(std::path::Path::to_path_buf)
    };
    let Some(data_dir) = data_dir else {
        return;
    };
    // 数据目录不存在（测试夹具的临时目录已回收）⇒ 无可写位置，静默跳过。
    // 生产环境该目录恒存在，因此这里不是降级路径。
    if !data_dir.is_dir() {
        return;
    }
    // 多租户域：日志不落会话/项目标识（最小化原则；读取端点也已 403）。
    let (session_id, project) = if trust_mode == TrustMode::MultiTenant {
        (None, None)
    } else {
        (session_id, project)
    };
    let record = serde_json::json!({
        "ts_ms": chrono::Utc::now().timestamp_millis(),
        "host": outcome.host,
        "source": outcome.source,
        "session_id": session_id,
        "project": project,
        "dry_run": outcome.dry_run,
        "candidates": outcome.candidates,
        "written": outcome.written,
        "skipped": outcome.skipped,
        "failed": outcome.failed,
        "dropped": outcome.dropped.len(),
        "elapsed_ms": started.elapsed().as_millis() as u64,
    });
    let _ = tokio::task::spawn_blocking(move || {
        if let Err(e) = append_jsonl(&data_dir, &record) {
            eprintln!("[v1/context/absorb] 诊断日志写入失败（不影响沉淀）: {}", e);
        }
    })
    .await;
}

/// 诊断日志的进程内串行锁。
///
/// 轮转（判定超阈值 → rename）与追加必须原子：两个并发 absorb 同时判定
/// 超阈值时，其中一个 `rename` 必然失败（Windows 目标已存在 / 源已移走）。
/// 该文件只是可重建的诊断日志，但也不应因并发而丢记录。
static ABSORB_LOG_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

/// 追加一行 JSON；文件超过阈值时先轮转（只保留一份旧档）。
fn append_jsonl(data_dir: &Path, record: &serde_json::Value) -> std::io::Result<()> {
    let _guard = ABSORB_LOG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = data_dir.join(ABSORB_LOG_FILE);
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > ABSORB_LOG_MAX_BYTES {
            let rotated = data_dir.join(format!("{ABSORB_LOG_FILE}.1"));
            // Windows 的 rename 不覆盖已存在目标，先删旧档。
            let _ = std::fs::remove_file(&rotated);
            std::fs::rename(&path, &rotated)?;
        }
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{}", record)?;
    Ok(())
}

/// 读取最近 N 条诊断记录（文件不存在 ⇒ 空列表，由调用方渲染诚实空态）。
///
/// 日志文件上限 4MB（超过即轮转），因此整文件读入是可控的；
/// 不做尾部 seek 是为了避免跨平台换行/编码差异带来的偏移错误。
pub fn read_recent_captures(data_dir: &Path, limit: usize) -> Vec<serde_json::Value> {
    let limit = limit.clamp(1, 200);
    let path = data_dir.join(ABSORB_LOG_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    text.lines()
        .rev()
        .take(limit)
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_store::MemoryStore;
    use crate::persistence::create_json_persistence;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lrc-absorb-log-{tag}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    fn shared_store(dir: &Path) -> SharedStore {
        let persistence =
            create_json_persistence(dir.to_str().expect("路径可转字符串")).expect("持久化");
        Arc::new(RwLock::new(MemoryStore::new(persistence)))
    }

    #[test]
    fn append_and_read_recent_captures() {
        let dir = temp_dir("basic");
        for i in 0..3 {
            append_jsonl(&dir, &serde_json::json!({"n": i})).expect("写入");
        }
        let recent = read_recent_captures(&dir, 2);
        assert_eq!(recent.len(), 2, "只返回最近 limit 条");
        assert_eq!(recent[0]["n"], 1);
        assert_eq!(recent[1]["n"], 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_missing_log_returns_empty() {
        let dir = temp_dir("missing");
        assert!(read_recent_captures(&dir, 10).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let dir = temp_dir("malformed");
        let path = dir.join(ABSORB_LOG_FILE);
        std::fs::write(&path, "{\"n\":1}\nnot-json\n{\"n\":2}\n").expect("写入");
        let recent = read_recent_captures(&dir, 10);
        assert_eq!(recent.len(), 2, "坏行必须跳过而不是整段失败");
        assert_eq!(recent[0]["n"], 1);
        assert_eq!(recent[1]["n"], 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_keeps_one_old_file() {
        let dir = temp_dir("rotate");
        let path = dir.join(ABSORB_LOG_FILE);
        // 直接造一个超过阈值的文件（内容不必是合法 JSONL，轮转只看大小）。
        std::fs::write(&path, vec![b'x'; (ABSORB_LOG_MAX_BYTES + 1) as usize]).expect("写入");
        append_jsonl(&dir, &serde_json::json!({"n": 1})).expect("追加");
        assert!(
            dir.join(format!("{ABSORB_LOG_FILE}.1")).exists(),
            "超过阈值必须轮转出旧档"
        );
        let recent = read_recent_captures(&dir, 10);
        assert_eq!(recent.len(), 1, "新文件只应包含轮转后的新记录");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// v0.11.1 审查 P2：`max_items` 截断必须显式记录，不能静默少写。
    #[tokio::test]
    async fn max_items_truncation_is_reported() {
        let dir = temp_dir("truncate");
        let store = shared_store(&dir);
        let items = (0..5)
            .map(|i| DeclaredItem {
                kind: "decision".to_string(),
                content: format!("截断测试结论 {i}：采用方案 {i}"),
                supersedes: None,
            })
            .collect();
        let outcome = absorb(
            store,
            TrustMode::LocalTrusted,
            AbsorbInput {
                items,
                max_items: Some(2),
                host: Some("test".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("沉淀应成功");
        assert_eq!(outcome.candidates, 2, "只处理 max_items 条");
        assert!(
            outcome
                .dropped
                .iter()
                .any(|reason| reason == "truncated_by_max_items: 3"),
            "必须记录被截断的条数：{:?}",
            outcome.dropped
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 回归审查 P2-H：多租户域的诊断日志不得落会话/项目标识。
    #[tokio::test]
    async fn multi_tenant_capture_redacts_session_and_project() {
        let dir = temp_dir("mt-redact");
        let store = shared_store(&dir);
        let outcome = absorb(
            store,
            TrustMode::MultiTenant,
            AbsorbInput {
                text: Some("[LRC]\n- decision: 多租户日志脱敏验证".to_string()),
                session_id: Some("sess-secret".to_string()),
                user_id: Some("u1".to_string()),
                project: Some("proj-secret".to_string()),
                host: Some("test".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("沉淀应成功");
        assert_eq!(outcome.written, 1, "应写入 1 条");

        let records = read_recent_captures(&dir, 1);
        assert_eq!(records.len(), 1, "应有一条诊断记录");
        assert!(
            records[0]["session_id"].is_null(),
            "多租户不得落会话标识: {}",
            records[0]
        );
        assert!(
            records[0]["project"].is_null(),
            "多租户不得落项目标识: {}",
            records[0]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 对照组：本地可信域必须保留会话/项目标识（既有输出不变）。
    #[tokio::test]
    async fn local_trusted_capture_keeps_session_and_project() {
        let dir = temp_dir("local-keep");
        let store = shared_store(&dir);
        absorb(
            store,
            TrustMode::LocalTrusted,
            AbsorbInput {
                text: Some("[LRC]\n- decision: 本地域日志保留归属验证".to_string()),
                session_id: Some("sess-keep".to_string()),
                project: Some("proj-keep".to_string()),
                host: Some("test".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("沉淀应成功");

        let records = read_recent_captures(&dir, 1);
        assert_eq!(records[0]["session_id"], "sess-keep", "{}", records[0]);
        assert_eq!(records[0]["project"], "proj-keep", "{}", records[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
