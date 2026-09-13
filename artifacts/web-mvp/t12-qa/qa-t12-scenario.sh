#!/bin/bash
# T12 QA 回合 14 独立验收：单场景端到端（真实二进制 + QA 自建本机 fixture + curl）。
# 用法：qa-t12-scenario.sh <binary> <scenario> <workdir> <app_port> <fixture_port>
# 覆盖：init/serve → 建物品/资料/准备/照片 → estimate → confirm → 建单 →
#       后台执行器真实跑 Tripo 链（上传→付费提交→轮询）→ 场景断言（qa-t12-verify.py）。
# 全程只连 127.0.0.1；结束时清理临时目录与进程。
set -uo pipefail

BIN="${1:?binary}"
SCENARIO="${2:?scenario}"
WORK="${3:?workdir}"
APP_PORT="${4:?app port}"
FIXTURE_PORT="${5:?fixture port}"

QA_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$QA_DIR/../../.." && pwd)"
FIXTURES="$REPO/tests/fixtures/assets"
FIXTURE_PY="$QA_DIR/qa-tripo-fixture.py"
VERIFY_PY="$QA_DIR/qa-t12-verify.py"
FAKE_KEY="qa-fake-tripo-key-9931"
PASSWORD="qa-t12-password-4f81"

mkdir -p "$WORK"
cd "$WORK" || exit 1

cleanup() {
  [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
  [ -n "${FIXTURE_PID:-}" ] && kill "$FIXTURE_PID" 2>/dev/null
  [ -n "${LSOF_PID:-}" ] && kill "$LSOF_PID" 2>/dev/null
  wait 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

printf '%s\n' "$PASSWORD" > pw.txt && chmod 600 pw.txt
printf 'a%.0s' $(seq 1 3000) > page-text.txt

status_of() { # $1=stage_kind → 打印状态
  python3 - "$1" <<'PY'
import sqlite3, sys
con = sqlite3.connect('./data/manual.sqlite3')
row = con.execute("select status from job_stages where job_id = ? and stage_kind = ?",
                  (open('job_id.txt').read().strip(), sys.argv[1])).fetchone()
print(row[0] if row else 'missing')
PY
}

wait_for_stage() { # $1=stage_kind $2=期望状态 $3=最长秒数
  local deadline=$(( $(date +%s) + $3 ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    local current; current=$(status_of "$1")
    [ "$current" = "$2" ] && return 0
    sleep 1
  done
  echo "!! 等待 $1 -> $2 超时（实际 $(status_of "$1")）"
  return 1
}

echo "== 场景 ${SCENARIO}（工作目录 ${WORK}）"
python3 "$FIXTURE_PY" --port "$FIXTURE_PORT" --log "$WORK/fixture.jsonl" --scenario "$SCENARIO" > fixture.out 2>&1 &
FIXTURE_PID=$!
for _ in $(seq 1 100); do
  grep -q "listening" fixture.out 2>/dev/null && break
  sleep 0.1
done
cat fixture.out

"$BIN" init --data-dir ./data --password-file ./pw.txt > init.log 2>&1 || { echo "init 失败"; cat init.log; exit 1; }
cp "$REPO/price-catalog.example.toml" ./prices.toml
cat > ./data/config.toml <<TOML
price_catalog_path = "./prices.toml"

[providers.tripo]
base_url = "http://127.0.0.1:${FIXTURE_PORT}/v3"
api_key_env = "QA_TRIPO_KEY"

[providers.manual_ai]
model = "gpt-5-mini"
api_key_env = "QA_MANUAL_AI_KEY"
TOML

QA_TRIPO_KEY="$FAKE_KEY" QA_MANUAL_AI_KEY="qa-fake-manual-ai-key" \
  "$BIN" serve --data-dir ./data --listen 127.0.0.1:${APP_PORT} > serve.log 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
grep -o '"event":"provider_handlers_registered"[^}]*' serve.log | head -1

( while kill -0 "$SERVER_PID" 2>/dev/null; do
    lsof -p "$SERVER_PID" -a -i -P -n 2>/dev/null >> "$WORK/lsof-samples.txt"
    sleep 0.2
  done ) &
LSOF_PID=$!

BASE="http://127.0.0.1:${APP_PORT}/api/v1"
json() { python3 -c "import json,sys;print(json.load(sys.stdin)$1)"; }

curl -s -c cookies.txt -H 'content-type: application/json' \
  -d "{\"password\":\"${PASSWORD}\"}" "$BASE/auth/login" -o login.json
CSRF=$(json "['data']['csrfToken']" < login.json)
AUTH=(-b cookies.txt -H "x-csrf-token: $CSRF")

ITEM=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"QA T12 相机","model":"QA-T12"}' "$BASE/items" | json "['data']['id']")
upload() { curl -s "${AUTH[@]}" -F "purpose=$2" -F "file=@$1" "$BASE/items/$ITEM/assets"; }
DOC_JSON=$(upload "$FIXTURES/sample-manual-text.pdf" document)
DOC=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceAssetId\":\"$(echo "$DOC_JSON" | json "['data']['id']")\",\"title\":\"QA 说明书\"}" \
  "$BASE/items/$ITEM/documents" | json "['data']['id']")
PDF_SHA=$(echo "$DOC_JSON" | json "['data']['sha256']")
PREP=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceSha256\":\"$PDF_SHA\"}" "$BASE/documents/$DOC/preparations" | json "['data']['id']")
for page in 1 2; do
  TXT=$(upload ./page-text.txt pageText | json "['data']['id']")
  IMG=$(upload "$FIXTURES/sample-photo-front.jpg" pageImage | json "['data']['id']")
  curl -s -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
    -d "{\"textAssetId\":\"$TXT\",\"imageAssetId\":\"$IMG\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
    "$BASE/preparations/$PREP/pages/$page" -o "page$page.json"
done
ETAG=$(curl -s "${AUTH[@]}" -D - -o /dev/null "$BASE/preparations/$PREP" | awk -F'"' 'tolower($1) ~ /^etag:/ {print $2}')
curl -s "${AUTH[@]}" -H 'content-type: application/json' -H "if-match: \"$ETAG\"" \
  -d '{"pageCount":2}' "$BASE/preparations/$PREP/complete" -o complete.json

FRONT_ASSET=$(upload "$FIXTURES/sample-photo-front.jpg" photo | json "['data']['id']")
LEFT_ASSET=$(upload "$FIXTURES/sample-photo-left.png" photo | json "['data']['id']")
FRONT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$FRONT_ASSET\",\"view\":\"front\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
LEFT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$LEFT_ASSET\",\"view\":\"left\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")

QUOTE=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"modelPreset\":\"tripo-h-v3.1-standard\"}" \
  "$BASE/items/$ITEM/estimates" | json "['data']['id']")
curl -s -X POST "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$QUOTE/confirm" -o confirm.json
JOB=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' -H 'idempotency-key: qa-t12-key' \
  -d "{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3000,\"manualAiUsdMicros\":50000}}" \
  "$BASE/items/$ITEM/jobs" | json "['data']['id']")
echo "$JOB" > job_id.txt
echo "== job=$JOB"

RESULT=0
case "$SCENARIO" in
  happy)          wait_for_stage tripo_poll succeeded 60 || RESULT=1 ;;
  poll_503)       wait_for_stage tripo_poll succeeded 60 || RESULT=1 ;;
  no_billing)     wait_for_stage tripo_poll succeeded 60 || RESULT=1 ;;
  token_verbatim) wait_for_stage tripo_poll succeeded 60 || RESULT=1 ;;
  business_error) wait_for_stage tripo_submit failed 45 || RESULT=1 ;;
  unknown)        wait_for_stage tripo_submit succeeded 45 || RESULT=1
                  sleep 8 ;;
  no_model)       wait_for_stage tripo_poll retry_wait 45 || RESULT=1 ;;
  token_unknown)  wait_for_stage tripo_upload retry_wait 45 || RESULT=1 ;;
  submit_429)     wait_for_stage tripo_poll succeeded 90 || RESULT=1 ;;
  disconnect)
    wait_for_stage tripo_submit submission_unknown 45 || RESULT=1
    echo "== 断连后：把阶段强行放回队列（模拟恢复/重试入口）"
    python3 - <<'PY'
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
job = open('job_id.txt').read().strip()
con.execute("update job_stages set status='queued', next_run_at=NULL where job_id=? and stage_kind='tripo_submit'", (job,))
con.commit()
PY
    sleep 6
    echo "== 重启服务（SIGKILL 模拟崩溃，同一 data-dir）"
    kill -9 "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null
    QA_TRIPO_KEY="$FAKE_KEY" QA_MANUAL_AI_KEY="qa-fake-manual-ai-key" \
      "$BIN" serve --data-dir ./data --listen 127.0.0.1:${APP_PORT} >> serve.log 2>&1 &
    SERVER_PID=$!
    for _ in $(seq 1 100); do
      curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
      sleep 0.1
    done
    sleep 6
    ;;
  *) echo "未知场景 $SCENARIO"; exit 2 ;;
esac

kill "$LSOF_PID" 2>/dev/null; wait "$LSOF_PID" 2>/dev/null
TOTAL_SAMPLES=$(wc -l < lsof-samples.txt 2>/dev/null | tr -d ' ')
NON_LOOPBACK=$(grep -c "->" lsof-samples.txt 2>/dev/null | head -1)
NON_LOOPBACK=$(grep "->" lsof-samples.txt 2>/dev/null | grep -v "127.0.0.1" | grep -c . || true)
echo "== lsof 采样：${TOTAL_SAMPLES} 行；非 loopback 连接行 ${NON_LOOPBACK}（应为 0）"
[ "${NON_LOOPBACK:-0}" = "0" ] || RESULT=1

FRONT_SHA=$(shasum -a 256 "$FIXTURES/sample-photo-front.jpg" | awk '{print $1}')
LEFT_SHA=$(shasum -a 256 "$FIXTURES/sample-photo-left.png" | awk '{print $1}')
python3 "$VERIFY_PY" "$SCENARIO" ./data/manual.sqlite3 "$JOB" "$WORK/fixture.jsonl" \
  "$FRONT_SHA" "$LEFT_SHA" "$FAKE_KEY" ./serve.log || RESULT=1

# 证据留存（fixture 字节记录 + serve 日志；工作目录随后由 trap 删除）。
cp "$WORK/fixture.jsonl" "$QA_DIR/qa-${SCENARIO}-fixture.jsonl" 2>/dev/null
cp ./serve.log "$QA_DIR/qa-${SCENARIO}-serve.log" 2>/dev/null
cp "$WORK/lsof-samples.txt" "$QA_DIR/qa-${SCENARIO}-lsof.txt" 2>/dev/null
cp ./data/manual.sqlite3 "$QA_DIR/qa-${SCENARIO}-manual.sqlite3" 2>/dev/null

# 临时 data-dir 与进程由 trap 清理；核对无残留由外层脚本做。
echo "== 场景 ${SCENARIO} 结果：$([ $RESULT = 0 ] && echo PASS || echo FAIL)"
exit $RESULT
