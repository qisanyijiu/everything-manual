//! 生成输入指纹与报价计划（contracts.md §4；T11）。
//!
//! 本模块把"报价算什么、快照冻结什么"变成纯函数与纯数据：
//! - [`GenerationFingerprint`] + [`compute_input_hash`]：报价与任务快照绑定的**输入指纹**
//!   （照片按多视图槽位顺序的 `photoId + blob sha256`、说明书原件哈希与页数、
//!   物品 revision 与型号、模型参数、prompt 版本、价格版本）；
//! - [`plan_manual_ai`]：把 ready preparation 的页输入（页文字字节数 / 是否发送页图）
//!   展开为 ≤5 页/批的批次计划与 **token 保守上界**（图像 token 用文档支持的保守假设）；
//! - [`tripo_amount`] / [`manual_ai_amount`]：按价格目录的精确十进制单价换算出
//!   **分列**金额（Tripo credits 与 Manual AI USD 不相加）与保守上界；全部整数运算
//!   （[`crate::cost`]），不允许浮点。
//!
//! 本模块不读取数据库、不做 IO、不知道 HTTP：持久化在
//! `crates/server/src/generation/**`（报价落库、冻结快照、幂等建单）。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cost::{MoneyError, add_minor, mul_div_ceil};
use crate::domain::Currency;

// ---------------------------------------------------------------------------
// 常量（合同默认值；改动需要需求确认）
// ---------------------------------------------------------------------------

/// 报价有效期（秒）：默认 10 分钟（PRD 假设 A-02、contracts.md §4）。
pub const QUOTE_TTL_SECONDS: u64 = 600;
/// 说明书 AI 每批最多页数（架构 §5.2：按最多 5 页/批处理）。
pub const MANUAL_PAGES_PER_BATCH: i64 = 5;
/// 每批 `max_output_tokens` 上限（报价按此保守预留；T14 的实际请求不得超过快照值）。
pub const MANUAL_AI_MAX_OUTPUT_TOKENS_PER_BATCH: i64 = 4096;
/// 每批提示词/schema 固定开销的**保守上界**（输入 token）。
pub const MANUAL_AI_PROMPT_OVERHEAD_TOKENS_UPPER: i64 = 1200;
/// 每页文字的输入 token 保守上界：1 token/字节（UTF-8 最坏情形，宁可高估）。
pub const MANUAL_AI_TOKENS_PER_TEXT_BYTE_UPPER: i64 = 1;
/// 每张页图的输入 token 保守上界：3000（文档支持的保守假设；页图长边 ≤2000px）。
pub const MANUAL_AI_TOKENS_PER_PAGE_IMAGE_UPPER: i64 = 3000;
/// 预计口径：每张页图约 1200 输入 token（只用于"预计"，预算一律用上界）。
pub const MANUAL_AI_TOKENS_PER_PAGE_IMAGE_EXPECTED: i64 = 1200;
/// 预计口径：文本约 3 字节/token（只用于"预计"）。
pub const MANUAL_AI_TEXT_BYTES_PER_TOKEN_EXPECTED: i64 = 3;
/// 预计口径：每页约 800 输出 token（只用于"预计"）。
pub const MANUAL_AI_EXPECTED_OUTPUT_TOKENS_PER_PAGE: i64 = 800;
/// 预计口径：每批固定开销（只用于"预计"）。
pub const MANUAL_AI_PROMPT_OVERHEAD_TOKENS_EXPECTED: i64 = 300;
/// 说明书提取提示词版本（快照冻结；T14 只能在其声明支持的版本上执行）。
pub const MANUAL_EXTRACT_PROMPT_VERSION: &str = "manual_extract_v1";

/// 生成任务的输入指纹（报价与 `generation_snapshots` 共同绑定）。
///
/// 字段取舍（RD 取舍，见 implementation.md §T11）：
/// - 照片按 **多视图槽位顺序** 记录 `photoId + sha256`：photos 行可变（PATCH 换资产），
///   只存 id 会漏掉"同 id 不同内容"（T07 QA 前置约束）；
/// - `item_revision` 与 `item_name`/`item_model` 一起入指纹：报价确认页展示物品身份，
///   物品任何编辑都要重新报价确认（"重生成总是新快照 + 新预算确认"）；
/// - `provider_config` 只含非密钥参数（模型名、质量、face_limit 等），不含 API key。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationFingerprint {
    pub item_id: String,
    pub item_revision: i64,
    pub item_name: String,
    pub item_model: String,
    pub preparation_id: String,
    pub preparation_source_sha256: String,
    pub page_count: i64,
    /// 多视图照片（槽位顺序 front→left→back→right；detail 不进入）。
    pub photos: Vec<PhotoFingerprint>,
    pub model_preset: String,
    pub provider_config: serde_json::Value,
    pub prompt_version: String,
    pub price_version: String,
}

/// 单张进入多视图请求的照片指纹。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoFingerprint {
    pub photo_id: String,
    pub view: String,
    /// 照片内容（blob）的 sha256 —— 与 photoId 一起冻结，防止换资产后复用旧报价。
    pub sha256: String,
}

/// Tripo 多视图生成的模型参数（架构 §5.3 默认值；随报价/快照冻结）。
///
/// 不含任何凭据；T12 构造请求体时只使用这些字段（不得静默改质量或换模型）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TripoParameters {
    /// 供应商模型名（如 `v3.1-20260211`）。
    pub model: String,
    pub texture: bool,
    pub pbr: bool,
    pub texture_quality: String,
    pub geometry_quality: String,
    pub face_limit: i64,
    /// 是否输出四边形网格（首版关闭）。
    pub quad: bool,
    /// 是否生成分件（首版关闭；外观模型不等于机械结构真值）。
    pub generate_parts: bool,
}

/// 输入指纹的 sha256（小写 hex）。同一逻辑输入必须得到同一指纹。
pub fn compute_input_hash(fingerprint: &GenerationFingerprint) -> String {
    let json = serde_json::to_string(fingerprint).expect("指纹结构总是可序列化");
    sha256_hex(json.as_bytes())
}

/// sha256 小写 hex（快照/幂等 body_hash/阶段 input_hash 共用）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// 说明书 AI 分批与 token 上界
// ---------------------------------------------------------------------------

/// 单页的报价输入（报价阶段从 pages + blobs 汇总；`text_bytes = None` 表示扫描页）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageInput {
    /// 1-based 页号（contracts.md §1）。
    pub page_number: i64,
    /// 页文字资产的内容字节数；`None` = 无文字层（发送页图）。
    pub text_bytes: Option<i64>,
}

/// 计划错误（不能算出可靠上界的输入不进入报价）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// 没有可用页（ready preparation 至少 1 页）。
    NoPages,
    /// 页数超过上限（PRD §5.3：≤100 页）或页号不连续。
    InvalidPages { detail: String },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPages => write!(f, "没有可用的页"),
            Self::InvalidPages { detail } => write!(f, "页集合非法：{detail}"),
        }
    }
}

impl std::error::Error for PlanError {}

/// 说明书 AI 的批次计划与 token 上界。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualAiPlan {
    pub page_from: i64,
    pub page_to: i64,
    pub page_count: i64,
    /// 每批的页号（1-based，升序；每批 ≤ [`MANUAL_PAGES_PER_BATCH`] 页）。
    pub batches: Vec<Vec<i64>>,
    /// 发送页文字的页（有文字层）。
    pub text_pages: Vec<i64>,
    /// 发送页图的页（无文字层；每页图计入保守 token 上界）。
    pub image_pages: Vec<i64>,
    /// 输入 token 的保守上界。
    pub input_tokens_upper_bound: i64,
    /// 输出 token 的保守上界（批次数 × 每批上限）。
    pub output_tokens_upper_bound: i64,
    /// 预计输入 token（展示"预计"口径，预算不使用）。
    pub input_tokens_expected: i64,
    /// 预计输出 token（同上）。
    pub output_tokens_expected: i64,
}

impl ManualAiPlan {
    /// 全部计划页（1-based 升序）：文字页与页图页的并集，每个页恰好一次。
    ///
    /// 用于批次处理器与 `manual_merge` 的**覆盖校验**（漏批检测）：
    /// 计划页集合 = 用户已确认的发送范围 = 合并必须覆盖的页集合。
    pub fn pages_overall(&self) -> Vec<i64> {
        let mut pages: Vec<i64> = self
            .text_pages
            .iter()
            .chain(self.image_pages.iter())
            .copied()
            .collect();
        pages.sort_unstable();
        pages
    }
}

/// 把 ready preparation 的页输入展开为批次计划与保守 token 上界。
///
/// 页必须连续 1..=N（ready preparation 的保证）；N 必须 ≥1 且 ≤100
/// （PRD §5.3 的页数上限；超出说明输入不可用于报价）。
pub fn plan_manual_ai(pages: &[PageInput]) -> Result<ManualAiPlan, PlanError> {
    if pages.is_empty() {
        return Err(PlanError::NoPages);
    }
    let page_count = pages.len() as i64;
    if page_count > 100 {
        return Err(PlanError::InvalidPages {
            detail: format!("页数 {page_count} 超过上限 100"),
        });
    }
    for (index, page) in pages.iter().enumerate() {
        let expected = index as i64 + 1;
        if page.page_number != expected {
            return Err(PlanError::InvalidPages {
                detail: format!(
                    "页号必须从 1 连续（期望 {expected}，实际 {}）",
                    page.page_number
                ),
            });
        }
    }

    let mut batches: Vec<Vec<i64>> = Vec::new();
    let mut text_pages: Vec<i64> = Vec::new();
    let mut image_pages: Vec<i64> = Vec::new();
    let mut input_upper: i64 = 0;
    let mut input_expected: i64 = 0;

    for chunk in pages.chunks(MANUAL_PAGES_PER_BATCH as usize) {
        let mut batch_pages: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut batch_upper = MANUAL_AI_PROMPT_OVERHEAD_TOKENS_UPPER;
        let mut batch_expected = MANUAL_AI_PROMPT_OVERHEAD_TOKENS_EXPECTED;
        for page in chunk {
            batch_pages.push(page.page_number);
            match page.text_bytes {
                Some(bytes) if bytes > 0 => {
                    text_pages.push(page.page_number);
                    batch_upper = batch_upper
                        .saturating_add(bytes.saturating_mul(MANUAL_AI_TOKENS_PER_TEXT_BYTE_UPPER));
                    // 向上取整（保守）；（`i64::div_ceil` 在当前工具链是 unstable）。
                    let per_token = MANUAL_AI_TEXT_BYTES_PER_TOKEN_EXPECTED;
                    batch_expected =
                        batch_expected.saturating_add((bytes + per_token - 1) / per_token);
                }
                // 扫描页（无文字层）或空文字：发送页图，按图像 token 上界计。
                _ => {
                    image_pages.push(page.page_number);
                    batch_upper = batch_upper.saturating_add(MANUAL_AI_TOKENS_PER_PAGE_IMAGE_UPPER);
                    batch_expected =
                        batch_expected.saturating_add(MANUAL_AI_TOKENS_PER_PAGE_IMAGE_EXPECTED);
                }
            }
        }
        input_upper = input_upper.saturating_add(batch_upper);
        input_expected = input_expected.saturating_add(batch_expected);
        batches.push(batch_pages);
    }

    let batch_count = batches.len() as i64;
    Ok(ManualAiPlan {
        page_from: 1,
        page_to: page_count,
        page_count,
        batches,
        text_pages,
        image_pages,
        input_tokens_upper_bound: input_upper,
        output_tokens_upper_bound: batch_count
            .saturating_mul(MANUAL_AI_MAX_OUTPUT_TOKENS_PER_BATCH),
        input_tokens_expected: input_expected,
        output_tokens_expected: page_count
            .saturating_mul(MANUAL_AI_EXPECTED_OUTPUT_TOKENS_PER_PAGE),
    })
}

// ---------------------------------------------------------------------------
// 价格换算（精确十进制 → 最小单位）
// ---------------------------------------------------------------------------

/// Tripo 预设计价（价格目录条目；`credits_decimal` 为供应商十进制字面量）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TripoPricing {
    pub preset: String,
    pub model: String,
    pub credits_decimal: String,
    /// 每次生成的 creditMinor（1/100 credit，精确换算）。
    pub credit_minor: i64,
}

/// 说明书 AI 计价（价格目录条目；USD 十进制字面量）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualAiPricing {
    pub model: String,
    /// 每 100 万输入 token 的 usdMicros。
    pub input_usd_micros_per_million_tokens: i64,
    /// 每 100 万输出 token 的 usdMicros。
    pub output_usd_micros_per_million_tokens: i64,
    /// 每张页图的 usdMicros。
    pub image_usd_micros_per_image: i64,
    /// 原始十进制字面量（仅展示；换算以整数单价为准）。
    pub input_price_decimal: String,
    pub output_price_decimal: String,
    pub image_price_decimal: String,
}

/// 金额分项（`amount_minor` 之和 == 该口径金额）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmountLine {
    pub code: String,
    pub description: String,
    /// 用量（生成次数 / token 数 / 图片张数）。
    pub quantity: i64,
    /// 用量单位（`generation` / `tokens` / `images`）。
    pub unit: String,
    /// 单价十进制字面量（仅展示，来自价格目录）。
    pub unit_price_decimal: String,
    /// 单价分母（每 N 单位多少 `unit_price_decimal`；如每 1_000_000 token）。
    pub unit_price_per_units: i64,
    pub amount_minor: i64,
}

/// 单个供应商的报价金额（分列；不相加）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAmount {
    pub currency: Currency,
    /// 预计金额（展示口径）。
    pub estimated_minor: i64,
    /// 保守上界（预算与预留口径；允许上限必须覆盖它）。
    pub upper_bound_minor: i64,
    /// 上界分项（`amount_minor` 之和 == `upper_bound_minor`）。
    pub upper_bound_lines: Vec<AmountLine>,
}

/// 两家供应商的分列金额。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteAmounts {
    pub tripo: ProviderAmount,
    pub manual_ai: ProviderAmount,
}

/// Tripo 固定价（预设定价，不随视图数变化）。
pub fn tripo_amount(pricing: &TripoPricing) -> Result<ProviderAmount, MoneyError> {
    let line = AmountLine {
        code: "multiviewGeneration".to_owned(),
        description: "多视图生成（标准质量，含纹理与 PBR）".to_owned(),
        quantity: 1,
        unit: "generation".to_owned(),
        unit_price_decimal: pricing.credits_decimal.clone(),
        unit_price_per_units: 1,
        amount_minor: pricing.credit_minor,
    };
    Ok(ProviderAmount {
        currency: Currency::CreditMinor,
        estimated_minor: pricing.credit_minor,
        upper_bound_minor: pricing.credit_minor,
        upper_bound_lines: vec![line],
    })
}

/// 说明书 AI 金额（输入 token + 输出 token + 页图；全部按单价精确换算后相加）。
///
/// 预算口径使用 [`ManualAiPlan`] 的**保守上界**（输入/输出 token 上界与全部页图），
/// 预计口径使用预计 token（预算不使用）。
pub fn manual_ai_amount(
    plan: &ManualAiPlan,
    pricing: &ManualAiPricing,
) -> Result<ProviderAmount, MoneyError> {
    const PER_MILLION: i64 = 1_000_000;

    let input_upper = mul_div_ceil(
        plan.input_tokens_upper_bound,
        pricing.input_usd_micros_per_million_tokens,
        PER_MILLION,
    )?;
    let output_upper = mul_div_ceil(
        plan.output_tokens_upper_bound,
        pricing.output_usd_micros_per_million_tokens,
        PER_MILLION,
    )?;
    let images_upper = mul_div_ceil(
        plan.image_pages.len() as i64,
        pricing.image_usd_micros_per_image,
        1,
    )?;
    let upper_bound = add_minor(add_minor(input_upper, output_upper)?, images_upper)?;

    let input_expected = mul_div_ceil(
        plan.input_tokens_expected,
        pricing.input_usd_micros_per_million_tokens,
        PER_MILLION,
    )?;
    let output_expected = mul_div_ceil(
        plan.output_tokens_expected,
        pricing.output_usd_micros_per_million_tokens,
        PER_MILLION,
    )?;
    let images_expected = images_upper;
    let estimated = add_minor(add_minor(input_expected, output_expected)?, images_expected)?;

    let lines = vec![
        AmountLine {
            code: "inputTokens".to_owned(),
            description: "输入 token（页文字/页图 + 提示词开销的保守上界）".to_owned(),
            quantity: plan.input_tokens_upper_bound,
            unit: "tokens".to_owned(),
            unit_price_decimal: pricing.input_price_decimal.clone(),
            unit_price_per_units: PER_MILLION,
            amount_minor: input_upper,
        },
        AmountLine {
            code: "outputTokens".to_owned(),
            description: "输出 token（批次数 × 每批上限）".to_owned(),
            quantity: plan.output_tokens_upper_bound,
            unit: "tokens".to_owned(),
            unit_price_decimal: pricing.output_price_decimal.clone(),
            unit_price_per_units: PER_MILLION,
            amount_minor: output_upper,
        },
        AmountLine {
            code: "pageImages".to_owned(),
            description: "页图（扫描页/无文字层页发送视觉输入）".to_owned(),
            quantity: plan.image_pages.len() as i64,
            unit: "images".to_owned(),
            unit_price_decimal: pricing.image_price_decimal.clone(),
            unit_price_per_units: 1,
            amount_minor: images_upper,
        },
    ];

    Ok(ProviderAmount {
        currency: Currency::UsdMicros,
        estimated_minor: estimated,
        upper_bound_minor: upper_bound,
        upper_bound_lines: lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::{CREDIT_MINOR_SCALE, Rounding, USD_MICROS_SCALE, parse_decimal_scaled};

    fn page(number: i64, text_bytes: Option<i64>) -> PageInput {
        PageInput {
            page_number: number,
            text_bytes,
        }
    }

    fn text_pricing() -> ManualAiPricing {
        ManualAiPricing {
            model: "gpt-test".to_owned(),
            input_usd_micros_per_million_tokens: 250_000, // 0.25 USD / 1M
            output_usd_micros_per_million_tokens: 2_000_000, // 2.00 USD / 1M
            image_usd_micros_per_image: 10_000,           // 0.01 USD / 张
            input_price_decimal: "0.25".to_owned(),
            output_price_decimal: "2.00".to_owned(),
            image_price_decimal: "0.01".to_owned(),
        }
    }

    #[test]
    fn fingerprint_hash_is_stable_and_content_sensitive() {
        let fingerprint = GenerationFingerprint {
            item_id: "item-1".to_owned(),
            item_revision: 3,
            item_name: "相机".to_owned(),
            item_model: "X100V".to_owned(),
            preparation_id: "prep-1".to_owned(),
            preparation_source_sha256: "a".repeat(64),
            page_count: 4,
            photos: vec![
                PhotoFingerprint {
                    photo_id: "photo-front".to_owned(),
                    view: "front".to_owned(),
                    sha256: "b".repeat(64),
                },
                PhotoFingerprint {
                    photo_id: "photo-left".to_owned(),
                    view: "left".to_owned(),
                    sha256: "c".repeat(64),
                },
            ],
            model_preset: "tripo-h-v3.1-standard".to_owned(),
            provider_config: serde_json::json!({ "tripo": { "model": "v3.1-20260211" } }),
            prompt_version: MANUAL_EXTRACT_PROMPT_VERSION.to_owned(),
            price_version: "2026-09-11".to_owned(),
        };
        let hash = compute_input_hash(&fingerprint);
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, compute_input_hash(&fingerprint), "同一输入必须同哈希");

        // 照片内容变化（换资产，photoId 不变）必须改变指纹。
        let mut changed = fingerprint.clone();
        changed.photos[0].sha256 = "d".repeat(64);
        assert_ne!(hash, compute_input_hash(&changed));

        // 物品 revision 变化（编辑物品）必须改变指纹。
        let mut edited = fingerprint.clone();
        edited.item_revision = 4;
        assert_ne!(hash, compute_input_hash(&edited));

        // 价格版本变化必须改变指纹。
        let mut repriced = fingerprint.clone();
        repriced.price_version = "2026-10-01".to_owned();
        assert_ne!(hash, compute_input_hash(&repriced));
    }

    #[test]
    fn manual_plan_batches_five_pages_and_bounds_tokens() {
        // 7 页：2 批；扫描页（无文字）走页图上界。
        let pages = vec![
            page(1, Some(3000)),
            page(2, Some(0)),
            page(3, None),
            page(4, Some(120)),
            page(5, Some(10)),
            page(6, Some(5)),
            page(7, None),
        ];
        let plan = plan_manual_ai(&pages).unwrap();
        assert_eq!(plan.batches.len(), 2);
        assert_eq!(plan.batches[0], vec![1, 2, 3, 4, 5]);
        assert_eq!(plan.batches[1], vec![6, 7]);
        assert_eq!(plan.text_pages, vec![1, 4, 5, 6]);
        // 空文字与无文字层都发送页图（保守）。
        assert_eq!(plan.image_pages, vec![2, 3, 7]);
        assert_eq!(plan.page_from, 1);
        assert_eq!(plan.page_to, 7);
        // 上界 = 2 批开销 + 文本字节 + 3 张图 × 3000。
        let expected_upper = 2 * MANUAL_AI_PROMPT_OVERHEAD_TOKENS_UPPER
            + (3000 + 120 + 10 + 5)
            + 3 * MANUAL_AI_TOKENS_PER_PAGE_IMAGE_UPPER;
        assert_eq!(plan.input_tokens_upper_bound, expected_upper);
        assert_eq!(plan.output_tokens_upper_bound, 2 * 4096);
        assert!(plan.input_tokens_expected < plan.input_tokens_upper_bound);
        assert!(plan.output_tokens_expected < plan.output_tokens_upper_bound);
    }

    #[test]
    fn manual_plan_rejects_empty_or_discontinuous_pages() {
        assert_eq!(plan_manual_ai(&[]), Err(PlanError::NoPages));
        let discontinuous = vec![page(1, Some(1)), page(3, Some(1))];
        assert!(matches!(
            plan_manual_ai(&discontinuous),
            Err(PlanError::InvalidPages { .. })
        ));
        let too_many: Vec<PageInput> = (1..=101).map(|n| page(n, Some(1))).collect();
        assert!(matches!(
            plan_manual_ai(&too_many),
            Err(PlanError::InvalidPages { .. })
        ));
    }

    #[test]
    fn amounts_are_exact_and_lines_sum_to_upper_bound() {
        let plan = plan_manual_ai(&[page(1, Some(3000)), page(2, None)]).unwrap();
        let pricing = text_pricing();
        let amount = manual_ai_amount(&plan, &pricing).unwrap();
        assert_eq!(amount.currency, Currency::UsdMicros);

        // 预算口径 = 上界分项之和（整数运算，无浮点）。
        let sum: i64 = amount
            .upper_bound_lines
            .iter()
            .map(|line| line.amount_minor)
            .sum();
        assert_eq!(sum, amount.upper_bound_minor);
        assert!(amount.estimated_minor <= amount.upper_bound_minor);
        // 1 批、1 张图、输入上界 = 1200 + 3000 + 3000（文本按 1 token/字节）。
        let input_line = &amount.upper_bound_lines[0];
        assert_eq!(input_line.quantity, 1200 + 3000 + 3000);
        assert_eq!(
            input_line.amount_minor,
            mul_div_ceil(input_line.quantity, 250_000, 1_000_000).unwrap()
        );
        let output_line = &amount.upper_bound_lines[1];
        assert_eq!(output_line.quantity, 4096);
        assert_eq!(output_line.amount_minor, 8192);
        let image_line = &amount.upper_bound_lines[2];
        assert_eq!(image_line.quantity, 1);
        assert_eq!(image_line.amount_minor, 10_000);
        assert_eq!(
            amount.upper_bound_minor,
            input_line.amount_minor + output_line.amount_minor + image_line.amount_minor
        );

        // Tripo：固定价（30 credits = 3000 creditMinor），预计 == 上界。
        let tripo = tripo_amount(&TripoPricing {
            preset: "tripo-h-v3.1-standard".to_owned(),
            model: "v3.1-20260211".to_owned(),
            credits_decimal: "30".to_owned(),
            credit_minor: parse_decimal_scaled("30", CREDIT_MINOR_SCALE, Rounding::Ceil).unwrap(),
        })
        .unwrap();
        assert_eq!(tripo.currency, Currency::CreditMinor);
        assert_eq!(tripo.upper_bound_minor, 3000);
        assert_eq!(tripo.estimated_minor, 3000);
        assert_eq!(tripo.upper_bound_lines[0].amount_minor, 3000);
    }

    /// 舍入边界：单价不能被 1e6 整除时，上界必须向上取整（绝不低估）。
    #[test]
    fn amount_rounding_boundaries_never_underestimate() {
        let plan = plan_manual_ai(&[page(1, Some(1))]).unwrap();
        let pricing = ManualAiPricing {
            model: "m".to_owned(),
            // 0.000001 USD / 1M tokens = 1 micro / 1M tokens → 1 token 也要 1 micro（Ceil）。
            input_usd_micros_per_million_tokens: 1,
            output_usd_micros_per_million_tokens: 0,
            image_usd_micros_per_image: 0,
            input_price_decimal: "0.000001".to_owned(),
            output_price_decimal: "0".to_owned(),
            image_price_decimal: "0".to_owned(),
        };
        let amount = manual_ai_amount(&plan, &pricing).unwrap();
        // 输入 token 上界 = 1200 + 1（1 字节文本）。
        assert_eq!(amount.upper_bound_lines[0].amount_minor, 1);
        assert_eq!(
            parse_decimal_scaled("0.005", USD_MICROS_SCALE, Rounding::Ceil).unwrap(),
            5000,
            "价格目录单价换算示例：0.005 USD = 5000 usdMicros"
        );
    }
}
