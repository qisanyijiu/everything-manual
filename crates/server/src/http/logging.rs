//! 请求日志中间件（T02 最小版；T04 起 requestId 全链路关联）。
//!
//! 每个请求生成 `requestId`（UUIDv7），**放入请求扩展**（[`RequestId`]，handler 用它
//! 构造错误响应的 `error.requestId`），写入响应头 `x-request-id`，并记录一条结构化日志：
//! `requestId / method / path / status / errorCode / durationMs`。
//!
//! 隐私约束（PRD §5.7）：**只记录 path，不记录查询串**——签名 URL 的凭据在查询串中；
//! 也不记录请求体（可能包含原始资料与密码）。

use std::time::Instant;

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

use super::error::{RequestId, ResponseErrorCode};

/// axum 中间件：请求 ID + 结构化访问日志。
pub async fn request_logging(mut request: Request, next: Next) -> Response {
    let request_id = RequestId(Uuid::now_v7().to_string());
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = Instant::now();
    // handler 与内层中间件（认证）从这里取同一个 requestId 写入错误响应体。
    request.extensions_mut().insert(request_id.clone());

    let mut response = next.run(request).await;

    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let status = response.status().as_u16();
    let error_code = response
        .extensions()
        .get::<ResponseErrorCode>()
        .map(|marker| marker.0.as_str());
    if let Ok(value) = HeaderValue::from_str(&request_id.0) {
        response.headers_mut().insert("x-request-id", value);
    }

    tracing::info!(
        requestId = %request_id.0,
        method = %method,
        path = %path,
        status,
        errorCode = error_code.unwrap_or("-"),
        durationMs = duration_ms,
        "http_request"
    );

    response
}
