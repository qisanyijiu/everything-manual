//! 持久任务执行器（T10 / REQ-024；contracts.md §5）。
//!
//! 设计要点（architecture.md §6）：
//! - **SQLite 是任务事实来源**：领取、租约、状态推进、attempt 事实都在数据库里；
//!   Tokio channel（[`JobExecutor::wake`]）只用来叫醒 worker，**不是队列**；
//! - **事务短小**：领取、事实写入、业务推进各自一个短事务，HTTPS 请求期间不持有事务；
//! - **租约 epoch**：领取即 `lease_epoch + 1`；所有业务推进带 guard
//!   （`status='running' AND lease_owner=? AND lease_epoch=? AND lease_until > now`）；
//!   租约过期的 worker 允许保存不可变事实（receipt／结果资产），但不得推进状态、
//!   不得解锁后续阶段；
//! - **付费提交窗口**（[`SubmissionWindow`]）：先 intent、再 submitting、再事实观察，
//!   业务推进另用当前 epoch；结果未知（`submission_unknown`）不自动重购；
//! - **恢复矩阵**（[`recover`]）：启动与每个 tick 收敛租约过期的 `running` 阶段：
//!   intent 未标记 submitting → 可安全重领；submitting 且无远端事实 → unknown；
//!   已有远端 ID → 继续查询（绝不重发付费 POST）；结果已落库而 checkpoint 未推进 →
//!   校验后补推进，不重新付费；
//! - **failpoint 仅测试构建**（[`failpoints`]）：`job-failpoints` feature 门控，
//!   生产路径不存在该分支（证据见 implementation.md §T10）。
//!
//! 边界：T10 用 fixture 阶段验证执行器机制，T12/T14 接通真实适配器；T15 补齐
//! 两条分支的组装（[`pipeline`]）与任务控制端点（[`control`]：取消/重试/对账）。

pub mod control;
pub mod executor;
pub mod failpoints;
pub mod handler;
pub mod pipeline;
pub mod recover;
pub mod submission;

pub use control::{
    CANCEL_NOTICE, CancelReport, JobControlError, ReconcileAction, ReconcileReport,
    ReconcileRequest, RetryReport,
};
pub use executor::{ExecutorConfig, ExecutorHandle, JobExecutor, StageRunReport, TickOutcome};
pub use handler::{
    MissingItem, ResumeHint, StageContext, StageFuture, StageHandler, StageOutcome, StageRegistry,
};
pub use pipeline::PipelineHandlers;
pub use recover::{RecoveryAction, RecoveryReport};
pub use submission::{RemoteTaskObservation, SubmissionWindow};

use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};

use manual_core::timestamps::Timestamp;

use crate::storage::StorageError;

/// 执行器错误：存储错误 + 处理器错误（后者只在日志与报告中出现，不冒充业务状态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    Storage(StorageError),
    /// 阶段处理器自身失败（例如 fixture/适配器返回无法解析的响应）。
    /// 不直接改业务状态：由处理器决定返回 `Retryable`／`NeedsInput`／`SubmissionUnknown`。
    Handler {
        stage_id: String,
        message: String,
    },
    /// 执行器配置非法（不应发生在成功启动之后）。
    Config {
        message: String,
    },
}

impl JobError {
    pub fn handler(stage_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Handler {
            stage_id: stage_id.into(),
            message: message.into(),
        }
    }

    /// 日志用的稳定标识（不含用户数据）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Storage(_) => "storage",
            Self::Handler { .. } => "handler",
            Self::Config { .. } => "config",
        }
    }
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "存储错误：{error}"),
            Self::Handler { stage_id, message } => {
                write!(f, "阶段处理器失败（stage={stage_id}）：{message}")
            }
            Self::Config { message } => write!(f, "执行器配置错误：{message}"),
        }
    }
}

impl std::error::Error for JobError {}

impl From<StorageError> for JobError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::Error> for JobError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(StorageError::from(error))
    }
}

impl From<sqlx::migrate::MigrateError> for JobError {
    fn from(error: sqlx::migrate::MigrateError) -> Self {
        Self::Storage(StorageError::from(error))
    }
}

/// 时钟抽象：执行器从不直接读系统时间，便于测试用 [`ManualClock`] 精确驱动
/// 租约过期、退避与 30 分钟总等待。
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Timestamp;
}

/// 系统时钟（生产路径）。
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// 可调时钟（**仅测试与开发注入使用**，与 T06 的 `SpaceProbe::scripted` 同类）。
#[derive(Debug, Default)]
pub struct ManualClock {
    millis: AtomicI64,
}

impl ManualClock {
    pub fn new(now: Timestamp) -> Self {
        Self {
            millis: AtomicI64::new(now.as_millis()),
        }
    }

    pub fn set(&self, now: Timestamp) {
        self.millis.store(now.as_millis(), Ordering::SeqCst);
    }

    pub fn advance_millis(&self, millis: i64) {
        self.millis.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_millis(self.millis.load(Ordering::SeqCst))
    }
}

/// jitter 采样源：返回 `0.0..=1.0`（退避的加性 jitter 比例）。
pub trait Jitter: Send + Sync + 'static {
    fn sample(&self) -> f64;
}

/// 系统 jitter：用单调纳秒的低位做廉价散列（不需要密码学随机）。
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemJitter;

impl Jitter for SystemJitter {
    fn sample(&self) -> f64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos())
            .unwrap_or_default();
        // splitmix64 低位 → [0,1)
        let mut x = u64::from(nanos).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        (x >> 11) as f64 / (1_u64 << 53) as f64
    }
}

/// 固定 jitter（测试用：0.0 = 无 jitter，基数可精确断言）。
#[derive(Debug, Clone, Copy)]
pub struct FixedJitter(pub f64);

impl Jitter for FixedJitter {
    fn sample(&self) -> f64 {
        self.0.clamp(0.0, 1.0)
    }
}

/// 新的 worker 身份（写入 `lease_owner`，仅用于诊断；正确性由 `lease_epoch` 保证）。
pub fn worker_owner() -> String {
    format!("{}:{}", std::process::id(), manual_core::ids::new_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_and_jitter_are_deterministic() {
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        assert_eq!(clock.now().as_millis(), 1_000);
        clock.advance_millis(500);
        assert_eq!(clock.now().as_millis(), 1_500);
        clock.set(Timestamp::from_millis(42));
        assert_eq!(clock.now().as_millis(), 42);

        let jitter = FixedJitter(0.5);
        assert_eq!(jitter.sample(), 0.5);
        assert_eq!(FixedJitter(5.0).sample(), 1.0);
        assert_eq!(FixedJitter(-1.0).sample(), 0.0);
        let system = SystemJitter.sample();
        assert!(
            (0.0..1.0).contains(&system),
            "jitter 样本应在 [0,1)：{system}"
        );
    }
}
