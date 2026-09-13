//! PDF 准备与页上传路由（T09 / REQ-014、REQ-015；contracts.md §3）。
//!
//! 四个端点：
//! - `POST /documents/{id}/preparations`：创建（或复用未完成的）preparation。
//!   同 document + 同 `sourceSha256` 已有 `preparing` 记录时返回该记录（200），
//!   否则新建（201）——这就是"重新进入同一 document 继续准备"的入口。
//! - `GET /preparations/{id}`：状态 + 已上传页 + 缺页 + revision（`ETag: "r<n>"`），
//!   断线续传据此只补缺页（不假定 IndexedDB 是事实来源）。
//! - `PUT /preparations/{id}/pages/{pageNumber}`：1-based 页上传。相同内容
//!   （资产 blob sha256 + viewport 都相同）幂等；内容变化必须带 `If-Match`（缺 428、
//!   过期 412）；`ready` 后拒写（422 `details.reason=preparationReady`）。
//! - `POST /preparations/{id}/complete`：`If-Match` + `pageCount` 封存为 ready，
//!   事务内校验页号连续 1..N 与资产归属；**不创建 job、不写费用账本、不外呼**。
//!
//! 与加密/超页数 PDF 的分工（ADR-003）：原 PDF 由浏览器解析，加密与"实际页数 >100"
//! 的**权威拒绝发生在浏览器**（那里才有 PDF 解析器）；服务端不信任客户端，
//! 在 PUT 与 complete 里对页号与 `pageCount` 再做 ≤100 页的校验
//! （422 `details.reason=pageLimitExceeded`），避免"客户端绕过限制写入 500 页"。

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, routing};
use manual_core::domain::AssetPurpose;
use manual_core::domain::PreparationState;
use manual_core::validation::{
    FieldIssue, MAX_PDF_PAGES, validate_page_number, validate_page_viewport,
};

use crate::storage::error::StorageError;
use crate::storage::repo::{
    assets as assets_repo, blobs as blobs_repo, documents, preparations as preps,
};

use super::dto::{
    PageDto, PagePutRequest, PageResponse, PreparationCompleteRequest, PreparationCreateRequest,
    PreparationDetailDto, PreparationDetailResponse, PreparationDto, PreparationResponse,
    ViewportDto,
};
use super::error::{ApiError, RequestId};
use super::precondition::{etag_value, parse_if_match};
use super::state::AppState;

/// 受会话保护的 preparation 路由（挂在与物品同级的 `/api/v1` 下）。
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/documents/{id}/preparations",
            routing::post(create_preparation),
        )
        .route("/preparations/{id}", routing::get(get_preparation))
        .route(
            "/preparations/{id}/pages/{page_number}",
            routing::put(put_page),
        )
        .route(
            "/preparations/{id}/complete",
            routing::post(complete_preparation),
        )
}

/// `POST /api/v1/documents/{id}/preparations`。
#[utoipa::path(
    post,
    path = "/api/v1/documents/{id}/preparations",
    tag = "preparations",
    summary = "创建或复用 PDF 准备记录",
    description = "校验 `sourceSha256` 与绑定的原件一致（不一致 422 `details.reason=sourceChanged`）。\
                   同一 document + 原件已有未完成（preparing）记录时**返回现有记录**（200），\
                   否则新建（201）——重新进入同一 document 继续准备走这条路径。\
                   加密与超页数 PDF 的权威拒绝在浏览器（那里才有 PDF 解析器，ADR-003），\
                   本端点不创建页记录、不进入 jobs、不产生任何费用。",
    params(("id" = String, Path, description = "document ID（UUIDv7）")),
    request_body = PreparationCreateRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 201, description = "已创建新的准备记录", body = PreparationResponse),
        (status = 200, description = "复用了未完成的准备记录", body = PreparationResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "document 不存在", body = super::dto::ApiErrorResponse),
        (status = 422, description = "字段级校验失败（details.fields）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn create_preparation(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(document_id): Path<String>,
    super::body::JsonBody(body): super::body::JsonBody<PreparationCreateRequest>,
) -> Response {
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let document = match documents::get(&mut connection, &document_id).await {
        Ok(Some(document)) => document,
        Ok(None) => {
            return ApiError::not_found(format!("document 不存在：{document_id}"))
                .render(&request_id);
        }
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };

    let Some(source_sha256) = body.source_sha256.as_deref().map(str::trim) else {
        return ApiError::field_validation(vec![FieldIssue::new(
            "sourceSha256",
            "必填：请携带当前绑定原件的 sha256（取自 document.sourceSha256）",
        )])
        .render(&request_id);
    };
    if source_sha256 != document.source_sha256 {
        return ApiError::unprocessable_reason(
            "sourceChanged",
            "原 PDF 已变化：请刷新资料后重新绑定或重新开始准备",
            serde_json::json!({ "currentSourceSha256": document.source_sha256 }),
        )
        .render(&request_id);
    }

    // 复用未完成记录：断线续传不创建新记录、不丢已完成页。
    match preps::find_preparing_for_document(&mut connection, &document_id, &document.source_sha256)
        .await
    {
        Ok(Some(existing)) => {
            return (
                StatusCode::OK,
                Json(PreparationResponse {
                    data: PreparationDto::from_preparation(&existing),
                }),
            )
                .into_response();
        }
        Ok(None) => {}
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    }

    match preps::create(
        &mut connection,
        preps::NewPreparation {
            document_id: document.id.clone(),
            source_sha256: document.source_sha256.clone(),
        },
    )
    .await
    {
        Ok(preparation) => {
            tracing::info!(
                requestId = %request_id,
                documentId = %document.id,
                preparationId = %preparation.id,
                "已创建 PDF 准备记录"
            );
            (
                StatusCode::CREATED,
                Json(PreparationResponse {
                    data: PreparationDto::from_preparation(&preparation),
                }),
            )
                .into_response()
        }
        Err(error) => ApiError::from_storage(error).render(&request_id),
    }
}

/// `GET /api/v1/preparations/{id}`。
#[utoipa::path(
    get,
    path = "/api/v1/preparations/{id}",
    tag = "preparations",
    summary = "准备状态、页状态与缺页（断线续传入口）",
    description = "响应带 `ETag: \"r<revision>\"`（封存与页覆盖用 If-Match）。\
                   `missingPages`：未封存时为空数组（总页数只有浏览器知道），ready 时为 1..pageCount 中缺失的页号。\
                   客户端只补缺页，不重传已完成页；服务端记录是事实来源，IndexedDB 不是。",
    params(("id" = String, Path, description = "preparation ID（UUIDv7）")),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "准备详情", body = PreparationDetailResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "preparation 不存在", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn get_preparation(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(id): Path<String>,
) -> Response {
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let preparation = match preps::get(&mut connection, &id).await {
        Ok(Some(preparation)) => preparation,
        Ok(None) => {
            return ApiError::not_found(format!("preparation 不存在：{id}")).render(&request_id);
        }
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };
    let pages = match preps::list_pages(&mut connection, &id).await {
        Ok(pages) => pages,
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };
    let missing_pages = match preparation.state {
        PreparationState::Ready => {
            let page_count = preparation.page_count.unwrap_or_default();
            let present: std::collections::HashSet<i64> =
                pages.iter().map(|page| page.page_number).collect();
            (1..=page_count)
                .filter(|number| !present.contains(number))
                .collect()
        }
        PreparationState::Preparing => Vec::new(),
    };
    let dto = PreparationDetailDto::new(&preparation, &pages, missing_pages);
    let mut response = Json(PreparationDetailResponse { data: dto }).into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&etag_value(preparation.revision)) {
        response
            .headers_mut()
            .insert(axum::http::header::ETAG, value);
    }
    response
}

/// `PUT /api/v1/preparations/{id}/pages/{pageNumber}`。
#[utoipa::path(
    put,
    path = "/api/v1/preparations/{id}/pages/{pageNumber}",
    tag = "preparations",
    summary = "上传单页（页文字/页图与 viewport）",
    description = "页码 1-based（≥1 且 ≤100）。资产必须属于该物品且 purpose 正确\
                   （pageText / pageImage，pageImage 必须是 JPEG），否则 404/422。\
                   相同内容（资产 blob sha256 与 viewport 都相同）重复提交幂等，\
                   不自增 revision；内容变化必须带 If-Match（缺 428、过期 412）；\
                   封存（ready）后拒写（422 `details.reason=preparationReady`）。",
    params(
        ("id" = String, Path, description = "preparation ID（UUIDv7）"),
        ("pageNumber" = i64, Path, description = "1-based 页号（1..100）"),
    ),
    request_body = PagePutRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "页已写入（相同内容幂等时同样返回 200）", body = PageResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "preparation 或资产不存在/不属该物品", body = super::dto::ApiErrorResponse),
        (status = 412, description = "If-Match 的 revision 已过期", body = super::dto::ApiErrorResponse),
        (status = 422, description = "字段/业务校验失败（页号、viewport、资产、ready 拒写）", body = super::dto::ApiErrorResponse),
        (status = 428, description = "内容变化但缺少 If-Match", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn put_page(
    State(state): State<AppState>,
    request_id: RequestId,
    Path((preparation_id, page_number)): Path<(String, i64)>,
    headers: HeaderMap,
    super::body::JsonBody(body): super::body::JsonBody<PagePutRequest>,
) -> Response {
    // 1) 页号（1-based、≤100）。
    let page_number = match validate_page_number(page_number) {
        Ok(page_number) => page_number,
        Err(issue) => {
            let reason = if page_number > MAX_PDF_PAGES {
                "pageLimitExceeded"
            } else {
                "invalidPageNumber"
            };
            return ApiError::unprocessable_reason(reason, issue.message, serde_json::json!({}))
                .render(&request_id);
        }
    };

    // 2) viewport（必填；尺寸/长边/旋转）。
    let Some(viewport) = body.viewport.map(ViewportDto::to_viewport) else {
        return ApiError::field_validation(vec![FieldIssue::new(
            "viewport",
            "必填：页图坐标以旋转后 viewport 左上角为原点，需要 width/height/rotation",
        )])
        .render(&request_id);
    };
    if let Err(issues) = validate_page_viewport(viewport) {
        return ApiError::field_validation(issues).render(&request_id);
    }

    // 3) If-Match：可选（相同内容幂等时不需要）；解析失败仍按 422（与其它路由一致）。
    let expected_revision = match headers.contains_key(super::precondition::IF_MATCH) {
        false => None,
        true => match parse_if_match(&headers) {
            Ok(revision) => Some(revision),
            Err(error) => return error.render(&request_id),
        },
    };

    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let item_id = match preps::item_id_of(&mut connection, &preparation_id).await {
        Ok(Some(item_id)) => item_id,
        Ok(None) => {
            return ApiError::not_found(format!("preparation 不存在：{preparation_id}"))
                .render(&request_id);
        }
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };

    // 4) 页文字资产（可选）：归属 + purpose=page_text + 内容可用。
    let text_asset_id = match validate_page_asset(
        &mut connection,
        &item_id,
        body.text_asset_id.as_deref(),
        AssetPurpose::PageText,
        false,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => return error.render(&request_id),
    };
    // 5) 页图资产（必填）：归属 + purpose=page_image + JPEG。
    let image_asset_id = match validate_page_asset(
        &mut connection,
        &item_id,
        body.image_asset_id.as_deref(),
        AssetPurpose::PageImage,
        true,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => return error.render(&request_id),
    };

    match preps::write_page(
        &mut connection,
        &preparation_id,
        preps::NewPage {
            page_number,
            text_asset_id,
            image_asset_id,
            viewport,
        },
        expected_revision,
    )
    .await
    {
        Ok((page, outcome)) => {
            tracing::info!(
                requestId = %request_id,
                preparationId = %preparation_id,
                pageNumber = page_number,
                changed = matches!(outcome, preps::PageWriteOutcome::Changed),
                "页已写入"
            );
            (
                StatusCode::OK,
                Json(PageResponse {
                    data: PageDto::from_page(&page),
                }),
            )
                .into_response()
        }
        Err(preps::PageWriteError::PreconditionRequired) => ApiError::precondition_required(
            "该页已有不同内容：覆盖需要 If-Match（先用 GET /preparations/{id} 取 ETag）",
        )
        .render(&request_id),
        Err(preps::PageWriteError::Storage(StorageError::NotWritable { state, .. })) => {
            ApiError::unprocessable_reason(
                "preparationReady",
                format!("准备已封存（{state}）：ready 后不可再写入页"),
                serde_json::json!({}),
            )
            .render(&request_id)
        }
        Err(preps::PageWriteError::Storage(error)) => {
            ApiError::from_storage(error).render(&request_id)
        }
    }
}

/// `POST /api/v1/preparations/{id}/complete`。
#[utoipa::path(
    post,
    path = "/api/v1/preparations/{id}/complete",
    tag = "preparations",
    summary = "封存准备（ready）",
    description = "`If-Match` + `pageCount`；事务内校验 1..N 连续、每页有页图资产、\
                   资产属于该物品且内容可用；缺页/资产不符 → 422 列出缺项（`details.missingPages` / `details.pages`）。\
                   成功标记 `clientDerived` 并保留原件供复核。**不创建 job、不写费用账本、不产生任何外呼**。",
    params(("id" = String, Path, description = "preparation ID（UUIDv7）")),
    request_body = PreparationCompleteRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "已封存（ready）", body = PreparationResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
        (status = 404, description = "preparation 不存在", body = super::dto::ApiErrorResponse),
        (status = 412, description = "If-Match 的 revision 已过期", body = super::dto::ApiErrorResponse),
        (status = 422, description = "缺页/资产不符/页数超限/已封存", body = super::dto::ApiErrorResponse),
        (status = 428, description = "缺少 If-Match", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn complete_preparation(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(id): Path<String>,
    headers: HeaderMap,
    super::body::JsonBody(body): super::body::JsonBody<PreparationCompleteRequest>,
) -> Response {
    let expected_revision = match parse_if_match(&headers) {
        Ok(revision) => revision,
        Err(error) => return error.render(&request_id),
    };

    let Some(page_count) = body.page_count else {
        return ApiError::field_validation(vec![FieldIssue::new(
            "pageCount",
            "必填：封存需要声明总页数（1..=100）",
        )])
        .render(&request_id);
    };
    if !(1..=MAX_PDF_PAGES).contains(&page_count) {
        return ApiError::unprocessable_reason(
            "pageLimitExceeded",
            format!("页数必须在 1..={MAX_PDF_PAGES} 之间（本次声明 {page_count}）"),
            serde_json::json!({ "pageCount": page_count, "maxPages": MAX_PDF_PAGES }),
        )
        .render(&request_id);
    }

    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let item_id = match preps::item_id_of(&mut connection, &id).await {
        Ok(Some(item_id)) => item_id,
        Ok(None) => {
            return ApiError::not_found(format!("preparation 不存在：{id}")).render(&request_id);
        }
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };

    match preps::complete(
        &mut connection,
        &id,
        &item_id,
        page_count,
        expected_revision,
    )
    .await
    {
        Ok(preparation) => {
            tracing::info!(
                requestId = %request_id,
                preparationId = %preparation.id,
                pageCount = page_count,
                "准备已封存（不创建 job、不产生费用）"
            );
            Json(PreparationResponse {
                data: PreparationDto::from_preparation(&preparation),
            })
            .into_response()
        }
        Err(preps::CompleteError::Incomplete(failure)) => {
            let (reason, details) = match failure {
                preps::CompleteFailure::MissingPages { missing } => (
                    "incompletePages",
                    serde_json::json!({ "missingPages": missing, "pageCount": page_count }),
                ),
                preps::CompleteFailure::AssetMismatch { details } => (
                    "assetMismatch",
                    serde_json::json!({
                        "pages": details
                            .iter()
                            .map(|problem| serde_json::json!({
                                "pageNumber": problem.page_number,
                                "problem": problem.problem,
                            }))
                            .collect::<Vec<_>>(),
                        "pageCount": page_count,
                    }),
                ),
            };
            ApiError::unprocessable_reason(reason, "无法封存：请先补齐列出的缺项", details)
                .render(&request_id)
        }
        Err(preps::CompleteError::Storage(StorageError::NotWritable { state, .. })) => {
            ApiError::unprocessable_reason(
                "preparationReady",
                format!("准备已处于 {state}：无需重复封存"),
                serde_json::json!({}),
            )
            .render(&request_id)
        }
        Err(preps::CompleteError::Storage(error)) => {
            ApiError::from_storage(error).render(&request_id)
        }
    }
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

/// 校验页资产：缺失/异常 → `Err(ApiError)`；`required` 时缺失 → 422 字段错误。
///
/// 归属校验走 `assets::find_for_item`（跨物品/不存在 → 404，不泄露存在性）；
/// purpose 与 MIME（页图必须是 JPEG）不符 → 422 字段级明细。
async fn validate_page_asset(
    conn: &mut sqlx::SqliteConnection,
    item_id: &str,
    asset_id: Option<&str>,
    purpose: AssetPurpose,
    required: bool,
) -> Result<Option<String>, ApiError> {
    let field = match purpose {
        AssetPurpose::PageText => "textAssetId",
        AssetPurpose::PageImage => "imageAssetId",
        _ => "assetId",
    };
    let Some(asset_id) = asset_id else {
        if required {
            return Err(ApiError::field_validation(vec![FieldIssue::new(
                field,
                "必填：每页都要有页图（扫描页同样上传页图）",
            )]));
        }
        return Ok(None);
    };

    let asset = match assets_repo::find_for_item(conn, item_id, asset_id).await {
        Ok(Some(asset)) => asset,
        Ok(None) => {
            return Err(ApiError::not_found("资产不存在或不属于该物品"));
        }
        Err(error) => return Err(ApiError::from_storage(error)),
    };
    if asset.purpose != purpose {
        let expected = match purpose {
            AssetPurpose::PageText => "pageText",
            AssetPurpose::PageImage => "pageImage",
            _ => "asset",
        };
        return Err(ApiError::field_validation(vec![FieldIssue::new(
            field,
            format!(
                "该资产不是{}（上传时 purpose 必须是 {expected}）",
                purpose_label(purpose)
            ),
        )]));
    }
    match blobs_repo::get(conn, &asset.blob_id).await {
        Ok(Some(blob)) => {
            if blob.storage_state != manual_core::domain::BlobStorageState::Stored {
                return Err(ApiError::unprocessable_reason(
                    "assetUnavailable",
                    "该资产当前不可用（内容缺失或已隔离）",
                    serde_json::json!({ "field": field }),
                ));
            }
            if purpose == AssetPurpose::PageImage
                && blob.mime != manual_core::validation::PAGE_IMAGE_MIME
            {
                return Err(ApiError::field_validation(vec![FieldIssue::new(
                    field,
                    format!(
                        "页图必须是白底 JPEG（收到 {}）：请让浏览器按 JPEG 渲染页图",
                        blob.mime
                    ),
                )]));
            }
        }
        Ok(None) => {
            tracing::error!(assetId = %asset_id, "资产引用的 blob 不存在");
            return Err(ApiError::internal("服务器内部错误：资产元数据不完整"));
        }
        Err(error) => return Err(ApiError::from_storage(error)),
    }
    Ok(Some(asset.id))
}

fn purpose_label(purpose: AssetPurpose) -> &'static str {
    match purpose {
        AssetPurpose::PageText => "页文字资产",
        AssetPurpose::PageImage => "页图资产",
        _ => "该用途的资产",
    }
}

/// 获取数据库连接；失败时返回 [`ApiError`]（调用方用本次请求的 requestId 渲染）。
async fn acquire(state: &AppState) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, ApiError> {
    state.database().pool().acquire().await.map_err(|error| {
        tracing::error!(error = %error, "获取数据库连接失败");
        ApiError::internal("服务器内部错误：数据库暂不可用")
    })
}
