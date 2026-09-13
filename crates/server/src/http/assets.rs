//! 资产路由（T06）：`POST /items/{id}/assets` 与 `GET/HEAD /assets/{id}/content`。
//!
//! 上传（REQ-011）：
//! - `multipart/form-data` 字段 `purpose` + `file`（流式读取，见 `crate::assets`）；
//! - 路由级 `DefaultBodyLimit` 取"最大用途上限 + multipart 开销"，**解析前**先按体积拒绝；
//!   读流时 `StagedWriter` 再次计数（用途上限，413）；
//! - 物品必须存在（404，不泄露归属）；成功 201 + 资产 DTO（不含磁盘路径）。
//!
//! 内容服务（contracts.md §7 的 Range 合同）：
//! - 200 完整 / 206 单区间 + `Content-Range` / 416 不可满足（带 `Content-Range: bytes */N`）
//!   / 多区间与非法语法回落 200 / `If-None-Match` 命中 304 / `If-Range` 不匹配 200；
//! - HEAD 与 GET 同头无 body；ETag 是内容 sha256 的强校验器；
//! - 不对 Range 响应做动态压缩（本应用不挂压缩中间件；测试断言无 `Content-Encoding`）。

use axum::Json;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing};
use manual_core::ApiErrorCode;
use manual_core::domain::{AssetPurpose, BlobStorageState};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;

use crate::assets::blob_store::{self, Staged, StagedWriter};
use crate::assets::upload::{self, AssetStore, UploadOutcome, UploadRequest};
use crate::assets::{AssetError, range, validate};
use crate::config::Limits;
use crate::storage::repo::{assets as assets_repo, items};

use super::dto::{AssetDto, AssetResponse};
use super::error::{ApiError, RequestId};
use super::state::AppState;

/// 受会话保护的资产路由。
///
/// 这里的 `DefaultBodyLimit` **覆盖**外层 API 的 JSON 上限（1 MiB）：axum 的该层
/// 以"最后写入请求扩展的值为准"，路由层在中间件之后执行，因此 multipart 上限生效，
/// 而 JSON 路由仍受外层限制（`tests/assets.rs` 有 >1 MiB 上传成功的用例守住这条）。
pub fn routes(limits: &Limits) -> Router<AppState> {
    let upload_limit = validate::max_upload_request_bytes(limits);
    let upload_limit = usize::try_from(upload_limit).unwrap_or(usize::MAX);
    Router::new()
        .route("/items/{id}/assets", routing::post(upload_asset))
        .route(
            "/assets/{id}/content",
            routing::get(get_asset_content).head(head_asset_content),
        )
        .layer(DefaultBodyLimit::max(upload_limit))
}

/// 合同化的 multipart 提取器：把 axum 的拒绝转成统一错误结构（与 `JsonBody` 同思路）。
pub struct MultipartBody(pub Multipart);

impl<S> FromRequest<S> for MultipartBody
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Response> {
        let request_id = request
            .extensions()
            .get::<RequestId>()
            .cloned()
            .unwrap_or_else(|| RequestId(uuid::Uuid::now_v7().to_string()));
        match Multipart::from_request(request, state).await {
            Ok(multipart) => Ok(Self(multipart)),
            // 目前唯一的拒绝分支：Content-Type 不是 multipart 或 boundary 缺失/非法 → 415。
            Err(rejection) => {
                tracing::debug!(rejection = ?rejection, "multipart 提取被拒绝");
                Err(ApiError::new(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    ApiErrorCode::UnsupportedMediaType,
                    "上传接口要求 Content-Type: multipart/form-data 且 boundary 合法",
                )
                .render(&request_id))
            }
        }
    }
}

/// `POST /api/v1/items/{id}/assets`（multipart：`purpose` + `file`）。
#[utoipa::path(
    post,
    path = "/api/v1/items/{id}/assets",
    tag = "assets",
    summary = "上传原始资料文件（multipart 流式）",
    description = "字段：`file`（二进制，流式读取）+ `purpose`（document/photo/pageImage/pageText）。\
                   内容类型按文件头判定（不采信 Content-Type/扩展名）：document 必须 PDF（≤50 MiB、≤100 页）、\
                   photo/pageImage 必须 JPEG/PNG（≤20 MiB，尺寸/像素在预算内）、pageText 为 UTF-8 文本。\
                   同 sha256 内容复用同一 blob（去重）；文件名只作元数据，绝不参与路径。\
                   伪造类型 415、超限 413、像素炸弹/解码失败/页数超限 422、物品不存在 404、\
                   磁盘预留不足 413（details.reason=insufficientStorage，不半提交）。",
    params(("id" = String, Path, description = "物品 ID（UUIDv7）")),
    request_body(content = super::dto::AssetUploadRequest, content_type = "multipart/form-data"),
    security(("sessionCookie" = [])),
    responses(
        (status = 201, description = "资产已保存（内容按 sha256 去重）", body = AssetResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF/Origin 校验失败（含 multipart 修改请求）", body = super::dto::ApiErrorResponse),
        (status = 404, description = "物品不存在", body = super::dto::ApiErrorResponse),
        (status = 413, description = "文件/物品累计/磁盘空间超限", body = super::dto::ApiErrorResponse),
        (status = 415, description = "内容类型不受支持（含伪造类型）", body = super::dto::ApiErrorResponse),
        (status = 422, description = "内容校验失败（像素炸弹/解码失败/页数超限/字段非法）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn upload_asset(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(item_id): Path<String>,
    headers: HeaderMap,
    mut multipart: MultipartBody,
) -> Response {
    match receive_upload(&state, &item_id, &headers, &mut multipart.0).await {
        Ok(outcome) => {
            tracing::info!(
                requestId = %request_id,
                itemId = %item_id,
                assetId = %outcome.asset.id,
                purpose = outcome.asset.purpose.as_str(),
                size = outcome.blob.size,
                sha256 = %outcome.blob.sha256,
                deduplicated = outcome.reused_existing_blob,
                "资产已保存"
            );
            (
                StatusCode::CREATED,
                Json(AssetResponse {
                    data: AssetDto::from_parts(&outcome.asset, &outcome.blob),
                }),
            )
                .into_response()
        }
        Err(error) => {
            let summary = error.log_summary();
            if error.is_client_error() {
                tracing::warn!(
                    requestId = %request_id,
                    itemId = %item_id,
                    error = %summary,
                    "上传被拒绝"
                );
            } else {
                tracing::error!(
                    requestId = %request_id,
                    itemId = %item_id,
                    error = %summary,
                    "上传失败"
                );
            }
            error.into_api_error().render(&request_id)
        }
    }
}

/// 一次上传的完整流程（错误一律经 [`AssetError`] 分类）。
async fn receive_upload(
    state: &AppState,
    item_id: &str,
    headers: &HeaderMap,
    multipart: &mut Multipart,
) -> Result<UploadOutcome, AssetError> {
    let limits = state.settings().limits;
    let store = state.assets().clone();

    // 1) 物品存在性：短连接，读完即释放（不跨请求体读取持有连接）。
    {
        let mut connection = acquire(state).await?;
        match items::get(&mut connection, item_id).await? {
            Some(_) => {}
            None => return Err(AssetError::not_found(format!("item 不存在：{item_id}"))),
        }
    }

    // 2) 解析前的体积与磁盘空间预检（Content-Length 已知时；chunked 由落盘前复检兜底）。
    let content_length = content_length(headers);
    let route_limit = validate::max_upload_request_bytes(&limits);
    if let Some(length) = content_length
        && length > route_limit
    {
        return Err(AssetError::payload_too_large(format!(
            "请求体超过上传上限：{length} > {route_limit} 字节"
        )));
    }
    store.ensure_request_space(content_length)?;

    // 3) 流式读取字段（写 tmp + 计数 + sha256；失败时守卫删除 tmp）。
    let (purpose, original_name, staged) = read_parts(&store, &limits, multipart).await?;

    // 4) 用途上限兜底：`purpose` 出现在 `file` 之后时，第 3 步只能用最大上限。
    let purpose_limit = validate::purpose_limit(&limits, purpose);
    if staged.size > purpose_limit {
        let error = AssetError::payload_too_large(format!(
            "文件超过该用途的大小上限：{} 字节 > {purpose_limit} 字节",
            staged.size
        ));
        blob_store::discard_staged(staged).await;
        return Err(error);
    }

    // 5) 校验 + 落盘 + 元数据事务。
    let mut connection = acquire(state).await?;
    store
        .finalize(
            &mut connection,
            &UploadRequest {
                item_id,
                purpose,
                original_name,
            },
            &limits,
            staged,
        )
        .await
}

/// 解析 multipart 的 `purpose` 与 `file` 两个字段。
async fn read_parts(
    store: &AssetStore,
    limits: &Limits,
    multipart: &mut Multipart,
) -> Result<(AssetPurpose, Option<String>, Staged), AssetError> {
    let mut purpose: Option<AssetPurpose> = None;
    let mut original_name: Option<String> = None;
    let mut staged: Option<Staged> = None;

    while let Some(mut field) = multipart.next_field().await.map_err(map_multipart_error)? {
        let name = field.name().unwrap_or_default().to_owned();
        match name.as_str() {
            "purpose" => {
                if purpose.is_some() {
                    return Err(AssetError::invalid_content("purpose 字段重复"));
                }
                let value = read_small_field(&mut field, 64).await?;
                purpose = Some(upload::parse_purpose(&value)?);
            }
            "file" => {
                if staged.is_some() {
                    return Err(AssetError::invalid_content("file 字段重复"));
                }
                original_name = upload::sanitize_original_name(field.file_name());
                let limit = match purpose {
                    Some(purpose) => validate::purpose_limit(limits, purpose),
                    // 字段顺序不保证：purpose 尚未到达时用最大用途上限，读完再兜底（见调用方）。
                    None => validate::max_upload_request_bytes(limits),
                };
                let label = purpose.map(|purpose| purpose.as_str()).unwrap_or("unknown");
                let mut writer = StagedWriter::create(&store.tmp_dir(), limit, label).await?;
                while let Some(chunk) = field.chunk().await.map_err(map_multipart_error)? {
                    writer.write(&chunk).await?;
                }
                staged = Some(writer.finish().await?);
            }
            other => {
                return Err(AssetError::invalid_content(format!(
                    "未知表单字段 {other:?}：只接受 purpose 与 file"
                )));
            }
        }
    }

    let Some(purpose) = purpose else {
        if let Some(staged) = staged {
            blob_store::discard_staged(staged).await;
        }
        return Err(AssetError::invalid_content("缺少 purpose 字段"));
    };
    let Some(staged) = staged else {
        return Err(AssetError::invalid_content("缺少 file 字段"));
    };
    Ok((purpose, original_name, staged))
}

/// 读取小字段（`purpose`）：限制总长度，防止把小字段当上传通道。
async fn read_small_field(
    field: &mut axum::extract::multipart::Field<'_>,
    max_bytes: usize,
) -> Result<String, AssetError> {
    let mut buffer: Vec<u8> = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(map_multipart_error)? {
        if buffer.len() + chunk.len() > max_bytes {
            return Err(AssetError::invalid_content(format!(
                "字段超过 {max_bytes} 字节上限"
            )));
        }
        buffer.extend_from_slice(&chunk);
    }
    String::from_utf8(buffer).map_err(|_| AssetError::invalid_content("字段不是合法 UTF-8"))
}

/// multipart 读取错误 → 资产错误（状态码分类沿用 axum/multer 的判定）。
fn map_multipart_error(error: axum::extract::multipart::MultipartError) -> AssetError {
    let status = error.status();
    let detail = error.body_text();
    AssetError::Multipart {
        detail,
        payload_too_large: status == StatusCode::PAYLOAD_TOO_LARGE,
        bad_request: status == StatusCode::BAD_REQUEST,
    }
}

fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

async fn acquire(state: &AppState) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, AssetError> {
    state.database().pool().acquire().await.map_err(|error| {
        tracing::error!(error = %error, "获取数据库连接失败");
        AssetError::io("数据库暂不可用".to_owned())
    })
}

// ---------------------------------------------------------------------------
// 内容服务：GET / HEAD /assets/{id}/content
// ---------------------------------------------------------------------------

/// `GET /api/v1/assets/{id}/content`（支持 HEAD；Range 合同见 contracts.md §7）。
#[utoipa::path(
    get,
    path = "/api/v1/assets/{id}/content",
    tag = "assets",
    summary = "读取资产内容（授权、ETag、Range）",
    description = "仅资产的拥有者可读（跨物品/不存在都返回 404）。ETag 是内容 sha256 的强校验器。\
                   完整 GET 200；合法单区间 206 + Content-Range/Length；不可满足 416 + `Content-Range: bytes */N`；\
                   多区间首版回落完整 200；HEAD 与 GET 同头无 body；If-None-Match 命中 304；\
                   If-Range 不匹配返回完整 200。响应不包含磁盘路径，也不做动态压缩。",
    params(("id" = String, Path, description = "资产 ID（UUIDv7）")),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "完整内容（或 HEAD 的响应头）"),
        (status = 206, description = "单区间内容（Content-Range）"),
        (status = 304, description = "If-None-Match 命中（无 body）"),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "资产不存在或不可见", body = super::dto::ApiErrorResponse),
        (status = 416, description = "区间不可满足（Content-Range: bytes */N）", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn get_asset_content(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    asset_content_response(state, request_id, id, headers, true).await
}

/// `HEAD /api/v1/assets/{id}/content`：与 GET 相同的头、无 body。
pub async fn head_asset_content(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    asset_content_response(state, request_id, id, headers, false).await
}

async fn asset_content_response(
    state: AppState,
    request_id: RequestId,
    id: String,
    headers: HeaderMap,
    include_body: bool,
) -> Response {
    let mut connection = match acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.into_api_error().render(&request_id),
    };
    let found = match assets_repo::get_with_blob(&mut connection, &id).await {
        Ok(found) => found,
        Err(error) => {
            return ApiError::from_storage(error).render(&request_id);
        }
    };
    let Some((_asset, blob)) = found else {
        return ApiError::not_found("资产不存在或不可见").render(&request_id);
    };
    if blob.storage_state != BlobStorageState::Stored {
        // 已隔离/缺失：不提供内容，也不泄露磁盘位置。
        tracing::error!(
            requestId = %request_id,
            assetId = %id,
            storageState = blob.storage_state.as_str(),
            "资产内容不可用（存储状态非 stored）"
        );
        return ApiError::not_found("资产内容当前不可用").render(&request_id);
    }

    let data_dir = state.assets().data_dir().to_path_buf();
    let path = blob_store::blob_path(&data_dir, &blob.sha256);
    let size = blob.size.max(0) as u64;
    let etag = range::etag_for_sha256(&blob.sha256);
    let content_type = alias_json_mime(&blob.mime);

    // If-None-Match 命中 → 304（GET 与 HEAD 都是"表示未变化"）。
    if range::if_none_match_hits(header_value(&headers, header::IF_NONE_MATCH), &etag) {
        return not_modified(&etag);
    }

    // If-Range 不匹配（含日期形式）→ 忽略 Range，返回完整表示。
    let range_header = header_value(&headers, header::RANGE);
    let decision = match header_value(&headers, header::IF_RANGE) {
        Some(if_range) if !range::if_range_allows(Some(if_range), &etag) => {
            range::RangeDecision::Full
        }
        _ => match range_header {
            None => range::RangeDecision::Full,
            Some(value) => range::parse_range_header(value, size),
        },
    };

    match decision {
        range::RangeDecision::Unsatisfiable => {
            let error = ApiError::new(
                StatusCode::RANGE_NOT_SATISFIABLE,
                ApiErrorCode::ValidationFailed,
                format!("请求的字节范围不可满足：资产大小为 {size} 字节"),
            )
            .with_header("content-range", format!("bytes */{size}"));
            let mut response = error.render(&request_id);
            response.headers_mut().insert(
                header::ACCEPT_RANGES,
                axum::http::HeaderValue::from_static("bytes"),
            );
            response
                .headers_mut()
                .insert(header::ETAG, etag_header(&etag));
            response
        }
        range::RangeDecision::Partial(byte_range) => {
            if !include_body {
                match tokio::fs::metadata(&path).await {
                    Ok(_) => {}
                    Err(_) => return missing_content(&request_id, &id),
                }
                return content_headers(
                    StatusCode::PARTIAL_CONTENT,
                    &etag,
                    content_type,
                    byte_range.length(),
                    Some(format!(
                        "bytes {}-{}/{size}",
                        byte_range.start, byte_range.end
                    )),
                    Body::empty(),
                );
            }
            let mut file = match tokio::fs::File::open(&path).await {
                Ok(file) => file,
                Err(_) => return missing_content(&request_id, &id),
            };
            if let Err(error) = file.seek(SeekFrom::Start(byte_range.start)).await {
                tracing::error!(error = %error, assetId = %id, "资产内容定位失败");
                return ApiError::internal("服务器内部错误：读取资产内容失败").render(&request_id);
            }
            let limited = file.take(byte_range.length());
            let body = Body::from_stream(ReaderStream::new(limited));
            content_headers(
                StatusCode::PARTIAL_CONTENT,
                &etag,
                content_type,
                byte_range.length(),
                Some(format!(
                    "bytes {}-{}/{size}",
                    byte_range.start, byte_range.end
                )),
                body,
            )
        }
        range::RangeDecision::Full => {
            if !include_body {
                match tokio::fs::metadata(&path).await {
                    Ok(_) => {}
                    Err(_) => return missing_content(&request_id, &id),
                }
                return content_headers(
                    StatusCode::OK,
                    &etag,
                    content_type,
                    size,
                    None,
                    Body::empty(),
                );
            }
            let file = match tokio::fs::File::open(&path).await {
                Ok(file) => file,
                Err(_) => return missing_content(&request_id, &id),
            };
            let body = Body::from_stream(ReaderStream::new(file));
            content_headers(StatusCode::OK, &etag, content_type, size, None, body)
        }
    }
}

/// 元数据在库但文件缺失：404（不泄露路径），并留下服务端错误日志供排障。
fn missing_content(request_id: &RequestId, asset_id: &str) -> Response {
    tracing::error!(
        requestId = %request_id,
        assetId = %asset_id,
        "资产元数据存在但内容文件缺失（blobs 目录被外部改动？）"
    );
    ApiError::not_found("资产内容当前不可用").render(request_id)
}

fn content_headers(
    status: StatusCode,
    etag: &str,
    content_type: &'static str,
    content_length: u64,
    content_range: Option<String>,
    body: Body,
) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(content_type),
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(&content_length.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    headers.insert(header::ETAG, etag_header(etag));
    headers.insert(
        header::ACCEPT_RANGES,
        axum::http::HeaderValue::from_static("bytes"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    if let Some(content_range) = content_range
        && let Ok(value) = axum::http::HeaderValue::from_str(&content_range)
    {
        headers.insert(header::CONTENT_RANGE, value);
    }
    response
}

fn not_modified(etag: &str) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NOT_MODIFIED;
    response
        .headers_mut()
        .insert(header::ETAG, etag_header(etag));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    response
}

fn etag_header(etag: &str) -> axum::http::HeaderValue {
    axum::http::HeaderValue::from_str(etag)
        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("\"invalid\""))
}

fn header_value(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// MIME 白名单到响应头字面量（避免每请求分配；未知类型退化为 `application/octet-stream`）。
fn alias_json_mime(mime: &str) -> &'static str {
    match mime {
        "application/pdf" => "application/pdf",
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "text/plain; charset=utf-8" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
