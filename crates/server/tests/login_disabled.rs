//! 免登录模式（`session.login_required = false`，默认）：
//! - 读取请求在没有 cookie 时自动获得唯一管理员的会话（Set-Cookie + CSRF token）；
//! - 修改请求仍需 CSRF token（没有 token → 403），跨站 Origin → 403；
//! - 拿到 cookie + token 后修改请求正常；
//! - 尚未 init（无管理员）时仍是 401。

mod common;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestDir, test_settings};
use serde_json::json;

async fn app() -> TestApp {
    let dir = TestDir::new("login-disabled");
    let mut settings = test_settings(dir.path());
    settings.session.login_required = false;
    TestApp::with_settings(dir, settings).await
}

#[tokio::test]
async fn reads_get_an_automatic_session_and_writes_keep_csrf() {
    let app = app().await;
    app.set_admin_password("unused-password-login-disabled")
        .await;

    let session = app.call(Method::GET, "/api/v1/auth/session").send().await;
    assert_eq!(session.status, StatusCode::OK, "{}", session.text());
    let cookie = session.session_cookie();
    let body = session.json();
    assert_eq!(body["data"]["loginRequired"], false);
    let csrf = body["data"]["csrfToken"].as_str().unwrap().to_owned();

    // 无 cookie 的读取也直接可用。
    let items = app.call(Method::GET, "/api/v1/items").send().await;
    assert_eq!(items.status, StatusCode::OK, "{}", items.text());

    // 无 cookie 的修改请求：没有 CSRF token → 403。
    let blind = app
        .call(Method::POST, "/api/v1/items")
        .json(&json!({ "name": "相机", "model": "M1" }))
        .send()
        .await;
    blind.assert_contract_error(StatusCode::FORBIDDEN, "CSRF_REJECTED");
    // 跨站 Origin → 403。
    let cross = app
        .call(Method::POST, "/api/v1/items")
        .cookie(&cookie)
        .csrf(&csrf)
        .origin("https://evil.example")
        .json(&json!({ "name": "相机", "model": "M1" }))
        .send()
        .await;
    assert_eq!(cross.status, StatusCode::FORBIDDEN);

    // cookie + token：正常写入。
    let created = app
        .call(Method::POST, "/api/v1/items")
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({ "name": "相机", "model": "M1" }))
        .send()
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
}

#[tokio::test]
async fn without_admin_it_is_still_unauthorized() {
    let app = app().await;
    let session = app.call(Method::GET, "/api/v1/auth/session").send().await;
    assert_eq!(session.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_required_mode_still_rejects_anonymous_reads() {
    let app = TestApp::new("login-required").await;
    app.set_admin_password("login-required-password").await;
    let session = app.call(Method::GET, "/api/v1/auth/session").send().await;
    assert_eq!(session.status, StatusCode::UNAUTHORIZED);
}
