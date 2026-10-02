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
    /// 受限授权文件控制的具名本地验证；先 --plan 核对，执行需独立授权。
    /// 仅引用已初始化的专用实例及已上传/准备资料，复用现有安全供应商配置。
    /// 本地实例访问权是此停服命令的管理边界；不创建网页登录会话。
    #[command(
        after_help = "计划：cargo xtask test-live --plan --case <案例.json>\n执行：cargo xtask test-live --case <案例.json> --budget-file <授权.json>\n授权文件须0600，creditMinor/usdMicros分别核对；不要把密码、API key或cookie放入命令、案例或授权。\n同一授权重跑恢复原任务；unknown仅对账，不自动重购。命令不确认知识/热点或发布。\n合同与示例：llmdoc/test-live.md。真实供应商调用仍须独立用户授权。"
    )]
    TestLive {
        #[arg(long)]
        case: PathBuf,
        #[arg(long)]
        budget_file: Option<PathBuf>,
        /// 只计算绑定资料的同源计划，不执行、不调用供应商
        #[arg(long)]
        plan: bool,
    },
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
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if std::env::args_os()
                .nth(1)
                .is_some_and(|arg| arg == "test-live")
                && !matches!(
                    error.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                ) =>
        {
            eprintln!(
                "未执行：命令参数无效。使用 cargo xtask test-live --help 核对参数；不要传递凭据值。未发起供应商请求。"
            );
            return ExitCode::FAILURE;
        }
        Err(error) => error.exit(),
    };

    let result = match cli.command {
        Command::TestLive {
            case,
            budget_file,
            plan,
        } => return test_live(case, budget_file, plan),
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

fn test_live(case: PathBuf, budget_file: Option<PathBuf>, plan: bool) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("未执行：本地运行环境不可用；未发起供应商请求。");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(everything_manual::test_live::run(
        everything_manual::test_live::RunOptions {
            case,
            budget_file,
            plan,
        },
    )) {
        Ok(result) => {
            println!(
                "{}\n{}",
                result.title,
                serde_json::to_string_pretty(&result.data).expect("report JSON serializes")
            );
            if result.success {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            if error.execution_started {
                eprintln!(
                    "执行未完成：{} [{}]。保留授权记录；用同一授权核对已有成果。",
                    error.message(),
                    error.code
                );
            } else {
                eprintln!(
                    "未执行：{} [{}]。未发起供应商请求。",
                    error.message(),
                    error.code
                );
            }
            ExitCode::FAILURE
        }
    }
}
