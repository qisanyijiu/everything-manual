#!/usr/bin/env bash
# 容器内步骤（由 scripts/linux-musl.sh 调用；也可单独在 Linux 机器上执行）。
#
# 前置：已在容器/Linux 机器上挂载工作树到 /build（工作目录 = /build）。
# 本脚本不隐式调用收费 API、不推送 Git、不发布公网内容。
#
# 用法：
#   bash scripts/container-linux-musl.sh [--skip-dist] [--offline-only]
set -euo pipefail

SKIP_DIST=0
OFFLINE_ONLY=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-dist) SKIP_DIST=1; shift ;;
    --offline-only) OFFLINE_ONLY=1; shift ;;
    *) echo "未知参数：$1" >&2; exit 2 ;;
  esac
done

log() { echo "[container] $*"; }

# ---------------------------------------------------------------------------
# 1. 依赖：musl 工具链 + file/binutils + Node（缺什么装什么；只在缺失时联网）
# ---------------------------------------------------------------------------
if [[ "$(id -u)" -eq 0 ]]; then
  SUDO=""
else
  SUDO="sudo"
fi
export DEBIAN_FRONTEND=noninteractive

if command -v apt-get >/dev/null 2>&1 && ! command -v musl-gcc >/dev/null 2>&1; then
  log "安装 musl-tools / file / binutils / ca-certificates"
  $SUDO apt-get update -qq
  $SUDO apt-get install -y -qq --no-install-recommends \
    musl-tools file binutils ca-certificates curl xz-utils build-essential >/dev/null
fi

if ! command -v cargo >/dev/null 2>&1; then
  log "安装 rustup（工具链由 rust-toolchain.toml 固定为 1.98.1）"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain none --no-modify-path
fi

if ! command -v node >/dev/null 2>&1; then
  log "安装 Node 22（构建期依赖；运行期不需要）"
  NODE_VERSION=v22.20.0
  curl -fsSL "https://nodejs.org/dist/${NODE_VERSION}/node-${NODE_VERSION}-linux-x64.tar.xz" -o /tmp/node.tar.xz
  $SUDO mkdir -p /usr/local/lib/nodejs
  $SUDO tar -xJf /tmp/node.tar.xz -C /usr/local/lib/nodejs
  $SUDO ln -sf "/usr/local/lib/nodejs/node-${NODE_VERSION}-linux-x64/bin/node" /usr/local/bin/node
  $SUDO ln -sf "/usr/local/lib/nodejs/node-${NODE_VERSION}-linux-x64/bin/npm" /usr/local/bin/npm
  rm -f /tmp/node.tar.xz
fi

log "rustc: $(rustc -V 2>/dev/null || echo '未安装（rustup 将按 rust-toolchain.toml 获取）')"
rustup toolchain install 1.98.1 --profile minimal --component rustfmt,clippy 2>/dev/null || true
rustup target add x86_64-unknown-linux-musl
log "node: $(node --version)；npm: $(npm --version)"

# 远端 crates 索引在本环境可能很慢：允许调用方用 CARGO_SOURCE_* 覆盖（见 ADR-010）。
export CARGO_NET_RETRY="${CARGO_NET_RETRY:-5}"

# ---------------------------------------------------------------------------
# 2. 构建 + 冒烟
# ---------------------------------------------------------------------------
if [[ "$SKIP_DIST" -eq 0 ]]; then
  log "cargo xtask dist --target x86_64-unknown-linux-musl"
  cargo xtask dist --target x86_64-unknown-linux-musl 2>&1 | tee /build/build-dist.log
fi

BINARY=/build/dist/x86_64-unknown-linux-musl/everything-manual
if [[ ! -f "$BINARY" ]]; then
  log "缺少二进制：$BINARY（先跑 dist）"
  exit 1
fi

log "file / ldd（静态链接证据）"
file "$BINARY" | tee /build/build-file-ldd.log
ldd "$BINARY" 2>&1 | tee -a /build/build-file-ldd.log || true
readelf -d "$BINARY" 2>&1 | head -20 | tee -a /build/build-file-ldd.log || true

log "cargo xtask smoke --binary $BINARY（§7 的 7 步；本环境无外网时即离线运行）"
cargo xtask smoke --binary "$BINARY" 2>&1 | tee /build/smoke-console.log

if [[ "$OFFLINE_ONLY" -eq 1 ]]; then
  log "离线模式（--network none）：以上 smoke 已在无网络命名空间内完成读取/恢复验证"
fi
log "完成"
