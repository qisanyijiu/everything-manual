#!/bin/bash
# T12 手工冒烟：**真实发布二进制** + 本机 Tripo fixture（python，仅 127.0.0.1）+ curl。
#
# 覆盖：init/serve（配置 tripo.base_url 指向本机 fixture）→ 建物品/资料/照片 →
#       estimate → confirm → 建单 → 后台执行器真实跑 Tripo 链：
#       上传（multipart 字段 file）→ 付费提交（只发一次）→ 轮询（先 running 后 success）
#       → 原始状态与归一化状态、modelUrl、credits 落库 → 账本按实际 credits 结算。
# 结束时清理临时目录；**全程零真实外网调用**（fixture 只绑定 127.0.0.1，
# 且日志里会核对请求目标）。
set -uo pipefail
BIN="${1:?用法: smoke-t12-e2e.sh <binary>}"
REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
FIXTURES="$REPO/tests/fixtures/assets"
FIXTURE_PY="$REPO/artifacts/web-mvp/t12-rd/tripo-fixture.py"
WORK="$(mktemp -d /tmp/em-t12-smoke-XXXXXX)"
APP_PORT=18221
FIXTURE_PORT=18222

cd "$WORK" || exit 1
cleanup() {
  [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
  [ -n "${FIXTURE_PID:-}" ] && kill "$FIXTURE_PID" 2>/dev/null
  wait "${SERVER_PID:-}" 2>/dev/null
  wait "${FIXTURE_PID:-}" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

printf 'smoke-password-t12-9f31c2\n' > pw.txt && chmod 600 pw.txt
printf 'a%.0s' $(seq 1 3000) > page-text.txt

json() { python3 -c "import json,sys;print(json.load(sys.stdin)$1)"; }

echo "== 工作目录：${WORK}（结束时删除）"
echo "== 启动本机 Tripo fixture（127.0.0.1:${FIXTURE_PORT}；只在回环监听）"
python3 "$FIXTURE_PY" --port "$FIXTURE_PORT" --log "$WORK/tripo-fixture.log" --running-polls 1 > fixture.log 2>&1 &
FIXTURE_PID=$!
READY=0
for _ in $(seq 1 100); do
  grep -q "listening" fixture.log 2>/dev/null && { READY=1; break; }
  sleep 0.1
done
[ "$READY" = 1 ] || { echo "fixture 未在 10s 内监听（fixture.log：）"; cat fixture.log; exit 1; }
grep "listening" fixture.log

echo "== 价格目录（version=2026-09-11；Tripo 30 credits）"
cp "$REPO/price-catalog.example.toml" ./prices.toml

echo "== init / serve（tripo.base_url 指向本机 fixture；凭据为测试专用假值）"
"$BIN" init --data-dir ./data --password-file ./pw.txt 2>&1 | tail -1
cat > ./data/config.toml <<TOML
price_catalog_path = "./prices.toml"

[providers.tripo]
base_url = "http://127.0.0.1:${FIXTURE_PORT}/v3"
api_key_env = "SMOKE_TRIPO_KEY"

[providers.manual_ai]
model = "gpt-5-mini"
api_key_env = "SMOKE_MANUAL_AI_KEY"
TOML
SMOKE_TRIPO_KEY="fake-tripo-key-not-a-real-credential" \
SMOKE_MANUAL_AI_KEY="fake-manual-ai-key-not-used" \
  "$BIN" serve --data-dir ./data --listen 127.0.0.1:${APP_PORT} > serve.log 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 80); do
  curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
BASE="http://127.0.0.1:${APP_PORT}/api/v1"
grep -o '"event":"provider_handlers_registered"[^}]*' serve.log | head -1

# 外呼采样：服务进程在整个冒烟期间的 TCP 连接快照（只应有 127.0.0.1）。
(
  while kill -0 "$SERVER_PID" 2>/dev/null; do
    lsof -p "$SERVER_PID" -a -i -P -n 2>/dev/null >> "$WORK/lsof-samples.txt"
    sleep 0.2
  done
) &
LSOF_PID=$!

echo "== 登录"
curl -s -c cookies.txt -H 'content-type: application/json' \
  -d '{"password":"smoke-password-t12-9f31c2"}' "$BASE/auth/login" -o login.json
CSRF=$(json "['data']['csrfToken']" < login.json)
AUTH=(-b cookies.txt -H "x-csrf-token: $CSRF")

echo "== 1) 物品 + 说明书 + 准备（2 页文字页）"
curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"T12 冒烟相机","model":"SK-T12"}' "$BASE/items" -o item.json
ITEM=$(json "['data']['id']" < item.json)
echo "  item=${ITEM}"

upload() { # $1=文件 $2=purpose
  curl -s "${AUTH[@]}" -F "purpose=$2" -F "file=@$1" "$BASE/items/$ITEM/assets"
}
DOC_ASSET_JSON=$(upload "$FIXTURES/sample-manual-text.pdf" document)
DOC_ASSET=$(echo "$DOC_ASSET_JSON" | json "['data']['id']")
PDF_SHA=$(echo "$DOC_ASSET_JSON" | json "['data']['sha256']")
DOC=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceAssetId\":\"$DOC_ASSET\",\"title\":\"冒烟说明书\"}" \
  "$BASE/items/$ITEM/documents" | json "['data']['id']")
PREP=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceSha256\":\"$PDF_SHA\"}" "$BASE/documents/$DOC/preparations" \
  | json "['data']['id']")
for page in 1 2; do
  PAGE_TEXT=$(upload ./page-text.txt pageText | json "['data']['id']")
  PAGE_IMAGE=$(upload "$FIXTURES/sample-photo-front.jpg" pageImage | json "['data']['id']")
  curl -s -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
    -d "{\"textAssetId\":\"$PAGE_TEXT\",\"imageAssetId\":\"$PAGE_IMAGE\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
    "$BASE/preparations/$PREP/pages/$page" -o "page$page.json"
  json "['data']['pageNumber']" < "page$page.json" > /dev/null \
    || { echo "第 $page 页写入失败："; cat "page$page.json"; exit 1; }
done
ETAG=$(curl -s "${AUTH[@]}" -D - -o /dev/null "$BASE/preparations/$PREP" \
  | awk -F'"' 'tolower($1) ~ /^etag:/ {print $2}')
curl -s "${AUTH[@]}" -H 'content-type: application/json' -H "if-match: \"$ETAG\"" \
  -d '{"pageCount":2}' "$BASE/preparations/$PREP/complete" -o complete.json
json "['data']['state']" < complete.json > /dev/null || { cat complete.json; exit 1; }
echo "  preparation=${PREP}（ready）"

echo "== 2) 照片：front + left"
FRONT_ASSET=$(upload "$FIXTURES/sample-photo-front.jpg" photo | json "['data']['id']")
LEFT_ASSET=$(upload "$FIXTURES/sample-photo-left.png" photo | json "['data']['id']")
FRONT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$FRONT_ASSET\",\"view\":\"front\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
LEFT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$LEFT_ASSET\",\"view\":\"left\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
echo "  front=${FRONT_ID} left=${LEFT_ID}"

echo "== 3) estimate + confirm + 建单"
curl -s -o estimate.json -w '  estimate HTTP %{http_code}\n' "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"modelPreset\":\"tripo-h-v3.1-standard\"}" \
  "$BASE/items/$ITEM/estimates"
QUOTE=$(json "['data']['id']" < estimate.json)
curl -s -o confirm.json -w '  confirm HTTP %{http_code}\n' -X POST "${AUTH[@]}" \
  "$BASE/items/$ITEM/estimates/$QUOTE/confirm"
BODY="{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3000,\"manualAiUsdMicros\":50000}}"
curl -s -o job.json -w '  jobs HTTP %{http_code}\n' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: smoke-t12-key' -d "$BODY" "$BASE/items/$ITEM/jobs"
JOB=$(json "['data']['id']" < job.json)
echo "  job=${JOB}"

echo "== 4) 等待后台执行器跑完 Tripo 链（fixture 第 1 次查询 running、之后 success）"
# SQLite 是任务事实来源：直接观察阶段状态（GET /jobs/{id} 端点属 T15）。
for _ in $(seq 1 90); do
  STATUS=$(python3 - <<PY
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
row = con.execute("select status from job_stages where job_id = ? and stage_kind = 'tripo_poll'", ("$JOB",)).fetchone()
print(row[0] if row else 'missing')
PY
)
  [ "$STATUS" = "succeeded" ] && break
  [ "$STATUS" = "failed" ] && break
  sleep 1
done
echo "  tripo_poll 状态：${STATUS}"

echo "== 5) fixture 请求记录（核对目标、体与次数）"
cat tripo-fixture.log

echo "== 6) 库内事实（原始状态 + 归一化状态 + 计费 + 阶段与账本）"
python3 - <<PY
import json, sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
print('  阶段（tripo 链）：')
for kind, status, attempts, polls, usage in con.execute(
        "select stage_kind, status, attempt_count, poll_count, usage_json from job_stages "
        "where job_id = ? and stage_kind like 'tripo%' order by created_at", ("$JOB",)):
    print(f'    {kind:14s} status={status:16s} attempt_count={attempts} poll_count={polls}')
    if usage:
        data = json.loads(usage)
        if kind == 'tripo_poll':
            print('      rawStatus =', data.get('rawStatus'), '/ normalizedStatus =', data.get('normalizedStatus'))
            print('      modelUrl  =', data.get('modelUrl'))
            print('      billing   =', data.get('billing'))
        if kind == 'tripo_upload':
            print('      uploads   =', [(u['view'], u['token'], u['tokenField']) for u in data.get('uploads', [])])
        if kind == 'tripo_submit':
            print('      remoteTaskId =', data.get('remoteTaskId'))
print('  provider_attempts：')
for row in con.execute(
        "select stage_id, submit_state, remote_task_id, substr(coalesce(last_error,''),1,60) "
        "from provider_attempts order by created_at"):
    print('   ', row)
print('  cost_ledger：')
for row in con.execute('select provider, currency, reserved, actual, state from cost_ledger order by provider'):
    print('   ', row)
print('  jobs.status =', con.execute('select status from jobs where id = ?', ("$JOB",)).fetchone()[0])
print('  provider_attempts 总数 =', con.execute('select count(*) from provider_attempts').fetchone()[0],
      '（付费提交只应产生 1 条）')
PY

echo "== 7) serve 日志中的 Tripo 事件（脱敏；不含密钥/签名 URL 查询串）"
grep -o '"event":"tripo_[a-z_]*"[^}]*' serve.log | head -10
SIGNATURE_HITS=$(grep -c "smoke-signature" serve.log || true)
echo "  签名串出现在日志中的次数（应为 0）：${SIGNATURE_HITS:-0}"

echo "== 8) 服务进程连接采样（lsof；只应有 127.0.0.1）"
kill "$LSOF_PID" 2>/dev/null
wait "$LSOF_PID" 2>/dev/null
TOTAL_SAMPLES=$(wc -l < lsof-samples.txt 2>/dev/null || echo 0)
NON_LOOPBACK=$(grep "->" lsof-samples.txt 2>/dev/null | grep -v "127.0.0.1" | grep -c . || true)
echo "  采样行数=${TOTAL_SAMPLES}；非 loopback 连接行数=${NON_LOOPBACK:-0}（应为 0）"
echo "== 冒烟结束（全部请求目标只可能是 127.0.0.1；见上面 fixture 记录）"
