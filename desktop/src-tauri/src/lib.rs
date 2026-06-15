//! LRC Desktop 库入口
//! Tauri 需要 lib.rs 作为 crate root

pub mod agent_detector;
pub mod commands;
pub mod config_wizard;
pub mod crypto;
pub mod integrity;
pub mod rate_limiter;
pub mod sidecar_manager;
pub mod tray;