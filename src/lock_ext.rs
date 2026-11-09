//! ============================================================
//! 许可证: Apache 2.0
//! 本文件提供中毒容忍的锁访问扩展，属于公开层 (Layer 1)。
//! ============================================================
//!
//! # 为什么需要本模块（全局视角，不是补丁）
//!
//! `std::sync::RwLock` / `Mutex` 在**持锁线程 panic** 后会永久中毒：
//! 之后每次 `.read().unwrap()` 都会 panic。于是一次局部故障会被放大成
//! "该锁相关的所有功能全挂"——这正是线上"自持失败"的典型形态
//! （一次超时 → 残留任务 panic → 锁中毒 → 后续请求全 500）。
//!
//! 本项目多数受锁保护的状态是**可重建的缓存/统计**（状态机、审计、调节器、
//! 索引缓存等）。对这些状态，中毒后继续使用（`into_inner`）远优于级联 panic：
//! 数据最多短暂不一致，但服务不中断。
//!
//! 本模块把这一策略收敛为**一处定义**，全项目统一使用，避免"有的地方容忍、
//! 有的地方 panic"的割裂（历史遗留：`semantic_index` 已容忍，其余多为 unwrap）。
//!
//! # 不变量
//!
//! - 中毒恢复**只**用于可重建状态；对"必须一致"的强不变量，调用方应自行校验。
//! - 恢复时**只告警一次**（进程级），避免中毒后在热路径上刷屏。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// 进程级"已告警"标记：中毒恢复只打印一次，避免热路径刷屏。
static POISON_WARNED: AtomicBool = AtomicBool::new(false);

/// 打印一次中毒告警（进程内仅首次）。
fn warn_poison_once(kind: &str) {
    if !POISON_WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "[LRC·锁] ⚠ 检测到 {kind} 中毒，已恢复（数据可能短暂不一致；后续同类告警已静默）"
        );
    }
}

/// 中毒容忍的 `RwLock` 访问。
///
/// 用法：把 `.read().unwrap()` 换成 `.read_recover()`，
/// `.write().unwrap()` 换成 `.write_recover()`。
pub trait LockExt<T: ?Sized> {
    /// 读锁；中毒时恢复内部数据并（首次）告警。
    fn read_recover(&self) -> RwLockReadGuard<'_, T>;
    /// 写锁；中毒时恢复内部数据并（首次）告警。
    fn write_recover(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> LockExt<T> for RwLock<T> {
    fn read_recover(&self) -> RwLockReadGuard<'_, T> {
        match self.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn_poison_once("RwLock 读锁");
                poisoned.into_inner()
            }
        }
    }

    fn write_recover(&self) -> RwLockWriteGuard<'_, T> {
        match self.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn_poison_once("RwLock 写锁");
                poisoned.into_inner()
            }
        }
    }
}

/// 中毒容忍的 `Mutex` 访问。
///
/// 用法：把 `.lock().unwrap()` 换成 `.lock_recover()`。
pub trait MutexExt<T: ?Sized> {
    /// 加锁；中毒时恢复内部数据并（首次）告警。
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> MutexExt<T> for Mutex<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        match self.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn_poison_once("Mutex");
                poisoned.into_inner()
            }
        }
    }
}
