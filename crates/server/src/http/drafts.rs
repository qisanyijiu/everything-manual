//! 草稿路由（T15 读取 + T19 受限字段 PATCH 与发布；contracts.md §2/§3）。
//!
//! - `GET /items/{id}/drafts/{draftId}`：版本化知识、模型、复核状态，带
//!   `ETag: "r<revision>"`；
//! - `PATCH /items/{id}/drafts/{draftId}`：`If-Match` 乐观锁（缺 428、过期 412），
//!   受限字段（status/hotspots/stepPoses/entities/modelReview），逐字段校验
//!   （引用存在、数值有限、旧模型 sha 拒绝、供应商快照只读）；未知字段 → 422；
//! - `POST /items/{id}/drafts/{draftId}/publish`：If-Match + `Idempotency-Key`，
//!   发布不变量全满足 → 201 不可变 release；否则 422 + `details.issues[]`。
//!
//! 语义固定：`needs_review` 只表示"可复核草稿已产出"，与发布无关；
//! **发布只发生在显式 publish 请求里**（无自动发布路径，ADR-005）。

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, routing};
use manual_core::domain::ManualDraft;
use manual_core::timestamps::Timestamp;
use manual_core::validation::FieldIssue;

use crate::drafts::service as drafts_service;
use crate::releases::service as releases_service;

use super::auth::SessionContext;
use super::body::JsonBody;
use super::dto::{
    DRAFT_NOTICES, DraftDto, DraftMissingItemDto, DraftPatchRequest, DraftResponse, DraftStatusDto,
    ReleaseDto, ReleaseResponse,
};
use super::error::{ApiError, RequestId};
use super::precondition::{etag_value, parse_if_match};
use super::state::AppState;

/// 受会话保护的草稿路由。
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/items/{id}/drafts/{draft_id}",
            routing::get(get_draft).patch(patch_draft),
        )
        .route(
            "/items/{id}/drafts/{draft_id}/publish",
            routing::post(publish_draft),
        )
}

/// `GET /api/v1/items/{id}/drafts/{draftId}`。
#[utoipa::path(
    get,
    path = "/api/v1/items/{id}/drafts/{draftId}",
    tag = "drafts",
    summary = "读取版本化草稿（带 ETag）",
    description = "返回草稿的知识聚合（含 `completeness`/`missing[]`：部分成功可展示）、\
                   模型版本引用与复核状态，带 `ETag: \"r<revision>\"` 供 PATCH 的 If-Match。\
                   `status = needs_review` 只表示可复核草稿已产出——生成完成不等于已发布，\
                   不存在自动发布路径。",
    params(
        ("id" = String, Path, description = "物品 ID（UUIDv7）"),
        ("draftId" = String, Path, description = "草稿 ID（UUIDv7）"),
    ),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "草稿（带 ETag）", body = DraftResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "草稿不存在或不属于该物品", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn get_draft(
    State(state): State<AppState>,
    request_id: RequestId,
    Path((item_id, draft_id)): Path<(String, String)>,
) -> Response {
    let mut connection = match super::estimates::acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    match drafts_service::read_draft(&mut connection, &item_id, &draft_id).await {
        Ok(draft) => draft_response(draft, StatusCode::OK),
        Err(error) => ApiError::from(error).render(&request_id),
    }
}

/// `PATCH /api/v1/items/{id}/drafts/{draftId}`（If-Match + 受限字段集）。
#[utoipa::path(
    patch,
    path = "/api/v1/items/{id}/drafts/{draftId}",
    tag = "drafts",
    summary = "复核写入：状态、热点校准、步骤视角、知识确认与修订、modelReview（If-Match）",
    description = "必须携带 If-Match（来自 GET 的 ETag）：缺失 428、revision 过期 412。\
                   受限字段：`status`（needs_review ↔ ready）、`hotspots`（新建/重新绑定/解绑；\
                   人工直接拾取 = upsert confirmed + anchor；unbound 时 anchor 必须为 null，\
                   不得用 [0,0,0] 占位；candidate/confirmed 的 anchor 必须与当前模型 \
                   revision+sha 完全一致，旧 sha 一律 422）、`stepPoses`（CameraPose 或 null \
                   清除；数值有限、fov 合理范围）、`entities`（confirmed/needs_review 切换、\
                   人工修订 userEdited、仅文本条目 textOnly）、`modelReview`（loaded 与 \
                   userConfirmed 两个用户声明，checkedAt 由服务器赋值）。\
                   **不能修改供应商事实快照**：knowledge 里的 Part/Step/Evidence 只读，\
                   人工修订写入独立覆盖层并保留原文本与出处；未知字段 422。\
                   空请求体 422；无实际变化的重复提交按幂等返回（不递增 revision，stale 仍 412）。\
                   动作写入 audit_events。",
    params(
        ("id" = String, Path, description = "物品 ID（UUIDv7）"),
        ("draftId" = String, Path, description = "草稿 ID（UUIDv7）"),
    ),
    request_body = DraftPatchRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "更新后的草稿（带新 ETag）", body = DraftResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF/Origin 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "草稿不存在或不属于该物品", body = super::dto::ApiErrorResponse),
        (status = 412, description = "revision 过期（details.currentRevision）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "字段级校验失败（details.fields；含未知字段/引用不存在/旧 sha 拒绝）", body = super::dto::ApiErrorResponse),
        (status = 428, description = "缺少 If-Match", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn patch_draft(
    State(state): State<AppState>,
    request_id: RequestId,
    axum::Extension(session): axum::Extension<SessionContext>,
    Path((item_id, draft_id)): Path<(String, String)>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<DraftPatchRequest>,
) -> Response {
    let expected_revision = match parse_if_match(&headers) {
        Ok(revision) => revision,
        Err(error) => return error.render(&request_id),
    };
    let patch = body.to_domain();
    if patch.is_empty() {
        return ApiError::field_validation(vec![FieldIssue::new(
            "body",
            "空请求体：请提供要更新的字段（status / hotspots / stepPoses / entities / modelReview）",
        )])
        .render(&request_id);
    }
    match drafts_service::patch_draft(
        state.database().pool(),
        &item_id,
        &draft_id,
        expected_revision,
        &patch,
        &session.admin_id,
        Timestamp::now(),
    )
    .await
    {
        Ok(draft) => {
            tracing::info!(
                requestId = %request_id,
                draftId = %draft.id,
                revision = draft.revision,
                status = draft.status.as_str(),
                "草稿复核内容已更新（人工事实修改/校准；仍不触发发布）"
            );
            draft_response(draft, StatusCode::OK)
        }
        Err(error) => ApiError::from(error).render(&request_id),
    }
}

/// `POST /api/v1/items/{id}/drafts/{draftId}/publish`（If-Match + Idempotency-Key）。
#[utoipa::path(
    post,
    path = "/api/v1/items/{id}/drafts/{draftId}/publish",
    tag = "drafts",
    summary = "发布不可变说明书版本（If-Match + Idempotency-Key）",
    description = "发布事务在**显式请求**中执行（不存在自动发布路径）。必须携带 If-Match \
                   （缺失 428、过期 412）与 `Idempotency-Key`（缺失 422；同 key 同 body 重放\
                   返回同一 release 并标记 `x-idempotent-replay: true`；同 key 不同 body 409）。\
                   发布不变量全部满足才返回 201：必需知识已确认或有明确人工修订记录、引用页\
                   存在、选中模型 validated 且 modelReview.loaded/userConfirmed 均 true 且 \
                   revision/hash 匹配、每个要发布的交互部件至少一个 confirmed 热点且 hash 匹配、\
                   步骤引用全部存在、无 stale/candidate 热点冒充 confirmed。不满足返回 422 且 \
                   `details.issues[]` 逐条列出；「仅文本条目」部件保留并明显标识，不为发布自动隐藏。\
                   release 与 manifest 不可变（数据库触发器拒绝修改），发布只写本地数据——\
                   不产生费用、不调用任何外部服务。",
    params(
        ("id" = String, Path, description = "物品 ID（UUIDv7）"),
        ("draftId" = String, Path, description = "草稿 ID（UUIDv7）"),
    ),
    security(("sessionCookie" = [])),
    responses(
        (status = 201, description = "已发布（不可变 release；重放同一 release）", body = ReleaseResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF/Origin 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "草稿不存在或不属于该物品", body = super::dto::ApiErrorResponse),
        (status = 409, description = "同 key 不同 body（IDEMPOTENCY_CONFLICT）", body = super::dto::ApiErrorResponse),
        (status = 412, description = "revision 过期/并发修改（details.currentRevision）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "缺 Idempotency-Key（details.fields）或发布不变量不满足（details.issues）", body = super::dto::ApiErrorResponse),
        (status = 428, description = "缺少 If-Match", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn publish_draft(
    State(state): State<AppState>,
    request_id: RequestId,
    axum::Extension(session): axum::Extension<SessionContext>,
    Path((item_id, draft_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let expected_revision = match parse_if_match(&headers) {
        Ok(revision) => revision,
        Err(error) => return error.render(&request_id),
    };
    let idempotency_key = headers
        .get(super::jobs::IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    // 幂等的 body 就是"发布这个草稿的这个 revision"（不变的规范形状）。
    let body_hash = manual_core::generation::sha256_hex(
        serde_json::json!({ "draftId": draft_id, "draftRevision": expected_revision })
            .to_string()
            .as_bytes(),
    );
    let data_dir = state.settings().data_dir.clone();
    match releases_service::publish_draft(
        state.database().pool(),
        &data_dir,
        &session.admin_id,
        &item_id,
        &draft_id,
        expected_revision,
        &idempotency_key,
        &body_hash,
        Timestamp::now(),
    )
    .await
    {
        Ok(outcome) => {
            // 不返回 release 的 ETag：release 不可变、没有"版本比较"语义；
            // 客户端需要的草稿新 revision 在 `draftRevisionAfterPublish` 里。
            let mut response = (
                StatusCode::CREATED,
                Json(ReleaseResponse {
                    data: ReleaseDto::from(
                        &outcome.release,
                        Some(outcome.draft_revision_after_publish),
                    ),
                }),
            )
                .into_response();
            if outcome.replayed {
                response
                    .headers_mut()
                    .insert("x-idempotent-replay", "true".parse().expect("静态头值"));
            }
            response
        }
        Err(error) => ApiError::from(error).render(&request_id),
    }
}

/// 草稿 → 响应（`{data}` + `ETag`）。
fn draft_response(draft: ManualDraft, status: StatusCode) -> Response {
    let etag = etag_value(draft.revision);
    let data = draft_dto(&draft);
    let mut response = (status, Json(DraftResponse { data })).into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&etag) {
        response.headers_mut().insert("etag", value);
    }
    response
}

/// 草稿 DTO：缺项从知识外壳的 `missing[]` 投影（与组装写入同源）。
fn draft_dto(draft: &ManualDraft) -> DraftDto {
    let knowledge = draft.knowledge_json.clone();
    let completeness = knowledge
        .get("completeness")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("partial")
        .to_owned();
    let missing: Vec<DraftMissingItemDto> = knowledge
        .get("missing")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| DraftMissingItemDto {
                    code: item
                        .get("code")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned(),
                    message: item
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    DraftDto {
        id: draft.id.clone(),
        item_id: draft.item_id.clone(),
        snapshot_id: draft.snapshot_id.clone(),
        model_revision_id: draft.model_revision_id.clone(),
        revision: draft.revision,
        status: DraftStatusDto::from_domain(draft.status),
        completeness,
        missing,
        knowledge,
        review: draft.review_json.clone(),
        notices: DRAFT_NOTICES
            .iter()
            .map(|notice| (*notice).to_owned())
            .collect(),
        created_at: draft.created_at,
        updated_at: draft.updated_at,
    }
}
