#!/usr/bin/env bash
# QA 回合 19：并发 POST /photos 500 "database is locked" 复现（自然并发路径 B：无外部锁）。
#
# 机制同上：photos.rs:90 的 deferred 事务（SELECT 占用 → INSERT 唯一索引）在 WAL 下读→写升级
# 遇活跃写者立即 SQLITE_BUSY。本脚本不使用任何外部写锁——并发全部来自服务端自己的客户端
# （多连接池 4 连接），复刻 e2e 全量首跑 QA-5 命中的场景。
#
# 用法：bash repro-conc-photos-500-natural.sh <输出日志> [轮数]
set -uo pipefail

OUT="${1:?用法: repro-conc-photos-500-natural.sh <输出日志> [轮数]}"
ROUNDS="${2:-40}"
REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
BIN="$REPO_ROOT/target/debug/everything-manual"
WORK="$(mktemp -d /tmp/em-r19-conc-B.XXXXXX)"
PORT=18092
BASE="http://127.0.0.1:$PORT"
PASSWORD="qa-r19-concurrency-password-b"
CONC=4

log() { echo "[$(date +%H:%M:%S)] $*"; }

cleanup() {
  if [ -n "${SERVER_PID:-}" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null
  fi
  pkill -f "listen 127.0.0.1:${PORT}" 2>/dev/null
  log "清理临时目录 ${WORK}"
  rm -rf "$WORK"
}
trap cleanup EXIT

{
  log "== 准备：临时 data-dir ${WORK}（真实二进制，端口 ${PORT}）=="
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
  log "服务端就绪 pid=${SERVER_PID}"

  CSRF=$(curl -fsS -c "$WORK/cookies.txt" -X POST "$BASE/api/v1/auth/login" \
    -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["csrfToken"])')

  log "== 自然并发：${ROUNDS} 轮 × ${CONC} 个并发 POST /photos（同一 item 同一 view=front；期望 1×201 + $((CONC-1))×422）=="
  : > "$WORK/statuses.txt"
  for round in $(seq 1 "$ROUNDS"); do
    ITEM=$(curl -fsS -b "$WORK/cookies.txt" -X POST "$BASE/api/v1/items" \
      -H "x-csrf-token: $CSRF" -H 'content-type: application/json' \
      -d "{\"name\":\"QA 并发 B 第 $round 轮\",\"model\":\"QA-R19-CONC-B\"}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])')
    ASSET=$(curl -fsS -b "$WORK/cookies.txt" -X POST "$BASE/api/v1/items/$ITEM/assets" \
      -H "x-csrf-token: $CSRF" -F purpose=photo \
      -F "file=@$REPO_ROOT/tests/fixtures/assets/sample-photo-front.jpg;type=image/jpeg" \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])')
    pids=()
    for i in $(seq 1 "$CONC"); do
      (
        curl -sS -b "$WORK/cookies.txt" -o "$WORK/r$round-$i.json" -w "%{http_code}\n" \
          -X POST "$BASE/api/v1/items/$ITEM/photos" -H "x-csrf-token: $CSRF" \
          -H 'content-type: application/json' -d "{\"assetId\":\"$ASSET\",\"view\":\"front\"}" \
          >> "$WORK/statuses.txt"
      ) &
      pids+=($!)
    done
    for pid in "${pids[@]}"; do wait "$pid"; done
    # 同轮不同 view 的并发（都应 201，用于覆盖非冲突型并发写）
    VIEWS=(left back right detail)
    pids=()
    for v in "${VIEWS[@]}"; do
      (
        curl -sS -b "$WORK/cookies.txt" -o "$WORK/r$round-$v.json" -w "%{http_code}\n" \
          -X POST "$BASE/api/v1/items/$ITEM/photos" -H "x-csrf-token: $CSRF" \
          -H 'content-type: application/json' -d "{\"assetId\":\"$ASSET\",\"view\":\"$v\"}" \
          >> "$WORK/statuses.txt"
      ) &
      pids+=($!)
    done
    for pid in "${pids[@]}"; do wait "$pid"; done
    # 并发写其它实体（物品）加压：同一时刻另有写事务
    pids=()
    for i in 1 2; do
      (
        curl -sS -b "$WORK/cookies.txt" -o /dev/null -w "%{http_code}\n" \
          -X POST "$BASE/api/v1/items" -H "x-csrf-token: $CSRF" \
          -H 'content-type: application/json' \
          -d "{\"name\":\"QA 并发 B 加压 $round-$i\",\"model\":\"QA-R19-CONC-B2\"}" \
          >> "$WORK/statuses.txt"
      ) &
      pids+=($!)
    done
    for pid in "${pids[@]}"; do wait "$pid"; done
  done

  log "== 结果直方图（HTTP 状态 × 次数）=="
  sort "$WORK/statuses.txt" | uniq -c | sort -rn

  log "== 服务端日志中的 database is locked / 500 =="
  grep -c "database is locked" "$WORK/server.log" | sed 's/^/database is locked 命中次数: /'
  grep -n "database is locked" "$WORK/server.log" | head -12
  grep -n '"status":500' "$WORK/server.log" | head -12

  log "== 每个 500 响应的 requestId 与请求体（前 6 条）=="
  for f in "$WORK"/r*.json; do
    if grep -q '"code":"INTERNAL"' "$f" 2>/dev/null; then
      echo "--- $f: $(head -c 240 "$f")"
    fi
  done | head -60
} 2>&1 | tee "$OUT"
