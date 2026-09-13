//! `job_stages` 表（持久执行单元）与 `job_stage_deps`（DAG 边）的仓储原语（T10 / REQ-024）。
//!
//! **边界**：只做持久化与 SQL 层不变量。业务规则（DAG 形状、退避序列、needs_input
//! 缺项、父 job 聚合）来自 [`manual_core::jobs`]；调度循环在 `crate::jobs::executor`。
//!
//! 关键语义（contracts.md §5；QA 按此复核）：
//! - **持久执行单元** = `job + stage_kind + batch_index`（唯一键在 0001 迁移）；
//!   批次各自有 `page_set`/`input_hash`/`result_asset_id`/`usage_json`；
//! - **依赖落库**：[`insert`] 按 core 的 DAG 写入 `job_stage_deps` 边，
//!   领取时用 NOT EXISTS 判定"依赖全部 succeeded"，不用内存 for 循环；
//! - **领取 = 条件更新**：`status IN ('queued','retry_wait','waiting_provider')`
//!   且到期 → `running` 并 `lease_epoch + 1`（原子取得新 epoch）。领取事务用
//!   `BEGIN IMMEDIATE` 拿写锁，容量谓词与领取在同一条语句快照内判定；
//! - **业务推进必须带租约 guard**（`status='running' AND lease_owner=? AND lease_epoch=? AND lease_until > now`）：
//!   租约已过期的 worker 不能推进状态或解锁后续阶段，只能保存事实（[`set_result_fact`]）；
//! - **租约 120s / 20s 续约**由执行器驱动（[`renew_lease`] 只做条件更新）；
//! - **文本列脱敏（统一入口，BUG-009 / OB-9 / ADR-034）**：所有写入 `last_error` /
//!   `needs_input_json` / `usage_json` 的文本都先经 [`crate::redaction`] 的统一入口，
//!   保证任何来源（下载传输错误、供应商错误摘要、恢复/对账注记）落库前都不含
//!   `scheme://…` 形态的临时地址；JSON 列按**逐字符串值**脱敏（句子保留、纯 URL 值
//!   变摘要对象/标签），非 JSON 文本走 `redact_text_urls`；裸 host、task_id、计数与
//!   摘要标签原样保留。

use std::time::Duration;

use sqlx::{Row, SqliteConnection, SqlitePool};

use manual_core::domain::{JobStage, JobStatus, StageKind};
use manual_core::ids;
use manual_core::jobs::{StageDependency, stage_dependency};
use manual_core::timestamps::Timestamp;

use crate::redaction::{JsonStringRedaction, redact_json_text_urls, redact_text_urls};
use crate::storage::error::StorageError;

use super::{parse_job_status, parse_stage_kind};

/// 条件更新宏：`binds` 依次绑定后接租约 guard 的 `(id, owner, epoch, now)`，返回行数。
///
/// 所有业务推进共用同一 guard 形状，避免某条语句漏掉 epoch 判定
/// （contracts.md §5：状态推进必须校验当前 leaseEpoch）。
macro_rules! guarded_advance {
    ($conn:expr, $sql:expr, $guard:expr, $now_millis:expr, $($bind:expr),* $(,)?) => {{
        sqlx::query($sql)
            $(.bind($bind))*
            .bind(&$guard.stage_id)
            .bind(&$guard.owner)
            .bind($guard.epoch)
            .bind($now_millis)
            .execute(&mut *$conn)
            .await?
            .rows_affected()
    }};
}

/// 新建阶段的输入（id/时间戳由仓储生成）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStage {
    pub job_id: String,
    pub stage_kind: StageKind,
    /// 非批处理阶段必须为 0（0001 迁移的 CHECK 兜底）。
    pub batch_index: i64,
    /// JSON 数组文本：本批页号（1-based）；非批处理阶段为 `None`。
    pub page_set_json: Option<String>,
    pub input_hash: String,
    /// 初始状态：入队事务里 `freeze_inputs` 直接 `succeeded`，
    /// 其余阶段通常 `queued`（T11 决定；执行器只消费）。
    pub status: JobStatus,
}

/// 租约 guard：业务状态推进必须携带领取时的 owner + epoch。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseGuard {
    pub stage_id: String,
    pub owner: String,
    pub epoch: i64,
}

/// 一次业务状态推进的载荷（每个变体一条**静态** SQL，无拼接）。
#[derive(Debug, Clone, PartialEq)]
pub enum StageAdvance {
    /// 产物校验成功（`result_asset_id`/`usage_json` 为 `None` 时保留既有值）。
    Succeeded {
        result_asset_id: Option<String>,
        usage_json: Option<String>,
    },
    /// 已提交远端 ID / 远端仍在进行：`waiting_provider` + `next_run_at`（轮询节奏）。
    WaitingProvider { next_run_at: Timestamp },
    /// 安全临时失败：`retry_wait`（`attempt_count + 1`）+ 原因 + 下次运行时间。
    RetryWait {
        next_run_at: Timestamp,
        last_error: String,
    },
    /// 资料/schema 不足：`needs_input` + 可行动缺项 JSON。
    NeedsInput {
        needs_input_json: String,
        last_error: String,
    },
    /// 付费创建结果未知：`submission_unknown`（暂停该分支后续购买）。
    SubmissionUnknown { last_error: String },
    /// 不可恢复失败：`failed`。
    Failed { last_error: String },
    /// 延后重试：`queued` + `next_run_at`，**不消耗安全重试额度**
    /// （用于"处理器尚未接入"这类环境问题，不能算作阶段失败）。
    Defer {
        next_run_at: Timestamp,
        last_error: String,
    },
    /// 重新入队（恢复用：租约过期但没有未决事实，可安全重领）。
    Requeue,
    /// 取消（恢复时发现 job 已取消 / 取消流程使用）。
    Cancel,
}

/// 领取参数。
#[derive(Debug, Clone)]
pub struct ClaimParams {
    /// worker 身份（写入 `lease_owner`，用于诊断；正确性由 epoch 保证）。
    pub owner: String,
    pub now: Timestamp,
    /// 租约时长（默认 120s；可配置降低，见架构 §6）。
    pub lease: Duration,
    /// 全局远端生成并发上限（默认 2）。
    pub remote_generation_limit: u32,
    /// 说明书批次并发上限（默认 2）。
    pub manual_ai_batch_limit: u32,
}

/// 插入阶段行并按 DAG 写入依赖边（同一个事务；调用方负责事务边界）。
///
/// 依赖规则来自 [`manual_core::jobs::stage_dependency`]：
/// - `Fixed(kinds)` → 指向该 job 中这些 kind 的**全部**已存在阶段；
/// - `AllBatchesOf(kind)` → 指向该 job 中该 kind 的全部批次
///   （因此 `manual_merge` 必须在批次创建之后插入，T11/T15 的建单顺序）。
pub async fn insert(
    conn: &mut SqliteConnection,
    new: NewStage,
    now: Timestamp,
) -> Result<JobStage, StorageError> {
    let stage = JobStage {
        id: ids::new_id(),
        job_id: new.job_id,
        stage_kind: new.stage_kind,
        batch_index: new.batch_index,
        page_set: match &new.page_set_json {
            Some(text) => {
                Some(
                    serde_json::from_str(text).map_err(|error| StorageError::Database {
                        detail: format!("page_set 不是合法 JSON 数组：{error}"),
                    })?,
                )
            }
            None => None,
        },
        input_hash: new.input_hash,
        result_asset_id: None,
        usage_json: None,
        status: new.status,
        lease_owner: None,
        lease_epoch: 0,
        lease_until: None,
        next_run_at: None,
        attempt_count: 0,
        poll_count: 0,
        last_error: None,
        needs_input_json: None,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO job_stages \
             (id, job_id, stage_kind, batch_index, page_set, input_hash, result_asset_id, usage_json, \
              status, lease_owner, lease_epoch, lease_until, next_run_at, attempt_count, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, ?, NULL, 0, NULL, NULL, 0, ?, ?)",
    )
    .bind(&stage.id)
    .bind(&stage.job_id)
    .bind(stage.stage_kind.as_str())
    .bind(stage.batch_index)
    .bind(&new.page_set_json)
    .bind(&stage.input_hash)
    .bind(stage.status.as_str())
    .bind(stage.created_at.as_millis())
    .bind(stage.updated_at.as_millis())
    .execute(&mut *conn)
    .await?;

    link_dependencies(conn, &stage, now).await?;
    Ok(stage)
}

/// 按 DAG 写入依赖边（幂等：主键冲突忽略）。
async fn link_dependencies(
    conn: &mut SqliteConnection,
    stage: &JobStage,
    now: Timestamp,
) -> Result<(), StorageError> {
    let rule = stage_dependency(stage.stage_kind);
    let kinds: Vec<StageKind> = match rule {
        StageDependency::None => return Ok(()),
        StageDependency::Fixed(kinds) => kinds.to_vec(),
        StageDependency::AllBatchesOf(kind) => vec![kind],
    };
    for kind in kinds {
        let rows = sqlx::query("SELECT id FROM job_stages WHERE job_id = ? AND stage_kind = ?")
            .bind(&stage.job_id)
            .bind(kind.as_str())
            .fetch_all(&mut *conn)
            .await?;
        for row in rows {
            let dependency_id: String = row.try_get("id")?;
            sqlx::query(
                "INSERT INTO job_stage_deps (stage_id, depends_on_stage_id, created_at) \
                 VALUES (?, ?, ?) ON CONFLICT (stage_id, depends_on_stage_id) DO NOTHING",
            )
            .bind(&stage.id)
            .bind(&dependency_id)
            .bind(now.as_millis())
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// 按 id 读取阶段；不存在返回 `Ok(None)`。
pub async fn get(conn: &mut SqliteConnection, id: &str) -> Result<Option<JobStage>, StorageError> {
    let row = sqlx::query(SELECT_STAGE_BY_ID_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| stage_from_row(&row)).transpose()
}

/// 列出 job 的全部阶段（稳定顺序：创建顺序 + kind + 批次）。
pub async fn list_for_job(
    conn: &mut SqliteConnection,
    job_id: &str,
) -> Result<Vec<JobStage>, StorageError> {
    let rows = sqlx::query(
        "SELECT id, job_id, stage_kind, batch_index, page_set, input_hash, result_asset_id, usage_json, \
                status, lease_owner, lease_epoch, lease_until, next_run_at, attempt_count, poll_count, \
                last_error, needs_input_json, created_at, updated_at \
           FROM job_stages WHERE job_id = ? ORDER BY created_at, stage_kind, batch_index",
    )
    .bind(job_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(stage_from_row).collect()
}

/// job 的阶段状态集合（父 job 聚合用）。
pub async fn statuses_for_job(
    conn: &mut SqliteConnection,
    job_id: &str,
) -> Result<Vec<JobStatus>, StorageError> {
    let rows = sqlx::query("SELECT status FROM job_stages WHERE job_id = ? ORDER BY id")
        .bind(job_id)
        .fetch_all(&mut *conn)
        .await?;
    rows.iter()
        .map(|row| parse_job_status(row.try_get::<String, _>("status")?.as_str()))
        .collect()
}

/// 某阶段依赖的 stage id 列表（测试与诊断用）。
pub async fn dependencies_of(
    conn: &mut SqliteConnection,
    stage_id: &str,
) -> Result<Vec<String>, StorageError> {
    let rows = sqlx::query(
        "SELECT depends_on_stage_id FROM job_stage_deps WHERE stage_id = ? ORDER BY depends_on_stage_id",
    )
    .bind(stage_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| row.try_get("depends_on_stage_id").map_err(Into::into))
        .collect()
}

/// 领取下一个可执行阶段（SQL 条件更新，原子取得新 `lease_epoch`）。
///
/// 领取条件（contracts.md §5）：
/// - 状态 `queued` / 到期 `retry_wait` / 到期 `waiting_provider`（轮询阶段）；
/// - job 未被取消（终态 job 不阻止其他分支继续跑完，`failed` 也不阻止）；
/// - 依赖阶段全部 `succeeded`（`job_stage_deps` NOT EXISTS 判定）；
/// - 并发分组未达上限（说明书批次 / 远端生成链）。
///
/// 用 `BEGIN IMMEDIATE` 持写锁再读再写：容量谓词与候选选择在同一快照内判定，
/// 两个 worker 同时领取同一行时条件更新只有一个成功（另一个得到 0 行重试）。
pub async fn claim_next(
    pool: &SqlitePool,
    params: &ClaimParams,
) -> Result<Option<JobStage>, StorageError> {
    let lease_until = params
        .now
        .checked_add_millis(params.lease.as_millis() as i64)
        .ok_or_else(|| StorageError::Database {
            detail: "租约到期时间溢出".to_owned(),
        })?;

    for _attempt in 0..4 {
        let mut tx = crate::storage::begin_write_pool(pool).await?;
        let candidate = sqlx::query(CLAIM_CANDIDATE_SQL)
            .bind(params.now.as_millis())
            .bind(i64::from(params.manual_ai_batch_limit))
            .bind(i64::from(params.remote_generation_limit))
            .fetch_optional(&mut *tx)
            .await?;
        let Some(row) = candidate else {
            tx.commit().await?;
            return Ok(None);
        };
        let stage_id: String = row.try_get("id")?;
        let expected_status: String = row.try_get("status")?;
        let expected_epoch: i64 = row.try_get("lease_epoch")?;

        let changed = sqlx::query(
            "UPDATE job_stages \
                SET status = 'running', lease_owner = ?, lease_epoch = lease_epoch + 1, \
                    lease_until = ?, next_run_at = NULL, updated_at = ? \
              WHERE id = ? AND status = ? AND lease_epoch = ?",
        )
        .bind(&params.owner)
        .bind(lease_until.as_millis())
        .bind(params.now.as_millis())
        .bind(&stage_id)
        .bind(&expected_status)
        .bind(expected_epoch)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if changed == 0 {
            // 极小概率的竞争窗口：回滚后重试（不产生错误状态）。
            tx.rollback().await?;
            continue;
        }
        let claimed = sqlx::query(SELECT_STAGE_BY_ID_SQL)
            .bind(&stage_id)
            .fetch_one(&mut *tx)
            .await?;
        let stage = stage_from_row(&claimed)?;
        tx.commit().await?;
        return Ok(Some(stage));
    }
    Err(StorageError::Database {
        detail: "领取阶段连续竞争失败（4 次）；本 tick 放弃，不修改任何状态".to_owned(),
    })
}

/// 续约：仅当仍是本 worker 的当前 epoch 且未过期时延长 `lease_until`。
///
/// 返回 `false` 表示租约已被接管（或阶段已推进）：调用方必须停止业务推进，
/// 只能保存事实（[`set_result_fact`]）。
pub async fn renew_lease(
    conn: &mut SqliteConnection,
    guard: &LeaseGuard,
    now: Timestamp,
    lease: Duration,
) -> Result<bool, StorageError> {
    let lease_until = now
        .checked_add_millis(lease.as_millis() as i64)
        .ok_or_else(|| StorageError::Database {
            detail: "租约到期时间溢出".to_owned(),
        })?;
    let changed = sqlx::query(
        "UPDATE job_stages SET lease_until = ?, updated_at = ? \
          WHERE id = ? AND status = 'running' AND lease_owner = ? AND lease_epoch = ? \
            AND lease_until IS NOT NULL AND lease_until > ?",
    )
    .bind(lease_until.as_millis())
    .bind(now.as_millis())
    .bind(&guard.stage_id)
    .bind(&guard.owner)
    .bind(guard.epoch)
    .bind(now.as_millis())
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

/// 带租约 guard 的业务状态推进（contracts.md §5 提交窗口第 4 步）。
///
/// 返回 `false` = 被拒绝（租约过期/已被接管/状态不是 `running`）：调用方记录
/// "推进被拒绝"，不得重试推进，也不得解锁后续阶段。
pub async fn advance(
    conn: &mut SqliteConnection,
    guard: &LeaseGuard,
    now: Timestamp,
    advance: &StageAdvance,
) -> Result<bool, StorageError> {
    let now_millis = now.as_millis();
    let changed = match advance {
        StageAdvance::Succeeded {
            result_asset_id,
            usage_json,
        } => {
            // 结果事实（usage_json）同样走统一入口（BUG-012：提供方文本里的签名 URL
            // 曾原样落库）。JSON 列逐字符串值脱敏：句子保留、纯 URL 值变摘要对象。
            let usage_json = usage_json
                .as_deref()
                .map(|text| redact_json_text_urls(text, JsonStringRedaction::SummaryObject).0);
            guarded_advance!(
                conn,
                "UPDATE job_stages \
                    SET status = 'succeeded', \
                        result_asset_id = COALESCE(?, result_asset_id), \
                        usage_json = COALESCE(?, usage_json), \
                        lease_owner = NULL, lease_until = NULL, next_run_at = NULL, last_error = NULL, \
                        updated_at = ? \
                  WHERE id = ? AND status = 'running' AND lease_owner = ? AND lease_epoch = ? AND lease_until > ?",
                guard,
                now_millis,
                result_asset_id.as_deref(),
                usage_json.as_deref(),
                now_millis,
            )
        }
        StageAdvance::WaitingProvider { next_run_at } => guarded_advance!(
            conn,
            "UPDATE job_stages \
                SET status = 'waiting_provider', poll_count = poll_count + 1, next_run_at = ?, \
                    lease_owner = NULL, lease_until = NULL, last_error = NULL, updated_at = ? \
              WHERE id = ? AND status = 'running' AND lease_owner = ? AND lease_epoch = ? AND lease_until > ?",
            guard,
            now_millis,
            next_run_at.as_millis(),
            now_millis,
        ),
        StageAdvance::RetryWait {
            next_run_at,
            last_error,
        } => {
            let last_error = redact_text_urls(last_error);
            guarded_advance!(
                conn,
                "UPDATE job_stages \
                    SET status = 'retry_wait', attempt_count = attempt_count + 1, next_run_at = ?, \
                        last_error = ?, lease_owner = NULL, lease_until = NULL, updated_at = ? \
                  WHERE id = ? AND status = 'running' AND lease_owner = ? AND lease_epoch = ? AND lease_until > ?",
                guard,
                now_millis,
                next_run_at.as_millis(),
                last_error.as_str(),
                now_millis,
            )
        }
        StageAdvance::NeedsInput {
            needs_input_json,
            last_error,
        } => {
            // JSON 列：逐字符串值脱敏且**保持字符串类型**——`message` 是可行动说明，
            // 不得整串变摘要对象（ADR-034；BUG-011 的写入侧同规则）。
            let needs_input_json =
                redact_json_text_urls(needs_input_json, JsonStringRedaction::KeepString).0;
            let last_error = redact_text_urls(last_error);
            guarded_advance!(
                conn,
                "UPDATE job_stages \
                    SET status = 'needs_input', needs_input_json = ?, last_error = ?, next_run_at = NULL, \
                        lease_owner = NULL, lease_until = NULL, updated_at = ? \
                  WHERE id = ? AND status = 'running' AND lease_owner = ? AND lease_epoch = ? AND lease_until > ?",
                guard,
                now_millis,
                needs_input_json.as_str(),
                last_error.as_str(),
                now_millis,
            )
        }
        StageAdvance::SubmissionUnknown { last_error } => {
            let last_error = redact_text_urls(last_error);
            guarded_advance!(
                conn,
                "UPDATE job_stages \
                    SET status = 'submission_unknown', last_error = ?, next_run_at = NULL, \
                        lease_owner = NULL, lease_until = NULL, updated_at = ? \
                  WHERE id = ? AND status = 'running' AND lease_owner = ? AND lease_epoch = ? AND lease_until > ?",
                guard,
                now_millis,
                last_error.as_str(),
                now_millis,
            )
        }
        StageAdvance::Failed { last_error } => {
            let last_error = redact_text_urls(last_error);
            guarded_advance!(
                conn,
                "UPDATE job_stages \
                    SET status = 'failed', last_error = ?, next_run_at = NULL, \
                        lease_owner = NULL, lease_until = NULL, updated_at = ? \
                  WHERE id = ? AND status = 'running' AND lease_owner = ? AND lease_epoch = ? AND lease_until > ?",
                guard,
                now_millis,
                last_error.as_str(),
                now_millis,
            )
        }
        StageAdvance::Defer {
            next_run_at,
            last_error,
        } => {
            let last_error = redact_text_urls(last_error);
            guarded_advance!(
                conn,
                "UPDATE job_stages \
                    SET status = 'queued', next_run_at = ?, last_error = ?, \
                        lease_owner = NULL, lease_until = NULL, updated_at = ? \
                  WHERE id = ? AND status = 'running' AND lease_owner = ? AND lease_epoch = ? AND lease_until > ?",
                guard,
                now_millis,
                next_run_at.as_millis(),
                last_error.as_str(),
                now_millis,
            )
        }
        StageAdvance::Requeue => guarded_advance!(
            conn,
            "UPDATE job_stages \
                SET status = 'queued', next_run_at = NULL, lease_owner = NULL, lease_until = NULL, updated_at = ? \
              WHERE id = ? AND status = 'running' AND lease_owner = ? AND lease_epoch = ? AND lease_until > ?",
            guard,
            now_millis,
            now_millis,
        ),
        StageAdvance::Cancel => guarded_advance!(
            conn,
            "UPDATE job_stages \
                SET status = 'cancelled', next_run_at = NULL, lease_owner = NULL, lease_until = NULL, updated_at = ? \
              WHERE id = ? AND status = 'running' AND lease_owner = ? AND lease_epoch = ? AND lease_until > ?",
            guard,
            now_millis,
            now_millis,
        ),
    };
    Ok(changed > 0)
}

/// 保存**不可变结果事实**（`result_asset_id` / `usage_json`）。
///
/// 不加租约 guard：合同明确"过期 worker 可保存不可变 receipt/结果事实，
/// 但不得推进业务状态"（contracts.md §5 提交窗口第 3 步与 Manual AI 分支）。
/// 业务推进（`status`）仍必须走 [`advance`]。
///
/// `usage_json` 写入前经统一脱敏入口（BUG-012）：提供方文本（`errorSummary` 等）
/// 里的临时/签名 URL 不得落库；JSON 结构与其它事实字段原样保留。
pub async fn set_result_fact(
    conn: &mut SqliteConnection,
    stage_id: &str,
    result_asset_id: Option<&str>,
    usage_json: Option<&str>,
    now: Timestamp,
) -> Result<(), StorageError> {
    let usage_json =
        usage_json.map(|text| redact_json_text_urls(text, JsonStringRedaction::SummaryObject).0);
    sqlx::query(
        "UPDATE job_stages \
            SET result_asset_id = COALESCE(?, result_asset_id), \
                usage_json = COALESCE(?, usage_json), updated_at = ? \
          WHERE id = ?",
    )
    .bind(result_asset_id)
    .bind(usage_json)
    .bind(now.as_millis())
    .bind(stage_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// 清理陈旧的结果事实（`result_asset_id` / `usage_json` 置 `NULL`）。
///
/// 使用场景（T14）：同步批次被**显式重新授权重算**（`needs_input` → 重新入队）时，
/// 在新一次付费请求发出前清掉上一次的结果引用，避免"新请求未持久化完整响应"时
/// 恢复程序把旧结果（例如上一次的拒答诊断）当成已完成结果补推进。
/// 旧结果资产/诊断 blob 仍保留在库与 data-dir（不删除事实内容，只解除本阶段的引用）。
///
/// 属于"事实"写入（不带租约 guard），调用方必须已经是该阶段的当前租约持有者
/// （处理器在领取后的执行路径中调用；成功推进过的阶段不会被重新领取）。
pub async fn reset_result_fact(
    pool: &SqlitePool,
    stage_id: &str,
    now: Timestamp,
) -> Result<(), StorageError> {
    let mut conn = pool.acquire().await?;
    sqlx::query(
        "UPDATE job_stages SET result_asset_id = NULL, usage_json = NULL, updated_at = ? \
          WHERE id = ? AND status = 'running'",
    )
    .bind(now.as_millis())
    .bind(stage_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// 取消 job 的未提交阶段（REQ-026 / contracts.md §5 取消行）。
///
/// 规则：`queued` / `retry_wait` / `needs_input` 一律取消；`running` 只在
/// **没有已接受 attempt**（即没有可证明的远端事实）时取消；`waiting_provider`
/// 与 `submission_unknown` 保留（供应商侧可能已在计费，收尾/对账属 T15）。
pub async fn cancel_unsubmitted_for_job(
    conn: &mut SqliteConnection,
    job_id: &str,
    now: Timestamp,
) -> Result<u64, StorageError> {
    let changed = sqlx::query(
        "UPDATE job_stages \
            SET status = 'cancelled', lease_owner = NULL, lease_until = NULL, next_run_at = NULL, updated_at = ? \
          WHERE job_id = ? \
            AND (status IN ('queued', 'retry_wait', 'needs_input') \
                 OR (status = 'running' AND NOT EXISTS ( \
                       SELECT 1 FROM provider_attempts a \
                        WHERE a.stage_id = job_stages.id AND a.submit_state = 'accepted')))",
    )
    .bind(now.as_millis())
    .bind(job_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed)
}

/// 可重试的阶段状态（T15 / REQ-026）：只有**已定性**的失败/缺项才允许人工重试。
///
/// `submission_unknown` **不在其中**：付费结果未经对账不得作为重试入口
/// （contracts.md §5「未知提交不得从此盲重试」）；`succeeded`/`cancelled`/`queued` 等
/// 也不可重试（已完成、已停止或在途）。
pub const fn retryable_stage_status(status: JobStatus) -> bool {
    matches!(status, JobStatus::Failed | JobStatus::NeedsInput)
}

/// 手动重试：把可重试阶段拉回 `queued`（人工授权的新一次执行）。
///
/// 语义（T15 / AC-040）：
/// - 只接受 `failed` / `needs_input`（其余状态不改动，返回 `false`）；
/// - 重置 `attempt_count`（手动重试是新的授权，不受上一次退避额度限制）
///   与 `needs_input_json`，清空租约与 `next_run_at`；
/// - **不**清 `result_asset_id`/`usage_json`：那是"发生过什么"的事实，
///   付费阶段在发出新请求前由处理器自己基于新鲜授权清理（见 T14 的
///   [`reset_result_fact`]）；恢复矩阵对本阶段的旧事实判定见 P3-1 修正。
pub async fn reset_for_retry(
    conn: &mut SqliteConnection,
    stage_id: &str,
    reason: &str,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let reason = redact_text_urls(reason);
    let changed = sqlx::query(
        "UPDATE job_stages \
            SET status = 'queued', attempt_count = 0, next_run_at = NULL, \
                lease_owner = NULL, lease_until = NULL, needs_input_json = NULL, \
                last_error = ?, updated_at = ? \
          WHERE id = ? AND status IN ('failed', 'needs_input')",
    )
    .bind(&reason)
    .bind(now.as_millis())
    .bind(stage_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

/// 重新排队**已成功但依赖本次重试阶段产物**的下游阶段（传递闭包）。
///
/// 场景（T15）：知识分支的批次/合并被重试后，先前为"部分草稿"组装过的
/// `assemble_draft` 已 `succeeded`——它的产物（草稿内容）基于旧输入，必须重算。
/// 规则：
/// - 只动**依赖闭包内**的 `succeeded` 阶段（不碰在途/阻塞阶段，也不碰无关分支——
///   "仅重跑指定可重试阶段，已完成成果保留"）；
/// - 清空它们的 `result_asset_id`/`usage_json`：旧结果不再是"可补推进的事实"，
///   避免恢复矩阵把它们当成已完成结果补推进（与 T14 的 `reset_result_fact` 同因）；
///   旧资产/诊断仍在库与 data-dir（只解除引用，不删除内容）。
pub async fn requeue_succeeded_dependents(
    conn: &mut SqliteConnection,
    stage_id: &str,
    reason: &str,
    now: Timestamp,
) -> Result<u64, StorageError> {
    let reason = redact_text_urls(reason);
    let changed = sqlx::query(
        "WITH RECURSIVE dependents(id) AS ( \
             SELECT stage_id FROM job_stage_deps WHERE depends_on_stage_id = ? \
             UNION \
             SELECT d.stage_id FROM job_stage_deps d JOIN dependents x ON d.depends_on_stage_id = x.id \
         ) \
         UPDATE job_stages \
            SET status = 'queued', next_run_at = NULL, lease_owner = NULL, lease_until = NULL, \
                result_asset_id = NULL, usage_json = NULL, last_error = ?, updated_at = ? \
          WHERE id IN (SELECT id FROM dependents) AND status = 'succeeded'",
    )
    .bind(stage_id)
    .bind(&reason)
    .bind(now.as_millis())
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed)
}

/// 对账后的阶段收敛（T15 / REQ-025）：`submission_unknown` → `queued` 或 `needs_input`。
///
/// - `queued`：管理员已给出可继续的事实（例如 `attachRemoteTask` 附加了查询验证过的
///   远端任务）且 job 未取消 → 执行器按既有恢复规则继续（不重新购买）；
/// - `needs_input`：只记录结论（例如 `recordNoTask`、或 job 已取消）→ 等人工显式动作，
///   不自动继续；
/// - 只接受 `submission_unknown`（其余状态返回 `false`，调用方不得静默继续）。
pub async fn apply_reconcile_resolution(
    conn: &mut SqliteConnection,
    stage_id: &str,
    resolve_to: JobStatus,
    note: &str,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let note = redact_text_urls(note);
    let needs_input_json = if resolve_to == JobStatus::NeedsInput {
        serde_json::to_string(&[MissingItemJson {
            code: "reconcile_resolved".to_owned(),
            message: note.clone(),
        }])
        .unwrap_or_else(|_| "[]".to_owned())
    } else {
        String::new()
    };
    let changed = sqlx::query(
        "UPDATE job_stages \
            SET status = ?, next_run_at = NULL, lease_owner = NULL, lease_until = NULL, \
                needs_input_json = CASE WHEN ? = 'needs_input' THEN ? ELSE NULL END, \
                last_error = ?, updated_at = ? \
          WHERE id = ? AND status = 'submission_unknown'",
    )
    .bind(resolve_to.as_str())
    .bind(resolve_to.as_str())
    .bind(&needs_input_json)
    .bind(&note)
    .bind(now.as_millis())
    .bind(stage_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

/// `needs_input_json` 的 JSON 形状（与 `crate::jobs::MissingItem` 的线上形状一致，
/// 避免仓储层依赖 jobs 模块导致循环引用）。
#[derive(serde::Serialize)]
struct MissingItemJson {
    code: String,
    message: String,
}

/// 租约已过期的 `running` 阶段（恢复扫描输入）。
pub async fn expired_running(
    pool: &SqlitePool,
    now: Timestamp,
) -> Result<Vec<JobStage>, StorageError> {
    let rows = sqlx::query(
        "SELECT id, job_id, stage_kind, batch_index, page_set, input_hash, result_asset_id, usage_json, \
                status, lease_owner, lease_epoch, lease_until, next_run_at, attempt_count, poll_count, \
                last_error, needs_input_json, created_at, updated_at \
           FROM job_stages \
          WHERE status = 'running' AND lease_until IS NOT NULL AND lease_until <= ? \
          ORDER BY updated_at",
    )
    .bind(now.as_millis())
    .fetch_all(pool)
    .await?;
    rows.iter().map(stage_from_row).collect()
}

/// 接管一个租约已过期的阶段：`lease_epoch + 1`，写入恢复方 owner 与新租约。
///
/// 这是恢复路径的"领取"：只有成功接管（返回 `true`）的恢复方才能随后推进该阶段；
/// 旧 worker 的推进仍会被 epoch guard 拒绝。
pub async fn take_over_expired(
    conn: &mut SqliteConnection,
    stage_id: &str,
    expected_epoch: i64,
    owner: &str,
    now: Timestamp,
    lease: Duration,
) -> Result<bool, StorageError> {
    let lease_until = now
        .checked_add_millis(lease.as_millis() as i64)
        .ok_or_else(|| StorageError::Database {
            detail: "租约到期时间溢出".to_owned(),
        })?;
    let changed = sqlx::query(
        "UPDATE job_stages \
            SET lease_epoch = lease_epoch + 1, lease_owner = ?, lease_until = ?, updated_at = ? \
          WHERE id = ? AND status = 'running' AND lease_epoch = ? \
            AND lease_until IS NOT NULL AND lease_until <= ?",
    )
    .bind(owner)
    .bind(lease_until.as_millis())
    .bind(now.as_millis())
    .bind(stage_id)
    .bind(expected_epoch)
    .bind(now.as_millis())
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

/// 稳定的列清单（与 [`stage_from_row`] 对应；静态 SQL，无拼接）。
const SELECT_STAGE_BY_ID_SQL: &str = "SELECT id, job_id, stage_kind, batch_index, page_set, input_hash, \
        result_asset_id, usage_json, status, lease_owner, lease_epoch, lease_until, next_run_at, \
        attempt_count, poll_count, last_error, needs_input_json, created_at, updated_at \
   FROM job_stages WHERE id = ?";

/// 领取候选（静态 SQL；容量谓词内联，`?` 依次为 now / 批次上限 / 远端上限）。
///
/// 依赖解锁规则（T15 起对 `assemble_draft` 放宽，理由见 implementation §T15）：
/// - 普通阶段：依赖必须**全部 `succeeded`**（contracts.md §5）；
/// - `assemble_draft`：依赖处于**已定性状态**即可（`succeeded` / `failed` /
///   `needs_input` / `submission_unknown` / `cancelled`）——某个分支头被阻塞时，
///   组装仍要产出**部分草稿**并标明缺项（REQ-030「部分成功可展示」）。
///   仍在途（`queued`/`retry_wait`/`running`/`waiting_provider`）的依赖依旧阻塞组装：
///   T10 已验收语义（分支上游失败 → `manual_merge` 保持 `queued` → 组装保持 `queued`）
///   不变；上游被人工重试后，[`requeue_succeeded_dependents`] 会把已完成的组装拉回队列。
const CLAIM_CANDIDATE_SQL: &str = "SELECT s.id, s.status, s.lease_epoch \
     FROM job_stages s \
     JOIN jobs j ON j.id = s.job_id \
    WHERE s.status IN ('queued', 'retry_wait', 'waiting_provider') \
      AND (s.next_run_at IS NULL OR s.next_run_at <= ?) \
      AND j.status <> 'cancelled' \
      AND NOT EXISTS ( \
            SELECT 1 FROM job_stage_deps d \
              JOIN job_stages dep ON dep.id = d.depends_on_stage_id \
             WHERE d.stage_id = s.id \
               AND CASE WHEN s.stage_kind = 'assemble_draft' THEN \
                     dep.status IN ('queued', 'retry_wait', 'running', 'waiting_provider') \
                   ELSE dep.status <> 'succeeded' \
                   END) \
      AND (CASE \
             WHEN s.stage_kind = 'manual_extract' THEN \
               (SELECT COUNT(*) FROM job_stages r \
                 WHERE r.status = 'running' AND r.stage_kind = 'manual_extract') < ? \
             WHEN s.stage_kind IN ('tripo_upload', 'tripo_submit', 'tripo_poll', 'model_download', 'model_validate') THEN \
               (SELECT COUNT(*) FROM job_stages r \
                 WHERE r.status = 'running' \
                   AND r.stage_kind IN ('tripo_upload', 'tripo_submit', 'tripo_poll', 'model_download', 'model_validate')) < ? \
             ELSE 1 \
           END) \
    ORDER BY COALESCE(s.next_run_at, 0), s.created_at, s.id \
    LIMIT 1";

fn json_column<T: serde::de::DeserializeOwned>(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<T>, StorageError> {
    let text: Option<String> = row.try_get(column)?;
    match text {
        Some(text) => {
            serde_json::from_str(&text)
                .map(Some)
                .map_err(|error| StorageError::Database {
                    detail: format!("列 {column} 不是合法 JSON：{error}"),
                })
        }
        None => Ok(None),
    }
}

fn stage_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<JobStage, StorageError> {
    let lease_until: Option<i64> = row.try_get("lease_until")?;
    let next_run_at: Option<i64> = row.try_get("next_run_at")?;
    Ok(JobStage {
        id: row.try_get("id")?,
        job_id: row.try_get("job_id")?,
        stage_kind: parse_stage_kind(row.try_get::<String, _>("stage_kind")?.as_str())?,
        batch_index: row.try_get("batch_index")?,
        page_set: json_column(row, "page_set")?,
        input_hash: row.try_get("input_hash")?,
        result_asset_id: row.try_get("result_asset_id")?,
        usage_json: json_column(row, "usage_json")?,
        status: parse_job_status(row.try_get::<String, _>("status")?.as_str())?,
        lease_owner: row.try_get("lease_owner")?,
        lease_epoch: row.try_get("lease_epoch")?,
        lease_until: lease_until.map(Timestamp::from_millis),
        next_run_at: next_run_at.map(Timestamp::from_millis),
        attempt_count: row.try_get("attempt_count")?,
        poll_count: row.try_get("poll_count")?,
        last_error: row.try_get("last_error")?,
        needs_input_json: json_column(row, "needs_input_json")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
        updated_at: Timestamp::from_millis(row.try_get("updated_at")?),
    })
}
