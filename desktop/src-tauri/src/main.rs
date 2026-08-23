// 隐藏控制台窗口：桌面应用不需要 CMD 窗口
// 普通用户看到 CMD 窗口会困惑，且关闭可能导致后端进程异常
#![windows_subsystem = "windows"]

use agent_detector::AgentDetectorRegistry;
use commands::AppStore;
use config_wizard::WizardState;
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
use lrc_desktop_lib::{
    agent_detector, commands, config_wizard, integrity, rate_limiter, sidecar_manager, tray,
};
use rate_limiter::RateLimiter;
use sidecar_manager::SidecarManager;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::Emitter; // v0.5.4 P2-14: Emitter trait 提供 emit() 方法，用于心跳协程通知前端
use tauri::Manager; // Manager trait 提供 app_handle() 等方法
use tauri::WindowEvent;
use tokio::sync::Mutex; // Tauri 2 异步命令需要 tokio::sync::Mutex (支持 Send) // v0.5.4: 窗口事件监听，用于应用关闭时清理 sidecar

fn main() {
    // ════════════════════════════════════════════════════════════════
    // v0.8.30 新增：WebView2 CDP 调试支持
    // 通过环境变量 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS 注入
    // --remote-debugging-port=9230 参数，启用 Chrome DevTools Protocol
    // 这样可以通过 CDP 自动化测试桌面端 WebView2 交互
    // 注意：必须在 Tauri 初始化前设置，否则 WebView2 已启动无法修改
    // ════════════════════════════════════════════════════════════════
    #[cfg(target_os = "windows")]
    {
        // 检测是否处于开发模式（TAURI_DEV 由 cargo tauri dev 自动设置，LRC_DEV_MODE 供手动设置）
        let is_dev = std::env::var("TAURI_DEV").is_ok() || std::env::var("LRC_DEV_MODE").is_ok();
        // v0.9.0 开发模式隔离：开发模式 CDP 端口 9231，稳定版 9230
        let cdp_port = if is_dev { "9231" } else { "9230" };
        // 读取现有环境变量（避免覆盖其他已有参数）
        let existing = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
        // CDP 参数：绑定 127.0.0.1 + 端口 + 允许所有 Origin 连接（开发/测试用）
        let cdp_args = format!(
            "--remote-debugging-address=127.0.0.1 --remote-debugging-port={} --remote-allow-origins=*",
            cdp_port
        );
        let combined = if existing.trim().is_empty() {
            cdp_args
        } else {
            format!("{} {}", existing.trim(), cdp_args)
        };
        // 先记录日志（防止后续 move 后无法引用）
        tracing::info!(
            "[CDP 调试] WebView2 环境变量已设置 (端口: {}): {}",
            cdp_port,
            &combined
        );
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", combined);
    }

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
        tracing::error!("═══════════════════════════════════════════════════════");
        tracing::error!(
            "LRC Sidecar 二进制文件不存在: {}",
            sidecar_binary_path.display()
        );
        tracing::error!("请先编译主项目: cargo build --release --features server");
        tracing::error!("或重新安装 LRC Desktop 以获取完整的 sidecar 二进制");
        tracing::error!("═══════════════════════════════════════════════════════");
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
            Arc::new(Mutex::new(registry))
        },
        rate_limiter: Mutex::new(RateLimiter::default()),
        sidecar_port: Mutex::new(None),
        configured_agent_count: Mutex::new(0),
        start_cancel_flag: Arc::new(AtomicBool::new(false)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        // 注册 IPC 命令（契约：前端通过 invoke 调用）
        .invoke_handler(tauri::generate_handler![
            commands::get_sidecar_status,
            commands::start_sidecar,
            commands::start_sidecar_for_project,
            commands::cancel_start_sidecar,
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
            commands::get_rules_status, // v0.8.0：信任中心规则状态查询
            commands::set_agent_manual_override, // v0.8.31 S-03：AI工具手动修正（向导齿轮图标）
            commands::bulk_apply_agent_overrides, // 批量应用 AI 工具手动修正
            commands::get_scan_cache_metadata,    // v0.8.31 S-05：获取扫描缓存元数据（时间戳+TTL）
            commands::force_invalidate_scan_cache, // v0.8.31 S-05：前端「重新扫描」按钮强制失效缓存
        ])
        .manage(app_store)
        // v0.5.4 P2-16 调试：页面加载事件追踪
        .on_page_load(|_webview, payload| {
            tracing::info!(
                "页面加载事件: {:?} - URL: {}",
                payload.event(),
                payload.url()
            );
        })
        .setup(|app| {
            // 构建系统托盘（右键菜单 + 双击打开仪表盘）
            tray::build_tray(app.app_handle())?;

            // ════════════════════════════════════════════════════════════════
            // v0.8.0 "归一" 新增：启动时自动写入 AI 规则文件
            // 不依赖 sidecar 启动，确保全新安装后首次启动即写入规则
            // 使用异步任务执行，不阻塞 setup() 回调
            // 规则写入失败时通过 Tauri 事件通知前端显示提示
            // ════════════════════════════════════════════════════════════════
            let rules_handle = app.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                tracing::info!("[v0.8.0] 启动时自动写入 AI 规则文件（不依赖 sidecar）");
                let registry = AgentDetectorRegistry::new();
                // 只写入已安装工具的规则，并以版本和工具快照判断是否需要重复写入。
                let tool_ids: Vec<String> = registry
                    .detect_installed()
                    .into_iter()
                    .map(|a| a.id)
                    .collect();
                let needs_update = {
                    let state = rules_handle.state::<AppStore>();
                    let wizard = state.wizard.lock().await;
                    wizard.rules_need_update(&tool_ids)
                };
                if !needs_update {
                    tracing::info!("[AI规则] 规则版本和工具列表未变化，跳过启动重复写入");
                    return;
                }
                tracing::info!("[AI规则] 检测到规则版本或工具列表变化，写入 {} 个工具", tool_ids.len());

                match registry.write_rules_for_agents(&tool_ids) {
                    Ok(written) => {
                        tracing::info!(
                            "[v0.8.0] 启动时规则写入完成，成功 {} 个工具: {:?}",
                            written.len(),
                            written
                        );
                        {
                            let state = rules_handle.state::<AppStore>();
                            let mut wizard = state.wizard.lock().await;
                            if let Err(e) = wizard.save_rules_state(written.clone()) {
                                tracing::warn!("[AI规则] 规则状态持久化失败: {}", e);
                            }
                        }
                        // 通知前端规则写入成功
                        let _ = rules_handle.emit(
                            "rules-write-completed",
                            serde_json::json!({
                                "success": true,
                                "written_count": written.len(),
                                "total_count": tool_ids.len(),
                                "tools": written,
                            }),
                        );
                    }
                    Err(e) => {
                        tracing::error!("[v0.8.0] 启动时规则写入失败: {}", e);
                        // 通知前端规则写入失败
                        let _ = rules_handle.emit(
                            "rules-write-failed",
                            serde_json::json!({
                                "success": false,
                                "error": e,
                                "message": "规则文件写入失败，AI 助手可能无法自动调用记忆工具",
                            }),
                        );
                    }
                }
            });

            // ════════════════════════════════════════════════════════════════
            // v0.5.4 P2-14 新增：Sidecar 心跳检测协程
            // 每 10 秒检测 sidecar 进程是否存活，崩溃后自动恢复。
            // 连续 3 次恢复失败后，通过 Tauri 事件通知前端"服务异常"。
            // ════════════════════════════════════════════════════════════════
            let monitor_handle = app.app_handle().clone();
            let (health_shutdown_tx, mut health_shutdown_rx) = tokio::sync::watch::channel(false);

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
                            // v0.9.0 开发模式隔离：优先选择开发端口 3111，避免意外连接稳定版
                            let is_dev_mode = std::env::var("TAURI_DEV").is_ok()
                                || std::env::var("LRC_DEV_MODE").is_ok();
                            if is_dev_mode {
                                // 开发模式：优先选 3111（开发端口），找不到则跳过（禁止回退到稳定版 3099）
                                if let Some(dev_sidecar) = probed.iter().find(|p| p.port == 3111) {
                                    let mut sidecar_port = state.sidecar_port.lock().await;
                                    *sidecar_port = Some(dev_sidecar.port);
                                    tracing::info!(
                                        "启动时探测：检测到外部 sidecar，端口 {}，项目 {} [开发模式]",
                                        dev_sidecar.port,
                                        if dev_sidecar.src_dir.is_empty() { "unknown" } else { &dev_sidecar.src_dir }
                                    );
                                    let _ = monitor_handle.emit(
                                        "sidecar-detected",
                                        serde_json::json!({
                                            "port": dev_sidecar.port,
                                            "src_dir": dev_sidecar.src_dir,
                                            "message": "检测到已运行的 LRC 服务"
                                        }),
                                    );
                                } else {
                                    tracing::warn!(
                                        "[开发模式] 未找到 3111 端口的开发版 sidecar，跳过探测（禁止回退到稳定版）"
                                    );
                                }
                            } else {
                                // 短暂获取 sidecar_port 锁存储结果
                                {
                                    let mut sidecar_port = state.sidecar_port.lock().await;
                                    *sidecar_port = Some(probed[0].port);
                                }
                                tracing::info!(
                                    "启动时探测：检测到外部 sidecar，端口 {}，项目 {}",
                                    probed[0].port,
                                    if probed[0].src_dir.is_empty() {
                                        "unknown"
                                    } else {
                                        &probed[0].src_dir
                                    }
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
                            }
                        } else {
                            tracing::info!("启动时探测：未检测到外部 sidecar");
                        }
                    }
                }

                // ════════════════════════════════════════════════════════════════
                // v0.8.16 入口体验修复：自动启动 sidecar
                // 用户痛点："打开桌面端，它不应该自动启动后端吗？"
                // 设计原则：与 VSCode/Cursor 等主流桌面应用对齐，打开即用
                //
                // 自动启动条件：
                //   1. 桌面端无管理的 sidecar 实例（sidecar_running == false）
                //   2. 未探测到外部 sidecar（probed 为空或未执行探测）
                //   3. wizard.setup_complete == true（首次安装不自动启动，引导用户走向导）
                //
                // 失败处理：
                //   - 自动启动失败后不重试（与心跳协程的自动恢复不同）
                //   - 仅发射 sidecar-auto-start-failed 事件，前端显示横幅让用户手动启动
                // ════════════════════════════════════════════════════════════════
                {
                    let state = monitor_handle.state::<AppStore>();
                    // 检查 wizard 是否已完成配置（首次安装不自动启动）
                    // v0.8.21 P0-01 修复（interaction-resilience-auditor）：
                    //   根因：wizard.json 文件意外丢失时，WizardState::load() 返回默认配置
                    //         (setup_complete=false)，导致 sidecar 永不自动启动，用户被困
                    //   修复：wizard.json 不存在时（file_existed=false）兜底视为已完成配置
                    //         - 首次安装：用户通过向导完成配置后 wizard.json 才会生成
                    //         - 文件丢失：sidecar 自动启动（全局模式），用户可继续使用
                    let (setup_complete, file_existed) = {
                        let wizard = state.wizard.lock().await;
                        (wizard.config().setup_complete, wizard.file_existed)
                    }; // wizard 锁立即释放

                    // P0-01 兜底：wizard.json 不存在时强制视为已完成配置
                    let effective_setup_complete = setup_complete || !file_existed;
                    if !file_existed && !setup_complete {
                        tracing::warn!(
                            "[v0.8.21 自动启动] wizard.json 不存在（file_existed=false），兜底视为已完成配置以避免 sidecar 永不自动启动"
                        );
                    }

                    if effective_setup_complete {
                        // 再次检查 sidecar 是否已在运行（probe 可能已检测到外部 sidecar）
                        let sidecar_running = {
                            let sidecar = state.sidecar.lock().await;
                            sidecar.is_running()
                        }; // sidecar 锁立即释放

                        if !sidecar_running {
                            tracing::info!("[v0.8.16 自动启动] wizard 已完成配置，自动启动 sidecar（全局模式）");
                            // 通知前端：正在自动启动
                            let _ = monitor_handle.emit(
                                "sidecar-auto-starting",
                                serde_json::json!({
                                    "message": "正在自动启动 LRC 服务..."
                                }),
                            );

                            // P0-2 + P1-2 修复：用 tokio::time::timeout 包裹 start_sidecar
                            // 根因：start_sidecar 卡死时（如 reqwest DNS 挂起），心跳 loop 永远不会启动
                            // v0.8.21 INV-08 修复：60s → 120s
                            //   根因：实测 sidecar 首次启动 + 索引初始化 + 健康检查可达 100s+，
                            //         60s 超时误判启动失败，导致用户看到"无法连接"
                            //   修复：提升到 120s，与 handleStartServiceClick 前端超时一致
                            // 超时后发射失败事件，确保心跳 loop 能继续启动
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(120),
                                commands::start_sidecar(
                                    state.clone(),
                                    monitor_handle.clone(),
                                    None,  // src_dir=None → 全局模式
                                    None,  // port=None → 自动选择
                                    None,  // multi_window=None → 默认值
                                ),
                            ).await {
                                Ok(Ok(port)) => {
                                    tracing::info!("[v0.8.16 自动启动] sidecar 启动成功，端口 {}", port);
                                    let _ = monitor_handle.emit(
                                        "sidecar-auto-started",
                                        serde_json::json!({
                                            "port": port,
                                            "message": "LRC 服务已自动启动"
                                        }),
                                    );
                                }
                                Ok(Err(e)) => {
                                    // v0.8.19 P0-2 修复（GAP-03/INV-010）：用结构化标记替代中文字符串匹配
                                    // 根因：v0.8.18 用 err_str.contains("已有 LRC 实例在运行") 匹配 E008，
                                    //   Display 措辞变更即静默失效（INV-010 违规）。
                                    // 修复：commands.rs 的 sidecar_error_to_user_message 已加入结构化标记：
                                    //   - E008+port: "[E008:port=XXX] ..."
                                    //   - E008+noport: "[E008:noport] ..."
                                    //   - E006 PortConflict: "端口 XXX 已被其他 LRC 服务占用..."
                                    // 策略：
                                    //   - PortConflict (E006) / [E008:port=] → 静默复用现有实例，发 started 事件
                                    //   - [E008:noport] → 僵尸 sidecar 场景，发 failed 事件并附正确清理提示
                                    let err_str = e.to_string();
                                    let is_port_conflict = err_str.contains("端口")
                                        && err_str.contains("已被其他 LRC 服务占用");
                                    let is_e008_with_port = err_str.contains("[E008:port=");
                                    let is_e008_noport = err_str.contains("[E008:noport]");
                                    if is_port_conflict || is_e008_with_port {
                                        tracing::info!(
                                            "[v0.8.19 自动启动] sidecar 已在运行（port_conflict={}, e008_with_port={}），静默复用: {}",
                                            is_port_conflict, is_e008_with_port, err_str
                                        );
                                        let _ = monitor_handle.emit(
                                            "sidecar-auto-started",
                                            serde_json::json!({
                                                "port": 0,
                                                "message": "LRC 服务已在运行"
                                            }),
                                        );
                                    } else if is_e008_noport {
                                        // 僵尸 sidecar 场景：进程活着但 /health 卡死，或锁文件 PID 被新进程复用
                                        // v0.8.19 P1-1 修复（INV-009）：清理提示路径改为正确路径
                                        //   旧（错误）：%APPDATA%\LoongRecall\.lrc.lock（该路径不存在）
                                        //   新（正确）：~/.loong-recall/ 下的 .lrc.lock 文件
                                        tracing::warn!(
                                            "[v0.8.19 自动启动] 检测到僵尸 sidecar（E008:noport），提示用户清理: {}",
                                            err_str
                                        );
                                        let home_dir = dirs::home_dir()
                                            .map(|p| p.display().to_string())
                                            .unwrap_or_else(|| "<用户主目录>".to_string());
                                        let _ = monitor_handle.emit(
                                            "sidecar-auto-start-failed",
                                            serde_json::json!({
                                                "error": e,
                                                "message": format!(
                                                    "检测到残留 LRC 进程但无法连接（可能已卡死）。\n请执行以下步骤清理：\n1. 打开任务管理器结束所有 code-memory-server.exe 进程\n2. 删除锁文件：{home}\\.loong-recall\\global\\data\\.lrc.lock\n   或 {home}\\.loong-recall\\projects\\*\\data\\.lrc.lock\n3. 重新打开 LRC 桌面端",
                                                    home = home_dir
                                                )
                                            }),
                                        );
                                    } else {
                                        tracing::error!("[v0.8.19 自动启动] sidecar 启动失败: {}", e);
                                        let _ = monitor_handle.emit(
                                            "sidecar-auto-start-failed",
                                            serde_json::json!({
                                                "error": e,
                                                "message": "LRC 服务自动启动失败，请手动启动"
                                            }),
                                        );
                                    }
                                }
                                Err(_elapsed) => {
                                    // P0-2 修复：整体超时（120s），确保心跳 loop 能继续启动
                                    // v0.8.21 INV-08：超时从 60s 提升到 120s
                                    tracing::error!("[v0.8.16 自动启动] sidecar 启动整体超时（120s）");
                                    let _ = monitor_handle.emit(
                                        "sidecar-auto-start-failed",
                                        serde_json::json!({
                                            "error": "自动启动整体超时（120s）",
                                            "message": "LRC 服务自动启动超时，请手动启动"
                                        }),
                                    );
                                }
                            }
                        } else {
                            tracing::info!("[v0.8.16 自动启动] sidecar 已在运行，跳过自动启动");
                        }
                    } else {
                        tracing::info!(
                            "[v0.8.16 自动启动] wizard 未完成配置（setup_complete=false, file_existed={}），跳过自动启动，引导用户走向导",
                            file_existed
                        );
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
                            last_instance_count,
                            consecutive_failures
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
                            // type alias 简化复杂类型（clippy::type_complexity）
                            type RecoveredHandle = (
                                String,
                                std::process::Child,
                                u16,
                                Option<String>,
                                Option<u32>,
                                Option<String>,
                            );
                            let mut recovered_handles: Vec<RecoveredHandle> = Vec::new();
                            // v0.8.9 G-001：心跳恢复使用独立的 cancel_flag，不与用户启动取消共享
                            // 心跳恢复是后台自动行为，不应被用户的 cancel_start_sidecar 干扰
                            // 心跳自身的关闭通过 health_shutdown_rx 控制
                            let heartbeat_cancel = std::sync::atomic::AtomicBool::new(false);

                            for info in dead_instances {
                                use sidecar_manager::{
                                    DeadInstanceInfo, SidecarManager, StartOptions,
                                };
                                let DeadInstanceInfo {
                                    project_key,
                                    src_dir,
                                    multi_window,
                                    llm_api,
                                } = info;

                                // v0.8.37 开发模式使用独立数据目录
                                // v0.9.0 修复：崩溃恢复在 dev 模式下也必须用 3111，
                                // 而非 None→默认 3099，否则会复用稳定版 sidecar（违反开发/稳定隔离）。
                                let is_dev = std::env::var("TAURI_DEV").is_ok()
                                    || std::env::var("LRC_DEV_MODE").is_ok();
                                let _dev_dd = if is_dev {
                                    let home = std::env::var("USERPROFILE")
                                        .or_else(|_| std::env::var("HOME"))
                                        .unwrap_or_else(|_| ".".to_string());
                                    Some(format!("{}/.loong-recall/dev/data", home))
                                } else {
                                    None
                                };
                                let target_port = if is_dev {
                                    3111
                                } else {
                                    sidecar_manager::DEFAULT_SIDECAR_PORT
                                };
                                let start_opts = StartOptions {
                                    src_dir: src_dir.as_deref(),
                                    port: Some(target_port),
                                    multi_window,
                                    llm_api: llm_api.as_deref(),
                                    cancel_flag: &heartbeat_cancel,
                                    progress_tx: None, // G-003：心跳恢复不需要进度反馈
                                    data_dir: _dev_dd.as_deref(),
                                };
                                match SidecarManager::spawn_and_wait(
                                    &binary_path,
                                    &project_key,
                                    &start_opts,
                                )
                                .await
                                {
                                    Ok((child, port)) => {
                                        tracing::info!(
                                            "Sidecar 崩溃恢复成功: 项目={}, 新端口={}",
                                            project_key,
                                            port
                                        );
                                        recovered_handles.push((
                                            project_key,
                                            child,
                                            port,
                                            src_dir,
                                            multi_window,
                                            llm_api,
                                        ));
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Sidecar 崩溃恢复失败: 项目={}, 错误: {}",
                                            project_key,
                                            e
                                        );
                                    }
                                }
                            }

                            // Phase 3: 插入恢复的实例（重新获取锁，无 I/O）
                            let recovered_count = recovered_handles.len();
                            if recovered_count > 0 {
                                let state = monitor_handle.state::<AppStore>();
                                let mut sidecar = state.sidecar.lock().await;
                                for (key, child, port, src_dir, multi_window, llm_api) in
                                    recovered_handles
                                {
                                    sidecar.insert_handle(
                                        &key,
                                        child,
                                        port,
                                        src_dir,
                                        multi_window,
                                        llm_api,
                                    );
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
                                before,
                                after,
                                before.saturating_sub(after)
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
    // ──────────────────────────────────────────────────────────────
    // v0.7.1 P3-5 说明：关于 std::mem::forget(_guard) 的安全性
    // ──────────────────────────────────────────────────────────────
    // 1. WorkerGuard 必须保持存活到进程结束，否则 non_blocking 缓冲区中
    //    未刷新的日志事件会丢失（drop 时 guard 会 flush 并关闭通道）。
    // 2. 此处使用 std::mem::forget 是 tracing-appender 官方推荐模式之一，
    //    参考：https://docs.rs/tracing-appender/latest/tracing_appender/non_blocking/index.html
    //    官方文档明确指出："The guard returned by non_blocking should be kept
    //    alive for the entire duration you wish for logs to be written."
    // 3. 替代方案为将 guard 存入 Tauri State，但 WorkerGuard 不是 Send，
    //    无法跨线程传递（Tauri State 要求 Send + Sync），因此不适用。
    // 4. 这不是"内存泄漏"——guard 内部仅持有通道句柄，无外部资源（文件句柄
    //    由 file_appender 持有并通过 Drop 关闭）；进程退出时由 OS 自动回收。
    // 5. 可选改进：将 guard 封装到 fn main() 作用域变量中，让其在 main
    //    返回时 drop（当前 init_logging 是独立函数，需调整签名）。
    std::mem::forget(_guard);

    // 构建日志订阅器：同时输出到控制台和文件
    let console_layer =
        tracing_subscriber::fmt::layer().with_filter(tracing_subscriber::filter::LevelFilter::INFO);

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
