#!/bin/bash
# BUG-008 复现与修复验证（T20 / AC-010）。
#
# 场景：
#   1) 修复前落库的历史 URL 证据（仓库内归档的 T20 样例备份，由修复前二进制产生）
#   2) 用修复后的 dist 二进制在该历史 data-dir 上 backup → 快照/恢复结果不含临时 URL
#   3) 任务详情 API 不回显完整签名 URL（历史行读取侧脱敏）
#   4) 源 data-dir 不被迁移（历史数据可用性不变）
# 全程本机回环（127.0.0.1），零外网、零付费。
#
# 用法：bash bug008-verify.sh <dist 二进制绝对路径>
set -u

BIN="${1:?用法: bug008-verify.sh <dist 二进制绝对路径>}"
case "${BIN}" in
  /*) ;;
  *) echo "FAIL 需要绝对路径：${BIN}"; exit 2 ;;
esac
[[ -x "${BIN}" ]] || { echo "FAIL 二进制不可执行：${BIN}"; exit 2; }

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
INFO="${ROOT}/artifacts/web-mvp/t20-rd/rehearsal-info.json"
WORK="$(mktemp -d /tmp/em-bug008-XXXXXX)"
PORT=18099
BASE="http://127.0.0.1:${PORT}/api/v1"
PASS=0
FAIL=0

ok()   { echo "  [通过] $1"; PASS=$((PASS+1)); }
bad()  { echo "  [失败] $1"; FAIL=$((FAIL+1)); }
expect_eq() { # 描述 期望 实际
  if [[ "$2" == "$3" ]]; then ok "$1（=$3）"; else bad "$1（期望 $2，实际 $3）"; fi
}
json_field() { # 文件 点路径
  python3 - "$1" "$2" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
for part in sys.argv[2].split("."):
    value = value[part] if isinstance(value, dict) else value[int(part)]
print(value)
PY
}
cleanup() {
  if [[ -n "${SERVE_PID:-}" ]]; then kill "${SERVE_PID}" 2>/dev/null; wait "${SERVE_PID}" 2>/dev/null; fi
  rm -rf "${WORK}"
}
trap cleanup EXIT

echo "== BUG-008 复现与修复验证 =="
echo "二进制 sha256: $(shasum -a 256 "${BIN}" | awk '{print $1}')"
echo "工作目录: ${WORK}"

# --- 1) 修复前证据（归档样例备份 = 旧二进制产生） -----------------------------
echo
echo "-- 1) 修复前证据（归档样例备份，旧二进制产生）"
BEFORE_URLS=$(sqlite3 "${SAMPLE}/database/manual.sqlite3" ".dump" | grep -c "://")
BEFORE_HOST=$(sqlite3 "${SAMPLE}/database/manual.sqlite3" ".dump" | grep -c "cdn.example.invalid")
echo "  旧快照 .dump 中 '://' 命中 ${BEFORE_URLS} 处、'cdn.example.invalid' 命中 ${BEFORE_HOST} 处"
sqlite3 "${SAMPLE}/database/manual.sqlite3" \
  "SELECT stage_kind, usage_json FROM job_stages WHERE instr(CAST(usage_json AS TEXT),'://')>0" \
  | sed 's/^/  行: /'
if [[ "${BEFORE_URLS}" -ge 1 ]]; then ok "复现：旧快照的 job_stages.usage_json 含临时 URL（= QA BUG-008 证据）"; else bad "复现失败：旧快照里找不到 URL"; fi

# --- 2) 恢复到本机 data-dir（历史数据仍带 URL：不强制迁移） -------------------
echo
echo "-- 2) restore 归档备份 → 新 data-dir"
"${BIN}" restore --from "${SAMPLE}" --data-dir "${WORK}/data" > "${WORK}/restore.log" 2>&1
expect_eq "restore 退出码" 0 "$?"
LIVE_DB="${WORK}/data/manual.sqlite3"
LIVE_URLS=$(sqlite3 "${LIVE_DB}" ".dump" | grep -c "://")
if [[ "${LIVE_URLS}" -ge 1 ]]; then ok "源 data-dir 仍保留历史 URL（不强制迁移，读取/备份时脱敏）"; else bad "源 data-dir 的历史 URL 被改动"; fi

# --- 3) 修复后 backup → 快照与恢复结果不含临时 URL ---------------------------
echo
echo "-- 3) backup（修复后二进制）"
"${BIN}" backup --data-dir "${WORK}/data" --out "${WORK}/backup-new" > "${WORK}/backup.log" 2>&1
expect_eq "backup 退出码" 0 "$?"
grep -q "临时 URL 脱敏" "${WORK}/backup.log" && ok "backup 输出报告脱敏处数" || bad "backup 输出缺少脱敏报告"
grep "临时 URL 脱敏" "${WORK}/backup.log" | sed 's/^/  /' | head -2
SNAP="${WORK}/backup-new/database/manual.sqlite3"
for needle in "://" "sign=" "/model.glb" "/preview.png"; do
  hits=$(sqlite3 "${SNAP}" ".dump" | grep -c -- "${needle}")
  expect_eq "新快照 .dump 不含 '${needle}'" 0 "${hits}"
done
SNAP_ROW=$(sqlite3 "${SNAP}" "SELECT CAST(usage_json AS TEXT) FROM job_stages WHERE stage_kind='tripo_poll'")
echo "${SNAP_ROW}" | grep -q '"redacted":true' && ok "快照保留摘要对象（redacted/host/sha256）" || bad "快照缺少摘要对象：${SNAP_ROW}"
echo "${SNAP_ROW}" | grep -q '"remoteTaskId":"t20-fixture-task-0001"' && ok "快照保留 task ID（恢复判据）" || bad "快照丢失 task ID：${SNAP_ROW}"

# 恢复新备份：同样干净
echo
echo "-- 4) restore 新备份 → 二次恢复结果"
"${BIN}" restore --from "${WORK}/backup-new" --data-dir "${WORK}/data2" > "${WORK}/restore2.log" 2>&1
expect_eq "restore（新备份）退出码" 0 "$?"
RESTORED_URLS=$(sqlite3 "${WORK}/data2/manual.sqlite3" ".dump" | grep -c "://")
expect_eq "恢复后的库不含 '://'" 0 "${RESTORED_URLS}"

# --- 5) 任务详情 API 不回显完整签名 URL（历史行读取侧脱敏） ------------------
echo
echo "-- 5) serve + 任务详情 API"
"${BIN}" serve --data-dir "${WORK}/data2" --listen "127.0.0.1:${PORT}" > "${WORK}/serve.log" 2>&1 &
SERVE_PID=$!
for _ in $(seq 1 50); do
  code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/api/v1/health/live")
  [[ "${code}" == "200" ]] && break
  sleep 0.1
done
expect_eq "health/live" 200 "${code}"

PASSWORD=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['password'])" "${INFO}")
printf '{"password":"%s"}' "${PASSWORD}" > "${WORK}/body.json"
code=$(curl -s -o "${WORK}/login.json" -w '%{http_code}' -c "${WORK}/jar" \
  -H 'Content-Type: application/json' --data-binary "@${WORK}/body.json" "${BASE}/auth/login")
expect_eq "登录" 200 "${code}"

JOB_ID=$(sqlite3 "${WORK}/data2/manual.sqlite3" "SELECT id FROM jobs LIMIT 1")
code=$(curl -s -o "${WORK}/job.json" -w '%{http_code}' -b "${WORK}/jar" "${BASE}/jobs/${JOB_ID}")
expect_eq "GET /jobs/{id}" 200 "${code}"
for needle in "://" "sign=" "/model.glb" "/preview.png" "cdn.example.invalid/preview"; do
  hits=$(grep -c -- "${needle}" "${WORK}/job.json")
  expect_eq "任务详情不含 '${needle}'" 0 "${hits}"
done
grep -q '"redacted":true' "${WORK}/job.json" && ok "任务详情给出摘要（redacted/host/sha256）" || bad "任务详情缺少摘要"
python3 - "${WORK}/job.json" <<'PY'
import json, sys
detail = json.load(open(sys.argv[1], encoding="utf-8"))
poll = [s for s in detail["data"]["stages"] if s["stageKind"] == "tripo_poll"][0]
print("  usage.modelUrl =", json.dumps(poll["usage"]["modelUrl"], ensure_ascii=False))
print("  usage.renderedImageUrl =", json.dumps(poll["usage"]["renderedImageUrl"], ensure_ascii=False))
PY

# --- 6) 导出包对照（AC-058） --------------------------------------------------
echo
echo "-- 6) 导出包（AC-058 对照）"
RELEASE_ID=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['releaseId'])" "${INFO}")
code=$(curl -s -o "${WORK}/export.zip" -w '%{http_code}' -b "${WORK}/jar" \
  "${BASE}/releases/${RELEASE_ID}/export")
expect_eq "GET /releases/{id}/export" 200 "${code}"
EXPORT_HITS=$(python3 - "${WORK}/export.zip" <<'PY'
import sys, zipfile
hits = 0
with zipfile.ZipFile(sys.argv[1]) as archive:
    for name in archive.namelist():
        data = archive.read(name)
        for needle in (b"://", b"sign=", b"cdn.example.invalid", b"127.0.0.1:"):
            hits += data.count(needle)
print(hits)
PY
)
expect_eq "导出包内 URL/签名/供应商主机命中" 0 "${EXPORT_HITS}"

# --- 7) 日志不回显签名 --------------------------------------------------------
echo
echo "-- 7) serve 日志"
# 唯一允许出现的 '://' 是启动横幅 "listening on http://<监听地址>"（本服务自己的地址）。
LOG_URL_HITS=$(grep -c "://" "${WORK}/serve.log")
LOG_LISTEN=$(grep -c "listening on http://" "${WORK}/serve.log")
expect_eq "serve 日志的 '://' 仅来自监听横幅" "${LOG_LISTEN}" "${LOG_URL_HITS}"
for needle in "sign=" "/model.glb" "/preview.png" "cdn.example.invalid/preview"; do
  hits=$(grep -c -- "${needle}" "${WORK}/serve.log")
  expect_eq "serve 日志不含 '${needle}'" 0 "${hits}"
done

echo
echo "== 结果：通过 ${PASS} 项，失败 ${FAIL} 项 =="
[[ "${FAIL}" -eq 0 ]] || exit 1
echo "注意：host（无 scheme/path/查询串）按脱敏规则保留为最小诊断信息，"
echo "      判据是 '://'、签名查询串与 URL path，而不是裸主机名。"
