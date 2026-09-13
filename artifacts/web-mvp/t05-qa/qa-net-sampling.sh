#!/bin/bash
# T05 QA 回合 5：fixture_harness 运行期间的外呼采样（lsof）
# 目的：补充证据——测试进程运行期间只出现回环连接，无任何非回环/外网连接。
set -u
cd /Users/qsyj/Code/rust/everything-manual || exit 1
OUT=artifacts/web-mvp/t05-qa
BIN=$(ls -t target/debug/deps/fixture_harness-* 2>/dev/null | grep -v '\.d$' | head -1)
[ -n "$BIN" ] || { echo "未找到 fixture_harness 测试二进制"; exit 1; }
echo "binary=$BIN"

LOOP=0
NONLOOP=0
SAMPLES=0
for i in $(seq 1 20); do
  "$BIN" --test-threads=2 > /dev/null 2>&1 &
  TPID=$!
  while kill -0 "$TPID" 2>/dev/null; do
    out=$(lsof -p "$TPID" -a -i -P -n 2>/dev/null | tail -n +2)
    SAMPLES=$((SAMPLES+1))
    if [ -n "$out" ]; then
      echo "--- sample (iter=$i) ---" >> "$OUT/net-sampling-raw.txt"
      echo "$out" >> "$OUT/net-sampling-raw.txt"
      n=$(echo "$out" | grep -c "127.0.0.1\|\[::1\]" || true)
      LOOP=$((LOOP+n))
      m=$(echo "$out" | grep -vc "127.0.0.1\|\[::1\]" || true)
      NONLOOP=$((NONLOOP+m))
    fi
    sleep 0.05
  done
  wait "$TPID" 2>/dev/null
done
echo "samples=$SAMPLES loopback_lines=$LOOP nonloopback_lines=$NONLOOP" | tee "$OUT/net-sampling-summary.txt"
