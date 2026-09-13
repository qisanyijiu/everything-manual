#!/bin/bash
# T05 QA 回合 5：flake 加压复现第二批（更高过订阅：16 burners + 4 并发 lib 套件）
set -u
cd /Users/qsyj/Code/rust/everything-manual || exit 1
OUT=artifacts/web-mvp/t05-qa
DBG=target/debug/deps/everything_manual-3471840b7a14fb48
FILTER=http::auth::limiter::tests::window_restarts_after_expiry_and_zero_limits_are_clamped
LOG=$OUT/flake-hunt2.log

pids=()
start_burn() { for _ in $(seq 1 16); do yes > /dev/null & pids+=($!); done; }
stop_burn() { kill "${pids[@]}" 2>/dev/null; wait 2>/dev/null; pids=(); }
trap 'stop_burn' EXIT

echo "=== flake hunt 2 start $(date +%Y-%m-%dT%H:%M:%S%z) loadavg=$(uptime | awk -F'load averages: ' '{print $2}') ===" > "$LOG"

echo "--- phase A: cargo test --workspace x12（16 burners）" >> "$LOG"
start_burn
fails=0
for i in $(seq 1 12); do
  if cargo test --workspace > "$OUT/hunt2-tmp.log" 2>&1; then
    echo "ws-round=$i ok" >> "$LOG"
  else
    fails=$((fails+1))
    cp "$OUT/hunt2-tmp.log" "$OUT/hunt2-ws-FAILED-$i.log"
    echo "ws-round=$i FAIL（完整输出 hunt2-ws-FAILED-$i.log）" >> "$LOG"
  fi
done
echo "phaseA workspace: rounds=12 failures=$fails" >> "$LOG"
stop_burn

echo "--- phase B: 4 并发 lib 套件 x25 轮（16 burners）" >> "$LOG"
start_burn
fails=0
for round in $(seq 1 25); do
  bpids=()
  for j in 1 2 3 4; do
    "$DBG" > "$OUT/hunt2-conc-$j.log" 2>&1 &
    bpids+=($!)
  done
  for j in 0 1 2 3; do
    if ! wait "${bpids[$j]}"; then
      fails=$((fails+1))
      cp "$OUT/hunt2-conc-$((j+1)).log" "$OUT/hunt2-conc-FAILED-round$round-j$((j+1)).log"
      echo "round=$round j$((j+1)) FAIL" >> "$LOG"
    fi
  done
done
echo "phaseB concurrent lib suites: rounds=25 x4 failures=$fails" >> "$LOG"
stop_burn

echo "--- phase C: 单测 x100（16 burners）" >> "$LOG"
start_burn
fails=0
for i in $(seq 1 100); do
  if ! "$DBG" --exact "$FILTER" > "$OUT/hunt2-tmp.log" 2>&1; then
    fails=$((fails+1)); cp "$OUT/hunt2-tmp.log" "$OUT/hunt2-single-FAILED-$i.log"
    echo "  iter=$i FAIL" >> "$LOG"
  fi
done
echo "phaseC debug-single: iterations=100 failures=$fails" >> "$LOG"
stop_burn

rm -f "$OUT/hunt2-tmp.log" "$OUT/hunt2-conc-1.log" "$OUT/hunt2-conc-2.log" "$OUT/hunt2-conc-3.log" "$OUT/hunt2-conc-4.log"
echo "=== flake hunt 2 done $(date +%Y-%m-%dT%H:%M:%S%z) ===" >> "$LOG"
cat "$LOG"
