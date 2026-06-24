// 隐藏控制台窗口：桌面应用不需要 CMD 窗口
// 普通用户看到 CMD 窗口会困惑，且关闭可能导致后端进程异常
#![windows_subsystem = "windows"]

/// LRC Desktop — Tauri 壳层主入口
///
/// 职责：
/// 1. 管理系统托盘（右键菜单、状态指示）
/// 2. 管理后台 sidecar 进程（lrc-sidecar）
/// 3. 嵌入仪表盘 WebView
/// 4. 首次配置向导
/// 5. Agent 自动检测与配置
///
/// 契约：所有 IPC 通信通过 Tauri Commands 进行，前端不直接调用 sidecar。
use lrc_desktop_lib::{agent_detector, commands, config_wizard, integrity, rate_limiter, sidecar_manager, tray};
use commands::AppStore;
use agent_detector::AgentDetectorRegistry;
use config_wizard::WizardState;
use rate_limiter::RateLimiter;
use sidecar_manager::SidecarManager;
use tauri::Manager; // Manager trait 提供 app_handle() 等方法
use tauri::Emitter; // v0.5.4 P2-14: Emitter trait 提供 emit() 方法，用于心跳协程通知前端
use tokio::sync::Mutex; // Tauri 2 异步命令需要 tokio::sync::Mutex (支持 Send)
use tauri::WindowEvent; // v0.5.4: 窗口事件监听，用于应用关闭时清理 sidecar

fn main() {
    // ════════════════════════════════════════════════════════════════
    // v0.5.1 增强：日志系统
    // 初始化日志输出到 %APPDATA%\LoongRecall\logs\ 目录
    // 同时保留控制台输出（开发模式），方便问题排查
    // ════════════════════════════════════════════════════════════════
    init_logging();

    // ── L2 保密层：启动时完整性校验 ──
    if let Err(e) = integrity::IntegrityChecker::verify_on_startup() {
        tracing::error!("L2 完整性校验失败: {e}");
        // 静默退出，不弹出提示（避免暴露校验逻辑）
        std::process::exit(1);
    }

    // 初始化全局状态
    let app_store = AppStore {
        wizard: Mutex::new(WizardState::load().expect("加载向导状态失败：无法确定配置目录")),
        sidecar: Mutex::new(SidecarManager::new(
            // sidecar 二进制路径（与桌面应用同级目录）
            std::env::current_exe()
                .unwrap_or_default()
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("lrc-sidecar")
                .with_extension(std::env::consts::EXE_EXTENSION)
                .display()
                .to_string(),
        )),
        agent_registry: {
            let mut registry = AgentDetectorRegistry::new();
            // 设置 LRC 二进制文件的绝对路径（与桌面应用同级目录）
            // 确保 MCP 配置使用绝对路径，IDE 无需依赖 PATH 环境变量
            let lrc_binary = std::env::current_exe()
                .unwrap_or_default()
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("lrc-sidecar")
                .with_extension(std::env::consts::EXE_EXTENSION);
            registry.set_lrc_binary_path(lrc_binary.display().to_string());
            Mutex::new(registry)
        },
        rate_limiter: Mutex::new(RateLimiter::default()),
        sidecar_port: Mutex::new(None),
        configured_agent_count: Mutex::new(0),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        // 注册 IPC 命令（契约：前端通过 invoke 调用）
        .invoke_handler(tauri::generate_handler![
            commands::get_sidecar_status,
            commands::start_sidecar,
            commands::start_sidecar_for_project,
            commands::stop_sidecar,
            commands::stop_sidecar_for_project,
            commands::list_sidecar_projects,
            commands::get_llm_config,
            commands::save_llm_config,
            commands::clear_llm_config,
            commands::test_llm_connection,
            commands::detect_agents,
            commands::detect_installed_agents,
            commands::discover_all_agents,
            commands::configure_agents,
            commands::save_configured_agents,
            commands::scan_ide_projects,
            commands::get_project_dir,
            commands::set_project_dir,
            commands::pick_project_dir,
            commands::get_wizard_state,
            commands::open_dashboard_window,
            commands::navigate_main_to_dashboard,
            commands::open_settings, // v0.5.5 P1-2：从仪表盘打开桌面端设置
            commands::update_tray_tooltip,
            commands::switch_project,
            commands::reset_wizard,
            commands::mark_complete,
            commands::verify_setup,
        ])
        .manage(app_store)
        // v0.5.4 P2-16 调试：页面加载事件追踪
        .on_page_load(|webview, payload| {
            tracing::info!("页面加载事件: {:?} - URL: {}", payload.event(), payload.url());
        })
        .setup(|app| {
            // 构建系统托盘（右键菜单 + 双击打开仪表盘）
            tray::build_tray(app.app_handle())?;

            // ════════════════════════════════════════════════════════════════
            // v0.5.4 P2-14 新增：Sidecar 心跳检测协程
            // 每 10 秒检测 sidecar 进程是否存活，崩溃后自动恢复。
            // 连续 3 次恢复失败后，通过 Tauri 事件通知前端"服务异常"。
            // ════════════════════════════════════════════════════════════════
            let monitor_handle = app.app_handle().clone();
            let (health_shutdown_tx, mut health_shutdown_rx) =
                tokio::sync::watch::channel(false);

            // v0.5.4 P2-14 修复：使用 tauri::async_runtime::spawn 而非 tokio::spawn
            // 原因：setup 回调不在 Tokio 运行时上下文中，直接调用 tokio::spawn 会 panic
            // tauri::async_runtime::spawn 会在 Tauri 管理的 Tokio 运行时中执行任务
            tauri::async_runtime::spawn(async move {
                tracing::info!("Sidecar 心跳检测协程已启动（间隔 10 秒）");
                let mut consecutive_failures = 0u32;
                let mut last_instance_count = 0usize;
                // M-6 修复：cleanup 计数器，每 30 次心跳（约 5 分钟）清理一次过期限流桶
                let mut cleanup_counter = 0u32;

                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {}
                        _ = health_shutdown_rx.changed() => {
                            tracing::info!("心跳检测协程收到关闭信号，退出");
                            break;
                        }
                    }

                    let state = monitor_handle.state::<AppStore>();
                    let mut sidecar = state.sidecar.lock().await;
                    let current_count = sidecar.list_instances().len();

                    if current_count == 0 && last_instance_count > 0 {
                        // 实例数从 > 0 变为 0：说明 sidecar 崩溃且恢复失败
                        consecutive_failures += 1;
                        tracing::error!(
                            "Sidecar 崩溃检测：实例数 {} → 0，连续失败 {} 次",
                            last_instance_count, consecutive_failures
                        );

                        if consecutive_failures >= 3 {
                            // 连续 3 次恢复失败，通知前端
                            let _ = monitor_handle.emit(
                                "sidecar-crash",
                                serde_json::json!({
                                    "message": "服务异常，请手动重启",
                                    "consecutive_failures": consecutive_failures
                                }),
                            );
                            tracing::error!(
                                "Sidecar 连续 {} 次恢复失败，已通知前端",
                                consecutive_failures
                            );
                        }
                    } else {
                        // 尝试恢复死亡的实例
                        // v0.5.7 修复 L-11：先释放 sidecar 锁，再用 tokio::task::spawn 包装
                        // recover_dead_instances，避免死锁并处理 panic
                        drop(sidecar);

                        let recover_handle = monitor_handle.clone();
                        let recover_result = tokio::task::spawn(async move {
                            let state = recover_handle.state::<AppStore>();
                            let mut sidecar = state.sidecar.lock().await;
                            sidecar.recover_dead_instances(None).await
                        }).await;

                        match recover_result {
                            Ok(recovered) if recovered > 0 => {
                                consecutive_failures = 0;
                                tracing::info!("Sidecar 崩溃后自动恢复 {} 个实例", recovered);
                                let _ = monitor_handle.emit(
                                    "sidecar-recovered",
                                    serde_json::json!({
                                        "message": "服务已自动恢复",
                                        "recovered": recovered
                                    }),
                                );
                            }
                            Ok(_) => {
                                // 无需恢复或无实例可恢复
                            }
                            Err(join_err) => {
                                tracing::error!(
                                    "心跳检测：recover_dead_instances 子任务 panic，已恢复并继续监控: {}",
                                    join_err
                                );
                            }
                        }

                        // 重新获取 sidecar 锁以更新 last_instance_count
                        sidecar = state.sidecar.lock().await;
                    }

                    last_instance_count = sidecar.list_instances().len();
                    // 显式释放 sidecar 锁，避免与 rate_limiter 锁同时持有
                    drop(sidecar);

                    // M-6 修复：每 5 分钟（30 次 × 10 秒）清理一次过期的限流桶
                    // 防止 RateLimiter 的 buckets HashMap 因大量客户端连接而无限增长
                    cleanup_counter += 1;
                    if cleanup_counter >= 30 {
                        cleanup_counter = 0;
                        let mut rate_limiter = state.rate_limiter.lock().await;
                        let before = rate_limiter.active_buckets();
                        rate_limiter.cleanup(std::time::Duration::from_secs(300));
                        let after = rate_limiter.active_buckets();
                        if before != after {
                            tracing::info!(
                                "RateLimiter 清理过期桶：{} → {}（清理 {} 个）",
                                before, after, before.saturating_sub(after)
                            );
                        }
                    }
                }
            });

            // ════════════════════════════════════════════════════════════════
            // v0.5.4 修复 C08：应用关闭时显式停止所有 sidecar 进程
            // 防止 spawn_blocking 超时后子进程变为僵尸进程
            // 当用户关闭窗口时，确保所有 sidecar 进程被正确终止
            // ════════════════════════════════════════════════════════════════
            if let Some(window) = app.get_webview_window("main") {
                let app_handle = app.app_handle().clone();
                let shutdown_tx = health_shutdown_tx.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Destroyed = event {
                        // v0.5.4 P2-14：通知心跳检测协程停止
                        let _ = shutdown_tx.send(true);
                        tracing::info!("主窗口已关闭，正在清理所有 sidecar 进程...");
                        let state = app_handle.state::<AppStore>();
                        let rt = tokio::runtime::Handle::current();
                        if let Err(e) = rt.block_on(async {
                            let mut sidecar = state.sidecar.lock().await;
                            sidecar.stop_all().await
                        }) {
                            tracing::error!("应用关闭时停止 sidecar 失败: {e}");
                        } else {
                            tracing::info!("所有 sidecar 进程已清理完毕");
                        }
                    }
                });
            }

            // 显示主窗口（配置向导 / 已就绪面板）
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
                window.set_focus()?;
            }

            tracing::info!("LRC Desktop 启动完成");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动 LRC Desktop 失败");
}

/// v0.5.1 新增：日志系统初始化
///
/// 将日志同时输出到：
/// 1. 控制台（开发模式，tracing_subscriber fmt layer）
/// 2. 文件（%APPDATA%\LoongRecall\logs\lrc-desktop.log）
///
/// 日志文件自动轮转：文件超过 10MB 时自动轮转。
/// 这解决了此前 sidecar 问题无法排查的根本原因——没有持久化日志。
fn init_logging() {
    use std::path::PathBuf;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer; // v0.5.1 修复：需要导入 Layer trait 以使用 with_filter

    // 确定日志目录
    let log_dir = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("LoongRecall")
        .join("logs");

    // 确保日志目录存在
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!("[LRC] 无法创建日志目录 {}: {}", log_dir.display(), e);
        // 回退到仅控制台输出
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .try_init();
        return;
    }

    // v0.5.7 修复 L-5：使用 tracing_appender::rolling 替代手动轮转
    // 原先的 remove_file + rename 非原子操作，崩溃时可能丢失日志文件。
    // tracing_appender::rolling 提供原子轮转，按天自动轮转日志文件。
    // 使用 non_blocking 确保日志写入不阻塞主线程。
    let file_appender = tracing_appender::rolling::daily(&log_dir, "lrc-desktop.log");
    let (non_blocking_file, _guard) = tracing_appender::non_blocking(file_appender);
    // 注意：_guard 必须保持存活，否则日志写入会停止。
    // 但由于 init_logging 在 main 开始时调用，guard 会随进程生命周期存活。
    // 为避免 guard 被 drop，将其泄漏（进程退出时自动清理）
    std::mem::forget(_guard);

    // 构建日志订阅器：同时输出到控制台和文件
    let console_layer = tracing_subscriber::fmt::layer()
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking_file)
        .with_ansi(false) // 文件中不需要 ANSI 颜色代码
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);

    let _ = tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .try_init();

    tracing::info!("═══════════════════════════════════════════════════════");
    tracing::info!("LRC Desktop v{} 启动", env!("CARGO_PKG_VERSION"));
    tracing::info!("日志目录: {}", log_dir.display());
    tracing::info!("═══════════════════════════════════════════════════════");
}