//! Finite high-confidence model-field mistake detection, not a general secret scanner.
use serde_json::Value;

pub const MODEL_FIELD_MESSAGE: &str = "这里需要模型名称；API 密钥请在密钥操作中选择替换后输入";
pub const MODEL_CONFIG_MESSAGE: &str =
    "模型配置需要修正；请前往设置修正并重启服务，再重新获取报价。";
pub const FROZEN_MODEL_MESSAGE: &str =
    "此报价的模型信息不可用，请重新获取报价；如配置仍需修正，请前往设置。";
pub const MODEL_ISSUE_REASON: &str = "providerModelInvalid";
pub const FROZEN_MODEL_REASON: &str = "quoteModelInvalid";

pub fn submission_stage(kind: manual_core::domain::StageKind) -> bool {
    use manual_core::domain::StageKind;
    matches!(
        kind,
        StageKind::ManualExtract
            | StageKind::ManualMerge
            | StageKind::TripoUpload
            | StageKind::TripoSubmit
            | StageKind::TripoSegment
    )
}

pub fn suspected_credential(value: &str) -> bool {
    // Unicode White_Space plus BOM: identical to the frontend's explicit set.
    let mut value = value.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if value
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Bearer"))
        && value.as_bytes().get(6) == Some(&b' ')
    {
        value = value[6..].trim_start_matches(' ');
    }
    value.strip_prefix("sk-").is_some_and(|suffix| {
        suffix.len() >= 16
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    })
}

pub fn provider_config_has_issue(value: &Value) -> bool {
    [
        "/tripo/model",
        "/manualAi/model",
        "/tripoSegmentation/model",
    ]
    .iter()
    .any(|path| {
        value
            .pointer(path)
            .and_then(Value::as_str)
            .is_some_and(suspected_credential)
    })
}

/// Only documented provider model positions, never document/item contents.
pub fn quote_json_has_issue(value: &Value) -> bool {
    [
        "/providerConfig/tripo/model",
        "/providerConfig/manualAi/model",
        "/providerConfig/tripoSegmentation/model",
        "/sendScope/tripo/model",
        "/sendScope/tripo/parameters/model",
        "/sendScope/manualAi/model",
        "/sendScope/tripo/segmentation/model",
    ]
    .iter()
    .any(|path| {
        value
            .pointer(path)
            .and_then(Value::as_str)
            .is_some_and(suspected_credential)
    })
}

/// Frozen job facts are checked independently of how the worker/control API is wired.
pub async fn ensure_job_models(
    conn: &mut sqlx::SqliteConnection,
    job_id: &str,
) -> Result<(), crate::http::error::ApiError> {
    use crate::http::error::ApiError;
    let row: Option<(String, String)> = sqlx::query_as("SELECT s.provider_config, s.budgets FROM generation_snapshots s JOIN jobs j ON j.snapshot_id = s.id WHERE j.id = ?")
        .bind(job_id).fetch_optional(&mut *conn).await.map_err(|_| ApiError::internal("无法读取任务模型配置"))?;
    if let Some((config, budgets)) = row {
        let config: Value =
            serde_json::from_str(&config).map_err(|_| ApiError::internal("任务配置无法解析"))?;
        let budgets: Value =
            serde_json::from_str(&budgets).map_err(|_| ApiError::internal("任务预算无法解析"))?;
        let mut issue = provider_config_has_issue(&config);
        if let Some(quote_id) = budgets.get("quoteId").and_then(Value::as_str)
            && let Some(quote) = crate::storage::repo::quotes::get(conn, quote_id)
                .await
                .map_err(ApiError::from_storage)?
        {
            issue |=
                crate::generation::estimate::quote_model_issue(&quote).map_err(ApiError::from)?;
        }
        if issue {
            return Err(ApiError::unprocessable_reason(
                FROZEN_MODEL_REASON,
                FROZEN_MODEL_MESSAGE,
                serde_json::json!({}),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_rule_and_legitimate_custom_models() {
        for candidate in [
            "sk-0123456789abcdef",
            " sk-0123456789abcdef ",
            "Bearer  sk-0123456789abcdef",
            "bEaReR sk-proj-fake_canary_12345",
            "\u{feff}sk-0123456789abcdef\u{feff}",
            "\u{85}sk-0123456789abcdef\u{85}",
        ] {
            assert!(suspected_credential(candidate));
        }
        for candidate in [
            "sk-0123456789abcde",
            "sk-local",
            "org/custom-model",
            "local:model-v2",
            "SK-0123456789abcdef",
            "Bearer\tsk-0123456789abcdef",
            "text sk-0123456789abcdef",
            "sk-0123456789abcdef.",
            "sk-0123456789abcde中文",
        ] {
            assert!(!suspected_credential(candidate));
        }
    }
}
