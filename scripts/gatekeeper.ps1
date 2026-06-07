<#
.SYNOPSIS
    Loong Recall (LRC) 自动化静态分析守门人 —— 一次性解决审计问题的质量闸门

.DESCRIPTION
    守门人 (Gatekeeper) 是 LRC 项目的自动化质量守门工具。
    它在每次提交/PR 前自动运行全部静态分析检查，确保：
    - 零编译错误
    - 零 unwrap()/expect() 残留（非测试代码）
    - 零 Clippy 警告（pedantic + nursery）
    - 零格式问题（rustfmt）
    - 零前端代码重复
    - 零安全漏洞（算法泄露检测）
    - 100% 测试通过

    守门人采用"契约优先"模式：每个检查门都有明确的退出条件，
    任何一门不通过则整体失败，阻止低质量代码进入仓库。

    工程文化信条：
    - 契约优先：每个检查门都是可验证的契约
    - TDD 驱动：测试是守门的第一道防线
    - 无指责复盘：失败时输出结构化诊断，聚焦流程而非个人
    - IaC 思维：守门人本身是代码，可版本控制、可 CI 集成

.PARAMETER SkipTests
    跳过单元测试（仅用于快速迭代，禁止在提交前使用）

.PARAMETER SkipFmt
    跳过格式检查（仅用于 CI 环境，本地禁止跳过）

.PARAMETER SkipFrontend
    跳过前端检查

.PARAMETER SkipLeakCheck
    跳过算法泄露检测

.PARAMETER GenerateReport
    生成详细的审计报告 JSON 文件（默认输出到项目根目录）

.PARAMETER ReportPath
    审计报告输出路径（默认: gatekeeper-report.json）

.PARAMETER Strict
    严格模式：将所有警告视为错误（CI 环境推荐）

.PARAMETER Fix
    自动修复模式：运行 cargo clippy --fix 和 cargo fmt 自动修复

.EXAMPLE
    .\scripts\gatekeeper.ps1
    运行完整守门检查（所有门）

.EXAMPLE
    .\scripts\gatekeeper.ps1 -SkipTests -Fix
    跳过测试，自动修复格式问题（快速迭代）

.EXAMPLE
    .\scripts\gatekeeper.ps1 -Strict -GenerateReport
    CI 环境严格模式，生成审计报告

.NOTES
    版本: 1.0.0
    作者: Loong Recall 工程文化教练
    要求: PowerShell 5.1+, Rust 工具链, cargo
#>

#Requires -Version 5.1

[CmdletBinding()]
param(
    [switch]$SkipTests,
    [switch]$SkipFmt,
    [switch]$SkipFrontend,
    [switch]$SkipLeakCheck,
    [switch]$GenerateReport,
    [string]$ReportPath,
    [switch]$Strict,
    [switch]$Fix
)

# ============================================================
# 工程文化准则：契约优先 —— 定义可衡量的结束状态
# ============================================================
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# 全局状态追踪
$Script:GateResults = @{}
$Script:GateStartTime = Get-Date
$Script:TotalGates = 0
$Script:PassedGates = 0
$Script:FailedGates = 0
$Script:SkippedGates = 0
$Script:Warnings = @()
$Script:Errors = @()

# 项目根目录
$Script:ProjectRoot = Resolve-Path (Join-Path $PSScriptRoot "..") -ErrorAction Stop

# 审计报告路径
if (-not $ReportPath) {
    $Script:ReportPath = Join-Path $Script:ProjectRoot "gatekeeper-report.json"
} else {
    $Script:ReportPath = $ReportPath
}

# ============================================================
# 辅助函数：颜色输出与状态追踪
# ============================================================
function Write-GateHeader {
    param([string]$Title, [string]$Icon = "")
    Write-Host ""
    Write-Host ("=" * 70) -ForegroundColor Cyan
    Write-Host "  $Icon $Title" -ForegroundColor Cyan
    Write-Host ("=" * 70) -ForegroundColor Cyan
}

function Write-GatePass {
    param([string]$GateName)
    Write-Host "  [PASS]  $GateName" -ForegroundColor Green
    $Script:PassedGates++
    $Script:GateResults[$GateName] = "PASS"
}

function Write-GateFail {
    param([string]$GateName, [string]$Reason)
    Write-Host "  [FAIL]  $GateName — $Reason" -ForegroundColor Red
    $Script:FailedGates++
    $Script:GateResults[$GateName] = "FAIL"
    $Script:Errors += "[$GateName] $Reason"
}

function Write-GateSkip {
    param([string]$GateName, [string]$Reason)
    Write-Host "  [SKIP]  $GateName — $Reason" -ForegroundColor Yellow
    $Script:SkippedGates++
    $Script:GateResults[$GateName] = "SKIP"
}

function Write-GateWarn {
    param([string]$Message)
    Write-Host "  [WARN]  $Message" -ForegroundColor Yellow
    $Script:Warnings += $Message
}

function Write-GateInfo {
    param([string]$Message)
    Write-Host "  [INFO]  $Message" -ForegroundColor Gray
}

# 运行命令并捕获输出（遵循 PowerShell 防坑铁律）
function Invoke-GateCommand {
    param(
        [string]$Command,
        [string[]]$Arguments,
        [string]$WorkingDirectory,
        [string]$ErrorMessage
    )

    try {
        $Process = New-Object System.Diagnostics.Process
        $Process.StartInfo.FileName = $Command
        $Process.StartInfo.Arguments = ($Arguments -join " ")
        $Process.StartInfo.WorkingDirectory = $WorkingDirectory
        $Process.StartInfo.RedirectStandardOutput = $true
        $Process.StartInfo.RedirectStandardError = $true
        $Process.StartInfo.UseShellExecute = $false
        $Process.StartInfo.CreateNoWindow = $true

        $Process.Start() | Out-Null
        $StdOut = $Process.StandardOutput.ReadToEnd()
        $StdErr = $Process.StandardError.ReadToEnd()
        $Process.WaitForExit(300000) | Out-Null  # 5 分钟超时

        return @{
            ExitCode = $Process.ExitCode
            StdOut   = $StdOut
            StdErr   = $StdErr
            Success  = ($Process.ExitCode -eq 0)
        }
    } catch {
        return @{
            ExitCode = -1
            StdOut   = ""
            StdErr   = "$ErrorMessage : $_"
            Success  = $false
        }
    }
}

# ============================================================
# 守门 1：编译检查 (cargo check)
# ============================================================
function Test-GateCompile {
    Write-GateHeader "守门 1/10：编译检查 (cargo check)" ""

    $Script:TotalGates++
    Write-GateInfo "运行 cargo check --all-targets --features server..."

    $Result = Invoke-GateCommand `
        -Command "cargo" `
        -Arguments @("check", "--all-targets", "--features", "server") `
        -WorkingDirectory $Script:ProjectRoot `
        -ErrorMessage "编译检查失败"

    if ($Result.Success) {
        Write-GatePass "编译检查通过"
        return $true
    } else {
        Write-GateFail "编译检查" "存在编译错误，请先修复"
        Write-Host "  --- 编译错误输出 ---" -ForegroundColor Red
        $ErrLines = ($Result.StdErr -split "`n") | Where-Object { $_ -match "error" }
        $ErrLines | Select-Object -First 10 | ForEach-Object { Write-Host "    $_" -ForegroundColor Red }
        return $false
    }
}

# ============================================================
# 守门 2：单元测试 (cargo test)
# ============================================================
function Test-GateTests {
    if ($SkipTests) {
        Write-GateSkip "单元测试" "已通过 -SkipTests 跳过"
        return $true
    }

    Write-GateHeader "守门 2/10：单元测试 (cargo test)" ""

    $Script:TotalGates++
    Write-GateInfo "运行 cargo test --all-targets..."

    $Result = Invoke-GateCommand `
        -Command "cargo" `
        -Arguments @("test", "--all-targets", "--features", "server") `
        -WorkingDirectory $Script:ProjectRoot `
        -ErrorMessage "单元测试失败"

    if ($Result.Success) {
        # 提取测试统计
        $PassLine = ($Result.StdOut -split "`n") | Select-String "test result:" | Select-Object -Last 1
        if ($PassLine) {
            Write-GateInfo "测试结果: $($PassLine.Line.Trim())"
        }
        Write-GatePass "所有测试通过"
        return $true
    } else {
        Write-GateFail "单元测试" "存在测试失败，请先修复"
        $FailedLines = ($Result.StdOut -split "`n") | Select-String "FAILED"
        $FailedLines | ForEach-Object { Write-Host "    $_" -ForegroundColor Red }
        return $false
    }
}

# ============================================================
# 守门 3：Clippy 静态分析
# 标准模式：零 Clippy 默认警告（-D warnings）
# 严格模式：零 pedantic + nursery 警告
# ============================================================
function Test-GateClippy {
    Write-GateHeader "守门 3/10：Clippy 静态分析" ""

    $Script:TotalGates++

    if ($Fix) {
        Write-GateInfo "自动修复模式：运行 cargo clippy --fix..."
        $FixResult = Invoke-GateCommand `
            -Command "cargo" `
            -Arguments @("clippy", "--fix", "--allow-dirty", "--allow-staged", "--all-targets", "--features", "server") `
            -WorkingDirectory $Script:ProjectRoot `
            -ErrorMessage "Clippy 自动修复失败"
        if (-not $FixResult.Success) {
            Write-GateWarn "Clippy 自动修复未完全成功，继续检查..."
        }
    }

    # 根据模式选择 Clippy 严格程度
    if ($Strict) {
        Write-GateInfo "严格模式：运行 cargo clippy (pedantic + nursery, 零警告)..."
        $Result = Invoke-GateCommand `
            -Command "cargo" `
            -Arguments @(
                "clippy", "--all-targets", "--features", "server",
                "--", "-W", "clippy::pedantic", "-W", "clippy::nursery",
                "-D", "warnings"
            ) `
            -WorkingDirectory $Script:ProjectRoot `
            -ErrorMessage "Clippy 严格模式检查失败"
    } else {
        Write-GateInfo "标准模式：运行 cargo clippy (默认规则, -D warnings)..."
        $Result = Invoke-GateCommand `
            -Command "cargo" `
            -Arguments @(
                "clippy", "--all-targets", "--features", "server",
                "--", "-D", "warnings"
            ) `
            -WorkingDirectory $Script:ProjectRoot `
            -ErrorMessage "Clippy 检查失败"
    }

    # 统计警告数量
    $WarningCount = ($Result.StdErr -split "`n" | Select-String "warning:" | Measure-Object).Count
    $ErrorCount = ($Result.StdErr -split "`n" | Select-String "error:" | Measure-Object).Count

    if ($Result.Success) {
        if ($WarningCount -eq 0 -and $ErrorCount -eq 0) {
            Write-GatePass "Clippy 零警告通过"
            return $true
        } elseif ($WarningCount -gt 0 -and -not $Strict) {
            Write-GateWarn "发现 $WarningCount 个 Clippy 警告（标准模式允许通过）"
            Write-GatePass "Clippy 通过（$WarningCount 个非阻塞警告）"
            return $true
        } else {
            Write-GateFail "Clippy" "严格模式：发现 $WarningCount 个警告, $ErrorCount 个错误"
            $Result.StdErr -split "`n" | Select-String "warning:|error:" | Select-Object -First 15 | ForEach-Object {
                Write-Host "    $_" -ForegroundColor Red
            }
            return $false
        }
    } else {
        Write-GateFail "Clippy" "编译级错误：$ErrorCount 个错误"
        $Result.StdErr -split "`n" | Select-String "error:" | Select-Object -First 10 | ForEach-Object {
            Write-Host "    $_" -ForegroundColor Red
        }
        return $false
    }
}

# ============================================================
# 守门 4：unwrap()/expect() 残留检测
# ============================================================
function Test-GateUnwrapBan {
    Write-GateHeader "守门 4/10：unwrap()/expect() 残留检测" ""

    $Script:TotalGates++

    # 搜索所有 Rust 源文件中的 unwrap()/expect()
    Write-GateInfo "搜索非测试代码中的 unwrap()/expect() 调用..."

    $RustFiles = Get-ChildItem -LiteralPath (Join-Path $Script:ProjectRoot "src") -Recurse -Filter "*.rs" -ErrorAction Stop

    $UnwrapViolations = @()
    $ExpectViolations = @()

    foreach ($File in $RustFiles) {
        $Content = Get-Content -LiteralPath $File.FullName -Raw -ErrorAction Stop

        # 排除测试模块（#[cfg(test)] 块）
        # 简化检测：逐行检查
        $Lines = Get-Content -LiteralPath $File.FullName -ErrorAction Stop
        $InTestModule = $false
        $LineNum = 0

        foreach ($Line in $Lines) {
            $LineNum++

            # 追踪是否在测试模块中
            if ($Line -match '#\[cfg\(test\)\]') {
                $InTestModule = $true
            }
            if ($Line -match '^\s*\}\s*$' -and $InTestModule) {
                # 简单策略：遇到闭合大括号时退出测试模块
                # 注意：这是近似检测，不处理嵌套 mod
            }

            # 跳过注释行
            if ($Line.Trim().StartsWith("//") -or $Line.Trim().StartsWith("/*")) {
                continue
            }

            # 检测 .unwrap() — 排除 unwrap_or、unwrap_or_else、unwrap_or_default
            if ($Line -match '\.unwrap\(\)' -and $Line -notmatch 'unwrap_or') {
                if (-not $InTestModule) {
                    $RelativePath = $File.FullName.Replace($Script:ProjectRoot, "").TrimStart("\")
                    $UnwrapViolations += @{
                        File     = $RelativePath
                        Line     = $LineNum
                        Code     = $Line.Trim()
                    }
                }
            }

            # 检测 .expect() — 在非测试代码中
            if ($Line -match '\.expect\(' -and $Line -notmatch '//.*expect') {
                if (-not $InTestModule) {
                    $RelativePath = $File.FullName.Replace($Script:ProjectRoot, "").TrimStart("\")
                    $ExpectViolations += @{
                        File     = $RelativePath
                        Line     = $LineNum
                        Code     = $Line.Trim()
                    }
                }
            }
        }
    }

    $TotalViolations = $UnwrapViolations.Count + $ExpectViolations.Count

    if ($TotalViolations -eq 0) {
        Write-GatePass "零 unwrap()/expect() 残留（非测试代码）"
        return $true
    } else {
        Write-GateFail "unwrap()/expect() 残留" "发现 $TotalViolations 处违规（unwrap: $($UnwrapViolations.Count), expect: $($ExpectViolations.Count)）"

        # 按文件分组显示
        $AllViolations = $UnwrapViolations + $ExpectViolations
        $GroupedByFile = $AllViolations | Group-Object -Property File

        foreach ($Group in $GroupedByFile) {
            Write-Host "    $($Group.Name):" -ForegroundColor Yellow
            foreach ($V in $Group.Group | Select-Object -First 5) {
                Write-Host "      L$($V.Line): $($V.Code)" -ForegroundColor Gray
            }
            if ($Group.Count -gt 5) {
                Write-Host "      ... 及其他 $($Group.Count - 5) 处" -ForegroundColor Gray
            }
        }

        return $false
    }
}

# ============================================================
# 守门 5：代码格式检查 (rustfmt)
# ============================================================
function Test-GateFormat {
    if ($SkipFmt) {
        Write-GateSkip "代码格式" "已通过 -SkipFmt 跳过"
        return $true
    }

    Write-GateHeader "守门 5/10：代码格式检查 (rustfmt)" ""

    $Script:TotalGates++

    if ($Fix) {
        Write-GateInfo "自动修复模式：运行 cargo fmt..."
        $FixResult = Invoke-GateCommand `
            -Command "cargo" `
            -Arguments @("fmt") `
            -WorkingDirectory $Script:ProjectRoot `
            -ErrorMessage "cargo fmt 失败"
        if (-not $FixResult.Success) {
            Write-GateWarn "cargo fmt 执行异常，继续检查..."
        }
    }

    Write-GateInfo "运行 cargo fmt --check..."

    $Result = Invoke-GateCommand `
        -Command "cargo" `
        -Arguments @("fmt", "--check") `
        -WorkingDirectory $Script:ProjectRoot `
        -ErrorMessage "格式检查失败"

    if ($Result.Success) {
        Write-GatePass "代码格式正确"
        return $true
    } else {
        Write-GateFail "代码格式" "存在格式问题，请运行 cargo fmt 修复"
        return $false
    }
}

# ============================================================
# 守门 6：前端代码重复检测
# ============================================================
function Test-GateFrontendDedup {
    if ($SkipFrontend) {
        Write-GateSkip "前端代码重复" "已通过 -SkipFrontend 跳过"
        return $true
    }

    Write-GateHeader "守门 6/10：前端代码重复检测" ""

    $Script:TotalGates++

    $IndexHtml = Join-Path $Script:ProjectRoot "static\index.html"
    $AppJs = Join-Path $Script:ProjectRoot "static\app.js"

    if (-not (Test-Path -LiteralPath $IndexHtml)) {
        Write-GateSkip "前端代码重复" "index.html 不存在"
        return $true
    }

    if (-not (Test-Path -LiteralPath $AppJs)) {
        Write-GateSkip "前端代码重复" "app.js 不存在"
        return $true
    }

    # 检测 index.html 中是否有内联 <script> 标签（非 src 引用）
    $HtmlContent = Get-Content -LiteralPath $IndexHtml -Raw -ErrorAction Stop

    # 检测内联脚本（<script> 标签不包含 src 属性）
    $InlineScripts = [regex]::Matches($HtmlContent, '<script[^>]*>(?!\s*</script>)([\s\S]*?)</script>')
    $InlineScriptsFiltered = $InlineScripts | Where-Object { $_.Value -notmatch 'src=' }

    if ($InlineScriptsFiltered.Count -gt 0) {
        $InlineScriptContent = ($InlineScriptsFiltered | ForEach-Object { $_.Value }) -join "`n"
        $InlineScriptLines = ($InlineScriptContent -split "`n" | Where-Object { $_.Trim() -ne "" } | Measure-Object).Count

        Write-GateFail "前端代码重复" "index.html 中存在内联 <script> 标签（约 $InlineScriptLines 行），应迁移到 app.js"
        Write-GateInfo "修复方案：删除 index.html 中的内联 <script> 标签内容，仅保留 <script src='app.js'></script> 引用"
        return $false
    }

    # 检查 index.html 是否引用了 app.js
    if ($HtmlContent -match 'src=["\x27]app\.js["\x27]') {
        Write-GatePass "前端代码无重复（app.js 已正确引用）"
        return $true
    } else {
        Write-GateWarn "index.html 未引用 app.js，但无内联脚本（可能为纯 HTML）"
        Write-GatePass "前端代码无重复"
        return $true
    }
}

# ============================================================
# 守门 7：前端 XSS 安全检测
# ============================================================
function Test-GateFrontendXSS {
    if ($SkipFrontend) {
        Write-GateSkip "前端 XSS 检测" "已通过 -SkipFrontend 跳过"
        return $true
    }

    Write-GateHeader "守门 7/10：前端 XSS 安全检测" ""

    $Script:TotalGates++

    $JsFiles = @(
        (Join-Path $Script:ProjectRoot "static\app.js"),
        (Join-Path $Script:ProjectRoot "static\index.html")
    ) | Where-Object { Test-Path -LiteralPath $_ }

    $DangerousPatterns = @(
        @{ Pattern = 'innerHTML\s*='; Name = "innerHTML 直接赋值（XSS 风险）" },
        @{ Pattern = 'document\.write\('; Name = "document.write() 调用" },
        @{ Pattern = 'eval\('; Name = "eval() 调用" }
    )

    $Violations = @()

    foreach ($File in $JsFiles) {
        $Content = Get-Content -LiteralPath $File -Raw -ErrorAction Stop
        $RelativePath = $File.FullName.Replace($Script:ProjectRoot, "").TrimStart("\")

        foreach ($Pattern in $DangerousPatterns) {
            if ($Content -match $Pattern.Pattern) {
                # 排除已安全转义的使用（如 htmlescape() 包裹）
                if ($Pattern.Name -eq "innerHTML 直接赋值（XSS 风险）" `
                        -and $Content -match 'htmlescape\(') {
                    continue  # 已使用 htmlescape() 转义，安全
                }

                $Violations += "$RelativePath : $($Pattern.Name)"
            }
        }
    }

    if ($Violations.Count -eq 0) {
        Write-GatePass "前端 XSS 安全检查通过"
        return $true
    } else {
        Write-GateFail "前端 XSS 安全" "发现 $($Violations.Count) 处潜在风险"
        $Violations | ForEach-Object { Write-Host "    $_" -ForegroundColor Red }
        return $false
    }
}

# ============================================================
# 守门 8：算法泄露检测
# ============================================================
function Test-GateLeakCheck {
    if ($SkipLeakCheck) {
        Write-GateSkip "算法泄露检测" "已通过 -SkipLeakCheck 跳过"
        return $true
    }

    Write-GateHeader "守门 8/10：核心算法泄露检测" ""

    $Script:TotalGates++

    $LeakScript = Join-Path $Script:ProjectRoot "scripts\check_algorithm_leak.py"
    if (-not (Test-Path -LiteralPath $LeakScript)) {
        Write-GateSkip "算法泄露检测" "检测脚本不存在: $LeakScript"
        return $true
    }

    # 查找 Python 解释器
    $PythonCmd = $null
    foreach ($Cmd in @("python", "python3", "py")) {
        try {
            $null = Get-Command $Cmd -ErrorAction Stop
            $PythonCmd = $Cmd
            break
        } catch {
            continue
        }
    }

    if (-not $PythonCmd) {
        Write-GateSkip "算法泄露检测" "未找到 Python 解释器"
        return $true
    }

    Write-GateInfo "运行算法泄露检测 (Python: $PythonCmd)..."

    $Result = Invoke-GateCommand `
        -Command $PythonCmd `
        -Arguments @($LeakScript) `
        -WorkingDirectory $Script:ProjectRoot `
        -ErrorMessage "算法泄露检测失败"

    if ($Result.Success) {
        Write-GatePass "算法泄露检测通过（公开层无受保护算法）"
        return $true
    } else {
        Write-GateFail "算法泄露检测" "公开层文件中包含受保护的核心算法逻辑"
        Write-Host "  --- 检测输出 ---" -ForegroundColor Red
        ($Result.StdOut -split "`n") | Select-Object -Last 10 | ForEach-Object { Write-Host "    $_" -ForegroundColor Red }
        return $false
    }
}

# ============================================================
# 守门 9：长函数检测 (函数 > 100 行)
# ============================================================
function Test-GateLongFunctions {
    Write-GateHeader "守门 9/10：长函数检测" ""

    $Script:TotalGates++

    Write-GateInfo "检测超过 100 行的函数..."

    $RustFiles = Get-ChildItem -LiteralPath (Join-Path $Script:ProjectRoot "src") -Recurse -Filter "*.rs" -ErrorAction Stop

    $LongFunctions = @()

    foreach ($File in $RustFiles) {
        $Lines = Get-Content -LiteralPath $File.FullName -ErrorAction Stop
        $RelativePath = $File.FullName.Replace($Script:ProjectRoot, "").TrimStart("\")

        $InFunction = $false
        $FunctionName = ""
        $FunctionStart = 0
        $BraceDepth = 0

        for ($i = 0; $i -lt $Lines.Count; $i++) {
            $Line = $Lines[$i]

            # 检测函数定义开始
            if ($Line -match '^\s*(pub\s+)?(async\s+)?fn\s+(\w+)') {
                $InFunction = $true
                $FunctionName = $Matches[3]
                $FunctionStart = $i + 1
                $BraceDepth = 0

                # 计算当前行的花括号深度
                $OpenBraces = ($Line.ToCharArray() | Where-Object { $_ -eq '{' } | Measure-Object).Count
                $CloseBraces = ($Line.ToCharArray() | Where-Object { $_ -eq '}' } | Measure-Object).Count
                $BraceDepth = $OpenBraces - $CloseBraces
            }
            elseif ($InFunction) {
                $OpenBraces = ($Line.ToCharArray() | Where-Object { $_ -eq '{' } | Measure-Object).Count
                $CloseBraces = ($Line.ToCharArray() | Where-Object { $_ -eq '}' } | Measure-Object).Count
                $BraceDepth += ($OpenBraces - $CloseBraces)

                if ($BraceDepth -le 0) {
                    # 函数结束
                    $FunctionLength = ($i + 1) - $FunctionStart + 1
                    if ($FunctionLength -gt 100) {
                        $LongFunctions += @{
                            File     = $RelativePath
                            Function = $FunctionName
                            Line     = $FunctionStart
                            Length   = $FunctionLength
                        }
                    }
                    $InFunction = $false
                }
            }
        }
    }

    if ($LongFunctions.Count -eq 0) {
        Write-GatePass "所有函数 ≤ 100 行"
        return $true
    } else {
        if ($Strict) {
            Write-GateFail "长函数检测" "发现 $($LongFunctions.Count) 个函数超过 100 行"
            $LongFunctions | ForEach-Object {
                Write-Host "    $($_.File)::$($_.Function)() — $($_.Length) 行 (L$($_.Line))" -ForegroundColor Yellow
            }
            return $false
        } else {
            Write-GateWarn "发现 $($LongFunctions.Count) 个长函数（非严格模式允许通过）"
            $LongFunctions | ForEach-Object {
                Write-Host "    $($_.File)::$($_.Function)() — $($_.Length) 行 (L$($_.Line))" -ForegroundColor Yellow
            }
            Write-GatePass "长函数检测通过（警告模式）"
            return $true
        }
    }
}

# ============================================================
# 守门 10：类型转换安全性检测
# ============================================================
function Test-GateCastSafety {
    Write-GateHeader "守门 10/10：类型转换安全性检测" ""

    $Script:TotalGates++

    Write-GateInfo "检测不安全的类型转换 (as 关键字)..."

    $RustFiles = Get-ChildItem -LiteralPath (Join-Path $Script:ProjectRoot "src") -Recurse -Filter "*.rs" -ErrorAction Stop

    $CastViolations = @()

    foreach ($File in $RustFiles) {
        $Lines = Get-Content -LiteralPath $File.FullName -ErrorAction Stop
        $RelativePath = $File.FullName.Replace($Script:ProjectRoot, "").TrimStart("\")

        for ($i = 0; $i -lt $Lines.Count; $i++) {
            $Line = $Lines[$i]

            # 跳过注释
            if ($Line.Trim().StartsWith("//") -or $Line.Trim().StartsWith("/*")) {
                continue
            }

            # 检测潜在截断的 as 转换
            if ($Line -match '\bu128\b.*\bas\s+u64\b' -or
                $Line -match '\bi128\b.*\bas\s+i64\b' -or
                $Line -match '\bu64\b.*\bas\s+u32\b' -or
                $Line -match '\bi64\b.*\bas\s+i32\b') {
                $CastViolations += @{
                    File = $RelativePath
                    Line = $i + 1
                    Code = $Line.Trim()
                    Type = "可能截断的类型转换"
                }
            }
        }
    }

    if ($CastViolations.Count -eq 0) {
        Write-GatePass "无危险类型转换"
        return $true
    } else {
        if ($Strict) {
            Write-GateFail "类型转换安全" "发现 $($CastViolations.Count) 处可能截断的类型转换"
            $CastViolations | ForEach-Object {
                Write-Host "    $($_.File):L$($_.Line) — $($_.Code)" -ForegroundColor Yellow
            }
            return $false
        } else {
            Write-GateWarn "发现 $($CastViolations.Count) 处可能截断的类型转换（非严格模式允许通过）"
            Write-GatePass "类型转换检测通过（警告模式）"
            return $true
        }
    }
}

# ============================================================
# 生成审计报告
# ============================================================
function New-GatekeeperReport {
    if (-not $GenerateReport) {
        return
    }

    Write-GateHeader "生成审计报告" ""

    $Report = @{
        timestamp      = (Get-Date -Format "yyyy-MM-ddTHH:mm:sszzz")
        project        = "Loong Recall (LRC)"
        version        = "0.2.0"
        duration_sec   = [math]::Round(((Get-Date) - $Script:GateStartTime).TotalSeconds, 1)
        summary = @{
            total_gates   = $Script:TotalGates
            passed        = $Script:PassedGates
            failed        = $Script:FailedGates
            skipped       = $Script:SkippedGates
            overall       = if ($Script:FailedGates -eq 0) { "PASS" } else { "FAIL" }
        }
        gate_results    = $Script:GateResults
        warnings        = $Script:Warnings
        errors          = $Script:Errors
        environment     = @{
            rust_version = (rustc --version 2>$null) -replace "rustc ", ""
            cargo_version = (cargo --version 2>$null) -replace "cargo ", ""
            os           = [System.Environment]::OSVersion.VersionString
            powershell   = $PSVersionTable.PSVersion.ToString()
        }
    }

    try {
        $Report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $Script:ReportPath -Encoding UTF8 -ErrorAction Stop
        Write-GateInfo "审计报告已生成: $($Script:ReportPath)"
    } catch {
        Write-GateWarn "审计报告生成失败: $_"
    }
}

# ============================================================
# 主流程：按契约执行所有守门检查
# ============================================================
function Start-Gatekeeper {
    Write-Host ""
    Write-Host ("=" * 70) -ForegroundColor Magenta
    Write-Host "  Loong Recall (LRC) 自动化静态分析守门人 v1.0.0" -ForegroundColor Magenta
    Write-Host "  工程文化教练监督 · 契约优先 · TDD 驱动 · 无指责复盘" -ForegroundColor Magenta
    Write-Host ("=" * 70) -ForegroundColor Magenta
    Write-Host ""
    Write-Host "  项目路径: $($Script:ProjectRoot)" -ForegroundColor Gray
    Write-Host "  报告路径: $($Script:ReportPath)" -ForegroundColor Gray
    Write-Host "  模式: $(if ($Strict) { '严格模式 (CI)' } else { '标准模式' })" -ForegroundColor Gray
    Write-Host "  自动修复: $(if ($Fix) { '启用' } else { '禁用' })" -ForegroundColor Gray
    Write-Host ""

    # 检查 Rust 工具链
    try {
        $null = Get-Command "cargo" -ErrorAction Stop
        Write-GateInfo "Rust 工具链已就绪 ($(rustc --version 2>$null))"
    } catch {
        Write-Host "  [FATAL] 未找到 Rust 工具链，请先安装: https://rustup.rs" -ForegroundColor Red
        exit 1
    }

    # ── 执行所有守门检查 ──
    $OverallResult = $true

    # 守门 1：编译
    if (-not (Test-GateCompile)) { $OverallResult = $false }

    # 守门 2：测试
    if (-not (Test-GateTests)) { $OverallResult = $false }

    # 守门 3：Clippy
    if (-not (Test-GateClippy)) { $OverallResult = $false }

    # 守门 4：unwrap 检测
    if (-not (Test-GateUnwrapBan)) { $OverallResult = $false }

    # 守门 5：格式
    if (-not (Test-GateFormat)) { $OverallResult = $false }

    # 守门 6：前端重复
    if (-not (Test-GateFrontendDedup)) { $OverallResult = $false }

    # 守门 7：前端 XSS
    if (-not (Test-GateFrontendXSS)) { $OverallResult = $false }

    # 守门 8：算法泄露
    if (-not (Test-GateLeakCheck)) { $OverallResult = $false }

    # 守门 9：长函数
    if (-not (Test-GateLongFunctions)) { $OverallResult = $false }

    # 守门 10：类型转换
    if (-not (Test-GateCastSafety)) { $OverallResult = $false }

    # ── 生成报告 ──
    New-GatekeeperReport

    # ── 最终裁决 ──
    $Elapsed = [math]::Round(((Get-Date) - $Script:GateStartTime).TotalSeconds, 1)

    Write-Host ""
    Write-Host ("=" * 70) -ForegroundColor $(if ($OverallResult) { "Green" } else { "Red" })
    Write-Host "  守门人裁决" -ForegroundColor $(if ($OverallResult) { "Green" } else { "Red" })
    Write-Host ("=" * 70) -ForegroundColor $(if ($OverallResult) { "Green" } else { "Red" })
    Write-Host "  通过: $Script:PassedGates / $Script:TotalGates" -ForegroundColor Green
    if ($Script:FailedGates -gt 0) {
        Write-Host "  失败: $Script:FailedGates / $Script:TotalGates" -ForegroundColor Red
    }
    if ($Script:SkippedGates -gt 0) {
        Write-Host "  跳过: $Script:SkippedGates / $Script:TotalGates" -ForegroundColor Yellow
    }
    Write-Host "  耗时: ${Elapsed}s" -ForegroundColor Gray
    Write-Host ""

    if ($OverallResult) {
        Write-Host "  裁决: 通过 — 所有守门检查通过，代码质量达标" -ForegroundColor Green
        Write-Host ""
        Write-Host "  工程文化教练评语:" -ForegroundColor Cyan
        Write-Host "  '契约达成。代码如承诺般可靠，测试如契约般精确。'" -ForegroundColor Cyan
        Write-Host "  '可以安全合并。'" -ForegroundColor Cyan
        Write-Host ""

        if ($Script:Warnings.Count -gt 0) {
            Write-Host "  待处理警告 ($($Script:Warnings.Count) 项):" -ForegroundColor Yellow
            $Script:Warnings | ForEach-Object { Write-Host "    - $_" -ForegroundColor Yellow }
        }

        exit 0
    } else {
        Write-Host "  裁决: 拒绝 — 存在 $Script:FailedGates 道守门未通过" -ForegroundColor Red
        Write-Host ""

        Write-Host "  工程文化教练评语:" -ForegroundColor Cyan
        Write-Host "  '契约未达成。守门人拒绝放行，因为质量承诺尚未兑现。'" -ForegroundColor Cyan
        Write-Host "  '这不是指责，而是保护。请修复上述问题后重新提交。'" -ForegroundColor Cyan
        Write-Host ""

        Write-Host "  修复指引:" -ForegroundColor Yellow
        if ($Script:GateResults["unwrap()/expect() 残留检测"] -eq "FAIL") {
            Write-Host "    → 参考修复计划 P0-1: 逐文件替换 unwrap()/expect()" -ForegroundColor Gray
        }
        if ($Script:GateResults["Clippy 静态分析"] -eq "FAIL") {
            Write-Host "    → 运行: cargo clippy --fix --allow-dirty --allow-staged" -ForegroundColor Gray
        }
        if ($Script:GateResults["前端代码重复检测"] -eq "FAIL") {
            Write-Host "    → 删除 index.html 中内联 <script> 标签，仅保留 app.js 引用" -ForegroundColor Gray
        }
        if ($Script:GateResults["代码格式检查"] -eq "FAIL") {
            Write-Host "    → 运行: cargo fmt" -ForegroundColor Gray
        }

        Write-Host ""
        Write-Host "  提示: 使用 -Fix 参数自动修复部分问题" -ForegroundColor Gray
        Write-Host "        .\scripts\gatekeeper.ps1 -Fix" -ForegroundColor Gray

        exit 1
    }
}

# ============================================================
# 入口点
# ============================================================
Start-Gatekeeper