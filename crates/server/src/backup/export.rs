//! 发布版导出自包含包（T20 / REQ-037、AC-058；contracts.md §3/§7、PRD §5.7）。
//!
//! `GET /api/v1/releases/{releaseId}/export` 下载一个 ZIP 包：
//!
//! ```text
//! manifest.json                    导出清单：schemaVersion / item / release / 知识 /
//!                                  相对资产清单（path + sha256 + 来源 + 大小 + mime）
//! release/manifest.json            发布时冻结的 release manifest（字节原样；sha256 校验）
//! assets/model/<sha256>.glb        该 release 的模型（只此一个，不再包含草稿/其它资产）
//! assets/document/<sha256>.pdf     该 release 的说明书原件
//! ```
//!
//! 边界（本合同卡明确）：
//! - **只导出该 release 有权资产**：资产清单来自冻结 manifest 的 `assets[]`（model /
//!   document），不导出照片、页图、页文字、其它物品或 data-dir 里的任何其它文件；
//! - **不含密钥、会话、绝对路径、临时云端 URL**：包内没有数据库、没有签名 URL；
//!   所有路径都是包内相对路径；`sourceUrl`（用户填写の出处链接）属于"来源"，
//!   按 contracts §7 随 manifest 保留，不是供应商临时 URL；
//! - **不承诺双击运行网站**：首版导出是数据便携与灾备；本卡**不开放导入接口**
//!   （contracts §7："将来导入时必须另设 zip-slip／解压炸弹验收"）。
//!
//! 实现要点：ZIP 为 STORE 无压缩、固定时间戳（[`super::zip`]），同一 release 的
//! 导出包字节确定；每个资产在打包前先流式校验 sha256/大小（与冻结 manifest 一致），
//! 不一致就失败（不生成"看起来成功、内容已损坏"的包）。包先写到
//! `<data-dir>/tmp/export-<uuid>.zip` 再流式响应，**不在内存里缓存大资产**。

use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

use manual_core::domain::{AssetPurpose, BlobStorageState, ManualRelease};
use manual_core::timestamps::Timestamp;

use crate::assets::blob_store;
use crate::storage::StorageError;
use crate::storage::repo;

use super::error::BackupError;
use super::files::{fingerprint_file, hex_digest};
use super::manifest::validate_relative_path;
use super::zip::{ZipEntry, ZipEntrySource, write_stored_zip};

/// 导出清单的 schema 版本（与 release manifest 的 `manual_release_v1` 区分）。
pub const RELEASE_EXPORT_SCHEMA_VERSION: &str = "manual_release_export_v1";
/// 导出清单文件名（包根）。
pub const EXPORT_MANIFEST_NAME: &str = "manifest.json";
/// 冻结 release manifest 在包内的名字（字节原样）。
pub const EXPORT_RELEASE_MANIFEST_NAME: &str = "release/manifest.json";
/// 导出包 MIME。
pub const EXPORT_CONTENT_TYPE: &str = "application/zip";

/// 导出错误（HTTP 层映射：NotFound → 404，其余 → 500 + 服务端日志）。
#[derive(Debug, Clone, PartialEq)]
pub enum ExportError {
    /// release 不存在（不泄露存在性）。
    NotFound {
        message: String,
    },
    /// 服务端数据完整性错误（manifest/资产损坏、归属不符、文件缺失）。
    Integrity {
        code: &'static str,
        message: String,
    },
    /// 运行时 I/O 失败。
    Io {
        message: String,
    },
    Storage(StorageError),
}

impl ExportError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }

    pub fn integrity(code: &'static str, message: impl Into<String>) -> Self {
        Self::Integrity {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &str {
        match self {
            Self::NotFound { .. } => "release_not_found",
            Self::Integrity { code, .. } => code,
            Self::Io { .. } => "export_io",
            Self::Storage(_) => "export_storage",
        }
    }
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { message } | Self::Integrity { message, .. } | Self::Io { message } => {
                formatter.write_str(message)
            }
            Self::Storage(error) => write!(formatter, "存储错误：{error}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<StorageError> for ExportError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::Error> for ExportError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(StorageError::from(error))
    }
}

impl From<BackupError> for ExportError {
    fn from(error: BackupError) -> Self {
        match error {
            BackupError::Integrity { code, message } => Self::Integrity { code, message },
            other => Self::Io {
                message: other.to_string(),
            },
        }
    }
}

/// 生成好的导出包（临时文件；由 HTTP 层流式响应后在 Drop 时删除）。
#[derive(Debug)]
pub struct ExportPackage {
    /// 包文件在 `<data-dir>/tmp/` 下的路径（0600）。
    pub path: PathBuf,
    /// 包字节数。
    pub size: u64,
    /// 包字节的 sha256（诊断与测试断言用；同一 release 两次导出字节一致）。
    pub sha256: String,
    /// 包内条目名（按写入顺序）。
    pub entries: Vec<String>,
}

impl ExportPackage {
    /// 打开包的流式读取句柄；句柄被丢弃（响应完成或客户端断开）时删除临时文件。
    pub async fn open_reader(&self) -> std::io::Result<ExportPackageReader> {
        let file = tokio::fs::File::open(&self.path).await?;
        Ok(ExportPackageReader {
            file,
            path: self.path.clone(),
        })
    }
}

/// 导出包临时文件的读取句柄：`AsyncRead` + Drop 时删除临时文件。
///
/// 直接实现 `AsyncRead`（而不是用 `ReaderStream` 再包一层 stream 守卫），
/// 这样 HTTP 层继续用与资产内容服务相同的 `ReaderStream::new(...)` 组装响应体。
#[derive(Debug)]
pub struct ExportPackageReader {
    file: tokio::fs::File,
    path: PathBuf,
}

impl tokio::io::AsyncRead for ExportPackageReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.file).poll_read(cx, buffer)
    }
}

impl Drop for ExportPackageReader {
    fn drop(&mut self) {
        // 响应结束/连接中断后清理临时文件（best effort；残留由启动扫描隔离）。
        if let Err(error) = std::fs::remove_file(&self.path) {
            tracing::debug!(error = %error, path = %self.path.display(), "清理导出临时文件失败（将由启动扫描隔离）");
        }
    }
}

/// 生成过程中的临时文件守卫：失败时删除半写的包（成功路径由
/// [`ExportPackageReader`] 在响应结束后删除）。
struct TmpPackageGuard {
    path: Option<PathBuf>,
}

impl TmpPackageGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn defuse(&mut self) {
        self.path = None;
    }
}

impl Drop for TmpPackageGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 兜底校验（T20/BUG-008；fail-closed）：导出清单里**本程序生成的字段**
/// 不得出现 URL 形态字符串。若将来有人往导出清单加字段、并把供应商临时地址
/// 带进来，这里直接失败（在生成包之前），而不是把链接发出去。
///
/// 刻意排除用户内容：`knowledge` / `review`（含用户自己填写的 `sourceUrl` 出处链接）
/// 与 `item`（用户输入的物品身份字段）——按 contracts §7 属"来源/用户数据"，
/// 随包保留是要求而不是泄露（`implementation.md` §T20-10 第 8 条）。
fn ensure_no_urls_in_generated_fields(
    export_manifest: &serde_json::Value,
) -> Result<(), ExportError> {
    let mut generated = export_manifest.clone();
    if let Some(object) = generated.as_object_mut() {
        object.remove("knowledge");
        object.remove("review");
        object.remove("item");
    }
    if let Some(url) = crate::redaction::first_url_like(&generated) {
        return Err(ExportError::integrity(
            "export_manifest_url_forbidden",
            format!(
                "导出清单的生成字段里出现 URL 形态字符串（{} 个字符）：本程序不允许把\
                 临时/签名地址写进导出包；请升级程序或联系维护者（不静默输出）",
                url.chars().count()
            ),
        ));
    }
    Ok(())
}

/// 生成导出包（写入 `<data-dir>/tmp/`，返回临时文件）。
pub async fn build_release_export(
    conn: &mut SqliteConnection,
    data_dir: &Path,
    release: &ManualRelease,
) -> Result<ExportPackage, ExportError> {
    // 1) 冻结 release manifest（字节 + sha256 校验；json 供取 item/知识/资产清单）。
    let (_manifest_asset, manifest_blob) =
        repo::assets::get_with_blob(&mut *conn, &release.manifest_asset_id)
            .await?
            .ok_or_else(|| {
                ExportError::integrity(
                    "release_manifest_missing",
                    format!("发布清单资产不存在：{}", release.manifest_asset_id),
                )
            })?;
    let manifest_path = blob_store::blob_path(data_dir, &manifest_blob.sha256);
    let manifest_bytes = tokio::fs::read(&manifest_path).await.map_err(|error| {
        ExportError::integrity(
            "release_manifest_unreadable",
            format!("发布清单文件不可读：{error}"),
        )
    })?;
    let manifest_sha256 = hex_digest(&Sha256::digest(&manifest_bytes));
    if manifest_sha256 != manifest_blob.sha256 {
        return Err(ExportError::integrity(
            "release_manifest_hash_mismatch",
            "发布清单文件内容与其 sha256 不符（存储被外部改动？）",
        ));
    }
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        ExportError::integrity(
            "release_manifest_invalid",
            format!("发布清单不是合法 JSON：{error}"),
        )
    })?;

    // 2) 物品摘要（当前名称/品牌/型号；发布时未冻结这些字段，导出为"当前身份信息"）。
    let item = repo::items::get(&mut *conn, &release.item_id)
        .await?
        .ok_or_else(|| {
            ExportError::integrity(
                "release_item_missing",
                format!("发布版本所属物品不存在：{}", release.item_id),
            )
        })?;

    // 3) 资产白名单：只取冻结 manifest 里登记的 model / document。
    let asset_entries = manifest
        .get("assets")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            ExportError::integrity(
                "release_manifest_invalid",
                "发布清单缺少 assets[] 数组（无法确定要导出的资产）",
            )
        })?;
    let mut files: Vec<serde_json::Value> = Vec::new();
    let mut zip_files: Vec<(String, PathBuf, super::files::FileFingerprint)> = Vec::new();
    for entry in asset_entries {
        let role = entry
            .get("role")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                ExportError::integrity("release_manifest_invalid", "发布清单资产缺少 role")
            })?;
        let expected_sha256 = entry
            .get("sha256")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                ExportError::integrity("release_manifest_invalid", "发布清单资产缺少 sha256")
            })?;
        let asset_id = entry
            .get("assetId")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                ExportError::integrity("release_manifest_invalid", "发布清单资产缺少 assetId")
            })?;
        let extension = match role {
            "model" => "glb",
            "document" => "pdf",
            other => {
                return Err(ExportError::integrity(
                    "release_asset_role_unsupported",
                    format!(
                        "发布清单包含本程序不认识的资产角色 {other:?}：\
                         该版本可能由更新版本发布，无法安全导出（不静默丢弃资产）"
                    ),
                ));
            }
        };

        let (asset, blob) = repo::assets::get_with_blob(&mut *conn, asset_id)
            .await?
            .ok_or_else(|| {
                ExportError::integrity(
                    "release_asset_missing",
                    format!("发布清单引用的资产不存在：{asset_id}"),
                )
            })?;
        if asset.item_id != release.item_id {
            return Err(ExportError::integrity(
                "release_asset_foreign_item",
                format!("发布清单引用的资产不属于该物品（{asset_id}）：拒绝导出"),
            ));
        }
        let expected_purpose = match role {
            "model" => AssetPurpose::Model,
            _ => AssetPurpose::Document,
        };
        if asset.purpose != expected_purpose {
            return Err(ExportError::integrity(
                "release_asset_purpose_mismatch",
                format!("发布清单资产的用途与角色不符（{asset_id}）"),
            ));
        }
        if blob.sha256 != expected_sha256 {
            return Err(ExportError::integrity(
                "release_asset_hash_mismatch",
                format!(
                    "发布清单登记的 sha256 与资产的 blob 不符（{asset_id}）：\
                     期望 {expected_sha256}，实际 {actual}",
                    actual = blob.sha256
                ),
            ));
        }
        if blob.storage_state != BlobStorageState::Stored {
            return Err(ExportError::integrity(
                "release_asset_unavailable",
                format!(
                    "发布清单引用的资产当前不可用（{asset_id}）：状态 {:?}",
                    blob.storage_state
                ),
            ));
        }
        let path = blob_store::blob_path(data_dir, &blob.sha256);
        let fingerprint = fingerprint_file(&path).await.map_err(|_| {
            ExportError::integrity(
                "release_asset_file_missing",
                format!("发布清单引用的资产文件缺失（{asset_id}）"),
            )
        })?;
        if fingerprint.sha256 != expected_sha256 {
            return Err(ExportError::integrity(
                "release_asset_file_corrupt",
                format!(
                    "资产文件内容与登记 sha256 不符（{asset_id}）：\
                     期望 {expected_sha256}，实际 {actual}",
                    actual = fingerprint.sha256
                ),
            ));
        }

        let relative = format!("assets/{role}/{expected_sha256}.{extension}");
        validate_relative_path(&relative)
            .map_err(|error| ExportError::integrity("export_path_unsafe", error.to_string()))?;
        files.push(json!({
            "path": relative,
            "role": role,
            "assetId": asset_id,
            "sha256": expected_sha256,
            "size": fingerprint.size,
            "mime": blob.mime,
            "source": entry.get("source").and_then(|value| value.as_str()).unwrap_or("unspecified"),
        }));
        zip_files.push((relative, path, fingerprint));
    }
    if zip_files.is_empty() {
        return Err(ExportError::integrity(
            "release_assets_empty",
            "发布清单没有任何可导出的资产（原件/模型缺失）：拒绝生成空包",
        ));
    }

    // 4) 导出清单（含合同要求的 schemaVersion / item / release / 知识 / 相对资产清单 /
    //    sha256 / 来源；全部为相对路径与标识，不含密钥、会话、绝对路径、临时云端 URL）。
    let export_manifest = json!({
        "schemaVersion": RELEASE_EXPORT_SCHEMA_VERSION,
        "exportedAtMillis": Timestamp::now().as_millis(),
        "item": {
            "id": item.id,
            "name": item.name,
            "brand": item.brand,
            "model": item.model,
            "variant": item.variant,
        },
        "release": {
            "releaseId": release.id,
            "itemId": release.item_id,
            "draftId": release.draft_id,
            "draftRevision": release.draft_revision,
            "modelRevisionId": release.model_revision_id,
            "publishedAtMillis": release.created_at.as_millis(),
            "manifestSha256": manifest_blob.sha256,
        },
        // 冻结的知识与复核声明（发布时不可变的快照；原样随包导出以便离线审阅）。
        "knowledge": manifest.get("knowledge").cloned().unwrap_or(serde_json::Value::Null),
        "review": manifest.get("review").cloned().unwrap_or(serde_json::Value::Null),
        "releaseManifest": {
            "path": EXPORT_RELEASE_MANIFEST_NAME,
            "sha256": manifest_blob.sha256,
            "size": manifest_bytes.len(),
        },
        "files": files,
        "notes": [
            "本包是数据便携与灾备用途：包含该发布版本的原件、模型、清单与哈希",
            "不包含密钥、会话、绝对路径与临时云端 URL；所有路径都是包内相对路径",
            "不承诺可直接双击运行网站；当前版本不提供导入接口",
        ],
    });
    // 4b) 兜底校验（T20/BUG-008；fail-closed）：见 [`ensure_no_urls_in_generated_fields`]。
    ensure_no_urls_in_generated_fields(&export_manifest)?;
    let export_manifest_bytes = serde_json::to_vec_pretty(&export_manifest).map_err(|error| {
        ExportError::integrity(
            "export_manifest_serialization_failed",
            format!("导出清单序列化失败：{error}"),
        )
    })?;

    // 5) 打包（STORE；先写 `<data-dir>/tmp/`，再流式响应）。
    let tmp_dir = blob_store::tmp_dir(data_dir);
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|error| ExportError::Io {
            message: format!("创建导出临时目录失败：{error}"),
        })?;
    let package_path = tmp_dir.join(format!("export-{}.zip", uuid::Uuid::now_v7()));
    let mut entries: Vec<ZipEntry<'_>> = Vec::with_capacity(zip_files.len() + 2);
    entries.push(ZipEntry {
        name: EXPORT_MANIFEST_NAME.to_owned(),
        source: ZipEntrySource::Bytes(&export_manifest_bytes),
    });
    entries.push(ZipEntry {
        name: EXPORT_RELEASE_MANIFEST_NAME.to_owned(),
        source: ZipEntrySource::Bytes(&manifest_bytes),
    });
    for (name, path, fingerprint) in &zip_files {
        entries.push(ZipEntry {
            name: name.clone(),
            source: ZipEntrySource::File { path, fingerprint },
        });
    }

    let file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&package_path)
        .await
        .map_err(|error| ExportError::Io {
            message: format!("创建导出包失败：{error}"),
        })?;
    super::files::set_owner_only_permissions(&package_path);
    // 失败（含本函数后续校验失败）时删除半写的包，不把坏包留在 tmp/。
    let mut guard = TmpPackageGuard::new(package_path.clone());
    let written = write_stored_zip(file, &entries)
        .await
        .map_err(ExportError::from)?;

    // 6) 包指纹（诊断/测试；同时确认包已完整落盘）。
    let fingerprint = fingerprint_file(&package_path)
        .await
        .map_err(ExportError::from)?;
    if fingerprint.size != written {
        return Err(ExportError::Io {
            message: format!(
                "导出包写入字节数不一致（写入 {written}，实际 {}）：已中止",
                fingerprint.size
            ),
        });
    }
    let entry_names = entries.iter().map(|entry| entry.name.clone()).collect();
    guard.defuse();
    tracing::info!(
        event = "release_export_built",
        releaseId = %release.id,
        itemId = %release.item_id,
        packageBytes = fingerprint.size,
        packageSha256 = %fingerprint.sha256,
        entries = entries.len(),
        "发布版导出包已生成（只含该 release 的原件/GLB/清单）"
    );
    Ok(ExportPackage {
        path: package_path,
        size: fingerprint.size,
        sha256: fingerprint.sha256,
        entries: entry_names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_constants_are_relative_paths() {
        validate_relative_path(EXPORT_MANIFEST_NAME).unwrap();
        validate_relative_path(EXPORT_RELEASE_MANIFEST_NAME).unwrap();
        assert_eq!(EXPORT_CONTENT_TYPE, "application/zip");
    }

    /// T20/BUG-008：生成字段出现 URL → 失败；用户内容（knowledge/review/item）不拦。
    #[test]
    fn generated_export_fields_must_not_contain_urls() {
        let clean = json!({
            "schemaVersion": RELEASE_EXPORT_SCHEMA_VERSION,
            "release": {"releaseId": "r1", "manifestSha256": "a".repeat(64)},
            "files": [{"path": "assets/model/aa.glb", "role": "model", "source": "tripo"}],
            "knowledge": {"sourceUrl": "https://user.example.com/spec?sig=x"},
            "review": {"notes": "https://user.example.com/other"},
            "item": {"name": "https://user.example.com/typed-as-name"},
            "notes": ["不包含临时云端 URL"],
        });
        ensure_no_urls_in_generated_fields(&clean).expect("用户内容里的 URL 不拦截");

        // 未来字段把临时地址带进来 → 明确失败（不静默输出）。
        let mut leaked = clean.clone();
        leaked["files"][0]["source"] = json!("https://cdn.example.invalid/model.glb?sign=x");
        let error = ensure_no_urls_in_generated_fields(&leaked).unwrap_err();
        assert_eq!(error.code(), "export_manifest_url_forbidden");
        assert!(!error.to_string().contains("sign=x"), "错误消息不回显 URL");
    }
}
