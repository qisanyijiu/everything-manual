#!/bin/bash
# T05 QA 回合 5：AC-013 release 路径独立冒烟
# 1) release 二进制 + 空 data-dir + 无密钥：init → serve → health/ready 200，
#    启动日志明确"未配置、不回退 mock"；进程无任何外部连接（lsof）。
# 2) providers.tripo.base_url 指向不可达地址（TEST-NET 203.0.113.0/24）→ ready 仍 200。
# 使用 QA 自建临时目录；脚本结束清理进程与目录。
set -u
REPO=/Users/qsyj/Code/rust/everything-manual
BIN=$REPO/dist/aarch64-apple-darwin/everything-manual
WORK=/tmp/em-t05-qa-release-smoke
PASSWORD="QA_RELEASE_T05_canary_pw"

rm -rf "$WORK"
mkdir -p "$WORK/bin" "$WORK/data"
cp "$BIN" "$WORK/bin/everything-manual"
cd "$WORK/bin" || exit 1

echo "=== binary sha256 ==="
shasum -a 256 everything-manual

umask 077
printf '%s' "$PASSWORD" > "$WORK/pw.txt"
chmod 600 "$WORK/pw.txt"

echo "=== init ==="
./everything-manual init --data-dir "$WORK/data" --password-file "$WORK/pw.txt"
echo "init exit=$?"

start_serve() {
  local tag="$1"
  ./everything-manual serve --data-dir "$WORK/data" --listen 127.0.0.1:0 > "$WORK/serve-$tag.log" 2>&1 &
  SERVE_PID=$!
  ADDR=""
  for _ in $(seq 1 60); do
    ADDR=$(grep -o 'listening on http://[0-9.:]*' "$WORK/serve-$tag.log" | head -1 | sed 's/listening on //')
    [ -n "$ADDR" ] && break
    sleep 0.2
  done
  echo "serve pid=$SERVE_PID addr=$ADDR"
}

stop_serve() {
  kill "$SERVE_PID" 2>/dev/null
  wait "$SERVE_PID" 2>/dev/null
}

# --- 阶段 1：默认配置（无 Provider 密钥） ---
echo "=== phase 1: serve without provider keys ==="
start_serve p1
[ -n "$ADDR" ] || { echo "FAIL: 未解析到监听地址"; cat "$WORK/serve-p1.log"; exit 1; }

curl -s -o "$WORK/health-live.json" -w "GET /health/live -> %{http_code}\n" "$ADDR/api/v1/health/live"
curl -s -o "$WORK/health-ready.json" -w "GET /health/ready -> %{http_code}\n" "$ADDR/api/v1/health/ready"
echo "ready body: $(cat "$WORK/health-ready.json")"

echo "--- lsof（进程持有的网络连接）---"
lsof -p "$SERVE_PID" -a -i -P -n 2>/dev/null | tee "$WORK/lsof-p1.txt"

echo "--- provider_not_configured 日志行 ---"
grep -o '"event":"provider_not_configured"[^\n]*' "$WORK/serve-p1.log" | head -4
echo "grep_provider_not_configured_count=$(grep -c 'provider_not_configured' "$WORK/serve-p1.log")"
echo "grep_回退mock_count=$(grep -c '回退 mock' "$WORK/serve-p1.log")"

echo "--- 登录 + /settings/status ---"
curl -s -c "$WORK/jar.txt" -o "$WORK/login.json" -w "POST /auth/login -> %{http_code}\n" \
  -X POST "$ADDR/api/v1/auth/login" -H 'content-type: application/json' \
  -d "{\"password\":\"$PASSWORD\"}"
CSRF=$(python3 -c "import json;print(json.load(open('$WORK/login.json'))['data']['csrfToken'])" 2>/dev/null)
curl -s -b "$WORK/jar.txt" -H "x-csrf-token: $CSRF" -o "$WORK/settings-status.json" -w "GET /settings/status -> %{http_code}\n" "$ADDR/api/v1/settings/status"
cat "$WORK/settings-status.json"; echo

echo "--- 进程仍无外部连接（二次采样）---"
lsof -p "$SERVE_PID" -a -i -P -n 2>/dev/null | tail -n +2 | grep -v "127.0.0.1" | tee "$WORK/lsof-p1-nonloopback.txt"
echo "nonloopback_lines=$(wc -l < "$WORK/lsof-p1-nonloopback.txt" | tr -d ' ')"
stop_serve
echo "serve exit=$?"

# --- 阶段 2：base_url 指向不可达云端 ---
echo "=== phase 2: unreachable cloud base_url ==="
cat > "$WORK/bin/config.toml" <<'EOF'
[providers.tripo]
base_url = "https://203.0.113.9/v3"
EOF
start_serve p2
[ -n "$ADDR" ] || { echo "FAIL: 未解析到监听地址"; cat "$WORK/serve-p2.log"; exit 1; }
curl -s -o "$WORK/health-ready-p2.json" -w "GET /health/ready -> %{http_code}\n" "$ADDR/api/v1/health/ready"
echo "ready body: $(cat "$WORK/health-ready-p2.json")"
echo "grep_provider_not_configured_count=$(grep -c 'provider_not_configured' "$WORK/serve-p2.log")"
lsof -p "$SERVE_PID" -a -i -P -n 2>/dev/null | tail -n +2 | grep -v "127.0.0.1" | tee "$WORK/lsof-p2-nonloopback.txt"
echo "nonloopback_lines=$(wc -l < "$WORK/lsof-p2-nonloopback.txt" | tr -d ' ')"
stop_serve

echo "=== 完成；保留 $WORK 供复核（QA 收尾时删除）==="
