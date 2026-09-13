//! `jobs` 表（父任务）的仓储原语（T10 / REQ-024）。
//!
//! **边界**：只做持久化与数据库层不变量。**建单入口（报价、快照、幂等键、
//! 费用预留）属 T11**；本模块提供执行器与后续卡共用的原语：
//! - `create`：入队一个 job（T11 在同一事务里与快照/预留一起调用）；
//! - `recompute_status`：按全部阶段状态聚合父 job 状态（规则见
//!   [`manual_core::jobs::aggregate_job_status`]），只在状态变化时自增 `revision`；
//! - `cancel`：取消 job（未提交阶段转 `cancelled`，已提交/未决阶段保留状态）。
//!
//! 父 job 的 `revision` 是乐观锁（contracts.md §1：`If-Match` 用于 cancel/retry，
//! 属 T15 的 HTTP 面）；执行器内部的状态推进也自增 revision，因为它是可观察的聚合变更。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::{Job, JobStatus};
use manual_core::ids;
use manual_core::jobs::aggregate_job_status;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

use super::{job_stages, parse_job_status};

/// 建单输入（`item_id` / `snapshot_id` 由 T11 的入队事务提供）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    pub item_id: String,
    pub snapshot_id: String,
}

const SELECT_JOB_SQL: &str = "SELECT id, item_id, snapshot_id, status, revision, created_at, updated_at \
     FROM jobs WHERE id = ?";

/// 创建 job（`status = queued`、`revision = 1`）。
pub async fn create(conn: &mut SqliteConnection, new: NewJob) -> Result<Job, StorageError> {
    let now = Timestamp::now();
    let job = Job {
        id: ids::new_id(),
        item_id: new.item_id,
        snapshot_id: new.snapshot_id,
        status: JobStatus::Queued,
        revision: 1,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO jobs (id, item_id, snapshot_id, status, revision, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&job.id)
    .bind(&job.item_id)
    .bind(&job.snapshot_id)
    .bind(job.status.as_str())
    .bind(job.revision)
    .bind(job.created_at.as_millis())
    .bind(job.updated_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(job)
}

/// 按 id 读取 job；不存在返回 `Ok(None)`。
pub async fn get(conn: &mut SqliteConnection, id: &str) -> Result<Option<Job>, StorageError> {
    let row = sqlx::query(SELECT_JOB_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| job_from_row(&row)).transpose()
}

/// 按全部阶段状态重算父 job 状态（contracts.md §5 父 job 行）。
///
/// 与阶段状态推进放在同一事务内调用，保证"阶段已 succeeded 而父 job 仍 running"
/// 不会成为持久状态（崩溃在两者之间时，下一次推进或恢复会重新聚合）。
/// 已终态（succeeded/failed/cancelled）不被内部聚合改写（见 core 的优先级说明）。
pub async fn recompute_status(
    conn: &mut SqliteConnection,
    job_id: &str,
    now: Timestamp,
) -> Result<Option<Job>, StorageError> {
    let Some(mut job) = get(conn, job_id).await? else {
        return Ok(None);
    };
    let stages = job_stages::statuses_for_job(conn, job_id).await?;
    let next = aggregate_job_status(job.status, &stages);
    if next == job.status {
        return Ok(Some(job));
    }
    let changed = sqlx::query(
        "UPDATE jobs SET status = ?, revision = revision + 1, updated_at = ? \
          WHERE id = ? AND status = ?",
    )
    .bind(next.as_str())
    .bind(now.as_millis())
    .bind(job_id)
    .bind(job.status.as_str())
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if changed == 0 {
        // 并发推进/取消抢先：重读返回权威状态，不覆盖别人的写入。
        return get(conn, job_id).await;
    }
    job = get(conn, job_id)
        .await?
        .ok_or_else(|| StorageError::NotFound {
            entity: "job",
            id: job_id.to_owned(),
        })?;
    Ok(Some(job))
}

/// 人工重试/对账后的父任务状态收敛（T15 / REQ-026）。
///
/// [`recompute_status`] 把终态视为不可改写——这对自动聚合是对的（"恢复先接管"不
/// 允许把已失败的 job 悄悄复活），但**人工重试**恰恰要求把 `failed` 的父任务拉回
/// 进行中：阶段已被重新排队，父 job 仍显示 `failed` 会自相矛盾。
///
/// 规则：仅当当前状态是 `failed` 时才允许"逃逸"终态——用 `running` 作为聚合基准
/// （其余优先级不变：unknown > needs_input > failed > 全部 succeeded > …）。
/// 若聚合结果仍是 `failed`（例如另一分支仍失败），保持原值不写库。
pub async fn recompute_status_after_retry(
    conn: &mut SqliteConnection,
    job_id: &str,
    now: Timestamp,
) -> Result<Option<Job>, StorageError> {
    let Some(job) = get(conn, job_id).await? else {
        return Ok(None);
    };
    if job.status != JobStatus::Failed {
        return recompute_status(conn, job_id, now).await;
    }
    let stages = job_stages::statuses_for_job(conn, job_id).await?;
    let next = aggregate_job_status(JobStatus::Running, &stages);
    if next == JobStatus::Failed {
        // 仍有失败分支（例如另一条分支仍未完成）：不制造"已经恢复"的假象。
        return Ok(Some(job));
    }
    let changed = sqlx::query(
        "UPDATE jobs SET status = ?, revision = revision + 1, updated_at = ? \
          WHERE id = ? AND status = 'failed'",
    )
    .bind(next.as_str())
    .bind(now.as_millis())
    .bind(job_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if changed == 0 {
        return get(conn, job_id).await;
    }
    get(conn, job_id).await
}

/// 取消结果（审计与 API 层展示用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOutcome {
    /// job 是否从非终态转为 `cancelled`（已终态 → false，不做无意义写入）。
    pub cancelled: bool,
    /// 本次转为 `cancelled` 的阶段数（未提交阶段；见 [`job_stages::cancel_unsubmitted_for_job`]）。
    pub stages_cancelled: u64,
    pub job: Job,
}

/// 取消 job（REQ-026 / contracts.md §5「取消」行）。
///
/// - 未提交阶段（`queued`/`retry_wait`/`needs_input`，以及无已接受 attempt 的 `running`）
///   转 `cancelled`：执行器永不再领取；
/// - 已提交阶段（`waiting_provider`）与 `submission_unknown` **保留原状态**：
///   供应商侧可能已在计费，收尾/对账属 T15，本函数不假装取消远端付费操作；
/// - 已终态 job 返回 `cancelled = false`（调用方按 T15 的 409 语义处理）。
pub async fn cancel(
    conn: &mut SqliteConnection,
    job_id: &str,
    now: Timestamp,
) -> Result<CancelOutcome, StorageError> {
    let job = get(conn, job_id)
        .await?
        .ok_or_else(|| StorageError::NotFound {
            entity: "job",
            id: job_id.to_owned(),
        })?;
    if job.status.is_terminal() {
        return Ok(CancelOutcome {
            cancelled: false,
            stages_cancelled: 0,
            job,
        });
    }
    let stages_cancelled = job_stages::cancel_unsubmitted_for_job(conn, job_id, now).await?;
    sqlx::query(
        "UPDATE jobs SET status = 'cancelled', revision = revision + 1, updated_at = ? \
          WHERE id = ? AND status <> 'cancelled'",
    )
    .bind(now.as_millis())
    .bind(job_id)
    .execute(&mut *conn)
    .await?;
    let job = get(conn, job_id)
        .await?
        .ok_or_else(|| StorageError::NotFound {
            entity: "job",
            id: job_id.to_owned(),
        })?;
    Ok(CancelOutcome {
        cancelled: true,
        stages_cancelled,
        job,
    })
}

/// 稳定排序的一页（T15 / REQ-031：任务中心列表）：`(created_at DESC, id DESC)`。
///
/// `item_id = None` 列出全部任务；`Some(id)` 只列该物品的任务（游标作用域与过滤条件
/// 绑定，见 `http::pagination`）。`limit` 由调用方封顶（默认 20、最多 100）。
pub async fn list_page(
    conn: &mut SqliteConnection,
    item_id: Option<&str>,
    cursor: Option<(i64, String)>,
    limit: u32,
) -> Result<Vec<Job>, StorageError> {
    let (cursor_millis, cursor_id) = match cursor {
        Some((millis, id)) => (Some(millis), Some(id)),
        None => (None, None),
    };
    let sql = if item_id.is_some() {
        LIST_FOR_ITEM_SQL
    } else {
        LIST_ALL_SQL
    };
    let rows = match item_id {
        Some(item_id) => {
            sqlx::query(sql)
                .bind(item_id)
                .bind(cursor_millis)
                .bind(cursor_millis)
                .bind(cursor_id)
                .bind(limit)
                .fetch_all(&mut *conn)
                .await?
        }
        None => {
            sqlx::query(sql)
                .bind(cursor_millis)
                .bind(cursor_millis)
                .bind(cursor_id)
                .bind(limit)
                .fetch_all(&mut *conn)
                .await?
        }
    };
    rows.iter().map(job_from_row).collect()
}

/// 列清单与 [`job_from_row`] 对应（静态 SQL；只差物品过滤条件）。
const LIST_ALL_SQL: &str = "SELECT id, item_id, snapshot_id, status, revision, created_at, updated_at \
       FROM jobs \
      WHERE (? IS NULL OR (created_at, id) < (?, ?)) \
      ORDER BY created_at DESC, id DESC \
      LIMIT ?";

const LIST_FOR_ITEM_SQL: &str = "SELECT id, item_id, snapshot_id, status, revision, created_at, updated_at \
       FROM jobs \
      WHERE item_id = ? AND (? IS NULL OR (created_at, id) < (?, ?)) \
      ORDER BY created_at DESC, id DESC \
      LIMIT ?";

fn job_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Job, StorageError> {
    Ok(Job {
        id: row.try_get("id")?,
        item_id: row.try_get("item_id")?,
        snapshot_id: row.try_get("snapshot_id")?,
        status: parse_job_status(row.try_get::<String, _>("status")?.as_str())?,
        revision: row.try_get("revision")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
        updated_at: Timestamp::from_millis(row.try_get("updated_at")?),
    })
}
