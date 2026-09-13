#!/bin/zsh
# QA 独立验收（T10）：合成 v4 库 → check 报"待迁移" → serve 自动升级 v5（迁移链只追加）。
# 说明：v4 库用 migrations/0001–0004 的 SQL 与与 sqlx 一致的 SHA-384 校验和构造
# （校验和算法先用全新 init 的 _sqlx_migrations 行反证，见 failpoint/migration 证据日志）。
set -u
BIN="$PWD/dist/aarch64-apple-darwin/everything-manual"
WORK="/tmp/em-t10-qa-mig-$$"
FRESH="$WORK/fresh"
DATA4="$WORK/data-v4"
mkdir -p "$FRESH" "$DATA4"
printf 'qa-mig-not-a-real-secret\n' > "$WORK/pw.txt"
chmod 600 "$WORK/pw.txt"
echo "== 二进制 sha256：$(shasum -a 256 "$BIN" | cut -d' ' -f1)"
echo "== 工作目录：$WORK"

"$BIN" init --data-dir "$FRESH" --password-file "$WORK/pw.txt" > "$WORK/fresh-init.log" 2>&1
echo "fresh_init_exit=$?"
cp -R "$FRESH/tmp" "$FRESH/logs" "$FRESH/blobs" "$FRESH/lock" "$DATA4/"
DB4="$DATA4/manual.sqlite3"
for f in migrations/0001_core_schema.sql migrations/0002_invariants.sql migrations/0003_photos_view_unique.sql migrations/0004_preparation_pages.sql; do
  sqlite3 "$DB4" < "$f" || { echo "应用 $f 失败"; exit 1; }
done
sqlite3 "$FRESH/manual.sqlite3" ".schema _sqlx_migrations" | sqlite3 "$DB4"
python3 - "$DB4" <<'PY'
import hashlib, sqlite3, sys
db = sqlite3.connect(sys.argv[1])
rows = [
    (1, "core schema", "migrations/0001_core_schema.sql"),
    (2, "invariants", "migrations/0002_invariants.sql"),
    (3, "photos view unique", "migrations/0003_photos_view_unique.sql"),
    (4, "preparation pages", "migrations/0004_preparation_pages.sql"),
]
for version, description, path in rows:
    digest = hashlib.sha384(open(path, "rb").read()).hexdigest()
    db.execute(
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time) "
        "VALUES (?, ?, '2026-09-12 00:00:00', 1, X'%s', 0)" % digest.upper(),
        (version, description),
    )
db.commit()
print("合成 v4 库：0001–0004 已应用，校验和与 sqlx 一致")
PY
sqlite3 "$DB4" "SELECT '迁移行数：'||COUNT(*)||'，最新 v'||MAX(version) FROM _sqlx_migrations;" \
               "SELECT 'job_stage_deps 存在：'||COUNT(*) FROM sqlite_master WHERE name='job_stage_deps';"

echo
echo "== check（只读，不得迁移）"
"$BIN" check --data-dir "$DATA4" > "$WORK/check-v4.log" 2>&1
echo "check_v4_exit=$?"
grep -E "数据库 schema|结果：" "$WORK/check-v4.log"
sqlite3 "$DB4" "SELECT 'check 后迁移行数：'||COUNT(*)||'，job_stage_deps 存在：'||(SELECT COUNT(*) FROM sqlite_master WHERE name='job_stage_deps');"

echo
echo "== serve（应自动迁移到 v5）"
"$BIN" serve --data-dir "$DATA4" --listen 127.0.0.1:18097 > "$WORK/serve-v4.log" 2>&1 &
SV=$!
sleep 3
grep -o '"event":"database_ready"[^}]*' "$WORK/serve-v4.log" | head -1
kill -TERM "$SV" 2>/dev/null; wait "$SV" 2>/dev/null
echo "serve_v4_exit=$?"
sqlite3 "$DB4" "SELECT '升级后迁移行数：'||COUNT(*)||'，最新 v'||MAX(version) FROM _sqlx_migrations;" \
               "SELECT 'job_stage_deps 存在：'||COUNT(*) FROM sqlite_master WHERE name='job_stage_deps';" \
               "SELECT 'job_stages 新列数：'||COUNT(*) FROM pragma_table_info('job_stages') WHERE name IN ('poll_count','last_error','needs_input_json');" \
               "SELECT 'admins 保留：'||COUNT(*) FROM admins;" \
               "PRAGMA integrity_check;" \
               "PRAGMA foreign_key_check;"
echo "== 升级后 check"
"$BIN" check --data-dir "$DATA4" > "$WORK/check-v4-after.log" 2>&1
echo "check_after_exit=$?"
grep -E "数据库 schema|结果：" "$WORK/check-v4-after.log"
rm -rf "$WORK"
echo "== 完成（临时目录已清理）"
