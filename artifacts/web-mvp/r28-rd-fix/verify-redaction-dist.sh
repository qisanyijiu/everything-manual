#!/bin/bash
# RD 回合 28（BUG-009/BUG-010 修复）dist 级自查脚本。
#
# 覆盖三件事（真实 release 二进制 + 真实 HTTP，全程只连 127.0.0.1，零外网、零付费）：
#   A. BUG-010：备份"非 JSON 文本列"兜底**不吞字**——注入
#      `链接 https://…?sign=canary已过期，请重试` → backup → 快照里 URL 变摘要标签，
#      但"已过期，请重试"必须保留。
#   B. BUG-009 读取侧：把传输失败消息（含完整签名 URL）直写 job_stages.last_error 与
#      provider_attempts.last_error（模拟修复前的历史行）→ serve → GET /jobs/{id}
#      → lastError 不得含 `://`/签名，须含 host 摘要；响应体整体 `://` 计数为 0。
#   C. 日志侧：serve 日志中 canary 出现次数为 0（写入路径的日志由 RD 单测断言
#      `log_summary()`；本脚本只做 canary 扫描，不冒充"传输失败可在 dist 上制造"）。
#
# 用法：bash artifacts/web-mvp/r28-rd-fix/verify-redaction-dist.sh <dist 二进制绝对路径>

set -uo pipefail
BIN="${1:?用法: verify-redaction-dist.sh <dist 二进制绝对路径>}"
RD_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${RD_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
INFO="${ROOT}/artifacts/web-mvp/t20-rd/rehearsal-info.json"
WORK="$(mktemp -d /tmp/em-r28-verify-XXXXXX)"
APP_PORT=18142
CANARY="rd28-canary-transport-signature-7d31"
PASS=0
FAIL=0

cleanup() {
  [[ -n "${SERVE_PID:-}" ]] && kill -9 "${SERVE_PID}" 2>/dev/null
  wait 2>/dev/null
  rm -rf "${WORK}"
}
trap cleanup EXIT
mkdir -p "${WORK}"; cd "${WORK}" || exit 1

check() { # check <描述> <条件命令...>
  local label="$1"; shift
  if "$@"; then PASS=$((PASS + 1)); echo "  [通过] ${label}"; else FAIL=$((FAIL + 1)); echo "  [失败] ${label}"; fi
}

echo "== RD 回合 28 · 脱敏 dist 级自查（canary=${CANARY}）"
"${BIN}" restore --from "${SAMPLE}" --data-dir ./data > restore.log 2>&1
echo "restore exit=$?"

# --- 注入历史行（修复前落库形态） ---
python3 - ./data/manual.sqlite3 "${CANARY}" <<'PY'
import sqlite3, sys
db, canary = sys.argv[1], sys.argv[2]
con = sqlite3.connect(db)
transport = ("模型下载传输失败（下载可安全重试；不产生供应商费用）：连接失败："
             f"error sending request for url (https://cdn.example.invalid/qa-r28/model.glb?sign={canary})")
n1 = con.execute("update job_stages set last_error = ? where stage_kind='model_download'",
                 (transport,)).rowcount
assert n1 == 1, n1
swallow = (f"链接 https://cdn.example.invalid/qa-r28/x.glb?sign={canary}已过期，请重试")
n2 = con.execute("update provider_attempts set last_error = ?", (swallow,)).rowcount
assert n2 >= 1, n2
con.commit()
print(f"  注入完成：job_stages={n1} 行、provider_attempts={n2} 行（含签名 URL 与紧邻中文）")
PY

# --- A. 备份兜底不吞字（BUG-010） ---
"${BIN}" backup --data-dir ./data --out ./snap > backup.log 2>&1
echo "backup exit=$?"
SNAP="./snap/database/manual.sqlite3"
python3 - "${SNAP}" "${CANARY}" ./swallow-check.txt <<'PY'
import sqlite3, sys
db, canary, out = sys.argv[1], sys.argv[2], sys.argv[3]
row = sqlite3.connect(db).execute(
    "select last_error from provider_attempts where last_error like '%已脱敏%'").fetchone()
text = row[0] if row else ""
open(out, "w").write(text)
print("  快照文本 =", text[:200])
assert canary not in text, "快照仍含签名"
assert "://" not in text, "快照仍含 URL 形态"
assert "已过期，请重试" in text, "BUG-010：URL 之后的中文被吞"
PY
check "A. 备份兜底：签名消失且'已过期，请重试'保留" test $? -eq 0
grep -q "已过期，请重试" ./swallow-check.txt && check "A. 快照文本含尾部中文" true || check "A. 快照文本含尾部中文" false
grep -q "://" ./swallow-check.txt && check "A. 快照文本无 '://'" false || check "A. 快照文本无 '://'" true

# --- B/C. 读取侧与日志侧 ---
printf '{"password":"%s"}' "$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['password'])" "${INFO}")" > body.json
chmod 600 body.json
QA_TRIPO_KEY=rd28 QA_MANUAL_AI_KEY=rd28 \
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
attempt_errors = [a.get("lastError") or "" for a in detail["data"]["attempts"]]
print("=== 观察（任务详情 API 响应）")
print("  stage.lastError =", (stage["lastError"] or "")[:200])
print("  attempts.lastError =", [e[:120] for e in attempt_errors])
assert canary not in raw, "响应体含签名"
assert "://" not in raw, "响应体含 URL 形态"
assert "host=cdn.example.invalid" in (stage["lastError"] or ""), "stage.lastError 缺 host 摘要标签"
assert "已脱敏" in stage["lastError"], "stage.lastError 未走摘要标签"
print("  断言通过：响应体无 '://'、无签名；lastError 为摘要标签")
PY
check "B. 任务详情读取侧：lastError 已脱敏" test $? -eq 0
check "C. serve 日志无签名 canary" bash -c "! grep -q '${CANARY}' serve.log"

echo
echo "== 结果：${PASS} 通过 / ${FAIL} 失败"
[[ "${FAIL}" -eq 0 ]]
