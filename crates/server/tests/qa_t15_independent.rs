//! QA 回合 17 · T15 独立验收用例（AC-047 / AC-048 / AC-037 / AC-039 / AC-040 端点侧 + 卡内项）。
//!
//! 独立性声明：本文件由 QA 现场编写，**不复用** `tests/pipeline.rs` 的用例、断言、
//! `test_support::FixtureServer` 场景脚本或 `tests/fixtures/responses/**` 的响应样例：
//! - HTTP 目标是 QA 手写的**原始 TCP fixture**（[`QaHttp`]，只用 `std::net`），只绑定
//!   `127.0.0.1:0`，按"方法 + 路径（精确/前缀）+ 请求体标记"路由，逐请求记录
//!   方法/target/请求头/原始字节；未匹配的请求返回 501 并被记为 unexpected；
//! - Tripo（上传/提交/查询/CDN 下载）与说明书 AI 的响应体全部由本文件现场构造，
//!   断言落在**服务端产物**（数据库状态、API 响应、账本、审计）而不是 fixture 自证；
//! - 全链路、取消、重试、对账均由 QA 自己驱动执行器与 API（不引用 RD 的冒烟脚本输出）。
//!
//! 覆盖：
//! - AC-047：全链路草稿 `needs_review`；`job succeeded ≠ published`；重启不重复创建草稿
//!   （崩溃现场还原 + 恢复重跑）；部分成功草稿（分支头阻塞 → `partial` + `missing[]`）；
//!   上游批次阻塞（分支头仍 `queued`）时**不组装**（核对 assemble 领取放宽的语义边界）；
//! - AC-048：`manual_releases` 计数 0；publish 路由 404（无自动发布路径）；草稿 notices；
//! - AC-037：`submission_unknown` 暂停该分支购买、retry 被拒且无副作用、三种 reconcile 动作的
//!   正负例（attach 仅 Tripo + 查询验证 + 二次确认；recordNoTask 需证据且不释放预留；
//!   authorizeReplacement 需再次预算确认 + 重复收费确认 + 旧未决账务保留）、审计保留；
//! - AC-039：取消未提交阶段停止推进、已提交阶段保留查询与账务、不声称取消远端付费操作、
//!   取消后不新增付费步骤（fixture 计数）、审计；
//! - AC-040：只重跑指定阶段、已完成成果保留、If-Match + Idempotency-Key、unknown 不得盲重试、
//!   重试不改变模型/质量预设（快照比对）；
//! - 卡内项：`assemble_draft` 幂等 upsert；草稿读取 ETag 与 PATCH 428/412/422/404；
//!   T14 P3-1（拒答/无知识批次在"结果已落库、checkpoint 未推进"的恢复路径不得补推进为成功知识，
//!   并带"产出知识批次仍按 T10 补推进"的正对照）。
//!
//! 零真实外网：全部 HTTP 目标为 127.0.0.1 上由本文件启动的 fixture；凭据是 canary。

mod common;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestDir};
use everything_manual::config::{ProviderSettings, SecretString};
use everything_manual::generation::catalog;
use everything_manual::jobs::executor::fixed_jitter_executor;
use everything_manual::jobs::{
    ExecutorConfig, JobExecutor, ManualClock, PipelineHandlers, StageRegistry,
};
use everything_manual::providers::register_provider_handlers;
use everything_manual::storage::repo::{
    attempts as attempts_repo, job_stages as stages_repo, jobs as jobs_repo, ledger as ledger_repo,
    snapshots as snapshots_repo,
};
use manual_core::domain::{Job, JobStage, JobStatus, LedgerState, ManualDraft, StageKind};
use manual_core::timestamps::Timestamp;
use serde_json::{Value, json};
use sqlx::SqlitePool;

const PASSWORD: &str = "qa-t15-password-4e77";
const CANARY_KEY: &str = "canary-qa-t15-not-a-real-key";
const MANUAL_AI_MODEL: &str = "qa-manual-model-v1";
const PRESET: &str = "tripo-h-v3.1-standard";

const TEST_CATALOG: &str = r#"
version = "2026-09-11"
snapshot_date = "2026-09-11"

[[tripo.presets]]
preset = "tripo-h-v3.1-standard"
model = "v3.1-20260211"
credits = "30"

[manual_ai.models.qa-manual-model-v1]
input_usd_per_million_tokens = "0.25"
output_usd_per_million_tokens = "2.00"
image_usd_per_image = "0.01"
"#;

const TRIPO_UPLOAD_PATH: &str = "/v3/files";
const TRIPO_SUBMIT_PATH: &str = "/v3/generation/multiview-to-model";
const TRIPO_TASKS_PREFIX: &str = "/v3/tasks/";
const MANUAL_PATH: &str = "/v1/responses";
const CDN_PREFIX: &str = "/cdn/";

// ---------------------------------------------------------------------------
// QA 自建原始 TCP fixture（按方法 + 路径 + 请求体标记路由）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QaMode {
    /// 完整响应（`content-length` 与实际 body 一致）。
    Full,
    /// 声明完整长度但只写前 `written` 字节后关闭写端：客户端拿到"被截断的响应"。
    Truncated { written: usize },
}

#[derive(Debug, Clone)]
struct QaScript {
    status: u16,
    mode: QaMode,
    content_type: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl QaScript {
    fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            mode: QaMode::Full,
            content_type: "application/json".to_owned(),
            headers: Vec::new(),
            body: value.to_string().into_bytes(),
        }
    }

    fn bytes(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            mode: QaMode::Full,
            content_type: content_type.to_owned(),
            headers: Vec::new(),
            body,
        }
    }

    fn truncated(body: Vec<u8>, written: usize) -> Self {
        Self {
            status: 200,
            mode: QaMode::Truncated { written },
            content_type: "application/json".to_owned(),
            headers: Vec::new(),
            body,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    Exact,
    Prefix,
}

#[derive(Debug, Clone)]
struct QaRoute {
    method: String,
    path: String,
    kind: MatchKind,
    /// 只为说明书 AI 用：请求体必须包含该标记才命中（按批内页内容路由，不依赖领取顺序）。
    body_marker: Option<String>,
    steps: Vec<QaScript>,
    cursor: usize,
    repeat_last: bool,
}

fn route(method: &str, path: &str, kind: MatchKind, steps: Vec<QaScript>) -> QaRoute {
    QaRoute {
        method: method.to_owned(),
        path: path.to_owned(),
        kind,
        body_marker: None,
        steps,
        cursor: 0,
        repeat_last: true,
    }
}

fn route_marked(method: &str, path: &str, marker: &str, steps: Vec<QaScript>) -> QaRoute {
    QaRoute {
        method: method.to_owned(),
        path: path.to_owned(),
        kind: MatchKind::Exact,
        body_marker: Some(marker.to_owned()),
        steps,
        cursor: 0,
        repeat_last: true,
    }
}

#[derive(Debug, Clone)]
struct QaRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl QaRequest {
    fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

struct QaState {
    routes: Mutex<Vec<QaRoute>>,
    requests: Mutex<Vec<QaRequest>>,
    unexpected: Mutex<Vec<String>>,
    connections: AtomicUsize,
}

/// 只绑定回环、逐请求记录、可脚本化截断与本机模型 CDN 的 fixture。
struct QaHttp {
    addr: SocketAddr,
    state: Arc<QaState>,
    stop: Arc<AtomicBool>,
}

impl QaHttp {
    fn start(routes: Vec<QaRoute>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定回环端口");
        listener.set_nonblocking(true).expect("非阻塞监听");
        let addr = listener.local_addr().expect("本地地址");
        assert!(addr.ip().is_loopback(), "QA fixture 只能是回环地址");
        let state = Arc::new(QaState {
            routes: Mutex::new(routes),
            requests: Mutex::new(Vec::new()),
            unexpected: Mutex::new(Vec::new()),
            connections: AtomicUsize::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread_state = Arc::clone(&state);
        let thread_stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name(format!("qa-t15-fixture-{}", addr.port()))
            .spawn(move || {
                while !thread_stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            thread_state.connections.fetch_add(1, Ordering::SeqCst);
                            handle_connection(stream, &thread_state);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
            .expect("启动 QA fixture 线程");
        Self { addr, state, stop }
    }

    fn tripo_base(&self) -> String {
        format!("http://{}/v3", self.addr)
    }

    fn manual_base(&self) -> String {
        format!("http://{}/v1", self.addr)
    }

    fn cdn_url(&self) -> String {
        format!(
            "http://{}{}model.glb?sign=qa-t15-signature-secret",
            self.addr, CDN_PREFIX
        )
    }

    fn requests(&self) -> Vec<QaRequest> {
        self.state.requests.lock().unwrap().clone()
    }

    fn count(&self, method: &str, target: &str) -> usize {
        self.requests()
            .into_iter()
            .filter(|request| request.method == method && request.target == target)
            .count()
    }

    /// 付费提交（Tripo 多视图生成）次数。
    fn paid_submits(&self) -> usize {
        self.count("POST", TRIPO_SUBMIT_PATH)
    }

    /// 说明书 AI 批次请求次数。
    fn manual_requests(&self) -> usize {
        self.count("POST", MANUAL_PATH)
    }

    /// 模型下载次数。
    fn downloads(&self) -> usize {
        self.requests()
            .into_iter()
            .filter(|request| request.method == "GET" && request.target.starts_with(CDN_PREFIX))
            .count()
    }

    fn total_requests(&self) -> usize {
        self.requests().len()
    }

    /// 目标为指定 method+target 的请求（含请求头，用于核对鉴权路径）。
    fn requests_to(&self, method: &str, target: &str) -> Vec<QaRequest> {
        self.requests()
            .into_iter()
            .filter(|request| request.method == method && request.target == target)
            .collect()
    }

    /// 任何非预期请求（未命中路由 / 目标不在白名单）——"模型让程序访问 URL"之类的信号。
    fn unexpected(&self) -> Vec<String> {
        self.state.unexpected.lock().unwrap().clone()
    }
}

impl Drop for QaHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn handle_connection(mut stream: TcpStream, state: &QaState) {
    // macOS/BSD：accept() 返回的 socket 继承监听 socket 的 O_NONBLOCK，
    // 不显式改回阻塞时 read() 会在客户端字节到达前返回 EAGAIN。
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        match stream.read(&mut chunk) {
            Ok(0) => break None,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(position) = find_subslice(&buffer, b"\r\n\r\n") {
                    break Some(position);
                }
            }
            Err(_) => break None,
        }
    };
    let Some(header_end) = header_end else {
        return;
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let content_length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => body.extend_from_slice(&chunk[..read]),
            Err(_) => break,
        }
    }
    let request = QaRequest {
        method: method.clone(),
        target: target.clone(),
        headers,
        body,
    };
    let path = target.split('?').next().unwrap_or_default().to_string();
    let body_text = request.body_text();
    state.requests.lock().unwrap().push(request);

    let script = {
        let mut routes = state.routes.lock().unwrap();
        let mut found = None;
        for route in routes.iter_mut() {
            let method_ok = route.method == method;
            let path_ok = match route.kind {
                MatchKind::Exact => route.path == path,
                MatchKind::Prefix => path.starts_with(&route.path),
            };
            let marker_ok = route
                .body_marker
                .as_ref()
                .is_none_or(|marker| body_text.contains(marker));
            if method_ok && path_ok && marker_ok {
                let index = route.cursor.min(route.steps.len().saturating_sub(1));
                if route.cursor < route.steps.len() {
                    route.cursor += 1;
                } else if !route.repeat_last {
                    state
                        .unexpected
                        .lock()
                        .unwrap()
                        .push(format!("{method} {path}（脚本已耗尽）"));
                }
                found = Some(route.steps[index].clone());
                break;
            }
        }
        found
    };

    let Some(script) = script else {
        state
            .unexpected
            .lock()
            .unwrap()
            .push(format!("{method} {path}（未命中任何脚本）"));
        write_response(
            &mut stream,
            501,
            &[0u8; 0],
            QaMode::Full,
            "application/json",
            &[],
        );
        return;
    };
    // 仅用于 lsof 观测的**正对照**（默认不延迟）：把连接窗口拉长到可被采样看见。
    if let Ok(text) = std::env::var("QA_T15_SLOW_MS")
        && let Ok(millis) = text.parse::<u64>()
        && millis > 0
    {
        std::thread::sleep(Duration::from_millis(millis));
    }
    write_response(
        &mut stream,
        script.status,
        &script.body,
        script.mode,
        &script.content_type,
        &script.headers,
    );
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    body: &[u8],
    mode: QaMode,
    content_type: &str,
    headers: &[(String, String)],
) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        501 => "Not Implemented",
        _ => "Status",
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    match mode {
        QaMode::Full => {
            let _ = stream.write_all(body);
        }
        QaMode::Truncated { written } => {
            let take = written.min(body.len());
            let _ = stream.write_all(&body[..take]);
        }
    }
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.shutdown(Shutdown::Both);
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ---------------------------------------------------------------------------
// QA 构造的供应商响应（不使用 RD 的 fixture 文件）
// ---------------------------------------------------------------------------

fn manual_envelope(id: &str, output_text: &str) -> Vec<u8> {
    json!({
        "id": id,
        "status": "completed",
        "model": MANUAL_AI_MODEL,
        "output": [{
            "type": "message",
            "content": [{ "type": "output_text", "text": output_text }]
        }],
        "usage": { "input_tokens": 11, "output_tokens": 22, "total_tokens": 33 }
    })
    .to_string()
    .into_bytes()
}

fn manual_refusal(id: &str) -> Vec<u8> {
    json!({
        "id": id,
        "status": "completed",
        "model": MANUAL_AI_MODEL,
        "output": [{ "type": "message", "content": [
            { "type": "refusal", "refusal": "QA 构造：拒绝提取" }
        ]}]
    })
    .to_string()
    .into_bytes()
}

fn manual_success_for(id: &str, pages: &[i64]) -> Vec<u8> {
    let parts: Vec<Value> = pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            json!({
                "id": format!("qa-part-{index}-{page}"),
                "name": format!("QA 部件 {index}（第 {page} 页）"),
                "description": "QA 现场构造的描述",
                "evidence": [{ "pageNumber": page, "quote": "QA 引文" }]
            })
        })
        .collect();
    let payload = json!({
        "schemaVersion": "manual_extract_v1",
        "parts": parts,
        "steps": [{
            "id": "qa-step-0",
            "title": "QA 步骤",
            "orderedActions": ["QA 动作一", "QA 动作二"],
            "partIds": [parts[0]["id"].clone()],
            "evidence": [{ "pageNumber": pages[0], "quote": "QA 步骤引文" }],
            "safetyNotes": []
        }],
        "specs": [],
        "uncertainties": []
    })
    .to_string();
    manual_envelope(id, &payload)
}

fn tripo_upload_script(token: &str) -> QaScript {
    QaScript::json(200, json!({ "code": 0, "data": { "file_token": token } }))
}

fn tripo_submit_script(task_id: &str) -> QaScript {
    QaScript::json(200, json!({ "code": 0, "data": { "task_id": task_id } }))
}

fn tripo_running_script(task_id: &str) -> QaScript {
    QaScript::json(
        200,
        json!({ "code": 0, "data": { "task_id": task_id, "status": "running", "progress": 42 } }),
    )
}

fn tripo_success_script(task_id: &str, model_url: &str) -> QaScript {
    QaScript::json(
        200,
        json!({
            "code": 0,
            "data": {
                "task_id": task_id,
                "status": "success",
                "progress": 100,
                "credits_consumed": 30,
                "output": { "model_url": model_url }
            }
        }),
    )
}

/// CDN 路由（前缀；返回给定字节）。
fn cdn_route(body: Vec<u8>) -> QaRoute {
    route(
        "GET",
        CDN_PREFIX,
        MatchKind::Prefix,
        vec![QaScript::bytes(200, "model/gltf-binary", body)],
    )
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("读取样例资产 {} 失败：{error}", path.display()))
}

/// 页内容标记（唯一、可路由）。
fn page_marker(page: i64) -> String {
    format!("QA-T15-PAGE-{page}-MARK")
}

fn marked_pages(count: i64) -> Vec<PageSpec> {
    (1..=count)
        .map(|page| PageSpec::Text(page_marker(page)))
        .collect()
}

// ---------------------------------------------------------------------------
// 应用、输入与 HTTP 工具
// ---------------------------------------------------------------------------

fn qa_settings(dir: &Path, fixture: &QaHttp) -> everything_manual::config::Settings {
    let mut settings = common::test_settings(dir);
    let mut tripo = common::configured_tripo(CANARY_KEY);
    tripo.base_url = fixture.tripo_base();
    settings.providers.tripo = tripo;
    settings.providers.manual_ai = ProviderSettings {
        name: "manual_ai",
        base_url: fixture.manual_base(),
        model: Some(MANUAL_AI_MODEL.to_owned()),
        api_key: Some(SecretString::new(CANARY_KEY)),
        key_source: Some("QA 现场注入".to_owned()),
    };
    // 模型下载：显式放行回环 fixture（测试构建；正式包见 validation-release §3）。
    settings.download.allowed_hosts = vec!["127.0.0.1".to_owned()];
    settings.download.allow_local_fixture = true;
    settings.price_catalog_path = Some(dir.join("price-catalog.toml"));
    std::fs::write(dir.join("price-catalog.toml"), TEST_CATALOG).expect("写入 QA 价格目录");
    settings.price_catalog = Some(catalog::parse(TEST_CATALOG).expect("QA 价格目录可解析"));
    settings
}

async fn qa_app(tag: &str, fixture: &QaHttp) -> (TestApp, String, String) {
    let dir = TestDir::new(tag);
    let settings = qa_settings(dir.path(), fixture);
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;
    let response = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&json!({ "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let csrf = response.json()["data"]["csrfToken"]
        .as_str()
        .expect("csrfToken")
        .to_owned();
    (app, response.session_cookie(), csrf)
}

struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new(tag: &str) -> Self {
        Self {
            boundary: format!("----qa-t15-{tag}"),
            body: Vec::new(),
        }
    }

    fn text_field(mut self, name: &str, value: &str) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self
    }

    fn file_field(mut self, name: &str, filename: &str, content_type: &str, bytes: &[u8]) -> Self {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n\
                 Content-Type: {content_type}\r\n\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self.body.extend_from_slice(bytes);
        self.body.extend_from_slice(b"\r\n");
        self
    }

    fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (
            format!("multipart/form-data; boundary={}", self.boundary),
            self.body,
        )
    }
}

// 参数即 multipart 的每一段（purpose/文件名/类型/字节），保持显式、不做 builder。
#[allow(clippy::too_many_arguments)]
async fn upload_asset(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    purpose: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> String {
    let (boundary, body) = Multipart::new(purpose)
        .text_field("purpose", purpose)
        .file_field("file", filename, content_type, bytes)
        .finish();
    let response = app
        .call(Method::POST, &format!("/api/v1/items/{item}/assets"))
        .cookie(cookie)
        .csrf(csrf)
        .raw_body(Some(&boundary), body)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()["data"]["id"].as_str().unwrap().to_owned()
}

#[derive(Debug, Clone)]
enum PageSpec {
    Text(String),
    Scan,
}

struct QaInputs {
    item: String,
    preparation: String,
    photo_ids: Vec<String>,
}

/// 物品 + PDF 文档 + N 页 + front/left 照片 + ready 准备（全部走公开 API）。
async fn build_inputs(app: &TestApp, cookie: &str, csrf: &str, pages: &[PageSpec]) -> QaInputs {
    let item = {
        let response = app
            .call(Method::POST, "/api/v1/items")
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({ "name": "QA T15 物品", "model": "QA-T15" }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        response.json()["data"]["id"].as_str().unwrap().to_owned()
    };
    let pdf = fixture_bytes("sample-manual-text.pdf");
    let doc_asset = upload_asset(
        app,
        cookie,
        csrf,
        &item,
        "document",
        "manual.pdf",
        "application/pdf",
        &pdf,
    )
    .await;
    let (document, source_sha) = {
        let response = app
            .call(Method::POST, &format!("/api/v1/items/{item}/documents"))
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({ "sourceAssetId": doc_asset, "title": "QA T15 样例说明书" }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        let json = response.json();
        (
            json["data"]["id"].as_str().unwrap().to_owned(),
            json["data"]["sourceSha256"].as_str().unwrap().to_owned(),
        )
    };
    let preparation = {
        let response = app
            .call(
                Method::POST,
                &format!("/api/v1/documents/{document}/preparations"),
            )
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({ "sourceSha256": source_sha }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        response.json()["data"]["id"].as_str().unwrap().to_owned()
    };

    let page_jpeg = fixture_bytes("sample-photo-front.jpg");
    for (index, page_spec) in pages.iter().enumerate() {
        let page = index as i64 + 1;
        let image = upload_asset(
            app,
            cookie,
            csrf,
            &item,
            "pageImage",
            "page.jpg",
            "image/jpeg",
            &page_jpeg,
        )
        .await;
        let text_asset = match page_spec {
            PageSpec::Text(content) => Some(
                upload_asset(
                    app,
                    cookie,
                    csrf,
                    &item,
                    "pageText",
                    "page.txt",
                    "text/plain",
                    content.as_bytes(),
                )
                .await,
            ),
            PageSpec::Scan => None,
        };
        let response = app
            .call(
                Method::PUT,
                &format!("/api/v1/preparations/{preparation}/pages/{page}"),
            )
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({
                "textAssetId": text_asset,
                "imageAssetId": image,
                "viewport": { "width": 1240, "height": 1754, "rotation": 0 },
            }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    }
    let etag = {
        let response = app
            .call(Method::GET, &format!("/api/v1/preparations/{preparation}"))
            .cookie(cookie)
            .send()
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        response.header("etag").expect("准备详情带 ETag").to_owned()
    };
    let completed = app
        .call(
            Method::POST,
            &format!("/api/v1/preparations/{preparation}/complete"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .header("if-match", &etag)
        .json(&json!({ "pageCount": pages.len() as i64 }))
        .send()
        .await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text());

    let mut photo_ids = Vec::new();
    for (view, name) in [
        ("front", "sample-photo-front.jpg"),
        ("left", "sample-photo-left.png"),
    ] {
        let bytes = fixture_bytes(name);
        let content_type = if name.ends_with(".png") {
            "image/png"
        } else {
            "image/jpeg"
        };
        let asset = upload_asset(
            app,
            cookie,
            csrf,
            &item,
            "photo",
            name,
            content_type,
            &bytes,
        )
        .await;
        let response = app
            .call(Method::POST, &format!("/api/v1/items/{item}/photos"))
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({ "assetId": asset, "view": view }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        photo_ids.push(response.json()["data"]["id"].as_str().unwrap().to_owned());
    }

    QaInputs {
        item,
        preparation,
        photo_ids,
    }
}

async fn create_job(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    inputs: &QaInputs,
    key: &str,
) -> String {
    let quote_id = {
        let response = app
            .call(
                Method::POST,
                &format!("/api/v1/items/{}/estimates", inputs.item),
            )
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({
                "preparationId": inputs.preparation,
                "photoIds": inputs.photo_ids,
                "modelPreset": PRESET,
            }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        response.json()["data"]["id"].as_str().unwrap().to_owned()
    };
    let confirmed = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{}/estimates/{quote_id}/confirm", inputs.item),
        )
        .cookie(cookie)
        .csrf(csrf)
        .send()
        .await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.text());
    let job = app
        .call(Method::POST, &format!("/api/v1/items/{}/jobs", inputs.item))
        .cookie(cookie)
        .csrf(csrf)
        .header("idempotency-key", key)
        .json(&json!({
            "quoteId": quote_id,
            "preparationId": inputs.preparation,
            "photoIds": inputs.photo_ids,
            "limits": { "tripoCreditMinor": 3000, "manualAiUsdMicros": 9_999_999 },
        }))
        .send()
        .await;
    assert_eq!(job.status, StatusCode::ACCEPTED, "{}", job.text());
    job.json()["data"]["id"].as_str().unwrap().to_owned()
}

// ---------------------------------------------------------------------------
// 执行器 / 数据库工具
// ---------------------------------------------------------------------------

fn pool(app: &TestApp) -> SqlitePool {
    app.state().database().pool().clone()
}

fn qa_executor(app: &TestApp, clock: Arc<ManualClock>) -> Arc<JobExecutor> {
    let settings = app.state().settings().clone();
    let mut registry = StageRegistry::new();
    register_provider_handlers(&mut registry, &settings).expect("Provider 处理器注册");
    let registered = PipelineHandlers::from_settings(&settings).register(&mut registry);
    assert!(
        registered.contains(&StageKind::AssembleDraft),
        "组装阶段必须注册（本地阶段，不依赖 Provider 配置）"
    );
    fixed_jitter_executor(
        pool(app),
        ExecutorConfig {
            lease: Duration::from_secs(120),
            renew: Duration::from_secs(20),
            ..ExecutorConfig::default()
        },
        registry,
        clock,
    )
}

async fn tick(executor: &Arc<JobExecutor>, clock: &ManualClock) {
    clock.advance_millis(20_000);
    let _ = executor.tick().await.expect("tick");
}

async fn stages_of(pool: &SqlitePool, job_id: &str) -> Vec<JobStage> {
    let mut conn = pool.acquire().await.expect("连接");
    stages_repo::list_for_job(&mut conn, job_id)
        .await
        .expect("列出阶段")
}

async fn stage_of(pool: &SqlitePool, job_id: &str, kind: StageKind) -> JobStage {
    stages_of(pool, job_id)
        .await
        .into_iter()
        .find(|stage| stage.stage_kind == kind)
        .unwrap_or_else(|| panic!("阶段 {} 不存在", kind.as_str()))
}

async fn batch_count(pool: &SqlitePool, job_id: &str, want: JobStatus) -> usize {
    stages_of(pool, job_id)
        .await
        .into_iter()
        .filter(|stage| stage.stage_kind == StageKind::ManualExtract && stage.status == want)
        .count()
}

async fn job_row(pool: &SqlitePool, job_id: &str) -> Job {
    let mut conn = pool.acquire().await.expect("连接");
    jobs_repo::get(&mut conn, job_id)
        .await
        .expect("读取 job")
        .expect("job 存在")
}

async fn job_snapshot(pool: &SqlitePool, job_id: &str) -> Value {
    let snapshot_id = job_row(pool, job_id).await.snapshot_id;
    let mut conn = pool.acquire().await.expect("连接");
    let snapshot = snapshots_repo::get(&mut conn, &snapshot_id)
        .await
        .expect("读取快照")
        .expect("快照存在");
    json!({
        "budgets": snapshot.budgets,
        "providerConfig": snapshot.provider_config,
        "promptVersion": snapshot.prompt_version,
        "priceVersion": snapshot.price_version,
    })
}

async fn draft_for_snapshot(pool: &SqlitePool, snapshot_id: &str) -> Option<ManualDraft> {
    let mut conn = pool.acquire().await.expect("连接");
    sqlx::query_as::<_, (String, String, String, Option<String>, i64, String, String, Option<String>)>(
        "SELECT id, item_id, snapshot_id, model_revision_id, revision, status, knowledge_json, review_json \
           FROM manual_drafts WHERE snapshot_id = ?",
    )
    .bind(snapshot_id)
    .fetch_optional(&mut *conn)
    .await
    .expect("读取草稿")
    .map(|row| ManualDraft {
        id: row.0,
        item_id: row.1,
        snapshot_id: row.2,
        model_revision_id: row.3,
        revision: row.4,
        status: match row.5.as_str() {
            "ready" => manual_core::domain::DraftStatus::Ready,
            _ => manual_core::domain::DraftStatus::NeedsReview,
        },
        knowledge_json: serde_json::from_str(&row.6).expect("草稿知识 JSON"),
        review_json: row.7.map(|text| serde_json::from_str(&text).expect("复核 JSON")),
        created_at: Timestamp::EPOCH,
        updated_at: Timestamp::EPOCH,
    })
}

async fn scalar_i64(pool: &SqlitePool, sql: &'static str, bind: Option<&str>) -> i64 {
    let mut conn = pool.acquire().await.expect("连接");
    let mut query = sqlx::query_scalar::<_, i64>(sql);
    if let Some(value) = bind {
        query = query.bind(value.to_owned());
    }
    query.fetch_one(&mut *conn).await.expect("标量查询")
}

async fn draft_count(pool: &SqlitePool, item_id: &str) -> i64 {
    scalar_i64(
        pool,
        "SELECT COUNT(*) FROM manual_drafts WHERE item_id = ?",
        Some(item_id),
    )
    .await
}

async fn release_count(pool: &SqlitePool) -> i64 {
    scalar_i64(pool, "SELECT COUNT(*) FROM manual_releases", None).await
}

async fn audit_count(pool: &SqlitePool, action: &str) -> i64 {
    scalar_i64(
        pool,
        "SELECT COUNT(*) FROM audit_events WHERE action = ?",
        Some(action),
    )
    .await
}

async fn audit_metadata(pool: &SqlitePool, action: &str) -> String {
    let mut conn = pool.acquire().await.expect("连接");
    sqlx::query_scalar::<_, String>(
        "SELECT COALESCE(metadata_json, '') FROM audit_events WHERE action = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(action)
    .fetch_one(&mut *conn)
    .await
    .expect("审计 metadata")
}

async fn ledger_entries(
    pool: &SqlitePool,
    snapshot_id: &str,
) -> Vec<manual_core::domain::CostLedgerEntry> {
    let mut conn = pool.acquire().await.expect("连接");
    ledger_repo::list_for_snapshot(&mut conn, snapshot_id)
        .await
        .expect("读取账本")
}

async fn attempt_count_for_stage(pool: &SqlitePool, stage_id: &str) -> i64 {
    scalar_i64(
        pool,
        "SELECT COUNT(*) FROM provider_attempts WHERE stage_id = ?",
        Some(stage_id),
    )
    .await
}

async fn latest_attempt(
    pool: &SqlitePool,
    stage_id: &str,
) -> Option<manual_core::domain::ProviderAttempt> {
    let mut conn = pool.acquire().await.expect("连接");
    attempts_repo::latest_for_stage(&mut conn, stage_id)
        .await
        .expect("读取 attempt")
}

/// 还原"结果事实已持久化、checkpoint 未推进"的崩溃现场（组装阶段：草稿行即产物）。
async fn reopen_stage_without_checkpoint(pool: &SqlitePool, stage_id: &str) {
    sqlx::query(
        "UPDATE job_stages SET status = 'running', lease_until = 1, \
            result_asset_id = NULL, usage_json = NULL WHERE id = ?",
    )
    .bind(stage_id)
    .execute(pool)
    .await
    .expect("还原崩溃现场（无 checkpoint）");
}

/// 还原"结果事实已落库（含 usage_json 的 producedKnowledge）、checkpoint 未推进"的现场。
async fn reopen_stage_with_result_fact(pool: &SqlitePool, stage_id: &str) {
    sqlx::query("UPDATE job_stages SET status = 'running', lease_until = 1 WHERE id = ?")
        .bind(stage_id)
        .execute(pool)
        .await
        .expect("还原崩溃现场（结果已落库）");
}

async fn drive_until_job(
    pool: &SqlitePool,
    executor: &Arc<JobExecutor>,
    clock: &ManualClock,
    job_id: &str,
    want: JobStatus,
    max_ticks: usize,
) -> Job {
    for _ in 0..max_ticks {
        let job = job_row(pool, job_id).await;
        if job.status == want {
            return job;
        }
        tick(executor, clock).await;
    }
    let job = job_row(pool, job_id).await;
    let stages = stages_of(pool, job_id).await;
    panic!(
        "job 未在 {max_ticks} tick 内达到 {}（实际 {}；阶段：{:?}）",
        want.as_str(),
        job.status.as_str(),
        stages
            .iter()
            .map(|stage| format!(
                "{}({})={}",
                stage.stage_kind.as_str(),
                stage.batch_index,
                stage.status.as_str()
            ))
            .collect::<Vec<_>>()
    );
}

async fn tick_until_stage(
    pool: &SqlitePool,
    executor: &Arc<JobExecutor>,
    clock: &ManualClock,
    job_id: &str,
    kind: StageKind,
    want: JobStatus,
    max_ticks: usize,
) -> JobStage {
    for _ in 0..max_ticks {
        let stage = stage_of(pool, job_id, kind).await;
        if stage.status == want {
            return stage;
        }
        tick(executor, clock).await;
    }
    let stage = stage_of(pool, job_id, kind).await;
    panic!(
        "阶段 {} 未在 {max_ticks} tick 内达到 {}（实际 {}；last_error={:?}）",
        kind.as_str(),
        want.as_str(),
        stage.status.as_str(),
        stage.last_error
    );
}

async fn job_detail(app: &TestApp, cookie: &str, job_id: &str) -> (Value, String) {
    let response = app
        .call(Method::GET, &format!("/api/v1/jobs/{job_id}"))
        .cookie(cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let etag = response
        .header("etag")
        .expect("任务详情必须带 ETag")
        .to_owned();
    (response.json()["data"].clone(), etag)
}

fn stage_json<'a>(detail: &'a Value, kind: &str, batch_index: i64) -> &'a Value {
    detail["stages"]
        .as_array()
        .expect("stages 数组")
        .iter()
        .find(|stage| {
            stage["stageKind"] == kind && stage["batchIndex"].as_i64() == Some(batch_index)
        })
        .unwrap_or_else(|| panic!("详情里找不到阶段 {kind}/{batch_index}：{detail}"))
}

async fn asset_bytes(app: &TestApp, cookie: &str, asset_id: &str) -> Vec<u8> {
    let response = app
        .call(Method::GET, &format!("/api/v1/assets/{asset_id}/content"))
        .cookie(cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.body
}

async fn get_draft(app: &TestApp, cookie: &str, item: &str, draft_id: &str) -> (Value, String) {
    let response = app
        .call(
            Method::GET,
            &format!("/api/v1/items/{item}/drafts/{draft_id}"),
        )
        .cookie(cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let etag = response
        .header("etag")
        .expect("草稿读取必须带 ETag")
        .to_owned();
    (response.json()["data"].clone(), etag)
}

async fn retry_stage(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    job_id: &str,
    stage_id: &str,
    etag: &str,
    key: &str,
) -> common::TestResponse {
    app.call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(cookie)
        .csrf(csrf)
        .header("if-match", etag)
        .header("idempotency-key", key)
        .json(&json!({ "stageId": stage_id }))
        .send()
        .await
}

fn error_field(response: &common::TestResponse, path: &str) -> Value {
    response.json()["error"][path].clone()
}

#[allow(clippy::too_many_arguments)]
async fn patch_draft(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    draft_id: &str,
    if_match: Option<&str>,
    body: Value,
) -> common::TestResponse {
    let mut builder = app
        .call(
            Method::PATCH,
            &format!("/api/v1/items/{item}/drafts/{draft_id}"),
        )
        .cookie(cookie)
        .csrf(csrf);
    if let Some(value) = if_match {
        builder = builder.header("if-match", value);
    }
    builder.json(&body).send().await
}

// ---------------------------------------------------------------------------
// 用例 1：全链路（AC-047 / AC-048 主路径）
// ---------------------------------------------------------------------------

/// 全链路脚手架：3 页文字 + front/left 照片；知识分支成功、模型分支成功。
struct FullChain {
    app: TestApp,
    fixture: QaHttp,
    cookie: String,
    csrf: String,
    job_id: String,
    inputs_item: String,
    clock: Arc<ManualClock>,
    executor: Arc<JobExecutor>,
}

async fn run_full_chain(tag: &str) -> FullChain {
    let full = fixture_bytes("sample-model.glb");
    let fixture = QaHttp::start(vec![
        route(
            "POST",
            TRIPO_UPLOAD_PATH,
            MatchKind::Exact,
            vec![tripo_upload_script("qa-token-0001")],
        ),
        route(
            "POST",
            TRIPO_SUBMIT_PATH,
            MatchKind::Exact,
            vec![tripo_submit_script("qa-task-0001")],
        ),
        route(
            "GET",
            TRIPO_TASKS_PREFIX,
            MatchKind::Prefix,
            vec![
                tripo_running_script("qa-task-0001"),
                tripo_success_script("qa-task-0001", ""),
            ],
        ),
        route(
            "POST",
            MANUAL_PATH,
            MatchKind::Exact,
            vec![QaScript::bytes(200, "application/json", Vec::new())],
        ),
    ]);
    // 模型 URL 依赖 fixture 端口：先启动拿到端口，再替换 tasks 脚本里的 URL。
    let model_url = fixture.cdn_url();
    fixture.state.routes.lock().unwrap()[2].steps[1] =
        tripo_success_script("qa-task-0001", &model_url);
    fixture.state.routes.lock().unwrap()[3].steps[0] = QaScript::bytes(
        200,
        "application/json",
        manual_success_for("qa-resp-1", &[1, 2, 3]),
    );
    fixture.state.routes.lock().unwrap()[3].body_marker = Some(page_marker(1));
    // CDN 路由（下载路径）。
    fixture.state.routes.lock().unwrap().push(cdn_route(full));

    let (app, cookie, csrf) = qa_app(tag, &fixture).await;
    let inputs = build_inputs(&app, &cookie, &csrf, &marked_pages(3)).await;
    let item = inputs.item.clone();
    let job_id = create_job(&app, &cookie, &csrf, &inputs, &format!("{tag}-key")).await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock));

    FullChain {
        app,
        fixture,
        cookie,
        csrf,
        job_id,
        inputs_item: item,
        clock,
        executor,
    }
}

#[tokio::test]
async fn qa_t15_full_chain_assembles_needs_review_draft_and_never_publishes() {
    let chain = run_full_chain("qa-t15-full").await;
    let pool = pool(&chain.app);
    let job = drive_until_job(
        &pool,
        &chain.executor,
        &chain.clock,
        &chain.job_id,
        JobStatus::Succeeded,
        60,
    )
    .await;
    assert_eq!(job.status, JobStatus::Succeeded, "fixture 全链路必须成功");

    // 组装阶段成功且 usage 记录了草稿事实。
    let assemble = stage_of(&pool, &chain.job_id, StageKind::AssembleDraft).await;
    assert_eq!(assemble.status, JobStatus::Succeeded);
    let usage = assemble.usage_json.expect("组装阶段 usage");
    assert_eq!(usage["completeness"], "complete");
    assert!(usage["draftId"].as_str().is_some());

    // job succeeded ≠ published：releases 为空；publish 是 T19 的显式动作，
    // 缺 If-Match 的请求被 428 拒绝且不产生 release（事实更新 2026-09-12，
    // 原断言"路由不存在（404）"在 T19 交付后不成立；守卫不弱化）。
    assert_eq!(release_count(&pool).await, 0, "不存在自动发布路径");
    let (detail, _etag) = job_detail(&chain.app, &chain.cookie, &chain.job_id).await;
    let draft_id = detail["draftId"]
        .as_str()
        .expect("任务详情给出草稿 id")
        .to_owned();
    let publish = chain
        .app
        .call(
            Method::POST,
            &format!(
                "/api/v1/items/{}/drafts/{draft_id}/publish",
                chain.inputs_item
            ),
        )
        .cookie(&chain.cookie)
        .csrf(&chain.csrf)
        .send()
        .await;
    assert_eq!(
        publish.status,
        StatusCode::PRECONDITION_REQUIRED,
        "publish 需显式 If-Match（缺 428）；实际 {}",
        publish.text()
    );
    assert_eq!(
        release_count(&pool).await,
        0,
        "被拒绝的 publish 不得产生 release"
    );

    // 草稿读取：ETag + needs_review + complete + 模型引用 + notices。
    let (draft, etag) = get_draft(&chain.app, &chain.cookie, &chain.inputs_item, &draft_id).await;
    assert_eq!(etag, "\"r1\"");
    assert_eq!(draft["status"], "needs_review");
    assert_eq!(draft["completeness"], "complete");
    assert!(draft["missing"].as_array().expect("missing").is_empty());
    assert!(draft["modelRevisionId"].as_str().is_some());
    assert_eq!(draft["knowledge"]["schemaVersion"], "manual_draft_v1");
    assert_eq!(draft["knowledge"]["completeness"], "complete");
    let parts = draft["knowledge"]["knowledge"]["parts"]
        .as_array()
        .expect("合并知识里的部件");
    assert_eq!(parts.len(), 3, "3 页各一个部件");
    let notices = draft["notices"].as_array().expect("notices");
    assert!(
        notices
            .iter()
            .any(|note| note.as_str().unwrap_or_default().contains("不等于已发布")),
        "必须明确提示生成完成不等于已发布：{notices:?}"
    );
    assert!(
        notices.iter().any(|note| note
            .as_str()
            .unwrap_or_default()
            .contains("不存在自动发布路径")),
        "必须明确不存在自动发布路径：{notices:?}"
    );
    assert!(draft["review"].is_null(), "T15 不写 modelReview（属 T19）");

    // 同一快照只有一份草稿。
    assert_eq!(draft_count(&pool, &chain.inputs_item).await, 1);

    // 列表行：draftId 可见、阶段计数不是百分比。
    let list = chain
        .app
        .call(
            Method::GET,
            &format!("/api/v1/jobs?itemId={}", chain.inputs_item),
        )
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    let rows = list.json()["data"].as_array().expect("列表数组").clone();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], chain.job_id.as_str());
    assert_eq!(rows[0]["draftId"], draft_id.as_str());
    assert_eq!(rows[0]["status"], "succeeded");
    let summary = &rows[0]["stageSummary"];
    assert_eq!(summary["succeeded"], summary["total"]);
    assert_eq!(summary["failed"], 0);
    assert_eq!(summary["unknown"], 0);
    assert!(rows[0]["stageSummary"]["total"].as_i64().unwrap_or(0) > 0);

    // fixture：每个付费动作恰好一次，且没有非预期请求。
    assert_eq!(chain.fixture.paid_submits(), 1, "付费提交恰 1 次");
    assert_eq!(chain.fixture.manual_requests(), 1, "说明书批次恰 1 次");
    assert_eq!(chain.fixture.downloads(), 1, "模型下载恰 1 次");
    assert!(
        chain.fixture.unexpected().is_empty(),
        "非预期请求：{:?}",
        chain.fixture.unexpected()
    );

    // 草稿已产出但发布入口不存在；服务端没有别的自动发布触发点（无后台 publish）。
    assert_eq!(audit_count(&pool, "draft_assembled").await, 1);
    chain.fixture.stop.store(true, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// 用例 2：重启/重放不重复创建草稿（AC-047 卡内项）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_t15_crash_replay_does_not_create_a_second_draft() {
    let chain = run_full_chain("qa-t15-replay").await;
    let pool = pool(&chain.app);
    drive_until_job(
        &pool,
        &chain.executor,
        &chain.clock,
        &chain.job_id,
        JobStatus::Succeeded,
        60,
    )
    .await;
    let snapshot_id = job_row(&pool, &chain.job_id).await.snapshot_id;
    let before = draft_for_snapshot(&pool, &snapshot_id)
        .await
        .expect("全链路后草稿存在");
    let requests_before = chain.fixture.total_requests();

    // 崩溃现场：草稿已写、组装 checkpoint 未推进（过程重启）。
    let assemble = stage_of(&pool, &chain.job_id, StageKind::AssembleDraft).await;
    reopen_stage_without_checkpoint(&pool, &assemble.id).await;

    // 恢复 + 重跑（同一执行器与时钟推进 = 服务重启后的恢复扫描 + 重新领取）。
    drive_until_job(
        &pool,
        &chain.executor,
        &chain.clock,
        &chain.job_id,
        JobStatus::Succeeded,
        30,
    )
    .await;

    let after = draft_for_snapshot(&pool, &snapshot_id)
        .await
        .expect("恢复后草稿仍在");
    assert_eq!(after.id, before.id, "重启不得新建第二份草稿");
    assert_eq!(after.revision, before.revision, "内容未变 → revision 不变");
    assert_eq!(after.knowledge_json, before.knowledge_json);
    assert_eq!(draft_count(&pool, &chain.inputs_item).await, 1);
    assert_eq!(release_count(&pool).await, 0);
    assert_eq!(
        chain.fixture.total_requests(),
        requests_before,
        "重放组装不得产生新的供应商请求（零外呼）"
    );
    assert_eq!(
        audit_count(&pool, "draft_assembled").await,
        1,
        "内容相同不写库 → 不追加审计"
    );
}

// ---------------------------------------------------------------------------
// 用例 3：部分成功草稿（模型分支被阻塞）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_t15_blocked_model_branch_yields_partial_draft_and_gates_retry() {
    let full = fixture_bytes("sample-model.glb");
    let truncated = full[..full.len() / 3].to_vec();
    let fixture = QaHttp::start(vec![
        route(
            "POST",
            TRIPO_UPLOAD_PATH,
            MatchKind::Exact,
            vec![tripo_upload_script("qa-token-partial")],
        ),
        route(
            "POST",
            TRIPO_SUBMIT_PATH,
            MatchKind::Exact,
            vec![tripo_submit_script("qa-task-partial")],
        ),
        route(
            "GET",
            TRIPO_TASKS_PREFIX,
            MatchKind::Prefix,
            vec![QaScript::json(
                200,
                json!({ "code": 0, "data": { "task_id": "qa-task-partial", "status": "running", "progress": 10 } }),
            )],
        ),
        route_marked(
            "POST",
            MANUAL_PATH,
            &page_marker(1),
            vec![QaScript::bytes(
                200,
                "application/json",
                manual_success_for("qa-resp-partial", &[1, 2, 3]),
            )],
        ),
        route(
            "GET",
            CDN_PREFIX,
            MatchKind::Prefix,
            vec![QaScript::bytes(200, "model/gltf-binary", truncated)],
        ),
    ]);
    // 远端任务在轮询第 2 次变成功（模型 URL 指向截断 GLB 的 CDN）。
    let model_url = fixture.cdn_url();
    {
        let mut routes = fixture.state.routes.lock().unwrap();
        routes[2].steps = vec![
            tripo_running_script("qa-task-partial"),
            tripo_success_script("qa-task-partial", &model_url),
        ];
    }

    let (app, cookie, csrf) = qa_app("qa-t15-partial", &fixture).await;
    let inputs = build_inputs(&app, &cookie, &csrf, &marked_pages(3)).await;
    let item = inputs.item.clone();
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "qa-t15-partial-key").await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    // 模型分支头 model_validate 进入 needs_input（GLB 截断 → 不静默改坏）。
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualMerge,
        JobStatus::Succeeded,
        60,
    )
    .await;
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::NeedsInput,
        60,
    )
    .await;

    // 分支头已定性 → 组装产出**部分草稿**并标明缺项。
    let assemble = tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::AssembleDraft,
        JobStatus::Succeeded,
        60,
    )
    .await;
    let usage = assemble.usage_json.expect("usage");
    assert_eq!(usage["completeness"], "partial");
    assert_eq!(usage["missingCodes"][0], "model_branch_incomplete");

    let (detail, _etag) = job_detail(&app, &cookie, &job_id).await;
    assert_eq!(
        detail["status"], "needs_input",
        "部分组装不得把父 job 冒充成 succeeded"
    );
    let draft_id = detail["draftId"].as_str().expect("部分草稿 id").to_owned();
    let (draft, _) = get_draft(&app, &cookie, &item, &draft_id).await;
    assert_eq!(draft["status"], "needs_review");
    assert_eq!(draft["completeness"], "partial");
    assert!(draft["modelRevisionId"].is_null(), "模型分支未完成");
    let missing = draft["missing"].as_array().expect("missing");
    assert_eq!(missing[0]["code"], "model_branch_incomplete");
    assert!(
        draft["knowledge"]["knowledge"]["parts"]
            .as_array()
            .expect("知识部件")
            .len()
            == 3,
        "知识分支产物必须保留"
    );
    assert_eq!(release_count(&pool).await, 0);

    // 观察（非阻断，记录到 qa-report）：模型分支头的重试受"预留必须仍占预算"约束。
    let validate = stage_of(&pool, &job_id, StageKind::ModelValidate).await;
    let (_, job_etag) = job_detail(&app, &cookie, &job_id).await;
    let retry = retry_stage(
        &app,
        &cookie,
        &csrf,
        &job_id,
        &validate.id,
        &job_etag,
        "qa-t15-partial-retry",
    )
    .await;
    assert_eq!(
        retry.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "观察：预留已结算（poll 成功即结算）时模型分支重试被拒；实际 {}",
        retry.text()
    );
    assert_eq!(
        error_field(&retry, "details")["reason"],
        "budgetNotHolding",
        "{}",
        retry.text()
    );
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::ModelValidate)
            .await
            .status,
        JobStatus::NeedsInput,
        "拒绝必须无副作用"
    );
    assert_eq!(fixture.paid_submits(), 1, "拒绝不得产生新的付费提交");
    // QA 观察（非阻断，供 qa-report P3 记录）：模型分支头被阻塞时的重试门槛。
    eprintln!(
        "QA-OBSERVE[model-head-retry] status={} reason={} ledger={:?} model_validate={} assemble={} draft_completeness={}",
        retry.status.as_u16(),
        error_field(&retry, "details")["reason"],
        ledger_entries(&pool, &job_row(&pool, &job_id).await.snapshot_id)
            .await
            .iter()
            .map(|entry| (entry.provider.as_str(), entry.state.as_str()))
            .collect::<Vec<_>>(),
        stage_of(&pool, &job_id, StageKind::ModelValidate)
            .await
            .status
            .as_str(),
        stage_of(&pool, &job_id, StageKind::AssembleDraft)
            .await
            .status
            .as_str(),
        usage["completeness"],
    );
    assert!(
        fixture.unexpected().is_empty(),
        "{:?}",
        fixture.unexpected()
    );
}

// ---------------------------------------------------------------------------
// 用例 4：上游批次 unknown → 不组装；对账后可补齐（assemble 领取放宽的语义边界）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_t15_upstream_unknown_blocks_assembly_until_reconciled() {
    let full = fixture_bytes("sample-model.glb");
    let truncated_manual = manual_success_for("qa-resp-unknown", &[1, 2, 3]);
    let fixture = QaHttp::start(vec![
        route(
            "POST",
            TRIPO_UPLOAD_PATH,
            MatchKind::Exact,
            vec![tripo_upload_script("qa-token-unknown")],
        ),
        route(
            "POST",
            TRIPO_SUBMIT_PATH,
            MatchKind::Exact,
            vec![tripo_submit_script("qa-task-unknown")],
        ),
        route(
            "GET",
            TRIPO_TASKS_PREFIX,
            MatchKind::Prefix,
            vec![
                tripo_running_script("qa-task-unknown"),
                tripo_success_script("qa-task-unknown", ""),
            ],
        ),
        route_marked(
            "POST",
            MANUAL_PATH,
            &page_marker(1),
            vec![
                // 第一次：声明完整长度但只写一半 → 该批 submission_unknown。
                QaScript::truncated(truncated_manual.clone(), truncated_manual.len() / 2),
                // 对账后重算：完整成功。
                QaScript::bytes(
                    200,
                    "application/json",
                    manual_success_for("qa-resp-unknown-2", &[1, 2, 3]),
                ),
            ],
        ),
    ]);
    let model_url = fixture.cdn_url();
    {
        let mut routes = fixture.state.routes.lock().unwrap();
        routes[2].steps = vec![
            tripo_running_script("qa-task-unknown"),
            tripo_success_script("qa-task-unknown", &model_url),
        ];
        routes.push(cdn_route(full));
    }

    let (app, cookie, csrf) = qa_app("qa-t15-unknown", &fixture).await;
    let inputs = build_inputs(&app, &cookie, &csrf, &marked_pages(3)).await;
    let item = inputs.item.clone();
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "qa-t15-unknown-key").await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    // 批次发出同步请求但完整响应未持久化 → submission_unknown；模型分支不受影响。
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualExtract,
        JobStatus::SubmissionUnknown,
        60,
    )
    .await;
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::Succeeded,
        80,
    )
    .await;

    // 关键语义核对：上游批次阻塞（分支头 merge 仍 queued）→ 组装保持 queued、无草稿。
    for _ in 0..6 {
        tick(&executor, &clock).await;
    }
    let merge = stage_of(&pool, &job_id, StageKind::ManualMerge).await;
    assert_eq!(
        merge.status,
        JobStatus::Queued,
        "分支头未定性 → 组装不得提前产出"
    );
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::AssembleDraft)
            .await
            .status,
        JobStatus::Queued
    );
    assert_eq!(draft_count(&pool, &item).await, 0, "不得产出误导性草稿");
    let (detail, etag) = job_detail(&app, &cookie, &job_id).await;
    assert_eq!(detail["status"], "submission_unknown");
    eprintln!(
        "QA-OBSERVE[upstream-unknown] job={} merge={} assemble={} drafts={} manual_requests={}",
        detail["status"],
        merge.status.as_str(),
        stage_of(&pool, &job_id, StageKind::AssembleDraft)
            .await
            .status
            .as_str(),
        draft_count(&pool, &item).await,
        fixture.manual_requests(),
    );
    let batch = stage_json(&detail, "manual_extract", 0);
    assert_eq!(batch["status"], "submission_unknown");
    assert!(
        detail["draftId"].is_null(),
        "unknown 期间任务详情不得给出草稿 id"
    );

    // 未知不得盲重试（无副作用）；同步链路不提供 attach。
    let batch_id = batch["id"].as_str().unwrap().to_owned();
    let attempt_rows = attempt_count_for_stage(&pool, &batch_id).await;
    let retry = retry_stage(
        &app,
        &cookie,
        &csrf,
        &job_id,
        &batch_id,
        &etag,
        "qa-t15-unknown-retry",
    )
    .await;
    assert_eq!(
        retry.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        retry.text()
    );
    assert_eq!(
        error_field(&retry, "details")["reason"],
        "stageNotRetryable"
    );
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::ManualExtract)
            .await
            .status,
        JobStatus::SubmissionUnknown
    );
    assert_eq!(
        attempt_count_for_stage(&pool, &batch_id).await,
        attempt_rows
    );
    assert_eq!(fixture.manual_requests(), 1, "拒绝不得产生第二次请求");

    let attach = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "attachRemoteTask",
            "stageId": batch_id,
            "remoteTaskId": "qa-ghost-0001",
            "acknowledgeMatches": true,
        }))
        .send()
        .await;
    assert_eq!(
        attach.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "同步 Manual AI 不得提供 attachRemoteTask：{}",
        attach.text()
    );
    assert_eq!(
        error_field(&attach, "details")["reason"],
        "attachRemoteTaskUnsupported"
    );
    assert_eq!(fixture.manual_requests(), 1, "被拒动作必须无副作用");

    // authorizeReplacement：缺 ack / 预算不足均拒绝；正例需要完整预算确认。
    let no_ack = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "authorizeReplacement",
            "stageId": batch_id,
            "limits": { "manualAiUsdMicros": 9_999_999 },
        }))
        .send()
        .await;
    assert_eq!(
        no_ack.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        no_ack.text()
    );
    let low_budget = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "authorizeReplacement",
            "stageId": batch_id,
            "acknowledgeDuplicateRisk": true,
            "limits": { "manualAiUsdMicros": 1 },
        }))
        .send()
        .await;
    assert_eq!(
        low_budget.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        low_budget.text()
    );
    assert_eq!(
        error_field(&low_budget, "details")["reason"],
        "budgetBelowPlannedUpperBound"
    );
    assert_eq!(fixture.manual_requests(), 1, "预算不足不得发起请求");

    let replacement = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "authorizeReplacement",
            "stageId": batch_id,
            "acknowledgeDuplicateRisk": true,
            "limits": { "manualAiUsdMicros": 9_999_999 },
        }))
        .send()
        .await;
    assert_eq!(replacement.status, StatusCode::OK, "{}", replacement.text());
    let body = replacement.json();
    assert_eq!(body["data"]["stageStatus"], "queued");
    assert!(
        body["data"]["notice"]
            .as_str()
            .unwrap()
            .contains("重复收费")
    );

    // 旧 attempt 定性为 failed（旧未决账务保留）；预留未释放。
    let snapshot_id = job_row(&pool, &job_id).await.snapshot_id;
    let ledger = ledger_entries(&pool, &snapshot_id).await;
    let manual_entry = ledger
        .iter()
        .find(|entry| entry.provider == manual_core::domain::ProviderKey::ManualAi)
        .expect("说明书 AI 预留");
    assert!(
        manual_entry.state == LedgerState::Reserved || manual_entry.state == LedgerState::Unknown,
        "unknown 预留不自动释放：{:?}",
        manual_entry.state
    );
    assert!(manual_entry.actual.is_none(), "unknown 不得把实际费用填 0");
    assert_eq!(
        audit_count(&pool, "job_reconcile_authorize_replacement").await,
        1
    );

    // 替代提交后：知识分支补齐 → 组装产出完整草稿；模型分支不被重新购买。
    let paid_before = fixture.paid_submits();
    let job = drive_until_job(&pool, &executor, &clock, &job_id, JobStatus::Succeeded, 80).await;
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(fixture.paid_submits(), paid_before, "替代提交不得重购模型");
    assert_eq!(fixture.manual_requests(), 2, "只对该批重新授权一次请求");
    let snapshot_id = job.snapshot_id.clone();
    let draft = draft_for_snapshot(&pool, &snapshot_id)
        .await
        .expect("对账后草稿");
    assert_eq!(draft.knowledge_json["completeness"], "complete");
    assert_eq!(release_count(&pool).await, 0);
    assert!(
        fixture.unexpected().is_empty(),
        "{:?}",
        fixture.unexpected()
    );
}

// ---------------------------------------------------------------------------
// 用例 5：Tripo unknown → attachRemoteTask 正负例（AC-037）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_t15_attach_remote_task_verifies_queries_and_does_not_repurchase() {
    let full = fixture_bytes("sample-model.glb");
    let truncated_submit =
        json!({ "code": 0, "data": { "task_id": "qa-task-attach-0001" } }).to_string();
    let fixture = QaHttp::start(vec![
        // 精确路由必须排在通用前缀之前：幽灵任务 → 404（查询验证失败）。
        route(
            "GET",
            "/v3/tasks/qa-ghost-task-9999",
            MatchKind::Exact,
            vec![QaScript::json(
                404,
                json!({ "code": 404, "message": "task not found" }),
            )],
        ),
        // 账户内真实存在的任务 → 可访问（管理员附加）。
        route(
            "GET",
            "/v3/tasks/qa-attached-task-0001",
            MatchKind::Exact,
            vec![QaScript::json(
                200,
                json!({ "code": 0, "data": { "task_id": "qa-attached-task-0001", "status": "running", "progress": 5 } }),
            )],
        ),
        route(
            "POST",
            TRIPO_UPLOAD_PATH,
            MatchKind::Exact,
            vec![tripo_upload_script("qa-token-attach")],
        ),
        route(
            "POST",
            TRIPO_SUBMIT_PATH,
            MatchKind::Exact,
            vec![
                // 第一次提交：响应被截断 → 没有 task ID → submission_unknown。
                QaScript::truncated(
                    truncated_submit.clone().into_bytes(),
                    truncated_submit.len() / 2,
                ),
                // 后续（不应发生）：若被重购则此处成功，作为"重购"信号。
                tripo_submit_script("qa-task-attach-repurchase"),
            ],
        ),
        route(
            "GET",
            TRIPO_TASKS_PREFIX,
            MatchKind::Prefix,
            vec![
                tripo_running_script("qa-attached-task-0001"),
                tripo_success_script("qa-attached-task-0001", ""),
            ],
        ),
        route_marked(
            "POST",
            MANUAL_PATH,
            &page_marker(1),
            vec![QaScript::bytes(
                200,
                "application/json",
                manual_success_for("qa-resp-attach", &[1, 2, 3]),
            )],
        ),
    ]);
    let model_url = fixture.cdn_url();
    {
        let mut routes = fixture.state.routes.lock().unwrap();
        // 精确任务路由：第 1 次是 attach 的查询验证（running），第 2 次起是轮询（success）。
        routes[1].steps = vec![
            tripo_running_script("qa-attached-task-0001"),
            tripo_success_script("qa-attached-task-0001", &model_url),
        ];
        routes[4].steps = routes[1].steps.clone();
        routes.push(cdn_route(full));
    }

    let (app, cookie, csrf) = qa_app("qa-t15-attach", &fixture).await;
    let inputs = build_inputs(&app, &cookie, &csrf, &marked_pages(3)).await;
    let item = inputs.item.clone();
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "qa-t15-attach-key").await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    let submit = tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoSubmit,
        JobStatus::SubmissionUnknown,
        60,
    )
    .await;
    // 知识分支独立完成。
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualMerge,
        JobStatus::Succeeded,
        60,
    )
    .await;
    assert_eq!(fixture.paid_submits(), 1, "付费 POST 恰 1 次");
    let submits_before = fixture.paid_submits();

    // 关键语义核对（模型侧）：远端提交未定性 → 模型分支头未定性 → 组装不得提前产出。
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::ModelValidate)
            .await
            .status,
        JobStatus::Queued
    );
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::AssembleDraft)
            .await
            .status,
        JobStatus::Queued
    );
    assert_eq!(
        draft_count(&pool, &item).await,
        0,
        "unknown 期间不得产出利用不完整输入的草稿"
    );

    // 缺 CSRF → 403（对账是修改请求，受会话 + CSRF 保护）。
    let (_, csrf_missing_etag) = job_detail(&app, &cookie, &job_id).await;
    let no_csrf = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .header("if-match", &csrf_missing_etag)
        .json(&json!({
            "action": "attachRemoteTask",
            "stageId": submit.id,
            "remoteTaskId": "qa-attached-task-0001",
            "acknowledgeMatches": true,
        }))
        .send()
        .await;
    assert_eq!(no_csrf.status, StatusCode::FORBIDDEN, "{}", no_csrf.text());

    // 未知不得从此盲重试（模型分支侧）：被拒且无副作用。
    let (_, retry_etag) = job_detail(&app, &cookie, &job_id).await;
    let blind_retry = retry_stage(
        &app,
        &cookie,
        &csrf,
        &job_id,
        &submit.id,
        &retry_etag,
        "qa-t15-attach-blind-retry",
    )
    .await;
    assert_eq!(
        blind_retry.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        blind_retry.text()
    );
    assert_eq!(
        error_field(&blind_retry, "details")["reason"],
        "stageNotRetryable"
    );
    assert_eq!(
        fixture.paid_submits(),
        submits_before,
        "盲重试不得产生第二次付费提交"
    );

    // 未登录 → 401（仅管理员）。
    let anonymous = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .json(&json!({
            "action": "attachRemoteTask",
            "stageId": submit.id,
            "remoteTaskId": "qa-attached-task-0001",
            "acknowledgeMatches": true,
        }))
        .send()
        .await;
    assert_eq!(
        anonymous.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        anonymous.text()
    );

    let (_, etag) = job_detail(&app, &cookie, &job_id).await;

    // 缺 remoteTaskId / 缺二次确认 → 422 字段级。
    let missing_id = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({ "action": "attachRemoteTask", "stageId": submit.id, "acknowledgeMatches": true }))
        .send()
        .await;
    assert_eq!(
        missing_id.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        missing_id.text()
    );
    let missing_ack = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({ "action": "attachRemoteTask", "stageId": submit.id, "remoteTaskId": "qa-attached-task-0001" }))
        .send()
        .await;
    assert_eq!(
        missing_ack.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        missing_ack.text()
    );

    // 查询验证失败（幽灵任务）→ 422 且无副作用。
    let ghost = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "attachRemoteTask",
            "stageId": submit.id,
            "remoteTaskId": "qa-ghost-task-9999",
            "acknowledgeMatches": true,
        }))
        .send()
        .await;
    assert_eq!(
        ghost.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        ghost.text()
    );
    assert_eq!(
        error_field(&ghost, "details")["reason"],
        "remoteTaskVerificationFailed"
    );
    let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
    assert!(attempt.remote_task_id.is_none(), "验证失败不得写入远端 ID");
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::TripoSubmit)
            .await
            .status,
        JobStatus::SubmissionUnknown
    );
    assert_eq!(fixture.paid_submits(), submits_before);
    assert_eq!(
        audit_count(&pool, "job_reconcile_attach_remote_task").await,
        0
    );

    // 正例：账户内可访问的任务 → 阶段回 queued，按已知 ID 继续查询（不重新购买）。
    let attached = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "attachRemoteTask",
            "stageId": submit.id,
            "remoteTaskId": "qa-attached-task-0001",
            "acknowledgeMatches": true,
        }))
        .send()
        .await;
    assert_eq!(attached.status, StatusCode::OK, "{}", attached.text());
    assert_eq!(attached.json()["data"]["stageStatus"], "queued");
    let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
    assert_eq!(
        attempt.remote_task_id.as_deref(),
        Some("qa-attached-task-0001")
    );
    assert_eq!(
        audit_count(&pool, "job_reconcile_attach_remote_task").await,
        1
    );
    let metadata = audit_metadata(&pool, "job_reconcile_attach_remote_task").await;
    assert!(
        metadata.contains("\"acknowledgedMatches\":true"),
        "{metadata}"
    );
    // 附加前必须用当前配置的 Tripo 凭据真实查询一次（查询验证：类型与账号可访问性）。
    let verification = fixture.requests_to("GET", "/v3/tasks/qa-attached-task-0001");
    assert_eq!(verification.len(), 1, "attach 必须查询验证一次");
    assert_eq!(
        verification[0]
            .headers
            .get("authorization")
            .map(|value| value.starts_with("Bearer ")),
        Some(true),
        "查询必须带当前配置的凭据（不猜测远端状态）：{:?}",
        verification[0].headers.keys().collect::<Vec<_>>()
    );

    // 继续推进：不重新购买、最终产出完整草稿。
    let job = drive_until_job(&pool, &executor, &clock, &job_id, JobStatus::Succeeded, 80).await;
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(
        fixture.paid_submits(),
        submits_before,
        "附加远端任务后必须沿用已提交任务（不得第二次付费提交）"
    );
    let draft = draft_for_snapshot(&pool, &job.snapshot_id)
        .await
        .expect("草稿");
    assert_eq!(draft.knowledge_json["completeness"], "complete");
    assert_eq!(release_count(&pool).await, 0);
    assert!(
        fixture.unexpected().is_empty(),
        "{:?}",
        fixture.unexpected()
    );
}

// ---------------------------------------------------------------------------
// 用例 6：recordNoTask 正负例（AC-037）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_t15_record_no_task_requires_evidence_and_keeps_reservation() {
    let full = fixture_bytes("sample-model.glb");
    let truncated_submit =
        json!({ "code": 0, "data": { "task_id": "qa-task-notask-0001" } }).to_string();
    let fixture = QaHttp::start(vec![
        route(
            "POST",
            TRIPO_UPLOAD_PATH,
            MatchKind::Exact,
            vec![tripo_upload_script("qa-token-notask")],
        ),
        route(
            "POST",
            TRIPO_SUBMIT_PATH,
            MatchKind::Exact,
            vec![
                QaScript::truncated(
                    truncated_submit.clone().into_bytes(),
                    truncated_submit.len() / 2,
                ),
                tripo_submit_script("qa-task-notask-0001"),
            ],
        ),
        route(
            "GET",
            TRIPO_TASKS_PREFIX,
            MatchKind::Prefix,
            vec![
                tripo_running_script("qa-task-notask-0001"),
                tripo_success_script("qa-task-notask-0001", ""),
            ],
        ),
        route_marked(
            "POST",
            MANUAL_PATH,
            &page_marker(1),
            vec![QaScript::bytes(
                200,
                "application/json",
                manual_success_for("qa-resp-notask", &[1, 2, 3]),
            )],
        ),
    ]);
    let model_url = fixture.cdn_url();
    {
        let mut routes = fixture.state.routes.lock().unwrap();
        routes[2].steps = vec![
            tripo_running_script("qa-task-notask-0001"),
            tripo_success_script("qa-task-notask-0001", &model_url),
        ];
        routes.push(cdn_route(full));
    }

    let (app, cookie, csrf) = qa_app("qa-t15-notask", &fixture).await;
    let inputs = build_inputs(&app, &cookie, &csrf, &marked_pages(3)).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "qa-t15-notask-key").await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    let submit = tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoSubmit,
        JobStatus::SubmissionUnknown,
        60,
    )
    .await;
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualMerge,
        JobStatus::Succeeded,
        60,
    )
    .await;

    // 缺证据 → 422 字段级；无副作用。
    let (_, etag) = job_detail(&app, &cookie, &job_id).await;
    let no_evidence = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({ "action": "recordNoTask", "stageId": submit.id }))
        .send()
        .await;
    assert_eq!(
        no_evidence.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        no_evidence.text()
    );
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::TripoSubmit)
            .await
            .status,
        JobStatus::SubmissionUnknown
    );

    // 正例：记录核查证据（管理员声明）→ attempt failed、阶段 needs_input、预留不释放。
    let recorded = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "recordNoTask",
            "stageId": submit.id,
            "evidence": "QA 现场核查：账户任务列表（2026-09-12 12:00）无该次提交对应记录",
        }))
        .send()
        .await;
    assert_eq!(recorded.status, StatusCode::OK, "{}", recorded.text());
    assert_eq!(recorded.json()["data"]["stageStatus"], "needs_input");
    let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
    assert_eq!(attempt.submit_state.as_str(), "failed");
    let snapshot_id = job_row(&pool, &job_id).await.snapshot_id;
    let ledger = ledger_entries(&pool, &snapshot_id).await;
    let tripo_entry = ledger
        .iter()
        .find(|entry| entry.provider == manual_core::domain::ProviderKey::Tripo)
        .expect("Tripo 预留");
    assert!(
        matches!(
            tripo_entry.state,
            LedgerState::Reserved | LedgerState::Unknown
        ),
        "预留不自动释放（未决账务保留）：{:?}",
        tripo_entry.state
    );
    assert!(tripo_entry.actual.is_none(), "不得把未知填 0");
    assert_eq!(audit_count(&pool, "job_reconcile_record_no_task").await, 1);
    let metadata = audit_metadata(&pool, "job_reconcile_record_no_task").await;
    assert!(metadata.contains("\"providerProof\":false"), "{metadata}");
    assert!(
        metadata.contains("\"reservationReleased\":false"),
        "{metadata}"
    );

    // 知识分支的合并成果在模型分支重试前后必须保持不变（只重跑指定阶段）。
    let merge_asset_before = stage_of(&pool, &job_id, StageKind::ManualMerge)
        .await
        .result_asset_id
        .expect("合并结果资产");

    // 显式重试：新 attempt（不新建预留）→ 第二个提交脚本成功 → 全链路完成。
    let (_, etag2) = job_detail(&app, &cookie, &job_id).await;
    let retry = retry_stage(
        &app,
        &cookie,
        &csrf,
        &job_id,
        &submit.id,
        &etag2,
        "qa-t15-notask-retry",
    )
    .await;
    assert_eq!(retry.status, StatusCode::OK, "{}", retry.text());
    assert_eq!(retry.json()["data"]["previousStatus"], "needs_input");
    let job = drive_until_job(&pool, &executor, &clock, &job_id, JobStatus::Succeeded, 80).await;
    assert_eq!(job.status, JobStatus::Succeeded);
    let merge_asset_after = stage_of(&pool, &job_id, StageKind::ManualMerge)
        .await
        .result_asset_id
        .expect("合并结果资产");
    assert_eq!(
        merge_asset_after, merge_asset_before,
        "模型分支重试不得覆盖知识分支的已完成成果"
    );
    let ledger = ledger_entries(&pool, &job.snapshot_id).await;
    let tripo_entries = ledger
        .iter()
        .filter(|entry| entry.provider == manual_core::domain::ProviderKey::Tripo)
        .count();
    assert_eq!(tripo_entries, 1, "替代执行不新建预留（旧条目保留）");
    assert_eq!(fixture.paid_submits(), 2, "一次未知 + 一次人工授权的新提交");
    let draft = draft_for_snapshot(&pool, &job.snapshot_id)
        .await
        .expect("草稿");
    assert_eq!(draft.knowledge_json["completeness"], "complete");
    assert_eq!(release_count(&pool).await, 0);
}

// ---------------------------------------------------------------------------
// 用例 7：取消（AC-039）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_t15_cancel_keeps_submitted_state_and_adds_no_paid_steps() {
    let full = fixture_bytes("sample-model.glb");
    let fixture = QaHttp::start(vec![
        route(
            "POST",
            TRIPO_UPLOAD_PATH,
            MatchKind::Exact,
            vec![tripo_upload_script("qa-token-cancel")],
        ),
        route(
            "POST",
            TRIPO_SUBMIT_PATH,
            MatchKind::Exact,
            vec![tripo_submit_script("qa-task-cancel-0001")],
        ),
        // 轮询恒为 running：任务停在 waiting_provider（已提交阶段）。
        route(
            "GET",
            TRIPO_TASKS_PREFIX,
            MatchKind::Prefix,
            vec![tripo_running_script("qa-task-cancel-0001")],
        ),
        route_marked(
            "POST",
            MANUAL_PATH,
            &page_marker(1),
            vec![QaScript::bytes(
                200,
                "application/json",
                manual_success_for("qa-resp-cancel", &[1, 2, 3]),
            )],
        ),
        cdn_route(full),
    ]);

    let (app, cookie, csrf) = qa_app("qa-t15-cancel", &fixture).await;
    let inputs = build_inputs(&app, &cookie, &csrf, &marked_pages(3)).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "qa-t15-cancel-key").await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoPoll,
        JobStatus::WaitingProvider,
        60,
    )
    .await;
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualMerge,
        JobStatus::Succeeded,
        60,
    )
    .await;
    let requests_before = fixture.total_requests();
    assert!(
        requests_before >= 3,
        "取消前已有真实请求：{requests_before}"
    );

    // 缺 If-Match → 428；过期 revision → 412。
    let no_if_match = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/cancel"))
        .cookie(&cookie)
        .csrf(&csrf)
        .send()
        .await;
    assert_eq!(
        no_if_match.status,
        StatusCode::PRECONDITION_REQUIRED,
        "{}",
        no_if_match.text()
    );
    let stale = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/cancel"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", "\"r1\"")
        .send()
        .await;
    assert_eq!(
        stale.status,
        StatusCode::PRECONDITION_FAILED,
        "{}",
        stale.text()
    );
    assert!(stale.json()["error"]["details"]["currentRevision"].is_number());

    let (detail, etag) = job_detail(&app, &cookie, &job_id).await;
    assert!(detail["revision"].as_i64().unwrap_or(0) > 1);
    let cancelled = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/cancel"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .send()
        .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{}", cancelled.text());
    let body = cancelled.json();
    assert_eq!(body["data"]["job"]["status"], "cancelled");
    let notice = body["data"]["notice"].as_str().expect("notice");
    assert!(
        notice.contains("不撤销") || notice.contains("不会被撤销"),
        "响应不得声称已取消远端付费操作：{notice}"
    );
    // 保留查询：取消后任务详情仍可读，阶段与付费 attempt 事实仍在。
    let (detail_after_cancel, _) = job_detail(&app, &cookie, &job_id).await;
    assert_eq!(detail_after_cancel["status"], "cancelled");
    assert!(
        detail_after_cancel["attempts"]
            .as_array()
            .map(|attempts| !attempts.is_empty())
            .unwrap_or(false),
        "已提交阶段的 attempt 事实必须保留可查：{detail_after_cancel}"
    );
    assert!(
        detail_after_cancel["reservations"]
            .as_array()
            .map(|rows| !rows.is_empty())
            .unwrap_or(false),
        "账务保留：{detail_after_cancel}"
    );
    let preserved = body["data"]["preservedStages"]
        .as_array()
        .expect("preserved");
    assert!(
        preserved.iter().any(
            |stage| stage["stageKind"] == "tripo_poll" && stage["status"] == "waiting_provider"
        ),
        "已提交阶段必须保留查询与账务：{preserved:?}"
    );

    // 未提交阶段停止；已完成成果保留；已提交阶段保留查询与账务。
    let stages = stages_of(&pool, &job_id).await;
    for stage in &stages {
        let expected = match stage.stage_kind {
            StageKind::TripoPoll => JobStatus::WaitingProvider,
            StageKind::FreezeInputs
            | StageKind::ManualExtract
            | StageKind::ManualMerge
            | StageKind::TripoUpload
            | StageKind::TripoSubmit => JobStatus::Succeeded,
            StageKind::ModelDownload | StageKind::ModelValidate | StageKind::AssembleDraft => {
                JobStatus::Cancelled
            }
        };
        assert_eq!(
            stage.status,
            expected,
            "{}（batch {}）取消后的状态不符",
            stage.stage_kind.as_str(),
            stage.batch_index
        );
    }
    let snapshot_id = job_row(&pool, &job_id).await.snapshot_id;
    let ledger = ledger_entries(&pool, &snapshot_id).await;
    assert!(
        ledger
            .iter()
            .all(|entry| entry.state != LedgerState::Released),
        "取消不得静默释放预留：{ledger:?}"
    );
    assert_eq!(audit_count(&pool, "job_cancelled").await, 1);
    let cancel_metadata = audit_metadata(&pool, "job_cancelled").await;
    assert!(
        cancel_metadata.contains("\"remoteCancellationNotClaimed\":true"),
        "审计必须显式记录「不声称取消远端付费操作」：{cancel_metadata}"
    );

    // 已终态再取消 → 422 cancelNotNeeded（不假装再次取消）。
    let (detail_cancelled, etag_cancelled) = job_detail(&app, &cookie, &job_id).await;
    let _ = detail_cancelled;
    let again = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/cancel"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag_cancelled)
        .send()
        .await;
    assert_eq!(
        again.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        again.text()
    );
    assert_eq!(error_field(&again, "details")["reason"], "cancelNotNeeded");
    assert_eq!(
        audit_count(&pool, "job_cancelled").await,
        1,
        "终态取消不再写审计"
    );

    // 取消后不新增任何付费步骤（也不继续轮询）。
    for _ in 0..8 {
        tick(&executor, &clock).await;
    }
    assert_eq!(
        fixture.total_requests(),
        requests_before,
        "取消后新增了供应商请求：{:?}",
        fixture
            .requests()
            .into_iter()
            .map(|request| format!("{} {}", request.method, request.target))
            .collect::<Vec<_>>()
    );

    // 取消后重试被拒且无副作用。
    let (detail2, etag2) = job_detail(&app, &cookie, &job_id).await;
    let poll_stage = stage_json(&detail2, "tripo_poll", 0)["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let retry = retry_stage(
        &app,
        &cookie,
        &csrf,
        &job_id,
        &poll_stage,
        &etag2,
        "qa-t15-cancel-retry",
    )
    .await;
    assert_eq!(
        retry.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        retry.text()
    );
    assert_eq!(error_field(&retry, "details")["reason"], "jobCancelled");
    assert_eq!(fixture.total_requests(), requests_before);
    assert!(
        fixture.unexpected().is_empty(),
        "{:?}",
        fixture.unexpected()
    );
}

// ---------------------------------------------------------------------------
// 用例 8：按分支重试（AC-040）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_t15_retry_only_knowledge_branch_preserves_model_artifacts() {
    let full = fixture_bytes("sample-model.glb");
    let fixture = QaHttp::start(vec![
        route(
            "POST",
            TRIPO_UPLOAD_PATH,
            MatchKind::Exact,
            vec![tripo_upload_script("qa-token-retry")],
        ),
        route(
            "POST",
            TRIPO_SUBMIT_PATH,
            MatchKind::Exact,
            vec![tripo_submit_script("qa-task-retry-0001")],
        ),
        route(
            "GET",
            TRIPO_TASKS_PREFIX,
            MatchKind::Prefix,
            vec![
                tripo_running_script("qa-task-retry-0001"),
                tripo_success_script("qa-task-retry-0001", ""),
            ],
        ),
        route_marked(
            "POST",
            MANUAL_PATH,
            &page_marker(1),
            vec![
                // 第一次拒答（不产生正式知识）；重试后成功。
                QaScript::bytes(200, "application/json", manual_refusal("qa-resp-refusal")),
                QaScript::bytes(
                    200,
                    "application/json",
                    manual_success_for("qa-resp-retry-ok", &[1, 2, 3]),
                ),
            ],
        ),
    ]);
    let model_url = fixture.cdn_url();
    {
        let mut routes = fixture.state.routes.lock().unwrap();
        routes[2].steps = vec![
            tripo_running_script("qa-task-retry-0001"),
            tripo_success_script("qa-task-retry-0001", &model_url),
        ];
        routes.push(cdn_route(full));
    }

    let (app, cookie, csrf) = qa_app("qa-t15-retry", &fixture).await;
    let inputs = build_inputs(&app, &cookie, &csrf, &marked_pages(3)).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "qa-t15-retry-key").await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    // 知识分支失败：批次 needs_input、不产出正式知识；模型分支独立成功。
    let batch = tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualExtract,
        JobStatus::NeedsInput,
        60,
    )
    .await;
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::Succeeded,
        80,
    )
    .await;
    let validate = stage_of(&pool, &job_id, StageKind::ModelValidate).await;
    let model_usage = validate.usage_json.clone().expect("模型 usage");
    let model_revision = model_usage["modelRevisionId"]
        .as_str()
        .expect("模型 revision");

    let (detail, etag) = job_detail(&app, &cookie, &job_id).await;
    assert_eq!(detail["status"], "needs_input");
    let batch_json = stage_json(&detail, "manual_extract", 0);
    assert_eq!(batch_json["status"], "needs_input");
    assert_eq!(
        batch_json["knowledgeProduced"], false,
        "拒答批次必须展示为未产出知识"
    );
    assert!(
        stage_json(&detail, "assemble_draft", 0)["status"] == "queued",
        "分支头未定性 → 不组装"
    );

    let snapshot_before = job_snapshot(&pool, &job_id).await;
    let submits_before = fixture.paid_submits();

    // 缺 CSRF → 403。
    let no_csrf = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .header("if-match", &etag)
        .header("idempotency-key", "qa-t15-retry-no-csrf")
        .json(&json!({ "stageId": batch.id }))
        .send()
        .await;
    assert_eq!(no_csrf.status, StatusCode::FORBIDDEN, "{}", no_csrf.text());

    // 缺 If-Match → 428；缺 Idempotency-Key → 422。
    let no_if_match = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("idempotency-key", "qa-t15-retry-missing-ifmatch")
        .json(&json!({ "stageId": batch.id }))
        .send()
        .await;
    assert_eq!(
        no_if_match.status,
        StatusCode::PRECONDITION_REQUIRED,
        "{}",
        no_if_match.text()
    );
    let no_key = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({ "stageId": batch.id }))
        .send()
        .await;
    assert_eq!(
        no_key.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        no_key.text()
    );

    // 正例：只重跑该批次。
    let retry = retry_stage(
        &app,
        &cookie,
        &csrf,
        &job_id,
        &batch.id,
        &etag,
        "qa-t15-retry-batch",
    )
    .await;
    assert_eq!(retry.status, StatusCode::OK, "{}", retry.text());
    let retry_body = retry.json();
    assert_eq!(retry_body["data"]["stageKind"], "manual_extract");
    assert_eq!(retry_body["data"]["previousStatus"], "needs_input");
    assert!(
        retry_body["data"]["notice"]
            .as_str()
            .unwrap()
            .contains("不改变模型/质量预设")
    );
    assert_eq!(
        fixture.paid_submits(),
        submits_before,
        "重试知识分支不得重新付费购买模型"
    );

    // 幂等：同 key 同 body 重放 → 不产生第二次执行。
    let replay = retry_stage(
        &app,
        &cookie,
        &csrf,
        &job_id,
        &batch.id,
        &etag,
        "qa-t15-retry-batch",
    )
    .await;
    assert_eq!(replay.status, StatusCode::OK, "{}", replay.text());
    assert_eq!(
        replay.header("x-idempotent-replay").as_deref(),
        Some("true"),
        "重放必须标记 x-idempotent-replay"
    );
    // 同 key 不同 body → 409。
    let conflict = retry_stage(
        &app,
        &cookie,
        &csrf,
        &job_id,
        &validate.id,
        &etag,
        "qa-t15-retry-batch",
    )
    .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT, "{}", conflict.text());

    // 恢复推进：完整草稿；模型分支成果与快照预设不变。
    let job = drive_until_job(&pool, &executor, &clock, &job_id, JobStatus::Succeeded, 80).await;
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(fixture.paid_submits(), submits_before, "模型分支未被重跑");
    assert_eq!(fixture.manual_requests(), 2, "说明书批次首跑 + 重试");
    let validate_after = stage_of(&pool, &job_id, StageKind::ModelValidate).await;
    assert_eq!(validate_after.usage_json.as_ref().unwrap(), &model_usage);
    assert_eq!(
        validate_after.usage_json.as_ref().unwrap()["modelRevisionId"].as_str(),
        Some(model_revision)
    );
    let snapshot_after = job_snapshot(&pool, &job_id).await;
    assert_eq!(snapshot_before, snapshot_after, "重试不得改变模型/质量预设");
    let draft = draft_for_snapshot(&pool, &job.snapshot_id)
        .await
        .expect("草稿");
    assert_eq!(draft.knowledge_json["completeness"], "complete");
    assert_eq!(
        draft.knowledge_json["sourceJobId"].as_str(),
        Some(job_id.as_str())
    );
    assert_eq!(release_count(&pool).await, 0);
    assert_eq!(audit_count(&pool, "job_stage_retry_requested").await, 1);
    assert!(
        fixture.unexpected().is_empty(),
        "{:?}",
        fixture.unexpected()
    );
}

// ---------------------------------------------------------------------------
// 用例 9：草稿契约（ETag / 428 / 412 / 422 / 404）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_t15_draft_patch_contract_and_no_publish_side_effects() {
    let chain = run_full_chain("qa-t15-draft").await;
    let pool = pool(&chain.app);
    drive_until_job(
        &pool,
        &chain.executor,
        &chain.clock,
        &chain.job_id,
        JobStatus::Succeeded,
        60,
    )
    .await;
    let (detail, _) = job_detail(&chain.app, &chain.cookie, &chain.job_id).await;
    let draft_id = detail["draftId"].as_str().unwrap().to_owned();
    let (draft, etag) = get_draft(&chain.app, &chain.cookie, &chain.inputs_item, &draft_id).await;
    assert_eq!(etag, "\"r1\"");
    assert_eq!(draft["status"], "needs_review");
    let knowledge_before = draft["knowledge"].clone();

    // 缺 If-Match → 428；非法 If-Match → 422。
    let missing = patch_draft(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs_item,
        &draft_id,
        None,
        json!({ "status": "ready" }),
    )
    .await;
    assert_eq!(
        missing.status,
        StatusCode::PRECONDITION_REQUIRED,
        "{}",
        missing.text()
    );
    let malformed = patch_draft(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs_item,
        &draft_id,
        Some("abc"),
        json!({ "status": "ready" }),
    )
    .await;
    assert_eq!(
        malformed.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        malformed.text()
    );
    // 未知字段（绕过 T19 校验的入口）→ 422。
    let unknown_field = patch_draft(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs_item,
        &draft_id,
        Some(&etag),
        json!({ "status": "ready", "knowledgeJson": { "hotspots": [] } }),
    )
    .await;
    assert_eq!(
        unknown_field.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "不得提供绕过 T19 校验的入口：{}",
        unknown_field.text()
    );
    // 空请求体 → 422。
    let empty = patch_draft(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs_item,
        &draft_id,
        Some(&etag),
        json!({}),
    )
    .await;
    assert_eq!(
        empty.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        empty.text()
    );

    // 正例：needs_review → ready（人工复核声明），不产生 release。
    let ready = patch_draft(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs_item,
        &draft_id,
        Some(&etag),
        json!({ "status": "ready" }),
    )
    .await;
    assert_eq!(ready.status, StatusCode::OK, "{}", ready.text());
    assert_eq!(ready.header("etag").as_deref(), Some("\"r2\""));
    assert_eq!(ready.header("etag").as_deref(), Some("\"r2\""));
    assert_eq!(ready.json()["data"]["status"], "ready");
    assert_eq!(release_count(&pool).await, 0, "草稿 ready 不等于已发布");
    assert_eq!(audit_count(&pool, "draft_status_changed").await, 1);

    // 同状态幂等（正确 revision 不递增）；stale revision → 412。
    let idempotent = patch_draft(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs_item,
        &draft_id,
        Some("\"r2\""),
        json!({ "status": "ready" }),
    )
    .await;
    assert_eq!(idempotent.status, StatusCode::OK, "{}", idempotent.text());
    assert_eq!(idempotent.header("etag").as_deref(), Some("\"r2\""));
    let stale = patch_draft(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs_item,
        &draft_id,
        Some(&etag),
        json!({ "status": "needs_review" }),
    )
    .await;
    assert_eq!(
        stale.status,
        StatusCode::PRECONDITION_FAILED,
        "{}",
        stale.text()
    );
    assert_eq!(stale.json()["error"]["details"]["currentRevision"], 2);

    // 知识内容不因状态变更被改写；跨物品 404。
    let (after, etag_after) =
        get_draft(&chain.app, &chain.cookie, &chain.inputs_item, &draft_id).await;
    assert_eq!(etag_after, "\"r2\"");
    assert_eq!(after["knowledge"], knowledge_before);
    assert_eq!(after["missing"], draft["missing"]);
    let other_item = {
        let response = chain
            .app
            .call(Method::POST, "/api/v1/items")
            .cookie(&chain.cookie)
            .csrf(&chain.csrf)
            .json(&json!({ "name": "QA 另一个物品", "model": "QA-OTHER" }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::CREATED);
        response.json()["data"]["id"].as_str().unwrap().to_owned()
    };
    let cross = chain
        .app
        .call(
            Method::GET,
            &format!("/api/v1/items/{other_item}/drafts/{draft_id}"),
        )
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "跨物品按不存在处理");
    let cross_patch = chain
        .app
        .call(
            Method::PATCH,
            &format!("/api/v1/items/{other_item}/drafts/{draft_id}"),
        )
        .cookie(&chain.cookie)
        .csrf(&chain.csrf)
        .header("if-match", "\"r2\"")
        .json(&json!({ "status": "needs_review" }))
        .send()
        .await;
    assert_eq!(cross_patch.status, StatusCode::NOT_FOUND);
    assert_eq!(release_count(&pool).await, 0);
}

// ---------------------------------------------------------------------------
// 用例 10：T14 P3-1（拒答批次恢复不得被补推进为成功知识）+ 正对照
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_t15_refused_batch_recovery_never_reports_success() {
    let fixture = QaHttp::start(vec![
        // 第 1 批（含第 1 页）→ 拒答；其余（第 2 批，扫描页）→ 成功。
        route_marked(
            "POST",
            MANUAL_PATH,
            &page_marker(1),
            vec![QaScript::bytes(
                200,
                "application/json",
                manual_refusal("qa-resp-b0"),
            )],
        ),
        route(
            "POST",
            MANUAL_PATH,
            MatchKind::Exact,
            vec![QaScript::bytes(
                200,
                "application/json",
                manual_success_for("qa-resp-b1", &[6]),
            )],
        ),
    ]);

    let (app, cookie, csrf) = qa_app("qa-t15-p31", &fixture).await;
    // 5 页文字 + 1 页扫描（扫描页走页图，不参与页文字标记路由）。
    let mut pages = marked_pages(5);
    pages.push(PageSpec::Scan);
    let inputs = build_inputs(&app, &cookie, &csrf, &pages).await;
    let item = inputs.item.clone();
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "qa-t15-p31-key").await;
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = qa_executor(&app, Arc::clone(&clock));
    let pool = pool(&app);

    // 6 页 → 2 批：批次 0（页 1..5）拒答 → needs_input；批次 1（页 6）成功。
    let batches = stages_of(&pool, &job_id).await;
    let extract: Vec<JobStage> = batches
        .into_iter()
        .filter(|stage| stage.stage_kind == StageKind::ManualExtract)
        .collect();
    assert_eq!(extract.len(), 2, "6 页必须分成 2 批");
    let refused = extract
        .iter()
        .find(|stage| {
            stage
                .page_set
                .as_ref()
                .is_some_and(|pages| pages.contains(&1))
        })
        .expect("批次 0（含第 1 页）")
        .clone();
    let produced = extract
        .iter()
        .find(|stage| {
            stage
                .page_set
                .as_ref()
                .is_some_and(|pages| pages.contains(&6))
        })
        .expect("批次 1（含第 6 页）")
        .clone();

    for _ in 0..40 {
        let current = stages_of(&pool, &job_id).await;
        let b0 = current.iter().find(|stage| stage.id == refused.id).unwrap();
        let b1 = current
            .iter()
            .find(|stage| stage.id == produced.id)
            .unwrap();
        if b0.status == JobStatus::NeedsInput && b1.status == JobStatus::Succeeded {
            break;
        }
        tick(&executor, &clock).await;
    }
    let b0 = stages_of(&pool, &job_id)
        .await
        .into_iter()
        .find(|stage| stage.id == refused.id)
        .unwrap();
    assert_eq!(b0.status, JobStatus::NeedsInput, "拒答批次停在 needs_input");
    assert!(b0.result_asset_id.is_some(), "拒答也写诊断结果资产（T14）");
    let refused_usage = b0.usage_json.clone().expect("拒答批次 usage");
    assert_eq!(refused_usage["producedKnowledge"], false);
    let requests_before = fixture.manual_requests();

    // 正对照：产出知识的批次在崩溃路径仍按 T10 补推进为 succeeded。
    let b1 = stages_of(&pool, &job_id)
        .await
        .into_iter()
        .find(|stage| stage.id == produced.id)
        .unwrap();
    assert_eq!(b1.status, JobStatus::Succeeded, "批次 1 应已成功");
    reopen_stage_with_result_fact(&pool, &b1.id).await;
    let report = executor.recover_expired_leases().await.expect("恢复扫描");
    assert_eq!(report.succeeded, 1, "正对照：产出知识的批次补推进");
    let b1_after = stages_of(&pool, &job_id)
        .await
        .into_iter()
        .find(|stage| stage.id == produced.id)
        .unwrap();
    assert_eq!(b1_after.status, JobStatus::Succeeded);

    // P3-1：拒答/无知识批次在"结果已落库、checkpoint 未推进"的恢复路径不得被补推进为成功。
    reopen_stage_with_result_fact(&pool, &b0.id).await;
    let report = executor.recover_expired_leases().await.expect("恢复扫描");
    assert_eq!(report.needs_input, 1, "拒答批次必须回到 needs_input");
    assert_eq!(report.succeeded, 0, "不得把拒答批次补推进为 succeeded");
    let b0_after = stages_of(&pool, &job_id)
        .await
        .into_iter()
        .find(|stage| stage.id == refused.id)
        .unwrap();
    assert_eq!(b0_after.status, JobStatus::NeedsInput);
    eprintln!(
        "QA-OBSERVE[p3-1-recovery] refused_batch={} produced_batch={} producedKnowledge={:?} merge={} assemble={} drafts={} manual_requests={}",
        b0_after.status.as_str(),
        b1_after.status.as_str(),
        b0_after
            .usage_json
            .as_ref()
            .and_then(|usage| usage.get("producedKnowledge")),
        stage_of(&pool, &job_id, StageKind::ManualMerge)
            .await
            .status
            .as_str(),
        stage_of(&pool, &job_id, StageKind::AssembleDraft)
            .await
            .status
            .as_str(),
        draft_count(&pool, &item).await,
        fixture.manual_requests(),
    );
    assert!(
        b0_after
            .needs_input_json
            .as_ref()
            .map(|value| value.to_string().contains("manual_ai_refusal"))
            .unwrap_or(false),
        "缺项必须沿用批次事实里的稳定错误码：{:?}",
        b0_after.needs_input_json
    );
    assert_eq!(
        batch_count(&pool, &job_id, JobStatus::Succeeded).await,
        1,
        "只有产出知识的批次是 succeeded"
    );
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::ManualMerge)
            .await
            .status,
        JobStatus::Queued,
        "批次未全部成功 → merge 不解锁"
    );
    assert_eq!(draft_count(&pool, &item).await, 0, "merge 未解锁 → 不组装");
    assert_eq!(
        fixture.manual_requests(),
        requests_before,
        "恢复不得重新请求（不重复付费）"
    );
    let (detail, _) = job_detail(&app, &cookie, &job_id).await;
    assert_eq!(detail["status"], "needs_input");
    let refused_row = stage_json(&detail, "manual_extract", refused.batch_index);
    assert_eq!(refused_row["knowledgeProduced"], false);
    assert!(
        fixture.unexpected().is_empty(),
        "{:?}",
        fixture.unexpected()
    );
}

// ---------------------------------------------------------------------------
// 用例 11：assemble_draft 的内容比较 upsert（内容变化才递增 revision + 回到 needs_review）
// ---------------------------------------------------------------------------

/// 卡片项「assemble_draft 幂等（内容比较 upsert）」的另一半：内容**相同**不写库（用例 2），
/// 内容**变化**才 `revision + 1` 并把人工"已复核"声明拉回 `needs_review`。
#[tokio::test]
async fn qa_t15_assembly_revision_follows_content_change_only() {
    let chain = run_full_chain("qa-t15-upsert").await;
    let pool = pool(&chain.app);
    drive_until_job(
        &pool,
        &chain.executor,
        &chain.clock,
        &chain.job_id,
        JobStatus::Succeeded,
        60,
    )
    .await;
    let snapshot_id = job_row(&pool, &chain.job_id).await.snapshot_id;
    let draft = draft_for_snapshot(&pool, &snapshot_id).await.expect("草稿");
    assert_eq!(draft.revision, 1);

    // 人工把草稿标为 ready（r2）：后续内容变化必须把状态拉回 needs_review（保守方向）。
    let (_, etag) = get_draft(&chain.app, &chain.cookie, &chain.inputs_item, &draft.id).await;
    assert_eq!(etag, "\"r1\"");
    let ready = patch_draft(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs_item,
        &draft.id,
        Some(&etag),
        json!({ "status": "ready" }),
    )
    .await;
    assert_eq!(ready.status, StatusCode::OK, "{}", ready.text());

    // 模拟"分支产物变化"：读取合并结果 → 改动一处事实 → 作为新资产落库 → 指到合并阶段。
    let merge = stage_of(&pool, &chain.job_id, StageKind::ManualMerge).await;
    let merge_asset = merge.result_asset_id.clone().expect("合并结果资产");
    let mut merged: Value =
        serde_json::from_slice(&asset_bytes(&chain.app, &chain.cookie, &merge_asset).await)
            .expect("合并结果资产是 JSON");
    merged["parts"][0]["name"] = json!("QA 现场改写后的部件名");
    let changed = upload_asset(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs_item,
        "pageText",
        "merged-changed.json",
        "text/plain",
        merged.to_string().as_bytes(),
    )
    .await;
    sqlx::query("UPDATE job_stages SET result_asset_id = ? WHERE id = ?")
        .bind(&changed)
        .bind(&merge.id)
        .execute(&pool)
        .await
        .expect("改写合并阶段结果资产（测试注入内容变化）");

    let job = job_row(&pool, &chain.job_id).await;
    let outcome =
        everything_manual::drafts::assemble_draft(&pool, chain.app.dir(), &job, Timestamp::now())
            .await
            .expect("内容变化后的组装必须成功");
    assert!(!outcome.created, "同一快照仍是一份草稿（不新建行）");
    // ready 声明把 revision 推到 2；内容变化再 +1（在**当前** revision 上递增）。
    assert_eq!(outcome.draft.revision, 3, "内容变化才递增 revision");
    assert_eq!(
        outcome.draft.status,
        manual_core::domain::DraftStatus::NeedsReview,
        "内容变化后旧的已复核声明不再适用"
    );
    assert_eq!(draft_count(&pool, &chain.inputs_item).await, 1);
    assert!(
        outcome.draft.knowledge_json["knowledge"]["parts"][0]["name"]
            .as_str()
            .unwrap()
            .contains("QA 现场改写")
    );

    // 再次组装（内容未变）→ 不写库、不递增。
    let again = everything_manual::drafts::assemble_draft(
        &pool,
        chain.app.dir(),
        &job_row(&pool, &chain.job_id).await,
        Timestamp::now(),
    )
    .await
    .expect("重复组装必须成功");
    assert!(!again.created);
    assert_eq!(again.draft.revision, 3, "内容相同不写库");
    assert_eq!(release_count(&pool).await, 0);
}
