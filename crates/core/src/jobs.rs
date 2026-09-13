//! 任务执行策略与状态机（contracts.md §5「任务阶段与崩溃语义」；T10）。
//!
//! 本模块是**纯逻辑**：不依赖 SQLx／Axum／系统时钟，只有可判定的规则与常量。
//! 持久化与调度在 `crates/server/src/jobs`（执行器、提交窗口、恢复）与
//! `crates/server/src/storage/repo`（SQL 原语）——两边共用这里的规则，
//! 避免"迁移/服务/测试各写一份 DAG 或退避序列"造成的漂移。
//!
//! 覆盖合同条目：
//! - 阶段 DAG 与依赖（[`stage_dependency`]）：解锁条件 = 依赖阶段全部 `succeeded`；
//! - 事件 → 状态转换表（[`next_stage_status`]）；
//! - 父 job 状态聚合（[`aggregate_job_status`]）；
//! - 安全重试上限与退避 2/4/8/16/32 秒 + jitter、`Retry-After` 上限（[`retry_delay_seconds`]）；
//! - 远端轮询 3 秒起逐步到 15 秒（[`poll_interval_seconds`]）；
//! - 总等待 30 分钟转 `needs_input`（[`remote_wait_exceeded`]，起点锚点见
//!   [`remote_wait_anchor_kind`] / [`remote_wait_anchor`]）；
//! - 并发分组（[`concurrency_group`]）与提交窗口形态（[`submission_style`]）。

use crate::domain::{JobStatus, ProviderAttempt, StageKind, SubmitState};
use crate::timestamps::Timestamp;

// ---------------------------------------------------------------------------
// 阶段 DAG
// ---------------------------------------------------------------------------

/// 一个阶段的依赖规则（DAG 的机器可读形式，contracts.md §5）。
///
/// 依赖关系必须落库（`job_stage_deps`），执行器用 SQL 判定"依赖是否全部 succeeded"；
/// 本枚举只描述规则，不持有运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageDependency {
    /// 无依赖：DAG 起点（`freeze_inputs`）。
    None,
    /// 固定依赖：同 job 内这些 kind 的**全部**阶段（含各批次）必须 succeeded。
    Fixed(&'static [StageKind]),
    /// 依赖同 job 内该 kind 的全部批次（`manual_merge` 等所有批次完成）。
    AllBatchesOf(StageKind),
}

/// 阶段的依赖规则。
///
/// ```text
/// freeze_inputs（入队事务）
///   ├─ manual_extract(batch 0..N-1) → manual_merge ─┐
///   └─ tripo_upload → tripo_submit → tripo_poll     │
///                    → model_download → model_validate ┤
///                                                     ↓
///                                               assemble_draft
/// ```
pub const fn stage_dependency(kind: StageKind) -> StageDependency {
    use StageKind::*;
    match kind {
        FreezeInputs => StageDependency::None,
        ManualExtract => StageDependency::Fixed(&[FreezeInputs]),
        ManualMerge => StageDependency::AllBatchesOf(ManualExtract),
        TripoUpload => StageDependency::Fixed(&[FreezeInputs]),
        TripoSubmit => StageDependency::Fixed(&[TripoUpload]),
        TripoPoll => StageDependency::Fixed(&[TripoSubmit]),
        ModelDownload => StageDependency::Fixed(&[TripoPoll]),
        ModelValidate => StageDependency::Fixed(&[ModelDownload]),
        AssembleDraft => StageDependency::Fixed(&[ManualMerge, ModelValidate]),
    }
}

/// DAG 中"远端生成链"的阶段（并发分组用）。
pub const REMOTE_GENERATION_STAGES: &[StageKind] = &[
    StageKind::TripoUpload,
    StageKind::TripoSubmit,
    StageKind::TripoPoll,
    StageKind::ModelDownload,
    StageKind::ModelValidate,
];

/// 并发分组（architecture.md §6：全局远端生成 2、说明书批次 2，可降低不可提高）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcurrencyGroup {
    /// 远端生成链（Tripo 上传→提交→查询→下载→校验）。
    RemoteGeneration,
    /// 说明书 AI 批次（`manual_extract` 的每个批次是独立执行单元，可并发）。
    ManualAiBatch,
}

/// 阶段所属的并发分组；`None` 表示本地阶段（不占远端并发额度）。
pub const fn concurrency_group(kind: StageKind) -> Option<ConcurrencyGroup> {
    match kind {
        StageKind::ManualExtract => Some(ConcurrencyGroup::ManualAiBatch),
        StageKind::TripoUpload
        | StageKind::TripoSubmit
        | StageKind::TripoPoll
        | StageKind::ModelDownload
        | StageKind::ModelValidate => Some(ConcurrencyGroup::RemoteGeneration),
        StageKind::FreezeInputs | StageKind::ManualMerge | StageKind::AssembleDraft => None,
    }
}

/// 付费提交窗口的形态（contracts.md §5 第 5 条：两个分支的恢复语义不同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionStyle {
    /// Tripo：返回远端 task ID，可查询；崩溃后按已知 task ID 继续查。
    AsyncRemoteTask,
    /// 说明书 AI（首版同步 Responses）：整批响应落库才算完成；
    /// `response_id` 不等于可轮询任务，**不提供** `attachRemoteTask` 恢复动作。
    SyncResponse,
}

impl SubmissionStyle {
    /// 线上取值（camelCase；T17 任务详情的 `submissionStyle` 用它，前端据此决定
    /// 对账面板是否提供 `attachRemoteTask`——同步链路不提供该动作）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AsyncRemoteTask => "asyncRemoteTask",
            Self::SyncResponse => "syncResponse",
        }
    }
}

/// 阶段是否需要"提交窗口"（先 intent、再 submitting、再事实观察）。
pub const fn submission_style(kind: StageKind) -> Option<SubmissionStyle> {
    match kind {
        StageKind::TripoSubmit => Some(SubmissionStyle::AsyncRemoteTask),
        StageKind::ManualExtract => Some(SubmissionStyle::SyncResponse),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 退避、轮询与总等待预算（contracts.md §5）
// ---------------------------------------------------------------------------

/// 安全重试上限（次）。超过 → `failed`，不无休止重试。
pub const MAX_SAFE_RETRIES: u32 = 5;
/// 退避基数（秒）：第 n 次安全重试等待 `RETRY_BACKOFF_SECONDS[n-1]` + jitter。
pub const RETRY_BACKOFF_SECONDS: [u64; MAX_SAFE_RETRIES as usize] = [2, 4, 8, 16, 32];
/// jitter 上限（占基数的比例）。加性 jitter，避免同刻重试的惊群。
pub const RETRY_JITTER_MAX_FRACTION: f64 = 0.25;
/// `Retry-After` 上限（秒）：尊重供应商指示的退避时间，但不接受无上限的延后。
pub const RETRY_AFTER_CAP_SECONDS: u64 = 300;
/// 远端轮询起始间隔（秒）。
pub const POLL_START_SECONDS: u64 = 3;
/// 远端轮询最大间隔（秒）。
pub const POLL_MAX_SECONDS: u64 = 15;
/// 总等待预算（秒）：自提交起超过后转 `needs_input` 并保留 task_id。
pub const REMOTE_WAIT_BUDGET_SECONDS: u64 = 1800;

/// 第 `retry_number` 次安全重试的等待秒数（1-based）+ jitter。
///
/// `jitter_fraction` 由调用方提供（0.0..=1.0 的样本），取
/// `[base, base * (1 + RETRY_JITTER_MAX_FRACTION)]`；测试传入固定样本可得到确定值。
/// `retry_number` 超过上限时返回最后一档（调用方在此之前就应转 `failed`）。
pub fn retry_delay_seconds(retry_number: u32, jitter_fraction: f64) -> u64 {
    let index = retry_number.saturating_sub(1).min(MAX_SAFE_RETRIES - 1) as usize;
    let base = RETRY_BACKOFF_SECONDS[index];
    let jitter = (base as f64) * RETRY_JITTER_MAX_FRACTION * jitter_fraction.clamp(0.0, 1.0);
    base + jitter.round() as u64
}

/// 是否还有安全重试额度：`retry_count` 为已用重试次数（0 起）。
pub const fn safe_retries_remain(retry_count: u32) -> bool {
    retry_count < MAX_SAFE_RETRIES
}

/// 尊重 `Retry-After`：返回实际等待秒数与"是否被上限截断"。
///
/// 供应商给出的秒数不超过上限时原样采用（不加 jitter）；超过上限时截断到
/// [`RETRY_AFTER_CAP_SECONDS`] 并标记（调用方必须记录，避免看起来"忽略了头部"）。
pub fn retry_after_seconds(header_seconds: u64) -> (u64, bool) {
    if header_seconds > RETRY_AFTER_CAP_SECONDS {
        (RETRY_AFTER_CAP_SECONDS, true)
    } else {
        (header_seconds, false)
    }
}

/// 第 `poll_count` 次后续查询的间隔（秒）：3 → 6 → 12 → 15 → 15 …（上限 15）。
pub fn poll_interval_seconds(poll_count: u32) -> u64 {
    let mut value = POLL_START_SECONDS;
    for _ in 0..poll_count {
        value = value.saturating_mul(2).min(POLL_MAX_SECONDS);
    }
    value
}

/// 是否已超过总等待预算（自首次提交起；`elapsed_seconds` 为已等待秒数）。
pub const fn remote_wait_exceeded(elapsed_seconds: u64) -> bool {
    elapsed_seconds >= REMOTE_WAIT_BUDGET_SECONDS
}

/// 远端等待链的**提交锚点阶段**：该阶段的远端事实由哪类阶段的 accepted 提交产生。
///
/// `tripo_poll` 及其下游（`model_download` / `model_validate`）自身**不建 attempt**
/// ——远端 task ID 与提交时刻都保存在同一 job 的 `tripo_submit` 已接受事实里
/// （T10 §T10-10 第 6 条、T12 适配器同构）。因此等待计时必须归到该提交事实上；
/// 只看"本阶段自己的 attempt"会让真实轮询链路上的 30 分钟预算恒不可达（BUG-003）。
pub const fn remote_wait_anchor_kind(kind: StageKind) -> Option<StageKind> {
    use StageKind::*;
    match kind {
        TripoSubmit | TripoPoll | ModelDownload | ModelValidate => Some(TripoSubmit),
        FreezeInputs | ManualExtract | ManualMerge | TripoUpload | AssembleDraft => None,
    }
}

/// 是否是可用于等待计时的远端事实：`accepted` 且带远端 task ID。
///
/// 只有这种事实证明"远端已有任务在跑"：intent/submitting/unknown 尚未证明被接受，
/// 同步链路（accepted 但只有 `response_id`）没有可等待的远端任务，都不能当起点。
pub const fn is_accepted_remote_fact(attempt: &ProviderAttempt) -> bool {
    matches!(attempt.submit_state, SubmitState::Accepted) && attempt.remote_task_id.is_some()
}

/// 远端等待起点：优先本阶段自己的 accepted 远端事实，否则回退到同一 job 的提交事实。
///
/// 回退分支覆盖真实轮询链路形态（轮询阶段没有自身 attempt）；两个候选都必须
/// 通过 [`is_accepted_remote_fact`]，未接受的 attempt 不会让预算提前起跑。
/// 归属范围由调用方限定为**同一 job**（见 `repo::attempts::latest_accepted_for_job`），
/// 多任务/多批次之间不会串用起点。
pub fn remote_wait_anchor<'a>(
    stage_attempt: Option<&'a ProviderAttempt>,
    job_submit_attempt: Option<&'a ProviderAttempt>,
) -> Option<&'a ProviderAttempt> {
    stage_attempt
        .filter(|attempt| is_accepted_remote_fact(attempt))
        .or_else(|| job_submit_attempt.filter(|attempt| is_accepted_remote_fact(attempt)))
}

/// 自等待起点（`started_at`）起的已等待秒数（向下取整到秒；时钟回拨按 0 计）。
pub const fn waited_seconds(started_at: Timestamp, now: Timestamp) -> u64 {
    let delta_millis = now.as_millis() - started_at.as_millis();
    if delta_millis <= 0 {
        0
    } else {
        (delta_millis / 1000) as u64
    }
}

// ---------------------------------------------------------------------------
// 事件 → 状态转换（contracts.md §5 表格）
// ---------------------------------------------------------------------------

/// 作用在**阶段**（或父 job）上的事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageEvent {
    /// 领取阶段：原子取得新 leaseEpoch（queued / 到期 retry_wait / 到期 waiting_provider）。
    Claimed,
    /// 已提交远端 ID：`running → waiting_provider`，轮询由 `next_run_at` 驱动。
    RemoteAccepted,
    /// 远端仍在进行：保持 `waiting_provider`，只更新 `next_run_at`。
    ProviderPending,
    /// 安全临时失败（429/5xx/超时）：→ `retry_wait`，超上限 → `failed`。
    SafeTemporaryFailure,
    /// 安全重试次数用尽：→ `failed`。
    RetryExhausted,
    /// 资料／schema／支持能力不足：→ `needs_input`，列出可行动缺项。
    InputsInsufficient,
    /// 付费创建结果未知：→ `submission_unknown`，暂停该分支后续购买。
    PaymentResultUnknown,
    /// 阶段产物校验成功：→ `succeeded`（只解锁依赖全部完成的阶段）。
    ArtifactValidated,
    /// 取消：→ `cancelled`（已提交阶段的收尾语义见执行器）。
    Cancelled,
}

/// 事件 → 新状态；非法转换返回 `None`（调用方不得静默忽略，应记录并拒绝写入）。
///
/// 与 contracts.md §5 的表格一一对应；`Cancelled` 只允许从非终态进入。
pub const fn next_stage_status(current: JobStatus, event: StageEvent) -> Option<JobStatus> {
    // 两个枚举都有 `Cancelled`／`Succeeded` 等同名变体，这里显式限定来源，避免歧义。
    use JobStatus as S;
    use StageEvent as E;
    match event {
        E::Claimed => match current {
            S::Queued | S::RetryWait | S::WaitingProvider => Some(S::Running),
            _ => None,
        },
        E::RemoteAccepted | E::ProviderPending => match current {
            S::Running | S::WaitingProvider => Some(S::WaitingProvider),
            _ => None,
        },
        E::SafeTemporaryFailure => match current {
            S::Running => Some(S::RetryWait),
            _ => None,
        },
        E::RetryExhausted => match current {
            S::Running | S::RetryWait => Some(S::Failed),
            _ => None,
        },
        E::InputsInsufficient => match current {
            S::Running | S::WaitingProvider | S::RetryWait => Some(S::NeedsInput),
            _ => None,
        },
        E::PaymentResultUnknown => match current {
            S::Running | S::WaitingProvider | S::RetryWait => Some(S::SubmissionUnknown),
            _ => None,
        },
        E::ArtifactValidated => match current {
            S::Running | S::WaitingProvider => Some(S::Succeeded),
            _ => None,
        },
        E::Cancelled => match current {
            S::Succeeded | S::Failed | S::Cancelled => None,
            _ => Some(JobStatus::Cancelled),
        },
    }
}

/// 父 job 状态聚合（contracts.md §5「job／stage 状态枚举」表的父 job 行）。
///
/// 优先级（RD 取舍，见 decisions.md ADR-020）：
/// `终态（succeeded/failed/cancelled）保持` → `submission_unknown` → `needs_input`
/// → `failed` → 全部 `succeeded` → `waiting_provider` → `running` → `queued`。
///
/// 理由：unknown 需要管理员对账（并暂停后续购买），needs_input 需要用户补齐输入，
/// 两者都必须在任务中心优先展示；`failed` 不阻塞独立分支继续完成（只改展示状态）。
pub fn aggregate_job_status(current: JobStatus, stages: &[JobStatus]) -> JobStatus {
    use JobStatus::*;
    if current.is_terminal() {
        return current;
    }
    if stages.is_empty() {
        return current;
    }
    if stages.contains(&SubmissionUnknown) {
        return SubmissionUnknown;
    }
    if stages.contains(&NeedsInput) {
        return NeedsInput;
    }
    if stages.contains(&Failed) {
        return Failed;
    }
    if stages.iter().all(|status| *status == Succeeded) {
        return Succeeded;
    }
    if stages.contains(&WaitingProvider) {
        return WaitingProvider;
    }
    // 有阶段在跑（或等退避）→ running；从未开始的 job（全部 queued）→ queued；
    // 其余（已有阶段完成、后续阶段排队）也算 running：任务已经在推进。
    if stages
        .iter()
        .any(|status| matches!(status, Running | RetryWait))
    {
        return Running;
    }
    if stages.iter().all(|status| *status == Queued) {
        return Queued;
    }
    Running
}

/// 取消时哪些阶段状态应直接转 `cancelled`（contracts.md §5：未提交阶段 cancelled）。
///
/// `waiting_provider`（已提交远端）与 `submission_unknown`（可能已收费）保持原状态，
/// 由 T15 的收尾／对账流程处理；`running` 由调用方结合 attempt 事实判断
/// （见 `repo::job_stages::cancel_unsubmitted_for_job`）。
pub const fn cancellable_stage_status(status: JobStatus) -> bool {
    matches!(
        status,
        JobStatus::Queued | JobStatus::RetryWait | JobStatus::NeedsInput
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dag_matches_contract_diagram() {
        use StageKind::*;
        assert_eq!(stage_dependency(FreezeInputs), StageDependency::None);
        assert_eq!(
            stage_dependency(ManualExtract),
            StageDependency::Fixed(&[FreezeInputs])
        );
        assert_eq!(
            stage_dependency(ManualMerge),
            StageDependency::AllBatchesOf(ManualExtract)
        );
        assert_eq!(
            stage_dependency(TripoSubmit),
            StageDependency::Fixed(&[TripoUpload])
        );
        assert_eq!(
            stage_dependency(ModelValidate),
            StageDependency::Fixed(&[ModelDownload])
        );
        assert_eq!(
            stage_dependency(AssembleDraft),
            StageDependency::Fixed(&[ManualMerge, ModelValidate])
        );
    }

    #[test]
    fn concurrency_groups_and_submission_styles() {
        assert_eq!(
            concurrency_group(StageKind::ManualExtract),
            Some(ConcurrencyGroup::ManualAiBatch)
        );
        for kind in REMOTE_GENERATION_STAGES {
            assert_eq!(
                concurrency_group(*kind),
                Some(ConcurrencyGroup::RemoteGeneration)
            );
        }
        assert_eq!(concurrency_group(StageKind::AssembleDraft), None);
        assert_eq!(
            submission_style(StageKind::TripoSubmit),
            Some(SubmissionStyle::AsyncRemoteTask)
        );
        assert_eq!(
            submission_style(StageKind::ManualExtract),
            Some(SubmissionStyle::SyncResponse)
        );
        assert_eq!(submission_style(StageKind::TripoPoll), None);
        // 线上取值（T17：前端按它决定对账面板是否提供 attachRemoteTask）。
        assert_eq!(SubmissionStyle::AsyncRemoteTask.as_str(), "asyncRemoteTask");
        assert_eq!(SubmissionStyle::SyncResponse.as_str(), "syncResponse");
    }

    #[test]
    fn backoff_sequence_respects_base_and_jitter_bounds() {
        for (index, base) in RETRY_BACKOFF_SECONDS.iter().enumerate() {
            let retry_number = index as u32 + 1;
            let no_jitter = retry_delay_seconds(retry_number, 0.0);
            assert_eq!(no_jitter, *base, "第 {retry_number} 次重试基数");
            let full_jitter = retry_delay_seconds(retry_number, 1.0);
            assert_eq!(
                full_jitter,
                *base + (*base as f64 * RETRY_JITTER_MAX_FRACTION).round() as u64
            );
            assert!(full_jitter >= *base);
            // 越界的 jitter 样本被夹紧，不会产生负值或超额。
            assert_eq!(retry_delay_seconds(retry_number, -1.0), *base);
            assert_eq!(retry_delay_seconds(retry_number, 9.0), full_jitter);
        }
        assert_eq!(RETRY_BACKOFF_SECONDS, [2, 4, 8, 16, 32]);
        assert!(retry_delay_seconds(9, 0.0) == 32, "超出上限取最后一档");
    }

    #[test]
    fn retry_budget_and_retry_after_cap() {
        assert!(safe_retries_remain(0));
        assert!(safe_retries_remain(MAX_SAFE_RETRIES - 1));
        assert!(!safe_retries_remain(MAX_SAFE_RETRIES));
        assert_eq!(retry_after_seconds(3), (3, false));
        assert_eq!(retry_after_seconds(0), (0, false));
        assert_eq!(
            retry_after_seconds(RETRY_AFTER_CAP_SECONDS + 1),
            (RETRY_AFTER_CAP_SECONDS, true)
        );
    }

    #[test]
    fn poll_ramp_and_wait_budget() {
        let sequence: Vec<u64> = (0..5).map(poll_interval_seconds).collect();
        assert_eq!(sequence, [3, 6, 12, 15, 15]);
        assert!(!remote_wait_exceeded(REMOTE_WAIT_BUDGET_SECONDS - 1));
        assert!(remote_wait_exceeded(REMOTE_WAIT_BUDGET_SECONDS));
    }

    /// 构造 attempt 事实（锚点用例；其余列不参与判定）。
    fn attempt(
        id: &str,
        state: SubmitState,
        remote_task_id: Option<&str>,
        started_at_millis: i64,
    ) -> ProviderAttempt {
        let at = Timestamp::from_millis(started_at_millis);
        ProviderAttempt {
            id: id.to_owned(),
            job_id: "job-1".to_owned(),
            stage_id: "stage-1".to_owned(),
            request_hash: "hash".to_owned(),
            submit_state: state,
            remote_task_id: remote_task_id.map(str::to_owned),
            response_id: None,
            started_at: at,
            last_error: None,
            created_at: at,
            updated_at: at,
        }
    }

    #[test]
    fn wait_anchor_kind_maps_remote_chain_to_submit() {
        use StageKind::*;
        assert_eq!(remote_wait_anchor_kind(TripoSubmit), Some(TripoSubmit));
        assert_eq!(remote_wait_anchor_kind(TripoPoll), Some(TripoSubmit));
        assert_eq!(remote_wait_anchor_kind(ModelDownload), Some(TripoSubmit));
        assert_eq!(remote_wait_anchor_kind(ModelValidate), Some(TripoSubmit));
        for kind in [
            FreezeInputs,
            ManualExtract,
            ManualMerge,
            TripoUpload,
            AssembleDraft,
        ] {
            assert_eq!(
                remote_wait_anchor_kind(kind),
                None,
                "{kind:?} 不参与远端等待"
            );
        }
    }

    #[test]
    fn wait_anchor_prefers_own_accepted_fact_and_falls_back_to_job_submit() {
        let accepted_submit = attempt("submit", SubmitState::Accepted, Some("task-1"), 1_000_000);
        let accepted_poll = attempt("poll", SubmitState::Accepted, Some("task-1"), 2_000_000);
        let intent = attempt("intent", SubmitState::Intent, None, 3_000_000);
        let accepted_sync = attempt("sync", SubmitState::Accepted, None, 4_000_000);

        // 真实形态（BUG-003）：轮询阶段没有自身 attempt → 回退到 job 提交事实。
        assert_eq!(
            remote_wait_anchor(None, Some(&accepted_submit)).map(|a| a.id.as_str()),
            Some("submit")
        );
        // 本阶段自己的 accepted 事实优先（提交阶段本体 / 已带远端事实的阶段）。
        assert_eq!(
            remote_wait_anchor(Some(&accepted_poll), Some(&accepted_submit)).map(|a| a.id.as_str()),
            Some("poll")
        );
        // 未接受的尝试不能当起点：继续回退；没有回退事实则返回 None（不计时）。
        assert_eq!(
            remote_wait_anchor(Some(&intent), Some(&accepted_submit)).map(|a| a.id.as_str()),
            Some("submit")
        );
        assert!(remote_wait_anchor(Some(&intent), None).is_none());
        // 同步链路 accepted 无远端 task ID：不构成等待链。
        assert!(remote_wait_anchor(Some(&accepted_sync), Some(&accepted_sync)).is_none());
        assert!(remote_wait_anchor(None, None).is_none());
        assert!(is_accepted_remote_fact(&accepted_submit));
        assert!(!is_accepted_remote_fact(&intent));
        assert!(!is_accepted_remote_fact(&accepted_sync));
    }

    #[test]
    fn waited_seconds_never_negative() {
        let start = Timestamp::from_millis(1_000_000);
        assert_eq!(waited_seconds(start, Timestamp::from_millis(1_000_000)), 0);
        assert_eq!(waited_seconds(start, Timestamp::from_millis(1_000_999)), 0);
        assert_eq!(waited_seconds(start, Timestamp::from_millis(1_001_000)), 1);
        assert_eq!(
            waited_seconds(start, Timestamp::from_millis(1_000_000 + 1_800_000)),
            REMOTE_WAIT_BUDGET_SECONDS
        );
        assert_eq!(
            waited_seconds(start, Timestamp::from_millis(999_999)),
            0,
            "时钟回拨不产生负等待"
        );
    }

    #[test]
    fn transition_table_matches_contract() {
        use JobStatus as S;
        use StageEvent as E;
        assert_eq!(next_stage_status(S::Queued, E::Claimed), Some(S::Running));
        assert_eq!(
            next_stage_status(S::RetryWait, E::Claimed),
            Some(S::Running)
        );
        assert_eq!(
            next_stage_status(S::WaitingProvider, E::Claimed),
            Some(S::Running)
        );
        assert_eq!(
            next_stage_status(S::Running, E::Claimed),
            None,
            "不得重复领取"
        );
        assert_eq!(next_stage_status(S::Succeeded, E::Claimed), None);

        assert_eq!(
            next_stage_status(S::Running, E::RemoteAccepted),
            Some(S::WaitingProvider)
        );
        assert_eq!(
            next_stage_status(S::WaitingProvider, E::ProviderPending),
            Some(S::WaitingProvider)
        );
        assert_eq!(
            next_stage_status(S::Running, E::SafeTemporaryFailure),
            Some(S::RetryWait)
        );
        assert_eq!(
            next_stage_status(S::Running, E::RetryExhausted),
            Some(S::Failed)
        );
        assert_eq!(
            next_stage_status(S::Running, E::InputsInsufficient),
            Some(S::NeedsInput)
        );
        assert_eq!(
            next_stage_status(S::Running, E::PaymentResultUnknown),
            Some(S::SubmissionUnknown)
        );
        assert_eq!(
            next_stage_status(S::Running, E::ArtifactValidated),
            Some(S::Succeeded)
        );
        assert_eq!(
            next_stage_status(S::Running, E::Cancelled),
            Some(S::Cancelled)
        );
        assert_eq!(next_stage_status(S::Succeeded, E::Cancelled), None);
        assert_eq!(next_stage_status(S::Failed, E::Cancelled), None);
    }

    #[test]
    fn parent_job_status_priority_and_branch_isolation() {
        use JobStatus::*;
        // 全部阶段成功 → 父 job succeeded（"最后组装完成"）。
        assert_eq!(
            aggregate_job_status(Running, &[Succeeded, Succeeded]),
            Succeeded
        );
        // unknown > needs_input > failed（展示优先级，见 ADR-020）。
        assert_eq!(
            aggregate_job_status(Running, &[SubmissionUnknown, Failed]),
            SubmissionUnknown
        );
        assert_eq!(
            aggregate_job_status(Running, &[NeedsInput, Failed, Succeeded]),
            NeedsInput
        );
        assert_eq!(aggregate_job_status(Running, &[Failed, Succeeded]), Failed);
        // 部分分支失败不阻塞独立分支：仍有 running 时父 job 保持可读状态。
        assert_eq!(
            aggregate_job_status(Failed, &[Failed, Running]),
            Failed,
            "终态（failed）不被内部推进改写"
        );
        assert_eq!(
            aggregate_job_status(Running, &[WaitingProvider, Succeeded]),
            WaitingProvider
        );
        assert_eq!(aggregate_job_status(Running, &[Running, Queued]), Running);
        assert_eq!(aggregate_job_status(Running, &[Queued, Queued]), Queued);
        assert_eq!(
            aggregate_job_status(Queued, &[Succeeded, Queued]),
            Running,
            "已有阶段完成、后续阶段排队 → 任务已在推进"
        );
        // 终态保护：已取消/已成功的 job 不被后续聚合改写。
        assert_eq!(aggregate_job_status(Cancelled, &[Succeeded]), Cancelled);
        assert_eq!(aggregate_job_status(Succeeded, &[Running]), Succeeded);
        // 无阶段时保持现状（不凭空判定成功）。
        assert_eq!(aggregate_job_status(Queued, &[]), Queued);
    }

    #[test]
    fn only_unsubmitted_statuses_are_cancellable() {
        use JobStatus::*;
        assert!(cancellable_stage_status(Queued));
        assert!(cancellable_stage_status(RetryWait));
        assert!(cancellable_stage_status(NeedsInput));
        assert!(!cancellable_stage_status(WaitingProvider));
        assert!(!cancellable_stage_status(SubmissionUnknown));
        assert!(!cancellable_stage_status(Running));
    }
}
