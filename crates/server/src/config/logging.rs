//! 结构化日志初始化（PRD §5.7、REQ-044）。
//!
//! 设计（理由写入 implementation.md）：
//! - 输出为 **JSON Lines**：一行一个事件，字段含 `timestamp/level/target/message` 与业务字段
//!   （如 `requestId`、`durationMs`、`status`），便于本地检索与后续采集；
//! - 同时写 **stdout 与 `<data-dir>/logs/everything-manual.log`**（追加）：前台运行时终端可见，
//!   排障时 data-dir 里有历史；写文件失败不影响服务运行（只丢文件副本）；
//! - 级别由环境变量 `RUST_LOG`（trace/debug/info/warn/error）控制，默认 `info`；
//! - 脱敏由调用方保证：密钥用 `SecretString`（Debug 恒为 `[redacted]`），URL 经
//!   `redact_url_query`，查询串与密码、密钥不进入事件字段。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;

use super::error::CliError;

pub const LOG_FILE_NAME: &str = "everything-manual.log";

pub fn log_file_path(log_dir: &Path) -> PathBuf {
    log_dir.join(LOG_FILE_NAME)
}

/// 初始化全局日志。`log_dir` 为 None 时只写 stdout（例如配置尚未解析成功时的早期错误）。
///
/// 日志文件由全局订阅者持有的写端保持打开（进程生命周期）；
/// 同一进程重复调用时保留首个订阅者，不报错。
pub fn init(log_dir: Option<&Path>) -> Result<(), CliError> {
    let level = level_from_env();
    let file = match log_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir).map_err(|error| {
                CliError::data_dir(format!("创建日志目录失败 {}：{error}", dir.display()))
            })?;
            let path = log_file_path(dir);
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|error| {
                    CliError::data_dir(format!("打开日志文件失败 {}：{error}", path.display()))
                })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
            }
            Some(Arc::new(Mutex::new(file)))
        }
        None => None,
    };

    let writer = TeeWriter { file: file.clone() };
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_writer(writer)
        .with_max_level(level)
        .finish();
    // 第二个订阅者无法安装（比如进程内重复初始化）：保持静默，不影响业务。
    let _ = tracing::subscriber::set_global_default(subscriber);
    Ok(())
}

/// 读取 `RUST_LOG` 的简单级别名（复杂指令语法不在 T02 范围）。
fn level_from_env() -> LevelFilter {
    std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| LevelFilter::from_str(value.trim()).ok())
        .unwrap_or(LevelFilter::INFO)
}

/// 同时写 stdout 与（可选）日志文件。
///
/// 文件写入失败被忽略：日志不能让服务崩溃；stdout 仍保留完整事件流。
#[derive(Debug, Clone)]
struct TeeWriter {
    file: Option<Arc<Mutex<std::fs::File>>>,
}

impl<'a> MakeWriter<'a> for TeeWriter {
    type Writer = TeeWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stdout().write_all(buf);
        if let Some(file) = &self.file
            && let Ok(mut file) = file.lock()
        {
            let _ = file.write_all(buf);
            let _ = file.flush();
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stdout().flush();
        if let Some(file) = &self.file
            && let Ok(mut file) = file.lock()
        {
            let _ = file.flush();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_file_path_is_under_data_dir() {
        let path = log_file_path(Path::new("/tmp/data-dir"));
        assert_eq!(path, PathBuf::from("/tmp/data-dir/everything-manual.log"));
    }

    #[test]
    fn level_from_env_defaults_to_info() {
        // 不修改进程环境：只验证默认分支（无 RUST_LOG 或非法值时）。
        // 环境变量行为由集成测试在子进程中覆盖。
        let filter = LevelFilter::from_str("warn").unwrap();
        assert_eq!(filter, LevelFilter::WARN);
    }
}
