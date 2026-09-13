//! `photos` 表的仓储原语（T07）：多视图照片与视图语义。
//!
//! **边界**：只做持久化（插入、读取、带 revision CAS 的更新、按物品列举与视图占用查询）。
//! 资产归属、JPEG/PNG 类型与视图枚举解析在 HTTP 层（`http::photos`）；本模块依赖
//! `photos_item_view_unique`（迁移 0003）作为"同一物品每视图最多一张"的最终防线。
//!
//! 两个集合语义（contracts.md §2；REQ-013）：
//! - [`list_for_item`]：全部照片（含 `detail`），固定槽位顺序 front→left→back→right→detail；
//! - [`list_multiview_for_item`]：**多视图集合**（排除 `detail`），供 Tripo 请求体（T12）使用。
//!
//! 排序使用 CASE 表达式固定槽位顺序，而不是字典序——UI-012 的五个槽位与
//! T12 的请求体顺序都不应受视图名排序影响。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::{Photo, PhotoView};
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 照片列表 SQL（`concat!` 编译期拼接：单一定义槽位顺序，两条查询只差 WHERE）。
macro_rules! photo_list_sql {
    ($where:literal) => {
        concat!(
            "SELECT id, item_id, asset_id, view, revision, created_at, updated_at \
               FROM photos WHERE ",
            $where,
            " ORDER BY CASE view \
                 WHEN 'front' THEN 1 WHEN 'left' THEN 2 WHEN 'back' THEN 3 \
                 WHEN 'right' THEN 4 WHEN 'detail' THEN 5 ELSE 9 END, id"
        )
    };
}

/// 全部照片（含 detail），槽位顺序 front→left→back→right→detail。
const LIST_ALL_SQL: &str = photo_list_sql!("item_id = ?");
/// 多视图集合（排除 detail）。
const LIST_MULTIVIEW_SQL: &str = photo_list_sql!("item_id = ? AND view <> 'detail'");

/// 新建 photo 的输入（服务器生成 id、`revision=1` 与时间戳）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPhoto {
    pub item_id: String,
    /// 所属物品的照片资产（调用方已校验归属与 `purpose=photo`）。
    pub asset_id: String,
    pub view: PhotoView,
}

/// 带 CAS 的更新输入（整体替换可编辑字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoUpdate {
    pub asset_id: String,
    pub view: PhotoView,
}

/// 插入照片行并返回完整领域对象（`revision = 1`）。
///
/// 视图占用冲突由 `photos_item_view_unique` 唯一索引拒绝
/// （[`StorageError::UniqueViolation`]；调用方在事务内先给出友好错误）。
pub async fn create(conn: &mut SqliteConnection, new: NewPhoto) -> Result<Photo, StorageError> {
    let now = Timestamp::now();
    let photo = Photo {
        id: ids::new_id(),
        item_id: new.item_id,
        asset_id: new.asset_id,
        view: new.view,
        revision: 1,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO photos (id, item_id, asset_id, view, revision, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&photo.id)
    .bind(&photo.item_id)
    .bind(&photo.asset_id)
    .bind(photo.view.as_str())
    .bind(photo.revision)
    .bind(photo.created_at.as_millis())
    .bind(photo.updated_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(photo)
}

/// 按 id 读取照片（不校验归属；归属校验用 [`find_for_item`]）。
pub async fn get(conn: &mut SqliteConnection, id: &str) -> Result<Option<Photo>, StorageError> {
    let row = sqlx::query(SELECT_PHOTO_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| photo_from_row(&row)).transpose()
}

/// 按 id + 物品读取：跨物品/不存在都返回 `Ok(None)`（不泄露存在性）。
pub async fn find_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
    id: &str,
) -> Result<Option<Photo>, StorageError> {
    let row = sqlx::query(
        "SELECT id, item_id, asset_id, view, revision, created_at, updated_at \
           FROM photos WHERE id = ? AND item_id = ?",
    )
    .bind(id)
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| photo_from_row(&row)).transpose()
}

/// 带 revision CAS 的更新：仅当 `revision == expected_revision` 时写入，成功 `+1`。
///
/// 失败语义与 `items::update` 一致：行不存在 → [`StorageError::NotFound`]；
/// revision 过期 → [`StorageError::RevisionConflict`]（带 `current_revision`）；
/// 视图被同物品另一张照片占用 → [`StorageError::UniqueViolation`]。
pub async fn update(
    conn: &mut SqliteConnection,
    item_id: &str,
    id: &str,
    expected_revision: i64,
    patch: PhotoUpdate,
) -> Result<Photo, StorageError> {
    let now = Timestamp::now().as_millis();
    let changed = sqlx::query(
        "UPDATE photos \
            SET asset_id = ?, view = ?, revision = revision + 1, updated_at = ? \
          WHERE id = ? AND item_id = ? AND revision = ?",
    )
    .bind(&patch.asset_id)
    .bind(patch.view.as_str())
    .bind(now)
    .bind(id)
    .bind(item_id)
    .bind(expected_revision)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    if changed == 0 {
        return match current_revision(conn, id).await? {
            Some(current_revision) => Err(StorageError::RevisionConflict {
                entity: "photo",
                id: id.to_owned(),
                current_revision,
            }),
            None => Err(StorageError::NotFound {
                entity: "photo",
                id: id.to_owned(),
            }),
        };
    }

    find_for_item(conn, item_id, id)
        .await?
        .ok_or_else(|| StorageError::Database {
            detail: format!("更新后读取 photo 失败：{id}"),
        })
}

/// 某物品的全部照片，按槽位顺序（front→left→back→right→detail）返回（含 `detail`）。
pub async fn list_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
) -> Result<Vec<Photo>, StorageError> {
    list_photos(conn, item_id, false).await
}

/// 某物品的**多视图集合**：排除 `detail`（它只用于理解与核对，不发送给 Tripo）。
///
/// T12 构造多视图请求体时必须使用本函数，而不是 [`list_for_item`]。
pub async fn list_multiview_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
) -> Result<Vec<Photo>, StorageError> {
    list_photos(conn, item_id, true).await
}

async fn list_photos(
    conn: &mut SqliteConnection,
    item_id: &str,
    multiview_only: bool,
) -> Result<Vec<Photo>, StorageError> {
    let sql = if multiview_only {
        LIST_MULTIVIEW_SQL
    } else {
        LIST_ALL_SQL
    };
    let rows = sqlx::query(sql).bind(item_id).fetch_all(&mut *conn).await?;
    rows.iter().map(photo_from_row).collect()
}

/// 占用某视图的照片 id（可排除指定照片，用于 PATCH 自身视图不变判定的场景）。
///
/// 返回 `Some(photo_id)` 表示冲突；调用方据此返回 422 `details.reason=viewOccupied`
/// 与 `existingPhotoId`（供 UI 提示"请先移除或改选"）。
pub async fn view_occupant(
    conn: &mut SqliteConnection,
    item_id: &str,
    view: PhotoView,
    exclude_photo_id: Option<&str>,
) -> Result<Option<String>, StorageError> {
    let row = sqlx::query(
        "SELECT id FROM photos \
          WHERE item_id = ? AND view = ? AND (? IS NULL OR id <> ?) \
          LIMIT 1",
    )
    .bind(item_id)
    .bind(view.as_str())
    .bind(exclude_photo_id)
    .bind(exclude_photo_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| row.try_get::<String, _>("id").map_err(StorageError::from))
        .transpose()
}

/// 静态 SQL（列清单与 [`photo_from_row`] 对应）。
const SELECT_PHOTO_SQL: &str = "SELECT id, item_id, asset_id, view, revision, created_at, updated_at \
     FROM photos WHERE id = ?";

// ---------------------------------------------------------------------------
// 照片 + 内容哈希（T11 报价/快照冻结用）
// ---------------------------------------------------------------------------

/// 照片及其内容 sha256（报价指纹与 `generation_snapshots.photo_hashes` 的输入）。
///
/// `photos.asset_id` 可变（PATCH 换资产），因此冻结快照必须同时记录**内容哈希**，
/// 只存 photoId 会漏掉"同 id 不同内容"（T07 QA 前置约束）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoWithHash {
    pub photo: Photo,
    /// 照片内容（blob）的 sha256。
    pub sha256: String,
}

/// 某物品的多视图照片 + 内容哈希（槽位顺序 front→left→back→right；排除 detail）。
pub async fn list_multiview_with_hash(
    conn: &mut SqliteConnection,
    item_id: &str,
) -> Result<Vec<PhotoWithHash>, StorageError> {
    let rows = sqlx::query(
        "SELECT p.id, p.item_id, p.asset_id, p.view, p.revision, p.created_at, p.updated_at, \
                b.sha256 AS blob_sha256 \
           FROM photos p \
           JOIN assets a ON a.id = p.asset_id \
           JOIN blobs b ON b.sha256 = a.blob_id \
          WHERE p.item_id = ? AND p.view <> 'detail' \
          ORDER BY CASE p.view \
              WHEN 'front' THEN 1 WHEN 'left' THEN 2 WHEN 'back' THEN 3 \
              WHEN 'right' THEN 4 ELSE 9 END, p.id",
    )
    .bind(item_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(photo_with_hash_from_row).collect()
}

/// 按 id + 物品读取照片 + 内容哈希（跨物品/不存在都返回 `Ok(None)`，不泄露存在性）。
pub async fn find_with_hash_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
    photo_id: &str,
) -> Result<Option<PhotoWithHash>, StorageError> {
    let row = sqlx::query(
        "SELECT p.id, p.item_id, p.asset_id, p.view, p.revision, p.created_at, p.updated_at, \
                b.sha256 AS blob_sha256 \
           FROM photos p \
           JOIN assets a ON a.id = p.asset_id \
           JOIN blobs b ON b.sha256 = a.blob_id \
          WHERE p.id = ? AND p.item_id = ?",
    )
    .bind(photo_id)
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| photo_with_hash_from_row(&row)).transpose()
}

fn photo_with_hash_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<PhotoWithHash, StorageError> {
    Ok(PhotoWithHash {
        photo: photo_from_row(row)?,
        sha256: row.try_get("blob_sha256")?,
    })
}

async fn current_revision(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<i64>, StorageError> {
    let revision: Option<i64> = sqlx::query_scalar("SELECT revision FROM photos WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(revision)
}

fn photo_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Photo, StorageError> {
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;
    let view: String = row.try_get("view")?;
    Ok(Photo {
        id: row.try_get("id")?,
        item_id: row.try_get("item_id")?,
        asset_id: row.try_get("asset_id")?,
        view: parse_view(&view)?,
        revision: row.try_get("revision")?,
        created_at: Timestamp::from_millis(created_at),
        updated_at: Timestamp::from_millis(updated_at),
    })
}

/// SQL 值（小写）→ 领域枚举；未知值视为数据损坏（不静默兜底）。
pub fn parse_view(value: &str) -> Result<PhotoView, StorageError> {
    PhotoView::from_wire(value).ok_or_else(|| StorageError::ConstraintViolation {
        detail: format!("photos.view 出现未知值：{value}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_parsing_rejects_unknown_values() {
        assert_eq!(parse_view("front").unwrap(), PhotoView::Front);
        assert_eq!(parse_view("detail").unwrap(), PhotoView::Detail);
        assert!(parse_view("top").is_err());
        assert!(parse_view("Front").is_err());
    }
}
