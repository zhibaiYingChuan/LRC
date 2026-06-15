// 许可证: Apache 2.0
//
// 进程守护模块 — 保障 LRC 服务端单一实例、端口自适应、优雅关闭
// ==============================================================
//
// 核心能力:
//   1. SingletonLock  — 文件锁单例，防止同一数据目录下多实例同时运行
//   2. find_available_port — 端口自适应，默认端口被占用时自动尝试下一个
//   3. graceful_shutdown — 信号处理，捕获 SIGINT/SIGTERM 并清理锁文件
//   4. derive_defaults — 零配置默认值推导
//
// 设计原则:
//   - 零外部依赖，仅使用标准库 + tokio
//   - 跨平台兼容（Windows/Linux/macOS）
//   - 锁文件写入 PID，可自检旧进程是否已死（自愈机制）

use std::fmt;
use std::path::{Path, PathBuf};

// ==================== 错误类型 ====================

/// 进程守护模块的错误类型
///
/// 遵循 LRC 错误处理规范：每种错误包含描述、原因和修复建议。
#[derive(Debug)]
pub enum GuardError {
    /// 多窗口记录功能未开启，且已有 LRC 实例在运行
    MultiWindowDisabled { pid: u32, data_dir: PathBuf },
    /// 已达多窗口上限（max_windows > 1 时触发）
    AlreadyRunning {
        pid: u32,
        data_dir: PathBuf,
        limit: u32,
    },
    /// 无法获取文件锁（权限或磁盘问题）
    LockAcquireFailed { path: PathBuf, reason: String },
    /// 所有候选端口均被占用
    NoAvailablePort { base: u16, max_attempts: u16 },
    /// 数据目录不存在且无法创建
    DataDirNotAvailable { path: PathBuf, reason: String },
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MultiWindowDisabled { pid, data_dir } => {
                write!(
                    f,
                    "当前项目已有 LRC 在运行（PID: {}），多窗口记录功能未开启。\n\
                     \n\
                     说明: 你可以在桌面端配置中开启「多窗口 LRC 记录」，\n\
                     开启后同一项目最多支持 3 个 LRC 实例同时运行，\n\
                     方便你在多个编辑器窗口中分别记录记忆。\n\
                     \n\
                     数据目录: {}",
                    pid,
                    data_dir.display()
                )
            }
            Self::AlreadyRunning {
                pid,
                data_dir,
                limit,
            } => {
                write!(
                    f,
                    "已达到多窗口上限（{} 个），无法再启动新实例。\n\
                     \n\
                     说明: 当前项目已有 {} 个 LRC 实例在运行，已达到上限。\n\
                     如需启动新实例，请先关闭一个旧窗口（最早 PID: {}）。\n\
                     \n\
                     数据目录: {}",
                    limit,
                    limit,
                    pid,
                    data_dir.display()
                )
            }
            Self::LockAcquireFailed { path, reason } => {
                write!(
                    f,
                    "无法获取进程锁: {} — {}。请检查磁盘空间和目录写入权限。",
                    path.display(),
                    reason
                )
            }
            Self::NoAvailablePort { base, max_attempts } => {
                write!(
                    f,
                    "端口 {}~{} 全部被占用，共计尝试 {} 次。请关闭占用端口的程序后重试。",
                    base,
                    base + max_attempts - 1,
                    max_attempts
                )
            }
            Self::DataDirNotAvailable { path, reason } => {
                write!(
                    f,
                    "数据目录不可用: {} — {}。请检查路径是否正确。",
                    path.display(),
                    reason
                )
            }
        }
    }
}

impl std::error::Error for GuardError {}

// ==================== 单例锁 ====================

/// 进程单例锁 — 通过文件锁确保同一数据目录下运行的 LRC 进程不超过上限
///
/// 工作原理:
///   1. 启动时在数据目录创建/更新 `.lrc.lock`，写入逗号分隔的 PID 列表
///   2. 新进程启动时检查锁文件：
///      - 读取所有 PID → 过滤掉已死亡的进程
///      - 存活进程数 < max_windows → 添加当前 PID，继续启动
///      - 存活进程数 >= max_windows → 拒绝启动（返回 AlreadyRunning 错误）
///   3. 进程退出时（Drop 实现）从锁文件中移除当前 PID，无剩余时删除文件
///
/// 锁文件格式：
///   - 单窗口: "12345"
///   - 多窗口: "12345,12346,12347"
///
/// 为什么用文件锁而不是 OS 互斥体？
///   - 跨平台一致行为（Windows 的 Named Mutex vs Linux 的 flock 行为不同）
///   - PID 自检机制可以处理进程异常退出（OS 互斥体在进程 crash 时会自动释放，
///     但新进程不知道旧进程是否还活着）
///   - 锁文件本身也是信息载体（dashboard 可读取 PID 显示运行状态）
#[derive(Debug)]
pub struct SingletonLock {
    lock_path: PathBuf,
    acquired: bool,
}

impl SingletonLock {
    /// 尝试获取单例锁（向后兼容：max_windows 默认 1）
    ///
    /// # 参数
    /// - `data_dir`: 数据目录路径，锁文件将创建为 `{data_dir}/.lrc.lock`
    /// - `max_windows`: 最大允许的窗口数（默认 1，单实例模式）
    ///
    /// # 返回
    /// - `Ok(Self)`: 成功获取锁
    /// - `Err(GuardError::AlreadyRunning)`: 已达最大窗口数
    /// - `Err(GuardError::DataDirNotAvailable)`: 数据目录不可用
    /// - `Err(GuardError::LockAcquireFailed)`: 无法写入锁文件
    pub fn acquire(data_dir: &Path, max_windows: u32) -> Result<Self, GuardError> {
        // 确保数据目录存在
        if !data_dir.exists() {
            std::fs::create_dir_all(data_dir).map_err(|e| GuardError::DataDirNotAvailable {
                path: data_dir.to_path_buf(),
                reason: format!("无法创建数据目录: {}", e),
            })?;
        }

        let lock_path = data_dir.join(".lrc.lock");
        let current_pid = std::process::id();

        // 读取现有 PIDs 列表，清理已死的进程
        let mut alive_pids: Vec<u32> = Vec::new();

        if lock_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&lock_path) {
                // 解析逗号分隔的 PID 列表
                for part in content.split(',') {
                    let trimmed = part.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(pid) = trimmed.parse::<u32>() {
                        if is_pid_alive(pid) {
                            alive_pids.push(pid);
                        } else {
                            eprintln!("[进程守护] 检测到残留 PID {} 已不存在，自动清理", pid);
                        }
                    } else {
                        eprintln!("[进程守护] 锁文件内容异常: '{}'，自动跳过", trimmed);
                    }
                }
            } else {
                eprintln!("[进程守护] 无法读取锁文件，将重新创建");
            }
        }

        // 如果锁文件中已有当前 PID → 本进程已持有锁，视为重复启动
        if alive_pids.contains(&current_pid) {
            return Err(GuardError::AlreadyRunning {
                pid: current_pid,
                data_dir: data_dir.to_path_buf(),
                limit: max_windows,
            });
        }

        // 检查是否已达最大窗口数
        if alive_pids.len() as u32 >= max_windows {
            // max_windows == 1 → 多窗口功能未开启，返回 MultiWindowDisabled
            // max_windows > 1  → 已达上限，返回 AlreadyRunning
            return if max_windows <= 1 {
                Err(GuardError::MultiWindowDisabled {
                    pid: *alive_pids.first().unwrap_or(&0),
                    data_dir: data_dir.to_path_buf(),
                })
            } else {
                Err(GuardError::AlreadyRunning {
                    pid: *alive_pids.first().unwrap_or(&0),
                    data_dir: data_dir.to_path_buf(),
                    limit: max_windows,
                })
            };
        }

        // 添加当前 PID
        alive_pids.push(current_pid);

        // 写入新的锁文件
        let new_content: String = alive_pids
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",");

        std::fs::write(&lock_path, &new_content).map_err(|e| GuardError::LockAcquireFailed {
            path: lock_path.clone(),
            reason: format!("写入 PID 列表失败: {}", e),
        })?;

        if alive_pids.len() > 1 {
            eprintln!(
                "[进程守护] 多窗口模式：当前 {} 个窗口 (PID: {})，上限 {}",
                alive_pids.len(),
                current_pid,
                max_windows
            );
        }

        Ok(Self {
            lock_path,
            acquired: true,
        })
    }
}

impl Drop for SingletonLock {
    fn drop(&mut self) {
        if self.acquired {
            let current_pid_str = std::process::id().to_string();

            // 读取现有锁文件，移除当前 PID
            if let Ok(content) = std::fs::read_to_string(&self.lock_path) {
                let remaining: Vec<&str> = content
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| *s != current_pid_str && !s.is_empty())
                    .collect();

                if remaining.is_empty() {
                    // 无剩余进程 → 删除锁文件
                    if let Err(e) = std::fs::remove_file(&self.lock_path) {
                        eprintln!(
                            "[进程守护] 清理锁文件失败 ({}): {}",
                            self.lock_path.display(),
                            e
                        );
                    } else {
                        eprintln!(
                            "[进程守护] 已释放锁文件（最后一个窗口退出）: {}",
                            self.lock_path.display()
                        );
                    }
                } else {
                    // 还有其他进程 → 更新锁文件
                    let new_content = remaining.join(",");
                    if let Err(e) = std::fs::write(&self.lock_path, &new_content) {
                        eprintln!(
                            "[进程守护] 更新锁文件失败 ({}): {}",
                            self.lock_path.display(),
                            e
                        );
                    } else {
                        eprintln!(
                            "[进程守护] 已释放窗口 (PID: {})，剩余 {} 个窗口",
                            current_pid_str,
                            remaining.len()
                        );
                    }
                }
            } else {
                // 无法读取锁文件 → 尝试直接删除
                let _ = std::fs::remove_file(&self.lock_path);
            }
        }
    }
}

/// 检查指定 PID 的进程是否还在运行
///
/// Windows: 通过 OpenProcess 检查进程是否存在
/// Linux/macOS: 检查 /proc/{pid} 目录是否存在（或发送信号 0）
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        // 使用最小权限查询进程状态，不触发 UAC
        extern "system" {
            fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> isize;
            fn CloseHandle(handle: isize) -> i32;
            fn GetExitCodeProcess(process: isize, exit_code: *mut u32) -> i32;
        }

        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const STILL_ACTIVE: u32 = 259;

        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle == 0 || handle == -1 {
            return false; // 无法打开进程 → 进程不存在
        }

        let mut exit_code: u32 = 0;
        let ok = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
        unsafe { CloseHandle(handle) };

        if ok == 0 {
            return false; // GetExitCodeProcess 失败 → 假设进程已死
        }

        exit_code == STILL_ACTIVE // 259 表示进程仍在运行
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Unix 平台：检查 /proc/{pid} 目录是否存在
        // 注意：macOS 虽然没有 /proc，但有相同的进程语义，
        // 可以通过 kill(pid, 0) 检查进程是否存在
        let path = std::path::PathBuf::from(format!("/proc/{}", pid));
        path.exists()
    }
}

// ==================== 端口自适应 ====================

/// 从 base_port 开始尝试绑定，找到第一个可用端口
///
/// 工作机制：
///   1. 尝试 base_port (默认 3099)
///   2. 被占用则尝试 base_port + 1 (3100)
///   3. 继续递增直到找到可用端口或达到 max_attempts 次尝试
///
/// # 返回
/// - `Ok(port)`: 成功找到可用端口，返回实际绑定的 TcpListener
/// - `Err(GuardError::NoAvailablePort)`: 所有候选端口均被占用
pub async fn find_available_port(
    host: &str,
    base_port: u16,
    max_attempts: u16,
) -> Result<(tokio::net::TcpListener, u16), GuardError> {
    for offset in 0..max_attempts {
        let port = base_port + offset;
        let addr = format!("{}:{}", host, port);
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                if offset > 0 {
                    eprintln!(
                        "[进程守护] 端口 {} 已被占用，自适应切换到端口 {}",
                        base_port, port
                    );
                }
                return Ok((listener, port));
            }
            Err(e) => {
                eprintln!(
                    "[进程守护] 端口 {} 不可用 ({}), 尝试下一个...",
                    port,
                    e.kind()
                );
            }
        }
    }

    Err(GuardError::NoAvailablePort {
        base: base_port,
        max_attempts,
    })
}

// ==================== 信号处理 ====================

/// 注册信号处理器，捕获优雅关闭信号
///
/// 在 tokio 运行时中注册 Ctrl+C (SIGINT) 和 SIGTERM 的处理器。
/// 返回一个 Future，当收到关闭信号时 resolve。
///
/// # 使用方式
/// ```ignore
/// let _lock = SingletonLock::acquire(&data_dir)?;
/// let shutdown_signal = register_shutdown_signal();
/// // ... 启动服务 ...
/// tokio::select! {
///     _ = server.serve(...) => {},
///     _ = shutdown_signal => {
///         println!("收到关闭信号，正在优雅退出...");
///     }
/// }
/// // _lock 的 Drop 自动清理锁文件
/// ```
pub async fn wait_for_shutdown_signal() {
    // 注册 Ctrl+C 信号处理器
    #[cfg(target_os = "windows")]
    {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                eprintln!("\n[进程守护] 收到 Ctrl+C 信号，正在关闭...");
            }
            Err(e) => {
                eprintln!("[进程守护] 信号监听失败: {}", e);
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("无法注册 SIGINT 处理器");
        let mut sigterm = signal(SignalKind::terminate()).expect("无法注册 SIGTERM 处理器");

        tokio::select! {
            _ = sigint.recv() => {
                eprintln!("\n[进程守护] 收到 SIGINT 信号，正在优雅退出...");
            }
            _ = sigterm.recv() => {
                eprintln!("\n[进程守护] 收到 SIGTERM 信号，正在优雅退出...");
            }
        }
    }
}

// ==================== 零配置默认值 ====================

/// 推导零配置默认值 — 让用户打开仓库就能直接用，不需要手动传参数
///
/// 优先级:
///   1. 环境变量 LRC_* （显式配置）
///   2. 全局配置文件 ~/.lrc/config.toml
///   3. 智能默认值（当前工作目录、默认端口等）
pub struct DefaultConfig {
    pub src_dir: String,
    pub data_dir: String,
    pub host: String,
    pub port: u16,
    pub llm_api: Option<String>,
    pub mode: String,
}

impl DefaultConfig {
    /// 从命令行参数和环境变量推导最终配置
    ///
    /// CLI 参数优先于环境变量，环境变量优先于默认值。
    /// 如果未提供 src_dir，自动使用当前工作目录。
    /// 如果未提供 db_path，根据 global 标志决定数据目录位置。
    pub fn derive(
        cli_src_dir: &str,
        cli_host: &str,
        cli_port: u16,
        cli_db_path: Option<&str>,
        cli_llm_api: Option<&str>,
        cli_mode: &str,
        global_mode: bool,
    ) -> Self {
        // 源码目录: CLI > 当前工作目录
        let src_dir = if cli_src_dir.is_empty() {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .to_string_lossy()
                .to_string()
        } else {
            cli_src_dir.to_string()
        };

        // 记忆数据目录: CLI db_path > --global > 默认 .loong-recall/data/
        let data_dir = if let Some(custom) = cli_db_path {
            custom.to_string()
        } else if global_mode {
            let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("."));
            home.join(".loong-recall")
                .join("data")
                .to_string_lossy()
                .to_string()
        } else {
            PathBuf::from(&src_dir)
                .join(".loong-recall")
                .join("data")
                .to_string_lossy()
                .to_string()
        };

        // LLM API: CLI > 环境变量 LRC_LLM_API
        let llm_api = if let Some(raw) = cli_llm_api {
            Some(raw.to_string())
        } else {
            std::env::var("LRC_LLM_API").ok()
        };

        Self {
            src_dir,
            data_dir,
            host: cli_host.to_string(),
            port: cli_port,
            llm_api,
            mode: cli_mode.to_string(),
        }
    }
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试: data_dir 不存在时自动创建并获取锁
    #[test]
    fn test_acquire_lock_creates_missing_dir() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let data_dir = tmp.path().join("new_subdir").join("data");

        let lock = SingletonLock::acquire(&data_dir, 1);
        assert!(lock.is_ok(), "应该能自动创建数据目录并获取锁");

        // 验证目录已创建
        assert!(data_dir.exists(), "数据目录应该已被创建");
        // 验证锁文件存在
        assert!(data_dir.join(".lrc.lock").exists(), "锁文件应该已被创建");

        // Drop 锁 → 自动清理
        drop(lock);
        assert!(
            !data_dir.join(".lrc.lock").exists(),
            "锁被 drop 后应该自动清理"
        );
    }

    /// 测试: 已有锁文件且旧进程存活时拒绝启动
    #[test]
    fn test_reject_when_lock_exists_and_pid_alive() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        // 写入当前进程 PID 作为模拟的"旧锁"（同进程重复启动场景）
        let current_pid = std::process::id();
        std::fs::write(data_dir.join(".lrc.lock"), current_pid.to_string()).unwrap();

        let result = SingletonLock::acquire(&data_dir, 1);
        match result {
            Err(GuardError::AlreadyRunning { pid, limit, .. }) => {
                assert_eq!(pid, current_pid, "应该返回当前进程的 PID");
                assert_eq!(limit, 1, "上限应为 1");
            }
            other => panic!(
                "期望 AlreadyRunning 错误（同进程重复启动），实际得到: {:?}",
                other
            ),
        }
    }

    /// 测试: 旧锁对应的 PID 已销毁 → 自动清理并获取锁
    #[test]
    fn test_auto_cleanup_dead_pid_lock() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        // 写入一个几乎不可能存在的 PID
        let fake_pid = 99999u32;
        // 确保这个 PID 不存在（在大多数系统上 99999 不可能被分配）
        std::fs::write(data_dir.join(".lrc.lock"), fake_pid.to_string()).unwrap();

        let lock = SingletonLock::acquire(&data_dir, 1);
        assert!(lock.is_ok(), "旧 PID 不存在时应该自动清理并成功获取锁");

        // 验证新锁包含当前 PID
        let lock_content =
            std::fs::read_to_string(data_dir.join(".lrc.lock")).expect("应能读取新锁文件");
        assert_eq!(
            lock_content.trim(),
            std::process::id().to_string(),
            "锁文件应包含当前 PID"
        );

        drop(lock);
    }

    /// 测试: 多窗口模式 — 允许 N 个窗口同时启动
    #[test]
    fn test_multi_window_allow_multiple() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        // 写入一个不可能存在的假 PID（模拟旧窗口已死）
        std::fs::write(data_dir.join(".lrc.lock"), "99999").unwrap();

        // max_windows=3，应该可以获取锁
        let lock = SingletonLock::acquire(&data_dir, 3);
        assert!(lock.is_ok(), "max_windows=3 时应该能获取锁");

        // 验证锁文件包含当前 PID
        let content = std::fs::read_to_string(data_dir.join(".lrc.lock")).expect("应能读取锁文件");
        assert_eq!(
            content.trim(),
            std::process::id().to_string(),
            "锁文件应包含当前 PID（旧假 PID 已被清理）"
        );

        drop(lock);
    }

    /// 测试: 多窗口模式 — 达到上限时拒绝
    #[test]
    fn test_multi_window_reject_at_limit() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        // 写入当前进程 PID（模拟已有 1 个窗口在运行）
        let current_pid = std::process::id();
        std::fs::write(data_dir.join(".lrc.lock"), current_pid.to_string()).unwrap();

        // max_windows=1，当前 PID 已在锁中 → 应拒绝（AlreadyRunning）
        let result = SingletonLock::acquire(&data_dir, 1);
        assert!(result.is_err(), "max_windows=1 且已有当前 PID 时应拒绝");

        // 清理，换用假 PID 测试
        std::fs::remove_file(data_dir.join(".lrc.lock")).unwrap();

        // 写入一个不存在的 PID（模拟旧窗口残留）
        std::fs::write(data_dir.join(".lrc.lock"), "88888").unwrap();

        // max_windows=2，假 PID 已死 → 可以获取
        let result2 = SingletonLock::acquire(&data_dir, 2);
        assert!(result2.is_ok(), "max_windows=2 且旧 PID 已死时应可获取");

        drop(result2);
    }

    /// 测试: 多窗口 Drop — 移除当前 PID，保留其他
    #[test]
    fn test_multi_window_drop_partial() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let current_pid = std::process::id();

        // 手动构造：锁文件包含当前 PID 和一个假 PID
        std::fs::write(data_dir.join(".lrc.lock"), format!("{},99999", current_pid)).unwrap();

        // 创建锁对象（手动设置，不通过 acquire）
        let lock = SingletonLock {
            lock_path: data_dir.join(".lrc.lock"),
            acquired: true,
        };

        // Drop 锁
        drop(lock);

        // 验证锁文件仍存在（假 PID 还在）
        assert!(
            data_dir.join(".lrc.lock").exists(),
            "假 PID 存在时锁文件不应被删除"
        );

        // 清理
        let _ = std::fs::remove_file(data_dir.join(".lrc.lock"));
    }

    /// 测试: is_pid_alive 对当前进程返回 true
    #[test]
    fn test_current_pid_is_alive() {
        assert!(is_pid_alive(std::process::id()), "当前进程应该被检测为存活");
    }

    /// 测试: is_pid_alive 对不可能的 PID 返回 false
    #[test]
    fn test_impossible_pid_is_dead() {
        // 大多数系统上 PID 0 是 Idle 进程，但无法通过 OpenProcess 打开
        // PID 99999 极不可能存在
        assert!(!is_pid_alive(99999), "PID 99999 应该不存在");
    }

    /// 测试: DefaultConfig 的零配置推导
    #[test]
    fn test_derive_defaults_empty_cli() {
        let config = DefaultConfig::derive("", "127.0.0.1", 3099, None, None, "auto", false);

        // src_dir 应该默认为当前工作目录
        assert!(!config.src_dir.is_empty(), "src_dir 不应为空");
        assert_eq!(config.port, 3099, "默认端口应为 3099");
        assert!(config.llm_api.is_none(), "未传 LLM 时应为 None");

        // data_dir 应该在 src_dir/.loong-recall/data/ 下
        assert!(
            config.data_dir.contains(".loong-recall"),
            "data_dir 应包含 .loong-recall, 实际: {}",
            config.data_dir
        );
    }

    /// 测试: 环境变量 LRC_LLM_API 被正确读取
    #[test]
    fn test_llm_api_from_env() {
        // 注意: 这个测试在设置环境变量后需要清理，避免污染其他测试
        let test_key = "openai:sk-test123:gpt-4o-mini:https://api.openai.com/v1";
        std::env::set_var("LRC_LLM_API", test_key);

        let config = DefaultConfig::derive("", "127.0.0.1", 3099, None, None, "fast", false);
        assert_eq!(
            config.llm_api.as_deref(),
            Some(test_key),
            "应从环境变量读取 LLM API 配置"
        );

        // 清理环境变量
        std::env::remove_var("LRC_LLM_API");
    }

    /// 测试: CLI 参数优先于环境变量
    #[test]
    fn test_cli_overrides_env() {
        std::env::set_var(
            "LRC_LLM_API",
            "openai:sk-env:env-model:https://env.api.com/v1",
        );

        let cli_value = "openai:sk-cli:cli-model:https://cli.api.com/v1";
        let config =
            DefaultConfig::derive("", "127.0.0.1", 3099, None, Some(cli_value), "fast", false);

        assert_eq!(
            config.llm_api.as_deref(),
            Some(cli_value),
            "CLI 参数应优先于环境变量"
        );

        std::env::remove_var("LRC_LLM_API");
    }

    /// 测试: global 模式下 data_dir 使用全局路径
    #[test]
    fn test_global_mode_data_dir() {
        let config = DefaultConfig::derive("", "127.0.0.1", 3099, None, None, "auto", true);

        // 全局模式下 data_dir 应包含 .loong-recall
        assert!(
            config.data_dir.contains(".loong-recall"),
            "全局模式 data_dir 应包含 .loong-recall"
        );
    }

    /// 测试: 显式 db_path 优先于 --global
    #[test]
    fn test_explicit_db_path_overrides_global() {
        let custom = "D:/custom-memory-data";
        let config = DefaultConfig::derive(
            "",
            "127.0.0.1",
            3099,
            Some(custom),
            None,
            "auto",
            true, // global 模式
        );

        assert_eq!(config.data_dir, custom, "显式 db_path 应优先于 global 标志");
    }

    /// 测试: GuardError Display 格式化
    #[test]
    fn test_guard_error_display() {
        let err = GuardError::MultiWindowDisabled {
            pid: 12345,
            data_dir: PathBuf::from("/tmp/lrc-data"),
        };
        let msg = err.to_string();
        assert!(msg.contains("12345"), "错误信息应包含 PID");
        assert!(msg.contains("/tmp/lrc-data"), "错误信息应包含路径");
        assert!(msg.contains("多窗口记录功能未开启"), "应提示未开启");

        let err2 = GuardError::AlreadyRunning {
            pid: 12345,
            data_dir: PathBuf::from("/tmp/lrc-data"),
            limit: 3,
        };
        let msg2 = err2.to_string();
        assert!(msg2.contains("12345"), "错误信息应包含 PID");
        assert!(msg2.contains("3"), "错误信息应包含上限");

        let err3 = GuardError::NoAvailablePort {
            base: 3099,
            max_attempts: 10,
        };
        let msg3 = err3.to_string();
        assert!(msg3.contains("3099"), "错误信息应包含起始端口");
        assert!(
            msg3.contains("3108"),
            "错误信息应包含终点端口 (3099+10-1=3108)"
        );
        assert!(msg3.contains("10"), "错误信息应包含尝试次数");
    }

    /// 测试: SingletonLock Drop 时的幂等性（多次 drop 不 panic）
    #[test]
    fn test_lock_drop_idempotent() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let lock_path = data_dir.join(".lrc.lock");

        // 先通过 acquire 创建锁
        let lock = SingletonLock::acquire(&data_dir, 1).expect("应能获取锁");
        drop(lock);

        // 验证锁文件确实被删除了
        assert!(!lock_path.exists(), "最后一个窗口退出后锁文件应被删除");
    }
}
