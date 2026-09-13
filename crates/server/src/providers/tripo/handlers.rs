//! Tripo 阶段处理器（`tripo_upload` / `tripo_submit` / `tripo_poll`）。
//!
//! 与执行器的分工（contracts.md §5；T10 的语义**不改动**）：
//! - 处理器只负责"做什么"（读快照、发 HTTP、保存**事实**）；
//! - 业务状态推进（`waiting_provider` / `retry_wait` / `submission_unknown` / …）由执行器
//!   带租约 epoch guard 完成；处理器返回 [`StageOutcome`] 表达结论；
//! - 付费提交走 [`StageContext::submission`] 的提交窗口：先 intent、再 submitting、
//!   再事实观察；**没有**任何自动重试层，未拿到 task ID 的结果一律
//!   `submission_unknown`（绝不自动重购，ADR-006）。
//!
//! 阶段行为（AC-041 / T13 的 AC-043、AC-044）：
//!
//! | 阶段 | 输入 | 输出/结论 |
//! | --- | --- | --- |
//! | `tripo_upload` | 快照照片（内容哈希 → blob 文件） | 每张图 `POST /files`（`file` 字段）→ token；**按内容哈希缓存**（同一内容不重复上传） |
//! | `tripo_submit` | 快照 `provider_config.tripo` + 已确认发送范围的视图 + upload token | `POST /generation/multiview-to-model`；`code==0` 且带 `task_id` → 事实观察 + `succeeded`；429 → 可重试；业务错误/3xx → `failed`（明确未计费，释放预留）；传输/5xx/缺 task_id → `submission_unknown`（保留预留） |
//! | `tripo_poll` | 同一 job `tripo_submit` 的 accepted 事实（task ID） | `GET /tasks/{id}`；原始状态与归一化状态都保存；`success` 必须带 `model_url` 才算成功；未知枚举保留原值并继续等待；查询失败 ≠ 生成失败（可退避重试）；`credits_consumed` 精确换算为 `creditMinor` 并按实际结算。**临时/签名 URL 不落库**：`modelUrl`/`renderedImageUrl` 只保存 sha256 摘要 + host（[`crate::redaction`]），链接本体进 [`EphemeralLinks`] |
//! | `model_download` | `tripo_poll` 事实中的 `remoteTaskId`（+ 进程内易失链接） | **独立无凭据 client**（不转发 bearer）、HTTPS + 允许域 + 每跳重定向与实际连接 IP 校验（pin IP 保留 SNI）、流式大小与 sha256 → 内容寻址落盘 + asset（`purpose=model`）；链接过期或易失缓存未命中（重启/恢复）→ **重新查询已知任务取新链接**（绝不重新提交付费请求）；中断可安全重试（整文件重下） |
//! | `model_validate` | 下载阶段的结果资产 + GLB 预算 | GLB 结构/内嵌资源/扩展/索引/有限坐标/面数与贴图预算（见 `assets::glb`）；通过 → 不可变 `model_revision`（`validated`）；超预算或结构问题 → `needs_input` + `rejected` revision（**原始模型与错误都保留**，不静默改坏模型、不自动降预算） |
//!
//! 不实现单图 `image-to-model` 分支：首版产品路径是"多视图"（PRD REQ-027；
//! architecture §5.3），单图模式需要新的产品决定（可否用 AI 造图/单张实物照片），
//! 本卡不擅自扩大发送范围。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use manual_core::domain::{
    AssetPurpose, CostLedgerEntry, ModelValidationState, ProviderKey, StageKind,
};
use manual_core::generation::sha256_hex;
use manual_core::timestamps::Timestamp;

use crate::assets::glb::{self, DownloadError, GlbBudget, ModelDownloader};
use crate::assets::upload::sanitize_original_name;
use crate::config::Settings;
use crate::generation::estimate::quote_payload;
use crate::generation::ledger as generation_ledger;
use crate::jobs::submission::record_result_fact;
use crate::jobs::{
    JobError, MissingItem, StageContext, StageFuture, StageHandler, StageOutcome, StageRegistry,
};
use crate::storage::repo;

use super::client::{
    TRIPO_SUBMIT_PATH, TRIPO_TASKS_PATH_PREFIX, TRIPO_UPLOAD_PATH, TripoClient, TripoError,
    TripoTimeouts,
};
use super::dto::{BillingFact, SubmitParameters, SubmitRequest, TaskData, ViewInput};
use super::links::EphemeralLinks;

/// 多视图允许的方向（`detail` 不进入多视图请求；顺序 = 槽位顺序）。
pub const MULTIVIEW_ORDER: [&str; 4] = ["front", "left", "back", "right"];

/// 单张照片允许的内容类型（首版只接受 JPEG/PNG：避免不同接口支持格式差异）。
pub const ALLOWED_IMAGE_MIMES: [&str; 2] = ["image/jpeg", "image/png"];

// ---------------------------------------------------------------------------
// 处理器集合（按配置注册）
// ---------------------------------------------------------------------------

/// Tripo 阶段处理器集合（`serve` 按配置注册；未配置时不注册任何处理器，
/// 已入队阶段被延后而不是假成功）。
pub struct TripoHandlers {
    client: Arc<TripoClient>,
    data_dir: PathBuf,
    downloader: Arc<ModelDownloader>,
    budget: GlbBudget,
    /// `tripo_poll` → `model_download` 的**易失**临时下载链接（T20/BUG-008：
    /// 签名 URL 不落库；进程内共享，重启后下载阶段按 task ID 重新查询）。
    links: Arc<EphemeralLinks>,
}

impl TripoHandlers {
    /// 从配置构造（要求 `providers.tripo` 已配置：api_key + model 在场）。
    pub fn from_settings(settings: &Settings) -> Result<Self, String> {
        let provider = &settings.providers.tripo;
        let api_key = provider
            .api_key
            .clone()
            .ok_or_else(|| "providers.tripo.api_key 未配置（生成不可用）".to_owned())?;
        let client = TripoClient::new(&provider.base_url, api_key, TripoTimeouts::default())?;
        let downloader = ModelDownloader::from_settings(settings);
        if downloader.policy().allow_local_fixture && !downloader.policy().local_fixture_allowed() {
            // 生产构建：该键不生效（测试构建开关未开启），明确告警而不是静默忽略。
            tracing::warn!(
                event = "download_local_fixture_ignored",
                "download.allow_local_fixture 已配置，但当前不是测试构建：本机 fixture 放行不生效\
                 （生产构建不放行明文 http 与回环地址）"
            );
        }
        Ok(Self {
            client: Arc::new(client),
            data_dir: settings.data_dir.clone(),
            downloader: Arc::new(downloader),
            budget: GlbBudget::default(),
            links: Arc::new(EphemeralLinks::new()),
        })
    }

    /// 测试注入：替换下载器与 GLB 预算（生产路径只用 [`Self::from_settings`]）。
    pub fn with_download(mut self, downloader: ModelDownloader, budget: GlbBudget) -> Self {
        self.downloader = Arc::new(downloader);
        self.budget = budget;
        self
    }

    /// 生效的 base_url（脱敏展示；不含密钥）。
    pub fn base_url(&self) -> &str {
        self.client.base_url()
    }

    /// 注册五个阶段处理器（`tripo_upload` / `tripo_submit` / `tripo_poll` /
    /// `model_download` / `model_validate`）。
    pub fn register(&self, registry: &mut StageRegistry) {
        registry.register(
            StageKind::TripoUpload,
            TripoUploadHandler::new(Arc::clone(&self.client), self.data_dir.clone()),
        );
        registry.register(
            StageKind::TripoSubmit,
            TripoSubmitHandler::new(Arc::clone(&self.client)),
        );
        registry.register(
            StageKind::TripoPoll,
            TripoPollHandler::new(Arc::clone(&self.client), Arc::clone(&self.links)),
        );
        registry.register(
            StageKind::ModelDownload,
            TripoModelDownloadHandler::new(
                Arc::clone(&self.client),
                Arc::clone(&self.downloader),
                Arc::clone(&self.links),
            ),
        );
        registry.register(
            StageKind::ModelValidate,
            TripoModelValidateHandler::new(self.data_dir.clone(), self.budget),
        );
    }
}

// ---------------------------------------------------------------------------
// 内部：输入加载与结论构造
// ---------------------------------------------------------------------------

/// 处理器内部的"立即结论"：输入/协议问题直接给出 StageOutcome，不再继续。
struct HandlerStop(StageOutcome);

impl From<StageOutcome> for HandlerStop {
    fn from(outcome: StageOutcome) -> Self {
        Self(outcome)
    }
}

type Loaded<T> = Result<T, HandlerStop>;

fn needs_input(code: &str, message: impl Into<String>) -> HandlerStop {
    HandlerStop(StageOutcome::NeedsInput {
        items: vec![MissingItem::new(code, message)],
    })
}

/// 已确认的发送范围（一个视图一张照片）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfirmedView {
    view: String,
    photo_id: String,
    sha256: String,
}

/// 执行阶段所需的冻结输入（快照参数 + 已确认发送范围）。
#[derive(Debug, Clone)]
struct ConfirmedInputs {
    parameters: SubmitParameters,
    views: Vec<ConfirmedView>,
}

impl ConfirmedInputs {
    fn has_front(&self) -> bool {
        self.views.iter().any(|view| view.view == "front")
    }
}

/// 读取冻结输入：
/// - 快照的 `provider_config.tripo`（生成参数；**不**替换为当前配置/默认值）；
/// - 已确认的发送范围（报价的 `sendScope.tripo.views`，REQ-021 用户确认过的视图集合），
///   并与快照的 `photo_ids`/`photo_hashes` 交叉核对（顺序与内容必须一一对应）。
async fn load_confirmed_inputs(ctx: &StageContext) -> Loaded<ConfirmedInputs> {
    let mut conn = ctx.pool.acquire().await.map_err(internal)?;
    let snapshot = repo::snapshots::get(&mut conn, &ctx.job.snapshot_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            needs_input(
                "generation_snapshot_missing",
                "任务快照不存在：无法确认冻结输入（不猜测、不重新购买）",
            )
        })?;
    let parameters =
        SubmitParameters::from_provider_config(&snapshot.provider_config).map_err(|detail| {
            needs_input(
                "provider_config_invalid",
                format!("快照中的供应商参数不可用：{detail}"),
            )
        })?;

    let quote_id = snapshot
        .budgets
        .get("quoteId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            needs_input(
                "tripo_send_scope_missing",
                "快照缺少 quoteId：无法回读用户已确认的发送范围（不猜测发送内容）",
            )
        })?;
    let quote = repo::quotes::get(&mut conn, &quote_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            needs_input(
                "tripo_send_scope_missing",
                "报价记录不存在：无法回读用户已确认的发送范围（不猜测发送内容）",
            )
        })?;
    let payload = quote_payload(&quote).await.map_err(|error| {
        needs_input(
            "tripo_send_scope_missing",
            format!("报价载荷无法回读：{error:?}（不猜测发送内容）"),
        )
    })?;

    let views: Vec<ConfirmedView> = payload
        .send_scope
        .tripo
        .views
        .iter()
        .map(|scope| ConfirmedView {
            view: scope.view.clone(),
            photo_id: scope.photo_id.clone(),
            sha256: scope.sha256.clone(),
        })
        .collect();
    if let Some(unknown_view) = views
        .iter()
        .find(|view| !MULTIVIEW_ORDER.contains(&view.view.as_str()))
    {
        return Err(needs_input(
            "unsupported_view",
            format!(
                "已确认发送范围里有非多视图方向 {}：只允许 front/left/back/right",
                unknown_view.view
            ),
        ));
    }

    // 与快照交叉核对（一一对应且顺序一致）：发送范围必须就是冻结的内容。
    let snapshot_ids: Vec<String> = serde_json::from_value(snapshot.photo_ids.clone())
        .map_err(|_| needs_input("snapshot_invalid", "快照 photo_ids 不是字符串数组"))?;
    let snapshot_hashes: Vec<String> = serde_json::from_value(snapshot.photo_hashes.clone())
        .map_err(|_| needs_input("snapshot_invalid", "快照 photo_hashes 不是字符串数组"))?;
    let views_ids: Vec<String> = views.iter().map(|view| view.photo_id.clone()).collect();
    let views_hashes: Vec<String> = views.iter().map(|view| view.sha256.clone()).collect();
    if snapshot_ids != views_ids || snapshot_hashes != views_hashes {
        return Err(needs_input(
            "tripo_send_scope_mismatch",
            "快照与已确认发送范围不一致（照片集合或内容哈希）：拒绝按不一致的内容发起付费请求",
        ));
    }

    Ok(ConfirmedInputs { parameters, views })
}

/// 读取本 job `tripo_upload` 阶段保存的上传事实（token 按内容哈希索引）。
async fn load_uploads(ctx: &StageContext) -> Loaded<Vec<UploadFact>> {
    let mut conn = ctx.pool.acquire().await.map_err(internal)?;
    let stages = repo::job_stages::list_for_job(&mut conn, &ctx.job.id)
        .await
        .map_err(internal)?;
    let upload_stage = stages
        .iter()
        .find(|stage| stage.stage_kind == StageKind::TripoUpload)
        .ok_or_else(|| {
            needs_input(
                "tripo_upload_missing",
                "找不到 tripo_upload 阶段：无法取得图片 token（不重新上传、不猜测）",
            )
        })?;
    let uploads = parse_upload_facts(upload_stage.usage_json.as_ref());
    if uploads.is_empty() {
        return Err(needs_input(
            "tripo_upload_tokens_missing",
            "tripo_upload 阶段没有可用的上传 token：请先完成上传阶段（不猜测 token）",
        ));
    }
    Ok(uploads)
}

/// 内部错误（数据库/执行器级）：交给执行器归一化（未决尝试 → submission_unknown）。
fn internal<E: Into<JobError>>(error: E) -> HandlerStop {
    HandlerStop(StageOutcome::Failed {
        reason: format!("内部错误：{}", error.into()),
    })
}

/// 观察写入前的租约检查：本 worker 是否仍是该阶段的当前租约持有者。
///
/// 合同允许**过期 worker 保存不可变事实**（receipt/结果），但本模块写入的是
/// "最近一次远端观察"——它是**可被覆盖**的快照（与一次性 receipt 不同）：若过期 worker
/// 的迟到响应覆盖了新结果，可能把 `modelUrl`／`billing` 等新事实抹掉。这里做一次
/// "仍是当前 `lease_epoch`"的乐观检查，把该窗口压到最小（`ctx.stage.lease_epoch` 是
/// 本次领取时的 epoch；被接管即变化）。
///
/// 不是硬保证（读-写之间仍有理论竞态）——硬保证需要在 storage 层增加"仅当前 epoch 可写"
/// 的条件写入原语，超出本卡允许改动的文件范围；这里选择**宁可不写**（读失败/阶段消失时返回
/// `false`），成功路径的最终事实仍由执行器按既有语义写入。
/// `pub` 是为了让集成测试能直接验证"错 epoch 不写"的分支（其余调用者都在本模块内）。
pub async fn is_current_lease_holder(pool: &sqlx::SqlitePool, stage_id: &str, epoch: i64) -> bool {
    let Ok(mut conn) = pool.acquire().await else {
        return false;
    };
    match repo::job_stages::get(&mut conn, stage_id).await {
        Ok(Some(stage)) => stage.lease_epoch == epoch,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// 上传事实（内容哈希 → token）
// ---------------------------------------------------------------------------

/// 一张已上传图片的事实（缓存键 = 内容哈希）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct UploadFact {
    view: String,
    sha256: String,
    token: String,
    token_field: String,
}

/// 解析阶段 `usage_json` 里的上传事实；损坏时按"没有"处理（重新上传是免费且幂等的）。
fn parse_upload_facts(usage: Option<&Value>) -> Vec<UploadFact> {
    let Some(uploads) = usage
        .and_then(|usage| usage.get("uploads"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    uploads
        .iter()
        .filter_map(|entry| {
            let view = entry.get("view")?.as_str()?.to_owned();
            let sha256 = entry.get("sha256")?.as_str()?.to_owned();
            let token = entry.get("token")?.as_str()?.to_owned();
            let token_field = entry
                .get("tokenField")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            Some(UploadFact {
                view,
                sha256,
                token,
                token_field,
            })
        })
        .collect()
}

/// 上传事实 → `usage_json`（稳定字段序；不含密钥）。
fn upload_facts_usage(uploads: &[UploadFact]) -> Value {
    json!({
        "uploads": uploads
            .iter()
            .map(|upload| json!({
                "view": upload.view,
                "sha256": upload.sha256,
                "token": upload.token,
                "tokenField": upload.token_field,
            }))
            .collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// tripo_upload
// ---------------------------------------------------------------------------

/// `tripo_upload`：把冻结快照中的照片上传到 `POST /files`（multipart 字段 `file`）。
///
/// 上传不是付费操作（credits 由生成计费）：传输失败/5xx/429 可安全重试；
/// 供应商业务拒绝（4xx/业务 code）是确定性失败。
pub struct TripoUploadHandler {
    client: Arc<TripoClient>,
    data_dir: PathBuf,
}

impl TripoUploadHandler {
    /// 构造（生产路径由 [`TripoHandlers::register`] 调用；测试可直接构造）。
    pub fn new(client: Arc<TripoClient>, data_dir: PathBuf) -> Self {
        Self { client, data_dir }
    }

    /// 读取 blob 内容（内容寻址；只接受 JPEG/PNG；缺失/隔离 → needs_input）。
    async fn read_photo(&self, ctx: &StageContext, sha256: &str) -> Loaded<(String, Vec<u8>)> {
        let mut conn = ctx.pool.acquire().await.map_err(internal)?;
        let blob = repo::blobs::get(&mut conn, sha256)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                needs_input(
                    "photo_blob_missing",
                    format!(
                        "照片内容（{}…）不在库中：请重新上传照片后重试",
                        &sha256[..8.min(sha256.len())]
                    ),
                )
            })?;
        if blob.storage_state != manual_core::domain::BlobStorageState::Stored {
            return Err(needs_input(
                "photo_blob_unavailable",
                format!(
                    "照片内容（{}…）状态异常（{:?}）：不可用于生成，请核对资料完整性",
                    &sha256[..8.min(sha256.len())],
                    blob.storage_state
                ),
            ));
        }
        let mime = blob.mime.to_ascii_lowercase();
        if !ALLOWED_IMAGE_MIMES.contains(&mime.as_str()) {
            return Err(needs_input(
                "unsupported_image_type",
                format!("只接受 JPEG/PNG 照片（实际 {mime}）：请替换照片后重试"),
            ));
        }
        let path = crate::assets::blob_path(&self.data_dir, sha256);
        let bytes = tokio::fs::read(&path).await.map_err(|error| {
            needs_input(
                "photo_blob_missing",
                format!(
                    "照片内容文件不可读（{}…）：{error}；请核对 data-dir 完整性",
                    &sha256[..8.min(sha256.len())]
                ),
            )
        })?;
        // magic 校验（第二道防线：库里的 mime 也来自上传时的校验）。
        let magic_matches = match mime.as_str() {
            "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
            "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            _ => false,
        };
        if !magic_matches {
            return Err(needs_input(
                "unsupported_image_type",
                "照片字节与声明的类型不符（magic 校验失败）：请重新上传照片",
            ));
        }
        Ok((mime, bytes))
    }
}

/// 便捷：文件扩展名（multipart 文件名与内容类型一致）。
fn file_extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        _ => "jpg",
    }
}

impl StageHandler for TripoUploadHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            let confirmed = match load_confirmed_inputs(ctx).await {
                Ok(inputs) => inputs,
                Err(stop) => return Ok(stop.0),
            };
            if !confirmed.has_front() {
                return Ok(needs_input(
                    "missing_front_view",
                    "缺少 front（正面）照片：请补齐实物照片后重试（不用 AI 造图代替）",
                )
                .0);
            }
            if confirmed.views.len() < 2 {
                return Ok(needs_input(
                    "insufficient_views",
                    "多视图至少需要两张真实输入（front + 至少一个侧视图）：请补齐后重试",
                )
                .0);
            }

            // 上传事实按内容哈希缓存：已在 usage 里出现的内容不重复上传。
            let mut uploads = parse_upload_facts(ctx.stage.usage_json.as_ref());
            for view in &confirmed.views {
                if uploads.iter().any(|upload| upload.sha256 == view.sha256) {
                    continue;
                }
                let (mime, bytes) = match self.read_photo(ctx, &view.sha256).await {
                    Ok(photo) => photo,
                    Err(stop) => return Ok(stop.0),
                };
                let file_name = format!("{}.{}", view.view, file_extension(&mime));
                match self.client.upload_image(&file_name, &mime, bytes).await {
                    Ok(data) => {
                        uploads.push(UploadFact {
                            view: view.view.clone(),
                            sha256: view.sha256.clone(),
                            token: data.token,
                            token_field: data.field.to_owned(),
                        });
                        // 每张图上传完立即保存事实（崩溃后不重复上传已成功的内容）；
                        // 只在仍是当前租约持有者时写入（避免迟到观察覆盖新结果）。
                        if is_current_lease_holder(&ctx.pool, &ctx.stage.id, ctx.stage.lease_epoch)
                            .await
                        {
                            record_result_fact(
                                &ctx.pool,
                                &ctx.stage.id,
                                None,
                                Some(&upload_facts_usage(&uploads).to_string()),
                                ctx.now,
                            )
                            .await
                            .map_err(JobError::from)?;
                        } else {
                            tracing::warn!(
                                event = "tripo_upload_fact_skipped_stale_lease",
                                jobId = %ctx.job.id,
                                stageId = %ctx.stage.id,
                                "租约已被接管：跳过本次上传事实写入（不覆盖新结果）"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            event = "tripo_upload_failed",
                            jobId = %ctx.job.id,
                            stageId = %ctx.stage.id,
                            view = %view.view,
                            errorCode = error.code(),
                            detail = %error.redacted(),
                            "图片上传失败"
                        );
                        return Ok(upload_failure_outcome(&error));
                    }
                }
            }
            Ok(StageOutcome::Succeeded {
                result_asset_id: None,
                usage: Some(upload_facts_usage(&uploads)),
            })
        })
    }
}

/// 上传失败的结论：付费风险为零（上传免费），协议/传输问题可退避重试；
/// 供应商业务拒绝（格式/权限）是确定性失败。
fn upload_failure_outcome(error: &TripoError) -> StageOutcome {
    match error {
        TripoError::Business { .. } | TripoError::Redirected { .. } => StageOutcome::Failed {
            reason: error.redacted(),
        },
        TripoError::RateLimited {
            retry_after_seconds,
        } => StageOutcome::Retryable {
            reason: error.redacted(),
            retry_after_seconds: *retry_after_seconds,
        },
        TripoError::Transport { .. }
        | TripoError::ServerError { .. }
        | TripoError::Unexpected { .. } => StageOutcome::Retryable {
            reason: format!("{}：可安全重试（上传不产生费用）", error.redacted()),
            retry_after_seconds: None,
        },
    }
}

// ---------------------------------------------------------------------------
// tripo_submit（付费）
// ---------------------------------------------------------------------------

/// `tripo_submit`：`POST /generation/multiview-to-model`（**付费**）。
pub struct TripoSubmitHandler {
    client: Arc<TripoClient>,
}

impl TripoSubmitHandler {
    pub fn new(client: Arc<TripoClient>) -> Self {
        Self { client }
    }
}

impl StageHandler for TripoSubmitHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            // 1) 已有远端事实（崩溃恢复：receipt 已落库、状态未推进）：不重发。
            if let Some(task_id) = ctx.known_remote_task_id().map(str::to_owned) {
                return Ok(StageOutcome::Succeeded {
                    result_asset_id: None,
                    usage: Some(json!({ "remoteTaskId": task_id, "resumed": true })),
                });
            }
            // 2) 防御：accepted 但没有远端 ID 的尝试是"无法对账"的形态，
            //    绝不能开新的一次付费提交（executor 会给 Fresh，这里显式拦住）。
            if let Some(attempt) = &ctx.attempt
                && attempt.submit_state == manual_core::domain::SubmitState::Accepted
                && attempt.remote_task_id.is_none()
            {
                return Ok(StageOutcome::SubmissionUnknown {
                    reason:
                        "该阶段已有 accepted 尝试但没有远端 task ID：需人工对账，不重新提交付费请求"
                            .to_owned(),
                });
            }

            let confirmed = match load_confirmed_inputs(ctx).await {
                Ok(inputs) => inputs,
                Err(stop) => return Ok(stop.0),
            };
            let uploads = match load_uploads(ctx).await {
                Ok(uploads) => uploads,
                Err(stop) => return Ok(stop.0),
            };

            // 3) 组装请求：缺失方向直接不提交对应对象；front 必须存在；至少两张真实输入。
            let views: Vec<ViewInput> = confirmed
                .views
                .iter()
                .filter_map(|view| {
                    uploads
                        .iter()
                        .find(|upload| upload.sha256 == view.sha256)
                        .map(|upload| ViewInput {
                            view: view.view.clone(),
                            token: upload.token.clone(),
                        })
                })
                .collect();
            if !views.iter().any(|view| view.view == "front") {
                return Ok(needs_input(
                    "missing_front_view",
                    "缺少 front（正面）输入：请补齐照片后重试（不用 AI 造图代替实物照片）",
                )
                .0);
            }
            if views.len() < 2 {
                return Ok(needs_input(
                    "insufficient_views",
                    "多视图至少需要两张真实输入（front + 至少一个侧视图）：缺失方向不会提交，请补齐后重试",
                )
                .0);
            }
            let request = SubmitRequest::new(&confirmed.parameters, &views);
            let body = request.to_bytes();
            let request_hash = sha256_hex(&body);

            // 4) 提交窗口：先 intent、再 submitting、再发请求（contracts §5）。
            //    先把账本联动需要的值取出来（`window` 会长期借用 ctx.submission）。
            let pool = ctx.pool.clone();
            let snapshot_id = ctx.job.snapshot_id.clone();
            let job_id = ctx.job.id.clone();
            let stage_id = ctx.stage.id.clone();
            let now = ctx.now;
            let window = &mut ctx.submission;
            window.begin_intent(&request_hash).await?;
            window.mark_submitting().await?;
            tracing::info!(
                event = "tripo_submit_sent",
                jobId = %job_id,
                stageId = %stage_id,
                views = views.len(),
                model = %confirmed.parameters.model,
                endpoint = TRIPO_SUBMIT_PATH,
                "付费提交已发出（单次请求；未返回 task ID 一律按结果未知处理）"
            );

            match self.client.submit_multiview(&body).await {
                Ok(data) => {
                    let task_id = data.task_id;
                    // 事实观察：即使租约刚过期也允许补写空 remote_task_id。
                    match window.record_remote_task_id(&task_id).await? {
                        crate::jobs::RemoteTaskObservation::Conflict { existing } => {
                            // 执行器会强制落 submission_unknown；这里保持一致的结论。
                            Ok(StageOutcome::SubmissionUnknown {
                                reason: format!(
                                    "远端 task ID 冲突（已有 {existing}）：不覆盖、不自动重购，等待对账"
                                ),
                            })
                        }
                        _ => {
                            tracing::info!(
                                event = "tripo_submit_accepted",
                                jobId = %job_id,
                                stageId = %stage_id,
                                remoteTaskId = %task_id,
                                "远端任务已创建（task ID 已持久化）"
                            );
                            Ok(StageOutcome::Succeeded {
                                result_asset_id: None,
                                usage: Some(json!({
                                    "remoteTaskId": task_id,
                                    "requestHash": request_hash,
                                    "endpoint": TRIPO_SUBMIT_PATH,
                                })),
                            })
                        }
                    }
                }
                Err(error) => {
                    let detail = error.redacted();
                    match &error {
                        // 明确可重试：429（可证明未被接受；尊重 Retry-After）。
                        TripoError::RateLimited { .. } => {
                            window.mark_failed(&detail).await?;
                            tracing::warn!(
                                event = "tripo_submit_rate_limited",
                                jobId = %job_id,
                                stageId = %stage_id,
                                "付费提交被限速：可证明未被接受，按退避重试（预留继续保留）"
                            );
                            Ok(StageOutcome::Retryable {
                                reason: detail,
                                retry_after_seconds: error.retry_after_seconds(),
                            })
                        }
                        // 明确拒绝（业务错误 / 4xx / 3xx）：确定性失败，释放预留（明确未计费）。
                        _ if error.is_definitively_refused() => {
                            window.mark_failed(&detail).await?;
                            let ledger_note =
                                release_tripo_reservation(&pool, &snapshot_id, &now).await;
                            tracing::warn!(
                                event = "tripo_submit_refused",
                                jobId = %job_id,
                                stageId = %stage_id,
                                detail = %detail,
                                ledger = %ledger_note,
                                "付费提交被供应商明确拒绝（无 task ID 产生）"
                            );
                            Ok(StageOutcome::Failed { reason: detail })
                        }
                        // 传输失败 / 5xx / 缺少 task ID：**不能证明未被接受** → 结果未知。
                        _ => {
                            window.mark_unknown(&detail).await?;
                            let attempt_id = window.attempt_id().map(str::to_owned);
                            let ledger_note = mark_tripo_reservation_unknown(
                                &pool,
                                &snapshot_id,
                                attempt_id.as_deref(),
                                &now,
                            )
                            .await;
                            tracing::error!(
                                event = "tripo_submit_submission_unknown",
                                jobId = %job_id,
                                stageId = %stage_id,
                                detail = %detail,
                                ledger = %ledger_note,
                                "付费提交结果未知：暂停该分支后续购买，等待管理员对账"
                            );
                            Ok(StageOutcome::SubmissionUnknown { reason: detail })
                        }
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// tripo_poll
// ---------------------------------------------------------------------------

/// `tripo_poll`：`GET /tasks/{task_id}`（查询远端任务；`task ID` 来自 `tripo_submit`
/// 的 accepted 事实，**绝不重新提交付费请求**）。
pub struct TripoPollHandler {
    client: Arc<TripoClient>,
    links: Arc<EphemeralLinks>,
}

impl TripoPollHandler {
    pub fn new(client: Arc<TripoClient>, links: Arc<EphemeralLinks>) -> Self {
        Self { client, links }
    }
}

impl StageHandler for TripoPollHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            let task_id = match ctx.known_remote_task_id().map(str::to_owned) {
                Some(task_id) => Some(task_id),
                None => {
                    let mut conn = ctx.pool.acquire().await?;
                    repo::attempts::latest_accepted_for_job(
                        &mut conn,
                        &ctx.job.id,
                        StageKind::TripoSubmit,
                    )
                    .await?
                    .and_then(|attempt| attempt.remote_task_id)
                }
            };
            let Some(task_id) = task_id else {
                // 上游事实缺失（依赖已满足却找不到 task ID）：完整性问题，
                // 不重新提交付费请求，交人工核对。
                return Ok(needs_input(
                    "tripo_task_id_missing",
                    "查询阶段找不到已提交的远端 task ID：请人工核对（不重新提交付费请求）",
                )
                .0);
            };

            match self.client.get_task(&task_id).await {
                Ok(task) => {
                    let normalized = super::status::NormalizedStatus::new(&task.status_raw);
                    let usage = task_usage(&task_id, &task, normalized.state());
                    // 事实：无论结论如何都保存原始状态与归一化状态（可诊断）。
                    // 只在仍是当前租约持有者时写入：这是**可覆盖的观察快照**，迟到的
                    // 旧响应不得把新结果（如 success 的 modelUrl/billing）抹掉。
                    if is_current_lease_holder(&ctx.pool, &ctx.stage.id, ctx.stage.lease_epoch)
                        .await
                    {
                        // 签名 URL 是**易失能力**，不落库（AC-010；见 `crate::redaction`）：
                        // 只交给进程内的 `model_download`（重启后由下载阶段按 task ID 重查）。
                        // 与事实写入同一（租约）条件：迟到的旧观察不覆盖更新的链接。
                        if let Some(model_url) = task.model_url.as_deref() {
                            self.links
                                .remember(&ctx.job.id, &task_id, model_url, ctx.now);
                        }
                        record_result_fact(
                            &ctx.pool,
                            &ctx.stage.id,
                            None,
                            Some(&usage.to_string()),
                            ctx.now,
                        )
                        .await?;
                    } else {
                        tracing::warn!(
                            event = "tripo_poll_fact_skipped_stale_lease",
                            jobId = %ctx.job.id,
                            stageId = %ctx.stage.id,
                            remoteTaskId = %task_id,
                            "租约已被接管：跳过本次查询观察写入（不覆盖新结果）"
                        );
                    }
                    if normalized.is_unrecognized() {
                        tracing::warn!(
                            event = "tripo_task_status_unrecognized",
                            jobId = %ctx.job.id,
                            stageId = %ctx.stage.id,
                            remoteTaskId = %task_id,
                            rawStatus = %normalized.raw(),
                            "远端返回未知状态：保留原值并继续等待（不猜测语义）"
                        );
                    }

                    match normalized.state() {
                        super::status::TripoState::Success => {
                            let Some(model_url) = task.model_url.as_deref() else {
                                // success 但缺可下载模型：不算成功（保留 task_id 继续查询）。
                                return Ok(StageOutcome::Retryable {
                                    reason: "远端任务 success 但缺少可下载模型 URL（output.model_url）：\
                                             不组装成功，保留 task_id 继续查询"
                                        .to_owned(),
                                    retry_after_seconds: None,
                                });
                            };
                            let ledger_note = settle_tripo_reservation(
                                &ctx.pool.clone(),
                                &ctx.job.snapshot_id.clone(),
                                task.billing.as_ref(),
                                &ctx.now,
                            )
                            .await;
                            tracing::info!(
                                event = "tripo_task_success",
                                jobId = %ctx.job.id,
                                stageId = %ctx.stage.id,
                                remoteTaskId = %task_id,
                                modelUrl = %crate::redaction::url_summary(model_url).to_label(),
                                ledger = %ledger_note,
                                "远端任务完成（链接摘要已保存为阶段事实、链接本体只留在进程内；\
                                 下载属 model_download 阶段）"
                            );
                            Ok(StageOutcome::Succeeded {
                                result_asset_id: None,
                                usage: Some(usage),
                            })
                        }
                        super::status::TripoState::Queued
                        | super::status::TripoState::Running
                        | super::status::TripoState::Unrecognized => Ok(StageOutcome::WaitingProvider),
                        super::status::TripoState::Failed => Ok(StageOutcome::Failed {
                            reason: format!("远端任务失败（status={}）：保留 task ID，可由人工对账", normalized.raw()),
                        }),
                        super::status::TripoState::Cancelled => Ok(StageOutcome::Failed {
                            reason: format!("远端任务已取消（status={}）：保留 task ID", normalized.raw()),
                        }),
                        super::status::TripoState::Banned => Ok(StageOutcome::Failed {
                            reason: "远端任务被封禁（status=banned）：产物不可用，需人工核对后重新生成（需新的预算确认）"
                                .to_owned(),
                        }),
                        super::status::TripoState::Expired => Ok(StageOutcome::Failed {
                            reason: "远端任务产物已过期（status=expired）：产物不可找回，需人工核对后重新生成（需新的预算确认）"
                                .to_owned(),
                        }),
                    }
                }
                // 查询失败 ≠ 生成失败：任务已购买、task ID 已保留，按退避重试（有上限）。
                Err(error) => {
                    let retry_after = error.retry_after_seconds();
                    let reason = match &error {
                        TripoError::Redirected { .. } => {
                            return Ok(StageOutcome::Failed {
                                reason: error.redacted(),
                            });
                        }
                        _ => format!("查询远端任务失败（不改变购买事实）：{}", error.redacted()),
                    };
                    tracing::warn!(
                        event = "tripo_poll_failed",
                        jobId = %ctx.job.id,
                        stageId = %ctx.stage.id,
                        remoteTaskId = %task_id,
                        errorCode = error.code(),
                        detail = %error.redacted(),
                        "远端任务查询失败：保留 task ID，按退避重试"
                    );
                    Ok(StageOutcome::Retryable {
                        reason,
                        retry_after_seconds: retry_after,
                    })
                }
            }
        })
    }
}

/// 阶段用量/事实 JSON（原始状态 + 归一化状态 + 链接摘要 + 计费；不含密钥与临时 URL）。
///
/// `modelUrl` 是**带签名的临时下载地址**：这里只保存它的 host 与 sha256 摘要
/// （`{"redacted":true,"host":…,"sha256":…}`，见 [`crate::redaction`]），
/// 链接本体只交给进程内的 [`EphemeralLinks`]（下载阶段用完即弃）。
/// 出网（HTTP DTO）路径另有一层兜底脱敏，防止历史数据回显。
fn task_usage(
    task_id: &str,
    task: &super::dto::TaskData,
    state: super::status::TripoState,
) -> Value {
    let mut usage = Map::new();
    usage.insert("remoteTaskId".to_owned(), json!(task_id));
    usage.insert("rawStatus".to_owned(), json!(task.status_raw));
    usage.insert("normalizedStatus".to_owned(), json!(state.as_str()));
    if let Some(progress) = &task.progress_raw {
        usage.insert("progress".to_owned(), json!(progress));
    }
    if let Some(model_url) = &task.model_url {
        usage.insert(
            "modelUrl".to_owned(),
            crate::redaction::url_summary(model_url).to_value(),
        );
    }
    if let Some(rendered) = &task.rendered_image_url {
        usage.insert(
            "renderedImageUrl".to_owned(),
            crate::redaction::url_summary(rendered).to_value(),
        );
    }
    if let Some(billing) = &task.billing {
        usage.insert("billing".to_owned(), billing_value(billing));
    }
    if let Some(problem) = &task.billing_problem {
        usage.insert("billingProblem".to_owned(), json!(problem));
    }
    usage.insert("dataKeys".to_owned(), json!(task.data_keys));
    Value::Object(usage)
}

fn billing_value(billing: &BillingFact) -> Value {
    json!({
        "sourceField": billing.source_field,
        "literal": billing.literal,
        "creditMinor": billing.credit_minor,
        "currency": "credit_minor",
    })
}

// ---------------------------------------------------------------------------
// model_download（T13 / AC-043）
// ---------------------------------------------------------------------------

/// 下载阶段需要的上游事实（来自 `tripo_poll` 的观察事实；**不含**重新提交付费请求的能力）。
///
/// 只带 `task_id`：链接本体是易失的（[`EphemeralLinks`] / 按需重查），
/// **不**从持久化事实里读 URL（T20/BUG-008）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct DownloadFacts {
    task_id: String,
}

/// 读取 `tripo_poll` 阶段事实里的 `remoteTaskId`（恢复判据是 task ID，不是 URL）。
///
/// 缺失时 `needs_input`（可诊断、可恢复）：**绝不**因为"下载失败"而重新提交付费请求。
async fn load_download_facts(ctx: &StageContext) -> Loaded<DownloadFacts> {
    let mut conn = ctx.pool.acquire().await.map_err(internal)?;
    let stages = repo::job_stages::list_for_job(&mut conn, &ctx.job.id)
        .await
        .map_err(internal)?;
    let poll = stages
        .iter()
        .find(|stage| stage.stage_kind == StageKind::TripoPoll)
        .ok_or_else(|| {
            needs_input(
                "tripo_poll_missing",
                "找不到 tripo_poll 阶段：无法取得模型链接（不会重新提交付费请求）",
            )
        })?;
    let usage = poll.usage_json.as_ref();
    let task_id = match usage
        .and_then(|usage| usage.get("remoteTaskId"))
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        Some(task_id) => task_id,
        None => {
            // 回退：同一 job 的 accepted 提交事实（与轮询阶段取 task ID 用的是同一份事实）。
            let attempt = repo::attempts::latest_accepted_for_job(
                &mut conn,
                &ctx.job.id,
                StageKind::TripoSubmit,
            )
            .await
            .map_err(internal)?;
            attempt
                .and_then(|attempt| attempt.remote_task_id)
                .ok_or_else(|| {
                    needs_input(
                        "tripo_task_id_missing",
                        "找不到已提交的远端 task ID：无法重新查询链接（不会重新购买）",
                    )
                })?
        }
    };
    Ok(DownloadFacts { task_id })
}

/// URL 路径最后一段 → 安全文件名校验（只作元数据；不含查询串）。
fn file_name_from_url(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back().map(str::to_owned))
        })
        .and_then(|last| sanitize_original_name(Some(&last)))
        .unwrap_or_else(|| "model.glb".to_owned())
}

/// 下载失败的结论映射（付费安全性：链接过期只重新**查询**，绝不重新购买）。
fn download_failure_outcome(error: DownloadError) -> StageOutcome {
    match &error {
        // 网络类失败（含 DNS 解析失败）：可安全重试（下载不产生供应商费用）。
        DownloadError::Transport { .. }
        | DownloadError::ServerError { .. }
        | DownloadError::ResolutionFailed { .. } => StageOutcome::Retryable {
            reason: error.message(),
            retry_after_seconds: None,
        },
        DownloadError::RateLimited {
            retry_after_seconds,
        } => StageOutcome::Retryable {
            reason: error.message(),
            retry_after_seconds: *retry_after_seconds,
        },
        DownloadError::LinkExpired { .. } => StageOutcome::Retryable {
            reason: format!("{}（下一轮将再次查询已知任务）", error.message()),
            retry_after_seconds: None,
        },
        // 需要人工改变条件（磁盘/产品预算/允许域配置/被拒地址）→ needs_input，
        // 缺项代码沿用下载错误码（稳定、可被前端与测试精确定位）。
        DownloadError::InsufficientStorage { .. } => StageOutcome::NeedsInput {
            items: vec![MissingItem::new(error.code(), error.message())],
        },
        DownloadError::TooLarge { .. } => StageOutcome::NeedsInput {
            items: vec![MissingItem::new(
                error.code(),
                format!(
                    "{}；请更换资料或重新生成（不自动降预算、不静默改坏模型）",
                    error.message()
                ),
            )],
        },
        // 允许域/协议/地址相关：部署配置或链接形态需要人工处理 → needs_input
        // （不是"重试就能好"的失败；不消耗安全重试额度）。
        DownloadError::HostNotAllowed { .. }
        | DownloadError::ForbiddenAddress { .. }
        | DownloadError::InsecureScheme { .. } => StageOutcome::NeedsInput {
            items: vec![MissingItem::new(error.code(), error.message())],
        },
        _ => StageOutcome::Failed {
            reason: error.message(),
        },
    }
}

/// `model_download`：下载模型链接（易失缓存优先，未命中按 task ID 重新查询），
/// 流式落盘为本地资产。
///
/// 付费安全性：本阶段**没有任何付费请求**；链接过期或易失缓存未命中（进程重启、
/// 崩溃恢复）时通过 `GET /tasks/{id}` 重新查询已知任务取新链接
/// （`POST /generation/multiview-to-model` 计数不增加）。
pub struct TripoModelDownloadHandler {
    client: Arc<TripoClient>,
    downloader: Arc<ModelDownloader>,
    links: Arc<EphemeralLinks>,
}

impl TripoModelDownloadHandler {
    pub fn new(
        client: Arc<TripoClient>,
        downloader: Arc<ModelDownloader>,
        links: Arc<EphemeralLinks>,
    ) -> Self {
        Self {
            client,
            downloader,
            links,
        }
    }

    /// 取得一条可用链接：先查进程内易失缓存（刚观察到的链接），未命中再
    /// `GET /tasks/{id}` 重新查询（免费；**绝不**重新提交付费请求）。
    async fn acquire_link(&self, ctx: &StageContext, task_id: &str) -> Loaded<String> {
        if let Some(model_url) = self.links.get(&ctx.job.id, task_id) {
            return Ok(model_url);
        }
        tracing::info!(
            event = "model_download_link_requeried",
            jobId = %ctx.job.id,
            stageId = %ctx.stage.id,
            remoteTaskId = %task_id,
            "临时链接不在本进程缓存（重启/恢复）：按 task ID 重新查询（不重新购买）"
        );
        match self.client.get_task(task_id).await {
            Ok(task) => match task.model_url.clone() {
                Some(model_url) => {
                    // 新链接同样只留在进程内（不落库）。
                    self.links
                        .remember(&ctx.job.id, task_id, &model_url, ctx.now);
                    Ok(model_url)
                }
                None => Err(artifact_unavailable_outcome(&task).into()),
            },
            Err(error) => Err(StageOutcome::Retryable {
                reason: format!(
                    "重新查询已知任务取模型链接失败（不改变购买事实）：{}",
                    error.redacted()
                ),
                retry_after_seconds: error.retry_after_seconds(),
            }
            .into()),
        }
    }

    /// 本地副本优先：重跑/重试时若该阶段已保存过模型资产且文件仍在，直接复用（不再下载）。
    async fn reuse_local_copy(&self, ctx: &StageContext) -> Loaded<Option<StageOutcome>> {
        let Some(asset_id) = ctx.stage.result_asset_id.clone() else {
            return Ok(None);
        };
        let mut conn = ctx.pool.acquire().await.map_err(internal)?;
        let Some((asset, blob)) = repo::assets::get_with_blob(&mut conn, &asset_id)
            .await
            .map_err(internal)?
        else {
            return Ok(None);
        };
        let path = crate::assets::blob_path(self.downloader.data_dir(), &blob.sha256);
        if blob.storage_state != manual_core::domain::BlobStorageState::Stored || !path.is_file() {
            return Ok(None);
        }
        tracing::info!(
            event = "model_download_reused_local_copy",
            jobId = %ctx.job.id,
            stageId = %ctx.stage.id,
            assetId = %asset.id,
            "已有本地模型副本：跳过下载（临时供应商链接可能已过期，本地副本优先）"
        );
        Ok(Some(StageOutcome::Succeeded {
            result_asset_id: Some(asset.id),
            usage: Some(json!({
                "sha256": blob.sha256,
                "sizeBytes": blob.size,
                "reusedLocalCopy": true,
            })),
        }))
    }

    /// 短事务提交 blob + asset（purpose=model）；文件此时已在内容寻址位置。
    async fn commit_asset(
        &self,
        ctx: &StageContext,
        downloaded: &crate::assets::glb::DownloadedModel,
        file_name: &str,
    ) -> Loaded<String> {
        let mut conn = ctx.pool.acquire().await.map_err(internal)?;
        // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
        let mut transaction = crate::storage::begin_write(&mut conn)
            .await
            .map_err(internal)?;
        repo::blobs::insert_if_absent(
            &mut transaction,
            &downloaded.sha256,
            downloaded.size,
            "model/gltf-binary",
        )
        .await
        .map_err(internal)?;
        let blob = repo::blobs::get(&mut transaction, &downloaded.sha256)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                HandlerStop(StageOutcome::Failed {
                    reason: "blob 元数据插入后读取失败".to_owned(),
                })
            })?;
        match blob.storage_state {
            manual_core::domain::BlobStorageState::Stored => {}
            manual_core::domain::BlobStorageState::Missing => {
                repo::blobs::set_storage_state(
                    &mut transaction,
                    &downloaded.sha256,
                    manual_core::domain::BlobStorageState::Stored,
                )
                .await
                .map_err(internal)?;
            }
            manual_core::domain::BlobStorageState::Quarantined => {
                return Err(needs_input(
                    "model_content_quarantined",
                    "该模型内容与此前被隔离的 blob 相同：请管理员先处理隔离记录（不静默解隔离）",
                ));
            }
        }
        let asset = repo::assets::insert(
            &mut transaction,
            repo::assets::NewAsset {
                blob_id: downloaded.sha256.clone(),
                item_id: ctx.job.item_id.clone(),
                purpose: AssetPurpose::Model,
                original_name: Some(file_name.to_owned()),
            },
        )
        .await
        .map_err(internal)?;
        transaction.commit().await.map_err(internal)?;
        Ok(asset.id)
    }

    /// 链接过期时的处理：**重新查询已知任务**取新链接（绝不重新提交付费请求）。
    ///
    /// `Ok(模型)` = 用新链接下载成功（由调用方继续提交元数据）；
    /// `Err(结论)` = 重新查询/下载的失败结论（可重试/明确失败）。
    async fn refresh_link_and_download(
        &self,
        ctx: &StageContext,
        facts: &DownloadFacts,
        first_error: &DownloadError,
    ) -> Result<crate::assets::glb::DownloadedModel, StageOutcome> {
        tracing::warn!(
            event = "model_download_link_expired",
            remoteTaskId = %facts.task_id,
            errorCode = first_error.code(),
            "模型链接过期：重新查询已知远端任务（不会重新购买）"
        );
        match self.client.get_task(&facts.task_id).await {
            Ok(task) => {
                let Some(model_url) = task.model_url.clone() else {
                    return Err(artifact_unavailable_outcome(&task));
                };
                // 续签后的链接同样只留在进程内（覆盖旧条目；不落库）。
                self.links
                    .remember(&ctx.job.id, &facts.task_id, &model_url, ctx.now);
                self.downloader
                    .download(&model_url)
                    .await
                    .map_err(download_failure_outcome)
            }
            Err(error) => {
                // 查询失败 ≠ 生成失败：保留已知事实（task ID 与预留），按退避重试。
                Err(StageOutcome::Retryable {
                    reason: format!(
                        "链接过期后重新查询任务失败（不改变购买事实）：{}",
                        error.redacted()
                    ),
                    retry_after_seconds: error.retry_after_seconds(),
                })
            }
        }
    }
}

/// 远端任务已无可下载产物（过期/封禁/失败/取消）：**明确不可恢复**，
/// 需用户确认新预算后才能重新生成（不自动重新购买）。
fn artifact_unavailable_outcome(task: &TaskData) -> StageOutcome {
    let normalized = super::status::NormalizedStatus::new(&task.status_raw);
    use super::status::TripoState;
    match normalized.state() {
        // success 但没有可下载链接：可能仍在物化，等待下一轮（不重购）。
        TripoState::Success => StageOutcome::Retryable {
            reason: "远端任务 success 但没有可下载模型链接：保留 task ID 继续查询（不重新购买）"
                .to_owned(),
            retry_after_seconds: None,
        },
        TripoState::Queued | TripoState::Running | TripoState::Unrecognized => {
            StageOutcome::Retryable {
                reason: format!(
                    "远端任务状态仍为 {}：保留 task ID 继续查询（不重新购买）",
                    normalized.raw()
                ),
                retry_after_seconds: None,
            }
        }
        TripoState::Expired | TripoState::Banned | TripoState::Failed | TripoState::Cancelled => {
            StageOutcome::Failed {
                reason: format!(
                    "远端产物不可找回（status={}，且本地没有副本）：不可自动恢复；\
                     如需重新生成请确认新的预算（不会自动重新购买）",
                    normalized.raw()
                ),
            }
        }
    }
}

impl StageHandler for TripoModelDownloadHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            // 0) 本地副本优先（重试/恢复时不重复下载；供应商链接可能已过期）。
            match self.reuse_local_copy(ctx).await {
                Ok(Some(outcome)) => return Ok(outcome),
                Ok(None) => {}
                Err(stop) => return Ok(stop.0),
            }
            // 1) 上游事实（task ID；链接按需取得，不读持久化的 URL）。
            let facts = match load_download_facts(ctx).await {
                Ok(facts) => facts,
                Err(stop) => return Ok(stop.0),
            };
            let model_url = match self.acquire_link(ctx, &facts.task_id).await {
                Ok(model_url) => model_url,
                Err(stop) => return Ok(stop.0),
            };
            let file_name = file_name_from_url(&model_url);
            let host = reqwest::Url::parse(&model_url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_owned))
                .unwrap_or_else(|| "(未知)".to_owned());

            // 2) 下载（失败分类见 `download_failure_outcome`；链接过期只重新查询）。
            let mut link_refreshed = false;
            let downloaded = match self.downloader.download(&model_url).await {
                Ok(downloaded) => downloaded,
                Err(error @ DownloadError::LinkExpired { .. }) => {
                    match self.refresh_link_and_download(ctx, &facts, &error).await {
                        Ok(downloaded) => {
                            link_refreshed = true;
                            downloaded
                        }
                        Err(outcome) => return Ok(outcome),
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        event = "model_download_failed",
                        jobId = %ctx.job.id,
                        stageId = %ctx.stage.id,
                        host = %host,
                        errorCode = error.code(),
                        detail = %error.log_summary(),
                        "模型下载失败（不重新购买）"
                    );
                    return Ok(download_failure_outcome(error));
                }
            };

            // 3) 元数据短事务（文件已先落盘）。
            let asset_id = match self.commit_asset(ctx, &downloaded, &file_name).await {
                Ok(asset_id) => asset_id,
                Err(stop) => return Ok(stop.0),
            };
            tracing::info!(
                event = "model_download_saved",
                jobId = %ctx.job.id,
                stageId = %ctx.stage.id,
                assetId = %asset_id,
                sha256 = %downloaded.sha256,
                sizeBytes = downloaded.size,
                host = %host,
                "模型已保存为本地资产（原始模型 URL 是临时链接，不作为永久地址保存）"
            );
            Ok(StageOutcome::Succeeded {
                result_asset_id: Some(asset_id),
                usage: Some(json!({
                    "sha256": downloaded.sha256,
                    "sizeBytes": downloaded.size,
                    "host": host,
                    "linkRefreshed": link_refreshed,
                })),
            })
        })
    }
}

// ---------------------------------------------------------------------------
// model_validate（T13 / AC-044）
// ---------------------------------------------------------------------------

/// `model_validate`：GLB 结构校验与产品预算判定 → 不可变 `model_revision`。
///
/// - 通过 → `validated` revision（含 bounds）；
/// - 超预算或结构问题 → `needs_input`（可行动原因）+ `rejected` revision；
///   **原始模型与错误都保留**（不静默改坏模型、不自动降预算、不自动重下）。
pub struct TripoModelValidateHandler {
    data_dir: PathBuf,
    budget: GlbBudget,
}

impl TripoModelValidateHandler {
    pub fn new(data_dir: PathBuf, budget: GlbBudget) -> Self {
        Self { data_dir, budget }
    }

    /// 读取下载阶段的结果资产（本地副本）与其文件路径。
    async fn load_downloaded(&self, ctx: &StageContext) -> Loaded<(String, String, u64, PathBuf)> {
        let mut conn = ctx.pool.acquire().await.map_err(internal)?;
        let stages = repo::job_stages::list_for_job(&mut conn, &ctx.job.id)
            .await
            .map_err(internal)?;
        let download = stages
            .iter()
            .find(|stage| stage.stage_kind == StageKind::ModelDownload)
            .ok_or_else(|| {
                needs_input(
                    "model_download_missing",
                    "找不到 model_download 阶段：无法取得要校验的模型（不重新下载）",
                )
            })?;
        let asset_id = download.result_asset_id.clone().ok_or_else(|| {
            needs_input(
                "model_download_result_missing",
                "下载阶段没有结果资产：请先完成模型下载（不重新下载、不重新购买）",
            )
        })?;
        let (asset, blob) = repo::assets::get_with_blob(&mut conn, &asset_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                needs_input(
                    "model_asset_missing",
                    format!("模型资产（{asset_id}）不存在：请人工核对（不自动重新下载）"),
                )
            })?;
        if asset.purpose != AssetPurpose::Model {
            return Err(needs_input(
                "model_asset_purpose_invalid",
                "下载阶段的结果资产用途不是 model：请人工核对（不猜测内容）",
            ));
        }
        if blob.storage_state != manual_core::domain::BlobStorageState::Stored {
            return Err(needs_input(
                "model_blob_unavailable",
                format!(
                    "模型内容文件状态异常（{:?}）：请核对 data-dir 完整性",
                    blob.storage_state
                ),
            ));
        }
        let path = crate::assets::blob_path(&self.data_dir, &blob.sha256);
        let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
            needs_input(
                "model_file_missing",
                format!("模型内容文件不可读：{error}；请核对 data-dir 完整性"),
            )
        })?;
        if metadata.len() != blob.size as u64 {
            return Err(needs_input(
                "model_file_size_mismatch",
                format!(
                    "模型文件大小（{} 字节）与元数据（{} 字节）不一致：请人工核对（不静默继续）",
                    metadata.len(),
                    blob.size
                ),
            ));
        }
        Ok((asset.id, blob.sha256, blob.size as u64, path))
    }

    /// 幂等写入 revision（`(item_id, sha256)` 唯一语义；重试不产生第二行）。
    async fn record_revision(
        &self,
        ctx: &StageContext,
        asset_id: &str,
        sha256: &str,
        validation_state: ModelValidationState,
        bounds: Option<Value>,
    ) -> Result<String, JobError> {
        let mut conn = ctx.pool.acquire().await?;
        let attempt =
            repo::attempts::latest_accepted_for_job(&mut conn, &ctx.job.id, StageKind::TripoSubmit)
                .await?;
        // `BEGIN IMMEDIATE`（BUG-006）：`get_or_create` 先读（按 sha 查重）后写（插入 revision）。
        let mut transaction = crate::storage::begin_write(&mut conn).await?;
        let revision = repo::model_revisions::get_or_create(
            &mut transaction,
            repo::model_revisions::NewModelRevision {
                item_id: ctx.job.item_id.clone(),
                asset_id: asset_id.to_owned(),
                sha256: sha256.to_owned(),
                provider_attempt_id: attempt.map(|attempt| attempt.id),
                bounds,
                validation_state,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(revision.id)
    }
}

impl StageHandler for TripoModelValidateHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            let (asset_id, sha256, size, path) = match self.load_downloaded(ctx).await {
                Ok(values) => values,
                Err(stop) => return Ok(stop.0),
            };

            // GLB 解析是阻塞 CPU 工作：放有界阻塞线程（architecture §6），不阻塞异步运行时。
            let budget = self.budget;
            let path_for_task = path.clone();
            let inspected =
                tokio::task::spawn_blocking(move || glb::inspect_glb_file(&path_for_task, &budget))
                    .await;
            let summary = match inspected {
                Ok(Ok(summary)) => summary,
                Ok(Err(error)) => {
                    // 校验未通过：保留原始模型与错误（写 rejected revision + 事实），
                    // 进入 needs_input 等待人工决定（不自动降预算、不静默改坏模型）。
                    let revision_id = match self
                        .record_revision(
                            ctx,
                            &asset_id,
                            &sha256,
                            ModelValidationState::Rejected,
                            None,
                        )
                        .await
                    {
                        Ok(id) => id,
                        Err(error) => return Ok(internal(error).0),
                    };
                    let failure_usage = json!({
                        "sha256": sha256,
                        "sizeBytes": size,
                        "assetId": asset_id,
                        "modelRevisionId": revision_id,
                        "validation": "rejected",
                        "failureCode": error.code(),
                    });
                    if is_current_lease_holder(&ctx.pool, &ctx.stage.id, ctx.stage.lease_epoch)
                        .await
                        && let Err(write_error) = record_result_fact(
                            &ctx.pool,
                            &ctx.stage.id,
                            None,
                            Some(&failure_usage.to_string()),
                            ctx.now,
                        )
                        .await
                    {
                        tracing::warn!(
                            event = "model_validate_fact_write_failed",
                            stageId = %ctx.stage.id,
                            error = %write_error,
                            "校验失败事实写入失败（错误仍会写入 needs_input）"
                        );
                    }
                    let suffix = if error.is_budget() {
                        "（超预算；原始模型已保留，未自动降预算、未修改模型）"
                    } else {
                        "（原始模型已保留，未静默修改）"
                    };
                    tracing::warn!(
                        event = "model_validate_rejected",
                        jobId = %ctx.job.id,
                        stageId = %ctx.stage.id,
                        assetId = %asset_id,
                        sha256 = %sha256,
                        failureCode = error.code(),
                        detail = %error.log_summary(),
                        "模型校验未通过：原始模型与错误已保留，等待人工决定"
                    );
                    return Ok(StageOutcome::NeedsInput {
                        items: vec![MissingItem::new(
                            error.code(),
                            format!("{}{suffix}", error.message()),
                        )],
                    });
                }
                Err(join_error) => {
                    return Ok(StageOutcome::Retryable {
                        reason: format!("GLB 校验线程未完成：{join_error}"),
                        retry_after_seconds: None,
                    });
                }
            };

            // 通过：不可变 revision（validated）+ bounds。
            let revision_id = match self
                .record_revision(
                    ctx,
                    &asset_id,
                    &sha256,
                    ModelValidationState::Validated,
                    Some(summary.bounds_json()),
                )
                .await
            {
                Ok(id) => id,
                Err(error) => return Ok(internal(error).0),
            };
            tracing::info!(
                event = "model_validated",
                jobId = %ctx.job.id,
                stageId = %ctx.stage.id,
                assetId = %asset_id,
                modelRevisionId = %revision_id,
                sha256 = %sha256,
                triangles = summary.triangles,
                vertices = summary.vertices,
                maxTextureDimension = summary.max_texture_dimension,
                "模型校验通过：已创建不可变 model revision（validated）"
            );
            Ok(StageOutcome::Succeeded {
                result_asset_id: Some(asset_id),
                usage: Some(json!({
                    "modelRevisionId": revision_id,
                    "sha256": sha256,
                    "sizeBytes": size,
                    "validation": "validated",
                    "triangles": summary.triangles,
                    "vertices": summary.vertices,
                    "meshes": summary.meshes,
                    "primitives": summary.primitives,
                    "images": summary.images,
                    "maxTextureDimension": summary.max_texture_dimension,
                    "bounds": summary.bounds_json(),
                })),
            })
        })
    }
}

// ---------------------------------------------------------------------------
// 账本联动（付费 attempt ↔ 预留；contracts.md §4）
// ---------------------------------------------------------------------------

/// 该 job 快照上的 Tripo 预留条目。
async fn tripo_ledger_entry(
    pool: &sqlx::SqlitePool,
    snapshot_id: &str,
) -> Result<Option<CostLedgerEntry>, JobError> {
    let mut conn = pool.acquire().await?;
    let entries = repo::ledger::list_for_snapshot(&mut conn, snapshot_id).await?;
    Ok(entries
        .into_iter()
        .find(|entry| entry.provider == ProviderKey::Tripo))
}

/// 结果未知：保留预留（`actual` 保持 NULL，不得填 0），并把 attempt 关联到该条目。
async fn mark_tripo_reservation_unknown(
    pool: &sqlx::SqlitePool,
    snapshot_id: &str,
    attempt_id: Option<&str>,
    now: &Timestamp,
) -> String {
    match tripo_ledger_entry(pool, snapshot_id).await {
        Ok(Some(entry)) => {
            let mut conn = match pool.acquire().await {
                Ok(conn) => conn,
                Err(error) => return format!("账本联动失败（获取连接）：{error}"),
            };
            match generation_ledger::mark_submission_unknown(&mut conn, &entry.id, attempt_id, *now)
                .await
            {
                Ok(outcome) => format!("预留保留为 unknown（{}）", outcome_name(&outcome)),
                Err(error) => format!("账本联动失败（保留预留不变）：{error}"),
            }
        }
        Ok(None) => "该快照没有 Tripo 预留条目（不创建）".to_owned(),
        Err(error) => format!("账本读取失败（保留预留不变）：{error}"),
    }
}

/// 明确未计费（供应商明确拒绝）：释放 Tripo 预留。
async fn release_tripo_reservation(
    pool: &sqlx::SqlitePool,
    snapshot_id: &str,
    now: &Timestamp,
) -> String {
    match tripo_ledger_entry(pool, snapshot_id).await {
        Ok(Some(entry)) => {
            let mut conn = match pool.acquire().await {
                Ok(conn) => conn,
                Err(error) => return format!("账本联动失败（获取连接）：{error}"),
            };
            match generation_ledger::release_definitely_not_billed(&mut conn, &entry.id, *now).await
            {
                Ok(outcome) => format!("付费提交被明确拒绝（未计费）→ {}", outcome_name(&outcome)),
                Err(error) => format!("账本释放失败（保留预留不变）：{error}"),
            }
        }
        Ok(None) => "该快照没有 Tripo 预留条目（不创建）".to_owned(),
        Err(error) => format!("账本读取失败（保留预留不变）：{error}"),
    }
}

/// 拿到供应商计费事实后按实际金额结算（同值幂等；无计费事实不猜测金额）。
async fn settle_tripo_reservation(
    pool: &sqlx::SqlitePool,
    snapshot_id: &str,
    billing: Option<&BillingFact>,
    now: &Timestamp,
) -> String {
    let Some(billing) = billing else {
        return "远端未提供计费字段：保留预留（不用猜测金额结算）".to_owned();
    };
    match tripo_ledger_entry(pool, snapshot_id).await {
        Ok(Some(entry)) => {
            let mut conn = match pool.acquire().await {
                Ok(conn) => conn,
                Err(error) => return format!("账本联动失败（获取连接）：{error}"),
            };
            match generation_ledger::settle_attempt(
                &mut conn,
                &entry.id,
                billing.credit_minor,
                *now,
            )
            .await
            {
                Ok(outcome) => format!(
                    "按供应商实际 credits（{} → {} creditMinor）结算：{}",
                    billing.literal,
                    billing.credit_minor,
                    outcome_name(&outcome)
                ),
                Err(error) => format!("账本结算失败（保留原状态）：{error}"),
            }
        }
        Ok(None) => "该快照没有 Tripo 预留条目（不创建）".to_owned(),
        Err(error) => format!("账本读取失败（保留原状态）：{error}"),
    }
}

fn outcome_name(outcome: &repo::ledger::LedgerOutcome) -> String {
    match outcome {
        repo::ledger::LedgerOutcome::Applied => "已生效".to_owned(),
        repo::ledger::LedgerOutcome::Idempotent => "重复事件（不变）".to_owned(),
        repo::ledger::LedgerOutcome::Rejected { reason } => format!("被拒绝：{reason}"),
    }
}

/// 阶段端点摘要（诊断/测试；不含用户数据）。
pub fn stage_endpoint_summary() -> (String, String, String) {
    (
        TRIPO_UPLOAD_PATH.to_owned(),
        TRIPO_SUBMIT_PATH.to_owned(),
        TRIPO_TASKS_PATH_PREFIX.to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_facts_roundtrip_and_tolerate_corruption() {
        let facts = vec![
            UploadFact {
                view: "front".to_owned(),
                sha256: "a".repeat(64),
                token: "tok-front".to_owned(),
                token_field: "file_token".to_owned(),
            },
            UploadFact {
                view: "left".to_owned(),
                sha256: "b".repeat(64),
                token: "tok-left".to_owned(),
                token_field: "image_token".to_owned(),
            },
        ];
        let usage = upload_facts_usage(&facts);
        assert_eq!(parse_upload_facts(Some(&usage)), facts);
        // 损坏的 usage 按"没有事实"处理（重新上传免费且幂等）。
        assert!(parse_upload_facts(Some(&json!({"uploads": [{"view": 1}]}))).is_empty());
        assert!(parse_upload_facts(None).is_empty());
    }

    #[test]
    fn confirmed_inputs_has_front_and_token_lookup() {
        let inputs = ConfirmedInputs {
            parameters: SubmitParameters {
                model: "v3.1-20260211".to_owned(),
                texture: true,
                pbr: true,
                texture_quality: "standard".to_owned(),
                geometry_quality: "standard".to_owned(),
                face_limit: 100_000,
                quad: false,
                generate_parts: false,
            },
            views: vec![ConfirmedView {
                view: "front".to_owned(),
                photo_id: "p1".to_owned(),
                sha256: "c".repeat(64),
            }],
        };
        assert!(inputs.has_front());
        assert!(!inputs.views.is_empty());
    }

    #[test]
    fn task_usage_keeps_raw_and_normalized_status() {
        let task = super::super::dto::TaskData {
            status_raw: "some_new_status".to_owned(),
            progress_raw: Some("7".to_owned()),
            model_url: None,
            rendered_image_url: None,
            billing: Some(BillingFact {
                source_field: "credits_consumed".to_owned(),
                literal: "30".to_owned(),
                credit_minor: 3000,
            }),
            billing_problem: None,
            data_keys: vec!["status".to_owned()],
        };
        let usage = task_usage(
            "task-1",
            &task,
            super::super::status::TripoState::Unrecognized,
        );
        assert_eq!(usage["rawStatus"], "some_new_status");
        assert_eq!(usage["normalizedStatus"], "unrecognized");
        assert_eq!(usage["billing"]["creditMinor"], 3000);
        assert_eq!(usage["remoteTaskId"], "task-1");
        assert!(usage.get("modelUrl").is_none());
    }

    /// T20/BUG-008：阶段事实里的临时/签名 URL 只保留 host + sha256 摘要，
    /// **不得**出现任何 URL 字符串或签名串（AC-010）。
    #[test]
    fn task_usage_redacts_temporary_urls_to_summary() {
        let model_url = "https://cdn.example.invalid/model.glb?sign=canary-usage-signature";
        let rendered = "https://cdn.example.invalid/preview.png?token=canary-render";
        let task = super::super::dto::TaskData {
            status_raw: "success".to_owned(),
            progress_raw: Some("100".to_owned()),
            model_url: Some(model_url.to_owned()),
            rendered_image_url: Some(rendered.to_owned()),
            billing: Some(BillingFact {
                source_field: "credits_consumed".to_owned(),
                literal: "30".to_owned(),
                credit_minor: 3000,
            }),
            billing_problem: None,
            data_keys: vec!["status".to_owned()],
        };
        let usage = task_usage("task-1", &task, super::super::status::TripoState::Success);
        let rendered_json = usage.to_string();
        assert!(
            !rendered_json.contains("://"),
            "阶段事实不得含 URL：{rendered_json}"
        );
        assert!(
            !rendered_json.contains("canary"),
            "阶段事实不得含签名/查询串：{rendered_json}"
        );
        assert_eq!(usage["modelUrl"]["redacted"], true);
        assert_eq!(
            usage["modelUrl"]["host"],
            crate::redaction::url_summary(model_url).host.unwrap()
        );
        assert_eq!(
            usage["modelUrl"]["sha256"],
            crate::redaction::url_summary(model_url).sha256_prefix
        );
        assert_eq!(usage["renderedImageUrl"]["redacted"], true);
        assert_eq!(usage["renderedImageUrl"]["host"], "cdn.example.invalid");
        // 诊断事实保留：task ID / 状态 / 计费不被脱敏破坏。
        assert_eq!(usage["remoteTaskId"], "task-1");
        assert_eq!(usage["normalizedStatus"], "success");
        assert_eq!(usage["billing"]["creditMinor"], 3000);
    }

    #[test]
    fn stage_endpoints_match_the_frozen_paths() {
        let (upload, submit, tasks) = stage_endpoint_summary();
        assert_eq!(upload, "/files");
        assert_eq!(submit, "/generation/multiview-to-model");
        assert_eq!(tasks, "/tasks/");
    }
}
