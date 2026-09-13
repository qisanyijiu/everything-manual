#!/usr/bin/env bash
# T04 手工冒烟：真实 data-dir + 发布二进制（embedded-ui）+ curl 走一遍
# 登录/会话/登出/401/403/428/412/429/JSON404/settings/ready。
# 原始输出直接进 smoke-manual.log（本脚本自身输出）。
set -u

ROOT=/Users/qsyj/Code/rust/everything-manual
BIN="$ROOT/dist/aarch64-apple-darwin/everything-manual"
PORT=18081
BASE="http://127.0.0.1:$PORT"
WORK=$(mktemp -d /tmp/em-t04-manual.XXXXXX)
ITEM_ID=01993000-0000-7000-8000-000000000001
PASSWORD=t04-smoke-password-1a2b

echo "== 环境 =="
echo "binary: $BIN"
echo "work:   $WORK"
uname -a
"$BIN" --version

cd "$WORK" || exit 1
printf '%s\n' "$PASSWORD" > pw.txt && chmod 600 pw.txt

echo
echo "== 1) init（受限文件提供密码；Argon2id 入 admins 表）=="
"$BIN" init --data-dir data --password-file pw.txt
echo "init 退出码=$?"

echo
echo "== 2) 直接插入一个物品（T04 没有创建端点；AC-016 属 T07）=="
sqlite3 data/manual.sqlite3 \
  "INSERT INTO items (id,name,brand,model,variant,revision,archived_at,created_at,updated_at) \
   VALUES ('$ITEM_ID','示例扳手','示例品牌','W-100',NULL,1,NULL,1789000000000,1789000000000);" \
  && echo "insert ok"
sqlite3 data/manual.sqlite3 "SELECT id,name,revision FROM items;"

echo
echo "== 3) serve（后台）=="
"$BIN" serve --data-dir data --listen "127.0.0.1:$PORT" >serve.log 2>&1 &
SERVE_PID=$!
for _ in $(seq 1 100); do
  if grep -q "listening on" serve.log 2>/dev/null; then break; fi
  sleep 0.1
done
grep "listening on" serve.log || { echo "服务未启动"; cat serve.log; exit 1; }

login_body="$WORK/login.json"
login_headers="$WORK/login.headers"

echo
echo "== 4) 未登录 GET /api/v1/items → 401（含 requestId）=="
curl -s -i "$BASE/api/v1/items" | tr -d '\r'

echo
echo "== 5) 正确密码登录 → 200 + Set-Cookie（HttpOnly/SameSite=Strict，HTTP 下无 Secure）+ csrfToken =="
curl -s -D "$login_headers" -o "$login_body" \
  -c "$WORK/cookies.txt" \
  -H 'content-type: application/json' \
  -d "{\"password\":\"$PASSWORD\"}" \
  "$BASE/api/v1/auth/login"
tr -d '\r' < "$login_headers"
cat "$login_body"; echo
CSRF=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["data"]["csrfToken"])' "$login_body")
echo "csrfToken=${CSRF:0:16}…（长度 ${#CSRF}）"

echo
echo "== 6) GET /auth/session → 200 + Cache-Control: no-store（同一 CSRF）=="
curl -s -i -b "$WORK/cookies.txt" "$BASE/api/v1/auth/session" | tr -d '\r'

echo
echo "== 7) PATCH 缺 CSRF → 403 CSRF_REJECTED =="
curl -s -i -X PATCH -b "$WORK/cookies.txt" \
  -H 'content-type: application/json' -H 'if-match: "r1"' \
  -d '{"name":"改名"}' "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'

echo
echo "== 8) PATCH 带 CSRF 但跨站 Origin → 403 ORIGIN_REJECTED =="
curl -s -i -X PATCH -b "$WORK/cookies.txt" \
  -H "x-csrf-token: $CSRF" -H 'origin: https://attacker.example' \
  -H 'content-type: application/json' -H 'if-match: "r1"' \
  -d '{"name":"改名"}' "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'

echo
echo "== 9) PATCH 带 CSRF、缺 If-Match → 428 PRECONDITION_REQUIRED =="
curl -s -i -X PATCH -b "$WORK/cookies.txt" \
  -H "x-csrf-token: $CSRF" \
  -H 'content-type: application/json' \
  -d '{"name":"改名"}' "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'

echo
echo "== 10) 同源 Origin + CSRF + If-Match \"r1\" → 200 新 ETag =="
curl -s -i -X PATCH -b "$WORK/cookies.txt" \
  -H "x-csrf-token: $CSRF" -H "origin: $BASE" \
  -H 'content-type: application/json' -H 'if-match: "r1"' \
  -d '{"name":"改名成功"}' "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'

echo
echo "== 11) 再次用旧 If-Match \"r1\" → 412 + details.currentRevision =="
curl -s -i -X PATCH -b "$WORK/cookies.txt" \
  -H "x-csrf-token: $CSRF" -H "origin: $BASE" \
  -H 'content-type: application/json' -H 'if-match: "r1"' \
  -d '{"name":"再改"}' "$BASE/api/v1/items/$ITEM_ID" | tr -d '\r'

echo
echo "== 12) GET /settings/status → 200（未配置 Provider，不含密钥）=="
curl -s -i -b "$WORK/cookies.txt" "$BASE/api/v1/settings/status" | tr -d '\r'

echo
echo "== 13) GET /health/ready → 200（process/data_directory/database/migrations）=="
curl -s -i "$BASE/api/v1/health/ready" | tr -d '\r'

echo
echo "== 14) GET /api/unknown → JSON 404（非 HTML）=="
curl -s -i "$BASE/api/unknown" | tr -d '\r'

echo
echo "== 15) POST /auth/logout（带 CSRF）→ 204；旧 cookie 再用 → 401 =="
curl -s -i -X POST -b "$WORK/cookies.txt" -H "x-csrf-token: $CSRF" "$BASE/api/v1/auth/logout" | tr -d '\r'
echo
curl -s -i -b "$WORK/cookies.txt" "$BASE/api/v1/items" | tr -d '\r'

echo
echo "== 16) 连续 5 次错误密码 → 401（通用提示）；第 6 次（密码正确）→ 429 + Retry-After =="
echo "（第 1 次失败的完整响应：提示不区分“未初始化/密码错误”，也不回显密码）"
curl -s -i -H 'content-type: application/json' \
  -d '{"password":"wrong-password"}' "$BASE/api/v1/auth/login" | tr -d '\r'
for i in 2 3 4 5; do
  code=$(curl -s -o /dev/null -w '%{http_code}' -H 'content-type: application/json' \
    -d '{"password":"wrong-password"}' "$BASE/api/v1/auth/login")
  echo "第 $i 次错误密码 → HTTP $code"
done
curl -s -i -H 'content-type: application/json' \
  -d "{\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login" | tr -d '\r'

echo
echo "== 17) data-dir 文件权限 =="
ls -la data data/manual.sqlite3

echo
echo "== 18) 服务端日志（节选：登录/401/403/412 的 requestId 关联）=="
grep -E "login|http_request|csrf|origin" serve.log | tail -20

kill "$SERVE_PID" 2>/dev/null
wait "$SERVE_PID" 2>/dev/null
echo
echo "== 冒烟结束（退出码 $?）=="
echo "清理：$WORK"
