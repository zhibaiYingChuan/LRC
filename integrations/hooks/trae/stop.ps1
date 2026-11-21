# LRC Stop Hook（Trae / Windows PowerShell）
#
# 触发时机：每轮任务结束（模型停止输出后）。
# 职责（WAL）：把本轮最终回复交给 /v1/context/absorb，沉淀为低权重摘要，
#              让宿主压缩上下文时不必再"记住"这些结论。
#
# 输出约定：Stop 钩子的 stdout 会被回灌给模型，本脚本**不输出任何内容**。
# 失败策略：任何异常都 exit 0，绝不阻塞或打断对话。
# 端口纪律：默认 3099；开发联调用 $env:LRC_BASE = 'http://127.0.0.1:3111'。
#
# 依赖：PowerShell 5.1+（无需 jq）

$ErrorActionPreference = 'Stop'

$LrcBase    = if ($env:LRC_BASE)    { $env:LRC_BASE }    else { 'http://127.0.0.1:3099' }
$LrcProject = if ($env:LRC_PROJECT) { $env:LRC_PROJECT } else { Split-Path -Leaf (Get-Location).Path }
$LrcToken   = $env:LRC_TOKEN
$DryRun     = ($env:LRC_ABSORB_DRY_RUN -eq '1')

try {
    # ---- 1) 读 stdin ----
    # 与 user-prompt-submit.ps1 相同的通道顺序：$input 优先，[Console]::In 兜底。
    # 原因：`powershell -File` 下 [Console]::In.ReadToEnd() 有时读不到管道内容。
    $Raw = ''
    try { $Raw = ($input | Out-String).Trim() } catch { $Raw = '' }
    if (-not $Raw -and [Console]::IsInputRedirected) {
        try { $Raw = [Console]::In.ReadToEnd() } catch { $Raw = '' }
    }
    if (-not $Raw) { exit 0 }

    # ---- 2) 解析宿主载荷 ----
    # 约定：以 `{` 开头就必须是宿主 JSON。JSON 解析失败、或解析成功但没有
    # 文本字段时**直接退出**——绝不能把整段 JSON 当正文提交，否则会沉淀出
    # `"type":"agent-turn-complete"` 一类垃圾记忆（v0.11.1 审查 P1）。
    $Text = ''
    $SessionId = ''
    if ($Raw -match '^\s*\{') {
        try {
            $Obj = $Raw | ConvertFrom-Json
            # stop_hook_active=true 表示本轮由 Stop 钩子自身触发，必须退出防止死循环。
            if ($Obj.stop_hook_active -eq $true) { exit 0 }
            if ($Obj.last_assistant_message) { $Text = [string]$Obj.last_assistant_message }
            if ($Obj.session_id) { $SessionId = [string]$Obj.session_id }
            if ($Obj.cwd) { $LrcProject = Split-Path -Leaf ([string]$Obj.cwd) }
        } catch {
            # 畸形 JSON：不是可沉淀的文本，静默退出。
            exit 0
        }
        if (-not $Text) { exit 0 }
    } else {
        # 非 JSON：Trae 之外的宿主可能直接传纯文本。
        $Text = $Raw
    }

    if (-not $Text -or $Text.Trim().Length -eq 0) { exit 0 }

    # ---- 3) 提交沉淀 ----
    # importance_cap=4：自动摘要永不与用户显式 remember 抢权重。
    # 不传 source：由服务端按"是否解析到 [LRC] 块"判定 model_declared / auto_distill，
    # 钩子硬编码会把模型声明误标成规则提炼。
    $Body = @{
        text           = $Text
        host           = 'trae'
        project        = $LrcProject
        importance_cap = 4
        max_items      = 10
        dry_run        = $DryRun
    }
    if ($SessionId) { $Body['session_id'] = $SessionId }

    $Headers = @{}
    if ($LrcToken) { $Headers['Authorization'] = "Bearer $LrcToken" }

    # PS 5.1 的 ConvertTo-Json 会把中文转成 \uXXXX（合法 JSON）；
    # 必须按 UTF-8 字节发送，否则服务端按默认编码解会乱码。
    $Json = $Body | ConvertTo-Json -Compress
    $Bytes = [System.Text.Encoding]::UTF8.GetBytes($Json)

    Invoke-WebRequest -Uri "$LrcBase/v1/context/absorb" -Method Post `
        -Headers $Headers -Body $Bytes -ContentType 'application/json; charset=utf-8' `
        -TimeoutSec 4 -UseBasicParsing | Out-Null

    if ($env:LRC_HOOK_LOG) {
        Add-Content -Path $env:LRC_HOOK_LOG -Value ("{0} stop absorb ok session={1}" -f (Get-Date -Format o), $SessionId)
    }
} catch {
    # 服务未启动 / 超时 / 编码异常：静默降级
    if ($env:LRC_HOOK_LOG) {
        try {
            Add-Content -Path $env:LRC_HOOK_LOG -Value ("{0} stop absorb failed: {1}" -f (Get-Date -Format o), $_.Exception.Message)
        } catch { }
    }
}

exit 0
