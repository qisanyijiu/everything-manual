#!/bin/bash
# T06 手工冒烟：真实 data-dir + release 二进制 + curl。
# 覆盖：登录/CSRF → 建物品行（T07 前用 sqlite3 直接插入）→ 上传 PNG/PDF →
#       去重 → 200/206/416/304/HEAD/ETag/If-Range → 磁盘布局。
set -uo pipefail
BIN="${1:?用法: manual-smoke.sh <binary>}"
ROOT="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d /tmp/em-t06-smoke-XXXXXX)"
PORT=18123
FIXTURES=/Users/qsyj/Code/rust/everything-manual/tests/fixtures/assets
cp "$BIN" "$WORK/everything-manual"
cd "$WORK" || exit 1
printf 'smoke-password-9f3a2c\n' > pw.txt && chmod 600 pw.txt

echo "== 工作目录：$WORK"
echo "== init"
"$WORK/everything-manual" init --data-dir ./data --password-file ./pw.txt 2>&1 | tail -3
echo "== serve（127.0.0.1:${PORT}）"
"$WORK/everything-manual" serve --data-dir ./data --listen 127.0.0.1:${PORT} > serve.log 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 50); do
  curl -sf "http://127.0.0.1:${PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
echo "-- serve 启动日志（asset_scan 行）"
grep -o '"event":"asset_scan"[^}]*' serve.log | head -2

echo "== 登录"
curl -s -c cookies.txt -D login-headers.txt -H 'content-type: application/json' \
  -d '{"password":"smoke-password-9f3a2c"}' "http://127.0.0.1:${PORT}/api/v1/auth/login" -o login.json
CSRF=$(python3 -c "import json;print(json.load(open('login.json'))['data']['csrfToken'])")
echo "csrf=${CSRF:0:12}…"
grep -i "^set-cookie" login-headers.txt | sed 's/\(em_session=.\{12\}\).*/\1…（截断）/' 

echo "== 建物品行（T07 之前用 sqlite3 直接插入；服务以 WAL 打开，不冲突）"
ITEM=$(python3 -c "import uuid;print(uuid.uuid4())")
NOW=$(python3 -c "import time;print(int(time.time()*1000))")
sqlite3 ./data/manual.sqlite3 "INSERT INTO items (id,name,brand,model,variant,revision,archived_at,created_at,updated_at) VALUES ('$ITEM','冒烟物品','Smoke','S-1',NULL,1,NULL,$NOW,$NOW);" && echo "item=$ITEM"

echo "== 1) 上传 PNG（purpose=photo）"
curl -s -b cookies.txt -H "x-csrf-token: $CSRF" \
  -F 'purpose=photo' -F "file=@$FIXTURES/sample-photo-left.png;type=image/png" \
  "http://127.0.0.1:${PORT}/api/v1/items/$ITEM/assets" -o upload-png.json
python3 -c "import json;d=json.load(open('upload-png.json'))['data'];print('  201 asset=',d['id'],'sha256=',d['sha256'][:16],'size=',d['size'],'mime=',d['mime'],'state=',d['storageState']);open('png-asset.txt','w').write(d['id']);open('png-sha.txt','w').write(d['sha256'])"
PNG_ASSET=$(cat png-asset.txt); PNG_SHA=$(cat png-sha.txt)

echo "== 2) 上传 PDF（purpose=document）"
curl -s -b cookies.txt -H "x-csrf-token: $CSRF" \
  -F 'purpose=document' -F "file=@$FIXTURES/sample-manual-text.pdf;type=application/pdf" \
  "http://127.0.0.1:${PORT}/api/v1/items/$ITEM/assets" -o upload-pdf.json
python3 -c "import json;d=json.load(open('upload-pdf.json'))['data'];print('  201 asset=',d['id'],'sha256=',d['sha256'][:16],'size=',d['size'],'mime=',d['mime'])"

echo "== 3) 重复上传同一 PNG → 去重（同 sha256、不同 asset id）"
curl -s -b cookies.txt -H "x-csrf-token: $CSRF" \
  -F 'purpose=photo' -F "file=@$FIXTURES/sample-photo-left.png;type=image/png" \
  "http://127.0.0.1:${PORT}/api/v1/items/$ITEM/assets" -o upload-png-2.json
python3 -c "
import json
d=json.load(open('upload-png-2.json'))['data']
print('  asset=',d['id'],'sha256=',d['sha256'][:16],'（与原 asset 不同 id、同内容）')"

echo "== 4) 伪造类型（.pdf 扩展名 + 假 Content-Type）→ 415"
printf 'not a pdf' > fake.pdf
curl -s -o fake-resp.json -w "  HTTP %{http_code} " -b cookies.txt -H "x-csrf-token: $CSRF" \
  -F 'purpose=document' -F "file=@fake.pdf;type=application/pdf" \
  "http://127.0.0.1:${PORT}/api/v1/items/$ITEM/assets"
python3 -c "import json;print(json.load(open('fake-resp.json'))['error']['code'])"

echo "== 5) GET 完整内容（200 / ETag / Accept-Ranges / Content-Length）"
curl -s -b cookies.txt -D get-headers.txt -o body-full.bin "http://127.0.0.1:${PORT}/api/v1/assets/$PNG_ASSET/content"
grep -iE "^(HTTP/|etag|content-length|content-type|accept-ranges|cache-control)" get-headers.txt | sed 's/^/  /'
echo "  body bytes=$(wc -c < body-full.bin) / 源文件 $(wc -c < $FIXTURES/sample-photo-left.png)"

echo "== 6) HEAD（同头无 body）"
curl -s -I -b cookies.txt "http://127.0.0.1:${PORT}/api/v1/assets/$PNG_ASSET/content" | grep -iE "^(HTTP/|etag|content-length)" | sed 's/^/  /'

echo "== 7) Range: bytes=0-9 → 206 + Content-Range"
curl -s -b cookies.txt -H 'Range: bytes=0-9' -D range-headers.txt -o range.bin "http://127.0.0.1:${PORT}/api/v1/assets/$PNG_ASSET/content"
grep -iE "^(HTTP/|content-range|content-length|content-encoding)" range-headers.txt | sed 's/^/  /'
echo "  区间字节数=$(wc -c < range.bin)"

echo "== 8) Range 不可满足 bytes=999999- → 416"
curl -s -b cookies.txt -H 'Range: bytes=999999-' -D r416-headers.txt -o r416.json "http://127.0.0.1:${PORT}/api/v1/assets/$PNG_ASSET/content"
grep -iE "^(HTTP/|content-range)" r416-headers.txt | sed 's/^/  /'

echo "== 9) If-None-Match 命中 → 304"
ETAG=$(grep -i '^etag:' get-headers.txt | tr -d '\r' | cut -d' ' -f2)
curl -s -b cookies.txt -H "If-None-Match: $ETAG" -D inm-headers.txt -o /dev/null "http://127.0.0.1:${PORT}/api/v1/assets/$PNG_ASSET/content"
grep -iE "^(HTTP/|etag)" inm-headers.txt | sed 's/^/  /'

echo "== 10) If-Range 不匹配 + Range → 完整 200"
curl -s -b cookies.txt -H 'Range: bytes=0-9' -H 'If-Range: "stale"' -D ir-headers.txt -o ir.bin "http://127.0.0.1:${PORT}/api/v1/assets/$PNG_ASSET/content"
grep -iE "^(HTTP/|content-length)" ir-headers.txt | sed 's/^/  /'

echo "== 11) 跨物品/不存在资产 → 404；无会话 → 401"
curl -s -o na.json -w "  不存在资产 HTTP %{http_code}\n" -b cookies.txt "http://127.0.0.1:${PORT}/api/v1/assets/01993000-0000-7000-8000-00000000dead/content"
curl -s -o nu.json -w "  无会话 HTTP %{http_code}\n" "http://127.0.0.1:${PORT}/api/v1/assets/$PNG_ASSET/content"

echo "== 12) 磁盘布局与去重（blobs 目录 / DB 计数）"
find ./data/blobs -type f | sed 's/^/  /'
sqlite3 ./data/manual.sqlite3 "SELECT '  blobs=' || COUNT(*) FROM blobs; SELECT '  assets=' || COUNT(*) FROM assets; SELECT '  distinct blob 文件=' || COUNT(DISTINCT blob_id) FROM assets;"

echo "== 13) 崩溃残留隔离：手工放一个 tmp 残留，重启服务后观察 asset_scan"
echo "half upload" > ./data/tmp/crashed.part
kill $SERVER_PID; wait $SERVER_PID 2>/dev/null
"$WORK/everything-manual" serve --data-dir ./data --listen 127.0.0.1:${PORT} > serve2.log 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 50); do curl -sf "http://127.0.0.1:${PORT}/api/v1/health/live" >/dev/null && break; sleep 0.1; done
grep -o '"event":"asset_scan"[^}]*' serve2.log | head -1 | sed 's/^/  /'
ls ./data/quarantine 2>/dev/null | sed 's/^/  quarantine: /'
kill $SERVER_PID; wait $SERVER_PID 2>/dev/null

echo "== 全部输出保存在 ${WORK}（仅列出关键文件）"
ls "$WORK" | head -30
if [ -z "${KEEP_SMOKE_DIR:-}" ]; then
  rm -rf "$WORK"
  echo "（已清理 ${WORK}；需要保留现场时用 KEEP_SMOKE_DIR=1 重跑）"
fi
