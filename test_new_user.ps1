# ============================================================
# 新用户完整模拟测试脚本 (v2 - 修正响应格式)
# ============================================================
param(
    [string]$BaseUrl = "http://127.0.0.1:3099"
)

$ErrorActionPreference = "Continue"
$totalTests = 0
$passedTests = 0
$failedTests = 0

function Test-Step {
    param([string]$Name, [ScriptBlock]$Test)
    $global:totalTests++
    Write-Host "  [$totalTests] $Name ... " -NoNewline
    try {
        $result = & $Test
        if ($result -is [bool] -and -not $result) {
            throw "assertion failed"
        }
        Write-Host "PASS" -ForegroundColor Green
        $global:passedTests++
        return $result
    } catch {
        Write-Host "FAIL: $_" -ForegroundColor Red
        $global:failedTests++
        return $null
    }
}

function Invoke-MCP {
    param([string]$ToolName, [hashtable]$Arguments, $Id = 1)
    $body = @{
        jsonrpc = "2.0"
        id = $Id
        method = "tools/call"
        params = @{
            name = $ToolName
            arguments = $Arguments
        }
    } | ConvertTo-Json -Depth 5 -Compress
    $resp = Invoke-RestMethod -Uri "$BaseUrl/mcp" -Method Post -ContentType "application/json" -Body $body
    return $resp.result
}

function Invoke-API {
    param([string]$Path, [hashtable]$Body, [string]$Method = "Post")
    $uri = "$BaseUrl$Path"
    $json = $Body | ConvertTo-Json -Depth 5 -Compress
    if ($Method -eq "Get") {
        return Invoke-RestMethod -Uri $uri -Method Get
    }
    return Invoke-RestMethod -Uri $uri -Method $Method -ContentType "application/json" -Body $json
}

# 从响应文本中提取记忆 ID
# 支持三种格式:
#   格式1: (ID: xxx)          — remember 响应
#   格式2: ID: `xxx`          — list_memories 响应
#   格式3: ID: xxx            — 纯文本
function Extract-MemoryId([string]$Text) {
    if ($Text -match '\(ID:\s*([a-f0-9-]+)\)') {
        return $Matches[1]
    }
    if ($Text -match 'ID:\s*`([a-f0-9-]+)`') {
        return $Matches[1]
    }
    if ($Text -match 'ID:\s*([a-f0-9-]+)') {
        return $Matches[1]
    }
    return $null
}

# ============================================================
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  Loong Recall New User Simulation Test" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan

# === 0. Health check ===
Write-Host "`n--- 0. Health Check ---" -ForegroundColor Yellow
Test-Step "GET /health returns text" {
    $r = Invoke-RestMethod -Uri "$BaseUrl/health"
    return ($r -is [string]) -and ($r.Length -gt 0)
}

# === 1. MCP initialize ===
Write-Host "`n--- 1. MCP Handshake ---" -ForegroundColor Yellow
Test-Step "initialize handshake" {
    $body = @{jsonrpc="2.0"; id=0; method="initialize"; params=@{protocolVersion="2024-11-05"; capabilities=@{}}} | ConvertTo-Json -Depth 5 -Compress
    $r = Invoke-RestMethod -Uri "$BaseUrl/mcp" -Method Post -ContentType "application/json" -Body $body
    return $null -ne $r.result
}

# === 2. Tools list ===
Write-Host "`n--- 2. MCP Tools List (expect 12) ---" -ForegroundColor Yellow
$toolsList = $null
Test-Step "tools/list returns 12 tools" {
    $body = @{jsonrpc="2.0"; id=0; method="tools/list"; params=@{}} | ConvertTo-Json -Depth 5 -Compress
    $r = Invoke-RestMethod -Uri "$BaseUrl/mcp" -Method Post -ContentType "application/json" -Body $body
    $toolsList = $r.result.tools
    return $toolsList.Count -eq 12
}

if ($toolsList) {
    Write-Host "  Actual tools:"
    $toolsList | ForEach-Object { Write-Host "    - $($_.name)" }
}

# === 3. Code search ===
Write-Host "`n--- 3. Code Search Tools ---" -ForegroundColor Yellow
Test-Step "search_code returns results" {
    $r = Invoke-MCP -ToolName "search_code" -Arguments @{query="luoshu"; top_k=3}
    $c = $r.content[0].text
    return ($c -match "luoshu" -or $c -match "LuoShu")
}

Test-Step "codebase_stats returns stats" {
    $r = Invoke-MCP -ToolName "codebase_stats" -Arguments @{}
    $c = $r.content[0].text
    return ($c.Length -gt 10)
}

# === 4. Memory tools ===
Write-Host "`n--- 4. Memory Management Tools ---" -ForegroundColor Yellow

# 4.1 remember
$memoryId = $null
Test-Step "remember writes memory" {
    $r = Invoke-MCP -ToolName "remember" -Arguments @{
        content="Project uses pnpm as package manager"
        memory_type="preference"
        tags=@("tooling", "pnpm")
        importance=7
    }
    $c = $r.content[0].text
    $id = Extract-MemoryId $c
    if ($id) {
        $global:memoryId = $id
    }
    return ($null -ne $id)
}
Write-Host "    Memory ID: $memoryId"

Test-Step "remember second memory" {
    $r = Invoke-MCP -ToolName "remember" -Arguments @{
        content="Database uses PostgreSQL 16"
        memory_type="decision"
        tags=@("database", "postgresql")
        importance=8
    }
    $c = $r.content[0].text
    return ($c -match "PostgreSQL")
}

Test-Step "remember with session privacy" {
    $r = Invoke-MCP -ToolName "remember" -Arguments @{
        content="This session uses test port 3099"
        memory_type="fact"
        privacy_level="session"
        session_id="test-session-001"
    }
    $c = $r.content[0].text
    return ($c -match "3099")
}

# 4.2 recall
Test-Step "recall finds memory" {
    $r = Invoke-MCP -ToolName "recall" -Arguments @{query="package manager"; top_k=5}
    $c = $r.content[0].text
    return ($c -match "pnpm")
}

Test-Step "recall with type filter" {
    $r = Invoke-MCP -ToolName "recall" -Arguments @{query="database"; memory_type="decision"; top_k=5}
    $c = $r.content[0].text
    return ($c -match "PostgreSQL")
}

# 4.3 list_memories
Test-Step "list_memories pagination" {
    $r = Invoke-MCP -ToolName "list_memories" -Arguments @{limit=10; offset=0}
    $c = $r.content[0].text
    return ($c.Length -gt 10)
}

# 4.4 memory_stats
Test-Step "memory_stats returns stats" {
    $r = Invoke-MCP -ToolName "memory_stats" -Arguments @{}
    $c = $r.content[0].text
    return ($c.Length -gt 10)
}

# 4.5 update_memory
Test-Step "update_memory updates content" {
    if (-not $memoryId) { throw "no memory id" }
    $r = Invoke-MCP -ToolName "update_memory" -Arguments @{
        memory_id=$memoryId
        content="Project uses pnpm as package manager (confirmed)"
    }
    $c = $r.content[0].text
    return ($c -match "pnpm")
}

# 4.6 correct_memory
Test-Step "correct_memory preserves history" {
    if (-not $memoryId) { throw "no memory id" }
    $r = Invoke-MCP -ToolName "correct_memory" -Arguments @{
        memory_id=$memoryId
        content="Project uses pnpm (user corrected)"
        reason="User manual correction"
    }
    $c = $r.content[0].text
    return ($c -match "pnpm" -or $c -match "correct")
}

# 4.7 recall_enhanced
Test-Step "recall_enhanced dual-path RRF" {
    $r = Invoke-MCP -ToolName "recall_enhanced" -Arguments @{query="package tool"; top_k=5}
    $c = $r.content[0].text
    return ($c.Length -gt 10)
}

# 4.8 dao_metrics
Test-Step "dao_metrics health indicators" {
    $r = Invoke-MCP -ToolName "dao_metrics" -Arguments @{}
    $c = $r.content[0].text
    return ($c.Length -gt 10)
}

# 4.9 archive
Test-Step "archive expired memories" {
    $r = Invoke-MCP -ToolName "archive" -Arguments @{}
    $c = $r.content[0].text
    return ($c.Length -gt 0)
}

# 4.10 forget
Test-Step "forget deletes memory" {
    if (-not $memoryId) { throw "no memory id" }
    $r = Invoke-MCP -ToolName "forget" -Arguments @{memory_id=$memoryId}
    $c = $r.content[0].text
    return ($c.Length -gt 0)
}

# === 5. v1 REST API ===
Write-Host "`n--- 5. v1 REST API Endpoints ---" -ForegroundColor Yellow

# 5.1 encode
$baguaCategory = $null
$encodeResult = $null
Test-Step "POST /v1/encode text->luoshu vector" {
    $global:encodeResult = Invoke-API -Path "/v1/encode" -Body @{text="package manager preference"}
    return ($encodeResult.luoshu_vector.Count -eq 9) -and ($null -ne $encodeResult.bagua_category)
}
Write-Host "    Bagua: $($encodeResult.bagua_category), depth: $($encodeResult.topological_depth)"

# 5.2 consolidate
Test-Step "POST /v1/memories/consolidate" {
    $r = Invoke-API -Path "/v1/memories/consolidate" -Body @{
        memories=@(
            @{content="Surface memory test 1"; timestamp="2026-06-03T10:00:00Z"},
            @{content="Surface memory test 2"; timestamp="2026-06-03T10:01:00Z"}
        )
    }
    return ($null -ne $r)
}

# 5.3 enrich
Test-Step "POST /v1/memories/enrich" {
    $r = Invoke-API -Path "/v1/memories/enrich" -Body @{query="package manager"; top_k=5}
    return ($null -ne $r)
}

# 5.4 correct
Test-Step "POST /v1/memories/correct (existing id)" {
    $rList = Invoke-MCP -ToolName "list_memories" -Arguments @{limit=1; offset=0}
    $c = $rList.content[0].text
    $testId = Extract-MemoryId $c
    if ($testId) {
        $r = Invoke-API -Path "/v1/memories/correct" -Body @{
            memory_id=$testId
            content="Corrected via v1 API test"
            reason="API integration test"
        }
        return ($null -ne $r)
    }
    throw "cannot find any memory id"
}

# 5.5 unfold
Test-Step "POST /v1/memories/unfold" {
    $rList = Invoke-MCP -ToolName "list_memories" -Arguments @{limit=1; offset=0}
    $c = $rList.content[0].text
    $testId = Extract-MemoryId $c
    if ($testId) {
        $r = Invoke-API -Path "/v1/memories/unfold" -Body @{memory_id=$testId}
        return ($null -ne $r)
    }
    throw "cannot find any memory id"
}

# 5.6 dao_metrics
Test-Step "GET /v1/health/dao_metrics" {
    $r = Invoke-API -Path "/v1/health/dao_metrics" -Method Get -Body @{}
    return ($null -ne $r.dao_isomorphism_score) -or ($null -ne $r.bagua_entropy)
}

# ============================================================
# Summary
# ============================================================
Write-Host "`n============================================================" -ForegroundColor Cyan
Write-Host "  Test Results Summary" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  Total:  $totalTests" -ForegroundColor White
Write-Host "  Passed: $passedTests" -ForegroundColor Green
if ($failedTests -gt 0) {
    Write-Host "  Failed: $failedTests" -ForegroundColor Red
} else {
    Write-Host "  Failed: $failedTests" -ForegroundColor White
}
$passRate = if ($totalTests -gt 0) { [math]::Round($passedTests / $totalTests * 100, 1) } else { 0 }
Write-Host "  Rate:   $passRate%" -ForegroundColor $(if ($failedTests -eq 0) { "Green" } else { "Yellow" })
Write-Host "============================================================`n" -ForegroundColor Cyan

exit $failedTests