//! 管理员密码输入（PRD REQ-002 / §5.1：无默认密码，密码不进命令行与 shell history）。
//!
//! 两条合法输入路径：
//! 1. **交互终端**：不回显读取两次并要求一致（`init` 手工初始化）；
//! 2. **受限文件**：`--password-file <path>`，文件权限必须为 0600（组/其他不可读），
//!    供无人值守初始化使用。
//!
//! 密码一律以 [`SecretString`] 包装，T02 只做读取与校验；Argon2 哈希与入库（T04）
//! 尚未实现，因此当前版本不会把密码写入磁盘（见 `commands.rs::run_init` 的输出说明）。

use std::io::{IsTerminal, Write};
use std::path::Path;

use super::error::CliError;
use super::secret::SecretString;

/// 密码最小长度（字符）。PRD 未规定下限，此处取 8 作为安全默认并在 implementation.md 记录。
pub const PASSWORD_MIN_CHARS: usize = 8;
/// 密码最大长度（字符），防止极端输入耗尽 KDF 以外的内存/时间预算。
pub const PASSWORD_MAX_CHARS: usize = 1024;

/// 读取管理员密码：优先受限文件，其次交互终端。
pub fn read_password(password_file: Option<&Path>) -> Result<SecretString, CliError> {
    match password_file {
        Some(path) => read_from_file(path),
        None => read_interactive(),
    }
}

/// 读取受限密钥/密码文件：必须是普通文件、组/其他不可读（Unix），内容非空。
pub fn read_restricted_file(path: &Path, what: &str) -> Result<SecretString, CliError> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| CliError::config(format!("无法读取{what} {}：{error}", path.display())))?;
    if !metadata.is_file() {
        return Err(CliError::config(format!(
            "{what}不是普通文件：{}",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(CliError::config(format!(
                "{what}权限过宽（{mode:o}）：{}；请执行 chmod 600 {} 后重试",
                path.display(),
                path.display()
            )));
        }
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|error| CliError::config(format!("无法读取{what} {}：{error}", path.display())))?;
    let trimmed = raw.trim_end_matches(['\n', '\r']);
    if trimmed.is_empty() {
        return Err(CliError::config(format!("{what}为空：{}", path.display())));
    }
    Ok(SecretString::new(trimmed.to_owned()))
}

fn read_from_file(path: &Path) -> Result<SecretString, CliError> {
    let secret = read_restricted_file(path, "密码文件")?;
    validate(secret)
}

fn read_interactive() -> Result<SecretString, CliError> {
    if !std::io::stdin().is_terminal() {
        return Err(CliError::usage(
            "标准输入不是终端（非交互环境）；无人值守初始化请使用 --password-file <受限文件>",
        ));
    }
    let first = read_hidden_line("请输入管理员密码（不回显，至少 8 个字符）：")?;
    let second = read_hidden_line("请再次输入密码以确认：")?;
    if first.expose() != second.expose() {
        return Err(CliError::usage("两次输入的密码不一致，未做任何初始化"));
    }
    validate(first)
}

/// 校验密码强度（当前规则：非空、8–1024 字符、不含 NUL）。
pub fn validate(secret: SecretString) -> Result<SecretString, CliError> {
    let value = secret.expose();
    if value.trim().is_empty() {
        return Err(CliError::usage("密码不能为空或全为空白字符"));
    }
    let count = secret.char_count();
    if count < PASSWORD_MIN_CHARS {
        return Err(CliError::usage(format!(
            "密码至少需要 {PASSWORD_MIN_CHARS} 个字符（当前 {count} 个）"
        )));
    }
    if count > PASSWORD_MAX_CHARS {
        return Err(CliError::usage(format!(
            "密码过长（上限 {PASSWORD_MAX_CHARS} 个字符）"
        )));
    }
    if value.contains('\0') {
        return Err(CliError::usage("密码不能包含 NUL 字符"));
    }
    Ok(secret)
}

/// 不回显地读取一行（提示写 stderr，避免污染 stdout 的结构化日志流）。
#[cfg(unix)]
fn read_hidden_line(prompt: &str) -> Result<SecretString, CliError> {
    use std::io::BufRead;

    eprint!("{prompt}");
    let _ = std::io::stderr().flush();

    let fd = libc::STDIN_FILENO;
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    let have_termios = unsafe { libc::tcgetattr(fd, &mut termios) } == 0;
    let original = termios;

    struct EchoGuard {
        fd: libc::c_int,
        original: libc::termios,
        active: bool,
    }
    impl Drop for EchoGuard {
        fn drop(&mut self) {
            if self.active {
                unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.original) };
            }
        }
    }

    let mut guard = EchoGuard {
        fd,
        original,
        active: false,
    };
    if have_termios {
        termios.c_lflag &= !libc::ECHO;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } == 0 {
            guard.active = true;
        }
    }

    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    drop(guard); // 恢复回显
    eprintln!();

    read.map_err(|error| CliError::usage(format!("读取密码失败：{error}")))?;
    Ok(SecretString::new(
        line.trim_end_matches(['\n', '\r']).to_owned(),
    ))
}

/// 非 Unix 平台暂不支持交互式不回显输入；使用 `--password-file`。
#[cfg(not(unix))]
fn read_hidden_line(_prompt: &str) -> Result<SecretString, CliError> {
    Err(CliError::usage(
        "当前平台不支持交互式密码输入；请使用 --password-file <受限文件>",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "em-password-test-{}-{name}-{nanos}",
            std::process::id()
        ))
    }

    fn write_file(path: &Path, content: &str) {
        std::fs::write(path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[test]
    fn accepts_restricted_file_and_trims_trailing_newline() {
        let path = temp_file("ok");
        write_file(&path, "s3cret-pass\n");
        let secret = read_password(Some(&path)).unwrap();
        assert_eq!(secret.expose(), "s3cret-pass");
        std::fs::remove_file(&path).ok();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_world_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_file("mode");
        std::fs::write(&path, "s3cret-pass\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = read_password(Some(&path)).unwrap_err();
        assert!(error.message.contains("chmod 600"), "{}", error.message);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_missing_empty_and_short_passwords() {
        let missing = temp_file("missing");
        assert!(read_password(Some(&missing)).is_err());

        let empty = temp_file("empty");
        write_file(&empty, "\n");
        assert!(
            read_password(Some(&empty))
                .unwrap_err()
                .message
                .contains("为空")
        );

        let short = temp_file("short");
        write_file(&short, "short\n");
        let error = read_password(Some(&short)).unwrap_err();
        assert!(error.message.contains("至少"), "{}", error.message);
        std::fs::remove_file(&empty).ok();
        std::fs::remove_file(&short).ok();
    }

    #[test]
    fn validation_boundaries() {
        assert!(validate(SecretString::new("1234567")).is_err());
        assert!(validate(SecretString::new("12345678")).is_ok());
        assert!(validate(SecretString::new("   ")).is_err());
        assert!(validate(SecretString::new("a".repeat(PASSWORD_MAX_CHARS + 1))).is_err());
        assert!(validate(SecretString::new("ok-pass\u{0}tail")).is_err());
    }
}
