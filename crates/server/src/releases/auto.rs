//! 自动发布：生成任务结束后，把草稿直接整理为已发布版本（取消人工「阅读与复核」环节）。
//!
//! 由 `serve` 装配的执行器在父任务进入终态（succeeded / failed）后调用：
//! 1. 找到该任务快照对应的草稿；没有模型或知识时无法发布，记录原因后跳过；
//! 2. 以系统身份写一次受限 PATCH：全部部件/步骤/规格标记已确认、模型复核两个声明为真、
//!    候选热点升为已确认、没有可用热点的部件标为「仅文本条目」；
//! 3. 走与人工发布相同的 [`super::service::publish_draft`]（同一发布不变量与 manifest 冻结），
//!    幂等键由任务与草稿 revision 派生：重复触发返回同一版本，不重复发布。
//!
//! 发布失败不影响任务状态：草稿保留，原因进服务端日志。

use std::collections::BTreeMap;
use std::path::Path;

use manual_core::knowledge::ReviewStatus;
use manual_core::timestamps::Timestamp;
use sqlx::SqlitePool;

use crate::drafts::aggregate::{
    DraftPatch, EntityReviewPatch, HotspotPatch, HotspotStatus, HotspotUpsert, ModelReviewPatch,
    ReviewOverlay,
};
use crate::drafts::knowledge::DraftKnowledge;
use crate::storage::repo;

/// 系统写入草稿时的操作者标识（审计可区分人工与自动）。
pub const AUTO_PUBLISH_ACTOR: &str = "system:auto_publish";

/// 自动发布结果（日志与测试断言用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoPublishOutcome {
    Published { release_id: String, replayed: bool },
    Skipped { reason: String },
}

/// 构造"整体确认"补丁（纯函数；已确认/已修订的条目不重复写入）。
pub fn confirm_all_patch(knowledge: &DraftKnowledge, overlay: &ReviewOverlay) -> DraftPatch {
    let Some(merged) = knowledge.knowledge.as_ref() else {
        return DraftPatch::default();
    };
    let model = knowledge.model.as_ref();
    let matches_model =
        |hotspot: &crate::drafts::aggregate::Hotspot| match (hotspot.anchor.as_ref(), model) {
            (Some(anchor), Some(model)) => {
                anchor.has_finite_values()
                    && anchor.matches_model(&model.revision_id, &model.sha256)
            }
            _ => false,
        };
    let upsert: Vec<HotspotUpsert> = knowledge
        .hotspots
        .iter()
        .filter(|hotspot| hotspot.status == HotspotStatus::Candidate && matches_model(hotspot))
        .map(|hotspot| HotspotUpsert {
            id: Some(hotspot.id.clone()),
            part_id: hotspot.part_id.clone(),
            status: HotspotStatus::Confirmed,
            anchor: hotspot.anchor.clone(),
        })
        .collect();
    let has_usable_hotspot = |part_id: &str| {
        knowledge.hotspots.iter().any(|hotspot| {
            hotspot.part_id == part_id
                && matches!(
                    hotspot.status,
                    HotspotStatus::Candidate | HotspotStatus::Confirmed
                )
                && matches_model(hotspot)
        })
    };

    let mut entities: BTreeMap<String, EntityReviewPatch> = BTreeMap::new();
    let mut confirm = |id: &str, text_only: bool| {
        let existing = overlay.entity(id);
        let reviewed = existing.is_some_and(|entry| entry.is_reviewed());
        let already_text_only = existing.is_some_and(|entry| entry.text_only);
        if reviewed && (!text_only || already_text_only) {
            return;
        }
        entities.insert(
            id.to_owned(),
            EntityReviewPatch {
                review_status: (!reviewed).then_some(ReviewStatus::Confirmed),
                user_edited: None,
                text_only: (text_only && !already_text_only).then_some(true),
            },
        );
    };
    for part in &merged.parts {
        confirm(&part.id, !has_usable_hotspot(&part.id));
    }
    for step in &merged.steps {
        confirm(&step.id, false);
    }
    for spec in &merged.specs {
        confirm(&spec.id, false);
    }

    let model_review_done = match (model, overlay.model_review.as_ref()) {
        (Some(model), Some(review)) => review.is_complete_for(&model.revision_id, &model.sha256),
        _ => false,
    };
    DraftPatch {
        hotspots: (!upsert.is_empty()).then_some(HotspotPatch {
            upsert,
            remove: Vec::new(),
        }),
        entities: (!entities.is_empty()).then_some(entities),
        model_review: (model.is_some() && !model_review_done).then_some(ModelReviewPatch {
            loaded: true,
            user_confirmed: true,
        }),
        ..Default::default()
    }
}

/// 为任务的草稿执行自动发布。
pub async fn publish_job(
    pool: &SqlitePool,
    data_dir: &Path,
    job_id: &str,
) -> Result<AutoPublishOutcome, String> {
    let skip = |reason: &str| {
        Ok(AutoPublishOutcome::Skipped {
            reason: reason.to_owned(),
        })
    };
    let (job, draft, admin_id) = {
        let mut conn = pool.acquire().await.map_err(|e| e.to_string())?;
        let Some(job) = repo::jobs::get(&mut conn, job_id)
            .await
            .map_err(|e| e.to_string())?
        else {
            return skip("job_missing");
        };
        let Some(draft) = repo::drafts::get_by_snapshot(&mut conn, &job.snapshot_id)
            .await
            .map_err(|e| e.to_string())?
        else {
            return skip("no_draft");
        };
        let admin_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM admins ORDER BY created_at ASC LIMIT 1")
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| e.to_string())?;
        let Some(admin_id) = admin_id else {
            return skip("no_admin");
        };
        // 发布会把草稿 revision 递增 1：草稿停在"上次发布 + 1"说明之后没有新内容，
        // 直接返回那个版本（重组装/重试改变了草稿时才再发布）。
        let published: Option<(String, i64)> = sqlx::query_as(
            "SELECT id, draft_revision FROM manual_releases WHERE draft_id = ? \
              ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(&draft.id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| e.to_string())?;
        if let Some((release_id, revision)) = published
            && revision + 1 == draft.revision
        {
            return Ok(AutoPublishOutcome::Published {
                release_id,
                replayed: true,
            });
        }
        (job, draft, admin_id)
    };

    let knowledge: DraftKnowledge =
        serde_json::from_value(draft.knowledge_json.clone()).map_err(|e| e.to_string())?;
    if knowledge.model.is_none() {
        return skip("no_model");
    }
    if knowledge.knowledge.is_none() {
        return skip("no_knowledge");
    }
    let overlay =
        crate::drafts::parse_review_overlay(&draft.review_json).map_err(|e| e.to_string())?;

    let patch = confirm_all_patch(&knowledge, &overlay);
    let draft = if patch.is_empty() {
        draft
    } else {
        crate::drafts::patch_draft(
            pool,
            &draft.item_id,
            &draft.id,
            draft.revision,
            &patch,
            AUTO_PUBLISH_ACTOR,
            Timestamp::now(),
        )
        .await
        .map_err(|e| format!("确认草稿失败：{e}"))?
    };

    let key = format!("auto-publish:{}:{}:r{}", job.id, draft.id, draft.revision);
    let body_hash = manual_core::generation::sha256_hex(
        serde_json::json!({ "draftId": draft.id, "draftRevision": draft.revision })
            .to_string()
            .as_bytes(),
    );
    match super::service::publish_draft(
        pool,
        data_dir,
        &admin_id,
        &draft.item_id,
        &draft.id,
        draft.revision,
        &key,
        &body_hash,
        Timestamp::now(),
    )
    .await
    {
        Ok(outcome) => Ok(AutoPublishOutcome::Published {
            release_id: outcome.release.id,
            replayed: outcome.replayed,
        }),
        Err(super::service::PublishError::Invariants(issues)) => Err(format!(
            "发布条件不满足：{}",
            issues
                .iter()
                .map(|issue| issue.message.clone())
                .collect::<Vec<_>>()
                .join("；")
        )),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drafts::aggregate::{Anchor, Hotspot};
    use crate::drafts::knowledge::DraftModelInfo;
    use manual_core::knowledge::{MergedKnowledge, Part, Spec, Step};

    fn knowledge() -> DraftKnowledge {
        let merged: MergedKnowledge = serde_json::from_value(serde_json::json!({
            "schemaVersion": "manual_extract_v1", "promptVersion": "p", "pageFrom": 1, "pageTo": 1,
            "coverage": { "complete": true, "pageCount": 1, "pages": [1], "batches": [] },
            "parts": [], "steps": [], "specs": [], "uncertainties": [], "conflicts": []
        }))
        .unwrap();
        let mut merged = merged;
        let part = |id: &str| -> Part {
            serde_json::from_value(
                serde_json::json!({ "id": id, "name": id, "description": "", "evidence": [],
                "reviewStatus": "needs_review", "sourceBatches": [0] }),
            )
            .unwrap()
        };
        merged.parts = vec![part("p-bound"), part("p-free")];
        merged.steps = vec![
            serde_json::from_value::<Step>(serde_json::json!({ "id": "s1", "title": "t",
            "orderedActions": [], "partIds": [], "evidence": [], "safetyNotes": [],
            "reviewStatus": "needs_review", "sourceBatches": [0] }))
            .unwrap(),
        ];
        merged.specs = vec![
            serde_json::from_value::<Spec>(serde_json::json!({ "id": "k1", "label": "l",
            "value": "v", "evidence": [], "reviewStatus": "needs_review", "sourceBatches": [0] }))
            .unwrap(),
        ];
        let model: DraftModelInfo = serde_json::from_value(serde_json::json!({
            "revisionId": "rev", "sha256": "sha", "assetId": "a", "validationState": "validated", "bounds": null
        }))
        .unwrap();
        let mut envelope = DraftKnowledge::build("job", Some(model), Some(merged), Vec::new());
        envelope.hotspots = vec![Hotspot {
            id: "h1".into(),
            part_id: "p-bound".into(),
            status: HotspotStatus::Candidate,
            anchor: Some(Anchor {
                model_revision_id: "rev".into(),
                model_sha256: "sha".into(),
                position_local: [0.0, 0.0, 0.0],
            }),
        }];
        envelope
    }

    #[test]
    fn confirms_everything_and_marks_unbound_parts_text_only() {
        let patch = confirm_all_patch(&knowledge(), &ReviewOverlay::default());
        let hotspots = patch.hotspots.unwrap();
        assert_eq!(hotspots.upsert.len(), 1);
        assert_eq!(hotspots.upsert[0].status, HotspotStatus::Confirmed);
        let entities = patch.entities.unwrap();
        assert_eq!(entities.len(), 4);
        assert_eq!(entities["p-bound"].text_only, None);
        assert_eq!(entities["p-free"].text_only, Some(true));
        assert_eq!(entities["s1"].review_status, Some(ReviewStatus::Confirmed));
        assert_eq!(
            patch.model_review,
            Some(ModelReviewPatch {
                loaded: true,
                user_confirmed: true
            })
        );
    }
}
