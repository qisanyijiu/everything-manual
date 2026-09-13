#!/usr/bin/env bash
# 真实环境：上传流写入中途磁盘被占满（ENOSPC 发生在写流阶段，预检/复检均已通过）。
# 观察：是否半提交（资产行/ blob 行 / tmp / blobs 文件），以及响应形态。
# 用法：bash disk-full-midwrite.sh <binary> <assets-dir>
set -u
BIN="${1:?binary}"
ASSETS="${2:?assets}"
IMG=/tmp/em-t06-qa-disk2.dmg
MNT=/tmp/em-t06-qa-mnt2
PORT=18099
URL="http://127.0.0.1:${PORT}"
PASSWORD='qa-midwrite-password-3b71'
SERVER_PID=""
cleanup() {
  [ -n "$SERVER_PID" ] && { kill "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null; }
  rm -f "$MNT/filler.bin" 2>/dev/null
  hdiutil detach "$MNT" >/dev/null 2>&1 || hdiutil detach -force "$MNT" >/dev/null 2>&1
  rm -f "$IMG"; rmdir "$MNT" 2>/dev/null
}
trap cleanup EXIT

echo "== 0) 建 64 MiB 镜像并挂载"
rm -f "$IMG"; mkdir -p "$MNT"
hdiutil create -size 64m -fs APFS -volname EMQA2 -quiet "$IMG" || exit 1
hdiutil attach -quiet -mountpoint "$MNT" "$IMG" || exit 1
df -h "$MNT" | sed 's/^/  /'

echo "== 1) init + serve"
printf '%s\n' "$PASSWORD" >"$MNT/pw.txt"; chmod 600 "$MNT/pw.txt"
"$BIN" init --data-dir "$MNT/data" --password-file "$MNT/pw.txt" >"$MNT/init.log" 2>&1
echo "  init exit=$?"
"$BIN" serve --data-dir "$MNT/data" --listen "127.0.0.1:${PORT}" >"$MNT/serve.log" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 80); do curl -sf "$URL/api/v1/health/live" >/dev/null 2>&1 && break; sleep 0.1; done
curl -s -c "$MNT/cookies.txt" -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" "$URL/api/v1/auth/login" -o "$MNT/login.json"
CSRF="$(python3 -c "import json;print(json.load(open('$MNT/login.json'))['data']['csrfToken'])")"
ITEM="$(python3 -c 'import uuid;print(uuid.uuid4())')"
NOW="$(python3 -c 'import time;print(int(time.time()*1000))')"
sqlite3 "$MNT/data/manual.sqlite3" "INSERT INTO items (id,name,brand,model,variant,revision,archived_at,created_at,updated_at) VALUES ('$ITEM','中途满','QA','M-1',NULL,1,NULL,$NOW,$NOW);"
echo "  item=$ITEM  文件大小=$(wc -c <"$ASSETS/big.pdf" | tr -d ' ')"

echo "== 2) 限速上传 40 MiB，同时用填充文件占满磁盘"
( curl -s --limit-rate 2500k -o "$MNT/upload.body" -D "$MNT/upload.hdr" -w '%{http_code}' \
  -b "$MNT/cookies.txt" -H "x-csrf-token: $CSRF" \
  -F 'purpose=document' -F "file=@$ASSETS/big.pdf;type=application/pdf" \
  "$URL/api/v1/items/$ITEM/assets" >"$MNT/upload.code" 2>"$MNT/upload.err" ) &
UPLOAD_PID=$!
sleep 4
echo "  填充磁盘（dd 可能因空间不足退出，属预期）"
dd if=/dev/zero of="$MNT/filler.bin" bs=1m count=60 >/dev/null 2>"$MNT/dd.err" || echo "    dd 退出非零（磁盘已满）：$(head -1 "$MNT/dd.err")"
df -h "$MNT" | sed 's/^/  /'
wait "$UPLOAD_PID"
echo "  上传 HTTP 状态：$(cat "$MNT/upload.code")"
echo "  响应体：$(head -c 400 "$MNT/upload.body")"
echo "  日志（磁盘/写入相关）："
grep -o '"level":"ERROR"[^}]*' "$MNT/serve.log" | tail -3 | sed 's/^/    /'
grep -o '"stage":"upload_tmp[^}]*' "$MNT/serve.log" | head -2 | sed 's/^/    /'

echo "== 3) 半提交核对"
echo "  assets 行：$(sqlite3 "$MNT/data/manual.sqlite3" 'SELECT COUNT(*) FROM assets;')"
echo "  blobs  行：$(sqlite3 "$MNT/data/manual.sqlite3" 'SELECT COUNT(*) FROM blobs;')"
echo "  tmp 文件数：$(find "$MNT/data/tmp" -type f | wc -l | tr -d ' ')"
echo "  blobs 文件数：$(find "$MNT/data/blobs" -type f | wc -l | tr -d ' ')"
echo "  blobs 文件大小：$(find "$MNT/data/blobs" -type f -exec wc -c {} \; | tr -s ' ' | sed 's/^/    /')"
echo "  quarantine：$(ls "$MNT/data/quarantine" 2>/dev/null | wc -l | tr -d ' ')"
echo "== 结束（镜像将在清理时卸载并删除）"
