/// LRC Desktop — Tauri 壳层主入口
///
/// 职责：
/// 1. 管理系统托盘（右键菜单、状态指示）
/// 2. 管理后台 sidecar 进程（code-memory-server）
/// 3. 嵌入仪表盘 WebView
/// 4. 首次配置向导
/// 5. Agent 自动检测与配置
///
/// 契约：所有 IPC 通信通过 Tauri Commands 进行，前端不直接调用 sidecar。
use lrc_desktop_lib::{agent_detector, commands, config_wizard, integrity, sidecar_manager, tray};
use commands::AppStore;
use agent_detector::AgentDetectorRegistry;
use config_wizard::WizardState;
use sidecar_manager::SidecarManager;
use tauri::Manager; // Manager trait 提供 app_handle() 等方法
use tokio::sync::Mutex; // Tauri 2 异步命令需要 tokio::sync::Mutex (支持 Send)

fn main() {
    // 初始化日志
    tracing_subscriber::fmt::init();

    // ── L2 保密层：启动时完整性校验 ──
    if let Err(e) = integrity::IntegrityChecker::verify_on_startup() {
        tracing::error!("L2 完整性校验失败: {e}");
        // 静默退出，不弹出提示（避免暴露校验逻辑）
        std::process::exit(1);
    }

    // 初始化全局状态
    let app_store = AppStore {
        wizard: Mutex::new(WizardState::load()),
        sidecar: Mutex::new(SidecarManager::new(
            // sidecar 二进制路径（与桌面应用同级目录）
            std::env::current_exe()
                .unwrap_or_default()
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("code-memory-server")
                .with_extension(std::env::consts::EXE_EXTENSION)
                .display()
                .to_string(),
        )),
        agent_registry: Mutex::new(AgentDetectorRegistry::new()),
        sidecar_port: Mutex::new(None),
        configured_agent_count: Mutex::new(0),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        // 注册 IPC 命令（契约：前端通过 invoke 调用）
        .invoke_handler(tauri::generate_handler![
            commands::get_sidecar_status,
            commands::start_sidecar,
            commands::stop_sidecar,
            commands::get_llm_config,
            commands::save_llm_config,
            commands::test_llm_connection,
            commands::detect_agents,
            commands::detect_installed_agents,
            commands::discover_all_agents,
            commands::configure_agents,
            commands::scan_ide_projects,
            commands::get_project_dir,
            commands::set_project_dir,
            commands::get_wizard_state,
            commands::open_dashboard_window,
            commands::navigate_main_to_dashboard,
            commands::update_tray_tooltip,
            commands::switch_project,
        ])
        .manage(app_store)
        .setup(|app| {
            // 构建系统托盘（右键菜单 + 双击打开仪表盘）
            tray::build_tray(app.app_handle())?;

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