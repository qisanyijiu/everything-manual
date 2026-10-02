//! Read-only workflow summaries. Memory/query batches are bounded, provider adapters are never called.
use crate::{
    config::{Settings, provider_overrides::ProviderConfigStore},
    http::{dto::*, error::ApiError},
    storage::repo::{item_summaries as repo, preparations::discovery},
};
use manual_core::{
    domain::{PhotoView, PreparationState},
    timestamps::Timestamp,
};
use sqlx::SqliteConnection;
use std::collections::BTreeMap;

pub async fn summarize(
    conn: &mut SqliteConnection,
    ids: &[String],
    selected_document: Option<&str>,
    settings: &Settings,
    config: &ProviderConfigStore,
) -> Result<ItemSummaryResponse, ApiError> {
    let facts = repo::read(conn, ids, selected_document)
        .await
        .map_err(ApiError::from_storage)?;
    // All-or-error is explicit: never silently omit a missing ID and let clients infer its state.
    if facts.items.len() != ids.len() || (selected_document.is_some() && facts.documents.len() != 1)
    {
        return Err(ApiError::not_found("物品或所选原件不存在"));
    }
    let documents: BTreeMap<_, _> = facts.documents.iter().map(|d| (d.id.as_str(), d)).collect();
    let document_ids: Vec<_> = facts.documents.iter().map(|d| d.id.clone()).collect();
    let items: BTreeMap<_, _> = facts
        .items
        .iter()
        .map(|f| (f.item.id.as_str(), &f.item))
        .collect();
    let photos: BTreeMap<_, Vec<_>> = items
        .keys()
        .map(|id| {
            (
                *id,
                facts
                    .photos
                    .iter()
                    .filter(|p| p.photo.item_id == *id)
                    .cloned()
                    .collect(),
            )
        })
        .collect();
    let quotes: BTreeMap<_, _> = facts
        .quotes
        .iter()
        .map(|q| (q.item_id.as_str(), q))
        .collect();
    let mut recommended: BTreeMap<String, (bool, usize, i64, String)> = BTreeMap::new();
    let mut quote_current: BTreeMap<String, bool> = BTreeMap::new();
    let mut before: Option<(i64, String)> = None;
    loop {
        let batch = discovery::batch_documents(
            conn,
            &document_ids,
            before.as_ref().map(|(t, id)| (*t, id.as_str())),
            None,
            100,
        )
        .await
        .map_err(ApiError::from_storage)?;
        if batch.is_empty() {
            break;
        }
        for fact in &batch {
            let prep = &fact.preparation;
            let Some(document) = documents.get(prep.document_id.as_str()) else {
                continue;
            };
            let readiness = crate::preparations::assess(fact, document, &settings.data_dir).await;
            if !readiness.compatible {
                continue;
            }
            let ready = prep.state == PreparationState::Ready;
            let rank = (
                ready,
                if ready {
                    0
                } else {
                    readiness.completed_page_count
                },
                prep.updated_at.as_millis(),
                prep.id.clone(),
            );
            if recommended
                .get(&document.item_id)
                .is_none_or(|old| rank > *old)
            {
                recommended.insert(document.item_id.clone(), rank);
            }
            if let Some(quote) = quotes.get(document.item_id.as_str())
                && quote.preparation_id == prep.id
                && ready
            {
                let current_photos = &photos[document.item_id.as_str()];
                let same_set = current_photos.len() == quote.photo_ids.len()
                    && current_photos
                        .iter()
                        .all(|p| quote.photo_ids.contains(&p.photo.id));
                let hash = crate::generation::estimate::recompute_input_hash(
                    items[document.item_id.as_str()],
                    prep,
                    current_photos,
                    &quote.model_preset,
                    &quote.provider_config,
                    &quote.price_version,
                )
                .await
                .map_err(ApiError::from)?;
                quote_current.insert(
                    document.item_id.clone(),
                    same_set && hash == quote.input_hash,
                );
            }
        }
        let last = &batch.last().expect("nonempty").preparation;
        before = Some((last.updated_at.as_millis(), last.id.clone()));
        if batch.len() < 100 {
            break;
        }
    }
    let mut data = Vec::with_capacity(facts.items.len());
    for fact in &facts.items {
        let id = &fact.item.id;
        let document = facts.documents.iter().find(|d| d.item_id == *id);
        let current_photos = &photos[id.as_str()];
        let views = current_photos
            .iter()
            .any(|p| p.photo.view == PhotoView::Front)
            && current_photos.iter().any(|p| {
                matches!(
                    p.photo.view,
                    PhotoView::Left | PhotoView::Right | PhotoView::Back
                )
            });
        let prep = recommended.get(id);
        let prepared = prep.is_some_and(|p| p.0);
        let quote = quotes.get(id.as_str()).copied();
        let current = quote_current.get(id).copied().unwrap_or(false);
        let usable_quote = if let Some(q) = quote {
            current
                && !q.is_expired(Timestamp::now())
                && config.ensure_generation_available().is_ok()
                && config.ensure_revision(&q.provider_config).is_ok()
                && settings
                    .price_catalog
                    .as_ref()
                    .is_some_and(|c| c.version == q.price_version)
                && !crate::generation::estimate::quote_model_issue(q).map_err(ApiError::from)?
        } else {
            false
        };
        use WorkflowStepState::{Complete, Missing, NeedsReview};
        let steps = WorkflowStepsDto {
            basic: if !fact.item.name.trim().is_empty() && !fact.item.model.trim().is_empty() {
                Complete
            } else {
                Missing
            },
            document: if document.is_some() {
                Complete
            } else {
                Missing
            },
            views: if views {
                Complete
            } else if quote.is_some() {
                NeedsReview
            } else {
                Missing
            },
            prepare: if prepared {
                Complete
            } else if fact.had_preparation && prep.is_none() {
                NeedsReview
            } else {
                Missing
            },
            confirm: if quote.is_some_and(|q| {
                current && q.consumed_job_id.is_some() || usable_quote && q.confirmed_at.is_some()
            }) {
                Complete
            } else if quote.is_some_and(|q| !usable_quote || q.consumed_job_id.is_some()) {
                NeedsReview
            } else {
                Missing
            },
        };
        let (action, target_id) = if let Some(id) = &fact.attention_job {
            (WorkflowAction::HandleJob, Some(id.clone()))
        } else if let Some(id) = &fact.running_job {
            (WorkflowAction::ViewJob, Some(id.clone()))
        } else if let Some(id) = &fact.draft_id {
            (WorkflowAction::ReviewDraft, Some(id.clone()))
        } else if let Some(id) = &fact.release_id {
            (WorkflowAction::ReadRelease, Some(id.clone()))
        } else if document.is_none() {
            (WorkflowAction::AddDocument, None)
        } else if !views {
            (WorkflowAction::AddViews, None)
        } else if !prepared {
            (WorkflowAction::Prepare, prep.map(|p| p.3.clone()))
        } else {
            (WorkflowAction::Confirm, quote.map(|q| q.id.clone()))
        };
        data.push(ItemSummaryDto {
            item_id: id.clone(),
            action,
            target_id,
            latest_release_id: fact.release_id.clone(),
            latest_release_draft_revision: fact.release_draft_revision,
            latest_release_created_at: fact
                .release_created_at
                .map(manual_core::timestamps::Timestamp::from_millis),
            document_id: document.map(|d| d.id.clone()),
            preparation_id: prep.map(|p| p.3.clone()),
            latest_quote_id: quote.map(|q| q.id.clone()),
            consumed_job_id: quote.and_then(|q| q.consumed_job_id.clone()),
            quote_expires_at: quote
                .filter(|q| q.consumed_job_id.is_none())
                .map(|q| q.expires_at),
            steps,
        });
    }
    Ok(ItemSummaryResponse { data })
}
