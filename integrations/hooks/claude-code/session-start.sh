#!/usr/bin/env bash
# LRC SessionStart Hook（Claude Code）
#
# 触发时机：会话启动 / 恢复。
# 职责：把 LRC 里最近的记忆摘要注入当前会话，让模型开局就知道"有哪些历史结论"，
#       避免它一上来就用 Read/Grep 全库翻找。
#
# 输出约定：stdout 会被宿主拼进上下文，因此只输出索引文本，不输出日志。
# 端口纪律：默认 3099；开发联调用 LRC_BASE=http://127.0.0.1:3111。
#
# 依赖：curl、jq
set -euo pipefail

LRC_BASE="${LRC_BASE:-http://127.0.0.1:3099}"
LRC_PROJECT="${LRC_PROJECT:-$(basename "$PWD")}"
LIMIT="${LRC_INDEX_LIMIT:-8}"

lrc_get() {
  local path="$1" timeout="${2:-8}"
  if [ -n "${LRC_TOKEN:-}" ]; then
    curl -sS --max-time "$timeout" "$LRC_BASE$path" \
      -H "Authorization: Bearer $LRC_TOKEN"
  else
    curl -sS --max-time "$timeout" "$LRC_BASE$path"
  fi
}

RECENT="$(lrc_get "/v1/memories/recent?limit=${LIMIT}" 8 2>/dev/null || true)"

if ! printf '%s' "$RECENT" | jq -e '.memories' >/dev/null 2>&1; then
  # 服务未启动 / 端口不符 / 无数据：静默降级，不污染上下文
  exit 0
fi

COUNT="$(printf '%s' "$RECENT" | jq -r '.memories | length' 2>/dev/null || echo 0)"
if [ "$COUNT" = "0" ]; then
  exit 0
fi

echo "【LRC 记忆索引 · 项目 ${LRC_PROJECT} · 最近 ${COUNT} 条】"
printf '%s' "$RECENT" | jq -r '
  .memories | to_entries[] |
  "\(.key + 1). [\(.value.memory_type)] \(.value.content_preview)"
'
echo "—— 以上为索引摘要。涉及项目历史 / 用户偏好 / 既有决策时，先调用 recall 取全文，不要直接 Read/Grep。"
