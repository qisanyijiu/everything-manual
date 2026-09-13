//! fixture 场景脚本：以 JSON 描述"每个路由每次请求做什么"。
//!
//! 设计意图（REQ-008 / AC-014）：
//! - **脚本即数据**：`tests/fixtures/scenarios/*.json` 是场景的唯一来源；测试不改
//!   fixture 行为时不需要重编译。服务端在收到请求时按路由顺序取"下一步"，
//!   步骤耗尽且未声明 `repeatLast` 时返回 501 并记入 script problem，
//!   **绝不返回通用成功**（缺脚本必须失败）。
//! - **响应体可引用文件**：`{"file": "responses/xxx.json"}` 相对 fixture 根目录
//!   （默认 `tests/fixtures/`）解析；加载时一次性读入内存，运行期不再做 IO。
//! - 反序列化阶段就把错误暴露出来：未知 `kind`、缺字段、文件缺失、`truncateAt`
//!   超出响应体长度都会在 `FixtureServer::start` 时 panic 并给出可读信息。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// fixture 资产根目录：`<repo>/tests/fixtures`（相对本 crate 的 `CARGO_MANIFEST_DIR`）。
pub fn fixtures_root() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    path.canonicalize().unwrap_or(path)
}

/// 路径匹配方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PathMatchSpec {
    /// 完全相等（不含查询串）。
    #[default]
    Exact,
    /// 前缀匹配（用于 `/v3/tasks/<id>` 这类带变量路径）。
    Prefix,
}

/// 单个路由的脚本。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RouteScript {
    /// HTTP 方法；`*` 表示任意方法。
    pub method: String,
    pub path: String,
    /// JSON 键为 `match`（`match` 是 Rust 关键字）。
    #[serde(rename = "match", default)]
    pub path_match: PathMatchSpec,
    /// 步骤耗尽后重复最后一步（默认 false：返回 501 并记录 script problem）。
    #[serde(default)]
    pub repeat_last: bool,
    pub steps: Vec<Step>,
}

/// 单次请求的行为。JSON 用 `"kind"` 标签区分。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Step {
    /// 立即返回完整响应。
    Respond { response: ResponseSpec },
    /// 等待 `delayMs` 再返回完整响应。
    Delay {
        delay_ms: u64,
        response: ResponseSpec,
    },
    /// 读完请求后直接关闭连接（FIN，无任何响应字节）。
    Disconnect,
    /// 读完请求后以 RST 中断连接（SO_LINGER=0）。
    Reset,
    /// 返回响应头与响应体的前 `truncateAt` 字节后半关闭写方向（客户端会读到截断）。
    HalfClose {
        response: ResponseSpec,
        truncate_at: usize,
    },
    /// 读完请求后保持连接但不写任何响应（客户端读超时）；`holdMs` 后服务端自行关闭。
    Timeout { hold_ms: u64 },
}

/// 响应描述。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResponseSpec {
    pub status: u16,
    /// 附加/覆盖响应头；未指定 `content-type` 时按响应体类型给默认值。
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub body: BodySpec,
}

/// 响应体来源。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum BodySpec {
    /// `{"json": {...}}`：序列化为 JSON 字节，默认 `content-type: application/json`。
    Json { json: Value },
    /// `{"text": "..."}`：原样 UTF-8 字节（畸形 JSON 也用这个表达），默认 `text/plain`。
    Text { text: String },
    /// `{"file": "responses/x.json"}`：读文件字节，默认 `application/json`。
    File { file: String },
}

/// 一个 fixture 场景（路由列表 + 响应体解析根目录）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Scenario {
    #[serde(default)]
    pub routes: Vec<RouteScript>,
    /// `BodySpec::File` 的解析根目录；不参与序列化。
    #[serde(skip)]
    pub fixture_root: PathBuf,
}

impl Scenario {
    /// 用代码构造场景（文件路径相对 `tests/fixtures/`）。
    pub fn new(routes: Vec<RouteScript>) -> Self {
        Self {
            routes,
            fixture_root: fixtures_root(),
        }
    }

    /// 从 JSON 文本解析（`fixture_root` 默认 `tests/fixtures/`）。
    pub fn from_json_str(text: &str) -> Self {
        Self::from_json_str_with_root(text, fixtures_root())
    }

    /// 从 JSON 文本解析并指定 fixture 根目录（便于测试用 `tempdir` 造脚本）。
    pub fn from_json_str_with_root(text: &str, fixture_root: PathBuf) -> Self {
        let mut scenario: Scenario =
            serde_json::from_str(text).unwrap_or_else(|error| panic!("场景 JSON 非法：{error}"));
        scenario.fixture_root = fixture_root;
        scenario
    }

    /// 读取 `tests/fixtures/scenarios/<name>`。
    pub fn load_scenario(name: &str) -> Self {
        let root = fixtures_root();
        Self::load_file(&root.join("scenarios").join(name), &root)
    }

    /// 读取任意路径的场景文件；`file` 类响应体相对 `fixture_root` 解析。
    pub fn load_file(path: &Path, fixture_root: &Path) -> Self {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("读取场景文件失败 {}：{error}", path.display()));
        Self::from_json_str_with_root(&text, fixture_root.to_path_buf())
    }

    /// 解析为服务端可直接执行的字节级脚本。
    pub fn resolve(&self) -> ResolvedScenario {
        ResolvedScenario {
            routes: self
                .routes
                .iter()
                .map(|route| resolve_route(route, &self.fixture_root))
                .collect(),
        }
    }
}

/// 字节级响应（`content-type` 默认值已补全）。
#[derive(Debug, Clone)]
pub struct ResolvedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// 字节级步骤。
#[derive(Debug, Clone)]
pub enum ResolvedStep {
    Respond(ResolvedResponse),
    Delay {
        delay_ms: u64,
        response: ResolvedResponse,
    },
    Disconnect,
    Reset,
    HalfClose {
        response: ResolvedResponse,
        truncate_at: usize,
    },
    Timeout {
        hold_ms: u64,
    },
}

/// 字节级路由。
#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    /// 已大写；`*` 表示任意方法。
    pub method: String,
    pub path: String,
    pub path_match: PathMatchSpec,
    pub repeat_last: bool,
    pub steps: Vec<ResolvedStep>,
}

impl ResolvedRoute {
    /// `path` 传入不含查询串的路径。
    pub fn matches(&self, method: &str, path: &str) -> bool {
        if self.method != "*" && !self.method.eq_ignore_ascii_case(method) {
            return false;
        }
        match self.path_match {
            PathMatchSpec::Exact => self.path == path,
            PathMatchSpec::Prefix => path.starts_with(&self.path),
        }
    }
}

/// 字节级场景。
#[derive(Debug, Clone)]
pub struct ResolvedScenario {
    pub routes: Vec<ResolvedRoute>,
}

fn resolve_route(route: &RouteScript, root: &Path) -> ResolvedRoute {
    ResolvedRoute {
        method: route.method.to_ascii_uppercase(),
        path: route.path.clone(),
        path_match: route.path_match,
        repeat_last: route.repeat_last,
        steps: route
            .steps
            .iter()
            .map(|step| resolve_step(step, root, &route.method, &route.path))
            .collect(),
    }
}

fn resolve_step(step: &Step, root: &Path, method: &str, path: &str) -> ResolvedStep {
    match step {
        Step::Respond { response } => ResolvedStep::Respond(resolve_response(response, root)),
        Step::Delay { delay_ms, response } => ResolvedStep::Delay {
            delay_ms: *delay_ms,
            response: resolve_response(response, root),
        },
        Step::Disconnect => ResolvedStep::Disconnect,
        Step::Reset => ResolvedStep::Reset,
        Step::HalfClose {
            response,
            truncate_at,
        } => {
            let response = resolve_response(response, root);
            assert!(
                *truncate_at <= response.body.len(),
                "{method} {path} 的 halfClose.truncateAt={truncate_at} 超出响应体长度 {}：\
                 截断场景必须真的少于完整响应",
                response.body.len()
            );
            ResolvedStep::HalfClose {
                response,
                truncate_at: *truncate_at,
            }
        }
        Step::Timeout { hold_ms } => ResolvedStep::Timeout { hold_ms: *hold_ms },
    }
}

fn resolve_response(spec: &ResponseSpec, root: &Path) -> ResolvedResponse {
    let (body, default_content_type) = match &spec.body {
        BodySpec::Json { json } => (
            serde_json::to_vec(json).expect("序列化场景 JSON 响应体"),
            "application/json; charset=utf-8",
        ),
        BodySpec::Text { text } => (text.clone().into_bytes(), "text/plain; charset=utf-8"),
        BodySpec::File { file } => {
            let path = if Path::new(file).is_absolute() {
                PathBuf::from(file)
            } else {
                root.join(file)
            };
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|error| panic!("读取响应体文件失败 {}：{error}", path.display()));
            (bytes, "application/json; charset=utf-8")
        }
    };
    let mut headers: Vec<(String, String)> = spec
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        headers.push(("content-type".to_owned(), default_content_type.to_owned()));
    }
    ResolvedResponse {
        status: spec.status,
        headers,
        body,
    }
}
