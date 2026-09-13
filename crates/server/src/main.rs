//! everything-manual 服务入口（T02：完整 CLI）。
//!
//! 子命令：`init` / `serve` / `check` / `backup` / `restore`（语义见
//! llmdoc/validation-release.md §5 与 `src/config/` 模块文档）。
//!
//! 约定：
//! - 错误以可读单行消息写 stderr（`错误：...`），退出码见 [`everything_manual::config::ExitCode`]，
//!   不输出 panic 堆栈；
//! - `serve` 成功后向 stdout 打印 `listening on http://<addr>`：
//!   `cargo xtask smoke-bootstrap` 依赖该行为定位实际端口（`--listen 127.0.0.1:0`）。

use std::process::ExitCode;

use everything_manual::config::commands;

#[tokio::main]
async fn main() -> ExitCode {
    match commands::run_from_args(std::env::args()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("错误：{}", error.message);
            ExitCode::from(error.exit_code.as_u8())
        }
    }
}
