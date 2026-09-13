//! 照片与视图 DTO（REQ-013；contracts.md §2/§3）。
//!
//! 视图枚举 `front/left/back/right/detail`（方向以物品自身为参照，PRD A-04）；
//! **同一物品每视图最多一张**（DB 唯一索引 + 服务层友好错误，见 ADR-016）。
//! `detail`（特写）只用于理解与核对，不属于多视图集合（`GET` 列表仍返回它，
//! T12 的 Tripo 请求体只用 `is_multiview()` 的视图）。
//!
//! `view` 在请求体中是字符串而不是 serde 枚举：为了在非法取值时给出**字段级**
//! `details.fields` 明细（枚举反序列化失败只会在提取器层得到通用 422）。

use manual_core::domain::{Photo, PhotoView};
use manual_core::timestamps::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 单个 photo 响应；配合 `ETag: "r<n>"` 返回（PATCH 需 If-Match）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PhotoResponse {
    pub data: PhotoDto,
}

/// 照片列表响应。
///
/// 物品的照片集合被视图唯一性约束上界为 5 条（front/left/back/right/detail 各一张），
/// 因此不设分页：`nextCursor` 恒为 null（不是截断）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PhotoListResponse {
    pub data: Vec<PhotoDto>,
    #[schema(nullable = true)]
    pub next_cursor: Option<String>,
}

/// photo 的线上表示。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PhotoDto {
    pub id: String,
    pub item_id: String,
    /// 照片资产 ID（`purpose=photo`，JPEG/PNG）；内容经 `/assets/{id}/content` 读取。
    pub asset_id: String,
    #[schema(value_type = String, example = "front")]
    pub view: PhotoView,
    /// 乐观锁版本；GET 时对应 `ETag: "r<revision>"`。
    pub revision: i64,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl From<Photo> for PhotoDto {
    fn from(photo: Photo) -> Self {
        Self {
            id: photo.id,
            item_id: photo.item_id,
            asset_id: photo.asset_id,
            view: photo.view,
            revision: photo.revision,
            created_at: photo.created_at,
            updated_at: photo.updated_at,
        }
    }
}

/// `POST /api/v1/items/{id}/photos` 请求体。
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhotoCreateRequest {
    /// 照片资产 ID；必须属于同一物品且 `purpose=photo`，否则 404/422。
    pub asset_id: String,
    /// 视图：front/left/back/right/detail（字符串 + 服务端校验以获得字段级错误明细；
    /// 缺失/非法 → 422 `details.fields[view]`，与 items 的必填字段同一约定）。
    #[schema(value_type = Option<String>, nullable = true, example = "front")]
    pub view: Option<String>,
}

/// `PATCH /api/v1/items/{id}/photos/{photoId}` 请求体。
///
/// 字段缺失 = 保持原值；显式 `null` = 字段级 422（照片必须始终关联资产与视图）。
/// 变更视图时若目标视图已被同物品的另一张照片占用 → 422 `details.reason=viewOccupied`。
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhotoPatchRequest {
    /// 替换照片资产（缺失 = 保持；null = 422）。
    #[serde(default, deserialize_with = "super::super::body::double_option")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub asset_id: Option<Option<String>>,
    /// 视图（缺失 = 保持；null = 422）。
    #[serde(default, deserialize_with = "super::super::body::double_option")]
    #[schema(value_type = Option<String>, nullable = true, example = "left")]
    pub view: Option<Option<String>>,
}
