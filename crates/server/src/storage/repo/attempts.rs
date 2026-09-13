//! `provider_attempts` 表的仓储原语（T10 / contracts.md §5 提交窗口）。
//!
//! **边界**：只做持久化与 SQL 层不变量。提交窗口的**顺序**（先 intent、再
//! submitting、再事实观察、最后业务推进）在 `crate::jobs::submission`；费用预留与
//! 结算属 T11（`cost_ledger`）。
//!
//! 关键语义（QA 按此复核）：
//! - `remote_task_id` 只允许 `null → 值` 或同值：0002 触发器是硬边界；本模块额外
//!   提供**可判定**的结果（[`RemoteTaskOutcome::Conflict`]），让调用方能在覆盖前
//!   记录冲突并停机告警，而不是靠解析约束错误消息；
//! - `submit_state = accepted` 的写入与远端事实**同一语句**完成（收到 ID 立即持久化；
//!   即使租约刚过期也允许把空 `remote_task_id` 补成返回值）；
//! - 未对账（intent/submitting/unknown）每阶段至多一行：0002 的部分唯一索引保证
//!   "同一阶段只允许一个未对账 attempt"，本模块的 `create_intent` 会先释放
//!   未发出请求的陈旧 intent（标记 `failed`）；
//! - **文本列脱敏（统一入口，BUG-009 / OB-9）**：写入 `last_error` 的文本
//!   （[`mark_unknown`]、[`mark_failed`]、[`set_last_error`]）先经
//!   [`crate::redaction::redact_text_urls`]，保证供应商临时地址的 `scheme://…`
//!   形态不落库；裸 host、task_id 与摘要标签原样保留。

use sqlx::{Row, SqliteConnection};

use manual_core::domain::{ProviderAttempt, SubmitState};
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::redaction::redact_text_urls;
use crate::storage::error::StorageError;

/// 新建 attempt（intent）的输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAttempt {
    pub job_id: String,
    pub stage_id: String,
    /// 请求指纹：同一阶段重试时若请求体变化（例如换了预算/模型）应能看出，
    /// 但**不得**用它自动重放付费请求（ADR-006）。
    pub request_hash: String,
}

/// 远端 task ID 的写入结果（冲突必须由调用方记录并停机告警，不得覆盖）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteTaskOutcome {
    /// 首次写入（`null → 值`）。
    Recorded,
    /// 已有相同值（重复观察，幂等）。
    SameAsRecorded,
    /// 已有**不同**值：不写库，调用方必须记录冲突并暂停该分支。
    Conflict { existing: String },
}

/// 静态 SQL（列清单见 [`ATTEMPT_COLUMNS`]；三条查询只差 WHERE/ORDER，分别写成常量，
/// 因为 SQLx 0.9 的 `SqlSafeStr` 在编译期拒绝动态拼接的 SQL 字符串）。
const SELECT_ATTEMPT_BY_ID_SQL: &str = "SELECT id, job_id, stage_id, request_hash, submit_state, remote_task_id, \
        response_id, started_at, last_error, created_at, updated_at \
   FROM provider_attempts WHERE id = ?";

const SELECT_ATTEMPT_LATEST_SQL: &str = "SELECT id, job_id, stage_id, request_hash, submit_state, remote_task_id, \
        response_id, started_at, last_error, created_at, updated_at \
   FROM provider_attempts WHERE stage_id = ? ORDER BY created_at DESC, id DESC LIMIT 1";

const SELECT_ATTEMPT_UNRESOLVED_SQL: &str = "SELECT id, job_id, stage_id, request_hash, submit_state, remote_task_id, \
        response_id, started_at, last_error, created_at, updated_at \
   FROM provider_attempts \
  WHERE stage_id = ? AND submit_state IN ('intent', 'submitting', 'unknown') \
  ORDER BY created_at DESC, id DESC LIMIT 1";

/// 首次 `null → 值` 之外的情况返回 `Conflict`（触发器为最终硬边界）。
pub async fn create_intent(
    conn: &mut SqliteConnection,
    new: NewAttempt,
    now: Timestamp,
) -> Result<ProviderAttempt, StorageError> {
    let attempt = ProviderAttempt {
        id: ids::new_id(),
        job_id: new.job_id,
        stage_id: new.stage_id,
        request_hash: new.request_hash,
        submit_state: SubmitState::Intent,
        remote_task_id: None,
        response_id: None,
        started_at: now,
        last_error: None,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO provider_attempts \
             (id, job_id, stage_id, request_hash, submit_state, remote_task_id, response_id, \
              started_at, last_error, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'intent', NULL, NULL, ?, NULL, ?, ?)",
    )
    .bind(&attempt.id)
    .bind(&attempt.job_id)
    .bind(&attempt.stage_id)
    .bind(&attempt.request_hash)
    .bind(attempt.started_at.as_millis())
    .bind(attempt.created_at.as_millis())
    .bind(attempt.updated_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(attempt)
}

/// 按 id 读取 attempt。
pub async fn get(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<ProviderAttempt>, StorageError> {
    let row = sqlx::query(SELECT_ATTEMPT_BY_ID_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| attempt_from_row(&row)).transpose()
}

/// 阶段最近一次 attempt（恢复判定输入）。
pub async fn latest_for_stage(
    conn: &mut SqliteConnection,
    stage_id: &str,
) -> Result<Option<ProviderAttempt>, StorageError> {
    let row = sqlx::query(SELECT_ATTEMPT_LATEST_SQL)
        .bind(stage_id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| attempt_from_row(&row)).transpose()
}

/// 某 job 的全部 attempt（按创建时间升序；任务详情展示与对账入口用）。
pub async fn list_for_job(
    conn: &mut SqliteConnection,
    job_id: &str,
) -> Result<Vec<ProviderAttempt>, StorageError> {
    let rows = sqlx::query(
        "SELECT id, job_id, stage_id, request_hash, submit_state, remote_task_id, \
                response_id, started_at, last_error, created_at, updated_at \
           FROM provider_attempts WHERE job_id = ? ORDER BY created_at, id",
    )
    .bind(job_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(attempt_from_row).collect()
}

/// 阶段当前的**未对账** attempt（intent/submitting/unknown；部分唯一索引保证至多一行）。
pub async fn unresolved_for_stage(
    conn: &mut SqliteConnection,
    stage_id: &str,
) -> Result<Option<ProviderAttempt>, StorageError> {
    let row = sqlx::query(SELECT_ATTEMPT_UNRESOLVED_SQL)
        .bind(stage_id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| attempt_from_row(&row)).transpose()
}

/// 第 2 步：在当前租约下标记 `submitting`，**之后**才允许发 HTTP POST。
pub async fn mark_submitting(
    conn: &mut SqliteConnection,
    id: &str,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let changed = sqlx::query(
        "UPDATE provider_attempts SET submit_state = 'submitting', updated_at = ? \
          WHERE id = ? AND submit_state = 'intent'",
    )
    .bind(now.as_millis())
    .bind(id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

/// 第 3 步：收到远端 task ID **立即持久化事实观察**（不受租约 epoch 限制）。
///
/// `null → 值` 或同值；不同值 → [`RemoteTaskOutcome::Conflict`]，不覆盖已有 ID。
pub async fn record_remote_task_id(
    conn: &mut SqliteConnection,
    id: &str,
    remote_task_id: &str,
    now: Timestamp,
) -> Result<RemoteTaskOutcome, StorageError> {
    match read_remote_task_id(conn, id).await? {
        // 行不存在：调用方拿到的 attempt id 已失效。
        None => Err(StorageError::NotFound {
            entity: "provider_attempt",
            id: id.to_owned(),
        }),
        // 已有不同值：不写库（触发器也是硬边界），调用方记录冲突并停机告警。
        Some(Some(existing)) if existing != remote_task_id => {
            Ok(RemoteTaskOutcome::Conflict { existing })
        }
        // 已有同值：幂等，同时把状态收敛到 accepted。
        Some(Some(_)) => {
            sqlx::query(
                "UPDATE provider_attempts SET submit_state = 'accepted', updated_at = ? \
                  WHERE id = ? AND remote_task_id = ?",
            )
            .bind(now.as_millis())
            .bind(id)
            .bind(remote_task_id)
            .execute(&mut *conn)
            .await?;
            Ok(RemoteTaskOutcome::SameAsRecorded)
        }
        // 空值 → 首次写入（即使租约刚过期也允许补成返回值）。
        Some(None) => {
            let changed = sqlx::query(
                "UPDATE provider_attempts \
                    SET remote_task_id = ?, submit_state = 'accepted', updated_at = ? \
                  WHERE id = ? AND remote_task_id IS NULL",
            )
            .bind(remote_task_id)
            .bind(now.as_millis())
            .bind(id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
            if changed > 0 {
                return Ok(RemoteTaskOutcome::Recorded);
            }
            // 竞态（本进程内不可能，保险分支）：读回并判定，绝不覆盖。
            match read_remote_task_id(conn, id).await? {
                Some(Some(existing)) if existing != remote_task_id => {
                    Ok(RemoteTaskOutcome::Conflict { existing })
                }
                Some(Some(_)) => Ok(RemoteTaskOutcome::SameAsRecorded),
                Some(None) => Ok(RemoteTaskOutcome::Recorded),
                None => Err(StorageError::NotFound {
                    entity: "provider_attempt",
                    id: id.to_owned(),
                }),
            }
        }
    }
}

/// 读取 attempt 的 `remote_task_id`；`Ok(None)` = 行不存在，`Ok(Some(None))` = 行存在但为空。
async fn read_remote_task_id(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<Option<String>>, StorageError> {
    let row: Option<Option<String>> =
        sqlx::query_scalar("SELECT remote_task_id FROM provider_attempts WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(row)
}

/// 同步链路（说明书 AI）：完整响应已持久化 → `accepted` + `response_id`。
///
/// **`response_id` 不等于可轮询任务**：后续恢复不得据此重新请求或"重取"响应
/// （contracts.md §5）。真正的完成态由调用方在同一短事务里保存结果资产与 usage。
pub async fn record_sync_response(
    conn: &mut SqliteConnection,
    id: &str,
    response_id: Option<&str>,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let changed = sqlx::query(
        "UPDATE provider_attempts \
            SET submit_state = 'accepted', response_id = COALESCE(?, response_id), \
                remote_task_id = NULL, updated_at = ? \
          WHERE id = ? AND submit_state IN ('intent', 'submitting')",
    )
    .bind(response_id)
    .bind(now.as_millis())
    .bind(id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

/// 结果未知（禁止自动重购）：`submitting → unknown`；已 unknown 时幂等。
pub async fn mark_unknown(
    conn: &mut SqliteConnection,
    id: &str,
    reason: &str,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let reason = redact_text_urls(reason);
    let changed = sqlx::query(
        "UPDATE provider_attempts \
            SET submit_state = 'unknown', last_error = ?, updated_at = ? \
          WHERE id = ? AND submit_state <> 'unknown'",
    )
    .bind(&reason)
    .bind(now.as_millis())
    .bind(id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

/// 明确失败（只有可证明未被接受的错误才允许走到这里）：→ `failed`。
pub async fn mark_failed(
    conn: &mut SqliteConnection,
    id: &str,
    reason: &str,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let reason = redact_text_urls(reason);
    let changed = sqlx::query(
        "UPDATE provider_attempts \
            SET submit_state = 'failed', last_error = ?, updated_at = ? \
          WHERE id = ? AND submit_state <> 'failed'",
    )
    .bind(&reason)
    .bind(now.as_millis())
    .bind(id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

/// 更新错误摘要（不改变 submit_state；用于冲突记录等）。
pub async fn set_last_error(
    conn: &mut SqliteConnection,
    id: &str,
    reason: &str,
    now: Timestamp,
) -> Result<(), StorageError> {
    let reason = redact_text_urls(reason);
    sqlx::query("UPDATE provider_attempts SET last_error = ?, updated_at = ? WHERE id = ?")
        .bind(&reason)
        .bind(now.as_millis())
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

fn attempt_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ProviderAttempt, StorageError> {
    let submit_state: String = row.try_get("submit_state")?;
    Ok(ProviderAttempt {
        id: row.try_get("id")?,
        job_id: row.try_get("job_id")?,
        stage_id: row.try_get("stage_id")?,
        request_hash: row.try_get("request_hash")?,
        submit_state: parse_submit_state(&submit_state)?,
        remote_task_id: row.try_get("remote_task_id")?,
        response_id: row.try_get("response_id")?,
        started_at: Timestamp::from_millis(row.try_get("started_at")?),
        last_error: row.try_get("last_error")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
        updated_at: Timestamp::from_millis(row.try_get("updated_at")?),
    })
}

fn parse_submit_state(value: &str) -> Result<SubmitState, StorageError> {
    match value {
        "intent" => Ok(SubmitState::Intent),
        "submitting" => Ok(SubmitState::Submitting),
        "accepted" => Ok(SubmitState::Accepted),
        "unknown" => Ok(SubmitState::Unknown),
        "failed" => Ok(SubmitState::Failed),
        other => Err(StorageError::Database {
            detail: format!("provider_attempts.submit_state 取值未知：{other}"),
        }),
    }
}

const SELECT_ACCEPTED_FOR_JOB_SQL: &str = "SELECT a.id, a.job_id, a.stage_id, a.request_hash, a.submit_state, \
        a.remote_task_id, a.response_id, a.started_at, a.last_error, a.created_at, a.updated_at \
   FROM provider_attempts a JOIN job_stages s ON s.id = a.stage_id \
  WHERE a.job_id = ? AND s.stage_kind = ? AND a.submit_state = 'accepted' \
    AND a.remote_task_id IS NOT NULL \
  ORDER BY a.created_at DESC, a.id DESC LIMIT 1";

/// 同一 job 中某类阶段已接受的远端事实（例如 `tripo_poll` 需要 `tripo_submit` 落库的
/// task ID）。返回 `Ok(None)` 表示还没有可用事实——调用方不得据此重新提交付费请求。
pub async fn latest_accepted_for_job(
    conn: &mut SqliteConnection,
    job_id: &str,
    stage_kind: manual_core::domain::StageKind,
) -> Result<Option<ProviderAttempt>, StorageError> {
    let row = sqlx::query(SELECT_ACCEPTED_FOR_JOB_SQL)
        .bind(job_id)
        .bind(stage_kind.as_str())
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| attempt_from_row(&row)).transpose()
}
