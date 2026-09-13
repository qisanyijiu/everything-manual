//! manual-core —— 领域类型与合同不变量。
//!
//! 依赖方向（architecture.md §4）：`server → core`。本 crate 不依赖 Axum、SQLx
//! 或浏览器运行时；T01 只包含最小的 HTTP 合同面，T03 起加入合同核心领域类型
//! （[`domain`]）、实体 ID（[`ids`]）与时间戳（[`timestamps`]）。
//!
//! 这里的类型是手写代码与生成 OpenAPI 共用的唯一来源（ADR-009）；前端类型由
//! `cargo xtask contracts` 从 Rust DTO 生成，禁止手抄。

pub mod cost;
pub mod domain;
pub mod generation;
pub mod ids;
pub mod jobs;
pub mod knowledge;
pub mod timestamps;
pub mod validation;

use serde::{Deserialize, Serialize};

/// HTTP API 路径前缀（contracts.md §1：所有路由统一 `/api/v1`）。
pub const API_PREFIX: &str = "/api/v1";

/// 统一错误码（语义见 llmdoc/contracts.md §1）。
///
/// 序列化为 SCREAMING_SNAKE_CASE（如 `NOT_FOUND`）。T04 起覆盖认证与统一错误映射
/// 的当前集合；后续卡按 contracts.md 继续追加（如 `PROVIDER_NOT_CONFIGURED`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiErrorCode {
    /// 资源或接口不存在。
    NotFound,
    /// 未登录或会话已失效（cookie 缺失/过期/已注销）。
    Unauthorized,
    /// CSRF token 缺失或不匹配（403）。
    CsrfRejected,
    /// Origin 不在允许列表（403）。
    OriginRejected,
    /// 请求体未通过校验（缺字段、格式错误、不支持的条件请求值）。
    ValidationFailed,
    /// 修改请求缺少 `If-Match`（428）。
    PreconditionRequired,
    /// 乐观锁冲突：`If-Match` 的 revision 已过期（412，`details.currentRevision`）。
    RevisionConflict,
    /// 登录失败次数超出限速窗口（429）。
    RateLimited,
    /// 请求体超过配置上限（413）。
    PayloadTooLarge,
    /// 请求类型不受支持（415，例如非 application/json）。
    UnsupportedMediaType,
    /// 供应商未配置（409；REQ-007：不回落 mock、不产生假成功）。
    ProviderNotConfigured,
    /// 价格目录缺失/不可用（409；REQ-020：缺价格配置时不能宣称精确费用）。
    PriceCatalogMissing,
    /// 幂等冲突：同 key 不同 body（409；contracts.md §4）。
    IdempotencyConflict,
    /// 未分类的服务端错误（错误详情只进服务端日志，不进响应）。
    Internal,
}

impl ApiErrorCode {
    /// 与序列化一致的字符串形式，用于日志与测试断言。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "NOT_FOUND",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::CsrfRejected => "CSRF_REJECTED",
            Self::OriginRejected => "ORIGIN_REJECTED",
            Self::ValidationFailed => "VALIDATION_FAILED",
            Self::PreconditionRequired => "PRECONDITION_REQUIRED",
            Self::RevisionConflict => "REVISION_CONFLICT",
            Self::RateLimited => "RATE_LIMITED",
            Self::PayloadTooLarge => "PAYLOAD_TOO_LARGE",
            Self::UnsupportedMediaType => "UNSUPPORTED_MEDIA_TYPE",
            Self::ProviderNotConfigured => "PROVIDER_NOT_CONFIGURED",
            Self::PriceCatalogMissing => "PRICE_CATALOG_MISSING",
            Self::IdempotencyConflict => "IDEMPOTENCY_CONFLICT",
            Self::Internal => "INTERNAL",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_prefix_is_versioned() {
        assert_eq!(API_PREFIX, "/api/v1");
    }

    #[test]
    fn error_code_wire_format_is_screaming_snake_case() {
        assert_eq!(
            serde_json::to_string(&ApiErrorCode::NotFound).unwrap(),
            "\"NOT_FOUND\""
        );
        assert_eq!(ApiErrorCode::Internal.as_str(), "INTERNAL");
        assert_eq!(
            serde_json::from_str::<ApiErrorCode>("\"NOT_FOUND\"").unwrap(),
            ApiErrorCode::NotFound
        );
        // 序列化字符串与 as_str 必须一致（日志与响应体共用同一取值）。
        for code in [
            ApiErrorCode::NotFound,
            ApiErrorCode::Unauthorized,
            ApiErrorCode::CsrfRejected,
            ApiErrorCode::OriginRejected,
            ApiErrorCode::ValidationFailed,
            ApiErrorCode::PreconditionRequired,
            ApiErrorCode::RevisionConflict,
            ApiErrorCode::RateLimited,
            ApiErrorCode::PayloadTooLarge,
            ApiErrorCode::UnsupportedMediaType,
            ApiErrorCode::ProviderNotConfigured,
            ApiErrorCode::PriceCatalogMissing,
            ApiErrorCode::IdempotencyConflict,
            ApiErrorCode::Internal,
        ] {
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(json, format!("\"{}\"", code.as_str()));
        }
    }
}
