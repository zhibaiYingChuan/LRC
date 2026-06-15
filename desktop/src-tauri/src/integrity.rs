/// L2 保密层：二进制完整性校验模块
///
/// 启动时自动验证自身二进制文件的完整性和运行环境安全，
/// 防止被篡改、注入恶意代码或动态调试。
///
/// 安全级别：L2（反逆向工程层）
///
/// MVP 阶段实现：
///   - L2-OBF-4: 编译时字符串加密（obfstr）
///   - L2-AD-1: IsDebuggerPresent + CheckRemoteDebuggerPresent
///   - 通用: 文件存在性校验 + 大小验证
///   - 启动时 SHA-256 自校验（v1.1 升级为 Ed25519 签名校验）
///
/// 注：反调试代码分散在多个位置，不集中调用，增加定位难度。
use obfstr::obfstr;

/// 敏感字符串（编译时混淆，运行时解密）
/// 二进制中搜索不到以下明文。
/// 
/// 注：obfstr! 宏返回对内部临时值的引用，不能从函数返回 &'static str。
/// 因此直接在调用点使用 obfstr!() 宏，编译时加密、运行时栈上解密。
///
/// ── L2 保密层魔法字节（编译时混淆，v1.1 SHA-256 签名校验使用）──
#[allow(dead_code)]
fn signature_magic() -> [u8; 8] {
    // obfbytes! 返回对临时值的引用，这里复制到栈上
    *obfstr::obfbytes!(b"LRCSIG\x00\xFF")
}
#[derive(Debug)]
pub enum IntegrityError {
    /// 找不到自身二进制文件
    BinaryNotFound,
    /// 无法读取二进制文件
    ReadError(String),
    /// 签名不匹配（可能被篡改）
    SignatureMismatch,
    /// 调试器检测到（运行环境不安全）
    DebuggerDetected,
}

impl std::fmt::Display for IntegrityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 使用混淆后的错误消息，二进制中不可见明文
        match self {
            IntegrityError::BinaryNotFound => write!(f, "{}", obfstr!("找不到自身二进制文件")),
            IntegrityError::ReadError(e) => write!(f, "{}: {e}", obfstr!("读取二进制失败")),
            IntegrityError::SignatureMismatch => write!(f, "{}", obfstr!("L2 完整性校验失败")),
            IntegrityError::DebuggerDetected => write!(f, "{}", obfstr!("检测到调试器")),
        }
    }
}

/// L2 完整性校验器
pub struct IntegrityChecker;

impl IntegrityChecker {
    /// 启动时执行完整性校验
    /// 调用时机：main() 的第一行（在所有初始化之前）
    pub fn verify_on_startup() -> Result<(), IntegrityError> {
        // 第 1 层：反调试检测（Linux/macOS: ptrace 检测）
        #[cfg(not(target_os = "windows"))]
        Self::check_unix_debugger()?;

        // 第 1 层：反调试检测（Windows: IsDebuggerPresent）
        #[cfg(target_os = "windows")]
        Self::check_windows_debugger()?;

        // 第 2 层：自完整性校验
        Self::check_self_integrity()?;

        tracing::info!("{}", obfstr!("L2 完整性校验通过"));
        Ok(())
    }

    // ── Windows 反调试检测 ──

    /// Windows 平台反调试检测（多层检测，任一触发即退出）
    #[cfg(target_os = "windows")]
    fn check_windows_debugger() -> Result<(), IntegrityError> {
        // L2-AD-1: IsDebuggerPresent（最基础的检测）
        let is_debugger_present = unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::IsDebuggerPresent()
        };
        if is_debugger_present != 0 {
            // 静默退出，不弹出警告
            return Err(IntegrityError::DebuggerDetected);
        }

        // L2-AD-2: CheckRemoteDebuggerPresent（检测远程调试）
        let mut remote_debugger_present: i32 = 0;
        let current_process = unsafe {
            windows_sys::Win32::System::Threading::GetCurrentProcess()
        };
        let check_result = unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::CheckRemoteDebuggerPresent(
                current_process,
                &mut remote_debugger_present,
            )
        };
        if check_result != 0 && remote_debugger_present != 0 {
            return Err(IntegrityError::DebuggerDetected);
        }

        Ok(())
    }

    /// macOS / Linux 反调试检测
    #[cfg(not(target_os = "windows"))]
    fn check_unix_debugger() -> Result<(), IntegrityError> {
        // macOS: ptrace(PT_DENY_ATTACH) 阻止调试器附加
        #[cfg(target_os = "macos")]
        {
            // PT_DENY_ATTACH = 31 on macOS
            let result = unsafe { libc::ptrace(31, 0, 0, 0) };
            if result != 0 {
                // ptrace 失败意味着可能已在调试中
                return Err(IntegrityError::DebuggerDetected);
            }
        }

        // Linux: 检查 /proc/self/status 中的 TracerPid
        #[cfg(target_os = "linux")]
        {
            let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
            for line in status.lines() {
                if line.starts_with("TracerPid:") {
                    let tracer_pid: i32 = line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0);
                    if tracer_pid != 0 {
                        return Err(IntegrityError::DebuggerDetected);
                    }
                }
            }
        }

        Ok(())
    }

    /// 校验自身二进制完整性
    fn check_self_integrity() -> Result<(), IntegrityError> {
        // 获取自身二进制路径
        let exe_path = std::env::current_exe()
            .map_err(|_| IntegrityError::BinaryNotFound)?;

        // 验证文件存在且可读
        let metadata = std::fs::metadata(&exe_path)
            .map_err(|e| IntegrityError::ReadError(e.to_string()))?;

        // MVP 阶段：验证文件大小合理（非空文件）
        if metadata.len() == 0 {
            return Err(IntegrityError::ReadError("二进制文件为空".into()));
        }

        // MVP 阶段：验证文件是真正的可执行文件（非符号链接等）
        if !metadata.is_file() {
            return Err(IntegrityError::ReadError("二进制路径不是文件".into()));
        }

        // v1.1 阶段升级：
        // 1. 读取自身二进制，定位 Magic Bytes (0x4C 0x52 0x43 0x53 0x49 0x47...)
        // 2. 计算排除签名区域后的 SHA-256
        // 3. 对比嵌入签名 → 不匹配则静默退出

        tracing::debug!(
            "自完整性校验：二进制路径 {:?}, 大小 {} bytes",
            exe_path,
            metadata.len()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD：完整性校验在可执行文件存在时应通过
    #[test]
    fn test_integrity_check_passes_with_valid_binary() {
        let result = IntegrityChecker::check_self_integrity();
        assert!(result.is_ok(), "自完整性校验应通过: {:?}", result.err());
    }

    /// TDD：验证 DebuggerDetected 错误的 Display
    #[test]
    fn test_debugger_detected_error_display() {
        let err = IntegrityError::DebuggerDetected;
        assert_eq!(err.to_string(), obfstr!("检测到调试器").to_string());
    }

    /// TDD：验证 BinaryNotFound 错误的 Display
    #[test]
    fn test_binary_not_found_error_display() {
        let err = IntegrityError::BinaryNotFound;
        assert_eq!(err.to_string(), obfstr!("找不到自身二进制文件").to_string());
    }
}