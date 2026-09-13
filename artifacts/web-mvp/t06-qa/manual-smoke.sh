#!/usr/bin/env bash
# T06 QA 独立手工冒烟（回合 6）：真实 release 二进制 + 真实 data-dir + curl。
# 不采信 RD 结果；所有断言现场构造，负例现场恢复。
# 用法：bash manual-smoke.sh <binary> <assets-dir> <work-dir>
set -u
BIN="${1:?binary}"
ASSETS="${2:?assets}"
WORK="${3:?work}"
PORT=18097
URL="http://127.0.0.1:${PORT}"
PASSWORD='qa-t06-password-7c41'
PASS=0
FAIL=0
SERVER_PID=""

ok() { PASS=$((PASS + 1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL + 1)); echo "  FAIL  $1"; }
chk() { # chk <label> <expected> <actual>
  if [ "$2" = "$3" ]; then ok "$1 = $3"; else bad "$1: expected [$2] got [$3]"; fi
}
log() { echo; echo "== $*"; }

req() { # req <name> <curl-args...>  → 写入 $WORK/<name>.body|.hdr，回显 HTTP 码
  local name="$1"
  shift
  curl -s -o "$WORK/$name.body" -D "$WORK/$name.hdr" -w '%{http_code}' "$@"
}
hdr() { grep -i "^$2:" "$WORK/$1.hdr" | head -1 | tr -d '\r' | cut -d' ' -f2- | sed 's/^ //'; }
code() { python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['error']['code'])" "$WORK/$1.body" 2>/dev/null || echo "NO-JSON"; }
jfield() { python3 -c "import json,sys;d=json.load(open(sys.argv[1]))['data'];print(d[sys.argv[2]])" "$WORK/$1.body" "$2" 2>/dev/null || echo "NO-FIELD"; }
jdet() { python3 -c "import json,sys;d=json.load(open(sys.argv[1]))['error'].get('details') or {};print(d.get(sys.argv[2],'NO-KEY'))" "$WORK/$1.body" "$2" 2>/dev/null || echo "NO-DETAIL"; }
jq_stat() { python3 -c "import json,sys;print(json.loads(sys.argv[1])['status'])" "$1"; }
jq_hdr() { python3 -c "import json,sys;print(json.loads(sys.argv[1])['headers'].get(sys.argv[2],'-'))" "$1" "$2"; }
jq_len() { python3 -c "import json,sys;print(json.loads(sys.argv[1])['bodyLen'])" "$1"; }

start_server() { # start_server <logfile>
  "$BIN" serve --data-dir "$WORK/data" --listen "127.0.0.1:${PORT}" >>"$WORK/$1" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 80); do
    curl -sf "$URL/api/v1/health/live" >/dev/null 2>&1 && return 0
    sleep 0.1
  done
  echo "  服务未在 8s 内就绪（见 $WORK/$1）"
  return 1
}
stop_server() {
  [ -n "$SERVER_PID" ] || return 0
  kill "$SERVER_PID" 2>/dev/null
  wait "$SERVER_PID" 2>/dev/null
  SERVER_PID=""
}

new_item() { # new_item <name>
  local id
  id="$(python3 -c 'import uuid;print(uuid.uuid4())')"
  local now
  now="$(python3 -c 'import time;print(int(time.time()*1000))')"
  sqlite3 "$WORK/data/manual.sqlite3" \
    "INSERT INTO items (id,name,brand,model,variant,revision,archived_at,created_at,updated_at) VALUES ('$id','$1','QA','Q-1',NULL,1,NULL,$now,$now);"
  echo "$id"
}
sql1() { sqlite3 "$WORK/data/manual.sqlite3" "$1"; }
bytes() { if [ -f "$1" ]; then wc -c <"$1" | tr -d ' '; else echo 0; fi; }
count_blob_files() { find "$WORK/data/blobs" -type f 2>/dev/null | wc -l | tr -d ' '; }
count_tmp_files() { find "$WORK/data/tmp" -type f 2>/dev/null | wc -l | tr -d ' '; }
blob_list() { find "$WORK/data/blobs" -type f 2>/dev/null | sort | xargs -I{} shasum -a 256 {} 2>/dev/null; }

QADIR="$(cd "$(dirname "$0")" && pwd)"
blob_sha_png="$(shasum -a 256 "$ASSETS/photo.png" | cut -d' ' -f1)"
blob_sha_jpg="$(shasum -a 256 "$ASSETS/photo.jpg" | cut -d' ' -f1)"
blob_sha_pdf="$(shasum -a 256 "$ASSETS/manual.pdf" | cut -d' ' -f1)"
blob_sha_big="$(shasum -a 256 "$ASSETS/big.pdf" | cut -d' ' -f1)"
png_size="$(wc -c <"$ASSETS/photo.png" | tr -d ' ')"
echo "QA 手工冒烟  $(date '+%Y-%m-%d %H:%M:%S %Z')  binary=$BIN"
shasum -a 256 "$BIN"

log "0) init + serve"
printf '%s\n' "$PASSWORD" >"$WORK/pw.txt" && chmod 600 "$WORK/pw.txt"
"$BIN" init --data-dir "$WORK/data" --password-file "$WORK/pw.txt" >"$WORK/init.log" 2>&1
echo "  init exit=$?"
start_server serve1.log || exit 1
chk "health/ready" 200 "$(curl -s -o /dev/null -w '%{http_code}' "$URL/api/v1/health/ready")"

log "1) 登录（会话 + CSRF）"
login_code="$(req login -c "$WORK/cookies.txt" -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" "$URL/api/v1/auth/login")"
chk "login" 200 "$login_code"
CSRF="$(python3 -c "import json;print(json.load(open('$WORK/login.body'))['data']['csrfToken'])")"
SESSION_COOKIE="$(grep -i '^set-cookie: em_session=' "$WORK/login.hdr" | head -1 | sed -E 's/^set-cookie: (em_session=[^;]*).*/\1/I' | tr -d '\r')"
echo "  csrf=${CSRF:0:10}…  cookie=${SESSION_COOKIE:0:20}…"
qa() { # qa <method> <path> [Header: value ...] → 一行 JSON（body 落 $WORK/*.body.qa），另存 $WORK/$name.qa.json
  local method="$1" path="$2"
  shift 2
  ( cd "$WORK" && python3 "$QADIR/qa-http.py" "$method" "$URL$path" "$SESSION_COOKIE" "$@" )
}
ITEM_A="$(new_item 'QA 物品 A')"
ITEM_B="$(new_item 'QA 物品 B')"
echo "  itemA=$ITEM_A itemB=$ITEM_B"

up() { # up <name-url> <ITEM> <field-args...>
  local name="$1" item="$2"
  shift 2
  req "$name" -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF" "$@" "$URL/api/v1/items/$item/assets"
}
upn() { # 带额外 curl 参数的上传（用于无 CSRF 等）
  local name="$1" item="$2"
  shift 2
  req "$name" "$@" "$URL/api/v1/items/$item/assets"
}

log "2) AC-018 正常上传（PDF/JPEG/PNG）+ 文件名只作元数据"
st="$(up u-png "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/photo.png;type=image/png;filename=../../../qa-pwn.png")"
chk "PNG 上传（filename=../../../qa-pwn.png）" 201 "$st"
chk "  sha256" "$blob_sha_png" "$(jfield u-png sha256)"
chk "  size" "$png_size" "$(jfield u-png size)"
chk "  mime" "image/png" "$(jfield u-png mime)"
chk "  originalName（只取 basename）" "qa-pwn.png" "$(jfield u-png originalName)"
chk "  storageState" "stored" "$(jfield u-png storageState)"
chk "  purpose" "photo" "$(jfield u-png purpose)"
PNG_ASSET="$(jfield u-png id)"

st="$(up u-jpg "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/photo.jpg;type=image/jpeg")"
chk "JPEG 上传" 201 "$st"
chk "  mime" "image/jpeg" "$(jfield u-jpg mime)"

st="$(up u-pdf "$ITEM_A" -F 'purpose=document' -F "file=@$ASSETS/manual.pdf;type=application/pdf;filename=../../../../pwned.pdf")"
chk "PDF 上传（filename=../../../../pwned.pdf）" 201 "$st"
chk "  mime" "application/pdf" "$(jfield u-pdf mime)"
chk "  originalName" "pwned.pdf" "$(jfield u-pdf originalName)"
PDF_ASSET="$(jfield u-pdf id)"

log "2b) 负例：Content-Type/后缀不可信（magic 为准）"
st="$(up u-png-claimed-pdf "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/photo.png;type=application/pdf;filename=really-a.pdf")"
chk "真 PNG 谎报 application/pdf + .pdf 后缀（purpose=photo）" 201 "$st"
chk "  仍按内容识别" "image/png" "$(jfield u-png-claimed-pdf mime)"
st="$(up u-text-claimed-png "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/broken.pdf;type=image/png;filename=photo.png")"
chk "非图片谎报 image/png（purpose=photo）" 415 "$st"
chk "  code" "UNSUPPORTED_MEDIA_TYPE" "$(code u-text-claimed-png)"

log "2c) 路径穿越不产生越界写入"
found=0
for p in "$WORK/qa-pwn.png" "$WORK/pwned.pdf" "$WORK/data/qa-pwn.png" "$WORK/data/pwned.pdf" "$(dirname "$WORK")/qa-pwn.png" "$(dirname "$WORK")/pwned.pdf" "$WORK/data/blobs/qa-pwn.png"; do
  [ -e "$p" ] && { found=1; echo "    越界文件存在：$p"; }
done
chk "data-dir 内外均无被穿越创建的文件" 0 "$found"
chk "内容只落在内容寻址路径（blobs 文件数）" 3 "$(count_blob_files)"

log "3) AC-018 内容按 sha256 去重（同一 blob）"
st="$(up u-png-dup "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/photo.png;type=image/png")"
chk "重复上传同内容" 201 "$st"
chk "  同 sha256" "$blob_sha_png" "$(jfield u-png-dup sha256)"
if [ "$(jfield u-png-dup id)" != "$PNG_ASSET" ]; then ok "  asset id 不同（新记录）"; else bad "  asset id 与首次相同"; fi
chk "  blobs 行数" 3 "$(sql1 'SELECT COUNT(*) FROM blobs;')"
chk "  assets 行数" 5 "$(sql1 'SELECT COUNT(*) FROM assets;')"
chk "  blobs/ 文件数" 3 "$(count_blob_files)"
chk "  物品 A 累计体积（去重后）" "$((png_size + $(wc -c <"$ASSETS/photo.jpg" | tr -d ' ') + $(wc -c <"$ASSETS/manual.pdf" | tr -d ' ')))" "$(sql1 "SELECT SUM(size) FROM blobs WHERE sha256 IN (SELECT DISTINCT blob_id FROM assets WHERE item_id='$ITEM_A');")"
chk "  tmp 无残留" 0 "$(count_tmp_files)"

log "4) AC-018 GET / HEAD / Range / ETag / 条件请求"
etag_expected="\"$blob_sha_png\""
st="$(req get-png -b "$WORK/cookies.txt" "$URL/api/v1/assets/$PNG_ASSET/content")"
chk "GET 200" 200 "$st"
cmp -s "$WORK/get-png.body" "$ASSETS/photo.png" && ok "GET 字节与源文件一致" || bad "GET 字节与源文件不一致"
chk "  content-length" "$png_size" "$(hdr get-png content-length)"
chk "  etag" "$etag_expected" "$(hdr get-png etag)"
chk "  accept-ranges" "bytes" "$(hdr get-png accept-ranges)"
chk "  content-type" "image/png" "$(hdr get-png content-type)"
chk "  x-content-type-options" "nosniff" "$(hdr get-png x-content-type-options)"
[ -z "$(hdr get-png content-encoding)" ] && ok "无 content-encoding（不动态压缩）" || bad "出现 content-encoding"

head_json="$(qa HEAD "/api/v1/assets/$PNG_ASSET/content")"
chk "HEAD 200（独立 HTTP 客户端量 body）" 200 "$(jq_stat "$head_json")"
chk "  body 字节数" 0 "$(jq_len "$head_json")"
chk "  content-length 与 GET 相同" "$png_size" "$(jq_hdr "$head_json" content-length)"
chk "  etag 与 GET 相同" "$etag_expected" "$(jq_hdr "$head_json" etag)"

for spec in "0-9:0:9" "10-:10:$((png_size - 1))" "-5:$((png_size - 5)):$((png_size - 1))"; do
  header="bytes=${spec%%:*}"
  rest="${spec#*:}"
  start="${rest%%:*}"
  end="${rest##*:}"
  st="$(req "range-${start}-${end}" -b "$WORK/cookies.txt" -H "Range: $header" "$URL/api/v1/assets/$PNG_ASSET/content")"
  chk "Range $header → 206" 206 "$st"
  chk "  content-range" "bytes ${start}-${end}/${png_size}" "$(hdr "range-${start}-${end}" content-range)"
  chk "  content-length" "$((end - start + 1))" "$(hdr "range-${start}-${end}" content-length)"
  python3 - "$WORK/range-${start}-${end}.body" "$ASSETS/photo.png" "$start" "$end" <<'PY' && ok "  区间字节与源文件一致" || bad "  区间字节与源文件不一致"
import sys
body = open(sys.argv[1], "rb").read()
src = open(sys.argv[2], "rb").read()
start, end = int(sys.argv[3]), int(sys.argv[4])
sys.exit(0 if body == src[start:end + 1] else 1)
PY
done

st="$(req r416 -b "$WORK/cookies.txt" -H 'Range: bytes=99999999-' "$URL/api/v1/assets/$PNG_ASSET/content")"
chk "Range bytes=99999999- → 416" 416 "$st"
chk "  content-range" "bytes */$png_size" "$(hdr r416 content-range)"
chk "  错误体 code" "VALIDATION_FAILED" "$(code r416)"
st="$(req r416b -b "$WORK/cookies.txt" -H 'Range: bytes=-0' "$URL/api/v1/assets/$PNG_ASSET/content")"
chk "Range bytes=-0 → 416" 416 "$st"
chk "  content-range" "bytes */$png_size" "$(hdr r416b content-range)"

st="$(req multi -b "$WORK/cookies.txt" -H 'Range: bytes=0-1,4-5' "$URL/api/v1/assets/$PNG_ASSET/content")"
chk "多区间 → 完整 200（不拼接）" 200 "$st"
cmp -s "$WORK/multi.body" "$ASSETS/photo.png" && ok "  多区间回落为完整内容" || bad "  多区间内容不完整"

st="$(req inm -b "$WORK/cookies.txt" -H "If-None-Match: $etag_expected" "$URL/api/v1/assets/$PNG_ASSET/content")"
chk "If-None-Match 命中 → 304" 304 "$st"
chk "  304 body 为空" 0 "$(bytes "$WORK/inm.body")"
chk "  304 带 etag" "$etag_expected" "$(hdr inm etag)"
st="$(req inm-miss -b "$WORK/cookies.txt" -H 'If-None-Match: "other"' "$URL/api/v1/assets/$PNG_ASSET/content")"
chk "If-None-Match 不命中 → 200" 200 "$st"

st="$(req ifr-ok -b "$WORK/cookies.txt" -H 'Range: bytes=0-9' -H "If-Range: $etag_expected" "$URL/api/v1/assets/$PNG_ASSET/content")"
chk "If-Range 匹配 → 206" 206 "$st"
for pair in "ifr-stale:\"stale\"" "ifr-weak:W/$etag_expected" "ifr-date:Wed, 21 Oct 2015 07:28:00 GMT"; do
  name="${pair%%:*}"
  value="${pair#*:}"
  st="$(req "$name" -b "$WORK/cookies.txt" -H 'Range: bytes=0-9' -H "If-Range: $value" "$URL/api/v1/assets/$PNG_ASSET/content")"
  chk "If-Range [$value] → 完整 200" 200 "$st"
  chk "  完整长度" "$png_size" "$(bytes "$WORK/$name.body")"
done

head_range_json="$(qa HEAD "/api/v1/assets/$PNG_ASSET/content" 'Range: bytes=0-9')"
chk "HEAD + Range → 206" 206 "$(jq_stat "$head_range_json")"
chk "  body 为空" 0 "$(jq_len "$head_range_json")"
chk "  content-range" "bytes 0-9/$png_size" "$(jq_hdr "$head_range_json" content-range)"
head_missing_json="$(qa HEAD "/api/v1/assets/01993000-0000-7000-8000-00000000dead/content")"
chk "HEAD 未知资产 → 404（无 body）" 404 "$(jq_stat "$head_missing_json")"

log "5) AC-019 伪造类型 / purpose / 请求形态"
printf 'this is definitely not a pdf, just text' >"$WORK/notpdf.bin"
st="$(up n-fake-doc "$ITEM_A" -F 'purpose=document' -F "file=@$WORK/notpdf.bin;type=application/pdf;filename=x.pdf")"
chk "文本冒充 PDF（无 %PDF- 头）→ 415" 415 "$st"
chk "  code" "UNSUPPORTED_MEDIA_TYPE" "$(code n-fake-doc)"
st="$(up n-png-doc "$ITEM_A" -F 'purpose=document' -F "file=@$ASSETS/photo.png;type=application/pdf;filename=m.pdf")"
chk "PNG 冒充 PDF → 415" 415 "$st"
st="$(up n-text-photo "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/broken.pdf;type=image/jpeg;filename=p.jpg")"
chk "非图片冒充 JPEG → 415" 415 "$st"
st="$(up n-purpose-model "$ITEM_A" -F 'purpose=model' -F "file=@$ASSETS/manual.pdf;type=application/pdf")"
chk "purpose=model → 422" 422 "$st"
st="$(up n-purpose-snake "$ITEM_A" -F 'purpose=page_image' -F "file=@$ASSETS/manual.pdf;type=application/pdf")"
chk "purpose=page_image（非线上值）→ 422" 422 "$st"
st="$(up n-no-purpose "$ITEM_A" -F "file=@$ASSETS/manual.pdf;type=application/pdf")"
chk "缺少 purpose → 422" 422 "$st"
st="$(up n-no-file "$ITEM_A" -F 'purpose=document')"
chk "缺少 file → 422" 422 "$st"
st="$(up n-extra-field "$ITEM_A" -F 'purpose=document' -F 'extra=x' -F "file=@$ASSETS/manual.pdf;type=application/pdf")"
chk "未知表单字段 → 422" 422 "$st"
st="$(upn n-not-multipart "$ITEM_A" -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF" -H 'content-type: application/json' -d '{}')"
chk "非 multipart → 415" 415 "$st"
st="$(upn n-no-csrf "$ITEM_A" -b "$WORK/cookies.txt" -F 'purpose=document' -F "file=@$ASSETS/manual.pdf;type=application/pdf")"
chk "缺 CSRF → 403" 403 "$st"
st="$(req n-no-session "$URL/api/v1/assets/$PNG_ASSET/content")"
chk "无会话读内容 → 401" 401 "$st"
st="$(req n-unknown -b "$WORK/cookies.txt" "$URL/api/v1/assets/01993000-0000-7000-8000-00000000dead/content")"
chk "未知资产 → 404" 404 "$st"
st="$(up n-ghost-item 01993000-0000-7000-8000-00000000beef -F 'purpose=document' -F "file=@$ASSETS/manual.pdf;type=application/pdf")"
chk "不存在的物品上传 → 404" 404 "$st"

log "6) AC-019 超限 / 像素炸弹 / 解码失败"
st="$(up n-over-photo "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/over-photo.bin;type=image/png;filename=big.png")"
chk "21 MiB 照片（>20 MiB）→ 413" 413 "$st"
st="$(up n-over-photo-late "$ITEM_A" -F "file=@$ASSETS/over-photo.bin;type=image/png;filename=big.png" -F 'purpose=photo')"
chk "21 MiB 照片（purpose 在 file 之后到达）→ 413 兜底" 413 "$st"
st="$(up n-over-request "$ITEM_A" -F 'purpose=document' -F "file=@$ASSETS/over-request.bin;type=application/pdf;filename=big.pdf")"
chk "52 MiB 请求体（>51 MiB 路由上限）→ 413" 413 "$st"
st="$(up n-bomb "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/bomb.png;type=image/png")"
chk "像素炸弹 60000×60000 → 422" 422 "$st"
chk "  details.reason" "imagePixels" "$(jdet n-bomb reason)"
chk "  details.width" 60000 "$(jdet n-bomb width)"
st="$(up n-trunc "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/trunc.png;type=image/png")"
chk "截断 PNG → 422" 422 "$st"
st="$(up n-crc "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/crc.png;type=image/png")"
chk "CRC 损坏 PNG → 422" 422 "$st"
st="$(up n-noeoi "$ITEM_A" -F 'purpose=photo' -F "file=@$ASSETS/noeoi.jpg;type=image/jpeg")"
chk "缺 EOI 的 JPEG → 422" 422 "$st"
st="$(up n-broken-pdf "$ITEM_A" -F 'purpose=document' -F "file=@$ASSETS/broken.pdf;type=application/pdf")"
chk "结构不完整 PDF → 422" 422 "$st"
st="$(up n-bad-text "$ITEM_A" -F 'purpose=pageText' -F "file=@$ASSETS/photo.png;type=text/plain;filename=t.txt")"
chk "非 UTF-8 的 pageText → 422" 422 "$st"

log "7) 失败路径无半提交（行数/文件数/tmp 与失败前一致）"
chk "  blobs 行数" 3 "$(sql1 'SELECT COUNT(*) FROM blobs;')"
chk "  assets 行数" 5 "$(sql1 'SELECT COUNT(*) FROM assets;')"
chk "  blobs/ 文件数" 3 "$(count_blob_files)"
chk "  tmp 文件数" 0 "$(count_tmp_files)"

log "8) 响应/日志不泄露磁盘路径"
leak=0
for f in "$WORK"/*.body "$WORK"/*.hdr; do
  grep -qF "$WORK/data" "$f" && { leak=$((leak + 1)); echo "    路径出现在 $(basename "$f")"; }
  grep -qF "blobs/" "$f" && { leak=$((leak + 1)); echo "    blobs/ 出现在 $(basename "$f")"; }
done
chk "响应中无 data-dir 路径与 blobs/ 字样" 0 "$leak"

log "9) BUG-001 生产语义复核：默认 5 次/60s + Retry-After（HTTP 层）"
for i in 1 2 3 4 5; do
  body="{\"password\":\"wrong-password-$i\"}"
  st="$(req "bad$i" -H 'content-type: application/json' -d "$body" "$URL/api/v1/auth/login")"
  chk "第 $i 次错误密码 → 401" 401 "$st"
done
st="$(req bad6 -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" "$URL/api/v1/auth/login")"
chk "第 6 次（正确密码）→ 429（限速未变）" 429 "$st"
chk "  retry-after" 60 "$(hdr bad6 retry-after)"
chk "  code" "RATE_LIMITED" "$(code bad6)"

log "10) 40 MiB PDF：流式上传 + 服务进程内存观察（item B，限速 5MB/s 便于采样）"
rows_before_big="$(sql1 'SELECT COUNT(*) FROM blobs;')"
files_before_big="$(count_blob_files)"
before_rss="$(ps -o rss= -p "$SERVER_PID" | tr -d ' ')"
echo "  上传前 RSS=${before_rss}KB"
( curl -s --limit-rate 5m -o "$WORK/u-big.body" -w '%{http_code}' -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF" \
  -F 'purpose=document' -F "file=@$ASSETS/big.pdf;type=application/pdf" \
  "$URL/api/v1/items/$ITEM_B/assets" >"$WORK/u-big.code" ) &
curl_pid=$!
max_rss=$before_rss
samples=0
while kill -0 "$curl_pid" 2>/dev/null; do
  rss="$(ps -o rss= -p "$SERVER_PID" 2>/dev/null | tr -d ' ')"
  [ -n "$rss" ] && { samples=$((samples + 1)); [ "$rss" -gt "$max_rss" ] && max_rss="$rss"; }
  echo "$rss" >>"$WORK/rss-samples.txt"
  sleep 0.05
done
wait "$curl_pid"
after_rss="$(ps -o rss= -p "$SERVER_PID" | tr -d ' ')"
delta_kb=$((max_rss - before_rss))
echo "  采样 $samples 次；峰值 RSS=${max_rss}KB（Δ=$((delta_kb / 1024))MiB），上传后 RSS=${after_rss}KB"
chk "40 MiB PDF 上传" 201 "$(cat "$WORK/u-big.code")"
chk "  blob 文件大小" "$(bytes "$ASSETS/big.pdf")" "$(bytes "$WORK/data/blobs/${blob_sha_big:0:2}/$blob_sha_big")"
chk "  新增一行 blob" "$((rows_before_big + 1))" "$(sql1 'SELECT COUNT(*) FROM blobs;')"
chk "  新增一个文件" "$((files_before_big + 1))" "$(count_blob_files)"
[ "$delta_kb" -lt 8388608 ] && ok "  RSS 增量 ($((delta_kb / 1024))MiB, $samples 次采样) 远小于文件大小 (40MiB) → 未整文件读入内存" || bad "  RSS 增量过大：${delta_kb}KB"

log "11) 共享 blob 安全（跨物品去重）"
png_refs_before="$(sql1 "SELECT COUNT(*) FROM assets WHERE blob_id='$blob_sha_png';")"
st="$(up u-png-b "$ITEM_B" -F 'purpose=photo' -F "file=@$ASSETS/photo.png;type=image/png")"
chk "同内容上传到物品 B" 201 "$st"
chk "  仍只有一行 blob（同内容不加行）" "$((rows_before_big + 1))" "$(sql1 'SELECT COUNT(*) FROM blobs;')"
chk "  该 blob 引用计数 +1" "$((png_refs_before + 1))" "$(sql1 "SELECT COUNT(*) FROM assets WHERE blob_id='$blob_sha_png';")"
chk "  blobs/ 文件数不变" "$((files_before_big + 1))" "$(count_blob_files)"

log "12) 崩溃残留隔离 + missing↔stored 收敛（重启 serve）"
stop_server
blobs_baseline="$(blob_list)"
echo "half-written-upload" >"$WORK/data/tmp/crashed.part"
orphan="$(python3 -c 'print("f"*64)')"
mkdir -p "$WORK/data/blobs/${orphan:0:2}"
printf 'orphan-blob-bytes' >"$WORK/data/blobs/${orphan:0:2}/$orphan"
jpg_file="$WORK/data/blobs/${blob_sha_jpg:0:2}/$blob_sha_jpg"
mv "$jpg_file" "$WORK/jpg-blob-moved-aside"
start_server serve2.log || exit 1
scan_line="$(grep -o '"event":"asset_scan"[^}]*' "$WORK/serve2.log" | head -1)"
echo "  $scan_line"
scan_field() { echo "$scan_line" | grep -o "\"$1\":[0-9]*" | cut -d: -f2; }
chk "  asset_scan.tmpQuarantined（启动扫描真实接线）" 1 "$(scan_field tmpQuarantined)"
chk "  asset_scan.blobsQuarantined" 1 "$(scan_field blobsQuarantined)"
chk "  asset_scan.blobsKept（被引用的 3 个文件仍在）" 3 "$(scan_field blobsKept)"
chk "  asset_scan.blobsMarkedMissing（被移走的 jpg）" 1 "$(scan_field blobsMarkedMissing)"
chk "  tmp 残留被隔离到 quarantine" "yes" "$(ls "$WORK/data/quarantine" 2>/dev/null | grep -qx 'tmp-crashed.part' && echo yes || echo no)"
chk "  孤儿 blob 被隔离到 quarantine" "yes" "$(ls "$WORK/data/quarantine" 2>/dev/null | grep -qx "blob-$orphan" && echo yes || echo no)"
chk "  隔离内容逐字节保留" "orphan-blob-bytes" "$(cat "$WORK/data/quarantine/blob-$orphan" 2>/dev/null)"
chk "  被引用的 blob 未被移动" "yes" "$([ -f "$WORK/data/blobs/${blob_sha_png:0:2}/$blob_sha_png" ] && echo yes || echo no)"
chk "  blobs/ 根残留被隔离（无散文件）" "0" "$(ls -p "$WORK/data/blobs" | grep -v / | wc -l | tr -d ' ')"
JPG_ASSET="$(sql1 "SELECT id FROM assets WHERE blob_id='$blob_sha_jpg' LIMIT 1;")"
chk "  文件被移走 → storage_state" "missing" "$(sql1 "SELECT storage_state FROM blobs WHERE sha256='$blob_sha_jpg';")"
st="$(req n-missing -b "$WORK/cookies.txt" "$URL/api/v1/assets/$JPG_ASSET/content")"
chk "  missing 资产内容 → 404" 404 "$st"
stop_server
mv "$WORK/jpg-blob-moved-aside" "$jpg_file"
start_server serve3.log || exit 1
chk "  文件恢复 → storage_state" "stored" "$(sql1 "SELECT storage_state FROM blobs WHERE sha256='$blob_sha_jpg';")"
st="$(req ok-restored -b "$WORK/cookies.txt" "$URL/api/v1/assets/$JPG_ASSET/content")"
chk "  恢复后可读 → 200" 200 "$st"
chk "  二次扫描后隔离区文件数不变" 2 "$(ls "$WORK/data/quarantine" | wc -l | tr -d ' ')"
chk "  blobs 目录内容与操作前一致（3 次重启 + 隔离后）" "$blobs_baseline" "$(blob_list)"
stop_server

echo
echo "===== 小结：PASS=$PASS FAIL=$FAIL ====="
exit "$FAIL"
