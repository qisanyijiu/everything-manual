//! PDF 准备与页上传 DTO（T09 / REQ-014、REQ-015；contracts.md §2/§3）。
//!
//! 约定：
//! - `pageNumber` **1-based**（contracts.md §1）；页集合与缺页都以它为准，不出现 0 基索引；
//! - `viewport` 描述**旋转后**的页图尺寸与旋转角（架构 §5.1：页图坐标原点为旋转后
//!   viewport 左上角）；`PUT` 必填，`GET` 对旧记录可为 null（不写占位值）；
//! - `state` 与 `viewport.rotation` 在请求体里用字符串/数字而非枚举反序列化，
//!   以便给出**字段级** `details.fields` 明细（与 photos 的 `view` 同一约定）；
//! - `PreparationDetailDto` 显式列出 preparation 字段而不 `#[serde(flatten)]`：
//!   OpenAPI 生成与前端类型更直白，避免 flatten 在 schema 上的边缘行为。

use manual_core::domain::{Page, PageViewport, Preparation, PreparationState};
use manual_core::timestamps::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 单个 preparation 响应（`{ data }` 包装）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PreparationResponse {
    pub data: PreparationDto,
}

/// `GET /preparations/{id}` 响应：准备状态 + 页状态 + 缺页（断线续传的读取入口）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PreparationDetailResponse {
    pub data: PreparationDetailDto,
}

/// 单页响应（`PUT .../pages/{n}` 返回写入后的页状态）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PageResponse {
    pub data: PageDto,
}

/// preparation 的线上表示。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreparationDto {
    pub id: String,
    pub document_id: String,
    /// 原件内容 sha256（与 document.sourceSha256 一致；准备期间核对字节未变）。
    pub source_sha256: String,
    /// `preparing` 或 `ready`；ready 后不可修改（contracts.md §2）。
    #[schema(value_type = String, example = "preparing")]
    pub state: PreparationState,
    /// 封存时声明的页数；未封存为 null。
    #[schema(nullable = true)]
    pub page_count: Option<i64>,
    /// `true` = 页资产由浏览器 PDF.js 派生上传；只证明字节一致，不证明来自原 PDF。
    pub client_derived: bool,
    /// 乐观锁版本；`GET` 时对应 `ETag: "r<revision>"`，页覆盖与封存需 `If-Match`。
    pub revision: i64,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl PreparationDto {
    pub fn from_preparation(preparation: &Preparation) -> Self {
        Self {
            id: preparation.id.clone(),
            document_id: preparation.document_id.clone(),
            source_sha256: preparation.source_sha256.clone(),
            state: preparation.state,
            page_count: preparation.page_count,
            client_derived: preparation.client_derived,
            revision: preparation.revision,
            created_at: preparation.created_at,
            updated_at: preparation.updated_at,
        }
    }
}

/// preparation 详情：状态 + 已上传页 + 缺页。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreparationDetailDto {
    pub id: String,
    pub document_id: String,
    pub source_sha256: String,
    #[schema(value_type = String, example = "preparing")]
    pub state: PreparationState,
    #[schema(nullable = true)]
    pub page_count: Option<i64>,
    pub client_derived: bool,
    pub revision: i64,
    /// 已上传页（按 `pageNumber` 升序）。
    pub pages: Vec<PageDto>,
    /// 缺页页号（升序）。
    ///
    /// 未封存（`preparing`）时服务端不知道原 PDF 总页数（PDF 由浏览器解析，ADR-003），
    /// 因此该数组为空，由客户端用自己的总页数计算"还差哪些页"；
    /// `ready` 时为 `1..pageCount` 中缺失的页号（正常应为空）。
    pub missing_pages: Vec<i64>,
    #[schema(value_type = String)]
    pub created_at: Timestamp,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl PreparationDetailDto {
    /// 组装详情；`missing_pages` 由调用方按 state 计算（见字段文档）。
    pub fn new(preparation: &Preparation, pages: &[Page], missing_pages: Vec<i64>) -> Self {
        Self {
            id: preparation.id.clone(),
            document_id: preparation.document_id.clone(),
            source_sha256: preparation.source_sha256.clone(),
            state: preparation.state,
            page_count: preparation.page_count,
            client_derived: preparation.client_derived,
            revision: preparation.revision,
            pages: pages.iter().map(PageDto::from_page).collect(),
            missing_pages,
            created_at: preparation.created_at,
            updated_at: preparation.updated_at,
        }
    }
}

/// 单页的线上表示。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageDto {
    /// 1-based 页号（contracts.md §1）。
    pub page_number: i64,
    /// 页文字资产（`purpose=pageText`）；扫描页无文字时为 null。
    #[schema(nullable = true)]
    pub text_asset_id: Option<String>,
    /// 页图资产（`purpose=pageImage`，白底 JPEG）。
    #[schema(nullable = true)]
    pub image_asset_id: Option<String>,
    /// 页图坐标参照（旋转后的尺寸与旋转角）；数据迁移前的旧记录为 null。
    #[schema(nullable = true)]
    pub viewport: Option<ViewportDto>,
    #[schema(value_type = String)]
    pub updated_at: Timestamp,
}

impl PageDto {
    pub fn from_page(page: &Page) -> Self {
        Self {
            page_number: page.page_number,
            text_asset_id: page.text_asset_id.clone(),
            image_asset_id: page.image_asset_id.clone(),
            viewport: page.viewport.map(ViewportDto::from_viewport),
            updated_at: page.updated_at,
        }
    }
}

/// 页图 viewport（旋转后的页图尺寸与旋转角）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewportDto {
    /// 页图宽度（像素，已含页面旋转）。
    pub width: u32,
    /// 页图高度（像素）。
    pub height: u32,
    /// 旋转角（度）：0/90/180/270。
    pub rotation: u16,
}

impl ViewportDto {
    pub fn from_viewport(viewport: PageViewport) -> Self {
        Self {
            width: viewport.width,
            height: viewport.height,
            rotation: viewport.rotation,
        }
    }

    pub fn to_viewport(self) -> PageViewport {
        PageViewport {
            width: self.width,
            height: self.height,
            rotation: self.rotation,
        }
    }
}

/// `POST /api/v1/documents/{id}/preparations` 请求体。
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparationCreateRequest {
    /// 原件内容 sha256；必须与 document 绑定的原件一致（否则 422），
    /// 用于确认浏览器准备的是同一份字节（contracts.md §3）。
    pub source_sha256: Option<String>,
}

/// `PUT /api/v1/preparations/{id}/pages/{pageNumber}` 请求体。
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PagePutRequest {
    /// 页文字资产（`purpose=pageText`）；扫描页/无文字页可为 null。
    #[schema(nullable = true)]
    pub text_asset_id: Option<String>,
    /// 页图资产（`purpose=pageImage`，白底 JPEG）；每页必需（封存时校验）。
    #[schema(nullable = true)]
    pub image_asset_id: Option<String>,
    /// 页图坐标参照（旋转后尺寸 + 旋转角）；必填。
    #[schema(nullable = true)]
    pub viewport: Option<ViewportDto>,
}

/// `POST /api/v1/preparations/{id}/complete` 请求体。
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparationCompleteRequest {
    /// 声明的总页数 N；服务端校验 1..N 连续存在且每页资产可用。
    #[schema(nullable = true)]
    pub page_count: Option<i64>,
}
