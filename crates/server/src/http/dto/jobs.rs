//! 任务读取与控制 DTO（T15 / REQ-025、REQ-026、REQ-030、REQ-031）。
//!
//! 机器合同（contracts.md §1/§3/§5）：
//! - 列表 `{data, nextCursor}`、单项 `{data}`；金额分列不合并（`ReservationDto` 复用 T11）；
//! - `cancel` / `retry` / `reconcile` 的响应携带**后果说明**（不声称已取消远端付费操作、
//!   不把未知当成失败、明确替代提交可能重复收费）；
//! - `stageSummary` 的聚合口径见 [`JobStageSummaryDto`]（不是线性百分比，UI 不得换算成百分比）。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use manual_core::timestamps::Timestamp;

use super::ReservationDto;

// ---------------------------------------------------------------------------
// GET /jobs
// ---------------------------------------------------------------------------

/// `GET /api/v1/jobs` 响应（游标分页；与既有列表同形 `{data, nextCursor}`）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobListResponse {
    pub data: Vec<JobSummaryDto>,
    /// 不透明游标（原样回传；已绑定产生它的过滤条件）；无下一页时为 null
    /// （与既有列表同一形状：`{data, nextCursor}`，contracts §1）。
    #[schema(nullable = true)]
    pub next_cursor: Option<String>,
}

/// 任务中心列表行（UI-029）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobSummaryDto {
    pub id: String,
    pub item_id: String,
    /// 物品名称/型号（列表展示；物品归档后仍可读）。
    pub item_name: String,
    pub item_model: String,
    /// 运行时状态（contracts.md §5 的枚举）。
    pub status: String,
    pub revision: i64,
    pub stage_summary: JobStageSummaryDto,
    /// 该任务的费用预留（分列；unknown 表示未决预留，等待对账——不得显示成 0）。
    pub reservations: Vec<ReservationDto>,
    /// 已产出的草稿（可为 null：尚未组装）。
    #[schema(nullable = true)]
    pub draft_id: Option<String>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

/// 阶段摘要（**计数**，不是线性百分比）。
///
/// 口径：`active = queued + running + retry_wait + waiting_provider`（在途）、
/// `blocked = needs_input`（等人工补齐）、`unknown = submission_unknown`（等对账）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobStageSummaryDto {
    pub total: usize,
    pub succeeded: usize,
    pub active: usize,
    pub blocked: usize,
    pub unknown: usize,
    pub failed: usize,
    pub cancelled: usize,
}

// ---------------------------------------------------------------------------
// GET /jobs/{id}
// ---------------------------------------------------------------------------

/// `GET /api/v1/jobs/{id}` 响应（带 `ETag: "r<revision>"`）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobDetailResponse {
    pub data: JobDetailDto,
}

/// 任务详情（UI-030/UI-034/UI-038：阶段、尝试、费用、缺项、错误、下一步）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobDetailDto {
    pub id: String,
    pub item: JobItemDto,
    pub snapshot_id: String,
    pub status: String,
    pub revision: i64,
    /// 阶段明细（含 `needsInput` 缺项与 `knowledgeProduced` 事实）。
    pub stages: Vec<JobStageDto>,
    /// 付费提交 attempt（对账入口需要 attemptId；远端 ID 为 opaque string）。
    pub attempts: Vec<JobAttemptDto>,
    /// 费用预留（分列；unknown 保留预留）。
    pub reservations: Vec<ReservationDto>,
    /// 已产出的草稿（可为 null）。
    #[schema(nullable = true)]
    pub draft_id: Option<String>,
    /// 预算语义说明（与报价/建单同一文案）。
    pub budget_notice: String,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

/// 任务所属物品的展示摘要。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobItemDto {
    pub id: String,
    pub name: String,
    pub model: String,
}

/// 阶段明细行。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobStageDto {
    pub id: String,
    pub stage_kind: String,
    pub batch_index: i64,
    pub status: String,
    /// 本批页号（1-based；非批处理阶段为 null）。
    #[schema(nullable = true)]
    pub page_set: Option<Vec<i64>>,
    pub attempt_count: i64,
    pub poll_count: i64,
    /// 下次自动运行时间（退避/轮询展示；可为 null）。
    #[schema(value_type = Option<String>, nullable = true)]
    pub next_run_at: Option<Timestamp>,
    /// 最近一次失败/阻塞原因摘要（不含密钥与原文）。
    #[schema(nullable = true)]
    pub last_error: Option<String>,
    /// `needs_input` 的可行动缺项。
    pub needs_input: Vec<JobMissingItemDto>,
    /// 结果资产（诊断/明细入口；不含内容）。
    #[schema(nullable = true)]
    pub result_asset_id: Option<String>,
    /// 阶段事实摘要（模型 revision、批次 outcome/计数、用量等）。
    /// **临时供应商 URL 不进入 API 响应**：T20/BUG-008 起新事实只保存
    /// `{"redacted":true,"host":…,"sha256":…}` 摘要；历史行里的 URL 形态字符串
    /// 在读取时替换为同一摘要（contracts §1/§7、AC-010）。
    #[schema(value_type = Option<serde_json::Value>, nullable = true)]
    pub usage: Option<serde_json::Value>,
    /// 仅 `manual_extract`：该批结果事实是否**产出正式知识**
    /// （T14 P3-1：展示与恢复同判据；null = 事实里没有该字段）。
    #[schema(nullable = true)]
    pub knowledge_produced: Option<bool>,
    /// 该阶段当前的**重试准入**（与服务端 `POST /jobs/{id}/retry` 的判定同源）。
    /// UI 只在 `allowed=true` 时渲染重试按钮；否则显示 `reason`/`message` 给出的
    /// 真实恢复路径（T15 P3①：不允许"写着可重试、点了被拒"的分叉文案）。
    pub retry: JobStageRetryDto,
    /// 付费提交形态（`asyncRemoteTask` = Tripo 远端任务：对账可用 attachRemoteTask；
    /// `syncResponse` = 说明书 AI 同步批次：不提供 attachRemoteTask）；非提交阶段为 null。
    #[schema(nullable = true)]
    pub submission_style: Option<String>,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

/// 阶段重试准入（判定见 `jobs::control::retry_gate`）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobStageRetryDto {
    pub allowed: bool,
    /// 拒绝码（`jobCancelled` / `stageNotRetryable` / `branchSubmissionUnknown` /
    /// `budgetNotHolding`）；`allowed=true` 时为 null。
    #[schema(nullable = true)]
    pub reason: Option<String>,
    /// 可行动的说明（与端点 422 `error.message` 同源）；`allowed=true` 时为 null。
    #[schema(nullable = true)]
    pub message: Option<String>,
}

/// 可行动缺项（`needs_input`）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobMissingItemDto {
    pub code: String,
    pub message: String,
}

/// 付费提交 attempt（对账面板数据）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobAttemptDto {
    pub id: String,
    pub stage_id: String,
    /// `intent` / `submitting` / `accepted` / `unknown` / `failed`。
    pub submit_state: String,
    /// 远端 task ID（opaque string；对账时人工比对，不校验格式）。
    #[schema(nullable = true)]
    pub remote_task_id: Option<String>,
    /// 同步响应 ID（**不假定可轮询/重取**）。
    #[schema(nullable = true)]
    pub response_id: Option<String>,
    #[schema(value_type = String)]
    pub started_at: Timestamp,
    #[schema(nullable = true)]
    pub last_error: Option<String>,
}

// ---------------------------------------------------------------------------
// POST /jobs/{id}/cancel
// ---------------------------------------------------------------------------

/// 取消响应。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CancelResponse {
    pub data: CancelResultDto,
}

/// 取消结果：语义必须如实（不声称已取消远端付费操作）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CancelResultDto {
    pub job: JobDetailDto,
    /// 本次转为 cancelled 的未提交阶段数。
    pub stages_cancelled: u64,
    /// 保留原状态的已提交/未决阶段（保留查询与账务）。
    pub preserved_stages: Vec<PreservedStageDto>,
    /// 固定文案：取消只停止本地推进，不撤销远端付费操作（不保证供应商撤单）。
    pub notice: String,
}

/// 被取消动作保留的阶段。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreservedStageDto {
    pub stage_id: String,
    pub stage_kind: String,
    pub status: String,
}

// ---------------------------------------------------------------------------
// POST /jobs/{id}/retry
// ---------------------------------------------------------------------------

/// `POST /api/v1/jobs/{id}/retry` 请求体（`Idempotency-Key` 头必填）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryRequest {
    /// 要重试的阶段（来自任务详情的阶段 id）。
    #[schema(nullable = true)]
    pub stage_id: Option<String>,
}

/// 重试响应。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RetryResponse {
    pub data: RetryResultDto,
}

/// 重试结果：只重跑指定阶段，已完成成果保留。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetryResultDto {
    pub job: JobDetailDto,
    pub stage_id: String,
    pub stage_kind: String,
    /// 重试前的阶段状态（`failed` / `needs_input`）。
    pub previous_status: String,
    /// 一并拉回队列的已完成下游阶段数（例如基于旧输入的部分草稿组装）。
    pub requeued_dependents: u64,
    /// 固定说明（"已完成部分不会被覆盖"）。
    pub notice: String,
}

// ---------------------------------------------------------------------------
// POST /jobs/{id}/reconcile
// ---------------------------------------------------------------------------

/// 对账动作（线上取值 camelCase）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReconcileActionDto {
    /// 仅 Tripo：附加账户中查到的任务 ID（查询验证 + 二次确认）。
    AttachRemoteTask,
    /// 记录"账户中无对应任务"的核查证据（管理员声明）。
    RecordNoTask,
    /// 授权替代提交（再次预算确认 + 重复收费风险确认）。
    AuthorizeReplacement,
}

/// `POST /api/v1/jobs/{id}/reconcile` 请求体。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReconcileRequestDto {
    #[schema(nullable = true)]
    pub action: Option<ReconcileActionDto>,
    /// 处于 `submission_unknown` 的阶段 id。
    #[schema(nullable = true)]
    pub stage_id: Option<String>,
    /// `attachRemoteTask`：账户中查到的远端任务 ID（opaque string，原样填写）。
    #[schema(nullable = true)]
    pub remote_task_id: Option<String>,
    /// `attachRemoteTask`：二次确认"该任务与本次未决提交对应"。
    #[schema(nullable = true)]
    pub acknowledge_matches: Option<bool>,
    /// `recordNoTask`：核查证据（管理员声明，不是供应商出具的证明）。
    #[schema(nullable = true)]
    pub evidence: Option<String>,
    /// `authorizeReplacement`：确认"可能重复收费"的风险。
    #[schema(nullable = true)]
    pub acknowledge_duplicate_risk: Option<bool>,
    /// `authorizeReplacement`：再次预算确认（必须覆盖冻结上界）。
    #[schema(nullable = true)]
    pub limits: Option<super::BudgetLimitsDto>,
}

/// 对账响应。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReconcileResponse {
    pub data: ReconcileResultDto,
}

/// 对账结果。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileResultDto {
    pub job: JobDetailDto,
    pub action: ReconcileActionDto,
    pub stage_id: String,
    /// 对账后阶段的目标状态（`queued` / `needs_input`）。
    pub stage_status: String,
    #[schema(nullable = true)]
    pub attempt_id: Option<String>,
    /// 动作后果的如实说明（不伪造证明、不自动释放预留、明确重复收费风险）。
    pub notice: String,
}
