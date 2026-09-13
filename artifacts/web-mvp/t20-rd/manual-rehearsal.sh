#!/usr/bin/env bash
# T20 手工演练（RD 交付要求）：建数据 → 备份 → 新空目录恢复 → 启动并读取同一 release
# （PDF 与 GLB 均打开）→ 导出自包含包并用标准工具校验。
#
# 前置：先跑造数用例（把已发布版本的 data-dir 留在 $WORK/data）：
#   EM_T20_REHEARSAL_DIR=/tmp/em-t20-rehearsal \
#     cargo test -p everything-manual --test backup_restore -- --ignored --nocapture prepare_rehearsal_datadir
#
# 用法（仓库根）：bash artifacts/web-mvp/t20-rd/manual-rehearsal.sh
# 全部原始输出由调用方 tee 到 artifacts/web-mvp/t20-rd/manual-rehearsal-output.log。
set -u

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
BIN="$REPO_ROOT/dist/aarch64-apple-darwin/everything-manual"
WORK="${EM_T20_REHEARSAL_DIR:-/tmp/em-t20-rehearsal}"
DATA="$WORK/data"
BACKUP="$WORK/backup-out"
RESTORED="$WORK/restored-data"
PORT=18099
FAILED=0

result() { # result <名称> <退出码>
  if [ "$2" -eq 0 ]; then echo "[通过] $1"; else echo "[失败] $1（退出码 $2）"; FAILED=1; fi
}

expect_exit() { # expect_exit <期望码> <名称> -- <命令...>
  local expected="$1"; shift
  local name="$1"; shift
  shift # --
  "$@"
  local code=$?
  if [ "$code" -eq "$expected" ]; then
    echo "[通过] ${name}（退出码 $code = 期望值）"
  else
    echo "[失败] ${name}（退出码 ${code}，期望 ${expected}）"; FAILED=1
  fi
}

start_serve() { # start_serve <data-dir> <日志文件>
  # `exec` 让子 shell 被二进制替换：$! 就是 serve 进程本身的 pid（否则杀掉的是
  # 包装子 shell，serve 会变孤儿继续持锁——实测踩过）。
  ( cd "$WORK" && exec "$BIN" serve --data-dir "$1" --listen "127.0.0.1:$PORT" > "$2" 2>&1 ) &
  echo $! > "$WORK/serve.pid"
  for _ in $(seq 1 100); do
    grep -q "listening on http" "$2" 2>/dev/null && return 0
    sleep 0.1
  done
  echo "serve 未在 10 秒内就绪："; cat "$2"; return 1
}

stop_serve() {
  if [ -f "$WORK/serve.pid" ]; then
    pid=$(cat "$WORK/serve.pid")
    kill -TERM "$pid" 2>/dev/null
    # 等待进程真正退出（锁由内核在退出时释放；不等会与后续 backup 抢锁）。
    for _ in $(seq 1 150); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.1
    done
    if kill -0 "$pid" 2>/dev/null; then
      echo "[失败] serve（pid ${pid}）未在 15 秒内退出"; FAILED=1
    else
      echo "serve 已停止（SIGTERM；pid ${pid} 已退出，排他锁已释放）"
    fi
    rm -f "$WORK/serve.pid"
  fi
}

echo "== 0. 演练前置 =="
echo "二进制：$BIN"
echo "二进制 sha256：$(shasum -a 256 "$BIN" | awk '{print $1}')"
echo "演练目录：$WORK"
# 上一次演练可能留下未清理的 serve（重复运行时先收尾，避免排他锁干扰）。
if [ -f "$WORK/serve.pid" ]; then
  STALE=$(cat "$WORK/serve.pid")
  kill -TERM "$STALE" 2>/dev/null && echo "清理上次演练遗留的 serve（pid ${STALE}）"
  sleep 0.5
  rm -f "$WORK/serve.pid"
fi
test -x "$BIN" || { echo "缺少 dist 二进制，请先 cargo xtask dist"; exit 1; }
test -f "$DATA/manual.sqlite3" || { echo "缺少演练数据；请先运行 prepare_rehearsal_datadir"; exit 1; }
INFO="$WORK/rehearsal-info.json"
ITEM=$(python3 -c "import json;print(json.load(open('$INFO'))['itemId'])")
RELEASE=$(python3 -c "import json;print(json.load(open('$INFO'))['releaseId'])")
MODEL_ASSET=$(python3 -c "import json;print(json.load(open('$INFO'))['modelAssetId'])")
MODEL_SHA=$(python3 -c "import json;print(json.load(open('$INFO'))['modelSha256'])")
DOC_ASSET=$(python3 -c "import json;print(json.load(open('$INFO'))['documentAssetId'])")
DOC_SHA=$(python3 -c "import json;print(json.load(open('$INFO'))['documentSha256'])")
MANIFEST_SHA=$(python3 -c "import json;print(json.load(open('$INFO'))['manifestSha256'])")
PASSWORD=$(python3 -c "import json;print(json.load(open('$INFO'))['password'])")
echo "item=$ITEM release=$RELEASE"

echo
echo "== 1. 服务运行中请求 backup（必须拒绝并要求先停服）=="
rm -rf "$BACKUP" "$RESTORED"
start_serve "$DATA" "$WORK/serve-source.log" || exit 1
expect_exit 5 "运行中 backup 被拒绝（exit 5，要求停服）" -- \
  "$BIN" backup --data-dir "$DATA" --out "$BACKUP"
echo "--- backup stderr（运行中）---"
"$BIN" backup --data-dir "$DATA" --out "$BACKUP" 2>&1 | tail -3
test ! -e "$BACKUP" && echo "[通过] 被拒时未创建任何备份目录" || { echo "[失败] 被拒却创建了目录"; FAILED=1; }

echo
echo "== 2. 停服后备份（一致快照 + 被引用 blob + manifest + sha256）=="
stop_serve
expect_exit 0 "停服 backup 成功" -- "$BIN" backup --data-dir "$DATA" --out "$BACKUP"
echo "--- 备份产物 ---"
find "$BACKUP" -type f | sort | sed "s|$BACKUP/||"
echo "--- manifest.json ---"
cat "$BACKUP/manifest.json"
echo "--- 快照校验（sha256 与 manifest 一致；无 WAL 边车）---"
SNAP_SHA=$(shasum -a 256 "$BACKUP/database/manual.sqlite3" | awk '{print $1}')
MANIFEST_DB_SHA=$(python3 -c "import json;print(json.load(open('$BACKUP/manifest.json'))['database']['sha256'])")
[ "$SNAP_SHA" = "$MANIFEST_DB_SHA" ] && echo "[通过] 快照 sha256 = manifest：$SNAP_SHA" || { echo "[失败] 快照 sha256 不符"; FAILED=1; }
echo "--- SHA256SUMS 全量校验（标准工具 shasum -c）---"
( cd "$BACKUP" && shasum -a 256 -c SHA256SUMS ) || { echo "[失败] SHA256SUMS 校验失败"; FAILED=1; }
ls "$BACKUP/database/" | grep -q -- "-wal\|-shm" && { echo "[失败] 快照带 WAL 边车"; FAILED=1; } || echo "[通过] 快照是单文件（无 -wal/-shm）"
echo "--- 备份内不含会话（打开快照查询 sessions）---"
python3 - "$BACKUP/database/manual.sqlite3" <<'PY'
import sqlite3, sys
conn = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
n = conn.execute("SELECT COUNT(*) FROM sessions").fetchone()[0]
print(f"sessions 行数 = {n}")
assert n == 0, "备份快照不得包含会话"
print("[通过] 快照会话为空（恢复后必须重新登录）")
PY
[ $? -eq 0 ] || FAILED=1

echo
echo "== 3. 恢复到新空目录 =="
expect_exit 0 "restore 到不存在的新目录" -- "$BIN" restore --from "$BACKUP" --data-dir "$RESTORED"
echo "--- 非空目标拒绝（exit 4）---"
mkdir -p "$WORK/nonempty" && echo keep > "$WORK/nonempty/keep.txt"
expect_exit 4 "非空目标被拒绝（exit 4）" -- "$BIN" restore --from "$BACKUP" --data-dir "$WORK/nonempty"
[ "$(cat "$WORK/nonempty/keep.txt")" = "keep" ] && echo "[通过] 现场未被改动"

echo
echo "== 4. 启动恢复目录并读取同一 release（PDF 与 GLB 均可打开）=="
start_serve "$RESTORED" "$WORK/serve-restored.log" || { FAILED=1; }
PORT_LINE=$(grep -o "http://127.0.0.1:[0-9]*" "$WORK/serve-restored.log" | head -1)
BASE="$PORT_LINE/api/v1"
JAR="$WORK/cookies.txt"
echo "登录（管理员口令随备份恢复）：$BASE/auth/login"
curl -s -c "$JAR" -X POST -H 'content-type: application/json' \
  -d "{\"password\":\"$PASSWORD\"}" "$BASE/auth/login" | head -c 200; echo
curl -s -b "$JAR" "$BASE/items/$ITEM/releases/$RELEASE" -o "$WORK/release-detail.json" -w "release detail HTTP %{http_code}\n"
python3 - "$WORK/release-detail.json" "$MANIFEST_SHA" <<'PY'
import json, sys
body = json.load(open(sys.argv[1]))["data"]
assert body["manifestSha256"] == sys.argv[2], f'manifest 哈希不符 {body["manifestSha256"]} != {sys.argv[2]}'
assert len(body["manifest"]["knowledge"]["knowledge"]["parts"]) > 0
print(f"[通过] 恢复后 release manifest 哈希一致（{sys.argv[2][:16]}…），知识含部件 {len(body["manifest"]["knowledge"]["knowledge"]["parts"])} 个")
PY
[ $? -eq 0 ] || FAILED=1

curl -s -b "$JAR" "$BASE/assets/$DOC_ASSET/content" -o "$WORK/recovered-manual.pdf" -w "PDF content HTTP %{http_code}\n"
curl -s -b "$JAR" "$BASE/assets/$MODEL_ASSET/content" -o "$WORK/recovered-model.glb" -w "GLB content HTTP %{http_code}\n"
echo "--- PDF ---"
file "$WORK/recovered-manual.pdf"
echo "sha256: $(shasum -a 256 "$WORK/recovered-manual.pdf" | awk '{print $1}')（期望 ${DOC_SHA}）"
[ "$(shasum -a 256 "$WORK/recovered-manual.pdf" | awk '{print $1}')" = "$DOC_SHA" ] && echo "[通过] PDF 字节与发布时一致" || { echo "[失败] PDF 字节不符"; FAILED=1; }
echo "--- GLB ---"
python3 - "$WORK/recovered-model.glb" "$MODEL_SHA" <<'PY'
import hashlib, struct, sys
data = open(sys.argv[1], "rb").read()
assert data[0:4] == b"glTF", "GLB magic 不符"
version, length = struct.unpack("<II", data[4:12])
assert version == 2, version
assert length == len(data), (length, len(data))
digest = hashlib.sha256(data).hexdigest()
assert digest == sys.argv[2], (digest, sys.argv[2])
print(f"[通过] GLB 可打开：glTF v{version}，声明长度 {length} 字节 = 文件长度，sha256 与发布时一致")
PY
[ $? -eq 0 ] || FAILED=1

echo
echo "== 5. 导出自包含包（GET /releases/{id}/export）+ 标准工具校验 =="
curl -s -b "$JAR" -D "$WORK/export-headers.txt" \
  "$BASE/releases/$RELEASE/export" -o "$WORK/export.zip" -w "export HTTP %{http_code}\n"
grep -i "content-type\|content-disposition" "$WORK/export-headers.txt"
echo "--- unzip -t（系统解压工具全量 CRC 校验）---"
unzip -t "$WORK/export.zip" || FAILED=1
echo "--- unzip -l ---"
unzip -l "$WORK/export.zip"
echo "--- python3 -m zipfile -t ---"
python3 -m zipfile -t "$WORK/export.zip" && echo "[通过] python zipfile 校验通过" || FAILED=1
echo "--- 包内清单（manifest.json 节选）---"
python3 - "$WORK/export.zip" <<'PY'
import json, zipfile
with zipfile.ZipFile(__import__("sys").argv[1]) as z:
    manifest = json.loads(z.read("manifest.json"))
    print(json.dumps({k: manifest[k] for k in ("schemaVersion", "item", "release", "files", "notes")}, ensure_ascii=False, indent=2)[:2200])
PY
echo "--- 敏感内容扫描（canary 密钥 / 会话 token / 绝对路径 / 临时云端 URL）---"
python3 - "$WORK/export.zip" "$WORK" <<'PY'
import sys, zipfile
raw = open(sys.argv[1], "rb").read()
text = raw.decode("utf-8", "ignore")
forbidden = ["canary-t20-not-a-real-key", "cdn.example.invalid", "openapi.tripo3d.ai", sys.argv[2], "/Users/", "em_session="]
for item in forbidden:
    assert item not in text, f"导出包出现禁含内容：{item}"
    print(f"[通过] 不含 {item!r}")
PY
[ $? -eq 0 ] || FAILED=1

echo
echo "== 6. 收尾 =="
stop_serve
if [ "$FAILED" -eq 0 ]; then
  echo "手工演练全部通过。"
else
  echo "手工演练存在失败项，见上面 [失败]。"
fi
exit "$FAILED"
