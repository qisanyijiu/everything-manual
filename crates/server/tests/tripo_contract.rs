//! T12 集成测试：**Tripo v3 HTTP 适配器**（PRD 修订 2 / ui_revision 2；REQ-027 主，AC-041）。
//!
//! 覆盖（命令 ↔ AC 见 implementation.md §T12）：
//! - **上传**：`POST /files`、multipart 字段 `file`、`Authorization: Bearer`、token 原样保存、
//!   内容哈希缓存（同一内容不重复上传）；
//! - **生成**：`POST /generation/multiview-to-model` 的 body 逐字段（`inputs` 的 view-key 形态、
//!   `model/texture/pbr/texture_quality/geometry_quality/face_limit/quad/generate_parts`）、
//!   缺失方向不提交、**没有 v2 字段形态**（`model_version`/`files`/`type`）；
//! - **查询**：`GET /tasks/{task_id}`、原始状态与归一化状态都保存（含 `banned`/`expired` 与
//!   **未知枚举保留原值**）、`success` 缺模型不算成功；
//! - **错误**：HTTP 200 但 `code != 0` → 业务失败并读取 `message`/`suggestion`（脱敏）；
//!   429（尊重 `Retry-After`）/ 5xx → 可退避重试；**付费 POST 被接受后断连 → 只发一次**
//!   （fixture 计数断言 + 执行器不重发）；
//! - **计费**：供应商十进制 credits 精确解析为 `creditMinor`，原始字面量与来源字段保留；
//! - **接线**：`register_provider_handlers` 在 Provider 未配置时不注册任何处理器，
//!   已配置时注册 upload/submit/poll 三个阶段。
//!
//! 隔离与门控：全部 HTTP 指向 T05 的本机 fixture（`127.0.0.1`，随机端口），
//! **零真实外网调用**（fixture 只绑定回环 + 用例计数断言；测试使用假凭据 canary）。
//! 真实收费调用只在 T23 的授权入口（AC-042）执行，本文件不产生任何付费请求。

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
    ExecutorConfig, JobExecutor, ManualClock, StageRegistry, TickOutcome,
};
use everything_manual::providers::tripo::{
    TRIPO_SUBMIT_PATH, TRIPO_TASKS_PATH_PREFIX, TRIPO_UPLOAD_PATH, TripoClient, TripoError,
    TripoTimeouts, status::NormalizedStatus,
};
use everything_manual::storage::repo::{job_stages as stages_repo, ledger as ledger_repo};
use manual_core::domain::{JobStage, JobStatus, ProviderKey, StageKind};
use manual_core::generation::sha256_hex;
use manual_core::timestamps::Timestamp;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use test_support::FixtureServer;
use test_support::scenario::{BodySpec, PathMatchSpec, ResponseSpec, RouteScript, Scenario, Step};

const PASSWORD: &str = "test-password-t12-7a02";
/// 测试用假凭据（canary）：断言不得出现在日志/记录/Debug 输出里。
const CANARY_KEY: &str = "canary-t12-not-a-real-key";
const TRIPO_MODEL: &str = "v3.1-20260211";
const MANUAL_AI_MODEL: &str = "gpt-5-mini";
const PRESET: &str = "tripo-h-v3.1-standard";

/// 测试价格目录（与 T11 用例同价：Tripo 30 credits）。
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

/// 响应体文件根（`tests/fixtures/`）。
const SUBMIT_SUCCESS: &str = "responses/tripo/submit_success.json";
const TASK_RUNNING: &str = "responses/tripo/task_running.json";
const TASK_SUCCESS: &str = "responses/tripo/task_success.json";
const TASK_SUCCESS_MISSING_MODEL: &str = "responses/tripo/task_success_missing_model.json";
const TASK_BANNED: &str = "responses/tripo/task_banned.json";
const TASK_EXPIRED: &str = "responses/tripo/task_expired.json";
const TASK_UNKNOWN_STATUS: &str = "responses/tripo/task_unknown_status.json";
const BUSINESS_ERROR_200: &str = "responses/tripo/business_error_200.json";
const UNAUTHORIZED_401: &str = "responses/tripo/unauthorized_401.json";
const SERVER_ERROR_503: &str = "responses/tripo/server_error_503.json";

/// fixture 场景使用的三个路径（与生产端点常量一致；断言记录里没有别的目标）。
fn api_paths() -> [String; 3] {
    let (upload, submit, tasks) = everything_manual::providers::tripo::stage_endpoint_summary();
    assert_eq!(
        tasks,
        everything_manual::providers::tripo::client::TRIPO_TASKS_PATH_PREFIX,
        "tasks 前缀常量必须一致"
    );
    [
        format!("/v3{upload}"),
        format!("/v3{submit}"),
        format!("/v3{tasks}"),
    ]
}

// ---------------------------------------------------------------------------
// fixture 工具
// ---------------------------------------------------------------------------

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("读取样例资产 {} 失败：{error}", path.display()))
}

fn respond_file(path: &str) -> Step {
    respond_file_with_status(200, path)
}

fn respond_file_with_status(status: u16, path: &str) -> Step {
    Step::Respond {
        response: ResponseSpec {
            status,
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

fn scenario(routes: Vec<RouteScript>) -> Scenario {
    Scenario::new(routes)
}

/// fixture 上的 v3 根（与生产 `providers.tripo.base_url` 同形态：带版本段）。
fn tripo_base_url(server: &FixtureServer) -> String {
    format!("{}/v3", server.base_url())
}

fn tripo_client(server: &FixtureServer) -> TripoClient {
    TripoClient::new(
        &tripo_base_url(server),
        SecretString::new(CANARY_KEY),
        TripoTimeouts::default(),
    )
    .expect("构造 Tripo 客户端")
}

// ---------------------------------------------------------------------------
// 客户端层：字节级协议（endpoint / 头 / body）
// ---------------------------------------------------------------------------

/// 上传：`POST /files`、multipart 字段 `file`、`Authorization: Bearer`、
/// token **原样保存**（不 trim／不截断／不当 UUID 校验）。
#[tokio::test]
async fn upload_sends_multipart_with_file_field_and_keeps_token_verbatim() {
    let server = FixtureServer::start(scenario(vec![exact_route(
        "POST",
        "/v3/files",
        vec![respond_json(json!({
            "code": 0,
            "data": { "file_token": " TOKEN-with Spaces_01 " }
        }))],
    )]));
    let client = tripo_client(&server);
    let image = fixture_bytes("sample-photo-front.jpg");
    let expected_hash = sha256_hex(&image);

    let data = client
        .upload_image("front.jpg", "image/jpeg", image.clone())
        .await
        .expect("上传成功");
    assert_eq!(
        data.token, " TOKEN-with Spaces_01 ",
        "token 必须按 opaque string 原样保存（不 trim、不截断、不校验 UUID）"
    );
    assert_eq!(data.field, "file_token");

    server.assert_called_once("POST", "/v3/files");
    let recorded = &server.requests_matching("POST", "/v3/files")[0];
    assert_eq!(
        recorded.header_value("authorization"),
        Some("Bearer [REDACTED]"),
        "必须携带 Bearer 凭据且记录已脱敏（保留 scheme）"
    );
    let content_type = recorded.header_value("content-type").expect("content-type");
    assert!(
        content_type.starts_with("multipart/form-data; boundary="),
        "{content_type}"
    );
    let body = recorded.body_text();
    assert!(
        body.contains("Content-Disposition: form-data; name=\"file\"; filename=\"front.jpg\""),
        "multipart 字段名必须是 file：{body}"
    );
    assert!(body.contains("Content-Type: image/jpeg"), "{body}");
    assert!(
        recorded
            .body
            .windows(image.len())
            .any(|window| window == image.as_slice()),
        "图片字节必须原样进入 multipart（源图哈希 {}）",
        expected_hash
    );
    assert!(
        !format!("{recorded:?}").contains(CANARY_KEY),
        "记录（含 Debug）不得包含密钥明文"
    );
    server.assert_no_script_problems();
}

/// 生成：endpoint、body 逐字段（含 `inputs` 的 view-key 形态与**没有 v2 字段形态**）。
#[tokio::test]
async fn submit_body_matches_frozen_parameters_without_v2_fields() {
    let server = FixtureServer::start(scenario(vec![exact_route(
        "POST",
        "/v3/generation/multiview-to-model",
        vec![respond_file(SUBMIT_SUCCESS)],
    )]));
    let client = tripo_client(&server);
    let parameters = everything_manual::providers::tripo::SubmitParameters {
        model: TRIPO_MODEL.to_owned(),
        texture: true,
        pbr: true,
        texture_quality: "standard".to_owned(),
        geometry_quality: "standard".to_owned(),
        face_limit: 100_000,
        quad: false,
        generate_parts: false,
    };
    // 缺失方向直接不提交对应对象（这里只有 front + left；back/right 不存在）。
    let request = everything_manual::providers::tripo::SubmitRequest::new(
        &parameters,
        &[
            everything_manual::providers::tripo::ViewInput {
                view: "front".to_owned(),
                token: "token-front".to_owned(),
            },
            everything_manual::providers::tripo::ViewInput {
                view: "left".to_owned(),
                token: "token-left".to_owned(),
            },
        ],
    );
    let body = request.to_bytes();
    let data = client.submit_multiview(&body).await.expect("提交成功");
    assert_eq!(
        data.task_id, "fixture-task-0001",
        "task_id 按 opaque string 保存"
    );

    server.assert_called_once("POST", "/v3/generation/multiview-to-model");
    let recorded = &server.requests_matching("POST", "/v3/generation/multiview-to-model")[0];
    assert_eq!(recorded.method, "POST");
    assert_eq!(
        recorded.header_value("content-type"),
        Some("application/json")
    );
    assert_eq!(
        recorded.header_value("authorization"),
        Some("Bearer [REDACTED]")
    );
    let sent = recorded.json_body();
    assert_eq!(sent["model"], TRIPO_MODEL);
    assert_eq!(sent["texture"], true);
    assert_eq!(sent["pbr"], true);
    assert_eq!(sent["texture_quality"], "standard");
    assert_eq!(sent["geometry_quality"], "standard");
    assert_eq!(sent["face_limit"], 100_000);
    assert_eq!(sent["quad"], false);
    assert_eq!(sent["generate_parts"], false);
    let inputs = sent["inputs"].as_array().expect("inputs 是数组");
    assert_eq!(inputs.len(), 2, "缺失方向不提交对应对象：{sent}");
    assert_eq!(inputs[0]["front"], "token-front", "view-key 形态");
    assert_eq!(inputs[1]["left"], "token-left");
    // 发送的字节与落库 request_hash 同源（无一字差异）。
    assert_eq!(recorded.body, body, "发出去的就是用于 hash 的同一份字节");
    server.assert_no_script_problems();
}

/// HTTP 200 但 `code != 0`：业务失败，读取 `message`/`suggestion`（脱敏），
/// 归入"可证明未被接受"（付费提交不重发）。
#[tokio::test]
async fn http_200_with_nonzero_code_is_business_failure() {
    let server = FixtureServer::start(scenario(vec![exact_route(
        "POST",
        "/v3/generation/multiview-to-model",
        vec![respond_file(BUSINESS_ERROR_200)],
    )]));
    let client = tripo_client(&server);
    let error = client
        .submit_multiview(br#"{"inputs":[{"front":"t"}]}"#)
        .await
        .expect_err("HTTP 200 + code!=0 必须失败");
    match &error {
        TripoError::Business {
            http_status,
            code,
            message,
            suggestion,
        } => {
            assert_eq!(*http_status, 200);
            assert_eq!(*code, Some(1201));
            assert_eq!(message.as_deref(), Some("invalid image token"));
            assert!(suggestion.as_deref().unwrap().contains("重新上传"));
        }
        other => panic!("应为业务失败，实际 {other:?}"),
    }
    assert!(error.is_definitively_refused(), "业务错误可证明未被接受");
    let summary = error.redacted();
    assert!(summary.contains("1201") && summary.contains("invalid image token"));
    server.assert_no_script_problems();
}

/// 4xx / 5xx / 429 的分类：429 带 `Retry-After`、5xx 是"不能证明未被接受"、
/// 其余 4xx 是明确拒绝。
#[tokio::test]
async fn http_statuses_are_classified_for_retry_policy() {
    let server = FixtureServer::start(scenario(vec![exact_route(
        "POST",
        "/v3/files",
        vec![
            respond_file_with_status(401, UNAUTHORIZED_401),
            Step::Respond {
                response: ResponseSpec {
                    status: 429,
                    headers: BTreeMap::from([("retry-after".to_owned(), "7".to_owned())]),
                    body: BodySpec::Json {
                        json: json!({ "code": 4290, "message": "too many requests" }),
                    },
                },
            },
            respond_file_with_status(503, SERVER_ERROR_503),
        ],
    )]));
    let client = tripo_client(&server);
    let bytes = b"\xFF\xD8\xFF-jpeg".to_vec();

    let unauthorized = client
        .upload_image("front.jpg", "image/jpeg", bytes.clone())
        .await
        .expect_err("401");
    assert!(matches!(
        unauthorized,
        TripoError::Business {
            http_status: 401,
            ..
        }
    ));
    assert!(unauthorized.is_definitively_refused());
    assert!(
        unauthorized
            .business_summary()
            .unwrap()
            .contains("invalid api key")
    );

    let limited = client
        .upload_image("front.jpg", "image/jpeg", bytes.clone())
        .await
        .expect_err("429");
    assert_eq!(
        limited.retry_after_seconds(),
        Some(7),
        "必须尊重 Retry-After"
    );
    assert!(limited.is_definitively_refused(), "429 = 可证明未被接受");

    let server_error = client
        .upload_image("front.jpg", "image/jpeg", bytes)
        .await
        .expect_err("503");
    assert!(matches!(
        server_error,
        TripoError::ServerError { status: 503 }
    ));
    assert!(
        !server_error.is_definitively_refused(),
        "含糊 5xx 不能证明请求未被接受"
    );
    server.assert_called_times("POST", "/v3/files", 3);
    server.assert_no_script_problems();
}

/// `code == 0` 但缺 `task_id`：协议不符 → 结果未知（绝不当作成功、绝不重发）。
#[tokio::test]
async fn submit_without_task_id_is_unexpected_and_not_provable_success() {
    let server = FixtureServer::start(scenario(vec![exact_route(
        "POST",
        "/v3/generation/multiview-to-model",
        vec![respond_json(json!({ "code": 0, "data": {} }))],
    )]));
    let client = tripo_client(&server);
    let error = client
        .submit_multiview(br#"{}"#)
        .await
        .expect_err("缺 task_id 必须失败");
    assert!(matches!(error, TripoError::Unexpected { .. }), "{error:?}");
    assert!(
        !error.is_definitively_refused(),
        "拿不到 task ID 时不能证明未被接受 → 结果未知"
    );
    assert!(error.redacted().contains("task_id"));
    server.assert_no_script_problems();
}

/// 查询：endpoint、原始状态保存、归一化映射（`banned`/`expired` 与未知枚举保留原值）、
/// `success` 缺模型不算成功。
#[tokio::test]
async fn task_query_keeps_raw_status_and_requires_model_for_success() {
    let server = FixtureServer::start(scenario(vec![prefix_route(
        "GET",
        "/v3/tasks/",
        false,
        vec![
            respond_file(TASK_RUNNING),
            respond_file(TASK_BANNED),
            respond_file(TASK_EXPIRED),
            respond_file(TASK_UNKNOWN_STATUS),
            respond_file(TASK_SUCCESS),
            respond_file(TASK_SUCCESS_MISSING_MODEL),
        ],
    )]));
    let client = tripo_client(&server);

    let running = client.get_task("fixture-task-0001").await.expect("查询");
    assert_eq!(running.status_raw, "running");
    assert_eq!(
        NormalizedStatus::new(&running.status_raw).state().as_str(),
        "running"
    );

    let banned = client.get_task("fixture-task-0001").await.expect("查询");
    assert_eq!(banned.status_raw, "banned");
    assert_eq!(
        NormalizedStatus::new(&banned.status_raw).state().as_str(),
        "banned"
    );

    let expired = client.get_task("fixture-task-0001").await.expect("查询");
    assert_eq!(expired.status_raw, "expired");
    assert_eq!(
        NormalizedStatus::new(&expired.status_raw).state().as_str(),
        "expired"
    );

    let unknown = client.get_task("fixture-task-0001").await.expect("查询");
    assert_eq!(unknown.status_raw, "queued_for_human_review");
    let normalized = NormalizedStatus::new(&unknown.status_raw);
    assert_eq!(normalized.state().as_str(), "unrecognized");
    assert!(normalized.is_unrecognized(), "未知枚举保留原值、不得猜测");

    let success = client.get_task("fixture-task-0001").await.expect("查询");
    assert_eq!(success.status_raw, "success");
    let model_url = success
        .model_url
        .as_deref()
        .expect("success 必须带可下载模型");
    assert!(model_url.starts_with("https://cdn.example.invalid/"));
    assert_eq!(
        success.billing.as_ref().map(|billing| billing.credit_minor),
        Some(3000),
        "30 credits = 3000 creditMinor"
    );

    let missing = client.get_task("fixture-task-0001").await.expect("查询");
    assert_eq!(missing.status_raw, "success");
    assert!(
        missing.model_url.is_none(),
        "success 缺可下载模型：调用方不得组装成功"
    );

    server.assert_called_times("GET", "/v3/tasks/fixture-task-0001", 6);
    server.assert_no_script_problems();
}

/// task ID 是 opaque string：进路径前按段转义（不截断、不改写）。
#[tokio::test]
async fn task_query_encodes_opaque_task_id_as_single_path_segment() {
    let server = FixtureServer::start(scenario(vec![prefix_route(
        "GET",
        "/v3/tasks/",
        false,
        vec![respond_file(TASK_RUNNING)],
    )]));
    let client = tripo_client(&server);
    let task_id = "task-0001/with?odd#chars";
    client.get_task(task_id).await.expect("查询");
    let recorded = &server.requests_matching("GET", "/v3/tasks/task-0001%2Fwith%3Fodd%23chars");
    assert_eq!(
        recorded.len(),
        1,
        "{}",
        server.recorded_summary().join("；")
    );
    server.assert_no_script_problems();
}

/// 计费：精确 decimal 解析（含多小数位、Ceil），原始字面量与来源字段保留；
/// 无法解析时不猜测金额。
#[tokio::test]
async fn credits_are_parsed_exactly_and_raw_literal_is_kept() {
    let server = FixtureServer::start(scenario(vec![prefix_route(
        "GET",
        "/v3/tasks/",
        true,
        vec![respond_json(json!({
            "code": 0,
            "data": {
                "task_id": "t-1",
                "status": "success",
                "credits_consumed": "30.5",
                "output": { "model_url": "https://cdn.example.invalid/m.glb?sig=x" }
            }
        }))],
    )]));
    let client = tripo_client(&server);
    let task = client.get_task("t-1").await.expect("查询");
    let billing = task.billing.as_ref().expect("计费字段存在");
    assert_eq!(billing.literal, "30.5", "原始字面量原样保留");
    assert_eq!(
        billing.credit_minor, 3050,
        "30.5 credits = 3050 creditMinor"
    );
    assert_eq!(billing.source_field, "credits_consumed");
    assert!(task.billing_problem.is_none());

    // 非法字面量：不猜测金额（记录诊断，不产出一笔"看似合法"的费用）。
    let bad = everything_manual::providers::tripo::dto::extract_billing(&json!({
        "credits_consumed": "about thirty"
    }));
    assert!(bad.is_err());
    server.assert_no_script_problems();
}

// ---------------------------------------------------------------------------
// 执行器层：注册、端到端、断连不重发、未知状态、缺模型、上传缓存
// ---------------------------------------------------------------------------

/// 带 Tripo fixture 配置的应用（含价格目录与登录）；`TestApp` 持有临时目录并在 Drop 时清理。
async fn tripo_app(tag: &str, base_url: &str) -> (TestApp, String, String) {
    let dir = TestDir::new(tag);
    let mut settings = common::test_settings(dir.path());
    let mut tripo = common::configured_tripo(CANARY_KEY);
    tripo.base_url = base_url.to_owned();
    settings.providers.tripo = tripo;
    settings.providers.manual_ai = ProviderSettings {
        name: "manual_ai",
        base_url: "https://api.openai.com/v1".to_owned(),
        model: Some(MANUAL_AI_MODEL.to_owned()),
        api_key: Some(SecretString::new("canary-manual-ai-key")),
        key_source: Some("测试注入".to_owned()),
    };
    settings.price_catalog_path = Some(dir.join("price-catalog.toml"));
    std::fs::write(dir.join("price-catalog.toml"), TEST_CATALOG).expect("写入价格目录");
    settings.price_catalog = Some(catalog::parse(TEST_CATALOG).expect("价格目录可解析"));
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;
    let login = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&json!({ "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.text());
    let csrf = login.json()["data"]["csrfToken"]
        .as_str()
        .expect("csrfToken")
        .to_owned();
    let cookie = login.session_cookie();
    (app, cookie, csrf)
}

struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new(tag: &str) -> Self {
        Self {
            boundary: format!("----em-t12-{tag}"),
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

/// 上传一个资产的参数（multipart 的 `purpose` + `file` 两部分）。
struct FileSpec<'a> {
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
    file: FileSpec<'_>,
) -> String {
    let (boundary, body) = Multipart::new(file.purpose)
        .text_field("purpose", file.purpose)
        .file_field("file", file.filename, file.content_type, file.bytes)
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

/// 走真实 API 创建一份可执行任务：物品 + ready 准备（2 页文字）+ front/left 照片 +
/// 报价 + 确认 + 建单（预留 + 阶段 DAG）。
async fn create_ready_job(app: &TestApp, cookie: &str, csrf: &str) -> String {
    let item = app
        .call(Method::POST, "/api/v1/items")
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "name": "T12 用例物品", "model": "X100V" }))
        .send()
        .await;
    assert_eq!(item.status, StatusCode::CREATED, "{}", item.text());
    let item_id = item.json()["data"]["id"].as_str().unwrap().to_owned();

    let pdf = fixture_bytes("sample-manual-text.pdf");
    let doc_asset = upload_asset(
        app,
        cookie,
        csrf,
        &item_id,
        FileSpec {
            purpose: "document",
            filename: "manual.pdf",
            content_type: "application/pdf",
            bytes: &pdf,
        },
    )
    .await;
    let document = app
        .call(Method::POST, &format!("/api/v1/items/{item_id}/documents"))
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
    let page_text = "a".repeat(64);
    for page in 1..=2_i64 {
        let image = upload_asset(
            app,
            cookie,
            csrf,
            &item_id,
            FileSpec {
                purpose: "pageImage",
                filename: "page.jpg",
                content_type: "image/jpeg",
                bytes: &page_jpeg,
            },
        )
        .await;
        let text = upload_asset(
            app,
            cookie,
            csrf,
            &item_id,
            FileSpec {
                purpose: "pageText",
                filename: "page.txt",
                content_type: "text/plain",
                bytes: page_text.as_bytes(),
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
                "textAssetId": text,
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
    let etag = current.header("etag").expect("准备详情 ETag");
    let completed = app
        .call(
            Method::POST,
            &format!("/api/v1/preparations/{preparation}/complete"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .header("if-match", &etag)
        .json(&json!({ "pageCount": 2 }))
        .send()
        .await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text());

    let mut photo_ids: Vec<String> = Vec::new();
    for (view, name, content_type) in [
        ("front", "sample-photo-front.jpg", "image/jpeg"),
        ("left", "sample-photo-left.png", "image/png"),
    ] {
        let bytes = fixture_bytes(name);
        let asset = upload_asset(
            app,
            cookie,
            csrf,
            &item_id,
            FileSpec {
                purpose: "photo",
                filename: name,
                content_type,
                bytes: &bytes,
            },
        )
        .await;
        let response = app
            .call(Method::POST, &format!("/api/v1/items/{item_id}/photos"))
            .cookie(cookie)
            .csrf(csrf)
            .json(&json!({ "assetId": asset, "view": view }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        photo_ids.push(response.json()["data"]["id"].as_str().unwrap().to_owned());
    }

    let estimate = app
        .call(Method::POST, &format!("/api/v1/items/{item_id}/estimates"))
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({
            "preparationId": preparation,
            "photoIds": photo_ids,
            "modelPreset": PRESET,
        }))
        .send()
        .await;
    assert_eq!(estimate.status, StatusCode::CREATED, "{}", estimate.text());
    let quote_id = estimate.json()["data"]["id"].as_str().unwrap().to_owned();

    let confirmed = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{item_id}/estimates/{quote_id}/confirm"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .send()
        .await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.text());

    let job = app
        .call(Method::POST, &format!("/api/v1/items/{item_id}/jobs"))
        .cookie(cookie)
        .csrf(csrf)
        .header("idempotency-key", "t12-e2e-key-0001")
        .json(&json!({
            "quoteId": quote_id,
            "preparationId": preparation,
            "photoIds": photo_ids,
            "limits": { "tripoCreditMinor": 3000, "manualAiUsdMicros": 100_000 },
        }))
        .send()
        .await;
    assert_eq!(job.status, StatusCode::ACCEPTED, "{}", job.text());
    job.json()["data"]["id"].as_str().unwrap().to_owned()
}

fn pool(app: &TestApp) -> SqlitePool {
    app.state().database().pool().clone()
}

fn tripo_executor(app: &TestApp, clock: Arc<ManualClock>) -> Arc<JobExecutor> {
    let settings = app.state().settings().clone();
    // 生产接线检查：T13 起 upload / submit / poll / model_download / model_validate
    // 五个阶段必须注册（T14 起同一接线在 `providers.manual_ai` 已配置时另注册
    // manual_extract / manual_merge：两个 Provider 相互独立）。
    let mut wiring = StageRegistry::new();
    let registered =
        everything_manual::providers::register_provider_handlers(&mut wiring, &settings)
            .expect("已配置的 Tripo 必须能注册");
    for stage in [
        StageKind::TripoUpload,
        StageKind::TripoSubmit,
        StageKind::TripoPoll,
        StageKind::ModelDownload,
        StageKind::ModelValidate,
    ] {
        assert!(
            registered.contains(&stage),
            "T13 起注册 upload/submit/poll + 下载/校验（实际：{registered:?}）"
        );
    }
    // 本文件只驱动 Tripo 分支：执行器注册表只放 Tripo 处理器
    // （说明书分支的端到端由 `manual_ai_contract.rs` 驱动）。
    let mut registry = StageRegistry::new();
    everything_manual::providers::tripo::TripoHandlers::from_settings(&settings)
        .expect("已配置的 Tripo 必须能构造处理器")
        .register(&mut registry);
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

/// 推进若干 tick（先推进时钟，再执行；返回执行过的报告）。
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
    stages_repo::list_for_job(&mut conn, job_id)
        .await
        .expect("列出阶段")
        .into_iter()
        .find(|stage| stage.stage_kind == kind)
        .unwrap_or_else(|| panic!("阶段不存在：{}", kind.as_str()))
}

async fn stage_status(pool: &SqlitePool, job_id: &str, kind: StageKind) -> JobStatus {
    stage_of(pool, job_id, kind).await.status
}

/// 反复 tick（每次推进 20s）直到目标阶段达到期望状态；超限 panic 并给出实际现场。
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
        "阶段 {} 未在 {max_ticks} tick 内达到 {}（实际 {}；last_error={:?}）",
        kind.as_str(),
        want.as_str(),
        stage.status.as_str(),
        stage.last_error
    );
}

/// 端到端：真实 HTTP 适配器对 fixture 跑通 upload → submit（付费 POST 一次）→ poll，
/// 并把原始状态、模型 URL、credits 与账本结算落库。
#[tokio::test]
async fn fixture_end_to_end_reaches_poll_success_with_billing_settled() {
    let server = FixtureServer::start(scenario(vec![
        exact_route(
            "POST",
            "/v3/files",
            vec![
                respond_json(json!({ "code": 0, "data": { "file_token": "token-front" } })),
                respond_json(json!({ "code": 0, "data": { "image_token": "token-left" } })),
            ],
        ),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_file(SUBMIT_SUCCESS)],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            true,
            vec![respond_file(TASK_RUNNING), respond_file(TASK_SUCCESS)],
        ),
    ]));
    let (app, cookie, csrf) = tripo_app("t12-e2e", &tripo_base_url(&server)).await;
    let job_id = create_ready_job(&app, &cookie, &csrf).await;
    let db = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = tripo_executor(&app, Arc::clone(&clock));

    // 上传：两张照片各一次（front→token-front，left→token-left）。
    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoUpload,
        JobStatus::Succeeded,
        12,
    )
    .await;
    let upload = stage_of(&db, &job_id, StageKind::TripoUpload).await;
    let uploads = upload.usage_json.as_ref().expect("上传事实").clone();
    assert_eq!(uploads["uploads"][0]["view"], "front");
    assert_eq!(uploads["uploads"][0]["token"], "token-front");
    assert_eq!(uploads["uploads"][0]["tokenField"], "file_token");
    assert_eq!(uploads["uploads"][1]["view"], "left");
    assert_eq!(uploads["uploads"][1]["token"], "token-left");
    assert_eq!(uploads["uploads"][1]["tokenField"], "image_token");
    assert_eq!(
        server.call_count("POST", "/v3/files"),
        2,
        "两张照片各上传一次"
    );

    // 提交（付费）：单次请求，task_id 立即持久化为事实。
    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoSubmit,
        JobStatus::Succeeded,
        12,
    )
    .await;
    server.assert_called_once("POST", "/v3/generation/multiview-to-model");
    let submit_record = &server.requests_matching("POST", "/v3/generation/multiview-to-model")[0];
    let sent = submit_record.json_body();
    assert_eq!(sent["inputs"][0]["front"], "token-front");
    assert_eq!(sent["inputs"][1]["left"], "token-left");
    assert!(sent.get("model_version").is_none(), "不得出现 v2 字段");
    let mut conn = db.acquire().await.expect("连接");
    let attempt = everything_manual::storage::repo::attempts::latest_for_stage(
        &mut conn,
        &stage_of(&db, &job_id, StageKind::TripoSubmit).await.id,
    )
    .await
    .expect("attempt")
    .expect("提交 attempt 存在");
    assert_eq!(
        attempt.submit_state.as_str(),
        "accepted",
        "拿到 task ID 后 attempt = accepted"
    );
    assert_eq!(attempt.remote_task_id.as_deref(), Some("fixture-task-0001"));
    drop(conn);

    // 查询：先 running（waiting_provider），再 success（含模型 URL + 30 credits）。
    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoPoll,
        JobStatus::Succeeded,
        20,
    )
    .await;
    let poll = stage_of(&db, &job_id, StageKind::TripoPoll).await;
    let usage = poll.usage_json.as_ref().expect("查询事实");
    assert_eq!(usage["remoteTaskId"], "fixture-task-0001");
    assert_eq!(usage["rawStatus"], "success");
    assert_eq!(usage["normalizedStatus"], "success");
    // T20/BUG-008：临时/签名 URL **不落库**——只保存 host + sha256 摘要（AC-010）；
    // 链接本体由 `tripo_poll` → `model_download` 的进程内易失缓存传递。
    let fixture_url = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures")
                .join(TASK_SUCCESS),
        )
        .expect("读取 task_success fixture"),
    )
    .expect("fixture JSON")["data"]["output"]["model_url"]
        .as_str()
        .expect("fixture model_url")
        .to_owned();
    let summary = everything_manual::redaction::url_summary(&fixture_url);
    assert_eq!(usage["modelUrl"]["redacted"], true);
    assert_eq!(usage["modelUrl"]["host"], "cdn.example.invalid");
    assert_eq!(
        usage["modelUrl"]["sha256"], summary.sha256_prefix,
        "摘要必须是 URL 的 sha256 前缀（可复核）"
    );
    let raw_usage = usage.to_string();
    assert!(
        !raw_usage.contains("://"),
        "阶段事实不得含 URL：{raw_usage}"
    );
    assert!(
        !raw_usage.contains("fixture-signature-not-a-real-token"),
        "阶段事实不得含签名：{raw_usage}"
    );
    assert_eq!(usage["billing"]["creditMinor"], 3000);
    assert_eq!(usage["billing"]["literal"], "30");
    assert_eq!(usage["billing"]["sourceField"], "credits_consumed");

    // 账本：按供应商实际 credits 结算（30 credits = 3000 creditMinor）。
    let mut conn = db.acquire().await.expect("连接");
    let snapshot_id: String = sqlx::query_scalar("SELECT snapshot_id FROM jobs WHERE id = ?")
        .bind(&job_id)
        .fetch_one(&mut *conn)
        .await
        .expect("读取快照 id");
    let entries = ledger_repo::list_for_snapshot(&mut conn, &snapshot_id)
        .await
        .expect("账本");
    let tripo = entries
        .iter()
        .find(|entry| entry.provider == ProviderKey::Tripo)
        .expect("Tripo 预留存在");
    assert_eq!(tripo.state.as_str(), "settled", "拿到计费事实后按实际结算");
    assert_eq!(tripo.actual, Some(3000));
    assert_eq!(tripo.reserved, 3000);
    let manual_ai = entries
        .iter()
        .find(|entry| entry.provider == ProviderKey::ManualAi)
        .expect("Manual AI 预留存在");
    assert_eq!(
        manual_ai.state.as_str(),
        "reserved",
        "另一分支（说明书 AI，属 T14）不受影响"
    );
    drop(conn);

    // 计数证据：全部 HTTP 都发往回环 fixture 的三个端点，没有别处。
    assert!(server.addr().ip().is_loopback());
    let allowed = api_paths();
    for request in server.requests() {
        assert!(
            allowed
                .iter()
                .any(|prefix| request.path.starts_with(prefix)),
            "出现预期之外的请求目标：{}",
            request.summary()
        );
    }
    assert_eq!(
        server.request_total(),
        2 + 1 + 2,
        "上传 2 + 付费提交 1 + 查询 2；{}",
        server.recorded_summary().join("；")
    );
    server.assert_no_script_problems();
}

/// 付费 POST 已被接受后断连：**只发一次**（fixture 计数），结果落
/// `submission_unknown`；再次领取（含"模拟重启后的新执行器"）也不重发。
#[tokio::test]
async fn paid_post_disconnect_is_never_resent() {
    let server = FixtureServer::start(scenario(vec![
        exact_route(
            "POST",
            "/v3/files",
            vec![
                respond_json(json!({ "code": 0, "data": { "file_token": "token-front" } })),
                respond_json(json!({ "code": 0, "data": { "file_token": "token-left" } })),
            ],
        ),
        // 供应商已接收请求（fixture 先记录再执行）但连接被断开：客户端无法证明未被接受。
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![Step::Disconnect],
        ),
        prefix_route("GET", "/v3/tasks/", true, vec![respond_file(TASK_RUNNING)]),
    ]));
    let (app, cookie, csrf) = tripo_app("t12-disconnect", &tripo_base_url(&server)).await;
    let job_id = create_ready_job(&app, &cookie, &csrf).await;
    let db = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = tripo_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoSubmit,
        JobStatus::SubmissionUnknown,
        12,
    )
    .await;
    server.assert_called_once("POST", "/v3/generation/multiview-to-model");

    let submit_stage = stage_of(&db, &job_id, StageKind::TripoSubmit).await;
    let mut conn = db.acquire().await.expect("连接");
    let attempt =
        everything_manual::storage::repo::attempts::latest_for_stage(&mut conn, &submit_stage.id)
            .await
            .expect("attempt")
            .expect("attempt 存在");
    assert_eq!(attempt.submit_state.as_str(), "unknown", "断连 = 结果未知");
    assert!(attempt.remote_task_id.is_none(), "没有拿到 task ID");
    drop(conn);

    // 预留保留（unknown 不释放、不填 0）。
    let mut conn = db.acquire().await.expect("连接");
    let snapshot_id: String = sqlx::query_scalar("SELECT snapshot_id FROM jobs WHERE id = ?")
        .bind(&job_id)
        .fetch_one(&mut *conn)
        .await
        .expect("快照 id");
    let entries = ledger_repo::list_for_snapshot(&mut conn, &snapshot_id)
        .await
        .expect("账本");
    let tripo = entries
        .iter()
        .find(|entry| entry.provider == ProviderKey::Tripo)
        .expect("Tripo 预留");
    assert_eq!(
        tripo.state.as_str(),
        "unknown",
        "结果未知 → 预留保留为 unknown"
    );
    assert!(tripo.actual.is_none(), "unknown 不得把实际费用填 0");
    assert_eq!(
        tripo.attempt_id.as_deref(),
        Some(attempt.id.as_str()),
        "attempt 已关联"
    );
    drop(conn);

    // 继续 tick（含"重启"后的新执行器）都不再发付费 POST。
    run_ticks(&executor, &clock, 10, 30_000).await;
    let restarted = tripo_executor(&app, Arc::clone(&clock));
    run_ticks(&restarted, &clock, 5, 30_000).await;
    server.assert_called_once("POST", "/v3/generation/multiview-to-model");

    // 把阶段强行放回可领取队列（模拟恢复/重试入口）：执行器按未决 attempt 判未知，
    // 仍然不调用处理器、不重发（最终防线）。
    sqlx::query("UPDATE job_stages SET status = 'queued', next_run_at = NULL WHERE id = ?")
        .bind(&submit_stage.id)
        .execute(&db)
        .await
        .expect("放回队列");
    run_ticks(&executor, &clock, 5, 5_000).await;
    assert_eq!(
        stage_status(&db, &job_id, StageKind::TripoSubmit).await,
        JobStatus::SubmissionUnknown,
        "未决提交事实存在时不得重新进入付费提交"
    );
    server.assert_called_once("POST", "/v3/generation/multiview-to-model");
    server.assert_no_script_problems();
}

/// 未知远端状态：原值保留（usage 的 rawStatus）并继续等待，不得猜测成功/失败。
#[tokio::test]
async fn unknown_remote_status_is_kept_verbatim_and_waits() {
    let server = FixtureServer::start(scenario(vec![
        exact_route(
            "POST",
            "/v3/files",
            vec![
                respond_json(json!({ "code": 0, "data": { "file_token": "token-front" } })),
                respond_json(json!({ "code": 0, "data": { "file_token": "token-left" } })),
            ],
        ),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_file(SUBMIT_SUCCESS)],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            true,
            vec![respond_file(TASK_UNKNOWN_STATUS)],
        ),
    ]));
    let (app, cookie, csrf) = tripo_app("t12-unknown", &tripo_base_url(&server)).await;
    let job_id = create_ready_job(&app, &cookie, &csrf).await;
    let db = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = tripo_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoSubmit,
        JobStatus::Succeeded,
        12,
    )
    .await;
    // 查询若干次：状态始终未知 → 保持 waiting_provider，不成功也不失败。
    run_ticks(&executor, &clock, 6, 20_000).await;
    let poll = stage_of(&db, &job_id, StageKind::TripoPoll).await;
    assert_eq!(
        poll.status,
        JobStatus::WaitingProvider,
        "未知状态进入可诊断的等待（last_error={:?}）",
        poll.last_error
    );
    let usage = poll.usage_json.as_ref().expect("查询事实");
    assert_eq!(
        usage["rawStatus"], "queued_for_human_review",
        "原值必须保留"
    );
    assert_eq!(usage["normalizedStatus"], "unrecognized");
    assert!(poll.poll_count >= 1, "轮询计数已推进");
    server.assert_no_script_problems();
}

/// `success` 缺可下载模型：不组装成功（按查询失败退避重试，耗尽后 failed）；
/// 付费提交仍然只有一次。
#[tokio::test]
async fn success_without_model_is_not_success_and_does_not_repurchase() {
    let server = FixtureServer::start(scenario(vec![
        exact_route(
            "POST",
            "/v3/files",
            vec![
                respond_json(json!({ "code": 0, "data": { "file_token": "token-front" } })),
                respond_json(json!({ "code": 0, "data": { "file_token": "token-left" } })),
            ],
        ),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_file(SUBMIT_SUCCESS)],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            true,
            vec![respond_file(TASK_SUCCESS_MISSING_MODEL)],
        ),
    ]));
    let (app, cookie, csrf) = tripo_app("t12-no-model", &tripo_base_url(&server)).await;
    let job_id = create_ready_job(&app, &cookie, &csrf).await;
    let db = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = tripo_executor(&app, Arc::clone(&clock));

    // 第一次查询：success 但缺模型 → retry_wait（不是 succeeded）。
    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoPoll,
        JobStatus::RetryWait,
        20,
    )
    .await;
    let poll = stage_of(&db, &job_id, StageKind::TripoPoll).await;
    assert_ne!(poll.status, JobStatus::Succeeded);
    assert!(
        poll.last_error
            .as_deref()
            .unwrap_or_default()
            .contains("缺少可下载模型"),
        "last_error={:?}",
        poll.last_error
    );
    // 退避耗尽后为 failed（仍保留 task ID、没有第二次付费提交）。
    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoPoll,
        JobStatus::Failed,
        30,
    )
    .await;
    server.assert_called_once("POST", "/v3/generation/multiview-to-model");
    let usage = stage_of(&db, &job_id, StageKind::TripoPoll)
        .await
        .usage_json
        .clone()
        .expect("查询事实");
    assert_eq!(usage["rawStatus"], "success");
    assert_eq!(usage["normalizedStatus"], "success");
    assert!(usage.get("modelUrl").is_none());
    server.assert_no_script_problems();
}

/// 查询失败 ≠ 生成失败：5xx / 429（含 `Retry-After`）→ 可退避重试（`retry_wait`），
/// 购买事实不变（仍然只有一次付费 POST、task ID 保留）。
#[tokio::test]
async fn poll_transient_failures_retry_without_changing_the_purchase_fact() {
    let server = FixtureServer::start(scenario(vec![
        exact_route(
            "POST",
            "/v3/files",
            vec![
                respond_json(json!({ "code": 0, "data": { "file_token": "token-front" } })),
                respond_json(json!({ "code": 0, "data": { "file_token": "token-left" } })),
            ],
        ),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_file(SUBMIT_SUCCESS)],
        ),
        prefix_route(
            "GET",
            "/v3/tasks/",
            true,
            vec![
                respond_file_with_status(503, SERVER_ERROR_503),
                Step::Respond {
                    response: ResponseSpec {
                        status: 429,
                        headers: BTreeMap::from([("retry-after".to_owned(), "5".to_owned())]),
                        body: BodySpec::Json {
                            json: json!({ "code": 4290, "message": "too many requests" }),
                        },
                    },
                },
            ],
        ),
    ]));
    let (app, cookie, csrf) = tripo_app("t12-poll-backoff", &tripo_base_url(&server)).await;
    let job_id = create_ready_job(&app, &cookie, &csrf).await;
    let db = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = tripo_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoSubmit,
        JobStatus::Succeeded,
        12,
    )
    .await;

    // 第一次查询：503 → retry_wait（不是 failed，也不是成功）。
    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoPoll,
        JobStatus::RetryWait,
        20,
    )
    .await;
    let first = stage_of(&db, &job_id, StageKind::TripoPoll).await;
    let first_error = first.last_error.clone().unwrap_or_default();
    assert!(
        first_error.contains("503"),
        "last_error={first_error:?}（查询失败 ≠ 生成失败）"
    );

    // 第二次查询：429 带 Retry-After=5 → 仍 retry_wait（执行器按 T10 语义截断/尊重上限）。
    let mut saw_retry_after = false;
    for _ in 0..12 {
        run_ticks(&executor, &clock, 1, 20_000).await;
        let stage = stage_of(&db, &job_id, StageKind::TripoPoll).await;
        let error = stage.last_error.clone().unwrap_or_default();
        if error.contains("429") {
            saw_retry_after = true;
            assert_eq!(stage.status, JobStatus::RetryWait);
            break;
        }
    }
    assert!(saw_retry_after, "应观察到 429 的退避重试");

    // 购买事实不变：付费提交仍只有一次，task ID 仍在 accepted 事实里。
    server.assert_called_once("POST", "/v3/generation/multiview-to-model");
    let mut conn = db.acquire().await.expect("连接");
    let attempt = everything_manual::storage::repo::attempts::latest_for_stage(
        &mut conn,
        &stage_of(&db, &job_id, StageKind::TripoSubmit).await.id,
    )
    .await
    .expect("attempt")
    .expect("支付 attempt");
    assert_eq!(attempt.submit_state.as_str(), "accepted");
    assert_eq!(attempt.remote_task_id.as_deref(), Some("fixture-task-0001"));
    drop(conn);
    assert!(server.call_count("GET", "/v3/tasks/fixture-task-0001") >= 2);
    server.assert_no_script_problems();
}

/// 上传按**内容哈希**缓存：阶段重新执行（重试/修复后重跑）不重复上传已成功的内容。
#[tokio::test]
async fn upload_reuses_content_hash_cache_on_rerun() {
    let server = FixtureServer::start(scenario(vec![
        exact_route(
            "POST",
            "/v3/files",
            vec![
                respond_json(json!({ "code": 0, "data": { "file_token": "token-front" } })),
                respond_json(json!({ "code": 0, "data": { "file_token": "token-left" } })),
                respond_json(json!({ "code": 0, "data": { "file_token": "unexpected" } })),
            ],
        ),
        exact_route(
            "POST",
            "/v3/generation/multiview-to-model",
            vec![respond_file(SUBMIT_SUCCESS)],
        ),
        prefix_route("GET", "/v3/tasks/", true, vec![respond_file(TASK_SUCCESS)]),
    ]));
    let (app, cookie, csrf) = tripo_app("t12-cache", &tripo_base_url(&server)).await;
    let job_id = create_ready_job(&app, &cookie, &csrf).await;
    let db = pool(&app);
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = tripo_executor(&app, Arc::clone(&clock));

    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoUpload,
        JobStatus::Succeeded,
        12,
    )
    .await;
    assert_eq!(server.call_count("POST", "/v3/files"), 2);
    let upload_stage = stage_of(&db, &job_id, StageKind::TripoUpload).await;
    let cached_usage = upload_stage.usage_json.clone().expect("上传事实");

    // 把上传阶段放回队列（等价"阶段被重新执行"）：内容哈希已缓存 → 0 次新上传。
    sqlx::query("UPDATE job_stages SET status = 'queued', next_run_at = NULL WHERE id = ?")
        .bind(&upload_stage.id)
        .execute(&db)
        .await
        .expect("放回队列");
    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoUpload,
        JobStatus::Succeeded,
        12,
    )
    .await;
    assert_eq!(
        server.call_count("POST", "/v3/files"),
        2,
        "同一内容哈希不得重复上传：{}",
        server.recorded_summary().join("；")
    );
    assert_eq!(
        stage_of(&db, &job_id, StageKind::TripoUpload)
            .await
            .usage_json,
        Some(cached_usage),
        "复用缓存后的上传事实保持不变"
    );

    // 下游提交仍然使用缓存的 token。
    tick_until_stage(
        &db,
        &executor,
        &clock,
        &job_id,
        StageKind::TripoSubmit,
        JobStatus::Succeeded,
        12,
    )
    .await;
    let sent = server.requests_matching("POST", "/v3/generation/multiview-to-model")[0].json_body();
    assert_eq!(sent["inputs"][0]["front"], "token-front");
    assert_eq!(sent["inputs"][1]["left"], "token-left");
    server.assert_no_script_problems();
}

/// 观察写入的租约守卫：只有"仍是当前 epoch 持有者"才写；
/// 被接管（epoch 变化）后不得写入（不覆盖新结果）。
#[tokio::test]
async fn stale_lease_cannot_overwrite_remote_observation() {
    let server = FixtureServer::start(scenario(vec![]));
    let (app, cookie, csrf) = tripo_app("t12-lease-guard", &tripo_base_url(&server)).await;
    let job_id = create_ready_job(&app, &cookie, &csrf).await;
    let db = pool(&app);
    let stage = stage_of(&db, &job_id, StageKind::TripoPoll).await;

    assert!(
        everything_manual::providers::tripo::is_current_lease_holder(
            &db,
            &stage.id,
            stage.lease_epoch
        )
        .await,
        "当前 epoch 必须允许写入观察"
    );
    assert!(
        !everything_manual::providers::tripo::is_current_lease_holder(
            &db,
            &stage.id,
            stage.lease_epoch + 1
        )
        .await,
        "被接管（epoch 变化）后不得写入观察：{}",
        stage.lease_epoch
    );
    assert!(
        !everything_manual::providers::tripo::is_current_lease_holder(&db, "not-a-stage", 1).await,
        "阶段不存在时不写入"
    );
    assert_eq!(server.request_total(), 0, "本用例不发起任何 HTTP");
}

/// 未配置 Provider：不注册任何处理器（阶段被延后），也不产生任何 HTTP 调用。
#[tokio::test]
async fn unconfigured_provider_registers_no_handlers_and_never_calls_the_fixture() {
    let server = FixtureServer::start(scenario(vec![]));
    let dir = TestDir::new("t12-unconfigured");
    let mut settings = common::test_settings(dir.path());
    settings.providers.tripo.base_url = server.base_url();
    let app = TestApp::with_settings(dir, settings).await;

    let mut registry = StageRegistry::new();
    let registered = everything_manual::providers::register_provider_handlers(
        &mut registry,
        app.state().settings(),
    )
    .expect("未配置不报错");
    assert!(registered.is_empty());
    assert!(registry.is_empty(), "缺密钥时不得注册 Tripo 处理器");
    assert_eq!(server.request_total(), 0, "未配置不得访问任何地址");
}

/// 逐字段核对：`/v3/files` 的记录只出现一次上传目标；生产默认 base_url 是官方 https。
#[test]
fn default_base_url_is_official_https_and_paths_are_frozen() {
    let defaults = everything_manual::config::DEFAULT_TRIPO_BASE_URL;
    assert!(defaults.starts_with("https://openapi.tripo3d.ai/"));
    let (upload, submit, tasks) = everything_manual::providers::tripo::stage_endpoint_summary();
    assert_eq!(upload, "/files");
    assert_eq!(submit, "/generation/multiview-to-model");
    assert_eq!(tasks, "/tasks/");
    assert_eq!(TRIPO_UPLOAD_PATH, "/files");
    assert_eq!(TRIPO_SUBMIT_PATH, "/generation/multiview-to-model");
    assert_eq!(TRIPO_TASKS_PATH_PREFIX, "/tasks/");
}

/// 测试进程"零真实外网"的守卫：客户端拒绝非 http(s) 的 base_url，
/// 且本文件所有用例的 base_url 都来自只绑定 `127.0.0.1` 的 fixture。
#[test]
fn client_guards_against_non_fixture_targets() {
    let key = || SecretString::new(CANARY_KEY);
    assert!(
        TripoClient::new(
            "ftp://openapi.tripo3d.ai/v3",
            key(),
            TripoTimeouts::default()
        )
        .is_err()
    );
    assert!(TripoClient::new("", key(), TripoTimeouts::default()).is_err());
    // fixture 只能绑定回环：测试目标不可能是外网地址。
    let server = FixtureServer::start(scenario(vec![]));
    assert!(server.addr().ip().is_loopback(), "{}", server.addr());
    assert!(server.base_url().starts_with("http://127.0.0.1:"));
}
