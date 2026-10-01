//! AS-01：真实路由、SQLite、受限文件；无外部供应商请求。
mod common;
use axum::http::{Method, StatusCode};
use common::{TestApp, TestDir, test_settings};
use everything_manual::config::{
    SecretString,
    provider_overrides::{FILE_NAME, ProviderConfigStore},
};
use serde_json::{Value, json};

async fn session(app: &TestApp) -> (String, String) {
    app.set_admin_password("api-settings-rd-password").await;
    let r = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&json!({"password":"api-settings-rd-password"}))
        .send()
        .await;
    (
        r.session_cookie(),
        r.json()["data"]["csrfToken"].as_str().unwrap().into(),
    )
}
async fn read(app: &TestApp, cookie: &str) -> Value {
    let r = app
        .call(Method::GET, "/api/v1/settings/providers")
        .cookie(cookie)
        .send()
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    r.json()["data"].clone()
}
fn edit(view: &Value) -> Value {
    json!({"action":"update","baseUrl":view["baseUrl"],"model":view["model"],"keyAction":"keep"})
}
fn body(view: &Value) -> Value {
    json!({"revision":view["revision"],"tripo":edit(&view["saved"]["tripo"]),"manualAi":edit(&view["saved"]["manualAi"])})
}
async fn put(app: &TestApp, cookie: &str, csrf: &str, body: &Value) -> common::TestResponse {
    app.call(Method::PUT, "/api/v1/settings/providers")
        .cookie(cookie)
        .csrf(csrf)
        .json(body)
        .send()
        .await
}

#[tokio::test]
async fn authentication_csrf_and_secret_structure_errors_do_not_echo() {
    let app = TestApp::new("api-settings-auth").await;
    assert_eq!(
        app.call(Method::GET, "/api/v1/settings/providers")
            .send()
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let (cookie, csrf) = session(&app).await;
    let view = read(&app, &cookie).await;
    let mut write = body(&view);
    assert_eq!(
        app.call(Method::PUT, "/api/v1/settings/providers")
            .cookie(&cookie)
            .json(&write)
            .send()
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    for field in ["action", "keyAction"] {
        write["tripo"][field] = json!("private-canary-enum-never-echo");
        let r = put(&app, &cookie, &csrf, &write).await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!r.text().contains("private-canary"));
        write = body(&view);
    }
    write["tripo"]["private-canary-field-never-echo"] = json!(true);
    let r = put(&app, &cookie, &csrf, &write).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!r.text().contains("private-canary"));
    assert!(!app.dir().join(FILE_NAME).exists());
}

#[tokio::test]
async fn keep_clear_restore_persist_without_copying_deployment_secret_and_revert_keeps_epoch() {
    let dir = TestDir::new("api-settings-keys");
    let mut settings = test_settings(dir.path());
    settings.providers.tripo.api_key = Some(SecretString::new("deployment-private-canary"));
    settings.providers.tripo.key_source = Some("not-public-path".into());
    let base = settings.clone();
    let app = TestApp::with_settings(dir, settings).await;
    let (cookie, csrf) = session(&app).await;
    let initial = read(&app, &cookie).await;
    let mut write = body(&initial);
    write["tripo"]["baseUrl"] = json!("http://127.0.0.1:19999/v3/");
    let saved = put(&app, &cookie, &csrf, &write).await;
    assert_eq!(saved.status, StatusCode::OK);
    assert!(saved.json()["data"]["pending"].as_bool().unwrap());
    assert!(!saved.text().contains("deployment-private-canary"));
    let file = std::fs::read_to_string(app.dir().join(FILE_NAME)).unwrap();
    assert!(!file.contains("deployment-private-canary"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(app.dir().join(FILE_NAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let mut restarted = base.clone();
    let loaded = ProviderConfigStore::load_from(
        &mut restarted,
        everything_manual::config::encrypted_secrets::Secrets::unavailable(),
        false,
    )
    .unwrap();
    assert!(!loaded.pending());
    assert_eq!(
        restarted.providers.tripo.base_url,
        "http://127.0.0.1:19999/v3"
    );
    assert!(restarted.providers.tripo.api_key == base.providers.tripo.api_key);
    let view = read(&app, &cookie).await;
    let mut clear = body(&view);
    clear["tripo"]["keyAction"] = json!("clear");
    assert_eq!(
        put(&app, &cookie, &csrf, &clear).await.status,
        StatusCode::OK
    );
    let mut restarted = base.clone();
    ProviderConfigStore::load_from(
        &mut restarted,
        everything_manual::config::encrypted_secrets::Secrets::unavailable(),
        false,
    )
    .unwrap();
    assert!(restarted.providers.tripo.api_key.is_none());
    let view = read(&app, &cookie).await;
    let mut restore = body(&view);
    restore["tripo"] = json!({"action":"restore"});
    let restored = put(&app, &cookie, &csrf, &restore).await;
    assert_eq!(restored.status, StatusCode::OK);
    assert_eq!(restored.json()["data"]["pending"], false);
    assert_ne!(restored.json()["data"]["revision"], initial["revision"]);
    assert!(
        app.state()
            .provider_config()
            .read()
            .await
            .ensure_revision(&json!({}))
            .is_err()
    );
    let mut restarted = base.clone();
    ProviderConfigStore::load_from(
        &mut restarted,
        everything_manual::config::encrypted_secrets::Secrets::unavailable(),
        false,
    )
    .unwrap();
    assert!(restarted.providers.tripo.api_key == base.providers.tripo.api_key);
    assert_eq!(
        put(&app, &cookie, &csrf, &write).await.status,
        StatusCode::CONFLICT
    );
    for query in [
        "SELECT COUNT(*) FROM jobs",
        "SELECT COUNT(*) FROM quotes",
        "SELECT COUNT(*) FROM provider_attempts",
        "SELECT COUNT(*) FROM cost_ledger",
    ] {
        let count: i64 = sqlx::query_scalar(query)
            .fetch_one(app.state().database().pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn validation_and_disk_failure_keep_both_providers_and_revision_unchanged() {
    let app = TestApp::new("api-settings-failures").await;
    let (cookie, csrf) = session(&app).await;
    let view = read(&app, &cookie).await;
    for url in [
        "http://example.com",
        "https://canary-user:canary-pass@example.com",
        "https://example.com?q=canary",
        "https://example.com#canary",
        "https://example.com\n",
    ] {
        let mut write = body(&view);
        write["tripo"]["baseUrl"] = json!(url);
        write["manualAi"]["model"] = json!("changed");
        let response = put(&app, &cookie, &csrf, &write).await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!response.text().contains("canary"));
        assert_eq!(read(&app, &cookie).await, view);
    }
    let mut write = body(&view);
    write["tripo"]["keyAction"] = json!("replace");
    write["tripo"]["apiKey"] = json!(" ");
    assert_eq!(
        put(&app, &cookie, &csrf, &write).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    std::fs::create_dir(app.dir().join(FILE_NAME)).unwrap();
    let mut write = body(&view);
    write["manualAi"]["model"] = json!("new-model");
    assert_eq!(
        put(&app, &cookie, &csrf, &write).await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(read(&app, &cookie).await, view);
}

#[tokio::test]
async fn loaded_bad_files_fail_closed_and_deployment_url_credentials_are_never_readable() {
    let dir = TestDir::new("api-settings-corrupt");
    let mut settings = test_settings(dir.path());
    settings.providers.tripo.base_url =
        "https://private-user:private-pass@example.com/v3?private-query".into();
    let store = ProviderConfigStore::deployment(&settings);
    let response = serde_json::to_string(&store.view()).unwrap();
    assert!(!response.contains("private-"));
    std::fs::write(dir.join(FILE_NAME), "malformed-private-canary").unwrap();
    assert!(
        ProviderConfigStore::load_from(
            &mut settings,
            everything_manual::config::encrypted_secrets::Secrets::unavailable(),
            false
        )
        .is_err()
    );
    #[cfg(unix)]
    {
        std::fs::remove_file(dir.join(FILE_NAME)).unwrap();
        std::os::unix::fs::symlink("not-present", dir.join(FILE_NAME)).unwrap();
        assert!(
            ProviderConfigStore::load_from(
                &mut settings,
                everything_manual::config::encrypted_secrets::Secrets::unavailable(),
                false
            )
            .is_err()
        );
    }
}
