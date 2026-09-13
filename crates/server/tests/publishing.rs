//! T19 集成测试：热点校准、知识确认与修订、发布事务与不可变版本
//! （PRD 修订 2 / ui_revision 2；REQ-033/REQ-034/REQ-035，AC-052–AC-056 的服务端侧）。
//!
//! 覆盖（命令 ↔ AC 见 implementation.md §T19）：
//! - **热点状态机与锚点规则**（AC-052）：人工直接拾取 `unbound → confirmed`；
//!   `unbound` 时 anchor 必须为 `null`（拒绝 [0,0,0] 占位）；candidate/confirmed
//!   必须有非空 anchor；数值必须有限（拒绝超范围字面量/NaN）；引用必须存在；
//!   供应商事实快照不可改（未知字段 422）；
//! - **stale 规则（真实链路，AC-053）**：重新生成模型（同一物品、新 sha 的新模型字节，
//!   经真实流水线下载/校验/落库）后打开新草稿：旧绑定进入 `stale`（anchor 保留），
//!   不再是有效热点；API 拒绝用旧 sha 提交 confirmed；重新绑定后回到 confirmed；
//!   旧发布版继续指向旧模型且字节可读；
//! - **知识确认与人工修订 + modelReview**（AC-054）：实体级 confirmed/needs_review；
//!   人工修改进覆盖层（userEdited + 编辑者/时间）而**不改供应商快照**；Evidence
//!   保持 1-based 且 bbox 为 null（不捏造框）；modelReview 由用户声明、服务器赋值
//!   `checkedAt`，`userConfirmed` 不能脱离 `loaded`；换模型清空 modelReview；
//! - **发布不变量逐条正/负例**（AC-055/AC-056）：未确认知识、缺 confirmed 热点、
//!   stale/冒充 confirmed、modelReview 未完成、模型未 validated、引用页不存在、
//!   步骤引用不存在 → 422 + `details.issues[]` 明细；仅文本条目保留并计数；
//! - **发布事务**（AC-055）：If-Match 缺 428；并发/陈旧 revision 412；幂等键重放
//!   返回同一 release；发布**不产生费用、不调用任何供应商**（fixture 调用计数 +
//!   账本行数前后不变）；发布后修改草稿不改变已发布内容（manifest 字节/哈希比对，
//!   且 release 指向的模型资产仍是旧模型字节）。
//!
//! 隔离与门控：与 T15 相同——全部 HTTP 指向 T05 本机 fixture（127.0.0.1 随机端口）、
//! 测试构建 + 显式配置才放行回环模型下载、零真实外网、零真实付费。

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestDir};
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

const PASSWORD: &str = "test-password-t19-b7c1";
/// 测试用假凭据（canary）：断言不得出现在日志/记录/Debug 输出里。
const CANARY_KEY: &str = "canary-t19-not-a-real-key";
const MANUAL_AI_MODEL: &str = "gpt-5-mini";
const PRESET: &str = "tripo-h-v3.1-standard";

/// 测试价格目录（与 T11–T15 用例同价）。
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
const TASK_ID: &str = "t19-fixture-task-0001";

// ---------------------------------------------------------------------------
// fixture 工具（与 T13/T15 用例同构）
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
    // 每个任务两次图片上传（front/left）；脚本按调用顺序消费。
    for index in 0..4 {
        upload_steps.push(respond_json(json!({
            "code": 0,
            "data": { "file_token": format!("token-{index}-t19") }
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
                "rendered_image_url": "https://cdn.example.invalid/preview.png",
            }
        }
    }))
}

/// CDN fixture：把模型字节写进**每调用唯一**的临时文件再提供。
///
/// 为什么必须唯一：多个用例并行（cargo test 默认多线程）会请求同一个路径
/// （`/model.glb`）与同一份字节；若共用文件名，`std::fs::write` 的
/// "截断 + 写入"会让另一个用例的下载读到 0 字节/半文件（真实出现过的假失败：
/// `model_validate needs_input：文件过短（0 字节）`）。
fn cdn_server(path: &str, bytes: Vec<u8>) -> (FixtureServer, String) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "em-t19-cdn-{}-{}-{}",
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

/// 第二个模型版本：在 `sample-model.glb` 的 BIN 起始处翻转一个顶点浮点的高位字节
/// （±1.0 → ±0.25）。结构、bufferView/accessor 范围与几何规模均不变（仍是合法 GLB），
/// 但字节与 sha256 不同——这就是"重新生成的新模型"在测试里的真实替代物
/// （真实网络下载 + T13 校验 + 新 revision 都走生产代码路径）。
fn variant_model_bytes() -> Vec<u8> {
    let mut bytes = fixture_bytes("sample-model.glb");
    let bin_marker = bytes
        .windows(4)
        .position(|window| window == b"BIN\0")
        .expect("GLB 必须包含 BIN chunk");
    let data_start = bin_marker + 4;
    // 第一个顶点分量的最高字节：f32 1.0(0x3F80_0000) 或 -1.0(0xBF80_0000) → 0x3E/0xBE。
    bytes[data_start + 3] ^= 0x01;
    let info = test_support::assets::validate_glb(&bytes).expect("变体模型必须仍是合法 GLB");
    assert!(info.triangles > 0, "变体模型必须非空几何");
    bytes
}

// ---------------------------------------------------------------------------
// 应用、输入与任务（HTTP 合同驱动；与 T15 同一手法）
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
            boundary: format!("----em-t19-{tag}"),
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

#[allow(clippy::too_many_arguments)] // 测试辅助：每个参数都是上传请求的一个显式字段
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

/// 物品 + PDF（3 页文字）+ front/left 照片 + ready 准备（与 T15 同一造数路径）。
async fn build_ready_inputs(app: &TestApp, cookie: &str, csrf: &str) -> ReadyInputs {
    let item = {
        let response = app
            .call(Method::POST, "/api/v1/items")
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({ "name": "发布测试物品", "model": "X100V" }))
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
            format!("T19-PAGE-{page}-CONTENT 后盖 螺钉 电池").as_bytes(),
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

// ---------------------------------------------------------------------------
// 测试链（应用 + 会话 + 输入 + 执行器）
// ---------------------------------------------------------------------------

struct Chain {
    app: TestApp,
    cookie: String,
    csrf: String,
    inputs: ReadyInputs,
    executor: Arc<JobExecutor>,
    clock: Arc<ManualClock>,
}

/// 建链：应用 + 登录 + 可生成输入 + 生产接线执行器（与 `serve` 同构）。
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

/// 创建并跑到**草稿产出**（真实流水线），返回 (job 状态, 草稿行)。
///
/// 两条分支都成功 → `succeeded` + 完整草稿；某条分支被阻塞（如模型校验失败）→
/// 父任务保持 `needs_input` 但草稿仍产出（部分成功可展示）。
async fn run_job_until_draft(chain: &Chain, key: &str) -> (JobStatus, ManualDraft) {
    let job_id = create_job(&chain.app, &chain.cookie, &chain.csrf, &chain.inputs, key).await;
    run_created_job_until_draft(chain, &job_id).await
}

/// 跑到指定任务产出草稿（或到达 `succeeded`/`needs_input` 终态）。
async fn run_created_job_until_draft(chain: &Chain, job_id: &str) -> (JobStatus, ManualDraft) {
    let pool = pool(chain);
    for _ in 0..80 {
        let job = {
            let mut conn = pool.acquire().await.expect("连接");
            repo::jobs::get(&mut conn, job_id)
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
            return (job.status, draft);
        }
        chain.clock.advance_millis(20_000);
        match chain.executor.tick().await.expect("tick") {
            TickOutcome::Executed(_) | TickOutcome::Idle => {}
        }
    }
    panic!(
        "任务未在 80 tick 内产出草稿：{}",
        stage_summary(&chain.app, job_id).await
    );
}

/// 创建并跑到 `succeeded` 的任务（完整草稿）。
async fn run_job_to_draft(chain: &Chain, key: &str) -> ManualDraft {
    let job_id = create_job(&chain.app, &chain.cookie, &chain.csrf, &chain.inputs, key).await;
    let (status, draft) = run_created_job_until_draft(chain, &job_id).await;
    if status != JobStatus::Succeeded {
        let summary = stage_summary(&chain.app, &job_id).await;
        panic!("fixture 全链路必须成功（实际 {status:?}）：{summary}");
    }
    draft
}

/// 阶段摘要（失败诊断用；不含密钥与路径）。
async fn stage_summary(app: &TestApp, job_id: &str) -> String {
    let pool = app.state().database().pool().clone();
    let mut conn = pool.acquire().await.expect("连接");
    let stages = repo::job_stages::list_for_job(&mut conn, job_id)
        .await
        .expect("列出阶段");
    stages
        .iter()
        .map(|stage| {
            format!(
                "{}[{}] attempts={} last_error={:?} needs_input={:?}",
                stage.stage_kind.as_str(),
                stage.status.as_str(),
                stage.attempt_count,
                stage.last_error.as_deref().unwrap_or(""),
                stage.needs_input_json
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

// ---------------------------------------------------------------------------
// HTTP 辅助
// ---------------------------------------------------------------------------

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

async fn patch_draft(
    chain: &Chain,
    draft_id: &str,
    etag: &str,
    body: Value,
) -> common::TestResponse {
    chain
        .app
        .call(
            Method::PATCH,
            &format!("/api/v1/items/{}/drafts/{draft_id}", chain.inputs.item),
        )
        .cookie(&chain.cookie)
        .csrf(&chain.csrf)
        .header("if-match", etag)
        .json(&body)
        .send()
        .await
}

/// PATCH 并断言 200（返回新视图）。
async fn patch_ok(chain: &Chain, draft_id: &str, view: &DraftView, body: Value) -> DraftView {
    let response = patch_draft(chain, draft_id, &view.etag, body).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    DraftView {
        json: response.json(),
        etag: response.header("etag").expect("更新后带 ETag"),
    }
}

/// PATCH 并断言 422（返回错误体，用于断言字段/原因）。
async fn patch_422(chain: &Chain, draft_id: &str, etag: &str, body: Value) -> Value {
    let response = patch_draft(chain, draft_id, etag, body).await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    response.json()
}

async fn publish(chain: &Chain, draft_id: &str, if_match: &str, key: &str) -> common::TestResponse {
    chain
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
        .header("if-match", if_match)
        .header("idempotency-key", key)
        .send()
        .await
}

fn draft_knowledge(view: &DraftView) -> &Value {
    &view.json["data"]["knowledge"]
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

/// 全部实体确认（发布不变量的"必需知识已确认"）。
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

fn sha256_hex(bytes: &[u8]) -> String {
    test_support::assets::sha256_hex(bytes)
}

// ===========================================================================
// 1) 热点状态机、锚点规则与供应商快照只读（AC-052/AC-053 的 API 侧）
// ===========================================================================

#[tokio::test]
async fn hotspot_state_machine_and_anchor_rules_follow_the_contract() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = cdn_server("/model.glb", fixture_bytes("sample-model.glb"));
    let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&model_url)]);
    let chain = chain("t19-hotspots", &tripo, &manual).await;
    let draft = run_job_to_draft(&chain, "t19-hotspot-key").await;
    let draft_id = draft.id.clone();
    let view = get_draft(&chain, &draft_id).await;
    let knowledge = draft_knowledge(&view).clone();
    let (revision_id, sha256) = model_identity(&knowledge);
    let parts = entity_ids(&knowledge, "parts");
    assert!(!parts.is_empty(), "真实知识必须有部件");
    let part = parts[0].clone();

    // 未绑定（unbound）不得携带 [0,0,0] 占位。
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "hotspots": { "upsert": [{
            "partId": part, "status": "unbound",
            "anchor": { "modelRevisionId": revision_id, "modelSha256": sha256, "positionLocal": [0.0, 0.0, 0.0] }
        }] } }),
    )
    .await;
    assert_eq!(error["error"]["code"], "VALIDATION_FAILED");
    assert!(
        error["error"]["details"]["fields"][0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("占位"),
        "{error}"
    );

    // confirmed 必须有非空 anchor。
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "hotspots": { "upsert": [{
            "partId": part, "status": "confirmed", "anchor": null
        }] } }),
    )
    .await;
    assert!(
        error["error"]["details"]["fields"][0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("非空 anchor"),
        "{error}"
    );

    // 旧 sha 的 confirmed → 拒绝（AC-053 的 API 侧）。
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "hotspots": { "upsert": [{
            "partId": part, "status": "confirmed",
            "anchor": {
                "modelRevisionId": "old-revision",
                "modelSha256": "b".repeat(64),
                "positionLocal": [0.1, 0.2, 0.3]
            }
        }] } }),
    )
    .await;
    assert!(
        error["error"]["details"]["fields"][0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("旧模型版本"),
        "{error}"
    );

    // 未知部件 / 未知热点 / 未知字段（快照只读）→ 422。
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "hotspots": { "upsert": [{
            "partId": "part-does-not-exist", "status": "unbound", "anchor": null
        }] } }),
    )
    .await;
    assert!(error.to_string().contains("部件不存在"), "{error}");
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "hotspots": { "upsert": [], "remove": ["hotspot-missing"] } }),
    )
    .await;
    assert!(error.to_string().contains("不存在"), "{error}");
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "knowledgeJson": { "parts": [] } }),
    )
    .await;
    assert_eq!(error["error"]["code"], "VALIDATION_FAILED", "{error}");
    // 超范围字面量（1e400）在解析层被拒绝 → 同样是 422（不是 NaN 入库）。
    let raw = format!(
        r#"{{"hotspots":{{"upsert":[{{"partId":"{part}","status":"confirmed","anchor":{{"modelRevisionId":"{revision_id}","modelSha256":"{sha256}","positionLocal":[1e400,0,0]}}}}]}}}}"#
    );
    let response = chain
        .app
        .call(
            Method::PATCH,
            &format!("/api/v1/items/{}/drafts/{draft_id}", chain.inputs.item),
        )
        .cookie(&chain.cookie)
        .csrf(&chain.csrf)
        .header("if-match", &view.etag)
        .raw_body(Some("application/json"), raw.into_bytes())
        .send()
        .await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );

    // unbound（anchor=null）→ confirmed（人工直接拾取）：状态机与数值原样落库。
    let view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "hotspots": { "upsert": [{
            "partId": part, "status": "unbound", "anchor": null
        }] } }),
    )
    .await;
    let hotspots = draft_knowledge(&view)["hotspots"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(hotspots.len(), 1);
    assert_eq!(hotspots[0]["status"], "unbound");
    assert!(
        hotspots[0]["anchor"].is_null(),
        "unbound 的 anchor 必须是 null（不是 [0,0,0]）：{}",
        hotspots[0]
    );
    let hotspot_id = hotspots[0]["id"].as_str().unwrap().to_owned();

    let local = [0.125, -0.25, 0.5];
    let view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "hotspots": { "upsert": [{
            "id": hotspot_id,
            "partId": part,
            "status": "confirmed",
            "anchor": {
                "modelRevisionId": revision_id.clone(),
                "modelSha256": sha256.clone(),
                "positionLocal": local,
            }
        }] } }),
    )
    .await;
    let hotspot = &draft_knowledge(&view)["hotspots"][0];
    assert_eq!(hotspot["status"], "confirmed");
    assert_eq!(hotspot["anchor"]["positionLocal"], json!(local));
    assert_eq!(hotspot["anchor"]["modelSha256"], sha256);
    // 第二次 PATCH（内容相同）按幂等返回：revision 不变。
    let again = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "hotspots": { "upsert": [{
            "id": hotspot_id,
            "partId": part,
            "status": "confirmed",
            "anchor": {
                "modelRevisionId": revision_id,
                "modelSha256": sha256,
                "positionLocal": local,
            }
        }] } }),
    )
    .await;
    assert_eq!(
        again.etag, view.etag,
        "无实际变化的重复提交不得递增 revision（幂等）"
    );
    // 解绑（remove）→ 热点消失。
    let view = patch_ok(
        &chain,
        &draft_id,
        &again,
        json!({ "hotspots": { "upsert": [], "remove": [hotspot_id.clone()] } }),
    )
    .await;
    assert!(
        draft_knowledge(&view)["hotspots"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // 审核留痕：draft_review_updated 记录了热点数量。
    let audit: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE action = 'draft_review_updated'",
    )
    .fetch_one(&pool(&chain))
    .await
    .unwrap();
    assert!(audit >= 2, "每次实际变化都必须留审计");

    cdn.shutdown();
    tripo.shutdown();
    manual.shutdown();
}

// ===========================================================================
// 2) 知识确认、人工修订、modelReview 与 bbox=null（AC-054）
// ===========================================================================

#[tokio::test]
async fn review_layer_keeps_supplier_snapshot_and_model_review_is_declared_by_user() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = cdn_server("/model.glb", fixture_bytes("sample-model.glb"));
    let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&model_url)]);
    let chain = chain("t19-review", &tripo, &manual).await;
    let draft = run_job_to_draft(&chain, "t19-review-key").await;
    let draft_id = draft.id.clone();
    let mut view = get_draft(&chain, &draft_id).await;
    let knowledge = draft_knowledge(&view).clone();
    let parts = entity_ids(&knowledge, "parts");
    let steps = entity_ids(&knowledge, "steps");
    let specs = entity_ids(&knowledge, "specs");
    let part = parts[0].clone();

    // Evidence 的 1-based 与 bbox=null（不捏造框）。
    let evidence = &knowledge["knowledge"]["parts"][0]["evidence"][0];
    assert!(
        evidence["pageNumber"].as_i64().unwrap_or(0) >= 1,
        "页码必须 1-based：{evidence}"
    );
    assert!(
        evidence["bbox"].is_null(),
        "本版本不产出 bbox，不得捏造：{evidence}"
    );
    assert!(evidence["documentId"].as_str().is_some(), "{evidence}");

    // 供应商快照只读：不支持该实体类型的字段 → 422。
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "entities": { part.clone(): { "userEdited": { "title": "错字段" } } } }),
    )
    .await;
    assert!(error.to_string().contains("不支持人工修订字段"), "{error}");
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "entities": { "ghost-entity": { "reviewStatus": "confirmed" } } }),
    )
    .await;
    assert!(error.to_string().contains("实体不存在"), "{error}");

    // 确认 + 人工修订：覆盖层记录 userEdited 与编辑者/时间，供应商快照不变。
    let original_name = knowledge["knowledge"]["parts"][0]["name"]
        .as_str()
        .unwrap()
        .to_owned();
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "entities": { part.clone(): {
            "reviewStatus": "confirmed",
            "userEdited": { "name": "后盖（人工修订）" }
        } } }),
    )
    .await;
    let review = &view.json["data"]["review"];
    let entry = &review["entities"][&part];
    assert_eq!(entry["reviewStatus"], "confirmed");
    assert_eq!(entry["userEdited"]["name"], "后盖（人工修订）");
    assert!(
        entry["editedAt"].as_i64().unwrap_or(0) > 0,
        "服务器赋编辑时间"
    );
    assert!(entry["editedBy"].as_str().is_some(), "服务器赋操作者");
    assert_eq!(
        draft_knowledge(&view)["knowledge"]["parts"][0]["name"],
        original_name.as_str(),
        "供应商事实快照不得被修改（原文本保留供对照）"
    );

    // needs_review 可切回（可切换语义）。
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "entities": { part.clone(): { "reviewStatus": "needs_review" } } }),
    )
    .await;
    assert_eq!(
        view.json["data"]["review"]["entities"][&part]["reviewStatus"],
        "needs_review"
    );

    // 「仅文本条目」不能用来跳过复核（换一个尚无任何复核记录的部件）。
    let untouched = parts
        .iter()
        .find(|candidate| **candidate != part)
        .cloned()
        .expect("真实知识至少两个部件");
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "entities": { untouched.clone(): { "textOnly": true } } }),
    )
    .await;
    assert!(
        error["error"]["details"]["fields"][0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("仅文本条目"),
        "{error}"
    );
    // 确认 + 仅文本条目可以（保留文字条目、不要求热点）。
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "entities": { untouched.clone(): { "reviewStatus": "confirmed", "textOnly": true } } }),
    )
    .await;
    assert_eq!(
        view.json["data"]["review"]["entities"][&untouched]["textOnly"],
        true
    );

    // modelReview：userConfirmed 不能脱离 loaded；loaded 由用户声明、checkedAt 服务器赋值。
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "modelReview": { "loaded": false, "userConfirmed": true } }),
    )
    .await;
    assert!(error.to_string().contains("loaded"), "{error}");
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "modelReview": { "loaded": true, "userConfirmed": false } }),
    )
    .await;
    let model_review = &view.json["data"]["review"]["modelReview"];
    assert_eq!(model_review["loaded"], true);
    assert_eq!(model_review["userConfirmed"], false);
    assert!(model_review["checkedAt"].as_i64().unwrap_or(0) > 0);
    let (revision_id, sha256) = model_identity(draft_knowledge(&view));
    assert_eq!(model_review["modelRevisionId"], revision_id.as_str());
    assert_eq!(model_review["modelSha256"], sha256.as_str());
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "modelReview": { "loaded": true, "userConfirmed": true } }),
    )
    .await;
    assert_eq!(
        view.json["data"]["review"]["modelReview"]["userConfirmed"],
        true
    );

    // 全部实体确认（供后续用例参考；本用例只验证写入语义）。
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        confirm_all(draft_knowledge(&view)),
    )
    .await;
    assert_eq!(
        view.json["data"]["review"]["entities"]
            .as_object()
            .unwrap()
            .len(),
        parts.len() + steps.len() + specs.len()
    );

    // 步骤视角：合法写入 + 非法 FOV 拒绝 + 清除动作分开。
    let step = steps[0].clone();
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "stepPoses": { step.clone(): {
            "positionLocal": [0.0, 0.0, 3.0], "targetLocal": [0.0, 0.0, 0.0],
            "upLocal": [0.0, 1.0, 0.0], "fov": 400.0
        } } }),
    )
    .await;
    assert!(error.to_string().contains("CameraPose"), "{error}");
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "stepPoses": { step.clone(): {
            "positionLocal": [0.0, 0.0, 3.0], "targetLocal": [0.0, 0.0, 0.0],
            "upLocal": [0.0, 1.0, 0.0], "fov": 40.0
        } } }),
    )
    .await;
    assert_eq!(
        draft_knowledge(&view)["stepPoses"][&step]["fov"],
        json!(40.0)
    );
    let error = patch_422(
        &chain,
        &draft_id,
        &view.etag,
        json!({ "stepPoses": { "ghost-step": {
            "positionLocal": [0.0, 0.0, 3.0], "targetLocal": [0.0, 0.0, 0.0],
            "upLocal": [0.0, 1.0, 0.0], "fov": 40.0
        } } }),
    )
    .await;
    assert!(error.to_string().contains("步骤不存在"), "{error}");
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "clearStepPoses": [step.clone()] }),
    )
    .await;
    assert!(
        draft_knowledge(&view)["stepPoses"]
            .as_object()
            .unwrap()
            .is_empty()
    );

    // 空请求体 422（不能把"什么都没写"当作成功）。
    let error = patch_422(&chain, &draft_id, &view.etag, json!({})).await;
    assert_eq!(error["error"]["code"], "VALIDATION_FAILED", "{error}");
    // 缺 If-Match → 428。
    let response = chain
        .app
        .call(
            Method::PATCH,
            &format!("/api/v1/items/{}/drafts/{draft_id}", chain.inputs.item),
        )
        .cookie(&chain.cookie)
        .csrf(&chain.csrf)
        .json(&json!({ "status": "ready" }))
        .send()
        .await;
    assert_eq!(
        response.status,
        StatusCode::PRECONDITION_REQUIRED,
        "{}",
        response.text()
    );

    cdn.shutdown();
    tripo.shutdown();
    manual.shutdown();
}

// ===========================================================================
// 3) 真实链路 stale：重新生成模型后旧绑定降级（AC-053）
// ===========================================================================

#[tokio::test]
async fn regenerated_model_marks_previous_bindings_stale_and_rejects_old_sha() {
    let manual = manual_ai_server(vec![
        respond_file(&responses_path("success.json")),
        respond_file(&responses_path("success.json")),
    ]);
    let (cdn_a, url_a) = cdn_server("/model-a.glb", fixture_bytes("sample-model.glb"));
    let variant = variant_model_bytes();
    let variant_sha = sha256_hex(&variant);
    let (cdn_b, url_b) = cdn_server("/model-b.glb", variant);
    assert_ne!(
        sha256_hex(&fixture_bytes("sample-model.glb")),
        variant_sha,
        "两个模型必须字节不同"
    );
    let tripo = tripo_server(
        vec![submit_success(), submit_success()],
        vec![task_success_with(&url_a), task_success_with(&url_b)],
    );
    let chain = chain("t19-stale", &tripo, &manual).await;

    // 第一次生成：绑定 confirmed（人工直接拾取）并全部确认。
    let draft_one = run_job_to_draft(&chain, "t19-stale-key-1").await;
    let view_one = get_draft(&chain, &draft_one.id).await;
    let knowledge_one = draft_knowledge(&view_one).clone();
    let (revision_one, sha_one) = model_identity(&knowledge_one);
    let parts_one = entity_ids(&knowledge_one, "parts");
    let upserts_one: Vec<Value> = parts_one
        .iter()
        .enumerate()
        .map(|(index, part)| {
            hotspot_upsert(
                part,
                &revision_one,
                &sha_one,
                [0.5 - 0.1 * index as f64, 0.5, 0.5],
            )
        })
        .collect();
    let mut view_one = patch_ok(
        &chain,
        &draft_one.id,
        &view_one,
        json!({ "hotspots": { "upsert": upserts_one } }),
    )
    .await;
    let hotspot_one = draft_knowledge(&view_one)["hotspots"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    view_one = patch_ok(
        &chain,
        &draft_one.id,
        &view_one,
        confirm_all(draft_knowledge(&view_one)),
    )
    .await;
    view_one = patch_ok(
        &chain,
        &draft_one.id,
        &view_one,
        json!({ "modelReview": { "loaded": true, "userConfirmed": true } }),
    )
    .await;

    // 发布第一版（旧模型）。
    let published_revision: i64 = view_one
        .etag
        .trim_matches('"')
        .trim_start_matches('r')
        .parse()
        .expect("ETag 形如 \"r<n>\"");
    let published = publish(&chain, &draft_one.id, &view_one.etag, "t19-stale-publish-1").await;
    assert_eq!(
        published.status,
        StatusCode::CREATED,
        "{}",
        published.text()
    );
    let release_one = published.json()["data"]["id"].as_str().unwrap().to_owned();
    let draft_revision_after = published.json()["data"]["draftRevisionAfterPublish"]
        .as_i64()
        .expect("发布响应给出草稿新 revision");

    // 第二次生成（同一物品的新任务 → 新快照 → 新草稿；模型字节不同）。
    let draft_two = run_job_to_draft(&chain, "t19-stale-key-2").await;
    assert_ne!(
        draft_two.id, draft_one.id,
        "新任务必须产出新草稿（同快照才复用）"
    );
    let view_two = get_draft(&chain, &draft_two.id).await;
    let knowledge_two = draft_knowledge(&view_two).clone();
    let (revision_two, sha_two) = model_identity(&knowledge_two);
    assert_ne!(revision_two, revision_one, "新模型必须是新 revision");
    assert_ne!(sha_two, sha_one, "新模型字节必须不同");
    assert_eq!(sha_two, variant_sha, "新模型哈希等于第二个 CDN 提供的字节");

    // 旧绑定被继承为 stale（anchor 保留作解释，不再是有效热点）。
    let hotspots = draft_knowledge(&view_two)["hotspots"]
        .as_array()
        .unwrap()
        .clone();
    let carried = hotspots
        .iter()
        .find(|hotspot| hotspot["id"] == hotspot_one.as_str())
        .unwrap_or_else(|| panic!("新草稿必须继承旧热点（标记 stale）：{hotspots:?}"));
    assert_eq!(
        carried["status"], "stale",
        "旧绑定必须进入 stale：{carried}"
    );
    assert_eq!(carried["anchor"]["modelRevisionId"], revision_one.as_str());
    assert_eq!(carried["anchor"]["modelSha256"], sha_one.as_str());

    // 尝试用旧 sha 提交 confirmed → 422（AC-053 原文）。
    let error = patch_422(
        &chain,
        &draft_two.id,
        &view_two.etag,
        json!({ "hotspots": { "upsert": [{
            "id": hotspot_one.clone(),
            "partId": parts_one[0].clone(),
            "status": "confirmed",
            "anchor": {
                "modelRevisionId": revision_one.clone(),
                "modelSha256": sha_one.clone(),
                "positionLocal": [0.5, 0.5, 0.5]
            }
        }] } }),
    )
    .await;
    assert!(
        error["error"]["details"]["fields"][0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("旧模型版本"),
        "{error}"
    );

    // 重新绑定到新模型 → confirmed 且 sha 匹配。
    let view_two = patch_ok(
        &chain,
        &draft_two.id,
        &view_two,
        json!({ "hotspots": { "upsert": [{
            "id": hotspot_one.clone(),
            "partId": parts_one[0].clone(),
            "status": "confirmed",
            "anchor": {
                "modelRevisionId": revision_two.clone(),
                "modelSha256": sha_two.clone(),
                "positionLocal": [0.6, 0.4, 0.2]
            }
        }] } }),
    )
    .await;
    let rebound = &draft_knowledge(&view_two)["hotspots"][0];
    assert_eq!(rebound["status"], "confirmed");
    assert_eq!(rebound["anchor"]["modelSha256"], sha_two.as_str());

    // 旧发布版继续指向旧模型且可读（字节哈希与发布时一致）。
    let detail = chain
        .app
        .call(
            Method::GET,
            &format!("/api/v1/items/{}/releases/{release_one}", chain.inputs.item),
        )
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.text());
    let detail = detail.json();
    assert_eq!(detail["data"]["modelRevisionId"], revision_one.as_str());
    assert_eq!(
        detail["data"]["manifest"]["model"]["sha256"],
        sha_one.as_str()
    );
    let manifest_sha = detail["data"]["manifestSha256"]
        .as_str()
        .unwrap()
        .to_owned();
    let model_asset = detail["data"]["manifest"]["model"]["assetId"]
        .as_str()
        .unwrap()
        .to_owned();
    let content = chain
        .app
        .call(
            Method::GET,
            &format!("/api/v1/assets/{model_asset}/content"),
        )
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(content.status, StatusCode::OK, "{}", content.text());
    assert_eq!(
        sha256_hex(&content.body),
        sha_one.as_str(),
        "旧发布版继续指向旧模型字节"
    );
    // 发布事务给草稿递增 revision（并发发布 412 的依据）。
    assert_eq!(draft_revision_after, 5, "发布给草稿递增一次 revision");
    // 新草稿的草稿状态与旧发布版本互不影响。
    assert_eq!(detail["data"]["draftId"], draft_one.id.as_str());
    assert_eq!(
        detail["data"]["draftRevision"],
        json!(published_revision),
        "release 记录发布时的 draftRevision"
    );
    assert_eq!(
        detail["data"]["manifest"]["draftRevision"],
        json!(published_revision)
    );
    assert!(!manifest_sha.is_empty());

    cdn_a.shutdown();
    cdn_b.shutdown();
    tripo.shutdown();
    manual.shutdown();
}

// ===========================================================================
// 4) 发布不变量逐条 422 明细 + 仅文本条目（AC-055/AC-056）
// ===========================================================================

#[tokio::test]
async fn publish_reports_each_invariant_violation_and_allows_text_only_parts() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = cdn_server("/model.glb", fixture_bytes("sample-model.glb"));
    let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&model_url)]);
    let chain = chain("t19-invariants", &tripo, &manual).await;
    let draft = run_job_to_draft(&chain, "t19-invariant-key").await;
    let draft_id = draft.id.clone();
    let view = get_draft(&chain, &draft_id).await;
    let knowledge = draft_knowledge(&view).clone();
    let (revision_id, sha256) = model_identity(&knowledge);
    let parts = entity_ids(&knowledge, "parts");
    let steps = entity_ids(&knowledge, "steps");
    let specs = entity_ids(&knowledge, "specs");

    // 什么都不满足：422 列出未确认知识、modelReview 缺失、缺 confirmed 热点。
    let response = publish(&chain, &draft_id, &view.etag, "t19-inv-key-1").await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let body = response.json();
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(
        body["error"]["details"]["reason"],
        "publishInvariantsViolated"
    );
    let issues = body["error"]["details"]["issues"]
        .as_array()
        .unwrap()
        .clone();
    let codes: Vec<&str> = issues
        .iter()
        .map(|issue| issue["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"knowledgeUnreviewed"), "{issues:?}");
    assert!(codes.contains(&"modelReviewMissing"), "{issues:?}");
    assert!(codes.contains(&"hotspotMissing"), "{issues:?}");
    assert!(
        issues
            .iter()
            .filter(|issue| issue["code"] == "knowledgeUnreviewed")
            .count()
            >= parts.len() + steps.len() + specs.len(),
        "每个未确认实体都应有明细：{issues:?}"
    );

    // 一条条补齐；用"未绑定热点"证明它不算 confirmed。
    let mut view = patch_ok(&chain, &draft_id, &view, confirm_all(&knowledge)).await;
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "modelReview": { "loaded": true, "userConfirmed": true } }),
    )
    .await;
    // 一个"仅文本条目"部件（确认 + textOnly）：不要求热点，但仍在发布内容里。
    let text_only_part = parts[0].clone();
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "entities": { text_only_part.clone(): { "reviewStatus": "confirmed", "textOnly": true } } }),
    )
    .await;
    let interactive: Vec<String> = parts
        .iter()
        .filter(|part| **part != text_only_part)
        .cloned()
        .collect();
    let upserts: Vec<Value> = interactive
        .iter()
        .enumerate()
        .map(|(index, part)| {
            hotspot_upsert(
                part,
                &revision_id,
                &sha256,
                [0.1 * (index as f64 + 1.0), 0.2, 0.3],
            )
        })
        .collect();
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "hotspots": { "upsert": upserts } }),
    )
    .await;
    // 再加一个 unbound 热点：不是 confirmed → 不满足"交互部件至少一个 confirmed"。
    // （这里给交互部件的热点已全部 confirmed，unbound 只是防御性状态。）
    let published = publish(&chain, &draft_id, &view.etag, "t19-inv-key-2").await;
    assert_eq!(
        published.status,
        StatusCode::CREATED,
        "{}",
        published.text()
    );
    let manifest = published.json()["data"].clone();
    let release_id = manifest["id"].as_str().unwrap().to_owned();

    // 详情：仅文本条目保留并计数（不自动隐藏），复核声明被冻结。
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
    let detail = detail.json();
    let knowledge = &detail["data"]["manifest"]["knowledge"];
    let manifest_parts: Vec<&str> = knowledge["knowledge"]["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| part["id"].as_str().unwrap())
        .collect();
    for part in &parts {
        assert!(
            manifest_parts.contains(&part.as_str()),
            "发布内容不得隐藏必需部件"
        );
    }
    assert_eq!(
        detail["data"]["manifest"]["counts"]["textOnlyParts"],
        json!(1)
    );
    assert_eq!(
        detail["data"]["manifest"]["review"]["entities"][&text_only_part]["textOnly"],
        json!(true)
    );
    assert_eq!(
        detail["data"]["manifest"]["review"]["modelReview"]["userConfirmed"],
        json!(true)
    );
    // 引用资产与来源（模型 + 原件）都在 manifest 里，含 sha256。
    let assets = detail["data"]["manifest"]["assets"].as_array().unwrap();
    assert!(
        assets
            .iter()
            .any(|asset| asset["role"] == "model" && asset["sha256"] == json!(sha256))
    );
    assert!(
        assets
            .iter()
            .any(|asset| asset["role"] == "document" && asset["source"] == "itemUpload")
    );
    let preparation_id = detail["data"]["manifest"]["documents"][0]["preparationId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        !preparation_id.is_empty(),
        "manifest 必须记录说明书原件与冻结准备：{}",
        detail["data"]["manifest"]["documents"]
    );

    cdn.shutdown();
    tripo.shutdown();
    manual.shutdown();
}

// ===========================================================================
// 5) 发布事务：缺 If-Match 428、并/陈旧 412、幂等重放、发布后改草稿不改 release、
//    发布不产生费用/外呼（AC-055/AC-056）
// ===========================================================================

#[tokio::test]
async fn publish_is_transactional_idempotent_and_immutable() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = cdn_server("/model.glb", fixture_bytes("sample-model.glb"));
    let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&model_url)]);
    let chain = chain("t19-publish", &tripo, &manual).await;
    let draft = run_job_to_draft(&chain, "t19-publish-key").await;
    let draft_id = draft.id.clone();
    let view = get_draft(&chain, &draft_id).await;
    let knowledge = draft_knowledge(&view).clone();
    let (revision_id, sha256) = model_identity(&knowledge);
    let parts = entity_ids(&knowledge, "parts");
    let upserts: Vec<Value> = parts
        .iter()
        .map(|part| hotspot_upsert(part, &revision_id, &sha256, [0.3, 0.1, 0.2]))
        .collect();
    let mut view = patch_ok(&chain, &draft_id, &view, confirm_all(&knowledge)).await;
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "hotspots": { "upsert": upserts } }),
    )
    .await;
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "modelReview": { "loaded": true, "userConfirmed": true } }),
    )
    .await;
    let etag_before = view.etag.clone();
    // ETag 形如 "r4"：记录的就是被发布的草稿内容版本。
    let publishable_revision: i64 = etag_before
        .trim_matches('"')
        .trim_start_matches('r')
        .parse()
        .expect("ETag 形如 \"r<n>\"");

    let pool = pool(&chain);
    let ledger_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger")
        .fetch_one(&pool)
        .await
        .unwrap();
    let attempts_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_attempts")
        .fetch_one(&pool)
        .await
        .unwrap();
    let fixture_calls_before = tripo.request_total() + manual.request_total() + cdn.request_total();

    // 缺 If-Match → 428（无副作用）。
    let response = chain
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
        .header("idempotency-key", "t19-pub-missing-match")
        .send()
        .await;
    assert_eq!(
        response.status,
        StatusCode::PRECONDITION_REQUIRED,
        "{}",
        response.text()
    );
    // 缺 Idempotency-Key → 422 字段级。
    let response = chain
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
        .header("if-match", &etag_before)
        .send()
        .await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    assert!(
        response.json()["error"]["details"]["fields"][0]["field"] == "idempotencyKey",
        "{}",
        response.text()
    );

    // 发布成功 → 201 不可变 release。
    let published = publish(&chain, &draft_id, &etag_before, "t19-pub-key-1").await;
    assert_eq!(
        published.status,
        StatusCode::CREATED,
        "{}",
        published.text()
    );
    let published_body = published.json();
    let release_id = published_body["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        published_body["data"]["draftRevision"],
        json!(publishable_revision),
        "release 记录发布时的 draftRevision"
    );
    assert!(
        published_body["data"]["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note.as_str().unwrap_or_default().contains("不可再修改"))
    );

    // 幂等重放（同 key 同 If-Match → 同一 release，标记 x-idempotent-replay）。
    let replay = publish(&chain, &draft_id, &etag_before, "t19-pub-key-1").await;
    assert_eq!(replay.status, StatusCode::CREATED, "{}", replay.text());
    assert_eq!(
        replay.header("x-idempotent-replay").as_deref(),
        Some("true"),
        "重放必须被标记"
    );
    assert_eq!(replay.json()["data"]["id"], release_id.as_str());
    // 发布响应给出草稿的下一个 revision（客户端据此刷新 If-Match）。
    let after_publish = replay.json()["data"]["draftRevisionAfterPublish"]
        .as_i64()
        .expect("发布响应必须给出草稿新 revision");
    assert_eq!(after_publish, publishable_revision + 1);
    // 同 key 不同 body（不同 If-Match）→ 409。
    let different_body = publish(
        &chain,
        &draft_id,
        &format!("\"r{after_publish}\""),
        "t19-pub-key-1",
    )
    .await;
    assert_eq!(
        different_body.status,
        StatusCode::CONFLICT,
        "{}",
        different_body.text()
    );
    assert_eq!(
        different_body.json()["error"]["code"],
        "IDEMPOTENCY_CONFLICT"
    );

    // 并发/陈旧 revision（新 key + 旧 If-Match）→ 412 + currentRevision。
    let stale = publish(&chain, &draft_id, &etag_before, "t19-pub-key-2").await;
    assert_eq!(
        stale.status,
        StatusCode::PRECONDITION_FAILED,
        "{}",
        stale.text()
    );
    assert_eq!(stale.json()["error"]["code"], "REVISION_CONFLICT");
    assert!(
        stale.json()["error"]["details"]["currentRevision"]
            .as_i64()
            .unwrap()
            >= 1
    );

    // 发布不产生费用、不调用供应商。
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cost_ledger")
            .fetch_one(&pool)
            .await
            .unwrap(),
        ledger_before,
        "发布不得新增费用记录"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM provider_attempts")
            .fetch_one(&pool)
            .await
            .unwrap(),
        attempts_before,
        "发布不得新增供应商调用"
    );
    assert_eq!(
        tripo.request_total() + manual.request_total() + cdn.request_total(),
        fixture_calls_before,
        "发布不得产生任何 fixture（供应商）调用"
    );

    // 发布后修改草稿：release 字节与哈希不变。
    let manifest_url = format!("/api/v1/items/{}/releases/{release_id}", chain.inputs.item);
    let before = chain
        .app
        .call(Method::GET, &manifest_url)
        .cookie(&chain.cookie)
        .send()
        .await;
    let before = before.json();
    let manifest_sha_before = before["data"]["manifestSha256"]
        .as_str()
        .unwrap()
        .to_owned();

    let view = get_draft(&chain, &draft_id).await;
    let modified = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "entities": { parts[0].clone(): { "reviewStatus": "needs_review" } } }),
    )
    .await;
    assert!(!modified.etag.is_empty());

    let after = chain
        .app
        .call(Method::GET, &manifest_url)
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(after.status, StatusCode::OK, "{}", after.text());
    let after = after.json();
    assert_eq!(
        after["data"]["manifestSha256"],
        manifest_sha_before.as_str(),
        "发布后修改草稿不得改变已发布内容"
    );
    assert_eq!(
        after["data"]["manifest"]["review"]["entities"][&parts[0]]["reviewStatus"], "confirmed",
        "旧发布版保留发布时的复核声明"
    );
    // release 行不可变（数据库触发器）：直接 UPDATE/DELETE 必须被拒绝。
    let update = sqlx::query("UPDATE manual_releases SET draft_revision = 99 WHERE id = ?")
        .bind(&release_id)
        .execute(&pool)
        .await;
    assert!(
        update.is_err(),
        "manual_releases 必须不可变（触发器拒绝 UPDATE）"
    );
    // 列表：一个 release，排序稳定。
    let list = chain
        .app
        .call(
            Method::GET,
            &format!("/api/v1/items/{}/releases", chain.inputs.item),
        )
        .cookie(&chain.cookie)
        .send()
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    let list = list.json();
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"][0]["id"], release_id.as_str());
    assert_eq!(list["data"][0]["modelRevisionId"], revision_id.as_str());

    cdn.shutdown();
    tripo.shutdown();
    manual.shutdown();
}

// ===========================================================================
// 6) 防御纵深：存储被篡改时发布仍必须拒绝（引用页/步骤引用/模型状态/冒充 confirmed）
// ===========================================================================

#[tokio::test]
async fn publish_rejects_tampered_references_and_mismatched_hotspots() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = cdn_server("/model.glb", fixture_bytes("sample-model.glb"));
    let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&model_url)]);
    let chain = chain("t19-tamper", &tripo, &manual).await;
    let draft = run_job_to_draft(&chain, "t19-tamper-key").await;
    let draft_id = draft.id.clone();
    let view = get_draft(&chain, &draft_id).await;
    let knowledge = draft_knowledge(&view).clone();
    let (revision_id, sha256) = model_identity(&knowledge);
    let parts = entity_ids(&knowledge, "parts");
    let upserts: Vec<Value> = parts
        .iter()
        .map(|part| hotspot_upsert(part, &revision_id, &sha256, [0.4, 0.2, 0.1]))
        .collect();
    let mut view = patch_ok(&chain, &draft_id, &view, confirm_all(&knowledge)).await;
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "hotspots": { "upsert": upserts } }),
    )
    .await;
    view = patch_ok(
        &chain,
        &draft_id,
        &view,
        json!({ "modelReview": { "loaded": true, "userConfirmed": true } }),
    )
    .await;
    let good_json = serde_json::to_string(draft_knowledge(&view)).unwrap();
    let pool = pool(&chain);

    let set_knowledge = |json: String| {
        let pool = pool.clone();
        let draft_id = draft_id.clone();
        async move {
            sqlx::query("UPDATE manual_drafts SET knowledge_json = ? WHERE id = ?")
                .bind(json)
                .bind(draft_id)
                .execute(&pool)
                .await
                .expect("篡改知识（测试注入）");
        }
    };

    let expect_issue = |body: &Value, code: &str| {
        let issues = body["error"]["details"]["issues"].as_array().unwrap();
        assert!(
            issues.iter().any(|issue| issue["code"] == code),
            "期望 issues 含 {code}：{issues:?}"
        );
    };

    // 引用页不存在（篡改为第 99 页）。
    let mut tampered: Value = serde_json::from_str(&good_json).unwrap();
    tampered["knowledge"]["parts"][0]["evidence"][0]["pageNumber"] = json!(99);
    set_knowledge(serde_json::to_string(&tampered).unwrap()).await;
    let response = publish(&chain, &draft_id, &view.etag, "t19-tamper-1").await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    expect_issue(&response.json(), "evidencePageMissing");

    // 步骤引用不存在的部件。
    let mut tampered: Value = serde_json::from_str(&good_json).unwrap();
    tampered["knowledge"]["steps"][0]["partIds"]
        .as_array_mut()
        .unwrap()
        .push(json!("part-ghost"));
    set_knowledge(serde_json::to_string(&tampered).unwrap()).await;
    let response = publish(&chain, &draft_id, &view.etag, "t19-tamper-2").await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    expect_issue(&response.json(), "stepPartReferenceMissing");

    // 模型被标为未通过校验。
    let mut tampered: Value = serde_json::from_str(&good_json).unwrap();
    tampered["model"]["validationState"] = json!("rejected");
    set_knowledge(serde_json::to_string(&tampered).unwrap()).await;
    let response = publish(&chain, &draft_id, &view.etag, "t19-tamper-3").await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    expect_issue(&response.json(), "modelNotValidated");

    // stale/candidate 冒充 confirmed（anchor 被改成别的 sha）。
    let mut tampered: Value = serde_json::from_str(&good_json).unwrap();
    tampered["hotspots"][0]["anchor"]["modelSha256"] = json!("c".repeat(64));
    set_knowledge(serde_json::to_string(&tampered).unwrap()).await;
    let response = publish(&chain, &draft_id, &view.etag, "t19-tamper-4").await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    expect_issue(&response.json(), "hotspotNotMatchingModel");

    // 恢复原内容（重新读草稿拿到新 revision）后可以发布。
    set_knowledge(good_json).await;
    let view = get_draft(&chain, &draft_id).await;
    let published = publish(&chain, &draft_id, &view.etag, "t19-tamper-5").await;
    assert_eq!(
        published.status,
        StatusCode::CREATED,
        "{}",
        published.text()
    );
    assert!(
        published.json()["data"]["manifestAssetId"]
            .as_str()
            .is_some()
    );

    cdn.shutdown();
    tripo.shutdown();
    manual.shutdown();
}

// ===========================================================================
// 7) 无模型的部分草稿：热点与 modelReview 写入必须拒绝（AC-052/AC-054 的边界）
// ===========================================================================

#[tokio::test]
async fn partial_draft_without_model_rejects_hotspots_and_model_review() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let full = fixture_bytes("sample-model.glb");
    let truncated = full[..full.len() / 3].to_vec();
    let (cdn, model_url) = cdn_server("/bad-model.glb", truncated);
    let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&model_url)]);
    let chain = chain("t19-partial", &tripo, &manual).await;
    let (job_status, draft) = run_job_until_draft(&chain, "t19-partial-key").await;
    assert_ne!(
        job_status,
        JobStatus::Succeeded,
        "模型分支被阻塞：父任务不得谎报成功"
    );
    let view = get_draft(&chain, &draft.id).await;
    let knowledge = draft_knowledge(&view).clone();
    assert!(
        knowledge["model"].is_null(),
        "截断模型必须被拒绝（模型分支 needs_input）：{knowledge}"
    );
    let parts = entity_ids(&knowledge, "parts");
    assert!(!parts.is_empty(), "知识分支仍应产出部件（部分成功可展示）");
    let error = patch_422(
        &chain,
        &draft.id,
        &view.etag,
        json!({ "hotspots": { "upsert": [{
            "partId": parts[0], "status": "confirmed",
            "anchor": { "modelRevisionId": "r", "modelSha256": "d".repeat(64), "positionLocal": [0.0, 0.0, 0.0] }
        }] } }),
    )
    .await;
    assert!(error.to_string().contains("旧模型版本"), "{error}");
    let error = patch_422(
        &chain,
        &draft.id,
        &view.etag,
        json!({ "modelReview": { "loaded": true, "userConfirmed": true } }),
    )
    .await;
    assert!(error.to_string().contains("没有可用的模型版本"), "{error}");

    cdn.shutdown();
    tripo.shutdown();
    manual.shutdown();
}
