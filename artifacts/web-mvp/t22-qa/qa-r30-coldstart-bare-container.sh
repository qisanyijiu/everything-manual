#!/usr/bin/env bash
# QA 回合 30（T22 Linux 半边）独立验收脚本：在**裸容器**里冷启动发布二进制。
#
# 目的（validation-release §7 第 7 步的 Linux 侧直接证据）：容器里**既没有 Node、没有 Python、
# 没有 cargo/rustc，也没有源码目录**——只有这一个二进制文件。若服务能 init/serve 并返回内嵌
# 页面、静态资源、JSON 404 与 health，则"运行期不依赖 Node/Python/源码/工具链"就不是推断。
#
# 用法：bash qa-r30-coldstart-bare-container.sh <绝对路径二进制> [镜像]
set -euo pipefail

BIN="${1:?用法: $0 <绝对路径二进制> [镜像]}"
IMAGE="${2:-alpine:3.20}"
PORT=18080

[[ -f "$BIN" ]] || { echo "二进制不存在：$BIN" >&2; exit 2; }

docker run --rm --platform linux/amd64 \
  -v "$BIN":/opt/em/everything-manual:ro \
  -e EM_PORT="$PORT" \
  --entrypoint /bin/sh "$IMAGE" -c '
set -eu
echo "== 容器内有什么（应为：无 node/python/cargo/源码） =="
for t in node python3 python cargo rustc git; do
  printf "%-8s %s\n" "$t" "$(command -v "$t" || echo 未安装)"
done
echo "根目录条目：$(ls /)"
echo "工作目录：$(pwd)"
echo "== 二进制 =="
ls -l /opt/em/everything-manual
echo "== init（全新 data-dir，无人值守密码文件） =="
mkdir -p /work/data
printf "qa-r30-bare-coldstart-pw\n" > /work/pw
chmod 600 /work/pw
# 先验证 fail-closed：权限过宽的密码文件必须被拒绝（本轮 644 实测退出码 3）
printf "qa-r30-bare-coldstart-pw\n" > /work/pw-wide
chmod 644 /work/pw-wide
if /opt/em/everything-manual init --data-dir /work/data-wide --password-file /work/pw-wide >/dev/null 2>&1; then
  echo "  [失败] 权限 644 的密码文件未被拒绝"; exit 1
else
  echo "  [检查] 权限过宽的密码文件被拒绝（fail-closed，退出码 $?）"
fi
/opt/em/everything-manual init --data-dir /work/data --password-file /work/pw
echo "init exit=$?"
echo "== serve（冷启动；仅内嵌资源） =="
/opt/em/everything-manual serve --data-dir /work/data --listen "127.0.0.1:${EM_PORT}" >/work/serve.log 2>&1 &
pid=$!
for i in $(seq 1 200); do
  wget -q -O /dev/null "http://127.0.0.1:${EM_PORT}/api/v1/health/live" 2>/dev/null && break
  sleep 0.1
done
fail=0
check() { # 名称 期望码 URL [额外断言]
  local name="$1" want="$2" url="$3"
  local code body
  body=$(wget -q -O - "http://127.0.0.1:${EM_PORT}${url}" 2>/dev/null || true)
  code=$(wget -q -O /dev/null -S "http://127.0.0.1:${EM_PORT}${url}" 2>&1 | awk "/^  HTTP\//{print \$2; exit}")
  printf "  [检查] %-34s -> %s（期望 %s）\n" "$name" "${code:-无}" "$want"
  [ "${code:-x}" = "$want" ] || fail=$((fail+1))
  [ "$code" = "$want" ] || printf "          body: %s\n" "$(printf "%s" "$body" | head -c 200)"
}
check "GET /（内嵌 index.html）" 200 /
check "GET /assets/index-D2JcMJ4C.js" 200 /assets/index-D2JcMJ4C.js
check "GET /assets/index-CE8-SplE.css" 200 /assets/index-CE8-SplE.css
check "GET /vendor/pdfjs/cmaps/78-EUC-H.bcmap" 200 /vendor/pdfjs/cmaps/78-EUC-H.bcmap
check "GET /api/v1/health/live" 200 /api/v1/health/live
check "GET /api/v1/health/ready" 200 /api/v1/health/ready
check "GET /api/unknown（JSON 404）" 404 /api/unknown
check "GET /library/some-item（SPA 回退）" 200 /library/some-item
check "GET /assets/definitely-missing.js" 404 /assets/definitely-missing.js
# 说明：busybox wget 在 404 时不输出 body，也不回显 Content-Type（实测），
# 故"未知 /api 必须是 JSON 而非 HTML"这条断言改由 smoke-bootstrap 在同一个发布二进制上
# 逐字断言（xtask/src/smoke.rs:177-184，要求 404 + application/json）；本脚本只断言状态码。
echo "  [说明] JSON 404 的内容类型断言由 smoke-bootstrap 覆盖（busybox wget 无法展示 404 响应头）"
echo "== 停服（SIGTERM） =="
kill -TERM "$pid"; wait "$pid" || true
echo "停服后 serve.log 末尾："; tail -3 /work/serve.log
echo "== 冷启动检查失败项：$fail =="
[ "$fail" -eq 0 ]
'
