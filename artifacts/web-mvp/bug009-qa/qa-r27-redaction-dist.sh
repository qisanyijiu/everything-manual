#!/bin/bash
# QA 回合 27 · BUG-009/BUG-010 复验（dist 二进制；读写两侧 + 备份兜底）。
#
# 目的（不采信 RD 脚本，QA 自建）：
#   1) 读取侧兜底：把**修复前形态**的签名 URL 注入 job_stages.last_error、
#      job_stages.needs_input_json、provider_attempts.last_error（历史行现场）→
#      真实 dist `serve` → 任务详情 DTO 不得出 `://`/签名，且摘要标签 + 尾部文本保留。
#   2) 写侧兜底（备份）：`backup` 快照里同三处 0 命中；URL 后紧邻中文/标点/引号/换行的
#      文本**不得被吞**（BUG-010）；源库不被改写；快照哈希与 manifest 一致。
#   3) 日志：serve 日志里 canary 0 命中；'://' 只来自监听横幅/baseUrl 回显。
# 全程只连 127.0.0.1；零外网、零付费。
#
# 用法：bash qa-r27-redaction-dist.sh <dist 二进制绝对路径>
set -uo pipefail

BIN="${1:?用法: qa-r27-redaction-dist.sh <绝对路径的 dist 二进制>}"
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
INFO="${ROOT}/artifacts/web-mvp/t20-rd/rehearsal-info.json"
WORK="$(mktemp -d /tmp/em-r27-redaction-XXXXXX)"
APP_PORT=18427
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

echo "== QA 回合 27 · 脱敏复验（dist）"
echo "   二进制 ${BIN}"
echo "   sha256 $(shasum -a 256 "${BIN}" | awk '{print $1}')"
echo "   工作目录 ${WORK}"

echo
echo "-- 1) restore 归档样例 → ./data，并注入修复前形态的签名 URL（QA 自造历史行现场）"
"${BIN}" restore --from "${SAMPLE}" --data-dir ./data > restore.log 2>&1
expect_eq "restore 退出码" 0 "$?"

python3 - <<'PY'
import hashlib, json, sqlite3

SCHEME = "https"
HOST = "cdn.qa-r27.invalid"
CASES = {
    "stage":  f"模型下载传输失败（下载可安全重试；不产生供应商费用）：连接失败：error sending request "
              f"for url ({SCHEME}://{HOST}/model.glb?sign=QA27-STAGE-CANARY-3f9a&expires=9999999999)",
    "needs":  f"模型文件超过上限：请更换资料（原始链接 {SCHEME}://{HOST}/big.glb?sign=QA27-NEEDS-CANARY-6d21）",
    "attempt": " | ".join([
        f"链接 {SCHEME}://{HOST}/x.glb?sign=QA27-ATTEMPT-CANARY-7c2d已过期，请重试",
        f"见 '{SCHEME}://{HOST}/y.glb?sign=QA27-QUOTE-CANARY-2e18' 后重试",
        f"下一行 {SCHEME}://{HOST}/z.glb?sign=QA27-NL-CANARY-5a4f：以及结束",
        f"结尾 {SCHEME}://{HOST}/w.glb?sign=QA27-PUNCT-CANARY-8b0e，结束",
    ]),
}
con = sqlite3.connect('./data/manual.sqlite3')
job = con.execute("select job_id from job_stages where stage_kind='model_download' limit 1").fetchone()[0]
con.execute("update job_stages set last_error=? where job_id=? and stage_kind='model_download'",
            (CASES["stage"], job))
con.execute("update job_stages set needs_input_json=? where job_id=? and stage_kind='model_download'",
            (json.dumps([{"code": "download_too_large", "message": CASES["needs"]}], ensure_ascii=False), job))
con.execute("update provider_attempts set last_error=? where job_id=? and submit_state='accepted'",
            (CASES["attempt"], job))
con.commit()
open('job_id.txt', 'w').write(job)
open('injected.json', 'w').write(json.dumps(CASES, ensure_ascii=False, indent=2))
print(f"  注入 job={job}")
print(f"  stage.last_error = {CASES['stage'][:120]}…")
print(f"  attempt.last_error（4 个边界样例）= {CASES['attempt']}")
PY
ok "注入完成（3 处：stage.last_error / needs_input_json / attempt.last_error）"

echo
echo "-- 2) 源库指纹（供"备份不改写源库"对比）"
sqlite3 ./data/manual.sqlite3 ".dump" > source-before.sql

echo
echo "-- 3) backup → 快照扫描（写侧兜底 + BUG-010 尾部保留）"
"${BIN}" backup --data-dir ./data --out ./backup > backup.log 2>&1
expect_eq "backup 退出码" 0 "$?"
sqlite3 ./backup/database/manual.sqlite3 ".dump" > snapshot.sql
python3 - <<'PY' | tee snapshot-scan.txt
import sqlite3
con = sqlite3.connect('./backup/database/manual.sqlite3')
needles = ['://', 'sign=', 'QA27-STAGE-CANARY-3f9a', 'QA27-NEEDS-CANARY-6d21',
           'QA27-ATTEMPT-CANARY-7c2d', 'QA27-QUOTE-CANARY-2e18',
           'QA27-NL-CANARY-5a4f', 'QA27-PUNCT-CANARY-8b0e']
print("== 快照逐表逐列扫描（job_stages / provider_attempts 全部文本列 × 全部判据）")
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
print("  （无 !! 行 = 0 命中）")
print(f"== URL 形态命中合计 = {bad}")
stage = con.execute("select last_error from job_stages where instr(last_error,'已脱敏')>0 limit 1").fetchone()
attempt = con.execute("select last_error from provider_attempts where instr(last_error,'已脱敏')>0 limit 1").fetchone()
needs = con.execute("select needs_input_json from job_stages where instr(needs_input_json,'://')>0 limit 1").fetchone()
print("== 快照内文本（stage）= ", stage[0] if stage else "(缺)")
print("== 快照内文本（attempt）= ", attempt[0] if attempt else "(缺)")
print("== 快照内 needs_input_json（非 JSON 列扫描口径；JSON 列走对象替换，见 BUG-011）= ",
      needs[0] if needs else "(该列已无 '://')")
PY
if grep -q '^  !! ' snapshot-scan.txt; then bad "快照扫描有命中（写侧兜底失败）"; else ok "快照 job_stages/provider_attempts 全部文本列 0 命中"; fi
expect_eq "快照 URL 形态命中合计" 0 "$(grep -o '命中合计 = [0-9]*' snapshot-scan.txt | grep -o '[0-9]*$')"
label_hits=$(hits snapshot.sql '临时供应商地址已脱敏')
if [[ "${label_hits}" -ge 3 ]]; then ok "快照摘要标签出现（3 处注入 → ${label_hits} 行）"; else bad "快照摘要标签不足（${label_hits} 行）"; fi

echo
echo "-- 4) BUG-010 逐片段核对（URL 之后紧邻的文本不得被吞）"
snapshot_text=$(python3 -c "
import sqlite3
con = sqlite3.connect('./backup/database/manual.sqlite3')
print(con.execute(\"select last_error from provider_attempts where instr(last_error,'已脱敏')>0 limit 1\").fetchone()[0])
print(con.execute(\"select last_error from job_stages where instr(last_error,'已脱敏')>0 limit 1\").fetchone()[0])
row = con.execute(\"select needs_input_json from job_stages where instr(needs_input_json,'已脱敏')>0 limit 1\").fetchone()
print(row[0] if row else '(needs_input_json 走 JSON 对象替换；BUG-011 现场见 qa-r27-bug011-evidence.sh)')
")
for fragment in "已过期，请重试" "' 后重试" "：以及结束" "，结束"; do
  if grep -qF -- "$fragment" <<<"${snapshot_text}"; then ok "尾部文本保留：${fragment}"; else bad "尾部文本被吞：${fragment}"; fi
done

echo
echo "-- 5) 快照完整性与源库不变"
manifest_sha=$(python3 -c "import json;print(json.load(open('./backup/manifest.json'))['database']['sha256'])")
actual_sha=$(shasum -a 256 ./backup/database/manual.sqlite3 | awk '{print $1}')
expect_eq "快照 db sha256 == manifest.database.sha256" "${manifest_sha}" "${actual_sha}"
( cd ./backup && shasum -a 256 -c SHA256SUMS > ../sha-check.log 2>&1 ) && ok "SHA256SUMS 全部 OK" || bad "SHA256SUMS 校验失败"
sqlite3 ./data/manual.sqlite3 ".dump" > source-after.sql
expect_eq "源库未被备份改写（dump 前后 sha256）" \
  "$(shasum -a 256 source-before.sql | awk '{print $1}')" \
  "$(shasum -a 256 source-after.sql | awk '{print $1}')"
expect_eq "源库仍保留注入的签名（未就地迁移；3 行）" 3 "$(grep -c 'cdn.qa-r27.invalid' source-after.sql)"

echo
echo "-- 6) serve（源库现场）→ 任务详情 DTO 读取侧兜底"
printf '{"password":"%s"}' "${PW}" > body.json
"${BIN}" serve --data-dir ./data --listen "127.0.0.1:${APP_PORT}" > serve.log 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
BASE="http://127.0.0.1:${APP_PORT}/api/v1"
curl -s -c jar -H 'content-type: application/json' --data-binary "@body.json" "${BASE}/auth/login" -o login.json
JOB=$(cat job_id.txt)
curl -s -b jar "${BASE}/jobs/${JOB}" -o job-detail.json
expect_eq "任务详情 HTTP 状态" 200 "$(curl -s -b jar -o /dev/null -w '%{http_code}' "${BASE}/jobs/${JOB}")"
for needle in '://' 'sign=' 'QA27-STAGE-CANARY-3f9a' 'QA27-NEEDS-CANARY-6d21' 'QA27-ATTEMPT-CANARY-7c2d' \
              'QA27-QUOTE-CANARY-2e18' 'QA27-NL-CANARY-5a4f' 'QA27-PUNCT-CANARY-8b0e'; do
  expect_eq "任务详情不含 '${needle}'" 0 "$(hits job-detail.json "$needle")"
done
python3 - <<'PY' | tee dto-view.txt
import json
body = json.load(open('job-detail.json'))
stages = {s['stageKind']: s for s in body['data']['stages']}
download = stages['model_download']
attempts = body['data']['attempts']
print("== DTO model_download.lastError =", download['lastError'])
print("== DTO model_download.needsInput[0].message =", download['needsInput'][0]['message'])
print("== DTO attempts[].lastError =", [a['lastError'] for a in attempts])
fragments = ["已过期，请重试", "' 后重试", "：以及结束", "，结束"]
texts = [download['lastError'], download['needsInput'][0]['message']] + [a['lastError'] for a in attempts]
missing = [f for f in fragments if not any(f in (t or '') for t in texts)]
print("== 尾部文本缺失 =", missing)
# 诊断能力：摘要标签、host、stage id、attempt 的远端 task 事实仍在
print("== 摘要标签命中 =", sum(t.count('临时供应商地址已脱敏') for t in texts if t))
print("== host 摘要 =", sum(t.count('host=cdn.qa-r27.invalid') for t in texts if t))
print("== 远端任务事实（attempts[].remoteTaskId 非空数） =",
      sum(1 for a in attempts if a.get('remoteTaskId')))
print("== stage id 仍在 =", bool(download.get('id')))
PY
grep -q "尾部文本缺失 = \[\]" dto-view.txt && ok "DTO 尾部文本全部保留" || bad "DTO 尾部文本有缺失"
python3 -c "
import json
body = json.load(open('job-detail.json'))
s = [x for x in body['data']['stages'] if x['stageKind']=='model_download'][0]
assert 'host=cdn.qa-r27.invalid' in s['lastError'], s['lastError']
assert '下载可安全重试' in s['lastError'], s['lastError']
" && ok "DTO 保留诊断（host 摘要 + 结论）" || bad "DTO 诊断信息缺失"

echo
echo "-- 6b) 导出兜底（发布包仍可导出、包内 0 命中）"
RELEASE_ID=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['releaseId'])" "${INFO}")
code=$(curl -s -b jar -o export.zip -w '%{http_code}' "${BASE}/releases/${RELEASE_ID}/export")
expect_eq "导出 HTTP 状态" 200 "${code}"
if [[ "$(head -c 2 export.zip)" == "PK" ]]; then ok "导出产物是 ZIP（PK magic）"; else bad "导出产物不是 ZIP"; fi
rm -rf export && mkdir -p export && (cd export && unzip -qq ../export.zip)
export_hits=0
for needle in '://' 'sign=' 'QA27-STAGE-CANARY-3f9a' 'QA27-NEEDS-CANARY-6d21' 'QA27-ATTEMPT-CANARY-7c2d' 'cdn.qa-r27.invalid'; do
  n=$(grep -rIl -- "$needle" export 2>/dev/null | wc -l | tr -d ' ')
  [[ "${n}" != "0" ]] && { bad "导出包命中 '${needle}'（${n} 个文件）"; export_hits=$((export_hits+1)); }
done
[[ "${export_hits}" == "0" ]] && ok "导出包 6 个判据 0 命中（导出白名单不含供应商事实表）"

echo
echo "-- 7) serve 日志核查"
for needle in 'QA27-STAGE-CANARY-3f9a' 'QA27-NEEDS-CANARY-6d21' 'QA27-ATTEMPT-CANARY-7c2d' 'sign=' 'cdn.qa-r27.invalid'; do
  expect_eq "serve 日志不含 '${needle}'" 0 "$(hits serve.log "$needle")"
done
allowed=$(grep "://" serve.log | grep -c -e "listening on http://" -e '"baseUrl"')
total_url=$(grep -c "://" serve.log)
expect_eq "serve 日志 '://' 只来自监听横幅/baseUrl 回显" "${allowed}" "${total_url}"
nonloopback=$(grep -o 'https\?://[a-zA-Z0-9.:_-]*' serve.log | grep -v "127.0.0.1" | grep -c .)
expect_eq "serve 日志里非回环主机数（零外网门禁）" 0 "${nonloopback}"

mkdir -p "${QA_DIR}/work-dist"
cp snapshot-scan.txt dto-view.txt job-detail.json backup.log serve.log snapshot.sql source-after.sql "${QA_DIR}/work-dist/" 2>/dev/null

echo
echo "== dist 复验结果：$([[ ${RESULT} = 0 ]] && echo PASS || echo FAIL)"
exit ${RESULT}
