//! ES-01 independent QA. Random canaries remain inside isolated memory/temp paths.
//! Native Keychain is opt-in: explicitly execute the ignored parent test, which uses
//! its own random service/account and cleans only that entry. Ordinary tests use DI.
mod common;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use common::TestDir;
use everything_manual::config::SecretString;
#[cfg(target_os = "macos")]
use everything_manual::config::encrypted_secrets::Envelope;
use everything_manual::config::encrypted_secrets::{MasterKeySource, SecretError, Secrets};
use everything_manual::config::provider_overrides::{FILE_NAME, ProviderConfigStore};
use serde_json::{Value, json};
use zeroize::Zeroizing;

fn random_master() -> [u8; 32] {
    let mut value = [0_u8; 32];
    getrandom::fill(&mut value).expect("OS random available");
    value
}

fn canary() -> SecretString {
    SecretString::new(format!("qa-{}", manual_core::ids::new_id()))
}

fn private_write(path: &Path, value: &[u8]) {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .expect("QA private file")
        .write_all(value)
        .expect("QA fixture write");
}

fn contains(bytes: &[u8], secret: &str) -> bool {
    bytes
        .windows(secret.len())
        .any(|window| window == secret.as_bytes())
}

fn no_secret(bytes: &[u8], secret: &SecretString) {
    assert!(
        !contains(bytes, secret.expose()),
        "secret presence must be false"
    );
}

fn mutate_hex(value: &mut String) {
    let first = if value.starts_with('0') { "1" } else { "0" };
    value.replace_range(0..1, first);
}

struct CountingSource {
    loads: AtomicUsize,
    creates: AtomicUsize,
    reject_load: bool,
}
impl CountingSource {
    fn new(reject_load: bool) -> Arc<Self> {
        Arc::new(Self {
            loads: AtomicUsize::new(0),
            creates: AtomicUsize::new(0),
            reject_load,
        })
    }
}
impl MasterKeySource for CountingSource {
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>, SecretError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        if self.reject_load {
            Err(SecretError::MasterUnavailable)
        } else {
            Ok(None)
        }
    }
    fn create(&self) -> Result<Zeroizing<[u8; 32]>, SecretError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Err(SecretError::MasterUnavailable)
    }
}

#[test]
fn es_qa_01_aead_random_nonce_and_purpose_roundtrip() {
    let secrets = Secrets::fixed(random_master());
    let secret = canary();
    for purpose in [
        "overlay:tripo",
        "overlay:manual-ai",
        "api-key-file:tripo",
        "api-key-file:manual-ai",
    ] {
        let first = secrets.encrypt(&secret, purpose).expect("encrypt first");
        let second = secrets.encrypt(&secret, purpose).expect("encrypt second");
        assert_eq!(first.format, "everything-manual-secret");
        assert_eq!(first.version, 1);
        assert_eq!(first.algorithm, "AES-256-GCM");
        assert_eq!(first.purpose, purpose);
        assert_eq!(first.nonce.len(), 24);
        assert_ne!(first.nonce, second.nonce);
        assert_ne!(first.ciphertext, second.ciphertext);
        assert!(
            secrets.decrypt(&first, purpose).unwrap() == secret,
            "first plaintext matches in memory"
        );
        assert!(
            secrets.decrypt(&second, purpose).unwrap() == secret,
            "second plaintext matches in memory"
        );
        no_secret(&serde_json::to_vec(&first).unwrap(), &secret);
        no_secret(format!("{secret:?}").as_bytes(), &secret);
    }
}

#[test]
fn es_qa_02_tamper_wrong_key_and_cross_purpose_fail_closed() {
    let secrets = Secrets::fixed(random_master());
    let secret = canary();
    let envelope = secrets.encrypt(&secret, "overlay:tripo").unwrap();
    let mut mutations = Vec::new();
    let mut changed = envelope.clone();
    mutate_hex(&mut changed.ciphertext);
    mutations.push(changed);
    let mut changed = envelope.clone();
    mutate_hex(&mut changed.nonce);
    mutations.push(changed);
    let mut changed = envelope.clone();
    changed.ciphertext.truncate(2);
    mutations.push(changed);
    let mut changed = envelope.clone();
    changed.nonce.truncate(2);
    mutations.push(changed);
    let mut changed = envelope.clone();
    changed.nonce.replace_range(0..1, "z");
    mutations.push(changed);
    let mut changed = envelope.clone();
    changed.version += 1;
    mutations.push(changed);
    let mut changed = envelope.clone();
    changed.algorithm = "AES-128-GCM".into();
    mutations.push(changed);
    let mut changed = envelope.clone();
    changed.format = "unrecognized".into();
    mutations.push(changed);
    let mut changed = envelope.clone();
    changed.purpose = "api-key-file:tripo".into();
    mutations.push(changed);
    for changed in mutations {
        let error = secrets
            .decrypt(&changed, "overlay:tripo")
            .expect_err("tampered ciphertext must reject");
        no_secret(error.to_string().as_bytes(), &secret);
    }
    for purpose in [
        "overlay:manual-ai",
        "api-key-file:tripo",
        "api-key-file:manual-ai",
    ] {
        assert!(
            secrets.decrypt(&envelope, purpose).is_err(),
            "cross-purpose use rejected"
        );
    }
    let mut rebound = envelope.clone();
    rebound.purpose = "api-key-file:tripo".into();
    assert!(
        secrets.decrypt(&rebound, "api-key-file:tripo").is_err(),
        "rewriting metadata cannot rebind AAD"
    );
    assert!(
        Secrets::fixed(random_master())
            .decrypt(&envelope, "overlay:tripo")
            .is_err(),
        "wrong master rejected"
    );
}

#[test]
fn es_qa_04_missing_or_denied_master_never_regenerates_on_read() {
    let secret = canary();
    let envelope = Secrets::fixed(random_master())
        .encrypt(&secret, "overlay:tripo")
        .unwrap();
    for denied in [false, true] {
        let source = CountingSource::new(denied);
        let secrets = Secrets::with_source(source.clone());
        assert!(secrets.decrypt(&envelope, "overlay:tripo").is_err());
        assert_eq!(
            source.creates.load(Ordering::SeqCst),
            0,
            "existing ciphertext must never create a replacement master"
        );
        assert!(secrets.encrypt(&secret, "overlay:tripo").is_err());
        assert_eq!(source.creates.load(Ordering::SeqCst), usize::from(!denied));
    }
    assert!(
        Secrets::unavailable()
            .encrypt(&secret, "overlay:tripo")
            .is_err()
    );
}

#[test]
fn es_qa_07_file_conversion_preserves_source_and_refuses_clobber() {
    let dir = TestDir::new("encrypted-secrets-qa-files");
    let secrets = Secrets::fixed(random_master());
    let secret = canary();
    let source = dir.join("legacy-input");
    private_write(&source, format!("{}\n", secret.expose()).as_bytes());
    let before = fs::read(&source).unwrap();
    for provider in ["tripo", "manual-ai"] {
        assert_eq!(
            secrets.read_api_key(&source, provider).unwrap_err(),
            SecretError::LegacyFile
        );
        let output = dir.join(&format!("{provider}.encrypted"));
        secrets
            .encrypt_api_key_file(&source, &output, provider)
            .expect("explicit conversion");
        let encrypted = fs::read(&output).unwrap();
        no_secret(&encrypted, &secret);
        assert!(
            secrets.read_api_key(&output, provider).unwrap() == secret,
            "reader matches only in memory"
        );
        let other = if provider == "tripo" {
            "manual-ai"
        } else {
            "tripo"
        };
        assert!(
            secrets.read_api_key(&output, other).is_err(),
            "provider-purpose binding"
        );
        assert!(
            secrets
                .encrypt_api_key_file(&source, &output, provider)
                .is_err(),
            "existing destination not overwritten"
        );
        assert!(
            fs::read(&output).unwrap() == encrypted,
            "existing ciphertext unchanged"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&output).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    assert!(
        fs::read(source).unwrap() == before,
        "external source unchanged"
    );
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        3,
        "no plaintext backup or temporary file"
    );
}

#[test]
fn es_qa_08_unsafe_files_and_failed_output_are_rejected() {
    let dir = TestDir::new("encrypted-secrets-qa-unsafe");
    let secret = canary();
    let secrets = Secrets::fixed(random_master());
    let source = dir.join("source");
    private_write(&source, secret.expose().as_bytes());
    let before = fs::read(&source).unwrap();
    let too_large = dir.join("oversize");
    private_write(&too_large, &vec![b'x'; 262_145]);
    let directory = dir.join("directory");
    fs::create_dir(&directory).unwrap();
    for invalid in [&too_large, &directory] {
        assert!(secrets.read_api_key(invalid, "tripo").is_err());
        assert!(
            secrets
                .encrypt_api_key_file(invalid, &dir.join("new-output"), "tripo")
                .is_err()
        );
        assert!(!dir.join("new-output").exists());
    }
    assert!(
        secrets
            .encrypt_api_key_file(&source, &dir.join("absent-parent/key"), "tripo")
            .is_err()
    );
    assert!(
        Secrets::unavailable()
            .encrypt_api_key_file(&source, &dir.join("no-master"), "tripo")
            .is_err()
    );
    assert!(!dir.join("no-master").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let linked = dir.join("linked");
        symlink(&source, &linked).unwrap();
        assert!(secrets.read_api_key(&linked, "tripo").is_err());
        assert!(
            secrets
                .encrypt_api_key_file(&linked, &dir.join("from-link"), "tripo")
                .is_err()
        );
        assert!(
            secrets
                .encrypt_api_key_file(&source, &linked, "tripo")
                .is_err()
        );
        fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            secrets
                .encrypt_api_key_file(&source, &dir.join("too-open"), "tripo")
                .is_err()
        );
        assert!(secrets.read_api_key(&source, "tripo").is_err());
    }
    assert!(
        fs::read(source).unwrap() == before,
        "source unchanged after all refusals"
    );
    assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

fn legacy_provider(key: Value) -> Value {
    json!({"baseUrl":"https://qa.example.invalid/v1", "model":"qa-model", "key":key})
}

fn master_hex(key: &[u8; 32]) -> String {
    key.iter().map(|value| format!("{value:02x}")).collect()
}

fn cli(dir: &Path, args: &[&str], master: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_everything-manual"))
        .args(args)
        .current_dir(dir)
        .env_clear()
        .env("EM_SECRETS_MASTER_KEY", master)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run isolated CLI")
}

fn no_cli_secret(output: &Output, secret: &SecretString, master: &str) {
    for bytes in [&output.stdout, &output.stderr] {
        no_secret(bytes, secret);
        assert!(
            !contains(bytes, master),
            "master output presence must be false"
        );
    }
}

#[test]
fn es_qa_07_cli_conversion_env_validation_and_plaintext_guidance() {
    let dir = TestDir::new("encrypted-secrets-qa-cli");
    let master = random_master();
    let encoded = master_hex(&master);
    let secret = canary();
    private_write(&dir.join("source"), secret.expose().as_bytes());
    let args = [
        "encrypt-api-key",
        "--provider",
        "tripo",
        "--input",
        "source",
        "--output",
        "result",
    ];
    for invalid in [
        "0".repeat(63),
        "0".repeat(65),
        "g".repeat(64),
        "é".repeat(32),
        String::new(),
    ] {
        let output = cli(dir.path(), &args, &invalid);
        assert!(!output.status.success(), "bad explicit master must fail");
        no_cli_secret(&output, &secret, &encoded);
        assert!(!dir.join("result").exists());
    }
    let output = cli(dir.path(), &args, &encoded);
    assert!(output.status.success(), "valid CLI conversion succeeds");
    no_cli_secret(&output, &secret, &encoded);
    assert!(
        Secrets::fixed(master)
            .read_api_key(&dir.join("result"), "tripo")
            .unwrap()
            == secret
    );
    let before = fs::read(dir.join("result")).unwrap();
    let duplicate = cli(dir.path(), &args, &encoded);
    assert!(!duplicate.status.success());
    assert!(fs::read(dir.join("result")).unwrap() == before);
    no_cli_secret(&duplicate, &secret, &encoded);
    let help = cli(dir.path(), &["encrypt-api-key", "--help"], &encoded);
    assert!(help.status.success());
    for name in ["--provider", "--input", "--output"] {
        assert!(String::from_utf8_lossy(&help.stdout).contains(name));
    }
    no_cli_secret(&help, &secret, &encoded);
    for provider in ["tripo", "manual_ai"] {
        fs::write(
            dir.join("config.toml"),
            format!("[providers.{provider}]\napi_key_file = \"source\"\n"),
        )
        .unwrap();
        let check = cli(dir.path(), &["check", "--data-dir", "data"], &encoded);
        assert!(
            !check.status.success(),
            "ordinary check rejects unencrypted external file"
        );
        assert!(
            String::from_utf8_lossy(&check.stderr).contains("encrypt-api-key"),
            "concrete conversion guidance"
        );
        no_cli_secret(&check, &secret, &encoded);
    }
    assert!(fs::read(dir.join("source")).unwrap() == secret.expose().as_bytes());
}

#[test]
fn es_qa_11_deployment_url_credentials_are_rejected_without_echo() {
    let dir = TestDir::new("encrypted-secrets-qa-url");
    let secret = canary();
    let encoded = master_hex(&random_master());
    for provider in ["tripo", "manual_ai"] {
        for url in [
            format!("https://user:{}@example.invalid/v1", secret.expose()),
            format!("https://example.invalid/v1?key={}", secret.expose()),
            format!("not-a-url:{}", secret.expose()),
        ] {
            fs::write(
                dir.join("config.toml"),
                format!(
                    "[providers.{provider}]\nbase_url = {}\n",
                    serde_json::to_string(&url).unwrap()
                ),
            )
            .unwrap();
            let check = cli(dir.path(), &["check", "--data-dir", "data"], &encoded);
            assert!(!check.status.success());
            no_cli_secret(&check, &secret, &encoded);
        }
    }
}

#[derive(Clone)]
struct MemoryLog(Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for MemoryLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn es_qa_10_directory_sync_before_and_after_commit_preserves_truth() {
    use everything_manual::config::encrypted_secrets::test_faults::fail_directory_sync;
    let app = common::TestApp::new("encrypted-secrets-qa-commit").await;
    let secrets = Secrets::fixed(random_master());
    let mut store =
        ProviderConfigStore::deployment(app.state().settings()).with_secrets(secrets.clone());
    let secret = canary();
    let write = |revision: &str,
                 model: &str|
     -> everything_manual::http::dto::ProviderSettingsWrite {
        serde_json::from_value(json!({"revision":revision,
          "tripo":{"action":"update","baseUrl":"https://qa.example.invalid/v3","model":model,"keyAction":"replace","apiKey":secret.expose()},
          "manualAi":{"action":"restore"}})).unwrap()
    };
    store
        .save(
            write("deployment", "qa-initial"),
            app.state().database().pool(),
        )
        .await
        .unwrap();
    let file = app.dir().join(FILE_NAME);
    let before = fs::read(&file).unwrap();
    let previous = store.revision().to_owned();
    let before_view = serde_json::to_value(store.view()).unwrap();
    {
        let _fault = fail_directory_sync(&file, false);
        assert!(
            store
                .save(
                    write(&previous, "qa-rejected"),
                    app.state().database().pool()
                )
                .await
                .is_err()
        );
    }
    assert!(
        fs::read(&file).unwrap() == before,
        "pre-commit failure leaves original ciphertext"
    );
    assert_eq!(serde_json::to_value(store.view()).unwrap(), before_view);
    let log = MemoryLog(Arc::new(std::sync::Mutex::new(Vec::new())));
    let sink = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || sink.clone())
        .finish();
    {
        let _logging = tracing::subscriber::set_default(subscriber);
        let _fault = fail_directory_sync(&file, true);
        store
            .save(
                write(&previous, "qa-committed"),
                app.state().database().pool(),
            )
            .await
            .expect("post-commit durability warning is not an uncommitted error");
    }
    assert_ne!(store.revision(), previous);
    assert!(store.pending());
    assert_eq!(
        store.view().saved.tripo.model.as_deref(),
        Some("qa-committed")
    );
    let mut settings = app.state().settings().clone();
    let loaded = ProviderConfigStore::load_from(&mut settings, secrets, false).unwrap();
    assert_eq!(
        loaded.revision(),
        store.revision(),
        "published file and saved memory revision agree"
    );
    assert!(settings.providers.tripo.api_key.as_ref() == Some(&secret));
    no_secret(&fs::read(file).unwrap(), &secret);
    let log = log.0.lock().unwrap();
    assert!(
        String::from_utf8_lossy(&log).contains("加密配置已提交，但目录同步失败"),
        "safe durability warning captured"
    );
    no_secret(&log, &secret);
}

#[test]
fn es_qa_05_legacy_conversion_keeps_revision_sources_and_epoch() {
    let secret = canary();
    let replacement = || json!({"mode":"replace", "value":secret.expose()});
    let cases = [
        (
            legacy_provider(replacement()),
            legacy_provider(replacement()),
        ),
        (
            legacy_provider(json!({"mode":"inherit"})),
            legacy_provider(json!({"mode":"clear"})),
        ),
        (Value::Null, legacy_provider(replacement())),
    ];
    for (tripo, manual_ai) in cases {
        let dir = TestDir::new("encrypted-secrets-qa-migrate");
        let mut settings = common::test_settings(dir.path());
        settings.providers.tripo.api_key = Some(canary());
        settings.providers.manual_ai.api_key = Some(canary());
        let deployment = settings.providers.clone();
        let revision = manual_core::ids::new_id();
        let legacy = json!({"revision":revision, "tripo":tripo, "manualAi":manual_ai});
        let file = dir.join(FILE_NAME);
        private_write(&file, &serde_json::to_vec(&legacy).unwrap());
        let secrets = Secrets::fixed(random_master());
        let store = ProviderConfigStore::load_from(&mut settings, secrets.clone(), true)
            .expect("valid legacy migration");
        assert_eq!(store.revision(), revision);
        assert!(
            !store.pending(),
            "format conversion does not create pending configuration"
        );
        assert!(
            store
                .ensure_revision(&json!({"configRevision":revision}))
                .is_ok(),
            "old matching epoch stays valid"
        );
        for (name, provider, original) in [
            ("tripo", &settings.providers.tripo, &deployment.tripo),
            (
                "manualAi",
                &settings.providers.manual_ai,
                &deployment.manual_ai,
            ),
        ] {
            match legacy[name]["key"]["mode"].as_str() {
                Some("replace") => assert!(
                    provider.api_key.as_ref() == Some(&secret),
                    "migrated value matches only in memory"
                ),
                Some("clear") => {
                    assert!(provider.api_key.is_none(), "clear still masks deployment")
                }
                Some("inherit") | None => assert!(
                    provider.api_key == original.api_key,
                    "keep/restore still inherits deployment"
                ),
                _ => panic!("unexpected QA legacy mode"),
            }
        }
        let bytes = fs::read(&file).unwrap();
        no_secret(&bytes, &secret);
        let view = serde_json::to_value(store.view()).unwrap();
        let mut restarted = common::test_settings(dir.path());
        restarted.providers = deployment;
        let loaded = ProviderConfigStore::load_from(&mut restarted, secrets, false)
            .expect("read migrated ciphertext");
        assert_eq!(serde_json::to_value(loaded.view()).unwrap(), view);
        assert!(
            fs::read(&file).unwrap() == bytes,
            "read does not re-encrypt or change nonce"
        );
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no migration backup or temporary artifact"
        );
    }
}

#[test]
fn es_qa_06_check_and_failed_migration_leave_source_unchanged() {
    let dir = TestDir::new("encrypted-secrets-qa-migration-failure");
    let secret = canary();
    let legacy = json!({"revision":manual_core::ids::new_id(), "tripo":legacy_provider(json!({"mode":"replace","value":secret.expose()})), "manualAi":null});
    let bytes = serde_json::to_vec(&legacy).unwrap();
    let file = dir.join(FILE_NAME);
    private_write(&file, &bytes);
    let source = CountingSource::new(false);
    let mut settings = common::test_settings(dir.path());
    let check =
        ProviderConfigStore::load_from(&mut settings, Secrets::with_source(source.clone()), false);
    assert!(check.is_err(), "read-only check asks for migration");
    assert_eq!(source.loads.load(Ordering::SeqCst), 0);
    assert_eq!(source.creates.load(Ordering::SeqCst), 0);
    assert!(
        fs::read(&file).unwrap() == bytes,
        "check leaves legacy bytes unchanged"
    );
    assert!(ProviderConfigStore::load_from(&mut settings, Secrets::unavailable(), true).is_err());
    assert!(
        fs::read(&file).unwrap() == bytes,
        "missing master leaves original unchanged"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let result =
            ProviderConfigStore::load_from(&mut settings, Secrets::fixed(random_master()), true);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            result.is_err(),
            "migration cannot write to read-only directory"
        );
        assert!(
            fs::read(&file).unwrap() == bytes,
            "write failure leaves legacy bytes unchanged"
        );
    }
    let secrets = Secrets::fixed(random_master());
    ProviderConfigStore::load_from(&mut settings, secrets.clone(), true)
        .expect("migration retry succeeds");
    let encrypted = fs::read(&file).unwrap();
    no_secret(&encrypted, &secret);
    let mut corrupt: Value = serde_json::from_slice(&encrypted).unwrap();
    corrupt["formatVersion"] = json!(99);
    fs::write(&file, serde_json::to_vec(&corrupt).unwrap()).unwrap();
    let before = fs::read(&file).unwrap();
    assert!(
        ProviderConfigStore::load_from(&mut settings, secrets, true).is_err(),
        "unknown encrypted format never treated as legacy"
    );
    assert!(fs::read(file).unwrap() == before);
}

#[tokio::test]
async fn es_qa_09_keep_injected_keys_does_not_persist_them_or_require_master() {
    let app = common::TestApp::new("encrypted-secrets-qa-keep").await;
    let mut settings = app.state().settings().clone();
    let tripo = canary();
    let manual = canary();
    settings.providers.tripo.api_key = Some(tripo.clone());
    settings.providers.manual_ai.api_key = Some(manual.clone());
    let source = CountingSource::new(true);
    let mut store = ProviderConfigStore::deployment(&settings)
        .with_secrets(Secrets::with_source(source.clone()));
    let body = json!({"revision":"deployment", "tripo":{"action":"update","baseUrl":"https://qa.example.invalid/v3","model":"qa-tripo","keyAction":"keep"}, "manualAi":{"action":"update","baseUrl":"https://qa.example.invalid/v1","model":"qa-manual","keyAction":"keep"}});
    let write: everything_manual::http::dto::ProviderSettingsWrite =
        serde_json::from_value(body).unwrap();
    store
        .save(write, app.state().database().pool())
        .await
        .expect("keep does not need encryption of injected key");
    let bytes = fs::read(app.dir().join(FILE_NAME)).unwrap();
    no_secret(&bytes, &tripo);
    no_secret(&bytes, &manual);
    assert_eq!(source.loads.load(Ordering::SeqCst), 0);
    assert_eq!(source.creates.load(Ordering::SeqCst), 0);
    let loaded =
        ProviderConfigStore::load_from(&mut settings, Secrets::unavailable(), false).unwrap();
    assert!(!loaded.pending());
    assert!(settings.providers.tripo.api_key.as_ref() == Some(&tripo));
    assert!(settings.providers.manual_ai.api_key.as_ref() == Some(&manual));
}

#[cfg(target_os = "macos")]
fn native_child(
    mode: &str,
    service: &str,
    account: &str,
    file: &Path,
    secret: &SecretString,
) -> Output {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "es_qa_native_keychain_child", "--ignored"])
        .env_clear()
        .env("ES_QA_NATIVE_MODE", mode)
        .env("ES_QA_NATIVE_SERVICE", service)
        .env("ES_QA_NATIVE_ACCOUNT", account)
        .env("ES_QA_NATIVE_FILE", file)
        .env("ES_QA_NATIVE_CANARY", secret.expose())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn isolated native child")
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "explicit AC003 native Keychain integration; never ordinary unit CI"]
fn es_qa_03_native_keychain_cross_process() {
    use everything_manual::config::encrypted_secrets::NativeKeychain;
    struct Cleanup(NativeKeychain);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.0.delete();
        }
    }
    let dir = TestDir::new("encrypted-secrets-qa-native");
    let service = format!(
        "org.everything-manual.qa.encrypted-secrets.{}",
        manual_core::ids::new_id()
    );
    let account = format!("qa-{}", manual_core::ids::new_id());
    let cleanup = Cleanup(NativeKeychain::new(&service, &account));
    assert!(
        cleanup
            .0
            .load()
            .expect("native preflight read permission")
            .is_none(),
        "new unique test entry must be absent"
    );
    let file = dir.join("ciphertext.json");
    let secret = canary();
    let first = native_child("create", &service, &account, &file, &secret);
    no_secret(&first.stdout, &secret);
    no_secret(&first.stderr, &secret);
    assert!(
        first.status.success(),
        "native first process success boolean; no private output attached"
    );
    let second = native_child("read", &service, &account, &file, &secret);
    no_secret(&second.stdout, &secret);
    no_secret(&second.stderr, &secret);
    assert!(
        second.status.success(),
        "native second process success boolean; no private output attached"
    );
    let bytes = fs::read(&file).unwrap();
    no_secret(&bytes, &secret);
    let master = cleanup
        .0
        .load()
        .expect("read only this test master")
        .expect("master exists after first process");
    assert!(
        !bytes.windows(32).any(|window| window == master.as_ref()),
        "ciphertext does not include raw master"
    );
    cleanup.0.delete().expect("cleanup exact test credential");
    assert!(
        cleanup.0.load().unwrap().is_none(),
        "test credential removed"
    );
    println!(
        "native_keychain_cross_process=true; plaintext_leak=false; raw_master_leak=false; cleanup_complete=true"
    );
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "child-only helper for explicit AC003"]
fn es_qa_native_keychain_child() {
    let mode = std::env::var("ES_QA_NATIVE_MODE").expect("parent must supply child mode");
    let service = std::env::var("ES_QA_NATIVE_SERVICE").unwrap();
    let account = std::env::var("ES_QA_NATIVE_ACCOUNT").unwrap();
    assert!(service.starts_with("org.everything-manual.qa.encrypted-secrets."));
    assert!(account.starts_with("qa-"));
    let file = std::path::PathBuf::from(std::env::var_os("ES_QA_NATIVE_FILE").unwrap());
    let secret = SecretString::new(std::env::var("ES_QA_NATIVE_CANARY").unwrap());
    let secrets = Secrets::native(&service, &account);
    if mode == "create" {
        let envelope = secrets
            .encrypt(&secret, "overlay:tripo")
            .expect("native create");
        private_write(&file, &serde_json::to_vec(&envelope).unwrap());
    } else {
        assert_eq!(mode, "read");
        let envelope: Envelope = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert!(
            secrets.decrypt(&envelope, "overlay:tripo").unwrap() == secret,
            "second-process key matches only in memory"
        );
    }
}
