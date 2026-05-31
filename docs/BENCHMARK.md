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

使用快速模式（默认，零外部依赖）：

```bash
git clone https://github.com/zhibaiYingChuan/LRC.git
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

- **CodeBERT 模式**：启用 `ml` feature 后首次启动需下载模型（~200MB），内存占用增加至 ~500MB
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

## 注意事项

- 测试前关闭不必要的后台进程，避免干扰 CPU 和磁盘 I/O
- 多次测试取平均值，消除偶然波动
- 大规模测试（百万级）建议在 Linux 环境下进行，文件系统性能更好
- `time` 命令测量的是端到端 HTTP 延迟，包含网络栈开销。更精确的测量可在代码层嵌入计时逻辑