#!/bin/bash
# QA 回合 26 · BUG-008 复验（新数据侧 + 恢复语义）：
#   1) 用**修复后的 dist 二进制**跑一条真实新链路（本机 QA fixture），断言
#      新产生的任务元数据（job_stages / provider_attempts 全部文本列）不含
#      供应商临时/签名 URL（判据见脚本内的 NEEDLES）。
#   2) 任务详情 API 不回显完整签名 URL，保留摘要（redacted/host/sha256）与 task ID。
#   3) 重启（SIGKILL）后 model_download 按 task ID **重新查询**（免费 GET），
#      付费提交计数不增加（恢复语义回归）。
# 全程只连 127.0.0.1；零外网、零付费。
#
# 用法：bash qa-r26-newdata-requery.sh <dist 二进制绝对路径>
set -uo pipefail

BIN="${1:?用法: qa-r26-newdata-requery.sh <绝对路径的 dist 二进制>}"
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
FIXTURES="${ROOT}/tests/fixtures/assets"
SRC_FIXTURE="${ROOT}/artifacts/web-mvp/t12-qa/qa-tripo-fixture.py"
FIXTURE_PY="${QA_DIR}/qa-r26-tripo-fixture.py"
WORK="$(mktemp -d /tmp/em-r26-newdata-XXXXXX)"
APP_PORT=18126
FIXTURE_PORT=18127
CANARY="qa-r26-canary-9f3c"
PASSWORD="qa-r26-password-7b1e"
RESULT=0

ok()  { echo "  [通过] $1"; }
bad() { echo "  [失败] $1"; RESULT=1; }
expect_eq() { if [[ "$2" == "$3" ]]; then ok "$1（=$3）"; else bad "$1（期望 $2，实际 $3）"; fi; }
hits() { local file="$1" needle="$2"; grep -c -- "$needle" "$file" 2>/dev/null | head -1; }

# QA 判据（本轮明确定义；不采信 RD 脚本）：
#   - `://`                = URL 形态（scheme 分隔符）
#   - `sign=` / `x-sign`   = 签名查询串
#   - `/qa-r26/model.glb`、`/qa-r26/preview.png` = URL path（本场景专用路径）
#   - 签名 canary 串        = 唯一随机串（http 整个文档里出现即泄露）
#   允许保留：裸 host（无 scheme/path/查询串）+ sha256 前 16 位 + task ID + 状态/计费。
NEEDLES=('://' 'sign=' '/qa-r26/model.glb' '/qa-r26/preview.png' "${CANARY}")

cleanup() {
  [[ -n "${SERVER_PID:-}" ]] && kill -9 "${SERVER_PID}" 2>/dev/null
  [[ -n "${FIXTURE_PID:-}" ]] && kill -9 "${FIXTURE_PID}" 2>/dev/null
  wait 2>/dev/null
  rm -rf "${WORK}"
}
trap cleanup EXIT

# --- fixture：QA 自己实现（回合 12），本回合换 canary/path/rendered 字段 -----------------
python3 - "${SRC_FIXTURE}" "${FIXTURE_PY}" "${CANARY}" <<'PY'
import sys
src, dst, canary = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(src, encoding="utf-8").read()
text = text.replace('SIGNATURE_CANARY = "qa-signature-canary-9931"',
                    f'SIGNATURE_CANARY = "{canary}"')
text = text.replace('TASK_ID = "qa-task-0001"', 'TASK_ID = "qa-r26-task-0001"')
text = text.replace('MODEL_URL = f"https://cdn.example.invalid/qa/model.glb?sign={SIGNATURE_CANARY}"',
                    'MODEL_URL = f"https://cdn.example.invalid/qa-r26/model.glb?sign={SIGNATURE_CANARY}"\n'
                    'RENDERED_URL = f"https://cdn.example.invalid/qa-r26/preview.png?sign={SIGNATURE_CANARY}"')
text = text.replace('"output": {"model_url": MODEL_URL},',
                    '"output": {"model_url": MODEL_URL, "rendered_image_url": RENDERED_URL},')
assert f'SIGNATURE_CANARY = "{canary}"' in text and "RENDERED_URL" in text
open(dst, "w", encoding="utf-8").write(text)
print(f"fixture patched → {dst}")
PY

echo "== QA 回合 26 新数据链路（工作目录 ${WORK}；二进制 $(shasum -a 256 "${BIN}" | awk '{print $1}')）"
mkdir -p "${WORK}"
cd "${WORK}" || exit 1

status_of() { # $1=stage_kind
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
    [[ "$(status_of "$1")" == "$2" ]] && return 0
    sleep 1
  done
  echo "!! 等待 $1 -> $2 超时（实际 $(status_of "$1")）"
  return 1
}

python3 "${FIXTURE_PY}" --port "${FIXTURE_PORT}" --log "${WORK}/fixture.jsonl" --scenario happy > fixture.out 2>&1 &
FIXTURE_PID=$!
for _ in $(seq 1 100); do grep -q "listening" fixture.out 2>/dev/null && break; sleep 0.1; done
cat fixture.out

printf '%s\n' "${PASSWORD}" > pw.txt && chmod 600 pw.txt
printf 'a%.0s' $(seq 1 3000) > page-text.txt
"${BIN}" init --data-dir ./data --password-file ./pw.txt > init.log 2>&1 || { echo "init 失败"; cat init.log; exit 1; }
cp "${ROOT}/price-catalog.example.toml" ./prices.toml
cat > ./data/config.toml <<TOML
price_catalog_path = "./prices.toml"

[providers.tripo]
base_url = "http://127.0.0.1:${FIXTURE_PORT}/v3"
api_key_env = "QA_TRIPO_KEY"

[providers.manual_ai]
base_url = "http://127.0.0.1:${FIXTURE_PORT}/v1"
model = "gpt-5-mini"
api_key_env = "QA_MANUAL_AI_KEY"
TOML

start_serve() {
  QA_TRIPO_KEY="qa-r26-fake-tripo-key" QA_MANUAL_AI_KEY="qa-r26-fake-manual-key" \
    "${BIN}" serve --data-dir ./data --listen "127.0.0.1:${APP_PORT}" >> serve.log 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 100); do
    curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
    sleep 0.1
  done
}
start_serve

BASE="http://127.0.0.1:${APP_PORT}/api/v1"
json() { python3 -c "import json,sys;print(json.load(sys.stdin)$1)"; }

curl -s -c cookies.txt -H 'content-type: application/json' \
  -d "{\"password\":\"${PASSWORD}\"}" "$BASE/auth/login" -o login.json
CSRF=$(json "['data']['csrfToken']" < login.json)
AUTH=(-b cookies.txt -H "x-csrf-token: $CSRF")
ITEM=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"QA R26 相机","model":"QA-R26"}' "$BASE/items" | json "['data']['id']")
upload() { curl -s "${AUTH[@]}" -F "purpose=$2" -F "file=@$1" "$BASE/items/$ITEM/assets"; }
DOC_JSON=$(upload "${FIXTURES}/sample-manual-text.pdf" document)
DOC=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceAssetId\":\"$(echo "$DOC_JSON" | json "['data']['id']")\",\"title\":\"QA R26 说明书\"}" \
  "$BASE/items/$ITEM/documents" | json "['data']['id']")
PDF_SHA=$(echo "$DOC_JSON" | json "['data']['sha256']")
PREP=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceSha256\":\"$PDF_SHA\"}" "$BASE/documents/$DOC/preparations" | json "['data']['id']")
for page in 1 2; do
  TXT=$(upload ./page-text.txt pageText | json "['data']['id']")
  IMG=$(upload "${FIXTURES}/sample-photo-front.jpg" pageImage | json "['data']['id']")
  curl -s -X PUT "${AUTH[@]}" -H 'content-type: application/json' \
    -d "{\"textAssetId\":\"$TXT\",\"imageAssetId\":\"$IMG\",\"viewport\":{\"width\":1240,\"height\":1754,\"rotation\":0}}" \
    "$BASE/preparations/$PREP/pages/$page" -o "page$page.json"
done
ETAG=$(curl -s "${AUTH[@]}" -D - -o /dev/null "$BASE/preparations/$PREP" | awk -F'"' 'tolower($1) ~ /^etag:/ {print $2}')
curl -s "${AUTH[@]}" -H 'content-type: application/json' -H "if-match: \"$ETAG\"" \
  -d '{"pageCount":2}' "$BASE/preparations/$PREP/complete" -o complete.json
FRONT_ASSET=$(upload "${FIXTURES}/sample-photo-front.jpg" photo | json "['data']['id']")
LEFT_ASSET=$(upload "${FIXTURES}/sample-photo-left.png" photo | json "['data']['id']")
FRONT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$FRONT_ASSET\",\"view\":\"front\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
LEFT_ID=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$LEFT_ASSET\",\"view\":\"left\"}" "$BASE/items/$ITEM/photos" | json "['data']['id']")
QUOTE=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"modelPreset\":\"tripo-h-v3.1-standard\"}" \
  "$BASE/items/$ITEM/estimates" | json "['data']['id']")
curl -s -X POST "${AUTH[@]}" "$BASE/items/$ITEM/estimates/$QUOTE/confirm" -o confirm.json
JOB=$(curl -s "${AUTH[@]}" -H 'content-type: application/json' -H 'idempotency-key: qa-r26-key' \
  -d "{\"quoteId\":\"$QUOTE\",\"preparationId\":\"$PREP\",\"photoIds\":[\"$FRONT_ID\",\"$LEFT_ID\"],\"limits\":{\"tripoCreditMinor\":3000,\"manualAiUsdMicros\":50000}}" \
  "$BASE/items/$ITEM/jobs" | json "['data']['id']")
echo "${JOB}" > job_id.txt
echo "== job=${JOB}"

echo
echo "-- 阶段 1：跑到 tripo_poll 成功（新数据写入）"
wait_for_stage tripo_poll succeeded 60 || bad "tripo_poll 未成功"

echo
echo "-- 阶段 2：新数据扫描（DB 全部文本列）"
sqlite3 ./data/manual.sqlite3 ".dump" > dump-phase1.sql
python3 - <<'PY' | tee scan-phase1.txt
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
needles = ['://', 'sign=', '/qa-r26/model.glb', '/qa-r26/preview.png', 'qa-r26-canary-9f3c']
print("== 全库逐表 '://' 命中（含表名，便于区分预期项）")
for (name,) in con.execute("select name from sqlite_master where type='table' order by name"):
    cols = [row[1] for row in con.execute(f"pragma table_info({name})")]
    for col in cols:
        q = f'select count(*) from "{name}" where instr(cast("{col}" as text), \'://\') > 0'
        try:
            n = con.execute(q).fetchone()[0]
        except sqlite3.Error as error:
            print(f"  !! {name}.{col} 扫描失败：{error}")
            continue
        if n:
            print(f"  {name}.{col}: {n}")
print
print("== 供应商事实表（job_stages / provider_attempts）全部文本列 × 全部判据")
for table in ("job_stages", "provider_attempts"):
    cols = [row[1] for row in con.execute(f"pragma table_info({table})")]
    for col in cols:
        for needle in needles:
            n = con.execute(
                f'select count(*) from "{table}" where instr(cast("{col}" as text), ?) > 0',
                (needle,)).fetchone()[0]
            if n:
                print(f"  !! {table}.{col} 命中 {needle!r}: {n}")
print("  （无输出 = 全部为 0）")
print
poll = con.execute(
    "select cast(usage_json as text) from job_stages where stage_kind='tripo_poll'").fetchone()[0]
print("== tripo_poll.usage_json（应只含摘要 + task id/状态/计费）")
print(" ", poll)
PY
if grep -q '!!' scan-phase1.txt; then bad "新数据扫描有命中（见 scan-phase1.txt）"; else ok "新数据 job_stages/provider_attempts 无 URL/签名/path 命中"; fi
python3 - <<'PY' | tee -a scan-phase1.txt
import sqlite3, json
con = sqlite3.connect('./data/manual.sqlite3')
usage = json.loads(con.execute("select usage_json from job_stages where stage_kind='tripo_poll'").fetchone()[0])
assert usage["remoteTaskId"] == "qa-r26-task-0001", usage
assert usage["modelUrl"]["redacted"] is True and usage["modelUrl"]["host"] == "cdn.example.invalid", usage
assert len(usage["modelUrl"]["sha256"]) == 16, usage
assert usage["renderedImageUrl"]["redacted"] is True, usage
assert usage["billing"]["creditMinor"] == 3000, usage
print("== 摘要与恢复判据保留：remoteTaskId / redacted / host / sha256(16) / billing 均在 ✔")
PY

echo
echo "-- 阶段 3：任务详情 API 不回显完整签名 URL"
curl -s -b cookies.txt "${BASE}/jobs/${JOB}" -o job-detail-1.json
for needle in "${NEEDLES[@]}"; do
  expect_eq "任务详情不含 '${needle}'" 0 "$(hits job-detail-1.json "$needle")"
done
python3 - job-detail-1.json <<'PY'
import json, sys
detail = json.load(open(sys.argv[1], encoding="utf-8"))["data"]
poll = [s for s in detail["stages"] if s["stageKind"] == "tripo_poll"][0]
print("  usage.modelUrl =", json.dumps(poll["usage"]["modelUrl"], ensure_ascii=False))
print("  usage.renderedImageUrl =", json.dumps(poll["usage"]["renderedImageUrl"], ensure_ascii=False))
PY

echo
echo "-- 阶段 4：重启（SIGKILL）后按 task ID 重新查询（不重新购买）"
submit_before=$(grep -c '"event": "submit"' fixture.jsonl)
poll_before=$(grep -c '"event": "poll"' fixture.jsonl)
python3 - <<'PY'
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
job = open('job_id.txt').read().strip()
# 让下载阶段重新入队（模拟"poll 已成功、下载未跑"的现场；缓存随进程重启清空）
cur = con.execute(
    "update job_stages set status='queued', next_run_at=NULL, last_error=NULL, needs_input_json=NULL "
    "where job_id=? and stage_kind='model_download'", (job,))
assert cur.rowcount == 1, cur.rowcount
con.commit()
print("  model_download 已重置为 queued（用例前提）")
PY
kill -9 "${SERVER_PID}" 2>/dev/null; wait "${SERVER_PID}" 2>/dev/null
start_serve
echo "  重启完成，等待下载阶段的新查询与结论…"
sleep 12
submit_after=$(grep -c '"event": "submit"' fixture.jsonl)
poll_after=$(grep -c '"event": "poll"' fixture.jsonl)
echo "  fixture 计数：submit ${submit_before} → ${submit_after}；poll ${poll_before} → ${poll_after}"
expect_eq "付费提交不增加（不重新购买）" "${submit_before}" "${submit_after}"
if [[ "${poll_after}" -gt "${poll_before}" ]]; then ok "重启后按 task ID 重新查询（免费 GET 增加）"; else bad "重启后没有重新查询"; fi
download_status=$(status_of model_download)
echo "  model_download 阶段状态 = ${download_status}（允许域为空 → 期望 needs_input，仅提示 host）"
python3 - <<'PY'
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
row = con.execute("select last_error from job_stages where stage_kind='model_download'").fetchone()
print("  model_download.last_error =", (row[0] or "(空)")[:240])
PY

echo
echo "-- 阶段 5：重启后的第二轮扫描（含 last_error 等非 JSON 文本列）"
sqlite3 ./data/manual.sqlite3 ".dump" > dump-phase2.sql
python3 - <<'PY' | tee scan-phase2.txt
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
needles = ['://', 'sign=', '/qa-r26/model.glb', '/qa-r26/preview.png', 'qa-r26-canary-9f3c']
print("== 重启后（第二进程）供应商事实表全部文本列扫描")
bad = 0
for table in ("job_stages", "provider_attempts"):
    cols = [row[1] for row in con.execute(f"pragma table_info({table})")]
    for col in cols:
        for needle in needles:
            n = con.execute(
                f'select count(*) from "{table}" where instr(cast("{col}" as text), ?) > 0',
                (needle,)).fetchone()[0]
            if n:
                bad += 1
                print(f"  !! {table}.{col} 命中 {needle!r}: {n}")
print("  （没有任何命中行 = 全部为 0）")
print("model_download.last_error 长度 =",
      len(con.execute("select coalesce(last_error,'') from job_stages where stage_kind='model_download'").fetchone()[0]))
PY
if grep -q '!!' scan-phase2.txt; then bad "重启后扫描有命中"; else ok "重启后 job_stages/provider_attempts 全部文本列无命中"; fi
curl -s -b cookies.txt "${BASE}/jobs/${JOB}" -o job-detail-2.json
for needle in "${NEEDLES[@]}"; do
  expect_eq "重启后任务详情不含 '${needle}'" 0 "$(hits job-detail-2.json "$needle")"
done

echo
echo "-- 阶段 5b：出网核查（全部流量必须是回环；零真实外网）"
nonloopback=$(grep -o 'https\?://[a-zA-Z0-9.:_-]*' serve.log | grep -v "127.0.0.1" | grep -c . )
expect_eq "serve 日志里非回环主机数（provider 全部指向本机 fixture）" 0 "${nonloopback}"
if [[ "${nonloopback}" -gt 0 ]]; then grep -o 'https\?://[a-zA-Z0-9.:_-]*' serve.log | grep -v "127.0.0.1" | sort -u | sed 's/^/  !! /'; fi

echo
echo "-- 阶段 6：日志（serve）"
# 允许的 '://' 来源：部署方自己的监听横幅与 provider baseUrl 配置回显（无签名、无 CDN path）。
allowed=$(grep "://" serve.log | grep -c -e "listening on http://" -e '"baseUrl"')
total_url=$(grep -c "://" serve.log)
other=$(grep "://" serve.log | grep -v -e "listening on http://" -e '"baseUrl"' | grep -c . )
expect_eq "serve 日志 '://' 只来自监听横幅/baseUrl 回显" "${allowed}" "${total_url}"
expect_eq "serve 日志其余 '://' 行" 0 "${other}"
for needle in "${CANARY}" 'sign=' '/qa-r26/model.glb'; do
  expect_eq "serve 日志不含 '${needle}'" 0 "$(hits serve.log "$needle")"
done

mkdir -p "${QA_DIR}/work-newdata"
cp ./scan-phase1.txt ./job-detail-1.json ./job-detail-2.json "${QA_DIR}/work-newdata/" 2>/dev/null
cp ./fixture.jsonl "${QA_DIR}/work-newdata/fixture.jsonl" 2>/dev/null
cp ./serve.log "${QA_DIR}/work-newdata/serve.log" 2>/dev/null
cp ./data/manual.sqlite3 "${QA_DIR}/work-newdata/manual.sqlite3" 2>/dev/null

echo
echo "== 新数据剧本结果：$([[ ${RESULT} = 0 ]] && echo PASS || echo FAIL)"
exit ${RESULT}
