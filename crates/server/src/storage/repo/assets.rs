//! `assets` 表的仓储原语（T06）：资产引用 blob 并归属物品。
//!
//! **边界**：只做持久化（插入、按 id 读取、按物品查询、物品累计用量）。
//! 归属校验、用途校验、体积限制在 `crate::assets` 服务层。
//!
//! 所有函数接受 `&mut SqliteConnection`，可与 blob 插入放进同一短事务
//! （T11 起"快照 + 预留 + job"也复用同一模式）。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::{Asset, AssetPurpose, Blob};
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 新建资产的输入（服务器生成 id 与时间戳；`original_name` 只是元数据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAsset {
    pub blob_id: String,
    pub item_id: String,
    pub purpose: AssetPurpose,
    pub original_name: Option<String>,
}

/// 插入资产行并返回完整领域对象。
pub async fn insert(conn: &mut SqliteConnection, new: NewAsset) -> Result<Asset, StorageError> {
    let asset = Asset {
        id: ids::new_id(),
        blob_id: new.blob_id,
        item_id: new.item_id,
        purpose: new.purpose,
        original_name: new.original_name,
        created_at: Timestamp::now(),
    };
    sqlx::query(
        "INSERT INTO assets (id, blob_id, item_id, purpose, original_name, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&asset.id)
    .bind(&asset.blob_id)
    .bind(&asset.item_id)
    .bind(asset.purpose.as_str())
    .bind(&asset.original_name)
    .bind(asset.created_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(asset)
}

/// 按 id 读取资产（不校验归属；归属校验由服务层/专用查询负责）。
pub async fn get(conn: &mut SqliteConnection, id: &str) -> Result<Option<Asset>, StorageError> {
    let row = sqlx::query(SELECT_ASSET_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| asset_from_row(&row)).transpose()
}

/// 按 id 读取资产 + 它引用的 blob（内容服务入口）。
///
/// 返回 `Ok(None)` 表示"资产不存在"；内容文件的可用性由调用方按 `storage_state`
/// 与文件系统判定（元数据在而文件不在 → 内容不可用，不泄露路径）。
pub async fn get_with_blob(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<(Asset, Blob)>, StorageError> {
    let row = sqlx::query(
        "SELECT a.id AS asset_id, a.blob_id, a.item_id, a.purpose, a.original_name, \
                a.created_at AS asset_created_at, \
                b.sha256, b.size, b.mime, b.storage_state, b.created_at AS blob_created_at \
           FROM assets a JOIN blobs b ON b.sha256 = a.blob_id \
          WHERE a.id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let asset = Asset {
        id: row.try_get("asset_id")?,
        blob_id: row.try_get("blob_id")?,
        item_id: row.try_get("item_id")?,
        purpose: parse_purpose(&row.try_get::<String, _>("purpose")?)?,
        original_name: row.try_get("original_name")?,
        created_at: Timestamp::from_millis(row.try_get("asset_created_at")?),
    };
    let blob = Blob {
        sha256: row.try_get("sha256")?,
        size: row.try_get("size")?,
        mime: row.try_get("mime")?,
        storage_state: crate::storage::repo::blobs::parse_storage_state(
            &row.try_get::<String, _>("storage_state")?,
        )?,
        created_at: Timestamp::from_millis(row.try_get("blob_created_at")?),
    };
    Ok(Some((asset, blob)))
}

/// 按 id + 物品读取资产：跨物品/不存在都返回 `Ok(None)`。
///
/// 这是"所有访问先校验 asset 归属"的仓储入口（contracts.md §2）：T07 的
/// document/photo 绑定与 T09 的页资产都必须经过它，避免"知道 id 就能引用"。
pub async fn find_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
    id: &str,
) -> Result<Option<Asset>, StorageError> {
    let row = sqlx::query(
        "SELECT id, blob_id, item_id, purpose, original_name, created_at \
           FROM assets WHERE id = ? AND item_id = ?",
    )
    .bind(id)
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| asset_from_row(&row)).transpose()
}

/// 物品当前占用的**去重**字节数（同一内容被多次引用只算一份；contracts.md §7 的物品累计上限）。
pub async fn item_total_bytes(
    conn: &mut SqliteConnection,
    item_id: &str,
) -> Result<u64, StorageError> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(size), 0) FROM blobs \
          WHERE sha256 IN (SELECT DISTINCT blob_id FROM assets WHERE item_id = ?)",
    )
    .bind(item_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(total.max(0) as u64)
}

/// 引用某 blob 的资产数量（共享 blob 场景的测试与诊断用）。
pub async fn count_for_blob(
    conn: &mut SqliteConnection,
    blob_id: &str,
) -> Result<i64, StorageError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assets WHERE blob_id = ?")
        .bind(blob_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(count)
}

const SELECT_ASSET_SQL: &str = "SELECT id, blob_id, item_id, purpose, original_name, created_at \
     FROM assets WHERE id = ?";

fn asset_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Asset, StorageError> {
    let created_at: i64 = row.try_get("created_at")?;
    let purpose: String = row.try_get("purpose")?;
    Ok(Asset {
        id: row.try_get("id")?,
        blob_id: row.try_get("blob_id")?,
        item_id: row.try_get("item_id")?,
        purpose: parse_purpose(&purpose)?,
        original_name: row.try_get("original_name")?,
        created_at: Timestamp::from_millis(created_at),
    })
}

/// SQL 值（snake_case）→ 领域枚举；未知值视为数据损坏。
pub fn parse_purpose(value: &str) -> Result<AssetPurpose, StorageError> {
    match value {
        "document" => Ok(AssetPurpose::Document),
        "photo" => Ok(AssetPurpose::Photo),
        "page_image" => Ok(AssetPurpose::PageImage),
        "page_text" => Ok(AssetPurpose::PageText),
        "model" => Ok(AssetPurpose::Model),
        "release_manifest" => Ok(AssetPurpose::ReleaseManifest),
        other => Err(StorageError::ConstraintViolation {
            detail: format!("assets.purpose 出现未知值：{other}"),
        }),
    }
}
