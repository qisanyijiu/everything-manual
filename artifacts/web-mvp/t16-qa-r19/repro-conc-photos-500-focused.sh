#!/usr/bin/env bash
# QA 回合 19：并发 POST /photos 500 聚焦量化（场景 S1 顺序单发 / S2 同视图 4 并发 / S3 异视图 4 并发）。
# 无外部锁：并发来自服务端自身客户端；另注：执行器 idle_poll=250ms 且每次 claim 用 BEGIN IMMEDIATE
# （持写锁），即使空闲也在周期性取写锁 → 与 photos.rs:90 的 deferred 读→写升级直接竞争。
# 服务端日志在本脚本结束时复制到 artifacts（临时目录随后删除）。
# 用法：bash repro-conc-photos-500-focused.sh <输出日志> <server日志副本路径>
set -uo pipefail

OUT="${1:?用法: repro-conc-photos-500-focused.sh <输出日志> <server日志副本>}"
SERVER_LOG_OUT="${2:?需要 server 日志副本路径}"
REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
BIN="$REPO_ROOT/target/debug/everything-manual"
WORK="$(mktemp -d /tmp/em-r19-conc-F.XXXXXX)"
PORT=18093
BASE="http://127.0.0.1:$PORT"
PASSWORD="qa-r19-concurrency-password-f"

S1_ROUNDS=20
S2_ROUNDS=10
S3_ROUNDS=10

log() { echo "[$(date +%H:%M:%S)] $*"; }

cleanup() {
  pkill -f "listen 127.0.0.1:${PORT}" 2>/dev/null
  if [ -f "$WORK/server.log" ]; then cp "$WORK/server.log" "$SERVER_LOG_OUT"; fi
  log "清理临时目录 ${WORK}"
  rm -rf "$WORK"
}
trap cleanup EXIT

{
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
  for _ in $(seq 1 60); do
    curl -fsS -o /dev/null "$BASE/api/v1/health/ready" 2>/dev/null && break
    sleep 0.25
  done
  log "服务端就绪（端口 ${PORT}，data-dir ${WORK}）"

  CSRF=$(curl -fsS -c "$WORK/cookies.txt" -X POST "$BASE/api/v1/auth/login" \
    -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["csrfToken"])')

  new_item_asset() {
    ITEM=$(curl -fsS -b "$WORK/cookies.txt" -X POST "$BASE/api/v1/items" \
      -H "x-csrf-token: $CSRF" -H 'content-type: application/json' \
      -d "{\"name\":\"QA 聚焦 $1\",\"model\":\"QA-R19-FOCUS\"}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])')
    ASSET=$(curl -fsS -b "$WORK/cookies.txt" -X POST "$BASE/api/v1/items/$ITEM/assets" \
      -H "x-csrf-token: $CSRF" -F purpose=photo \
      -F "file=@$REPO_ROOT/tests/fixtures/assets/sample-photo-front.jpg;type=image/jpeg" \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])')
  }
  photo_post() { # $1=view $2=输出文件
    curl -sS -b "$WORK/cookies.txt" -o "$2" -w '%{http_code}' \
      -X POST "$BASE/api/v1/items/$ITEM/photos" -H "x-csrf-token: $CSRF" \
      -H 'content-type: application/json' -d "{\"assetId\":\"$ASSET\",\"view\":\"$1\"}"
  }

  echo "### S1 顺序单发（无用户并发；模拟普通用户单张上传，执行器 250ms tick 仍会取写锁）"
  s1_ok=0; s1_500=0; s1_other=0
  for r in $(seq 1 "$S1_ROUNDS"); do
    new_item_asset "S1-${r}"
    code=$(photo_post front "$WORK/s1-$r.json")
    case "$code" in
      201) s1_ok=$((s1_ok+1));;
      500) s1_500=$((s1_500+1)); echo "  S1 round $r → 500: $(head -c 200 "$WORK/s1-$r.json")";;
      *) s1_other=$((s1_other+1)); echo "  S1 round $r → $code";;
    esac
  done
  echo "S1 结果：201×$s1_ok  500×$s1_500  其它×$s1_other  （共 ${S1_ROUNDS}）"

  echo "### S2 同视图 4 并发（同一 item 同一 view=front；理想 1×201 + 3×422）"
  s2_ok=0; s2_422=0; s2_500=0; s2_other=0
  for r in $(seq 1 "$S2_ROUNDS"); do
    new_item_asset "S2-${r}"
    pids=(); outs=()
    for i in 1 2 3 4; do
      photo_post front "$WORK/s2-$r-$i.json" > "$WORK/s2-$r-$i.code" &
      pids+=($!)
    done
    for pid in "${pids[@]}"; do wait "$pid"; done
    codes=""; for i in 1 2 3 4; do c=$(cat "$WORK/s2-$r-$i.code"); codes="$codes $c"; case "$c" in 201) s2_ok=$((s2_ok+1));; 422) s2_422=$((s2_422+1));; 500) s2_500=$((s2_500+1));; *) s2_other=$((s2_other+1));; esac; done
    echo "  S2 round $r →$codes"
  done
  echo "S2 结果：201×$s2_ok  422×$s2_422  500×$s2_500  其它×$s2_other  （共 $((S2_ROUNDS*4))）"

  echo "### S3 异视图 4 并发（front/left/back/right 各一；理想 4×201）"
  s3_ok=0; s3_500=0; s3_other=0
  for r in $(seq 1 "$S3_ROUNDS"); do
    new_item_asset "S3-${r}"
    pids=(); idx=0
    for v in front left back right; do
      idx=$((idx+1))
      photo_post "$v" "$WORK/s3-$r-$idx.json" > "$WORK/s3-$r-$idx.code" &
      pids+=($!)
    done
    for pid in "${pids[@]}"; do wait "$pid"; done
    codes=""; for i in 1 2 3 4; do c=$(cat "$WORK/s3-$r-$i.code"); codes="$codes $c"; case "$c" in 201) s3_ok=$((s3_ok+1));; 500) s3_500=$((s3_500+1));; *) s3_other=$((s3_other+1));; esac; done
    echo "  S3 round $r →$codes"
  done
  echo "S3 结果：201×$s3_ok  500×$s3_500  其它×$s3_other  （共 $((S3_ROUNDS*4))）"

  echo "### 服务端日志统计"
  echo "database is locked 行数：$(grep -c 'database is locked' "$WORK/server.log")"
  echo "code 5（SQLITE_BUSY）：$(grep -c '(code: 5) database is locked' "$WORK/server.log")"
  echo "code 517（SQLITE_BUSY_SNAPSHOT）：$(grep -c '(code: 517) database is locked' "$WORK/server.log")"
  echo 'status":500 行数：'"$(grep -c '"status":500' "$WORK/server.log")"
  echo "500 的路径分布："
  grep '"status":500' "$WORK/server.log" | python3 -c '
import sys, json, collections
paths = collections.Counter()
for line in sys.stdin:
    try:
        rec = json.loads(line)
        paths[(rec["fields"]["method"], rec["fields"]["path"].rsplit("/", 1)[-1])] += 1
    except Exception:
        pass
for (method, tail), n in paths.most_common(10):
    print(f"  {method} .../{tail}: {n}")
'
} 2>&1 | tee "$OUT"
