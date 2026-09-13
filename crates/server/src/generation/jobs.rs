//! 任务创建服务（T11 / REQ-022）：重新校验 → 同一事务内冻结快照 + 预留费用 + 建单 + 幂等。
//!
//! 语义（contracts.md §4；PRD AC-031/AC-032）：
//! - 服务端**重新校验**：引用（物品/准备/照片归属）、报价未过期、输入未变（重算输入指纹）、
//!   价格版本未变、预算足够（授权上限必须覆盖服务器计算的保守上界）；
//! - **不接受前端传入的费用数值**：金额一律从报价快照回读（服务端生成），
//!   请求体里根本没有费用字段（未知字段 422）；
//! - **幂等**：`Idempotency-Key` + `admin+method+route+key` 唯一；相同 body_hash 重放返回
//!   原 job（不新建、不重复预留）；不同 body_hash → 409 `IDEMPOTENCY_CONFLICT`；
//! - **一份报价只能建一份任务**：`quotes.consumed_at` 条件更新保证（重生成必须新报价）；
//! - **同一事务**：快照、预留、job、阶段、幂等记录、审计事件要么全部提交要么全部回滚
//!   （事务中断不留半笔预留；测试构建用断点注入验证）。
//!
//! 阶段按 contracts.md §5 的 DAG 建齐：`freeze_inputs` 在入队事务里直接 `succeeded`，
//! `manual_extract` 批次按 ≤5 页/批展开，`manual_merge` 必须在批次之后插入（依赖边），
//! 之后是 Tripo 链与 `assemble_draft`。真实阶段执行属 T12/T14/T15。

use std::collections::BTreeSet;

use manual_core::domain::{
    CostLedgerEntry, Currency, GenerationSnapshot, Job, ProviderKey, StageKind,
};
use manual_core::generation::{
    MANUAL_EXTRACT_PROMPT_VERSION, ManualAiPlan, plan_manual_ai, sha256_hex,
};
use manual_core::timestamps::Timestamp;
use manual_core::validation::FieldIssue;
use serde::Serialize;
use sqlx::SqliteConnection;

use crate::config::Settings;
use crate::http::dto::{JobCreateRequest, QuoteDto};
use crate::storage::StorageError;
use crate::storage::repo::job_stages::{self, NewStage};
use crate::storage::repo::photos::PhotoWithHash;
use crate::storage::repo::snapshots::{self, NewSnapshot};
use crate::storage::repo::{
    audit, idempotency, items, jobs as jobs_repo, ledger, preparations as preps, quotes,
    quotes::QuoteRecord,
};

use super::GenerationError;
use super::estimate::{
    load_photos_for_item, load_preparation_for_item, quote_payload, recompute_input_hash,
};

/// 幂等记录的 scope（contracts.md §4：`admin + method + route + key`）。
pub const IDEMPOTENCY_METHOD: &str = "POST";
/// 路由**模板**（不是具体路径）。
pub const IDEMPOTENCY_ROUTE: &str = "/api/v1/items/{id}/jobs";
/// 幂等键长度上限（防止异常大的头部进入库；超出 422 字段级）。
pub const IDEMPOTENCY_KEY_MAX_CHARS: usize = 200;

/// 审计动作：云端发送确认（REQ-021）。
pub const AUDIT_QUOTE_CONFIRMED: &str = "generation_send_scope_confirmed";
/// 审计动作：任务建立并冻结费用（REQ-022）。
pub const AUDIT_JOB_CREATED: &str = "generation_job_created";

/// 建单结果（首建与重放共用同一形状）。
#[derive(Debug, Clone)]
pub struct JobCreation {
    pub job: Job,
    pub snapshot: GenerationSnapshot,
    pub reservations: Vec<CostLedgerEntry>,
    /// 是否为同键同 body 的重放（未新建任何记录）。
    pub replayed: bool,
}

/// 校验并创建任务（`POST /items/{id}/jobs`；首次 202，重放返回同一 job）。
#[allow(clippy::too_many_arguments)]
pub async fn create_job(
    settings: &Settings,
    conn: &mut SqliteConnection,
    item_id: &str,
    request: &JobCreateRequest,
    idempotency_key: &str,
    admin_id: &str,
    now: Timestamp,
) -> Result<JobCreation, GenerationError> {
    // 1) 字段级校验 + body_hash（键的作用域 = 管理员 + 方法 + 路由 + key）。
    let validated = validate_job_request(request)?;
    let key = validate_idempotency_key(idempotency_key)?;
    let body_hash = compute_body_hash(&validated)?;

    // 2) 快速重放检查（真正的并发竞争由事务内的唯一键兜底）。
    if let Some(creation) = lookup_replay(conn, admin_id, &key, &body_hash).await? {
        return Ok(creation);
    }

    // 3) 引用与报价状态。
    let item = items::get(conn, item_id)
        .await?
        .ok_or_else(|| GenerationError::not_found(format!("物品不存在：{item_id}")))?;
    let quote = quotes::get(conn, &validated.quote_id)
        .await?
        .filter(|quote| quote.item_id == item_id)
        .ok_or_else(|| GenerationError::not_found(format!("报价不存在：{}", validated.quote_id)))?;
    let payload = quote_payload(&quote).await?;

    if let Err(error) = check_quote_state(&quote, &payload, &validated, now) {
        // 并发同键竞争窗口：另一请求可能已消费该报价并写下幂等记录 → 按重放返回。
        if is_quote_already_used(&error)
            && let Some(creation) = lookup_replay(conn, admin_id, &key, &body_hash).await?
        {
            return Ok(creation);
        }
        return Err(error);
    }

    // 4) 价格版本与输入指纹（输入未变 = 照片内容/说明书/物品版本都与报价时一致）。
    let Some(catalog) = settings.price_catalog.as_ref() else {
        return Err(GenerationError::PriceCatalogMissing {
            missing: vec!["priceCatalog（报价校验需要价格目录在场）".to_owned()],
        });
    };
    if catalog.version != quote.price_version {
        return Err(GenerationError::unprocessable_with(
            "priceVersionChanged",
            "价格版本已更新：请重新获取报价并重新确认发送内容",
            serde_json::json!({
                "quotePriceVersion": quote.price_version,
                "currentPriceVersion": catalog.version,
            }),
        ));
    }
    let photos = load_photos_for_item(conn, item_id, &quote.photo_ids).await?;
    let preparation = load_preparation_for_item(conn, item_id, &quote.preparation_id).await?;
    let recomputed = recompute_input_hash(
        &item,
        &preparation,
        &photos,
        &quote.model_preset,
        &quote.provider_config,
        &quote.price_version,
    )
    .await?;
    if recomputed != quote.input_hash {
        return Err(GenerationError::unprocessable_with(
            "inputChanged",
            "资料已更新（照片/说明书/物品信息已变化）：请重新获取报价并重新确认发送内容",
            serde_json::json!({ "quoteId": quote.id }),
        ));
    }

    // 5) 计划（批次页集合用于阶段建单；与报价时同源计算）。
    let page_inputs = preps::page_quote_inputs(conn, &preparation.id).await?;
    let plan = plan_manual_ai(&page_inputs).map_err(|plan_error| {
        GenerationError::unprocessable_with(
            "pageSetUnusable",
            format!("无法从当前准备记录计算计划：{plan_error}"),
            serde_json::json!({ "preparationId": preparation.id }),
        )
    })?;

    // 6) 同一事务：快照 + 预留 + job + 阶段 + 幂等记录 + 审计。
    // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
    let tx = crate::storage::begin_write(conn).await?;
    let result = create_in_transaction(
        tx,
        &item.id,
        item.revision,
        &preparation.id,
        &quote,
        &payload,
        &plan,
        &validated,
        &photos,
        &body_hash,
        &key,
        admin_id,
        now,
    )
    .await;

    match result {
        Ok(creation) => Ok(creation),
        Err(CreateTransactionError::IdempotencyRace) => {
            // 并发同键：唯一键拒绝后按已存在的记录返回（同 body）或 409（不同 body）。
            let existing =
                idempotency::find(conn, admin_id, IDEMPOTENCY_METHOD, IDEMPOTENCY_ROUTE, &key)
                    .await?
                    .ok_or_else(|| {
                        GenerationError::Storage(StorageError::Database {
                            detail: "幂等键竞争后找不到已存在的记录".to_owned(),
                        })
                    })?;
            replay_or_conflict(conn, existing, &body_hash).await
        }
        Err(CreateTransactionError::QuoteAlreadyUsed) => {
            // 并发同键竞争：同 key 同 body 的另一请求先消费了报价 → 返回它的 job。
            if let Some(creation) = lookup_replay(conn, admin_id, &key, &body_hash).await? {
                return Ok(creation);
            }
            let job_id = quotes::get(conn, &quote.id)
                .await?
                .and_then(|record| record.consumed_job_id);
            Err(GenerationError::unprocessable_with(
                "quoteAlreadyUsed",
                "该报价已经创建过任务：请重新获取报价（重生成需要新快照 + 新预算确认）",
                serde_json::json!({ "jobId": job_id }),
            ))
        }
        Err(CreateTransactionError::Generation(error)) => Err(error),
    }
}

// ---------------------------------------------------------------------------
// 事务内建单
// ---------------------------------------------------------------------------

enum CreateTransactionError {
    /// 同键并发：唯一键拒绝（外层读回并按重放/冲突处理）。
    IdempotencyRace,
    /// 报价已被消费（外层附上已有 job id）。
    QuoteAlreadyUsed,
    Generation(GenerationError),
}

impl From<StorageError> for CreateTransactionError {
    fn from(error: StorageError) -> Self {
        Self::Generation(GenerationError::Storage(error))
    }
}

impl From<sqlx::Error> for CreateTransactionError {
    /// 事务原语（commit/rollback）失败按存储错误处理（不吞掉，也不重试）。
    fn from(error: sqlx::Error) -> Self {
        Self::Generation(GenerationError::Storage(StorageError::from(error)))
    }
}

#[allow(clippy::too_many_arguments)]
async fn create_in_transaction(
    tx: sqlx::Transaction<'_, sqlx::Sqlite>,
    item_id: &str,
    item_revision: i64,
    preparation_id: &str,
    quote: &QuoteRecord,
    payload: &QuoteDto,
    plan: &ManualAiPlan,
    validated: &ValidatedJobRequest,
    photos: &[PhotoWithHash],
    body_hash: &str,
    key: &str,
    admin_id: &str,
    now: Timestamp,
) -> Result<JobCreation, CreateTransactionError> {
    let mut tx = tx;

    // 1) 冻结输入快照（photo_ids 与 photo_hashes 一一对应、槽位顺序；不含任何密钥）。
    let budgets = serde_json::json!({
        "quoteId": quote.id,
        "priceVersion": quote.price_version,
        "authorized": {
            "tripoCreditMinor": validated.limits.tripo_credit_minor,
            "manualAiUsdMicros": validated.limits.manual_ai_usd_micros,
        },
        "upperBound": {
            "tripoCreditMinor": payload.amounts.tripo.upper_bound_minor,
            "manualAiUsdMicros": payload.amounts.manual_ai.upper_bound_minor,
        },
        "estimated": {
            "tripoCreditMinor": payload.amounts.tripo.estimated_minor,
            "manualAiUsdMicros": payload.amounts.manual_ai.estimated_minor,
        },
        "budgetNotice": crate::http::dto::BUDGET_NOTICE,
    });
    let snapshot = snapshots::insert(
        &mut tx,
        NewSnapshot {
            item_id: item_id.to_owned(),
            // 指纹已在建单前重算并与报价一致，因此这里的物品版本与报价冻结时相同。
            item_revision,
            preparation_id: preparation_id.to_owned(),
            photo_ids_json: serde_json::to_string(&quote.photo_ids).map_err(serialization)?,
            photo_hashes_json: serde_json::to_string(&quote.photo_hashes).map_err(serialization)?,
            provider_config_json: serde_json::to_string(&quote.provider_config)
                .map_err(serialization)?,
            prompt_version: MANUAL_EXTRACT_PROMPT_VERSION.to_owned(),
            price_version: quote.price_version.clone(),
            budgets_json: serde_json::to_string(&budgets).map_err(serialization)?,
        },
        now,
    )
    .await?;

    // 2) 预留费用 = 服务器计算的保守上界（**不采信前端数值**；分列、不相加）。
    let reservations = vec![
        ledger::reserve(
            &mut tx,
            ledger::NewReservation {
                snapshot_id: snapshot.id.clone(),
                provider: ProviderKey::Tripo,
                currency: Currency::CreditMinor,
                reserved: payload.amounts.tripo.upper_bound_minor,
                price_version: quote.price_version.clone(),
            },
            now,
        )
        .await?,
        ledger::reserve(
            &mut tx,
            ledger::NewReservation {
                snapshot_id: snapshot.id.clone(),
                provider: ProviderKey::ManualAi,
                currency: Currency::UsdMicros,
                reserved: payload.amounts.manual_ai.upper_bound_minor,
                price_version: quote.price_version.clone(),
            },
            now,
        )
        .await?,
    ];

    // 3) 创建 job（queued；阶段执行属 T10 执行器 / T12 / T14）。
    let job = jobs_repo::create(
        &mut tx,
        jobs_repo::NewJob {
            item_id: item_id.to_owned(),
            snapshot_id: snapshot.id.clone(),
        },
    )
    .await?;

    // 4) 消费报价（一份报价一份任务；竞争失败即回滚，不产生第二份生成单）。
    if !quotes::consume(&mut tx, &quote.id, &job.id, now).await? {
        tx.rollback().await?;
        return Err(CreateTransactionError::QuoteAlreadyUsed);
    }

    // 5) 阶段 DAG（freeze_inputs 直接 succeeded；manual_merge 必须在批次之后插入）。
    for stage in build_stage_plan(&job.id, plan, quote, photos) {
        job_stages::insert(&mut tx, stage, now).await?;
    }

    // 6) 幂等记录（唯一键竞争 = 并发同键，交给外层按重放/409 处理）。
    match idempotency::insert(
        &mut tx,
        idempotency::NewIdempotencyRecord {
            admin_id: admin_id.to_owned(),
            method: IDEMPOTENCY_METHOD.to_owned(),
            route: IDEMPOTENCY_ROUTE.to_owned(),
            key: key.to_owned(),
            body_hash: body_hash.to_owned(),
            resource_id: Some(job.id.clone()),
            response_status: Some(202),
        },
        now,
    )
    .await
    {
        Ok(_) => {}
        Err(StorageError::UniqueViolation { .. }) => {
            tx.rollback().await?;
            return Err(CreateTransactionError::IdempotencyRace);
        }
        Err(error) => return Err(error.into()),
    }

    // 7) 审计：建单与冻结（费用确认已在 confirm 步骤留痕）。
    audit::record(
        &mut tx,
        audit::NewAuditEvent {
            entity_type: "job".to_owned(),
            entity_id: job.id.clone(),
            actor: Some(admin_id.to_owned()),
            action: AUDIT_JOB_CREATED.to_owned(),
            result: "accepted".to_owned(),
            metadata_json: Some(
                serde_json::json!({
                    "quoteId": quote.id,
                    "snapshotId": snapshot.id,
                    "itemId": item_id,
                    "priceVersion": quote.price_version,
                    "authorized": {
                        "tripoCreditMinor": validated.limits.tripo_credit_minor,
                        "manualAiUsdMicros": validated.limits.manual_ai_usd_micros,
                    },
                    "upperBound": {
                        "tripoCreditMinor": payload.amounts.tripo.upper_bound_minor,
                        "manualAiUsdMicros": payload.amounts.manual_ai.upper_bound_minor,
                    },
                })
                .to_string(),
            ),
        },
        now,
    )
    .await?;

    // 8) 测试构建的断点：所有写入已发生、尚未提交（验证"事务中断不留半笔预留"）。
    // owner 用本请求的幂等键：并行测试各自注册自己的键，互不干扰（T10 的 owner 分区约定）。
    #[cfg(feature = "job-failpoints")]
    crate::job_failpoint!(key, crate::jobs::failpoints::GENERATION_BEFORE_COMMIT);

    tx.commit().await?;
    Ok(JobCreation {
        job,
        snapshot,
        reservations,
        replayed: false,
    })
}

/// 组装阶段（contracts.md §5 的 DAG 顺序与依赖规则；`job_stages::insert` 自动写依赖边）。
fn build_stage_plan(
    job_id: &str,
    plan: &ManualAiPlan,
    quote: &QuoteRecord,
    photos: &[PhotoWithHash],
) -> Vec<NewStage> {
    let mut stages: Vec<NewStage> = Vec::new();
    // freeze_inputs：入队事务里直接 succeeded（输入已在上面冻结）。
    stages.push(NewStage {
        job_id: job_id.to_owned(),
        stage_kind: StageKind::FreezeInputs,
        batch_index: 0,
        page_set_json: None,
        input_hash: quote.input_hash.clone(),
        status: manual_core::domain::JobStatus::Succeeded,
    });

    // manual_extract：按 ≤5 页/批展开（每批独立持久执行单元）。
    let mut batch_hashes: Vec<serde_json::Value> = Vec::new();
    for (index, pages) in plan.batches.iter().enumerate() {
        let batch_index = index as i64;
        let page_set_json = serde_json::to_string(pages).expect("页号数组总是可序列化");
        let hash = stage_input_hash(
            StageKind::ManualExtract,
            &[
                ("preparationId", serde_json::json!(quote.preparation_id)),
                (
                    "promptVersion",
                    serde_json::json!(MANUAL_EXTRACT_PROMPT_VERSION),
                ),
                ("batchIndex", serde_json::json!(batch_index)),
                ("pages", serde_json::json!(pages)),
            ],
        );
        batch_hashes.push(serde_json::json!(hash));
        stages.push(NewStage {
            job_id: job_id.to_owned(),
            stage_kind: StageKind::ManualExtract,
            batch_index,
            page_set_json: Some(page_set_json),
            input_hash: hash,
            status: manual_core::domain::JobStatus::Queued,
        });
    }

    // manual_merge：批次的确定性合并（必须在全部批次之后插入）。
    let merge_hash = stage_input_hash(
        StageKind::ManualMerge,
        &[
            (
                "promptVersion",
                serde_json::json!(MANUAL_EXTRACT_PROMPT_VERSION),
            ),
            ("batches", serde_json::Value::Array(batch_hashes.clone())),
        ],
    );
    stages.push(NewStage {
        job_id: job_id.to_owned(),
        stage_kind: StageKind::ManualMerge,
        batch_index: 0,
        page_set_json: None,
        input_hash: merge_hash.clone(),
        status: manual_core::domain::JobStatus::Queued,
    });

    // Tripo 链（上传 → 提交 → 查询 → 下载 → 校验）。
    let photo_hashes: Vec<serde_json::Value> = photos
        .iter()
        .map(|photo| serde_json::json!(photo.sha256))
        .collect();
    let upload_hash = stage_input_hash(
        StageKind::TripoUpload,
        &[
            (
                "photoHashes",
                serde_json::Value::Array(photo_hashes.clone()),
            ),
            ("modelPreset", serde_json::json!(quote.model_preset)),
        ],
    );
    let submit_hash = stage_input_hash(
        StageKind::TripoSubmit,
        &[
            ("uploadHash", serde_json::json!(upload_hash)),
            ("providerConfig", quote.provider_config.clone()),
        ],
    );
    let poll_hash = stage_input_hash(
        StageKind::TripoPoll,
        &[("submitHash", serde_json::json!(submit_hash))],
    );
    let download_hash = stage_input_hash(
        StageKind::ModelDownload,
        &[("pollHash", serde_json::json!(poll_hash))],
    );
    let validate_hash = stage_input_hash(
        StageKind::ModelValidate,
        &[("downloadHash", serde_json::json!(download_hash))],
    );
    for (kind, hash) in [
        (StageKind::TripoUpload, upload_hash),
        (StageKind::TripoSubmit, submit_hash),
        (StageKind::TripoPoll, poll_hash),
        (StageKind::ModelDownload, download_hash),
        (StageKind::ModelValidate, validate_hash.clone()),
    ] {
        stages.push(NewStage {
            job_id: job_id.to_owned(),
            stage_kind: kind,
            batch_index: 0,
            page_set_json: None,
            input_hash: hash,
            status: manual_core::domain::JobStatus::Queued,
        });
    }

    // assemble_draft：两条分支都完成后组装（依赖边由 DAG 规则物化）。
    stages.push(NewStage {
        job_id: job_id.to_owned(),
        stage_kind: StageKind::AssembleDraft,
        batch_index: 0,
        page_set_json: None,
        input_hash: stage_input_hash(
            StageKind::AssembleDraft,
            &[
                ("mergeHash", serde_json::json!(merge_hash)),
                ("validateHash", serde_json::json!(validate_hash)),
            ],
        ),
        status: manual_core::domain::JobStatus::Queued,
    });

    stages
}

/// 阶段 `input_hash`：kind + 命名部分的确定性 JSON 的 sha256（无随机、无时间）。
fn stage_input_hash(kind: StageKind, parts: &[(&str, serde_json::Value)]) -> String {
    let mut map = serde_json::Map::new();
    map.insert(
        "kind".to_owned(),
        serde_json::Value::String(kind.as_str().to_owned()),
    );
    for (name, value) in parts {
        map.insert((*name).to_owned(), value.clone());
    }
    sha256_hex(serde_json::Value::Object(map).to_string().as_bytes())
}

// ---------------------------------------------------------------------------
// 校验与重放
// ---------------------------------------------------------------------------

/// 校验后的建单请求（body_hash 的规范形状）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedJobRequest {
    pub quote_id: String,
    pub preparation_id: String,
    pub photo_ids: Vec<String>,
    pub limits: BudgetLimits,
}

/// 用户授权的分列上限（整数最小单位；负数 422）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetLimits {
    pub tripo_credit_minor: i64,
    pub manual_ai_usd_micros: i64,
}

/// `body_hash` 的规范形状（字段顺序固定；photoIds 保持请求顺序）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalJobBody<'a> {
    quote_id: &'a str,
    preparation_id: &'a str,
    photo_ids: &'a [String],
    limits: CanonicalLimits,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalLimits {
    tripo_credit_minor: i64,
    manual_ai_usd_micros: i64,
}

/// 字段级校验（缺字段一次列全）。
pub(crate) fn validate_job_request(
    request: &JobCreateRequest,
) -> Result<ValidatedJobRequest, GenerationError> {
    let mut issues: Vec<FieldIssue> = Vec::new();
    let quote_id = request
        .quote_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if quote_id.is_none() {
        issues.push(FieldIssue::new(
            "quoteId",
            "必填：请先获取报价并确认发送范围",
        ));
    }
    let preparation_id = request
        .preparation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if preparation_id.is_none() {
        issues.push(FieldIssue::new(
            "preparationId",
            "必填：与报价绑定的准备记录",
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
            "必填：与报价一致的多视图照片集合",
        ));
    }
    let mut seen = BTreeSet::new();
    if photo_ids.iter().any(|id| !seen.insert(id.clone())) {
        issues.push(FieldIssue::new("photoIds", "同一张照片只能出现一次"));
    }
    let limits = request.limits;
    let limits = match limits {
        None => {
            issues.push(FieldIssue::new(
                "limits",
                "必填：请给出本次授权的分列上限（必须覆盖服务端计算的保守上界）",
            ));
            None
        }
        Some(limits) => match (limits.tripo_credit_minor, limits.manual_ai_usd_micros) {
            (Some(tripo), Some(manual_ai)) => {
                if tripo < 0 {
                    issues.push(FieldIssue::new("limits.tripoCreditMinor", "不能为负"));
                }
                if manual_ai < 0 {
                    issues.push(FieldIssue::new("limits.manualAiUsdMicros", "不能为负"));
                }
                Some(BudgetLimits {
                    tripo_credit_minor: tripo,
                    manual_ai_usd_micros: manual_ai,
                })
            }
            (tripo, manual_ai) => {
                if tripo.is_none() {
                    issues.push(FieldIssue::new(
                        "limits.tripoCreditMinor",
                        "必填：Tripo credits 上限（整数最小单位 creditMinor）",
                    ));
                }
                if manual_ai.is_none() {
                    issues.push(FieldIssue::new(
                        "limits.manualAiUsdMicros",
                        "必填：说明书 AI USD 上限（整数最小单位 usdMicros）",
                    ));
                }
                None
            }
        },
    };
    if !issues.is_empty() {
        return Err(GenerationError::FieldValidation(issues));
    }
    Ok(ValidatedJobRequest {
        quote_id: quote_id.expect("无问题时必有值"),
        preparation_id: preparation_id.expect("无问题时必有值"),
        photo_ids,
        limits: limits.expect("无问题时必有值"),
    })
}

/// 幂等键校验（缺失/空/超长 → 422 字段级；键本身只是定位符，不含业务语义）。
pub(crate) fn validate_idempotency_key(key: &str) -> Result<String, GenerationError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(GenerationError::FieldValidation(vec![FieldIssue::new(
            "idempotencyKey",
            "缺少 Idempotency-Key 头：创建任务必须携带幂等键（前端每轮报价生成一个并复用）",
        )]));
    }
    if trimmed.chars().count() > IDEMPOTENCY_KEY_MAX_CHARS {
        return Err(GenerationError::FieldValidation(vec![FieldIssue::new(
            "idempotencyKey",
            format!("幂等键过长（上限 {IDEMPOTENCY_KEY_MAX_CHARS} 字符）"),
        )]));
    }
    Ok(trimmed.to_owned())
}

fn compute_body_hash(validated: &ValidatedJobRequest) -> Result<String, GenerationError> {
    let canonical = CanonicalJobBody {
        quote_id: &validated.quote_id,
        preparation_id: &validated.preparation_id,
        photo_ids: &validated.photo_ids,
        limits: CanonicalLimits {
            tripo_credit_minor: validated.limits.tripo_credit_minor,
            manual_ai_usd_micros: validated.limits.manual_ai_usd_micros,
        },
    };
    let bytes = serde_json::to_vec(&canonical).map_err(|error| {
        GenerationError::Storage(StorageError::Database {
            detail: format!("body_hash 序列化失败：{error}"),
        })
    })?;
    Ok(sha256_hex(&bytes))
}

/// 报价状态校验（过期/已消费/未确认/输入集合不符/预算不足）。
fn check_quote_state(
    quote: &QuoteRecord,
    payload: &QuoteDto,
    validated: &ValidatedJobRequest,
    now: Timestamp,
) -> Result<(), GenerationError> {
    if quote.is_expired(now) {
        return Err(GenerationError::unprocessable_with(
            "quoteExpired",
            "报价已过期：请重新获取报价（并重新确认发送内容）",
            serde_json::json!({
                "quoteId": quote.id,
                "expiresAt": quote.expires_at.to_rfc3339(),
            }),
        ));
    }
    if quote.is_consumed() {
        return Err(GenerationError::unprocessable_with(
            "quoteAlreadyUsed",
            "该报价已经创建过任务：请重新获取报价（重生成需要新快照 + 新预算确认）",
            serde_json::json!({ "jobId": quote.consumed_job_id }),
        ));
    }
    if !quote.is_confirmed() {
        return Err(GenerationError::unprocessable_with(
            "confirmationRequired",
            "尚未确认发送给云端的内容：请先在确认页勾选确认（不默认勾选）",
            serde_json::json!({ "quoteId": quote.id }),
        ));
    }
    let quote_photos: BTreeSet<&String> = quote.photo_ids.iter().collect();
    let requested_photos: BTreeSet<&String> = validated.photo_ids.iter().collect();
    if quote.preparation_id != validated.preparation_id || quote_photos != requested_photos {
        return Err(GenerationError::unprocessable_with(
            "inputChanged",
            "提交的输入与报价不一致（准备记录或照片集合已变化）：请重新获取报价并重新确认",
            serde_json::json!({ "quoteId": quote.id }),
        ));
    }
    let authorized = validated.limits;
    if authorized.tripo_credit_minor < payload.amounts.tripo.upper_bound_minor
        || authorized.manual_ai_usd_micros < payload.amounts.manual_ai.upper_bound_minor
    {
        return Err(GenerationError::unprocessable_with(
            "budgetBelowPlannedUpperBound",
            "允许上限低于服务端计算的保守上界：请提高预算或调整资料后重新确认（不自动降质量/换模型）",
            serde_json::json!({
                "tripoUpperBoundCreditMinor": payload.amounts.tripo.upper_bound_minor,
                "manualAiUpperBoundUsdMicros": payload.amounts.manual_ai.upper_bound_minor,
                "authorized": {
                    "tripoCreditMinor": authorized.tripo_credit_minor,
                    "manualAiUsdMicros": authorized.manual_ai_usd_micros,
                },
            }),
        ));
    }
    Ok(())
}

/// 该错误是否是"报价已被消费"（并发同键竞争窗口需要优先按重放处理）。
fn is_quote_already_used(error: &GenerationError) -> bool {
    matches!(
        error,
        GenerationError::Unprocessable { reason, .. } if *reason == "quoteAlreadyUsed"
    )
}

/// 幂等记录命中时返回重放（同 body）或 409（不同 body）；没有记录返回 `None`。
async fn lookup_replay(
    conn: &mut SqliteConnection,
    admin_id: &str,
    key: &str,
    body_hash: &str,
) -> Result<Option<JobCreation>, GenerationError> {
    match idempotency::find(conn, admin_id, IDEMPOTENCY_METHOD, IDEMPOTENCY_ROUTE, key).await? {
        Some(record) => replay_or_conflict(conn, record, body_hash).await.map(Some),
        None => Ok(None),
    }
}

/// 重放（同 body_hash → 返回原 job）或冲突（不同 body_hash → 409）。
async fn replay_or_conflict(
    conn: &mut SqliteConnection,
    record: manual_core::domain::IdempotencyRecord,
    body_hash: &str,
) -> Result<JobCreation, GenerationError> {
    if record.body_hash != body_hash {
        return Err(GenerationError::IdempotencyConflict {
            message: "该 Idempotency-Key 已用于不同的请求内容：请使用新的键，或链接已存在的任务"
                .to_owned(),
            details: serde_json::json!({
                "reason": "idempotencyKeyReused",
                "existingResourceId": record.resource_id,
            }),
        });
    }
    let job_id = record.resource_id.clone().ok_or_else(|| {
        GenerationError::Storage(StorageError::Database {
            detail: "幂等记录缺少 resource_id".to_owned(),
        })
    })?;
    let job = jobs_repo::get(conn, &job_id).await?.ok_or_else(|| {
        GenerationError::Storage(StorageError::Database {
            detail: format!("幂等记录指向的 job 不存在：{job_id}"),
        })
    })?;
    let snapshot = snapshots::get(conn, &job.snapshot_id)
        .await?
        .ok_or_else(|| {
            GenerationError::Storage(StorageError::Database {
                detail: format!("job 的快照不存在：{}", job.snapshot_id),
            })
        })?;
    let reservations = ledger::list_for_snapshot(conn, &snapshot.id).await?;
    Ok(JobCreation {
        job,
        snapshot,
        reservations,
        replayed: true,
    })
}

fn serialization(error: serde_json::Error) -> CreateTransactionError {
    CreateTransactionError::Generation(GenerationError::Storage(StorageError::Database {
        detail: format!("序列化失败：{error}"),
    }))
}
