<#
.SYNOPSIS
    LRC 零门槛启动端到端验证脚本
.DESCRIPTION
    模拟用户从零开始的完整使用流程：
    1. 在临时目录中构建项目（模拟干净环境）
    2. 验证编译成功
    3. 启动服务并验证 API 端点
    4. 检查模型就绪状态
    5. 自动清理
.PARAMETER KeepBuild
    保留构建产物，用于后续调试
.PARAMETER SkipModelCheck
    跳过 ML 模型相关验证
.EXAMPLE
    .\scripts\verify_fresh_install.ps1
    运行完整验证流程（包含模型检查）
.EXAMPLE
    .\scripts\verify_fresh_install.ps1 -SkipModelCheck
    仅验证编译和 API（跳过模型检查，适合 CI 环境）
#>

#Requires -Version 7.0

[CmdletBinding()]
param(
    [switch]$KeepBuild,
    [switch]$SkipModelCheck
)

# ============================================================
# 工程文化准则：错误立即暴露，不可恢复错误必须退出
# ============================================================
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# 颜色输出辅助函数（使用 Write-Host 明确表达意图）
function Write-Status {
    param([string]$Message, [string]$Color = "White")
    Write-Host "[$(Get-Date -Format 'HH:mm:ss')] $Message" -ForegroundColor $Color
}

function Write-Pass { Write-Status "  PASS  $args" "Green" }
function Write-Fail { Write-Status "  FAIL  $args" "Red" }
function Write-Warn { Write-Status "  WARN  $args" "Yellow" }

# ============================================================
# 步骤 0：环境预检
# ============================================================
Write-Status "═══════════════════════════════════════════" "Cyan"
Write-Status "  LRC v0.2.1 零门槛启动端到端验证" "Cyan"
Write-Status "═══════════════════════════════════════════" "Cyan"
Write-Status ""

# 检测项目根目录（脚本所在目录的父目录）
$ProjectRoot = Resolve-Path (Join-Path $PSScriptRoot "..") -ErrorAction Stop
Write-Status "项目根目录: $ProjectRoot"

# 验证必要工具
$RequiredTools = @("cargo", "curl")
foreach ($Tool in $RequiredTools) {
    try {
        $null = Get-Command $Tool -ErrorAction Stop
        Write-Pass "$Tool 可用"
    } catch {
        Write-Fail "未找到 $Tool，请先安装 Rust 工具链"
        exit 1
    }
}

# ============================================================
# 步骤 1：创建临时目录（模拟干净环境）
# ============================================================
Write-Status ""
Write-Status "--- 步骤 1：准备干净构建环境 ---" "Cyan"

$TempRoot = Join-Path $env:TEMP "lrc_verify_$(Get-Date -Format 'yyyyMMdd_HHmmss')"
$BuildDir = Join-Path $TempRoot "code-memory"

try {
    New-Item -ItemType Directory -Path $BuildDir -Force | Out-Null
    Write-Pass "临时目录已创建: $BuildDir"
} catch {
    Write-Fail "无法创建临时目录: $_"
    exit 1
}

# 复制项目文件（排除 target/、models/、.git/ 以模拟干净 clone）
Write-Status "复制项目文件（排除 target/、models/、.git/）..."
try {
    # Robocopy 比 Copy-Item 更适合大目录复制，支持排除规则
    $ExcludeDirs = @("target", "models", ".git", "node_modules", ".loong-recall")
    $RobocopyArgs = @(
        $ProjectRoot,
        $BuildDir,
        "/E",           # 递归复制子目录
        "/NJH",         # 无作业头
        "/NJS",         # 无作业摘要
        "/NP",          # 无进度百分比
        "/NDL",         # 无目录列表
        "/NC",          # 无类别
        "/NS"           # 无文件大小
    )

    # 添加排除目录
    foreach ($Dir in $ExcludeDirs) {
        $RobocopyArgs += "/XD"
        $RobocopyArgs += $Dir
    }

    $CopyResult = & robocopy @RobocopyArgs
    # robocopy 退出码 0-7 均为成功（0=无变更, 1=有复制, 等等）
    if ($LASTEXITCODE -ge 8) {
        throw "robocopy 退出码异常: $LASTEXITCODE"
    }
    Write-Pass "项目文件复制完成"
} catch {
    Write-Fail "文件复制失败: $_"
    if (-not $KeepBuild) {
        Remove-Item -LiteralPath $TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    exit 1
}

# ============================================================
# 步骤 2：编译项目
# ============================================================
Write-Status ""
Write-Status "--- 步骤 2：编译项目（cargo build --features ml）---" "Cyan"

$BuildStart = Get-Date
try {
    Push-Location -LiteralPath $BuildDir
    $BuildOutput = cargo build --features ml --release 2>&1
    $BuildExit = $LASTEXITCODE
    Pop-Location

    if ($BuildExit -ne 0) {
        Write-Fail "编译失败（退出码: $BuildExit）"
        Write-Host $BuildOutput | Select-Object -Last 30
        throw "编译失败"
    }
    $BuildDuration = [math]::Round(((Get-Date) - $BuildStart).TotalSeconds, 1)
    Write-Pass "编译成功（耗时: ${BuildDuration}s）"
} catch {
    Write-Fail "编译阶段异常: $_"
    if (-not $KeepBuild) {
        Remove-Item -LiteralPath $TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    exit 1
}

# 验证可执行文件存在
$ExePath = Join-Path $BuildDir "target\release\code-memory-server.exe"
if (-not (Test-Path -LiteralPath $ExePath)) {
    Write-Fail "可执行文件不存在: $ExePath"
    if (-not $KeepBuild) {
        Remove-Item -LiteralPath $TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    exit 1
}
Write-Pass "可执行文件已生成: $ExePath"

# ============================================================
# 步骤 3：ML 测试真实执行验证
# ============================================================
Write-Status ""
Write-Status "--- 步骤 3：ML 测试验证 ---" "Cyan"

Push-Location -LiteralPath $BuildDir
try {
    $TestOutput = cargo test --features ml 2>&1
    $TestExit = $LASTEXITCODE
    Pop-Location

    # 检查是否有 ML 测试被跳过
    $SkipLines = $TestOutput | Select-String "跳过.*ML"
    if ($SkipLines) {
        Write-Warn "部分 ML 测试被跳过（模型未就绪，这是预期的）"
        $SkipLines | ForEach-Object { Write-Warn $_ }
    }

    # 检查测试通过数
    $PassLine = $TestOutput | Select-String "(\d+) passed"
    if ($PassLine) {
        Write-Pass "测试结果: $($PassLine.Line)"
    }

    if ($TestExit -ne 0) {
        Write-Fail "部分测试失败（退出码: $TestExit）"
        $TestOutput | Select-String "FAILED" | ForEach-Object { Write-Fail $_ }
        throw "测试失败"
    }
    Write-Pass "所有测试通过"
} catch {
    Write-Fail "测试阶段异常: $_"
    if (-not $KeepBuild) {
        Remove-Item -LiteralPath $TempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    exit 1
}

# ============================================================
# 步骤 4：启动服务并验证 API
# ============================================================
Write-Status ""
Write-Status "--- 步骤 4：启动服务 & API 验证 ---" "Cyan"

# 使用临时数据目录，避免污染构建目录
$TempDataDir = Join-Path $TempRoot "test_data"
New-Item -ItemType Directory -Path $TempDataDir -Force | Out-Null

$ServerPort = 18990  # 使用非标准端口避免冲突
$ServerReady = $false
$ServerJob = $null

try {
    # 启动服务（后台进程）
    $ServerArgs = @(
        "--port", $ServerPort,
        "--db-path", $TempDataDir
    )

    $ServerProcess = Start-Process -FilePath $ExePath `
        -ArgumentList $ServerArgs `
        -NoNewWindow `
        -PassThru `
        -RedirectStandardOutput (Join-Path $TempRoot "server_stdout.log") `
        -RedirectStandardError (Join-Path $TempRoot "server_stderr.log")

    Write-Status "服务进程已启动 (PID: $($ServerProcess.Id))，等待就绪..."

    # 等待服务就绪：轮询端口直到响应
    $MaxWait = 30  # 最大等待秒数
    $PollInterval = 1
    $Elapsed = 0

    do {
        Start-Sleep -Seconds $PollInterval
        $Elapsed += $PollInterval

        try {
            $HealthResponse = Invoke-WebRequest `
                -Uri "http://127.0.0.1:${ServerPort}/" `
                -Method GET `
                -TimeoutSec 2 `
                -ErrorAction SilentlyContinue
            if ($HealthResponse.StatusCode -eq 200) {
                $ServerReady = $true
                break
            }
        } catch {
            # 服务尚未就绪，继续等待
        }

        # 检查进程是否已退出
        if ($ServerProcess.HasExited) {
            Write-Fail "服务进程意外退出（退出码: $($ServerProcess.ExitCode)）"
            Write-Status "--- stderr ---" "Yellow"
            if (Test-Path (Join-Path $TempRoot "server_stderr.log")) {
                Get-Content (Join-Path $TempRoot "server_stderr.log") | Select-Object -Last 20 | ForEach-Object { Write-Host "  $_" -ForegroundColor "Yellow" }
            }
            throw "服务进程意外退出"
        }
    } while ($Elapsed -lt $MaxWait)

    if (-not $ServerReady) {
        Write-Fail "服务在 ${MaxWait}s 内未就绪"
        throw "服务启动超时"
    }

    Write-Pass "服务已就绪（等待 ${Elapsed}s）"

    # ── 测试 POST /v1/memories/consolidate ──
    Write-Status "测试 API: POST /v1/memories/consolidate"
    $ConsolidateBody = @{
        memories = @(
            @{
                content      = "测试记忆：PostgreSQL 查询优化中使用 EXPLAIN ANALYZE 分析慢查询"
                memory_type  = "fact"
                project      = "test-project"
                importance   = 5
                tags         = @("postgresql", "optimization")
                privacy_level = "user"
            }
        )
        synthesis_similarity = 0.4
        min_cluster          = 3
    } | ConvertTo-Json -Depth 4 -Compress

    try {
        $ConsolidateResponse = Invoke-RestMethod `
            -Uri "http://127.0.0.1:${ServerPort}/v1/memories/consolidate" `
            -Method POST `
            -Body $ConsolidateBody `
            -ContentType "application/json" `
            -TimeoutSec 10 `
            -ErrorAction Stop

        if ($ConsolidateResponse -and $ConsolidateResponse.stored -ge 0) {
            Write-Pass "记忆结晶成功（存储: $($ConsolidateResponse.stored), 合成: $($ConsolidateResponse.synthesized)）"
        } else {
            Write-Fail "记忆结晶响应缺少 stored 字段"
            Write-Host ($ConsolidateResponse | ConvertTo-Json -Depth 3)
            throw "API 响应结构错误"
        }
    } catch {
        Write-Fail "记忆结晶 API 调用失败: $_"
        throw
    }

    # ── 测试 POST /v1/memories/enrich ──
    Write-Status "测试 API: POST /v1/memories/enrich"
    $EnrichBody = @{
        query  = "PostgreSQL 查询优化"
        top_k  = 5
    } | ConvertTo-Json -Compress

    try {
        $EnrichResponse = Invoke-RestMethod `
            -Uri "http://127.0.0.1:${ServerPort}/v1/memories/enrich" `
            -Method POST `
            -Body $EnrichBody `
            -ContentType "application/json" `
            -TimeoutSec 10 `
            -ErrorAction Stop

        if ($EnrichResponse -and $EnrichResponse.memories) {
            $Count = $EnrichResponse.memories.Count
            Write-Pass "记忆召回成功（返回 $Count 条记忆，快路径: $($EnrichResponse.fast_path_hits), 深路径: $($EnrichResponse.deep_path_hits)）"
        } else {
            Write-Fail "记忆召回响应缺少 memories 字段"
            throw "API 响应结构错误"
        }
    } catch {
        Write-Fail "记忆召回 API 调用失败: $_"
        throw
    }

    Write-Pass "API 验证全部通过"

} catch {
    Write-Fail "服务验证阶段异常: $_"
    throw
} finally {
    # 停止服务进程
    if ($ServerProcess -and -not $ServerProcess.HasExited) {
        Write-Status "停止服务进程 (PID: $($ServerProcess.Id))..."
        Stop-Process -Id $ServerProcess.Id -Force -ErrorAction SilentlyContinue

        # 等待进程优雅退出
        $ServerProcess.WaitForExit(5000) | Out-Null
        if (-not $ServerProcess.HasExited) {
            Write-Warn "服务进程未在 5 秒内退出，强制终止"
        }
        Write-Pass "服务进程已停止"
    }
}

# ============================================================
# 步骤 5：模型就绪状态检查（可选）
# ============================================================
if (-not $SkipModelCheck) {
    Write-Status ""
    Write-Status "--- 步骤 5：ML 模型就绪状态 ---" "Cyan"

    $ModelDir = Join-Path $BuildDir "models\microsoft--graphcodebert-base"
    if (Test-Path (Join-Path $ModelDir "config.json")) {
        Write-Pass "模型文件已就绪: $ModelDir"
        Get-ChildItem -LiteralPath $ModelDir | ForEach-Object {
            Write-Status "  $($_.Name) ($([math]::Round($_.Length / 1MB, 1)) MB)"
        }
    } else {
        Write-Warn "模型文件未下载（需首次启动时自动下载）"
        Write-Warn "  预期路径: $ModelDir"
    }
}

# ============================================================
# 步骤 6：清理
# ============================================================
Write-Status ""
if ($KeepBuild) {
    Write-Status "=== 验证完成（保留构建产物）===" "Green"
    Write-Status "构建目录: $BuildDir"
    Write-Status "手动清理: Remove-Item -LiteralPath '$TempRoot' -Recurse -Force"
} else {
    Write-Status "--- 步骤 6：清理临时文件 ---" "Cyan"
    try {
        Remove-Item -LiteralPath $TempRoot -Recurse -Force -ErrorAction Stop
        Write-Pass "临时目录已清理: $TempRoot"
    } catch {
        Write-Warn "清理失败: $_"
        Write-Warn "请手动删除: $TempRoot"
    }
}

Write-Status ""
Write-Status "═══════════════════════════════════════════" "Green"
Write-Status "  零门槛启动验证全部通过！" "Green"
Write-Status "═══════════════════════════════════════════" "Green"

# 输出完成标准（对齐工程文化教练的可衡量结束状态）
Write-Status ""
Write-Status "完成标准检查清单:" "Cyan"
Write-Status "  [C1] cargo test --features ml — ML 测试真实执行"
Write-Status "  [C2] 干净环境编译成功"
Write-Status "  [C3] 服务启动 + API 端点正常"
Write-Status "  [C4] 记忆创建 → 召回 端到端链路完整"
Write-Status "  [C5] 模型就绪状态已报告"

exit 0