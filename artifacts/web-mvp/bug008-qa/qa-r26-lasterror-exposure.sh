#!/bin/bash
# QA 回合 26 · 读取侧证据：`job_stages.last_error` 经任务详情 API **原样输出**
# （对比：同一响应的 usage 已被脱敏）。
#
# 手法：restore 归档样例库 → 把 last_error 写成与
# `crates/server/tests/qa_t20_bug008_independent.rs` 实测到的
# 传输失败消息**逐字符相同**的文本（含签名 URL）→ serve → GET /jobs/{id} →
# 观察响应体是否出现 `://` / `sign=` / canary。
#
# 本脚本**不是**通过断言判定缺陷，而是把"读取侧是否原样透传"的真值打印出来。
# 全程只连 127.0.0.1；零外网、零付费。

set -uo pipefail
BIN="${1:?用法: qa-r26-lasterror-exposure.sh <dist 二进制绝对路径>}"
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
INFO="${ROOT}/artifacts/web-mvp/t20-rd/rehearsal-info.json"
WORK="$(mktemp -d /tmp/em-r26-lasterror-XXXXXX)"
APP_PORT=18130
CANARY="qa26-canary-transport-signature-7d31"   # 与 QA 探针测试同一 canary

cleanup() {
  [[ -n "${SERVE_PID:-}" ]] && kill -9 "${SERVE_PID}" 2>/dev/null
  wait 2>/dev/null
  rm -rf "${WORK}"
}
trap cleanup EXIT
mkdir -p "${WORK}"; cd "${WORK}" || exit 1

echo "== QA 回合 26 · last_error 读取侧透传观察"
"${BIN}" restore --from "${SAMPLE}" --data-dir ./data > restore.log 2>&1
echo "restore exit=$?"

# 与探针测试输出逐字符一致的传输失败消息（生产里由 classify_transport 生成）。
MESSAGE="模型下载传输失败（下载可安全重试；不产生供应商费用）：连接失败：error sending request for url (https://cdn.example.invalid/qa-r26/model.glb?sign=${CANARY})"
python3 - ./data/manual.sqlite3 "${MESSAGE}" <<'PY'
import sqlite3, sys
db, message = sys.argv[1], sys.argv[2]
con = sqlite3.connect(db)
cur = con.execute("update job_stages set last_error = ? where stage_kind='model_download'", (message,))
assert cur.rowcount == 1, cur.rowcount
# 对照组：usage_json 里放同一个 URL（读取侧应被脱敏为摘要）
url = "https://cdn.example.invalid/qa-r26/model.glb?sign=qa26-canary-transport-signature-7d31"
cur = con.execute(
    "update job_stages set usage_json = ? where stage_kind='model_download'",
    ('{"sha256":"aa","sizeBytes":1,"host":"cdn.example.invalid","redactedTest":"%s"}' % url,))
assert cur.rowcount == 1
con.commit()
print("  注入完成：last_error 含签名 URL；usage_json 也含同一 URL（对照）")
PY

printf '{"password":"%s"}' "$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['password'])" "${INFO}")" > body.json
chmod 600 body.json
QA_TRIPO_KEY=qa-r26 QA_MANUAL_AI_KEY=qa-r26 \
  "${BIN}" serve --data-dir ./data --listen "127.0.0.1:${APP_PORT}" > serve.log 2>&1 &
SERVE_PID=$!
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
BASE="http://127.0.0.1:${APP_PORT}/api/v1"
curl -s -c jar -H 'content-type: application/json' --data-binary @body.json "${BASE}/auth/login" -o login.json
JOB_ID=$(python3 -c "import sqlite3;print(sqlite3.connect('./data/manual.sqlite3').execute('select id from jobs').fetchone()[0])")
curl -s -b jar "${BASE}/jobs/${JOB_ID}" -o detail.json
python3 - detail.json "${CANARY}" <<'PY' | tee observation.txt
import json, sys
raw = open(sys.argv[1], encoding="utf-8").read()
canary = sys.argv[2]
detail = json.loads(raw)
stage = [s for s in detail["data"]["stages"] if s["stageKind"] == "model_download"][0]
print("=== 观察（任务详情 API 响应）")
print("  last_error 原样包含签名 URL ？", canary in (stage["lastError"] or ""))
print("  last_error 是否含 '://'      ：", "://" in (stage["lastError"] or ""))
print("  同一响应里 usage 是否已脱敏   ： usage 含 '://' =", "://" in json.dumps(stage["usage"], ensure_ascii=False))
print("  响应体整体 '://' 出现次数（HTTP 200 的 JSON 文本）=", raw.count("://"))
print("  last_error =", (stage["lastError"] or "")[:220])
print("  usage      =", json.dumps(stage["usage"], ensure_ascii=False)[:220])
PY
cp observation.txt "${QA_DIR}/work-historical/qa26-lasterror-observation.txt" 2>/dev/null || { mkdir -p "${QA_DIR}/work-historical"; cp observation.txt "${QA_DIR}/work-historical/qa26-lasterror-observation.txt"; }
echo "== 观察结束（结论见上）"
