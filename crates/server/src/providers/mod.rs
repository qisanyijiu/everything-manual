//! 外部 Provider 适配器（architecture.md §4：`providers/` 负责供应商格式转换，
//! 业务层不直接拼 URL）。
//!
//! 本模块只做两件事：
//! 1. **按配置注册阶段处理器**（[`register_provider_handlers`]，`serve` 启动时调用）：
//!    未配置的 Provider **不注册任何处理器**——已入队阶段被执行器延后并记录原因，
//!    不假成功、不回退 mock/fixture（REQ-007 / AC-013）；
//! 2. 暴露适配器类型给集成测试与后续卡（T12 的 [`tripo`]、T14 的 [`manual_ai`]）。
//!
//! 隔离约束（contracts.md §6 末段）：Mock 与真实 Provider 共用领域接口但初始化
//! 显式互斥；生产启动禁止无配置时回退 mock。测试进程的"零真实外网"由
//! fixture 只绑定 `127.0.0.1` + 全部 base_url 指向该 fixture + 调用计数断言保证。

pub mod manual_ai;
pub mod tripo;

use std::fmt;

use manual_core::domain::StageKind;

use crate::config::Settings;
use crate::jobs::StageRegistry;

/// 已配置但客户端无法构造（配置错误；不静默降级为"未配置"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSetupError {
    pub provider: &'static str,
    pub message: String,
}

impl fmt::Display for ProviderSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Provider {} 已配置但初始化失败：{}",
            self.provider, self.message
        )
    }
}

impl std::error::Error for ProviderSetupError {}

/// 按配置注册阶段处理器；返回实际注册的阶段类型（日志与测试断言用）。
///
/// 当前注册：Tripo 已配置 → `tripo_upload` / `tripo_submit` / `tripo_poll` /
/// `model_download` / `model_validate`（T13）；说明书 AI 已配置 → `manual_extract`
/// / `manual_merge`（T14）。未配置 → 不注册（阶段被延后，不假成功）。两个 Provider
/// 相互独立：任一未配置不影响另一个（REQ-029 的知识分支与 REQ-028 的模型分支分开）。
pub fn register_provider_handlers(
    registry: &mut StageRegistry,
    settings: &Settings,
) -> Result<Vec<StageKind>, ProviderSetupError> {
    let mut registered: Vec<StageKind> = Vec::new();
    if settings.providers.manual_ai.configured() {
        let handlers = manual_ai::ManualAiHandlers::from_settings(settings).map_err(|message| {
            ProviderSetupError {
                provider: "manual_ai",
                message,
            }
        })?;
        tracing::info!(
            event = "provider_handlers_registered",
            provider = "manual_ai",
            baseUrl = %crate::config::secret::redact_url_query(handlers.base_url()),
            stages = "manual_extract,manual_merge",
            "说明书 AI 阶段处理器已注册（Responses 同步单次请求；拒答/截断/格式错不产生正式知识）"
        );
        handlers.register(registry);
        registered.extend([StageKind::ManualExtract, StageKind::ManualMerge]);
    }
    if settings.providers.tripo.configured() {
        let handlers = tripo::TripoHandlers::from_settings(settings).map_err(|message| {
            ProviderSetupError {
                provider: "tripo",
                message,
            }
        })?;
        tracing::info!(
            event = "provider_handlers_registered",
            provider = "tripo",
            baseUrl = %crate::config::secret::redact_url_query(handlers.base_url()),
            stages = "tripo_upload,tripo_submit,tripo_poll,model_download,model_validate",
            "Tripo 阶段处理器已注册（真实 HTTP；未返回 task ID 的付费提交不自动重发；\
             模型下载使用独立无凭据 client）"
        );
        handlers.register(registry);
        registered.extend([
            StageKind::TripoUpload,
            StageKind::TripoSubmit,
            StageKind::TripoPoll,
            StageKind::ModelDownload,
            StageKind::ModelValidate,
        ]);
    }
    Ok(registered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        Concurrency, Jobs, Limits, ProviderSettings, Providers, SecretString, Session,
    };
    use std::path::PathBuf;

    fn settings_with_tripo(api_key: Option<&str>) -> Settings {
        Settings {
            config_path: None,
            data_dir: PathBuf::from("/tmp/em-provider-test"),
            listen: "127.0.0.1:8080".parse().expect("监听地址"),
            public_origin: None,
            tls: None,
            trusted_proxy_cidrs: Vec::new(),
            providers: Providers {
                tripo: ProviderSettings {
                    name: "tripo",
                    base_url: crate::config::DEFAULT_TRIPO_BASE_URL.to_owned(),
                    model: Some(crate::config::DEFAULT_TRIPO_MODEL.to_owned()),
                    api_key: api_key.map(SecretString::new),
                    key_source: api_key.map(|_| "测试注入".to_owned()),
                },
                manual_ai: ProviderSettings {
                    name: "manual_ai",
                    base_url: crate::config::DEFAULT_MANUAL_AI_BASE_URL.to_owned(),
                    model: None,
                    api_key: None,
                    key_source: None,
                },
            },
            limits: Limits::default(),
            concurrency: Concurrency::default(),
            jobs: Jobs::default(),
            session: Session::default(),
            price_catalog_path: None,
            price_catalog: None,
            download: crate::config::DownloadSettings::default(),
        }
    }

    /// 未配置 → 不注册任何处理器（阶段被延后，不假成功、不回退 fixture）。
    #[test]
    fn unconfigured_provider_registers_no_handlers() {
        let settings = settings_with_tripo(None);
        let mut registry = StageRegistry::new();
        let registered =
            register_provider_handlers(&mut registry, &settings).expect("未配置不报错");
        assert!(registered.is_empty());
        assert!(registry.is_empty(), "缺密钥时不得注册任何 Tripo 处理器");
    }

    /// 已配置 → 恰好注册五个 Tripo 阶段处理器（T13 起含下载/校验），且客户端可构造。
    #[test]
    fn configured_provider_registers_five_tripo_stages() {
        let settings = settings_with_tripo(Some("canary-provider-key"));
        let mut registry = StageRegistry::new();
        let registered =
            register_provider_handlers(&mut registry, &settings).expect("已配置必须能注册");
        assert_eq!(
            registered,
            vec![
                StageKind::TripoUpload,
                StageKind::TripoSubmit,
                StageKind::TripoPoll,
                StageKind::ModelDownload,
                StageKind::ModelValidate,
            ]
        );
        assert!(registry.contains(StageKind::ModelDownload));
        assert!(registry.contains(StageKind::ModelValidate));
        // 说明书 AI 未配置（本构造里 manual_ai 无密钥）→ 不得注册手册 AI 阶段。
        assert!(!registry.contains(StageKind::ManualExtract));
        assert!(!registry.contains(StageKind::ManualMerge));
    }

    /// 说明书 AI 已配置（T14）：注册 `manual_extract` / `manual_merge`；
    /// 与 Tripo 相互独立（Tripo 未配置时不注册 Tripo 阶段）。
    #[test]
    fn configured_manual_ai_registers_extract_and_merge_independently() {
        let mut settings = settings_with_tripo(None);
        settings.providers.manual_ai = ProviderSettings {
            name: "manual_ai",
            base_url: crate::config::DEFAULT_MANUAL_AI_BASE_URL.to_owned(),
            model: Some("gpt-test".to_owned()),
            api_key: Some(SecretString::new("canary-manual-ai-key")),
            key_source: Some("测试注入".to_owned()),
        };
        let mut registry = StageRegistry::new();
        let registered =
            register_provider_handlers(&mut registry, &settings).expect("已配置必须能注册");
        assert_eq!(
            registered,
            vec![StageKind::ManualExtract, StageKind::ManualMerge]
        );
        assert!(registry.contains(StageKind::ManualExtract));
        assert!(registry.contains(StageKind::ManualMerge));
        assert!(
            !registry.contains(StageKind::TripoSubmit),
            "Tripo 未配置：不得注册任何 Tripo 阶段"
        );

        // 缺 model（只有密钥）→ `configured()` 为 false → 不注册。
        settings.providers.manual_ai.model = None;
        let mut registry = StageRegistry::new();
        let registered = register_provider_handlers(&mut registry, &settings).expect("不报错");
        assert!(registered.is_empty());
        assert!(registry.is_empty());
    }

    /// 缺 model（只有密钥）时 `configured()` 为 false → 不注册。
    #[test]
    fn provider_without_model_is_not_configured() {
        let mut settings = settings_with_tripo(Some("canary-provider-key"));
        settings.providers.tripo.model = None;
        let mut registry = StageRegistry::new();
        let registered = register_provider_handlers(&mut registry, &settings).expect("不报错");
        assert!(registered.is_empty());
        assert!(registry.is_empty());
    }
}
