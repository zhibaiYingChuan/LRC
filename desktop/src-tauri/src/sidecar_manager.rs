/// Sidecar 进程管理器
///
/// 管理 lrc-sidecar 子进程的生命周期。
/// 支持多项目同时运行：每个项目对应一个独立的 sidecar 进程。
///
/// 生命周期保证：
///   - Drop 时自动 kill 所有子进程（防止僵尸进程）
///   - 启动时等待健康检查通过（最多 10 秒）
///   - 端口自适应：每个 sidecar 自动扫描可用端口
///
/// 默认端口：3099（与 sidecar 默认值一致）。
/// 注意：不要传 0，因为 0 会导致 sidecar 尝试绑定特权端口（<1024）而失败。
pub const DEFAULT_SIDECAR_PORT: u16 = 3099;
/// 端口扫描范围：实际端口 = 起始端口 + 0..PORT_SCAN_RANGE
/// 与 server.rs 中 find_available_port 的 scan_range(100) 保持一致
const PORT_SCAN_RANGE: u16 = 100;
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// Windows: 隐藏 sidecar 进程的 CMD 窗口
// 普通用户看到 CMD 窗口会困惑，且误关闭可能导致后端进程异常
// 使用 CREATE_NO_WINDOW 标志确保进程完全静默运行
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Windows 进程创建标志：CREATE_NO_WINDOW = 0x08000000
/// 进程在后台静默运行，不显示任何控制台窗口
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 获取 sidecar 日志目录
/// 与 main.rs 中 init_logging 使用相同的日志目录：$APPDATA/LoongRecall/logs/
/// v0.5.7 新增：用于将 sidecar stderr 重定向到日志文件，便于排查启动失败原因
fn get_sidecar_log_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        Some(
            std::path::PathBuf::from(appdata)
                .join("LoongRecall")
                .join("logs"),
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        // macOS/Linux: ~/.local/share/LoongRecall/logs/
        let home = std::env::var("HOME").ok()?;
        Some(std::path::PathBuf::from(home).join(".local/share/LoongRecall/logs"))
    }
}

/// 单个 Sidecar 实例的运行状态（可序列化，供前端使用）
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SidecarInstance {
    /// 项目路径（用于标识）
    pub project_dir: String,
    /// 运行状态
    pub state: SidecarState,
    /// 是否正在运行（供前端直接使用布尔值）
    pub running: bool,
    /// 实际绑定的端口
    pub port: u16,
    /// 进程 PID
    pub pid: u32,
}

/// Sidecar 运行状态
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum SidecarState {
    /// 未启动
    Stopped,
    /// 启动中（等待健康检查）
    Starting,
    /// 运行中
    Running,
    /// 发生错误
    Error(String),
}

/// v0.5.15 新增：探测到的外部 sidecar 实例信息
///
/// 应用场景：用户先打开 IDE（MCP 已连接 sidecar），再打开桌面端时，
/// 桌面端的 instances HashMap 为空，但 sidecar 实际已在端口上运行。
/// 此结构体表示通过端口扫描探测到的、非桌面端启动的 sidecar 实例。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProbedSidecar {
    /// 实际绑定的端口
    pub port: u16,
    /// 服务源码目录（从 /health 响应中获取）
    pub src_dir: String,
    /// 已运行秒数
    pub uptime_seconds: i64,
}

// ════════════════════════════════════════════════════════════════
// v0.8.9 G-003/G-004：启动进度反馈 + 结构化错误
// ════════════════════════════════════════════════════════════════

/// 启动进度事件（G-003：通过 Tauri event 通知前端）
///
/// 前端通过 `listen('sidecar-start-progress', ...)` 接收。
/// `stage` 标识当前阶段，`progress` 为 0-100 的百分比，`message` 为人类可读描述。
#[derive(Debug, Clone, serde::Serialize)]
pub struct StartProgress {
    /// 阶段标识（spawn / health_check / ready / error）
    pub stage: String,
    /// 进度百分比 0-100
    pub progress: u8,
    /// 人类可读描述
    pub message: String,
}

impl StartProgress {
    /// 创建进度事件
    pub fn new(stage: &str, progress: u8, message: impl Into<String>) -> Self {
        Self {
            stage: stage.to_string(),
            progress,
            message: message.into(),
        }
    }
}

/// 启动参数集合（v0.8.9：避免函数参数过多）
///
/// 打包 sidecar 启动所需的运行时配置，使 `spawn_and_wait` / `start_for_project`
/// / `restart_project` 的函数签名保持简洁（≤ 3 个参数），避免触发
/// `clippy::too_many_arguments`。
///
/// 所有字段均为引用或 Copy 类型，结构体本身可按值传递。
pub struct StartOptions<'a> {
    /// 服务源码目录
    pub src_dir: Option<&'a str>,
    /// 指定端口（None 则使用默认端口 3099）
    pub port: Option<u16>,
    /// 多窗口数量
    pub multi_window: Option<u32>,
    /// LLM API 配置字符串（`||` 分隔）
    pub llm_api: Option<&'a str>,
    /// 取消标志（G-001：用于中断启动流程）
    pub cancel_flag: &'a AtomicBool,
    /// 进度事件发送端（G-003：用于通知前端启动进度）
    pub progress_tx: Option<&'a tokio::sync::mpsc::Sender<StartProgress>>,
    /// 自定义数据目录（v0.8.37 新增，用于开发版与稳定版数据隔离）
    /// 设置后 sidecar 使用此目录而非默认的 ~/.loong-recall/global/data/
    pub data_dir: Option<&'a str>,
}

/// 通过 PID 强制终止进程
///
/// 用于 SingletonConflict 场景：健康检查无法检测到现有 sidecar 时，
/// 强制终止旧进程后重新启动新 sidecar。
///
/// v0.8.39 修复（v2）：如果进程已不存在（taskkill/kill -9 报错 "not found"），
/// 视为终止成功。这是修复竞态条件的核心——PID 在检测和终止之间已消失时，
/// 不应返回 false 导致启动失败。
///
/// v0.8.41 修复：在 kill 前验证进程名，避免误杀被 PID 重用后的其他进程。
/// 这是 E008:noport 根因修复的一部分——PID 可能已被系统进程重用。
///
/// Windows 使用 taskkill /F，Unix 使用 SIGKILL
pub fn kill_process_by_pid(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        // 步骤 1: 检查进程是否存在，并验证进程名是否匹配 sidecar
        // 使用 tasklist /FI "PID eq <pid>" /FO CSV /NH 查询
        // CSV 输出格式: "image_name.exe","pid","session_name","session#","mem_usage"
        let check = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
            .output();
        let process_status = match &check {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let lower = stdout.to_lowercase();

                // 检查 PID 是否存在于输出中
                if !lower.contains(&pid.to_string()) {
                    // PID 不存在 → 进程已消失，视为终止成功
                    "not_found"
                } else if lower.contains("lrc-sidecar")
                    || lower.contains("lrc_sidecar")
                    || lower.contains("code-memory")
                    || lower.contains("code_memory")
                {
                    // PID 存在且进程名匹配 sidecar → 可以安全终止
                    "sidecar"
                } else {
                    // PID 存在但进程名不是 sidecar → PID 已被其他进程重用！
                    // 这是 E008:noport 的根因：旧 sidecar 已死，PID 被其他进程占用
                    tracing::warn!(
                        "PID={} 已被其他进程重用（进程名不匹配 sidecar），跳过终止",
                        pid
                    );
                    "reused"
                }
            }
            Err(_) => {
                // v0.8.43 修复：tasklist 执行失败时视为"未找到进程"而非"sidecar"。
                // 根因：保守返回 "sidecar" 导致 taskkill 对已不存在的 PID 执行失败，
                // 返回 false，阻止桌面端继续启动流程，形成卡死。
                "not_found"
            }
        };

        match process_status {
            "not_found" => {
                tracing::warn!(
                    "旧 sidecar 进程 (PID={}) 已不存在，无需终止（竞态条件已处理）",
                    pid
                );
                true
            }
            "reused" => {
                // PID 被其他进程重用，不能 kill，但视为"终止成功"（因为旧 sidecar 已死）
                // 锁文件中的残留记录会在新 sidecar 的 acquire() 中被清理
                tracing::warn!(
                    "PID={} 已被其他进程重用，旧 sidecar 已死，视为终止成功",
                    pid
                );
                true
            }
            _ => {
                // 步骤 2: 确认是 sidecar 进程，强制终止
                match std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .output()
                {
                    Ok(output) => {
                        if output.status.success() {
                            tracing::warn!("已强制终止旧 sidecar 进程 (PID={})", pid);
                            true
                        } else {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            tracing::warn!("终止旧 sidecar 进程 (PID={}) 失败: {}", pid, stderr);
                            false
                        }
                    }
                    Err(e) => {
                        tracing::warn!("执行 taskkill 失败 (PID={}): {}", pid, e);
                        false
                    }
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Unix: 先检查进程是否还存在
        let alive_check = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output();
        let process_alive = match &alive_check {
            Ok(output) => output.status.success(),
            Err(_) => true,
        };
        if !process_alive {
            tracing::warn!(
                "旧 sidecar 进程 (PID={}) 已不存在，无需终止（竞态条件已处理）",
                pid
            );
            return true;
        }
        // 检查进程名是否匹配 sidecar
        let comm_path = std::path::PathBuf::from(format!("/proc/{}/comm", pid));
        if let Ok(content) = std::fs::read_to_string(&comm_path) {
            let name = content.trim();
            if !name.contains("lrc-sidecar")
                && !name.contains("lrc_sidecar")
                && !name.contains("code-memory")
                && !name.contains("code_memory")
            {
                tracing::warn!(
                    "PID={} 进程名 '{}' 不匹配 sidecar，跳过终止（PID 已重用）",
                    pid,
                    name
                );
                return true;
            }
        }
        // 先尝试 SIGTERM（优雅终止）
        let result = std::process::Command::new("kill")
            .arg(&pid.to_string())
            .output();
        if let Ok(output) = &result {
            if output.status.success() {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
        // 再尝试 SIGKILL（强制终止）
        match std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    tracing::warn!("已强制终止旧 sidecar 进程 (PID={})", pid);
                    true
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::warn!("终止旧 sidecar 进程 (PID={}) 失败: {}", pid, stderr);
                    false
                }
            }
            Err(e) => {
                tracing::warn!("执行 kill 失败 (PID={}): {}", pid, e);
                false
            }
        }
    }
}

/// 结构化启动错误（G-004：错误码 + 分类体系）
///
/// 替代原先的 `String` 错误，提供机器可读的错误码和分类，
/// 前端可根据 `code` 做差异化处理（如端口冲突时提示"先停止现有服务"）。
#[derive(Debug, Clone, serde::Serialize)]
pub enum SidecarStartError {
    /// 二进制文件未找到（E001）
    BinaryNotFound { path: String },
    /// 进程启动失败（E002）
    SpawnFailed { reason: String },
    /// 健康检查超时（E003）
    HealthCheckTimeout { port: u16, attempts: u32 },
    /// 子进程意外退出（E004）
    ProcessDied {
        pid: u32,
        log_hint: String,
        log_empty: bool,
    },
    /// 用户取消启动（E005）
    UserCancelled,
    /// 端口被外部 sidecar 占用（E006）
    PortConflict { port: u16, src_dir: String },
    /// HTTP 客户端创建失败（E007）
    HttpClientError { reason: String },
    /// 单例锁冲突（E008，v0.8.17 新增）— sidecar 退出码 2
    ///
    /// 场景：已有 sidecar 实例在运行，新 sidecar 因锁冲突主动 exit(2) 退出。
    /// 与 ProcessDied 的区别：这不是崩溃，而是 sidecar 主动退出让位给已有实例。
    /// 修复策略：提示用户"已有实例运行"，提供"复用现有实例"按钮（扫描健康端口）。
    /// existing_port 为已探测到的健康 sidecar 端口（None 表示未探测到）。
    SingletonConflict {
        pid: u32,
        existing_port: Option<u16>,
    },
}

impl SidecarStartError {
    /// 错误码（机器可读，前端可据此做差异化处理）
    pub fn code(&self) -> &'static str {
        match self {
            Self::BinaryNotFound { .. } => "E001",
            Self::SpawnFailed { .. } => "E002",
            Self::HealthCheckTimeout { .. } => "E003",
            Self::ProcessDied { .. } => "E004",
            Self::UserCancelled => "E005",
            Self::PortConflict { .. } => "E006",
            Self::HttpClientError { .. } => "E007",
            Self::SingletonConflict { .. } => "E008",
        }
    }
}

impl std::fmt::Display for SidecarStartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BinaryNotFound { path } => {
                write!(f, "LRC 服务程序未找到: {path}")
            }
            Self::SpawnFailed { reason } => {
                write!(f, "启动 sidecar 失败: {reason}")
            }
            Self::HealthCheckTimeout { port, attempts } => {
                write!(f, "健康检查超时（端口 {port}, 尝试 {attempts} 次）")
            }
            Self::ProcessDied { pid, log_hint, .. } => {
                write!(f, "Sidecar 进程 PID={pid} 启动后意外退出{log_hint}")
            }
            Self::UserCancelled => write!(f, "用户取消启动"),
            Self::PortConflict { port, src_dir } => {
                write!(f, "端口 {port} 已有 sidecar 运行（src_dir: {src_dir}），请先停止现有实例或复用该端口")
            }
            Self::HttpClientError { reason } => {
                write!(f, "创建 HTTP 客户端失败: {reason}")
            }
            Self::SingletonConflict { pid, existing_port } => {
                if let Some(port) = existing_port {
                    write!(
                        f,
                        "已有 LRC 实例在运行（PID={pid}，端口 {port}），已自动复用现有实例"
                    )
                } else {
                    write!(
                        f,
                        "已有 LRC 实例在运行（PID={pid}），请复用现有实例或先停止后再启动"
                    )
                }
            }
        }
    }
}

impl std::error::Error for SidecarStartError {}

/// 允许 `?` 运算符在返回 `Result<_, String>` 的函数中自动转换
impl From<SidecarStartError> for String {
    fn from(e: SidecarStartError) -> Self {
        e.to_string()
    }
}

/// v0.5.17 新增：启动准备结果
///
/// 三阶段锁安全模式的第一阶段返回值：
///   Phase 1 (prepare_start)  → PrepareResult
///   Phase 2 (spawn_and_wait) → (Child, u16)
///   Phase 3 (insert_handle)  → ()
///
/// 此枚举用于 Phase 1，表示是否需要继续 Phase 2。
#[derive(Debug)]
pub enum PrepareResult {
    /// 项目已运行，直接返回端口（无需 Phase 2/3）
    AlreadyRunning(u16),
    /// 需要启动新实例（继续 Phase 2/3）
    NeedStart,
}

/// v0.5.17 新增：死亡实例信息（用于崩溃恢复的三阶段编排）
///
/// Phase 1 (collect_dead_instances) 返回此结构体列表，
/// 调用方在释放锁后遍历列表执行 Phase 2 (spawn_and_wait)，
/// 然后重新获取锁执行 Phase 3 (insert_handle)。
#[derive(Debug, Clone)]
pub struct DeadInstanceInfo {
    /// 项目标识
    pub project_key: String,
    /// 源码目录
    pub src_dir: Option<String>,
    /// 多窗口配置
    pub multi_window: Option<u32>,
    /// LLM API 配置
    pub llm_api: Option<String>,
}

/// Sidecar 进程管理器（支持多项目）
pub struct SidecarManager {
    /// 所有运行中的 sidecar 实例，按项目路径索引
    instances: HashMap<String, SidecarHandle>,
    /// 二进制路径
    binary_path: String,
}

/// 单个 sidecar 进程句柄
struct SidecarHandle {
    child: Child,
    port: u16,
    project_dir: String,
    /// v0.5.1 新增：保存启动参数，用于崩溃恢复时自动重启
    src_dir: Option<String>,
    multi_window: Option<u32>,
    llm_api: Option<String>,
}

/// Drop 守卫：确保所有子进程在管理器被销毁时被 kill 并回收
impl Drop for SidecarManager {
    fn drop(&mut self) {
        for (project_dir, handle) in self.instances.drain() {
            let pid = handle.child.id();
            tracing::info!(
                "SidecarManager 释放，kill 子进程 project={}, PID={}",
                project_dir,
                pid
            );
            // 尝试优雅终止
            let mut child = handle.child;
            let _ = child.kill();
            // M-12 修复：kill 后必须 wait 回收子进程，否则产生僵尸进程
            // 使用 try_wait 轮询 + 短超时（3 秒），避免 Drop 中无限阻塞
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                match child.try_wait() {
                    Ok(Some(_status)) => {
                        tracing::debug!("子进程 PID={} 已退出，僵尸进程已回收", pid);
                        break;
                    }
                    Ok(None) => {
                        // 进程仍在运行，检查是否超时
                        if std::time::Instant::now() >= deadline {
                            tracing::warn!(
                                "等待子进程 PID={} 退出超时（3秒），可能残留僵尸进程",
                                pid
                            );
                            break;
                        }
                        // 短暂休眠后重试，避免 CPU 空转
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(e) => {
                        tracing::warn!("等待子进程 PID={} 退出失败: {}", pid, e);
                        break;
                    }
                }
            }
        }
    }
}

impl SidecarManager {
    /// 创建新的 sidecar 管理器
    ///
    /// 自动搜索多个可能位置（按优先级）：
    /// 1. 指定的 binary_path
    /// 2. 同目录下的 lrc-sidecar.exe
    /// 3. resources/ 子目录
    pub fn new(binary_path: String) -> Self {
        // 自动搜索 sidecar 二进制（如果指定路径不存在）
        let resolved_path = if std::path::Path::new(&binary_path).exists() {
            binary_path
        } else {
            Self::find_sidecar_binary()
        };
        tracing::info!("Sidecar 二进制路径: {resolved_path}");
        Self {
            instances: HashMap::new(),
            binary_path: resolved_path,
        }
    }

    /// 创建用于测试的 sidecar 管理器（跳过自动搜索）
    ///
    /// 直接使用指定的 binary_path，不自动搜索真实 sidecar 二进制。
    /// 避免测试意外启动真实 sidecar 进程导致卡住。
    #[cfg(test)]
    pub fn for_testing(binary_path: String) -> Self {
        Self {
            instances: HashMap::new(),
            binary_path,
        }
    }

    /// 自动搜索 sidecar 二进制文件
    fn find_sidecar_binary() -> String {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let binary_name = format!("lrc-sidecar{}", std::env::consts::EXE_SUFFIX);

        // 搜索路径（按优先级）
        // Windows: resources 与 exe 同目录（安装目录根）
        // macOS: resources 在 Contents/Resources/，exe 在 Contents/MacOS/
        // Linux AppImage: resources 在挂载点根目录
        let search_paths: Vec<std::path::PathBuf> = [
            exe_dir.join(&binary_name), // 同目录（Windows 安装目录根）
            exe_dir.join("resources").join(&binary_name), // resources/ 子目录
            exe_dir.parent().unwrap_or(&exe_dir).join(&binary_name), // 上级目录
            exe_dir
                .parent()
                .unwrap_or(&exe_dir)
                .join("Resources")
                .join(&binary_name), // macOS: Contents/Resources/
        ]
        .into_iter()
        .collect();

        for path in &search_paths {
            if path.exists() {
                tracing::info!("找到 sidecar: {}", path.display());
                return path.display().to_string();
            }
        }

        // 回退到默认路径（在 Tauri 开发模式下）
        tracing::warn!("未找到 sidecar 二进制，使用默认路径");
        exe_dir.join(&binary_name).display().to_string()
    }

    /// 获取所有运行中的实例信息
    pub fn list_instances(&self) -> Vec<SidecarInstance> {
        self.instances
            .iter()
            .map(|(project_dir, handle)| SidecarInstance {
                project_dir: project_dir.clone(),
                state: SidecarState::Running,
                running: true,
                port: handle.port,
                pid: handle.child.id(),
            })
            .collect()
    }

    /// 获取指定项目的实例信息
    pub fn get_instance(&self, project_dir: &str) -> Option<SidecarInstance> {
        self.instances
            .get(project_dir)
            .map(|handle| SidecarInstance {
                project_dir: handle.project_dir.clone(),
                state: SidecarState::Running,
                running: true,
                port: handle.port,
                pid: handle.child.id(),
            })
    }

    /// 检查是否有 sidecar 正在运行
    pub fn is_running(&self) -> bool {
        !self.instances.is_empty()
    }

    /// 检查指定项目的 sidecar 是否正在运行
    pub fn is_project_running(&self, project_dir: &str) -> bool {
        self.instances.contains_key(project_dir)
    }

    /// 获取 sidecar 二进制路径（供关联函数 spawn_and_wait 使用）
    pub fn binary_path(&self) -> &str {
        &self.binary_path
    }

    // ════════════════════════════════════════════════════════════════
    // v0.5.17 三阶段锁安全模式
    //
    // 解决 v0.5.15/v0.5.16 审计发现的隐藏锁竞争问题：
    //   - get_sidecar_status 持有锁调用 recover_dead_instances（最多 40s）
    //   - 心跳协程持有锁调用 recover_dead_instances（最多 40s）
    //   - start_sidecar 持有锁调用 start → wait_for_health（最多 40s）
    //   - start_sidecar_for_project 持有锁调用 start_for_project（最多 40s）
    //   - switch_project 持有锁调用 start（最多 40s）
    //
    // 三阶段模式：
    //   Phase 1: prepare_start / collect_dead_instances
    //            检查状态 + 移除死亡实例，返回参数（持锁，无 I/O，<1ms）
    //   Phase 2: spawn_and_wait（关联函数）
    //            启动子进程 + 健康检查（释放锁，I/O，最多 40s）
    //   Phase 3: insert_handle
    //            插入新实例（重新获取锁，无 I/O，<1ms）
    // ════════════════════════════════════════════════════════════════

    /// Phase 1（启动场景）：检查项目是否已运行，如已死亡则移除
    ///
    /// **不执行任何 I/O**，可安全在持有 sidecar 锁时调用。
    /// 执行时间 < 1ms（仅 HashMap 查找 + try_wait）。
    ///
    /// 返回 `PrepareResult::AlreadyRunning(port)` 表示无需启动，
    /// 返回 `PrepareResult::NeedStart` 表示需要执行 Phase 2。
    pub fn prepare_start(&mut self, project_key: &str) -> PrepareResult {
        if let Some(handle) = self.instances.get_mut(project_key) {
            if Self::is_process_alive(&mut handle.child) {
                tracing::info!(
                    "项目 {} 的 sidecar 已在运行 (PID={}, port={})",
                    project_key,
                    handle.child.id(),
                    handle.port
                );
                return PrepareResult::AlreadyRunning(handle.port);
            }
            // 进程已死，清理
            tracing::warn!("项目 {} 的 sidecar 已退出，重新启动", project_key);
            self.instances.remove(project_key);
        }
        PrepareResult::NeedStart
    }

    /// Phase 1（崩溃恢复场景）：收集所有死亡实例信息并从 instances 移除
    ///
    /// **不执行任何 I/O**，可安全在持有 sidecar 锁时调用。
    /// 执行时间 < 1ms（仅遍历 HashMap + try_wait）。
    ///
    /// 返回死亡实例列表，调用方在释放锁后遍历执行 Phase 2/3。
    pub fn collect_dead_instances(&mut self) -> Vec<DeadInstanceInfo> {
        let mut dead_keys = Vec::new();
        for (key, handle) in self.instances.iter_mut() {
            if !Self::is_process_alive(&mut handle.child) {
                dead_keys.push(key.clone());
            }
        }

        let mut dead_instances = Vec::new();
        for key in dead_keys {
            if let Some(handle) = self.instances.remove(&key) {
                tracing::warn!("检测到 sidecar 已死亡: 项目={}, 端口={}", key, handle.port);
                dead_instances.push(DeadInstanceInfo {
                    project_key: key,
                    src_dir: handle.src_dir,
                    multi_window: handle.multi_window,
                    llm_api: handle.llm_api,
                });
            }
        }
        dead_instances
    }

    /// Phase 2：启动子进程 + 等待健康检查（关联函数，不持有锁）
    ///
    /// **这是最耗时的阶段**（最多 40 秒），**必须在释放 sidecar 锁的情况下调用**。
    ///
    /// 此函数是关联函数（不需要 `&self`），因此调用方可以在释放 sidecar 锁后
    /// 安全调用，不会阻塞其他需要 sidecar 锁的命令。
    ///
    /// 返回 `(Child, port)` 供 Phase 3 使用。
    pub async fn spawn_and_wait(
        binary_path: &str,
        project_key: &str,
        opts: &StartOptions<'_>,
    ) -> Result<(Child, u16), SidecarStartError> {
        // 解构启动参数（所有字段均为 Copy 类型，按值取出）
        let StartOptions {
            src_dir,
            port,
            multi_window,
            llm_api,
            cancel_flag,
            progress_tx,
            data_dir,
        } = *opts;
        let actual_port = port.unwrap_or(DEFAULT_SIDECAR_PORT);

        // G-003：发送"正在检查端口"进度
        if let Some(tx) = progress_tx {
            let _ = tx.try_send(StartProgress::new("port_check", 5, "正在检查端口..."));
        }

        // 构建启动参数
        let mut cmd = Command::new(binary_path);
        // Windows: 隐藏 sidecar 进程的 CMD 窗口
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);
        // 守护模式：不自动打开浏览器（桌面端自行管理 UI）
        cmd.args(["--daemon", "--port", &actual_port.to_string()]);

        if let Some(dir) = src_dir {
            cmd.args(["--src-dir", dir]);
        } else {
            // v0.6.0 修复:桌面端无 src_dir 时使用全局模式,避免基于 cwd 生成指纹目录
            // 桌面端场景无明确项目概念,全局模式更符合用户直觉
            cmd.arg("--global");
        }

        // v0.8.37 新增：自定义数据目录（开发版与稳定版数据隔离）
        if let Some(dd) = data_dir {
            cmd.args(["--data-dir", dd]);
            tracing::info!("使用自定义数据目录: {}", dd);
        }

        // 多窗口模式
        if let Some(mw) = multi_window {
            cmd.args(["--multi-window", &mw.to_string()]);
            tracing::info!("多窗口模式：{} 个 LRC 实例上限", mw);
        }

        // v0.5.4 安全修复：使用环境变量传递 LLM API Key
        if let Some(llm) = llm_api {
            if !llm.is_empty() {
                cmd.env("LRC_LLM_API", llm);
                let llm_type = if llm.contains("||") {
                    llm.split("||").next()
                } else {
                    llm.split(':').next()
                }
                .unwrap_or("unknown");
                tracing::info!(
                    "已通过环境变量传递 LLM 配置到 Sidecar（项目: {}, 类型: {}）",
                    project_key,
                    llm_type
                );
            }
        }

        // v0.5.7 修复：将 sidecar stderr 重定向到日志文件
        if let Some(log_dir) = get_sidecar_log_dir() {
            let _ = std::fs::create_dir_all(&log_dir);
            let log_path = log_dir.join("lrc-sidecar.log");
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                Ok(file) => {
                    cmd.stderr(Stdio::from(file));
                    tracing::debug!("Sidecar stderr 重定向到: {}", log_path.display());
                }
                Err(e) => {
                    tracing::warn!("无法打开 sidecar 日志文件 {}: {}", log_path.display(), e);
                }
            }
        }

        // v0.8.9 G-002/G-009：spawn 前检查目标端口是否已有健康 sidecar
        // 场景：桌面端崩溃后重启，旧 sidecar 仍在端口上运行。
        // 如果直接 spawn，新 sidecar 会因端口冲突绑定到其他端口，
        // 导致两个 sidecar 进程同时运行（孤儿进程问题）。
        // 修复：检测到已有健康 sidecar 时返回错误，由调用方决定是否复用。
        // 使用 200ms 超时：端口未开放时应快速失败，不阻塞 spawn 流程。
        // check_sidecar_health 自带 2s 超时太长，用 tokio::time::timeout 包裹。
        let port_check = tokio::time::timeout(
            Duration::from_millis(200),
            Self::check_sidecar_health(actual_port),
        )
        .await;
        if let Ok(Some(probed)) = port_check {
            tracing::warn!(
                "G-002：端口 {} 已有健康 sidecar 运行（src_dir: {}, uptime: {}s），spawn 被阻止",
                actual_port,
                probed.src_dir,
                probed.uptime_seconds
            );
            return Err(SidecarStartError::PortConflict {
                port: actual_port,
                src_dir: probed.src_dir,
            });
        }

        // G-003：发送"正在启动服务进程"进度
        if let Some(tx) = progress_tx {
            let _ = tx.try_send(StartProgress::new("spawn", 10, "正在启动 LRC 服务进程..."));
        }

        // v0.8.15 P1-1 修复：显式设置 sidecar 子进程 cwd，避免在 System32 下运行
        // 根因：从开始菜单快捷方式启动时 cwd 可能为 C:\Windows\System32
        // sidecar 内部可能基于 cwd 做相对路径操作，导致权限或路径异常
        if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
            let work_dir = std::path::Path::new(&home).join(".loong-recall");
            let _ = std::fs::create_dir_all(&work_dir);
            cmd.current_dir(&work_dir);
            tracing::debug!("Sidecar cwd 设置为: {}", work_dir.display());
        }

        // 启动子进程
        let mut child = cmd.spawn().map_err(|e| SidecarStartError::SpawnFailed {
            reason: e.to_string(),
        })?;

        let pid = child.id();

        // v0.8.17 P0-3 + G-001 修复：将 100ms → 500ms → 1500ms，覆盖单例锁冲突时间窗口
        // 根因：sidecar 需先执行 risk_aware_guard + CLI 解析 + 配置推导 + SingletonLock::acquire，
        // HCSE 韧性验证实测平均耗时 1035ms（3 次测量一致）。500ms 时 try_wait 返回 Ok(None)，
        // fast-path 为死代码，E008 检测延迟至 wait_for_health_static 第 2-3 次迭代（~1500ms）。
        // 1500ms 能稳定捕获单例锁冲突（exit code 2）+ DLL 加载失败 + 配置错误。
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // v0.8.17 P0-2 修复：检查进程是否已退出，并获取退出码区分错误类型
        // 退出码协议：2=单例锁冲突，3=端口冲突，4=数据目录错误，5=锁获取失败，1=其他
        match child.try_wait() {
            Ok(Some(status)) => {
                let exit_code = status.code().unwrap_or(1);

                if exit_code == 2 {
                    // 退出码 2 = 单例锁冲突：sidecar 检测到已有实例运行，主动退出让位
                    tracing::warn!(
                        "v0.8.17：sidecar PID={pid} 因单例锁冲突主动退出（exit code 2），\
                         尝试探测已有 sidecar 实例以复用"
                    );
                    // 探测 actual_port 是否有健康 sidecar
                    let existing_port = if Self::check_sidecar_health(actual_port).await.is_some() {
                        Some(actual_port)
                    } else {
                        // actual_port 未找到，扫描相邻端口（端口自适应范围）
                        Self::find_healthy_sidecar_port(actual_port).await
                    };

                    // v0.8.43 修复：无健康 sidecar 时清理锁文件中的残留 PID，打破死亡螺旋
                    // 根因：exit code 2 仅表示 sidecar 检测到锁文件中有"存活"的 PID，
                    // 但该 PID 可能已被其他进程重用（is_pid_alive 误判）。
                    // 桌面端收到 SingletonConflict 后尝试终止 sidecar 的 PID（已死），
                    // 但锁文件中的残留 PID 永远不会被清理，新 sidecar 重复同一错误。
                    if existing_port.is_none() {
                        let lock_dir = data_dir.map(std::path::PathBuf::from)
                            .unwrap_or_else(|| {
                                // 默认全局数据目录（与 sidecar --global 一致）
                                let home = std::env::var("USERPROFILE")
                                    .or_else(|_| std::env::var("HOME"))
                                    .unwrap_or_else(|_| ".".to_string());
                                std::path::PathBuf::from(home)
                                    .join(".loong-recall")
                                    .join("global")
                                    .join("data")
                            });
                        let lock_path = lock_dir.join(".lrc.lock");
                        if lock_path.exists() {
                            if let Ok(content) = std::fs::read_to_string(&lock_path) {
                                let stale_pids: Vec<&str> = content.split(',')
                                    .map(|s| s.trim())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                                tracing::warn!(
                                    "E008 死亡螺旋检测：锁文件 {} 含残留 PID=[{}]，清理后重试",
                                    lock_path.display(),
                                    stale_pids.join(",")
                                );
                            }
                            // 清空锁文件（删除残留 PID，让新 sidecar 创建新锁）
                            let _ = std::fs::write(&lock_path, "");
                            tracing::warn!(
                                "已清空锁文件 {}，打破 E008 死亡螺旋",
                                lock_path.display()
                            );
                        }
                    }

                    return Err(SidecarStartError::SingletonConflict { pid, existing_port });
                }

                // 其他退出码 = 真实崩溃（DLL 缺失、配置错误等）
                // v0.8.17 G-017 修复：为 exit code 3/4/5 添加专属诊断信息，避免全部误判为"意外退出"
                let exit_code_hint = match exit_code {
                    3 => "（退出码 3 = 端口绑定失败 NoAvailablePort，请检查端口占用）",
                    4 => "（退出码 4 = 数据目录错误 DataDirNotAvailable，请检查数据目录权限）",
                    5 => "（退出码 5 = 锁获取失败 LockAcquireFailed，请清理 .lrc.lock 文件）",
                    _ => "",
                };
                let (log_hint, log_empty) = get_sidecar_log_dir()
                    .map(|d| {
                        let log_path = d.join("lrc-sidecar.log");
                        let content = std::fs::read_to_string(&log_path).unwrap_or_default();
                        let is_empty = content.trim().is_empty();
                        let hint = if is_empty {
                            format!("{exit_code_hint}，日志为空（{}\\lrc-sidecar.log），疑似运行时依赖缺失", d.display())
                        } else {
                            let last_lines: Vec<&str> = content.lines().rev().take(3).collect();
                            format!("{exit_code_hint}，日志末尾: {}（完整日志: {}\\lrc-sidecar.log）", last_lines.join(" | "), d.display())
                        };
                        (hint, is_empty)
                    })
                    .unwrap_or_else(|| (exit_code_hint.to_string(), false));
                tracing::error!(
                    "v0.8.17：sidecar PID={pid} 启动后 1500ms 内退出（exit code {}）{log_hint}",
                    exit_code
                );
                return Err(SidecarStartError::ProcessDied {
                    pid,
                    log_hint,
                    log_empty,
                });
            }
            Ok(None) => {
                // 进程仍在运行，继续健康检查
            }
            Err(e) => {
                tracing::warn!("try_wait 失败 (pid: {:?}): {}", pid, e);
            }
        }

        // G-003：发送"服务进程已启动"进度
        if let Some(tx) = progress_tx {
            let _ = tx.try_send(StartProgress::new(
                "health_check",
                15,
                format!("服务进程已启动 (PID={pid}), 正在健康检查..."),
            ));
        }

        // 等待健康检查通过
        // v0.8.9 修复 G-010：健康检查失败时显式 kill 子进程，防止孤儿进程
        // std::process::Child 的 Drop 不会 kill 子进程，必须显式 kill + wait
        let port =
            match Self::wait_for_health_static(&mut child, actual_port, cancel_flag, progress_tx)
                .await
            {
                Ok(port) => port,
                Err(e) => {
                    tracing::warn!("健康检查失败，正在清理子进程 (pid: {:?}): {}", pid, e);
                    // v0.8.10 L5-03：kill/wait 错误不再静默吞掉，记录日志便于排查
                    if let Err(kill_err) = child.kill() {
                        tracing::error!(
                            "清理子进程失败 (pid: {:?}): kill 返回错误: {}",
                            pid,
                            kill_err
                        );
                    }
                    if let Err(wait_err) = child.wait() {
                        tracing::warn!("等待子进程退出失败 (pid: {:?}): {}", pid, wait_err);
                    }
                    return Err(e);
                }
            };

        // G-003：发送"服务已就绪"进度
        if let Some(tx) = progress_tx {
            let _ = tx.try_send(StartProgress::new(
                "ready",
                100,
                format!("服务已就绪 (port={port})"),
            ));
        }

        tracing::info!(
            "Sidecar 已启动: 项目={}, PID={}, port={}",
            project_key,
            pid,
            port
        );
        Ok((child, port))
    }

    /// Phase 3：插入实例到管理器
    ///
    /// **不执行任何 I/O**，可安全在持有 sidecar 锁时调用。
    /// 执行时间 < 1ms（仅 HashMap 插入）。
    pub fn insert_handle(
        &mut self,
        project_key: &str,
        child: Child,
        port: u16,
        src_dir: Option<String>,
        multi_window: Option<u32>,
        llm_api: Option<String>,
    ) {
        let project_dir = src_dir.clone().unwrap_or_else(|| project_key.to_string());
        self.instances.insert(
            project_key.to_string(),
            SidecarHandle {
                child,
                port,
                project_dir,
                src_dir,
                multi_window,
                llm_api,
            },
        );
    }

    /// 等待 sidecar 健康检查通过（关联函数版本，不需要 &self）
    ///
    /// 端口自适应扫描：sidecar 可能因端口冲突而绑定到不同端口，
    /// 因此从起始端口开始扫描 PORT_SCAN_RANGE 个端口，找到实际绑定的端口。
    ///
    /// 每 500ms 检查一次，最多尝试 20 次（10 秒）。
    /// 注意：单端口 HTTP 超时为 2 秒，最坏情况下 20 × 2 = 40 秒。
    async fn wait_for_health_static(
        child: &mut Child,
        start_port: u16,
        cancel_flag: &AtomicBool,
        progress_tx: Option<&tokio::sync::mpsc::Sender<StartProgress>>,
    ) -> Result<u16, SidecarStartError> {
        let pid = child.id();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| SidecarStartError::HttpClientError {
                reason: e.to_string(),
            })?;

        for attempt in 1..=20 {
            // v0.8.9 G-001：检查取消标志，前端 abort 时终止等待
            if cancel_flag.load(Ordering::SeqCst) {
                return Err(SidecarStartError::UserCancelled);
            }

            // G-003：发送进度事件
            if let Some(tx) = progress_tx {
                let _ = tx.try_send(StartProgress::new(
                    "health_check",
                    (attempt as u8 * 5).min(95),
                    format!("健康检查第 {attempt}/20 次"),
                ));
            }

            // v0.8.17 P0-2 修复：检查进程是否还活着，并获取退出码区分错误类型
            // 退出码协议：2=单例锁冲突，3=端口冲突，4=数据目录错误，5=锁获取失败，1=其他
            match child.try_wait() {
                Ok(Some(status)) => {
                    let exit_code = status.code().unwrap_or(1);

                    if exit_code == 2 {
                        // 退出码 2 = 单例锁冲突：sidecar 主动退出让位给已有实例
                        tracing::warn!(
                            "v0.8.17：sidecar PID={pid} 在健康检查期间因单例锁冲突退出（exit code 2）"
                        );
                        let existing_port = Self::find_healthy_sidecar_port(start_port).await;
                        return Err(SidecarStartError::SingletonConflict { pid, existing_port });
                    }

                    // 其他退出码 = 真实崩溃
                    // v0.8.15 P0-2 修复：进程死亡时读取日志内容，提供可操作的诊断信息
                    let (log_hint, log_empty) = get_sidecar_log_dir()
                        .map(|d| {
                            let log_path = d.join("lrc-sidecar.log");
                            let content = std::fs::read_to_string(&log_path).unwrap_or_default();
                            let is_empty = content.trim().is_empty();
                            let hint = if is_empty {
                                format!(
                                    "，日志为空（{}\\lrc-sidecar.log），疑似运行时依赖缺失",
                                    d.display()
                                )
                            } else {
                                let last_lines: Vec<&str> = content.lines().rev().take(3).collect();
                                format!(
                                    "，日志末尾: {}（完整日志: {}\\lrc-sidecar.log）",
                                    last_lines.join(" | "),
                                    d.display()
                                )
                            };
                            (hint, is_empty)
                        })
                        .unwrap_or_else(|| (String::new(), false));
                    tracing::error!(
                        "v0.8.17：sidecar PID={pid} 在健康检查期间退出（exit code {}）{log_hint}",
                        exit_code
                    );
                    return Err(SidecarStartError::ProcessDied {
                        pid,
                        log_hint,
                        log_empty,
                    });
                }
                Ok(None) => {
                    // 进程仍在运行，继续健康检查
                }
                Err(e) => {
                    tracing::warn!("try_wait 失败 (pid: {:?}): {}", pid, e);
                }
            }

            // 端口自适应：从起始端口开始扫描
            for offset in 0..PORT_SCAN_RANGE {
                let port = start_port + offset;
                let health_url = format!("http://127.0.0.1:{port}/health");

                match client.get(&health_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if offset > 0 {
                            tracing::info!(
                                "Sidecar 端口自适应: {} → {} (第{attempt}次尝试)",
                                start_port,
                                port
                            );
                        }
                        return Ok(port);
                    }
                    Ok(resp) => {
                        tracing::debug!(
                            "Sidecar 健康检查 port={port} 第{attempt}/20次: HTTP {}",
                            resp.status()
                        );
                    }
                    Err(_) => {
                        // 连接被拒绝，继续尝试下一个端口
                    }
                }
            }

            tracing::debug!(
                "Sidecar 健康检查 第{attempt}/20次: 端口 {start_port}~{} 均不可用",
                start_port + PORT_SCAN_RANGE - 1
            );

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        tracing::error!("Sidecar 健康检查超时（20次/10秒），进程 PID={pid} 仍在运行但不可达");
        Err(SidecarStartError::HealthCheckTimeout {
            port: start_port,
            attempts: 20,
        })
    }

    /// v0.8.17 新增：扫描端口范围寻找健康的 sidecar 实例
    ///
    /// 从 start_port 开始扫描前 10 个端口（覆盖常见的端口自适应范围），
    /// 返回第一个健康 sidecar 的端口。
    ///
    /// 设计考量：
    ///   - 只扫描 10 个端口（而非全部 100 个），最坏耗时 2s（10×200ms）
    ///   - 200ms 超时确保端口未开放时快速失败
    ///   - 用于 SingletonConflict 场景：sidecar 因单例锁冲突退出后，
    ///     桌面端扫描端口寻找已运行的 sidecar 实例以复用
    pub async fn find_healthy_sidecar_port(start_port: u16) -> Option<u16> {
        for offset in 0..10u16 {
            let port = start_port + offset;
            // 用 200ms 超时包裹 check_sidecar_health（默认 2s 太慢）
            if let Ok(Some(_)) =
                tokio::time::timeout(Duration::from_millis(200), Self::check_sidecar_health(port))
                    .await
            {
                return Some(port);
            }
        }
        None
    }

    /// v0.5.16 新增：快速检查指定端口上的 sidecar 是否健康
    ///
    /// 仅检查单个端口（非扫描），适用于状态查询时的快速验证。
    /// 不需要 SidecarManager 实例，因此可以在不持有 sidecar 锁的情况下调用。
    ///
    /// 返回 Some(ProbedSidecar) 如果端口上有 loong-recall 服务在运行，
    /// 否则返回 None。
    pub async fn check_sidecar_health(port: u16) -> Option<ProbedSidecar> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .ok()?;

        let url = format!("http://127.0.0.1:{port}/health");
        let resp = client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let body: serde_json::Value = resp.json().await.ok()?;
        let service = body.get("service").and_then(|v| v.as_str()).unwrap_or("");
        if service != "loong-recall" {
            return None;
        }

        Some(ProbedSidecar {
            port,
            src_dir: body
                .get("src_dir")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            uptime_seconds: body
                .get("uptime_seconds")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        })
    }

    /// v0.5.16 新增：探测端口上已运行的 sidecar（非桌面端启动的）
    ///
    /// 关联函数，不需要 &self，因此可以在不持有 sidecar 锁的情况下调用。
    ///
    /// 应用场景：用户先打开 IDE（MCP 已连接 sidecar），再打开桌面端时，
    /// 桌面端的 instances HashMap 为空，但 sidecar 实际已在端口上运行。
    /// 此方法扫描 DEFAULT_SIDECAR_PORT..DEFAULT_SIDECAR_PORT+PORT_SCAN_RANGE
    /// 端口范围，向每个端口的 /health 端点发送 GET 请求，
    /// 返回所有健康检查通过且 service="loong-recall" 的 sidecar 实例信息。
    ///
    /// 重要：此函数会扫描 100 个端口，耗时约 500ms。
    /// 不要在持有 sidecar 锁的情况下调用，否则会阻塞其他需要 sidecar 锁的操作。
    /// 应在独立异步任务中调用。
    pub async fn probe_existing_sidecar() -> Vec<ProbedSidecar> {
        // 短超时：端口未开放时应快速失败，避免 100 个端口顺序等待
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("probe_existing_sidecar: 创建 HTTP 客户端失败: {e}");
                return Vec::new();
            }
        };

        let start_port = DEFAULT_SIDECAR_PORT;
        let end_port = DEFAULT_SIDECAR_PORT + PORT_SCAN_RANGE;

        tracing::info!(
            "开始探测外部 sidecar：扫描端口范围 {}-{}",
            start_port,
            end_port - 1
        );

        // 并发扫描所有端口
        let mut handles = Vec::with_capacity(PORT_SCAN_RANGE as usize);
        for port in start_port..end_port {
            let client = client.clone();
            handles.push(tokio::spawn(async move {
                let url = format!("http://127.0.0.1:{port}/health");
                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        // 解析响应体，验证是否为 loong-recall 服务
                        match resp.json::<serde_json::Value>().await {
                            Ok(body) => {
                                let service =
                                    body.get("service").and_then(|v| v.as_str()).unwrap_or("");
                                if service == "loong-recall" {
                                    let src_dir = body
                                        .get("src_dir")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let uptime_seconds = body
                                        .get("uptime_seconds")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0);
                                    Some(ProbedSidecar {
                                        port,
                                        src_dir,
                                        uptime_seconds,
                                    })
                                } else {
                                    // 端口上有其他服务，跳过
                                    None
                                }
                            }
                            Err(_) => None,
                        }
                    }
                    _ => None,
                }
            }));
        }

        // 收集所有探测结果
        let mut probed = Vec::new();
        for handle in handles {
            if let Ok(Some(result)) = handle.await {
                probed.push(result);
            }
        }

        if !probed.is_empty() {
            tracing::info!(
                "探测到 {} 个外部 sidecar 实例：{:?}",
                probed.len(),
                probed
                    .iter()
                    .map(|p| (p.port, &p.src_dir))
                    .collect::<Vec<_>>()
            );
        } else {
            tracing::info!("未探测到外部 sidecar 实例");
        }

        probed
    }

    /// 检查进程是否存活（跨平台，静态方法）
    ///
    /// Windows: 通过子进程句柄的 try_wait 检查
    /// Unix: 发送信号 0 检查进程是否存在
    fn is_process_alive(child: &mut Child) -> bool {
        #[cfg(target_os = "windows")]
        {
            match child.try_wait() {
                Ok(None) => true,     // 进程仍在运行
                Ok(Some(_)) => false, // 进程已退出
                Err(_) => true,       // try_wait 失败时保守假设仍在运行
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            // SAFETY: kill(pid, 0) 是 POSIX 标准调用，信号 0 仅检查进程是否存在，不发送实际信号
            unsafe { libc::kill(child.id() as i32, 0) == 0 }
        }
    }

    /// 启动 sidecar 进程（兼容旧接口，使用默认项目标识）
    /// 返回实际使用的端口号
    pub async fn start(
        &mut self,
        src_dir: Option<String>,
        port: Option<u16>,
        multi_window: Option<u32>,
        llm_api: Option<String>,
        cancel_flag: &AtomicBool,
        progress_tx: Option<&tokio::sync::mpsc::Sender<StartProgress>>,
    ) -> Result<u16, String> {
        // 使用项目路径作为默认标识，若未指定则使用 "default"
        let project_key = src_dir.clone().unwrap_or_else(|| "default".to_string());
        // 构造启动参数集合（v0.8.9：统一使用 StartOptions）
        let opts = StartOptions {
            src_dir: src_dir.as_deref(),
            port,
            multi_window,
            llm_api: llm_api.as_deref(),
            cancel_flag,
            progress_tx,
            data_dir: None,
        };
        self.start_for_project(&project_key, &opts).await
    }

    /// 为指定项目启动 sidecar 进程（内部使用三阶段方法）
    ///
    /// 每个项目可以有独立的 sidecar，绑定不同端口。
    /// 如果该项目的 sidecar 已在运行，直接返回端口。
    /// 返回实际使用的端口号。
    ///
    /// **注意**：此方法内部顺序调用 Phase 1→2→3，虽然 Phase 2 使用关联函数，
    /// 但调用方仍持有 `&mut self`（即 sidecar 锁），因此在 Phase 2 期间锁不会被释放。
    /// 如需避免锁竞争，调用方应直接使用三阶段编排：
    ///   1. `prepare_start()`（持锁）→ 释放锁
    ///   2. `SidecarManager::spawn_and_wait()`（不持锁）
    ///   3. `insert_handle()`（重新获取锁）
    pub async fn start_for_project(
        &mut self,
        project_key: &str,
        opts: &StartOptions<'_>,
    ) -> Result<u16, String> {
        // Phase 1: 检查是否已运行（无 I/O）
        match self.prepare_start(project_key) {
            PrepareResult::AlreadyRunning(port) => return Ok(port),
            PrepareResult::NeedStart => {}
        }

        // Phase 2: 启动子进程 + 健康检查（I/O，关联函数）
        let (child, port) = Self::spawn_and_wait(&self.binary_path, project_key, opts).await?;

        // Phase 3: 插入实例（无 I/O）
        // insert_handle 需要 Option<String>（拥有），从 &str 转换
        self.insert_handle(
            project_key,
            child,
            port,
            opts.src_dir.map(|s| s.to_string()),
            opts.multi_window,
            opts.llm_api.map(|s| s.to_string()),
        );

        Ok(port)
    }

    /// 停止 sidecar 进程（兼容旧接口，停止所有实例）
    pub async fn stop(&mut self) -> Result<(), String> {
        self.stop_all().await
    }

    /// 停止所有项目的 sidecar 进程
    pub async fn stop_all(&mut self) -> Result<(), String> {
        let project_keys: Vec<String> = self.instances.keys().cloned().collect();
        for key in project_keys {
            let _ = self.stop_project(&key).await;
        }
        Ok(())
    }

    /// 停止指定项目的 sidecar 进程
    pub async fn stop_project(&mut self, project_key: &str) -> Result<(), String> {
        if let Some(mut handle) = self.instances.remove(project_key) {
            let pid = handle.child.id();
            // 先尝试优雅终止
            let _ = handle.child.kill();
            // 等待进程退出（最多 5 秒）
            let wait_result = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::task::spawn_blocking(move || handle.child.wait()),
            )
            .await
            .map_err(|_| format!("等待 sidecar 退出超时 (项目: {project_key})"))?
            .map_err(|e| format!("等待 sidecar 退出失败: {e}"))?;

            tracing::info!(
                "Sidecar 已停止: 项目={}, PID={}: {:?}",
                project_key,
                pid,
                wait_result
            );
        }
        Ok(())
    }

    /// v0.5.1 新增：崩溃恢复 — 检测并自动重启已死亡的 sidecar 实例
    ///
    /// 遍历所有运行中的实例，检查进程是否存活。
    /// 如果进程已死亡，自动使用保存的启动参数重新启动。
    /// 返回恢复的实例数量。
    ///
    /// **v0.5.17 警告**：此方法内部使用 `collect_dead_instances` + `spawn_and_wait` +
    /// `insert_handle` 三阶段方法，但调用方仍持有 `&mut self`（即 sidecar 锁），
    /// 因此 Phase 2 期间锁不会被释放。
    ///
    /// **如需避免锁竞争**，调用方应直接使用三阶段编排：
    /// ```ignore
    /// // Phase 1: 收集死亡实例（持锁，无 I/O）
    /// let dead = {
    ///     let mut sidecar = store.sidecar.lock().await;
    ///     sidecar.collect_dead_instances()
    /// }; // 锁释放
    ///
    /// // Phase 2: 逐个重启（不持锁，I/O）
    /// let binary_path = ...;
    /// let mut recovered = Vec::new();
    /// for info in dead {
    ///     if let Ok((child, port)) = SidecarManager::spawn_and_wait(...).await {
    ///         recovered.push((info.project_key, child, port, ...));
    ///     }
    /// }
    ///
    /// // Phase 3: 插入恢复的实例（重新获取锁，无 I/O）
    /// if !recovered.is_empty() {
    ///     let mut sidecar = store.sidecar.lock().await;
    ///     for (key, child, port, ...) in recovered {
    ///         sidecar.insert_handle(&key, child, port, ...);
    ///     }
    /// }
    /// ```
    pub async fn recover_dead_instances(
        &mut self,
        fresh_llm_api: Option<String>,
        cancel_flag: &AtomicBool,
    ) -> usize {
        // Phase 1: 收集死亡实例（无 I/O）
        let dead_instances = self.collect_dead_instances();
        if dead_instances.is_empty() {
            return 0;
        }

        let mut recovered = 0usize;
        for info in dead_instances {
            let DeadInstanceInfo {
                project_key,
                src_dir,
                multi_window,
                llm_api,
            } = info;

            // v0.5.4 修复：优先使用传入的最新 LLM 配置
            let effective_llm = fresh_llm_api.clone().or(llm_api);

            // Phase 2: 启动子进程 + 健康检查（I/O）
            let start_opts = StartOptions {
                src_dir: src_dir.as_deref(),
                port: None,
                multi_window,
                llm_api: effective_llm.as_deref(),
                cancel_flag,
                progress_tx: None, // G-003：心跳恢复不需要进度反馈
                data_dir: None,
            };
            match Self::spawn_and_wait(&self.binary_path, &project_key, &start_opts).await {
                Ok((child, port)) => {
                    // Phase 3: 插入实例（无 I/O）
                    self.insert_handle(
                        &project_key,
                        child,
                        port,
                        src_dir,
                        multi_window,
                        effective_llm,
                    );
                    recovered += 1;
                    tracing::info!(
                        "Sidecar 崩溃恢复成功: 项目={}, 新端口={}",
                        project_key,
                        port
                    );
                }
                Err(e) => {
                    tracing::error!("Sidecar 崩溃恢复失败: 项目={}, 错误: {}", project_key, e);
                }
            }
        }

        if recovered > 0 {
            tracing::info!("崩溃恢复完成，共恢复 {} 个实例", recovered);
        }
        recovered
    }

    /// 重启指定项目的 sidecar
    pub async fn restart_project(
        &mut self,
        project_key: &str,
        opts: &StartOptions<'_>,
    ) -> Result<u16, String> {
        self.stop_project(project_key).await?;
        self.start_for_project(project_key, opts).await
    }

    // v0.5.17: 旧的 wait_for_health(&self, ...) 已被 wait_for_health_static 替代。
    // 新方法是关联函数，不需要 &self，可在不持有 sidecar 锁的情况下调用。
    // 这避免了在持有锁时执行 40 秒健康检查的锁竞争问题。
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;

    /// TDD：测试初始状态无运行实例
    #[test]
    fn test_initial_state_is_stopped() {
        let manager = SidecarManager::for_testing("test-server.exe".into());
        assert!(!manager.is_running());
        assert!(manager.list_instances().is_empty());
    }

    /// TDD：测试多实例管理
    #[test]
    fn test_multiple_instances() {
        let manager = SidecarManager::for_testing("test-server.exe".into());
        // 初始状态无实例
        assert!(!manager.is_project_running("project_a"));
        assert!(!manager.is_project_running("project_b"));
        assert_eq!(manager.list_instances().len(), 0);
    }

    // ════════════════════════════════════════════════════════════════
    // v0.5.17 并发压力测试
    //
    // 目标：验证三阶段锁安全模式的核心特性：
    //   1. Phase 1 (prepare_start/collect_dead_instances) 不执行 I/O，锁持有 < 1ms
    //   2. Phase 2 (spawn_and_wait) 是关联函数，不持有 sidecar 锁
    //   3. Phase 2 期间，其他任务可以获取 sidecar 锁
    //   4. 并发调用不会死锁
    // ════════════════════════════════════════════════════════════════

    /// 测试 prepare_start 对空管理器返回 NeedStart
    #[test]
    fn test_prepare_start_returns_need_start_for_empty() {
        let mut manager = SidecarManager::for_testing("nonexistent.exe".into());
        let result = manager.prepare_start("project_a");
        assert!(matches!(result, PrepareResult::NeedStart));
    }

    /// 测试 prepare_start 对已运行项目返回 AlreadyRunning
    #[test]
    fn test_prepare_start_returns_already_running() {
        let mut manager = SidecarManager::for_testing("nonexistent.exe".into());
        // 启动一个真实子进程（会立即退出）用于测试
        #[cfg(target_os = "windows")]
        let child = Command::new("cmd")
            .args(["/c", "timeout", "10"])
            .spawn()
            .unwrap();
        #[cfg(not(target_os = "windows"))]
        let child = Command::new("sleep").arg("10").spawn().unwrap();

        let pid = child.id();
        manager.insert_handle(
            "test_project",
            child,
            3099,
            Some("/test/dir".into()),
            None,
            None,
        );

        // prepare_start 应检测到进程存活，返回 AlreadyRunning
        let result = manager.prepare_start("test_project");
        match result {
            PrepareResult::AlreadyRunning(port) => {
                assert_eq!(port, 3099);
            }
            PrepareResult::NeedStart => panic!("进程 PID={} 应该存活", pid),
        }

        // 清理：kill 子进程
        if let Some(mut handle) = manager.instances.remove("test_project") {
            let _ = handle.child.kill();
            let _ = handle.child.wait();
        }
    }

    /// 并发压力测试：10 个 prepare_start 调用应在 < 100ms 内完成
    ///
    /// 验证 Phase 1 不执行 I/O，锁持有时间极短。
    #[tokio::test]
    async fn test_concurrent_prepare_start_no_blocking() {
        let manager = Arc::new(tokio::sync::Mutex::new(SidecarManager::for_testing(
            "nonexistent.exe".into(),
        )));

        let start = Instant::now();
        let mut handles = Vec::new();

        for i in 0..10 {
            let mgr = manager.clone();
            handles.push(tokio::spawn(async move {
                let key = format!("project_{}", i);
                let mut m = mgr.lock().await;
                m.prepare_start(&key)
            }));
        }

        for handle in handles {
            let _ = handle.await;
        }

        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "10 个 prepare_start 调用耗时 {:?}，预期 < 100ms（每个应 < 1ms）",
            elapsed
        );
    }

    /// 并发压力测试：10 个 collect_dead_instances 调用应在 < 100ms 内完成
    #[tokio::test]
    async fn test_concurrent_collect_dead_instances_no_blocking() {
        let manager = Arc::new(tokio::sync::Mutex::new(SidecarManager::for_testing(
            "nonexistent.exe".into(),
        )));

        let start = Instant::now();
        let mut handles = Vec::new();

        for _ in 0..10 {
            let mgr = manager.clone();
            handles.push(tokio::spawn(async move {
                let mut m = mgr.lock().await;
                m.collect_dead_instances()
            }));
        }

        for handle in handles {
            let _ = handle.await;
        }

        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "10 个 collect_dead_instances 调用耗时 {:?}，预期 < 100ms",
            elapsed
        );
    }

    /// 核心测试：验证三阶段锁安全模式 — Phase 2 期间锁可被获取
    ///
    /// 模拟三阶段编排：
    ///   Phase 1: prepare_start（持锁）→ 释放锁
    ///   Phase 2: 模拟长时间 I/O（不持锁）
    ///   Phase 3: insert_handle（重新获取锁）
    ///
    /// 在 Phase 2 期间，另一个任务应该能立即获取 sidecar 锁。
    #[tokio::test]
    async fn test_three_phase_lock_safety_phase2_releases_lock() {
        let manager = Arc::new(tokio::sync::Mutex::new(SidecarManager::for_testing(
            "nonexistent.exe".into(),
        )));

        // Phase 1: prepare_start（持锁，< 1ms）
        let prepare = {
            let mut m = manager.lock().await;
            m.prepare_start("test_project")
        }; // 锁释放
        assert!(matches!(prepare, PrepareResult::NeedStart));

        // 启动监控任务：在 Phase 2 期间尝试获取锁
        let mgr_clone = manager.clone();
        let lock_monitor = tokio::spawn(async move {
            // 等待 50ms 确保主任务已进入 Phase 2
            tokio::time::sleep(Duration::from_millis(50)).await;
            // 尝试获取锁 — 应该能立即获取，因为 Phase 2 不持有锁
            let acquire_start = Instant::now();
            let _m = mgr_clone.lock().await;
            acquire_start.elapsed()
        });

        // Phase 2: 模拟长时间 I/O（200ms，不持锁）
        // 在真实场景中，这是 spawn_and_wait 调用（最多 40 秒）
        tokio::time::sleep(Duration::from_millis(200)).await;

        // 获取监控任务的锁获取时间
        let lock_acquire_time = lock_monitor.await.unwrap();
        assert!(
            lock_acquire_time.as_millis() < 50,
            "Phase 2 期间锁获取耗时 {:?}，预期 < 50ms（说明 Phase 2 不持有锁）",
            lock_acquire_time
        );

        // Phase 3: 重新获取锁（应该能立即获取，因为监控任务已释放）
        let _m = manager.lock().await;
    }

    /// 对比测试：验证旧模式（持有锁时执行 I/O）会被检测到
    ///
    /// 这个测试模拟 v0.5.15 的错误模式：在持有锁时执行长时间 I/O。
    /// 用于验证测试框架能检测到锁竞争问题。
    #[tokio::test]
    async fn test_old_pattern_holding_lock_during_io_is_detectable() {
        let manager = Arc::new(tokio::sync::Mutex::new(SidecarManager::for_testing(
            "nonexistent.exe".into(),
        )));

        // 启动监控任务：在主任务持有锁期间尝试获取锁
        let mgr_clone = manager.clone();
        let lock_monitor = tokio::spawn(async move {
            // 等待 50ms 确保主任务已获取锁
            tokio::time::sleep(Duration::from_millis(50)).await;
            // 尝试获取锁 — 应该被阻塞，因为主任务持有锁
            let acquire_start = Instant::now();
            let _m = mgr_clone.lock().await;
            acquire_start.elapsed()
        });

        // 模拟旧模式：持有锁时执行长时间 I/O（200ms）
        {
            let _m = manager.lock().await;
            // 在持有锁时执行 I/O（模拟 v0.5.15 的 wait_for_health）
            tokio::time::sleep(Duration::from_millis(200)).await;
        } // 锁释放

        // 获取监控任务的锁获取时间
        let lock_acquire_time = lock_monitor.await.unwrap();
        // 旧模式下，锁获取时间应该 > 100ms（因为被阻塞了 ~150ms）
        assert!(
            lock_acquire_time.as_millis() >= 100,
            "旧模式下锁获取耗时 {:?}，预期 >= 100ms（说明锁被持有期间阻塞了其他任务）",
            lock_acquire_time
        );
    }

    /// 测试 spawn_and_wait 在 binary 不存在时快速失败
    ///
    /// 验证 Phase 2 不会因为 binary 不存在而长时间阻塞。
    #[tokio::test]
    async fn test_spawn_and_wait_fails_fast_on_missing_binary() {
        let start = Instant::now();
        let cancel_flag = AtomicBool::new(false);
        let opts = StartOptions {
            src_dir: None,
            port: None,
            multi_window: None,
            llm_api: None,
            cancel_flag: &cancel_flag,
            progress_tx: None, // G-003：测试不需要进度反馈
            data_dir: None,
        };
        let result =
            SidecarManager::spawn_and_wait("nonexistent-binary-xyz.exe", "test_project", &opts)
                .await;

        let elapsed = start.elapsed();

        assert!(result.is_err(), "binary 不存在时应返回错误");
        // v0.8.9 G-002：spawn 前有 200ms 端口预检，放宽到 500ms
        assert!(
            elapsed.as_millis() < 500,
            "spawn_and_wait 在 binary 不存在时耗时 {:?}，预期 < 500ms（含 G-002 端口预检）",
            elapsed
        );
    }

    /// 并发压力测试：模拟 get_sidecar_status + start_sidecar + get_wizard_state 并发
    ///
    /// 模拟 3 种命令同时调用 sidecar 锁的场景：
    ///   - get_sidecar_status: 短暂持锁获取 list_instances
    ///   - start_sidecar: Phase 1 短暂持锁 → Phase 2 不持锁 → Phase 3 短暂持锁
    ///   - get_wizard_state: 短暂持锁检查 is_running
    ///
    /// 所有命令应在 < 500ms 内完成（因为 Phase 2 不持锁）。
    #[tokio::test]
    async fn test_concurrent_status_start_wizard_no_blocking() {
        let manager = Arc::new(tokio::sync::Mutex::new(SidecarManager::for_testing(
            "nonexistent.exe".into(),
        )));
        let sidecar_port = Arc::new(tokio::sync::Mutex::new(None::<u16>));

        let start = Instant::now();
        let mut handles = Vec::new();

        // 模拟 5 轮并发调用
        for round in 0..5 {
            let mgr1 = manager.clone();
            let mgr2 = manager.clone();
            let mgr3 = manager.clone();
            let _port_clone = sidecar_port.clone();

            // 模拟 get_sidecar_status: 短暂持锁获取 list_instances
            handles.push(tokio::spawn(async move {
                let m = mgr1.lock().await;
                let _ = m.list_instances();
            }));

            // 模拟 get_wizard_state: 短暂持锁检查 is_running
            handles.push(tokio::spawn(async move {
                let m = mgr3.lock().await;
                let _ = m.is_running();
            }));

            // 模拟 start_sidecar: Phase 1 → Phase 2（模拟）→ Phase 3
            // 由于 binary 不存在，Phase 2 会快速失败
            handles.push(tokio::spawn(async move {
                let project_key = format!("project_{}", round);
                // Phase 1
                let _prepare = {
                    let mut m = mgr2.lock().await;
                    m.prepare_start(&project_key)
                };
                // Phase 2: spawn_and_wait 会快速失败（binary 不存在）
                let cancel_flag = AtomicBool::new(false);
                let opts = StartOptions {
                    src_dir: None,
                    port: None,
                    multi_window: None,
                    llm_api: None,
                    cancel_flag: &cancel_flag,
                    progress_tx: None, // G-003：测试不需要进度反馈
                    data_dir: None,
                };
                let _ =
                    SidecarManager::spawn_and_wait("nonexistent.exe", &project_key, &opts).await;
                // 不执行 Phase 3（因为 Phase 2 失败了）
            }));
        }

        for handle in handles {
            let _ = handle.await;
        }

        let elapsed = start.elapsed();
        // v0.8.9 G-002：每个 spawn_and_wait 含 200ms 端口预检，5 个并发约 200-400ms
        assert!(
            elapsed.as_millis() < 1000,
            "15 个并发调用（5 轮 × 3 命令）耗时 {:?}，预期 < 1000ms（含 G-002 端口预检）",
            elapsed
        );
    }

    /// 测试 collect_dead_instances 返回正确的死亡实例信息
    #[test]
    fn test_collect_dead_instances_returns_dead_info() {
        let mut manager = SidecarManager::for_testing("nonexistent.exe".into());

        // 启动一个会立即退出的子进程
        #[cfg(target_os = "windows")]
        let child = Command::new("cmd").args(["/c", "exit"]).spawn().unwrap();
        #[cfg(not(target_os = "windows"))]
        let child = Command::new("true").spawn().unwrap();

        manager.insert_handle(
            "dead_project",
            child,
            3099,
            Some("/test/dir".into()),
            Some(5),
            Some("openai||key".into()),
        );

        // 等待子进程退出
        std::thread::sleep(Duration::from_millis(100));

        // collect_dead_instances 应检测到死亡实例
        let dead = manager.collect_dead_instances();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].project_key, "dead_project");
        assert_eq!(dead[0].src_dir, Some("/test/dir".into()));
        assert_eq!(dead[0].multi_window, Some(5));
        assert_eq!(dead[0].llm_api, Some("openai||key".into()));

        // 确认实例已被移除
        assert!(!manager.is_project_running("dead_project"));
    }

    /// 测试 insert_handle 后实例可被查询
    #[test]
    fn test_insert_handle_makes_instance_queryable() {
        let mut manager = SidecarManager::for_testing("nonexistent.exe".into());

        #[cfg(target_os = "windows")]
        let child = Command::new("cmd")
            .args(["/c", "timeout", "10"])
            .spawn()
            .unwrap();
        #[cfg(not(target_os = "windows"))]
        let child = Command::new("sleep").arg("10").spawn().unwrap();

        manager.insert_handle(
            "test_project",
            child,
            3099,
            Some("/test/dir".into()),
            None,
            None,
        );

        assert!(manager.is_project_running("test_project"));
        assert!(manager.is_running());

        let inst = manager.get_instance("test_project").unwrap();
        assert_eq!(inst.port, 3099);
        assert_eq!(inst.project_dir, "/test/dir");

        // 清理
        if let Some(mut handle) = manager.instances.remove("test_project") {
            let _ = handle.child.kill();
            let _ = handle.child.wait();
        }
    }

    /// 测试 binary_path 访问器
    #[test]
    fn test_binary_path_accessor() {
        // SidecarManager::new 在路径不存在时会自动搜索，因此只验证返回非空
        let manager = SidecarManager::for_testing("nonexistent.exe".into());
        assert!(!manager.binary_path().is_empty(), "binary_path 不应为空");
    }

    // ════════════════════════════════════════════════════════════════
    // 启动链路端到端测试
    //
    // 模拟 main.rs setup 回调和心跳协程的完整启动序列，
    // 验证整个启动过程中锁不被长时间持有。
    // ════════════════════════════════════════════════════════════════

    /// 启动链路 E2E 测试：模拟 main.rs setup 回调的启动序列
    ///
    /// 验证以下步骤的锁安全性：
    /// 1. 检查 sidecar 是否运行（短暂持锁，<1ms）
    /// 2. 探测端口上的外部 sidecar（不持锁，~500ms）
    /// 3. 如果探测到，更新 sidecar_port（短暂持锁，<1ms）
    /// 4. 在整个过程中，其他任务可以获取 sidecar 锁
    #[tokio::test]
    async fn test_startup_sequence_no_lock_blocking() {
        let manager = Arc::new(tokio::sync::Mutex::new(SidecarManager::for_testing(
            "nonexistent.exe".into(),
        )));
        let sidecar_port = Arc::new(tokio::sync::Mutex::new(None::<u16>));

        // 启动监控任务：在启动序列期间持续尝试获取锁
        let mgr_monitor = manager.clone();
        let lock_monitor = tokio::spawn(async move {
            let mut max_acquire_time = Duration::from_millis(0);
            for _ in 0..10 {
                let acquire_start = Instant::now();
                let _m = mgr_monitor.lock().await;
                let elapsed = acquire_start.elapsed();
                if elapsed > max_acquire_time {
                    max_acquire_time = elapsed;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            max_acquire_time
        });

        // Step 1: 检查 sidecar 是否运行（短暂持锁）
        let sidecar_running = {
            let sidecar = manager.lock().await;
            sidecar.is_running()
        }; // 锁释放
        assert!(!sidecar_running);

        // Step 2: 探测端口上的外部 sidecar（不持锁）
        // 这会扫描 100 个端口，但在测试环境中应该快速返回空
        let _probed = SidecarManager::probe_existing_sidecar().await;

        // Step 3: sidecar_port 更新（如果有探测到的话）
        // 在测试环境中，probe 返回空，所以这步跳过
        {
            let _port = sidecar_port.lock().await;
        }

        // 等待监控任务完成
        let max_lock_acquire = lock_monitor.await.unwrap();

        // 验证：启动序列期间，锁获取时间不应超过 50ms
        // （因为所有锁持有都应该是短暂的，probe 不持锁）
        assert!(
            max_lock_acquire.as_millis() < 50,
            "启动序列期间最大锁获取耗时 {:?}，预期 < 50ms",
            max_lock_acquire
        );
    }

    /// 启动链路 E2E 测试：模拟心跳协程的崩溃恢复序列
    ///
    /// 验证以下步骤的锁安全性：
    /// 1. 心跳检测：获取锁 → list_instances → 释放锁（<1ms）
    /// 2. Phase 1: collect_dead_instances（持锁，<1ms）→ 释放锁
    /// 3. Phase 2: spawn_and_wait（不持锁，快速失败因为 binary 不存在）
    /// 4. Phase 3: insert_handle（重新获取锁，<1ms）
    /// 5. 整个恢复过程中，其他任务可以获取 sidecar 锁
    #[tokio::test]
    async fn test_heartbeat_recovery_sequence_no_lock_blocking() {
        let manager = Arc::new(tokio::sync::Mutex::new(SidecarManager::for_testing(
            "nonexistent.exe".into(),
        )));

        // 启动监控任务：在恢复序列期间持续尝试获取锁
        let mgr_monitor = manager.clone();
        let lock_monitor = tokio::spawn(async move {
            let mut max_acquire_time = Duration::from_millis(0);
            for _ in 0..10 {
                let acquire_start = Instant::now();
                let _m = mgr_monitor.lock().await;
                let elapsed = acquire_start.elapsed();
                if elapsed > max_acquire_time {
                    max_acquire_time = elapsed;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            max_acquire_time
        });

        // Step 1: 心跳检测（短暂持锁）
        let current_count = {
            let sidecar = manager.lock().await;
            sidecar.list_instances().len()
        }; // 锁释放
        assert_eq!(current_count, 0);

        // Step 2: Phase 1 — 收集死亡实例（持锁，<1ms）
        let (dead_instances, binary_path) = {
            let mut sidecar = manager.lock().await;
            let dead = sidecar.collect_dead_instances();
            let binary = sidecar.binary_path().to_string();
            (dead, binary)
        }; // 锁释放

        // Step 3: Phase 2 — 逐个重启（不持锁）
        let heartbeat_cancel = AtomicBool::new(false);
        for info in dead_instances {
            let opts = StartOptions {
                src_dir: info.src_dir.as_deref(),
                port: None,
                multi_window: info.multi_window,
                llm_api: info.llm_api.as_deref(),
                cancel_flag: &heartbeat_cancel,
                progress_tx: None, // G-003：测试不需要进度反馈
                data_dir: None,
            };
            let _ = SidecarManager::spawn_and_wait(&binary_path, &info.project_key, &opts).await;
            // spawn_and_wait 会快速失败（binary 不存在）
        }

        // Step 4: Phase 3 — 插入恢复的实例（如果有）
        // 在测试环境中，Phase 2 全部失败，所以没有实例需要插入

        // 等待监控任务完成
        let max_lock_acquire = lock_monitor.await.unwrap();

        // 验证：恢复序列期间，锁获取时间不应超过 50ms
        assert!(
            max_lock_acquire.as_millis() < 50,
            "恢复序列期间最大锁获取耗时 {:?}，预期 < 50ms",
            max_lock_acquire
        );
    }

    /// 启动链路 E2E 测试：模拟 start_sidecar 命令的完整三阶段编排
    ///
    /// 验证以下步骤的锁安全性：
    /// 1. Phase 1: prepare_start（持锁，<1ms）→ 释放锁
    /// 2. Phase 2: spawn_and_wait（不持锁，快速失败）
    /// 3. Phase 3: insert_handle（重新获取锁，<1ms）
    /// 4. 整个过程中，其他任务可以获取 sidecar 锁
    #[tokio::test]
    async fn test_start_sidecar_three_phase_no_lock_blocking() {
        let manager = Arc::new(tokio::sync::Mutex::new(SidecarManager::for_testing(
            "nonexistent.exe".into(),
        )));

        // 启动监控任务
        let mgr_monitor = manager.clone();
        let lock_monitor = tokio::spawn(async move {
            let mut max_acquire_time = Duration::from_millis(0);
            for _ in 0..10 {
                let acquire_start = Instant::now();
                let _m = mgr_monitor.lock().await;
                let elapsed = acquire_start.elapsed();
                if elapsed > max_acquire_time {
                    max_acquire_time = elapsed;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            max_acquire_time
        });

        // Phase 1: prepare_start（持锁）
        let prepare = {
            let mut sidecar = manager.lock().await;
            sidecar.prepare_start("test_project")
        }; // 锁释放
        assert!(matches!(prepare, PrepareResult::NeedStart));

        // Phase 2: spawn_and_wait（不持锁，会快速失败）
        let binary_path = {
            let sidecar = manager.lock().await;
            sidecar.binary_path().to_string()
        };
        let cancel_flag = AtomicBool::new(false);
        let opts = StartOptions {
            src_dir: None,
            port: None,
            multi_window: None,
            llm_api: None,
            cancel_flag: &cancel_flag,
            progress_tx: None, // G-003：测试不需要进度反馈
            data_dir: None,
        };
        let _ = SidecarManager::spawn_and_wait(&binary_path, "test_project", &opts).await;

        // Phase 3: 不执行（Phase 2 失败了）

        // 等待监控任务完成
        let max_lock_acquire = lock_monitor.await.unwrap();

        assert!(
            max_lock_acquire.as_millis() < 50,
            "start_sidecar 三阶段编排期间最大锁获取耗时 {:?}，预期 < 50ms",
            max_lock_acquire
        );
    }
}
