# 全局动态审计报告 v1.0

> 审计日期：2026-07-27
> 审计范围：g:\code-memory 全项目（前端 + 后端 + 文档 + 配置）
> 审计视角：前后端API一致性、代码质量、安全性、文档一致性、前端UI交互、配置与构建

---

## 一、审计总览

| 审计维度 | 严重问题 | 中等问题 | 低等问题 | 总计 |
|---------|---------|---------|---------|------|
| 前后端API一致性 | 4 | 2 | 1 | 7 |
| 代码质量 | 2 | 3 | 2 | 7 |
| 安全性 | 3 | 2 | 1 | 6 |
| 文档一致性 | 1 | 3 | 2 | 6 |
| 前端UI交互 | 2 | 4 | 2 | 8 |
| 配置与构建 | 0 | 2 | 1 | 3 |
| **合计** | **12** | **16** | **9** | **37** |

---

## 二、前后端API一致性审计

### 2.1 后端 API 完整清单（19个 v1 端点 + 7个 api 端点 + 12个 MCP 工具）

**v1 REST API 端点（src/v1_api.rs）：**
1. `POST /v1/encode` → encode_handler
2. `POST /v1/memories/consolidate` → consolidate_handler
3. `POST /v1/memories/enrich` → enrich_handler
4. `POST /v1/memories/correct` → correct_handler
5. `POST /v1/memories/unfold` → unfold_handler
6. `GET /v1/health/dao_metrics` → dao_metrics_handler
7. `GET /v1/health/system` → system_health_handler
8. `GET /v1/health/detailed` → detailed_health_handler
9. `POST /v1/feedback` → feedback_handler
10. `GET /v1/audit-trail` → audit_trail_handler
11. `GET /v1/memories/stats` → memories_stats_handler
12. `GET /v1/memories/recent` → recent_memories_handler
13. `GET /v1/trust/data-location` → data_location_handler
14. `GET /v1/trust/network-audit` → network_audit_handler
15. `GET /v1/trust/audit-integrity` → audit_integrity_handler
16. `GET /v1/captains-log` → captains_log_handler
17. `GET /v1/code/search` → code_search_handler
18. `GET /v1/version/check` → version_check_handler
19. `GET /v1/benchmarks/report` → benchmarks_report_handler

**api 端点（src/server.rs）：**
1. `GET /api/config` → config_handler
2. `POST /api/config/llm` → config_llm_handler
3. `GET /api/project/info` → project_info_handler
4. `GET /api/embedder/status` → embedder_status_handler
5. `POST /api/embedder/download` → embedder_download_handler
6. `POST /api/embedder/apply` → embedder_apply_handler
7. `POST /api/embedder/test` → embedder_test_handler
8. `GET /api/tools/detect` → tools_detect_handler

### 2.2 前端调用了但后端缺失的 API（🔴 严重）

| # | 前端调用路径 | 用途 | 严重程度 |
|---|------------|------|---------|
| 1 | `/v1/memories/list` | 导出备份-获取记忆列表 | 🔴 严重 |
| 2 | `/v1/memories/archive` | 导出备份-归档记忆 | 🔴 严重 |
| 3 | `/v1/memories/remember` | 导入备份-写入记忆 | 🔴 严重 |

### 2.3 前端未定义但 HTML 中引用的函数（🔴 严重）

| # | 函数名 | 引用位置 | 严重程度 |
|---|--------|---------|---------|
| 1 | `wizardStep4Search()` | index.html onclick | 🔴 严重 |

### 2.4 后端有但前端未调用的 API（🟡 中等）

| # | API 路径 | 用途 |
|---|---------|------|
| 1 | `POST /v1/feedback` | 用户反馈 |
| 2 | `GET /v1/version/check` | 版本检查 |

---

## 三、代码质量审计

### 3.1 严重问题（🔴）

| # | 位置 | 问题 | 风险 |
|---|------|------|------|
| 1 | src/server.rs 多处 handler | unwrap() 调用可能导致 panic | 服务崩溃 |
| 2 | static/app.js | 多处 getElementById 结果未做 null 检查 | 运行时错误 |

### 3.2 中等问题（🟡）

| # | 位置 | 问题 |
|---|------|------|
| 1 | src/server.rs | embedder handler 中错误处理不够完善 |
| 2 | static/app.js | 异步函数缺少 try-catch |
| 3 | static/app.js | console.log 调试代码残留 |

---

## 四、安全性审计

### 4.1 高风险（🔴）

| # | 位置 | 风险 | 详情 |
|---|------|------|------|
| 1 | embedder_download_handler | 路径遍历 | model_id 虽有白名单，但写入路径构造需验证 |
| 2 | tools_detect_handler | 命令注入 | Command::new 执行外部命令，参数需严格校验 |
| 3 | server.rs CORS | CSRF 风险 | 使用 `CorsLayer::permissive()` 完全宽松 |

### 4.2 中风险（🟡）

| # | 位置 | 风险 |
|---|------|------|
| 1 | config_llm_handler | API Key 可能泄露到日志 |
| 2 | embedder_apply_handler | config.toml 写入路径校验不充分 |

### 4.3 低风险（🟢）

| # | 位置 | 风险 |
|---|------|------|
| 1 | static/app.js | innerHTML 拼接存在潜在 XSS |

---

## 五、文档一致性审计

### 5.1 路线图 vs 实现对照

| 版本 | 功能 | 状态 |
|------|------|------|
| v0.6.0 | 统一 Embedder 抽象层 | ✅ 已完成 |
| v0.6.0 | 默认 BGE-small-zh | ✅ 已完成 |
| v0.6.0 | 下载进度提示与重试 | ✅ 已完成 |
| v0.6.0 | 模型管理 CLI | ✅ 已完成 |
| v0.7.0 | 预设场景模板 | ⚠️ 部分实现 |
| v0.8.0 | 结晶历史持久化 | ⚠️ 部分实现 |
| v0.9.0 | 一键隐私检查 | ⚠️ 未完全实现 |

### 5.2 缺失文档

- CHANGELOG.md 缺失
- 前端页面功能文档缺失
- API 文档未与最新端点同步

---

## 六、前端UI交互审计

### 6.1 严重问题（🔴）

| # | 问题 | 位置 |
|---|------|------|
| 1 | 重复 id 属性 | index.html 多处 |
| 2 | 表单元素缺少 label 关联 | 设置页面 |

### 6.2 中等问题（🟡）

| # | 问题 |
|---|------|
| 1 | 移动端响应式布局不完善 |
| 2 | 缺少 ARIA 可访问性属性 |
| 3 | CSS !important 滥用 |
| 4 | 部分图标 alt 属性为空 |

---

## 七、配置与构建审计

### 7.1 中等问题（🟡）

| # | 问题 |
|---|------|
| 1 | Cargo.toml 可能有未使用的依赖 |
| 2 | 缺少 .env.example 配置模板 |

### 7.2 已有资源确认

- ✅ Dockerfile 存在
- ✅ docker-compose.yml 存在
- ✅ CI/CD 配置存在（.github/workflows/release.yml）
- ✅ models/ 目录存在且有模型文件
- ✅ tests/benchmarks.rs 存在

---

## 八、优先修复排序

### P0 - 阻断性问题（必须立即修复）
1. 前端调用缺失 API：`/v1/memories/list`、`/v1/memories/archive`、`/v1/memories/remember`
2. 未定义函数：`wizardStep4Search()`
3. CORS 安全配置（permissive → 限制域名）

### P1 - 严重问题（本轮迭代修复）
4. embedder handler 路径遍历防护
5. tools_detect_handler 命令注入防护
6. 前端 getElementById null 检查
7. API Key 日志泄露防护

### P2 - 中等问题（后续迭代修复）
8. 未使用 API 的前端对接
9. 文档同步更新
10. 前端可访问性改进
11. console.log 清理
12. CHANGELOG.md 创建
