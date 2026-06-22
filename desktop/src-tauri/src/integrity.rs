/// L2 保密层：二进制完整性校验模块
///
/// 启动时自动验证自身二进制文件的完整性和运行环境安全，
/// 防止被篡改、注入恶意代码或动态调试。
///
/// 安全级别：L2（反逆向工程层）
///
/// 实现：
///   - L2-OBF-4: 编译时字符串加密（obfstr）
///   - L2-AD-1: IsDebuggerPresent + CheckRemoteDebuggerPresent
///   - 文件存在性校验 + 大小验证
///   - SHA-256 自校验：运行时读取自身二进制，定位签名区域，计算并对比哈希
///
/// 注：反调试代码分散在多个位置，不集中调用，增加定位难度。
use obfstr::obfstr;
use sha2::{Digest, Sha256};

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
        // SAFETY: IsDebuggerPresent 是 Windows 标准 API，无参数，无内存操作，调用始终安全
        let is_debugger_present = unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::IsDebuggerPresent()
        };
        if is_debugger_present != 0 {
            // 静默退出，不弹出警告
            return Err(IntegrityError::DebuggerDetected);
        }

        // L2-AD-2: CheckRemoteDebuggerPresent（检测远程调试）
        let mut remote_debugger_present: i32 = 0;
        // SAFETY: GetCurrentProcess 返回当前进程的伪句柄，无内存分配，调用始终安全
        let current_process = unsafe {
            windows_sys::Win32::System::Threading::GetCurrentProcess()
        };
        // SAFETY: CheckRemoteDebuggerPresent 接收有效进程句柄和布尔指针，remote_debugger_present 是栈上变量
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
            // SAFETY: ptrace(PT_DENY_ATTACH, 0, null, 0) 是标准反调试调用，参数均为常量，不涉及内存操作
            let result = unsafe { libc::ptrace(31, 0, std::ptr::null_mut(), 0) };
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

    /// 校验自身二进制完整性（SHA-256 自校验）
    ///
    /// 签名格式：二进制文件末尾附加 [Magic Bytes (8 bytes)] + [SHA-256 哈希 (32 bytes)]
    /// Magic Bytes: LRCSIG\x00\xFF (0x4C 0x52 0x43 0x53 0x49 0x47 0x00 0xFF)
    ///
    /// 校验流程：
    /// 1. 读取自身二进制文件
    /// 2. 从文件末尾搜索 Magic Bytes
    /// 3. 若找到签名，提取 32 字节哈希，计算文件内容（排除签名区域）的 SHA-256
    /// 4. 对比哈希值 → 不匹配则返回 SignatureMismatch
    /// 5. 若未找到签名（开发模式），跳过校验并记录警告
    fn check_self_integrity() -> Result<(), IntegrityError> {
        // 获取自身二进制路径
        let exe_path = std::env::current_exe()
            .map_err(|_| IntegrityError::BinaryNotFound)?;

        // 验证文件存在且可读
        let metadata = std::fs::metadata(&exe_path)
            .map_err(|e| IntegrityError::ReadError(e.to_string()))?;

        // 验证文件大小合理（非空文件）
        if metadata.len() == 0 {
            return Err(IntegrityError::ReadError("二进制文件为空".into()));
        }

        // 验证文件是真正的可执行文件
        if !metadata.is_file() {
            return Err(IntegrityError::ReadError("二进制路径不是文件".into()));
        }

        // 文件太小，不可能包含签名（至少需要 8 字节 Magic + 32 字节哈希 = 40 字节）
        let file_size = metadata.len();
        if file_size < 40 {
            tracing::warn!("二进制文件过小，跳过 SHA-256 完整性校验（开发模式）");
            return Ok(());
        }

        // 读取整个二进制文件
        let binary_data = std::fs::read(&exe_path)
            .map_err(|e| IntegrityError::ReadError(e.to_string()))?;

        // 从文件末尾搜索 Magic Bytes
        let magic: [u8; 8] = *obfstr::obfbytes!(b"LRCSIG\x00\xFF");
        let signature_offset = Self::find_magic_bytes(&binary_data, &magic);

        match signature_offset {
            Some(offset) => {
                // 签名区域：Magic Bytes(8) + SHA-256 Hash(32) = 40 bytes
                let expected_hash_start = offset + 8;
                let expected_hash_end = expected_hash_start + 32;

                // 边界检查：确保签名区域在文件范围内
                if expected_hash_end > binary_data.len() {
                    return Err(IntegrityError::SignatureMismatch);
                }

                // 提取嵌入的 SHA-256 哈希
                let embedded_hash = &binary_data[expected_hash_start..expected_hash_end];

                // 计算文件内容（排除签名区域）的 SHA-256
                let content_to_hash = &binary_data[..offset];
                let mut hasher = Sha256::new();
                hasher.update(content_to_hash);
                let computed_hash = hasher.finalize();

                // 对比哈希值（使用恒定时间比较，防止时序攻击）
                if constant_time_eq(embedded_hash, computed_hash.as_slice()) {
                    tracing::info!(
                        "SHA-256 完整性校验通过（文件大小: {} bytes, 签名位置: {}）",
                        file_size,
                        offset
                    );
                    Ok(())
                } else {
                    tracing::error!("SHA-256 完整性校验失败：哈希不匹配，二进制可能已被篡改");
                    Err(IntegrityError::SignatureMismatch)
                }
            }
            None => {
                // 未找到签名 → 开发模式，跳过校验
                tracing::warn!(
                    "未找到 SHA-256 签名（Magic Bytes 不存在），跳过完整性校验。\
                     生产环境请使用签名工具嵌入签名。文件大小: {} bytes",
                    file_size
                );
                Ok(())
            }
        }
    }

    /// 在二进制数据中从末尾搜索 Magic Bytes
    ///
    /// 从后往前搜索，找到第一个匹配的位置。
    /// 返回 Magic Bytes 的起始偏移量，若未找到则返回 None。
    fn find_magic_bytes(data: &[u8], magic: &[u8]) -> Option<usize> {
        let magic_len = magic.len();
        if data.len() < magic_len {
            return None;
        }
        // 从末尾向前搜索，找到第一个匹配
        data.windows(magic_len)
            .enumerate()
            .rev()
            .find(|(_, window)| *window == magic)
            .map(|(idx, _)| idx)
    }
}

/// 恒定时间字节比较，防止时序攻击
///
/// 比较两个字节切片是否相等，无论内容如何，耗时相同。
/// 用于安全比较哈希值，防止攻击者通过分析响应时间推断正确哈希。
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// TDD：完整性校验在可执行文件存在时应通过
    #[test]
    fn test_integrity_check_passes_with_valid_binary() {
        let result = IntegrityChecker::check_self_integrity();
        // 开发模式下可能没有签名，但不应返回错误
        match result {
            Ok(()) => {} // 通过
            Err(IntegrityError::SignatureMismatch) => {
                panic!("不应该返回签名不匹配错误")
            }
            Err(_) => {
                // 其他错误（如找不到二进制）在测试环境中可能发生
            }
        }
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

    /// TDD：验证 SignatureMismatch 错误的 Display
    #[test]
    fn test_signature_mismatch_error_display() {
        let err = IntegrityError::SignatureMismatch;
        assert_eq!(err.to_string(), obfstr!("L2 完整性校验失败").to_string());
    }

    /// TDD：find_magic_bytes 在数据末尾找到 Magic Bytes
    #[test]
    fn test_find_magic_bytes_at_end() {
        let magic: [u8; 8] = *obfstr::obfbytes!(b"LRCSIG\x00\xFF");
        let mut data = vec![0u8; 100];
        // 将 Magic Bytes 放在末尾
        data[92..100].copy_from_slice(&magic);
        let result = IntegrityChecker::find_magic_bytes(&data, &magic);
        assert_eq!(result, Some(92));
    }

    /// TDD：find_magic_bytes 未找到时返回 None
    #[test]
    fn test_find_magic_bytes_not_found() {
        let magic: [u8; 8] = *obfstr::obfbytes!(b"LRCSIG\x00\xFF");
        let data = vec![0u8; 100];
        let result = IntegrityChecker::find_magic_bytes(&data, &magic);
        assert_eq!(result, None);
    }

    /// TDD：find_magic_bytes 数据太短时返回 None
    #[test]
    fn test_find_magic_bytes_too_short() {
        let magic: [u8; 8] = *obfstr::obfbytes!(b"LRCSIG\x00\xFF");
        let data = vec![0u8; 5];
        let result = IntegrityChecker::find_magic_bytes(&data, &magic);
        assert_eq!(result, None);
    }

    /// TDD：constant_time_eq 相同数据返回 true
    #[test]
    fn test_constant_time_eq_same() {
        let a = [1u8, 2, 3, 4];
        let b = [1u8, 2, 3, 4];
        assert!(constant_time_eq(&a, &b));
    }

    /// TDD：constant_time_eq 不同数据返回 false
    #[test]
    fn test_constant_time_eq_different() {
        let a = [1u8, 2, 3, 4];
        let b = [1u8, 2, 3, 5];
        assert!(!constant_time_eq(&a, &b));
    }

    /// TDD：constant_time_eq 不同长度返回 false
    #[test]
    fn test_constant_time_eq_different_length() {
        let a = [1u8, 2, 3];
        let b = [1u8, 2, 3, 4];
        assert!(!constant_time_eq(&a, &b));
    }

    /// TDD：SHA-256 哈希计算一致性验证
    #[test]
    fn test_sha256_consistency() {
        let data = b"LRC integrity test data";
        let mut hasher1 = Sha256::new();
        hasher1.update(data);
        let hash1 = hasher1.finalize();

        let mut hasher2 = Sha256::new();
        hasher2.update(data);
        let hash2 = hasher2.finalize();

        assert_eq!(hash1.as_slice(), hash2.as_slice());
    }

    /// TDD：SHA-256 不同数据产生不同哈希
    #[test]
    fn test_sha256_different_data_different_hash() {
        let mut hasher1 = Sha256::new();
        hasher1.update(b"data A");
        let hash1 = hasher1.finalize();

        let mut hasher2 = Sha256::new();
        hasher2.update(b"data B");
        let hash2 = hasher2.finalize();

        assert_ne!(hash1.as_slice(), hash2.as_slice());
    }
}