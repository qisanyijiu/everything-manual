//! `blobs` 表的仓储原语（T06）：内容寻址存储的元数据（sha256 唯一）。
//!
//! **边界**：这里只有持久化原语（插入/读取/状态收敛/引用集合），不做体积校验、
//! 不写文件、不判断用途配额——那些在 `crate::assets` 服务层。
//!
//! 关键语义（contracts.md §2）：
//! - `sha256` 是主键：同一内容只有一行，多个 asset 共享同一 blob；
//! - `storage_state` ∈ stored/quarantined/missing：`missing` 表示元数据在、文件不在；
//! - 插入是幂等的（`ON CONFLICT DO NOTHING`），重复上传不会重复写元数据。

use std::collections::HashSet;

use sqlx::{Row, SqliteConnection};

use manual_core::domain::{Blob, BlobStorageState};
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

const SELECT_BLOB_SQL: &str =
    "SELECT sha256, size, mime, storage_state, created_at FROM blobs WHERE sha256 = ?";

/// 幂等插入 blob 元数据（已存在则不动，返回 false 表示已存在）。
pub async fn insert_if_absent(
    conn: &mut SqliteConnection,
    sha256: &str,
    size: u64,
    mime: &str,
) -> Result<bool, StorageError> {
    let affected = sqlx::query(
        "INSERT INTO blobs (sha256, size, mime, storage_state, created_at) \
         VALUES (?, ?, ?, 'stored', ?) \
         ON CONFLICT (sha256) DO NOTHING",
    )
    .bind(sha256)
    .bind(size as i64)
    .bind(mime)
    .bind(Timestamp::now().as_millis())
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// 按 sha256 读取 blob；不存在返回 `Ok(None)`。
pub async fn get(conn: &mut SqliteConnection, sha256: &str) -> Result<Option<Blob>, StorageError> {
    let row = sqlx::query(SELECT_BLOB_SQL)
        .bind(sha256)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| blob_from_row(&row)).transpose()
}

/// 收敛存储状态（`missing` ↔ `stored`；**不**动 `quarantined`）。
pub async fn set_storage_state(
    conn: &mut SqliteConnection,
    sha256: &str,
    state: BlobStorageState,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE blobs SET storage_state = ? WHERE sha256 = ?")
        .bind(state.as_str())
        .bind(sha256)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// 全部 blob 的 sha256 集合（崩溃残留扫描/隔离用：判断文件是否被引用）。
///
/// 注意：这是"元数据在库"的全集，与 `storage_state` 无关——被隔离或标记 missing 的行
/// 同样算引用，扫描不会因为状态而误隔离仍在引用的文件。
pub async fn all_sha256(conn: &mut SqliteConnection) -> Result<HashSet<String>, StorageError> {
    let rows = sqlx::query("SELECT sha256 FROM blobs")
        .fetch_all(&mut *conn)
        .await?;
    rows.iter()
        .map(|row| {
            row.try_get::<String, _>("sha256")
                .map_err(StorageError::from)
        })
        .collect()
}

fn blob_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Blob, StorageError> {
    let created_at: i64 = row.try_get("created_at")?;
    let state: String = row.try_get("storage_state")?;
    Ok(Blob {
        sha256: row.try_get("sha256")?,
        size: row.try_get("size")?,
        mime: row.try_get("mime")?,
        storage_state: parse_storage_state(&state)?,
        created_at: Timestamp::from_millis(created_at),
    })
}

/// SQL 值（snake_case）→ 领域枚举；未知值视为数据损坏而不是静默兜底。
pub fn parse_storage_state(value: &str) -> Result<BlobStorageState, StorageError> {
    match value {
        "stored" => Ok(BlobStorageState::Stored),
        "quarantined" => Ok(BlobStorageState::Quarantined),
        "missing" => Ok(BlobStorageState::Missing),
        other => Err(StorageError::ConstraintViolation {
            detail: format!("blobs.storage_state 出现未知值：{other}"),
        }),
    }
}
