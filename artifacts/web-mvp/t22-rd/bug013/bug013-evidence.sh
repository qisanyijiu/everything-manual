#!/usr/bin/env bash
# BUG-013 修复取证（在 Linux 容器内运行；由 RD 于 T22 修复回合编写）。
#
# 目的：
#   1) 连跑 3 次精确用例 legacy_schema_backup_restores_and_migrates_automatically，要求 3/3 通过
#      （回合 30 在 Linux 上为 3/3 失败，panic 于 backup_restore.rs:2390 left:-1 right:0）；
#   2) 跑 cargo test --workspace --no-fail-fast，要求由 557 passed / 1 failed / 2 ignored
#      变为 558 passed / 0 failed / 2 ignored（测试目标数 37 不变）。
#
# 不连外网做任何产品/收费调用；只用本地源码与 cargo 缓存；日志写到 /evidence。
set -uo pipefail

EVIDENCE_DIR="${EVIDENCE_DIR:-/evidence}"
WORK_DIR="${EM_WORK_DIR:-/src/everything-manual}"
EXACT_TEST="legacy_schema_backup_restores_and_migrates_automatically"
EXACT_LOG="$EVIDENCE_DIR/linux-bug013-exact-test.log"
WORKSPACE_LOG="$EVIDENCE_DIR/linux-workspace-test-raw.log"
SUMMARY_LOG="$EVIDENCE_DIR/linux-bug013-summary.txt"

log() { echo "[evidence] $*"; }

: >"$EVIDENCE_DIR/.keep"

echo "== 运行环境（证明是 Linux 容器内，而不是宿主 macOS）=="
uname -a
head -2 /etc/os-release | tr '\n' ' '
echo
echo "rustc: $(rustc -vV | tr '\n' ' ')"
echo "cargo: $(cargo -V)"
echo "cpu: $(nproc) vCPU"
echo "CARGO_HOME=${CARGO_HOME:-<未设置>} RUSTUP_HOME=${RUSTUP_HOME:-<未设置>} HOME=$HOME"

cd "$WORK_DIR" || exit 1

echo
echo "== 工作树版本与修复现场（必须能看到 settle helper 与两处调用）=="
git rev-parse HEAD 2>/dev/null || echo "（无 git 元数据，工作树是 rsync 副本）"
git status --porcelain 2>/dev/null | head
grep -n "settle_after_listening_line" \
  crates/server/tests/common/mod.rs \
  crates/server/tests/backup_restore.rs \
  crates/server/tests/config_cli.rs \
  || { echo "修复标记缺失，取证终止" | tee -a "$SUMMARY_LOG"; exit 2; }

exact_failures=0
{
  echo "== 精确用例连跑 3 次：cargo test -p everything-manual --test backup_restore -- --exact $EXACT_TEST =="
  for round in 1 2 3; do
    echo
    echo "-------- 第 $round 次（开始 $(date -u +%H:%M:%SZ)）--------"
    cargo test -p everything-manual --test backup_restore -- --exact "$EXACT_TEST"
    rc=$?
    echo "EXIT=$rc（第 $round 次）"
    [ "$rc" -ne 0 ] && exact_failures=$((exact_failures + 1))
  done
} 2>&1 | tee "$EXACT_LOG"

echo
echo "== 全量：cargo test --workspace --no-fail-fast =="
cargo test --workspace --no-fail-fast 2>&1 | tee "$WORKSPACE_LOG"
workspace_rc=${PIPESTATUS[0]}
echo "workspace cargo EXIT=$workspace_rc"

{
  echo "BUG-013 取证汇总（$(date -u +%Y-%m-%dT%H:%M:%SZ)）"
  echo "精确用例连跑 3 次：失败次数=$exact_failures / 3"
  echo "workspace cargo 退出码=$workspace_rc"
  echo "测试目标数与 passed/failed/ignored 汇总（解析 'test result:' 行）："
  awk '
    /^     Running/ || /^    Running/ { binaries += 1 }
    /^   Doc-tests/ { doctests += 1 }
    /^test result:/ {
      results += 1;
      for (i = 1; i <= NF; i++) {
        if ($i == "passed;") passed += $(i-1);
        if ($i == "failed;") failed += $(i-1);
        if ($i == "ignored;") ignored += $(i-1);
      }
    }
    END {
      printf "  Running 行（测试二进制）= %d；Doc-tests = %d；test result 行 = %d\n", binaries, doctests, results;
      printf "  passed=%d failed=%d ignored=%d\n", passed, failed, ignored;
    }
  ' "$WORKSPACE_LOG"
  echo "失败的测试目标与用例（若有）："
  grep -E "^test .* FAILED" "$WORKSPACE_LOG" || echo "  （无）"
  grep -n "test result: FAILED" "$WORKSPACE_LOG" || echo "  （无 FAILED 的 test result 行）"
} 2>&1 | tee "$SUMMARY_LOG"

echo
echo "== 取证结束 =="
