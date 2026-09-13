//! 配置文件（TOML）与环境变量覆盖。
//!
//! 合同（PRD REQ-003、validation-release.md §5）：
//! - 配置键为 snake_case；**未知键是错误**（每个层级 `deny_unknown_fields`），不静默忽略；
//! - 优先级：CLI 非密钥项 > 环境变量 > TOML > 默认（本模块负责“环境变量 > TOML”一层）；
//! - 密钥不写入配置文件：`api_key_env` 指向承载密钥的环境变量名，
//!   `api_key_file` 指向受限权限文件（二选一，见 `mod.rs::resolve_provider`）。
//!
//! 环境变量名表是**显式白名单**：只识别下表中列出的变量，其他 `EM_*` 变量一律忽略，
//! 避免外部环境（CI、shell 配置）意外改变服务行为。命名规则：`EM_` + 键路径大写，
//! 层级用 `__` 分隔（如 `EM_PROVIDERS__TRIPO__API_KEY_ENV`）。

use std::path::Path;

use serde::Deserialize;

use super::error::CliError;

pub const ENV_CONFIG: &str = "EM_CONFIG";
pub const ENV_DATA_DIR: &str = "EM_DATA_DIR";
pub const ENV_LISTEN: &str = "EM_LISTEN";
pub const ENV_PUBLIC_ORIGIN: &str = "EM_PUBLIC_ORIGIN";
pub const ENV_PRICE_CATALOG_PATH: &str = "EM_PRICE_CATALOG_PATH";
pub const ENV_TLS_CERT_FILE: &str = "EM_TLS_CERT_FILE";
pub const ENV_TLS_KEY_FILE: &str = "EM_TLS_KEY_FILE";
pub const ENV_TRUSTED_PROXY_CIDRS: &str = "EM_TRUSTED_PROXY_CIDRS";
pub const ENV_TRIPO_BASE_URL: &str = "EM_PROVIDERS__TRIPO__BASE_URL";
pub const ENV_TRIPO_MODEL: &str = "EM_PROVIDERS__TRIPO__MODEL";
pub const ENV_TRIPO_API_KEY_ENV: &str = "EM_PROVIDERS__TRIPO__API_KEY_ENV";
pub const ENV_TRIPO_API_KEY_FILE: &str = "EM_PROVIDERS__TRIPO__API_KEY_FILE";
pub const ENV_MANUAL_AI_BASE_URL: &str = "EM_PROVIDERS__MANUAL_AI__BASE_URL";
pub const ENV_MANUAL_AI_MODEL: &str = "EM_PROVIDERS__MANUAL_AI__MODEL";
pub const ENV_MANUAL_AI_API_KEY_ENV: &str = "EM_PROVIDERS__MANUAL_AI__API_KEY_ENV";
pub const ENV_MANUAL_AI_API_KEY_FILE: &str = "EM_PROVIDERS__MANUAL_AI__API_KEY_FILE";
pub const ENV_LIMITS_MAX_JSON_REQUEST_BYTES: &str = "EM_LIMITS__MAX_JSON_REQUEST_BYTES";
pub const ENV_LIMITS_MAX_PDF_BYTES: &str = "EM_LIMITS__MAX_PDF_BYTES";
pub const ENV_LIMITS_MAX_PDF_PAGES: &str = "EM_LIMITS__MAX_PDF_PAGES";
pub const ENV_LIMITS_MAX_PHOTO_BYTES: &str = "EM_LIMITS__MAX_PHOTO_BYTES";
pub const ENV_LIMITS_MAX_GLB_BYTES: &str = "EM_LIMITS__MAX_GLB_BYTES";
pub const ENV_LIMITS_MAX_ITEM_TOTAL_BYTES: &str = "EM_LIMITS__MAX_ITEM_TOTAL_BYTES";
pub const ENV_CONCURRENCY_REMOTE_GENERATION: &str = "EM_CONCURRENCY__REMOTE_GENERATION";
pub const ENV_CONCURRENCY_MANUAL_AI_BATCHES: &str = "EM_CONCURRENCY__MANUAL_AI_BATCHES";
pub const ENV_SESSION_TTL_HOURS: &str = "EM_SESSION__TTL_HOURS";
pub const ENV_SESSION_LOGIN_RATE_LIMIT_PER_MINUTE: &str = "EM_SESSION__LOGIN_RATE_LIMIT_PER_MINUTE";
pub const ENV_SESSION_LOGIN_RATE_LIMIT_WINDOW_SECONDS: &str =
    "EM_SESSION__LOGIN_RATE_LIMIT_WINDOW_SECONDS";
pub const ENV_SESSION_COOKIE_SECURE: &str = "EM_SESSION__COOKIE_SECURE";
pub const ENV_JOBS_LEASE_SECONDS: &str = "EM_JOBS__LEASE_SECONDS";
pub const ENV_JOBS_RENEW_SECONDS: &str = "EM_JOBS__RENEW_SECONDS";
pub const ENV_DOWNLOAD_ALLOWED_HOSTS: &str = "EM_DOWNLOAD__ALLOWED_HOSTS";
pub const ENV_DOWNLOAD_ALLOW_LOCAL_FIXTURE: &str = "EM_DOWNLOAD__ALLOW_LOCAL_FIXTURE";

/// 读取非空环境变量；未设置或空字符串视为“未提供”（空值不覆盖已有配置）。
pub fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// 配置文件顶层结构（全部可选；未提供的键在解析阶段落到默认值）。
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    /// data-dir 路径；也可由 `--data-dir` / `EM_DATA_DIR` 提供。
    pub data_dir: Option<String>,
    /// 监听地址（IP:端口），默认 `127.0.0.1:8080`。
    pub listen: Option<String>,
    /// 浏览器访问本站的公开来源（`http(s)://host[:port]`），用于 T04 的 Origin 校验。
    pub public_origin: Option<String>,
    /// 价格目录文件路径（T11 消费）；设置时必须是存在的文件。
    pub price_catalog_path: Option<String>,
    pub tls: Option<TlsSection>,
    /// 可信反向代理网段；非 loopback 监听在没有内置 TLS 时必须显式配置（PRD §5.1）。
    pub trusted_proxy_cidrs: Option<Vec<String>>,
    pub providers: Option<ProvidersSection>,
    pub limits: Option<LimitsSection>,
    pub concurrency: Option<ConcurrencySection>,
    /// 任务执行器租约/续约（T10）。
    pub jobs: Option<JobsSection>,
    /// 会话与登录限速（T04；PRD §5.1 / A-05）。
    pub session: Option<SessionSection>,
    /// 模型产物下载的允许域策略（T13；architecture.md §7）。
    pub download: Option<DownloadSection>,
}

/// 模型产物下载策略（T13）。
///
/// `allowed_hosts` 是**精确匹配**（不含端口、不含通配）的允许域名单：空 = 拒绝一切下载
/// （明确报"未配置允许域"，不猜测供应商 CDN 域名）。`allow_local_fixture` 只用于测试配置：
/// 允许明文 http 与回环地址（本机 fixture），且**仅在本 crate 的测试构建中生效**
/// （生产构建忽略并记录告警，见 `assets::glb::download`）。
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DownloadSection {
    pub allowed_hosts: Option<Vec<String>>,
    pub allow_local_fixture: Option<bool>,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TlsSection {
    pub cert_file: Option<String>,
    pub key_file: Option<String>,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProvidersSection {
    pub tripo: Option<ProviderSection>,
    pub manual_ai: Option<ProviderSection>,
}

/// 单个供应商配置。密钥本体不在文件里：`api_key_env` 给出环境变量名，
/// `api_key_file` 给出受限文件路径（二选一）。
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSection {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub api_key_file: Option<String>,
}

/// 输入体积限制（PRD §5.3 默认值；T06/T09/T13 消费）。
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LimitsSection {
    pub max_json_request_bytes: Option<u64>,
    pub max_pdf_bytes: Option<u64>,
    pub max_pdf_pages: Option<u32>,
    pub max_photo_bytes: Option<u64>,
    pub max_glb_bytes: Option<u64>,
    pub max_item_total_bytes: Option<u64>,
}

/// 并发上限（架构 §6：可降低，不可未经确认提高）。
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConcurrencySection {
    pub remote_generation: Option<u32>,
    pub manual_ai_batches: Option<u32>,
}

/// 任务执行器（T10）：租约与续约间隔（秒）。默认 120 / 20（架构 §6）。
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobsSection {
    pub lease_seconds: Option<u64>,
    pub renew_seconds: Option<u64>,
}

/// 会话与登录限速（T04）。`cookie_secure` 是字符串枚举：`auto` / `always` / `never`。
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionSection {
    pub ttl_hours: Option<i64>,
    pub login_rate_limit_per_minute: Option<u32>,
    pub login_rate_limit_window_seconds: Option<u64>,
    pub cookie_secure: Option<String>,
}

/// 从 TOML 文本解析配置；未知键（任意层级）与非 TOML 语法都返回可读错误。
pub fn parse(text: &str, source: &str) -> Result<FileConfig, CliError> {
    toml::from_str::<FileConfig>(text)
        .map_err(|error| CliError::config(format!("配置文件解析失败（{source}）：{error}")))
}

/// 读取并解析配置文件。文件必须存在（调用方负责“可选性”判断）。
pub fn load(path: &Path) -> Result<FileConfig, CliError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        CliError::config(format!("无法读取配置文件 {}：{error}", path.display()))
    })?;
    parse(&text, &path.display().to_string())
}

/// 应用环境变量白名单覆盖（环境变量 > TOML）。
///
/// 只识别本模块列出的常量；整数键解析失败视为配置错误，不静默丢值。
pub fn apply_env(config: &mut FileConfig) -> Result<(), CliError> {
    if let Some(value) = env_nonempty(ENV_DATA_DIR) {
        config.data_dir = Some(value);
    }
    if let Some(value) = env_nonempty(ENV_LISTEN) {
        config.listen = Some(value);
    }
    if let Some(value) = env_nonempty(ENV_PUBLIC_ORIGIN) {
        config.public_origin = Some(value);
    }
    if let Some(value) = env_nonempty(ENV_PRICE_CATALOG_PATH) {
        config.price_catalog_path = Some(value);
    }
    if let Some(value) = env_nonempty(ENV_TLS_CERT_FILE) {
        config.tls.get_or_insert_with(Default::default).cert_file = Some(value);
    }
    if let Some(value) = env_nonempty(ENV_TLS_KEY_FILE) {
        config.tls.get_or_insert_with(Default::default).key_file = Some(value);
    }
    if let Some(value) = env_nonempty(ENV_TRUSTED_PROXY_CIDRS) {
        config.trusted_proxy_cidrs = Some(
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect(),
        );
    }

    let tripo = [
        (ENV_TRIPO_BASE_URL, 0usize),
        (ENV_TRIPO_MODEL, 1),
        (ENV_TRIPO_API_KEY_ENV, 2),
        (ENV_TRIPO_API_KEY_FILE, 3),
    ];
    let manual_ai = [
        (ENV_MANUAL_AI_BASE_URL, 0usize),
        (ENV_MANUAL_AI_MODEL, 1),
        (ENV_MANUAL_AI_API_KEY_ENV, 2),
        (ENV_MANUAL_AI_API_KEY_FILE, 3),
    ];
    apply_provider_env(config, &tripo, ProviderKind::Tripo);
    apply_provider_env(config, &manual_ai, ProviderKind::ManualAi);

    let limits = [
        (ENV_LIMITS_MAX_JSON_REQUEST_BYTES, 0usize),
        (ENV_LIMITS_MAX_PDF_BYTES, 1),
        (ENV_LIMITS_MAX_PDF_PAGES, 2),
        (ENV_LIMITS_MAX_PHOTO_BYTES, 3),
        (ENV_LIMITS_MAX_GLB_BYTES, 4),
        (ENV_LIMITS_MAX_ITEM_TOTAL_BYTES, 5),
    ];
    for (name, index) in limits {
        let Some(value) = env_u64(name)? else {
            continue;
        };
        let section = config.limits.get_or_insert_with(Default::default);
        match index {
            0 => section.max_json_request_bytes = Some(value),
            1 => section.max_pdf_bytes = Some(value),
            2 => section.max_pdf_pages = Some(value as u32),
            3 => section.max_photo_bytes = Some(value),
            4 => section.max_glb_bytes = Some(value),
            5 => section.max_item_total_bytes = Some(value),
            _ => unreachable!("limits 映射表越界"),
        }
    }

    if let Some(value) = env_i64(ENV_SESSION_TTL_HOURS)? {
        config
            .session
            .get_or_insert_with(Default::default)
            .ttl_hours = Some(value);
    }
    if let Some(value) = env_u64(ENV_SESSION_LOGIN_RATE_LIMIT_PER_MINUTE)? {
        config
            .session
            .get_or_insert_with(Default::default)
            .login_rate_limit_per_minute = Some(value as u32);
    }
    if let Some(value) = env_u64(ENV_SESSION_LOGIN_RATE_LIMIT_WINDOW_SECONDS)? {
        config
            .session
            .get_or_insert_with(Default::default)
            .login_rate_limit_window_seconds = Some(value);
    }
    if let Some(value) = env_nonempty(ENV_SESSION_COOKIE_SECURE) {
        config
            .session
            .get_or_insert_with(Default::default)
            .cookie_secure = Some(value);
    }

    let concurrency = [
        (ENV_CONCURRENCY_REMOTE_GENERATION, 0usize),
        (ENV_CONCURRENCY_MANUAL_AI_BATCHES, 1),
    ];
    for (name, index) in concurrency {
        let Some(value) = env_u64(name)? else {
            continue;
        };
        let section = config.concurrency.get_or_insert_with(Default::default);
        match index {
            0 => section.remote_generation = Some(value as u32),
            1 => section.manual_ai_batches = Some(value as u32),
            _ => unreachable!("concurrency 映射表越界"),
        }
    }

    if let Some(value) = env_u64(ENV_JOBS_LEASE_SECONDS)? {
        config
            .jobs
            .get_or_insert_with(Default::default)
            .lease_seconds = Some(value);
    }
    if let Some(value) = env_u64(ENV_JOBS_RENEW_SECONDS)? {
        config
            .jobs
            .get_or_insert_with(Default::default)
            .renew_seconds = Some(value);
    }

    if let Some(value) = env_nonempty(ENV_DOWNLOAD_ALLOWED_HOSTS) {
        config
            .download
            .get_or_insert_with(Default::default)
            .allowed_hosts = Some(
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect(),
        );
    }
    if let Some(value) = env_nonempty(ENV_DOWNLOAD_ALLOW_LOCAL_FIXTURE) {
        let parsed = match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            other => {
                return Err(CliError::config(format!(
                    "环境变量 {ENV_DOWNLOAD_ALLOW_LOCAL_FIXTURE} 只能是 true/false：{other}"
                )));
            }
        };
        config
            .download
            .get_or_insert_with(Default::default)
            .allow_local_fixture = Some(parsed);
    }
    Ok(())
}

enum ProviderKind {
    Tripo,
    ManualAi,
}

fn apply_provider_env(config: &mut FileConfig, names: &[(&str, usize)], kind: ProviderKind) {
    if names.iter().all(|(name, _)| env_nonempty(name).is_none()) {
        return;
    }
    let providers = config.providers.get_or_insert_with(Default::default);
    let section = match kind {
        ProviderKind::Tripo => providers.tripo.get_or_insert_with(Default::default),
        ProviderKind::ManualAi => providers.manual_ai.get_or_insert_with(Default::default),
    };
    for (name, index) in names {
        let Some(value) = env_nonempty(name) else {
            continue;
        };
        match index {
            0 => section.base_url = Some(value),
            1 => section.model = Some(value),
            2 => section.api_key_env = Some(value),
            3 => section.api_key_file = Some(value),
            _ => unreachable!("provider 映射表越界"),
        }
    }
}

fn env_u64(name: &str) -> Result<Option<u64>, CliError> {
    match env_nonempty(name) {
        None => Ok(None),
        Some(value) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|_| CliError::config(format!("环境变量 {name} 不是有效整数：{value}"))),
    }
}

fn env_i64(name: &str) -> Result<Option<i64>, CliError> {
    match env_nonempty(name) {
        None => Ok(None),
        Some(value) => value
            .parse::<i64>()
            .map(Some)
            .map_err(|_| CliError::config(format!("环境变量 {name} 不是有效整数：{value}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_and_full_config() {
        let minimal = parse("listen = \"127.0.0.1:9000\"\n", "test").unwrap();
        assert_eq!(minimal.listen.as_deref(), Some("127.0.0.1:9000"));

        let full = parse(
            r#"
data_dir = "./var/data"
listen = "127.0.0.1:8080"
public_origin = "http://127.0.0.1:8080"
price_catalog_path = "./prices.toml"
trusted_proxy_cidrs = ["10.0.0.0/8"]

[tls]
cert_file = "/etc/em/cert.pem"
key_file = "/etc/em/key.pem"

[providers.tripo]
base_url = "https://openapi.tripo3d.ai/v3"
model = "v3.1-20260211"
api_key_env = "TRIPO_API_KEY"

[providers.manual_ai]
model = "gpt-example"
api_key_env = "MANUAL_AI_API_KEY"

[limits]
max_pdf_bytes = 1048576
max_pdf_pages = 10

[concurrency]
remote_generation = 1
"#,
            "test",
        )
        .unwrap();
        assert_eq!(full.data_dir.as_deref(), Some("./var/data"));
        assert_eq!(full.trusted_proxy_cidrs.as_ref().unwrap().len(), 1);
        let tripo = full.providers.as_ref().unwrap().tripo.as_ref().unwrap();
        assert_eq!(tripo.api_key_env.as_deref(), Some("TRIPO_API_KEY"));
        assert_eq!(full.limits.as_ref().unwrap().max_pdf_pages, Some(10));
        assert_eq!(
            full.concurrency.as_ref().unwrap().remote_generation,
            Some(1)
        );
    }

    #[test]
    fn unknown_keys_are_errors_at_every_level() {
        let top = parse("unknown_key = 1\n", "test").unwrap_err();
        assert_eq!(top.exit_code, super::super::ExitCode::Config);
        assert!(top.message.contains("unknown_key"), "{}", top.message);

        let nested = parse("[providers.tripo]\nbogus = true\n", "test").unwrap_err();
        assert!(nested.message.contains("bogus"), "{}", nested.message);

        let deep = parse("[limits]\nmax_upload = 1\n", "test").unwrap_err();
        assert!(deep.message.contains("max_upload"), "{}", deep.message);
    }

    #[test]
    fn malformed_toml_is_reported_with_source() {
        let error = parse("listen = ", "config.toml").unwrap_err();
        assert!(error.message.contains("config.toml"), "{}", error.message);
    }

    #[test]
    fn example_config_file_matches_schema() {
        // 仓库根 config.example.toml 必须与 schema 一致（防止示例漂移为“未知键”）。
        let text = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config.example.toml"
        ));
        let config = parse(text, "config.example.toml").expect("示例配置必须是合法配置");
        // 示例以“可复制后直接运行”为准：给出 data_dir 与两个 provider 段，
        // TLS 等仅在有明确前提前才启用（见示例内注释）。
        assert!(config.data_dir.is_some(), "示例需给出 data_dir");
        assert!(config.providers.is_some(), "示例需给出 providers 段");
    }
}
