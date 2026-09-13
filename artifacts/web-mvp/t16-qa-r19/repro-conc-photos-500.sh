#!/usr/bin/env bash
# QA 回合 19：并发 POST /photos 500 "database is locked" 复现（确定性路径 A：外部写者持锁）。
#
# 机制：photos.rs 用 deferred 事务（SELECT 占用 → INSERT），WAL 下 deferred 的读→写升级遇活跃写者
# 会立即 SQLITE_BUSY（busy_timeout=5s 不等待）——与 T14 已修的 begin_intent 同类。
# 本脚本：临时 data-dir + 真实二进制；python3 持 BEGIN IMMEDIATE 写锁期间发 POST /photos。
# 用法：bash repro-conc-photos-500.sh <输出日志>
set -uo pipefail

OUT="${1:?用法: repro-conc-photos-500.sh <输出日志>}"
REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
BIN="$REPO_ROOT/target/debug/everything-manual"
WORK="$(mktemp -d /tmp/em-r19-conc-A.XXXXXX)"
PORT=18091
BASE="http://127.0.0.1:$PORT"
PASSWORD="qa-r19-concurrency-password"

log() { echo "[$(date +%H:%M:%S)] $*"; }

cleanup() {
  if [ -n "${SERVER_PID:-}" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null
    wait "$SERVER_PID" 2>/dev/null
  fi
  if [ -n "${HOLD_PID:-}" ] && kill -0 "$HOLD_PID" 2>/dev/null; then kill "$HOLD_PID" 2>/dev/null; fi
  log "清理临时目录 $WORK"
  rm -rf "$WORK"
}
trap cleanup EXIT

{
  log "== 准备：临时 data-dir ${WORK}（真实二进制 ${BIN}）=="
  mkdir -p "$WORK/data"
  printf '%s\n' "$PASSWORD" > "$WORK/password.txt"
  chmod 600 "$WORK/password.txt"
  "$BIN" init --data-dir "$WORK/data" --password-file "$WORK/password.txt" >/dev/null 2>&1 || { log "init 失败"; exit 9; }

  cp "$REPO_ROOT/price-catalog.example.toml" "$WORK/price-catalog.toml"
  cat > "$WORK/config.toml" <<EOF
price_catalog_path = "$WORK/price-catalog.toml"

[providers.tripo]
base_url = "http://127.0.0.1:1"
model = "v3.1-20260211"
api_key_env = "EM_E2E_TRIPO_KEY"

[providers.manual_ai]
base_url = "http://127.0.0.1:1"
model = "gpt-5-mini"
api_key_env = "EM_E2E_MANUAL_AI_KEY"
EOF
  EM_E2E_TRIPO_KEY=fake EM_E2E_MANUAL_AI_KEY=fake \
    "$BIN" serve --data-dir "$WORK/data" --config "$WORK/config.toml" --listen "127.0.0.1:$PORT" \
    > "$WORK/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 60); do
    curl -fsS -o /dev/null "$BASE/api/v1/health/ready" 2>/dev/null && break
    sleep 0.25
  done
  log "服务端就绪 pid=${SERVER_PID}；DB=$WORK/data/manual.sqlite3"

  CSRF=$(curl -fsS -c "$WORK/cookies.txt" -X POST "$BASE/api/v1/auth/login" \
    -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["csrfToken"])')
  ITEM=$(curl -fsS -b "$WORK/cookies.txt" -X POST "$BASE/api/v1/items" \
    -H "x-csrf-token: $CSRF" -H 'content-type: application/json' \
    -d '{"name":"QA 并发复现","model":"QA-R19-CONC"}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])')
  ASSET=$(curl -fsS -b "$WORK/cookies.txt" -X POST "$BASE/api/v1/items/$ITEM/assets" \
    -H "x-csrf-token: $CSRF" -F purpose=photo \
    -F "file=@$REPO_ROOT/tests/fixtures/assets/sample-photo-front.jpg;type=image/jpeg" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])')
  log "造数完成 item=$ITEM asset=$ASSET"

  post_photo() { # $1=view
    curl -sS -b "$WORK/cookies.txt" -o "$WORK/resp-$1.json" -w '%{http_code}' \
      -X POST "$BASE/api/v1/items/$ITEM/photos" -H "x-csrf-token: $CSRF" \
      -H 'content-type: application/json' -d "{\"assetId\":\"$ASSET\",\"view\":\"$1\"}"
  }

  log "== 对照组（无并发写者）：POST /photos view=front =="
  CODE=$(post_photo front)
  log "   → HTTP $CODE $(head -c 300 "$WORK/resp-front.json" 2>/dev/null)"

  log "== 实验组：python3 持 BEGIN IMMEDIATE 写锁 4s，锁内发 POST /photos view=left =="
  python3 - "$WORK/data/manual.sqlite3" 4 > "$WORK/lock-holder.log" 2>&1 <<'PY' &
import sqlite3, sys, time
db, hold = sys.argv[1], float(sys.argv[2])
con = sqlite3.connect(db, timeout=0.1)
con.execute("BEGIN IMMEDIATE")
print("writer: BEGIN IMMEDIATE acquired", flush=True)
time.sleep(hold)
con.rollback()
print("writer: released", flush=True)
PY
  HOLD_PID=$!
  sleep 0.7   # 等锁确实拿到
  CODE=$(post_photo left)
  log "   锁内 POST /photos → HTTP $CODE"
  log "   响应体：$(cat "$WORK/resp-left.json" 2>/dev/null | head -c 400)"
  log "   SQLite 侧验证：锁内第二个写者应失败——$(python3 - "$WORK/data/manual.sqlite3" <<'PY'
import sqlite3, sys
try:
    con = sqlite3.connect(sys.argv[1], timeout=0.1)
    con.execute("BEGIN IMMEDIATE")
    print("意外：拿到了写锁")
except sqlite3.OperationalError as e:
    print(f"BEGIN IMMEDIATE 被拒：{e}")
PY
)"
  kill "$HOLD_PID" 2>/dev/null; wait "$HOLD_PID" 2>/dev/null; HOLD_PID=""

  log "== 锁释放后对照（补偿）：POST /photos view=left =="
  CODE=$(post_photo left)
  log "   → HTTP $CODE $(head -c 300 "$WORK/resp-left.json" 2>/dev/null)"

  log "== 服务端日志中的并发错误行 =="
  grep -n "database is locked\|存储错误\|服务器内部错误" "$WORK/server.log" | tail -10
  echo "---- server.log 全文（截断 200 行）----"
  tail -200 "$WORK/server.log"
} 2>&1 | tee "$OUT"
