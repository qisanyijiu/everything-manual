//! T01 引导合同集成测试：health JSON、统一错误结构、未知 `/api/*` JSON 404。
//!
//! T04 起应用需要真实应用状态（数据库 + 配置），因此本文件用 `common` 的临时
//! data-dir 装配真实路由；`/health/ready` 的断言随 T04 的数据层检查同步更新
//! （T01 计划中的扩展点：`checks` 增加 data_directory/database/migrations）。
//!
//! 该测试目标不启用 embedded-ui，因此不要求 `apps/web/dist`（见 server build.rs）。
//! 启用 `embedded-ui` 时本文件追加内嵌页面/静态资源用例（需先构建 dist）。

mod common;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestResponse};

/// 每次调用使用一个独立的临时 data-dir（测试相互隔离）。
async fn get(uri: &str) -> TestResponse {
    let app = TestApp::new("bootstrap").await;
    app.call(Method::GET, uri).send().await
}

#[tokio::test]
async fn health_live_returns_exact_minimal_json() {
    let response = get("/api/v1/health/live").await;

    assert_eq!(response.status, StatusCode::OK);
    assert!(
        response
            .header("content-type")
            .as_deref()
            .is_some_and(|ct| ct.starts_with("application/json")),
        "content-type 应为 JSON，实际 {:?}",
        response.header("content-type")
    );

    // 全量比较：探针响应不得夹带配置、版本或环境细节。
    assert_eq!(
        response.json(),
        serde_json::json!({ "data": { "status": "ok" } })
    );
}

#[tokio::test]
async fn health_ready_reports_declared_checks_only() {
    let response = get("/api/v1/health/ready").await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        serde_json::json!({
            "data": {
                "status": "ready",
                "checks": [
                    { "name": "process", "status": "ok" },
                    { "name": "data_directory", "status": "ok" },
                    { "name": "database", "status": "ok" },
                    { "name": "migrations", "status": "ok" }
                ]
            }
        }),
        "T04 就绪语义：数据层检查项齐全且不输出配置细节"
    );
}

#[tokio::test]
async fn unknown_api_path_returns_json_404_not_html() {
    let response = get("/api/unknown").await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert!(
        response
            .header("content-type")
            .as_deref()
            .is_some_and(|ct| ct.starts_with("application/json")),
        "未知 /api/* 必须 JSON 404，不能返回 index.html；实际 {:?}",
        response.header("content-type")
    );

    let body = response.json();
    assert_eq!(body["error"]["code"], "NOT_FOUND");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "错误信息不能为空"
    );
    assert!(body["error"]["details"].is_null());
    let request_id = body["error"]["requestId"].as_str().unwrap_or_default();
    assert!(
        uuid::Uuid::parse_str(request_id).is_ok(),
        "requestId 应为 UUID，实际 {request_id:?}"
    );

    // 结构上只允许 error.code/message/details/requestId 四个键。
    let keys: Vec<&String> = body["error"].as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 4, "错误体不得包含额外字段：{keys:?}");
    // requestId 与响应头一致（T04 全链路关联）。
    assert_eq!(response.header("x-request-id").as_deref(), Some(request_id));
}

#[tokio::test]
async fn unknown_versioned_api_path_also_returns_json_404() {
    // 防止 nest 前缀之外的 /api/v1/* 落到 SPA fallback。
    let response = get("/api/v1/does-not-exist").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.json()["error"]["code"], "NOT_FOUND");
}

#[cfg(not(feature = "embedded-ui"))]
#[tokio::test]
async fn non_api_unknown_path_is_not_html_200_without_embedded_ui() {
    let response = get("/library").await;
    assert_eq!(
        response.status,
        StatusCode::NOT_FOUND,
        "未启用 embedded-ui 时不应提供任何页面"
    );
    assert!(
        !response
            .header("content-type")
            .as_deref()
            .is_some_and(|ct| ct.starts_with("text/html")),
        "未内嵌前端时不得返回 HTML"
    );
}

// ---------------------------------------------------------------------------
// embedded-ui 构建的补充用例：需要先 `npm --prefix apps/web run build` 产出 dist。
// 运行：cargo test -p everything-manual --features embedded-ui --test bootstrap
// ---------------------------------------------------------------------------

#[cfg(feature = "embedded-ui")]
mod embedded_ui {
    use super::*;

    #[tokio::test]
    async fn root_serves_embedded_index_html() {
        let response = get("/").await;
        assert_eq!(response.status, StatusCode::OK);
        assert!(
            response
                .header("content-type")
                .as_deref()
                .is_some_and(|ct| ct.starts_with("text/html"))
        );
        assert!(
            response.text().contains("id=\"root\""),
            "index.html 应包含挂载点"
        );
    }

    #[tokio::test]
    async fn spa_navigation_path_returns_index_html() {
        let response = get("/library/some-item").await;
        assert_eq!(response.status, StatusCode::OK);
        assert!(
            response
                .header("content-type")
                .as_deref()
                .is_some_and(|ct| ct.starts_with("text/html"))
        );
    }

    #[tokio::test]
    async fn embedded_hashed_asset_is_served() {
        let index = get("/").await;
        let html = index.text();
        let asset_path = html
            .split('"')
            .find(|part| part.starts_with("/assets/") && part.ends_with(".js"))
            .expect("index.html 应引用 /assets/*.js")
            .to_owned();

        let asset = get(&asset_path).await;
        assert_eq!(asset.status, StatusCode::OK, "{asset_path} 应可访问");
        assert!(
            asset
                .header("content-type")
                .as_deref()
                .is_some_and(|ct| ct.starts_with("text/javascript")),
            "JS 资源 content-type 应为 text/javascript，实际 {:?}",
            asset.header("content-type")
        );
        assert!(!asset.body.is_empty());
    }

    #[tokio::test]
    async fn missing_static_asset_is_404_not_html() {
        let response = get("/assets/definitely-missing.js").await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert!(
            !response
                .header("content-type")
                .as_deref()
                .is_some_and(|ct| ct.starts_with("text/html"))
        );
    }
}
