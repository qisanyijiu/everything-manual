//! 资产 DTO（REQ-011；contracts.md §3）。
//!
//! 响应形状 `{ "data": … }`（contracts.md §1）；**不返回磁盘路径**：
//! 客户端只需要 id/sha256/体积/类型/存储状态，文件位置是服务器内部事实。
//! `purpose` / `storageState` 的线上值：camelCase 枚举（`pageImage`、`stored`），
//! 与 SQL 的 snake_case 取值不同（contracts.md §1，ADR-012 结论 2）。

use manual_core::domain::{Asset, AssetPurpose, Blob, BlobStorageState};
use manual_core::timestamps::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 单个资产响应。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetResponse {
    pub data: AssetDto,
}

/// 资产的线上表示。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssetDto {
    pub id: String,
    pub item_id: String,
    #[schema(value_type = String, example = "photo")]
    pub purpose: AssetPurpose,
    /// 原文件名（仅元数据；不含任何目录成分，见 `assets::upload::sanitize_original_name`）。
    #[schema(nullable = true)]
    pub original_name: Option<String>,
    /// 内容 sha256（内容寻址的主键；同内容不同 asset 共享同一值）。
    pub sha256: String,
    /// 字节数。
    pub size: i64,
    /// 服务器判定的 MIME（来自实测内容，不采信客户端声明）。
    #[schema(example = "image/jpeg")]
    pub mime: String,
    #[schema(value_type = String, example = "stored")]
    pub storage_state: BlobStorageState,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
}

impl AssetDto {
    /// 由领域对象组装（asset 行 + 它引用的 blob 行）。
    pub fn from_parts(asset: &Asset, blob: &Blob) -> Self {
        Self {
            id: asset.id.clone(),
            item_id: asset.item_id.clone(),
            purpose: asset.purpose,
            original_name: asset.original_name.clone(),
            sha256: blob.sha256.clone(),
            size: blob.size,
            mime: blob.mime.clone(),
            storage_state: blob.storage_state,
            created_at: asset.created_at,
        }
    }
}

/// `POST /items/{id}/assets` 的 `multipart/form-data` 请求体（仅用于 OpenAPI 描述）。
///
/// 实际解析由 `http::assets` 的流式 multipart 完成；这里声明字段形状，
/// 让前端生成的类型与真实请求一致。
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssetUploadRequest {
    /// 文件字节（流式读取；不把整个文件放进内存）。
    #[schema(value_type = String, format = Binary)]
    pub file: Vec<u8>,
    /// 用途：document / photo / pageImage / pageText。
    #[schema(example = "document")]
    pub purpose: String,
}
