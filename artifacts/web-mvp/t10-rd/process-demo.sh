#!/bin/zsh
# T10 手工进程级演示：真实二进制 serve + 入队阶段 + kill -9 + 重启 + 观察恢复。
#
# 演示内容（原始输出见同目录 process-demo.log）：
#   1. init 一个全新 data-dir，写入一条 job 与两个阶段（freeze_inputs 已成功、manual_extract 待执行）；
#   2. 起 serve（真实 release 二进制）：任务执行器领取 manual_extract，但本卡未注册处理器
#      → 延后（不假成功、不消耗重试额度、不产生任何 attempt），并把原因写入 last_error；
#   3. kill -9（SIGKILL）服务进程：不做任何优雅退出；
#   4. 重启前把阶段改成"付费提交结果未知"的现场（running + 租约过期 + attempt=submitting）；
#   5. 重启 serve：执行器恢复扫描收敛为 submission_unknown（保留现场，等待对账），
#      之后不再领取该阶段；
#   6. 观察：进程重启不修改 job 数、不产生第二个 attempt、不发出任何外部请求。
set -u
BIN="$PWD/dist/aarch64-apple-darwin/everything-manual"
WORK="/tmp/em-t10-demo-$$"
DATA="$WORK/data"
mkdir -p "$WORK"
echo "== 工作目录：$WORK"
echo "== 二进制：$BIN（sha256 $(shasum -a 256 "$BIN" | cut -d' ' -f1)）"
echo "pw-demo-not-a-real-secret" > "$WORK/pw.txt"
chmod 600 "$WORK/pw.txt"

echo
echo "== [1] init"
"$BIN" init --data-dir "$DATA" --password-file "$WORK/pw.txt" 2>&1 | tail -4

NOW=$(python3 -c 'import time;print(int(time.time()*1000))')
DB="$DATA/manual.sqlite3"
echo
echo "== [2] 写入 job 与阶段（T11 之前没有建单 API，这里用 SQL 直接入队）"
sqlite3 "$DB" <<SQL
INSERT INTO items (id,name,brand,model,variant,revision,archived_at,created_at,updated_at)
  VALUES ('item-demo','演示物品','Demo','D1',NULL,1,NULL,$NOW,$NOW);
INSERT INTO blobs (sha256,size,mime,storage_state,created_at)
  VALUES ('$(printf 'a%.0s' {1..64})',1024,'application/pdf','stored',$NOW);
INSERT INTO assets (id,blob_id,item_id,purpose,original_name,created_at)
  VALUES ('asset-demo','$(printf 'a%.0s' {1..64})','item-demo','document','manual.pdf',$NOW);
INSERT INTO documents (id,item_id,source_asset_id,source_sha256,title,source_url,created_at,updated_at)
  VALUES ('doc-demo','item-demo','asset-demo','$(printf 'a%.0s' {1..64})','说明书',NULL,$NOW,$NOW);
INSERT INTO preparations (id,document_id,source_sha256,state,page_count,revision,client_derived,created_at,updated_at)
  VALUES ('prep-demo','doc-demo','$(printf 'a%.0s' {1..64})','ready',4,1,1,$NOW,$NOW);
INSERT INTO generation_snapshots
  (id,item_id,item_revision,preparation_id,photo_ids,photo_hashes,provider_config,prompt_version,price_version,budgets,created_at)
  VALUES ('snap-demo','item-demo',1,'prep-demo','["p1"]','["h1"]','{}','prompt-v1','price-v1','{}',$NOW);
INSERT INTO jobs (id,item_id,snapshot_id,status,revision,created_at,updated_at)
  VALUES ('job-demo','item-demo','snap-demo','running',1,$NOW,$NOW);
INSERT INTO job_stages (id,job_id,stage_kind,batch_index,page_set,input_hash,status,lease_epoch,attempt_count,created_at,updated_at)
  VALUES ('stage-freeze','job-demo','freeze_inputs',0,NULL,'hash-freeze','succeeded',0,0,$NOW,$NOW);
INSERT INTO job_stages (id,job_id,stage_kind,batch_index,page_set,input_hash,status,lease_epoch,attempt_count,created_at,updated_at)
  VALUES ('stage-batch','job-demo','manual_extract',0,'[1,2,3,4]','hash-batch','queued',0,0,$NOW,$NOW);
INSERT INTO job_stage_deps (stage_id,depends_on_stage_id,created_at)
  VALUES ('stage-batch','stage-freeze',$NOW);
SQL
sqlite3 "$DB" "SELECT '阶段入队：'||id||' status='||status||' epoch='||lease_epoch FROM job_stages ORDER BY id;"

echo
echo "== [3] 起 serve（真实 release 二进制，监听 127.0.0.1:0）"
"$BIN" serve --data-dir "$DATA" --listen 127.0.0.1:18099 > "$WORK/serve-1.log" 2>&1 &
PID1=$!
sleep 3
grep -o '"event":"job_executor_start"[^}]*' "$WORK/serve-1.log" | head -1
grep -o '"event":"job_executor_no_handlers"' "$WORK/serve-1.log" | head -1
sqlite3 "$DB" "SELECT '领取后延后：'||id||' status='||status||' attempt_count='||attempt_count||' lease_epoch='||lease_epoch||' last_error='||COALESCE(last_error,'(无)') FROM job_stages WHERE id='stage-batch';"
sqlite3 "$DB" "SELECT 'attempt 数：'||COUNT(*) FROM provider_attempts;"

echo
echo "== [4] kill -9（SIGKILL，无优雅退出）PID=$PID1"
kill -9 "$PID1"
wait "$PID1" 2>/dev/null
sleep 1
echo "进程状态：$(ps -p $PID1 > /dev/null 2>&1 && echo 存活 || echo 已终止)"

echo
echo "== [5] 造出"付费提交结果未知"现场（running + 租约过期 + attempt=submitting）"
PAST=$((NOW - 60000))
sqlite3 "$DB" <<SQL
UPDATE job_stages SET status='running', lease_owner='dead-worker', lease_epoch=lease_epoch+1, lease_until=$PAST WHERE id='stage-batch';
INSERT INTO provider_attempts (id,job_id,stage_id,request_hash,submit_state,remote_task_id,response_id,started_at,last_error,created_at,updated_at)
  VALUES ('attempt-demo','job-demo','stage-batch','hash-request','intent',NULL,NULL,$PAST,NULL,$PAST,$PAST);
SQL
sqlite3 "$DB" "UPDATE provider_attempts SET submit_state='submitting' WHERE id='attempt-demo';"
sqlite3 "$DB" "SELECT '重启前：'||id||' status='||status||' lease_epoch='||lease_epoch FROM job_stages WHERE id='stage-batch';"

echo
echo "== [6] 重启 serve（同一 data-dir）"
"$BIN" serve --data-dir "$DATA" --listen 127.0.0.1:18099 > "$WORK/serve-2.log" 2>&1 &
PID2=$!
sleep 3
grep -o '"event":"job_recovery"[^}]*' "$WORK/serve-2.log" | head -1
grep -o '"event":"job_recovery_applied"[^}]*' "$WORK/serve-2.log" | head -1
sqlite3 "$DB" "SELECT '重启后：'||id||' status='||status||' last_error='||COALESCE(substr(last_error,1,40),'(无)') FROM job_stages WHERE id='stage-batch';"
sqlite3 "$DB" "SELECT 'attempt：'||id||' submit_state='||submit_state||' remote_task_id='||COALESCE(remote_task_id,'(空)') FROM provider_attempts;"
sqlite3 "$DB" "SELECT 'job 数：'||COUNT(*) FROM jobs;"
sqlite3 "$DB" "SELECT 'attempt 数：'||COUNT(*) FROM provider_attempts;"

echo
echo "== [7] 收尾：结束本脚本启动的进程"
kill "$PID2" 2>/dev/null
wait "$PID2" 2>/dev/null
echo "== 完成（data-dir 保留在 $WORK 供人工检查）"
