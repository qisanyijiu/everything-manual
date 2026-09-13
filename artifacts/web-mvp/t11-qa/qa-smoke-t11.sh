#!/bin/bash
# T11 QA 独立二进制冒烟（回合 12）：只用 curl + sqlite3 观察真实服务端行为。
# 与 RD 脚本独立：另一端口、另一套数据、另加"非法价格目录必须拒绝启动"与
# HTTP 层 20 次重放/未知字段拒绝；结束时清理临时目录与进程。
set -uo pipefail
BIN="${1:?用法: qa-smoke-t11.sh <binary 绝对路径>}"
REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
FIX="$REPO/tests/fixtures/assets"
WORK="$(mktemp -d /tmp/em-t11-qa-XXXXXX)"
PORT=18331
cp "$BIN" "$WORK/everything-manual"
cd "$WORK" || exit 1
cleanup() {
  [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
  [ -n "${SERVER_PID:-}" ] && wait "$SERVER_PID" 2>/dev/null
  chmod -R u+w "$WORK" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT
json() { python3 -c "import json,sys;print(json.load(sys.stdin)$1)"; }
echo "== QA 工作目录：${WORK}（退出时删除）；二进制：${BIN}"

printf 'qa-smoke-password-11a2\n' > pw.txt && chmod 600 pw.txt
printf 'b%.0s' $(seq 1 2000) > page-text.txt
cp "$REPO/price-catalog.example.toml" ./prices.toml
cp "$REPO/price-catalog.example.toml" ./prices-bad.toml
printf 'credits = "0.005"\n' >> ./prices-bad.toml   # 重复键 → 目录非法

echo "== 1) 非法价格目录：check 必须非零（不静默用猜测价格）"
"$WORK/everything-manual" init --data-dir ./data --password-file ./pw.txt >/dev/null 2>&1
printf 'price_catalog_path = "./prices-bad.toml"\n' > ./data/config.toml
"$WORK/everything-manual" check --data-dir ./data > check-bad.log 2>&1
BAD_CHECK=$?
echo "  check 退出码 = ${BAD_CHECK}（期望非 0）"; tail -2 check-bad.log
"$WORK/everything-manual" serve --data-dir ./data --listen 127.0.0.1:${PORT} > serve-bad.log 2>&1
BAD_SERVE=$?
echo "  serve 退出码 = ${BAD_SERVE}（期望 3=配置错误）"

echo "== 2) 正常启动（假凭据，只连 loopback）"
cat > ./data/config.toml <<'TOML'
price_catalog_path = "./prices.toml"

[providers.tripo]
api_key_env = "QA_SMOKE_TRIPO_KEY"

[providers.manual_ai]
model = "gpt-5-mini"
api_key_env = "QA_SMOKE_MANUAL_KEY"
TOML
"$WORK/everything-manual" check --data-dir ./data > check-good.log 2>&1
echo "  check 退出码 = $?（期望 0）；版本行：$(grep -iE 'schema|库 v' check-good.log | head -2)"
start_server() {
  QA_SMOKE_TRIPO_KEY="fake-not-used" QA_SMOKE_MANUAL_KEY="fake-not-used" \
    "$WORK/everything-manual" serve --data-dir ./data --listen 127.0.0.1:${PORT} >> serve.log 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 80); do
    curl -sf "http://127.0.0.1:${PORT}/api/v1/health/ready" >/dev/null && return 0
    sleep 0.1
  done
  echo "  服务未就绪"; exit 1
}
stop_server() {
  kill "$SERVER_PID" 2>/dev/null
  [ -n "${SERVER_PID:-}" ] && wait "$SERVER_PID" 2>/dev/null
  SERVER_PID=""
}
start_server
BASE="http://127.0.0.1:${PORT}/api/v1"
curl -s -c cookies.txt -H 'content-type: application/json' \
  -d '{"password":"qa-smoke-password-11a2"}' "$BASE/auth/login" -o login.json
CSRF=$(json "['data']['csrfToken']" < login.json)
AUTH=(-b cookies.txt -H "x-csrf-token: $CSRF")

echo "== 3) 物品 / PDF / 2 页准备（1 文字页 + 1 扫描页）/ front+left 照片"
ITEM=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"QA 独立冒烟物品","model":"QA-SMOKE-11"}' "$BASE/items" | json "['data']['id']")
upload() { curl -s "${AUTH[@]}" -F "purpose=$2" -F "file=@$1" "$BASE/items/$ITEM/assets"; }
DOC_ASSET_JSON=$(upload "$FIX/sample-manual-text.pdf" document)
DOC_ASSET=$(echo "$DOC_ASSET_JSON" | json "['data']['id']")
PDF_SHA=$(echo "$DOC_ASSET_JSON" | json "['data']['sha256']")
DOC=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceAssetId\":\"$DOC_ASSET\",\"title\":\"QA 冒烟说明书\"}" "$BASE/items/$ITEM/documents" | json "['data']['id']")
PREP=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceSha256\":\"$PDF_SHA\"}" "$BASE/documents/$DOC/preparations" | json "['data']['id']")
P1T=$(upload ./page-text.txt pageText | json "['data']['id']")
P1I=$(upload "$FIX/sample-photo-front.jpg" pageImage | json "['data']['id']")
PC=$(curl -s -o /dev/null -w '%{http_code}' -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"textAssetId\":\"$P1T\",\"imageAssetId\":\"$P1I\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
  "$BASE/preparations/$PREP/pages/1")
echo "  第 1 页写入 HTTP $PC"
P2I=$(upload "$FIX/sample-photo-front.jpg" pageImage | json "['data']['id']")
PC=$(curl -s -o /dev/null -w '%{http_code}' -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"textAssetId\":null,\"imageAssetId\":\"$P2I\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
  "$BASE/preparations/$PREP/pages/2")
echo "  第 2 页写入 HTTP $PC"
ETAG=$(curl -s "${AUTH[@]}" -D - -o /dev/null "$BASE/preparations/$PREP" | awk -F'"' 'tolower($1) ~ /^etag:/ {print $2}')
PC=$(curl -s -o complete.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' -H "if-match: \"$ETAG\"" \
  -d '{"pageCount":2}' "$BASE/preparations/$PREP/complete")
echo "  准备封存 HTTP $PC"
[ "$PC" = "200" ] || { cat complete.json; echo; exit 1; }
FRONT_ASSET_JSON=$(upload "$FIX/sample-photo-front.jpg" photo)
FRONT_ASSET=$(echo "$FRONT_ASSET_JSON" | json "['data']['id']")
FRONT_SHA=$(echo "$FRONT_ASSET_JSON" | json "['data']['sha256']")
LEFT_ASSET=$(upload "$FIX/sample-photo-left.png" photo | json "['data']['id']")
FRONT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$FRONT_ASSET\",\"view\":\"front\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
LEFT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$LEFT_ASSET\",\"view\":\"left\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")

echo "== 4) estimate：QA 用 python 独立复算上界并对照响应"
EST_CODE=$(curl -s -o estimate.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"modelPreset\":\"tripo-h-v3.1-standard\"}" \
  "$BASE/items/$ITEM/estimates")
echo "  HTTP ${EST_CODE}（期望 201）"
[ "$EST_CODE" = "201" ] || { cat estimate.json; echo; exit 1; }
QUOTE=$(json "['data']['id']" < estimate.json)
python3 - "$WORK/data/manual.sqlite3" <<'PY'
import json, sys, sqlite3, math
d = json.load(open('estimate.json'))['data']
# QA 独立复算：1 批（2 页）、页 1 文字 2000 字节、页 2 页图。
input_upper = 1200 + 2000 + 3000
inp = math.ceil(input_upper * 0.25)      # 0.25 USD / 1M 输入 token
outp = math.ceil(4096 * 2.00)            # 2.00 USD / 1M 输出 token
img = 10_000                             # 0.01 USD / 张页图
expect = inp + outp + img
got = d['amounts']['manualAi']['upperBoundMinor']
print(f"  QA 手算 ManualAI 上界={expect}（输入 {inp} + 输出 {outp} + 页图 {img}）；响应={got}")
assert got == expect, "上界与 QA 手算不一致"
assert d['amounts']['tripo']['upperBoundMinor'] == 3000
assert d['amounts']['tripo']['currency'] == 'creditMinor'
assert d['amounts']['manualAi']['currency'] == 'usdMicros'
con = sqlite3.connect(sys.argv[1])
print('  DB：cost_ledger =', con.execute('select count(*) from cost_ledger').fetchone()[0],
      ' jobs =', con.execute('select count(*) from jobs').fetchone()[0],
      ' attempts =', con.execute('select count(*) from provider_attempts').fetchone()[0],
      ' quotes =', con.execute('select count(*) from quotes').fetchone()[0])
PY

echo "== 5) 未确认提交 → 422；确认 → 200（audit）"
BODY="{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3000,\"manualAiUsdMicros\":50000}}"
C1=$(curl -s -o unconfirmed.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: qa-smoke-unconfirmed' -d "$BODY" "$BASE/items/$ITEM/jobs")
echo "  未确认 HTTP $C1 reason=$(json "['error']['details']['reason']" < unconfirmed.json)"
C2=$(curl -s -o confirm.json -w '%{http_code}' -X POST "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$QUOTE/confirm")
echo "  确认 HTTP $C2 at=$(json "['data']['confirmedAt']" < confirm.json)"

echo "== 6) 未知字段（前端费用/质量）→ 422（结构性拒绝）"
C3=$(curl -s -o unknown-field.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: qa-smoke-unknown' \
  -d "{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3000,\"manualAiUsdMicros\":50000},\"tripoCreditMinor\":1}" \
  "$BASE/items/$ITEM/jobs")
echo "  未知字段 HTTP $C3 code=$(json "['error']['code']" < unknown-field.json)"

echo "== 7) 20 次同键重放（HTTP）"
FIRST=""
for i in $(seq 1 20); do
  H=$(curl -s -D - -o job-$i.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
    -H 'idempotency-key: qa-smoke-replay' -d "$BODY" "$BASE/items/$ITEM/jobs")
  JID=$(json "['data']['id']" < job-$i.json)
  [ -z "$FIRST" ] && FIRST="$JID"
  if [ "$i" -eq 1 ]; then
    echo "  第 1 次 HTTP $H job=${JID}（无重放头：$(echo "$H" | grep -c . )）"
  elif [ "$i" -eq 2 ]; then
    echo "  第 2 次 HTTP $H 重放头=$(echo "$H" | head -1)"
  fi
  [ "$JID" = "$FIRST" ] || { echo "  第 $i 次 job id 变化：$JID != $FIRST"; exit 1; }
done
echo "  20 次全部返回同一 job id：$FIRST"
C4=$(curl -s -o conflict.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: qa-smoke-replay' \
  -d "{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3001,\"manualAiUsdMicros\":50000}}" \
  "$BASE/items/$ITEM/jobs")
echo "  同键不同 body HTTP $C4 code=$(json "['error']['code']" < conflict.json)"

echo "== 8) 库内不变量"
python3 - "$WORK/data/manual.sqlite3" "$FRONT_SHA" <<'PY'
import sqlite3, sys, json
con = sqlite3.connect(sys.argv[1])
q = lambda s: con.execute(s).fetchone()[0]
print('  jobs =', q('select count(*) from jobs'), ' snapshots =', q('select count(*) from generation_snapshots'),
      ' cost_ledger =', q('select count(*) from cost_ledger'), ' 幂等记录 =', q('select count(*) from idempotency_records'))
print('  quotes =', q('select count(*) from quotes'), ' 已消费 =', q('select count(*) from quotes where consumed_job_id is not null'))
for row in con.execute('select provider, currency, reserved, actual, state from cost_ledger order by provider'):
    print('   ledger：', row)
print('  审计：', [r[0] for r in con.execute('select action from audit_events order by created_at')])
print('  provider_attempts =', q('select count(*) from provider_attempts'), '（无付费提交）')
ids, hashes = con.execute('select photo_ids, photo_hashes from generation_snapshots').fetchone()
print('  快照 photo_ids =', ids)
print('  快照 photo_hashes =', hashes)
print('  QA 独立算出的 front 内容 sha256 =', sys.argv[2])
assert sys.argv[2] in hashes, '快照必须冻结照片内容哈希'
cfg = q("select provider_config from generation_snapshots")
assert 'api_key' not in cfg and 'secret' not in cfg.lower() and 'fake-not-used' not in cfg
print('  快照 provider_config 不含密钥：OK')
PY
echo "== 8b) 服务重启后同键重放 → 仍返回同一 job（不产生第二份生成单）"
stop_server
start_server
R=$(curl -s -o replay-after-restart.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: qa-smoke-replay' -d "$BODY" "$BASE/items/$ITEM/jobs")
RJID=$(json "['data']['id']" < replay-after-restart.json)
echo "  重启后重放 HTTP $R job=${RJID}（期望 202 且 = ${FIRST}）"
python3 - "$WORK/data/manual.sqlite3" <<'PY'
import sqlite3, sys
con = sqlite3.connect(sys.argv[1])
print('  重启后 jobs =', con.execute('select count(*) from jobs').fetchone()[0],
      ' cost_ledger =', con.execute('select count(*) from cost_ledger').fetchone()[0])
PY
[ "$RJID" = "$FIRST" ] || { echo "  重启后重放返回了不同 job"; exit 1; }

echo "== 8c) 价格版本变化（改文件 + 改 version + 重启）→ 旧报价提交被拒，不创建 job"
Q2=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"modelPreset\":\"tripo-h-v3.1-standard\"}" \
  "$BASE/items/$ITEM/estimates" | json "['data']['id']")
curl -s -o /dev/null -X POST "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$Q2/confirm"
stop_server
sed -i '' 's/^version = "2026-09-11"$/version = "2026-10-01"/' ./prices.toml
grep -m1 '^version' ./prices.toml
start_server
V=$(curl -s -o version-changed.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: qa-smoke-version' \
  -d "{\"quoteId\":\"$Q2\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3000,\"manualAiUsdMicros\":50000}}" \
  "$BASE/items/$ITEM/jobs")
echo "  HTTP $V reason=$(json "['error']['details']['reason']" < version-changed.json)"
python3 - "$WORK/data/manual.sqlite3" <<'PY'
import sqlite3, sys
con = sqlite3.connect(sys.argv[1])
print('  jobs =', con.execute('select count(*) from jobs').fetchone()[0], '（必须仍为 1）')
PY

echo "== 9) 服务进程外连检查（不应有非 loopback 连接）"
lsof -a -p "$SERVER_PID" -i -nP 2>/dev/null | grep -v LISTEN | grep -v "127.0.0.1" | grep -v "^COMMAND" \
  && { echo "  发现非 loopback 连接（异常）"; exit 1; } || echo "  无非 loopback 连接：OK"
echo "== QA 冒烟完成"
