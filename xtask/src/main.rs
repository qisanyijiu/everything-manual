//! `cargo xtask` —— 工程命令入口（命令语义见 llmdoc/validation-release.md §2）。
//!
//! 通过根 `.cargo/config.toml` 的 alias 调用：`cargo xtask <command>`。
//! 本工具不隐式调用收费 API、不推送 Git、不发布公网内容。

mod check;
mod contracts;
mod dist;
mod smoke;
mod util;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "xtask",
    about = "万物说明书工程命令（contracts / check / dist / smoke / smoke-bootstrap）"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 从 Rust DTO 生成 contracts/openapi.json 与 apps/web/src/api/generated.ts
    Contracts {
        /// 只比较不写入：临时生成后与工作树比较，有差异非零退出
        #[arg(long)]
        check: bool,
    },
    /// 本地全量检查：fmt、clippy、Rust 测试、前端 lint/typecheck/test、合同漂移
    Check,
    /// 构建 release 单二进制并产出 SHA256、licenses、build-info 与动态依赖清单
    Dist {
        /// 目标 triple（如 aarch64-apple-darwin）
        #[arg(long)]
        target: String,
        /// 产出后清掉本包构件重新构建一次，比对二进制 sha256（可复现性证据）
        #[arg(long)]
        check_reproducible: bool,
    },
    /// T01 冷目录冒烟：拷贝二进制到新临时目录，验证内嵌页面、静态资源与 health
    SmokeBootstrap {
        /// release 二进制的绝对路径
        #[arg(long)]
        binary: PathBuf,
    },
    /// T22 正式包冷目录冒烟：restore 合法样例备份 → 生产服务 → 认证/资源/Range/
    /// 持久化/断网/备份恢复链（validation-release §7 的 7 步）
    Smoke {
        /// release 二进制的绝对路径
        #[arg(long)]
        binary: PathBuf,
        /// 合法样例备份目录（默认 T20 演练样例 artifacts/web-mvp/t20-rd/sample-backup）
        #[arg(long)]
        backup: Option<PathBuf>,
        /// 样例备份的管理员口令（默认使用 T20 演练口令；不是生产凭据）
        #[arg(long)]
        password: Option<String>,
        /// 失败时保留临时目录以便排障
        #[arg(long)]
        keep: bool,
        /// 跳过 macOS sandbox-exec 断网复检（仅排障用；默认在可用时执行）
        #[arg(long)]
        skip_offline_sandbox: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Contracts { check } => contracts::run(check),
        Command::Check => check::run(),
        Command::Dist {
            target,
            check_reproducible,
        } => dist::run(&dist::DistOptions {
            target,
            check_reproducible,
        }),
        Command::SmokeBootstrap { binary } => smoke::run_bootstrap(&binary),
        Command::Smoke {
            binary,
            backup,
            password,
            keep,
            skip_offline_sandbox,
        } => smoke::run(&smoke::SmokeOptions {
            binary,
            backup,
            password,
            keep,
            skip_offline_sandbox,
        }),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("\nxtask 失败: {error:#}");
            ExitCode::FAILURE
        }
    }
}
