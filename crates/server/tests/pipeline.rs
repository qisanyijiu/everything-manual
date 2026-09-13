//! T15 集成测试：**两条分支组装草稿**（PRD 修订 2 / ui_revision 2；REQ-030 主，
//! AC-047、AC-048，以及 AC-037/AC-039/AC-040 的端点侧）。
//!
//! 覆盖（命令 ↔ AC 见 implementation.md §T15）：
//! - **fixture 全链路成功**：知识分支（`manual_extract` → `manual_merge`）与模型分支
//!   （`tripo_*` → `model_validate`）独立完成 → `assemble_draft` → 草稿
//!   `needs_review`；父 job `succeeded` **不等于已发布**（`manual_releases` 为空，
//!   不存在 publish/自动发布路径）；
//! - **分支独立与部分成功**：模型成功、知识失败 → 只重试知识分支（Tripo 付费提交计数
//!   不变）；反向（知识成功、模型失败）只重试模型分支（`manual_merge` 结果资产不变）；
//!   分支头被阻塞时产出**部分草稿**并标明缺项；
//! - **幂等**：重启/重放/重跑组装不重复创建草稿（同 id、同 revision）；
//! - **unknown**：不提供可用 retry（被拒且无副作用）；`reconcile` 三种动作的正/负例
//!   （未登录 401；`attachRemoteTask` 对同步 Manual AI 不可用、地址验证失败无副作用；
//!   `recordNoTask` 要求核查证据；`authorizeReplacement` 要求预算确认与重复收费确认）；
//! - **取消**：未提交阶段不再推进；已提交阶段保留查询与账务；取消后**不新增任何付费
//!   步骤**（fixture 计数断言）；审计记录可查；
//! - **草稿契约**：读取带 `ETag`；PATCH 缺 `If-Match` 428、过期 412、未知字段 422；
//! - **T14 P3-1**：拒答批次在"结果已落库、checkpoint 未推进"的恢复路径按
//!   `producedKnowledge` 判定为 `needs_input`（不得标记成"产出了知识"）。
//!
//! 隔离与门控：全部 HTTP 指向 T05 的本机 fixture（`127.0.0.1`，随机端口），
//! **零真实外网调用**（fixture 只绑定回环 + 调用计数断言；测试使用假凭据 canary）。

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
use manual_core::domain::{
    DraftStatus, Job, JobStage, JobStatus, LedgerState, ManualDraft, ProviderKey, StageKind,
    SubmitState,
};
use manual_core::timestamps::Timestamp;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use test_support::FixtureServer;
use test_support::scenario::{BodySpec, PathMatchSpec, ResponseSpec, RouteScript, Scenario, Step};

const PASSWORD: &str = "test-password-t15-a410";
/// 测试用假凭据（canary）：断言不得出现在日志/记录/Debug 输出里。
const CANARY_KEY: &str = "canary-t15-not-a-real-key";
const MANUAL_AI_MODEL: &str = "gpt-5-mini";
const PRESET: &str = "tripo-h-v3.1-standard";

/// 测试价格目录（与 T11–T14 用例同价）。
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
const TASK_ID: &str = "t15-fixture-task-0001";

// ---------------------------------------------------------------------------
// fixture 工具（与 T13/T14 用例同构）
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

fn respond_json_status(status: u16, value: Value) -> Step {
    Step::Respond {
        response: ResponseSpec {
            status,
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

fn exact_route_repeat(method: &str, path: &str, steps: Vec<Step>) -> RouteScript {
    RouteScript {
        method: method.to_owned(),
        path: path.to_owned(),
        path_match: PathMatchSpec::Exact,
        repeat_last: true,
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

fn scenario(routes: Vec<RouteScript>) -> Scenario {
    Scenario::new(routes)
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

fn tripo_response(name: &str) -> String {
    format!("responses/tripo/{name}")
}

/// 说明书 AI fixture：`POST /v1/responses` 按给定步骤响应。
fn manual_ai_server(steps: Vec<Step>) -> FixtureServer {
    FixtureServer::start(scenario(vec![exact_route("POST", MANUAL_AI_PATH, steps)]))
}

fn manual_ai_base_url(server: &FixtureServer) -> String {
    format!("{}/v1", server.base_url())
}

/// Tripo API fixture：上传 / 提交 / 查询三条路由。
fn tripo_server(
    upload_steps: Vec<Step>,
    submit_steps: Vec<Step>,
    poll_steps: Vec<Step>,
    poll_repeat: bool,
) -> FixtureServer {
    FixtureServer::start(scenario(vec![
        exact_route("POST", TRIPO_UPLOAD_PATH, upload_steps),
        exact_route("POST", TRIPO_SUBMIT_PATH, submit_steps),
        prefix_route("GET", TRIPO_TASKS_PREFIX, poll_repeat, poll_steps),
    ]))
}

fn tripo_base_url(server: &FixtureServer) -> String {
    format!("{}/v3", server.base_url())
}

fn upload_steps() -> Vec<Step> {
    vec![
        respond_json(json!({ "code": 0, "data": { "file_token": "token-front-0001" } })),
        respond_json(json!({ "code": 0, "data": { "file_token": "token-left-0001" } })),
    ]
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

fn task_running() -> Step {
    respond_json(json!({
        "code": 0,
        "data": { "task_id": TASK_ID, "status": "running", "progress": 42 }
    }))
}

/// 模型 CDN fixture（下载阶段的目标）。
fn cdn_server(path: &str, body: BodySpec) -> FixtureServer {
    FixtureServer::start(scenario(vec![exact_route(
        "GET",
        path,
        vec![Step::Respond {
            response: ResponseSpec {
                status: 200,
                headers: BTreeMap::new(),
                body,
            },
        }],
    )]))
}

/// 正常 GLB 的 CDN。
fn glb_cdn() -> (FixtureServer, String) {
    let server = cdn_server(
        "/model.glb",
        BodySpec::File {
            file: "assets/sample-model.glb".to_owned(),
        },
    );
    let url = format!(
        "{}/model.glb?sign=fixture-signature-secret",
        server.base_url()
    );
    (server, url)
}

/// 截断 GLB 的 CDN（`model_validate` 必须拒绝 → needs_input，模型分支阻塞）。
///
/// 下载阶段不做结构校验（T13：结构检查在 `model_validate`），因此这里只要能提供一个
/// 字节层面合法的 HTTP 响应即可；截断文件的完整副本写在系统临时目录（测试隔离）。
fn truncated_glb_cdn() -> (FixtureServer, String) {
    let full = fixture_bytes("sample-model.glb");
    let truncated = full[..full.len() / 3].to_vec();
    let path = std::env::temp_dir().join(format!(
        "em-t15-truncated-glb-{}-{}.glb",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::write(&path, &truncated).expect("写入截断 GLB");
    let server = cdn_server(
        "/bad-model.glb",
        BodySpec::File {
            file: path.to_string_lossy().into_owned(),
        },
    );
    let url = format!("{}/bad-model.glb", server.base_url());
    (server, url)
}

// ---------------------------------------------------------------------------
// 应用与输入准备
// ---------------------------------------------------------------------------

/// 两个 Provider 都已配置（指向本机 fixture）+ 下载显式放行本机 fixture 的测试应用。
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
            boundary: format!("----em-t15-{tag}"),
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

struct UploadSpec<'a> {
    purpose: &'a str,
    filename: &'a str,
    content_type: &'a str,
    bytes: &'a [u8],
}

async fn upload_asset(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    spec: UploadSpec<'_>,
) -> String {
    let (boundary, body) = Multipart::new(spec.purpose)
        .text_field("purpose", spec.purpose)
        .file_field("file", spec.filename, spec.content_type, spec.bytes)
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

/// 物品 + PDF（3 页文字）+ front/left 照片 + ready 准备。
async fn build_ready_inputs(app: &TestApp, cookie: &str, csrf: &str) -> ReadyInputs {
    let item = {
        let response = app
            .call(Method::POST, "/api/v1/items")
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({ "name": "组装测试物品", "model": "X100V" }))
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
        UploadSpec {
            purpose: "document",
            filename: "manual.pdf",
            content_type: "application/pdf",
            bytes: &pdf,
        },
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
            UploadSpec {
                purpose: "pageImage",
                filename: "page.jpg",
                content_type: "image/jpeg",
                bytes: &page_jpeg,
            },
        )
        .await;
        let text_asset = upload_asset(
            app,
            cookie,
            csrf,
            &item,
            UploadSpec {
                purpose: "pageText",
                filename: "page.txt",
                content_type: "text/plain",
                bytes: format!("T15-PAGE-{page}-CONTENT 后盖 螺钉 电池").as_bytes(),
            },
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
            UploadSpec {
                purpose: "photo",
                filename: name,
                content_type,
                bytes: &bytes,
            },
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

/// 报价 → 确认 → 建单（返回 job id）。
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
            "limits": { "tripoCreditMinor": 3000, "manualAiUsdMicros": 100_000 },
        }))
        .send()
        .await;
    assert_eq!(job.status, StatusCode::ACCEPTED, "{}", job.text());
    job.json()["data"]["id"].as_str().unwrap().to_owned()
}

// ---------------------------------------------------------------------------
// 执行器与断言工具
// ---------------------------------------------------------------------------

fn pool(app: &TestApp) -> SqlitePool {
    app.state().database().pool().clone()
}

/// 生产接线：Provider 处理器 + 组装处理器（T15 起 `serve` 同时注册两者）。
fn pipeline_executor(app: &TestApp, clock: Arc<ManualClock>) -> Arc<JobExecutor> {
    let settings = app.state().settings().clone();
    let mut registry = StageRegistry::new();
    register_provider_handlers(&mut registry, &settings).expect("已配置的 Provider 必须能注册");
    let pipeline = PipelineHandlers::from_settings(&settings);
    let registered = pipeline.register(&mut registry);
    assert!(
        registered.contains(&StageKind::AssembleDraft),
        "组装阶段必须注册（本地阶段，不依赖 Provider）"
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

async fn run_ticks(
    executor: &Arc<JobExecutor>,
    clock: &ManualClock,
    ticks: usize,
    step_millis: i64,
) -> Vec<everything_manual::jobs::StageRunReport> {
    let mut reports = Vec::new();
    for _ in 0..ticks {
        clock.advance_millis(step_millis);
        match executor.tick().await.expect("tick") {
            TickOutcome::Executed(report) => reports.push(report),
            TickOutcome::Idle => {}
        }
    }
    reports
}

async fn stage_of(pool: &SqlitePool, job_id: &str, kind: StageKind) -> JobStage {
    let mut conn = pool.acquire().await.expect("连接");
    repo::job_stages::list_for_job(&mut conn, job_id)
        .await
        .expect("列出阶段")
        .into_iter()
        .find(|stage| stage.stage_kind == kind)
        .unwrap_or_else(|| panic!("阶段不存在：{}", kind.as_str()))
}

async fn stage_status(pool: &SqlitePool, job_id: &str, kind: StageKind) -> JobStatus {
    stage_of(pool, job_id, kind).await.status
}

async fn stage_by_id(pool: &SqlitePool, stage_id: &str) -> JobStage {
    let mut conn = pool.acquire().await.expect("连接");
    repo::job_stages::get(&mut conn, stage_id)
        .await
        .expect("读取阶段")
        .expect("阶段必须存在")
}

async fn job_row(pool: &SqlitePool, job_id: &str) -> Job {
    let mut conn = pool.acquire().await.expect("连接");
    repo::jobs::get(&mut conn, job_id)
        .await
        .expect("读取任务")
        .expect("任务必须存在")
}

/// 反复 tick（每次推进 20s）直到目标阶段达到期望状态。
async fn tick_until_stage(
    pool: &SqlitePool,
    executor: &Arc<JobExecutor>,
    clock: &ManualClock,
    job_id: &str,
    kind: StageKind,
    want: JobStatus,
    max_ticks: usize,
) {
    for _ in 0..max_ticks {
        if stage_status(pool, job_id, kind).await == want {
            return;
        }
        run_ticks(executor, clock, 1, 20_000).await;
    }
    let stage = stage_of(pool, job_id, kind).await;
    panic!(
        "阶段 {} 未在 {max_ticks} tick 内达到 {}（实际 {}；last_error={:?}；needs_input={:?}）",
        kind.as_str(),
        want.as_str(),
        stage.status.as_str(),
        stage.last_error,
        stage.needs_input_json
    );
}

async fn draft_for_snapshot(pool: &SqlitePool, snapshot_id: &str) -> Option<ManualDraft> {
    let mut conn = pool.acquire().await.expect("连接");
    repo::drafts::get_by_snapshot(&mut conn, snapshot_id)
        .await
        .expect("读取草稿")
}

async fn releases_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM manual_releases")
        .fetch_one(pool)
        .await
        .expect("统计 releases")
}

async fn audit_count(pool: &SqlitePool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM audit_events WHERE action = ?")
        .bind(action)
        .fetch_one(pool)
        .await
        .expect("统计审计事件")
}

async fn ledger_entry(
    pool: &SqlitePool,
    snapshot_id: &str,
    provider: ProviderKey,
) -> Option<manual_core::domain::CostLedgerEntry> {
    let mut conn = pool.acquire().await.expect("连接");
    repo::ledger::list_for_snapshot(&mut conn, snapshot_id)
        .await
        .expect("读取账本")
        .into_iter()
        .find(|entry| entry.provider == provider)
}

/// 任务详情的 ETag（后续 cancel/retry/reconcile 的 If-Match）。
async fn job_etag(app: &TestApp, cookie: &str, job_id: &str) -> String {
    let response = app
        .call(Method::GET, &format!("/api/v1/jobs/{job_id}"))
        .cookie(cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.header("etag").expect("任务详情必须带 ETag")
}

async fn get_job_json(app: &TestApp, cookie: &str, job_id: &str) -> Value {
    let response = app
        .call(Method::GET, &format!("/api/v1/jobs/{job_id}"))
        .cookie(cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json()
}

/// 模拟"结果已落库、checkpoint 未推进"的崩溃现场（T14 P3-1 的恢复路径）。
async fn crash_stage_before_checkpoint(pool: &SqlitePool, stage_id: &str) {
    let now = Timestamp::now().as_millis();
    sqlx::query(
        "UPDATE job_stages SET status = 'running', lease_owner = 'crashed-worker-t15', \
            lease_until = ?, updated_at = ? WHERE id = ?",
    )
    .bind(now - 10_000)
    .bind(now)
    .bind(stage_id)
    .execute(pool)
    .await
    .expect("构造崩溃现场");
}

/// 恢复扫描（不带处理器的执行器：只收敛过期租约）。
async fn recover_with_empty_registry(
    app: &TestApp,
    clock: Arc<ManualClock>,
) -> everything_manual::jobs::RecoveryReport {
    let executor = fixed_jitter_executor(
        pool(app),
        ExecutorConfig::default(),
        StageRegistry::new(),
        clock,
    );
    executor.recover_expired_leases().await.expect("恢复扫描")
}

// ===========================================================================
// 1) fixture 全链路成功 → 草稿 needs_review；job succeeded ≠ published
// ===========================================================================

#[tokio::test]
async fn full_chain_assembles_needs_review_draft_and_never_publishes() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_running(), task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t15-full-chain",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t15-full-chain-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::AssembleDraft,
        JobStatus::Succeeded,
        80,
    )
    .await;

    // 两条分支各自完成。
    for kind in [
        StageKind::ManualExtract,
        StageKind::ManualMerge,
        StageKind::TripoUpload,
        StageKind::TripoSubmit,
        StageKind::TripoPoll,
        StageKind::ModelDownload,
        StageKind::ModelValidate,
    ] {
        assert_eq!(
            stage_status(&pool, &job_id, kind).await,
            JobStatus::Succeeded,
            "阶段 {} 必须独立完成",
            kind.as_str()
        );
    }

    // 父 job succeeded = 可复核草稿已产出（不是已发布）。
    let job = job_row(&pool, &job_id).await;
    assert_eq!(job.status, JobStatus::Succeeded);
    let draft = draft_for_snapshot(&pool, &job.snapshot_id)
        .await
        .expect("组装后必须存在草稿");
    assert_eq!(draft.status, DraftStatus::NeedsReview);
    assert_eq!(draft.revision, 1, "首次组装 revision = 1");
    assert!(
        draft.model_revision_id.is_some(),
        "模型分支完成 → 草稿引用不可变模型版本"
    );

    // **不存在自动发布路径**：装配完成后 releases 仍为空；publish 是显式动作
    // （T19 起路由存在），未携带 If-Match/Idempotency-Key 的调用不会发布任何内容。
    // 事实更新（T19 交付回合，2026-09-12）：原断言"publish 路由不存在（404）"不成立；
    // 此处改为断言显式动作的前置条件与副作用，守卫本身不弱化（releases 必须仍为 0）。
    assert_eq!(
        releases_count(&pool).await,
        0,
        "不得自动发布（不存在 release）"
    );
    let publish_attempt = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{}/drafts/{}/publish", inputs.item, draft.id),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({}))
        .send()
        .await;
    assert_eq!(
        publish_attempt.status,
        StatusCode::PRECONDITION_REQUIRED,
        "publish 是显式动作：缺 If-Match 时不得发布（428；实际 {}）",
        publish_attempt.text()
    );
    assert_eq!(
        releases_count(&pool).await,
        0,
        "被拒绝的 publish 不得产生 release"
    );

    // 草稿知识聚合：完整（无缺项）+ 模型/知识两侧都在。
    let knowledge = draft.knowledge_json.clone();
    assert_eq!(knowledge["completeness"], "complete");
    assert!(knowledge["missing"].as_array().unwrap().is_empty());
    assert!(knowledge["model"]["sha256"].as_str().is_some());
    assert!(knowledge["knowledge"]["parts"].as_array().unwrap().len() >= 2);
    assert!(
        knowledge["knowledge"]["coverage"]["complete"]
            .as_bool()
            .unwrap_or(false)
    );

    // HTTP 契约：草稿读取带 ETag；任务详情暴露 draftId 与知识产出事实。
    let draft_get = app
        .call(
            Method::GET,
            &format!("/api/v1/items/{}/drafts/{}", inputs.item, draft.id),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(draft_get.status, StatusCode::OK, "{}", draft_get.text());
    assert_eq!(draft_get.header("etag").as_deref(), Some("\"r1\""));
    let body = draft_get.json();
    assert_eq!(body["data"]["status"], "needs_review");
    assert_eq!(body["data"]["completeness"], "complete");
    assert!(
        body["data"]["notices"][0]
            .as_str()
            .unwrap_or_default()
            .contains("不等于已发布"),
        "草稿必须携带固定语义说明"
    );

    let detail = get_job_json(&app, &cookie, &job_id).await;
    assert_eq!(detail["data"]["draftId"], draft.id);
    let batch = detail["data"]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["stageKind"] == "manual_extract")
        .expect("批次阶段");
    assert_eq!(
        batch["knowledgeProduced"], true,
        "成功批次的知识产出事实必须可读（与恢复同判据）"
    );
    // 阶段事实里**不出现临时供应商签名地址**（T20/BUG-008；contracts §1/§7、AC-010）。
    let raw_detail = detail.to_string();
    assert!(
        !raw_detail.contains("sign=fixture-signature-secret"),
        "任务详情不得输出供应商签名查询串"
    );
    assert!(
        !raw_detail.contains("http://127.0.0.1"),
        "任务详情不得输出完整 URL（只保留 sha256 摘要 + host）"
    );
    assert!(
        !raw_detail.contains("://"),
        "任务详情整体不得出现 URL 形态字符串：{raw_detail}"
    );
    let poll_stage = detail["data"]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["stageKind"] == "tripo_poll")
        .expect("查询阶段");
    assert_eq!(
        poll_stage["usage"]["modelUrl"]["redacted"], true,
        "临时模型地址只应是摘要对象：{poll_stage}"
    );
    assert_eq!(poll_stage["usage"]["modelUrl"]["host"], "127.0.0.1");
    assert_eq!(
        poll_stage["usage"]["normalizedStatus"], "success",
        "脱敏不得丢掉可诊断的事实（状态/计数保留）"
    );

    // 付费提交各一次：组装/下载/校验不产生新费用。
    tripo.assert_called_times("POST", TRIPO_SUBMIT_PATH, 1);
    manual.assert_called_times("POST", MANUAL_AI_PATH, 1);
    assert_eq!(cdn.call_count("GET", "/model.glb"), 1);
    tripo.assert_no_script_problems();
    manual.assert_no_script_problems();
}

// ===========================================================================
// 1b) 任务详情：历史行（修复前落库）的临时 URL 读取侧脱敏（T20/BUG-008）
// ===========================================================================

/// 历史 `usage_json` 里的完整供应商签名 URL → 详情返回摘要对象（无 URL、无签名），
/// 但 task ID 等可诊断事实保留（AC-010；contracts §1/§7）。
#[tokio::test]
async fn job_detail_redacts_historical_signed_urls() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_running(), task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t20-historical-url-detail",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t20-historical-url-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::AssembleDraft,
        JobStatus::Succeeded,
        80,
    )
    .await;

    // 造"修复前落库"的现场：poll 阶段事实里直接是完整签名 URL（历史形态）。
    let historical = format!("{}/model.glb?sign=canary-historical-detail", cdn.base_url());
    let historical_usage = json!({
        "remoteTaskId": TASK_ID,
        "rawStatus": "success",
        "normalizedStatus": "success",
        "modelUrl": historical,
        "renderedImageUrl": "https://cdn.example.invalid/preview.png",
    });
    let updated = sqlx::query(
        "UPDATE job_stages SET usage_json = ? WHERE job_id = ? AND stage_kind = 'tripo_poll'",
    )
    .bind(historical_usage.to_string())
    .bind(&job_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(updated.rows_affected(), 1, "用例前提：存在 tripo_poll 阶段");

    let detail = get_job_json(&app, &cookie, &job_id).await;
    let raw = detail.to_string();
    assert!(!raw.contains("://"), "详情不得含 URL：{raw}");
    assert!(
        !raw.contains("canary-historical-detail"),
        "详情不得含签名：{raw}"
    );
    assert!(
        !raw.contains("/model.glb") && !raw.contains("/preview.png"),
        "详情不得含 URL path：{raw}"
    );
    let poll = detail["data"]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["stageKind"] == "tripo_poll")
        .expect("查询阶段");
    assert_eq!(poll["usage"]["modelUrl"]["redacted"], true);
    assert_eq!(
        poll["usage"]["modelUrl"]["sha256"],
        everything_manual::redaction::url_summary(&historical).sha256_prefix,
        "摘要可复核（sha256 前缀）"
    );
    assert_eq!(poll["usage"]["remoteTaskId"], TASK_ID, "task ID 保留");
    assert_eq!(poll["usage"]["normalizedStatus"], "success", "状态保留");
}

// ===========================================================================
// 2) 组装幂等：重启/重放/重跑不重复创建 draft
// ===========================================================================

#[tokio::test]
async fn replayed_assembly_does_not_create_a_second_draft() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (_cdn, model_url) = glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t15-idempotent",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t15-idempotent-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::AssembleDraft,
        JobStatus::Succeeded,
        80,
    )
    .await;
    let job = job_row(&pool, &job_id).await;
    let draft = draft_for_snapshot(&pool, &job.snapshot_id)
        .await
        .expect("草稿");
    assert_eq!(draft.revision, 1);

    // 重放 1：直接再次组装（同一输入 → 内容不变 → 不新建、不递增 revision）。
    let again = everything_manual::drafts::assemble_draft(&pool, app.dir(), &job, Timestamp::now())
        .await
        .expect("重复组装必须成功");
    assert!(!again.created, "重复组装不得新建草稿");
    assert_eq!(again.draft.id, draft.id);
    assert_eq!(again.draft.revision, 1, "内容相同不得递增 revision");

    // 重放 2：模拟"崩溃在结果落库与 checkpoint 之间"→ 恢复重跑组装阶段。
    crash_stage_before_checkpoint(
        &pool,
        &stage_of(&pool, &job_id, StageKind::AssembleDraft).await.id,
    )
    .await;
    let report = recover_with_empty_registry(&app, Arc::clone(&clock)).await;
    assert_eq!(report.requeued, 1, "无未决事实 → 重新入队（不假成功）");
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::AssembleDraft,
        JobStatus::Succeeded,
        10,
    )
    .await;

    assert_eq!(
        repo::drafts::count_for_item(&mut pool.acquire().await.unwrap(), &inputs.item)
            .await
            .expect("统计草稿"),
        1,
        "重启/重放不得产生第二份草稿"
    );
    let after = draft_for_snapshot(&pool, &job.snapshot_id)
        .await
        .expect("草稿");
    assert_eq!(after.id, draft.id);
    assert_eq!(after.revision, 1, "同一内容重跑不产生新版本");
    // 组装不产生任何新外呼。
    tripo.assert_called_times("POST", TRIPO_SUBMIT_PATH, 1);
    manual.assert_called_times("POST", MANUAL_AI_PATH, 1);
}

// ===========================================================================
// 3) 模型成功、知识失败 → 只重试知识分支（反向亦然）
// ===========================================================================

#[tokio::test]
async fn knowledge_failure_retries_only_the_knowledge_branch() {
    // 第一次提取拒答（needs_input），重试后成功。
    let manual = manual_ai_server(vec![
        respond_file(&responses_path("refusal.json")),
        respond_file(&responses_path("success.json")),
    ]);
    let (cdn, model_url) = glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t15-knowledge-retry",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t15-knowledge-retry-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualExtract,
        JobStatus::NeedsInput,
        40,
    )
    .await;
    // 模型分支继续独立完成；知识分支阻塞 → merge 不解锁（T10 语义）。
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::Succeeded,
        40,
    )
    .await;
    assert_eq!(
        stage_status(&pool, &job_id, StageKind::ManualMerge).await,
        JobStatus::Queued,
        "批次未成功 → merge 不解锁"
    );
    let validate_stage = stage_of(&pool, &job_id, StageKind::ModelValidate).await;
    let model_revision = validate_stage.usage_json.as_ref().unwrap()["modelRevisionId"]
        .as_str()
        .expect("validate 阶段记录 modelRevisionId")
        .to_owned();
    let model_validate_usage = validate_stage.usage_json.clone();
    let detail = get_job_json(&app, &cookie, &job_id).await;
    assert_eq!(detail["data"]["status"], "needs_input");
    let batch_stage = stage_of(&pool, &job_id, StageKind::ManualExtract).await;
    assert_eq!(
        batch_stage.usage_json.as_ref().unwrap()["producedKnowledge"],
        false,
        "拒答批次的事实必须如实（未产出正式知识）"
    );

    // 重试只作用于知识分支。
    let etag = job_etag(&app, &cookie, &job_id).await;
    let retry = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .header("idempotency-key", "t15-knowledge-retry-action")
        .json(&json!({ "stageId": batch_stage.id }))
        .send()
        .await;
    assert_eq!(retry.status, StatusCode::OK, "{}", retry.text());
    let retry_body = retry.json();
    assert_eq!(retry_body["data"]["stageId"], batch_stage.id);
    assert_eq!(retry_body["data"]["previousStatus"], "needs_input");
    assert_eq!(retry_body["data"]["requeuedDependents"], 0);

    // 幂等：同 key 同 body 重放不产生第二个 attempt。
    let replay = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &job_etag(&app, &cookie, &job_id).await)
        .header("idempotency-key", "t15-knowledge-retry-action")
        .json(&json!({ "stageId": batch_stage.id }))
        .send()
        .await;
    assert_eq!(replay.status, StatusCode::OK, "{}", replay.text());
    assert_eq!(
        replay.header("x-idempotent-replay").as_deref(),
        Some("true"),
        "同键同 body 必须按重放返回"
    );

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::AssembleDraft,
        JobStatus::Succeeded,
        60,
    )
    .await;
    let job = job_row(&pool, &job_id).await;
    assert_eq!(job.status, JobStatus::Succeeded);
    let draft = draft_for_snapshot(&pool, &job.snapshot_id)
        .await
        .expect("重试后必须产出完整草稿");
    assert_eq!(draft.knowledge_json["completeness"], "complete");

    // 模型分支的成果没有被覆盖：同一 revision、同一 usage、CDN 只下载一次；
    // Tripo 付费提交仍是一次（重试没有偷偷重新购买）。
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::ModelValidate)
            .await
            .usage_json,
        model_validate_usage
    );
    {
        let mut conn = pool.acquire().await.unwrap();
        let revision = repo::model_revisions::get(&mut conn, &model_revision)
            .await
            .unwrap()
            .expect("revision 必须仍在");
        assert_eq!(
            draft.model_revision_id.as_deref(),
            Some(model_revision.as_str())
        );
        assert_eq!(
            revision.validation_state,
            manual_core::domain::ModelValidationState::Validated
        );
    }
    tripo.assert_called_times("POST", TRIPO_SUBMIT_PATH, 1);
    assert_eq!(cdn.call_count("GET", "/model.glb"), 1);
    manual.assert_called_times("POST", MANUAL_AI_PATH, 2);
    assert_eq!(audit_count(&pool, "job_stage_retry_requested").await, 1);
}

/// T17：任务详情的 `stages[].retry` 必须与 `POST /jobs/{id}/retry` 的判定一致
/// （界面只在 `allowed=true` 时渲染重试按钮；被拒时照实显示原因）。
///
/// 现场：知识批次拒答（`needs_input`，manual_ai 预留仍占用 → 可重试）；
/// 模型分支远端成功但 GLB 截断（`needs_input`，tripo 已结算 → `budgetNotHolding`）。
#[tokio::test]
async fn job_detail_retry_availability_matches_the_retry_endpoint() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("refusal.json"))]);
    let (cdn, model_url) = truncated_glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t17-retry-gate",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t17-retry-gate-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualExtract,
        JobStatus::NeedsInput,
        40,
    )
    .await;
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::NeedsInput,
        40,
    )
    .await;

    let detail = get_job_json(&app, &cookie, &job_id).await;
    assert_eq!(detail["data"]["status"], "needs_input");
    let stage_json = |kind: &str| -> Value {
        detail["data"]["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stage| stage["stageKind"] == kind)
            .unwrap_or_else(|| panic!("找不到阶段 {kind}：{detail}"))
            .clone()
    };

    let batch = stage_json("manual_extract");
    assert_eq!(batch["status"], "needs_input");
    assert_eq!(
        batch["retry"]["allowed"], true,
        "知识分支预留仍占用预算 → 如实显示可重试：{batch}"
    );
    assert!(batch["retry"]["reason"].is_null());
    assert_eq!(
        batch["submissionStyle"], "syncResponse",
        "同步批次：对账不提供 attachRemoteTask"
    );

    let validate = stage_json("model_validate");
    assert_eq!(validate["status"], "needs_input");
    assert_eq!(
        validate["retry"]["allowed"], false,
        "tripo 已结算 → 重试没有预算背书，必须如实显示不可重试：{validate}"
    );
    assert_eq!(validate["retry"]["reason"], "budgetNotHolding");
    assert!(validate["submissionStyle"].is_null());

    let submit = stage_json("tripo_submit");
    assert_eq!(
        submit["retry"]["reason"], "stageNotRetryable",
        "succeeded 的阶段不是重试入口：{submit}"
    );
    assert_eq!(submit["submissionStyle"], "asyncRemoteTask");

    // 与端点交叉核对：详情说"不可重试"就必须真的被拒（同 reason）；
    // 详情说"可重试"就必须真的被接受。
    let etag = job_etag(&app, &cookie, &job_id).await;
    let rejected = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .header("idempotency-key", "t17-retry-gate-model")
        .json(&json!({ "stageId": validate["id"] }))
        .send()
        .await;
    assert_eq!(
        rejected.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        rejected.text()
    );
    assert_eq!(
        rejected.json()["error"]["details"]["reason"],
        validate["retry"]["reason"],
        "详情与端点的判据必须同源"
    );

    let accepted = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .header("idempotency-key", "t17-retry-gate-batch")
        .json(&json!({ "stageId": batch["id"] }))
        .send()
        .await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.text());
    assert_eq!(accepted.json()["data"]["previousStatus"], "needs_input");

    // 拒答批次仍未产出正式知识；重试只改该阶段状态（付费提交仍是 1 次）。
    assert_eq!(
        stage_status(&pool, &job_id, StageKind::ManualMerge).await,
        JobStatus::Queued
    );
    tripo.assert_called_times("POST", TRIPO_SUBMIT_PATH, 1);
    assert_eq!(cdn.call_count("GET", "/bad-model.glb"), 1);
}

#[tokio::test]
async fn model_failure_retries_only_the_model_branch() {
    // 远端任务被封禁（终态失败，不释放预留）→ 人工重试后成功。
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (_cdn, model_url) = glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![
            respond_file(&tripo_response("task_banned.json")),
            task_success_with(&model_url),
        ],
        true,
    );
    let app = pipeline_app(
        "t15-model-retry",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t15-model-retry-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoPoll,
        JobStatus::Failed,
        40,
    )
    .await;
    // 知识分支独立完成（并行推进）。
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualMerge,
        JobStatus::Succeeded,
        40,
    )
    .await;
    let merge_stage = stage_of(&pool, &job_id, StageKind::ManualMerge).await;
    let merge_asset = merge_stage.result_asset_id.clone().expect("合并结果资产");
    assert_eq!(
        stage_status(&pool, &job_id, StageKind::AssembleDraft).await,
        JobStatus::Queued,
        "模型分支头未定性 → 组装保持排队（T10 语义）"
    );

    let poll_stage = stage_of(&pool, &job_id, StageKind::TripoPoll).await;
    let etag = job_etag(&app, &cookie, &job_id).await;
    let retry = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .header("idempotency-key", "t15-model-retry-action")
        .json(&json!({ "stageId": poll_stage.id }))
        .send()
        .await;
    assert_eq!(retry.status, StatusCode::OK, "{}", retry.text());

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::AssembleDraft,
        JobStatus::Succeeded,
        80,
    )
    .await;
    let job = job_row(&pool, &job_id).await;
    assert_eq!(job.status, JobStatus::Succeeded);
    let draft = draft_for_snapshot(&pool, &job.snapshot_id)
        .await
        .expect("草稿");
    assert_eq!(draft.knowledge_json["completeness"], "complete");

    // 只重跑 tripo_poll：付费提交没有被重新执行（POST 计数仍为 1），
    // 知识分支的合并结果资产没有被覆盖。
    tripo.assert_called_times("POST", TRIPO_SUBMIT_PATH, 1);
    manual.assert_called_times("POST", MANUAL_AI_PATH, 1);
    assert_eq!(
        stage_of(&pool, &job_id, StageKind::ManualMerge)
            .await
            .result_asset_id
            .as_deref(),
        Some(merge_asset.as_str()),
        "已完成分支的结果资产保留"
    );
}

// ===========================================================================
// 4) 部分成功：分支头被阻塞 → 草稿仍产出并标明缺项
// ===========================================================================

#[tokio::test]
async fn blocked_model_branch_still_produces_a_partial_draft_with_missing_items() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = truncated_glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t15-partial",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t15-partial-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));

    // 模型校验拒绝（截断 GLB）→ 模型分支头阻塞；知识分支完整。
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
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::AssembleDraft,
        JobStatus::Succeeded,
        40,
    )
    .await;

    let job = job_row(&pool, &job_id).await;
    assert_eq!(
        job.status,
        JobStatus::NeedsInput,
        "部分成功：父 job 显示阻塞分支（不是 succeeded）"
    );
    let draft = draft_for_snapshot(&pool, &job.snapshot_id)
        .await
        .expect("部分成功也要产出草稿");
    assert_eq!(draft.status, DraftStatus::NeedsReview);
    assert_eq!(draft.knowledge_json["completeness"], "partial");
    assert!(
        draft.model_revision_id.is_none(),
        "模型分支未完成 → 草稿不含模型版本"
    );
    let missing = draft.knowledge_json["missing"].as_array().unwrap();
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!(missing[0]["code"], "model_branch_incomplete");
    let missing_message = missing[0]["message"].as_str().unwrap_or_default();
    assert!(
        missing_message.contains("model_validate"),
        "缺项必须指明阻塞的阶段：{missing_message}"
    );
    // T17 / T15 P3①：草稿缺项文案不得承诺"可对该阶段重试"——该分支的 tripo 预留
    // 已结算（`tripo_poll` 成功即结算），retry 端点会以 `budgetNotHolding` 拒绝；
    // 可执行动作一律以任务详情的 `retry` 字段为准。
    assert!(
        !missing_message.contains("可对该阶段重试"),
        "缺项文案不得承诺可重试（实际被 budgetNotHolding 拒绝）：{missing_message}"
    );
    assert!(
        missing_message.contains("任务中心"),
        "缺项文案必须指向照实呈现可用动作的任务中心：{missing_message}"
    );
    assert!(
        draft.knowledge_json["knowledge"]["parts"]
            .as_array()
            .is_some(),
        "知识分支的完整产物仍保留"
    );
    assert_eq!(releases_count(&pool).await, 0, "部分草稿同样不得自动发布");

    // 缺项经 HTTP 可读（任务详情 + 草稿读取）。
    let detail = get_job_json(&app, &cookie, &job_id).await;
    assert_eq!(detail["data"]["status"], "needs_input");
    let validate = detail["data"]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["stageKind"] == "model_validate")
        .expect("校验阶段");
    assert_eq!(validate["status"], "needs_input");
    assert!(
        !validate["needsInput"].as_array().unwrap().is_empty(),
        "阻塞阶段必须给出可行动缺项"
    );
    // T17 / T15 P3①：任务详情如实给出重试准入（与服务端端点同源判据）。
    assert_eq!(
        validate["retry"]["allowed"], false,
        "模型分支头阻塞的现场必须如实显示不可重试：{validate}"
    );
    assert_eq!(validate["retry"]["reason"], "budgetNotHolding");
    assert!(
        validate["retry"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("报价"),
        "被拒时必须给出真实可行的恢复路径（重新报价）：{validate}"
    );
    let draft_get = app
        .call(
            Method::GET,
            &format!("/api/v1/items/{}/drafts/{}", inputs.item, draft.id),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(draft_get.status, StatusCode::OK);
    let body = draft_get.json();
    assert_eq!(body["data"]["completeness"], "partial");
    assert_eq!(
        body["data"]["missing"][0]["code"],
        "model_branch_incomplete"
    );
    assert_eq!(cdn.call_count("GET", "/bad-model.glb"), 1);
}

// ===========================================================================
// 5) 草稿契约：ETag / PATCH 428/412/422
// ===========================================================================

#[tokio::test]
async fn draft_patch_requires_if_match_and_rejects_unknown_fields() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (_cdn, model_url) = glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t15-draft-patch",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t15-draft-patch-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::AssembleDraft,
        JobStatus::Succeeded,
        80,
    )
    .await;
    let job = job_row(&pool, &job_id).await;
    let draft = draft_for_snapshot(&pool, &job.snapshot_id)
        .await
        .expect("草稿");
    let draft_url = format!("/api/v1/items/{}/drafts/{}", inputs.item, draft.id);

    // 缺 If-Match → 428；非法 If-Match → 422。
    let missing = app
        .call(Method::PATCH, &draft_url)
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({ "status": "ready" }))
        .send()
        .await;
    missing.assert_contract_error(StatusCode::PRECONDITION_REQUIRED, "PRECONDITION_REQUIRED");
    let garbage = app
        .call(Method::PATCH, &draft_url)
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", "*")
        .json(&json!({ "status": "ready" }))
        .send()
        .await;
    garbage.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 未知字段（T19 的字段级校验范围）→ 422，不提供绕过入口。
    let unknown = app
        .call(Method::PATCH, &draft_url)
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", "\"r1\"")
        .json(&json!({ "knowledgeJson": { "parts": [] } }))
        .send()
        .await;
    assert_eq!(
        unknown.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        unknown.text()
    );

    // 正常更新：needs_review → ready（人工标记复核完成，仍不发布）。
    let updated = app
        .call(Method::PATCH, &draft_url)
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", "\"r1\"")
        .json(&json!({ "status": "ready" }))
        .send()
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.header("etag").as_deref(), Some("\"r2\""));
    assert_eq!(updated.json()["data"]["status"], "ready");
    assert_eq!(releases_count(&pool).await, 0, "改草稿不产生 release");

    // 过期 revision → 412（details.currentRevision 可见）。
    let stale = app
        .call(Method::PATCH, &draft_url)
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", "\"r1\"")
        .json(&json!({ "status": "needs_review" }))
        .send()
        .await;
    stale.assert_contract_error(StatusCode::PRECONDITION_FAILED, "REVISION_CONFLICT");
    assert_eq!(stale.json()["error"]["details"]["currentRevision"], 2);

    // 同状态幂等（正确 revision → 200，不递增）。
    let same = app
        .call(Method::PATCH, &draft_url)
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", "\"r2\"")
        .json(&json!({ "status": "ready" }))
        .send()
        .await;
    assert_eq!(same.status, StatusCode::OK, "{}", same.text());
    assert_eq!(same.json()["data"]["revision"], 2);

    // 跨物品读取 → 404（不泄露存在性）。
    let other = app
        .call(
            Method::GET,
            &format!(
                "/api/v1/items/01993000-0000-7000-8000-000000000001/drafts/{}",
                draft.id
            ),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
    assert_eq!(audit_count(&pool, "draft_status_changed").await, 1);
}

// ===========================================================================
// 6) unknown：retry 被拒；reconcile 三动作（同步链路）
// ===========================================================================

#[tokio::test]
async fn submission_unknown_rejects_retry_and_reconcile_actions_follow_the_contract() {
    // 说明书 AI 同步批次：第一次 5xx（结果未知），第二次成功（替代提交后使用）。
    let manual = manual_ai_server(vec![
        respond_json_status(500, json!({ "error": "internal" })),
        respond_file(&responses_path("success.json")),
    ]);
    let (_cdn, model_url) = glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t15-unknown-manual",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t15-unknown-manual-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualExtract,
        JobStatus::SubmissionUnknown,
        20,
    )
    .await;
    let batch = stage_of(&pool, &job_id, StageKind::ManualExtract).await;
    let job = job_row(&pool, &job_id).await;
    assert_eq!(
        job.status,
        JobStatus::SubmissionUnknown,
        "父 job 必须展示 unknown（需要管理员对账）"
    );
    let calls_before = manual.call_count("POST", MANUAL_AI_PATH);

    // ① unknown 不是重试入口：被拒且无副作用。
    let etag = job_etag(&app, &cookie, &job_id).await;
    let rejected = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .header("idempotency-key", "t15-unknown-retry")
        .json(&json!({ "stageId": batch.id }))
        .send()
        .await;
    assert_eq!(
        rejected.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        rejected.text()
    );
    assert_eq!(
        rejected.json()["error"]["details"]["reason"],
        "stageNotRetryable"
    );
    assert_eq!(
        stage_by_id(&pool, &batch.id).await.status,
        JobStatus::SubmissionUnknown,
        "被拒不得改变阶段状态"
    );
    assert_eq!(
        manual.call_count("POST", MANUAL_AI_PATH),
        calls_before,
        "被拒不得产生任何新请求"
    );

    // ② 未登录（单管理员系统：不存在第二角色）→ 401。
    let unauthenticated = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .header("if-match", &etag)
        .json(&json!({
            "action": "recordNoTask",
            "stageId": batch.id,
            "evidence": "对账面板 2026-09-12 查询：无任务",
        }))
        .send()
        .await;
    unauthenticated.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");

    // ③ attachRemoteTask 对同步链路不可用（不假定 response_id 可轮询）。
    let attach_unsupported = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "attachRemoteTask",
            "stageId": batch.id,
            "remoteTaskId": "some-task",
            "acknowledgeMatches": true,
        }))
        .send()
        .await;
    assert_eq!(
        attach_unsupported.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        attach_unsupported.text()
    );
    assert_eq!(
        attach_unsupported.json()["error"]["details"]["reason"],
        "attachRemoteTaskUnsupported"
    );

    // ④ recordNoTask 必须填核查证据。
    let no_evidence = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({ "action": "recordNoTask", "stageId": batch.id }))
        .send()
        .await;
    assert_eq!(no_evidence.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(no_evidence.json()["error"]["details"]["fields"].is_array());

    // ⑤ authorizeReplacement 需要预算确认与重复收费确认。
    let no_ack = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "authorizeReplacement",
            "stageId": batch.id,
            "limits": { "manualAiUsdMicros": 100_000 },
        }))
        .send()
        .await;
    assert_eq!(no_ack.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(no_ack.json()["error"]["details"]["fields"].is_array());
    let low_budget = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "authorizeReplacement",
            "stageId": batch.id,
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
        low_budget.json()["error"]["details"]["reason"],
        "budgetBelowPlannedUpperBound"
    );

    // ⑥ recordNoTask 正例：attempt → failed、阶段 → needs_input、审计保留、预留不释放。
    let snapshot_id = job.snapshot_id.clone();
    let reserved_before = ledger_entry(&pool, &snapshot_id, ProviderKey::ManualAi)
        .await
        .expect("预留");
    assert_eq!(reserved_before.state, LedgerState::Unknown);
    let recorded = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "recordNoTask",
            "stageId": batch.id,
            "evidence": "2026-09-12 12:00 在供应商控制台任务列表按时间范围查询：无对应任务；通话记录 #synthetic",
        }))
        .send()
        .await;
    assert_eq!(recorded.status, StatusCode::OK, "{}", recorded.text());
    let recorded_body = recorded.json();
    assert_eq!(recorded_body["data"]["action"], "recordNoTask");
    assert_eq!(recorded_body["data"]["stageStatus"], "needs_input");
    assert!(
        recorded_body["data"]["notice"]
            .as_str()
            .unwrap_or_default()
            .contains("不是供应商证明"),
        "文案必须说明这是管理员声明"
    );
    assert_eq!(
        stage_by_id(&pool, &batch.id).await.status,
        JobStatus::NeedsInput
    );
    let entry_after = ledger_entry(&pool, &snapshot_id, ProviderKey::ManualAi)
        .await
        .expect("预留");
    assert_eq!(
        entry_after.state,
        LedgerState::Unknown,
        "unknown 预留不自动释放"
    );
    assert_eq!(entry_after.actual, None, "不得把实际费用填 0");
    assert_eq!(audit_count(&pool, "job_reconcile_record_no_task").await, 1);

    // ⑦ 记录证据后：该阶段可显式重试（新一次付费提交）；重放不产生第二个 attempt。
    let etag = job_etag(&app, &cookie, &job_id).await;
    let retry = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .header("idempotency-key", "t15-after-record-no-task")
        .json(&json!({ "stageId": batch.id }))
        .send()
        .await;
    assert_eq!(retry.status, StatusCode::OK, "{}", retry.text());
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualExtract,
        JobStatus::Succeeded,
        10,
    )
    .await;
    {
        let mut conn = pool.acquire().await.unwrap();
        let attempts = repo::attempts::list_for_job(&mut conn, &job_id)
            .await
            .expect("attempt 列表")
            .into_iter()
            .filter(|attempt| attempt.stage_id == batch.id)
            .count();
        assert_eq!(
            attempts, 2,
            "替代/重试必须创建新 attempt（旧 attempt 保留）"
        );
    }
    manual.assert_called_times("POST", MANUAL_AI_PATH, 2);
    // 同一预留继续占用预算（不因对账自动释放/结算）。
    let entry_final = ledger_entry(&pool, &snapshot_id, ProviderKey::ManualAi)
        .await
        .expect("预留");
    assert_eq!(entry_final.state, LedgerState::Unknown);
}

// ===========================================================================
// 7) attachRemoteTask（仅 Tripo）：查询验证 + 无重复购买地恢复
// ===========================================================================

#[tokio::test]
async fn attach_remote_task_resumes_without_repurchase() {
    // 付费提交返回 5xx（结果未知，不能证明未被接受）。
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = glb_cdn();
    let tripo = FixtureServer::start(scenario(vec![
        exact_route("POST", TRIPO_UPLOAD_PATH, upload_steps()),
        exact_route(
            "POST",
            TRIPO_SUBMIT_PATH,
            vec![respond_json_status(
                500,
                json!({ "code": 500, "message": "boom" }),
            )],
        ),
        exact_route_repeat(
            "GET",
            &format!("{TRIPO_TASKS_PREFIX}found-in-account-0042"),
            vec![task_running(), task_success_with(&model_url)],
        ),
        prefix_route(
            "GET",
            TRIPO_TASKS_PREFIX,
            true,
            vec![respond_json_status(
                404,
                json!({ "code": 404, "message": "task not found" }),
            )],
        ),
    ]));
    let app = pipeline_app(
        "t15-attach",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t15-attach-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoSubmit,
        JobStatus::SubmissionUnknown,
        20,
    )
    .await;
    let submit = stage_of(&pool, &job_id, StageKind::TripoSubmit).await;
    assert_eq!(
        job_row(&pool, &job_id).await.status,
        JobStatus::SubmissionUnknown
    );
    tripo.assert_called_times("POST", TRIPO_SUBMIT_PATH, 1);

    // 负例 1：缺二次确认 → 422 字段级。
    let etag = job_etag(&app, &cookie, &job_id).await;
    let no_ack = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "attachRemoteTask",
            "stageId": submit.id,
            "remoteTaskId": "found-in-account-0042",
        }))
        .send()
        .await;
    assert_eq!(no_ack.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(no_ack.json()["error"]["details"]["fields"].is_array());

    // 负例 2：供应商查询验证失败（账户里查不到）→ 422，无副作用、不伪造"不存在"证明。
    let not_found = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "attachRemoteTask",
            "stageId": submit.id,
            "remoteTaskId": "typo-not-in-account",
            "acknowledgeMatches": true,
        }))
        .send()
        .await;
    assert_eq!(
        not_found.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        not_found.text()
    );
    assert_eq!(
        not_found.json()["error"]["details"]["reason"],
        "remoteTaskVerificationFailed"
    );
    assert_eq!(
        stage_by_id(&pool, &submit.id).await.status,
        JobStatus::SubmissionUnknown,
        "验证失败不得改变阶段状态（无副作用）"
    );

    // 正例：附加账户中查到的任务 → 阶段恢复排队 → 按已知 ID 继续（不重新购买）。
    let attached = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/reconcile"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({
            "action": "attachRemoteTask",
            "stageId": submit.id,
            "remoteTaskId": "found-in-account-0042",
            "acknowledgeMatches": true,
        }))
        .send()
        .await;
    assert_eq!(attached.status, StatusCode::OK, "{}", attached.text());
    let attached_body = attached.json();
    assert_eq!(attached_body["data"]["stageStatus"], "queued");
    assert_eq!(
        audit_count(&pool, "job_reconcile_attach_remote_task").await,
        1
    );
    {
        let mut conn = pool.acquire().await.unwrap();
        let attempt = repo::attempts::latest_for_stage(&mut conn, &submit.id)
            .await
            .expect("attempt")
            .expect("attempt 必须存在");
        assert_eq!(attempt.submit_state, SubmitState::Accepted);
        assert_eq!(
            attempt.remote_task_id.as_deref(),
            Some("found-in-account-0042")
        );
    }

    // 阶段恢复执行：查询成功 → 下载 → 校验 → 组装；付费提交仍然只有一次。
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::AssembleDraft,
        JobStatus::Succeeded,
        80,
    )
    .await;
    tripo.assert_called_times("POST", TRIPO_SUBMIT_PATH, 1);
    assert_eq!(
        tripo.call_count("GET", &format!("{TRIPO_TASKS_PREFIX}found-in-account-0042")),
        2,
        "附加后按已知任务继续查询（首次 running、随后 success）"
    );
    assert_eq!(cdn.call_count("GET", "/model.glb"), 1);
    let job = job_row(&pool, &job_id).await;
    assert_eq!(job.status, JobStatus::Succeeded);
    assert!(draft_for_snapshot(&pool, &job.snapshot_id).await.is_some());
    tripo.assert_no_script_problems();
}

// ===========================================================================
// 8) 取消：未提交阶段停止；已提交阶段保留查询与账务；取消后无新付费步骤
// ===========================================================================

#[tokio::test]
async fn cancel_stops_unsubmitted_stages_and_keeps_submitted_records() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, model_url) = glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_running(), task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t15-cancel",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t15-cancel-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));

    // 推进到"付费提交已完成、仍在等待远端"：这是"已提交阶段"。
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoPoll,
        JobStatus::WaitingProvider,
        40,
    )
    .await;
    let poll = stage_of(&pool, &job_id, StageKind::TripoPoll).await;
    let job = job_row(&pool, &job_id).await;
    let calls_before = tripo.request_total();
    let downloads_before = cdn.request_total();

    // 取消：If-Match 必填。
    let no_match = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/cancel"))
        .cookie(&cookie)
        .csrf(&csrf)
        .send()
        .await;
    no_match.assert_contract_error(StatusCode::PRECONDITION_REQUIRED, "PRECONDITION_REQUIRED");

    let etag = job_etag(&app, &cookie, &job_id).await;
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
    let notice = body["data"]["notice"].as_str().unwrap_or_default();
    assert!(
        notice.contains("不会被撤销") || notice.contains("不保证供应商撤单"),
        "取消文案不得声称已取消远端付费操作：{notice}"
    );
    let preserved = body["data"]["preservedStages"].as_array().unwrap();
    assert!(
        preserved
            .iter()
            .any(|stage| stage["stageKind"] == "tripo_poll"),
        "已提交阶段必须保留：{preserved:?}"
    );
    assert_eq!(audit_count(&pool, "job_cancelled").await, 1);

    // 已提交阶段的状态、attempt 与账务保留可查。
    assert_eq!(
        stage_by_id(&pool, &poll.id).await.status,
        JobStatus::WaitingProvider,
        "已提交阶段保持原状态（本地取消不撤销远端）"
    );
    assert_eq!(
        stage_status(&pool, &job_id, StageKind::ModelDownload).await,
        JobStatus::Cancelled,
        "未提交阶段必须转 cancelled（不再领取）"
    );
    let snapshot_id = job.snapshot_id.clone();
    let entry = ledger_entry(&pool, &snapshot_id, ProviderKey::Tripo)
        .await
        .expect("Tripo 预留");
    assert!(
        matches!(entry.state, LedgerState::Reserved | LedgerState::Settled),
        "账务记录保留（不因取消静默释放）：{:?}",
        entry.state
    );
    let detail = get_job_json(&app, &cookie, &job_id).await;
    let attempts = detail["data"]["attempts"].as_array().unwrap();
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt["remoteTaskId"] == TASK_ID),
        "远端 task ID 保留可查（对账/账务需要）"
    );

    // 取消后不再产生任何新外呼（含新的付费步骤）。
    let reports = run_ticks(&executor, &clock, 20, 20_000).await;
    assert!(
        reports.is_empty(),
        "取消后执行器不得再领取该 job 的任何阶段：{reports:?}"
    );
    assert_eq!(
        tripo.request_total(),
        calls_before,
        "取消后不得新增 Tripo 请求"
    );
    assert_eq!(
        cdn.request_total(),
        downloads_before,
        "取消后不得新增下载请求"
    );
    tripo.assert_called_times("POST", TRIPO_SUBMIT_PATH, 1);

    // 已终态：再次取消 → 422（cancelNotNeeded）；旧 ETag → 412。
    let new_etag = job_etag(&app, &cookie, &job_id).await;
    let again = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/cancel"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &new_etag)
        .send()
        .await;
    assert_eq!(
        again.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        again.text()
    );
    assert_eq!(
        again.json()["error"]["details"]["reason"],
        "cancelNotNeeded"
    );

    // 取消后 retry 被拒（不新增付费步骤）。
    let retry = app
        .call(Method::POST, &format!("/api/v1/jobs/{job_id}/retry"))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &new_etag)
        .header("idempotency-key", "t15-cancel-retry")
        .json(&json!({ "stageId": poll.id }))
        .send()
        .await;
    assert_eq!(
        retry.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        retry.text()
    );
    assert_eq!(retry.json()["error"]["details"]["reason"], "jobCancelled");
}

// ===========================================================================
// 9) 任务列表（GET /jobs）：游标分页、itemId 过滤、严格参数
// ===========================================================================

#[tokio::test]
async fn job_list_supports_cursor_pagination_and_item_filter() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (_cdn, model_url) = glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t15-job-list",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_one = create_job(&app, &cookie, &csrf, &inputs, "t15-job-list-key-1").await;
    let job_two = create_job(&app, &cookie, &csrf, &inputs, "t15-job-list-key-2").await;

    // 默认列表：两行（新→旧），含物品/阶段摘要/费用预留/草稿 id 字段。
    let list = app
        .call(Method::GET, "/api/v1/jobs")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    let body = list.json();
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], job_two, "稳定排序：新任务在前");
    assert_eq!(rows[0]["itemName"], "组装测试物品");
    assert_eq!(rows[0]["itemModel"], "X100V");
    assert_eq!(rows[0]["status"], "queued");
    assert_eq!(
        rows[0]["stageSummary"]["total"], 9,
        "阶段计数（不是百分比）"
    );
    assert_eq!(
        rows[0]["reservations"].as_array().unwrap().len(),
        2,
        "分列预留"
    );
    assert!(rows[0]["draftId"].is_null());

    // limit + 游标：只返回其后一行。
    let first_page = app
        .call(Method::GET, "/api/v1/jobs?limit=1")
        .cookie(&cookie)
        .send()
        .await;
    let first_body = first_page.json();
    assert_eq!(first_body["data"].as_array().unwrap().len(), 1);
    let cursor = first_body["nextCursor"]
        .as_str()
        .expect("还有下一页")
        .to_owned();
    let second_page = app
        .call(
            Method::GET,
            &format!("/api/v1/jobs?limit=1&cursor={cursor}"),
        )
        .cookie(&cookie)
        .send()
        .await;
    let second_body = second_page.json();
    assert_eq!(second_body["data"].as_array().unwrap().len(), 1);
    assert_eq!(second_body["data"][0]["id"], job_one);
    assert!(second_body["nextCursor"].is_null(), "最后一页无游标");

    // itemId 过滤：命中与不命中；游标作用域与过滤条件绑定。
    let filtered = app
        .call(Method::GET, &format!("/api/v1/jobs?itemId={}", inputs.item))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(filtered.json()["data"].as_array().unwrap().len(), 2);
    let other = app
        .call(
            Method::GET,
            "/api/v1/jobs?itemId=01993000-0000-7000-8000-000000000009",
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(other.json()["data"].as_array().unwrap().len(), 0);
    let scope_mismatch = app
        .call(
            Method::GET,
            &format!("/api/v1/jobs?itemId={}&cursor={cursor}", inputs.item),
        )
        .cookie(&cookie)
        .send()
        .await;
    scope_mismatch.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 严格查询参数：未知参数/非法 limit → 422 字段级。
    for query in ["?frobnicate=1", "?limit=0", "?limit=1000"] {
        let rejected = app
            .call(Method::GET, &format!("/api/v1/jobs{query}"))
            .cookie(&cookie)
            .send()
            .await;
        assert_eq!(
            rejected.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "查询 {query} 必须被拒：{}",
            rejected.text()
        );
        assert!(rejected.json()["error"]["details"]["fields"].is_array());
    }

    // 任务详情可读（未登录 401；跨任务阶段不属于该任务 → 404）。
    let unauthenticated = app.call(Method::GET, "/api/v1/jobs").send().await;
    unauthenticated.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
    let detail = get_job_json(&app, &cookie, &job_one).await;
    assert_eq!(detail["data"]["item"]["id"], inputs.item);
    assert_eq!(detail["data"]["stages"].as_array().unwrap().len(), 9);
    assert_eq!(detail["data"]["attempts"].as_array().unwrap().len(), 0);
    let missing = app
        .call(
            Method::GET,
            "/api/v1/jobs/01993000-0000-7000-8000-00000000000a",
        )
        .cookie(&cookie)
        .send()
        .await;
    missing.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
}

// ===========================================================================
// 9) T14 P3-1：拒答批次的恢复路径不得补推进为"产出了知识"
// ===========================================================================

#[tokio::test]
async fn refused_batch_recovery_never_reports_success() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("refusal.json"))]);
    let (_cdn, model_url) = glb_cdn();
    let tripo = tripo_server(
        upload_steps(),
        vec![submit_success()],
        vec![task_success_with(&model_url)],
        true,
    );
    let app = pipeline_app(
        "t15-p31",
        &tripo_base_url(&tripo),
        &manual_ai_base_url(&manual),
    )
    .await;
    let (cookie, csrf) = logged_in(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf).await;
    let job_id = create_job(&app, &cookie, &csrf, &inputs, "t15-p31-key").await;
    let pool = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = pipeline_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ManualExtract,
        JobStatus::NeedsInput,
        20,
    )
    .await;
    let batch = stage_of(&pool, &job_id, StageKind::ManualExtract).await;
    assert!(
        batch.result_asset_id.is_some(),
        "拒答批次的诊断结果资产已落库（T14 行为）"
    );

    // 崩溃现场：结果已落库、checkpoint 未推进（T14 P3-1 的形态）→ 恢复扫描。
    crash_stage_before_checkpoint(&pool, &batch.id).await;
    let report = recover_with_empty_registry(&app, Arc::clone(&clock)).await;
    assert_eq!(report.needs_input, 1, "{report:?}");
    assert_eq!(report.succeeded, 0, "拒答批次不得被补推进为 succeeded");
    let recovered = stage_by_id(&pool, &batch.id).await;
    assert_eq!(recovered.status, JobStatus::NeedsInput);
    let items = recovered.needs_input_json.clone().unwrap();
    assert_eq!(items[0]["code"], "manual_ai_refusal");

    // 展示口径同判据：批次知识产出事实为 false；下游 merge 不解锁。
    tick_until_stage(
        &pool,
        &executor,
        &clock,
        &job_id,
        StageKind::ModelValidate,
        JobStatus::Succeeded,
        40,
    )
    .await;
    assert_eq!(
        stage_status(&pool, &job_id, StageKind::ManualMerge).await,
        JobStatus::Queued,
        "未产出知识的批次 → merge 不解锁（草稿不会出现知识空洞）"
    );
    let detail = get_job_json(&app, &cookie, &job_id).await;
    let batch_json = detail["data"]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["stageKind"] == "manual_extract")
        .expect("批次阶段");
    assert_eq!(batch_json["knowledgeProduced"], false);
    assert_eq!(batch_json["status"], "needs_input");
    // 模型分支仍可独立完成；知识分支阻塞 → 无草稿（T10 语义：上游阻塞不组装）。
    assert_eq!(detail["data"]["status"], "needs_input");
    assert!(
        draft_for_snapshot(&pool, &job_row(&pool, &job_id).await.snapshot_id)
            .await
            .is_none(),
        "批次级阻塞时 merge 未定性：不产出草稿"
    );
    manual.assert_called_times("POST", MANUAL_AI_PATH, 1);
}
