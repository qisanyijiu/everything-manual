//! 工程命令共用工具：仓库根定位与子进程执行（失败不被吞掉）。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// 仓库根目录。xtask 编译自 `<root>/xtask`，取编译期 manifest 目录的父目录。
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 应位于仓库根目录下的子目录")
        .to_path_buf()
}

/// 在指定目录运行命令并透传输出；退出码非零即返回错误。
pub fn run_in<P, I, A>(cwd: &Path, program: P, args: I) -> Result<()>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect();
    println!(
        "$ (cd {} && {} {})",
        cwd.display(),
        program.as_ref().to_string_lossy(),
        args.join(" ")
    );

    let status = Command::new(program.as_ref())
        .args(&args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("无法启动命令 {}", program.as_ref().to_string_lossy()))?;
    if !status.success() {
        bail!(
            "命令失败（退出码 {:?}）：{} {}",
            status.code(),
            program.as_ref().to_string_lossy(),
            args.join(" ")
        );
    }
    Ok(())
}

/// 同 [`run_in`]，但附加环境变量（如发布构建的 `RUSTFLAGS`）。
pub fn run_in_env<P, I, A>(cwd: &Path, program: P, args: I, envs: &[(&str, &str)]) -> Result<()>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect();
    println!(
        "$ (cd {} && {} {} {})",
        cwd.display(),
        envs.iter()
            .map(|(key, value)| format!("{key}={value:?}"))
            .collect::<Vec<_>>()
            .join(" "),
        program.as_ref().to_string_lossy(),
        args.join(" ")
    );

    let mut command = Command::new(program.as_ref());
    command.args(&args).current_dir(cwd);
    for (key, value) in envs {
        command.env(key, value);
    }
    let status = command
        .status()
        .with_context(|| format!("无法启动命令 {}", program.as_ref().to_string_lossy()))?;
    if !status.success() {
        bail!(
            "命令失败（退出码 {:?}）：{} {}",
            status.code(),
            program.as_ref().to_string_lossy(),
            args.join(" ")
        );
    }
    Ok(())
}

/// 在指定目录运行命令并捕获 stdout（要求成功）；stderr 原样透传。
pub fn capture_in<P, I, A>(cwd: &Path, program: P, args: I) -> Result<String>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect();
    let output = Command::new(program.as_ref())
        .args(&args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("无法启动命令 {}", program.as_ref().to_string_lossy()))?;
    if !output.status.success() {
        bail!(
            "命令失败（退出码 {:?}）：{} {}\n{}",
            output.status.code(),
            program.as_ref().to_string_lossy(),
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 字节内容的 sha256 十六进制摘要（内存数据，供冒烟比对下载内容）。
pub fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 构建机（宿主）triple，如 `aarch64-apple-darwin`（取 `rustc -vV` 的 host 行）。
pub fn host_triple() -> Result<String> {
    let raw = capture_in(Path::new("."), "rustc", ["-vV"])?;
    raw.lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_owned())
        .context("rustc -vV 输出缺少 host 行")
}

/// 在指定目录捕获命令 stdout 与 stderr（不要求成功；用于依赖检查等只读命令）。
pub fn capture_in_with_stderr<P, I, A>(cwd: &Path, program: P, args: I) -> Result<(bool, String)>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect();
    let output = Command::new(program.as_ref())
        .args(&args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("无法启动命令 {}", program.as_ref().to_string_lossy()))?;
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), combined))
}

/// sha256 十六进制摘要。
pub fn sha256_hex(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file =
        std::fs::File::open(path).with_context(|| format!("打开文件失败：{}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
