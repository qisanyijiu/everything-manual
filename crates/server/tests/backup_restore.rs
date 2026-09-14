//! T20 集成测试：备份、恢复、导出与迁移门禁
//! （PRD 修订 2 / ui_revision 2；REQ-005、REQ-037，AC-009、AC-010、AC-058）。
//!
//! 覆盖（命令 ↔ AC 见 implementation.md §T20）：
//! - **备份要求停服**（AC-009）：真实 `serve` 子进程持有 data-dir 排他锁时，
//!   `backup` 退出码 5 并说明需先停服；停服后成功；产物含一致 SQLite 快照
//!   （VACUUM INTO，含 WAL；**不是主文件复制**）+ 全部被引用 blob + manifest + sha256；
//!   快照不含会话（恢复后需重新登录）；备份不修改源数据；`--out` 已存在不覆盖；
//! - **恢复**（AC-010）：目标必须不存在或为空；损坏 blob／快照／manifest 的备份
//!   一律失败（退出码 7）且**不创建目标目录**（保留现场）；备份库 schema 比程序新
//!   拒绝恢复（退出码 4）；新空目录恢复成功后**真实起服务读取同一 release**：
//!   manifest 哈希一致、PDF 与 GLB 字节与发布时一致、管理员口令仍可登录；
//! - **导出**（AC-058）：`GET /releases/{releaseId}/export` 返回 ZIP，只含该 release
//!   的原件（PDF）与 GLB + 导出清单 + 冻结 manifest；不含密钥（canary）、会话 token、
//!   绝对路径、临时云端 URL；其它物品/照片/页图的资产字节不在包内；未登录 401、
//!   未知 release 404；
//! - **迁移门禁**：旧 schema（v1）备份可恢复并自动迁移（数据保留、`serve` 输出
//!   升级提示）；比程序新的 schema 被拒（不修改目标）。
//!
//! 隔离与门控：与 T13/T15/T19 相同——造数走本机 fixture（127.0.0.1 随机端口）、
//! 测试构建 + 显式配置才放行回环模型下载、零真实外网、零真实付费；
//! CLI 用例全部用真实二进制子进程 + 临时目录，不接触开发者真实 data-dir。

mod common;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use common::{TestApp, TestDir};
use everything_manual::backup::manifest::{BACKUP_SCHEMA_VERSION, BackupManifest};
use everything_manual::backup::zip::parse_stored_zip;
use everything_manual::config::{ProviderSettings, SecretString};
use everything_manual::generation::catalog;
use everything_manual::jobs::executor::fixed_jitter_executor;
use everything_manual::jobs::{
    ExecutorConfig, JobExecutor, ManualClock, PipelineHandlers, StageRegistry, TickOutcome,
};
use everything_manual::providers::register_provider_handlers;
use everything_manual::storage::repo;
use manual_core::domain::{JobStatus, ManualDraft};
use manual_core::timestamps::Timestamp;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use test_support::FixtureServer;
use test_support::scenario::{BodySpec, PathMatchSpec, ResponseSpec, RouteScript, Scenario, Step};

const BIN: &str = env!("CARGO_BIN_EXE_everything-manual");
const PASSWORD: &str = "test-password-t20-backup";
/// 测试用假凭据（canary）：导出包与备份不得出现（内容扫描断言）。
const CANARY_KEY: &str = "canary-t20-not-a-real-key";
const MANUAL_AI_MODEL: &str = "gpt-5-mini";
const PRESET: &str = "tripo-h-v3.1-standard";
/// fixture 里出现在 Tripo 任务响应中的"临时云端 URL"（不得进入导出包）。
const TEMP_CLOUD_URL: &str = "cdn.example.invalid";

const TEST_CATALOG: &str = r#"
version = "2026-09-11"
snapshot_date = "2026-09-11"

[[tripo.presets]]
preset = "tripo-h-v3.1-standard"
model = "v3.1-20260211"
credits = "30"

[manual_ai.models.gpt-5-mini]
input_usd_per_million_tokens = "0.25"
output_usd_per_million_tokens = "2.00"
image_usd_per_image = "0.01"
"#;

const RESPONSES_DIR: &str = "responses/manual_ai";
const MANUAL_AI_PATH: &str = "/v1/responses";
const TRIPO_UPLOAD_PATH: &str = "/v3/files";
const TRIPO_SUBMIT_PATH: &str = "/v3/generation/multiview-to-model";
const TRIPO_TASKS_PREFIX: &str = "/v3/tasks/";
const TASK_ID: &str = "t20-fixture-task-0001";

// ---------------------------------------------------------------------------
// CLI 子进程工具（与 config_cli.rs 同一手法）
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CmdOutput {
    status: i32,
    stdout: String,
    stderr: String,
}

/// 在指定工作目录运行二进制（清空环境变量，保证用例确定性）。
fn run(dir: &Path, args: &[&str]) -> CmdOutput {
    let output = Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("二进制应可启动");
    let output = CmdOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    };
    assert!(
        !output.stderr.contains("panicked at"),
        "错误路径不得 panic：{output:?}"
    );
    output
}

/// 真实 `serve` 子进程（持有 data-dir 排他锁）。
struct ServeProcess {
    child: Child,
    addr: String,
    /// 保持接收端存活：一旦丢弃，读线程会关闭 stdout 管道，子进程后续 `println!`
    /// 会因 EPIPE 直接 panic（exit 101）。`_` 前缀表示只用于保活，不再读取。
    _stdout_lines: mpsc::Receiver<String>,
    stdout_seen: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
}

impl ServeProcess {
    fn start(work_dir: &Path, data_dir: &Path) -> Self {
        let mut child = Command::new(BIN)
            .args([
                "serve",
                "--data-dir",
                data_dir.to_str().expect("路径是 UTF-8"),
                "--listen",
                "127.0.0.1:0",
            ])
            .current_dir(work_dir)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("serve 应能启动");

        let (sender, receiver) = mpsc::channel::<String>();
        let stdout = child.stdout.take().expect("stdout 已管道化");
        let stdout_seen = Arc::new(Mutex::new(String::new()));
        let stdout_sink = stdout_seen.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                stdout_sink.lock().unwrap().push_str(&format!("{line}\n"));
                if sender.send(line).is_err() {
                    break;
                }
            }
        });

        let stderr = Arc::new(Mutex::new(String::new()));
        let stderr_sink = stderr.clone();
        let stderr_pipe = child.stderr.take().expect("stderr 已管道化");
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr_pipe), &mut text);
            *stderr_sink.lock().unwrap() = text;
        });

        let deadline = Instant::now() + Duration::from_secs(20);
        let addr = loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("等待 listening 行超时");
            match receiver.recv_timeout(remaining) {
                Ok(line) => {
                    if let Some(addr) = line.strip_prefix("listening on http://") {
                        break addr.trim().to_owned();
                    }
                }
                Err(error) => panic!(
                    "serve 未打印 listening 行（{error}）；stderr：{}",
                    stderr.lock().unwrap()
                ),
            }
        };

        // `listening on` 打印在 SIGTERM 处理器注册之前（见 common::settle_after_listening_line
        // 与 BUG-013）：在把句柄交给调用方之前先 settle，使后续任何 SIGTERM 都不会命中
        // "信号处理器尚未安装"的启动竞态。只加等待，不改任何断言。
        common::settle_after_listening_line();

        Self {
            child,
            addr,
            _stdout_lines: receiver,
            stdout_seen,
            stderr,
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// 全部已见 stdout（启动提示断言用）。
    fn all_stdout(&self) -> String {
        self.stdout_seen.lock().unwrap().clone()
    }

    /// SIGTERM 并等待优雅退出（释放锁），返回 (退出码, stdout, stderr)。
    fn terminate(mut self) -> (i32, String, String) {
        let pid = self.child.id().to_string();
        let _ = Command::new("kill")
            .args(["-TERM", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + Duration::from_secs(15);
        let code = loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                break status.code().unwrap_or(-1);
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("SIGTERM 后未在 15 秒内退出");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let stdout = self.all_stdout();
        let stderr = self.stderr.lock().unwrap().clone();
        (code, stdout, stderr)
    }
}

// ---------------------------------------------------------------------------
// 文件指纹与备份 manifest 工具
// ---------------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    test_support::assets::sha256_hex(bytes)
}

fn fingerprint(path: &Path) -> (String, u64) {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("读取 {}：{error}", path.display()));
    (sha256_hex(&bytes), bytes.len() as u64)
}

fn read_backup_manifest(backup_dir: &Path) -> BackupManifest {
    let bytes = std::fs::read(backup_dir.join("manifest.json")).expect("读取备份 manifest");
    BackupManifest::parse(&bytes).expect("备份 manifest 必须是本程序认识的格式")
}

/// 备份目录里相对路径（`a/b`）的绝对路径。
fn backup_file(backup_dir: &Path, relative: &str) -> PathBuf {
    backup_dir.join(relative)
}

// ---------------------------------------------------------------------------
// fixture 工具（与 T19 用例同构）
// ---------------------------------------------------------------------------

fn respond_file(path: &str) -> Step {
    Step::Respond {
        response: ResponseSpec {
            status: 200,
            headers: BTreeMap::new(),
            body: BodySpec::File {
                file: path.to_owned(),
            },
        },
    }
}

fn respond_json(value: Value) -> Step {
    Step::Respond {
        response: ResponseSpec {
            status: 200,
            headers: BTreeMap::new(),
            body: BodySpec::Json { json: value },
        },
    }
}

fn exact_route(method: &str, path: &str, steps: Vec<Step>) -> RouteScript {
    RouteScript {
        method: method.to_owned(),
        path: path.to_owned(),
        path_match: PathMatchSpec::Exact,
        repeat_last: false,
        steps,
    }
}

fn prefix_route(method: &str, path: &str, repeat_last: bool, steps: Vec<Step>) -> RouteScript {
    RouteScript {
        method: method.to_owned(),
        path: path.to_owned(),
        path_match: PathMatchSpec::Prefix,
        repeat_last,
        steps,
    }
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("读取样例资产 {} 失败：{error}", path.display()))
}

fn responses_path(name: &str) -> String {
    format!("{RESPONSES_DIR}/{name}")
}

fn manual_ai_server(steps: Vec<Step>) -> FixtureServer {
    FixtureServer::start(Scenario::new(vec![exact_route(
        "POST",
        MANUAL_AI_PATH,
        steps,
    )]))
}

fn tripo_server(submit_steps: Vec<Step>, poll_steps: Vec<Step>) -> FixtureServer {
    let mut upload_steps = Vec::new();
    for index in 0..4 {
        upload_steps.push(respond_json(json!({
            "code": 0,
            "data": { "file_token": format!("token-{index}-t20") }
        })));
    }
    FixtureServer::start(Scenario::new(vec![
        exact_route("POST", TRIPO_UPLOAD_PATH, upload_steps),
        exact_route("POST", TRIPO_SUBMIT_PATH, submit_steps),
        prefix_route("GET", TRIPO_TASKS_PREFIX, false, poll_steps),
    ]))
}

fn submit_success() -> Step {
    respond_json(json!({ "code": 0, "data": { "task_id": TASK_ID } }))
}

fn task_success_with(model_url: &str) -> Step {
    respond_json(json!({
        "code": 0,
        "data": {
            "task_id": TASK_ID,
            "status": "success",
            "progress": 100,
            "credits_consumed": 30,
            "output": {
                "model_url": model_url,
                "rendered_image_url": format!("https://{TEMP_CLOUD_URL}/preview.png"),
            }
        }
    }))
}

/// CDN fixture：模型字节写进**每调用唯一**的临时文件（并行用例共路径会互相截断，
/// 见 T19 的间歇假失败记录）。
fn cdn_server(path: &str, bytes: Vec<u8>) -> (FixtureServer, String) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "em-t20-cdn-{}-{}-{}",
        std::process::id(),
        sequence,
        path.replace('/', "_")
    ));
    std::fs::write(&file, &bytes).expect("写入 CDN 模型文件");
    let server = FixtureServer::start(Scenario::new(vec![exact_route(
        "GET",
        path,
        vec![Step::Respond {
            response: ResponseSpec {
                status: 200,
                headers: BTreeMap::new(),
                body: BodySpec::File {
                    file: file.to_string_lossy().into_owned(),
                },
            },
        }],
    )]));
    let url = format!("{}{}", server.base_url(), path);
    (server, url)
}

// ---------------------------------------------------------------------------
// 测试链（发布一个 release 的真实流水线）
// ---------------------------------------------------------------------------

async fn pipeline_app(tag: &str, tripo_base: &str, manual_ai_base: &str) -> TestApp {
    let dir = TestDir::new(tag);
    let mut settings = common::test_settings(dir.path());
    let mut tripo = common::configured_tripo(CANARY_KEY);
    tripo.base_url = tripo_base.to_owned();
    settings.providers.tripo = tripo;
    settings.providers.manual_ai = ProviderSettings {
        name: "manual_ai",
        base_url: manual_ai_base.to_owned(),
        model: Some(MANUAL_AI_MODEL.to_owned()),
        api_key: Some(SecretString::new(CANARY_KEY)),
        key_source: Some("测试注入".to_owned()),
    };
    settings.download.allowed_hosts = vec!["127.0.0.1".to_owned()];
    settings.download.allow_local_fixture = true;
    settings.price_catalog_path = Some(dir.join("price-catalog.toml"));
    std::fs::write(dir.join("price-catalog.toml"), TEST_CATALOG).expect("写入测试价格目录");
    settings.price_catalog = Some(catalog::parse(TEST_CATALOG).expect("测试价格目录必须可解析"));
    TestApp::with_settings(dir, settings).await
}

async fn logged_in(app: &TestApp) -> (String, String) {
    app.set_admin_password(PASSWORD).await;
    let response = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&json!({ "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let csrf = response.json()["data"]["csrfToken"]
        .as_str()
        .expect("登录响应包含 csrfToken")
        .to_owned();
    (response.session_cookie(), csrf)
}

struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new(tag: &str) -> Self {
        Self {
            boundary: format!("----em-t20-{tag}"),
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

struct ReadyInputs {
    item: String,
    preparation: String,
    photo_ids: Vec<String>,
}

/// 物品 + PDF（3 页）+ front/left 照片 + ready 准备（与 T15/T19 同一造数路径）。
async fn build_ready_inputs(app: &TestApp, cookie: &str, csrf: &str) -> ReadyInputs {
    let item = {
        let response = app
            .call(Method::POST, "/api/v1/items")
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({ "name": "备份测试物品", "model": "X100V" }))
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
    let document = app
        .call(Method::POST, &format!("/api/v1/items/{item}/documents"))
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "sourceAssetId": doc_asset, "title": "样例说明书" }))
        .send()
        .await;
    assert_eq!(document.status, StatusCode::CREATED, "{}", document.text());
    let document_id = document.json()["data"]["id"].as_str().unwrap().to_owned();
    let source_sha = document.json()["data"]["sourceSha256"]
        .as_str()
        .unwrap()
        .to_owned();

    let created = app
        .call(
            Method::POST,
            &format!("/api/v1/documents/{document_id}/preparations"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "sourceSha256": source_sha }))
        .send()
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let preparation = created.json()["data"]["id"].as_str().unwrap().to_owned();

    let page_jpeg = fixture_bytes("sample-photo-front.jpg");
    for page in 1..=3 {
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
        let text_asset = upload_asset(
            app,
            cookie,
            csrf,
            &item,
            "pageText",
            "page.txt",
            "text/plain",
            format!("T20-PAGE-{page}-CONTENT 后盖 螺钉 电池").as_bytes(),
        )
        .await;
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
    let current = app
        .call(Method::GET, &format!("/api/v1/preparations/{preparation}"))
        .cookie(cookie)
        .send()
        .await;
    let etag = current.header("etag").expect("准备详情必须带 ETag");
    let completed = app
        .call(
            Method::POST,
            &format!("/api/v1/preparations/{preparation}/complete"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .header("if-match", &etag)
        .json(&json!({ "pageCount": 3 }))
        .send()
        .await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text());

    let mut photo_ids = Vec::new();
    for (view, name, content_type) in [
        ("front", "sample-photo-front.jpg", "image/jpeg"),
        ("left", "sample-photo-left.png", "image/png"),
    ] {
        let bytes = fixture_bytes(name);
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

    ReadyInputs {
        item,
        preparation,
        photo_ids,
    }
}

async fn create_job(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    inputs: &ReadyInputs,
    key: &str,
) -> String {
    let estimate = app
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
    assert_eq!(estimate.status, StatusCode::CREATED, "{}", estimate.text());
    let quote_id = estimate.json()["data"]["id"].as_str().unwrap().to_owned();

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
            "limits": { "tripoCreditMinor": 3000, "manualAiUsdMicros": 500_000 },
        }))
        .send()
        .await;
    assert_eq!(job.status, StatusCode::ACCEPTED, "{}", job.text());
    job.json()["data"]["id"].as_str().unwrap().to_owned()
}

struct Chain {
    app: TestApp,
    cookie: String,
    csrf: String,
    inputs: ReadyInputs,
    executor: Arc<JobExecutor>,
    clock: Arc<ManualClock>,
}

async fn chain(tag: &str, tripo: &FixtureServer, manual: &FixtureServer) -> Chain {
    let app = pipeline_app(
        tag,
        &format!("{}/v3", tripo.base_url()),
        &format!("{}/v1", manual.base_url()),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let settings = app.state().settings().clone();
    let mut registry = StageRegistry::new();
    register_provider_handlers(&mut registry, &settings).expect("已配置的 Provider 必须能注册");
    PipelineHandlers::from_settings(&settings).register(&mut registry);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = fixed_jitter_executor(
        app.state().database().pool().clone(),
        ExecutorConfig {
            lease: Duration::from_secs(120),
            renew: Duration::from_secs(20),
            ..ExecutorConfig::default()
        },
        registry,
        clock.clone(),
    );
    Chain {
        app,
        cookie,
        csrf,
        inputs,
        executor,
        clock,
    }
}

fn pool(chain: &Chain) -> SqlitePool {
    chain.app.state().database().pool().clone()
}

async fn run_job_to_draft(chain: &Chain, key: &str) -> ManualDraft {
    let job_id = create_job(&chain.app, &chain.cookie, &chain.csrf, &chain.inputs, key).await;
    let pool = pool(chain);
    for _ in 0..80 {
        let job = {
            let mut conn = pool.acquire().await.expect("连接");
            repo::jobs::get(&mut conn, &job_id)
                .await
                .expect("读取任务")
                .expect("任务存在")
        };
        let draft = {
            let mut conn = pool.acquire().await.expect("连接");
            repo::drafts::get_by_snapshot(&mut conn, &job.snapshot_id)
                .await
                .expect("读取草稿")
        };
        if let Some(draft) = draft
            && (job.status == JobStatus::Succeeded || job.status == JobStatus::NeedsInput)
        {
            assert_eq!(
                job.status,
                JobStatus::Succeeded,
                "fixture 全链路必须成功（草稿已产出但任务 {}）",
                job.status.as_str()
            );
            return draft;
        }
        chain.clock.advance_millis(20_000);
        match chain.executor.tick().await.expect("tick") {
            TickOutcome::Executed(_) | TickOutcome::Idle => {}
        }
    }
    panic!("任务未在 80 tick 内产出草稿");
}

struct DraftView {
    json: Value,
    etag: String,
}

async fn get_draft(chain: &Chain, draft_id: &str) -> DraftView {
    let response = chain
        .app
        .call(
            Method::GET,
            &format!("/api/v1/items/{}/drafts/{draft_id}", chain.inputs.item),
        )
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    DraftView {
        json: response.json(),
        etag: response.header("etag").expect("草稿必须带 ETag"),
    }
}

async fn patch_ok(chain: &Chain, draft_id: &str, view: &DraftView, body: Value) -> DraftView {
    let response = chain
        .app
        .call(
            Method::PATCH,
            &format!("/api/v1/items/{}/drafts/{draft_id}", chain.inputs.item),
        )
        .cookie(&chain.cookie)
        .csrf(&chain.csrf)
        .header("if-match", &view.etag)
        .json(&body)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    DraftView {
        json: response.json(),
        etag: response.header("etag").expect("更新后带 ETag"),
    }
}

fn entity_ids(knowledge: &Value, field: &str) -> Vec<String> {
    knowledge["knowledge"][field]
        .as_array()
        .expect("知识数组")
        .iter()
        .map(|entry| entry["id"].as_str().expect("实体 id").to_owned())
        .collect()
}

fn model_identity(knowledge: &Value) -> (String, String) {
    let model = &knowledge["model"];
    (
        model["revisionId"].as_str().expect("revisionId").to_owned(),
        model["sha256"].as_str().expect("sha256").to_owned(),
    )
}

fn confirm_all(knowledge: &Value) -> Value {
    let mut entities = serde_json::Map::new();
    for field in ["parts", "steps", "specs"] {
        for id in entity_ids(knowledge, field) {
            entities.insert(id, json!({ "reviewStatus": "confirmed" }));
        }
    }
    json!({ "entities": Value::Object(entities) })
}

fn hotspot_upsert(part_id: &str, revision_id: &str, sha256: &str, local: [f64; 3]) -> Value {
    json!({
        "partId": part_id,
        "status": "confirmed",
        "anchor": {
            "modelRevisionId": revision_id,
            "modelSha256": sha256,
            "positionLocal": local,
        }
    })
}

/// 已发布版本的关键标识（后续备份/恢复/导出断言共用）。
struct PublishedRelease {
    release_id: String,
    item_id: String,
    manifest_sha256: String,
    model_asset_id: String,
    model_sha256: String,
    document_asset_id: String,
    document_sha256: String,
}

/// 完整发布一次：确认知识 → 确认热点 → modelReview → 发布 → 读回详情。
async fn publish_release(chain: &Chain, key: &str) -> PublishedRelease {
    let draft = run_job_to_draft(chain, key).await;
    let draft_id = draft.id.clone();
    let view = get_draft(chain, &draft_id).await;
    let knowledge = view.json["data"]["knowledge"].clone();
    let (revision_id, sha256) = model_identity(&knowledge);
    let parts = entity_ids(&knowledge, "parts");
    let upserts: Vec<Value> = parts
        .iter()
        .map(|part| hotspot_upsert(part, &revision_id, &sha256, [0.3, 0.1, 0.2]))
        .collect();
    let mut view = patch_ok(chain, &draft_id, &view, confirm_all(&knowledge)).await;
    view = patch_ok(
        chain,
        &draft_id,
        &view,
        json!({ "hotspots": { "upsert": upserts } }),
    )
    .await;
    view = patch_ok(
        chain,
        &draft_id,
        &view,
        json!({ "modelReview": { "loaded": true, "userConfirmed": true } }),
    )
    .await;

    let published = chain
        .app
        .call(
            Method::POST,
            &format!(
                "/api/v1/items/{}/drafts/{draft_id}/publish",
                chain.inputs.item
            ),
        )
        .cookie(&chain.cookie)
        .csrf(&chain.csrf)
        .header("if-match", &view.etag)
        .header("idempotency-key", key)
        .send()
        .await;
    assert_eq!(
        published.status,
        StatusCode::CREATED,
        "{}",
        published.text()
    );
    let release_id = published.json()["data"]["id"].as_str().unwrap().to_owned();

    let detail = chain
        .app
        .call(
            Method::GET,
            &format!("/api/v1/items/{}/releases/{release_id}", chain.inputs.item),
        )
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.text());
    let body = detail.json();
    let manifest = body["data"]["manifest"].clone();
    let manifest_sha256 = body["data"]["manifestSha256"]
        .as_str()
        .expect("manifestSha256")
        .to_owned();
    let mut model = None;
    let mut document = None;
    for entry in manifest["assets"].as_array().expect("assets[]") {
        match entry["role"].as_str() {
            Some("model") => {
                model = Some((
                    entry["assetId"].as_str().unwrap().to_owned(),
                    entry["sha256"].as_str().unwrap().to_owned(),
                ))
            }
            Some("document") => {
                document = Some((
                    entry["assetId"].as_str().unwrap().to_owned(),
                    entry["sha256"].as_str().unwrap().to_owned(),
                ))
            }
            _ => {}
        }
    }
    let (model_asset_id, model_sha256) = model.expect("发布 manifest 必须有模型资产");
    let (document_asset_id, document_sha256) = document.expect("发布 manifest 必须有原件资产");
    PublishedRelease {
        release_id,
        item_id: chain.inputs.item.clone(),
        manifest_sha256,
        model_asset_id,
        model_sha256,
        document_asset_id,
        document_sha256,
    }
}

/// 建一条完整的"已发布版本"测试链（fixture 全链路 → 发布）。
async fn published_chain(tag: &str) -> (Chain, PublishedRelease, FixtureServer, FixtureServer) {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = cdn_server("/model.glb", fixture_bytes("sample-model.glb"));
    let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&model_url)]);
    let chain = chain(tag, &tripo, &manual).await;
    let release = publish_release(&chain, &format!("{tag}-publish-key")).await;
    // CDN fixture 在发布后不再需要；保留 tripo/manual 直到用例结束（fixture 线程自行退出）。
    drop(cdn);
    (chain, release, tripo, manual)
}

// ---------------------------------------------------------------------------
// 1) 备份：停服要求、快照一致性、blob 完整性、不覆盖
// ---------------------------------------------------------------------------

#[tokio::test]
async fn backup_requires_stopped_server_and_produces_consistent_snapshot() {
    let (chain, release, _tripo, _manual) = published_chain("t20-backup").await;
    let data_dir = chain.app.dir().to_path_buf();
    let work = TestDir::new("t20-backup-cli");
    let backup_dir = work.join("backup-out");

    // 造一条真实会话（备份/快照不得包含会话）。
    let admin_id = {
        let mut conn = chain.app.state().database().pool().acquire().await.unwrap();
        repo::admins::get_single(&mut conn)
            .await
            .unwrap()
            .expect("管理员已创建")
            .id
    };
    let (_cookie, _csrf, _session_id) = chain.app.insert_session(&admin_id, 3_600_000).await;
    let sessions_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool(&chain))
        .await
        .unwrap();
    assert!(sessions_before >= 1, "用例前提：存在会话");

    // 源数据指纹（备份不得修改源 data-dir 的数据）。
    let source_db_before = fingerprint(&data_dir.join("manual.sqlite3"));
    let source_blob_shas: Vec<String> =
        sqlx::query_scalar::<_, String>("SELECT sha256 FROM blobs ORDER BY sha256")
            .fetch_all(&pool(&chain))
            .await
            .unwrap();
    let source_blob_hashes_before: Vec<(String, u64)> = source_blob_shas
        .iter()
        .map(|sha| fingerprint(&data_dir.join("blobs").join(&sha[..2]).join(sha)))
        .collect();

    // 阶段 1：真实 serve 持有排他锁 → backup 拒绝并要求先停服（退出码 5）。
    let server = ServeProcess::start(work.path(), &data_dir);
    let refused = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--out",
            backup_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(refused.status, 5, "{refused:?}");
    assert!(
        refused.stderr.contains("停止服务") || refused.stderr.contains("停服"),
        "必须说明需先停服：{refused:?}"
    );
    assert!(!backup_dir.exists(), "被拒绝时不得创建输出目录");

    // 阶段 2：停服后备份成功。
    let (exit, stdout, stderr) = server.terminate();
    assert_eq!(
        exit, 0,
        "serve 应优雅退出；stdout：{stdout}\nstderr：{stderr}"
    );
    let ok = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--out",
            backup_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(ok.status, 0, "{ok:?}");
    assert!(ok.stdout.contains("备份完成"), "{ok:?}");

    // 产物：快照 + 每个被引用 blob + manifest（相对路径；无 -wal/-shm 边车）。
    let manifest = read_backup_manifest(&backup_dir);
    assert_eq!(manifest.schema_version, BACKUP_SCHEMA_VERSION);
    assert_eq!(manifest.database.path, "database/manual.sqlite3");
    assert!(
        manifest.database.sessions_removed,
        "备份必须清空会话（REQ-005/AC-010）"
    );
    assert_eq!(
        manifest.database.sessions_removed_count, sessions_before,
        "清空的会话行数与备份前的库一致"
    );
    assert_eq!(
        manifest.counts.releases, 1,
        "发布版本数进入 manifest（用于恢复后核对）"
    );
    let snapshot_path = backup_file(&backup_dir, &manifest.database.path);
    let (snapshot_sha, snapshot_size) = fingerprint(&snapshot_path);
    assert_eq!(snapshot_sha, manifest.database.sha256);
    assert_eq!(snapshot_size, manifest.database.size);
    assert!(
        !backup_dir.join("database/manual.sqlite3-wal").exists()
            && !backup_dir.join("database/manual.sqlite3-shm").exists(),
        "快照必须是单文件（无 WAL 边车）"
    );
    let mut manifest_shas: Vec<String> = manifest
        .blobs
        .iter()
        .map(|blob| blob.sha256.clone())
        .collect();
    manifest_shas.sort();
    let mut source_shas = source_blob_shas.clone();
    source_shas.sort();
    assert_eq!(manifest_shas, source_shas, "备份必须含全部被引用 blob");
    for blob in &manifest.blobs {
        let path = backup_file(&backup_dir, &blob.path);
        let (sha, size) = fingerprint(&path);
        assert_eq!(sha, blob.sha256, "blob {} 内容完整", blob.sha256);
        assert_eq!(size, blob.size);
    }

    // SHA256SUMS：标准工具（`shasum -a 256 -c`）可直接校验整个备份。
    let sums =
        std::fs::read_to_string(backup_dir.join("SHA256SUMS")).expect("备份必须带 SHA256SUMS");
    assert_eq!(
        sums.lines().count(),
        manifest.blobs.len() + 2,
        "SHA256SUMS 应覆盖快照、全部 blob 与 manifest：{sums}"
    );
    for line in sums.lines() {
        let (sha, path) = line
            .split_once("  ")
            .expect("SHA256SUMS 行格式：<sha>  <path>");
        assert_eq!(path, path.trim_start_matches("./"));
        assert!(!path.starts_with('/') && !path.contains(".."), "{path}");
        let (actual, _) = fingerprint(&backup_file(&backup_dir, path));
        assert_eq!(actual, sha, "SHA256SUMS 与实际文件不符：{path}");
    }

    // 快照是可用的 SQLite 数据库：会话为空、外键完整、发布版本仍在。
    let snapshot_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&snapshot_path)
                .read_only(true),
        )
        .await
        .expect("快照可打开");
    let sessions_in_snapshot: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&snapshot_pool)
        .await
        .unwrap();
    assert_eq!(sessions_in_snapshot, 0, "快照不得包含任何会话");
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&snapshot_pool)
        .await
        .unwrap();
    assert!(violations.is_empty(), "快照外键必须完整");
    let release_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM manual_releases")
        .fetch_one(&snapshot_pool)
        .await
        .unwrap();
    assert_eq!(release_rows, 1);
    snapshot_pool.close().await;

    // 源 data-dir 未被备份修改。
    assert_eq!(
        fingerprint(&data_dir.join("manual.sqlite3")),
        source_db_before,
        "备份不得修改源数据库主文件"
    );
    for (index, sha) in source_blob_shas.iter().enumerate() {
        assert_eq!(
            fingerprint(&data_dir.join("blobs").join(&sha[..2]).join(sha)),
            source_blob_hashes_before[index],
            "备份不得修改源 blob"
        );
    }
    assert!(!release.release_id.is_empty());

    // 不覆盖：同一输出路径再备份一次必须被拒（现有产物逐字节不变）。
    let manifest_before = std::fs::read(backup_dir.join("manifest.json")).unwrap();
    let again = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--out",
            backup_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(again.status, 4, "{again:?}");
    assert!(again.stderr.contains("已存在"), "{again:?}");
    assert_eq!(
        std::fs::read(backup_dir.join("manifest.json")).unwrap(),
        manifest_before,
        "被拒的备份不得改动已有备份"
    );

    // 嵌套路径（输出在 data-dir 内）被拒（退出码 4），避免"备份跟着数据一起丢"。
    let nested = data_dir.join("backup-inside");
    let nested_out = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--out",
            nested.to_str().unwrap(),
        ],
    );
    assert_eq!(nested_out.status, 4, "{nested_out:?}");
    assert!(!nested.exists());
}

// ---------------------------------------------------------------------------
// 1b) T20/BUG-008：供应商临时/签名 URL 不进入任务元数据、备份快照与导出包
// ---------------------------------------------------------------------------

/// 打开（只读）一个 data-dir / 快照数据库。
async fn open_readonly(path: &Path) -> SqlitePool {
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(path)
                .read_only(true),
        )
        .await
        .expect("数据库可只读打开")
}

/// 查询供应商事实表（`job_stages` / `provider_attempts`）里命中 `needle` 的行数。
///
/// 与 A-010/AC-010 的复核口径一致：这两张表按合同只存系统/供应商事实，
/// 不允许出现临时 URL 的完整子串（含签名查询串、scheme 与 path）。
async fn provider_fact_hits(pool: &SqlitePool, needle: &str) -> i64 {
    let stages: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM job_stages WHERE \
         instr(COALESCE(CAST(usage_json AS TEXT), ''), ?) > 0 \
         OR instr(COALESCE(CAST(needs_input_json AS TEXT), ''), ?) > 0 \
         OR instr(COALESCE(last_error, ''), ?) > 0",
    )
    .bind(needle)
    .bind(needle)
    .bind(needle)
    .fetch_one(pool)
    .await
    .expect("扫描 job_stages");
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_attempts WHERE \
         instr(COALESCE(last_error, ''), ?) > 0 \
         OR instr(COALESCE(remote_task_id, ''), ?) > 0 \
         OR instr(COALESCE(response_id, ''), ?) > 0",
    )
    .bind(needle)
    .bind(needle)
    .bind(needle)
    .fetch_one(pool)
    .await
    .expect("扫描 provider_attempts");
    stages + attempts
}

/// 断言：库里没有临时 URL 的完整子串（scheme/签名/path 都不在），但任务 ID
/// 与「摘要 + host」仍在（诊断与恢复判据不丢）。
async fn assert_no_temporary_urls(pool: &SqlitePool, context: &str) {
    for needle in [
        "://",
        "sign=canary-historical",
        "/preview.png",
        "/model.glb",
    ] {
        assert_eq!(
            provider_fact_hits(pool, needle).await,
            0,
            "{context}：供应商事实不得含 {needle:?}"
        );
    }
}

/// 新写入的任务元数据不含临时/签名 URL（T20/BUG-008 的写入侧断言）。
#[tokio::test]
async fn fresh_pipeline_metadata_contains_no_temporary_urls() {
    let (chain, _release, _tripo, _manual) = published_chain("t20-url-fresh").await;
    let live = pool(&chain);

    // 新事实：全链路跑完后，供应商事实表里 0 处 URL 子串。
    assert_no_temporary_urls(&live, "新产生的任务元数据").await;

    // 但可诊断事实保留：task ID、状态、计费与链接摘要（host + sha256 前缀）。
    let usage: String = sqlx::query_scalar(
        "SELECT CAST(usage_json AS TEXT) FROM job_stages WHERE stage_kind = 'tripo_poll'",
    )
    .fetch_one(&live)
    .await
    .expect("查询阶段事实");
    let usage: Value = serde_json::from_str(&usage).expect("usage 是 JSON");
    assert_eq!(usage["remoteTaskId"], TASK_ID);
    assert_eq!(usage["normalizedStatus"], "success");
    assert_eq!(usage["billing"]["creditMinor"], 3000);
    assert_eq!(usage["modelUrl"]["redacted"], true, "{usage}");
    assert_eq!(
        usage["modelUrl"]["host"], "127.0.0.1",
        "host 是允许保留的最小诊断信息"
    );
    assert_eq!(
        usage["renderedImageUrl"]["redacted"], true,
        "预览图地址（{TEMP_CLOUD_URL}）同样是临时链接：{usage}"
    );
    assert_eq!(usage["renderedImageUrl"]["host"], TEMP_CLOUD_URL, "{usage}");
    assert_eq!(
        usage["modelUrl"]["sha256"].as_str().unwrap().len(),
        16,
        "摘要 = sha256 前 16 位：{usage}"
    );
}

/// 历史数据（修复前落库的 URL）：**不强制迁移源库**；备份快照与恢复结果都不含
/// 临时 URL，恢复后仍可按 task_id 继续查询（AC-010 + 恢复语义）。
#[tokio::test]
async fn backup_redacts_historical_provider_urls_without_touching_source() {
    let (chain, release, _tripo, _manual) = published_chain("t20-url-scrub").await;
    let data_dir = chain.app.dir().to_path_buf();
    let work = TestDir::new("t20-url-scrub-cli");
    let backup_dir = work.join("backup-out");
    let live = pool(&chain);

    // 造历史现场：修复前落库的 usage_json（完整 modelUrl + renderedImageUrl）
    // 与 provider_attempts.last_error（非 JSON 文本，含同一签名串）。
    let historical_url =
        format!("https://{TEMP_CLOUD_URL}/fixture/model.glb?sign=canary-historical");
    let historical_usage = json!({
        "remoteTaskId": TASK_ID,
        "rawStatus": "success",
        "normalizedStatus": "success",
        "modelUrl": historical_url,
        "renderedImageUrl": format!("https://{TEMP_CLOUD_URL}/preview.png"),
        "billing": { "creditMinor": 3000, "literal": "30", "sourceField": "credits_consumed" },
    });
    let updated =
        sqlx::query("UPDATE job_stages SET usage_json = ? WHERE stage_kind = 'tripo_poll'")
            .bind(historical_usage.to_string())
            .execute(&live)
            .await
            .unwrap();
    assert_eq!(updated.rows_affected(), 1, "用例前提：存在 tripo_poll 阶段");
    let updated =
        sqlx::query("UPDATE provider_attempts SET last_error = ? WHERE remote_task_id = ?")
            .bind(format!("查询远端任务失败：{historical_url} 已过期"))
            .bind(TASK_ID)
            .execute(&live)
            .await
            .unwrap();
    assert_eq!(
        updated.rows_affected(),
        1,
        "用例前提：存在 accepted attempt"
    );
    assert!(
        provider_fact_hits(&live, "://").await > 0,
        "用例前提：源库确实存在历史 URL（未被预先迁移）"
    );
    let source_db_before = fingerprint(&data_dir.join("manual.sqlite3"));

    // 备份：退出 0，stdout 报告脱敏处数。
    let ok = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--out",
            backup_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(ok.status, 0, "{ok:?}");
    assert!(
        ok.stdout.contains("临时 URL 脱敏"),
        "备份输出必须报告脱敏：{ok:?}"
    );

    // 快照：0 处 URL 子串；摘要与 task ID 在。
    let snapshot_path = backup_dir.join("database").join("manual.sqlite3");
    let snapshot = open_readonly(&snapshot_path).await;
    assert_no_temporary_urls(&snapshot, "备份快照").await;
    let scrubbed: String = sqlx::query_scalar(
        "SELECT CAST(usage_json AS TEXT) FROM job_stages WHERE stage_kind = 'tripo_poll'",
    )
    .fetch_one(&snapshot)
    .await
    .expect("快照内查询阶段事实");
    let scrubbed: Value = serde_json::from_str(&scrubbed).expect("usage 是 JSON");
    assert_eq!(scrubbed["remoteTaskId"], TASK_ID, "恢复判据（task ID）保留");
    assert_eq!(scrubbed["modelUrl"]["redacted"], true, "{scrubbed}");
    assert_eq!(scrubbed["modelUrl"]["host"], TEMP_CLOUD_URL);
    assert_eq!(scrubbed["billing"]["creditMinor"], 3000, "计费事实保留");
    let attempt_error: String = sqlx::query_scalar(
        "SELECT COALESCE(last_error, '') FROM provider_attempts WHERE remote_task_id = ?",
    )
    .bind(TASK_ID)
    .fetch_one(&snapshot)
    .await
    .expect("快照内 attempt");
    assert!(
        attempt_error.contains("sha256=") && !attempt_error.contains("://"),
        "非 JSON 文本列也要兜底脱敏：{attempt_error}"
    );
    snapshot.close().await;

    // 源 data-dir 不被迁移、不被修改（历史数据可用性不受影响）。
    assert!(
        provider_fact_hits(&live, "://").await > 0,
        "源库不被强制迁移（读取/备份时脱敏）"
    );
    assert_eq!(
        fingerprint(&data_dir.join("manual.sqlite3")),
        source_db_before,
        "备份不得修改源数据库"
    );

    // 恢复：退出 0；恢复后的库同样干净，且 task ID/摘要仍在。
    let restored = work.join("restored");
    let restored_out = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            restored.to_str().unwrap(),
        ],
    );
    assert_eq!(restored_out.status, 0, "{restored_out:?}");
    let restored_pool = open_readonly(&restored.join("manual.sqlite3")).await;
    assert_no_temporary_urls(&restored_pool, "恢复后的库").await;
    let restored_task: String = sqlx::query_scalar(
        "SELECT json_extract(CAST(usage_json AS TEXT), '$.remoteTaskId') FROM job_stages \
         WHERE stage_kind = 'tripo_poll'",
    )
    .fetch_one(&restored_pool)
    .await
    .expect("恢复后仍可按 task_id 重新查询链接");
    assert_eq!(restored_task, TASK_ID);
    restored_pool.close().await;

    // 导出（AC-058 对照）：历史行存在时导出包同样不含该临时 URL。
    let response = chain
        .app
        .call(
            Method::GET,
            &format!("/api/v1/releases/{}/export", release.release_id),
        )
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let entries = parse_stored_zip(&response.body).expect("导出包必须是合法 ZIP");
    for (name, bytes) in &entries {
        let text = String::from_utf8_lossy(bytes);
        assert!(
            !text.contains("canary-historical") && !text.contains(TEMP_CLOUD_URL),
            "导出包条目 {name} 不得含供应商临时 URL"
        );
    }
}

/// BUG-011（回合 27）：备份/恢复对 `needs_input_json` 的**句子型 message** 只替换
/// URL 片段——整句说明与同列其它条目原样保留，恢复后仍是可反序列化的缺项列表
/// （修复前整串被替换为摘要对象 → `needsInput` 静默为空）。
#[tokio::test]
async fn backup_and_restore_keep_needs_input_sentences_intact() {
    let (chain, _release, _tripo, _manual) = published_chain("t29-needs-sentence").await;
    let data_dir = chain.app.dir().to_path_buf();
    let work = TestDir::new("t29-needs-sentence-cli");
    let backup_dir = work.join("backup-out");
    let live = pool(&chain);

    // 产品可达形态：`DownloadError::InsecureScheme` 的 message 含 `http://host`；
    // 同列再放一条无 URL 的条目（整列不得因一条命中而丢失）。
    let message = "模型下载必须使用 HTTPS（实际 http://cdn.example.invalid/m.glb）：拒绝下载";
    let needs = json!([
        {"code": "download_insecure_scheme", "message": message},
        {"code": "retry", "message": "下载可安全重试（task-9）"}
    ]);
    let updated = sqlx::query(
        "UPDATE job_stages SET status = 'needs_input', needs_input_json = ? \
          WHERE stage_kind = 'model_download'",
    )
    .bind(needs.to_string())
    .execute(&live)
    .await
    .unwrap();
    assert_eq!(
        updated.rows_affected(),
        1,
        "用例前提：存在 model_download 阶段"
    );

    let ok = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--out",
            backup_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(ok.status, 0, "{ok:?}");

    // 快照：JSON 合法、message 仍是字符串、句子只丢 URL 片段、同列其它条目在。
    let snapshot_path = backup_dir.join("database").join("manual.sqlite3");
    let snapshot = open_readonly(&snapshot_path).await;
    let stored: String = sqlx::query_scalar(
        "SELECT CAST(needs_input_json AS TEXT) FROM job_stages WHERE stage_kind = 'model_download'",
    )
    .fetch_one(&snapshot)
    .await
    .expect("快照内查询缺项");
    let parsed: Value = serde_json::from_str(&stored).expect("快照内仍是合法 JSON");
    let items = parsed.as_array().expect("仍是列表");
    assert_eq!(items.len(), 2, "同列其它条目不得丢失：{parsed}");
    let scrubbed = items[0]["message"]
        .as_str()
        .expect("message 仍是字符串（BUG-011：不得整串变摘要对象）");
    assert!(!scrubbed.contains("://"), "{scrubbed}");
    assert!(
        scrubbed.starts_with("模型下载必须使用 HTTPS（实际 "),
        "URL 之前的文本必须保留：{scrubbed}"
    );
    assert!(
        scrubbed.ends_with("）：拒绝下载"),
        "URL 之后的文本必须保留：{scrubbed}"
    );
    assert!(scrubbed.contains("host=cdn.example.invalid"), "{scrubbed}");
    assert_eq!(items[0]["code"], "download_insecure_scheme", "code 不变");
    assert_eq!(items[1]["message"], "下载可安全重试（task-9）");
    snapshot.close().await;

    // 恢复：整列仍能被读取侧按类型解析（模拟 `stage_dto` 的缺项解析）。
    let restored = work.join("restored");
    let restored_out = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            restored.to_str().unwrap(),
        ],
    );
    assert_eq!(restored_out.status, 0, "{restored_out:?}");
    let restored_pool = open_readonly(&restored.join("manual.sqlite3")).await;
    let stored: String = sqlx::query_scalar(
        "SELECT CAST(needs_input_json AS TEXT) FROM job_stages WHERE stage_kind = 'model_download'",
    )
    .fetch_one(&restored_pool)
    .await
    .expect("恢复库内查询缺项");
    let restored_items: Vec<Value> =
        serde_json::from_str(&stored).expect("恢复库仍是可解析的缺项列表");
    assert_eq!(restored_items.len(), 2, "恢复后缺项不得静默消失");
    assert!(
        restored_items[0]["message"]
            .as_str()
            .expect("message 是字符串")
            .ends_with("）：拒绝下载"),
        "{restored_items:?}"
    );
    restored_pool.close().await;
}

// ---------------------------------------------------------------------------
// 2) 备份：源 blob 损坏 → 完整性失败（退出码 7），不修改源数据
// ---------------------------------------------------------------------------

#[tokio::test]
async fn backup_fails_on_corrupted_source_blob_without_touching_source() {
    let (chain, _release, _tripo, _manual) = published_chain("t20-backup-corrupt").await;
    let data_dir = chain.app.dir().to_path_buf();
    let work = TestDir::new("t20-backup-corrupt-cli");

    // 选一个被引用的 blob 翻转一个字节（模拟磁盘损坏）。
    let victim: String = sqlx::query_scalar("SELECT sha256 FROM blobs ORDER BY sha256 LIMIT 1")
        .fetch_one(&pool(&chain))
        .await
        .unwrap();
    let victim_path = data_dir.join("blobs").join(&victim[..2]).join(&victim);
    let mut bytes = std::fs::read(&victim_path).unwrap();
    bytes[0] ^= 0xFF;
    std::fs::write(&victim_path, &bytes).unwrap();
    let corrupted_fingerprint = fingerprint(&victim_path);

    let backup_dir = work.join("backup-out");
    let out = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--out",
            backup_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status, 7, "损坏 blob → 完整性失败（退出码 7）：{out:?}");
    assert!(
        out.stderr.contains("sha256") || out.stderr.contains("不符"),
        "{out:?}"
    );
    assert!(
        !backup_dir.join("manifest.json").exists(),
        "失败不得写出 manifest"
    );
    assert_eq!(
        fingerprint(&victim_path),
        corrupted_fingerprint,
        "失败不得改动源文件（保留现场）"
    );
}

// ---------------------------------------------------------------------------
// 3) 恢复：目标前置条件、损坏备份拒绝、schema 门禁
// ---------------------------------------------------------------------------

#[tokio::test]
async fn restore_verifies_backup_and_rejects_damage_without_creating_target() {
    let (chain, _release, _tripo, _manual) = published_chain("t20-restore-verify").await;
    let data_dir = chain.app.dir().to_path_buf();
    let work = TestDir::new("t20-restore-verify-cli");
    let backup_dir = work.join("backup-out");
    let ok = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--out",
            backup_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(ok.status, 0, "{ok:?}");

    // (0) 目标"存在但为空"是合法的（合同：不存在**或为空**）。
    let empty_target = work.join("empty-target");
    std::fs::create_dir_all(&empty_target).unwrap();
    let into_empty = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            empty_target.to_str().unwrap(),
        ],
    );
    assert_eq!(into_empty.status, 0, "{into_empty:?}");
    assert!(empty_target.join("manual.sqlite3").is_file());
    assert!(empty_target.join("blobs").is_dir());

    // (a) 目标已存在且非空 → 拒绝（退出码 4），现场不动。
    let nonempty = work.join("nonempty");
    std::fs::create_dir_all(&nonempty).unwrap();
    std::fs::write(nonempty.join("keep.txt"), "keep").unwrap();
    let refused = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            nonempty.to_str().unwrap(),
        ],
    );
    assert_eq!(refused.status, 4, "{refused:?}");
    assert!(refused.stderr.contains("非空"), "{refused:?}");
    assert_eq!(
        std::fs::read_to_string(nonempty.join("keep.txt")).unwrap(),
        "keep"
    );
    assert!(
        !nonempty.join("manual.sqlite3").exists(),
        "被拒的恢复不得写入数据库"
    );

    // (b) 目标是一个文件 → 拒绝。
    let file_target = work.join("target-file");
    std::fs::write(&file_target, "not a dir").unwrap();
    let refused_file = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            file_target.to_str().unwrap(),
        ],
    );
    assert_eq!(refused_file.status, 4, "{refused_file:?}");

    // (b2) 备份里的 blob 是符号链接（人为改动的备份）→ 拒绝，不把链接指向的内容
    //      复制进恢复结果（与将来 ZIP 导入的 zip-slip 同一思路）。
    let manifest = read_backup_manifest(&backup_dir);
    let linked = manifest.blobs.last().expect("备份至少一个 blob").clone();
    let linked_path = backup_file(&backup_dir, &linked.path);
    let linked_bytes = std::fs::read(&linked_path).unwrap();
    std::fs::remove_file(&linked_path).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(backup_dir.join("manifest.json"), &linked_path).unwrap();
    let target = work.join("restored-symlink-blob");
    let symlink_out = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            target.to_str().unwrap(),
        ],
    );
    #[cfg(unix)]
    {
        assert_eq!(symlink_out.status, 7, "{symlink_out:?}");
        assert!(symlink_out.stderr.contains("符号链接"), "{symlink_out:?}");
        assert!(!target.exists(), "校验失败时不得创建目标目录");
    }
    #[cfg(unix)]
    {
        std::fs::remove_file(&linked_path).unwrap();
        std::fs::write(&linked_path, &linked_bytes).unwrap();
    }

    // (c) 备份 blob 损坏 → 退出码 7，目标目录不被创建。
    let victim = manifest.blobs.first().expect("备份至少一个 blob").clone();
    let victim_path = backup_file(&backup_dir, &victim.path);
    let mut bytes = std::fs::read(&victim_path).unwrap();
    bytes[0] ^= 0xFF;
    std::fs::write(&victim_path, &bytes).unwrap();
    let target = work.join("restored-corrupt-blob");
    let corrupt = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            target.to_str().unwrap(),
        ],
    );
    assert_eq!(corrupt.status, 7, "{corrupt:?}");
    assert!(
        corrupt.stderr.contains(&victim.sha256),
        "{}",
        corrupt.stderr
    );
    assert!(!target.exists(), "校验失败时不得创建目标目录（保留现场）");

    // (d) 快照损坏（hash 不符）→ 退出码 7，目标不被创建。
    let mut snapshot_bytes =
        std::fs::read(backup_file(&backup_dir, &manifest.database.path)).unwrap();
    let last = snapshot_bytes.len() - 1;
    snapshot_bytes[last] ^= 0xFF;
    std::fs::write(
        backup_file(&backup_dir, &manifest.database.path),
        &snapshot_bytes,
    )
    .unwrap();
    let target = work.join("restored-corrupt-db");
    let corrupt_db = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            target.to_str().unwrap(),
        ],
    );
    assert_eq!(corrupt_db.status, 7, "{corrupt_db:?}");
    assert!(!target.exists(), "校验失败时不得创建目标目录");

    // (e) manifest 缺失 → 退出码 7。
    std::fs::write(
        backup_file(&backup_dir, &manifest.database.path),
        &snapshot_bytes, // 修复快照的字节损坏（该用例只验证 manifest 缺失）
    )
    .unwrap();
    std::fs::remove_file(backup_dir.join("manifest.json")).unwrap();
    let target = work.join("restored-no-manifest");
    let no_manifest = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            target.to_str().unwrap(),
        ],
    );
    assert_eq!(no_manifest.status, 7, "{no_manifest:?}");
    assert!(!target.exists());

    // 备份来源不存在 → 退出码 4。
    let missing = run(
        work.path(),
        &[
            "restore",
            "--from",
            work.join("no-such-backup").to_str().unwrap(),
            "--data-dir",
            work.join("target-missing").to_str().unwrap(),
        ],
    );
    assert_eq!(missing.status, 4, "{missing:?}");
}

/// 打开备份快照（DELETE 日志模式）、执行 SQL、关闭，并刷新 manifest 的 sha256/大小。
async fn rewrite_snapshot_and_manifest(backup_dir: &Path, sql: &'static str) {
    let manifest_path = backup_dir.join("manifest.json");
    let mut manifest = read_backup_manifest(backup_dir);
    let snapshot_path = backup_file(backup_dir, &manifest.database.path);

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&snapshot_path)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete),
        )
        .await
        .expect("打开快照");
    sqlx::query(sql).execute(&pool).await.expect("执行改写 SQL");
    pool.close().await;

    let (sha256, size) = fingerprint(&snapshot_path);
    manifest.database.sha256 = sha256;
    manifest.database.size = size;
    std::fs::write(&manifest_path, manifest.to_bytes().unwrap()).unwrap();
}

#[tokio::test]
async fn restore_rejects_backup_with_newer_schema() {
    let (chain, _release, _tripo, _manual) = published_chain("t20-restore-newer").await;
    let data_dir = chain.app.dir().to_path_buf();
    let work = TestDir::new("t20-restore-newer-cli");
    let backup_dir = work.join("backup-out");
    let ok = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--out",
            backup_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(ok.status, 0, "{ok:?}");

    // 构造"比程序更新的 schema"：插入一条更高版本的迁移记录（模拟新程序升级过）。
    rewrite_snapshot_and_manifest(
        &backup_dir,
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time) \
         VALUES (99, 'future migration', 0, 1, X'00', 0)",
    )
    .await;

    let target = work.join("restored");
    let refused = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            target.to_str().unwrap(),
        ],
    );
    assert_eq!(
        refused.status, 4,
        "比程序新的 schema 拒绝恢复（与 check/serve 同一语义）：{refused:?}"
    );
    assert!(refused.stderr.contains("schema"), "{refused:?}");
    assert!(!target.exists(), "拒绝时不得创建目标目录");
}

// ---------------------------------------------------------------------------
// 4) 恢复：新空目录 → 真实起服务读取同一 release（PDF 与 GLB）
// ---------------------------------------------------------------------------

/// 真实 HTTP 客户端（reqwest 已是生产依赖；这里只访问本机回环）。
struct Http {
    client: reqwest::Client,
    base: String,
    cookie: String,
}

impl Http {
    async fn login(base: &str, password: &str) -> Self {
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{base}/api/v1/auth/login"))
            .json(&json!({ "password": password }))
            .send()
            .await
            .expect("登录请求");
        assert_eq!(response.status(), 200, "登录应成功（口令哈希随备份恢复）");
        let raw_cookie = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with("em_session="))
            .expect("登录返回会话 cookie")
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        Self {
            client,
            base: base.to_owned(),
            cookie: raw_cookie,
        }
    }

    async fn get(&self, path: &str) -> (u16, Vec<u8>, String) {
        let response = self
            .client
            .get(format!("{}{path}", self.base))
            .header("cookie", &self.cookie)
            .send()
            .await
            .expect("GET 请求");
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = response.bytes().await.expect("响应体").to_vec();
        (status, bytes, content_type)
    }

    async fn get_json(&self, path: &str) -> Value {
        let (status, bytes, _) = self.get(path).await;
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
        serde_json::from_slice(&bytes).expect("JSON 响应")
    }
}

#[tokio::test]
async fn restore_into_new_directory_reproduces_readable_release_pdf_and_glb() {
    let (chain, release, _tripo, _manual) = published_chain("t20-restore-e2e").await;
    let data_dir = chain.app.dir().to_path_buf();
    let work = TestDir::new("t20-restore-e2e-cli");
    let backup_dir = work.join("backup-out");

    // 备份前留痕：源目录的模型/原件字节（恢复后必须逐字节一致）。
    let model_blob = data_dir
        .join("blobs")
        .join(&release.model_sha256[..2])
        .join(&release.model_sha256);
    let document_blob = data_dir
        .join("blobs")
        .join(&release.document_sha256[..2])
        .join(&release.document_sha256);
    let model_bytes = std::fs::read(&model_blob).unwrap();
    let document_bytes = std::fs::read(&document_blob).unwrap();

    let ok = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--out",
            backup_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(ok.status, 0, "{ok:?}");

    // 恢复到一个不存在的新目录。
    let restored = work.join("restored-data");
    let restored_out = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            restored.to_str().unwrap(),
        ],
    );
    assert_eq!(restored_out.status, 0, "{restored_out:?}");
    assert!(restored_out.stdout.contains("恢复完成"), "{restored_out:?}");
    for sub in ["tmp", "logs", "blobs"] {
        assert!(restored.join(sub).is_dir(), "恢复后必须有 {sub}/");
    }
    assert!(restored.join("lock").is_file());
    assert!(restored.join("manual.sqlite3").is_file());

    // 真实起服务读取：同一 release、PDF、GLB 均可读（含字节比对）。
    let server = ServeProcess::start(work.path(), &restored);
    let http = Http::login(&server.base_url(), PASSWORD).await;
    let detail = http
        .get_json(&format!(
            "/api/v1/items/{}/releases/{}",
            release.item_id, release.release_id
        ))
        .await;
    assert_eq!(
        detail["data"]["manifestSha256"], release.manifest_sha256,
        "恢复后 release manifest 哈希与发布时一致"
    );
    assert_eq!(detail["data"]["id"], release.release_id);

    let (status, pdf_bytes, content_type) = http
        .get(&format!(
            "/api/v1/assets/{}/content",
            release.document_asset_id
        ))
        .await;
    assert_eq!(status, 200);
    assert_eq!(content_type, "application/pdf");
    assert_eq!(sha256_hex(&pdf_bytes), release.document_sha256);
    assert_eq!(pdf_bytes, document_bytes);
    assert!(pdf_bytes.starts_with(b"%PDF-"), "PDF 可打开（头部合法）");

    let (status, glb_bytes, content_type) = http
        .get(&format!(
            "/api/v1/assets/{}/content",
            release.model_asset_id
        ))
        .await;
    assert_eq!(status, 200);
    // T06 的响应头白名单（`alias_json_mime`）不含 model/gltf-binary，GLB 以
    // application/octet-stream 提供（T18 的阅读器按字节加载，不依赖该头）。
    // 见 implementation.md §T20 的已知观察（不改 T06 已验收行为）。
    assert_eq!(content_type, "application/octet-stream");
    assert_eq!(sha256_hex(&glb_bytes), release.model_sha256);
    assert_eq!(glb_bytes, model_bytes);
    assert_eq!(&glb_bytes[0..4], b"glTF", "GLB 可打开（magic 合法）");
    // GLB 声明长度与文件一致（结构完整，不是截断文件）。
    let declared = u32::from_le_bytes([glb_bytes[8], glb_bytes[9], glb_bytes[10], glb_bytes[11]]);
    assert_eq!(declared as usize, glb_bytes.len());

    let (exit, stdout, stderr) = server.terminate();
    assert_eq!(exit, 0);
    assert!(!stderr.contains("panicked"), "serve 不得 panic：{stderr}");
    assert!(
        !stdout.contains("schema 已自动升级"),
        "同版本恢复不应触发迁移"
    );
}

// ---------------------------------------------------------------------------
// 5) 导出：只含该 release 有权资产；无密钥/会话/绝对路径/临时云端 URL
// ---------------------------------------------------------------------------

#[tokio::test]
async fn export_contains_only_release_assets_and_no_secrets() {
    let (chain, release, _tripo, _manual) = published_chain("t20-export").await;

    // 同一物品的另一个资产（不同字节、未被 release 引用）：导出包不得包含它。
    let other_asset_bytes = fixture_bytes("sample-manual-rotated.pdf");
    let other_asset = upload_asset(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs.item,
        "document",
        "other.pdf",
        "application/pdf",
        &other_asset_bytes,
    )
    .await;
    let other_sha = sha256_hex(&other_asset_bytes);
    assert!(!other_asset.is_empty());

    // 会话 token（导出包不得包含）。
    let (session_cookie, _csrf, _session_id) = {
        let mut conn = chain.app.state().database().pool().acquire().await.unwrap();
        let admin = repo::admins::get_single(&mut conn).await.unwrap().unwrap();
        drop(conn);
        chain.app.insert_session(&admin.id, 3_600_000).await
    };
    let session_token = session_cookie
        .strip_prefix("em_session=")
        .expect("会话 cookie 形如 em_session=...")
        .to_owned();

    // 导出（经真实路由 + 会话）。
    let response = chain
        .app
        .call(
            Method::GET,
            &format!("/api/v1/releases/{}/export", release.release_id),
        )
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.header("content-type").as_deref(),
        Some("application/zip")
    );
    let disposition = response.header("content-disposition").unwrap_or_default();
    assert!(disposition.starts_with("attachment"), "{disposition}");
    assert!(disposition.contains(".zip"), "{disposition}");

    let entries = parse_stored_zip(&response.body).expect("导出包必须是合法 ZIP（含 CRC 校验）");
    let names: Vec<String> = entries.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(
        names,
        vec![
            "manifest.json".to_owned(),
            "release/manifest.json".to_owned(),
            format!("assets/model/{}.glb", release.model_sha256),
            format!("assets/document/{}.pdf", release.document_sha256),
        ],
        "只允许导出清单、冻结 manifest 与两个 release 资产"
    );

    let find = |name: &str| -> Vec<u8> {
        entries
            .iter()
            .find(|(entry, _)| entry == name)
            .map(|(_, data)| data.clone())
            .unwrap_or_else(|| panic!("缺少条目 {name}"))
    };
    let export_manifest: Value =
        serde_json::from_slice(&find("manifest.json")).expect("导出清单是 JSON");
    assert_eq!(export_manifest["schemaVersion"], "manual_release_export_v1");
    assert_eq!(export_manifest["release"]["releaseId"], release.release_id);
    assert_eq!(export_manifest["item"]["id"], release.item_id);
    assert_eq!(export_manifest["item"]["model"], "X100V");
    assert!(
        export_manifest["knowledge"].is_object(),
        "导出清单必须带冻结知识"
    );
    assert_eq!(
        export_manifest["releaseManifest"]["sha256"],
        release.manifest_sha256
    );
    let files = export_manifest["files"].as_array().expect("files[]");
    assert_eq!(files.len(), 2, "只导出 model 与 document");
    for file in files {
        let path = file["path"].as_str().unwrap();
        assert!(path.starts_with("assets/"), "{path}");
        assert!(!path.starts_with('/') && !path.contains(".."), "{path}");
        assert!(file["sha256"].as_str().unwrap().len() == 64);
        assert!(file["source"].as_str().is_some(), "必须记录来源");
    }

    // 冻结 manifest 字节原样（sha256 与发布详情一致）。
    let frozen = find("release/manifest.json");
    assert_eq!(sha256_hex(&frozen), release.manifest_sha256);
    // 资产字节与发布时一致。
    assert_eq!(
        sha256_hex(&find(&format!("assets/model/{}.glb", release.model_sha256))),
        release.model_sha256
    );
    assert_eq!(
        sha256_hex(&find(&format!(
            "assets/document/{}.pdf",
            release.document_sha256
        ))),
        release.document_sha256
    );

    // 内容扫描：不含密钥（canary）、会话 token、绝对路径、临时云端 URL；
    // 其它资产（本用例的"其它物品/角色"照片）字节不在包内。
    let data_dir_text = chain.app.dir().to_string_lossy().into_owned();
    let all_bytes: Vec<u8> = response.body.clone();
    let all_text = String::from_utf8_lossy(&all_bytes).into_owned();
    for forbidden in [
        CANARY_KEY,
        session_token.as_str(),
        data_dir_text.as_str(),
        TEMP_CLOUD_URL,
        "openapi.tripo3d.ai",
        "Bearer ",
    ] {
        assert!(
            !all_text.contains(forbidden),
            "导出包不得包含 {forbidden:?}"
        );
    }
    assert!(
        !all_bytes
            .windows(other_asset_bytes.len())
            .any(|window| window == other_asset_bytes.as_slice()),
        "导出包不得包含声明外资产的字节"
    );
    assert!(!all_text.contains(&other_sha));

    // 未登录 → 401；未知 release → 404。
    let anonymous = chain
        .app
        .call(
            Method::GET,
            &format!("/api/v1/releases/{}/export", release.release_id),
        )
        .send()
        .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    let unknown = chain
        .app
        .call(
            Method::GET,
            &format!(
                "/api/v1/releases/{}/export",
                "01930000-0000-7000-8000-000000000000"
            ),
        )
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.text());

    // 导出后临时文件被清理（响应读完即删除；tmp/ 不留包）。
    for entry in std::fs::read_dir(chain.app.dir().join("tmp")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            !name.starts_with("export-"),
            "导出临时文件应被清理，发现 {name}"
        );
    }
}

// ---------------------------------------------------------------------------
// 6) 迁移门禁：旧 schema（v1）备份可恢复并自动迁移
// ---------------------------------------------------------------------------

const MIGRATION_0001: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../migrations/0001_core_schema.sql"
));

/// 造一个 v1（仅 0001）的 data-dir：结构 + 旧库 + 一条物品、一个 blob 与资产。
async fn build_v1_data_dir(dir: &Path) -> String {
    everything_manual::config::datadir::ensure_initialized(dir).expect("初始化结构");

    let legacy_dir = dir.join("legacy-migrations");
    std::fs::create_dir_all(&legacy_dir).unwrap();
    std::fs::write(legacy_dir.join("0001_core_schema.sql"), MIGRATION_0001).unwrap();
    let legacy_migrator = sqlx::migrate::Migrator::new(legacy_dir.as_path())
        .await
        .expect("解析旧迁移目录");

    let db_path = dir.join("manual.sqlite3");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("连接旧库");
    legacy_migrator.run(&pool).await.expect("应用 v1 迁移");
    let applied: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(applied, 1, "测试前提：旧库只到 v1");

    // 旧库中的用户数据：物品 + blob 文件 + blob/assets 行。
    let item_id = manual_core::ids::new_id();
    let blob_bytes = b"legacy-v1-blob-payload".to_vec();
    let blob_sha = sha256_hex(&blob_bytes);
    let now = Timestamp::now().as_millis();
    sqlx::query(
        "INSERT INTO items (id, name, brand, model, variant, revision, archived_at, created_at, updated_at) \
         VALUES (?, '升级前数据', 'Fuji', 'X100V', NULL, 3, NULL, ?, ?)",
    )
    .bind(&item_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("写入旧物品");
    sqlx::query(
        "INSERT INTO blobs (sha256, size, mime, storage_state, created_at) \
         VALUES (?, ?, 'application/pdf', 'stored', ?)",
    )
    .bind(&blob_sha)
    .bind(blob_bytes.len() as i64)
    .bind(now)
    .execute(&pool)
    .await
    .expect("写入旧 blob 行");
    let asset_id = manual_core::ids::new_id();
    sqlx::query(
        "INSERT INTO assets (id, blob_id, item_id, purpose, original_name, created_at) \
         VALUES (?, ?, ?, 'document', 'legacy.pdf', ?)",
    )
    .bind(&asset_id)
    .bind(&blob_sha)
    .bind(&item_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("写入旧资产行");
    pool.close().await;

    let blob_path = dir.join("blobs").join(&blob_sha[..2]).join(&blob_sha);
    std::fs::create_dir_all(blob_path.parent().unwrap()).unwrap();
    std::fs::write(&blob_path, &blob_bytes).unwrap();
    item_id
}

#[tokio::test]
async fn legacy_schema_backup_restores_and_migrates_automatically() {
    let work = TestDir::new("t20-legacy-cli");
    let legacy_data = work.join("legacy-data");
    let item_id = build_v1_data_dir(&legacy_data).await;

    // 备份 v1 data-dir（备份不解释 schema、不迁移源库）。
    let backup_dir = work.join("backup-out");
    let ok = run(
        work.path(),
        &[
            "backup",
            "--data-dir",
            legacy_data.to_str().unwrap(),
            "--out",
            backup_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(ok.status, 0, "{ok:?}");
    let manifest = read_backup_manifest(&backup_dir);
    assert_eq!(
        manifest.database.schema_version, 1,
        "manifest 如实记录备份时的 schema 版本"
    );
    let source_still_v1: i64 = {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(legacy_data.join("manual.sqlite3"))
                    .read_only(true),
            )
            .await
            .unwrap();
        let version = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        pool.close().await;
        version
    };
    assert_eq!(source_still_v1, 1, "备份不得迁移源库");

    // 恢复 → 新目录（仍是 v1；迁移发生在启动时）。
    let restored = work.join("restored-data");
    let restored_out = run(
        work.path(),
        &[
            "restore",
            "--from",
            backup_dir.to_str().unwrap(),
            "--data-dir",
            restored.to_str().unwrap(),
        ],
    );
    assert_eq!(restored_out.status, 0, "{restored_out:?}");
    assert!(
        restored_out.stdout.contains("schema v1"),
        "恢复输出应如实报告备份的 schema 版本：{restored_out:?}"
    );

    // 升级前预检：`check` 报告"待迁移"并给出"升级前先备份"提示（不修改数据）。
    let checked = run(
        work.path(),
        &["check", "--data-dir", restored.to_str().unwrap()],
    );
    assert_eq!(checked.status, 0, "{checked:?}");
    assert!(
        checked.stdout.contains("待迁移"),
        "check 必须报告旧 schema 待迁移：{checked:?}"
    );
    assert!(
        checked.stdout.contains("升级提示") && checked.stdout.contains("备份"),
        "check 必须给出升级前备份提示：{checked:?}"
    );

    // 真实 serve 启动 → 自动迁移 + 升级提示（T20 的"升级前备份"提示）。
    let server = ServeProcess::start(work.path(), &restored);
    let stdout = server.all_stdout();
    assert!(
        stdout.contains("schema 已自动升级"),
        "serve 必须提示自动升级与回滚边界：{stdout}"
    );
    let (exit, stdout, stderr) = server.terminate();
    assert_eq!(
        exit, 0,
        "serve 应优雅退出；stdout：{stdout}\nstderr：{stderr}"
    );
    assert!(!stderr.contains("panicked"), "serve 不得 panic：{stderr}");

    // 迁移后：数据保留、schema 到程序版本。
    let database = everything_manual::storage::Database::open_and_migrate(&restored)
        .await
        .expect("打开恢复后的库");
    let program = database.program_schema_version();
    assert_eq!(
        database.applied_schema_version().await.unwrap(),
        program,
        "启动后 schema 应到达程序版本"
    );
    let mut conn = database.pool().acquire().await.unwrap();
    let item = repo::items::get(&mut conn, &item_id)
        .await
        .unwrap()
        .expect("升级后旧数据仍在");
    assert_eq!(item.name, "升级前数据");
    assert_eq!(item.revision, 3);
    drop(conn);
    database.close().await;
}

// ---------------------------------------------------------------------------
// 7) 手工演练造数（RD 交付要求；默认 #[ignore]，不进常规测试运行）
// ---------------------------------------------------------------------------

/// 递归复制目录（演练造数用；把临时链里的 data-dir 复制到演练目录）。
fn copy_dir_all(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir_all(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// 手工演练第 1 步：造一个"已发布版本"的 data-dir 并留在 `$EM_T20_REHEARSAL_DIR/data`，
/// 同时写出 `rehearsal-info.json`（口令与关键 id/sha256；口令是测试用假凭据）。
///
/// 运行方式（工作目录必须在仓库根，且需要 `EM_T20_REHEARSAL_DIR`）：
/// ```text
/// EM_T20_REHEARSAL_DIR=/tmp/em-t20-rehearsal \
///   cargo test -p everything-manual --test backup_restore -- --ignored --nocapture prepare_rehearsal_datadir
/// ```
/// 之后的 CLI 步骤（serve → backup → restore → 读取 release/PDF/GLB → export）由
/// `artifacts/web-mvp/t20-rd/manual-rehearsal.sh` 用真实二进制执行。
#[tokio::test]
#[ignore = "手工演练造数（需要 EM_T20_REHEARSAL_DIR；不在常规 cargo test 中执行）"]
async fn prepare_rehearsal_datadir() {
    let target = std::env::var("EM_T20_REHEARSAL_DIR").expect(
        "必须设置 EM_T20_REHEARSAL_DIR（例如 /tmp/em-t20-rehearsal）；\
         本用例只用于手工演练造数",
    );
    let root = PathBuf::from(target);
    let data_dir = root.join("data");
    if root.exists() {
        std::fs::remove_dir_all(&root).expect("清理旧演练目录");
    }
    std::fs::create_dir_all(&root).unwrap();

    let (chain, release, _tripo, _manual) = published_chain("t20-rehearsal").await;
    copy_dir_all(chain.app.dir(), &data_dir);

    let info = json!({
        "password": PASSWORD,
        "itemId": release.item_id,
        "releaseId": release.release_id,
        "manifestSha256": release.manifest_sha256,
        "modelAssetId": release.model_asset_id,
        "modelSha256": release.model_sha256,
        "documentAssetId": release.document_asset_id,
        "documentSha256": release.document_sha256,
    });
    std::fs::write(
        root.join("rehearsal-info.json"),
        format!("{}\n", serde_json::to_string_pretty(&info).unwrap()),
    )
    .unwrap();
    println!(
        "演练数据已就绪：{}\n{}",
        data_dir.display(),
        serde_json::to_string_pretty(&info).unwrap()
    );
}
