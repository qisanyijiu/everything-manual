//! 说明书 AI 阶段处理器（`manual_extract`（按 `batch_index`）/ `manual_merge`）。
//!
//! 与执行器的分工（contracts.md §5；**不改变 T10 语义**）：
//! - 处理器只负责"做什么"（读冻结输入、发一次 HTTP、保存**事实**）；
//! - 业务状态推进（`retry_wait` / `needs_input` / `submission_unknown` / `succeeded`）
//!   由执行器带租约 epoch guard 完成；处理器返回 [`StageOutcome`] 表达结论；
//! - 付费提交走 [`StageContext::submission`] 的提交窗口：先 intent、再 submitting、
//!   再**先持久化结果资产**、最后在同一短事务写 usage/receipt；
//! - **同步链路恢复语义**：完整响应已持久化（`result_asset_id` 非空）→ 恢复补推进、
//!   不重跑；已发请求但完整响应未持久化 → 该批 `submission_unknown`，`response_id`
//!   不假定可轮询/重取，且**绝不自动重发**（ADR-006）。
//!
//! 阶段行为（AC-045 / AC-046）：
//!
//! | 阶段 | 输入（冻结） | 输出/结论 |
//! | --- | --- | --- |
//! | `manual_extract`（batch_index） | 快照（preparation/模型/prompt 版本/价格版本）、已确认发送范围、本批 `page_set` | `POST {base_url}/responses`（`text.format` JSON Schema、strict、≤5 页/批）→ 解析 `output[].content[]`；拒答/incomplete/截断/畸形 JSON/引用校验失败 → **不产生正式知识**（`needs_input` + 结果/诊断资产）；正常 → 批次结果资产 + `succeeded` |
//! | `manual_merge` | 全部 `manual_extract` 批次的结果资产 | 本地确定性合并（去重保留出处、同名不同事实保留冲突）→ 合并结果资产；覆盖不完整 → `needs_input`（**不解锁下游**，不调用 AI） |
//!
//! **超预算不再请求**：每个批次在发请求前重新计算冻结计划（`plan_manual_ai`，
//! 与报价/建单同源），要求本阶段的 `page_set` 与计划批次一致、`max_output_tokens`
//! 不超过每批上限、快照的说明书 AI 预留**仍占用预算**（reserved/unknown）且授权上限
//! 覆盖保守上界；任一不满足 → `needs_input` 且**零请求**（fixture 计数断言）。
//!
//! **提示注入防护**：资料是待分析数据不是指令（见 [`super::prompt`]）；请求体只由
//! 冻结输入构造（预算/模型/页集合不可由页内容改变）；模型无工具权限（请求里不存在
//! `tools`/`functions`/URL 参数）；服务端只接受"存在输入页 + 同批部件"的引用；
//! 页内容中的任何文字都不会被本模块解析为 URL 或命令。

use std::path::PathBuf;
use std::sync::Arc;

use manual_core::domain::{CostLedgerEntry, JobStatus, ProviderKey, StageKind};
use manual_core::generation::{
    MANUAL_AI_MAX_OUTPUT_TOKENS_PER_BATCH, MANUAL_EXTRACT_PROMPT_VERSION, plan_manual_ai,
};
use manual_core::knowledge::{
    BatchExtractionResult, BatchOutcome, BatchValidationContext, MergedKnowledge, merge_batches,
    validate_batch_output,
};
use manual_core::timestamps::Timestamp;
use serde_json::json;
use sqlx::SqliteConnection;

use crate::config::Settings;
use crate::generation::estimate::quote_payload;
use crate::generation::ledger as generation_ledger;
use crate::jobs::submission::record_result_fact;
use crate::jobs::{
    JobError, MissingItem, StageContext, StageFuture, StageHandler, StageOutcome, StageRegistry,
};
use crate::storage::repo;

use super::client::{MANUAL_AI_RESPONSES_PATH, ManualAiClient, ManualAiError, ManualAiTimeouts};
use super::dto::{ExtractRequest, InputContent, jpeg_data_url};
use super::prompt::{PromptPage, build_batch_prompt};
use super::store::{self, DerivedAssetError};

/// 错误/结论的稳定代码（`needs_input` 缺项与日志；QA/前端按这些码断言）。
pub const CODE_SNAPSHOT_MISSING: &str = "manual_snapshot_missing";
pub const CODE_SEND_SCOPE_MISSING: &str = "manual_send_scope_missing";
pub const CODE_SEND_SCOPE_MISMATCH: &str = "manual_send_scope_mismatch";
pub const CODE_PROMPT_VERSION_UNSUPPORTED: &str = "manual_prompt_version_unsupported";
pub const CODE_BATCH_NOT_IN_PLAN: &str = "manual_batch_not_in_frozen_plan";
pub const CODE_OUTPUT_TOKEN_LIMIT: &str = "manual_output_token_limit_exceeded";
pub const CODE_BUDGET_NOT_HOLDING: &str = "manual_budget_not_holding";
pub const CODE_BRANCH_PAUSED: &str = "manual_branch_paused_by_unknown";
pub const CODE_PAGE_PAYLOAD: &str = "manual_page_payload_unavailable";
pub const CODE_REFUSAL: &str = "manual_ai_refusal";
pub const CODE_INCOMPLETE: &str = "manual_ai_incomplete";
pub const CODE_EMPTY_OUTPUT: &str = "manual_ai_empty_output";
pub const CODE_ENVELOPE_INVALID: &str = "manual_ai_envelope_invalid";
pub const CODE_RESPONSE_FAILED: &str = "manual_ai_response_failed";
pub const CODE_REQUEST_REJECTED: &str = "manual_ai_request_rejected";
pub const CODE_MERGE_INPUT_MISSING: &str = "manual_merge_input_missing";

/// 处理器集合（`serve` 按配置注册；未配置时**不注册任何处理器**，
/// 已入队阶段被延后而不是假成功）。
pub struct ManualAiHandlers {
    client: Arc<ManualAiClient>,
    data_dir: PathBuf,
}

impl ManualAiHandlers {
    /// 从配置构造（要求 `providers.manual_ai` 已配置：api_key + model 在场）。
    pub fn from_settings(settings: &Settings) -> Result<Self, String> {
        let provider = &settings.providers.manual_ai;
        let api_key = provider
            .api_key
            .clone()
            .ok_or_else(|| "providers.manual_ai.api_key 未配置（说明书 AI 不可用）".to_owned())?;
        // 模型名是"已配置"的判据之一（`configured()`）；**请求实际使用的模型来自任务
        // 快照**（不跟随当前配置），因此处理器本身不保存它。
        if provider.model.is_none() {
            return Err("providers.manual_ai.model 未配置（说明书 AI 不可用）".to_owned());
        }
        let client = ManualAiClient::new(&provider.base_url, api_key, ManualAiTimeouts::default())?;
        Ok(Self {
            client: Arc::new(client),
            data_dir: settings.data_dir.clone(),
        })
    }

    /// 生效的 base_url（脱敏展示；不含密钥）。
    pub fn base_url(&self) -> &str {
        self.client.base_url()
    }

    /// 注册两个阶段处理器（`manual_extract` / `manual_merge`）。
    pub fn register(&self, registry: &mut StageRegistry) {
        registry.register(
            StageKind::ManualExtract,
            ManualExtractHandler::new(Arc::clone(&self.client), self.data_dir.clone()),
        );
        registry.register(
            StageKind::ManualMerge,
            ManualMergeHandler::new(self.data_dir.clone()),
        );
    }
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

/// 处理器内部的"立即结论"：输入/协议问题直接给出 StageOutcome，不再继续。
struct HandlerStop(StageOutcome);

type Loaded<T> = Result<T, HandlerStop>;

fn needs_input(code: &str, message: impl Into<String>) -> HandlerStop {
    HandlerStop(StageOutcome::NeedsInput {
        items: vec![MissingItem::new(code, message)],
    })
}

fn internal<E: Into<JobError>>(error: E) -> HandlerStop {
    HandlerStop(StageOutcome::Failed {
        reason: format!("内部错误：{}", error.into()),
    })
}

fn short_summary(text: &str, limit: usize) -> String {
    let mut chars: Vec<char> = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        chars.push('…');
    }
    chars.into_iter().collect()
}

fn join_pages(pages: &[i64]) -> String {
    pages
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

// ---------------------------------------------------------------------------
// 冻结输入（快照 + 已确认发送范围 + 重新计算的计划）
// ---------------------------------------------------------------------------

/// 本批执行所需的冻结输入（全部来自快照/报价/准备记录，**不读当前配置**）。
#[derive(Debug, Clone)]
struct FrozenInputs {
    item_name: String,
    item_model: String,
    model: String,
    prompt_version: String,
    schema_version: String,
    document_id: String,
    preparation_id: String,
    price_version: String,
    max_output_tokens: i64,
    /// 本批页（1-based，来自冻结计划的批次）。
    pages: Vec<i64>,
    /// 本批以页图发送的页（扫描页/无文字层页）。
    image_pages: Vec<i64>,
}

/// 读取并交叉校验冻结输入；任何不一致 → `needs_input`（不猜测、不降级）。
async fn load_frozen_inputs(ctx: &StageContext) -> Loaded<FrozenInputs> {
    let mut conn = ctx.pool.acquire().await.map_err(internal)?;
    let snapshot = repo::snapshots::get(&mut conn, &ctx.job.snapshot_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            needs_input(
                CODE_SNAPSHOT_MISSING,
                "任务快照不存在：无法确认冻结输入（不猜测、不重新购买）",
            )
        })?;

    let quote_id = snapshot
        .budgets
        .get("quoteId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            needs_input(
                CODE_SEND_SCOPE_MISSING,
                "快照缺少 quoteId：无法回读用户已确认的发送范围（不猜测发送内容）",
            )
        })?;
    let quote = repo::quotes::get(&mut conn, &quote_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            needs_input(
                CODE_SEND_SCOPE_MISSING,
                "报价记录不存在：无法回读用户已确认的发送范围（不猜测发送内容）",
            )
        })?;
    let payload = quote_payload(&quote).await.map_err(|error| {
        needs_input(
            CODE_SEND_SCOPE_MISSING,
            format!("报价载荷无法回读：{error:?}（不猜测发送内容）"),
        )
    })?;
    let scope = payload.send_scope.manual_ai;

    // prompt/schema 版本绑定：快照、报价发送范围与代码支持的版本必须一致。
    if snapshot.prompt_version != MANUAL_EXTRACT_PROMPT_VERSION
        || scope.prompt_version != MANUAL_EXTRACT_PROMPT_VERSION
    {
        return Err(needs_input(
            CODE_PROMPT_VERSION_UNSUPPORTED,
            format!(
                "提示词版本不一致（快照 {}、发送范围 {}、支持 {}）：拒绝执行",
                snapshot.prompt_version, scope.prompt_version, MANUAL_EXTRACT_PROMPT_VERSION
            ),
        ));
    }
    // 模型来自快照冻结配置（**不跟随当前配置**）；与报价发送范围交叉核对。
    let frozen_model = snapshot
        .provider_config
        .get("manualAi")
        .and_then(|manual| manual.get("model"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            needs_input(
                CODE_SEND_SCOPE_MISSING,
                "快照缺少 manualAi.model：无法确认冻结模型（不猜测）",
            )
        })?;
    if frozen_model != scope.model {
        return Err(needs_input(
            CODE_SEND_SCOPE_MISMATCH,
            "快照模型与已确认发送范围的模型不一致：拒绝按不一致的配置发起付费请求",
        ));
    }

    // 准备记录（documentId 按 preparation → document 反查）。
    let preparation = repo::preparations::get(&mut conn, &snapshot.preparation_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            needs_input(
                CODE_SNAPSHOT_MISSING,
                "准备记录不存在：无法确认冻结输入（不猜测）",
            )
        })?;

    // 重新计算冻结计划（与报价/建单同源），并校验本批的 page_set。
    let page_inputs = repo::preparations::page_quote_inputs(&mut conn, &snapshot.preparation_id)
        .await
        .map_err(internal)?;
    let plan = plan_manual_ai(&page_inputs).map_err(|error| {
        needs_input(
            CODE_BATCH_NOT_IN_PLAN,
            format!("无法从冻结准备记录计算计划：{error}"),
        )
    })?;
    let batch_index = ctx.stage.batch_index;
    let plan_batch = plan.batches.get(batch_index as usize).ok_or_else(|| {
        needs_input(
            CODE_BATCH_NOT_IN_PLAN,
            format!(
                "本批 batchIndex={batch_index} 超出冻结计划的批次数（{}）：拒绝发起计划外请求",
                plan.batches.len()
            ),
        )
    })?;
    let stage_pages = ctx.stage.page_set.clone().ok_or_else(|| {
        needs_input(
            CODE_BATCH_NOT_IN_PLAN,
            "本批缺少 page_set：无法确认冻结输入（不猜测页集合）",
        )
    })?;
    if &stage_pages != plan_batch {
        return Err(needs_input(
            CODE_BATCH_NOT_IN_PLAN,
            format!(
                "本批 page_set（{}）与冻结计划批次（{}）不一致：拒绝按计划外页集合发起请求",
                join_pages(&stage_pages),
                join_pages(plan_batch)
            ),
        ));
    }
    // 与用户已确认的发送范围交叉核对（页集合）。
    let mut scope_pages: Vec<i64> = scope
        .text_pages
        .iter()
        .chain(scope.image_pages.iter())
        .copied()
        .collect();
    scope_pages.sort_unstable();
    if scope_pages != plan.pages_overall() {
        return Err(needs_input(
            CODE_SEND_SCOPE_MISMATCH,
            "计划页集合与已确认发送范围不一致：拒绝按不一致的内容发起请求",
        ));
    }

    // 输出 token 上限（“超预算不再请求”）：冻结发送范围给出的是**全部批次**的
    // 输出上限（= 批次数 × 每批上限）；单批请求不得超过每批上限，也不得让
    // "批次数 × 每批上限" 超出冻结上限（否则属于计划外支出 → 拒绝执行）。
    let batch_count = plan.batches.len() as i64;
    let required_total = MANUAL_AI_MAX_OUTPUT_TOKENS_PER_BATCH.saturating_mul(batch_count.max(1));
    if scope.max_output_tokens < required_total {
        return Err(needs_input(
            CODE_OUTPUT_TOKEN_LIMIT,
            format!(
                "冻结发送范围的输出 token 上限（{}）低于计划上界（{required_total} = {batch_count} 批 × {}）：\
                 拒绝发起超预算请求（不自动降级/换模型）",
                scope.max_output_tokens, MANUAL_AI_MAX_OUTPUT_TOKENS_PER_BATCH
            ),
        ));
    }

    // 预算占用：快照的说明书 AI 预留必须仍占用预算，且授权上限覆盖保守上界。
    let entries = repo::ledger::list_for_snapshot(&mut conn, &snapshot.id)
        .await
        .map_err(internal)?;
    check_budget_holding(&entries, &snapshot.budgets)?;

    // 分支暂停：同 job 的其它批次结果未知 → 不再发起本分支的新购买。
    let stages = repo::job_stages::list_for_job(&mut conn, &ctx.job.id)
        .await
        .map_err(internal)?;
    if let Some(paused) = stages.iter().find(|stage| {
        stage.stage_kind == StageKind::ManualExtract
            && stage.id != ctx.stage.id
            && stage.status == JobStatus::SubmissionUnknown
    }) {
        return Err(needs_input(
            CODE_BRANCH_PAUSED,
            format!(
                "同分支批次 {} 结果未知（submission_unknown）：暂停该分支后续购买，\
                 需管理员对账后继续（本批不发起新请求）",
                paused.batch_index
            ),
        ));
    }

    let image_pages: Vec<i64> = plan_batch
        .iter()
        .copied()
        .filter(|page| plan.image_pages.contains(page))
        .collect();

    Ok(FrozenInputs {
        item_name: scope.item_name,
        item_model: scope.item_model,
        model: frozen_model,
        prompt_version: snapshot.prompt_version.clone(),
        schema_version: manual_core::knowledge::MANUAL_EXTRACT_SCHEMA_VERSION.to_owned(),
        document_id: preparation.document_id.clone(),
        preparation_id: snapshot.preparation_id.clone(),
        price_version: snapshot.price_version.clone(),
        // 单批请求上限 = 每批上限（冻结范围是全部批次的上限，已在上方校验足够）。
        max_output_tokens: MANUAL_AI_MAX_OUTPUT_TOKENS_PER_BATCH,
        pages: plan_batch.clone(),
        image_pages,
    })
}

/// 预算占用检查（预留必须仍占预算；授权上限必须覆盖保守上界）。
fn check_budget_holding(
    entries: &[CostLedgerEntry],
    budgets: &serde_json::Value,
) -> Result<(), HandlerStop> {
    let entry = entries
        .iter()
        .find(|entry| entry.provider == ProviderKey::ManualAi);
    let Some(entry) = entry else {
        return Err(needs_input(
            CODE_BUDGET_NOT_HOLDING,
            "该快照没有说明书 AI 的费用预留：拒绝发起无预算背书的请求（请检查任务快照）",
        ));
    };
    if !manual_core::cost::ledger_state_holds_budget(entry.state) {
        return Err(needs_input(
            CODE_BUDGET_NOT_HOLDING,
            format!(
                "说明书 AI 预留不占用预算（state={}）：拒绝发起超出授权估算的请求",
                entry.state.as_str()
            ),
        ));
    }
    let authorized = budgets
        .get("authorized")
        .and_then(|value| value.get("manualAiUsdMicros"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let upper_bound = budgets
        .get("upperBound")
        .and_then(|value| value.get("manualAiUsdMicros"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(i64::MAX);
    if authorized < upper_bound {
        return Err(needs_input(
            CODE_BUDGET_NOT_HOLDING,
            format!(
                "授权上限（{authorized} usdMicros）低于保守上界（{upper_bound} usdMicros）：\
                 拒绝发起请求（不自动降质量/换模型）"
            ),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 页负载（文字/页图）
// ---------------------------------------------------------------------------

/// 单页负载（文字页带正文；页图页带 JPEG 字节）。
#[derive(Debug, Clone, PartialEq)]
enum PagePayload {
    Text { page: i64, text: String },
    Image { page: i64, bytes: Vec<u8> },
}

/// 读取本批页负载。
///
/// 分类遵循**冻结计划**（文字层字节数 > 0 = 文字页；空/无文字层 = 页图页，
/// architecture §5.2「扫描页文字为空时发送页图」）；内容缺失/类型不符 → `needs_input`。
async fn load_page_payloads(
    ctx: &StageContext,
    data_dir: &std::path::Path,
    inputs: &FrozenInputs,
) -> Loaded<Vec<PagePayload>> {
    let mut payloads = Vec::with_capacity(inputs.pages.len());
    for page_number in &inputs.pages {
        let planned_image = inputs.image_pages.contains(page_number);
        let mut conn = ctx.pool.acquire().await.map_err(internal)?;
        let page = repo::preparations::get_page(&mut conn, &inputs.preparation_id, *page_number)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                needs_input(
                    CODE_PAGE_PAYLOAD,
                    format!("第 {page_number} 页不在准备记录中：请核对资料完整性"),
                )
            })?;
        if planned_image {
            let asset_id = page.image_asset_id.clone().ok_or_else(|| {
                needs_input(
                    CODE_PAGE_PAYLOAD,
                    format!("第 {page_number} 页没有页图资产：无法发送页图（请补齐准备记录）"),
                )
            })?;
            let (mime, bytes) = read_asset(&mut conn, data_dir, &asset_id).await?;
            if mime != "image/jpeg" || !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                return Err(needs_input(
                    CODE_PAGE_PAYLOAD,
                    format!(
                        "第 {page_number} 页的页图不是 JPEG（mime={mime}）：\
                         本卡只发送 JPEG data URL，请重新准备该页"
                    ),
                ));
            }
            payloads.push(PagePayload::Image {
                page: *page_number,
                bytes,
            });
        } else {
            let asset_id = page.text_asset_id.clone().ok_or_else(|| {
                needs_input(
                    CODE_PAGE_PAYLOAD,
                    format!("第 {page_number} 页缺少页文字资产：无法确认页内容（请补齐准备记录）"),
                )
            })?;
            let (mime, bytes) = read_asset(&mut conn, data_dir, &asset_id).await?;
            if !mime.starts_with("text/") {
                return Err(needs_input(
                    CODE_PAGE_PAYLOAD,
                    format!("第 {page_number} 页的文字资产类型异常（mime={mime}）：请重新准备该页"),
                ));
            }
            let text = String::from_utf8(bytes).map_err(|_| {
                needs_input(
                    CODE_PAGE_PAYLOAD,
                    format!("第 {page_number} 页的文字不是合法 UTF-8：请重新准备该页"),
                )
            })?;
            if text.trim().is_empty() {
                return Err(needs_input(
                    CODE_PAGE_PAYLOAD,
                    format!("第 {page_number} 页的文字为空：请重新准备该页"),
                ));
            }
            payloads.push(PagePayload::Text {
                page: *page_number,
                text,
            });
        }
    }
    Ok(payloads)
}

/// 读取资产内容（校验 blob 状态与文件存在性）。
async fn read_asset(
    conn: &mut SqliteConnection,
    data_dir: &std::path::Path,
    asset_id: &str,
) -> Loaded<(String, Vec<u8>)> {
    let (_asset, blob) = repo::assets::get_with_blob(&mut *conn, asset_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            needs_input(
                CODE_PAGE_PAYLOAD,
                "页资产不存在（完整性异常）：请核对资料完整性",
            )
        })?;
    let bytes = store::read_blob_bytes(&mut *conn, data_dir, &blob.sha256)
        .await
        .map_err(|error| needs_input(CODE_PAGE_PAYLOAD, error.detail.clone()))?;
    Ok((blob.mime, bytes))
}

// ---------------------------------------------------------------------------
// manual_extract
// ---------------------------------------------------------------------------

/// `manual_extract`：对一批（≤5 页）调用一次 Responses 提取。
pub struct ManualExtractHandler {
    client: Arc<ManualAiClient>,
    data_dir: PathBuf,
}

impl ManualExtractHandler {
    pub fn new(client: Arc<ManualAiClient>, data_dir: PathBuf) -> Self {
        Self { client, data_dir }
    }

    /// 本批请求体（确定性字节；`request_hash` 与线上 body 同源）。
    fn build_request(inputs: &FrozenInputs, payloads: &[PagePayload]) -> ExtractRequest {
        let prompt_pages: Vec<PromptPage<'_>> = payloads
            .iter()
            .map(|payload| match payload {
                PagePayload::Text { page, text } => PromptPage::Text {
                    page_number: *page,
                    text,
                },
                PagePayload::Image { page, .. } => PromptPage::Image { page_number: *page },
            })
            .collect();
        let prompt = build_batch_prompt(&inputs.item_name, &inputs.item_model, &prompt_pages);
        let mut content = vec![InputContent::InputText { text: prompt }];
        for payload in payloads {
            if let PagePayload::Image { bytes, .. } = payload {
                content.push(InputContent::InputImage {
                    image_url: jpeg_data_url(bytes),
                });
            }
        }
        ExtractRequest::new(&inputs.model, content, inputs.max_output_tokens)
    }

    /// 响应结论 → (`BatchOutcome`, 错误码, 摘要)；`None` = 可以尝试解析结构化输出。
    ///
    /// **提供方文本一律先脱敏**（OB-11 / ADR-034）：refusal / incompleteReason /
    /// 信封解析错误的原文可能内嵌带签名的临时 URL，该摘要会进入三种持久化产物
    /// （`usage_json.errorSummary`、批次结果资产 blob、`needs_input_json` 的 message），
    /// 因此在**最早的产生点**走统一入口（仓储写入侧另有兜底）。
    fn failure_of(
        parsed: Option<&super::dto::ParsedResponse>,
        envelope_error: Option<&str>,
    ) -> Option<(BatchOutcome, &'static str, String)> {
        Self::failure_of_raw(parsed, envelope_error).map(|(outcome, code, summary)| {
            (outcome, code, crate::redaction::redact_text_urls(&summary))
        })
    }

    /// [`Self::failure_of`] 的未脱敏形态（只在同一模块内使用）。
    fn failure_of_raw(
        parsed: Option<&super::dto::ParsedResponse>,
        envelope_error: Option<&str>,
    ) -> Option<(BatchOutcome, &'static str, String)> {
        if let Some(detail) = envelope_error {
            return Some((
                BatchOutcome::EnvelopeInvalid,
                CODE_ENVELOPE_INVALID,
                format!("响应信封不可解析（原始响应已保存为诊断）：{detail}"),
            ));
        }
        let parsed = parsed.expect("无信封错误时必有解析结果");
        if parsed.has_refusal {
            let text = parsed
                .refusal
                .as_deref()
                .map(|value| short_summary(value, 200))
                .unwrap_or_else(|| "（未给出原因）".to_owned());
            return Some((
                BatchOutcome::Refusal,
                CODE_REFUSAL,
                format!("模型拒答（refusal）：{text}"),
            ));
        }
        if !parsed.is_completed() {
            let status = parsed
                .status
                .clone()
                .unwrap_or_else(|| "unknown".to_owned());
            if status == "incomplete" {
                let reason = parsed
                    .incomplete_reason
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned());
                return Some((
                    BatchOutcome::Incomplete,
                    CODE_INCOMPLETE,
                    format!("响应被截断（incomplete: {reason}）：该批不产生正式知识"),
                ));
            }
            return Some((
                BatchOutcome::ResponseFailed,
                CODE_RESPONSE_FAILED,
                format!("供应商响应状态为 {status}：该批不产生正式知识"),
            ));
        }
        match parsed.output_text.as_deref() {
            None => Some((
                BatchOutcome::EmptyOutput,
                CODE_EMPTY_OUTPUT,
                "响应中没有 output_text：该批不产生正式知识".to_owned(),
            )),
            Some(text) if text.trim().is_empty() => Some((
                BatchOutcome::EmptyOutput,
                CODE_EMPTY_OUTPUT,
                "output_text 为空：该批不产生正式知识".to_owned(),
            )),
            Some(_) => None,
        }
    }
}

impl StageHandler for ManualExtractHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            // 0) 同步链路没有"远端可轮询任务"：已有远端事实属异常形态，拒绝继续。
            if ctx.known_remote_task_id().is_some() {
                return Ok(needs_input(
                    CODE_ENVELOPE_INVALID,
                    "该批次存在远端 task 事实（同步链路不应出现）：请人工核对（不重新请求）",
                )
                .0);
            }

            // 1) 冻结输入与预算核对（任何一项不满足 → 零请求）。
            let inputs = match load_frozen_inputs(ctx).await {
                Ok(inputs) => inputs,
                Err(stop) => return Ok(stop.0),
            };

            // 2) 页负载（文字页正文 / 扫描页 JPEG data URL）。
            let payloads = match load_page_payloads(ctx, &self.data_dir, &inputs).await {
                Ok(payloads) => payloads,
                Err(stop) => return Ok(stop.0),
            };

            // 3) 请求体（确定性字节）。
            let request = Self::build_request(&inputs, &payloads);
            let body = request.to_bytes();
            let request_hash = manual_core::generation::sha256_hex(&body);

            let pool = ctx.pool.clone();
            let job_id = ctx.job.id.clone();
            let item_id = ctx.job.item_id.clone();
            let snapshot_id = ctx.job.snapshot_id.clone();
            let stage_id = ctx.stage.id.clone();
            let batch_index = ctx.stage.batch_index;
            let now = ctx.now;
            let window = &mut ctx.submission;

            // 4) 提交窗口：intent → submitting → 清理陈旧结果事实（fresh 重试）→ 发请求。
            window.begin_intent(&request_hash).await?;
            window.mark_submitting().await?;
            if let Err(error) = repo::job_stages::reset_result_fact(&pool, &stage_id, now).await {
                return Ok(internal(error).0);
            }
            tracing::info!(
                event = "manual_extract_sent",
                jobId = %job_id,
                stageId = %stage_id,
                batchIndex = batch_index,
                pages = %join_pages(&inputs.pages),
                images = inputs.image_pages.len(),
                endpoint = MANUAL_AI_RESPONSES_PATH,
                "说明书提取请求已发出（同步单次请求；未持久化完整响应一律按结果未知处理）"
            );

            let response = self.client.extract_batch(&body).await;
            let raw = match response {
                Ok(raw) => raw,
                Err(error) => {
                    return Ok(http_failure_outcome(
                        &pool,
                        window,
                        &snapshot_id,
                        &job_id,
                        &stage_id,
                        &error,
                        &now,
                    )
                    .await);
                }
            };

            // 5) 解析（宽松读取信封；refusal/incomplete 另行处理）。
            let (parsed, envelope_error) = match ManualAiClient::parse_success(&raw) {
                Ok(parsed) => (Some(parsed), None),
                Err(ManualAiError::Unexpected { detail }) => (None, Some(detail)),
                Err(other) => {
                    // 防御：`parse_success` 只返回 `Unexpected`。
                    return Ok(http_failure_outcome(
                        &pool,
                        window,
                        &snapshot_id,
                        &job_id,
                        &stage_id,
                        &other,
                        &now,
                    )
                    .await);
                }
            };

            // 6) 先持久化**原始响应** blob（受限诊断路径；失败只告警不阻断结论）。
            let diagnostic = match store::persist_derived_asset(
                &pool,
                &self.data_dir,
                &item_id,
                &raw.body,
                &format!("manual_extract_batch_{batch_index}_response.json"),
            )
            .await
            {
                Ok(asset) => Some(asset),
                Err(error) => {
                    tracing::warn!(
                        event = "manual_extract_diagnostic_write_failed",
                        stageId = %stage_id,
                        detail = %error,
                        "原始响应诊断写入失败（继续按结论处理）"
                    );
                    None
                }
            };
            let diagnostic_sha = diagnostic.as_ref().map(|asset| asset.sha256.clone());

            // 7) 结论 → 批次结果（**正式知识只来自 completed + 服务端校验通过**）。
            let pages = &inputs.pages;
            let failure = Self::failure_of(parsed.as_ref(), envelope_error.as_deref());
            let response_id = parsed.as_ref().and_then(|parsed| parsed.id.clone());
            let batch_result = match failure {
                Some((outcome, code, summary)) => BatchExtractionResult::not_produced(
                    batch_index,
                    pages,
                    outcome,
                    &inputs.document_id,
                    &inputs.preparation_id,
                    &inputs.schema_version,
                    &inputs.prompt_version,
                    code,
                    summary,
                    response_id,
                    diagnostic_sha.clone(),
                ),
                None => {
                    let text = parsed
                        .as_ref()
                        .and_then(|parsed| parsed.output_text.clone())
                        .unwrap_or_default();
                    let context = BatchValidationContext {
                        document_id: &inputs.document_id,
                        preparation_id: &inputs.preparation_id,
                        pages,
                        image_pages: &inputs.image_pages,
                        schema_version: &inputs.schema_version,
                    };
                    match validate_batch_output(&text, &context) {
                        Ok(result) => BatchExtractionResult::completed(
                            batch_index,
                            pages,
                            &inputs.document_id,
                            &inputs.preparation_id,
                            &inputs.schema_version,
                            &inputs.prompt_version,
                            result,
                            response_id,
                        ),
                        Err(error) => {
                            let outcome =
                                if error.code == manual_core::knowledge::CODE_INVALID_FORMAT {
                                    // output_text 不是合法 JSON：与"JSON 合法但不符合 schema"分开。
                                    BatchOutcome::InvalidFormat
                                } else {
                                    BatchOutcome::SchemaViolation
                                };
                            tracing::warn!(
                                event = "manual_extract_schema_rejected",
                                jobId = %job_id,
                                stageId = %stage_id,
                                batchIndex = batch_index,
                                errorCode = error.code,
                                "模型输出未通过服务端校验：该批不产生正式知识（不做正则抢救）"
                            );
                            // 校验错误文本可能引用模型输出里的字段值 → 与 failure_of 同规则先脱敏。
                            BatchExtractionResult::not_produced(
                                batch_index,
                                pages,
                                outcome,
                                &inputs.document_id,
                                &inputs.preparation_id,
                                &inputs.schema_version,
                                &inputs.prompt_version,
                                error.code,
                                crate::redaction::redact_text_urls(&error.detail_text()),
                                response_id,
                                diagnostic_sha.clone(),
                            )
                        }
                    }
                }
            };

            let outcome = batch_result.outcome;
            let produced = batch_result.produced_knowledge;
            let error_code = batch_result.error_code.clone();
            let error_summary = batch_result.error_summary.clone();
            let response_id = batch_result.response_id.clone();
            let entity_counts = json!({
                "parts": batch_result.parts.len(),
                "steps": batch_result.steps.len(),
                "specs": batch_result.specs.len(),
                "uncertainties": batch_result.uncertainties.len(),
            });
            let result_bytes = serde_json::to_vec(&batch_result).unwrap_or_default();

            // 8) 持久化批次结果资产（先资产、后 usage/receipt）。
            let result_asset = match store::persist_derived_asset(
                &pool,
                &self.data_dir,
                &item_id,
                &result_bytes,
                &format!("manual_extract_batch_{batch_index}.json"),
            )
            .await
            {
                Ok(asset) => asset,
                Err(error) => return Ok(derived_asset_failure(error).0),
            };

            // 9) usage/receipt（同一短事务；receipt 允许过期 worker 保存）。
            let usage = json!({
                "batchIndex": batch_index,
                "pages": pages,
                "outcome": outcome.as_str(),
                "producedKnowledge": produced,
                "schemaVersion": inputs.schema_version,
                "promptVersion": inputs.prompt_version,
                "model": inputs.model,
                "priceVersion": inputs.price_version,
                "responseId": response_id,
                "diagnosticSha256": diagnostic_sha,
                "usage": parsed.as_ref().and_then(|parsed| parsed.usage).map(|usage| json!({
                    "inputTokens": usage.input_tokens,
                    "outputTokens": usage.output_tokens,
                    "totalTokens": usage.total_tokens,
                })),
                "entityCounts": entity_counts,
                "errorCode": error_code,
                "errorSummary": error_summary.as_deref().map(|text| short_summary(text, 300)),
            });
            let usage_text = usage.to_string();
            if let Err(error) = window
                .record_sync_response(
                    response_id.as_deref(),
                    Some(&usage_text),
                    Some(&result_asset.asset_id),
                )
                .await
            {
                return Ok(internal(error).0);
            }

            if produced {
                tracing::info!(
                    event = "manual_extract_completed",
                    jobId = %job_id,
                    stageId = %stage_id,
                    batchIndex = batch_index,
                    pages = %join_pages(pages),
                    "说明书批次提取完成（结果资产与 receipt 已持久化）"
                );
                Ok(StageOutcome::Succeeded {
                    result_asset_id: Some(result_asset.asset_id),
                    usage: Some(usage),
                })
            } else {
                let code = error_code.unwrap_or_else(|| CODE_RESPONSE_FAILED.to_owned());
                let summary = error_summary.unwrap_or_else(|| "该批未产出正式知识".to_owned());
                tracing::warn!(
                    event = "manual_extract_not_produced",
                    jobId = %job_id,
                    stageId = %stage_id,
                    batchIndex = batch_index,
                    outcome = outcome.as_str(),
                    errorCode = %code,
                    "该批未产出正式知识（拒答/截断/格式错/校验失败）：等待人工复核，不自动重试"
                );
                Ok(StageOutcome::NeedsInput {
                    items: vec![MissingItem::new(
                        code,
                        format!(
                            "{summary}；该批不产生正式知识（不自动重试、不重复付费），\
                             请人工复核后对该批重新授权重算"
                        ),
                    )],
                })
            }
        })
    }
}

/// HTTP 层失败的结论（429 可退避；4xx 明确拒绝可行动；传输/5xx 结果未知）。
async fn http_failure_outcome(
    pool: &sqlx::SqlitePool,
    window: &mut crate::jobs::SubmissionWindow,
    snapshot_id: &str,
    job_id: &str,
    stage_id: &str,
    error: &ManualAiError,
    now: &Timestamp,
) -> StageOutcome {
    let detail = error.redacted();
    match error {
        ManualAiError::RateLimited { .. } => {
            if let Err(mark_error) = window.mark_failed(&detail).await {
                tracing::warn!(stageId = %stage_id, error = %mark_error, "attempt 标记失败（429 路径）");
            }
            tracing::warn!(
                event = "manual_extract_rate_limited",
                jobId = %job_id,
                stageId = %stage_id,
                "说明书提取被限速：可证明未被处理，按退避重试（预留继续保留）"
            );
            StageOutcome::Retryable {
                reason: detail,
                retry_after_seconds: error.retry_after_seconds(),
            }
        }
        _ if error.is_definitively_refused() => {
            if let Err(mark_error) = window.mark_failed(&detail).await {
                tracing::warn!(stageId = %stage_id, error = %mark_error, "attempt 标记失败（明确拒绝路径）");
            }
            tracing::warn!(
                event = "manual_extract_refused",
                jobId = %job_id,
                stageId = %stage_id,
                detail = %detail,
                "供应商明确拒绝：不自动重试（需人工核对配置/输入后重新授权）"
            );
            // 明确拒绝（4xx/3xx）：该批未计费；但**不自动释放快照预留**
            // （同一预留覆盖全部批次，其它批次仍需预算背书）。
            StageOutcome::NeedsInput {
                items: vec![MissingItem::new(
                    CODE_REQUEST_REJECTED,
                    format!(
                        "{detail}；该批未产生计费，未产出正式知识（不自动重试、不重复付费），\
                         请核对 providers.manual_ai 配置与输入后重新授权"
                    ),
                )],
            }
        }
        _ => {
            if let Err(mark_error) = window.mark_unknown(&detail).await {
                tracing::warn!(stageId = %stage_id, error = %mark_error, "attempt 标记失败（未知路径）");
            }
            let attempt_id = window.attempt_id().map(str::to_owned);
            let ledger_note =
                mark_manual_reservation_unknown(pool, snapshot_id, attempt_id.as_deref(), now)
                    .await;
            tracing::error!(
                event = "manual_extract_submission_unknown",
                jobId = %job_id,
                stageId = %stage_id,
                detail = %detail,
                ledger = %ledger_note,
                "已发出请求但没有持久化完整响应：该批 submission_unknown（不重发，等待对账）"
            );
            StageOutcome::SubmissionUnknown { reason: detail }
        }
    }
}

/// 结果资产写入失败 → `needs_input`（完整性/磁盘问题，不重复付费）。
fn derived_asset_failure(error: DerivedAssetError) -> HandlerStop {
    HandlerStop(StageOutcome::NeedsInput {
        items: vec![MissingItem::new(
            error.code,
            format!(
                "批次结果资产写入失败：{}；该批不产生正式知识，请核对 data-dir 完整性后重新授权",
                error.detail
            ),
        )],
    })
}

/// 结果未知：保留预留（`actual` 保持 NULL），并把 attempt 关联到条目。
async fn mark_manual_reservation_unknown(
    pool: &sqlx::SqlitePool,
    snapshot_id: &str,
    attempt_id: Option<&str>,
    now: &Timestamp,
) -> String {
    match manual_ledger_entry(pool, snapshot_id).await {
        Ok(Some(entry)) => {
            let mut conn = match pool.acquire().await {
                Ok(conn) => conn,
                Err(error) => return format!("账本联动失败（获取连接）：{error}"),
            };
            match generation_ledger::mark_submission_unknown(&mut conn, &entry.id, attempt_id, *now)
                .await
            {
                Ok(outcome) => format!("预留保留为 unknown（{outcome:?}）"),
                Err(error) => format!("账本联动失败（保留预留不变）：{error}"),
            }
        }
        Ok(None) => "该快照没有说明书 AI 预留条目（不创建）".to_owned(),
        Err(error) => format!("账本读取失败（保留预留不变）：{error}"),
    }
}

async fn manual_ledger_entry(
    pool: &sqlx::SqlitePool,
    snapshot_id: &str,
) -> Result<Option<CostLedgerEntry>, crate::storage::StorageError> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(crate::storage::StorageError::from)?;
    let entries = repo::ledger::list_for_snapshot(&mut conn, snapshot_id).await?;
    Ok(entries
        .into_iter()
        .find(|entry| entry.provider == ProviderKey::ManualAi))
}

// ---------------------------------------------------------------------------
// manual_merge
// ---------------------------------------------------------------------------

/// `manual_merge`：本地确定性合并（去重保留出处；冲突保留待复核）。
pub struct ManualMergeHandler {
    data_dir: PathBuf,
}

impl ManualMergeHandler {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// 读取全部批次的持久化结果（缺一不可；按 batch_index 升序）。
    async fn load_batches(&self, ctx: &StageContext) -> Loaded<Vec<BatchExtractionResult>> {
        let mut conn = ctx.pool.acquire().await.map_err(internal)?;
        let stages = repo::job_stages::list_for_job(&mut conn, &ctx.job.id)
            .await
            .map_err(internal)?;
        drop(conn);
        let mut batch_stages: Vec<_> = stages
            .iter()
            .filter(|stage| stage.stage_kind == StageKind::ManualExtract)
            .collect();
        batch_stages.sort_by_key(|stage| stage.batch_index);
        if batch_stages.is_empty() {
            return Err(needs_input(
                CODE_MERGE_INPUT_MISSING,
                "找不到任何 manual_extract 批次：不产出合并结果（请检查任务阶段）",
            ));
        }
        let mut results = Vec::with_capacity(batch_stages.len());
        for stage in batch_stages {
            if stage.status != JobStatus::Succeeded {
                return Err(needs_input(
                    CODE_MERGE_INPUT_MISSING,
                    format!(
                        "批次 {} 状态为 {}（不是 succeeded）：全部批次成功才允许合并（不在 AI 层重试）",
                        stage.batch_index,
                        stage.status.as_str()
                    ),
                ));
            }
            let asset_id = stage.result_asset_id.clone().ok_or_else(|| {
                needs_input(
                    CODE_MERGE_INPUT_MISSING,
                    format!(
                        "批次 {} 缺少结果资产：无法合并（完整性异常，不重新请求）",
                        stage.batch_index
                    ),
                )
            })?;
            let bytes = store::read_asset_bytes(&ctx.pool, &self.data_dir, &asset_id)
                .await
                .map_err(|error| {
                    needs_input(
                        CODE_MERGE_INPUT_MISSING,
                        format!(
                            "批次 {} 的结果资产不可读：{}（完整性异常）",
                            stage.batch_index, error.detail
                        ),
                    )
                })?;
            let parsed: BatchExtractionResult =
                serde_json::from_slice(&bytes).map_err(|error| {
                    needs_input(
                        CODE_MERGE_INPUT_MISSING,
                        format!(
                            "批次 {} 的结果资产不是合法 JSON：{error}",
                            stage.batch_index
                        ),
                    )
                })?;
            results.push(parsed);
        }
        Ok(results)
    }
}

impl StageHandler for ManualMergeHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            // 0) 本地副本优先：已有结果资产（恢复/重跑）→ 复用，不重复派生。
            if let Some(asset_id) = ctx.stage.result_asset_id.clone()
                && let Some(existing) =
                    store::existing_result(&ctx.pool, &self.data_dir, &asset_id).await
            {
                return Ok(StageOutcome::Succeeded {
                    result_asset_id: Some(existing.asset_id),
                    usage: None,
                });
            }

            // 1) 批次结果（全部 succeeded 且结果资产可读）。
            let batches = match self.load_batches(ctx).await {
                Ok(batches) => batches,
                Err(stop) => return Ok(stop.0),
            };

            // 2) 计划页（准备记录）与 prompt 版本（快照）。
            let snapshot_and_plan = async {
                let mut conn = ctx.pool.acquire().await.map_err(internal)?;
                let snapshot = repo::snapshots::get(&mut conn, &ctx.job.snapshot_id)
                    .await
                    .map_err(internal)?
                    .ok_or_else(|| {
                        needs_input(
                            CODE_SNAPSHOT_MISSING,
                            "任务快照不存在：无法确认冻结输入（不猜测）",
                        )
                    })?;
                let page_inputs =
                    repo::preparations::page_quote_inputs(&mut conn, &snapshot.preparation_id)
                        .await
                        .map_err(internal)?;
                drop(conn);
                let plan = plan_manual_ai(&page_inputs).map_err(|error| {
                    needs_input(
                        CODE_MERGE_INPUT_MISSING,
                        format!("无法从冻结准备记录计算计划页集合：{error}"),
                    )
                })?;
                Ok::<_, HandlerStop>((snapshot, plan))
            }
            .await;
            let (snapshot, plan) = match snapshot_and_plan {
                Ok(values) => values,
                Err(stop) => return Ok(stop.0),
            };

            // 3) 本地确定性合并（覆盖完整性 + 去重保留出处 + 冲突保留）。
            let merged: MergedKnowledge =
                match merge_batches(&batches, &plan.pages_overall(), &snapshot.prompt_version) {
                    Ok(merged) => merged,
                    Err(error) => {
                        tracing::warn!(
                            event = "manual_merge_blocked",
                            jobId = %ctx.job.id,
                            stageId = %ctx.stage.id,
                            errorCode = error.code,
                            detail = %error.detail,
                            "合并前置校验未通过：不产出合并结果（等待人工处理）"
                        );
                        return Ok(StageOutcome::NeedsInput {
                            items: vec![MissingItem::new(
                                error.code,
                                format!(
                                    "{}；合并未产出结果（不调用 AI、不自动重试）",
                                    error.detail
                                ),
                            )],
                        });
                    }
                };

            let usage = json!({
                "schemaVersion": merged.schema_version,
                "promptVersion": merged.prompt_version,
                "pageFrom": merged.page_from,
                "pageTo": merged.page_to,
                "coverage": merged.coverage,
                "partCount": merged.parts.len(),
                "stepCount": merged.steps.len(),
                "specCount": merged.specs.len(),
                "uncertaintyCount": merged.uncertainties.len(),
                "conflictCount": merged.conflicts.len(),
            });
            let bytes = serde_json::to_vec(&merged).unwrap_or_default();
            let asset = match store::persist_derived_asset(
                &ctx.pool,
                &self.data_dir,
                &ctx.job.item_id,
                &bytes,
                "manual_merged_knowledge.json",
            )
            .await
            {
                Ok(asset) => asset,
                Err(error) => return Ok(derived_asset_failure(error).0),
            };
            // 结果事实先落库（不可变；checkpoint 由执行器带租约推进）。
            if let Err(error) = record_result_fact(
                &ctx.pool,
                &ctx.stage.id,
                Some(&asset.asset_id),
                Some(&usage.to_string()),
                ctx.now,
            )
            .await
            {
                return Ok(internal(error).0);
            }
            tracing::info!(
                event = "manual_merge_completed",
                jobId = %ctx.job.id,
                stageId = %ctx.stage.id,
                batches = merged.coverage.batches.len(),
                pages = merged.coverage.page_count,
                conflicts = merged.conflicts.len(),
                "说明书知识合并完成（本地确定性合并；冲突保留为待复核）"
            );
            Ok(StageOutcome::Succeeded {
                result_asset_id: Some(asset.asset_id),
                usage: Some(usage),
            })
        })
    }
}

// ---------------------------------------------------------------------------
// 诊断
// ---------------------------------------------------------------------------

/// 阶段端点摘要（诊断/测试；不含用户数据）。
pub fn stage_endpoint_summary() -> String {
    MANUAL_AI_RESPONSES_PATH.to_owned()
}
