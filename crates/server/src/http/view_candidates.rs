//! 视图候选图与一次性视图排列（ADR-044）。
//!
//! - `GET    /items/{id}/view-candidates`：未删除的候选（含建议视图、置信度、来源页）；
//! - `POST   /items/{id}/view-candidates`：登记一张候选（资产须为本物品的 `photo`）；
//!   `classify=true` 时由说明书 AI 给出建议视图（失败不阻塞，候选记为"未判断"）；
//! - `POST   /items/{id}/view-candidates/{candidateId}/dismiss` / `restore`：删除 / 撤销删除（软删除）；
//! - `PUT    /items/{id}/photos/arrangement`：按槽位一次性确定全部视图照片
//!   （拖拽交换、移出槽位、删除都在前端完成后整体提交；避免逐条 PATCH 撞唯一索引）。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use manual_core::domain::PhotoView;
use manual_core::validation::{FieldIssue, validate_photo_view};

use crate::storage::repo::{blobs as blobs_repo, photos, view_candidates as repo};

use super::dto::{PhotoDto, PhotoListResponse};
use super::error::{ApiError, RequestId};
use super::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/items/{id}/view-candidates",
            routing::get(list_candidates).post(create_candidate),
        )
        .route(
            "/items/{id}/view-candidates/{candidate_id}/dismiss",
            routing::post(dismiss_candidate),
        )
        .route(
            "/items/{id}/view-candidates/{candidate_id}/restore",
            routing::post(restore_candidate),
        )
        .route(
            "/items/{id}/photos/arrangement",
            routing::put(arrange_photos),
        )
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewCandidateDto {
    pub id: String,
    pub item_id: String,
    pub asset_id: String,
    #[schema(nullable = true)]
    pub document_id: Option<String>,
    /// 来源页（1-based）；手动上传的候选为 null。
    #[schema(nullable = true)]
    pub page_number: Option<i64>,
    /// `embedded`（PDF 内嵌位图）/ `region`（渲染页的图形区域裁剪）/ `upload`（用户上传）。
    pub source: String,
    /// 建议视图（front/left/back/right/detail）；null = 未判断或不像产品视图。
    #[schema(nullable = true, value_type = Option<String>)]
    pub suggested_view: Option<String>,
    #[schema(nullable = true)]
    pub confidence: Option<f64>,
    /// 判断理由（简短，供用户核对）。
    #[schema(nullable = true)]
    pub note: Option<String>,
    #[schema(value_type = String)]
    pub created_at: manual_core::timestamps::Timestamp,
}

impl From<repo::ViewCandidate> for ViewCandidateDto {
    fn from(c: repo::ViewCandidate) -> Self {
        Self {
            id: c.id,
            item_id: c.item_id,
            asset_id: c.asset_id,
            document_id: c.document_id,
            page_number: c.page_number,
            source: c.source,
            suggested_view: c.suggested_view.map(|v| v.as_str().to_owned()),
            confidence: c.confidence,
            note: c.note,
            created_at: c.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ViewCandidateListResponse {
    pub data: Vec<ViewCandidateDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ViewCandidateResponse {
    pub data: ViewCandidateDto,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewCandidateCreateRequest {
    /// 候选图资产（本物品、`purpose=photo`、JPEG/PNG）。
    pub asset_id: String,
    #[serde(default)]
    #[schema(nullable = true)]
    pub document_id: Option<String>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub page_number: Option<i64>,
    /// `embedded` / `region` / `upload`。
    pub source: String,
    /// 是否请说明书 AI 判断建议视图（默认 true）。
    #[serde(default)]
    #[schema(nullable = true)]
    pub classify: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhotoArrangementRequest {
    /// 每个视图要放的照片资产；省略或 null = 该视图为空。资产须为本物品 `photo`，且同一资产不能放两个视图。
    #[serde(default)]
    pub slots: std::collections::BTreeMap<String, Option<String>>,
}

#[utoipa::path(get, path = "/api/v1/items/{id}/view-candidates", tag = "photos",
    summary = "视图候选图（从说明书 PDF 拆出的待选图）",
    params(("id" = String, Path, description = "物品 ID")),
    security(("sessionCookie" = [])),
    responses((status = 200, body = ViewCandidateListResponse), (status = 404, body = super::dto::ApiErrorResponse)))]
pub async fn list_candidates(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(item_id): Path<String>,
) -> Response {
    let mut conn = match super::photos::acquire(&state).await {
        Ok(c) => c,
        Err(e) => return e.render(&request_id),
    };
    if let Err(e) = super::photos::ensure_item(&mut conn, &item_id).await {
        return e.render(&request_id);
    }
    match repo::list_active(&mut conn, &item_id).await {
        Ok(list) => Json(ViewCandidateListResponse {
            data: list.into_iter().map(Into::into).collect(),
        })
        .into_response(),
        Err(e) => ApiError::from_storage(e).render(&request_id),
    }
}

#[utoipa::path(post, path = "/api/v1/items/{id}/view-candidates", tag = "photos",
    summary = "登记一张视图候选图（可选由说明书 AI 判断建议视图）",
    description = "资产须属于本物品且为照片（JPEG/PNG）。同一资产重复登记返回已有候选（200，不重复判断）。\
                   classify 默认 true：调用已配置的说明书 AI（与提取同一服务端密钥），失败时候选记为未判断（仍 201），\
                   不阻塞用户手动排列。候选不进入报价/生成快照。",
    params(("id" = String, Path, description = "物品 ID")),
    request_body = ViewCandidateCreateRequest,
    security(("sessionCookie" = [])),
    responses((status = 201, body = ViewCandidateResponse), (status = 200, body = ViewCandidateResponse),
               (status = 404, body = super::dto::ApiErrorResponse), (status = 422, body = super::dto::ApiErrorResponse)))]
pub async fn create_candidate(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(item_id): Path<String>,
    super::body::JsonBody(body): super::body::JsonBody<ViewCandidateCreateRequest>,
) -> Response {
    if !matches!(body.source.as_str(), "embedded" | "region" | "upload") {
        return ApiError::field_validation(vec![FieldIssue::new(
            "source",
            "source 只能是 embedded / region / upload",
        )])
        .render(&request_id);
    }
    if body.page_number.is_some_and(|n| n < 1) {
        return ApiError::field_validation(vec![FieldIssue::new("pageNumber", "页码从 1 开始")])
            .render(&request_id);
    }
    let mut conn = match super::photos::acquire(&state).await {
        Ok(c) => c,
        Err(e) => return e.render(&request_id),
    };
    if let Err(e) = super::photos::ensure_item(&mut conn, &item_id).await {
        return e.render(&request_id);
    }
    let asset = match super::photos::check_photo_asset(&mut conn, &item_id, &body.asset_id).await {
        Ok(a) => a,
        Err(e) => return e.render(&request_id),
    };
    if let Ok(list) = repo::list_active(&mut conn, &item_id).await
        && let Some(existing) = list.into_iter().find(|c| c.asset_id == asset.id)
    {
        return (
            StatusCode::OK,
            Json(ViewCandidateResponse {
                data: existing.into(),
            }),
        )
            .into_response();
    }

    let (mut suggested, mut confidence, mut note) = (None, None, None);
    if body.classify.unwrap_or(true) {
        let provider = {
            state
                .provider_config()
                .read()
                .await
                .active_providers()
                .manual_ai
                .clone()
        };
        match (provider.api_key.as_ref(), provider.model.as_deref()) {
            (Some(key), Some(model)) => {
                let blob = blobs_repo::get(&mut conn, &asset.blob_id)
                    .await
                    .ok()
                    .flatten();
                let bytes = blob.as_ref().and_then(|b| {
                    std::fs::read(crate::assets::blob_store::blob_path(
                        state.assets().data_dir(),
                        &b.sha256,
                    ))
                    .ok()
                    .map(|bytes| (bytes, b.mime.clone()))
                });
                if let Some((bytes, mime)) = bytes {
                    match crate::providers::manual_ai::view_classify::classify(
                        &provider.base_url,
                        key,
                        model,
                        &bytes,
                        &mime,
                    )
                    .await
                    {
                        Ok(guess) => {
                            suggested = guess
                                .view
                                .as_deref()
                                .and_then(|v| validate_photo_view(Some(v)).ok());
                            confidence = Some(guess.confidence);
                            note = Some(if guess.is_product_view {
                                guess.reason
                            } else {
                                format!("不像产品视图：{}", guess.reason)
                            });
                        }
                        Err(reason) => {
                            tracing::warn!(requestId = %request_id, reason = %reason, "视图候选分类失败（候选保存为未判断）");
                            note = Some("自动判断失败，请手动排列".to_owned());
                        }
                    }
                }
            }
            _ => note = Some("说明书 AI 未配置，未自动判断".to_owned()),
        }
    }
    match repo::create(
        &mut conn,
        repo::NewViewCandidate {
            item_id: item_id.clone(),
            asset_id: asset.id,
            document_id: body.document_id,
            page_number: body.page_number,
            source: body.source,
            suggested_view: suggested,
            confidence,
            note: note.map(|n| n.chars().take(200).collect()),
        },
    )
    .await
    {
        Ok((candidate, created)) => {
            let status = if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            (
                status,
                Json(ViewCandidateResponse {
                    data: candidate.into(),
                }),
            )
                .into_response()
        }
        Err(e) => ApiError::from_storage(e).render(&request_id),
    }
}

async fn set_dismissed(
    state: AppState,
    request_id: RequestId,
    item_id: String,
    candidate_id: String,
    dismissed: bool,
) -> Response {
    let mut conn = match super::photos::acquire(&state).await {
        Ok(c) => c,
        Err(e) => return e.render(&request_id),
    };
    match repo::set_dismissed(&mut conn, &item_id, &candidate_id, dismissed).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => ApiError::not_found("候选不存在或不属于该物品").render(&request_id),
        Err(e) => ApiError::from_storage(e).render(&request_id),
    }
}

#[utoipa::path(post, path = "/api/v1/items/{id}/view-candidates/{candidateId}/dismiss", tag = "photos",
    summary = "删除一张候选（软删除，可撤销；不影响已排列的照片）",
    params(("id" = String, Path, description = "物品 ID"), ("candidateId" = String, Path, description = "候选 ID")),
    security(("sessionCookie" = [])),
    responses((status = 204, description = "已删除"), (status = 404, body = super::dto::ApiErrorResponse)))]
pub async fn dismiss_candidate(
    State(state): State<AppState>,
    request_id: RequestId,
    Path((item_id, candidate_id)): Path<(String, String)>,
) -> Response {
    set_dismissed(state, request_id, item_id, candidate_id, true).await
}

#[utoipa::path(post, path = "/api/v1/items/{id}/view-candidates/{candidateId}/restore", tag = "photos",
    summary = "撤销删除候选",
    params(("id" = String, Path, description = "物品 ID"), ("candidateId" = String, Path, description = "候选 ID")),
    security(("sessionCookie" = [])),
    responses((status = 204, description = "已恢复"), (status = 404, body = super::dto::ApiErrorResponse)))]
pub async fn restore_candidate(
    State(state): State<AppState>,
    request_id: RequestId,
    Path((item_id, candidate_id)): Path<(String, String)>,
) -> Response {
    set_dismissed(state, request_id, item_id, candidate_id, false).await
}

#[utoipa::path(put, path = "/api/v1/items/{id}/photos/arrangement", tag = "photos",
    summary = "一次性确定全部视图照片（拖拽排列的保存）",
    description = "slots 给出 front/left/back/right/detail 各自的照片资产（省略/null = 空）。资产须为本物品照片且不重复。\
                   在一个写事务内替换本物品的全部照片行（同资产保留原照片 id），因此互换两个视图不会撞唯一索引。\
                   已开始任务使用冻结快照，不受影响；已有报价会因输入变化失效。",
    params(("id" = String, Path, description = "物品 ID")),
    request_body = PhotoArrangementRequest,
    security(("sessionCookie" = [])),
    responses((status = 200, body = PhotoListResponse), (status = 404, body = super::dto::ApiErrorResponse), (status = 422, body = super::dto::ApiErrorResponse)))]
pub async fn arrange_photos(
    State(state): State<AppState>,
    request_id: RequestId,
    Path(item_id): Path<String>,
    super::body::JsonBody(body): super::body::JsonBody<PhotoArrangementRequest>,
) -> Response {
    let mut issues = Vec::new();
    let mut slots: Vec<(PhotoView, String)> = Vec::new();
    for (key, asset) in &body.slots {
        let Ok(view) = validate_photo_view(Some(key)) else {
            issues.push(FieldIssue::new("slots", format!("未知视图：{key}")));
            continue;
        };
        if let Some(asset) = asset {
            if slots.iter().any(|(_, a)| a == asset) {
                issues.push(FieldIssue::new("slots", "同一张图不能放在两个视图"));
            }
            slots.push((view, asset.clone()));
        }
    }
    if !issues.is_empty() {
        return ApiError::field_validation(issues).render(&request_id);
    }
    let mut conn = match super::photos::acquire(&state).await {
        Ok(c) => c,
        Err(e) => return e.render(&request_id),
    };
    if let Err(e) = super::photos::ensure_item(&mut conn, &item_id).await {
        return e.render(&request_id);
    }
    for (_, asset) in &slots {
        if let Err(e) = super::photos::check_photo_asset(&mut conn, &item_id, asset).await {
            return e.render(&request_id);
        }
    }
    let mut tx = match crate::storage::begin_write(&mut conn).await {
        Ok(tx) => tx,
        Err(_) => return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id),
    };
    let arranged = match photos::arrange(&mut tx, &item_id, &slots).await {
        Ok(list) => list,
        Err(e) => return ApiError::from_storage(e).render(&request_id),
    };
    if tx.commit().await.is_err() {
        return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
    }
    tracing::info!(requestId = %request_id, itemId = %item_id, slots = arranged.len(), "视图排列已保存");
    let mut data: Vec<PhotoDto> = arranged.into_iter().map(Into::into).collect();
    let order = |view: &PhotoView| {
        ["front", "left", "back", "right", "detail"]
            .iter()
            .position(|v| *v == view.as_str())
            .unwrap_or(9)
    };
    data.sort_by_key(|photo| order(&photo.view));
    Json(PhotoListResponse {
        data,
        next_cursor: None,
    })
    .into_response()
}
