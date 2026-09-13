//! 崩溃恢复矩阵（contracts.md §5 第 5 条；T10）。
//!
//! 触发条件：阶段 `status = 'running'` 且租约已过期（`lease_until <= now`）。
//! 这意味着持锁 worker 已经死亡（或长时间无法续约），阶段需要被收敛到确定状态。
//! 恢复**先接管**（`lease_epoch + 1`，见 `repo::job_stages::take_over_expired`），
//! 再用同一套带 epoch guard 的推进写入：旧 worker 之后无论如何都无法覆盖结果。
//!
//! 判定表（[`plan`]；与 contracts.md §5 的 recovery 规则一一对应）：
//!
//! | 现场 | 恢复动作 | 理由 |
//! | --- | --- | --- |
//! | job 已取消 | `Cancel` | 取消后不发新付费步骤 |
//! | `result_asset_id` 非空 | 见 [`plan_succeed`] | 结果已持久化、checkpoint 未推进：不重新付费；**但拒答/无知识批次（`producedKnowledge=false`）补推进为 `needs_input`**（T14 P3-1），不标记成"产出了知识" |
//! | 无 attempt | `Requeue` | 没有未决事实，可安全重领 |
//! | attempt `intent` | `Requeue` | 未标记 submitting：请求未发出，可安全重领 |
//! | attempt `failed` | `Requeue` | 已定性的失败（可证明未被接受） |
//! | Tripo `submitting`/`unknown` + 有 task ID | `Succeed` | 远端事实已存在：提交阶段完成，后续阶段按已知 ID 继续查询 |
//! | Tripo `submitting` + 无 task ID | `SubmissionUnknown` | 客户端无法证明请求未被接受 |
//! | 同步 Manual AI `submitting` | `SubmissionUnknown` | 已发请求但完整响应未持久化；`response_id` 不假定可轮询 |
//! | 同步 Manual AI `accepted` 但无结果事实 | `NeedsInput` | 结果事实缺失的完整性问题：人工核对，不重新付费 |

use manual_core::domain::{JobStage, JobStatus, ProviderAttempt, SubmitState};
use manual_core::jobs::{SubmissionStyle, submission_style};

use super::MissingItem;

/// 恢复动作（执行器负责落库）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// 结果事实已在库：校验后补推进为 `succeeded`（不重新请求、不重新付费）。
    Succeed {
        /// 需要校验存在性的结果资产（`None` = 远端事实已在 attempt 上）。
        result_asset_id: Option<String>,
    },
    /// 付费结果未知：该分支暂停购买，等待管理员对账。
    SubmissionUnknown {
        attempt_id: Option<String>,
        reason: String,
    },
    /// 没有未决事实：重新入队（`queued`）。
    Requeue,
    /// 完整性问题：需要人工核对（不自动重试、不重新付费）。
    NeedsInput {
        items: Vec<MissingItem>,
        reason: String,
    },
    /// job 已取消：本地阶段直接取消。
    Cancel,
}

/// 批次结果事实的"是否产出正式知识"判据（T14 P3-1 修正）。
///
/// `usage_json` 的 `producedKnowledge` 与批次结果资产在同一事务写入
/// （T14 的 `record_sync_response`），是拒答/截断/格式错等**诊断结果**与
/// 正式知识结果的区分字段。只认显式 `false`：字段缺失（旧事实/测试夹具）或
/// `true` 都保持既有 T10 语义——"结果已持久化 → 校验后补推进，不重新付费"。
pub fn batch_produced_knowledge(usage: Option<&serde_json::Value>) -> Option<bool> {
    usage?.get("producedKnowledge")?.as_bool()
}

/// "结果已持久化、checkpoint 未推进"时的补推进动作（T14 P3-1）。
///
/// 修正前：`result_asset_id` 非空一律补推进为 `succeeded`——但 T14 的**失败路径
/// 也会写结果资产**（拒答/incomplete/格式错的诊断批次），会被误标成"产出了知识"。
/// 现在按 `producedKnowledge` 判定：
/// - 显式 `false` → `needs_input`（缺项沿用批次事实里的稳定错误码与摘要），
///   保持"该批未产出正式知识"的展示口径；不重新付费、不自动重试；
/// - 其余 → `Succeed`（结果资产存在性由执行器校验，缺失按完整性问题处理）。
pub fn plan_succeed(stage: &JobStage) -> RecoveryAction {
    let usage = stage.usage_json.as_ref();
    if batch_produced_knowledge(usage) == Some(false) {
        let code = usage
            .and_then(|usage| usage.get("errorCode"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("manual_batch_without_knowledge")
            .to_owned();
        let summary = usage
            .and_then(|usage| usage.get("errorSummary"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("该批次未产出正式知识（拒答/截断/格式错/校验失败）")
            .to_owned();
        return RecoveryAction::NeedsInput {
            items: vec![MissingItem::new(
                code,
                format!(
                    "{summary}；恢复时按批次结果事实判定为未产出正式知识（不自动重试、不重复付费），\
                     请人工复核后对该批重新授权重算"
                ),
            )],
            reason: "该批次结果事实显示未产出正式知识（producedKnowledge=false）".to_owned(),
        };
    }
    RecoveryAction::Succeed {
        result_asset_id: stage.result_asset_id.clone(),
    }
}

/// 恢复判定（纯函数；执行器负责接管租约与落库）。
pub fn plan(
    stage: &JobStage,
    attempt: Option<&ProviderAttempt>,
    job_status: JobStatus,
) -> RecoveryAction {
    use RecoveryAction::{Cancel, NeedsInput, Requeue, SubmissionUnknown, Succeed};

    if matches!(job_status, JobStatus::Cancelled) {
        return Cancel;
    }
    if stage.result_asset_id.is_some() {
        // "result 已持久化而 checkpoint 未推进时，恢复程序校验已有结果并补推进，不重新付费"。
        // 但"结果已持久化"不等于"产出了知识"（T14 P3-1）：按结果事实判定，见 [`plan_succeed`]。
        return plan_succeed(stage);
    }
    let style = submission_style(stage.stage_kind);
    let Some(attempt) = attempt else {
        return Requeue;
    };
    match attempt.submit_state {
        SubmitState::Intent | SubmitState::Failed => Requeue,
        SubmitState::Submitting | SubmitState::Unknown => {
            let has_remote_task = attempt.remote_task_id.clone();
            match (style, has_remote_task) {
                (Some(SubmissionStyle::AsyncRemoteTask), Some(_)) => Succeed {
                    result_asset_id: None,
                },
                (Some(SubmissionStyle::AsyncRemoteTask), None) => SubmissionUnknown {
                    attempt_id: Some(attempt.id.clone()),
                    reason: "付费创建结果未知：attempt 已标记 submitting 但没有远端 task ID，\
                             客户端无法证明请求未被接受"
                        .to_owned(),
                },
                _ => SubmissionUnknown {
                    attempt_id: Some(attempt.id.clone()),
                    reason: "已发出同步请求但没有持久化完整响应：该批进入 submission_unknown，\
                             response_id 不假定可轮询，只能由管理员对账"
                        .to_owned(),
                },
            }
        }
        SubmitState::Accepted => match style {
            Some(SubmissionStyle::AsyncRemoteTask) => match &attempt.remote_task_id {
                Some(_) => Succeed {
                    result_asset_id: None,
                },
                None => SubmissionUnknown {
                    attempt_id: Some(attempt.id.clone()),
                    reason: "attempt 标记 accepted 但没有远端 task ID：无法继续查询，需人工对账"
                        .to_owned(),
                },
            },
            Some(SubmissionStyle::SyncResponse) => NeedsInput {
                items: vec![MissingItem::new(
                    "manual_batch_result_missing",
                    "该批次的响应事实已记录但结果事实缺失；请人工核对后重新授权，不自动重新付费",
                )],
                reason: "同步批次结果事实缺失（完整性异常）".to_owned(),
            },
            _ => Requeue,
        },
    }
}

/// 一次恢复扫描的统计（日志与测试断言用；不含用户数据）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// 扫描到的"租约已过期的 running 阶段"数量。
    pub scanned: usize,
    /// 被本 worker 接管并收敛的数量。
    pub recovered: usize,
    /// 接管失败（已被其他 worker 收敛）的数量。
    pub skipped: usize,
    pub requeued: usize,
    pub succeeded: usize,
    pub submission_unknown: usize,
    pub needs_input: usize,
    pub cancelled: usize,
}

impl RecoveryReport {
    /// 一行日志摘要。
    pub fn summary(&self) -> String {
        format!(
            "scanned={} recovered={} skipped={} requeued={} succeeded={} unknown={} needsInput={} cancelled={}",
            self.scanned,
            self.recovered,
            self.skipped,
            self.requeued,
            self.succeeded,
            self.submission_unknown,
            self.needs_input,
            self.cancelled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use manual_core::domain::StageKind;
    use manual_core::timestamps::Timestamp;
    use serde_json::{Value, json};

    fn stage(kind: StageKind, result_asset_id: Option<&str>, usage: Option<Value>) -> JobStage {
        JobStage {
            id: "stage-1".to_owned(),
            job_id: "job-1".to_owned(),
            stage_kind: kind,
            batch_index: 0,
            page_set: None,
            input_hash: "hash".to_owned(),
            result_asset_id: result_asset_id.map(str::to_owned),
            usage_json: usage,
            status: JobStatus::Running,
            lease_owner: Some("worker".to_owned()),
            lease_epoch: 3,
            lease_until: Some(Timestamp::from_millis(1)),
            next_run_at: None,
            attempt_count: 0,
            poll_count: 0,
            last_error: None,
            needs_input_json: None,
            created_at: Timestamp::EPOCH,
            updated_at: Timestamp::EPOCH,
        }
    }

    /// T14 P3-1：拒答批次（`producedKnowledge=false`）即使结果资产已落库，
    /// 恢复也必须补推进为 `needs_input` 而不是 `succeeded`。
    #[test]
    fn refused_batch_result_fact_reverts_to_needs_input_not_success() {
        let refused = stage(
            StageKind::ManualExtract,
            Some("asset-1"),
            Some(json!({
                "outcome": "refusal",
                "producedKnowledge": false,
                "errorCode": "manual_ai_refusal",
                "errorSummary": "模型拒答（refusal）：不提供该内容",
            })),
        );
        match plan(&refused, None, JobStatus::Running) {
            RecoveryAction::NeedsInput { items, .. } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].code, "manual_ai_refusal");
                assert!(
                    items[0].message.contains("未产出正式知识"),
                    "{}",
                    items[0].message
                );
            }
            other => panic!("拒答批次必须补推进为 needs_input，实际 {other:?}"),
        }

        // 产出了知识的批次（显式 true）保持 T10 语义：补推进为 succeeded。
        let produced = stage(
            StageKind::ManualExtract,
            Some("asset-2"),
            Some(json!({ "producedKnowledge": true })),
        );
        assert_eq!(
            plan(&produced, None, JobStatus::Running),
            RecoveryAction::Succeed {
                result_asset_id: Some("asset-2".to_owned()),
            }
        );

        // 旧事实/测试夹具没有该字段：不改变既有语义（不能只凭字段缺失判失败）。
        let legacy = stage(StageKind::ManualExtract, Some("asset-3"), Some(json!({})));
        assert_eq!(batch_produced_knowledge(legacy.usage_json.as_ref()), None);
        assert_eq!(
            plan(&legacy, None, JobStatus::Running),
            RecoveryAction::Succeed {
                result_asset_id: Some("asset-3".to_owned()),
            }
        );

        // 非批次阶段（合并/模型校验）的结果事实不受该判定影响。
        let merge = stage(
            StageKind::ManualMerge,
            Some("asset-4"),
            Some(json!({ "coverage": { "complete": true } })),
        );
        assert_eq!(
            plan(&merge, None, JobStatus::Running),
            RecoveryAction::Succeed {
                result_asset_id: Some("asset-4".to_owned()),
            }
        );
    }
}
