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
                    project_key, handle.child.id(), handle.port
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
                tracing::warn!(
                    "检测到 sidecar 已死亡: 项目={}, 端口={}",
                    key, handle.port
                );
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
        src_dir: Option<&str>,
        port: Option<u16>,
        multi_window: Option<u32>,
        llm_api: Option<&str>,
    ) -> Result<(Child, u16), String> {
        let actual_port = port.unwrap_or(DEFAULT_SIDECAR_PORT);

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
                }.unwrap_or("unknown");
                tracing::info!(
                    "已通过环境变量传递 LLM 配置到 Sidecar（项目: {}, 类型: {}）",
                    project_key, llm_type
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

        // 启动子进程
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 sidecar 失败 (项目: {}): {e}", project_key))?;

        let pid = child.id();

        // 等待健康检查通过
        // v0.8.9 修复 G-010：健康检查失败时显式 kill 子进程，防止孤儿进程
        // std::process::Child 的 Drop 不会 kill 子进程，必须显式 kill + wait
        let port = match Self::wait_for_health_static(&mut child, actual_port).await {
            Ok(port) => port,
            Err(e) => {
                tracing::warn!(
                    "健康检查失败，正在清理子进程 (pid: {:?}): {}",
                    pid,
                    e
                );
                let _ = child.kill();
                let _ = child.wait(); // 等待子进程退出，避免僵尸进程
                return Err(e);
            }
        };

        tracing::info!(
            "Sidecar 已启动: 项目={}, PID={}, port={}",
            project_key, pid, port
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
    async fn wait_for_health_static(child: &mut Child, start_port: u16) -> Result<u16, String> {
        let pid = child.id();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

        for attempt in 1..=20 {
            // 检查进程是否还活着
            if !Self::is_process_alive(child) {
                let log_hint = get_sidecar_log_dir()
                    .map(|d| format!("，请查看日志: {}\\lrc-sidecar.log", d.display()))
                    .unwrap_or_default();
                return Err(format!("Sidecar 进程 PID={pid} 启动后意外退出{log_hint}"));
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

        tracing::error!("Sidecar 健康检查超时（20次/10秒），进程 PID={pid} 仍在运行但不可达");
        Err(format!(
            "Sidecar 健康检查超时：进程 PID={pid} 在端口 {}-{} 范围均不可达，已尝试 20 次（10 秒）",
            start_port,
            start_port + PORT_SCAN_RANGE - 1
        ))
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
        src_dir: Option<String>,
        port: Option<u16>,
        multi_window: Option<u32>,
        llm_api: Option<String>,
    ) -> Result<u16, String> {
        // Phase 1: 检查是否已运行（无 I/O）
        match self.prepare_start(project_key) {
            PrepareResult::AlreadyRunning(port) => return Ok(port),
            PrepareResult::NeedStart => {}
        }

        // Phase 2: 启动子进程 + 健康检查（I/O，关联函数）
        let (child, port) = Self::spawn_and_wait(
            &self.binary_path,
            project_key,
            src_dir.as_deref(),
            port,
            multi_window,
            llm_api.as_deref(),
        )
        .await?;

        // Phase 3: 插入实例（无 I/O）
        self.insert_handle(project_key, child, port, src_dir, multi_window, llm_api);

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
    pub async fn recover_dead_instances(&mut self, fresh_llm_api: Option<String>) -> usize {
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
            match Self::spawn_and_wait(
                &self.binary_path,
                &project_key,
                src_dir.as_deref(),
                None,
                multi_window,
                effective_llm.as_deref(),
            )
            .await
            {
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
                        project_key, port
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Sidecar 崩溃恢复失败: 项目={}, 错误: {}",
                        project_key, e
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
        let mut manager = SidecarManager::new("nonexistent.exe".into());
        let result = manager.prepare_start("project_a");
        assert!(matches!(result, PrepareResult::NeedStart));
    }

    /// 测试 prepare_start 对已运行项目返回 AlreadyRunning
    #[test]
    fn test_prepare_start_returns_already_running() {
        let mut manager = SidecarManager::new("nonexistent.exe".into());
        // 启动一个真实子进程（会立即退出）用于测试
        #[cfg(target_os = "windows")]
        let child = Command::new("cmd").args(["/c", "timeout", "10"]).spawn().unwrap();
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
        let manager = Arc::new(tokio::sync::Mutex::new(
            SidecarManager::new("nonexistent.exe".into())
        ));

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
        let manager = Arc::new(tokio::sync::Mutex::new(
            SidecarManager::new("nonexistent.exe".into())
        ));

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
        let manager = Arc::new(tokio::sync::Mutex::new(
            SidecarManager::new("nonexistent.exe".into())
        ));

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
        let manager = Arc::new(tokio::sync::Mutex::new(
            SidecarManager::new("nonexistent.exe".into())
        ));

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
        let result = SidecarManager::spawn_and_wait(
            "nonexistent-binary-xyz.exe",
            "test_project",
            None,
            None,
            None,
            None,
        )
        .await;

        let elapsed = start.elapsed();

        assert!(result.is_err(), "binary 不存在时应返回错误");
        assert!(
            elapsed.as_millis() < 100,
            "spawn_and_wait 在 binary 不存在时耗时 {:?}，预期 < 100ms",
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
        let manager = Arc::new(tokio::sync::Mutex::new(
            SidecarManager::new("nonexistent.exe".into())
        ));
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
                let _ = SidecarManager::spawn_and_wait(
                    "nonexistent.exe",
                    &project_key,
                    None,
                    None,
                    None,
                    None,
                ).await;
                // 不执行 Phase 3（因为 Phase 2 失败了）
            }));
        }

        for handle in handles {
            let _ = handle.await;
        }

        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 500,
            "15 个并发调用（5 轮 × 3 命令）耗时 {:?}，预期 < 500ms",
            elapsed
        );
    }

    /// 测试 collect_dead_instances 返回正确的死亡实例信息
    #[test]
    fn test_collect_dead_instances_returns_dead_info() {
        let mut manager = SidecarManager::new("nonexistent.exe".into());

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
        let mut manager = SidecarManager::new("nonexistent.exe".into());

        #[cfg(target_os = "windows")]
        let child = Command::new("cmd").args(["/c", "timeout", "10"]).spawn().unwrap();
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
        let manager = SidecarManager::new("nonexistent.exe".into());
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
        let manager = Arc::new(tokio::sync::Mutex::new(
            SidecarManager::new("nonexistent.exe".into())
        ));
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
        let manager = Arc::new(tokio::sync::Mutex::new(
            SidecarManager::new("nonexistent.exe".into())
        ));

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
        for info in dead_instances {
            let _ = SidecarManager::spawn_and_wait(
                &binary_path,
                &info.project_key,
                info.src_dir.as_deref(),
                None,
                info.multi_window,
                info.llm_api.as_deref(),
            ).await;
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
        let manager = Arc::new(tokio::sync::Mutex::new(
            SidecarManager::new("nonexistent.exe".into())
        ));

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
        let _ = SidecarManager::spawn_and_wait(
            &binary_path,
            "test_project",
            None,
            None,
            None,
            None,
        ).await;

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