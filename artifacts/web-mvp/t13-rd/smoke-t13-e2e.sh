#!/bin/bash
# T13 手工端到端冒烟：**真实（测试构建）二进制** + 本机 fixture（python，仅 127.0.0.1）+ curl。
#
# 为什么是"测试构建"：模型下载的本机 fixture 放行需要"测试构建开关 + 显式测试配置"两道门
# （download.allow_local_fixture 只在 feature `job-failpoints` 的构建里生效）；发布构建
# （xtask dist）不放行任何明文 http / 回环地址。因此本脚本用
#   cargo build --features job-failpoints
# 构建的二进制，并在 serve 日志里核对一次"生产构建不放行"的编译期开关语义。
#
# 覆盖（命令 ↔ AC 见 llmdoc implementation.md §T13）：
#   建物品/资料/照片 → estimate → confirm → 建单 → 后台执行器真实跑链：
#   上传 → 付费提交（只发一次）→ 查询（success + 过期链接）→ **下载被 403 拒绝** →
#   **重新查询已知任务取新链接**（付费提交计数不增加）→ 下载 → GLB 校验 →
#   **不可变 model_revision（validated）**；并核对 CDN 请求不带 Authorization、
#   临时签名 URL 不作为永久地址保存。
# 结束时清理临时目录；**全程零真实外网调用**（fixture 只绑定 127.0.0.1）。
set -uo pipefail
BIN="${1:?用法: smoke-t13-e2e.sh <test-build-binary>}"
REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
FIXTURES="$REPO/tests/fixtures/assets"
FIXTURE_PY="$REPO/artifacts/web-mvp/t13-rd/model-fixture.py"
WORK="$(mktemp -d /tmp/em-t13-smoke-XXXXXX)"
APP_PORT=18321
FIXTURE_PORT=18322

cd "$WORK" || exit 1
cleanup() {
  [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
  [ -n "${FIXTURE_PID:-}" ] && kill "$FIXTURE_PID" 2>/dev/null
  wait "${SERVER_PID:-}" 2>/dev/null
  wait "${FIXTURE_PID:-}" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

printf 'smoke-password-t13-7b21e4\n' > pw.txt && chmod 600 pw.txt
printf 'a%.0s' $(seq 1 3000) > page-text.txt

json() { python3 -c "import json,sys;print(json.load(sys.stdin)$1)"; }

echo "== 二进制：$BIN"
echo "== 工作目录：${WORK}（结束时删除）"
echo "== 启动本机 fixture（127.0.0.1:${FIXTURE_PORT}；只在回环监听）"
python3 "$FIXTURE_PY" --port "$FIXTURE_PORT" --log "$WORK/fixture.jsonl" \
  --glb "$FIXTURES/sample-model.glb" > fixture.log 2>&1 &
FIXTURE_PID=$!
READY=0
for _ in $(seq 1 100); do
  grep -q "listening" fixture.log 2>/dev/null && { READY=1; break; }
  sleep 0.1
done
[ "$READY" = 1 ] || { echo "fixture 未在 10s 内监听："; cat fixture.log; exit 1; }
grep "listening" fixture.log

echo "== 价格目录（Tripo 30 credits）"
cp "$REPO/price-catalog.example.toml" ./prices.toml

echo "== init / serve（tripo.base_url 指向本机 fixture；下载允许域 = 127.0.0.1）"
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
# 测试配置：仅在本机 fixture 冒烟里允许明文 http + 回环地址（生产构建忽略该键）。
allowed_hosts = ["127.0.0.1"]
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
grep -o '"event":"provider_handlers_registered"[^}]*' serve.log | head -1

# 外呼采样：服务进程的 TCP 连接快照（只应有 127.0.0.1）。
(
  while kill -0 "$SERVER_PID" 2>/dev/null; do
    lsof -p "$SERVER_PID" -a -i -P -n 2>/dev/null >> "$WORK/lsof-samples.txt"
    sleep 0.2
  done
) &
LSOF_PID=$!

echo "== 登录"
curl -s -c cookies.txt -H 'content-type: application/json' \
  -d '{"password":"smoke-password-t13-7b21e4"}' "$BASE/auth/login" -o login.json
CSRF=$(json "['data']['csrfToken']" < login.json)
AUTH=(-b cookies.txt -H "x-csrf-token: $CSRF")

echo "== 1) 物品 + 说明书 + 准备（2 页文字页）"
curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"T13 冒烟相机","model":"SK-T13"}' "$BASE/items" -o item.json
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
  -H 'idempotency-key: smoke-t13-key' -d "$BODY" "$BASE/items/$ITEM/jobs"
JOB=$(json "['data']['id']" < job.json)
echo "  job=${JOB}"

echo "== 4) 等待后台执行器跑完 下载 → 校验（fixture：链接先过期 → 重查 → 成功）"
for _ in $(seq 1 90); do
  STATUS=$(python3 - <<PY
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
row = con.execute("select status from job_stages where job_id = ? and stage_kind = 'model_validate'", ("$JOB",)).fetchone()
print(row[0] if row else 'missing')
PY
)
  [ "$STATUS" = "succeeded" ] && break
  [ "$STATUS" = "failed" ] && break
  [ "$STATUS" = "needs_input" ] && break
  sleep 1
done
echo "  model_validate 状态：${STATUS}"

echo "== 5) fixture 请求记录（付费提交次数、CDN 是否带 Authorization、链接刷新）"
cat fixture.jsonl
SUBMITS=$(grep -c '"note": "paid-submit"' fixture.jsonl || true)
CDN_AUTH=$(grep '"model-download-fresh"' fixture.jsonl | grep -c '"authorization": true' || true)
echo "  付费提交次数（应为 1）：${SUBMITS}"
echo "  CDN 请求带 Authorization 的次数（应为 0）：${CDN_AUTH}"

echo "== 6) 库内事实（下载/校验阶段、asset、不可变 revision、临时 URL 不落地）"
python3 - <<PY
import json, sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
print('  阶段（T13 链）：')
for kind, status, result_asset, usage in con.execute(
        "select stage_kind, status, result_asset_id, usage_json from job_stages "
        "where job_id = ? and stage_kind in ('tripo_upload','tripo_submit','tripo_poll','model_download','model_validate') "
        "order by created_at", ("$JOB",)):
    print(f'    {kind:16s} status={status:12s} result_asset={result_asset}')
    if usage:
        data = json.loads(usage)
        if kind == 'model_download':
            print('      sha256 =', data.get('sha256'), ' sizeBytes =', data.get('sizeBytes'),
                  ' linkRefreshed =', data.get('linkRefreshed'))
        if kind == 'model_validate':
            print('      validation =', data.get('validation'), ' triangles =', data.get('triangles'),
                  ' modelRevisionId =', data.get('modelRevisionId'))
            print('      bounds =', data.get('bounds'))
print('  assets（purpose=model）：')
for row in con.execute("select id, blob_id, item_id, purpose, original_name from assets where purpose = 'model'"):
    print('   ', row)
print('  model_revisions（不可变版本）：')
for row in con.execute("select id, sha256, asset_id, validation_state, substr(bounds,1,80), provider_attempt_id is not null from model_revisions"):
    print('   ', row)
print('  blobs（model/gltf-binary）：')
for row in con.execute("select sha256, size, mime from blobs where mime = 'model/gltf-binary'"):
    print('   ', row)
print('  provider_attempts 总数 =', con.execute('select count(*) from provider_attempts').fetchone()[0],
      '（付费提交只应产生 1 条）')
print('  jobs.status =', con.execute('select status from jobs where id = ?', ("$JOB",)).fetchone()[0])
# 临时签名 URL 不得作为永久地址保存（assets / model_revisions / 下载与校验阶段 usage）
haystack = []
for table, column in [('assets','original_name'), ('model_revisions','bounds')]:
    haystack += [str(r[0]) for r in con.execute(f'select {column} from {table}')]
for (usage,) in con.execute("select usage_json from job_stages where job_id = ? and stage_kind in ('model_download','model_validate')", ("$JOB",)):
    haystack.append(usage or '')
print('  签名 URL/查询串出现在 assets/model_revisions/下载校验 usage 的次数（应为 0）：',
      sum(('sign=' in s or 'http' in s) for s in haystack))
PY

echo "== 7) serve 日志中的 T13 事件（脱敏；不含密钥与签名 URL 查询串）"
grep -o '"event":"model_[a-z_]*"[^}]*' serve.log | head -8
SIGNATURE_HITS=$(grep -c "sign=expired-old\|sign=fresh-new" serve.log || true)
echo "  签名串出现在日志中的次数（应为 0）：${SIGNATURE_HITS:-0}"

echo "== 8) 磁盘产物（blobs / tmp）"
find ./data/blobs -type f | sed 's|^|  blob: |'
echo "  tmp 残留 .part 文件数（应为 0）：$(find ./data/tmp -name '*.part' | wc -l | tr -d ' ')"

echo "== 9) 服务进程连接采样（lsof；只应有 127.0.0.1）"
kill "$LSOF_PID" 2>/dev/null
wait "$LSOF_PID" 2>/dev/null
TOTAL_SAMPLES=$(wc -l < lsof-samples.txt 2>/dev/null || echo 0)
NON_LOOPBACK=$(grep "->" lsof-samples.txt 2>/dev/null | grep -v "127.0.0.1" | grep -c . || true)
echo "  采样行数=${TOTAL_SAMPLES}；非 loopback 连接行数=${NON_LOOPBACK:-0}（应为 0）"
echo "== 冒烟结束（全部请求目标只可能是 127.0.0.1；见 fixture.jsonl）"
