//! `quotes` 表的仓储原语（T11 / REQ-020、REQ-021、REQ-022）。
//!
//! **边界**：只做持久化与"一次性事实"的条件更新：
//! - 报价内容（含金额、到期时间）冻结：0006 触发器拒绝 UPDATE 快照列；
//! - 确认（`confirmed_at`/`confirmation_json`）与消费（`consumed_at`/`consumed_job_id`）
//!   只允许 `NULL → 值`，重复调用按幂等读回；业务语义（何时允许确认、提交时怎么校验）
//!   在 `crate::generation::estimate` / `crate::generation::jobs`。
//!
//! 服务端**不采信前端传入的费用数值**：金额只从这里回读 `quote_json`（服务器生成）。

use sqlx::{Row, SqliteConnection};

use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 一条报价记录（快照列 + 确认/消费状态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteRecord {
    pub id: String,
    pub item_id: String,
    pub preparation_id: String,
    pub photo_ids: Vec<String>,
    pub photo_hashes: Vec<String>,
    pub input_hash: String,
    pub model_preset: String,
    pub provider_config: serde_json::Value,
    pub price_version: String,
    pub price_snapshot_date: String,
    pub page_count: i64,
    pub max_output_tokens: i64,
    /// 完整报价载荷（线上形状的 JSON；回读时不依赖前端）。
    pub quote_json: String,
    pub expires_at: Timestamp,
    pub confirmed_at: Option<Timestamp>,
    pub confirmation_json: Option<serde_json::Value>,
    pub consumed_at: Option<Timestamp>,
    pub consumed_job_id: Option<String>,
    pub created_at: Timestamp,
}

impl QuoteRecord {
    /// 是否仍未过期（`now < expires_at`）。
    pub fn is_expired(&self, now: Timestamp) -> bool {
        now >= self.expires_at
    }

    pub fn is_confirmed(&self) -> bool {
        self.confirmed_at.is_some()
    }

    pub fn is_consumed(&self) -> bool {
        self.consumed_at.is_some()
    }
}

/// 新建报价的输入（JSON 文本由调用方序列化）。
///
/// `id` 由调用方生成：报价载荷（`quote_json`）里包含自己的 id，且报价内容不可回填，
/// 因此 id 必须在插入前确定（服务端用 UUIDv7）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewQuote {
    pub id: String,
    pub item_id: String,
    pub preparation_id: String,
    pub photo_ids_json: String,
    pub photo_hashes_json: String,
    pub input_hash: String,
    pub model_preset: String,
    pub provider_config_json: String,
    pub price_version: String,
    pub price_snapshot_date: String,
    pub page_count: i64,
    pub max_output_tokens: i64,
    pub quote_json: String,
    pub expires_at: Timestamp,
}

/// 插入报价（服务端生成 id 与创建时间）。
pub async fn insert(
    conn: &mut SqliteConnection,
    new: NewQuote,
    now: Timestamp,
) -> Result<QuoteRecord, StorageError> {
    let record = QuoteRecord {
        id: new.id,
        item_id: new.item_id,
        preparation_id: new.preparation_id,
        photo_ids: parse_string_array(&new.photo_ids_json, "quotes.photo_ids")?,
        photo_hashes: parse_string_array(&new.photo_hashes_json, "quotes.photo_hashes")?,
        input_hash: new.input_hash,
        model_preset: new.model_preset,
        provider_config: parse_json(&new.provider_config_json, "quotes.provider_config")?,
        price_version: new.price_version,
        price_snapshot_date: new.price_snapshot_date,
        page_count: new.page_count,
        max_output_tokens: new.max_output_tokens,
        quote_json: new.quote_json,
        expires_at: new.expires_at,
        confirmed_at: None,
        confirmation_json: None,
        consumed_at: None,
        consumed_job_id: None,
        created_at: now,
    };
    sqlx::query(
        "INSERT INTO quotes \
             (id, item_id, preparation_id, photo_ids, photo_hashes, input_hash, model_preset, \
              provider_config, price_version, price_snapshot_date, page_count, max_output_tokens, \
              quote_json, expires_at, confirmed_at, confirmation_json, consumed_at, consumed_job_id, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL, NULL, ?)",
    )
    .bind(&record.id)
    .bind(&record.item_id)
    .bind(&record.preparation_id)
    .bind(&new.photo_ids_json)
    .bind(&new.photo_hashes_json)
    .bind(&record.input_hash)
    .bind(&record.model_preset)
    .bind(&new.provider_config_json)
    .bind(&record.price_version)
    .bind(&record.price_snapshot_date)
    .bind(record.page_count)
    .bind(record.max_output_tokens)
    .bind(&record.quote_json)
    .bind(record.expires_at.as_millis())
    .bind(record.created_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(record)
}

/// 按 id 读取报价。
pub async fn get(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<QuoteRecord>, StorageError> {
    let row = sqlx::query(SELECT_QUOTE_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| quote_from_row(&row)).transpose()
}

/// 记录确认（`NULL → 值`，只会成功一次）。
///
/// 返回 `Some(confirmed_at)` = 本次写入的确认时间；`None` = 已有确认（幂等），
/// 调用方读回原记录返回（**覆盖首次确认时间/范围是被禁止的**，触发器兜底）。
pub async fn mark_confirmed(
    conn: &mut SqliteConnection,
    id: &str,
    confirmation_json: &str,
    now: Timestamp,
) -> Result<Option<Timestamp>, StorageError> {
    // 触发器 `quotes_confirmation_frozen` 会在 OLD.confirmed_at 非空时 ABORT：
    // WHERE 条件保证正常情况下只有首次确认走到 UPDATE。
    let changed = sqlx::query(
        "UPDATE quotes SET confirmed_at = ?, confirmation_json = ? \
          WHERE id = ? AND confirmed_at IS NULL",
    )
    .bind(now.as_millis())
    .bind(confirmation_json)
    .bind(id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok((changed > 0).then_some(now))
}

/// 消费报价（把它绑定到唯一一份创建出来的 job）。
///
/// 返回 `true` = 本次消费成功；`false` = 已被消费（调用方读回 `consumed_job_id`
/// 并按"同一报价已创建任务"处理，不新建）。
pub async fn consume(
    conn: &mut SqliteConnection,
    id: &str,
    job_id: &str,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let changed = sqlx::query(
        "UPDATE quotes SET consumed_at = ?, consumed_job_id = ? \
          WHERE id = ? AND consumed_at IS NULL",
    )
    .bind(now.as_millis())
    .bind(job_id)
    .bind(id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

const SELECT_QUOTE_SQL: &str = "SELECT id, item_id, preparation_id, photo_ids, photo_hashes, input_hash, \
        model_preset, provider_config, price_version, price_snapshot_date, page_count, \
        max_output_tokens, quote_json, expires_at, confirmed_at, confirmation_json, \
        consumed_at, consumed_job_id, created_at \
   FROM quotes WHERE id = ?";

fn parse_json(text: &str, column: &str) -> Result<serde_json::Value, StorageError> {
    serde_json::from_str(text).map_err(|error| StorageError::Database {
        detail: format!("{column} 不是合法 JSON：{error}"),
    })
}

fn parse_string_array(text: &str, column: &str) -> Result<Vec<String>, StorageError> {
    serde_json::from_str(text).map_err(|error| StorageError::Database {
        detail: format!("{column} 不是 JSON 字符串数组：{error}"),
    })
}

fn quote_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<QuoteRecord, StorageError> {
    let photo_ids_json: String = row.try_get("photo_ids")?;
    let photo_hashes_json: String = row.try_get("photo_hashes")?;
    let provider_config_json: String = row.try_get("provider_config")?;
    let confirmation_json: Option<String> = row.try_get("confirmation_json")?;
    let confirmed_at: Option<i64> = row.try_get("confirmed_at")?;
    let consumed_at: Option<i64> = row.try_get("consumed_at")?;
    Ok(QuoteRecord {
        id: row.try_get("id")?,
        item_id: row.try_get("item_id")?,
        preparation_id: row.try_get("preparation_id")?,
        photo_ids: parse_string_array(&photo_ids_json, "quotes.photo_ids")?,
        photo_hashes: parse_string_array(&photo_hashes_json, "quotes.photo_hashes")?,
        input_hash: row.try_get("input_hash")?,
        model_preset: row.try_get("model_preset")?,
        provider_config: parse_json(&provider_config_json, "quotes.provider_config")?,
        price_version: row.try_get("price_version")?,
        price_snapshot_date: row.try_get("price_snapshot_date")?,
        page_count: row.try_get("page_count")?,
        max_output_tokens: row.try_get("max_output_tokens")?,
        quote_json: row.try_get("quote_json")?,
        expires_at: Timestamp::from_millis(row.try_get("expires_at")?),
        confirmed_at: confirmed_at.map(Timestamp::from_millis),
        confirmation_json: match confirmation_json {
            Some(text) => Some(parse_json(&text, "quotes.confirmation_json")?),
            None => None,
        },
        consumed_at: consumed_at.map(Timestamp::from_millis),
        consumed_job_id: row.try_get("consumed_job_id")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
    })
}
