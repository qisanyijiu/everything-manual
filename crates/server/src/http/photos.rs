//! 照片与视图路由（REQ-013 / AC-021）：
//! `POST/GET /items/{id}/photos`、`GET/PATCH /items/{id}/photos/{photoId}`。
//!
//! 规则（QA 按此复核）：
//! - `assetId` 必须是**同一物品**的照片资产（`find_for_item` 是唯一归属入口）：
//!   不存在或跨物品 → 404（不泄露存在性）；`purpose=photo` 且 JPEG/PNG 才可绑定 → 否则 422；
//! - `view ∈ {front,left,back,right,detail}`，非法值 → 422 `details.fields[view]`；
//! - **同一物品每视图最多一张**（第二张被拒，不静默覆盖）：服务层在事务内先给出
//!   422 `details.reason=viewOccupied` + `existingPhotoId`（UI 文案"请先移除或改选"），
//!   并发窗口由 `photos_item_view_unique`（迁移 0003）兜底，同样映射为该 422；
//! - PATCH 需 `If-Match`（缺 428、过期 412 + `details.currentRevision`），可改视图或换资产；
//! - `detail` 只用于理解与核对：GET 列表返回它，但**多视图集合**
//!   （`repo::photos::list_multiview_for_item`，T12 构造 Tripo 请求体时使用）不含它。

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing};
use manual_core::domain::{AssetPurpose, BlobStorageState, Photo, PhotoView};
use manual_core::validation::{BODY_FIELD, FieldIssue, validate_photo_view};

use crate::storage::repo::{assets as assets_repo, blobs as blobs_repo, items, photos};

use super::dto::{
    PhotoCreateRequest, PhotoDto, PhotoListResponse, PhotoPatchRequest, PhotoResponse,
};
use super::error::{ApiError, RequestId};
use super::precondition::{etag_value, parse_if_match};
use super::state::AppState;

/// 受会话保护的照片路由。
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/items/{id}/photos",
            routing::get(list_photos).post(create_photo),
        )
        .route(
            "/items/{id}/photos/{photoId}",
            routing::get(get_photo).patch(patch_photo),
        )
}

/// `POST /api/v1/items/{id}/photos` —— 为某视图绑定一张照片。
#[utoipa::path(
    post,
    path = "/api/v1/items/{id}/photos",
    tag = "photos",
    summary = "添加视图照片（同一视图最多一张）",
    description = "assetId 必须属于同一物品且为照片（JPEG/PNG）；view ∈ front/left/back/right/detail。\
                   同一物品同一视图已有照片时返回 422（details.reason=viewOccupied + existingPhotoId），\
                   不静默覆盖——换视图请 PATCH 已有照片。detail 为特写，不进入多视图请求体。",
    params(("id" = String, Path, description = "物品 ID（UUIDv7）")),
    request_body = PhotoCreateRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 201, description = "已添加（响应带 ETag: \"r1\"）", body = PhotoResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品或资产不存在/不属该物品", body = super::dto::ApiErrorResponse),
        (status = 422, description = "视图非法、资产类型不符或视图已被占用（details.fields / details.reason）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn create_photo(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(item_id): Path<String>,
    super::body::JsonBody(body): super::body::JsonBody<PhotoCreateRequest>,
) -> Response {
    let view = match validate_photo_view(body.view.as_deref()) {
        Ok(view) => view,
        Err(issue) => return ApiError::field_validation(vec![issue]).render(&request_id),
    };

    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    if let Err(error) = ensure_item(&mut connection, &item_id).await {
        return error.render(&request_id);
    }
    let asset = match check_photo_asset(&mut connection, &item_id, &body.asset_id).await {
        Ok(asset) => asset,
        Err(error) => return error.render(&request_id),
    };

    // 占用检查与插入在同一短事务内（服务层给出可读错误；唯一索引兜底并发）。
    // 事务借用连接：提交/回滚后连接仍可继续查询（失败分支要在同一连接上查占用者）。
    // `BEGIN IMMEDIATE`（BUG-006）：本事务先读（`view_occupant`）后写（INSERT），
    // deferred 事务的读→写升级遇活跃写者会**立即** SQLITE_BUSY（busy_timeout 不等待）
    // → 500；先取写锁则由 busy_timeout 正常等待（见 `storage::tx` 模块文档）。
    let mut transaction = match crate::storage::begin_write(&mut connection).await {
        Ok(transaction) => transaction,
        Err(error) => {
            tracing::error!(error = %error, "开启事务失败");
            return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
        }
    };
    match photos::view_occupant(&mut transaction, &item_id, view, None).await {
        Ok(Some(existing)) => {
            return view_occupied(&request_id, view, &existing);
        }
        Ok(None) => {}
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    }
    let created = photos::create(
        &mut transaction,
        photos::NewPhoto {
            item_id: item_id.clone(),
            asset_id: asset.id.clone(),
            view,
        },
    )
    .await;
    match created {
        Ok(photo) => {
            if let Err(error) = transaction.commit().await {
                tracing::error!(error = %error, "提交照片事务失败");
                return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
            }
            tracing::info!(
                requestId = %request_id,
                itemId = %item_id,
                photoId = %photo.id,
                view = view.as_str(),
                "视图照片已添加"
            );
            photo_response(photo, StatusCode::CREATED)
        }
        Err(crate::storage::StorageError::UniqueViolation { .. }) => {
            // 并发窗口：索引拒绝。回滚事务后查询占用者，给出同样的可读错误。
            drop(transaction);
            let occupant = photos::view_occupant(&mut connection, &item_id, view, None)
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
            view_occupied(&request_id, view, &occupant)
        }
        Err(error) => ApiError::from_storage(error).render(&request_id),
    }
}

/// `GET /api/v1/items/{id}/photos` —— 全部视图照片（含 detail，固定槽位顺序）。
#[utoipa::path(
    get,
    path = "/api/v1/items/{id}/photos",
    tag = "photos",
    summary = "视图照片列表（含 detail；每视图最多一张，无分页）",
    description = "按槽位顺序 front→left→back→right→detail 返回。集合被视图唯一性上界为 5 条，\
                   因此 nextCursor 恒为 null（不是截断），也不支持任何查询参数（分页参数会被 422 拒绝，\
                   而不是静默忽略）。detail 在此可见，但不属于多视图集合。",
    params(("id" = String, Path, description = "物品 ID（UUIDv7）")),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "照片列表", body = PhotoListResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品不存在", body = super::dto::ApiErrorResponse),
        (status = 422, description = "携带了不支持的查询参数（details.fields）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn list_photos(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(item_id): Path<String>,
    Query(params): Query<Vec<(String, String)>>,
) -> Response {
    // 该集合被视图唯一性上界为 5 条：分页/过滤参数没有语义，显式拒绝而不是静默忽略。
    if let Some((name, _)) = params.first() {
        return ApiError::field_validation(vec![FieldIssue::new(
            name,
            "该集合上界为 5 条（每视图最多一张），不支持分页或过滤参数",
        )])
        .render(&request_id);
    }

    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    if let Err(error) = ensure_item(&mut connection, &item_id).await {
        return error.render(&request_id);
    }
    match photos::list_for_item(&mut connection, &item_id).await {
        Ok(rows) => Json(PhotoListResponse {
            data: rows.into_iter().map(PhotoDto::from).collect(),
            next_cursor: None,
        })
        .into_response(),
        Err(error) => ApiError::from_storage(error).render(&request_id),
    }
}

/// `GET /api/v1/items/{id}/photos/{photoId}` —— 单张照片 + `ETag`（供 PATCH 取 If-Match）。
#[utoipa::path(
    get,
    path = "/api/v1/items/{id}/photos/{photoId}",
    tag = "photos",
    summary = "读取视图照片",
    description = "返回照片并带 ETag: \"r<revision>\"；跨物品/不存在 → 404。",
    params(
        ("id" = String, Path, description = "物品 ID（UUIDv7）"),
        ("photoId" = String, Path, description = "照片 ID（UUIDv7）"),
    ),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "照片", body = PhotoResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "照片不存在或不属于该物品", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn get_photo(
    State(state): State<AppState>,
    request_id: RequestId,
    Path((item_id, photo_id)): Path<(String, String)>,
) -> Response {
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    match photos::find_for_item(&mut connection, &item_id, &photo_id).await {
        Ok(Some(photo)) => photo_response(photo, StatusCode::OK),
        Ok(None) => ApiError::not_found("照片不存在或不属于该物品").render(&request_id),
        Err(error) => ApiError::from_storage(error).render(&request_id),
    }
}

/// `PATCH /api/v1/items/{id}/photos/{photoId}` —— 修改视图或替换资产（If-Match）。
#[utoipa::path(
    patch,
    path = "/api/v1/items/{id}/photos/{photoId}",
    tag = "photos",
    summary = "修改视图照片（If-Match 乐观锁）",
    description = "字段缺失 = 保持原值；显式 null = 422（照片必须始终有资产与视图）；空请求体 = 422。\
                   修改视图时若目标视图已被同物品另一张照片占用 → 422 \
                   （details.reason=viewOccupied + existingPhotoId）。缺 If-Match → 428；\
                   revision 过期 → 412 + details.currentRevision。",
    params(
        ("id" = String, Path, description = "物品 ID（UUIDv7）"),
        ("photoId" = String, Path, description = "照片 ID（UUIDv7）"),
    ),
    request_body = PhotoPatchRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "更新后的照片（带新 ETag）", body = PhotoResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "照片/资产不存在或不属于该物品", body = super::dto::ApiErrorResponse),
        (status = 412, description = "revision 过期（details.currentRevision）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "字段级校验失败或视图占用（details.fields / details.reason）", body = super::dto::ApiErrorResponse),
        (status = 428, description = "缺少 If-Match", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn patch_photo(
    State(state): State<AppState>,
    request_id: RequestId,
    Path((item_id, photo_id)): Path<(String, String)>,
    headers: HeaderMap,
    super::body::JsonBody(patch): super::body::JsonBody<PhotoPatchRequest>,
) -> Response {
    let expected_revision = match parse_if_match(&headers) {
        Ok(revision) => revision,
        Err(error) => return error.render(&request_id),
    };
    let (asset_id, view) = match validate_photo_patch(patch) {
        Ok(validated) => validated,
        Err(issues) => return ApiError::field_validation(issues).render(&request_id),
    };

    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    if let Err(error) = ensure_item(&mut connection, &item_id).await {
        return error.render(&request_id);
    }
    let current = match photos::find_for_item(&mut connection, &item_id, &photo_id).await {
        Ok(Some(photo)) => photo,
        Ok(None) => return ApiError::not_found("照片不存在或不属于该物品").render(&request_id),
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };

    let target_asset = match &asset_id {
        None => current.asset_id.clone(),
        Some(asset_id) => match check_photo_asset(&mut connection, &item_id, asset_id).await {
            Ok(asset) => asset.id,
            Err(error) => return error.render(&request_id),
        },
    };
    let target_view = view.unwrap_or(current.view);

    // 用 `&mut connection` 开事务：事务借用连接，提交/回滚后连接仍可继续查询
    // （失败分支需要在同一连接上查占用者给出可读错误）。
    // `BEGIN IMMEDIATE`（BUG-006）：先读（`view_occupant`）后写（UPDATE），
    // 与 `create_photo` 同因同修（见 `storage::tx`）。
    let mut transaction = match crate::storage::begin_write(&mut connection).await {
        Ok(transaction) => transaction,
        Err(error) => {
            tracing::error!(error = %error, "开启事务失败");
            return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
        }
    };
    match photos::view_occupant(&mut transaction, &item_id, target_view, Some(&photo_id)).await {
        Ok(Some(existing)) => return view_occupied(&request_id, target_view, &existing),
        Ok(None) => {}
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    }
    let updated = photos::update(
        &mut transaction,
        &item_id,
        &photo_id,
        expected_revision,
        photos::PhotoUpdate {
            asset_id: target_asset,
            view: target_view,
        },
    )
    .await;
    match updated {
        Ok(photo) => {
            if let Err(error) = transaction.commit().await {
                tracing::error!(error = %error, "提交照片事务失败");
                return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
            }
            tracing::info!(
                requestId = %request_id,
                itemId = %item_id,
                photoId = %photo.id,
                view = target_view.as_str(),
                revision = photo.revision,
                "视图照片已更新"
            );
            photo_response(photo, StatusCode::OK)
        }
        Err(crate::storage::StorageError::UniqueViolation { .. }) => {
            // 并发窗口：索引拒绝。回滚事务后查询占用者，给出同样的可读错误。
            drop(transaction);
            let occupant =
                photos::view_occupant(&mut connection, &item_id, target_view, Some(&photo_id))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
            view_occupied(&request_id, target_view, &occupant)
        }
        Err(error) => ApiError::from_storage(error).render(&request_id),
    }
}

/// PATCH 输入的字段级校验：至少一个字段；显式 `null` 一律拒绝。
fn validate_photo_patch(
    patch: PhotoPatchRequest,
) -> Result<(Option<String>, Option<PhotoView>), Vec<FieldIssue>> {
    if patch.asset_id.is_none() && patch.view.is_none() {
        return Err(vec![FieldIssue::new(
            BODY_FIELD,
            "请求体不能为空：至少提供一个可修改字段（assetId/view）",
        )]);
    }
    let mut issues = Vec::new();
    let asset_id = match patch.asset_id {
        None => None,
        Some(None) => {
            issues.push(FieldIssue::new(
                "assetId",
                "不能为 null：照片必须关联资产（换图请提供新资产 ID）",
            ));
            None
        }
        Some(Some(asset_id)) => Some(asset_id),
    };
    let view = match patch.view {
        None => None,
        Some(None) => {
            issues.push(FieldIssue::new(
                "view",
                "不能为 null：照片必须有视图（改选视图请提供新取值）",
            ));
            None
        }
        Some(Some(raw)) => match validate_photo_view(Some(&raw)) {
            Ok(view) => Some(view),
            Err(issue) => {
                issues.push(issue);
                None
            }
        },
    };
    if !issues.is_empty() {
        return Err(issues);
    }
    Ok((asset_id, view))
}

/// 物品必须存在（404，不泄露归属）。
async fn ensure_item(
    connection: &mut sqlx::SqliteConnection,
    item_id: &str,
) -> Result<(), ApiError> {
    match items::get(connection, item_id).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(ApiError::not_found(format!("item 不存在：{item_id}"))),
        Err(error) => Err(ApiError::from_storage(error)),
    }
}

/// 校验照片资产：归属同一物品（否则 404）、`purpose=photo`、JPEG/PNG、内容可用。
async fn check_photo_asset(
    connection: &mut sqlx::SqliteConnection,
    item_id: &str,
    asset_id: &str,
) -> Result<manual_core::domain::Asset, ApiError> {
    let field = "assetId";
    let asset = match assets_repo::find_for_item(connection, item_id, asset_id).await {
        Ok(Some(asset)) => asset,
        Ok(None) => return Err(ApiError::not_found("资产不存在或不属于该物品")),
        Err(error) => return Err(ApiError::from_storage(error)),
    };
    if asset.purpose != AssetPurpose::Photo {
        return Err(ApiError::field_validation(vec![FieldIssue::new(
            field,
            "该资产不是照片（上传时 purpose 必须是 photo）",
        )]));
    }
    let blob = match blobs_repo::get(connection, &asset.blob_id).await {
        Ok(Some(blob)) => blob,
        Ok(None) => {
            tracing::error!(assetId = %asset.id, "资产引用的 blob 不存在");
            return Err(ApiError::internal("服务器内部错误：资产元数据不完整"));
        }
        Err(error) => return Err(ApiError::from_storage(error)),
    };
    if blob.mime != "image/jpeg" && blob.mime != "image/png" {
        return Err(ApiError::field_validation(vec![FieldIssue::new(
            field,
            "该资产的内容不是 JPEG/PNG 图片（照片只接受这两种格式）",
        )]));
    }
    if blob.storage_state != BlobStorageState::Stored {
        return Err(ApiError::unprocessable_reason(
            "assetUnavailable",
            "该资产当前不可用（内容缺失或已隔离）",
            serde_json::json!({ "storageState": blob.storage_state.as_str() }),
        ));
    }
    Ok(asset)
}

/// 422 `details.reason=viewOccupied`：该视图已被同物品的另一张照片占用。
fn view_occupied(request_id: &RequestId, view: PhotoView, existing_photo_id: &str) -> Response {
    ApiError::unprocessable_reason(
        "viewOccupied",
        format!(
            "该视图（{}）已有照片：请先调整或更换已有照片的视图，而不是新增第二张",
            view.as_str()
        ),
        serde_json::json!({
            "view": view.as_str(),
            "existingPhotoId": existing_photo_id,
        }),
    )
    .render(request_id)
}

/// `{data}` + `ETag` 响应（PATCH/GET 的 If-Match 依据）。
fn photo_response(photo: Photo, status: StatusCode) -> Response {
    let etag = etag_value(photo.revision);
    let mut response = (
        status,
        Json(PhotoResponse {
            data: PhotoDto::from(photo),
        }),
    )
        .into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

/// 获取数据库连接；失败时返回 [`ApiError`]（调用方渲染，减少 Result 体积）。
async fn acquire(state: &AppState) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, ApiError> {
    state.database().pool().acquire().await.map_err(|error| {
        tracing::error!(error = %error, "获取数据库连接失败");
        ApiError::internal("服务器内部错误：数据库暂不可用")
    })
}
