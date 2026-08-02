# LRC 推送前预检脚本
# 执行 PRE_PUSH_CHECKLIST.md 中定义的所有检查项
# 使用方式：.\preflight_check.ps1
# 依赖：PowerShell 7+, Rust toolchain, Node.js
# 编码：必须以 UTF-8 with BOM 保存（PS5.1 兼容性要求）

# ══════════════════════════════════════════════════
# 编码一致性保护 (PowerShell 专家规范，防乱码铁律)
# ══════════════════════════════════════════════════
if ($PSVersionTable.PSVersion.Major -lt 6) {
    try { & chcp 65001 > $null } catch { }
}
$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$PSDefaultParameterValues['*:Encoding'] = 'utf8'

$ErrorActionPreference = 'Stop'
$exitCode = 0
$failedItems = @()

function Check-Item {
    param([string]$Name, [scriptblock]$Script)
    Write-Host "  [检查] $Name..." -ForegroundColor Yellow
    try {
        & $Script
        Write-Host "    ✓ 通过" -ForegroundColor Green
    } catch {
        Write-Host "    ✗ 失败: $_" -ForegroundColor Red
        # ⚠ PowerShell 铁律：${Name} 显式分隔防止 $Name: 被解析为 Provider 路径
        $script:failedItems += "${Name}: $($_.Exception.Message)"
        $script:exitCode = 1
    }
}

Write-Host "══════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host " LRC 推送前预检脚本 v0.8.23" -ForegroundColor Cyan
Write-Host "══════════════════════════════════════════════════" -ForegroundColor Cyan

# 一、代码质量检查
Write-Host "`n[一] 代码质量检查" -ForegroundColor Magenta

# 1. 代码格式
Check-Item -Name "代码格式 (cargo fmt)" -Script {
    $result = cargo fmt --all -- --check 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "格式检查失败，请运行 cargo fmt 修复`n$result"
    }
}

# 2. Clippy 静态检查
Check-Item -Name "Clippy 静态检查" -Script {
    $result = cargo clippy --features server -- -D warnings 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Clippy 警告，请修复后重试`n$result"
    }
}

# 3. 编译检查
Check-Item -Name "编译检查 (cargo check)" -Script {
    $result = cargo check --features server 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "编译失败`n$result"
    }
}

# 4. 单元测试
Check-Item -Name "单元测试" -Script {
    $result = cargo test --features server 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "测试失败`n$result"
    }
}

# 5. 桌面端编译
Check-Item -Name "桌面端编译" -Script {
    Push-Location desktop/src-tauri
    try {
        $result = cargo check 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "桌面端编译失败`n$result"
        }
    } finally {
        Pop-Location
    }
}

# 二、版本号一致性检查
Write-Host "`n[二] 版本号一致性检查" -ForegroundColor Magenta

$targetVersion = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | ForEach-Object { $_.Matches.Groups[1].Value })

Check-Item -Name "根 Cargo.toml 版本号" -Script {
    if (-not $targetVersion) { throw "无法读取根 Cargo.toml 版本号" }
    Write-Host "      目标版本: $targetVersion" -ForegroundColor Gray
}

Check-Item -Name "desktop Cargo.toml 版本号一致" -Script {
    $v = (Select-String -Path "desktop/src-tauri/Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | ForEach-Object { $_.Matches.Groups[1].Value })
    if ($v -ne $targetVersion) { throw "版本不一致: 期望 $targetVersion, 实际 $v" }
}

Check-Item -Name "tauri.conf.json 版本号一致" -Script {
    $v = (Select-String -Path "desktop/src-tauri/tauri.conf.json" -Pattern '"version"\s*:\s*"([^"]+)"' | ForEach-Object { $_.Matches.Groups[1].Value })
    if ($v -ne $targetVersion) { throw "版本不一致: 期望 $targetVersion, 实际 $v" }
}

Check-Item -Name "app.js APP_VERSION 一致" -Script {
    $v = (Select-String -Path "static/app.js" -Pattern "APP_VERSION\s*=\s*'([^']+)'" | ForEach-Object { $_.Matches.Groups[1].Value })
    if ($v -ne $targetVersion) { throw "版本不一致: 期望 $targetVersion, 实际 $v" }
}

Check-Item -Name "index.html meta version 一致" -Script {
    $v = (Select-String -Path "static/index.html" -Pattern 'version\s*=\s*"([^"]+)"' | ForEach-Object { $_.Matches.Groups[1].Value })
    if ($v -ne $targetVersion) { throw "版本不一致: 期望 $targetVersion, 实际 $v" }
}

Check-Item -Name "CHANGELOG 有当前版本条目" -Script {
    # ⚠ 兼容两种格式："## v0.8.31" 和 "## [0.8.31] - YYYY-MM-DD"
    $pattern1 = 'v' + [regex]::Escape($targetVersion)
    $pattern2 = '\[' + [regex]::Escape($targetVersion) + '\]'
    if (-not (Select-String -Path "CHANGELOG.md" -Pattern $pattern1) -and
        -not (Select-String -Path "CHANGELOG.md" -Pattern $pattern2)) {
        throw "CHANGELOG.md 中未找到 $targetVersion 条目（已检查模式：$pattern1 和 $pattern2）"
    }
}

# 三、安全与合规检查
Write-Host "`n[三] 安全与合规检查" -ForegroundColor Magenta

Check-Item -Name "Git 未跟踪文件检查" -Script {
    $untracked = git status --short
    $untracked | Where-Object { $_ -match '^\?\?' } | ForEach-Object {
        Write-Host "    ⚠ 未跟踪文件: $_" -ForegroundColor Yellow
    }
}

Check-Item -Name "敏感文件检查" -Script {
    $sensitive = git diff --cached --name-only | Where-Object {
        $_ -match '\.env|credentials\.json|\.secret|\.key'
    }
    if ($sensitive) {
        throw "含敏感文件: $($sensitive -join ', ')"
    }
}

Check-Item -Name "大文件检查" -Script {
    $largeFiles = git diff --stat | Select-String '(\d+) \+' | ForEach-Object {
        $size = [int]$_.Matches.Groups[1].Value
        if ($size -gt 1000) { $_.Line }
    }
    if ($largeFiles) {
        Write-Host "    ⚠ 大文件: $largeFiles" -ForegroundColor Yellow
    }
}

# 四、汇总
Write-Host "`n══════════════════════════════════════════════════" -ForegroundColor Cyan
if ($exitCode -eq 0) {
    Write-Host "  ✓ 全部检查通过！" -ForegroundColor Green
} else {
    Write-Host "  ✗ 以下检查项失败：" -ForegroundColor Red
    $failedItems | ForEach-Object { Write-Host "    - $_" -ForegroundColor Red }
}
Write-Host "══════════════════════════════════════════════════" -ForegroundColor Cyan
exit $exitCode