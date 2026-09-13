//! embedded-ui：把 `apps/web/dist` 编译进二进制（本地单文件发布路径）。
//!
//! 该模块只在 `embedded-ui` feature 下编译；`build.rs` 已保证 dist 存在，
//! 普通测试构建不启用该 feature、不要求 dist。
//!
//! 服务规则（architecture.md §7）：
//! - 无扩展名的路径按 SPA 导航处理，返回 index.html（深链接刷新）；
//! - 带扩展名的路径只按静态资源查找，未命中返回 404，绝不返回 HTML 200；
//! - `/api/*` 的 404 在 router fallback 中已拦截，不会进入本模块。

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// Vite 构建产物目录（相对本 crate 的 Cargo.toml）。
#[derive(RustEmbed)]
#[folder = "../../apps/web/dist"]
struct WebDist;

const INDEX_FILE: &str = "index.html";

/// 单一路径入口：只接受 GET/HEAD。
pub fn response_for(req: &Request<Body>) -> Response {
    if req.method() != axum::http::Method::GET && req.method() != axum::http::Method::HEAD {
        return (StatusCode::METHOD_NOT_ALLOWED, "仅支持 GET/HEAD\n").into_response();
    }

    let path = req.uri().path().trim_start_matches('/');
    if path.is_empty() {
        return index_response();
    }
    if is_asset_path(path) {
        return match WebDist::get(path) {
            Some(file) => file_response(path, file.data.into_owned()),
            None => (StatusCode::NOT_FOUND, "资源不存在\n").into_response(),
        };
    }
    index_response()
}

/// 末段带 `.` 视为静态资源路径（如 `/assets/index-abc.js`）；否则按导航处理。
fn is_asset_path(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'))
}

fn index_response() -> Response {
    match WebDist::get(INDEX_FILE) {
        Some(file) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from(file.data.into_owned()))
            .expect("index.html 响应构造失败"),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "内嵌资源损坏：缺少 index.html\n",
        )
            .into_response(),
    }
}

fn file_response(path: &str, bytes: Vec<u8>) -> Response {
    // Vite 的 /assets/* 文件名带内容哈希，可长缓存；其余（favicon 等）不缓存。
    let cache_control = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type_for(path))
        .header(header::CACHE_CONTROL, cache_control)
        .body(Body::from(bytes))
        .expect("静态资源响应构造失败")
}

fn content_type_for(path: &str) -> &'static str {
    let extension = path.rsplit('.').next().unwrap_or_default();
    match extension {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
