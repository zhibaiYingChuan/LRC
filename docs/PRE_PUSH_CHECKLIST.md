# LRC 推送前预检清单

> v0.8.23 新增 — 每次推送到 GitHub 前必须执行以下检查。
> 与 `PUSH_STANDARD.md` 配合使用，前者定义文件分类，后者定义操作流程。
> 参考 `HCSE_RELEASE_PROTOCOL.md` 获取完整的 CI/CD 环境差异分析。

---

## 一、代码质量检查

| # | 检查项 | 命令 | 通过标准 | 备注 |
|---|--------|------|---------|------|
| 1 | 代码格式 | `cargo fmt --all -- --check` | 无 diff | 与 CI Rustfmt job 对齐 |
| 2 | Clippy 静态检查 | `cargo clippy --features server -- -D warnings` | 无 warning | 拒绝一切 warnings |
| 3 | 编译检查 | `cargo check --features server` | 编译成功 | 确保无编译错误 |
| 4 | 单元测试 | `cargo test --features server` | 全部 PASS | 含集成测试 |
| 5 | 桌面端编译 | `cd desktop/src-tauri && cargo check` | 编译成功 | 确保桌面端可编译 |

## 二、版本号一致性检查（9 处）

| # | 文件 | 版本号 | 检查方式 |
|---|------|--------|---------|
| 1 | `Cargo.toml` | 与 target 一致 | `grep '^version' Cargo.toml` |
| 2 | `desktop/src-tauri/Cargo.toml` | 与 #1 一致 | `grep '^version' desktop/src-tauri/Cargo.toml` |
| 3 | `desktop/src-tauri/tauri.conf.json` | 与 #1 一致 | `grep '"version"' desktop/src-tauri/tauri.conf.json` |
| 4 | `Cargo.lock` | 与 #1 一致 | `grep -A1 'name = "code-memory"' Cargo.lock` |
| 5 | `desktop/src-tauri/Cargo.lock` | 与 #1 一致 | `grep -A1 'name = "lrc-desktop"' desktop/src-tauri/Cargo.lock` |
| 6 | `desktop/package.json` | 与 #1 一致 | `grep '"version"' desktop/package.json` |
| 7 | `static/app.js` (APP_VERSION) | 与 #1 一致（v0.8.25+ 为 fallback，动态版本从后端获取） | `grep "APP_VERSION" static/app.js \| head -1` — 注意匹配 `const APP_VERSION = '0.8.25'` 中的值 |
| 8 | `static/index.html` (meta + 状态栏 + 系统信息) | 与 #1 一致（共 3 处：meta name="version"、#sys-version、#status-version） | `grep 'name="version"' static/index.html` 检查 meta；`grep 'id="sys-version"' static/index.html` 检查系统信息；`grep 'id="status-version"' static/index.html` 检查状态栏 |
| 9 | `CHANGELOG.md` | 新增版本条目 | 确认 CHANGELOG 已更新，且条目格式与现有条目一致 |

**同步命令**（版本号不一致时执行）：
```bash
# 更新 Cargo.lock
cargo check --features server
# 更新 desktop Cargo.lock
cd desktop/src-tauri && cargo check
```

## 三、安全与合规检查

| # | 检查项 | 命令/方法 | 通过标准 |
|---|--------|----------|---------|
| 1 | GH Token 扫描 | `git diff --cached` | 无 secrets 泄露 |
| 2 | 敏感文件检查 | `git status` | 无 `.env`、`credentials.json` 等 |
| 3 | 文件权限 | `git ls-files --stage` | 无可执行权限误设 |
| 4 | 大文件检查 | `git diff --stat` | 无 >1MB 的二进制文件 |
| 5 | 未跟踪文件检查 | `git status --short` | 确认所有未跟踪文件是否应推送 |

## 四、CI/CD 配置检查

| # | 检查项 | 说明 |
|---|--------|------|
| 1 | Actions SHA 固定 | 所有 `uses:` 引用使用 commit SHA，不用 `@v1` 等 tag |
| 2 | Harden Runner 配置 | 所有 CI job 包含 `step-security/harden-runner` |
| 3 | 最小权限原则 | 每个 job 显式声明 `permissions:`，遵循最小权限 |
| 4 | 跨平台兼容 | 检查 `ci.yml` build-matrix 的 Windows/macOS/Linux 编译 |
| 5 | 发布合规 | 检查 `release.yml` preflight 门禁是否完整 |

## 五、预检命令（一键执行）

```powershell
# PowerShell 一键预检脚本
Write-Host "=== 1. 代码格式检查 ===" -ForegroundColor Cyan
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { Write-Host "FAIL: 格式检查未通过" -ForegroundColor Red; exit 1 }

Write-Host "=== 2. Clippy 静态检查 ===" -ForegroundColor Cyan
cargo clippy --features server -- -D warnings
if ($LASTEXITCODE -ne 0) { Write-Host "FAIL: Clippy 未通过" -ForegroundColor Red; exit 1 }

Write-Host "=== 3. 编译检查 ===" -ForegroundColor Cyan
cargo check --features server
if ($LASTEXITCODE -ne 0) { Write-Host "FAIL: 编译检查未通过" -ForegroundColor Red; exit 1 }

Write-Host "=== 4. 单元测试 ===" -ForegroundColor Cyan
cargo test --features server
if ($LASTEXITCODE -ne 0) { Write-Host "FAIL: 单元测试未通过" -ForegroundColor Red; exit 1 }

Write-Host "=== 5. 版本号一致性检查 ===" -ForegroundColor Cyan
$CARGO_VER = (Select-String '^version' Cargo.toml).Line -replace '.*"([^"]+)".*','$1'
$LOCK_VER = (Select-String -Pattern 'name = "code-memory"' -Context 0,1 Cargo.lock | Select-String 'version').Line -replace '.*"([^"]+)".*','$1'
if ($CARGO_VER -ne $LOCK_VER) { Write-Host "FAIL: Cargo.lock 版本号不一致 ($CARGO_VER vs $LOCK_VER)" -ForegroundColor Red; exit 1 }
Write-Host "  版本号一致性: $CARGO_VER OK" -ForegroundColor Green

Write-Host "=== 所有预检检查通过 ===" -ForegroundColor Green
```

## 六、推送前确认

- [ ] 代码质量检查全部通过（1-5）
- [ ] 版本号 9 处一致
- [ ] 无敏感文件泄露
- [ ] CI/CD 配置正确
- [ ] CHANGELOG 已更新
- [ ] 已运行 `preflight_check.ps1`（如存在）