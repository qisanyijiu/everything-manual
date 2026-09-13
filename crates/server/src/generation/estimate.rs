//! 报价服务（T11 / REQ-020、REQ-021）：校验前置条件、计算计划与分列金额、落库报价快照。
//!
//! 语义（contracts.md §4）：
//! - **只计算计划**：不调用任何生成服务、不写 `cost_ledger`、不创建 job；
//! - **服务端重算**：金额只来自价格目录（精确十进制换算），不接受前端传入的费用；
//! - **前置校验**：名称/型号非空（数据库 CHECK 已保证，仍显式兜底）、preparation ready、
//!   至少 front + left/back/right 之一、图片同物品且每视图唯一、Provider 与价格配置存在；
//!   缺项在**同一响应**里列出（`details.items`），缺配置返回 409；
//! - **发送范围可回读**：报价载荷含确认页所需的全部信息（视图集合、页范围/页图、
//!   模型名与参数、价格版本与保守上界），落库与响应同源。

use manual_core::domain::{Item, PhotoView, Preparation, PreparationState};
use manual_core::generation::{
    GenerationFingerprint, MANUAL_EXTRACT_PROMPT_VERSION, PhotoFingerprint, QUOTE_TTL_SECONDS,
    QuoteAmounts, compute_input_hash, manual_ai_amount, plan_manual_ai, tripo_amount,
};
use manual_core::timestamps::Timestamp;
use manual_core::validation::FieldIssue;
use sqlx::SqliteConnection;

use crate::config::Settings;
use crate::http::dto::{
    BUDGET_NOTICE, EstimateRequest, ManualAiConfigDto, ManualAiSendScopeDto, PageRangeDto,
    PhotoScopeDto, ProviderConfigDto, QuoteDto, SendScopeDto, TripoParametersDto,
    TripoSendScopeDto,
};
use crate::storage::repo::photos::PhotoWithHash;
use crate::storage::repo::quotes::{self, NewQuote, QuoteRecord};
use crate::storage::repo::{audit, items, photos, preparations as preps};

use super::GenerationError;

/// 前置缺项的 `details.items[].code`（QA 与前端按此零歧义识别）。
pub mod precondition_codes {
    /// 物品名称为空（数据库 CHECK 之外的显式兜底）。
    pub const ITEM_NAME_MISSING: &str = "itemNameMissing";
    /// 物品型号为空。
    pub const ITEM_MODEL_MISSING: &str = "itemModelMissing";
    /// preparation 尚未 ready（可用 `missingPages` 续传）。
    pub const PREPARATION_NOT_READY: &str = "preparationNotReady";
    /// detail（特写）照片不进入多视图请求。
    pub const DETAIL_NOT_ALLOWED: &str = "detailViewNotAllowed";
    /// 缺少 front（正面）视图。
    pub const MISSING_FRONT: &str = "missingFrontView";
    /// 缺少 left/back/right 中的任一侧视图。
    pub const MISSING_SIDE: &str = "missingSideView";
}

/// 生成报价：校验 → 计算 → 落库（`quotes`，含完整载荷与到期时间）。
pub async fn create_estimate(
    settings: &Settings,
    conn: &mut SqliteConnection,
    item_id: &str,
    request: &EstimateRequest,
    now: Timestamp,
) -> Result<QuoteRecord, GenerationError> {
    // 1) 字段级形状（缺字段一次性列全，不给库层猜）。
    let (preparation_id, photo_ids, model_preset) = validate_estimate_request(request)?;

    // 2) 供应商与价格配置（缺 → 409，明确缺项；不回落 mock）。
    let pricing = resolve_pricing(settings, &model_preset)?;
    let provider_config = pricing.provider_config.clone();

    // 3) 物品与准备记录（归属不符与不存在同为 404）。
    let item = items::get(conn, item_id)
        .await?
        .ok_or_else(|| GenerationError::not_found(format!("物品不存在：{item_id}")))?;
    let preparation = load_preparation_for_item(conn, item_id, &preparation_id).await?;

    // 4) 前置条件（一次性列出全部缺项；有缺项就不计算金额）。
    let photo_ids: Vec<String> = photo_ids;
    let resolved_photos = resolve_requested_photos(conn, item_id, &photo_ids).await?;
    let missing = collect_preconditions(&item, &preparation, &resolved_photos);
    if !missing.is_empty() {
        return Err(GenerationError::unprocessable_with(
            "preconditionsFailed",
            "报价前置条件未满足：请按明细补齐后重试",
            serde_json::json!({
                "items": missing,
                "presentViews": resolved_photos.iter().map(|photo| photo.photo.view.as_str()).collect::<Vec<_>>(),
            }),
        ));
    }

    // 5) 计划与金额（页集合来自 ready preparation；金额全部整数运算）。
    let pages = preps::page_quote_inputs(conn, &preparation.id).await?;
    let plan = plan_manual_ai(&pages).map_err(|plan_error| {
        GenerationError::unprocessable_with(
            "pageSetUnusable",
            format!("无法从当前准备记录计算计划：{plan_error}"),
            serde_json::json!({ "preparationId": preparation.id }),
        )
    })?;
    let amounts = QuoteAmounts {
        tripo: tripo_amount(&pricing.preset.pricing()).map_err(money_error)?,
        manual_ai: manual_ai_amount(&plan, &pricing.manual_ai).map_err(money_error)?,
    };

    // 6) 冻结指纹与载荷（照片按槽位顺序；不保存任何密钥）。
    let provider_config_json = serde_json::to_value(&provider_config).map_err(|error| {
        GenerationError::Storage(crate::storage::StorageError::Database {
            detail: format!("供应商配置无法序列化：{error}"),
        })
    })?;
    let fingerprint = build_fingerprint(
        &item,
        &preparation,
        &resolved_photos,
        &model_preset,
        &provider_config_json,
        &pricing.price_version,
    );
    let input_hash = compute_input_hash(&fingerprint);

    let amounts_dto = QuoteDto::amounts_from(&amounts);
    let send_scope = SendScopeDto {
        tripo: TripoSendScopeDto {
            views: resolved_photos
                .iter()
                .map(|photo| PhotoScopeDto {
                    view: photo.photo.view.as_str().to_owned(),
                    photo_id: photo.photo.id.clone(),
                    sha256: photo.sha256.clone(),
                })
                .collect(),
            model: provider_config.tripo.model.clone(),
            preset: provider_config.tripo.preset.clone(),
            parameters: provider_config.tripo.clone(),
        },
        manual_ai: ManualAiSendScopeDto::from_plan(
            &item,
            &provider_config.manual_ai.model,
            &provider_config.manual_ai.prompt_version,
            &plan,
            plan.output_tokens_upper_bound,
        ),
        price_version: pricing.price_version.clone(),
        price_snapshot_date: pricing.price_snapshot_date.clone(),
        planned_upper_bound: QuoteDto::amounts_from(&amounts),
        budget_notice: BUDGET_NOTICE.to_owned(),
    };
    let quote = QuoteDto {
        // id 在落库前生成：落库载荷与响应必须逐字节同源（报价不可回填）。
        id: manual_core::ids::new_id(),
        item_id: item.id.clone(),
        preparation_id: preparation.id.clone(),
        model_preset: model_preset.clone(),
        provider_config: provider_config.clone(),
        page_count: plan.page_count,
        page_range: PageRangeDto {
            from: plan.page_from,
            to: plan.page_to,
        },
        max_output_tokens: plan.output_tokens_upper_bound,
        price_version: pricing.price_version.clone(),
        price_snapshot_date: pricing.price_snapshot_date.clone(),
        amounts: amounts_dto,
        send_scope,
        expires_at: now
            .checked_add_millis((QUOTE_TTL_SECONDS * 1000) as i64)
            .ok_or_else(|| GenerationError::unprocessable("clockOverflow", "报价到期时间溢出"))?,
        confirmed_at: None,
        consumed_at: None,
        consumed_job_id: None,
        created_at: now,
        budget_notice: BUDGET_NOTICE.to_owned(),
    };

    // 落库载荷与响应同源：同一个 `QuoteDto` 的序列化结果。
    let quote_json = serde_json::to_string(&quote).map_err(serialization_error)?;
    let record = quotes::insert(
        conn,
        NewQuote {
            id: quote.id.clone(),
            item_id: item.id.clone(),
            preparation_id: preparation.id.clone(),
            photo_ids_json: serde_json::to_string(
                &resolved_photos
                    .iter()
                    .map(|photo| photo.photo.id.clone())
                    .collect::<Vec<_>>(),
            )
            .map_err(serialization_error)?,
            photo_hashes_json: serde_json::to_string(
                &resolved_photos
                    .iter()
                    .map(|photo| photo.sha256.clone())
                    .collect::<Vec<_>>(),
            )
            .map_err(serialization_error)?,
            input_hash,
            model_preset: model_preset.clone(),
            provider_config_json: serde_json::to_string(&provider_config_json)
                .map_err(serialization_error)?,
            price_version: pricing.price_version.clone(),
            price_snapshot_date: pricing.price_snapshot_date.clone(),
            page_count: plan.page_count,
            max_output_tokens: plan.output_tokens_upper_bound,
            quote_json,
            expires_at: quote.expires_at,
        },
        now,
    )
    .await?;
    Ok(record)
}

/// 确认结果（首次确认与幂等读回共用同一形状）。
#[derive(Debug, Clone)]
pub struct QuoteConfirmation {
    pub confirmation: crate::http::dto::ConfirmationDto,
    /// `true` = 之前已确认（本次未写审计、未改时间）。
    pub already_confirmed: bool,
}

/// 记录"云端发送确认"（REQ-021）：未确认不允许提交任务；**不存在默认勾选**。
///
/// 语义：
/// - 过期报价不允许确认（`quoteExpired`；确认了也提交不了，提前告知）；
/// - 确认与审计在同一短事务；重复确认幂等（返回首次确认时间与范围，不覆盖）；
/// - 审计只记必要摘要（视图/页范围/模型/价格版本/上界），不含密钥与资料内容。
pub async fn confirm_quote(
    conn: &mut SqliteConnection,
    item_id: &str,
    quote_id: &str,
    admin_id: &str,
    now: Timestamp,
) -> Result<QuoteConfirmation, GenerationError> {
    let quote = quotes::get(conn, quote_id)
        .await?
        .filter(|quote| quote.item_id == item_id)
        .ok_or_else(|| GenerationError::not_found(format!("报价不存在：{quote_id}")))?;
    if quote.is_expired(now) {
        return Err(GenerationError::unprocessable_with(
            "quoteExpired",
            "报价已过期：请重新获取报价后再确认",
            serde_json::json!({
                "quoteId": quote.id,
                "expiresAt": quote.expires_at.to_rfc3339(),
            }),
        ));
    }
    let payload = quote_payload(&quote).await?;

    // 幂等读回：已有确认直接返回首次确认（不覆盖时间与范围）。
    if let (Some(_), Some(json)) = (&quote.confirmed_at, &quote.confirmation_json) {
        let confirmation: crate::http::dto::ConfirmationDto = serde_json::from_value(json.clone())
            .map_err(|error| {
                GenerationError::Storage(crate::storage::StorageError::Database {
                    detail: format!("quotes.confirmation_json 无法解析：{error}"),
                })
            })?;
        return Ok(QuoteConfirmation {
            confirmation,
            already_confirmed: true,
        });
    }

    let confirmation = crate::http::dto::ConfirmationDto {
        quote_id: quote.id.clone(),
        confirmed_at: now,
        send_scope: payload.send_scope.clone(),
        summary: format!("已确认发送范围（{}）", now.to_rfc3339()),
    };
    let confirmation_json = serde_json::to_string(&confirmation).map_err(serialization_error)?;

    // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
    let mut tx = crate::storage::begin_write(conn).await?;
    let newly = quotes::mark_confirmed(&mut tx, &quote.id, &confirmation_json, now).await?;
    if newly.is_some() {
        audit::record(
            &mut tx,
            audit::NewAuditEvent {
                entity_type: "quote".to_owned(),
                entity_id: quote.id.clone(),
                actor: Some(admin_id.to_owned()),
                action: super::jobs::AUDIT_QUOTE_CONFIRMED.to_owned(),
                result: "accepted".to_owned(),
                metadata_json: Some(scope_summary(&payload).to_string()),
            },
            now,
        )
        .await?;
    }
    tx.commit().await?;

    Ok(QuoteConfirmation {
        confirmation,
        already_confirmed: newly.is_none(),
    })
}

/// 确认审计的必要摘要（不含密钥、不含资料内容）。
fn scope_summary(payload: &QuoteDto) -> serde_json::Value {
    serde_json::json!({
        "quoteId": payload.id,
        "itemId": payload.item_id,
        "preparationId": payload.preparation_id,
        "tripoViews": payload
            .send_scope
            .tripo
            .views
            .iter()
            .map(|view| serde_json::json!({ "view": view.view, "photoId": view.photo_id }))
            .collect::<Vec<_>>(),
        "tripoModel": payload.send_scope.tripo.model,
        "tripoPreset": payload.send_scope.tripo.preset,
        "tripoParameters": payload.send_scope.tripo.parameters,
        "itemName": payload.send_scope.manual_ai.item_name,
        "itemModel": payload.send_scope.manual_ai.item_model,
        "manualAiModel": payload.send_scope.manual_ai.model,
        "promptVersion": payload.send_scope.manual_ai.prompt_version,
        "pageFrom": payload.send_scope.manual_ai.page_from,
        "pageTo": payload.send_scope.manual_ai.page_to,
        "textPages": payload.send_scope.manual_ai.text_pages.len(),
        "imagePages": payload.send_scope.manual_ai.image_pages.len(),
        "priceVersion": payload.price_version,
        "upperBound": {
            "tripoCreditMinor": payload.amounts.tripo.upper_bound_minor,
            "manualAiUsdMicros": payload.amounts.manual_ai.upper_bound_minor,
        },
    })
}

/// 重新计算报价载荷（`jobs` 侧做输入比对与金额回读；不信任前端）。
///
/// 注意：这里解析的是**创建时冻结**的 `quote_json`，其中
/// `confirmed_at`/`consumed_at`/`consumed_job_id` 恒为 null；若要把载荷返回给
/// 界面（回读/回显），调用方必须用 `QuoteRecord` 的当前列覆盖这三个字段
/// （见 `http::estimates::get_estimate` 的 BUG-004 修复）。
pub async fn quote_payload(record: &QuoteRecord) -> Result<QuoteDto, GenerationError> {
    serde_json::from_str(&record.quote_json).map_err(|error| {
        GenerationError::Storage(crate::storage::StorageError::Database {
            detail: format!("quotes.quote_json 无法解析：{error}"),
        })
    })
}

/// 重新计算输入指纹（报价校验与建单校验共用；照片集合由调用方给出）。
pub async fn recompute_input_hash(
    item: &Item,
    preparation: &Preparation,
    photos: &[PhotoWithHash],
    model_preset: &str,
    provider_config: &serde_json::Value,
    price_version: &str,
) -> Result<String, GenerationError> {
    let fingerprint = build_fingerprint(
        item,
        preparation,
        photos,
        model_preset,
        provider_config,
        price_version,
    );
    Ok(compute_input_hash(&fingerprint))
}

/// 按报价里的照片集合读回照片（建单时"输入未变"的比对入口）。
pub async fn load_photos_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
    photo_ids: &[String],
) -> Result<Vec<PhotoWithHash>, GenerationError> {
    resolve_requested_photos(conn, item_id, photo_ids).await
}

/// 按物品读回 preparation（归属不符与不存在同为 404）。
pub async fn load_preparation_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
    preparation_id: &str,
) -> Result<Preparation, GenerationError> {
    match preps::item_id_of(conn, preparation_id).await? {
        Some(owner) if owner == item_id => {}
        _ => {
            return Err(GenerationError::not_found(format!(
                "preparation 不存在或不属于该物品：{preparation_id}"
            )));
        }
    }
    preps::get(conn, preparation_id)
        .await?
        .ok_or_else(|| GenerationError::not_found(format!("preparation 不存在：{preparation_id}")))
}

// ---------------------------------------------------------------------------
// 内部实现
// ---------------------------------------------------------------------------

/// 字段级校验（缺字段一次列全）。
pub(crate) fn validate_estimate_request(
    request: &EstimateRequest,
) -> Result<(String, Vec<String>, String), GenerationError> {
    let mut issues: Vec<FieldIssue> = Vec::new();
    let preparation_id = request
        .preparation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if preparation_id.is_none() {
        issues.push(FieldIssue::new(
            "preparationId",
            "必填：报价必须绑定一份 ready 的 PDF 准备记录",
        ));
    }
    let photo_ids: Vec<String> = request
        .photo_ids
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    if photo_ids.is_empty() {
        issues.push(FieldIssue::new(
            "photoIds",
            "必填：至少需要 front 与 left/back/right 中的一个视图照片",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    if photo_ids.iter().any(|id| !seen.insert(id.clone())) {
        issues.push(FieldIssue::new(
            "photoIds",
            "同一张照片只能出现一次（每视图最多一张）",
        ));
    }
    let model_preset = request
        .model_preset
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if model_preset.is_none() {
        issues.push(FieldIssue::new(
            "modelPreset",
            "必填：请选择价格目录中受支持的模型预设",
        ));
    }
    if !issues.is_empty() {
        return Err(GenerationError::FieldValidation(issues));
    }
    Ok((
        preparation_id.expect("无问题时必有值"),
        photo_ids,
        model_preset.expect("无问题时必有值"),
    ))
}

/// 解析后的定价上下文（报价与后续快照/账本共用同一组事实）。
pub(crate) struct PricingContext {
    pub preset: crate::generation::catalog::TripoPreset,
    pub provider_config: ProviderConfigDto,
    pub manual_ai: manual_core::generation::ManualAiPricing,
    pub price_version: String,
    pub price_snapshot_date: String,
}

/// 解析供应商配置与价格（缺配置 → 409；预设不存在 → 422）。
fn resolve_pricing(
    settings: &Settings,
    model_preset: &str,
) -> Result<PricingContext, GenerationError> {
    let providers = &settings.providers;
    let mut missing: Vec<String> = Vec::new();
    if !providers.tripo.configured() {
        missing.extend(
            providers
                .tripo
                .missing()
                .into_iter()
                .map(|item| format!("tripo.{item}")),
        );
    }
    if !providers.manual_ai.configured() {
        missing.extend(
            providers
                .manual_ai
                .missing()
                .into_iter()
                .map(|item| format!("manual_ai.{item}")),
        );
    }
    if !missing.is_empty() {
        return Err(GenerationError::ProviderNotConfigured { missing });
    }

    let Some(catalog) = settings.price_catalog.as_ref() else {
        return Err(GenerationError::PriceCatalogMissing {
            missing: vec!["priceCatalog（未配置 price_catalog_path 或文件为空）".to_owned()],
        });
    };
    let Some(preset) = catalog.tripo_preset(model_preset) else {
        return Err(GenerationError::unprocessable_with(
            "modelPresetUnsupported",
            format!("该模型预设不在受支持清单中：{model_preset}"),
            serde_json::json!({ "supportedPresets": catalog.preset_keys() }),
        ));
    };
    let configured_tripo_model = providers.tripo.model.as_deref().unwrap_or_default();
    if configured_tripo_model != preset.parameters.model {
        return Err(GenerationError::PriceCatalogMissing {
            missing: vec![format!(
                "tripoModelMismatch（providers.tripo.model={configured_tripo_model}，预设 {} 对应 {}）",
                preset.key, preset.parameters.model
            )],
        });
    }
    let manual_ai_model = providers.manual_ai.model.as_deref().unwrap_or_default();
    let Some(manual_pricing) = catalog.manual_ai_pricing(manual_ai_model) else {
        return Err(GenerationError::PriceCatalogMissing {
            missing: vec![format!(
                "manualAiModel:{manual_ai_model}（价格目录里没有该模型的单价）"
            )],
        });
    };

    let provider_config = ProviderConfigDto {
        tripo: TripoParametersDto::new(&preset.key, &preset.parameters),
        manual_ai: ManualAiConfigDto {
            model: manual_ai_model.to_owned(),
            prompt_version: MANUAL_EXTRACT_PROMPT_VERSION.to_owned(),
        },
    };
    Ok(PricingContext {
        preset: preset.clone(),
        provider_config,
        manual_ai: manual_pricing.clone(),
        price_version: catalog.version.clone(),
        price_snapshot_date: catalog.snapshot_date.clone(),
    })
}

/// 解析请求里的照片（归属/存在性；顺序按多视图槽位）。
pub(crate) async fn resolve_requested_photos(
    conn: &mut SqliteConnection,
    item_id: &str,
    photo_ids: &[String],
) -> Result<Vec<PhotoWithHash>, GenerationError> {
    let mut resolved: Vec<PhotoWithHash> = Vec::with_capacity(photo_ids.len());
    for photo_id in photo_ids {
        let photo = photos::find_with_hash_for_item(conn, item_id, photo_id)
            .await?
            .ok_or_else(|| GenerationError::not_found("照片不存在或不属于该物品".to_owned()))?;
        resolved.push(photo);
    }
    resolved.sort_by_key(|photo| slot_order(photo.photo.view));
    Ok(resolved)
}

/// 多视图槽位顺序（front→left→back→right→detail）。
pub(crate) fn slot_order(view: PhotoView) -> u8 {
    match view {
        PhotoView::Front => 1,
        PhotoView::Left => 2,
        PhotoView::Back => 3,
        PhotoView::Right => 4,
        PhotoView::Detail => 5,
    }
}

/// 前置条件缺项（一次列全；顺序稳定）。
pub(crate) fn collect_preconditions(
    item: &Item,
    preparation: &Preparation,
    photos: &[PhotoWithHash],
) -> Vec<serde_json::Value> {
    use precondition_codes as codes;
    let mut missing: Vec<serde_json::Value> = Vec::new();
    if item.name.trim().is_empty() {
        missing.push(serde_json::json!({
            "code": codes::ITEM_NAME_MISSING,
            "message": "物品缺少名称：请先补全物品信息",
        }));
    }
    if item.model.trim().is_empty() {
        missing.push(serde_json::json!({
            "code": codes::ITEM_MODEL_MISSING,
            "message": "物品缺少型号：请先补全物品信息",
        }));
    }
    if preparation.state != PreparationState::Ready {
        missing.push(serde_json::json!({
            "code": codes::PREPARATION_NOT_READY,
            "message": "PDF 准备尚未完成（state=preparing）：请完成后重试（可用缺页接口续传）",
        }));
    }
    if photos.iter().any(|photo| !photo.photo.view.is_multiview()) {
        missing.push(serde_json::json!({
            "code": codes::DETAIL_NOT_ALLOWED,
            "message": "detail（特写）照片不进入多视图请求：请只选择 front/left/back/right",
        }));
    }
    let has_front = photos
        .iter()
        .any(|photo| photo.photo.view == PhotoView::Front);
    if !has_front {
        missing.push(serde_json::json!({
            "code": codes::MISSING_FRONT,
            "message": "缺少 front（正面）视图照片",
        }));
    }
    let has_side = photos.iter().any(|photo| {
        matches!(
            photo.photo.view,
            PhotoView::Left | PhotoView::Back | PhotoView::Right
        )
    });
    if !has_side {
        missing.push(serde_json::json!({
            "code": codes::MISSING_SIDE,
            "message": "至少需要 left/back/right 中的一个侧视图照片（当前一个都没有）",
        }));
    }
    missing
}

/// 输入指纹（照片按槽位顺序；`photoId + sha256` 一起冻结）。
pub(crate) fn build_fingerprint(
    item: &Item,
    preparation: &Preparation,
    photos: &[PhotoWithHash],
    model_preset: &str,
    provider_config: &serde_json::Value,
    price_version: &str,
) -> GenerationFingerprint {
    GenerationFingerprint {
        item_id: item.id.clone(),
        item_revision: item.revision,
        item_name: item.name.clone(),
        item_model: item.model.clone(),
        preparation_id: preparation.id.clone(),
        preparation_source_sha256: preparation.source_sha256.clone(),
        page_count: preparation.page_count.unwrap_or_default(),
        photos: photos
            .iter()
            .map(|photo| PhotoFingerprint {
                photo_id: photo.photo.id.clone(),
                view: photo.photo.view.as_str().to_owned(),
                sha256: photo.sha256.clone(),
            })
            .collect(),
        model_preset: model_preset.to_owned(),
        provider_config: provider_config.clone(),
        prompt_version: MANUAL_EXTRACT_PROMPT_VERSION.to_owned(),
        price_version: price_version.to_owned(),
    }
}

fn money_error(error: manual_core::cost::MoneyError) -> GenerationError {
    GenerationError::Storage(crate::storage::StorageError::Database {
        detail: format!("金额换算失败：{error}"),
    })
}

fn serialization_error(error: serde_json::Error) -> GenerationError {
    GenerationError::Storage(crate::storage::StorageError::Database {
        detail: format!("序列化失败：{error}"),
    })
}
