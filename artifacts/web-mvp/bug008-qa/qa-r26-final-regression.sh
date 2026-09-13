#!/bin/bash
# QA 回合 26 收尾回归：workspace 全量、xtask check 7 项、dist 二次重建同哈希、smoke-bootstrap。
# 用法：bash qa-r26-final-regression.sh
set -u
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
cd "${ROOT}" || exit 1

run() { local name="$1"; shift; echo "== ${name}：$*"; "$@" > "${QA_DIR}/qa-${name}.log" 2>&1; echo "== ${name} exit=$?"; }

run cargo-test-workspace cargo test --workspace
run xtask-check-final cargo xtask check
run xtask-dist-2 cargo xtask dist --target aarch64-apple-darwin
shasum -a 256 dist/aarch64-apple-darwin/everything-manual | tee "${QA_DIR}/qa-dist-sha256-2.txt"
run xtask-smoke-bootstrap-final cargo xtask smoke-bootstrap --binary "${ROOT}/dist/aarch64-apple-darwin/everything-manual"
echo "== workspace 汇总"
grep -E "^test result" "${QA_DIR}/qa-cargo-test-workspace.log" | awk '{p+=$4; f+=$6; i+=$8} END {print "  passed="p" failed="f" ignored="i}'
echo "== xtask check 明细"
grep -E "\[通过\]|\[失败\]" "${QA_DIR}/qa-xtask-check-final.log"
echo "== 结束 $(date -u +%Y-%m-%dT%H:%M:%SZ)"
