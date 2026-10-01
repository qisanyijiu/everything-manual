//! 网页供应商覆盖：秘密仅在受限文件；运行配置固定，代次与准入共用进程锁。
//! 文件不存在时保持部署行为；空覆盖文件仍保留修订，旧报价不能因恢复而复活。
use std::fs;
use std::path::PathBuf;

use axum::http::StatusCode;
use manual_core::ApiErrorCode;
use manual_core::validation::FieldIssue;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::encrypted_secrets::{
    Envelope, SecretError, Secrets, read_private_file, write_private_file,
};
use super::{ProviderSettings, Providers, SecretString, Settings};
use crate::http::dto::{
    ConfigSource, KeyEditAction, ProviderEdit, ProviderEditAction, ProviderSettingsData,
    ProviderSettingsWrite, ProviderView, ProviderViews,
};
use crate::http::error::ApiError;

pub const FILE_NAME: &str = "provider-overrides.json";
pub const LEGACY_REVISION: &str = "deployment";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
enum KeyOverride {
    Inherit,
    Clear,
    Replace {
        #[serde(with = "private_secret")]
        value: SecretString,
    },
}
mod private_secret {
    use super::*;
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<SecretString, D::Error> {
        String::deserialize(deserializer).map(SecretString::new)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderOverride {
    base_url: String,
    model: Option<String>,
    key: KeyOverride,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Overlay {
    revision: String,
    tripo: Option<ProviderOverride>,
    manual_ai: Option<ProviderOverride>,
}
impl Default for Overlay {
    fn default() -> Self {
        Self {
            revision: LEGACY_REVISION.into(),
            tripo: None,
            manual_ai: None,
        }
    }
}

/// AppState 的 RwLock 保护此状态。读锁必须覆盖会接受新执行动作的整个事务。
pub struct ProviderConfigStore {
    path: PathBuf,
    secrets: Secrets,
    deployment: Providers,
    active: Providers,
    active_overlay: Overlay,
    saved: Overlay,
}
impl ProviderConfigStore {
    /// 直接装配的测试与嵌入者保持无隐式 IO；生产必须使用 load。
    pub fn deployment(settings: &Settings) -> Self {
        Self {
            path: settings.data_dir.join(FILE_NAME),
            secrets: Secrets::unavailable(),
            deployment: settings.providers.clone(),
            active: settings.providers.clone(),
            active_overlay: Overlay::default(),
            saved: Overlay::default(),
        }
    }
    pub fn with_secrets(mut self, secrets: Secrets) -> Self {
        self.secrets = secrets;
        self
    }
    /// Read-only; check must neither initialize the Keychain nor migrate plaintext.
    pub fn load(settings: &mut Settings) -> Result<Self, super::CliError> {
        Self::load_from(
            settings,
            Secrets::from_environment().map_err(|e| super::CliError::config(e.to_string()))?,
            false,
        )
    }
    /// migrate_legacy=true only after acquiring the serve data-dir exclusive lock.
    pub fn load_from(
        settings: &mut Settings,
        secrets: Secrets,
        migrate_legacy: bool,
    ) -> Result<Self, super::CliError> {
        let mut store = Self::deployment(settings).with_secrets(secrets);
        let bytes = match fs::symlink_metadata(&store.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(store),
            Err(_) => return Err(super::CliError::config("无法读取网页 API 配置文件")),
            Ok(_) => read_private_file(&store.path)
                .map_err(|e| super::CliError::config(e.to_string()))?,
        };
        let saved = match serde_json::from_slice::<DiskOverlay>(&bytes) {
            Ok(disk) => disk
                .open(&store.secrets)
                .map_err(|e| super::CliError::config(e.to_string()))?,
            Err(_) => {
                let legacy: Overlay = serde_json::from_slice(&bytes)
                    .map_err(|_| super::CliError::config("网页 API 配置损坏；未回退到部署配置"))?;
                validate_overlay(&legacy)
                    .map_err(|_| super::CliError::config("旧网页 API 配置内容无效；未迁移"))?;
                if !migrate_legacy {
                    return Err(super::CliError::config(
                        "检测到旧网页 API 配置；check 为只读，请先停服并使用 serve 在排他锁下迁移",
                    ));
                }
                write_overlay(&store.path, &legacy, &store.secrets)
                    .map_err(|e| super::CliError::config(e.to_string()))?;
                legacy
            }
        };
        validate_overlay(&saved)
            .map_err(|_| super::CliError::config("网页 API 配置文件内容无效；未回退到部署配置"))?;
        store.saved = saved.clone();
        store.active_overlay = saved;
        store.active = store.effective(&store.saved);
        settings.providers = store.active.clone();
        Ok(store)
    }
    fn effective(&self, overlay: &Overlay) -> Providers {
        Providers {
            tripo: apply(&self.deployment.tripo, overlay.tripo.as_ref()),
            manual_ai: apply(&self.deployment.manual_ai, overlay.manual_ai.as_ref()),
        }
    }
    pub fn pending(&self) -> bool {
        !providers_equal(&self.active, &self.effective(&self.saved))
    }
    pub fn revision(&self) -> &str {
        &self.saved.revision
    }
    pub fn view(&self) -> ProviderSettingsData {
        let saved = self.effective(&self.saved);
        ProviderSettingsData {
            revision: self.saved.revision.clone(),
            pending: self.pending(),
            active: views(&self.active, &self.active_overlay),
            saved: views(&saved, &self.saved),
        }
    }
    pub fn ensure_available(&self) -> Result<(), ApiError> {
        if self.pending() {
            Err(gate_error(
                "providerConfigPending",
                "API 配置待重启生效，请前往设置；新报价、确认与生成操作暂不可用",
            ))
        } else {
            Ok(())
        }
    }
    pub fn ensure_revision(&self, config: &Value) -> Result<(), ApiError> {
        self.ensure_available()?;
        let revision = config
            .get("configRevision")
            .and_then(Value::as_str)
            .unwrap_or(LEGACY_REVISION);
        if revision != self.revision() {
            Err(gate_error(
                "providerConfigChanged",
                "API 配置已改变，请重新报价并确认；旧任务不能跨配置重试",
            ))
        } else {
            Ok(())
        }
    }
    pub async fn ensure_job(
        &self,
        conn: &mut sqlx::SqliteConnection,
        job_id: &str,
    ) -> Result<(), ApiError> {
        self.ensure_available()?;
        let config: Option<String> = sqlx::query_scalar("SELECT s.provider_config FROM generation_snapshots s JOIN jobs j ON j.snapshot_id = s.id WHERE j.id = ?")
            .bind(job_id).fetch_optional(conn).await.map_err(|_| ApiError::internal("无法读取任务配置"))?;
        if let Some(config) = config {
            self.ensure_revision(
                &serde_json::from_str::<Value>(&config)
                    .map_err(|_| ApiError::internal("任务配置无法解析"))?,
            )?;
        }
        Ok(())
    }
    /// 调用者持写锁：CAS、阻断检查、原子持久化、发布新代次为同一序列化边界。
    pub async fn save(
        &mut self,
        request: ProviderSettingsWrite,
        pool: &sqlx::SqlitePool,
    ) -> Result<(), ApiError> {
        if request.revision != self.saved.revision {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                ApiErrorCode::RevisionConflict,
                "配置已在其他页面更新，你的编辑尚未保存",
            )
            .with_details(
                json!({"reason":"providerConfigConflict", "currentRevision":self.saved.revision}),
            ));
        }
        let mut issues = Vec::new();
        let mut candidate = self.saved.clone();
        candidate.tripo = edit(
            "tripo",
            request.tripo,
            &self.deployment.tripo,
            self.saved.tripo.as_ref(),
            &mut issues,
        );
        candidate.manual_ai = edit(
            "manualAi",
            request.manual_ai,
            &self.deployment.manual_ai,
            self.saved.manual_ai.as_ref(),
            &mut issues,
        );
        if !issues.is_empty() {
            return Err(ApiError::field_validation(issues));
        }
        if candidate == self.saved {
            return Ok(());
        }
        // 一个终态 job 仍可能留下远端等待或未知提交；账务 unknown 同样不可越过。
        let blocked: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE status NOT IN ('succeeded','failed','cancelled')) OR EXISTS(SELECT 1 FROM provider_attempts WHERE submit_state IN ('intent','submitting','unknown')) OR EXISTS(SELECT 1 FROM job_stages WHERE status IN ('queued','running','waiting_provider','retry_wait','submission_unknown') AND stage_kind IN ('manual_extract','manual_merge','tripo_upload','tripo_submit','tripo_poll','model_download')) OR EXISTS(SELECT 1 FROM cost_ledger WHERE state = 'unknown')")
            .fetch_one(pool).await.map_err(|_| ApiError::internal("无法检查任务状态；配置未保存"))?;
        if blocked != 0 {
            return Err(gate_error(
                "providerConfigBusy",
                "尚有任务或供应商操作未处理，暂不能更改 API 配置；请前往任务中心处理",
            ));
        }
        candidate.revision = manual_core::ids::new_id();
        // 验证客户端可构造，不发送 HTTP。未配置不构造，沿用现有启动语义。
        let effective = self.effective(&candidate);
        for provider in [&effective.tripo, &effective.manual_ai] {
            if let Some(key) = &provider.api_key {
                axum::http::HeaderValue::from_str(&format!("Bearer {}", key.expose())).map_err(
                    |_| {
                        ApiError::field_validation(vec![FieldIssue::new(
                            &format!(
                                "{}.apiKey",
                                if provider.name == "tripo" {
                                    "tripo"
                                } else {
                                    "manualAi"
                                }
                            ),
                            "密钥不能作为有效的 Authorization 值",
                        )])
                    },
                )?;
            }
        }
        // Clearing/restoring also overwrites existing ciphertext; a lost/replaced master is not a bypass.
        if [&self.saved.tripo, &self.saved.manual_ai]
            .into_iter()
            .flatten()
            .any(|provider| matches!(provider.key, KeyOverride::Replace { .. }))
        {
            self.secrets
                .ensure_existing_key()
                .map_err(|e| ApiError::internal(e.to_string()))?;
        }
        write_overlay(&self.path, &candidate, &self.secrets)
            .map_err(|e| ApiError::internal(e.to_string()))?;
        self.saved = candidate;
        Ok(())
    }
}
fn gate_error(reason: &'static str, message: &'static str) -> ApiError {
    ApiError::unprocessable_reason(reason, message, json!({}))
}
fn providers_equal(a: &Providers, b: &Providers) -> bool {
    fn equal(a: &ProviderSettings, b: &ProviderSettings) -> bool {
        a.base_url == b.base_url && a.model == b.model && a.api_key == b.api_key
    }
    equal(&a.tripo, &b.tripo) && equal(&a.manual_ai, &b.manual_ai)
}
fn apply(base: &ProviderSettings, overlay: Option<&ProviderOverride>) -> ProviderSettings {
    let mut result = base.clone();
    if let Some(overlay) = overlay {
        result.base_url = overlay.base_url.clone();
        result.model = overlay.model.clone();
        match &overlay.key {
            KeyOverride::Inherit => {}
            KeyOverride::Clear => {
                result.api_key = None;
                result.key_source = None;
            }
            KeyOverride::Replace { value } => {
                result.api_key = Some(value.clone());
                result.key_source = Some("网页配置".into());
            }
        }
    }
    result
}
fn views(providers: &Providers, overlay: &Overlay) -> ProviderViews {
    ProviderViews {
        tripo: view(&providers.tripo, overlay.tripo.as_ref()),
        manual_ai: view(&providers.manual_ai, overlay.manual_ai.as_ref()),
    }
}
fn view(provider: &ProviderSettings, overlay: Option<&ProviderOverride>) -> ProviderView {
    let web = overlay.is_some();
    ProviderView {
        base_url: display_url(&provider.base_url),
        model: provider.model.clone(),
        key_configured: provider.api_key.is_some(),
        // Settings 不保存环境/TOML/default 的字段级 provenance，统一称部署配置，不能按值猜测。
        base_url_source: if web {
            ConfigSource::Web
        } else {
            ConfigSource::Deployment
        },
        model_source: if web {
            ConfigSource::Web
        } else if provider.model.is_none() {
            ConfigSource::Unconfigured
        } else {
            ConfigSource::Deployment
        },
        key_source: match overlay.map(|o| &o.key) {
            Some(KeyOverride::Replace { .. } | KeyOverride::Clear) => ConfigSource::Web,
            _ if provider.api_key.is_some() => ConfigSource::Deployment,
            _ => ConfigSource::Unconfigured,
        },
    }
}
fn display_url(value: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(value) else {
        return "（部署地址无效）".into();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.as_str().trim_end_matches('/').to_owned()
}
fn edit(
    prefix: &str,
    mut request: ProviderEdit,
    deployment: &ProviderSettings,
    old: Option<&ProviderOverride>,
    issues: &mut Vec<FieldIssue>,
) -> Option<ProviderOverride> {
    if request.action == ProviderEditAction::Restore {
        return None;
    }
    let base_url = request.base_url.take().unwrap_or_default();
    let base_url = match normalize_url(&base_url) {
        Ok(value) => value,
        Err(message) => {
            issues.push(FieldIssue::new(&format!("{prefix}.baseUrl"), message));
            base_url
        }
    };
    let model = request.model.take().unwrap_or_default().trim().to_owned();
    if model.chars().count() > 128 || model.chars().any(char::is_control) {
        issues.push(FieldIssue::new(
            &format!("{prefix}.model"),
            "模型最多 128 字符，且不能包含控制字符",
        ));
    }
    let model = if model.is_empty() { None } else { Some(model) };
    let key = match request.key_action {
        Some(KeyEditAction::Keep) => old.map(|o| o.key.clone()).unwrap_or(KeyOverride::Inherit),
        Some(KeyEditAction::Clear) => KeyOverride::Clear,
        Some(KeyEditAction::Replace) => {
            let raw = zeroize::Zeroizing::new(request.api_key.take().unwrap_or_default());
            let value = SecretString::new(raw.trim());
            if !valid_key(value.expose()) {
                issues.push(FieldIssue::new(
                    &format!("{prefix}.apiKey"),
                    "新密钥不能为空、超过 4096 字符或包含空白、控制字符",
                ));
            }
            KeyOverride::Replace { value }
        }
        None => {
            issues.push(FieldIssue::new(
                &format!("{prefix}.keyAction"),
                "请选择保留、替换或清除密钥",
            ));
            KeyOverride::Inherit
        }
    };
    if old.is_none()
        && base_url == deployment.base_url
        && model == deployment.model
        && key == KeyOverride::Inherit
    {
        return None;
    }
    Some(ProviderOverride {
        base_url,
        model,
        key,
    })
}
fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 4096
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
        && axum::http::HeaderValue::from_str(&format!("Bearer {value}")).is_ok()
}
pub fn normalize_url(raw: &str) -> Result<String, &'static str> {
    let value = raw.trim();
    let error = "地址须为 HTTPS（仅 localhost、127.0.0.1、[::1] 可使用 HTTP），不能含账号密码、查询参数、片段、空白或控制字符；最多 2048 字符";
    if value.chars().count() > 2048
        || raw.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
    {
        return Err(error);
    }
    let url = reqwest::Url::parse(value).map_err(|_| error)?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
    {
        return Err(error);
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
fn validate_overlay(overlay: &Overlay) -> Result<(), ()> {
    if uuid::Uuid::parse_str(&overlay.revision).is_err() {
        return Err(());
    }
    for provider in [&overlay.tripo, &overlay.manual_ai].into_iter().flatten() {
        normalize_url(&provider.base_url).map_err(|_| ())?;
        if provider
            .model
            .as_ref()
            .is_some_and(|m| m.chars().count() > 128 || m.chars().any(char::is_control))
        {
            return Err(());
        }
        if let KeyOverride::Replace { value } = &provider.key
            && !valid_key(value.expose())
        {
            return Err(());
        }
    }
    Ok(())
}
// Disk DTOs never serialize the in-memory plaintext variant.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiskOverlay {
    format_version: u32,
    revision: String,
    tripo: Option<DiskProvider>,
    manual_ai: Option<DiskProvider>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiskProvider {
    base_url: String,
    model: Option<String>,
    key: DiskKey,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
enum DiskKey {
    Inherit,
    Clear,
    Replace { encrypted: Envelope },
}
impl DiskOverlay {
    fn seal(overlay: &Overlay, secrets: &Secrets) -> Result<Self, SecretError> {
        fn provider(
            value: &Option<ProviderOverride>,
            secrets: &Secrets,
            purpose: &str,
        ) -> Result<Option<DiskProvider>, SecretError> {
            value
                .as_ref()
                .map(|value| {
                    Ok(DiskProvider {
                        base_url: value.base_url.clone(),
                        model: value.model.clone(),
                        key: match &value.key {
                            KeyOverride::Inherit => DiskKey::Inherit,
                            KeyOverride::Clear => DiskKey::Clear,
                            KeyOverride::Replace { value } => DiskKey::Replace {
                                encrypted: secrets.encrypt(value, purpose)?,
                            },
                        },
                    })
                })
                .transpose()
        }
        Ok(Self {
            format_version: 1,
            revision: overlay.revision.clone(),
            tripo: provider(&overlay.tripo, secrets, "overlay:tripo")?,
            manual_ai: provider(&overlay.manual_ai, secrets, "overlay:manual-ai")?,
        })
    }
    fn open(self, secrets: &Secrets) -> Result<Overlay, SecretError> {
        if self.format_version != 1 {
            return Err(SecretError::InvalidEnvelope);
        }
        fn provider(
            value: Option<DiskProvider>,
            secrets: &Secrets,
            purpose: &str,
        ) -> Result<Option<ProviderOverride>, SecretError> {
            value
                .map(|value| {
                    Ok(ProviderOverride {
                        base_url: value.base_url,
                        model: value.model,
                        key: match value.key {
                            DiskKey::Inherit => KeyOverride::Inherit,
                            DiskKey::Clear => KeyOverride::Clear,
                            DiskKey::Replace { encrypted } => KeyOverride::Replace {
                                value: secrets.decrypt(&encrypted, purpose)?,
                            },
                        },
                    })
                })
                .transpose()
        }
        Ok(Overlay {
            revision: self.revision,
            tripo: provider(self.tripo, secrets, "overlay:tripo")?,
            manual_ai: provider(self.manual_ai, secrets, "overlay:manual-ai")?,
        })
    }
}
fn write_overlay(
    path: &std::path::Path,
    overlay: &Overlay,
    secrets: &Secrets,
) -> Result<(), SecretError> {
    let disk = DiskOverlay::seal(overlay, secrets)?;
    let bytes = serde_json::to_vec(&disk).map_err(|_| SecretError::WriteFailed)?;
    // Authenticate every newly produced ciphertext before publishing, including migration.
    if disk.open(secrets)? != *overlay {
        return Err(SecretError::InvalidEnvelope);
    }
    write_private_file(path, &bytes, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls_reject_unsafe_forms_and_allow_explicit_loopback() {
        for value in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com?q=x",
            "https://example.com#x",
            "https://example.com/\npath",
            "file:///x",
        ] {
            assert!(normalize_url(value).is_err());
        }
        for value in [
            "https://example.com/v1/",
            "http://localhost:9000",
            "http://127.0.0.1:9000/v3",
            "http://[::1]:9000",
        ] {
            assert!(normalize_url(value).is_ok());
        }
    }
    #[test]
    fn secret_format_is_checked_without_echoing() {
        for value in ["", "two words", "abc\r\nx", "abc\u{007f}"] {
            assert!(!valid_key(value));
        }
        assert!(valid_key("fixture-fake-key"));
        assert_eq!(
            format!(
                "{:?}",
                KeyOverride::Replace {
                    value: SecretString::new("never-print")
                }
            ),
            "Replace { value: [redacted] }"
        );
    }
}
