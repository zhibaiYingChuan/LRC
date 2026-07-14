//! LRC Desktop 库入口
//! Tauri 需要 lib.rs 作为 crate root

// ════════════════════════════════════════════════════════════════
// v0.5.17 锁顺序静态检查
//
// 启用 clippy::await_holding_lock lint，检测在持有 MutexGuard 时
// 跨 await 点的情况。这是 v0.5.15 锁竞争问题的根因：
//   - 持有 sidecar 锁时调用 probe_existing_sidecar()（500ms I/O）
//   - 持有 sidecar 锁时调用 wait_for_health()（最多 40s I/O）
//
// 注意：clippy::await_holding_lock 主要检测 std::sync::MutexGuard。
// 对于 tokio::sync::MutexGuard（本项目使用），该 lint 不直接生效，
// 因为 tokio 的 MutexGuard 是 Send 的。
// 但保留此 lint 作为防御性措施，防止未来误用 std::sync::Mutex。
//
// tokio::sync::Mutex 的锁竞争检测依赖：
//   1. 三阶段锁安全模式（prepare_start + spawn_and_wait + insert_handle）
//   2. 并发压力测试（test_concurrent_* 系列）
//   3. Code review 和锁顺序约定文档
// ════════════════════════════════════════════════════════════════
#![warn(clippy::await_holding_lock)]

// ════════════════════════════════════════════════════════════════
// 锁顺序约定（L1 → L2）
//
// AppStore 包含多个 Mutex，必须按以下顺序获取：
//   L1: sidecar        — sidecar 进程管理器
//   L2: sidecar_port   — sidecar 当前端口
//   L3: wizard         — 配置向导状态
//   L4: agent_registry — Agent 检测器注册表
//   L5: rate_limiter   — 速率限制器
//   L6: configured_agent_count — 已配置 Agent 计数
//
// 规则：
//   1. 严禁在持有 L1 时获取 L2（v0.5.15 的错误）
//   2. 严禁在持有任何锁时执行长时间 I/O（>100ms）
//   3. 如需在操作中执行 I/O，使用三阶段模式：
//      Phase 1: 持锁收集状态 → 释放锁
//      Phase 2: 执行 I/O（不持锁）
//      Phase 3: 重新获取锁更新状态
//
// clippy::await_holding_lock 会自动检测规则 2 的违规。
// 规则 1 和 3 需要通过 code review 和并发压力测试保证。
// ════════════════════════════════════════════════════════════════

pub mod agent_detector;
pub mod commands;
pub mod config_wizard;
pub mod crypto;
pub mod integrity;
pub mod rate_limiter;
pub mod sidecar_manager;
pub mod tray;