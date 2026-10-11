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
use std::sync::{OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// **快照一致性闸门**（v0.9.11 / G5 / CR-37）—— 让"某一时刻的一致快照"从**承诺**变成**机制**。
///
/// ## 为什么必须存在（这是一处真实的承诺-实现脱节）
/// `backup.rs` 的锁序文档早就写着"备份的语义就是某一时刻的一致快照，**必须与并发的写操作互斥**"，
/// 但**写路径从不取备份锁** —— 于是 `create_backup` 逐文件 `fs::copy` 期间，
/// 实时写可以把新的 `memories.json` 落盘、而 `audit.jsonl` 还停在旧版本，
/// 快照里因此可能出现**悬空审计引用**（记忆存在但审计缺失，或反之）。
/// 文档承诺与实现不符 ⇒ 属"最危险的一类缺陷"（读代码的人会以为已互斥）。
///
/// ## 语义（读写锁的经典用法）
/// | 角色 | 取锁 | 并发性 |
/// |---|---|---|
/// | **写入方**（任何会改变数据文件内容的落盘） | [`writer_guard`]（**读**锁） | 写入方彼此**可并发** |
/// | **快照方**（`create_backup` / `restore_backup`） | [`snapshot_guard`]（**写**锁） | **独占**，期间无任何落盘 |
///
/// ## 锁序（防 ABBA）
/// - 写入方顺序：`cache` → `JSON_WRITE_LOCK` → **本闸门（读）**；
/// - 快照方顺序：`BACKUP_OPERATION_LOCK` → **本闸门（写）**，且**从不**取上面两把锁。
///
/// ⇒ 快照方只取闸门，写入方在闸门之下不再取别的锁，**两模块之间无环路**。
///
/// ## 注意（不可重入）
/// 持 [`snapshot_guard`]（写）期间**不得**再调用 [`write_atomic`] 或
/// `persistence::json::atomic_write`（它们会取读锁 ⇒ 同线程自锁死）。
/// 快照/恢复因此坚持**文件级 `fs::copy`**，不走原子写。
static SNAPSHOT_GATE: OnceLock<RwLock<()>> = OnceLock::new();

fn gate() -> &'static RwLock<()> {
    SNAPSHOT_GATE.get_or_init(|| RwLock::new(()))
}

/// 写入方取用的**共享**守卫：说明"我正在改数据文件"。
///
/// 必须包住整个"写完一个文件"的临界区（含 tmp 写 + rename）。
/// 中毒（持锁线程 panic）时取回内部值：与 [`crate::test_env::lock`] 同款理由 ——
/// 不因一次失败连带炸掉所有写入。
pub fn writer_guard() -> RwLockReadGuard<'static, ()> {
    gate()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 快照方取用的**独占**守卫：说明"我正在做一致性快照"。
///
/// 一旦取得，所有 [`writer_guard`] 持有者与等待者都被排除 ⇒ 7 个文件取自同一时刻。
pub fn snapshot_guard() -> RwLockWriteGuard<'static, ()> {
    gate()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
    // v0.9.11（G5 / CR-37）：进入"我在改数据文件"的共享临界区 ——
    //   与 create_backup/restore_backup 的独占快照区间互斥（防止快照取到半新半旧的一组文件）。
    let _writer = writer_guard();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("atomic");
    let tmp_path = path.with_file_name(format!("{}.{}.tmp", file_name, uuid::Uuid::new_v4()));

    // v0.9.11（G5 收尾 / CR-40）：写入 + **fsync 后再 rename**。
    //   修前只用 `fs::write` + `rename`：两者的**落盘顺序不受保证** ——
    //   断电/崩溃后可能出现"目录项已指向新文件名，内容却仍在页缓存里"
    //   ⇒ 目标文件变成 **0 字节或截断**。对 `memories.json` 而言等于**全部记忆丢失**。
    //   故先把数据 `sync_all()` 落盘，再 rename；rename 之后再同步一次**父目录**
    //   （仅 Unix 可对目录打开句柄；Windows 的 NTFS 日志保证目录项持久性）。
    let write_result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp_path, path)
    })();
    if let Err(error) = write_result {
        // 失败路径清理：临时文件可能已创建（写入失败）或未创建（rename 失败前已写成功）。
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error);
    }
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent() {
            if let Ok(dir) = std::fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod snapshot_gate_tests {
    use super::*;

    /// ★守卫（v0.9.11 / G5 / CR-37）：**写入与快照必须互斥**。
    ///
    /// 用 `try_*` 做**非阻塞**判定，因此不会让测试挂起：
    /// 若二者共用同一把读写锁，则"持写锁时取读锁"与"持读锁时取写锁"都必然失败。
    /// （本断言方向确定：失败只可能来自我们自己持有的守卫，不会因并发用例而误报。）
    #[test]
    fn snapshot_gate_excludes_writers_and_vice_versa() {
        {
            let _snapshot = snapshot_guard();
            assert!(
                gate().try_read().is_err(),
                "快照（写锁）持有期间，写入方必须被排除 —— 否则快照会取到半新半旧的文件组"
            );
        }
        {
            let _writer = writer_guard();
            assert!(
                gate().try_write().is_err(),
                "写入（读锁）持有期间，快照必须被排除"
            );
        }
    }

    /// `write_atomic` 必须真的取闸门读锁（源码级断言，防回归）
    ///
    /// ⚠️ 断言范围**必须截到测试模块之前**：本测试自身源码就含 `writer_guard()`
    /// 字样，若扫到文件尾会"自己匹配自己"⇒ 守卫恒真、失去鉴别力。
    #[test]
    fn write_atomic_acquires_writer_guard() {
        let src = include_str!("atomic_file.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        let start = prod
            .find("pub fn write_atomic(")
            .expect("必须存在 write_atomic");
        let body = &prod[start..];
        assert!(
            body.contains("writer_guard()"),
            "write_atomic 必须取 writer_guard()，否则备份一致性承诺会再次变成空话（CR-37）"
        );
    }
}
