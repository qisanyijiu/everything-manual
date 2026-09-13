#!/bin/bash
# QA 回合 25 · T20 独立验收 · 阶段 3
# 目的：AC-010（恢复后同一 release/PDF/GLB 可读；会话未复活）与 AC-058
#       （导出自包含包、只含该 release 有权资产、无密钥/绝对路径/会话/临时云端 URL）。
set -u
BIN=/Users/qsyj/Code/rust/everything-manual/dist/aarch64-apple-darwin/everything-manual
QA=/Users/qsyj/Code/rust/everything-manual/artifacts/web-mvp/t20-qa
WORK=/tmp/em-t20-qa
RESTORED=$WORK/neg/t-control
PORT=18081
BASE=http://127.0.0.1:$PORT
PASSWORD='test-password-t20-backup'
ITEM=01a09677-6737-7665-a84b-335aaf2a4af8
RELEASE=01a09677-685a-7542-8e2f-0b866c8a1fbc
DOC_ASSET=01a09677-674f-70f2-bb1c-9538edcf75b5
MODEL_ASSET=01a09677-6839-73f4-af93-ec3acdea7db5
DOC_SHA=e18cf61a61c9f9834a73e7454c9f1c3a50e897908474f21edac4759fd3695cda
MODEL_SHA=a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb
MANIFEST_SHA=c2ec7def5b84a2bdad698be1bab9c14f0332255779d6a03c00a9c266b3d12465
NEW_ITEM=$(cat "$WORK/new-item-id.txt")
OUT=$WORK/exported
rm -rf "$OUT"; mkdir -p "$OUT"

step() { printf '\n=== [%s] %s\n' "$1" "$2"; }

step 0 "恢复目录与备份 manifest 的一致性（快照 sha256）"
MANIFEST_DB_SHA=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["database"]["sha256"])' "$WORK/my-backup/manifest.json")
ACTUAL=$(shasum -a 256 "$RESTORED/manual.sqlite3" | awk '{print $1}')
echo "manifest=$MANIFEST_DB_SHA"; echo "restored=$ACTUAL"
[ "$MANIFEST_DB_SHA" = "$ACTUAL" ] && echo "OK: 恢复出的库与备份快照逐字节一致" || echo "FAIL"

step 1 "备份来源（RD 样例备份）中参考字节的独立指纹（用于 4. 的比对）"
shasum -a 256 /Users/qsyj/Code/rust/everything-manual/artifacts/web-mvp/t20-rd/sample-backup/blobs/e1/$DOC_SHA
shasum -a 256 /Users/qsyj/Code/rust/everything-manual/artifacts/web-mvp/t20-rd/sample-backup/blobs/a9/$MODEL_SHA

step 2 "起服务（恢复目录）并登录（口令哈希保留 → 能登录）"
cd "$WORK" || exit 9
"$BIN" serve --data-dir "$RESTORED" --listen 127.0.0.1:$PORT > "$WORK/serve-restored.log" 2>&1 &
SERVE_PID=$!
for i in $(seq 1 50); do curl -s -o /dev/null "$BASE/api/v1/health/ready" && break; sleep 0.2; done
echo "serve pid=$SERVE_PID"
LOGIN=$(curl -s -c "$WORK/restored-cookies.txt" -H 'Content-Type: application/json' -H "Origin: $BASE" \
  -d "{\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login")
echo "login: $(echo "$LOGIN" | head -c 120)"
CSRF=$(echo "$LOGIN" | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["csrfToken"])')

step 3 "旧会话 cookie（备份前的会话）必须失效（备份不含会话）"
OLD_CODE=$(curl -s -o "$OUT/old-session-body.json" -w '%{http_code}' -b "$WORK/cookies.txt" "$BASE/api/v1/items")
echo "旧会话 GET /items -> HTTP ${OLD_CODE} (expect 401)"
cat "$OUT/old-session-body.json" | head -c 200; echo

step 4 "同一 release 可读（manifest 哈希 / PDF / GLB 字节）"
curl -s -b "$WORK/restored-cookies.txt" "$BASE/api/v1/items/$ITEM/releases/$RELEASE" > "$OUT/release.json"
python3 - "$OUT/release.json" "$MANIFEST_SHA" <<'PY'
import json,sys
d=json.load(open(sys.argv[1]))["data"]
print("releaseId:", d["id"])
print("manifestSha256 与发布时一致:", d["manifestSha256"]==sys.argv[2], d["manifestSha256"][:16])
PY
curl -s -b "$WORK/restored-cookies.txt" -o "$OUT/doc.pdf" -D "$OUT/doc.headers" "$BASE/api/v1/assets/$DOC_ASSET/content"
curl -s -b "$WORK/restored-cookies.txt" -o "$OUT/model.glb" -D "$OUT/model.headers" "$BASE/api/v1/assets/$MODEL_ASSET/content"
echo "PDF:  $(shasum -a 256 "$OUT/doc.pdf")  期望 $DOC_SHA"
echo "GLB:  $(shasum -a 256 "$OUT/model.glb")  期望 $MODEL_SHA"
echo "PDF 头: $(file -b "$OUT/doc.pdf")"
echo "GLB 头: $(python3 -c 'import struct;b=open("'"$OUT"'/model.glb","rb").read();print("magic",b[:4],"declared",struct.unpack("<I",b[8:12])[0],"actual",len(b))')"
echo "PDF content-type: $(grep -i '^content-type' "$OUT/doc.headers" | tr -d '\r')"

step 5 "恢复后的库里含 kill -9 前写入的 WAL-only 物品"
curl -s -b "$WORK/restored-cookies.txt" "$BASE/api/v1/items/$NEW_ITEM" > "$OUT/new-item.json"
python3 - "$OUT/new-item.json" <<'PY'
import json,sys
d=json.load(open(sys.argv[1]))["data"]
print("id:",d["id"],"| name:",d["name"],"| model:",d["model"])
PY

step 6 "导出（GET /api/v1/releases/{id}/export）"
curl -s -b "$WORK/restored-cookies.txt" -D "$OUT/export.headers" -o "$OUT/export.zip" "$BASE/api/v1/releases/$RELEASE/export"
grep -iE '^(HTTP|content-type|content-disposition|content-length)' "$OUT/export.headers" | tr -d '\r'
ls -l "$OUT/export.zip"

step 7 "ZIP 独立校验：系统 unzip 与 python zipfile"
unzip -t "$OUT/export.zip" | tail -3
python3 - "$OUT/export.zip" <<'PY'
import zipfile,sys
z=zipfile.ZipFile(sys.argv[1])
print("testzip:", z.testzip())
print("entries:")
for i in z.infolist(): print("  ", i.filename, i.file_size, i.date_time)
PY

step 8 "包内容断言（条目、哈希、冻结 manifest 字节原样、来源、相对清单）"
python3 - "$OUT/export.zip" "$DOC_SHA" "$MODEL_SHA" "$MANIFEST_SHA" <<'PY'
import hashlib,json,sys,zipfile
zip_path, doc_sha, model_sha, manifest_sha = sys.argv[1:5]
z=zipfile.ZipFile(zip_path)
names=z.namelist()
expect=["manifest.json","release/manifest.json",f"assets/model/{model_sha}.glb",f"assets/document/{doc_sha}.pdf"]
print("条目与期望一致:", names==expect, names)
frozen=z.read("release/manifest.json")
print("冻结 manifest sha256 一致:", hashlib.sha256(frozen).hexdigest()==manifest_sha)
m=json.loads(z.read("manifest.json"))
print("schemaVersion:", m["schemaVersion"])
print("release.releaseId:", m["release"]["releaseId"], "| item.model:", m["item"]["model"])
print("releaseManifest.sha256 一致:", m["releaseManifest"]["sha256"]==manifest_sha)
print("files[] 条数:", len(m["files"]))
for f in m["files"]:
    print("   ", f["role"], f["path"], "sha256 len", len(f["sha256"]), "size", f["size"], "source", f["source"])
    assert not f["path"].startswith("/") and ".." not in f["path"]
print("knowledge 为对象:", isinstance(m["knowledge"], dict))
print("notes:", json.dumps(m["notes"], ensure_ascii=False)[:200])
PY

step 9 "整包扫描：不得含会话 token / 密钥 / 绝对路径 / 临时云端 URL / 口令哈希 / 其它资产字节"
python3 - "$OUT/export.zip" "$WORK/session-token.txt" "$RESTORED/manual.sqlite3" "$WORK/my-backup/database/manual.sqlite3" "$WORK/data" "$DOC_SHA" "$MODEL_SHA" "$MANIFEST_SHA" <<'PY'
import hashlib,os,sqlite3,sys,zipfile
zip_path, token_file, restored_db, backup_db, data_dir, doc_sha, model_sha, manifest_sha = sys.argv[1:9]
blob=open(zip_path,'rb').read()
token=open(token_file).read().strip()
con=sqlite3.connect("file:%s?mode=ro"%restored_db, uri=True)
hash_value=con.execute("SELECT password_hash FROM admins").fetchone()[0]
con.close()
for name,needle in [("会话 token",token),("口令哈希",hash_value),("canary 假密钥","canary"),("fixture 临时云端 URL","cdn.example.invalid"),
                    ("fixture 本机 URL","127.0.0.1:60643"),("Bearer","Bearer "),("em_session cookie 名","em_session="),
                    ("data-dir 绝对路径",data_dir),("构建机路径 /Users/","/Users/"),("QA 工作路径 /tmp/em-t20-qa","/tmp/em-t20-qa")]:
    print(f"含 {name}: {needle.encode() in blob if isinstance(needle,str) else needle in blob}")
# 其它 blob 的字节不得出现在包里（只允许 model/document/manifest 三个 blob）
z=zipfile.ZipFile(zip_path)
allowed={doc_sha,model_sha,manifest_sha}
extra=0
for root,_,files in os.walk(os.path.join(data_dir,"blobs")):
    for f in files:
        if f in allowed: continue
        b=open(os.path.join(root,f),'rb').read()
        if len(b)>=40 and b in blob:
            print("  !! 未授权 blob 字节出现在包内:", f); extra+=1
print("未授权 blob 字节命中数:", extra)
PY

step 10 "未登录 401 / 未知 release 404"
echo "无 cookie: HTTP $(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/v1/releases/$RELEASE/export")"
echo "未知 release: HTTP $(curl -s -o /dev/null -w '%{http_code}' -b "$WORK/restored-cookies.txt" "$BASE/api/v1/releases/01930000-0000-7000-8000-000000000000/export")"

step 11 "导出后 tmp/ 无残留"
ls -A "$RESTORED/tmp" | head -5; echo "tmp entries=$(ls -A "$RESTORED/tmp" | wc -l)"

step 12 "停服"
kill -TERM "$SERVE_PID"; sleep 1
if kill -0 "$SERVE_PID" 2>/dev/null; then echo "still running"; kill -9 "$SERVE_PID"; else echo "OK: serve 已停止"; fi
tail -5 "$WORK/serve-restored.log"
echo
echo "phase3 完成"
