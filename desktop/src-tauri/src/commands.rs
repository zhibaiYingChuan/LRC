/// IPC 命令处理模块
///
/// 契约优先：所有命令的输入/输出结构体在此定义，
/// 前端通过 `invoke('command_name', { ... })` 调用。
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager; // Manager trait 提供 get_webview_window 等方法
use tauri::State; // v0.5.5 P1-2：Emitter trait 提供 emit 方法（open_settings 命令使用）
                  // v0.5.4 修复：移除未使用的 Emitter import（emit 已从 detect_agents 中移除）
use tokio::sync::Mutex; // 使用 tokio::sync::Mutex 以支持跨 await 持有

use crate::agent_detector::{
    get_scan_cache_timestamp_ms, invalidate_scan_cache, AgentDetectorRegistry, AgentInfo,
    ProjectInfo, RulesStatus,
};
use crate::config_wizard::WizardState;
use crate::rate_limiter::RateLimiter;
use crate::sidecar_manager::{SidecarManager, SidecarStartError, StartOptions, StartProgress};
use crate::tray; // 托盘模块的 open_dashboard 函数

// ── v0.5.1 辅助函数：消除重复代码 ──

/// 从向导配置中获取 LLM API 字符串（消除 3 处重复调用）
///
/// 此函数统一了 start_sidecar、start_sidecar_for_project、switch_project
/// 三处获取 LLM 配置的逻辑，避免未来修改时遗漏同步。
async fn get_llm_api_from_wizard(store: &State<'_, AppStore>) -> Option<String> {
    let wizard = store.wizard.lock().await;
    wizard.config().to_llm_api_string()
}

/// v0.5.7 新增：sidecar 启动后的公共后处理逻辑（消除 M-15 重复代码）
///
/// 统一处理 start_sidecar、start_sidecar_for_project、switch_project 三处
/// sidecar 启动后的自动升级 MCP 配置和写入全局 IDE 规则文件逻辑。
///
/// 注意：此函数不持有 sidecar 锁，避免锁持有时间过长（M-3/M-4 修复的一部分）。
/// project_key 用于日志标识，传入 None 表示默认项目。
async fn post_sidecar_start(store: &State<'_, AppStore>, port: u16, project_key: Option<&str>) {
    // v0.9.0 开发模式隔离：开发模式下严禁修改用户的 MCP 配置和 IDE 规则文件
    // 这些操作会触达稳定版用户的全局配置文件，导致端口被意外切换
    if is_dev_mode() {
        tracing::info!(
            "[开发模式] 跳过 MCP 自动升级和 IDE 规则写入（避免触碰稳定版用户配置）"
        );
        return;
    }

    // v0.5.5：自动检测并升级旧版本 MCP 配置
    // v0.5.6：规则文件改为全局级，不再依赖 project_dir
    let project_dir = {
        let wizard = store.wizard.lock().await;
        wizard.config().project_dir.clone()
    };
    let project_path = project_dir.as_ref().map(std::path::Path::new);

    // v0.8.25 CODE-04 修复：为 auto_upgrade_configs 添加 10s 超时保护
    //   根因：post_sidecar_start 中的 auto_upgrade_configs 无超时保护，
    //   文件系统慢（如杀毒软件扫描、网络驱动器）时可能无限阻塞。
    //   修复：先获取已安装的 agent_ids，再分别在 spawn_blocking 中执行耗时操作，
    //   各加 10s 超时，避免锁持有时间过长。
    let installed_agent_ids = {
        let registry = store.agent_registry.lock().await;
        registry
            .detect_installed()
            .iter()
            .map(|info| info.id.clone())
            .collect::<Vec<String>>()
    };

    // 1. auto_upgrade_configs（10s 超时）
    // v0.8.26 修复：克隆 Arc<Mutex> 避免生命周期逃逸（spawn_blocking 需要 'static）
    let registry_arc = store.agent_registry.clone();
    let project_path_clone = project_path.map(|p| p.to_path_buf());
    let upgrade_result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            let registry = registry_arc.blocking_lock();
            registry.auto_upgrade_configs(port, project_path_clone.as_deref())
        }),
    )
    .await;
    match upgrade_result {
        Ok(Ok(Ok(upgraded))) => {
            if !upgraded.is_empty() {
                match project_key {
                    Some(key) => {
                        tracing::info!("[sidecar] 自动升级完成（项目 {}）: {:?}", key, upgraded)
                    }
                    None => tracing::info!("[sidecar] 自动升级完成: {:?}", upgraded),
                }
            }
        }
        Ok(Ok(Err(e))) => {
            tracing::warn!("[sidecar] 自动升级失败（不影响 sidecar 运行）: {}", e);
        }
        Ok(Err(_)) => {
            tracing::warn!("[sidecar] 自动升级 spawn_blocking panic（不影响 sidecar 运行）");
        }
        Err(_) => {
            tracing::warn!("[sidecar] 自动升级超时（10s，不影响 sidecar 运行）");
        }
    }

    // 2. write_rules_for_agents（10s 超时，v0.8.25 CODE-04 修复）
    if !installed_agent_ids.is_empty() {
        // v0.8.26 修复：克隆 Arc<Mutex> 避免生命周期逃逸
        let registry_arc = store.agent_registry.clone();
        let ids = installed_agent_ids.clone();
        let write_result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::task::spawn_blocking(move || {
                let registry = registry_arc.blocking_lock();
                registry.write_rules_for_agents(&ids)
            }),
        )
        .await;
        match write_result {
            Ok(Ok(Ok(written))) => {
                if !written.is_empty() {
                    match project_key {
                        Some(key) => tracing::info!(
                            "[sidecar] 已为项目 {} 写入 {} 个全局 IDE 规则文件",
                            key,
                            written.len()
                        ),
                        None => tracing::info!(
                            "[sidecar] 已为 {} 个已安装 AI 工具写入全局 IDE 规则文件",
                            written.len()
                        ),
                    }
                }
            }
            Ok(Ok(Err(e))) => {
                tracing::warn!("[sidecar] 全局规则文件写入失败（不影响 sidecar）: {}", e);
            }
            Ok(Err(_)) => {
                tracing::warn!("[sidecar] 全局规则文件写入 spawn_blocking panic（不影响 sidecar）");
            }
            Err(_) => {
                tracing::warn!("[sidecar] 全局规则文件写入超时（10s，不影响 sidecar 运行）");
            }
        }
    }
}

/// 在主窗口 iframe 中显示仪表盘（统一入口，消除 2 处重复的 JS 注入逻辑）
///
/// 原先 navigate_main_to_dashboard 和 open_dashboard_window 有几乎相同的逻辑，
/// 现统一在此函数中。adjust_window 控制是否调整窗口大小和标题。
fn show_dashboard_in_main_window(
    app: &tauri::AppHandle,
    port: u16,
    adjust_window: bool,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        // v0.5.4 修复：使用 Wizard 命名空间替代全局函数
        let js = format!(
            "if(window.Wizard&&window.Wizard.showDashboardEmbed){{window.Wizard.showDashboardEmbed({})}}else{{window.location.hash='#dashboard'}}",
            port
        );
        window
            .eval(&js)
            .map_err(|e| format!("显示仪表盘失败: {e}"))?;

        if adjust_window {
            window
                .set_size(tauri::Size::Physical(tauri::PhysicalSize::new(1200, 800)))
                .map_err(|e| format!("调整窗口大小失败: {e}"))?;
            window
                .set_resizable(true)
                .map_err(|e| format!("设置可缩放失败: {e}"))?;
            window
                .set_title("LRC 仪表盘 — AI 工具的记忆")
                .map_err(|e| format!("设置标题失败: {e}"))?;
        }

        tracing::info!(
            "仪表盘已在主窗口 iframe 中显示 (port={port}, adjust_window={adjust_window})"
        );
    } else {
        // 回退：通过托盘模块（不创建新窗口）
        tray::open_dashboard(app);
    }
    Ok(())
}

/// v0.8.9 G-004：结构化错误 → 用户友好消息（类型安全，无需字符串匹配）
///
/// 与 `user_friendly_error` 的字符串匹配不同，此函数直接匹配枚举变体，
/// 不会因错误信息措辞变化而漏匹配。前端可据此做差异化处理。
fn sidecar_error_to_user_message(e: &SidecarStartError) -> String {
    match e {
        SidecarStartError::UserCancelled => "启动已取消。".to_string(),
        SidecarStartError::PortConflict { port, .. } => {
            format!(
                "端口 {} 已被其他 LRC 服务占用，请先停止现有服务再启动。",
                port
            )
        }
        SidecarStartError::BinaryNotFound { .. } => {
            "LRC 服务程序未找到，请重新安装或联系技术支持。".to_string()
        }
        SidecarStartError::SpawnFailed { reason } => {
            format!("启动 LRC 服务失败：{reason}")
        }
        SidecarStartError::HealthCheckTimeout { port, .. } => {
            format!(
                "LRC 服务健康检查超时（端口 {}），请检查系统资源或重启应用。",
                port
            )
        }
        SidecarStartError::ProcessDied {
            pid,
            log_hint,
            log_empty,
        } => {
            // v0.8.15 P0-2 修复：日志为空时提供可操作建议
            // 根因：DLL 加载失败发生在入口前，日志为空，用户无从下手
            if *log_empty {
                format!(
                    "LRC 服务进程（PID={pid}）启动后立即退出{log_hint}。\n\
                    可能原因：系统缺少 VC++ 运行时库（VCRUNTIME140.dll）。\n\
                    建议：安装 Microsoft Visual C++ Redistributable 后重试，或联系技术支持。"
                )
            } else {
                format!("LRC 服务进程（PID={pid}）启动后意外退出{log_hint}。")
            }
        }
        SidecarStartError::HttpClientError { reason } => {
            format!("内部错误：{reason}")
        }
        SidecarStartError::SingletonConflict { pid, existing_port } => {
            // v0.8.17 P0-2 修复：单例锁冲突时明确提示"已有实例运行"，而非"意外退出"
            // v0.8.19 P0-2 修复（GAP-03/INV-010）：加入结构化标记 [E008:port=XXX] / [E008:noport]
            //   替代 main.rs 中脆弱的中文字符串匹配，Display 措辞变更不会影响判定
            if let Some(port) = existing_port {
                format!(
                    "[E008:port={port}] 已有 LRC 实例在运行（PID={pid}，端口 {port}）。\n\
                    新启动的 sidecar 因单例锁冲突主动退出，这是正常行为。\n\
                    点击「复用现有实例」可直接连接到正在运行的 LRC 服务。"
                )
            } else {
                format!(
                    "[E008:noport] 已有 LRC 实例在运行（PID={pid}）。\n\
                    新启动的 sidecar 因单例锁冲突主动退出，这是正常行为。\n\
                    请在信任中心查看正在运行的实例，或先停止现有实例后再启动。"
                )
            }
        }
    }
}

/// v0.5.4 P1-6 新增：用户友好的错误消息映射
///
/// 将技术错误信息翻译为用户可理解的提示，并附带修复建议。
/// 覆盖常见的错误模式：ENOENT、Connection refused、RwLock 毒化等。
fn user_friendly_error(err: &str) -> String {
    let err_lower = err.to_lowercase();

    // ── v0.8.9 G-001：用户取消启动（最优先匹配，避免被其他规则误捕获） ──
    if err.contains("用户取消启动") || err_lower.contains("cancel") && err_lower.contains("start")
    {
        return "启动已取消。".to_string();
    }

    // ── v0.8.9 G-002：端口被外部 sidecar 占用 ──
    if err.contains("已有 sidecar 运行") {
        return "端口已被其他 LRC 服务占用，请先停止现有服务再启动。".to_string();
    }

    // ── Sidecar 相关错误 ──
    if err_lower.contains("enosys") || err_lower.contains("error 0x80004005") {
        return "LRC 服务程序无法在当前系统运行，请重新下载安装。".to_string();
    }
    // v0.5.7 修复：添加数字错误码匹配（不依赖 OS 语言）
    // 中文 Windows 上 std::io::Error 的错误信息是中文，不包含 "not found"/"enoent"
    if err_lower.contains("os error 2")
        || err_lower.contains("enoent")
        || (err_lower.contains("sidecar") && err_lower.contains("not found"))
        || err_lower.contains("lrc-sidecar") && err_lower.contains("not found")
        || err_lower.contains("系统找不到指定的文件")
    {
        return "LRC 服务程序未找到，请重新安装或联系技术支持。".to_string();
    }
    // os error 5 = 拒绝访问（权限不足）
    if err_lower.contains("os error 5")
        || err_lower.contains("拒绝访问")
        || (err_lower.contains("permission denied") && err_lower.contains("sidecar"))
    {
        return "没有权限启动 LRC 服务，请以管理员身份运行或检查杀毒软件拦截。".to_string();
    }
    // os error 32 = 文件被占用
    if err_lower.contains("os error 32")
        || err_lower.contains("文件已被另一个进程使用")
        || err_lower.contains("being used by another process")
    {
        return "LRC 服务文件被占用，请关闭其他 LRC 实例后重试。".to_string();
    }
    if err_lower.contains("sidecar")
        && (err_lower.contains("not running") || err_lower.contains("未启动"))
    {
        return "LRC 服务未启动，请点击「启动服务」按钮。".to_string();
    }
    if err_lower.contains("connection refused")
        || err_lower.contains("connect refused")
        || err_lower.contains("tcp connect error")
    {
        return "无法连接到 LRC 服务，请检查端口是否被占用，或重启应用。".to_string();
    }
    // v0.5.4 P2-13 修复：健康检查超时匹配（中英文，更具体的错误优先）
    // wait_for_health 返回的中文错误 "健康检查超时" 需要正确匹配
    if (err_lower.contains("health check") && err_lower.contains("timeout"))
        || err_lower.contains("健康检查超时")
    {
        return "LRC 服务启动超时，请检查端口是否被占用，或关闭防火墙后重试。".to_string();
    }
    // v0.5.4 P2-13 修复：添加中文超时关键词匹配
    if err_lower.contains("timeout")
        || err_lower.contains("timed out")
        || err_lower.contains("超时")
    {
        return "LRC 服务启动超时，请检查系统资源是否充足，或重启应用后重试。".to_string();
    }

    // ── 配置相关错误 ──
    if err_lower.contains("rwlock")
        && (err_lower.contains("poison")
            || err_lower.contains("poisoned")
            || err_lower.contains("毒化"))
    {
        return "LRC 遇到内部错误（配置锁异常），请重启应用。".to_string();
    }
    if err_lower.contains("serialize")
        || err_lower.contains("deserialize")
        || err_lower.contains("json") && err_lower.contains("parse")
    {
        return "配置文件格式错误，请尝试重置配置或重启应用。".to_string();
    }
    if err_lower.contains("permission denied")
        || err_lower.contains("access denied")
        || err_lower.contains("eacces")
    {
        return "没有权限写入配置文件，请检查磁盘权限或以管理员身份运行。".to_string();
    }
    if err_lower.contains("disk")
        || err_lower.contains("no space")
        || err_lower.contains("storage") && err_lower.contains("full")
    {
        return "磁盘空间不足，无法保存配置。请清理磁盘后重试。".to_string();
    }
    if err_lower.contains("config")
        && (err_lower.contains("save") || err_lower.contains("write") || err_lower.contains("写入"))
    {
        return "无法保存配置，请检查磁盘空间和权限，或重启应用。".to_string();
    }

    // ── 项目目录相关错误 ──
    if err_lower.contains("project") && err_lower.contains("not found")
        || err_lower.contains("路径不存在")
    {
        return "项目路径不存在，请选择有效的项目目录。".to_string();
    }
    if err_lower.contains("not a directory") || err_lower.contains("不是有效的目录") {
        return "选择的路径不是有效的目录，请选择项目文件夹。".to_string();
    }

    // ── 网络相关错误 ──
    if err_lower.contains("dns") || err_lower.contains("resolve") {
        return "网络无法解析服务器地址，请检查网络连接。".to_string();
    }
    if err_lower.contains("ssl") || err_lower.contains("tls") || err_lower.contains("certificate") {
        return "安全连接失败，请检查系统时间和网络代理设置。".to_string();
    }

    // ── 端口占用 ──
    if err_lower.contains("port")
        && (err_lower.contains("in use")
            || err_lower.contains("occupied")
            || err_lower.contains("被占用")
            || err_lower.contains("already in use"))
    {
        return "端口被占用，请关闭占用端口的程序，或重启应用自动切换端口。".to_string();
    }

    // ── 速率限制 ──
    if err_lower.contains("429") || err_lower.contains("请求过于频繁") {
        return "操作过于频繁，请稍后重试。".to_string();
    }

    // ── 默认：保留原始错误，但添加前缀引导用户 ──
    format!(
        "操作失败：{}。如果问题持续，请重启应用或联系技术支持。",
        err
    )
}

/// 应用全局状态（线程安全，支持异步）
/// v0.5.4 锁获取顺序约束（防止死锁）：
///
/// 多个锁同时获取时，必须按以下层级顺序，不可逆序：
///   Level 1（先获取）: sidecar / agent_registry / rate_limiter
///   Level 2（后获取）: sidecar_port / configured_agent_count / wizard
///   Level 3（最后）: wizard（仅当 Level 2 为 configured_agent_count 时）
///
/// 违反此顺序将导致死锁。所有新增命令必须遵循此约束。
pub struct AppStore {
    pub wizard: Mutex<WizardState>,
    pub sidecar: Mutex<SidecarManager>,
    pub agent_registry: Arc<Mutex<AgentDetectorRegistry>>,
    /// 速率限制器（L3 运行时保护，防止 IPC 命令滥用）
    pub rate_limiter: Mutex<RateLimiter>,
    /// sidecar 当前端口（启动后记录，供托盘等模块使用）
    pub sidecar_port: Mutex<Option<u16>>,
    /// 已配置的 Agent 数量（供托盘 tooltip 使用）
    pub configured_agent_count: Mutex<usize>,
    /// 启动取消标志（v0.8.9 G-001：前端 abort 时通知后端终止启动）
    pub start_cancel_flag: Arc<AtomicBool>,
}

// ── Sidecar 管理命令 ──

/// Sidecar 状态响应（结构化，供前端状态栏使用）
#[derive(Serialize)]
pub struct SidecarStatusResponse {
    /// 是否正在运行
    pub running: bool,
    /// 当前状态描述
    pub state: String,
    /// 端口号（运行中时有效）
    pub port: Option<u16>,
    /// 进程 PID（运行中时有效）
    pub pid: Option<u32>,
}

/// 获取 sidecar 运行状态（返回所有运行中的实例）
/// v0.5.1 增强：每次查询时自动检测并恢复已崩溃的 sidecar 实例
/// v0.5.4 修复：崩溃恢复时传入最新 LLM 配置，避免使用旧值
/// v0.5.16 修复：重写状态检测逻辑，避免在持有 sidecar 锁时扫描端口
///              1. 先在 sidecar 锁内执行崩溃恢复和获取实例列表，然后释放锁
///              2. instances 为空时，检查 sidecar_port 并执行快速健康检查
///              3. 健康检查不持有 sidecar 锁，不会阻塞 start_sidecar 等命令
/// v0.5.17 修复：移除 get_sidecar_status 中的 recover_dead_instances 调用
///              状态查询不应触发崩溃恢复（最多 40 秒阻塞），恢复由心跳协程负责。
///              状态查询仅返回当前实例列表 + sidecar_port 健康检查。
#[tauri::command]
pub async fn get_sidecar_status(
    store: State<'_, AppStore>,
) -> Result<Vec<SidecarStatusResponse>, String> {
    // v0.5.17 修复：状态查询不再触发崩溃恢复
    // 原先调用 recover_dead_instances（最多 40 秒），导致 get_sidecar_status
    // 在持有 sidecar 锁时阻塞所有其他命令（start_sidecar 等）。
    // 崩溃恢复由心跳协程（main.rs）负责，状态查询仅返回当前状态。
    let instances = {
        let sidecar = store.sidecar.lock().await;
        sidecar.list_instances()
    }; // sidecar 锁立即释放（<1ms）

    // 如果桌面端管理的实例不为空，直接返回
    if !instances.is_empty() {
        return Ok(instances
            .iter()
            .map(|inst| SidecarStatusResponse {
                running: true,
                state: format!("Running (project: {})", inst.project_dir),
                port: Some(inst.port),
                pid: Some(inst.pid),
            })
            .collect());
    }

    // v0.5.16 新增：instances 为空时，检查 sidecar_port 是否指向外部 sidecar
    // 不扫描 100 个端口（避免锁竞争），只检查 sidecar_port 指定的单个端口
    let sidecar_port = *store.sidecar_port.lock().await;
    if let Some(port) = sidecar_port {
        // 快速健康检查（不持有 sidecar 锁，不会阻塞其他命令）
        if let Some(probed) = SidecarManager::check_sidecar_health(port).await {
            return Ok(vec![SidecarStatusResponse {
                running: true,
                state: format!(
                    "Running (external, project: {}, uptime: {}s)",
                    if probed.src_dir.is_empty() {
                        "unknown"
                    } else {
                        &probed.src_dir
                    },
                    probed.uptime_seconds
                ),
                port: Some(probed.port),
                pid: None,
            }]);
        } else {
            // 健康检查失败，清除过期的 sidecar_port
            let mut saved_port = store.sidecar_port.lock().await;
            *saved_port = None;
            tracing::info!(
                "get_sidecar_status: sidecar_port {} 健康检查失败，已清除",
                port
            );
        }
    }

    Ok(Vec::new())
}

/// 启动 sidecar 进程
/// v0.5.4 P1-6 修复：错误信息人性化
/// v0.8.9 G-003：通过 Tauri event 向前端推送启动进度
/// v0.8.37 新增：检测是否处于开发模式
///
/// 开发模式下（npm run tauri dev），使用独立的端口和数据目录，
/// 避免与稳定版（已安装的桌面端）冲突。
/// 稳定版端口: 3099, 数据目录: ~/.loong-recall/global/data/
/// 开发版端口: 3100, 数据目录: ~/.loong-recall/dev/data/
fn is_dev_mode() -> bool {
    std::env::var("TAURI_DEV").is_ok() || std::env::var("LRC_DEV_MODE").is_ok()
}

/// 获取开发模式的端口（如果处于开发模式且未指定端口）
fn dev_mode_port(requested_port: Option<u16>) -> u16 {
    if is_dev_mode() && requested_port.is_none() {
        3111
    } else {
        requested_port.unwrap_or(crate::sidecar_manager::DEFAULT_SIDECAR_PORT)
    }
}

/// 获取开发模式的数据目录（如果处于开发模式）
fn dev_mode_data_dir() -> Option<String> {
    if is_dev_mode() {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        Some(format!("{}/.loong-recall/dev/data", home))
    } else {
        None
    }
}

/// v0.8.9 G-004：使用结构化错误 SidecarStartError
#[tauri::command]
pub async fn start_sidecar(
    store: State<'_, AppStore>,
    app: tauri::AppHandle,
    src_dir: Option<String>,
    port: Option<u16>,
    multi_window: Option<u32>,
) -> Result<u16, String> {
    // L3 运行时保护：速率限制检查
    {
        let mut limiter = store.rate_limiter.lock().await;
        if limiter.should_throttle("cmd:start_sidecar") {
            return Err(user_friendly_error("请求过于频繁"));
        }
    }

    // v0.8.9 G-001：重置取消标志，允许新的启动请求
    store.start_cancel_flag.store(false, Ordering::SeqCst);

    // v0.8.9 G-003：创建进度通道，spawn 转发任务将进度事件推送到前端
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<StartProgress>(32);
    let app_for_progress = app.clone();
    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            let _ = app_for_progress.emit("sidecar-start-progress", progress);
        }
    });

    // v0.5.1 重构：统一使用 get_llm_api_from_wizard 辅助函数
    let llm_api = get_llm_api_from_wizard(&store).await;

    // v0.8.0 "归一"修复: 桌面端始终使用全局模式，不再回退到 wizard.project_dir
    //   v0.6.1 P1-1 曾添加 wizard.project_dir 回退以修复空指纹目录问题
    //   但这导致 wizard.project_dir 有值时走项目指纹模式，违反 project_memory.md 约束:
    //     "Desktop client must use global mode by default"
    //   v0.8.0 决策: wizard.project_dir 仅用于 MCP 配置，不决定数据存储位置
    //   显式 src_dir（如 switch_project 调用）仍可走项目指纹模式
    let effective_src_dir = src_dir.clone();

    // v0.5.17 三阶段锁安全模式：避免在持有 sidecar 锁时执行 wait_for_health（最多 40s）
    //   Phase 1: prepare_start（持锁，<1ms）→ 释放锁
    //   Phase 2: spawn_and_wait（不持锁，I/O）
    //   Phase 3: insert_handle（重新获取锁，<1ms）
    let project_key = effective_src_dir
        .clone()
        .unwrap_or_else(|| "default".to_string());

    // Phase 1: 检查是否已运行（持锁，无 I/O）
    let prepare = {
        let mut sidecar = store.sidecar.lock().await;
        sidecar.prepare_start(&project_key)
    }; // sidecar 锁立即释放

    let port = match prepare {
        crate::sidecar_manager::PrepareResult::AlreadyRunning(port) => port,
        crate::sidecar_manager::PrepareResult::NeedStart => {
            // v0.8.9 G-002：Phase 1.5 — 检测端口是否被外部 sidecar 占用
            // 场景：桌面端崩溃后重启，旧 sidecar 仍在端口上运行。
            // 复用现有 sidecar，避免 spawn 重复进程（孤儿进程问题）。
            let target_port = dev_mode_port(port);
            if let Some(probed) = SidecarManager::check_sidecar_health(target_port).await {
                tracing::info!(
                    "G-002：端口 {} 已有健康 sidecar（src_dir: {}, uptime: {}s），复用现有实例",
                    target_port,
                    probed.src_dir,
                    probed.uptime_seconds
                );
                // 复用现有 sidecar，不执行 Phase 2/3
                target_port
            } else if let Some(found) = SidecarManager::find_healthy_sidecar_port(target_port).await
            {
                // v0.8.38 新增：扫描端口区间（3099-3119）寻找健康 sidecar
                // 根因：用户关闭桌面端后重新打开，旧 sidecar 仍在端口 3099 运行，
                // 但桌面端可能因端口自适应/设备切换而误判无 sidecar，
                // 尝试启动新 sidecar 时触发单例锁冲突 E008。
                tracing::info!(
                    "G-002：端口区间扫描发现健康 sidecar 在端口 {}，复用现有实例",
                    found
                );
                found
            } else {
                // Phase 2: 启动子进程 + 健康检查（不持锁，I/O，最多 40s）
                let binary_path = {
                    let sidecar = store.sidecar.lock().await;
                    sidecar.binary_path().to_string()
                }; // 锁立即释放

                let _dev_dd = dev_mode_data_dir();
                let start_opts = StartOptions {
                    src_dir: effective_src_dir.as_deref(),
                    // v0.9.0 修复：必须传 dev_mode_port 计算出的端口（dev=3111，stable=3099），
                    // 而非原始 port（可能为 None）。否则 dev 模式下 spawn 用默认 3099，
                    // 与稳定版冲突并静默复用稳定版 sidecar（违反开发/稳定隔离）。
                    port: Some(target_port),
                    multi_window,
                    llm_api: llm_api.as_deref(),
                    cancel_flag: &store.start_cancel_flag,
                    progress_tx: Some(&progress_tx),
                    data_dir: _dev_dd.as_deref(),
                };
                let (child, port) = match SidecarManager::spawn_and_wait(
                    &binary_path,
                    &project_key,
                    &start_opts,
                )
                .await
                {
                    Ok(pair) => pair,
                    Err(crate::sidecar_manager::SidecarStartError::SingletonConflict {
                        pid,
                        existing_port: None,
                    }) => {
                        // v0.8.39 修复：SingletonConflict 且 health check 不可达时，
                        // 强制终止旧进程后重启，而不是让用户手动操作。
                        tracing::warn!(
                            "E008: 旧 sidecar (PID={}) 不可达，强制终止后重启",
                            pid
                        );
                        if crate::sidecar_manager::kill_process_by_pid(pid) {
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            // v0.8.44 R1 修复（HCSE 韧性验证）：二级重试添加 120s 显式超时
                            //   根因：start_sidecar 的二级重试无显式超时，与 start_sidecar_for_project 不一致
                            //   修复：用 tokio::time::timeout(120s) 包裹
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(120),
                                SidecarManager::spawn_and_wait(
                                    &binary_path,
                                    &project_key,
                                    &start_opts,
                                ),
                            )
                            .await
                            {
                                Ok(Ok(pair)) => pair,
                                Ok(Err(e)) => return Err(sidecar_error_to_user_message(&e)),
                                Err(_) => return Err("启动超时（120s），请检查系统资源后重试。".to_string()),
                            }
                        } else {
                            // v0.8.43 修复（GAP-L1-08 P0）：三级兜底 — kill 失败后清理锁文件再重试一次
                            // 根因：审计报告指出 kill_process_by_pid 失败后无三级操作路径，用户被困
                            // 修复：尝试清理锁文件（最后手段），然后重试一次启动
                            tracing::warn!(
                                "E008 三级兜底：kill_process_by_pid(PID={}) 失败，尝试清理锁文件后重试",
                                pid
                            );
                            // 尝试清理锁文件
                            let lock_dir = {
                                let home = std::env::var("USERPROFILE")
                                    .or_else(|_| std::env::var("HOME"))
                                    .unwrap_or_else(|_| ".".to_string());
                                std::path::PathBuf::from(home)
                                    .join(".loong-recall")
                                    .join("global")
                                    .join("data")
                            };
                            let lock_path = lock_dir.join(".lrc.lock");
                            if lock_path.exists() {
                                let _ = std::fs::write(&lock_path, "");
                                tracing::warn!("E008 三级兜底：已清空锁文件 {}", lock_path.display());
                            }
                            // v0.8.44 R1 修复（HCSE 韧性验证）：三级兜底添加 120s 显式超时
                            //   根因：start_sidecar 的三级兜底无显式超时，与 start_sidecar_for_project 不一致
                            //   修复：用 tokio::time::timeout(120s) 包裹
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(120),
                                SidecarManager::spawn_and_wait(
                                    &binary_path,
                                    &project_key,
                                    &start_opts,
                                ),
                            )
                            .await
                            {
                                Ok(Ok(pair)) => pair,
                                Ok(Err(e)) => {
                                    // 三级兜底也失败，给用户明确的操作指引
                                    let err_msg = format!(
                                        "启动失败: 无法终止旧 sidecar 进程 (PID={})，\
                                         请以管理员身份运行或检查杀毒软件拦截。\
                                         （错误详情: {}）",
                                        pid,
                                        sidecar_error_to_user_message(&e)
                                    );
                                    return Err(err_msg);
                                }
                                Err(_) => {
                                    return Err(format!(
                                        "启动失败: 无法终止旧 sidecar 进程 (PID={})，\
                                         启动超时（120s），请检查系统资源后重试。",
                                        pid
                                    ));
                                }
                            }
                        }
                    }
                    Err(e) => return Err(sidecar_error_to_user_message(&e)),
                };

                // Phase 3: 插入实例（重新获取锁，无 I/O，<1ms）
                {
                    let mut sidecar = store.sidecar.lock().await;
                    sidecar.insert_handle(
                        &project_key,
                        child,
                        port,
                        effective_src_dir,
                        multi_window,
                        llm_api,
                    );
                }

                port
            }
        }
    };

    // 保存端口供其他模块（托盘等）使用
    {
        let mut saved_port = store.sidecar_port.lock().await;
        *saved_port = Some(port);
    }

    // v0.5.7 重构 M-15：使用公共后处理函数（不持有 sidecar 锁）
    post_sidecar_start(&store, port, None).await;

    Ok(port)
}

/// 为指定项目启动 sidecar（不停止其他项目）
///
/// 与 start_sidecar 不同，此命令不会停止已有的 sidecar 实例，
/// 允许同时运行多个项目的 sidecar 服务。
/// v0.5.4 P1-6 修复：错误信息人性化
/// v0.5.17 修复：三阶段锁安全模式，避免在持有锁时执行 wait_for_health（最多 40s）
#[tauri::command]
pub async fn start_sidecar_for_project(
    store: State<'_, AppStore>,
    app: tauri::AppHandle,
    project_key: String,
    src_dir: Option<String>,
    port: Option<u16>,
    multi_window: Option<u32>,
) -> Result<u16, String> {
    // v0.8.9 G-001：重置取消标志，允许新的启动请求
    store.start_cancel_flag.store(false, Ordering::SeqCst);

    // v0.8.9 G-003：创建进度通道
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<StartProgress>(32);
    let app_for_progress = app.clone();
    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            let _ = app_for_progress.emit("sidecar-start-progress", progress);
        }
    });

    // v0.5.1 重构：统一使用 get_llm_api_from_wizard 辅助函数
    let llm_api = get_llm_api_from_wizard(&store).await;

    // v0.5.17 三阶段锁安全模式
    // Phase 1: 检查是否已运行（持锁，无 I/O，<1ms）
    let prepare = {
        let mut sidecar = store.sidecar.lock().await;
        sidecar.prepare_start(&project_key)
    }; // sidecar 锁立即释放

    let port = match prepare {
        crate::sidecar_manager::PrepareResult::AlreadyRunning(port) => port,
        crate::sidecar_manager::PrepareResult::NeedStart => {
            // v0.8.9 G-002：Phase 1.5 — 检测端口是否被外部 sidecar 占用
            let target_port = dev_mode_port(port);
            if let Some(probed) = SidecarManager::check_sidecar_health(target_port).await {
                tracing::info!(
                    "G-002：端口 {} 已有健康 sidecar（src_dir: {}），复用现有实例（项目: {}）",
                    target_port,
                    probed.src_dir,
                    project_key
                );
                target_port
            } else if let Some(found) = SidecarManager::find_healthy_sidecar_port(target_port).await
            {
                // v0.8.38 新增：扫描端口区间（3099-3119）寻找健康 sidecar
                tracing::info!(
                    "G-002：端口区间扫描发现健康 sidecar 在端口 {}，复用现有实例（项目: {}）",
                    found,
                    project_key
                );
                found
            } else {
                // Phase 2: 启动子进程 + 健康检查（不持锁，I/O，最多 40s）
                let binary_path = {
                    let sidecar = store.sidecar.lock().await;
                    sidecar.binary_path().to_string()
                };

                let _dev_dd = dev_mode_data_dir();
                let start_opts = StartOptions {
                    src_dir: src_dir.as_deref(),
                    // v0.9.0 修复：与 start_sidecar 一致，必须传 dev_mode_port 计算的端口
                    // （dev=3111，stable=3099），而非原始 port（可能为 None）。
                    // 否则 dev 模式下 spawn 用默认 3099，与稳定版冲突并复用稳定版 sidecar。
                    port: Some(target_port),
                    multi_window,
                    llm_api: llm_api.as_deref(),
                    cancel_flag: &store.start_cancel_flag,
                    progress_tx: Some(&progress_tx),
                    data_dir: _dev_dd.as_deref(),
                };
                // v0.8.21 FM-05 修复（interaction-resilience-auditor）：
                //   与 switch_project 一致，start_sidecar_for_project 也加 120s 超时保护
                let (child, port) = match tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    SidecarManager::spawn_and_wait(&binary_path, &project_key, &start_opts),
                )
                .await
                {
                    Ok(Ok(result)) => result,
                    Ok(Err(crate::sidecar_manager::SidecarStartError::SingletonConflict {
                        pid,
                        existing_port: None,
                    })) => {
                        // v0.8.39 修复：SingletonConflict 且 health check 不可达时，
                        // 强制终止旧进程后重启
                        tracing::warn!(
                            "E008: 旧 sidecar (PID={}) 不可达，强制终止后重启（项目: {}）",
                            pid,
                            project_key
                        );
                        if !crate::sidecar_manager::kill_process_by_pid(pid) {
                            // v0.8.43 修复（GAP-L1-08 P0）：三级兜底 — kill 失败后清理锁文件再重试一次
                            tracing::warn!(
                                "E008 三级兜底（项目）：kill_process_by_pid(PID={}) 失败，尝试清理锁文件后重试",
                                pid
                            );
                            let lock_dir = {
                                let home = std::env::var("USERPROFILE")
                                    .or_else(|_| std::env::var("HOME"))
                                    .unwrap_or_else(|_| ".".to_string());
                                std::path::PathBuf::from(home)
                                    .join(".loong-recall")
                                    .join("global")
                                    .join("data")
                            };
                            let lock_path = lock_dir.join(".lrc.lock");
                            if lock_path.exists() {
                                let _ = std::fs::write(&lock_path, "");
                                tracing::warn!("E008 三级兜底（项目）：已清空锁文件 {}", lock_path.display());
                            }
                            // 最后尝试一次启动（带 120s 超时）
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(120),
                                SidecarManager::spawn_and_wait(
                                    &binary_path,
                                    &project_key,
                                    &start_opts,
                                ),
                            )
                            .await
                            {
                                Ok(Ok(result)) => {
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                    result
                                }
                                Ok(Err(e)) => {
                                    return Err(format!(
                                        "启动失败: 无法终止旧 sidecar 进程 (PID={})，\
                                         请手动结束该进程后重试。（错误详情: {}）",
                                        pid,
                                        sidecar_error_to_user_message(&e)
                                    ));
                                }
                                Err(_elapsed) => {
                                    store.start_cancel_flag.store(true, Ordering::SeqCst);
                                    return Err(user_friendly_error(
                                        "项目 sidecar 启动超时（120s），请稍后重试或检查 LRC 服务状态",
                                    ));
                                }
                            }
                        } else {
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(120),
                                SidecarManager::spawn_and_wait(
                                    &binary_path,
                                    &project_key,
                                    &start_opts,
                                ),
                            )
                            .await
                            {
                                Ok(Ok(result)) => result,
                                Ok(Err(e)) => {
                                    return Err(sidecar_error_to_user_message(&e))
                                }
                                Err(_elapsed) => {
                                    store.start_cancel_flag.store(true, Ordering::SeqCst);
                                    return Err(user_friendly_error(
                                        "项目 sidecar 启动超时（120s），请稍后重试或检查 LRC 服务状态",
                                    ));
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => return Err(sidecar_error_to_user_message(&e)),
                    Err(_elapsed) => {
                        store.start_cancel_flag.store(true, Ordering::SeqCst);
                        return Err(user_friendly_error(
                            "项目 sidecar 启动超时（120s），请稍后重试或检查 LRC 服务状态",
                        ));
                    }
                };

                // Phase 3: 插入实例（重新获取锁，无 I/O，<1ms）
                {
                    let mut sidecar = store.sidecar.lock().await;
                    sidecar.insert_handle(
                        &project_key,
                        child,
                        port,
                        src_dir,
                        multi_window,
                        llm_api,
                    );
                }

                port
            }
        }
    };

    // 保存最新端口
    {
        let mut saved_port = store.sidecar_port.lock().await;
        *saved_port = Some(port);
    }

    // v0.5.7 重构 M-15：使用公共后处理函数（不持有 sidecar 锁）
    post_sidecar_start(&store, port, Some(&project_key)).await;

    Ok(port)
}

/// 取消正在进行的 sidecar 启动（v0.8.9 G-001）
///
/// 前端 abort 启动请求时调用此命令，设置取消标志，
/// 后端 `spawn_and_wait` 中的健康检查循环会检测此标志并立即终止。
///
/// **注意**：此命令仅设置标志，不会中断已 spawn 的子进程。
/// 子进程清理由 `spawn_and_wait` 的错误处理路径完成（kill + wait）。
#[tauri::command]
pub async fn cancel_start_sidecar(store: State<'_, AppStore>) -> Result<(), String> {
    store.start_cancel_flag.store(true, Ordering::SeqCst);
    tracing::info!("收到取消 sidecar 启动请求，标志已设置");
    Ok(())
}

/// 停止指定项目的 sidecar 进程
/// v0.5.4 P1-6 修复：错误信息人性化
#[tauri::command]
pub async fn stop_sidecar_for_project(
    store: State<'_, AppStore>,
    project_key: String,
) -> Result<(), String> {
    let mut sidecar = store.sidecar.lock().await;
    sidecar
        .stop_project(&project_key)
        .await
        .map_err(|e| user_friendly_error(&e))
}

/// 停止 sidecar 进程
/// v0.5.4 P1-6 修复：错误信息人性化
/// v0.5.7 二次审计修复：缩小 sidecar 锁持有范围，避免 L1→L2 锁嵌套
#[tauri::command]
pub async fn stop_sidecar(store: State<'_, AppStore>) -> Result<(), String> {
    // v0.5.7：先持有 sidecar 锁执行 stop()，释放后再获取 sidecar_port 锁
    {
        let mut sidecar = store.sidecar.lock().await;
        sidecar.stop().await.map_err(|e| user_friendly_error(&e))?;
    } // sidecar 锁在此释放

    // 清除端口记录（单独获取 sidecar_port 锁，避免锁嵌套）
    {
        let mut saved_port = store.sidecar_port.lock().await;
        *saved_port = None;
    }
    Ok(())
}

// ── LLM 配置命令 ──

/// LLM API 配置响应
#[derive(Serialize)]
pub struct LlmConfigResponse {
    pub configured: bool,
    pub llm_type: String,
    pub model: Option<String>,
}

/// 获取 LLM 配置状态
#[tauri::command]
pub async fn get_llm_config(store: State<'_, AppStore>) -> Result<LlmConfigResponse, String> {
    let wizard = store.wizard.lock().await;
    let config = wizard.config();
    Ok(LlmConfigResponse {
        configured: config.llm_configured,
        llm_type: config.llm_type.clone(),
        model: config.llm_model.clone(),
    })
}

/// 保存 LLM 配置
/// v0.5.4 P1-6 修复：错误信息人性化
#[tauri::command]
pub async fn save_llm_config(
    store: State<'_, AppStore>,
    llm_api: String,
) -> Result<LlmConfigResponse, String> {
    // L3 运行时保护：速率限制检查
    {
        let mut limiter = store.rate_limiter.lock().await;
        if limiter.should_throttle("cmd:save_llm_config") {
            return Err(user_friendly_error("请求过于频繁"));
        }
    }

    let mut wizard = store.wizard.lock().await;
    wizard
        .save_llm_config(&llm_api)
        .map_err(|e| user_friendly_error(&e))?;
    let config = wizard.config();
    let response = LlmConfigResponse {
        configured: config.llm_configured,
        llm_type: config.llm_type.clone(),
        model: config.llm_model.clone(),
    };
    // v0.5.7 二次审计修复：提前克隆 llm_api_str，释放 wizard 锁后再获取 sidecar_port 锁
    let llm_api_str = config.to_llm_api_string();
    drop(wizard); // 释放 wizard 锁，避免与 sidecar_port 锁嵌套

    // v0.5.4 修复：保存 LLM 配置后，同步到 Sidecar 的内存状态
    // 否则仪表盘（通过 GET /api/config）仍显示"未配置"
    let sidecar_port = store.sidecar_port.lock().await;
    if let Some(port) = *sidecar_port {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        // 忽略同步失败（Sidecar 可能未启动），不影响主流程
        let _ = client
            .post(format!("http://127.0.0.1:{port}/api/config/llm"))
            .json(&serde_json::json!({ "llm_api": llm_api_str }))
            .send()
            .await;
    }

    Ok(response)
}

/// 清除 LLM 配置
#[tauri::command]
pub async fn clear_llm_config(store: State<'_, AppStore>) -> Result<LlmConfigResponse, String> {
    // v0.5.7 二次审计修复：缩小 wizard 锁持有范围，避免与 sidecar_port 锁嵌套
    {
        let mut wizard = store.wizard.lock().await;
        // 清除 LLM 配置（保留其他配置不变）
        wizard
            .save_llm_config("")
            .map_err(|e| user_friendly_error(&e))?;
    } // wizard 锁在此释放

    let response = LlmConfigResponse {
        configured: false,
        llm_type: "none".to_string(),
        model: None,
    };

    // v0.5.4 修复：同步清除到 Sidecar 内存状态
    let sidecar_port = store.sidecar_port.lock().await;
    if let Some(port) = *sidecar_port {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        let _ = client
            .post(format!("http://127.0.0.1:{port}/api/config/llm"))
            .json(&serde_json::json!({ "llm_api": "" }))
            .send()
            .await;
    }

    Ok(response)
}

/// LLM 连接测试结果
#[derive(Serialize)]
pub struct LlmTestResult {
    pub success: bool,
    pub message: String,
    /// 检测到的模型列表（仅成功时返回）
    pub models: Option<Vec<String>>,
}

/// v0.5.4 新增：API Key 输入清洗
///
/// 用户从网页复制 API Key 时可能带入首尾空格、换行符等不可见字符，
/// 这些字符会导致 API 请求失败（401/403），且用户难以排查。
/// 清洗规则：trim 首尾空白 + 移除 \r\n\t 等控制字符。
fn clean_api_key(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_control() || *c == ' ') // 保留空格，过滤其他控制字符
        .collect()
}

/// 测试 LLM API 连接（由 Rust 后端代理，避免浏览器 CSP 限制）
///
/// 前端直接向 LLM 提供商发请求会被 CSP 拦截，
/// 此命令通过 reqwest 在 Rust 侧完成网络请求，不受 CSP 限制。
/// v0.5.4 修复：API Key 输入清洗，trim + 过滤不可见字符
#[tauri::command]
pub async fn test_llm_connection(
    provider: String,
    api_key: String,
    base_url: String,
    model: Option<String>,
) -> Result<LlmTestResult, String> {
    // v0.5.4 修复：清洗 API Key — trim 空白 + 过滤 \r\n 等控制字符
    let api_key = clean_api_key(&api_key);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let base = base_url.trim_end_matches('/');

    if provider == "ollama" {
        // Ollama 本地服务测试
        let resp = client.get(format!("{base}/api/tags")).send().await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let data: serde_json::Value =
                    r.json().await.map_err(|e| format!("解析响应失败: {e}"))?;
                let models: Vec<String> = data["models"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(LlmTestResult {
                    success: true,
                    message: format!("Ollama 连接成功！已安装 {} 个模型", models.len()),
                    models: Some(models),
                })
            }
            Err(e) => {
                // v0.5.4 修复：详细错误分类
                let msg = if e.is_timeout() {
                    "Ollama 连接超时，请确认服务是否已启动".to_string()
                } else if e.is_connect() {
                    format!("无法连接 Ollama 服务（{}），请确认 Ollama 是否在运行", base)
                } else {
                    format!("Ollama 连接失败：{e}")
                };
                Ok(LlmTestResult {
                    success: false,
                    message: msg,
                    models: None,
                })
            }
            Ok(r) => Ok(LlmTestResult {
                success: false,
                message: format!("Ollama 返回错误 (HTTP {})", r.status()),
                models: None,
            }),
        }
    } else {
        // 云端 API 测试：先尝试 /models 端点
        let models_url = format!("{base}/models");
        let resp = client
            .get(&models_url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let data: serde_json::Value =
                    r.json().await.map_err(|e| format!("解析响应失败: {e}"))?;
                let models: Vec<String> = data["data"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m["id"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(LlmTestResult {
                    success: true,
                    message: if models.is_empty() {
                        "连接成功！API Key 验证通过".to_string()
                    } else {
                        format!("连接成功！检测到 {} 个可用模型", models.len())
                    },
                    models: Some(models),
                })
            }
            Ok(r) if r.status() == 401 || r.status() == 403 => Ok(LlmTestResult {
                success: false,
                message: "API Key 无效或无权访问，请检查 Key 是否正确".to_string(),
                models: None,
            }),
            Ok(r) if r.status() == 402 => {
                // v0.5.4 新增：余额不足
                Ok(LlmTestResult {
                    success: false,
                    message: "账户余额不足（402 Payment Required），请充值后重试".to_string(),
                    models: None,
                })
            }
            Ok(r) if r.status() == 429 => {
                // v0.5.4 新增：频率限制
                Ok(LlmTestResult {
                    success: false,
                    message: "请求过于频繁（429），请稍后重试".to_string(),
                    models: None,
                })
            }
            Err(e) => {
                // v0.5.4 修复：网络错误详细分类
                let msg = if e.is_timeout() {
                    format!("连接超时：无法在 10 秒内连接到 {base}，请检查网络或 API 地址")
                } else if e.is_connect() {
                    format!("网络不通：无法连接到 {base}，请检查网络连接和 API 地址是否正确")
                } else {
                    format!("网络请求失败：{e}")
                };
                Ok(LlmTestResult {
                    success: false,
                    message: msg,
                    models: None,
                })
            }
            _ => {
                // /models 端点不可用，尝试 chat/completions
                let test_model = model.unwrap_or_else(|| "gpt-4o-mini".to_string());
                let chat_url = format!("{base}/chat/completions");
                let chat_resp = client
                    .post(&chat_url)
                    .header("Authorization", format!("Bearer {api_key}"))
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "model": test_model,
                        "messages": [{ "role": "user", "content": "hi" }],
                        "max_tokens": 5,
                    }))
                    .send()
                    .await;

                match chat_resp {
                    Ok(r) if r.status().is_success() => Ok(LlmTestResult {
                        success: true,
                        message: "连接成功！API Key 和模型均验证通过".to_string(),
                        models: None,
                    }),
                    Ok(r) if r.status() == 401 || r.status() == 403 => Ok(LlmTestResult {
                        success: false,
                        message: "API Key 无效，请检查 Key 是否正确".to_string(),
                        models: None,
                    }),
                    Ok(r) if r.status() == 402 => {
                        // v0.5.4 新增：余额不足
                        Ok(LlmTestResult {
                            success: false,
                            message: "账户余额不足（402），请充值后重试".to_string(),
                            models: None,
                        })
                    }
                    Ok(r) if r.status() == 429 => {
                        // v0.5.4 新增：频率限制
                        Ok(LlmTestResult {
                            success: false,
                            message: "请求过于频繁（429），请稍后重试".to_string(),
                            models: None,
                        })
                    }
                    Ok(r) if r.status() == 404 => {
                        // v0.5.4 新增：模型不可用
                        Ok(LlmTestResult {
                            success: false,
                            message: format!(
                                "模型 \"{test_model}\" 不可用（404），请确认模型名称是否正确"
                            ),
                            models: None,
                        })
                    }
                    Ok(r) => Ok(LlmTestResult {
                        success: false,
                        message: format!(
                            "连接成功但模型可能不可用 (HTTP {})，请手动设置模型名",
                            r.status()
                        ),
                        models: None,
                    }),
                    Err(e) => {
                        // v0.5.4 修复：网络错误详细分类
                        let msg = if e.is_timeout() {
                            "请求超时：API 响应时间过长，请检查网络或更换 API 地址".to_string()
                        } else if e.is_connect() {
                            "网络不通：无法建立连接，请检查网络和 API 地址".to_string()
                        } else {
                            format!("网络请求失败：{e}")
                        };
                        Ok(LlmTestResult {
                            success: false,
                            message: msg,
                            models: None,
                        })
                    }
                }
            }
        }
    }
}

// ── Agent 检测命令 ──

/// 检测所有已安装的 Agent
#[tauri::command]
pub async fn detect_agents(store: State<'_, AppStore>) -> Result<Vec<AgentInfo>, String> {
    // v0.5.7 修复：添加后端超时，避免前端超时后后端仍持锁导致死循环
    // detect_all() 内部主要是 Path::exists（轻量），正常 < 1 秒
    // 30 秒超时作为兜底，防止异常情况（如网络盘、杀毒软件扫描）导致卡死
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let registry = store.agent_registry.lock().await;
        registry.detect_all()
    })
    .await;

    match result {
        Ok(agents) => {
            tracing::info!("detect_agents 完成，检测到 {} 个 Agent", agents.len());
            Ok(agents)
        }
        Err(_) => {
            tracing::error!("detect_agents 超时（30秒），可能存在锁竞争或文件系统慢");
            Err(
                "AI 工具检测超时（30秒），可能是杀毒软件扫描或网络盘响应慢，请重启应用后重试"
                    .to_string(),
            )
        }
    }
}

/// 仅返回已安装的 Agent（过滤掉未安装的）
#[tauri::command]
pub async fn detect_installed_agents(store: State<'_, AppStore>) -> Result<Vec<AgentInfo>, String> {
    let registry = store.agent_registry.lock().await;
    Ok(registry.detect_installed())
}

/// v0.6.0 新增：获取工具的手动配置指引
///
/// 对于不支持 MCP 自动配置的工具，返回手动配置文档。
/// 前端可据此展示"如何手动配置"面板，包含配置路径、模板和官方文档链接。
#[tauri::command]
pub async fn get_agent_config_guide(agent_id: String) -> Result<Option<String>, String> {
    // 调用 agent_detector 模块的公开函数获取配置指引
    let guide = crate::agent_detector::get_manual_config_guide(&agent_id);
    Ok(guide.map(|s| s.to_string()))
}

/// 全面发现：已知工具 + 未知 dot 目录中的潜在 AI 工具
///
/// 返回 (已知工具列表, 未知工具列表)
///
/// v0.8.31 S-03：应用 manual_agent_overrides 覆盖（用户点击齿轮手动修正的优先级最高）
/// v0.8.33 HCSE FM-05：外层 30 秒超时兜底，防止磁盘卡死/网络盘慢导致前端卡死
#[tauri::command]
pub async fn discover_all_agents(
    store: State<'_, AppStore>,
) -> Result<(Vec<AgentInfo>, Vec<AgentInfo>), String> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        // 锁顺序约束（Level 1 → Level 2）：先取 agent_registry（L1），释放后再取 wizard（L2）
        let (mut known_list, unknown_list) = {
            let registry = store.agent_registry.lock().await;
            registry.discover_all()
        }; // agent_registry 锁已释放（<1ms）

        // Level 2：取 wizard 锁读取手动修正
        let overrides = {
            let wizard = store.wizard.lock().await;
            wizard.get_manual_agent_overrides()
        }; // wizard 锁已释放（<1ms）

        // 对已知工具列表应用手动覆盖（用户纠正优先级最高，无论自动检测结果如何）
        if !overrides.is_empty() {
            let mut applied = 0usize;
            for agent in known_list.iter_mut() {
                if let Some(&force_installed) = overrides.get(&agent.id) {
                    if agent.installed != force_installed {
                        tracing::info!(
                            "[Agent检测] 手动修正生效: id={} 原={} → 强制={}",
                            agent.id,
                            agent.installed,
                            force_installed
                        );
                        agent.installed = force_installed;
                        applied += 1;
                    }
                }
            }
            if applied > 0 {
                tracing::debug!("[Agent检测] 共应用 {} 条用户手动修正", applied);
            }
        }

        (known_list, unknown_list)
    })
    .await;

    match result {
        Ok(tuple) => Ok(tuple),
        Err(_) => {
            tracing::error!("discover_all_agents 触发 HCSE FM-05 30 秒超时兜底");
            Err("AI 工具检测超时（30 秒）。可能是磁盘响应慢或网络盘扫描卡住，请点击「重新扫描」重试，或关闭杀毒软件后再试。".to_string())
        }
    }
}

// ════════════════════════════════════════════════════════════════
// v0.8.31 S-05：扫描缓存元数据 + 强制失效命令
// ════════════════════════════════════════════════════════════════
// 前端「重新扫描」按钮：
//   1. 调用 invalidate_scan_cache 清空缓存
//   2. 调用 discover_all_agents 重新检测（会触发重新扫描）
//   3. 调用 get_scan_cache_metadata 获取新的扫描时间戳并展示
// ════════════════════════════════════════════════════════════════

/// v0.8.31 S-05：扫描缓存元数据（返回给前端显示「上次扫描时间」）
#[derive(Debug, Clone, Serialize)]
pub struct ScanCacheMetadata {
    /// 缓存创建时间（ms，UNIX EPOCH），0 表示还没扫描过
    pub timestamp_ms: u64,
    /// 缓存 TTL（ms），默认 24h
    pub ttl_ms: u64,
    /// 当前缓存是否仍然有效（未过期）
    pub valid: bool,
}

/// v0.8.31 S-05：获取扫描缓存的元数据（时间戳、TTL、有效状态）
#[tauri::command]
pub async fn get_scan_cache_metadata() -> ScanCacheMetadata {
    const TTL_MS: u64 = 24 * 60 * 60 * 1000;
    let ts_ms = get_scan_cache_timestamp_ms();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_millis() as u64;
    let age = now.saturating_sub(ts_ms);
    ScanCacheMetadata {
        timestamp_ms: ts_ms,
        ttl_ms: TTL_MS,
        valid: ts_ms > 0 && age <= TTL_MS,
    }
}

/// v0.8.31 S-05：强制使扫描缓存失效
/// 前端「重新扫描」按钮调用此命令后，应紧接着调用 discover_all_agents 触发新扫描。
///
/// v0.8.31 HCSE RISK-P1-02 修复：加入 RateLimiter 防护，避免攻击者高频调用
///   导致全局 RwLock 写锁竞争（DoS 攻击向量），与 configure_agents 使用同一速率策略。
/// v0.8.33 HCSE FM-19：触发 429 时 1/2/4/8s 指数退避重试 4 次；耗尽后降级直接执行（
///   因为前端「重新扫描」是用户高意图操作，DoS 风险远低于后端配置持久化写入场景）。
#[tauri::command]
pub async fn force_invalidate_scan_cache(store: State<'_, AppStore>) -> Result<(), String> {
    const KEY: &str = "cmd:force_invalidate_scan_cache";
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        let pass = {
            let mut limiter = store.rate_limiter.lock().await;
            !limiter.should_throttle(KEY)
        };
        if pass {
            break;
        }
        if attempts >= 4 {
            tracing::warn!(
                "[扫描缓存失效] 限流4次重试全部耗尽，HCSE FM-19降级放行（避免用户点击卡死）"
            );
            break; // 降级：直接放行（RwLock 写锁竞争风险 vs 用户体验，选择后者）
        }
        let wait_ms = 1000u64 * (1u64 << (attempts - 1));
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
    }
    invalidate_scan_cache();
    Ok(())
}

/// 为选定的 Agent 配置 MCP 连接
///
/// 配置完成后自动更新托盘 tooltip 显示 Agent 数量，
/// 并持久化 configured_agents 到 wizard.json（P2-05 修复）。
#[tauri::command]
pub async fn configure_agents(
    app: tauri::AppHandle,
    store: State<'_, AppStore>,
    agent_ids: Vec<String>,
    port: u16,
) -> Result<Vec<String>, String> {
    // v0.9.0 开发模式隔离：开发模式下禁止修改用户的全局 MCP 配置和 IDE 规则文件
    if is_dev_mode() {
        tracing::warn!(
            "[开发模式] configure_agents 被调用但被拒绝（避免触碰稳定版用户配置）"
        );
        return Err("开发模式下不允许配置 Agent（避免修改稳定版用户的全局 IDE 配置）。请使用稳定版桌面端进行配置。".to_string());
    }

    // L3 运行时保护：速率限制检查
    {
        let mut limiter = store.rate_limiter.lock().await;
        if limiter.should_throttle("cmd:configure_agents") {
            return Err(user_friendly_error("请求过于频繁"));
        }
    }

    // v0.5.4 修复：获取项目目录，用于写入 AI 规则文件
    let project_dir = {
        let wizard = store.wizard.lock().await;
        wizard.config().project_dir.clone()
    };

    // v0.5.7 修复：添加后端超时，避免文件写入慢导致卡死
    // configure() 涉及多个文件读写（MCP 配置 + AI 规则），给 60 秒
    let project_path = project_dir.as_ref().map(std::path::Path::new);
    let config_result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let registry = store.agent_registry.lock().await;
        registry.configure(&agent_ids, port, project_path)
    })
    .await;

    let result = match config_result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(user_friendly_error(&e)),
        Err(_) => {
            tracing::error!("configure_agents 超时（60秒）");
            return Err(
                "Agent 配置超时（60秒），可能是磁盘写入慢或杀毒软件拦截，请暂时关闭杀毒软件后重试"
                    .to_string(),
            );
        }
    };

    // 更新 Agent 计数
    let mut count = store.configured_agent_count.lock().await;
    *count = result.len();
    // 更新托盘 tooltip
    tray::update_tooltip(&app, *count);
    // 持久化 configured_agents 到 wizard.json（P2-05 修复）
    let mut wizard = store.wizard.lock().await;
    wizard
        .save_configured_agents(agent_ids)
        .map_err(|e| user_friendly_error(&e))?;
    Ok(result)
}

/// v0.5.12 新增：保存已配置的 Agent 列表（用于清理过期 configured_agents）
///
/// 前端在 showReadyPanel 中过滤 configured_agents 后，调用此命令
/// 将清理后的列表持久化到 wizard.json
#[tauri::command]
pub async fn save_configured_agents(
    store: State<'_, AppStore>,
    agent_ids: Vec<String>,
) -> Result<(), String> {
    let mut wizard = store.wizard.lock().await;
    wizard
        .save_configured_agents(agent_ids)
        .map_err(|e| user_friendly_error(&e))?;
    tracing::info!("[配置] 已保存 configured_agents");
    Ok(())
}

/// 扫描已安装 IDE 的项目列表
///
/// 传入已选中的 IDE agent_ids（如 ["trae", "cursor"]），
/// 返回每个 IDE 对应的项目列表
#[tauri::command]
pub async fn scan_ide_projects(
    store: State<'_, AppStore>,
    ide_ids: Vec<String>,
) -> Result<Vec<ProjectInfo>, String> {
    // v0.5.7 修复：添加后端超时，避免文件系统慢导致卡死
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let registry = store.agent_registry.lock().await;
        registry.scan_ide_projects(&ide_ids)
    })
    .await;

    match result {
        Ok(projects) => {
            tracing::info!("scan_ide_projects 完成，扫描到 {} 个项目", projects.len());
            Ok(projects)
        }
        Err(_) => {
            tracing::error!("scan_ide_projects 超时（30秒）");
            Err("项目扫描超时（30秒），可能是磁盘响应慢，请减少选择的 IDE 数量后重试".to_string())
        }
    }
}

/// v0.8.31 S-03：设置 AI 工具的手动修正（用户在向导步骤1点击齿轮图标时触发）
///
/// # 参数
/// - agent_id: 工具 ID（如 "trae"、"trae-cn"、"codebuddy"）
/// - override_installed:
///   - Some(true)  = 用户强制标记为「已安装」（纠正漏检）
///   - Some(false) = 用户强制标记为「未安装」（纠正误检）
///   - None        = 用户选择「恢复自动检测」（清除该工具的手动修正）
///
/// # 优先级
/// manual_override 在 discover_all_agents、detect_agents、detect_installed_agents
/// 三个查询命令中均以最高优先级应用，永不被自动检测结果覆盖。
/// 用户通过「重新检测」按钮刷新缓存时，手动修正结果保持不变。
///
/// v0.8.33 HCSE FM-09：单个修正 10s 超时兜底；FM-19 触发 429 时 1/2/4/8s 指数退避重试 4 次；
///   限流耗尽后降级放行（手动修正是用户主动高意图操作，DoS 风险远低于配置写入）。
#[tauri::command]
pub async fn set_agent_manual_override(
    store: State<'_, AppStore>,
    agent_id: String,
    override_installed: Option<bool>,
) -> Result<(), String> {
    // 参数校验先做（不占限流桶；空输入直接拒）
    let trimmed = agent_id.trim().to_string();
    if trimmed.is_empty() {
        return Err(user_friendly_error("工具 ID 不能为空"));
    }

    // L3 运行时保护：FM-19 指数退避限流（1s / 2s / 4s / 8s 共 4 次）
    const THROTTLE_KEY: &str = "cmd:set_agent_manual_override";
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        let pass = {
            let mut limiter = store.rate_limiter.lock().await;
            !limiter.should_throttle(THROTTLE_KEY)
        };
        if pass {
            break;
        }
        if attempts >= 4 {
            tracing::warn!(
                "[Agent手动修正] 限流4次重试全部耗尽，HCSE FM-19降级放行（id={}）",
                trimmed
            );
            break; // 降级：放行（该操作是用户主动点击齿轮产生，DoS概率低）
        }
        let wait_ms = 1000u64 * (1u64 << (attempts - 1));
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
    }

    let action = match override_installed {
        Some(true) => "强制标记为已安装",
        Some(false) => "强制标记为未安装",
        None => "恢复自动检测",
    };

    // HCSE FM-09：10 秒超时（Wizard::set_agent_manual_override → 内部持久化 wizard.json）
    let store_clone = store.clone();
    let trimmed_clone = trimmed.clone();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async move {
        let mut wizard = store_clone.wizard.lock().await;
        wizard
            .set_agent_manual_override(&trimmed_clone, override_installed)
            .map_err(|e| user_friendly_error(&e))
    })
    .await;

    match result {
        Ok(Ok(())) => {
            tracing::info!("[Agent手动修正] 工具={} 操作={}", trimmed, action);
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            tracing::error!(
                "[Agent手动修正] HCSE FM-09 10秒超时：工具={} 操作={}",
                trimmed,
                action
            );
            Err(format!(
                "手动修正超时（10秒）：工具={}。可能是配置文件写入被杀毒软件拦截，请关闭杀毒软件后重试，或点击「重新扫描」后再修正。",
                trimmed
            ))
        }
    }
}

/// v0.8.33 HCSE FM-09：批量应用 AI 工具手动修正（恢复/跨端同步场景）
///
/// 整批 10 秒超时；成功条目不回滚；失败时用 BulkApplyResult.errors 返回每条错误。
#[tauri::command]
pub async fn bulk_apply_agent_overrides(
    store: State<'_, AppStore>,
    overrides: std::collections::HashMap<String, Option<bool>>,
) -> Result<BulkApplyResult, String> {
    use std::collections::HashMap;

    let total = overrides.len();
    if total == 0 {
        return Ok(BulkApplyResult {
            total: 0,
            succeeded: 0,
            failed: 0,
            errors: HashMap::new(),
        });
    }

    // FM-19 指数退避（批量桶单独隔离，避免影响 user-interactive 单个修改）
    const THROTTLE_KEY: &str = "cmd:bulk_apply_agent_overrides";
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        let pass = {
            let mut limiter = store.rate_limiter.lock().await;
            !limiter.should_throttle(THROTTLE_KEY)
        };
        if pass {
            break;
        }
        if attempts >= 4 {
            tracing::warn!("[批量修正] 限流耗尽4次，FM-19降级放行（{}条）", total);
            break;
        }
        let wait_ms = 1000u64 * (1u64 << (attempts - 1));
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
    }

    let store_clone = store.clone();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async move {
        let mut wizard = store_clone.wizard.lock().await;
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut errors: HashMap<String, String> = HashMap::new();
        for (agent_id, val) in overrides {
            let trimmed = agent_id.trim().to_string();
            if trimmed.is_empty() {
                failed += 1;
                errors.insert(agent_id, "工具 ID 不能为空".to_string());
                continue;
            }
            match wizard.set_agent_manual_override(&trimmed, val) {
                Ok(()) => succeeded += 1,
                Err(e) => {
                    failed += 1;
                    errors.insert(trimmed, e);
                }
            }
        }
        BulkApplyResult {
            total,
            succeeded,
            failed,
            errors,
        }
    })
    .await;

    match result {
        Ok(r) => {
            tracing::info!(
                "[批量修正] HCSE FM-09 完成：total={} ok={} fail={}",
                r.total,
                r.succeeded,
                r.failed
            );
            Ok(r)
        }
        Err(_) => {
            tracing::error!("[批量修正] HCSE FM-09 10秒超时");
            Err("批量手动修正超时（10秒）：请减少一次性修改的工具数量后重试。".to_string())
        }
    }
}

/// v0.8.33 HCSE FM-09：bulk_apply_agent_overrides 的返回结构
#[derive(Debug, Clone, Serialize)]
pub struct BulkApplyResult {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    /// key = 出错的 agent_id，value = 错误消息
    pub errors: std::collections::HashMap<String, String>,
}

// ── 项目目录命令 ──

/// 获取当前项目目录
#[tauri::command]
pub async fn get_project_dir(store: State<'_, AppStore>) -> Result<Option<String>, String> {
    let wizard = store.wizard.lock().await;
    Ok(wizard.config().project_dir.clone())
}

/// 设置项目目录
/// v0.5.4 修复：设置前验证路径存在
/// v0.5.4 P1-6 修复：错误信息人性化
#[tauri::command]
pub async fn set_project_dir(
    store: State<'_, AppStore>,
    project_dir: String,
) -> Result<(), String> {
    // v0.5.4 修复：设置项目目录前验证路径存在
    let path = std::path::Path::new(&project_dir);
    if !path.exists() {
        return Err(user_friendly_error("项目路径不存在"));
    }
    if !path.is_dir() {
        return Err(user_friendly_error("路径不是有效的目录"));
    }
    let mut wizard = store.wizard.lock().await;
    wizard
        .set_project_dir(&project_dir)
        .map_err(|e| user_friendly_error(&e))
}

/// 打开文件夹选择对话框，让用户选择项目目录
#[tauri::command]
pub async fn pick_project_dir() -> Result<Option<String>, String> {
    let path = rfd::AsyncFileDialog::new()
        .pick_folder()
        .await
        .map(|handle| handle.path().display().to_string());
    Ok(path)
}

/// 向导配置状态响应
#[derive(Serialize)]
pub struct WizardStateResponse {
    /// 是否已完成首次配置
    pub setup_complete: bool,
    /// 项目目录路径
    pub project_dir: Option<String>,
    /// LLM 是否已配置
    pub llm_configured: bool,
    /// LLM 提供商类型（openai/ollama/none）
    pub llm_type: String,
    /// LLM 模型名
    pub llm_model: Option<String>,
    /// 已配置的 Agent 列表
    pub configured_agents: Vec<String>,
    /// Sidecar 是否在运行
    pub sidecar_running: bool,
    /// Sidecar 当前端口
    pub sidecar_port: Option<u16>,
    /// v0.5.4 新增：配置文件是否从损坏状态恢复
    /// 前端可据此显示"配置已重置，请重新配置"的提示
    pub config_corrupted: bool,
}

/// 列出所有运行中的 sidecar 项目（供托盘面板使用）
#[tauri::command]
pub async fn list_sidecar_projects(
    store: State<'_, AppStore>,
) -> Result<Vec<crate::sidecar_manager::SidecarInstance>, String> {
    let sidecar = store.sidecar.lock().await;
    Ok(sidecar.list_instances())
}

/// 获取向导配置状态
///
/// 前端用于判断是显示配置向导还是"已就绪"面板
#[tauri::command]
pub async fn get_wizard_state(store: State<'_, AppStore>) -> Result<WizardStateResponse, String> {
    // v0.5.6 修复 H-2：锁顺序改为先 sidecar（L1）后 sidecar_port（L2），避免 AB-BA 死锁
    // 先获取 wizard 数据并释放锁
    let (
        setup_complete,
        project_dir,
        llm_configured,
        llm_type,
        llm_model,
        configured_agents,
        corrupted,
    ) = {
        let wizard = store.wizard.lock().await;
        let config = wizard.config();
        (
            config.setup_complete,
            config.project_dir.clone(),
            config.llm_configured,
            config.llm_type.clone(),
            config.llm_model.clone(),
            config.configured_agents.clone(),
            wizard.corrupted_on_load,
        )
    };

    // v0.5.16 修复：不再在持有 sidecar 锁时扫描 100 个端口（避免 500ms 阻塞其他命令）
    // 1. 短暂持有 sidecar 锁仅检查 is_running()，立即释放
    // 2. 如果桌面端无管理实例，检查 sidecar_port 指向的外部 sidecar 是否健康
    // 3. 健康检查不持有任何锁，不会阻塞 start_sidecar 等命令
    let sidecar_running = {
        let sidecar = store.sidecar.lock().await;
        sidecar.is_running()
    }; // sidecar 锁立即释放

    // 如果桌面端无管理实例，检查 sidecar_port 指向的外部 sidecar
    let sidecar_running = if sidecar_running {
        true
    } else {
        let port = *store.sidecar_port.lock().await;
        if let Some(port) = port {
            // 快速健康检查（2 秒超时，不持有 sidecar 锁）
            if let Some(probed) = SidecarManager::check_sidecar_health(port).await {
                tracing::info!(
                    "get_wizard_state: 外部 sidecar 运行中，端口 {}，uptime {}s",
                    probed.port,
                    probed.uptime_seconds
                );
                true
            } else {
                // 健康检查失败，清除过期的 sidecar_port
                let mut saved_port = store.sidecar_port.lock().await;
                *saved_port = None;
                tracing::info!(
                    "get_wizard_state: sidecar_port {} 健康检查失败，已清除",
                    port
                );
                false
            }
        } else {
            false
        }
    };

    let sidecar_port = *store.sidecar_port.lock().await;

    Ok(WizardStateResponse {
        setup_complete,
        project_dir,
        llm_configured,
        llm_type,
        llm_model,
        configured_agents,
        sidecar_running,
        sidecar_port,
        config_corrupted: corrupted,
    })
}

/// 在 Tauri 主窗口 iframe 中打开仪表盘（统一入口，不建新窗口）
///
/// v0.5.1 重构：与 navigate_main_to_dashboard 合并为统一实现，
/// 通过 show_dashboard_in_main_window 消除重复代码。
/// adjust_window=false 表示不调整窗口大小（托盘/菜单调用时保持原窗口大小）。
#[tauri::command]
pub async fn open_dashboard_window(
    app: tauri::AppHandle,
    store: State<'_, AppStore>,
) -> Result<(), String> {
    // v0.9.0 开发模式隔离：开发模式默认端口 3111
    let dev_default = if is_dev_mode() { 3111 } else { 3099 };
    let port = store.sidecar_port.lock().await.unwrap_or(dev_default);
    show_dashboard_in_main_window(&app, port, false)
}

/// 在主窗口 iframe 中显示仪表盘（向导完成后使用，调整窗口大小）
///
/// v0.5.1 重构：与 open_dashboard_window 合并为统一实现，
/// 通过 show_dashboard_in_main_window 消除重复代码。
/// adjust_window=true 表示调整窗口为 1200x800、可缩放。
#[tauri::command]
pub async fn navigate_main_to_dashboard(
    app: tauri::AppHandle,
    store: State<'_, AppStore>,
) -> Result<(), String> {
    // v0.9.0 开发模式隔离：开发模式默认端口 3111
    let dev_default = if is_dev_mode() { 3111 } else { 3099 };
    let port = store.sidecar_port.lock().await.unwrap_or(dev_default);
    show_dashboard_in_main_window(&app, port, true)
}

/// v0.5.5 P1-2：从仪表盘打开桌面端设置面板
///
/// 仪表盘嵌入模式下，用户点击"修改配置"按钮时调用。
/// 通过 Tauri 事件通知前端打开设置面板，实现仪表盘与桌面端的无缝衔接。
#[tauri::command]
pub async fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    // 发送事件到主窗口，前端监听后打开设置面板
    app.emit("open-settings", ()).map_err(|e| e.to_string())?;
    Ok(())
}

/// 更新托盘悬浮提示
///
/// 前端在 Agent 配置完成后调用，显示当前连接的 Agent 数量。
#[tauri::command]
pub async fn update_tray_tooltip(
    app: tauri::AppHandle,
    store: State<'_, AppStore>,
) -> Result<(), String> {
    let count = *store.configured_agent_count.lock().await;
    tray::update_tooltip(&app, count);
    Ok(())
}

/// 切换项目目录响应
#[derive(Serialize)]
pub struct SwitchProjectResponse {
    pub success: bool,
    pub message: String,
    pub port: u16,
    pub project_dir: String,
}

/// 切换项目目录
///
/// 更新项目路径、重启 sidecar 以重新索引新项目。
/// 契约：托盘菜单"切换项目"调用此命令。
/// v0.5.4 修复：返回结构化响应（含端口），前端无需再调用 get_sidecar_status
/// v0.5.4 P1-6 修复：错误信息人性化
#[tauri::command]
pub async fn switch_project(
    store: State<'_, AppStore>,
    app: tauri::AppHandle,
    project_dir: String,
    multi_window: Option<u32>,
) -> Result<SwitchProjectResponse, String> {
    // L3 运行时保护：速率限制检查
    {
        let mut limiter = store.rate_limiter.lock().await;
        if limiter.should_throttle("cmd:switch_project") {
            return Err(user_friendly_error("请求过于频繁"));
        }
    }

    // v0.8.14 FM-11 修复：重置取消标志，避免 start_sidecar 取消后 switch_project 永久失效
    // 根因：switch_project 复用 start_cancel_flag 但未在入口重置，用户取消启动后 flag 残留 true
    store.start_cancel_flag.store(false, Ordering::SeqCst);

    // v0.8.9 G-003：创建进度通道
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<StartProgress>(32);
    let app_for_progress = app.clone();
    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            let _ = app_for_progress.emit("sidecar-start-progress", progress);
        }
    });

    // v0.5.4 修复：切换项目前验证路径存在
    let path = std::path::Path::new(&project_dir);
    if !path.exists() {
        return Err(user_friendly_error("项目路径不存在"));
    }
    if !path.is_dir() {
        return Err(user_friendly_error("路径不是有效的目录"));
    }

    // 1. 保存新项目路径并提取 LLM 配置
    let llm_api = {
        let mut wizard = store.wizard.lock().await;
        wizard
            .set_project_dir(&project_dir)
            .map_err(|e| user_friendly_error(&e))?;
        wizard.config().to_llm_api_string()
    };

    // v0.5.6 修复：切换项目后，确保全局 IDE 规则文件存在
    // v0.5.11 修复：改为为所有已安装的 AI 工具写入规则（与 post_sidecar_start 一致）
    // v0.8.25 CODE-04 修复：添加 10s 超时保护
    // v0.9.0 开发模式隔离：开发模式下跳过规则写入（避免触碰稳定版用户配置）
    if !is_dev_mode() {
        let installed_agent_ids = {
            let registry = store.agent_registry.lock().await;
            registry
                .detect_installed()
                .iter()
                .map(|info| info.id.clone())
                .collect::<Vec<String>>()
        };
        if !installed_agent_ids.is_empty() {
            // v0.8.26 修复：克隆 Arc<Mutex> 避免生命周期逃逸
            let registry_arc = store.agent_registry.clone();
            let ids = installed_agent_ids.clone();
            let write_result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tokio::task::spawn_blocking(move || {
                    let registry = registry_arc.blocking_lock();
                    registry.write_rules_for_agents(&ids)
                }),
            )
            .await;
            match write_result {
                Ok(Ok(Ok(written))) => {
                    tracing::info!(
                        "[切换项目] 已确保 {} 个全局 IDE 规则文件存在: {}",
                        written.len(),
                        project_dir
                    );
                }
                Ok(Ok(Err(e))) => {
                    tracing::warn!("[切换项目] 全局规则文件写入失败（不影响 sidecar）: {}", e);
                }
                Ok(Err(_)) => {
                    tracing::warn!("[切换项目] 全局规则文件写入 panic（不影响 sidecar）");
                }
                Err(_) => {
                    tracing::warn!("[切换项目] 全局规则文件写入超时（10s，不影响 sidecar）");
                }
            }
        }
    }

    // 2. 重启 sidecar 以重新索引
    // v0.5.17 三阶段锁安全模式：避免在持有 sidecar 锁时执行 wait_for_health（最多 40s）
    //   Phase 1: stop + prepare_start（持锁，stop 最多 5s，无其他 I/O）→ 释放锁
    //   Phase 2: spawn_and_wait（不持锁，I/O，最多 40s）
    //   Phase 3: insert_handle（重新获取锁，<1ms）
    let project_key = project_dir.clone();

    // Phase 1: 停止旧实例 + 检查是否需要启动（持锁）
    let prepare = {
        let mut sidecar = store.sidecar.lock().await;
        if sidecar.is_running() {
            sidecar.stop().await.map_err(|e| user_friendly_error(&e))?;
        }
        // stop 后所有实例已清除，prepare_start 一定返回 NeedStart
        sidecar.prepare_start(&project_key)
    }; // sidecar 锁立即释放

    let port = match prepare {
        crate::sidecar_manager::PrepareResult::AlreadyRunning(port) => port,
        crate::sidecar_manager::PrepareResult::NeedStart => {
            // Phase 2: 启动子进程 + 健康检查（不持锁，I/O，最多 40s）
            let binary_path = {
                let sidecar = store.sidecar.lock().await;
                sidecar.binary_path().to_string()
            };

            let _dev_dd = dev_mode_data_dir();
            // v0.9.0 修复：切换项目在 dev 模式下也必须用 3111，而非 None→默认 3099，
            // 否则会复用稳定版 sidecar（违反开发/稳定隔离）。
            let target_port = dev_mode_port(None);
            let start_opts = StartOptions {
                src_dir: Some(&project_dir),
                port: Some(target_port),
                multi_window,
                llm_api: llm_api.as_deref(),
                cancel_flag: &store.start_cancel_flag,
                progress_tx: Some(&progress_tx),
                data_dir: _dev_dd.as_deref(),
            };
            // v0.8.21 FM-05 修复（interaction-resilience-auditor）：
            //   根因：switch_project 调用 spawn_and_wait 无超时保护，DNS/健康检查卡死时
            //         前端 switch_project 永久挂起，用户无法取消
            //   修复：用 tokio::time::timeout(120s) 包裹，与 start_sidecar 自动启动超时一致
            //   超时返回友好错误，前端可提示用户重试或取消
            let (child, port) = match tokio::time::timeout(
                std::time::Duration::from_secs(120),
                SidecarManager::spawn_and_wait(&binary_path, &project_key, &start_opts),
            )
            .await
            {
                Ok(Ok(result)) => result,
                Ok(Err(e)) => return Err(sidecar_error_to_user_message(&e)),
                Err(_elapsed) => {
                    // 超时：设置取消标志，让 spawn_and_wait 内部健康检查循环尽快退出
                    store.start_cancel_flag.store(true, Ordering::SeqCst);
                    return Err(user_friendly_error(
                        "切换项目超时（120s），请稍后重试或检查 LRC 服务状态",
                    ));
                }
            };

            // Phase 3: 插入实例（重新获取锁，无 I/O，<1ms）
            {
                let mut sidecar = store.sidecar.lock().await;
                sidecar.insert_handle(
                    &project_key,
                    child,
                    port,
                    Some(project_dir.clone()),
                    multi_window,
                    llm_api,
                );
            }

            port
        }
    };

    {
        let mut saved_port = store.sidecar_port.lock().await;
        *saved_port = Some(port);
    }

    Ok(SwitchProjectResponse {
        success: true,
        port,
        project_dir: project_dir.clone(),
        message: format!(
            "已切换到项目 {}，sidecar 已重启 (port={})",
            project_dir, port
        ),
    })
}

/// v0.5.3 新增：重置向导配置，让用户重新进入配置向导
///
/// 清除 setup_complete 标记和项目/Agent 配置，
/// 但保留 LLM API Key（避免用户重新输入）。
/// 调用后用户重新打开应用时将看到配置向导。
/// v0.5.4 P1-6 修复：错误信息人性化
#[tauri::command]
pub async fn reset_wizard(store: State<'_, AppStore>) -> Result<(), String> {
    let mut wizard = store.wizard.lock().await;
    wizard.reset().map_err(|e| user_friendly_error(&e))
}

/// v0.5.4 P2-16 修复：标记配置完成
///
/// 在前端 finishConfiguration() 流程结束后调用，将 setup_complete 设为 true 并持久化。
/// 修复前：finishConfiguration() 未调用此命令，导致 setup_complete 始终为 false，
/// 用户每次启动应用都需要重新配置。
#[tauri::command]
pub async fn mark_complete(store: State<'_, AppStore>) -> Result<(), String> {
    let mut wizard = store.wizard.lock().await;
    wizard.mark_complete().map_err(|e| user_friendly_error(&e))
}

/// v0.5.4 P0-4 新增：配置完成后自动验证
///
/// 验证项：
/// 1. Sidecar 是否启动成功（通过 /health 端点）
/// 2. MCP 服务器是否可达（检查端口监听）
/// 3. LLM 是否配置
/// 4. Agent 是否已配置
///
/// 返回结构化验证结果，前端据此显示"一切正常"或具体错误。
#[derive(Debug, Serialize)]
pub struct VerifySetupResult {
    /// 整体状态：true 表示所有检查通过
    pub all_ok: bool,
    /// Sidecar 状态
    pub sidecar_running: bool,
    pub sidecar_port: Option<u16>,
    pub sidecar_message: String,
    /// LLM 状态
    pub llm_configured: bool,
    pub llm_message: String,
    /// Agent 状态
    pub agents_configured: bool,
    pub agents_count: usize,
    pub agents_message: String,
    /// 项目状态
    pub project_configured: bool,
    pub project_message: String,
    /// 综合建议
    pub suggestion: String,
}

#[tauri::command]
pub async fn verify_setup(store: State<'_, AppStore>) -> Result<VerifySetupResult, String> {
    let mut result = VerifySetupResult {
        all_ok: true,
        sidecar_running: false,
        sidecar_port: None,
        sidecar_message: String::new(),
        llm_configured: false,
        llm_message: String::new(),
        agents_configured: false,
        agents_count: 0,
        agents_message: String::new(),
        project_configured: false,
        project_message: String::new(),
        suggestion: String::new(),
    };

    // ── 1. 检查 Sidecar 状态 ──
    // v0.5.4 修复：端口从 sidecar_port 获取，而非 wizard.config().port（WizardConfig 无此字段）
    // v0.5.6 修复 H-1：锁顺序改为先 sidecar（L1）后 sidecar_port（L2），避免 AB-BA 死锁
    let sidecar = store.sidecar.lock().await;
    let sidecar_running = sidecar.is_running();
    drop(sidecar);
    let port = *store.sidecar_port.lock().await;

    if sidecar_running {
        if let Some(port) = port {
            // 通过 /health 端点验证
            // v0.5.6 修复 M-7：添加 3 秒超时，避免防火墙 DROP 规则导致永久阻塞
            let health_url = format!("http://127.0.0.1:{port}/health");
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
            match client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    result.sidecar_running = true;
                    result.sidecar_port = Some(port);
                    result.sidecar_message = format!("服务运行正常（端口 {port}）");
                }
                Ok(resp) => {
                    result.all_ok = false;
                    result.sidecar_running = false;
                    result.sidecar_message =
                        format!("服务响应异常（HTTP {}）", resp.status().as_u16());
                }
                Err(e) => {
                    result.all_ok = false;
                    result.sidecar_running = false;
                    result.sidecar_message = format!("服务未响应：{}", e);
                }
            }
        } else {
            result.all_ok = false;
            result.sidecar_running = false;
            result.sidecar_message = "后台服务已启动但端口未知，请重启服务".to_string();
        }
    } else {
        result.all_ok = false;
        result.sidecar_running = false;
        result.sidecar_message = "后台服务未启动，请点击「启动服务」按钮".to_string();
    }

    // ── 2. 检查 LLM 配置 ──
    // v0.5.4 修复：使用正确的字段名 llm_configured / llm_type（而非 llm_api_key / llm_provider）
    {
        let wizard = store.wizard.lock().await;
        let llm_configured = wizard.config().llm_configured;
        result.llm_configured = llm_configured;
        if llm_configured {
            let provider = wizard.config().llm_type.clone();
            result.llm_message = format!("LLM 已配置（{}）", provider);
        } else {
            // LLM 是可选的，不标记 all_ok = false
            result.llm_message = "LLM 未配置（可选，不影响基础功能）".to_string();
        }
    }

    // ── 3. 检查 Agent 配置 ──
    {
        let wizard = store.wizard.lock().await;
        let agents = wizard.config().configured_agents.clone();
        result.agents_count = agents.len();
        if agents.is_empty() {
            result.all_ok = false;
            result.agents_message = "未配置任何 AI 工具，请返回步骤 1 重新检测".to_string();
        } else {
            result.agents_configured = true;
            result.agents_message = format!("已配置 {} 个 AI 工具", agents.len());
        }
    }

    // ── 4. 检查项目配置 ──
    {
        let wizard = store.wizard.lock().await;
        let project_dir = wizard.config().project_dir.clone();
        if let Some(dir) = project_dir {
            result.project_configured = true;
            result.project_message = format!("项目目录：{}", dir);
        } else {
            result.all_ok = false;
            result.project_message = "未设置项目目录，请返回步骤 1 选择项目".to_string();
        }
    }

    // ── 5. 综合建议 ──
    if result.all_ok {
        result.suggestion = "一切正常！你的 AI 助手现在可以使用 LRC 记忆功能了。".to_string();
    } else {
        let mut issues = Vec::new();
        if !result.sidecar_running {
            issues.push("启动后台服务");
        }
        if !result.agents_configured {
            issues.push("配置 AI 工具");
        }
        if !result.project_configured {
            issues.push("选择项目目录");
        }
        result.suggestion = format!("请先解决以下问题：{}", issues.join("、"));
    }

    tracing::info!(
        "[验证] 配置验证完成: all_ok={}, sidecar={}, llm={}, agents={}, project={}",
        result.all_ok,
        result.sidecar_running,
        result.llm_configured,
        result.agents_configured,
        result.project_configured
    );

    Ok(result)
}

/// 打开数据目录（用户友好功能：右下角"数据目录"点击后调用）
///
/// 路径策略：
///   1. 优先使用 sidecar 实际运行的数据目录（通过 sidecar_port 健康检查获取）
///   2. 回退到 ~/.loong-recall/ 根目录（跨平台）
///
/// 失败时返回用户可理解的错误消息（v0.5.4 P1-6 规范）。
#[tauri::command]
pub async fn open_data_dir(store: State<'_, AppStore>) -> Result<String, String> {
    // 阶段 1：尝试通过 sidecar HTTP API 获取实际数据目录路径
    // 这样能精确定位到当前项目的数据目录（~/.loong-recall/projects/{fp}/data/）
    let sidecar_port = {
        let port = store.sidecar_port.lock().await;
        *port
    };

    if let Some(port) = sidecar_port {
        let url = format!("http://127.0.0.1:{port}/v1/trust/data-location");
        match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?
            .get(&url)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    // API 字段为 data_directory（见 server.rs /v1/trust/data-location）
                    if let Some(path) = json.get("data_directory").and_then(|v| v.as_str()) {
                        if !path.is_empty() {
                            // 确保目录存在后打开
                            let _ = std::fs::create_dir_all(path);
                            open::that(path).map_err(|e| {
                                user_friendly_error(&format!("打开数据目录失败: {e}"))
                            })?;
                            tracing::info!("[数据目录] 已通过 sidecar API 打开: {}", path);
                            return Ok(path.to_string());
                        }
                    }
                }
            }
            _ => {
                tracing::debug!("[数据目录] sidecar API 不可用，回退到根目录");
            }
        }
    }

    // 阶段 2：回退到 ~/.loong-recall/ 根目录
    let home = dirs::home_dir()
        .ok_or_else(|| user_friendly_error("无法获取用户主目录，请检查系统环境变量。"))?;
    let root = home.join(".loong-recall");

    // 目录不存在时创建（首次使用场景）
    std::fs::create_dir_all(&root)
        .map_err(|e| user_friendly_error(&format!("创建数据目录失败: {e}")))?;

    let path_str = root.display().to_string();
    open::that(&root).map_err(|e| user_friendly_error(&format!("打开数据目录失败: {e}")))?;

    tracing::info!("[数据目录] 已打开根目录: {}", path_str);
    Ok(path_str)
}

/// v0.8.0 "归一" 新增：获取所有 AI 工具的规则文件状态
///
/// 用于信任中心展示各工具的 LRC 规则写入状态。
/// 返回 [{ tool_id, rules_path, exists, version, needs_update, last_modified }]
///
/// 此命令不依赖 sidecar，直接读取文件系统，可在 sidecar 未启动时调用。
#[tauri::command]
pub async fn get_rules_status(store: State<'_, AppStore>) -> Result<Vec<RulesStatus>, String> {
    let registry = store.agent_registry.lock().await;
    let status = registry.get_rules_status();
    tracing::info!("[v0.9.0] 规则状态查询完成，共 {} 个已安装工具", status.len());
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v0.5.4 P2-13 修复：验证中文健康检查超时错误能被正确匹配
    #[test]
    fn test_user_friendly_error_health_check_timeout_chinese() {
        let err = "Sidecar 健康检查超时：进程 PID=12345 在端口 3099-3198 范围均不可达，已尝试 20 次（10 秒）。请检查端口是否被占用或防火墙设置。";
        let friendly = user_friendly_error(err);
        assert!(
            friendly.contains("启动超时"),
            "中文健康检查超时应匹配到友好提示，实际: {}",
            friendly
        );
        assert!(
            friendly.contains("端口") || friendly.contains("防火墙"),
            "友好提示应包含端口或防火墙建议，实际: {}",
            friendly
        );
    }

    /// v0.5.4 P2-13 修复：验证英文健康检查超时错误仍能正确匹配
    #[test]
    fn test_user_friendly_error_health_check_timeout_english() {
        let err = "Sidecar health check timeout: process not responding";
        let friendly = user_friendly_error(err);
        assert!(
            friendly.contains("启动超时"),
            "英文健康检查超时应匹配到友好提示，实际: {}",
            friendly
        );
    }

    /// v0.5.4 P2-13 修复：验证通用中文超时错误能被正确匹配
    #[test]
    fn test_user_friendly_error_generic_timeout_chinese() {
        let err = "操作超时，请重试";
        let friendly = user_friendly_error(err);
        assert!(
            friendly.contains("启动超时") || friendly.contains("超时"),
            "中文超时应匹配到友好提示，实际: {}",
            friendly
        );
    }
}
