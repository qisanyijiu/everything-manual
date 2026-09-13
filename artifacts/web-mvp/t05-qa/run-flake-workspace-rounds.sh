#!/bin/bash
# T05 QA 回合 5：复现 T04 limiter 单测 flake —— workspace 多轮复跑
# 每轮记录：开始/结束时间、退出码、test result 汇总行；失败轮保留完整输出。
set -u
cd /Users/qsyj/Code/rust/everything-manual || exit 1
OUT=artifacts/web-mvp/t05-qa
ROUNDS=${1:-8}
for i in $(seq 1 "$ROUNDS"); do
  log="$OUT/flake-ws-round-$i.log"
  start=$(date +%H:%M:%S)
  cargo test --workspace > "$log" 2>&1
  code=$?
  end=$(date +%H:%M:%S)
  fails=$(grep -c "test result: FAILED" "$log")
  summary=$(grep -E "test result:" "$log" | grep -v "0 passed; 0 failed; 0 ignored; 0 measured" | tr '\n' '|')
  echo "round=$i start=$start end=$end exit=$code failed_suites=$fails summaries=$summary" >> "$OUT/flake-workspace-rounds.log"
  if [ "$code" -ne 0 ] || [ "$fails" -gt 0 ]; then
    cp "$log" "$OUT/flake-ws-round-$i-FAILED.log"
    echo "round=$i FAILED — 完整输出保留在 flake-ws-round-$i-FAILED.log" >> "$OUT/flake-workspace-rounds.log"
  fi
done
echo "done" >> "$OUT/flake-workspace-rounds.log"
