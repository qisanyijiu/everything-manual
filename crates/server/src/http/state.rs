//! 应用状态：路由层共享的数据库、配置与限速器（T04）。
//!
//! `serve` 启动时构造一次；单元/集成测试用同一构造函数装配真实的 SQLite data-dir。
//! 状态内不含会话明文，也没有任何"后门"入口（REQ-002：无匿名访问、无临时后门）。

use std::sync::Arc;
use std::time::Duration;

use crate::assets::AssetStore;
use crate::config::Settings;
use crate::storage::Database;

use super::auth::limiter::LoginRateLimiter;

/// 路由共享状态（`Clone` 只克隆连接池、Arc 与服务句柄）。
#[derive(Clone)]
pub struct AppState {
    database: Database,
    settings: Arc<Settings>,
    login_limiter: Arc<LoginRateLimiter>,
    assets: AssetStore,
    provider_config:
        Arc<tokio::sync::RwLock<crate::config::provider_overrides::ProviderConfigStore>>,
}

impl AppState {
    /// 按配置构造：会话 TTL、登录限速与 cookie Secure 判定都取自 [`Settings`]；
    /// 资产服务使用 data-dir + 真实剩余空间探测。
    pub fn new(database: Database, settings: Settings) -> Self {
        let assets = AssetStore::new(settings.data_dir.clone());
        Self::with_asset_store(database, settings, assets)
    }

    /// 注入资产服务句柄（测试用：磁盘不足场景需要可控的剩余空间探测）。
    pub fn with_asset_store(database: Database, settings: Settings, assets: AssetStore) -> Self {
        let login_limiter = LoginRateLimiter::new(
            settings.session.login_rate_limit_per_minute,
            Duration::from_secs(settings.session.login_rate_limit_window_seconds),
        );
        Self {
            database,
            provider_config: Arc::new(tokio::sync::RwLock::new(
                crate::config::provider_overrides::ProviderConfigStore::deployment(&settings),
            )),
            settings: Arc::new(settings),
            login_limiter: Arc::new(login_limiter),
            assets,
        }
    }

    pub fn database(&self) -> &Database {
        &self.database
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// 显式装配已加载的配置；HTTP 与 worker 必须使用同一份应用后的 Settings。
    pub fn with_provider_config(
        mut self,
        config: crate::config::provider_overrides::ProviderConfigStore,
    ) -> Self {
        self.provider_config = Arc::new(tokio::sync::RwLock::new(config));
        self
    }

    pub fn provider_config(
        &self,
    ) -> &tokio::sync::RwLock<crate::config::provider_overrides::ProviderConfigStore> {
        &self.provider_config
    }
    pub fn provider_config_handle(
        &self,
    ) -> Arc<tokio::sync::RwLock<crate::config::provider_overrides::ProviderConfigStore>> {
        self.provider_config.clone()
    }

    pub fn login_limiter(&self) -> &LoginRateLimiter {
        &self.login_limiter
    }

    /// 资产服务（上传/内容服务共用；`data-dir` 与空间探测都在这里）。
    pub fn assets(&self) -> &AssetStore {
        &self.assets
    }
}

impl std::fmt::Debug for AppState {
    /// 不实现派生 Debug：`Settings` 含密钥包装，调试输出只给结构摘要。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("dataDir", &self.database.data_dir())
            .finish_non_exhaustive()
    }
}
