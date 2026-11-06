﻿# LRC UserPromptSubmit Hook（Trae / Windows PowerShell）
#
# 触发时机：用户每次提交 prompt 之前。
# 职责（"稀释"）：把 LRC 的强制规则重新推到模型注意力里，避免长会话中
#                 规则被稀释；命中"历史/偏好/决策"类关键词时，额外注入记忆索引。
#
# 输出约定：stdout 会被拼进本轮上下文，因此只输出提示文本，不输出日志。
# 端口纪律：默认 3099；开发联调用 $env:LRC_BASE = 'http://127.0.0.1:3111'。
#
# 依赖：PowerShell 5.1+（无需 jq，用 ConvertFrom-Json）

param(
    # 宿主若支持直接传参，用 -PromptText "..."；否则走 stdin / 环境变量
    [string]$PromptText = ''
)

$ErrorActionPreference = 'Stop'

$LrcBase    = if ($env:LRC_BASE)    { $env:LRC_BASE }    else { 'http://127.0.0.1:3099' }
$LrcProject = if ($env:LRC_PROJECT) { $env:LRC_PROJECT } else { Split-Path -Leaf (Get-Location).Path }
$LrcToken   = $env:LRC_TOKEN
$AlwaysRemind = if ($env:LRC_ALWAYS_REMIND) { $env:LRC_ALWAYS_REMIND } else { '1' }

# ---- 1) 读入本轮 prompt ----
# 三条通道按优先级尝试，因为不同宿主传参方式不同：
#   ① -PromptText 参数；② LRC_PROMPT 环境变量；③ stdin（$input 优先，[Console]::In 兜底）
# 为什么 stdin 排最后：若宿主把 stdin 当管道打开却不关闭，读 stdin 会一直阻塞。
#   实测：`powershell -File` 下 [Console]::In.ReadToEnd() 读不到管道内容（ConsoleInLen=0），
#         而 $input 能读到（InputVarLen=21），故 stdin 通道内 $input 排在前面。
$Raw = $PromptText
if (-not $Raw -and $env:LRC_PROMPT) { $Raw = $env:LRC_PROMPT }
if (-not $Raw) {
    try { $Raw = ($input | Out-String).Trim() } catch { $Raw = '' }
}
if (-not $Raw -and [Console]::IsInputRedirected) {
    try { $Raw = [Console]::In.ReadToEnd() } catch { $Raw = '' }
}

$Prompt = $Raw
if ($Raw -match '^\s*\{') {
    try {
        $Obj = $Raw | ConvertFrom-Json
        if ($Obj.prompt) { $Prompt = [string]$Obj.prompt }
    } catch { }
}

# ---- 2) 关键词判定：是否属于"该先 recall"的提问 ----
$Pattern = '之前|上次|以前|历史|偏好|约定|选型|决定|怎么改|改过|定过|之前定|上次说'
$NeedRecall = $Prompt -match $Pattern

# ---- 3) 命中关键词时取记忆索引 ----
$IndexBlock = ''
if ($NeedRecall) {
    try {
        $Headers = @{}
        if ($LrcToken) { $Headers['Authorization'] = "Bearer $LrcToken" }
        # PS 5.1 的 Invoke-RestMethod 对不带 charset 的 JSON 按 ISO-8859-1 解码，中文会乱码；
        # 改为取原始字节后显式按 UTF-8 解码（实测：直接 Invoke-RestMethod 得到 "å·²å®æ"）。
        $WebResp = Invoke-WebRequest -Uri "$LrcBase/v1/memories/recent?limit=8" `
            -Headers $Headers -TimeoutSec 8 -Method Get -UseBasicParsing
        $JsonText = [System.Text.Encoding]::UTF8.GetString($WebResp.RawContentStream.ToArray())
        $Resp = $JsonText | ConvertFrom-Json
        if ($Resp.memories -and $Resp.memories.Count -gt 0) {
            $Lines = @("【LRC 记忆索引 · 项目 $LrcProject · 最近 $($Resp.memories.Count) 条】")
            $i = 1
            foreach ($m in $Resp.memories) {
                $Lines += "$i. [$($m.memory_type)] $($m.content_preview)"
                $i++
            }
            $IndexBlock = ($Lines -join "`n")
        }
    } catch {
        # 服务未启动 / 端口不符：静默降级，不污染上下文
        $IndexBlock = ''
    }
}

# ---- 4) 输出 ----
if ($IndexBlock) { Write-Output $IndexBlock }

if ($NeedRecall) {
    Write-Output '【LRC 强制】本轮提问涉及项目历史 / 偏好 / 既有决策：必须先调用 recall 取全文，禁止直接 Read/Grep 全库翻找。'
} elseif ($AlwaysRemind -eq '1') {
    Write-Output '【LRC】涉及历史与决策先 recall；完成代码修改后 remember。'
}
