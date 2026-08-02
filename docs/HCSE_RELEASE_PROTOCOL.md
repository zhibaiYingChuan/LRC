# HCSE 发布规范：动态差异分析协议

> 本文档是 hcse-release-compliance 智能体的项目级检查清单。
> 全局框架见 `~/.trae-cn/user_rules/hcse-framework.md`。
> 核心原则：**检查项由 git diff 动态生成，而非静态清单**。

---

## 一、调用智能体时的标准 query 模板

调用 `hcse-release-compliance` 智能体时，必须按以下结构组织 query：

```
请对本次发布变更执行 HCSE 动态差异分析。

## 变更范围
git diff origin/main...HEAD --stat 的输出：<粘贴>

## 动态依赖分析要求
1. 对每个新增/修改的 CI 步骤（.github/workflows/*.yml），列出前置依赖并验证
2. 对每个新增/修改的构建步骤（build.rs/tauri.conf.json/Cargo.toml），列出前置依赖并验证
3. 对比本地环境与 CI 环境差异，标注本地有但 CI 无的缓存/文件/工具

## 项目检查清单
参考 docs/HCSE_RELEASE_PROTOCOL.md 的环境差异表和依赖清单

## 输出要求
- 依赖断裂风险清单（按严重级别排序）
- 环境差异风险清单
- 未覆盖的未知风险声明
```

---

## 二、本地 vs CI 环境差异检查表

> 每次发布前必须逐项核对。本地能跑 ≠ CI 能跑。

| 差异点 | 本地环境 | CI 环境 | 风险 | 验证方式 |
|--------|---------|---------|------|---------|
| sidecar 二进制 | `desktop/src-tauri/lrc-sidecar.exe` 缓存存在 | 干净环境无缓存 | cargo check(desktop) panic | CI 创建占位文件（已落地 v0.8.9） |
| cargo target-dir | `~/.cargo/config.toml` 可能配置自定义路径 | 默认 `target/` | build.rs 找不到产物 | build.rs 已有多候选路径搜索 |
| 全局工具链 | rustup 管理多版本 | `dtolnay/rust-toolchain@stable` | MSRV 不匹配 | preflight 检查 MSRV 一致性 |
| Linux 系统依赖 | 本地通常不开发 Linux | CI ubuntu-latest | Tauri 编译缺 libwebkit2gtk | ci.yml 已安装系统依赖 |
| PowerShell vs Bash | 本地 PowerShell | CI Linux 用 bash | hook 脚本兼容问题 | pre-commit hook 已用 bash + exit 0 |
| 环境变量 | 本地可能有 .env | CI 无 .env | 编译/运行时缺变量 | 检查 workflow env 块定义 |

---

## 三、CI 步骤前置依赖清单

> 每次新增/修改 CI 步骤时，必须填写此清单（PUSH_STANDARD.md 16.7 节强制要求）。

| 步骤名 | 前置依赖 | 准备方式 | 已验证版本 |
|--------|---------|---------|-----------|
| cargo check (server) | 无 | — | ✓ v0.7.1 |
| cargo check (desktop) | lrc-sidecar 文件（tauri.conf.json resources glob） | 创建占位文件 | ✓ v0.8.9 |
| Tauri config lint | 无 | — | ✓ v0.8.8 |
| E2E Smoke Test | sidecar 二进制 | cargo build --release | ✓ v0.7.1 |
| build-sidecar (release) | 无 | cargo build --release | ✓ v0.7.1 |
| build-desktop (release) | sidecar 二进制 + 系统依赖 | 先编译 sidecar 并复制 + apt-get install | ✓ v0.8.8 |
| Clippy | 无 | — | ✓ v0.7.1 |
| Rustfmt | 无 | — | ✓ v0.8.8 |

---

## 四、动态差异分析流程

### 4.1 提取变更差异

```powershell
# CI 配置变更
git diff origin/main...HEAD -- .github/workflows/

# 构建配置变更
git diff origin/main...HEAD -- build.rs desktop/src-tauri/build.rs desktop/src-tauri/tauri.conf.json Cargo.toml desktop/src-tauri/Cargo.toml

# pre-commit hook 变更（虽不在 git 跟踪，但需人工检查）
# 检查 .git/hooks/pre-commit 是否修改
```

### 4.2 对每个新增/修改的步骤执行依赖分析

对 diff 中每个新增的 `run:` 命令或构建步骤：

1. **文件依赖**：命令引用了哪些文件路径？这些文件在 CI 干净环境是否存在？
2. **环境依赖**：命令依赖哪些环境变量？是否在 workflow `env` 块定义？
3. **顺序依赖**：命令是否依赖前置步骤的输出？前置步骤是否保证产出？
4. **缓存依赖**：本地能跑是否因为依赖了本地缓存？CI 干净环境是否有等价准备？

### 4.3 输出风险清单

```
| 风险项 | 严重级别 | 根因 | 修复建议 |
|--------|---------|------|---------|
| 新增 cargo check(desktop) 缺 sidecar | P0 | tauri resources glob 要求文件存在 | 创建占位文件 |
```

---

## 五、发布前检查流程（三层门禁）

### 门禁 1：本地 pre-commit hook
- cargo fmt --check
- cargo clippy -D warnings
- cargo check --features server
- cargo test
- 算法泄露检测
- **新增（v0.8.9）**：hook 末尾 exit 0（防 echo Bad fd 导致退出码 1）

### 门禁 2：CI（ci.yml）
- Rustfmt / Clippy / Unit Tests / E2E Smoke Test
- 跨平台 Build Check（三平台）
- **新增（v0.8.9）**：cargo check(desktop) 前创建占位 sidecar
- Tauri config lint（禁止 ["nsis"] 单平台）

### 门禁 3：Release preflight（release.yml）
- 7 项检查：fmt + clippy + check + test + tauri配置 + MSRV一致性 + 版本号一致性
- build-sidecar/build-desktop 依赖 preflight 通过

---

## 六、故障复盘规则

任何 CI 故障事后分析必须回答：

1. **故障模式分类**：是否属于已知失败模式？是否属于基础设施故障？
2. **门禁覆盖**：三层门禁是否应拦截？为什么没拦截？
3. **检查清单更新**：是否需要在本文档新增检查项？
4. **智能体盲区**：hcse-release-compliance 调用时是否缺少相关上下文？
5. **基础设施故障**：故障是否由 GitHub Actions runner 基础设施引起？
   - 如果是，是否有重试/容错机制？是否需要新增重试规则？

---

## 七、历史故障案例索引

| 版本 | 故障 | 根因 | 修复 | 检查项更新 |
|------|------|------|------|-----------|
| v0.8.7 | macOS/Linux 无 bundle | tauri targets=["nsis"] | 改 "all" | 新增 Tauri config lint |
| v0.8.8 | 三平台 Build Check 全挂 | cargo check(desktop) 缺 sidecar | 创建占位文件 | 新增 CI 步骤前置依赖清单 |
| v0.8.9 | pre-commit hook exit 1 | echo Bad fd in PowerShell | exit 0 | hook 末尾强制 exit 0 |
| v0.8.23 | Windows Build Check DNS 失败 | Windows runner DNS 缓存过期，无法解析 github.com | 添加 checkout 重试机制 | 见第八节 |
| v0.8.23 | Ubuntu Build Check exit 143 | apt-get install 无超时选项，挂起被 SIGTERM 杀死 | 添加 apt-get 超时选项 + --fix-missing | 见第八节 |
| v0.8.24 | Windows Build Sidecar git clone 失败 | Windows runner TCP 连接重置 / fetch-pack 损坏 | 升级 checkout 重试为 3 次 + 30 秒等待 | 见第八节 |

---

## 八、基础设施故障模式（v0.8.24 新增）

> 本节记录由 GitHub Actions runner 基础设施引起的故障模式及应对策略。
> 智能体调用时，必须检查这些模式是否已正确处理。

### 8.1 Windows runner 网络不稳定

**故障表现**（GitHub Actions Windows runner 偶发）：
- `fetch-pack: invalid index-pack output`
- `early EOF`
- `RPC failed; curl 56 Recv failure: Connection was reset`
- `Could not resolve host: github.com`

**根因**：Windows runner 到 github.com 的底层 TCP 连接不稳定，DNS 解析偶发失败。

**强制规则**：
- 所有在 Windows runner 上运行的 job，其 `actions/checkout` 步骤必须添加重试机制
- 重试机制必须满足：最多 3 次，每次等待 ≥ 30 秒
- 失败后必须清理 `.git` 目录再重试（防止 partial checkout 干扰）

**标准实现模板**：
```yaml
- name: Checkout
  id: checkout
  uses: actions/checkout@v5
  continue-on-error: true
- name: Checkout (retry on failure)
  if: steps.checkout.outcome == 'failure'
  shell: pwsh
  run: |
    $maxRetries = 3; $retryDelay = 30
    for ($i = 1; $i -le $maxRetries; $i++) {
      Start-Sleep -Seconds $retryDelay
      Remove-Item -Recurse -Force ".git" -ErrorAction SilentlyContinue
      git clone "https://github.com/$env:GITHUB_REPOSITORY.git" .
      git checkout $env:GITHUB_SHA
      if ($LASTEXITCODE -eq 0) { return }
    }
    throw "Checkout 失败：已重试 $maxRetries 次"
```

### 8.2 Ubuntu apt-get 挂起

**故障表现**：
- `Process completed with exit code 143`（SIGTERM）
- job 在 apt-get install 步骤无限期挂起后被杀死

**根因**：apt-get install 默认无超时选项，下载某个包挂起时进程永不退出。

**强制规则**：
- 所有 `apt-get install` 命令必须添加 `-o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30`
- 必须添加 `--fix-missing` 防止单个包下载失败阻塞整个安装
- `apt-get update` 也必须添加相同超时选项
