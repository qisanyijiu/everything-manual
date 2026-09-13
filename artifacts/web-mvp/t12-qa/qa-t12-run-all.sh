#!/bin/bash
# T12 QA 回合 14：依次跑全部场景（真实 dist 二进制 + QA 自建 fixture）。
# 用法：qa-t12-run-all.sh [binary]（默认 dist/aarch64-apple-darwin/everything-manual）
set -uo pipefail
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="${1:-$QA_DIR/../../../dist/aarch64-apple-darwin/everything-manual}"

FAILED=0
run() { # $1=scenario $2=app_port $3=fixture_port
  echo "########## 场景 $1 ##########"
  bash "$QA_DIR/qa-t12-scenario.sh" "$BIN" "$1" "/tmp/qa-t12-$1" "$2" "$3" \
    > "$QA_DIR/qa-$1-run.log" 2>&1
  local code=$?
  echo "场景 $1 exit=$code"
  [ $code = 0 ] || FAILED=1
}

run happy          19201 19202
run disconnect     19203 19204
run unknown        19205 19206
run business_error 19207 19208
run no_model       19209 19210
run token_unknown  19211 19212
run submit_429     19213 19214
run poll_503       19215 19216
run no_billing     19217 19218
run token_verbatim 19219 19220

echo "########## 汇总：$([ $FAILED = 0 ] && echo ALL-PASS || echo HAS-FAIL) ##########"
exit $FAILED
