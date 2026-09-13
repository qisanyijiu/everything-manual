//! 批次结果的持久化（“先持久化响应结果资产，再在同一短事务保存 usage/receipt”，
//! contracts.md §5）。
//!
//! 顺序（两条独立步骤，崩溃语义见 T10 恢复矩阵）：
//!
//! 1. **结果资产**：内容寻址落盘（`blobs/<前 2 位>/<sha256>`）+ 短事务提交
//!    `blobs`/`assets` 元数据（先文件、后元数据；与 T06 上传同构）；
//! 2. 调用方随后把 `result_asset_id`/`usage`/receipt 写入（提取：`record_sync_response`；
//!    合并：`record_result_fact`）。
//!
//! 资产 purpose 说明（**已知的语义妥协，见 implementation §T14 与 ADR-024**）：
//! 冻结的 `assets.purpose` CHECK 只有 `document/photo/page_image/page_text/model`，
//! 没有"批次结果"专用值；新增取值需要重建 `assets` 表（迁移），而本切片不追加
//! 会改变 schema 版本的迁移（`tests/storage.rs` / `tests/config_cli.rs` 按 v6 冻结，
//! 不在本卡允许写入范围内）。因此批次结果复用 `page_text`（"由页派生的文本内容"）
//! ——这也是 T10 测试设施 `seed_result_asset` 的既有约定
//! （`purpose: AssetPurpose::PageText`, `mime: application/json`）。
//! 批次结果资产不参与任何按 purpose 的既有查询（`pages.text_asset_id` 按 id 引用、
//! `page_quote_inputs` 按 id JOIN），因此不影响页/报价语义。
//!
//! 受限诊断路径：失败路径与成功路径的**原始响应字节**都以 `blob` 形式保存
//! （内容寻址；sha256 记录在批次结果 JSON 的 `diagnosticSha256`），不出现在任何
//! HTTP DTO 中，只供管理员/后续卡在 data-dir 内经引用核对。

use std::path::Path;

use manual_core::domain::AssetPurpose;
use sqlx::{SqliteConnection, SqlitePool};

use crate::assets::blob_path;
use crate::assets::blob_store::{StagedWriter, promote, tmp_dir};
use crate::storage::repo;

/// 派生资产（批次结果 / 诊断）的字节上限（远大于任何合法提取结果）。
pub const MAX_DERIVED_ASSET_BYTES: u64 = 8 * 1024 * 1024;

/// 已持久化的派生资产。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedAsset {
    pub asset_id: String,
    pub sha256: String,
    pub size: u64,
}

/// 派生资产的失败（映射到 `needs_input` 或内部错误，由调用方决定）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedAssetError {
    pub code: &'static str,
    pub detail: String,
}

impl DerivedAssetError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for DerivedAssetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}：{}", self.code, self.detail)
    }
}

impl std::error::Error for DerivedAssetError {}

/// 内容寻址写入 + 提交 `blobs`/`assets` 元数据（短事务；先文件后元数据）。
///
/// 幂等：同一内容重复写入命中同一 blob（`insert_if_absent`），但会各产生一行
/// `assets`（归属行）；调用方应先查 [`existing_result`] 避免重复派生。
pub async fn persist_derived_asset(
    pool: &SqlitePool,
    data_dir: &Path,
    item_id: &str,
    bytes: &[u8],
    original_name: &str,
) -> Result<DerivedAsset, DerivedAssetError> {
    if bytes.is_empty() {
        return Err(DerivedAssetError::new(
            "manual_derived_asset_empty",
            "派生资产内容为空：不写入",
        ));
    }
    if bytes.len() as u64 > MAX_DERIVED_ASSET_BYTES {
        return Err(DerivedAssetError::new(
            "manual_derived_asset_too_large",
            format!(
                "派生资产 {} 字节超过上限 {MAX_DERIVED_ASSET_BYTES} 字节",
                bytes.len()
            ),
        ));
    }

    // 1) 内容寻址落盘（tmp → fsync → 原子 rename）。
    let mut writer = StagedWriter::create(&tmp_dir(data_dir), MAX_DERIVED_ASSET_BYTES, "manual_ai")
        .await
        .map_err(|error| {
            DerivedAssetError::new(
                "manual_derived_asset_io",
                format!("创建暂存失败：{error:?}"),
            )
        })?;
    writer.write(bytes).await.map_err(|error| {
        DerivedAssetError::new(
            "manual_derived_asset_io",
            format!("写入暂存失败：{error:?}"),
        )
    })?;
    let staged = writer.finish().await.map_err(|error| {
        DerivedAssetError::new("manual_derived_asset_io", format!("落盘失败：{error:?}"))
    })?;
    let sha256 = staged.sha256.clone();
    let size = staged.size;
    promote(&staged, data_dir).await.map_err(|error| {
        DerivedAssetError::new(
            "manual_derived_asset_io",
            format!("原子移动失败：{error:?}"),
        )
    })?;

    // 2) 短事务提交元数据（blobs 幂等插入 + assets 归属行）。
    let mut conn = pool.acquire().await.map_err(|error| {
        DerivedAssetError::new("manual_derived_asset_db", format!("获取连接失败：{error}"))
    })?;
    // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）。
    let mut transaction = crate::storage::begin_write(&mut conn)
        .await
        .map_err(|error| {
            DerivedAssetError::new("manual_derived_asset_db", format!("开启事务失败：{error}"))
        })?;
    repo::blobs::insert_if_absent(&mut transaction, &sha256, size, "application/json")
        .await
        .map_err(|error| {
            DerivedAssetError::new(
                "manual_derived_asset_db",
                format!("blob 元数据失败：{error}"),
            )
        })?;
    let blob = repo::blobs::get(&mut transaction, &sha256)
        .await
        .map_err(|error| {
            DerivedAssetError::new("manual_derived_asset_db", format!("blob 读取失败：{error}"))
        })?
        .ok_or_else(|| {
            DerivedAssetError::new("manual_derived_asset_db", "blob 元数据插入后读取不到")
        })?;
    match blob.storage_state {
        manual_core::domain::BlobStorageState::Stored => {}
        manual_core::domain::BlobStorageState::Missing => {
            repo::blobs::set_storage_state(
                &mut transaction,
                &sha256,
                manual_core::domain::BlobStorageState::Stored,
            )
            .await
            .map_err(|error| {
                DerivedAssetError::new(
                    "manual_derived_asset_db",
                    format!("blob 状态收敛失败：{error}"),
                )
            })?;
        }
        manual_core::domain::BlobStorageState::Quarantined => {
            return Err(DerivedAssetError::new(
                "manual_derived_asset_quarantined",
                "该内容与此前被隔离的 blob 相同：请管理员先处理隔离记录（不静默解隔离）",
            ));
        }
    }
    let asset = repo::assets::insert(
        &mut transaction,
        repo::assets::NewAsset {
            blob_id: sha256.clone(),
            item_id: item_id.to_owned(),
            // 见模块文档：purpose 复用页派生文本（冻结 CHECK 无批次结果取值）。
            purpose: AssetPurpose::PageText,
            original_name: Some(original_name.to_owned()),
        },
    )
    .await
    .map_err(|error| {
        DerivedAssetError::new("manual_derived_asset_db", format!("资产行失败：{error}"))
    })?;
    transaction.commit().await.map_err(|error| {
        DerivedAssetError::new("manual_derived_asset_db", format!("提交失败：{error}"))
    })?;
    Ok(DerivedAsset {
        asset_id: asset.id,
        sha256,
        size,
    })
}

/// 已有的结果资产（重跑合并/恢复时**优先复用**：不重复派生、不产生第二行）。
pub async fn existing_result(
    pool: &SqlitePool,
    data_dir: &Path,
    asset_id: &str,
) -> Option<DerivedAsset> {
    let mut conn = pool.acquire().await.ok()?;
    let (_asset, blob) = repo::assets::get_with_blob(&mut conn, asset_id)
        .await
        .ok()??;
    if blob.storage_state != manual_core::domain::BlobStorageState::Stored {
        return None;
    }
    if !blob_path(data_dir, &blob.sha256).is_file() {
        return None;
    }
    Some(DerivedAsset {
        asset_id: asset_id.to_owned(),
        sha256: blob.sha256,
        size: blob.size as u64,
    })
}

/// 读取资产内容（资产 → blob → 文件）。
pub async fn read_asset_bytes(
    pool: &SqlitePool,
    data_dir: &Path,
    asset_id: &str,
) -> Result<Vec<u8>, DerivedAssetError> {
    let mut conn = pool.acquire().await.map_err(|error| {
        DerivedAssetError::new("manual_blob_db", format!("获取连接失败：{error}"))
    })?;
    let (asset, blob) = repo::assets::get_with_blob(&mut conn, asset_id)
        .await
        .map_err(|error| {
            DerivedAssetError::new("manual_blob_db", format!("资产读取失败：{error}"))
        })?
        .ok_or_else(|| DerivedAssetError::new("manual_blob_missing", "资产不存在"))?;
    let _ = asset;
    read_blob_bytes(&mut conn, data_dir, &blob.sha256).await
}

/// 读取 blob 内容（校验状态与文件存在性）。
pub async fn read_blob_bytes(
    conn: &mut SqliteConnection,
    data_dir: &Path,
    sha256: &str,
) -> Result<Vec<u8>, DerivedAssetError> {
    let blob = repo::blobs::get(&mut *conn, sha256)
        .await
        .map_err(|error| {
            DerivedAssetError::new("manual_blob_db", format!("blob 读取失败：{error}"))
        })?
        .ok_or_else(|| DerivedAssetError::new("manual_blob_missing", "blob 元数据不存在"))?;
    if blob.storage_state != manual_core::domain::BlobStorageState::Stored {
        return Err(DerivedAssetError::new(
            "manual_blob_unavailable",
            format!("blob 状态异常（{}）", blob.storage_state.as_str()),
        ));
    }
    tokio::fs::read(blob_path(data_dir, sha256))
        .await
        .map_err(|error| {
            DerivedAssetError::new("manual_blob_missing", format!("blob 文件不可读：{error}"))
        })
}
