#!/usr/bin/env bash
# 观察一次上传中 tmp → blobs 的原子落盘顺序（200ms 采样）。
# 期望：上传期间 tmp 恰好 1 个 .part 且在增长；此期间 assets/blobs 行数与 blobs/ 文件数不变；
#       完成后 tmp 归零、blobs/ 文件 +1、assets 行 +1（文件先于元数据、元数据可见即文件在）。
set -u
BIN="${1:?binary}"
ASSETS="${2:?assets}"
WORK=/tmp/em-t06-qa-order
PORT=18101
URL="http://127.0.0.1:${PORT}"
PASSWORD='qa-order-password-8a2f'
rm -rf "$WORK"; mkdir -p "$WORK"
printf '%s\n' "$PASSWORD" >"$WORK/pw.txt"; chmod 600 "$WORK/pw.txt"
"$BIN" init --data-dir "$WORK/data" --password-file "$WORK/pw.txt" >/dev/null 2>&1
"$BIN" serve --data-dir "$WORK/data" --listen "127.0.0.1:${PORT}" >"$WORK/serve.log" 2>&1 &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null; wait $SERVER_PID 2>/dev/null; rm -rf "$WORK"' EXIT
for _ in $(seq 1 80); do curl -sf "$URL/api/v1/health/live" >/dev/null 2>&1 && break; sleep 0.1; done
curl -s -c "$WORK/cookies.txt" -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" "$URL/api/v1/auth/login" -o "$WORK/login.json"
CSRF="$(python3 -c "import json;print(json.load(open('$WORK/login.json'))['data']['csrfToken'])")"
ITEM="$(python3 -c 'import uuid;print(uuid.uuid4())')"
NOW="$(python3 -c 'import time;print(int(time.time()*1000))')"
sqlite3 "$WORK/data/manual.sqlite3" "INSERT INTO items (id,name,brand,model,variant,revision,archived_at,created_at,updated_at) VALUES ('$ITEM','顺序物品','QA','O-1',NULL,1,NULL,$NOW,$NOW);"

q() { sqlite3 "$WORK/data/manual.sqlite3" "$1"; }
echo "t(ms)  tmp文件  tmp字节  blobs文件  assets行  blobs行"
( curl -s --limit-rate 4m -o "$WORK/upload.body" -w '%{http_code}' -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF" \
  -F 'purpose=document' -F "file=@$ASSETS/big.pdf;type=application/pdf" \
  "$URL/api/v1/items/$ITEM/assets" >"$WORK/upload.code" ) &
UP=$!
start=$(python3 -c 'import time;print(int(time.time()*1000))')
while kill -0 "$UP" 2>/dev/null; do
  now=$(python3 -c 'import time;print(int(time.time()*1000))')
  tmpn=$(find "$WORK/data/tmp" -type f 2>/dev/null | wc -l | tr -d ' ')
  tmpb=$(find "$WORK/data/tmp" -type f -exec stat -f%z {} \; 2>/dev/null | paste -sd+ - | bc 2>/dev/null || echo 0)
  blobsn=$(find "$WORK/data/blobs" -type f 2>/dev/null | wc -l | tr -d ' ')
  assetsn=$(q 'SELECT COUNT(*) FROM assets;')
  blobsr=$(q 'SELECT COUNT(*) FROM blobs;')
  echo "$((now-start))  $tmpn  ${tmpb:-0}  $blobsn  $assetsn  $blobsr"
  sleep 0.2
done
wait "$UP"
sleep 0.3
now=$(python3 -c 'import time;print(int(time.time()*1000))')
tmpn=$(find "$WORK/data/tmp" -type f 2>/dev/null | wc -l | tr -d ' ')
blobsn=$(find "$WORK/data/blobs" -type f 2>/dev/null | wc -l | tr -d ' ')
assetsn=$(q 'SELECT COUNT(*) FROM assets;')
blobsr=$(q 'SELECT COUNT(*) FROM blobs;')
echo "final($((now-start)))  $tmpn  0  $blobsn  $assetsn  $blobsr"
echo "HTTP=$(cat "$WORK/upload.code")"
