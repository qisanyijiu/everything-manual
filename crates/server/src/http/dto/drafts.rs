//! 草稿 DTO（T15 读取 + T19 受限字段 PATCH；contracts.md §2/§3）。
//!
//! - `GET /items/{id}/drafts/{draftId}`：版本化知识、模型、复核状态 + `ETag: "r<n>"`；
//! - `PATCH`：**受限字段集**（status / hotspots / stepPoses / entities / modelReview），
//!   服务端逐字段校验（引用存在、数值有限、旧 sha 拒绝、供应商快照只读）；
//!   未知字段一律 422（`deny_unknown_fields`，不提供绕过校验的入口）；
//! - `POST .../publish`：发布（If-Match + Idempotency-Key）→ 201 不可变 release 或
//!   422 不变量明细（见 `http::dto::releases`）；
//! - 语义提示固定携带：`needs_review` 不等于已发布，**不存在自动发布路径**（ADR-005）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use manual_core::knowledge::ReviewStatus;
use manual_core::timestamps::Timestamp;

use crate::drafts::aggregate::{DraftPatch, EntityReviewPatch};

/// 聚合线上类型（PATCH 请求的嵌套结构；服务端语义类型与存储形状同源，
/// 在 `drafts::aggregate` 定义并在此 re-export，避免手抄两份）。
pub use crate::drafts::aggregate::{
    Anchor, CameraPose, HotspotPatch, HotspotStatus, HotspotUpsert, ModelReviewPatch, UserEdit,
};

/// 草稿状态（线上 snake_case：`needs_review` / `ready`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatusDto {
    /// 生成完成后的默认状态：需要人工复核（不等于已发布）。
    NeedsReview,
    /// 人工把草稿标记为复核完成（仍不触发发布；发布是 T19 的显式动作）。
    Ready,
}

impl DraftStatusDto {
    pub const fn from_domain(status: manual_core::domain::DraftStatus) -> Self {
        match status {
            manual_core::domain::DraftStatus::NeedsReview => Self::NeedsReview,
            manual_core::domain::DraftStatus::Ready => Self::Ready,
        }
    }

    pub const fn to_domain(self) -> manual_core::domain::DraftStatus {
        match self {
            Self::NeedsReview => manual_core::domain::DraftStatus::NeedsReview,
            Self::Ready => manual_core::domain::DraftStatus::Ready,
        }
    }
}

/// `GET /items/{id}/drafts/{draftId}` 响应（带 `ETag`）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DraftResponse {
    pub data: DraftDto,
}

/// 版本化草稿。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftDto {
    pub id: String,
    pub item_id: String,
    pub snapshot_id: String,
    /// 模型分支产出的不可变版本（部分草稿可能为 null）。
    #[schema(nullable = true)]
    pub model_revision_id: Option<String>,
    /// 聚合 revision（`ETag: "r<n>"`；PATCH 用 If-Match）。
    pub revision: i64,
    /// `needs_review` / `ready`（**都不等于已发布**）。
    pub status: DraftStatusDto,
    /// 完备性：`complete` / `partial`（部分成功可展示）。
    pub completeness: String,
    /// 缺项清单（`partial` 时非空；代码稳定，供 UI 给"去补齐"入口）。
    pub missing: Vec<DraftMissingItemDto>,
    /// 版本化知识聚合（外壳 `schemaVersion` + 合并结果；结构见 `drafts::knowledge`）。
    #[schema(value_type = serde_json::Value)]
    pub knowledge: serde_json::Value,
    /// 复核状态（modelReview 等；T15 尚未写入，T19 起使用）。
    #[schema(value_type = Option<serde_json::Value>, nullable = true)]
    pub review: Option<serde_json::Value>,
    /// 固定语义说明（生成完成 ≠ 已发布；不存在自动发布）。
    pub notices: Vec<String>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

/// 草稿缺项。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftMissingItemDto {
    pub code: String,
    pub message: String,
}

/// 线上复核状态（与 core `ReviewStatus` 同值集；core 不依赖 utoipa，此枚举只做线上形状）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatusDto {
    /// 人工确认的事实（AI 产出默认不是它）。
    Confirmed,
    NeedsReview,
}

impl ReviewStatusDto {
    pub const fn to_domain(self) -> ReviewStatus {
        match self {
            Self::Confirmed => ReviewStatus::Confirmed,
            Self::NeedsReview => ReviewStatus::NeedsReview,
        }
    }
}

/// 实体级复核写入的线上形状（见 `drafts::aggregate::EntityReviewPatch`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntityReviewPatchDto {
    #[schema(nullable = true)]
    pub review_status: Option<ReviewStatusDto>,
    #[schema(nullable = true)]
    pub user_edited: Option<UserEdit>,
    /// 「仅文本条目」：不进入 3D 但保留并明显标识（需同时确认或有修订记录）。
    #[schema(nullable = true)]
    pub text_only: Option<bool>,
}

impl EntityReviewPatchDto {
    fn to_domain(&self) -> EntityReviewPatch {
        EntityReviewPatch {
            review_status: self.review_status.map(ReviewStatusDto::to_domain),
            user_edited: self.user_edited.clone(),
            text_only: self.text_only,
        }
    }
}

/// `PATCH /items/{id}/drafts/{draftId}` 请求体（T19 受限字段集）。
///
/// 每个字段都可缺省；**至少提供一个字段**，否则 422（空请求体）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftPatchRequest {
    /// 目标状态（`needs_review` / `ready`）；不触发发布。
    #[schema(nullable = true)]
    pub status: Option<DraftStatusDto>,
    /// 热点写入（新建/重新绑定/解绑；人工直接拾取 = upsert confirmed + anchor）。
    #[schema(nullable = true)]
    pub hotspots: Option<HotspotPatch>,
    /// 步骤视角写入：`{ "<stepId>": CameraPose }`（保存/覆盖；「保存当前视角」动作）。
    #[schema(nullable = true)]
    pub step_poses: Option<BTreeMap<String, CameraPose>>,
    /// 要清除视角的步骤 id（「清除该步骤视角」动作；与写入分开）。
    #[schema(nullable = true)]
    pub clear_step_poses: Option<Vec<String>>,
    /// 实体级复核（确认/取消确认、人工修订、仅文本条目）。
    #[schema(nullable = true)]
    pub entities: Option<BTreeMap<String, EntityReviewPatchDto>>,
    /// modelReview 的两个用户声明（`checkedAt` 由服务器赋值）。
    #[schema(nullable = true)]
    pub model_review: Option<ModelReviewPatch>,
}

impl DraftPatchRequest {
    /// 线上形状 → 服务端语义类型（唯一转换点）。
    pub fn to_domain(&self) -> DraftPatch {
        DraftPatch {
            status: self.status.map(DraftStatusDto::to_domain),
            hotspots: self.hotspots.clone(),
            step_poses: self.step_poses.clone(),
            clear_step_poses: self.clear_step_poses.clone(),
            entities: self.entities.as_ref().map(|entities| {
                entities
                    .iter()
                    .map(|(id, patch)| (id.clone(), patch.to_domain()))
                    .collect()
            }),
            model_review: self.model_review.clone(),
        }
    }
}

/// 草稿语义的固定说明（响应携带；UI 直接展示）。
pub const DRAFT_NOTICES: [&str; 2] = [
    "生成完成不等于已发布：草稿需要人工确认知识并完成热点校准后，显式发布才会产出不可变版本",
    "不存在自动发布路径：后台不会自动创建 release",
];
