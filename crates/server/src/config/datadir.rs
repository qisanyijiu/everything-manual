//! data-dir 布局与排他锁（architecture.md §6、PRD A-09）。
//!
//! 布局（T02 建立的部分；`manual.sqlite3` 属 T03）：
//!
//! ```text
//! <data-dir>/
//!   tmp/     上传流临时文件（T06 起使用）
//!   logs/    结构化日志（everything-manual.log，追加写）
//!   blobs/   内容寻址的二进制资产（T06 起使用）
//!   lock     flock 排他锁文件（内容为持有者 pid 与启动时间，仅供诊断）
//! ```
//!
//! 排他锁用 `flock(LOCK_EX|LOCK_NB)`：同一 data-dir 第二个进程**必须失败**而不是静默降级；
//! 锁随进程结束（含 SIGKILL）自动释放，不会留下需要人工清理的陈旧锁。
//! 不支持 NFS/共享盘多实例（A-09），锁语义只在本地文件系统上有保证。
//! 本文件为 Unix 实现；其他平台返回明确错误而不是假装成功。

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use super::error::CliError;

/// 子目录清单（init 创建、check 校验）。
pub const SUBDIRS: [&str; 3] = ["tmp", "logs", "blobs"];
pub const LOCK_FILE_NAME: &str = "lock";

pub fn lock_path(root: &Path) -> PathBuf {
    root.join(LOCK_FILE_NAME)
}

/// 初始化（或补全）data-dir 结构：根目录、子目录、锁文件。幂等，不破坏已有内容。
///
/// 返回本次新建／确认的项，用于运维输出。返回不包含 `manual.sqlite3`：
/// 数据库由 T03 的迁移建立，T02 不创建空库，避免留下无 schema 的假数据库。
pub fn ensure_initialized(root: &Path) -> Result<Vec<String>, CliError> {
    let mut items = Vec::new();
    create_dir(root, &mut items)?;
    for name in SUBDIRS {
        create_dir(&root.join(name), &mut items)?;
    }
    create_lock_file(root, &mut items)?;
    Ok(items)
}

fn create_dir(path: &Path, items: &mut Vec<String>) -> Result<(), CliError> {
    if path.is_dir() {
        items.push(format!("目录已存在：{}", path.display()));
        return Ok(());
    }
    std::fs::create_dir_all(path)
        .map_err(|error| CliError::data_dir(format!("创建目录失败 {}：{error}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // data-dir 与子目录仅所有者可访问（隐私资料；PRD §5.7）。
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| CliError::data_dir(format!("设置目录权限失败 {}：{error}", path.display())),
        )?;
    }
    items.push(format!("已创建：{}", path.display()));
    Ok(())
}

fn create_lock_file(root: &Path, items: &mut Vec<String>) -> Result<(), CliError> {
    let path = lock_path(root);
    if path.is_file() {
        items.push(format!("锁文件已存在：{}", path.display()));
        return Ok(());
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| {
            CliError::data_dir(format!("创建锁文件失败 {}：{error}", path.display()))
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    drop(file);
    items.push(format!("已创建：{}", path.display()));
    Ok(())
}

/// 校验 data-dir 结构（不修改任何内容）。问题以可读文本返回，调用方决定处置。
pub fn verify(root: &Path) -> Result<Vec<String>, CliError> {
    if !root.exists() {
        return Err(CliError::data_dir(format!(
            "data-dir 不存在：{}；请先运行 everything-manual init --data-dir {}",
            root.display(),
            root.display()
        )));
    }
    if !root.is_dir() {
        return Err(CliError::data_dir(format!(
            "data-dir 不是目录：{}",
            root.display()
        )));
    }

    let mut problems = Vec::new();
    let mut ok_items = Vec::new();
    for name in SUBDIRS {
        let path = root.join(name);
        if path.is_dir() {
            ok_items.push(format!("{name}/"));
        } else {
            problems.push(format!("缺少目录 {name}/"));
        }
    }
    if lock_path(root).is_file() {
        ok_items.push(LOCK_FILE_NAME.to_owned());
    } else {
        problems.push(format!("缺少锁文件 {LOCK_FILE_NAME}"));
    }

    if !problems.is_empty() {
        return Err(CliError::data_dir(format!(
            "data-dir 结构不完整（{}）：{}；请运行 everything-manual init --data-dir {} 修复",
            root.display(),
            problems.join("、"),
            root.display()
        )));
    }
    Ok(ok_items)
}

/// 写探针：在 tmp/ 中创建并删除一个临时文件，验证目录可写。
pub fn probe_writable(root: &Path) -> Result<(), CliError> {
    let probe = root
        .join("tmp")
        .join(format!(".write-probe-{}", std::process::id()));
    std::fs::write(&probe, b"ok").map_err(|error| {
        CliError::data_dir(format!("data-dir 不可写（{}）：{error}", probe.display()))
    })?;
    std::fs::remove_file(&probe).map_err(|error| {
        CliError::data_dir(format!("清理写探针失败 {}：{error}", probe.display()))
    })?;
    Ok(())
}

/// data-dir 排他锁。持有期间同一 data-dir 的其他进程无法获取锁（[`ExitCode::Locked`]）。
/// 进程退出（含被 Kill）时由操作系统自动释放。
#[derive(Debug)]
pub struct DirLock {
    path: PathBuf,
    // 保持文件描述符打开 = 保持 flock；字段本身不读取。
    _file: File,
}

impl DirLock {
    pub fn acquire(root: &Path) -> Result<Self, CliError> {
        let path = lock_path(root);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                CliError::data_dir(format!("无法打开锁文件 {}：{error}", path.display()))
            })?;

        lock_file_exclusive(&file, &path)?;
        // 锁已持有：写入持有者信息供诊断（内容不是权威判断依据）。
        write_holder_info(&mut file);

        Ok(Self { path, _file: file })
    }

    /// 锁文件路径（诊断用）。
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// `flock(LOCK_EX|LOCK_NB)`：非阻塞抢锁；被占用时返回 [`CliError::locked`]。
#[cfg(unix)]
fn lock_file_exclusive(file: &File, path: &Path) -> Result<(), CliError> {
    use std::os::unix::io::AsRawFd;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    let holder = std::fs::read_to_string(path).unwrap_or_default();
    let holder = holder.trim();
    let holder_note = if holder.is_empty() {
        String::new()
    } else {
        format!("，持有者信息：{holder}")
    };
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return Err(CliError::locked(format!(
            "data-dir 正被另一个进程使用（排他锁 {} 已被持有{holder_note}）；\
             同一 data-dir 只允许一个进程，请先停止该进程",
            path.display()
        )));
    }
    Err(CliError::data_dir(format!(
        "获取排他锁失败（{}）：{error}（A-09：不支持 NFS/共享盘多实例）",
        path.display()
    )))
}

#[cfg(not(unix))]
fn lock_file_exclusive(_file: &File, _path: &Path) -> Result<(), CliError> {
    Err(CliError::data_dir(
        "当前平台不支持 data-dir 排他锁（T02 仅在 Unix 上实现 flock）；\
         为避免多实例损坏数据，拒绝启动",
    ))
}

#[cfg(unix)]
fn write_holder_info(file: &mut File) {
    use std::io::{Seek, SeekFrom, Write};
    let _ = file.set_len(0);
    let _ = file.seek(SeekFrom::Start(0));
    let pid = std::process::id();
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let _ = writeln!(file, "pid={pid} started_at_unix={started}");
    let _ = file.flush();
}

#[cfg(not(unix))]
fn write_holder_info(_file: &mut File) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "em-datadir-test-{}-{name}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn ensures_and_verifies_layout() {
        let root = temp_root("layout");
        let items = ensure_initialized(&root).unwrap();
        assert!(items.iter().any(|item| item.contains("tmp")));
        for name in SUBDIRS {
            assert!(root.join(name).is_dir(), "{name} 应存在");
        }
        assert!(lock_path(&root).is_file());
        assert!(
            !root.join("manual.sqlite3").exists(),
            "T02 不创建数据库文件（T03 才建库）"
        );

        let ok = verify(&root).unwrap();
        assert_eq!(ok.len(), SUBDIRS.len() + 1);
        probe_writable(&root).unwrap();
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn verify_reports_missing_structure() {
        let root = temp_root("missing");
        let error = verify(&root).unwrap_err();
        assert_eq!(error.exit_code, super::super::ExitCode::DataDir);

        std::fs::create_dir_all(&root).unwrap();
        let error = verify(&root).unwrap_err();
        assert!(error.message.contains("tmp/"), "{}", error.message);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn lock_excludes_second_holder_in_process() {
        // flock 以打开文件描述为单位：同进程再次打开同一文件也会冲突。
        let root = temp_root("lock");
        ensure_initialized(&root).unwrap();
        let first = DirLock::acquire(&root).unwrap();
        let second = DirLock::acquire(&root).unwrap_err();
        assert_eq!(second.exit_code, super::super::ExitCode::Locked);
        assert!(second.message.contains("正被另一个进程使用"));
        drop(first);
        DirLock::acquire(&root).unwrap();
        std::fs::remove_dir_all(&root).ok();
    }
}
