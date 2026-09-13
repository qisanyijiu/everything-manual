#!/bin/bash
# RD R29 自查 · BUG-011/BUG-012 修复后的 dist 全链路 canary 扫描（RD 自建，独立于 QA 脚本）。
#
# 链路： restore 归档样例 → 注入产品可达形态的 canary（stage 三条文本列 + attempt）
#        → backup（快照扫描：URL 形态 0 命中；JSON 列**逐字符串值**脱敏、
#                  句子与同列其它条目完整保留、message 仍是字符串）
#        → restore → serve → 任务详情 DTO（needsInput 不得为空；usage.errorSummary 是字符串）
#        → 导出发布包（白名单产物 0 命中）→ serve 日志（canary 0 命中、零外网）
# 全程只连 127.0.0.1；零外网、零付费。
#
# 用法：bash verify-redaction-dist.sh <dist 二进制绝对路径>
set -uo pipefail

BIN="${1:?用法: verify-redaction-dist.sh <绝对路径的 dist 二进制>}"
RD_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${RD_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
INFO="${ROOT}/artifacts/web-mvp/t20-rd/rehearsal-info.json"
WORK="$(mktemp -d /tmp/em-r29-rdverify-XXXXXX)"
APP_PORT=18429
RESULT=0
SERVER_PID=""

ok()  { echo "  [通过] $1"; }
bad() { echo "  [失败] $1"; RESULT=1; }
expect_eq() { if [[ "$2" == "$3" ]]; then ok "$1（=$3）"; else bad "$1（期望 $2，实际 $3）"; fi; }
hits() { grep -c -- "$2" "$1" 2>/dev/null | head -1; }

cleanup() {
  [[ -n "${SERVER_PID}" ]] && kill -9 "${SERVER_PID}" 2>/dev/null
  wait 2>/dev/null
  rm -rf "${WORK}"
}
trap cleanup EXIT

PW=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['password'])" "${INFO}")
mkdir -p "${WORK}"
cd "${WORK}" || exit 1

echo "== RD R29 自查 · 脱敏全链路（dist）"
echo "   二进制 ${BIN}"
echo "   sha256 $(shasum -a 256 "${BIN}" | awk '{print $1}')"
echo "   工作目录 ${WORK}"

echo
echo "-- 1) restore 归档样例 → ./data，注入 canary（产品可达形态）"
"${BIN}" restore --from "${SAMPLE}" --data-dir ./data > restore.log 2>&1
expect_eq "restore 退出码" 0 "$?"

python3 - <<'PY'
import json, sqlite3

HOST = "cdn.rd29.invalid"
CASES = {
    "stage":   f"模型下载传输失败（下载可安全重试；不产生供应商费用）：连接失败：error sending request "
               f"for url (https://{HOST}/m.glb?sign=RD29-STAGE-CANARY-1d55&expires=9999999999)",
    "attempt": f"链接 https://{HOST}/x.glb?sign=RD29-ATTEMPT-CANARY-6f38 已过期，请重试",
}
USAGE = {
    "batchIndex": 0,
    "outcome": "refusal",
    "remoteTaskId": "task-rd29-usage",
    "billing": {"creditMinor": 3000},
    "errorSummary": f"模型拒答（refusal）：下载参考 https://{HOST}/p.png?sign=RD29-USAGE-CANARY-4e71 也失败",
}
NEEDS = [
    {"code": "download_insecure_scheme",
     "message": f"模型下载必须使用 HTTPS（实际 http://{HOST}/m.glb?sign=RD29-NEEDS-CANARY-8c02）：拒绝下载"},
    {"code": "retry", "message": "下载可安全重试（task-9）"},
]
con = sqlite3.connect('./data/manual.sqlite3')
job = con.execute("select job_id from job_stages where stage_kind='model_download' limit 1").fetchone()[0]
con.execute("update job_stages set status='needs_input', last_error=?, needs_input_json=?, usage_json=? "
            "where job_id=? and stage_kind='model_download'",
            (CASES["stage"], json.dumps(NEEDS, ensure_ascii=False), json.dumps(USAGE, ensure_ascii=False), job))
con.execute("update provider_attempts set last_error=? where job_id=? and submit_state='accepted'",
            (CASES["attempt"], job))
con.commit()
print(f"  注入 job={job}")
print(f"  stage.last_error = {CASES['stage'][:110]}…")
print(f"  attempt.last_error = {CASES['attempt']}")
print(f"  usage.errorSummary = {USAGE['errorSummary']}")
print(f"  needs_input_json = {json.dumps(NEEDS, ensure_ascii=False)}")
open('job_id.txt', 'w').write(job)
PY
ok "注入完成（last_error / needs_input_json / usage_json / attempt.last_error 四处）"

echo
echo "-- 2) 源库指纹（供"备份不改写源库"对比）"
sqlite3 ./data/manual.sqlite3 ".dump" > source-before.sql

echo
echo "-- 3) backup → 快照扫描（0 命中 + JSON 结构感知）"
"${BIN}" backup --data-dir ./data --out ./backup > backup.log 2>&1
expect_eq "backup 退出码" 0 "$?"
python3 - <<'PY' | tee snapshot-scan.txt
import json, sqlite3
con = sqlite3.connect('./backup/database/manual.sqlite3')
# 判据沿用 QA 回合 26/27 表：`://`/`sign=`/canary 命中即泄露；**裸 host 允许保留**
# （ADR-033：摘要标签里的 host=… 是最小诊断信息），故 host 不入 DB 扫描判据。
needles = ['://', 'sign=', 'RD29-STAGE-CANARY-1d55', 'RD29-ATTEMPT-CANARY-6f38',
           'RD29-USAGE-CANARY-4e71', 'RD29-NEEDS-CANARY-8c02']
print("== 快照逐表逐列扫描（job_stages / provider_attempts 全部列 × 判据）")
bad = 0
for table in ("job_stages", "provider_attempts"):
    cols = [row[1] for row in con.execute(f"pragma table_info({table})")]
    for col in cols:
        for needle in needles:
            n = con.execute(f'select count(*) from "{table}" where instr(cast("{col}" as text), ?) > 0',
                            (needle,)).fetchone()[0]
            if n:
                bad += 1
                print(f"  !! {table}.{col} 命中 {needle!r}: {n}")
print(f"== 判据命中合计 = {bad}")

row = con.execute("select usage_json from job_stages where stage_kind='model_download'").fetchone()[0]
usage = json.loads(row)
summary = usage["errorSummary"]
print("== usage.errorSummary 类型 =", type(summary).__name__, "| 内容 =", summary)
print("== usage.remoteTaskId =", usage.get("remoteTaskId"), "| billing =", usage.get("billing"))
row = con.execute("select needs_input_json from job_stages where stage_kind='model_download'").fetchone()[0]
items = json.loads(row)
print("== needsInput 条数 =", len(items))
for item in items:
    print("   -", item["code"], "|", item["message"])
open('snapshot-json-view.txt', 'w').write("\n".join([
    f"usage.errorSummary={summary}",
    f"needsInput={json.dumps(items, ensure_ascii=False)}",
]))
PY
expect_eq "快照判据命中合计" 0 "$(grep -o '命中合计 = [0-9]*' snapshot-scan.txt | grep -o '[0-9]*$')"

python3 - <<'PY' && ok "快照 JSON 列：句子保留、字符串类型保持（BUG-011/BUG-012）" || bad "快照 JSON 列形态不符合预期"
import json
view = open('snapshot-json-view.txt').read()
usage_line = [line for line in view.splitlines() if line.startswith('usage.errorSummary=')][0]
needs_line = [line for line in view.splitlines() if line.startswith('needsInput=')][0]
summary = usage_line.split('=', 1)[1]
assert '://' not in summary, summary
assert summary.endswith('也失败'), summary
assert 'host=cdn.rd29.invalid' in summary, summary
assert 'task-rd29-usage' not in summary  # 摘要里不该混入其它字段（防串）
items = json.loads(needs_line.split('=', 1)[1])
assert len(items) == 2, items                     # 同列其它条目不得丢失
assert isinstance(items[0]['message'], str)       # message 仍是字符串（不得变对象）
assert items[0]['message'].endswith('）：拒绝下载'), items[0]
assert '://' not in items[0]['message'], items[0]
assert items[1]['message'] == '下载可安全重试（task-9）', items[1]
PY

echo
echo "-- 4) 快照完整性与源库不变"
manifest_sha=$(python3 -c "import json;print(json.load(open('./backup/manifest.json'))['database']['sha256'])")
actual_sha=$(shasum -a 256 ./backup/database/manual.sqlite3 | awk '{print $1}')
expect_eq "快照 db sha256 == manifest.database.sha256" "${manifest_sha}" "${actual_sha}"
( cd ./backup && shasum -a 256 -c SHA256SUMS > ../sha-check.log 2>&1 ) && ok "SHA256SUMS 全部 OK" || bad "SHA256SUMS 校验失败"
sqlite3 ./data/manual.sqlite3 ".dump" > source-after.sql
expect_eq "源库未被备份改写（dump 前后 sha256）" \
  "$(shasum -a 256 source-before.sql | awk '{print $1}')" \
  "$(shasum -a 256 source-after.sql | awk '{print $1}')"
expect_eq "源库仍保留注入的 canary（未就地迁移；5 处：stage 行 3 + 两条 attempt 行各 1）" 5 \
  "$(grep -o 'cdn.rd29.invalid' source-after.sql | wc -l | tr -d ' ')"

echo
echo "-- 5) restore → serve → 任务详情 DTO（needsInput 不得为空）"
"${BIN}" restore --from ./backup --data-dir ./data2 > restore2.log 2>&1
expect_eq "restore（快照）退出码" 0 "$?"
printf '{"password":"%s"}' "${PW}" > body.json
"${BIN}" serve --data-dir ./data2 --listen "127.0.0.1:${APP_PORT}" > serve.log 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
BASE="http://127.0.0.1:${APP_PORT}/api/v1"
curl -s -c jar -H 'content-type: application/json' --data-binary "@body.json" "${BASE}/auth/login" -o login.json
JOB=$(cat job_id.txt)
expect_eq "任务详情 HTTP 状态" 200 "$(curl -s -b jar -o job-detail.json -w '%{http_code}' "${BASE}/jobs/${JOB}")"
for needle in '://' 'sign=' 'RD29-STAGE-CANARY-1d55' 'RD29-ATTEMPT-CANARY-6f38' \
              'RD29-USAGE-CANARY-4e71' 'RD29-NEEDS-CANARY-8c02'; do
  expect_eq "恢复库详情不含 '${needle}'" 0 "$(hits job-detail.json "$needle")"
done
python3 - <<'PY' | tee dto-view.txt
import json
body = json.load(open('job-detail.json'))
stage = [s for s in body['data']['stages'] if s['stageKind'] == 'model_download'][0]
print("== DTO stage.status =", stage['status'])
print("== DTO needsInput 条数 =", len(stage['needsInput']))
print("== DTO needsInput =", json.dumps(stage['needsInput'], ensure_ascii=False))
print("== DTO usage.errorSummary =", stage['usage'].get('errorSummary'))
attempts = [a['lastError'] for a in body['data']['attempts'] if a.get('lastError')]
print("== DTO attempts.lastError =", attempts)
assert len(stage['needsInput']) == 2, "恢复后缺项不得静默为空"
assert stage['needsInput'][0]['message'].endswith('）：拒绝下载'), stage['needsInput'][0]
assert stage['needsInput'][1]['message'] == '下载可安全重试（task-9）'
assert isinstance(stage['usage'].get('errorSummary'), str)
PY
[[ $? -eq 0 ]] && ok "恢复库 DTO：缺项完整、errorSummary 是字符串、尾部文本保留" || bad "恢复库 DTO 形态不符合预期"

echo
echo "-- 5b) 导出兜底（发布包 0 命中）"
RELEASE_ID=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['releaseId'])" "${INFO}")
code=$(curl -s -b jar -o export.zip -w '%{http_code}' "${BASE}/releases/${RELEASE_ID}/export")
expect_eq "导出 HTTP 状态" 200 "${code}"
if [[ "$(head -c 2 export.zip)" == "PK" ]]; then ok "导出产物是 ZIP（PK magic）"; else bad "导出产物不是 ZIP"; fi
rm -rf export && mkdir -p export && (cd export && unzip -qq ../export.zip)
export_hits=0
for needle in 'RD29-STAGE-CANARY-1d55' 'RD29-ATTEMPT-CANARY-6f38' 'RD29-USAGE-CANARY-4e71' 'RD29-NEEDS-CANARY-8c02'; do
  n=$(grep -rIl -- "$needle" export 2>/dev/null | wc -l | tr -d ' ')
  [[ "${n}" != "0" ]] && { bad "导出包命中 '${needle}'（${n} 个文件）"; export_hits=$((export_hits+1)); }
done
[[ "${export_hits}" == "0" ]] && ok "导出包 canary 0 命中"

echo
echo "-- 6) serve 日志核查"
for needle in 'RD29-STAGE-CANARY-1d55' 'RD29-ATTEMPT-CANARY-6f38' 'RD29-USAGE-CANARY-4e71' \
              'RD29-NEEDS-CANARY-8c02' 'sign=' 'cdn.rd29.invalid'; do
  expect_eq "serve 日志不含 '${needle}'" 0 "$(hits serve.log "$needle")"
done
allowed=$(grep "://" serve.log | grep -c -e "listening on http://" -e '"baseUrl"')
total_url=$(grep -c "://" serve.log)
expect_eq "serve 日志 '://' 只来自监听横幅/baseUrl 回显" "${allowed}" "${total_url}"
nonloopback=$(grep -o 'https\?://[a-zA-Z0-9.:_-]*' serve.log | grep -v "127.0.0.1" | grep -c .)
expect_eq "serve 日志里非回环主机数（零外网门禁）" 0 "${nonloopback}"

mkdir -p "${RD_DIR}/work-dist"
cp snapshot-scan.txt snapshot-json-view.txt dto-view.txt job-detail.json backup.log serve.log source-after.sql "${RD_DIR}/work-dist/" 2>/dev/null

echo
echo "== RD R29 dist 自查结果：$([[ ${RESULT} = 0 ]] && echo PASS || echo FAIL)"
exit ${RESULT}
