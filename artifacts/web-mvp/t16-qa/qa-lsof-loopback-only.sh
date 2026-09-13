#!/usr/bin/env bash
# QA 回合 18（T16）零外网观测：只统计本 e2e 运行自己的进程族（playwright/vite/serve/chrome-headless-shell），
# 采样它们的全部 socket，输出任何非回环的 ESTABLISHED/SYN_SENT 行。输出为空 = 该运行无外部活动连接。
set -u
LOG="${1:-/tmp/qa-t16-lsof.log}"
: > "$LOG"
for i in $(seq 1 "${2:-12}"); do
  pids=$(pgrep -f "everything-manual|apps/web/node_modules/.bin/(vite|playwright)|chrome-headless-shell" 2>/dev/null | tr '\n' ',' | sed 's/,$//')
  if [ -n "${pids:-}" ]; then
    lsof -n -P -i -a -p "$pids" 2>/dev/null | grep -E "ESTABLISHED|SYN_SENT" | grep -vE "127\.0\.0\.1|\[::1\]" >> "$LOG"
  fi
  sleep 1
done
echo "# sample rounds=${2:-12}; non-loopback active connections: $(grep -c . "$LOG" || true)" >> "$LOG"
