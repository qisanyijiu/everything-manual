//! Bounded read batches for preparation discovery. No mutations or blob reads.
use crate::storage::error::StorageError;
use manual_core::domain::Preparation;
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

pub struct AssetFact {
    pub item_id: Option<String>,
    pub purpose: Option<String>,
    pub sha256: Option<String>,
    pub mime: Option<String>,
    pub storage_state: Option<String>,
    pub size: Option<i64>,
}
pub struct PageFact {
    pub number: i64,
    pub viewport: Option<String>,
    pub image: AssetFact,
    pub text_id: Option<String>,
    pub text: AssetFact,
}
pub struct PreparationFacts {
    pub preparation: Preparation,
    pub format_version: Option<i64>,
    pub pages: Vec<PageFact>,
}

/// At most 100 preparation rows plus their bounded page rows; two SQL queries per batch.
pub async fn batch(
    conn: &mut SqliteConnection,
    document_id: &str,
    before: Option<(i64, &str)>,
    only_id: Option<&str>,
    limit: u32,
) -> Result<Vec<PreparationFacts>, StorageError> {
    batch_documents(conn, &[document_id.to_owned()], before, only_id, limit).await
}

/// Shared bounded scan for at most 100 selected documents (item workflow summaries).
pub async fn batch_documents(
    conn: &mut SqliteConnection,
    document_ids: &[String],
    before: Option<(i64, &str)>,
    only_id: Option<&str>,
    limit: u32,
) -> Result<Vec<PreparationFacts>, StorageError> {
    if document_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, document_id, source_sha256, state, page_count, client_derived, revision, created_at, updated_at, format_version FROM preparations WHERE document_id IN (",
    );
    let mut ids = query.separated(", ");
    for id in document_ids.iter().take(100) {
        ids.push_bind(id);
    }
    ids.push_unseparated(")");
    if let Some((millis, id)) = before {
        query
            .push(" AND (updated_at, id) < (")
            .push_bind(millis)
            .push(", ")
            .push_bind(id)
            .push(")");
    }
    if let Some(id) = only_id {
        query.push(" AND id = ").push_bind(id);
    }
    query
        .push(" ORDER BY updated_at DESC, id DESC LIMIT ")
        .push_bind(limit.min(100));
    let rows = query.build().fetch_all(&mut *conn).await?;
    let mut facts = rows
        .iter()
        .map(|row| {
            Ok(PreparationFacts {
                preparation: super::preparation_from_row(row)?,
                format_version: row.try_get("format_version")?,
                pages: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, StorageError>>()?;
    if facts.is_empty() {
        return Ok(facts);
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH selected_pages AS (SELECT p.*, ROW_NUMBER() OVER (PARTITION BY preparation_id ORDER BY page_number) AS ordinal FROM pages p WHERE preparation_id IN (",
    );
    let mut separated = query.separated(", ");
    for entry in &facts {
        separated.push_bind(&entry.preparation.id);
    }
    separated.push_unseparated(")), bounded_pages AS (SELECT * FROM selected_pages WHERE ordinal <= 101) SELECT p.preparation_id, p.page_number, p.viewport_json, p.text_asset_id, ia.item_id AS i_item, ia.purpose AS i_purpose, ib.sha256 AS i_sha, ib.mime AS i_mime, ib.storage_state AS i_state, ib.size AS i_size, ta.item_id AS t_item, ta.purpose AS t_purpose, tb.sha256 AS t_sha, tb.mime AS t_mime, tb.storage_state AS t_state, tb.size AS t_size FROM bounded_pages p LEFT JOIN assets ia ON ia.id = p.image_asset_id LEFT JOIN blobs ib ON ib.sha256 = ia.blob_id LEFT JOIN assets ta ON ta.id = p.text_asset_id LEFT JOIN blobs tb ON tb.sha256 = ta.blob_id ORDER BY p.preparation_id, p.page_number");
    for row in query.build().fetch_all(&mut *conn).await? {
        let id: String = row.try_get("preparation_id")?;
        let target = facts
            .iter_mut()
            .find(|entry| entry.preparation.id == id)
            .expect("selected IDs only");
        let asset = |prefix: &str| -> Result<AssetFact, sqlx::Error> {
            Ok(AssetFact {
                item_id: row.try_get(format!("{prefix}_item").as_str())?,
                purpose: row.try_get(format!("{prefix}_purpose").as_str())?,
                sha256: row.try_get(format!("{prefix}_sha").as_str())?,
                mime: row.try_get(format!("{prefix}_mime").as_str())?,
                storage_state: row.try_get(format!("{prefix}_state").as_str())?,
                size: row.try_get(format!("{prefix}_size").as_str())?,
            })
        };
        target.pages.push(PageFact {
            number: row.try_get("page_number")?,
            viewport: row.try_get("viewport_json")?,
            image: asset("i")?,
            text_id: row.try_get("text_asset_id")?,
            text: asset("t")?,
        });
    }
    Ok(facts)
}
