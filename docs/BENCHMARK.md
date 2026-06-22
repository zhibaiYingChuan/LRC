# 性能测试指南

本文档说明如何在本地复现 Loong Recall 的性能基准测试。

---

## 测试环境要求

| 项目 | 最低要求 | 推荐配置 |
|---|---|---|
| CPU | Intel i5 / AMD R5 | Intel i7 / AMD R7 |
| 内存 | 4 GB 可用 | 16 GB 可用 |
| 磁盘 | SSD（推荐） | NVMe SSD |
| 操作系统 | Windows 10+ / Linux / macOS | Linux (Ubuntu 22.04+) |
| Rust | 1.75+ | 1.80+ |

---

## 编译

使用快速模式（默认，除 Rust 工具链外零外部依赖）：

```bash
git clone https://github.com/zhibaiYingChuan/LRC.git
# 国内用户如遇 GitHub 下载缓慢，可使用镜像：
# git clone https://gitcode.com/gcw_M73FIiUo/LRC
cd LRC
cargo build --release --features server
```

编译产物位于 `target/release/code-memory-server`。

---

## 测试方法

### 1. 启动服务

```bash
# HTTP 模式（便于用 curl 发送测试请求）
./target/release/code-memory-server --src-dir ./src --port 3099
```

### 2. 写入测试记忆

使用 `remember` 工具批量写入记忆。以下为概念性示例，实际测试时根据数据规模调整：

```bash
# 写入单条记忆
curl -X POST http://127.0.0.1:3099/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/call",
    "params": {
      "name": "remember",
      "arguments": {
        "content": "测试记忆内容",
        "memory_type": "fact",
        "importance": 5
      }
    }
  }'
```

### 3. 测量检索延迟

使用 `recall` 工具并在请求前后记录时间戳：

```bash
# 测量单次检索延迟
time curl -s -X POST http://127.0.0.1:3099/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
      "name": "recall",
      "arguments": {
        "query": "测试查询",
        "top_k": 5
      }
    }
  }' > /dev/null
```

---

## 预期性能数据

以下为参考数据（基于 Intel i7-13700K / 32GB DDR5 / NVMe SSD）：

| 记忆规模 | 检索延迟 (P50) | 检索延迟 (P99) | 内存占用 |
|---|---|---|---|
| 1,000 条 | < 1ms | < 2ms | < 5 MB |
| 10,000 条 | < 3ms | < 5ms | < 10 MB |
| 100,000 条 | < 10ms | < 15ms | < 20 MB |
| 1,000,000 条 | < 20ms | < 30ms | < 50 MB |

> 实际性能受 CPU 型号、内存速度、磁盘 I/O 等因素影响，以上数据仅供参考。

### 测试条件说明

上表中数据的测试条件：

- **编码模式**：快速模式（`FastEncoder`，默认），未启用 CodeBERT
- **GPU 加速**：未使用，全部基于 CPU 计算
- **ROI 配置**：使用系统默认的可配置区域参数
- **衰减机制**：指数衰减模型正常工作
- **数据分布**：随机生成的混合类型记忆（fact / preference / decision），模拟真实使用场景
- **并发**：单客户端串行请求，未启用并发

> 如果启用 CodeBERT 模式或调整 ROI 配置，实际延迟会有所不同。以上数据旨在展示系统在默认配置下的基线性能，不等同于所有部署场景的精确值。

---

## 性能影响因素

### 有利因素

- **SSD 存储**：数据持久化在磁盘上，NVMe SSD 可显著降低冷启动加载时间
- **多核 CPU**：编码和检索可利用多核并行加速
- **`--global` 模式**：全局记忆模式减少不必要的项目级初始化开销

### 需注意因素

- **CodeBERT 模式**：启用 `ml` feature 后首次启动需下载模型（~200MB），国内用户自动使用 hf-mirror.com 镜像下载。内存占用增加至 ~500MB
- **超大项目代码库**：代码索引（`search_code` 路径）的耗时与项目文件数量成正比，与记忆检索（`recall` 路径）相互独立
- **首次索引**：首次对项目代码建立索引需要遍历全部文件，大型项目可能需要数秒至数十秒

---

## 对比基准

如需与其它记忆系统进行性能对比，建议在相同硬件环境下测试以下指标：

1. **写入吞吐**：每秒可写入的记忆条目数
2. **检索延迟**：不同记忆规模下的 P50 和 P99 检索延迟
3. **内存占用**：不同记忆规模下的常驻内存（RSS）
4. **冷启动时间**：从进程启动到首次检索可用的时间
5. **磁盘占用**：存储 N 条记忆所需的磁盘空间

---

## 公平性改革说明

### 改革背景

LRC 基准测试经历了从"测架构"到"测能力"的根本性改革。改革的核心是将"验证架构"（测有没有洛书编码/LLM翻译器）转变为"验证效果"（测能不能做到知识更新/模糊查询/双关词区分）。

### 改革原则

- **不利用 ground truth**: 所有文档 importance=5（统一），不利用答案信息差异化
- **使用标准指标**: BEIR 标准指标 + LongMemEval 标准指标
- **蓄水池抽样随机文档**: 不偏向相关文档
- **跳过合成记忆**: 避免 synthesis 类型干扰评估
- **不修改 LRC 源代码**: 使用原版 release 二进制

### 改革效果

- LRC 原生基准 TF-IDF 模式: 11/11 PASS, 总评分 0.94
- LRC 原生基准 LLM 模式: 9/11 PASS, 总评分 0.79
- LongMemEval 公平版 v3: Session Recall@10=85.74%

---

## 6 次基准测试结果概览

LRC v0.5.6 在 6 个标准基准测试上进行了全面评估，覆盖关键词检索、自然语言问题、多跳推理、金融领域问题、长时记忆能力和综合记忆能力。

### 基准测试一览

| 序号 | 基准测试 | 评估目标 | 关键结果 | 报告链接 |
| :---: | :--- | :--- | :--- | :--- |
| 1 | MS MARCO | 关键词检索能力 | TF-IDF MRR=0.7749, LLM MRR=0.8895 | [报告](../benchmarks/reports/LRC_MSMARCO_REPORT.md) |
| 2 | Natural Questions | 自然语言问题检索 | TF-IDF MRR=0.5389, LLM MRR=0.8016 | [报告](../benchmarks/reports/LRC_NQ_REPORT.md) |
| 3 | HotpotQA | 多跳推理检索 | TF-IDF MRR=0.7964, LLM MRR=0.9383 | [报告](../benchmarks/reports/LRC_HOTPOTQA_REPORT.md) |
| 4 | FiQA | 金融领域检索 | TF-IDF MRR=0.2729, LLM MRR=0.4453 | [报告](../benchmarks/reports/LRC_FIQA_REPORT.md) |
| 5 | LRC 原生基准 | 综合记忆能力 | TF-IDF 11/11 PASS, 总评分 0.94 | [报告](../benchmarks/reports/LRC_NATIVE_BENCHMARK_TFIDF.md) |
| 6 | LongMemEval | 长时记忆能力 | v3 公平 Session Recall@10=85.74% | [报告](../benchmarks/reports/LRC_LONGMEMEVAL_REPORT.md) |

### 综合评级

| 检索能力 | 评级 | 说明 |
| :--- | :--- | :--- |
| TF-IDF 检索 | ★★★★☆ | 关键词/实体名匹配优秀，自然语言/金融领域较弱 |
| LLM 查询翻译 | ★★★★☆ | BEIR 数据集核心能力，增益 +14.8% ~ +63.2% |
| 洛书几何编码 | ★★★☆☆ | 结构化知识检索辅助，不适合自然语言对话 |
| 检索延迟 | ★★★★★ | TF-IDF 极快（2.6-21ms），LLM 可接受（~1s） |
| 综合记忆能力 | ★★★★★ | LRC 原生基准公平版 11/11 PASS |

> 完整的 6 次基准测试汇总对比报告请参考 [LRC_BENCHMARK_SUMMARY.md](../benchmarks/reports/LRC_BENCHMARK_SUMMARY.md)。

---

## LongMemEval 基准测试 (ICLR 2025) — 公平版

Loong Recall 已在 [LongMemEval](https://github.com/xiaowu0162/LongMemEval) 基准测试上进行评估。LongMemEval 是 ICLR 2025 收录的长时记忆评估基准，包含 500 个高质量测试用例，覆盖五大能力维度：信息抽取、多会话推理、知识更新、时序推理和拒绝应答。

### 公平性说明

本次评估采用 v3 公平版策略：
- **注入粒度**: 会话级 + Turn 级双层索引
- **importance**: 统一 5（不利用 has_answer 差异化）
- **不利用 ground truth**: 公平条件下评估 LRC 的真实检索基本功

### 核心结果（v3 公平版）

| 指标 | v1（仅会话级） | v2（作弊） | v3（公平） |
| :--- | ---: | ---: | ---: |
| Session Recall@10 | 72.77% | 88.51% | **85.74%** |
| Turn Recall@10 | 38.09% | 72.98% | **61.70%** |
| Session MRR | 0.4099 | 0.5752 | 0.5499 |
| Turn MRR | 0.1713 | 0.3432 | 0.2864 |
| 公平性 | 公平 | 不公平 | **公平** |

### 关键发现

1. **v3（公平）vs v1（公平）**: Turn 级注入让 Session Recall 从 72.77% 提升到 85.74%（+12.97%），即使不利用 ground truth，Turn 级注入本身就有巨大价值。

2. **v2（作弊）vs v3（公平）**: importance 差异化额外带来 Session Recall +2.77%，这 2.77% 就是"作弊"的收益，也就是 LRC 重要性加权机制的上限。

3. **v3（公平）85.74% 是 LRC 在公平条件下的真实基本功**，且 0 错误 500 实例稳定。

### 按问题类型（v3 公平版）

| 问题类型 | Session R@10 | Turn R@10 |
| :--- | ---: | ---: |
| single-session-assistant | **98.21%** | 71.43% |
| knowledge-update | **95.83%** | 76.39% |
| single-session-preference | 90.00% | 70.00% |
| multi-session | 85.12% | 53.72% |
| single-session-user | 79.69% | 59.38% |
| temporal-reasoning | 77.17% | 55.91% |

### deep 模式说明

v3-deep 模式（洛书几何 + 八卦预过滤）在 LongMemEval 上 5 实例全 ✗（0% 召回）。原因：
- 八卦预过滤将查询编码为 8 维向量后投影到八卦分类，只保留同卦或相邻卦的记忆
- LongMemEval 的问题和答案内容可能被分到不同八卦，导致预过滤阶段直接排除正确答案
- deep 模式是为结构化知识检索设计（如代码库搜索），而非自然语言对话的关键词匹配
- 这不代表 LRC 的洛书几何编码"无用"，只是不在这个基准测试的适用范围内

### 性能表现

- **平均记忆注入耗时**: ~0.105s / 实例（含 30-40 个会话的全部 turn）
- **平均检索耗时**: ~0.0026s / 查询
- **总评估时间**: < 60 秒（500 条实例全部完成）

### 运行方式

```bash
cd LongMemEval
# 下载数据集（如未下载）
python download_data.py

# 使用 LRC 的搜索 API 进行评估
# 详细评测脚本请参考 benchmarks/scripts/lrc_fair_eval_v3.py
```

> 完整的 LongMemEval 公平对比报告请参考 [LRC_LONGMEMEVAL_REPORT.md](../benchmarks/reports/LRC_LONGMEMEVAL_REPORT.md)。

---

## 基准测试方法论

如需了解 LRC 基准测试的设计哲学、测试维度和实现细节，请参考 [基准测试方法论白皮书](../tests/BENCHMARK_METHODOLOGY.md)。

该文档涵盖：
- 基准测试的设计哲学和测试维度
- 各项指标的测量方法和计算公式
- 测试数据集的生成策略
- 结果分析和解读指南

---

## 注意事项

- 测试前关闭不必要的后台进程，避免干扰 CPU 和磁盘 I/O
- 多次测试取平均值，消除偶然波动
- 大规模测试（百万级）建议在 Linux 环境下进行，文件系统性能更好
- `time` 命令测量的是端到端 HTTP 延迟，包含网络栈开销。更精确的测量可在代码层嵌入计时逻辑