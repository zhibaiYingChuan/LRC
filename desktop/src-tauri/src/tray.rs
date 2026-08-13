/// 系统托盘模块
///
/// 管理托盘图标、右键菜单、状态指示、动态 tooltip。
/// 契约：通过 TrayBuilder 构建，在 setup 阶段注册。
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

use crate::commands::AppStore; // 获取 sidecar 端口

/// 托盘菜单项 ID 常量（契约：前端通过事件监听这些 ID）
pub mod menu_ids {
    pub const TOGGLE_WINDOW: &str = "toggle_window";
    pub const SETTINGS: &str = "settings";
    pub const SWITCH_PROJECT: &str = "switch_project";
    pub const ABOUT: &str = "about";
    pub const QUIT: &str = "quit";
}

/// 托盘图标 ID（用于运行时查找和更新 tooltip）
pub const TRAY_ICON_ID: &str = "lrc-main-tray";

/// 构建系统托盘
///
/// 包含：
/// - 显式设置托盘图标（使用 app 默认图标，解决图标不可见问题）
/// - 左键单击 → 显示/隐藏主窗口（统一入口，不创建新窗口）
/// - 右键菜单（显示/隐藏主窗口、设置、切换项目、关于、退出）
/// - 动态悬浮提示（显示 Agent 数量、运行中的项目列表和端口）
///
/// 产品决策：所有操作统一在主窗口内完成，不再创建独立的仪表盘窗口。
/// 修复 v0.5.1：显式设置托盘图标，解决 Windows 系统托盘图标不可见的问题。
pub fn build_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    // 构建右键菜单（移除"打开仪表盘"，统一在主窗口内操作）
    let menu = MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id(menu_ids::TOGGLE_WINDOW, "显示/隐藏主窗口").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id(menu_ids::SETTINGS, "设置").build(app)?)
        .item(&MenuItemBuilder::with_id(menu_ids::SWITCH_PROJECT, "切换项目").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id(menu_ids::ABOUT, "关于 LRC Desktop").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id(menu_ids::QUIT, "退出").build(app)?)
        .build()?;

    // 获取 app 默认图标（由 tauri.conf.json 的 bundle.icon 定义）
    // v0.5.1 修复：显式设置图标，解决托盘图标在 Windows 系统托盘中不可见的问题
    let tray_icon = app.default_window_icon().cloned();

    // 构建托盘图标（使用唯一 ID 以便后续查找）
    let mut tray_builder = TrayIconBuilder::with_id(TRAY_ICON_ID)
        .menu(&menu)
        .tooltip("LRC Desktop — AI 工具的记忆");

    // 显式设置图标（如果可用）
    if let Some(icon) = tray_icon {
        tray_builder = tray_builder.icon(icon);
        tracing::info!("托盘图标已显式设置");
    } else {
        tracing::warn!("无法获取默认窗口图标，托盘图标可能不可见");
    }

    let _tray = tray_builder
        .on_menu_event(move |app, event| {
            handle_menu_event(app, event.id().as_ref());
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击托盘图标 → 显示/隐藏主窗口（统一入口）
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    tracing::info!("系统托盘已创建 (id={})", TRAY_ICON_ID);
    Ok(())
}

/// 更新托盘悬浮提示
///
/// 根据已配置的 Agent 数量和运行中的项目动态更新 tooltip 文本。
/// 契约：在 configure_agents 或 sidecar 状态变更后调用。
pub fn update_tooltip<R: Runtime>(app: &AppHandle<R>, agent_count: usize) {
    // 获取 sidecar 状态和项目列表
    let (sidecar_running, project_count, project_list) = app
        .try_state::<AppStore>()
        .map(|store| {
            let sidecar = store.sidecar.try_lock();
            match sidecar {
                Ok(guard) => {
                    let instances = guard.list_instances();
                    let running = !instances.is_empty();
                    let count = instances.len();
                    let projects: Vec<String> = instances
                        .iter()
                        .map(|inst| {
                            // 截取项目路径的最后一部分作为显示名称
                            let name = std::path::Path::new(&inst.project_dir)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| inst.project_dir.clone());
                            format!("{} (端口:{})", name, inst.port)
                        })
                        .collect();
                    (running, count, projects)
                }
                Err(_) => (false, 0, Vec::new()),
            }
        })
        .unwrap_or((false, 0, Vec::new()));

    let tooltip = if project_count > 0 {
        format!(
            "LRC Desktop — {} 个 Agent | {} 个项目运行中\n{}",
            agent_count,
            project_count,
            project_list.join("\n")
        )
    } else if sidecar_running {
        format!("LRC Desktop — {} 个 Agent 已连接 | 服务运行中", agent_count)
    } else {
        format!("LRC Desktop — {} 个 Agent 已连接", agent_count)
    };

    if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
        let _ = tray.set_tooltip(Some(&tooltip));
    }
}

/// 处理右键菜单事件
fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, menu_id: &str) {
    match menu_id {
        menu_ids::TOGGLE_WINDOW => {
            toggle_main_window(app);
        }
        menu_ids::SETTINGS => {
            // 打开设置页面（在主窗口 WebView 中导航）
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval("window.location.hash = '#settings'");
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        menu_ids::SWITCH_PROJECT => {
            // 打开项目切换界面（在主窗口中导航）
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval("window.location.hash = '#wizard-switch-project'");
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        menu_ids::ABOUT => {
            // 打开关于页面（在主窗口中导航）
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval("window.location.hash = '#about'");
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        menu_ids::QUIT => {
            tracing::info!("用户从托盘菜单退出");
            app.exit(0);
        }
        _ => {
            tracing::warn!("未知托盘菜单项: {menu_id}");
        }
    }
}

/// 切换主窗口的显示/隐藏状态
///
/// 产品决策：所有操作统一在主窗口内完成，不创建新窗口。
/// 左键单击托盘和"显示/隐藏主窗口"菜单项都调用此函数。
fn toggle_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            tracing::info!("主窗口已隐藏");
        } else {
            let _ = window.show();
            let _ = window.set_focus();
            tracing::info!("主窗口已显示");
        }
    }
}

/// 在主窗口 iframe 中打开仪表盘（统一入口，不创建新窗口）
///
/// 【产品决策】所有操作统一在主窗口内完成，不创建独立窗口。
/// 仪表盘通过 iframe 嵌入在主窗口的 wizard HTML 中。
/// 此函数作为回退方案，仅在主窗口不存在时使用。
///
/// 契约：仪表盘由 sidecar HTTP 服务提供，通过主窗口 iframe 加载。
pub fn open_dashboard<R: Runtime>(app: &AppHandle<R>) {
    // v0.9.0 开发模式隔离：开发模式默认端口 3111
    let is_dev = std::env::var("TAURI_DEV").is_ok() || std::env::var("LRC_DEV_MODE").is_ok();
    let dev_default = if is_dev { 3111 } else { 3099 };
    // 获取 sidecar 实际端口
    let port = app
        .try_state::<AppStore>()
        .and_then(|store| store.sidecar_port.try_lock().ok().and_then(|guard| *guard))
        .unwrap_or(dev_default);

    // 在主窗口 iframe 中显示仪表盘（统一入口）
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let js = format!("if(window.Wizard&&window.Wizard.showDashboardEmbed){{window.Wizard.showDashboardEmbed({})}}else{{console.warn('showDashboardEmbed 未定义')}}", port);
        let _ = window.eval(&js);
        tracing::info!("仪表盘已在主窗口 iframe 中打开 (port={port})");
    } else {
        tracing::error!("主窗口不存在，无法打开仪表盘");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD：验证菜单 ID 常量定义完整
    #[test]
    fn test_menu_ids_defined() {
        assert!(!menu_ids::TOGGLE_WINDOW.is_empty());
        assert!(!menu_ids::SETTINGS.is_empty());
        assert!(!menu_ids::SWITCH_PROJECT.is_empty());
        assert!(!menu_ids::ABOUT.is_empty());
        assert!(!menu_ids::QUIT.is_empty());
    }

    /// TDD：验证菜单 ID 互不相同
    #[test]
    fn test_menu_ids_unique() {
        let ids = [
            menu_ids::TOGGLE_WINDOW,
            menu_ids::SETTINGS,
            menu_ids::SWITCH_PROJECT,
            menu_ids::ABOUT,
            menu_ids::QUIT,
        ];
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "菜单 ID 必须唯一");
    }

    /// TDD：验证托盘图标 ID 已定义
    #[test]
    fn test_tray_icon_id_defined() {
        assert!(!TRAY_ICON_ID.is_empty());
        assert_eq!(TRAY_ICON_ID, "lrc-main-tray");
    }
}
