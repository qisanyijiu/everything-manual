//! JSON 请求体提取器 [`JsonBody`]：把 axum 默认的拒绝转换成本项目统一错误结构。
//!
//! 为什么不用 `axum::Json<T>` 直接提取：它的拒绝响应是**纯文本/非合同结构**，
//! 会绕开 `{ error: { code, message, details, requestId } }`（contracts.md §1）。
//! 本提取器把拒绝映射为：
//! - 415 `UNSUPPORTED_MEDIA_TYPE`：`Content-Type` 不是 `application/json`；
//! - 413 `PAYLOAD_TOO_LARGE`：请求体超过配置上限（由路由上的 `DefaultBodyLimit` 生效）；
//! - 422 `VALIDATION_FAILED`：JSON 语法错误或字段不符合接口结构。
//!
//! 请求体**从不写日志**（PRD §5.7）：本模块只把字节交给 serde。

use axum::Json;
use axum::extract::FromRequest;
use axum::extract::rejection::JsonRejection;
use serde::de::DeserializeOwned;

use super::error::ApiError;

/// 合同化的 JSON 请求体。
pub struct JsonBody<T>(pub T);

impl<T, S> FromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    /// 拒绝类型直接是最终 [`axum::response::Response`]：这样错误体里的 `requestId`
    /// 能取到本次请求的 ID（与响应头、请求日志一致）。
    type Rejection = axum::response::Response;

    async fn from_request(
        request: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let request_id = request
            .extensions()
            .get::<super::error::RequestId>()
            .cloned()
            .map(|id| id.0)
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        match Json::<T>::from_request(request, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(map_rejection(rejection).render(&request_id)),
        }
    }
}

/// 双层 `Option` 反序列化（PATCH 语义的基石，见 ADR-016）。
///
/// 配合 `#[serde(default, deserialize_with = "double_option")]` 使用：
/// - 字段**缺失** → `default` 生效 → `None`（保持原值）；
/// - 字段出现且为 `null` → `Some(None)`（可选字段 = 清空；必填字段 = 字段错误）；
/// - 字段出现且有值 → `Some(Some(value))`。
///
/// 没有这一层就无法区分"没提供"与"提供了 null"，会让 `{"brand": null}` 静默保留原值
/// ——这正是 T04 QA 记录的缺陷形态。
pub fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    let inner: Option<T> = serde::Deserialize::deserialize(deserializer)?;
    Ok(Some(inner))
}

fn map_rejection(rejection: JsonRejection) -> ApiError {
    match rejection {
        JsonRejection::MissingJsonContentType(_) => ApiError::unsupported_media_type(),
        JsonRejection::JsonSyntaxError(_) => ApiError::validation_failed("请求体不是合法 JSON"),
        JsonRejection::JsonDataError(error) => {
            // body_text 形如「Failed to deserialize … missing field `password`」：
            // 只包含结构与字段名，不含用户数据以外的内容。
            ApiError::validation_failed(format!("请求体不符合接口结构：{}", error.body_text()))
        }
        other => {
            if other.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
                ApiError::payload_too_large()
            } else {
                ApiError::validation_failed("请求体无法读取")
            }
        }
    }
}
