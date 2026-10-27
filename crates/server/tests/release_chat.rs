//! 说明书问答：`POST /items/{id}/releases/{releaseId}/chat`。
//!
//! 守住的合同（本机 loopback fixture，零外网/付费）：
//! - 资料来自发布版冻结知识；请求带服务端密钥、strict json_schema、store=false；
//! - 回答页码按说明书页数过滤；
//! - 未配置说明书 AI → 409 且不发请求；非法消息 → 422 且不发请求；
//! - 上游失败 → 502，响应不含上游原文与密钥；
//! - 跨物品 release → 404。

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode};
use common::{TestApp, TestDir, test_settings};
use everything_manual::config::SecretString;
use serde_json::{Value, json};

const KEY: &str = "chat-canary-key-5k2";

struct Fixture {
    base: String,
    reply: Arc<Mutex<(u16, Value)>>,
    calls: Arc<Mutex<Vec<(bool, Value)>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let reply = Arc::new(Mutex::new((200, Value::Null)));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (response, captured) = (reply.clone(), calls.clone());
        let app = Router::new().fallback(move |request: Request<Body>| {
            let (response, captured) = (response.clone(), captured.clone());
            async move {
                let authorized = request
                    .headers()
                    .get("authorization")
                    .is_some_and(|v| v.as_bytes() == format!("Bearer {KEY}").as_bytes());
                let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
                    .await
                    .unwrap();
                captured.lock().unwrap().push((
                    authorized,
                    serde_json::from_slice(&bytes).unwrap_or(Value::Null),
                ));
                let (status, body) = response.lock().unwrap().clone();
                Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
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
    fn answer(&self, answer: &str, pages: &[i64]) {
        let text = json!({ "answer": answer, "citedPages": pages }).to_string();
        *self.reply.lock().unwrap() = (
            200,
            json!({ "output": [{ "content": [{ "type": "output_text", "text": text }] }] }),
        );
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

/// 应用 + 内置示例发布版 → (app, cookie, csrf, itemId, releaseId)。
async fn app_with_release(
    base: &str,
    key: Option<&str>,
) -> (TestApp, String, String, String, String) {
    let dir = TestDir::new("release-chat");
    let mut settings = test_settings(dir.path());
    settings.providers.manual_ai.base_url = base.into();
    settings.providers.manual_ai.api_key = key.map(SecretString::new);
    settings.providers.manual_ai.model = Some("chat-model".into());
    let app = TestApp::with_settings(dir, settings).await;
    let admin = app.set_admin_password("release-chat-password").await;
    {
        let mut conn = app.state().database().pool().acquire().await.unwrap();
        assert!(
            everything_manual::config::sample::seed(&mut conn, app.dir(), &admin)
                .await
                .unwrap()
        );
    }
    let (item, release): (String, String) =
        sqlx::query_as("SELECT item_id, id FROM manual_releases")
            .fetch_one(app.state().database().pool())
            .await
            .unwrap();
    let (cookie, csrf, _) = app.insert_session(&admin, 60_000).await;
    (app, cookie, csrf, item, release)
}

async fn ask(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    path: &str,
    body: Value,
) -> common::TestResponse {
    app.call(Method::POST, path)
        .cookie(cookie)
        .csrf(csrf)
        .json(&body)
        .send()
        .await
}

#[tokio::test]
async fn answers_from_release_knowledge_and_filters_pages() {
    let fixture = Fixture::start().await;
    let (app, cookie, csrf, item, release) = app_with_release(&fixture.base, Some(KEY)).await;
    fixture.answer("先让机器狗趴下，再插入快充线。", &[2, 2, 99]);
    let path = format!("/api/v1/items/{item}/releases/{release}/chat");
    let r = ask(
        &app,
        &cookie,
        &csrf,
        &path,
        json!({ "messages": [
            { "role": "user", "content": "怎么充电？" },
            { "role": "assistant", "content": "请看第 2 页。" },
            { "role": "user", "content": "详细说说" }
        ]}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let data = &r.json()["data"];
    assert_eq!(data["answer"], "先让机器狗趴下，再插入快充线。");
    assert_eq!(data["citedPages"], json!([2]));
    assert!(data["documentId"].is_string());

    let calls = fixture.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let (authorized, body) = &calls[0];
    assert!(authorized, "必须带服务端密钥");
    assert_eq!(body["model"], "chat-model");
    assert_eq!(body["store"], false);
    assert_eq!(body["text"]["format"]["strict"], true);
    let input = body["input"].as_array().unwrap();
    assert_eq!(input.len(), 4);
    let context = input[0]["content"][0]["text"].as_str().unwrap();
    assert!(context.contains("给机器狗充电"), "资料应含发布版步骤");
    assert!(context.contains("头部"), "资料应含部件");
    assert!(context.contains("共 4 页"));
    assert_eq!(input[3]["content"][0]["text"], "详细说说");
}

#[tokio::test]
async fn rejects_without_calling_provider() {
    let fixture = Fixture::start().await;
    let (app, cookie, csrf, item, release) = app_with_release(&fixture.base, Some(KEY)).await;
    let path = format!("/api/v1/items/{item}/releases/{release}/chat");
    for body in [
        json!({ "messages": [] }),
        json!({ "messages": [{ "role": "assistant", "content": "hi" }] }),
        json!({ "messages": [{ "role": "system", "content": "hi" }] }),
        json!({ "messages": [{ "role": "user", "content": "  " }] }),
    ] {
        let r = ask(&app, &cookie, &csrf, &path, body).await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    }
    let r = ask(
        &app,
        &cookie,
        &csrf,
        &format!("/api/v1/items/01a00000-0000-7000-8000-000000000000/releases/{release}/chat"),
        json!({ "messages": [{ "role": "user", "content": "hi" }] }),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = app
        .call(Method::POST, &path)
        .json(&json!({ "messages": [{ "role": "user", "content": "hi" }] }))
        .send()
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(fixture.count(), 0);
}

#[tokio::test]
async fn unconfigured_provider_is_409() {
    let fixture = Fixture::start().await;
    let (app, cookie, csrf, item, release) = app_with_release(&fixture.base, None).await;
    let r = ask(
        &app,
        &cookie,
        &csrf,
        &format!("/api/v1/items/{item}/releases/{release}/chat"),
        json!({ "messages": [{ "role": "user", "content": "hi" }] }),
    )
    .await;
    r.assert_contract_error(StatusCode::CONFLICT, "PROVIDER_NOT_CONFIGURED");
    assert_eq!(fixture.count(), 0);
}

#[tokio::test]
async fn upstream_failure_is_502_without_leaking() {
    let fixture = Fixture::start().await;
    let (app, cookie, csrf, item, release) = app_with_release(&fixture.base, Some(KEY)).await;
    *fixture.reply.lock().unwrap() = (500, json!({ "error": "upstream-private-diagnostic" }));
    let r = ask(
        &app,
        &cookie,
        &csrf,
        &format!("/api/v1/items/{item}/releases/{release}/chat"),
        json!({ "messages": [{ "role": "user", "content": "hi" }] }),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_GATEWAY);
    assert!(!r.text().contains("upstream-private-diagnostic"));
    assert!(!r.text().contains(KEY));
    assert_eq!(fixture.count(), 1, "不自动重试");
}
