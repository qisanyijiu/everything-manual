#!/usr/bin/env bash
# Linux x86_64 (musl) 原生构建 + 正式包冒烟编排脚本（T22）。
#
# 用法（在仓库根或任意目录执行）：
#   scripts/linux-musl.sh [--image IMAGE] [--build-dir DIR] [--cache-dir DIR]
#                         [--arch amd64|arm64] [--keep]
#
# 行为：
#   1. 把工作树（排除 target/node_modules/dist/.git/*.log）复制到 BUILD_DIR，宿主工作树不被容器改写；
#      `*.log` 也排除：上一轮失败留下的容器日志（如 xtask-check.log）会被 `--delete` 清掉，
#      而它们正是排查证据；仓库本身不跟踪任何非 artifacts 的 .log（已核对）。
#   2. 在 Linux 容器内原生执行 `cargo xtask dist --target x86_64-unknown-linux-musl`
#      与 `cargo xtask smoke --binary <abs>`（validation-release §7 的 7 步）；
#   3. 把 dist 产物与日志拷回宿主 `artifacts/web-mvp/t22-rd/linux/`。
#
# 离线证据（§7 第 5 步）在容器内单独执行：
#   scripts/linux-musl.sh --offline-only        # docker run --network none，跑整条 smoke
#   日志与产物落在 `artifacts/web-mvp/t22-rd/linux/offline/`（不覆盖在线轮次的证据）。
#
# 样例备份（smoke §7 第 2 步的输入）：仓库那份的 `database/manual.sqlite3` 被 .gitignore
# 排除（`*.sqlite3`），全新 checkout 上不完整；用下面这条在容器内用仓库自己的 T20 造数
# 用例 + 正式二进制 `backup` 重新生成一份自洽备份（落在 BUILD_DIR，不回写仓库）：
#   scripts/linux-musl.sh --prepare-sample-backup
# 已有完整备份时也可用 EM_T20_SAMPLE_BACKUP=<目录> 指定。
#
# Linux 侧回归（改了 xtask / 产品代码后跑；与本地/CI 同一入口）：
#   scripts/linux-musl.sh --check      # 容器内 cargo xtask check + smoke-bootstrap
#   日志落在 `artifacts/web-mvp/t22-rd/linux/regression/`。
#
# 目录约定（改这里前先读 xtask/src/dist.rs 的隔离扫描）：
#   - 工作树挂到容器内 **/src/everything-manual**（EM_LINUX_WORK_MOUNT 可覆盖）。这个挂载点
#     不能随便取短名：隔离扫描把"仓库根"当**子串**在二进制里搜，实测
#     ① `/build` 与 remap 目标 `/build/home`、`/build/repo` 自撞；
#     ② `/work` 与依赖 panic 路径里极常见的 `.../worker.rs` 自撞（sqlx/tokio 各若干处）。
#     两个容器挂载名都在真实构建里触发过误报，故改用带项目名的深路径。
#   - CARGO_HOME 固定为容器内 `$HOME/.cargo`（HOME=/root），即与 RUSTUP_HOME 一致的 rustup 安装对；
#     且必须在 $HOME 之下，registry/src 的 panic 路径才会被 $HOME 那条 remap 一并归一化并被
#     隔离扫描拒绝（见 ADR-035；CARGO_HOME 在 $HOME 之外＝脏路径既不归一化也不被拒绝）。
#   - 两个缓存目录（cargo/rustup）由宿主持久化并挂载，供 `--offline-only` 轮次在无网络下复用；
#     首次运行从镜像 /usr/local/{cargo,rustup} 播种（不下载工具链）。
#
# 备注：
#   - Apple Silicon 宿主用 --platform linux/amd64（Rosetta/QEMU 执行 x86_64），
#     这正是"Linux x86_64 二进制真实运行"的证据；宿主为 x86_64 Linux 时无需 --platform。
#   - 镜像需自带或可安装：rustup 工具链（rust-toolchain.toml = 1.98.1）、Node 22、musl-tools、file。
#   - 下载来源默认走实测可达的镜像（deb.debian.org / static.rust-lang.org / registry.npmjs.org
#     在本环境不可达或极慢），可用 EM_LINUX_* / RUSTUP_DIST_SERVER / npm_config_registry 覆盖；
#     包内容由 rustup sha256 清单、npm integrity、Cargo.lock checksum 校验，只换传输来源。
#   - 不改宿主 node_modules（容器内是复制出来的独立目录）。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${EM_LINUX_IMAGE:-docker.1ms.run/library/rust:1.98.1-bookworm}"
BUILD_DIR="${EM_LINUX_BUILD_DIR:-/tmp/em-linux-musl-build}"
CACHE_DIR="${EM_LINUX_CACHE_DIR:-/tmp/em-linux-musl-cache}"
WORK_MOUNT="${EM_LINUX_WORK_MOUNT:-/src/everything-manual}"
ARTIFACT_DIR="$REPO_ROOT/artifacts/web-mvp/t22-rd/linux"
PLATFORM_FLAG=()
OFFLINE_ONLY=0
PREPARE_SAMPLE_BACKUP=0
RUN_CHECK=0
REPRODUCIBLE=0
KEEP=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --image) IMAGE="$2"; shift 2 ;;
    --build-dir) BUILD_DIR="$2"; shift 2 ;;
    --cache-dir) CACHE_DIR="$2"; shift 2 ;;
    --arch) PLATFORM_FLAG=("--platform" "linux/$2"); shift 2 ;;
    --offline-only) OFFLINE_ONLY=1; shift ;;
    --prepare-sample-backup) PREPARE_SAMPLE_BACKUP=1; shift ;;
    --check) RUN_CHECK=1; shift ;;
    --check-reproducible) REPRODUCIBLE=1; shift ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,48p' "$0"; exit 0 ;;
    *) echo "未知参数：$1" >&2; exit 2 ;;
  esac
done

if [[ ${#PLATFORM_FLAG[@]} -eq 0 ]]; then
  # Apple Silicon 宿主默认走 amd64 容器（目标平台是 x86_64）。
  if [[ "$(uname -m)" == "arm64" && "$(uname -s)" == "Darwin" ]]; then
    PLATFORM_FLAG=("--platform" "linux/amd64")
  fi
fi
# bash 3.2（macOS /bin/bash）+ set -u：空数组展开会报 unbound variable。
PLATFORM_ARGS=(${PLATFORM_FLAG[@]+"${PLATFORM_FLAG[@]}"})

STEPS_LOG="$BUILD_DIR/host-steps.log"
mkdir -p "$BUILD_DIR"
: >"$STEPS_LOG"

step() {
  local name="$1"
  shift
  local start=$SECONDS
  local rc=0
  set +e
  "$@"
  rc=$?
  set -e
  printf '[host-step] %-20s exit=%s elapsed=%ss\n' "$name" "$rc" "$((SECONDS - start))" | tee -a "$STEPS_LOG"
  return "$rc"
}

echo "== 准备构建目录：$BUILD_DIR =="
step rsync-worktree rsync -a --delete \
  --exclude 'target/' --exclude 'node_modules/' --exclude 'apps/web/dist/' \
  --exclude '.git/' --exclude '.claude/' --exclude '.codex/' \
  --exclude 'artifacts/' --exclude 'dist/' \
  --exclude '*.log' \
  "$REPO_ROOT/" "$BUILD_DIR/"
# 冒烟需要 T20 合法样例备份（仓库 artifacts 下），单独复制。
# 注意：`.gitignore` 排除 `*.sqlite3`，所以全新 checkout 的样例备份**缺少**
# database/manual.sqlite3（manifest/SHA256SUMS 在版本控制里、DB 不在），此时不能拿仓库那份
# 覆盖构建目录里已重新生成的完整备份（否则 manifest 与 DB 对不上，restore 校验必失败）。
SAMPLE_SRC="${EM_T20_SAMPLE_BACKUP:-$REPO_ROOT/artifacts/web-mvp/t20-rd/sample-backup}"
SAMPLE_DST="$BUILD_DIR/artifacts/web-mvp/t20-rd/sample-backup"
mkdir -p "$SAMPLE_DST"
if [[ -f "$SAMPLE_SRC/database/manual.sqlite3" ]]; then
  step rsync-sample-backup rsync -a --delete "$SAMPLE_SRC/" "$SAMPLE_DST/"
elif [[ -f "$SAMPLE_DST/database/manual.sqlite3" ]]; then
  echo "   仓库样例备份缺 database/manual.sqlite3（*.sqlite3 不入库）：沿用构建目录内已生成的完整备份"
  echo "   （重新生成：scripts/linux-musl.sh --prepare-sample-backup，或设 EM_T20_SAMPLE_BACKUP=…）"
else
  echo "   警告：样例备份不完整且构建目录里也没有可用的完整备份，smoke 第 2 步会失败" >&2
  step rsync-sample-backup rsync -a "$SAMPLE_SRC/" "$SAMPLE_DST/"
fi

# ---------------------------------------------------------------------------
# 挂载点变更保护：target/ 里的 build script 二进制烧进了编译时的 CARGO_MANIFEST_DIR
# （`cargo:rerun-if-changed=<旧路径>/migrations`），cargo 的指纹只看源码 mtime，
# 换挂载点后会**复用旧 build script**，导致 build.rs 按旧路径找不到 apps/web/dist 而 panic。
# ---------------------------------------------------------------------------
mkdir -p "$CACHE_DIR"
MARKER="$CACHE_DIR/work-mount.txt"
if [[ -f "$MARKER" && "$(cat "$MARKER")" != "$WORK_MOUNT" ]]; then
  echo "== 挂载点由 $(cat "$MARKER") 变为 ${WORK_MOUNT}：清理 target/ 与 dist/，避免复用旧构建指纹 =="
  rm -rf "$BUILD_DIR/target" "$BUILD_DIR/dist"
fi
printf '%s' "$WORK_MOUNT" >"$MARKER"

# ---------------------------------------------------------------------------
# 缓存播种：让两次运行（在线构建 + 离线 smoke）共享同一套 cargo/rustup 缓存
# ---------------------------------------------------------------------------
echo "== 缓存目录：$CACHE_DIR =="
mkdir -p "$CACHE_DIR/cargo" "$CACHE_DIR/rustup"
if [[ ! -e "$CACHE_DIR/cargo/bin/rustup" ]]; then
  echo "   播种 cargo 缓存（镜像 /usr/local/cargo：rustup 代理；CARGO_HOME 与 RUSTUP_HOME 必须配对）"
  docker run --rm ${PLATFORM_ARGS[@]+"${PLATFORM_ARGS[@]}"} -v "$CACHE_DIR/cargo":/dst \
    --entrypoint bash "$IMAGE" -c 'cp -a /usr/local/cargo/. /dst/'
fi
if [[ ! -d "$CACHE_DIR/rustup/toolchains" ]]; then
  echo "   播种 rustup 缓存（镜像 /usr/local/rustup：固定 1.98.1 工具链，避免重新下载）"
  docker run --rm ${PLATFORM_ARGS[@]+"${PLATFORM_ARGS[@]}"} -v "$CACHE_DIR/rustup":/dst \
    --entrypoint bash "$IMAGE" -c 'cp -a /usr/local/rustup/. /dst/'
fi
if [[ ! -d "$CACHE_DIR/cargo/registry" && -d "$HOME/.cargo/registry" ]]; then
  echo "   复用宿主 ~/.cargo/registry（crate 源码包，与平台无关）"
  rsync -a "$HOME/.cargo/registry/" "$CACHE_DIR/cargo/registry/"
fi

CONTAINER_ENV=(
  -e HOME=/root
  -e CARGO_HOME=/root/.cargo
  -e RUSTUP_HOME=/usr/local/rustup
  -e PATH=/usr/local/cargo/bin:/root/.cargo/bin:/usr/local/bin:/usr/bin:/bin
  # Docker Desktop 默认 VM 内存有限（本机 8 GiB / 10 vCPU）：并发 rustc 过多会触发 OOM，
  # 这里显式限并发（可用 EM_LINUX_CARGO_JOBS 覆盖）。
  -e CARGO_BUILD_JOBS="${EM_LINUX_CARGO_JOBS:-8}"
)
CONTAINER_MOUNTS=(
  -v "$BUILD_DIR":"$WORK_MOUNT"
  -v "$CACHE_DIR/cargo":/root/.cargo
  -v "$CACHE_DIR/rustup":/usr/local/rustup
)

if [[ "$RUN_CHECK" -eq 1 ]]; then
  echo "== Linux 侧回归：cargo xtask check + smoke-bootstrap（容器内原生执行）=="
  # 失败也要回收日志：回归失败时日志本身就是证据（不能因 set -e 提前退出而丢掉）。
  RUN_RC=0
  step linux-regression docker run --rm ${PLATFORM_ARGS[@]+"${PLATFORM_ARGS[@]}"} \
    "${CONTAINER_MOUNTS[@]}" "${CONTAINER_ENV[@]}" \
    -e EM_WORK_DIR="$WORK_MOUNT" \
    -e EM_LINUX_REPRODUCIBLE="$REPRODUCIBLE" \
    -w "$WORK_MOUNT" "$IMAGE" bash "$WORK_MOUNT/scripts/container-linux-musl.sh" --check \
    || RUN_RC=$?
  mkdir -p "$ARTIFACT_DIR/regression"
  for log in xtask-check.log smoke-bootstrap.log steps.log; do
    cp -f "$BUILD_DIR/$log" "$ARTIFACT_DIR/regression/" 2>/dev/null || true
  done
  cp -f "$STEPS_LOG" "$ARTIFACT_DIR/regression/host-steps.log" 2>/dev/null || true
  if [[ "$RUN_RC" -ne 0 ]]; then
    echo "== 回归未全部通过（退出码 ${RUN_RC}）：日志已回收到 $ARTIFACT_DIR/regression/ ==" >&2
    exit "$RUN_RC"
  fi
  echo "== 完成：回归日志在 $ARTIFACT_DIR/regression/ =="
  exit 0
fi

if [[ "$PREPARE_SAMPLE_BACKUP" -eq 1 ]]; then
  echo "== 重新生成 T20 样例备份（容器内：造数用例 + 正式二进制 backup）=="
  step prepare-sample-backup docker run --rm ${PLATFORM_ARGS[@]+"${PLATFORM_ARGS[@]}"} \
    "${CONTAINER_MOUNTS[@]}" "${CONTAINER_ENV[@]}" \
    -e EM_WORK_DIR="$WORK_MOUNT" \
    -e EM_LINUX_REPRODUCIBLE="$REPRODUCIBLE" \
    -w "$WORK_MOUNT" "$IMAGE" bash "$WORK_MOUNT/scripts/container-linux-musl.sh" \
      --prepare-sample-backup
  echo "== 生成结果 =="
  ls -la "$BUILD_DIR/artifacts/web-mvp/t20-rd/sample-backup/"
  echo "（构建目录内的样例备份已可被 smoke 使用；如需回填仓库，见 implementation 记录）"
  exit 0
fi

if [[ "$OFFLINE_ONLY" -eq 1 ]]; then
  echo "== 离线模式：--network none 下运行整条 smoke（§7 第 5 步证据）=="
  RUN_RC=0
  step offline-smoke docker run --rm ${PLATFORM_ARGS[@]+"${PLATFORM_ARGS[@]}"} --network none \
    "${CONTAINER_MOUNTS[@]}" "${CONTAINER_ENV[@]}" \
    -e EM_WORK_DIR="$WORK_MOUNT" \
    -e EM_LINUX_REPRODUCIBLE="$REPRODUCIBLE" \
    -w "$WORK_MOUNT" "$IMAGE" bash "$WORK_MOUNT/scripts/container-linux-musl.sh" \
      --skip-dist --offline-only \
    || RUN_RC=$?

  echo "== 回收离线证据到 $ARTIFACT_DIR/offline/ =="
  mkdir -p "$ARTIFACT_DIR/offline"
  cp -f "$BUILD_DIR/smoke-console.log" "$ARTIFACT_DIR/offline/offline-smoke-console.log" 2>/dev/null || true
  cp -f "$BUILD_DIR"/network-none-*.log "$ARTIFACT_DIR/offline/" 2>/dev/null || true
  cp -f "$BUILD_DIR/steps.log" "$ARTIFACT_DIR/offline/offline-steps.log" 2>/dev/null || true
  cp -f "$BUILD_DIR/host-steps.log" "$ARTIFACT_DIR/offline/offline-host-steps.log" 2>/dev/null || true
  if [[ "$RUN_RC" -ne 0 ]]; then
    echo "== 离线 smoke 未通过（退出码 ${RUN_RC}）：日志已回收到 $ARTIFACT_DIR/offline/ ==" >&2
    exit "$RUN_RC"
  fi
  echo "== 完成 =="
  exit 0
fi

echo "== 容器内原生构建 + 冒烟（image=$IMAGE ${PLATFORM_ARGS[*]:-host}）=="
RUN_RC=0
step docker-build-and-smoke docker run --rm ${PLATFORM_ARGS[@]+"${PLATFORM_ARGS[@]}"} \
  "${CONTAINER_MOUNTS[@]}" "${CONTAINER_ENV[@]}" \
  -e EM_WORK_DIR="$WORK_MOUNT" \
  -e EM_LINUX_REPRODUCIBLE="$REPRODUCIBLE" \
  -w "$WORK_MOUNT" "$IMAGE" bash "$WORK_MOUNT/scripts/container-linux-musl.sh" \
  || RUN_RC=$?

echo "== 回收产物 =="
mkdir -p "$ARTIFACT_DIR"
rsync -a "$BUILD_DIR/dist/x86_64-unknown-linux-musl/" \
         "$ARTIFACT_DIR/dist-x86_64-unknown-linux-musl/"
for log in build-dist.log build-file-ldd.log smoke-console.log steps.log binary-hash.log \
           env-report.log rustup-toolchain.log rustup-target.log \
           t20-rehearsal-data.log t20-sample-backup.log; do
  cp -f "$BUILD_DIR/$log" "$ARTIFACT_DIR/" 2>/dev/null || true
done
cp -f "$STEPS_LOG" "$ARTIFACT_DIR/host-steps.log" 2>/dev/null || true
if [[ "$KEEP" -eq 0 ]]; then
  echo "（保留构建目录以便复核：${BUILD_DIR}）"
fi
if [[ "$RUN_RC" -ne 0 ]]; then
  echo "== 构建/冒烟未通过（退出码 ${RUN_RC}）：产物与日志已回收到 $ARTIFACT_DIR/ ==" >&2
  exit "$RUN_RC"
fi
echo "== 完成：产物在 $ARTIFACT_DIR/ =="
