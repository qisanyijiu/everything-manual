//! 发布与版本读取服务（T19 / REQ-035、REQ-036；contracts.md §2/§3/§7）。
//!
//! **发布事务**（`POST /items/{id}/drafts/{draftId}/publish`；If-Match + Idempotency-Key）：
//!
//! 1. 幂等键校验（缺失/空/超长 → 422 字段级；同 key 不同 body → 409）；
//! 2. 重放检查（同 key 同 body → 返回**同一 release**，`x-idempotent-replay: true`）；
//! 3. 草稿归属（跨物品 404）与 `If-Match` revision（过期 412）；
//! 4. **发布不变量逐条校验**（[`super::invariants`]）→ 不满足 422 + `details.issues[]`；
//! 5. manifest 冻结：把草稿聚合（知识 + 热点 + 视角 + 复核声明）与全部引用资产的
//!    sha256/来源写成 JSON，作为**内容寻址资产**（purpose `release_manifest`）落盘
//!    （tmp → fsync → 原子 rename，与上传同一套 blob 安全语义）；
//! 6. 写事务：草稿 revision CAS（并发发布/发布前编辑 → 第二个请求 412）→
//!    blob/asset/release 行 → 审计 `release_published` → 幂等记录（唯一键兜底并发）。
//!
//! **不可变性**：`manual_releases` 由 0002 触发器拒绝 UPDATE/DELETE；release 只引用
//! manifest 资产与 model revision，不引用会变的 draft 内容——因此"发布后修改 draft
//! 不改变已发布版本"（AC-055 的字节/哈希比对）。
//!
//! **不产生费用/外呼**：本服务只读写本地数据库与 data-dir（发布是纯本地事务）。

use std::path::Path;

use serde_json::json;
use sqlx::{SqliteConnection, SqlitePool};

use manual_core::domain::ManualRelease;
use manual_core::timestamps::Timestamp;
use manual_core::validation::FieldIssue;

use crate::assets::blob_store;
use crate::drafts::aggregate::ReviewOverlay;
use crate::drafts::knowledge::DraftKnowledge;
use crate::storage::StorageError;
use crate::storage::repo;
use crate::storage::repo::releases as releases_repo;

use super::invariants::{FrozenInput, PublishIssue, check_publish_invariants};

/// 发布幂等的范围键（admin + method + route + key 唯一；route 用模板不是具体路径）。
pub const PUBLISH_IDEMPOTENCY_METHOD: &str = "POST";
pub const PUBLISH_IDEMPOTENCY_ROUTE: &str = "/api/v1/items/{id}/drafts/{draftId}/publish";

/// manifest 的 schema 版本（发布格式；T20 导出按它识别）。
pub const RELEASE_SCHEMA_VERSION: &str = "manual_release_v1";

/// manifest 体积上限（知识聚合 + 资产清单；防御异常输入，正常远小于此值）。
pub const MANIFEST_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// manifest 的 MIME（内容寻址资产；不参与网页直接展示）。
pub const MANIFEST_MIME: &str = "application/json";

/// 审计动作：发布（contracts.md §2「发布」）。
pub const AUDIT_RELEASE_PUBLISHED: &str = "release_published";

/// 发布服务错误（HTTP 层映射见 `http::error`）。
#[derive(Debug, Clone, PartialEq)]
pub enum PublishError {
    /// 草稿不存在或不属于该物品（不泄露存在性）。
    NotFound {
        entity: &'static str,
        id: String,
    },
    /// 请求字段级问题（缺 Idempotency-Key/超长 → 422 `details.fields`）。
    FieldIssues(Vec<FieldIssue>),
    /// 发布不变量不满足（422 `details.issues[]`；逐条给定位信息）。
    Invariants(Vec<PublishIssue>),
    /// 同 key 不同 body（409）。
    IdempotencyConflict {
        message: String,
        details: serde_json::Value,
    },
    /// 存储内容损坏（草稿 JSON/manifest 资产不可读等）。
    Integrity {
        code: &'static str,
        message: String,
    },
    Storage(StorageError),
}

impl PublishError {
    fn integrity(code: &'static str, message: impl Into<String>) -> Self {
        Self::Integrity {
            code,
            message: message.into(),
        }
    }

    /// 稳定错误码（日志与测试断言用；不含用户数据）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "release_not_found",
            Self::FieldIssues(_) => "publish_field_validation",
            Self::Invariants(_) => "publish_invariants_violated",
            Self::IdempotencyConflict { .. } => "idempotency_conflict",
            Self::Integrity { code, .. } => code,
            Self::Storage(_) => "publish_storage",
        }
    }
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { entity, id } => write!(formatter, "{entity} 不存在：{id}"),
            Self::FieldIssues(issues) => write!(formatter, "字段校验失败（{} 项）", issues.len()),
            Self::Invariants(issues) => {
                write!(formatter, "发布不变量不满足（{} 项）", issues.len())
            }
            Self::IdempotencyConflict { message, .. } => formatter.write_str(message),
            Self::Integrity { code, message } => write!(formatter, "{code}：{message}"),
            Self::Storage(error) => write!(formatter, "存储错误：{error}"),
        }
    }
}

impl std::error::Error for PublishError {}

impl From<StorageError> for PublishError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::Error> for PublishError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(StorageError::from(error))
    }
}

impl From<crate::drafts::DraftServiceError> for PublishError {
    fn from(error: crate::drafts::DraftServiceError) -> Self {
        use crate::drafts::DraftServiceError;
        match error {
            DraftServiceError::NotFound { entity, id } => Self::NotFound { entity, id },
            DraftServiceError::Integrity { code, message } => Self::Integrity { code, message },
            DraftServiceError::FieldIssues(issues) => Self::FieldIssues(issues),
            DraftServiceError::Storage(error) => Self::Storage(error),
        }
    }
}

/// 一次发布的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct PublishOutcome {
    pub release: ManualRelease,
    /// 幂等重放（同 key 同 body）：返回既有 release，不新建。
    pub replayed: bool,
    /// 发布后草稿的 revision（客户端下一次 PATCH/发布要用它做 If-Match）。
    /// 发布是聚合根上的显式操作：每次发布都给草稿递增 revision，因此并发/陈旧
    /// If-Match 的第二个发布必然 412（AC-056）。
    pub draft_revision_after_publish: i64,
}

/// 发布草稿（详见模块文档的六步）。
#[allow(clippy::too_many_arguments)]
pub async fn publish_draft(
    pool: &SqlitePool,
    data_dir: &Path,
    admin_id: &str,
    item_id: &str,
    draft_id: &str,
    expected_revision: i64,
    idempotency_key: &str,
    body_hash: &str,
    now: Timestamp,
) -> Result<PublishOutcome, PublishError> {
    let key = validate_idempotency_key(idempotency_key)?;
    let mut conn = pool.acquire().await?;

    // 1) 重放检查（真正的并发竞争由幂等记录唯一键兜底）。
    if let Some(record) = repo::idempotency::find(
        &mut conn,
        admin_id,
        PUBLISH_IDEMPOTENCY_METHOD,
        PUBLISH_IDEMPOTENCY_ROUTE,
        &key,
    )
    .await?
    {
        return replay_publish(&mut conn, record, body_hash).await;
    }

    // 2) 草稿归属 + revision（合同 412）。
    let draft = crate::drafts::read_draft(&mut conn, item_id, draft_id).await?;
    if draft.revision != expected_revision {
        return Err(PublishError::Storage(StorageError::RevisionConflict {
            entity: "manual_draft",
            id: draft_id.to_owned(),
            current_revision: draft.revision,
        }));
    }

    // 3) 冻结内容解析（损坏内容按完整性错误处理，不猜）。
    let knowledge: DraftKnowledge =
        serde_json::from_value(draft.knowledge_json.clone()).map_err(|error| {
            PublishError::integrity(
                "release_knowledge_unreadable",
                format!("草稿知识不是合法形状：{error}"),
            )
        })?;
    let overlay: ReviewOverlay = crate::drafts::parse_review_overlay(&draft.review_json)?;

    // 4) 冻结输入（快照 → preparation）：引用页校验的事实来源。
    let frozen = load_frozen_input(&mut conn, &draft.snapshot_id).await?;

    // 5) 发布不变量（逐条检查；不满足 → 422 明细）。
    let issues = check_publish_invariants(&knowledge, &overlay, frozen.as_ref());
    if !issues.is_empty() {
        return Err(PublishError::Invariants(issues));
    }
    let frozen = frozen.ok_or_else(|| {
        PublishError::integrity(
            "release_input_missing",
            "发布不变量通过但找不到冻结输入（不应发生）",
        )
    })?;

    // 6) manifest 冻结（模型 + 资产 sha256 + 来源）。
    let model_revision_id = knowledge
        .model
        .as_ref()
        .map(|model| model.revision_id.clone())
        .ok_or_else(|| {
            PublishError::integrity("release_model_missing", "发布时缺少模型版本（不应发生）")
        })?;
    let release_id = manual_core::ids::new_id();
    let manifest = build_manifest(
        &mut conn,
        &release_id,
        &draft,
        &knowledge,
        &overlay,
        &model_revision_id,
        &frozen,
        now,
    )
    .await?;
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|error| {
        PublishError::integrity(
            "release_manifest_serialization_failed",
            format!("manifest 序列化失败：{error}"),
        )
    })?;
    if manifest_bytes.len() as u64 > MANIFEST_MAX_BYTES {
        return Err(PublishError::integrity(
            "release_manifest_too_large",
            format!(
                "manifest 超过体积上限（{} 字节）：请检查知识聚合规模",
                MANIFEST_MAX_BYTES
            ),
        ));
    }

    // 7) 写盘（tmp → fsync → 原子 rename；失败不半提交，元数据失败不删文件）。
    let staged = stage_manifest(data_dir, &manifest_bytes).await?;

    // 8) 写事务：草稿 CAS → blob/asset/release → 审计 → 幂等记录。
    let tx = crate::storage::begin_write(&mut conn).await?;
    let mut tx = tx;
    let draft = match releases_repo_draft_bump(&mut tx, draft_id, expected_revision, now).await {
        Ok(draft) => draft,
        Err(error) => {
            // 并发发布/发布前编辑：release 不写入；已 promote 的 manifest 文件成为
            // 孤儿（由启动扫描按引用收敛，与上传失败路径同一语义）。
            tx.rollback().await?;
            return Err(PublishError::Storage(error));
        }
    };
    repo::blobs::insert_if_absent(&mut tx, &staged.sha256, staged.size, MANIFEST_MIME).await?;
    // 内容刚由 promote 写入：行已存在但状态异常时按上传路径同样收敛（missing → stored；
    // quarantined 不静默解隔离）。
    if let Some(blob) = repo::blobs::get(&mut tx, &staged.sha256).await? {
        match blob.storage_state {
            manual_core::domain::BlobStorageState::Stored => {}
            manual_core::domain::BlobStorageState::Missing => {
                repo::blobs::set_storage_state(
                    &mut tx,
                    &staged.sha256,
                    manual_core::domain::BlobStorageState::Stored,
                )
                .await?;
            }
            manual_core::domain::BlobStorageState::Quarantined => {
                tx.rollback().await?;
                return Err(PublishError::integrity(
                    "release_manifest_quarantined",
                    "该 manifest 内容与已隔离的 blob 相同：请管理员先处理隔离记录",
                ));
            }
        }
    }
    let manifest_asset = repo::assets::insert(
        &mut tx,
        repo::assets::NewAsset {
            blob_id: staged.sha256.clone(),
            item_id: item_id.to_owned(),
            purpose: manual_core::domain::AssetPurpose::ReleaseManifest,
            original_name: None,
        },
    )
    .await?;
    let release = releases_repo::insert(
        &mut tx,
        releases_repo::NewRelease {
            item_id: item_id.to_owned(),
            draft_id: draft_id.to_owned(),
            // 发布时草稿的 revision（bump 后 = expected_revision + 1；记录的是被发布的内容版本）。
            draft_revision: expected_revision,
            model_revision_id: model_revision_id.clone(),
            manifest_asset_id: manifest_asset.id.clone(),
        },
        now,
    )
    .await?;
    repo::audit::record(
        &mut tx,
        repo::audit::NewAuditEvent {
            entity_type: "manual_release".to_owned(),
            entity_id: release.id.clone(),
            actor: Some(admin_id.to_owned()),
            action: AUDIT_RELEASE_PUBLISHED.to_owned(),
            result: "published".to_owned(),
            metadata_json: Some(
                json!({
                    "itemId": item_id,
                    "draftId": draft_id,
                    "draftRevision": expected_revision,
                    "draftRevisionAfterPublish": draft.revision,
                    "modelRevisionId": model_revision_id,
                    "manifestAssetId": release.manifest_asset_id,
                    "manifestSha256": staged.sha256,
                    "hotspotCount": knowledge.hotspots.len(),
                    "textOnlyPartCount": overlay
                        .entities
                        .values()
                        .filter(|entry| entry.text_only)
                        .count(),
                })
                .to_string(),
            ),
        },
        now,
    )
    .await?;
    match repo::idempotency::insert(
        &mut tx,
        repo::idempotency::NewIdempotencyRecord {
            admin_id: admin_id.to_owned(),
            method: PUBLISH_IDEMPOTENCY_METHOD.to_owned(),
            route: PUBLISH_IDEMPOTENCY_ROUTE.to_owned(),
            key: key.clone(),
            body_hash: body_hash.to_owned(),
            resource_id: Some(release.id.clone()),
            response_status: Some(201),
        },
        now,
    )
    .await
    {
        Ok(_) => {}
        Err(StorageError::UniqueViolation { .. }) => {
            // 并发同键：回滚后按已存在的记录重放/报冲突（release 已由另一个请求写入）。
            tx.rollback().await?;
            let record = repo::idempotency::find(
                &mut conn,
                admin_id,
                PUBLISH_IDEMPOTENCY_METHOD,
                PUBLISH_IDEMPOTENCY_ROUTE,
                &key,
            )
            .await?
            .ok_or_else(|| {
                PublishError::integrity(
                    "release_idempotency_lost",
                    "并发发布幂等键竞争后找不到已存在的记录",
                )
            })?;
            return replay_publish(&mut conn, record, body_hash).await;
        }
        Err(error) => return Err(error.into()),
    }
    tx.commit().await?;

    tracing::info!(
        event = "release_published",
        itemId = %item_id,
        draftId = %draft_id,
        releaseId = %release.id,
        draftRevision = expected_revision,
        manifestSha256 = %staged.sha256,
        "说明书版本已发布（不可变；改草稿不影响此版本）"
    );
    Ok(PublishOutcome {
        release,
        replayed: false,
        draft_revision_after_publish: draft.revision,
    })
}

/// 列出某物品的发布版本（发布时间倒序）。
pub async fn list_releases(
    conn: &mut SqliteConnection,
    item_id: &str,
    limit: i64,
) -> Result<Vec<ManualRelease>, PublishError> {
    Ok(releases_repo::list_for_item(conn, item_id, limit).await?)
}

/// 读取发布记录（跨物品按不存在处理）。
pub async fn read_release(
    conn: &mut SqliteConnection,
    item_id: &str,
    release_id: &str,
) -> Result<ManualRelease, PublishError> {
    releases_repo::get_for_item(conn, item_id, release_id)
        .await?
        .ok_or_else(|| PublishError::NotFound {
            entity: "manual_release",
            id: release_id.to_owned(),
        })
}

/// 读取并解析 release 的 manifest（读取端；字节损坏 → 完整性错误，不猜）。
pub async fn read_manifest(
    conn: &mut SqliteConnection,
    data_dir: &Path,
    release: &ManualRelease,
) -> Result<(serde_json::Value, String), PublishError> {
    let (_asset, blob) = repo::assets::get_with_blob(&mut *conn, &release.manifest_asset_id)
        .await?
        .ok_or_else(|| {
            PublishError::integrity(
                "release_manifest_missing",
                format!("发布清单资产不存在：{}", release.manifest_asset_id),
            )
        })?;
    let path = blob_store::blob_path(data_dir, &blob.sha256);
    let bytes = tokio::fs::read(&path).await.map_err(|error| {
        PublishError::integrity(
            "release_manifest_unreadable",
            format!("发布清单文件不可读：{error}"),
        )
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        PublishError::integrity(
            "release_manifest_invalid",
            format!("发布清单不是合法 JSON：{error}"),
        )
    })?;
    Ok((value, blob.sha256))
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

/// 幂等键校验（缺失/空/超长 → 422 字段级；与 T11/T15 同一规则）。
fn validate_idempotency_key(key: &str) -> Result<String, PublishError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(PublishError::FieldIssues(vec![FieldIssue::new(
            "idempotencyKey",
            "缺少 Idempotency-Key 头：发布必须携带幂等键（重放返回同一发布版本）",
        )]));
    }
    if trimmed.chars().count() > crate::generation::jobs::IDEMPOTENCY_KEY_MAX_CHARS {
        return Err(PublishError::FieldIssues(vec![FieldIssue::new(
            "idempotencyKey",
            format!(
                "幂等键过长（上限 {} 字符）",
                crate::generation::jobs::IDEMPOTENCY_KEY_MAX_CHARS
            ),
        )]));
    }
    Ok(trimmed.to_owned())
}

/// 重放（同 key 同 body → 同一 release）或 409（同 key 不同 body）。
async fn replay_publish(
    conn: &mut SqliteConnection,
    record: manual_core::domain::IdempotencyRecord,
    body_hash: &str,
) -> Result<PublishOutcome, PublishError> {
    if record.body_hash != body_hash {
        return Err(PublishError::IdempotencyConflict {
            message: "该 Idempotency-Key 已用于不同的请求内容：请使用新的键".to_owned(),
            details: json!({
                "reason": "idempotencyKeyReused",
                "existingResourceId": record.resource_id,
            }),
        });
    }
    let release_id = record.resource_id.clone().ok_or_else(|| {
        PublishError::integrity("release_idempotency_lost", "发布幂等记录缺少 resource_id")
    })?;
    let release = releases_repo::get(conn, &release_id)
        .await?
        .ok_or(PublishError::NotFound {
            entity: "manual_release",
            id: release_id,
        })?;
    // 重放也要给出"当前草稿 revision"（客户端刷新后再操作）。
    let current: Option<i64> =
        sqlx::query_scalar("SELECT revision FROM manual_drafts WHERE id = ?")
            .bind(&release.draft_id)
            .fetch_optional(&mut *conn)
            .await?;
    let draft_revision_after_publish = current.unwrap_or(release.draft_revision);
    Ok(PublishOutcome {
        release,
        replayed: true,
        draft_revision_after_publish,
    })
}

/// 草稿 revision 递增（发布是聚合根上的显式操作；并发第二个请求 → 412）。
async fn releases_repo_draft_bump(
    tx: &mut sqlx::SqliteConnection,
    draft_id: &str,
    expected_revision: i64,
    now: Timestamp,
) -> Result<manual_core::domain::ManualDraft, StorageError> {
    crate::storage::repo::drafts::bump_revision_for_publish(tx, draft_id, expected_revision, now)
        .await
}

/// 读取冻结输入事实（快照 → preparation）。
async fn load_frozen_input(
    conn: &mut SqliteConnection,
    snapshot_id: &str,
) -> Result<Option<FrozenInput>, PublishError> {
    let Some(snapshot) = repo::snapshots::get(&mut *conn, snapshot_id).await? else {
        return Ok(None);
    };
    let Some(preparation) = repo::preparations::get(&mut *conn, &snapshot.preparation_id).await?
    else {
        return Ok(None);
    };
    Ok(Some(FrozenInput {
        preparation_id: preparation.id,
        document_id: preparation.document_id,
        page_count: preparation.page_count.unwrap_or(0),
        ready: preparation.state == manual_core::domain::PreparationState::Ready,
    }))
}

/// 构建不可变 manifest（草稿聚合 + 复核 + 资产清单与来源）。
#[allow(clippy::too_many_arguments)]
async fn build_manifest(
    conn: &mut SqliteConnection,
    release_id: &str,
    draft: &manual_core::domain::ManualDraft,
    knowledge: &DraftKnowledge,
    overlay: &ReviewOverlay,
    model_revision_id: &str,
    frozen: &FrozenInput,
    now: Timestamp,
) -> Result<serde_json::Value, PublishError> {
    let model_revision = repo::model_revisions::get(&mut *conn, model_revision_id)
        .await?
        .ok_or_else(|| {
            PublishError::integrity(
                "release_model_missing",
                format!("模型版本不存在：{model_revision_id}"),
            )
        })?;

    let mut assets: Vec<serde_json::Value> = Vec::new();
    // 模型资产（GLB）：sha256 与来源（Tripo 生成 vs 未标注）。
    let (_model_asset, model_blob) =
        repo::assets::get_with_blob(&mut *conn, &model_revision.asset_id)
            .await?
            .ok_or_else(|| {
                PublishError::integrity(
                    "release_model_asset_missing",
                    "模型资产不存在：发布清单无法记录 sha256",
                )
            })?;
    assets.push(json!({
        "role": "model",
        "assetId": model_revision.asset_id,
        "sha256": model_blob.sha256,
        "size": model_blob.size,
        "source": if model_revision.provider_attempt_id.is_some() { "tripo" } else { "unspecified" },
    }));

    // 说明书原件（PDF）：sha256 与来源（本地上传）。
    let document = repo::documents::get(&mut *conn, &frozen.document_id)
        .await?
        .ok_or_else(|| {
            PublishError::integrity(
                "release_document_missing",
                format!("说明书原件不存在：{}", frozen.document_id),
            )
        })?;
    let (_document_asset, document_blob) =
        repo::assets::get_with_blob(&mut *conn, &document.source_asset_id)
            .await?
            .ok_or_else(|| {
                PublishError::integrity(
                    "release_document_asset_missing",
                    "说明书原件资产不存在：发布清单无法记录 sha256",
                )
            })?;
    assets.push(json!({
        "role": "document",
        "assetId": document.source_asset_id,
        "sha256": document_blob.sha256,
        "size": document_blob.size,
        "source": "itemUpload",
    }));

    let parts = knowledge
        .knowledge
        .as_ref()
        .map(|merged| merged.parts.len())
        .unwrap_or(0);
    let steps = knowledge
        .knowledge
        .as_ref()
        .map(|merged| merged.steps.len())
        .unwrap_or(0);
    let specs = knowledge
        .knowledge
        .as_ref()
        .map(|merged| merged.specs.len())
        .unwrap_or(0);
    let confirmed_hotspots = knowledge
        .hotspots
        .iter()
        .filter(|hotspot| hotspot.status == crate::drafts::HotspotStatus::Confirmed)
        .count();
    let stale_hotspots = knowledge
        .hotspots
        .iter()
        .filter(|hotspot| hotspot.status.is_stale())
        .count();
    let text_only_parts: Vec<String> = overlay
        .entities
        .iter()
        .filter(|(_, entry)| entry.text_only)
        .map(|(id, _)| id.clone())
        .collect();

    Ok(json!({
        "schemaVersion": RELEASE_SCHEMA_VERSION,
        "releaseId": release_id,
        "itemId": draft.item_id,
        "draftId": draft.id,
        "draftRevision": draft.revision,
        "publishedAt": now.as_millis(),
        "model": {
            "revisionId": model_revision.id,
            "sha256": model_revision.sha256,
            "assetId": model_revision.asset_id,
            "validationState": model_revision.validation_state.as_str(),
            "bounds": model_revision.bounds,
        },
        // 冻结的草稿聚合（部件/步骤/规格/出处/热点/步骤视角/缺项；原样不可变）。
        "knowledge": knowledge,
        // 冻结的复核声明（实体确认/人工修订/仅文本条目/modelReview）。
        "review": overlay,
        "assets": assets,
        "documents": [{
            "documentId": document.id,
            "preparationId": frozen.preparation_id,
            "title": document.title,
            "sourceAssetId": document.source_asset_id,
            "sourceSha256": document.source_sha256,
            "sourceUrl": document.source_url,
        }],
        "counts": {
            "parts": parts,
            "steps": steps,
            "specs": specs,
            "hotspots": knowledge.hotspots.len(),
            "confirmedHotspots": confirmed_hotspots,
            "staleHotspots": stale_hotspots,
            "textOnlyParts": text_only_parts.len(),
            "textOnlyPartIds": text_only_parts,
        },
    }))
}

/// manifest 落盘（tmp → fsync → 原子 rename；与上传同一套 blob 安全语义）。
async fn stage_manifest(data_dir: &Path, bytes: &[u8]) -> Result<blob_store::Staged, PublishError> {
    let tmp = blob_store::tmp_dir(data_dir);
    let mut writer = blob_store::StagedWriter::create(&tmp, MANIFEST_MAX_BYTES, "release_manifest")
        .await
        .map_err(|error| {
            PublishError::integrity("release_manifest_write_failed", format!("{error:?}"))
        })?;
    writer.write(bytes).await.map_err(|error| {
        PublishError::integrity("release_manifest_write_failed", format!("{error:?}"))
    })?;
    let staged = writer.finish().await.map_err(|error| {
        PublishError::integrity("release_manifest_write_failed", format!("{error:?}"))
    })?;
    blob_store::promote(&staged, data_dir)
        .await
        .map_err(|error| {
            PublishError::integrity("release_manifest_write_failed", format!("{error:?}"))
        })?;
    Ok(staged)
}
