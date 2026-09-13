//! 上传服务：暂存 → 校验 → 落盘 → 短事务提交元数据（REQ-011）。
//!
//! [`AssetStore`] 是路由层持有的资产服务句柄（data-dir + 可注入的剩余空间探测）。
//! HTTP handler 负责 multipart 字段解析与流读取，本模块负责"一份内容如何变成
//! blob 文件 + blob/asset 两行元数据"，以及所有校验与错误分类。
//!
//! 关键不变量（QA 按此复核）：
//! - **先文件、后元数据**：只有 rename 到 `blobs/<前缀>/<sha256>` 之后才提交元数据；
//! - **元数据失败不删除文件**：可能是被其他 asset 引用的共享 blob（AC-019）；
//! - **去重**：同 sha256 复用同一 blob 行与同一文件（`ON CONFLICT DO NOTHING`）；
//! - **原文件名只是元数据**：见 [`sanitize_original_name`]；
//! - **失败路径丢弃自己的 tmp 文件**，不留半提交资产。

use std::path::{Path, PathBuf};

use manual_core::domain::{Asset, AssetPurpose, Blob};
use sqlx::SqliteConnection;

use crate::config::Limits;
use crate::storage::StorageError;
use crate::storage::repo::{assets as assets_repo, blobs as blobs_repo};

use super::blob_store::{self, SpaceProbe, Staged};
use super::error::AssetError;
use super::{validate, validate::Detected};

/// 资产服务句柄（`AppState` 持有；测试可注入 [`SpaceProbe`]）。
#[derive(Debug, Clone)]
pub struct AssetStore {
    data_dir: PathBuf,
    space: SpaceProbe,
}

impl AssetStore {
    /// 生产构造：真实 `statvfs` 空间探测。
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            space: SpaceProbe::Statvfs,
        }
    }

    /// 测试/运维构造：注入空间探测（磁盘满场景可测，见 AC-019）。
    pub fn with_space_probe(data_dir: impl Into<PathBuf>, space: SpaceProbe) -> Self {
        Self {
            data_dir: data_dir.into(),
            space,
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn space_probe(&self) -> &SpaceProbe {
        &self.space
    }

    /// 上传暂存目录（`<data-dir>/tmp`）。
    pub fn tmp_dir(&self) -> PathBuf {
        blob_store::tmp_dir(&self.data_dir)
    }

    /// 文件内容的最终路径（只用 sha256 拼路径，见 [`blob_store::blob_path`]）。
    pub fn blob_path(&self, sha256: &str) -> PathBuf {
        blob_store::blob_path(&self.data_dir, sha256)
    }

    /// 解析请求体之前的最低空间预检（`Content-Length` 已知时；chunked 由落盘前复检兜底）。
    pub fn ensure_request_space(&self, content_length: Option<u64>) -> Result<(), AssetError> {
        let Some(required) = content_length.filter(|bytes| *bytes > 0) else {
            return Ok(());
        };
        blob_store::ensure_space(&self.space, &self.data_dir, required)
    }

    /// 完成一次上传：校验 → 物品累计 → 落盘前复检 → 原子落盘 → 元数据短事务。
    pub async fn finalize(
        &self,
        conn: &mut SqliteConnection,
        request: &UploadRequest<'_>,
        limits: &Limits,
        staged: Staged,
    ) -> Result<UploadOutcome, AssetError> {
        let detected = match validate::validate(&staged.path, request.purpose, limits) {
            Ok(detected) => detected,
            Err(error) => {
                blob_store::discard_staged(staged).await;
                return Err(error);
            }
        };

        let current_total = match assets_repo::item_total_bytes(conn, request.item_id).await {
            Ok(total) => total,
            Err(error) => {
                blob_store::discard_staged(staged).await;
                return Err(error.into());
            }
        };
        if current_total.saturating_add(staged.size) > limits.max_item_total_bytes {
            let error = AssetError::ItemTotalExceeded {
                limit: limits.max_item_total_bytes,
                current: current_total,
                incoming: staged.size,
            };
            blob_store::discard_staged(staged).await;
            return Err(error);
        }

        // 落盘前复检：chunked/未知长度上传在这里才第一次判定空间。
        if let Err(error) = blob_store::ensure_space(&self.space, &self.data_dir, staged.size) {
            blob_store::discard_staged(staged).await;
            return Err(error);
        }

        if let Err(error) = blob_store::promote(&staged, &self.data_dir).await {
            blob_store::discard_staged(staged).await;
            return Err(error);
        }

        // 从这里开始：文件已在内容寻址位置。元数据失败**不删除文件**（共享 blob 安全），
        // 也不需要删除：孤儿文件由启动扫描按引用隔离/收敛。
        self.commit_metadata(conn, request, &staged, detected).await
    }

    async fn commit_metadata(
        &self,
        conn: &mut SqliteConnection,
        request: &UploadRequest<'_>,
        staged: &Staged,
        detected: Detected,
    ) -> Result<UploadOutcome, AssetError> {
        // 写事务统一 `BEGIN IMMEDIATE`（`storage::tx`，BUG-006）：首条语句虽已是写，
        // 但后续语句顺序变化不该把本事务悄悄退回"读→写升级"的立即失败语义。
        let mut transaction = crate::storage::begin_write(conn)
            .await
            .map_err(|error| AssetError::io(format!("开启元数据事务失败：{error}")))?;

        let inserted = blobs_repo::insert_if_absent(
            &mut transaction,
            &staged.sha256,
            staged.size,
            detected.mime,
        )
        .await?;

        let blob = blobs_repo::get(&mut transaction, &staged.sha256)
            .await?
            .ok_or_else(|| AssetError::io("blob 元数据插入后读取失败".to_owned()))?;

        match blob.storage_state {
            manual_core::domain::BlobStorageState::Stored => {}
            manual_core::domain::BlobStorageState::Missing => {
                // 元数据在、文件不在：本次上传补齐了内容 → 收敛回 stored。
                blobs_repo::set_storage_state(
                    &mut transaction,
                    &staged.sha256,
                    manual_core::domain::BlobStorageState::Stored,
                )
                .await?;
            }
            manual_core::domain::BlobStorageState::Quarantined => {
                // 内容与已隔离 blob 相同：不静默解隔离，也不删除文件（保留现场）。
                return Err(AssetError::invalid_content(
                    "该内容与此前被隔离的 blob 相同：请管理员先处理隔离记录后再上传",
                ));
            }
        }

        let asset = match assets_repo::insert(
            &mut transaction,
            assets_repo::NewAsset {
                blob_id: staged.sha256.clone(),
                item_id: request.item_id.to_owned(),
                purpose: request.purpose,
                original_name: request.original_name.clone(),
            },
        )
        .await
        {
            Ok(asset) => asset,
            Err(StorageError::ForeignKeyViolation { detail }) => {
                tracing::warn!(detail = %detail, "资产归属的物品不存在（外键拒绝）");
                return Err(AssetError::not_found(format!(
                    "item 不存在：{}",
                    request.item_id
                )));
            }
            Err(error) => return Err(error.into()),
        };

        transaction
            .commit()
            .await
            .map_err(|error| AssetError::io(format!("提交元数据事务失败：{error}")))?;

        Ok(UploadOutcome {
            asset,
            blob: Blob {
                storage_state: manual_core::domain::BlobStorageState::Stored,
                ..blob
            },
            reused_existing_blob: !inserted,
        })
    }
}

/// 一次上传的请求信息（来自 multipart 字段，已在 handler 内解析/校验）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRequest<'a> {
    pub item_id: &'a str,
    pub purpose: AssetPurpose,
    pub original_name: Option<String>,
}

/// 上传成功的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadOutcome {
    pub asset: Asset,
    pub blob: Blob,
    /// true = 命中已有 sha256（内容去重），没有新建 blob 行。
    pub reused_existing_blob: bool,
}

/// 解析 multipart 的 `purpose` 字段值（线上 camelCase，contracts.md §1/§3）。
///
/// 只接受本路由的取值集合（document/photo/pageImage/pageText）；`model` 由 T13 的
/// 模型下载通道产生，**不**允许通过上传接口伪造。
pub fn parse_purpose(value: &str) -> Result<AssetPurpose, AssetError> {
    match value.trim() {
        "document" => Ok(AssetPurpose::Document),
        "photo" => Ok(AssetPurpose::Photo),
        "pageImage" => Ok(AssetPurpose::PageImage),
        "pageText" => Ok(AssetPurpose::PageText),
        other => Err(AssetError::invalid_content(format!(
            "purpose 非法：{other:?}（只接受 document / photo / pageImage / pageText）"
        ))),
    }
}

/// 原文件名 → 只作元数据的安全字符串（REQ-011：文件名不参与路径拼接）。
///
/// 规则：去掉任何目录成分（`/`、`\` 两种分隔符都算）、去掉控制字符、截断到 255 字符；
/// 结果为空则记 `None`（例如文件名只由 `../` 组成）。
pub fn sanitize_original_name(name: Option<&str>) -> Option<String> {
    let name = name?;
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .trim_matches(|character: char| character.is_control())
        .trim();
    let cleaned: String = base
        .chars()
        .filter(|character| !character.is_control())
        .take(255)
        .collect();
    let cleaned = cleaned.trim().to_owned();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        None
    } else {
        Some(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purpose_wire_values_are_camel_case() {
        assert_eq!(parse_purpose("document").unwrap(), AssetPurpose::Document);
        assert_eq!(parse_purpose("pageImage").unwrap(), AssetPurpose::PageImage);
        assert_eq!(parse_purpose("pageText").unwrap(), AssetPurpose::PageText);
        // SQL 风格值不是线上值；model 不允许由上传接口产生。
        assert!(parse_purpose("page_image").is_err());
        assert!(parse_purpose("model").is_err());
        assert!(parse_purpose("").is_err());
    }

    #[test]
    fn original_name_never_keeps_path_components() {
        assert_eq!(
            sanitize_original_name(Some("../../etc/passwd.pdf")).as_deref(),
            Some("passwd.pdf")
        );
        assert_eq!(
            sanitize_original_name(Some("/tmp/evil.pdf")).as_deref(),
            Some("evil.pdf")
        );
        assert_eq!(
            sanitize_original_name(Some("..\\..\\windows\\evil.pdf")).as_deref(),
            Some("evil.pdf")
        );
        assert_eq!(
            sanitize_original_name(Some("说明书 扫描件.pdf")).as_deref(),
            Some("说明书 扫描件.pdf")
        );
        assert_eq!(sanitize_original_name(Some("../../")), None);
        assert_eq!(sanitize_original_name(Some("")), None);
        assert_eq!(sanitize_original_name(None), None);
        let long = "a".repeat(400);
        assert_eq!(
            sanitize_original_name(Some(&long)).unwrap().chars().count(),
            255
        );
    }
}
