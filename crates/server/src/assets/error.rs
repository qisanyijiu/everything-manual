//! 资产服务的错误类型与 HTTP 映射（contracts.md §1 的错误码集合）。
//!
//! 映射表（QA 按此复核）：
//!
//! | 场景 | 状态 | code |
//! | --- | --- | --- |
//! | 文件超过用途上限 / 请求体超过上传上限 | 413 | `PAYLOAD_TOO_LARGE` |
//! | 磁盘可用空间不足（`details.reason=insufficientStorage`） | 413 | `PAYLOAD_TOO_LARGE` |
//! | 物品累计体积超限（`details.reason=itemTotalLimit`） | 413 | `PAYLOAD_TOO_LARGE` |
//! | 内容类型与用途不符 / 伪造类型（magic 不是声称的类型） | 415 | `UNSUPPORTED_MEDIA_TYPE` |
//! | 像素炸弹、结构/解码校验失败、PDF 页数超限、purpose 非法 | 422 | `VALIDATION_FAILED` |
//! | 资产不存在、已隔离、内容文件缺失、跨物品归属 | 404 | `NOT_FOUND` |
//! | IO/数据库内部错误（细节只进服务端日志） | 500 | `INTERNAL` |
//!
//! **磁盘满为什么是 413 而不是 507**：contracts.md §1 的稳定错误码集合里没有
//! 507/`INSUFFICIENT_STORAGE`，而 PRD AC-019 只要求"明确的磁盘不足错误"。这里选择
//! 413 + `details.reason=insufficientStorage`（可被前端与测试精确定位），并把
//! "是否新增 507/独立错误码"留作合同变更提案（见 implementation §T06 已知限制），
//! 不在本卡私自扩展 `manual_core::ApiErrorCode`。

use manual_core::ApiErrorCode;
use serde_json::json;

use crate::storage::StorageError;

use super::blob_store::insufficient_space_message;

/// 资产服务错误。
#[derive(Debug, Clone)]
pub enum AssetError {
    /// 体积超过上限（请求体/本用途上限/物品累计/磁盘可用空间，见 `details.reason`）。
    PayloadTooLarge {
        message: String,
        details: Option<serde_json::Value>,
    },
    /// 内容类型不受支持（伪造类型、用途与内容不符）。
    UnsupportedType { message: String },
    /// 内容校验失败（magic/结构/尺寸/像素/页数/编码）。
    InvalidContent {
        message: String,
        details: Option<serde_json::Value>,
    },
    /// 资源不存在或不可见（含跨物品归属；不泄露存在性）。
    NotFound { message: String },
    /// 磁盘可用空间不足：明确错误，不半提交资产。
    InsufficientStorage { required: u64, available: u64 },
    /// 物品累计体积超限（contracts.md §7：默认 500 MiB）。
    ItemTotalExceeded {
        limit: u64,
        current: u64,
        incoming: u64,
    },
    /// multipart 请求体层面的错误（可携带 HTTP 层判定的状态码）。
    Multipart {
        detail: String,
        payload_too_large: bool,
        bad_request: bool,
    },
    /// 存储层错误（映射走统一规则）。
    Storage(StorageError),
    /// 文件系统/内部错误（细节只进日志）。
    Io { detail: String },
}

impl AssetError {
    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::PayloadTooLarge {
            message: message.into(),
            details: None,
        }
    }

    pub fn unsupported_type(message: impl Into<String>) -> Self {
        Self::UnsupportedType {
            message: message.into(),
        }
    }

    pub fn invalid_content(message: impl Into<String>) -> Self {
        Self::InvalidContent {
            message: message.into(),
            details: None,
        }
    }

    pub fn invalid_content_with_details(
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self::InvalidContent {
            message: message.into(),
            details: Some(details),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }

    pub fn insufficient_storage(required: u64, available: u64) -> Self {
        Self::InsufficientStorage {
            required,
            available,
        }
    }

    pub fn io(detail: impl Into<String>) -> Self {
        Self::Io {
            detail: detail.into(),
        }
    }

    /// 错误是否属于"客户端可以修正"的类别（用于日志级别：4xx 记 warn，5xx 记 error）。
    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            Self::PayloadTooLarge { .. }
                | Self::UnsupportedType { .. }
                | Self::InvalidContent { .. }
                | Self::NotFound { .. }
                | Self::InsufficientStorage { .. }
                | Self::ItemTotalExceeded { .. }
                | Self::Multipart { .. }
        )
    }

    /// 日志用摘要（**不含**磁盘路径与文件内容）。
    pub fn log_summary(&self) -> String {
        match self {
            Self::PayloadTooLarge { message, .. } => format!("体积超限：{message}"),
            Self::UnsupportedType { message } => format!("类型不支持：{message}"),
            Self::InvalidContent { message, .. } => format!("内容校验失败：{message}"),
            Self::NotFound { message } => format!("资产不可见：{message}"),
            Self::InsufficientStorage {
                required,
                available,
            } => format!("磁盘空间不足：需要 {required} 字节，可用 {available} 字节"),
            Self::ItemTotalExceeded {
                limit,
                current,
                incoming,
            } => format!("物品累计体积超限：上限 {limit}，已有 {current}，本次 {incoming}"),
            Self::Multipart { detail, .. } => {
                // 只进服务端日志；截断防止异常长文本污染日志。
                let detail: String = detail.chars().take(200).collect();
                format!("multipart 请求体错误：{detail}")
            }
            Self::Storage(error) => format!("存储错误：{error}"),
            Self::Io { detail } => format!("内部错误：{detail}"),
        }
    }

    /// 转成统一错误响应结构（由 handler 用本次请求的 `requestId` 渲染）。
    pub fn into_api_error(self) -> crate::http::error::ApiError {
        use crate::http::error::ApiError;
        match self {
            Self::PayloadTooLarge { message, details } => {
                let error = ApiError::payload_too_large_with_message(message);
                match details {
                    Some(details) => error.with_details(details),
                    None => error,
                }
            }
            Self::UnsupportedType { message } => ApiError::new(
                axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                ApiErrorCode::UnsupportedMediaType,
                message,
            ),
            Self::InvalidContent { message, details } => {
                let error = ApiError::validation_failed(message);
                match details {
                    Some(details) => error.with_details(details),
                    None => error,
                }
            }
            Self::NotFound { message } => ApiError::not_found(message),
            Self::InsufficientStorage {
                required,
                available,
            } => ApiError::payload_too_large_with_message(insufficient_space_message(
                required, available,
            ))
            .with_details(json!({
                "reason": "insufficientStorage",
                "requiredBytes": required,
                "availableBytes": available,
            })),
            Self::ItemTotalExceeded {
                limit,
                current,
                incoming,
            } => ApiError::payload_too_large_with_message(format!(
                "物品累计体积将超过上限：上限 {limit} 字节，已有 {current} 字节，\
                 本次 {incoming} 字节；请先清理该物品的其他资料"
            ))
            .with_details(json!({
                "reason": "itemTotalLimit",
                "limitBytes": limit,
                "currentBytes": current,
                "incomingBytes": incoming,
            })),
            Self::Multipart {
                detail,
                payload_too_large,
                bad_request,
            } => {
                // 响应只给固定文案：底层解析细节（可能含请求头片段）只进服务端日志。
                if payload_too_large {
                    ApiError::payload_too_large_with_message("请求体超过上传大小上限")
                } else if bad_request {
                    ApiError::validation_failed(
                        "multipart 请求体不合法：缺少 boundary、部件头不完整或字段未正常结束",
                    )
                } else {
                    tracing::error!(detail = %detail, "multipart 读取失败");
                    ApiError::internal("服务器内部错误：读取上传内容失败")
                }
            }
            Self::Storage(error) => ApiError::from_storage(error),
            Self::Io { detail } => {
                tracing::error!(detail = %detail, "资产服务内部错误（细节只进服务端日志）");
                ApiError::internal("服务器内部错误：请稍后重试或查看服务端日志")
            }
        }
    }
}

impl From<StorageError> for AssetError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}
