#!/usr/bin/env bash
# T08 浏览器联调运行脚本：真实发布二进制（临时 data-dir + init）+ Vite dev + 真实 Chrome（CDP）。
#
# 用法：bash artifacts/web-mvp/t08-rd/run-browser-walkthrough.sh
# 产物：同目录截图 01..08、browser-walkthrough.log（本脚本输出）、server.log、vite.log
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
BIN="$ROOT/dist/aarch64-apple-darwin/everything-manual"
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
PASSWORD="t08-browser-password"
CDP_PORT=9333
BACKEND_PORT=8080
WORK="$(mktemp -d /tmp/em-t08-browser-XXXXXX)"
SERVER_PID=""
VITE_PID=""
CHROME_PID=""
VITE_CHILD_PID=""

cleanup() {
  set +e
  [ -n "$CHROME_PID" ] && kill "$CHROME_PID" 2>/dev/null
  # npm run dev 会再 fork 一个 vite 子进程：显式按监听端口收尾，避免残留 dev server。
  [ -n "$VITE_CHILD_PID" ] && kill "$VITE_CHILD_PID" 2>/dev/null
  [ -n "$VITE_PID" ] && kill "$VITE_PID" 2>/dev/null
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null
  wait 2>/dev/null
  cp "$WORK/server.log" "$SCRIPT_DIR/server.log" 2>/dev/null
  cp "$WORK/vite.log" "$SCRIPT_DIR/vite.log" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

if [ ! -x "$BIN" ]; then
  echo "缺少发布二进制：$BIN（先运行 cargo xtask dist --target aarch64-apple-darwin）" >&2
  exit 1
fi
if lsof -nP -iTCP:$BACKEND_PORT -sTCP:LISTEN >/dev/null 2>&1; then
  echo "端口 $BACKEND_PORT 已被占用，请先停止占用进程" >&2
  exit 1
fi

echo "== 1/6 初始化临时 data-dir =="
printf '%s' "$PASSWORD" > "$WORK/pw.txt"
chmod 600 "$WORK/pw.txt"
"$BIN" init --data-dir "$WORK/data" --password-file "$WORK/pw.txt"

echo "== 2/6 启动后端（127.0.0.1:${BACKEND_PORT}）=="
"$BIN" serve --data-dir "$WORK/data" --listen 127.0.0.1:$BACKEND_PORT > "$WORK/server.log" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 100); do
  code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$BACKEND_PORT/api/v1/health/live" || true)"
  [ "$code" = "200" ] && break
  sleep 0.1
done
[ "${code:-}" = "200" ] || { echo "后端未就绪" >&2; cat "$WORK/server.log" >&2; exit 1; }
echo "后端 health/live = $code"

echo "== 3/6 启动 Vite dev（127.0.0.1:5173）=="
(cd "$ROOT/apps/web" && npm run dev > "$WORK/vite.log" 2>&1) &
VITE_PID=$!
VITE_CHILD_PID=""
for _ in $(seq 1 200); do
  code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:5173/" || true)"
  [ "$code" = "200" ] && break
  sleep 0.1
done
[ "${code:-}" = "200" ] || { echo "Vite 未就绪" >&2; cat "$WORK/vite.log" >&2; exit 1; }
VITE_CHILD_PID="$(lsof -nP -iTCP:5173 -sTCP:LISTEN -t 2>/dev/null | head -1)"
echo "Vite = $code (pid ${VITE_CHILD_PID:-unknown})"

echo "== 4/6 启动无头 Chrome（CDP :${CDP_PORT}）=="
"$CHROME" --headless=new --disable-gpu --no-first-run --no-default-browser-check \
  --user-data-dir="$WORK/chrome" --remote-debugging-port=$CDP_PORT about:blank \
  > "$WORK/chrome.log" 2>&1 &
CHROME_PID=$!
for _ in $(seq 1 100); do
  curl -s -o /dev/null "http://127.0.0.1:$CDP_PORT/json/version" && break
  sleep 0.1
done

echo "== 5/6 浏览器走查 =="
set +e
CDP_PORT=$CDP_PORT APP_URL=http://127.0.0.1:5173 OUT_DIR="$SCRIPT_DIR" EM_PASSWORD="$PASSWORD" \
  node "$SCRIPT_DIR/browser-walkthrough.mjs"
WALKTHROUGH_EXIT=$?
set -e

echo "== 6/6 结束（退出码 ${WALKTHROUGH_EXIT}）=="
exit $WALKTHROUGH_EXIT
