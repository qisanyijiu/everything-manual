//! 视图候选图（ADR-044；`view_candidates` 表）。
//!
//! 候选只是"待选"：不参与报价/生成快照。确定排列时由 `photos::arrange` 把选中候选的
//! 资产写成 photos 行；删除候选是软删除（`dismissed_at`），可撤销、可审计。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::PhotoView;
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

#[derive(Debug, Clone, PartialEq)]
pub struct ViewCandidate {
    pub id: String,
    pub item_id: String,
    pub asset_id: String,
    pub document_id: Option<String>,
    pub page_number: Option<i64>,
    pub source: String,
    pub suggested_view: Option<PhotoView>,
    pub confidence: Option<f64>,
    pub note: Option<String>,
    pub dismissed_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewViewCandidate {
    pub item_id: String,
    pub asset_id: String,
    pub document_id: Option<String>,
    pub page_number: Option<i64>,
    pub source: String,
    pub suggested_view: Option<PhotoView>,
    pub confidence: Option<f64>,
    pub note: Option<String>,
}

macro_rules! select_sql {
    ($where:literal) => {
        concat!(
            "SELECT id, item_id, asset_id, document_id, page_number, source, suggested_view, \
             confidence, note, dismissed_at, created_at FROM view_candidates ",
            $where
        )
    };
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ViewCandidate, StorageError> {
    let view: Option<String> = row.try_get("suggested_view")?;
    Ok(ViewCandidate {
        id: row.try_get("id")?,
        item_id: row.try_get("item_id")?,
        asset_id: row.try_get("asset_id")?,
        document_id: row.try_get("document_id")?,
        page_number: row.try_get("page_number")?,
        source: row.try_get("source")?,
        suggested_view: view.as_deref().map(super::photos::parse_view).transpose()?,
        confidence: row.try_get("confidence")?,
        note: row.try_get("note")?,
        dismissed_at: row
            .try_get::<Option<i64>, _>("dismissed_at")?
            .map(Timestamp::from_millis),
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
    })
}

/// 新建；同一物品同一资产已有候选时返回已有行（幂等：重复解析不产生重复候选）。
pub async fn create(
    conn: &mut SqliteConnection,
    new: NewViewCandidate,
) -> Result<(ViewCandidate, bool), StorageError> {
    if let Some(existing) = sqlx::query(select_sql!("WHERE item_id = ? AND asset_id = ?"))
        .bind(&new.item_id)
        .bind(&new.asset_id)
        .fetch_optional(&mut *conn)
        .await?
    {
        return Ok((from_row(&existing)?, false));
    }
    let id = ids::new_id();
    let now = Timestamp::now().as_millis();
    sqlx::query(
        "INSERT INTO view_candidates (id, item_id, asset_id, document_id, page_number, source, \
         suggested_view, confidence, note, dismissed_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)",
    )
    .bind(&id)
    .bind(&new.item_id)
    .bind(&new.asset_id)
    .bind(&new.document_id)
    .bind(new.page_number)
    .bind(&new.source)
    .bind(new.suggested_view.map(PhotoView::as_str))
    .bind(new.confidence)
    .bind(&new.note)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    let row = sqlx::query(select_sql!("WHERE id = ?"))
        .bind(&id)
        .fetch_one(&mut *conn)
        .await?;
    Ok((from_row(&row)?, true))
}

/// 未删除的候选（按页码、创建时间排序）。
pub async fn list_active(
    conn: &mut SqliteConnection,
    item_id: &str,
) -> Result<Vec<ViewCandidate>, StorageError> {
    let rows = sqlx::query(select_sql!(
        "WHERE item_id = ? AND dismissed_at IS NULL ORDER BY page_number IS NULL, page_number, created_at, id"
    ))
    .bind(item_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn get_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
    id: &str,
) -> Result<Option<ViewCandidate>, StorageError> {
    let row = sqlx::query(select_sql!("WHERE id = ? AND item_id = ?"))
        .bind(id)
        .bind(item_id)
        .fetch_optional(&mut *conn)
        .await?;
    row.as_ref().map(from_row).transpose()
}

/// 软删除 / 恢复（`dismissed = false` 用于撤销）。返回是否找到该候选。
pub async fn set_dismissed(
    conn: &mut SqliteConnection,
    item_id: &str,
    id: &str,
    dismissed: bool,
) -> Result<bool, StorageError> {
    let changed =
        sqlx::query("UPDATE view_candidates SET dismissed_at = ? WHERE id = ? AND item_id = ?")
            .bind(if dismissed {
                Some(Timestamp::now().as_millis())
            } else {
                None
            })
            .bind(id)
            .bind(item_id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    Ok(changed > 0)
}
