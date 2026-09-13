#!/bin/zsh
# QA 独立进程级验收（T10 / 回合 10）：真实 release-dist 二进制。
#
# 覆盖（全部由 QA 另写，不引用 RD 的 process-demo.sh 结论）：
#   A. 全新 data-dir：init → check → schema v5 结构与既有表不变量抽查；
#   B. 迁移链：合成 v4 库（0001–0004 + 与 sqlx 一致的 sha384 校验和）→ check 报"待迁移"
#      → serve 自动迁移到 v5（只追加，不重建）；迁移后再 check 报"已就绪"；
#   C. serve 启停与执行器：job_executor_start / no_handlers；已入队阶段被延后
#      （不假成功、不消耗重试、不产生 attempt）；SIGTERM 优雅停止 + 排他锁释放；
#   D. 重启恢复：人为造"running + 租约过期 + attempt=submitting"现场，
#      重启后由恢复扫描收敛为 submission_unknown（不新增 job/attempt）。
set -u
BIN="$PWD/dist/aarch64-apple-darwin/everything-manual"
WORK="/tmp/em-t10-qa-$$"
DATA="$WORK/data"
DATA4="$WORK/data-v4"
mkdir -p "$WORK"
echo "== 工作目录：$WORK"
echo "== 二进制：$BIN"
echo "== sha256：$(shasum -a 256 "$BIN" | cut -d' ' -f1)"
printf 'qa-t10-not-a-real-secret\n' > "$WORK/pw.txt"
chmod 600 "$WORK/pw.txt"
DB="$DATA/manual.sqlite3"
DB4="$DATA4/manual.sqlite3"

echo
echo "== [A] 全新 data-dir：init + check"
"$BIN" init --data-dir "$DATA" --password-file "$WORK/pw.txt" > "$WORK/init.log" 2>&1
echo "init_exit=$?"
grep -o "schema v[0-9]*" "$WORK/init.log" | head -1
"$BIN" check --data-dir "$DATA" > "$WORK/check-fresh.log" 2>&1
echo "check_exit=$?"
grep -E "数据库 schema|迁移链" "$WORK/check-fresh.log"

echo
echo "-- schema 结构抽查（v5 追加物 + 既有表不变量）"
sqlite3 "$DB" "SELECT '迁移行数：'||COUNT(*) FROM _sqlx_migrations;" \
                "SELECT 'job_stage_deps 列：'||GROUP_CONCAT(name,',') FROM pragma_table_info('job_stage_deps');" \
                "SELECT 'job_stages 新列：'||GROUP_CONCAT(name,',') FROM pragma_table_info('job_stages') WHERE name IN ('poll_count','last_error','needs_input_json');" \
                "SELECT 'job_stages 唯一键：'||GROUP_CONCAT(name,'+') FROM (SELECT name FROM pragma_index_list('job_stages') WHERE \"unique\"=1);" \
                "PRAGMA foreign_keys;" \
                "PRAGMA journal_mode;" \
                "PRAGMA integrity_check;" 2>&1

echo
echo "== [B] 合成 v4 库 → check 报待迁移 → serve 升级到 v5"
mkdir -p "$DATA4"
cp -R "$DATA/logs" "$DATA/blobs" "$DATA/lock" "$DATA4/" 2>/dev/null
rm -f "$DB4"
sqlite3 "$DB4" < migrations/0001_core_schema.sql 2>&1 | head -3
sqlite3 "$DB4" < migrations/0002_invariants.sql 2>&1 | head -3
sqlite3 "$DB4" < migrations/0003_photos_view_unique.sql 2>&1 | head -3
sqlite3 "$DB4" < migrations/0004_preparation_pages.sql 2>&1 | head -3
sqlite3 "$DATA/manual.sqlite3" ".schema _sqlx_migrations" | sqlite3 "$DB4"
python3 - "$DB4" <<'PY'
import hashlib, sqlite3, sys
db = sqlite3.connect(sys.argv[1])
rows = [
    (1, "core schema", "migrations/0001_core_schema.sql", 0),
    (2, "invariants", "migrations/0002_invariants.sql", 0),
    (3, "photos view unique", "migrations/0003_photos_view_unique.sql", 0),
    (4, "preparation pages", "migrations/0004_preparation_pages.sql", 0),
]
for version, description, path, ms in rows:
    digest = hashlib.sha384(open(path, "rb").read()).hexdigest()
    db.execute(
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time) "
        "VALUES (?, ?, '2026-09-12 00:00:00', 1, X'%s', %d)" % (digest.upper(), ms),
        (version, description),
    )
db.commit()
db.close()
print("合成 v4 库完成（0001–0004 + 校验和）")
PY
sqlite3 "$DB4" "SELECT '迁移行数：'||COUNT(*)||'，最新 v'||MAX(version) FROM _sqlx_migrations;"
"$BIN" check --data-dir "$DATA4" > "$WORK/check-v4.log" 2>&1
echo "check_v4_exit=$?"
grep -E "数据库 schema" "$WORK/check-v4.log"

"$BIN" serve --data-dir "$DATA4" --listen 127.0.0.1:18097 > "$WORK/serve-v4.log" 2>&1 &
SVPID=$!
sleep 3
grep -o '"event":"database_ready"[^}]*' "$WORK/serve-v4.log" | head -1
kill "$SVPID" 2>/dev/null; wait "$SVPID" 2>/dev/null
echo "serve_v4_exit=$?"
sqlite3 "$DB4" "SELECT '升级后迁移行数：'||COUNT(*)||'，最新 v'||MAX(version) FROM _sqlx_migrations;" \
               "SELECT 'job_stage_deps 存在：'||COUNT(*) FROM sqlite_master WHERE name='job_stage_deps';" \
               "SELECT 'job_stages 新列数：'||COUNT(*) FROM pragma_table_info('job_stages') WHERE name IN ('poll_count','last_error','needs_input_json');" \
               "PRAGMA integrity_check;"
"$BIN" check --data-dir "$DATA4" > "$WORK/check-v4-after.log" 2>&1
echo "check_v4_after_exit=$?"
grep -E "数据库 schema" "$WORK/check-v4-after.log"

echo
echo "== [C] serve 启停 + 执行器（已在队列的阶段被延后）"
NOW=$(python3 -c 'import time;print(int(time.time()*1000))')
SHA=$(python3 -c "import hashlib;print(hashlib.sha256(b'qa-t10').hexdigest())")
sqlite3 "$DB" <<SQL
INSERT INTO items (id,name,brand,model,variant,revision,archived_at,created_at,updated_at)
  VALUES ('item-qa','QA 物品','QA','Q1',NULL,1,NULL,$NOW,$NOW);
INSERT INTO blobs (sha256,size,mime,storage_state,created_at)
  VALUES ('$SHA',128,'application/pdf','stored',$NOW);
INSERT INTO assets (id,blob_id,item_id,purpose,original_name,created_at)
  VALUES ('asset-qa','$SHA','item-qa','document','qa.pdf',$NOW);
INSERT INTO documents (id,item_id,source_asset_id,source_sha256,title,source_url,created_at,updated_at)
  VALUES ('doc-qa','item-qa','asset-qa','$SHA','QA 说明书',NULL,$NOW,$NOW);
INSERT INTO preparations (id,document_id,source_sha256,state,page_count,revision,client_derived,created_at,updated_at)
  VALUES ('prep-qa','doc-qa','$SHA','ready',4,1,1,$NOW,$NOW);
INSERT INTO generation_snapshots
  (id,item_id,item_revision,preparation_id,photo_ids,photo_hashes,provider_config,prompt_version,price_version,budgets,created_at)
  VALUES ('snap-qa','item-qa',1,'prep-qa','["p1"]','["h1"]','{}','prompt-v1','price-v1','{}',$NOW);
INSERT INTO jobs (id,item_id,snapshot_id,status,revision,created_at,updated_at)
  VALUES ('job-qa','item-qa','snap-qa','queued',1,$NOW,$NOW);
INSERT INTO job_stages (id,job_id,stage_kind,batch_index,page_set,input_hash,status,lease_epoch,attempt_count,created_at,updated_at)
  VALUES ('stage-qa-freeze','job-qa','freeze_inputs',0,NULL,'h-freeze','succeeded',0,0,$NOW,$NOW);
INSERT INTO job_stages (id,job_id,stage_kind,batch_index,page_set,input_hash,status,lease_epoch,attempt_count,created_at,updated_at)
  VALUES ('stage-qa-batch','job-qa','manual_extract',0,'[1,2,3,4]','h-batch','queued',0,0,$NOW,$NOW);
INSERT INTO job_stage_deps (stage_id,depends_on_stage_id,created_at)
  VALUES ('stage-qa-batch','stage-qa-freeze',$NOW);
SQL
sqlite3 "$DB" "SELECT '入队：'||id||' status='||status FROM job_stages ORDER BY id;"

"$BIN" serve --data-dir "$DATA" --listen 127.0.0.1:18098 > "$WORK/serve-1.log" 2>&1 &
PID1=$!
sleep 4
echo "serve1_pid=$PID1"
grep -o '"event":"job_executor_start"[^}]*' "$WORK/serve-1.log" | head -1
grep -o '"event":"job_executor_no_handlers"' "$WORK/serve-1.log" | head -1
grep -o '"event":"job_stage_handler_missing"[^}]*' "$WORK/serve-1.log" | head -1
sqlite3 "$DB" "SELECT '延后后：id='||id||' status='||status||' attempt_count='||attempt_count||' lease_owner='||COALESCE(lease_owner,'(空)')||' next_run_at_set='||CASE WHEN next_run_at IS NULL THEN 'no' ELSE 'yes' END FROM job_stages WHERE id='stage-qa-batch';" \
               "SELECT 'last_error：'||COALESCE(substr(last_error,1,60),'(空)') FROM job_stages WHERE id='stage-qa-batch';" \
               "SELECT 'provider_attempts 行数：'||COUNT(*) FROM provider_attempts;" \
               "SELECT 'job 状态：'||status FROM jobs WHERE id='job-qa';"

echo
echo "-- SIGTERM 优雅停止"
kill -TERM "$PID1"
wait "$PID1" 2>/dev/null
echo "serve1_exit=$?"
grep -o '"event":"job_executor_stopped"[^}]*' "$WORK/serve-1.log" | head -1
grep -o '"event":"serve_stop"' "$WORK/serve-1.log" | head -1
grep -o "已停止：data-dir 排他锁已释放。" "$WORK/serve-1.log" | head -1

echo
echo "== [D] 重启恢复：造 'running + 租约过期 + attempt=submitting' 现场"
PAST=$((NOW - 120000))
sqlite3 "$DB" "UPDATE job_stages SET status='running', lease_owner='qa-dead-worker', lease_epoch=lease_epoch+1, lease_until=$PAST WHERE id='stage-qa-batch';" \
               "INSERT INTO provider_attempts (id,job_id,stage_id,request_hash,submit_state,remote_task_id,response_id,started_at,last_error,created_at,updated_at) VALUES ('attempt-qa','job-qa','stage-qa-batch','h-request','intent',NULL,NULL,$PAST,NULL,$PAST,$PAST);" \
               "UPDATE provider_attempts SET submit_state='submitting' WHERE id='attempt-qa';" \
               "SELECT '重启前：'||id||' status='||status||' epoch='||lease_epoch FROM job_stages WHERE id='stage-qa-batch';"

"$BIN" serve --data-dir "$DATA" --listen 127.0.0.1:18098 > "$WORK/serve-2.log" 2>&1 &
PID2=$!
sleep 4
echo "serve2_pid=$PID2"
grep -o '"event":"job_recovery"[^}]*' "$WORK/serve-2.log" | head -1
grep -o '"event":"job_recovery_applied"[^}]*' "$WORK/serve-2.log" | head -1
sqlite3 "$DB" "SELECT '重启后：'||id||' status='||status||' last_error='||COALESCE(substr(last_error,1,50),'(空)') FROM job_stages WHERE id='stage-qa-batch';" \
               "SELECT 'attempt：'||id||' submit_state='||submit_state||' remote_task_id='||COALESCE(remote_task_id,'(空)') FROM provider_attempts;" \
               "SELECT 'job 数：'||COUNT(*) FROM jobs;" \
               "SELECT 'attempt 数：'||COUNT(*) FROM provider_attempts;" \
               "SELECT 'job 状态：'||status FROM jobs WHERE id='job-qa';"

echo
echo "== [E] 收尾（只结束本脚本启动的进程；临时目录删除）"
kill -TERM "$PID2" 2>/dev/null; wait "$PID2" 2>/dev/null
echo "serve2_exit=$?"
sleep 1
if pgrep -f "everything-manual serve --data-dir $WORK" > /dev/null; then
  echo "残留进程：有（异常）"; pgrep -fl "everything-manual serve --data-dir $WORK"
else
  echo "残留进程：无"
fi
rm -rf "$WORK"
echo "== 完成（临时目录已清理）"
