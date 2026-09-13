//! 崩溃断点注入（**仅测试构建可用**）。
//!
//! 门控方式：Cargo feature `job-failpoints`。生产构建（`cargo build`／`cargo xtask dist`）
//! **不启用**该 feature，`job_failpoint!` 宏展开为空语句，二进制中不存在断点分支与
//! 相关字符串（证据见 implementation.md §T10 的 `strings`/`nm` 对比）。
//! feature 只由 `[dev-dependencies]` 的自引用开启（测试目标），不进入发布路径。
//!
//! 用法：
//! - 进程内测试：[`set`] / [`clear`] 注册断点动作；
//! - 子进程（进程级 SIGKILL 演示）：环境变量 `EM_TEST_FAILPOINT=<name>:<action>`
//!   （例如 `paid_after_submitting_before_request:hang:60000`），不需要在测试里改代码。
//!
//! 断点清单（与 validation-release.md §3 的崩溃断点对应，覆盖 ≥3 个）：
//! 1. [`PAID_AFTER_INTENT_BEFORE_SUBMITTING`]：付费 POST 发出前（intent 已持久化）；
//! 2. [`PAID_AFTER_SUBMITTING_BEFORE_REQUEST`]：已标记 submitting、请求尚未发出；
//! 3. [`PAID_AFTER_RESPONSE_BEFORE_RECEIPT`]：供应商已接受但响应事实未落库；
//! 4. [`PAID_AFTER_RECEIPT_BEFORE_ADVANCE`]：task ID 已落库但业务状态未推进；
//! 5. [`MANUAL_AFTER_REQUEST_BEFORE_RESPONSE`]：同步批次请求已发、完整响应未持久化；
//! 6. [`RESULT_FACT_BEFORE_CHECKPOINT`]：结果事实已持久化、checkpoint 未推进。

#[cfg(feature = "job-failpoints")]
mod enabled {
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    /// 断点动作。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FailpointAction {
        /// 在当前位置 panic（进程内测试可捕获；async 任务会以 panic 结束）。
        Panic,
        /// `std::process::abort()`（模拟硬崩溃；仅子进程演示使用）。
        Abort,
        /// `std::process::exit(code)`（仅子进程演示使用）。
        Exit(i32),
        /// 挂起指定毫秒（给外部 `kill -9` 留出窗口）。
        Hang(u64),
    }

    /// 断点清单（实现文件内使用常量而非字符串字面量，避免拼写漂移）。
    pub const PAID_AFTER_INTENT_BEFORE_SUBMITTING: &str = "paid_after_intent_before_submitting";
    pub const PAID_AFTER_SUBMITTING_BEFORE_REQUEST: &str = "paid_after_submitting_before_request";
    pub const PAID_AFTER_RESPONSE_BEFORE_RECEIPT: &str = "paid_after_response_before_receipt";
    pub const PAID_AFTER_RECEIPT_BEFORE_ADVANCE: &str = "paid_after_receipt_before_advance";
    pub const MANUAL_AFTER_REQUEST_BEFORE_RESPONSE: &str = "manual_after_request_before_response";
    pub const RESULT_FACT_BEFORE_CHECKPOINT: &str = "result_fact_before_checkpoint";
    /// T11：建单事务内、提交前的断点（快照 + 预留 + job + 阶段 + 幂等记录已写入，
    /// 事务尚未提交）——用于验证"事务中断不留半笔预留"。
    pub const GENERATION_BEFORE_COMMIT: &str = "generation_after_reserve_before_commit";

    /// 环境变量（子进程演示）。
    pub const ENV_FAILPOINT: &str = "EM_TEST_FAILPOINT";

    /// 注册表按 **worker owner**（`JobExecutor::owner()`）分区：同一进程里的多个
    /// 执行器/测试互不干扰（各自只命中自己注册的断点）。`"*"` 表示匹配任意 owner。
    pub type RegistryKey = (String, String);

    static REGISTRY: OnceLock<Mutex<BTreeMap<RegistryKey, FailpointAction>>> = OnceLock::new();

    fn registry() -> &'static Mutex<BTreeMap<RegistryKey, FailpointAction>> {
        REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
    }

    /// 注册（或覆盖）某个 worker 的断点动作。
    pub fn set(owner: &str, name: &str, action: FailpointAction) {
        registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert((owner.to_owned(), name.to_owned()), action);
    }

    /// 清空全部注册（测试收尾用；避免用例间互相影响）。
    pub fn clear() {
        registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    /// 只清空某个 worker 的注册（并行测试收尾用：不要抹掉其他执行器的断点）。
    pub fn clear_owner(owner: &str) {
        registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|(registered_owner, _), _| registered_owner != owner);
    }

    /// 命中断点：先查本 worker 的注册，再查 `"*"`，最后读 `EM_TEST_FAILPOINT`。
    ///
    /// 未命中时是空操作（生产路径没有该 feature，连函数都不存在）。
    pub fn hit(owner: &str, name: &str) {
        let registered = {
            let registry = registry()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            registry
                .get(&(owner.to_owned(), name.to_owned()))
                .or_else(|| registry.get(&("*".to_owned(), name.to_owned())))
                .copied()
        };
        let Some(action) = registered.or_else(|| action_from_env(name)) else {
            return;
        };
        match action {
            FailpointAction::Panic => panic!("命中测试断点 {name}（job-failpoints feature）"),
            FailpointAction::Abort => std::process::abort(),
            FailpointAction::Exit(code) => std::process::exit(code),
            FailpointAction::Hang(millis) => std::thread::sleep(Duration::from_millis(millis)),
        }
    }

    /// `EM_TEST_FAILPOINT=<name>[:<action>[:<millis>]]`；名字不匹配返回 `None`。
    fn action_from_env(name: &str) -> Option<FailpointAction> {
        let value = std::env::var(ENV_FAILPOINT).ok()?;
        let mut parts = value.split(':');
        let target = parts.next().unwrap_or_default().trim();
        if target != name {
            return None;
        }
        match parts.next().map(str::trim) {
            None | Some("") | Some("panic") => Some(FailpointAction::Panic),
            Some("abort") => Some(FailpointAction::Abort),
            Some("exit") => Some(FailpointAction::Exit(90)),
            Some("hang") => {
                let millis = parts
                    .next()
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .unwrap_or(60_000);
                Some(FailpointAction::Hang(millis))
            }
            Some(other) => panic!(
                "环境变量 {ENV_FAILPOINT} 的断点动作未知：{other}；\
                 可用：panic / abort / exit / hang[:millis]"
            ),
        }
    }
}

#[cfg(feature = "job-failpoints")]
pub use enabled::*;

/// 命中一个测试断点。
///
/// 生产构建（未启用 `job-failpoints`）展开为**空语句**：断点名与其分支都不存在
/// （宏参数不参与展开，因此连常量引用都不会进入二进制）。
#[cfg(feature = "job-failpoints")]
#[macro_export]
macro_rules! job_failpoint {
    ($owner:expr, $name:expr) => {{ $crate::jobs::failpoints::hit($owner, $name) }};
}

#[cfg(not(feature = "job-failpoints"))]
#[macro_export]
macro_rules! job_failpoint {
    ($owner:expr, $name:expr) => {{}};
}
