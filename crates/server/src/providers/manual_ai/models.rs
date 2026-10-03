//! Explicit model discovery from the active provider only; no generation or configuration writes.
use std::time::Duration;

use axum::http::StatusCode;
use manual_core::ApiErrorCode;
use serde::Deserialize;
use zeroize::{Zeroize, Zeroizing};

use crate::config::{ProviderSettings, model_guard, provider_overrides, secret};
use crate::http::error::ApiError;

const MAX_BYTES: usize = 256 * 1024;
const MAX_MODELS: usize = 1_000;

#[derive(Deserialize)]
struct ModelPage {
    data: Vec<ModelId>,
}
#[derive(Deserialize)]
struct ModelId {
    id: String,
}
impl Drop for ModelId {
    fn drop(&mut self) {
        self.id.zeroize();
    }
}

fn upstream_error(message: &'static str) -> ApiError {
    // Never retain/format an upstream body, URL, reqwest error, or credential.
    ApiError::new(StatusCode::BAD_GATEWAY, ApiErrorCode::Internal, message)
}

pub(crate) async fn read_models(provider: &ProviderSettings) -> Result<Vec<String>, ApiError> {
    let key = provider.api_key.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            ApiErrorCode::ProviderNotConfigured,
            "当前生效的说明书 AI 配置没有密钥；请保存配置并重启服务后读取模型",
        )
    })?;
    let base = provider_overrides::normalize_url(&provider.base_url).map_err(|_| {
        ApiError::validation_failed("当前生效的说明书 AI 地址无效；请修正配置并重启服务")
    })?;
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));
    if reqwest::Url::parse(&base)
        .ok()
        .is_some_and(|url| matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|_| upstream_error("无法建立模型读取连接"))?;
    let mut response = client
        .get(format!("{base}/models"))
        .bearer_auth(key.expose())
        .send()
        .await
        .map_err(|_| {
            upstream_error("无法读取模型：连接失败或超时，请检查当前生效的地址与服务状态")
        })?;
    if !response.status().is_success() {
        return Err(upstream_error(match response.status().as_u16() {
            401 | 403 => "模型服务拒绝认证，请检查当前生效的说明书 AI 密钥与权限",
            404 | 405 => "当前说明书 AI 服务不支持读取模型，请手动填写服务支持的模型名称",
            300..=399 => "模型服务返回重定向，已停止读取；请检查当前生效的基础地址",
            429 => "模型服务暂时限流，请稍后手动重试",
            _ => "模型服务未成功返回列表，请稍后手动重试",
        }));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BYTES as u64)
    {
        return Err(upstream_error("模型列表超过读取上限"));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| upstream_error("模型列表读取中断或超时，请稍后手动重试"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
            return Err(upstream_error("模型列表超过读取上限"));
        }
        bytes.extend_from_slice(&chunk);
    }
    if secret::response_requires_discard(&bytes, key) {
        return Err(upstream_error(
            "模型列表无法安全读取，请手动填写服务支持的模型名称",
        ));
    }
    let page: ModelPage = serde_json::from_slice(&bytes)
        .map_err(|_| upstream_error("模型服务返回了无效列表，请手动填写服务支持的模型名称"))?;
    if page.data.len() > MAX_MODELS
        || page.data.iter().any(|model| {
            model.id.is_empty()
                || model.id.chars().count() > 128
                || model
                    .id
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || c == '\u{feff}')
                || model_guard::suspected_credential(&model.id)
        })
    {
        return Err(upstream_error(
            "模型列表包含无效或疑似凭据的名称，已停止读取",
        ));
    }
    let mut models: Vec<String> = page.data.iter().map(|model| model.id.clone()).collect();
    models.sort();
    models.dedup();
    Ok(models)
}
