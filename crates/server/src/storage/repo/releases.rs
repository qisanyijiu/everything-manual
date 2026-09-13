//! `manual_releases` 表的仓储原语（T19 / REQ-035；contracts.md §2/§3/§7）。
//!
//! **边界**：只做插入与读取。发布不变量、manifest 构建与幂等在 `crate::releases`
//! （服务层）；`manual_releases` 由 0002 的触发器保护为**不可变**（UPDATE/DELETE
//! 都被数据库拒绝），因此本模块没有更新/删除函数。
//!
//! 关键语义：
//! - `draft_revision` 记录发布时草稿的 revision：之后修改草稿不会改变已发布内容
//!   （release 只引用不可变 manifest 资产，不引用会变的 draft 行）；
//! - `manifest_asset_id` 指向 `assets` 中的 `release_manifest` 资产（内容寻址）。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::ManualRelease;
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 新建发布记录的输入（id 与时间由仓储生成）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRelease {
    pub item_id: String,
    pub draft_id: String,
    pub draft_revision: i64,
    pub model_revision_id: String,
    pub manifest_asset_id: String,
}

/// 插入发布记录（不可变；无 upsert 语义——重复发布由幂等键在服务层拦截）。
pub async fn insert(
    conn: &mut SqliteConnection,
    new: NewRelease,
    now: Timestamp,
) -> Result<ManualRelease, StorageError> {
    let release = ManualRelease {
        id: ids::new_id(),
        item_id: new.item_id,
        draft_id: new.draft_id,
        draft_revision: new.draft_revision,
        model_revision_id: new.model_revision_id,
        manifest_asset_id: new.manifest_asset_id,
        created_at: now,
    };
    sqlx::query(
        "INSERT INTO manual_releases \
             (id, item_id, draft_id, draft_revision, model_revision_id, manifest_asset_id, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&release.id)
    .bind(&release.item_id)
    .bind(&release.draft_id)
    .bind(release.draft_revision)
    .bind(&release.model_revision_id)
    .bind(&release.manifest_asset_id)
    .bind(release.created_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(release)
}

/// 按 id 读取发布记录（不校验归属；调用方用 [`get_for_item`] 做归属过滤）。
pub async fn get(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<ManualRelease>, StorageError> {
    let row = sqlx::query(SELECT_RELEASE_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| release_from_row(&row)).transpose()
}

/// 按 id + 物品读取（跨物品按不存在处理，不泄露存在性）。
pub async fn get_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
    id: &str,
) -> Result<Option<ManualRelease>, StorageError> {
    let row = sqlx::query(
        "SELECT id, item_id, draft_id, draft_revision, model_revision_id, manifest_asset_id, created_at \
           FROM manual_releases WHERE id = ? AND item_id = ?",
    )
    .bind(id)
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| release_from_row(&row)).transpose()
}

/// 某物品的发布版本（发布时间倒序；U-03：排序由服务端给定）。
pub async fn list_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
    limit: i64,
) -> Result<Vec<ManualRelease>, StorageError> {
    let rows = sqlx::query(
        "SELECT id, item_id, draft_id, draft_revision, model_revision_id, manifest_asset_id, created_at \
           FROM manual_releases WHERE item_id = ? \
          ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(item_id)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(release_from_row).collect()
}

/// 某物品的发布版本数（测试与诊断用）。
pub async fn count_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
) -> Result<i64, StorageError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM manual_releases WHERE item_id = ?")
        .bind(item_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(count)
}

/// 某草稿是否已发布过（发布事务的并发/重放判定的辅助信息；不阻止合法再发布）。
pub async fn exists_for_draft(
    conn: &mut SqliteConnection,
    draft_id: &str,
) -> Result<bool, StorageError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM manual_releases WHERE draft_id = ?")
        .bind(draft_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(count > 0)
}

const SELECT_RELEASE_SQL: &str = "SELECT id, item_id, draft_id, draft_revision, model_revision_id, \
        manifest_asset_id, created_at FROM manual_releases WHERE id = ?";

fn release_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ManualRelease, StorageError> {
    Ok(ManualRelease {
        id: row.try_get("id")?,
        item_id: row.try_get("item_id")?,
        draft_id: row.try_get("draft_id")?,
        draft_revision: row.try_get("draft_revision")?,
        model_revision_id: row.try_get("model_revision_id")?,
        manifest_asset_id: row.try_get("manifest_asset_id")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
    })
}
