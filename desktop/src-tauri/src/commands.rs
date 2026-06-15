/// IPC 命令处理模块
///
/// 契约优先：所有命令的输入/输出结构体在此定义，
/// 前端通过 `invoke('command_name', { ... })` 调用。
use serde::Serialize;
use tauri::State;
use tauri::Manager; // Manager trait 提供 get_webview_window 等方法
use tokio::sync::Mutex; // 使用 tokio::sync::Mutex 以支持跨 await 持有

use crate::agent_detector::{AgentDetectorRegistry, AgentInfo, ProjectInfo};
use crate::config_wizard::WizardState;
use crate::sidecar_manager::SidecarManager;
use crate::tray; // 托盘模块的 open_dashboard 函数

/// 应用全局状态（线程安全，支持异步）
pub struct AppStore {
    pub wizard: Mutex<WizardState>,
    pub sidecar: Mutex<SidecarManager>,
    pub agent_registry: Mutex<AgentDetectorRegistry>,
    /// sidecar 当前端口（启动后记录，供托盘等模块使用）
    pub sidecar_port: Mutex<Option<u16>>,
    /// 已配置的 Agent 数量（供托盘 tooltip 使用）
    pub configured_agent_count: Mutex<usize>,
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

/// 获取 sidecar 运行状态
#[tauri::command]
pub async fn get_sidecar_status(
    store: State<'_, AppStore>,
) -> Result<SidecarStatusResponse, String> {
    let sidecar = store.sidecar.lock().await;
    let port_guard = store.sidecar_port.lock().await;
    let status = sidecar.status();
    Ok(SidecarStatusResponse {
        running: sidecar.is_running(),
        state: format!("{:?}", status),
        port: *port_guard,
        pid: match status {
            crate::sidecar_manager::SidecarState::Running { pid, .. } => Some(*pid),
            _ => None,
        },
    })
}

/// 启动 sidecar 进程
#[tauri::command]
pub async fn start_sidecar(
    store: State<'_, AppStore>,
    src_dir: Option<String>,
    port: Option<u16>,
    multi_window: Option<u32>,
) -> Result<u16, String> {
    // 从向导配置中读取 LLM API 配置，传递给 Sidecar
    // 修复：桌面端向导配置的 LLM 现在会正确同步到 Sidecar 服务
    let llm_api = {
        let wizard = store.wizard.lock().await;
        wizard.config().to_llm_api_string()
    };

    let mut sidecar = store.sidecar.lock().await;
    let port = sidecar.start(src_dir, port, multi_window, llm_api).await?;
    // 保存端口供其他模块（托盘等）使用
    let mut saved_port = store.sidecar_port.lock().await;
    *saved_port = Some(port);
    Ok(port)
}

/// 停止 sidecar 进程
#[tauri::command]
pub async fn stop_sidecar(
    store: State<'_, AppStore>,
) -> Result<(), String> {
    let mut sidecar = store.sidecar.lock().await;
    sidecar.stop().await?;
    // 清除端口记录
    let mut saved_port = store.sidecar_port.lock().await;
    *saved_port = None;
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
pub async fn get_llm_config(
    store: State<'_, AppStore>,
) -> Result<LlmConfigResponse, String> {
    let wizard = store.wizard.lock().await;
    let config = wizard.config();
    Ok(LlmConfigResponse {
        configured: config.llm_configured,
        llm_type: config.llm_type.clone(),
        model: config.llm_model.clone(),
    })
}

/// 保存 LLM 配置
#[tauri::command]
pub async fn save_llm_config(
    store: State<'_, AppStore>,
    llm_api: String,
) -> Result<LlmConfigResponse, String> {
    let mut wizard = store.wizard.lock().await;
    wizard.save_llm_config(&llm_api)?;
    let config = wizard.config();
    Ok(LlmConfigResponse {
        configured: config.llm_configured,
        llm_type: config.llm_type.clone(),
        model: config.llm_model.clone(),
    })
}

/// LLM 连接测试结果
#[derive(Serialize)]
pub struct LlmTestResult {
    pub success: bool,
    pub message: String,
    /// 检测到的模型列表（仅成功时返回）
    pub models: Option<Vec<String>>,
}

/// 测试 LLM API 连接（由 Rust 后端代理，避免浏览器 CSP 限制）
///
/// 前端直接向 LLM 提供商发请求会被 CSP 拦截，
/// 此命令通过 reqwest 在 Rust 侧完成网络请求，不受 CSP 限制。
#[tauri::command]
pub async fn test_llm_connection(
    provider: String,
    api_key: String,
    base_url: String,
    model: Option<String>,
) -> Result<LlmTestResult, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let base = base_url.trim_end_matches('/');

    if provider == "ollama" {
        // Ollama 本地服务测试
        let resp = client
            .get(format!("{base}/api/tags"))
            .send()
            .await
            .map_err(|e| format!("无法连接 Ollama 服务: {e}"))?;

        if resp.status().is_success() {
            let data: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {e}"))?;
            let models: Vec<String> = data["models"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect())
                .unwrap_or_default();
            Ok(LlmTestResult {
                success: true,
                message: format!("Ollama 连接成功！已安装 {} 个模型", models.len()),
                models: Some(models),
            })
        } else {
            Ok(LlmTestResult {
                success: false,
                message: format!("Ollama 返回错误 (HTTP {})", resp.status()),
                models: None,
            })
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
                let data: serde_json::Value = r.json().await.map_err(|e| format!("解析响应失败: {e}"))?;
                let models: Vec<String> = data["data"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect())
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
            Ok(r) if r.status() == 401 || r.status() == 403 => {
                Ok(LlmTestResult {
                    success: false,
                    message: "API Key 无效或无权访问，请检查 Key 是否正确".to_string(),
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
                    Ok(r) if r.status().is_success() => {
                        Ok(LlmTestResult {
                            success: true,
                            message: "连接成功！API Key 和模型均验证通过".to_string(),
                            models: None,
                        })
                    }
                    Ok(r) if r.status() == 401 || r.status() == 403 => {
                        Ok(LlmTestResult {
                            success: false,
                            message: "API Key 无效，请检查".to_string(),
                            models: None,
                        })
                    }
                    Ok(r) => {
                        Ok(LlmTestResult {
                            success: false,
                            message: format!("连接成功但模型可能不可用 (HTTP {})，请手动设置模型名", r.status()),
                            models: None,
                        })
                    }
                    Err(e) => {
                        Ok(LlmTestResult {
                            success: false,
                            message: format!("连接失败：{e}"),
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
pub async fn detect_agents(
    store: State<'_, AppStore>,
) -> Result<Vec<AgentInfo>, String> {
    let registry = store.agent_registry.lock().await;
    Ok(registry.detect_all())
}

/// 仅返回已安装的 Agent（过滤掉未安装的）
#[tauri::command]
pub async fn detect_installed_agents(
    store: State<'_, AppStore>,
) -> Result<Vec<AgentInfo>, String> {
    let registry = store.agent_registry.lock().await;
    Ok(registry.detect_installed())
}

/// 全面发现：已知工具 + 未知 dot 目录中的潜在 AI 工具
///
/// 返回 (已知工具列表, 未知工具列表)
#[tauri::command]
pub async fn discover_all_agents(
    store: State<'_, AppStore>,
) -> Result<(Vec<AgentInfo>, Vec<AgentInfo>), String> {
    let registry = store.agent_registry.lock().await;
    Ok(registry.discover_all())
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
    let registry = store.agent_registry.lock().await;
    let result = registry.configure(&agent_ids, port)?;
    // 更新 Agent 计数
    let mut count = store.configured_agent_count.lock().await;
    *count = result.len();
    // 更新托盘 tooltip
    tray::update_tooltip(&app, *count);
    // 持久化 configured_agents 到 wizard.json（P2-05 修复）
    let mut wizard = store.wizard.lock().await;
    wizard.save_configured_agents(agent_ids)?;
    Ok(result)
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
    let registry = store.agent_registry.lock().await;
    Ok(registry.scan_ide_projects(&ide_ids))
}

// ── 项目目录命令 ──

/// 获取当前项目目录
#[tauri::command]
pub async fn get_project_dir(
    store: State<'_, AppStore>,
) -> Result<Option<String>, String> {
    let wizard = store.wizard.lock().await;
    Ok(wizard.config().project_dir.clone())
}

/// 设置项目目录
#[tauri::command]
pub async fn set_project_dir(
    store: State<'_, AppStore>,
    project_dir: String,
) -> Result<(), String> {
    let mut wizard = store.wizard.lock().await;
    wizard.set_project_dir(&project_dir)
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
    /// 已配置的 Agent 列表
    pub configured_agents: Vec<String>,
    /// Sidecar 是否在运行
    pub sidecar_running: bool,
    /// Sidecar 当前端口
    pub sidecar_port: Option<u16>,
}

/// 获取向导配置状态
/// 
/// 前端用于判断是显示配置向导还是"已就绪"面板
#[tauri::command]
pub async fn get_wizard_state(
    store: State<'_, AppStore>,
) -> Result<WizardStateResponse, String> {
    let wizard = store.wizard.lock().await;
    let config = wizard.config();

    // 检查 sidecar 运行状态
    let sidecar = store.sidecar.lock().await;
    let sidecar_running = sidecar.is_running();
    let sidecar_port = store.sidecar_port.lock().await;

    Ok(WizardStateResponse {
        setup_complete: config.setup_complete,
        project_dir: config.project_dir.clone(),
        llm_configured: config.llm_configured,
        configured_agents: config.configured_agents.clone(),
        sidecar_running,
        sidecar_port: *sidecar_port,
    })
}

/// 在 Tauri 内嵌 WebView 中打开仪表盘
/// 
/// 前端通过 invoke('open_dashboard_window') 调用，
/// 代替之前的 window.open() 外部浏览器方式。
/// 注意：此命令会创建新的独立窗口，托盘菜单使用。
#[tauri::command]
pub async fn open_dashboard_window(
    app: tauri::AppHandle,
) -> Result<(), String> {
    tray::open_dashboard(&app);
    Ok(())
}

/// 导航主窗口到仪表盘（向导完成后使用，不弹新窗口）
///
/// 将主窗口直接导航到 sidecar 提供的仪表盘 URL，
/// 并调整窗口大小为 1200x800、启用可缩放。
/// 契约：向导完成 → 主窗口变为仪表盘，同一窗口内过渡。
#[tauri::command]
pub async fn navigate_main_to_dashboard(
    app: tauri::AppHandle,
    store: State<'_, AppStore>,
) -> Result<(), String> {
    let port = store
        .sidecar_port
        .lock()
        .await
        .unwrap_or(3099);

    let url = format!("http://127.0.0.1:{port}/dashboard?embedded=tauri");

    if let Some(window) = app.get_webview_window("main") {
        // 导航到仪表盘 URL
        window
            .eval(&format!("window.location.replace('{url}')"))
            .map_err(|e| format!("导航失败: {e}"))?;

        // 调整窗口大小和属性
        window
            .set_size(tauri::Size::Physical(tauri::PhysicalSize::new(1200, 800)))
            .map_err(|e| format!("调整窗口大小失败: {e}"))?;
        window
            .set_resizable(true)
            .map_err(|e| format!("设置可缩放失败: {e}"))?;
        window
            .set_title("LRC 仪表盘 — AI 代码记忆")
            .map_err(|e| format!("设置标题失败: {e}"))?;

        tracing::info!("主窗口已导航到仪表盘 (port={port})");
    } else {
        // 回退：创建新的仪表盘窗口
        tray::open_dashboard(&app);
    }

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

/// 切换项目目录
/// 
/// 更新项目路径、重启 sidecar 以重新索引新项目。
/// 契约：托盘菜单"切换项目"调用此命令。
#[tauri::command]
pub async fn switch_project(
    store: State<'_, AppStore>,
    project_dir: String,
    multi_window: Option<u32>,
) -> Result<String, String> {
    // 1. 保存新项目路径并提取 LLM 配置
    let llm_api = {
        let mut wizard = store.wizard.lock().await;
        wizard.set_project_dir(&project_dir)?;
        wizard.config().to_llm_api_string()
    };
    
    // 2. 重启 sidecar 以重新索引
    let mut sidecar = store.sidecar.lock().await;
    if sidecar.is_running() {
        sidecar.stop().await?;
    }
    let port = sidecar.start(Some(project_dir.clone()), None, multi_window, llm_api).await?;
    let mut saved_port = store.sidecar_port.lock().await;
    *saved_port = Some(port);
    
    Ok(format!("已切换到项目 {}，sidecar 已重启 (port={})", project_dir, port))
}