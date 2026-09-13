//! 命令行定义（clap）。
//!
//! 合同（validation-release.md §5）：
//! - `init --data-dir <dir>`：初始化 data-dir，密码**交互输入或来自受限文件**；
//! - `serve --data-dir <dir> [--listen <addr>]`：默认 `127.0.0.1:8080`；
//! - `check --data-dir <dir>`：只验证配置／目录／schema，不调用任何外部 API；
//! - `backup --data-dir <dir> --out <新路径>`：**要求先停服**（排他锁）的一致快照 +
//!   被引用 blob + manifest + sha256；`--out` 必须不存在（T20）。
//! - `restore --from <备份> --data-dir <新目录>`：目标必须不存在或为空；先校验
//!   hash/外键/引用再写入（T20）。
//!
//! 密码不存在任何命令行参数形式：输入只经交互终端（不回显）或 `--password-file`，
//! 不会进入 shell history（REQ-002、§5.1）。

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// 万物说明书服务：单管理员、自托管资料库（React 前端 + Rust 服务）。
#[derive(Debug, Parser)]
#[command(
    name = "everything-manual",
    version,
    about = "万物说明书服务（init / serve / check / backup / restore）",
    long_about = "万物说明书：上传说明书与多视图照片，生成可校准的交互说明书。\n\
                  数据保存在独立 data-dir；同一 data-dir 同时只允许一个进程持有排他锁。",
    disable_help_subcommand = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// 初始化 data-dir（创建目录结构与锁文件）
    Init(InitArgs),
    /// 启动 HTTP 服务（默认仅监听 127.0.0.1）
    Serve(ServeArgs),
    /// 校验配置、data-dir 与 schema；不发起外部请求、不产生费用
    Check(CheckArgs),
    /// 备份 data-dir（要求先停止服务；产出快照 + 被引用 blob + manifest，不覆盖已有输出）
    Backup(BackupArgs),
    /// 从备份恢复到新 data-dir（目标必须不存在或为空；校验 hash/外键/引用后写入）
    Restore(RestoreArgs),
}

/// 各子命令共用的非密钥选项。
#[derive(Debug, Args)]
pub struct CommonArgs {
    /// data-dir 路径（也可用 EM_DATA_DIR 或配置键 data_dir 提供）
    #[arg(long, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,
    /// 配置文件路径；默认依次查找 <data-dir>/config.toml、./config.toml
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// 从受限文件读取管理员密码（无人值守；文件权限要求 0600，无回显终端时必需）
    #[arg(long, value_name = "FILE")]
    pub password_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// 监听地址；默认 127.0.0.1:8080（非 loopback 需 TLS 或可信代理配置）
    #[arg(long, value_name = "ADDR")]
    pub listen: Option<SocketAddr>,
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// 覆盖监听地址以验证启动安全规则（与 serve 的同一规则）
    #[arg(long, value_name = "ADDR")]
    pub listen: Option<SocketAddr>,
}

#[derive(Debug, Args)]
pub struct BackupArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// 备份输出路径（必须是不存在的路径，避免覆盖；相对路径相对当前工作目录）
    #[arg(long, value_name = "PATH")]
    pub out: PathBuf,
}

#[derive(Debug, Args)]
pub struct RestoreArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// 备份来源路径（`backup` 产出的目录；相对路径相对当前工作目录）
    #[arg(long, value_name = "PATH")]
    pub from: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("everything-manual").chain(args.iter().copied()))
    }

    #[test]
    fn dispatches_all_subcommands() {
        assert!(matches!(
            parse(&["init", "--data-dir", "/tmp/x"]).unwrap().command,
            Command::Init(_)
        ));
        assert!(matches!(
            parse(&["serve", "--data-dir", "/tmp/x", "--listen", "127.0.0.1:0"])
                .unwrap()
                .command,
            Command::Serve(_)
        ));
        assert!(matches!(
            parse(&["check", "--data-dir", "/tmp/x"]).unwrap().command,
            Command::Check(_)
        ));
        assert!(matches!(
            parse(&["backup", "--data-dir", "/tmp/x", "--out", "/tmp/b"])
                .unwrap()
                .command,
            Command::Backup(_)
        ));
        assert!(matches!(
            parse(&["restore", "--from", "/tmp/b", "--data-dir", "/tmp/x"])
                .unwrap()
                .command,
            Command::Restore(_)
        ));
    }

    #[test]
    fn password_is_never_a_command_line_option() {
        let error = parse(&["init", "--data-dir", "/tmp/x", "--password", "hunter2"])
            .expect_err("--password 必须不存在");
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("--password"), "{error}");
    }

    #[test]
    fn unknown_subcommand_is_a_usage_error() {
        let error = parse(&["frobnicate"]).expect_err("未知子命令必须报错");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn bad_listen_value_is_reported() {
        let error = parse(&["serve", "--listen", "not-an-addr"]).expect_err("非法地址必须报错");
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("not-an-addr"), "{error}");
    }

    #[test]
    fn missing_required_backup_out_fails() {
        let error = parse(&["backup", "--data-dir", "/tmp/x"]).expect_err("--out 必填");
        assert_eq!(error.exit_code(), 2);
    }
}
