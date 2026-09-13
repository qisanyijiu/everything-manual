//! 设置/能力状态 DTO（REQ-007、AC-012）。
//!
//! 边界：只暴露**配置状态与公开限制**——不返回密钥、密钥来源、文件路径、
//! 完整配置或供应商凭据（PRD §5.7）。未配置时如实返回 `false`，不存在 mock 回退。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `GET /api/v1/settings/status` 响应。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SettingsStatusResponse {
    pub data: SettingsStatusData,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettingsStatusData {
    /// 各供应商是否具备发起真实请求的全部配置（密钥 + 模型）。
    pub providers_configured: ProvidersConfigured,
    /// 价格目录状态（T11；只说配置与否与版本，不含路径与内容）。
    pub price_catalog: PriceCatalogStatus,
    /// 生效的输入限制（PRD §5.3 默认值或配置覆盖值）。
    pub limits: LimitsStatus,
    /// 服务能力开关；未满足前置条件时如实为 false。
    pub capabilities: CapabilitiesStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersConfigured {
    /// Tripo（模型生成）是否已配置。
    pub tripo: bool,
    /// 说明书 AI 是否已配置。
    pub manual_ai: bool,
}

/// 生效限制（字节数与页数；均为公开参数，不含路径与密钥）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LimitsStatus {
    pub max_json_request_bytes: u64,
    pub max_pdf_bytes: u64,
    pub max_pdf_pages: u32,
    pub max_photo_bytes: u64,
    pub max_glb_bytes: u64,
    pub max_item_total_bytes: u64,
}

/// 价格目录状态（T11）：只返回"是否可用 + 版本/快照日期"，不返回文件路径或内容。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PriceCatalogStatus {
    /// 是否已配置且解析成功（未配置 = 生成/报价不可用，409 `PRICE_CATALOG_MISSING`）。
    pub configured: bool,
    /// 价格版本（未配置为 null）。
    #[schema(nullable = true, example = "2026-09-11")]
    pub version: Option<String>,
    /// 价格快照日期（未配置为 null）。
    #[schema(nullable = true, example = "2026-09-11")]
    pub snapshot_date: Option<String>,
}

/// 能力开关。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesStatus {
    /// 生成能力：Tripo 与说明书 AI **都**已配置、且价格目录可用时才为 true。
    /// 未满足前置条件时 estimate/jobs 返回明确错误（不返回 0 费用假成功）。
    pub generation: bool,
}
