# ============================================================
# Loong Recall (LRC) Pre-Commit Hook
# ============================================================
# 工程文化教练契约：每次提交前自动运行守门人检查
#
# 用法：
#   手动运行：  .\scripts\pre-commit.ps1
#   Git hook：  拷贝到 .git/hooks/pre-commit（或通过 git config 配置）
#
# 检查项：
#   1. cargo check（编译）
#   2. cargo fmt --check（格式）
#   3. cargo clippy（默认规则，零警告）
#   4. unwrap()/expect() 残留检测（非测试代码）
#
# 注意：pre-commit hook 跳过耗时的 cargo test，测试由 CI 执行。
# ============================================================

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$GatekeeperScript = Join-Path $ScriptDir "gatekeeper.ps1"

if (-not (Test-Path $GatekeeperScript)) {
    Write-Host "[PRE-COMMIT] 守门人脚本不存在: $GatekeeperScript" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Pre-Commit 守门人检查" -ForegroundColor Cyan
Write-Host "  工程文化教练 · 契约优先" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 调用守门人（跳过测试和前端检查，加快提交速度）
& $GatekeeperScript -SkipTests -SkipFrontend -SkipLeakCheck

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "[PRE-COMMIT] 守门人拒绝放行！请修复上述问题后重新提交。" -ForegroundColor Red
    Write-Host "  提示：运行 .\scripts\gatekeeper.ps1 -Fix 可自动修复部分问题" -ForegroundColor Yellow
    exit 1
}

Write-Host ""
Write-Host "[PRE-COMMIT] 守门人放行，提交允许。" -ForegroundColor Green
exit 0