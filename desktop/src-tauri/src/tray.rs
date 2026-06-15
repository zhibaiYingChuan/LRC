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
    pub const OPEN_DASHBOARD: &str = "open_dashboard";
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
/// - 双击打开仪表盘
/// - 右键菜单（打开仪表盘/设置/切换项目/关于/退出）
/// - 动态悬浮提示（显示 Agent 数量和连接状态）
pub fn build_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    // 构建右键菜单
    let menu = MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id(menu_ids::OPEN_DASHBOARD, "打开仪表盘").build(app)?)
        .item(&MenuItemBuilder::with_id(menu_ids::SETTINGS, "设置").build(app)?)
        .item(&MenuItemBuilder::with_id(menu_ids::SWITCH_PROJECT, "切换项目").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id(menu_ids::ABOUT, "关于 LRC Desktop").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id(menu_ids::QUIT, "退出").build(app)?)
        .build()?;

    // 构建托盘图标（使用唯一 ID 以便后续查找）
    let _tray = TrayIconBuilder::with_id(TRAY_ICON_ID)
        .menu(&menu)
        .tooltip("LRC Desktop — AI 代码记忆")
        .on_menu_event(move |app, event| {
            handle_menu_event(app, event.id().as_ref());
        })
        .on_tray_icon_event(|tray, event| {
            // 双击托盘图标 → 打开仪表盘
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                open_dashboard(tray.app_handle());
            }
        })
        .build(app)?;

    tracing::info!("系统托盘已创建 (id={})", TRAY_ICON_ID);
    Ok(())
}

/// 更新托盘悬浮提示
/// 
/// 根据已配置的 Agent 数量动态更新 tooltip 文本。
/// 契约：在 configure_agents 完成后调用。
pub fn update_tooltip<R: Runtime>(app: &AppHandle<R>, agent_count: usize) {
    // 获取 sidecar 状态
    let sidecar_running = app
        .try_state::<AppStore>()
        .map(|store| {
            // 使用 try_lock 避免阻塞，托盘更新不阻塞主流程
            store.sidecar.try_lock().is_ok_and(|s| s.is_running())
        })
        .unwrap_or(false);

    let tooltip = format!(
        "LRC Desktop — {} 个 Agent 已连接{}",
        agent_count,
        if sidecar_running { " | 服务运行中" } else { "" }
    );

    if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
        let _ = tray.set_tooltip(Some(&tooltip));
    }
}

/// 处理右键菜单事件
fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, menu_id: &str) {
    match menu_id {
        menu_ids::OPEN_DASHBOARD => {
            open_dashboard(app);
        }
        menu_ids::SETTINGS => {
            // 打开设置页面（在 WebView 中）
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval("window.location.hash = '#settings'");
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        menu_ids::SWITCH_PROJECT => {
            // 打开项目切换界面（直接跳转到步骤2：项目选择）
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval("window.location.hash = '#wizard-switch-project'");
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        menu_ids::ABOUT => {
            // 打开关于页面
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

/// 在 Tauri 内嵌 WebView 中打开仪表盘
/// 
/// 优先使用 sidecar 实际端口，未启动时先尝试启动 sidecar，再回退到默认端口。
/// 如果仪表盘窗口已存在，则聚焦而非创建新窗口。
/// 
/// 契约：仪表盘由 sidecar HTTP 服务提供，通过 WebView 加载。
pub fn open_dashboard<R: Runtime>(app: &AppHandle<R>) {
    // 如果仪表盘窗口已存在，直接聚焦
    if let Some(window) = app.get_webview_window("dashboard") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }

    // 从全局状态获取 sidecar 实际端口
    let port = app
        .try_state::<AppStore>()
        .and_then(|store| {
            store.sidecar_port.try_lock().ok().and_then(|guard| *guard)
        })
        .unwrap_or(3099); // 回退到默认端口

    let url = tauri::Url::parse(&format!("http://127.0.0.1:{port}/dashboard?embedded=tauri")).unwrap_or_else(|_| {
        tauri::Url::parse("http://127.0.0.1:3099/dashboard?embedded=tauri").unwrap()
    });

    // 创建内嵌 WebView 窗口加载仪表盘
    match tauri::WebviewWindowBuilder::new(app, "dashboard", tauri::WebviewUrl::External(url))
        .title("LRC 仪表盘 — AI 代码记忆")
        .inner_size(1200.0, 800.0)
        .min_inner_size(800.0, 600.0)
        .center()
        .build()
    {
        Ok(_) => {
            tracing::info!("仪表盘 WebView 窗口已创建 (port={port})");
        }
        Err(e) => {
            tracing::error!("创建仪表盘窗口失败 (port={port}): {e}");
            // 回退：在外部浏览器中打开
            let fallback_url = format!("http://127.0.0.1:{port}/dashboard?embedded=tauri");
            let _ = open::that(&fallback_url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD：验证菜单 ID 常量定义完整
    #[test]
    fn test_menu_ids_defined() {
        assert!(!menu_ids::OPEN_DASHBOARD.is_empty());
        assert!(!menu_ids::SETTINGS.is_empty());
        assert!(!menu_ids::SWITCH_PROJECT.is_empty());
        assert!(!menu_ids::ABOUT.is_empty());
        assert!(!menu_ids::QUIT.is_empty());
    }

    /// TDD：验证菜单 ID 互不相同
    #[test]
    fn test_menu_ids_unique() {
        let ids = [
            menu_ids::OPEN_DASHBOARD,
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