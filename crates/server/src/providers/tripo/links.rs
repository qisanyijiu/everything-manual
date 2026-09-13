//! 进程内**易失**的临时下载链接（T20 · BUG-008）。
//!
//! 背景：`tripo_poll` 观察到的 `output.model_url` 是**带签名的临时地址**，
//! 不能作为持久化任务元数据保存（AC-010：备份/导出不含临时云端 URL；见
//! [`crate::redaction`]）。但 `model_download` 阶段需要一条可用链接。
//!
//! 分工（不破坏恢复语义）：
//!
//! - **持久化事实** = `task_id`（+ 状态/计费/链接摘要）：恢复的判据是 task_id，
//!   不是 URL（contracts §5「链接过期 → 重新查询已知任务取新链接，不重新购买」）；
//! - **易失链接** = 本模块：只在观察到它的进程内暂存（有界、不落库、不落盘），
//!   省掉"刚查到就再查一次"的重复请求；
//! - **缓存未命中**（进程重启、恢复、条目被淘汰、跨进程）→ 下载阶段按 task_id
//!   重新 `GET /tasks/{id}` 取新链接（免费查询，绝不重新购买）。
//!
//! 有界与不进日志：最多保留 [`MAX_ENTRIES`] 条，超出按观察顺序淘汰最旧；
//! `Debug` 不打印 URL 内容（避免经 `tracing` 或断言消息泄露签名串）。

use std::collections::VecDeque;
use std::fmt;
use std::sync::Mutex;

use manual_core::timestamps::Timestamp;

/// 进程内最多保留的链接条数（超出淘汰最旧；每个条目只占一个短字符串）。
pub const MAX_ENTRIES: usize = 64;

/// 一条易失链接（`(job, task)` → 观察到的模型 URL）。
///
/// 不实现 `Debug` 的字段直出：`Debug` 只显示 job/task/时间，不显示 URL。
struct Entry {
    job_id: String,
    task_id: String,
    model_url: String,
    observed_at_millis: i64,
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("job_id", &self.job_id)
            .field("task_id", &self.task_id)
            .field("observed_at_millis", &self.observed_at_millis)
            .field("model_url", &"[redacted]")
            .finish()
    }
}

/// 有界、进程内的易失链接表（`Mutex` 保护；临界区只有内存操作，不跨 `await`）。
#[derive(Default)]
pub struct EphemeralLinks {
    entries: Mutex<VecDeque<Entry>>,
}

impl fmt::Debug for EphemeralLinks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self
            .entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0);
        f.debug_struct("EphemeralLinks")
            .field("entries", &count)
            .finish()
    }
}

impl EphemeralLinks {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记下观察到的链接（同一 `(job, task)` 覆盖旧值：新观察优先）。
    pub fn remember(&self, job_id: &str, task_id: &str, model_url: &str, now: Timestamp) {
        let Ok(mut entries) = self.entries.lock() else {
            // 锁中毒（仅可能在 panic 中）：宁可退化为"缓存未命中"（重新查询），
            // 也不能 panic 掉任务执行。
            return;
        };
        entries.retain(|entry| !(entry.job_id == job_id && entry.task_id == task_id));
        entries.push_back(Entry {
            job_id: job_id.to_owned(),
            task_id: task_id.to_owned(),
            model_url: model_url.to_owned(),
            observed_at_millis: now.as_millis(),
        });
        while entries.len() > MAX_ENTRIES {
            entries.pop_front();
        }
    }

    /// 取一条链接（不移除：同一链接可能被重试的下载阶段再用一次；
    /// 过期时下载阶段会按 task_id 重新查询并覆盖）。
    pub fn get(&self, job_id: &str, task_id: &str) -> Option<String> {
        let entries = self.entries.lock().ok()?;
        entries
            .iter()
            .rev()
            .find(|entry| entry.job_id == job_id && entry.task_id == task_id)
            .map(|entry| entry.model_url.clone())
    }

    /// 当前条目数（测试与诊断用；不含任何 URL）。
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use manual_core::timestamps::Timestamp;

    #[test]
    fn remember_get_and_overwrite() {
        let links = EphemeralLinks::new();
        let now = Timestamp::now();
        assert!(links.get("job-a", "task-1").is_none());

        links.remember(
            "job-a",
            "task-1",
            "https://cdn.example.invalid/a?sign=one",
            now,
        );
        assert_eq!(
            links.get("job-a", "task-1").as_deref(),
            Some("https://cdn.example.invalid/a?sign=one")
        );
        // 不同 job / task 互不串台。
        assert!(links.get("job-b", "task-1").is_none());
        assert!(links.get("job-a", "task-2").is_none());

        // 新观察覆盖旧值（续签后的链接优先）。
        links.remember(
            "job-a",
            "task-1",
            "https://cdn.example.invalid/a?sign=two",
            now,
        );
        assert_eq!(
            links.get("job-a", "task-1").as_deref(),
            Some("https://cdn.example.invalid/a?sign=two")
        );
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn bounded_eviction_and_debug_hides_urls() {
        let links = EphemeralLinks::new();
        let now = Timestamp::now();
        for index in 0..(MAX_ENTRIES + 5) {
            links.remember(
                "job",
                &format!("task-{index}"),
                &format!("https://cdn.example.invalid/m?sign=canary-{index}"),
                now,
            );
        }
        assert_eq!(links.len(), MAX_ENTRIES);
        assert!(links.get("job", "task-0").is_none(), "最旧的条目被淘汰");
        let entry = Entry {
            job_id: "job".to_owned(),
            task_id: "task".to_owned(),
            model_url: "https://cdn.example.invalid/m?sign=canary-signed".to_owned(),
            observed_at_millis: 1,
        };
        let debug = format!("{links:?} {entry:?}");
        assert!(
            !debug.contains("canary"),
            "Debug/诊断不得泄露签名串：{debug}"
        );
    }
}
