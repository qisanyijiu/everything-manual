//! 崩溃残留的按引用扫描／隔离（architecture.md §6）。
//!
//! 崩溃时序（本模块存在的理由）：
//!
//! ```text
//! A. tmp 里写到一半崩溃          → tmp 残留（无元数据，无引用）
//! B. rename 到 blobs/ 后崩溃     → 孤儿 blob 文件（无元数据行）
//! C. 元数据事务提交后崩溃        → 文件 + 元数据都在（正常状态，不能被隔离）
//! D. 库被恢复/回滚到旧状态       → 文件被其他 sha256 行/资产引用（绝不能删）
//! ```
//!
//! 因此扫描只做两件事，且**只移动不删除**：
//! 1. `tmp/*` → `quarantine/`（隔离，保留现场供排障）；
//! 2. `blobs/**` 中**没有被任何 blob 行引用**的文件 → `quarantine/`；
//!    被引用的文件（含 `quarantined`/`missing` 状态的行，以及多资产共享的 blob）一律不动。
//!
//! 同时做状态收敛（不涉及删除）：文件在而状态是 `missing` → 回到 `stored`；
//! 状态是 `stored` 而文件不在 → 标记 `missing`（GET 内容会返回 404，管理员可重新上传）。
//!
//! 调用点：`serve` 启动、取得排他锁并迁移之后、开始监听之前（此时没有在途上传，
//! 因此 tmp 里的任何文件都必然是上次崩溃的残留）。

use std::collections::HashSet;
use std::path::Path;

use manual_core::domain::BlobStorageState;
use sqlx::SqliteConnection;

use crate::storage::repo::blobs as blobs_repo;

use super::blob_store::{BLOBS_DIR_NAME, QUARANTINE_DIR_NAME, TMP_DIR_NAME};
use super::error::AssetError;

/// 一次扫描的结果（写日志用，不包含任何用户数据）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// 被隔离的 tmp 残留文件数。
    pub tmp_quarantined: usize,
    /// 被隔离的无引用 blob 文件数。
    pub blobs_quarantined: usize,
    /// 被扫描确认仍在引用的 blob 文件数。
    pub blobs_kept: usize,
    /// 状态由 `missing` 收敛回 `stored` 的行数（文件其实在）。
    pub blobs_restored: usize,
    /// 状态由 `stored` 标记为 `missing` 的行数（文件不在）。
    pub blobs_marked_missing: usize,
}

impl ScanReport {
    /// 是否有任何动作（日志分级用）。
    pub fn is_empty(&self) -> bool {
        self.tmp_quarantined == 0
            && self.blobs_quarantined == 0
            && self.blobs_restored == 0
            && self.blobs_marked_missing == 0
    }
}

/// 扫描引用、隔离残留文件、收敛 blob 状态。返回动作摘要。
pub async fn scan_and_quarantine(
    conn: &mut SqliteConnection,
    data_dir: &Path,
) -> Result<ScanReport, AssetError> {
    let referenced = blobs_repo::all_sha256(conn).await?;
    let fs_report = scan_filesystem(data_dir, &referenced)?;
    let (blobs_restored, blobs_marked_missing) =
        reconcile_states(conn, &fs_report.present_sha256).await?;
    Ok(ScanReport {
        tmp_quarantined: fs_report.tmp_quarantined,
        blobs_quarantined: fs_report.blobs_quarantined,
        blobs_kept: fs_report.blobs_kept,
        blobs_restored,
        blobs_marked_missing,
    })
}

struct FilesystemReport {
    tmp_quarantined: usize,
    blobs_quarantined: usize,
    blobs_kept: usize,
    /// 磁盘上确实存在的 blob sha256 集合（状态收敛用）。
    present_sha256: HashSet<String>,
}

fn scan_filesystem(
    data_dir: &Path,
    referenced: &HashSet<String>,
) -> Result<FilesystemReport, AssetError> {
    let quarantine_root = data_dir.join(QUARANTINE_DIR_NAME);
    let mut report = FilesystemReport {
        tmp_quarantined: 0,
        blobs_quarantined: 0,
        blobs_kept: 0,
        present_sha256: HashSet::new(),
    };

    // 1) tmp 残留：启动扫描时不存在在途上传，全部隔离。
    let tmp = data_dir.join(TMP_DIR_NAME);
    if let Ok(entries) = std::fs::read_dir(&tmp) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            quarantine_file(&quarantine_root, &path, "tmp")?;
            report.tmp_quarantined += 1;
        }
    }

    // 2) blobs/：按引用保留或隔离。
    let blobs = data_dir.join(BLOBS_DIR_NAME);
    if let Ok(prefixes) = std::fs::read_dir(&blobs) {
        for prefix in prefixes.flatten() {
            let prefix_path = prefix.path();
            if prefix_path.is_dir() {
                let Ok(files) = std::fs::read_dir(&prefix_path) else {
                    continue;
                };
                for file in files.flatten() {
                    let path = file.path();
                    if !path.is_file() {
                        continue;
                    }
                    let name = file.file_name().to_string_lossy().into_owned();
                    if is_sha256(&name) && referenced.contains(&name) {
                        report.blobs_kept += 1;
                        report.present_sha256.insert(name);
                    } else {
                        quarantine_file(&quarantine_root, &path, "blob")?;
                        report.blobs_quarantined += 1;
                    }
                }
            } else if prefix_path.is_file() {
                // blobs/ 根下的散文件：不是合法布局，隔离而不是删除。
                quarantine_file(&quarantine_root, &prefix_path, "blob")?;
                report.blobs_quarantined += 1;
            }
        }
    }

    Ok(report)
}

/// 状态收敛：`missing` ↔ `stored`（`quarantined` 不参与，人工处理）。
async fn reconcile_states(
    conn: &mut SqliteConnection,
    present: &HashSet<String>,
) -> Result<(usize, usize), AssetError> {
    let existing = blobs_repo::all_sha256(conn).await?;
    let mut restored = 0_usize;
    let mut marked_missing = 0_usize;
    for sha256 in &existing {
        let Some(blob) = blobs_repo::get(conn, sha256).await? else {
            continue;
        };
        match blob.storage_state {
            BlobStorageState::Stored if !present.contains(sha256) => {
                blobs_repo::set_storage_state(conn, sha256, BlobStorageState::Missing).await?;
                marked_missing += 1;
            }
            BlobStorageState::Missing if present.contains(sha256) => {
                blobs_repo::set_storage_state(conn, sha256, BlobStorageState::Stored).await?;
                restored += 1;
            }
            _ => {}
        }
    }
    Ok((restored, marked_missing))
}

/// 把文件移动到 `quarantine/`（**不删除**；重名时追加序号）。
fn quarantine_file(quarantine_root: &Path, path: &Path, kind: &str) -> Result<(), AssetError> {
    std::fs::create_dir_all(quarantine_root)
        .map_err(|error| AssetError::io(format!("创建隔离目录失败：{error}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(quarantine_root, std::fs::Permissions::from_mode(0o700));
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_owned());
    let mut target = quarantine_root.join(format!("{kind}-{name}"));
    let mut counter = 1_u32;
    while target.exists() {
        target = quarantine_root.join(format!("{kind}-{counter}-{name}"));
        counter += 1;
    }
    std::fs::rename(path, &target)
        .map_err(|error| AssetError::io(format!("隔离文件失败：{error}")))?;
    Ok(())
}

fn is_sha256(name: &str) -> bool {
    name.len() == 64
        && name
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_shape_predicate() {
        assert!(is_sha256(&"a".repeat(64)));
        assert!(!is_sha256(&"a".repeat(63)));
        assert!(!is_sha256(&"A".repeat(64)), "只接受小写十六进制");
        assert!(!is_sha256("not-a-hash"));
    }

    #[test]
    fn scan_report_emptiness() {
        assert!(ScanReport::default().is_empty());
        let report = ScanReport {
            tmp_quarantined: 1,
            ..Default::default()
        };
        assert!(!report.is_empty());
    }
}
