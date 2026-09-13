//! 说明书绑定路由（REQ-012 / AC-020）：`POST` 与 `GET /items/{id}/documents`。
//!
//! 绑定规则（QA 按此复核）：
//! - `sourceAssetId` 必须是**同一物品**的资产（`repo::assets::find_for_item` 是唯一
//!   归属入口）：不存在或跨物品 → 404（不泄露存在性，`contracts.md` §2）；
//! - 资产必须 `purpose=document` 且内容为 PDF（`blobs.mime = application/pdf`）→
//!   否则 422 `details.fields[sourceAssetId]`；
//! - 资产当前不可用（`storage_state` 非 stored）→ 422（不绑定读不到的原件）；
//! - `sourceSha256` 取自 blob（内容寻址主键），供 T09 准备阶段核对字节未变；
//! - `sourceUrl` 只作**出处记录**：校验绝对 http(s) URL 后原样保存，
//!   **服务端不发起任何抓取**（本服务依赖树里没有 HTTP 客户端；测试用本机计数监听器
//!   证明 0 次外呼）；
//! - 加密/超页数 PDF 的权威拒绝在 T09（REQ-014），本卡按 REQ-012 只校验类型与归属。
//!
//! 读取侧：`GET /items/{id}/documents` 是 T07 增加的集合路由（contracts.md §3 列出
//! 核心路由，未禁止读取集合；前端文档卡片与"归档后资料仍可读"的证据都依赖它）。

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing};
use manual_core::domain::AssetPurpose;
use manual_core::validation::{FieldIssue, validate_document_fields};

use crate::storage::repo::{assets as assets_repo, blobs as blobs_repo, documents, items};

use super::dto::{DocumentCreateRequest, DocumentDto, DocumentListResponse, DocumentResponse};
use super::error::{ApiError, RequestId};
use super::pagination::{Cursor, parse_list_params};
use super::state::AppState;

/// 受会话保护的说明书路由。
pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/items/{id}/documents",
        routing::get(list_documents).post(create_document),
    )
}

/// `POST /api/v1/items/{id}/documents` —— 绑定已上传的 PDF 原件。
#[utoipa::path(
    post,
    path = "/api/v1/items/{id}/documents",
    tag = "documents",
    summary = "绑定说明书原件（PDF 资产 → document）",
    description = "校验资产归属同一物品且为 PDF（purpose=document）；跨物品/未知资产 → 404。\
                   保存 sourceSha256（原件 blob 哈希）。sourceUrl 只作出处记录，服务端**不会**\
                   访问该地址（只接受绝对 http(s) URL）。加密与超页数 PDF 的拒绝发生在准备阶段（T09）。",
    params(("id" = String, Path, description = "物品 ID（UUIDv7）")),
    request_body = DocumentCreateRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 201, description = "已绑定", body = DocumentResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品或资产不存在/不属该物品", body = super::dto::ApiErrorResponse),
        (status = 422, description = "字段级校验失败（details.fields）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn create_document(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(item_id): Path<String>,
    super::body::JsonBody(body): super::body::JsonBody<DocumentCreateRequest>,
) -> Response {
    let fields = match validate_document_fields(body.title.as_deref(), body.source_url.as_deref()) {
        Ok(fields) => fields,
        Err(issues) => return ApiError::field_validation(issues).render(&request_id),
    };

    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    match items::get(&mut connection, &item_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return ApiError::not_found(format!("item 不存在：{item_id}")).render(&request_id);
        }
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    }

    // 归属校验：跨物品与不存在都返回 404（不泄露存在性）。
    let asset =
        match assets_repo::find_for_item(&mut connection, &item_id, &body.source_asset_id).await {
            Ok(Some(asset)) => asset,
            Ok(None) => {
                return ApiError::not_found("资产不存在或不属于该物品").render(&request_id);
            }
            Err(error) => return ApiError::from_storage(error).render(&request_id),
        };
    if asset.purpose != AssetPurpose::Document {
        return ApiError::field_validation(vec![FieldIssue::new(
            "sourceAssetId",
            "该资产不是说明书原件（上传时 purpose 必须是 document）",
        )])
        .render(&request_id);
    }

    let blob = match blobs_repo::get(&mut connection, &asset.blob_id).await {
        Ok(Some(blob)) => blob,
        Ok(None) => {
            // 外键保证不应发生：blob 行缺失属于数据损坏。
            tracing::error!(requestId = %request_id, assetId = %asset.id, "资产引用的 blob 不存在");
            return ApiError::internal("服务器内部错误：资产元数据不完整").render(&request_id);
        }
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };
    if blob.mime != "application/pdf" {
        return ApiError::field_validation(vec![FieldIssue::new(
            "sourceAssetId",
            "该资产的内容不是 PDF（说明书原件必须是 PDF）",
        )])
        .render(&request_id);
    }
    if blob.storage_state != manual_core::domain::BlobStorageState::Stored {
        return ApiError::unprocessable_reason(
            "assetUnavailable",
            "该资产当前不可用（内容缺失或已隔离）",
            serde_json::json!({ "storageState": blob.storage_state.as_str() }),
        )
        .render(&request_id);
    }

    match documents::create(
        &mut connection,
        documents::NewDocument {
            item_id: item_id.clone(),
            source_asset_id: asset.id.clone(),
            // 内容寻址：blob id 即 sha256（绑定那一刻的原件哈希）。
            source_sha256: blob.sha256.clone(),
            title: fields.title,
            source_url: fields.source_url,
        },
    )
    .await
    {
        Ok(document) => {
            tracing::info!(
                requestId = %request_id,
                itemId = %item_id,
                documentId = %document.id,
                sourceSha256 = %document.source_sha256,
                "说明书原件已绑定"
            );
            (
                StatusCode::CREATED,
                Json(DocumentResponse {
                    data: DocumentDto::from(document),
                }),
            )
                .into_response()
        }
        Err(error) => ApiError::from_storage(error).render(&request_id),
    }
}

/// `GET /api/v1/items/{id}/documents` —— 某物品的 document 集合（分页）。
#[utoipa::path(
    get,
    path = "/api/v1/items/{id}/documents",
    tag = "documents",
    summary = "说明书绑定列表（游标分页）",
    description = "按 createdAt DESC, id DESC 分页；游标不透明且只在本集合（同一物品）内有效。\
                   归档物品的资料同样可读。",
    params(
        ("id" = String, Path, description = "物品 ID（UUIDv7）"),
        ("limit" = Option<u32>, Query, description = "每页条数，默认 20，最大 100"),
        ("cursor" = Option<String>, Query, description = "上一页返回的 nextCursor（原样回传）"),
    ),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "document 列表", body = DocumentListResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品不存在", body = super::dto::ApiErrorResponse),
        (status = 422, description = "查询参数非法（details.fields）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn list_documents(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(item_id): Path<String>,
    Query(params): Query<Vec<(String, String)>>,
) -> Response {
    let parsed = match parse_list_params(&params, &[]) {
        Ok(parsed) => parsed,
        Err(issues) => return ApiError::field_validation(issues).render(&request_id),
    };
    let scope = format!("documents:{item_id}");
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
    match items::get(&mut connection, &item_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return ApiError::not_found(format!("item 不存在：{item_id}")).render(&request_id);
        }
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    }
    let mut rows =
        match documents::list_page(&mut connection, &item_id, cursor, parsed.limit + 1).await {
            Ok(rows) => rows,
            Err(error) => return ApiError::from_storage(error).render(&request_id),
        };
    let has_more = rows.len() as u32 > parsed.limit;
    rows.truncate(parsed.limit as usize);
    let next_cursor = if has_more {
        rows.last()
            .map(|document| Cursor::encode(&scope, document.created_at.as_millis(), &document.id))
    } else {
        None
    };
    let data: Vec<DocumentDto> = rows.into_iter().map(DocumentDto::from).collect();
    Json(DocumentListResponse { data, next_cursor }).into_response()
}

/// 获取数据库连接；失败时返回 [`ApiError`]（调用方用本次请求的 requestId 渲染）。
async fn acquire(state: &AppState) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, ApiError> {
    state.database().pool().acquire().await.map_err(|error| {
        tracing::error!(error = %error, "获取数据库连接失败");
        ApiError::internal("服务器内部错误：数据库暂不可用")
    })
}
