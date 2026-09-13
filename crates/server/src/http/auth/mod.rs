//! 认证与会话（REQ-002；contracts.md §3；PRD §5.1）。
//!
//! 本模块负责：
//! - `POST /auth/login`：Origin 检查 + 登录限速 + Argon2id 校验；
//! - `GET /auth/session`：会话恢复（`Cache-Control: no-store`），返回派生 CSRF token；
//! - `POST /auth/logout`：撤销会话并清 cookie；
//! - [`auth_guard`] 中间件：**所有受保护 `/api/v1` 路由**的会话校验，以及所有
//!   修改请求（含未来的 multipart）的 **CSRF + Origin** 检查。
//!
//! 安全语义（实现依据，QA 按此复核）：
//! - 会话 cookie `em_session`：`HttpOnly` + `SameSite=Strict` + `Path=/`；`Secure`
//!   由 [`crate::config::Settings::cookie_secure`] 判定（显式配置优先，auto 时只看
//!   服务端已知事实：`public_origin` 的 scheme 或内置 TLS 配置；**不读取任何
//!   `X-Forwarded-*`**）；
//! - 明文会话 token 只出现在 `Set-Cookie`，不落库（库里只有 SHA-256 哈希）、不写日志；
//! - CSRF token 由会话 token 派生（见 [`tokens`]），前端经 `X-CSRF-Token` 回传；
//!   缺失或不匹配 → 403 `CSRF_REJECTED`；
//! - `Origin` 头存在时必须命中允许列表（`public_origin`，或未配置时与请求 `Host`
//!   同源且 scheme 与服务端判定一致），否则 403 `ORIGIN_REJECTED`；浏览器对修改请求
//!   总会带上 Origin，缺失时由强制的 CSRF token 兜底（并给非浏览器客户端留出路径）；
//! - 登录失败限速（[`limiter`]）默认 5 次/分钟 → 429 + `Retry-After`；不区分失败原因、
//!   **不记录请求体**；
//! - 会话绝对有效期默认 7 天（A-05），无滑动续期；过期/已撤销与不存在都返回同一个 401。

pub mod limiter;
pub mod password;
pub mod tokens;

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::Extension;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing};
use manual_core::timestamps::Timestamp;

use crate::config::{Cidr, Settings};
use crate::storage::repo::sessions;
use crate::storage::repo::sessions::NewSession;

use super::body::JsonBody;
use super::dto::{AdminSummary, LoginRequest, LoginResponse, SessionData, SessionResponse};
use super::error::{ApiError, RequestId};
use super::state::AppState;

/// 会话 cookie 名。不带 `Domain`（主机限定），`Path=/`。
pub const SESSION_COOKIE_NAME: &str = "em_session";
/// 修改请求必须携带的 CSRF 头。
pub const CSRF_HEADER: &str = "x-csrf-token";
/// 未识别来源 IP（无法获得对端地址，例如进程内测试）。
const UNKNOWN_IP: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

/// 已认证请求的会话上下文（由 [`auth_guard`] 插入请求扩展）。
#[derive(Clone)]
pub struct SessionContext {
    pub session_id: String,
    pub admin_id: String,
    pub csrf_hash: String,
    pub expires_at: Timestamp,
    /// 会话 token 明文（来自本次请求的 cookie）：仅用于派生 CSRF token 返回给前端。
    /// 私有字段 + 手工 `Debug`，避免进入任何日志或调试输出。
    session_token: String,
}

impl SessionContext {
    /// 前端应当持有的 CSRF token（与登录时返回的一致）。
    pub fn csrf_token(&self) -> String {
        tokens::csrf_token_for(&self.session_token)
    }

    /// 会话数据载荷（登录与 `GET /auth/session` 共用同一形状）。
    pub fn to_data(&self) -> SessionData {
        SessionData {
            admin: AdminSummary {
                id: self.admin_id.clone(),
            },
            csrf_token: self.csrf_token(),
            expires_at: self.expires_at,
        }
    }
}

impl fmt::Debug for SessionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 会话 token 明文不出现在 Debug 输出（可能被日志框架打印）。
        f.debug_struct("SessionContext")
            .field("sessionId", &self.session_id)
            .field("adminId", &self.admin_id)
            .finish_non_exhaustive()
    }
}

/// 对端地址提取器：`serve` 经 `into_make_service_with_connect_info` 提供；
/// 进程内测试（`oneshot`）没有它，值为 `None`。
pub struct PeerAddress(pub Option<SocketAddr>);

impl<S> axum::extract::FromRequestParts<S> for PeerAddress
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0),
        ))
    }
}

// ---------------------------------------------------------------------------
// 路由
// ---------------------------------------------------------------------------

/// 公开认证路由（不带会话中间件）。
pub fn public_routes() -> Router<AppState> {
    Router::new().route("/auth/login", routing::post(login))
}

/// 受会话保护的路由（`serve` 在路由组装时统一挂 [`auth_guard`]）。
pub fn protected_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/session", routing::get(session))
        .route("/auth/logout", routing::post(logout))
}

// ---------------------------------------------------------------------------
// 会话中间件
// ---------------------------------------------------------------------------

/// 受保护路由的会话守卫：cookie → 会话查找 → （修改请求）Origin + CSRF 校验。
///
/// 连接只在守卫内短暂持有（查完即释放），不跨 handler 持有事务/连接。
pub async fn auth_guard(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .cloned()
        .unwrap_or_else(|| RequestId(uuid::Uuid::now_v7().to_string()));

    let Some(session_token) = session_token_from_cookie(request.headers()) else {
        return ApiError::unauthorized().render(&request_id);
    };

    let session = {
        let mut connection = match state.database().pool().acquire().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::error!(error = %error, "获取数据库连接失败");
                return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
            }
        };
        match sessions::find_active(
            &mut connection,
            &tokens::session_token_hash(&session_token),
            Timestamp::now(),
        )
        .await
        {
            Ok(session) => session,
            Err(error) => return ApiError::from_storage(error).render(&request_id),
        }
    };

    let Some(session) = session else {
        return ApiError::unauthorized().render(&request_id);
    };

    if is_mutating(request.method()) {
        if let Err(error) = origin_check(state.settings(), request.headers()) {
            tracing::warn!(
                requestId = %request_id,
                path = %request.uri().path(),
                errorCode = error.code.as_str(),
                "修改请求的 Origin 校验失败"
            );
            return error.render(&request_id);
        }
        if let Err(error) = csrf_check(request.headers(), &session.csrf_hash) {
            tracing::warn!(
                requestId = %request_id,
                path = %request.uri().path(),
                errorCode = error.code.as_str(),
                "修改请求的 CSRF 校验失败"
            );
            return error.render(&request_id);
        }
    }

    request.extensions_mut().insert(SessionContext {
        session_id: session.id,
        admin_id: session.admin_id,
        csrf_hash: session.csrf_hash,
        expires_at: session.expires_at,
        session_token,
    });
    next.run(request).await
}

/// 修改请求的方法集合：CSRF 与 Origin 检查覆盖全部（**包括未来的 multipart POST**）。
fn is_mutating(method: &axum::http::Method) -> bool {
    matches!(
        *method,
        axum::http::Method::POST
            | axum::http::Method::PUT
            | axum::http::Method::PATCH
            | axum::http::Method::DELETE
    )
}

// ---------------------------------------------------------------------------
// handler：登录 / 会话 / 注销
// ---------------------------------------------------------------------------

/// `POST /api/v1/auth/login`。
///
/// 顺序：限速检查 → Origin 检查 → 读取管理员 → Argon2id 校验 → 创建会话。
/// 失败一律 401 同一文案；限速期间 429（`Retry-After`）；**请求体不写日志**。
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth",
    summary = "管理员登录",
    description = "校验密码并创建会话。成功时经 Set-Cookie 返回 HttpOnly + SameSite=Strict \
                   （HTTPS 判定下加 Secure）的会话 cookie，响应体返回 CSRF token。\
                   失败限速默认 5 次/分钟（429 + Retry-After）；错误不区分字段原因。",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "登录成功", body = LoginResponse),
        (status = 401, description = "凭据无效", body = super::dto::ApiErrorResponse),
        (status = 403, description = "Origin 不被允许", body = super::dto::ApiErrorResponse),
        (status = 422, description = "请求体不符合接口结构", body = super::dto::ApiErrorResponse),
        (status = 429, description = "登录失败次数超出限速窗口", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    request_id: RequestId,
    peer: PeerAddress,
    headers: HeaderMap,
    JsonBody(body): JsonBody<LoginRequest>,
) -> Response {
    let ip = resolve_client_ip(peer.0, &headers, &state.settings().trusted_proxy_cidrs);

    if let Err(retry_after) = state.login_limiter().check(ip) {
        tracing::warn!(
            requestId = %request_id,
            clientIp = %ip,
            retryAfterSeconds = retry_after,
            "登录被限速（窗口内失败次数达到上限）"
        );
        return ApiError::rate_limited(retry_after).render(&request_id);
    }

    if let Err(error) = origin_check(state.settings(), &headers) {
        return error.render(&request_id);
    }

    let admin = {
        let mut connection = match state.database().pool().acquire().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::error!(error = %error, "获取数据库连接失败");
                return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
            }
        };
        match crate::storage::repo::admins::get_single(&mut connection).await {
            Ok(admin) => admin,
            Err(error) => return ApiError::from_storage(error).render(&request_id),
        }
    };

    let Some(admin) = admin else {
        // 未初始化：与密码错误同一响应（不泄露系统状态），但仍消耗一次限速额度。
        state.login_limiter().record_failure(ip);
        tracing::warn!(requestId = %request_id, clientIp = %ip, "登录失败：管理员尚未初始化");
        return ApiError::login_failed().render(&request_id);
    };

    // Argon2id 校验是 CPU/内存密集操作：放到阻塞线程池，不占用异步运行时线程。
    let password = body.password;
    let stored_hash = admin.password_hash.clone();
    let verified =
        tokio::task::spawn_blocking(move || password::verify_password(&password, &stored_hash))
            .await;

    match verified {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) => {
            state.login_limiter().record_failure(ip);
            tracing::warn!(requestId = %request_id, clientIp = %ip, "登录失败：凭据无效");
            return ApiError::login_failed().render(&request_id);
        }
        Ok(Err(error)) => {
            tracing::error!(error = %error, "口令校验失败（存储的哈希不可用）");
            return ApiError::internal("服务器内部错误：凭据存储异常").render(&request_id);
        }
        Err(error) => {
            tracing::error!(error = %error, "口令校验任务异常");
            return ApiError::internal("服务器内部错误：请稍后重试").render(&request_id);
        }
    }

    state.login_limiter().record_success(ip);

    let session_token = match tokens::generate_session_token() {
        Ok(token) => token,
        Err(detail) => {
            tracing::error!(error = %detail, "生成会话 token 失败");
            return ApiError::internal("服务器内部错误：无法创建会话").render(&request_id);
        }
    };
    let csrf_token = tokens::csrf_token_for(&session_token);
    let ttl_seconds = state.settings().session.ttl_hours * 3600;
    let expires_at = match Timestamp::now().checked_add_millis(ttl_seconds * 1000) {
        Some(expires_at) => expires_at,
        None => return ApiError::internal("服务器内部错误：会话有效期溢出").render(&request_id),
    };

    let created = {
        let mut connection = match state.database().pool().acquire().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::error!(error = %error, "获取数据库连接失败");
                return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
            }
        };
        if let Err(error) = sessions::purge_expired(&mut connection, Timestamp::now()).await {
            tracing::warn!(error = %error, "清理过期会话失败（不影响本次登录）");
        }
        sessions::create(
            &mut connection,
            NewSession {
                admin_id: admin.id.clone(),
                session_token_hash: tokens::session_token_hash(&session_token),
                csrf_hash: tokens::csrf_hash(&csrf_token),
                expires_at,
            },
        )
        .await
    };

    let session = match created {
        Ok(session) => session,
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    };

    tracing::info!(
        requestId = %request_id,
        adminId = %admin.id,
        sessionId = %session.id,
        expiresAt = %session.expires_at,
        "登录成功（未记录请求体）"
    );

    let payload = SessionData {
        admin: AdminSummary {
            id: admin.id.clone(),
        },
        csrf_token,
        expires_at: session.expires_at,
    };
    json_no_store(StatusCode::OK, LoginResponse { data: payload }).with_session_cookie(
        &session_cookie(
            &session_token,
            ttl_seconds,
            state.settings().cookie_secure(),
        ),
    )
}

/// `GET /api/v1/auth/session` —— 刷新页面后的会话恢复；`Cache-Control: no-store`。
#[utoipa::path(
    get,
    path = "/api/v1/auth/session",
    tag = "auth",
    summary = "读取当前会话",
    description = "返回管理员摘要与 CSRF token；响应带 Cache-Control: no-store。未登录返回 401。",
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "当前会话", body = SessionResponse),
        (status = 401, description = "未登录或会话已失效", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn session(Extension(context): Extension<SessionContext>) -> Response {
    json_no_store(
        StatusCode::OK,
        SessionResponse {
            data: context.to_data(),
        },
    )
}

/// `POST /api/v1/auth/logout` —— 撤销当前会话并清 cookie（204）。
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    summary = "注销当前会话",
    description = "撤销会话并清除 cookie；旧 cookie 立即失效（再访问受保护路由返回 401）。\
                   属于修改请求：需携带 X-CSRF-Token，缺失/跨站 Origin 返回 403。",
    security(("sessionCookie" = [])),
    responses(
        (status = 204, description = "已注销"),
        (status = 401, description = "未登录或会话已失效", body = super::dto::ApiErrorResponse),
        (status = 403, description = "CSRF 校验失败", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn logout(
    State(state): State<AppState>,
    Extension(context): Extension<SessionContext>,
    request_id: RequestId,
) -> Response {
    let mut connection = match state.database().pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::error!(error = %error, "获取数据库连接失败");
            return ApiError::internal("服务器内部错误：数据库暂不可用").render(&request_id);
        }
    };
    match sessions::revoke(&mut connection, &context.session_id, Timestamp::now()).await {
        Ok(_) => {}
        Err(error) => return ApiError::from_storage(error).render(&request_id),
    }
    tracing::info!(
        requestId = %request_id,
        adminId = %context.admin_id,
        sessionId = %context.session_id,
        "会话已注销"
    );
    (
        StatusCode::NO_CONTENT,
        [(
            header::SET_COOKIE,
            cleared_session_cookie(state.settings().cookie_secure()),
        )],
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// cookie / 来源 / CSRF 工具
// ---------------------------------------------------------------------------

/// 会话 cookie 值（`Max-Age` 为绝对有效期；`Secure` 由调用方判定传入）。
pub fn session_cookie(token: &str, ttl_seconds: i64, secure: bool) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ttl_seconds}"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// 清除会话 cookie（登出/失效）。
pub fn cleared_session_cookie(secure: bool) -> String {
    let mut cookie =
        format!("{SESSION_COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// 从 `Cookie` 头读取会话 token（名值精确匹配；值来自我们自己的 hex 编码）。
pub fn session_token_from_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    let prefix = format!("{SESSION_COOKIE_NAME}=");
    for part in raw.split(';') {
        if let Some(value) = part.trim().strip_prefix(&prefix)
            && !value.is_empty()
        {
            return Some(value.to_owned());
        }
    }
    None
}

/// CSRF 校验：头缺失或不匹配都是 403（交给同一个错误，不给探测者差异信息）。
fn csrf_check(headers: &HeaderMap, stored_csrf_hash: &str) -> Result<(), ApiError> {
    let presented = headers
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(presented) = presented else {
        return Err(ApiError::csrf_rejected());
    };
    if tokens::constant_time_eq(&tokens::csrf_hash(presented), stored_csrf_hash) {
        Ok(())
    } else {
        Err(ApiError::csrf_rejected())
    }
}

/// Origin 校验。未携带 `Origin` 视为通过（CSRF token 仍强制；浏览器对修改请求总会携带）。
fn origin_check(settings: &Settings, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    let origin = match origin.to_str() {
        Ok(origin) => origin,
        Err(_) => return Err(ApiError::origin_rejected("<非 ASCII Origin>")),
    };
    if origin_allowed(settings, headers, origin) {
        Ok(())
    } else {
        Err(ApiError::origin_rejected(origin))
    }
}

fn origin_allowed(settings: &Settings, headers: &HeaderMap, origin: &str) -> bool {
    if let Some(public_origin) = settings.public_origin.as_deref() {
        // 显式声明了公开来源：只接受它（与 scheme/host/端口精确一致）。
        return normalize_origin(origin).as_deref() == Some(public_origin);
    }
    // 未声明：只接受与请求 Host 同源的 Origin；scheme 由服务端已知事实判定
    // （不读取 X-Forwarded-Proto）。
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let expected_scheme = if settings.cookie_secure() {
        "https"
    } else {
        "http"
    };
    same_origin(origin, host, expected_scheme)
}

/// 把 Origin 规范化为 `scheme://host[:port]`（小写、去默认端口）；不是合法 Origin 时为 None。
fn normalize_origin(origin: &str) -> Option<String> {
    let origin = origin.trim();
    let (scheme, rest) = origin.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    if rest.is_empty() || rest.contains('/') || rest.contains('?') || rest.contains('#') {
        return None;
    }
    Some(format!(
        "{scheme}://{}",
        strip_default_port(&rest.to_ascii_lowercase(), &scheme)
    ))
}

/// 判断 `Origin` 与请求的 `Host` 是否同源（scheme 必须等于服务端判定的 scheme）。
fn same_origin(origin: &str, host_header: &str, expected_scheme: &str) -> bool {
    let Some(origin) = normalize_origin(origin) else {
        return false;
    };
    let Some((scheme, origin_host)) = origin.split_once("://") else {
        return false;
    };
    if scheme != expected_scheme {
        return false;
    }
    let host = strip_default_port(&host_header.trim().to_ascii_lowercase(), expected_scheme);
    if host.is_empty() || host.contains('/') {
        return false;
    }
    origin_host == host
}

fn strip_default_port(host: &str, scheme: &str) -> String {
    let default_suffix = if scheme == "https" { ":443" } else { ":80" };
    host.strip_suffix(default_suffix).unwrap_or(host).to_owned()
}

/// 解析限速用的来源 IP。
///
/// - 无对端地址（进程内测试）：固定哨兵地址；
/// - 对端**不在** `trusted_proxy_cidrs` 内：一律使用 socket 对端地址，
///   `X-Forwarded-For` 完全不参与（未受信的转发头不能改变行为）；
/// - 对端在受信代内：从 `X-Forwarded-For` **右往左**取第一个不受信地址（跳过受信代理链）；
///   只要遇到无法解析的值或全部受信，就退回对端地址（fail-closed）。
pub fn resolve_client_ip(
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
    trusted_proxies: &[Cidr],
) -> IpAddr {
    let Some(peer) = peer else {
        return UNKNOWN_IP;
    };
    let peer_ip = peer.ip();
    if !trusted_proxies.iter().any(|cidr| cidr.contains(peer_ip)) {
        return peer_ip;
    }
    let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    else {
        return peer_ip;
    };
    for candidate in forwarded.split(',').rev().map(str::trim) {
        let Ok(candidate_ip) = candidate.parse::<IpAddr>() else {
            return peer_ip;
        };
        if !trusted_proxies
            .iter()
            .any(|cidr| cidr.contains(candidate_ip))
        {
            return candidate_ip;
        }
    }
    peer_ip
}

/// `{ data }` 响应 + `Cache-Control: no-store`（含凭据信息的响应不允许被缓存）。
fn json_no_store<T: serde::Serialize>(status: StatusCode, payload: T) -> Response {
    let mut response = (status, Json(payload)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

/// 给响应追加 `Set-Cookie`（用于登录）。
trait WithSessionCookie {
    fn with_session_cookie(self, cookie: &str) -> Response;
}

impl WithSessionCookie for Response {
    fn with_session_cookie(mut self, cookie: &str) -> Response {
        if let Ok(value) = axum::http::HeaderValue::from_str(cookie) {
            self.headers_mut().insert(header::SET_COOKIE, value);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CliOverrides, Settings, TlsConfig};
    use std::path::PathBuf;

    fn base_settings() -> Settings {
        let overrides = CliOverrides {
            data_dir: Some(PathBuf::from("/tmp/em-auth-unit")),
            config: None,
            listen: None,
        };
        // 不读环境：直接构造（字段全部公开，测试构造比环境变量更确定）。
        Settings {
            config_path: None,
            data_dir: overrides.data_dir.unwrap(),
            listen: "127.0.0.1:8080".parse().unwrap(),
            public_origin: None,
            tls: None,
            trusted_proxy_cidrs: Vec::new(),
            providers: crate::config::Providers {
                tripo: crate::config::ProviderSettings {
                    name: "tripo",
                    base_url: crate::config::DEFAULT_TRIPO_BASE_URL.to_owned(),
                    model: Some(crate::config::DEFAULT_TRIPO_MODEL.to_owned()),
                    api_key: None,
                    key_source: None,
                },
                manual_ai: crate::config::ProviderSettings {
                    name: "manual_ai",
                    base_url: crate::config::DEFAULT_MANUAL_AI_BASE_URL.to_owned(),
                    model: None,
                    api_key: None,
                    key_source: None,
                },
            },
            limits: crate::config::Limits::default(),
            concurrency: crate::config::Concurrency::default(),
            jobs: crate::config::Jobs::default(),
            session: crate::config::Session::default(),
            price_catalog_path: None,
            price_catalog: None,
            download: crate::config::DownloadSettings::default(),
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                axum::http::HeaderName::try_from(*name).unwrap(),
                axum::http::HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn session_cookie_attributes_include_secure_only_when_requested() {
        let plain = session_cookie("abc", 604_800, false);
        assert_eq!(
            plain,
            "em_session=abc; Path=/; HttpOnly; SameSite=Strict; Max-Age=604800"
        );
        assert!(!plain.contains("Secure"));

        let secure = session_cookie("abc", 604_800, true);
        assert!(secure.ends_with("; Secure"), "{secure}");
        assert!(cleared_session_cookie(true).contains("Max-Age=0"));
        assert!(cleared_session_cookie(false).contains("SameSite=Strict"));
    }

    #[test]
    fn cookie_parsing_matches_exact_name_only() {
        let map = headers(&[("cookie", "theme=dark; em_session=token-value; other=1")]);
        assert_eq!(
            session_token_from_cookie(&map).as_deref(),
            Some("token-value")
        );
        let empty = headers(&[("cookie", "em_session=; other=1")]);
        assert!(session_token_from_cookie(&empty).is_none());
        let none = headers(&[("cookie", "em_session_extra=1")]);
        assert!(session_token_from_cookie(&none).is_none());
    }

    #[test]
    fn origin_allowed_uses_public_origin_or_same_host() {
        let settings = base_settings();
        let host = headers(&[("host", "127.0.0.1:8080")]);
        assert!(origin_allowed(&settings, &host, "http://127.0.0.1:8080"));
        assert!(!origin_allowed(&settings, &host, "http://127.0.0.1:80"));
        assert!(!origin_allowed(&settings, &host, "https://127.0.0.1:8080"));
        assert!(!origin_allowed(&settings, &host, "http://evil.example"));
        assert!(!origin_allowed(
            &settings,
            &host,
            "http://127.0.0.1:8080/path"
        ));
        assert!(!origin_allowed(&settings, &host, "file:///etc/passwd"));

        let mut with_origin = base_settings();
        with_origin.public_origin = Some("https://manual.example".to_owned());
        let any_host = headers(&[("host", "10.0.0.5:8080")]);
        assert!(origin_allowed(
            &with_origin,
            &any_host,
            "https://manual.example"
        ));
        assert!(!origin_allowed(
            &with_origin,
            &any_host,
            "http://10.0.0.5:8080"
        ));
    }

    #[test]
    fn origin_missing_passes_but_bad_origin_fails() {
        let settings = base_settings();
        let host = headers(&[("host", "127.0.0.1:8080")]);
        assert!(
            origin_check(&settings, &host).is_ok(),
            "缺失 Origin 由 CSRF 兜底"
        );
        let cross_site = headers(&[
            ("host", "127.0.0.1:8080"),
            ("origin", "https://attacker.example"),
        ]);
        let error = origin_check(&settings, &cross_site).unwrap_err();
        assert_eq!(error.status, StatusCode::FORBIDDEN);
        assert_eq!(error.code.as_str(), "ORIGIN_REJECTED");
    }

    #[test]
    fn csrf_check_requires_matching_header() {
        let token = tokens::csrf_token_for("session-token-abc");
        let stored = tokens::csrf_hash(&token);
        assert!(csrf_check(&headers(&[(CSRF_HEADER, token.as_str())]), &stored).is_ok());
        assert!(csrf_check(&headers(&[]), &stored).is_err(), "缺失必须拒绝");
        assert!(csrf_check(&headers(&[(CSRF_HEADER, "other")]), &stored).is_err());
    }

    #[test]
    fn client_ip_resolution_ignores_untrusted_forwarded_headers() {
        let trusted = vec![Cidr::parse("10.0.0.0/8").unwrap()];
        let peer: SocketAddr = "203.0.113.5:5555".parse().unwrap();
        let spoofed = headers(&[("x-forwarded-for", "1.2.3.4")]);
        assert_eq!(
            resolve_client_ip(Some(peer), &spoofed, &trusted),
            "203.0.113.5".parse::<IpAddr>().unwrap(),
            "未受信对端不得用 XFF 改变来源"
        );

        let proxy: SocketAddr = "10.0.0.9:5555".parse().unwrap();
        let chain = headers(&[("x-forwarded-for", "198.51.100.7, 10.0.0.2")]);
        assert_eq!(
            resolve_client_ip(Some(proxy), &chain, &trusted),
            "198.51.100.7".parse::<IpAddr>().unwrap(),
            "受信代理链右往左取第一个不受信地址"
        );

        let all_trusted = headers(&[("x-forwarded-for", "10.0.0.2, 10.0.0.3")]);
        assert_eq!(
            resolve_client_ip(Some(proxy), &all_trusted, &trusted),
            "10.0.0.9".parse::<IpAddr>().unwrap()
        );

        let broken = headers(&[("x-forwarded-for", "not-an-ip")]);
        assert_eq!(
            resolve_client_ip(Some(proxy), &broken, &trusted),
            "10.0.0.9".parse::<IpAddr>().unwrap()
        );

        assert_eq!(resolve_client_ip(None, &headers(&[]), &trusted), UNKNOWN_IP);
    }

    #[test]
    fn tls_config_alone_marks_cookies_secure() {
        let mut settings = base_settings();
        assert!(!settings.cookie_secure());
        settings.tls = Some(TlsConfig {
            cert_file: PathBuf::from("/tmp/cert.pem"),
            key_file: PathBuf::from("/tmp/key.pem"),
        });
        // auto 判定包含"内置 TLS 配置"这一服务端已知事实。
        assert!(settings.cookie_secure());
        settings.tls = None;
        settings.public_origin = Some("https://manual.example".to_owned());
        assert!(settings.cookie_secure());
    }
}
