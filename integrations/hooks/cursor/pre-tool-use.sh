#!/usr/bin/env bash
# LRC preToolUse Hook（Cursor）
#
# 触发时机：模型即将执行 Read / Grep / Shell 之前。
# 职责（重定向）：当这次工具调用的参数明显是在"翻历史"（关键词命中）时，
#                 拦下来并提示先调用 recall，而不是让模型全库扫描。
#
# 两种模式（LRC_HOOK_MODE）：
#   warn（默认）—— 放行，但附一条提示，让模型自己改道。安全，不会打断工作流。
#   deny        —— 直接拒绝本次调用。约束力强，但可能误伤，建议先跑 warn 观察一周。
#
# 端口纪律：本脚本不访问网络，只做参数判定，因此不涉及 3099/3111。
#
# 依赖：jq
set -euo pipefail

MODE="${LRC_HOOK_MODE:-warn}"

STDIN_JSON=""
if [ ! -t 0 ]; then
  STDIN_JSON="$(cat || true)"
fi

if [ -z "$STDIN_JSON" ]; then
  exit 0
fi

# 兼容不同版本的字段命名：tool_name / toolName，tool_input / toolInput
TOOL="$(printf '%s' "$STDIN_JSON" | jq -r '.tool_name // .toolName // empty' 2>/dev/null || true)"
INPUT="$(printf '%s' "$STDIN_JSON" | jq -c '.tool_input // .toolInput // {}' 2>/dev/null || echo '{}')"

# 只关心"读/搜"类工具；写类工具不拦
case "$TOOL" in
  Read|read_file|Grep|grep|Glob|glob|Shell|shell|run_terminal_cmd) ;;
  *) exit 0 ;;
esac

# 判定：参数里出现"翻历史"的意图词
PATTERN='之前|上次|以前|历史|偏好|约定|选型|决定|怎么改|改过|定过'
if ! printf '%s' "$INPUT" | grep -Eq "$PATTERN"; then
  exit 0
fi

MSG='【LRC 重定向】本次调用看起来是在翻项目历史 / 偏好 / 既有决策。请先调用 recall 取全文；recall 未命中再回退到内置搜索，并在回复中说明"LRC 未命中，已回退内置搜索"。'

if [ "$MODE" = "deny" ]; then
  jq -n --arg m "$MSG" '{permission:"deny", user_message:$m, agent_message:$m}'
else
  jq -n --arg m "$MSG" '{permission:"allow", agent_message:$m}'
fi
