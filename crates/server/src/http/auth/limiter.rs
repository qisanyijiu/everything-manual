//! 登录失败限速（REQ-002、PRD §5.1：默认 5 次/分钟 → 429，可配置）。
//!
//! 语义（**窗口与重置**，QA 按此复核）：
//! - **固定窗口**：每个来源 IP 维护 `(window_started, failures)`。窗口内累计失败达到上限后，
//!   后续尝试一律 429（`Retry-After` 给出窗口剩余秒数），**不消耗也不延长窗口**；
//! - 窗口到期后计数自动重置（下次失败开启新窗口）；登录成功立即清空该 IP 的计数；
//! - 只统计**失败**（密码错误），成功的登录不占额度；
//! - 只驻留内存：进程重启即清零（单管理员自托管，不引入持久化攻击面）；
//!   键是来源 IP（来源判定见 `auth::client_ip`，未受信来源的 `X-Forwarded-For` 不参与）；
//! - 表大小有界：每次操作顺带清理超过一个窗口未活动的条目。
//!
//! **时间源可注入（BUG-001）**：生产路径用 [`TimeSource::System`]（单调时钟，行为与
//! 默认值不变）；单元测试用 [`TimeSource::Manual`] 手动推进时间。原来的测试用真实
//! `sleep` + 毫秒级窗口，断言依赖"两次调用之间调度延迟小于窗口"，在高负载机器上会
//! 偶发失败（时序 flake）。注入时钟后，窗口到期与重置完全由测试驱动，断言一字不改，
//! 也不再需要 sleep。

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 单个来源的失败计数。
#[derive(Debug, Clone, Copy)]
struct AttemptState {
    window_started: Instant,
    failures: u32,
}

/// 时间源：生产用系统单调时钟，测试用手动时钟。
#[derive(Debug, Clone)]
pub enum TimeSource {
    /// 生产默认：`Instant::now()`。
    System,
    /// 测试：由 [`ManualClock`] 决定"现在"。
    Manual(Arc<ManualClock>),
}

impl TimeSource {
    fn now(&self) -> Instant {
        match self {
            Self::System => Instant::now(),
            Self::Manual(clock) => clock.now(),
        }
    }
}

/// 手动时钟：以创建时刻为基准，只按 [`ManualClock::advance`] 前进（测试专用）。
#[derive(Debug)]
pub struct ManualClock {
    base: Instant,
    offset_millis: std::sync::atomic::AtomicU64,
}

impl ManualClock {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            offset_millis: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// 当前（受控）时刻。
    pub fn now(&self) -> Instant {
        self.base
            + Duration::from_millis(self.offset_millis.load(std::sync::atomic::Ordering::SeqCst))
    }

    /// 手动推进时间。
    pub fn advance(&self, duration: Duration) {
        self.offset_millis.fetch_add(
            duration.as_millis() as u64,
            std::sync::atomic::Ordering::SeqCst,
        );
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

/// 内存限速器（每个 `AppState` 一个）。
#[derive(Debug)]
pub struct LoginRateLimiter {
    max_failures: u32,
    window: Duration,
    attempts: Mutex<HashMap<IpAddr, AttemptState>>,
    time_source: TimeSource,
}

impl LoginRateLimiter {
    /// `max_failures` 至少为 1、`window` 至少 1 毫秒（配置层已校验非零；此处做
    /// 保护性收口，只防止"限速形同虚设"，不限制运维选择更长的窗口）。
    ///
    /// 生产构造：使用系统单调时钟。
    pub fn new(max_failures: u32, window: Duration) -> Self {
        Self::with_time_source(max_failures, window, TimeSource::System)
    }

    /// 指定时间源（测试用注入时钟；语义与默认值完全相同）。
    pub fn with_time_source(max_failures: u32, window: Duration, time_source: TimeSource) -> Self {
        Self {
            max_failures: max_failures.max(1),
            window: window.max(Duration::from_millis(1)),
            attempts: Mutex::new(HashMap::new()),
            time_source,
        }
    }

    /// 窗口上限（`/settings/status` 与测试读取；不返回给未认证请求）。
    pub fn max_failures(&self) -> u32 {
        self.max_failures
    }

    /// 是否允许尝试。`Err(retry_after_seconds)` = 当前处于限速窗口。
    pub fn check(&self, ip: IpAddr) -> Result<(), u64> {
        let now = self.time_source.now();
        let mut attempts = self.lock();
        self.prune(&mut attempts, now);
        match attempts.get(&ip) {
            Some(state)
                if state.failures >= self.max_failures
                    && now.duration_since(state.window_started) < self.window =>
            {
                let remaining = self
                    .window
                    .saturating_sub(now.duration_since(state.window_started));
                // 向上取整：剩余不足 1 秒也提示 1 秒，避免 Retry-After: 0 造成重试风暴。
                let seconds = remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0);
                Err(seconds.max(1))
            }
            _ => Ok(()),
        }
    }

    /// 记录一次失败（开启或延续当前窗口）。
    pub fn record_failure(&self, ip: IpAddr) {
        let now = self.time_source.now();
        let mut attempts = self.lock();
        self.prune(&mut attempts, now);
        attempts
            .entry(ip)
            .and_modify(|state| {
                if now.duration_since(state.window_started) >= self.window {
                    *state = AttemptState {
                        window_started: now,
                        failures: 1,
                    };
                } else {
                    state.failures = state.failures.saturating_add(1);
                }
            })
            .or_insert(AttemptState {
                window_started: now,
                failures: 1,
            });
    }

    /// 登录成功：清零该来源的失败计数。
    pub fn record_success(&self, ip: IpAddr) {
        let mut attempts = self.lock();
        attempts.remove(&ip);
    }

    /// 清理超过一个窗口未活动的条目（表大小有界）。
    fn prune(&self, attempts: &mut HashMap<IpAddr, AttemptState>, now: Instant) {
        let window = self.window;
        attempts.retain(|_, state| now.duration_since(state.window_started) < window * 3);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, AttemptState>> {
        // 限速器内部不做 IO，毒化只可能来自 panic；此时退化为空表比让服务 500 更好。
        self.attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        "203.0.113.7".parse().unwrap()
    }

    /// 手动时钟构造的限速器（BUG-001：测试不再依赖真实时间流逝）。
    fn limiter_with_clock(
        max_failures: u32,
        window: Duration,
    ) -> (LoginRateLimiter, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new());
        let limiter = LoginRateLimiter::with_time_source(
            max_failures,
            window,
            TimeSource::Manual(clock.clone()),
        );
        (limiter, clock)
    }

    #[test]
    fn blocks_after_limit_within_window_and_recovers_after_it() {
        // 60ms 窗口：仅测试加速，语义与生产默认（60s）相同。
        let (limiter, clock) = limiter_with_clock(2, Duration::from_millis(60));
        assert!(limiter.check(ip()).is_ok());
        limiter.record_failure(ip());
        assert!(limiter.check(ip()).is_ok(), "第 2 次尝试仍应放行");
        limiter.record_failure(ip());
        let retry = limiter.check(ip()).unwrap_err();
        assert!(retry >= 1, "Retry-After 至少 1 秒：{retry}");
        assert!(limiter.check(ip()).is_err(), "窗口内持续 429");

        // 窗口内推进 59ms 仍被限速（旧断言用真实 sleep，时间由测试精确驱动）。
        clock.advance(Duration::from_millis(59));
        assert!(limiter.check(ip()).is_err(), "窗口内持续 429");

        clock.advance(Duration::from_millis(1));
        assert!(limiter.check(ip()).is_ok(), "窗口结束后自动重置");
    }

    #[test]
    fn success_clears_failures_and_ips_are_isolated() {
        let (limiter, _clock) = limiter_with_clock(1, Duration::from_secs(60));
        limiter.record_failure(ip());
        assert!(limiter.check(ip()).is_err());
        limiter.record_success(ip());
        assert!(limiter.check(ip()).is_ok(), "成功登录立即重置计数");

        let other: IpAddr = "203.0.113.8".parse().unwrap();
        limiter.record_failure(ip());
        assert!(limiter.check(other).is_ok(), "不同来源互不影响");
    }

    #[test]
    fn window_restarts_after_expiry_and_zero_limits_are_clamped() {
        let (limiter, _clock) = limiter_with_clock(0, Duration::from_millis(1));
        assert_eq!(limiter.max_failures(), 1, "0 次上限被保护性收口为 1");
        limiter.record_failure(ip());
        assert!(limiter.check(ip()).is_err());

        let (limiter, clock) = limiter_with_clock(1, Duration::from_millis(40));
        limiter.record_failure(ip());
        // 时间由测试推进：不再存在"sleep 与断言之间被调度延迟 ≥40ms 就失败"的窗口。
        clock.advance(Duration::from_millis(60));
        limiter.record_failure(ip());
        assert!(
            limiter.check(ip()).is_err(),
            "过期后的失败开启新窗口（旧计数不累计）"
        );
        // 新窗口从第二次失败起算：距它只过了 39ms（< 40ms）→ 仍在限速窗口内；
        // 若沿用旧窗口起点（已过 99ms）这里会错误放行。
        clock.advance(Duration::from_millis(39));
        assert!(
            limiter.check(ip()).is_err(),
            "新窗口的起点是第二次失败，而不是第一次"
        );
        clock.advance(Duration::from_millis(1));
        assert!(limiter.check(ip()).is_ok(), "新窗口到期后再次重置");
    }

    /// BUG-001 的回归守护：毫秒级窗口 + 手动时钟时，**任意次数**重复判定都完全确定，
    /// 不依赖两次调用之间的真实调度延迟（旧实现下该循环会因 ≥1ms 的延迟而失败）。
    #[test]
    fn millisecond_window_is_deterministic_with_injected_clock() {
        let (limiter, clock) = limiter_with_clock(1, Duration::from_millis(1));
        limiter.record_failure(ip());
        for round in 0..1000 {
            assert!(
                limiter.check(ip()).is_err(),
                "第 {round} 次判定：窗口内必须持续 429（不推进时钟）"
            );
        }
        clock.advance(Duration::from_millis(1));
        assert!(limiter.check(ip()).is_ok(), "推进满一个窗口后自动重置");
    }
}
