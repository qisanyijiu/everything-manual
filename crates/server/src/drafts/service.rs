//! 草稿组装与最小草稿操作（T15 / REQ-030）。
//!
//! 组装输入（全部来自**冻结事实**，不重新调用任何供应商）：
//! - 知识分支：`manual_merge` 阶段的 `result_asset_id`（`MergedKnowledge` JSON）；
//! - 模型分支：`model_validate` 阶段的 `usage_json.modelRevisionId`
//!   → `model_revisions`（必须 `validated` 且属于同一物品）。
//!
//! 幂等（REQ-030「重启不重复创建 draft」）：`manual_drafts.snapshot_id` 唯一 +
//! [`repo::drafts::upsert_assembled`] 的"内容相同则不动"。因此：
//! - 崩溃恢复/重放/人工重试重复执行装配：不产生第二行草稿；
//! - 内容变化（重试后的新知识/新模型）才递增 revision 并回到 `needs_review`。
//!
//! 部分成功（REQ-030「部分成功可展示」）：任一分支头处于
//! `failed/needs_input/submission_unknown/cancelled` 时仍产出草稿，
//! 并在 `missing[]` 里列出缺项代码与可行动说明；`completeness = partial`。
//!
//! **不自动发布**：本模块只写 `manual_drafts`，从不写 `manual_releases`
//! （ADR-005；发布属 T19 的显式动作）。

use std::path::Path;

use sqlx::{SqliteConnection, SqlitePool};

use manual_core::domain::{Job, JobStatus, ManualDraft, StageKind};
use manual_core::knowledge::MergedKnowledge;
use manual_core::timestamps::Timestamp;
use manual_core::validation::FieldIssue;

use crate::storage::StorageError;
use crate::storage::repo::{self, drafts as drafts_repo};

use super::aggregate::{self, DraftPatch, ReviewOverlay};
use super::knowledge::{
    CODE_KNOWLEDGE_BRANCH_INCOMPLETE, CODE_MODEL_BRANCH_INCOMPLETE, CODE_MODEL_REVISION_MISSING,
    DraftKnowledge, DraftMissingItem, DraftModelInfo,
};

/// 缺项/说明消息的长度上限（只存必要摘要，contracts.md §2）。
const MESSAGE_MAX_CHARS: usize = 300;

/// 审计动作：组装产出/更新草稿（服务器动作）。
pub const AUDIT_DRAFT_ASSEMBLED: &str = "draft_assembled";
/// 审计动作：人工修改草稿状态（contracts.md §2「人工事实修改」）。
pub const AUDIT_DRAFT_STATUS_CHANGED: &str = "draft_status_changed";
/// 审计动作：人工复核/修订（确认知识、热点校准、视角保存、modelReview；T19）。
pub const AUDIT_DRAFT_REVIEW_UPDATED: &str = "draft_review_updated";

/// 缺项代码：旧热点因部件不存在未继承（T19 组装继承）。
pub const CODE_HOTSPOTS_DETACHED: &str = "hotspots_detached";
/// 缺项代码：步骤视角因模型/步骤变化未继承（T19 组装继承）。
pub const CODE_STEP_POSES_DROPPED: &str = "step_poses_dropped";

/// 草稿服务错误（HTTP 层与阶段处理器各自映射；不泄露内部细节）。
#[derive(Debug, Clone, PartialEq)]
pub enum DraftServiceError {
    /// 资源不存在（含跨物品引用：按不存在处理，不泄露存在性）。
    NotFound {
        entity: &'static str,
        id: String,
    },
    /// 组装输入完整性异常（结果资产缺失/不可读/不是合法 JSON、revision 与物品不符）。
    /// 映射为阶段 `needs_input`：不重试、不重复付费，等人工核对。
    Integrity {
        code: &'static str,
        message: String,
    },
    /// 受限字段 PATCH 的字段级校验失败（T19；HTTP 422 + `details.fields`）。
    FieldIssues(Vec<FieldIssue>),
    Storage(StorageError),
}

impl DraftServiceError {
    fn integrity(code: &'static str, message: impl Into<String>) -> Self {
        Self::Integrity {
            code,
            message: message.into(),
        }
    }

    /// 稳定错误码（日志与测试断言用；不含用户数据）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "draft_not_found",
            Self::Integrity { code, .. } => code,
            Self::FieldIssues(_) => "draft_field_validation",
            Self::Storage(_) => "draft_storage",
        }
    }
}

impl std::fmt::Display for DraftServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { entity, id } => write!(formatter, "{entity} 不存在：{id}"),
            Self::Integrity { code, message } => write!(formatter, "{code}：{message}"),
            Self::FieldIssues(issues) => {
                write!(formatter, "字段校验失败（{} 项）", issues.len())
            }
            Self::Storage(error) => write!(formatter, "存储错误：{error}"),
        }
    }
}

impl std::error::Error for DraftServiceError {}

impl From<StorageError> for DraftServiceError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::Error> for DraftServiceError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(StorageError::from(error))
    }
}

/// 一次组装的结果（阶段处理器与测试断言用）。
#[derive(Debug, Clone, PartialEq)]
pub struct AssembleOutcome {
    pub draft: ManualDraft,
    /// 是否新建了草稿行（重放/重跑为 `false`）。
    pub created: bool,
    /// 缺项（`partial` 时非空）。
    pub missing: Vec<DraftMissingItem>,
}

/// 组装草稿（`assemble_draft` 阶段的核心；幂等、不调用任何供应商）。
pub async fn assemble_draft(
    pool: &SqlitePool,
    data_dir: &Path,
    job: &Job,
    now: Timestamp,
) -> Result<AssembleOutcome, DraftServiceError> {
    let mut conn = pool.acquire().await?;
    let stages = repo::job_stages::list_for_job(&mut conn, &job.id).await?;

    // 知识分支：manual_merge 的结果资产（仅 succeeded 才有产物）。
    let mut missing: Vec<DraftMissingItem> = Vec::new();
    let knowledge = match stages
        .iter()
        .find(|stage| stage.stage_kind == StageKind::ManualMerge)
    {
        None => {
            missing.push(DraftMissingItem::new(
                CODE_KNOWLEDGE_BRANCH_INCOMPLETE,
                "知识分支不存在（找不到 manual_merge 阶段）：请核对任务阶段",
            ));
            None
        }
        Some(stage) if stage.status == JobStatus::Succeeded => {
            let asset_id = stage.result_asset_id.clone().ok_or_else(|| {
                DraftServiceError::integrity(
                    "draft_merge_result_missing",
                    "合并阶段成功但没有结果资产：请人工核对（不重新调用 AI）",
                )
            })?;
            let bytes = read_asset_bytes(&mut conn, data_dir, &asset_id).await?;
            let merged: MergedKnowledge = serde_json::from_slice(&bytes).map_err(|error| {
                DraftServiceError::integrity(
                    "draft_merge_result_invalid",
                    format!("合并结果资产不是合法的知识 JSON：{error}"),
                )
            })?;
            Some(merged)
        }
        Some(stage) => {
            missing.push(branch_missing_item(
                CODE_KNOWLEDGE_BRANCH_INCOMPLETE,
                "知识分支未产出合并知识",
                stage.stage_kind,
                stage.status,
                stage.last_error.as_deref(),
            ));
            None
        }
    };

    // 模型分支：model_validate 的 usage 记录 revision id（仅 succeeded 才有）。
    let model = match stages
        .iter()
        .find(|stage| stage.stage_kind == StageKind::ModelValidate)
    {
        None => {
            missing.push(DraftMissingItem::new(
                CODE_MODEL_BRANCH_INCOMPLETE,
                "模型分支不存在（找不到 model_validate 阶段）：请核对任务阶段",
            ));
            None
        }
        Some(stage) if stage.status == JobStatus::Succeeded => {
            let revision_id = stage
                .usage_json
                .as_ref()
                .and_then(|usage| usage.get("modelRevisionId"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    DraftServiceError::integrity(
                        "draft_model_revision_missing",
                        "模型校验阶段成功但缺少 modelRevisionId 事实：请人工核对（不重新下载/购买）",
                    )
                })?;
            let revision = repo::model_revisions::get(&mut conn, &revision_id)
                .await?
                .ok_or_else(|| {
                    DraftServiceError::integrity(
                        CODE_MODEL_REVISION_MISSING,
                        format!("模型版本（{revision_id}）不存在：请人工核对（不自动重下）"),
                    )
                })?;
            if revision.item_id != job.item_id {
                return Err(DraftServiceError::integrity(
                    CODE_MODEL_REVISION_MISSING,
                    "模型版本不属于该物品：拒绝组装（引用不一致）",
                ));
            }
            if revision.validation_state != manual_core::domain::ModelValidationState::Validated {
                missing.push(DraftMissingItem::new(
                    CODE_MODEL_REVISION_MISSING,
                    format!(
                        "模型版本未通过校验（{}）：草稿不含模型，可更换资料/重试模型分支",
                        revision.validation_state.as_str()
                    ),
                ));
                None
            } else {
                Some(DraftModelInfo {
                    revision_id: revision.id.clone(),
                    sha256: revision.sha256.clone(),
                    validation_state: revision.validation_state,
                    asset_id: revision.asset_id.clone(),
                    bounds: revision.bounds.clone(),
                })
            }
        }
        Some(stage) => {
            missing.push(branch_missing_item(
                CODE_MODEL_BRANCH_INCOMPLETE,
                "模型分支未产出可校验模型",
                stage.stage_kind,
                stage.status,
                stage.last_error.as_deref(),
            ));
            None
        }
    };

    let mut envelope = DraftKnowledge::build(&job.id, model, knowledge, missing);
    // T19 旧绑定继承（AC-053）：换模型重生成时，旧热点进入 stale（保留解释、
    // 不显示为有效热点、不可发布）；部件已消失的旧绑定丢弃并在缺项里报告。
    let previous = match drafts_repo::get_by_snapshot(&mut conn, &job.snapshot_id).await? {
        Some(draft) => Some(draft),
        None => drafts_repo::latest_for_item(&mut conn, &job.item_id).await?,
    };
    if let Some(previous) = &previous
        && let Ok(previous_knowledge) =
            serde_json::from_value::<DraftKnowledge>(previous.knowledge_json.clone())
    {
        let report = aggregate::carry_forward_previous(&mut envelope, &previous_knowledge);
        if report.hotspots_dropped > 0 {
            envelope.add_missing(DraftMissingItem::new(
                CODE_HOTSPOTS_DETACHED,
                format!(
                    "{} 个旧热点因对应部件不在本次知识中未保留：请在新部件上重新绑定（不静默复用旧绑定）",
                    report.hotspots_dropped
                ),
            ));
        }
        if report.poses_dropped > 0 {
            envelope.add_missing(DraftMissingItem::new(
                CODE_STEP_POSES_DROPPED,
                format!(
                    "{} 个步骤视角因模型/步骤变化未继承：请在当前模型上重新保存视角",
                    report.poses_dropped
                ),
            ));
        }
    }
    let knowledge_json = serde_json::to_string(&envelope).map_err(|error| {
        DraftServiceError::integrity(
            "draft_serialization_failed",
            format!("草稿知识序列化失败：{error}"),
        )
    })?;
    let missing = envelope.missing_items().to_vec();

    // 幂等 upsert（唯一键 = snapshot_id）+ 审计（同一事务）。
    // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
    let tx = crate::storage::begin_write(&mut conn).await?;
    let mut tx = tx;
    let assembled = drafts_repo::upsert_assembled(
        &mut tx,
        drafts_repo::NewAssembledDraft {
            item_id: job.item_id.clone(),
            snapshot_id: job.snapshot_id.clone(),
            model_revision_id: envelope
                .model
                .as_ref()
                .map(|model| model.revision_id.clone()),
            knowledge_json,
        },
        now,
    )
    .await?;
    if assembled.changed {
        repo::audit::record(
            &mut tx,
            repo::audit::NewAuditEvent {
                entity_type: "manual_draft".to_owned(),
                entity_id: assembled.draft.id.clone(),
                actor: Some("system".to_owned()),
                action: AUDIT_DRAFT_ASSEMBLED.to_owned(),
                result: if assembled.created {
                    "created".to_owned()
                } else {
                    "updated".to_owned()
                },
                metadata_json: Some(
                    serde_json::json!({
                        "jobId": job.id,
                        "snapshotId": job.snapshot_id,
                        "revision": assembled.draft.revision,
                        "completeness": envelope.completeness.as_str(),
                        "missingCount": missing.len(),
                    })
                    .to_string(),
                ),
            },
            now,
        )
        .await?;
    }
    tx.commit().await?;

    Ok(AssembleOutcome {
        draft: assembled.draft,
        created: assembled.created,
        missing,
    })
}

/// 读取草稿并校验物品归属（跨物品 → 按不存在处理）。
pub async fn read_draft(
    conn: &mut SqliteConnection,
    item_id: &str,
    draft_id: &str,
) -> Result<ManualDraft, DraftServiceError> {
    let draft = drafts_repo::get(conn, draft_id)
        .await?
        .filter(|draft| draft.item_id == item_id)
        .ok_or_else(|| DraftServiceError::NotFound {
            entity: "manual_draft",
            id: draft_id.to_owned(),
        })?;
    Ok(draft)
}

/// 草稿受限字段更新（T19：`status` + 热点/视角 + 实体复核 + modelReview）。
///
/// 语义（contracts.md §1/§2/§3；PRD AC-052/AC-054）：
/// - **If-Match 必填**：`expected_revision` 由调用方从请求头解析；过期 → 412
///   （`details.currentRevision`）。**无实际变化**的提交按幂等返回当前文档
///   （不递增 revision；stale revision 仍 412）——这是 T15 以来的既有语义。
/// - **原子应用**：字段级校验任一失败 → 422 `details.fields`，不写入任何内容。
/// - **不修改供应商事实快照**：`knowledge.knowledge` 只读；人工修订进 `review_json`
///   覆盖层（`userEdited` + 时间/操作者），`deny_unknown_fields` 拒绝任何直改入口。
/// - 动作写入 `audit_events`（`draft_status_changed` / `draft_review_updated`）。
pub async fn patch_draft(
    pool: &SqlitePool,
    item_id: &str,
    draft_id: &str,
    expected_revision: i64,
    patch: &DraftPatch,
    actor: &str,
    now: Timestamp,
) -> Result<ManualDraft, DraftServiceError> {
    let mut conn = pool.acquire().await?;
    // `BEGIN IMMEDIATE`（BUG-006）：本事务先读（归属/revision）后写（CAS + 审计）。
    let tx = crate::storage::begin_write(&mut conn).await?;
    let mut tx = tx;
    // 先校验归属（不存在/跨物品 → 404），再做字段校验与 revision CAS。
    let current = read_draft(&mut tx, item_id, draft_id).await?;

    let mut knowledge: DraftKnowledge = serde_json::from_value(current.knowledge_json.clone())
        .map_err(|error| {
            DraftServiceError::integrity(
                "draft_knowledge_unreadable",
                format!("草稿知识不是 manual_draft_v1 的合法形状：{error}"),
            )
        })?;
    let mut overlay: ReviewOverlay = match &current.review_json {
        None => ReviewOverlay::default(),
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
            DraftServiceError::integrity(
                "draft_review_unreadable",
                format!("草稿复核记录不是合法形状：{error}"),
            )
        })?,
    };
    let knowledge_before = knowledge.clone();
    let overlay_before = overlay.clone();
    let status = patch.status.unwrap_or(current.status);

    if let Err(issues) =
        aggregate::apply_draft_patch(&mut knowledge, &mut overlay, patch, actor, now)
    {
        tx.rollback().await?;
        return Err(DraftServiceError::FieldIssues(issues));
    }

    let knowledge_changed = knowledge != knowledge_before;
    let review_changed = overlay != overlay_before;
    let status_changed = status != current.status;
    if !knowledge_changed && !review_changed && !status_changed {
        // 幂等：没有任何变化（重复提交同一内容/同状态）。If-Match 仍必须成立。
        if current.revision != expected_revision {
            tx.rollback().await?;
            return Err(DraftServiceError::Storage(StorageError::RevisionConflict {
                entity: "manual_draft",
                id: draft_id.to_owned(),
                current_revision: current.revision,
            }));
        }
        tx.rollback().await?;
        return Ok(current);
    }

    // 只在内容确实变化时重写对应列（未变化的列保持原字节：避免序列化往返造成假变化）。
    let knowledge_json = if knowledge_changed {
        Some(serde_json::to_string(&knowledge).map_err(|error| {
            DraftServiceError::integrity(
                "draft_serialization_failed",
                format!("草稿知识序列化失败：{error}"),
            )
        })?)
    } else {
        None
    };
    let review_json: Option<Option<String>> = if review_changed {
        Some(Some(serde_json::to_string(&overlay).map_err(|error| {
            DraftServiceError::integrity(
                "draft_serialization_failed",
                format!("草稿复核记录序列化失败：{error}"),
            )
        })?))
    } else {
        None
    };

    let updated = drafts_repo::update_content(
        &mut tx,
        draft_id,
        expected_revision,
        knowledge_json.as_deref(),
        review_json.as_ref().map(|value| value.as_deref()),
        status,
        now,
    )
    .await?;

    if status_changed {
        repo::audit::record(
            &mut tx,
            repo::audit::NewAuditEvent {
                entity_type: "manual_draft".to_owned(),
                entity_id: updated.id.clone(),
                actor: Some(actor.to_owned()),
                action: AUDIT_DRAFT_STATUS_CHANGED.to_owned(),
                result: status.as_str().to_owned(),
                metadata_json: Some(
                    serde_json::json!({
                        "from": current.status.as_str(),
                        "to": status.as_str(),
                        "revision": updated.revision,
                    })
                    .to_string(),
                ),
            },
            now,
        )
        .await?;
    }
    if knowledge_changed || review_changed {
        let mut sections: Vec<&str> = Vec::new();
        if knowledge_changed {
            sections.push("knowledge");
        }
        if review_changed {
            sections.push("review");
        }
        repo::audit::record(
            &mut tx,
            repo::audit::NewAuditEvent {
                entity_type: "manual_draft".to_owned(),
                entity_id: updated.id.clone(),
                actor: Some(actor.to_owned()),
                action: AUDIT_DRAFT_REVIEW_UPDATED.to_owned(),
                result: sections.join("+"),
                metadata_json: Some(
                    serde_json::json!({
                        "revision": updated.revision,
                        "sections": sections,
                        "hotspotCount": knowledge.hotspots.len(),
                        "staleHotspotCount": knowledge
                            .hotspots
                            .iter()
                            .filter(|hotspot| hotspot.status.is_stale())
                            .count(),
                        "stepPoseCount": knowledge.step_poses.len(),
                        "reviewedEntityCount": overlay
                            .entities
                            .values()
                            .filter(|entry| entry.is_reviewed())
                            .count(),
                        "modelReviewLoaded": overlay
                            .model_review
                            .as_ref()
                            .is_some_and(|review| review.loaded),
                        "modelReviewConfirmed": overlay
                            .model_review
                            .as_ref()
                            .is_some_and(|review| review.user_confirmed),
                    })
                    .to_string(),
                ),
            },
            now,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(updated)
}

/// 读取并解析草稿的复核覆盖层（读取端用；损坏时给完整性错误，不猜）。
pub fn parse_review_overlay(
    review_json: &Option<serde_json::Value>,
) -> Result<ReviewOverlay, DraftServiceError> {
    match review_json {
        None => Ok(ReviewOverlay::default()),
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
            DraftServiceError::integrity(
                "draft_review_unreadable",
                format!("草稿复核记录不是合法形状：{error}"),
            )
        }),
    }
}

/// 分支未完成时的缺项构造（含阶段状态与最近原因摘要；不含密钥与路径）。
fn branch_missing_item(
    code: &str,
    branch: &str,
    kind: StageKind,
    status: JobStatus,
    last_error: Option<&str>,
) -> DraftMissingItem {
    // 文案口径（T17 修复 T15 P3①）：**不**在这里承诺"可对该阶段重试"。
    // 能否重试取决于阶段状态、同分支未对账提交与该分支的预算背书，判定在
    // `jobs::control::retry_gate`（任务详情的 `stages[].retry` 与 retry 端点同源）。
    // 这里只说明"发生了什么"，把"可执行动作"交给任务中心照实呈现，
    // 避免出现"写着可重试、点了被 422 budgetNotHolding 拒绝"的分叉文案。
    let detail = match status {
        JobStatus::SubmissionUnknown => {
            "付费提交结果未知：请管理员先对账（不自动重试、不重复购买）".to_owned()
        }
        JobStatus::NeedsInput => last_error
            .map(|error| {
                format!(
                    "{}；请在任务中心查看该阶段可用的恢复动作（重试需要该分支仍有预算背书）",
                    short_summary(error)
                )
            })
            .unwrap_or_else(|| "需要补充输入：请在任务中心查看缺项与可用的恢复动作".to_owned()),
        JobStatus::Failed => last_error
            .map(|error| {
                format!(
                    "{}；请在任务中心查看该阶段可用的恢复动作",
                    short_summary(error)
                )
            })
            .unwrap_or_else(|| "阶段失败：请在任务中心查看可用的恢复动作".to_owned()),
        JobStatus::Cancelled => "阶段已取消".to_owned(),
        JobStatus::Queued
        | JobStatus::RetryWait
        | JobStatus::Running
        | JobStatus::WaitingProvider => {
            "分支仍在执行：草稿为当前可见的部分结果，完成后会自动更新".to_owned()
        }
        JobStatus::Succeeded => "已成功（不应出现在缺项里）".to_owned(),
    };
    DraftMissingItem::new(
        code,
        format!(
            "{branch}（{} = {}）：{detail}",
            kind.as_str(),
            status.as_str()
        ),
    )
}

/// 摘要截断（只存必要摘要；不进入错误路径的原文）。
fn short_summary(text: &str) -> String {
    let mut chars: Vec<char> = text.chars().take(MESSAGE_MAX_CHARS).collect();
    if text.chars().count() > MESSAGE_MAX_CHARS {
        chars.push('…');
    }
    chars.into_iter().collect()
}

/// 读取资产内容（资产 → blob → 文件；校验状态与存在性）。
async fn read_asset_bytes(
    conn: &mut SqliteConnection,
    data_dir: &Path,
    asset_id: &str,
) -> Result<Vec<u8>, DraftServiceError> {
    let (_asset, blob) = repo::assets::get_with_blob(&mut *conn, asset_id)
        .await?
        .ok_or_else(|| {
            DraftServiceError::integrity(
                "draft_result_asset_missing",
                format!("结果资产不存在（{asset_id}）：请人工核对（不重新请求）"),
            )
        })?;
    if blob.storage_state != manual_core::domain::BlobStorageState::Stored {
        return Err(DraftServiceError::integrity(
            "draft_result_asset_unavailable",
            format!(
                "结果资产内容状态异常（{}）：请核对 data-dir 完整性",
                blob.storage_state.as_str()
            ),
        ));
    }
    let path = crate::assets::blob_path(data_dir, &blob.sha256);
    tokio::fs::read(&path).await.map_err(|error| {
        DraftServiceError::integrity(
            "draft_result_asset_unreadable",
            format!("结果资产文件不可读：{error}"),
        )
    })
}
