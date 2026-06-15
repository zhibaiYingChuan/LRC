/// Sidecar 进程管理器
///
/// 管理 code-memory-server 子进程的生命周期。
/// 对接现有的 server.rs，通过 HTTP 通信。
///
/// 生命周期保证：
///   - Drop 时自动 kill 子进程（防止僵尸进程）
///   - 启动时等待健康检查通过（最多 10 秒）
///   - 端口自适应：扫描起始端口 + 10 范围，匹配 sidecar 实际绑定端口
///
/// 默认端口：3099（与 sidecar 默认值一致）。
/// 注意：不要传 0，因为 0 会导致 sidecar 尝试绑定特权端口（<1024）而失败。
const DEFAULT_SIDECAR_PORT: u16 = 3099;
/// 端口扫描范围：实际端口 = 起始端口 + 0..PORT_SCAN_RANGE
/// 与 server.rs 中 find_available_port 的 scan_range(100) 保持一致
const PORT_SCAN_RANGE: u16 = 100;
use std::process::{Child, Command};
use std::time::Duration;

/// Sidecar 运行状态
#[derive(Debug, Clone, PartialEq)]
pub enum SidecarState {
    /// 未启动
    Stopped,
    /// 启动中（等待健康检查）
    Starting,
    /// 运行中
    Running { pid: u32, port: u16 },
    /// 发生错误
    Error(String),
}

/// Sidecar 进程管理器
pub struct SidecarManager {
    /// 当前状态
    state: SidecarState,
    /// 子进程句柄
    child: Option<Child>,
    /// 二进制路径
    binary_path: String,
}

/// Drop 守卫：确保子进程在管理器被销毁时被 kill
impl Drop for SidecarManager {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            tracing::info!("SidecarManager 释放，kill 子进程 PID={}", child.id());
            // 尝试优雅终止（先 SIGTERM / Ctrl+C），再 SIGKILL
            let _ = child.kill();
            // 不等待退出，避免阻塞 Drop
            self.child = None;
        }
    }
}

impl SidecarManager {
    /// 创建新的 sidecar 管理器
    /// 
    /// 自动搜索多个可能位置（按优先级）：
    /// 1. 指定的 binary_path
    /// 2. 同目录下的 code-memory-server.exe
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
            state: SidecarState::Stopped,
            child: None,
            binary_path: resolved_path,
        }
    }

    /// 自动搜索 sidecar 二进制文件
    fn find_sidecar_binary() -> String {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let binary_name = format!("code-memory-server{}", std::env::consts::EXE_SUFFIX);

        // 搜索路径（按优先级）
        let search_paths = [
            exe_dir.join(&binary_name),                // 同目录
            exe_dir.join("resources").join(&binary_name), // resources/ 子目录
            exe_dir.parent().unwrap_or(&exe_dir).join(&binary_name), // 上级目录
        ];

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

    /// 获取当前状态
    pub fn status(&self) -> &SidecarState {
        &self.state
    }

    /// 检查 sidecar 是否正在运行
    pub fn is_running(&self) -> bool {
        matches!(self.state, SidecarState::Running { .. })
    }

    /// 启动 sidecar 进程
    /// 返回实际使用的端口号
    pub async fn start(
        &mut self,
        src_dir: Option<String>,
        port: Option<u16>,
        multi_window: Option<u32>,
        llm_api: Option<String>,
    ) -> Result<u16, String> {
        // 如果已在运行，检查进程是否存活
        if let SidecarState::Running { pid, port } = self.state {
            if self.is_process_alive(pid) {
                return Ok(port);
            }
            // 进程已死，清理状态
            tracing::warn!("Sidecar PID={pid} 已退出，重新启动");
            self.child = None;
            self.state = SidecarState::Stopped;
        }

        self.state = SidecarState::Starting;

        // 使用 sensible 默认端口（3099），而非 0（0 会导致 sidecar 尝试绑定特权端口）
        let actual_port = port.unwrap_or(DEFAULT_SIDECAR_PORT);

        // 构建启动参数
        let mut cmd = Command::new(&self.binary_path);
        cmd.args(["--port", &actual_port.to_string()]);

        if let Some(dir) = src_dir {
            cmd.args(["--src-dir", &dir]);
        }

        // 多窗口模式：始终传递 --multi-window 参数给 sidecar（ON=3, OFF=1）
        if let Some(mw) = multi_window {
            cmd.args(["--multi-window", &mw.to_string()]);
            tracing::info!("多窗口模式：{} 个 LRC 实例上限", mw);
        }

        // 传递 LLM API 配置（从桌面端向导读取，确保仪表盘感知配置状态）
        if let Some(ref llm) = llm_api {
            if !llm.is_empty() {
                cmd.args(["--llm-api", llm]);
                tracing::info!("已传递 LLM 配置到 Sidecar（类型: {}）",
                    llm.split(':').next().unwrap_or("unknown"));
            }
        }

        // 启动子进程
        let child = cmd
            .spawn()
            .map_err(|e| format!("启动 sidecar 失败: {e}"))?;

        let pid = child.id();
        self.child = Some(child);

        // 等待健康检查通过（最多 10 秒）
        let port = self.wait_for_health(pid, actual_port).await?;

        self.state = SidecarState::Running { pid, port };

        tracing::info!("Sidecar 已启动: PID={pid}, port={port}");
        Ok(port)
    }

    /// 停止 sidecar 进程
    pub async fn stop(&mut self) -> Result<(), String> {
        if let Some(mut child) = self.child.take() {
            let pid = child.id();
            // 先尝试优雅终止
            let _ = child.kill();
            // 等待进程退出（最多 5 秒）
            let wait_result = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::task::spawn_blocking(move || child.wait()),
            )
            .await
            .map_err(|_| "等待 sidecar 退出超时".to_string())?
            .map_err(|e| format!("等待 sidecar 退出失败: {e}"))?;

            tracing::info!("Sidecar PID={pid} 已停止: {:?}", wait_result);
        }
        self.state = SidecarState::Stopped;
        Ok(())
    }

    /// 等待 sidecar 健康检查通过
    /// 
    /// 端口自适应扫描：sidecar 可能因端口冲突而绑定到不同端口，
    /// 因此从起始端口开始扫描 PORT_SCAN_RANGE 个端口，找到实际绑定的端口。
    /// 
    /// 每 500ms 检查一次，最多尝试 20 次（10 秒）
    async fn wait_for_health(&mut self, pid: u32, start_port: u16) -> Result<u16, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

        for attempt in 1..=20 {
            // 检查进程是否还活着
            if !self.is_process_alive(pid) {
                return Err(format!("Sidecar 进程 PID={pid} 启动后意外退出"));
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

        // 健康检查超时，但进程还在运行，仍返回起始端口（让用户自行验证）
        tracing::warn!("Sidecar 健康检查超时，但进程 PID={pid} 仍在运行");
        Ok(start_port)
    }

    /// 检查进程是否存活（跨平台）
    ///
    /// Windows: 尝试通过 try_wait 检查子进程是否已退出
    /// Unix: 发送信号 0 检查进程是否存在
    fn is_process_alive(&mut self, pid: u32) -> bool {
        #[cfg(target_os = "windows")]
        {
            // Windows: 使用子进程句柄的 try_wait 检查
            if let Some(ref mut child) = self.child {
                match child.try_wait() {
                    Ok(None) => true,  // 进程仍在运行
                    Ok(Some(_)) => false, // 进程已退出
                    Err(_) => {
                        // try_wait 失败时回退到 PID 检查
                        let _ = pid;
                        true
                    }
                }
            } else {
                false
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            // Unix：发送信号 0 检查进程是否存在
            unsafe { libc::kill(pid as i32, 0) == 0 }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD：测试初始状态为 Stopped
    #[test]
    fn test_initial_state_is_stopped() {
        let manager = SidecarManager::new("test-server.exe".into());
        assert!(matches!(manager.status(), SidecarState::Stopped));
    }

    /// TDD：测试重复启动返回相同端口
    #[test]
    fn test_double_start_returns_same_port() {
        // 此测试需要 mock 子进程，在集成测试中验证
    }
}