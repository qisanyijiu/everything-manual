//! `GET /api/v1/settings/status` —— 配置状态与能力（REQ-007、AC-012）。
//!
//! **只返回状态，不返回配置内容**：不包含密钥、密钥来源（环境变量名/文件路径）、
//! 监听地址、data-dir、供应商 base_url 或任何完整配置。未配置时如实为 `false`，
//! 不存在 mock 回退（ADR-007 / REQ-007）。

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
    Router::new().route("/settings/status", routing::get(status))
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
