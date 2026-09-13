//! `generation_snapshots` 表的仓储原语（T11 / REQ-022）。
//!
//! **边界**：只做持久化。快照的"输入不可变"由 0002 触发器钉在 schema 上
//! （`UPDATE` 一律拒绝）；本模块提供 `insert`/`get`，不定义报价或预算语义
//! （那些在 `crate::generation`）。
//!
//! 冻结内容（contracts.md §2/§4）：
//! - `photo_ids` 与 `photo_hashes` **一一对应**（照片行可变，只存 id 会漏掉换资产）；
//! - `provider_config` 只含非密钥参数（模型名、质量、face_limit 等）；
//! - `budgets` 记录本次授权的上限、服务器计算的保守上界与价格版本引用；
//! - `item_revision` 是报价/确认时看到的物品版本（编辑物品不改变已开始任务）。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::GenerationSnapshot;
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 新建快照的输入（JSON 文本由调用方序列化；本模块只做 `json_valid` 兜底）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSnapshot {
    pub item_id: String,
    pub item_revision: i64,
    pub preparation_id: String,
    /// JSON 数组文本：多视图照片 id（槽位顺序）。
    pub photo_ids_json: String,
    /// JSON 数组文本：与 `photo_ids_json` 对齐的 blob sha256。
    pub photo_hashes_json: String,
    /// JSON 文本：非密钥供应商配置快照。
    pub provider_config_json: String,
    pub prompt_version: String,
    pub price_version: String,
    /// JSON 文本：预算（授权上限 + 保守上界 + 价格版本）。
    pub budgets_json: String,
}

/// 插入快照并返回完整领域对象。
pub async fn insert(
    conn: &mut SqliteConnection,
    new: NewSnapshot,
    now: Timestamp,
) -> Result<GenerationSnapshot, StorageError> {
    let snapshot = GenerationSnapshot {
        id: ids::new_id(),
        item_id: new.item_id,
        item_revision: new.item_revision,
        preparation_id: new.preparation_id,
        photo_ids: parse_json(&new.photo_ids_json, "generation_snapshots.photo_ids")?,
        photo_hashes: parse_json(&new.photo_hashes_json, "generation_snapshots.photo_hashes")?,
        provider_config: parse_json(
            &new.provider_config_json,
            "generation_snapshots.provider_config",
        )?,
        prompt_version: new.prompt_version,
        price_version: new.price_version,
        budgets: parse_json(&new.budgets_json, "generation_snapshots.budgets")?,
        created_at: now,
    };
    sqlx::query(
        "INSERT INTO generation_snapshots \
             (id, item_id, item_revision, preparation_id, photo_ids, photo_hashes, \
              provider_config, prompt_version, price_version, budgets, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&snapshot.id)
    .bind(&snapshot.item_id)
    .bind(snapshot.item_revision)
    .bind(&snapshot.preparation_id)
    .bind(&new.photo_ids_json)
    .bind(&new.photo_hashes_json)
    .bind(&new.provider_config_json)
    .bind(&snapshot.prompt_version)
    .bind(&snapshot.price_version)
    .bind(&new.budgets_json)
    .bind(snapshot.created_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(snapshot)
}

/// 按 id 读取快照。
pub async fn get(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<GenerationSnapshot>, StorageError> {
    let row = sqlx::query(
        "SELECT id, item_id, item_revision, preparation_id, photo_ids, photo_hashes, \
                provider_config, prompt_version, price_version, budgets, created_at \
           FROM generation_snapshots WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| snapshot_from_row(&row)).transpose()
}

fn parse_json(text: &str, column: &str) -> Result<serde_json::Value, StorageError> {
    serde_json::from_str(text).map_err(|error| StorageError::Database {
        detail: format!("{column} 不是合法 JSON：{error}"),
    })
}

fn snapshot_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<GenerationSnapshot, StorageError> {
    // JSON 列在 SQLite 中是 TEXT：读出文本再解析（不依赖 sqlx 的 JSON 类型映射）。
    let text = |column: &str| -> Result<String, StorageError> { Ok(row.try_get(column)?) };
    Ok(GenerationSnapshot {
        id: row.try_get("id")?,
        item_id: row.try_get("item_id")?,
        item_revision: row.try_get("item_revision")?,
        preparation_id: row.try_get("preparation_id")?,
        photo_ids: parse_json(&text("photo_ids")?, "generation_snapshots.photo_ids")?,
        photo_hashes: parse_json(&text("photo_hashes")?, "generation_snapshots.photo_hashes")?,
        provider_config: parse_json(
            &text("provider_config")?,
            "generation_snapshots.provider_config",
        )?,
        prompt_version: row.try_get("prompt_version")?,
        price_version: row.try_get("price_version")?,
        budgets: parse_json(&text("budgets")?, "generation_snapshots.budgets")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
    })
}
