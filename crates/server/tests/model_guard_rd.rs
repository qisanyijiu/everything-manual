//! PC06: isolated finite canaries, real routes/SQLite, fixed test encryption key.
mod common;
use axum::http::{Method, StatusCode};
use common::{TestApp, TestDir, test_settings};
use everything_manual::config::{
    SecretString,
    encrypted_secrets::{Secrets, write_private_file},
    provider_overrides::{FILE_NAME, ProviderConfigStore},
};
use serde_json::{Value, json};

const CANARY: &str = "sk-pc06_fake_only_0123456789";
const KEY: &str = "pc06-fake-deployment-key";

#[test]
fn pc06_cli_check_reads_toml_and_environment_mistakes_without_echo_or_mutation() {
    let dir = TestDir::new("pc06-cli");
    let password = dir.join("password.txt");
    write_private_file(&password, b"pc06-fake-cli-password\n", false).unwrap();
    let config = dir.join("config.toml");
    let contents = format!(
        "[providers.manual_ai]\nmodel = \"{CANARY}\"\napi_key_env = \"EM_PC06_FAKE_KEY\"\n"
    );
    std::fs::write(&config, &contents).unwrap();
    let command = || {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_everything-manual"));
        command
            .current_dir(dir.path())
            .env_clear()
            .env("EM_SECRETS_MASTER_KEY", "35".repeat(32))
            .env("EM_PC06_FAKE_KEY", KEY);
        command
    };
    let init = command()
        .arg("init")
        .arg("--data-dir")
        .arg(dir.path())
        .arg("--password-file")
        .arg(password)
        .output()
        .unwrap();
    assert!(init.status.success());
    for from_env in [false, true] {
        let mut child = command();
        if from_env {
            child.env("EM_PROVIDERS__MANUAL_AI__MODEL", format!("Bearer {CANARY}"));
        }
        let output = child
            .arg("check")
            .arg("--data-dir")
            .arg(dir.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!text.contains(CANARY) && !text.contains(KEY));
        assert!(text.contains("疑似误填密钥"));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), contents);
        assert!(!dir.join(FILE_NAME).exists());
    }
}

#[test]
fn pc06_readonly_legacy_web_models_preserve_all_key_modes_and_bytes() {
    for mode in ["inherit", "clear", "replace"] {
        let dir = TestDir::new("pc06-key-modes");
        let mut settings = test_settings(dir.path());
        settings.providers.tripo.api_key = Some(SecretString::new(KEY));
        settings.providers.manual_ai.api_key = Some(SecretString::new(KEY));
        let secrets = Secrets::fixed([35; 32]);
        let key = |purpose| {
            if mode == "replace" {
                json!({"mode":"replace","encrypted":secrets.encrypt(&SecretString::new("pc06-fake-web-key"), purpose).unwrap()})
            } else {
                json!({"mode":mode})
            }
        };
        let old = json!({"formatVersion":1,"revision":manual_core::ids::new_id(),"tripo":{"baseUrl":settings.providers.tripo.base_url,"model":CANARY,"key":key("overlay:tripo")},"manualAi":{"baseUrl":settings.providers.manual_ai.base_url,"model":CANARY,"key":key("overlay:manual-ai")}}).to_string();
        write_private_file(&dir.join(FILE_NAME), old.as_bytes(), false).unwrap();
        let store = ProviderConfigStore::load_from(&mut settings, secrets, false).unwrap();
        let view = serde_json::to_value(store.view()).unwrap();
        assert!(!view.to_string().contains(CANARY));
        for phase in ["active", "saved"] {
            for provider in ["tripo", "manualAi"] {
                assert_eq!(view[phase][provider]["model"], Value::Null);
                assert_eq!(view[phase][provider]["modelIssue"], "suspectedCredential");
                assert_eq!(view[phase][provider]["keyConfigured"], mode != "clear");
            }
        }
        assert_eq!(std::fs::read_to_string(dir.join(FILE_NAME)).unwrap(), old);
        assert!(!store.pending());
    }
}

async fn session(app: &TestApp) -> (String, String) {
    app.set_admin_password("pc06-fake-login-password").await;
    let r = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&json!({"password":"pc06-fake-login-password"}))
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
    assert!(!r.text().contains(CANARY));
    r.json()["data"].clone()
}
fn body(view: &Value) -> Value {
    let edit = |name: &str| json!({"action":"update","baseUrl":view["saved"][name]["baseUrl"],"model":view["saved"][name]["model"],"keyAction":"keep"});
    json!({"revision":view["revision"],"tripo":edit("tripo"),"manualAi":edit("manualAi")})
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
async fn pc06_new_suspect_models_are_refused_atomically_for_both_providers() {
    let app = TestApp::new("pc06-new").await;
    let (cookie, csrf) = session(&app).await;
    let view = read(&app, &cookie).await;
    for provider in ["tripo", "manualAi"] {
        for suspect in [
            CANARY.to_owned(),
            format!("  bEaReR  {CANARY} "),
            format!("\u{feff}{CANARY}\u{feff}"),
        ] {
            let mut write = body(&view);
            write[provider]["model"] = json!(suspect);
            write[provider]["keyAction"] = json!("replace");
            write[provider]["apiKey"] = json!("fake-key-must-not-be-saved");
            let r = put(&app, &cookie, &csrf, &write).await;
            assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(!r.text().contains(CANARY));
            assert_eq!(
                r.json()["error"]["details"]["fields"][0]["field"],
                format!("{provider}.model")
            );
            assert_eq!(read(&app, &cookie).await["revision"], view["revision"]);
            assert!(!app.dir().join(FILE_NAME).exists());
        }
    }
}

#[tokio::test]
async fn pc06_old_deployment_is_hidden_but_readable_and_correction_preserves_key_actions() {
    for action in ["keep", "replace", "clear"] {
        let dir = TestDir::new("pc06-deployment");
        let mut settings = test_settings(dir.path());
        settings.providers.manual_ai.model = Some(CANARY.into());
        settings.providers.manual_ai.api_key = Some(SecretString::new(KEY));
        let deployment = settings.clone();
        assert!(!format!("{settings:?}").contains(CANARY));
        assert!(!settings.providers.manual_ai.status_line().contains(CANARY));
        let app = TestApp::with_settings(dir, settings.clone()).await;
        *app.state().provider_config().write().await =
            ProviderConfigStore::deployment(&settings).with_secrets(Secrets::fixed([31; 32]));
        let (cookie, csrf) = session(&app).await;
        let view = read(&app, &cookie).await;
        assert_eq!(view["active"]["manualAi"]["model"], Value::Null);
        assert_eq!(
            view["active"]["manualAi"]["modelIssue"],
            "suspectedCredential"
        );
        assert_eq!(view["saved"]["manualAi"]["keyConfigured"], true);
        assert!(!app.dir().join(FILE_NAME).exists());
        let status = app
            .call(Method::GET, "/api/v1/settings/status")
            .cookie(&cookie)
            .send()
            .await;
        assert_eq!(status.status, StatusCode::OK);
        assert_eq!(
            status.json()["data"]["providerModelIssues"]["manualAi"],
            "suspectedCredential"
        );
        assert!(!status.text().contains(CANARY));
        assert_eq!(
            app.call(Method::GET, "/api/v1/health/ready")
                .send()
                .await
                .status,
            StatusCode::OK
        );
        let rejected = put(&app, &cookie, &csrf, &body(&view)).await;
        assert_eq!(rejected.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!app.dir().join(FILE_NAME).exists());
        let mut write = body(&view);
        write["manualAi"]["model"] = json!("org/custom-model");
        write["manualAi"]["keyAction"] = json!(action);
        if action == "replace" {
            write["manualAi"]["apiKey"] = json!("pc06-new-fake-key");
        }
        let r = put(&app, &cookie, &csrf, &write).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        assert_eq!(
            r.json()["data"]["active"]["manualAi"]["modelIssue"],
            "suspectedCredential"
        );
        assert_eq!(
            r.json()["data"]["saved"]["manualAi"]["modelIssue"],
            Value::Null
        );
        assert_eq!(r.json()["data"]["pending"], true);
        let bytes = std::fs::read_to_string(app.dir().join(FILE_NAME)).unwrap();
        assert!(
            !bytes.contains(CANARY) && !bytes.contains(KEY) && !bytes.contains("pc06-new-fake-key")
        );
        if action == "replace" {
            assert!(bytes.contains("ciphertext"));
        }
        let mut restarted = deployment.clone();
        let loaded =
            ProviderConfigStore::load_from(&mut restarted, Secrets::fixed([31; 32]), false)
                .unwrap();
        assert!(!loaded.pending());
        assert!(!restarted.providers.manual_ai.model_issue());
        assert_eq!(
            restarted.providers.manual_ai.model.as_deref(),
            Some("org/custom-model")
        );
        assert_eq!(
            restarted
                .providers
                .manual_ai
                .api_key
                .as_ref()
                .map(|k| k.expose()),
            match action {
                "replace" => Some("pc06-new-fake-key"),
                "clear" => None,
                _ => Some(KEY),
            }
        );
    }
}

#[tokio::test]
async fn pc06_existing_web_value_loads_without_rewriting_and_requires_explicit_clear() {
    let dir = TestDir::new("pc06-web");
    let mut settings = test_settings(dir.path());
    settings.providers.tripo.api_key = Some(SecretString::new(KEY));
    let deployment = settings.clone();
    let old = json!({"formatVersion":1,"revision":manual_core::ids::new_id(),"tripo":{"baseUrl":settings.providers.tripo.base_url,"model":CANARY,"key":{"mode":"inherit"}},"manualAi":null}).to_string();
    write_private_file(&dir.join(FILE_NAME), old.as_bytes(), false).unwrap();
    let store =
        ProviderConfigStore::load_from(&mut settings, Secrets::fixed([32; 32]), false).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join(FILE_NAME)).unwrap(), old);
    let app = TestApp::with_settings(dir, settings).await;
    *app.state().provider_config().write().await = store;
    let (cookie, csrf) = session(&app).await;
    let view = read(&app, &cookie).await;
    assert_eq!(view["saved"]["tripo"]["modelSource"], "web");
    let mut write = body(&view);
    write["manualAi"]["model"] = json!("local:model-v2");
    assert_eq!(
        put(&app, &cookie, &csrf, &write).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        std::fs::read_to_string(app.dir().join(FILE_NAME)).unwrap(),
        old
    );
    write["tripo"]["clearModel"] = json!(true);
    let r = put(&app, &cookie, &csrf, &write).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["data"]["saved"]["tripo"]["model"], Value::Null);
    assert_eq!(r.json()["data"]["saved"]["tripo"]["keyConfigured"], true);
    let mut restarted = deployment;
    ProviderConfigStore::load_from(&mut restarted, Secrets::fixed([32; 32]), false).unwrap();
    assert!(restarted.providers.tripo.model.is_none());
    assert_eq!(restarted.providers.tripo.api_key.unwrap().expose(), KEY);
    // A valid deployment is the third explicit correction path.
    let current = read(&app, &cookie).await;
    let mut restore = body(&current);
    restore["tripo"] = json!({"action":"restore"});
    let restored = put(&app, &cookie, &csrf, &restore).await;
    assert_eq!(restored.status, StatusCode::OK);
    assert_eq!(
        restored.json()["data"]["saved"]["tripo"]["model"],
        everything_manual::config::DEFAULT_TRIPO_MODEL
    );
    assert_eq!(
        restored.json()["data"]["saved"]["tripo"]["keyConfigured"],
        true
    );
}

#[tokio::test]
async fn pc06_restore_bad_deployment_refuses_without_revision_or_file_changes() {
    let dir = TestDir::new("pc06-restore");
    let mut settings = test_settings(dir.path());
    settings.providers.tripo.model = Some(CANARY.into());
    let old = json!({"formatVersion":1,"revision":manual_core::ids::new_id(),"tripo":{"baseUrl":settings.providers.tripo.base_url,"model":"sk-local","key":{"mode":"inherit"}},"manualAi":null}).to_string();
    write_private_file(&dir.join(FILE_NAME), old.as_bytes(), false).unwrap();
    let store =
        ProviderConfigStore::load_from(&mut settings, Secrets::fixed([33; 32]), false).unwrap();
    let app = TestApp::with_settings(dir, settings).await;
    *app.state().provider_config().write().await = store;
    let (cookie, csrf) = session(&app).await;
    let view = read(&app, &cookie).await;
    let mut write = body(&view);
    write["tripo"] = json!({"action":"restore"});
    write["manualAi"]["model"] = json!("org/custom-model");
    let r = put(&app, &cookie, &csrf, &write).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!r.text().contains(CANARY));
    assert!(r.text().contains("部署模型疑似误填密钥"));
    assert_eq!(
        std::fs::read_to_string(app.dir().join(FILE_NAME)).unwrap(),
        old
    );
    assert_eq!(read(&app, &cookie).await["revision"], view["revision"]);
}
