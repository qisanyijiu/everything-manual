//! 发布版本 DTO（T19 / REQ-035、REQ-036；contracts.md §2/§3/§7）。
//!
//! - `POST /items/{id}/drafts/{draftId}/publish` → 201 不可变 release（本文件）；
//! - `GET /items/{id}/releases` → 版本列表（发布时间倒序；U-03 服务端排序）；
//! - `GET /items/{id}/releases/{releaseId}` → 完整 manifest（`knowledge` / `review` /
//!   `assets` / `documents` 冻结内容 + 引用资产的 sha256 与来源）。
//!
//! manifest 是版本化 JSON 聚合（`manual_release_v1`），在 OpenAPI 中保持开放结构
//! （与 `DraftDto.knowledge` 同一理由：schema 由版本常量管理，不由生成器展开）；
//! 其中的 `model.assetId` 指向**本地**资产，不出现临时云端 URL、绝对路径或密钥。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use manual_core::domain::ManualRelease;
use manual_core::timestamps::Timestamp;

/// 发布动作/读取的固定语义说明（响应携带；UI 直接展示）。
pub const RELEASE_NOTICES: [&str; 2] = [
    "已发布版本不可再修改：之后对草稿的修改不会改变该版本",
    "模型 URL 指向本地资产；导出/阅读不依赖外部服务",
];

/// 发布版本（列表与创建响应的形状；详情在 [`ReleaseDetailDto`] 扩展 manifest）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseDto {
    pub id: String,
    pub item_id: String,
    pub draft_id: String,
    /// 发布时草稿的 revision（之后修改草稿不改变该版本）。
    pub draft_revision: i64,
    pub model_revision_id: String,
    pub manifest_asset_id: String,
    /// 发布后草稿的下一个 revision（并发发布/编辑会得到 412 的依据）。
    #[schema(nullable = true)]
    pub draft_revision_after_publish: Option<i64>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    /// 固定语义说明（不可变、模型指向本地资产）。
    pub notices: Vec<String>,
}

impl ReleaseDto {
    pub fn from(release: &ManualRelease, draft_revision_after_publish: Option<i64>) -> Self {
        Self {
            id: release.id.clone(),
            item_id: release.item_id.clone(),
            draft_id: release.draft_id.clone(),
            draft_revision: release.draft_revision,
            model_revision_id: release.model_revision_id.clone(),
            manifest_asset_id: release.manifest_asset_id.clone(),
            draft_revision_after_publish,
            created_at: release.created_at,
            notices: RELEASE_NOTICES
                .iter()
                .map(|note| (*note).to_owned())
                .collect(),
        }
    }
}

/// 发布详情：在摘要之上携带冻结 manifest 与 manifest 内容的 sha256。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseDetailDto {
    pub id: String,
    pub item_id: String,
    pub draft_id: String,
    pub draft_revision: i64,
    pub model_revision_id: String,
    pub manifest_asset_id: String,
    /// manifest 资产的 sha256（字节比对用：发布后修改草稿不改变它）。
    pub manifest_sha256: String,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    /// 冻结 manifest（`manual_release_v1`：model / knowledge / review / assets / documents / counts）。
    #[schema(value_type = serde_json::Value)]
    pub manifest: serde_json::Value,
    pub notices: Vec<String>,
}

impl ReleaseDetailDto {
    pub fn from_detail(
        release: &ManualRelease,
        manifest_sha256: String,
        manifest: serde_json::Value,
    ) -> Self {
        Self {
            id: release.id.clone(),
            item_id: release.item_id.clone(),
            draft_id: release.draft_id.clone(),
            draft_revision: release.draft_revision,
            model_revision_id: release.model_revision_id.clone(),
            manifest_asset_id: release.manifest_asset_id.clone(),
            manifest_sha256,
            created_at: release.created_at,
            manifest,
            notices: RELEASE_NOTICES
                .iter()
                .map(|note| (*note).to_owned())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReleaseResponse {
    pub data: ReleaseDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReleaseDetailResponse {
    pub data: ReleaseDetailDto,
}

/// `GET /items/{id}/releases` 列表响应（`{data, nextCursor}`；MVP 一次给全，`
/// nextCursor` 为 null——版本数由发布次数决定，量级远小于分页阈值）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseListResponse {
    pub data: Vec<ReleaseDto>,
    #[schema(nullable = true)]
    pub next_cursor: Option<String>,
}
