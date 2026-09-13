//! `items` 的仓储原语：创建、读取、带 revision CAS 的更新。
//!
//! **边界（T07 后）**：这里只有持久化原语与 SQL 层不变量（NOT NULL / CHECK / 外键 /
//! revision 条件更新 / 归档过滤）。业务校验（名称与型号的长度/空白、字段级 422、
//! PATCH 清空语义）在 `manual_core::validation` + HTTP 层（T07）；本模块的
//! `create`/`update` 不做业务规则判断，只用数据库约束兜底
//! （例如空白名称仍会得到 [`StorageError::ConstraintViolation`]）。
//!
//! 乐观锁协议（contracts.md §1）：可编辑聚合根带整数 `revision`；更新必须携带
//! 期望 revision，条件更新失败时返回 [`StorageError::RevisionConflict`] 并携带
//! `current_revision`（供 412 的 `details.currentRevision`）。
//!
//! 所有函数接受 `&mut SqliteConnection`，因此调用方可以自由组合事务
//! （T11 起要求"快照 + 预留 + job"在同一事务内写入）：
//!
//! ```ignore
//! let mut conn = pool.acquire().await?;          // 单条语句
//! let item = repo::items::create(&mut conn, new_item).await?;
//!
//! let mut tx = storage::begin_write(conn).await?; // 写事务一律 BEGIN IMMEDIATE（storage::tx）
//! let item = repo::items::create(&mut tx, new_item).await?;
//! tx.commit().await?;
//! ```

use sqlx::{Row, SqliteConnection};

use manual_core::domain::Item;
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 新建物品的输入（服务器生成 id/revision/时间戳）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    pub name: String,
    pub brand: Option<String>,
    pub model: String,
    pub variant: Option<String>,
}

/// 更新物品的输入（整体替换可编辑字段；`archived` 控制归档标记）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemUpdate {
    pub name: String,
    pub brand: Option<String>,
    pub model: String,
    pub variant: Option<String>,
    /// true = 归档（首次归档记录时间，已归档保持原时间）；false = 取消归档。
    pub archived: bool,
}

/// 创建物品：`revision = 1`、`archived_at = NULL`。
pub async fn create(conn: &mut SqliteConnection, new: NewItem) -> Result<Item, StorageError> {
    let now = Timestamp::now();
    let item = Item {
        id: ids::new_id(),
        name: new.name,
        brand: new.brand,
        model: new.model,
        variant: new.variant,
        revision: 1,
        archived_at: None,
        created_at: now,
        updated_at: now,
    };

    sqlx::query(
        "INSERT INTO items (id, name, brand, model, variant, revision, archived_at, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?)",
    )
    .bind(&item.id)
    .bind(&item.name)
    .bind(&item.brand)
    .bind(&item.model)
    .bind(&item.variant)
    .bind(item.revision)
    .bind(item.created_at.as_millis())
    .bind(item.updated_at.as_millis())
    .execute(&mut *conn)
    .await?;

    Ok(item)
}

/// 按 id 读取物品；不存在返回 `Ok(None)`。
pub async fn get(conn: &mut SqliteConnection, id: &str) -> Result<Option<Item>, StorageError> {
    let row = sqlx::query(SELECT_ITEM_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| item_from_row(&row)).transpose()
}

/// 带 revision CAS 的更新：仅当当前 `revision == expected_revision` 时写入，
/// 成功则 `revision + 1` 并返回更新后的行。
///
/// 失败时区分两种情况：
/// - 行不存在 → [`StorageError::NotFound`]；
/// - revision 过期 → [`StorageError::RevisionConflict`]（带 `current_revision`）。
pub async fn update(
    conn: &mut SqliteConnection,
    id: &str,
    expected_revision: i64,
    patch: ItemUpdate,
) -> Result<Item, StorageError> {
    let now = Timestamp::now().as_millis();
    let changed = sqlx::query(
        "UPDATE items \
            SET name = ?, brand = ?, model = ?, variant = ?, \
                revision = revision + 1, \
                archived_at = CASE WHEN ? THEN COALESCE(archived_at, ?) ELSE NULL END, \
                updated_at = ? \
          WHERE id = ? AND revision = ?",
    )
    .bind(&patch.name)
    .bind(&patch.brand)
    .bind(&patch.model)
    .bind(&patch.variant)
    .bind(patch.archived)
    .bind(now)
    .bind(now)
    .bind(id)
    .bind(expected_revision)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    if changed == 0 {
        return match current_revision(conn, id).await? {
            Some(current_revision) => Err(StorageError::RevisionConflict {
                entity: "item",
                id: id.to_owned(),
                current_revision,
            }),
            None => Err(StorageError::NotFound {
                entity: "item",
                id: id.to_owned(),
            }),
        };
    }

    get(conn, id).await?.ok_or_else(|| StorageError::Database {
        detail: format!("更新后读取 item 失败：{id}"),
    })
}

/// 列表过滤条件（T07，REQ-010）：默认列表只含未归档；`archived=true` 只含已归档。
///
/// 归档是"停用"而不是删除：归档物品的单条读取、资料/照片/资产引用都保持可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchivedFilter {
    /// 未归档（默认列表）。
    Active,
    /// 已归档（`archived=true` 列表）。
    Archived,
}

/// 稳定排序的一页：`(created_at DESC, id DESC)`，`limit` 由调用方封顶
/// （默认 20、最多 100，见 `http::pagination`）。
///
/// `cursor` 为 `(created_at_millis, id)`：只返回严格排在其后的行
/// （SQLite 行值比较，排序键与 ID 同时参与，重复时间戳也不会漏行/重行）。
/// 过滤条件在 SQL 内判定（`archived_at IS NULL` / `IS NOT NULL`），游标只负责位置。
pub async fn list_page(
    conn: &mut SqliteConnection,
    filter: ArchivedFilter,
    cursor: Option<(i64, String)>,
    limit: u32,
) -> Result<Vec<Item>, StorageError> {
    let (cursor_millis, cursor_id) = match cursor {
        Some((millis, id)) => (Some(millis), Some(id)),
        None => (None, None),
    };
    let sql = match filter {
        ArchivedFilter::Active => LIST_ACTIVE_SQL,
        ArchivedFilter::Archived => LIST_ARCHIVED_SQL,
    };
    let rows = sqlx::query(sql)
        .bind(cursor_millis)
        .bind(cursor_millis)
        .bind(cursor_id)
        .bind(limit)
        .fetch_all(&mut *conn)
        .await?;
    rows.iter().map(item_from_row).collect()
}

/// 列清单与 [`item_from_row`] 对应；两条 SQL 只差归档条件（静态 SQL，无拼接）。
const LIST_ACTIVE_SQL: &str = "SELECT id, name, brand, model, variant, revision, archived_at, created_at, updated_at \
       FROM items \
      WHERE (? IS NULL OR (created_at, id) < (?, ?)) AND archived_at IS NULL \
      ORDER BY created_at DESC, id DESC \
      LIMIT ?";

const LIST_ARCHIVED_SQL: &str = "SELECT id, name, brand, model, variant, revision, archived_at, created_at, updated_at \
       FROM items \
      WHERE (? IS NULL OR (created_at, id) < (?, ?)) AND archived_at IS NOT NULL \
      ORDER BY created_at DESC, id DESC \
      LIMIT ?";

/// 静态 SQL（列清单与 [`item_from_row`] 对应；不含任何拼接的用户输入）。
const SELECT_ITEM_SQL: &str = "SELECT id, name, brand, model, variant, revision, archived_at, created_at, updated_at \
     FROM items WHERE id = ?";

async fn current_revision(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<i64>, StorageError> {
    let revision: Option<i64> = sqlx::query_scalar("SELECT revision FROM items WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(revision)
}

fn item_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Item, StorageError> {
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;
    let archived_at: Option<i64> = row.try_get("archived_at")?;
    Ok(Item {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        brand: row.try_get("brand")?,
        model: row.try_get("model")?,
        variant: row.try_get("variant")?,
        revision: row.try_get("revision")?,
        archived_at: archived_at.map(Timestamp::from_millis),
        created_at: Timestamp::from_millis(created_at),
        updated_at: Timestamp::from_millis(updated_at),
    })
}
