#!/usr/bin/env bash
# QA 回合 21 · T17 的"零真实外网"独立观测。
#
# 用法：qa-r21-lsof-loopback-only.sh <日志前缀> <要运行并观测的命令…>
# 观测：运行期间每 0.2s 对本轮相关进程（everything-manual serve / vite / node-playwright）
#       采样 `lsof -nP -iTCP -a -p <pid>`，统计 ESTABLISHED 与其中的**非回环**数量。
# 灵敏度说明：日志里同时给出 ESTABLISHED 的对端去重列表——若只出现 127.0.0.1/[::1]，
#   说明采样确实能看到连接（本轮 fixture/后端均为回环），而不是"什么都没抓到"。
set -u
PREFIX="${1:?日志前缀必填}"
shift
LOG="${PREFIX}-lsof-loopback-only.log"
SAMPLE="${PREFIX}-lsof-sample.txt"
: > "$LOG"
: > "$SAMPLE"

echo "== T17 零外网观测 date=$(date -u +%Y-%m-%dT%H:%M:%SZ) 命令：$*" >> "$LOG"

"$@" > "${PREFIX}-run.log" 2>&1 &
CMD_PID=$!
echo "命令 pid=${CMD_PID}" >> "$LOG"

samples=0
while kill -0 "$CMD_PID" 2>/dev/null; do
  pids=$(pgrep -f "target/debug/everything-manual serve" ; pgrep -f "vite" ; pgrep -f "playwright" ; pgrep -f "npm-cli.js run test:e2e" 2>/dev/null) || true
  for pid in $pids; do
    lsof -nP -iTCP -a -p "$pid" 2>/dev/null | sed "s/^/SAMPLE ${pid} /" >> "$SAMPLE"
  done
  samples=$((samples + 1))
  sleep 0.2
done
wait "$CMD_PID"
exit_code=$?
echo "命令 exit=$exit_code samples=$samples" >> "$LOG"

total=$(grep -c '^SAMPLE ' "$SAMPLE" || true)
listen=$(grep '^SAMPLE ' "$SAMPLE" | grep -c '(LISTEN)' || true)
established=$(grep '^SAMPLE ' "$SAMPLE" | grep -c 'ESTABLISHED' || true)
non_loopback=$(grep '^SAMPLE ' "$SAMPLE" | grep 'ESTABLISHED' | grep -vc '127\.0\.0\.1\|\[::1\]' || true)

{
  echo "采样行总数=${total}（LISTEN=${listen} / ESTABLISHED=${established} / ESTABLISHED 非回环=${non_loopback}）"
  echo "-- ESTABLISHED 对端（去重，最多 20 条）"
  grep '^SAMPLE ' "$SAMPLE" | grep 'ESTABLISHED' | sed 's/.*-> //' | sort | uniq -c | sort -rn | head -20
  echo "-- 非回环 ESTABLISHED 明细（应为空）"
  grep '^SAMPLE ' "$SAMPLE" | grep 'ESTABLISHED' | grep -v '127\.0\.0\.1\|\[::1\]' | head -20
  echo "-- 命令结果摘要"
  tail -3 "${PREFIX}-run.log" 2>/dev/null
} >> "$LOG"

rm -f "$SAMPLE"
echo "非回环 ESTABLISHED=${non_loopback}（明细见 ${LOG}）"
[ "$exit_code" = "0" ] && [ "$non_loopback" = "0" ] && exit 0 || exit 1
