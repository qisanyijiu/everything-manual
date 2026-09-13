#!/bin/bash
# QA 回合 26 · 导出兜底策略的**误伤检查**：用户内容里的出处链接（knowledge.sourceUrl）
# 必须**不**触发 fail-closed（否则"用户内容含 URL 就导不出"= 误伤）。
# 手法：restore 归档样例 → 把用户链接写进冻结 release manifest 的 knowledge 子树
# （重算 blob sha 并改 assets.blob_id，release 行不动）→ 导出 → 期望 200 且包内
# manifest.json 的 knowledge 保留该链接（而“生成字段”出现 URL 时仍拒绝，见主剧本第 9 步）。
# 全程只连 127.0.0.1；零外网、零付费。
set -uo pipefail
BIN="${1:?用法: qa-r26-export-userurl.sh <dist 二进制绝对路径>}"
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
SAMPLE="${ROOT}/artifacts/web-mvp/t20-rd/sample-backup"
INFO="${ROOT}/artifacts/web-mvp/t20-rd/rehearsal-info.json"
WORK="$(mktemp -d /tmp/em-r26-export-user-XXXXXX)"
APP_PORT=18132
USER_CANARY="qa-r26-user-src-58be"
RESULT=0
ok()  { echo "  [通过] $1"; }
bad() { echo "  [失败] $1"; RESULT=1; }
cleanup() { [[ -n "${SERVE_PID:-}" ]] && kill -9 "${SERVE_PID}" 2>/dev/null; wait 2>/dev/null; rm -rf "${WORK}"; }
trap cleanup EXIT
mkdir -p "${WORK}"; cd "${WORK}" || exit 1

PW=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['password'])" "${INFO}")
RELEASE_ID=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['releaseId'])" "${INFO}")

echo "== QA 回合 26 · 导出：用户内容 URL 不误伤（canary=${USER_CANARY}）"
"${BIN}" restore --from "${SAMPLE}" --data-dir ./data > restore.log 2>&1 || { echo "restore 失败"; exit 1; }
python3 - ./data "${USER_CANARY}" <<'PY'
import hashlib, json, os, sqlite3, sys
data_dir, canary = sys.argv[1], sys.argv[2]
con = sqlite3.connect(os.path.join(data_dir, "manual.sqlite3"))
asset_id, sha, mime = con.execute(
    "select a.id, b.sha256, b.mime from assets a join blobs b on b.sha256 = a.blob_id "
    "where a.purpose='release_manifest'").fetchone()
path = os.path.join(data_dir, "blobs", sha[:2], sha)
manifest = json.load(open(path, encoding="utf-8"))
manifest["knowledge"]["knowledge"]["userNote"] = f"用户出处链接 https://user.example.invalid/spec?sig={canary}"
raw = json.dumps(manifest, ensure_ascii=False).encode("utf-8")
new_sha = hashlib.sha256(raw).hexdigest()
os.makedirs(os.path.join(data_dir, "blobs", new_sha[:2]), exist_ok=True)
open(os.path.join(data_dir, "blobs", new_sha[:2], new_sha), "wb").write(raw)
con.execute("insert or replace into blobs (sha256, size, mime, storage_state, created_at) values (?,?,?,?,?)",
            (new_sha, len(raw), mime, "stored", 1789233087241))
con.execute("update assets set blob_id = ? where id = ?", (new_sha, asset_id))
con.commit()
print(f"  注入完成：knowledge 内含用户链接（新 manifest sha={new_sha[:12]}…）")
PY

printf '{"password":"%s"}' "${PW}" > body.json && chmod 600 body.json
QA_TRIPO_KEY=qa-r26 QA_MANUAL_AI_KEY=qa-r26 \
  "${BIN}" serve --data-dir ./data --listen "127.0.0.1:${APP_PORT}" > serve.log 2>&1 &
SERVE_PID=$!
for _ in $(seq 1 100); do
  curl -sf "http://127.0.0.1:${APP_PORT}/api/v1/health/ready" >/dev/null && break
  sleep 0.1
done
BASE="http://127.0.0.1:${APP_PORT}/api/v1"
curl -s -c jar -H 'content-type: application/json' --data-binary @body.json "${BASE}/auth/login" -o login.json
code=$(curl -s -o export.zip -w '%{http_code}' -b jar "${BASE}/releases/${RELEASE_ID}/export")
if [[ "${code}" == "200" ]]; then ok "用户内容含 URL 时导出仍成功（200）"; else bad "导出被误伤（status=${code}）"; fi
python3 - export.zip "${USER_CANARY}" <<'PY'
import json, sys, zipfile
canary = sys.argv[2]
with zipfile.ZipFile(sys.argv[1]) as archive:
    names = archive.namelist()
    export_manifest = json.loads(archive.read("manifest.json"))
    frozen = archive.read("release/manifest.json").decode("utf-8")
print("  条目：", ", ".join(sorted(names)))
note = export_manifest["knowledge"]["knowledge"]["userNote"]
print("  manifest.json 的 knowledge.userNote =", note)
assert canary in note, note
print("  ✔ 用户出处链接随包保留（未被 fail-closed 拦下、也未被清洗）")
assert "://" in note, note
print("  ✔ 包内 '://' 仅来自用户内容字段（这是要求而不是泄露）")
PY
if [[ $? -eq 0 ]]; then ok "导出包保留用户链接（并含 '://'，符合 ADR-032 第 4 条）"; else bad "导出包内容检查失败"; fi

echo "== 结果：$([[ ${RESULT} = 0 ]] && echo PASS || echo FAIL)"
exit ${RESULT}
