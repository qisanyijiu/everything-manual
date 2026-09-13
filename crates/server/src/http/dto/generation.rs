//! 报价、确认与任务创建 DTO（T11 / REQ-020、REQ-021、REQ-022、REQ-023）。
//!
//! 机器合同约定（contracts.md §1/§4）：
//! - 金额**分列**：Tripo `creditMinor` 与 Manual AI `usdMicros` 各自带单位，不相加；
//!   同时给出 `*Display` 字符串（credits 两位小数 / USD 最多六位小数），但**整数最小单位
//!   才是权威值**（前端不得用 display 字符串做预算判断）；
//! - 报价载荷（`QuoteDto`）是"确认页所需数据 + 服务端冻结依据"的同一形状：
//!   落库 `quotes.quote_json` 与响应逐字节同源，回读时不信任前端；
//! - 每个金额块都带 `estimatedMinor`（预计）与 `upperBoundMinor`（保守上界）；
//!   允许上限必须覆盖上界（服务端校验），否则提交任务 422；
//! - 文案语义（REQ-023）：预算保证"本应用不主动发起超出授权估算的请求"，
//!   **不冒充供应商账户级硬封顶**——该说明随 `budgetNotice` 字段返回给界面。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use manual_core::cost::{currency_wire_name, format_credits, format_usd_micros};
use manual_core::domain::{Currency, LedgerState, ProviderKey};
use manual_core::generation::{AmountLine, ManualAiPlan, ProviderAmount, TripoParameters};
use manual_core::timestamps::Timestamp;
use manual_core::validation::FieldIssue;

/// 预算语义的固定说明（REQ-023 要求界面必须如实表述，不冒充供应商硬封顶）。
pub const BUDGET_NOTICE: &str = "预算上限只表示本应用不会主动发起超出本次授权估算的请求，不是供应商账户级硬封顶；\
     供应商实际计费以账单为准";

// ---------------------------------------------------------------------------
// POST /items/{id}/estimates
// ---------------------------------------------------------------------------

/// `POST /api/v1/items/{id}/estimates` 请求体（字段可选以便给出字段级 422 明细）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EstimateRequest {
    /// ready 状态的 PDF 准备记录（报价绑定它的原件哈希与页集合）。
    #[schema(nullable = true)]
    pub preparation_id: Option<String>,
    /// 进入 Tripo 多视图的照片（front + 至少一个侧视图；不含 detail）。
    #[schema(nullable = true)]
    pub photo_ids: Option<Vec<String>>,
    /// Tripo 生成预设（价格目录的 `preset`；目录里没有的预设不支持生成）。
    #[schema(nullable = true, example = "tripo-h-v3.1-standard")]
    pub model_preset: Option<String>,
}

/// 报价响应（`{ data }` 包装）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuoteResponse {
    pub data: QuoteDto,
}

/// 一条报价（服务端计算并冻结；落库与响应同源）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuoteDto {
    pub id: String,
    pub item_id: String,
    pub preparation_id: String,
    pub model_preset: String,
    /// 非密钥的供应商配置（模型名与生成参数）；确认页展示。
    pub provider_config: ProviderConfigDto,
    /// ready preparation 的页数。
    pub page_count: i64,
    pub page_range: PageRangeDto,
    /// 说明书 AI 的最大输出 token（批次数 × 每批上限；预算口径之一）。
    pub max_output_tokens: i64,
    pub price_version: String,
    pub price_snapshot_date: String,
    /// 分列金额（Tripo credits 与 Manual AI USD）。
    pub amounts: QuoteAmountsDto,
    /// 云端发送范围（确认页必须逐项展示；未确认不允许提交）。
    pub send_scope: SendScopeDto,
    /// 报价到期时间（默认 10 分钟；过期后提交任务被拒，需重新报价）。
    #[schema(value_type = String)]
    pub expires_at: Timestamp,
    /// 确认时间；未确认为 null（**不存在默认勾选**）。
    #[schema(value_type = Option<String>, nullable = true)]
    pub confirmed_at: Option<Timestamp>,
    /// 已被某份任务消费的时间；一份报价只能创建一份任务。
    #[schema(value_type = Option<String>, nullable = true)]
    pub consumed_at: Option<Timestamp>,
    /// 消费它的任务 id（UI 用于"该操作已存在一个任务"的链接）。
    #[schema(nullable = true)]
    pub consumed_job_id: Option<String>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    /// 预算语义说明（固定文案；不得改写为"供应商硬封顶"）。
    pub budget_notice: String,
}

/// `[from, to]`（1-based、闭区间；contracts.md §1 页码基准）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageRangeDto {
    pub from: i64,
    pub to: i64,
}

/// 非密钥的供应商配置快照。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfigDto {
    pub tripo: TripoParametersDto,
    pub manual_ai: ManualAiConfigDto,
}

/// Tripo 生成参数（`preset` 为价格目录里的预设名）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TripoParametersDto {
    /// 价格目录中的预设名。
    pub preset: String,
    pub model: String,
    pub texture: bool,
    pub pbr: bool,
    pub texture_quality: String,
    pub geometry_quality: String,
    pub face_limit: i64,
    pub quad: bool,
    pub generate_parts: bool,
}

impl TripoParametersDto {
    pub fn new(preset: &str, parameters: &TripoParameters) -> Self {
        Self {
            preset: preset.to_owned(),
            model: parameters.model.clone(),
            texture: parameters.texture,
            pbr: parameters.pbr,
            texture_quality: parameters.texture_quality.clone(),
            geometry_quality: parameters.geometry_quality.clone(),
            face_limit: parameters.face_limit,
            quad: parameters.quad,
            generate_parts: parameters.generate_parts,
        }
    }
}

/// 说明书 AI 的模型与提示词版本。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualAiConfigDto {
    pub model: String,
    pub prompt_version: String,
}

/// 分列金额。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuoteAmountsDto {
    pub tripo: ProviderAmountDto,
    pub manual_ai: ProviderAmountDto,
}

/// 金额币种（线上取值 `creditMinor` / `usdMicros`；contracts.md §1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum AmountCurrencyDto {
    /// Tripo credit 的 1/100。
    CreditMinor,
    /// USD 的 1/1,000,000。
    UsdMicros,
}

impl AmountCurrencyDto {
    pub fn from_currency(currency: Currency) -> Self {
        match currency {
            Currency::CreditMinor => Self::CreditMinor,
            Currency::UsdMicros => Self::UsdMicros,
        }
    }

    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::CreditMinor => "creditMinor",
            Self::UsdMicros => "usdMicros",
        }
    }
}

/// 单个供应商的金额块（预计 + 保守上界 + 上界分项）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAmountDto {
    pub currency: AmountCurrencyDto,
    /// 权威值：整数最小单位。
    pub estimated_minor: i64,
    /// 权威值：整数最小单位（预算口径）。
    pub upper_bound_minor: i64,
    /// 可读展示（credits 两位小数 / USD 最多六位小数）；不用于计算。
    pub estimated_display: String,
    pub upper_bound_display: String,
    /// 上界分项（`amountMinor` 之和等于 `upperBoundMinor`）。
    pub upper_bound_lines: Vec<AmountLineDto>,
}

impl ProviderAmountDto {
    pub fn from_amount(amount: &ProviderAmount) -> Self {
        let display = |minor: i64| match amount.currency {
            Currency::CreditMinor => format!("{} credits", format_credits(minor)),
            Currency::UsdMicros => format!("{} USD", format_usd_micros(minor)),
        };
        Self {
            currency: AmountCurrencyDto::from_currency(amount.currency),
            estimated_minor: amount.estimated_minor,
            upper_bound_minor: amount.upper_bound_minor,
            estimated_display: display(amount.estimated_minor),
            upper_bound_display: display(amount.upper_bound_minor),
            upper_bound_lines: amount
                .upper_bound_lines
                .iter()
                .map(|line| AmountLineDto::from_line(line, amount.currency))
                .collect(),
        }
    }
}

/// 金额分项（单价保留目录里的十进制字面量，金额为整数最小单位）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AmountLineDto {
    pub code: String,
    pub description: String,
    pub quantity: i64,
    pub unit: String,
    pub unit_price_decimal: String,
    pub unit_price_per_units: i64,
    pub amount_minor: i64,
    pub amount_display: String,
}

impl AmountLineDto {
    fn from_line(line: &AmountLine, currency: Currency) -> Self {
        let amount_display = match currency {
            Currency::CreditMinor => format!("{} credits", format_credits(line.amount_minor)),
            Currency::UsdMicros => format!("{} USD", format_usd_micros(line.amount_minor)),
        };
        Self {
            code: line.code.clone(),
            description: line.description.clone(),
            quantity: line.quantity,
            unit: line.unit.clone(),
            unit_price_decimal: line.unit_price_decimal.clone(),
            unit_price_per_units: line.unit_price_per_units,
            amount_minor: line.amount_minor,
            amount_display,
        }
    }
}

/// 云端发送范围（REQ-021：确认页必须逐项列出"将发送什么给谁"）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendScopeDto {
    pub tripo: TripoSendScopeDto,
    pub manual_ai: ManualAiSendScopeDto,
    pub price_version: String,
    pub price_snapshot_date: String,
    /// 本次计划的保守上界（确认页与预算输入框的对照值）。
    pub planned_upper_bound: QuoteAmountsDto,
    pub budget_notice: String,
}

/// 发送给 Tripo 的内容。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TripoSendScopeDto {
    /// 多视图照片（槽位顺序；detail 不在其中）。
    pub views: Vec<PhotoScopeDto>,
    pub model: String,
    pub preset: String,
    /// 生成参数（质量/face_limit/quad/分件等；确认页必须展示"模型名与参数"）。
    pub parameters: TripoParametersDto,
}

/// 单张发送照片（视图 + 图片 id + 内容哈希）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PhotoScopeDto {
    pub view: String,
    pub photo_id: String,
    pub sha256: String,
}

/// 发送给说明书 AI 的内容。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualAiSendScopeDto {
    /// 随请求发送的物品身份文本（名称与型号；REQ-021 的"型号文本"）。
    pub item_name: String,
    pub item_model: String,
    pub model: String,
    pub prompt_version: String,
    pub page_from: i64,
    pub page_to: i64,
    pub page_count: i64,
    /// 发送页文字的页（1-based）。
    pub text_pages: Vec<i64>,
    /// 发送页图的页（1-based；扫描页/无文字层页）。
    pub image_pages: Vec<i64>,
    pub max_output_tokens: i64,
}

impl ManualAiSendScopeDto {
    /// 组装发送范围（物品身份文本来自报价时的物品行，用于 REQ-021 的告知）。
    pub fn from_plan(
        item: &manual_core::domain::Item,
        model: &str,
        prompt_version: &str,
        plan: &ManualAiPlan,
        max_output_tokens: i64,
    ) -> Self {
        Self {
            item_name: item.name.clone(),
            item_model: item.model.clone(),
            model: model.to_owned(),
            prompt_version: prompt_version.to_owned(),
            page_from: plan.page_from,
            page_to: plan.page_to,
            page_count: plan.page_count,
            text_pages: plan.text_pages.clone(),
            image_pages: plan.image_pages.clone(),
            max_output_tokens,
        }
    }
}

// ---------------------------------------------------------------------------
// POST /items/{id}/estimates/{quoteId}/confirm
// ---------------------------------------------------------------------------

/// 确认响应（`{ data }` 包装）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConfirmationResponse {
    pub data: ConfirmationDto,
}

/// 一次云端发送确认（写入 `audit_events`，随报价冻结）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationDto {
    pub quote_id: String,
    #[schema(value_type = String)]
    pub confirmed_at: Timestamp,
    /// 确认时展示给用户的发送范围（与报价同源；不得被后续修改）。
    pub send_scope: SendScopeDto,
    /// 本次确认对应的用户可见说明（"已确认发送范围（时间）"）。
    pub summary: String,
}

// ---------------------------------------------------------------------------
// POST /items/{id}/jobs
// ---------------------------------------------------------------------------

/// `POST /api/v1/items/{id}/jobs` 请求体。
///
/// **没有费用数值字段**：金额一律由服务端从报价快照回读（"不接受前端传入的费用数值"）。
/// `limits` 是用户本次授权的上限，必须覆盖服务端计算的保守上界。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobCreateRequest {
    /// 已确认且未过期的报价。
    #[schema(nullable = true)]
    pub quote_id: Option<String>,
    #[schema(nullable = true)]
    pub preparation_id: Option<String>,
    #[schema(nullable = true)]
    pub photo_ids: Option<Vec<String>>,
    /// 用户授权的预算上限（分列；不得小于服务端计算的保守上界）。
    #[schema(nullable = true)]
    pub limits: Option<BudgetLimitsDto>,
}

/// 用户授权的分列预算上限。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BudgetLimitsDto {
    #[schema(nullable = true)]
    pub tripo_credit_minor: Option<i64>,
    #[schema(nullable = true)]
    pub manual_ai_usd_micros: Option<i64>,
}

/// 任务创建响应（首次 202；同键同 body 重放返回同一 job）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobResponse {
    pub data: JobDto,
}

/// 任务摘要（本卡只创建；阶段执行属 T10/T12/T14/T15）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobDto {
    pub id: String,
    pub item_id: String,
    pub snapshot_id: String,
    /// 运行时状态（contracts.md §5；`queued` 起步）。
    pub status: String,
    pub revision: i64,
    /// 本次冻结的费用预留（分列；金额为服务端计算的保守上界）。
    pub reservations: Vec<ReservationDto>,
    /// 费用语义说明（与报价同一文案）。
    pub budget_notice: String,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

/// 一笔费用预留（`cost_ledger` 的对外投影）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReservationDto {
    pub provider: ProviderKeyDto,
    pub currency: AmountCurrencyDto,
    /// 预留金额（整数最小单位；unknown 时仍保留该值）。
    pub reserved_minor: i64,
    pub reserved_display: String,
    /// `reserved` / `settled` / `released` / `unknown`（unknown = 未决预留，等待对账）。
    pub state: String,
}

/// 供应商维度（线上取值 `tripo` / `manual_ai`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKeyDto {
    Tripo,
    ManualAi,
}

impl ProviderKeyDto {
    pub fn from_provider(provider: ProviderKey) -> Self {
        match provider {
            ProviderKey::Tripo => Self::Tripo,
            ProviderKey::ManualAi => Self::ManualAi,
        }
    }
}

impl ReservationDto {
    pub fn from_entry(entry: &manual_core::domain::CostLedgerEntry) -> Self {
        Self {
            provider: ProviderKeyDto::from_provider(entry.provider),
            currency: AmountCurrencyDto::from_currency(entry.currency),
            reserved_minor: entry.reserved,
            reserved_display: match entry.currency {
                Currency::CreditMinor => format!("{} credits", format_credits(entry.reserved)),
                Currency::UsdMicros => format!("{} USD", format_usd_micros(entry.reserved)),
            },
            state: entry.state.as_str().to_owned(),
        }
    }
}

impl QuoteDto {
    /// 金额块（供确认页与任务详情复用）。
    pub fn amounts_from(amounts: &manual_core::generation::QuoteAmounts) -> QuoteAmountsDto {
        QuoteAmountsDto {
            tripo: ProviderAmountDto::from_amount(&amounts.tripo),
            manual_ai: ProviderAmountDto::from_amount(&amounts.manual_ai),
        }
    }
}

/// 币种名称（日志与测试断言用；与 `AmountCurrencyDto` 的线上取值一致）。
pub fn currency_name(currency: Currency) -> &'static str {
    currency_wire_name(currency)
}

/// 账本状态名称（`unknown` 表示"未决预留（等待对账）"，不是 0）。
pub fn ledger_state_name(state: LedgerState) -> &'static str {
    state.as_str()
}

/// 字段级 422 的便捷构造（`generation` 服务层与 HTTP 层共用同一形状）。
pub fn field_issue(field: &str, message: impl Into<String>) -> FieldIssue {
    FieldIssue::new(field, message)
}
