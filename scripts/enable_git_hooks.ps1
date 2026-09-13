#Requires -Version 5.1
# ============================================================
# Loong Recall (LRC) - 启用仓库本地 Git 钩子
# ============================================================
# 作用：将 Git 的 core.hooksPath 指向仓库内的 .githooks/ 目录，
#       使 pre-commit 钩子纳入版本控制、随克隆自动分发。
#
# 背景（代码审查修复 D-11）：原钩子仅存在于 .git/hooks/pre-commit，
#       而 .git/ 不纳入版本控制，导致新克隆环境无钩子保护。
#
# 用法：
#   .\scripts\enable_git_hooks.ps1
# 撤销：
#   git config --local --unset core.hooksPath
#
# 说明：core.hooksPath 为相对路径时，Git 以工作区根目录为基准解析，
#       故写入 '.githooks' 即可，不依赖绝对路径。
#
# 注意：本脚本必须以 UTF-8 with BOM 保存，否则 PS5.1 下中文注释会乱码。
# ============================================================

# 编码一致性保护（PS5.1 与 PS7 兼容）
if ($PSVersionTable.PSVersion.Major -lt 6) {
    chcp 65001 > $null
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

# 错误处理：将非终止错误升级为终止错误，统一交由 try/catch 处置
$ErrorActionPreference = 'Stop'

try {
    # 1. 定位仓库根目录：以脚本自身位置推导，避免依赖当前工作目录
    $scriptDirectory = Split-Path -Path $PSCommandPath -Parent   # ...\scripts
    $repoRoot = Split-Path -Path $scriptDirectory -Parent        # 仓库根
    Write-Host "[1/3] 仓库根目录: $repoRoot" -ForegroundColor Cyan

    # 2. 校验钩子目录与钩子文件存在性（-LiteralPath 避免路径被当作通配符）
    $hooksDirectory = Join-Path -Path $repoRoot -ChildPath '.githooks'
    $hookFile = Join-Path -Path $hooksDirectory -ChildPath 'pre-commit'
    if (-not (Test-Path -LiteralPath $hooksDirectory -PathType Container)) {
        throw "未找到钩子目录: $hooksDirectory"
    }
    if (-not (Test-Path -LiteralPath $hookFile -PathType Leaf)) {
        throw "未找到钩子文件: $hookFile"
    }
    Write-Host "[2/3] 钩子目录校验通过: $hooksDirectory" -ForegroundColor Green

    # 3. 写入仓库本地配置（--local 仅影响当前仓库，不污染全局 Git 配置）
    & git -C $repoRoot config --local core.hooksPath '.githooks'
    if ($LASTEXITCODE -ne 0) {
        throw "git config 执行失败，退出码: $LASTEXITCODE"
    }

    # 4. 回读校验，确认配置已生效（避免"以为写成功"的假阳性）
    $configuredPath = & git -C $repoRoot config --local --get core.hooksPath
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($configuredPath)) {
        throw '回读 core.hooksPath 失败，配置未生效'
    }
    Write-Host "[3/3] 已启用钩子: core.hooksPath = $configuredPath" -ForegroundColor Green

    Write-Host ''
    Write-Host '启用成功。此后 git commit 将自动执行 .githooks/pre-commit（5 项检查）。' -ForegroundColor Green
    Write-Host '撤销方式: git config --local --unset core.hooksPath' -ForegroundColor Yellow
}
catch {
    Write-Host ''
    Write-Host "启用失败: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host '排查建议: 确认当前目录是 Git 仓库，且 .githooks/pre-commit 文件存在。' -ForegroundColor Yellow
    exit 1
}
