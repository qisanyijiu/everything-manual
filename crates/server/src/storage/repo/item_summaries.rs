//! Four bounded batch reads. No per-item details, mutations, or large draft/manifest blobs.
use super::{
    documents, items,
    photos::{self, PhotoWithHash},
    quotes::{self, QuoteRecord},
};
use crate::storage::error::StorageError;
use manual_core::domain::{Document, Item};
use sqlx::{Row, SqliteConnection};

pub struct ItemFacts {
    pub item: Item,
    pub attention_job: Option<String>,
    pub running_job: Option<String>,
    pub draft_id: Option<String>,
    pub release_id: Option<String>,
    pub release_draft_revision: Option<i64>,
    pub release_created_at: Option<i64>,
    pub had_preparation: bool,
}
pub struct Batch {
    pub items: Vec<ItemFacts>,
    pub documents: Vec<Document>,
    pub photos: Vec<PhotoWithHash>,
    pub quotes: Vec<QuoteRecord>,
}
/// Input ids are validated and deduplicated by HTTP (1..100). Result size <= 100 items/docs/quotes, <=400 photos.
pub async fn read(
    conn: &mut SqliteConnection,
    ids: &[String],
    document_id: Option<&str>,
) -> Result<Batch, StorageError> {
    let ids = serde_json::to_string(ids).expect("string IDs serialize");
    // Publishing CAS-bumps the draft but freezes its pre-publish content revision.
    // Recognize that exact post-publish revision only through the audit fact written
    // in the same transaction. A subsequent edit must remain a review task. Do not
    // infer publication from revision +/- 1 or read large release manifests here.
    let rows = sqlx::query("SELECT i.*,
      (SELECT j.id FROM jobs j WHERE j.item_id=i.id AND j.status IN ('needs_input','submission_unknown','failed') ORDER BY j.updated_at DESC,j.id DESC LIMIT 1) attention_job,
      (SELECT j.id FROM jobs j WHERE j.item_id=i.id AND j.status IN ('queued','running','waiting_provider','retry_wait') ORDER BY j.updated_at DESC,j.id DESC LIMIT 1) running_job,
      (SELECT d.id FROM manual_drafts d WHERE d.item_id=i.id AND NOT EXISTS (
        SELECT 1 FROM manual_releases r WHERE r.draft_id=d.id AND r.item_id=d.item_id AND (
          r.draft_revision=d.revision OR EXISTS (
            SELECT 1 FROM audit_events a
             WHERE a.entity_type='manual_release' AND a.entity_id=r.id
               AND a.action='release_published' AND a.result='published'
               AND a.created_at=r.created_at
               AND CASE WHEN json_valid(a.metadata_json) THEN
                 json_extract(a.metadata_json,'$.itemId')=d.item_id
                 AND json_extract(a.metadata_json,'$.draftId')=d.id
                 AND json_type(a.metadata_json,'$.draftRevision')='integer'
                 AND json_extract(a.metadata_json,'$.draftRevision')=r.draft_revision
                 AND json_type(a.metadata_json,'$.draftRevisionAfterPublish')='integer'
                 AND json_extract(a.metadata_json,'$.draftRevisionAfterPublish')=d.revision
               ELSE 0 END
          )
        )) ORDER BY d.updated_at DESC,d.id DESC LIMIT 1) draft_id,
      (SELECT r.id FROM manual_releases r WHERE r.item_id=i.id ORDER BY r.created_at DESC,r.id DESC LIMIT 1) release_id,
      (SELECT r.draft_revision FROM manual_releases r WHERE r.item_id=i.id ORDER BY r.created_at DESC,r.id DESC LIMIT 1) release_draft_revision,
      (SELECT r.created_at FROM manual_releases r WHERE r.item_id=i.id ORDER BY r.created_at DESC,r.id DESC LIMIT 1) release_created_at,
      EXISTS(SELECT 1 FROM preparations p JOIN documents d ON d.id=p.document_id WHERE d.item_id=i.id) had_preparation
      FROM items i JOIN json_each(?) selected ON selected.value=i.id ORDER BY i.id")
      .bind(&ids).fetch_all(&mut *conn).await?;
    let items = rows
        .iter()
        .map(|r| {
            Ok(ItemFacts {
                item: items::item_from_row(r)?,
                attention_job: r.try_get("attention_job")?,
                running_job: r.try_get("running_job")?,
                draft_id: r.try_get("draft_id")?,
                release_id: r.try_get("release_id")?,
                release_draft_revision: r.try_get("release_draft_revision")?,
                release_created_at: r.try_get("release_created_at")?,
                had_preparation: r.try_get("had_preparation")?,
            })
        })
        .collect::<Result<Vec<_>, StorageError>>()?;
    let rows = sqlx::query("WITH ranked AS (SELECT d.*,ROW_NUMBER() OVER(PARTITION BY d.item_id ORDER BY d.updated_at DESC,d.id DESC) ordinal FROM documents d JOIN json_each(?) selected ON selected.value=d.item_id WHERE (? IS NULL OR d.id=?)) SELECT * FROM ranked WHERE ordinal=1")
      .bind(&ids).bind(document_id).bind(document_id).fetch_all(&mut *conn).await?;
    let documents = rows
        .iter()
        .map(documents::document_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let rows = sqlx::query("SELECT p.*,b.sha256 AS blob_sha256 FROM photos p JOIN json_each(?) selected ON selected.value=p.item_id JOIN assets a ON a.id=p.asset_id AND a.item_id=p.item_id JOIN blobs b ON b.sha256=a.blob_id AND b.storage_state='stored' WHERE p.view<>'detail' ORDER BY p.item_id,CASE p.view WHEN 'front' THEN 1 WHEN 'left' THEN 2 WHEN 'back' THEN 3 WHEN 'right' THEN 4 ELSE 9 END,p.id")
      .bind(&ids).fetch_all(&mut *conn).await?;
    let photos = rows
        .iter()
        .map(|r| {
            Ok(PhotoWithHash {
                photo: photos::photo_from_row(r)?,
                sha256: r.try_get("blob_sha256")?,
            })
        })
        .collect::<Result<Vec<_>, StorageError>>()?;
    let rows = sqlx::query("WITH ranked AS (SELECT q.*,ROW_NUMBER() OVER(PARTITION BY q.item_id ORDER BY q.created_at DESC,q.id DESC) ordinal FROM quotes q JOIN json_each(?) selected ON selected.value=q.item_id) SELECT * FROM ranked WHERE ordinal=1")
      .bind(&ids).fetch_all(&mut *conn).await?;
    let quotes = rows
        .iter()
        .map(quotes::quote_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Batch {
        items,
        documents,
        photos,
        quotes,
    })
}
