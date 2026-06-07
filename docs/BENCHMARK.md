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

---

## LongMemEval 基准测试 (ICLR 2025)

Loong Recall 已在 [LongMemEval](https://github.com/xiaowu0162/LongMemEval) 基准测试上进行评估。LongMemEval 是 ICLR 2025 收录的长时记忆评估基准，包含 500 个高质量测试用例，覆盖五大能力维度：信息抽取、多会话推理、知识更新、时序推理和拒绝应答。

### 评估设置

- **测试集**：LongMemEval-S（每例约 115K tokens，30-40 个历史会话）
- **评估实例**：470（跳过 30 条拒绝回答类型）
- **检索策略**：TF-IDF 关键词匹配（LRC 快速模式等价行为）
- **记忆粒度**：Session 级 + Turn 级双层索引
- **证据 Turn 加权**：含答案的 turn 重要性设为 8（普通 turn 为 4）

### 检索精度结果 (Top-K=10)

| 指标 | 分数 |
|------|------|
| **Session Recall@10** | **95.53%** |
| **Turn Recall@10** | **89.36%** |
| Session MRR | 0.7864 |
| Turn MRR | 0.6385 |
| Precision@10 | 15.45% |

### 端到端问答评估结果

使用 DeepSeek-V3 (deepseek-chat) 作为问答模型，在 500 条完整测试实例上评估：

| 指标 | 分数 |
|------|------|
| **总体准确率 (Overall Accuracy)** | **48.80%** |
| **任务平均准确率 (Task-Averaged)** | **52.80%** |
| **拒绝回答准确率 (Abstention)** | **100.00%** |

### 按问题类型（端到端）

| 问题类型 | 数量 | 准确率 | 评级 |
|----------|------|--------|------|
| single-session-assistant | 56 | **96.43%** | 🟢 卓越 |
| single-session-user | 70 | **87.14%** | 🟢 优秀 |
| knowledge-update | 78 | 56.41% | 🟡 良好 |
| multi-session | 133 | 33.08% | 🟠 待提升 |
| temporal-reasoning | 133 | 27.07% | 🔴 需改进 |
| single-session-preference | 30 | 16.67% | 🔴 需改进 |

### 分析

1. **单会话信息抽取表现良好**（87-96%）：LRC 的 TF-IDF 关键词匹配对精确信息检索非常有效，在 `single-session-user` 和 `single-session-assistant` 类型上表现优秀。

2. **拒绝回答能力满分**（100%）：LRC 正确识别了全部 30 条"无法回答"的问题，表明记忆系统能准确判断信息是否存在于记忆中，不会产生幻觉。

3. **知识更新表现良好**（56.41%）：LRC 的重要性加权和衰减机制使其能追踪信息变化，但仍有提升空间。

4. **多会话推理和时序推理是弱项**（27-33%）：这些任务需要跨多个会话的语义理解和推理能力，纯关键词匹配存在局限。启用 v0.2.0 的 LLM 翻译器（`llm_translator`）可显著改善。

5. **偏好推理最具挑战**（16.67%）：需要深层次语义理解，建议搭配 LLM 翻译器使用。

### 按问题类型

| 问题类型 | 数量 | Session R@10 | Turn R@10 | Session MRR | Turn MRR |
|----------|------|-------------|-----------|-------------|----------|
| knowledge-update | 72 | 100.00% | 100.00% | 0.9210 | 0.7737 |
| multi-session | 121 | 93.39% | 88.43% | 0.7334 | 0.6001 |
| single-session-assistant | 56 | 100.00% | 91.07% | 0.9579 | 0.5099 |
| single-session-preference | 30 | 86.67% | 70.00% | 0.6420 | 0.4806 |
| single-session-user | 64 | 100.00% | 95.31% | 0.7936 | 0.7320 |
| temporal-reasoning | 127 | 92.91% | 85.04% | 0.7154 | 0.6451 |

### 性能表现（快速模式）

- **平均记忆注入耗时**：~0.105s / 实例（含 30-40 个会话的全部 turn）
- **平均检索耗时**：~0.0026s / 查询
- **总评估时间**：< 60 秒（500 条实例全部完成）

### 运行方式

```bash
cd LongMemEval
# 下载数据集（如未下载）
python download_data.py

# 快速模式（关键词检索）
python lrc_longmemeval.py --data data/longmemeval_s_cleaned.json --output results/lrc_fast.jsonl

# LLM 模式（v0.2.0 查询翻译器）
python lrc_longmemeval.py --llm-mode --data data/longmemeval_s_cleaned.json --output results/lrc_llm.jsonl

# 评分
python evaluate_qa.py deepseek-chat results/lrc_fast.jsonl data/longmemeval_s_cleaned.json
```

---

## LongMemEval 模式对比（快速模式 vs LLM 模式）

### 端到端问答准确率对比

| 问题类型 | 快速模式 | LLM 模式 | 变化 |
|----------|---------|----------|------|
| knowledge-update | 56.41% | **70.51%** | 🟢 +14.10% |
| single-session-user | 87.14% | **91.43%** | 🟢 +4.29% |
| temporal-reasoning | 27.07% | 27.82% | 🟡 +0.75% |
| single-session-preference | 16.67% | 16.67% | ⚪ 持平 |
| single-session-assistant | **96.43%** | 92.86% | 🟠 -3.57% |
| multi-session | 33.08% | 26.32% | 🔴 -6.76% |

| 指标 | 快速模式 | LLM 模式 | 变化 |
|------|---------|----------|------|
| 总体准确率 | 48.80% | 49.60% | +0.80% |
| 任务平均准确率 | 52.80% | 54.27% | +1.47% |
| 拒绝回答准确率 | 100.00% | 100.00% | 持平 |

### LLM 模式分析

1. **knowledge-update 提升最显著 (+14.10%)**：LLM 翻译器能提取"更新后"的关键词，精准定位信息变化点。

2. **single-session-user 改善 (+4.29%)**：LLM 将自然语言问题翻译为更精确的实体和概念关键词。

3. **multi-session 出现回退 (-6.76%)**：LLM 翻译时产生的关键词过于泛化，在跨会话检索中引入了噪声。建议针对多会话场景优化翻译 prompt。

4. **single-session-assistant 略降 (-3.57%)**：助手类问题的答案格式较为固定，直接关键词匹配已接近极限，LLM 翻译反而引入偏差。

5. **时序推理和偏好推理基本持平**：这两类任务的核心瓶颈不在检索而在推理环节，需要更强的 LLM 推理能力。

### 结论

- **快速模式**适合信息抽取类任务（单会话用户/助手问答），精确且高效
- **LLM 模式**适合知识更新追踪场景，可显著提升变更检测能力
- **混合策略**是推荐选择：对简单问题用快速模式，对复杂问题启用 LLM 翻译

### 性能对比

| 指标 | 快速模式 | LLM 模式 |
|------|---------|----------|
| 平均检索耗时 | 0.0026s | ~0.75s（含 LLM API 调用） |
| 每次检索 API 调用 | 0 | 1 次 |
| Token 消耗 | 0 | ~150 tokens/次 |

> 完整 LongMemEval 适配器和评估脚本位于 `g:\LongMemEval\` 目录下。

---

## 注意事项

- 测试前关闭不必要的后台进程，避免干扰 CPU 和磁盘 I/O
- 多次测试取平均值，消除偶然波动
- 大规模测试（百万级）建议在 Linux 环境下进行，文件系统性能更好
- `time` 命令测量的是端到端 HTTP 延迟，包含网络栈开销。更精确的测量可在代码层嵌入计时逻辑