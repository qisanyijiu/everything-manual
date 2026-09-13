#!/bin/bash
# T05 QA 回合 5：T04 limiter 单测 flake 的加压复现（真实时间依赖 + CPU 竞争）
set -u
cd /Users/qsyj/Code/rust/everything-manual || exit 1
OUT=artifacts/web-mvp/t05-qa
DBG=target/debug/deps/everything_manual-3471840b7a14fb48
REL=target/release/deps/everything_manual-60682445c9cbf8ee
FILTER=http::auth::limiter::tests::window_restarts_after_expiry_and_zero_limits_are_clamped
LOG=$OUT/flake-hunt.log

pids=()
start_burn() { for _ in $(seq 1 12); do yes > /dev/null & pids+=($!); done; }
stop_burn() { kill "${pids[@]}" 2>/dev/null; wait 2>/dev/null; pids=(); }
trap 'stop_burn' EXIT

echo "=== flake hunt start $(date +%Y-%m-%dT%H:%M:%S%z) ===" > "$LOG"

echo "--- phase 1: 单测 x80（debug，12 个 CPU burner 加压）" >> "$LOG"
start_burn
fails=0
for i in $(seq 1 80); do
  if ! "$DBG" --exact "$FILTER" > "$OUT/flake-tmp.log" 2>&1; then
    fails=$((fails+1)); cp "$OUT/flake-tmp.log" "$OUT/flake-single-debug-FAILED-$i.log"
    echo "  iter=$i FAIL" >> "$LOG"
  fi
done
echo "phase1 debug-single: iterations=80 failures=$fails" >> "$LOG"
stop_burn

echo "--- phase 2: 单测 x60（release，12 burners）" >> "$LOG"
start_burn
fails=0
for i in $(seq 1 60); do
  if ! "$REL" --exact "$FILTER" > "$OUT/flake-tmp.log" 2>&1; then
    fails=$((fails+1)); cp "$OUT/flake-tmp.log" "$OUT/flake-single-release-FAILED-$i.log"
    echo "  iter=$i FAIL" >> "$LOG"
  fi
done
echo "phase2 release-single: iterations=60 failures=$fails" >> "$LOG"
stop_burn

echo "--- phase 3: 整个 lib 测试二进制 x10（debug，12 burners）" >> "$LOG"
start_burn
fails=0
for i in $(seq 1 10); do
  if ! "$DBG" > "$OUT/flake-tmp.log" 2>&1; then
    fails=$((fails+1)); cp "$OUT/flake-tmp.log" "$OUT/flake-libsuite-FAILED-$i.log"
    echo "  iter=$i FAIL" >> "$LOG"
  fi
done
echo "phase3 debug-lib-suite: iterations=10 failures=$fails" >> "$LOG"
stop_burn

echo "--- phase 4: cargo test --lib --test-threads=1 x3 / =16 x3（12 burners）" >> "$LOG"
start_burn
for mode in 1 1 1 16 16 16; do
  if cargo test -p everything-manual --lib -- --test-threads=$mode > "$OUT/flake-tmp.log" 2>&1; then
    echo "phase4 test-threads=$mode: ok" >> "$LOG"
  else
    cp "$OUT/flake-tmp.log" "$OUT/flake-threads-$mode-FAILED-$(date +%s).log"
    echo "phase4 test-threads=$mode: FAIL" >> "$LOG"
  fi
done
stop_burn

# phase 5：workspace 全量 x2 在 burners 下（还原 RD 报告的失败场景）
echo "--- phase 5: cargo test --workspace x2（12 burners）" >> "$LOG"
start_burn
for i in 1 2; do
  if cargo test --workspace > "$OUT/flake-tmp.log" 2>&1; then
    echo "phase5 workspace round=$i: ok" >> "$LOG"
  else
    cp "$OUT/flake-tmp.log" "$OUT/flake-ws-loaded-FAILED-$i.log"
    echo "phase5 workspace round=$i: FAIL" >> "$LOG"
  fi
done
stop_burn

rm -f "$OUT/flake-tmp.log"
echo "=== flake hunt done $(date +%Y-%m-%dT%H:%M:%S%z) ===" >> "$LOG"
cat "$LOG"
