//! ES-01 isolated checks. Every master key is fake and explicitly injected; no Keychain IO.
mod common;
use common::{TestApp, TestDir, test_settings};
use everything_manual::config::{
    SecretString,
    encrypted_secrets::{
        Envelope, MasterKeySource, SecretError, Secrets, read_private_file, write_private_file,
    },
    provider_overrides::{FILE_NAME, ProviderConfigStore},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn aead_random_nonce_authenticated_metadata_and_wrong_key_fail_closed() {
    let secrets = Secrets::fixed([7; 32]);
    let value = SecretString::new("rd-fake-secret-canary");
    let envelope = secrets.encrypt(&value, "overlay:tripo").unwrap();
    let again = secrets.encrypt(&value, "overlay:tripo").unwrap();
    assert_ne!(envelope.nonce, again.nonce);
    assert_ne!(envelope.ciphertext, again.ciphertext);
    assert!(secrets.decrypt(&envelope, "overlay:tripo").unwrap() == value);
    assert!(
        Secrets::fixed([8; 32])
            .decrypt(&envelope, "overlay:tripo")
            .is_err()
    );
    assert!(secrets.decrypt(&envelope, "overlay:manual-ai").is_err());
    assert!(secrets.decrypt(&envelope, "api-key-file:tripo").is_err());
    for field in [
        "format",
        "version",
        "algorithm",
        "purpose",
        "nonce",
        "ciphertext",
    ] {
        let mut changed = serde_json::to_value(&envelope).unwrap();
        changed[field] = if field == "version" {
            json!(2)
        } else {
            json!("00")
        };
        let changed: Envelope = serde_json::from_value(changed).unwrap();
        assert!(secrets.decrypt(&changed, "overlay:tripo").is_err());
    }
}

struct Disappearing {
    reads: AtomicUsize,
    creates: AtomicUsize,
}
impl MasterKeySource for Disappearing {
    fn load(&self) -> Result<Option<zeroize::Zeroizing<[u8; 32]>>, SecretError> {
        Ok((self.reads.fetch_add(1, Ordering::SeqCst) == 0)
            .then(|| zeroize::Zeroizing::new([2; 32])))
    }
    fn create(&self) -> Result<zeroize::Zeroizing<[u8; 32]>, SecretError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Ok(zeroize::Zeroizing::new([3; 32]))
    }
}
#[test]
fn previously_used_master_disappearing_never_recreates() {
    let source = Arc::new(Disappearing {
        reads: AtomicUsize::new(0),
        creates: AtomicUsize::new(0),
    });
    let secrets = Secrets::with_source(source.clone());
    let secret = SecretString::new("fake-only");
    secrets.encrypt(&secret, "overlay:tripo").unwrap();
    assert!(secrets.clone().encrypt(&secret, "overlay:tripo").is_err());
    assert_eq!(source.creates.load(Ordering::SeqCst), 0);
    assert!(
        Secrets::unavailable()
            .decrypt(
                &Secrets::fixed([1; 32])
                    .encrypt(&secret, "overlay:tripo")
                    .unwrap(),
                "overlay:tripo"
            )
            .is_err()
    );
}

struct Switchable {
    present: std::sync::atomic::AtomicBool,
    creates: AtomicUsize,
}
impl MasterKeySource for Switchable {
    fn load(&self) -> Result<Option<zeroize::Zeroizing<[u8; 32]>>, SecretError> {
        Ok(self
            .present
            .load(Ordering::SeqCst)
            .then(|| zeroize::Zeroizing::new([3; 32])))
    }
    fn create(&self) -> Result<zeroize::Zeroizing<[u8; 32]>, SecretError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Ok(zeroize::Zeroizing::new([4; 32]))
    }
}
#[tokio::test]
async fn lost_master_prevents_keep_clear_restore_from_overwriting_saved_ciphertext() {
    let dir = TestDir::new("encrypted-live-loss");
    let settings = test_settings(dir.path());
    let app = TestApp::with_settings(dir, settings.clone()).await;
    let source = Arc::new(Switchable {
        present: std::sync::atomic::AtomicBool::new(true),
        creates: AtomicUsize::new(0),
    });
    let mut store = ProviderConfigStore::deployment(&settings)
        .with_secrets(Secrets::with_source(source.clone()));
    let request = |revision: &str, action: &str| {
        serde_json::from_value(json!({"revision":revision,"tripo":if action=="restore" { json!({"action":"restore"}) } else { json!({"action":"update","baseUrl":if action=="replace" { "https://example.test/v3" } else { "https://example.test/new" },"model":"fixture","keyAction":action,"apiKey":if action=="replace" { Some("live-loss-fake-key") } else { None }}) },"manualAi":{"action":"restore"}})).unwrap()
    };
    store
        .save(
            request("deployment", "replace"),
            app.state().database().pool(),
        )
        .await
        .unwrap();
    let revision = store.revision().to_owned();
    let original = std::fs::read(app.dir().join(FILE_NAME)).unwrap();
    source.present.store(false, Ordering::SeqCst);
    for action in ["keep", "clear", "restore"] {
        assert!(
            store
                .save(request(&revision, action), app.state().database().pool())
                .await
                .is_err()
        );
        assert_eq!(store.revision(), revision);
        assert!(std::fs::read(app.dir().join(FILE_NAME)).unwrap() == original);
    }
    assert_eq!(source.creates.load(Ordering::SeqCst), 0);
}

#[test]
fn external_files_convert_without_overwriting_source_and_reject_plaintext() {
    let dir = TestDir::new("encrypted-file");
    let input = dir.join("plain.key");
    let output = dir.join("key.api-key.enc");
    write_private_file(&input, b"rd-local-file-canary\n", false).unwrap();
    let secrets = Secrets::fixed([8; 32]);
    assert_eq!(
        secrets.read_api_key(&input, "tripo").unwrap_err(),
        SecretError::LegacyFile
    );
    secrets
        .encrypt_api_key_file(&input, &output, "tripo")
        .unwrap();
    assert!(secrets.read_api_key(&output, "tripo").unwrap().expose() == "rd-local-file-canary");
    assert!(secrets.read_api_key(&output, "manual-ai").is_err());
    assert!(
        secrets
            .encrypt_api_key_file(&input, &output, "tripo")
            .is_err()
    );
    assert!(read_private_file(&input).unwrap().as_slice() == b"rd-local-file-canary\n");
    assert!(
        !std::fs::read_to_string(&output)
            .unwrap()
            .contains("rd-local-file-canary")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        assert_eq!(
            std::fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );
        symlink(&input, dir.join("link")).unwrap();
        assert!(read_private_file(&dir.join("link")).is_err());
        std::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private_file(&input).is_err());
    }
    assert!(read_private_file(dir.path()).is_err());
}

fn legacy(revision: &str) -> Value {
    json!({"revision":revision,"tripo":{"baseUrl":"https://example.test/v3","model":"v3.1-20260211","key":{"mode":"replace","value":"rd-legacy-canary"}},"manualAi":null})
}
#[test]
fn locked_migration_preserves_revision_and_read_only_check_changes_nothing() {
    let dir = TestDir::new("encrypted-migration");
    let mut settings = test_settings(dir.path());
    let revision = manual_core::ids::new_id();
    let bytes = serde_json::to_vec(&legacy(&revision)).unwrap();
    write_private_file(&dir.join(FILE_NAME), &bytes, false).unwrap();
    assert!(ProviderConfigStore::load_from(&mut settings, Secrets::fixed([5; 32]), false).is_err());
    assert!(std::fs::read(dir.join(FILE_NAME)).unwrap() == bytes);
    let loaded =
        ProviderConfigStore::load_from(&mut settings, Secrets::fixed([5; 32]), true).unwrap();
    assert_eq!(loaded.revision(), revision);
    assert!(!loaded.pending());
    assert!(
        loaded
            .ensure_revision(&json!({"configRevision":revision}))
            .is_ok()
    );
    assert!(settings.providers.tripo.api_key.as_ref().unwrap().expose() == "rd-legacy-canary");
    assert!(
        !std::fs::read_to_string(dir.join(FILE_NAME))
            .unwrap()
            .contains("rd-legacy-canary")
    );
    assert!(
        ProviderConfigStore::load_from(
            &mut test_settings(dir.path()),
            Secrets::fixed([5; 32]),
            false
        )
        .is_ok()
    );
}
#[test]
fn failed_or_malformed_migration_leaves_original_bytes() {
    for malformed in [false, true] {
        let dir = TestDir::new("encrypted-migration-failure");
        let mut settings = test_settings(dir.path());
        let bytes = if malformed {
            b"{bad-json".to_vec()
        } else {
            serde_json::to_vec(&legacy(&manual_core::ids::new_id())).unwrap()
        };
        write_private_file(&dir.join(FILE_NAME), &bytes, false).unwrap();
        assert!(
            ProviderConfigStore::load_from(&mut settings, Secrets::unavailable(), true).is_err()
        );
        assert!(std::fs::read(dir.join(FILE_NAME)).unwrap() == bytes);
    }
}

#[tokio::test]
async fn web_save_encrypts_both_and_restart_retains_epoch_at_maximum_size() {
    use axum::http::Method;
    let dir = TestDir::new("encrypted-web");
    let settings = test_settings(dir.path());
    let app = TestApp::with_settings(dir, settings.clone()).await;
    *app.state().provider_config().write().await =
        ProviderConfigStore::deployment(&settings).with_secrets(Secrets::fixed([9; 32]));
    app.set_admin_password("fake-password-for-rd").await;
    let login = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&json!({"password":"fake-password-for-rd"}))
        .send()
        .await;
    let cookie = login.session_cookie();
    let csrf = login.json()["data"]["csrfToken"]
        .as_str()
        .unwrap()
        .to_owned();
    let key = "密".repeat(4096);
    let request = json!({"revision":"deployment","tripo":{"action":"update","baseUrl":"https://example.test/v3","model":"model","keyAction":"replace","apiKey":key},"manualAi":{"action":"update","baseUrl":"https://example.test/v1","model":"model","keyAction":"replace","apiKey":key}});
    let r = app
        .call(Method::PUT, "/api/v1/settings/providers")
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&request)
        .send()
        .await;
    assert_eq!(r.status, axum::http::StatusCode::OK);
    let revision = r.json()["data"]["revision"].as_str().unwrap().to_owned();
    assert!(!r.text().contains(&key));
    assert!(
        !std::fs::read_to_string(app.dir().join(FILE_NAME))
            .unwrap()
            .contains(&key)
    );
    let mut settings = settings;
    let reloaded =
        ProviderConfigStore::load_from(&mut settings, Secrets::fixed([9; 32]), false).unwrap();
    assert!(!reloaded.pending());
    assert_eq!(reloaded.revision(), revision);
    assert!(settings.providers.tripo.api_key.as_ref().unwrap().expose() == key);
    assert!(
        settings
            .providers
            .manual_ai
            .api_key
            .as_ref()
            .unwrap()
            .expose()
            == key
    );
}

#[test]
fn cli_uses_explicit_master_and_never_accepts_plaintext_as_argument() {
    let dir = TestDir::new("encrypted-cli");
    let input = dir.join("plain.key");
    let output = dir.join("new.api-key.enc");
    write_private_file(&input, b"rd-command-canary", false).unwrap();
    let run = |master: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_everything-manual"))
            .args(["encrypt-api-key", "--provider", "manual-ai", "--input"])
            .arg(&input)
            .arg("--output")
            .arg(&output)
            .env("EM_SECRETS_MASTER_KEY", master)
            .output()
            .unwrap()
    };
    let invalid = run("invalid-fake-master");
    assert!(!invalid.status.success());
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("invalid-fake-master"));
    assert!(!output.exists());
    let valid = run(&"ab".repeat(32));
    assert!(valid.status.success());
    assert!(
        Secrets::fixed([0xab; 32])
            .read_api_key(&output, "manual-ai")
            .unwrap()
            .expose()
            == "rd-command-canary"
    );
    assert!(!run(&"ab".repeat(32)).status.success());
}

#[test]
fn known_key_reflection_detects_direct_and_decoded_json_without_url_echo() {
    let key = SecretString::new("fake-canary");
    for bytes in [
        br#"{"message":"fake-canary"}"#.as_slice(),
        br#"{"message":"\u0066ake-canary"}"#,
        br#"{"\u0066ake-canary":"value"}"#,
    ] {
        assert!(everything_manual::config::secret::response_requires_discard(bytes, &key));
    }
    assert!(
        !everything_manual::config::secret::response_requires_discard(
            br#"{"message":"ordinary"}"#,
            &key
        )
    );
    let result = everything_manual::providers::tripo::client::TripoClient::new(
        "https://user:fake-canary@example.test",
        key,
        everything_manual::providers::tripo::client::TripoTimeouts::default(),
    );
    assert!(result.is_err());
    assert!(!result.err().unwrap().contains("fake-canary"));
}

/// BUG-ES-001: malformed envelopes cannot bypass decoded-key checks or reach raw diagnostics.
/// Controls verify valid bodies and HTTP/business classifications, without printing secrets.
#[tokio::test]
async fn malformed_json_escaped_response_is_discarded_and_classification_preserved() {
    use everything_manual::providers::{
        manual_ai::{ManualAiClient, ManualAiError, ManualAiTimeouts},
        tripo::{TripoClient, TripoError, TripoTimeouts},
    };
    use test_support::{
        FixtureServer,
        scenario::{BodySpec, PathMatchSpec, ResponseSpec, RouteScript, Scenario, Step},
    };
    let key = SecretString::new("rd-es-bug-only");
    let escaped_all: String = key
        .expose()
        .encode_utf16()
        .map(|value| format!("\\u{value:04x}"))
        .collect();
    let escaped_mixed = format!("\\u0072{}", &key.expose()[1..]);
    for escaped in [escaped_all, escaped_mixed] {
        let quoted = format!("\"{escaped}\"");
        assert!(
            serde_json::from_str::<String>(&quoted)
                .ok()
                .is_some_and(|decoded| decoded == key.expose())
        );
        let valid = format!("{{\"code\":123,\"message\":{quoted}}}");
        let malformed = format!("{{\"code\":123,\"message\":{quoted},");
        assert!(
            everything_manual::config::secret::response_requires_discard(valid.as_bytes(), &key)
        );
        assert!(
            everything_manual::config::secret::response_requires_discard(
                malformed.as_bytes(),
                &key
            )
        );
        assert!(
            !everything_manual::config::secret::response_requires_discard(
                br#"{"message":"ordinary safe message"}"#,
                &key
            )
        );
        let text_step = |status: u16, text: &str| Step::Respond {
            response: ResponseSpec {
                status,
                headers: std::collections::BTreeMap::from([("retry-after".into(), "17".into())]),
                body: BodySpec::Text { text: text.into() },
            },
        };
        let route = |path: &str, steps| RouteScript {
            method: "POST".into(),
            path: path.into(),
            path_match: PathMatchSpec::Exact,
            repeat_last: false,
            steps,
        };
        let server = FixtureServer::start(Scenario::new(vec![
            route(
                "/v3/generation/multiview-to-model",
                vec![
                    text_step(400, &malformed),
                    text_step(200, r#"{"code":0,"data":{"task_id":"safe-task"}}"#),
                    text_step(422, r#"{"code":401,"message":"safe rejection"}"#),
                    text_step(200, r#"{"code":422,"message":"safe business rejection"}"#),
                    text_step(429, &malformed),
                    text_step(503, &malformed),
                ],
            ),
            route(
                "/v1/responses",
                vec![
                    text_step(200, &malformed),
                    Step::Respond {
                        response: ResponseSpec {
                            status: 200,
                            headers: Default::default(),
                            body: BodySpec::File {
                                file: "responses/manual_ai/success.json".into(),
                            },
                        },
                    },
                    text_step(400, r#"{"error":{"message":"safe rejection"}}"#),
                    text_step(429, &malformed),
                    text_step(503, &malformed),
                ],
            ),
        ]));
        let tripo = TripoClient::new(
            &format!("{}/v3", server.base_url()),
            key.clone(),
            TripoTimeouts::default(),
        )
        .ok()
        .expect("fixture client must construct");
        let error = tripo
            .submit_multiview(b"{}")
            .await
            .err()
            .expect("business error");
        assert!(matches!(
            error,
            TripoError::Business {
                http_status: 400,
                ..
            }
        ));
        assert!(error.is_definitively_refused());
        assert!(!error.redacted().contains(&quoted));
        assert!(!error.redacted().contains(key.expose()));
        assert!(
            error
                .redacted()
                .contains("供应商响应包含敏感凭据或无法安全解析，内容已丢弃")
        );
        let success = tripo
            .submit_multiview(b"{}")
            .await
            .ok()
            .expect("valid safe response succeeds");
        assert!(success.task_id == "safe-task");
        let error = tripo
            .submit_multiview(b"{}")
            .await
            .err()
            .expect("business error");
        assert!(matches!(
            error,
            TripoError::Business {
                http_status: 422,
                code: Some(401),
                ..
            }
        ));
        assert!(error.redacted().contains("safe rejection"));
        let error = tripo
            .submit_multiview(b"{}")
            .await
            .err()
            .expect("business error");
        assert!(matches!(
            error,
            TripoError::Business {
                http_status: 200,
                code: Some(422),
                ..
            }
        ));
        assert!(matches!(
            tripo.submit_multiview(b"{}").await,
            Err(TripoError::RateLimited {
                retry_after_seconds: Some(17)
            })
        ));
        assert!(matches!(
            tripo.submit_multiview(b"{}").await,
            Err(TripoError::ServerError { status: 503 })
        ));

        let manual = ManualAiClient::new(
            &format!("{}/v1", server.base_url()),
            key.clone(),
            ManualAiTimeouts::default(),
        )
        .ok()
        .expect("fixture client must construct");
        let error = manual
            .extract_batch(b"{}")
            .await
            .err()
            .expect("unsafe response must not return RawResponse");
        assert!(matches!(error, ManualAiError::Unexpected { .. }));
        assert!(!error.is_definitively_refused());
        assert!(!error.redacted().contains(&quoted));
        assert!(!error.redacted().contains(key.expose()));
        assert!(
            error
                .redacted()
                .contains("供应商响应包含敏感凭据或无法安全解析，内容已丢弃")
        );
        let raw = manual
            .extract_batch(b"{}")
            .await
            .ok()
            .expect("valid safe response succeeds");
        assert!(ManualAiClient::parse_success(&raw).is_ok());
        let error = manual
            .extract_batch(b"{}")
            .await
            .err()
            .expect("business error");
        assert!(matches!(
            error,
            ManualAiError::Business {
                http_status: 400,
                ..
            }
        ));
        assert!(error.redacted().contains("safe rejection"));
        assert!(matches!(
            manual.extract_batch(b"{}").await,
            Err(ManualAiError::RateLimited {
                retry_after_seconds: Some(17)
            })
        ));
        assert!(matches!(
            manual.extract_batch(b"{}").await,
            Err(ManualAiError::ServerError { status: 503 })
        ));
        assert_eq!(
            server.call_count("POST", "/v3/generation/multiview-to-model"),
            6
        );
        assert_eq!(server.call_count("POST", "/v1/responses"), 5);
        server.assert_no_script_problems();
    }
}
