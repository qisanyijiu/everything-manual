//! 集成测试共享工具（T04 起）：临时 data-dir、真实 SQLite、进程内路由调用。
//!
//! 设计：
//! - 每个用例一个独立临时目录（`Drop` 自动清理），不接触真实 data-dir；
//! - `Settings` 在测试中**直接构造**（字段全部公开），不经环境变量加载：
//!   避免测试间因进程级环境变量互相干扰；
//! - 请求经 `Router::oneshot` 进程内执行，不监听端口、不产生外部网络调用。

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use everything_manual::config::{
    Cidr, Concurrency, CookieSecurePolicy, DEFAULT_MANUAL_AI_BASE_URL, DEFAULT_TRIPO_BASE_URL,
    DEFAULT_TRIPO_MODEL, DownloadSettings, Jobs, Limits, ProviderSettings, Providers, SecretString,
    Session, Settings, datadir,
};
use everything_manual::http::state::AppState;
use everything_manual::storage::repo::sessions::{self, NewSession};
use everything_manual::storage::{Database, repo};
use http_body_util::BodyExt;
use manual_core::timestamps::Timestamp;
use tower::ServiceExt;

/// 自动清理的临时目录。
pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub fn new(tag: &str) -> Self {
        // 并行用例可能在极短时间内取到相同的系统时间，因此再加一个进程内自增序号
        // 保证目录唯一（曾实测两个并行用例撞到同一 nanos 导致迁移冲突）。
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "em-auth-{tag}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("创建测试临时目录");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 测试用配置（与生产默认一致的最小集合；可按用例覆盖）。
pub fn test_settings(data_dir: &Path) -> Settings {
    Settings {
        config_path: None,
        data_dir: data_dir.to_path_buf(),
        listen: "127.0.0.1:8080".parse().expect("监听地址"),
        public_origin: None,
        tls: None,
        trusted_proxy_cidrs: Vec::new(),
        providers: Providers {
            tripo: ProviderSettings {
                name: "tripo",
                base_url: DEFAULT_TRIPO_BASE_URL.to_owned(),
                model: Some(DEFAULT_TRIPO_MODEL.to_owned()),
                api_key: None,
                key_source: None,
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
        // 默认空名单：拒绝一切模型下载（用例按需显式配置允许域）。
        download: DownloadSettings::default(),
    }
}

/// 已装配的应用：临时 data-dir（结构 + 迁移完成的数据库）+ 进程内路由。
pub struct TestApp {
    router: Router,
    state: AppState,
    dir: TestDir,
}

impl TestApp {
    /// 标准测试应用（laoder：默认配置，无 Provider 配置，无限速自定义）。
    pub async fn new(tag: &str) -> Self {
        let dir = TestDir::new(tag);
        let settings = test_settings(dir.path());
        Self::with_settings(dir, settings).await
    }

    /// 用自定义 `Settings` 装配（如 HTTPS public_origin、短限速窗口）。
    pub async fn with_settings(dir: TestDir, settings: Settings) -> Self {
        datadir::ensure_initialized(settings.data_dir.as_path()).expect("初始化 data-dir 结构");
        let database = Database::open_and_migrate(&settings.data_dir)
            .await
            .expect("打开并迁移数据库");
        let state = AppState::new(database, settings);
        let router = everything_manual::http::router::build_app(state.clone());
        Self { router, state, dir }
    }

    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// 路由句柄（克隆）：需要把请求放进独立任务（并发竞争、观察 panic）时使用。
    pub fn router_handle(&self) -> Router {
        self.router.clone()
    }

    /// 写入/更新管理员密码（等价 `init` 的持久化步骤）。
    pub async fn set_admin_password(&self, password: &str) -> String {
        let hash = everything_manual::http::auth::password::hash_password(password)
            .expect("计算 Argon2 哈希");
        let mut connection = self
            .state
            .database()
            .pool()
            .acquire()
            .await
            .expect("获取连接");
        match repo::admins::get_single(&mut connection)
            .await
            .expect("读取管理员")
        {
            Some(admin) => {
                repo::admins::update_password(&mut connection, &admin.id, &hash)
                    .await
                    .expect("更新管理员密码");
                admin.id
            }
            None => {
                repo::admins::insert(&mut connection, &hash)
                    .await
                    .expect("创建管理员")
                    .id
            }
        }
    }

    /// 直接插入一个会话行（用于构造"已过期会话"等状态），返回
    /// `(cookie, csrfToken, sessionId)`。
    ///
    /// `ttl_millis <= 0` 表示"已过期"：先按约束（`expires_at > created_at`）插入，
    /// 再把 `expires_at` 改到过去——这是测试专用手法，等价于真实会话过期后的库状态。
    pub async fn insert_session(
        &self,
        admin_id: &str,
        ttl_millis: i64,
    ) -> (String, String, String) {
        let token = everything_manual::http::auth::tokens::generate_session_token()
            .expect("生成会话 token");
        let csrf = everything_manual::http::auth::tokens::csrf_token_for(&token);
        let now = Timestamp::now();
        let insert_ttl = ttl_millis.abs().max(1_000);
        let expires_at = now
            .checked_add_millis(insert_ttl)
            .expect("会话过期时间溢出");
        let mut connection = self
            .state
            .database()
            .pool()
            .acquire()
            .await
            .expect("获取连接");
        let session = sessions::create(
            &mut connection,
            NewSession {
                admin_id: admin_id.to_owned(),
                session_token_hash: everything_manual::http::auth::tokens::session_token_hash(
                    &token,
                ),
                csrf_hash: everything_manual::http::auth::tokens::csrf_hash(&csrf),
                expires_at,
            },
        )
        .await
        .expect("创建会话");
        if ttl_millis <= 0 {
            let past = now.checked_add_millis(ttl_millis).expect("过期时间溢出");
            // 表的 CHECK 要求 expires_at > created_at：把 created_at 也一起前移，
            // 得到"过去创建、过去过期"的等价状态。
            let created = past.checked_add_millis(-3_600_000).expect("创建时间溢出");
            sqlx::query("UPDATE sessions SET created_at = ?, expires_at = ? WHERE id = ?")
                .bind(created.as_millis())
                .bind(past.as_millis())
                .bind(&session.id)
                .execute(&mut *connection)
                .await
                .expect("把会话改为已过期");
        }
        (format!("em_session={token}"), csrf, session.id)
    }

    /// 发起一次请求（进程内）：`app.call(...).cookie(..).json(..).send().await`。
    pub fn call(&self, method: Method, uri: &str) -> CallBuilder<'_> {
        CallBuilder {
            app: self,
            method,
            uri: uri.to_owned(),
            headers: Vec::new(),
            body: None,
        }
    }
}

/// 请求构造器：链式设置 cookie / CSRF / Origin / JSON 体。
pub struct CallBuilder<'a> {
    app: &'a TestApp,
    method: Method,
    uri: String,
    headers: Vec<(String, String)>,
    body: Option<(Option<String>, Vec<u8>)>,
}

impl<'a> CallBuilder<'a> {
    pub fn cookie(mut self, cookie: &str) -> Self {
        self.headers.push(("cookie".to_owned(), cookie.to_owned()));
        self
    }

    pub fn csrf(mut self, token: &str) -> Self {
        self.headers
            .push(("x-csrf-token".to_owned(), token.to_owned()));
        self
    }

    pub fn origin(mut self, origin: &str) -> Self {
        self.headers.push(("origin".to_owned(), origin.to_owned()));
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn json(mut self, value: &serde_json::Value) -> Self {
        self.body = Some((
            Some("application/json".to_owned()),
            serde_json::to_vec(value).expect("序列化 JSON"),
        ));
        self
    }

    /// 原始请求体（可指定或省略 Content-Type；用于 415/413/422 用例）。
    pub fn raw_body(mut self, content_type: Option<&str>, body: Vec<u8>) -> Self {
        self.body = Some((content_type.map(str::to_owned), body));
        self
    }

    pub async fn send(self) -> TestResponse {
        let mut builder = Request::builder().method(self.method).uri(&self.uri);
        // 与真实浏览器/HTTP 客户端一致地提供 Host（Origin 同源判定依赖它）。
        if !self
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("host"))
        {
            builder = builder.header("host", "127.0.0.1:8080");
        }
        for (name, value) in &self.headers {
            builder = builder.header(name, HeaderValue::from_str(value).expect("头值合法"));
        }
        let body = match self.body {
            None => Body::empty(),
            Some((content_type, bytes)) => {
                if let Some(content_type) = content_type {
                    builder = builder.header("content-type", content_type);
                }
                Body::from(bytes)
            }
        };
        let request = builder.body(body).expect("构造请求");
        let response = self
            .app
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router 应能处理请求");
        TestResponse::collect(response).await
    }
}

/// 收拢后的响应（状态 + 头 + 字节体）。
pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl TestResponse {
    async fn collect(response: axum::response::Response) -> Self {
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("读取响应体")
            .to_bytes()
            .to_vec();
        Self {
            status,
            headers,
            body,
        }
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("响应体应是合法 JSON")
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    /// 全部 `Set-Cookie` 值。
    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(str::to_owned)
            .collect()
    }

    /// 会话 cookie 名值对（`em_session=<token>`），可直接回传。
    pub fn session_cookie(&self) -> String {
        let raw = self
            .set_cookies()
            .into_iter()
            .find(|cookie| cookie.starts_with("em_session="))
            .expect("响应应包含会话 cookie");
        raw.split(';')
            .next()
            .expect("cookie 至少包含名值对")
            .to_owned()
    }

    pub fn error_code(&self) -> Option<String> {
        self.json()["error"]["code"].as_str().map(str::to_owned)
    }

    pub fn error_request_id(&self) -> Option<String> {
        self.json()["error"]["requestId"]
            .as_str()
            .map(str::to_owned)
    }

    /// 统一错误结构自检：四个键、requestId 与响应头一致。
    pub fn assert_contract_error(&self, expected_status: StatusCode, expected_code: &str) {
        assert_eq!(self.status, expected_status, "响应体：{}", self.text());
        let body = self.json();
        assert_eq!(
            body["error"]["code"],
            expected_code,
            "响应体：{}",
            self.text()
        );
        let keys: Vec<&String> = body["error"].as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 4, "错误体不得包含额外字段：{keys:?}");
        let request_id = body["error"]["requestId"].as_str().unwrap_or_default();
        assert!(
            uuid::Uuid::parse_str(request_id).is_ok(),
            "requestId 应为 UUID，实际 {request_id:?}"
        );
        assert_eq!(
            self.header("x-request-id").as_deref(),
            Some(request_id),
            "响应头的 requestId 必须与错误体一致"
        );
    }
}

/// 构造可信代理网段列表（测试辅助）。
pub fn cidr(value: &str) -> Cidr {
    Cidr::parse(value).expect("合法 CIDR")
}

/// 配置一个已配置（假凭据）的 Tripo：用于验证 `/settings/status` 不泄露密钥与来源。
pub fn configured_tripo(canary_key: &str) -> ProviderSettings {
    ProviderSettings {
        name: "tripo",
        base_url: DEFAULT_TRIPO_BASE_URL.to_owned(),
        model: Some(DEFAULT_TRIPO_MODEL.to_owned()),
        api_key: Some(SecretString::new(canary_key)),
        key_source: Some(format!("环境变量 CANARY_SOURCE_{canary_key}")),
    }
}

/// 显式 cookie Secure 策略（供 HTTPS 判定用例）。
pub fn session_with_secure(policy: CookieSecurePolicy) -> Session {
    Session {
        cookie_secure: policy,
        ..Session::default()
    }
}
