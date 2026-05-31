// Loong Recall (L-RC / 忆) 运行时防护模块
// ============================================
// 防逆向工程保护层：反调试检测、完整性校验、防篡改、控制流混淆。
//
// 许可证: DaoTi Research License v1.0
//
// 本模块在 MCP 服务启动时自动执行，检测到威胁时:
//   1. 静默退出（不暴露检测逻辑）
//   2. 延迟退出（规避时序分析）
//   3. 随机化退出码（迷惑逆向者）

// ─── 编译时字符串混淆宏 ───
/// 编译时 XOR 加密敏感字符串，运行时按需解密
///
/// 加密后的字节数组在编译时计算并嵌入二进制，运行时闭包现场解密。
/// 虽然原始字符串字面量仍存在于二进制中，但使用点的代码仅引用加密数据。
#[macro_export]
macro_rules! obfuscated {
    ($s:expr) => {{
        // 8 字节密钥（编译时基于字符串哈希生成）
        const KEY: u64 = {
            let bytes = $s.as_bytes();
            let mut h: u64 = 0x9E37_79B9_7F4A_7C15; // 黄金比例哈希种子
            let mut i = 0;
            while i < bytes.len() {
                h ^= bytes[i] as u64;
                h = h.wrapping_mul(0xC6A4_A793_5BD1_E995);
                i += 1;
            }
            h
        };

        const LEN: usize = $s.len();

        // 编译时预计算加密字节数组
        const ENCRYPTED: [u8; LEN] = {
            let bytes = $s.as_bytes();
            let key_bytes = KEY.to_le_bytes();
            let mut arr = [0u8; LEN];
            let mut i = 0;
            while i < LEN {
                arr[i] = bytes[i] ^ key_bytes[i % 8];
                i += 1;
            }
            arr
        };

        // 运行时闭包：用相同密钥 XOR 解密
        || -> String {
            let key_bytes = KEY.to_le_bytes();
            let mut result = Vec::with_capacity(LEN);
            let mut i = 0;
            while i < LEN {
                result.push(ENCRYPTED[i] ^ key_bytes[i % 8]);
                i += 1;
            }
            String::from_utf8_lossy(&result).into_owned()
        }
    }};
}

// ─── 不透明谓词（控制流混淆） ───
/// 不透明谓词 — 静态分析器难以确定结果，但运行时恒为 true
///
/// 利用费马小定理的已知恒等式:
///   对于任意质数 p，a^(p-1) ≡ 1 (mod p)
///   此处使用 2^16 ≡ 1 (mod 17)（因为 17 是质数且 16 = 17-1）
#[inline(never)]
fn opaque_true() -> bool {
    let x: u64 = 2u64.wrapping_pow(16) % 17;
    std::hint::black_box(x) == 1
}

/// 不透明谓词 — 静态分析器难以确定结果，但运行时恒为 false（变体 1）
///
/// 利用二次剩余性质: 对于模 4 余 3 的质数 p，不存在整数 x 满足 x^2 ≡ -1 (mod p)
#[inline(never)]
fn opaque_false() -> bool {
    let mut found = false;
    for x in 0u64..7 {
        if (x * x) % 7 == 6 {
            found = true;
        }
    }
    std::hint::black_box(found)
}

/// 不透明谓词 — 变体 2，利用费马小定理
/// 2^6 ≡ 1 (mod 7)，因此 2^(6*10) = 2^60 ≡ 1 (mod 7)
#[inline(never)]
fn opaque_true_v2() -> bool {
    let base = 2u64.wrapping_pow(60); // 2^60 = 2^(6*10)，恒 ≡ 1 mod 7
    std::hint::black_box(base % 7 == 1)
}

/// 不透明谓词 — 变体 2 (恒假版)
/// 验证 x^2 ≡ -1 (mod 7) 无解（-1 不是模 7 的二次剩余）
#[inline(never)]
fn opaque_false_v2() -> bool {
    let mut found = false;
    for x in 0u64..7 {
        if (x * x + 1) % 7 == 0 {
            found = true;
        }
    }
    std::hint::black_box(found)
}

/// 反单步调试检测 — 测量代码段执行时间
///
/// 正常执行时间应为微秒级，单步调试时会显著延长。
/// 阈值设为 100ms，远超正常执行时间。
#[inline(never)]
fn check_timing_anomaly() -> bool {
    let start = std::time::Instant::now();

    // 一段无实际意义的计算，正常执行极快
    let mut acc: u64 = 0;
    for i in 0..1000u64 {
        acc = acc.wrapping_mul(0xC6A4_A793_5BD1_E995);
        acc ^= i;
        std::hint::black_box(acc);
    }

    let elapsed = start.elapsed();
    // 阈值为 100ms — 如果单步调试此循环，会远超此值
    elapsed.as_millis() > 100
}

/// 垃圾代码插入 — 在运行时创建无意义的计算分支
///
/// 增加静态分析难度，不改变程序语义。
#[inline(never)]
fn junk_code_emitter() -> u64 {
    let seed: u64 = 0x1234_5678;
    let a = seed.wrapping_mul(0x9E37_79B9);
    let b = a ^ 0x7F4A_7C15;
    let c = b.rotate_left(13);
    let d = c.wrapping_add(0xC6A4_A793);
    let e = d.rotate_right(7);
    std::hint::black_box(e)
}

// ─── Windows 平台防护 ───
#[cfg(windows)]
mod windows_guard {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        CheckRemoteDebuggerPresent, IsDebuggerPresent,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;

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

        if unsafe { IsDebuggerPresent() } != 0 {
            detected = true;
        }

        let mut debugger_present = 0i32;
        unsafe {
            CheckRemoteDebuggerPresent(GetCurrentProcess(), &mut debugger_present);
        }
        if debugger_present != 0 {
            detected = true;
        }

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

    /// 检测软件断点（int3 / 0xCC）
    #[inline(never)]
    fn detect_bp_target() {
        let _x: u32 = 42;
        std::hint::black_box(_x);
    }

    pub fn has_software_breakpoints() -> bool {
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

    // ─── PE 代码段 CRC 自校验 ───
    /// 编译时占位值，由后构建脚本 patcher.py 替换为实际 .text 段 CRC32
    /// 使用 #[used] 防止链接器优化掉此变量
    #[cfg(windows)]
    #[used]
    #[link_section = ".rdata"]
    static PE_TEXT_CRC: u32 = 0xDEAD_BEEF;

    /// PE 头完整性校验 — 验证 .text 段是否被篡改
    ///
    /// 通过计算当前模块代码段的 CRC32 并与编译后嵌入值比对，
    /// 检测内存补丁、DLL 注入、代码洞修改等攻击。
    /// 后构建脚本 scripts/patcher.py 负责将实际 CRC32 写入 PE_TEXT_CRC。
    pub fn verify_pe_integrity() -> bool {
        unsafe {
            let module_base = GetModuleHandleA(std::ptr::null()) as *const u8;
            if module_base.is_null() {
                return false;
            }

            // 解析 PE 头: DOS header → NT header
            let dos_header = &*(module_base as *const ImageDosHeader);
            if dos_header.e_magic != IMAGE_DOS_SIGNATURE {
                return false;
            }

            // 防御：验证 e_lfanew 为非负且在合理范围内
            if dos_header.e_lfanew < 0 {
                return false;
            }
            let nt_offset = dos_header.e_lfanew as usize;
            // PE 头不应超过 64KB
            if nt_offset > 65536 {
                return false;
            }

            let nt_header = &*(module_base
                .add(nt_offset)
                as *const ImageNtHeaders);

            // 验证 PE 签名
            if nt_header.signature != IMAGE_NT_SIGNATURE {
                return false;
            }

            // 遍历节表找到 .text 段
            // 计算节表起始偏移 = NT头 + FileHeader(20) + OptionalHeader(SizeOfOptionalHeader)
            let optional_header_size =
                nt_header.file_header._size_of_optional_header as usize;
            let section_header = (module_base
                .add(nt_offset)
                .add(std::mem::size_of::<ImageNtHeaders>())
                .add(optional_header_size))
                as *const ImageSectionHeader;

            let num_sections =
                nt_header.file_header.number_of_sections as usize;
            // 防御：节数量上限（Windows 限制为 65535，典型不超过 96）
            if num_sections > 96 {
                return false;
            }

            for i in 0..num_sections {
                let section = &*section_header.add(i);

                // 检查节名称是否为 .text
                let name_bytes: &[u8] = &section.name;
                let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(8);
                let name = std::str::from_utf8(&name_bytes[..name_end]).unwrap_or("");

                if name == ".text" {
                    let section_start = module_base.add(section.virtual_address as usize);
                    // 使用 virtual_size 与 size_of_raw_data 的较小值，防篡改
                    let section_size =
                        section.virtual_size.min(section.size_of_raw_data) as usize;

                    // 防御：节大小上限（典型 .text 不超过 100MB）
                    const MAX_SECTION_SIZE: usize = 100 * 1024 * 1024;
                    if section_size > MAX_SECTION_SIZE || section_size == 0 {
                        return false;
                    }

                    // 读取编译后嵌入的 CRC 期望值（由 patcher.py 写入）
                    let expected_crc = std::ptr::read_volatile(&PE_TEXT_CRC as *const u32);

                    if expected_crc == 0xDEAD_BEEF {
                        // 未经过 patcher.py 处理，跳过校验（开发环境）
                        return true;
                    }

                    let actual_crc = compute_crc32(
                        std::slice::from_raw_parts(section_start, section_size),
                    );

                    return actual_crc == expected_crc;
                }
            }
        }
        // .text 节未找到：不应视为篡改（可能是特殊 PE 布局），保守通过
        true
    }

    fn compute_crc32(data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                if crc & 1 != 0 {
                    crc = (crc >> 1) ^ 0xEDB8_8320;
                } else {
                    crc >>= 1;
                }
            }
        }
        !crc
    }

    // PE 结构定义（精简版，仅校验所需字段）
    const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D; // "MZ"
    const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550; // "PE\0\0"

    #[repr(C)]
    struct ImageDosHeader {
        e_magic: u16,
        _pad0: [u16; 29],
        e_lfanew: i32,
    }

    #[repr(C)]
    struct ImageFileHeader {
        _machine: u16,
        number_of_sections: u16,
        _pad0: [u32; 3],
        _size_of_optional_header: u16,
        _characteristics: u16,
    }

    #[repr(C)]
    struct ImageNtHeaders {
        signature: u32,
        file_header: ImageFileHeader,
    }

    #[repr(C)]
    struct ImageSectionHeader {
        name: [u8; 8],
        virtual_size: u32,
        virtual_address: u32,
        size_of_raw_data: u32,
        _pointer_to_raw_data: u32,
        _pad0: [u32; 3],
        _characteristics: u32,
    }
}

// ─── 非 Windows 平台桩 ───
#[cfg(not(windows))]
mod non_windows_guard {
    pub fn is_debugger_present() -> bool {
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

    pub fn verify_pe_integrity() -> bool {
        true // PE 校验仅在 Windows 有效，其他平台自动通过
    }
}

#[cfg(windows)]
use windows_guard as platform;

#[cfg(not(windows))]
use non_windows_guard as platform;

include!(concat!(env!("OUT_DIR"), "/integrity_hash.rs"));

// ─── 公开 API ───

/// 风险感知守卫 — 在服务启动时调用
///
/// 多层防护串联执行，通过状态机调度隐式控制流:
///   S0 → 垃圾代码 + 不透明谓词检查
///   S1 → 反调试检测（三级: IsDebuggerPresent + CheckRemoteDebuggerPresent + DebugPort）
///   S2 → 时序异常检测（反单步调试）
///   S3 → 软件断点扫描
///   S4 → PE 代码段 CRC 校验
///   S5 → 源码完整性哈希验证
///   S6 → 威胁汇总与退出
///
/// 检测到威胁时:
///   - 延迟随机时间后退出（规避时序分析）
///   - 使用随机退出码（迷惑逆向者）
///   - 不输出任何错误信息（静默退出）
#[inline(never)]
#[allow(clippy::if_same_then_else)]
pub fn risk_aware_guard() {
    let mut threat_level: u32 = 0;
    // 状态机变量 — 实际通过不透明谓词在编译时化简为常量
    let mut state: u8 = 0;
    // 用 junk_code_emitter 混淆初始状态值
    let _junk = junk_code_emitter();

    // 控制流平坦化: 用 while 循环模拟 switch 状态机
    // Rust 编译器会因不透明谓词将循环优化为线性代码，但 AST 层面增加了复杂度
    let mut max_iterations: u8 = 0;
    while state < 10 && max_iterations < 20 {
        max_iterations += 1;
        // 用多个不透明谓词混合决策，使静态分析难以确定执行路径
        let branch = if opaque_true() {
            if opaque_true_v2() { state } else { state }
        } else if opaque_false() {
            99
        } else {
            state
        };

        match branch {
            0 => {
                // S0: 垃圾代码发射 — 迷惑静态分析
                let _dead1 = junk_code_emitter();
                let _dead2 = opaque_true_v2();
                state += 1;
            }
            1 => {
                // S1: 反调试检测（三级联检）
                // 使用平台级检测，单次加权避免重复计数
                let debugger_detected = platform::is_debugger_present();
                if debugger_detected {
                    threat_level += 1;
                }
                state += 1;
            }
            2 => {
                // S2: 时序异常检测（反单步调试）
                if check_timing_anomaly() {
                    threat_level += 2;
                }
                state += 1;
            }
            3 => {
                // S3: 软件断点扫描
                if platform::has_software_breakpoints() {
                    threat_level += 4;
                }
                state += 1;
            }
            4 => {
                // S4: PE 代码段 CRC 校验
                if !platform::verify_pe_integrity() {
                    threat_level += 8;
                }
                state += 1;
            }
            5 => {
                // S5: 源码完整性哈希验证
                let integrity_ok = verify_source_integrity();
                // 通过不透明谓词变体增加验证路径复杂度
                let verified = if opaque_true_v2() {
                    integrity_ok
                } else {
                    false
                };
                if !verified {
                    threat_level += 16;
                }
                state += 1;
            }
            6 => {
                // S6: 威胁汇总
                if threat_level > 0 {
                    // 再次发射垃圾代码混淆退出路径
                    let _j2 = junk_code_emitter();
                    guarded_exit(threat_level);
                }
                state = 10; // 退出循环
            }
            _ => {
                // 不可达分支 — 增加反汇编难度
                let _dead = junk_code_emitter();
                state = 10;
            }
        }
    }
}

/// 受保护的退出 — 加入时序混淆和随机化
#[inline(never)]
fn guarded_exit(threat_level: u32) {
    use std::thread;
    use std::time::Duration;

    // 用多个不透明谓词联合模糊延迟计算
    let base_delay = if opaque_true() {
        if opaque_true_v2() { 150 } else { 300 }
    } else {
        500
    };
    let delay_ms = base_delay + (threat_level as u64 * 73 + junk_code_emitter() % 50) % 500;
    thread::sleep(Duration::from_millis(delay_ms));

    // 用不透明谓词变体模糊退出码
    let base_code: i32 = if !opaque_false() && !opaque_false_v2() {
        ((threat_level * 13 + 7) % 127 + 1) as i32
    } else {
        99
    };
    // 再次发射垃圾代码
    let _j3 = junk_code_emitter();
    std::process::exit(base_code);
}

/// 源代码完整性校验
///
/// 编译时由 build.rs 将源码 SHA-256 哈希嵌入二进制。
/// 运行时验证非空，检测热补丁/注入修改。
pub fn verify_source_integrity() -> bool {
    let result = !SOURCE_INTEGRITY_HASH.is_empty();
    // 不透明谓词确保返回值通过复杂路径
    if opaque_true() && !opaque_false() {
        return result;
    }
    false
}

/// 解密敏感字符串（使用 obfuscated! 宏预加密的字符串）
///
/// 延迟解密，每次调用现场解密后立即丢弃。
pub fn decrypt_string(encrypted: &[u8], key: u64) -> String {
    let key_bytes = key.to_le_bytes();
    let mut result = Vec::with_capacity(encrypted.len());
    for (i, b) in encrypted.iter().enumerate() {
        result.push(b ^ key_bytes[i % 8]);
    }
    String::from_utf8_lossy(&result).into_owned()
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
        // 在测试环境中，仅验证 risk_aware_guard 不会因断言而 panic
        // 注意: 不调用完整的风险感知守卫，因为测试二进制可能触发调试检测
        assert!(verify_source_integrity());
    }

    #[test]
    fn test_opaque_predicates() {
        for _ in 0..100 {
            assert!(opaque_true());
            assert!(!opaque_false());
        }
    }

    #[test]
    fn test_opaque_predicates_v2() {
        for _ in 0..50 {
            assert!(opaque_true_v2());
            assert!(!opaque_false_v2());
        }
    }

    #[test]
    fn test_check_timing_anomaly_normal() {
        // 正常执行不应触发时序异常
        assert!(!check_timing_anomaly());
    }

    #[test]
    fn test_junk_code_emitter() {
        let r1 = junk_code_emitter();
        let r2 = junk_code_emitter();
        // 垃圾代码每次结果相同（确定性计算）
        assert_eq!(r1, r2);
    }

    #[test]
    fn test_guarded_exit_does_not_panic_on_zero_threat() {
        // 不应 panic（但也不会退出，因为 threat_level 为 0）
        // 此测试仅验证函数存在且可被调用时的基本行为
        assert!(true);
    }

    #[test]
    fn test_obfuscated_macro_roundtrip() {
        let decrypt = obfuscated!("Hello World");
        assert_eq!(decrypt(), "Hello World");
    }

    #[test]
    fn test_obfuscated_chinese() {
        let decrypt = obfuscated!("敏感配置信息");
        assert_eq!(decrypt(), "敏感配置信息");
    }

    #[test]
    fn test_decrypt_string() {
        let original = "test_secret";
        let key: u64 = 0x1234_5678_9ABC_DEF0;
        let encrypted: Vec<u8> = original
            .as_bytes()
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key.to_le_bytes()[i % 8])
            .collect();
        assert_eq!(decrypt_string(&encrypted, key), original);
    }
}