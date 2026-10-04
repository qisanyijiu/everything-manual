//! 自动分件 + 自动绑定的流水线阶段处理器（ADR-045）。
//!
//! - `TripoSegment`：通过已确认报价调用 `POST /mesh/segment`，持久化远端事实后
//!   由执行器轮询；成功后校验分件 GLB 并挂载到草稿。问题保留为可恢复状态。
//! - `AutoBind`：仅对语义节点名与说明书部件名明确一致的项目写入候选，等待人工复核。
//!   原始编号节点不按距离或顺序猜测身份；人工绑定、动作和姿势编辑仍受支持。
//!
//! 分件属于付费提交，必须来自已确认报价并持久化 attempt。未知提交暂停对账，
//! 已知 task ID 重启后只继续查询。自动绑定只产出名称明确匹配的候选供复核。

use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::{SecretString, Settings};
use crate::jobs::{
    MissingItem, StageContext, StageFuture, StageHandler, StageOutcome, StageRegistry,
};
use crate::providers::tripo::status::{NormalizedStatus, TripoState};
use crate::providers::tripo::{TripoClient, TripoError, TripoTimeouts};
use crate::storage::StorageError;
use manual_core::domain::StageKind;

fn jerr<E: std::fmt::Display>(e: E) -> crate::jobs::JobError {
    crate::jobs::JobError::Storage(StorageError::Database {
        detail: e.to_string(),
    })
}

// ---------------------------------------------------------------------------
// TripoSegment
// ---------------------------------------------------------------------------

pub struct TripoSegmentHandler {
    client: TripoClient,
    data_dir: PathBuf,
    settings: Arc<Settings>,
}

impl TripoSegmentHandler {
    pub fn new(base_url: &str, api_key: SecretString, settings: Arc<Settings>) -> Self {
        Self {
            client: TripoClient::new(base_url, api_key, TripoTimeouts::default())
                .expect("validated Tripo provider URL"),
            data_dir: settings.data_dir.clone(),
            settings,
        }
    }
}

fn segment_input(code: &str, message: &str) -> StageOutcome {
    StageOutcome::needs_input(vec![MissingItem::new(code, message)])
}

/// One combined reservation covers the two confirmed operations. Missing
/// provider billing is never inferred from a price: the reservation stays held.
async fn settle_segment_ledger(
    ctx: &StageContext,
    segment_actual: Option<i64>,
) -> Result<(), crate::jobs::JobError> {
    let Some(segment_actual) = segment_actual else {
        return Ok(());
    };
    let mut conn = ctx.pool.acquire().await?;
    let stages = crate::storage::repo::job_stages::list_for_job(&mut conn, &ctx.job.id).await?;
    let model_actual = stages
        .iter()
        .find(|s| s.stage_kind == StageKind::TripoPoll)
        .and_then(|s| s.usage_json.as_ref())
        .and_then(|u| u.pointer("/billing/creditMinor"))
        .and_then(serde_json::Value::as_i64);
    let Some(model_actual) = model_actual else {
        return Ok(());
    };
    let total = manual_core::cost::add_minor(model_actual, segment_actual).map_err(jerr)?;
    for entry in
        crate::storage::repo::ledger::list_for_snapshot(&mut conn, &ctx.job.snapshot_id).await?
    {
        if entry.provider == manual_core::domain::ProviderKey::Tripo {
            // A replacement purchase cannot settle away an older unknown bill.
            if entry.state == manual_core::domain::LedgerState::Unknown
                && entry.attempt_id.as_deref() != ctx.submission.attempt_id()
            {
                continue;
            }
            if let crate::storage::repo::ledger::LedgerOutcome::Rejected { reason } =
                crate::generation::ledger::settle_attempt(&mut conn, &entry.id, total, ctx.now)
                    .await?
            {
                return Err(jerr(reason));
            }
        }
    }
    Ok(())
}

impl StageHandler for TripoSegmentHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            let mut conn = ctx.pool.acquire().await?;
            let snapshot =
                crate::storage::repo::snapshots::get(&mut conn, &ctx.job.snapshot_id).await?;
            let Some(snapshot) = snapshot else {
                return Ok(segment_input(
                    "generation_snapshot_missing",
                    "任务快照缺失，请人工核对",
                ));
            };
            let config = snapshot
                .provider_config
                .get("tripoSegmentation")
                .cloned()
                .and_then(|v| {
                    serde_json::from_value::<crate::http::dto::TripoSegmentationDto>(v).ok()
                });
            let Some(config) = config else {
                return Ok(segment_input(
                    "segmentation_not_authorized",
                    "该任务的报价未包含分件费用，请重新获取报价并确认",
                ));
            };
            let known = ctx.known_remote_task_id().map(str::to_owned);
            if known.is_none()
                && ctx
                    .attempt
                    .as_ref()
                    .is_some_and(|a| a.submit_state == manual_core::domain::SubmitState::Accepted)
            {
                return Ok(StageOutcome::SubmissionUnknown {
                    reason: "分件已接受但缺少任务 ID：请对账，不重新提交".to_owned(),
                });
            }
            let seg_task_id = if let Some(id) = known {
                if let Some(attempt) = &ctx.attempt {
                    ctx.submission.bind_attempt(attempt.id.clone());
                }
                id
            } else {
                let quote_id = snapshot
                    .budgets
                    .get("quoteId")
                    .and_then(serde_json::Value::as_str);
                let quote = match quote_id {
                    Some(id) => crate::storage::repo::quotes::get(&mut conn, id).await?,
                    None => None,
                };
                let Some(quote) = quote.filter(|q| {
                    q.confirmed_at.is_some()
                        && q.consumed_job_id.as_deref() == Some(ctx.job.id.as_str())
                }) else {
                    return Ok(segment_input(
                        "segmentation_not_authorized",
                        "分件缺少已确认并用于本任务的报价，未发起请求",
                    ));
                };
                let payload = crate::generation::estimate::quote_payload(&quote)
                    .await
                    .map_err(|e| jerr(format!("{e:?}")))?;
                let entries =
                    crate::storage::repo::ledger::list_for_snapshot(&mut conn, &snapshot.id)
                        .await?;
                let entry = entries.iter().find(|e| {
                    e.provider == manual_core::domain::ProviderKey::Tripo
                        && manual_core::cost::ledger_state_holds_budget(e.state)
                });
                let authorized = snapshot
                    .budgets
                    .pointer("/authorized/tripoCreditMinor")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(-1);
                if payload.provider_config.tripo_segmentation.as_ref() != Some(&config)
                    || config.model != "v2.0-20260430"
                    || config.credit_minor <= 0
                    || config.segmentation_granularity != "detailed"
                    || !config.split_by_connectivity
                    || payload.amounts.tripo.upper_bound_minor > authorized
                    || !entry.is_some_and(|e| e.reserved >= payload.amounts.tripo.upper_bound_minor)
                    || !payload.amounts.tripo.upper_bound_lines.iter().any(|l| {
                        l.code == "meshSegmentation" && l.amount_minor == config.credit_minor
                    })
                {
                    return Ok(segment_input(
                        "segmentation_budget_missing",
                        "分件参数或预算与已确认报价不一致，未发起请求",
                    ));
                }
                let stages =
                    crate::storage::repo::job_stages::list_for_job(&mut conn, &ctx.job.id).await?;
                // Assembly also succeeds for partial inputs. A paid enhancement
                // must not start when its original model branch failed or its
                // validated model is absent from the assembled draft.
                let model_stage_succeeded = stages.iter().any(|s| {
                    s.stage_kind == StageKind::ModelValidate
                        && s.status == manual_core::domain::JobStatus::Succeeded
                });
                let draft =
                    crate::storage::repo::drafts::get_by_snapshot(&mut conn, &ctx.job.snapshot_id)
                        .await?;
                let model =
                    if let Some(id) = draft.as_ref().and_then(|d| d.model_revision_id.as_deref()) {
                        crate::storage::repo::model_revisions::get(&mut conn, id).await?
                    } else {
                        None
                    };
                if !model_stage_succeeded
                    || !model.is_some_and(|m| {
                        m.item_id == ctx.job.item_id
                            && m.validation_state
                                == manual_core::domain::ModelValidationState::Validated
                    })
                {
                    return Ok(segment_input(
                        "segmentation_model_not_validated",
                        "原模型尚未校验通过并组装到草稿，未提交付费分件；请先处理模型分支",
                    ));
                }
                if stages
                    .iter()
                    .find(|s| s.stage_kind == StageKind::TripoPoll)
                    .and_then(|s| s.usage_json.as_ref())
                    .and_then(|u| u.pointer("/billing/creditMinor"))
                    .and_then(serde_json::Value::as_i64)
                    .is_some_and(|actual| {
                        actual
                            .checked_add(config.credit_minor)
                            .is_none_or(|total| total > authorized)
                    })
                {
                    return Ok(segment_input(
                        "segmentation_budget_exceeded",
                        "模型实际计费加分件估算超出授权预算，未提交分件",
                    ));
                }
                let Some(input) = crate::storage::repo::attempts::latest_accepted_for_job(
                    &mut conn,
                    &ctx.job.id,
                    StageKind::TripoSubmit,
                )
                .await?
                .and_then(|a| a.remote_task_id) else {
                    return Ok(segment_input(
                        "tripo_task_id_missing",
                        "找不到原模型的任务 ID，请人工核对",
                    ));
                };
                drop(conn);
                let body = serde_json::to_vec(&json!({"model": config.model, "input": input,
                    "segmentation_granularity": config.segmentation_granularity,
                    "split_by_connectivity": config.split_by_connectivity}))
                .map_err(jerr)?;
                let hash = manual_core::generation::sha256_hex(&body);
                ctx.submission.begin_intent(&hash).await?;
                ctx.submission.mark_submitting().await?;
                match self.client.submit_segment(&body).await {
                    Ok(data) => {
                        if matches!(
                            ctx.submission.record_remote_task_id(&data.task_id).await?,
                            crate::jobs::RemoteTaskObservation::Conflict { .. }
                        ) {
                            return Ok(StageOutcome::SubmissionUnknown {
                                reason: "分件任务 ID 冲突，等待对账".to_owned(),
                            });
                        }
                        // Query on the next executor run, after the receipt has been committed.
                        return Ok(StageOutcome::WaitingProvider);
                    }
                    Err(error) => {
                        let reason = error.redacted();
                        if matches!(error, TripoError::RateLimited { .. }) {
                            ctx.submission.mark_failed(&reason).await?;
                            return Ok(StageOutcome::Retryable {
                                reason,
                                retry_after_seconds: error.retry_after_seconds(),
                            });
                        }
                        if error.is_definitively_refused() {
                            ctx.submission.mark_failed(&reason).await?;
                            settle_segment_ledger(ctx, Some(0)).await?;
                            return Ok(StageOutcome::Failed { reason });
                        }
                        ctx.submission.mark_unknown(&reason).await?;
                        let mut conn = ctx.pool.acquire().await?;
                        for entry in
                            crate::storage::repo::ledger::list_for_snapshot(&mut conn, &snapshot.id)
                                .await?
                        {
                            if entry.provider == manual_core::domain::ProviderKey::Tripo
                                && manual_core::cost::ledger_state_holds_budget(entry.state)
                            {
                                crate::generation::ledger::mark_submission_unknown(
                                    &mut conn,
                                    &entry.id,
                                    ctx.submission.attempt_id(),
                                    ctx.now,
                                )
                                .await?;
                            }
                        }
                        return Ok(StageOutcome::SubmissionUnknown { reason });
                    }
                }
            };
            drop(conn);
            ctx.submission.check_call().await?;
            let task = match self.client.get_task(&seg_task_id).await {
                Ok(task) => task,
                Err(e) => {
                    return Ok(StageOutcome::Retryable {
                        reason: e.redacted(),
                        retry_after_seconds: e.retry_after_seconds(),
                    });
                }
            };
            let state = NormalizedStatus::new(&task.status_raw).state();
            let mut usage = json!({"remoteTaskId": seg_task_id, "rawStatus": task.status_raw, "normalizedStatus": state.as_str()});
            if let Some(b) = &task.billing {
                usage["billing"] =
                    json!({"creditMinor": b.credit_minor, "sourceField": b.source_field});
            }
            crate::jobs::submission::record_result_fact(
                &ctx.pool,
                &ctx.stage.id,
                None,
                Some(&usage.to_string()),
                ctx.now,
            )
            .await?;
            match state {
                TripoState::Queued | TripoState::Running | TripoState::Unrecognized => {
                    return Ok(StageOutcome::WaitingProvider);
                }
                TripoState::Failed
                | TripoState::Cancelled
                | TripoState::Banned
                | TripoState::Expired => {
                    settle_segment_ledger(ctx, task.billing.as_ref().map(|b| b.credit_minor))
                        .await?;
                    return Ok(StageOutcome::Failed {
                        reason: format!("分件任务 {}，已保留 task ID 和计费事实", task.status_raw),
                    });
                }
                TripoState::Success => {}
            }
            settle_segment_ledger(ctx, task.billing.as_ref().map(|b| b.credit_minor)).await?;
            let Some(model_url) = task.model_url else {
                return Ok(StageOutcome::Retryable {
                    reason: "分件 success 但没有模型 URL，保留任务继续查询".to_owned(),
                    retry_after_seconds: None,
                });
            };
            let draft = {
                let mut conn = ctx.pool.acquire().await?;
                crate::storage::repo::drafts::get_by_snapshot(&mut conn, &ctx.job.snapshot_id)
                    .await?
            };
            let Some(draft) = draft else {
                return Ok(segment_input(
                    "draft_missing",
                    "草稿缺失，保留远端分件任务，请人工核对",
                ));
            };
            let source = format!("tripo:mesh_segment {} task={seg_task_id}", config.model);
            if draft
                .knowledge_json
                .pointer("/interactive/partsModel/source")
                .and_then(serde_json::Value::as_str)
                == Some(source.as_str())
            {
                // Attachment committed before an interrupted checkpoint: use the
                // same immutable asset, without creating a new draft revision.
                let asset = draft
                    .knowledge_json
                    .pointer("/interactive/partsModel/assetId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                return Ok(StageOutcome::Succeeded {
                    result_asset_id: asset,
                    usage: Some(usage),
                });
            }
            // 下载分件 GLB（独立无凭据客户端；签名地址只保留在本次执行内）
            let downloader = crate::assets::glb::ModelDownloader::from_settings(&self.settings);
            let downloaded = match downloader.download(&model_url).await {
                Ok(d) => d,
                Err(reason) => {
                    return Ok(StageOutcome::Retryable {
                        reason: format!(
                            "分件模型下载失败：{}",
                            crate::redaction::redact_text_urls(&format!("{reason}"))
                        ),
                        retry_after_seconds: None,
                    });
                }
            };
            match crate::drafts::parts_model::attach_parts_model(
                &ctx.pool,
                &self.data_dir,
                crate::drafts::parts_model::AttachInput {
                    item_id: &draft.item_id,
                    draft_id: &draft.id,
                    expected_revision: draft.revision,
                    staged: crate::assets::blob_store::Staged {
                        path: downloaded.path,
                        sha256: downloaded.sha256,
                        size: downloaded.size,
                    },
                    source,
                    actor: "system:tripo_segment",
                },
                ctx.now,
            )
            .await
            {
                Ok(draft) => {
                    tracing::info!(jobId = %ctx.job.id, draftId = %draft.id, nodes = ?draft.revision, "分件模型已自动挂载");
                    let asset_id = draft
                        .knowledge_json
                        .pointer("/interactive/partsModel/assetId")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    usage["draftId"] = json!(draft.id);
                    usage["draftRevision"] = json!(draft.revision);
                    Ok(StageOutcome::Succeeded {
                        result_asset_id: asset_id,
                        usage: Some(usage),
                    })
                }
                Err(reason) => Ok(StageOutcome::NeedsInput {
                    items: vec![MissingItem::new(
                        "parts_model_attach_failed",
                        format!("分件模型挂载失败：{reason}；保留任务，重试只重新查询和挂载"),
                    )],
                }),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// AutoBind — 在 TripoSegment 之后运行
// ---------------------------------------------------------------------------

pub struct AutoBindHandler {
    data_dir: PathBuf,
}

impl AutoBindHandler {
    pub fn new(settings: &Settings) -> Self {
        Self {
            data_dir: settings.data_dir.clone(),
        }
    }
}

impl StageHandler for AutoBindHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            let draft_row = {
                let mut conn = ctx.pool.acquire().await.map_err(jerr)?;
                crate::storage::repo::drafts::get_by_snapshot(&mut conn, &ctx.job.snapshot_id)
                    .await
                    .map_err(jerr)?
            };
            let Some(draft) = draft_row else {
                return Ok(StageOutcome::Succeeded {
                    result_asset_id: None,
                    usage: Some(json!({"skipped": true, "reason": "no_draft"})),
                });
            };
            let knowledge: crate::drafts::knowledge::DraftKnowledge =
                match serde_json::from_value(draft.knowledge_json.clone()) {
                    Ok(k) => k,
                    Err(_) => {
                        return Ok(StageOutcome::Succeeded {
                            result_asset_id: None,
                            usage: Some(json!({"skipped": true, "reason": "knowledge_unreadable"})),
                        });
                    }
                };
            let interactive = knowledge.interactive.as_ref();
            if interactive.is_none() {
                tracing::info!(jobId = %ctx.job.id, "auto_bind：草稿没有分件模型（tripo_segment 可能失败），跳过绑定");
                return Ok(StageOutcome::Succeeded {
                    result_asset_id: None,
                    usage: Some(json!({"skipped": true, "reason": "no_parts_model"})),
                });
            }
            let parts = knowledge
                .knowledge
                .as_ref()
                .map(|k| &k.parts)
                .cloned()
                .unwrap_or_default();
            let node_names = &interactive.unwrap().parts_model.node_names;
            let model = knowledge.model.as_ref();
            let Some(model) = model else {
                return Ok(StageOutcome::Succeeded {
                    result_asset_id: None,
                    usage: Some(json!({"skipped": true, "reason": "no_model"})),
                });
            };
            // 从分件 GLB 读取节点包围盒
            let parts_asset = &interactive.unwrap().parts_model.asset_id;
            let blob = {
                let mut conn = ctx.pool.acquire().await.map_err(jerr)?;
                crate::storage::repo::assets::get_with_blob(&mut conn, parts_asset)
                    .await
                    .map_err(jerr)?
            };
            let blob_sha = blob
                .as_ref()
                .map(|(_, b)| b.sha256.clone())
                .unwrap_or_default();
            let glb_path = crate::assets::blob_store::blob_path(&self.data_dir, &blob_sha);
            // 读取 GLB 节点包围盒（用于交互推导）
            let node_bounds = read_node_bounds(&glb_path);
            // Only unambiguous semantic names produce candidates. Geometry and
            // manual part order cannot establish which mesh is a lens or button.
            let bindings = heuristic_bind(&parts, node_names, &node_bounds)
                .into_iter()
                .filter(|(id, _, _)| {
                    !knowledge.hotspots.iter().any(|h| &h.part_id == id)
                        && !interactive
                            .unwrap()
                            .bindings
                            .iter()
                            .any(|b| &b.part_id == id)
                })
                .collect::<Vec<_>>();
            if bindings.is_empty() {
                return Ok(StageOutcome::Succeeded {
                    result_asset_id: None,
                    usage: Some(
                        json!({"skipped": true, "reason": "no_unambiguous_semantic_match", "draftId": draft.id}),
                    ),
                });
            }
            let hotspots: Vec<serde_json::Value> = bindings
                .iter()
                .map(|(part_id, _nodes, center)| {
                    json!({
                        "partId": part_id,
                        "status": "candidate",
                        "anchor": {
                            "modelRevisionId": model.revision_id,
                            "modelSha256": model.sha256,
                            "positionLocal": center,
                        }
                    })
                })
                .collect();
            let mut binding_defs: Vec<serde_json::Value> = interactive
                .unwrap()
                .bindings
                .iter()
                .map(|b| serde_json::to_value(b).expect("binding serializes"))
                .collect();
            binding_defs.extend(bindings.iter().map(|(part_id, nodes, _)| {
                json!({ "partId": part_id, "nodes": nodes, "status": "auto" })
            }));
            // 推导动作和姿势
            let steps_list = knowledge
                .knowledge
                .as_ref()
                .map(|k| k.steps.clone())
                .unwrap_or_default();
            let actions = derive_actions(&parts, &bindings, &node_bounds, &steps_list);
            let poses = derive_poses(&node_bounds, &parts);
            let hotspot_count = hotspots.len();
            let action_count = actions.len();
            let pose_count = poses.len();
            tracing::info!(jobId = %ctx.job.id, hotspots = hotspot_count, actions = action_count, poses = pose_count, "auto_bind：准备 PATCH");
            // PATCH 草稿
            let patch_body = crate::drafts::aggregate::DraftPatch {
                hotspots: Some(crate::drafts::aggregate::HotspotPatch {
                    upsert: hotspots
                        .into_iter()
                        .filter_map(|v| serde_json::from_value(v).ok())
                        .collect(),
                    remove: Vec::new(),
                }),
                interactive: Some(crate::drafts::interactive::InteractivePatch {
                    bindings: Some(serde_json::from_value(json!(binding_defs)).unwrap_or_default()),
                    actions: interactive
                        .unwrap()
                        .actions
                        .is_empty()
                        .then(|| serde_json::from_value(json!(actions)).unwrap_or_default()),
                    poses: interactive
                        .unwrap()
                        .poses
                        .is_empty()
                        .then(|| serde_json::from_value(json!(poses)).unwrap_or_default()),
                }),
                ..Default::default()
            };
            match crate::drafts::service::patch_draft(
                &ctx.pool,
                &draft.item_id,
                &draft.id,
                draft.revision,
                &patch_body,
                "system:auto_bind",
                ctx.now,
            )
            .await
            {
                Ok(updated) => {
                    let h = updated
                        .knowledge_json
                        .get("hotspots")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    tracing::info!(jobId = %ctx.job.id, draftId = %updated.id, hotspots = h, "自动绑定完成");
                    Ok(StageOutcome::Succeeded {
                        result_asset_id: None,
                        usage: Some(json!({
                            "draftId": updated.id, "hotspots": h, "bindings": binding_defs.len(),
                            "actions": actions.len(), "poses": poses.len(),
                        })),
                    })
                }
                Err(reason) => {
                    tracing::warn!(jobId = %ctx.job.id, reason = %reason, "自动绑定 PATCH 失败，跳过");
                    if let crate::drafts::service::DraftServiceError::FieldIssues(ref issues) =
                        reason
                    {
                        for issue in issues {
                            tracing::warn!(field = %issue.field, message = %issue.message, "字段问题");
                        }
                    }
                    Ok(StageOutcome::Succeeded {
                        result_asset_id: None,
                        usage: Some(json!({"skipped": true, "reason": format!("{reason}")})),
                    })
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// 启发式绑定与交互推导（纯函数，无外部依赖）
// ---------------------------------------------------------------------------

fn read_node_bounds(glb_path: &Path) -> std::collections::BTreeMap<String, ([f64; 3], [f64; 3])> {
    let mut out = std::collections::BTreeMap::new();
    let bytes = match std::fs::read(glb_path) {
        Ok(b) => b,
        Err(_) => return out,
    };
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return out;
    }
    let json_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    if json_len + 20 > bytes.len() {
        return out;
    }
    let doc: serde_json::Value = match serde_json::from_slice(&bytes[20..20 + json_len]) {
        Ok(v) => v,
        Err(_) => return out,
    };
    let nodes = doc
        .get("nodes")
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();
    let meshes = doc
        .get("meshes")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    let accessors = doc
        .get("accessors")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    for node in &nodes {
        let name = node
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_owned();
        let translation: [f64; 3] = node
            .get("translation")
            .and_then(|t| t.as_array())
            .map(|a| {
                [
                    a.first().and_then(|v| v.as_f64()).unwrap_or(0.0),
                    a.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
                    a.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0),
                ]
            })
            .unwrap_or([0.0; 3]);
        let mesh_idx = node
            .get("mesh")
            .and_then(|m| m.as_u64())
            .map(|m| m as usize);
        if let Some(idx) = mesh_idx
            && let Some(mesh) = meshes.get(idx)
        {
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for prim in mesh
                .get("primitives")
                .and_then(|p| p.as_array())
                .cloned()
                .unwrap_or_default()
            {
                let pos_idx = prim
                    .pointer("/attributes/POSITION")
                    .and_then(|p| p.as_u64())
                    .map(|p| p as usize);
                if let Some(acc) = pos_idx.and_then(|i| accessors.get(i))
                    && let (Some(mn), Some(mx)) = (
                        acc.get("min").and_then(|v| v.as_array()),
                        acc.get("max").and_then(|v| v.as_array()),
                    )
                {
                    for k in 0..3 {
                        lo[k] = lo[k].min(
                            mn.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) + translation[k],
                        );
                        hi[k] = hi[k].max(
                            mx.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) + translation[k],
                        );
                    }
                }
            }
            if lo[0].is_finite() {
                out.insert(name, (lo, hi));
            }
        }
    }
    out
}

fn center(b: &([f64; 3], [f64; 3])) -> [f64; 3] {
    [
        (b.0[0] + b.1[0]) / 2.0,
        (b.0[1] + b.1[1]) / 2.0,
        (b.0[2] + b.1[2]) / 2.0,
    ]
}

/// Conservative semantic-name matching; opaque/numbered nodes have no inferred identity.
fn heuristic_bind(
    parts: &[manual_core::knowledge::Part],
    node_names: &[String],
    bounds: &std::collections::BTreeMap<String, ([f64; 3], [f64; 3])>,
) -> Vec<(String, Vec<String>, [f64; 3])> {
    let mut result = Vec::new();
    let mut used_nodes = std::collections::HashSet::new();
    let normalized = |name: &str| -> String {
        fold_traditional(name)
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    for part in parts {
        let name = normalized(&part.name);
        if name.is_empty() || name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let matching: Vec<_> = node_names
            .iter()
            .filter(|n| {
                normalized(n) == name
                    && bounds.contains_key(n.as_str())
                    && !used_nodes.contains(n.as_str())
            })
            .collect();
        if matching.len() == 1 {
            let node = matching[0];
            used_nodes.insert(node.clone());
            result.push((part.id.clone(), vec![node.clone()], center(&bounds[node])));
        }
    }
    result
}

/// 从分件节点包围盒推导动作（相机/设备类产品）。
/// 按部件名称中的关键词匹配典型操作（盖子→外翻取下，按钮→按下，转盘→转动，杆→扳动）。
fn derive_actions(
    parts: &[manual_core::knowledge::Part],
    bindings: &[(String, Vec<String>, [f64; 3])],
    bounds: &std::collections::BTreeMap<String, ([f64; 3], [f64; 3])>,
    steps: &[manual_core::knowledge::Step],
) -> Vec<serde_json::Value> {
    use serde_json::json;
    let all_bounds: Vec<[f64; 3]> = bounds.values().flat_map(|b| vec![b.0, b.1]).collect();
    if all_bounds.is_empty() {
        return Vec::new();
    }
    let lo = [
        all_bounds
            .iter()
            .map(|b| b[0])
            .fold(f64::INFINITY, f64::min),
        all_bounds
            .iter()
            .map(|b| b[1])
            .fold(f64::INFINITY, f64::min),
        all_bounds
            .iter()
            .map(|b| b[2])
            .fold(f64::INFINITY, f64::min),
    ];
    let hi = [
        all_bounds
            .iter()
            .map(|b| b[0])
            .fold(f64::NEG_INFINITY, f64::max),
        all_bounds
            .iter()
            .map(|b| b[1])
            .fold(f64::NEG_INFINITY, f64::max),
        all_bounds
            .iter()
            .map(|b| b[2])
            .fold(f64::NEG_INFINITY, f64::max),
    ];
    let volume = (hi[0] - lo[0]) * (hi[1] - lo[1]) * (hi[2] - lo[2]);
    let mut actions = Vec::new();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let part_name: std::collections::HashMap<&str, &str> = parts
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();
    let steps_for = |keywords: &[&str]| -> Vec<String> {
        steps
            .iter()
            .filter(|s| keywords.iter().any(|k| s.title.contains(k)))
            .take(6)
            .map(|s| s.id.clone())
            .collect()
    };
    let mut labels: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (pid, nodes, c) in bindings {
        let raw_name = part_name.get(pid.as_str()).copied().unwrap_or("");
        // 同一部件名在说明书里常出现多次（不同页的同名部件）：同名只生成一个动作。
        // 去重键忽略空白（「跟焦 / 变焦切换键」与「跟焦/变焦切换键」是同一个部件）。
        let dedupe_key: String = raw_name.chars().filter(|c| !c.is_whitespace()).collect();
        if !labels.insert(dedupe_key) {
            continue;
        }
        // 规则按简体关键字匹配；繁体说明书先做常用字折叠。
        let folded = fold_traditional(raw_name);
        let name = folded.as_str();
        let ns: Vec<String> = nodes
            .iter()
            .filter(|n| bounds.contains_key(n.as_str()) && !used.contains(n.as_str()))
            .cloned()
            .collect();
        if ns.is_empty() {
            continue;
        }
        let b = (
            ns.iter()
                .map(|n| bounds[n].0)
                .fold([f64::INFINITY; 3], |a, b| {
                    [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])]
                }),
            ns.iter()
                .map(|n| bounds[n].1)
                .fold([f64::NEG_INFINITY; 3], |a, b| {
                    [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]
                }),
        );
        let node_vol = (b.1[0] - b.0[0]) * (b.1[1] - b.0[1]) * (b.1[2] - b.0[2]);
        if node_vol > 0.25 * volume {
            continue;
        } // skip body-sized nodes
        let r = |n: &[String], pivot: [f64; 3], axis: [f64; 3], deg: f64| json!({"nodes": n, "kind": "rotate", "pivot": pivot, "axis": axis, "angleDeg": deg});
        let t = |n: &[String], v: [f64; 3]| json!({"nodes": n, "kind": "translate", "vector": v});
        // 「手柄」只有在是电池盖一类可拆件时才做拆卸动作（云台/咖啡机的手柄是主体或另有语义）。
        let is_cover_grip = name.contains("手柄") && (name.contains("电池") || name.contains("盖"));
        if name.contains("电池盖") || is_cover_grip {
            let side = if c[0] < 0.0 { -1.0 } else { 1.0 };
            let hinge = [if side < 0.0 { b.1[0] } else { b.0[0] }, b.0[1], b.1[2]];
            actions.push(json!({"id": format!("open-{}", actions.len()), "label": format!("取下{name}"), "description": "露出电池仓（外观示意）。",
                "triggerPartIds": [pid], "mode": "toggle", "durationMs": 900,
                "steps": [t(&ns, [side * 0.18, -0.05, 0.08]), r(&ns, hinge, [0.0, 1.0, 0.0], side * 25.0)],
                "stepIds": steps_for(&["电池"])}));
        } else if name.contains("后盖") {
            let hinge = [b.1[0], c[1], b.0[2]];
            actions.push(json!({"id": format!("open-{}", actions.len()), "label": format!("打开{name}"), "description": "装入胶片时打开（外观示意）。",
                "triggerPartIds": [pid], "mode": "toggle", "durationMs": 1000,
                "steps": [r(&ns, hinge, [0.0, 1.0, 0.0], -70.0)], "stepIds": steps_for(&["胶片"])}));
        } else if !name.contains("膝盖")
            && (name.contains("翻盖")
                || name.contains("掀盖")
                || name.ends_with("盖")
                || name.ends_with("盖板"))
        {
            // 通用翻盖：沿顶部后缘为铰链向上掀开。
            let up = if (b.1[1] - b.0[1]) <= (b.1[2] - b.0[2]) {
                1
            } else {
                2
            };
            let mut hinge = *c;
            hinge[up] = b.1[up];
            hinge[0] = b.0[0];
            actions.push(json!({"id": format!("open-{}", actions.len()), "label": format!("打开{raw_name}"), "description": "掀开盖子（外观示意）。",
                "triggerPartIds": [pid], "mode": "toggle", "durationMs": 900,
                "steps": [r(&ns, hinge, [0.0, 0.0, 1.0], 60.0)], "stepIds": steps_for(&[&raw_name[..raw_name.chars().take(2).map(|c| c.len_utf8()).sum::<usize>()]])}));
        } else if ["尘盒", "水箱", "滤网", "抽屉", "滴水盘"]
            .iter()
            .any(|k| name.ends_with(k))
        {
            // 只匹配以模块名结尾的部件（「集尘盒卡扣」「集尘盒盖」是子部件，不整体抽出）。
            // 可抽出的模块：沿离物体中心最近的外侧方向平移拉出。
            let mid = [
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            ];
            let d = [c[0] - mid[0], c[1] - mid[1], c[2] - mid[2]];
            let axis = if d[0].abs() >= d[2].abs() { 0 } else { 2 };
            let mut v = [0.0; 3];
            v[axis] = if d[axis] >= 0.0 { 0.15 } else { -0.15 };
            actions.push(json!({"id": format!("pull-{}", actions.len()), "label": format!("取出{raw_name}"), "description": "抽出模块（外观示意）。",
                "triggerPartIds": [pid], "mode": "toggle", "durationMs": 800,
                "steps": [t(&ns, v)], "stepIds": steps_for(&[&raw_name[..raw_name.chars().take(2).map(|c| c.len_utf8()).sum::<usize>()]])}));
        } else if name.contains("滤碗手柄") || name.contains("冲泡头手柄") {
            // 咖啡机滤碗手柄：先逆时针旋出，再向下取出。
            let mut down = [0.0; 3];
            down[1] = -0.1;
            actions.push(json!({"id": format!("pull-{}", actions.len()), "label": format!("取下{raw_name}"), "description": "旋出并取下（外观示意）。",
                "triggerPartIds": [pid], "mode": "toggle", "durationMs": 1000,
                "steps": [r(&ns, *c, [0.0, 1.0, 0.0], -40.0), t(&ns, down)], "stepIds": steps_for(&["滤碗", "手柄"])}));
        } else if name.contains("扳机") {
            let pivot = [c[0], b.1[1], c[2]];
            actions.push(json!({"id": format!("press-{}", actions.len()), "label": format!("扣动{raw_name}"), "description": null,
                "triggerPartIds": [pid], "mode": "pulse", "durationMs": 400,
                "steps": [r(&ns, pivot, [0.0, 0.0, 1.0], 15.0)], "stepIds": steps_for(&["扳机"])}));
        } else if name.contains("摇杆") {
            let pivot = [c[0], b.0[1], c[2]];
            actions.push(json!({"id": format!("lever-{}", actions.len()), "label": format!("推动{raw_name}"), "description": null,
                "triggerPartIds": [pid], "mode": "pulse", "durationMs": 600,
                "steps": [r(&ns, pivot, [0.0, 0.0, 1.0], 20.0)], "stepIds": steps_for(&["摇杆"])}));
        } else if name.contains("按钮")
            || name.contains("快门")
            || name.ends_with("键")
            || name.contains("开关")
        {
            actions.push(json!({"id": format!("press-{}", actions.len()), "label": format!("按下{raw_name}"), "description": null,
                "triggerPartIds": [pid], "mode": "pulse", "durationMs": 300,
                "steps": [t(&ns, [0.0, -0.012, 0.0])], "stepIds": steps_for(&[&name[..name.chars().take(2).map(|c| c.len_utf8()).sum::<usize>()]])}));
        } else if name.contains("转盘")
            || name.contains("拨盘")
            || name.contains("旋钮")
            || name.contains("拨轮")
        {
            actions.push(json!({"id": format!("turn-{}", actions.len()), "label": format!("转动{raw_name}"), "description": null,
                "triggerPartIds": [pid], "mode": "pulse", "durationMs": 900,
                "steps": [r(&ns, *c, [0.0, 1.0, 0.0], 60.0)], "stepIds": steps_for(&[&name[..name.chars().take(2).map(|c| c.len_utf8()).sum::<usize>()]])}));
        } else if name.contains("杆") && !name.contains("摇杆") {
            let pivot = [b.0[0] + 0.03, c[1], c[2]];
            actions.push(json!({"id": format!("lever-{}", actions.len()), "label": format!("扳动{raw_name}"), "description": null,
                "triggerPartIds": [pid], "mode": "pulse", "durationMs": 700,
                "steps": [r(&ns, pivot, [0.0, 1.0, 0.0], -35.0)], "stepIds": steps_for(&[&name[..name.chars().take(2).map(|c| c.len_utf8()).sum::<usize>()]])}));
        } else {
            continue;
        }
        for n in &ns {
            used.insert(n.clone());
        }
    }
    actions.into_iter().take(12).collect()
}

/// 动作规则用到的繁体字 → 简体（只覆盖规则关键字，不做通用转换）。
fn fold_traditional(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            '鍵' => '键',
            '蓋' => '盖',
            '塵' => '尘',
            '濾' => '滤',
            '網' => '网',
            '開' => '开',
            '關' => '关',
            '鈕' => '钮',
            '轉' => '转',
            '撥' => '拨',
            '輪' => '轮',
            '電' => '电',
            '門' => '门',
            '桿' => '杆',
            '後' => '后',
            '機' => '机',
            '盤' => '盘',
            '屜' => '屉',
            other => other,
        })
        .collect()
}

/// 说明书文本是否表明这是四足/机器狗一类可摆姿势的产品。
fn looks_like_quadruped(parts: &[manual_core::knowledge::Part]) -> bool {
    const KEYWORDS: &[&str] = &[
        "机器狗",
        "四足",
        "仿生四足",
        "cyberdog",
        "quadruped",
        "spot",
    ];
    parts.iter().any(|part| {
        let name = part.name.to_ascii_lowercase();
        KEYWORDS
            .iter()
            .any(|kw| name.contains(kw) || part.name.contains(kw))
    })
}

/// 垂直轴：Y 或 Z 中跨度更大、且底部有若干细长件的那根。Tripo 分件朝向不稳定。
fn vertical_axis(span: [f64; 3]) -> usize {
    if span[2] > span[1] * 1.15 { 2 } else { 1 }
}

/// 四个身体象限里各取最高的贴地细长节点作为小腿（固定贴地阈值会漏掉离地几毫米的腿）。
fn find_legs(
    bounds: &std::collections::BTreeMap<String, ([f64; 3], [f64; 3])>,
    lo: [f64; 3],
    hi: [f64; 3],
    span: [f64; 3],
    up: usize,
    body_axis: usize,
    side_axis: usize,
) -> Vec<&str> {
    let center = |b: &([f64; 3], [f64; 3])| {
        [
            (b.0[0] + b.1[0]) / 2.0,
            (b.0[1] + b.1[1]) / 2.0,
            (b.0[2] + b.1[2]) / 2.0,
        ]
    };
    let mid_body = (lo[body_axis] + hi[body_axis]) / 2.0;
    let mid_side = (lo[side_axis] + hi[side_axis]) / 2.0;
    let mut best: std::collections::BTreeMap<(bool, bool), (&str, f64)> =
        std::collections::BTreeMap::new();
    for (n, b) in bounds {
        let height = b.1[up] - b.0[up];
        if b.0[up] > lo[up] + 0.3 * span[up]
            || height < 0.15 * span[up]
            || (b.1[body_axis] - b.0[body_axis]) > 0.4 * span[body_axis]
        {
            continue;
        }
        let c = center(b);
        let key = (c[body_axis] > mid_body, c[side_axis] < mid_side);
        if best.get(&key).is_none_or(|(_, h)| height > *h) {
            best.insert(key, (n.as_str(), height));
        }
    }
    best.values().map(|(n, _)| *n).collect()
}

/// 从分件节点包围盒推导四足机器人姿势。
/// 找身体轴和头部方向，识别四条腿（大腿/小腿/足垫），生成站立/趴下/坐下/握手/作揖。
fn derive_poses(
    bounds: &std::collections::BTreeMap<String, ([f64; 3], [f64; 3])>,
    parts: &[manual_core::knowledge::Part],
) -> Vec<serde_json::Value> {
    use serde_json::json;
    if bounds.len() < 8 {
        return Vec::new();
    } // too few parts for a robot
    if !looks_like_quadruped(parts) {
        return Vec::new();
    }
    let all: Vec<[f64; 3]> = bounds.values().flat_map(|b| vec![b.0, b.1]).collect();
    let lo = [
        all.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min),
        all.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min),
        all.iter().map(|b| b[2]).fold(f64::INFINITY, f64::min),
    ];
    let hi = [
        all.iter().map(|b| b[0]).fold(f64::NEG_INFINITY, f64::max),
        all.iter().map(|b| b[1]).fold(f64::NEG_INFINITY, f64::max),
        all.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max),
    ];
    let span = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    // 垂直轴不可靠地由包围盒比例决定（站立的机器狗在 Y、Z 方向尺寸接近）：
    // 优先试比例推断的轴，再试另一个候选轴，取能在四个象限各找到一条腿的那个。
    let preferred = vertical_axis(span);
    let mut found = None;
    for up in [preferred, if preferred == 1 { 2 } else { 1 }] {
        let body_axis = [0, 1, 2]
            .into_iter()
            .filter(|&a| a != up)
            .max_by(|&a, &b| {
                span[a]
                    .partial_cmp(&span[b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(0);
        let side_axis = [0, 1, 2]
            .into_iter()
            .find(|&a| a != up && a != body_axis)
            .unwrap_or(2);
        let shins = find_legs(bounds, lo, hi, span, up, body_axis, side_axis);
        if shins.len() == 4 {
            found = Some((up, body_axis, side_axis, shins));
            break;
        }
    }
    let Some((up, body_axis, side_axis, shins)) = found else {
        return Vec::new();
    };
    // Head direction: tallest parts on one end
    let tall: Vec<(&str, f64)> = bounds
        .iter()
        .filter(|(_, b)| b.1[up] > lo[up] + 0.7 * span[up])
        .map(|(n, b)| (n.as_str(), (b.0[body_axis] + b.1[body_axis]) / 2.0))
        .collect();
    let head_mean = if tall.is_empty() {
        (lo[body_axis] + hi[body_axis]) / 2.0
    } else {
        tall.iter().map(|(_, c)| c).sum::<f64>() / tall.len() as f64
    };
    let head_sign = if head_mean >= (lo[body_axis] + hi[body_axis]) / 2.0 {
        1.0
    } else {
        -1.0
    };
    let mut swing = [0.0; 3]; // rotation axis perpendicular to body and up
    swing[side_axis] = 1.0;
    // Classify legs into corners
    let mid_body = (lo[body_axis] + hi[body_axis]) / 2.0;
    let mid_side = (lo[side_axis] + hi[side_axis]) / 2.0;
    struct Leg {
        thigh: Option<String>,
        shin: String,
        feet: Vec<String>,
    }
    let mut legs: std::collections::BTreeMap<String, Leg> = std::collections::BTreeMap::new();
    for sh in &shins {
        let c = center(bounds.get(*sh).unwrap());
        let front = (c[body_axis] - mid_body) * head_sign > 0.0;
        let left = c[side_axis] < mid_side;
        let corner = format!(
            "{}{}",
            if front { "F" } else { "R" },
            if left { "L" } else { "R" }
        );
        let thighs: Vec<&str> = bounds
            .iter()
            .filter(|(n, b)| {
                !shins.contains(&n.as_str())
                    && (center(b)[body_axis] - c[body_axis]).abs()
                        < 0.12 * span[body_axis].max(0.08)
                    && (center(b)[side_axis] - c[side_axis]).abs()
                        < 0.12 * span[side_axis].max(0.08)
                    && center(b)[up] > c[up]
                    && (b.1[up] - b.0[up]) < 0.45 * span[up]
            })
            .map(|(n, _)| n.as_str())
            .collect();
        let thigh = thighs
            .into_iter()
            .max_by(|a, b| {
                let va = bounds[*a];
                let vb = bounds[*b];
                let sa = (va.1[0] - va.0[0]) * (va.1[1] - va.0[1]) * (va.1[2] - va.0[2]);
                let sb = (vb.1[0] - vb.0[0]) * (vb.1[1] - vb.0[1]) * (vb.1[2] - vb.0[2]);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(str::to_owned);
        legs.entry(corner).or_insert(Leg {
            thigh,
            shin: sh.to_string(),
            feet: Vec::new(),
        });
    }
    // Attach feet (small floor-level nodes) to nearest shin
    for (n, b) in bounds {
        if shins.contains(&n.as_str())
            || (b.1[up] - b.0[up]) > 0.18 * span[up]
            || b.0[up] > lo[up] + 0.1 * span[up]
        {
            continue;
        }
        let c = center(b);
        if let Some((_, leg)) = legs.iter_mut().min_by(|(_, la), (_, lb)| {
            let ca = center(bounds.get(la.shin.as_str()).unwrap());
            let cb = center(bounds.get(lb.shin.as_str()).unwrap());
            let da =
                (ca[body_axis] - c[body_axis]).powi(2) + (ca[side_axis] - c[side_axis]).powi(2);
            let db =
                (cb[body_axis] - c[body_axis]).powi(2) + (cb[side_axis] - c[side_axis]).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            leg.feet.push(n.clone());
        }
    }
    if legs.len() != 4 {
        return Vec::new();
    } // exactly 4 legs = quadruped; cameras/devices return empty
    let all_nodes: Vec<String> = bounds.keys().cloned().collect();
    let hip = |n: &str| -> [f64; 3] {
        let b = bounds.get(n).unwrap();
        let mut p = center(b);
        p[up] = b.1[up] - 0.04;
        p
    };
    let knee = |n: &str| -> [f64; 3] {
        let b = bounds.get(n).unwrap();
        let mut p = center(b);
        p[up] = b.1[up] - 0.015;
        p
    };
    let r = |nodes: &[String], pivot: [f64; 3], axis: &[f64; 3], deg: f64| json!({"nodes": nodes, "kind": "rotate", "pivot": pivot, "axis": axis, "angleDeg": deg});
    let leg_steps = |corner: &str, h: f64, k: f64| -> Vec<serde_json::Value> {
        let Some(leg) = legs.get(corner) else {
            return Vec::new();
        };
        let mut lower: Vec<String> = vec![leg.shin.clone()];
        lower.extend(leg.feet.iter().cloned());
        match &leg.thigh {
            None => vec![r(&lower, hip(&leg.shin), &swing, h)],
            Some(th) => {
                let mut upper = vec![th.clone()];
                upper.extend(lower.iter().cloned());
                vec![
                    r(&lower, knee(&leg.shin), &swing, k),
                    r(&upper, hip(th), &swing, h),
                ]
            }
        }
    };
    let pose = |id: &str,
                label: &str,
                desc: &str,
                ms: u32,
                per: &[(&str, f64, f64)],
                pitch: f64,
                pivot: [f64; 3]|
     -> serde_json::Value {
        let mut steps = Vec::new();
        for (corner, h, k) in per {
            steps.extend(leg_steps(corner, *h, *k));
        }
        // 每步最多引用 64 个节点（validate_interactive_patch）：整体俯仰按同一枢轴分块。
        if pitch.abs() > 0.001 {
            for chunk in all_nodes.chunks(64) {
                steps.push(r(chunk, pivot, &swing, pitch));
            }
        }
        json!({"id": id, "label": label, "description": desc, "durationMs": ms, "steps": steps})
    };
    vec![
        json!({"id": "stand", "label": "站立", "description": "默认站立姿态（模型原始姿态）。", "durationMs": 500, "steps": [r(&all_nodes[..1], [0.0; 3], &swing, 0.0)]}),
        pose(
            "lie-down",
            "趴下",
            "腿部收起、腹部贴地。",
            1200,
            &[
                ("FL", -50.0, 115.0),
                ("FR", -50.0, 115.0),
                ("RL", -50.0, 115.0),
                ("RR", -50.0, 115.0),
            ],
            0.0,
            [0.0; 3],
        ),
        pose(
            "sit",
            "坐下",
            "后腿收起、前腿支撑。",
            1100,
            &[
                ("RL", -55.0, 120.0),
                ("RR", -55.0, 120.0),
                ("FL", 10.0, 0.0),
                ("FR", 10.0, 0.0),
            ],
            24.0,
            {
                let mut p = [0.0; 3];
                p[body_axis] = lo[body_axis] + 0.15;
                p[up] = lo[up] + 0.3;
                p
            },
        ),
        pose(
            "shake-hand",
            "握手",
            "抬起右前腿。",
            900,
            &[("FR", 65.0, -20.0)],
            0.0,
            [0.0; 3],
        ),
        pose(
            "bow",
            "作揖",
            "前腿弯曲下压。",
            1100,
            &[("FL", -45.0, 110.0), ("FR", -45.0, 110.0)],
            -16.0,
            {
                let mut p = [0.0; 3];
                p[body_axis] = hi[body_axis] - 0.2;
                p[up] = lo[up] + 0.3;
                p
            },
        ),
    ]
}

// ---------------------------------------------------------------------------
// 注册
// ---------------------------------------------------------------------------

pub fn register(registry: &mut StageRegistry, settings: &Settings) -> Vec<StageKind> {
    let mut registered = Vec::new();
    if let (Some(key), true) = (
        settings.providers.tripo.api_key.clone(),
        settings.providers.tripo.configured(),
    ) {
        let settings_arc = Arc::new(settings.clone());
        registry.register(
            StageKind::TripoSegment,
            TripoSegmentHandler::new(&settings.providers.tripo.base_url, key, settings_arc),
        );
        registered.push(StageKind::TripoSegment);
    }
    // AutoBind 只需要草稿和分件 GLB，不需要额外的 Provider 配置
    registry.register(StageKind::AutoBind, AutoBindHandler::new(settings));
    registered.push(StageKind::AutoBind);
    registered
}

#[cfg(test)]
mod tests {
    use super::*;
    use manual_core::knowledge::{Evidence, Part, ReviewStatus};

    fn part(name: &str) -> Part {
        Part {
            id: format!("part-{name}"),
            name: name.to_owned(),
            description: String::new(),
            evidence: Vec::<Evidence>::new(),
            review_status: ReviewStatus::NeedsReview,
            source_batches: vec![0],
        }
    }

    fn box_at(cx: f64, cy: f64, cz: f64, sx: f64, sy: f64, sz: f64) -> ([f64; 3], [f64; 3]) {
        (
            [cx - sx / 2.0, cy - sy / 2.0, cz - sz / 2.0],
            [cx + sx / 2.0, cy + sy / 2.0, cz + sz / 2.0],
        )
    }

    #[test]
    fn opaque_numbered_nodes_never_inherit_manual_part_order() {
        let bounds = std::collections::BTreeMap::from([
            ("part_0".to_owned(), box_at(0.0, 0.0, 0.0, 1.0, 1.0, 1.0)),
            ("part_1".to_owned(), box_at(0.5, 0.0, 0.0, 0.1, 0.1, 0.1)),
        ]);
        let nodes = bounds.keys().cloned().collect::<Vec<_>>();
        assert!(heuristic_bind(&[part("镜头"), part("快门按钮")], &nodes, &bounds).is_empty());
    }

    #[test]
    fn semantic_binding_requires_one_exact_normalized_name() {
        let bounds = std::collections::BTreeMap::from([
            ("電池 蓋".to_owned(), box_at(0.2, 0.0, 0.0, 0.1, 0.1, 0.1)),
            ("LENS".to_owned(), box_at(-0.2, 0.0, 0.0, 0.1, 0.1, 0.1)),
            ("Lens".to_owned(), box_at(-0.1, 0.0, 0.0, 0.1, 0.1, 0.1)),
        ]);
        let nodes = bounds.keys().cloned().collect::<Vec<_>>();
        let bindings = heuristic_bind(
            &[part("电池盖"), part("lens"), part("按钮")],
            &nodes,
            &bounds,
        );
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].0, "part-电池盖");
        assert_eq!(bindings[0].1, ["電池 蓋"]);
    }

    #[test]
    fn actions_cover_traditional_names_and_dedupe_labels() {
        let mut bounds = std::collections::BTreeMap::new();
        bounds.insert("body".into(), box_at(0.0, 0.0, 0.0, 1.0, 0.3, 1.0));
        bounds.insert("lid".into(), box_at(0.1, 0.14, 0.0, 0.2, 0.02, 0.2));
        bounds.insert("bin".into(), box_at(0.35, 0.05, 0.0, 0.15, 0.1, 0.2));
        bounds.insert("key".into(), box_at(-0.1, 0.15, 0.0, 0.03, 0.01, 0.03));
        bounds.insert("key2".into(), box_at(-0.2, 0.15, 0.0, 0.03, 0.01, 0.03));
        let mut dup = part("回充鍵");
        dup.id = "part-dup".into();
        let mut spaced = part("回 充 鍵");
        spaced.id = "part-spaced".into();
        let parts = vec![
            part("掀蓋口"),
            part("集塵盒"),
            part("回充鍵"),
            dup,
            part("集塵盒卡扣"),
            spaced,
            part("四肢/膝蓋"),
        ];
        let bindings = vec![
            (
                "part-掀蓋口".to_owned(),
                vec!["lid".to_owned()],
                [0.1, 0.14, 0.0],
            ),
            (
                "part-集塵盒".to_owned(),
                vec!["bin".to_owned()],
                [0.35, 0.05, 0.0],
            ),
            (
                "part-回充鍵".to_owned(),
                vec!["key".to_owned()],
                [-0.1, 0.15, 0.0],
            ),
            (
                "part-dup".to_owned(),
                vec!["key2".to_owned()],
                [-0.2, 0.15, 0.0],
            ),
            (
                "part-集塵盒卡扣".to_owned(),
                vec!["clip".to_owned()],
                [0.3, 0.1, 0.0],
            ),
            (
                "part-spaced".to_owned(),
                vec!["key3".to_owned()],
                [-0.3, 0.15, 0.0],
            ),
            (
                "part-四肢/膝蓋".to_owned(),
                vec!["knee".to_owned()],
                [0.2, -0.1, 0.2],
            ),
        ];
        bounds.insert("clip".into(), box_at(0.3, 0.1, 0.0, 0.02, 0.02, 0.02));
        bounds.insert("key3".into(), box_at(-0.3, 0.15, 0.0, 0.03, 0.01, 0.03));
        bounds.insert("knee".into(), box_at(0.2, -0.1, 0.2, 0.04, 0.04, 0.04));
        let actions = derive_actions(&parts, &bindings, &bounds, &[]);
        let labels: Vec<_> = actions
            .iter()
            .map(|a| a["label"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            labels,
            ["打开掀蓋口", "取出集塵盒", "按下回充鍵"],
            "{actions:?}"
        );
    }

    #[test]
    fn camera_parts_do_not_get_quadruped_poses() {
        let mut bounds = std::collections::BTreeMap::new();
        for i in 0..12 {
            bounds.insert(
                format!("tripo_part_{i}"),
                box_at((i as f64) * 0.04 - 0.2, 0.0, 0.0, 0.05, 0.08, 0.04),
            );
        }
        let poses = derive_poses(&bounds, &[part("照相机"), part("快门释放按钮")]);
        assert!(poses.is_empty(), "{poses:?}");
    }

    #[test]
    fn quadruped_name_with_four_low_legs_gets_poses() {
        let mut bounds = std::collections::BTreeMap::new();
        bounds.insert("body".into(), box_at(0.0, 0.12, 0.0, 0.5, 0.12, 0.2));
        bounds.insert("head".into(), box_at(0.28, 0.16, 0.0, 0.1, 0.1, 0.1));
        // four shins touching the floor
        bounds.insert("shin_fl".into(), box_at(0.18, 0.05, 0.08, 0.04, 0.12, 0.04));
        bounds.insert(
            "shin_fr".into(),
            box_at(0.18, 0.05, -0.08, 0.04, 0.12, 0.04),
        );
        bounds.insert(
            "shin_rl".into(),
            box_at(-0.18, 0.05, 0.08, 0.04, 0.12, 0.04),
        );
        bounds.insert(
            "shin_rr".into(),
            box_at(-0.18, 0.05, -0.08, 0.04, 0.12, 0.04),
        );
        bounds.insert("foot_fl".into(), box_at(0.18, 0.01, 0.08, 0.03, 0.02, 0.03));
        bounds.insert("extra".into(), box_at(0.0, 0.18, 0.0, 0.08, 0.04, 0.08));
        let poses = derive_poses(&bounds, &[part("仿生四足机器人"), part("电源键")]);
        let ids: Vec<_> = poses
            .iter()
            .filter_map(|p| p.get("id").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            ids,
            ["stand", "lie-down", "sit", "shake-hand", "bow"],
            "{poses:?}"
        );
    }

    #[test]
    fn standing_dog_whose_box_looks_z_up_still_gets_poses() {
        // 站立机器狗：Y 是真实高度，但 Z 跨度略大，比例推断会误选 Z 为垂直轴。
        let mut bounds = std::collections::BTreeMap::new();
        bounds.insert("body".into(), box_at(0.0, 0.1, 0.0, 0.7, 0.2, 0.9));
        bounds.insert("head".into(), box_at(0.0, 0.25, 0.4, 0.2, 0.2, 0.15));
        for (name, x, z) in [
            ("fl", 0.2, 0.3),
            ("fr", -0.2, 0.3),
            ("rl", 0.2, -0.3),
            ("rr", -0.2, -0.3),
        ] {
            bounds.insert(format!("shin_{name}"), box_at(x, -0.25, z, 0.06, 0.3, 0.06));
        }
        for i in 0..4 {
            bounds.insert(
                format!("bit_{i}"),
                box_at(0.0, 0.15, (i as f64) * 0.1 - 0.2, 0.05, 0.05, 0.05),
            );
        }
        let poses = derive_poses(&bounds, &[part("仿生四足机器人")]);
        assert_eq!(poses.len(), 5, "{poses:?}");
    }

    #[test]
    fn pose_steps_respect_node_limit_on_large_models() {
        let mut bounds = std::collections::BTreeMap::new();
        bounds.insert("body".into(), box_at(0.0, 0.12, 0.0, 0.5, 0.12, 0.2));
        for (name, x, z) in [
            ("fl", 0.18, 0.08),
            ("fr", 0.18, -0.08),
            ("rl", -0.18, 0.08),
            ("rr", -0.18, -0.08),
        ] {
            bounds.insert(format!("shin_{name}"), box_at(x, 0.05, z, 0.04, 0.12, 0.04));
        }
        for i in 0..80 {
            bounds.insert(format!("bit_{i}"), box_at(0.0, 0.15, 0.0, 0.01, 0.01, 0.01));
        }
        let poses = derive_poses(&bounds, &[part("机器狗")]);
        assert!(!poses.is_empty());
        for pose in &poses {
            for step in pose["steps"].as_array().unwrap() {
                let n = step["nodes"].as_array().unwrap().len();
                assert!((1..=64).contains(&n), "{} step has {n} nodes", pose["id"]);
            }
        }
    }
}
