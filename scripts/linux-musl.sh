#!/usr/bin/env bash
# Linux x86_64 (musl) 原生构建 + 正式包冒烟编排脚本（T22）。
#
# 用法（在仓库根或任意目录执行）：
#   scripts/linux-musl.sh [--image IMAGE] [--build-dir DIR] [--arch amd64|arm64] [--keep]
#
# 行为：
#   1. 把工作树（排除 target/node_modules/dist/.git）复制到 BUILD_DIR，宿主工作树不被容器改写；
#   2. 在 Linux 容器内原生执行 `cargo xtask dist --target x86_64-unknown-linux-musl`
#      与 `cargo xtask smoke --binary <abs>`（validation-release §7 的 7 步）；
#   3. 把 dist 产物与日志拷回宿主 `dist/` 与 `artifacts/web-mvp/t22-rd/linux/`。
#
# 离线证据（§7 第 5 步）在容器内单独执行：
#   scripts/linux-musl.sh --offline-only        # docker run --network none，跑整条 smoke
#
# 备注：
#   - Apple Silicon 宿主用 --platform linux/amd64（Rosetta/QEMU 执行 x86_64），
#     这正是"Linux x86_64 二进制真实运行"的证据；宿主为 x86_64 Linux 时无需 --platform。
#   - 镜像需自带或可安装：rustup 工具链（rust-toolchain.toml = 1.98.1）、Node 22、musl-tools、file。
#     推荐 `rust:1.98.1-bookworm` + 容器内经 NodeSource 装 Node，或 `node:22-bookworm` + rustup。
#   - 不改宿主 node_modules（容器内是复制出来的独立目录）。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${EM_LINUX_IMAGE:-docker.1ms.run/library/rust:1.98.1-bookworm}"
BUILD_DIR="${EM_LINUX_BUILD_DIR:-/tmp/em-linux-musl-build}"
PLATFORM_FLAG=()
OFFLINE_ONLY=0
KEEP=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --image) IMAGE="$2"; shift 2 ;;
    --build-dir) BUILD_DIR="$2"; shift 2 ;;
    --arch) PLATFORM_FLAG=("--platform" "linux/$2"); shift 2 ;;
    --offline-only) OFFLINE_ONLY=1; shift ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "未知参数：$1" >&2; exit 2 ;;
  esac
done

if [[ ${#PLATFORM_FLAG[@]} -eq 0 ]]; then
  # Apple Silicon 宿主默认走 amd64 容器（目标平台是 x86_64）。
  if [[ "$(uname -m)" == "arm64" && "$(uname -s)" == "Darwin" ]]; then
    PLATFORM_FLAG=("--platform" "linux/amd64")
  fi
fi

echo "== 准备构建目录：$BUILD_DIR =="
mkdir -p "$BUILD_DIR"
rsync -a --delete \
  --exclude 'target/' --exclude 'node_modules/' --exclude 'apps/web/dist/' \
  --exclude '.git/' --exclude '.claude/' --exclude '.codex/' \
  --exclude 'artifacts/' --exclude 'dist/' \
  "$REPO_ROOT/" "$BUILD_DIR/"
# 冒烟需要 T20 合法样例备份（仓库 artifacts 下），单独复制。
mkdir -p "$BUILD_DIR/artifacts/web-mvp/t20-rd"
rsync -a "$REPO_ROOT/artifacts/web-mvp/t20-rd/sample-backup/" \
         "$BUILD_DIR/artifacts/web-mvp/t20-rd/sample-backup/"

if [[ "$OFFLINE_ONLY" -eq 1 ]]; then
  echo "== 离线模式：--network none 下运行整条 smoke（§7 第 5 步证据）=="
  docker run --rm "${PLATFORM_FLAG[@]}" --network none \
    -v "$BUILD_DIR":/build -w /build \
    -e CARGO_HOME=/build/.cargo-home -e RUSTUP_HOME=/build/.rustup-home \
    -e PATH=/build/.cargo-home/bin:/usr/local/bin:/usr/bin:/bin \
    "$IMAGE" bash /build/scripts/container-linux-musl.sh \
      --skip-dist --offline-only
  exit 0
fi

echo "== 容器内原生构建 + 冒烟（image=$IMAGE ${PLATFORM_FLAG[*]:-host}）=="
docker run --rm "${PLATFORM_FLAG[@]}" \
  -v "$BUILD_DIR":/build -w /build \
  -e CARGO_HOME=/build/.cargo-home -e RUSTUP_HOME=/build/.rustup-home \
  -e PATH=/build/.cargo-home/bin:/usr/local/bin:/usr/bin:/bin \
  "$IMAGE" bash /build/scripts/container-linux-musl.sh

echo "== 回收产物 =="
mkdir -p "$REPO_ROOT/artifacts/web-mvp/t22-rd/linux"
rsync -a "$BUILD_DIR/dist/x86_64-unknown-linux-musl/" \
         "$REPO_ROOT/artifacts/web-mvp/t22-rd/linux/dist-x86_64-unknown-linux-musl/"
cp -f "$BUILD_DIR"/build-*.log "$REPO_ROOT/artifacts/web-mvp/t22-rd/linux/" 2>/dev/null || true
cp -f "$BUILD_DIR/smoke-console.log" "$REPO_ROOT/artifacts/web-mvp/t22-rd/linux/" 2>/dev/null || true
if [[ "$KEEP" -eq 0 ]]; then
  echo "（保留构建目录以便复核：$BUILD_DIR）"
fi
echo "== 完成：产物在 artifacts/web-mvp/t22-rd/linux/ =="
