//! 供应商模型下载：SSRF 防护 + 流式落盘（T13 / REQ-028；architecture.md §7）。
//!
//! 与 T12 的 API 客户端**完全分离**：
//! - 本模块的 HTTP client **不带任何默认头**（不携带 API bearer token）：模型 CDN 只用来取字节；
//! - 只允许 **HTTPS**（测试构建 + 显式测试配置时例外，见下）；
//! - **关闭自动重定向**，逐跳重新做完整校验（scheme/允许域/解析/地址），最多 [`DownloadPolicy::max_redirects`] 跳；
//! - 允许域名单**精确匹配**（小写，不含端口与通配）；空名单 = 拒绝一切下载（不猜测 CDN 域名）；
//! - 每个请求把连接 **pin 到已校验的 IP**（`reqwest::ClientBuilder::resolve_to_addrs`），
//!   同时保留 URL 中的原始 hostname（Host 头与 TLS SNI/证书校验仍按原域名），
//!   避免"校验后再解析一次"造成 DNS 重绑定；
//! - DNS 返回的**所有**地址都必须通过校验（混合私网/公网答案 → 整次拒绝）；
//! - 拒绝 私网/回环/链路本地/未指定/组播/文档与保留网段；
//! - 显式 `retry(reqwest::retry::never())`：关闭 reqwest 0.13 的默认重试层
//!   （QA 回合 14 的 P3 建议：未来启用 http2 时默认策略会重发请求，改变下载/付费安全边界；
//!   本模块的下载重试语义由 T10 执行器按"可安全重试"分类，不由 HTTP 客户端自行重发）；
//! - 大小上限（`limits.max_glb_bytes`）与连接/整体超时；响应体**流式**落 tmp
//!   （边下边计数与 sha256，不整文件入内存）；
//! - 落盘顺序沿用 T06：tmp → fsync → 原子 rename 到内容寻址位置；失败丢弃自己的 tmp，
//!   不留半提交产物。
//!
//! **测试专用放行**（本机 fixture 的明文 http + 回环地址）要求两个条件同时成立：
//! 配置 `download.allow_local_fixture = true`（显式测试配置）**且**本 crate 处于
//! **测试构建**（feature `job-failpoints`，唯一由 [dev-dependencies] 自引用开启的
//! feature）。生产构建（`cargo build` / `cargo xtask dist`）即使配置了该键也不会
//! 放行本机地址（[`effective_local_fixture`] 的编译期开关为 false）：私网/链路本地
//! 等地址无论如何配置都一律拒绝，只有回环地址在满足上述两个条件时放行。

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use manual_core::timestamps::Timestamp;
use reqwest::header::{LOCATION, RETRY_AFTER};
use reqwest::{Url, redirect};

use crate::assets::blob_store::{self, SpaceProbe, StagedWriter};
use crate::assets::error::AssetError;
use crate::config::Settings;

/// 本 crate 是否为测试构建（唯一测试构建 feature；由 `[dev-dependencies]` 自引用开启）。
///
/// 生产构建（`cargo build`、`cargo xtask dist`）为 `false`：`allow_local_fixture`
/// 在生产构建中是**惰性配置**（不生效）。
pub const TEST_BUILD: bool = cfg!(feature = "job-failpoints");

/// 测试放行的判定（纯函数：两个条件都必须成立；便于单测覆盖四种组合）。
pub const fn effective_local_fixture(configured: bool, test_build: bool) -> bool {
    configured && test_build
}

/// 下载策略（配置 → 政策；默认拒绝一切下载）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadPolicy {
    /// 允许域（精确匹配、小写、不含端口/通配）；空 = 拒绝一切下载。
    pub allowed_hosts: Vec<String>,
    /// 仅测试配置：允许明文 http + 回环地址（本机 fixture）。见模块文档的两道门。
    pub allow_local_fixture: bool,
    /// 单文件字节上限（与 `limits.max_glb_bytes` 一致）。
    pub max_bytes: u64,
    /// 允许的重定向跳数上限。
    pub max_redirects: u32,
    /// TCP/TLS 连接超时。
    pub connect_timeout: Duration,
    /// 单次请求整体超时（含响应体读取；150 MiB 慢速下载需要更长）。
    pub request_timeout: Duration,
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        Self {
            allowed_hosts: Vec::new(),
            allow_local_fixture: false,
            max_bytes: 150 * 1_048_576,
            max_redirects: 5,
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(600),
        }
    }
}

impl DownloadPolicy {
    /// 从服务配置构造（允许域来自 `[download]`，大小上限来自 `limits.max_glb_bytes`）。
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            allowed_hosts: settings.download.allowed_hosts.clone(),
            allow_local_fixture: settings.download.allow_local_fixture,
            max_bytes: settings.limits.max_glb_bytes,
            ..Self::default()
        }
    }

    /// 本进程是否放行"本机 fixture"（配置开关 + 测试构建两道门）。
    pub fn local_fixture_allowed(&self) -> bool {
        effective_local_fixture(self.allow_local_fixture, TEST_BUILD)
    }
}

/// 下载错误（`code()` 为稳定标识；分类决定上层能否安全重试）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    /// URL 不可用（缺少主机名、含账号密码、无法解析）。
    InvalidUrl { detail: String },
    /// 主机不在允许域名单内（或名单为空 = 未配置）。
    HostNotAllowed { host: String },
    /// 生产只允许 HTTPS（明文 http 需要测试构建 + 显式测试配置）。
    InsecureScheme { scheme: String, host: String },
    /// 地址被拒绝（私网/回环/链路本地/保留网段等）。
    ForbiddenAddress {
        host: String,
        ip: IpAddr,
        reason: &'static str,
    },
    /// DNS 解析失败。
    ResolutionFailed { host: String, detail: String },
    /// 重定向超过上限。
    TooManyRedirects { limit: u32 },
    /// 3xx 响应缺少可用的 Location（或 Location 非法）。
    BadRedirect { detail: String },
    /// 链接过期（HTTP 401/403/404/410）：**重新查询已知任务取新链接**，不重新购买。
    LinkExpired { status: u16 },
    /// 429 限速（可安全重试；尊重 Retry-After）。
    RateLimited { retry_after_seconds: Option<u64> },
    /// 5xx（可安全重试）。
    ServerError { status: u16 },
    /// 其它 HTTP 状态（明确失败）。
    HttpStatus { status: u16 },
    /// 响应体超过大小上限。
    TooLarge { limit: u64, declared: Option<u64> },
    /// 磁盘空间不足（不半提交；提示清理磁盘）。
    InsufficientStorage { required: u64, available: u64 },
    /// 传输错误（连接/超时/读体中断）：下载可安全重试（不是付费操作）。
    Transport { detail: String },
    /// 本地 IO（写 tmp/rename 失败）。
    Io { detail: String },
}

impl DownloadError {
    /// 稳定错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUrl { .. } => "download_invalid_url",
            Self::HostNotAllowed { .. } => "download_host_not_allowed",
            Self::InsecureScheme { .. } => "download_insecure_scheme",
            Self::ForbiddenAddress { .. } => "download_forbidden_address",
            Self::ResolutionFailed { .. } => "download_dns",
            Self::TooManyRedirects { .. } => "download_too_many_redirects",
            Self::BadRedirect { .. } => "download_bad_redirect",
            Self::LinkExpired { .. } => "download_link_expired",
            Self::RateLimited { .. } => "download_rate_limited",
            Self::ServerError { .. } => "download_server_error",
            Self::HttpStatus { .. } => "download_http_status",
            Self::TooLarge { .. } => "download_too_large",
            Self::InsufficientStorage { .. } => "download_insufficient_storage",
            Self::Transport { .. } => "download_transport",
            Self::Io { .. } => "download_io",
        }
    }

    /// 是否可安全重试：网络类错误可以（下载不产生供应商费用）；链接过期由上层
    /// **重新查询**后重试；地址/域/协议类错误是配置问题，重试无意义。
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::ServerError { .. } | Self::Transport { .. }
        )
    }

    /// 面向用户/阶段的说明（不含完整签名 URL）。
    pub fn message(&self) -> String {
        match self {
            Self::InvalidUrl { detail } => format!("模型下载地址不合法：{detail}"),
            Self::HostNotAllowed { host } => format!(
                "模型下载域名（{host}）不在允许域名单内（download.allowed_hosts）：拒绝下载\
                 （不猜测供应商 CDN 域名，请按部署文档配置后重试）"
            ),
            Self::InsecureScheme { scheme, host } => {
                format!("模型下载必须使用 HTTPS（实际 {scheme}://{host}）：拒绝下载")
            }
            Self::ForbiddenAddress { host, ip, reason } => format!(
                "模型下载目标（{host} → {ip}）属于{reason}：拒绝连接（SSRF 防护；\
                 私网/回环/链路本地地址一律不访问）"
            ),
            Self::ResolutionFailed { host, detail } => {
                format!("模型下载域名解析失败（{host}）：{detail}")
            }
            Self::TooManyRedirects { limit } => {
                format!("模型下载重定向超过 {limit} 跳：停止跟随（每一跳都必须通过域名与地址校验）")
            }
            Self::BadRedirect { detail } => format!("模型下载重定向不合法：{detail}"),
            Self::LinkExpired { status } => format!(
                "模型下载链接已过期（HTTP {status}）：将重新查询已存在的远端任务取新链接\
                 （不会重新购买）"
            ),
            Self::RateLimited {
                retry_after_seconds,
            } => match retry_after_seconds {
                Some(seconds) => {
                    format!("模型下载被限速（429，Retry-After={seconds}s）：将退避重试")
                }
                None => "模型下载被限速（429）：将退避重试".to_owned(),
            },
            Self::ServerError { status } => {
                format!("模型 CDN 返回服务器错误（HTTP {status}）：下载可安全重试（不产生费用）")
            }
            Self::HttpStatus { status } => {
                format!("模型下载失败（HTTP {status}）：请核对模型链接与允许域配置")
            }
            Self::TooLarge { limit, declared } => match declared {
                Some(size) => {
                    format!("模型文件超过上限：{size} 字节 > {limit} 字节（未保存任何资产）")
                }
                None => format!("模型文件超过上限（{limit} 字节）：已停止读取（未保存任何资产）"),
            },
            Self::InsufficientStorage {
                required,
                available,
            } => format!(
                "磁盘可用空间不足：模型下载至少需要 {required} 字节，当前可用 {available} 字节；\
                 请清理磁盘后重试（未保存任何资产）"
            ),
            Self::Transport { detail } => {
                format!("模型下载传输失败（下载可安全重试；不产生供应商费用）：{detail}")
            }
            Self::Io { detail } => format!("模型下载落盘失败：{detail}"),
        }
    }

    /// 日志一行（与用户消息相同；不含密钥与查询串）。
    pub fn log_summary(&self) -> String {
        format!("{}：{}", self.code(), self.message())
    }
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.log_summary())
    }
}

impl std::error::Error for DownloadError {}

/// DNS 解析抽象（生产 = 系统解析器；测试可注入静态映射模拟 DNS 重绑定）。
pub type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, DownloadError>> + Send + 'a>>;

/// 主机 → 地址列表。**所有**返回的地址都必须通过地址校验才会发起连接。
pub trait HostResolver: Send + Sync + 'static {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a>;
}

/// 生产解析器（系统 DNS；只在通过允许域校验之后调用）。
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemHostResolver;

impl HostResolver for SystemHostResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host, port))
                .await
                .map_err(|error| DownloadError::ResolutionFailed {
                    host: host.to_owned(),
                    detail: error.to_string(),
                })?;
            Ok(addrs.map(|addr| addr.ip()).collect())
        })
    }
}

/// 一次成功下载的结果（文件已在内容寻址位置）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedModel {
    pub sha256: String,
    pub size: u64,
    pub path: PathBuf,
}

/// 已校验的下载目标（host 用于 Host 头/SNI，addr 用于 pin 连接）。
#[derive(Debug, Clone)]
struct ValidatedTarget {
    url: Url,
    host: String,
    addr: IpAddr,
}

/// 模型下载器（无 API 凭据；每个请求 pin 到已校验 IP）。
pub struct ModelDownloader {
    data_dir: PathBuf,
    policy: DownloadPolicy,
    resolver: Arc<dyn HostResolver>,
    space: SpaceProbe,
}

impl ModelDownloader {
    /// 生产构造：系统 DNS + `statvfs` 空间探测。
    pub fn new(data_dir: impl Into<PathBuf>, policy: DownloadPolicy) -> Self {
        Self {
            data_dir: data_dir.into(),
            policy,
            resolver: Arc::new(SystemHostResolver),
            space: SpaceProbe::Statvfs,
        }
    }

    /// 从服务配置构造（`[download]` + `limits.max_glb_bytes`）。
    pub fn from_settings(settings: &Settings) -> Self {
        Self::new(
            settings.data_dir.clone(),
            DownloadPolicy::from_settings(settings),
        )
    }

    /// 测试注入：自定义解析器（DNS 重绑定模拟）。
    pub fn with_resolver(mut self, resolver: Arc<dyn HostResolver>) -> Self {
        self.resolver = resolver;
        self
    }

    /// 测试注入：空间探测（磁盘满场景）。
    pub fn with_space_probe(mut self, space: SpaceProbe) -> Self {
        self.space = space;
        self
    }

    pub fn policy(&self) -> &DownloadPolicy {
        &self.policy
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 下载一个模型 URL 到内容寻址位置（`blobs/<sha 前 2 位>/<sha256>`）。
    ///
    /// 只接受 HTTPS + 允许域 + 通过地址校验的目标；逐跳验证重定向。
    pub async fn download(&self, url: &str) -> Result<DownloadedModel, DownloadError> {
        let mut current = Url::parse(url).map_err(|error| DownloadError::InvalidUrl {
            detail: error.to_string(),
        })?;
        let mut hops = 0_u32;
        loop {
            let target = self.validate_target(&current).await?;
            let client = self.build_client(&target)?;
            let response = client
                .get(target.url.clone())
                .send()
                .await
                .map_err(classify_transport)?;
            let status = response.status().as_u16();
            match status {
                200..=299 => return self.stream_to_blob(response).await,
                status_code @ (301 | 302 | 303 | 307 | 308) => {
                    if hops >= self.policy.max_redirects {
                        return Err(DownloadError::TooManyRedirects {
                            limit: self.policy.max_redirects,
                        });
                    }
                    let location = response
                        .headers()
                        .get(LOCATION)
                        .and_then(|value| value.to_str().ok())
                        .ok_or_else(|| DownloadError::BadRedirect {
                            detail: format!("HTTP {status_code} 响应缺少 Location 头"),
                        })?;
                    let next =
                        current
                            .join(location)
                            .map_err(|error| DownloadError::BadRedirect {
                                detail: format!("Location 无法解析为绝对地址：{error}"),
                            })?;
                    hops += 1;
                    tracing::info!(
                        event = "model_download_redirect",
                        hop = hops,
                        status = status_code,
                        host = %next.host_str().unwrap_or("?"),
                        "模型下载跟随重定向：下一跳将重新做完整校验"
                    );
                    current = next;
                }
                401 | 403 | 404 | 410 => return Err(DownloadError::LinkExpired { status }),
                429 => {
                    let retry_after_seconds = response
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.trim().parse::<u64>().ok());
                    return Err(DownloadError::RateLimited {
                        retry_after_seconds,
                    });
                }
                other if other >= 500 => return Err(DownloadError::ServerError { status: other }),
                other => return Err(DownloadError::HttpStatus { status: other }),
            }
        }
    }

    /// 校验一个 URL：scheme、允许域、DNS 解析、地址范围（每跳都重新执行）。
    async fn validate_target(&self, url: &Url) -> Result<ValidatedTarget, DownloadError> {
        if url.host_str().is_none() {
            return Err(DownloadError::InvalidUrl {
                detail: "缺少主机名".to_owned(),
            });
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(DownloadError::InvalidUrl {
                detail: "URL 不允许包含账号密码（凭据）".to_owned(),
            });
        }
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let scheme = url.scheme().to_owned();
        let local_fixture = self.policy.local_fixture_allowed();
        match scheme.as_str() {
            "https" => {}
            "http" if local_fixture => {}
            _ => return Err(DownloadError::InsecureScheme { scheme, host }),
        }
        if !self
            .policy
            .allowed_hosts
            .iter()
            .any(|allowed| allowed == &host)
        {
            return Err(DownloadError::HostNotAllowed { host });
        }
        let port = url
            .port_or_known_default()
            .unwrap_or(if scheme == "https" { 443 } else { 80 });

        // IP 字面量：不需要解析，直接做地址校验。
        if let Ok(ip) = host.parse::<IpAddr>() {
            check_address(&host, ip, local_fixture)?;
            return Ok(ValidatedTarget {
                url: url.clone(),
                host,
                addr: ip,
            });
        }

        let addresses = self.resolver.resolve(&host, port).await?;
        if addresses.is_empty() {
            return Err(DownloadError::ResolutionFailed {
                host,
                detail: "解析结果为空".to_owned(),
            });
        }
        // 任一地址被拒 → 整次拒绝（避免"混合公网/私网答案"绕过防护）。
        for address in &addresses {
            check_address(&host, *address, local_fixture)?;
        }
        Ok(ValidatedTarget {
            url: url.clone(),
            host,
            addr: addresses[0],
        })
    }

    /// 构造本跳的 client：pin 到已校验 IP、不跟随重定向、显式关闭默认重试层。
    fn build_client(&self, target: &ValidatedTarget) -> Result<reqwest::Client, DownloadError> {
        reqwest::Client::builder()
            .connect_timeout(self.policy.connect_timeout)
            .timeout(self.policy.request_timeout)
            // 不自动跟随重定向：由本模块逐跳校验（避免把请求带到未校验的地址）。
            .redirect(redirect::Policy::none())
            // 显式关闭 reqwest 0.13 的默认重试层（QA 回合 14 P3）：
            // 默认策略只在 http2/http3 下可能生效，但"启用即改变安全边界"——这里固定为不重发。
            .retry(reqwest::retry::never())
            // 连接 pin 到已校验 IP；URL 的 hostname（Host 头与 TLS SNI）保持不变，
            // 因此不存在"校验后再解析一次"的 DNS 重绑定窗口。
            .resolve_to_addrs(&target.host, &[SocketAddr::new(target.addr, 0)])
            .build()
            .map_err(|error| DownloadError::Io {
                detail: format!("构造下载客户端失败：{error}"),
            })
    }

    /// 流式落盘：边下边计数与 sha256（不整文件入内存）→ fsync → 原子 rename。
    async fn stream_to_blob(
        &self,
        mut response: reqwest::Response,
    ) -> Result<DownloadedModel, DownloadError> {
        let declared = response.content_length();
        if let Some(length) = declared
            && length > self.policy.max_bytes
        {
            return Err(DownloadError::TooLarge {
                limit: self.policy.max_bytes,
                declared: Some(length),
            });
        }
        // 解析前预检：声明长度已知时先查剩余空间（不足 → 明确错误，不半提交）。
        blob_store::ensure_space(&self.space, &self.data_dir, declared.unwrap_or(0))
            .map_err(|error| map_asset_error(error, self.policy.max_bytes))?;

        let tmp_dir = blob_store::tmp_dir(&self.data_dir);
        let mut writer = StagedWriter::create(&tmp_dir, self.policy.max_bytes, "model_download")
            .await
            .map_err(|error| map_asset_error(error, self.policy.max_bytes))?;
        let started = Instant::now();
        let mut written: u64 = 0;
        loop {
            let chunk = response.chunk().await.map_err(classify_transport)?;
            let Some(chunk) = chunk else { break };
            writer
                .write(&chunk)
                .await
                .map_err(|error| map_asset_error(error, self.policy.max_bytes))?;
            written += chunk.len() as u64;
        }
        let staged = writer
            .finish()
            .await
            .map_err(|error| map_asset_error(error, self.policy.max_bytes))?;
        if staged.size != written {
            return Err(DownloadError::Io {
                detail: "下载计数与暂存文件大小不一致（未保存任何资产）".to_owned(),
            });
        }
        // 落盘前复检（chunked / 未知长度的下载在这里第一次判定空间）。
        if let Err(error) = blob_store::ensure_space(&self.space, &self.data_dir, staged.size) {
            blob_store::discard_staged(staged).await;
            return Err(map_asset_error(error, self.policy.max_bytes));
        }
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let path = match blob_store::promote(&staged, &self.data_dir).await {
            Ok(path) => path,
            Err(error) => {
                blob_store::discard_staged(staged).await;
                return Err(map_asset_error(error, self.policy.max_bytes));
            }
        };
        tracing::info!(
            event = "model_download_completed",
            sha256 = %staged.sha256,
            sizeBytes = staged.size,
            elapsedMs = elapsed_ms,
            savedAt = %Timestamp::now().to_rfc3339(),
            "模型已下载并原子落盘（内容寻址；临时供应商 URL 不落地为永久地址）"
        );
        Ok(DownloadedModel {
            sha256: staged.sha256,
            size: staged.size,
            path,
        })
    }
}

/// 传输层错误分类（连接/超时/读体失败 → 可安全重试）。
///
/// **reqwest 错误文本的处理（BUG-009）**：reqwest 0.13.5 的 `Display` 在请求错误后
/// 追加 ` for url (<完整 URL，含查询串>)`（`reqwest-0.13.5/src/error.rs:299-302`），
/// 直接拼接会把供应商签名 URL 写进 `job_stages.last_error`/日志。这里：
///
/// 1. 先用 [`reqwest::Error::without_url`] 去掉 URL（保留错误类别的原文，例如
///    `error sending request` / `operation timed out`）；
/// 2. 再对拼好的 detail 走**统一脱敏入口** [`crate::redaction::redact_text_urls`]
///    兜底（即使未来 reqwest 在消息正文里再嵌 URL 也不会出网/落库）。
///
/// 诊断能力不受影响：错误类别、`code()`（`download_transport`）、目标 host（日志字段与
/// 摘要标签）与"下载可安全重试、链接过期按 task_id 重查"的结论都在。
fn classify_transport(error: reqwest::Error) -> DownloadError {
    let kind = if error.is_timeout() {
        "超时"
    } else if error.is_connect() {
        "连接失败"
    } else if error.is_body() || error.is_decode() {
        "响应体读取中断"
    } else {
        "传输错误"
    };
    let detail = crate::redaction::redact_text_urls(&format!("{kind}：{}", error.without_url()));
    DownloadError::Transport { detail }
}

/// `AssetError`（落盘层）→ 下载错误。
fn map_asset_error(error: AssetError, max_bytes: u64) -> DownloadError {
    match error {
        AssetError::PayloadTooLarge { .. } => DownloadError::TooLarge {
            limit: max_bytes,
            declared: None,
        },
        AssetError::InsufficientStorage {
            required,
            available,
        } => DownloadError::InsufficientStorage {
            required,
            available,
        },
        AssetError::Io { detail } => DownloadError::Io { detail },
        other => DownloadError::Io {
            detail: other.log_summary(),
        },
    }
}

/// 地址校验：返回 `Err` 表示禁止连接（`local_fixture` 只放行**回环**地址，
/// 且只在测试构建 + 显式测试配置时生效）。
fn check_address(host: &str, ip: IpAddr, local_fixture: bool) -> Result<(), DownloadError> {
    match classify_address(ip) {
        None => Ok(()),
        Some("回环地址") if local_fixture => Ok(()),
        Some(reason) => Err(DownloadError::ForbiddenAddress {
            host: host.to_owned(),
            ip,
            reason,
        }),
    }
}

/// 非公网地址分类（`None` = 允许的全球单播地址）。
fn classify_address(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => classify_v4(&v4),
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return Some("回环地址");
            }
            if v6.is_unspecified() {
                return Some("未指定地址");
            }
            if v6.is_unique_local() {
                return Some("唯一本地地址（fc00::/7）");
            }
            if v6.is_unicast_link_local() {
                return Some("链路本地地址（fe80::/10）");
            }
            if v6.is_multicast() {
                return Some("组播地址");
            }
            let segments = v6.segments();
            if segments[0] == 0x2001 && segments[1] == 0x0db8 {
                return Some("文档保留地址（2001:db8::/32）");
            }
            if let Some(v4) = v6.to_ipv4_mapped() {
                return classify_v4(&v4);
            }
            None
        }
    }
}

fn classify_v4(v4: &Ipv4Addr) -> Option<&'static str> {
    if v4.is_loopback() {
        return Some("回环地址");
    }
    if v4.is_private() {
        return Some("私网地址");
    }
    if v4.is_link_local() {
        return Some("链路本地地址（169.254.0.0/16）");
    }
    if v4.is_broadcast() {
        return Some("广播地址");
    }
    if v4.is_documentation() {
        return Some("文档保留地址");
    }
    if v4.is_multicast() {
        return Some("组播地址");
    }
    if v4.is_unspecified() {
        return Some("未指定地址");
    }
    let octets = v4.octets();
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return Some("运营商级 NAT 地址（100.64.0.0/10）");
    }
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
        return Some("IETF 协议保留地址（192.0.0.0/24）");
    }
    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
        return Some("基准测试保留地址（198.18.0.0/15）");
    }
    if octets[0] >= 240 {
        return Some("保留地址（240.0.0.0/4）");
    }
    None
}

/// 下载一个 URL（便捷入口；内部构造 [`ModelDownloader`]）。
pub async fn download_model(
    data_dir: &Path,
    policy: DownloadPolicy,
    url: &str,
) -> Result<DownloadedModel, DownloadError> {
    ModelDownloader::new(data_dir.to_path_buf(), policy)
        .download(url)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_fixture_requires_both_config_flag_and_test_build() {
        assert!(effective_local_fixture(true, true));
        assert!(
            !effective_local_fixture(true, false),
            "生产构建不得放行本机 fixture"
        );
        assert!(!effective_local_fixture(false, true));
        assert!(!effective_local_fixture(false, false));
        // 本文件在测试目标中编译：测试构建开关为真。
        assert_eq!(TEST_BUILD, cfg!(feature = "job-failpoints"));
    }

    #[test]
    fn private_and_reserved_addresses_are_classified() {
        for (ip, expect) in [
            ("127.0.0.1", "回环地址"),
            ("::1", "回环地址"),
            ("10.1.2.3", "私网地址"),
            ("172.16.5.5", "私网地址"),
            ("192.168.1.1", "私网地址"),
            ("169.254.10.10", "链路本地地址（169.254.0.0/16）"),
            ("0.0.0.0", "未指定地址"),
            ("255.255.255.255", "广播地址"),
            ("224.0.0.1", "组播地址"),
            ("192.0.2.10", "文档保留地址"),
            ("198.18.0.1", "基准测试保留地址（198.18.0.0/15）"),
            ("100.100.0.1", "运营商级 NAT 地址（100.64.0.0/10）"),
            ("240.0.0.1", "保留地址（240.0.0.0/4）"),
            ("fe80::1", "链路本地地址（fe80::/10）"),
            ("fd00::1", "唯一本地地址（fc00::/7）"),
            ("2001:db8::1", "文档保留地址（2001:db8::/32）"),
            ("::ffff:10.0.0.1", "私网地址"),
        ] {
            let ip: IpAddr = ip.parse().expect("测试地址");
            assert_eq!(classify_address(ip), Some(expect), "{ip}");
        }
        for allowed in ["8.8.8.8", "1.1.1.1", "2606:4700::1111", "93.184.216.34"] {
            let ip: IpAddr = allowed.parse().expect("测试地址");
            assert_eq!(classify_address(ip), None, "{ip} 应视为公网地址");
        }
    }

    #[test]
    fn loopback_is_allowed_only_with_the_test_fixture_gate() {
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(check_address("cdn.example", loopback, true).is_ok());
        let error = check_address("cdn.example", loopback, false).unwrap_err();
        assert_eq!(error.code(), "download_forbidden_address");

        // 私网地址即使放行本机 fixture 也一律拒绝。
        let private: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(check_address("cdn.example", private, true).is_err());
    }

    #[test]
    fn policy_defaults_reject_everything() {
        let policy = DownloadPolicy::default();
        assert!(policy.allowed_hosts.is_empty());
        assert!(!policy.allow_local_fixture);
        assert_eq!(policy.max_redirects, 5);
        assert_eq!(policy.max_bytes, 150 * 1_048_576);
    }

    #[test]
    fn error_codes_and_retry_classification() {
        assert!(
            DownloadError::Transport {
                detail: "x".to_owned()
            }
            .is_retryable()
        );
        assert!(DownloadError::ServerError { status: 503 }.is_retryable());
        assert!(
            !DownloadError::HostNotAllowed {
                host: "evil.test".to_owned()
            }
            .is_retryable()
        );
        assert_eq!(
            DownloadError::LinkExpired { status: 403 }.code(),
            "download_link_expired"
        );
        assert!(
            DownloadError::LinkExpired { status: 403 }
                .message()
                .contains("不会重新购买")
        );
    }
}
