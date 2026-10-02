//! 设置/能力状态 DTO（REQ-007、AC-012）。
//!
//! status 保留运行状态与公开限制；providers 新增脱敏配置读取与私有覆盖写入。
//! 读取不返回密钥字符、环境变量名或路径；写入 DTO 不实现 Debug/Serialize。

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
    /// 额外的配置切换门禁；原有字段仍表示当前运行配置。
    pub provider_config_pending: bool,
    /// 各供应商是否具备发起真实请求的全部配置（密钥 + 模型）。
    pub providers_configured: ProvidersConfigured,
    /// 模型误填状态，不含疑似值或其任何摘要。
    pub provider_model_issues: ProviderModelIssues,
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

/// 只包含可公开配置及来源，不含密钥字符、摘要或路径。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub base_url: String,
    pub model: Option<String>,
    pub model_issue: Option<ModelIssue>,
    pub key_configured: bool,
    pub base_url_source: ConfigSource,
    pub model_source: ConfigSource,
    pub key_source: ConfigSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ModelIssue {
    SuspectedCredential,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelIssues {
    pub tripo: Option<ModelIssue>,
    pub manual_ai: Option<ModelIssue>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ConfigSource {
    Web,
    Deployment,
    Default,
    Unconfigured,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderViews {
    pub tripo: ProviderView,
    pub manual_ai: ProviderView,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettingsData {
    /// 不透明的已保存修订；保存时原样回传。
    pub revision: String,
    pub pending: bool,
    pub active: ProviderViews,
    pub saved: ProviderViews,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderSettingsResponse {
    pub data: ProviderSettingsData,
}

/// Explicit discovery using the active Manual AI configuration; IDs only, no upstream metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ManualAiModelsResponse {
    /// Valid model IDs, sorted and deduplicated; selection does not save configuration.
    pub data: Vec<String>,
}

// 不派生 Debug/Serialize：写入 DTO 只用于瞬时输入，不可进入日志或读取响应。
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSettingsWrite {
    pub revision: String,
    pub tripo: ProviderEdit,
    pub manual_ai: ProviderEdit,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderEdit {
    pub action: ProviderEditAction,
    pub base_url: Option<String>,
    pub model: Option<String>,
    /// 明确清空已隐藏问题模型的意图；model:null 本身不代表授权清空。
    pub clear_model: Option<bool>,
    pub key_action: Option<KeyEditAction>,
    #[schema(write_only)]
    pub api_key: Option<String>,
}

impl Drop for ProviderEdit {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        if let Some(key) = &mut self.api_key {
            key.zeroize();
        }
        if let Some(model) = &mut self.model {
            model.zeroize();
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProviderEditAction {
    Update,
    Restore,
}

#[derive(Debug, Clone, Copy, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum KeyEditAction {
    Keep,
    Replace,
    Clear,
}
