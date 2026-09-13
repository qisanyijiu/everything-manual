//! 发布与版本（T19 / REQ-035、REQ-036；contracts.md §3/§7、ADR-005）。
//!
//! 本模块是**唯一的** `manual_releases` 写入者：只有 [`service::publish_draft`]
//! （由 `POST /items/{id}/drafts/{draftId}/publish` 触发）会创建发布版本。
//! 组装草稿（`crate::drafts`）与任何后台阶段都不会写 release——不存在自动发布路径。
//!
//! - [`invariants`]：发布不变量的纯函数判据（逐条可测；服务端是唯一权威）；
//! - [`service`]：发布事务、manifest 冻结与版本读取。

pub mod invariants;
pub mod service;

pub use invariants::{FrozenInput, PublishIssue, check_publish_invariants};
pub use service::{
    PUBLISH_IDEMPOTENCY_METHOD, PUBLISH_IDEMPOTENCY_ROUTE, PublishError, PublishOutcome,
    RELEASE_SCHEMA_VERSION, list_releases, publish_draft, read_manifest, read_release,
};
