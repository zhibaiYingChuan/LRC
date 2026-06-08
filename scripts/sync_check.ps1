# LRC 本地-远程同步检查脚本
# 用途: 每次提交前检查本地是否领先于远程，防止版本割裂
# 用法: .\scripts\sync_check.ps1

param(
    [switch]$AutoFix = $false  # 自动修复：推送本地领先的提交
)

$ErrorActionPreference = "Stop"

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  LRC 本地-远程同步状态检查" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 1. 检查 git 仓库状态
$branch = git rev-parse --abbrev-ref HEAD 2>$null
if (-not $branch) {
    Write-Host "[错误] 当前目录不是 git 仓库" -ForegroundColor Red
    exit 1
}

Write-Host "[1/4] 当前分支: $branch" -ForegroundColor Green

# 2. 拉取远程最新状态
Write-Host "[2/4] 拉取远程最新状态..." -ForegroundColor Yellow
git fetch origin $branch 2>&1 | Out-Null

# 3. 对比本地与远程
$local = git rev-parse HEAD
$remote = git rev-parse "origin/$branch"
$behind = git rev-list --count "HEAD..origin/$branch" 2>$null
$ahead = git rev-list --count "origin/$branch..HEAD" 2>$null

Write-Host "[3/4] 同步状态:" -ForegroundColor Yellow

if ($ahead -eq "0" -and $behind -eq "0") {
    Write-Host "  ✓ 本地与远程完全同步" -ForegroundColor Green
} elseif ($ahead -ne "0" -and $behind -eq "0") {
    Write-Host "  ⚠ 本地领先远程 $ahead 个提交" -ForegroundColor Magenta
} elseif ($ahead -eq "0" -and $behind -ne "0") {
    Write-Host "  ⚠ 远程领先本地 $behind 个提交" -ForegroundColor Yellow
} else {
    Write-Host "  ❌ 本地与远程已分叉！本地领先 $ahead，远程领先 $behind" -ForegroundColor Red
}

# 4. 检查未提交的修改
$modified = git status --porcelain 2>$null
Write-Host "[4/4] 工作区状态:" -ForegroundColor Yellow
if ($modified) {
    Write-Host "  ⚠ 存在未提交的修改:" -ForegroundColor Magenta
    $modified | ForEach-Object { Write-Host "    $_" }
} else {
    Write-Host "  ✓ 工作区干净" -ForegroundColor Green
}

# 5. 自动修复（可选）
if ($AutoFix) {
    Write-Host ""
    Write-Host "--- 自动修复模式 ---" -ForegroundColor Cyan

    if ($ahead -ne "0" -and $behind -eq "0") {
        Write-Host "推送本地领先的 $ahead 个提交..." -ForegroundColor Yellow
        git push origin $branch
        Write-Host "  ✓ 推送完成" -ForegroundColor Green
    }

    if ($behind -ne "0") {
        Write-Host "拉取远程领先的 $behind 个提交..." -ForegroundColor Yellow
        git pull --rebase origin $branch
        Write-Host "  ✓ 拉取完成" -ForegroundColor Green
    }
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  检查完成" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan