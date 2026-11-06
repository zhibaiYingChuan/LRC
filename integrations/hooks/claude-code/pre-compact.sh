#!/usr/bin/env bash
# LRC PreCompact Hook（Claude Code）
#
# 触发时机：上下文即将被压缩（自动 80% 或用户手动 /compact）。
# 职责：把本轮对话里值得留存的结论迁移进 LRC，然后输出一行极简索引，
#       替代宿主原生摘要（原生摘要会丢失约束与指代锚点）。
#
# 端口纪律：默认 3099（本地正式）。开发联调请用 LRC_BASE=http://127.0.0.1:3111，
#           不要改本文件。
#
# 依赖：curl、jq
set -euo pipefail

LRC_BASE="${LRC_BASE:-http://127.0.0.1:3099}"
LRC_PROJECT="${LRC_PROJECT:-$(basename "$PWD")}"
MAX_ITEMS="${LRC_MAX_ITEMS:-40}"

# ---- 统一的 POST 封装：LRC_TOKEN 存在时自动带 Bearer ----
lrc_post() {
  local path="$1" body="$2" timeout="${3:-20}"
  if [ -n "${LRC_TOKEN:-}" ]; then
    curl -sS --max-time "$timeout" -X POST "$LRC_BASE$path" \
      -H 'Content-Type: application/json' \
      -H "Authorization: Bearer $LRC_TOKEN" \
      -d "$body"
  else
    curl -sS --max-time "$timeout" -X POST "$LRC_BASE$path" \
      -H 'Content-Type: application/json' \
      -d "$body"
  fi
}

# ---- 1) 取 transcript 路径 ----
# Claude Code 通过 stdin 传 JSON：{session_id, transcript_path, trigger, hook_event_name}
STDIN_JSON=""
if [ ! -t 0 ]; then
  STDIN_JSON="$(cat || true)"
fi

TRANSCRIPT="${CLAUDE_TRANSCRIPT_PATH:-}"
if [ -z "$TRANSCRIPT" ] && [ -n "$STDIN_JSON" ]; then
  TRANSCRIPT="$(printf '%s' "$STDIN_JSON" | jq -r '.transcript_path // empty' 2>/dev/null || true)"
fi

if [ -z "$TRANSCRIPT" ] || [ ! -f "$TRANSCRIPT" ]; then
  echo "[归档] 未拿到 transcript 路径，本次跳过迁移（LRC 未受影响）"
  exit 0
fi

# ---- 2) 优先走提炼接口（P1-1 提供；未实现时返回 404，走降级） ----
DISTILLED=""
DISTILLED="$(lrc_post /v1/context/distill \
  "$(jq -n --arg p "$TRANSCRIPT" --arg proj "$LRC_PROJECT" --argjson n "$MAX_ITEMS" \
      '{transcript_path:$p, project:$proj, max_items:$n}')" 20 2>/dev/null || true)"

if printf '%s' "$DISTILLED" | jq -e '.items' >/dev/null 2>&1; then
  # ---- 2a) 提炼可用：批量迁移（migrate 幂等，重复迁移走合并） ----
  # 只取 items 并补上 project：distill 的响应还带 count/index_line，
  # 直接整包转发虽然也能被忽略，但会丢掉项目归属。
  MIGRATE_BODY="$(printf '%s' "$DISTILLED" \
    | jq -c --arg p "$LRC_PROJECT" '{items: .items, project: $p}')"
  MIGRATED="$(lrc_post /v1/context/migrate "$MIGRATE_BODY" 30 2>/dev/null || true)"
  WRITTEN="$(printf '%s' "$MIGRATED" | jq -r '.written // 0' 2>/dev/null || echo 0)"
  INDEX_LINE="$(printf '%s' "$DISTILLED" | jq -r '.index_line // empty' 2>/dev/null || true)"
  [ -z "$INDEX_LINE" ] && INDEX_LINE="前序对话已提炼 ${WRITTEN} 条至 LRC（项目：${LRC_PROJECT}）"
  echo "[归档] ${INDEX_LINE}"
  exit 0
fi

# ---- 2b) 降级：只迁移不提炼 ----
# 把对话尾部原文作为一条 code_context 记忆写入，tags 含 archived 便于后续清理。
TAIL_TEXT="$(tail -n 200 "$TRANSCRIPT" | head -c 4000)"
BODY="$(jq -n \
  --arg c "[归档·未提炼] 会话尾部原文（项目：${LRC_PROJECT}）
${TAIL_TEXT}" \
  --arg p "$LRC_PROJECT" \
  '{content:$c, memory_type:"code_context", importance:4, project:$p,
    tags:["archived","pre-compact"]}')"

if lrc_post /v1/memories/remember "$BODY" 30 >/dev/null 2>&1; then
  echo "[归档] 前序对话已迁移至 LRC（未提炼模式，项目：${LRC_PROJECT}）；检索用 recall 或 /v1/memories/recent"
else
  echo "[归档] LRC 写入失败（服务未启动或端口不符），本次未迁移"
fi
