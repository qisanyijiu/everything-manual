//! 执行器：领取 → 执行 → 带租约 epoch 的推进（T10 / REQ-024）。
//!
//! 并发模型（architecture.md §6）：
//! - 后台循环把可执行阶段**领取后 spawn**，并发上限由 SQL 领取谓词保证
//!   （全局远端生成 2、说明书批次 2，可配置降低）；
//! - 每个阶段执行期间有独立的续约任务（默认 20s 续约、租约 120s）；
//! - 退出时停止领取，等待短在途写入（[`ExecutorConfig::shutdown_grace`]），
//!   超时中止任务——被硬杀的进程由租约过期 + 恢复矩阵兜底。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sqlx::SqlitePool;
use tokio::sync::Notify;
use tokio::task::{JoinHandle, JoinSet};

use manual_core::domain::{JobStage, JobStatus, ProviderAttempt, StageKind, SubmitState};
use manual_core::jobs::{
    MAX_SAFE_RETRIES, REMOTE_WAIT_BUDGET_SECONDS, poll_interval_seconds, remote_wait_anchor,
    remote_wait_anchor_kind, retry_after_seconds, retry_delay_seconds, safe_retries_remain,
    waited_seconds,
};
use manual_core::timestamps::Timestamp;

use crate::config::{MAX_MANUAL_AI_BATCH_CONCURRENCY, MAX_REMOTE_GENERATION_CONCURRENCY, Settings};
use crate::storage::repo::job_stages::{ClaimParams, LeaseGuard, StageAdvance};
use crate::storage::repo::{self, job_stages};

use super::handler::{MissingItem, ResumeHint, StageContext, StageOutcome, StageRegistry};
use super::recover::{self, RecoveryReport};
use super::submission::SubmissionWindow;
use super::{Clock, FixedJitter, Jitter, JobError, SystemClock, SystemJitter, worker_owner};

/// 执行器配置（默认值即合同值；**可降低**，提高要经需求确认）。
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// 租约时长（默认 120s）。
    pub lease: Duration,
    /// 续约间隔（默认 20s；必须小于 `lease`）。
    pub renew: Duration,
    /// 全局远端生成并发上限（默认 2）。
    pub remote_generation_limit: u32,
    /// 说明书批次并发上限（默认 2）。
    pub manual_ai_batch_limit: u32,
    /// 空转轮询间隔（用 `wake` 唤醒时不等待）。
    pub idle_poll: Duration,
    /// 退出时等待在途阶段完成的宽限（超过则中止任务，交由租约/恢复处理）。
    pub shutdown_grace: Duration,
    /// 未注册处理器的阶段被延后的时间（不假成功，也不占用重试额度）。
    pub unregistered_handler_delay: Duration,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            lease: Duration::from_secs(120),
            renew: Duration::from_secs(20),
            remote_generation_limit: MAX_REMOTE_GENERATION_CONCURRENCY,
            manual_ai_batch_limit: MAX_MANUAL_AI_BATCH_CONCURRENCY,
            idle_poll: Duration::from_millis(250),
            shutdown_grace: Duration::from_secs(5),
            unregistered_handler_delay: Duration::from_secs(60),
        }
    }
}

impl ExecutorConfig {
    /// 从服务配置构造（`concurrency` 与 `jobs` 段；租约/续约不得为 0 且 renew < lease）。
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            lease: Duration::from_secs(settings.jobs.lease_seconds),
            renew: Duration::from_secs(settings.jobs.renew_seconds),
            remote_generation_limit: settings.concurrency.remote_generation,
            manual_ai_batch_limit: settings.concurrency.manual_ai_batches,
            ..Self::default()
        }
    }

    /// 启动前校验：非法配置明确报错（不静默回退默认值）。
    pub fn validate(&self) -> Result<(), JobError> {
        let invalid = |message: String| Err(JobError::Config { message });
        if self.lease.is_zero() {
            return invalid("租约时长必须大于 0".to_owned());
        }
        if self.renew.is_zero() || self.renew >= self.lease {
            return invalid(format!(
                "续约间隔（{}s）必须大于 0 且小于租约时长（{}s），否则租约会在续约前过期",
                self.renew.as_secs(),
                self.lease.as_secs()
            ));
        }
        if self.remote_generation_limit == 0
            || self.remote_generation_limit > MAX_REMOTE_GENERATION_CONCURRENCY
        {
            return invalid(format!(
                "远端生成并发上限须在 1..={MAX_REMOTE_GENERATION_CONCURRENCY}（可降低，不可未经确认提高）"
            ));
        }
        if self.manual_ai_batch_limit == 0
            || self.manual_ai_batch_limit > MAX_MANUAL_AI_BATCH_CONCURRENCY
        {
            return invalid(format!(
                "说明书批次并发上限须在 1..={MAX_MANUAL_AI_BATCH_CONCURRENCY}（可降低，不可未经确认提高）"
            ));
        }
        Ok(())
    }
}

/// 一次 tick 的结果。
#[derive(Debug, Clone)]
pub enum TickOutcome {
    /// 没有可领取的阶段（或并发额度已满）。
    Idle,
    /// 执行了一个阶段（含推进结果）。
    Executed(StageRunReport),
}

/// 单个阶段的执行报告（测试断言与日志用；不含用户资料）。
#[derive(Debug, Clone)]
pub struct StageRunReport {
    pub job_id: String,
    pub stage_id: String,
    pub stage_kind: StageKind,
    /// 处理器返回的结果（或执行器归一化后的结果）。
    pub outcome: StageOutcome,
    /// 推进成功时的阶段新状态；`None` = 推进被拒绝（租约过期/被接管）。
    pub status: Option<JobStatus>,
    /// 附加说明（退避秒数、Retry-After 截断、延后原因等）。
    pub note: Option<String>,
}

/// 持久任务执行器。
pub struct JobExecutor {
    pool: SqlitePool,
    config: ExecutorConfig,
    registry: Arc<StageRegistry>,
    clock: Arc<dyn Clock>,
    jitter: Arc<dyn Jitter>,
    owner: String,
    /// 有新工作可用（T11 建单、T15 重试后调用 [`JobExecutor::wake`]）。
    wake: Notify,
    /// 退出信号（与 `wake` 分开，避免"唤醒"和"停机"互相吞掉通知）。
    shutdown_signal: Notify,
    shutdown: AtomicBool,
}

impl JobExecutor {
    /// 生产构造：系统时钟 + 系统 jitter + 空处理器注册表（真实适配器由 T12/T14/T15 注册）。
    pub fn new(pool: SqlitePool, config: ExecutorConfig, registry: StageRegistry) -> Arc<Self> {
        Self::with_runtime(
            pool,
            config,
            Arc::new(registry),
            Arc::new(SystemClock),
            Arc::new(SystemJitter),
        )
    }

    /// 注入时钟与 jitter（测试用：[`super::ManualClock`] + [`FixedJitter`]）。
    pub fn with_runtime(
        pool: SqlitePool,
        config: ExecutorConfig,
        registry: Arc<StageRegistry>,
        clock: Arc<dyn Clock>,
        jitter: Arc<dyn Jitter>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            config,
            registry,
            clock,
            jitter,
            owner: worker_owner(),
            wake: Notify::new(),
            shutdown_signal: Notify::new(),
            shutdown: AtomicBool::new(false),
        })
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn config(&self) -> &ExecutorConfig {
        &self.config
    }

    pub fn registry(&self) -> &StageRegistry {
        &self.registry
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// 通知有新工作（不阻塞；`Notify::notify_one` 会保留一个许可，不会丢唤醒）。
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// 请求退出：停止领取，让在途阶段在宽限期内收尾。
    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.shutdown_signal.notify_waiters();
        self.wake.notify_one();
    }

    /// 启动后台 worker（`serve` 调用），返回停机句柄。
    pub fn start(self: &Arc<Self>) -> ExecutorHandle {
        let executor = Arc::clone(self);
        let task = tokio::spawn(async move { executor.run_loop().await });
        ExecutorHandle {
            executor: Arc::clone(self),
            task: Some(task),
        }
    }

    /// 单次调度：先收敛过期租约，再领取并执行一个阶段。
    ///
    /// 测试用它做确定性推进；生产用 [`JobExecutor::start`] 的循环。
    pub async fn tick(&self) -> Result<TickOutcome, JobError> {
        self.recover_expired_leases().await?;
        match self.claim().await? {
            None => Ok(TickOutcome::Idle),
            Some(stage) => Ok(TickOutcome::Executed(self.execute(stage).await?)),
        }
    }

    /// 恢复扫描：收敛所有"租约已过期的 running 阶段"（恢复矩阵见 `jobs::recover`）。
    pub async fn recover_expired_leases(&self) -> Result<RecoveryReport, JobError> {
        let now = self.now();
        let expired = job_stages::expired_running(&self.pool, now).await?;
        let mut report = RecoveryReport {
            scanned: expired.len(),
            ..RecoveryReport::default()
        };
        for stage in expired {
            match self.recover_one(&stage, now).await {
                Ok(Some(action_code)) => {
                    report.recovered += 1;
                    match action_code {
                        "requeued" => report.requeued += 1,
                        "succeeded" => report.succeeded += 1,
                        "submission_unknown" => report.submission_unknown += 1,
                        "needs_input" => report.needs_input += 1,
                        "cancelled" => report.cancelled += 1,
                        _ => {}
                    }
                }
                Ok(None) => report.skipped += 1,
                Err(error) => {
                    report.skipped += 1;
                    tracing::warn!(
                        event = "job_recovery_failed",
                        stageId = %stage.id,
                        jobId = %stage.job_id,
                        error = %error,
                        "恢复过期租约阶段失败（下一轮重试；不修改业务状态）"
                    );
                }
            }
        }
        if report.scanned > 0 {
            tracing::info!(
                event = "job_recovery",
                owner = %self.owner,
                detail = %report.summary(),
                "过期租约恢复扫描完成"
            );
        }
        Ok(report)
    }

    /// 恢复单个阶段：接管租约（epoch + 1）→ 判定 → 落库 → 聚合父 job 状态。
    ///
    /// 返回 `Ok(None)` 表示接管失败（已被其他 worker 收敛）。
    async fn recover_one(
        &self,
        stage: &JobStage,
        now: Timestamp,
    ) -> Result<Option<&'static str>, JobError> {
        // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
        let mut tx = crate::storage::begin_write_pool(&self.pool).await?;
        let taken = job_stages::take_over_expired(
            &mut tx,
            &stage.id,
            stage.lease_epoch,
            &self.owner,
            now,
            self.config.lease,
        )
        .await?;
        if !taken {
            tx.rollback().await?;
            return Ok(None);
        }
        let attempt = repo::attempts::latest_for_stage(&mut tx, &stage.id).await?;
        let job = repo::jobs::get(&mut tx, &stage.job_id).await?;
        let job_status = job.map(|job| job.status).unwrap_or(JobStatus::Failed);
        let action = recover::plan(stage, attempt.as_ref(), job_status);
        let guard = LeaseGuard {
            stage_id: stage.id.clone(),
            owner: self.owner.clone(),
            epoch: stage.lease_epoch + 1,
        };
        let (advance, code, note) = match &action {
            recover::RecoveryAction::Succeed { result_asset_id } => {
                // "恢复程序校验已有结果并补推进"：结果资产必须在库里，否则按完整性问题处理。
                if let Some(asset_id) = result_asset_id {
                    let exists = repo::assets::get(&mut tx, asset_id).await?.is_some();
                    if !exists {
                        (
                            StageAdvance::NeedsInput {
                                needs_input_json: serde_json::to_string(&[MissingItem::new(
                                    "stage_result_asset_missing",
                                    "阶段记录了结果资产但资产行不存在；请人工核对（不自动重新请求）",
                                )])
                                .unwrap_or_else(|_| "[]".to_owned()),
                                last_error: "结果资产缺失（完整性异常）".to_owned(),
                            },
                            "needs_input",
                            Some("结果资产不存在".to_owned()),
                        )
                    } else {
                        (
                            StageAdvance::Succeeded {
                                result_asset_id: None,
                                usage_json: None,
                            },
                            "succeeded",
                            None,
                        )
                    }
                } else {
                    (
                        StageAdvance::Succeeded {
                            result_asset_id: None,
                            usage_json: None,
                        },
                        "succeeded",
                        None,
                    )
                }
            }
            recover::RecoveryAction::SubmissionUnknown { attempt_id, reason } => {
                if let Some(attempt_id) = attempt_id {
                    repo::attempts::mark_unknown(&mut tx, attempt_id, reason, now).await?;
                }
                (
                    StageAdvance::SubmissionUnknown {
                        last_error: reason.clone(),
                    },
                    "submission_unknown",
                    Some(reason.clone()),
                )
            }
            recover::RecoveryAction::Requeue => (StageAdvance::Requeue, "requeued", None),
            recover::RecoveryAction::NeedsInput { items, reason } => (
                StageAdvance::NeedsInput {
                    needs_input_json: serde_json::to_string(items)
                        .unwrap_or_else(|_| "[]".to_owned()),
                    last_error: reason.clone(),
                },
                "needs_input",
                Some(reason.clone()),
            ),
            recover::RecoveryAction::Cancel => (StageAdvance::Cancel, "cancelled", None),
        };
        let advanced = job_stages::advance(&mut tx, &guard, now, &advance).await?;
        if advanced {
            repo::jobs::recompute_status(&mut tx, &stage.job_id, now).await?;
        }
        tx.commit().await?;
        if !advanced {
            tracing::warn!(
                event = "job_recovery_advance_rejected",
                stageId = %stage.id,
                "恢复推进被拒绝（阶段状态已被其他写入改变）"
            );
            return Ok(None);
        }
        tracing::info!(
            event = "job_recovery_applied",
            stageId = %stage.id,
            jobId = %stage.job_id,
            stageKind = stage.stage_kind.as_str(),
            action = code,
            detail = note.as_deref().unwrap_or(""),
            "过期阶段已收敛"
        );
        Ok(Some(code))
    }

    /// 领取一个阶段（SQL 条件更新；并发上限在领取谓词内判定）。
    async fn claim(&self) -> Result<Option<JobStage>, JobError> {
        let params = ClaimParams {
            owner: self.owner.clone(),
            now: self.now(),
            lease: self.config.lease,
            remote_generation_limit: self.config.remote_generation_limit,
            manual_ai_batch_limit: self.config.manual_ai_batch_limit,
        };
        Ok(job_stages::claim_next(&self.pool, &params).await?)
    }

    /// 执行一个已领取的阶段：准备上下文 → 续约任务 → 处理器 → 冲突归一化 → 推进。
    async fn execute(&self, stage: JobStage) -> Result<StageRunReport, JobError> {
        let now = self.now();
        let mut conn = self.pool.acquire().await?;
        let job = repo::jobs::get(&mut conn, &stage.job_id)
            .await?
            .ok_or_else(|| JobError::handler(&stage.id, format!("job 不存在：{}", stage.job_id)))?;
        let attempt = repo::attempts::latest_for_stage(&mut conn, &stage.id).await?;
        drop(conn);

        let guard = LeaseGuard {
            stage_id: stage.id.clone(),
            owner: self.owner.clone(),
            epoch: stage.lease_epoch,
        };
        let resume = resume_hint(attempt.as_ref());

        // 未决事实：不调用处理器，也不产生任何新请求。
        if let ResumeHint::SubmissionUnknown { reason, .. } = &resume {
            tracing::warn!(
                event = "job_stage_submission_unknown",
                stageId = %stage.id,
                jobId = %stage.job_id,
                stageKind = stage.stage_kind.as_str(),
                "阶段存在未决提交事实：直接落 submission_unknown（不重发付费请求）"
            );
            return self
                .apply(
                    &stage,
                    &guard,
                    attempt.as_ref(),
                    StageOutcome::SubmissionUnknown {
                        reason: reason.clone(),
                    },
                    now,
                )
                .await;
        }

        let Some(handler) = self.registry.get(stage.stage_kind) else {
            // 未接入的适配器：不假成功、不消耗重试额度，延后并记录原因。
            let next_run_at = now
                .checked_add_millis(self.config.unregistered_handler_delay.as_millis() as i64)
                .unwrap_or(now);
            tracing::warn!(
                event = "job_stage_handler_missing",
                stageId = %stage.id,
                stageKind = stage.stage_kind.as_str(),
                registered = %self
                    .registry
                    .registered()
                    .iter()
                    .map(|kind| kind.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                "阶段处理器未注册（适配器属 T12/T14/T15）：延后重试，不推进业务状态"
            );
            return self
                .defer(
                    &stage,
                    &guard,
                    next_run_at,
                    "阶段处理器未注册（T12/T14/T15 接入前不执行）",
                    now,
                )
                .await;
        };

        let mut ctx = StageContext {
            pool: self.pool.clone(),
            job,
            stage: stage.clone(),
            now,
            resume,
            attempt: attempt.clone(),
            submission: SubmissionWindow::new(
                self.pool.clone(),
                stage.job_id.clone(),
                stage.id.clone(),
                self.owner.clone(),
                now,
            ),
        };
        let lease_lost = Arc::new(AtomicBool::new(false));
        let renewal = self.spawn_renewal(&guard, Arc::clone(&lease_lost));

        let outcome = match handler.run(&mut ctx).await {
            Ok(outcome) => outcome,
            Err(error) => {
                let mapped = self.map_handler_error(&ctx, &error).await;
                tracing::warn!(
                    event = "job_stage_handler_error",
                    stageId = %stage.id,
                    jobId = %stage.job_id,
                    stageKind = stage.stage_kind.as_str(),
                    errorCode = error.code(),
                    // 诊断摘要（脱敏：处理器错误的 Display 不含密钥/原文；见各适配器）。
                    detail = %error,
                    mapped = mapped.code(),
                    "阶段处理器返回错误"
                );
                mapped
            }
        };
        renewal.abort();
        let _ = renewal.await;
        if lease_lost.load(Ordering::SeqCst) {
            tracing::warn!(
                event = "job_stage_lease_lost",
                stageId = %stage.id,
                jobId = %stage.job_id,
                "执行期间租约被接管：只能保存事实，业务推进将被拒绝"
            );
        }

        // 远端 task ID 冲突优先于处理器结论：不得假装成功，不得覆盖已有 ID。
        let outcome = match ctx.submission.conflict() {
            Some(detail) => StageOutcome::SubmissionUnknown {
                reason: detail.to_owned(),
            },
            None => outcome,
        };
        self.apply(&stage, &guard, attempt.as_ref(), outcome, now)
            .await
    }

    /// 处理器错误 → 状态语义。
    ///
    /// 安全网：若提交窗口已经留下未决 attempt（`submitting`/`unknown`），一律按
    /// `submission_unknown` 处理（客户端无法证明付费请求未被接受）；否则按安全临时失败
    /// 处理（可重试、有退避）。
    async fn map_handler_error(&self, ctx: &StageContext, error: &JobError) -> StageOutcome {
        let unresolved = ctx
            .submission
            .attempt()
            .await
            .ok()
            .flatten()
            .is_some_and(|attempt| {
                matches!(
                    attempt.submit_state,
                    SubmitState::Submitting | SubmitState::Unknown
                )
            });
        if unresolved {
            return StageOutcome::SubmissionUnknown {
                reason: format!("处理器失败且提交结果未知：{error}"),
            };
        }
        StageOutcome::Retryable {
            reason: format!("处理器失败：{error}"),
            retry_after_seconds: None,
        }
    }

    /// 归一并推进业务状态（带租约 epoch guard）。
    async fn apply(
        &self,
        stage: &JobStage,
        guard: &LeaseGuard,
        attempt: Option<&ProviderAttempt>,
        outcome: StageOutcome,
        now: Timestamp,
    ) -> Result<StageRunReport, JobError> {
        // 1) 不可变事实优先落库（无 epoch guard：过期 worker 也能保存结果事实）。
        if let StageOutcome::Succeeded {
            result_asset_id,
            usage,
        } = &outcome
            && (result_asset_id.is_some() || usage.is_some())
        {
            let usage_text = usage.as_ref().map(|value| value.to_string());
            let mut conn = self.pool.acquire().await?;
            job_stages::set_result_fact(
                &mut conn,
                &stage.id,
                result_asset_id.as_deref(),
                usage_text.as_deref(),
                now,
            )
            .await?;
            drop(conn);
        }
        // 崩在"结果事实已持久化、checkpoint 未推进"：恢复会校验结果并补推进，不重新付费。
        crate::job_failpoint!(
            self.owner.as_str(),
            crate::jobs::failpoints::RESULT_FACT_BEFORE_CHECKPOINT
        );

        // 2) 归一化：状态机规则（退避、轮询节奏、总等待预算）在核心策略里。
        //    等待预算的起点是"该远端等待链的 accepted 提交事实"：轮询阶段自身没有
        //    attempt（BUG-003），这里在判定前取一次同一 job 的提交事实作锚点回退。
        let wait_anchor = match &outcome {
            StageOutcome::WaitingProvider => self.wait_anchor(stage).await?,
            _ => None,
        };
        let (advance, note) = plan_advance(
            stage,
            attempt,
            wait_anchor.as_ref(),
            &outcome,
            now,
            self.jitter.as_ref(),
        );

        // 3) 业务推进 + 父 job 聚合（同一短事务）。
        // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
        let mut tx = crate::storage::begin_write_pool(&self.pool).await?;
        let advanced = job_stages::advance(&mut tx, guard, now, &advance).await?;
        if advanced {
            repo::jobs::recompute_status(&mut tx, &stage.job_id, now).await?;
        }
        tx.commit().await?;

        let status = if advanced {
            Some(target_status(&advance))
        } else {
            tracing::warn!(
                event = "job_stage_advance_rejected_stale_lease",
                stageId = %stage.id,
                jobId = %stage.job_id,
                stageKind = stage.stage_kind.as_str(),
                outcome = outcome.code(),
                "业务推进被拒绝：租约已过期或被接管（事实已保存，状态不变、后续阶段不解锁）"
            );
            None
        };
        Ok(StageRunReport {
            job_id: stage.job_id.clone(),
            stage_id: stage.id.clone(),
            stage_kind: stage.stage_kind,
            outcome,
            status,
            note,
        })
    }

    /// 该远端等待链的提交事实（等待起点锚点；BUG-003）。
    ///
    /// 轮询及其下游阶段**自身不建 attempt**：远端 task ID 与提交时刻都在同一 job 的
    /// accepted 提交事实里（`tripo_submit`；与处理器取 task ID 用的是同一份事实，
    /// 见 `repo::attempts::latest_accepted_for_job`）。按 job 归属查询，多 job 不串用。
    async fn wait_anchor(&self, stage: &JobStage) -> Result<Option<ProviderAttempt>, JobError> {
        let Some(anchor_kind) = remote_wait_anchor_kind(stage.stage_kind) else {
            return Ok(None);
        };
        let mut conn = self.pool.acquire().await?;
        Ok(repo::attempts::latest_accepted_for_job(&mut conn, &stage.job_id, anchor_kind).await?)
    }

    /// 延后一个阶段（`queued` + `next_run_at`，不消耗安全重试额度），仍带租约 guard。
    async fn defer(
        &self,
        stage: &JobStage,
        guard: &LeaseGuard,
        next_run_at: Timestamp,
        reason: &str,
        now: Timestamp,
    ) -> Result<StageRunReport, JobError> {
        let advance = StageAdvance::Defer {
            next_run_at,
            last_error: reason.to_owned(),
        };
        // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
        let mut tx = crate::storage::begin_write_pool(&self.pool).await?;
        let advanced = job_stages::advance(&mut tx, guard, now, &advance).await?;
        if advanced {
            repo::jobs::recompute_status(&mut tx, &stage.job_id, now).await?;
        }
        tx.commit().await?;
        Ok(StageRunReport {
            job_id: stage.job_id.clone(),
            stage_id: stage.id.clone(),
            stage_kind: stage.stage_kind,
            outcome: StageOutcome::Retryable {
                reason: reason.to_owned(),
                retry_after_seconds: None,
            },
            status: advanced.then_some(JobStatus::Queued),
            note: Some(if advanced {
                format!(
                    "{reason}；已延后到 {}（不消耗安全重试额度）",
                    next_run_at.to_rfc3339()
                )
            } else {
                format!("{reason}；延后写入被拒绝（租约已过期）")
            }),
        })
    }

    /// 续约任务：每 `renew` 延长租约；被接管或以失败告终时停下（推进随后会被拒绝）。
    fn spawn_renewal(&self, guard: &LeaseGuard, lost: Arc<AtomicBool>) -> JoinHandle<()> {
        let pool = self.pool.clone();
        let config = self.config.clone();
        let clock = Arc::clone(&self.clock);
        let owner = self.owner.clone();
        let guard = guard.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(config.renew).await;
                let now = clock.now();
                let renewed = match pool.acquire().await {
                    Ok(mut conn) => {
                        job_stages::renew_lease(&mut conn, &guard, now, config.lease).await
                    }
                    Err(error) => Err(error.into()),
                };
                match renewed {
                    Ok(true) => {}
                    Ok(false) => {
                        lost.store(true, Ordering::SeqCst);
                        tracing::warn!(
                            event = "job_stage_lease_renewal_rejected",
                            stageId = %guard.stage_id,
                            owner = %owner,
                            "续约被拒绝：阶段已被接管或已推进"
                        );
                        return;
                    }
                    Err(error) => {
                        lost.store(true, Ordering::SeqCst);
                        tracing::warn!(
                            event = "job_stage_lease_renewal_failed",
                            stageId = %guard.stage_id,
                            error = %error,
                            "续约失败：停止续约（租约到期后由恢复兜底）"
                        );
                        return;
                    }
                }
            }
        })
    }

    /// 后台调度循环（`serve` 与并发测试使用）。
    async fn run_loop(self: Arc<Self>) {
        let mut running: JoinSet<()> = JoinSet::new();
        loop {
            if self.is_shutdown() {
                break;
            }
            if let Err(error) = self.recover_expired_leases().await {
                tracing::warn!(event = "job_recovery_failed", error = %error, "恢复扫描失败");
            }
            loop {
                match self.claim().await {
                    Ok(Some(stage)) => {
                        let executor = Arc::clone(&self);
                        running.spawn(async move {
                            if let Err(error) = executor.execute(stage).await {
                                tracing::warn!(event = "job_stage_failed", error = %error, "阶段执行失败");
                            }
                        });
                    }
                    Ok(None) => break,
                    Err(error) => {
                        tracing::warn!(event = "job_claim_failed", error = %error, "领取阶段失败");
                        break;
                    }
                }
            }
            let idle = tokio::time::sleep(self.config.idle_poll);
            tokio::pin!(idle);
            if running.is_empty() {
                tokio::select! {
                    _ = self.wake.notified() => {},
                    _ = &mut idle => {},
                    _ = self.shutdown_signal.notified() => {},
                }
            } else {
                tokio::select! {
                    _ = running.join_next() => {},
                    _ = self.wake.notified() => {},
                    _ = &mut idle => {},
                    _ = self.shutdown_signal.notified() => {},
                }
            }
        }

        // 退出：停止领取后等待短在途写入／checkpoint；超时中止（硬杀路径由租约兜底）。
        let deadline = tokio::time::Instant::now() + self.config.shutdown_grace;
        while !running.is_empty() {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                tracing::warn!(
                    event = "job_shutdown_abort_inflight",
                    inflight = running.len(),
                    graceSeconds = self.config.shutdown_grace.as_secs(),
                    "宽限期结束仍有在途阶段：中止任务（租约到期后由恢复矩阵收敛）"
                );
                running.abort_all();
                while running.join_next().await.is_some() {}
                break;
            }
            tokio::select! {
                _ = running.join_next() => {},
                _ = tokio::time::sleep(remaining) => {},
            }
        }
        tracing::info!(event = "job_executor_stopped", owner = %self.owner, "任务执行器已停止领取");
    }
}

/// 停机句柄：`Drop` 时请求退出并中止循环任务（正常路径请显式 `shutdown().await`）。
pub struct ExecutorHandle {
    executor: Arc<JobExecutor>,
    task: Option<JoinHandle<()>>,
}

impl ExecutorHandle {
    pub fn executor(&self) -> &Arc<JobExecutor> {
        &self.executor
    }

    /// 优雅停止：停止领取 → 等待在途阶段（宽限期）→ 返回。
    pub async fn shutdown(mut self) {
        self.executor.request_shutdown();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for ExecutorHandle {
    fn drop(&mut self) {
        self.executor.request_shutdown();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// 由 attempt 事实得到恢复提示（contracts.md §5 第 5 条）。
fn resume_hint(attempt: Option<&ProviderAttempt>) -> ResumeHint {
    let Some(attempt) = attempt else {
        return ResumeHint::Fresh;
    };
    match attempt.submit_state {
        SubmitState::Intent | SubmitState::Failed => ResumeHint::Fresh,
        SubmitState::Accepted => match &attempt.remote_task_id {
            Some(remote_task_id) => ResumeHint::HasRemoteTask {
                attempt_id: attempt.id.clone(),
                remote_task_id: remote_task_id.clone(),
            },
            // 同步链路（说明书 AI）的 accepted 没有远端任务可查询：按普通执行；
            // 结果事实已由提交窗口落库，执行器按"成功"推进（不重新请求）。
            None => ResumeHint::Fresh,
        },
        SubmitState::Submitting | SubmitState::Unknown => ResumeHint::SubmissionUnknown {
            attempt_id: attempt.id.clone(),
            reason: match attempt.last_error.as_deref() {
                Some(detail) if !detail.is_empty() => detail.to_owned(),
                _ => "付费创建结果未知：未对账 attempt 未收敛".to_owned(),
            },
        },
    }
}

/// 结果 → 状态推进载荷（退避、轮询节奏、总等待预算在此归一化）。
///
/// `attempt` 是该阶段自己的最近 attempt；`wait_anchor` 是同一 job 的提交事实
/// （仅 `WaitingProvider` 需要；轮询阶段没有自身 attempt 时靠它取等待起点，BUG-003）。
fn plan_advance(
    stage: &JobStage,
    attempt: Option<&ProviderAttempt>,
    wait_anchor: Option<&ProviderAttempt>,
    outcome: &StageOutcome,
    now: Timestamp,
    jitter: &dyn Jitter,
) -> (StageAdvance, Option<String>) {
    match outcome {
        // 结果资产与 usage 已在 apply 第 1 步作为**事实**落库（无 epoch guard）；
        // 这里只推进 checkpoint（状态），不再重复写事实列。
        StageOutcome::Succeeded { .. } => (
            StageAdvance::Succeeded {
                result_asset_id: None,
                usage_json: None,
            },
            None,
        ),
        StageOutcome::WaitingProvider => {
            // 等待起点 = 该远端等待链的 accepted 提交事实：优先本阶段自己的 accepted
            // attempt，否则回退到 job 的提交事实（轮询阶段无自身 attempt 的真实形态）。
            // 只用本阶段 attempt 会让真实轮询链路上预算恒不可达（BUG-003）。
            let anchor = remote_wait_anchor(attempt, wait_anchor);
            let waited = anchor
                .map(|anchor| waited_seconds(anchor.started_at, now))
                .unwrap_or(0);
            if manual_core::jobs::remote_wait_exceeded(waited) {
                let minutes = REMOTE_WAIT_BUDGET_SECONDS / 60;
                let items = vec![MissingItem::new(
                    "remote_wait_budget_exceeded",
                    format!(
                        "已自动等待超过 {minutes} 分钟：已停止自动等待，远端任务 ID 已保留；\
                         补齐后由用户触发继续（仅查询，不重新购买）"
                    ),
                )];
                (
                    StageAdvance::NeedsInput {
                        needs_input_json: serde_json::to_string(&items)
                            .unwrap_or_else(|_| "[]".to_owned()),
                        last_error: format!("远端等待超过 {minutes} 分钟（task_id 已保留）"),
                    },
                    Some(format!(
                        "总等待 {waited}s（≥ {minutes} 分钟）→ needs_input（保留 task_id：{}）",
                        anchor
                            .and_then(|anchor| anchor.remote_task_id.as_deref())
                            .unwrap_or("(无)")
                    )),
                )
            } else {
                let seconds = poll_interval_seconds(stage.poll_count as u32);
                (
                    StageAdvance::WaitingProvider {
                        next_run_at: now
                            .checked_add_millis((seconds as i64) * 1000)
                            .unwrap_or(now),
                    },
                    Some(format!(
                        "下次查询 {seconds}s 后（轮询第 {} 次；远端已等待 {waited}s）",
                        stage.poll_count + 1
                    )),
                )
            }
        }
        StageOutcome::Retryable {
            reason,
            retry_after_seconds: retry_after,
        } => {
            let used = stage.attempt_count.max(0) as u32;
            if !safe_retries_remain(used) {
                return (
                    StageAdvance::Failed {
                        last_error: format!("安全重试已用尽（{MAX_SAFE_RETRIES} 次）：{reason}"),
                    },
                    Some(format!("安全重试已用尽（{MAX_SAFE_RETRIES} 次）→ failed")),
                );
            }
            let retry_number = used + 1;
            let (seconds, note) = match retry_after {
                Some(header_seconds) => {
                    let (honored, capped) = retry_after_seconds(*header_seconds);
                    let note = capped.then(|| {
                        format!(
                            "Retry-After={header_seconds}s 超过上限，已截断为 {honored}s（已记录）"
                        )
                    });
                    (honored, note)
                }
                None => (
                    retry_delay_seconds(retry_number, jitter.sample()),
                    Some(format!(
                        "第 {retry_number}/{MAX_SAFE_RETRIES} 次重试（含 jitter）"
                    )),
                ),
            };
            (
                StageAdvance::RetryWait {
                    next_run_at: now
                        .checked_add_millis((seconds as i64) * 1000)
                        .unwrap_or(now),
                    last_error: reason.clone(),
                },
                Some(
                    note.unwrap_or_else(|| format!("第 {retry_number}/{MAX_SAFE_RETRIES} 次重试")),
                ),
            )
        }
        StageOutcome::NeedsInput { items } => (
            StageAdvance::NeedsInput {
                needs_input_json: serde_json::to_string(items).unwrap_or_else(|_| "[]".to_owned()),
                last_error: items
                    .first()
                    .map(|item| item.message.clone())
                    .unwrap_or_else(|| "需要补充输入".to_owned()),
            },
            Some(format!("needs_input：{} 项缺项", items.len())),
        ),
        StageOutcome::SubmissionUnknown { reason } => (
            StageAdvance::SubmissionUnknown {
                last_error: reason.clone(),
            },
            Some("submission_unknown：暂停该分支后续购买，等待对账".to_owned()),
        ),
        StageOutcome::Failed { reason } => (
            StageAdvance::Failed {
                last_error: reason.clone(),
            },
            Some("明确失败：不消耗安全重试额度，直接 failed".to_owned()),
        ),
    }
}

/// 推进载荷对应的目标状态（报告与断言用）。
fn target_status(advance: &StageAdvance) -> JobStatus {
    match advance {
        StageAdvance::Succeeded { .. } => JobStatus::Succeeded,
        StageAdvance::WaitingProvider { .. } => JobStatus::WaitingProvider,
        StageAdvance::RetryWait { .. } => JobStatus::RetryWait,
        StageAdvance::NeedsInput { .. } => JobStatus::NeedsInput,
        StageAdvance::SubmissionUnknown { .. } => JobStatus::SubmissionUnknown,
        StageAdvance::Failed { .. } => JobStatus::Failed,
        StageAdvance::Requeue | StageAdvance::Defer { .. } => JobStatus::Queued,
        StageAdvance::Cancel => JobStatus::Cancelled,
    }
}

/// 便捷构造（测试）：固定 jitter 的执行器。
pub fn fixed_jitter_executor(
    pool: SqlitePool,
    config: ExecutorConfig,
    registry: StageRegistry,
    clock: Arc<dyn Clock>,
) -> Arc<JobExecutor> {
    JobExecutor::with_runtime(
        pool,
        config,
        Arc::new(registry),
        clock,
        Arc::new(FixedJitter(0.0)),
    )
}
