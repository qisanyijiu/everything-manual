//! Explicit model discovery: real loopback HTTP, synthetic credentials, isolated SQLite.
mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode};
use common::{TestApp, TestDir, test_settings};
use everything_manual::config::{
    SecretString,
    encrypted_secrets::Secrets,
    provider_overrides::{FILE_NAME, ProviderConfigStore},
};
use serde_json::{Value, json};

const PATH: &str = "/api/v1/settings/providers/manual-ai/models";
const KEY: &str = "models-active-canary-73x";
const NEXT_KEY: &str = "models-saved-canary-94y";
#[derive(Clone)]
struct Reply {
    status: u16,
    body: Vec<u8>,
    streamed: bool,
    delay: Duration,
}
impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            body: serde_json::to_vec(&value).unwrap(),
            streamed: false,
            delay: Duration::ZERO,
        }
    }
}
struct Fixture {
    base: String,
    reply: Arc<Mutex<Reply>>,
    calls: Arc<Mutex<Vec<(String, String, bool)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn start(key: &'static str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let reply = Arc::new(Mutex::new(Reply::json(
            json!({"data":[{"id":"org/model-v2"},{"id":"local:model-v1"},{"id":"org/model-v2"}],"ignored":"not returned"}),
        )));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (response, captured) = (reply.clone(), calls.clone());
        let app = Router::new().fallback(move |request: Request<Body>| {
            let (response, captured) = (response.clone(), captured.clone());
            async move {
                captured.lock().unwrap().push((
                    request.method().to_string(),
                    request.uri().to_string(),
                    request.headers().get("authorization").is_some_and(|value| {
                        value.as_bytes() == format!("Bearer {key}").as_bytes()
                    }),
                ));
                let reply = response.lock().unwrap().clone();
                tokio::time::sleep(reply.delay).await;
                let body = if reply.streamed {
                    Body::from_stream(tokio_util::io::ReaderStream::new(std::io::Cursor::new(
                        reply.body,
                    )))
                } else {
                    Body::from(reply.body)
                };
                Response::builder()
                    .status(reply.status)
                    .header("content-type", "application/json")
                    .header("location", "/redirected-must-not-be-followed")
                    .body(body)
                    .unwrap()
            }
        });
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base,
            reply,
            calls,
            task,
        }
    }
    fn set(&self, reply: Reply) {
        *self.reply.lock().unwrap() = reply;
    }
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn app_for(base: &str, key: Option<&str>) -> TestApp {
    let dir = TestDir::new("models");
    let mut settings = test_settings(dir.path());
    settings.providers.manual_ai.base_url = base.into();
    settings.providers.manual_ai.api_key = key.map(SecretString::new);
    // Discovery must work when there is no valid configured model, Tripo key or price catalog.
    settings.providers.manual_ai.model = None;
    TestApp::with_settings(dir, settings).await
}
async fn session(app: &TestApp) -> (String, String) {
    let admin = app.set_admin_password("models-local-test-password").await;
    let (cookie, csrf, _) = app.insert_session(&admin, 60_000).await;
    (cookie, csrf)
}
async fn read(app: &TestApp, cookie: &str, csrf: &str) -> common::TestResponse {
    app.call(Method::POST, PATH)
        .cookie(cookie)
        .csrf(csrf)
        .send()
        .await
}
async fn assert_no_purchase(app: &TestApp) {
    for query in [
        "SELECT COUNT(*) FROM jobs",
        "SELECT COUNT(*) FROM quotes",
        "SELECT COUNT(*) FROM provider_attempts",
        "SELECT COUNT(*) FROM cost_ledger",
        "SELECT COUNT(*) FROM generation_snapshots",
    ] {
        let count: i64 = sqlx::query_scalar(query)
            .fetch_one(app.state().database().pool())
            .await
            .unwrap();
        assert_eq!(count, 0, "{query}");
    }
}
fn assert_safe_failure(response: &common::TestResponse) {
    assert_eq!(response.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        response.header("cache-control").as_deref(),
        Some("no-store")
    );
    assert!(!response.text().contains(KEY));
    assert!(!response.text().contains("sk-01234567890123456789"));
    assert!(!response.text().contains("upstream-private-diagnostic"));
    assert!(response.json()["data"].is_null());
    assert!(response.json()["error"]["requestId"].is_string());
}

#[tokio::test]
async fn requires_session_csrf_and_origin_before_any_provider_request() {
    let fixture = Fixture::start(KEY).await;
    let app = app_for(&fixture.base, Some(KEY)).await;
    assert_eq!(
        app.call(Method::POST, PATH).send().await.status,
        StatusCode::UNAUTHORIZED
    );
    let (cookie, csrf) = session(&app).await;
    assert_eq!(
        app.call(Method::POST, PATH)
            .cookie(&cookie)
            .send()
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        read(&app, &cookie, "wrong-token").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.call(Method::POST, PATH)
            .cookie(&cookie)
            .csrf(&csrf)
            .origin("https://untrusted.example")
            .send()
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.call(Method::GET, PATH)
            .cookie(&cookie)
            .send()
            .await
            .status,
        StatusCode::METHOD_NOT_ALLOWED
    );
    app.call(Method::GET, "/api/v1/settings/providers")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(fixture.count(), 0);
    assert!(!app.dir().join(FILE_NAME).exists());
    assert_no_purchase(&app).await;
}

#[tokio::test]
async fn pending_uses_active_then_reloaded_encrypted_configuration_and_never_writes() {
    let active = Fixture::start(KEY).await;
    let saved = Fixture::start(NEXT_KEY).await;
    let app = app_for(&active.base, Some(KEY)).await;
    let deployment = app.state().settings().clone();
    *app.state().provider_config().write().await =
        ProviderConfigStore::deployment(&deployment).with_secrets(Secrets::fixed([29; 32]));
    let (cookie, csrf) = session(&app).await;
    let initial = app
        .call(Method::GET, "/api/v1/settings/providers")
        .cookie(&cookie)
        .send()
        .await
        .json();
    let result = app.call(Method::PUT, "/api/v1/settings/providers").cookie(&cookie).csrf(&csrf).json(&json!({
        "revision":initial["data"]["revision"], "tripo":{"action":"restore"},
        "manualAi":{"action":"update","baseUrl":saved.base,"model":"next-model","keyAction":"replace","apiKey":NEXT_KEY}
    })).send().await;
    assert_eq!(result.status, StatusCode::OK);
    assert_eq!(result.json()["data"]["pending"], true);
    assert_eq!(
        active.count() + saved.count(),
        0,
        "GET/PUT configuration must not discover automatically"
    );
    let before = std::fs::read(app.dir().join(FILE_NAME)).unwrap();
    assert!(!String::from_utf8_lossy(&before).contains(NEXT_KEY));
    let models = read(&app, &cookie, &csrf).await;
    assert_eq!(models.status, StatusCode::OK);
    assert_eq!(models.header("cache-control").as_deref(), Some("no-store"));
    assert_eq!(
        models.json(),
        json!({"data":["local:model-v1","org/model-v2"]})
    );
    assert_eq!(
        *active.calls.lock().unwrap(),
        vec![("GET".into(), "/v1/models".into(), true)]
    );
    assert_eq!(saved.count(), 0);
    assert_eq!(std::fs::read(app.dir().join(FILE_NAME)).unwrap(), before);
    let mut restarted_settings = deployment;
    let loaded =
        ProviderConfigStore::load_from(&mut restarted_settings, Secrets::fixed([29; 32]), false)
            .unwrap();
    assert!(!loaded.pending());
    *app.state().provider_config().write().await = loaded;
    assert_eq!(read(&app, &cookie, &csrf).await.status, StatusCode::OK);
    assert_eq!(active.count(), 1);
    assert_eq!(
        *saved.calls.lock().unwrap(),
        vec![("GET".into(), "/v1/models".into(), true)]
    );
    assert_eq!(std::fs::read(app.dir().join(FILE_NAME)).unwrap(), before);
    assert_no_purchase(&app).await;
}

#[tokio::test]
async fn missing_key_or_unsafe_active_address_fails_without_network() {
    let fixture = Fixture::start(KEY).await;
    let app = app_for(&fixture.base, None).await;
    let (cookie, csrf) = session(&app).await;
    let missing = read(&app, &cookie, &csrf).await;
    assert_eq!(missing.status, StatusCode::CONFLICT);
    assert_eq!(missing.json()["error"]["code"], "PROVIDER_NOT_CONFIGURED");
    for url in [
        "http://non-loopback.invalid/v1",
        "https://user:pass@example.invalid",
        "https://example.invalid/v1?private=x",
        "https://example.invalid/v1#private",
    ] {
        let mut settings = app.state().settings().clone();
        settings.providers.manual_ai.base_url = url.into();
        settings.providers.manual_ai.api_key = Some(SecretString::new(KEY));
        *app.state().provider_config().write().await = ProviderConfigStore::deployment(&settings);
        assert_eq!(
            read(&app, &cookie, &csrf).await.status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(fixture.count(), 0);
    assert_no_purchase(&app).await;
}

#[tokio::test]
async fn upstream_errors_and_redirects_never_echo_or_retry_or_follow() {
    let fixture = Fixture::start(KEY).await;
    let app = app_for(&fixture.base, Some(KEY)).await;
    let (cookie, csrf) = session(&app).await;
    for status in [301, 302, 307, 308, 400, 401, 403, 404, 405, 429, 500] {
        let mut reply =
            Reply::json(json!({"error":{"message":format!("{KEY} upstream-private-diagnostic")}}));
        reply.status = status;
        fixture.set(reply);
        let before = fixture.count();
        assert_safe_failure(&read(&app, &cookie, &csrf).await);
        assert_eq!(fixture.count(), before + 1);
    }
    assert!(
        fixture
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(method, path, auth)| method == "GET" && path == "/v1/models" && *auth)
    );
    assert_no_purchase(&app).await;
}

#[tokio::test]
async fn malformed_ids_and_credential_echo_are_rejected_without_partial_results() {
    let fixture = Fixture::start(KEY).await;
    let app = app_for(&fixture.base, Some(KEY)).await;
    let (cookie, csrf) = session(&app).await;
    for id in [
        "",
        " space",
        "line\nbreak",
        "sk-01234567890123456789",
        "Bearer sk-01234567890123456789",
        KEY,
        &"a".repeat(129),
    ] {
        fixture.set(Reply::json(
            json!({"data":[{"id":"valid-first"},{"id":id}]}),
        ));
        assert_safe_failure(&read(&app, &cookie, &csrf).await);
    }
    for body in [
        json!({"data":[{"id":42}]}),
        json!({"data":{}}),
        json!({"data":[{"id":"safe"}],"metadata":KEY}),
        json!({KEY:"echo as property name","data":[]}),
        json!({"data":vec![json!({"id":"same"}); 1001]}),
    ] {
        fixture.set(Reply::json(body));
        assert_safe_failure(&read(&app, &cookie, &csrf).await);
    }
    for body in [
        b"not-json upstream-private-diagnostic".to_vec(),
        format!(
            "{{\"data\":[{{\"id\":\"{}\"}}]}}",
            KEY.replace('-', "\\u002d")
        )
        .into_bytes(),
    ] {
        let mut reply = Reply::json(json!({}));
        reply.body = body;
        fixture.set(reply);
        assert_safe_failure(&read(&app, &cookie, &csrf).await);
    }
    fixture.set(Reply::json(json!({"data":[]})));
    let empty = read(&app, &cookie, &csrf).await;
    assert_eq!(empty.status, StatusCode::OK);
    assert_eq!(empty.json(), json!({"data":[]}));
    assert_no_purchase(&app).await;
}

#[tokio::test]
async fn declared_and_streamed_body_caps_reject_instead_of_truncating() {
    let fixture = Fixture::start(KEY).await;
    let app = app_for(&fixture.base, Some(KEY)).await;
    let (cookie, csrf) = session(&app).await;
    for streamed in [false, true] {
        let mut reply = Reply::json(json!({"data":[],"huge":"x".repeat(256 * 1024)}));
        reply.streamed = streamed;
        fixture.set(reply);
        assert_safe_failure(&read(&app, &cookie, &csrf).await);
    }
    assert_eq!(fixture.count(), 2);
}

#[tokio::test]
async fn overall_timeout_is_bounded_and_does_not_retry() {
    let fixture = Fixture::start(KEY).await;
    let app = app_for(&fixture.base, Some(KEY)).await;
    let (cookie, csrf) = session(&app).await;
    let mut reply = Reply::json(json!({"data":[]}));
    reply.delay = Duration::from_secs(30);
    fixture.set(reply);
    let start = Instant::now();
    let response = tokio::time::timeout(Duration::from_secs(15), read(&app, &cookie, &csrf))
        .await
        .expect("10s HTTP timeout must finish within 15s test guard");
    assert_safe_failure(&response);
    assert!(start.elapsed() < Duration::from_secs(15));
    assert_eq!(fixture.count(), 1);
    assert_no_purchase(&app).await;
}
