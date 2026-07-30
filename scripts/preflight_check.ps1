# ============================================================
# LRC Preflight Check Script (v1.0)
# ============================================================
# One-click pre-release audit covering 8 domains (PUSH_STANDARD.md Ch.14)
# Usage: powershell -File scripts/preflight_check.ps1
# Exit: 0 = all pass, 1 = has failures
# ============================================================

param([switch]$VerboseOutput)

$ErrorActionPreference = "Continue"
$script:failCount = 0
$script:passCount = 0
$script:warnCount = 0
$script:results = @()

# Ensure cargo in PATH (Windows compatibility)
$env:PATH += ";$env:USERPROFILE\.cargo\bin"

function Write-Check {
    param([string]$domain, [string]$check, [string]$status, [string]$detail)
    $script:results += [PSCustomObject]@{
        Domain = $domain
        Check  = $check
        Status = $status
        Detail = $detail
    }
    switch ($status) {
        "PASS" {
            Write-Host "  [PASS] $check" -ForegroundColor Green
            $script:passCount++
        }
        "FAIL" {
            Write-Host "  [FAIL] $check" -ForegroundColor Red
            if ($detail) { Write-Host "         $detail" -ForegroundColor Yellow }
            $script:failCount++
        }
        "WARN" {
            Write-Host "  [WARN] $check" -ForegroundColor Yellow
            if ($detail) { Write-Host "         $detail" -ForegroundColor DarkYellow }
            $script:warnCount++
        }
    }
}

function Invoke-CommandSilent {
    param([scriptblock]$block)
    $output = & $block 2>&1
    $exitCode = $LASTEXITCODE
    return @{ Output = $output; ExitCode = $exitCode }
}

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  LRC Preflight Check — 8-Domain Pre-Release Audit" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan

# ============================================================
# Domain 1: Code Compilation
# ============================================================
Write-Host "`n[Domain 1] Code Compilation" -ForegroundColor Cyan

# 1.1 Main project compile check
$result = Invoke-CommandSilent { cargo check --features server }
if ($result.ExitCode -eq 0) {
    Write-Check "1. Compile" "Main project cargo check --features server" "PASS" ""
} else {
    Write-Check "1. Compile" "Main project cargo check --features server" "FAIL" "Compile failed, run 'cargo check --features server' for details"
}

# 1.2 Desktop compile check
$result = Invoke-CommandSilent { Push-Location desktop/src-tauri; cargo check; Pop-Location }
if ($result.ExitCode -eq 0) {
    Write-Check "1. Compile" "Desktop cargo check" "PASS" ""
} else {
    Write-Check "1. Compile" "Desktop cargo check" "FAIL" "Desktop compile failed, run 'cd desktop/src-tauri; cargo check' for details"
}

# ============================================================
# Domain 2: Code Quality
# ============================================================
Write-Host "`n[Domain 2] Code Quality" -ForegroundColor Cyan

# 2.1 Format check
$result = Invoke-CommandSilent { cargo fmt --all -- --check }
if ($result.ExitCode -eq 0) {
    Write-Check "2. Quality" "cargo fmt --check" "PASS" ""
} else {
    Write-Check "2. Quality" "cargo fmt --check" "FAIL" "Run 'cargo fmt --all' to fix"
}

# 2.2 Clippy check (server feature)
$result = Invoke-CommandSilent { cargo clippy --features server -- -D warnings }
if ($result.ExitCode -eq 0) {
    Write-Check "2. Quality" "cargo clippy --features server -- -D warnings" "PASS" ""
} else {
    Write-Check "2. Quality" "cargo clippy --features server -- -D warnings" "FAIL" "Run 'cargo clippy --features server --fix' to fix"
}

# 2.3 Clippy check (all targets)
$result = Invoke-CommandSilent { cargo clippy --all-targets -- -D warnings }
if ($result.ExitCode -eq 0) {
    Write-Check "2. Quality" "cargo clippy --all-targets -- -D warnings" "PASS" ""
} else {
    Write-Check "2. Quality" "cargo clippy --all-targets -- -D warnings" "FAIL" "Run 'cargo clippy --all-targets --fix' to fix"
}

# 2.4 Algorithm leak detection
$result = Invoke-CommandSilent { python scripts/check_algorithm_leak.py }
if ($result.ExitCode -eq 0) {
    Write-Check "2. Quality" "Algorithm leak detection" "PASS" ""
} else {
    Write-Check "2. Quality" "Algorithm leak detection" "FAIL" "Run 'python scripts/check_algorithm_leak.py --verbose' for details"
}

# ============================================================
# Domain 3: Cross-Platform Config
# ============================================================
Write-Host "`n[Domain 3] Cross-Platform Config" -ForegroundColor Cyan

# 3.1 Tauri targets check (must not be single-platform like ["nsis"])
$tauriContent = Get-Content "desktop/src-tauri/tauri.conf.json" -Raw
if ($tauriContent -match '"targets"\s*:\s*"all"') {
    Write-Check "3. Cross-Platform" "Tauri targets = 'all'" "PASS" ""
} elseif ($tauriContent -match '"targets"\s*:\s*\[' -and $tauriContent -match '"nsis"' -and -not ($tauriContent -match '"all"')) {
    Write-Check "3. Cross-Platform" "Tauri targets config" "FAIL" "targets contains only 'nsis', macOS/Linux will have no bundle. Change to 'all'"
} else {
    Write-Check "3. Cross-Platform" "Tauri targets config" "WARN" "Could not determine targets value, verify manually"
}

# 3.2 MSRV consistency
$mainMsrv = (Select-String -Path Cargo.toml -Pattern '^rust-version' | Select-Object -First 1).Line -replace '.*"(.*)".*', '$1'
$desktopMsrv = (Select-String -Path desktop/src-tauri/Cargo.toml -Pattern '^rust-version' | Select-Object -First 1).Line -replace '.*"(.*)".*', '$1'
if ($mainMsrv -and $desktopMsrv -and ($mainMsrv -eq $desktopMsrv)) {
    Write-Check "3. Cross-Platform" "MSRV consistency (main=$mainMsrv, desktop=$desktopMsrv)" "PASS" ""
} else {
    Write-Check "3. Cross-Platform" "MSRV consistency" "FAIL" "Main MSRV=$mainMsrv, Desktop MSRV=$desktopMsrv — must be equal"
}

# ============================================================
# Domain 4: README.md (simplified)
# ============================================================
Write-Host "`n[Domain 4] README.md" -ForegroundColor Cyan

# 4.1 No file:// links
$fileLinks = Select-String -Path README.md -Pattern 'file:///' -ErrorAction SilentlyContinue
if (-not $fileLinks) {
    Write-Check "4. README" "No file:// links" "PASS" ""
} else {
    Write-Check "4. README" "No file:// links" "FAIL" "Found $($fileLinks.Count) file:// links in README.md"
}

# 4.2 Rust badge version matches Cargo.toml MSRV
$badgeMatch = [regex]::Match((Get-Content README.md -Raw), 'Rust-(\d+\.\d+)')
$cargoMsrvMatch = [regex]::Match((Get-Content Cargo.toml -Raw), 'rust-version\s*=\s*"(\d+\.\d+)"')
if ($badgeMatch.Success -and $cargoMsrvMatch.Success) {
    if ($badgeMatch.Groups[1].Value -eq $cargoMsrvMatch.Groups[1].Value) {
        Write-Check "4. README" "Rust badge ($($badgeMatch.Groups[1].Value)) = MSRV ($($cargoMsrvMatch.Groups[1].Value))" "PASS" ""
    } else {
        Write-Check "4. README" "Rust badge version" "FAIL" "Badge=$($badgeMatch.Groups[1].Value) vs MSRV=$($cargoMsrvMatch.Groups[1].Value)"
    }
} else {
    Write-Check "4. README" "Rust badge version" "WARN" "Could not extract badge or MSRV version"
}

# ============================================================
# Domain 5: Version Consistency
# ============================================================
Write-Host "`n[Domain 5] Version Consistency" -ForegroundColor Cyan

# v0.8.13 修复：统一使用 Select-String 提取版本号，避免 Get-Content -Raw + BOM 导致正则失配
$cargoLine = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$desktopLine = (Select-String -Path desktop/src-tauri/Cargo.toml -Pattern '^version\s*=\s*"(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$tauriLine = (Select-String -Path desktop/src-tauri/tauri.conf.json -Pattern '"version"\s*:\s*"(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
# v0.8.13 修复：必须检查 Cargo.lock 中 code-memory 包的版本号
# v0.8.12 CI 失败根因：Cargo.toml=0.8.12 但 Cargo.lock=0.8.11，导致 cargo 重新编译触发问题
$cargoLockMatch = Select-String -Path Cargo.lock -Pattern 'name = "code-memory"' -Context 0,1 | Select-Object -First 1
$cargoLockVer = ""
if ($cargoLockMatch) {
    $lockVerMatch = [regex]::Match($cargoLockMatch.Context.PostContext, 'version\s*=\s*"(.*)"')
    if ($lockVerMatch.Success) { $cargoLockVer = $lockVerMatch.Groups[1].Value }
}

$versions = @{
    "Cargo.toml"         = $cargoLine
    "desktop Cargo.toml" = $desktopLine
    "tauri.conf.json"    = $tauriLine
    "Cargo.lock"         = $cargoLockVer
}

$allSame = $true
$firstVer = $cargoLine
foreach ($kv in $versions.GetEnumerator()) {
    if ($kv.Value -ne $firstVer) { $allSame = $false }
}

if ($allSame -and $cargoLine) {
    Write-Check "5. Version" "Version consistency (all = $cargoLine)" "PASS" ""
} else {
    # v0.8.13 修复：Join-String 在 PS 5.1 不可用，改用 -join 运算符
    $detailParts = $versions.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }
    $detail = $detailParts -join ", "
    Write-Check "5. Version" "Version consistency" "FAIL" $detail
    # v0.8.13 修复：如果 Cargo.lock 版本号不一致，提供自动修复提示
    if ($cargoLockVer -and $cargoLockVer -ne $cargoVer) {
        Write-Check "5. Version" "Cargo.lock sync" "FAIL" "Run 'cargo check --features server' to update Cargo.lock, then commit"
    }
}

# ============================================================
# Domain 6: CHANGELOG.md
# ============================================================
Write-Host "`n[Domain 6] CHANGELOG.md" -ForegroundColor Cyan

$changelogContent = Get-Content CHANGELOG.md -Raw -ErrorAction SilentlyContinue
if ($changelogContent -and $changelogContent -match "##\s*\[$cargoLine\]") {
    Write-Check "6. CHANGELOG" "CHANGELOG has entry for v$cargoLine" "PASS" ""
} else {
    Write-Check "6. CHANGELOG" "CHANGELOG entry for v$cargoLine" "FAIL" "Add '## [$cargoLine] - <date>' section to CHANGELOG.md"
}

# ============================================================
# Domain 7: User Documentation
# ============================================================
Write-Host "`n[Domain 7] User Documentation" -ForegroundColor Cyan
Write-Check "7. Docs" "User docs manual review" "WARN" "Manual check: verify docs/USER_GUIDE.md matches current features"

# ============================================================
# Domain 8: CI/CD Config
# ============================================================
Write-Host "`n[Domain 8] CI/CD Config" -ForegroundColor Cyan

# 8.1 release.yml has preflight job
$releaseContent = Get-Content .github/workflows/release.yml -Raw
if ($releaseContent -match 'preflight:' -and $releaseContent -match 'needs:\s*preflight') {
    Write-Check "8. CI/CD" "release.yml has preflight job with needs" "PASS" ""
} else {
    Write-Check "8. CI/CD" "release.yml preflight job" "FAIL" "Missing preflight job or needs: preflight in release.yml"
}

# 8.2 ci.yml has desktop cargo check
$ciContent = Get-Content .github/workflows/ci.yml -Raw
if ($ciContent -match 'cargo check \(desktop\)') {
    Write-Check "8. CI/CD" "ci.yml has desktop cargo check" "PASS" ""
} else {
    Write-Check "8. CI/CD" "ci.yml desktop cargo check" "FAIL" "Missing desktop cargo check in ci.yml build-matrix"
}

# 8.3 ci.yml has tauri config lint
if ($ciContent -match 'Tauri config lint') {
    Write-Check "8. CI/CD" "ci.yml has tauri config lint" "PASS" ""
} else {
    Write-Check "8. CI/CD" "ci.yml tauri config lint" "FAIL" "Missing tauri config lint in ci.yml"
}

# ============================================================
# Summary
# ============================================================
Write-Host "`n============================================================" -ForegroundColor Cyan
Write-Host "  Audit Summary" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  Passed: $script:passCount" -ForegroundColor Green
Write-Host "  Failed: $script:failCount" -ForegroundColor Red
Write-Host "  Warnings: $script:warnCount" -ForegroundColor Yellow

if ($script:failCount -gt 0) {
    Write-Host "`n  RESULT: FAILED — fix all FAIL items before release" -ForegroundColor Red
    Write-Host "  Run with -VerboseOutput for detailed output" -ForegroundColor DarkGray
    exit 1
} else {
    Write-Host "`n  RESULT: ALL CHECKS PASSED — ready for release" -ForegroundColor Green
    exit 0
}
