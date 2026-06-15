# Changelog

All notable changes to Loong Recall (LRC) will be documented in this file.

## [0.4.0] — 2026-06-15

### 新增功能

- **桌面端应用 (LRC Desktop v0.2.0)**：基于 Tauri 2 构建的原生桌面应用，支持系统托盘、后台 sidecar 管理、仪表盘内嵌展示
- **项目身份标准化**：通过规范化路径 SHA256 哈希生成项目指纹，实现跨 IDE 的同一项目识别
- **统一数据目录**：采用 `~/.loong-recall/projects/{fingerprint}/data/` 结构隔离不同项目数据
- **数据迁移机制**：自动检测并安全迁移旧版数据到新版目录结构，采用复制策略保护数据安全
- **记忆导出/导入**：支持 JSON 格式的记忆数据导出（项目级/全局）和导入，导入采用追加模式
- **配置向导自动迁移**：已有有效配置时自动跳过首次设置向导，桌面端开箱即用

### 改进

- **端口参数处理**：CLI `--port` 参数优先级高于配置文件，防止端口被意外覆盖
- **Sidecar 端口扫描**：扫描范围扩展至 100 个端口，与服务端 `find_available_port` 逻辑一致
- **仪表盘 API 地址**：自动检测 `window.location.origin` 替代硬编码 `localhost:3099`
- **根路径重定向**：`/` 自动跳转至 `/dashboard`，改善导航体验

### 新增 API

- `GET /api/project/info` — 返回项目指纹、规范路径、源目录信息
- `GET /v1/code/search` — 代码语义搜索（支持多关键词、分页）
- `GET /v1/memories/stats` — 记忆统计

### 验证

- **端到端测试**: 8/8 全部通过（健康检查、仪表盘(200 OK)、项目信息(fingerprint: 8fec0647)、代码搜索(129,767 索引块)、记忆统计(74条)、系统健康、数据位置(本地存储)、CLI导出(88KB JSON)）
- 服务连续运行超过 5 天零故障，端口 3099 稳定
- 数据存储于 `~/.loong-recall/projects/{fingerprint}/data/`，与 IDE 安装目录解耦

---

## [0.3.1] — 2026-05-31

### 新增功能

- LLM 可视化配置界面
- 自动打开浏览器功能
- 桌面端 Agent 全面支持

### 修复

- 添加 [workspace] 声明防止 Cargo 工作区冲突

---

## [0.3.0] — 2026-05-15

### 新增功能

- 桌面端 Agent 检测与配置
- L2 独立贡献量化测试 (Ablation Study)
- 长期同步机制和外部测试套件