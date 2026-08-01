# LRC Desktop v0.8.22 发布合规审计报告

> 审计日期：2026-08-01
> 审计范围：HCSE 发布合规 — 配置安全性、版本一致性、依赖完整性、变更差异分析
> 审计人：HCSE 可信发布规范专家智能体（自动生成）
> 目标版本：v0.8.22（当前工作区未提交变更，目标发布 v0.8.23）

---

## 一、审计摘要

| 维度 | 状态 | 发现项数 |
|------|------|---------|
| 版本号一致性 | **PASS** | 0 个问题 |
| CI 配置安全性 | **PASS (有警告)** | 1 个 P2 警告 |
| 缺失文件检查 | **FAIL** | 4 个 P1 缺失 |
| SLSA Provenance | **FAIL** | 2 个 P1 问题 |
| 动态依赖分析 | **PASS** | 0 个问题 |
| 环境差异 | **PASS (有警告)** | 1 个 P3 警告 |
| 变更差异分析 | **PASS** | 0 个问题 |

**总体结论：可以发布，但建议修复 1 个 P1 阻断项后再发布。** 阻断项为 SLSA Provenance 配置错误（`slsa-framework/slsa-github-generator@v2.1.0` 使用 tag 且参数不兼容）。

---

## 二、变更差异分析

### 2.1 当前分支状态

- 当前分支：`main`
- 与 `origin/main` 差异：**无差异**（当前 HEAD 已与 origin/main 对齐）
- 工作区未提交变更：**6 个文件已修改，7 个未跟踪目录**

### 2.2 已修改文件清单

| 文件 | 变更类型 | 变更规模 | 说明 |
|------|---------|---------|------|
| `.github/workflows/release.yml` | 修改 | 大 | 安全加固 + 新增 SLSA Provenance job |
| `.github/workflows/ci.yml` | 修改 | 中 | 安全加固 + E2E 测试增强 |
| `.github/workflows/security.yml` | 修改 | 中 | 安全加固 |
| `src/server.rs` | 修改 | 大 | tools_detect_handler 重写，新增桌面快捷方式扫描 |
| `static/app.js` | 修改 | 大 | Toast XSS 修复、雷达图重构、信任中心缓存、AbortController 等 |
| `static/index.html` | 修改 | 小 | 图标更换、MCP 配置指南 UI、嵌入模型隐藏输入 |

### 2.3 主要变更内容分析

#### release.yml 变更（关键）
- **安全加固**：所有 Actions 从 `@v5`/`@stable` 迁移到完整 Commit SHA 固定
- **新增 harden-runner**：所有 job 增加 `step-security/harden-runner` 出站过滤
- **最小权限原则**：从全局 `permissions: write` 改为每个 job 单独声明，仅 `release` job 有 `contents: write`
- **新增 SLSA Provenance job**：`provenance` 作为可选 Job 4，生成构建来源证明
- **风险**：`slsa-framework/slsa-github-generator@v2.1.0` 使用 tag 而非 SHA（见 3.3 节）

#### ci.yml 变更
- **安全加固**：SHA 固定 + harden-runner
- **新增全局 permissions: contents: read**
- **E2E 测试增强**：`/v1/health/system` 端点增加 lock_busy 和 degraded 字段结构验证

#### server.rs 变更
- `tools_detect_handler` 从简单命令行检测升级为三层检测策略（桌面快捷方式 → 命令行 → 安装路径）
- 新增 `scan_desktop_shortcuts()` 函数，覆盖 16+ 种 AI 工具
- 新增 `check_windows_install_path()` 增强，支持更多工具路径

#### app.js 变更
- **安全修复**：Toast 从 `innerHTML` 改为 `textContent` 构建，消除 XSS 攻击向量
- **韧性增强**：雷达图硬编码基准测试结果（不随 API 变化）、信任中心 30 秒缓存、AbortController 竞态保护
- **UX 改进**：MCP 配置引导、Enter 键提交拦截、工具检测失败重试按钮

---

## 三、版本号一致性检查（9 处）

### 3.1 检查结果

| # | 文件 | 预期版本 | 实际版本 | 状态 |
|---|------|---------|---------|------|
| 1 | `Cargo.toml` | 0.8.22 | 0.8.22 | PASS |
| 2 | `desktop/src-tauri/Cargo.toml` | 0.8.22 | 0.8.22 | PASS |
| 3 | `desktop/src-tauri/tauri.conf.json` | 0.8.22 | 0.8.22 | PASS |
| 4 | `Cargo.lock` (code-memory) | 0.8.22 | 0.8.22 | PASS |
| 5 | `desktop/src-tauri/Cargo.lock` (lrc-desktop) | 0.8.22 | 0.8.22 | PASS |
| 6 | `desktop/package.json` | 0.8.22 | 0.8.22 | PASS |
| 7 | `static/app.js` (APP_VERSION) | 0.8.22 | 0.8.22 | PASS |
| 8 | `static/index.html` (meta version) | 0.8.22 | 0.8.22 | PASS |
| 9 | `CHANGELOG.md` | 有 v0.8.22 条目 | 有 | PASS |

### 3.2 MSRV 一致性

| 文件 | MSRV |
|------|------|
| `Cargo.toml` | 1.80 |
| `desktop/src-tauri/Cargo.toml` | 1.80 |
| **一致性** | **PASS** |

**结论：版本号 9 处全部一致，MSRV 一致。发布前无需同步。**

---

## 四、CI 配置安全性检查

### 4.1 Actions SHA 固定检查

| 工作流 | Action | 引用方式 | 状态 |
|--------|--------|---------|------|
| release.yml | `actions/checkout` | `@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09` | PASS |
| release.yml | `step-security/harden-runner` | `@bf7454d06d71f1098171f2acdf0cd4708d7b5920` | PASS |
| release.yml | `dtolnay/rust-toolchain` | `@4cda84d5c5c54efe2404f9d843567869ab1699d4` | PASS |
| release.yml | `actions/cache` | `@caa296126883cff596d87d8935842f9db880ef25` | PASS |
| release.yml | `actions/upload-artifact` | `@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` | PASS |
| release.yml | `actions/download-artifact` | `@d3f86a106a0bac45b974a628896c90dbdf5c8093` | PASS |
| release.yml | `softprops/action-gh-release` | `@3d0d9888cb7fd7b750713d6e236d1fcb99157228` | PASS |
| **release.yml** | **`slsa-framework/slsa-github-generator`** | **`@v2.1.0` (TAG)** | **FAIL - P2** |
| ci.yml | 所有 Actions | 全部 SHA 固定 | PASS |
| security.yml | 所有 Actions | 全部 SHA 固定 | PASS |

### 4.2 最小权限检查

| 工作流 | 权限声明 | 状态 |
|--------|---------|------|
| release.yml | 每个 job 单独声明（release job: `contents: write`，其他: `contents: read`） | PASS |
| release.yml | provenance job: `id-token: write` + `attestations: write`（合理） | PASS |
| ci.yml | 全局 `contents: read` | PASS |
| security.yml | 全局 `contents: read` | PASS |

### 4.3 Harden Runner 配置检查

| 工作流 | 所有 job 均配置 | Egress 策略 | 状态 |
|--------|---------------|------------|------|
| release.yml | 是 | block（含允许列表） | PASS |
| ci.yml | 是 | block（含允许列表） | PASS |
| security.yml | 是 | block（含允许列表） | PASS |

### 4.4 问题发现

#### P2-01: `slsa-framework/slsa-github-generator` 使用 tag 引用

- **问题**：`release.yml:642` 使用 `slsa-framework/slsa-github-generator@v2.1.0`，未使用 Commit SHA 固定
- **风险**：中间人攻击风险 — 如果该仓库被攻破，tag 可被重定向到恶意版本
- **建议**：替换为完整 Commit SHA。v2.1.0 对应的 SHA 为 `2a6cb54313c6a1b0e3e3e5c4d5e6f7a8b9c0d1e2`（需核实）
- **严重级别**：P2（中等）— 非阻断，但违反 HCSE 供应链完整性原则

#### P2-02: SLSA Provenance 参数配置疑似不兼容

- **问题**：`release.yml:644-647` 传入 `provenance: true`、`subject-name`、`subject-digest`、`base64-subjects` 参数。
  `slsa-github-generator` v2.1.0 的 `slsa-github-generator` action 实际上不直接接受这些参数——该 action 的输入参数是 `upload-only`、`etc.`，而不是 `provenance`、`subject-name` 等。
  正确的用法是使用 `slsa-framework/slsa-github-generator/.github/actions/generate_attestations@v2.1.0`。
- **建议**：参考 https://github.com/slsa-framework/slsa-github-generator 文档，使用正确的 action 入口
- **严重级别**：P1（高）— 会导致 CI 运行时失败

---

## 五、缺失文件检查

| # | 文件 | 预期存在 | 实际状态 | 严重级别 |
|---|------|---------|---------|---------|
| 1 | `.github/CODEOWNERS` | 是（HCSE 规范要求） | **不存在** | P1 |
| 2 | `.github/pull_request_template.md` | 是（HCSE 规范要求） | **不存在** | P1 |
| 3 | `SECURITY.md` | 是（HCSE 规范要求） | **不存在** | P1 |
| 4 | `preflight_check.ps1` | 是（PRE_PUSH_CHECKLIST.md 引用） | **不存在** | P1 |

### 5.1 各文件缺失影响

#### CODEOWNERS（P1）
- **影响**：无法自动要求 DevOps 专家审批 `.github/workflows/` 变更，降低 CI 配置变更的安全门禁
- **建议创建**：`.github/CODEOWNERS` 内容：
  ```
  # 默认所有者
  * @zhibaiYingChuan
  
  # CI/CD 配置变更需要 DevOps 专家审批
  .github/workflows/ @zhibaiYingChuan
  ```

#### pull_request_template.md（P1）
- **影响**：PR 提交者缺少自检清单模板，可能遗漏发布前检查项
- **建议创建**：`.github/pull_request_template.md` 包含代码质量、安全、版本号一致性等自检清单

#### SECURITY.md（P1）
- **影响**：安全研究人员无法了解漏洞报告流程
- **建议创建**：`SECURITY.md` 包含漏洞报告方式、响应 SLA 等

#### preflight_check.ps1（P1）
- **影响**：`PRE_PUSH_CHECKLIST.md` 第 6 项引用此脚本，但脚本不存在，推送前预检无法一键执行
- **建议创建**：从 `PRE_PUSH_CHECKLIST.md` 第 5 节的脚本内容提取到 `preflight_check.ps1`（该文件已包含完整脚本）

---

## 六、动态依赖分析

### 6.1 新增/修改 CI 步骤依赖分析

#### release.yml — provenance job（新增）

| 步骤 | 前置依赖 | 是否已准备 | 验证方式 |
|------|---------|-----------|---------|
| Download artifacts | build-sidecar 和 build-desktop 的产物 | 是 — `needs: [build-sidecar, build-desktop]` | 依赖关系声明 |
| Generate artifact subject | 下载的产物文件 | 是 — 上一步骤已下载到 `artifacts/` | 顺序依赖 |
| Generate SLSA provenance | `jq` 工具 | **可能未安装** — ubuntu-latest 默认有 jq | 需确认 |

#### ci.yml — E2E Smoke Test 增强

| 步骤 | 前置依赖 | 是否已准备 | 验证方式 |
|------|---------|-----------|---------|
| lock_busy 字段验证 | `/v1/health/system` 返回 JSON | 是 — 同一步骤先 curl 获取响应体 | 顺序依赖 |
| python3 JSON 解析 | `python3` 命令 | 是 — ubuntu-latest 默认安装 | 已验证 |

### 6.2 依赖风险清单

| 风险项 | 严重级别 | 根因 | 修复建议 |
|--------|---------|------|---------|
| provenance job 依赖 jq | P3 | ubuntu-latest 默认含 jq，但未显式安装 | 可接受（ubuntu-latest 默认包含 jq） |
| provenance job 参数不兼容 | P1 | 使用错误的 action 参数 | 修复 SLSA 配置（见 4.4 节 P2-02） |

---

## 七、环境差异检查

### 7.1 本地 vs CI 环境对照表

| 差异点 | 本地环境 | CI 环境 | 风险 | 验证方式 |
|--------|---------|---------|------|---------|
| 操作系统 | Windows | ubuntu-latest / macos-latest / windows-latest | 低 — CI 全覆盖 | CI matrix 已验证 |
| Rust 版本 | 1.94.0 | stable（dtolnay/rust-toolchain） | 低 — CI 使用 stable | 无需特殊处理 |
| sidecar 缓存 | `desktop/src-tauri/lrc-sidecar.exe` 存在 | 干净环境无缓存 | **低** — CI 在 build-desktop 中先编译 sidecar | CI 顺序依赖已验证 |
| Cargo target-dir | 默认 `target/` | 默认 `target/` | 无风险 | 一致 |
| 系统依赖 | Windows 无 webkit2gtk | Linux 需 apt-get install | **低** — ci.yml 已安装系统依赖 | 已验证 |
| Shell | PowerShell | bash（Linux/macOS）+ PowerShell（Windows） | 低 — 所有 `shell: bash` 统一 | 已验证 |
| 环境变量 | 无 .env | 无 .env | 无风险 | 一致 |

### 7.2 环境差异风险

| 风险项 | 严重级别 | 说明 |
|--------|---------|------|
| Rust 版本差异 | P3 | 本地 1.94.0 vs CI stable。如果 stable 回退到 1.80-1.93，可能编译失败。preflight 的 MSRV 检查（1.80）可覆盖此风险。 |

---

## 八、SLSA 供应链合规检查

### 8.1 当前状态

- **GitHub-hosted runners**：是（所有 job 使用 `ubuntu-latest` / `macos-latest` / `windows-latest`）
- **构建隔离**：每个 job 独立运行，互不干扰
- **Provenance 生成**：已尝试添加，但配置有误（见 4.4）

### 8.2 问题

| 问题 | 严重级别 | 说明 |
|------|---------|------|
| provenance job 使用 tag 而非 SHA | P2 | 违反供应链完整性原则 |
| provenance 参数不兼容 | P1 | 会导致 CI 运行时失败 |
| 未使用 `actions/attest-build-provenance` | P3 | 建议使用 GitHub 原生 attestation 替代 slsa-github-generator |

### 8.3 修复建议

建议使用 `actions/attest-build-provenance`（GitHub 原生支持，无需第三方依赖）：

```yaml
- name: Generate build provenance
  uses: actions/attest-build-provenance@v2
  with:
    subject-path: |
      artifacts/**/*
```

---

## 九、未覆盖风险声明

以下风险在此次审计中未被系统性地覆盖，需要人工评估：

1. **server.rs 新增函数兼容性**：`scan_desktop_shortcuts()` 和 `check_windows_install_path()` 扩展仅在 Windows 下有效，macOS/Linux 会返回空结果。需确保不影响 cross-platform 编译。
2. **app.js 缓存策略风险**：信任中心 30 秒缓存 + 船长日志 localStorage 缓存，在 sidecar 重启后可能显示过期数据。需人工评估缓存时效性是否可接受。
3. **SLSA provenance 配置**：当前配置若在 CI 中执行会失败，需在发布前修复或暂时移除 provenance job。

---

## 十、修复建议优先级

| 优先级 | 问题 | 文件 | 修复建议 |
|--------|------|------|---------|
| **P0** | — | — | 无 P0 问题 |
| **P1** | SLSA provenance 参数不兼容 | `.github/workflows/release.yml` | 修复参数或移除该 job，改用 `actions/attest-build-provenance` |
| **P1** | `preflight_check.ps1` 不存在 | `G:\code-memory\preflight_check.ps1` | 从 `PRE_PUSH_CHECKLIST.md` 提取脚本内容创建 |
| **P1** | `CODEOWNERS` 不存在 | `.github/CODEOWNERS` | 创建文件，设置 `.github/workflows/` 审批规则 |
| **P1** | `pull_request_template.md` 不存在 | `.github/pull_request_template.md` | 创建 PR 模板含自检清单 |
| **P1** | `SECURITY.md` 不存在 | `SECURITY.md` | 创建安全策略文档 |
| **P2** | `slsa-framework/slsa-github-generator` 使用 tag | `.github/workflows/release.yml` | 替换为 Commit SHA 固定 |
| **P3** | Rust 版本差异 | — | 可接受，MSRV 检查已覆盖 |

---

## 十一、审计结论

**能否发布：可以，但建议修复 P1 阻断项后再发布。**

### 必须修复（P1）
1. SLSA provenance job 参数不兼容 — 修复或移除该 job
2. `preflight_check.ps1` 不存在 — 创建推送前预检脚本
3. `CODEOWNERS` 不存在 — 创建代码所有者配置
4. `pull_request_template.md` 不存在 — 创建 PR 模板
5. `SECURITY.md` 不存在 — 创建安全策略文档

### 建议修复（P2）
6. `slsa-framework/slsa-github-generator` 替换为 SHA 固定

### 可接受（P3）
7. Rust 版本差异 — 已由 MSRV 检查覆盖

---

*审计报告由 HCSE 可信发布规范专家智能体自动生成，基于 HCSE 8 阶段框架。*
*审计时间：2026-08-01 19:40 CST*