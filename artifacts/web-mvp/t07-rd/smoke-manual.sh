#!/bin/bash
# T07 手工冒烟：真实 data-dir + 发布二进制 + curl。
# 覆盖：创建物品（201/UUIDv7/revision）→ 字段级 422 → 上传 PDF/照片 →
#       绑定 document（sourceUrl 0 外呼）→ 添加 front/detail 照片 →
#       同视图第二张被拒 → PATCH 清空 brand → 归档 → 列表/详情/引用核对 →
#       删除路由 405 → 过期 revision 412 → 并发/错误核对。
# 结束时清理临时目录（不自留现场）。
set -uo pipefail
BIN="${1:?用法: smoke-manual.sh <binary>}"
WORK="$(mktemp -d /tmp/em-t07-smoke-XXXXXX)"
PORT=18137
FIXTURES=/Users/qsyj/Code/rust/everything-manual/tests/fixtures/assets
cp "$BIN" "$WORK/everything-manual"
cd "$WORK" || exit 1
cleanup() {
  [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
  wait "$SERVER_PID" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT
printf 'smoke-password-t07-9f3a2c\n' > pw.txt && chmod 600 pw.txt

show() { python3 -c "
import json,sys
d=json.load(open(sys.argv[1]))
print(json.dumps(d, ensure_ascii=False)[:600])
" "$1"; }

echo "== 工作目录：${WORK}（结束时删除）"
echo "== init"
"$WORK/everything-manual" init --data-dir ./data --password-file ./pw.txt 2>&1 | tail -2
echo "== serve 127.0.0.1:${PORT}"
"$WORK/everything-manual" serve --data-dir ./data --listen 127.0.0.1:${PORT} > serve.log 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 50); do
  curl -sf "http://127.0.0.1:${PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
BASE="http://127.0.0.1:${PORT}/api/v1"

echo "== 登录"
curl -s -c cookies.txt -H 'content-type: application/json' \
  -d '{"password":"smoke-password-t07-9f3a2c"}' "$BASE/auth/login" -o login.json
CSRF=$(python3 -c "import json;print(json.load(open('login.json'))['data']['csrfToken'])")
echo "csrf=${CSRF:0:12}…（截断）"
AUTH=(-b cookies.txt -H "x-csrf-token: $CSRF")

echo "== 1) 创建物品（201 + UUIDv7 + revision=1 + ETag）"
curl -s -D create-headers.txt "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"  冒烟相机  ","brand":"SmokeBrand","model":"SK-100","variant":"标准版"}' \
  "$BASE/items" -o create.json
grep -i "^HTTP/\|^etag" create-headers.txt
ITEM=$(python3 -c "import json;d=json.load(open('create.json'))['data'];print(d['id'])")
python3 -c "
import json
d=json.load(open('create.json'))['data']
print('  201 item=',d['id'],'revision=',d['revision'],'name=',d['name'],'brand=',d['brand'])"
echo "  详情 GET："
curl -s "${AUTH[@]}" "$BASE/items/$ITEM" | show /dev/stdin

echo "== 2) 字段级 422（空白 name + 超长 model + 未知字段）"
curl -s -o invalid.json -w "  空白名称 → HTTP %{http_code}\n" "${AUTH[@]}" -H 'content-type: application/json' \
  -d '{"name":"   ","model":"M"}' "$BASE/items"
python3 -c "
import json
d=json.load(open('invalid.json'))['error']
print('  code=',d['code'],'fields=',[(f['field'],f['message']) for f in d['details']['fields']])"
LONG=$(python3 -c "print('M'*201)")
curl -s -o invalid2.json -w "  超长 model → HTTP %{http_code}\n" "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"name\":\"N\",\"model\":\"$LONG\"}" "$BASE/items"
show invalid2.json

echo "== 3) 上传资产（PDF=document、PNG/JPEG=photo）"
curl -s "${AUTH[@]}" -F 'purpose=document' \
  -F "file=@$FIXTURES/sample-manual-text.pdf;type=application/pdf" \
  "$BASE/items/$ITEM/assets" -o pdf.json
PDF_ASSET=$(python3 -c "import json;print(json.load(open('pdf.json'))['data']['id'])")
PDF_SHA=$(python3 -c "import json;print(json.load(open('pdf.json'))['data']['sha256'])")
echo "  PDF asset=$PDF_ASSET sha256=${PDF_SHA:0:16}…"
curl -s "${AUTH[@]}" -F 'purpose=photo' \
  -F "file=@$FIXTURES/sample-photo-front.jpg;type=image/jpeg" \
  "$BASE/items/$ITEM/assets" -o jpg.json
JPG_ASSET=$(python3 -c "import json;print(json.load(open('jpg.json'))['data']['id'])")
echo "  JPEG asset=$JPG_ASSET"
curl -s "${AUTH[@]}" -F 'purpose=photo' \
  -F "file=@$FIXTURES/sample-photo-left.png;type=image/png" \
  "$BASE/items/$ITEM/assets" -o png.json
PNG_ASSET=$(python3 -c "import json;print(json.load(open('png.json'))['data']['id'])")
echo "  PNG asset=$PNG_ASSET"

echo "== 4) 绑定说明书（sourceUrl 指向 discard 端口，仅记录不抓取）"
curl -s -o document.json -w "  HTTP %{http_code}\n" "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"sourceAssetId\":\"$PDF_ASSET\",\"title\":\"SK-100 说明书\",\"sourceUrl\":\"http://127.0.0.1:9/never-fetched.pdf\"}" \
  "$BASE/items/$ITEM/documents"
python3 -c "
import json
d=json.load(open('document.json'))['data']
print('  document=',d['id'],'sourceSha256=',d['sourceSha256'][:16],'… sourceUrl=',d['sourceUrl'])"

echo "== 5) 添加 front（PNG）与 detail（JPEG）照片"
curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$PNG_ASSET\",\"view\":\"front\"}" "$BASE/items/$ITEM/photos" -o photo-front.json
FRONT_ID=$(python3 -c "import json;print(json.load(open('photo-front.json'))['data']['id'])")
echo "  front photo=$FRONT_ID"
curl -s "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$JPG_ASSET\",\"view\":\"detail\"}" "$BASE/items/$ITEM/photos" -o photo-detail.json
echo "  detail photo=$(python3 -c "import json;print(json.load(open('photo-detail.json'))['data']['id'])")"

echo "== 6) 同视图第二张 → 422 viewOccupied（不静默覆盖）"
curl -s -o photo-dup.json -w "  HTTP %{http_code}\n" "${AUTH[@]}" -H 'content-type: application/json' \
  -d "{\"assetId\":\"$JPG_ASSET\",\"view\":\"front\"}" "$BASE/items/$ITEM/photos"
python3 -c "
import json
d=json.load(open('photo-dup.json'))['error']
print('  reason=',d['details']['reason'],'view=',d['details']['view'],'existingPhotoId=',d['details']['existingPhotoId'])"

echo "== 7) PATCH 清空 brand（显式 null = 清空，T04 遗留问题）"
curl -s -o patch-null.json -w "  HTTP %{http_code}\n" "${AUTH[@]}" -X PATCH -H 'content-type: application/json' \
  -H 'if-match: "r1"' -d '{"brand":null}' "$BASE/items/$ITEM"
python3 -c "
import json
d=json.load(open('patch-null.json'))['data']
print('  revision=',d['revision'],'brand=',d['brand'],'（应为 None）')"

echo "== 8) 必填字段 null → 422 且不改数据；缺 If-Match → 428；过期 → 412"
curl -s -o patch-bad.json -w "  name=null（带 r2）→ HTTP %{http_code}\n" "${AUTH[@]}" -X PATCH -H 'content-type: application/json' \
  -H 'if-match: "r2"' -d '{"name":null}' "$BASE/items/$ITEM"
show patch-bad.json
curl -s -o patch-428.json -w "  缺 If-Match → HTTP %{http_code}\n" "${AUTH[@]}" -X PATCH -H 'content-type: application/json' \
  -d '{"name":"x"}' "$BASE/items/$ITEM"
curl -s -o patch-412.json -w "  过期 r1 → HTTP %{http_code}\n" "${AUTH[@]}" -X PATCH -H 'content-type: application/json' \
  -H 'if-match: "r1"' -d '{"name":"x"}' "$BASE/items/$ITEM"
python3 -c "
import json
d=json.load(open('patch-412.json'))['error']
print('  code=',d['code'],'currentRevision=',d['details']['currentRevision'])"

echo "== 9) 归档（r2 → r3）"
curl -s -o archive.json -w "  HTTP %{http_code}\n" "${AUTH[@]}" -X PATCH -H 'content-type: application/json' \
  -H 'if-match: "r2"' -d '{"archived":true}' "$BASE/items/$ITEM"
python3 -c "
import json
d=json.load(open('archive.json'))['data']
print('  revision=',d['revision'],'archivedAt=',d['archivedAt'])"

echo "== 10) 归档后列表：默认不出现、archived=true 出现"
curl -s "${AUTH[@]}" "$BASE/items?limit=100" -o list-default.json
curl -s "${AUTH[@]}" "$BASE/items?limit=100&archived=true" -o list-archived.json
python3 -c "
import json
d=json.load(open('list-default.json'))
a=json.load(open('list-archived.json'))
print('  默认列表 ids=',[i['id'] for i in d['data']],'nextCursor=',d['nextCursor'])
print('  归档列表 ids=',[i['id'] for i in a['data']])"

echo "== 11) 归档后引用仍完整可读（document/photo/资产内容）"
curl -s -o details.json -w "  GET item → HTTP %{http_code}\n" "${AUTH[@]}" "$BASE/items/$ITEM"
curl -s -o documents.json -w "  GET documents → HTTP %{http_code}\n" "${AUTH[@]}" "$BASE/items/$ITEM/documents"
curl -s -o photos.json -w "  GET photos → HTTP %{http_code}\n" "${AUTH[@]}" "$BASE/items/$ITEM/photos"
curl -s -o content.bin -w "  GET asset content → HTTP %{http_code} bytes=%{size_download}\n" "${AUTH[@]}" "$BASE/assets/$PDF_ASSET/content"
python3 -c "
import json
docs=json.load(open('documents.json'))['data']
photos=json.load(open('photos.json'))['data']
print('  documents=',[(d['id'][:8],d['sourceSha256'][:8]) for d in docs])
print('  photos=',[(p['view'],p['id'][:8]) for p in photos])"

echo "== 12) 无永久删除（DELETE → 405）与未知删除路径（404）"
curl -s -o /dev/null -w "  DELETE /items/{id} → HTTP %{http_code}\n" "${AUTH[@]}" -X DELETE "$BASE/items/$ITEM"
curl -s -o /dev/null -w "  DELETE /items/{id}/permanent → HTTP %{http_code}\n" "${AUTH[@]}" -X DELETE "$BASE/items/$ITEM/permanent"

echo "== 13) 服务端日志（关键事件，无请求体/密钥）"
grep -o '"message":"[^"]*"' serve.log | sort | uniq -c | sort -rn | head -8

echo "== 冒烟结束（临时目录将在退出时删除）"
