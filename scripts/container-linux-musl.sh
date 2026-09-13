#!/usr/bin/env bash
# 容器内步骤（由 scripts/linux-musl.sh 调用；也可单独在 Linux 机器上执行）。
#
# 前置：已在容器/Linux 机器上挂载工作树到 ${EM_WORK_DIR}（工作目录 = 该挂载点）。
# 本脚本不隐式调用收费 API、不推送 Git、不发布公网内容。
#
# 目录约定（与 scripts/linux-musl.sh 一致，见该脚本头部说明）：
#   $EM_WORK_DIR     工作树挂载点，默认 /src/everything-manual（不能用 /build 或 /work：
#                    后者会与 dist 隔离扫描的"仓库根子串"自撞，见宿主脚本注释）
#   $HOME/.cargo     CARGO_HOME，必须位于 $HOME 之下 —— xtask dist 的隔离扫描把
#                    registry/src 的 panic 路径按 $HOME 归一化为 /build/home；
#                    CARGO_HOME 若在 $HOME 之外，这些构建机路径既不会被归一化、
#                    也不会被扫描拒绝（运行包会带脏路径）。本脚本用 HOME=/root。
#   $RUSTUP_HOME     默认 /usr/local/rustup（镜像自带固定工具链，由宿主缓存目录挂载持久化）
#
# 传输镜像（可用环境变量覆盖；只改下载来源，不改内容：rustup 校验 sha256 清单、
# npm ci 校验 package-lock 的 integrity、cargo 校验 Cargo.lock 的 checksum；Cargo.lock
# 的 source 仍是官方 crates.io，见 ADR-010）：
#   RUSTUP_DIST_SERVER / CARGO_SOURCE_CRATES_IO_REPLACE_WITH / npm_config_registry
#   宿主侧脚本默认使用国内可达镜像（本环境 static.rust-lang.org、deb.debian.org、
#   registry.npmjs.org 实测不可达或极慢）。
#
# 用法：
#   bash scripts/container-linux-musl.sh [--skip-dist] [--offline-only]
#     --skip-dist     跳过 dist（复用已产出的 dist/<triple>/everything-manual）
#     --offline-only  不下载任何依赖（apt/node/rustup 全部跳过），供
#                     `docker run --network none` 下跑整条 smoke 采集 Linux 离线证据。
set -euo pipefail

WORK_DIR="${EM_WORK_DIR:-/src/everything-manual}"
SKIP_DIST=0
OFFLINE_ONLY=0
PREPARE_SAMPLE_BACKUP=0
RUN_CHECK=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-dist) SKIP_DIST=1; shift ;;
    --offline-only) OFFLINE_ONLY=1; shift ;;
    --prepare-sample-backup) PREPARE_SAMPLE_BACKUP=1; SKIP_DIST=1; shift ;;
    --check) RUN_CHECK=1; SKIP_DIST=1; shift ;;
    *) echo "未知参数：$1" >&2; exit 2 ;;
  esac
done

log() { echo "[container] $*"; }

STEPS_LOG="$WORK_DIR/steps.log"
: >"$STEPS_LOG"

# 逐步执行并记录退出码与耗时（每步日志单独落盘 <工作树>/<name>.log，逐次运行覆盖，由宿主脚本回收）。
step() {
  local name="$1"
  shift
  local start=$SECONDS
  local rc=0
  set +e
  "$@" 2>&1 | tee "$WORK_DIR/$name.log"
  rc=${PIPESTATUS[0]}
  set -e
  printf '[step] %-22s exit=%s elapsed=%ss\n' "$name" "$rc" "$((SECONDS - start))" | tee -a "$STEPS_LOG"
  return "$rc"
}

# ---------------------------------------------------------------------------
# 0. 传输镜像与网络参数（可用环境变量覆盖，默认值针对本环境实测可达的镜像）
# ---------------------------------------------------------------------------
export RUSTUP_DIST_SERVER="${RUSTUP_DIST_SERVER:-https://rsproxy.cn}"
export RUSTUP_UPDATE_ROOT="${RUSTUP_UPDATE_ROOT:-https://rsproxy.cn/rustup}"
export CARGO_SOURCE_CRATES_IO_REPLACE_WITH="${CARGO_SOURCE_CRATES_IO_REPLACE_WITH:-rsproxy}"
export CARGO_SOURCE_RSPROXY_REGISTRY="${CARGO_SOURCE_RSPROXY_REGISTRY:-sparse+https://rsproxy.cn/index/}"
export npm_config_registry="${npm_config_registry:-https://registry.npmmirror.com}"
export npm_config_audit="${npm_config_audit:-false}"
export npm_config_fund="${npm_config_fund:-false}"
# 远端 crates 索引在本环境可能很慢：允许调用方用 CARGO_SOURCE_* 覆盖（见 ADR-010）。
export CARGO_NET_RETRY="${CARGO_NET_RETRY:-5}"
export DEBIAN_FRONTEND=noninteractive

log "CARGO_HOME=${CARGO_HOME:-<未设置>} RUSTUP_HOME=${RUSTUP_HOME:-<未设置>} HOME=$HOME"
log "镜像：rustup=$RUSTUP_DIST_SERVER npm=$npm_config_registry crates=$CARGO_SOURCE_RSPROXY_REGISTRY"

# ---------------------------------------------------------------------------
# 1. 依赖：musl 工具链 + cmake + file/binutils + Node（缺什么装什么；离线模式跳过）
# ---------------------------------------------------------------------------
if [[ "$OFFLINE_ONLY" -eq 1 ]]; then
  log "离线模式：跳过 apt/node/rustup 等一切下载"
  for tool in cargo rustc file readelf; do
    command -v "$tool" >/dev/null 2>&1 || {
      log "离线模式缺少必需工具：$tool"
      exit 1
    }
  done
  # 断网自证（不靠调用方口头声明 `--network none`）：在**本命名空间内**外连必须失败，
  # 随后整条 smoke 都在同一命名空间里跑。
  run_network_none_probe() {
    echo "== 内核路由表 /proc/net/route（表头之外无条目＝无默认路由）=="
    cat /proc/net/route
    echo "== /etc/resolv.conf =="
    cat /etc/resolv.conf 2>/dev/null || echo "（无）"
    echo "== 尝试 DNS 解析与 HTTPS 外连（应失败）=="
    getent hosts registry.npmjs.org 2>&1 | head -2 || true
    curl -sS -m 8 -o /dev/null -w "http_code=%{http_code}\n" https://static.crates.io/ 2>&1 | head -3 || true
    echo "（以上失败即为断网证据；后续 smoke 在同一容器内运行）"
  }
  step network-none-probe run_network_none_probe
else
  if [[ "$(id -u)" -eq 0 ]]; then
    SUDO=""
  else
    SUDO="sudo"
  fi

  if command -v apt-get >/dev/null 2>&1 && ! command -v musl-gcc >/dev/null 2>&1; then
    # 镜像源走 http://deb.debian.org，在本环境实测 503/未签名（宿主透明代理 fake-IP 拦截）；
    # 换成实测可达的 https 镜像。注意 debian 与 debian-security 是两个不同路径，
    # 不能用同一个 URI 前缀随便替换。
    APT_MIRROR="${EM_LINUX_APT_MIRROR:-https://mirrors.ustc.edu.cn}"
    if [[ -f /etc/apt/sources.list.d/debian.sources || -f /etc/apt/sources.list ]]; then
      $SUDO rm -f /etc/apt/sources.list.d/debian.sources /etc/apt/sources.list
      $SUDO tee /etc/apt/sources.list.d/em-linux-mirror.sources >/dev/null <<EOF
Types: deb
URIs: ${APT_MIRROR}/debian
Suites: bookworm bookworm-updates
Components: main
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg

Types: deb
URIs: ${APT_MIRROR}/debian-security
Suites: bookworm-security
Components: main
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg
EOF
    fi
    log "安装 musl-tools / cmake / file / binutils（apt 镜像 ${APT_MIRROR}）"
    $SUDO apt-get update -qq
    $SUDO apt-get install -y -qq --no-install-recommends \
      musl-tools file binutils ca-certificates curl xz-utils build-essential cmake >/dev/null
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    log "安装 rustup（工具链由 rust-toolchain.toml 固定为 1.98.1）"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain none --no-modify-path
  fi

  if ! command -v node >/dev/null 2>&1; then
    log "安装 Node 22（构建期依赖；运行期不需要）"
    # v22.22.2 而非 v22.20.0：apps/web 的 jsdom@30 声明 engines node ^22.22.2（v22.20.0
    # 会触发 npm EBADENGINE 警告，属于"工具链与锁文件声明不符"的隐患）。
    NODE_VERSION="${EM_LINUX_NODE_VERSION:-v22.22.2}"
    NODE_MIRROR="${EM_LINUX_NODE_MIRROR:-https://registry.npmmirror.com/-/binary/node}"
    if ! curl -fsSL -m 300 "${NODE_MIRROR}/${NODE_VERSION}/node-${NODE_VERSION}-linux-x64.tar.xz" -o /tmp/node.tar.xz; then
      log "Node 镜像不可达，回退 nodejs.org"
      curl -fsSL -m 600 "https://nodejs.org/dist/${NODE_VERSION}/node-${NODE_VERSION}-linux-x64.tar.xz" -o /tmp/node.tar.xz
    fi
    $SUDO mkdir -p /usr/local/lib/nodejs
    $SUDO tar -xJf /tmp/node.tar.xz -C /usr/local/lib/nodejs
    $SUDO ln -sf "/usr/local/lib/nodejs/node-${NODE_VERSION}-linux-x64/bin/node" /usr/local/bin/node
    $SUDO ln -sf "/usr/local/lib/nodejs/node-${NODE_VERSION}-linux-x64/bin/npm" /usr/local/bin/npm
    rm -f /tmp/node.tar.xz
  fi

  log "rustc: $(rustc -V 2>/dev/null || echo '未安装（rustup 将按 rust-toolchain.toml 获取）')"
  # rust-toolchain.toml 声明了 rustfmt/clippy：缺失时 rustup 会在每次调用时尝试联网补齐
  # （离线容器里会直接失败），因此这里显式安装到位。
  step rustup-toolchain rustup toolchain install 1.98.1 --profile minimal --component rustfmt,clippy
  step rustup-target rustup target add x86_64-unknown-linux-musl
  log "node: $(node --version)；npm: $(npm --version)；cmake: $(cmake --version | head -1)；musl-gcc: $(musl-gcc --version | head -1)"
fi

# ---------------------------------------------------------------------------
# 2. 构建 + 冒烟
# ---------------------------------------------------------------------------
# 运行环境证据（证明"Linux 原生运行"，不是宿主 macOS 结果）。
run_env_report() {
  echo "uname: $(uname -a)"
  echo "os: $(head -2 /etc/os-release | tr '\n' ' ')"
  echo "arch: $(uname -m)"
  echo "rustc: $(rustc -vV | tr '\n' ' ')"
  echo "cargo: $(cargo -V)"
  echo "rustup: $(rustup -V)"
  echo "node: $(node --version 2>/dev/null || echo '未安装')"
  echo "npm: $(npm --version 2>/dev/null || echo '未安装')"
  echo "cc: $(cc --version 2>/dev/null | head -1 || echo '未安装')"
  echo "musl-gcc: $(musl-gcc --version 2>/dev/null | head -1 || echo '未安装')"
  echo "cmake: $(cmake --version 2>/dev/null | head -1 || echo '未安装')"
  echo "file: $(file --version 2>/dev/null | head -1)"
  echo "ldd: $(ldd --version 2>&1 | head -1)"
  echo "readelf: $(readelf --version | head -1)"
  echo "cpu: $(nproc) vCPU；mem: $(awk '/MemTotal/{print $2/1024/1024" GiB"}' /proc/meminfo)"
}
step env-report run_env_report

if [[ "$SKIP_DIST" -eq 0 ]]; then
  log "cargo fetch --locked（预取锁文件全部 crate，供离线轮次复用）"
  step cargo-fetch cargo fetch --locked
  DIST_ARGS=()
  # 可复现证据（独立重链接后比对 sha256）由宿主脚本 --check-reproducible 传入。
  if [[ "${EM_LINUX_REPRODUCIBLE:-0}" == "1" ]]; then
    DIST_ARGS+=(--check-reproducible)
  fi
  log "cargo xtask dist --target x86_64-unknown-linux-musl ${DIST_ARGS[*]:-}"
  step build-dist cargo xtask dist --target x86_64-unknown-linux-musl \
    ${DIST_ARGS[@]+"${DIST_ARGS[@]}"}
fi

BINARY="$WORK_DIR/dist/x86_64-unknown-linux-musl/everything-manual"
if [[ ! -f "$BINARY" ]]; then
  log "缺少二进制：${BINARY}（先跑 dist）"
  exit 1
fi

# ---------------------------------------------------------------------------
# 2b. 重新生成 T20 样例备份（仅 --prepare-sample-backup）
#     `.gitignore` 排除了 `*.sqlite3`，所以样例备份里的 database/manual.sqlite3 不在
#     版本控制内；全新 checkout 上 `cargo xtask smoke` 的 §7 第 2 步必然失败。
#     这里用仓库自己的 T20 造数用例 + 正式二进制的 backup 重新生成一份自洽备份。
# ---------------------------------------------------------------------------
if [[ "$PREPARE_SAMPLE_BACKUP" -eq 1 ]]; then
  REHEARSAL_DIR=/tmp/em-t20-rehearsal
  export EM_T20_REHEARSAL_DIR="$REHEARSAL_DIR"
  log "T20 造数：cargo test … prepare_rehearsal_datadir（EM_T20_REHEARSAL_DIR=${REHEARSAL_DIR}）"
  step t20-rehearsal-data cargo test -p everything-manual --test backup_restore -- \
    --ignored --nocapture prepare_rehearsal_datadir

  log "用正式二进制生成样例备份（backup --out 新路径）"
  rm -rf /tmp/em-t20-backup-out
  run_sample_backup() {
    "$BINARY" backup --data-dir "$REHEARSAL_DIR/data" --out /tmp/em-t20-backup-out
    ( cd /tmp/em-t20-backup-out && sha256sum -c SHA256SUMS )
  }
  step t20-sample-backup run_sample_backup

  SAMPLE_DIR="$WORK_DIR/artifacts/web-mvp/t20-rd/sample-backup"
  rm -rf "$SAMPLE_DIR"
  mkdir -p "$(dirname "$SAMPLE_DIR")"
  cp -a /tmp/em-t20-backup-out "$SAMPLE_DIR"
  log "样例备份已就绪：$SAMPLE_DIR"
  ls -la "$SAMPLE_DIR"
  log "完成（--prepare-sample-backup；逐步退出码与耗时见 ${STEPS_LOG}）"
  cat "$STEPS_LOG"
  exit 0
fi

# ---------------------------------------------------------------------------
# 2c. Linux 侧回归（仅 --check）：与本地/CI 同一入口 `cargo xtask check`，
#     外加 T01 最小链 `smoke-bootstrap`（验证 dist 产物本身可启动）。
# ---------------------------------------------------------------------------
if [[ "$RUN_CHECK" -eq 1 ]]; then
  # 先跑独立的 smoke-bootstrap：`cargo xtask check` 可能因某个测试失败而中断，
  # 先跑可保证这条最小链的结论总是被采集。
  log "回归：cargo xtask smoke-bootstrap --binary $BINARY"
  step smoke-bootstrap cargo xtask smoke-bootstrap --binary "$BINARY"
  log "回归：cargo xtask check（fmt / clippy / workspace 测试 / 前端 / 合同）"
  step xtask-check cargo xtask check
  log "完成（--check；逐步退出码与耗时见 $STEPS_LOG）"
  cat "$STEPS_LOG"
  exit 0
fi

log "file / ldd / readelf（静态链接证据）"
run_file_ldd() {
  file "$BINARY"
  echo "--- ldd ---"
  ldd "$BINARY" 2>&1 || true
  echo "--- readelf -d（全文）---"
  readelf -d "$BINARY" 2>&1 || true
  echo "--- DT_NEEDED 条目数（静态 musl 应为 0）---"
  readelf -d "$BINARY" 2>/dev/null | grep -c "(NEEDED)" || true
}
step build-file-ldd run_file_ldd

log "二进制指纹（sha256 与字节数，在 Linux 内计算）"
run_binary_hash() {
  sha256sum "$BINARY"
  stat -c "bytes=%s" "$BINARY"
}
step binary-hash run_binary_hash

log "cargo xtask smoke --binary ${BINARY}（§7 的 7 步）"
step smoke-console cargo xtask smoke --binary "$BINARY"

if [[ "$OFFLINE_ONLY" -eq 1 ]]; then
  log "离线模式（--network none）：以上 smoke 已在无网络命名空间内完成读取/恢复验证"
fi
log "完成（逐步退出码与耗时见 ${STEPS_LOG}）"
cat "$STEPS_LOG"
