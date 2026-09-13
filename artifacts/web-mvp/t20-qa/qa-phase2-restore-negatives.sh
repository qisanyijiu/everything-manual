#!/bin/bash
# QA 回合 25 · T20 独立验收 · 阶段 2
# 目的：AC-010 的负例（非空目标、损坏 blob、符号链接、路径穿越、非法 manifest、
#       新 schema、缺 blob 文件、目标被 flock 持有）——全部由 QA 自己构造。
set -u
BIN=/Users/qsyj/Code/rust/everything-manual/dist/aarch64-apple-darwin/everything-manual
WORK=/tmp/em-t20-qa
NEG=$WORK/neg
SRC=$WORK/my-backup

step() { printf '\n=== [%s] %s\n' "$1" "$2"; }

rm -rf "$NEG"; mkdir -p "$NEG"
run_case() { # name from target -> 打印退出码
  local name="$1" from="$2" target="$3"
  "$BIN" restore --from "$from" --data-dir "$target" > "$NEG/$name.out" 2> "$NEG/$name.err"
  local code=$?
  echo "exit=$code"
  echo "stderr: $(head -c 400 "$NEG/$name.err")"
  if [ -e "$target" ]; then echo "target EXISTS: $(ls -A "$target" | tr '\n' ' ')"; else echo "target NOT created"; fi
}

step A "非空目标目录 -> 期望 4，现场不动"
mkdir -p "$NEG/nonempty" && echo keep > "$NEG/nonempty/keep.txt"
run_case nonempty "$SRC" "$NEG/nonempty"
echo "keep.txt = $(cat "$NEG/nonempty/keep.txt")"

step B "目标是已存在文件 -> 期望 4"
echo "file" > "$NEG/target-file"
run_case target-file "$SRC" "$NEG/target-file"

step C "备份 blob 损坏（翻转 1 字节）-> 期望 7，目标不创建"
cp -R "$SRC" "$NEG/backup-corrupt"
BLOB=$(ls "$NEG/backup-corrupt/blobs"/*/* | head -1)
echo "victim=$BLOB"
python3 - "$BLOB" <<'PY'
import sys
p=sys.argv[1]
b=bytearray(open(p,'rb').read()); b[0]^=0xFF; open(p,'wb').write(bytes(b))
print("flipped 1 byte, size", len(b))
PY
run_case corrupt-blob "$NEG/backup-corrupt" "$NEG/t-corrupt"

step D "备份 blob 被换成符号链接 -> 期望 7"
cp -R "$SRC" "$NEG/backup-symlink"
BLOB2=$(ls "$NEG/backup-symlink/blobs"/*/* | tail -1)
rm "$BLOB2" && ln -s "$NEG/backup-symlink/manifest.json" "$BLOB2"
echo "symlink: $(ls -l "$BLOB2" | sed 's/.*-> /-> /')"
run_case symlink-blob "$NEG/backup-symlink" "$NEG/t-symlink"

step E "备份缺一个 blob 文件 -> 期望 7"
cp -R "$SRC" "$NEG/backup-missing-blob"
BLOB3=$(ls "$NEG/backup-missing-blob/blobs"/*/* | head -1)
rm "$BLOB3"
run_case missing-blob "$NEG/backup-missing-blob" "$NEG/t-missingblob"

step F "manifest 路径穿越（../../）-> 期望 7，目标不创建"
cp -R "$SRC" "$NEG/backup-traversal"
python3 - "$NEG/backup-traversal/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; m=json.load(open(p))
m["blobs"][0]["path"]="../../etc/passwd"
json.dump(m,open(p,'w'),ensure_ascii=False,indent=2)
print("path ->", m["blobs"][0]["path"])
PY
run_case traversal "$NEG/backup-traversal" "$NEG/t-traversal"

step F2 "manifest 绝对路径 -> 期望 7"
cp -R "$SRC" "$NEG/backup-abs"
python3 - "$NEG/backup-abs/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; m=json.load(open(p))
m["database"]["path"]="/etc/passwd"
json.dump(m,open(p,'w'),ensure_ascii=False,indent=2)
PY
run_case absolute-path "$NEG/backup-abs" "$NEG/t-abs"

step G "manifest 多余字段（未知字段）-> 期望 7，不 panic"
cp -R "$SRC" "$NEG/backup-unknown-field"
python3 - "$NEG/backup-unknown-field/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; m=json.load(open(p)); m["surprise"]=True
json.dump(m,open(p,'w'),ensure_ascii=False,indent=2)
PY
run_case unknown-field "$NEG/backup-unknown-field" "$NEG/t-unknown"

step G2 "manifest sha256 明显非法（短串）-> 期望受控失败（7），不 panic/不 101"
cp -R "$SRC" "$NEG/backup-short-sha"
python3 - "$NEG/backup-short-sha/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; m=json.load(open(p)); m["blobs"][0]["sha256"]="x"
json.dump(m,open(p,'w'),ensure_ascii=False,indent=2)
PY
run_case short-sha "$NEG/backup-short-sha" "$NEG/t-shortsha"

step G3 "manifest 未知格式版本 -> 期望 7"
cp -R "$SRC" "$NEG/backup-schema-fmt"
python3 - "$NEG/backup-schema-fmt/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; m=json.load(open(p)); m["schemaVersion"]="manual_backup_v9"
json.dump(m,open(p,'w'),ensure_ascii=False,indent=2)
PY
run_case bad-schema-format "$NEG/backup-schema-fmt" "$NEG/t-schemafmt"

step H "快照带 WAL 边车 -> 期望 7（拒绝'只复制主文件'的假快照）"
cp -R "$SRC" "$NEG/backup-sidecar"
echo "fake" > "$NEG/backup-sidecar/database/manual.sqlite3-wal"
run_case sidecar "$NEG/backup-sidecar" "$NEG/t-sidecar"

step I "备份 schema 比程序新 -> 期望 4（与 check/serve 同一门禁）"
cp -R "$SRC" "$NEG/backup-newer"
python3 - "$NEG/backup-newer" <<'PY'
import hashlib,json,sys,sqlite3
d=sys.argv[1]; snap=d+"/database/manual.sqlite3"
con=sqlite3.connect(snap)
con.execute("INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES (99,'future',0,1,X'00',0)")
con.commit(); con.close()
h=hashlib.sha256(open(snap,'rb').read()).hexdigest(); size=len(open(snap,'rb').read())
m=json.load(open(d+"/manifest.json")); m["database"]["sha256"]=h; m["database"]["size"]=size
json.dump(m,open(d+"/manifest.json",'w'),ensure_ascii=False,indent=2)
print("injected v99, snapshot sha", h[:16])
PY
run_case newer-schema "$NEG/backup-newer" "$NEG/t-newer"

step J "备份目录不存在 -> 期望 4"
run_case missing-backup "$NEG/no-such-backup" "$NEG/t-nobackup"

step K "恢复目标被其他进程 flock 持有 -> 观察实际退出码"
mkdir -p "$NEG/locked-empty"
python3 - "$NEG/locked-empty/lock" <<'PY' &
import fcntl,sys,time
f=open(sys.argv[1],'w'); fcntl.flock(f,fcntl.LOCK_EX)
print("flock held", flush=True); time.sleep(8)
PY
sleep 1
run_case locked-target "$SRC" "$NEG/locked-empty"
wait

step L "合法恢复（对照）-> 期望 0"
run_case control "$SRC" "$NEG/t-control"

echo
echo "phase2 完成"
