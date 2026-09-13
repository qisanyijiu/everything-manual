#!/bin/bash
# BUG-008 复验（QA 回合 26）回归链：QA 现场重跑，日志留在本目录。
# 用法：bash qa-regression-chain.sh
set -u
QA_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${QA_DIR}/../../.." && pwd)"
BIN="${ROOT}/dist/aarch64-apple-darwin/everything-manual"
cd "${ROOT}" || exit 1

run() { # 名称 命令...
  local name="$1"; shift
  echo "== ${name}：$*"
  "$@" > "${QA_DIR}/qa-${name}.log" 2>&1
  echo "== ${name} exit=$?"
}

shasum -a 256 contracts/openapi.json apps/web/src/api/generated.ts > "${QA_DIR}/qa-contracts-hashes-before.txt"

run cargo-test-workspace cargo test --workspace
run cargo-test-backup_restore cargo test -p everything-manual --test backup_restore
run cargo-test-tripo_contract cargo test -p everything-manual --test tripo_contract
run cargo-test-pipeline cargo test -p everything-manual --test pipeline
run cargo-test-model_assets cargo test -p everything-manual --test model_assets
run cargo-test-jobs_recovery cargo test -p everything-manual --test jobs_recovery
run xtask-check cargo xtask check
run xtask-contracts-check cargo xtask contracts --check
run xtask-smoke-bootstrap cargo xtask smoke-bootstrap --binary "${BIN}"

shasum -a 256 contracts/openapi.json apps/web/src/api/generated.ts > "${QA_DIR}/qa-contracts-hashes-after.txt"
echo "== 合同生成物哈希对比（应完全一致）"
diff "${QA_DIR}/qa-contracts-hashes-before.txt" "${QA_DIR}/qa-contracts-hashes-after.txt" && echo "  一致（未改工作树）"
echo "== 回归链结束 $(date -u +%Y-%m-%dT%H:%M:%SZ)"
