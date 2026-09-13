//! API 错误类型与统一错误响应构造（contracts.md §1）。
//!
//! 所有非 2xx 的应用错误使用同一结构：
//! `{ "error": { code, message, details, requestId } }`，并带 `x-request-id` 头。
//!
//! 约束：
//! - `requestId` 与请求日志、响应头是**同一个值**：由最外层请求日志中间件生成并放入
//!   请求扩展（[`RequestId`]），handler 取出后经 [`ApiError::render`] 写入响应体；
//!   没有扩展时（单元测试）才现取一个新 UUID；
//! - 不向用户输出堆栈、SQL、密钥或完整供应商签名 URL：`Internal` 的 `message`
//!   固定为通用文案，真实错误只进服务端日志（调用方负责 `tracing::error!`）。

use axum::Json;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use manual_core::ApiErrorCode;

use super::dto::{ApiErrorBody, ApiErrorResponse};

/// 请求诊断 ID（UUIDv7）。由请求日志中间件插入请求扩展，handler 经提取器取得。
///
/// 实现 `FromRequestParts` 后可直接作为 handler 参数：`request_id: RequestId`。
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

impl std::ops::Deref for RequestId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<S> axum::extract::FromRequestParts<S> for RequestId
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<RequestId>()
            .cloned()
            .ok_or_else(|| ApiError::internal("请求缺少诊断 ID（服务端中间件未装配）"))
    }
}

/// 应用级错误。用 [`ApiError::render`] 转成统一 JSON 响应（推荐在 handler 中显式传入
/// 请求的 [`RequestId`]；`IntoResponse` 仅在无请求上下文时用于测试与兜底）。
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: ApiErrorCode,
    pub message: String,
    pub details: Option<serde_json::Value>,
    /// 额外响应头（如 429 的 `Retry-After`）。
    pub headers: Vec<(&'static str, String)>,
}

/// 响应扩展标记：供请求日志中间件读取统一错误码（`errorCode` 字段）。
///
/// 只挂在进程内的 [`axum::response::Response`] 扩展上，不影响响应体与机器合同。
#[derive(Debug, Clone, Copy)]
pub struct ResponseErrorCode(pub ApiErrorCode);

impl ApiError {
    pub fn new(status: StatusCode, code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: None,
            headers: Vec::new(),
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    /// 404：资源或接口不存在。
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ApiErrorCode::NotFound, message)
    }

    /// 401：未登录或会话已失效（cookie 缺失/过期/已注销）。
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ApiErrorCode::Unauthorized,
            "未登录或会话已失效：请重新登录",
        )
    }

    /// 401：登录失败（密码错误或管理员未初始化）。**不区分具体原因**（不泄露系统状态）。
    pub fn login_failed() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ApiErrorCode::Unauthorized,
            "登录失败：凭据无效",
        )
    }

    /// 403：CSRF token 缺失或不匹配。
    pub fn csrf_rejected() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ApiErrorCode::CsrfRejected,
            "缺少或无效的 CSRF token：修改请求必须携带 X-CSRF-Token（取自登录或 GET /auth/session）",
        )
    }

    /// 403：Origin 不在允许列表。
    pub fn origin_rejected(origin: &str) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ApiErrorCode::OriginRejected,
            "请求来源（Origin）不被允许：请从本站页面发起请求",
        )
        .with_details(serde_json::json!({ "origin": origin }))
    }

    /// 422：请求体未通过校验。
    pub fn validation_failed(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ApiErrorCode::ValidationFailed,
            message,
        )
    }

    /// 422 + `details.fields[]` 字段级明细（T07：items/documents/photos 的输入校验）。
    ///
    /// 统一结构：`{ "fields": [ { "field": "name", "message": "…" }, … ] }`；
    /// `field` 为线上 camelCase 字段名，请求体整体问题用 `"body"`。
    pub fn field_validation(issues: Vec<manual_core::validation::FieldIssue>) -> Self {
        Self::validation_failed("请求内容未通过校验：请修正标出的字段后重试")
            .with_details(serde_json::json!({ "fields": issues }))
    }

    /// 422 + `details.reason`（已存在记录的**业务规则**冲突，如"该视图已有照片"）。
    ///
    /// 沿用 T06 的 `details.reason` 惯例（imagePixels/itemTotalLimit/insufficientStorage）：
    /// contracts.md §1 的稳定错误码集合没有独立的 409 码，本卡不改合同，用 422 + 可判定的
    /// reason 让前端零歧义识别（见 ADR-016）。
    pub fn unprocessable_reason(
        reason: &str,
        message: impl Into<String>,
        extra: serde_json::Value,
    ) -> Self {
        let mut details = extra.as_object().cloned().unwrap_or_default();
        details.insert(
            "reason".to_owned(),
            serde_json::Value::String(reason.to_owned()),
        );
        Self::validation_failed(message).with_details(serde_json::Value::Object(details))
    }

    /// 428：修改请求缺少 `If-Match`（contracts.md §1）。
    pub fn precondition_required(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::PRECONDITION_REQUIRED,
            ApiErrorCode::PreconditionRequired,
            message,
        )
    }

    /// 412：`If-Match` 的 revision 已过期；`details.currentRevision` 供前端刷新后重试。
    pub fn revision_conflict(current_revision: i64) -> Self {
        Self::new(
            StatusCode::PRECONDITION_FAILED,
            ApiErrorCode::RevisionConflict,
            "该资源已被其他操作更新：请刷新后重试",
        )
        .with_details(serde_json::json!({ "currentRevision": current_revision }))
    }

    /// 429：登录失败次数超出限速窗口。
    pub fn rate_limited(retry_after_seconds: u64) -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            ApiErrorCode::RateLimited,
            "登录尝试过于频繁：请稍后重试",
        )
        .with_header("retry-after", retry_after_seconds.to_string())
    }

    /// 413：请求体超过配置上限。
    pub fn payload_too_large() -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ApiErrorCode::PayloadTooLarge,
            "请求体超过允许的大小上限",
        )
    }

    /// 413：带具体原因（上传用途上限、物品累计上限、磁盘可用空间不足）。
    ///
    /// 原因必须能让用户采取行动（T06：AC-019 的"明确的磁盘不足错误"）。
    pub fn payload_too_large_with_message(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ApiErrorCode::PayloadTooLarge,
            message,
        )
    }

    /// 415：请求类型不受支持。
    pub fn unsupported_media_type() -> Self {
        Self::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ApiErrorCode::UnsupportedMediaType,
            "请求类型不受支持：JSON 接口要求 Content-Type: application/json",
        )
    }

    /// 500：未分类的服务端错误（细节不进入响应）。
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorCode::Internal,
            message,
        )
    }

    /// 409：供应商未配置（REQ-007：不回落 mock；`details.missing` 列出缺项）。
    pub fn provider_not_configured(missing: Vec<String>) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ApiErrorCode::ProviderNotConfigured,
            "供应商未配置：请在服务端配置密钥与模型后重试（已有资料仍可读，不会用 mock 冒充）",
        )
        .with_details(serde_json::json!({ "missing": missing }))
    }

    /// 409：价格目录缺失/不可用（REQ-020：缺价格配置时不能宣称精确费用）。
    pub fn price_catalog_missing(missing: Vec<String>) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ApiErrorCode::PriceCatalogMissing,
            "价格配置不完整：正式生成已阻止（不会按猜测的价格计算）",
        )
        .with_details(serde_json::json!({ "missing": missing }))
    }

    /// 409：幂等冲突（同 key 不同 body；contracts.md §4）。
    pub fn idempotency_conflict(message: impl Into<String>, details: serde_json::Value) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ApiErrorCode::IdempotencyConflict,
            message,
        )
        .with_details(details)
    }

    /// 存储层错误 → HTTP 错误。
    ///
    /// 不泄露 SQL：数据库约束/内部错误的细节只写服务端日志，响应只给通用文案；
    /// `RevisionConflict` 是合同要求的例外——`details.currentRevision` 明确可见
    /// （contracts.md §1）。
    pub fn from_storage(error: crate::storage::StorageError) -> Self {
        use crate::storage::StorageError;
        match error {
            StorageError::NotFound { entity, id } => {
                Self::not_found(format!("{entity} 不存在：{id}"))
            }
            StorageError::RevisionConflict {
                current_revision, ..
            } => Self::revision_conflict(current_revision),
            StorageError::ForeignKeyViolation { detail }
            | StorageError::UniqueViolation { detail }
            | StorageError::ConstraintViolation { detail } => {
                tracing::warn!(detail = %detail, "请求因数据库约束被拒绝");
                Self::validation_failed("请求内容未通过数据库约束校验")
            }
            other => {
                tracing::error!(error = %other, "存储错误（细节只进服务端日志）");
                Self::internal("服务器内部错误：请稍后重试或查看服务端日志")
            }
        }
    }

    /// 组装响应体、诊断 ID 与响应头。
    pub fn render(self, request_id: &str) -> Response {
        let body = ApiErrorResponse {
            error: ApiErrorBody {
                code: self.code,
                message: self.message,
                details: self.details,
                request_id: request_id.to_owned(),
            },
        };
        let mut response = (self.status, Json(body)).into_response();
        if let Ok(value) = HeaderValue::from_str(request_id) {
            response.headers_mut().insert("x-request-id", value);
        }
        for (name, value) in &self.headers {
            if let (Ok(name), Ok(value)) = (
                axum::http::HeaderName::try_from(*name),
                HeaderValue::from_str(value),
            ) {
                response.headers_mut().insert(name, value);
            }
        }
        response
            .extensions_mut()
            .insert(ResponseErrorCode(self.code));
        response
    }

    /// 无请求上下文时的兜底（测试专用；生产路径应传真实 [`RequestId`]）。
    pub fn into_api_response(self) -> Response {
        let generated = uuid::Uuid::now_v7().to_string();
        self.render(&generated)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        self.into_api_response()
    }
}

/// 生成请求服务错误 → 合同错误结构（T11）。
///
/// 映射规则（与 T07/T09 的既有惯例一致，见 ADR-016/ADR-019）：
/// - 结构/字段问题 → 422 + `details.fields`；
/// - 业务前置/冲突（过期、输入变化、预算不足、未确认…）→ 422 + `details.reason`；
/// - 未配置/幂等冲突 → 409（`PROVIDER_NOT_CONFIGURED` / `PRICE_CATALOG_MISSING` /
///   `IDEMPOTENCY_CONFLICT`，contracts.md §1 的 409 语义）；
/// - 存储/内部错误 → 由 [`ApiError::from_storage`] 统一处理（不泄露 SQL）。
impl From<crate::generation::GenerationError> for ApiError {
    fn from(error: crate::generation::GenerationError) -> Self {
        use crate::generation::GenerationError;
        match error {
            GenerationError::NotFound { message } => Self::not_found(message),
            GenerationError::FieldValidation(issues) => Self::field_validation(issues),
            GenerationError::Unprocessable {
                reason,
                message,
                details,
            } => Self::unprocessable_reason(reason, message, details),
            GenerationError::ProviderNotConfigured { missing } => {
                Self::provider_not_configured(missing)
            }
            GenerationError::PriceCatalogMissing { missing } => {
                Self::price_catalog_missing(missing)
            }
            GenerationError::IdempotencyConflict { message, details } => {
                Self::idempotency_conflict(message, details)
            }
            GenerationError::Storage(storage_error) => Self::from_storage(storage_error),
        }
    }
}

/// 任务控制动作错误 → 合同错误结构（T15：cancel/retry/reconcile）。
///
/// 映射规则（与 T11 的生成错误一致）：
/// - 404：job/阶段不存在（含跨 job 引用）；
/// - 422 + `details.fields`：字段级校验（缺 Idempotency-Key、缺必填字段/确认）；
/// - 422 + `details.reason`：业务前置不满足（不可重试、已取消、未知未对账、预算不足…）；
/// - 409：幂等冲突与"供应商未配置"（合同 §1）；
/// - 412：`If-Match` revision 过期。
impl From<crate::jobs::JobControlError> for ApiError {
    fn from(error: crate::jobs::JobControlError) -> Self {
        use crate::jobs::JobControlError;
        match error {
            JobControlError::NotFound { entity, id } => {
                Self::not_found(format!("{entity} 不存在：{id}"))
            }
            JobControlError::FieldValidation(issues) => Self::field_validation(issues),
            JobControlError::RevisionConflict {
                current_revision, ..
            } => Self::revision_conflict(current_revision),
            JobControlError::NotAllowed {
                reason,
                message,
                details,
            } => Self::unprocessable_reason(reason, message, details),
            JobControlError::IdempotencyConflict { message, details } => {
                Self::idempotency_conflict(message, details)
            }
            JobControlError::ProviderNotConfigured { missing } => {
                Self::provider_not_configured(missing)
            }
            JobControlError::Storage(storage_error) => Self::from_storage(storage_error),
        }
    }
}

/// 草稿服务错误 → 合同错误结构（T15/T19）。
impl From<crate::drafts::DraftServiceError> for ApiError {
    fn from(error: crate::drafts::DraftServiceError) -> Self {
        use crate::drafts::DraftServiceError;
        match error {
            DraftServiceError::NotFound { entity, id } => {
                Self::not_found(format!("{entity} 不存在：{id}"))
            }
            DraftServiceError::Integrity { code, message } => {
                Self::unprocessable_reason(code, message, serde_json::json!({}))
            }
            DraftServiceError::FieldIssues(issues) => Self::field_validation(issues),
            DraftServiceError::Storage(storage_error) => Self::from_storage(storage_error),
        }
    }
}

/// 发布服务错误 → 合同错误结构（T19；AC-055/AC-056）。
///
/// 映射规则：
/// - 404：草稿/发布版本不存在（含跨物品）；
/// - 422 + `details.fields`：缺 `Idempotency-Key` 等字段级问题；
/// - 422 + `details.issues`：**发布不变量逐条明细**（`reason: publishInvariantsViolated`）；
/// - 412：`If-Match` revision 过期/并发修改（`details.currentRevision`）；
/// - 409：同 key 不同 body（`IDEMPOTENCY_CONFLICT`）；
/// - 500：内部/存储错误（细节只进服务端日志）。
impl From<crate::releases::PublishError> for ApiError {
    fn from(error: crate::releases::PublishError) -> Self {
        use crate::releases::PublishError;
        match error {
            PublishError::NotFound { entity, id } => {
                Self::not_found(format!("{entity} 不存在：{id}"))
            }
            PublishError::FieldIssues(issues) => Self::field_validation(issues),
            PublishError::Invariants(issues) => {
                let issues_json: Vec<serde_json::Value> = issues
                    .iter()
                    .map(|issue| serde_json::to_value(issue).unwrap_or(serde_json::Value::Null))
                    .collect();
                Self::validation_failed(
                    "发布条件不满足：请按下列不满足项逐条处理后重试（不自动隐藏必需内容、不代你确认）",
                )
                .with_details(serde_json::json!({
                    "reason": "publishInvariantsViolated",
                    "issues": issues_json,
                }))
            }
            PublishError::IdempotencyConflict { message, details } => {
                Self::idempotency_conflict(message, details)
            }
            PublishError::Integrity { code, message } => {
                Self::unprocessable_reason(code, message, serde_json::json!({}))
            }
            PublishError::Storage(storage_error) => Self::from_storage(storage_error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn not_found_response_uses_contract_error_shape() {
        let response = ApiError::not_found("接口不存在").into_api_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(response.headers().contains_key("x-request-id"));

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "NOT_FOUND");
        assert_eq!(value["error"]["message"], "接口不存在");
        assert!(value["error"]["details"].is_null());
        let request_id = value["error"]["requestId"].as_str().unwrap();
        assert!(
            uuid::Uuid::parse_str(request_id).is_ok(),
            "requestId 应为 UUID"
        );
    }

    #[tokio::test]
    async fn render_uses_supplied_request_id_and_extra_headers() {
        let response = ApiError::rate_limited(42).render("01993000-0000-7000-8000-000000000002");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()["retry-after"], "42");
        assert_eq!(
            response.headers()["x-request-id"],
            "01993000-0000-7000-8000-000000000002"
        );

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "RATE_LIMITED");
        assert_eq!(
            value["error"]["requestId"],
            "01993000-0000-7000-8000-000000000002"
        );
    }

    #[tokio::test]
    async fn conflict_error_exposes_current_revision_only_in_details() {
        let response = ApiError::revision_conflict(8).into_api_response();
        assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "REVISION_CONFLICT");
        assert_eq!(value["error"]["details"]["currentRevision"], 8);
        // 顶层只有 error 一个键，错误体只有四个键。
        assert_eq!(value.as_object().unwrap().len(), 1);
        assert_eq!(value["error"].as_object().unwrap().len(), 4);
    }
}
