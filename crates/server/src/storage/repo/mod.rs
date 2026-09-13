//! 仓储原语（repository）：路由/服务层访问数据库的唯一入口。
//!
//! 分层（architecture.md §4）：handler 负责 DTO 与认证，服务层负责用例，
//! **仓储层只负责持久化**。本模块提供的事务都是短事务，不跨请求持有。
//!
//! 已实现：`items`（T03，含 revision CAS 与归档过滤）、`admins`/`sessions`（T04）、
//! `blobs`/`assets`（T06，内容寻址存储与资产归属）、`documents`/`photos`（T07，
//! 说明书绑定与多视图照片）、`preparations`（T09，PDF 页准备与封存）、
//! `jobs`/`job_stages`/`attempts`/`audit`（T10，任务执行器：领取、租约 epoch、
//! DAG 依赖边、提交窗口事实）、`quotes`/`snapshots`/`ledger`/`idempotency`（T11，
//! 报价快照、冻结输入、费用预留与幂等记录）、`model_revisions`（T13，不可变模型版本）、
//! `drafts`（T15，版本化草稿：幂等组装与 revision CAS）、`releases`（T19，
//! 不可变发布记录）。
//! 后续卡在其后按需追加模块：导出与恢复（T20）。

pub mod admins;
pub mod assets;
pub mod attempts;
pub mod audit;
pub mod blobs;
pub mod documents;
pub mod drafts;
pub mod idempotency;
pub mod items;
pub mod job_stages;
pub mod jobs;
pub mod ledger;
pub mod model_revisions;
pub mod photos;
pub mod preparations;
pub mod quotes;
pub mod releases;
pub mod sessions;
pub mod snapshots;

use manual_core::domain::{JobStatus, StageKind};

use crate::storage::error::StorageError;

/// SQL 文本 → [`JobStatus`]；未知取值按损坏数据处理（不静默兜底）。
pub(crate) fn parse_job_status(value: &str) -> Result<JobStatus, StorageError> {
    JobStatus::from_sql(value).ok_or_else(|| StorageError::Database {
        detail: format!("job status 取值未知：{value}"),
    })
}

/// SQL 文本 → [`StageKind`]；未知取值按损坏数据处理。
pub(crate) fn parse_stage_kind(value: &str) -> Result<StageKind, StorageError> {
    StageKind::from_sql(value).ok_or_else(|| StorageError::Database {
        detail: format!("stage_kind 取值未知：{value}"),
    })
}
