#!/usr/bin/env bash
# QA 回合 29（T21）· 崩溃断点「blob rename 后 DB 事务前」在**正式 dist 二进制**上的
# 真实进程重启核对（validation-release §3：每个断点重启后核对任务数、远端请求数、
# 费用预留、资产引用与不可变版本）。
#
# 做法：
#  1) init 临时 data-dir → serve（正式二进制）→ 登录 → 建物品 → 上传 PDF（201）；
#  2) SIGKILL 服务（硬崩溃）；
#  3) 构造崩溃现场：blobs/<前缀>/<sha256> 已 rename 落地但元数据未提交（孤儿文件），
#     外加一个 rename 之前的 tmp 残留；
#  4) 重启 serve（同一 data-dir）→ 执行启动例程（含资产扫描）；
#  5) 核对：被引用资产逐字节可读；孤儿/tmp 进 quarantine；jobs/cost_ledger/
#     provider_attempts/model_revisions 行数不变；孤儿内容重传 201 且 blob 行不重复。
#
# 用法：bash qa-r29-blob-breakpoint-dist.sh <dist 二进制绝对路径> [端口]
set -euo pipefail

BIN="${1:?用法: qa-r29-blob-breakpoint-dist.sh <binary> [port]}"
PORT="${2:-19181}"
WORK="$(mktemp -d /tmp/em-r29-blobfp-XXXXXX)"
DATA="$WORK/data"
PASS_FILE="$WORK/password.txt"
LOG1="$WORK/serve-1.log"
LOG2="$WORK/serve-2.log"
COOKIE="$WORK/cookies.txt"
BASE="http://127.0.0.1:$PORT"
PASSWORD="qa-r29-blobfp-password-3d21"
SERVER_PID=""

cleanup() {
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

wait_health() {
  for _ in $(seq 1 150); do
    if curl -fsS "$BASE/health/live" > /dev/null 2>&1; then return 0; fi
    sleep 0.1
  done
  echo "服务未就绪：$BASE/health/live" >&2
  return 1
}

count() { # 表行数（sqlite3）
  sqlite3 "$DATA/manual.sqlite3" "SELECT COUNT(*) FROM $1;"
}

echo "== QA R29 断点四（blob rename 后 DB 事务前）· dist 二进制真实重启 =="
echo "binary:    $BIN"
echo "sha256:    $(shasum -a 256 "$BIN" | awk '{print $1}')"
echo "time:      $(date '+%Y-%m-%d %H:%M:%S %Z')"
echo "data-dir:  $DATA"

printf '%s\n' "$PASSWORD" > "$PASS_FILE"
chmod 600 "$PASS_FILE"
"$BIN" init --data-dir "$DATA" --password-file "$PASS_FILE" > "$WORK/init.log" 2>&1

"$BIN" serve --data-dir "$DATA" --listen "127.0.0.1:$PORT" > "$LOG1" 2>&1 &
SERVER_PID=$!
wait_health

LOGIN="$(curl -fsS -c "$COOKIE" -H 'content-type: application/json' \
  -d "{\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login")"
CSRF="$(printf '%s' "$LOGIN" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["csrfToken"])')"

ITEM="$(curl -fsS -b "$COOKIE" -H "x-csrf-token: $CSRF" -H 'content-type: application/json' \
  -d '{"name":"断点四物品","brand":"QA","model":"R29-BLOB"}' "$BASE/api/v1/items" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])')"

PDF="$WORK/sample.pdf"
cp "$(dirname "$0")/../../../tests/fixtures/assets/sample-manual-text.pdf" "$PDF"
ASSET_JSON="$(curl -fsS -b "$COOKIE" -H "x-csrf-token: $CSRF" \
  -F purpose=document -F "file=@$PDF;type=application/pdf" \
  "$BASE/api/v1/items/$ITEM/assets")"
ASSET="$(printf '%s' "$ASSET_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["id"])')"
SHA="$(printf '%s' "$ASSET_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["sha256"])')"
BLOB="$(printf '%s' "$SHA" | cut -c1-2)"
BLOB_PATH="$DATA/blobs/$BLOB/$SHA"
[[ -f "$BLOB_PATH" ]] || { echo "被引用 blob 未落盘：$BLOB_PATH" >&2; exit 1; }
echo "已上传资产：asset=$ASSET sha256=$SHA"

JOBS_BEFORE="$(count jobs)"
LEDGER_BEFORE="$(count cost_ledger)"
ATTEMPTS_BEFORE="$(count provider_attempts)"
REVISIONS_BEFORE="$(count model_revisions)"
echo "崩溃前：jobs=$JOBS_BEFORE cost_ledger=$LEDGER_BEFORE provider_attempts=$ATTEMPTS_BEFORE model_revisions=$REVISIONS_BEFORE"

# --- 硬崩溃 ---
kill -9 "$SERVER_PID"
wait "$SERVER_PID" 2>/dev/null || true
SERVER_PID=""
echo "已 kill -9 服务（pid 参见日志）"

# --- 崩溃现场：rename 已落地、元数据未提交；另有 tmp 残留 ---
# 孤儿内容用**另一份合法 PDF**（与已上传的不同 sha）：后续"重传该内容必须 201"才可验证。
ORPHAN_FILE="$WORK/orphan.pdf"
cp "$(dirname "$0")/../../../tests/fixtures/assets/sample-manual-rotated.pdf" "$ORPHAN_FILE"
ORPHAN_SHA="$(shasum -a 256 "$ORPHAN_FILE" | awk '{print $1}')"
ORPHAN_PREFIX="$(printf '%s' "$ORPHAN_SHA" | cut -c1-2)"
mkdir -p "$DATA/blobs/$ORPHAN_PREFIX" "$DATA/tmp"
cp "$ORPHAN_FILE" "$DATA/blobs/$ORPHAN_PREFIX/$ORPHAN_SHA"
printf 'partial-upload' > "$DATA/tmp/qa-r29-crash.part"
echo "已注入孤儿 blob：blobs/$ORPHAN_PREFIX/${ORPHAN_SHA}（无元数据行）与 tmp 残留"

# --- 重启（同一 data-dir） ---
"$BIN" serve --data-dir "$DATA" --listen "127.0.0.1:$PORT" > "$LOG2" 2>&1 &
SERVER_PID=$!
wait_health
echo "服务已重启"

FAIL=0
check() { # 描述 期望 实际
  if [[ "$2" == "$3" ]]; then echo "  [OK]   $1：$3"; else echo "  [FAIL] $1：期望 $2，实际 $3"; FAIL=1; fi
}

# 被引用资产仍可逐字节读回。
curl -fsS -b "$COOKIE" -o "$WORK/readback.pdf" "$BASE/api/v1/assets/$ASSET/content"
READBACK_SHA="$(shasum -a 256 "$WORK/readback.pdf" | awk '{print $1}')"
check "被引用资产逐字节可读（sha256）" "$SHA" "$READBACK_SHA"
check "任务数不变（jobs）" "$JOBS_BEFORE" "$(count jobs)"
check "远端请求不变（provider_attempts）" "$ATTEMPTS_BEFORE" "$(count provider_attempts)"
check "费用预留不变（cost_ledger）" "$LEDGER_BEFORE" "$(count cost_ledger)"
check "不可变版本不变（model_revisions）" "$REVISIONS_BEFORE" "$(count model_revisions)"
check "孤儿 blob 已被隔离（离开内容寻址位置）" "0" "$([[ -f "$DATA/blobs/$ORPHAN_PREFIX/$ORPHAN_SHA" ]] && echo 1 || echo 0)"
check "tmp 残留已清空" "0" "$(find "$DATA/tmp" -type f | wc -l | tr -d ' ')"
check "隔离区含 2 个文件（孤儿 blob + tmp 残留）" "2" "$(find "$DATA/quarantine" -type f | wc -l | tr -d ' ')"

# 孤儿内容重传 → 201，且不存在重复 blob 行。
BLOB_ROWS_BEFORE="$(count blobs)"
STATUS="$(curl -sS -o /dev/null -w '%{http_code}' -b "$COOKIE" -H "x-csrf-token: $CSRF" \
  -F purpose=document -F "file=@$ORPHAN_FILE;type=application/pdf" \
  "$BASE/api/v1/items/$ITEM/assets")"
echo "重传孤儿内容 HTTP=${STATUS}（415/422 属预期外，201 才通过）"
if [[ "$STATUS" == "201" ]]; then echo "  [OK]   重传成功"; else echo "  [FAIL] 重传失败：HTTP $STATUS"; FAIL=1; fi
check "blob 行数只 +1（旧的被隔离文件不重复计数）" "$((BLOB_ROWS_BEFORE + 1))" "$(count blobs)"

echo
if [[ "$FAIL" == "0" ]]; then echo "结论：断点四在 dist 二进制真实重启下全部核对通过"; else echo "结论：存在失败项"; fi
echo "日志：serve-1=${LOG1} serve-2=${LOG2}（随临时目录清理，现场摘要在 stdout）"
exit "$FAIL"
