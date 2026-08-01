# WebView2 CDP 简单测试脚本
$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}

$wsUrl = (Invoke-RestMethod -Uri 'http://127.0.0.1:9223/json' -UseBasicParsing -TimeoutSec 5)[0].webSocketDebuggerUrl
Write-Host "WS URL: $wsUrl"

# 连接 WebSocket
Add-Type -AssemblyName System.Net.WebSockets
$ws = New-Object System.Net.WebSockets.ClientWebSocket
$cts = New-Object System.Threading.CancellationTokenSource(15000)
$connectTask = $ws.ConnectAsync([Uri]$wsUrl, $cts.Token)
while (-not $connectTask.IsCompleted) { Start-Sleep -Milliseconds 100 }
if ($ws.State -ne 'Open') { Write-Host "连接失败"; exit 1 }
Write-Host "已连接" -ForegroundColor Green

# 发送 CDP Runtime.evaluate 命令
$jsCode = "JSON.stringify({ title: document.title, url: location.href, bodyLen: document.body?.innerText?.length || 0 })"
$cdpCmd = @{ id = 1; method = 'Runtime.evaluate'; params = @{ expression = $jsCode; returnByValue = $true } } | ConvertTo-Json -Compress
Write-Host "发送: $cdpCmd"

$bytes = [Text.Encoding]::UTF8.GetBytes($cdpCmd)
$seg = [System.ArraySegment[byte]]::new($bytes)
$null = $ws.SendAsync($seg, 'Text', $true, $cts.Token)

# 接收响应
$buf = New-Object byte[] 65536
$rseg = [System.ArraySegment[byte]]::new($buf)
$result = ""
$timeout = 0
while ($timeout -lt 100) {
    if ($ws.State -ne 'Open') { break }
    $rtask = $ws.ReceiveAsync($rseg, $cts.Token)
    $wait = 0
    while (-not $rtask.IsCompleted -and $wait -lt 5000) { Start-Sleep -Milliseconds 50; $wait += 50 }
    if ($rtask.IsCompleted) {
        $result += [Text.Encoding]::UTF8.GetString($buf, 0, $rtask.Result.Count)
        if ($rtask.Result.EndOfMessage) { break }
    }
    $timeout++
}
Write-Host "响应: $result"

# 解析结果
try {
    $obj = $result | ConvertFrom-Json
    if ($obj.result) {
        Write-Host "=== 页面状态 ===" -ForegroundColor Cyan
        $state = $obj.result.result.value | ConvertFrom-Json
        $state | Format-List
    } elseif ($obj.error) {
        Write-Host "CDP错误: $($obj.error | ConvertTo-Json)" -ForegroundColor Red
    }
} catch { Write-Host "解析失败: $result" }

$ws.Dispose()
Write-Host "完成" -ForegroundColor Green