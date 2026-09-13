#!/usr/bin/env bash
# QA 回合 17 · T15 的"零真实外网"独立观测（带正对照）。
#
# 用法：qa-lsof-loopback-only.sh <qa_t15 测试二进制绝对路径> <日志前缀> [QA_T15_SLOW_MS]
#   - 第 3 参数（可选）= 正对照延迟：让 fixture 每个响应前 sleep N 毫秒，
#     把连接窗口拉长到 100ms 采样能看见 ESTABLISHED；用它证明采样有灵敏度。
# 观测：对测试进程 `lsof -nP -iTCP -a -p <pid>` 逐次采样，
#       统计 LISTEN / ESTABLISHED / ESTABLISHED-非回环。
set -u
BIN="${1:?测试二进制路径必填}"
PREFIX="${2:?日志前缀必填}"
SLOW_MS="${3:-0}"

LOG="${PREFIX}-lsof-loopback-only.log"
SAMPLE="/tmp/qa-t15-lsof-sample-$$.txt"
: > "$LOG"
: > "$SAMPLE"

echo "== T15 零外网观测：binary=$BIN slow_ms=$SLOW_MS date=$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$LOG"

if [ "$SLOW_MS" != "0" ]; then
  QA_T15_SLOW_MS="$SLOW_MS" "$BIN" --test-threads=1 > "${PREFIX}-slowrun.log" 2>&1 &
else
  "$BIN" --test-threads=1 > "${PREFIX}-run.log" 2>&1 &
fi
PID=$!
echo "test pid=${PID}" >> "$LOG"

samples=0
while kill -0 "$PID" 2>/dev/null; do
  # 汇总行自身含 "TCP "，先打前缀再统计，避免污染。
  lsof -nP -iTCP -a -p "$PID" 2>/dev/null | sed 's/^/SAMPLE /' >> "$SAMPLE"
  samples=$((samples + 1))
  sleep 0.1
done
wait "$PID"
exit_code=$?
echo "test exit=$exit_code samples=$samples" >> "$LOG"

lines=$(grep -c '^SAMPLE ' "$SAMPLE" || true)
listen=$(grep '^SAMPLE ' "$SAMPLE" | grep -c '(LISTEN)' || true)
established=$(grep '^SAMPLE ' "$SAMPLE" | grep -c 'ESTABLISHED' || true)
# 非回环 = 行中不出现 127.0.0.1 / [::1]
non_loopback=$(grep '^SAMPLE ' "$SAMPLE" | grep 'ESTABLISHED' | grep -vc '127\.0\.0\.1\|\[::1\]' || true)

{
  echo "采样行总数=${lines}（LISTEN=${listen} / ESTABLISHED=${established} / ESTABLISHED 非回环=${non_loopback}）"
  echo "-- ESTABLISHED 对端（去重，最多 20 条）"
  grep '^SAMPLE ' "$SAMPLE" | grep 'ESTABLISHED' | sed 's/.*-> //' | sort | uniq -c | sort -rn | head -20
  echo "-- LISTEN（去重，最多 20 条）"
  grep '^SAMPLE ' "$SAMPLE" | grep '(LISTEN)' | awk '{print $(NF-1)}' | sort | uniq -c | sort -rn | head -20
  echo "-- 测试结果摘要"
  tail -5 "${PREFIX}-run.log" 2>/dev/null || tail -5 "${PREFIX}-slowrun.log" 2>/dev/null
} >> "$LOG"

rm -f "$SAMPLE"
echo "非回环 ESTABLISHED=${non_loopback}（明细见 ${LOG}）"
[ "$exit_code" = "0" ] && [ "$non_loopback" = "0" ] && exit 0 || exit 1
