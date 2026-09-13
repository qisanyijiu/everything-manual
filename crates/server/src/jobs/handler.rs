//! 阶段处理器接口与结果类型（T10）。
//!
//! 真实适配器（T12 的 Tripo、T14 的说明书 AI）与测试用的 fixture 阶段实现**同一接口**：
//! 处理器只负责"做什么"（上传、提交、查询、下载、提取、组装），
//! **不负责**推进业务状态——状态推进由执行器在租约 epoch 条件下完成
//! （contracts.md §5 提交窗口第 4 步），因此过期 worker 即使跑完处理器也无法解锁后续阶段。
//!
//! 处理器需要发出付费请求时使用 [`StageContext::submission`]（提交窗口），
//! 而不是自己写 attempt 行：窗口内部固定了"先 intent、再 submitting、再事实观察"的顺序。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;

use manual_core::domain::{Job, JobStage, ProviderAttempt, StageKind};
use manual_core::timestamps::Timestamp;

use super::{JobError, submission::SubmissionWindow};

/// 处理器返回的 future（避免为 `dyn` 引入 async-trait 依赖）。
pub type StageFuture<'a> =
    Pin<Box<dyn Future<Output = Result<StageOutcome, JobError>> + Send + 'a>>;

/// 单个阶段类型的处理器。
pub trait StageHandler: Send + Sync {
    /// 执行阶段动作。可以读写数据库（快照、资产、结果事实），但**不得**直接推进
    /// 阶段业务状态（那是执行器的职责，带租约 guard）。
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a>;
}

/// 阶段类型 → 处理器。生产启动时按已配置的 Provider 注册（T12/T14/T15）；
/// 未注册的阶段不会被静默跳过：执行器把该阶段延后并记录原因（不假成功）。
#[derive(Default)]
pub struct StageRegistry {
    handlers: HashMap<StageKind, Arc<dyn StageHandler>>,
}

impl StageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<H: StageHandler + 'static>(
        &mut self,
        kind: StageKind,
        handler: H,
    ) -> &mut Self {
        self.handlers.insert(kind, Arc::new(handler));
        self
    }

    pub fn get(&self, kind: StageKind) -> Option<Arc<dyn StageHandler>> {
        self.handlers.get(&kind).cloned()
    }

    pub fn contains(&self, kind: StageKind) -> bool {
        self.handlers.contains_key(&kind)
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// 已注册的阶段类型（稳定顺序，供日志与 `check` 展示）。
    pub fn registered(&self) -> Vec<StageKind> {
        let mut kinds: Vec<StageKind> = self.handlers.keys().copied().collect();
        kinds.sort_by_key(|kind| kind.as_str());
        kinds
    }
}

/// 可行动缺项（`needs_input` 时逐条展示给用户，REQ-024 / UI-031）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingItem {
    /// 稳定代码（前端按它给"去补齐"入口，例如 `missing_photo_view`）。
    pub code: String,
    /// 面向用户的说明（不含内部路径与密钥）。
    pub message: String,
}

impl MissingItem {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// 阶段执行结果（执行器据此选择状态转换；所有业务推进都带租约 epoch guard）。
#[derive(Debug, Clone, PartialEq)]
pub enum StageOutcome {
    /// 产物校验成功（只解锁依赖全部完成的阶段）。
    Succeeded {
        result_asset_id: Option<String>,
        /// JSON：用量／计费摘要（不保存密钥与完整响应）。
        usage: Option<Value>,
    },
    /// 远端仍在进行（或已提交远端 ID）：`waiting_provider` + `next_run_at`（轮询节奏）。
    WaitingProvider,
    /// 安全临时失败（429/5xx/超时/断连）：`retry_wait`，超上限 → `failed`。
    Retryable {
        reason: String,
        /// 服务器给出的 `Retry-After` 秒数（如有）：尊重它（上限见核心策略）。
        retry_after_seconds: Option<u64>,
    },
    /// 资料／schema／支持能力不足：`needs_input`，逐条列出可行动缺项。
    NeedsInput { items: Vec<MissingItem> },
    /// 付费创建结果未知：`submission_unknown`（暂停该分支后续购买，等待管理员对账）。
    SubmissionUnknown { reason: String },
    /// **明确失败**（适配器能证明不可重试，例如供应商返回"任务永久失败"）：
    /// 直接 `failed`，不消耗安全重试额度。自动重试只走 [`StageOutcome::Retryable`]。
    Failed { reason: String },
}

impl StageOutcome {
    /// 便捷构造：成功且无产物资产。
    pub fn succeeded() -> Self {
        Self::Succeeded {
            result_asset_id: None,
            usage: None,
        }
    }

    pub fn needs_input(items: Vec<MissingItem>) -> Self {
        Self::NeedsInput { items }
    }

    /// 结果事实（用于日志摘要）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Succeeded { .. } => "succeeded",
            Self::WaitingProvider => "waitingProvider",
            Self::Retryable { .. } => "retryable",
            Self::NeedsInput { .. } => "needsInput",
            Self::SubmissionUnknown { .. } => "submissionUnknown",
            Self::Failed { .. } => "failed",
        }
    }
}

/// 领取阶段时的恢复提示（来自 attempt 事实；contracts.md §5 第 5 条）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeHint {
    /// 没有未决事实：正常执行（含"intent 尚未标记 submitting 可安全领取"）。
    Fresh,
    /// 已有远端 task ID：**继续查询**，绝不重新提交付费请求。
    HasRemoteTask {
        attempt_id: String,
        remote_task_id: String,
    },
    /// 该阶段的付费结果未知：执行器直接落 `submission_unknown`，不调用处理器。
    SubmissionUnknown { attempt_id: String, reason: String },
}

/// 一次阶段执行的全部上下文。
pub struct StageContext {
    /// 连接池：处理器读快照/资产、写结果事实用（短事务，不跨 HTTP 持有）。
    pub pool: SqlitePool,
    pub job: Job,
    /// 已领取的阶段快照（`lease_owner`/`lease_epoch` 即当前租约）。
    pub stage: JobStage,
    /// 本次执行的时钟快照（执行器统一取一次，避免同一次执行内时间跳变）。
    pub now: Timestamp,
    pub resume: ResumeHint,
    /// 该阶段最近一次 attempt（可能是未决事实）。
    pub attempt: Option<ProviderAttempt>,
    /// 付费提交窗口（仅付费阶段使用；非付费阶段不调用即可）。
    pub submission: SubmissionWindow,
}

impl StageContext {
    /// 当前租约 epoch（业务推进由执行器校验，处理器只用于日志）。
    pub fn lease_epoch(&self) -> i64 {
        self.stage.lease_epoch
    }

    /// 本次执行是否应"继续查询已知远端任务"。
    pub fn known_remote_task_id(&self) -> Option<&str> {
        match &self.resume {
            ResumeHint::HasRemoteTask { remote_task_id, .. } => Some(remote_task_id),
            _ => None,
        }
    }
}
