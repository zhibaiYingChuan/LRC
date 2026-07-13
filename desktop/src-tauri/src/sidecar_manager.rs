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
const DEFAULT_SIDECAR_PORT: u16 = 3099;
/// 端口扫描范围：实际端口 = 起始端口 + 0..PORT_SCAN_RANGE
/// 与 server.rs 中 find_available_port 的 scan_range(100) 保持一致
const PORT_SCAN_RANGE: u16 = 100;
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
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
        Some(std::path::PathBuf::from(appdata).join("LoongRecall").join("logs"))
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
            tracing::info!("SidecarManager 释放，kill 子进程 project={}, PID={}", project_dir, pid);
            // 尝试优雅终止
            let mut child = handle.child;
            let _ = child.kill();
            // M-12 修复：kill 后必须 wait 回收子进程，否则产生僵尸进程
            // 使用 try_wait 轮询 + 短超时（3 秒），避免 Drop 中无限阻塞
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                match child.try_wait() {
                    Ok(Some(_status)) => {
                        tracing::debug!(
                            "子进程 PID={} 已退出，僵尸进程已回收",
                            pid
                        );
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
                        tracing::warn!(
                            "等待子进程 PID={} 退出失败: {}",
                            pid, e
                        );
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
            exe_dir.join(&binary_name),                               // 同目录（Windows 安装目录根）
            exe_dir.join("resources").join(&binary_name),            // resources/ 子目录
            exe_dir.parent().unwrap_or(&exe_dir).join(&binary_name), // 上级目录
            exe_dir.parent().unwrap_or(&exe_dir).join("Resources").join(&binary_name), // macOS: Contents/Resources/
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
        self.instances.get(project_dir).map(|handle| SidecarInstance {
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
                                let service = body.get("service").and_then(|v| v.as_str()).unwrap_or("");
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
                probed.iter().map(|p| (p.port, &p.src_dir)).collect::<Vec<_>>()
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
                Ok(None) => true,      // 进程仍在运行
                Ok(Some(_)) => false,  // 进程已退出
                Err(_) => true,        // try_wait 失败时保守假设仍在运行
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
    ) -> Result<u16, String> {
        // 使用项目路径作为默认标识，若未指定则使用 "default"
        let project_key = src_dir.clone().unwrap_or_else(|| "default".to_string());
        self.start_for_project(&project_key, src_dir, port, multi_window, llm_api).await
    }

    /// 为指定项目启动 sidecar 进程
    /// 
    /// 每个项目可以有独立的 sidecar，绑定不同端口。
    /// 如果该项目的 sidecar 已在运行，直接返回端口。
    /// 返回实际使用的端口号。
    pub async fn start_for_project(
        &mut self,
        project_key: &str,
        src_dir: Option<String>,
        port: Option<u16>,
        multi_window: Option<u32>,
        llm_api: Option<String>,
    ) -> Result<u16, String> {
        // 如果该项目的 sidecar 已在运行，检查进程是否存活
        if let Some(handle) = self.instances.get_mut(project_key) {
            if Self::is_process_alive(&mut handle.child) {
                tracing::info!(
                    "项目 {} 的 sidecar 已在运行 (PID={}, port={})",
                    project_key, handle.child.id(), handle.port
                );
                return Ok(handle.port);
            }
            // 进程已死，清理
            tracing::warn!("项目 {} 的 sidecar 已退出，重新启动", project_key);
            self.instances.remove(project_key);
        }

        // 使用 sensible 默认端口（3099），而非 0
        let actual_port = port.unwrap_or(DEFAULT_SIDECAR_PORT);

        // 构建启动参数
        let mut cmd = Command::new(&self.binary_path);
        // Windows: 隐藏 sidecar 进程的 CMD 窗口
        // 使用 CREATE_NO_WINDOW 确保进程完全静默，不弹出任何控制台
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);
        // 守护模式：不自动打开浏览器（桌面端自行管理 UI）
        cmd.args(["--daemon", "--port", &actual_port.to_string()]);

        if let Some(ref dir) = src_dir {
            cmd.args(["--src-dir", dir]);
        }

        // 多窗口模式
        if let Some(mw) = multi_window {
            cmd.args(["--multi-window", &mw.to_string()]);
            tracing::info!("多窗口模式：{} 个 LRC 实例上限", mw);
        }

        // v0.5.4 安全修复：使用环境变量传递 LLM API Key，而非命令行参数
        // 命令行参数会暴露在进程列表中，任何系统进程查看工具都能看到 API Key
        // 环境变量仅在当前进程及子进程中可见，无法被其他进程读取
        if let Some(ref llm) = llm_api {
            if !llm.is_empty() {
                cmd.env("LRC_LLM_API", llm);
                // M-5 修复：兼容 || 和 : 两种分隔符，提取 LLM 类型用于日志
                let llm_type = if llm.contains("||") {
                    llm.split("||").next()
                } else {
                    llm.split(':').next()
                }.unwrap_or("unknown");
                tracing::info!(
                    "已通过环境变量传递 LLM 配置到 Sidecar（项目: {}, 类型: {}）",
                    project_key,
                    llm_type
                );
            }
        }

        // v0.5.7 修复：将 sidecar stderr 重定向到日志文件，便于排查启动失败原因
        // 之前 stderr 完全丢弃，sidecar 启动失败时（如反调试误杀、端口绑定失败、
        // SingletonLock 冲突）无法获取错误信息，用户只看到笼统的"后台服务启动失败"
        if let Some(log_dir) = get_sidecar_log_dir() {
            // 确保日志目录存在
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

        // 启动子进程
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 sidecar 失败 (项目: {}): {e}", project_key))?;

        let pid = child.id();

        // 等待健康检查通过（最多 10 秒）
        let port = self.wait_for_health(&mut child, actual_port).await?;

        // 存储实例（保存启动参数用于崩溃恢复）
        let project_dir = src_dir.clone().unwrap_or_else(|| project_key.to_string());
        self.instances.insert(
            project_key.to_string(),
            SidecarHandle {
                child,
                port,
                project_dir: project_dir.clone(),
                src_dir: src_dir.clone(),
                multi_window,
                llm_api: llm_api.clone(),
            },
        );

        tracing::info!(
            "Sidecar 已启动: 项目={}, PID={}, port={}",
            project_key, pid, port
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
                project_key, pid, wait_result
            );
        }
        Ok(())
    }

    /// v0.5.1 新增：崩溃恢复 — 检测并自动重启已死亡的 sidecar 实例
    ///
    /// 遍历所有运行中的实例，检查进程是否存活。
    /// 如果进程已死亡，自动使用保存的启动参数重新启动。
    /// 返回恢复的实例数量。
    pub async fn recover_dead_instances(&mut self, fresh_llm_api: Option<String>) -> usize {
        let mut recovered = 0usize;
        // 先收集所有已死亡的实例 key（避免借用冲突）
        let dead_keys: Vec<String> = {
            let mut keys = Vec::new();
            for (key, handle) in self.instances.iter_mut() {
                if !Self::is_process_alive(&mut handle.child) {
                    keys.push(key.clone());
                }
            }
            keys
        };

        for key in dead_keys {
            // 获取保存的启动参数
            let (src_dir, multi_window, llm_api) = if let Some(handle) = self.instances.remove(&key) {
                tracing::warn!(
                    "检测到 sidecar 已死亡: 项目={}, 端口={}, 尝试自动恢复...",
                    key, handle.port
                );
                // v0.5.4 修复：优先使用传入的最新 LLM 配置，而非崩溃前保存的旧值
                // 用户可能在 sidecar 崩溃后更新了 LLM 配置，恢复时应使用最新值
                let effective_llm = fresh_llm_api.clone().or(handle.llm_api);
                (handle.src_dir, handle.multi_window, effective_llm)
            } else {
                continue;
            };

            // 尝试重新启动
            match self
                .start_for_project(&key, src_dir, None, multi_window, llm_api)
                .await
            {
                Ok(port) => {
                    recovered += 1;
                    tracing::info!(
                        "Sidecar 崩溃恢复成功: 项目={}, 新端口={}",
                        key, port
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Sidecar 崩溃恢复失败: 项目={}, 错误: {}",
                        key, e
                    );
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
        src_dir: Option<String>,
        port: Option<u16>,
        multi_window: Option<u32>,
        llm_api: Option<String>,
    ) -> Result<u16, String> {
        self.stop_project(project_key).await?;
        self.start_for_project(project_key, src_dir, port, multi_window, llm_api).await
    }

    /// 等待 sidecar 健康检查通过
    /// 
    /// 端口自适应扫描：sidecar 可能因端口冲突而绑定到不同端口，
    /// 因此从起始端口开始扫描 PORT_SCAN_RANGE 个端口，找到实际绑定的端口。
    /// 
    /// 每 500ms 检查一次，最多尝试 20 次（10 秒）
    async fn wait_for_health(&self, child: &mut Child, start_port: u16) -> Result<u16, String> {
        let pid = child.id();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

        for attempt in 1..=20 {
            // 检查进程是否还活着
            if !Self::is_process_alive(child) {
                // v0.5.7 修复：提示用户查看 sidecar 日志文件，获取真正的退出原因
                let log_hint = get_sidecar_log_dir()
                    .map(|d| format!("，请查看日志: {}\\lrc-sidecar.log", d.display()))
                    .unwrap_or_default();
                return Err(format!("Sidecar 进程 PID={pid} 启动后意外退出{log_hint}"));
            }

            // 端口自适应：从起始端口开始扫描（sidecar 可能已绑定到其他端口）
            for offset in 0..PORT_SCAN_RANGE {
                let port = start_port + offset;
                let health_url = format!("http://127.0.0.1:{port}/health");

                match client.get(&health_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        if offset > 0 {
                            tracing::info!(
                                "Sidecar 端口自适应: {} → {} (第{attempt}次尝试)",
                                start_port, port
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

        // 健康检查超时，返回错误而非假成功（修复 H08）
        // 用户需要知道 sidecar 未就绪的真实状态，而非被"假成功"误导
        tracing::error!("Sidecar 健康检查超时（20次/10秒），进程 PID={pid} 仍在运行但不可达");
        Err(format!(
            "Sidecar 健康检查超时：进程 PID={pid} 在端口 {}-{} 范围均不可达，已尝试 20 次（10 秒）。请检查端口是否被占用或防火墙设置。",
            start_port,
            start_port + PORT_SCAN_RANGE - 1
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD：测试初始状态无运行实例
    #[test]
    fn test_initial_state_is_stopped() {
        let manager = SidecarManager::new("test-server.exe".into());
        assert!(!manager.is_running());
        assert!(manager.list_instances().is_empty());
    }

    /// TDD：测试多实例管理
    #[test]
    fn test_multiple_instances() {
        let manager = SidecarManager::new("test-server.exe".into());
        // 初始状态无实例
        assert!(!manager.is_project_running("project_a"));
        assert!(!manager.is_project_running("project_b"));
        assert_eq!(manager.list_instances().len(), 0);
    }
}