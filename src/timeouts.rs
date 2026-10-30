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
//! LOCK_WAIT(6s)  <  EXPLORE_INNER_BUDGET(10s)  <  HTTP_HANDLER_BUDGET(15s)  <  HTTP_OUTER_TIMEOUT(120s)
//! LOCK_WAIT(6s)  <  CONSOLIDATE_INNER_TIMEOUT(20s) / BENCHMARKS_INNER_TIMEOUT(20s)  <  HTTP_OUTER_TIMEOUT(120s)
//! ```
//!
//! ## 2026-10-25 调整：外层 30s → 120s（评测现场证据驱动）
//!
//! AML 文本赛道续跑实测（库 298 MB / 14.6 万条）：单次 `/v1/memories/consolidate`
//! 的全量 JSON 重写已达 25~35 秒，**越过原 30 秒外层超时** ⇒ 路由层直接返回 504；
//! 适配层对写路径不重试 504（防重复写），只能把 503 抛给平台 ⇒ 平台每 1~3 分钟收到
//! 一条 `ADD_SERVICE_UNAVAILABLE`，累计到阈值后终止评测（2026-10-25 00:46）。
//! 提到 120 秒后链路为「LRC 120s < adapter 180s < nginx 600s」，余量 5 倍。
//! 内层预算未改（编码/推理仍按 20s 收敛），本次只放宽"排队 + 全量重写"这一段。

use std::time::Duration;

/// **外层**：路由 `TimeoutLayer`。超过即断开连接（`504`）。
pub const HTTP_OUTER_TIMEOUT: Duration = Duration::from_secs(120);

/// **中层**：单请求的业务处理预算（`enrich` / MCP `search` 等 handler 内层）。
pub const HTTP_HANDLER_BUDGET: Duration = Duration::from_secs(15);

/// **内层**：联想探索的内部时间预算（必须严格小于中层，否则"到点优雅返回"不可达）。
pub const EXPLORE_INNER_BUDGET: Duration = Duration::from_secs(10);

/// 全局存储锁的**等待上限**（所有取锁点统一；超时统一返回 `store_busy`）。
///
/// # 为什么由 2s 提到 6s（2026-10-15，6k 库 Full 形状自检实测）
///
/// 单次 `enrich`（`top_k=100`，1 物理核）**全程持锁 ≈1.0s**。因此 **3 并发**时
/// 第三个请求的锁等待 ≈ **2×1.0 = 2.0s**，恰好压在旧的 2s 上限上 ⇒ 实测
/// **49/303 = 16.17% 的请求返回 `search_busy`(503)**（`50 轮 × 每轮 1 个`，
/// 与算术完全吻合）。
///
/// 那 16% 当时**并未丢题**——适配层对瞬时状态码有有界重试（默认 5 次）兜住了。
/// 但代价是：① 每次失败白等 0.6~8s 退避后再重试 ⇒ 延迟虚增；② 重试本身加剧
/// 拥塞，在高并发下会形成正反馈。
///
/// 提到 6s 后，串行化可容纳约 6 个并发请求（6×1.0s）而不失败。**阶梯仍单调**
/// （6s < `EXPLORE_INNER_BUDGET` 10s），且**不改变任何检索结果**——只是把
/// "快速失败"换成"排队成功"。语义上的代价仅为：锁真被长任务占用时，请求会
/// 多等几秒才返回（外层 15s/30s 预算下完全容纳）。
///
/// ⚠️ 已知结构性约束（未修）：单次 `enrich` 全程持锁约 1s，故并发度显著高于
/// ~6 时本上限仍会不足——那需要把只读阶段移出锁（结构性改动），不在本轮。
pub const LOCK_WAIT: Duration = Duration::from_secs(6);

/// 取锁轮询间隔（有界自旋的步长）。
pub const LOCK_POLL: Duration = Duration::from_millis(10);

/// **内层**：`consolidate` 的聚类计算预算（必须严格小于外层 ⇒ 恢复协作式取消，CR-05）。
pub const CONSOLIDATE_INNER_TIMEOUT: Duration = Duration::from_secs(20);

/// **内层**：`benchmarks/report` 的执行预算（必须严格小于外层，CR-14）。
pub const BENCHMARKS_INNER_TIMEOUT: Duration = Duration::from_secs(20);

/// **内层**：`consolidate` 的**编码阶段**预算（锁外长任务）。
///
/// P1-5（2026-10-30）：编码在锁外执行，1 核实测 65~71 条分片需 70~100s。
/// 若外层 [`HTTP_OUTER_TIMEOUT`] 已丢弃 future（连接已断），编码线程仍会跑完并
/// 继续取锁落盘 ⇒ "用户收到 504、数据却写进去了"的**幽灵写入**，
/// 且适配层对写路径不重试 504，平台侧会看到"失败但数据存在"的矛盾状态。
/// 预算取 100s（< 外层 120s，留 20s 给取锁与落盘），超预算即放弃落盘。
pub const CONSOLIDATE_ENCODE_BUDGET: Duration = Duration::from_secs(100);

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
        // P1-5：编码预算同样必须严格小于外层，否则"超预算即放弃"与
        // "外层已断开"这两个时刻会错位，幽灵写入重新出现。
        assert!(
            CONSOLIDATE_INNER_TIMEOUT < CONSOLIDATE_ENCODE_BUDGET,
            "聚类内层预算必须 < 编码预算"
        );
        assert!(
            CONSOLIDATE_ENCODE_BUDGET < HTTP_OUTER_TIMEOUT,
            "编码预算必须 < 外层超时（P1-5 幽灵写入防线）"
        );
    }

    #[test]
    fn poll_interval_is_sub_second() {
        assert!(LOCK_POLL < LOCK_WAIT, "轮询步长必须远小于等待上限");
        assert!(LOCK_POLL.as_millis() >= 1, "步长不得为 0（会变成忙等）");
    }
}
