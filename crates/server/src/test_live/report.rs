use super::{
    LiveError, Result, RunResult,
    contract::{Budget, Case, Mode},
    plan,
};
use crate::{config::encrypted_secrets::write_private_file, storage::repo};
use manual_core::domain::{DraftStatus, JobStatus};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use std::{fs, path::Path};

fn artifact(output: &Path, name: &str, bytes: &[u8]) -> Result<Value> {
    let sha = manual_core::generation::sha256_hex(bytes);
    let path = output.join(name);
    if path.exists() {
        if plan::file_digest(&path)? != sha {
            return Err(LiveError::new("output"));
        }
    } else {
        write_private_file(&path, bytes, false).map_err(|_| LiveError::new("output"))?;
    }
    Ok(json!({"file":name,"sha256":sha,"bytes":bytes.len()}))
}

pub struct ReportContext<'a> {
    pub case: &'a Case,
    pub budget: &'a Budget,
    pub plan: &'a Value,
    pub pool: &'a SqlitePool,
    pub data_dir: &'a Path,
    pub job_id: &'a str,
    pub output: &'a Path,
}

pub async fn finish(context: ReportContext<'_>, stop: Option<&'static str>) -> Result<RunResult> {
    let ReportContext {
        case,
        budget,
        plan,
        pool,
        data_dir,
        job_id,
        output,
    } = context;
    let mut conn = pool
        .acquire()
        .await
        .map_err(|_| LiveError::new("storage"))?;
    let job = repo::jobs::get(&mut conn, job_id)
        .await
        .map_err(|_| LiveError::new("storage"))?
        .ok_or_else(|| LiveError::new("journal"))?;
    let stages = repo::job_stages::list_for_job(&mut conn, job_id)
        .await
        .map_err(|_| LiveError::new("storage"))?;
    let entries = repo::ledger::list_for_snapshot(&mut conn, &job.snapshot_id)
        .await
        .map_err(|_| LiveError::new("storage"))?;
    let attempts = repo::attempts::list_for_job(&mut conn, job_id)
        .await
        .map_err(|_| LiveError::new("storage"))?;
    let draft = repo::drafts::get_by_snapshot(&mut conn, &job.snapshot_id)
        .await
        .map_err(|_| LiveError::new("storage"))?;
    let mut saved = Vec::new();
    let mut draft_id = None;
    if let Some(draft) = &draft {
        draft_id = Some(draft.id.clone());
        if let Some(id) = &draft.model_revision_id {
            let model = repo::model_revisions::get(&mut conn, id)
                .await
                .map_err(|_| LiveError::new("storage"))?
                .ok_or_else(|| LiveError::new("materialAsset"))?;
            let bytes = fs::read(crate::assets::blob_path(data_dir, &model.sha256))
                .map_err(|_| LiveError::new("materialAsset"))?;
            if manual_core::generation::sha256_hex(&bytes) != model.sha256 {
                return Err(LiveError::new("materialAsset"));
            }
            saved.push(artifact(output, "model.glb", &bytes)?);
        }
        // The artifact is extracted structured knowledge, never the original PDF
        // or raw provider response. The public report only carries its identity/hash.
        let knowledge: crate::drafts::knowledge::DraftKnowledge =
            serde_json::from_value(draft.knowledge_json.clone())
                .map_err(|_| LiveError::new("materialAsset"))?;
        if let Some(knowledge) = knowledge.knowledge {
            let bytes=serde_json::to_vec_pretty(&json!({"schemaVersion":1,"draftId":draft.id,"status":"needs_review","knowledge":knowledge})).map_err(|_|LiveError::new("output"))?;
            saved.push(artifact(output, "knowledge.json", &bytes)?);
        }
    }
    // Preserve branch outputs even before an assemble_draft checkpoint exists.
    // Their local DB/asset identities remain in the report; no raw error text,
    // usage JSON, provider URL, credentials, session or original text is copied.
    let mut assets = Vec::new();
    for stage in &stages {
        if let Some(asset_id) = &stage.result_asset_id
            && let Some((asset, blob)) = repo::assets::get_with_blob(&mut conn, asset_id)
                .await
                .map_err(|_| LiveError::new("storage"))?
        {
            assets.push(json!({"stageId":stage.id,"assetId":asset.id,"sha256":blob.sha256,"purpose":asset.purpose.as_str()}));
        }
    }
    let unknown = stages
        .iter()
        .any(|s| s.status == JobStatus::SubmissionUnknown)
        || entries
            .iter()
            .any(|e| e.state == manual_core::domain::LedgerState::Unknown)
        || attempts.iter().any(|a| {
            matches!(
                a.submit_state,
                manual_core::domain::SubmitState::Submitting
                    | manual_core::domain::SubmitState::Unknown
            )
        });
    let mut sums = [0_i64; 2];
    let mut overflow = false;
    for e in &entries {
        if let Some(actual) = e.actual {
            let index = if e.currency == manual_core::domain::Currency::CreditMinor {
                0
            } else {
                1
            };
            match sums[index].checked_add(actual) {
                Some(sum) => sums[index] = sum,
                None => overflow = true,
            }
        }
    }
    let risk =
        overflow || sums[0] > budget.limits.credit_minor || sums[1] > budget.limits.usd_micros;
    let success = stop.is_none()
        && !unknown
        && !risk
        && job.status == JobStatus::Succeeded
        && stages.iter().all(|s| s.status == JobStatus::Succeeded)
        && draft
            .as_ref()
            .is_some_and(|d| d.status == DraftStatus::NeedsReview)
        && saved.iter().any(|a| a["file"] == "model.glb")
        && saved.iter().any(|a| a["file"] == "knowledge.json");
    let title = if success {
        if case.mode == Mode::LoopbackFixture {
            "本机链路验证通过；真实供应商验收未运行 · 已生成待复核草稿"
        } else {
            "已生成待复核草稿；人工复核及真实供应商正式验收仍待完成"
        }
    } else if unknown {
        "付费提交结果待对账，未发起再次购买"
    } else {
        "执行未完成；已保存成果保留"
    };
    let data = json!({
        "schemaVersion":1,
        "caseId":case.case_id,
        "mode":case.mode,
        "authorizationId":budget.authorization_id,
        "planHash":budget.plan_hash,
        "inputHash":plan["inputHash"],
        "jobId":job.id,
        "snapshotId":job.snapshot_id,
        "draftId":draft_id,
        "status":if success{"needs_review"}else if unknown{"submission_unknown"}else{"needs_input"},
        "stoppedBy":stop,
        "budgetRisk":risk,
        "plannedUpperBound":plan["requiredLimits"],
        "authorizedLimits":budget.limits,
        "maxInitialGenerations":budget.max_initial_generations,
        "retryScopes":budget.retry_scopes,
        "costs":entries.iter().map(|e|json!({"provider":e.provider.as_str(),"currency":e.currency.as_str(),"reserved":e.reserved,"actual":e.actual,"state":e.state.as_str()})).collect::<Vec<_>>(),
        "stages":stages.iter().map(|s|json!({"id":s.id,"kind":s.stage_kind.as_str(),"batchIndex":s.batch_index,"inputHash":s.input_hash,"status":s.status.as_str()})).collect::<Vec<_>>(),
        "attempts":attempts.iter().map(|a|json!({"id":a.id,"stageId":a.stage_id,"state":a.submit_state.as_str(),"remoteTaskIdHash":a.remote_task_id.as_ref().map(plan::digest)})).collect::<Vec<_>>(),
        "savedArtifacts":saved,
        "savedAssets":assets,
        "notCompleted":stages.iter().filter(|s|s.status!=JobStatus::Succeeded).map(|s|json!({"kind":s.stage_kind.as_str(),"batchIndex":s.batch_index,"status":s.status.as_str()})).collect::<Vec<_>>(),
        "nextAction":if success{"在网页人工复核知识、模型和热点；由用户显式发布"}else if unknown{"通过现有任务对账入口核对付费事实；保留授权记录，不自动重购"}else{"核对未完成阶段或授权；保留成果，使用同一授权恢复"},
        "realSupplierAcceptance":"NOT_RUN",
        "AC-042":"NOT_RUN",
        "T23":"NOT_RUN",
        "automaticReviews":0,
        "automaticReleases":0,
        "reportFile":"report.json"
    });
    write_private_file(
        &output.join("report.json"),
        &serde_json::to_vec_pretty(&data).map_err(|_| LiveError::new("output"))?,
        true,
    )
    .map_err(|_| LiveError::new("output"))?;
    Ok(RunResult {
        success,
        title,
        data,
    })
}
