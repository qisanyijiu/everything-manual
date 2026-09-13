//! 配置、CLI、日志与 data-dir 生命周期（T02）。
//!
//! 对外合同：PRD §3 REQ-003/REQ-005（占位）/REQ-007、validation-release.md §5
//! 管理员运行合同、architecture.md §6/§7。
//!
//! 关键规则：
//! - 配置优先级：**CLI 非密钥项 > 环境变量 > TOML > 默认**；未知配置键报错，不静默忽略；
//! - 密钥不进配置文件：`api_key_env`（环境变量名）或 `api_key_file`（受限文件，0600）二选一；
//! - 缺密钥不影响启动（可浏览已有资料），但配置状态如实标注“未配置”，**不存在 mock 回退**
//!   （T02 尚没有任何 Provider 实现，这是构造性保证；T12/T14 接入时必须保持显式互斥）；
//! - 非 loopback 监听必须配置内置 TLS 或可信反向代理，否则拒绝启动（§7）；
//! - 相对路径一律相对进程工作目录解析（文档写明，避免歧义）。

pub mod cli;
pub mod commands;
pub mod datadir;
pub mod error;
pub mod file;
pub mod logging;
pub mod password;
pub mod secret;

pub use error::{CliError, ExitCode};
pub use secret::SecretString;

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use crate::generation::catalog::{self, PriceCatalog};

use file::FileConfig;

/// 默认监听地址（PRD §5.1；只在 loopback 上监听）。
pub const DEFAULT_LISTEN: &str = "127.0.0.1:8080";
/// Tripo v3 全球基础地址（architecture.md §5.3）。
pub const DEFAULT_TRIPO_BASE_URL: &str = "https://openapi.tripo3d.ai/v3";
/// Tripo H 系列默认模型（architecture.md §5.3 价格快照 2026-09-11）。
pub const DEFAULT_TRIPO_MODEL: &str = "v3.1-20260211";
/// 说明书 AI 参考适配器基础地址（OpenAI Responses；architecture.md §5.2）。
pub const DEFAULT_MANUAL_AI_BASE_URL: &str = "https://api.openai.com/v1";
/// 默认配置文件名（`<data-dir>/config.toml` 或工作目录下）。
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// 并发上限（architecture.md §6：可降低，不可未经确认提高）。
pub const MAX_REMOTE_GENERATION_CONCURRENCY: u32 = 2;
pub const MAX_MANUAL_AI_BATCH_CONCURRENCY: u32 = 2;

/// CLI 非密钥覆盖项（最高优先级）。
#[derive(Debug, Default, Clone)]
pub struct CliOverrides {
    pub data_dir: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub listen: Option<SocketAddr>,
}

/// 监听安全模式（serve 放行时的依据，check 同样校验）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityMode {
    /// loopback 监听（默认；无需 TLS）。
    Loopback,
    /// 非 loopback + 显式可信反向代理网段（明文监听，但信任边界显式声明）。
    TrustedProxy,
}

impl SecurityMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SecurityMode::Loopback => "loopback",
            SecurityMode::TrustedProxy => "可信反向代理模式",
        }
    }
}

/// 解析后的配置（密钥已按 `api_key_env`/`api_key_file` 注入；Debug 输出不含密钥明文）。
#[derive(Debug, Clone)]
pub struct Settings {
    /// 实际读取的配置文件（None = 未找到，使用内置默认）。
    pub config_path: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub listen: SocketAddr,
    pub public_origin: Option<String>,
    pub tls: Option<TlsConfig>,
    pub trusted_proxy_cidrs: Vec<Cidr>,
    pub providers: Providers,
    pub limits: Limits,
    pub concurrency: Concurrency,
    /// 任务执行器租约/续约（T10）。
    pub jobs: Jobs,
    pub session: Session,
    pub price_catalog_path: Option<PathBuf>,
    /// 解析后的价格目录（T11；路径未配置时为 `None` = 生成不可用，返回
    /// 409 `PRICE_CATALOG_MISSING`）。文件存在但非法在启动时即失败（退出码 3），
    /// 不带着猜测价格运行。
    pub price_catalog: Option<PriceCatalog>,
    /// 模型产物下载的允许域策略（T13；默认空名单 = 拒绝一切下载，不猜测 CDN 域名）。
    pub download: DownloadSettings,
}

/// 模型产物下载的允许域策略（T13 / REQ-028；architecture.md §7）。
///
/// - `allowed_hosts`：**精确匹配**（小写、不含端口/通配）的允许域名单；空 = 拒绝一切下载；
/// - `allow_local_fixture`：**仅测试配置**（本机 fixture 的明文 http + 回环地址）。
///   即使配置为 true，也只在测试构建中生效（生产构建忽略，见
///   `assets::glb::download::TEST_BUILD_SWITCH`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadSettings {
    pub allowed_hosts: Vec<String>,
    pub allow_local_fixture: bool,
}

impl DownloadSettings {
    /// `check`/状态摘要行（不含任何密钥；只显示策略本身）。
    pub fn summary_line(&self) -> String {
        let hosts = if self.allowed_hosts.is_empty() {
            "(空：拒绝一切下载；未配置允许域时不猜测供应商 CDN 域名)".to_owned()
        } else {
            self.allowed_hosts.join(", ")
        };
        format!(
            "allowed_hosts=[{hosts}] allow_local_fixture={}（仅测试构建生效）",
            self.allow_local_fixture
        )
    }
}

/// 会话与登录限速（PRD §5.1 / 假设 A-05；默认值为安全默认值，可配置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    /// 会话绝对有效期（小时；默认 168 = 7 天，无滑动续期）。
    pub ttl_hours: i64,
    /// 登录失败限速：窗口内允许的失败次数（默认 5）。
    pub login_rate_limit_per_minute: u32,
    /// 登录失败限速窗口（秒；默认 60，即"5 次/分钟"）。
    pub login_rate_limit_window_seconds: u64,
    /// cookie `Secure` 策略（默认 [`CookieSecurePolicy::Auto`]）。
    pub cookie_secure: CookieSecurePolicy,
}

/// cookie `Secure` 属性策略。
///
/// 判定只依赖**服务端已知事实**，不使用任何 `X-Forwarded-*`（architecture.md §7：
/// 未验证的转发头不得决定 cookie 安全属性）：
/// - `Auto`（默认）：`public_origin` 是 `https://` 或配置了内置 TLS 时加 `Secure`；
/// - `Always` / `Never`：运维显式声明（例如已在可信反向代理后终止 TLS）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CookieSecurePolicy {
    #[default]
    Auto,
    Always,
    Never,
}

impl Session {
    /// 默认会话 TTL（小时）= 7 天。
    pub const DEFAULT_TTL_HOURS: i64 = 168;
    /// 默认登录失败限速 = 5 次/分钟。
    pub const DEFAULT_LOGIN_RATE_LIMIT: u32 = 5;
    /// 默认限速窗口（秒）。
    pub const DEFAULT_LOGIN_RATE_LIMIT_WINDOW_SECONDS: u64 = 60;
}

impl Default for Session {
    fn default() -> Self {
        Self {
            ttl_hours: Self::DEFAULT_TTL_HOURS,
            login_rate_limit_per_minute: Self::DEFAULT_LOGIN_RATE_LIMIT,
            login_rate_limit_window_seconds: Self::DEFAULT_LOGIN_RATE_LIMIT_WINDOW_SECONDS,
            cookie_secure: CookieSecurePolicy::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Providers {
    pub tripo: ProviderSettings,
    pub manual_ai: ProviderSettings,
}

/// 单个供应商的解析结果。`api_key` 为 None 表示“未配置”——生成能力不可用，
/// 但服务仍可启动浏览已有资料（REQ-007）；**不存在任何 mock 回退分支**。
#[derive(Debug, Clone)]
pub struct ProviderSettings {
    pub name: &'static str,
    pub base_url: String,
    pub model: Option<String>,
    pub api_key: Option<SecretString>,
    /// 密钥来源描述（如“环境变量 TRIPO_API_KEY”“受限文件 /etc/em/tripo.key”），
    /// 用于 `/settings/status` 与 `check` 展示；不含密钥内容。
    pub key_source: Option<String>,
}

impl ProviderSettings {
    /// 是否具备发起真实请求的全部条件（密钥 + 模型）。
    pub fn configured(&self) -> bool {
        self.api_key.is_some() && self.model.is_some()
    }

    /// 缺失项（用于可行动的错误说明；不返回密钥本身）。
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.api_key.is_none() {
            missing.push("api_key");
        }
        if self.model.is_none() {
            missing.push("model");
        }
        missing
    }

    /// `check`/状态展示行：只说明配置与否与来源，不含密钥。
    pub fn status_line(&self) -> String {
        if self.configured() {
            let source = self.key_source.as_deref().unwrap_or("未知");
            format!("已配置（密钥来源：{source}）")
        } else {
            let missing = self.missing().join("、");
            format!("未配置（缺少：{missing}；生成与报价不可用，已有资料仍可读）")
        }
    }
}

/// 输入体积/页数限制（PRD §5.3 默认值）。
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_json_request_bytes: u64,
    pub max_pdf_bytes: u64,
    pub max_pdf_pages: u32,
    pub max_photo_bytes: u64,
    pub max_glb_bytes: u64,
    pub max_item_total_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_json_request_bytes: 1_048_576, // 1 MiB
            max_pdf_bytes: 50 * 1_048_576,     // 50 MiB
            max_pdf_pages: 100,
            max_photo_bytes: 20 * 1_048_576,       // 20 MiB
            max_glb_bytes: 150 * 1_048_576,        // 150 MiB
            max_item_total_bytes: 500 * 1_048_576, // 500 MiB
        }
    }
}

impl Limits {
    pub fn summary(&self) -> String {
        format!(
            "json={}B pdf={}B/{}页 photo={}B glb={}B item_total={}B",
            self.max_json_request_bytes,
            self.max_pdf_bytes,
            self.max_pdf_pages,
            self.max_photo_bytes,
            self.max_glb_bytes,
            self.max_item_total_bytes
        )
    }
}

/// 任务执行器（T10）：租约与续约间隔（架构 §6 默认 120s / 20s）。
///
/// 可以调小（测试与低配环境），**不可调大到与续约脱节**：`renew < lease` 是硬约束，
/// 否则租约会在续约前过期，运行中的阶段会被恢复扫描接管。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jobs {
    pub lease_seconds: u64,
    pub renew_seconds: u64,
}

impl Jobs {
    /// 默认租约 120 秒（架构 §6）。
    pub const DEFAULT_LEASE_SECONDS: u64 = 120;
    /// 默认续约间隔 20 秒（架构 §6）。
    pub const DEFAULT_RENEW_SECONDS: u64 = 20;
    /// 租约上限：允许调小，也允许在合理范围内调大（不得超过 10 倍默认值）。
    pub const MAX_LEASE_SECONDS: u64 = 600;
}

impl Default for Jobs {
    fn default() -> Self {
        Self {
            lease_seconds: Jobs::DEFAULT_LEASE_SECONDS,
            renew_seconds: Jobs::DEFAULT_RENEW_SECONDS,
        }
    }
}

/// 并发上限（架构 §6 默认值）。
#[derive(Debug, Clone, Copy)]
pub struct Concurrency {
    pub remote_generation: u32,
    pub manual_ai_batches: u32,
}

impl Default for Concurrency {
    fn default() -> Self {
        Self {
            remote_generation: MAX_REMOTE_GENERATION_CONCURRENCY,
            manual_ai_batches: MAX_MANUAL_AI_BATCH_CONCURRENCY,
        }
    }
}

/// 单个 CIDR 网段（可信反向代理判断，T04 消费）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl Cidr {
    pub fn parse(value: &str) -> Result<Self, String> {
        let (addr_part, prefix_part) = value
            .split_once('/')
            .ok_or_else(|| format!("缺少前缀长度（形如 10.0.0.0/8）：{value}"))?;
        let addr: IpAddr = addr_part
            .trim()
            .parse()
            .map_err(|_| format!("IP 地址无法解析：{value}"))?;
        let prefix: u8 = prefix_part
            .trim()
            .parse()
            .map_err(|_| format!("前缀长度无法解析：{value}"))?;
        let max = match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix > max {
            return Err(format!("前缀长度超出范围（{max} 以内）：{value}"));
        }
        Ok(Self { addr, prefix })
    }

    /// 判断 IP 是否落在本网段内。
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(network), IpAddr::V4(candidate)) => {
                let mask = prefix_mask_u32(self.prefix);
                (u32::from(network) & mask) == (u32::from(candidate) & mask)
            }
            (IpAddr::V6(network), IpAddr::V6(candidate)) => {
                let mask = prefix_mask_u128(self.prefix);
                (u128::from(network) & mask) == (u128::from(candidate) & mask)
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for Cidr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

fn prefix_mask_u32(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    }
}

fn prefix_mask_u128(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    }
}

impl Settings {
    /// 加载并解析配置（优先级 CLI > 环境 > TOML > 默认）。
    pub fn load(overrides: &CliOverrides) -> Result<Self, CliError> {
        let cwd = std::env::current_dir()
            .map_err(|error| CliError::internal(format!("无法获取当前工作目录：{error}")))?;

        // 1) 定位配置文件：--config / EM_CONFIG > <data-dir>/config.toml > ./config.toml。
        let explicit = overrides
            .config
            .clone()
            .or_else(|| file::env_nonempty(file::ENV_CONFIG).map(PathBuf::from));
        let (config_path, mut config) = match explicit {
            Some(path) => {
                let path = absolute_from(&cwd, &path);
                if !path.is_file() {
                    return Err(CliError::config(format!(
                        "配置文件不存在：{}（来自 --config 或 EM_CONFIG）",
                        path.display()
                    )));
                }
                let parsed = file::load(&path)?;
                (Some(path), parsed)
            }
            None => {
                let data_dir_hint = overrides
                    .data_dir
                    .clone()
                    .or_else(|| file::env_nonempty(file::ENV_DATA_DIR).map(PathBuf::from));
                let candidate = data_dir_hint
                    .map(|dir| absolute_from(&cwd, &dir).join(CONFIG_FILE_NAME))
                    .filter(|path| path.is_file())
                    .or_else(|| {
                        let path = cwd.join(CONFIG_FILE_NAME);
                        path.is_file().then_some(path)
                    });
                match candidate {
                    Some(path) => {
                        let parsed = file::load(&path)?;
                        (Some(path), parsed)
                    }
                    None => (None, FileConfig::default()),
                }
            }
        };

        // 2) 环境变量 > TOML。
        file::apply_env(&mut config)?;

        // 3) CLI 非密钥项 > 环境变量。
        if let Some(dir) = &overrides.data_dir {
            config.data_dir = Some(dir.to_string_lossy().into_owned());
        }
        if let Some(listen) = overrides.listen {
            config.listen = Some(listen.to_string());
        }

        // 4) 解析与校验（默认值收口处）。
        let data_dir = match config.data_dir.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => absolute_from(&cwd, Path::new(value)),
            _ => {
                return Err(CliError::config(
                    "缺少 data_dir：请使用 --data-dir <dir>、环境变量 EM_DATA_DIR 或配置键 data_dir",
                ));
            }
        };

        let listen: SocketAddr = match config.listen.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => value.parse().map_err(|_| {
                CliError::config(format!(
                    "listen 必须是 IP:端口（例如 127.0.0.1:8080），无法解析：{value}"
                ))
            })?,
            _ => DEFAULT_LISTEN
                .parse()
                .expect("默认监听地址是合法 SocketAddr"),
        };

        let public_origin = config
            .public_origin
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| validate_origin_like("public_origin", value))
            .transpose()?;

        let price_catalog_path = config
            .price_catalog_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| absolute_from(&cwd, Path::new(value)));
        if let Some(path) = &price_catalog_path
            && !path.is_file()
        {
            return Err(CliError::config(format!(
                "price_catalog_path 指向的文件不存在：{}",
                path.display()
            )));
        }
        // T11：价格目录在启动时一次性解析（精确十进制换算）；非法目录 = 配置错误，
        // 不降级为"没有价格"继续运行（避免用猜测的价格生成任务）。
        let price_catalog = match &price_catalog_path {
            Some(path) => Some(catalog::load(path).map_err(|catalog_error| {
                CliError::config(format!(
                    "price_catalog_path 的价格目录不可用（{}）：{catalog_error}",
                    path.display()
                ))
            })?),
            None => None,
        };

        let tls = resolve_tls(&cwd, &config)?;
        let trusted_proxy_cidrs = resolve_cidrs(&config)?;
        let limits = resolve_limits(&config)?;
        let concurrency = resolve_concurrency(&config)?;
        let jobs = resolve_jobs(&config)?;
        let session = resolve_session(&config)?;
        let providers = resolve_providers(&cwd, &config)?;
        let download = resolve_download(&config)?;

        Ok(Self {
            config_path,
            data_dir,
            listen,
            public_origin,
            tls,
            trusted_proxy_cidrs,
            providers,
            limits,
            concurrency,
            jobs,
            session,
            price_catalog_path,
            price_catalog,
            download,
        })
    }

    /// cookie 是否加 `Secure`（判定依据只含服务端已知事实，见 [`CookieSecurePolicy`]）。
    pub fn cookie_secure(&self) -> bool {
        match self.session.cookie_secure {
            CookieSecurePolicy::Always => true,
            CookieSecurePolicy::Never => false,
            CookieSecurePolicy::Auto => {
                let origin_is_https = self
                    .public_origin
                    .as_deref()
                    .is_some_and(|origin| origin.starts_with("https://"));
                origin_is_https || self.tls.is_some()
            }
        }
    }

    /// 监听安全评估（serve 放行/拒绝、check 校验共用）。
    ///
    /// 规则（PRD §5.1、architecture.md §7）：
    /// - 配置了内置 TLS：T02 尚未实现 rustls 监听 → 明确拒绝，**不会静默降级为明文**；
    /// - loopback：放行（开发/单机默认）；
    /// - 非 loopback：必须显式配置 `trusted_proxy_cidrs`（可信反向代理），否则拒绝启动；
    /// - 任何情况下都不默认相信 `X-Forwarded-*`（T04 起按 trusted_proxy_cidrs 判定来源）。
    pub fn evaluate_listen_security(&self) -> Result<SecurityMode, CliError> {
        if let Some(tls) = &self.tls {
            return Err(CliError::insecure_listen(format!(
                "已配置 TLS 证书（cert={}，key={}），但内置 TLS 监听尚未实现（后续卡提供）；\
                 请移除 tls 配置（loopback 开发）或改用反向代理模式（trusted_proxy_cidrs）",
                tls.cert_file.display(),
                tls.key_file.display()
            )));
        }
        if self.listen.ip().is_loopback() {
            return Ok(SecurityMode::Loopback);
        }
        if !self.trusted_proxy_cidrs.is_empty() {
            return Ok(SecurityMode::TrustedProxy);
        }
        Err(CliError::insecure_listen(format!(
            "拒绝启动：监听地址 {} 不是 loopback，且未配置 tls.cert_file/key_file 或 \
             trusted_proxy_cidrs（可信反向代理）；公网/局域网部署必须使用 TLS 或明确的可信代理模式",
            self.listen
        )))
    }

    /// `check` 输出的有效配置摘要（脱敏：不含任何密钥内容与查询串）。
    pub fn summary_lines(&self) -> Vec<String> {
        let config_source = self
            .config_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "内置默认（未找到配置文件）".to_owned());
        let tls = match &self.tls {
            Some(tls) => format!(
                "已配置（cert={} key={}；内置 TLS 监听未实现，见监听安全）",
                tls.cert_file.display(),
                tls.key_file.display()
            ),
            None => "(未配置)".to_owned(),
        };
        let proxies = if self.trusted_proxy_cidrs.is_empty() {
            "(空)".to_owned()
        } else {
            self.trusted_proxy_cidrs
                .iter()
                .map(Cidr::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        vec![
            format!("配置文件：{config_source}"),
            format!("data_dir = {}", self.data_dir.display()),
            format!("listen = {}", self.listen),
            format!(
                "public_origin = {}",
                self.public_origin.as_deref().unwrap_or("(未设置)")
            ),
            format!("tls = {tls}"),
            format!("trusted_proxy_cidrs = {proxies}"),
            format!("providers.tripo = {}", self.providers.tripo.status_line()),
            format!(
                "providers.manual_ai = {}",
                self.providers.manual_ai.status_line()
            ),
            format!("limits = {}", self.limits.summary()),
            format!(
                "concurrency = remote_generation={} manual_ai_batches={}",
                self.concurrency.remote_generation, self.concurrency.manual_ai_batches
            ),
            format!(
                "jobs = lease={}s renew={}s",
                self.jobs.lease_seconds, self.jobs.renew_seconds
            ),
            format!(
                "session = ttl={}h login_rate_limit={}/{}s cookie_secure={}",
                self.session.ttl_hours,
                self.session.login_rate_limit_per_minute,
                self.session.login_rate_limit_window_seconds,
                self.cookie_secure()
            ),
            format!("download = {}", self.download.summary_line()),
            format!(
                "price_catalog = {}",
                match (&self.price_catalog_path, &self.price_catalog) {
                    (Some(path), Some(catalog)) => format!(
                        "{}（价格版本 {}，快照 {}）",
                        path.display(),
                        catalog.version,
                        catalog.snapshot_date
                    ),
                    (Some(path), None) => format!("{}（未加载）", path.display()),
                    (None, _) => "(未设置；生成与报价不可用)".to_owned(),
                }
            ),
        ]
    }
}

fn resolve_providers(cwd: &Path, config: &FileConfig) -> Result<Providers, CliError> {
    let tripo_section = config
        .providers
        .as_ref()
        .and_then(|providers| providers.tripo.as_ref());
    let manual_ai_section = config
        .providers
        .as_ref()
        .and_then(|providers| providers.manual_ai.as_ref());

    let tripo = resolve_provider(
        "providers.tripo",
        "tripo",
        DEFAULT_TRIPO_BASE_URL,
        Some(DEFAULT_TRIPO_MODEL),
        tripo_section,
        cwd,
    )?;
    let manual_ai = resolve_provider(
        "providers.manual_ai",
        "manual_ai",
        DEFAULT_MANUAL_AI_BASE_URL,
        None,
        manual_ai_section,
        cwd,
    )?;
    Ok(Providers { tripo, manual_ai })
}

fn resolve_provider(
    key: &str,
    name: &'static str,
    default_base_url: &str,
    default_model: Option<&str>,
    section: Option<&file::ProviderSection>,
    cwd: &Path,
) -> Result<ProviderSettings, CliError> {
    let base_url = section
        .and_then(|section| section.base_url.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_base_url);
    let base_url = validate_origin_like(&format!("{key}.base_url"), base_url)?;

    let model = section
        .and_then(|section| section.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| default_model.map(str::to_owned));

    let api_key_env = section
        .and_then(|section| section.api_key_env.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let api_key_file = section
        .and_then(|section| section.api_key_file.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| absolute_from(cwd, Path::new(value)));

    if api_key_env.is_some() && api_key_file.is_some() {
        return Err(CliError::config(format!(
            "{key} 的 api_key_env 与 api_key_file 只能设置一个"
        )));
    }

    let (api_key, key_source) = match (api_key_env, api_key_file) {
        (Some(env_name), None) => match file::env_nonempty(env_name) {
            Some(value) => (
                Some(SecretString::new(value)),
                Some(format!("环境变量 {env_name}")),
            ),
            None => (None, None),
        },
        (None, Some(path)) => {
            let secret = password::read_restricted_file(&path, "密钥文件").map_err(|error| {
                CliError::config(format!("{key}.api_key_file 不可用：{}", error.message))
            })?;
            (Some(secret), Some(format!("受限文件 {}", path.display())))
        }
        _ => (None, None),
    };

    Ok(ProviderSettings {
        name,
        base_url,
        model,
        api_key,
        key_source,
    })
}

fn resolve_tls(cwd: &Path, config: &FileConfig) -> Result<Option<TlsConfig>, CliError> {
    let Some(section) = &config.tls else {
        return Ok(None);
    };
    let cert = section
        .cert_file
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let key = section
        .key_file
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (cert, key) {
        (None, None) => Err(CliError::config(
            "tls 段已设置但缺少 cert_file/key_file；如需内置 TLS 需两者成对配置",
        )),
        (Some(_), None) | (None, Some(_)) => Err(CliError::config(
            "tls.cert_file 与 tls.key_file 必须成对配置",
        )),
        (Some(cert), Some(key)) => {
            let cert_file = absolute_from(cwd, Path::new(cert));
            let key_file = absolute_from(cwd, Path::new(key));
            require_pem_file(&cert_file, "tls.cert_file")?;
            require_pem_file(&key_file, "tls.key_file")?;
            Ok(Some(TlsConfig {
                cert_file,
                key_file,
            }))
        }
    }
}

/// TLS 证书/私钥的最低限度校验：存在、可读、看起来是 PEM（避免明显配错静默通过）。
fn require_pem_file(path: &Path, key: &str) -> Result<(), CliError> {
    let head = std::fs::read(path)
        .map_err(|error| CliError::config(format!("{key} 无法读取 {}：{error}", path.display())))?;
    let head = &head[..head.len().min(128)];
    let head = String::from_utf8_lossy(head);
    if !head.contains("-----BEGIN") {
        return Err(CliError::config(format!(
            "{key} 不是 PEM 文件（未找到 -----BEGIN 头）：{}",
            path.display()
        )));
    }
    Ok(())
}

fn resolve_cidrs(config: &FileConfig) -> Result<Vec<Cidr>, CliError> {
    let mut cidrs = Vec::new();
    for value in config.trusted_proxy_cidrs.iter().flatten() {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let cidr = Cidr::parse(value)
            .map_err(|error| CliError::config(format!("trusted_proxy_cidrs 非法（{error}）")))?;
        cidrs.push(cidr);
    }
    Ok(cidrs)
}

fn resolve_limits(config: &FileConfig) -> Result<Limits, CliError> {
    let mut limits = Limits::default();
    let Some(section) = &config.limits else {
        return Ok(limits);
    };
    if let Some(value) = section.max_json_request_bytes {
        limits.max_json_request_bytes = value;
    }
    if let Some(value) = section.max_pdf_bytes {
        limits.max_pdf_bytes = value;
    }
    if let Some(value) = section.max_pdf_pages {
        if value == 0 {
            return Err(CliError::config("limits.max_pdf_pages 必须大于 0"));
        }
        limits.max_pdf_pages = value;
    }
    if let Some(value) = section.max_photo_bytes {
        limits.max_photo_bytes = value;
    }
    if let Some(value) = section.max_glb_bytes {
        limits.max_glb_bytes = value;
    }
    if let Some(value) = section.max_item_total_bytes {
        limits.max_item_total_bytes = value;
    }
    for (name, value) in [
        (
            "limits.max_json_request_bytes",
            limits.max_json_request_bytes,
        ),
        ("limits.max_pdf_bytes", limits.max_pdf_bytes),
        ("limits.max_photo_bytes", limits.max_photo_bytes),
        ("limits.max_glb_bytes", limits.max_glb_bytes),
        ("limits.max_item_total_bytes", limits.max_item_total_bytes),
    ] {
        if value == 0 {
            return Err(CliError::config(format!("{name} 必须大于 0")));
        }
    }
    Ok(limits)
}

fn resolve_session(config: &FileConfig) -> Result<Session, CliError> {
    let mut session = Session::default();
    let Some(section) = &config.session else {
        return Ok(session);
    };
    if let Some(value) = section.ttl_hours {
        if value <= 0 {
            return Err(CliError::config("session.ttl_hours 必须大于 0"));
        }
        session.ttl_hours = value;
    }
    if let Some(value) = section.login_rate_limit_per_minute {
        if value == 0 {
            return Err(CliError::config(
                "session.login_rate_limit_per_minute 必须大于 0（限速不可关闭）",
            ));
        }
        session.login_rate_limit_per_minute = value;
    }
    if let Some(value) = section.login_rate_limit_window_seconds {
        if value == 0 {
            return Err(CliError::config(
                "session.login_rate_limit_window_seconds 必须大于 0",
            ));
        }
        session.login_rate_limit_window_seconds = value;
    }
    if let Some(value) = &section.cookie_secure {
        session.cookie_secure = match value.as_str() {
            "auto" => CookieSecurePolicy::Auto,
            "always" => CookieSecurePolicy::Always,
            "never" => CookieSecurePolicy::Never,
            other => {
                return Err(CliError::config(format!(
                    "session.cookie_secure 只能是 auto/always/never，实际：{other}"
                )));
            }
        };
    }
    Ok(session)
}

/// 下载允许域策略（T13）：键名与取值都在启动时校验，非法配置**明确报错**，
/// 不静默忽略（避免"以为配了允许域、实际没配"）。
fn resolve_download(config: &FileConfig) -> Result<DownloadSettings, CliError> {
    let mut settings = DownloadSettings::default();
    let Some(section) = &config.download else {
        return Ok(settings);
    };
    if let Some(hosts) = &section.allowed_hosts {
        for host in hosts {
            let host = host.trim();
            if host.is_empty() {
                continue;
            }
            let normalized = host.to_ascii_lowercase();
            // 只接受主机名（或 IP 字面量）：不含 scheme、端口、路径、通配符与空白。
            if normalized.contains(['/', ':', '?', '#', '@', '*'])
                || normalized.contains(char::is_whitespace)
            {
                return Err(CliError::config(format!(
                    "download.allowed_hosts 只接受精确主机名（不含 scheme/端口/路径/通配符）：{host}"
                )));
            }
            if !settings.allowed_hosts.contains(&normalized) {
                settings.allowed_hosts.push(normalized);
            }
        }
    }
    if let Some(value) = section.allow_local_fixture {
        settings.allow_local_fixture = value;
    }
    Ok(settings)
}

fn resolve_concurrency(config: &FileConfig) -> Result<Concurrency, CliError> {
    let mut concurrency = Concurrency::default();
    let Some(section) = &config.concurrency else {
        return Ok(concurrency);
    };
    if let Some(value) = section.remote_generation {
        if !(1..=MAX_REMOTE_GENERATION_CONCURRENCY).contains(&value) {
            return Err(CliError::config(format!(
                "concurrency.remote_generation 取值须在 1..={MAX_REMOTE_GENERATION_CONCURRENCY}（架构 §6：可降低，不可未经确认提高）"
            )));
        }
        concurrency.remote_generation = value;
    }
    if let Some(value) = section.manual_ai_batches {
        if !(1..=MAX_MANUAL_AI_BATCH_CONCURRENCY).contains(&value) {
            return Err(CliError::config(format!(
                "concurrency.manual_ai_batches 取值须在 1..={MAX_MANUAL_AI_BATCH_CONCURRENCY}（架构 §6：可降低，不可未经确认提高）"
            )));
        }
        concurrency.manual_ai_batches = value;
    }
    Ok(concurrency)
}

/// 任务执行器配置（T10）：默认 120s 租约 / 20s 续约；必须满足 `1 <= renew < lease`。
fn resolve_jobs(config: &FileConfig) -> Result<Jobs, CliError> {
    let mut jobs = Jobs::default();
    let Some(section) = &config.jobs else {
        return Ok(jobs);
    };
    if let Some(value) = section.lease_seconds {
        if value == 0 || value > Jobs::MAX_LEASE_SECONDS {
            return Err(CliError::config(format!(
                "jobs.lease_seconds 取值须在 1..={}（默认 {}；过大的租约会让崩溃恢复长时间不生效）",
                Jobs::MAX_LEASE_SECONDS,
                Jobs::DEFAULT_LEASE_SECONDS
            )));
        }
        jobs.lease_seconds = value;
    }
    if let Some(value) = section.renew_seconds {
        if value == 0 {
            return Err(CliError::config(
                "jobs.renew_seconds 必须大于 0（续约不可关闭）",
            ));
        }
        jobs.renew_seconds = value;
    }
    if jobs.renew_seconds >= jobs.lease_seconds {
        return Err(CliError::config(format!(
            "jobs.renew_seconds（{}s）必须小于 jobs.lease_seconds（{}s），否则租约在续约前过期",
            jobs.renew_seconds, jobs.lease_seconds
        )));
    }
    Ok(jobs)
}

/// 校验并规范化 `http(s)://host[:port][/path]` 形式的值（拒绝查询串与空白）。
fn validate_origin_like(key: &str, value: &str) -> Result<String, CliError> {
    let value = value.trim();
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .ok_or_else(|| {
            CliError::config(format!(
                "{key} 必须是 http:// 或 https:// 开头的地址：{value}"
            ))
        })?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() {
        return Err(CliError::config(format!("{key} 缺少主机名：{value}")));
    }
    if value.contains(char::is_whitespace) || value.contains('?') || value.contains('#') {
        return Err(CliError::config(format!(
            "{key} 不允许包含空白、查询串或片段：{value}"
        )));
    }
    Ok(value.trim_end_matches('/').to_owned())
}

/// 相对路径按进程工作目录解析为绝对路径（文档写明）。
///
/// 仅做字面上的 `.` 组件清理；**不解析 `..`、不解析符号链接**（避免改变实际访问语义）。
fn absolute_from(cwd: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        if component == std::path::Component::CurDir {
            continue;
        }
        out.push(component.as_os_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_parse_and_contains() {
        let cidr = Cidr::parse("10.0.0.0/8").unwrap();
        assert!(cidr.contains("10.1.2.3".parse().unwrap()));
        assert!(!cidr.contains("11.0.0.1".parse().unwrap()));

        let v6 = Cidr::parse("::1/128").unwrap();
        assert!(v6.contains("::1".parse().unwrap()));
        assert!(!v6.contains("::2".parse().unwrap()));
        assert!(!v6.contains("10.0.0.1".parse().unwrap()));

        let zero = Cidr::parse("0.0.0.0/0").unwrap();
        assert!(zero.contains("203.0.113.9".parse().unwrap()));

        assert!(Cidr::parse("10.0.0.0").is_err());
        assert!(Cidr::parse("10.0.0.0/33").is_err());
        assert!(Cidr::parse("not-an-ip/8").is_err());
    }

    #[test]
    fn origin_like_validation() {
        assert_eq!(
            validate_origin_like("public_origin", "https://example.com/").unwrap(),
            "https://example.com"
        );
        assert!(validate_origin_like("public_origin", "ftp://example.com").is_err());
        assert!(validate_origin_like("public_origin", "https://").is_err());
        assert!(validate_origin_like("public_origin", "https://example.com?a=1").is_err());
        assert!(validate_origin_like("public_origin", "https://exa mple.com").is_err());
    }

    #[test]
    fn provider_status_reports_missing_without_secret() {
        let provider = ProviderSettings {
            name: "tripo",
            base_url: DEFAULT_TRIPO_BASE_URL.to_owned(),
            model: Some(DEFAULT_TRIPO_MODEL.to_owned()),
            api_key: None,
            key_source: None,
        };
        assert!(!provider.configured());
        assert_eq!(provider.missing(), vec!["api_key"]);
        let line = provider.status_line();
        assert!(line.contains("未配置"), "{line}");
        assert!(!line.contains("mock"), "{line}");

        let configured = ProviderSettings {
            api_key: Some(SecretString::new("sk-very-secret")),
            key_source: Some("环境变量 TRIPO_API_KEY".to_owned()),
            ..provider.clone()
        };
        let line = configured.status_line();
        assert!(line.contains("已配置"), "{line}");
        assert!(!line.contains("sk-very-secret"), "{line}");
    }

    #[test]
    fn settings_debug_does_not_leak_api_key() {
        let settings = Settings {
            config_path: None,
            data_dir: PathBuf::from("/tmp/em"),
            listen: DEFAULT_LISTEN.parse().unwrap(),
            public_origin: None,
            tls: None,
            trusted_proxy_cidrs: Vec::new(),
            providers: Providers {
                tripo: ProviderSettings {
                    name: "tripo",
                    base_url: DEFAULT_TRIPO_BASE_URL.to_owned(),
                    model: Some(DEFAULT_TRIPO_MODEL.to_owned()),
                    api_key: Some(SecretString::new("sk-leak-canary")),
                    key_source: Some("环境变量 TRIPO_API_KEY".to_owned()),
                },
                manual_ai: ProviderSettings {
                    name: "manual_ai",
                    base_url: DEFAULT_MANUAL_AI_BASE_URL.to_owned(),
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
            download: DownloadSettings::default(),
        };
        let debug = format!("{settings:?}");
        assert!(!debug.contains("sk-leak-canary"), "{debug}");
        let summary = settings.summary_lines().join("\n");
        assert!(!summary.contains("sk-leak-canary"), "{summary}");
        assert!(summary.contains("providers.tripo = 已配置"), "{summary}");
    }
}
