//! 发布版本读取与导出自包含包（T19/T20 / REQ-035、REQ-036、REQ-037；contracts.md §3）。
//!
//! - `GET /items/{id}/releases`：版本列表（发布时间倒序；已发布内容不可变）；
//! - `GET /items/{id}/releases/{releaseId}`：完整 manifest（冻结知识 + 复核声明 +
//!   资产 sha256 与来源 + 说明书原件引用），供阅读器按 release 展示四者联动；
//! - `GET /releases/{releaseId}/export`：下载自包含包（原件／GLB／manifest／哈希；
//!   只导出该 release 有权资产，不含密钥、会话、绝对路径与临时云端 URL；T20）。
//!
//! 授权：会话保护；跨物品 release 按不存在处理（不泄露存在性）。发布动作本身在
//! `http::drafts::publish_draft`（同一路由组，见 router 组装）。

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, routing};
use tokio_util::io::ReaderStream;

use crate::backup::export as export_service;
use crate::releases::service as releases_service;
use crate::storage::repo;

use super::dto::{ReleaseDetailDto, ReleaseDetailResponse, ReleaseDto, ReleaseListResponse};
use super::error::{ApiError, RequestId};
use super::state::AppState;

/// 受会话保护的发布版本路由。
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/items/{id}/releases", routing::get(list_releases))
        .route(
            "/items/{id}/releases/{release_id}",
            routing::get(get_release),
        )
        // 导出路由在契约里不带 item 前缀（contracts §3：`GET /releases/{releaseId}/export`）。
        .route(
            "/releases/{release_id}/export",
            routing::get(export_release),
        )
}

/// 版本列表的默认上限（MVP 单物品发布次数有限；排序由服务端给定）。
const RELEASE_LIST_LIMIT: i64 = 100;

/// `GET /api/v1/items/{id}/releases`。
#[utoipa::path(
    get,
    path = "/api/v1/items/{id}/releases",
    tag = "releases",
    summary = "发布版本列表（发布时间倒序）",
    description = "返回该物品的不可变发布版本（`draftRevision` / `modelRevisionId` / \
                   manifest 资产）。已发布内容不可修改；列表不返回 manifest 内容，\
                   完整 manifest 用 `GET /items/{id}/releases/{releaseId}`。",
    params(("id" = String, Path, description = "物品 ID（UUIDv7）")),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "版本列表", body = ReleaseListResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn list_releases(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(item_id): Path<String>,
) -> Response {
    let mut connection = match super::estimates::acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    match releases_service::list_releases(&mut connection, &item_id, RELEASE_LIST_LIMIT).await {
        Ok(releases) => {
            let data = releases
                .iter()
                .map(|release| ReleaseDto::from(release, None))
                .collect();
            (
                StatusCode::OK,
                Json(ReleaseListResponse {
                    data,
                    next_cursor: None,
                }),
            )
                .into_response()
        }
        Err(error) => ApiError::from(error).render(&request_id),
    }
}

/// `GET /api/v1/items/{id}/releases/{releaseId}`。
#[utoipa::path(
    get,
    path = "/api/v1/items/{id}/releases/{releaseId}",
    tag = "releases",
    summary = "读取发布版本与完整 manifest（不可变）",
    description = "返回被冻结的 manifest：`model`（本地资产 assetId / sha256 / revision）、\
                   `knowledge`（部件/步骤/规格/出处/热点/步骤视角）、`review`（实体确认、\
                   人工修订、modelReview 声明）、`assets`（引用资产的 sha256 与来源）、\
                   `documents`（说明书原件与准备）。`manifestSha256` 供字节比对：\
                   发布后修改草稿不会改变已发布版本。",
    params(
        ("id" = String, Path, description = "物品 ID（UUIDv7）"),
        ("releaseId" = String, Path, description = "发布版本 ID（UUIDv7）"),
    ),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "发布版本与 manifest", body = ReleaseDetailResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "发布版本不存在或不属于该物品", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn get_release(
    State(state): State<AppState>,
    request_id: RequestId,
    Path((item_id, release_id)): Path<(String, String)>,
) -> Response {
    let mut connection = match super::estimates::acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let release = match releases_service::read_release(&mut connection, &item_id, &release_id).await
    {
        Ok(release) => release,
        Err(error) => return ApiError::from(error).render(&request_id),
    };
    let data_dir = state.settings().data_dir.clone();
    match releases_service::read_manifest(&mut connection, &data_dir, &release).await {
        Ok((manifest, sha256)) => (
            StatusCode::OK,
            Json(ReleaseDetailResponse {
                data: ReleaseDetailDto::from_detail(&release, sha256, manifest),
            }),
        )
            .into_response(),
        Err(error) => ApiError::from(error).render(&request_id),
    }
}

/// `GET /api/v1/releases/{releaseId}/export`（T20 / REQ-037、AC-058）。
///
/// 返回 `application/zip` 附件：导出清单 + 冻结 release manifest + 原件（PDF）+
/// GLB；只导出来自该 release 冻结 manifest 的资产（不导出照片、页图、其它物品或
/// data-dir 的其它内容），不含密钥、会话、绝对路径与临时云端 URL。
///
/// 包先生成到 `<data-dir>/tmp/` 再流式响应（大模型不进内存）；响应结束或客户端
/// 断开时临时文件被删除。失败语义：release 不存在 → 404；服务端数据损坏/资产缺失
/// → 500（真实原因只进服务端日志，不返回内部细节）。
#[utoipa::path(
    get,
    path = "/api/v1/releases/{releaseId}/export",
    tag = "releases",
    summary = "导出发布版自包含包（ZIP：原件 / GLB / manifest / 哈希）",
    description = "下载该发布版本的自包含包：`manifest.json`（schemaVersion、item、release、\
                   知识、相对资产清单与 sha256/来源）、`release/manifest.json`（冻结清单，\
                   字节原样）、`assets/model/<sha256>.glb`、`assets/document/<sha256>.pdf`。\
                   只导出该 release 有权资产；不含密钥、会话、绝对路径与临时云端 URL；\
                   不承诺可直接双击运行网站（数据便携与灾备）。",
    params(("releaseId" = String, Path, description = "发布版本 ID（UUIDv7）")),
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "ZIP 自包含包（Content-Disposition: attachment）",
             content_type = "application/zip", body = String),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "发布版本不存在", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn export_release(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(release_id): Path<String>,
) -> Response {
    let mut connection = match super::estimates::acquire(&state).await {
        Ok(connection) => connection,
        Err(error) => return error.render(&request_id),
    };
    let release = match repo::releases::get(&mut connection, &release_id).await {
        Ok(Some(release)) => release,
        Ok(None) => {
            return ApiError::not_found("发布版本不存在").render(&request_id);
        }
        Err(error) => {
            tracing::error!(error = %error, releaseId = %release_id, "读取发布版本失败");
            return ApiError::internal("服务器内部错误：读取发布版本失败").render(&request_id);
        }
    };
    let data_dir = state.settings().data_dir.clone();
    let package =
        match export_service::build_release_export(&mut connection, &data_dir, &release).await {
            Ok(package) => package,
            Err(export_service::ExportError::NotFound { message }) => {
                return ApiError::not_found(message).render(&request_id);
            }
            Err(error) => {
                // 真实原因（含完整性错误码）只进服务端日志；响应不泄露内部路径与细节。
                tracing::error!(
                    requestId = %request_id,
                    releaseId = %release_id,
                    code = error.code(),
                    error = %error,
                    "导出发布版失败"
                );
                return ApiError::internal("服务器内部错误：导出包生成失败").render(&request_id);
            }
        };

    let reader = match package.open_reader().await {
        Ok(reader) => reader,
        Err(error) => {
            tracing::error!(
                requestId = %request_id,
                releaseId = %release_id,
                error = %error,
                "打开导出包失败"
            );
            return ApiError::internal("服务器内部错误：导出包不可读").render(&request_id);
        }
    };

    let mut response = Response::new(Body::from_stream(ReaderStream::new(reader)));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(export_service::EXPORT_CONTENT_TYPE),
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(&package.size.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    // 文件名只由 release id 派生（服务端生成的 UUID），不含任何用户输入。
    if let Ok(value) = axum::http::HeaderValue::from_str(&format!(
        "attachment; filename=\"release-{release_id}.zip\""
    )) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    tracing::info!(
        requestId = %request_id,
        releaseId = %release_id,
        packageBytes = package.size,
        packageSha256 = %package.sha256,
        entries = package.entries.len(),
        "发布版导出包已响应（流式；结束后清理临时文件）"
    );
    response
}
