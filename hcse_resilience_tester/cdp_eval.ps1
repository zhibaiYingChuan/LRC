# HCSE CDP 直连评估脚本 v2 — 通过 WebSocket 连接 WebView2 CDP 端点
# 验证 Phase 3：运行时验证 RV-Monitor
# 兼容 PowerShell 5.1（不依赖 Add-Type，直接用类型限定名）
param(
    [string]$CdpUrl = 'ws://127.0.0.1:9223/devtools/page/4D97F19F9CFC09E11622D78D4BB00803',
    [string]$ScriptFile
)

# UTF-8 输出编码（修复中文乱码）
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}
$OutputEncoding = [System.Text.Encoding]::UTF8

if (-not $ScriptFile) { Write-Error '必须指定 -ScriptFile'; exit 1 }
$jsExpression = Get-Content -Raw -Path $ScriptFile

# Phase 6 安全沙箱：路径白名单校验
$allowedRoots = @('g:\code-memory\hcse_resilience_tester', 'g:\code-memory\temp', 'g:\code-memory\logs')
$scriptFullPath = (Resolve-Path $ScriptFile).Path
$inWhitelist = $false
foreach ($root in $allowedRoots) {
    if ($scriptFullPath -like "$root*") { $inWhitelist = $true; break }
}
if (-not $inWhitelist) {
    Write-Error "[PathValidator] 拒绝执行白名单外脚本: $scriptFullPath (HCSE Phase 6 Hard Halt)"
    exit 2
}

# Phase 6 数据脱敏
function Invoke-DataSanitization {
    param([string]$Text)
    if (-not $Text) { return $Text }
    $Text = [regex]::Replace($Text, '"value"\s*:\s*"[^"]*"', '"value":"[COOKIE_VALUE_REDACTED]"', 'IgnoreCase')
    $Text = [regex]::Replace($Text, '(?i)(authorization["\s:]+bearer\s+)[A-Za-z0-9\-_\.]+', '$1[BEARER_TOKEN_REDACTED]')
    $Text = [regex]::Replace($Text, '(?i)"authorization"\s*:\s*"[^"]*"', '"authorization":"[BEARER_TOKEN_REDACTED]"')
    $Text = [regex]::Replace($Text, '[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}', '[EMAIL_REDACTED]')
    $Text = [regex]::Replace($Text, '(?<![0-9])1[3-9][0-9]{9}(?![0-9])', '[PHONE_REDACTED]')
    return $Text
}

# Phase 6 资源看门狗 MAX_CPU_TIME=60s
$global:maxCpuSec = 60
$global:watchdog = [System.Diagnostics.Stopwatch]::StartNew()

try {
    # 直接用类型限定名创建 ClientWebSocket（PS 5.1 兼容）
    $ws = New-Object 'System.Net.WebSockets.ClientWebSocket'
    $cts = New-Object System.Threading.CancellationTokenSource
    $cts.CancelAfter([TimeSpan]::FromSeconds(30))

    Write-Host "[CDP] connecting $CdpUrl"
    $connectTask = $ws.ConnectAsync([Uri]$CdpUrl, $cts.Token)
    while (-not $connectTask.IsCompleted) {
        if ($watchdog.Elapsed.TotalSeconds -gt $global:maxCpuSec) {
            Write-Error "[Watchdog] > MAX_CPU_TIME=$($global:maxCpuSec)s Hard Halt"
            $ws.Abort(); exit 3
        }
        Start-Sleep -Milliseconds 100
    }
    if ($connectTask.IsFaulted) {
        $errMsg = $connectTask.Exception.InnerException.Message
        if (-not $errMsg) { $errMsg = $connectTask.Exception.Message }
        Write-Error "[CDP] connect failed: $errMsg"
        exit 4
    }
    Write-Host "[CDP] connected state=$($ws.State)"

    $recvBuf = New-Object byte[] 262144
    $recvSeg = New-Object 'System.ArraySegment[byte]' ($recvBuf, 0, $recvBuf.Length)

    # Phase 3 RV-Monitor：CDP 存活预检
    $livenessPayload = @{ id = 1; method = 'Browser.getVersion'; params = @{} } | ConvertTo-Json -Compress
    $livBytes = [Text.Encoding]::UTF8.GetBytes($livenessPayload)
    $livSeg = New-Object 'System.ArraySegment[byte]' ($livBytes, 0, $livBytes.Length)
    $null = $ws.SendAsync($livSeg, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $cts.Token)
    $recvTask = $ws.ReceiveAsync($recvSeg, $cts.Token)
    $w1 = 0
    while (-not $recvTask.IsCompleted -and $w1 -lt 10000) { Start-Sleep -Milliseconds 50; $w1 += 50 }
    if ($recvTask.IsCompleted) {
        $livResp = [Text.Encoding]::UTF8.GetString($recvBuf, 0, $recvTask.Result.Count)
        Write-Host "[CDP Liveness] $livResp"
    } else {
        Write-Host "[CDP Liveness] TIMEOUT — CDP channel may be dead"
    }

    # 执行目标 JS
    $evalPayload = @{
        id = 2
        method = 'Runtime.evaluate'
        params = @{ expression = $jsExpression; returnByValue = $true; awaitPromise = $true; timeout = 20000 }
    } | ConvertTo-Json -Depth 10
    $evalBytes = [Text.Encoding]::UTF8.GetBytes($evalPayload)
    $evalSeg = New-Object 'System.ArraySegment[byte]' ($evalBytes, 0, $evalBytes.Length)
    $null = $ws.SendAsync($evalSeg, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $cts.Token)

    $recvTask2 = $ws.ReceiveAsync($recvSeg, $cts.Token)
    $w2 = 0
    while (-not $recvTask2.IsCompleted -and $w2 -lt 25000) { Start-Sleep -Milliseconds 100; $w2 += 100 }
    if ($recvTask2.IsCompleted) {
        $evalResp = [Text.Encoding]::UTF8.GetString($recvBuf, 0, $recvTask2.Result.Count)
        # 大响应可能分片，循环读取直到 EndOfMessage
        $fullResp = $evalResp
        while ($recvTask2.Result -and -not $recvTask2.Result.EndOfMessage -and $fullResp.Length -lt 1000000) {
            $recvTask3 = $ws.ReceiveAsync($recvSeg, $cts.Token)
            $w3 = 0
            while (-not $recvTask3.IsCompleted -and $w3 -lt 5000) { Start-Sleep -Milliseconds 50; $w3 += 50 }
            if ($recvTask3.IsCompleted) {
                $fullResp += [Text.Encoding]::UTF8.GetString($recvBuf, 0, $recvTask3.Result.Count)
                $recvTask2 = $recvTask3
            } else { break }
        }
        $fullResp = Invoke-DataSanitization $fullResp
        Write-Host "[CDP Eval Result]"
        Write-Host $fullResp
    } else {
        Write-Host "[CDP Eval] TIMEOUT 25s"
    }

    try { $null = $ws.CloseAsync([System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure, 'done', $cts.Token) } catch {}
    Start-Sleep -Milliseconds 200
}
catch {
    Write-Error "[CDP] exception: $($_.Exception.Message)"
    exit 5
}
finally {
    if ($ws) { $ws.Dispose() }
    $watchdog.Stop()
    Write-Host "[Watchdog] total: $([math]::Round($watchdog.Elapsed.TotalSeconds,2))s"
}
