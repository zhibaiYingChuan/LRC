#!/usr/bin/env bash
# LRC Stop Hook（Cursor）
#
# 触发时机：一轮 Agent 回复结束（Cursor 的 `stop` 事件）。
# 职责（WAL）：把本轮最终回复交给 `lrc absorb`，沉淀为低权重摘要。
#
# 设计：**零 jq 依赖**。Cursor 的 stop 载荷是 JSON，直接原样交给
# `lrc absorb -`；字段兼容（last_assistant_message / text / conversation_id /
# workspace_roots / stop_hook_active）由 CLI 的 host_payload 统一处理。
#
# 输出约定：stdout 必须为空（Cursor 会把 stop 钩子的输出回灌给模型）。
# 失败策略：任何异常都 exit 0，绝不阻塞或打断对话。
# 端口纪律：默认 3099；开发联调用 LRC_BASE=http://127.0.0.1:3111。
#
# 依赖：bash + lrc（PATH 中，或用 LRC_BIN 指定绝对路径）

set -uo pipefail

LRC_BIN="${LRC_BIN:-lrc}"
if ! command -v "$LRC_BIN" >/dev/null 2>&1; then
  # 未安装 CLI：静默降级（与 pre-tool-use.sh 的「没有 jq 就放行」同一纪律）
  exit 0
fi

# 原样透传 stdin：CLI 负责解析 JSON、stop_hook_active 防循环与项目归属。
cat 2>/dev/null | "$LRC_BIN" absorb - --host cursor --no-spawn --timeout 4000 --json >/dev/null 2>&1
CODE=$?

if [ -n "${LRC_HOOK_LOG:-}" ]; then
  printf '%s cursor stop absorb exit=%s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$CODE" \
    >> "$LRC_HOOK_LOG" 2>/dev/null || true
fi

exit 0
