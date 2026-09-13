//! 说明书绑定 DTO（REQ-012；contracts.md §2/§3）。
//!
//! `sourceUrl` 只作出处记录：服务端**不会**访问该地址（上传本地文件是唯一资料入口），
//! 校验只保证它是绝对 http(s) URL。`sourceSha256` 来自被绑定资产的 blob
//! （内容寻址主键），供准备阶段（T09）核对原件字节未变。

use manual_core::domain::Document;
use manual_core::timestamps::Timestamp;
use manual_core::validation::{DOCUMENT_TITLE_MAX_CHARS, SOURCE_URL_MAX_CHARS};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 编译期防漂移：`#[schema(max_length = …)]` 不接受常量表达式，把字面量与
/// `manual_core::validation` 的校验上限钉在一起。
const _: () = {
    assert!(
        DOCUMENT_TITLE_MAX_CHARS == 200 && SOURCE_URL_MAX_CHARS == 2000,
        "DocumentCreateRequest 的 maxLength 字面量已与校验常量脱节"
    );
};

/// 单个 document 响应。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DocumentResponse {
    pub data: DocumentDto,
}

/// document 列表响应（`{data, nextCursor}`，按 created_at DESC 分页）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentListResponse {
    pub data: Vec<DocumentDto>,
    #[schema(
        nullable = true,
        example = "v1:documents:01993000-0000-7000-8000-000000000001:1789171200000:01993000-0000-7000-8000-000000000002"
    )]
    pub next_cursor: Option<String>,
}

/// document 的线上表示。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDto {
    pub id: String,
    pub item_id: String,
    /// 绑定的 PDF 资产 ID（`purpose=document`）。
    pub source_asset_id: String,
    /// 原件内容 sha256（=该资产的 blob id）；准备阶段据此校验字节一致。
    pub source_sha256: String,
    pub title: String,
    /// 出处链接；仅记录，服务端不抓取。
    #[schema(nullable = true)]
    pub source_url: Option<String>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl From<Document> for DocumentDto {
    fn from(document: Document) -> Self {
        Self {
            id: document.id,
            item_id: document.item_id,
            source_asset_id: document.source_asset_id,
            source_sha256: document.source_sha256,
            title: document.title,
            source_url: document.source_url,
            created_at: document.created_at,
            updated_at: document.updated_at,
        }
    }
}

/// `POST /api/v1/items/{id}/documents` 请求体。
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentCreateRequest {
    /// 已上传的 PDF 资产 ID；必须属于同一物品且 `purpose=document`，否则 404/422。
    pub source_asset_id: String,
    /// 标题；必填，≤[`DOCUMENT_TITLE_MAX_CHARS`] 字符。
    #[schema(max_length = 200)]
    pub title: Option<String>,
    /// 可选出处链接（绝对 http(s) URL，≤[`SOURCE_URL_MAX_CHARS`] 字符）；服务端不访问该地址。
    #[schema(nullable = true, max_length = 2000)]
    pub source_url: Option<String>,
}
