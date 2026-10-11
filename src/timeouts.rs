//! ============================================================
//! 许可证: Apache 2.0
//! 本文件定义「超时阶梯的单一真源」，属于公开层 (Layer 1)。
//! ============================================================
//!
//! ## 为什么需要（结构性修复）
//!
//! 各层超时此前**各自硬编码**，一旦出现"内层 ≥ 外层"，外层 `TimeoutLayer` 会**先丢弃 future**
//! ⇒ 内层那段"超时后置位取消标志"的分支**永不执行**，精心写的**优雅收敛变成不可达代码**。
//! 实测两处倒挂：
//!   - `consolidate`：内层 **60s** > 外层 30s（CR-05）⇒ O(n²) 聚类空跑到底；
//!   - `benchmarks/report`：内层 **90s** > 外层 30s（CR-14）。
//!
//! 本模块把阶梯各层**集中到一处**，并用 [`tests::timeout_ladder_is_monotonic`] 断言
//! **锁等待 < 内层 < 中层 < 外层** ⇒ 该类缺陷在 CI 就被拦下，而不是等线上出现"取消失效"。
//!
//! ## 分层约定
//! ```text
//! LOCK_WAIT(2s)  <  EXPLORE_INNER_BUDGET(10s)  <  HTTP_HANDLER_BUDGET(15s)  <  HTTP_OUTER_TIMEOUT(30s)
//! LOCK_WAIT(2s)  <  CONSOLIDATE_INNER_TIMEOUT(20s) / BENCHMARKS_INNER_TIMEOUT(20s)  <  HTTP_OUTER_TIMEOUT(30s)
//! ```

use std::time::Duration;

/// **外层**：路由 `TimeoutLayer`。超过即断开连接（`504`）。
pub const HTTP_OUTER_TIMEOUT: Duration = Duration::from_secs(30);

/// **中层**：单请求的业务处理预算（`enrich` / MCP `search` 等 handler 内层）。
pub const HTTP_HANDLER_BUDGET: Duration = Duration::from_secs(15);

/// **内层**：联想探索的内部时间预算（必须严格小于中层，否则"到点优雅返回"不可达）。
pub const EXPLORE_INNER_BUDGET: Duration = Duration::from_secs(10);

/// 全局存储锁的**等待上限**（所有取锁点统一；超时统一返回 `store_busy`）。
pub const LOCK_WAIT: Duration = Duration::from_secs(2);

/// 取锁轮询间隔（有界自旋的步长）。
pub const LOCK_POLL: Duration = Duration::from_millis(10);

/// **内层**：`consolidate` 的聚类计算预算（必须严格小于外层 ⇒ 恢复协作式取消，CR-05）。
pub const CONSOLIDATE_INNER_TIMEOUT: Duration = Duration::from_secs(20);

/// **内层**：`benchmarks/report` 的执行预算（必须严格小于外层，CR-14）。
pub const BENCHMARKS_INNER_TIMEOUT: Duration = Duration::from_secs(20);

#[cfg(test)]
mod tests {
    use super::*;

    /// ★守卫（CR-05 / CR-14 的回归防线）：**超时阶梯必须单调** —— 内层 < 中层 < 外层。
    ///
    /// 失败含义：某条路径的内层超时 ≥ 外层 ⇒ 外层先丢弃 future ⇒
    /// 内层的"取消/优雅收敛"分支成为**不可达代码**（历史真实缺陷）。
    #[test]
    fn timeout_ladder_is_monotonic() {
        // 锁等待必须是最小的一层
        assert!(
            LOCK_WAIT < EXPLORE_INNER_BUDGET,
            "锁等待必须 < 探索内部预算"
        );
        assert!(
            LOCK_WAIT < CONSOLIDATE_INNER_TIMEOUT,
            "锁等待必须 < 聚类内层预算"
        );

        // 探索路径：2s < 10s < 15s < 30s
        assert!(
            EXPLORE_INNER_BUDGET < HTTP_HANDLER_BUDGET,
            "探索内部预算必须 < handler 预算"
        );
        assert!(
            HTTP_HANDLER_BUDGET < HTTP_OUTER_TIMEOUT,
            "handler 预算必须 < 外层超时"
        );

        // 重活路径：2s < 20s < 30s
        assert!(
            CONSOLIDATE_INNER_TIMEOUT < HTTP_OUTER_TIMEOUT,
            "consolidate 内层必须 < 外层，否则协作式取消不可达（CR-05）"
        );
        assert!(
            BENCHMARKS_INNER_TIMEOUT < HTTP_OUTER_TIMEOUT,
            "benchmarks 内层必须 < 外层（CR-14）"
        );
    }

    #[test]
    fn poll_interval_is_sub_second() {
        assert!(LOCK_POLL < LOCK_WAIT, "轮询步长必须远小于等待上限");
        assert!(LOCK_POLL.as_millis() >= 1, "步长不得为 0（会变成忙等）");
    }
}
