#!/usr/bin/env bash
# T06 QA 手工冒烟（二段）：加密 PDF / 对象流 PDF / pageText 边界 / 空资产 Range / 元数据一致性。
# 复用 manual-smoke.sh 的 data-dir（$WORK），重新登录。
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
chk() { if [ "$2" = "$3" ]; then ok "$1 = $3"; else bad "$1: expected [$2] got [$3]"; fi; }
log() { echo; echo "== $*"; }
bytes() { if [ -f "$1" ]; then wc -c <"$1" | tr -d ' '; else echo 0; fi; }
sql1() { sqlite3 "$WORK/data/manual.sqlite3" "$1"; }
req() { local name="$1"; shift; curl -s -o "$WORK/$name.body" -D "$WORK/$name.hdr" -w '%{http_code}' "$@"; }
jfield() { python3 -c "import json,sys;d=json.load(open(sys.argv[1]))['data'];print(d[sys.argv[2]])" "$WORK/$1.body" "$2" 2>/dev/null || echo "NO-FIELD"; }
code() { python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['error']['code'])" "$WORK/$1.body" 2>/dev/null || echo "NO-JSON"; }

"$BIN" serve --data-dir "$WORK/data" --listen "127.0.0.1:${PORT}" >>"$WORK/serve4.log" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 80); do curl -sf "$URL/api/v1/health/live" >/dev/null 2>&1 && break; sleep 0.1; done
trap 'kill $SERVER_PID 2>/dev/null; wait $SERVER_PID 2>/dev/null' EXIT

echo "QA 手工冒烟（二段） $(date '+%Y-%m-%d %H:%M:%S %Z')"
req login -c "$WORK/cookies2.txt" -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" "$URL/api/v1/auth/login" >/dev/null
CSRF="$(python3 -c "import json;print(json.load(open('$WORK/login.body'))['data']['csrfToken'])")"
ITEM_A="$(sql1 "SELECT id FROM items WHERE name='QA 物品 A';")"
echo "  itemA=$ITEM_A"
up() { local name="$1"; shift; req "$name" -b "$WORK/cookies2.txt" -H "x-csrf-token: $CSRF" "$@" "$URL/api/v1/items/$ITEM_A/assets"; }

log "13) 加密 PDF 与对象流 PDF：上传接受（权威拒绝在 T09）"
python3 - "$ASSETS/manual.pdf" "$WORK/encrypted.pdf" "$WORK/objstm.pdf" <<'PY'
import sys
src = open(sys.argv[1], "rb").read()
# 加密：trailer 加 /Encrypt 9 0 R（REQ-012：拒绝在准备阶段）
enc = src.replace(b"trailer\n<< /Size", b"trailer\n<< /Encrypt 9 0 R /Size")
assert enc != src
open(sys.argv[2], "wb").write(enc)
# 对象流：/Root 指向的对象在纯对象区不可定位（真实场景是压在 /ObjStm 里）。
# 就地改写对象头（等长替换），保持 xref/startxref 偏移不变。
replaced = src.replace(b"1 0 obj\n<< /Type /Catalog", b"X 0 obj\n<< /Type /Catalog", 1)
assert replaced != src
open(sys.argv[3], "wb").write(replaced)
PY
st="$(up e-encrypted -F 'purpose=document' -F "file=@$WORK/encrypted.pdf;type=application/pdf")"
chk "加密 PDF（trailer /Encrypt）→ 201（REQ-012 拒绝在 T09）" 201 "$st"
st="$(up e-unparsed -F 'purpose=document' -F "file=@$WORK/objstm.pdf;type=application/pdf")"
chk "页树不可定位（模拟对象流）→ 201（页数权威判定在 T09）" 201 "$st"
grep -o '"stage":"pdf_probe"[^}]*' "$WORK/serve4.log" | head -2 | sed 's/^/  /'

log "14) pageText 边界：正常 / 超 2 MiB / 空文件 Range"
printf '第 1 页文字 QA\n' >"$WORK/page1.txt"
st="$(up t-ok -F 'purpose=pageText' -F "file=@$WORK/page1.txt;type=text/plain")"
chk "正常 pageText → 201" 201 "$st"
chk "  mime" "text/plain; charset=utf-8" "$(jfield t-ok mime)"
python3 -c "open('$WORK/toobig.txt','w').write('a'*(2*1024*1024+1))"
st="$(up t-toobig -F 'purpose=pageText' -F "file=@$WORK/toobig.txt;type=text/plain")"
chk "2 MiB+1 的 pageText → 413" 413 "$st"
chk "  code" "PAYLOAD_TOO_LARGE" "$(code t-toobig)"
: >"$WORK/empty.txt"
st="$(up t-empty -F 'purpose=pageText' -F "file=@$WORK/empty.txt;type=text/plain")"
chk "空 pageText → 201" 201 "$st"
chk "  size" 0 "$(jfield t-empty size)"
EMPTY_ASSET="$(jfield t-empty id)"
st="$(req empty-range -b "$WORK/cookies2.txt" -H 'Range: bytes=0-' "$URL/api/v1/assets/$EMPTY_ASSET/content")"
chk "空资产任何区间 → 416" 416 "$st"
chk "  content-range" "bytes */0" "$(grep -i '^content-range:' "$WORK/empty-range.hdr" | tr -d '\r' | cut -d' ' -f2-)"
st="$(req empty-get -b "$WORK/cookies2.txt" "$URL/api/v1/assets/$EMPTY_ASSET/content")"
chk "空资产完整 GET → 200" 200 "$st"
chk "  content-length" 0 "$(grep -i '^content-length:' "$WORK/empty-get.hdr" | tr -d '\r' | cut -d' ' -f2-)"

log "15) 元数据 ↔ 磁盘一致性：每条 asset 的 blob 文件存在且 sha256 相符"
python3 - "$WORK/data" <<'PY' && ok "全部 asset 都有对应文件且 sha256 相符（先文件后元数据的可观察结论）" || bad "存在元数据在而文件缺失/哈希不符"
import hashlib, os, sqlite3, sys
root = sys.argv[1]
conn = sqlite3.connect(os.path.join(root, "manual.sqlite3"))
rows = conn.execute(
    "SELECT a.id, a.blob_id, b.size FROM assets a JOIN blobs b ON b.sha256 = a.blob_id"
).fetchall()
bad = []
for asset_id, sha, size in rows:
    path = os.path.join(root, "blobs", sha[:2], sha)
    if not os.path.exists(path):
        bad.append((asset_id, sha, "missing"))
        continue
    if os.path.getsize(path) != size:
        bad.append((asset_id, sha, "size"))
        continue
    digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
    if digest != sha:
        bad.append((asset_id, sha, "sha"))
print(f"  assets={len(rows)} files_ok={len(rows) - len(bad)} bad={bad}")
sys.exit(1 if bad else 0)
PY
chk "  PRAGMA integrity_check" "ok" "$(sql1 'PRAGMA integrity_check;')"
chk "  tmp 目录为空" 0 "$(find "$WORK/data/tmp" -type f | wc -l | tr -d ' ')"

echo
echo "===== 二段小结：PASS=$PASS FAIL=$FAIL ====="
exit "$FAIL"
