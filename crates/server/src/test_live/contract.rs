use super::{LiveError, Result};
use manual_core::timestamps::Timestamp;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Case {
    pub schema_version: u32,
    pub case_id: String,
    pub mode: Mode,
    pub instance: Instance,
    pub material: Material,
    pub generation: Generation,
    pub output_directory: PathBuf,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    LoopbackFixture,
    Real,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Instance {
    pub instance_id: String,
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Material {
    pub item_id: String,
    pub item_model: String,
    pub document_id: String,
    pub source_sha256: String,
    pub preparation_id: String,
    pub photos: Vec<Photo>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Photo {
    pub id: String,
    pub sha256: String,
    pub view: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Generation {
    pub model_preset: String,
    pub tripo: Provider,
    pub manual_ai: Provider,
    pub price_version: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Provider {
    pub identity: String,
    pub model: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Limits {
    pub credit_minor: i64,
    pub usd_micros: i64,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Budget {
    pub schema_version: u32,
    pub authorization_id: String,
    pub case_id: String,
    pub case_hash: String,
    pub plan_hash: String,
    pub allowed: bool,
    pub expires_at: Timestamp,
    pub limits: Limits,
    pub max_initial_generations: u32,
    pub retry_scopes: Vec<RetryScope>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryScope {
    pub stage_kind: String,
    pub batch_index: i64,
    pub stage_input_hash: String,
    pub operation: RetryOperation,
    pub max_additional_attempts: u32,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RetryOperation {
    SafeRetry,
}

pub fn slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        && !crate::config::model_guard::suspected_credential(value)
}
pub fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}
fn safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && !value.chars().any(char::is_control)
        && !value.contains("://")
        && !crate::config::model_guard::suspected_credential(value)
}
impl Case {
    pub fn read(path: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(path).map_err(|_| LiveError::new("caseFile"))?;
        if !metadata.is_file() || metadata.len() > 65536 {
            return Err(LiveError::new("caseFile"));
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(path).map_err(|_| LiveError::new("caseFile"))?;
        if !file
            .metadata()
            .map_err(|_| LiveError::new("caseFile"))?
            .is_file()
        {
            return Err(LiveError::new("caseFile"));
        }
        let mut bytes = Vec::new();
        file.take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| LiveError::new("caseFile"))?;
        if bytes.len() > 65536 {
            return Err(LiveError::new("caseFile"));
        }
        let case: Self = serde_json::from_slice(&bytes).map_err(|_| LiveError::new("caseJson"))?;
        case.validate()?;
        Ok(case)
    }
    pub fn validate(&self) -> Result<()> {
        let m = &self.material;
        let g = &self.generation;
        if self.schema_version != 1
            || !slug(&self.case_id)
            || !slug(&self.instance.instance_id)
            || !id(&m.item_id)
            || !id(&m.document_id)
            || !id(&m.preparation_id)
            || !hash(&m.source_sha256)
            || !safe_label(&m.item_model)
            || !safe_label(&g.model_preset)
            || !safe_label(&g.price_version)
            || !safe_label(&g.tripo.model)
            || !safe_label(&g.manual_ai.model)
            || g.tripo.identity != "tripo"
            || g.manual_ai.identity != "manual_ai"
            || !(2..=4).contains(&m.photos.len())
            || !self.instance.data_dir.is_absolute()
            || !self.instance.config_file.is_absolute()
            || !self.output_directory.is_absolute()
        {
            return Err(LiveError::new("caseFields"));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut views = std::collections::BTreeSet::new();
        for photo in &m.photos {
            if !id(&photo.id)
                || !hash(&photo.sha256)
                || !matches!(photo.view.as_str(), "front" | "left" | "back" | "right")
                || !ids.insert(&photo.id)
                || !views.insert(&photo.view)
            {
                return Err(LiveError::new("caseFields"));
            }
        }
        if !views.contains(&"front".to_owned()) {
            return Err(LiveError::new("caseFields"));
        }
        Ok(())
    }
}
impl Budget {
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = crate::config::encrypted_secrets::read_private_file(path)
            .map_err(|_| LiveError::new("budgetFilePermissions"))?;
        let budget: Self =
            serde_json::from_slice(&bytes).map_err(|_| LiveError::new("budgetJson"))?;
        budget.validate()?;
        Ok(budget)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || !slug(&self.authorization_id)
            || !slug(&self.case_id)
            || !hash(&self.case_hash)
            || !hash(&self.plan_hash)
            || self.max_initial_generations != 1
            || self.limits.credit_minor < 0
            || self.limits.usd_micros < 0
            || self.retry_scopes.len() > 21
        {
            return Err(LiveError::new("budgetFields"));
        }
        if !self.allowed {
            return Err(LiveError::new("notAuthorized"));
        }
        if self.expires_at <= Timestamp::now() {
            return Err(LiveError::new("authorizationExpired"));
        }
        let mut unique = std::collections::BTreeSet::new();
        for retry in &self.retry_scopes {
            if !matches!(retry.stage_kind.as_str(), "tripo_submit" | "manual_extract")
                || retry.batch_index < 0
                || !hash(&retry.stage_input_hash)
                || !(1..=5).contains(&retry.max_additional_attempts)
                || !unique.insert((&retry.stage_kind, retry.batch_index))
            {
                return Err(LiveError::new("retryScope"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn budget() -> Value {
        json!({"schemaVersion":1,"authorizationId":"fixture-only","caseId":"fixture-case","caseHash":"a".repeat(64),"planHash":"b".repeat(64),"allowed":true,"expiresAt":Timestamp::now().checked_add_millis(60000).unwrap(),"limits":{"creditMinor":3000,"usdMicros":100000},"maxInitialGenerations":1,"retryScopes":[]})
    }
    #[test]
    fn budget_rejects_unknown_fractional_negative_string_and_overflow() {
        for invalid in [
            json!(-1),
            json!(1.5),
            json!("3000"),
            serde_json::from_str::<Value>("9223372036854775808").unwrap(),
            Value::Null,
        ] {
            for field in ["creditMinor", "usdMicros"] {
                let mut value = budget();
                value["limits"][field] = invalid.clone();
                assert!(
                    serde_json::from_value::<Budget>(value).map_or(true, |b| b.validate().is_err())
                );
            }
        }
        for path in ["top", "nested"] {
            let mut value = budget();
            if path == "top" {
                value["apiKey"] = json!("fake-canary");
            } else {
                value["limits"]["extra"] = json!(0);
            }
            assert!(serde_json::from_value::<Budget>(value).is_err());
        }
        let mut value = budget();
        value["limits"]["creditMinor"] = json!(i64::MAX);
        assert!(
            serde_json::from_value::<Budget>(value)
                .unwrap()
                .validate()
                .is_ok()
        );
    }
    #[test]
    fn authorization_rejects_false_expired_extra_initial_and_unscoped_retries() {
        for (field, value) in [
            ("allowed", json!(false)),
            ("expiresAt", json!("2001-01-01T00:00:00Z")),
            ("maxInitialGenerations", json!(2)),
            ("authorizationId", json!("sk-proj-qa_fake_0123456789")),
        ] {
            let mut b = budget();
            b[field] = value;
            assert!(
                serde_json::from_value::<Budget>(b)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        let mut b = budget();
        b["retryScopes"] = json!([{"stageKind":"all","batchIndex":0,"stageInputHash":"a".repeat(64),"operation":"safeRetry","maxAdditionalAttempts":1}]);
        assert!(
            serde_json::from_value::<Budget>(b)
                .unwrap()
                .validate()
                .is_err()
        );
        assert!(!slug("sk-proj-qa_fake_0123456789"));
        assert!(!slug("../escape"));
        assert!(slug("fixture-only-001"));
    }
}
