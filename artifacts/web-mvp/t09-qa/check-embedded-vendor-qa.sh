#!/usr/bin/env bash
# QA 独立复核（回合 9，T09）：release 二进制内嵌的 PDF.js vendor 资源可服务且字节一致。
# 与 `artifacts/web-mvp/t09-rd/check-embedded-vendor.sh` **无共享代码**（QA 自写）。
#
# 用法：bash artifacts/web-mvp/t09-qa/check-embedded-vendor-qa.sh <dist 二进制绝对路径>
set -euo pipefail

SOURCE_BIN="${1:?用法: check-embedded-vendor-qa.sh <绝对路径>}"
PORT=18097
DIR="$(mktemp -d /tmp/em-t09-qa-embed.XXXXXX)"
BIN="$DIR/everything-manual"
cp "$SOURCE_BIN" "$BIN"
printf 'qa-embed-password\n' > "$DIR/pw.txt"
chmod 600 "$DIR/pw.txt"
"$BIN" init --data-dir "$DIR/data" --password-file "$DIR/pw.txt" >"$DIR/init.log" 2>&1
"$BIN" serve --data-dir "$DIR/data" --listen "127.0.0.1:$PORT" >"$DIR/server.log" 2>&1 &
PID=$!
cleanup() { kill "$PID" 2>/dev/null || true; rm -rf "$DIR"; }
trap cleanup EXIT

for _ in $(seq 1 40); do
  if curl -sf "http://127.0.0.1:$PORT/api/v1/health/ready" >/dev/null 2>&1; then break; fi
  sleep 0.25
done

echo "QA 内嵌 vendor 复核（二进制：${SOURCE_BIN}）"
FAIL=0
for path in \
  "/vendor/pdfjs/cmaps/UniGB-UCS2-H.bcmap" \
  "/vendor/pdfjs/cmaps/UniJIS-UCS2-H.bcmap" \
  "/vendor/pdfjs/standard_fonts/LiberationSans-Regular.ttf" \
  "/vendor/pdfjs/standard_fonts/FoxitSerif.pfb" \
  "/vendor/pdfjs/wasm/openjpeg.wasm" \
  "/vendor/pdfjs/wasm/qcms_bg.wasm" \
  "/vendor/pdfjs/wasm/jbig2.wasm" \
  "/vendor/pdfjs/iccs/CGATS001Compat-v2-micro.icc"; do
  CODE=$(curl -s -o "$DIR/body" -w "%{http_code}" "http://127.0.0.1:$PORT$path")
  SIZE=$(wc -c < "$DIR/body" | tr -d ' ')
  printf "%-58s HTTP %s  %s bytes\n" "$path" "$CODE" "$SIZE"
  if [ "$CODE" != "200" ] || [ "$SIZE" = "0" ]; then FAIL=1; fi
done

# 字节一致性：站点服务的 CMap 与 node_modules 源文件逐字节比较
NODE_MODULE_FILE="apps/web/node_modules/pdfjs-dist/cmaps/UniGB-UCS2-H.bcmap"
curl -s "http://127.0.0.1:$PORT/vendor/pdfjs/cmaps/UniGB-UCS2-H.bcmap" -o "$DIR/served.bcmap"
SERVED_SHA=$(shasum -a 256 "$DIR/served.bcmap" | awk '{print $1}')
SOURCE_SHA=$(shasum -a 256 "$NODE_MODULE_FILE" | awk '{print $1}')
echo "served sha256 = ${SERVED_SHA}"
echo "source sha256 = ${SOURCE_SHA}"
if [ "${SERVED_SHA}" != "${SOURCE_SHA}" ]; then FAIL=1; fi

# 内嵌 UI 的 CSS 必须带 BUG-002 修复规则（.notices 容器不拦截指针 + 顶栏之下定位）
CSS_PATH=$(curl -s "http://127.0.0.1:$PORT/" | grep -o '/assets/[^"]*\.css' | head -1)
echo "内嵌 CSS: ${CSS_PATH}"
curl -s "http://127.0.0.1:$PORT${CSS_PATH}" -o "$DIR/app.css"
grep -q 'pointer-events:none' "$DIR/app.css" && echo "CSS 含 pointer-events:none" || { echo "CSS 缺 pointer-events:none"; FAIL=1; }
grep -q -- '--notices-top' "$DIR/app.css" && echo "CSS 含 --notices-top" || { echo "CSS 缺 --notices-top"; FAIL=1; }

# 未知 API 仍是 JSON 404（SPA fallback 不吞 API）
CODE=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:$PORT/api/v1/qa-unknown")
echo "未知 API 状态码：${CODE}（期望 404）"
[ "$CODE" = "404" ] || FAIL=1

if [ "${FAIL}" = "0" ]; then
  echo "QA 内嵌 vendor 复核：全部通过"
else
  echo "QA 内嵌 vendor 复核：存在失败项"; exit 1
fi
