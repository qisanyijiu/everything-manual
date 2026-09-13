#!/usr/bin/env bash
# T09 证据脚本：确认新增的 PDF.js vendor 资源随 release 二进制内嵌并能按静态路径服务。
# 用法：bash artifacts/web-mvp/t09-rd/check-embedded-vendor.sh
# 前置：repo 根已执行 npm --prefix apps/web run build 与 cargo xtask dist --target <本机 target>。
set -euo pipefail
SOURCE_BIN="${1:-$(pwd)/dist/aarch64-apple-darwin/everything-manual}"
PORT=18099
DIR="$(mktemp -d /tmp/em-t09-embed.XXXXXX)"
# 与 `xtask smoke-bootstrap` 同手法：先拷贝到全新临时目录再运行
# （macOS 对位于仓库 dist/ 下的可执行文件会以 SIGKILL 拦截直接执行；
#  拷贝后运行与本机实测一致，也更接近 T22 的"冷目录"语义）。
BIN="$DIR/everything-manual"
cp "$SOURCE_BIN" "$BIN"
printf 'embed-check-password\n' > "$DIR/pw.txt"
chmod 600 "$DIR/pw.txt"
"$BIN" init --data-dir "$DIR/data" --password-file "$DIR/pw.txt" >/dev/null
"$BIN" serve --data-dir "$DIR/data" --listen "127.0.0.1:$PORT" > "$DIR/server.log" 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null || true' EXIT
sleep 2
for path in \
  "/vendor/pdfjs/cmaps/UniGB-UCS2-H.bcmap" \
  "/vendor/pdfjs/standard_fonts/LiberationSans-Regular.ttf" \
  "/vendor/pdfjs/wasm/openjpeg.wasm" \
  "/vendor/pdfjs/wasm/qcms_bg.wasm" \
  "/vendor/pdfjs/iccs/CGATS001Compat-v2-micro.icc"; do
  printf "%-56s" "$path"
  curl -s -o /dev/null -w "HTTP %{http_code} %{size_download} bytes\n" "http://127.0.0.1:$PORT$path"
done
printf "%-56s" "/vendor/pdfjs/cmaps/UniGB-UCS2-H.bcmap 的 sha256（应与 node_modules 一致）"
curl -s "http://127.0.0.1:$PORT/vendor/pdfjs/cmaps/UniGB-UCS2-H.bcmap" | shasum -a 256
