#!/usr/bin/env bash
# QA 回合 29（T21）· PRD AC-061 元数据 API 性能记录（本地无供应商等待的普通元数据 API
# p95 目标 ≤ 200ms，PRD §5.5 / validation-release §4）。
#
# 用法：bash qa-r29-metadata-api-p95.sh <dist 二进制绝对路径> [端口] [请求数]
# 说明：只在本机 127.0.0.1 起服务与临时 data-dir；无外网、无付费调用；结束清理进程与目录。
set -euo pipefail

BIN="${1:?用法: qa-r29-metadata-api-p95.sh <binary> [port] [requests] [raw-out-dir]}"
PORT="${2:-19180}"
N="${3:-300}"
RAW_OUT="${4:-}"
WORK="$(mktemp -d /tmp/em-r29-perf-XXXXXX)"
DATA="$WORK/data"
PASS_FILE="$WORK/password.txt"
SERVER_LOG="$WORK/serve.log"
COOKIE="$WORK/cookies.txt"
BASE="http://127.0.0.1:$PORT"
PASSWORD="qa-r29-perf-password-7c31"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

printf '%s\n' "$PASSWORD" > "$PASS_FILE"
chmod 600 "$PASS_FILE"

"$BIN" init --data-dir "$DATA" --password-file "$PASS_FILE" > "$WORK/init.log" 2>&1
"$BIN" serve --data-dir "$DATA" --listen "127.0.0.1:$PORT" > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 100); do
  if curl -fsS "$BASE/health/live" > /dev/null 2>&1; then break; fi
  sleep 0.1
done
curl -fsS "$BASE/health/live" > /dev/null

LOGIN="$(curl -fsS -c "$COOKIE" -H 'content-type: application/json' \
  -d "{\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login")"
CSRF="$(printf '%s' "$LOGIN" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["csrfToken"])')"

# 造数：30 个物品（列表页有一次真实分页查询的量级）。
for i in $(seq 1 30); do
  curl -fsS -b "$COOKIE" -H "x-csrf-token: $CSRF" -H 'content-type: application/json' \
    -d "{\"name\":\"性能样品 $i\",\"brand\":\"QA\",\"model\":\"R29-$i\"}" \
    "$BASE/api/v1/items" > /dev/null
done

measure() {
  local label="$1" path="$2" count="$3" out="$4"
  : > "$out"
  # 预热（连接复用与缓存不纳入统计）。
  for _ in $(seq 1 10); do curl -fsS -b "$COOKIE" -o /dev/null "$BASE$path"; done
  for _ in $(seq 1 "$count"); do
    curl -fsS -b "$COOKIE" -o /dev/null -w '%{time_total}\n' "$BASE$path" >> "$out"
  done
  LC_ALL=C sort -n "$out" -o "$out"
  awk -v label="$label" -v n="$count" '
    { v[NR]=$1 }
    END {
      p50 = v[int(n*0.50)] * 1000
      p95 = v[int(n*0.95 - 0.0000001) + 1] * 1000
      max = v[n] * 1000
      printf "%s：样本 %d，p50=%.1fms，p95=%.1fms，max=%.1fms\n", label, n, p50, p95, max
    }' "$out"
}

echo "== QA R29 元数据 API 时延（本机 127.0.0.1，dist 二进制）=="
echo "binary: $BIN"
"$BIN" --version || true
echo "requests: ${N}，端口: ${PORT}，时间: $(date '+%Y-%m-%d %H:%M:%S %Z')"
echo

ITEMS_OUT="$WORK/items.times"
measure "GET /api/v1/items（列表，30 个物品）" "/api/v1/items" "$N" "$ITEMS_OUT"
measure "GET /api/v1/jobs（空列表）" "/api/v1/jobs" "$((N / 3))" "$WORK/jobs.times"
measure "GET /health/ready" "/health/ready" "$((N / 3))" "$WORK/ready.times"

echo
echo "判决：p95 ≤ 200ms 目标（PRD AC-061）"
P95_ITEMS="$(awk 'BEGIN{n='"$N"'; } {v[NR]=$1} END{printf "%.1f", v[int(n*0.95 - 0.0000001)+1]*1000}' "$ITEMS_OUT")"
awk -v p95="$P95_ITEMS" 'BEGIN { if (p95 <= 200) print "  GET /api/v1/items p95=" p95 "ms → 通过"; else print "  GET /api/v1/items p95=" p95 "ms → 不达标" }'
echo "（判定依据：本机 CPU/磁盘；不含供应商等待。）"

if [[ -n "$RAW_OUT" ]]; then
  mkdir -p "$RAW_OUT"
  cp "$ITEMS_OUT" "$RAW_OUT/metadata-items-times.txt"
  cp "$WORK/jobs.times" "$RAW_OUT/metadata-jobs-times.txt"
  cp "$WORK/ready.times" "$RAW_OUT/metadata-ready-times.txt"
  echo "原始样本：$RAW_OUT/metadata-*-times.txt"
fi
