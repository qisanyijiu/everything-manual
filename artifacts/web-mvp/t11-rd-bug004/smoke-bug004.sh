#!/bin/bash
# BUG-004 修复手工冒烟：真实发布二进制 + 真实 data-dir。
#
# 覆盖（与缺陷复现同序）：建报价 → 回读（三字段 null）→ 确认 → 回读（confirmedAt 有值）
#   → 建单 → 回读（consumedAt/consumedJobId = 真实 job）→ 与 DB 三列逐值比对。
# 结束后清理临时目录；只连本机 loopback，无任何真实 Provider 调用。
set -uo pipefail
BIN="${1:?用法: smoke-bug004.sh <binary>}"
REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
FIXTURES="$REPO/tests/fixtures/assets"
WORK="$(mktemp -d /tmp/em-t11-bug004-XXXXXX)"
PORT=18212
cp "$BIN" "$WORK/everything-manual"
cd "$WORK" || exit 1
cleanup() {
  [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
  wait "${SERVER_PID:-}" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT
printf 'smoke-password-bug004-9e2f\n' > pw.txt && chmod 600 pw.txt
printf 'a%.0s' $(seq 1 3000) > page-text.txt

json() { python3 -c "import json,sys;print(json.load(sys.stdin)$1)"; }

echo "== init / serve（Provider 假凭据；不会被调用）"
cp "$REPO/price-catalog.example.toml" ./prices.toml
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

curl -s -c cookies.txt -H 'content-type: application/json' \
  -d '{"password":"smoke-password-bug004-9e2f"}' "$BASE/auth/login" -o login.json
CSRF=$(json "['data']['csrfToken']" < login.json)
AUTH=(-b cookies.txt -H "x-csrf-token: $CSRF")

echo "== 物品 + 准备（1 页文字 + 1 页扫描）+ front/left 照片"
curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"BUG-004 冒烟相机","model":"SK-B4"}' "$BASE/items" -o item.json
ITEM=$(json "['data']['id']" < item.json)
upload() { curl -s "${AUTH[@]}" -F "purpose=$2" -F "file=@$1" "$BASE/items/$ITEM/assets"; }
DOC_ASSET_JSON=$(upload "$FIXTURES/sample-manual-text.pdf" document)
DOC_ASSET=$(echo "$DOC_ASSET_JSON" | json "['data']['id']")
PDF_SHA=$(echo "$DOC_ASSET_JSON" | json "['data']['sha256']")
DOC=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceAssetId\":\"$DOC_ASSET\",\"title\":\"冒烟说明书\"}" \
  "$BASE/items/$ITEM/documents" | json "['data']['id']")
PREP=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceSha256\":\"$PDF_SHA\"}" "$BASE/documents/$DOC/preparations" | json "['data']['id']")
PAGE_TEXT=$(upload ./page-text.txt pageText | json "['data']['id']")
PAGE_IMAGE=$(upload "$FIXTURES/sample-photo-front.jpg" pageImage | json "['data']['id']")
curl -s -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"textAssetId\":\"$PAGE_TEXT\",\"imageAssetId\":\"$PAGE_IMAGE\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
  "$BASE/preparations/$PREP/pages/1" -o /dev/null
PAGE_IMAGE2=$(upload "$FIXTURES/sample-photo-front.jpg" pageImage | json "['data']['id']")
curl -s -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"textAssetId\":null,\"imageAssetId\":\"$PAGE_IMAGE2\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
  "$BASE/preparations/$PREP/pages/2" -o /dev/null
ETAG=$(curl -s "${AUTH[@]}" -D - -o /dev/null "$BASE/preparations/$PREP" \
  | awk -F'"' 'tolower($1) ~ /^etag:/ {print $2}')
curl -s "${AUTH[@]}" -H 'content-type: application/json' -H "if-match: \"$ETAG\"" \
  -d '{"pageCount":2}' "$BASE/preparations/$PREP/complete" -o complete.json
python3 -c "import json;d=json.load(open('complete.json'))['data'];print('  准备状态：',d['state'],'页数：',d['pageCount'])" || {
  echo "封存失败："; cat complete.json; exit 1; }
FRONT_ASSET=$(upload "$FIXTURES/sample-photo-front.jpg" photo | json "['data']['id']")
LEFT_ASSET=$(upload "$FIXTURES/sample-photo-left.png" photo | json "['data']['id']")
FRONT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$FRONT_ASSET\",\"view\":\"front\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
LEFT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$LEFT_ASSET\",\"view\":\"left\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")

echo "== 1) 建报价（201）"
curl -s -o estimate.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"modelPreset\":\"tripo-h-v3.1-standard\"}" \
  "$BASE/items/$ITEM/estimates"
QUOTE=$(json "['data']['id']" < estimate.json)

echo "== 2) 回读（未确认未消费）"
curl -s -o get-before.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$QUOTE"
python3 -c "
import json;d=json.load(open('get-before.json'))['data']
print('  confirmedAt =',repr(d['confirmedAt']),' consumedAt =',repr(d['consumedAt']),' consumedJobId =',repr(d['consumedJobId']))"

echo "== 3) 确认（200）"
curl -s -o confirm.json -w '  HTTP %{http_code}\n' -X POST "${AUTH[@]}" \
  "$BASE/items/$ITEM/estimates/$QUOTE/confirm"
CONFIRMED_AT=$(json "['data']['confirmedAt']" < confirm.json)
echo "  确认响应 confirmedAt = $CONFIRMED_AT"

echo "== 4) 回读（已确认未消费）：confirmedAt 必须等于确认响应；消费仍为 null"
curl -s -o get-confirmed.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$QUOTE"
python3 - <<PY || { echo "  断言失败：回读未反映确认事实"; exit 1; }
import json
d = json.load(open('get-confirmed.json'))['data']
print('  confirmedAt =', repr(d['confirmedAt']), ' consumedAt =', repr(d['consumedAt']), ' consumedJobId =', repr(d['consumedJobId']))
assert d['confirmedAt'] == "$CONFIRMED_AT", '回读 confirmedAt 必须等于确认响应'
print('  与确认响应一致：OK')
PY

echo "== 5) 建单（202）"
BODY="{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3000,\"manualAiUsdMicros\":50000}}"
curl -s -o job.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: bug004-smoke-key' -d "$BODY" "$BASE/items/$ITEM/jobs"
JOB=$(json "['data']['id']" < job.json)
echo "  job = $JOB"

echo "== 6) 回读（已消费）：consumedAt/consumedJobId 必须反映真实 job"
curl -s -o get-consumed.json -w '  HTTP %{http_code}\n' "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$QUOTE"
python3 - <<PY || { echo "  断言失败：回读未反映消费事实"; exit 1; }
import json
d = json.load(open('get-consumed.json'))['data']
print('  confirmedAt =', repr(d['confirmedAt']), ' consumedAt =', repr(d['consumedAt']), ' consumedJobId =', repr(d['consumedJobId']))
assert d['confirmedAt'] == "$CONFIRMED_AT", '消费不改变确认时间'
assert d['consumedJobId'] == "$JOB", '回读 consumedJobId 必须等于真实 job'
assert isinstance(d['consumedAt'], str), '回读 consumedAt 必须是时间字符串'
print('  与真实 job 一致：OK')
PY

echo "== 7) 与 DB 三列逐值比对（quotes 表事实）"
python3 - <<'PY'
import json, sqlite3
d = json.load(open('get-consumed.json'))['data']
con = sqlite3.connect('./data/manual.sqlite3')
row = con.execute('select confirmed_at, consumed_at, consumed_job_id from quotes').fetchone()
print('  DB 原始毫秒：confirmed_at =', row[0], ' consumed_at =', row[1], ' consumed_job_id =', row[2])
print('  GET 回读    ：confirmedAt =', d['confirmedAt'], ' consumedAt =', d['consumedAt'], ' consumedJobId =', d['consumedJobId'])
print('  （confirmed/consumed 时间戳格式为 RFC3339 毫秒，与 DB 毫秒值同源）')
print('  jobs =', con.execute('select count(*) from jobs').fetchone()[0],
      ' provider_attempts =', con.execute('select count(*) from provider_attempts').fetchone()[0])
PY

echo "== 冒烟完成（临时目录将在退出时删除）"
