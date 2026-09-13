#!/bin/bash
# QA 回合 27 · BUG-011 现场（备份快照对 JSON 文本列 needs_input_json 走"整串对象替换"）：
#   restore 归档样例 → 注入**产品可达**形态的 needs_input 消息（`download_insecure_scheme`，
#   文案里含 `http://host`）→ backup → 对比源库/快照 → restore 快照 → serve →
#   任务详情 `needsInput` 是否还在。
# 本脚本不判 BUG-009/010 的通过与否（那是 qa-r27-redaction-dist.sh 的职责），只留 BUG-011 证据。
# 全程只连 127.0.0.1；零外网、零付费。
#
# 用法：bash qa-r27-bug011-evidence.sh <dist 二进制绝对路径>
set -uo pipefail

BIN="${1:?用法: qa-r27-bug011-evidence.sh <绝对路径的 dist 二进制>}"
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
INFO="${ROOT}/artifacts/web-mvp/t20-rd/rehearsal-info.json"
WORK="$(mktemp -d /tmp/em-r27-bug011-XXXXXX)"
APP_PORT=18433
SERVER_PID=""
cleanup() {
  [[ -n "${SERVER_PID}" ]] && kill -9 "${SERVER_PID}" 2>/dev/null
  wait 2>/dev/null
  rm -rf "${WORK}"
}
trap cleanup EXIT
PW=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['password'])" "${INFO}")
mkdir -p "${WORK}"; cd "${WORK}" || exit 1

echo "== QA 回合 27 · BUG-011 现场（dist $(shasum -a 256 "${BIN}" | awk '{print $1}') ）"
"${BIN}" restore --from "${SAMPLE}" --data-dir ./data > restore.log 2>&1 || { echo "restore 失败"; exit 1; }
python3 - <<'PY'
import json, sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
# 产品可达形态：模型下载链接是 http:// → DownloadError::InsecureScheme 的 message 含 `http://host`。
msg = "模型下载必须使用 HTTPS（实际 http://cdn.qa27.invalid/m.glb）：拒绝下载"
con.execute("update job_stages set status='needs_input', needs_input_json=? where stage_kind='model_download'",
            (json.dumps([{"code": "download_insecure_scheme", "message": msg}], ensure_ascii=False),))
con.commit()
print("  源库 needs_input_json =", con.execute(
    "select needs_input_json from job_stages where stage_kind='model_download'").fetchone()[0])
open('job_id.txt', 'w').write(con.execute(
    "select job_id from job_stages where stage_kind='model_download'").fetchone()[0])
PY
"${BIN}" backup --data-dir ./data --out ./backup > backup.log 2>&1 || { echo "backup 失败"; exit 1; }
echo "  快照 needs_input_json = $(python3 -c "
import sqlite3
print(sqlite3.connect('./backup/database/manual.sqlite3').execute(\"select needs_input_json from job_stages where stage_kind='model_download'\").fetchone()[0])")"
"${BIN}" restore --from ./backup --data-dir ./data2 > restore2.log 2>&1 || { echo "restore2 失败"; exit 1; }
printf '{"password":"%s"}' "${PW}" > body.json
"${BIN}" serve --data-dir ./data2 --listen "127.0.0.1:${APP_PORT}" > serve.log 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
BASE="http://127.0.0.1:${APP_PORT}/api/v1"
curl -s -c jar -H 'content-type: application/json' --data-binary @body.json "${BASE}/auth/login" -o login.json
curl -s -b jar "${BASE}/jobs/$(cat job_id.txt)" -o detail.json
python3 - <<'PY'
import json
body = json.load(open('detail.json'))
stage = [s for s in body['data']['stages'] if s['stageKind'] == 'model_download'][0]
print("  恢复库 stage.status   =", stage['status'])
print("  恢复库 needsInput     =", stage['needsInput'])
print("  恢复库 needsInput 条数 =", len(stage['needsInput']))
PY
echo "== 结论：若'恢复库 needsInput 条数 = 0'而源库/快照行不可解析 → BUG-011（备份快照把整条 message 替换为摘要对象，恢复后缺项静默消失）"
