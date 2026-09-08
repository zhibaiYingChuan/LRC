# 联想质量 API 验证脚本（dev 3111）
# 验证三个场景：食物联想 / 语义旁路（重要日子）/ 诚实空态（无关查询）
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$queries = @('今晚吃什么？', '重要的日子', '量子物理是什么')
foreach ($q in $queries) {
    $body = @{ query = $q; depth = 4; width = 3 } | ConvertTo-Json -Compress
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($body)
    try {
        $resp = Invoke-RestMethod -Uri "http://127.0.0.1:3111/v1/associations/explore" -Method Post -Body $bytes -ContentType "application/json; charset=utf-8" -TimeoutSec 20
        $nodes = @($resp.nodes)
        Write-Host "`n=== 查询: $q ==="
        Write-Host ("weak_match: {0} | nodes: {1} | edges: {2}" -f $resp.weak_match, $nodes.Count, @($resp.edges).Count)
        $i = 0
        foreach ($n in $nodes) {
            if ($i -ge 5) { break }
            $content = $n.content
            if ($content -and $content.Length -gt 40) { $content = $content.Substring(0, 40) }
            Write-Host ("  [{0}] ({1}) {2}" -f $n.id, $n.memory_type, $content)
            $i++
        }
    } catch {
        Write-Host "`n=== 查询: $q === FAIL: $($_.Exception.Message)"
    }
}
