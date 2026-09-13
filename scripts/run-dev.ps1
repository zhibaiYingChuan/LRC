<#
.SYNOPSIS
    LRC 开发版启动脚本 — 双实例隔离方案（【死规则】不允许影响稳定版）
    启动一个独立的 LRC 实例（端口 3100，数据目录与稳定版隔离），
    用于日常开发测试，不影响稳定版（桌面端 3099 端口）。

.DESCRIPTION
    【死规则】任何开发操作都不得影响稳定版的运行。必须使用本脚本维护双实例隔离。
    
    稳定版（桌面端）: 端口 3099, 数据目录 ~/.loong-recall/global/data/, 进程 PID 8296
    开发版（本脚本）: 端口 3100, 数据目录 ~/.loong-recall/dev/data/, 独立进程

    使用方式:
    1. 直接运行: .\scripts\run-dev.ps1
    2. 构建后运行: .\scripts\run-dev.ps1 -Build
    3. 指定端口: .\scripts\run-dev.ps1 -Port 3101

    重要：开发过程中使用本脚本启动的实例进行记忆操作，确保所有变更记录到开发版数据库。
           千万不要操作稳定版的端口（3099）、进程或数据目录。

.NOTES
    作者: LRC Team
    版本: 2.0 - 强制隔离规则版本
#>

param(
    [Alias('p')]
    [int]$Port = 3100,

    [Alias('b')]
    [switch]$Build = $false,

    [Alias('s')]
    [string]$SourceDir = (Get-Location).Path,

    [Alias('d')]
    [string]$DataDir = "$env:USERPROFILE\.loong-recall\dev\data"
)

# ============================================================
# 配置
# ============================================================
$DevPort = $Port
$DevDataDir = $DataDir
$ProjectRoot = $SourceDir
$SidecarExe = Join-Path $ProjectRoot "target\release\lrc-sidecar.exe"
$ServerExe = Join-Path $ProjectRoot "target\release\code-memory-server.exe"
$SidecarDir = Join-Path $ProjectRoot "scripts\sidecar"

# 确保数据目录存在
if (-not (Test-Path $DevDataDir)) {
    New-Item -Path $DevDataDir -ItemType Directory -Force | Out-Null
    Write-Host "[创建] 开发版数据目录: $DevDataDir"
}

# ============================================================
# 停止旧进程
# ============================================================
Write-Host "[检查] 端口 $DevPort 上是否已有 LRC 进程..."
$existingPid = (Get-NetTCPConnection -LocalPort $DevPort -ErrorAction SilentlyContinue).OwningProcess
if ($existingPid) {
    $process = Get-Process -Id $existingPid -ErrorAction SilentlyContinue
    if ($process -and $process.ProcessName -like "*code-memory*") {
        Write-Host "[停止] 旧 LRC 进程 (PID: $existingPid)..."
        Stop-Process -Id $existingPid -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }
}

# ============================================================
# 构建
# ============================================================
if ($Build) {
    Write-Host "[构建] 编译 release 版本..."
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    Set-Location $ProjectRoot
    cargo build --release --features server
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[错误] 编译失败！" -ForegroundColor Red
        exit 1
    }
    Write-Host "[构建] 编译完成" -ForegroundColor Green
}

# ============================================================
# 检查二进制是否存在
# ============================================================
$useCargo = $false
if (-not (Test-Path $ServerExe)) {
    Write-Host "[警告] 未找到预编译二进制: $ServerExe" -ForegroundColor Yellow
    $useCargo = $true
    Write-Host "[提示] 将使用 'cargo run' 启动（首次启动较慢，编译后自动缓存）" -ForegroundColor Yellow
}

# ============================================================
# 启动开发版
# ============================================================
Write-Host "`n============================================" -ForegroundColor Cyan
Write-Host "  启动 LRC 开发版" -ForegroundColor Cyan
Write-Host "  端口: $DevPort" -ForegroundColor Cyan
Write-Host "  数据目录: $DevDataDir" -ForegroundColor Cyan
Write-Host "  源码目录: $ProjectRoot" -ForegroundColor Cyan
Write-Host "============================================`n" -ForegroundColor Cyan

if ($useCargo) {
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    Set-Location $ProjectRoot
    cargo run --features server -- --port $DevPort --data-dir $DevDataDir --global
}
else {
    & $ServerExe --port $DevPort --data-dir $DevDataDir --global
}