#!/usr/bin/env bash
# QA 回合 16 · T14：独立观测"零真实外网"（带正对照）。
#
# 做法：直接运行 QA 自建测试二进制（`qa_t14_independent`），在运行期间按 200ms 采样
# `lsof -nP -iTCP -a -p <pid>`，把每条 TCP 连接写入原始采样文件（每行以 `SAMPLE:` 前缀
# 标记，避免把汇总行本身计入）。随后汇总：
#   - 采样到的 ESTABLISHED 条数（**正对照**：其中一条用例把响应延迟 1.5s，
#     使连接窗口可被采样看见，若为 0 说明采样方法本身没看到东西，观察不成立）；
#   - 非回环对端条数（必须为 0：DNS/外网连接会在 lsof 中显示为远端地址）。
#
# 用法：bash artifacts/web-mvp/t14-qa/qa-lsof-loopback-only.sh <test-binary-absolute-path> <log-path>
set -u

BINARY="${1:?需要测试二进制路径}"
LOG="${2:?需要输出日志路径}"

: > "$LOG"
{
  echo "== QA lsof loopback-only observation =="
  echo "binary: $BINARY"
  echo "time: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee -a "$LOG"

"$BINARY" --test-threads=4 > /tmp/qa-t14-lsof-test.out 2>&1 &
TEST_PID=$!
echo "test pid: $TEST_PID" | tee -a "$LOG"

SAMPLES=0
while kill -0 "$TEST_PID" 2>/dev/null; do
  lsof -nP -iTCP -a -p "$TEST_PID" 2>/dev/null | tail -n +2 | while read -r line; do
    echo "SAMPLE: $(date -u +%H:%M:%S) $line"
  done >> "$LOG"
  SAMPLES=$((SAMPLES + 1))
  sleep 0.2
done

wait "$TEST_PID"
CODE=$?
{
  echo "test exit: $CODE"
  echo "samples: $SAMPLES"
  echo "-- test output tail --"
  tail -5 /tmp/qa-t14-lsof-test.out
  echo "-- 汇总 --"
  TOTAL=$(grep -c "^SAMPLE:" "$LOG" || true)
  LISTEN=$(grep "^SAMPLE:" "$LOG" | grep -c "(LISTEN)" || true)
  ESTABLISHED=$(grep "^SAMPLE:" "$LOG" | grep -c "ESTABLISHED" || true)
  NON_LOOPBACK=$(grep "^SAMPLE:" "$LOG" | grep -v "(LISTEN)" | grep -v "127\.0\.0\.1" | grep -v "\[::1\]" | wc -l | tr -d ' ')
  echo "TCP 连接采样行数: ${TOTAL} （LISTEN ${LISTEN} / ESTABLISHED ${ESTABLISHED}）"
  echo "非回环对端条数: $NON_LOOPBACK"
} | tee -a "$LOG"
rm -f /tmp/qa-t14-lsof-test.out
exit "$CODE"
