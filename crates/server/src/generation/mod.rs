//! 生成请求服务（T11）：报价、确认、冻结输入、费用预留与幂等建单。
//!
//! 对应合同（contracts.md §4「输入快照与费用合同」）与 PRD REQ-020–REQ-023：
//! - [`estimate`]：`POST /items/{id}/estimates` 的校验与计算（只计算计划，
//!   不调用任何生成服务、不产生费用记录）；
//! - [`jobs`]：`POST /items/{id}/jobs` 的重新校验、同一事务内冻结快照 + 预留费用 +
//!   创建 job + 幂等记录；
//! - [`catalog`]：`price_catalog_path` 的解析（精确十进制 → 最小整数单位）；
//! - [`ledger`]：预留/结算/释放的服务级封装（unknown 保留预留，不填 0）。
//!
//! 错误统一为 [`GenerationError`]，由 HTTP 层一次性映射为合同错误结构
//! （422 字段级 / 422 + `details.reason` / 409 未配置与幂等冲突）。

pub mod catalog;
pub mod estimate;
pub mod jobs;
pub mod ledger;

use manual_core::validation::FieldIssue;

use crate::storage::StorageError;

/// 生成请求服务的错误（HTTP 层映射见 `http::estimates` / `http::jobs` 的 `From`）。
#[derive(Debug, Clone)]
pub enum GenerationError {
    /// 404：资源不存在或不属于该物品（跨物品与"不存在"响应一致，不泄露存在性）。
    NotFound { message: String },
    /// 422 + `details.fields`：请求结构/字段级校验失败。
    FieldValidation(Vec<FieldIssue>),
    /// 422 + `details.reason`：业务前置/冲突（沿用 T06/T07/T09 的 reason 惯例，
    /// contracts.md §1 的稳定错误码集合没有独立 409 码）。
    Unprocessable {
        reason: &'static str,
        message: String,
        details: serde_json::Value,
    },
    /// 409 `PROVIDER_NOT_CONFIGURED`：缺供应商配置（不回落 mock，不返回假成功）。
    ProviderNotConfigured { missing: Vec<String> },
    /// 409 `PRICE_CATALOG_MISSING`：缺价格目录、目录里没有该模型/预设，
    /// 或配置的 Tripo 模型与预设不一致（不能宣称精确费用）。
    PriceCatalogMissing { missing: Vec<String> },
    /// 409 `IDEMPOTENCY_CONFLICT`：同 key 不同 body。
    IdempotencyConflict {
        message: String,
        details: serde_json::Value,
    },
    /// 存储层错误（由 HTTP 层按统一规则映射；不泄露 SQL）。
    Storage(StorageError),
}

impl GenerationError {
    /// `details.reason` 快捷构造（`details` 为空对象）。
    pub fn unprocessable(reason: &'static str, message: impl Into<String>) -> Self {
        Self::Unprocessable {
            reason,
            message: message.into(),
            details: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    /// 带明细的 `details.reason`。
    pub fn unprocessable_with(
        reason: &'static str,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self::Unprocessable {
            reason,
            message: message.into(),
            details,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }
}

impl From<StorageError> for GenerationError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::Error> for GenerationError {
    /// 事务原语（begin/commit/rollback）失败按存储错误处理（不吞掉、不重试）。
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(StorageError::from(error))
    }
}
