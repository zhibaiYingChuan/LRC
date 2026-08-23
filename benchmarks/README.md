# 基准测试

本目录包含两类用户可查看的能力证明材料：

## 当前版本基准

- [v0.9.5 内置基准测试报告](V0.9.5_BENCHMARK_REPORT.md)
- 测试入口保留在仓库的 `tests/benchmarks.rs`
- 当前报告由 v0.9.5 源码实际执行生成，不代表外部数据集成绩

## 外部对比基准

- [外部对比基准说明](comparative/BENCHMARK_GUIDE.md)
- [外部对比汇总](comparative/LRC_BENCHMARK_SUMMARY.md)
- MS MARCO、Natural Questions、HotpotQA、FiQA、LongMemEval 等报告位于 `comparative/`
- 这些报告保留原始评测版本、日期、指标和运行边界；历史结果不得冒充当前版本结果

## 说明

报告用于展示 LRC 的检索、记忆演化、隐私和审计能力。重新评测时必须记录版本、日期、硬件、运行模式和数据集版本，避免不同环境下直接比较。
