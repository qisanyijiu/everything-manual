//! 任务路由（T11 创建 / T15 读取与控制；contracts.md §3/§4/§5）。
//!
//! 创建：`POST /items/{id}/jobs`（首次 **202**）：服务端重新校验引用、报价未过期、
//! 输入未变、预算足够，然后在**同一事务**内冻结 `generation_snapshot`、写
//! `cost_ledger` 预留、创建 job 与阶段、写幂等记录与审计事件。
//!
//! - `Idempotency-Key` 必填；相同 key + 相同 body 重放返回**同一 job**（不新建、
//!   不重复预留，响应头 `x-idempotent-replay: true`）；同 key 不同 body → 409
//!   `IDEMPOTENCY_CONFLICT`；
//! - **不接受前端传入的费用数值**：请求体没有费用字段（未知字段 422），
//!   预留金额 = 服务端计算的保守上界（报价快照回读）；
//! - 202 只表示**已入队**；阶段执行由后台执行器负责（真实适配器属 T12/T14）。
//!
//! 读取与控制（T15）：
//! - `GET /jobs`（游标分页）/ `GET /jobs/{id}`（阶段、费用、缺项、**ETag**）；
//! - `POST /jobs/{id}/cancel`（If-Match）：未提交阶段停止推进；已提交阶段保留
//!   查询与账务，**不声称已取消远端付费操作**；
//! - `POST /jobs/{id}/retry`（If-Match + Idempotency-Key）：只重跑 `failed`/`needs_input`
//!   阶段，`submission_unknown` 必须先对账；
//! - `POST /jobs/{id}/reconcile`（If-Match）：仅管理员（单管理员系统 = 已认证会话）
//!   处理 `submission_unknown`；三种动作细则见 `jobs::control`。

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, routing};
use manual_core::domain::{Job, JobStatus};
use manual_core::timestamps::Timestamp;
use manual_core::validation::FieldIssue;
use sqlx::SqliteConnection;

use crate::generation::jobs as jobs_service;
use crate::jobs::control as control_service;

use super::auth::SessionContext;
use super::body::JsonBody;
use super::dto::{
    BUDGET_NOTICE, CancelResponse, CancelResultDto, JobAttemptDto, JobCreateRequest, JobDetailDto,
    JobDetailResponse, JobDto, JobItemDto, JobListResponse, JobMissingItemDto, JobResponse,
    JobStageDto, JobStageRetryDto, JobStageSummaryDto, JobSummaryDto, PreservedStageDto,
    ReconcileActionDto, ReconcileRequestDto, ReconcileResponse, ReconcileResultDto, ReservationDto,
    RetryRequest, RetryResponse, RetryResultDto,
};
use super::error::{ApiError, RequestId};
use super::estimates::acquire;
use super::pagination::{Cursor, ListParams, parse_list_params};
use super::precondition::{etag_value, parse_if_match};
use super::state::AppState;
use crate::storage::repo;

/// `Idempotency-Key` 请求头（contracts.md §4）。
pub const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// 任务列表游标作用域（与过滤条件绑定：切换 itemId 必须从头分页）。
const SCOPE_ALL: &str = "jobs";
const SCOPE_ITEM_PREFIX: &str = "jobs:item:";

/// 重试的固定说明（不改变模型/质量预设；已完成成果保留）。
const RETRY_NOTICE: &str = "只重跑指定阶段：已完成的其他阶段成果保留，不改变模型/质量预设，\
     不自动降质量或换模型";

/// 受会话保护的任务路由。
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/items/{id}/jobs", routing::post(create_job))
        .route("/jobs", routing::get(list_jobs))
        .route("/jobs/{id}", routing::get(get_job))
        .route("/jobs/{id}/cancel", routing::post(cancel_job))
        .route("/jobs/{id}/retry", routing::post(retry_job))
        .route("/jobs/{id}/reconcile", routing::post(reconcile_job))
}

/// `POST /api/v1/items/{id}/jobs`。
#[utoipa::path(
    post,
    path = "/api/v1/items/{id}/jobs",
    tag = "generation",
    summary = "冻结输入并创建生成任务（幂等；首次 202）",
    description = "请求必须携带 `Idempotency-Key`（同 key 同 body 重放返回同一 job；\
                   同 key 不同 body → 409 `IDEMPOTENCY_CONFLICT`）。\
                   服务端重新校验报价未过期、已确认发送范围、输入未变（重算输入指纹）、\
                   价格版本未变、授权上限覆盖服务端保守上界（否则 422，不自动降质量/换模型）。\
                   同一事务内冻结快照 + 写入分列费用预留 + 创建 job 与阶段；\
                   **不接受前端传入的费用数值**（请求体无费用字段，金额以服务端计算为准）。\
                   一份报价只能创建一份任务（重复提交需重新报价）。202 只表示已入队。",
    params(("id" = String, Path, description = "物品 ID（UUIDv7）")),
    request_body = JobCreateRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 202, description = "任务已创建（重放返回同一任务；x-idempotent-replay 标记）", body = JobResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品/报价不存在（含跨物品）", body = super::dto::ApiErrorResponse),
        (status = 409, description = "同 key 不同 body（IDEMPOTENCY_CONFLICT）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "前置/预算/确认/输入变化等（details.reason）或字段级明细", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn create_job(
    State(state): State<AppState>,
    request_id: RequestId,
    axum::Extension(session): axum::Extension<SessionContext>,
    Path(item_id): Path<String>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<JobCreateRequest>,
) -> Response {
    let idempotency_key = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    match jobs_service::create_job(
        state.settings(),
        &mut connection,
        &item_id,
        &body,
        idempotency_key,
        &session.admin_id,
        Timestamp::now(),
    )
    .await
    {
        Ok(creation) => {
            let data = JobDto {
                id: creation.job.id.clone(),
                item_id: creation.job.item_id.clone(),
                snapshot_id: creation.job.snapshot_id.clone(),
                status: creation.job.status.as_str().to_owned(),
                revision: creation.job.revision,
                reservations: creation
                    .reservations
                    .iter()
                    .map(ReservationDto::from_entry)
                    .collect(),
                budget_notice: BUDGET_NOTICE.to_owned(),
                created_at: creation.job.created_at,
                updated_at: creation.job.updated_at,
            };
            if creation.replayed {
                tracing::info!(
                    requestId = %request_id,
                    jobId = %creation.job.id,
                    "同键同 body 重放：返回原任务（未新建、未重复预留）"
                );
            } else {
                tracing::info!(
                    requestId = %request_id,
                    jobId = %creation.job.id,
                    snapshotId = %creation.snapshot.id,
                    "任务已创建（冻结快照 + 分列费用预留，同事务）"
                );
            }
            let mut response = (StatusCode::ACCEPTED, Json(JobResponse { data })).into_response();
            if creation.replayed {
                response
                    .headers_mut()
                    .insert("x-idempotent-replay", "true".parse().expect("静态头值"));
            }
            response
        }
        Err(error) => ApiError::from(error).render(&request_id),
    }
}

// ---------------------------------------------------------------------------
// GET /jobs（列表）
// ---------------------------------------------------------------------------

/// `GET /api/v1/jobs` —— `{data, nextCursor}`；可选按物品过滤。
#[utoipa::path(
    get,
    path = "/api/v1/jobs",
    tag = "jobs",
    summary = "任务中心列表（游标分页）",
    description = "稳定排序 (createdAt DESC, id DESC)。可选 `itemId` 只列该物品的任务；\
                   nextCursor 是**不透明**字符串（已绑定过滤条件）——切换过滤条件时必须从头分页。\
                   每行给出整体状态、**阶段计数摘要**（active/blocked/unknown/failed…，不是百分比）、\
                   分列费用预留与草稿 id。未知状态如实返回（前端轮询用）。",
    params(
        ("limit" = Option<u32>, Query, description = "每页条数，默认 20，最大 100"),
        ("cursor" = Option<String>, Query, description = "上一页返回的 nextCursor（原样回传）"),
        ("itemId" = Option<String>, Query, description = "只看该物品的任务"),
    ),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "任务列表", body = JobListResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 422, description = "查询参数非法（details.fields）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn list_jobs(
    State(state): State<AppState>,
    request_id: RequestId,
    Query(params): Query<Vec<(String, String)>>,
) -> Response {
    let parsed: ListParams = match parse_list_params(&params, &["itemId"]) {
        Ok(parsed) => parsed,
        Err(issues) => return ApiError::field_validation(issues).render(&request_id),
    };
    let item_filter = super::pagination::raw_value(&params, "itemId")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let scope = match &item_filter {
        Some(item_id) => format!("{SCOPE_ITEM_PREFIX}{item_id}"),
        None => SCOPE_ALL.to_owned(),
    };
    let cursor = match parsed.cursor.as_deref() {
        None => None,
        Some(value) => match Cursor::parse(value, &scope) {
            Ok(cursor) => Some(cursor.into_tuple()),
            Err(issue) => return ApiError::field_validation(vec![issue]).render(&request_id),
        },
    };

    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let mut rows = match repo::jobs::list_page(
        &mut connection,
        item_filter.as_deref(),
        cursor,
        parsed.limit + 1,
    )
    .await
    {
        Ok(rows) => rows,
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };
    let has_more = rows.len() as u32 > parsed.limit;
    rows.truncate(parsed.limit as usize);
    let next_cursor = if has_more {
        rows.last()
            .map(|job| Cursor::encode(&scope, job.created_at.as_millis(), &job.id))
    } else {
        None
    };
    let mut data = Vec::with_capacity(rows.len());
    for job in &rows {
        match load_job_summary(&mut connection, job).await {
            Ok(summary) => data.push(summary),
            Err(error) => return error.render(&request_id),
        }
    }
    Json(JobListResponse { data, next_cursor }).into_response()
}

// ---------------------------------------------------------------------------
// GET /jobs/{id}（详情）
// ---------------------------------------------------------------------------

/// `GET /api/v1/jobs/{id}` —— 任务详情（带 `ETag: "r<revision>"`）。
#[utoipa::path(
    get,
    path = "/api/v1/jobs/{id}",
    tag = "jobs",
    summary = "任务详情（阶段/尝试/费用/缺项；ETag 供 cancel/retry/reconcile 的 If-Match）",
    description = "返回阶段明细（含 `needsInput` 缺项与 `knowledgeProduced` 事实——该批是否产出\
                   正式知识，与恢复判据同源）、付费 attempt（对账入口）、分列费用预留与草稿 id。\
                   未知状态如实返回；`submission_unknown` 显示对账入口而不是重试。",
    params(("id" = String, Path, description = "任务 ID（UUIDv7）")),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "任务详情（带 ETag）", body = JobDetailResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "任务不存在", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn get_job(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(job_id): Path<String>,
) -> Response {
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let job = match repo::jobs::get(&mut connection, &job_id).await {
        Ok(Some(job)) => job,
        Ok(None) => {
            return ApiError::not_found(format!("job 不存在：{job_id}")).render(&request_id);
        }
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };
    match load_job_detail(&mut connection, &job).await {
        Ok(detail) => job_response(detail, StatusCode::OK),
        Err(error) => error.render(&request_id),
    }
}

// ---------------------------------------------------------------------------
// POST /jobs/{id}/cancel
// ---------------------------------------------------------------------------

/// `POST /api/v1/jobs/{id}/cancel`（If-Match）。
#[utoipa::path(
    post,
    path = "/api/v1/jobs/{id}/cancel",
    tag = "jobs",
    summary = "取消任务（未提交阶段停止推进；已提交阶段保留查询与账务）",
    description = "必须携带 If-Match（任务详情的 ETag）。未提交阶段（含无已接受 attempt 的 \
                   running）转 cancelled；**已提交给供应商的阶段（waiting_provider）与 \
                   submission_unknown 保持原状态**：供应商侧可能已在计费，取消不保证对方撤单。\
                   响应固定携带 notice，明确「取消不撤销远端付费操作」；动作写入 audit_events。\
                   已终态任务 → 422（cancelNotNeeded）。",
    params(("id" = String, Path, description = "任务 ID（UUIDv7）")),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "取消结果（job + 保留阶段 + 后果说明）", body = CancelResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF/Origin 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "任务不存在", body = super::dto::ApiErrorResponse),
        (status = 412, description = "revision 过期（details.currentRevision）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "已终态（details.reason=cancelNotNeeded）或 If-Match 非法", body = super::dto::ApiErrorResponse),
        (status = 428, description = "缺少 If-Match", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn cancel_job(
    State(state): State<AppState>,
    request_id: RequestId,
    axum::Extension(session): axum::Extension<SessionContext>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let expected_revision = match parse_if_match(&headers) {
        Ok(revision) => revision,
        Err(error) => return error.render(&request_id),
    };
    let report = match control_service::cancel_job(
        state.database().pool(),
        &job_id,
        expected_revision,
        &session.admin_id,
        Timestamp::now(),
    )
    .await
    {
        Ok(report) => report,
        Err(error) => return ApiError::from(error).render(&request_id),
    };
    tracing::info!(
        requestId = %request_id,
        jobId = %job_id,
        stagesCancelled = report.stages_cancelled,
        preserved = report.preserved.len(),
        "任务已取消（未提交阶段停止；已提交阶段保留查询与账务；不声称已取消远端付费操作）"
    );
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    match load_job_detail(&mut connection, &report.job).await {
        Ok(detail) => Json(CancelResponse {
            data: CancelResultDto {
                job: detail,
                stages_cancelled: report.stages_cancelled,
                preserved_stages: report
                    .preserved
                    .iter()
                    .map(|stage| PreservedStageDto {
                        stage_id: stage.stage_id.clone(),
                        stage_kind: stage.stage_kind.as_str().to_owned(),
                        status: stage.status.as_str().to_owned(),
                    })
                    .collect(),
                notice: report.notice,
            },
        })
        .into_response(),
        Err(error) => error.render(&request_id),
    }
}

// ---------------------------------------------------------------------------
// POST /jobs/{id}/retry
// ---------------------------------------------------------------------------

/// `POST /api/v1/jobs/{id}/retry`（If-Match + Idempotency-Key）。
#[utoipa::path(
    post,
    path = "/api/v1/jobs/{id}/retry",
    tag = "jobs",
    summary = "按分支重试指定阶段（只重跑失败/缺项的阶段）",
    description = "必须携带 If-Match 与 `Idempotency-Key`（重放不产生第二个 attempt）。\
                   只接受 `failed` / `needs_input` 阶段：它们被拉回队列，已完成的其他阶段成果保留，\
                   **不改变模型/质量预设**。`submission_unknown` 不是重试入口（422，需先对账）；\
                   同分支存在未对账提交或任务已取消时同样拒绝且无副作用。动作写入 audit_events。",
    params(("id" = String, Path, description = "任务 ID（UUIDv7）")),
    request_body = RetryRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "重试已入队（重放返回同一结果；x-idempotent-replay 标记）", body = RetryResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF/Origin 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "任务或阶段不存在", body = super::dto::ApiErrorResponse),
        (status = 409, description = "同 key 不同 body（IDEMPOTENCY_CONFLICT）", body = super::dto::ApiErrorResponse),
        (status = 412, description = "revision 过期（details.currentRevision）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "不可重试/缺 Idempotency-Key/未知未对账（details.reason/fields）", body = super::dto::ApiErrorResponse),
        (status = 428, description = "缺少 If-Match", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn retry_job(
    State(state): State<AppState>,
    request_id: RequestId,
    axum::Extension(session): axum::Extension<SessionContext>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<RetryRequest>,
) -> Response {
    let expected_revision = match parse_if_match(&headers) {
        Ok(revision) => revision,
        Err(error) => return error.render(&request_id),
    };
    let stage_id = body
        .stage_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let Some(stage_id) = stage_id else {
        return ApiError::field_validation(vec![FieldIssue::new(
            "stageId",
            "必填：请从任务详情中选择要重试的阶段",
        )])
        .render(&request_id);
    };
    let idempotency_key = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    // body_hash 的规范形状（字段顺序固定）：同 key 不同 body → 409。
    let body_hash = manual_core::generation::sha256_hex(
        serde_json::json!({ "stageId": stage_id })
            .to_string()
            .as_bytes(),
    );

    let report = match control_service::retry_stage(
        state.database().pool(),
        &session.admin_id,
        &job_id,
        expected_revision,
        &stage_id,
        &idempotency_key,
        &body_hash,
        Timestamp::now(),
    )
    .await
    {
        Ok(report) => report,
        Err(error) => return ApiError::from(error).render(&request_id),
    };
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let detail = match load_job_detail(&mut connection, &report.job).await {
        Ok(detail) => detail,
        Err(error) => return error.render(&request_id),
    };
    let mut response = (
        StatusCode::OK,
        Json(RetryResponse {
            data: RetryResultDto {
                job: detail,
                stage_id: report.stage_id.clone(),
                stage_kind: report.stage_kind.as_str().to_owned(),
                previous_status: report.previous_status.as_str().to_owned(),
                requeued_dependents: report.requeued_dependents,
                notice: RETRY_NOTICE.to_owned(),
            },
        }),
    )
        .into_response();
    if report.replayed {
        response
            .headers_mut()
            .insert("x-idempotent-replay", "true".parse().expect("静态头值"));
    }
    response
}

// ---------------------------------------------------------------------------
// POST /jobs/{id}/reconcile
// ---------------------------------------------------------------------------

/// `POST /api/v1/jobs/{id}/reconcile`（If-Match；仅管理员会话可达）。
#[utoipa::path(
    post,
    path = "/api/v1/jobs/{id}/reconcile",
    tag = "jobs",
    summary = "对账 submission_unknown（attachRemoteTask / recordNoTask / authorizeReplacement）",
    description = "只处理 `submission_unknown` 的阶段；必须携带 If-Match。单管理员自托管下\
                   \"仅管理员\"=已认证会话。`attachRemoteTask`（**仅 Tripo**）：附加账户中查到的\
                   任务 ID，服务端会查询验证可访问性与任务形态，并要求 `acknowledgeMatches` 二次确认；\
                   `recordNoTask`：要求 `evidence` 核查证据（管理员声明，**不是**供应商出具的证明）；\
                   `authorizeReplacement`：要求 `acknowledgeDuplicateRisk` 与 `limits` 再次预算确认\
                   （覆盖冻结上界），创建新 attempt、保留旧 attempt 未决账务。unknown 预留不自动\
                   释放；全部动作写入 audit_events。",
    params(("id" = String, Path, description = "任务 ID（UUIDv7）")),
    request_body = ReconcileRequestDto,
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "对账结果（action 后果说明）", body = ReconcileResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF/Origin 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "任务或阶段不存在", body = super::dto::ApiErrorResponse),
        (status = 409, description = "供应商未配置（PROVIDER_NOT_CONFIGURED）", body = super::dto::ApiErrorResponse),
        (status = 412, description = "revision 过期（details.currentRevision）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "动作/字段/验证失败或前置不满足（details.reason/fields）", body = super::dto::ApiErrorResponse),
        (status = 428, description = "缺少 If-Match", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn reconcile_job(
    State(state): State<AppState>,
    request_id: RequestId,
    axum::Extension(session): axum::Extension<SessionContext>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<ReconcileRequestDto>,
) -> Response {
    let expected_revision = match parse_if_match(&headers) {
        Ok(revision) => revision,
        Err(error) => return error.render(&request_id),
    };
    let mut issues: Vec<FieldIssue> = Vec::new();
    let action = match body.action {
        Some(action) => Some(match action {
            ReconcileActionDto::AttachRemoteTask => {
                control_service::ReconcileAction::AttachRemoteTask
            }
            ReconcileActionDto::RecordNoTask => control_service::ReconcileAction::RecordNoTask,
            ReconcileActionDto::AuthorizeReplacement => {
                control_service::ReconcileAction::AuthorizeReplacement
            }
        }),
        None => {
            issues.push(FieldIssue::new(
                "action",
                "必填：attachRemoteTask / recordNoTask / authorizeReplacement",
            ));
            None
        }
    };
    let stage_id = body
        .stage_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if stage_id.is_none() {
        issues.push(FieldIssue::new(
            "stageId",
            "必填：处于 submission_unknown 的阶段 id（来自任务详情）",
        ));
    }
    if !issues.is_empty() {
        return ApiError::field_validation(issues).render(&request_id);
    }
    let request = control_service::ReconcileRequest {
        action: action.expect("无 field 问题时 action 必有值"),
        stage_id: stage_id.expect("无 field 问题时 stageId 必有值"),
        remote_task_id: body.remote_task_id,
        acknowledge_matches: body.acknowledge_matches.unwrap_or(false),
        evidence: body.evidence,
        acknowledge_duplicate_risk: body.acknowledge_duplicate_risk.unwrap_or(false),
        limits_tripo_credit_minor: body
            .limits
            .as_ref()
            .and_then(|limits| limits.tripo_credit_minor),
        limits_manual_ai_usd_micros: body
            .limits
            .as_ref()
            .and_then(|limits| limits.manual_ai_usd_micros),
    };

    let report = match control_service::reconcile(
        state.database().pool(),
        state.settings(),
        &job_id,
        expected_revision,
        &request,
        &session.admin_id,
        Timestamp::now(),
    )
    .await
    {
        Ok(report) => report,
        Err(error) => return ApiError::from(error).render(&request_id),
    };
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    match load_job_detail(&mut connection, &report.job).await {
        Ok(detail) => Json(ReconcileResponse {
            data: ReconcileResultDto {
                job: detail,
                action: match report.action {
                    control_service::ReconcileAction::AttachRemoteTask => {
                        ReconcileActionDto::AttachRemoteTask
                    }
                    control_service::ReconcileAction::RecordNoTask => {
                        ReconcileActionDto::RecordNoTask
                    }
                    control_service::ReconcileAction::AuthorizeReplacement => {
                        ReconcileActionDto::AuthorizeReplacement
                    }
                },
                stage_id: report.stage_id.clone(),
                stage_status: report.stage_status.as_str().to_owned(),
                attempt_id: report.attempt_id.clone(),
                notice: report.notice,
            },
        })
        .into_response(),
        Err(error) => error.render(&request_id),
    }
}

// ---------------------------------------------------------------------------
// 组装（列表/详情/控制响应共用）
// ---------------------------------------------------------------------------

/// 列表行。
async fn load_job_summary(
    conn: &mut SqliteConnection,
    job: &Job,
) -> Result<JobSummaryDto, ApiError> {
    let item = repo::items::get(conn, &job.item_id)
        .await
        .map_err(ApiError::from_storage)?
        .ok_or_else(|| ApiError::internal("任务引用的物品不存在（数据完整性异常）"))?;
    let stages = repo::job_stages::list_for_job(conn, &job.id)
        .await
        .map_err(ApiError::from_storage)?;
    let reservations = repo::ledger::list_for_snapshot(conn, &job.snapshot_id)
        .await
        .map_err(ApiError::from_storage)?;
    let draft_id = repo::drafts::get_by_snapshot(conn, &job.snapshot_id)
        .await
        .map_err(ApiError::from_storage)?
        .map(|draft| draft.id);
    Ok(JobSummaryDto {
        id: job.id.clone(),
        item_id: job.item_id.clone(),
        item_name: item.name,
        item_model: item.model,
        status: job.status.as_str().to_owned(),
        revision: job.revision,
        stage_summary: stage_summary(stages.iter().map(|stage| stage.status)),
        reservations: reservations
            .iter()
            .map(ReservationDto::from_entry)
            .collect(),
        draft_id,
        created_at: job.created_at,
        updated_at: job.updated_at,
    })
}

/// 阶段计数摘要（不是线性百分比；口径见 DTO 文档）。
fn stage_summary(statuses: impl Iterator<Item = JobStatus>) -> JobStageSummaryDto {
    let mut summary = JobStageSummaryDto {
        total: 0,
        succeeded: 0,
        active: 0,
        blocked: 0,
        unknown: 0,
        failed: 0,
        cancelled: 0,
    };
    for status in statuses {
        summary.total += 1;
        match status {
            JobStatus::Succeeded => summary.succeeded += 1,
            JobStatus::Queued
            | JobStatus::Running
            | JobStatus::RetryWait
            | JobStatus::WaitingProvider => summary.active += 1,
            JobStatus::NeedsInput => summary.blocked += 1,
            JobStatus::SubmissionUnknown => summary.unknown += 1,
            JobStatus::Failed => summary.failed += 1,
            JobStatus::Cancelled => summary.cancelled += 1,
        }
    }
    summary
}

/// 详情（阶段、尝试、费用、草稿）。
async fn load_job_detail(conn: &mut SqliteConnection, job: &Job) -> Result<JobDetailDto, ApiError> {
    let item = repo::items::get(conn, &job.item_id)
        .await
        .map_err(ApiError::from_storage)?
        .ok_or_else(|| ApiError::internal("任务引用的物品不存在（数据完整性异常）"))?;
    let stages = repo::job_stages::list_for_job(conn, &job.id)
        .await
        .map_err(ApiError::from_storage)?;
    let attempts = repo::attempts::list_for_job(conn, &job.id)
        .await
        .map_err(ApiError::from_storage)?;
    let reservations = repo::ledger::list_for_snapshot(conn, &job.snapshot_id)
        .await
        .map_err(ApiError::from_storage)?;
    let draft_id = repo::drafts::get_by_snapshot(conn, &job.snapshot_id)
        .await
        .map_err(ApiError::from_storage)?
        .map(|draft| draft.id);
    Ok(JobDetailDto {
        id: job.id.clone(),
        item: JobItemDto {
            id: item.id,
            name: item.name,
            model: item.model,
        },
        snapshot_id: job.snapshot_id.clone(),
        status: job.status.as_str().to_owned(),
        revision: job.revision,
        stages: stages
            .iter()
            .map(|stage| stage_dto(stage, job.status, &stages, &reservations))
            .collect(),
        attempts: attempts.iter().map(attempt_dto).collect(),
        reservations: reservations
            .iter()
            .map(ReservationDto::from_entry)
            .collect(),
        draft_id,
        budget_notice: BUDGET_NOTICE.to_owned(),
        created_at: job.created_at,
        updated_at: job.updated_at,
    })
}

/// attempt DTO（对账面板数据）。
///
/// `last_error` 走统一脱敏入口（读取侧兜底）：新写入的 attempt 事实在仓储层已脱敏，
/// 但**历史行**可能仍有修复前落库的临时签名 URL；contracts §1 不允许向用户输出
/// 完整供应商签名 URL，因此这里逐字段替换为摘要标签（不改写源库）。
fn attempt_dto(attempt: &manual_core::domain::ProviderAttempt) -> JobAttemptDto {
    JobAttemptDto {
        id: attempt.id.clone(),
        stage_id: attempt.stage_id.clone(),
        submit_state: attempt.submit_state.as_str().to_owned(),
        remote_task_id: attempt.remote_task_id.clone(),
        response_id: attempt.response_id.clone(),
        started_at: attempt.started_at,
        last_error: attempt
            .last_error
            .as_deref()
            .map(crate::redaction::redact_text_urls),
    }
}

/// 阶段 DTO（`needs_input` 缺项、`knowledgeProduced` 事实与重试准入）。
fn stage_dto(
    stage: &manual_core::domain::JobStage,
    job_status: JobStatus,
    all_stages: &[manual_core::domain::JobStage],
    reservations: &[manual_core::domain::CostLedgerEntry],
) -> JobStageDto {
    // 缺项消息同样走统一脱敏入口（读取侧兜底覆盖历史行；新写入已在仓储层脱敏）。
    // 解析失败**如实记录**（BUG-011：修复前损坏的行曾被静默吞成空列表，
    // stage.status 仍是 needs_input 但界面看不到任何缺项）。
    let needs_input: Vec<JobMissingItemDto> = match stage.needs_input_json.as_ref() {
        Some(value) => match serde_json::from_value::<Vec<JobMissingItemDto>>(value.clone()) {
            Ok(items) => items
                .into_iter()
                .map(|item| JobMissingItemDto {
                    code: item.code,
                    message: crate::redaction::redact_text_urls(&item.message),
                })
                .collect(),
            Err(error) => {
                tracing::warn!(
                    event = "job_detail_needs_input_parse_failed",
                    stageId = %stage.id,
                    stageKind = stage.stage_kind.as_str(),
                    detail = %error,
                    "阶段缺项 JSON 无法解析为列表：详情按空列表展示（源库未被改写；\
                     检查是否有旧规则写入的形态或数据损坏）"
                );
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    // T14 P3-1：展示与恢复同判据——"是否产出正式知识"读批次结果事实里的
    // `producedKnowledge`（与结果资产同事务写入）；非批次阶段为 null。
    let knowledge_produced = if stage.stage_kind == manual_core::domain::StageKind::ManualExtract {
        stage
            .usage_json
            .as_ref()
            .and_then(|usage| usage.get("producedKnowledge"))
            .and_then(serde_json::Value::as_bool)
    } else {
        None
    };
    // T17 / T15 P3①：重试准入与服务端端点同源（`jobs::control::retry_gate`）。
    // 界面据此决定是否渲染重试按钮；被拒时照实显示原因与真实恢复路径。
    let ledger_holds = match control_service::branch_provider(stage.stage_kind) {
        Some(provider) => reservations
            .iter()
            .find(|entry| entry.provider == provider)
            .map(|entry| manual_core::cost::ledger_state_holds_budget(entry.state))
            .unwrap_or(false),
        None => true,
    };
    let gate = control_service::retry_gate(job_status, all_stages, stage, ledger_holds);
    let submission_style = manual_core::jobs::submission_style(stage.stage_kind)
        .map(|style| style.as_str().to_owned());
    JobStageDto {
        id: stage.id.clone(),
        stage_kind: stage.stage_kind.as_str().to_owned(),
        batch_index: stage.batch_index,
        status: stage.status.as_str().to_owned(),
        page_set: stage.page_set.clone(),
        attempt_count: stage.attempt_count,
        poll_count: stage.poll_count,
        next_run_at: stage.next_run_at,
        // 读取侧兜底（同 `usage`）：历史行里可能仍有修复前的临时签名 URL
        // （BUG-009：传输错误曾把 reqwest 的 ` for url (…)` 原样落库）。
        last_error: stage
            .last_error
            .as_deref()
            .map(crate::redaction::redact_text_urls),
        needs_input,
        result_asset_id: stage.result_asset_id.clone(),
        // 脱敏后返回：新写入的阶段事实已只含链接摘要；历史行里可能仍有
        // 修复前落库的临时签名下载地址（T12/T13 的已知留存），这里是读取侧兜底。
        // contracts §1 明确不允许把完整供应商签名 URL 输出给前端；id/计数/状态保留。
        usage: stage.usage_json.as_ref().map(redact_urls),
        knowledge_produced,
        retry: JobStageRetryDto {
            allowed: gate.allowed,
            reason: gate.reason.map(str::to_owned),
            message: gate.message,
        },
        submission_style,
        updated_at: stage.updated_at,
    }
}

/// 脱敏：把 URL 形态的字符串（含临时供应商签名地址）替换为 **sha256 摘要 + host**
/// 的自描述对象（[`crate::redaction`] 的统一规则）。
///
/// 为什么需要：T20/BUG-008 之后新写入的阶段事实已只含摘要，但**历史行**（修复前
/// 落库）里可能仍有带签名的 `output.model_url`；contracts.md §1 明确"不向用户输出……
/// 完整供应商签名 URL"，因此 API 响应逐层替换（读取侧兜底，不改写源库）。
/// 只替换含 `://` 的字符串；id/计数/状态/计费字段原样保留。
fn redact_urls(value: &serde_json::Value) -> serde_json::Value {
    let mut redacted = value.clone();
    crate::redaction::redact_urls_in_json(&mut redacted);
    redacted
}

/// `{data}` + `ETag` 响应（任务详情的 revision 用于 cancel/retry/reconcile 的 If-Match）。
fn job_response(detail: JobDetailDto, status: StatusCode) -> Response {
    let etag = etag_value(detail.revision);
    let mut response = (status, Json(JobDetailResponse { data: detail })).into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&etag) {
        response.headers_mut().insert("etag", value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use manual_core::domain::{JobStage, ProviderAttempt, StageKind, SubmitState};

    /// BUG-009 复现形态的 canary（只出现在测试字符串里）。
    const CANARY: &str = "qa-canary-transport-signature-7d31";

    fn stage_with(
        last_error: Option<&str>,
        needs_input_json: Option<serde_json::Value>,
    ) -> JobStage {
        let now = Timestamp::from_millis(1_700_000_000_000);
        JobStage {
            id: "stage-1".to_owned(),
            job_id: "job-1".to_owned(),
            stage_kind: StageKind::ModelDownload,
            batch_index: 0,
            page_set: None,
            input_hash: "hash-1".to_owned(),
            result_asset_id: None,
            usage_json: None,
            status: JobStatus::RetryWait,
            lease_owner: None,
            lease_epoch: 0,
            lease_until: None,
            next_run_at: None,
            attempt_count: 1,
            poll_count: 0,
            last_error: last_error.map(str::to_owned),
            needs_input_json,
            created_at: now,
            updated_at: now,
        }
    }

    /// 历史行（修复前落库）里的 `lastError`/缺项消息同样不得回显签名 URL；
    /// URL 之后的普通文本必须保留（BUG-010）。
    #[test]
    fn stage_dto_redacts_historical_signed_urls() {
        let stage = stage_with(
            Some(
                "模型下载传输失败（下载可安全重试；不产生供应商费用）：连接失败：\
                 error sending request for url (https://cdn.example.invalid/m.glb?sign=qa-canary-transport-signature-7d31)",
            ),
            Some(serde_json::json!([{
                "code": "download_transport",
                "message": format!("链接过期：https://cdn.example.invalid/m.glb?sign={CANARY}已过期，请按 task-77 重查"),
            }])),
        );
        let dto = stage_dto(
            &stage,
            JobStatus::RetryWait,
            std::slice::from_ref(&stage),
            &[],
        );

        let last_error = dto.last_error.expect("lastError 保留（只替换 URL）");
        assert!(!last_error.contains("://"), "{last_error}");
        assert!(!last_error.contains(CANARY), "{last_error}");
        assert!(
            last_error.contains("host=cdn.example.invalid"),
            "{last_error}"
        );
        assert!(last_error.contains("下载可安全重试"), "{last_error}");
        assert_eq!(dto.id, stage.id, "id 原样保留");
        assert_eq!(dto.attempt_count, 1, "计数原样保留");

        let message = &dto.needs_input[0].message;
        assert!(!message.contains("://"), "{message}");
        assert!(!message.contains(CANARY), "{message}");
        assert!(message.contains("已过期，请按 task-77 重查"), "{message}");
        assert_eq!(dto.needs_input[0].code, "download_transport");
    }

    /// BUG-011（回合 27）读取侧：同列多条缺项中一条含 URL 时，**所有条目**照常展示；
    /// 整串是 URL 的 message 也保持字符串（值类型契约）。
    #[test]
    fn stage_dto_keeps_every_needs_input_item_when_one_contains_url() {
        let stage = stage_with(
            None,
            Some(serde_json::json!([
                {"code": "download_insecure_scheme", "message": format!("模型下载必须使用 HTTPS（实际 http://cdn.example.invalid/m.glb?sign={CANARY}）：拒绝下载")},
                {"code": "retry", "message": "下载可安全重试（task-9）"},
                {"code": "pure_url", "message": format!("https://cdn.example.invalid/m.glb?sign={CANARY}")}
            ])),
        );
        let dto = stage_dto(
            &stage,
            JobStatus::NeedsInput,
            std::slice::from_ref(&stage),
            &[],
        );
        assert_eq!(dto.needs_input.len(), 3, "同列条目不得丢失");
        assert_eq!(dto.needs_input[0].code, "download_insecure_scheme");
        let first = &dto.needs_input[0].message;
        assert!(first.ends_with("）：拒绝下载"), "整句说明保留：{first}");
        assert!(!first.contains("://") && !first.contains(CANARY), "{first}");
        assert_eq!(dto.needs_input[1].message, "下载可安全重试（task-9）");
        let pure = &dto.needs_input[2].message;
        assert!(!pure.contains("://") && !pure.contains(CANARY), "{pure}");
        assert!(pure.contains("host=cdn.example.invalid"), "{pure}");
    }

    /// 历史 attempt 的 `lastError` 同样脱敏；干净文本与 task_id 原样保留。
    #[test]
    fn attempt_dto_redacts_historical_signed_urls() {
        let now = Timestamp::from_millis(1_700_000_000_000);
        let attempt = ProviderAttempt {
            id: "attempt-1".to_owned(),
            job_id: "job-1".to_owned(),
            stage_id: "stage-1".to_owned(),
            request_hash: "hash-1".to_owned(),
            submit_state: SubmitState::Unknown,
            remote_task_id: Some("task-77".to_owned()),
            response_id: None,
            started_at: now,
            last_error: Some(format!(
                "传输失败：error sending request for url (https://cdn.example.invalid/m.glb?sign={CANARY})"
            )),
            created_at: now,
            updated_at: now,
        };
        let dto = attempt_dto(&attempt);
        let last_error = dto.last_error.expect("lastError 保留");
        assert!(!last_error.contains("://"), "{last_error}");
        assert!(!last_error.contains(CANARY), "{last_error}");
        assert_eq!(dto.remote_task_id.as_deref(), Some("task-77"));

        // 干净文本不被改写（脱敏是幂等的最小干预）。
        let clean = ProviderAttempt {
            last_error: Some(
                "模型下载链接已过期（HTTP 403）：将重新查询 task-77（不重新购买）".to_owned(),
            ),
            ..attempt
        };
        assert_eq!(
            attempt_dto(&clean).last_error.as_deref(),
            clean.last_error.as_deref()
        );
    }
}
