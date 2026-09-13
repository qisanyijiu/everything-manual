//! T04 集成测试：认证、会话、CSRF/Origin、登录限速、统一错误与 requestId、
//! If-Match 乐观锁、`/settings/status`、`/health/ready` 数据层语义（PRD 修订 1）。
//!
//! 覆盖的验收条件：
//! - AC-003：未登录 401（含 requestId）、登录 cookie 属性与 CSRF token、注销后旧 cookie 401、
//!   `GET /auth/session` 带 `Cache-Control: no-store`；
//! - AC-004：无 CSRF/跨站 Origin → 403；错误密码连续 → 429；缺 If-Match → 428；
//!   过期 revision → 412 且 `details.currentRevision`；错误响应不泄露堆栈或 SQL；
//! - AC-012（`/settings/status` 侧）：未配置时 `providersConfigured=false` 且无密钥泄露；
//! - AC-066（错误结构侧）：错误体固定四键、`requestId` 与响应头一致；
//! - T04 卡：`/api/unknown` JSON 404、`/health/ready` 反映 DB/迁移/data-dir 真实状态。
//!
//! 全部用例使用临时 data-dir + 真实 SQLite；无任何网络调用；测试用假凭据。

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{TestApp, cidr, configured_tripo, test_settings};

const PASSWORD: &str = "test-password-9f3a1c";
const CANARY_KEY: &str = "sk-canary-9f3a1c-not-a-real-key";

/// 登录并返回 `(cookie, csrfToken)`。
async fn login(app: &TestApp, password: &str) -> (String, String) {
    let response = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&serde_json::json!({ "password": password }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let csrf = response.json()["data"]["csrfToken"]
        .as_str()
        .expect("登录响应包含 csrfToken")
        .to_owned();
    (response.session_cookie(), csrf)
}

#[tokio::test]
async fn unauthenticated_protected_routes_return_401_with_request_id() {
    let app = TestApp::new("unauth").await;
    app.set_admin_password(PASSWORD).await;

    for uri in [
        "/api/v1/items",
        "/api/v1/settings/status",
        "/api/v1/auth/session",
    ] {
        let response = app.call(Method::GET, uri).send().await;
        response.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
        let message = response.json()["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(!message.is_empty());
        // 不泄露堆栈/SQL。
        assert!(!message.contains("SELECT"), "{message}");
    }

    // 未登录的修改请求同样 401（先于 CSRF 检查）。
    let patch = app
        .call(Method::PATCH, "/api/v1/items/whatever")
        .json(&serde_json::json!({ "name": "x" }))
        .send()
        .await;
    patch.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
}

#[tokio::test]
async fn login_sets_httponly_strict_cookie_and_session_has_no_store() {
    let app = TestApp::new("login").await;
    app.set_admin_password(PASSWORD).await;

    let response = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&serde_json::json!({ "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.header("cache-control").as_deref(),
        Some("no-store"),
        "登录响应含 CSRF token，必须 no-store"
    );

    let cookies = response.set_cookies();
    let session_cookie = cookies
        .iter()
        .find(|cookie| cookie.starts_with("em_session="))
        .expect("必须设置会话 cookie");
    assert!(session_cookie.contains("HttpOnly"), "{session_cookie}");
    assert!(
        session_cookie.contains("SameSite=Strict"),
        "{session_cookie}"
    );
    assert!(session_cookie.contains("Path=/"), "{session_cookie}");
    assert!(
        session_cookie.contains("Max-Age=604800"),
        "默认 7 天绝对有效期（A-05）：{session_cookie}"
    );
    assert!(
        !session_cookie.contains("Secure"),
        "明文 HTTP（loopback 默认）下不得加 Secure：{session_cookie}"
    );
    assert!(
        !session_cookie.contains("Domain="),
        "cookie 不得设置 Domain：{session_cookie}"
    );

    // 响应体：csrfToken + admin id + expiresAt（RFC3339）；不得包含会话 token 明文。
    let body = response.json();
    let csrf = body["data"]["csrfToken"].as_str().unwrap_or_default();
    assert_eq!(csrf.len(), 64, "CSRF token 应为 32 字节 hex");
    let token = session_cookie
        .trim_start_matches("em_session=")
        .split(';')
        .next()
        .unwrap();
    assert!(!csrf.contains(token) && !response.text().contains(token));
    assert!(body["data"]["admin"]["id"].as_str().is_some());
    let expires = body["data"]["expiresAt"].as_str().unwrap_or_default();
    assert!(
        manual_core::timestamps::Timestamp::from_rfc3339(expires).is_ok(),
        "expiresAt 必须是 RFC3339：{expires}"
    );

    // AC-003：`GET /auth/session` 带 same cookie 恢复，且 Cache-Control: no-store。
    let cookie = response.session_cookie();
    let session = app
        .call(Method::GET, "/api/v1/auth/session")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(session.status, StatusCode::OK, "{}", session.text());
    assert_eq!(session.header("cache-control").as_deref(), Some("no-store"));
    assert_eq!(
        session.json()["data"]["csrfToken"],
        csrf,
        "同一会话的 CSRF 必须稳定"
    );
}

#[tokio::test]
async fn https_declared_origin_marks_cookie_secure() {
    // 通过 public_origin 显式声明 HTTPS 部署（唯一不依赖未验证转发头的判定依据）。
    let dir = common::TestDir::new("secure-cookie");
    let mut settings = test_settings(dir.path());
    settings.public_origin = Some("https://manual.example".to_owned());
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;

    let response = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&serde_json::json!({ "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let cookie = response
        .set_cookies()
        .into_iter()
        .find(|cookie| cookie.starts_with("em_session="))
        .expect("会话 cookie");
    assert!(
        cookie.contains("; Secure"),
        "HTTPS 判定下必须加 Secure：{cookie}"
    );

    // 跨站 Origin 在显式 public_origin 下被拒（登录本身也做 Origin 检查）。
    let cross_site = app
        .call(Method::POST, "/api/v1/auth/login")
        .origin("https://attacker.example")
        .json(&serde_json::json!({ "password": PASSWORD }))
        .send()
        .await;
    cross_site.assert_contract_error(StatusCode::FORBIDDEN, "ORIGIN_REJECTED");
}

#[tokio::test]
async fn wrong_password_is_401_and_repeated_failures_are_429() {
    let app = TestApp::new("rate-limit").await;
    app.set_admin_password(PASSWORD).await;

    for _ in 0..5 {
        let response = app
            .call(Method::POST, "/api/v1/auth/login")
            .json(&serde_json::json!({ "password": "wrong-password" }))
            .send()
            .await;
        response.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
        assert!(
            !response.text().contains("wrong-password"),
            "错误响应不得回显密码"
        );
    }

    // 第 6 次：即使密码正确也在限速窗口内 → 429 + Retry-After。
    let limited = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&serde_json::json!({ "password": PASSWORD }))
        .send()
        .await;
    limited.assert_contract_error(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED");
    let retry_after: u64 = limited
        .header("retry-after")
        .expect("429 必须带 Retry-After")
        .parse()
        .expect("Retry-After 是整数秒");
    assert!((1..=60).contains(&retry_after), "retry_after={retry_after}");
}

#[tokio::test]
async fn rate_limit_window_expires_and_success_resets_counter() {
    // 短窗口应用：窗口内 1 次失败即限速；窗口过后自动重置；成功登录清零。
    let dir = common::TestDir::new("rate-window");
    let mut settings = test_settings(dir.path());
    settings.session.login_rate_limit_per_minute = 1;
    settings.session.login_rate_limit_window_seconds = 1;
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;

    let first = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&serde_json::json!({ "password": "nope" }))
        .send()
        .await;
    first.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");

    let blocked = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&serde_json::json!({ "password": PASSWORD }))
        .send()
        .await;
    blocked.assert_contract_error(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED");

    // 窗口（1 秒）过后恢复；成功登录再清零。
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let (cookie, _csrf) = login(&app, PASSWORD).await;
    assert!(cookie.starts_with("em_session="));

    let again = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&serde_json::json!({ "password": "nope" }))
        .send()
        .await;
    again.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
    let after_failure = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&serde_json::json!({ "password": PASSWORD }))
        .send()
        .await;
    after_failure.assert_contract_error(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED");
    // 再次等待窗口过后登录成功（成功清零后立刻可再登录）。
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let (_, _) = login(&app, PASSWORD).await;
}

#[tokio::test]
async fn mutating_requests_require_csrf_and_same_site_origin() {
    let app = TestApp::new("csrf-origin").await;
    let admin_id = app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app, PASSWORD).await;
    let item = create_item(&app, "示例物品").await;

    // 1) 缺 CSRF → 403（带 cookie 但无 X-CSRF-Token）。
    let missing = app
        .call(Method::PATCH, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .header("if-match", "\"r1\"")
        .json(&serde_json::json!({ "name": "改名" }))
        .send()
        .await;
    missing.assert_contract_error(StatusCode::FORBIDDEN, "CSRF_REJECTED");

    // 2) 跨站 Origin → 403（带正确 CSRF 也不行）。
    let cross_site = app
        .call(Method::PATCH, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .csrf(&csrf)
        .origin("https://attacker.example")
        .header("if-match", "\"r1\"")
        .json(&serde_json::json!({ "name": "改名" }))
        .send()
        .await;
    cross_site.assert_contract_error(StatusCode::FORBIDDEN, "ORIGIN_REJECTED");

    // 3) 同源 Origin + 正确 CSRF → 成功。
    let ok = app
        .call(Method::PATCH, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .csrf(&csrf)
        .origin("http://127.0.0.1:8080")
        .header("if-match", "\"r1\"")
        .json(&serde_json::json!({ "name": "改名成功" }))
        .send()
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());
    assert_eq!(ok.json()["data"]["name"], "改名成功");

    // 4) 注销也需要 CSRF（POST 修改请求）——缺失即 403，且会话保持有效。
    let logout_without_csrf = app
        .call(Method::POST, "/api/v1/auth/logout")
        .cookie(&cookie)
        .send()
        .await;
    logout_without_csrf.assert_contract_error(StatusCode::FORBIDDEN, "CSRF_REJECTED");
    let session_still_valid = app
        .call(Method::GET, "/api/v1/auth/session")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(session_still_valid.status, StatusCode::OK);

    let _ = admin_id;
}

#[tokio::test]
async fn if_match_missing_is_428_and_stale_revision_is_412_with_current_revision() {
    let app = TestApp::new("if-match").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app, PASSWORD).await;
    let item = create_item(&app, "扳手").await;

    // GET 返回 ETag: "r1"。
    let fetched = app
        .call(Method::GET, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(fetched.status, StatusCode::OK, "{}", fetched.text());
    assert_eq!(fetched.header("etag").as_deref(), Some("\"r1\""));
    assert_eq!(fetched.json()["data"]["revision"], 1);

    // 缺 If-Match → 428（即使 CSRF 正确）。
    let missing = app
        .call(Method::PATCH, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&serde_json::json!({ "name": "新名字" }))
        .send()
        .await;
    missing.assert_contract_error(StatusCode::PRECONDITION_REQUIRED, "PRECONDITION_REQUIRED");

    // 非法 If-Match → 422。
    let malformed = app
        .call(Method::PATCH, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", "*")
        .json(&serde_json::json!({ "name": "新名字" }))
        .send()
        .await;
    malformed.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 正常更新 r1 → r2。
    let updated = app
        .call(Method::PATCH, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", "\"r1\"")
        .json(&serde_json::json!({ "name": "新名字" }))
        .send()
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.json()["data"]["revision"], 2);
    assert_eq!(updated.header("etag").as_deref(), Some("\"r2\""));

    // 再用旧 revision r1 → 412 且 details.currentRevision = 2。
    let stale = app
        .call(Method::PATCH, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", "\"r1\"")
        .json(&serde_json::json!({ "name": "再改" }))
        .send()
        .await;
    stale.assert_contract_error(StatusCode::PRECONDITION_FAILED, "REVISION_CONFLICT");
    assert_eq!(stale.json()["error"]["details"]["currentRevision"], 2);
    // 412 不泄露 SQL/堆栈。
    assert!(!stale.text().contains("SELECT"));

    // 不存在的物品 → 404；列表返回 {data, nextCursor}。
    let missing_item = app
        .call(
            Method::GET,
            "/api/v1/items/01993000-0000-7000-8000-0000000000ff",
        )
        .cookie(&cookie)
        .send()
        .await;
    missing_item.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");

    let list = app
        .call(Method::GET, "/api/v1/items?limit=1")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(list.status, StatusCode::OK);
    let body = list.json();
    assert!(body["data"].is_array());
    assert!(body.get("nextCursor").is_some());

    // 分页参数非法 → 422（不落入 500）。
    let bad_limit = app
        .call(Method::GET, "/api/v1/items?limit=0")
        .cookie(&cookie)
        .send()
        .await;
    bad_limit.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
}

#[tokio::test]
async fn logout_revokes_session_and_expired_session_is_401() {
    let app = TestApp::new("logout").await;
    let admin_id = app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app, PASSWORD).await;

    let logout = app
        .call(Method::POST, "/api/v1/auth/logout")
        .cookie(&cookie)
        .csrf(&csrf)
        .send()
        .await;
    assert_eq!(logout.status, StatusCode::NO_CONTENT, "{}", logout.text());
    assert!(logout.body.is_empty());
    let cleared = logout.set_cookies().join("; ");
    assert!(
        cleared.contains("em_session=;") && cleared.contains("Max-Age=0"),
        "登出必须清 cookie：{cleared}"
    );

    // 旧 cookie 再访问 → 401。
    let after_logout = app
        .call(Method::GET, "/api/v1/auth/session")
        .cookie(&cookie)
        .send()
        .await;
    after_logout.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");

    // 过期会话 → 401（直接插入已过期会话行）。
    let (expired_cookie, _csrf, _session_id) = app.insert_session(&admin_id, -60_000).await;
    let expired = app
        .call(Method::GET, "/api/v1/items")
        .cookie(&expired_cookie)
        .send()
        .await;
    expired.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");

    // 伪造 token（库里没有）→ 401。
    let forged = app
        .call(Method::GET, "/api/v1/items")
        .cookie("em_session=deadbeef")
        .send()
        .await;
    forged.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
}

#[tokio::test]
async fn unknown_api_paths_are_json_404_without_session() {
    let app = TestApp::new("json-404").await;
    app.set_admin_password(PASSWORD).await;

    for uri in ["/api/unknown", "/api/v1/does-not-exist", "/api"] {
        let response = app.call(Method::GET, uri).send().await;
        response.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
        assert!(
            response
                .header("content-type")
                .unwrap_or_default()
                .starts_with("application/json"),
            "{uri} 必须返回 JSON"
        );
    }
}

#[tokio::test]
async fn settings_status_reports_configuration_without_secrets() {
    let app = TestApp::new("settings").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, _csrf) = login(&app, PASSWORD).await;

    let response = app
        .call(Method::GET, "/api/v1/settings/status")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body = response.json();
    assert_eq!(body["data"]["providersConfigured"]["tripo"], false);
    assert_eq!(body["data"]["providersConfigured"]["manualAi"], false);
    assert_eq!(body["data"]["capabilities"]["generation"], false);
    assert_eq!(body["data"]["limits"]["maxPdfPages"], 100);
    assert_eq!(body["data"]["limits"]["maxJsonRequestBytes"], 1_048_576);
    // 不返回密钥或完整配置：不含 base_url、data-dir、监听、密钥字面量。
    let text = response.text();
    for forbidden in [
        "openapi.tripo3d.ai",
        "api.openai.com",
        "api_key",
        "apiKey",
        "data_dir",
        "dataDir",
        "listen",
    ] {
        assert!(
            !text.contains(forbidden),
            "settings/status 泄露 {forbidden}：{text}"
        );
    }

    // 已配置（假凭据）时：状态为 true，但响应不得出现密钥或来源字符串。
    let dir = common::TestDir::new("settings-configured");
    let mut settings = test_settings(dir.path());
    settings.providers.tripo = configured_tripo(CANARY_KEY);
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, _) = login(&app, PASSWORD).await;
    let configured = app
        .call(Method::GET, "/api/v1/settings/status")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(
        configured.json()["data"]["providersConfigured"]["tripo"],
        true
    );
    assert_eq!(
        configured.json()["data"]["capabilities"]["generation"],
        false
    );
    let text = configured.text();
    assert!(!text.contains(CANARY_KEY), "响应泄露密钥：{text}");
    assert!(!text.contains("CANARY_SOURCE"), "响应泄露密钥来源：{text}");
}

#[tokio::test]
async fn ready_reflects_data_layer_and_reports_not_ready_when_db_unavailable() {
    let app = TestApp::new("ready").await;

    let ready = app.call(Method::GET, "/api/v1/health/ready").send().await;
    assert_eq!(ready.status, StatusCode::OK, "{}", ready.text());
    let body = ready.json();
    assert_eq!(body["data"]["status"], "ready");
    let names: Vec<String> = body["data"]["checks"]
        .as_array()
        .expect("checks 数组")
        .iter()
        .map(|check| {
            assert_eq!(check["status"], "ok");
            check["name"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        names,
        vec!["process", "data_directory", "database", "migrations"],
        "检查项顺序与命名是合同"
    );
    // 不泄露配置细节（无路径、无版本号）。
    let text = ready.text();
    assert!(
        !text.contains("manual.sqlite3") && !text.contains("v2") && !text.contains("v3"),
        "{text}"
    );

    // 负例 1：data-dir 只读 → data_directory 失败（503）。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(app.dir(), std::fs::Permissions::from_mode(0o500))
            .expect("把 data-dir 改为只读");
        let readonly = app.call(Method::GET, "/api/v1/health/ready").send().await;
        assert_eq!(readonly.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(readonly.json()["data"]["status"], "not_ready");
        let data_directory = readonly.json()["data"]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == "data_directory")
            .expect("data_directory 检查项")
            .clone();
        assert_eq!(data_directory["status"], "fail");
        // 恢复权限，保证临时目录可被清理。
        std::fs::set_permissions(app.dir(), std::fs::Permissions::from_mode(0o700))
            .expect("恢复 data-dir 权限");
    }

    // 负例 2：数据库文件被移走 → data_directory 失败（503），不泄露路径。
    std::fs::remove_file(app.dir().join("manual.sqlite3")).expect("移走数据库文件");
    let missing_db = app.call(Method::GET, "/api/v1/health/ready").send().await;
    assert_eq!(missing_db.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(missing_db.json()["data"]["status"], "not_ready");
    let data_directory = missing_db.json()["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "data_directory")
        .expect("data_directory 检查项")
        .clone();
    assert_eq!(data_directory["status"], "fail");
    assert!(!missing_db.text().contains("manual.sqlite3"));

    // 负例 3：数据库连接不可用（连接池已关闭）→ database 失败。
    app.state().database().pool().close().await;
    let unavailable = app.call(Method::GET, "/api/v1/health/ready").send().await;
    assert_eq!(unavailable.status, StatusCode::SERVICE_UNAVAILABLE);
    let body = unavailable.json();
    assert_eq!(body["data"]["status"], "not_ready");
    let database = body["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "database")
        .expect("database 检查项");
    assert_eq!(database["status"], "fail");
}

#[tokio::test]
async fn json_body_errors_use_unified_envelope() {
    let app = TestApp::new("json-errors").await;
    app.set_admin_password(PASSWORD).await;

    // 非 JSON content-type → 415。
    let wrong_type = app
        .call(Method::POST, "/api/v1/auth/login")
        .raw_body(Some("text/plain"), b"password=x".to_vec())
        .send()
        .await;
    wrong_type.assert_contract_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE");

    // 语法错误的 JSON → 422。
    let malformed = app
        .call(Method::POST, "/api/v1/auth/login")
        .raw_body(Some("application/json"), b"{not-json".to_vec())
        .send()
        .await;
    malformed.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 缺字段 → 422（不 500）。
    let missing_field = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&serde_json::json!({}))
        .send()
        .await;
    missing_field.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 超限请求体（默认 1 MiB）→ 413。
    let oversized = app
        .call(Method::POST, "/api/v1/auth/login")
        .raw_body(Some("application/json"), vec![b'x'; 1_100_000])
        .send()
        .await;
    oversized.assert_contract_error(StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE");
}

#[tokio::test]
async fn trusted_proxy_source_ip_is_used_for_rate_limiting() {
    // 受信代理链解析的集成证据：XFF（来自受信代理）决定限速键；
    // 该单元语义在 http::auth::tests 有穷举覆盖，这里只验证与配置项连通。
    let dir = common::TestDir::new("proxy");
    let mut settings = test_settings(dir.path());
    settings.session.login_rate_limit_per_minute = 1;
    settings.trusted_proxy_cidrs = vec![cidr("10.0.0.0/8")];
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;

    // 进程内测试没有 ConnectInfo：来源固定为哨兵地址，因此失败一次即限速。
    let first = app
        .call(Method::POST, "/api/v1/auth/login")
        .header("x-forwarded-for", "198.51.100.7")
        .json(&serde_json::json!({ "password": "nope" }))
        .send()
        .await;
    first.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
    let second = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&serde_json::json!({ "password": PASSWORD }))
        .send()
        .await;
    second.assert_contract_error(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED");
}

/// 直接经仓储创建物品（T04 没有创建端点；AC-016 的创建属 T07）。
async fn create_item(app: &TestApp, name: &str) -> String {
    use everything_manual::storage::repo::items::{self, NewItem};
    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    items::create(
        &mut connection,
        NewItem {
            name: name.to_owned(),
            brand: Some("示例品牌".to_owned()),
            model: "M-100".to_owned(),
            variant: None,
        },
    )
    .await
    .expect("创建物品")
    .id
}
