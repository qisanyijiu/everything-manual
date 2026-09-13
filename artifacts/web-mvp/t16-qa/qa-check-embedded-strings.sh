#!/usr/bin/env bash
# QA 回合 18（T16）：独立复核 dist 单二进制内嵌前端是否含 T16 关键文案。
# 只启动自己拉起的进程并只结束它；临时目录用 trap 清理。
set -euo pipefail
BIN="${1:?用法: qa-check-embedded-strings.sh <dist 二进制绝对路径>}"
PORT="${2:-18091}"
WORK="$(mktemp -d /tmp/qa-t16-embed.XXXXXX)"
PID=""
cleanup() { [ -n "${PID}" ] && kill "${PID}" 2>/dev/null || true; rm -rf "${WORK}"; }
trap cleanup EXIT
echo "workdir=${WORK}（仅含二进制复制与 data-dir）"
cp "${BIN}" "${WORK}/everything-manual"
printf 'qa-t16-embedded-strings\n' > "${WORK}/password.txt"
chmod 600 "${WORK}/password.txt"
"${WORK}/everything-manual" init --data-dir "${WORK}/data" --password-file "${WORK}/password.txt" >/dev/null
"${WORK}/everything-manual" serve --data-dir "${WORK}/data" --listen "127.0.0.1:${PORT}" >"${WORK}/serve.log" 2>&1 &
PID=$!
for _ in $(seq 1 40); do
  if curl -fsS "http://127.0.0.1:${PORT}/api/v1/health/ready" >/dev/null 2>&1; then break; fi
  sleep 0.25
done
INDEX="$(curl -fsS "http://127.0.0.1:${PORT}/")"
JS="$(printf '%s' "${INDEX}" | grep -o '/assets/index-[A-Za-z0-9_-]*\.js' | head -1)"
echo "index.html 内嵌 JS：${JS}"
BODY="$(curl -fsS "http://127.0.0.1:${PORT}${JS}")"
echo "JS 字节数：$(printf '%s' "${BODY}" | wc -c)"
FAIL=0
# 说明：预算语义文案（"不是供应商账户级硬封顶"）来自服务端 budgetNotice（前端原样渲染），
# 故不在前端 JS 里断言，改在下方对二进制字节本身断言。
for s in "已受理（202）" "不代表生成结果" "发送给说明书 AI" "将发送的资料与确认" "重新获取报价" "生成 3D 与说明书草稿"; do
  if printf '%s' "${BODY}" | grep -qF "$s"; then
    echo "[命中] $s"
  else
    echo "[缺失] $s"; FAIL=1
  fi
done
# 服务端固定文案（BUDGET_NOTICE）必须随二进制发布：直接对二进制字节断言。
# 注意：对二进制内容做多字节匹配必须用字节语义（LC_ALL=C），否则 zh_CN.UTF-8 下 grep
# 会因无效多字节序列而漏报（本脚本踩过一次）。
for s in "不是供应商账户级硬封顶" "供应商实际计费以账单为准"; do
  if LC_ALL=C grep -aq "$s" "${WORK}/everything-manual"; then
    echo "[命中·二进制] $s"
  else
    echo "[缺失·二进制] $s"; FAIL=1
  fi
done
exit ${FAIL}
