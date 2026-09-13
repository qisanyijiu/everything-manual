#!/bin/bash
# QA 回合 28 · BUG-011 复验（QA 自建，独立于 RD 脚本）：
#   restore 归档样例 → 注入 QA 自定的句子型 needs_input（4 条：句中夹 URL / 纯文本 /
#   句首 URL / 整串 URL；message 契约必须仍是字符串）与 usage_json 句子
#   → 源库 dump 指纹（备份不得改写源库）→ backup（快照：JSON 合法、句子逐字保留、
#     仅 URL 片段变摘要标签、同列其它条目不得丢失、全列 0 命中判据）
#   → restore 新目录 → serve → 任务详情 needsInput 逐字核对（与快照一致）
#   → 诊断：故意注入损坏的 needs_input_json，确认日志如实记录
#     `job_detail_needs_input_parse_failed`（不静默）且 HTTP 仍 200
#   → 兜底真实性：临时 ALTER TABLE 新增一个 TEXT 列并注入 canary URL，
#     确认 backup 的"表 × 全列"扫描**无需改代码**即覆盖新列
# 全程只连 127.0.0.1；零外网、零付费。
#
# 用法：bash qa-r28-bug011-sentence.sh <dist 二进制绝对路径>
set -uo pipefail

BIN="${1:?用法: qa-r28-bug011-sentence.sh <绝对路径的 dist 二进制>}"
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
INFO="${ROOT}/artifacts/web-mvp/t20-rd/rehearsal-info.json"
WORK="$(mktemp -d /tmp/em-r28-bug011-XXXXXX)"
APP_PORT=18438
SERVER_PID=""
RESULT=0
# bash 层判据用的 canary（python 内为同一字面量）
CANARY="QA28-SENT-CANARY-5a1c"

ok()  { echo "  [通过] $1"; }
bad() { echo "  [失败] $1"; RESULT=1; }
expect_eq() { if [[ "$2" == "$3" ]]; then ok "$1（=$3）"; else bad "$1（期望 $2，实际 $3）"; fi; }
hits() { grep -c -- "$2" "$1" 2>/dev/null | head -1; }

stop_server() {
  if [[ -n "${SERVER_PID}" ]]; then kill -9 "${SERVER_PID}" 2>/dev/null; wait 2>/dev/null; SERVER_PID=""; fi
}
cleanup() {
  stop_server
  rm -rf "${WORK}"
}
trap cleanup EXIT

PW=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['password'])" "${INFO}")
mkdir -p "${WORK}"
cd "${WORK}" || exit 1

echo "== QA 回合 28 · BUG-011 复验（句子保真 / 多条目 / 不静默 / 新列兜底）"
echo "   sha256 $(shasum -a 256 "${BIN}" | awk '{print $1}')"
echo "   工作目录 ${WORK}"

echo
echo "-- 1) restore 归档样例 → ./data，注入 QA 句子（4 条）与 usage 句子"
"${BIN}" restore --from "${SAMPLE}" --data-dir ./data > restore.log 2>&1 || { echo "restore 失败"; exit 1; }
python3 - <<'PY'
import json, sqlite3

CANARY = "QA28-SENT-CANARY-5a1c"
NEEDS = [
    {"code": "download_insecure_scheme",
     "message": "模型下载必须使用 HTTPS（实际 http://cdn.qa28.invalid/m.glb?sign=" + CANARY +
                "）：拒绝下载，请检查来源后重试"},
    {"code": "retry", "message": "下载可安全重试（task-9），无需重新付费"},
    {"code": "download_transport",
     "message": "https://cdn.qa28.invalid/a.glb?sign=" + CANARY + " 已过期，请按 task_id 重查（不重新购买）"},
    {"code": "download_url_access", "message": "https://cdn.qa28.invalid/m.glb?sign=" + CANARY},
]
USAGE = {
    "remoteTaskId": "task-qa28-sentence",
    "billing": {"creditMinor": 3000},
    "errorSummary": "模型拒答（refusal）：下载参考 https://cdn.qa28.invalid/p.png?sign=" + CANARY + " 也失败",
}
con = sqlite3.connect('./data/manual.sqlite3')
job = con.execute("select job_id from job_stages where stage_kind='model_download' limit 1").fetchone()[0]
con.execute(
    "update job_stages set status='needs_input', needs_input_json=?, usage_json=? "
    "where job_id=? and stage_kind='model_download'",
    (json.dumps(NEEDS, ensure_ascii=False), json.dumps(USAGE, ensure_ascii=False), job))
con.commit()
print("  注入 job =", job)
print("  源库 needs_input_json =", con.execute(
    "select needs_input_json from job_stages where stage_kind='model_download'").fetchone()[0])
open('job_id.txt', 'w').write(job)
PY

echo
echo "-- 2) 源库指纹（备份不得改写源库）"
sqlite3 ./data/manual.sqlite3 ".dump" > source-before.sql

echo
echo "-- 3) backup → 快照：JSON 合法 / 句子逐字 / 类型保持 / 全列 0 命中"
"${BIN}" backup --data-dir ./data --out ./backup > backup.log 2>&1
expect_eq "backup 退出码" 0 "$?"

python3 - > snapshot-check.txt <<'PY'
import json, re, sqlite3, sys

CANARY = "QA28-SENT-CANARY-5a1c"
LABEL = r"（临时供应商地址已脱敏：host=cdn\.qa28\.invalid；sha256=[0-9a-f]{16}…）"
con = sqlite3.connect('./backup/database/manual.sqlite3')

bad = 0
def check(name, cond, detail=""):
    global bad
    print(("  [通过] " if cond else "  [失败] ") + name + ("" if cond else f"：{detail}"))
    if not cond:
        bad += 1

# 3a) 全列判据扫描（含新增列；两张事实表的全部列）
needles = ['://', 'sign=', CANARY, '/qa-r28/']
for table in ("job_stages", "provider_attempts"):
    cols = [row[1] for row in con.execute(f"pragma table_info({table})")]
    for col in cols:
        for needle in needles:
            n = con.execute(f'select count(*) from "{table}" where instr(cast("{col}" as text), ?) > 0',
                            (needle,)).fetchone()[0]
            check(f"快照 {table}.{col} 不含 {needle!r}", n == 0, f"命中 {n} 行")

# 3b) needs_input_json：JSON 合法、4 条、message 全是字符串、句子逐字
raw = con.execute("select needs_input_json from job_stages where stage_kind='model_download'").fetchone()[0]
items = json.loads(raw)  # 解析失败即 JSON 非法 → 抛异常
check("快照 needs_input_json 是合法 JSON 数组", isinstance(items, list) and len(items) == 4,
      f"len={len(items) if isinstance(items, list) else type(items).__name__}")
named = {item["code"]: item for item in items}
check("全部 message 仍是字符串（KeepString 契约）",
      all(isinstance(item.get("message"), str) for item in items),
      json.dumps(items, ensure_ascii=False)[:200])
m0 = named["download_insecure_scheme"]["message"]
check("句中夹 URL：句子逐字保留（只 URL 片段变标签）",
      re.fullmatch("模型下载必须使用 HTTPS（实际 " + LABEL + "）：拒绝下载，请检查来源后重试", m0) is not None,
      m0)
check("无 URL 条目逐字未动",
      named["retry"]["message"] == "下载可安全重试（task-9），无需重新付费",
      named["retry"]["message"])
m2 = named["download_transport"]["message"]
check("句首 URL：尾部文本保留",
      re.fullmatch(LABEL + " 已过期，请按 task_id 重查（不重新购买）", m2) is not None, m2)
m3 = named["download_url_access"]["message"]
check("整串 URL（KeepString）：只留标签、类型仍是字符串",
      re.fullmatch(LABEL, m3) is not None, m3)

# 3c) usage_json：句子保留、事实字段保留、纯 URL 值不受 KeepString 影响
usage = json.loads(con.execute("select usage_json from job_stages where stage_kind='model_download'").fetchone()[0])
summary = usage["errorSummary"]
check("usage.errorSummary 是字符串且句子尾部保留",
      isinstance(summary, str) and summary.endswith("也失败") and "临时供应商地址已脱敏" in summary, summary)
check("usage 事实字段保留（id/计费）",
      usage.get("remoteTaskId") == "task-qa28-sentence" and usage.get("billing", {}).get("creditMinor") == 3000,
      json.dumps(usage, ensure_ascii=False))

# 3d) 快照完整性 + 源库未改写 + 源库仍保留 canary（证明现场有意义）
manifest_sha = json.load(open('./backup/manifest.json'))['database']['sha256']
import hashlib
actual = hashlib.sha256(open('./backup/database/manual.sqlite3', 'rb').read()).hexdigest()
check("快照 sha256 == manifest", manifest_sha == actual, f"{manifest_sha} != {actual}")
open('snapshot-items.json', 'w').write(json.dumps(items, ensure_ascii=False))
sys.exit(1 if bad else 0)
PY
snapshot_rc=$?
cat snapshot-check.txt
[[ ${snapshot_rc} -eq 0 ]] && ok "快照检查全部通过" || bad "快照检查存在失败项"

sqlite3 ./data/manual.sqlite3 ".dump" > source-after.sql
expect_eq "源库未被备份改写（dump 前后 sha256）" \
  "$(shasum -a 256 source-before.sql | awk '{print $1}')" \
  "$(shasum -a 256 source-after.sql | awk '{print $1}')"
expect_eq "源库仍保留 canary（未就地迁移；3 条 needs_input + 1 处 usage = 4）" 4 \
  "$(grep -o "${CANARY}" source-after.sql | wc -l | tr -d ' ')"

echo
echo "-- 4) restore 快照 → 新目录 → serve → 任务详情逐字核对"
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
JOB=$(cat job_id.txt)
expect_eq "任务详情 HTTP 状态" 200 "$(curl -s -b jar -o job-detail.json -w '%{http_code}' "${BASE}/jobs/${JOB}")"
for needle in '://' 'sign=' "${CANARY}" '/qa-r28/'; do
  expect_eq "恢复库详情不含 '${needle}'" 0 "$(hits job-detail.json "${needle}")"
done
python3 - <<'PY' && ok "恢复库 DTO：needsInput 4 条且与快照逐字一致、status=needs_input" || bad "恢复库 DTO 形态不符合预期"
import json
body = json.load(open('job-detail.json'))
stage = [s for s in body['data']['stages'] if s['stageKind'] == 'model_download'][0]
snapshot_items = json.load(open('snapshot-items.json'))
assert stage['status'] == 'needs_input', stage['status']
assert len(stage['needsInput']) == 4, f"缺项静默为空/条数不符：{stage['needsInput']}"
assert stage['needsInput'] == snapshot_items, json.dumps(stage['needsInput'], ensure_ascii=False)
print("  DTO needsInput =", json.dumps(stage['needsInput'], ensure_ascii=False)[:220], "…")
PY
expect_eq "serve 日志无解析失败告警（正常路径不告警）" 0 "$(hits serve.log 'job_detail_needs_input_parse_failed')"
python3 - <<'PY' && ok "恢复库 DB 无 canary 残留" || bad "恢复库 DB 残留 canary"
import sqlite3
con = sqlite3.connect('./data2/manual.sqlite3')
tables = [r[0] for r in con.execute("select name from sqlite_master where type='table' and name not like 'sqlite_%'")]
for table in tables:
    for col in [r[1] for r in con.execute(f"pragma table_info({table})")]:
        n = con.execute(f'select count(*) from "{table}" where instr(cast("{col}" as text), ?) > 0',
                        ("QA28-SENT-CANARY-5a1c",)).fetchone()[0]
        assert n == 0, f"{table}.{col} 命中 {n}"
print("  全库扫描 0 命中")
PY

echo
echo "-- 5) 诊断：损坏的 needs_input_json 必须如实告警（不静默空列表）"
stop_server
python3 - <<'PY'
import sqlite3
con = sqlite3.connect('./data2/manual.sqlite3')
con.execute("update job_stages set needs_input_json='{\"broken\":true}' where stage_kind='model_download'")
con.commit()
PY
"${BIN}" serve --data-dir ./data2 --listen "127.0.0.1:${APP_PORT}" > serve2.log 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
curl -s -c jar2 -H 'content-type: application/json' --data-binary @body.json "${BASE}/auth/login" -o login2.json
expect_eq "损坏行的任务详情 HTTP 状态" 200 "$(curl -s -b jar2 -o job-detail2.json -w '%{http_code}' "${BASE}/jobs/${JOB}")"
python3 -c "
import json
stage = [s for s in json.load(open('job-detail2.json'))['data']['stages'] if s['stageKind']=='model_download'][0]
assert stage['needsInput'] == [], stage['needsInput']
print('  损坏行 needsInput =', stage['needsInput'], '（按空列表展示）')
" && ok "损坏行按空列表展示且仍可打开" || bad "损坏行行为异常"
[[ "$(hits serve2.log 'job_detail_needs_input_parse_failed')" -ge 1 ]] \
  && ok "日志出现 job_detail_needs_input_parse_failed（不静默）" \
  || bad "日志未记录解析失败（静默吞掉）"
stop_server

echo
echo "-- 6) 新列兜底真实性：临时 ALTER TABLE 新增 3 个 TEXT 列（不改任何生产代码）"
echo "      · qa_r28_probe_text：**非 JSON** 的裸 URL（模拟未来新的纯文本错误列）"
echo "      · qa_r28_probe_sentence：JSON 列，句子含 URL（模拟未来新的说明列）"
echo "      · qa_r28_probe_whole：JSON 列，值是整串 URL（观察默认形态 = 按列名的 mode 选择）"
python3 - <<'PY'
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
CANARY = "QA28-PROBE-CANARY-7b3e"
con.execute("alter table job_stages add column qa_r28_probe_text text")
con.execute("alter table job_stages add column qa_r28_probe_sentence text")
con.execute("alter table job_stages add column qa_r28_probe_whole text")
con.execute(
    "update job_stages set qa_r28_probe_text=?, qa_r28_probe_sentence=?, qa_r28_probe_whole=? "
    "where stage_kind='model_download'",
    (f"https://cdn.qa28.invalid/probe.glb?sign={CANARY}",
     '{"message": "' + f"新增列句子含 https://cdn.qa28.invalid/s.glb?sign={CANARY} 与其它文字" + '"}',
     '{"url": "' + f"https://cdn.qa28.invalid/w.glb?sign={CANARY}" + '"}'))
con.commit()
print("  源库新列三值已注入")
PY
"${BIN}" backup --data-dir ./data --out ./backup2 > backup2.log 2>&1
expect_eq "backup（含新列）退出码" 0 "$?"
python3 - <<'PY'
import json, sqlite3

con = sqlite3.connect('./backup2/database/manual.sqlite3')
bad = 0
def check(name, cond, detail=""):
    global bad
    print(("  [通过] " if cond else "  [失败] ") + name + ("" if cond else f"：{detail}"))
    if not cond:
        bad += 1

cols = [r[1] for r in con.execute("pragma table_info(job_stages)")]
check("快照保留新列（VACUUM INTO 复制 schema）",
      all(c in cols for c in ("qa_r28_probe_text", "qa_r28_probe_sentence", "qa_r28_probe_whole")), cols)
row = con.execute(
    "select qa_r28_probe_text, qa_r28_probe_sentence, qa_r28_probe_whole from job_stages "
    "where stage_kind='model_download'").fetchone()
text_col, sentence_col, whole_col = row
check("新非 JSON 文本列被文本级兜底（无 :// / canary）",
      '://' not in text_col and 'QA28-PROBE-CANARY-7b3e' not in text_col and 'host=cdn.qa28.invalid' in text_col,
      text_col)
parsed_sentence = json.loads(sentence_col)
check("新 JSON 句子列：片段替换、句子与类型保持",
      isinstance(parsed_sentence["message"], str) and parsed_sentence["message"].endswith("与其它文字")
      and '://' not in sentence_col and 'QA28-PROBE-CANARY-7b3e' not in sentence_col, sentence_col)
print("  观察（非断言失败）：新 JSON 整串 URL 列的默认形态 =", whole_col[:60],
      "（按列名 mode 选择；新字符串契约列需在 json_string_mode_for_column 登记——残余缺口，见报告）")

# 全表全列扫描新 canary
for table in ("job_stages", "provider_attempts"):
    for col in [r[1] for r in con.execute(f"pragma table_info({table})")]:
        n = con.execute(f'select count(*) from "{table}" where instr(cast("{col}" as text), ?) > 0',
                        ("QA28-PROBE-CANARY-7b3e",)).fetchone()[0]
        check(f"快照 {table}.{col} 不含新 canary", n == 0, f"命中 {n}")
import sys
sys.exit(1 if bad else 0)
PY
[[ $? -eq 0 ]] && ok "新列兜底检查全部通过" || bad "新列兜底检查存在失败项"

echo
echo "== QA 回合 28 · BUG-011 复验结果：$([[ ${RESULT} = 0 ]] && echo PASS || echo FAIL)"
cp snapshot-check.txt snapshot-items.json job-detail.json job-detail2.json backup.log backup2.log serve.log serve2.log source-after.sql "${QA_DIR}/" 2>/dev/null
exit ${RESULT}
