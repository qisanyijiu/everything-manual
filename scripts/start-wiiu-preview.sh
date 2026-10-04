#!/usr/bin/env bash
# Start the existing Wii U sample preview. Run from any directory; Ctrl+C stops both services.
set -euo pipefail

if [[ $# -ne 0 ]]; then
  echo "用法：$0（无需参数）" >&2
  exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
backend="$repo_root/var/prd-completion/chrome-live-model-discovery-bin/everything-manual-long-llm-timeout"
data_dir="$repo_root/var/chrome-live-wiiu-20261003"
web_dir="$repo_root/apps/web"
api_url=http://127.0.0.1:8082
web_url=http://127.0.0.1:5173

if [[ ! -x $backend ]]; then
  echo "找不到预览后端：$backend" >&2
  exit 1
fi
if [[ ! -f $data_dir/config.toml || ! -f $data_dir/manual.sqlite3 ]]; then
  echo "找不到 Wii U 样本数据：$data_dir" >&2
  exit 1
fi
if [[ ! -x $web_dir/node_modules/.bin/vite ]]; then
  echo "前端依赖尚未安装；请先运行 npm --prefix '$web_dir' ci" >&2
  exit 1
fi
for command_name in node curl lsof; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "缺少命令：$command_name" >&2
    exit 1
  fi
done
for port in 8082 5173; do
  if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "端口 $port 已被占用；请先检查现有服务，脚本不会结束它。" >&2
    exit 1
  fi
done

backend_pid=
web_pid=
cleanup() {
  trap - EXIT INT TERM
  if [[ -n $web_pid ]]; then
    kill "$web_pid" 2>/dev/null || true
    wait "$web_pid" 2>/dev/null || true
  fi
  if [[ -n $backend_pid ]]; then
    kill "$backend_pid" 2>/dev/null || true
    wait "$backend_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

echo "正在启动后端：$api_url"
cd "$repo_root"
EM_CONCURRENCY__MANUAL_AI_BATCHES=1 \
EM_MANUAL_AI_REQUEST_TIMEOUT_SECONDS=600 \
  "$backend" serve --data-dir "$data_dir" --listen 127.0.0.1:8082 &
backend_pid=$!

echo '等待后端就绪；如果 macOS 弹出钥匙串授权，请在系统弹窗中处理。'
attempt=0
until curl --fail --silent --show-error --max-time 2 \
  "$api_url/api/v1/health/ready" >/dev/null 2>&1; do
  if ! kill -0 "$backend_pid" 2>/dev/null; then
    wait "$backend_pid" || true
    echo '后端启动失败，请查看上方日志。' >&2
    exit 1
  fi
  ((attempt += 1))
  if ((attempt % 30 == 0)); then
    echo '仍在等待后端；按 Ctrl+C 可停止本次启动。'
  fi
  sleep 1
done

echo "正在启动前端：$web_url"
cd "$web_dir"
EM_API_PROXY_TARGET="$api_url" \
EM_PREVIEW_PROXY_ORIGIN="$api_url" \
  ./node_modules/.bin/vite --host 127.0.0.1 --port 5173 &
web_pid=$!

echo "请在 Chrome 打开 $web_url/；按 Ctrl+C 停止服务。"
while kill -0 "$backend_pid" 2>/dev/null && kill -0 "$web_pid" 2>/dev/null; do
  sleep 1
done
if ! kill -0 "$backend_pid" 2>/dev/null; then
  wait "$backend_pid" || true
  echo '后端已退出，请查看上方日志。' >&2
  exit 1
fi
wait "$web_pid"
