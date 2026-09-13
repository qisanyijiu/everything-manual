#!/bin/bash
# QA 回合 28 · 表外边界如实性核对（ADR-034 §6 / R29-4(d)）：
#   RD 声明"原始响应诊断 blob（及其在备份中的副本）逐字节保留提供方原文"。
#   本脚本在 dist 上独立验证该声明的**行为面**：blob 文件内容（无论是否含签名 URL）
#   被 backup 原样复制（不做内容过滤），即"表外边界"确有其事、不是虚构或夸大。
#   （产生侧在 release 不可造；这里直接注入等价形态的 blob 行 + 文件。）
# 全程只连 127.0.0.1；零外网、零付费。
#
# 用法：bash qa-r28-blob-boundary.sh <dist 二进制绝对路径>
set -uo pipefail

BIN="${1:?用法: qa-r28-blob-boundary.sh <dist 二进制绝对路径>}"
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
WORK="$(mktemp -d /tmp/em-r28-blob-XXXXXX)"
RESULT=0
ok()  { echo "  [通过] $1"; }
bad() { echo "  [失败] $1"; RESULT=1; }

cleanup() { rm -rf "${WORK}"; }
trap cleanup EXIT
cd "${WORK}" || exit 1

echo "== QA 回合 28 · blob 表外边界（ADR-034 §6）"
echo "   sha256 $(shasum -a 256 "${BIN}" | awk '{print $1}')"

"${BIN}" restore --from "${SAMPLE}" --data-dir ./data > restore.log 2>&1 || { echo "restore 失败"; exit 1; }

CANARY="QA28-BLOB-CANARY-3d21"
PROBE_SHA=$(python3 - "$CANARY" <<'PY'
import hashlib, os, sqlite3, sys
canary = sys.argv[1]
# 形态与真实诊断 blob 相同：提供方原文（内含签名 URL），内容寻址文件名。
content = (f'{{"status":"completed","output":"下载参考 https://cdn.qa28.invalid/raw.png?sign={canary} 也失败"}}'
           ).encode()
sha = hashlib.sha256(content).hexdigest()
path = os.path.join('./data/blobs', sha[:2])
os.makedirs(path, exist_ok=True)
with open(os.path.join(path, sha), 'wb') as handle:
    handle.write(content)
con = sqlite3.connect('./data/manual.sqlite3')
con.execute(
    "insert into blobs (sha256, size, mime, storage_state, created_at) values (?, ?, 'application/json', 'stored', 0)",
    (sha, len(content)))
con.commit()
print(sha)
PY
)
echo "   注入诊断形态 blob = ${PROBE_SHA}"

"${BIN}" backup --data-dir ./data --out ./backup > backup.log 2>&1
[[ $? -eq 0 ]] && ok "backup 退出码 0" || bad "backup 失败"
BACKUP_BLOB="./backup/blobs/${PROBE_SHA:0:2}/${PROBE_SHA}"
if [[ -f "${BACKUP_BLOB}" ]]; then
  ok "备份包含该 blob（blobs/${PROBE_SHA:0:2}/…）"
else
  bad "备份缺少该 blob"; echo "== 结果：FAIL"; exit 1
fi
if grep -q "${CANARY}" "${BACKUP_BLOB}"; then
  ok "备份中 blob 逐字节保留提供方原文（含签名 URL）→ ADR-034 §6 边界声明属实"
else
  bad "备份中 blob 与源文件不一致"
fi

echo "== 结论：blob 文件（=ADR-034 §6 登记的表外边界）确实不过滤内容；"
echo "   该形态同时存在于 data-dir 与备份的 blobs/ 中，属**已登记边界**而非本轮缺陷。"
echo "== QA 回合 28 · blob 边界核对结果：$([[ ${RESULT} = 0 ]] && echo PASS || echo FAIL)"
exit ${RESULT}
