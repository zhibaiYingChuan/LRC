# LRC LongMemEval-S fast 232 实例复现脚本
#
# 口径：500 实例取前 250，跳过 18 条 abstention，有效 232；3 分片并行；Top-K=10。
# 重要：fast 模式必须使用**非 ML 构建**，否则写入会强制语义编码，出现 404s/实例假慢。
#   构建：cargo build --release --features server
#
# 用法（在仓库根目录）：
#   pwsh eval/longmemeval/scripts/run_fast_232.ps1 `
#     -Exe target/release/code-memory-server.exe `
#     -Data eval/longmemeval/data/longmemeval_s_cleaned.json
#
# 可选：-Python python -NShards 3 -Max 250 -TopK 10 -MaxInjectSeconds 30
param(
    [string]$Exe = "",
    [string]$Data = "",
    [string]$OutDir = "",
    [string]$DataDir = "",
    [string]$Python = "python",
    [int]$NShards = 3,
    [int]$Max = 250,
    [int]$TopK = 10,
    [double]$MaxInjectSeconds = 30
)

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$EvalDir = Resolve-Path (Join-Path $ScriptDir '..')
$RepoRoot = Split-Path -Parent (Split-Path -Parent $EvalDir)

if (-not $Exe) {
    $exeName = if ($env:OS -eq 'Windows_NT') { 'code-memory-server.exe' } else { 'code-memory-server' }
    $Exe = Join-Path (Join-Path $RepoRoot 'target/release') $exeName
}
if (-not $Data) { $Data = Join-Path (Join-Path $EvalDir 'data') 'longmemeval_s_cleaned.json' }
if (-not $OutDir) { $OutDir = Join-Path $EvalDir 'logs' }
if (-not $DataDir) { $DataDir = Join-Path (Join-Path $EvalDir 'data_eval') 'fast_232' }

$configPath = Join-Path (Join-Path $EvalDir 'configs') 'fast_232.json'
$expectedPath = Join-Path (Join-Path $EvalDir 'expected') 'fast_232.json'
if (-not (Test-Path $configPath)) { throw "口径配置不存在: $configPath" }
if (-not (Test-Path $expectedPath)) { throw "期望值文件不存在: $expectedPath" }
$config = Get-Content -Raw -Encoding UTF8 $configPath | ConvertFrom-Json
$expected = Get-Content -Raw -Encoding UTF8 $expectedPath | ConvertFrom-Json

if (-not (Test-Path $Exe)) {
    throw "sidecar 不存在: $Exe`n请先构建非 ML 版本：cargo build --release --features server"
}
if (-not (Test-Path $Data)) {
    throw "数据集不存在: $Data`n请先运行：python eval/longmemeval/scripts/download_data.py"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Get-ChildItem (Join-Path $OutDir 'fast_232_s*.log') -ErrorAction SilentlyContinue | Remove-Item -Force

# Start-Process 在 PowerShell 5 下不会自动给含空格的参数加引号，这里显式处理。
function ConvertTo-QuotedArg([string]$Value) {
    if ($Value -match '\s') { return '"' + $Value + '"' }
    return $Value
}

$jobs = @()
for ($i = 0; $i -lt $NShards; $i++) {
    $out = Join-Path $OutDir "fast_232_s$i.log"
    $argList = @(
        (Join-Path $ScriptDir 'lrc_fair_eval_v3.py'),
        '--mode', 'fast',
        '--max', "$Max",
        '--top-k', "$TopK",
        '--shard', "$i",
        '--n-shards', "$NShards",
        '--data', $Data,
        '--data-sha256', "$($config.dataset_sha256)",
        '--exe', $Exe,
        '--data-dir', $DataDir,
        '--log', $out,
        '--max-inject-seconds', "$MaxInjectSeconds"
    )
    $argString = ($argList | ForEach-Object { ConvertTo-QuotedArg $_ }) -join ' '
    $startArgs = @{
        FilePath = $Python
        ArgumentList = $argString
        WorkingDirectory = $ScriptDir
        PassThru = $true
    }
    if ($env:OS -eq 'Windows_NT') { $startArgs['WindowStyle'] = 'Hidden' }
    $proc = Start-Process @startArgs
    $jobs += $proc
    Write-Output "shard $i pid=$($proc.Id) log=$out"
}

Write-Output "launched $($jobs.Count) shards; waiting..."
$jobs | Wait-Process

$failed = @()
foreach ($job in $jobs) {
    $job.Refresh()
    if ($job.ExitCode -ne 0) {
        $failed += "pid=$($job.Id) exit=$($job.ExitCode)"
    }
}
if ($failed.Count -gt 0) {
    throw "分片执行失败: $($failed -join '; ')。请查看 $OutDir 下的日志。"
}

# 汇总并与 expected/fast_232.json 逐项比对；任何缺片/缺字段/偏离都视为失败。
function Get-LogMetric([string]$Text, [string]$Pattern, [string]$Name, [string]$LogPath) {
    $m = [regex]::Match($Text, $Pattern)
    if (-not $m.Success) {
        throw "日志 $LogPath 缺少字段：$Name"
    }
    return $m.Groups[1].Value
}

$total = 0
$sr = 0.0
$tr = 0.0
$sm = 0.0
$tm = 0.0
foreach ($i in 0..($NShards - 1)) {
    $log = Join-Path $OutDir "fast_232_s$i.log"
    if (-not (Test-Path $log)) {
        throw "缺少分片日志: $log（该分片可能启动失败）"
    }
    # Windows PowerShell 5 默认按 ANSI 解码；日志是 UTF-8，必须显式指定。
    $text = Get-Content $log -Raw -Encoding UTF8
    $n = [int](Get-LogMetric $text '评估实例数:\s*(\d+)' '评估实例数' $log)
    if ($n -le 0) { throw "分片 $i 的评估实例数为 0：$log" }
    $s = [double](Get-LogMetric $text 'Session Recall@10:\s*([0-9.]+)' 'Session Recall@10' $log)
    $t = [double](Get-LogMetric $text 'Turn Recall@10:\s*([0-9.]+)' 'Turn Recall@10' $log)
    $smv = [double](Get-LogMetric $text 'Session MRR:\s*([0-9.]+)' 'Session MRR' $log)
    $tmv = [double](Get-LogMetric $text 'Turn MRR:\s*([0-9.]+)' 'Turn MRR' $log)
    $total += $n
    $sr += $s * $n
    $tr += $t * $n
    $sm += $smv * $n
    $tm += $tmv * $n
}

$wantN = [int]$expected.n_evaluated
if ($total -ne $wantN) {
    throw "有效实例数 $total != expected $wantN；分片不完整或数据/筛选口径不一致。"
}

$actual = @{
    'Session Recall@10' = $sr / $total
    'Turn Recall@10' = $tr / $total
    'Session MRR' = $sm / $total
    'Turn MRR' = $tm / $total
}
$wanted = @{
    'Session Recall@10' = [double]$expected.session_recall_at_10
    'Turn Recall@10' = [double]$expected.turn_recall_at_10
    'Session MRR' = [double]$expected.session_mrr
    'Turn MRR' = [double]$expected.turn_mrr
}
$recallTol = [double]$expected.tolerance_pp / 100.0
$mrrTol = 0.02
$deviations = @()

Write-Output ("AGGREGATE n={0} SessionR@10={1:P2} TurnR@10={2:P2} SessionMRR={3:F4} TurnMRR={4:F4}" -f `
    $total, $actual['Session Recall@10'], $actual['Turn Recall@10'], $actual['Session MRR'], $actual['Turn MRR'])

foreach ($key in $actual.Keys) {
    $tol = if ($key -like '*Recall*') { $recallTol } else { $mrrTol }
    $delta = [math]::Abs($actual[$key] - $wanted[$key])
    if ($delta -gt $tol) {
        $deviations += ("{0}: actual={1:F4} expected={2:F4} delta={3:F4} > {4:F4}" -f `
            $key, $actual[$key], $wanted[$key], $delta, $tol)
    }
}
if ($deviations.Count -gt 0) {
    throw ("指标偏离预期：`n  " + ($deviations -join "`n  ") + "`n先核对数据集 SHA256、分片清单与构建是否非 ML。")
}

Write-Output "PASS: 232 口径复现通过（指标在 expected 容差内）。"
