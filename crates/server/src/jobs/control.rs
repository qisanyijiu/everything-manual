//! 任务控制动作：取消 / 按分支重试 / 对账（T15 / REQ-025、REQ-026；contracts.md §5）。
//!
//! 三个动作的共同原则：
//! - **If-Match**：修改的是父 job 聚合（revision CAS），过期 → 412；
//! - **审计**：每个动作写 `audit_events`（只存必要摘要，含原因与结果）；
//! - **付费安全**：不自动重复购买——`submission_unknown` 不是重试入口（必须对账），
//!   取消后不再产生任何新的付费步骤，对账的 `authorizeReplacement` 必须显式确认
//!   重复收费风险并再次预算确认；`unknown` 预留不自动释放。
//!
//! 语义裁定（RD，理由见 implementation.md §T15）：
//! - **取消**（[`cancel_job`]）：未提交阶段转 `cancelled`；已提交阶段
//!   （`waiting_provider`）与 `submission_unknown` 保持原状态——供应商侧可能已在计费，
//!   本地取消不保证对方撤单；响应/文案不得声称"已取消远端付费操作"。取消后执行器
//!   不再领取该 job 的任何阶段（T10 已验收语义），已提交阶段的 attempt、远端 task ID
//!   与预留账务保留可查（"保留查询与账务收尾"）。
//! - **重试**（[`retry_stage`]）：只接受 `failed` / `needs_input` 的阶段，把它们拉回
//!   `queued`；已完成成果保留（只重置该阶段，不改快照/模型/质量预设）；同分支存在
//!   `submission_unknown` 时拒绝（先对账）；已取消的 job 拒绝（取消后不新增购买）。
//! - **对账**（[`reconcile`]）：`attachRemoteTask`（仅 Tripo，查询验证可访问性 + 用户
//!   二次确认）/ `recordNoTask`（要求核查证据；是管理员声明，**不是**供应商出具的证明）/
//!   `authorizeReplacement`（再次预算确认 + 明确重复收费风险）。三者都不自动释放预留。

use manual_core::domain::{Job, JobStage, JobStatus, ProviderAttempt, StageKind, SubmitState};
use manual_core::jobs::{SubmissionStyle, submission_style};
use manual_core::timestamps::Timestamp;
use manual_core::validation::FieldIssue;
use serde_json::json;
use sqlx::{SqliteConnection, SqlitePool};

use crate::config::Settings;
use crate::storage::StorageError;
use crate::storage::repo::{self, job_stages as stages_repo};

/// 重试幂等记录的 scope（contracts.md §4：`admin + method + route + key`）。
pub const RETRY_IDEMPOTENCY_METHOD: &str = "POST";
pub const RETRY_IDEMPOTENCY_ROUTE: &str = "/api/v1/jobs/{id}/retry";

/// 审计动作（稳定字符串；QA/测试按这些码断言）。
pub const AUDIT_JOB_CANCELLED: &str = "job_cancelled";
pub const AUDIT_STAGE_RETRY_REQUESTED: &str = "job_stage_retry_requested";
pub const AUDIT_RECONCILE_ATTACH_REMOTE_TASK: &str = "job_reconcile_attach_remote_task";
pub const AUDIT_RECONCILE_RECORD_NO_TASK: &str = "job_reconcile_record_no_task";
pub const AUDIT_RECONCILE_AUTHORIZE_REPLACEMENT: &str = "job_reconcile_authorize_replacement";

/// 取消语义的固定文案（响应与 UI 都不得改写成"已取消远端付费操作"）。
pub const CANCEL_NOTICE: &str = "取消只停止本地的后续推进：已提交给供应商的付费操作不会被撤销，\
     已提交阶段保留查询与账务记录（不保证供应商撤单）";

/// 核查证据长度上限（只存必要摘要）。
pub const MAX_EVIDENCE_CHARS: usize = 500;

/// 阶段重试准入的判定结果（T17 / T15 P3① 的直接修复）。
///
/// **一处判据两处使用**：`POST /jobs/{id}/retry` 用它决定放行/拒绝，
/// `GET /jobs/{id}` 的 `stages[].retry` 用它告诉界面"这个阶段现在到底能不能重试、
/// 不能的话原因是什么"。这样界面不会出现"写着可重试、点了被 422 拒绝"的分叉文案。
///
/// `details` 只用于端点的 422 `error.details`（与既有 reason/details 形状一致），
/// 不进入任务详情的 DTO。
#[derive(Debug, Clone, PartialEq)]
pub struct RetryGate {
    pub allowed: bool,
    /// 稳定拒绝码（与端点 `details.reason` 同值）：
    /// `jobCancelled` / `stageNotRetryable` / `branchSubmissionUnknown` / `budgetNotHolding`。
    pub reason: Option<&'static str>,
    /// 可行动的说明（与端点 422 message 同源）。
    pub message: Option<String>,
    pub details: serde_json::Value,
}

impl RetryGate {
    fn allowed() -> Self {
        Self {
            allowed: true,
            reason: None,
            message: None,
            details: serde_json::Value::Null,
        }
    }

    fn denied(reason: &'static str, message: String, details: serde_json::Value) -> Self {
        Self {
            allowed: false,
            reason: Some(reason),
            message: Some(message),
            details,
        }
    }
}

/// 重试准入判定（按 [`retry_stage`] 的既有顺序与文案；判定为纯函数，便于单测与详情复用）。
///
/// `ledger_holds` 表示该阶段所属付费分支的预留是否仍占用预算
/// （`reserved`/`unknown` → true；`settled`/`released`/缺失 → false；本地阶段传 true）。
pub fn retry_gate(
    job_status: JobStatus,
    stages: &[JobStage],
    target: &JobStage,
    ledger_holds: bool,
) -> RetryGate {
    if job_status == JobStatus::Cancelled {
        return RetryGate::denied(
            "jobCancelled",
            "任务已取消：取消后不再发起任何新的付费步骤（重试被拒绝，无副作用）".to_owned(),
            json!({ "status": job_status.as_str() }),
        );
    }
    if !stages_repo::retryable_stage_status(target.status) {
        return RetryGate::denied(
            "stageNotRetryable",
            format!(
                "阶段 {}（{}）当前状态为 {}：只有 failed/needs_input 阶段可作为重试入口\
                 （submission_unknown 必须先对账；未知结果不得从此盲重试）",
                target.stage_kind.as_str(),
                target.id,
                target.status.as_str()
            ),
            json!({
                "stageId": target.id,
                "stageKind": target.stage_kind.as_str(),
                "status": target.status.as_str(),
            }),
        );
    }
    if let Some(blocking) = stages.iter().find(|other| {
        other.id != target.id
            && same_branch(other.stage_kind, target.stage_kind)
            && other.status == JobStatus::SubmissionUnknown
    }) {
        return RetryGate::denied(
            "branchSubmissionUnknown",
            format!(
                "同一分支的阶段 {} 存在未对账的付费提交（submission_unknown）：\
                 请先对账，暂停该分支后续购买",
                blocking.stage_kind.as_str()
            ),
            json!({
                "blockingStageId": blocking.id,
                "blockingStageKind": blocking.stage_kind.as_str(),
            }),
        );
    }
    if let Some(provider) = branch_provider(target.stage_kind)
        && !ledger_holds
    {
        return RetryGate::denied(
            "budgetNotHolding",
            format!(
                "{} 分支的预留未占用预算（已释放/已结算或缺失）：重试会重新发起请求，\
                 请重新获取报价并确认预算后再执行",
                provider.as_str()
            ),
            json!({ "provider": provider.as_str(), "stageId": target.id }),
        );
    }
    RetryGate::allowed()
}

/// 控制动作错误（HTTP 层映射为合同错误结构）。
#[derive(Debug, Clone, PartialEq)]
pub enum JobControlError {
    NotFound {
        entity: &'static str,
        id: String,
    },
    /// 字段级校验（422 + details.fields）。
    FieldValidation(Vec<FieldIssue>),
    /// 乐观锁冲突（412 + details.currentRevision）。
    RevisionConflict {
        entity: &'static str,
        id: String,
        current_revision: i64,
    },
    /// 业务前置不满足（422 + details.reason；沿用 T07/T09/T11 的 `details.reason` 惯例）。
    NotAllowed {
        reason: &'static str,
        message: String,
        details: serde_json::Value,
    },
    /// 幂等冲突（409 `IDEMPOTENCY_CONFLICT`）。
    IdempotencyConflict {
        message: String,
        details: serde_json::Value,
    },
    /// 供应商未配置（409 `PROVIDER_NOT_CONFIGURED`）。
    ProviderNotConfigured {
        missing: Vec<String>,
    },
    Storage(StorageError),
}

impl JobControlError {
    fn not_allowed(
        reason: &'static str,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self::NotAllowed {
            reason,
            message: message.into(),
            details,
        }
    }
}

impl From<StorageError> for JobControlError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::Error> for JobControlError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(StorageError::from(error))
    }
}

impl std::fmt::Display for JobControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { entity, id } => write!(formatter, "{entity} 不存在：{id}"),
            Self::FieldValidation(issues) => {
                write!(formatter, "字段校验失败（{} 项）", issues.len())
            }
            Self::RevisionConflict {
                current_revision, ..
            } => {
                write!(formatter, "revision 冲突（当前 {current_revision}）")
            }
            Self::NotAllowed { reason, .. } => write!(formatter, "不允许的操作：{reason}"),
            Self::IdempotencyConflict { .. } => write!(formatter, "幂等键冲突"),
            Self::ProviderNotConfigured { .. } => write!(formatter, "供应商未配置"),
            Self::Storage(error) => write!(formatter, "存储错误：{error}"),
        }
    }
}

impl std::error::Error for JobControlError {}

/// 取消结果。
#[derive(Debug, Clone)]
pub struct CancelReport {
    pub job: Job,
    /// 本次转为 `cancelled` 的未提交阶段数。
    pub stages_cancelled: u64,
    /// 保留原状态的已提交/未决阶段（`kind`、`status`）：账务与对账仍可查。
    pub preserved: Vec<PreservedStage>,
    /// 固定文案（响应必须携带；不得声称已取消远端付费操作）。
    pub notice: String,
}

/// 被取消动作保留的阶段（已提交/未决）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservedStage {
    pub stage_id: String,
    pub stage_kind: StageKind,
    pub status: JobStatus,
}

/// 重试结果。
#[derive(Debug, Clone)]
pub struct RetryReport {
    pub job: Job,
    pub stage_id: String,
    pub stage_kind: StageKind,
    pub previous_status: JobStatus,
    /// 一并拉回队列的已完成下游阶段数（例如旧的"部分草稿"组装）。
    pub requeued_dependents: u64,
    /// 是否为同键同 body 的重放（未产生新效果）。
    pub replayed: bool,
}

/// 对账动作类型（请求体 `action`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileAction {
    AttachRemoteTask,
    RecordNoTask,
    AuthorizeReplacement,
}

impl ReconcileAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AttachRemoteTask => "attachRemoteTask",
            Self::RecordNoTask => "recordNoTask",
            Self::AuthorizeReplacement => "authorizeReplacement",
        }
    }
}

/// 对账请求（已解析的字段；HTTP 层负责 JSON 形状）。
#[derive(Debug, Clone)]
pub struct ReconcileRequest {
    pub action: ReconcileAction,
    pub stage_id: String,
    /// `attachRemoteTask`：账户中查到的远端任务 ID（opaque string）。
    pub remote_task_id: Option<String>,
    /// `attachRemoteTask`：用户二次确认"该任务与这次 attempt 对应"。
    pub acknowledge_matches: bool,
    /// `recordNoTask`：核查证据（管理员声明，不是供应商证明）。
    pub evidence: Option<String>,
    /// `authorizeReplacement`：明确接受重复收费风险。
    pub acknowledge_duplicate_risk: bool,
    /// `authorizeReplacement`：再次预算确认（分列上限，必须覆盖冻结上界）。
    pub limits_tripo_credit_minor: Option<i64>,
    pub limits_manual_ai_usd_micros: Option<i64>,
}

/// 对账结果。
#[derive(Debug, Clone)]
pub struct ReconcileReport {
    pub job: Job,
    pub action: ReconcileAction,
    pub stage_id: String,
    pub stage_status: JobStatus,
    pub attempt_id: Option<String>,
    /// 固定文案（动作后果的如实说明）。
    pub notice: String,
}

// ---------------------------------------------------------------------------
// 取消
// ---------------------------------------------------------------------------

/// 取消任务（`POST /jobs/{id}/cancel`）。
pub async fn cancel_job(
    pool: &SqlitePool,
    job_id: &str,
    expected_revision: i64,
    actor: &str,
    now: Timestamp,
) -> Result<CancelReport, JobControlError> {
    let mut conn = pool.acquire().await?;
    let job = require_job(&mut conn, job_id).await?;
    if job.revision != expected_revision {
        return Err(JobControlError::RevisionConflict {
            entity: "job",
            id: job_id.to_owned(),
            current_revision: job.revision,
        });
    }
    if job.status.is_terminal() {
        return Err(JobControlError::not_allowed(
            "cancelNotNeeded",
            format!("任务已处于终态（{}）：无需取消", job.status.as_str()),
            json!({ "status": job.status.as_str(), "revision": job.revision }),
        ));
    }

    // `BEGIN IMMEDIATE`（BUG-006）：`jobs::cancel` 先读 job（状态判定）再写（状态 + 阶段 + 审计）。
    let tx = crate::storage::begin_write(&mut conn).await?;
    let mut tx = tx;
    let outcome = repo::jobs::cancel(&mut tx, job_id, now).await?;
    let stages = stages_repo::list_for_job(&mut tx, job_id).await?;
    let preserved: Vec<PreservedStage> = stages
        .iter()
        .filter(|stage| {
            matches!(
                stage.status,
                JobStatus::WaitingProvider | JobStatus::SubmissionUnknown
            )
        })
        .map(|stage| PreservedStage {
            stage_id: stage.id.clone(),
            stage_kind: stage.stage_kind,
            status: stage.status,
        })
        .collect();
    repo::audit::record(
        &mut tx,
        repo::audit::NewAuditEvent {
            entity_type: "job".to_owned(),
            entity_id: job_id.to_owned(),
            actor: Some(actor.to_owned()),
            action: AUDIT_JOB_CANCELLED.to_owned(),
            result: if outcome.cancelled {
                "cancelled".to_owned()
            } else {
                "notNeeded".to_owned()
            },
            metadata_json: Some(
                json!({
                    "stagesCancelled": outcome.stages_cancelled,
                    "preservedSubmittedStages": preserved
                        .iter()
                        .map(|stage| stage.stage_kind.as_str())
                        .collect::<Vec<_>>(),
                    "remoteCancellationNotClaimed": true,
                })
                .to_string(),
            ),
        },
        now,
    )
    .await?;
    tx.commit().await?;

    Ok(CancelReport {
        job: outcome.job,
        stages_cancelled: outcome.stages_cancelled,
        preserved,
        notice: CANCEL_NOTICE.to_owned(),
    })
}

// ---------------------------------------------------------------------------
// 重试
// ---------------------------------------------------------------------------

/// 按分支重试（`POST /jobs/{id}/retry`；If-Match + Idempotency-Key）。
///
/// `body_hash` 由 HTTP 层按规范化的请求体计算（同 key 同 body 重放 → 同一效果；
/// 同 key 不同 body → 409）。
// 参数多但都是显式的控制面输入（幂等键/body_hash/actor/时钟），保持签名可读、不做打包。
#[allow(clippy::too_many_arguments)]
pub async fn retry_stage(
    pool: &SqlitePool,
    admin_id: &str,
    job_id: &str,
    expected_revision: i64,
    stage_id: &str,
    idempotency_key: &str,
    body_hash: &str,
    now: Timestamp,
) -> Result<RetryReport, JobControlError> {
    // 1) 幂等键与重放检查（真正的并发竞争由唯一键兜底）。
    let key = validate_idempotency_key(idempotency_key)?;
    let mut conn = pool.acquire().await?;
    if let Some(record) = repo::idempotency::find(
        &mut conn,
        admin_id,
        RETRY_IDEMPOTENCY_METHOD,
        RETRY_IDEMPOTENCY_ROUTE,
        &key,
    )
    .await?
    {
        return replay_retry(&mut conn, record, body_hash, now).await;
    }

    // 2) 父 job 前置：存在、revision、未取消。
    let job = require_job(&mut conn, job_id).await?;
    if job.revision != expected_revision {
        return Err(JobControlError::RevisionConflict {
            entity: "job",
            id: job_id.to_owned(),
            current_revision: job.revision,
        });
    }
    // 3) 阶段前置：存在；准入判定（取消/可重试状态/同分支未知/预算背书）由
    //    [`retry_gate`] 单点决定——任务详情的 `stages[].retry` 用它同一份判据，
    //    避免界面文案与端点行为分叉（T15 P3①）。
    let stages = stages_repo::list_for_job(&mut conn, job_id).await?;
    let stage = stages
        .iter()
        .find(|stage| stage.id == stage_id)
        .ok_or_else(|| JobControlError::NotFound {
            entity: "job_stage",
            id: stage_id.to_owned(),
        })?;
    // 3b) 付费分支的预算背书：重试会再次发起请求，对应预留必须仍占用预算
    //     （reserved/unknown）。已被释放（明确未计费）或缺失的预留 = 这次重试没有
    //     预算背书 → 拒绝，请重新报价（REQ-023：不自动降质量/不无预算花费）。
    let ledger_holds = match branch_provider(stage.stage_kind) {
        Some(provider) => {
            let entries = repo::ledger::list_for_snapshot(&mut conn, &job.snapshot_id).await?;
            entries
                .iter()
                .find(|entry| entry.provider == provider)
                .map(|entry| manual_core::cost::ledger_state_holds_budget(entry.state))
                .unwrap_or(false)
        }
        None => true,
    };
    let gate = retry_gate(job.status, &stages, stage, ledger_holds);
    if !gate.allowed {
        return Err(JobControlError::not_allowed(
            gate.reason.unwrap_or("stageNotRetryable"),
            gate.message.unwrap_or_default(),
            gate.details,
        ));
    }
    let previous_status = stage.status;

    // 4) 事务：重置阶段 + 拉回已完成的下游 + 审计 + 幂等记录 + 父 job 状态收敛。
    // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
    let tx = crate::storage::begin_write(&mut conn).await?;
    let mut tx = tx;
    let reset = stages_repo::reset_for_retry(
        &mut tx,
        &stage.id,
        "人工按分支重试：重置为待执行（已完成成果保留，不改变模型/质量预设）",
        now,
    )
    .await?;
    if !reset {
        tx.rollback().await?;
        return Err(JobControlError::not_allowed(
            "stageNotRetryable",
            "阶段状态已被其他操作改变：请刷新任务详情后重试",
            json!({ "stageId": stage.id }),
        ));
    }
    let requeued_dependents = stages_repo::requeue_succeeded_dependents(
        &mut tx,
        &stage.id,
        "上游阶段被人工重试：该阶段的旧产物基于旧输入，需重新计算",
        now,
    )
    .await?;
    repo::audit::record(
        &mut tx,
        repo::audit::NewAuditEvent {
            entity_type: "job_stage".to_owned(),
            entity_id: stage.id.clone(),
            actor: Some(admin_id.to_owned()),
            action: AUDIT_STAGE_RETRY_REQUESTED.to_owned(),
            result: "queued".to_owned(),
            metadata_json: Some(
                json!({
                    "jobId": job_id,
                    "stageKind": stage.stage_kind.as_str(),
                    "batchIndex": stage.batch_index,
                    "previousStatus": previous_status.as_str(),
                    "requeuedDependents": requeued_dependents,
                })
                .to_string(),
            ),
        },
        now,
    )
    .await?;
    match repo::idempotency::insert(
        &mut tx,
        repo::idempotency::NewIdempotencyRecord {
            admin_id: admin_id.to_owned(),
            method: RETRY_IDEMPOTENCY_METHOD.to_owned(),
            route: RETRY_IDEMPOTENCY_ROUTE.to_owned(),
            key: key.clone(),
            body_hash: body_hash.to_owned(),
            resource_id: Some(stage.id.clone()),
            response_status: Some(200),
        },
        now,
    )
    .await
    {
        Ok(_) => {}
        Err(StorageError::UniqueViolation { .. }) => {
            // 并发同键：回滚后按已存在的记录重放/报冲突。
            tx.rollback().await?;
            let record = repo::idempotency::find(
                &mut conn,
                admin_id,
                RETRY_IDEMPOTENCY_METHOD,
                RETRY_IDEMPOTENCY_ROUTE,
                &key,
            )
            .await?
            .ok_or_else(|| {
                JobControlError::Storage(StorageError::Database {
                    detail: "重试幂等键竞争后找不到已存在的记录".to_owned(),
                })
            })?;
            return replay_retry(&mut conn, record, body_hash, now).await;
        }
        Err(error) => return Err(error.into()),
    }
    let job = repo::jobs::recompute_status_after_retry(&mut tx, job_id, now)
        .await?
        .ok_or_else(|| JobControlError::NotFound {
            entity: "job",
            id: job_id.to_owned(),
        })?;
    tx.commit().await?;

    tracing::info!(
        event = "job_stage_retry_queued",
        jobId = %job_id,
        stageId = %stage.id,
        stageKind = stage.stage_kind.as_str(),
        previousStatus = previous_status.as_str(),
        requeuedDependents = requeued_dependents,
        "人工重试：仅重置指定阶段（已完成成果保留）"
    );
    Ok(RetryReport {
        job,
        stage_id: stage.id.clone(),
        stage_kind: stage.stage_kind,
        previous_status,
        requeued_dependents,
        replayed: false,
    })
}

/// 重放（同 key 同 body）或 409（同 key 不同 body）。
async fn replay_retry(
    conn: &mut SqliteConnection,
    record: manual_core::domain::IdempotencyRecord,
    body_hash: &str,
    now: Timestamp,
) -> Result<RetryReport, JobControlError> {
    if record.body_hash != body_hash {
        return Err(JobControlError::IdempotencyConflict {
            message: "该 Idempotency-Key 已用于不同的请求内容：请使用新的键".to_owned(),
            details: json!({
                "reason": "idempotencyKeyReused",
                "existingResourceId": record.resource_id,
            }),
        });
    }
    let stage_id = record.resource_id.clone().ok_or_else(|| {
        JobControlError::Storage(StorageError::Database {
            detail: "重试幂等记录缺少 resource_id".to_owned(),
        })
    })?;
    let stage =
        stages_repo::get(conn, &stage_id)
            .await?
            .ok_or_else(|| JobControlError::NotFound {
                entity: "job_stage",
                id: stage_id.clone(),
            })?;
    let job = require_job(conn, &stage.job_id).await?;
    let _ = now;
    Ok(RetryReport {
        job,
        stage_id: stage.id.clone(),
        stage_kind: stage.stage_kind,
        previous_status: stage.status,
        requeued_dependents: 0,
        replayed: true,
    })
}

/// 幂等键校验（缺失/空/超长 → 422 字段级；与 T11 建单同一规则）。
fn validate_idempotency_key(key: &str) -> Result<String, JobControlError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(JobControlError::FieldValidation(vec![FieldIssue::new(
            "idempotencyKey",
            "缺少 Idempotency-Key 头：重试必须携带幂等键（重放不产生第二个 attempt）",
        )]));
    }
    if trimmed.chars().count() > crate::generation::jobs::IDEMPOTENCY_KEY_MAX_CHARS {
        return Err(JobControlError::FieldValidation(vec![FieldIssue::new(
            "idempotencyKey",
            format!(
                "幂等键过长（上限 {} 字符）",
                crate::generation::jobs::IDEMPOTENCY_KEY_MAX_CHARS
            ),
        )]));
    }
    Ok(trimmed.to_owned())
}

// ---------------------------------------------------------------------------
// 对账
// ---------------------------------------------------------------------------

/// 对账（`POST /jobs/{id}/reconcile`；仅管理员会话可达）。
pub async fn reconcile(
    pool: &SqlitePool,
    settings: &Settings,
    job_id: &str,
    expected_revision: i64,
    request: &ReconcileRequest,
    actor: &str,
    now: Timestamp,
) -> Result<ReconcileReport, JobControlError> {
    let mut conn = pool.acquire().await?;
    let job = require_job(&mut conn, job_id).await?;
    if job.revision != expected_revision {
        return Err(JobControlError::RevisionConflict {
            entity: "job",
            id: job_id.to_owned(),
            current_revision: job.revision,
        });
    }
    let stages = stages_repo::list_for_job(&mut conn, job_id).await?;
    let stage = stages
        .iter()
        .find(|stage| stage.id == request.stage_id)
        .ok_or_else(|| JobControlError::NotFound {
            entity: "job_stage",
            id: request.stage_id.clone(),
        })?;
    if stage.status != JobStatus::SubmissionUnknown {
        return Err(JobControlError::not_allowed(
            "stageNotUnknown",
            format!(
                "阶段 {}（{}）当前状态为 {}：对账只处理 submission_unknown 的阶段",
                stage.stage_kind.as_str(),
                stage.id,
                stage.status.as_str()
            ),
            json!({ "stageId": stage.id, "status": stage.status.as_str() }),
        ));
    }
    let attempt = repo::attempts::latest_for_stage(&mut conn, &stage.id).await?;

    match request.action {
        ReconcileAction::AttachRemoteTask => {
            attach_remote_task(
                &mut conn,
                settings,
                &job,
                stage,
                attempt.as_ref(),
                request,
                actor,
                now,
            )
            .await
        }
        ReconcileAction::RecordNoTask => {
            record_no_task(
                &mut conn,
                &job,
                stage,
                attempt.as_ref(),
                request,
                actor,
                now,
            )
            .await
        }
        ReconcileAction::AuthorizeReplacement => {
            authorize_replacement(
                &mut conn,
                &job,
                stage,
                attempt.as_ref(),
                request,
                actor,
                now,
            )
            .await
        }
    }
}

/// `attachRemoteTask`：仅 Tripo；查询验证类型与账号可访问性 + 用户二次确认。
#[allow(clippy::too_many_arguments)]
async fn attach_remote_task(
    conn: &mut SqliteConnection,
    settings: &Settings,
    job: &Job,
    stage: &JobStage,
    attempt: Option<&ProviderAttempt>,
    request: &ReconcileRequest,
    actor: &str,
    now: Timestamp,
) -> Result<ReconcileReport, JobControlError> {
    // ① 仅异步远端任务链路（Tripo）可用；同步 Manual AI 不提供该恢复选项。
    match submission_style(stage.stage_kind) {
        Some(SubmissionStyle::AsyncRemoteTask) => {}
        Some(SubmissionStyle::SyncResponse) => {
            return Err(JobControlError::not_allowed(
                "attachRemoteTaskUnsupported",
                "同步链路（说明书 AI）不提供 attachRemoteTask：完整响应未持久化时\
                 不能假定 response_id 可轮询/重取，请改用 recordNoTask 或 authorizeReplacement",
                json!({ "stageKind": stage.stage_kind.as_str() }),
            ));
        }
        None => {
            return Err(JobControlError::not_allowed(
                "attachRemoteTaskNotApplicable",
                "该阶段不是付费提交阶段：attachRemoteTask 只适用于远端任务提交",
                json!({ "stageKind": stage.stage_kind.as_str() }),
            ));
        }
    }
    // ② 字段：远端 task ID + 用户二次确认。
    let remote_task_id = request
        .remote_task_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            JobControlError::FieldValidation(vec![FieldIssue::new(
                "remoteTaskId",
                "必填：请填写在供应商账户中查到的任务 ID（opaque string，原样填写）",
            )])
        })?;
    if !request.acknowledge_matches {
        return Err(JobControlError::FieldValidation(vec![FieldIssue::new(
            "acknowledgeMatches",
            "必须显式二次确认：该任务与本次未决提交对应（不确认不允许附加）",
        )]));
    }
    let attempt = attempt.ok_or_else(|| {
        JobControlError::not_allowed(
            "attemptMissing",
            "该阶段没有可对账的 attempt 记录：请人工核对（不猜测远端状态）",
            json!({ "stageId": stage.id }),
        )
    })?;
    if !matches!(
        attempt.submit_state,
        SubmitState::Submitting | SubmitState::Unknown
    ) {
        return Err(JobControlError::not_allowed(
            "attemptNotUnresolved",
            format!(
                "attempt（{}）的提交状态为 {}：只有未决提交可附加远端任务",
                attempt.id,
                attempt.submit_state.as_str()
            ),
            json!({ "attemptId": attempt.id, "submitState": attempt.submit_state.as_str() }),
        ));
    }
    // ③ 供应商查询验证：类型与账号可访问性（失败不伪造"不存在"证明）。
    let verification = verify_remote_task(settings, &remote_task_id)
        .await
        .map_err(|error| {
            tracing::warn!(
                event = "reconcile_attach_verification_failed",
                jobId = %job.id,
                stageId = %stage.id,
                errorCode = error.code(),
                detail = %error.message,
                "附加远端任务的查询验证未通过：保留旧 attempt 与未决账务（无副作用）"
            );
            JobControlError::not_allowed(
                "remoteTaskVerificationFailed",
                format!(
                    "无法用当前供应商账户验证任务 {remote_task_id}：{}（未修改任何记录）",
                    error.message
                ),
                json!({
                    "reason": "remoteTaskVerificationFailed",
                    "remoteTaskId": remote_task_id,
                }),
            )
        })?;

    // ④ 落库：事实观察（null→值）+ 阶段恢复排队 + 审计。
    // `BEGIN IMMEDIATE`（BUG-006）：`record_remote_task_id` 先读（现状判定/冲突）再写。
    let tx = crate::storage::begin_write(conn).await?;
    let mut tx = tx;
    let outcome =
        repo::attempts::record_remote_task_id(&mut tx, &attempt.id, &remote_task_id, now).await?;
    match outcome {
        repo::attempts::RemoteTaskOutcome::Conflict { existing } => {
            tx.rollback().await?;
            return Err(JobControlError::not_allowed(
                "remoteTaskIdConflict",
                format!(
                    "该 attempt 已记录不同的远端 task ID（{existing}）：\
                     不覆盖已有事实，请人工核对两个 ID"
                ),
                json!({ "existingRemoteTaskId": existing }),
            ));
        }
        repo::attempts::RemoteTaskOutcome::Recorded
        | repo::attempts::RemoteTaskOutcome::SameAsRecorded => {}
    }
    // 已取消的 job 只记录事实（执行器不会领取已取消 job 的任何阶段），
    // 不把阶段拉回队列，避免制造"会继续推进"的假象。
    let resolve_to = if job.status == JobStatus::Cancelled {
        JobStatus::NeedsInput
    } else {
        JobStatus::Queued
    };
    let note = if resolve_to == JobStatus::Queued {
        "管理员附加远端任务（查询验证通过）：按已确认任务继续查询，不重新购买"
    } else {
        "任务已取消：远端任务事实已记录（保留查询/账务），不再自动推进"
    };
    stages_repo::apply_reconcile_resolution(&mut tx, &stage.id, resolve_to, note, now).await?;
    repo::audit::record(
        &mut tx,
        repo::audit::NewAuditEvent {
            entity_type: "provider_attempt".to_owned(),
            entity_id: attempt.id.clone(),
            actor: Some(actor.to_owned()),
            action: AUDIT_RECONCILE_ATTACH_REMOTE_TASK.to_owned(),
            result: outcome_code(&outcome).to_owned(),
            metadata_json: Some(
                json!({
                    "jobId": job.id,
                    "stageId": stage.id,
                    "remoteTaskId": remote_task_id,
                    "verification": {
                        "providerStatus": verification.status_raw,
                        "taskTypeChecked": true,
                        "accountAccessible": true,
                        "dataKeys": verification.data_keys,
                    },
                    "acknowledgedMatches": true,
                    "jobCancelled": job.status == JobStatus::Cancelled,
                })
                .to_string(),
            ),
        },
        now,
    )
    .await?;
    let job = repo::jobs::recompute_status(&mut tx, &job.id, now)
        .await?
        .unwrap_or_else(|| job.clone());
    tx.commit().await?;
    tracing::info!(
        event = "reconcile_attached_remote_task",
        jobId = %job.id,
        stageId = %stage.id,
        attemptId = %attempt.id,
        remoteTaskId = %remote_task_id,
        "管理员对账：已附加远端任务（不重新购买）"
    );
    Ok(ReconcileReport {
        job,
        action: ReconcileAction::AttachRemoteTask,
        stage_id: stage.id.clone(),
        stage_status: resolve_to,
        attempt_id: Some(attempt.id.clone()),
        notice: "已附加账户中查到的远端任务（已查询验证可访问）：后续按该任务继续查询，\
                 不重新提交付费请求"
            .to_owned(),
    })
}

/// `recordNoTask`：要求核查证据；是管理员声明，不是供应商出具的证明。
async fn record_no_task(
    conn: &mut SqliteConnection,
    job: &Job,
    stage: &JobStage,
    attempt: Option<&ProviderAttempt>,
    request: &ReconcileRequest,
    actor: &str,
    now: Timestamp,
) -> Result<ReconcileReport, JobControlError> {
    let evidence = request
        .evidence
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            JobControlError::FieldValidation(vec![FieldIssue::new(
                "evidence",
                "必填：请填写核查证据（例如账户内任务列表截图说明/查询时间与结果）；\
                 这是你的核查声明，本应用不代为伪造供应商“不存在”证明",
            )])
        })?;
    if evidence.chars().count() > MAX_EVIDENCE_CHARS {
        return Err(JobControlError::FieldValidation(vec![FieldIssue::new(
            "evidence",
            format!("核查证据过长（上限 {MAX_EVIDENCE_CHARS} 字符）"),
        )]));
    }
    let attempt = attempt.ok_or_else(|| {
        JobControlError::not_allowed(
            "attemptMissing",
            "该阶段没有可对账的 attempt 记录：请人工核对（不猜测远端状态）",
            json!({ "stageId": stage.id }),
        )
    })?;

    // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
    let tx = crate::storage::begin_write(conn).await?;
    let mut tx = tx;
    let reason = format!(
        "管理员核查记录（{actor}）：账户中未找到与该次提交对应的任务；\
         这是管理员声明而非供应商出具的证明。证据：{evidence}"
    );
    repo::attempts::mark_failed(&mut tx, &attempt.id, &reason, now).await?;
    // 结果未知的预留**不自动释放**：是否计费仍以供应商账单为准。
    let note = "管理员已记录核查证据（远端无对应任务）：该阶段可按重试重新授权执行；\
                未决预留不自动释放";
    stages_repo::apply_reconcile_resolution(&mut tx, &stage.id, JobStatus::NeedsInput, note, now)
        .await?;
    repo::audit::record(
        &mut tx,
        repo::audit::NewAuditEvent {
            entity_type: "provider_attempt".to_owned(),
            entity_id: attempt.id.clone(),
            actor: Some(actor.to_owned()),
            action: AUDIT_RECONCILE_RECORD_NO_TASK.to_owned(),
            result: "adminAssertedNoRemoteTask".to_owned(),
            metadata_json: Some(
                json!({
                    "jobId": job.id,
                    "stageId": stage.id,
                    "attemptId": attempt.id,
                    "evidence": evidence,
                    "providerProof": false,
                    "reservationReleased": false,
                })
                .to_string(),
            ),
        },
        now,
    )
    .await?;
    let job = repo::jobs::recompute_status(&mut tx, &job.id, now)
        .await?
        .unwrap_or_else(|| job.clone());
    tx.commit().await?;
    tracing::warn!(
        event = "reconcile_recorded_no_task",
        jobId = %job.id,
        stageId = %stage.id,
        attemptId = %attempt.id,
        "管理员对账：记录「账户中无对应任务」（管理员声明，预留不自动释放）"
    );
    Ok(ReconcileReport {
        job,
        action: ReconcileAction::RecordNoTask,
        stage_id: stage.id.clone(),
        stage_status: JobStatus::NeedsInput,
        attempt_id: Some(attempt.id.clone()),
        notice: "已记录核查证据（管理员声明，不是供应商证明）：该阶段可显式重试；\
                 未决预留不自动释放，重复收费风险由本次重试的授权承担"
            .to_owned(),
    })
}

/// `authorizeReplacement`：再次预算确认 + 明确重复收费风险；保留旧 attempt 未决账务。
async fn authorize_replacement(
    conn: &mut SqliteConnection,
    job: &Job,
    stage: &JobStage,
    attempt: Option<&ProviderAttempt>,
    request: &ReconcileRequest,
    actor: &str,
    now: Timestamp,
) -> Result<ReconcileReport, JobControlError> {
    if job.status == JobStatus::Cancelled {
        return Err(JobControlError::not_allowed(
            "jobCancelled",
            "任务已取消：取消后不再发起任何新的付费步骤（替代提交被拒绝，无副作用）",
            json!({ "status": job.status.as_str() }),
        ));
    }
    if !request.acknowledge_duplicate_risk {
        return Err(JobControlError::FieldValidation(vec![FieldIssue::new(
            "acknowledgeDuplicateRisk",
            "必须显式确认「可能重复收费」的风险：替代提交会产生新的付费请求，\
             旧提交的账务仍保留",
        )]));
    }
    let attempt = attempt.ok_or_else(|| {
        JobControlError::not_allowed(
            "attemptMissing",
            "该阶段没有可对账的 attempt 记录：请人工核对（不猜测远端状态）",
            json!({ "stageId": stage.id }),
        )
    })?;

    // 再次预算确认：给出的分列上限必须覆盖冻结快照的保守上界（provider 由阶段分支决定）。
    let snapshot = repo::snapshots::get(conn, &job.snapshot_id)
        .await?
        .ok_or_else(|| JobControlError::NotFound {
            entity: "generation_snapshot",
            id: job.snapshot_id.clone(),
        })?;
    let (provider, required_key, provided) = match branch_of(stage.stage_kind) {
        Branch::Model => (
            "tripo",
            "tripoCreditMinor",
            request.limits_tripo_credit_minor,
        ),
        Branch::Knowledge => (
            "manual_ai",
            "manualAiUsdMicros",
            request.limits_manual_ai_usd_micros,
        ),
        Branch::Local => {
            return Err(JobControlError::not_allowed(
                "authorizeReplacementNotApplicable",
                "该阶段不是付费提交阶段：替代提交只适用于付费分支",
                json!({ "stageKind": stage.stage_kind.as_str() }),
            ));
        }
    };
    let upper_key = "upperBound";
    let upper = snapshot
        .budgets
        .get(upper_key)
        .and_then(|value| value.get(required_key))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(i64::MAX);
    let provided = provided.ok_or_else(|| {
        JobControlError::FieldValidation(vec![FieldIssue::new(
            "limits",
            format!(
                "必填：替代提交需要再次预算确认（{provider} 的 {required_key} 上限，\
                 必须覆盖冻结上界 {upper}）"
            ),
        )])
    })?;
    if provided < upper {
        return Err(JobControlError::not_allowed(
            "budgetBelowPlannedUpperBound",
            format!(
                "替代提交的授权上限（{provided}）低于冻结的保守上界（{upper}）：\
                 拒绝发起超预算请求（不自动降质量/换模型）"
            ),
            json!({
                "provider": provider,
                "authorized": provided,
                "upperBound": upper,
            }),
        ));
    }

    // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
    let tx = crate::storage::begin_write(conn).await?;
    let mut tx = tx;
    let reason = format!(
        "管理员授权替代提交（{actor}）：旧提交结果未知将被替换，可能产生重复收费；\
         旧 attempt 的未决账务保留"
    );
    repo::attempts::mark_failed(&mut tx, &attempt.id, &reason, now).await?;
    // 保留旧 attempt 与账本（unknown 预留不自动释放）；新一次提交在阶段被领取时
    // 按提交窗口重新建 intent（该 attempt 已定性为 failed，可安全新建）。
    let note = "管理员授权替代提交（可能重复收费；旧未决账务保留）：阶段重新排队，\
                新一次提交会创建新的 attempt";
    stages_repo::apply_reconcile_resolution(&mut tx, &stage.id, JobStatus::Queued, note, now)
        .await?;
    repo::audit::record(
        &mut tx,
        repo::audit::NewAuditEvent {
            entity_type: "provider_attempt".to_owned(),
            entity_id: attempt.id.clone(),
            actor: Some(actor.to_owned()),
            action: AUDIT_RECONCILE_AUTHORIZE_REPLACEMENT.to_owned(),
            result: "authorized".to_owned(),
            metadata_json: Some(
                json!({
                    "jobId": job.id,
                    "stageId": stage.id,
                    "previousAttemptId": attempt.id,
                    "provider": provider,
                    "authorizedMinor": provided,
                    "plannedUpperBoundMinor": upper,
                    "acknowledgedDuplicateRisk": true,
                    "previousReservationRetained": true,
                })
                .to_string(),
            ),
        },
        now,
    )
    .await?;
    let job = repo::jobs::recompute_status_after_retry(&mut tx, &job.id, now)
        .await?
        .unwrap_or_else(|| job.clone());
    tx.commit().await?;
    tracing::warn!(
        event = "reconcile_authorized_replacement",
        jobId = %job.id,
        stageId = %stage.id,
        previousAttemptId = %attempt.id,
        "管理员对账：授权替代提交（可能重复收费；旧未决账务保留）"
    );
    Ok(ReconcileReport {
        job,
        action: ReconcileAction::AuthorizeReplacement,
        stage_id: stage.id.clone(),
        stage_status: JobStatus::Queued,
        attempt_id: Some(attempt.id.clone()),
        notice: "已授权替代提交：本次将产生新的付费请求，可能重复收费；\
                 旧 attempt 的未决账务与预留保留，不自动释放"
            .to_owned(),
    })
}

/// 远端任务的查询验证结果（只保留可诊断摘要：原始状态、字段名清单；不含 URL/凭据）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteTaskVerification {
    status_raw: String,
    data_keys: Vec<String>,
}

/// 验证错误（脱敏后的可行动说明）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteTaskVerificationError {
    code: &'static str,
    message: String,
}

impl RemoteTaskVerificationError {
    /// 稳定错误码（日志用；不含用户数据）。
    fn code(&self) -> &'static str {
        self.code
    }
}

/// `attachRemoteTask` 的供应商查询验证（仅 Tripo；**不写库**）。
///
/// 做三件事：
/// 1. 用**当前配置**的 Tripo 凭据查询 `GET /tasks/{id}`（未配置 → 409，不猜测）；
/// 2. 要求响应是任务形态（含 `status`，且含 `progress`/`output`/`task_id` 之一）：
///    只凭 HTTP 200 不足以证明"这是个任务"；
/// 3. 账户可访问性由供应商自身的鉴权语义保证：查询成功即该任务在当前账户可见，
///    查询失败（401/403/404/业务错误）→ 明确失败，**不**据此宣称"任务不存在"
///    （那是 `recordNoTask` 的管理员核查声明，不是机器可证的结论）。
///
/// 已知边界：T12 的 `TaskData` 不解析任务类型字段（`type`），因此"类型"检查以
/// 任务形态字段为基础；等 T23 用真实响应收敛字段后可在不改合同的前提下加强。
async fn verify_remote_task(
    settings: &Settings,
    remote_task_id: &str,
) -> Result<RemoteTaskVerification, RemoteTaskVerificationError> {
    use crate::providers::tripo::{TripoClient, TripoTimeouts};

    let provider = &settings.providers.tripo;
    if !provider.configured() {
        return Err(RemoteTaskVerificationError {
            code: "providerNotConfigured",
            message: format!("Tripo 未配置（缺：{}）", provider.missing().join("、")),
        });
    }
    let api_key = provider
        .api_key
        .clone()
        .ok_or_else(|| RemoteTaskVerificationError {
            code: "providerNotConfigured",
            message: "Tripo 缺少 api_key".to_owned(),
        })?;
    let client = TripoClient::new(&provider.base_url, api_key, TripoTimeouts::default()).map_err(
        |detail| RemoteTaskVerificationError {
            code: "providerConfigInvalid",
            message: format!("Tripo 客户端无法构造：{detail}"),
        },
    )?;
    let task =
        client
            .get_task(remote_task_id)
            .await
            .map_err(|error| RemoteTaskVerificationError {
                code: error.code(),
                message: error.redacted(),
            })?;
    let generation_shape = task
        .data_keys
        .iter()
        .any(|key| matches!(key.as_str(), "progress" | "output" | "task_id"));
    if !generation_shape {
        return Err(RemoteTaskVerificationError {
            code: "unexpectedResponse",
            message: format!(
                "该 ID 的响应不是任务形态（字段：{}）：拒绝附加（不猜测类型）",
                task.data_keys.join(",")
            ),
        });
    }
    Ok(RemoteTaskVerification {
        status_raw: task.status_raw,
        data_keys: task.data_keys,
    })
}

fn outcome_code(outcome: &repo::attempts::RemoteTaskOutcome) -> &'static str {
    match outcome {
        repo::attempts::RemoteTaskOutcome::Recorded => "recorded",
        repo::attempts::RemoteTaskOutcome::SameAsRecorded => "sameAsRecorded",
        repo::attempts::RemoteTaskOutcome::Conflict { .. } => "conflict",
    }
}

/// 阶段分支（重试的"同分支暂停"与对账的 provider 判定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    /// 模型分支（Tripo：上传→提交→查询→下载→校验）。
    Model,
    /// 知识分支（说明书 AI：批次→合并）。
    Knowledge,
    /// 本地阶段（不涉及付费分支暂停）。
    Local,
}

pub const fn branch_of(kind: StageKind) -> Branch {
    match kind {
        StageKind::ManualExtract | StageKind::ManualMerge => Branch::Knowledge,
        StageKind::TripoUpload
        | StageKind::TripoSubmit
        | StageKind::TripoPoll
        | StageKind::ModelDownload
        | StageKind::ModelValidate => Branch::Model,
        StageKind::FreezeInputs | StageKind::AssembleDraft => Branch::Local,
    }
}

/// 分支对应的供应商（`Local` → `None`；用于预算背书与对账的 provider 判定）。
pub const fn branch_provider(kind: StageKind) -> Option<manual_core::domain::ProviderKey> {
    match branch_of(kind) {
        Branch::Model => Some(manual_core::domain::ProviderKey::Tripo),
        Branch::Knowledge => Some(manual_core::domain::ProviderKey::ManualAi),
        Branch::Local => None,
    }
}

/// 两个阶段是否属于同一付费分支（`Local` 不与任何阶段同分支）。
fn same_branch(left: StageKind, right: StageKind) -> bool {
    matches!(
        (branch_of(left), branch_of(right)),
        (Branch::Model, Branch::Model) | (Branch::Knowledge, Branch::Knowledge)
    )
}

async fn require_job(conn: &mut SqliteConnection, job_id: &str) -> Result<Job, JobControlError> {
    repo::jobs::get(conn, job_id)
        .await?
        .ok_or_else(|| JobControlError::NotFound {
            entity: "job",
            id: job_id.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branches_are_isolated_for_pause_semantics() {
        assert_eq!(branch_of(StageKind::ManualExtract), Branch::Knowledge);
        assert_eq!(branch_of(StageKind::ManualMerge), Branch::Knowledge);
        assert_eq!(branch_of(StageKind::TripoSubmit), Branch::Model);
        assert_eq!(branch_of(StageKind::ModelValidate), Branch::Model);
        assert_eq!(branch_of(StageKind::AssembleDraft), Branch::Local);
        assert!(same_branch(
            StageKind::ManualExtract,
            StageKind::ManualMerge
        ));
        assert!(same_branch(
            StageKind::TripoUpload,
            StageKind::ModelValidate
        ));
        assert!(!same_branch(
            StageKind::ManualMerge,
            StageKind::ModelValidate
        ));
        assert!(!same_branch(
            StageKind::AssembleDraft,
            StageKind::AssembleDraft
        ));
    }
}
