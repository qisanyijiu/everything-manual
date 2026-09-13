#!/bin/bash
# T13 补充证据：**发布（dist）构建不放行本机 fixture**。
#
# 语义（见 crates/server/src/assets/glb/download.rs 模块文档）：本机 fixture 放行需要
# "显式测试配置 + 测试构建开关"两道门。本脚本用 `xtask dist` 产出的**发布二进制**，
# 配置 `[download] allow_local_fixture = true`（即使运维误配），并让任务真实走到
# model_download 阶段，核对：
#   1) serve 日志出现 `download_local_fixture_ignored`（该键在生产构建中不生效）；
#   2) model_download 阶段为 needs_input（download_insecure_scheme）而不是下载成功；
#   3) fixture 记录里**没有任何**模型 CDN 请求（连不上、连也不连）；
#   4) 付费提交仍只有 1 次（缺配置不会触发重新购买）。
# 全程零真实外网调用（fixture 只绑定 127.0.0.1）。
set -uo pipefail
BIN="${1:?用法: verify-release-gate.sh <release-binary>}"
REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
FIXTURES="$REPO/tests/fixtures/assets"
FIXTURE_PY="$REPO/artifacts/web-mvp/t13-rd/model-fixture.py"
WORK="$(mktemp -d /tmp/em-t13-gate-XXXXXX)"
APP_PORT=18331
FIXTURE_PORT=18332

cd "$WORK" || exit 1
cleanup() {
  [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
  [ -n "${FIXTURE_PID:-}" ] && kill "$FIXTURE_PID" 2>/dev/null
  wait "${SERVER_PID:-}" 2>/dev/null
  wait "${FIXTURE_PID:-}" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

printf 'gate-password-t13-51ab\n' > pw.txt && chmod 600 pw.txt
printf 'a%.0s' $(seq 1 3000) > page-text.txt
json() { python3 -c "import json,sys;print(json.load(sys.stdin)$1)"; }

echo "== 二进制（发布构建）：$BIN"
python3 "$FIXTURE_PY" --port "$FIXTURE_PORT" --log "$WORK/fixture.jsonl" \
  --glb "$FIXTURES/sample-model.glb" > fixture.log 2>&1 &
FIXTURE_PID=$!
for _ in $(seq 1 100); do
  grep -q "listening" fixture.log 2>/dev/null && break
  sleep 0.1
done
grep "listening" fixture.log

cp "$REPO/price-catalog.example.toml" ./prices.toml
"$BIN" init --data-dir ./data --password-file ./pw.txt 2>&1 | tail -1
cat > ./data/config.toml <<TOML
price_catalog_path = "./prices.toml"

[providers.tripo]
base_url = "http://127.0.0.1:${FIXTURE_PORT}/v3"
api_key_env = "SMOKE_TRIPO_KEY"

[providers.manual_ai]
model = "gpt-5-mini"
api_key_env = "SMOKE_MANUAL_AI_KEY"

[download]
allowed_hosts = ["127.0.0.1"]
# 运维误配：发布构建必须忽略这个键（不放行明文 http/回环）。
allow_local_fixture = true
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

echo "== 0) 编译期开关语义（发布构建必须忽略 allow_local_fixture）"
grep -o '"event":"download_local_fixture_ignored"[^}]*' serve.log | head -1

curl -s -c cookies.txt -H 'content-type: application/json' \
  -d '{"password":"gate-password-t13-51ab"}' "$BASE/auth/login" -o login.json
CSRF=$(json "['data']['csrfToken']" < login.json)
AUTH=(-b cookies.txt -H "x-csrf-token: $CSRF")

ITEM=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"T13 门禁用例","model":"SK-GATE"}' "$BASE/items" | json "['data']['id']")
upload() { curl -s "${AUTH[@]}" -F "purpose=$2" -F "file=@$1" "$BASE/items/$ITEM/assets"; }
DOC_ASSET_JSON=$(upload "$FIXTURES/sample-manual-text.pdf" document)
DOC_ASSET=$(echo "$DOC_ASSET_JSON" | json "['data']['id']")
PDF_SHA=$(echo "$DOC_ASSET_JSON" | json "['data']['sha256']")
DOC=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceAssetId\":\"$DOC_ASSET\",\"title\":\"门禁\"}" \
  "$BASE/items/$ITEM/documents" | json "['data']['id']")
PREP=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceSha256\":\"$PDF_SHA\"}" "$BASE/documents/$DOC/preparations" | json "['data']['id']")
for page in 1 2; do
  PAGE_TEXT=$(upload ./page-text.txt pageText | json "['data']['id']")
  PAGE_IMAGE=$(upload "$FIXTURES/sample-photo-front.jpg" pageImage | json "['data']['id']")
  curl -s -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
    -d "{\"textAssetId\":\"$PAGE_TEXT\",\"imageAssetId\":\"$PAGE_IMAGE\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
    "$BASE/preparations/$PREP/pages/$page" -o /dev/null
done
ETAG=$(curl -s "${AUTH[@]}" -D - -o /dev/null "$BASE/preparations/$PREP" \
  | awk -F'"' 'tolower($1) ~ /^etag:/ {print $2}')
curl -s "${AUTH[@]}" -H 'content-type: application/json' -H "if-match: \"$ETAG\"" \
  -d '{"pageCount":2}' "$BASE/preparations/$PREP/complete" -o /dev/null
FRONT_ASSET=$(upload "$FIXTURES/sample-photo-front.jpg" photo | json "['data']['id']")
LEFT_ASSET=$(upload "$FIXTURES/sample-photo-left.png" photo | json "['data']['id']")
FRONT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$FRONT_ASSET\",\"view\":\"front\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
LEFT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$LEFT_ASSET\",\"view\":\"left\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
QUOTE=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"modelPreset\":\"tripo-h-v3.1-standard\"}" \
  "$BASE/items/$ITEM/estimates" | json "['data']['id']")
curl -s -o /dev/null -X POST "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$QUOTE/confirm"
BODY="{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3000,\"manualAiUsdMicros\":50000}}"
JOB=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' -H 'idempotency-key: smoke-t13-gate' \
  -d "$BODY" "$BASE/items/$ITEM/jobs" | json "['data']['id']")
echo "== job=${JOB}（等待 model_download 结论）"
for _ in $(seq 1 90); do
  STATUS=$(python3 - <<PY
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
row = con.execute("select status from job_stages where job_id = ? and stage_kind = 'model_download'", ("$JOB",)).fetchone()
print(row[0] if row else 'missing')
PY
)
  [ "$STATUS" = "needs_input" ] && break
  [ "$STATUS" = "failed" ] && break
  [ "$STATUS" = "succeeded" ] && break
  sleep 1
done
echo "  model_download 状态：${STATUS}（期望 needs_input；发布构建不放行本机 fixture）"

echo "== 阶段缺项与 fixture 记录"
python3 - <<PY
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
row = con.execute("select needs_input_json, last_error from job_stages where job_id = ? and stage_kind = 'model_download'", ("$JOB",)).fetchone()
print('  needs_input =', row[0])
print('  last_error  =', row[1])
print('  model 资产数（应为 0）=', con.execute("select count(*) from assets where purpose = 'model'").fetchone()[0])
print('  model_revisions 数（应为 0）=', con.execute('select count(*) from model_revisions').fetchone()[0])
print('  provider_attempts（付费提交只应 1 条）=', con.execute('select count(*) from provider_attempts').fetchone()[0])
PY
HTTP_CDN=$(grep -c 'model-download' fixture.jsonl || true)
echo "  fixture 记录中的模型 CDN 请求次数（应为 0）：${HTTP_CDN}"
cat fixture.jsonl
