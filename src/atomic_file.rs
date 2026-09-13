//! 原子文件写入工具（Layer 1 公共设施）
//!
//! v0.9.7 新增（GLOBAL_CODE_REVIEW_REPORT P3-3「原子写入逻辑重复 4 处」）：
//!   修复前，`arch_config.rs` / `config.rs` / `data_dir.rs` / `engine/audit_trail.rs`
//!   各自手写了一遍"先写临时文件再 rename"的逻辑，且实现质量参差：
//!     - `config.rs` / `data_dir.rs` / `arch_config.rs` 用**固定**临时名
//!       （`{}.tmp` / `{}.json.tmp`）→ 并发写同一路径时两方争用同一临时文件，
//!       可能一方 rename 到另一方写了一半的内容；
//!     - 写入或 rename 失败时**不清理**临时文件 → 残留 `.tmp` 垃圾。
//!   本模块把该逻辑收敛为唯一实现：临时名带 UUID 保证唯一，失败路径清理，
//!   并保持"同目录内 rename 是原子操作"这一语义（临时文件与原文件同目录）。
//!
//! 设计边界：
//!   - 本模块**不加锁**。跨进程/跨实例的串行化由调用方按各自锁序负责
//!     （见 `persistence/json.rs` 的「锁序契约」）。
//!   - 返回 `std::io::Result<()>`，调用方按自身错误类型 `map_err` 转换。

use std::path::Path;

/// 原子写入：先写同目录下的唯一临时文件，再 `rename` 覆盖目标路径。
///
/// # 语义
/// - 临时文件名形如 `{原名}.{uuid}.tmp`，与目标文件**同目录**——保证 `rename`
///   不跨文件系统，从而具备原子性。
/// - 写入或 `rename` 任一步失败时，尽力删除临时文件后返回错误，
///   避免残留垃圾文件。
/// - 目标文件已存在时会被原子替换；不存在时被原子创建。
///
/// # 参数
/// - `path`：目标文件路径。
/// - `bytes`：要写入的完整内容。
///
/// # 错误
/// 透传底层 `std::io::Error`（磁盘满、权限不足、目录不存在等）。
///
/// # 注意
/// 调用方须自行确保 `path` 的父目录已存在（本函数不创建父目录）。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("atomic");
    let tmp_path = path.with_file_name(format!("{}.{}.tmp", file_name, uuid::Uuid::new_v4()));

    if let Err(error) =
        std::fs::write(&tmp_path, bytes).and_then(|_| std::fs::rename(&tmp_path, path))
    {
        // 失败路径清理：临时文件可能已创建（写入失败）或未创建（rename 失败前已写成功）。
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error);
    }
    Ok(())
}
