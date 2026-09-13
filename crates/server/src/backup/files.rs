//! 备份／导出共用的文件原语：流式哈希、校验复制、目录 fsync。
//!
//! 全部按块处理（默认 128 KiB），**不把文件读进内存**：GLB 上限 150 MiB、
//! PDF 上限 50 MiB（contracts.md §7），单次操作的内存占用与块大小同阶。
//!
//! 落盘顺序与上传路径一致（tmp → 校验 → 原子 rename 的思路不适用于备份，
//! 因为备份目录本身是新的、只被本进程写入）：写文件 → `sync_all`（fsync）→
//! 结束时 `fsync_dir` 父目录，保证崩溃后目录项不丢。

use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::error::BackupError;
use super::zip::Crc32;

/// 单次读写的块大小。
pub const CHUNK_BYTES: usize = 128 * 1024;

/// 一个文件的指纹（一次读盘同时得到 sha256、大小与 CRC32）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFingerprint {
    pub sha256: String,
    pub size: u64,
    pub crc32: u32,
}

/// 流式计算文件的 sha256 / 大小 / CRC32（导出 ZIP 的 local header 需要 CRC，
/// 内容校验需要 sha256——一次读盘同时得到，避免为同一个大文件多次读盘）。
pub async fn fingerprint_file(path: &Path) -> Result<FileFingerprint, BackupError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| BackupError::io(format!("无法读取文件 {}：{error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut crc = Crc32::new();
    let mut size: u64 = 0;
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer).await.map_err(|error| {
            BackupError::io(format!("读取文件失败 {}：{error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        crc.update(&buffer[..read]);
        size += read as u64;
    }
    Ok(FileFingerprint {
        sha256: hex_digest(&hasher.finalize()),
        size,
        crc32: crc.finalize(),
    })
}

/// 复制文件到目标路径（0600 + fsync），同时计算 sha256 与大小。
///
/// 目标路径必须不存在（备份/恢复都只写新目录，绝不覆盖）。
pub async fn copy_file(src: &Path, dst: &Path) -> Result<FileFingerprint, BackupError> {
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            BackupError::io(format!("创建目录失败 {}：{error}", parent.display()))
        })?;
    }
    let mut input = tokio::fs::File::open(src)
        .await
        .map_err(|error| BackupError::io(format!("无法读取文件 {}：{error}", src.display())))?;
    let mut output = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(dst)
        .await
        .map_err(|error| {
            BackupError::io(format!(
                "创建目标文件失败（已存在则不覆盖） {}：{error}",
                dst.display()
            ))
        })?;
    set_owner_only_permissions(dst);

    let mut hasher = Sha256::new();
    let mut crc = Crc32::new();
    let mut size: u64 = 0;
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    loop {
        let read = input.read(&mut buffer).await.map_err(|error| {
            BackupError::io(format!("读取源文件失败 {}：{error}", src.display()))
        })?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).await.map_err(|error| {
            BackupError::io(format!("写入目标文件失败 {}：{error}", dst.display()))
        })?;
        hasher.update(&buffer[..read]);
        crc.update(&buffer[..read]);
        size += read as u64;
    }
    output.flush().await.map_err(|error| {
        BackupError::io(format!("flush 目标文件失败 {}：{error}", dst.display()))
    })?;
    output.sync_all().await.map_err(|error| {
        BackupError::io(format!("fsync 目标文件失败 {}：{error}", dst.display()))
    })?;
    drop(output);
    Ok(FileFingerprint {
        sha256: hex_digest(&hasher.finalize()),
        size,
        crc32: crc.finalize(),
    })
}

/// 写入一个小文件（manifest 等；0600 + fsync；目标必须不存在）。
pub async fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            BackupError::io(format!("创建目录失败 {}：{error}", parent.display()))
        })?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .await
        .map_err(|error| {
            BackupError::io(format!(
                "创建文件失败（已存在则不覆盖） {}：{error}",
                path.display()
            ))
        })?;
    set_owner_only_permissions(path);
    file.write_all(bytes)
        .await
        .map_err(|error| BackupError::io(format!("写入文件失败 {}：{error}", path.display())))?;
    file.flush()
        .await
        .map_err(|error| BackupError::io(format!("flush 文件失败 {}：{error}", path.display())))?;
    file.sync_all()
        .await
        .map_err(|error| BackupError::io(format!("fsync 文件失败 {}：{error}", path.display())))?;
    Ok(())
}

/// 目录 fsync（best effort：失败只记日志——目录项持久性受文件系统影响，
/// 不能因为不支持目录 fsync 就让备份/恢复整体失败；与上传路径同一处理）。
pub fn fsync_dir(dir: &Path) {
    match std::fs::File::open(dir) {
        Ok(handle) => {
            if let Err(error) = handle.sync_all() {
                tracing::warn!(error = %error, dir = %dir.display(), "fsync 目录失败");
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, dir = %dir.display(), "打开目录以 fsync 失败");
        }
    }
}

/// 把新写入的文件收紧到 0600（不依赖 umask；失败被忽略，与 T02/T03 一致）。
pub fn set_owner_only_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// 十六进制小写 sha256。
pub fn hex_digest(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fingerprint_matches_known_sha256_and_crc32() {
        let dir = std::env::temp_dir().join(format!(
            "em-backup-files-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("payload.bin");
        std::fs::write(&path, b"123456789").unwrap();

        let fingerprint = fingerprint_file(&path).await.unwrap();
        assert_eq!(fingerprint.size, 9);
        // sha256("123456789") 的标准测试向量。
        assert_eq!(
            fingerprint.sha256,
            "15e2b0d3c33891ebb0f1ef609ec419420c20e320ce94c65fbc8c3312448eb225"
        );
        // CRC32("123456789") 的标准测试向量（IEEE 802.3）。
        assert_eq!(fingerprint.crc32, 0xCBF4_3926);

        // 复制：目标不存在、内容与指纹一致、目标权限 0600。
        let dst = dir.join("copy.bin");
        let copied = copy_file(&path, &dst).await.unwrap();
        assert_eq!(copied, fingerprint);
        assert_eq!(std::fs::read(&dst).unwrap(), b"123456789");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "备份文件必须是 0600");
        }
        // 目标已存在 → 拒绝覆盖。
        let error = copy_file(&path, &dst).await.unwrap_err();
        assert!(error.to_string().contains("不覆盖"), "{error}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
