#!/bin/bash
# T11 手工冒烟：真实发布二进制 + 真实 data-dir + curl（原始输出存档）。
#
# 覆盖：建物品 → 传 PDF → 绑定 document → 准备（2 页：文字页 + 扫描页）→ 封存 →
#       传 front/left 照片 → 缺视图 422 → estimate（分列金额/上界/expiresAt/发送范围，
#       无费用记录）→ 未确认提交 422 → 预算低于上界 422 → 确认（audit_events）→
#       建单（首次 202）→ 同键重放 202 同一 job（x-idempotent-replay）→
#       同键不同 body 409 → 重放 20 次后仍只有 1 job / 每供应商 1 笔预留。
# 结束时清理临时目录（不自留现场）；全程只连本机 loopback，无任何真实 Provider 调用。
set -uo pipefail
BIN="${1:?用法: smoke-manual.sh <binary>}"
REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
FIXTURES="$REPO/tests/fixtures/assets"
WORK="$(mktemp -d /tmp/em-t11-smoke-XXXXXX)"
PORT=18211
cp "$BIN" "$WORK/everything-manual"
cd "$WORK" || exit 1
cleanup() {
  [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
  wait "$SERVER_PID" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT
printf 'smoke-password-t11-7c4d1a\n' > pw.txt && chmod 600 pw.txt
printf 'a%.0s' $(seq 1 3000) > page-text.txt

json() { python3 -c "import json,sys;print(json.load(sys.stdin)$1)"; }

echo "== 工作目录：${WORK}（结束时删除）"
echo "== 价格目录（version=2026-09-11；Tripo 30 credits；说明书 AI 示例单价）"
cp "$REPO/price-catalog.example.toml" ./prices.toml

echo "== init / serve（Provider 用假凭据；不会被调用）"
"$WORK/everything-manual" init --data-dir ./data --password-file ./pw.txt 2>&1 | tail -1
cat > ./data/config.toml <<'TOML'
price_catalog_path = "./prices.toml"

[providers.tripo]
api_key_env = "SMOKE_TRIPO_KEY"

[providers.manual_ai]
model = "gpt-5-mini"
api_key_env = "SMOKE_MANUAL_AI_KEY"
TOML
SMOKE_TRIPO_KEY="fake-tripo-key-not-used" SMOKE_MANUAL_AI_KEY="fake-manual-ai-key-not-used" \
  "$WORK/everything-manual" serve --data-dir ./data --listen 127.0.0.1:${PORT} > serve.log 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 50); do
  curl -sf "http://127.0.0.1:${PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
BASE="http://127.0.0.1:${PORT}/api/v1"

echo "== 登录"
curl -s -c cookies.txt -H 'content-type: application/json' \
  -d '{"password":"smoke-password-t11-7c4d1a"}' "$BASE/auth/login" -o login.json
CSRF=$(json "['data']['csrfToken']" < login.json)
AUTH=(-b cookies.txt -H "x-csrf-token: $CSRF")

echo "== 1) 物品 + 说明书 + 准备（2 页：第 1 页文字，第 2 页扫描）"
curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"T11 冒烟相机","model":"SK-T11"}' "$BASE/items" -o item.json
ITEM=$(json "['data']['id']" < item.json)
echo "  item=${ITEM}"

upload() { # $1=文件 $2=purpose（stdout：curl 响应 JSON）
  curl -s "${AUTH[@]}" -F "purpose=$2" -F "file=@$1" "$BASE/items/$ITEM/assets"
}
DOC_ASSET_JSON=$(upload "$FIXTURES/sample-manual-text.pdf" document)
DOC_ASSET=$(echo "$DOC_ASSET_JSON" | json "['data']['id']")
PDF_SHA=$(echo "$DOC_ASSET_JSON" | json "['data']['sha256']")
DOC=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceAssetId\":\"$DOC_ASSET\",\"title\":\"冒烟说明书\"}" \
  "$BASE/items/$ITEM/documents" | json "['data']['id']")
echo "  document=${DOC} sourceSha256=${PDF_SHA:0:12}…"
PREP=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceSha256\":\"$PDF_SHA\"}" "$BASE/documents/$DOC/preparations" \
  | json "['data']['id']")
echo "  preparation=${PREP}"

PAGE_TEXT=$(upload ./page-text.txt pageText | json "['data']['id']")
PAGE_IMAGE=$(upload "$FIXTURES/sample-photo-front.jpg" pageImage | json "['data']['id']")
curl -s -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"textAssetId\":\"$PAGE_TEXT\",\"imageAssetId\":\"$PAGE_IMAGE\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
  "$BASE/preparations/$PREP/pages/1" -o page1.json
json "['data']['pageNumber']" < page1.json > /dev/null || { echo "第 1 页写入失败："; cat page1.json; exit 1; }
PAGE_IMAGE2=$(upload "$FIXTURES/sample-photo-front.jpg" pageImage | json "['data']['id']")
curl -s -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"textAssetId\":null,\"imageAssetId\":\"$PAGE_IMAGE2\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
  "$BASE/preparations/$PREP/pages/2" -o page2.json
json "['data']['pageNumber']" < page2.json > /dev/null || { echo "第 2 页写入失败："; cat page2.json; exit 1; }
ETAG=$(curl -s "${AUTH[@]}" -D - -o /dev/null "$BASE/preparations/$PREP" \
  | awk -F'"' 'tolower($1) ~ /^etag:/ {print $2}')
curl -s "${AUTH[@]}" -H 'content-type: application/json' -H "if-match: \"$ETAG\"" \
  -d '{"pageCount":2}' "$BASE/preparations/$PREP/complete" -o complete.json
python3 -c "import json;d=json.load(open('complete.json'))['data'];print('  准备状态：',d['state'],'页数：',d['pageCount'])" || {
  echo "封存失败："; cat complete.json; exit 1; }

echo "== 2) 照片：front + left"
FRONT_ASSET=$(upload "$FIXTURES/sample-photo-front.jpg" photo | json "['data']['id']")
LEFT_ASSET=$(upload "$FIXTURES/sample-photo-left.png" photo | json "['data']['id']")
FRONT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$FRONT_ASSET\",\"view\":\"front\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
LEFT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$LEFT_ASSET\",\"view\":\"left\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
echo "  front=${FRONT_ID} left=${LEFT_ID}"

echo "== 3) 缺侧视图 → 422 明细"
curl -s -o precond.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\"],\"modelPreset\":\"tripo-h-v3.1-standard\"}" \
  "$BASE/items/$ITEM/estimates"
python3 -c "
import json;d=json.load(open('precond.json'))['error']
print('  code=',d['code'],'reason=',d['details']['reason'],'items=',[i['code'] for i in d['details']['items']])"

echo "== 4) estimate（只计算计划；无费用记录）"
curl -s -D estimate-headers.txt -o estimate.json "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"modelPreset\":\"tripo-h-v3.1-standard\"}" \
  "$BASE/items/$ITEM/estimates"
grep -i "^HTTP/" estimate-headers.txt
QUOTE=$(json "['data']['id']" < estimate.json)
python3 - <<'PY'
import json, sqlite3
d = json.load(open('estimate.json'))['data']
print('  quote =', d['id'])
print('  分列：Tripo', d['amounts']['tripo']['upperBoundDisplay'], '=',
      d['amounts']['tripo']['upperBoundMinor'], d['amounts']['tripo']['currency'])
print('        ManualAI', d['amounts']['manualAi']['upperBoundDisplay'], '=',
      d['amounts']['manualAi']['upperBoundMinor'], d['amounts']['manualAi']['currency'])
print('  expiresAt =', d['expiresAt'], ' priceVersion =', d['priceVersion'],
      ' 快照日期 =', d['priceSnapshotDate'], ' maxOutputTokens =', d['maxOutputTokens'])
print('  发送范围：views =', [v['view'] for v in d['sendScope']['tripo']['views']],
      ' 页 =', (d['sendScope']['manualAi']['pageFrom'], d['sendScope']['manualAi']['pageTo']),
      ' 文字页 =', d['sendScope']['manualAi']['textPages'],
      ' 页图页 =', d['sendScope']['manualAi']['imagePages'])
print('  预算说明：', d['budgetNotice'])
con = sqlite3.connect('./data/manual.sqlite3')
print('  DB：cost_ledger =', con.execute('select count(*) from cost_ledger').fetchone()[0],
      ' jobs =', con.execute('select count(*) from jobs').fetchone()[0],
      ' provider_attempts =', con.execute('select count(*) from provider_attempts').fetchone()[0],
      ' quotes =', con.execute('select count(*) from quotes').fetchone()[0])
PY

echo "== 5) 未确认提交 → 422（不创建 job）"
curl -s -o unconfirmed.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: smoke-unconfirmed' \
  -d "{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3000,\"manualAiUsdMicros\":50000}}" \
  "$BASE/items/$ITEM/jobs"
python3 -c "import json;print('  reason=',json.load(open('unconfirmed.json'))['error']['details']['reason'])"

echo "== 7) 确认发送范围（audit_events）"
curl -s -o confirm.json -w '  HTTP %{http_code}\n' -X POST "${AUTH[@]}" \
  "$BASE/items/$ITEM/estimates/$QUOTE/confirm"
python3 -c "import json;d=json.load(open('confirm.json'))['data'];print('  confirmedAt=',d['confirmedAt'],' summary=',d['summary'])"

echo "== 7b) 预算低于服务端上界 → 422（已确认后单独校验预算）"
curl -s -o budget.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: smoke-budget' \
  -d "{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":1,\"manualAiUsdMicros\":1}}" \
  "$BASE/items/$ITEM/jobs"
python3 -c "import json;d=json.load(open('budget.json'))['error'];print('  code=',d['code'],'reason=',d['details']['reason'])"

echo "== 8) 建单（首次 202）→ 同键重放（同一 job）→ 同键不同 body（409）"
BODY="{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3000,\"manualAiUsdMicros\":50000}}"
curl -s -D job1-headers.txt -o job1.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: smoke-t11-key' -d "$BODY" "$BASE/items/$ITEM/jobs"
grep -i "^HTTP/" job1-headers.txt
JOB=$(json "['data']['id']" < job1.json)
python3 -c "
import json;d=json.load(open('job1.json'))['data']
print('  job=',d['id'],' status=',d['status'],' snapshot=',d['snapshotId'])
print('  预留：',[(r['provider'],r['currency'],r['reservedMinor'],r['state']) for r in d['reservations']])"
curl -s -D job2-headers.txt -o job2.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: smoke-t11-key' -d "$BODY" "$BASE/items/$ITEM/jobs"
grep -i "^x-idempotent-replay" job2-headers.txt || echo "  （缺 x-idempotent-replay）"
JOB2=$(json "['data']['id']" < job2.json)
[ "$JOB" = "$JOB2" ] && echo "  重放返回同一 job：OK"
curl -s -o conflict.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: smoke-t11-key' \
  -d "${BODY/\"tripoCreditMinor\":3000/\"tripoCreditMinor\":4000}" "$BASE/items/$ITEM/jobs"
python3 -c "import json;d=json.load(open('conflict.json'))['error'];print('  同键不同 body：code=',d['code'],'reason=',d['details']['reason'])"

echo "== 9) 库内不变量（重放 20 次后仍只有 1 job / 每供应商 1 笔预留）"
for _ in $(seq 1 18); do
  curl -s -o /dev/null "${AUTH[@]}" -H 'content-type: application/json' \
    -H 'idempotency-key: smoke-t11-key' -d "$BODY" "$BASE/items/$ITEM/jobs"
done
python3 - <<'PY'
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
print('  quotes =', con.execute('select count(*) from quotes').fetchone()[0],
      ' 已消费 =', con.execute('select count(*) from quotes where consumed_job_id is not null').fetchone()[0])
print('  jobs =', con.execute('select count(*) from jobs').fetchone()[0],
      ' snapshots =', con.execute('select count(*) from generation_snapshots').fetchone()[0],
      ' cost_ledger =', con.execute('select count(*) from cost_ledger').fetchone()[0])
for row in con.execute('select provider, currency, reserved, actual, state from cost_ledger order by provider'):
    print('   ledger 行：', row)
print('  audit_events =', [r[0] for r in con.execute('select action from audit_events order by created_at')])
print('  幂等记录 =', con.execute('select count(*) from idempotency_records').fetchone()[0])
print('  快照冻结（photo_ids, photo_hashes）=', con.execute(
    'select photo_ids, photo_hashes from generation_snapshots').fetchone())
print('  provider_attempts =', con.execute('select count(*) from provider_attempts').fetchone()[0],
      '（无付费提交）')
PY

echo "== 冒烟完成（临时目录将在退出时删除）"
