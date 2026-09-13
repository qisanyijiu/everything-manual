#!/bin/zsh
# QA 独立验收（T11）：合成 v5 库 → check 只读报"待迁移" → serve 自动升到 v6（迁移链只追加）。
# 手法沿用回合 10 QA：按 0001–0005 的 SQL + 与 sqlx 一致的 SHA-384 校验和构造旧库；
# 校验和算法先用全新 init 的 _sqlx_migrations 行反证。
set -u
BIN="${1:?用法: qa-t11-migration-chain.sh <binary 绝对路径>}"
REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$REPO" || exit 1
WORK="$(mktemp -d /tmp/em-t11-qa-mig-XXXXXX)"
FRESH="$WORK/fresh"
DATA5="$WORK/data-v5"
mkdir -p "$FRESH" "$DATA5"
printf 'qa-mig-t11-not-a-real-secret\n' > "$WORK/pw.txt" && chmod 600 "$WORK/pw.txt"
echo "== 二进制 sha256：$(shasum -a 256 "$BIN" | cut -d' ' -f1)"
echo "== 工作目录：$WORK（结束清理）"

"$BIN" init --data-dir "$FRESH" --password-file "$WORK/pw.txt" > "$WORK/fresh-init.log" 2>&1
echo "fresh_init_exit=$?"
sqlite3 "$FRESH/manual.sqlite3" "SELECT '全新库：迁移行数='||COUNT(*)||'，最新 v'||MAX(version) FROM _sqlx_migrations;"

# 1) 反证校验和算法：全新库的行必须等于对应迁移文件的 SHA-384（大写 hex）
python3 - "$FRESH/manual.sqlite3" "$REPO" <<'PY'
import hashlib, sqlite3, sys, glob, os, re
db = sqlite3.connect(sys.argv[1]); repo = sys.argv[2]
files = {}
for path in glob.glob(os.path.join(repo, "migrations", "*.sql")):
    version = int(os.path.basename(path)[:4])
    files[version] = path
ok = True
for version, checksum in db.execute("select version, checksum from _sqlx_migrations order by version"):
    digest = hashlib.sha384(open(files[version], "rb").read()).hexdigest().upper()
    got = bytes(checksum).hex().upper()
    same = got == digest
    ok = ok and same
    print(f"  v{version}: checksum {'= ' if same else '≠ '}SHA-384(000{version})")
assert ok, "校验和算法与迁移文件不一致"
PY

# 2) 合成 v5 库：结构目录 + 0001–0005 + _sqlx_migrations 行
cp -R "$FRESH/tmp" "$FRESH/logs" "$FRESH/blobs" "$FRESH/lock" "$DATA5/"
DB5="$DATA5/manual.sqlite3"
for f in 0001_core_schema 0002_invariants 0003_photos_view_unique 0004_preparation_pages 0005_job_execution; do
  sqlite3 "$DB5" < "$REPO/migrations/$f.sql" || { echo "应用 $f 失败"; exit 1; }
done
sqlite3 "$FRESH/manual.sqlite3" ".schema _sqlx_migrations" | sqlite3 "$DB5"
python3 - "$DB5" "$REPO" <<'PY'
import hashlib, sqlite3, sys, os
db = sqlite3.connect(sys.argv[1]); repo = sys.argv[2]
names = {
    1: ("core schema", "0001_core_schema.sql"),
    2: ("invariants", "0002_invariants.sql"),
    3: ("photos view unique", "0003_photos_view_unique.sql"),
    4: ("preparation pages", "0004_preparation_pages.sql"),
    5: ("job execution", "0005_job_execution.sql"),
}
for version, (description, name) in names.items():
    digest = hashlib.sha384(open(os.path.join(repo, "migrations", name), "rb").read()).hexdigest()
    db.execute(
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time) "
        "VALUES (?, ?, '2026-09-12 00:00:00', 1, X'%s', 0)" % digest.upper(),
        (version, description),
    )
db.commit()
print("  合成 v5 库完成（0001–0005 + sqlx 行）")
PY
sqlite3 "$DB5" "SELECT '  迁移行数：'||COUNT(*)||'，最新 v'||MAX(version) FROM _sqlx_migrations;" \
               "SELECT '  quotes 表存在（应为 0）：'||COUNT(*) FROM sqlite_master WHERE name='quotes';"

echo
echo "== check（只读，不得迁移）"
"$BIN" check --data-dir "$DATA5" > "$WORK/check-v5.log" 2>&1
echo "check_v5_exit=$?"
grep -E "数据库|schema|结果" "$WORK/check-v5.log" | head -4
sqlite3 "$DB5" "SELECT COUNT(*) FROM _sqlx_migrations;" "SELECT COUNT(*) FROM sqlite_master WHERE name='quotes';" | paste -sd' ' - | awk '{print "  check 后：迁移行数=" $1 "，quotes 表=" $2}' 

echo
echo "== serve（应自动迁移到 v6）"
"$BIN" serve --data-dir "$DATA5" --listen 127.0.0.1:18297 > "$WORK/serve-v5.log" 2>&1 &
SV=$!
for _ in $(seq 1 60); do
  curl -sf "http://127.0.0.1:18297/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
grep -o '"event":"database_ready"[^}]*' "$WORK/serve-v5.log" | head -1
kill -TERM "$SV" 2>/dev/null; wait "$SV" 2>/dev/null
echo "serve_v5_exit=$?"
sqlite3 "$DB5" \
  "SELECT '  升级后迁移行数：'||COUNT(*)||'，最新 v'||MAX(version) FROM _sqlx_migrations;" \
  "SELECT '  quotes 表：'||COUNT(*) FROM sqlite_master WHERE name='quotes';" \
  "SELECT '  quotes 索引：'||GROUP_CONCAT(name) FROM sqlite_master WHERE type='index' AND tbl_name='quotes' AND name LIKE 'quotes_%';" \
  "SELECT '  0006 触发器数：'||COUNT(*) FROM sqlite_master WHERE type='trigger' AND name IN ('quotes_snapshot_immutable','quotes_confirmation_frozen','quotes_consumption_frozen');" \
  "SELECT '  活跃预留唯一索引：'||COUNT(*) FROM sqlite_master WHERE name='cost_ledger_active_reservation';" \
  "SELECT '  admins 表存在：'||COUNT(*) FROM sqlite_master WHERE name='admins';" \
  "PRAGMA integrity_check;" \
  "PRAGMA foreign_key_check;"

echo
echo "== 升级后 check"
"$BIN" check --data-dir "$DATA5" > "$WORK/check-v5-after.log" 2>&1
echo "check_after_exit=$?"
grep -E "数据库|schema|结果" "$WORK/check-v5-after.log" | head -4
rm -rf "$WORK"
echo "== 完成（临时目录已清理）"
