//! `cargo xtask check` —— T01 范围的可执行检查子集。
//!
//! 步骤（全部执行、逐个报告，最后统一判定，不短路隐藏失败）：
//! 1. `cargo fmt --check`
//! 2. `cargo clippy --workspace --all-targets -- -D warnings`
//! 3. `cargo test --workspace`
//! 4. `npm --prefix apps/web run lint`
//! 5. `npm --prefix apps/web run typecheck`
//! 6. `npm --prefix apps/web run test -- --run`
//! 7. `cargo xtask contracts --check`
//!
//! 后续卡（T21/T22）按需追加 e2e、安全矩阵与发布检查。

use anyhow::{Result, bail};

use crate::contracts;
use crate::util::{repo_root, run_in};

pub fn run() -> Result<()> {
    let root = repo_root();
    let web = root.join("apps/web");

    let mut failed: Vec<String> = Vec::new();

    let step = |name: &str, result: Result<()>, failed: &mut Vec<String>| match result {
        Ok(()) => println!("[通过] {name}"),
        Err(error) => {
            println!("[失败] {name}: {error:#}");
            failed.push(name.to_owned());
        }
    };

    step(
        "cargo fmt --check",
        run_in(&root, "cargo", ["fmt", "--check"]),
        &mut failed,
    );
    step(
        "cargo clippy --workspace --all-targets -- -D warnings",
        run_in(
            &root,
            "cargo",
            [
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        &mut failed,
    );
    step(
        "cargo test --workspace",
        run_in(&root, "cargo", ["test", "--workspace"]),
        &mut failed,
    );
    step(
        "npm --prefix apps/web run lint",
        run_in(&web, "npm", ["run", "lint"]),
        &mut failed,
    );
    step(
        "npm --prefix apps/web run typecheck",
        run_in(&web, "npm", ["run", "typecheck"]),
        &mut failed,
    );
    step(
        "npm --prefix apps/web run test -- --run",
        run_in(&web, "npm", ["run", "test", "--", "--run"]),
        &mut failed,
    );
    step(
        "cargo xtask contracts --check",
        contracts::run(true),
        &mut failed,
    );

    if !failed.is_empty() {
        bail!("以下检查未通过：{}", failed.join("、"));
    }
    println!("\n全部检查通过。");
    Ok(())
}
