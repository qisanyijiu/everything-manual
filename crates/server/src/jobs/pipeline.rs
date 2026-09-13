//! 组装阶段处理器（`assemble_draft`；T15 / REQ-030）。
//!
//! 位置（contracts.md §5 的 DAG）：知识分支（`manual_extract` 批次 → `manual_merge`）
//! 与模型分支（`tripo_*` → `model_validate`）汇合后组装草稿。本处理器：
//!
//! - **不调用任何供应商**（零外呼、零新费用）：输入只有冻结事实（合并结果资产 +
//!   validated 模型版本）；
//! - **幂等**：重复执行不重复创建草稿（唯一键 + 内容比较，见
//!   [`crate::drafts::service::assemble_draft`]）；
//! - **部分成功可展示**：某条分支头被阻塞（`failed`/`needs_input`/
//!   `submission_unknown`）时也产出草稿，缺项写进 `missing[]`；
//! - 结果事实是 `usage_json`（`draftId`/`revision`/`completeness`/`missingCount`）；
//!   **不写独立结果资产**——草稿行本身就是产物，重跑幂等（无"陈旧结果补推进"风险）。
//!
//! 与执行器的分工不变：本处理器只返回 [`StageOutcome`]，业务状态推进（带租约
//! epoch guard）由执行器完成。

use std::path::PathBuf;

use manual_core::domain::StageKind;
use serde_json::json;

use crate::config::Settings;
use crate::drafts::service::{DraftServiceError, assemble_draft};
use crate::jobs::{
    JobError, MissingItem, StageContext, StageFuture, StageHandler, StageOutcome, StageRegistry,
};

/// 组装处理器（`serve` 启动时无条件注册：组装不依赖任何 Provider 配置）。
pub struct PipelineHandlers {
    data_dir: PathBuf,
}

impl PipelineHandlers {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            data_dir: settings.data_dir.clone(),
        }
    }

    /// 注册 `assemble_draft`（本卡唯一的流水线本地阶段）。
    pub fn register(&self, registry: &mut StageRegistry) -> Vec<StageKind> {
        registry.register(
            StageKind::AssembleDraft,
            AssembleDraftHandler::new(self.data_dir.clone()),
        );
        vec![StageKind::AssembleDraft]
    }
}

/// `assemble_draft`：把两条分支的冻结产物组装成 `needs_review` 草稿。
pub struct AssembleDraftHandler {
    data_dir: PathBuf,
}

impl AssembleDraftHandler {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}

impl StageHandler for AssembleDraftHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            match assemble_draft(&ctx.pool, &self.data_dir, &ctx.job, ctx.now).await {
                Ok(outcome) => {
                    let draft = outcome.draft;
                    let usage = json!({
                        "draftId": draft.id,
                        "draftRevision": draft.revision,
                        "created": outcome.created,
                        "completeness": if outcome.missing.is_empty() {
                            "complete"
                        } else {
                            "partial"
                        },
                        "missingCodes": outcome
                            .missing
                            .iter()
                            .map(|item| item.code.clone())
                            .collect::<Vec<_>>(),
                    });
                    tracing::info!(
                        event = "draft_assembled",
                        jobId = %ctx.job.id,
                        stageId = %ctx.stage.id,
                        draftId = %draft.id,
                        draftRevision = draft.revision,
                        created = outcome.created,
                        missing = outcome.missing.len(),
                        "草稿已组装（生成完成不等于已发布：status = needs_review，不存在自动发布）"
                    );
                    Ok(StageOutcome::Succeeded {
                        // 草稿行即产物；不写独立结果资产（重跑幂等）。
                        result_asset_id: None,
                        usage: Some(usage),
                    })
                }
                // 完整性异常：不重试、不重复付费；列出可行动缺项等人工核对。
                Err(DraftServiceError::Integrity { code, message }) => {
                    tracing::warn!(
                        event = "draft_assembly_blocked",
                        jobId = %ctx.job.id,
                        stageId = %ctx.stage.id,
                        errorCode = code,
                        "组装输入完整性异常：产出缺项而不是假成功"
                    );
                    Ok(StageOutcome::NeedsInput {
                        items: vec![MissingItem::new(code, message)],
                    })
                }
                Err(DraftServiceError::NotFound { .. }) => Ok(StageOutcome::NeedsInput {
                    items: vec![MissingItem::new(
                        "draft_assembly_input_missing",
                        "组装输入不存在（job/快照/阶段引用不一致）：请人工核对（不重新请求）",
                    )],
                }),
                // 字段级问题只可能来自 HTTP PATCH（请求字段校验）；组装路径没有
                // 请求输入，出现即内部不一致：按缺项中止，不重试（也不假装成功）。
                Err(DraftServiceError::FieldIssues(_)) => Ok(StageOutcome::NeedsInput {
                    items: vec![MissingItem::new(
                        "draft_assembly_field_issues",
                        "组装过程出现字段级校验问题（不应发生）：请人工核对",
                    )],
                }),
                Err(DraftServiceError::Storage(error)) => Err(JobError::from(error)),
            }
        })
    }
}
