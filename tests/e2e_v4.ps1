# LRC P2 E2E Test v4 - Simplified for PS 5.1
$ErrorActionPreference = "Stop"
$exe = Join-Path $PSScriptRoot "..\target\release\code-memory-server.exe"
$testRoot = Join-Path $env:TEMP "lrc_e2e_v4"

Write-Host "=== LRC P2 E2E Test v4 ===" -ForegroundColor Cyan
Write-Host "Binary: $exe"
Write-Host ""

# Cleanup
function Kill-All-LRC {
  Get-Process code-memory-server -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 2
}

# Wait for HTTP health
function Wait-Health($Port, $Timeout = 40) {
  $dead = (Get-Date).AddSeconds($Timeout)
  $n = 0
  while ((Get-Date) -lt $dead) {
    $n++
    try {
      $r = Invoke-WebRequest "http://localhost:$Port/health" -TimeoutSec 3 -UseBasicParsing
      if ($r.StatusCode -eq 200) {
        Write-Host "  [OK] Port $Port ready (${n}s)" -ForegroundColor DarkGray
        return $true
      }
    } catch {}
    Start-Sleep 1
  }
  Write-Host "  [FAIL] Port $Port not ready (${n} attempts)" -ForegroundColor Red
  return $false
}

# Start LRC
function Start-LRC($Src, $Db, $Port, $Extra) {
  $a = @("--src-dir",$Src,"--db-path",$Db,"--port",$Port,"--mode","fast") + $Extra
  $o = Join-Path $Db "out.txt"
  $e = Join-Path $Db "err.txt"
  return Start-Process -FilePath $exe -ArgumentList $a -WindowStyle Hidden -PassThru -RedirectStandardOutput $o -RedirectStandardError $e
}

# Setup test project
Remove-Item -Recurse -Force $testRoot -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $testRoot | Out-Null
$src = Join-Path $testRoot "src"
New-Item -ItemType Directory -Force $src | Out-Null
"fn hello() {} struct Foo { x: i32 } impl Foo { fn new() -> Self { Foo { x: 0 } } } pub trait Runner { fn run(&self); }" | Out-File (Join-Path $src "main.rs") -Encoding utf8

$p = 0
$f = 0

# ==== Test 1: Dashboard ====
Write-Host "[Test 1/6] Dashboard Mode" -ForegroundColor Yellow
Kill-All-LRC
try {
  $d = Join-Path $testRoot "t1"; New-Item -ItemType Directory -Force $d | Out-Null
  $proc = Start-LRC $src $d 3221 @("--dashboard")
  $ok = Wait-Health 3221
  if (-not $ok) { throw "Health failed" }
  $lf = Join-Path $d ".lrc.lock"
  if (-not (Test-Path $lf)) { throw "No lock file" }
  $lc = Get-Content $lf -Raw
  Write-Host "  Lock: $lc"
  if (-not $proc.HasExited) { $proc.Kill(); $proc.WaitForExit(3000) }
  Start-Sleep 2
  Write-Host "[PASS] Test 1" -ForegroundColor Green
  $p++
} catch {
  Write-Host "[FAIL] Test 1: $_" -ForegroundColor Red
  $f++
  Kill-All-LRC
}

# ==== Test 2: Single Window Reject ====
Write-Host ""
Write-Host "[Test 2/6] Single Window Reject" -ForegroundColor Yellow
Kill-All-LRC
try {
  $d = Join-Path $testRoot "t2"; New-Item -ItemType Directory -Force $d | Out-Null
  $p1 = Start-LRC $src $d 3231 @()
  $ok = Wait-Health 3231
  if (-not $ok) { throw "P1 failed" }
  Write-Host "  Lock: $(Get-Content (Join-Path $d '.lrc.lock') -Raw)"
  $e2 = Join-Path $d "err2.txt"
  $p2 = Start-Process -FilePath $exe -ArgumentList "--src-dir",$src,"--db-path",$d,"--port","3232","--mode","fast" -WindowStyle Hidden -PassThru -Wait -RedirectStandardError $e2
  $ex2 = $p2.ExitCode
  Write-Host "  P2 exit: $ex2 (expected: non-zero)"
  if ($ex2 -eq 0) { throw "2nd not rejected" }
  if (-not $p1.HasExited) { $p1.Kill(); $p1.WaitForExit(3000) }
  Start-Sleep 2
  Write-Host "[PASS] Test 2" -ForegroundColor Green
  $p++
} catch {
  Write-Host "[FAIL] Test 2: $_" -ForegroundColor Red
  $f++
  Kill-All-LRC
}

# ==== Test 3: Multi-Window 3 ====
Write-Host ""
Write-Host "[Test 3/6] Multi-Window 3" -ForegroundColor Yellow
Kill-All-LRC
try {
  $d = Join-Path $testRoot "t3"; New-Item -ItemType Directory -Force $d | Out-Null
  $procs = @()
  for ($i = 0; $i -lt 3; $i++) {
    $port = 3241 + $i
    Write-Host "  Window $($i+1) on $port"
    $pp = Start-LRC $src $d $port @("--multi-window","3")
    $ok = Wait-Health $port
    if (-not $ok) { throw "Window $($i+1) failed" }
    $procs += $pp
    Start-Sleep 1
  }
  $lf = Join-Path $d ".lrc.lock"
  $lc = Get-Content $lf -Raw
  $cnt = ($lc -split "," | Where-Object { $_ -match '\d+' }).Count
  Write-Host "  Lock: $lc ($cnt PIDs)"
  if ($cnt -lt 3) { throw "Expected 3 PIDs, got $cnt" }
  for ($i = 2; $i -ge 0; $i--) {
    if (-not $procs[$i].HasExited) { $procs[$i].Kill(); $procs[$i].WaitForExit(3000) }
    Start-Sleep 1
  }
  Start-Sleep 2
  Write-Host "[PASS] Test 3" -ForegroundColor Green
  $p++
} catch {
  Write-Host "[FAIL] Test 3: $_" -ForegroundColor Red
  $f++
  Kill-All-LRC
}

# ==== Test 4: Multi-Window Overflow ====
Write-Host ""
Write-Host "[Test 4/6] Multi-Window Overflow" -ForegroundColor Yellow
Kill-All-LRC
try {
  $d = Join-Path $testRoot "t4"; New-Item -ItemType Directory -Force $d | Out-Null
  $procs = @()
  for ($i = 0; $i -lt 2; $i++) {
    $port = 3251 + $i
    Write-Host "  Window $($i+1) on $port"
    $pp = Start-LRC $src $d $port @("--multi-window","2")
    $ok = Wait-Health $port
    if (-not $ok) { throw "Window $($i+1) failed" }
    $procs += $pp
    Start-Sleep 1
  }
  Write-Host "  3rd window (should fail)..."
  $e3 = Join-Path $d "err3.txt"
  $p3 = Start-Process -FilePath $exe -ArgumentList "--src-dir",$src,"--db-path",$d,"--port","3253","--mode","fast","--multi-window","2" -WindowStyle Hidden -PassThru -Wait -RedirectStandardError $e3
  Write-Host "  3rd exit: $($p3.ExitCode)"
  if ($p3.ExitCode -eq 0) { throw "3rd not rejected" }
  foreach ($pp in $procs) {
    if (-not $pp.HasExited) { $pp.Kill(); $pp.WaitForExit(3000) }
  }
  Start-Sleep 2
  Write-Host "[PASS] Test 4" -ForegroundColor Green
  $p++
} catch {
  Write-Host "[FAIL] Test 4: $_" -ForegroundColor Red
  $f++
  Kill-All-LRC
}

# ==== Test 5: Port Auto-Adapt ====
Write-Host ""
Write-Host "[Test 5/6] Port Auto-Adapt" -ForegroundColor Yellow
Kill-All-LRC
try {
  $d = Join-Path $testRoot "t5"; New-Item -ItemType Directory -Force $d | Out-Null
  $tcp = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, 3401)
  $tcp.Start() | Out-Null
  Write-Host "  Port 3401 blocked"
  $pp = Start-LRC $src $d 3401 @()
  Start-Sleep 25
  # 逐个端口扫描，避免 PowerShell 5.1 嵌套 try/break 解析问题
  $found = $false
  $adaptedPort = 0
  $ports = @(3402, 3403, 3404, 3405, 3406, 3407, 3408, 3409, 3410)
  foreach ($scanPort in $ports) {
    $scanResult = $false
    try {
      $r = Invoke-WebRequest "http://127.0.0.1:$scanPort/health" -TimeoutSec 3 -UseBasicParsing
      if ($r.StatusCode -eq 200) { $scanResult = $true }
    } catch { }
    if ($scanResult) {
      Write-Host ("  [OK] Adapted to " + $scanPort) -ForegroundColor Green
      $found = $true
      $adaptedPort = $scanPort
      break
    }
    Write-Host ("  Port " + $scanPort + ": not available") -ForegroundColor DarkGray
    Start-Sleep 1
  }
  if (-not $found) {
    $errOut = Get-Content (Join-Path $d "err.txt") -Raw -ErrorAction SilentlyContinue
    $alive = -not $pp.HasExited
    Write-Host "  [DEBUG] Process alive: $alive" -ForegroundColor DarkGray
    if ($errOut) { Write-Host ("  [DEBUG] stderr: " + $errOut.Substring([Math]::Max(0, $errOut.Length - 200))) -ForegroundColor DarkGray }
  }
  $tcp.Stop()
  if (-not $pp.HasExited) { $pp.Kill(); $pp.WaitForExit(3000) }
  Start-Sleep 2
  if (-not $found) { throw "Port auto-adapt failed" }
  Write-Host ("[PASS] Test 5 (adapted to " + $adaptedPort + ")") -ForegroundColor Green
  $p++
} catch {
  Write-Host "[FAIL] Test 5: $_" -ForegroundColor Red
  $f++
  Kill-All-LRC
}

# ==== Test 6: Dead PID Cleanup ====
Write-Host ""
Write-Host "[Test 6/6] Dead PID Cleanup" -ForegroundColor Yellow
Kill-All-LRC
try {
  $d = Join-Path $testRoot "t6"; New-Item -ItemType Directory -Force $d | Out-Null
  $lf = Join-Path $d ".lrc.lock"
  Set-Content $lf "99999" -NoNewline
  Write-Host "  Wrote dead PID 99999"
  $pp = Start-LRC $src $d 3271 @()
  $ok = Wait-Health 3271
  if (-not $ok) { throw "LRC failed" }
  $lc = Get-Content $lf -Raw
  Write-Host "  Lock: $lc"
  if ($lc -match "99999") { throw "Dead PID still in lock" }
  if (-not $pp.HasExited) { $pp.Kill(); $pp.WaitForExit(3000) }
  Start-Sleep 2
  Write-Host "[PASS] Test 6" -ForegroundColor Green
  $p++
} catch {
  Write-Host "[FAIL] Test 6: $_" -ForegroundColor Red
  $f++
  Kill-All-LRC
}

# ==== Results ====
Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
$t = $p + $f
if ($f -eq 0) {
  Write-Host "  ALL $t/$t TESTS PASSED!" -ForegroundColor Green
} else {
  Write-Host "  $p passed, $f failed" -ForegroundColor Red
}
Write-Host "========================================" -ForegroundColor Cyan

Kill-All-LRC
Remove-Item -Recurse -Force $testRoot -ErrorAction SilentlyContinue
if ($f -gt 0) { exit 1 } else { exit 0 }