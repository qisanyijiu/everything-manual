#!/usr/bin/env bash
# One entry point for the shared preview library, with an optional local source frontend.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
preview_root=${EM_PREVIEW_ROOT:-"$repo_root/var/preview"}
data_dir="$preview_root/data"
private_dir="$preview_root/private"
pid_file="$private_dir/server.pid"
mode_file="$private_dir/transport"
source_ui_marker="$private_dir/source-ui"
source_ui_pid_file="$private_dir/source-ui.pid"
source_ui_port_file="$private_dir/source-ui-port"
source_ui_target_file="$private_dir/source-ui-target"
web_dir="$repo_root/apps/web"
vite_entry="$web_dir/node_modules/vite/bin/vite.js"
launcher_lock="$private_dir/launcher.lock"
launcher_owner="$launcher_lock/owner.pid"
launcher_reclaim="$launcher_lock/reclaim.pid"
launcher_owned=0
launcher_stamp=
launcher_owner_stamp=
reclaim_stamp=
port=${EM_PREVIEW_PORT:-8080}
origin="http://127.0.0.1:$port"
mode=native
action=start
open_browser=1
ui_choice=auto
source_ui=0
ui_port=${EM_PREVIEW_UI_PORT:-5173}
if [[ $(uname -s) == Linux ]]; then mode=docker; fi

usage() {
  cat <<'EOF'
用法：bash scripts/start-project.sh [--native|--docker] [--source-ui|--embedded-ui] [--no-open] [--status|--stop]
默认打开统一资料库 var/preview/data（macOS 用内嵌前端程序，Linux 用 Docker）。
--docker   使用同一个资料库启动 Docker Compose
--native   使用本机发行程序
--source-ui 使用本机后端及当前源码前端，并记住此预览方式（需已安装前端依赖）
--embedded-ui 使用程序内嵌前端，并清除源码前端预览设置
--no-open  启动后只输出地址
--status   查看服务状态
--stop     停止本脚本启动的服务，保留资料与配置
环境：EM_PREVIEW_ROOT / EM_PREVIEW_PORT / EM_PREVIEW_UI_PORT / EM_PREVIEW_BINARY / EM_DOCKER_PLATFORM
EOF
}

for arg in "$@"; do
  case "$arg" in
    --native) mode=native ;;
    --docker) mode=docker ;;
    --source-ui) mode=native; ui_choice=source ;;
    --embedded-ui) ui_choice=embedded ;;
    --no-open) open_browser=0 ;;
    --status) action=status ;;
    --stop) action=stop ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知选项：$arg" >&2; usage >&2; exit 2 ;;
  esac
done
if [[ ! $port =~ ^[0-9]{1,5}$ ]] || ((10#$port < 1024 || 10#$port > 65535)); then
  echo 'EM_PREVIEW_PORT 必须为1024–65535之间的整数。' >&2; exit 2
fi
port=$((10#$port))
origin="http://127.0.0.1:$port"
if [[ $preview_root != /* || $preview_root == / || $preview_root == "$repo_root" || $preview_root == "$HOME" ]]; then
  echo 'EM_PREVIEW_ROOT 必须是独立数据目录的绝对路径。' >&2; exit 2
fi
if [[ -L $preview_root || -L $private_dir || -L $data_dir ]]; then echo '资料及私有配置目录不能是符号链接。' >&2; exit 2; fi
if [[ $action == start && $(id -u) == 0 ]]; then
  echo '请使用普通用户启动，以保持资料文件和容器用户的权限一致。' >&2; exit 2
fi
for ui_file in "$source_ui_marker" "$source_ui_pid_file" "$source_ui_port_file" "$source_ui_target_file" "$preview_root/logs/source-ui.log"; do
  if [[ -L $ui_file || ( -e $ui_file && ! -f $ui_file ) ]]; then
    echo '源码前端状态必须保存在普通文件中，不能是符号链接。' >&2; exit 2
  fi
done
select_source_ui() {
  source_ui=0
  if [[ $ui_choice == source || ( $ui_choice == auto && -f $source_ui_marker ) ]]; then
    if [[ $action == start && -f $source_ui_marker && $(cat "$source_ui_marker") != enabled ]]; then
      echo '源码前端预览设置无效，请检查 private/source-ui。' >&2; return 2
    fi
    source_ui=1
  fi
  if [[ $action == start && $mode == native && $source_ui == 1 ]]; then
    if [[ ! $ui_port =~ ^[0-9]{1,5}$ ]] || ((10#$ui_port < 1024 || 10#$ui_port > 65535)); then
      echo 'EM_PREVIEW_UI_PORT 必须为1024–65535之间的整数。' >&2; return 2
    fi
    ui_port=$((10#$ui_port))
    if [[ $ui_port == "$port" ]]; then echo '源码前端端口不能与后端端口相同。' >&2; return 2; fi
  fi
}
select_source_ui
umask 077

native_running() {
  [[ -f $pid_file ]] || return 1
  local pid command_line owner
  pid=$(cat "$pid_file")
  [[ $pid =~ ^[0-9]+$ ]] || return 1
  owner=$(ps -p "$pid" -o uid= 2>/dev/null | tr -d ' ') || return 1
  [[ $owner == "$(id -u)" ]] || return 1
  command_line=$(ps -p "$pid" -o command= 2>/dev/null) || return 1
  [[ $command_line == *' serve '* && $command_line == *"--data-dir $data_dir --listen "* ]]
}

source_ui_running() {
  [[ -f $source_ui_pid_file && -f $source_ui_port_file ]] || return 1
  local pid saved_port command_line owner
  pid=$(cat "$source_ui_pid_file")
  saved_port=$(cat "$source_ui_port_file")
  [[ $pid =~ ^[1-9][0-9]*$ && $saved_port =~ ^[0-9]{1,5}$ ]] || return 1
  ((10#$saved_port >= 1024 && 10#$saved_port <= 65535)) || return 1
  owner=$(ps -p "$pid" -o uid= 2>/dev/null | tr -d ' ') || return 1
  [[ $owner == "$(id -u)" ]] || return 1
  command_line=$(ps -p "$pid" -o command= 2>/dev/null) || return 1
  [[ $command_line == *" $vite_entry --host 127.0.0.1 --port $saved_port --strictPort" ]]
}

file_stamp() {
  if [[ $(uname -s) == Darwin ]]; then stat -f '%u:%Lp:%d:%i' "$1";
  else stat -c '%u:%a:%d:%i' "$1"; fi
}

release_reclaim() {
  if [[ -n $reclaim_stamp && ! -L $launcher_reclaim && -f $launcher_reclaim ]] &&
      [[ $(file_stamp "$launcher_reclaim" 2>/dev/null) == "$reclaim_stamp" ]]; then
    rm -f -- "$launcher_reclaim"
  fi
  reclaim_stamp=
}

release_launcher_lock() {
  release_reclaim
  if [[ $launcher_owned == 1 && ! -L $launcher_lock && -d $launcher_lock &&
        ! -L $launcher_owner && -f $launcher_owner ]] &&
      [[ $(file_stamp "$launcher_lock" 2>/dev/null) == "$launcher_stamp" &&
         $(file_stamp "$launcher_owner" 2>/dev/null) == "$launcher_owner_stamp" &&
         $(cat "$launcher_owner") == "$$" ]]; then
    rm -f -- "$launcher_owner"
    # Only an empty directory belonging to this invocation may be removed.
    rmdir "$launcher_lock" 2>/dev/null || true
  fi
}

pid_exists() {
  kill -0 "$1" 2>/dev/null || ps -p "$1" -o pid= >/dev/null 2>&1
}

acquire_launcher_lock() {
  local attempt owner owner_stamp directory_stamp path
  for attempt in $(seq 1 20); do
    if mkdir "$launcher_lock" 2>/dev/null; then
      launcher_stamp=$(file_stamp "$launcher_lock")
      (set -o noclobber; printf '%s\n' "$$" > "$launcher_owner")
      launcher_owner_stamp=$(file_stamp "$launcher_owner")
      launcher_owned=1
      return 0
    fi
    if [[ -L $launcher_lock || ! -d $launcher_lock ]]; then
      echo '启动锁不是普通目录；请检查 private/launcher.lock。' >&2; return 1
    fi
    directory_stamp=$(file_stamp "$launcher_lock")
    if [[ $directory_stamp != "$(id -u):700:"* ]]; then
      echo '启动锁的所有者或权限不符；不会删除它。' >&2; return 1
    fi
    if [[ ! -e $launcher_owner && ! -L $launcher_owner ]]; then
      # Another launcher may be between mkdir and writing its owner PID.
      sleep 0.1; continue
    fi
    if [[ -L $launcher_owner || ! -f $launcher_owner ]]; then
      echo '启动锁的 owner.pid 不是普通文件；不会删除它。' >&2; return 1
    fi
    owner_stamp=$(file_stamp "$launcher_owner")
    owner=$(cat "$launcher_owner")
    if [[ $owner_stamp != "$(id -u):600:"* || ! $owner =~ ^[1-9][0-9]*$ ]]; then
      echo '启动锁的 owner.pid 无效；不会删除它。' >&2; return 1
    fi
    if pid_exists "$owner"; then
      echo '另一个启动或停止操作正在进行，请稍后重试。' >&2; return 1
    fi
    # A hard link is an atomic, exclusive claim on this exact stale owner file.
    # Concurrent recovery cannot unlink an owner created by a newer launcher.
    if [[ -e $launcher_reclaim || -L $launcher_reclaim ]]; then
      if [[ -L $launcher_reclaim || ! -f $launcher_reclaim ]] ||
          [[ $(file_stamp "$launcher_reclaim" 2>/dev/null) != "$owner_stamp" ]]; then
        echo '启动锁包含未知回收标记；不会删除它。' >&2; return 1
      fi
      sleep 0.1; continue
    fi
    if ! ln -n "$launcher_owner" "$launcher_reclaim" 2>/dev/null; then
      sleep 0.1; continue
    fi
    reclaim_stamp=$(file_stamp "$launcher_reclaim")
    if [[ $(file_stamp "$launcher_lock") != "$directory_stamp" ||
          $(file_stamp "$launcher_owner") != "$owner_stamp" ||
          $reclaim_stamp != "$owner_stamp" || $(cat "$launcher_owner") != "$owner" ]] ||
        pid_exists "$owner"; then
      release_reclaim
      echo '启动锁在检查期间发生变化，请重试。' >&2; return 1
    fi
    for path in "$launcher_lock"/* "$launcher_lock"/.[!.]* "$launcher_lock"/..?*; do
      if [[ -e $path || -L $path ]] && [[ $path != "$launcher_owner" && $path != "$launcher_reclaim" ]]; then
        release_reclaim
        echo '启动锁包含未知文件；不会删除它。' >&2; return 1
      fi
    done
    rm -f -- "$launcher_owner"
    release_reclaim
    if ! rmdir "$launcher_lock"; then
      echo '启动锁无法安全回收，请检查 private/launcher.lock。' >&2; return 1
    fi
  done
  echo '启动锁尚未写入有效 owner.pid 或正在回收，请稍后重试。' >&2
  return 1
}

stop_native() {
  if native_running; then
    local pid attempt=0
    pid=$(cat "$pid_file")
    kill -TERM "$pid"
    while kill -0 "$pid" 2>/dev/null; do
      ((attempt += 1))
      if ((attempt > 60)); then echo '服务尚未退出，请查看日志；不会强制终止。' >&2; return 1; fi
      sleep 1
    done
    echo '本机服务已停止。'
  fi
  rm -f "$pid_file"
}

stop_source_ui() {
  if source_ui_running; then
    local pid attempt=0
    pid=$(cat "$source_ui_pid_file")
    kill -TERM "$pid"
    while source_ui_running; do
      ((attempt += 1))
      if ((attempt > 60)); then echo '源码前端尚未退出，请查看日志；不会强制终止。' >&2; return 1; fi
      sleep 1
    done
    echo '源码前端已停止。'
  fi
  rm -f -- "$source_ui_pid_file" "$source_ui_port_file" "$source_ui_target_file"
}

source_ui_preflight() {
  command -v node >/dev/null || { echo '源码前端需要 Node.js 22.12 或更高版本。' >&2; return 1; }
  if [[ ! -f $vite_entry ]]; then
    echo "源码前端依赖尚未安装，请先运行：npm --prefix \"$web_dir\" ci" >&2; return 1
  fi
  node -e 'const [major, minor] = process.versions.node.split(".").map(Number); process.exit(major > 22 || (major === 22 && minor >= 12) ? 0 : 1)' || {
    echo '源码前端需要 Node.js 22.12 或更高版本。' >&2; return 1
  }
  if ! source_ui_running || [[ $(cat "$source_ui_port_file") != "$ui_port" ]]; then
    local http_status
    http_status=$(curl --silent --max-time 2 --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:$ui_port/" || true)
    if [[ $http_status =~ ^[1-9][0-9][0-9]$ ]]; then
      echo "端口 $ui_port 已有 HTTP 服务；不会接管它，请设置其他 EM_PREVIEW_UI_PORT。" >&2; return 1
    fi
  fi
}

start_source_ui() {
  local ui_origin="http://127.0.0.1:$ui_port" ui_pid ready=0 attempt
  if source_ui_running && [[ $(cat "$source_ui_port_file") == "$ui_port" &&
      -f $source_ui_target_file && $(cat "$source_ui_target_file") == "$origin" ]]; then
    if ! curl --fail --silent --max-time 2 "$ui_origin/@vite/client" > /dev/null || ! source_ui_running; then
      echo '源码前端进程运行但未就绪，请检查日志；PID已保留，可用 --stop 停止。' >&2; return 1
    fi
  else
    stop_source_ui
    (
      cd "$web_dir"
      export EM_API_PROXY_TARGET="$origin" EM_PREVIEW_PROXY_ORIGIN="$origin" EM_WEB_PORT="$ui_port"
      exec nohup "$(command -v node)" "$vite_entry" --host 127.0.0.1 --port "$ui_port" --strictPort
    ) > "$preview_root/logs/source-ui.log" 2>&1 < /dev/null &
    ui_pid=$!
    printf '%s\n' "$ui_pid" > "$source_ui_pid_file"
    printf '%s\n' "$ui_port" > "$source_ui_port_file"
    printf '%s\n' "$origin" > "$source_ui_target_file"
    for attempt in $(seq 1 60); do
      if ! kill -0 "$ui_pid" 2>/dev/null; then
        if [[ -f $source_ui_pid_file && ! -L $source_ui_pid_file && $(cat "$source_ui_pid_file") == "$ui_pid" ]]; then
          rm -f -- "$source_ui_pid_file" "$source_ui_port_file" "$source_ui_target_file"
        fi
        echo "源码前端启动失败，请查看 $preview_root/logs/source-ui.log" >&2; return 1
      fi
      # Our fresh log proves this process bound the port; another HTTP service cannot satisfy readiness.
      if grep -Fq "$ui_origin/" "$preview_root/logs/source-ui.log" &&
          curl --fail --silent --max-time 2 "$ui_origin/@vite/client" > /dev/null && source_ui_running; then
        ready=1; break
      fi
      sleep 1
    done
    if [[ $ready != 1 ]]; then echo '源码前端仍在启动，请检查日志；PID已保留，可用 --stop 停止。' >&2; return 1; fi
  fi
  printf 'enabled\n' > "$source_ui_marker"
  browser_origin="$ui_origin"
}

docker_environment() {
  command -v docker >/dev/null || { echo '请先安装并启动 Docker Engine / Docker Desktop。' >&2; return 1; }
  if [[ -z ${DOCKER_HOST:-} && -S "$repo_root/var/preview/vm/linux/sock/docker.sock" ]]; then
    export DOCKER_HOST="unix://$repo_root/var/preview/vm/linux/sock/docker.sock"
    if [[ -z ${DOCKER_CONFIG:-} ]]; then
      export DOCKER_CONFIG="$private_dir/docker-cli"
      if [[ $action != status ]]; then
        mkdir -p "$DOCKER_CONFIG"
        chmod 700 "$DOCKER_CONFIG"
      fi
    fi
  fi
  export EM_DATA_PATH="$data_dir"
  export EM_RUNTIME_CONFIG_PATH="$private_dir/docker-config"
  export EM_ADMIN_PASSWORD_FILE="$private_dir/password.txt"
  export EM_MASTER_KEY_FILE="$private_dir/docker-master.key"
  export EM_RUNTIME_UID="$(id -u)"
  export EM_RUNTIME_GID="$(id -g)"
  export EM_HTTP_PORT="$port"
  export EM_PUBLIC_ORIGIN="$origin"
  if [[ -f "$data_dir/price-catalog.toml" ]]; then
    export EM_CONTAINER_PRICE_CATALOG_PATH=/data/price-catalog.toml
  else
    unset EM_CONTAINER_PRICE_CATALOG_PATH
  fi
  docker compose version >/dev/null
  docker info >/dev/null 2>&1 || { echo 'Docker 引擎不可用；请先启动 Docker。' >&2; return 1; }
}

compose() { docker compose --project-directory "$repo_root" -f "$repo_root/compose.yaml" -p everything-manual "$@"; }

if [[ $action == status ]]; then
  status_running=0
  if native_running; then
    if [[ -f "$private_dir/port" ]]; then port=$(cat "$private_dir/port"); fi
    echo "本机服务运行中：http://127.0.0.1:$port/"; status_running=1
  fi
  if source_ui_running; then
    echo "源码前端运行中：http://127.0.0.1:$(cat "$source_ui_port_file")/"; status_running=1
  elif [[ $source_ui == 1 ]]; then
    echo '源码前端预览已启用，进程尚未运行。'
  fi
  if [[ -f $mode_file && $(cat "$mode_file") == docker ]]; then
    docker_environment; compose ps; exit 0
  fi
  if [[ $status_running == 0 ]]; then echo '统一资料库服务尚未运行。'; fi
  exit 0
fi
mkdir -p "$private_dir" "$preview_root/logs"
chmod 700 "$private_dir" "$preview_root/logs"
trap release_launcher_lock EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
acquire_launcher_lock
# Reload the saved preference after obtaining the lock, in case another launcher changed it.
select_source_ui
if [[ $action == stop ]]; then
  stop_source_ui
  stop_native
  if [[ -f $mode_file && $(cat "$mode_file") == docker ]]; then docker_environment; compose stop; fi
  rm -f "$mode_file"
  exit 0
fi
if [[ $ui_choice == embedded ]]; then
  stop_source_ui
  rm -f -- "$source_ui_marker"
fi

ensure_password() {
  local password confirmation
  if [[ -L "$private_dir/password.txt" ]]; then echo '密码文件不能是符号链接。' >&2; return 1; fi
  if [[ -f "$private_dir/password.txt" ]]; then chmod 600 "$private_dir/password.txt"; return; fi
  if [[ ! -t 0 ]]; then echo '首次启动需在终端设置登录密码，或准备0600的private/password.txt。' >&2; return 1; fi
  read -r -s -p '设置登录密码：' password; echo
  read -r -s -p '再次输入：' confirmation; echo
  if [[ -z $password || $password != "$confirmation" ]]; then echo '两次输入不一致。' >&2; return 1; fi
  printf '%s\n' "$password" > "$private_dir/password.txt"
  unset password confirmation
  chmod 600 "$private_dir/password.txt"
}

if [[ $mode == docker ]]; then
  stop_source_ui
  docker_environment
  ensure_password
  mkdir -p "$data_dir" "$private_dir/docker-config"
  chmod 700 "$private_dir/docker-config"
  if [[ -L "$private_dir/docker-master.key" ]]; then echo '主密钥文件不能是符号链接。' >&2; exit 2; fi
  if [[ ! -e "$private_dir/docker-master.key" ]]; then
    (set -o noclobber; od -An -N32 -tx1 /dev/urandom | tr -d ' \n' > "$private_dir/docker-master.key")
  fi
  chmod 600 "$private_dir/docker-master.key"
  stop_native
  compose config --quiet
  proxy_image=${EM_PROXY_IMAGE:-nginx:1.28-alpine}
  platform=${EM_DOCKER_PLATFORM:-linux/amd64}
  if ! docker image inspect --platform "$platform" "$proxy_image" >/dev/null 2>&1; then
    docker pull --platform "$platform" "$proxy_image"
  fi
  printf 'docker\n' > "$mode_file"
  compose up -d --build --pull never --wait --wait-timeout 300
else
  command -v curl >/dev/null || { echo '缺少curl。' >&2; exit 1; }
  if [[ $source_ui == 1 ]]; then source_ui_preflight; else stop_source_ui; fi
  if [[ -f $mode_file && $(cat "$mode_file") == docker ]]; then docker_environment; compose stop; fi
  if native_running && [[ -f "$private_dir/port" && $(cat "$private_dir/port") != "$port" ]]; then stop_native; fi
  if ! native_running; then
    http_status=$(curl --silent --max-time 2 --output /dev/null --write-out '%{http_code}' "$origin/" || true)
    if [[ $http_status =~ ^[1-9][0-9][0-9]$ ]]; then
      echo "端口 $port 已有 HTTP 服务；请用 --status 查看本项目状态，或设置其他 EM_PREVIEW_PORT。" >&2
      exit 1
    fi
    binary=${EM_PREVIEW_BINARY:-}
    if [[ -z $binary && -f "$private_dir/native-binary" ]]; then binary=$(cat "$private_dir/native-binary"); fi
    if [[ -z $binary ]]; then
      case "$(uname -s)-$(uname -m)" in
        Darwin-arm64) triple=aarch64-apple-darwin ;;
        Linux-x86_64) triple=x86_64-unknown-linux-musl ;;
        Linux-aarch64) triple=aarch64-unknown-linux-musl ;;
        *) echo '尚无此平台的本机发行程序；请使用--docker。' >&2; exit 1 ;;
      esac
      binary="$repo_root/dist/$triple/everything-manual"
      if [[ ! -x $binary ]]; then (cd "$repo_root"; cargo xtask dist --target "$triple"); fi
    fi
    if [[ ! -x $binary ]]; then echo "找不到发行程序：$binary" >&2; exit 1; fi
    if [[ -f "$private_dir/secrets-backend" && -z ${EM_SECRETS_BACKEND:-} ]]; then
      backend=$(cat "$private_dir/secrets-backend")
      case "$backend" in file|keychain) export EM_SECRETS_BACKEND="$backend" ;; *) echo '无效密钥后端配置。' >&2; exit 1 ;; esac
    fi
    export EM_PUBLIC_ORIGIN="$origin"
    if [[ ! -f "$data_dir/manual.sqlite3" ]]; then
      ensure_password
      "$binary" init --data-dir "$data_dir" --password-file "$private_dir/password.txt"
    fi
    nohup "$binary" serve --data-dir "$data_dir" --listen "127.0.0.1:$port" > "$preview_root/logs/server.log" 2>&1 < /dev/null &
    server_pid=$!
    printf '%s\n' "$server_pid" > "$pid_file"
    printf '%s\n' "$port" > "$private_dir/port"
    printf 'native\n' > "$mode_file"
    clear_failed_start() {
      if [[ -f $pid_file && ! -L $pid_file && $(cat "$pid_file") == "$server_pid" ]]; then
        rm -f -- "$pid_file" "$mode_file" "$private_dir/port"
      fi
    }
    ready=0
    for attempt in $(seq 1 60); do
      if ! kill -0 "$server_pid" 2>/dev/null; then
        clear_failed_start
        echo "启动失败，请查看 $preview_root/logs/server.log" >&2; exit 1
      fi
      if curl --fail --silent --max-time 2 "$origin/api/v1/health/ready" > /dev/null; then
        if kill -0 "$server_pid" 2>/dev/null; then ready=1; break; fi
        clear_failed_start
        echo "启动失败，请查看 $preview_root/logs/server.log" >&2; exit 1
      fi
      sleep 1
    done
    if [[ $ready != 1 ]]; then echo '服务仍在启动，请检查日志（包括系统钥匙串提示）。' >&2; exit 1; fi
  else
    if ! curl --fail --silent --max-time 2 "$origin/api/v1/health/ready" > /dev/null || ! native_running; then
      echo '本机服务进程运行但未就绪，请检查日志；PID已保留，可用 --stop 停止。' >&2
      exit 1
    fi
  fi
fi

browser_origin="$origin"
if [[ $mode == native && $source_ui == 1 ]]; then start_source_ui; fi
echo "统一资料库：$data_dir"
echo "Chrome 地址：$browser_origin/"
if [[ $mode == native && $source_ui == 1 ]]; then echo "后端地址：$origin/"; fi
echo '停止：bash scripts/start-project.sh --stop'
if [[ $open_browser == 1 ]]; then
  if [[ $(uname -s) == Darwin ]]; then
    open -a 'Google Chrome' "$browser_origin/" || true
  elif command -v google-chrome >/dev/null; then
    google-chrome "$browser_origin/" >/dev/null 2>&1 &
  fi
fi
