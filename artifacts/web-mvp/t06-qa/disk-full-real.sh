#!/usr/bin/env bash
# 磁盘不足的**真实环境**验证：小磁盘镜像上跑 release 二进制（真实 statvfs，无注入）。
# 用法：bash disk-full-real.sh <binary> <assets-dir>
set -u
BIN="${1:?binary}"
ASSETS="${2:?assets}"
IMG=/tmp/em-t06-qa-disk.dmg
MNT=/tmp/em-t06-qa-mnt
PORT=18098
URL="http://127.0.0.1:${PORT}"
PASSWORD='qa-diskfull-password-5e19'
PASS=0
FAIL=0
SERVER_PID=""
ok() { PASS=$((PASS + 1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL + 1)); echo "  FAIL  $1"; }
chk() { if [ "$2" = "$3" ]; then ok "$1 = $3"; else bad "$1: expected [$2] got [$3]"; fi; }
log() { echo; echo "== $*"; }
req() { local name="$1"; shift; curl -s -o "$MNT/$name.body" -D "$MNT/$name.hdr" -w '%{http_code}' "$@"; }
code() { python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['error']['code'])" "$MNT/$1.body" 2>/dev/null || echo NO-JSON; }
jdet() { python3 -c "import json,sys;e=json.load(open(sys.argv[1]))['error'];d=e.get('details') or {};print(d.get(sys.argv[2],'NO-KEY'))" "$MNT/$1.body" "$2" 2>/dev/null || echo NO-DETAIL; }

cleanup() {
  [ -n "$SERVER_PID" ] && { kill "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null; }
  hdiutil detach "$MNT" >/dev/null 2>&1 || hdiutil detach -force "$MNT" >/dev/null 2>&1
  rm -f "$IMG"
  rmdir "$MNT" 2>/dev/null
}
trap cleanup EXIT

log "0) 建 32 MiB 磁盘镜像并挂载（真实文件系统，真实 statvfs）"
rm -f "$IMG"
mkdir -p "$MNT"
hdiutil create -size 32m -fs APFS -volname EMQA -quiet "$IMG" || { echo "镜像创建失败"; exit 1; }
hdiutil attach -quiet -mountpoint "$MNT" "$IMG" || { echo "挂载失败"; exit 1; }
df -h "$MNT" | sed 's/^/  /'

log "1) init + serve（data-dir 在镜像上）"
printf '%s\n' "$PASSWORD" >"$MNT/pw.txt"
chmod 600 "$MNT/pw.txt"
"$BIN" init --data-dir "$MNT/data" --password-file "$MNT/pw.txt" >"$MNT/init.log" 2>&1
echo "  init exit=$?（$MNT/data 已建立）"
"$BIN" serve --data-dir "$MNT/data" --listen "127.0.0.1:${PORT}" >"$MNT/serve.log" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 80); do curl -sf "$URL/api/v1/health/live" >/dev/null 2>&1 && break; sleep 0.1; done
chk "health/ready" 200 "$(curl -s -o /dev/null -w '%{http_code}' "$URL/api/v1/health/ready")"
req login -c "$MNT/cookies.txt" -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" "$URL/api/v1/auth/login" >/dev/null
CSRF="$(python3 -c "import json;print(json.load(open('$MNT/login.body'))['data']['csrfToken'])")"
ITEM="$(python3 -c 'import uuid;print(uuid.uuid4())')"
NOW="$(python3 -c 'import time;print(int(time.time()*1000))')"
sqlite3 "$MNT/data/manual.sqlite3" "INSERT INTO items (id,name,brand,model,variant,revision,archived_at,created_at,updated_at) VALUES ('$ITEM','磁盘满物品','QA','D-1',NULL,1,NULL,$NOW,$NOW);" && echo "  item=$ITEM"

log "2) 上传 40 MiB PDF（可用空间远小于它）→ 期望 413 + details.reason=insufficientStorage"
st="$(req big -b "$MNT/cookies.txt" -H "x-csrf-token: $CSRF" -F 'purpose=document' -F "file=@$ASSETS/big.pdf;type=application/pdf" "$URL/api/v1/items/$ITEM/assets")"
chk "HTTP 状态" 413 "$st"
chk "  error.code" "PAYLOAD_TOO_LARGE" "$(code big)"
chk "  details.reason" "insufficientStorage" "$(jdet big reason)"
echo "  message: $(python3 -c "import json;print(json.load(open('$MNT/big.body'))['error']['message'])" 2>/dev/null)"
chk "  tmp 无残留" 0 "$(find "$MNT/data/tmp" -type f | wc -l | tr -d ' ')"
chk "  blobs 无文件" 0 "$(find "$MNT/data/blobs" -type f | wc -l | tr -d ' ')"
chk "  无资产行" 0 "$(sqlite3 "$MNT/data/manual.sqlite3" 'SELECT COUNT(*) FROM assets;')"
chk "  无 blob 行" 0 "$(sqlite3 "$MNT/data/manual.sqlite3" 'SELECT COUNT(*) FROM blobs;')"

log "3) 小文件（可用空间内）仍可成功上传（证明不是把一切都拒掉）"
printf '第 1 页文字\n' >"$MNT/page1.txt"
st="$(req small -b "$MNT/cookies.txt" -H "x-csrf-token: $CSRF" -F 'purpose=pageText' -F "file=@$MNT/page1.txt;type=text/plain" "$URL/api/v1/items/$ITEM/assets")"
chk "pageText 201" 201 "$st"
chk "  blobs 文件数" 1 "$(find "$MNT/data/blobs" -type f | wc -l | tr -d ' ')"

log "4) 记录写入期间的服务端错误日志（磁盘相关）"
grep -o '"level":"\(WARN\|ERROR\)"[^}]*' "$MNT/serve.log" | head -5 | sed 's/^/  /'

echo
echo "===== 磁盘满（真实环境）小结：PASS=$PASS FAIL=$FAIL ====="
exit "$FAIL"
