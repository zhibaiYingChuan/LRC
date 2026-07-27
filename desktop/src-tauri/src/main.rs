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
    let sidecar_binary_path = std::env::current_exe()
        .unwrap_or_default()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("lrc-sidecar")
        .with_extension(std::env::consts::EXE_EXTENSION);

    // v0.6.0 P0-D 修复：启动时检查 sidecar 二进制是否存在
    // 若不存在，打印明确的错误日志（不阻断启动，让用户能看到提示）
    if !sidecar_binary_path.exists() {
        tracing::error!(
            "═══════════════════════════════════════════════════════"
        );
        tracing::error!("LRC Sidecar 二进制文件不存在: {}", sidecar_binary_path.display());
        tracing::error!("请先编译主项目: cargo build --release --features server");
        tracing::error!("或重新安装 LRC Desktop 以获取完整的 sidecar 二进制");
        tracing::error!(
            "═══════════════════════════════════════════════════════"
        );
    } else {
        tracing::info!("LRC Sidecar 二进制: {}", sidecar_binary_path.display());
    }

    let app_store = AppStore {
        // v0.6.0 P3-1 修复：expect 改为 unwrap_or_else 优雅降级，避免配置目录异常时 panic
        wizard: Mutex::new(WizardState::load().unwrap_or_else(|e| {
            tracing::warn!("加载向导状态失败，使用默认状态: {}", e);
            WizardState::default()
        })),
        sidecar: Mutex::new(SidecarManager::new(
            sidecar_binary_path.display().to_string(),
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
            commands::get_agent_config_guide,
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
            commands::open_data_dir, // v0.6.0：右下角"数据目录"点击打开文件夹
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

                // ════════════════════════════════════════════════════════════════
                // v0.5.16 修复：启动时探测端口上已运行的外部 sidecar
                // 场景：用户先打开 IDE（MCP 已连接 sidecar），再打开桌面端，
                //       桌面端的 instances HashMap 为空，但 sidecar 实际已在端口上运行。
                //
                // 安全设计（与 v0.5.15 的关键区别）：
                //   1. 短暂持有 sidecar 锁仅检查 is_running()，立即释放（<1μs）
                //   2. 用关联函数 SidecarManager::probe_existing_sidecar() 扫描端口，
                //      不持有 sidecar 锁，不会阻塞 start_sidecar 等命令
                //   3. 扫描完成后，短暂获取 sidecar_port 锁存储结果（<1μs）
                //
                // v0.5.15 的错误：在持有 sidecar 锁时调用 probe_existing_sidecar()，
                //   导致 500ms 内所有需要 sidecar 锁的命令被阻塞，前端超时级联失败。
                // ════════════════════════════════════════════════════════════════
                {
                    let state = monitor_handle.state::<AppStore>();
                    let sidecar_running = {
                        let sidecar = state.sidecar.lock().await;
                        sidecar.is_running()
                    }; // sidecar 锁立即释放

                    if !sidecar_running {
                        tracing::info!("启动时探测：桌面端无管理的实例，扫描端口上的外部 sidecar");
                        // 关联函数调用，不持有 sidecar 锁，不会阻塞 start_sidecar 等命令
                        let probed = SidecarManager::probe_existing_sidecar().await;
                        if !probed.is_empty() {
                            // 短暂获取 sidecar_port 锁存储结果
                            {
                                let mut sidecar_port = state.sidecar_port.lock().await;
                                *sidecar_port = Some(probed[0].port);
                            }
                            tracing::info!(
                                "启动时探测：检测到外部 sidecar，端口 {}，项目 {}",
                                probed[0].port,
                                if probed[0].src_dir.is_empty() { "unknown" } else { &probed[0].src_dir }
                            );
                            // 通知前端状态已更新（前端可据此刷新向导状态）
                            let _ = monitor_handle.emit(
                                "sidecar-detected",
                                serde_json::json!({
                                    "port": probed[0].port,
                                    "src_dir": probed[0].src_dir,
                                    "message": "检测到已运行的 LRC 服务"
                                }),
                            );
                        } else {
                            tracing::info!("启动时探测：未检测到外部 sidecar");
                        }
                    }
                }

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
                        // v0.5.17 三阶段锁安全模式：避免在持有 sidecar 锁时执行
                        // spawn_and_wait（最多 40s），改为三阶段编排：
                        //   Phase 1: collect_dead_instances（持锁，<1ms）→ 释放锁
                        //   Phase 2: 循环 spawn_and_wait（不持锁，I/O）
                        //   Phase 3: 循环 insert_handle（重新获取锁，<1ms）
                        drop(sidecar);

                        // Phase 1: 收集死亡实例（持锁，无 I/O）
                        let (dead_instances, binary_path) = {
                            let state = monitor_handle.state::<AppStore>();
                            let mut sidecar = state.sidecar.lock().await;
                            let dead = sidecar.collect_dead_instances();
                            let binary = sidecar.binary_path().to_string();
                            (dead, binary)
                        }; // sidecar 锁立即释放

                        let recovered_count = if dead_instances.is_empty() {
                            0usize
                        } else {
                            // Phase 2: 逐个重启死亡实例（不持锁，I/O）
                            let mut recovered_handles: Vec<(String, std::process::Child, u16, Option<String>, Option<u32>, Option<String>)> = Vec::new();

                            for info in dead_instances {
                                use sidecar_manager::{DeadInstanceInfo, SidecarManager};
                                let DeadInstanceInfo {
                                    project_key,
                                    src_dir,
                                    multi_window,
                                    llm_api,
                                } = info;

                                match SidecarManager::spawn_and_wait(
                                    &binary_path,
                                    &project_key,
                                    src_dir.as_deref(),
                                    None,
                                    multi_window,
                                    llm_api.as_deref(),
                                ).await {
                                    Ok((child, port)) => {
                                        tracing::info!(
                                            "Sidecar 崩溃恢复成功: 项目={}, 新端口={}",
                                            project_key, port
                                        );
                                        recovered_handles.push((project_key, child, port, src_dir, multi_window, llm_api));
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Sidecar 崩溃恢复失败: 项目={}, 错误: {}",
                                            project_key, e
                                        );
                                    }
                                }
                            }

                            // Phase 3: 插入恢复的实例（重新获取锁，无 I/O）
                            let recovered_count = recovered_handles.len();
                            if recovered_count > 0 {
                                let state = monitor_handle.state::<AppStore>();
                                let mut sidecar = state.sidecar.lock().await;
                                for (key, child, port, src_dir, multi_window, llm_api) in recovered_handles {
                                    sidecar.insert_handle(&key, child, port, src_dir, multi_window, llm_api);
                                }
                            }

                            recovered_count
                        };

                        if recovered_count > 0 {
                            consecutive_failures = 0;
                            tracing::info!("Sidecar 崩溃后自动恢复 {} 个实例", recovered_count);
                            let _ = monitor_handle.emit(
                                "sidecar-recovered",
                                serde_json::json!({
                                    "message": "服务已自动恢复",
                                    "recovered": recovered_count
                                }),
                            );
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
            // v0.5.14 架构调整：桌面端关闭时不再停止 sidecar 进程
            // 桌面端只是 MCP 服务的配置工具，sidecar 作为独立后台服务运行。
            // 关闭桌面端不影响 MCP 服务可用性，用户可通过桌面端"停止服务"按钮停止。
            // 仅停止桌面端管理的内部协程（心跳检测等）。
            // ════════════════════════════════════════════════════════════════
            if let Some(window) = app.get_webview_window("main") {
                let shutdown_tx = health_shutdown_tx.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Destroyed = event {
                        // v0.5.4 P2-14：通知心跳检测协程停止
                        let _ = shutdown_tx.send(true);
                        tracing::info!("主窗口已关闭，sidecar 服务将继续在后台运行");
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