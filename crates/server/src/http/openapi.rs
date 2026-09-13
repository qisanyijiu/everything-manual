//! OpenAPI 3.1 导出（ADR-009 的机器合同入口）。
//!
//! `cargo xtask contracts` 调用 [`openapi_pretty_json`] 写出 `contracts/openapi.json`，
//! 再由 openapi-typescript 生成 `apps/web/src/api/generated.ts`。

use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::releases::invariants::PublishIssue;

use super::dto::{
    AdminSummary, AmountCurrencyDto, AmountLineDto, Anchor, ApiErrorBody, ApiErrorResponse,
    AssetDto, AssetResponse, AssetUploadRequest, BudgetLimitsDto, CameraPose, CancelResponse,
    CancelResultDto, CapabilitiesStatus, CheckStatus, ConfirmationDto, ConfirmationResponse,
    DocumentCreateRequest, DocumentDto, DocumentListResponse, DocumentResponse, DraftDto,
    DraftMissingItemDto, DraftPatchRequest, DraftResponse, DraftStatusDto, EntityReviewPatchDto,
    EstimateRequest, HealthLiveResponse, HealthReadyResponse, HotspotPatch, HotspotStatus,
    HotspotUpsert, ItemCreateRequest, ItemDto, ItemListResponse, ItemPatchRequest, ItemResponse,
    JobAttemptDto, JobCreateRequest, JobDetailDto, JobDetailResponse, JobDto, JobItemDto,
    JobListResponse, JobMissingItemDto, JobResponse, JobStageDto, JobStageSummaryDto,
    JobSummaryDto, LimitsStatus, LivenessData, LivenessStatus, LoginRequest, LoginResponse,
    ManualAiConfigDto, ManualAiSendScopeDto, ModelReviewPatch, PageDto, PagePutRequest,
    PageRangeDto, PageResponse, PhotoCreateRequest, PhotoDto, PhotoListResponse, PhotoPatchRequest,
    PhotoResponse, PhotoScopeDto, PreparationCompleteRequest, PreparationCreateRequest,
    PreparationDetailDto, PreparationDetailResponse, PreparationDto, PreparationResponse,
    PreservedStageDto, PriceCatalogStatus, ProviderAmountDto, ProviderConfigDto, ProviderKeyDto,
    ProvidersConfigured, QuoteAmountsDto, QuoteDto, QuoteResponse, ReadinessCheck,
    ReadinessCheckName, ReadinessData, ReadinessStatus, ReconcileActionDto, ReconcileRequestDto,
    ReconcileResponse, ReconcileResultDto, ReleaseDetailDto, ReleaseDetailResponse, ReleaseDto,
    ReleaseListResponse, ReleaseResponse, ReservationDto, RetryRequest, RetryResponse,
    RetryResultDto, ReviewStatusDto, SendScopeDto, SessionData, SessionResponse,
    SettingsStatusData, SettingsStatusResponse, TripoParametersDto, TripoSendScopeDto, UserEdit,
    ViewportDto,
};
use super::{
    assets, auth, documents, drafts, estimates, health, items, jobs, photos, preparations,
    releases, settings,
};

/// API 文档定义。
///
/// 说明：
/// - 受会话保护的操作标注 `security(("sessionCookie" = []))`；健康探针与登录无 security；
/// - 未知 `/api/*` 的 JSON 404 由 fallback 实现，OpenAPI 无法表达 fallback 路由，
///   其响应结构与本文件的 `ApiErrorResponse` 一致。
#[derive(OpenApi)]
#[openapi(
    info(
        title = "万物说明书 API",
        version = "0.1.0",
        description = "自托管交互说明书的本地 HTTP API。健康探针、认证/会话、设置状态、\
                       资产上传/内容服务（T06）、物品与说明书绑定/多视图照片（T07）、\
                       其余路由按 llmdoc/contracts.md 在同一文档上扩展。"
    ),
    paths(
        health::live,
        health::ready,
        auth::login,
        auth::session,
        auth::logout,
        settings::status,
        items::list_items,
        items::create_item,
        items::get_item,
        items::patch_item,
        documents::create_document,
        documents::list_documents,
        photos::create_photo,
        photos::list_photos,
        photos::get_photo,
        photos::patch_photo,
        assets::upload_asset,
        assets::get_asset_content,
        preparations::create_preparation,
        preparations::get_preparation,
        preparations::put_page,
        preparations::complete_preparation,
        estimates::create_estimate,
        estimates::get_estimate,
        estimates::confirm_estimate,
        jobs::create_job,
        jobs::list_jobs,
        jobs::get_job,
        jobs::cancel_job,
        jobs::retry_job,
        jobs::reconcile_job,
        drafts::get_draft,
        drafts::patch_draft,
        drafts::publish_draft,
        releases::list_releases,
        releases::get_release,
        releases::export_release,
    ),
    components(schemas(
        ApiErrorResponse,
        ApiErrorBody,
        HealthLiveResponse,
        LivenessData,
        LivenessStatus,
        HealthReadyResponse,
        ReadinessData,
        ReadinessStatus,
        ReadinessCheck,
        ReadinessCheckName,
        CheckStatus,
        LoginRequest,
        LoginResponse,
        SessionResponse,
        SessionData,
        AdminSummary,
        SettingsStatusResponse,
        SettingsStatusData,
        ProvidersConfigured,
        PriceCatalogStatus,
        LimitsStatus,
        CapabilitiesStatus,
        ItemResponse,
        ItemListResponse,
        ItemDto,
        ItemCreateRequest,
        ItemPatchRequest,
        DocumentResponse,
        DocumentListResponse,
        DocumentDto,
        DocumentCreateRequest,
        PhotoResponse,
        PhotoListResponse,
        PhotoDto,
        PhotoCreateRequest,
        PhotoPatchRequest,
        AssetResponse,
        AssetDto,
        AssetUploadRequest,
        PreparationResponse,
        PreparationDetailResponse,
        PageResponse,
        PreparationDto,
        PreparationDetailDto,
        PageDto,
        ViewportDto,
        PreparationCreateRequest,
        PagePutRequest,
        PreparationCompleteRequest,
        EstimateRequest,
        QuoteResponse,
        QuoteDto,
        ProviderConfigDto,
        TripoParametersDto,
        ManualAiConfigDto,
        PageRangeDto,
        QuoteAmountsDto,
        ProviderAmountDto,
        AmountCurrencyDto,
        AmountLineDto,
        SendScopeDto,
        TripoSendScopeDto,
        PhotoScopeDto,
        ManualAiSendScopeDto,
        ConfirmationResponse,
        ConfirmationDto,
        JobCreateRequest,
        BudgetLimitsDto,
        JobResponse,
        JobDto,
        ReservationDto,
        ProviderKeyDto,
        JobListResponse,
        JobSummaryDto,
        JobStageSummaryDto,
        JobDetailResponse,
        JobDetailDto,
        JobItemDto,
        JobStageDto,
        JobMissingItemDto,
        JobAttemptDto,
        CancelResponse,
        CancelResultDto,
        PreservedStageDto,
        RetryRequest,
        RetryResponse,
        RetryResultDto,
        ReconcileActionDto,
        ReconcileRequestDto,
        ReconcileResponse,
        ReconcileResultDto,
        DraftResponse,
        DraftDto,
        DraftMissingItemDto,
        DraftPatchRequest,
        DraftStatusDto,
        ReviewStatusDto,
        EntityReviewPatchDto,
        HotspotPatch,
        HotspotUpsert,
        HotspotStatus,
        Anchor,
        CameraPose,
        UserEdit,
        ModelReviewPatch,
        PublishIssue,
        ReleaseResponse,
        ReleaseDto,
        ReleaseDetailResponse,
        ReleaseDetailDto,
        ReleaseListResponse,
    )),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// 会话 cookie 安全方案（`em_session`，HttpOnly + SameSite=Strict）。
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::new);
        components.add_security_scheme(
            "sessionCookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(auth::SESSION_COOKIE_NAME))),
        );
    }
}

/// 导出确定性排序的 OpenAPI 3.1 文档。
///
/// 经 `serde_json::Value`（BTreeMap）重新序列化，保证键顺序稳定；
/// 结尾统一补换行，便于 `contracts --check` 做字节比较。
pub fn openapi_pretty_json() -> Result<String, serde_json::Error> {
    let document = ApiDoc::openapi();
    let mut value = serde_json::to_value(&document)?;
    strip_empty_license(&mut value);
    let mut json = serde_json::to_string_pretty(&value)?;
    json.push('\n');
    Ok(json)
}

/// utoipa 默认写出 `info.license.name = ""`。本项目尚未确定许可证，
/// 空名许可证是误导性字段：直接移除，而不是编造一个名称。
fn strip_empty_license(value: &mut serde_json::Value) {
    let Some(info) = value.get_mut("info").and_then(|info| info.as_object_mut()) else {
        return;
    };
    let is_empty = info
        .get("license")
        .and_then(|license| license.get("name"))
        .and_then(|name| name.as_str())
        .is_some_and(str::is_empty);
    if is_empty {
        info.remove("license");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> serde_json::Value {
        let json = openapi_pretty_json().unwrap();
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn openapi_is_3_1_and_declares_health_paths() {
        let value = document();
        assert_eq!(value["openapi"], "3.1.0");
        assert!(value["paths"]["/api/v1/health/live"]["get"].is_object());
        assert!(value["paths"]["/api/v1/health/ready"]["get"].is_object());
        assert!(value["components"]["schemas"]["HealthLiveResponse"].is_object());
        // 未确定许可证前不得输出空名 license 字段。
        assert!(value["info"].get("license").is_none());
        // 错误 details 是任意 JSON 或 null，不能被收窄成对象。
        let details = &value["components"]["schemas"]["ApiErrorBody"]["properties"]["details"];
        assert!(
            details.get("type").is_none() || details["type"].as_array().is_some(),
            "details 应是开放 schema，实际 {details}"
        );
    }

    #[test]
    fn openapi_declares_auth_settings_and_items_routes() {
        let value = document();
        for path in [
            "/api/v1/auth/login",
            "/api/v1/auth/session",
            "/api/v1/auth/logout",
            "/api/v1/settings/status",
            "/api/v1/items",
            "/api/v1/items/{id}",
        ] {
            assert!(value["paths"].get(path).is_some(), "缺少路径 {path}");
        }
        assert!(value["paths"]["/api/v1/auth/login"]["post"].is_object());
        assert!(value["paths"]["/api/v1/items/{id}"]["patch"].is_object());

        // 受保护操作声明了会话 cookie 方案；探针与登录不声明 security。
        assert!(value["paths"]["/api/v1/items"]["get"]["security"].is_array());
        assert!(
            value["paths"]["/api/v1/health/ready"]["get"]
                .get("security")
                .is_none(),
            "健康探针不得声明会话要求"
        );
        let schemes = &value["components"]["securitySchemes"]["sessionCookie"];
        assert_eq!(schemes["type"], "apiKey");
        assert_eq!(schemes["in"], "cookie");
        assert_eq!(schemes["name"], "em_session");
    }

    #[test]
    fn openapi_declares_asset_upload_and_content_routes() {
        let value = document();
        let upload = &value["paths"]["/api/v1/items/{id}/assets"]["post"];
        assert!(upload.is_object(), "缺少上传路由");
        assert_eq!(
            upload["requestBody"]["content"]["multipart/form-data"]["schema"]["$ref"],
            "#/components/schemas/AssetUploadRequest"
        );
        assert_eq!(
            upload["responses"]["201"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/AssetResponse"
        );
        let content = &value["paths"]["/api/v1/assets/{id}/content"]["get"];
        assert!(content.is_object(), "缺少内容路由");
        assert!(content["responses"].get("206").is_some(), "应声明 206");
        assert!(content["responses"].get("304").is_some(), "应声明 304");
        assert!(content["responses"].get("416").is_some(), "应声明 416");
    }

    #[test]
    fn openapi_declares_t07_item_document_and_photo_routes() {
        let value = document();
        for (path, method) in [
            ("/api/v1/items", "post"),
            ("/api/v1/items/{id}/documents", "post"),
            ("/api/v1/items/{id}/documents", "get"),
            ("/api/v1/items/{id}/photos", "post"),
            ("/api/v1/items/{id}/photos", "get"),
            ("/api/v1/items/{id}/photos/{photoId}", "get"),
            ("/api/v1/items/{id}/photos/{photoId}", "patch"),
        ] {
            assert!(
                value["paths"][path][method].is_object(),
                "缺少 {method} {path}"
            );
        }
        // 创建返回 201；PATCH 声明 428/412；不声明任何 DELETE 操作。
        assert!(
            value["paths"]["/api/v1/items"]["post"]["responses"]["201"].is_object(),
            "创建物品必须声明 201"
        );
        for (path, method) in [
            ("/api/v1/items/{id}", "patch"),
            ("/api/v1/items/{id}/photos/{photoId}", "patch"),
        ] {
            let responses = &value["paths"][path][method]["responses"];
            assert!(responses.get("428").is_some(), "{path} 应声明 428");
            assert!(responses.get("412").is_some(), "{path} 应声明 412");
        }
        for path in [
            "/api/v1/items",
            "/api/v1/items/{id}",
            "/api/v1/items/{id}/documents",
            "/api/v1/items/{id}/photos",
            "/api/v1/items/{id}/photos/{photoId}",
        ] {
            assert!(
                value["paths"][path].get("delete").is_none(),
                "MVP 不提供永久删除：{path} 不得声明 DELETE"
            );
        }
        // maxLength 字面量与校验常量一致（防止两处漂移）。
        let properties = &value["components"]["schemas"]["ItemCreateRequest"]["properties"];
        assert_eq!(
            properties["name"]["maxLength"].as_u64().map(|v| v as usize),
            Some(manual_core::validation::ITEM_NAME_MAX_CHARS)
        );
        assert_eq!(
            properties["brand"]["maxLength"]
                .as_u64()
                .map(|v| v as usize),
            Some(manual_core::validation::ITEM_BRAND_MAX_CHARS)
        );
        assert_eq!(
            properties["model"]["maxLength"]
                .as_u64()
                .map(|v| v as usize),
            Some(manual_core::validation::ITEM_MODEL_MAX_CHARS)
        );
        assert_eq!(
            properties["variant"]["maxLength"]
                .as_u64()
                .map(|v| v as usize),
            Some(manual_core::validation::ITEM_VARIANT_MAX_CHARS)
        );
    }

    #[test]
    fn openapi_declares_release_export_route() {
        let value = document();
        let export = &value["paths"]["/api/v1/releases/{releaseId}/export"]["get"];
        assert!(export.is_object(), "缺少导出自包含包路由（REQ-037）");
        // 合同：受会话保护；响应是 application/zip 二进制附件。
        assert!(export["security"].is_array(), "导出必须声明会话要求");
        assert_eq!(
            export["responses"]["200"]["content"]["application/zip"]["schema"]["type"],
            "string"
        );
        assert!(
            export["responses"]["200"]["content"]
                .get("application/json")
                .is_none(),
            "导出成功响应不是 JSON"
        );
    }

    #[test]
    fn openapi_error_responses_use_contract_shape() {
        let value = document();
        let login_401 = &value["paths"]["/api/v1/auth/login"]["post"]["responses"]["401"];
        assert_eq!(
            login_401["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ApiErrorResponse"
        );
        // 428/412 在 PATCH 上声明（QA 的 AC-004 合同面）。
        let patch = &value["paths"]["/api/v1/items/{id}"]["patch"]["responses"];
        assert!(patch.get("428").is_some() && patch.get("412").is_some());
    }

    #[test]
    fn openapi_export_is_deterministic() {
        let first = openapi_pretty_json().unwrap();
        let second = openapi_pretty_json().unwrap();
        assert_eq!(first, second);
    }
}
