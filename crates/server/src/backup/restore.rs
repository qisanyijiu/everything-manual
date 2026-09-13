//! 备份恢复（T20 / REQ-005、AC-010；architecture.md §7、validation-release.md §5）。
//!
//! 顺序是**先全量校验、再写入**（"恢复到新空目录后校验引用与哈希再使用"）：
//!
//! 1. 目标目录必须**不存在或为空**（非空拒绝，绝不合并/覆盖既有 data-dir）；
//! 2. 读 `manifest.json`：格式版本、相对路径安全性（zip-slip 同口径的防穿越检查）；
//! 3. 校验快照与**每一个** blob：文件存在、大小一致、sha256 一致（损坏 → 退出码 7）；
//! 4. 打开快照（只读）：schema 门禁（备份比程序新 → 拒绝）、`PRAGMA foreign_key_check`
//!    （外键完整性）、每个 `blobs` 行都在 manifest 中被登记（引用完整性）；
//! 5. 只有以上全部通过，才创建目标 data-dir 结构与排他锁，并复制快照与 blob
//!    （复制过程中再次计算 sha256，磁盘写坏不静默）；
//! 6. 恢复后再校验一次（外键 + 每个被引用 blob 都有文件），最后才报告成功。
//!
//! **失败保留现场**：第 1–4 步失败时目标目录尚未被创建；第 5 步之后的失败保留
//! 已写入的部分（不删除），错误消息说明"目标目录处于未完成状态，请清理后重试"。
//! 源备份目录**始终只读**，任何失败都不会修改它。
//!
//! **回滚语义**：恢复的是备份里的 schema 版本；程序回滚不等于数据库回滚
//! （旧程序不能读取比它新的 schema，升级前必须备份——见 `backup` 的 manifest notes）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use sqlx::SqliteConnection;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Row};

use crate::config::datadir;
use crate::storage::migrations;

use super::error::BackupError;
use super::files::{copy_file, fingerprint_file, fsync_dir};
use super::manifest::{
    BACKUP_BLOBS_DIR, BACKUP_DATABASE_RELATIVE, BACKUP_MANIFEST_FILE, BackupCounts, BackupManifest,
};

/// 恢复完成的摘要（CLI 输出与日志用）。
#[derive(Debug, Clone)]
pub struct RestoreOutcome {
    pub data_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub database_sha256: String,
    pub schema_version: i64,
    pub blobs_restored: usize,
    pub missing_blobs: usize,
    pub counts: BackupCounts,
}

/// 从备份目录恢复到一个**不存在或为空**的目标目录。
pub async fn restore_backup(
    backup_dir: &Path,
    target: &Path,
) -> Result<RestoreOutcome, BackupError> {
    // ---- 1) 备份目录与 manifest -------------------------------------------
    if !backup_dir.is_dir() {
        return Err(BackupError::path(format!(
            "备份目录不存在或不是目录：{}（--from）",
            backup_dir.display()
        )));
    }
    if paths_overlap(backup_dir, target) {
        return Err(BackupError::path(format!(
            "恢复目标不能与备份目录互相嵌套（备份 {}，目标 {}）",
            backup_dir.display(),
            target.display()
        )));
    }
    let manifest_path = backup_dir.join(BACKUP_MANIFEST_FILE);
    let manifest_bytes = tokio::fs::read(&manifest_path).await.map_err(|error| {
        BackupError::integrity(
            "backup_manifest_missing",
            format!("备份缺少 manifest（{}）：{error}", manifest_path.display()),
        )
    })?;
    let manifest = BackupManifest::parse(&manifest_bytes)?;
    manifest.validate_relative_paths()?;

    // ---- 2) 目标前置条件（任何写入之前）-----------------------------------
    if target.exists() {
        if !target.is_dir() {
            return Err(BackupError::path(format!(
                "恢复目标已存在且不是目录：{}；请指定不存在或为空的目标",
                target.display()
            )));
        }
        let mut entries = tokio::fs::read_dir(target).await.map_err(|error| {
            BackupError::path(format!(
                "无法读取恢复目标目录（{}）：{error}",
                target.display()
            ))
        })?;
        if entries
            .next_entry()
            .await
            .map_err(|error| BackupError::io(format!("读取目标目录失败：{error}")))?
            .is_some()
        {
            return Err(BackupError::path(format!(
                "恢复目标已存在且非空：{}；恢复只允许写入不存在或为空的目录（不合并、不覆盖）",
                target.display()
            )));
        }
    }

    // ---- 3) 全量校验（源只读；失败时目标尚未创建）--------------------------
    let snapshot_path = backup_dir.join(BACKUP_DATABASE_RELATIVE);
    verify_file(
        &snapshot_path,
        &manifest.database.sha256,
        manifest.database.size,
        "backup_database_corrupt",
        "备份快照数据库",
    )
    .await?;
    for blob in &manifest.blobs {
        let path = backup_dir.join(&blob.path);
        verify_file(
            &path,
            &blob.sha256,
            blob.size,
            "backup_blob_corrupt",
            "备份中的 blob",
        )
        .await?;
    }

    // 备份单文件约定：快照不应带 -wal/-shm 边车（本程序的备份会把日志模式归一到
    // DELETE）。边车存在说明备份是"只复制了主文件"或被人为拼凑——WAL 里可能还有
    // 未落盘的已提交事务，绝不能当作完整快照使用。
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = snapshot_path.as_os_str().to_os_string();
        sidecar.push(suffix);
        if Path::new(&sidecar).exists() {
            return Err(BackupError::integrity(
                "backup_database_sidecar",
                format!(
                    "备份快照带有 WAL 边车文件（{}）：快照不完整（不能只复制运行中 WAL 数据库的主文件），\
                     请重新备份",
                    Path::new(&sidecar).display()
                ),
            ));
        }
    }

    // ---- 4) 快照内部校验（schema 门禁 + 外键 + 引用）------------------------
    let snapshot_report = inspect_snapshot(&snapshot_path, &manifest).await?;

    // ---- 5) 写入目标 -------------------------------------------------------
    datadir::ensure_initialized(target).map_err(|error| BackupError::path(error.message))?;
    let lock = datadir::DirLock::acquire(target).map_err(|error| {
        if error.exit_code == crate::config::ExitCode::Locked {
            BackupError::locked(format!(
                "恢复目标 {} 正被另一个进程持有排他锁：{}；请先停止该进程",
                target.display(),
                error.message
            ))
        } else {
            BackupError::path(error.message)
        }
    })?;

    let destination_snapshot = crate::storage::database_path(target);
    let copied = copy_file(&snapshot_path, &destination_snapshot)
        .await
        .map_err(leave_behind)?;
    if copied.sha256 != manifest.database.sha256 {
        return Err(leave_behind(BackupError::io(format!(
            "复制快照后 sha256 与备份不符（期望 {expected}，实际 {actual}）",
            expected = manifest.database.sha256,
            actual = copied.sha256
        ))));
    }

    let mut blobs_restored = 0_usize;
    for blob in &manifest.blobs {
        let relative = Path::new(&blob.path);
        let file_name = relative
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let destination = target
            .join(BACKUP_BLOBS_DIR)
            .join(&blob.sha256[..2])
            .join(file_name);
        let copied = copy_file(&backup_dir.join(&blob.path), &destination)
            .await
            .map_err(leave_behind)?;
        if copied.sha256 != blob.sha256 {
            return Err(leave_behind(BackupError::io(format!(
                "恢复 blob 时 sha256 不符（期望 {expected}，实际 {actual}）",
                expected = blob.sha256,
                actual = copied.sha256
            ))));
        }
        blobs_restored += 1;
    }
    fsync_dir(&target.join(BACKUP_BLOBS_DIR));
    fsync_dir(target);

    // ---- 6) 恢复后复检（引用与哈希）----------------------------------------
    let missing: HashSet<String> = manifest
        .missing_blobs
        .iter()
        .map(|entry| entry.sha256.clone())
        .collect();
    verify_restored_database(target, &missing).await?;

    let outcome = RestoreOutcome {
        data_dir: target.to_path_buf(),
        backup_dir: backup_dir.to_path_buf(),
        database_sha256: snapshot_report.database_sha256.clone(),
        schema_version: snapshot_report.schema_version,
        blobs_restored,
        missing_blobs: manifest.missing_blobs.len(),
        counts: snapshot_report.counts,
    };
    drop(lock);
    tracing::info!(
        event = "restore_completed",
        dataDir = %target.display(),
        backupDir = %backup_dir.display(),
        schemaVersion = outcome.schema_version,
        databaseSha256 = %outcome.database_sha256,
        blobsRestored = outcome.blobs_restored,
        missingBlobs = outcome.missing_blobs,
        "恢复完成（校验 hash/外键/引用后写入新目录）"
    );
    Ok(outcome)
}

/// 恢复过程的失败：保留已写入的目标目录，提示清理后再试。
fn leave_behind(error: BackupError) -> BackupError {
    match error {
        BackupError::Io { message } | BackupError::Path { message } => BackupError::Io {
            message: format!(
                "{message}；目标目录保留了未完成状态（保留现场，不删除），请检查磁盘后清理该目录并重试"
            ),
        },
        other => other,
    }
}

/// 校验一个备份文件：存在 + 大小 + sha256。
async fn verify_file(
    path: &Path,
    expected_sha256: &str,
    expected_size: u64,
    code: &'static str,
    label: &str,
) -> Result<(), BackupError> {
    // 拒绝符号链接：备份目录可能被人为改动，不能让"备份里的一个链接"把
    // 目标目录外的文件复制进恢复结果（与将来 ZIP 导入的 zip-slip 同一思路）。
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(BackupError::integrity(
                code,
                format!(
                    "{label}是符号链接（备份内容不可信）：{}；未创建/修改任何目标目录",
                    path.display()
                ),
            ));
        }
        Ok(_) => {}
        Err(_) => {
            return Err(BackupError::integrity(
                code,
                format!(
                    "{label}缺失或不可读：{}（备份不完整；未创建/修改任何目标目录）",
                    path.display()
                ),
            ));
        }
    }
    let fingerprint = match fingerprint_file(path).await {
        Ok(fingerprint) => fingerprint,
        Err(_) => {
            return Err(BackupError::integrity(
                code,
                format!(
                    "{label}缺失或不可读：{}（备份不完整；未创建/修改任何目标目录）",
                    path.display()
                ),
            ));
        }
    };
    if fingerprint.sha256 != expected_sha256 {
        return Err(BackupError::integrity(
            code,
            format!(
                "{label}的 sha256 与 manifest 不符：{}\n  期望：{expected_sha256}\n  实际：{actual}\n\
                 备份已损坏（未创建/修改任何目标目录，保留现场）",
                path.display(),
                actual = fingerprint.sha256
            ),
        ));
    }
    if fingerprint.size != expected_size {
        return Err(BackupError::integrity(
            code,
            format!(
                "{label}的大小与 manifest 不符（期望 {expected_size} 字节，实际 {size} 字节）：{}",
                path.display(),
                size = fingerprint.size
            ),
        ));
    }
    Ok(())
}

/// 快照只读检查的结果。
struct SnapshotReport {
    schema_version: i64,
    database_sha256: String,
    counts: BackupCounts,
}

/// 打开备份快照（只读）：schema 门禁、外键检查、引用完整性（每个 blob 行都要在
/// manifest 中登记——备份缺一个被引用 blob 就不算完整备份）。
async fn inspect_snapshot(
    snapshot: &Path,
    manifest: &BackupManifest,
) -> Result<SnapshotReport, BackupError> {
    let options = SqliteConnectOptions::new()
        .filename(snapshot)
        .create_if_missing(false)
        .read_only(true);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|error| {
            BackupError::integrity(
                "backup_database_unreadable",
                format!("备份快照数据库无法打开：{error}"),
            )
        })?;

    let result = async {
        let schema_version = migrations::applied_schema_version_conn(&mut connection).await?;
        let program = migrations::program_schema_version();
        if schema_version > program {
            // 与 check/serve/init 同一语义：库比程序新一律拒绝打开（退出码 4）。
            return Err(BackupError::path(format!(
                "备份的 schema 版本（v{schema_version}）比程序支持的版本（v{program}）新：\
                 请使用创建该备份的（或更新的）程序恢复；本程序不会打开比它新的数据库"
            )));
        }

        foreign_key_check(&mut connection).await?;

        let registered: HashSet<&str> = manifest
            .blobs
            .iter()
            .map(|blob| blob.sha256.as_str())
            .chain(
                manifest
                    .missing_blobs
                    .iter()
                    .map(|blob| blob.sha256.as_str()),
            )
            .collect();
        let rows = sqlx::query("SELECT sha256 FROM blobs ORDER BY sha256")
            .fetch_all(&mut connection)
            .await
            .map_err(BackupError::from)?;
        for row in rows {
            let sha256: String = row.try_get("sha256")?;
            if !registered.contains(sha256.as_str()) {
                return Err(BackupError::integrity(
                    "backup_referenced_blob_missing",
                    format!(
                        "备份缺少被引用的 blob：{sha256}（manifest 未登记该内容）；\
                         备份不完整，未创建/修改任何目标目录"
                    ),
                ));
            }
        }

        let items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items")
            .fetch_one(&mut connection)
            .await?;
        let assets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assets")
            .fetch_one(&mut connection)
            .await?;
        let releases: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM manual_releases")
            .fetch_one(&mut connection)
            .await?;
        let blobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
            .fetch_one(&mut connection)
            .await?;

        Ok(SnapshotReport {
            schema_version,
            database_sha256: manifest.database.sha256.clone(),
            counts: BackupCounts {
                items,
                assets,
                releases,
                blobs,
            },
        })
    }
    .await;
    connection.close().await.ok();
    result
}

/// `PRAGMA foreign_key_check`：任何一行都意味着外键完整性被破坏。
async fn foreign_key_check(connection: &mut SqliteConnection) -> Result<(), BackupError> {
    let rows = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *connection)
        .await
        .map_err(BackupError::from)?;
    if rows.is_empty() {
        return Ok(());
    }
    let detail: Vec<String> = rows
        .iter()
        .take(5)
        .map(|row| {
            let table: String = row.try_get("table").unwrap_or_default();
            let rowid: i64 = row.try_get("rowid").unwrap_or_default();
            let parent: String = row.try_get("parent").unwrap_or_default();
            format!("表 {table} 行 {rowid} 引用 {parent}")
        })
        .collect();
    Err(BackupError::integrity(
        "backup_foreign_key_violation",
        format!(
            "备份数据库存在 {} 处外键完整性问题：{}（备份已损坏或来源数据库本身损坏）",
            rows.len(),
            detail.join("；")
        ),
    ))
}

/// 恢复后复检：库可打开、外键完整、每个被引用 blob 都有文件（缺失清单内的除外）。
async fn verify_restored_database(
    data_dir: &Path,
    missing: &HashSet<String>,
) -> Result<(), BackupError> {
    let options = SqliteConnectOptions::new()
        .filename(crate::storage::database_path(data_dir))
        .create_if_missing(false)
        .read_only(true);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|error| {
            BackupError::io(format!(
                "恢复后无法打开数据库（{}）：{error}；目标目录保留现场",
                data_dir.display()
            ))
        })?;

    let result = async {
        foreign_key_check(&mut connection).await?;
        let rows = sqlx::query("SELECT sha256 FROM blobs ORDER BY sha256")
            .fetch_all(&mut connection)
            .await?;
        for row in rows {
            let sha256: String = row.try_get("sha256")?;
            if missing.contains(&sha256) {
                continue;
            }
            let path = crate::assets::blob_store::blob_path(data_dir, &sha256);
            if !path.is_file() {
                return Err(BackupError::integrity(
                    "restore_blob_missing_after_copy",
                    format!(
                        "恢复后校验失败：blob {} 的文件不在目标目录（{}）",
                        sha256,
                        path.display()
                    ),
                ));
            }
        }
        Ok(())
    }
    .await;
    connection.close().await.ok();
    result
}

/// 恢复目标与备份目录是否互相嵌套。
fn paths_overlap(backup_dir: &Path, target: &Path) -> bool {
    target.starts_with(backup_dir) || backup_dir.starts_with(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_detection_covers_nesting_both_ways() {
        let backup = Path::new("/srv/em/backups/b1");
        assert!(paths_overlap(backup, Path::new("/srv/em/backups")));
        assert!(paths_overlap(
            backup,
            Path::new("/srv/em/backups/b1/restored")
        ));
        assert!(!paths_overlap(backup, Path::new("/srv/em/data")));
    }
}
