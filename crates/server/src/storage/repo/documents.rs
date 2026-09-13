//! `documents` 表的仓储原语（T07）：说明书原件与物品的绑定记录。
//!
//! **边界**：只做持久化（插入、读取、按物品分页）。资产归属、PDF 类型与
//! `sourceUrl` 校验在 HTTP 层（`http::documents`）完成——本模块不做业务规则判断，
//! 外键（item_id / source_asset_id / source_sha256）是最后的兜底。
//!
//! 记录不可变（无 revision、无 UPDATE 路径）：绑定后如需换原件，由 T09 起新
//! preparation 走同一 document；契约（contracts.md §3）未定义 document 修改路由。
//!
//! 所有函数接受 `&mut SqliteConnection`，可与其它写入放进同一短事务。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::Document;
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 新建 document 的输入（服务器生成 id 与时间戳）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDocument {
    pub item_id: String,
    /// 所属物品的 PDF 资产（调用方已校验归属与 `purpose=document`）。
    pub source_asset_id: String,
    /// 原件内容 sha256（=资产引用的 blob id，内容寻址）。
    pub source_sha256: String,
    pub title: String,
    /// 仅作出处记录；服务端不据此发起抓取。
    pub source_url: Option<String>,
}

/// 插入 document 行并返回完整领域对象。
pub async fn create(
    conn: &mut SqliteConnection,
    new: NewDocument,
) -> Result<Document, StorageError> {
    let now = Timestamp::now();
    let document = Document {
        id: ids::new_id(),
        item_id: new.item_id,
        source_asset_id: new.source_asset_id,
        source_sha256: new.source_sha256,
        title: new.title,
        source_url: new.source_url,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO documents (id, item_id, source_asset_id, source_sha256, title, source_url, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&document.id)
    .bind(&document.item_id)
    .bind(&document.source_asset_id)
    .bind(&document.source_sha256)
    .bind(&document.title)
    .bind(&document.source_url)
    .bind(document.created_at.as_millis())
    .bind(document.updated_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(document)
}

/// 按 id + 物品读取：跨物品/不存在都返回 `Ok(None)`（不泄露存在性）。
pub async fn find_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
    id: &str,
) -> Result<Option<Document>, StorageError> {
    let row = sqlx::query(
        "SELECT id, item_id, source_asset_id, source_sha256, title, source_url, created_at, updated_at \
           FROM documents WHERE id = ? AND item_id = ?",
    )
    .bind(id)
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| document_from_row(&row)).transpose()
}

/// 按 id 读取 document（不校验物品；调用方按需再校验归属）。
pub async fn get(conn: &mut SqliteConnection, id: &str) -> Result<Option<Document>, StorageError> {
    let row = sqlx::query(
        "SELECT id, item_id, source_asset_id, source_sha256, title, source_url, created_at, updated_at \
           FROM documents WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| document_from_row(&row)).transpose()
}

/// 某物品的一页 document：`(created_at DESC, id DESC)`；`cursor` 语义同 `items::list_page`。
pub async fn list_page(
    conn: &mut SqliteConnection,
    item_id: &str,
    cursor: Option<(i64, String)>,
    limit: u32,
) -> Result<Vec<Document>, StorageError> {
    let (cursor_millis, cursor_id) = match cursor {
        Some((millis, id)) => (Some(millis), Some(id)),
        None => (None, None),
    };
    let rows = sqlx::query(
        "SELECT id, item_id, source_asset_id, source_sha256, title, source_url, created_at, updated_at \
           FROM documents \
          WHERE item_id = ? AND (? IS NULL OR (created_at, id) < (?, ?)) \
          ORDER BY created_at DESC, id DESC \
          LIMIT ?",
    )
    .bind(item_id)
    .bind(cursor_millis)
    .bind(cursor_millis)
    .bind(cursor_id)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(document_from_row).collect()
}

fn document_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Document, StorageError> {
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;
    Ok(Document {
        id: row.try_get("id")?,
        item_id: row.try_get("item_id")?,
        source_asset_id: row.try_get("source_asset_id")?,
        source_sha256: row.try_get("source_sha256")?,
        title: row.try_get("title")?,
        source_url: row.try_get("source_url")?,
        created_at: Timestamp::from_millis(created_at),
        updated_at: Timestamp::from_millis(updated_at),
    })
}
