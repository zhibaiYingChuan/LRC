#!/usr/bin/env bash
# PreCompact / SessionStart Hook 的端到端自动化测试。
#
# 模拟 Claude Code 的真实调用方式：把 `{transcript_path, hook_event_name}` 从 stdin 喂进去，
# 断言 ① 脚本输出含 `[归档]`；② mock 服务真的收到了 distill + migrate（或降级时的 remember）。
#
# 依赖：bash、curl、jq、python3
# 用法：bash tests/test-pre-compact.sh
#
# 注意：Windows 上请在 **Git Bash** 里跑（WSL 的 127.0.0.1 与 Windows 不互通）。
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOOK_DIR="$(cd "$HERE/.." && pwd)"
PORT="${TEST_PORT:-3199}"
BASE="http://127.0.0.1:${PORT}"
LOG="/tmp/lrc_mock_requests_$$.jsonl"
TRANSCRIPT="/tmp/lrc_fake_transcript_$$.jsonl"
MOCK_PID=""

PASS=0
FAIL=0

check() {
  local name="$1" cond="$2" detail="${3:-}"
  if [ "$cond" = "1" ]; then
    echo "  PASS  $name"
    PASS=$((PASS + 1))
  else
    echo "  FAIL  $name${detail:+ -> $detail}"
    FAIL=$((FAIL + 1))
  fi
}

cleanup() {
  [ -n "$MOCK_PID" ] && kill "$MOCK_PID" 2>/dev/null
  rm -f "$LOG" "$TRANSCRIPT"
}
trap cleanup EXIT

start_mock() {
  local mode="$1"
  [ -n "$MOCK_PID" ] && kill "$MOCK_PID" 2>/dev/null && sleep 0.3
  rm -f "$LOG"
  MOCK_MODE="$mode" MOCK_LOG="$LOG" python3 "$HERE/mock-lrc.py" "$PORT" &
  MOCK_PID=$!
  for _ in $(seq 1 40); do
    if curl -sS -m 1 "$BASE/v1/memories/recent?limit=1" >/dev/null 2>&1; then return 0; fi
    sleep 0.2
  done
  echo "mock 启动失败（端口 $PORT 被占用？）"
  exit 1
}

# 统计某个接口被调用的次数
count_calls() {
  local path="$1"
  [ -f "$LOG" ] || { echo 0; return; }
  jq -r --arg p "$path" 'select(.path == $p) | .path' "$LOG" 2>/dev/null | wc -l | tr -d ' '
}

# 取某个接口最后一次请求体里的字段
last_body_field() {
  local path="$1" field="$2"
  [ -f "$LOG" ] || { echo ""; return; }
  jq -r --arg p "$path" --arg f "$field" \
    "select(.path == \$p) | .body.${field} // empty" "$LOG" 2>/dev/null | tail -1
}

printf 'user: 我们决定用 pnpm 替代 npm。\nassistant: 好的。\n' > "$TRANSCRIPT"

echo "=== 1) 提炼路径：distill 可用 ==="
start_mock distill
OUT="$(printf '{"hook_event_name":"PreCompact","transcript_path":"%s"}' "$TRANSCRIPT" \
  | LRC_BASE="$BASE" LRC_PROJECT=demo bash "$HOOK_DIR/pre-compact.sh" 2>&1)"
echo "  输出: $OUT"
check "输出含 [归档]" "$(printf '%s' "$OUT" | grep -q '\[归档\]' && echo 1 || echo 0)" "$OUT"
check "调用了 distill" "$([ "$(count_calls /v1/context/distill)" -ge 1 ] && echo 1 || echo 0)"
check "调用了 migrate" "$([ "$(count_calls /v1/context/migrate)" -ge 1 ] && echo 1 || echo 0)"
check "distill 请求带 project=demo" \
  "$([ "$(last_body_field /v1/context/distill project)" = "demo" ] && echo 1 || echo 0)" \
  "$(last_body_field /v1/context/distill project)"
check "migrate 请求带 project=demo" \
  "$([ "$(last_body_field /v1/context/migrate project)" = "demo" ] && echo 1 || echo 0)" \
  "$(last_body_field /v1/context/migrate project)"

echo "=== 2) 降级路径：distill 返回 404 ==="
start_mock fallback
OUT="$(printf '{"hook_event_name":"PreCompact","transcript_path":"%s"}' "$TRANSCRIPT" \
  | LRC_BASE="$BASE" LRC_PROJECT=demo bash "$HOOK_DIR/pre-compact.sh" 2>&1)"
echo "  输出: $OUT"
check "输出含 [归档]" "$(printf '%s' "$OUT" | grep -q '\[归档\]' && echo 1 || echo 0)" "$OUT"
check "降级时调用 remember" "$([ "$(count_calls /v1/memories/remember)" -ge 1 ] && echo 1 || echo 0)"
check "降级时不调用 migrate" "$([ "$(count_calls /v1/context/migrate)" -eq 0 ] && echo 1 || echo 0)"

echo "=== 3) 无 transcript：应跳过且不报错 ==="
OUT="$(printf '{}' | LRC_BASE="$BASE" LRC_PROJECT=demo bash "$HOOK_DIR/pre-compact.sh" 2>&1)"
RC=$?
check "退出码 0" "$([ "$RC" -eq 0 ] && echo 1 || echo 0)" "rc=$RC"
check "提示跳过" "$(printf '%s' "$OUT" | grep -q '跳过' && echo 1 || echo 0)" "$OUT"

echo "=== 4) 服务不可达：应降级不崩溃 ==="
OUT="$(printf '{"transcript_path":"%s"}' "$TRANSCRIPT" \
  | LRC_BASE=http://127.0.0.1:1 LRC_PROJECT=demo bash "$HOOK_DIR/pre-compact.sh" 2>&1)"
RC=$?
check "退出码 0" "$([ "$RC" -eq 0 ] && echo 1 || echo 0)" "rc=$RC"
check "有失败提示" "$(printf '%s' "$OUT" | grep -q '未迁移\|失败' && echo 1 || echo 0)" "$OUT"

echo "=== 5) SessionStart：注入索引 ==="
start_mock distill
OUT="$(LRC_BASE="$BASE" LRC_PROJECT=demo bash "$HOOK_DIR/session-start.sh" 2>&1)"
echo "  输出: $OUT"
check "输出含记忆索引" "$(printf '%s' "$OUT" | grep -q 'LRC 记忆索引' && echo 1 || echo 0)" "$OUT"
check "输出含条目" "$(printf '%s' "$OUT" | grep -q 'RwLock' && echo 1 || echo 0)" "$OUT"

echo
echo "通过 $PASS 项，失败 $FAIL 项"
[ "$FAIL" -eq 0 ] || exit 1
