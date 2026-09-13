#!/bin/bash
# QA 回合 25 · T20 独立验收 · 阶段 1
# 目的：AC-009 的"运行中拒绝 + 停服后成功"，以及"备份不是复制运行中 WAL 主文件"的独立证据。
# 全部结论来自本脚本的真实命令输出（原始日志 qa-phase1.log）。
set -u
BIN=/Users/qsyj/Code/rust/everything-manual/dist/aarch64-apple-darwin/everything-manual
QA=/Users/qsyj/Code/rust/everything-manual/artifacts/web-mvp/t20-qa
WORK=/tmp/em-t20-qa
DATA=$WORK/data
PORT=18080
BASE=http://127.0.0.1:$PORT
JAR=$WORK/cookies.txt
PASSWORD='test-password-t20-backup'

step() { printf '\n=== [%s] %s\n' "$1" "$2"; }

step 0 "启动真实 serve（dist 二进制，持有 data-dir 排他锁）"
cd "$WORK" || exit 9
"$BIN" serve --data-dir "$DATA" --listen 127.0.0.1:$PORT > "$WORK/serve.log" 2>&1 &
SERVE_PID=$!
echo "serve pid=$SERVE_PID"
for i in $(seq 1 50); do
  if curl -s -o /dev/null "$BASE/api/v1/health/ready"; then break; fi
  sleep 0.2
done
curl -s "$BASE/api/v1/health/ready" | head -c 300; echo

step 1 "登录（拿到会话 cookie；备份必须不含该会话）"
LOGIN=$(curl -s -D "$WORK/login.headers" -c "$JAR" -H 'Content-Type: application/json' \
  -H "Origin: $BASE" -d "{\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login")
echo "login body: $(echo "$LOGIN" | head -c 200)"
CSRF=$(echo "$LOGIN" | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["csrfToken"])')
SESSION_TOKEN=$(python3 - "$JAR" <<'PY'
import sys
for line in open(sys.argv[1]):
    if line.startswith("#") or not line.strip():
        continue
    parts = line.split("\t")
    if len(parts) >= 7 and parts[5] == "em_session":
        print(parts[6])
PY
)
echo "csrf=${CSRF:0:12}... session_token=${SESSION_TOKEN:0:12}..."
echo "$SESSION_TOKEN" > "$WORK/session-token.txt"

step 2 "运行中执行 backup -> 期望退出码 5、说明需先停服、不创建输出"
"$BIN" backup --data-dir "$DATA" --out "$WORK/backup-while-running" \
  > "$WORK/backup-running.out" 2> "$WORK/backup-running.err"
echo "exit=$?"
echo "--- stdout ---"; cat "$WORK/backup-running.out"
echo "--- stderr ---"; cat "$WORK/backup-running.err"
if [ -e "$WORK/backup-while-running" ]; then echo "FAIL: 被拒绝时创建了输出"; else echo "OK: 被拒绝时未创建输出目录"; fi

step 3 "运行中写入新数据（走真实 HTTP；commit 只会进 WAL）"
ITEM_JSON=$(curl -s -b "$JAR" -X POST -H 'Content-Type: application/json' \
  -H "Origin: $BASE" -H "X-CSRF-Token: $CSRF" \
  -d '{"name":"QA 独立验收物品","brand":"QA","model":"WAL-ONLY-2026"}' "$BASE/api/v1/items")
echo "create item: $(echo "$ITEM_JSON" | head -c 300)"
NEW_ITEM_ID=$(echo "$ITEM_JSON" | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["id"])' 2>/dev/null)
echo "$NEW_ITEM_ID" > "$WORK/new-item-id.txt"
echo "new item id: $NEW_ITEM_ID"

step 4 "记录源库/WAL 指纹（主文件在 kill -9 之后仍不得包含新数据）"
sqlite3 "file:$DATA/manual.sqlite3?mode=ro" "SELECT COUNT(*) FROM items;" > "$WORK/main-copy-count-before.txt" 2>&1
echo "主文件（未含 WAL）items 数 = $(cat "$WORK/main-copy-count-before.txt")"
shasum -a 256 "$DATA/manual.sqlite3" | tee "$WORK/maindb.sha.before"
ls -l "$DATA/manual.sqlite3"* | tee "$WORK/dbfiles.before"

step 5 "kill -9（模拟崩溃停机：锁由内核释放，WAL 里保留已提交事务）"
kill -9 "$SERVE_PID"
sleep 0.5
if kill -0 "$SERVE_PID" 2>/dev/null; then echo "FAIL: serve 仍在运行"; else echo "OK: serve 已终止"; fi
ls -l "$DATA/manual.sqlite3"* | tee "$WORK/dbfiles.after-kill"

step 6 "负对照：把主文件复制到别处读（模拟'只复制主文件'的备份）"
cp "$DATA/manual.sqlite3" "$WORK/naive-copy.sqlite3"
sqlite3 "file:$WORK/naive-copy.sqlite3?mode=ro" "SELECT COUNT(*) FROM items;" > "$WORK/naive-count.txt" 2>&1
echo "只复制主文件的副本 items 数 = $(cat "$WORK/naive-count.txt")"
sqlite3 "file:$WORK/naive-copy.sqlite3?mode=ro" "SELECT COUNT(*) FROM items WHERE id='$NEW_ITEM_ID';" > "$WORK/naive-newitem.txt" 2>&1
echo "只复制主文件的副本中是否含新物品 = $(cat "$WORK/naive-newitem.txt") （1=含，0=不含）"

step 7 "停服后 backup -> 期望退出码 0"
SOURCE_FP_BEFORE=$(shasum -a 256 "$DATA/manual.sqlite3" "$DATA"/blobs/*/* | shasum -a 256 | awk '{print $1}')
echo "源 data-dir 全量指纹（主文件+全部 blob）前置：$SOURCE_FP_BEFORE"
"$BIN" backup --data-dir "$DATA" --out "$WORK/my-backup" > "$WORK/backup-ok.out" 2> "$WORK/backup-ok.err"
echo "exit=$?"
cat "$WORK/backup-ok.out"
cat "$WORK/backup-ok.err"
SOURCE_FP_AFTER=$(shasum -a 256 "$DATA/manual.sqlite3" "$DATA"/blobs/*/* | shasum -a 256 | awk '{print $1}')
echo "源 data-dir 全量指纹（主文件+全部 blob）后置：$SOURCE_FP_AFTER"
if [ "$SOURCE_FP_BEFORE" = "$SOURCE_FP_AFTER" ]; then echo "OK: 备份前后源数据逐字节一致"; else echo "FAIL: 源数据被改动"; fi

step 8 "快照里必须含 kill -9 前已提交的新物品（证明快照含 WAL 中已提交事务）"
SNAP=$WORK/my-backup/database/manual.sqlite3
sqlite3 "file:$SNAP?mode=ro" "SELECT COUNT(*) FROM items;" | tee "$WORK/snapshot-count.txt"
sqlite3 "file:$SNAP?mode=ro" "SELECT COUNT(*) FROM items WHERE id='$NEW_ITEM_ID';" | tee "$WORK/snapshot-newitem.txt"
sqlite3 "file:$SNAP?mode=ro" "SELECT name,model FROM items ORDER BY created_at;" | tee "$WORK/snapshot-items.txt"

step 9 "快照内容检查：sessions=0、管理员口令哈希保留、外键完整、无 -wal/-shm"
sqlite3 "file:$SNAP?mode=ro" "SELECT COUNT(*) FROM sessions;" | tee "$WORK/snapshot-sessions.txt"
sqlite3 "file:$SNAP?mode=ro" "SELECT COUNT(*) FROM admins WHERE password_hash IS NOT NULL AND length(password_hash)>0;" | tee "$WORK/snapshot-admin.txt"
sqlite3 "file:$SNAP?mode=ro" "PRAGMA foreign_key_check;" | tee "$WORK/snapshot-fk.txt"
sqlite3 "file:$SNAP?mode=ro" "PRAGMA integrity_check;" | tee "$WORK/snapshot-integrity.txt"
ls -l "$WORK/my-backup/database/"

step 10 "SHA256SUMS 用系统 shasum 全量校验 + 快照 sha256 与 manifest 一致"
(cd "$WORK/my-backup" && shasum -a 256 -c SHA256SUMS) | tee "$WORK/shasums-check.txt" | tail -5
MANIFEST_DB_SHA=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["database"]["sha256"])' "$WORK/my-backup/manifest.json")
ACTUAL_DB_SHA=$(shasum -a 256 "$SNAP" | awk '{print $1}')
echo "manifest.database.sha256 = $MANIFEST_DB_SHA"
echo "实际快照 sha256          = $ACTUAL_DB_SHA"
[ "$MANIFEST_DB_SHA" = "$ACTUAL_DB_SHA" ] && echo "OK: 快照与 manifest 一致" || echo "FAIL: 不一致"

step 11 "已存在的输出路径不被覆盖（再备份一次 -> 退出码 4，产物逐字节不变）"
MANIFEST_BEFORE=$(shasum -a 256 "$WORK/my-backup/manifest.json" | awk '{print $1}')
"$BIN" backup --data-dir "$DATA" --out "$WORK/my-backup" > "$WORK/backup-again.out" 2> "$WORK/backup-again.err"
echo "exit=$?"
cat "$WORK/backup-again.err"
MANIFEST_AFTER=$(shasum -a 256 "$WORK/my-backup/manifest.json" | awk '{print $1}')
[ "$MANIFEST_BEFORE" = "$MANIFEST_AFTER" ] && echo "OK: 已有备份逐字节不变" || echo "FAIL: 已有备份被改动"

step 12 "备份输出在 data-dir 内（嵌套）-> 退出码 4"
"$BIN" backup --data-dir "$DATA" --out "$DATA/inside" > "$WORK/backup-nested.out" 2> "$WORK/backup-nested.err"
echo "exit=$?"; cat "$WORK/backup-nested.err"
[ -e "$DATA/inside" ] && echo "FAIL: 创建了嵌套输出" || echo "OK: 未创建嵌套输出"

echo
echo "phase1 完成"
