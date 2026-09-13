//! 付费提交窗口（contracts.md §5「提交窗口必须按以下顺序测试」；T10）。
//!
//! 五步顺序（本模块把这些步骤变成不可绕过的 API）：
//!
//! 1. **持久化 attempt intent**（费用预留属 T11；本模块只保证"同一阶段只允许一个
//!    未对账 attempt"的 attempt 侧）→ [`SubmissionWindow::begin_intent`]；
//! 2. **在当前租约下标记 submitting**，然后才允许发 HTTP POST →
//!    [`SubmissionWindow::mark_submitting`]；
//! 3. **收到 ID 立即持久化事实观察**：即使租约刚过期，也允许把该 attempt 的空
//!    `remote_task_id` 补成返回值；已有不同 ID → 记录冲突并停机告警，**不覆盖**
//!    → [`SubmissionWindow::record_remote_task_id`]；
//! 4. **业务状态推进另用当前 leaseEpoch 条件更新**（执行器负责；过期 worker 不能
//!    解锁后续阶段）；
//! 5. **恢复语义**：Tripo `submitting` 且无 task ID 一律 unknown、有 ID 继续查；
//!    同步 Manual AI 不套用该轮询规则 —— 已发请求却没有持久化完整响应即
//!    `submission_unknown`，`response_id` 不假定可轮询（见 `jobs::recover`）。
//!
//! 事实写入（intent/submitting/receipt/结果）**不带**租约 guard：它们是"发生过什么"
//! 的记录，不是"接下来做什么"的决定；状态推进才需要 guard。

use serde_json::Value;
use sqlx::SqlitePool;

use manual_core::domain::ProviderAttempt;
use manual_core::timestamps::Timestamp;

use crate::storage::StorageError;
use crate::storage::repo;

use super::JobError;

/// 远端 task ID 的观察结果（冲突必须被记录并停机告警，不得覆盖已有值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteTaskObservation {
    /// 首次写入（`null → 值`）。
    Recorded,
    /// 已记录同值（重复观察，幂等）。
    SameAsRecorded,
    /// 已记录**不同**值：窗口已改为 `submission_unknown` 并写入审计；
    /// 处理器必须按未知结果处理，不得假装成功。
    Conflict { existing: String },
}

/// 一次付费提交窗口（每个阶段一次执行创建一个）。
///
/// 生命周期：`begin_intent` → `mark_submitting` → 发请求 → 观察事实
/// （`record_remote_task_id` 或 `record_sync_response`）→ 执行器推进业务状态。
pub struct SubmissionWindow {
    pool: SqlitePool,
    job_id: String,
    stage_id: String,
    /// worker 身份：断点注入按 owner 分区（同一进程多个执行器互不干扰）。
    owner: String,
    now: Timestamp,
    attempt_id: Option<String>,
    conflict: Option<String>,
}

impl SubmissionWindow {
    pub fn new(
        pool: SqlitePool,
        job_id: String,
        stage_id: String,
        owner: impl Into<String>,
        now: Timestamp,
    ) -> Self {
        Self {
            pool,
            job_id,
            stage_id,
            owner: owner.into(),
            now,
            attempt_id: None,
            conflict: None,
        }
    }

    /// 本窗口所属的 worker 身份（日志与测试断点按 owner 分区）。
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// 本窗口对应的阶段 / job（处理器日志与诊断用）。
    pub fn stage_id(&self) -> &str {
        &self.stage_id
    }

    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    /// 绑定到一个**已存在**的 attempt（恢复路径：处理器继续用同一 attempt 观察事实）。
    ///
    /// 用于"有 task ID 继续查"的场景：窗口不新建 intent，而是在同一 attempt 上
    /// 重新观察远端事实（不同 ID 仍按冲突处理，不覆盖）。
    pub fn bind_attempt(&mut self, attempt_id: impl Into<String>) {
        self.attempt_id = Some(attempt_id.into());
    }

    /// 当前 attempt id（未开始窗口时为 `None`）。
    pub fn attempt_id(&self) -> Option<&str> {
        self.attempt_id.as_deref()
    }

    /// remote_task_id 冲突详情（非 `None` 时执行器强制落 `submission_unknown`）。
    pub fn conflict(&self) -> Option<&str> {
        self.conflict.as_deref()
    }

    /// 第 1 步：持久化 attempt intent。
    ///
    /// 同阶段若存在"未标记 submitting"的陈旧 intent（上一次执行在发出请求前中断），
    /// 先标记为 `failed`（未发出请求，可安全重领）再建新 intent；
    /// 若存在 `submitting`/`unknown`，说明结果未定，**拒绝**新建（恢复流程必须先对账）。
    pub async fn begin_intent(&mut self, request_hash: &str) -> Result<String, JobError> {
        // `BEGIN IMMEDIATE`：本事务**先读后写**（先查未决 attempt、再插入 intent）。
        // WAL 下 deferred 事务的"读→写升级"遇到活跃写者会立即返回 SQLITE_BUSY
        // （sqlx 报 `database is locked`，busy_timeout 不生效），把一次付费批次误判为
        // 可重试失败（安全但浪费重试额度；T14 冒烟实测 1 次）。先取写锁则由 busy_timeout
        // 正常等待（与 `job_stages::claim_next` 同一模式）。统一封装见 `storage::tx`。
        let mut tx = crate::storage::begin_write_pool(&self.pool).await?;
        if let Some(existing) =
            repo::attempts::unresolved_for_stage(&mut tx, &self.stage_id).await?
        {
            match existing.submit_state {
                manual_core::domain::SubmitState::Intent => {
                    repo::attempts::mark_failed(
                        &mut tx,
                        &existing.id,
                        "阶段被重新领取前未发出请求（intent 未标记 submitting）",
                        self.now,
                    )
                    .await?;
                }
                manual_core::domain::SubmitState::Submitting
                | manual_core::domain::SubmitState::Unknown => {
                    tx.rollback().await?;
                    return Err(JobError::handler(
                        &self.stage_id,
                        format!(
                            "该阶段已有未对账 attempt（{}）：必须先对账，不得新建付费提交",
                            existing.submit_state.as_str()
                        ),
                    ));
                }
                manual_core::domain::SubmitState::Accepted
                | manual_core::domain::SubmitState::Failed => {}
            }
        }
        let attempt = repo::attempts::create_intent(
            &mut tx,
            repo::attempts::NewAttempt {
                job_id: self.job_id.clone(),
                stage_id: self.stage_id.clone(),
                request_hash: request_hash.to_owned(),
            },
            self.now,
        )
        .await?;
        tx.commit().await?;
        self.attempt_id = Some(attempt.id.clone());
        // 崩在"intent 已落库、尚未标记 submitting"：恢复时可安全重领（未发出请求）。
        crate::job_failpoint!(
            self.owner.as_str(),
            crate::jobs::failpoints::PAID_AFTER_INTENT_BEFORE_SUBMITTING
        );
        Ok(attempt.id)
    }

    /// 第 2 步：标记 `submitting`。**返回后才能发 HTTP POST**。
    pub async fn mark_submitting(&mut self) -> Result<(), JobError> {
        let attempt_id = self.require_attempt()?;
        let mut conn = self.pool.acquire().await?;
        let changed = repo::attempts::mark_submitting(&mut conn, attempt_id, self.now).await?;
        if !changed {
            return Err(JobError::handler(
                &self.stage_id,
                "attempt 不处于 intent 状态，无法标记 submitting",
            ));
        }
        // 崩在"已标记 submitting、请求未发出"：恢复按 unknown 处理（客户端无法证明未发出）。
        crate::job_failpoint!(
            self.owner.as_str(),
            crate::jobs::failpoints::PAID_AFTER_SUBMITTING_BEFORE_REQUEST
        );
        Ok(())
    }

    /// 第 3 步（异步链路，Tripo）：收到远端 task ID 立即持久化事实观察。
    ///
    /// 冲突处理是**生产路径的硬行为**：已有不同 ID 时记录审计事件 + 错误日志，
    /// 把 attempt 标为 `unknown`、本窗口标记冲突（执行器据此落 `submission_unknown`），
    /// 且**不覆盖**已有 ID（触发器是最终边界）。
    pub async fn record_remote_task_id(
        &mut self,
        remote_task_id: &str,
    ) -> Result<RemoteTaskObservation, JobError> {
        let attempt_id = self.require_attempt()?.to_owned();
        // 崩在"响应已到、事实未落库"：客户端视角等价于"供应商接受但响应未到" → unknown。
        crate::job_failpoint!(
            self.owner.as_str(),
            crate::jobs::failpoints::PAID_AFTER_RESPONSE_BEFORE_RECEIPT
        );
        let mut conn = self.pool.acquire().await?;
        let outcome =
            repo::attempts::record_remote_task_id(&mut conn, &attempt_id, remote_task_id, self.now)
                .await?;
        match &outcome {
            repo::attempts::RemoteTaskOutcome::Recorded
            | repo::attempts::RemoteTaskOutcome::SameAsRecorded => {
                // 崩在"task ID 已落库、业务状态未推进"：恢复按已知 ID 继续查询，不重发。
                crate::job_failpoint!(
                    self.owner.as_str(),
                    crate::jobs::failpoints::PAID_AFTER_RECEIPT_BEFORE_ADVANCE
                );
                Ok(match outcome {
                    repo::attempts::RemoteTaskOutcome::Recorded => RemoteTaskObservation::Recorded,
                    _ => RemoteTaskObservation::SameAsRecorded,
                })
            }
            repo::attempts::RemoteTaskOutcome::Conflict { existing } => {
                let detail = format!(
                    "provider_attempts.remote_task_id 冲突：已有 {existing}，本次返回 {remote_task_id}；\
                     保留已有值并暂停该分支（不覆盖、不自动重购）"
                );
                repo::attempts::set_last_error(&mut conn, &attempt_id, &detail, self.now).await?;
                repo::attempts::mark_unknown(&mut conn, &attempt_id, &detail, self.now).await?;
                repo::audit::record(
                    &mut conn,
                    repo::audit::NewAuditEvent {
                        entity_type: "provider_attempt".to_owned(),
                        entity_id: attempt_id.clone(),
                        actor: Some("system".to_owned()),
                        action: "provider_attempt_remote_task_id_conflict".to_owned(),
                        result: "submission_unknown".to_owned(),
                        metadata_json: Some(
                            serde_json::json!({
                                "existingRemoteTaskId": existing,
                                "returnedRemoteTaskId": remote_task_id,
                                "stageId": self.stage_id,
                                "jobId": self.job_id,
                            })
                            .to_string(),
                        ),
                    },
                    self.now,
                )
                .await?;
                drop(conn);
                self.conflict = Some(detail.clone());
                tracing::error!(
                    event = "provider_attempt_remote_task_id_conflict",
                    jobId = %self.job_id,
                    stageId = %self.stage_id,
                    attemptId = %attempt_id,
                    "远端 task ID 冲突：保留已有值，该分支暂停并等待管理员对账（停机告警）"
                );
                Ok(RemoteTaskObservation::Conflict {
                    existing: existing.clone(),
                })
            }
        }
    }

    /// 第 3 步（同步链路，说明书 AI）：**完整响应已持久化**后写入 receipt。
    ///
    /// 调用方必须先把结果与 usage 落库（"先持久化响应结果资产，再在同一短事务保存
    /// usage、receipt 和完成 checkpoint"）；本方法只写 attempt 侧的 receipt。
    /// `response_id` 不假定可轮询，也**不**允许据此重取。
    pub async fn record_sync_response(
        &mut self,
        response_id: Option<&str>,
        usage_json: Option<&str>,
        result_asset_id: Option<&str>,
    ) -> Result<(), JobError> {
        let attempt_id = self.require_attempt()?.to_owned();
        // 崩在"同步请求已发、完整响应未持久化"：该批进入 submission_unknown。
        crate::job_failpoint!(
            self.owner.as_str(),
            crate::jobs::failpoints::MANUAL_AFTER_REQUEST_BEFORE_RESPONSE
        );
        // "先持久化响应结果资产，再在同一短事务保存 usage、receipt 和完成 checkpoint"：
        // 这里的短事务同时写 receipt（attempt）与结果事实（阶段列）；完成 checkpoint
        // （状态推进）仍由执行器带租约 epoch 完成。
        // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
        let mut tx = crate::storage::begin_write_pool(&self.pool).await?;
        let changed =
            repo::attempts::record_sync_response(&mut tx, &attempt_id, response_id, self.now)
                .await?;
        if !changed {
            tx.rollback().await?;
            return Err(JobError::handler(
                &self.stage_id,
                "attempt 不处于 intent/submitting，无法记录同步响应",
            ));
        }
        // 结果事实：允许过期 worker 保存（合同：不可变 receipt/result 事实）。
        repo::job_stages::set_result_fact(
            &mut tx,
            &self.stage_id,
            result_asset_id,
            usage_json,
            self.now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// 明确失败（只有可证明未被接受的错误才允许）：标记 attempt `failed`。
    pub async fn mark_failed(&mut self, reason: &str) -> Result<(), JobError> {
        let attempt_id = self.require_attempt()?.to_owned();
        let mut conn = self.pool.acquire().await?;
        repo::attempts::mark_failed(&mut conn, &attempt_id, reason, self.now).await?;
        Ok(())
    }

    /// 结果未知：标记 attempt `unknown`（禁止自动重购）。
    pub async fn mark_unknown(&mut self, reason: &str) -> Result<(), JobError> {
        let attempt_id = self.require_attempt()?.to_owned();
        let mut conn = self.pool.acquire().await?;
        repo::attempts::mark_unknown(&mut conn, &attempt_id, reason, self.now).await?;
        Ok(())
    }

    /// 读取当前 attempt 行（测试与日志断言用）。
    pub async fn attempt(&self) -> Result<Option<ProviderAttempt>, JobError> {
        let Some(attempt_id) = &self.attempt_id else {
            return Ok(None);
        };
        let mut conn = self.pool.acquire().await?;
        repo::attempts::get(&mut conn, attempt_id)
            .await
            .map_err(Into::into)
    }

    fn require_attempt(&self) -> Result<&str, JobError> {
        self.attempt_id.as_deref().ok_or_else(|| {
            JobError::handler(
                &self.stage_id,
                "提交窗口尚未开始（必须先调用 begin_intent）",
            )
        })
    }
}

/// 结果事实写入（非付费阶段也可用：`model_download` 等把产物资产落库后再推进 checkpoint）。
///
/// 与 [`SubmissionWindow::record_sync_response`] 的区别：这里不含 attempt（没有付费提交）。
pub async fn record_result_fact(
    pool: &SqlitePool,
    stage_id: &str,
    result_asset_id: Option<&str>,
    usage_json: Option<&str>,
    now: Timestamp,
) -> Result<(), StorageError> {
    let mut conn = pool.acquire().await?;
    repo::job_stages::set_result_fact(&mut conn, stage_id, result_asset_id, usage_json, now).await
}

/// 便捷：把 `serde_json::Value` 序列化为列文本（`None` 保持 `None`）。
pub fn json_text(value: Option<&Value>) -> Option<String> {
    value.map(|value| value.to_string())
}
