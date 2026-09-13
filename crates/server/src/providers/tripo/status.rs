//! Tripo 任务状态归一化（contracts.md §6、architecture.md §5.3）。
//!
//! 规则（T12 卡 / AC-041）：
//! - 生命周期的已知状态 = `queued / running / success / failed / cancelled / banned /
//!   expired`（architecture §5.3 与任务生命周期页：**`banned` 与 `expired` 也在集合里**）；
//! - **未知值保留原值**并进入"可诊断的等待/待处理"，**不得猜测**语义；
//! - 归一化只做"匹配"层面的容错（trim + ASCII 小写），**原始字面量始终原样保留**；
//! - `success` 必须有可下载模型（由调用方检查 `output.model_url`），否则不组装成功。
//!
//! 产物过期（`expired`）与下载链接过期是不同的错误：前者是任务的产物生命周期
//! （不可找回，需重新生成），后者可以重新查询获取新链接（T13 处理）。

use serde::Serialize;

/// 归一化后的任务状态机视角（未知值单列一类，不映射到已知状态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TripoState {
    Queued,
    Running,
    Success,
    Failed,
    Cancelled,
    /// 供应商封禁（生命周期终态；保留原因由上层诊断）。
    Banned,
    /// 任务产物过期（生命周期终态；与"下载链接过期"不同）。
    Expired,
    /// 未知枚举：**原值保留**，进入可诊断的等待/待处理。
    Unrecognized,
}

impl TripoState {
    /// 稳定字符串（落库/日志；`raw` 另有原值）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Banned => "banned",
            Self::Expired => "expired",
            Self::Unrecognized => "unrecognized",
        }
    }

    /// 已知状态的解析（仅用于匹配；大小写与首尾空白容错，语义不做猜测）。
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "success" => Self::Success,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "banned" => Self::Banned,
            "expired" => Self::Expired,
            _ => Self::Unrecognized,
        }
    }

    /// 是否为终态（不会自行变化；仍需按各自语义处理）。
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Success | Self::Failed | Self::Cancelled | Self::Banned | Self::Expired
        )
    }
}

/// 原始状态 + 归一化状态（两者都必须保存：原始值用于诊断与 UI "状态待确认"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedStatus {
    raw: String,
    state: TripoState,
}

impl NormalizedStatus {
    /// 由供应商原始字面量构造。
    pub fn new(raw: &str) -> Self {
        Self {
            raw: raw.to_owned(),
            state: TripoState::parse(raw),
        }
    }

    /// 供应商原始值（原样；未知枚举也保留）。
    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn state(&self) -> TripoState {
        self.state
    }

    /// 未知枚举（"状态待确认，将自动重新查询"的判据）。
    pub fn is_unrecognized(&self) -> bool {
        self.state == TripoState::Unrecognized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_statuses_map_and_raw_is_preserved() {
        for (raw, expected) in [
            ("queued", TripoState::Queued),
            ("running", TripoState::Running),
            ("success", TripoState::Success),
            ("failed", TripoState::Failed),
            ("cancelled", TripoState::Cancelled),
            ("banned", TripoState::Banned),
            ("expired", TripoState::Expired),
        ] {
            let normalized = NormalizedStatus::new(raw);
            assert_eq!(normalized.state(), expected, "{raw}");
            assert_eq!(normalized.raw(), raw);
        }
        // 匹配层面的容错不改变原始值。
        let normalized = NormalizedStatus::new(" Running ");
        assert_eq!(normalized.state(), TripoState::Running);
        assert_eq!(normalized.raw(), " Running ");
    }

    #[test]
    fn unknown_statuses_are_kept_verbatim_and_never_guessed() {
        for raw in ["weird_new_state", "SUCCESSFUL", "0", "processing"] {
            let normalized = NormalizedStatus::new(raw);
            assert_eq!(normalized.state(), TripoState::Unrecognized, "{raw}");
            assert_eq!(normalized.raw(), raw, "未知状态必须保留原值");
            assert!(normalized.is_unrecognized());
        }
        assert!(!TripoState::Unrecognized.is_terminal());
        assert!(TripoState::Banned.is_terminal());
        assert!(TripoState::Expired.is_terminal());
    }
}
