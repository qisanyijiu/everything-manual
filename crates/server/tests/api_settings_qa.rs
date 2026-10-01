//! AS-01 QA independent HTTP/SQLite checks. No worker, no provider HTTP, no secret logging.
//! This suite intentionally uses repository-backed abnormal states for leftover-action gates;
//! real upload/quote/worker/restart coverage lives in api-settings-qa.spec.ts.
mod common;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestDir, TestResponse};
use everything_manual::config::SecretString;
use everything_manual::config::encrypted_secrets::Secrets;
use everything_manual::config::provider_overrides::{FILE_NAME, ProviderConfigStore};
use everything_manual::storage::repo;
use manual_core::domain::{JobStatus, StageKind};
use manual_core::{ids, timestamps::Timestamp};
use serde_json::{Value, json};

const ENDPOINT: &str = "/api/v1/settings/providers";

fn qa_secrets() -> Secrets {
    static MASTER: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    Secrets::fixed(*MASTER.get_or_init(|| {
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key).expect("QA random master");
        key
    }))
}

async fn app(tag: &str) -> (TestApp, String, String) {
    let dir = TestDir::new(&format!("api-settings-qa-{tag}"));
    let mut settings = common::test_settings(dir.path());
    settings.providers.tripo = common::configured_tripo(&format!("qa-{}", ids::new_id()));
    settings.providers.manual_ai.model = Some("qa-manual-a".into());
    settings.providers.manual_ai.api_key = Some(SecretString::new(format!("qa-{}", ids::new_id())));
    settings.providers.manual_ai.key_source = Some("受限文件 /private/qa-secret-source".into());
    let app = TestApp::with_settings(dir, settings).await;
    *app.state().provider_config().write().await =
        ProviderConfigStore::deployment(app.state().settings()).with_secrets(qa_secrets());
    let admin = app.set_admin_password("qa-settings-password").await;
    let (cookie, csrf, _) = app.insert_session(&admin, 3_600_000).await;
    (app, cookie, csrf)
}

async fn read(app: &TestApp, cookie: &str) -> Value {
    let response = app.call(Method::GET, ENDPOINT).cookie(cookie).send().await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.header("cache-control").as_deref(),
        Some("no-store")
    );
    response.json()["data"].clone()
}

fn request(view: &Value) -> Value {
    let edit = |provider: &str| json!({"action":"update", "baseUrl":view["saved"][provider]["baseUrl"], "model":view["saved"][provider]["model"], "keyAction":"keep"});
    json!({"revision":view["revision"], "tripo":edit("tripo"), "manualAi":edit("manualAi")})
}

async fn save(app: &TestApp, cookie: &str, csrf: &str, body: &Value) -> TestResponse {
    // Valid writes are checked against the actual product DTO before sending, not a QA DTO.
    let _: everything_manual::http::dto::ProviderSettingsWrite =
        serde_json::from_value(body.clone()).expect("valid product DTO");
    app.call(Method::PUT, ENDPOINT)
        .cookie(cookie)
        .csrf(csrf)
        .json(body)
        .send()
        .await
}

fn reason(response: &TestResponse, expected: &str) {
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.json()["error"]["details"]["reason"], expected);
}

#[tokio::test]
async fn as_qa_http_auth_no_store_and_secret_free_read() {
    let (app, cookie, csrf) = app("auth").await;
    let before = read(&app, &cookie).await;
    let body = request(&before);
    assert_eq!(
        app.call(Method::GET, ENDPOINT).send().await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.call(Method::PUT, ENDPOINT)
            .json(&body)
            .send()
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    for token in [None, Some("wrong-csrf")] {
        let mut call = app.call(Method::PUT, ENDPOINT).cookie(&cookie).json(&body);
        if let Some(token) = token {
            call = call.csrf(token);
        }
        assert_eq!(call.send().await.status, StatusCode::FORBIDDEN);
    }
    assert_eq!(
        app.call(Method::PUT, ENDPOINT)
            .cookie(&cookie)
            .csrf(&csrf)
            .origin("https://other.invalid")
            .json(&body)
            .send()
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(read(&app, &cookie).await, before);
    assert!(!app.dir().join(FILE_NAME).exists());
    let text = before.to_string();
    for provider in [
        &app.state().settings().providers.tripo,
        &app.state().settings().providers.manual_ai,
    ] {
        assert!(
            !text.contains(provider.api_key.as_ref().unwrap().expose()),
            "secret leak boolean"
        );
    }
    assert!(!text.contains("qa-secret-source"));
    assert_eq!(before["active"]["tripo"]["baseUrlSource"], "deployment");
    assert_eq!(before["active"]["tripo"]["modelSource"], "deployment");
}

#[tokio::test]
async fn as_qa_legacy_deployment_url_userinfo_never_reads_back() {
    let dir = TestDir::new("api-settings-qa-userinfo");
    let mut settings = common::test_settings(dir.path());
    let marker = format!("qa-{}", ids::new_id());
    settings.providers.tripo.base_url = format!("https://user:{marker}@example.invalid/v3");
    let app = TestApp::with_settings(dir, settings).await;
    let admin = app.set_admin_password("qa-settings-password").await;
    let (cookie, _, _) = app.insert_session(&admin, 60_000).await;
    let view = read(&app, &cookie).await;
    assert!(
        !view.to_string().contains(&marker),
        "legacy credential leak boolean"
    );
    assert_eq!(
        view["active"]["tripo"]["baseUrl"],
        "https://example.invalid/v3"
    );
}

#[tokio::test]
async fn as_qa_validation_matrix_and_decode_errors_are_atomic_and_redacted() {
    let (app, cookie, csrf) = app("validation").await;
    let before = read(&app, &cookie).await;
    let marker = format!("qa-{}", ids::new_id());
    let mut cases = Vec::new();
    for url in [
        format!("https://user:{marker}@example.invalid/v1"),
        format!("https://example.invalid/v1?key={marker}"),
        format!("https://example.invalid/v1#{marker}"),
        "http://example.invalid/v1".into(),
        "file:///tmp/key".into(),
        format!("https://{}.invalid", "x".repeat(2050)),
        "https://example.invalid/\npath".into(),
    ] {
        let mut body = request(&before);
        body["tripo"]["baseUrl"] = json!(url);
        cases.push(body);
    }
    for model in ["m".repeat(129), "model\ninvalid".into()] {
        let mut body = request(&before);
        body["manualAi"]["model"] = json!(model);
        cases.push(body);
    }
    for key in [
        "".into(),
        " ".into(),
        "x".repeat(4097),
        format!("{marker} invalid"),
        format!("{marker}\r\ninvalid"),
    ] {
        let mut body = request(&before);
        body["tripo"]["keyAction"] = json!("replace");
        body["tripo"]["apiKey"] = json!(key);
        cases.push(body);
    }
    for field in ["action", "keyAction"] {
        let mut body = request(&before);
        body["tripo"][field] = json!(marker);
        cases.push(body);
    }
    for body in cases {
        let response = app
            .call(Method::PUT, ENDPOINT)
            .cookie(&cookie)
            .csrf(&csrf)
            .json(&body)
            .send()
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            !response.text().contains(&marker),
            "422 must not echo input marker"
        );
        assert_eq!(read(&app, &cookie).await, before);
        assert!(!app.dir().join(FILE_NAME).exists());
    }
}

#[tokio::test]
async fn as_qa_three_key_actions_restore_and_load_fail_closed() {
    let (app, cookie, csrf) = app("keys").await;
    let initial = read(&app, &cookie).await;
    let mut body = request(&initial);
    body["tripo"]["model"] = json!("qa-web-tripo");
    body["tripo"]["keyAction"] = json!("replace");
    let replacement = format!("qa-{}", ids::new_id());
    body["tripo"]["apiKey"] = json!(replacement);
    body["manualAi"]["model"] = json!("qa-web-manual");
    assert_eq!(
        save(&app, &cookie, &csrf, &body).await.status,
        StatusCode::OK
    );
    let replaced = read(&app, &cookie).await;
    assert_eq!(replaced["pending"], true);
    assert_eq!(replaced["active"], initial["active"]);
    assert_eq!(replaced["saved"]["tripo"]["keySource"], "web");
    assert_eq!(replaced["saved"]["manualAi"]["keySource"], "deployment");
    assert!(!replaced.to_string().contains(&replacement));
    let mut effective = app.state().settings().clone();
    let loaded = ProviderConfigStore::load_from(&mut effective, qa_secrets(), false)
        .expect("load encrypted saved overlay");
    assert!(!loaded.pending());
    assert!(
        effective.providers.tripo.api_key.as_ref().unwrap().expose() == replacement,
        "replacement matches in memory"
    );
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
    let mut keep = request(&replaced);
    keep["tripo"]["model"] = json!("qa-web-tripo-2");
    assert_eq!(
        save(&app, &cookie, &csrf, &keep).await.status,
        StatusCode::OK
    );
    let mut effective = app.state().settings().clone();
    ProviderConfigStore::load_from(&mut effective, qa_secrets(), false).unwrap();
    assert!(
        effective.providers.tripo.api_key.as_ref().unwrap().expose() == replacement,
        "keep replacement in memory"
    );
    let mut clear = request(&read(&app, &cookie).await);
    clear["tripo"]["keyAction"] = json!("clear");
    assert_eq!(
        save(&app, &cookie, &csrf, &clear).await.status,
        StatusCode::OK
    );
    let mut effective = app.state().settings().clone();
    ProviderConfigStore::load_from(&mut effective, qa_secrets(), false).unwrap();
    assert!(
        effective.providers.tripo.api_key.is_none(),
        "clear must mask deployment credential"
    );
    let mut restore = request(&read(&app, &cookie).await);
    restore["tripo"] = json!({"action":"restore"});
    assert_eq!(
        save(&app, &cookie, &csrf, &restore).await.status,
        StatusCode::OK
    );
    let mut effective = app.state().settings().clone();
    ProviderConfigStore::load_from(&mut effective, qa_secrets(), false).unwrap();
    assert!(
        effective.providers.tripo.api_key == app.state().settings().providers.tripo.api_key,
        "restore deployment credential"
    );
    assert_eq!(
        effective.providers.manual_ai.model.as_deref(),
        Some("qa-web-manual")
    );
    std::fs::write(app.dir().join(FILE_NAME), "{invalid").unwrap();
    assert!(
        ProviderConfigStore::load_from(&mut app.state().settings().clone(), qa_secrets(), false)
            .is_err(),
        "corruption must stop load"
    );
}

#[tokio::test]
async fn as_qa_write_failure_and_revision_conflict_never_publish_half_config() {
    let (app, cookie, csrf) = app("write").await;
    let before = read(&app, &cookie).await;
    let mut body = request(&before);
    body["tripo"]["model"] = json!("qa-changed");
    body["manualAi"]["model"] = json!("qa-changed");
    std::fs::create_dir(app.dir().join(FILE_NAME)).unwrap();
    assert_eq!(
        save(&app, &cookie, &csrf, &body).await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(read(&app, &cookie).await, before);
    std::fs::remove_dir(app.dir().join(FILE_NAME)).unwrap();
    assert_eq!(
        save(&app, &cookie, &csrf, &body).await.status,
        StatusCode::OK
    );
    let bytes = std::fs::read(app.dir().join(FILE_NAME)).unwrap();
    let current = read(&app, &cookie).await;
    body["manualAi"]["model"] = json!("qa-stale");
    let conflict = save(&app, &cookie, &csrf, &body).await;
    assert_eq!(conflict.status, StatusCode::CONFLICT);
    assert_eq!(
        conflict.json()["error"]["details"]["reason"],
        "providerConfigConflict"
    );
    assert_eq!(read(&app, &cookie).await, current);
    assert!(
        std::fs::read(app.dir().join(FILE_NAME)).unwrap() == bytes,
        "conflict leaves private file unchanged"
    );
}

/// Minimal real FK chain; fake document bytes are never executed or sent anywhere.
async fn seed_job(app: &TestApp, kind: StageKind) -> (String, String) {
    let now = Timestamp::now();
    let mut conn = app.state().database().pool().acquire().await.unwrap();
    let item = repo::items::create(
        &mut conn,
        repo::items::NewItem {
            name: "AS QA gate fixture".into(),
            brand: None,
            model: "QA".into(),
            variant: None,
        },
    )
    .await
    .unwrap();
    let sha = "a".repeat(64);
    sqlx::query("INSERT INTO blobs(sha256,size,mime,storage_state,created_at) VALUES(?,64,'application/pdf','stored',?)").bind(&sha).bind(now.as_millis()).execute(&mut *conn).await.unwrap();
    let asset = ids::new_id();
    let document = ids::new_id();
    let preparation = ids::new_id();
    sqlx::query("INSERT INTO assets(id,blob_id,item_id,purpose,original_name,created_at) VALUES(?,?,?,'document','qa.pdf',?)").bind(&asset).bind(&sha).bind(&item.id).bind(now.as_millis()).execute(&mut *conn).await.unwrap();
    sqlx::query("INSERT INTO documents(id,item_id,source_asset_id,source_sha256,title,created_at,updated_at) VALUES(?,?,?,?,'QA',?,?)").bind(&document).bind(&item.id).bind(&asset).bind(&sha).bind(now.as_millis()).bind(now.as_millis()).execute(&mut *conn).await.unwrap();
    sqlx::query("INSERT INTO preparations(id,document_id,source_sha256,state,page_count,revision,client_derived,created_at,updated_at) VALUES(?,?,?,'ready',1,1,1,?,?)").bind(&preparation).bind(&document).bind(&sha).bind(now.as_millis()).bind(now.as_millis()).execute(&mut *conn).await.unwrap();
    let snapshot = repo::snapshots::insert(
        &mut conn,
        repo::snapshots::NewSnapshot {
            item_id: item.id.clone(),
            item_revision: 1,
            preparation_id: preparation,
            photo_ids_json: "[]".into(),
            photo_hashes_json: "[]".into(),
            provider_config_json: "{\"configRevision\":\"deployment\"}".into(),
            prompt_version: "qa".into(),
            price_version: "qa".into(),
            budgets_json: "{}".into(),
        },
        now,
    )
    .await
    .unwrap();
    let job = repo::jobs::create(
        &mut conn,
        repo::jobs::NewJob {
            item_id: item.id,
            snapshot_id: snapshot.id,
        },
    )
    .await
    .unwrap();
    let stage = repo::job_stages::insert(
        &mut conn,
        repo::job_stages::NewStage {
            job_id: job.id.clone(),
            stage_kind: kind,
            batch_index: 0,
            page_set_json: None,
            input_hash: "qa-input-hash".into(),
            status: JobStatus::Failed,
        },
        now,
    )
    .await
    .unwrap();
    (job.id, stage.id)
}

#[tokio::test]
async fn as_qa_all_nonterminal_jobs_and_terminal_leftovers_block_change() {
    let (app, cookie, csrf) = app("busy").await;
    let (job, stage) = seed_job(&app, StageKind::TripoSubmit).await;
    let before = read(&app, &cookie).await;
    let unchanged = request(&before);
    let mut changed = unchanged.clone();
    changed["tripo"]["model"] = json!("qa-change");
    let pool = app.state().database().pool();
    for state in [
        "queued",
        "running",
        "waiting_provider",
        "retry_wait",
        "needs_input",
        "submission_unknown",
    ] {
        sqlx::query("UPDATE jobs SET status=? WHERE id=?")
            .bind(state)
            .bind(&job)
            .execute(pool)
            .await
            .unwrap();
        reason(
            &save(&app, &cookie, &csrf, &changed).await,
            "providerConfigBusy",
        );
        assert_eq!(
            save(&app, &cookie, &csrf, &unchanged).await.status,
            StatusCode::OK
        );
    }
    sqlx::query("UPDATE jobs SET status='cancelled' WHERE id=?")
        .bind(&job)
        .execute(pool)
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    let attempt = repo::attempts::create_intent(
        &mut conn,
        repo::attempts::NewAttempt {
            job_id: job.clone(),
            stage_id: stage.clone(),
            request_hash: "qa-request".into(),
        },
        Timestamp::now(),
    )
    .await
    .unwrap();
    drop(conn);
    for state in ["intent", "submitting", "unknown"] {
        sqlx::query("UPDATE provider_attempts SET submit_state=? WHERE id=?")
            .bind(state)
            .bind(&attempt.id)
            .execute(pool)
            .await
            .unwrap();
        reason(
            &save(&app, &cookie, &csrf, &changed).await,
            "providerConfigBusy",
        );
    }
    sqlx::query("UPDATE provider_attempts SET submit_state='failed' WHERE id=?")
        .bind(&attempt.id)
        .execute(pool)
        .await
        .unwrap();
    for state in [
        "queued",
        "running",
        "waiting_provider",
        "retry_wait",
        "submission_unknown",
    ] {
        sqlx::query("UPDATE job_stages SET status=? WHERE id=?")
            .bind(state)
            .bind(&stage)
            .execute(pool)
            .await
            .unwrap();
        reason(
            &save(&app, &cookie, &csrf, &changed).await,
            "providerConfigBusy",
        );
    }
    assert_eq!(read(&app, &cookie).await, before);
    assert!(!app.dir().join(FILE_NAME).exists());
}

#[tokio::test]
async fn as_qa_save_retry_race_is_exclusive_and_old_retry_stays_invalid_after_restore() {
    let (app, cookie, csrf) = app("race").await;
    let (job, stage) = seed_job(&app, StageKind::AssembleDraft).await;
    sqlx::query("UPDATE jobs SET status='failed' WHERE id=?")
        .bind(&job)
        .execute(app.state().database().pool())
        .await
        .unwrap();
    let before = read(&app, &cookie).await;
    let mut changed = request(&before);
    changed["tripo"]["model"] = json!("qa-change");
    let retry_uri = format!("/api/v1/jobs/{job}/retry");
    let retry_body = json!({"stageId":stage});
    let (save_result, retry_result) = tokio::join!(
        save(&app, &cookie, &csrf, &changed),
        app.call(Method::POST, &retry_uri)
            .cookie(&cookie)
            .csrf(&csrf)
            .header("if-match", "\"r1\"")
            .header("idempotency-key", "api-settings-qa-race")
            .json(&retry_body)
            .send()
    );
    assert_ne!(
        save_result.status.is_success(),
        retry_result.status.is_success(),
        "exactly one operation may be accepted"
    );
    if retry_result.status.is_success() {
        reason(&save_result, "providerConfigBusy");
        sqlx::query("UPDATE jobs SET status='failed' WHERE id=?")
            .bind(&job)
            .execute(app.state().database().pool())
            .await
            .unwrap();
        sqlx::query("UPDATE job_stages SET status='failed' WHERE id=?")
            .bind(&stage)
            .execute(app.state().database().pool())
            .await
            .unwrap();
        assert_eq!(
            save(&app, &cookie, &csrf, &changed).await.status,
            StatusCode::OK
        );
    } else {
        reason(&retry_result, "providerConfigPending");
    }
    let current = read(&app, &cookie).await;
    let restore = json!({"revision":current["revision"],"tripo":{"action":"restore"},"manualAi":{"action":"restore"}});
    assert_eq!(
        save(&app, &cookie, &csrf, &restore).await.status,
        StatusCode::OK
    );
    assert_eq!(read(&app, &cookie).await["pending"], false);
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM jobs WHERE id=?")
        .bind(&job)
        .fetch_one(app.state().database().pool())
        .await
        .unwrap();
    let old = app
        .call(Method::POST, &retry_uri)
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &format!("\"r{revision}\""))
        .header("idempotency-key", "api-settings-qa-old-retry")
        .json(&retry_body)
        .send()
        .await;
    reason(&old, "providerConfigChanged");
    assert!(
        old.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("重新报价"),
        "must explain changed configuration rather than a storage error"
    );
    let attempts: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_attempts")
        .fetch_one(app.state().database().pool())
        .await
        .unwrap();
    assert_eq!(attempts, 0);
}
