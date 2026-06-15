// 许可证: Apache 2.0
//
// 桌面仪表盘模块 — 双击 exe 打开浏览器查看记忆管理面板
// ==========================================================
//
// 核心能力:
//   1. open_dashboard — 在默认浏览器中打开仪表盘 URL
//   2. 零配置体验 — 用户双击 exe 即可看到可视化记忆管理界面
//
// 设计原则:
//   - 复用现有 HTTP 服务的 /dashboard 路由（static/index.html）
//   - 通过 webbrowser crate 跨平台打开浏览器
//   - 条件编译：仅 dashboard feature 启用时生效

/// 在默认浏览器中打开仪表盘 URL
///
/// # 参数
/// - `url`: 仪表盘页面的完整 URL（如 http://localhost:3099/dashboard）
///
/// # 返回
/// - `Ok(())`: 浏览器已成功打开
/// - `Err(String)`: 打开失败的原因
#[cfg(feature = "webbrowser")]
pub fn open_dashboard(url: &str) -> Result<(), String> {
    webbrowser::open(url).map_err(|e| format!("无法打开浏览器: {e}"))
}

/// 未启用 webbrowser 时的降级实现 — 打印手动访问提示
#[cfg(not(feature = "webbrowser"))]
pub fn open_dashboard(url: &str) -> Result<(), String> {
    // 降级：无法自动打开浏览器，提示用户手动访问
    eprintln!("[仪表盘] 浏览器自动打开功能未启用");
    eprintln!("[仪表盘] 请手动访问: {url}");
    Ok(())
}
