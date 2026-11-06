# LRC SessionStart Hook（Trae / Windows PowerShell）
#
# 触发时机：Trae 创建 Session 后、发起第一个对话前。
# 职责：把 LRC 里最近的记忆摘要注入当前会话，让模型开局就知道有哪些历史结论。
#
# 输出约定：stdout 会被 Trae 拼进上下文，因此只输出索引文本，不输出日志。
# 端口纪律：默认 3099（本地正式）；开发联调用 $env:LRC_BASE = 'http://127.0.0.1:3111'。
#
# 依赖：PowerShell 5.1+（无需 jq）

$ErrorActionPreference = 'Stop'

$LrcBase    = if ($env:LRC_BASE)    { $env:LRC_BASE }    else { 'http://127.0.0.1:3099' }
$LrcProject = if ($env:LRC_PROJECT) { $env:LRC_PROJECT } else { Split-Path -Leaf (Get-Location).Path }
$LrcToken   = $env:LRC_TOKEN
$Limit      = if ($env:LRC_INDEX_LIMIT) { $env:LRC_INDEX_LIMIT } else { '8' }

try {
    $Headers = @{}
    if ($LrcToken) { $Headers['Authorization'] = "Bearer $LrcToken" }

    # PS 5.1 的 Invoke-RestMethod 对不带 charset 的 JSON 按 ISO-8859-1 解码，中文会乱码；
    # 改为取原始字节后显式按 UTF-8 解码。
    $WebResp = Invoke-WebRequest -Uri "$LrcBase/v1/memories/recent?limit=$Limit" `
        -Headers $Headers -TimeoutSec 8 -Method Get -UseBasicParsing
    $JsonText = [System.Text.Encoding]::UTF8.GetString($WebResp.RawContentStream.ToArray())
    $Resp = $JsonText | ConvertFrom-Json

    if ($Resp.memories -and $Resp.memories.Count -gt 0) {
        Write-Output "【LRC 记忆索引 · 项目 $LrcProject · 最近 $($Resp.memories.Count) 条】"
        $i = 1
        foreach ($m in $Resp.memories) {
            Write-Output "$i. [$($m.memory_type)] $($m.content_preview)"
            $i++
        }
        Write-Output '—— 以上为索引摘要。涉及项目历史 / 用户偏好 / 既有决策时，先调用 recall 取全文，不要直接 Read/Grep。'
    }
} catch {
    # 服务未启动 / 端口不符：静默降级，不污染上下文
    exit 0
}
