//! 路由组装（T01 骨架；T04 接入应用状态、认证与设置/物品路由）。
//!
//! 结构：
//! - **公开**：`GET /health/live`、`GET /health/ready`、`POST /auth/login`（登录在
//!   handler 内做 Origin 检查与限速，不需要会话）；
//! - **受会话保护**（统一挂 [`auth::auth_guard`]）：`/auth/session`、`/auth/logout`、
//!   `/settings/status`、`/items*`；守卫同时承担所有修改请求的 CSRF + Origin 检查；
//! - **未知 `/api/*` 一律 JSON 404**，绝不落到 SPA fallback；SPA fallback 只服务 HTML 导航。

use axum::Router;
use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::http::Request;
use axum::middleware;
#[cfg(not(feature = "embedded-ui"))]
use axum::response::IntoResponse;
use axum::response::Response;

use super::error::ApiError;
use super::logging::request_logging;
use super::state::AppState;
use super::{
    assets, auth, documents, drafts, estimates, health, items, jobs, photos, preparations,
    releases, settings,
};

/// 组装应用路由。
///
/// 中间件顺序（从外到内）：请求日志（requestId/耗时/状态码）→ 会话守卫
/// （认证 + 修改请求的 CSRF/Origin）→ handler。JSON 请求体上限由配置决定。
pub fn build_app(state: AppState) -> Router {
    let body_limit = state.settings().limits.max_json_request_bytes as usize;

    let public = Router::new()
        .merge(health::routes())
        .merge(auth::public_routes());
    let protected = Router::new()
        .merge(auth::protected_routes())
        .merge(settings::routes())
        .merge(items::routes())
        // T07：说明书绑定与多视图照片（物品子资源）。
        .merge(documents::routes())
        .merge(photos::routes())
        // T09：PDF 准备的创建/读取/逐页上传/封存（REQ-014、REQ-015）。
        .merge(preparations::routes())
        // T11：报价与云端发送确认（REQ-020、REQ-021）＋ 冻结/预留/幂等建单（REQ-022）。
        .merge(estimates::routes())
        // T15：任务读取与控制（REQ-025/026/031）＋ 草稿读取与受限复核写入（REQ-030）。
        .merge(jobs::routes())
        .merge(drafts::routes())
        // T19：发布版本列表与完整 manifest 读取（发布动作在 drafts 的 publish 路由）。
        .merge(releases::routes())
        // T06：上传路由自带更大的 `DefaultBodyLimit`（覆盖下面的 JSON 上限），
        // 内容路由受同一上限保护（无请求体，无实际影响）。
        .merge(assets::routes(&state.settings().limits))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_guard,
        ));

    let api_v1 = public
        .merge(protected)
        .layer(DefaultBodyLimit::max(body_limit));

    // 前缀来自 core 的唯一常量（utoipa 注解中的字面量由 contracts --check 防漂移）。
    Router::new()
        .nest(manual_core::API_PREFIX, api_v1)
        .fallback(fallback)
        .layer(middleware::from_fn(request_logging))
        .with_state(state)
}

/// 根 fallback：
/// - `/api` 与 `/api/*` 的未知路径 → JSON 404（不能返回 index.html）；
/// - 其余路径在 embedded-ui 构建下走 SPA 静态资源；未启用时返回纯文本 404。
async fn fallback(req: Request<Body>) -> Response {
    let path = req.uri().path().to_owned();
    if path == "/api" || path.starts_with("/api/") {
        // requestId 与响应头/请求日志同源（取中间件放入的扩展）。
        let request_id = req
            .extensions()
            .get::<super::error::RequestId>()
            .cloned()
            .unwrap_or_else(|| super::error::RequestId(uuid::Uuid::now_v7().to_string()));
        return ApiError::not_found("接口不存在").render(&request_id);
    }
    non_api_fallback(&req)
}

/// 非 `/api` 未知路径的处理按 feature 分派（feature 关闭时不提供任何页面）。
#[cfg(feature = "embedded-ui")]
fn non_api_fallback(req: &Request<Body>) -> Response {
    super::embedded::response_for(req)
}

#[cfg(not(feature = "embedded-ui"))]
fn non_api_fallback(_req: &Request<Body>) -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        "本构建未启用 embedded-ui（前端未内嵌）；开发期请访问 Vite 提供的 127.0.0.1:5173。\n",
    )
        .into_response()
}
