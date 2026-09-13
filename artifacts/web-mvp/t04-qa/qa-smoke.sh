#!/usr/bin/env bash
# T04 QA 独立手工冒烟（QA 回合 4）——不采信 RD 脚本结论，独立复现。
# 覆盖：init 落库/重跑语义、cookie 属性、Secure 不信转发头、CSRF/Origin、If-Match 428/412、
# 401/403/404/405、settings 无密钥、ready 数据层负例、logout、限速、日志与库内哈希核对。
set -u

ROOT=/Users/qsyj/Code/rust/everything-manual
BIN="$ROOT/dist/aarch64-apple-darwin/everything-manual"
PORT=18095
BASE="http://127.0.0.1:$PORT"
WORK=$(mktemp -d /tmp/em-t04-qa.XXXXXX)
PASSWORD='qa-t04-canary-pw-7f2c'
PASSWORD2='qa-t04-canary-pw-REUSED-9b41'
ITEM_ID=01993000-0000-7000-8000-0000000000aa

step() { echo; echo "===== $* ====="; }
sql() { sqlite3 "$WORK/data/manual.sqlite3" "$@"; }

echo "QA smoke start: $(date -u +%Y-%m-%dT%H:%M:%SZ) (UTC)"
echo "binary: $BIN  sha256=$(shasum -a 256 "$BIN" | awk '{print $1}')"
echo "work:   $WORK"
"$BIN" --version
uname -a

cd "$WORK" || exit 1
printf '%s\n' "$PASSWORD" > pw.txt && chmod 600 pw.txt

step "0) init 前 data-dir（应不存在）"
ls -la "$WORK" || true

step "1) init（受限文件）→ 库/管理员凭据落库"
"$BIN" init --data-dir data --password-file pw.txt; echo "init 退出码=$?"
step "1a) admins 表内容（哈希，非明文）"
sql "SELECT id, length(password_hash), substr(password_hash,1,40) FROM admins;"
sql "SELECT count(*) AS admins_count FROM admins;"
step "1b) 明文是否出现在整个 data-dir（预期 0 命中）"
grep -rIl "$PASSWORD" data 2>/dev/null | wc -l
step "1c) init 输出/日志中是否含明文（预期 0 命中）"
grep -c "$PASSWORD" "$WORK"/data/logs/everything-manual.log 2>/dev/null || echo "0（无命中）"

step "2) 插入一个物品（T04 无创建端点，AC-016 属 T07）"
sql "INSERT INTO items (id,name,brand,model,variant,revision,archived_at,created_at,updated_at) VALUES ('$ITEM_ID','QA示例扳手','QA品牌','QA-M100',NULL,1,NULL,1789000000000,1789000000000);"
sql "SELECT id,name,revision FROM items;"

step "3) check 摘要（session 行）"
"$BIN" check --data-dir data; echo "check 退出码=$?"

step "4) serve（后台，127.0.0.1:${PORT}）"
"$BIN" serve --data-dir data --listen "127.0.0.1:$PORT" >serve.log 2>&1 &
SERVE_PID=$!
for _ in $(seq 1 100); do
  if grep -q "listening on" serve.log 2>/dev/null; then break; fi
  sleep 0.1
done
grep "listening on" serve.log || { echo "服务未启动"; cat serve.log; exit 1; }
echo "serve pid=$SERVE_PID"

LOGIN_H="$WORK/login.headers"; LOGIN_B="$WORK/login.body"

step "5) 未登录 GET /api/v1/items → 401（四键 + requestId 与响应头一致）"
curl -sS -i "$BASE/api/v1/items" | tr -d '\r'
echo "--- 未登录 401 的 requestId 关联（从响应头取，核对日志）---"
RID401=""
step "6) 登录（正确密码）→ cookie 属性 + csrfToken"
curl -sS -D "$LOGIN_H" -o "$LOGIN_B" -c "$WORK/cookies.txt" \
  -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" \
  "$BASE/api/v1/auth/login"
tr -d '\r' < "$LOGIN_H"
cat "$LOGIN_B"; echo
CSRF=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["data"]["csrfToken"])' "$LOGIN_B")
COOKIE_TOKEN=$(grep -o 'em_session=[^;]*' "$WORK/cookies.txt" | head -1 | cut -d= -f2)
echo "csrf 长度=${#CSRF}  session token 长度=${#COOKIE_TOKEN}"

step "6a) 库内会话 token 是否只存哈希（预期 sha256(token) 命中）"
echo "sha256(cookie token) = $(printf '%s' "$COOKIE_TOKEN" | shasum -a 256 | awk '{print $1}')"
sql "SELECT session_token_hash, csrf_hash, length(csrf_hash), created_at, expires_at, revoked_at FROM sessions;"
echo "sha256(csrf)        = $(printf '%s' "$CSRF" | shasum -a 256 | awk '{print $1}')"
step "6b) 明文 token / csrf 是否出现在库文件与日志（预期 0）"
grep -c "$COOKIE_TOKEN" data/manual.sqlite3 2>/dev/null | head -1 || true
strings data/manual.sqlite3 | grep -c "$COOKIE_TOKEN" || echo "0"
grep -c "$CSRF" serve.log || echo "0（日志无 csrf）"

step "7) GET /auth/session → 200 + no-store + 同一 CSRF"
curl -sS -i -b "$WORK/cookies.txt" "$BASE/api/v1/auth/session" | tr -d '\r'

step "8) Secure 判定不相信 X-Forwarded-*：伪造 XFP/XFProto 登录不应带 Secure"
XF_B="$WORK/login-xf.body"
curl -sS -D "$WORK/login-xf.headers" -o "$XF_B" \
  -H 'content-type: application/json' \
  -H 'x-forwarded-proto: https' -H 'x-forwarded-ssl: on' -H 'forwarded: proto=https' \
  -d "{\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login"
grep -i '^set-cookie' "$WORK/login-xf.headers" | tr -d '\r'
if grep -i '^set-cookie' "$WORK/login-xf.headers" | grep -qi 'secure'; then
  echo "结论：XFP 伪造导致 Secure=是（异常）"
else
  echo "结论：XFP 伪造未影响 Secure（符合 T04-3）"
fi

step "9) PATCH 缺 CSRF → 403 CSRF_REJECTED"
curl -sS -i -X PATCH -b "$WORK/cookies.txt" -H 'if-match: "r1"' \
  -H 'content-type: application/json' -d '{"name":"改名"}' \
  "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'

step "10) PATCH 带 CSRF 但跨站 Origin → 403 ORIGIN_REJECTED"
curl -sS -i -X PATCH -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF" \
  -H 'origin: https://attacker.example' -H 'if-match: "r1"' \
  -H 'content-type: application/json' -d '{"name":"改名"}' \
  "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'

step "11) PATCH 跨会话 CSRF（用另一会话的 CSRF）→ 403"
CSRF_B="$WORK/login2.body"
curl -sS -o "$CSRF_B" -c "$WORK/cookies2.txt" -H 'content-type: application/json' \
  -d "{\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login"
CSRF2=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["data"]["csrfToken"])' "$CSRF_B")
curl -sS -o /dev/null -w 'sessionA cookie + sessionB csrf → HTTP %{http_code}\n' -X PATCH \
  -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF2" -H 'if-match: "r1"' \
  -H 'content-type: application/json' -d '{"name":"改名"}' \
  "$BASE/api/v1/items/$ITEM_ID"

step "12) PATCH 缺 If-Match → 428"
curl -sS -i -X PATCH -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF" \
  -H 'content-type: application/json' -d '{"name":"改名"}' \
  "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'

step "13) PATCH If-Match: * → 422（严格解析）"
curl -sS -o /dev/null -w 'If-Match:* → HTTP %{http_code}\n' -X PATCH -b "$WORK/cookies.txt" \
  -H "x-csrf-token: $CSRF" -H 'if-match: *' \
  -H 'content-type: application/json' -d '{"name":"改名"}' "$BASE/api/v1/items/$ITEM_ID"

step "14) GET item → ETag；PATCH If-Match \"r1\" → 200 + 新 ETag"
curl -sS -i -b "$WORK/cookies.txt" "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r' | grep -E 'HTTP/|etag|revision|content-type'
curl -sS -i -X PATCH -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF" -H 'origin: http://127.0.0.1:18095' \
  -H 'if-match: "r1"' -H 'content-type: application/json' -d '{"name":"改名成功"}' \
  "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'

step "15) 过期 revision（再用 r1）→ 412 + details.currentRevision"
curl -sS -i -X PATCH -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF" \
  -H 'if-match: "r1"' -H 'content-type: application/json' -d '{"name":"再改"}' \
  "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'

step "16) 列表 {data,nextCursor}；非法 limit → 422"
curl -sS -b "$WORK/cookies.txt" "$BASE/api/v1/items?limit=1" | head -c 400; echo
curl -sS -o /dev/null -w 'limit=0 → HTTP %{http_code}\n' -b "$WORK/cookies.txt" "$BASE/api/v1/items?limit=0"

step "17) /api/unknown → JSON 404；DELETE /api/v1/items → 405 语义；/api/v1/items/xxx DELETE"
curl -sS -i "$BASE/api/unknown" | tr -d '\r'
curl -sS -i -X DELETE -b "$WORK/cookies.txt" "$BASE/api/v1/items" | tr -d '\r'
curl -sS -i -X DELETE -b "$WORK/cookies.txt" "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'
curl -sS -o /dev/null -w 'GET 不存在物品 → HTTP %{http_code}\n' -b "$WORK/cookies.txt" "$BASE/api/v1/items/01993000-0000-7000-8000-0000000000ff"

step "18) /settings/status（未配置 Provider，无密钥）"
curl -sS -i -b "$WORK/cookies.txt" "$BASE/api/v1/settings/status" | tr -d '\r'
echo "--- 相关字面量命中检查（预期 0）---"
curl -sS -b "$WORK/cookies.txt" "$BASE/api/v1/settings/status" > "$WORK/settings.json"
for needle in api_key apiKey base_url baseUrl data_dir dataDir listen openapi.tripo3d api.openai secret; do
  printf '%s: %s\n' "$needle" "$(grep -c "$needle" "$WORK/settings.json" || true)"
done

step "19) /health/ready（正常）→ 200 四项"
curl -sS -i "$BASE/api/v1/health/ready" | tr -d '\r'

step "20) ready 负例：把数据库文件移走 → 503 not_ready data_directory"
mv data/manual.sqlite3 data/manual.sqlite3.bak
curl -sS -o "$WORK/ready-neg.json" -w 'HTTP %{http_code}\n' "$BASE/api/v1/health/ready"
cat "$WORK/ready-neg.json"; echo
mv data/manual.sqlite3.bak data/manual.sqlite3
curl -sS -o /dev/null -w '恢复后 ready → HTTP %{http_code}\n' "$BASE/api/v1/health/ready"
ls -la data/manual.sqlite3*

step "21) 伪造/过期 cookie → 401"
curl -sS -o /dev/null -w '伪造 cookie → HTTP %{http_code}\n' -H 'cookie: em_session=deadbeef' "$BASE/api/v1/items"
ADMIN_ID=$(sql "SELECT id FROM admins LIMIT 1;")
sql "INSERT INTO sessions (id, admin_id, session_token_hash, csrf_hash, created_at, expires_at, revoked_at) VALUES ('01993000-0000-7000-8000-00000000ee01','$ADMIN_ID', '$(printf '%s' 'expired-token-canary' | shasum -a 256 | awk '{print $1}')', '$(printf '%s' 'x' | shasum -a 256 | awk '{print $1}')', 1788000000000, 1788100000000, NULL);"
curl -sS -o /dev/null -w '过期会话 cookie → HTTP %{http_code}\n' -H 'cookie: em_session=expired-token-canary' "$BASE/api/v1/items"

step "22) 注销（带 CSRF）→ 204 + 清 cookie；旧 cookie → 401"
curl -sS -i -X POST -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF" "$BASE/api/v1/auth/logout" | tr -d '\r'
curl -sS -o /dev/null -w '注销后旧 cookie → HTTP %{http_code}\n' -b "$WORK/cookies.txt" "$BASE/api/v1/items"
sql "SELECT id, revoked_at IS NOT NULL AS revoked FROM sessions WHERE session_token_hash='$(printf '%s' "$COOKIE_TOKEN" | shasum -a 256 | awk '{print $1}')';"

step "23) 限速：5 次错误密码 → 401；第 6 次（正确密码）→ 429 + Retry-After"
for i in 1 2 3 4 5; do
  code=$(curl -sS -o /dev/null -w '%{http_code}' -H 'content-type: application/json' \
    -d '{"password":"wrong-password"}' "$BASE/api/v1/auth/login")
  echo "第 $i 次错误密码 → HTTP $code"
done
curl -sS -i -H 'content-type: application/json' -d "{\"password\":\"$PASSWORD\"}" \
  "$BASE/api/v1/auth/login" | tr -d '\r'

step "24) 日志核对：requestId 关联、无请求体、无查询串、无密码"
echo "--- http_request 行（节选）---"
grep http_request serve.log | tail -8
echo "--- 401 响应体 requestId 与日志同值核对 ---"
RID=$(curl -sS "$BASE/api/v1/items" | python3 -c 'import json,sys;print(json.load(sys.stdin)["error"]["requestId"])')
echo "response requestId=$RID"
grep -c "$RID" serve.log
echo "--- 日志中是否出现密码/查询串 canary（预期 0 命中）---"
grep -c "$PASSWORD" serve.log || echo "0（日志无密码）"
curl -sS -o /dev/null -b "$WORK/cookies.txt" "$BASE/api/v1/items?secret=QA_QUERY_CANARY_1" || true
grep -c "QA_QUERY_CANARY_1" serve.log || echo "0（日志无查询串）"
echo "--- 日志是否记录请求体（登录行不应出现 body 字段）---"
grep -c '"body"' serve.log || echo "0（无 body 字段）"

step "25) 停服后重跑 init（重置密码）→ 已更新 + 撤销全部会话；旧 cookie 失效"
kill "$SERVE_PID" 2>/dev/null; wait "$SERVE_PID" 2>/dev/null
printf '%s\n' "$PASSWORD2" > pw2.txt && chmod 600 pw2.txt
"$BIN" init --data-dir data --password-file pw2.txt; echo "init(重跑) 退出码=$?"
sql "SELECT count(*) AS sessions_total, sum(revoked_at IS NOT NULL) AS revoked FROM sessions;"
sql "SELECT substr(password_hash,1,40) FROM admins;"
echo "--- 明文 PASSWORD2 是否落盘（预期 0）---"
strings data/manual.sqlite3 | grep -c "$PASSWORD2" || echo "0"

step "26) 新密码登录成功；旧密码失败"
"$BIN" serve --data-dir data --listen "127.0.0.1:$PORT" >serve2.log 2>&1 &
SERVE_PID2=$!
for _ in $(seq 1 100); do
  if grep -q "listening on" serve2.log 2>/dev/null; then break; fi
  sleep 0.1
done
curl -sS -o /dev/null -w '旧密码 → HTTP %{http_code}\n' -H 'content-type: application/json' \
  -d "{\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login"
curl -sS -o /dev/null -w '新密码 → HTTP %{http_code}\n' -H 'content-type: application/json' \
  -d "{\"password\":\"$PASSWORD2\"}" "$BASE/api/v1/auth/login"

step "27) 停服并检查残留"
kill "$SERVE_PID2" 2>/dev/null; wait "$SERVE_PID2" 2>/dev/null
ls -la "$WORK"; ls -la data
echo "--- 残留进程检查 ---"
pgrep -fl "everything-manual serve" || echo "无残留 serve 进程"
cp serve2.log "$ROOT/artifacts/web-mvp/t04-qa/serve-restart.log" 2>/dev/null
cp data/logs/everything-manual.log "$ROOT/artifacts/web-mvp/t04-qa/server-log-full.log" 2>/dev/null
echo "QA smoke end: $(date -u +%Y-%m-%dT%H:%M:%SZ) (UTC)"
rm -rf "$WORK"
echo "已清理：${WORK}；残留检查 $(ls -d $WORK 2>/dev/null || echo 已删除)"
