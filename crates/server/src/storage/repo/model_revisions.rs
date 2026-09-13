//! `model_revisions` 表的仓储原语（T13 / REQ-028）。
//!
//! **边界**：只做持久化。GLB 校验、下载、blob 落盘在 `crate::assets`，阶段状态推进
//! 在 `crate::jobs`（执行器带租约 epoch guard）。
//!
//! 关键语义（contracts.md §2/§7）：
//! - **模型字节不可变**：一行 revision 引用一个 asset（内容按 sha256 寻址）；
//!   `validation_state = validated` 才可进入阅读器（T18/T19 消费）；
//! - `rejected` 用于"已下载并保留原始模型、但校验未通过"的记录（超预算/结构问题），
//!   它**不是**可用版本，也不允许被选入草稿/发布；
//! - [`get_or_create`] 以 `(item_id, sha256)` 幂等：同一内容重复校验（重试/恢复）
//!   不会产生第二行，也不会覆盖已有状态。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::{ModelRevision, ModelValidationState};
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 新模型的输入（`id`/`created_at` 由服务器生成）。
#[derive(Debug, Clone, PartialEq)]
pub struct NewModelRevision {
    pub item_id: String,
    pub asset_id: String,
    pub sha256: String,
    /// 产生该模型的付费提交 attempt（可空：例如人工导入）。
    pub provider_attempt_id: Option<String>,
    /// JSON：包围盒与结构摘要（校验失败时为 `None`）。
    pub bounds: Option<serde_json::Value>,
    pub validation_state: ModelValidationState,
}

/// 静态 SQL（sqlx 0.9 要求静态字符串；动态值一律走 bind 参数）。
const SELECT_BY_ID_SQL: &str = "SELECT id, item_id, asset_id, sha256, provider_attempt_id, bounds, validation_state, created_at \
       FROM model_revisions WHERE id = ?";
const SELECT_BY_SHA_SQL: &str = "SELECT id, item_id, asset_id, sha256, provider_attempt_id, bounds, validation_state, created_at \
       FROM model_revisions WHERE item_id = ? AND sha256 = ?";
const SELECT_FOR_ITEM_SQL: &str = "SELECT id, item_id, asset_id, sha256, provider_attempt_id, bounds, validation_state, created_at \
       FROM model_revisions WHERE item_id = ? ORDER BY created_at DESC, id DESC";

/// 按 `(item_id, sha256)` 幂等创建；已存在则返回已有行（不改状态、不覆盖 bounds）。
pub async fn get_or_create(
    conn: &mut SqliteConnection,
    new: NewModelRevision,
) -> Result<ModelRevision, StorageError> {
    if let Some(existing) = find_by_sha(conn, &new.item_id, &new.sha256).await? {
        return Ok(existing);
    }
    let revision = ModelRevision {
        id: ids::new_id(),
        item_id: new.item_id,
        asset_id: new.asset_id,
        sha256: new.sha256,
        provider_attempt_id: new.provider_attempt_id,
        bounds: new.bounds,
        validation_state: new.validation_state,
        created_at: Timestamp::now(),
    };
    sqlx::query(
        "INSERT INTO model_revisions \
         (id, item_id, asset_id, sha256, provider_attempt_id, bounds, validation_state, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&revision.id)
    .bind(&revision.item_id)
    .bind(&revision.asset_id)
    .bind(&revision.sha256)
    .bind(&revision.provider_attempt_id)
    .bind(
        revision
            .bounds
            .as_ref()
            .map(serde_json::Value::to_string),
    )
    .bind(revision.validation_state.as_str())
    .bind(revision.created_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(revision)
}

/// 按 id 读取。
pub async fn get(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<ModelRevision>, StorageError> {
    let row = sqlx::query(SELECT_BY_ID_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| revision_from_row(&row)).transpose()
}

/// 按 `(item_id, sha256)` 读取（内容相同即同一版本）。
pub async fn find_by_sha(
    conn: &mut SqliteConnection,
    item_id: &str,
    sha256: &str,
) -> Result<Option<ModelRevision>, StorageError> {
    let row = sqlx::query(SELECT_BY_SHA_SQL)
        .bind(item_id)
        .bind(sha256)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| revision_from_row(&row)).transpose()
}

/// 某物品的全部模型版本（新→旧）。
pub async fn list_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
) -> Result<Vec<ModelRevision>, StorageError> {
    let rows = sqlx::query(SELECT_FOR_ITEM_SQL)
        .bind(item_id)
        .fetch_all(&mut *conn)
        .await?;
    rows.iter().map(revision_from_row).collect()
}

fn revision_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ModelRevision, StorageError> {
    let state: String = row.try_get("validation_state")?;
    let bounds: Option<String> = row.try_get("bounds")?;
    let bounds = match bounds {
        Some(text) => {
            Some(
                serde_json::from_str(&text).map_err(|error| StorageError::Database {
                    detail: format!("model_revisions.bounds 不是合法 JSON：{error}"),
                })?,
            )
        }
        None => None,
    };
    Ok(ModelRevision {
        id: row.try_get("id")?,
        item_id: row.try_get("item_id")?,
        asset_id: row.try_get("asset_id")?,
        sha256: row.try_get("sha256")?,
        provider_attempt_id: row.try_get("provider_attempt_id")?,
        bounds,
        validation_state: parse_validation_state(&state)?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
    })
}

/// SQL 值 → 领域枚举；未知值按损坏数据处理（不静默兜底）。
pub fn parse_validation_state(value: &str) -> Result<ModelValidationState, StorageError> {
    match value {
        "pending" => Ok(ModelValidationState::Pending),
        "validated" => Ok(ModelValidationState::Validated),
        "rejected" => Ok(ModelValidationState::Rejected),
        other => Err(StorageError::ConstraintViolation {
            detail: format!("model_revisions.validation_state 出现未知值：{other}"),
        }),
    }
}
