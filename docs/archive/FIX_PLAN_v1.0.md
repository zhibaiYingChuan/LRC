# 修复优化计划 v1.0

> 基于全局动态审计报告 v1.0，由创始者智能体和产品经理联合制定
> 日期：2026-07-27

## 审计报告修正

创始者智能体实证验证后发现 2 处误报：
- **P0-2 wizardStep4Search()**：经全代码库搜索，该函数仅在审计报告中出现，index.html 实际无此引用。**降级为无需修复**。
- **P1-5 命令注入**：tools_detect_handler 的 Command::new 参数全部硬编码，无注入风险。真实风险为 PATH 劫持，性质降级。

## 修复步骤清单（15步，禁止批量修改，分步执行）

### 阶段一：P0 阻断性问题（步骤 1-6）

| 步骤 | 编号 | 内容 | 文件 |
|------|------|------|------|
| 1 | P0-1a | 新增 /v1/memories/list 路由 | src/v1_api.rs |
| 2 | P0-1b | 新增 /v1/memories/archive 路由 | src/v1_api.rs |
| 3 | P0-1c | 新增 /v1/memories/remember 路由 | src/v1_api.rs |
| 4 | P0-1d | 前端 backupMemories 契约对齐 | static/app.js |
| 5 | P0-2 | 无需修复（误报澄清） | - |
| 6 | P0-3 | CORS 安全配置收紧 | src/server.rs |

### 阶段二：P1 严重问题（步骤 7-11）

| 步骤 | 编号 | 内容 | 文件 |
|------|------|------|------|
| 7 | P1-4 | embedder 路径遍历防御加固 | src/server.rs |
| 8 | P1-5 | tools_detect PATH 劫持防护 | src/server.rs |
| 9 | P1-6a | app.js getElementById null检查(1) | static/app.js |
| 10 | P1-6b | app.js getElementById null检查(2) | static/app.js |
| 11 | P1-7 | API Key 日志泄露加固 | src/server.rs |

### 阶段三：P2 中等问题（步骤 12-15）

| 步骤 | 编号 | 内容 | 文件 |
|------|------|------|------|
| 12 | P2-8 | 清理 console.log 调试残留 | static/app.js |
| 13 | P2-9 | 异步函数 try-catch 补全 | static/app.js |
| 14 | P2-10 | 修复重复 id="sys-data-dir" | static/index.html + app.js |
| 15 | P2-11 | 文档同步更新 | docs/ |

## 执行原则
1. **禁止批量修改**：每个步骤独立执行、独立验证
2. **PowerShell 操作**：需启用 PowerShell 专家技能验证
3. **每步验证**：cargo build + 前端冒烟测试
