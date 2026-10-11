# 修复优化计划 v2.0

> 基于全局动态审计报告 v2.0，由创始者智能体和产品经理联合制定
> 日期：2026-07-28
> 核心目标：修复 P0 阻断问题，编译桌面端，进行真实测试

## 审计报告要点

v2.0 审计共发现 30 个问题，其中 9 个 P0 问题阻断桌面端编译和运行。与 v1.0 的关键差异：
- v1.0 遗漏了桌面端集成问题（sidecar 二进制不存在、frontendDist 错误、版本号不一致）
- v1.0 测试报告虚假（服务未真正运行）
- v1.0 已修复 CORS、路径遍历、3个新增 API（v2.0 确认有效）

## 修复步骤清单（18步，禁止批量修改，分步执行）

### 阶段一：P0 配置阻断问题（步骤 1-4）

| 步骤 | 编号 | 内容 | 文件 | 验证 |
|------|------|------|------|------|
| 1 | P0-B | 修正 frontendDist 指向 | tauri.conf.json | 路径检查 |
| 2 | P0-C | 统一版本号为 0.6.0 | 3个配置文件 | 版本号检查 |
| 3 | P0-G | 收紧 CSP 配置 | tauri.conf.json | 配置检查 |
| 4 | P1-B | 调整窗口尺寸 | tauri.conf.json | 配置检查 |

### 阶段二：P0 代码质量问题（步骤 5-9）

| 步骤 | 编号 | 内容 | 文件 | 验证 |
|------|------|------|------|------|
| 5 | P0-E-a | server.rs expect() 替换为 match（1/3） | server.rs | cargo check |
| 6 | P0-E-b | server.rs expect() 替换为 match（2/3） | server.rs | cargo check |
| 7 | P0-E-c | server.rs expect() 替换为 match（3/3） | server.rs | cargo check |
| 8 | P0-F-a | v1_api.rs consolidate 错误处理 | v1_api.rs | cargo check |
| 9 | P0-F-b | v1_api.rs enrich 错误处理 | v1_api.rs | cargo check |

### 阶段三：P0 桌面端集成（步骤 10-11）

| 步骤 | 编号 | 内容 | 文件 | 验证 |
|------|------|------|------|------|
| 10 | P0-D | main.rs 添加 sidecar 二进制检查 | main.rs | cargo check |
| 11 | P1-A | build.rs release 构建强制检查 | build.rs | cargo check |

### 阶段四：P1 关键问题（步骤 12-14）

| 步骤 | 编号 | 内容 | 文件 | 验证 |
|------|------|------|------|------|
| 12 | P1-D | app.js innerHTML 转义加固 | app.js | 浏览器验证 |
| 13 | P1-E | manager.rs 边界条件处理 | manager.rs | cargo check |
| 14 | P1-G | embedder_apply TOML 转义 | server.rs | cargo check |

### 阶段五：编译与测试（步骤 15-18）

| 步骤 | 编号 | 内容 | 验证 |
|------|------|------|------|
| 15 | 编译 | 关闭 lrcmcp 服务，编译主项目 release | cargo build --release |
| 16 | 编译 | 编译桌面端 | npm run build |
| 17 | 测试 | 启动桌面端，模拟用户真实测试 | 桌面端运行 |
| 18 | 报告 | 形成 TEST_REPORT_v2.0.md | 报告文档 |

## 执行原则

1. **禁止批量修改**：每个步骤独立执行、独立验证
2. **PowerShell 操作**：需启用 PowerShell 专家技能验证
3. **每步验证**：cargo check + 配置检查
4. **真实测试**：必须启动桌面端，模拟用户实际操作

## 关键决策

### 决策1：frontendDist 指向
- **选项A**：指向 `../../static`（主项目静态资源）
- **选项B**：将 static 复制到 desktop/src/
- **选择**：选项A，避免代码重复，主项目 static 是单一来源

### 决策2：expect() 修复策略
- **选项A**：全部替换为 match
- **选项B**：替换为 `?` 操作符
- **选择**：选项A，match 更显式，便于错误处理

### 决策3：版本号统一
- **选择**：全部统一为 0.6.0（与主项目一致）
