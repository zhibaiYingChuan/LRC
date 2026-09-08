param(
    [string]$File = "G:\code-memory\scripts\life_memories_batch.json"
)
# 生活类记忆批量写入脚本（dev 库 3111）
# 用途：为联想中心提供真实生活数据，验证联想展现效果
# 用法：.\add_life_memories.ps1 [-File <批次json路径>]，默认第一批
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$data = Get-Content $File -Raw -Encoding UTF8 | ConvertFrom-Json
$ok = 0; $fail = 0; $dup = 0
foreach ($m in $data.memories) {
    $body = @{
        content      = $m.content
        memory_type  = $m.memory_type
        tags         = $m.tags
        importance   = $m.importance
    } | ConvertTo-Json -Compress
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($body)
    try {
        $resp = Invoke-RestMethod -Uri "http://127.0.0.1:3111/v1/memories/remember" -Method Post -Body $bytes -ContentType "application/json; charset=utf-8" -TimeoutSec 10
        if ($resp.duplicate) { $dup++ } else { $ok++ }
    } catch {
        $fail++
        Write-Host "[FAIL] $($m.content.Substring(0, [Math]::Min(20, $m.content.Length))) -> $($_.Exception.Message)"
    }
}
Write-Host "write done: ok=$ok dup=$dup fail=$fail total=$($data.memories.Count)"