//! HTTP DTO —— 机器合同的唯一来源（ADR-009）。
//!
//! 约定（contracts.md §1）：JSON 字段 camelCase、单项响应 `{ "data": ... }`、
//! 错误统一 `error.code/message/details/requestId`。前端类型由
//! `cargo xtask contracts` 生成，禁止手抄。
//!
//! 按资源拆分子模块（本文件保留错误与健康探针；T04 起新增 auth/settings/items，
//! 并在此统一 re-export，`openapi.rs` 与 handler 只引用 `super::dto::X`）。

pub mod assets;
pub mod auth;
pub mod documents;
pub mod drafts;
pub mod generation;
pub mod items;
pub mod jobs;
pub mod photos;
pub mod preparations;
pub mod releases;
pub mod settings;

pub use assets::*;
pub use auth::*;
pub use documents::*;
pub use drafts::*;
pub use generation::*;
pub use items::*;
pub use jobs::*;
pub use photos::*;
pub use preparations::*;
pub use releases::*;
pub use settings::*;

use manual_core::ApiErrorCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ---------------------------------------------------------------------------
// 统一错误响应
// ---------------------------------------------------------------------------

/// 统一错误响应包。任何非 2xx 的应用错误都使用该结构。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorResponse {
    pub error: ApiErrorBody,
}

/// 错误体。不输出堆栈、SQL、密钥或完整供应商签名 URL（contracts.md §1）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrorBody {
    /// 机器可读错误码（SCREAMING_SNAKE_CASE，来自 `manual_core::ApiErrorCode`）。
    #[schema(value_type = String, example = "NOT_FOUND")]
    pub code: ApiErrorCode,
    /// 面向用户的中文信息，不含内部细节。
    #[schema(example = "接口不存在")]
    pub message: String,
    /// 结构化细节；无细节时为 null。
    #[schema(value_type = Option<serde_json::Value>)]
    pub details: Option<serde_json::Value>,
    /// 本次响应的诊断 ID（T04 起与请求日志关联）。
    #[schema(example = "01993000-0000-7000-8000-000000000001")]
    pub request_id: String,
}

// ---------------------------------------------------------------------------
// 健康探针
// ---------------------------------------------------------------------------

/// `GET /api/v1/health/live` 响应包。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub struct HealthLiveResponse {
    pub data: LivenessData,
}

/// 存活载荷：进程能响应请求即为 ok。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub struct LivenessData {
    pub status: LivenessStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LivenessStatus {
    Ok,
}

/// `GET /api/v1/health/ready` 响应包。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthReadyResponse {
    pub data: ReadinessData,
}

/// 就绪载荷。
///
/// T04 语义：检查项为 `process` / `data_directory` / `database` / `migrations`，
/// **任一项失败**时 `status = not_ready` 且整体返回 503。**不检查云端可达**
/// （REQ-007/REQ-038：供应商不可达不得使就绪失败），也不返回任何配置细节。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessData {
    pub status: ReadinessStatus,
    pub checks: Vec<ReadinessCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStatus {
    Ready,
    NotReady,
}

/// 单项自检结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Fail,
}

/// 自检项名称。T04 起为四项（顺序固定：process、data_directory、database、migrations）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCheckName {
    /// 进程可响应请求（存活即通过）。
    Process,
    /// data-dir 存在、可写、数据库文件在预期位置（不创建/修改任何内容）。
    DataDirectory,
    /// 数据库连接可用（只读探测 `SELECT 1`）。
    Database,
    /// 已应用迁移版本与程序支持版本一致。
    Migrations,
}

/// 单个自检项。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub struct ReadinessCheck {
    pub name: ReadinessCheckName,
    pub status: CheckStatus,
}
