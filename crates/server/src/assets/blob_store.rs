//! blob 文件布局、流式暂存（tmp）、原子落盘与剩余空间检查。
//!
//! 布局（architecture.md §6）：
//!
//! ```text
//! <data-dir>/blobs/<sha256 前 2 位>/<sha256>     最终内容寻址位置（一个内容一个文件）
//! <data-dir>/tmp/<uuid>.part                     上传中的暂存文件（崩溃残留由扫描隔离）
//! <data-dir>/quarantine/…                        隔离区（只移动不删除，保留现场）
//! ```
//!
//! 落盘顺序：流式写 tmp（同时计数与哈希）→ `flush` + `sync_all`（文件 fsync）→
//! 创建目标目录 → `rename`（同文件系统内原子）→ fsync 目标目录（保证目录项持久）。
//! 只有在 rename 之后才允许提交元数据；反过来，元数据提交失败**不删除**已落盘文件
//! （可能是被其他 asset 引用的共享 blob），由引用扫描隔离孤儿。

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::error::AssetError;

/// 内容寻址目录名（architecture.md §6）。
pub const BLOBS_DIR_NAME: &str = "blobs";
/// 上传暂存目录名（`init` 已创建，T02 `SUBDIRS`）。
pub const TMP_DIR_NAME: &str = "tmp";
/// 隔离目录名（按需创建，不进 `init` 的必需结构：`verify` 只校验 T02 的 SUBDIRS）。
pub const QUARANTINE_DIR_NAME: &str = "quarantine";

/// 文件内容的最终路径：`blobs/<sha256 前 2 位>/<sha256>`。
///
/// 路径**只**由哈希拼出：任何用户提供的文件名都不会进入这里（REQ-011）。
pub fn blob_path(data_dir: &Path, sha256: &str) -> PathBuf {
    let prefix = &sha256[..2];
    data_dir.join(BLOBS_DIR_NAME).join(prefix).join(sha256)
}

/// tmp 目录。
pub fn tmp_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(TMP_DIR_NAME)
}

/// 隔离目录。
pub fn quarantine_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(QUARANTINE_DIR_NAME)
}

/// 剩余空间探测（可注入：磁盘满场景必须可测，不能靠真的写满磁盘）。
#[derive(Debug, Clone)]
pub enum SpaceProbe {
    /// 生产默认：`statvfs` 读取文件系统可用字节（非 Unix 平台退化为"跳过检查"，见下）。
    Statvfs,
    /// 测试用固定值：`available_bytes` 恒返回该值（0 = 磁盘已满）。
    Fixed(u64),
    /// 测试用序列：每次探测弹出一个值（弹空后保持最后一个），用于区分
    /// "解析前预检"与"落盘前复检"两个检查点。
    Scripted(std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<u64>>>),
}

impl SpaceProbe {
    /// 构造脚本化探测（至少给出一个值；最后一个值会一直重复）。
    pub fn scripted(values: impl IntoIterator<Item = u64>) -> Self {
        let mut queue: std::collections::VecDeque<u64> = values.into_iter().collect();
        if queue.is_empty() {
            queue.push_back(0);
        }
        Self::Scripted(std::sync::Arc::new(std::sync::Mutex::new(queue)))
    }

    /// 目标路径所在文件系统的可用字节。
    ///
    /// 非 Unix 平台没有 `statvfs` 实现：返回 `None` 表示"无法判定"，
    /// 调用方跳过预留检查而不是假装有空间（本项目发布平台为 macOS/Linux，见 PRD §5.6）。
    pub fn available_bytes(&self, path: &Path) -> std::io::Result<Option<u64>> {
        match self {
            Self::Fixed(bytes) => Ok(Some(*bytes)),
            Self::Scripted(queue) => {
                let mut queue = queue.lock().unwrap_or_else(|error| error.into_inner());
                if queue.len() > 1 {
                    Ok(Some(queue.pop_front().unwrap_or(0)))
                } else {
                    Ok(queue.front().copied())
                }
            }
            Self::Statvfs => statvfs_available(path),
        }
    }
}

#[cfg(unix)]
fn statvfs_available(path: &Path) -> std::io::Result<Option<u64>> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "路径包含 NUL 字节，无法探测剩余空间",
        )
    })?;
    // SAFETY: `statvfs` 只写传入的 `statvfs` 结构体；路径是合法的 C 字符串，调用期间有效。
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // f_bavail（非特权用户可用块）× f_frsize，而不是 f_blocks（含 root 预留）。
    Ok(Some(stats.f_bavail as u64 * stats.f_frsize as u64))
}

#[cfg(not(unix))]
fn statvfs_available(_path: &Path) -> std::io::Result<Option<u64>> {
    Ok(None)
}

/// 剩余空间不足时返回的人类可读提示（不含磁盘路径以外的信息）。
pub fn insufficient_space_message(required: u64, available: u64) -> String {
    format!(
        "磁盘可用空间不足：本次上传至少需要 {required} 字节，当前可用 {available} 字节；\
         请清理磁盘后重试（未保存任何资产）"
    )
}

/// 已写满并 fsync 的暂存文件（尚未 rename 到 blobs/）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    /// tmp 中的路径。
    pub path: PathBuf,
    /// 内容的 sha256（小写十六进制）。
    pub sha256: String,
    /// 字节数（读流时累计，与哈希同时计算）。
    pub size: u64,
}

/// 暂存文件的清理守卫：只要 [`StagedWriter`] 在写入阶段被丢弃（含 `?` 提前返回、
/// 读取中断、panic），tmp 文件都会被删掉，不留半文件。
#[derive(Debug)]
struct TmpFileGuard {
    path: Option<PathBuf>,
}

impl TmpFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("守卫仍持有路径")
    }

    fn defuse(&mut self) -> PathBuf {
        self.path.take().expect("守卫仍持有路径")
    }
}

impl Drop for TmpFileGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.path
            && let Err(error) = std::fs::remove_file(path)
        {
            tracing::warn!(error = %error, "清理上传暂存文件失败（将由启动扫描隔离）");
        }
    }
}

/// 一次上传的暂存写入器：流式落盘 + 计数 + sha256。
///
/// **不缓存整个文件**：每次 [`Self::write`] 只处理当前 chunk，内存占用与 chunk 同阶。
#[derive(Debug)]
pub struct StagedWriter {
    file: tokio::fs::File,
    guard: TmpFileGuard,
    hasher: Sha256,
    written: u64,
    limit: u64,
}

impl StagedWriter {
    /// 在 `<data-dir>/tmp/` 中创建暂存文件；`limit` 是本用途允许的最大字节数。
    pub async fn create(
        tmp_dir: &Path,
        limit: u64,
        purpose_label: &str,
    ) -> Result<Self, AssetError> {
        tokio::fs::create_dir_all(tmp_dir)
            .await
            .map_err(|error| AssetError::io(format!("创建临时目录失败：{error}")))?;
        let path = tmp_dir.join(format!("{}.part", uuid::Uuid::now_v7()));
        let file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .await
            .map_err(|error| AssetError::io(format!("创建暂存文件失败：{error}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        tracing::debug!(
            stage = "upload_tmp_created",
            purpose = purpose_label,
            limitBytes = limit,
            "上传暂存文件已创建"
        );
        Ok(Self {
            file,
            guard: TmpFileGuard::new(path),
            hasher: Sha256::new(),
            written: 0,
            limit,
        })
    }

    /// 写入一个 chunk。**边读边再次计数**：HTTP 层体积上限之外的第二道防线
    /// （contracts.md §7），超过本用途上限立即 413，不继续读流。
    pub async fn write(&mut self, chunk: &[u8]) -> Result<(), AssetError> {
        let next = self.written.saturating_add(chunk.len() as u64);
        if next > self.limit {
            return Err(AssetError::payload_too_large(format!(
                "文件超过该用途的大小上限：上限 {} 字节，已接收 {} 字节（未保存任何资产）",
                self.limit, next
            )));
        }
        self.file
            .write_all(chunk)
            .await
            .map_err(|error| AssetError::io(format!("写入暂存文件失败：{error}")))?;
        self.hasher.update(chunk);
        self.written = next;
        Ok(())
    }

    /// 已接收字节数（进度/诊断用）。
    pub fn written(&self) -> u64 {
        self.written
    }

    /// flush + fsync，返回可 rename 的 [`Staged`]（守卫随之解除）。
    pub async fn finish(mut self) -> Result<Staged, AssetError> {
        self.file
            .flush()
            .await
            .map_err(|error| AssetError::io(format!("flush 暂存文件失败：{error}")))?;
        // fsync：确保 rename 之后崩溃也不会得到"有目录项、内容仍在页缓存"的半文件。
        self.file
            .sync_all()
            .await
            .map_err(|error| AssetError::io(format!("fsync 暂存文件失败：{error}")))?;
        let digest = self.hasher.finalize();
        let sha256: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Staged {
            path: self.guard.defuse(),
            sha256,
            size: self.written,
        })
    }

    /// 暂存文件路径（诊断/日志用；不进入任何响应）。
    pub fn path(&self) -> &Path {
        self.guard.path()
    }
}

/// 删除暂存文件（best effort；失败只记日志——隔离扫描会兜底）。
pub async fn discard_staged(staged: Staged) {
    if let Err(error) = tokio::fs::remove_file(&staged.path).await {
        tracing::warn!(
            error = %error,
            "删除上传暂存文件失败（将由启动扫描隔离）"
        );
    }
}

/// 把暂存文件原子移动到内容寻址位置，并 fsync 目标目录。
///
/// 幂等：同 sha256 的重复上传会 rename 覆盖到同一路径，内容按定义完全相同；
/// **不删除**任何既有文件（共享 blob 场景）。
pub async fn promote(staged: &Staged, data_dir: &Path) -> Result<PathBuf, AssetError> {
    let target = blob_path(data_dir, &staged.sha256);
    let parent = target
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| AssetError::io("blob 目标路径缺少父目录".to_owned()))?;
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|error| AssetError::io(format!("创建 blob 目录失败：{error}")))?;
    tokio::fs::rename(&staged.path, &target)
        .await
        .map_err(|error| AssetError::io(format!("原子移动 blob 失败：{error}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600));
    }
    // 目录 fsync：rename 的目录项必须先于元数据事务持久（否则崩溃后 DB 有行、文件不见）。
    sync_dir(&parent);
    sync_dir(&tmp_dir(data_dir));
    Ok(target)
}

/// 对目录调用 fsync（阻塞调用很小，且只在一次上传的收尾发生）。
fn sync_dir(dir: &Path) {
    match std::fs::File::open(dir) {
        Ok(handle) => {
            if let Err(error) = handle.sync_all() {
                tracing::warn!(error = %error, "fsync 目录失败（继续提交元数据）");
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, "打开目录以 fsync 失败（继续提交元数据）");
        }
    }
}

/// 一次上传前的空间预检（`content_length` 已知时用请求声明的体积，未知时用 0 = 只做最低校验）。
pub fn ensure_space(probe: &SpaceProbe, data_dir: &Path, required: u64) -> Result<(), AssetError> {
    match probe.available_bytes(data_dir) {
        Ok(Some(available)) if available >= required => Ok(()),
        Ok(Some(available)) => Err(AssetError::insufficient_storage(required, available)),
        // 无法判定（非 Unix）：不阻塞上传，只在日志留痕。
        Ok(None) => Ok(()),
        Err(error) => Err(AssetError::io(format!(
            "检查剩余空间失败（{}）：{error}",
            data_dir.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_path_is_derived_from_hash_only() {
        let sha = "a".repeat(64);
        let path = blob_path(Path::new("/data"), &sha);
        assert_eq!(path, Path::new("/data/blobs/aa").join(&sha));
    }

    #[test]
    fn space_probe_fixed_is_deterministic() {
        let probe = SpaceProbe::Fixed(1234);
        assert_eq!(probe.available_bytes(Path::new("/")).unwrap(), Some(1234));
    }

    #[cfg(unix)]
    #[test]
    fn statvfs_reports_some_space_for_temp_dir() {
        let probe = SpaceProbe::Statvfs;
        let available = probe.available_bytes(&std::env::temp_dir()).unwrap();
        assert!(
            available.is_some_and(|bytes| bytes > 0),
            "临时目录应有可用空间：{available:?}"
        );
    }

    #[test]
    fn insufficient_space_message_mentions_both_numbers() {
        let message = insufficient_space_message(2048, 100);
        assert!(
            message.contains("2048") && message.contains("100"),
            "{message}"
        );
    }
}
