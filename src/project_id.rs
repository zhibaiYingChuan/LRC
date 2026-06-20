// ============================================================
// 许可证: Apache 2.0
// 本文件实现项目指纹计算，属于公开层 (Layer 1)。
// ============================================================
//
// 项目身份标准化模块 — 通过规范化路径的 SHA256 哈希生成项目指纹
//
// 核心能力:
//   1. 路径规范化（解析符号链接、统一分隔符）
//   2. 跨平台一致性（Windows 转小写处理大小写不敏感）
//   3. SHA256 哈希生成唯一指纹
//   4. 幂等性保证（同一路径多次调用返回相同指纹）
//
// 设计原则:
//   - 同一物理路径，无论从哪个 IDE 打开，指纹一致
//   - 防止路径格式差异（绝对路径 vs 相对路径、符号链接）
//   - 指纹前 16 字符，平衡唯一性和可读性

use sha2::{Digest, Sha256};
use std::path::Path;

/// 计算项目指纹（SHA256 哈希的前 16 个十六进制字符）
///
/// # 算法
/// 1. 规范化路径（解析符号链接、统一分隔符）
/// 2. 转小写（Windows 文件系统不区分大小写）
/// 3. SHA256 哈希 → 取前 16 字符
///
/// # 幂等性
/// 同一项目路径多次调用返回相同指纹，跨 IDE 保持一致。
///
/// # 示例
/// ```ignore
/// let fp = project_fingerprint(Path::new("C:\\Users\\me\\project"));
/// assert_eq!(fp.len(), 16);
/// ```
pub fn project_fingerprint(src_dir: &Path) -> String {
    // 步骤1: 规范化路径 — 解析符号链接、统一分隔符为平台本地格式
    let canonical = src_dir
        .canonicalize()
        .unwrap_or_else(|_| src_dir.to_path_buf());

    // 步骤2: 转小写 — Windows 文件系统不区分大小写，确保 C:\Foo 和 c:\foo 指纹一致
    let normalized = canonical.to_string_lossy().to_lowercase();

    // 步骤3: SHA256 哈希取前 16 字符
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let result = hasher.finalize();
    let hex = format!("{:x}", result);
    hex[..16].to_string()
}

/// 计算项目指纹，同时返回规范化后的路径
///
/// 与 `project_fingerprint` 功能相同，但额外返回规范化路径，
/// 方便在日志和调试中展示实际使用的路径。
pub fn project_fingerprint_with_path(src_dir: &Path) -> (String, String) {
    let canonical = src_dir
        .canonicalize()
        .unwrap_or_else(|_| src_dir.to_path_buf());
    let normalized = canonical.to_string_lossy().to_lowercase();
    // v0.5.4 P2-21 修复：去除 Windows Verbatim 路径前缀（\\?\）用于显示
    // 修复前：canonical_path 显示为 \\?\C:\Users\Administrator，对用户不友好
    // 修复后：canonical_path 显示为 C:\Users\Administrator
    // 注意：指纹计算仍然使用 normalized（带前缀的小写路径），保持向后兼容
    let canonical_path = strip_verbatim_prefix(&canonical.to_string_lossy().to_string());

    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let result = hasher.finalize();
    let hex = format!("{:x}", result);
    let fingerprint = hex[..16].to_string();

    (fingerprint, canonical_path)
}

/// 去除 Windows Verbatim 路径前缀（\\?\）和 UNC 前缀（\\?\UNC\）
/// 使路径显示更友好，如 `\\?\C:\Users\Admin` → `C:\Users\Admin`
fn strip_verbatim_prefix(path: &str) -> String {
    if path.starts_with(r"\\?\UNC\") {
        // UNC 路径：\\?\UNC\server\share → \\server\share
        format!(r"\\{}", &path[7..])
    } else if path.starts_with(r"\\?\") {
        // Verbatim 路径：\\?\C:\Users → C:\Users
        path[4..].to_string()
    } else {
        path.to_string()
    }
}

/// 验证指纹格式是否合法（16 位十六进制字符串）
pub fn is_valid_fingerprint(fp: &str) -> bool {
    fp.len() == 16 && fp.chars().all(|c| c.is_ascii_hexdigit())
}

// ==================== 单元测试（TDD：红→绿→重构） ====================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 测试: 指纹长度固定为 16 字符
    #[test]
    fn test_fingerprint_length() {
        let fp = project_fingerprint(Path::new("."));
        assert_eq!(fp.len(), 16, "指纹长度必须为 16 字符");
    }

    /// 测试: 指纹为十六进制字符串
    #[test]
    fn test_fingerprint_is_hex() {
        let fp = project_fingerprint(Path::new("."));
        assert!(
            fp.chars().all(|c| c.is_ascii_hexdigit()),
            "指纹必须全部是十六进制字符，实际: {fp}"
        );
    }

    /// 测试: 幂等性 — 同一路径多次调用返回相同指纹
    #[test]
    fn test_idempotent() {
        let fp1 = project_fingerprint(Path::new("."));
        let fp2 = project_fingerprint(Path::new("."));
        assert_eq!(fp1, fp2, "同一路径多次调用必须返回相同指纹");
    }

    /// 测试: 不同路径返回不同指纹
    #[test]
    fn test_different_paths_different_fingerprints() {
        let fp1 = project_fingerprint(Path::new("/tmp/project_a"));
        let fp2 = project_fingerprint(Path::new("/tmp/project_b"));
        assert_ne!(fp1, fp2, "不同路径必须返回不同指纹");
    }

    /// 测试: 使用临时目录验证规范化路径
    #[test]
    fn test_real_directory_fingerprint() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let fp = project_fingerprint(tmp.path());
        assert_eq!(fp.len(), 16);
        assert!(is_valid_fingerprint(&fp), "指纹格式验证失败: {fp}");
    }

    /// 测试: 不存在的路径也应该能生成指纹（容错处理）
    #[test]
    fn test_nonexistent_path() {
        let nonexistent = PathBuf::from("/tmp/definitely_not_exist_path_12345");
        let fp = project_fingerprint(&nonexistent);
        assert_eq!(fp.len(), 16, "不存在路径也应能生成指纹");
        assert!(is_valid_fingerprint(&fp));
    }

    /// 测试: 带路径信息的指纹生成
    #[test]
    fn test_fingerprint_with_path() {
        let (fp, path) = project_fingerprint_with_path(Path::new("."));
        assert_eq!(fp.len(), 16);
        assert!(!path.is_empty(), "规范化路径不应为空");
        assert!(is_valid_fingerprint(&fp));
    }

    /// 测试: is_valid_fingerprint 验证
    #[test]
    fn test_valid_fingerprint_check() {
        assert!(is_valid_fingerprint("a1b2c3d4e5f6a7b8"));
        assert!(!is_valid_fingerprint("a1b2c3")); // 太短
        assert!(!is_valid_fingerprint("a1b2c3d4e5f6a7b8extra")); // 太长
        assert!(!is_valid_fingerprint("g1b2c3d4h5f6i7j8")); // 包含非十六进制字符
        assert!(!is_valid_fingerprint("")); // 空字符串
    }

    /// 测试: 大小写相同的路径生成相同指纹（Windows 场景）
    #[test]
    fn test_case_insensitive_fingerprint() {
        // 创建临时目录，模拟大小写不同但指向同一位置
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let path_str = tmp.path().to_string_lossy().to_string();

        // 使用不同大小写构造路径
        let fp_upper = project_fingerprint(Path::new(&path_str.to_uppercase()));
        let fp_lower = project_fingerprint(Path::new(&path_str.to_lowercase()));

        // 因为 canonicalize 会解析为实际路径，所以指纹应该一致
        assert_eq!(fp_upper, fp_lower, "大小写不同的同一路径应生成相同指纹");
    }

    /// 测试: 相对路径和绝对路径指向同一目录时指纹一致
    #[test]
    fn test_relative_vs_absolute() {
        let tmp = tempfile::TempDir::new().expect("创建临时目录失败");
        let abs_path = tmp.path().to_path_buf();

        // 切换到临时目录
        let original_dir = std::env::current_dir().ok();
        std::env::set_current_dir(&abs_path).ok();

        let fp_abs = project_fingerprint(&abs_path);
        let fp_rel = project_fingerprint(Path::new("."));

        // 恢复原目录
        if let Some(dir) = original_dir {
            std::env::set_current_dir(dir).ok();
        }

        assert_eq!(
            fp_abs, fp_rel,
            "相对路径和绝对路径指向同一目录时指纹应一致\n  绝对路径: {fp_abs}\n  相对路径: {fp_rel}"
        );
    }
}
