#!/usr/bin/env bash
# T08 QA 独立浏览器走查运行脚本（QA 自写；不引用 RD 脚本/截图）。
# 真实发布二进制（临时 data-dir + init，内嵌 UI，SPA fallback）+ 真实 Chrome（CDP）。
# 用法：bash artifacts/web-mvp/t08-qa/run-walkthrough-qa.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
BIN="$ROOT/dist/aarch64-apple-darwin/everything-manual"
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
PASSWORD="t08-qa-password-3f7a"
CDP_PORT=9444
APP_PORT=8099
WORK="$(mktemp -d /tmp/em-t08-qa-XXXXXX)"
SERVER_PID=""
CHROME_PID=""

cleanup() {
  set +e
  [ -n "$CHROME_PID" ] && kill "$CHROME_PID" 2>/dev/null
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null
  wait 2>/dev/null
  cp "$WORK/server.log" "$SCRIPT_DIR/server-qa.log" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

if [ ! -x "$BIN" ]; then
  echo "缺少发布二进制：$BIN" >&2
  exit 1
fi
for port in "$APP_PORT" "$CDP_PORT"; do
  if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "端口 $port 已被占用" >&2
    exit 1
  fi
done

echo "== 1/4 初始化临时 data-dir =="
printf '%s' "$PASSWORD" > "$WORK/pw.txt"
chmod 600 "$WORK/pw.txt"
"$BIN" init --data-dir "$WORK/data" --password-file "$WORK/pw.txt"

echo "== 2/4 启动服务（内嵌 UI，127.0.0.1:${APP_PORT}）=="
"$BIN" serve --data-dir "$WORK/data" --listen 127.0.0.1:$APP_PORT > "$WORK/server.log" 2>&1 &
SERVER_PID=$!
code=""
for _ in $(seq 1 150); do
  code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$APP_PORT/api/v1/health/live" || true)"
  [ "$code" = "200" ] && break
  sleep 0.1
done
[ "${code:-}" = "200" ] || { echo "服务未就绪" >&2; cat "$WORK/server.log" >&2; exit 1; }
echo "health/live = $code"

echo "== 3/4 启动无头 Chrome（CDP :${CDP_PORT}）=="
"$CHROME" --headless=new --disable-gpu --no-first-run --no-default-browser-check \
  --user-data-dir="$WORK/chrome" --remote-debugging-port=$CDP_PORT about:blank \
  > "$WORK/chrome.log" 2>&1 &
CHROME_PID=$!
for _ in $(seq 1 100); do
  curl -s -o /dev/null "http://127.0.0.1:$CDP_PORT/json/version" && break
  sleep 0.1
done

echo "== 4/4 浏览器走查 =="
set +e
CDP_PORT=$CDP_PORT APP_URL="http://127.0.0.1:$APP_PORT" OUT_DIR="$SCRIPT_DIR" EM_PASSWORD="$PASSWORD" \
  node "$SCRIPT_DIR/walkthrough-qa.mjs"
WALK_EXIT=$?
set -e
echo "== 走查结束（退出码 ${WALK_EXIT}）=="
exit ${WALK_EXIT}
