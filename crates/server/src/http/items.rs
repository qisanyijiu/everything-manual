//! 物品路由（REQ-010 / AC-016 / AC-017）：创建、分页列表、读取与 PATCH（归档）。
//!
//! T04 曾在此提供 If-Match 的最小载体；T07 补齐业务规则：
//! - **创建**（`POST /items`，201）：服务器生成 UUIDv7 与 `revision=1`；名称与型号必填，
//!   缺失/空白/超长 → 422 + `details.fields`（逐字段）；品牌与变体可选；
//!   同品牌型号**不强制唯一**（不同配置并存，不用唯一约束代替业务判断）。
//! - **列表**（`GET /items`）：`{data, nextCursor}`，默认 20／最多 100；默认只返回未归档，
//!   `archived=true` 只返回已归档；游标绑定过滤条件（见 `http::pagination`）。
//! - **PATCH 清空语义**（ADR-016）：字段缺失 = 保持；`brand`/`variant` 显式 `null` 或空白
//!   = 清空；`name`/`model` 显式 `null` 或空白 = 422；空请求体 = 422。
//!   任何成功的 PATCH 才递增 `revision`；校验失败不改数据、不递增。
//! - **归档**：`PATCH {"archived": true|false}`；归档物品默认列表不可见、单条仍可读，
//!   其 document/photo/资产引用完整保留（物理删除不在 MVP 合同内，删除路由 → 405）。
//! - 并发：`revision` CAS 在 SQL 条件更新内完成，后到者 412 + `details.currentRevision`。

use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing};
use manual_core::domain::Item;
use manual_core::validation::{
    FieldIssue, ItemCreateInput, ItemPatchInput, validate_item_create, validate_item_patch,
};

use crate::storage::repo::items::{self, ArchivedFilter, ItemUpdate};

use super::dto::{ItemCreateRequest, ItemDto, ItemListResponse, ItemPatchRequest, ItemResponse};
use super::error::{ApiError, RequestId};
use super::pagination::{Cursor, parse_list_params, raw_value};
use super::precondition::{etag_value, parse_if_match};
use super::state::AppState;

/// 物品列表游标的作用域（绑定过滤条件，见 `http::pagination`）。
const SCOPE_ACTIVE: &str = "items:active";
const SCOPE_ARCHIVED: &str = "items:archived";

/// 受会话保护的物品路由（由 `router` 挂到 [`super::auth::auth_guard`] 之上）。
///
/// 注意：**没有** `DELETE /items` 或 `DELETE /items/{id}`——MVP 不提供永久删除
/// （归档代替删除；`contracts.md` §3），未注册的方法由 axum 返回 405 + `Allow`。
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/items", routing::get(list_items).post(create_item))
        .route("/items/{id}", routing::get(get_item).patch(patch_item))
}

/// `GET /api/v1/items` —— `{data, nextCursor}`；默认只看未归档。
#[utoipa::path(
    get,
    path = "/api/v1/items",
    tag = "items",
    summary = "物品列表（游标分页，默认排除归档）",
    description = "稳定排序 (createdAt DESC, id DESC)。`archived` 缺省或 false 只返回未归档物品，\
                   `archived=true` 只返回已归档物品；nextCursor 是**不透明**字符串（形如 \
                   `v1:items:active:<millis>:<id>`），已绑定产生它的过滤条件——切换过滤条件时必须\
                   从头分页，复用旧游标会得到 422。未知/重复/非法查询参数 → 422 字段级明细。",
    params(
        ("limit" = Option<u32>, Query, description = "每页条数，默认 20，最大 100"),
        ("cursor" = Option<String>, Query, description = "上一页返回的 nextCursor（原样回传）"),
        ("archived" = Option<bool>, Query, description = "true 只看已归档；缺省/false 只看未归档"),
    ),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "物品列表", body = ItemListResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 422, description = "查询参数非法（details.fields）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn list_items(
    State(state): State<AppState>,
    request_id: RequestId,
    Query(params): Query<Vec<(String, String)>>,
) -> Response {
    let mut issues: Vec<FieldIssue> = Vec::new();
    let parsed = match parse_list_params(&params, &["archived"]) {
        Ok(parsed) => Some(parsed),
        Err(mut found) => {
            issues.append(&mut found);
            None
        }
    };
    let archived = match raw_value(&params, "archived") {
        None => false,
        Some("true") => true,
        Some("false") => false,
        Some(other) => {
            issues.push(FieldIssue::new(
                "archived",
                format!("只接受 true/false（收到 {other:?}）"),
            ));
            false
        }
    };
    if !issues.is_empty() {
        return ApiError::field_validation(issues).render(&request_id);
    }
    let parsed = parsed.expect("无字段问题时参数一定已解析");

    let scope = if archived {
        SCOPE_ARCHIVED
    } else {
        SCOPE_ACTIVE
    };
    let cursor = match parsed.cursor.as_deref() {
        None => None,
        Some(value) => match Cursor::parse(value, scope) {
            Ok(cursor) => Some(cursor.into_tuple()),
            Err(issue) => return ApiError::field_validation(vec![issue]).render(&request_id),
        },
    };

    let mut connection = match state.database().pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::error!(error = %error, "获取数据库连接失败");
            return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
        }
    };
    let filter = if archived {
        ArchivedFilter::Archived
    } else {
        ArchivedFilter::Active
    };
    // 多取一条判断是否还有下一页（不伪造 nextCursor）。
    let mut rows = match items::list_page(&mut connection, filter, cursor, parsed.limit + 1).await {
        Ok(rows) => rows,
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };
    let has_more = rows.len() as u32 > parsed.limit;
    rows.truncate(parsed.limit as usize);
    let next_cursor = if has_more {
        rows.last()
            .map(|item| Cursor::encode(scope, item.created_at.as_millis(), &item.id))
    } else {
        None
    };
    let data: Vec<ItemDto> = rows.into_iter().map(ItemDto::from).collect();
    Json(ItemListResponse { data, next_cursor }).into_response()
}

/// `POST /api/v1/items` —— 创建物品（201 + `ETag: "r1"`）。
#[utoipa::path(
    post,
    path = "/api/v1/items",
    tag = "items",
    summary = "创建物品",
    description = "服务器生成 UUIDv7 id 与整数 revision（从 1 起）。名称与型号必填：缺失、空白或\
                   超长（name/model ≤200、brand ≤100、variant ≤200 字符）→ 422 + `details.fields`。\
                   同品牌型号不强制唯一（允许不同配置并存）。未知字段 → 422。",
    request_body = ItemCreateRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 201, description = "已创建（响应带 ETag: \"r1\"）", body = ItemResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 422, description = "字段级校验失败（details.fields）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn create_item(
    State(state): State<AppState>,
    request_id: RequestId,
    super::body::JsonBody(body): super::body::JsonBody<ItemCreateRequest>,
) -> Response {
    let fields = match validate_item_create(ItemCreateInput {
        name: body.name,
        brand: body.brand,
        model: body.model,
        variant: body.variant,
    }) {
        Ok(fields) => fields,
        Err(issues) => return ApiError::field_validation(issues).render(&request_id),
    };

    let mut connection = match state.database().pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::error!(error = %error, "获取数据库连接失败");
            return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
        }
    };
    match items::create(
        &mut connection,
        items::NewItem {
            name: fields.name,
            brand: fields.brand,
            model: fields.model,
            variant: fields.variant,
        },
    )
    .await
    {
        Ok(item) => {
            tracing::info!(requestId = %request_id, itemId = %item.id, "物品已创建");
            item_response(item, StatusCode::CREATED)
        }
        Err(error) => ApiError::from_storage(error).render(&request_id),
    }
}

/// `GET /api/v1/items/{id}` —— 带 `ETag`；不存在 404；归档物品仍可读。
#[utoipa::path(
    get,
    path = "/api/v1/items/{id}",
    tag = "items",
    summary = "读取物品",
    description = "返回物品并带 ETag: \"r<revision>\"，供后续 PATCH 的 If-Match 使用。\
                   归档物品同样可读（归档不删除数据）。",
    params(("id" = String, Path, description = "物品 ID（UUIDv7）")),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "物品", body = ItemResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品不存在", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn get_item(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(id): Path<String>,
) -> Response {
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    match items::get(&mut connection, &id).await {
        Ok(Some(item)) => with_etag(item),
        Ok(None) => ApiError::not_found(format!("item 不存在：{id}")).render(&request_id),
        Err(error) => ApiError::from_storage(error).render(&request_id),
    }
}

/// `PATCH /api/v1/items/{id}` —— 需要 `If-Match`；缺 428、过期 412。
#[utoipa::path(
    patch,
    path = "/api/v1/items/{id}",
    tag = "items",
    summary = "更新物品（If-Match 乐观锁；归档用 archived 字段）",
    description = "必须携带 If-Match（形如 \"r7\"，来自 GET/创建响应的 ETag）：缺失 428、revision \
                   过期 412（details.currentRevision）。字段语义：缺失 = 保持原值；brand/variant 显式 \
                   null（或空白字符串）= 清空；name/model 显式 null 或空白 = 422；空请求体 = 422。\
                   archived=true 归档（记录归档时间）、false 取消归档。校验失败不修改数据、不递增 \
                   revision。MVP 不提供永久删除（DELETE → 405）。",
    params(("id" = String, Path, description = "物品 ID（UUIDv7）")),
    request_body = ItemPatchRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "更新后的物品（带新 ETag）", body = ItemResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品不存在", body = super::dto::ApiErrorResponse),
        (status = 412, description = "revision 过期（details.currentRevision）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "字段级校验失败或 If-Match 非法（details.fields）", body = super::dto::ApiErrorResponse),
        (status = 428, description = "缺少 If-Match", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn patch_item(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    super::body::JsonBody(patch): super::body::JsonBody<ItemPatchRequest>,
) -> Response {
    let expected_revision = match parse_if_match(&headers) {
        Ok(revision) => revision,
        Err(error) => return error.render(&request_id),
    };
    // 先做纯规则校验（不触碰数据库）：失败即 422，不改数据、不递增 revision。
    let patch = match validate_item_patch(ItemPatchInput {
        name: patch.name,
        brand: patch.brand,
        model: patch.model,
        variant: patch.variant,
        archived: patch.archived,
    }) {
        Ok(patch) => patch,
        Err(issues) => return ApiError::field_validation(issues).render(&request_id),
    };

    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let current = match items::get(&mut connection, &id).await {
        Ok(Some(item)) => item,
        Ok(None) => return ApiError::not_found(format!("item 不存在：{id}")).render(&request_id),
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };
    let currently_archived = current.is_archived();
    let update = ItemUpdate {
        name: patch.name.unwrap_or(current.name),
        // Some(None) = 清空；Some(Some(v)) = 新值；None = 保持（T04 的静默保留问题在此修复）。
        brand: patch.brand.unwrap_or(current.brand),
        model: patch.model.unwrap_or(current.model),
        variant: patch.variant.unwrap_or(current.variant),
        archived: patch.archived.unwrap_or(currently_archived),
    };

    match items::update(&mut connection, &id, expected_revision, update).await {
        Ok(item) => {
            tracing::info!(
                requestId = %request_id,
                itemId = %item.id,
                revision = item.revision,
                archived = item.is_archived(),
                "物品已更新"
            );
            with_etag(item)
        }
        Err(error) => ApiError::from_storage(error).render(&request_id),
    }
}

/// 获取数据库连接；失败时返回 [`ApiError`]（调用方用本次请求的 requestId 渲染）。
async fn acquire(state: &AppState) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, ApiError> {
    state.database().pool().acquire().await.map_err(|error| {
        tracing::error!(error = %error, "获取数据库连接失败");
        ApiError::internal("服务器内部错误：数据库暂不可用")
    })
}

/// `{data}` + `ETag` 响应（contracts.md §1：可编辑聚合根的 GET 返回 `"r<n>"`）。
fn item_response(item: Item, status: StatusCode) -> Response {
    let etag = etag_value(item.revision);
    let mut response = (
        status,
        Json(ItemResponse {
            data: ItemDto::from(item),
        }),
    )
        .into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

fn with_etag(item: Item) -> Response {
    item_response(item, StatusCode::OK)
}
