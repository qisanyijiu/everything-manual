#!/usr/bin/env bash
# BUG-001 复验回归（QA 回合 6）：可注入时钟修复后的稳定性回归。
# 运行：bash artifacts/web-mvp/t06-qa/bug001-regression.sh <out-dir>
set -u
OUT="${1:?usage: bug001-regression.sh <out-dir>}"
mkdir -p "$OUT"
cd "$(dirname "$0")/../../.." || exit 1
ROOT="$PWD"
echo "root=$ROOT out=$OUT date=$(date '+%Y-%m-%d %H:%M:%S %Z')"

run() { # run <logfile> <cmd...>
  local log="$1"; shift
  echo "=== $log : $* ==="
  { date '+%Y-%m-%d %H:%M:%S %Z'; "$@"; echo "EXIT=$?"; } >"$OUT/$log" 2>&1
  tail -3 "$OUT/$log" | sed 's/^/    /'
}

for round in 1 2 3 4 5 6; do
  run "ws-round-$round.log" cargo test --workspace
done
for round in 1 2 3 4; do
  run "ws-t16-round-$round.log" cargo test --workspace -- --test-threads=16
done
for round in 1 2 3; do
  run "lib-round-$round.log" cargo test -p everything-manual --lib
done
for round in 1 2 3; do
  run "limiter-t16-round-$round.log" cargo test -p everything-manual --lib limiter -- --test-threads=16
done
echo "ALL DONE $(date '+%Y-%m-%d %H:%M:%S %Z')"
