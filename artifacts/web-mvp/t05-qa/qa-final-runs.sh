#!/bin/bash
# T05 QA 回合 5：AC-014/AC-015/回归 的正式命令（串行执行，全部留日志）
set -u
cd /Users/qsyj/Code/rust/everything-manual || exit 1
OUT=artifacts/web-mvp/t05-qa

echo "== 1) fixture_harness (debug)"
cargo test -p everything-manual --test fixture_harness > "$OUT/fixture-harness-qa.log" 2>&1
echo "exit=$? $(grep -E 'test result:' "$OUT/fixture-harness-qa.log" | tail -1)"

echo "== 2) fixture_harness (release)"
cargo test --release -p everything-manual --test fixture_harness > "$OUT/fixture-harness-release-qa.log" 2>&1
echo "exit=$? $(grep -E 'test result:' "$OUT/fixture-harness-release-qa.log" | tail -1)"

echo "== 3) 回归集成测试（bootstrap/config_cli/storage/auth_api）"
cargo test -p everything-manual --test bootstrap --test config_cli --test storage --test auth_api > "$OUT/regression-targets-qa.log" 2>&1
echo "exit=$? $(grep -cE 'test result: ok' "$OUT/regression-targets-qa.log") ok-suites; failures=$(grep -cE 'test result: FAILED' "$OUT/regression-targets-qa.log")"

echo "== 4) cargo xtask check"
cargo xtask check > "$OUT/xtask-check-qa.log" 2>&1
echo "exit=$? 通过步数=$(grep -c '\[通过\]' "$OUT/xtask-check-qa.log") 失败步数=$(grep -c '\[失败\]' "$OUT/xtask-check-qa.log")"

echo "== 5) cargo xtask contracts --check"
cargo xtask contracts --check > "$OUT/contracts-check-qa.log" 2>&1
echo "exit=$?"

echo "== 6) cargo tree（生产依赖树 / dev 反向）"
{
  echo "--- --edges normal | test-support 命中数:"
  cargo tree -p everything-manual --edges normal | grep -c "test-support"
  echo "--- --edges normal -i test-support:"
  cargo tree -p everything-manual --edges normal -i test-support 2>&1 | head -5
  echo "--- --edges dev -i test-support:"
  cargo tree -p everything-manual --edges dev -i test-support 2>&1 | head -8
  echo "--- --edges normal | reqwest 命中数:"
  cargo tree -p everything-manual --edges normal | grep -c "reqwest"
} > "$OUT/cargo-tree-qa.log" 2>&1
echo "exit=$? $(head -2 "$OUT/cargo-tree-qa.log" | tail -1)"

echo "== 7) cargo xtask dist"
cargo xtask dist --target aarch64-apple-darwin > "$OUT/dist-qa.log" 2>&1
echo "exit=$?"
shasum -a 256 dist/aarch64-apple-darwin/everything-manual | tee "$OUT/dist-hash-qa.txt"

echo "== 8) smoke-bootstrap"
cargo xtask smoke-bootstrap --binary "$PWD/dist/aarch64-apple-darwin/everything-manual" > "$OUT/smoke-bootstrap-qa.log" 2>&1
echo "exit=$? 检查项=$(grep -c '\[检查\]' "$OUT/smoke-bootstrap-qa.log")"

echo "ALL DONE"
