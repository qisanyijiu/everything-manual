//! `preparations` / `pages` 表的仓储原语（T09 / REQ-014、REQ-015）。
//!
//! **边界**：只做持久化与数据库层不变量（state、revision CAS、页号唯一）。
//! 资产归属 / purpose / 体积 / 页数上限等业务校验在 HTTP 服务层
//! （`http::preparations`）完成；外键与复合主键是最后的兜底。
//!
//! 关键语义（QA 按此复核）：
//! - **页号 1-based 唯一**：`(preparation_id, page_number)` 是复合主键（0001 迁移）。
//! - **ready 后不可修改**：写函数在自己的事务内先读 `state`，非 `preparing` 一律
//!   [`StorageError::NotWritable`]，不依赖调用方"先查再写"（避免 TOCTOU）。
//! - **revision CAS**：仅当页内容发生变化时自增 `preparations.revision`；
//!   覆盖已有页要求调用方给出期望 revision（不匹配 → `RevisionConflict`）。
//! - **幂等按内容哈希**：相同内容（blob sha256 与 viewport 都一致）重复提交不写库、
//!   不自增 revision，返回 [`PageWriteOutcome::Unchanged`]。
//! - **complete 的校验在同一事务内**：页号连续 1..N、每页有页图、资产属于该物品且
//!   blob 处于 `stored`；失败回滚，不留 ready 的半状态，也不创建 job / 费用记录。
//!
//! 所有函数接受 `&mut SqliteConnection`，可与其它写入放进同一短事务。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::{Page, PageViewport, Preparation, PreparationState};
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 新建 preparation 的输入（服务器生成 id 与时间戳）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPreparation {
    pub document_id: String,
    pub source_sha256: String,
}

/// 写入单页的输入（调用方已完成资产归属/purpose/viewport 校验）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPage {
    pub page_number: i64,
    pub text_asset_id: Option<String>,
    pub image_asset_id: Option<String>,
    pub viewport: PageViewport,
}

/// 页写入结果：区分"新建/覆盖"与"同内容幂等命中"（后者不自增 revision）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageWriteOutcome {
    /// 新页或内容已变化的覆盖（preparation.revision 自增）。
    Changed,
    /// 相同内容（资产 blob 与 viewport 都一致）重复提交：不写库、不自增 revision。
    Unchanged,
}

/// 页写入失败（业务分支与存储错误分开，HTTP 层分别映射 428/412/422）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageWriteError {
    /// 内容变化但调用方未提供 `If-Match`（HTTP 层映射 428）。
    PreconditionRequired,
    /// preparation 不存在 / 已 ready / revision 冲突等。
    Storage(StorageError),
}

/// complete 的失败分类（缺页 / 资产不可用），供 HTTP 层映射 422 并列出缺项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteFailure {
    /// 声明了 N 页但 1..N 中有页缺失（缺失页号升序）。
    MissingPages { missing: Vec<i64> },
    /// 某页的资产不属于该物品 / 内容不可用（哈希不可核）。
    AssetMismatch { details: Vec<PageAssetProblem> },
}

/// 单页的资产问题（complete 422 的 `details.pages[]` 明细）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageAssetProblem {
    pub page_number: i64,
    pub problem: String,
}

/// complete 的失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteError {
    /// 业务校验失败（HTTP 422，列出缺项）。
    Incomplete(CompleteFailure),
    /// 存储/并发错误。
    Storage(StorageError),
}

/// 插入 preparation 行并返回完整领域对象。
pub async fn create(
    conn: &mut SqliteConnection,
    new: NewPreparation,
) -> Result<Preparation, StorageError> {
    let now = Timestamp::now();
    let preparation = Preparation {
        id: ids::new_id(),
        document_id: new.document_id,
        source_sha256: new.source_sha256,
        state: PreparationState::Preparing,
        page_count: None,
        client_derived: false,
        revision: 1,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO preparations \
           (id, document_id, source_sha256, state, page_count, client_derived, revision, created_at, updated_at) \
         VALUES (?, ?, ?, ?, NULL, 0, ?, ?, ?)",
    )
    .bind(&preparation.id)
    .bind(&preparation.document_id)
    .bind(&preparation.source_sha256)
    .bind(preparation.state.as_str())
    .bind(preparation.revision)
    .bind(preparation.created_at.as_millis())
    .bind(preparation.updated_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(preparation)
}

/// 按 id 读取 preparation。
pub async fn get(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<Preparation>, StorageError> {
    let row = sqlx::query(SELECT_PREPARATION_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| preparation_from_row(&row)).transpose()
}

/// 同一 document + 原件哈希下**未完成**的 preparation（断线续传的复用入口）。
///
/// 取最新一条：同一原件可能先后存在多个记录（例如 ready 后重新准备），
/// 只有 `preparing` 的记录可继续写入。
pub async fn find_preparing_for_document(
    conn: &mut SqliteConnection,
    document_id: &str,
    source_sha256: &str,
) -> Result<Option<Preparation>, StorageError> {
    let row = sqlx::query(
        "SELECT id, document_id, source_sha256, state, page_count, client_derived, revision, created_at, updated_at \
           FROM preparations \
          WHERE document_id = ? AND source_sha256 = ? AND state = 'preparing' \
          ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(document_id)
    .bind(source_sha256)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| preparation_from_row(&row)).transpose()
}

/// preparation 所属物品 id（经 document 关联；完整性由外键保证）。
pub async fn item_id_of(
    conn: &mut SqliteConnection,
    preparation_id: &str,
) -> Result<Option<String>, StorageError> {
    let row = sqlx::query(
        "SELECT d.item_id FROM preparations p JOIN documents d ON d.id = p.document_id \
          WHERE p.id = ?",
    )
    .bind(preparation_id)
    .fetch_optional(&mut *conn)
    .await?;
    match row {
        Some(row) => Ok(Some(row.try_get("item_id")?)),
        None => Ok(None),
    }
}

/// 某 preparation 的全部页（按页号升序）。
pub async fn list_pages(
    conn: &mut SqliteConnection,
    preparation_id: &str,
) -> Result<Vec<Page>, StorageError> {
    let rows = sqlx::query(
        "SELECT preparation_id, page_number, text_asset_id, image_asset_id, viewport_json, created_at, updated_at \
           FROM pages WHERE preparation_id = ? ORDER BY page_number ASC",
    )
    .bind(preparation_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(page_from_row).collect()
}

/// 报价输入：每页的文字层内容字节数（`None` = 无文字层/内容缺失，按发送页图计）。
///
/// 报价的 token 保守上界需要"这页发文字还是发页图"以及文字体量；用 JOIN 一次取回，
/// 不逐页查询。页图资产是否存在由 `complete()` 保证，这里不重复校验。
pub async fn page_quote_inputs(
    conn: &mut SqliteConnection,
    preparation_id: &str,
) -> Result<Vec<manual_core::generation::PageInput>, StorageError> {
    let rows = sqlx::query(
        "SELECT p.page_number, tb.size AS text_size \
           FROM pages p \
           LEFT JOIN assets ta ON ta.id = p.text_asset_id \
           LEFT JOIN blobs tb ON tb.sha256 = ta.blob_id \
          WHERE p.preparation_id = ? ORDER BY p.page_number ASC",
    )
    .bind(preparation_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut inputs = Vec::with_capacity(rows.len());
    for row in rows {
        let text_size: Option<i64> = row.try_get("text_size")?;
        inputs.push(manual_core::generation::PageInput {
            page_number: row.try_get("page_number")?,
            text_bytes: text_size,
        });
    }
    Ok(inputs)
}

/// 单页读取。
pub async fn get_page(
    conn: &mut SqliteConnection,
    preparation_id: &str,
    page_number: i64,
) -> Result<Option<Page>, StorageError> {
    let row = sqlx::query(
        "SELECT preparation_id, page_number, text_asset_id, image_asset_id, viewport_json, created_at, updated_at \
           FROM pages WHERE preparation_id = ? AND page_number = ?",
    )
    .bind(preparation_id)
    .bind(page_number)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| page_from_row(&row)).transpose()
}

/// 单页写入（新建或覆盖），带 ready 门禁、幂等判定与 revision CAS。
///
/// 语义：
/// - preparation 不存在 → [`PageWriteError::Storage`]（`NotFound`）；
/// - preparation 已 ready → `NotWritable`（422）；
/// - 内容指纹（text/image blob sha256）与 viewport 都相同 → [`PageWriteOutcome::Unchanged`]；
/// - 内容变化：`expected_revision` 为 `Some` 时按 CAS 更新（不匹配 → `RevisionConflict`）；
///   为 `None` 时返回 [`PageWriteError::PreconditionRequired`]（HTTP 428），
///   不再有"静默覆盖"路径。
///
/// 判定与写入在同一事务内完成，避免调用方"先查内容再写"的 TOCTOU。
pub async fn write_page(
    conn: &mut SqliteConnection,
    preparation_id: &str,
    page: NewPage,
    expected_revision: Option<i64>,
) -> Result<(Page, PageWriteOutcome), PageWriteError> {
    // `BEGIN IMMEDIATE`（BUG-006）：`write_page_in_tx` 先读（preparation/页/资产）后写。
    let mut tx = crate::storage::begin_write(conn)
        .await
        .map_err(|error| PageWriteError::Storage(error.into()))?;
    match write_page_in_tx(&mut tx, preparation_id, page, expected_revision).await {
        Ok(result) => {
            tx.commit()
                .await
                .map_err(|error| PageWriteError::Storage(error.into()))?;
            Ok(result)
        }
        Err(error) => {
            tx.rollback().await.ok();
            Err(error)
        }
    }
}

async fn write_page_in_tx(
    tx: &mut Tx<'_>,
    preparation_id: &str,
    page: NewPage,
    expected_revision: Option<i64>,
) -> Result<(Page, PageWriteOutcome), PageWriteError> {
    let storage = PageWriteError::Storage;
    let preparation = load_in_tx(tx, preparation_id).await.map_err(storage)?;
    if preparation.state != PreparationState::Preparing {
        return Err(storage(StorageError::NotWritable {
            entity: "preparation",
            id: preparation_id.to_owned(),
            state: preparation.state.as_str().to_owned(),
        }));
    }

    let existing = get_page_in_tx(tx, preparation_id, page.page_number)
        .await
        .map_err(storage)?;
    let now = Timestamp::now();

    let next_text = match &page.text_asset_id {
        Some(id) => asset_blob_id(tx, id).await.map_err(storage)?,
        None => None,
    };
    let next_image = match &page.image_asset_id {
        Some(id) => asset_blob_id(tx, id).await.map_err(storage)?,
        None => None,
    };

    if let Some(existing) = &existing {
        let existing_text = match &existing.text_asset_id {
            Some(id) => asset_blob_id(tx, id).await.map_err(storage)?,
            None => None,
        };
        let existing_image = match &existing.image_asset_id {
            Some(id) => asset_blob_id(tx, id).await.map_err(storage)?,
            None => None,
        };
        // 幂等：内容哈希（blob sha256）与 viewport 都相同 → 不写库、不自增 revision。
        if existing_text == next_text
            && existing_image == next_image
            && existing.viewport == Some(page.viewport)
        {
            return Ok((existing.clone(), PageWriteOutcome::Unchanged));
        }

        let Some(expected) = expected_revision else {
            return Err(PageWriteError::PreconditionRequired);
        };
        let updated = sqlx::query(
            "UPDATE preparations SET revision = revision + 1, updated_at = ? \
              WHERE id = ? AND revision = ? AND state = 'preparing'",
        )
        .bind(now.as_millis())
        .bind(preparation_id)
        .bind(expected)
        .execute(&mut **tx)
        .await
        .map_err(|error| storage(error.into()))?;
        if updated.rows_affected() == 0 {
            let current = load_in_tx(tx, preparation_id).await.map_err(storage)?;
            return Err(storage(StorageError::RevisionConflict {
                entity: "preparation",
                id: preparation_id.to_owned(),
                current_revision: current.revision,
            }));
        }

        sqlx::query(
            "UPDATE pages SET text_asset_id = ?, image_asset_id = ?, viewport_json = ?, updated_at = ? \
              WHERE preparation_id = ? AND page_number = ?",
        )
        .bind(&page.text_asset_id)
        .bind(&page.image_asset_id)
        .bind(viewport_json(&page.viewport))
        .bind(now.as_millis())
        .bind(preparation_id)
        .bind(page.page_number)
        .execute(&mut **tx)
        .await
        .map_err(|error| storage(error.into()))?;
    } else {
        sqlx::query(
            "INSERT INTO pages \
               (preparation_id, page_number, text_asset_id, image_asset_id, viewport_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(preparation_id)
        .bind(page.page_number)
        .bind(&page.text_asset_id)
        .bind(&page.image_asset_id)
        .bind(viewport_json(&page.viewport))
        .bind(now.as_millis())
        .bind(now.as_millis())
        .execute(&mut **tx)
        .await
        .map_err(|error| storage(error.into()))?;
        sqlx::query("UPDATE preparations SET revision = revision + 1, updated_at = ? WHERE id = ?")
            .bind(now.as_millis())
            .bind(preparation_id)
            .execute(&mut **tx)
            .await
            .map_err(|error| storage(error.into()))?;
    }

    let written = get_page_in_tx(tx, preparation_id, page.page_number)
        .await
        .map_err(storage)?
        .ok_or_else(|| {
            storage(StorageError::NotFound {
                entity: "page",
                id: format!("{preparation_id}#{}", page.page_number),
            })
        })?;
    Ok((written, PageWriteOutcome::Changed))
}

/// 封存 preparation：事务内校验页集合并置为 ready。
///
/// 校验（失败返回 [`CompleteFailure`]，由 HTTP 层映射 422 并列出缺项）：
/// 1. 1..=page_count 的每一页都存在；
/// 2. 每页至少有一个页图资产（扫描页允许没有页文字）；
/// 3. 页资产属于该物品且 blob 处于 `stored`（内容寻址下 blob id 即 sha256，
///    "哈希不符"在读取层面等价于内容不可用）。
///
/// 成功后：`state = ready`、`page_count = N`、`client_derived = 1`、revision 自增。
/// **不创建 job、不写费用账本、不产生任何外呼**（REQ-015/AC-025）。
pub async fn complete(
    conn: &mut SqliteConnection,
    preparation_id: &str,
    item_id: &str,
    page_count: i64,
    expected_revision: i64,
) -> Result<Preparation, CompleteError> {
    // `BEGIN IMMEDIATE`（BUG-006）：`complete_in_tx` 先读（preparation/页集合）后写（封存）。
    let mut tx = crate::storage::begin_write(conn)
        .await
        .map_err(|error| CompleteError::Storage(error.into()))?;
    match complete_in_tx(
        &mut tx,
        preparation_id,
        item_id,
        page_count,
        expected_revision,
    )
    .await
    {
        Ok(sealed) => {
            tx.commit()
                .await
                .map_err(|error| CompleteError::Storage(error.into()))?;
            Ok(sealed)
        }
        Err(error) => {
            tx.rollback().await.ok();
            Err(error)
        }
    }
}

async fn complete_in_tx(
    tx: &mut Tx<'_>,
    preparation_id: &str,
    item_id: &str,
    page_count: i64,
    expected_revision: i64,
) -> Result<Preparation, CompleteError> {
    let storage = CompleteError::Storage;
    let preparation = load_in_tx(tx, preparation_id).await.map_err(storage)?;
    if preparation.state != PreparationState::Preparing {
        return Err(storage(StorageError::NotWritable {
            entity: "preparation",
            id: preparation_id.to_owned(),
            state: preparation.state.as_str().to_owned(),
        }));
    }
    if preparation.revision != expected_revision {
        return Err(storage(StorageError::RevisionConflict {
            entity: "preparation",
            id: preparation_id.to_owned(),
            current_revision: preparation.revision,
        }));
    }

    let pages = list_pages_in_tx(tx, preparation_id)
        .await
        .map_err(storage)?;
    let present: std::collections::HashSet<i64> =
        pages.iter().map(|page| page.page_number).collect();
    let missing: Vec<i64> = (1..=page_count)
        .filter(|number| !present.contains(number))
        .collect();
    if !missing.is_empty() {
        return Err(CompleteError::Incomplete(CompleteFailure::MissingPages {
            missing,
        }));
    }

    let mut problems: Vec<PageAssetProblem> = Vec::new();
    for page in pages.iter().filter(|page| page.page_number <= page_count) {
        let Some(image_asset_id) = page.image_asset_id.as_deref() else {
            problems.push(PageAssetProblem {
                page_number: page.page_number,
                problem: "缺少页图资产（扫描页也需要上传页图）".to_owned(),
            });
            continue;
        };
        for (label, asset_id) in [
            ("页图", Some(image_asset_id)),
            ("页文字", page.text_asset_id.as_deref()),
        ] {
            let Some(asset_id) = asset_id else { continue };
            match asset_usable(tx, item_id, asset_id).await.map_err(storage)? {
                None => {}
                Some(reason) => problems.push(PageAssetProblem {
                    page_number: page.page_number,
                    problem: format!("{label}资产不可用：{reason}"),
                }),
            }
        }
    }
    if !problems.is_empty() {
        return Err(CompleteError::Incomplete(CompleteFailure::AssetMismatch {
            details: problems,
        }));
    }

    let now = Timestamp::now();
    let updated = sqlx::query(
        "UPDATE preparations \
            SET state = 'ready', page_count = ?, client_derived = 1, revision = revision + 1, updated_at = ? \
          WHERE id = ? AND revision = ? AND state = 'preparing'",
    )
    .bind(page_count)
    .bind(now.as_millis())
    .bind(preparation_id)
    .bind(expected_revision)
    .execute(&mut **tx)
    .await
    .map_err(|error| storage(error.into()))?;
    if updated.rows_affected() == 0 {
        let current = load_in_tx(tx, preparation_id).await.map_err(storage)?;
        return Err(storage(StorageError::RevisionConflict {
            entity: "preparation",
            id: preparation_id.to_owned(),
            current_revision: current.revision,
        }));
    }

    load_in_tx(tx, preparation_id).await.map_err(storage)
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

const SELECT_PREPARATION_SQL: &str = "SELECT id, document_id, source_sha256, state, page_count, \
     client_derived, revision, created_at, updated_at FROM preparations WHERE id = ?";

/// 本模块内部使用的短事务别名（外部只经 `&mut SqliteConnection` 调用）。
type Tx<'a> = sqlx::Transaction<'a, sqlx::Sqlite>;

async fn load_in_tx(tx: &mut Tx<'_>, id: &str) -> Result<Preparation, StorageError> {
    let row = sqlx::query(SELECT_PREPARATION_SQL)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| StorageError::NotFound {
            entity: "preparation",
            id: id.to_owned(),
        })?;
    preparation_from_row(&row)
}

async fn list_pages_in_tx(
    tx: &mut Tx<'_>,
    preparation_id: &str,
) -> Result<Vec<Page>, StorageError> {
    let rows = sqlx::query(
        "SELECT preparation_id, page_number, text_asset_id, image_asset_id, viewport_json, created_at, updated_at \
           FROM pages WHERE preparation_id = ? ORDER BY page_number ASC",
    )
    .bind(preparation_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.iter().map(page_from_row).collect()
}

async fn get_page_in_tx(
    tx: &mut Tx<'_>,
    preparation_id: &str,
    page_number: i64,
) -> Result<Option<Page>, StorageError> {
    let row = sqlx::query(
        "SELECT preparation_id, page_number, text_asset_id, image_asset_id, viewport_json, created_at, updated_at \
           FROM pages WHERE preparation_id = ? AND page_number = ?",
    )
    .bind(preparation_id)
    .bind(page_number)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| page_from_row(&row)).transpose()
}

async fn asset_blob_id(tx: &mut Tx<'_>, asset_id: &str) -> Result<Option<String>, StorageError> {
    let row = sqlx::query("SELECT blob_id FROM assets WHERE id = ?")
        .bind(asset_id)
        .fetch_optional(&mut **tx)
        .await?;
    match row {
        Some(row) => Ok(Some(row.try_get("blob_id")?)),
        None => Ok(None),
    }
}

/// 单页资产可用性：`Ok(None)` 表示可用；`Ok(Some(reason))` 表示不可用。
async fn asset_usable(
    tx: &mut Tx<'_>,
    item_id: &str,
    asset_id: &str,
) -> Result<Option<String>, StorageError> {
    let row = sqlx::query(
        "SELECT a.item_id, a.blob_id, b.storage_state, b.mime \
           FROM assets a JOIN blobs b ON b.sha256 = a.blob_id WHERE a.id = ?",
    )
    .bind(asset_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(Some("资产不存在".to_owned()));
    };
    let asset_item: String = row.try_get("item_id")?;
    if asset_item != item_id {
        return Ok(Some("资产不属于该物品".to_owned()));
    }
    let storage_state: String = row.try_get("storage_state")?;
    if storage_state != "stored" {
        return Ok(Some(format!("内容不可用（storage_state={storage_state}）")));
    }
    let mime: String = row.try_get("mime")?;
    if mime.is_empty() {
        return Ok(Some("内容类型缺失".to_owned()));
    }
    Ok(None)
}

fn viewport_json(viewport: &PageViewport) -> String {
    serde_json::to_string(viewport).expect("viewport 可序列化")
}

fn preparation_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Preparation, StorageError> {
    let state: String = row.try_get("state")?;
    let client_derived: i64 = row.try_get("client_derived")?;
    Ok(Preparation {
        id: row.try_get("id")?,
        document_id: row.try_get("document_id")?,
        source_sha256: row.try_get("source_sha256")?,
        state: parse_state(&state)?,
        page_count: row.try_get("page_count")?,
        client_derived: client_derived != 0,
        revision: row.try_get("revision")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
        updated_at: Timestamp::from_millis(row.try_get("updated_at")?),
    })
}

fn page_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Page, StorageError> {
    let viewport_json: Option<String> = row.try_get("viewport_json")?;
    let viewport = match viewport_json {
        None => None,
        Some(json) => Some(
            serde_json::from_str::<PageViewport>(&json).map_err(|error| {
                StorageError::ConstraintViolation {
                    detail: format!("pages.viewport_json 不是合法 viewport：{error}"),
                }
            })?,
        ),
    };
    Ok(Page {
        preparation_id: row.try_get("preparation_id")?,
        page_number: row.try_get("page_number")?,
        text_asset_id: row.try_get("text_asset_id")?,
        image_asset_id: row.try_get("image_asset_id")?,
        viewport,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
        updated_at: Timestamp::from_millis(row.try_get("updated_at")?),
    })
}

/// SQL 值 → 领域枚举；未知值视为数据损坏。
pub fn parse_state(value: &str) -> Result<PreparationState, StorageError> {
    match value {
        "preparing" => Ok(PreparationState::Preparing),
        "ready" => Ok(PreparationState::Ready),
        other => Err(StorageError::ConstraintViolation {
            detail: format!("preparations.state 出现未知值：{other}"),
        }),
    }
}
