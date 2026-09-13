//! 报价与确认路由（T11 / REQ-020、REQ-021；contracts.md §3/§4）。
//!
//! 三个端点：
//! - `POST /items/{id}/estimates`（201）：校验前置条件并生成报价快照。
//!   **只计算计划**：不调用任何生成服务、不写费用记录；缺价格配置 409
//!   `PRICE_CATALOG_MISSING`、缺 Provider 409 `PROVIDER_NOT_CONFIGURED`、
//!   前置缺项 422 + `details.items` 逐条列出。
//! - `GET /items/{id}/estimates/{quoteId}`（200）：回读报价（确认页刷新后不丢信息；
//!   含到期时间与确认/消费状态）。
//! - `POST /items/{id}/estimates/{quoteId}/confirm`（200）：记录"云端发送确认"
//!   （REQ-021）。**不存在默认勾选**：只有该显式调用才把报价标记为已确认；
//!   确认动作写 `audit_events`；重复确认幂等（返回首次确认时间与范围）。

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, routing};
use manual_core::timestamps::Timestamp;

use crate::generation::estimate as estimate_service;

use super::auth::SessionContext;
use super::body::JsonBody;
use super::dto::{ConfirmationDto, ConfirmationResponse, EstimateRequest, QuoteResponse};
use super::error::{ApiError, RequestId};
use super::state::AppState;

/// 受会话保护的报价路由（挂在 `/api/v1` 下，由 auth_guard 统一保护）。
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/items/{id}/estimates", routing::post(create_estimate))
        .route(
            "/items/{id}/estimates/{quote_id}",
            routing::get(get_estimate),
        )
        .route(
            "/items/{id}/estimates/{quote_id}/confirm",
            routing::post(confirm_estimate),
        )
}

/// `POST /api/v1/items/{id}/estimates`。
#[utoipa::path(
    post,
    path = "/api/v1/items/{id}/estimates",
    tag = "generation",
    summary = "生成前报价（只计算计划，不调用生成服务）",
    description = "校验：名称/型号非空、preparation ready、至少 front + left/back/right 之一、\
                   图片同物品且每视图唯一、Provider 与价格配置存在。\
                   返回分列金额（Tripo creditMinor / Manual AI usdMicros，不相加）、\
                   保守上界、价格版本与快照日期、发送范围（确认页数据）、expiresAt（默认 10 分钟）。\
                   **不调用任何生成服务、不产生费用记录**。缺价格 → 409 `PRICE_CATALOG_MISSING`；\
                   缺 Provider → 409 `PROVIDER_NOT_CONFIGURED`；前置缺项 → 422 `details.items`。",
    params(("id" = String, Path, description = "物品 ID（UUIDv7）")),
    request_body = EstimateRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 201, description = "报价已生成（绑定输入指纹，可在有效期内确认并提交）", body = QuoteResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品/准备/照片不存在或不属于该物品", body = super::dto::ApiErrorResponse),
        (status = 409, description = "Provider 或价格配置缺失（PROVIDER_NOT_CONFIGURED / PRICE_CATALOG_MISSING）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "字段级或前置条件缺项（details.fields / details.items）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn create_estimate(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(item_id): Path<String>,
    JsonBody(body): JsonBody<EstimateRequest>,
) -> Response {
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    match estimate_service::create_estimate(
        state.settings(),
        &mut connection,
        &item_id,
        &body,
        Timestamp::now(),
    )
    .await
    {
        Ok(record) => match estimate_service::quote_payload(&record).await {
            Ok(payload) => {
                tracing::info!(
                    requestId = %request_id,
                    itemId = %item_id,
                    quoteId = %record.id,
                    pageCount = record.page_count,
                    "报价已生成（只计算计划，不调用生成服务）"
                );
                (StatusCode::CREATED, Json(QuoteResponse { data: payload })).into_response()
            }
            Err(error) => ApiError::from(error).render(&request_id),
        },
        Err(error) => ApiError::from(error).render(&request_id),
    }
}

/// `GET /api/v1/items/{id}/estimates/{quoteId}`。
#[utoipa::path(
    get,
    path = "/api/v1/items/{id}/estimates/{quoteId}",
    tag = "generation",
    summary = "回读报价（确认页刷新后不丢信息）",
    description = "返回报价载荷（含分列金额、保守上界、发送范围、expiresAt 与确认/消费状态）。\
                   过期报价仍可读（界面据此提示重新报价）；提交任务时服务端会再次校验。",
    params(
        ("id" = String, Path, description = "物品 ID（UUIDv7）"),
        ("quoteId" = String, Path, description = "报价 ID（UUIDv7）"),
    ),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "报价载荷", body = QuoteResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品或报价不存在（含跨物品）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn get_estimate(
    State(state): State<AppState>,
    request_id: RequestId,
    Path((item_id, quote_id)): Path<(String, String)>,
) -> Response {
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let record = match crate::storage::repo::quotes::get(&mut connection, &quote_id).await {
        Ok(Some(record)) if record.item_id == item_id => record,
        Ok(_) => {
            return ApiError::not_found(format!("报价不存在：{quote_id}")).render(&request_id);
        }
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };
    match estimate_service::quote_payload(&record).await {
        Ok(mut payload) => {
            // BUG-004：`quote_json` 是创建时冻结的载荷，`confirmed_at`/`consumed_at`/
            // `consumed_job_id` 在其中恒为 null（报价只能先创建、后确认/消费）。
            // 回读（确认页刷新路径）必须以持久层当前列覆盖这三个字段，否则界面会
            // 显示与 DB 事实相反的状态（UI-024 的"已确认发送范围（时间）"、UI-026
            // 的"该操作已存在一个任务"链接）。冻结输入部分（金额、上界、价格版本、
            // expiresAt、发送范围）仍只来自 `quote_json`；本函数只读不写，不回写
            // `quote_json`，报价是否仍可用由确认/提交时的服务端校验决定。
            payload.confirmed_at = record.confirmed_at;
            payload.consumed_at = record.consumed_at;
            payload.consumed_job_id = record.consumed_job_id;
            Json(QuoteResponse { data: payload }).into_response()
        }
        Err(error) => ApiError::from(error).render(&request_id),
    }
}

/// `POST /api/v1/items/{id}/estimates/{quoteId}/confirm`。
#[utoipa::path(
    post,
    path = "/api/v1/items/{id}/estimates/{quoteId}/confirm",
    tag = "generation",
    summary = "确认把选定资料发送给云端（REQ-021）",
    description = "显式确认后才允许提交任务；**不存在默认勾选**。确认动作写入 audit_events\
                   （含发送给 Tripo 的视图、发送给说明书 AI 的页范围/页图与型号文本、模型名、\
                   价格版本与预算上界）。重复确认幂等：返回首次确认时间与范围，不覆盖。\
                   过期报价不允许确认（422 `quoteExpired`）。本端点不产生任何费用记录。",
    params(
        ("id" = String, Path, description = "物品 ID（UUIDv7）"),
        ("quoteId" = String, Path, description = "报价 ID（UUIDv7）"),
    ),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "确认已记录（首次或幂等读回）", body = ConfirmationResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品或报价不存在（含跨物品）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "报价已过期（details.reason=quoteExpired）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn confirm_estimate(
    State(state): State<AppState>,
    request_id: RequestId,
    axum::Extension(session): axum::Extension<SessionContext>,
    Path((item_id, quote_id)): Path<(String, String)>,
) -> Response {
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    match estimate_service::confirm_quote(
        &mut connection,
        &item_id,
        &quote_id,
        &session.admin_id,
        Timestamp::now(),
    )
    .await
    {
        Ok(result) => {
            let ConfirmationDto { quote_id, .. } = result.confirmation.clone();
            tracing::info!(
                requestId = %request_id,
                quoteId = %quote_id,
                alreadyConfirmed = result.already_confirmed,
                "云端发送确认已记录（audit_events）"
            );
            Json(ConfirmationResponse {
                data: result.confirmation,
            })
            .into_response()
        }
        Err(error) => ApiError::from(error).render(&request_id),
    }
}

/// 获取数据库连接；失败时返回 [`ApiError`]（调用方用本次请求的 requestId 渲染）。
pub(crate) async fn acquire(
    state: &AppState,
) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, ApiError> {
    state.database().pool().acquire().await.map_err(|error| {
        tracing::error!(error = %error, "获取数据库连接失败");
        ApiError::internal("服务器内部错误：数据库暂不可用")
    })
}
