//! Read-only discovery. Bounded SQL batches, metadata-only blob checks, no migration on GET.
use crate::assets::blob_store;
use crate::http::dto::{
    PreparationCandidateDto, PreparationDto, PreparationListResponse, PreparationReadinessDto,
};
use crate::storage::error::StorageError;
use crate::storage::repo::preparations::discovery::{self, AssetFact, PreparationFacts};
use manual_core::domain::{AssetPurpose, Document, PageViewport, PreparationState};
use manual_core::validation::{MAX_PDF_PAGES, validate_page_viewport};
use sqlx::SqliteConnection;
use std::path::Path;

async fn usable(asset: &AssetFact, item: &str, purpose: &str, mime: &str, data_dir: &Path) -> bool {
    if asset.item_id.as_deref() != Some(item)
        || asset.purpose.as_deref() != Some(purpose)
        || asset.storage_state.as_deref() != Some("stored")
        || asset.mime.as_deref() != Some(mime)
    {
        return false;
    }
    let (Some(sha), Some(size)) = (&asset.sha256, asset.size) else {
        return false;
    };
    // Never construct a filesystem path from a corrupt digest.
    if sha.len() != 64 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) || size < 0 {
        return false;
    }
    tokio::fs::symlink_metadata(blob_store::blob_path(data_dir, sha))
        .await
        .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() == size as u64)
}

pub async fn assess(
    facts: &PreparationFacts,
    document: &Document,
    data_dir: &Path,
) -> PreparationReadinessDto {
    let prep = &facts.preparation;
    let mut valid = Vec::new();
    let mut invalid_shape = false;
    let mut missing_assets = false;
    for page in &facts.pages {
        let viewport = page
            .viewport
            .as_deref()
            .and_then(|json| serde_json::from_str::<PageViewport>(json).ok());
        let shape = (1..=MAX_PDF_PAGES).contains(&page.number)
            && viewport.is_some_and(|value| validate_page_viewport(value).is_ok())
            && prep.page_count.is_none_or(|total| page.number <= total);
        let image = usable(
            &page.image,
            &document.item_id,
            AssetPurpose::PageImage.as_str(),
            "image/jpeg",
            data_dir,
        )
        .await;
        let text = page.text_id.is_none()
            || usable(
                &page.text,
                &document.item_id,
                AssetPurpose::PageText.as_str(),
                "text/plain; charset=utf-8",
                data_dir,
            )
            .await;
        invalid_shape |= !shape;
        missing_assets |= !image || !text;
        if shape && image && text {
            valid.push(page.number);
        }
    }
    let total_valid = prep
        .page_count
        .is_none_or(|value| (1..=MAX_PDF_PAGES).contains(&value));
    let missing = prep
        .page_count
        .filter(|_| total_valid)
        .map_or_else(Vec::new, |total| {
            (1..=total)
                .filter(|number| !valid.contains(number))
                .collect()
        });
    let version_known = facts.format_version == Some(1)
        || (facts.format_version.is_none()
            && !facts.pages.is_empty()
            && !invalid_shape
            && !missing_assets);
    let problem = if prep.document_id != document.id || prep.source_sha256 != document.source_sha256
    {
        Some((
            "sourceMismatch",
            "记录不属于当前原件或原件哈希已变化，请重新准备。",
        ))
    } else if facts.format_version.is_some_and(|version| version != 1) {
        Some((
            "unsupportedFormat",
            "准备格式不受支持，请重新准备；旧记录保留。",
        ))
    } else if invalid_shape || !total_valid {
        Some((
            "invalidPages",
            "页码或页图坐标格式不符合 v1，请重新准备；旧记录保留。",
        ))
    } else if missing_assets {
        Some((
            "missingAssets",
            "页资产缺失、归属或格式不符，无法复用；请重新准备。",
        ))
    } else if !version_known {
        Some((
            "unsupportedFormat",
            "旧记录没有可核实的 v1 页资产或格式标识，请重新准备。",
        ))
    } else if prep.state == PreparationState::Ready
        && (prep.page_count.is_none() || !missing.is_empty())
    {
        Some((
            "incompleteReady",
            "已封存记录缺少完整页资产，不能直接使用；请重新准备。",
        ))
    } else {
        None
    };
    PreparationReadinessDto {
        format_version: version_known.then(|| "v1".to_owned()),
        compatible: problem.is_none(),
        reason: problem.map(|(code, _)| code.to_owned()),
        explanation: problem.map(|(_, message)| message.to_owned()),
        completed_page_count: valid.len(),
        completed_pages: valid,
        missing_pages: missing,
    }
}

pub async fn inspect(
    conn: &mut SqliteConnection,
    document: &Document,
    id: &str,
    data_dir: &Path,
) -> Result<PreparationReadinessDto, StorageError> {
    let facts = discovery::batch(conn, &document.id, None, Some(id), 1).await?;
    let facts = facts.first().ok_or_else(|| StorageError::NotFound {
        entity: "preparation",
        id: id.to_owned(),
    })?;
    Ok(assess(facts, document, data_dir).await)
}

fn rank(candidate: &PreparationCandidateDto) -> (bool, usize, i64, &str) {
    let preparation = &candidate.preparation;
    (
        preparation.state == PreparationState::Ready,
        if preparation.state == PreparationState::Ready {
            0
        } else {
            candidate.readiness.completed_page_count
        },
        preparation.updated_at.as_millis(),
        &preparation.id,
    )
}

pub async fn discover(
    conn: &mut SqliteConnection,
    document: &Document,
    data_dir: &Path,
    cursor: Option<(i64, String)>,
    limit: u32,
) -> Result<PreparationListResponse, StorageError> {
    let mut page = Vec::new();
    let mut recommended: Option<PreparationCandidateDto> = None;
    let mut before: Option<(i64, String)> = None;
    loop {
        let batch = discovery::batch(
            conn,
            &document.id,
            before.as_ref().map(|(time, id)| (*time, id.as_str())),
            None,
            100,
        )
        .await?;
        if batch.is_empty() {
            break;
        }
        for facts in &batch {
            let candidate = PreparationCandidateDto {
                preparation: PreparationDto::from_preparation(&facts.preparation),
                readiness: assess(facts, document, data_dir).await,
            };
            let key = (
                candidate.preparation.updated_at.as_millis(),
                candidate.preparation.id.as_str(),
            );
            if cursor
                .as_ref()
                .is_none_or(|(time, id)| key < (*time, id.as_str()))
                && page.len() <= limit as usize
            {
                page.push(candidate.clone());
            }
            if candidate.readiness.compatible
                && recommended
                    .as_ref()
                    .is_none_or(|old| rank(&candidate) > rank(old))
            {
                recommended = Some(candidate);
            }
        }
        let last = &batch.last().expect("nonempty").preparation;
        before = Some((last.updated_at.as_millis(), last.id.clone()));
    }
    let has_more = page.len() > limit as usize;
    page.truncate(limit as usize);
    let next_cursor = if has_more {
        page.last().map(|last| {
            crate::http::pagination::Cursor::encode(
                &format!("preparations:{}", document.id),
                last.preparation.updated_at.as_millis(),
                &last.preparation.id,
            )
        })
    } else {
        None
    };
    Ok(PreparationListResponse {
        data: page,
        next_cursor,
        recommended_preparation_id: recommended
            .as_ref()
            .map(|entry| entry.preparation.id.clone()),
        recommended,
    })
}
