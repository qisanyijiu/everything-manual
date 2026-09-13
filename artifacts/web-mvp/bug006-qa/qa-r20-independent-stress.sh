#!/usr/bin/env bash
# QA 回合 20 · BUG-006 复验：独立并发压测（QA 自写；不复用 RD 的 photos_concurrency 用例）。
#
# 覆盖（均为"真实二进制 + 真实 HTTP 合同 + 真实 multipart 上传"）：
#   P1 同物品多视图并发（4 视图 ×15 轮）→ 每轮 4×201
#   P2 同视图 6 并发（超过连接池 4）×10 轮 → 每轮恰好 1×201 + 5×422 `viewOccupied`
#      （附：每个 422 的 details.reason 必须为 viewOccupied；GET /photos 该视图恰好 1 张）
#   P3 跨物品并发（8 物品 × 4 视图 = 32 并发）→ 全 201
#   P4 常驻外部写者（python `BEGIN IMMEDIATE` 循环，每 ~4ms 取写锁）下并发：
#      同物品 4 视图 ×10 轮 + 同视图 4 并发 ×10 轮 → 0×500、422 语义正确（并统计写者竞争次数）
#   P5 prepare/complete 与照片写并发：5 轮（PUT page + 4 照片写并发）+ 3 轮（complete + 2 照片写并发）
#   P6 60 并发突发（3 物品 × 5 视图 × 4 份，混合同视图冲突）→ 0×500
#   P7 服务端日志：database is locked / code 5 / code 517 / status:500 全为 0
#
# 断言只使用合同状态码与 0×500（同 RD 守卫口径），不卡紧时序。
# 用法：bash qa-r20-independent-stress.sh <输出日志> <服务端日志副本路径>
set -uo pipefail

OUT="${1:?用法: qa-r20-independent-stress.sh <输出日志> <server日志副本>}"
SERVER_LOG_OUT="${2:?需要 server 日志副本路径}"
REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
BIN="$REPO_ROOT/target/debug/everything-manual"
WORK="$(mktemp -d /tmp/em-r20-stress.XXXXXX)"
PORT=18095
BASE="http://127.0.0.1:$PORT"
PASSWORD="qa-r20-stress-password"
PHOTO="$REPO_ROOT/tests/fixtures/assets/sample-photo-front.jpg"
PHOTO2="$REPO_ROOT/tests/fixtures/assets/sample-photo-left.png"
PDF="$REPO_ROOT/tests/fixtures/assets/sample-manual-text.pdf"

STATUSES="$WORK/statuses.txt"
FAILS="$WORK/failures.txt"
: > "$STATUSES"; : > "$FAILS"

log() { echo "[$(date +%H:%M:%S)] $*"; }
record() { echo "$1 $2" >> "$STATUSES"; }
fail() { echo "FAIL: $*" | tee -a "$FAILS"; }
ok() { echo "  OK: $*"; }

cleanup() {
  if [ -n "${WRITER_PID:-}" ] && kill -0 "$WRITER_PID" 2>/dev/null; then kill "$WRITER_PID" 2>/dev/null; fi
  pkill -f "listen 127.0.0.1:${PORT}" 2>/dev/null
  if [ -f "$WORK/server.log" ]; then cp "$WORK/server.log" "$SERVER_LOG_OUT"; fi
  if [ -f "$WORK/writer.log" ]; then cp "$WORK/writer.log" "${SERVER_LOG_OUT%.log}-writer.log"; fi
  if [ -f "$STATUSES" ]; then cp "$STATUSES" "${SERVER_LOG_OUT%.log}-statuses.txt"; fi
  log "清理临时目录 ${WORK}"
  rm -rf "$WORK"
}
trap cleanup EXIT

json_get() { # $1=file $2=dotted.path
  python3 -c '
import json, sys
with open(sys.argv[1]) as handle:
    value = json.load(handle)
for key in sys.argv[2].split("."):
    value = value[int(key)] if isinstance(value, list) else value[key]
print(value)
' "$1" "$2"
}

{
  # ---------- 准备 ----------
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
  log "服务端就绪（端口 ${PORT}，data-dir ${WORK}，执行器 idle_poll=250ms 常驻）"

  CSRF=$(curl -fsS -c "$WORK/cookies.txt" -X POST "$BASE/api/v1/auth/login" \
    -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["csrfToken"])')
  JAR="$WORK/cookies.txt"

  new_item() { # $1=标签 → echo item_id
    curl -fsS -b "$JAR" -X POST "$BASE/api/v1/items" -H "x-csrf-token: $CSRF" \
      -H 'content-type: application/json' -d "{\"name\":\"QA 压测 $1\",\"model\":\"QA-R20-STRESS\"}" \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])'
  }
  up_asset() { # $1=item $2=purpose $3=file → echo asset_id
    curl -fsS -b "$JAR" -X POST "$BASE/api/v1/items/$1/assets" -H "x-csrf-token: $CSRF" \
      -F "purpose=$2" -F "file=@$3" \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])'
  }
  post_photo_bg() { # $1=item $2=asset $3=view $4=outfile（后台）
    curl -sS -b "$JAR" -o "$4.json" -w '%{http_code}' -X POST "$BASE/api/v1/items/$1/photos" \
      -H "x-csrf-token: $CSRF" -H 'content-type: application/json' \
      -d "{\"assetId\":\"$2\",\"view\":\"$3\"}" > "$4.code" &
  }
  wait_all() { local p; for p in "$@"; do wait "$p"; done; }

  # ---------- P1 同物品多视图并发 ----------
  log "== P1 同物品多视图并发（15 轮 × 4 视图；期望每轮 4×201）=="
  p1_ok=0; p1_bad=0
  for r in $(seq 1 15); do
    ITEM=$(new_item "P1-$r"); ASSET=$(up_asset "$ITEM" photo "$PHOTO")
    pids=(); i=0
    for v in front left back right; do
      i=$((i+1)); post_photo_bg "$ITEM" "$ASSET" "$v" "$WORK/p1-$r-$i"; pids+=($!)
    done
    wait_all "${pids[@]}"
    for i in 1 2 3 4; do
      c=$(cat "$WORK/p1-$r-$i.code"); record P1 "$c"
      if [ "$c" = "201" ]; then p1_ok=$((p1_ok+1)); else p1_bad=$((p1_bad+1)); fail "P1 round $r: HTTP $c $(head -c 160 "$WORK/p1-$r-$i.json")"; fi
    done
  done
  ok "P1: 201×${p1_ok}，异常×$p1_bad"

  # ---------- P2 同视图 6 并发 ----------
  log "== P2 同视图 6 并发（超过连接池 4）×10 轮；期望每轮 1×201 + 5×422 viewOccupied =="
  p2_ok=0; p2_422=0; p2_bad=0
  for r in $(seq 1 10); do
    ITEM=$(new_item "P2-$r"); ASSET=$(up_asset "$ITEM" photo "$PHOTO")
    pids=(); i=0
    for i in 1 2 3 4 5 6; do
      post_photo_bg "$ITEM" "$ASSET" front "$WORK/p2-$r-$i"; pids+=($!)
    done
    wait_all "${pids[@]}"
    round_201=0; round_422=0
    for i in 1 2 3 4 5 6; do
      c=$(cat "$WORK/p2-$r-$i.code"); record P2 "$c"
      case "$c" in
        201) round_201=$((round_201+1)); p2_ok=$((p2_ok+1));;
        422) round_422=$((round_422+1)); p2_422=$((p2_422+1))
             reason=$(json_get "$WORK/p2-$r-$i.json" "error.details.reason" 2>/dev/null || echo "?")
             [ "$reason" = "viewOccupied" ] || { p2_bad=$((p2_bad+1)); fail "P2 round $r: 422 但 reason=$reason"; };;
        *) p2_bad=$((p2_bad+1)); fail "P2 round $r: HTTP $c $(head -c 160 "$WORK/p2-$r-$i.json")";;
      esac
    done
    [ "$round_201" -eq 1 ] && [ "$round_422" -eq 5 ] || { p2_bad=$((p2_bad+1)); fail "P2 round $r: 201×$round_201 / 422×${round_422}（期望 1/5）"; }
    # 落库核对：该视图恰好 1 张（不被静默接受第二张）
    curl -fsS -b "$JAR" "$BASE/api/v1/items/$ITEM/photos" -o "$WORK/p2-$r-list.json"
    count=$(python3 -c '
import json,sys
data = json.load(open(sys.argv[1]))["data"]
photos = data["items"] if isinstance(data, dict) else data
print(len([p for p in photos if p["view"] == "front"]))
' "$WORK/p2-$r-list.json")
    [ "$count" = "1" ] || { p2_bad=$((p2_bad+1)); fail "P2 round $r: 落库该视图 $count 张（期望 1）"; }
  done
  ok "P2: 201×$p2_ok / 422×${p2_422}（均 viewOccupied）/ 异常×$p2_bad"

  # ---------- P3 跨物品并发 ----------
  log "== P3 跨物品并发：8 物品 × 4 视图 = 32 并发（期望全 201）=="
  p3_ok=0; p3_bad=0
  declare -a p3_items=() p3_assets=()
  for n in 1 2 3 4 5 6 7 8; do
    ITEM=$(new_item "P3-$n"); ASSET=$(up_asset "$ITEM" photo "$PHOTO")
    p3_items+=("$ITEM"); p3_assets+=("$ASSET")
  done
  pids=(); idx=0
  for n in 0 1 2 3 4 5 6 7; do
    for v in front left back right; do
      idx=$((idx+1)); post_photo_bg "${p3_items[$n]}" "${p3_assets[$n]}" "$v" "$WORK/p3-$idx"; pids+=($!)
    done
  done
  wait_all "${pids[@]}"
  for i in $(seq 1 32); do
    c=$(cat "$WORK/p3-$i.code"); record P3 "$c"
    if [ "$c" = "201" ]; then p3_ok=$((p3_ok+1)); else p3_bad=$((p3_bad+1)); fail "P3 #$i: HTTP $c $(head -c 160 "$WORK/p3-$i.json")"; fi
  done
  ok "P3: 201×$p3_ok / 异常×$p3_bad"

  # ---------- P4 常驻外部写者下并发 ----------
  log "== P4 常驻外部写者（python BEGIN IMMEDIATE 循环，~4ms/次）下并发 =="
  rm -f "$WORK/writer.stop"
  python3 - "$WORK/data/manual.sqlite3" "$WORK/writer.stop" > "$WORK/writer.log" 2>&1 <<'PY' &
import os, sqlite3, sys, time
db, stop_file = sys.argv[1], sys.argv[2]
con = sqlite3.connect(db, timeout=10.0)
con.execute("PRAGMA busy_timeout=5000")
con.execute("CREATE TABLE IF NOT EXISTS qa_probe_writes (n INTEGER)")
con.commit()
acquired = busy = 0
n = 0
start = time.time()
while not os.path.exists(stop_file):
    try:
        con.execute("BEGIN IMMEDIATE")
        con.execute("INSERT INTO qa_probe_writes VALUES (?)", (n,))
        con.commit()
        acquired += 1
    except sqlite3.OperationalError:
        try:
            con.rollback()
        except sqlite3.OperationalError:
            pass
        busy += 1
    n += 1
    if n % 100 == 0:
        print(f"writer progress: acquired={acquired} busy={busy} total={n} elapsed={time.time()-start:.1f}s", flush=True)
    time.sleep(0.004)
print(f"writer: acquired={acquired} busy={busy} total={n}")
PY
  WRITER_PID=$!
  sleep 0.5
  # 写者确实在持锁：单进程快速探针（busy_timeout=0，500 次抢锁，统计被拒次数）
  probe_out=$(python3 - "$WORK/data/manual.sqlite3" <<'PY'
import sqlite3, sys
busy = 0
attempts = 500
for _ in range(attempts):
    con = sqlite3.connect(sys.argv[1], timeout=0)
    con.execute("PRAGMA busy_timeout=0")
    try:
        con.execute("BEGIN IMMEDIATE")
        con.rollback()
    except sqlite3.OperationalError:
        busy += 1
    con.close()
print(f"probe: attempts={attempts} busy={busy}")
PY
)
  log "   写者竞争探针（busy_timeout=0）：${probe_out}（busy>0 说明写者确实在持锁竞争）"

  p4_ok=0; p4_422=0; p4_bad=0
  for r in $(seq 1 10); do
    ITEM=$(new_item "P4-A-$r"); ASSET=$(up_asset "$ITEM" photo "$PHOTO")
    pids=(); i=0
    for v in front left back right; do
      i=$((i+1)); post_photo_bg "$ITEM" "$ASSET" "$v" "$WORK/p4a-$r-$i"; pids+=($!)
    done
    wait_all "${pids[@]}"
    for i in 1 2 3 4; do
      c=$(cat "$WORK/p4a-$r-$i.code"); record P4 "$c"
      if [ "$c" = "201" ]; then p4_ok=$((p4_ok+1)); else p4_bad=$((p4_bad+1)); fail "P4-A round $r: HTTP $c $(head -c 160 "$WORK/p4a-$r-$i.json")"; fi
    done
    ITEM=$(new_item "P4-B-$r"); ASSET=$(up_asset "$ITEM" photo "$PHOTO2")
    pids=(); i=0
    for i in 1 2 3 4; do
      post_photo_bg "$ITEM" "$ASSET" detail "$WORK/p4b-$r-$i"; pids+=($!)
    done
    wait_all "${pids[@]}"
    r201=0; r422=0
    for i in 1 2 3 4; do
      c=$(cat "$WORK/p4b-$r-$i.code"); record P4 "$c"
      case "$c" in
        201) r201=$((r201+1)); p4_ok=$((p4_ok+1));;
        422) r422=$((r422+1)); p4_422=$((p4_422+1))
             reason=$(json_get "$WORK/p4b-$r-$i.json" "error.details.reason" 2>/dev/null || echo "?")
             [ "$reason" = "viewOccupied" ] || { p4_bad=$((p4_bad+1)); fail "P4-B round $r: 422 但 reason=$reason"; };;
        *) p4_bad=$((p4_bad+1)); fail "P4-B round $r: HTTP $c $(head -c 160 "$WORK/p4b-$r-$i.json")";;
      esac
    done
    [ "$r201" -eq 1 ] && [ "$r422" -eq 3 ] || { p4_bad=$((p4_bad+1)); fail "P4-B round $r: 201×$r201 / 422×${r422}（期望 1/3）"; }
  done
  touch "$WORK/writer.stop"; wait "$WRITER_PID" 2>/dev/null; WRITER_PID=""
  log "   写者统计：$(tail -1 "$WORK/writer.log")"
  grep -q "acquired=[1-9]" "$WORK/writer.log" || fail "P4: 常驻写者未成功持锁（测试前提不成立）：$(tail -3 "$WORK/writer.log" | tr '\n' ' ')"
  ok "P4: 201×$p4_ok / 422×$p4_422 / 异常×$p4_bad"

  # ---------- P5 prepare/complete 与照片写并发 ----------
  log "== P5 prepare/complete 与照片写并发 =="
  p5_ok=0; p5_bad=0
  for r in $(seq 1 5); do
    ITEM=$(new_item "P5-$r")
    PDF_ASSET=$(up_asset "$ITEM" document "$PDF")
    DOC=$(curl -fsS -b "$JAR" -X POST "$BASE/api/v1/items/$ITEM/documents" -H "x-csrf-token: $CSRF" \
      -H 'content-type: application/json' -d "{\"sourceAssetId\":\"$PDF_ASSET\",\"title\":\"QA P5 $r\"}" \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])')
    SHA=$(curl -fsS -b "$JAR" "$BASE/api/v1/items/$ITEM/documents" | python3 -c '
import json,sys
payload = json.load(sys.stdin)["data"]
docs = payload["items"] if isinstance(payload, dict) else payload
print(docs[0]["sourceSha256"])')
    PREP=$(curl -fsS -b "$JAR" -X POST "$BASE/api/v1/documents/$DOC/preparations" -H "x-csrf-token: $CSRF" \
      -H 'content-type: application/json' -d "{\"sourceSha256\":\"$SHA\"}" \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])')
    IMG=$(up_asset "$ITEM" pageImage "$PHOTO")
    PHOTO_ASSET=$(up_asset "$ITEM" photo "$PHOTO")
    PHOTO_ASSET2=$(up_asset "$ITEM" photo "$PHOTO2")
    # 并发：PUT 第 1/2 页 + 4 视图照片写
    pids=()
    curl -sS -b "$JAR" -o "$WORK/p5-$r-page1.json" -w '%{http_code}' -X PUT \
      "$BASE/api/v1/preparations/$PREP/pages/1" -H "x-csrf-token: $CSRF" -H 'content-type: application/json' \
      -d "{\"imageAssetId\":\"$IMG\",\"viewport\":{\"width\":800,\"height\":600,\"rotation\":0}}" > "$WORK/p5-$r-page1.code" &
    pids+=($!)
    curl -sS -b "$JAR" -o "$WORK/p5-$r-page2.json" -w '%{http_code}' -X PUT \
      "$BASE/api/v1/preparations/$PREP/pages/2" -H "x-csrf-token: $CSRF" -H 'content-type: application/json' \
      -d "{\"imageAssetId\":\"$IMG\",\"viewport\":{\"width\":800,\"height\":600,\"rotation\":90}}" > "$WORK/p5-$r-page2.code" &
    pids+=($!)
    i=0
    for v in front left back right; do
      i=$((i+1)); post_photo_bg "$ITEM" "$PHOTO_ASSET" "$v" "$WORK/p5-$r-p$i"; pids+=($!)
    done
    wait_all "${pids[@]}"
    for f in page1 page2; do
      c=$(cat "$WORK/p5-$r-$f.code"); record P5 "$c"
      if [ "$c" = "200" ]; then p5_ok=$((p5_ok+1)); else p5_bad=$((p5_bad+1)); fail "P5 round $r $f: HTTP $c $(head -c 200 "$WORK/p5-$r-$f.json")"; fi
    done
    for i in 1 2 3 4; do
      c=$(cat "$WORK/p5-$r-p$i.code"); record P5 "$c"
      if [ "$c" = "201" ]; then p5_ok=$((p5_ok+1)); else p5_bad=$((p5_bad+1)); fail "P5 round $r photo$i: HTTP $c $(head -c 200 "$WORK/p5-$r-p$i.json")"; fi
    done
    # 并发：complete + 照片写（detail 视图尚空）
    ETAG=$(curl -fsS -D - -o /dev/null -b "$JAR" "$BASE/api/v1/preparations/$PREP" | grep -i '^etag:' | tr -d '\r' | awk '{print $2}')
    pids=()
    curl -sS -b "$JAR" -o "$WORK/p5-$r-complete.json" -w '%{http_code}' -X POST \
      "$BASE/api/v1/preparations/$PREP/complete" -H "x-csrf-token: $CSRF" -H "if-match: $ETAG" \
      -H 'content-type: application/json' -d '{"pageCount":2}' > "$WORK/p5-$r-complete.code" &
    pids+=($!)
    post_photo_bg "$ITEM" "$PHOTO_ASSET2" detail "$WORK/p5-$r-detail1"; pids+=($!)
    post_photo_bg "$ITEM" "$PHOTO_ASSET" detail "$WORK/p5-$r-detail2"; pids+=($!)
    wait_all "${pids[@]}"
    c=$(cat "$WORK/p5-$r-complete.code"); record P5 "$c"
    if [ "$c" = "200" ]; then p5_ok=$((p5_ok+1)); else p5_bad=$((p5_bad+1)); fail "P5 round $r complete: HTTP $c $(head -c 200 "$WORK/p5-$r-complete.json")"; fi
    d201=0; d422=0
    for i in 1 2; do
      c=$(cat "$WORK/p5-$r-detail$i.code"); record P5 "$c"
      if [ "$c" = "201" ]; then d201=$((d201+1)); p5_ok=$((p5_ok+1)); elif [ "$c" = "422" ]; then d422=$((d422+1)); p5_ok=$((p5_ok+1)); else p5_bad=$((p5_bad+1)); fail "P5 round $r detail$i: HTTP $c $(head -c 200 "$WORK/p5-$r-detail$i.json")"; fi
    done
    [ "$d201" -eq 1 ] && [ "$d422" -eq 1 ] || log "   P5 round $r 观察：detail 并发 201×$d201 / 422×${d422}（时序性，不计失败）"
  done
  ok "P5: 期望码计数×$p5_ok / 异常×$p5_bad"

  # ---------- P6 60 并发突发 ----------
  log "== P6 60 并发突发（3 物品 × 5 视图 × 4 份；同视图冲突混合）=="
  p6_ok=0; p6_422=0; p6_bad=0
  declare -a p6_items=() p6_assets=()
  for n in 1 2 3; do
    ITEM=$(new_item "P6-$n"); ASSET=$(up_asset "$ITEM" photo "$PHOTO")
    p6_items+=("$ITEM"); p6_assets+=("$ASSET")
  done
  pids=(); idx=0
  for n in 0 1 2; do
    for v in front left back right detail; do
      for dup in 1 2 3 4; do
        idx=$((idx+1))
        post_photo_bg "${p6_items[$n]}" "${p6_assets[$n]}" "$v" "$WORK/p6-$idx"; pids+=($!)
      done
    done
  done
  wait_all "${pids[@]}"
  for i in $(seq 1 60); do
    c=$(cat "$WORK/p6-$i.code"); record P6 "$c"
    case "$c" in
      201) p6_ok=$((p6_ok+1));;
      422) p6_422=$((p6_422+1));;
      *) p6_bad=$((p6_bad+1)); fail "P6 #$i: HTTP $c $(head -c 160 "$WORK/p6-$i.json")";;
    esac
  done
  ok "P6: 201×$p6_ok / 422×$p6_422 / 异常×$p6_bad"

  # ---------- P7 服务端日志 ----------
  log "== P7 服务端日志（并发错误与 500）=="
  locked=$(grep -c "database is locked" "$WORK/server.log" || true)
  code5=$(grep -c "(code: 5) database is locked" "$WORK/server.log" || true)
  code517=$(grep -c "(code: 517) database is locked" "$WORK/server.log" || true)
  s500=$(grep -c '"status":500' "$WORK/server.log" || true)
  log "   database is locked=$locked  code5=$code5  code517=$code517  status:500=$s500"
  [ "$locked" = "0" ] || fail "P7: database is locked 命中 $locked 次"
  [ "$code5" = "0" ] || fail "P7: code 5 命中 $code5 次"
  [ "$code517" = "0" ] || fail "P7: code 517 命中 $code517 次"
  [ "$s500" = "0" ] || fail "P7: status:500 命中 $s500 次"

  # ---------- 汇总 ----------
  log "== 状态码直方图（阶段 × 状态 × 次数）=="
  sort "$STATUSES" | uniq -c | sort -k2,2 -k3,3
  log "== 每阶段汇总 =="
  for p in P1 P2 P3 P4 P5 P6; do
    total=$(awk -v p="$p" '$1==p' "$STATUSES" | wc -l | tr -d ' ')
    bad=$(awk -v p="$p" '$1==p && $2!=201 && $2!=200 && $2!=422' "$STATUSES" | wc -l | tr -d ' ')
    echo "  $p: 请求 ${total}，非 {200,201,422} 的响应 $bad"
  done
  log "== 断言失败 =="
  if [ -s "$FAILS" ]; then
    cat "$FAILS"
    log "总判定：FAIL（$(grep -c FAIL "$FAILS") 条断言失败）"
    exit 1
  else
    log "总判定：PASS（0×500、0×database is locked，合同状态码正确）"
  fi
} 2>&1 | tee "$OUT"
exit ${PIPESTATUS[0]}
