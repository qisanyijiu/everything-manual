#!/bin/bash
# QA 回合 26 · BUG-010 复现：备份"非 JSON 文本列"的兜底脱敏（redaction::redact_urls_in_text）
# 右向扫描不限制字符类 → URL 后**紧邻**的非 URL 字符被一并吞掉（静默丢字）。
# 手法：restore 归档样例 → 往 provider_attempts.last_error 注入 3 个中文紧邻样例 →
# backup → 打印快照内该列，逐片段核对是否保留。
# 全程只连 127.0.0.1；零外网、零付费。
set -uo pipefail
BIN="${1:?用法: qa-r26-text-swallow.sh <dist 二进制绝对路径>}"
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
WORK="$(mktemp -d /tmp/em-r26-swallow-XXXXXX)"
cleanup() { rm -rf "${WORK}"; }
trap cleanup EXIT
mkdir -p "${WORK}"; cd "${WORK}" || exit 1

echo "== QA 回合 26 · 文本兜底脱敏的右向吞字复现（BUG-010）"
"${BIN}" restore --from "${SAMPLE}" --data-dir ./data > restore.log 2>&1 || { echo "restore 失败"; exit 1; }
python3 - <<'PY'
import sqlite3
con = sqlite3.connect('./data/manual.sqlite3')
cases = [
    "链接 https://cdn.example.invalid/x.glb已过期，请重试",
    "见（https://cdn.example.invalid/y.glb）后重试",
    "参考 https://cdn.example.invalid/z.glb；以及 https://cdn.example.invalid/w.glb。",
]
row = con.execute("select id from provider_attempts limit 1").fetchone()[0]
con.execute("update provider_attempts set last_error = ? where id = ?", (" || ".join(cases), row))
con.commit()
print("  已注入 3 个样例（原文见下）")
print("  原文 =", " || ".join(cases))
PY
"${BIN}" backup --data-dir ./data --out ./backup > backup.log 2>&1 || { echo "backup 失败"; exit 1; }
grep -o "临时 URL 脱敏：[^（]*" backup.log | head -1
python3 - <<'PY'
import sqlite3
con = sqlite3.connect('./backup/database/manual.sqlite3')
value = con.execute("select last_error from provider_attempts limit 1").fetchone()[0]
print("  快照 =", value)
print("== 逐片段核对（原文片段是否保留）")
for fragment in ("已过期", "请重试", "）后重试", "；以及"):
    print(f"  {fragment!r} 保留 = {fragment in value}")
lost = [f for f in ("已过期",) if f not in value]
print(f"== 结论：{'发现静默丢字（BUG-010 复现）' if lost else '未复现'}")
PY
