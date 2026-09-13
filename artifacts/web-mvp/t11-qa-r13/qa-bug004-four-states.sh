#!/bin/bash
# T11 QA 回合 13（BUG-004 复验）：真实二进制 + curl + sqlite3 交叉核对
# `GET /items/{id}/estimates/{quoteId}` 在四种状态下的响应是否等于 DB 事实：
#   S1 未确认未消费 / S2 已确认未消费 / S3 已消费 / S4 已过期（可读不可用）。
# 独立于 RD 脚本：另一端口（18341）、另一套数据、另一目录；退出即清理。
# 说明：S4 的过期报价无法经 HTTP 造出（服务端时钟 + 0006 触发器冻结 expires_at），
# 用 sqlite3 在 DB 层克隆（只改 id 与 quote_json 内的 id、expires_at），并在日志中标注"DB 合成"。
set -uo pipefail
BIN="${1:?用法: qa-bug004-four-states.sh <binary 绝对路径>}"
REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
FIX="$REPO/tests/fixtures/assets"
WORK="$(mktemp -d /tmp/em-r13-bug004-XXXXXX)"
PORT=18341
DB="$WORK/data/manual.sqlite3"
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
echo "== QA r13 工作目录：${WORK}（退出时删除）；二进制：${BIN}"
shasum -a 256 "$BIN"

printf 'qa-r13-password-b004\n' > pw.txt && chmod 600 pw.txt
printf 'c%.0s' $(seq 1 2000) > page-text.txt
cp "$REPO/price-catalog.example.toml" ./prices.toml

echo "== 0) init + 正常启动（假凭据，只连 loopback）"
"$WORK/everything-manual" init --data-dir ./data --password-file ./pw.txt >/dev/null 2>&1 || exit 1
cat > ./data/config.toml <<'TOML'
price_catalog_path = "./prices.toml"

[providers.tripo]
api_key_env = "QA_R13_TRIPO_KEY"

[providers.manual_ai]
model = "gpt-5-mini"
api_key_env = "QA_R13_MANUAL_KEY"
TOML
"$WORK/everything-manual" check --data-dir ./data > check.log 2>&1
echo "  check 退出码 = $?（期望 0）"
start_server() {
  QA_R13_TRIPO_KEY="fake-not-used" QA_R13_MANUAL_KEY="fake-not-used" \
    "$WORK/everything-manual" serve --data-dir ./data --listen 127.0.0.1:${PORT} >> serve.log 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 80); do
    curl -sf "http://127.0.0.1:${PORT}/api/v1/health/ready" >/dev/null && return 0
    sleep 0.1
  done
  echo "  服务未就绪"; exit 1
}
start_server
BASE="http://127.0.0.1:${PORT}/api/v1"
curl -s -c cookies.txt -H 'content-type: application/json' \
  -d '{"password":"qa-r13-password-b004"}' "$BASE/auth/login" -o login.json
CSRF=$(json "['data']['csrfToken']" < login.json)
AUTH=(-b cookies.txt -H "x-csrf-token: $CSRF")

echo "== 1) 物品 / PDF / 2 页准备 / front+left 照片"
ITEM=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"QA r13 BUG-004 物品","model":"QA-R13-B004"}' "$BASE/items" | json "['data']['id']")
upload() { curl -s "${AUTH[@]}" -F "purpose=$2" -F "file=@$1" "$BASE/items/$ITEM/assets"; }
DOC_ASSET_JSON=$(upload "$FIX/sample-manual-text.pdf" document)
DOC_ASSET=$(echo "$DOC_ASSET_JSON" | json "['data']['id']")
PDF_SHA=$(echo "$DOC_ASSET_JSON" | json "['data']['sha256']")
DOC=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceAssetId\":\"$DOC_ASSET\",\"title\":\"QA r13 说明书\"}" "$BASE/items/$ITEM/documents" | json "['data']['id']")
PREP=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceSha256\":\"$PDF_SHA\"}" "$BASE/documents/$DOC/preparations" | json "['data']['id']")
P1T=$(upload ./page-text.txt pageText | json "['data']['id']")
P1I=$(upload "$FIX/sample-photo-front.jpg" pageImage | json "['data']['id']")
PC=$(curl -s -o /dev/null -w '%{http_code}' -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"textAssetId\":\"$P1T\",\"imageAssetId\":\"$P1I\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
  "$BASE/preparations/$PREP/pages/1")
[ "$PC" = "200" ] || { echo "  第 1 页写入 HTTP $PC"; exit 1; }
P2I=$(upload "$FIX/sample-photo-front.jpg" pageImage | json "['data']['id']")
PC=$(curl -s -o /dev/null -w '%{http_code}' -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"textAssetId\":null,\"imageAssetId\":\"$P2I\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
  "$BASE/preparations/$PREP/pages/2")
[ "$PC" = "200" ] || { echo "  第 2 页写入 HTTP $PC"; exit 1; }
ETAG=$(curl -s "${AUTH[@]}" -D - -o /dev/null "$BASE/preparations/$PREP" | awk -F'"' 'tolower($1) ~ /^etag:/ {print $2}')
PC=$(curl -s -o complete.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' -H "if-match: \"$ETAG\"" \
  -d '{"pageCount":2}' "$BASE/preparations/$PREP/complete")
[ "$PC" = "200" ] || { echo "  准备封存 HTTP $PC"; cat complete.json; exit 1; }
FRONT_ASSET=$(upload "$FIX/sample-photo-front.jpg" photo | json "['data']['id']")
LEFT_ASSET=$(upload "$FIX/sample-photo-left.png" photo | json "['data']['id']")
FRONT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$FRONT_ASSET\",\"view\":\"front\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
LEFT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$LEFT_ASSET\",\"view\":\"left\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")

echo "== 2) 报价 201（S1 起点）"
EST_CODE=$(curl -s -o estimate.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"modelPreset\":\"tripo-h-v3.1-standard\"}" \
  "$BASE/items/$ITEM/estimates")
echo "  HTTP ${EST_CODE}（期望 201）"
[ "$EST_CODE" = "201" ] || { cat estimate.json; echo; exit 1; }
QUOTE=$(json "['data']['id']" < estimate.json)
TRIPO_UB=$(json "['data']['amounts']['tripo']['upperBoundMinor']" < estimate.json)
MANUAL_UB=$(json "['data']['amounts']['manualAi']['upperBoundMinor']" < estimate.json)
read_estimate() {  # $1=quoteId  $2=输出文件
  curl -s -o "$2" -w '%{http_code}' "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$1"
}
sha_db() { sqlite3 "$DB" "$1"; }

echo "== S1｜未确认未消费：GET 三字段 null 且 = DB 三列 null"
G1=$(read_estimate "$QUOTE" s1-get.json); echo "  GET HTTP ${G1}（期望 200）"
python3 - s1-get.json estimate.json "$DB" "$QUOTE" <<'PY'
import json, sqlite3, sys
from datetime import datetime, timezone
def rfc3339(ms):
    # 与 time crate 的 Rfc3339 一致：小数部分去掉尾随 0，为 0 则整体省略
    base = datetime.fromtimestamp(ms // 1000, tz=timezone.utc).strftime('%Y-%m-%dT%H:%M:%S')
    frac = ms % 1000
    return base + 'Z' if frac == 0 else f"{base}.{f'{frac:03d}'.rstrip('0')}Z"
got = json.load(open(sys.argv[1]))['data']
created = json.load(open(sys.argv[2]))['data']
con = sqlite3.connect(sys.argv[3])
# 渲染口径自检：用创建立即返回的 expiresAt（服务端 to_rfc3339）对照 DB 冻结的 expires_at
db_expires = con.execute("select expires_at from quotes where id=?", (sys.argv[4],)).fetchone()[0]
assert created['expiresAt'] == rfc3339(db_expires), \
    f"渲染口径自检失败：服务端 {created['expiresAt']} vs 本脚本推导 {rfc3339(db_expires)}"
conf, cons, job = con.execute("select confirmed_at, consumed_at, consumed_job_id from quotes where id=?", (sys.argv[4],)).fetchone()
assert conf is None and cons is None and job is None, f"DB 前置：三列必须为 NULL，实际 {conf},{cons},{job}"
for k in ('confirmedAt', 'consumedAt', 'consumedJobId'):
    assert got[k] is None, f"S1 GET {k} 必须为 null（不得隐式确认）：{got[k]}"
# 冻结部分：创建响应与回读逐值一致
frozen = {k: v for k, v in got.items() if k not in ('confirmedAt', 'consumedAt', 'consumedJobId')}
base = {k: v for k, v in created.items() if k not in ('confirmedAt', 'consumedAt', 'consumedJobId')}
assert frozen == base, "S1 冻结部分与创建响应不一致"
print("  S1 OK：GET 三字段 null、与 DB 三列一致；金额/expiresAt/priceVersion/sendScope 与创建响应逐值一致")
PY
QJ_S1=$(sha_db "select quote_json from quotes where id='$QUOTE'" | shasum -a 256 | awk '{print $1}')

echo "== S2｜已确认未消费：GET confirmedAt = 确认响应 = DB converted(RFC3339)"
C2=$(curl -s -o confirm.json -w '%{http_code}' -X POST "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$QUOTE/confirm")
CONFIRM_AT=$(json "['data']['confirmedAt']" < confirm.json)
echo "  确认 HTTP ${C2}（期望 200）confirmedAt=${CONFIRM_AT}"
G2=$(read_estimate "$QUOTE" s2-get.json); echo "  GET HTTP ${G2}（期望 200）"
python3 - s2-get.json confirm.json estimate.json "$DB" "$QUOTE" <<'PY'
import json, sqlite3, sys
from datetime import datetime, timezone
def rfc3339(ms):
    # 与 time crate 的 Rfc3339 一致：小数部分去掉尾随 0，为 0 则整体省略
    base = datetime.fromtimestamp(ms // 1000, tz=timezone.utc).strftime('%Y-%m-%dT%H:%M:%S')
    frac = ms % 1000
    return base + 'Z' if frac == 0 else f"{base}.{f'{frac:03d}'.rstrip('0')}Z"
got = json.load(open(sys.argv[1]))['data']
conf_resp = json.load(open(sys.argv[2]))['data']['confirmedAt']
created = json.load(open(sys.argv[3]))['data']
con = sqlite3.connect(sys.argv[4])
conf, cons, job = con.execute("select confirmed_at, consumed_at, consumed_job_id from quotes where id=?", (sys.argv[5],)).fetchone()
assert got['confirmedAt'] == conf_resp, f"S2 GET confirmedAt({got['confirmedAt']}) != 确认响应({conf_resp})"
assert got['confirmedAt'] == rfc3339(conf), f"S2 GET confirmedAt({got['confirmedAt']}) != DB({rfc3339(conf)})"
assert got['consumedAt'] is None and got['consumedJobId'] is None, "S2 消费字段必须仍为 null"
assert cons is None and job is None, "S2 DB 消费列必须仍为 NULL"
frozen = {k: v for k, v in got.items() if k not in ('confirmedAt', 'consumedAt', 'consumedJobId')}
base = {k: v for k, v in created.items() if k not in ('confirmedAt', 'consumedAt', 'consumedJobId')}
assert frozen == base, "S2 冻结部分被回读改写"
print(f"  S2 OK：GET confirmedAt == 确认响应 == DB({conf} ms → {rfc3339(conf)})；消费字段仍 null；冻结部分不变")
PY

echo "== S3｜已消费：GET consumedAt/consumedJobId = DB = 真实 job；quote_json 未被回写"
C3=$(curl -s -o job.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: qa-r13-bug004-job' \
  -d "{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":$TRIPO_UB,\"manualAiUsdMicros\":$MANUAL_UB}}" \
  "$BASE/items/$ITEM/jobs")
JOB=$(json "['data']['id']" < job.json)
echo "  建单 HTTP ${C3}（期望 202）job=${JOB}"
G3=$(read_estimate "$QUOTE" s3-get.json); echo "  GET HTTP ${G3}（期望 200）"
python3 - s3-get.json "$DB" "$QUOTE" "$JOB" "$CONFIRM_AT" estimate.json <<'PY'
import json, sqlite3, sys
from datetime import datetime, timezone
def rfc3339(ms):
    base = datetime.fromtimestamp(ms // 1000, tz=timezone.utc).strftime('%Y-%m-%dT%H:%M:%S')
    frac = ms % 1000
    return base + 'Z' if frac == 0 else f"{base}.{f'{frac:03d}'.rstrip('0')}Z"
got = json.load(open(sys.argv[1]))['data']
con = sqlite3.connect(sys.argv[2])
conf, cons, job = con.execute("select confirmed_at, consumed_at, consumed_job_id from quotes where id=?", (sys.argv[3],)).fetchone()
assert got['consumedJobId'] == sys.argv[4], f"S3 GET consumedJobId({got['consumedJobId']}) != 真实 job({sys.argv[4]})"
assert got['consumedJobId'] == job, f"S3 GET consumedJobId 与 DB({job}) 不一致"
assert got['consumedAt'] == rfc3339(cons), f"S3 GET consumedAt({got['consumedAt']}) != DB({rfc3339(cons)})"
assert got['confirmedAt'] == sys.argv[5], "S3 确认时间必须保持首次确认值"
assert got['confirmedAt'] == rfc3339(conf), "S3 confirmedAt 必须 = DB"
created = json.load(open(sys.argv[6]))['data']
frozen = {k: v for k, v in got.items() if k not in ('confirmedAt', 'consumedAt', 'consumedJobId')}
base = {k: v for k, v in created.items() if k not in ('confirmedAt', 'consumedAt', 'consumedJobId')}
assert frozen == base, "S3 冻结部分（金额/expiresAt/价格版本/发送范围）被回读改写"
print(f"  S3 OK：GET consumedAt == DB({cons} ms → {rfc3339(cons)})；consumedJobId == DB == 真实 job；confirmedAt 保持；冻结部分不变")
PY
QJ_S3=$(sha_db "select quote_json from quotes where id='$QUOTE'" | shasum -a 256 | awk '{print $1}')
echo "  quote_json sha256：S1=$QJ_S1  S3=$QJ_S3"
[ "$QJ_S1" = "$QJ_S3" ] || { echo "  quote_json 被回写（冻结语义被破坏）"; exit 1; }
python3 - "$DB" "$QUOTE" estimate.json <<'PY'
import json, sqlite3, sys
stored = json.loads(sqlite3.connect(sys.argv[1]).execute("select quote_json from quotes where id=?", (sys.argv[2],)).fetchone()[0])
created = json.load(open(sys.argv[3]))['data']
assert stored == created, "DB quote_json 必须仍等于创建响应（三字段在冻结载荷中仍为 null）"
assert stored['consumedAt'] is None and stored['consumedJobId'] is None and stored['confirmedAt'] is None
print("  OK：DB quote_json 仍为创建时冻结载荷（三字段 null），未被回读/消费回写")
PY
C3b=$(curl -s -o reuse.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: qa-r13-bug004-reuse' \
  -d "{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":$TRIPO_UB,\"manualAiUsdMicros\":$MANUAL_UB}}" \
  "$BASE/items/$ITEM/jobs")
echo "  已消费报价新键重提 HTTP ${C3b} reason=$(json "['error']['details']['reason']" < reuse.json)（期望 422 quoteAlreadyUsed）"
NJOBS=$(sha_db "select count(*) from jobs")
echo "  jobs = ${NJOBS}（期望 1）"
[ "$NJOBS" = "1" ] || exit 1

echo "== S4｜已过期（DB 合成：HTTP 造不出过期报价，expires_at 被触发器冻结）"
PAST=$(( $(date +%s) * 1000 - 120000 ))
python3 - s4-clone.json "$DB" "$QUOTE" "$PAST" <<'PY'
import json, sqlite3, sys, uuid
from datetime import datetime, timezone
def rfc3339(ms):
    base = datetime.fromtimestamp(ms // 1000, tz=timezone.utc).strftime('%Y-%m-%dT%H:%M:%S')
    frac = ms % 1000
    return base + 'Z' if frac == 0 else f"{base}.{f'{frac:03d}'.rstrip('0')}Z"
con = sqlite3.connect(sys.argv[2])
row = con.execute("select id, item_id, preparation_id, photo_ids, photo_hashes, input_hash, model_preset, provider_config, price_version, price_snapshot_date, page_count, max_output_tokens, quote_json, created_at, confirmed_at, confirmation_json from quotes where id=?", (sys.argv[3],)).fetchone()
new_id = str(uuid.uuid4())
past_ms = int(sys.argv[4])
# 与真实"过期报价"一致：载荷内的 id 与 expiresAt 与列同源（服务层创建过期报价时两者写入同一时刻）
payload = json.loads(row[12].replace(row[0], new_id))
payload['expiresAt'] = rfc3339(past_ms)
qj = json.dumps(payload, ensure_ascii=False, separators=(',', ':'))
sql = ("insert into quotes (id, item_id, preparation_id, photo_ids, photo_hashes, input_hash, model_preset, provider_config, price_version, price_snapshot_date, page_count, max_output_tokens, quote_json, expires_at, confirmed_at, confirmation_json, consumed_at, consumed_job_id, created_at) "
       "values (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,NULL,NULL,?)")
con.execute(sql, (new_id, *row[1:12], qj, past_ms, None, None, row[13]))
con.commit()
json.dump({"quoteId": new_id, "source": row[0]}, open(sys.argv[1], "w"))
print(f"  合成过期报价（未确认未消费）：{new_id}（源报价 {row[0]}），expires_at={past_ms} → {rfc3339(past_ms)}")
PY
QUOTE_EXPIRED=$(json "['quoteId']" < s4-clone.json)
GS4=$(read_estimate "$QUOTE_EXPIRED" s4-get.json); echo "  GET HTTP ${GS4}（期望 200：可读）"
python3 - s4-get.json "$DB" "$QUOTE_EXPIRED" estimate.json <<'PY'
import json, sqlite3, sys
from datetime import datetime, timezone
def rfc3339(ms):
    base = datetime.fromtimestamp(ms // 1000, tz=timezone.utc).strftime('%Y-%m-%dT%H:%M:%S')
    frac = ms % 1000
    return base + 'Z' if frac == 0 else f"{base}.{f'{frac:03d}'.rstrip('0')}Z"
got = json.load(open(sys.argv[1]))['data']
con = sqlite3.connect(sys.argv[2])
exp, conf, cons, job = con.execute("select expires_at, confirmed_at, consumed_at, consumed_job_id from quotes where id=?", (sys.argv[3],)).fetchone()
assert got['expiresAt'] == rfc3339(exp), "S4 回读 expiresAt 必须 = DB 冻结值"
assert got['confirmedAt'] is None and got['consumedAt'] is None and got['consumedJobId'] is None, "S4 未确认未消费的状态必须如实为 null（过期不派生状态）"
# 冻结部分（除 id 与合成改过的 expiresAt 外）与创建响应一致，且不存在派生的"是否可用"字段
created = json.load(open(sys.argv[4]))['data']
frozen = {k: v for k, v in got.items() if k not in ('id', 'expiresAt', 'confirmedAt', 'consumedAt', 'consumedJobId')}
base = {k: v for k, v in created.items() if k not in ('id', 'expiresAt', 'confirmedAt', 'consumedAt', 'consumedJobId')}
assert frozen == base, "S4 冻结部分（金额/价格版本/发送范围）与创建响应不一致"
assert not any('expire' in k.lower() and k != 'expiresAt' for k in got), "回读不得新增派生可用性字段"
print(f"  S4a OK：过期报价 200 可读；expiresAt == DB({rfc3339(exp)})；三字段 null（不把过期当已确认/已消费）；无派生可用性字段")
PY
CX=$(curl -s -o s4-confirm.json -w '%{http_code}' -X POST "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$QUOTE_EXPIRED/confirm")
JX=$(curl -s -o s4-job.json -w '%{http_code}' "${AUTH[@]}" -H 'content-type: application/json' \
  -H 'idempotency-key: qa-r13-bug004-expired' \
  -d "{\"quoteId\":\"$QUOTE_EXPIRED\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":$TRIPO_UB,\"manualAiUsdMicros\":$MANUAL_UB}}" \
  "$BASE/items/$ITEM/jobs")
echo "  过期报价 confirm HTTP ${CX} reason=$(json "['error']['details']['reason']" < s4-confirm.json)（期望 422 quoteExpired）"
echo "  过期报价 submit  HTTP ${JX} reason=$(json "['error']['details']['reason']" < s4-job.json)（期望 422 quoteExpired）"
python3 - "$DB" "$QUOTE_EXPIRED" <<'PY'
import sqlite3, sys
con = sqlite3.connect(sys.argv[1])
conf, cons, job = con.execute("select confirmed_at, consumed_at, consumed_job_id from quotes where id=?", (sys.argv[2],)).fetchone()
assert conf is None and cons is None and job is None, "被拒后状态列必须保持 NULL"
print(f"  S4a OK：confirm/提交均被拒（可用性由服务端动作判定）；DB 三列仍 NULL；jobs =", con.execute("select count(*) from jobs").fetchone()[0])
PY
[ "$(sha_db 'select count(*) from jobs')" = "1" ] || exit 1

echo "== S4b｜已确认但已过期（合成：克隆 S2 已确认行 + expires_at 过去）：事实不因过期消失"
python3 - s4b-clone.json "$DB" "$QUOTE" "$PAST" <<'PY'
import json, sqlite3, sys, uuid
from datetime import datetime, timezone
def rfc3339(ms):
    base = datetime.fromtimestamp(ms // 1000, tz=timezone.utc).strftime('%Y-%m-%dT%H:%M:%S')
    frac = ms % 1000
    return base + 'Z' if frac == 0 else f"{base}.{f'{frac:03d}'.rstrip('0')}Z"
con = sqlite3.connect(sys.argv[2])
row = con.execute("select id, item_id, preparation_id, photo_ids, photo_hashes, input_hash, model_preset, provider_config, price_version, price_snapshot_date, page_count, max_output_tokens, quote_json, created_at, confirmed_at, confirmation_json from quotes where id=?", (sys.argv[3],)).fetchone()
new_id = str(uuid.uuid4())
past_ms = int(sys.argv[4])
payload = json.loads(row[12].replace(row[0], new_id))
payload['expiresAt'] = rfc3339(past_ms)
qj = json.dumps(payload, ensure_ascii=False, separators=(',', ':'))
sql = ("insert into quotes (id, item_id, preparation_id, photo_ids, photo_hashes, input_hash, model_preset, provider_config, price_version, price_snapshot_date, page_count, max_output_tokens, quote_json, expires_at, confirmed_at, confirmation_json, consumed_at, consumed_job_id, created_at) "
       "values (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,NULL,NULL,?)")
con.execute(sql, (new_id, *row[1:12], qj, past_ms, row[14], row[15], row[13]))
con.commit()
json.dump({"quoteId": new_id}, open(sys.argv[1], "w"))
print(f"  合成已确认+已过期报价：{new_id}（沿用真实确认列 {row[14]}，expires_at={rfc3339(past_ms)}）")
PY
QUOTE_EXP_CONF=$(json "['quoteId']" < s4b-clone.json)
GS4B=$(read_estimate "$QUOTE_EXP_CONF" s4b-get.json); echo "  GET HTTP ${GS4B}（期望 200）"
python3 - s4b-get.json "$DB" "$QUOTE_EXP_CONF" "$CONFIRM_AT" estimate.json <<'PY'
import json, sqlite3, sys
from datetime import datetime, timezone
def rfc3339(ms):
    base = datetime.fromtimestamp(ms // 1000, tz=timezone.utc).strftime('%Y-%m-%dT%H:%M:%S')
    frac = ms % 1000
    return base + 'Z' if frac == 0 else f"{base}.{f'{frac:03d}'.rstrip('0')}Z"
got = json.load(open(sys.argv[1]))['data']
con = sqlite3.connect(sys.argv[2])
conf, cons, job = con.execute("select confirmed_at, consumed_at, consumed_job_id from quotes where id=?", (sys.argv[3],)).fetchone()
assert got['confirmedAt'] == sys.argv[4], f"S4b 过期报价的确认事实必须保留：{got['confirmedAt']} != {sys.argv[4]}"
assert got['confirmedAt'] == rfc3339(conf), "S4b confirmedAt 必须 = DB"
assert got['consumedAt'] is None and got['consumedJobId'] is None, "S4b 未消费必须为 null"
created = json.load(open(sys.argv[5]))['data']
frozen = {k: v for k, v in got.items() if k not in ('id', 'expiresAt', 'confirmedAt', 'consumedAt', 'consumedJobId')}
base = {k: v for k, v in created.items() if k not in ('id', 'expiresAt', 'confirmedAt', 'consumedAt', 'consumedJobId')}
assert frozen == base, "S4b 冻结部分与创建响应不一致"
print(f"  S4b OK：过期但已确认的报价回读仍给出 confirmedAt == DB == 首次确认({sys.argv[4]})；消费字段 null；冻结部分不变")
PY

echo "== 5) 收口：岗位不变量 + 无外连"
python3 - "$DB" <<'PY'
import sqlite3, sys
con = sqlite3.connect(sys.argv[1])
q = lambda s: con.execute(s).fetchone()[0]
print('  jobs =', q('select count(*) from jobs'), ' quotes =', q('select count(*) from quotes'),
      '（1 源 + 1 已确认源 + 2 合成过期）')
print('  provider_attempts =', q('select count(*) from provider_attempts'), '（期望 0：无付费提交）')
PY
lsof -a -p "$SERVER_PID" -i -nP 2>/dev/null | grep -v LISTEN | grep -v "127.0.0.1" | grep -v "^COMMAND" \
  && { echo "  发现非 loopback 连接（异常）"; exit 1; } || echo "  无非 loopback 连接：OK"
echo "== QA r13 BUG-004 四状态复验完成"
