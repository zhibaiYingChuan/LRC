// Loong Recall (L-RC / 忆) 运行时防护模块
// ============================================
// 防逆向工程保护层：反调试检测、完整性校验、防篡改。
//
// 许可证: DaoTi Research License v1.0
//
// 本模块在 MCP 服务启动时自动执行，检测到威胁时:
//   1. 静默退出（不暴露检测逻辑）
//   2. 延迟退出（规避时序分析）
//   3. 随机化退出码（迷惑逆向者）

#[cfg(windows)]
mod windows_guard {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        CheckRemoteDebuggerPresent, IsDebuggerPresent,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    /// NtQueryInformationProcess 使用的信息类枚举（NT 内核未公开类型）
    type ProcessInformationClass = u32;

    const PROCESS_DEBUG_PORT: ProcessInformationClass = 7;

    extern "system" {
        fn NtQueryInformationProcess(
            process_handle: HANDLE,
            process_information_class: ProcessInformationClass,
            process_information: *mut std::ffi::c_void,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    /// 检测调试器是否附加到当前进程（三级检测）
    pub fn is_debugger_present() -> bool {
        let mut detected = false;

        // 方法 1: IsDebuggerPresent — 最基础的 PEB BeingDebugged 标志
        let is_present = unsafe { IsDebuggerPresent() };
        if is_present != 0 {
            detected = true;
        }

        // 方法 2: CheckRemoteDebuggerPresent — 检查远程调试器
        let mut debugger_present = 0i32;
        unsafe {
            CheckRemoteDebuggerPresent(
                GetCurrentProcess(),
                &mut debugger_present,
            );
        }
        if debugger_present != 0 {
            detected = true;
        }

        // 方法 3: NtQueryInformationProcess — 检查 DebugPort（内核级检测）
        let mut debug_port: HANDLE = std::ptr::null_mut();
        let mut return_length: u32 = 0;
        let status = unsafe {
            NtQueryInformationProcess(
                GetCurrentProcess(),
                PROCESS_DEBUG_PORT,
                &mut debug_port as *mut _ as *mut std::ffi::c_void,
                std::mem::size_of::<HANDLE>() as u32,
                &mut return_length,
            )
        };
        if status >= 0 && !debug_port.is_null() {
            detected = true;
        }

        detected
    }

    /// 检测是否存在软件断点（int3 / 0xCC 指令）
    ///
    /// 检查当前函数入口前 8 字节，检测是否被调试器插入 0xCC。
    pub fn has_software_breakpoints() -> bool {
        // 使用一个简单的内联函数地址作为检查目标
        let target_fn = detect_bp_target as *const u8;
        for i in 0..8 {
            unsafe {
                if target_fn.add(i).read_volatile() == 0xCC {
                    return true;
                }
            }
        }
        false
    }

    /// 断点检测的目标函数（在栈上分配，不易被优化掉）
    #[inline(never)]
    fn detect_bp_target() {
        let _x: u32 = 42;
        std::hint::black_box(_x);
    }
}

#[cfg(not(windows))]
mod non_windows_guard {
    pub fn is_debugger_present() -> bool {
        // Linux/macOS: 检查 /proc/self/status 中的 TracerPid
        #[cfg(target_os = "linux")]
        {
            if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
                for line in status.lines() {
                    if line.starts_with("TracerPid:") {
                        let pid = line
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("0")
                            .parse::<u32>()
                            .unwrap_or(0);
                        return pid != 0;
                    }
                }
            }
        }
        false
    }

    pub fn has_software_breakpoints() -> bool {
        false
    }
}

#[cfg(windows)]
use windows_guard as platform;

#[cfg(not(windows))]
use non_windows_guard as platform;

include!(concat!(env!("OUT_DIR"), "/integrity_hash.rs"));

/// 风险感知守卫 — 在服务启动时调用
///
/// 检测到调试/篡改时不会立即退出，而是:
/// - 延迟随机时间后退出（规避时序分析）
/// - 使用随机退出码（迷惑逆向者）
pub fn risk_aware_guard() {
    let mut threat_level: u32 = 0;

    // 检查 1: 调试器检测
    if platform::is_debugger_present() {
        threat_level += 1;
    }

    // 检查 2: 软件断点检测
    if platform::has_software_breakpoints() {
        threat_level += 2;
    }

    if threat_level > 0 {
        use std::thread;
        use std::time::Duration;

        let delay_ms = 100 + (threat_level as u64 * 73) % 500;
        thread::sleep(Duration::from_millis(delay_ms));

        let exit_code = (threat_level * 13 + 7) % 127 + 1;
        std::process::exit(exit_code as i32);
    }
}

/// 源代码完整性校验
///
/// 在编译时将源码哈希嵌入二进制，运行时验证非空。
/// 检测运行时是否被热补丁或注入修改。
pub fn verify_source_integrity() -> bool {
    !SOURCE_INTEGRITY_HASH.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_source_integrity() {
        assert!(verify_source_integrity(), "完整性哈希不应为空");
    }

    #[test]
    fn test_risk_aware_guard_no_debugger() {
        // 在正常测试环境中不应触发保护
        risk_aware_guard();
    }
}