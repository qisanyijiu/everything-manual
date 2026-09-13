#!/usr/bin/env bash
# T07 QA 独立冒烟（QA 自写，不复用 RD 脚本）：
# 用发布二进制 + 真实 data-dir + curl 独立复现 AC-016/017/020/021 与卡内裁定项。
# 结束时不留下进程与临时目录；只读仓库文件，不修改仓库内容。
set -uo pipefail

BINARY="${1:?usage: smoke-manual-qa.sh <absolute-binary-path>}"
BINARY="$(cd "$(dirname "$BINARY")" && pwd)/$(basename "$BINARY")"
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
WORK="$(mktemp -d /tmp/em-t07qa-XXXXXX)"
DATA="$WORK/data"
PWFILE="$WORK/password.txt"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
CPORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
BASE="http://127.0.0.1:$PORT/api/v1"
JAR="$WORK/jar.txt"
SERVER_PID=""
LISTENER_PID=""

PASS=0; FAIL=0; FAILED=()

cleanup() {
  [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null
  [[ -n "$LISTENER_PID" ]] && kill "$LISTENER_PID" 2>/dev/null
  wait 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

expect() { # name expected actual
  if [[ "$2" == "$3" ]]; then
    echo "PASS  $1  ($3)"; PASS=$((PASS+1))
  else
    echo "FAIL  $1  expected=$2 actual=$3"; FAIL=$((FAIL+1)); FAILED+=("$1")
  fi
}

jget() { # file dotted.path -> value（字符串不带引号，null 打印 null，数组/对象打印 JSON）
  python3 - "$1" "$2" <<'PY'
import json, sys
d = json.load(open(sys.argv[1], encoding="utf-8"))
cur = d
for part in sys.argv[2].split("."):
    cur = cur[int(part)] if part.isdigit() else cur[part]
if isinstance(cur, str):
    print(cur)
elif cur is None:
    print("null")
else:
    print(json.dumps(cur, ensure_ascii=False))
PY
}

# call METHOD PATH [BODYJSON] [extra curl args...]
# 写 body 到 $WORK/last.json、headers 到 $WORK/last.headers，回显状态码。
call() {
  local method="$1" path="$2" body="${3:-}"
  shift $(( $# < 3 ? $# : 3 ))
  local args=(-s -o "$WORK/last.json" -D "$WORK/last.headers" -w '%{http_code}' -X "$method" -b "$JAR" -c "$JAR")
  [[ -n "$body" ]] && args+=(-H 'Content-Type: application/json' --data-binary "@$body")
  [[ -n "${CSRF:-}" ]] && args+=(-H "X-CSRF-Token: $CSRF")
  args+=("$@")
  curl "${args[@]}" "$BASE$path"
}
last_header() { grep -i "^$1:" "$WORK/last.headers" | tail -1 | sed 's/^[^:]*: //' | tr -d '\r'; }
json_body() { printf '%s' "$1" > "$WORK/body.json"; }
snapshot() { cp "$WORK/last.json" "$WORK/$1"; }

CSRF=""
echo "== 环境 =="
echo "binary=$BINARY"; echo "port=$PORT cport=$CPORT work=$WORK"

# --- 0. init / serve -------------------------------------------------------
printf '%s' "qa-t07-password-8f2c" > "$PWFILE"; chmod 600 "$PWFILE"
"$BINARY" init --data-dir "$DATA" --password-file "$PWFILE" > "$WORK/init.log" 2>&1
expect "init exit 0" 0 $?
"$BINARY" check --data-dir "$DATA" > "$WORK/check-before.log" 2>&1
expect "check(运行前) exit 0" 0 $?
grep -q "v3" "$WORK/check-before.log" && echo "INFO  check 输出: $(tr '\n' ' ' < "$WORK/check-before.log")"

"$BINARY" serve --data-dir "$DATA" --listen "127.0.0.1:$PORT" > "$WORK/server.log" 2>&1 &
SERVER_PID=$!
code=""
for _ in $(seq 1 100); do
  code="$(curl -s -o "$WORK/health.json" -w '%{http_code}' "http://127.0.0.1:$PORT/health/live" || true)"
  [[ "$code" == "200" ]] && break
  sleep 0.1
done
expect "health/live 200" 200 "$code"
ready_code="$(curl -s -o "$WORK/ready.json" -w '%{http_code}' "http://127.0.0.1:$PORT/health/ready")"
expect "health/ready 200" 200 "$ready_code"
grep -qi "v3" "$WORK/ready.json" && { echo "WARN  ready 泄露 v3 字样"; FAIL=$((FAIL+1)); FAILED+=("ready-version-leak"); } || echo "PASS  ready 不含 v3 字样"

# --- 1. 登录 ---------------------------------------------------------------
json_body '{"password":"qa-t07-password-8f2c"}'
code="$(curl -s -o "$WORK/login.json" -w '%{http_code}' -c "$JAR" -H 'Content-Type: application/json' --data-binary "@$WORK/body.json" "$BASE/auth/login")"
expect "登录 200" 200 "$code"
CSRF="$(jget "$WORK/login.json" "data.csrfToken")"
[[ -n "$CSRF" && "$CSRF" != "null" ]] && { echo "PASS  拿到 csrfToken"; PASS=$((PASS+1)); } || { echo "FAIL  csrfToken 缺失"; FAIL=$((FAIL+1)); FAILED+=("csrf"); }

# --- 2. AC-016 创建 + 字段级 422 -------------------------------------------
json_body '{"name":"  相机 QA  ","brand":" 富士 ","model":" X100V ","variant":"银色"}'
code="$(call POST /items "$WORK/body.json")"
snapshot create.json
ETAG="$(last_header etag)"
expect "S1 创建 201" 201 "$code"
expect "S1 ETag \"r1\"" '"r1"' "$ETAG"
expect "S1 revision=1" 1 "$(jget "$WORK/create.json" data.revision)"
expect "S1 name 去空白" "相机 QA" "$(jget "$WORK/create.json" data.name)"
expect "S1 brand 去空白" "富士" "$(jget "$WORK/create.json" data.brand)"
ITEM="$(jget "$WORK/create.json" data.id)"
UUIDV="$(python3 -c 'import uuid,sys; print(uuid.UUID(sys.argv[1]).version)' "$ITEM")"
expect "S1 id 是 UUIDv7" 7 "$UUIDV"

json_body "{\"name\":\"   \",\"brand\":\"$(python3 -c 'print("B"*101)')\",\"model\":\"$(python3 -c 'print("M"*201)')\"}"
code="$(call POST /items "$WORK/body.json")"
snapshot invalid.json
expect "S2 空白/超长 422" 422 "$code"
expect "S2 错误码" VALIDATION_FAILED "$(jget "$WORK/invalid.json" error.code)"
FIELDS="$(python3 -c 'import json,sys; print(",".join(sorted(f["field"] for f in json.load(open(sys.argv[1]))["error"]["details"]["fields"])))' "$WORK/invalid.json")"
expect "S2 字段级 name,brand,model" "brand,model,name" "$FIELDS"

json_body '{}'
code="$(call POST /items "$WORK/body.json")"
snapshot empty-create.json
expect "S3 空对象 422" 422 "$code"
FIELDS="$(python3 -c 'import json,sys; print(",".join(sorted(f["field"] for f in json.load(open(sys.argv[1]))["error"]["details"]["fields"])))' "$WORK/empty-create.json")"
expect "S3 name+model 同报" "model,name" "$FIELDS"

code="$(call GET /items)"
snapshot list-after-fail.json
expect "S3 失败不落行（仍 1 条）" 1 "$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["data"]))' "$WORK/list-after-fail.json")"

# --- 3. AC-016 列表 {data,nextCursor} + 分页 20/100 ------------------------
code="$(call GET /items)"
snapshot list1.json
expect "S4 列表 200" 200 "$code"
expect "S4 nextCursor=null（1 条）" null "$(jget "$WORK/list1.json" nextCursor)"
python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert isinstance(d.get("data"), list) and "nextCursor" in d' "$WORK/list1.json" \
  && { echo "PASS  S4 顶层 {data,nextCursor}"; PASS=$((PASS+1)); } || { echo "FAIL  S4 顶层结构"; FAIL=$((FAIL+1)); FAILED+=("S4-结构"); }

for index in $(seq 1 20); do
  json_body "{\"name\":\"批量物品 $index\",\"model\":\"BULK-$index\"}"
  code="$(call POST /items "$WORK/body.json")"
  [[ "$code" == "201" ]] || { echo "FAIL  批量创建 $index ($code)"; FAIL=$((FAIL+1)); FAILED+=("bulk-$index"); break; }
done
code="$(call GET "/items")"
snapshot page1.json
expect "S4b 默认页 20 条（共 21）" 20 "$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["data"]))' "$WORK/page1.json")"
CURSOR="$(jget "$WORK/page1.json" nextCursor)"
python3 -c 'import sys; sys.exit(0 if sys.argv[1].startswith("v1:items:active:") else 1)' "$CURSOR" \
  && { echo "PASS  S4b 游标前缀 v1:items:active"; PASS=$((PASS+1)); } || { echo "FAIL  S4b 游标前缀: $CURSOR"; FAIL=$((FAIL+1)); FAILED+=("S4b-cursor"); }
ENC="$(python3 -c 'import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1], safe=""))' "$CURSOR")"
code="$(call GET "/items?cursor=$ENC")"
snapshot page2.json
expect "S4b 第二页 1 条" 1 "$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["data"]))' "$WORK/page2.json")"
expect "S4b 第二页 nextCursor=null" null "$(jget "$WORK/page2.json" nextCursor)"
DUP="$(python3 -c 'import json,sys
a=json.load(open(sys.argv[1]))["data"]; b=json.load(open(sys.argv[2]))["data"]
ids=[r["id"] for r in a+b]; print("ok" if len(set(ids))==21 else "dup:%d"%len(ids))' "$WORK/page1.json" "$WORK/page2.json")"
expect "S4b 两页 21 条不重不漏" ok "$DUP"

# --- 4. AC-016/017 PATCH 清空语义、If-Match、并发 ---------------------------
json_body '{"brand":null}'
code="$(call PATCH "/items/$ITEM" "$WORK/body.json" -H 'If-Match: "r1"')"
snapshot clear.json
expect "S5 {\"brand\":null} 200" 200 "$code"
expect "S5 brand=null（清空）" null "$(jget "$WORK/clear.json" data.brand)"
expect "S5 revision=2" 2 "$(jget "$WORK/clear.json" data.revision)"

json_body '{"variant":"限量版"}'
code="$(call PATCH "/items/$ITEM" "$WORK/body.json" -H 'If-Match: "r2"')"
snapshot keep.json
expect "S6 缺失字段=保持（brand 仍 null）" null "$(jget "$WORK/keep.json" data.brand)"
expect "S6 variant 已更新" "限量版" "$(jget "$WORK/keep.json" data.variant)"
expect "S6 revision=3" 3 "$(jget "$WORK/keep.json" data.revision)"

json_body '{}'
code="$(call PATCH "/items/$ITEM" "$WORK/body.json" -H 'If-Match: "r3"')"
snapshot empty-patch.json
expect "S7 空体 422" 422 "$code"
expect "S7 field=body" body "$(jget "$WORK/empty-patch.json" error.details.fields.0.field)"

json_body '{"name":null}'
code="$(call PATCH "/items/$ITEM" "$WORK/body.json" -H 'If-Match: "r3"')"
snapshot null-name.json
expect "S8 name:null 422" 422 "$code"
expect "S8 field=name" name "$(jget "$WORK/null-name.json" error.details.fields.0.field)"

json_body '{"archived":null}'
code="$(call PATCH "/items/$ITEM" "$WORK/body.json" -H 'If-Match: "r3"')"
snapshot null-archived.json
expect "S9 archived:null 422" 422 "$code"
expect "S9 field=archived" archived "$(jget "$WORK/null-archived.json" error.details.fields.0.field)"

json_body '{"name":"无所谓"}'
code="$(call PATCH "/items/$ITEM" "$WORK/body.json")"
expect "S10 缺 If-Match 428" 428 "$code"
code="$(call PATCH "/items/$ITEM" "$WORK/body.json" -H 'If-Match: "r1"')"
snapshot stale.json
expect "S10 过期 r1 412" 412 "$code"
expect "S10 details.currentRevision=3" 3 "$(jget "$WORK/stale.json" error.details.currentRevision)"
expect "S10 校验失败不递增（仍 3）" 3 "$(call GET "/items/$ITEM" >/dev/null; jget "$WORK/last.json" data.revision)"

json_body '{"name":"并发甲"}'; cp "$WORK/body.json" "$WORK/conc-a.json"
json_body '{"name":"并发乙"}'; cp "$WORK/body.json" "$WORK/conc-b.json"
curl -s -o "$WORK/conc-a.out" -w '%{http_code}\n' -X PATCH -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' -H 'If-Match: "r3"' --data-binary "@$WORK/conc-a.json" "$BASE/items/$ITEM" > "$WORK/conc-a.code" &
PA=$!
curl -s -o "$WORK/conc-b.out" -w '%{http_code}\n' -X PATCH -b "$JAR" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' -H 'If-Match: "r3"' --data-binary "@$WORK/conc-b.json" "$BASE/items/$ITEM" > "$WORK/conc-b.code" &
PB=$!
wait $PA $PB
expect "S11 并发恰一 200 一 412" "200 412 " "$(cat "$WORK/conc-a.code" "$WORK/conc-b.code" | sort | tr '\n' ' ')"
LOSER="$WORK/conc-a.out"; WINNER="$WORK/conc-b.out"
[[ "$(cat "$WORK/conc-a.code")" == "412" ]] || { LOSER="$WORK/conc-b.out"; WINNER="$WORK/conc-a.out"; }
expect "S11 loser currentRevision=4" 4 "$(jget "$LOSER" error.details.currentRevision)"
code="$(call GET "/items/$ITEM")"
snapshot final-item.json
expect "S11 后到者未覆盖（revision=4）" 4 "$(jget "$WORK/final-item.json" data.revision)"
expect "S11 胜者写入可见" "$(jget "$WINNER" data.name)" "$(jget "$WORK/final-item.json" data.name)"

# --- 5. AC-020 绑定 document + source_url 0 抓取 ----------------------------
PDF="$ROOT/tests/fixtures/assets/sample-manual-text.pdf"
PDF_SHA="$(shasum -a 256 "$PDF" | awk '{print $1}')"
code="$(call POST "/items/$ITEM/assets" "" -F 'purpose=document' -F "file=@$PDF;type=application/pdf;filename=manual.pdf")"
snapshot pdf-asset.json
expect "S14 上传 PDF 201" 201 "$code"
expect "S14 返回 sha256 一致" "$PDF_SHA" "$(jget "$WORK/pdf-asset.json" data.sha256)"
PDF_ASSET="$(jget "$WORK/pdf-asset.json" data.id)"

python3 - "$CPORT" "$WORK/count.log" <<'PY' > "$WORK/listener.out" 2>&1 &
import socket, sys, time
port, out = int(sys.argv[1]), sys.argv[2]
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", port)); s.listen(16); s.settimeout(0.25)
deadline = time.time() + 25
with open(out, "w") as f:
    f.write("ready\n"); f.flush()
    while time.time() < deadline:
        try:
            conn, addr = s.accept()
            f.write("connection from %s\n" % (addr,)); f.flush(); conn.close()
        except socket.timeout:
            continue
PY
LISTENER_PID=$!
sleep 0.4
json_body "{\"sourceAssetId\":\"$PDF_ASSET\",\"title\":\"  X100V 说明书  \",\"sourceUrl\":\"http://127.0.0.1:$CPORT/manual.pdf\"}"
code="$(call POST "/items/$ITEM/documents" "$WORK/body.json")"
snapshot document.json
expect "S15 绑定 201" 201 "$code"
expect "S15 sourceSha256=原件 sha" "$PDF_SHA" "$(jget "$WORK/document.json" data.sourceSha256)"
expect "S15 title 去空白" "X100V 说明书" "$(jget "$WORK/document.json" data.title)"
expect "S15 sourceUrl 原样" "http://127.0.0.1:$CPORT/manual.pdf" "$(jget "$WORK/document.json" data.sourceUrl)"
sleep 1.2
expect "S16 source_url 0 次外呼" 0 "$(grep -c '^connection' "$WORK/count.log" || true)"
kill "$LISTENER_PID" 2>/dev/null; LISTENER_PID=""

json_body '{"name":"另一个物品","model":"OTH-1"}'
code="$(call POST /items "$WORK/body.json")"
snapshot item2.json
ITEM2="$(jget "$WORK/item2.json" data.id)"
code="$(call POST "/items/$ITEM2/assets" "" -F 'purpose=document' -F "file=@$PDF;type=application/pdf;filename=manual.pdf")"
ASSET2="$(jget "$WORK/last.json" data.id)"
json_body "{\"sourceAssetId\":\"$ASSET2\",\"title\":\"t\"}"
code="$(call POST "/items/$ITEM/documents" "$WORK/body.json")"
snapshot cross-doc.json
expect "S17 跨物品资产 404" 404 "$code"
json_body '{"sourceAssetId":"01993000-0000-7000-8000-0000000000ff","title":"t"}'
code="$(call POST "/items/$ITEM/documents" "$WORK/body.json")"
snapshot ghost-doc.json
expect "S17 未知资产 404" 404 "$code"
expect "S17 两者响应逐字相同" "$(jget "$WORK/ghost-doc.json" error.message)" "$(jget "$WORK/cross-doc.json" error.message)"

# --- 6. AC-021 照片与视图 --------------------------------------------------
JPEG="$ROOT/tests/fixtures/assets/sample-photo-front.jpg"
PNG="$ROOT/tests/fixtures/assets/sample-photo-left.png"
call POST "/items/$ITEM/assets" "" -F 'purpose=photo' -F "file=@$JPEG;type=image/jpeg;filename=front.jpg" > /dev/null
P_JPG="$(jget "$WORK/last.json" data.id)"
call POST "/items/$ITEM/assets" "" -F 'purpose=photo' -F "file=@$PNG;type=image/png;filename=left.png" > /dev/null
P_PNG="$(jget "$WORK/last.json" data.id)"
json_body "{\"assetId\":\"$P_JPG\",\"view\":\"front\"}"
code="$(call POST "/items/$ITEM/photos" "$WORK/body.json")"
snapshot photo-front.json
expect "S18 front 201" 201 "$code"
expect "S18 ETag \"r1\"" '"r1"' "$(last_header etag)"
PHOTO="$(jget "$WORK/photo-front.json" data.id)"
json_body "{\"assetId\":\"$P_PNG\",\"view\":\"front\"}"
code="$(call POST "/items/$ITEM/photos" "$WORK/body.json")"
snapshot photo-second-front.json
expect "S18 同视图第二张 422" 422 "$code"
expect "S18 reason=viewOccupied" viewOccupied "$(jget "$WORK/photo-second-front.json" error.details.reason)"
expect "S18 existingPhotoId 指认" "$PHOTO" "$(jget "$WORK/photo-second-front.json" error.details.existingPhotoId)"
json_body "{\"assetId\":\"$P_PNG\",\"view\":\"detail\"}"
code="$(call POST "/items/$ITEM/photos" "$WORK/body.json")"
expect "S18 detail 201" 201 "$code"
code="$(call GET "/items/$ITEM/photos")"
snapshot photos.json
expect "S18 列表槽位序 front,detail" "front,detail" "$(python3 -c 'import json,sys; print(",".join(r["view"] for r in json.load(open(sys.argv[1]))["data"]))' "$WORK/photos.json")"
json_body "{\"assetId\":\"$P_PNG\",\"view\":\"top\"}"
code="$(call POST "/items/$ITEM/photos" "$WORK/body.json")"
expect "S18 非法视图 422" 422 "$code"
expect "S18 field=view" view "$(jget "$WORK/last.json" error.details.fields.0.field)"
json_body "{\"assetId\":\"$P_PNG\"}"
code="$(call POST "/items/$ITEM/photos" "$WORK/body.json")"
snapshot photo-no-view.json
expect "S18 缺 view 422" 422 "$code"
expect "S18 field=view（缺失/非法同形态）" view "$(jget "$WORK/photo-no-view.json" error.details.fields.0.field)"
json_body '{"view":"top"}'
code="$(call POST "/items/$ITEM/photos" "$WORK/body.json")"
snapshot photo-no-asset.json
expect "S18b 缺 assetId 422（记录形态）" 422 "$code"
echo "INFO  S18b 缺 assetId 响应：$(cat "$WORK/photo-no-asset.json")"

json_body '{"view":"left"}'
code="$(call PATCH "/items/$ITEM/photos/$PHOTO" "$WORK/body.json")"
expect "S19 PATCH 缺 If-Match 428" 428 "$code"
code="$(call PATCH "/items/$ITEM/photos/$PHOTO" "$WORK/body.json" -H 'If-Match: "r1"')"
snapshot photo-moved.json
expect "S19 改视图 200" 200 "$code"
expect "S19 revision=2" 2 "$(jget "$WORK/photo-moved.json" data.revision)"
code="$(call PATCH "/items/$ITEM/photos/$PHOTO" "$WORK/body.json" -H 'If-Match: "r1"')"
snapshot photo-stale.json
expect "S19 过期 412" 412 "$code"
expect "S19 currentRevision=2" 2 "$(jget "$WORK/photo-stale.json" error.details.currentRevision)"
json_body '{"view":"back"}'
code="$(call PATCH "/items/$ITEM2/photos/$PHOTO" "$WORK/body.json" -H 'If-Match: "r2"')"
snapshot photo-cross.json
expect "S20 跨物品 photoId 404" 404 "$code"

# --- 7. 归档语义 + 无删除 API ---------------------------------------------
json_body '{"archived":true}'
code="$(call PATCH "/items/$ITEM" "$WORK/body.json" -H 'If-Match: "r4"')"
snapshot archived.json
expect "S12 归档 200" 200 "$code"
expect "S12 revision=5" 5 "$(jget "$WORK/archived.json" data.revision)"
[[ "$(jget "$WORK/archived.json" data.archivedAt)" != "null" ]] && { echo "PASS  S12 archivedAt 就位"; PASS=$((PASS+1)); } || { echo "FAIL  S12 archivedAt"; FAIL=$((FAIL+1)); FAILED+=("S12-at"); }
code="$(call GET "/items?limit=100")"
snapshot active-list.json
expect "S12 默认列表不含归档" no "$(python3 -c 'import json,sys; ids=[r["id"] for r in json.load(open(sys.argv[1]))["data"]]; print("yes" if sys.argv[2] in ids else "no")' "$WORK/active-list.json" "$ITEM")"
code="$(call GET "/items?archived=true")"
snapshot archived-list.json
expect "S12 archived=true 只含归档" "1 True" "$(python3 -c 'import json,sys; ids=[r["id"] for r in json.load(open(sys.argv[1]))["data"]]; print(len(ids), sys.argv[2] in ids)' "$WORK/archived-list.json" "$ITEM")"
code="$(call GET "/items/$ITEM/documents")"
snapshot docs-after-archive.json
expect "S21 归档后 documents 可读" "$PDF_SHA" "$(jget "$WORK/docs-after-archive.json" data.0.sourceSha256)"
code="$(call GET "/items/$ITEM/photos")"
expect "S21 归档后 photos 可读" 200 "$code"
code="$(call GET "/assets/$PDF_ASSET/content" "")"
cp "$WORK/last.json" "$WORK/pdf-back.bin"
expect "S21 归档后资产内容 200" 200 "$code"
expect "S21 资产字节一致" "$PDF_SHA" "$(shasum -a 256 "$WORK/pdf-back.bin" | awk '{print $1}')"

code="$(call DELETE "/items/$ITEM")"
ALLOW="$(last_header allow)"
expect "S13 DELETE /items/{id} 405" 405 "$code"
[[ "$ALLOW" == *GET* && "$ALLOW" == *PATCH* ]] && { echo "PASS  S13 Allow 头 = $ALLOW"; PASS=$((PASS+1)); } || { echo "FAIL  S13 Allow 头缺方法: $ALLOW"; FAIL=$((FAIL+1)); FAILED+=("S13-allow"); }
for uri in "/items" "/items/$ITEM/documents" "/items/$ITEM/photos" "/items/$ITEM/photos/$PHOTO"; do
  code="$(call DELETE "$uri")"
  expect "S13 DELETE $uri 405" 405 "$code"
done
code="$(call DELETE "/items/$ITEM/permanent")"
snapshot del-unknown.json
expect "S13 未知删除路径 404" 404 "$code"
expect "S13 404 JSON 错误码" NOT_FOUND "$(jget "$WORK/del-unknown.json" error.code)"

# --- 7b. AC-017 前半段：同品牌型号不同配置并存互不覆盖 ---------------------
json_body '{"name":"同型号 标准版","brand":"富士","model":"X100V","variant":"标准版"}'
code="$(call POST /items "$WORK/body.json")"
snapshot twin-a.json
expect "S25 并存记录 A 201" 201 "$code"
TWIN_A="$(jget "$WORK/twin-a.json" data.id)"
json_body '{"name":"同型号 增强版","brand":"富士","model":"X100V","variant":"增强版"}'
code="$(call POST /items "$WORK/body.json")"
snapshot twin-b.json
expect "S25 并存记录 B 201（同品牌型号不唯一）" 201 "$code"
TWIN_B="$(jget "$WORK/twin-b.json" data.id)"
expect "S25 两条 id 不同" diff "$([[ "$TWIN_A" != "$TWIN_B" ]] && echo diff || echo same)"
json_body '{"variant":"限量版"}'
code="$(call PATCH "/items/$TWIN_A" "$WORK/body.json" -H 'If-Match: "r1"')"
expect "S25 改 A 200" 200 "$code"
code="$(call GET "/items/$TWIN_B")"
snapshot twin-b-after.json
expect "S25 B 未被覆盖（variant 不变）" "增强版" "$(jget "$WORK/twin-b-after.json" data.variant)"
expect "S25 B revision 仍为 1" 1 "$(jget "$WORK/twin-b-after.json" data.revision)"

# --- 8. 查询参数严格性 + 无磁盘路径 ----------------------------------------
for uri in "/items?limit=101" "/items?limit=0" "/items?cursor=bogus" "/items?archived=maybe" "/items?limit=1&limit=2"; do
  code="$(call GET "$uri")"
  expect "S22 $uri → 422" 422 "$code"
done

code="$(call GET "/items?archived=true&cursor=$ENC")"
expect "S22 跨过滤条件复用游标 422" 422 "$code"

LEAK="$(grep -l "$WORK" "$WORK"/*.json "$WORK"/*.bin 2>/dev/null | grep -v -e count.log -e jar.txt || true)"
expect "S23 响应无磁盘路径" "" "$LEAK"

# --- 8b. 新路由同样受会话保护（无 cookie → 401） ----------------------------
for uri in "/items/01993000-0000-7000-8000-000000000001/documents" "/items/01993000-0000-7000-8000-000000000001/photos" "/items/01993000-0000-7000-8000-000000000001/photos/01993000-0000-7000-8000-000000000002"; do
  code="$(curl -s -o "$WORK/last.json" -w '%{http_code}' "http://127.0.0.1:$PORT/api/v1$uri")"
  expect "S26 无会话 $uri → 401" 401 "$code"
done

# --- 9. 停服、check、schema v3、唯一索引兜底 -------------------------------
kill "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null; SERVER_PID=""
"$BINARY" check --data-dir "$DATA" > "$WORK/check-after.log" 2>&1
expect "S24 停服后 check exit 0" 0 $?
head -5 "$WORK/check-after.log"

DB="$DATA/manual.sqlite3"
expect "S24 迁移 3 条" 3 "$(sqlite3 "$DB" 'SELECT count(*) FROM _sqlx_migrations;')"
expect "S24 唯一索引存在" 1 "$(sqlite3 "$DB" "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='photos_item_view_unique';")"
EXISTING_VIEW="$(sqlite3 "$DB" "SELECT view FROM photos WHERE item_id='$ITEM' LIMIT 1;")"
EXISTING_ASSET="$(sqlite3 "$DB" "SELECT asset_id FROM photos WHERE item_id='$ITEM' LIMIT 1;")"
ERR="$(sqlite3 "$DB" "INSERT INTO photos (id,item_id,asset_id,view,revision,created_at,updated_at) VALUES ('qa-dup-0001','$ITEM','$EXISTING_ASSET','$EXISTING_VIEW',1,1,1);" 2>&1)"
case "$ERR" in *UNIQUE*) { echo "PASS  S24 索引兜底：重复 (item,view) 被 UNIQUE 拒绝"; PASS=$((PASS+1)); };;
  *) { echo "FAIL  S24 索引兜底未生效：$ERR"; FAIL=$((FAIL+1)); FAILED+=("S24-index"); };; esac

echo
echo "==== 汇总：PASS=$PASS FAIL=$FAIL ===="
[[ $FAIL -eq 0 ]] || printf '失败项：%s\n' "${FAILED[@]}"
exit $(( FAIL > 0 ? 1 : 0 ))
