# Loong Recall (LRC) v0.5.0 Windows 一键安装脚本
# 用法: irm https://raw.githubusercontent.com/zhibaiYingChuan/LRC/main/scripts/install.ps1 | iex
# 或本地运行: .\scripts\install.ps1

param(
    [string]$InstallPath = "$env:USERPROFILE\.lrc",
    [switch]$SkipBuild,
    [string]$Version = "latest"
)

$ErrorActionPreference = "Stop"
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Loong Recall (LRC) v0.5.0 安装程序" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 检查 Rust 工具链
Write-Host "[1/5] 检查 Rust 工具链..." -ForegroundColor Yellow
try {
    $rustVersion = rustc --version 2>&1
    Write-Host "  Rust 已安装: $rustVersion" -ForegroundColor Green
} catch {
    Write-Host "  Rust 未安装，请先安装 Rust: https://rustup.rs" -ForegroundColor Red
    Write-Host "  运行: winget install Rustlang.Rustup" -ForegroundColor Yellow
    exit 1
}

# 检查 cargo 版本
$cargoVersion = cargo --version 2>&1
Write-Host "  Cargo 已安装: $cargoVersion" -ForegroundColor Green

# 创建安装目录
Write-Host "[2/5] 创建安装目录..." -ForegroundColor Yellow
New-Item -ItemType Directory -Path $InstallPath -Force | Out-Null
Write-Host "  安装目录: $InstallPath" -ForegroundColor Green

# 克隆仓库
Write-Host "[3/5] 下载 LRC 源码..." -ForegroundColor Yellow
$repoPath = Join-Path $InstallPath "repo"
if (Test-Path $repoPath) {
    Write-Host "  仓库已存在，正在更新..." -ForegroundColor Yellow
    Push-Location $repoPath
    git pull origin main 2>&1 | Out-Null
    Pop-Location
} else {
    git clone https://github.com/zhibaiYingChuan/LRC.git $repoPath 2>&1 | Out-Null
}
Write-Host "  源码已下载到: $repoPath" -ForegroundColor Green

# 编译
if (-not $SkipBuild) {
    Write-Host "[4/5] 编译 LRC（可能需要几分钟）..." -ForegroundColor Yellow
    Push-Location $repoPath
    cargo build --release --features server 2>&1 | Out-Null
    Pop-Location
    Write-Host "  编译完成" -ForegroundColor Green
}

# 配置环境变量
Write-Host "[5/5] 配置环境..." -ForegroundColor Yellow
$binPath = Join-Path $repoPath "target\release"
$exePath = Join-Path $binPath "code-memory.exe"

if (-not (Test-Path $exePath)) {
    Write-Host "  编译产物未找到，请检查编译是否成功" -ForegroundColor Red
    exit 1
}

# 添加到 PATH（用户级）
$currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($currentPath -notlike "*$binPath*") {
    [Environment]::SetEnvironmentVariable("Path", "$currentPath;$binPath", "User")
    Write-Host "  已添加到 PATH: $binPath" -ForegroundColor Green
}

# 创建 lrc 命令别名
$lrcScript = @"
@echo off
"$exePath" %*
"@
$lrcScriptPath = Join-Path $InstallPath "lrc.cmd"
Set-Content -Path $lrcScriptPath -Value $lrcScript

# 确保 lrc 命令在 PATH 中
if ($currentPath -notlike "*$InstallPath*") {
    $updatedPath = [Environment]::GetEnvironmentVariable("Path", "User")
    [Environment]::SetEnvironmentVariable("Path", "$updatedPath;$InstallPath", "User")
    Write-Host "  已添加 lrc 命令到 PATH" -ForegroundColor Green
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Green
Write-Host "  安装完成！" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
Write-Host ""
Write-Host "  使用方式:" -ForegroundColor White
Write-Host "    lrc --http                   启动 HTTP 服务" -ForegroundColor Cyan
Write-Host "    lrc --src-dir .             启动 MCP 模式" -ForegroundColor Cyan
Write-Host "    lrc --version               查看版本" -ForegroundColor Cyan
Write-Host ""
Write-Host "  仪表盘: http://localhost:3099/dashboard" -ForegroundColor Cyan
Write-Host ""
Write-Host "  PATH 已更新，请重新打开终端后使用。" -ForegroundColor Yellow
Write-Host ""