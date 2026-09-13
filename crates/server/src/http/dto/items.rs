//! 物品 DTO（REQ-010；contracts.md §2/§3）。
//!
//! PATCH 语义（T07 裁定，见 ADR-016 与 `http::items` 的 OpenAPI 描述）：
//! - 字段**缺失** = 保持原值；
//! - `brand`/`variant` 显式 `null`（或空白字符串）= **清空**；
//! - `name`/`model` 显式 `null` 或空白 = 字段级 422（必填字段不能用 null 清空）；
//! - 空请求体 `{}` = 字段级 422（不静默无操作、不空递增 revision）。
//!
//! 长度上限（PRD 只写"超长 422"）：name/model ≤200 字符、brand ≤100、variant ≤200，
//! 按字符数（`chars().count()`）计，先 trim。常量来自 `manual_core::validation`。

use manual_core::domain::Item;
use manual_core::timestamps::Timestamp;
use manual_core::validation::{
    ITEM_BRAND_MAX_CHARS, ITEM_MODEL_MAX_CHARS, ITEM_NAME_MAX_CHARS, ITEM_VARIANT_MAX_CHARS,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 编译期防漂移：`#[schema(max_length = …)]` 不接受常量表达式，这里把字面量
/// 与 `manual_core::validation` 的校验上限钉在一起（改一处而忘另一处会编译失败）。
const _: () = {
    assert!(
        ITEM_NAME_MAX_CHARS == 200 && ITEM_MODEL_MAX_CHARS == 200,
        "ItemCreateRequest/ItemPatchRequest 的 name/model maxLength 字面量已与校验常量脱节"
    );
    assert!(
        ITEM_BRAND_MAX_CHARS == 100 && ITEM_VARIANT_MAX_CHARS == 200,
        "ItemCreateRequest/ItemPatchRequest 的 brand/variant maxLength 字面量已与校验常量脱节"
    );
};

/// 单个物品响应；配合 `ETag: "r<n>"` 返回。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ItemResponse {
    pub data: ItemDto,
}

/// 物品列表响应（`{data, nextCursor}`；游标分页，默认 20／最多 100）。
///
/// `nextCursor` 绑定产生它的过滤条件（items:active / items:archived）；换条件必须重开分页。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemListResponse {
    pub data: Vec<ItemDto>,
    /// 下一页游标；没有更多时为 null。对客户端不透明，不得自行拼接。
    #[schema(
        nullable = true,
        example = "v1:items:active:1789171200000:01993000-0000-7000-8000-000000000001"
    )]
    pub next_cursor: Option<String>,
}

/// 物品的线上表示（字段名与 `manual_core::domain::Item` 一致）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemDto {
    pub id: String,
    pub name: String,
    #[schema(nullable = true)]
    pub brand: Option<String>,
    pub model: String,
    #[schema(nullable = true)]
    pub variant: Option<String>,
    /// 乐观锁版本；GET 时对应 `ETag: "r<revision>"`。
    pub revision: i64,
    /// 归档时间；null 表示未归档。归档代替物理删除（已发布资料与资产仍可读）。
    #[schema(value_type = Option<String>, nullable = true)]
    pub archived_at: Option<Timestamp>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl From<Item> for ItemDto {
    fn from(item: Item) -> Self {
        Self {
            id: item.id,
            name: item.name,
            brand: item.brand,
            model: item.model,
            variant: item.variant,
            revision: item.revision,
            archived_at: item.archived_at,
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}

/// `POST /api/v1/items` 请求体：创建物品（服务器生成 UUIDv7 与 revision=1）。
///
/// 名称与型号必填（缺失/空白/超长 → 422 `details.fields`）；品牌与变体可选；
/// 同品牌型号不强制唯一（允许不同配置并存，不用唯一约束代替业务判断）。
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemCreateRequest {
    /// 物品名称；必填，≤[`ITEM_NAME_MAX_CHARS`] 字符（trim 后非空）。
    #[schema(max_length = 200)]
    pub name: Option<String>,
    /// 品牌；可选，≤[`ITEM_BRAND_MAX_CHARS`] 字符。
    #[schema(nullable = true, max_length = 100)]
    pub brand: Option<String>,
    /// 准确型号；必填，≤[`ITEM_MODEL_MAX_CHARS`] 字符（trim 后非空）。
    #[schema(max_length = 200)]
    pub model: Option<String>,
    /// 变体/配置；可选，≤[`ITEM_VARIANT_MAX_CHARS`] 字符。
    #[schema(nullable = true, max_length = 200)]
    pub variant: Option<String>,
}

/// `PATCH /api/v1/items/{id}` 请求体。
///
/// 文本字段是双层 Option（见 [`crate::http::body::double_option`]）：缺失 = 保持原值；
/// `null` = 清空可选字段 / 拒绝必填字段。
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemPatchRequest {
    /// 名称（缺失 = 保持；null = 422）。
    #[serde(default, deserialize_with = "super::super::body::double_option")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub name: Option<Option<String>>,
    /// 品牌（缺失 = 保持；null 或空白 = 清空）。
    #[serde(default, deserialize_with = "super::super::body::double_option")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub brand: Option<Option<String>>,
    /// 型号（缺失 = 保持；null = 422）。
    #[serde(default, deserialize_with = "super::super::body::double_option")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub model: Option<Option<String>>,
    /// 变体/配置（缺失 = 保持；null 或空白 = 清空）。
    #[serde(default, deserialize_with = "super::super::body::double_option")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub variant: Option<Option<String>>,
    /// 归档开关：true 记录归档时间（已归档保持原时间），false 取消归档；
    /// 显式 null = 422（状态字段没有"清空"语义，缺失才是保持原值）。
    #[serde(default, deserialize_with = "super::super::body::double_option")]
    #[schema(value_type = Option<bool>, nullable = true)]
    pub archived: Option<Option<bool>>,
}
