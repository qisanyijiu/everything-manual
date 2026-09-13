//! `idempotency_records` 表的仓储原语（T11 / contracts.md §4）。
//!
//! 语义（contracts.md §4「幂等逻辑」）：
//! - 调用者的一次操作生成并复用 key → 按 `admin + method + route + key` 唯一；
//! - **相同 body_hash** → 返回原资源（重放，不新建）；
//! - **不同 body_hash** → 409（服务层映射 `IDEMPOTENCY_CONFLICT`）；
//! - 重复点击、连接断开、服务器重启不能产生第二份本地生成单；记录至少保留到
//!   相关业务记录删除（本模块不做 TTL）。
//!
//! 唯一键是 SQL 层兜底：两个并发同 key 请求只有一个能插入成功，另一个按
//! `UniqueViolation` 读回已存在记录（服务层处理）。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::IdempotencyRecord;
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 新建幂等记录的输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIdempotencyRecord {
    pub admin_id: String,
    pub method: String,
    /// 路由**模板**（如 `/api/v1/items/{id}/jobs`），不是具体路径。
    pub route: String,
    pub key: String,
    pub body_hash: String,
    pub resource_id: Option<String>,
    pub response_status: Option<i64>,
}

/// 按 `admin + method + route + key` 查找（范围内唯一）。
pub async fn find(
    conn: &mut SqliteConnection,
    admin_id: &str,
    method: &str,
    route: &str,
    key: &str,
) -> Result<Option<IdempotencyRecord>, StorageError> {
    let row = sqlx::query(
        "SELECT id, admin_id, method, route, \"key\", body_hash, resource_id, response_status, created_at \
           FROM idempotency_records \
          WHERE admin_id = ? AND method = ? AND route = ? AND \"key\" = ?",
    )
    .bind(admin_id)
    .bind(method)
    .bind(route)
    .bind(key)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| record_from_row(&row)).transpose()
}

/// 插入幂等记录（唯一键冲突由调用方按"并发重放"处理）。
pub async fn insert(
    conn: &mut SqliteConnection,
    new: NewIdempotencyRecord,
    now: Timestamp,
) -> Result<IdempotencyRecord, StorageError> {
    let record = IdempotencyRecord {
        id: ids::new_id(),
        admin_id: new.admin_id,
        method: new.method,
        route: new.route,
        key: new.key,
        body_hash: new.body_hash,
        resource_id: new.resource_id,
        response_status: new.response_status,
        created_at: now,
    };
    sqlx::query(
        "INSERT INTO idempotency_records \
             (id, admin_id, method, route, \"key\", body_hash, resource_id, response_status, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&record.id)
    .bind(&record.admin_id)
    .bind(&record.method)
    .bind(&record.route)
    .bind(&record.key)
    .bind(&record.body_hash)
    .bind(&record.resource_id)
    .bind(record.response_status)
    .bind(record.created_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(record)
}

fn record_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<IdempotencyRecord, StorageError> {
    Ok(IdempotencyRecord {
        id: row.try_get("id")?,
        admin_id: row.try_get("admin_id")?,
        method: row.try_get("method")?,
        route: row.try_get("route")?,
        key: row.try_get("key")?,
        body_hash: row.try_get("body_hash")?,
        resource_id: row.try_get("resource_id")?,
        response_status: row.try_get("response_status")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
    })
}
