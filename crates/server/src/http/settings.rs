//! 设置：旧 status 保留运行配置状态语义；providers 提供受保护的网页配置。
//! providers 读取仅含地址/模型/来源类别/密钥存在状态，写入只保存私有覆盖、重启生效。
//! 不返回密钥、环境变量名或路径；保存不发起供应商请求（AS-01）。

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing};

use super::dto::{
    CapabilitiesStatus, LimitsStatus, PriceCatalogStatus, ProvidersConfigured, SettingsStatusData,
    SettingsStatusResponse,
};
use super::state::AppState;

/// 受会话保护的设置路由。
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/settings/status", routing::get(status))
        .route(
            "/settings/providers",
            routing::get(get_providers).put(put_providers),
        )
}

/// `GET /api/v1/settings/status`
#[utoipa::path(
    get,
    path = "/api/v1/settings/status",
    tag = "settings",
    summary = "配置状态与能力",
    description = "返回 providersConfigured / limits / capabilities。**不含密钥或完整配置**；\
                   未配置的供应商如实为 false（不回落 mock）。",
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "配置状态", body = SettingsStatusResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn status(State(state): State<AppState>) -> Response {
    let settings = state.settings();
    let price_catalog = match settings.price_catalog.as_ref() {
        Some(catalog) => PriceCatalogStatus {
            configured: true,
            version: Some(catalog.version.clone()),
            snapshot_date: Some(catalog.snapshot_date.clone()),
        },
        None => PriceCatalogStatus {
            configured: false,
            version: None,
            snapshot_date: None,
        },
    };
    let data = SettingsStatusData {
        provider_config_pending: state.provider_config().read().await.pending(),
        providers_configured: ProvidersConfigured {
            tripo: settings.providers.tripo.configured(),
            manual_ai: settings.providers.manual_ai.configured(),
        },
        price_catalog,
        limits: LimitsStatus {
            max_json_request_bytes: settings.limits.max_json_request_bytes,
            max_pdf_bytes: settings.limits.max_pdf_bytes,
            max_pdf_pages: settings.limits.max_pdf_pages,
            max_photo_bytes: settings.limits.max_photo_bytes,
            max_glb_bytes: settings.limits.max_glb_bytes,
            max_item_total_bytes: settings.limits.max_item_total_bytes,
        },
        capabilities: CapabilitiesStatus {
            // T11 起：生成需要两个 Provider 都可用 + 价格目录在场（否则 409 明确报缺项）。
            generation: settings.providers.tripo.configured()
                && settings.providers.manual_ai.configured()
                && settings.price_catalog.is_some(),
        },
    };
    Json(SettingsStatusResponse { data }).into_response()
}

#[utoipa::path(get, path = "/api/v1/settings/providers", tag = "settings", security(("sessionCookie" = [])),
    responses((status = 200, description = "当前与已保存供应商配置；无密钥，no-store", body = super::dto::ProviderSettingsResponse),
    (status = 401, description = "未登录", body = super::dto::ApiErrorResponse)))]
pub async fn get_providers(State(state): State<AppState>) -> Response {
    provider_response(state.provider_config().read().await.view())
}

#[utoipa::path(put, path = "/api/v1/settings/providers", tag = "settings", security(("sessionCookie" = [])),
    request_body = super::dto::ProviderSettingsWrite,
    responses((status = 200, description = "整体保存；重启生效，无任何供应商请求", body = super::dto::ProviderSettingsResponse),
    (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
    (status = 403, description = "CSRF/Origin 校验失败", body = super::dto::ApiErrorResponse),
    (status = 409, description = "配置修订冲突", body = super::dto::ApiErrorResponse),
    (status = 422, description = "字段无效或尚有未决任务", body = super::dto::ApiErrorResponse),
    (status = 500, description = "持久化失败", body = super::dto::ApiErrorResponse)))]
pub async fn put_providers(
    State(state): State<AppState>,
    request_id: super::error::RequestId,
    body: Result<Json<super::dto::ProviderSettingsWrite>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let body = match body {
        Ok(Json(body)) => body,
        Err(error) => {
            // serde 的 unknown variant/field 诊断会引用原输入；秘密入口只返回固定结构错误。
            let error = match error.status() {
                axum::http::StatusCode::PAYLOAD_TOO_LARGE => {
                    super::error::ApiError::payload_too_large()
                }
                axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE => super::error::ApiError::new(
                    axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    manual_core::ApiErrorCode::UnsupportedMediaType,
                    "请求需要 application/json",
                ),
                _ => super::error::ApiError::validation_failed(
                    "API 配置请求结构无效，请检查字段与操作类型",
                ),
            };
            return error
                .with_header("cache-control", "no-store")
                .render(&request_id);
        }
    };
    let mut config = state.provider_config().write().await;
    match config.save(body, state.database().pool()).await {
        Ok(()) => provider_response(config.view()),
        Err(error) => error
            .with_header("cache-control", "no-store")
            .render(&request_id),
    }
}
fn provider_response(data: super::dto::ProviderSettingsData) -> Response {
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(super::dto::ProviderSettingsResponse { data }),
    )
        .into_response()
}
