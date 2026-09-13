#!/bin/bash
# QA 回合 26 · BUG-008 复验（历史数据侧）：用**修复前二进制产生的归档样例库**
# （QA 自己复现其 URL 行）+ QA **自己注入**的历史现场（JSON 与非 JSON 文本列），
# 验证：
#   1) backup 快照内供应商事实表（job_stages / provider_attempts 全部文本列）不含
#      临时/签名 URL（判据：'://'、'sign='、URL path、签名 canary）；
#   2) 源 data-dir **不被就地改写**（库文件 + dump + 全部 blob 指纹不变，无 UPDATE）；
#   3) 快照仍可用：restore 退出 0、行数与资产保留、serve 后可登录、任务详情不回显
#      URL、PDF/GLB 字节可读、导出包干净；
#   4) 不过度清洗：用户填写的出处链接（documents.source_url）必须原样保留；
#   5) 兜底（fail-closed）：把 URL 注入"导出清单的生成字段"来源后，导出必须失败
#      且不出包（export_manifest_url_forbidden）。
# 全程只连 127.0.0.1；零外网、零付费。
#
# 用法：bash qa-r26-historical-backup.sh <dist 二进制绝对路径>
set -uo pipefail

BIN="${1:?用法: qa-r26-historical-backup.sh <绝对路径的 dist 二进制>}"
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
INFO="${ROOT}/artifacts/web-mvp/t20-rd/rehearsal-info.json"
WORK="$(mktemp -d /tmp/em-r26-hist-XXXXXX)"
APP_PORT=18128
APP2_PORT=18129
CANARY="qa-r26-hist-9c4d"
USER_CANARY="qa-r26-user-link-3ad9"
RESULT=0

ok()  { echo "  [通过] $1"; }
bad() { echo "  [失败] $1"; RESULT=1; }
expect_eq() { if [[ "$2" == "$3" ]]; then ok "$1（=$3）"; else bad "$1（期望 $2，实际 $3）"; fi; }
hits() { grep -c -- "$2" "$1" 2>/dev/null | head -1; }

cleanup() {
  for pid in ${SERVE_PIDS:-}; do kill -9 "${pid}" 2>/dev/null; done
  wait 2>/dev/null
  rm -rf "${WORK}"
}
trap cleanup EXIT
SERVE_PIDS=""

PW=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['password'])" "${INFO}")
RELEASE_ID=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['releaseId'])" "${INFO}")

echo "== QA 回合 26 历史数据剧本（工作目录 ${WORK}）"
echo "二进制 sha256: $(shasum -a 256 "${BIN}" | awk '{print $1}')"
mkdir -p "${WORK}"
cd "${WORK}" || exit 1

# --- 0) QA 自己复核归档样例（修复前产物）确实含临时 URL ----------------------
echo
echo "-- 0) 归档样例库（修复前二进制产生）的 URL 行（QA 自证）"
sqlite3 "${SAMPLE}/database/manual.sqlite3" ".dump" > sample-dump.sql
expect_eq "样例库 dump 的 '://' 行数（修复前证据）" 1 "$(hits sample-dump.sql '://')"
sqlite3 "${SAMPLE}/database/manual.sqlite3" \
  "select stage_kind || ' | ' || substr(usage_json, instr(usage_json,'modelUrl'), 120) from job_stages where instr(usage_json,'://')>0" | sed 's/^/  /'

# --- 1) restore 到本机 data-dir --------------------------------------------
echo
echo "-- 1) restore 归档样例 → ${WORK}/data"
"${BIN}" restore --from "${SAMPLE}" --data-dir ./data > restore1.log 2>&1
expect_eq "restore 退出码" 0 "$?"
LIVE=./data/manual.sqlite3
expect_eq "源库仍保留历史 URL（不强制迁移）" 1 "$(hits <(sqlite3 "${LIVE}" .dump) '://')"

# --- 2) QA 自己注入历史现场（JSON + 非 JSON 文本 + 用户内容） -----------------
echo
echo "-- 2) QA 注入历史现场（canary=${CANARY}）"
python3 - "${LIVE}" "${CANARY}" "${USER_CANARY}" <<'PY'
import json, sqlite3, sys
db, canary, user_canary = sys.argv[1], sys.argv[2], sys.argv[3]
con = sqlite3.connect(db)
job = con.execute("select id from jobs").fetchone()[0]
stage = con.execute("select id from job_stages where stage_kind='tripo_poll'").fetchone()[0]
attempt = con.execute("select id from provider_attempts limit 1").fetchone()[0]

url_model = f"https://cdn.example.invalid/qa-r26-hist/model.glb?sign={canary}&expires=1799999999"
url_preview = f"https://cdn.example.invalid/qa-r26-hist/preview.png?sign={canary}"
usage = json.dumps({
    "remoteTaskId": "t20-fixture-task-0001",
    "rawStatus": "success",
    "normalizedStatus": "success",
    "progress": "100",
    "modelUrl": url_model,
    "renderedImageUrl": url_preview,
    "billing": {"creditMinor": 3000, "currency": "credit_minor", "literal": "30",
                "sourceField": "credits_consumed"},
    "dataKeys": ["credits_consumed", "output", "progress", "status", "task_id"],
}, ensure_ascii=False)
con.execute("update job_stages set usage_json = ? where id = ?", (usage, stage))
# 非 JSON 文本列（历史实现可能把带签名 URL 的错误/诊断写成自由文本）
con.execute("update job_stages set last_error = ? where id = ?",
            (f"模型下载失败：链接过期 https://cdn.example.invalid/qa-r26-hist/model.glb?sign={canary} 请重试", stage))
con.execute("update provider_attempts set last_error = ? where id = ?",
            (f"查询远端任务失败 {url_preview}（已过期）", attempt))
# 用户填写的出处链接（contracts §7「来源」）：不属于供应商临时地址，必须保留
con.execute("update documents set source_url = ?",
            (f"https://user.example.invalid/spec?sig={user_canary}",))
con.commit()
rows = con.execute("select count(*) from job_stages where instr(cast(usage_json as text),'://')>0").fetchone()[0]
print(f"  注入完成：含 URL 的 usage 行 {rows}；job={job}")
PY
injected=$(python3 -c "
import sqlite3
con=sqlite3.connect('${LIVE}')
n=0
for t,c in (('job_stages','usage_json'),('job_stages','last_error'),('provider_attempts','last_error')):
    n+=con.execute(f'select count(*) from {t} where instr(coalesce({c},\'\'),\'://\')>0').fetchone()[0]
print(n)")
expect_eq "注入现场：供应商事实表含 URL 的行数" 3 "${injected}"

# --- 3) 源库指纹（备份前） ---------------------------------------------------
echo
echo "-- 3) 备份前：源库与 blob 指纹"
fingerprint() {
  {
    shasum -a 256 "${LIVE}"
    sqlite3 "${LIVE}" .dump | shasum -a 256
    find ./data/blobs -type f | sort | xargs shasum -a 256
  }
}
fingerprint > before.txt
echo "  库文件 sha256 = $(head -1 before.txt | awk '{print $1}')"

# --- 4) backup ---------------------------------------------------------------
echo
echo "-- 4) backup（修复后二进制）"
"${BIN}" backup --data-dir ./data --out ./backup > backup.log 2>&1
expect_eq "backup 退出码" 0 "$?"
grep -E "临时 URL 脱敏|tempUrlsRedacted|脱敏" backup.log | sed 's/^/  /' | head -3
if grep -q "脱敏" backup.log; then ok "backup 输出报告脱敏处数"; else bad "backup 输出缺少脱敏报告"; fi

# --- 5) 源库不变 -------------------------------------------------------------
echo
echo "-- 5) 备份后：源库指纹必须不变（无就地 UPDATE）"
fingerprint > after.txt
if diff -q before.txt after.txt >/dev/null; then ok "源库文件/dump/blob 指纹逐行一致（未被改写）"; else bad "源库被改写"; diff before.txt after.txt | head -5; fi
expect_eq "源库仍含注入的 URL（不迁移）" 3 "${injected}"

# --- 6) 快照扫描 -------------------------------------------------------------
echo
echo "-- 6) 快照扫描（判据：'://'、'sign='、path、canary）"
SNAP=./backup/database/manual.sqlite3
sqlite3 "${SNAP}" ".dump" > snapshot-dump.sql
python3 - "${SNAP}" "${CANARY}" "${USER_CANARY}" > scan-snapshot.txt 2>&1 <<'PY'

import sqlite3, sys, json
db, canary, user_canary = sys.argv[1], sys.argv[2], sys.argv[3]
con = sqlite3.connect(db)
needles = ['://', 'sign=', '/qa-r26-hist/model.glb', '/qa-r26-hist/preview.png', canary]
bad = 0
print("== 供应商事实表全部文本列 × 判据")
for table in ("job_stages", "provider_attempts"):
    cols = [row[1] for row in con.execute(f"pragma table_info({table})")]
    for col in cols:
        for needle in needles:
            n = con.execute(f'select count(*) from "{table}" where instr(cast("{col}" as text), ?) > 0',
                            (needle,)).fetchone()[0]
            if n:
                bad += 1
                print(f"  !! {table}.{col} 命中 {needle!r}: {n}")
print("  （无命中行 = 全部为 0）")
print("== 快照内 tripo_poll.usage_json（应只含摘要 + task id/计费）")
usage = con.execute("select cast(usage_json as text) from job_stages where stage_kind='tripo_poll'").fetchone()[0]
print(" ", usage)
data = json.loads(usage)
assert data["remoteTaskId"] == "t20-fixture-task-0001", data
assert data["modelUrl"]["redacted"] is True and data["modelUrl"]["host"] == "cdn.example.invalid", data
assert len(data["modelUrl"]["sha256"]) == 16, data
assert data["billing"]["creditMinor"] == 3000, data
print("== 摘要（redacted/host/sha256）+ task id + 计费保留 ✔")
print("== 非 JSON 文本列 last_error 的兜底结果")
for table in ("job_stages", "provider_attempts"):
    for (rowid, value) in con.execute(f"select rowid, coalesce(last_error,'') from {table} where last_error is not null and last_error <> ''"):
        if 'sha256=' in value or table == 'job_stages':
            print(f"  {table}[{rowid}] = {value[:200]}")
print("== 用户出处链接（documents.source_url）必须原样保留（并不过度清洗）")
kept = con.execute("select coalesce(source_url,'') from documents").fetchone()[0]
print(" ", kept)
assert user_canary in kept, kept
if bad:
    print(f"!! 快照命中 {bad} 处")
    sys.exit(3)
PY
scan_rc=$?
cat scan-snapshot.txt
if [[ ${scan_rc} -eq 0 ]]; then ok "快照：供应商事实 0 命中、摘要保留、用户出处链接保留"; else bad "快照扫描有命中或用户链接被清掉（rc=${scan_rc}）"; fi

echo
echo "-- 6b) 快照 manifest / SHA256SUMS"
expect_eq "快照 sha256 == manifest.database.sha256" \
  "$(python3 -c "import json;print(json.load(open('./backup/manifest.json'))['database']['sha256'])")" \
  "$(shasum -a 256 ./backup/database/manual.sqlite3 | awk '{print $1}')"
python3 -c "
import json
notes = json.load(open('./backup/manifest.json'))['notes']
print('  manifest notes:'); [print('   -', n) for n in notes]
assert any('脱敏' in n for n in notes), 'manifest notes 必须说明脱敏'
print('  ✔ notes 含脱敏说明')
" || bad "manifest notes 未说明脱敏"
( cd ./backup && shasum -a 256 -c SHA256SUMS > /dev/null 2>&1 ) && ok "shasum -a 256 -c SHA256SUMS 全部 OK" || bad "SHA256SUMS 校验失败"

# --- 7) restore 快照 → 新目录 -------------------------------------------------
echo
echo "-- 7) restore 新快照 → ./data2（恢复后仍可用）"
"${BIN}" restore --from ./backup --data-dir ./data2 > restore2.log 2>&1
expect_eq "restore（新快照）退出码" 0 "$?"
python3 - "${LIVE}" ./data2/manual.sqlite3 <<'PY'
import sqlite3, sys
src, dst = sys.argv[1], sys.argv[2]
tables = ["items", "jobs", "job_stages", "provider_attempts", "assets", "blobs",
          "pages", "documents", "preparations", "photos", "manual_releases", "model_revisions"]
print("== 行数对比（源 → 恢复后）")
for table in tables:
    a = sqlite3.connect(src).execute(f"select count(*) from {table}").fetchone()[0]
    b = sqlite3.connect(dst).execute(f"select count(*) from {table}").fetchone()[0]
    mark = "✔" if a == b else "!!"
    print(f"  {mark} {table}: {a} → {b}")
needles = ['://', 'sign=', 'qa-r26-hist-9c4d']
hits = 0
con = sqlite3.connect(dst)
for table in ("job_stages", "provider_attempts"):
    cols = [row[1] for row in con.execute(f"pragma table_info({table})")]
    for col in cols:
        for needle in needles:
            hits += con.execute(f'select count(*) from "{table}" where instr(cast("{col}" as text), ?) > 0', (needle,)).fetchone()[0]
print(f"== 恢复库供应商事实表 URL 命中 = {hits}（应为 0）")
assert hits == 0
print("== 恢复库用户出处链接保留：", con.execute("select source_url from documents").fetchone()[0])
PY
if [[ $? -eq 0 ]]; then ok "恢复库：行数保留、供应商事实 0 命中、用户链接保留"; else bad "恢复库检查失败（见上）"; fi

# --- 8) serve（恢复库）+ 任务详情 + 导出 --------------------------------------
echo
echo "-- 8) serve 恢复库 → 登录 → 任务详情 → 导出"
printf '%s' "${PW}" > pw.txt && chmod 600 pw.txt
printf '{"password":"%s"}' "${PW}" > body.json
QA_TRIPO_KEY="qa-r26-fake" QA_MANUAL_AI_KEY="qa-r26-fake" \
  "${BIN}" serve --data-dir ./data2 --listen "127.0.0.1:${APP_PORT}" > serve2.log 2>&1 &
SERVE_PIDS="${SERVE_PIDS} $!"
SERVE_PID=$!
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
BASE2="http://127.0.0.1:${APP_PORT}/api/v1"
code=$(curl -s -o login2.json -w '%{http_code}' -c jar2 -H 'content-type: application/json' \
  --data-binary "@body.json" "${BASE2}/auth/login")
expect_eq "恢复后可登录（口令哈希保留）" 200 "${code}"
JOB_ID=$(python3 -c "import sqlite3;print(sqlite3.connect('./data2/manual.sqlite3').execute('select id from jobs').fetchone()[0])")
code=$(curl -s -o job-detail.json -w '%{http_code}' -b jar2 "${BASE2}/jobs/${JOB_ID}")
expect_eq "GET /jobs/{id}" 200 "${code}"
for needle in '://' 'sign=' "${CANARY}" '/qa-r26-hist/'; do
  expect_eq "恢复库任务详情不含 '${needle}'" 0 "$(hits job-detail.json "$needle")"
done
python3 - job-detail.json <<'PY'
import json, sys
detail = json.load(open(sys.argv[1], encoding="utf-8"))["data"]
for stage in detail["stages"]:
    if stage["stageKind"] == "tripo_poll":
        print("  usage.modelUrl =", json.dumps(stage["usage"]["modelUrl"], ensure_ascii=False))
        print("  usage.remoteTaskId =", json.dumps(stage["usage"]["remoteTaskId"]))
PY

code=$(curl -s -o export.zip -w '%{http_code}' -b jar2 "${BASE2}/releases/${RELEASE_ID}/export")
expect_eq "GET /releases/{id}/export" 200 "${code}"
python3 - export.zip <<'PY' | tee export-scan.txt
import sys, zipfile
needles = [b"://", b"sign=", b"qa-r26-hist-9c4d", b"qa-r26-user-link-3ad9", b"cdn.example.invalid"]
hits = 0
with zipfile.ZipFile(sys.argv[1]) as archive:
    for name in archive.namelist():
        data = archive.read(name)
        for needle in needles:
            n = data.count(needle)
            if n:
                print(f"  !! {name} 命中 {needle!r}: {n}")
                hits += n
print(f"== 导出包 URL/签名 canary 命中 = {hits}（应为 0）")
print("   条目：", ", ".join(sorted(zipfile.ZipFile(sys.argv[1]).namelist())))
PY
if grep -q '!!' export-scan.txt; then bad "导出包含 URL/签名"; else ok "导出包干净（历史 URL 与用户链接都未进入）"; fi

# PDF / GLB 可读（恢复后同一 release 资产字节一致）
DOC_SHA=$(python3 -c "import sqlite3;print(sqlite3.connect('./data2/manual.sqlite3').execute(\"select blob_id from assets where purpose='document'\").fetchone()[0])")
MODEL_SHA=$(python3 -c "import json;print(json.load(open('${ROOT}/artifacts/web-mvp/t20-rd/rehearsal-info.json'))['modelSha256'])")
for name in "${DOC_SHA}" "${MODEL_SHA}"; do
  got=$(shasum -a 256 "./data2/blobs/${name:0:2}/${name}" | awk '{print $1}')
  expect_eq "恢复库 blob ${name:0:8}… 字节一致" "${name}" "${got}"
done

# --- 8b) 恢复后的历史库仍按 task_id 继续查询（不重新购买） --------------------
echo
echo "-- 8b) 恢复库 + 本机 fixture：下载阶段按 task_id 重新查询（免费 GET，无付费 POST）"
FIXTURE_PORT=18131
python3 "${QA_DIR}/qa-r26-tripo-fixture.py" --port "${FIXTURE_PORT}" --log ./fixture2.jsonl --scenario happy > fixture2.out 2>&1 &
FIXTURE_PID=$!
for _ in $(seq 1 100); do grep -q "listening" fixture2.out 2>/dev/null && break; sleep 0.1; done
kill -9 "${SERVE_PID}" 2>/dev/null; wait "${SERVE_PID}" 2>/dev/null
# 让下载阶段重新入队（清掉本地副本引用：模拟"需要重新取链接"的恢复现场）
python3 - <<'PY'
import sqlite3
con = sqlite3.connect('./data2/manual.sqlite3')
cur = con.execute(
    "update job_stages set status='queued', next_run_at=NULL, last_error=NULL, "
    "needs_input_json=NULL, result_asset_id=NULL where stage_kind='model_download'")
assert cur.rowcount == 1, cur.rowcount
con.commit()
print("  model_download 已重置为 queued 且清空本地副本引用（用例前提）")
PY
cat > ./data2/config.toml <<TOML
[providers.tripo]
base_url = "http://127.0.0.1:${FIXTURE_PORT}/v3"
api_key_env = "QA_TRIPO_KEY"
TOML
QA_TRIPO_KEY="qa-r26-fake" \
  "${BIN}" serve --data-dir ./data2 --listen "127.0.0.1:${APP_PORT}" > serve2b.log 2>&1 &
SERVE_PID=$!
SERVE_PIDS="${SERVE_PIDS} ${SERVE_PID}"
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
sleep 10
polls=$(grep -c '"event": "poll"' ./fixture2.jsonl 2>/dev/null)
submits=$(grep -c '"event": "submit"' ./fixture2.jsonl 2>/dev/null)
polls=${polls:-0}; submits=${submits:-0}
echo "  fixture 计数：poll=${polls}（期望 ≥1，按 task_id 重查）；submit=${submits}（期望 0）"
if [[ "${polls}" -ge 1 ]]; then ok "恢复库按 task_id 重新查询（免费 GET）"; else bad "恢复库未重新查询"; fi
expect_eq "恢复路径没有付费 POST" 0 "${submits}"
python3 - <<'PY'
import sqlite3
con = sqlite3.connect('./data2/manual.sqlite3')
row = con.execute("select status, coalesce(last_error,'') from job_stages where stage_kind='model_download'").fetchone()
print(f"  下载阶段状态={row[0]}；last_error={row[1][:120]}")
PY
kill -9 "${FIXTURE_PID}" 2>/dev/null
kill -9 "${SERVE_PID}" 2>/dev/null

# --- 9) fail-closed：把 URL 注入导出清单"生成字段"的来源 ----------------------
echo
echo "-- 9) fail-closed（导出）：注入 URL → 导出必须失败且不出包"
cp -R ./data2 ./data3
python3 - "./data3" "${CANARY}" <<'PY'
import hashlib, json, os, sqlite3, sys
data_dir, canary = sys.argv[1], sys.argv[2]
con = sqlite3.connect(os.path.join(data_dir, "manual.sqlite3"))
asset_id, sha, mime = con.execute(
    "select a.id, b.sha256, b.mime from assets a join blobs b on b.sha256 = a.blob_id "
    "where a.purpose='release_manifest'").fetchone()
path = os.path.join(data_dir, "blobs", sha[:2], sha)
manifest = json.load(open(path, encoding="utf-8"))
# 把 URL 写进"生成字段"的来源（历史/未来版本可能这么写；fail-closed 必须拦住）
manifest["assets"][0]["source"] = f"https://cdn.example.invalid/qa-r26-crafted/victim.glb?sign={canary}"
raw = json.dumps(manifest, ensure_ascii=False).encode("utf-8")
new_sha = hashlib.sha256(raw).hexdigest()
os.makedirs(os.path.join(data_dir, "blobs", new_sha[:2]), exist_ok=True)
open(os.path.join(data_dir, "blobs", new_sha[:2], new_sha), "wb").write(raw)
con.execute("insert or replace into blobs (sha256, size, mime, storage_state, created_at) values (?,?,?,?,?)",
            (new_sha, len(raw), mime, "stored", 1789233087241))
con.execute("update assets set blob_id = ? where id = ?", (new_sha, asset_id))
con.commit()
print(f"  注入完成：新 manifest sha={new_sha[:12]}…（asset={asset_id}）")
PY
QA_TRIPO_KEY="qa-r26-fake" QA_MANUAL_AI_KEY="qa-r26-fake" \
  "${BIN}" serve --data-dir ./data3 --listen "127.0.0.1:${APP2_PORT}" > serve3.log 2>&1 &
SERVE3_PID=$!
SERVE_PIDS="${SERVE_PIDS} ${SERVE3_PID}"
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:${APP2_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
BASE3="http://127.0.0.1:${APP2_PORT}/api/v1"
curl -s -o login3.json -c jar3 -H 'content-type: application/json' --data-binary "@body.json" "${BASE3}/auth/login"
code=$(curl -s -o export-crafted.bin -w '%{http_code}' -b jar3 "${BASE3}/releases/${RELEASE_ID}/export")
echo "  导出 HTTP 状态 = ${code}（期望 5xx，拒绝而不是带出链接）"
if [[ "${code}" == 5* ]]; then ok "注入 URL 后导出被拒绝（fail-closed）"; else bad "注入 URL 后导出未拒绝（status=${code}）"; fi
if head -c 2 export-crafted.bin 2>/dev/null | grep -q "PK"; then bad "拒绝时不应产生 ZIP 包（响应体是错误 JSON）"; else ok "拒绝时未产生 ZIP 包（响应体为错误 JSON，非 zip）"; fi
if grep -q "export_manifest_url_forbidden" serve3.log; then ok "服务端日志给出 export_manifest_url_forbidden"; else bad "服务端日志缺少 export_manifest_url_forbidden"; fi
for needle in "${CANARY}" 'sign='; do
  expect_eq "拒绝响应体不含 '${needle}'" 0 "$(hits export-crafted.bin "$needle")"
done
kill -9 "${SERVE3_PID}" 2>/dev/null

# 证据留存
mkdir -p "${QA_DIR}/work-historical"
cp before.txt after.txt scan-snapshot.txt export-scan.txt "${QA_DIR}/work-historical/" 2>/dev/null
cp job-detail.json backup.log serve2.log serve3.log "${QA_DIR}/work-historical/" 2>/dev/null
sqlite3 ./backup/database/manual.sqlite3 ".dump" > "${QA_DIR}/work-historical/snapshot-dump.sql" 2>/dev/null

echo
echo "== 历史数据剧本结果：$([[ ${RESULT} = 0 ]] && echo PASS || echo FAIL)"
exit ${RESULT}
