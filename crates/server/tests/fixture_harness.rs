//! T05 集成测试：**无费用 HTTP fixture 测试设施**（PRD 修订 1，REQ-008 主 / REQ-007）。
//!
//! 覆盖的验收条件：
//! - AC-014：本机 fixture 覆盖成功 / 延迟 / 断连（FIN）/ RST / 半关闭截断 / 429（含
//!   Retry-After）/ 5xx / 畸形 JSON / 超时；记录每次调用的方法/路径/请求头（脱敏）/
//!   请求体/次数；**缺脚本返回 501 而不是通用成功**；断言测试进程无真实外网调用；
//!   样例资产 sha256 与来源许可证记录在 `tests/fixtures/README.md` 与 implementation.md；
//! - AC-013（默认测试入口侧）：缺 API key 的服务不回落 mock/fixture、`/health/ready`
//!   不依赖云端、本机 fixture 记录 0 次调用；
//! - 交换条件：`test-support` 只作为 dev-dependency（生产源码不引用、`[dependencies]`
//!   不含该 crate），样例资产由仓库内生成器确定性产出。
//!
//! 全部请求都发往本机 fixture（`LocalHttpClient` 只接受回环 IP 字面量，见
//! `guarded_client_refuses_non_loopback_targets`）；测试用假凭据 canary。

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use common::TestApp;
use test_support::assets::{sha256_hex, validate_glb, validate_jpeg, validate_pdf, validate_png};
use test_support::client::{ClientError, LocalHttpClient};
use test_support::presets::{
    MANUAL_AI_RESPONSES_PATH, TRIPO_SUBMIT_PATH, TRIPO_TASKS_PREFIX, TRIPO_UPLOAD_PATH,
};
use test_support::scenario::{Scenario, fixtures_root};
use test_support::{FixtureServer, ScriptProblemKind, presets};

const PASSWORD: &str = "t05-test-password-7c2f";
/// 假密钥 canary：必须只出现在请求头，且记录中必须被脱敏。
const CANARY_KEY: &str = "sk-canary-t05-not-a-real-key";

/// 客户端读超时 400ms：延迟场景（250ms）可成功，超时场景（holdMs=60000）必然超时。
fn client() -> LocalHttpClient {
    LocalHttpClient::with_read_timeout(Duration::from_millis(400))
}

/// `tests/fixtures/scenarios/behavior_matrix.json`。
fn matrix_server() -> FixtureServer {
    FixtureServer::from_scenario_file("behavior_matrix.json")
}

// ---------------------------------------------------------------------------
// 场景行为（AC-014）
// ---------------------------------------------------------------------------

#[test]
fn success_route_records_method_path_headers_and_body() {
    let server = matrix_server();
    let response = client()
        .get(&server.url("/fixture/success"))
        .expect("成功场景应返回响应");
    assert_eq!(response.status, 200);
    assert_eq!(response.json().expect("JSON 响应体")["route"], "success");

    server.assert_called_once("GET", "/fixture/success");
    server.assert_no_script_problems();
    assert_eq!(server.request_total(), 1);

    let requests = server.requests_matching("GET", "/fixture/success");
    let recorded = &requests[0];
    assert_eq!(recorded.method, "GET");
    assert_eq!(recorded.path, "/fixture/success");
    assert_eq!(recorded.target, "/fixture/success");
    assert_eq!(recorded.query, None);
    assert_eq!(
        recorded.header_value("host"),
        Some(server.addr().to_string().as_str()),
        "记录应包含 Host 头"
    );
    assert_eq!(
        recorded.outcome,
        test_support::RecordingOutcome::Scripted {
            route_index: 0,
            step_index: 0
        }
    );
}

#[test]
fn delay_scenario_is_observed_by_the_client() {
    let server = matrix_server();
    let started = Instant::now();
    // 读超时放宽到 2s：延迟场景在负载高的机器上也不应误判为超时。
    let response = LocalHttpClient::with_read_timeout(Duration::from_secs(2))
        .get(&server.url("/fixture/delay"))
        .expect("延迟后仍应成功");
    let elapsed = started.elapsed();
    assert_eq!(response.status, 200);
    assert!(
        elapsed >= Duration::from_millis(250),
        "延迟脚本必须真的等待（delayMs=250），实际 {elapsed:?}"
    );
    server.assert_called_once("GET", "/fixture/delay");
    server.assert_no_script_problems();
}

#[test]
fn disconnect_scenario_yields_connection_error_and_is_still_recorded() {
    let server = matrix_server();
    let error = client()
        .get(&server.url("/fixture/disconnect"))
        .expect_err("断连必须是错误，不能当成功");
    assert!(
        matches!(error, ClientError::ConnectionClosed | ClientError::Io(_)),
        "实际错误：{error}"
    );
    // 记录先于行为：即使整条连接被断开，也能断言"调用发生了几次"。
    server.assert_called_once("GET", "/fixture/disconnect");
}

#[test]
fn reset_scenario_yields_io_error() {
    let server = matrix_server();
    let error = client()
        .get(&server.url("/fixture/reset"))
        .expect_err("RST 必须是错误");
    let is_reset_or_eof = match &error {
        ClientError::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
        ),
        ClientError::ConnectionClosed => true,
        _ => false,
    };
    assert!(is_reset_or_eof, "实际错误：{error}");
    server.assert_called_once("GET", "/fixture/reset");
}

#[test]
fn half_close_truncates_the_body() {
    let server = matrix_server();
    let error = client()
        .get(&server.url("/fixture/half-close"))
        .expect_err("截断的响应必须是错误");
    match &error {
        ClientError::TruncatedBody { expected, actual } => {
            assert_eq!(*expected, 20, "content-length 为完整响应体长度");
            assert_eq!(*actual, 8, "halfClose.truncateAt=8");
        }
        other => panic!("期望 TruncatedBody，实际：{other}"),
    }
    server.assert_called_once("GET", "/fixture/half-close");
}

#[test]
fn rate_limit_429_carries_retry_after() {
    let server = matrix_server();
    let response = client()
        .get(&server.url("/fixture/rate-limited"))
        .expect("429 是完整响应");
    assert_eq!(response.status, 429);
    assert_eq!(
        response.header("retry-after"),
        Some("2"),
        "429 场景必须带 Retry-After（重试退避要读它）"
    );
    assert_eq!(
        response.json().expect("JSON 错误体")["error"]["code"],
        "RATE_LIMITED"
    );
    server.assert_called_once("GET", "/fixture/rate-limited");
    server.assert_no_script_problems();
}

#[test]
fn server_error_5xx_is_served_as_is() {
    let server = matrix_server();
    let response = client()
        .get(&server.url("/fixture/server-error"))
        .expect("5xx 是完整响应");
    assert_eq!(response.status, 503);
    assert_eq!(response.text(), "upstream unavailable");
    assert_eq!(
        response.header("content-type"),
        Some("text/plain; charset=utf-8")
    );
    server.assert_called_once("GET", "/fixture/server-error");
}

#[test]
fn malformed_json_is_served_verbatim() {
    let server = matrix_server();
    let response = client()
        .get(&server.url("/fixture/malformed-json"))
        .expect("HTTP 层仍应成功");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.header("content-type"),
        Some("application/json"),
        "畸形 JSON 也按声明的 content-type 提供"
    );
    assert!(
        response.json().is_err(),
        "fixture 必须原样提供畸形 JSON，让适配器自己失败：{}",
        response.text()
    );
    assert!(response.text().ends_with("{\"code\": 0, \"data\": {"));
    server.assert_called_once("GET", "/fixture/malformed-json");
}

#[test]
fn timeout_scenario_times_out_the_client() {
    let server = matrix_server();
    let started = Instant::now();
    let error = client()
        .get(&server.url("/fixture/timeout"))
        .expect_err("超时必须是错误");
    assert!(matches!(error, ClientError::Timeout), "实际错误：{error}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "客户端（400ms）应远早于 fixture 的 holdMs（60s）失败，实际 {:?}",
        started.elapsed()
    );
    server.assert_called_once("GET", "/fixture/timeout");
}

#[test]
fn missing_route_returns_501_instead_of_generic_success() {
    let server = matrix_server();
    let response = client()
        .get(&server.url("/fixture/not-scripted"))
        .expect("501 也是一个完整响应");
    assert_eq!(response.status, 501, "缺脚本必须显式失败，不能返回通用成功");
    assert_eq!(
        response.json().expect("501 错误体是 JSON")["error"],
        "fixture script missing"
    );

    let problems = server.script_problems();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].kind, ScriptProblemKind::NoRoute);
    assert!(problems[0].describe().contains("/fixture/not-scripted"));
    // 记录仍保存：可以断言"被测代码访问了这个未脚本化路径"。
    server.assert_called_once("GET", "/fixture/not-scripted");
}

#[test]
fn exhausted_script_returns_501_unless_repeat_last() {
    let server = matrix_server();

    // repeatLast：同一步骤可重复命中。
    for _ in 0..3 {
        let response = client()
            .get(&server.url("/fixture/repeats"))
            .expect("repeatLast 路由");
        assert_eq!(response.status, 200);
    }
    server.assert_called_times("GET", "/fixture/repeats", 3);
    server.assert_no_script_problems();

    // 未声明 repeatLast：第一次成功，第二次必须显式失败并记录脚本问题。
    assert_eq!(
        client()
            .get(&server.url("/fixture/once"))
            .expect("首次")
            .status,
        200
    );
    let second = client()
        .get(&server.url("/fixture/once"))
        .expect("501 响应");
    assert_eq!(second.status, 501);
    assert!(
        second.json().expect("JSON 501 体")["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("exhausted"),
        "说明信息应指出步骤耗尽"
    );
    server.assert_called_times("GET", "/fixture/once", 2);

    let problems = server.script_problems();
    assert_eq!(problems.len(), 1, "只有 once 路由应产生脚本问题");
    assert!(
        matches!(problems[0].kind, ScriptProblemKind::ScriptExhausted { .. }),
        "实际：{:?}",
        problems[0]
    );
}

// ---------------------------------------------------------------------------
// 供应商路径与"只调用一次"的计数/请求体断言
// ---------------------------------------------------------------------------

#[test]
fn tripo_happy_flow_records_a_single_paid_post() {
    let server = presets::tripo_happy();
    let http = client();

    // 1) 上传（multipart 的字节协议由 T12 细测；这里断言路径与记录能力）。
    let upload = http
        .post_bytes(
            &server.url(TRIPO_UPLOAD_PATH),
            Some("image/png"),
            b"fixture-front-image-bytes",
        )
        .expect("上传响应");
    assert_eq!(upload.status, 200);
    assert_eq!(
        upload.json().expect("JSON")["data"]["image_token"],
        "fixture-image-token-front-0001"
    );

    // 2) 付费提交：恰好一次，且请求体可断言。
    let submit_body = serde_json::json!({
        "inputs": [
            { "front": "fixture-image-token-front-0001" },
            { "left": "fixture-image-token-left-0001" }
        ],
        "model": "v3.1-20260211",
        "texture": true,
        "pbr": true,
        "texture_quality": "standard",
        "geometry_quality": "standard",
        "face_limit": 100000,
        "quad": false,
        "generate_parts": false
    });
    let body_bytes = serde_json::to_vec(&submit_body).expect("序列化请求体");
    let submit = http
        .request(
            "POST",
            &server.url(TRIPO_SUBMIT_PATH),
            &[
                ("content-type", "application/json"),
                ("authorization", "Bearer sk-canary-t05-not-a-real-key"),
            ],
            Some(&body_bytes),
        )
        .expect("提交响应");
    assert_eq!(submit.status, 200);
    assert_eq!(
        submit.json().expect("JSON")["data"]["task_id"],
        "fixture-task-0001"
    );

    // "付费 POST 只发了 1 次"——本卡的示例断言 API。
    server.assert_called_once("POST", TRIPO_SUBMIT_PATH);
    let recorded = &server.requests_matching("POST", TRIPO_SUBMIT_PATH)[0];
    let recorded_body = recorded.json_body();
    assert_eq!(recorded_body["model"], "v3.1-20260211");
    assert_eq!(recorded_body["face_limit"], 100000);
    assert_eq!(recorded_body["quad"], false);
    assert_eq!(recorded_body["generate_parts"], false);
    assert_eq!(
        recorded_body["inputs"][0]["front"],
        "fixture-image-token-front-0001"
    );
    assert_eq!(
        recorded.header_value("content-type"),
        Some("application/json")
    );

    // 3) 请求头脱敏：保留 scheme，不保留密钥明文。
    assert_eq!(
        recorded.header_value("authorization"),
        Some("Bearer [REDACTED]"),
        "Authorization 必须脱敏但保留 scheme（便于断言是否携带 bearer）"
    );
    assert!(
        !format!("{recorded:?}").contains(CANARY_KEY),
        "记录（含 Debug 输出）不得包含密钥明文"
    );

    // 4) 轮询两次：第一次 running、第二次重复返回 success（repeatLast）。
    let task_url = server.url(&format!("{TRIPO_TASKS_PREFIX}fixture-task-0001"));
    let poll_running = http.get(&task_url).expect("第一次轮询");
    assert_eq!(
        poll_running.json().expect("JSON")["data"]["status"],
        "running"
    );
    let poll_success = http.get(&task_url).expect("第二次轮询");
    assert_eq!(
        poll_success.json().expect("JSON")["data"]["status"],
        "success"
    );

    server.assert_called_times("GET", &format!("{TRIPO_TASKS_PREFIX}fixture-task-0001"), 2);
    server.assert_called_times("POST", TRIPO_UPLOAD_PATH, 1);
    assert_eq!(
        server.request_total(),
        4,
        "{}",
        server.recorded_summary().join("；")
    );
    server.assert_no_script_problems();
}

#[test]
fn manual_ai_happy_flow_serves_responses_payload() {
    let server = presets::manual_ai_happy();
    let request = serde_json::json!({
        "model": "manual-ai-fixture-model",
        "input": [ {
            "role": "user",
            "content": [ { "type": "input_text", "text": "extract parts/steps" } ]
        } ],
        "text": {
            "format": {
                "type": "json_schema",
                "name": "manual_extract_v1",
                "strict": true,
                "schema": { "type": "object" }
            }
        }
    });
    let response = client()
        .post_json(&server.url(MANUAL_AI_RESPONSES_PATH), &request)
        .expect("Responses 响应");
    assert_eq!(response.status, 200);
    let payload = response.json().expect("JSON");
    assert_eq!(payload["status"], "completed");
    let inner_text = payload["output"][0]["content"][0]["text"]
        .as_str()
        .expect("output[].content[].text");
    let inner: serde_json::Value =
        serde_json::from_str(inner_text).expect("内层 strict JSON 应可解析");
    assert_eq!(inner["schemaVersion"], "manual_extract_v1");
    assert_eq!(inner["steps"][0]["evidence"][0]["pageNumber"], 1);

    server.assert_called_once("POST", MANUAL_AI_RESPONSES_PATH);
    let recorded = &server.requests_matching("POST", MANUAL_AI_RESPONSES_PATH)[0];
    assert_eq!(
        recorded.header_value("content-type"),
        Some("application/json")
    );
    assert_eq!(
        recorded.json_body()["text"]["format"]["type"],
        "json_schema"
    );
    server.assert_no_script_problems();
}

// ---------------------------------------------------------------------------
// 外网隔离（AC-014："测试进程不得产生真实外网调用"）
// ---------------------------------------------------------------------------

#[test]
fn guarded_client_refuses_non_loopback_targets() {
    let http = client();

    let hostname = http
        .get("http://example.com/fixture")
        .expect_err("主机名必须被拒绝（不做 DNS）");
    assert!(
        matches!(hostname, ClientError::NotIpLiteral { .. }),
        "{hostname}"
    );

    let public_v4 = http
        .get("http://93.184.216.34/")
        .expect_err("公网 IPv4 必须被拒绝");
    assert!(
        matches!(public_v4, ClientError::NotLoopback { .. }),
        "{public_v4}"
    );

    let public_v6 = http
        .get("http://[2001:db8::1]/")
        .expect_err("公网 IPv6 必须被拒绝");
    assert!(
        matches!(public_v6, ClientError::NotLoopback { .. }),
        "{public_v6}"
    );

    let https = http
        .get("https://127.0.0.1:443/")
        .expect_err("https 必须被拒绝（fixture 无 TLS）");
    assert!(
        matches!(https, ClientError::UnsupportedScheme { .. }),
        "{https}"
    );

    // 回环地址是唯一放行目标。
    let server = matrix_server();
    assert!(
        server.addr().ip().is_loopback(),
        "fixture 只允许绑定回环地址，实际 {}",
        server.addr()
    );
    assert_eq!(
        client()
            .get(&server.url("/fixture/success"))
            .expect("回环可访问")
            .status,
        200
    );
}

// ---------------------------------------------------------------------------
// 生产隔离（"测试开关不能进入生产默认"）
// ---------------------------------------------------------------------------

#[test]
fn test_support_stays_out_of_the_release_dependency_tree() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest_text = std::fs::read_to_string(crate_dir.join("Cargo.toml")).expect("读取清单");
    let manifest: toml::Value = toml::from_str(&manifest_text).expect("解析清单");
    let dependencies = manifest["dependencies"]
        .as_table()
        .expect("[dependencies] 段存在");
    assert!(
        !dependencies.contains_key("test-support"),
        "test-support 不得出现在 [dependencies]（会进入生产依赖树）"
    );
    let dev_dependencies = manifest["dev-dependencies"]
        .as_table()
        .expect("[dev-dependencies] 段存在");
    assert!(
        dev_dependencies.contains_key("test-support"),
        "fixture_harness 依赖 test-support（[dev-dependencies]）"
    );

    // 生产源码不得引用 fixture crate（也不得出现第二套"仅测试"开关）。
    let mut checked = 0_usize;
    for file in rust_sources(&crate_dir.join("src")) {
        let text = std::fs::read_to_string(&file).expect("读取源码");
        assert!(
            !text.contains("test_support") && !text.contains("test-support"),
            "{} 引用了 test-support",
            file.display()
        );
        checked += 1;
    }
    assert!(checked > 0, "应至少检查到生产源码文件");

    // 默认 Provider 地址是官方 https 域名，不是本机 fixture 或 mock。
    for base_url in [
        everything_manual::config::DEFAULT_TRIPO_BASE_URL,
        everything_manual::config::DEFAULT_MANUAL_AI_BASE_URL,
    ] {
        assert!(base_url.starts_with("https://"), "{base_url}");
        assert!(!base_url.contains("127.0.0.1"), "{base_url}");
        assert!(!base_url.contains("localhost"), "{base_url}");
    }
}

/// 递归收集 `.rs` 源码文件。
fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).expect("读取目录") {
            let path = entry.expect("目录项").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// AC-013（默认测试入口侧）：缺 API key 的服务不回落 mock/fixture，就绪不依赖云端。
#[tokio::test]
async fn app_without_provider_keys_never_contacts_the_fixture() {
    // 一支"如果被回退调用就会留下记录"的 fixture 服务器。
    let fixture = presets::tripo_happy();
    let app = TestApp::new("t05-no-keys").await;

    let ready = app.call(Method::GET, "/api/v1/health/ready").send().await;
    assert_eq!(
        ready.status,
        StatusCode::OK,
        "缺密钥的服务必须仍然就绪（ready 不依赖云端）"
    );

    let admin_id = app.set_admin_password(PASSWORD).await;
    let (cookie, _csrf, _session) = app.insert_session(&admin_id, 3_600_000).await;
    let status = app
        .call(Method::GET, "/api/v1/settings/status")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(status.status, StatusCode::OK);
    let body = status.json();
    assert_eq!(body["data"]["providersConfigured"]["tripo"], false);
    assert_eq!(body["data"]["providersConfigured"]["manualAi"], false);
    assert_eq!(body["data"]["capabilities"]["generation"], false);

    let settings = app.state().settings();
    assert_eq!(
        settings.providers.tripo.base_url,
        everything_manual::config::DEFAULT_TRIPO_BASE_URL
    );
    assert!(settings.providers.tripo.api_key.is_none());
    assert!(settings.providers.manual_ai.api_key.is_none());

    // 关键断言：缺密钥时不存在"回退到本机 fixture/mock"的路径。
    assert_eq!(
        fixture.request_total(),
        0,
        "服务不得因缺密钥访问本机 Provider fixture：{:?}",
        fixture.recorded_summary()
    );
}

// ---------------------------------------------------------------------------
// 样例资产（AC-014 第 5 项）
// ---------------------------------------------------------------------------

/// 仓库中提交的样例资产 sha256（与 `tests/fixtures/README.md` 一致）。
///
/// 顺序即 `test_support::generate::build_all()` 的顺序（生成器与清单逐项对齐）。
const PINNED_ASSETS: [(&str, &str); 9] = [
    (
        "sample-model.glb",
        "a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb",
    ),
    (
        "sample-manual-text.pdf",
        "e18cf61a61c9f9834a73e7454c9f1c3a50e897908474f21edac4759fd3695cda",
    ),
    (
        "sample-manual-scan.pdf",
        "4080019bb8f1d0b9db446add474721734d7dbf8f18e0f62bfa5c59c661db2ce7",
    ),
    // T09：PDF 准备的边界与失败样例（旋转页 / 非拉丁字体 / 加密 / 101 页）。
    (
        "sample-manual-rotated.pdf",
        "4a8821d3bf058c24f0378b958ccc98f20feedfbc32bd1f216a1e1a8b83a400fd",
    ),
    (
        "sample-manual-nonlatin.pdf",
        "d6640acbd6d3d6aec3e5fac59aa366c9263340669d74124849e1216488fb9030",
    ),
    (
        "sample-manual-encrypted.pdf",
        "2916fa405c1bd33812808ee18d7b1522c019ecf741f4749ba965b1b340e21503",
    ),
    (
        "sample-manual-many-pages.pdf",
        "da6bc156855dcfe45afc6a223ad9ea0164fe6e6c478a263cb1df392a3e6a6b85",
    ),
    (
        "sample-photo-front.jpg",
        "122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586",
    ),
    (
        "sample-photo-left.png",
        "0a74e5a5b542aa62440b48aee24838b664802bf5a98cd73fed45123cd7d2c890",
    ),
];

fn asset_bytes(name: &str) -> Vec<u8> {
    let path = fixtures_root().join("assets").join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()))
}

#[test]
fn sample_assets_are_generated_deterministically_and_validate() {
    let generated = test_support::generate::build_all();
    assert_eq!(generated.len(), PINNED_ASSETS.len(), "生成器与清单数量一致");
    for ((name, bytes), (pinned_name, pinned_hash)) in generated.iter().zip(PINNED_ASSETS.iter()) {
        assert_eq!(name, pinned_name);
        // 1) 生成器确定性：同一源码两次生成字节一致（sha256 固定）。
        assert_eq!(
            &sha256_hex(bytes),
            pinned_hash,
            "{name} 重新生成的字节与固定哈希不符（生成器输入疑似变化）"
        );
        // 2) 仓库资产 == 生成结果：证明"自建"而非来源不明的外部文件。
        let committed = asset_bytes(name);
        assert_eq!(
            &committed, bytes,
            "{name} 仓库字节与生成结果不一致；请运行 cargo run -p test-support --bin generate-fixtures 并更新记录"
        );
    }

    // 3) GLB 结构解析（magic/版本/长度/chunk/范围/自包含），并对照 PRD §5.3 预算。
    let glb = validate_glb(&asset_bytes("sample-model.glb")).expect("GLB 结构校验");
    assert_eq!(glb.version, 2);
    assert_eq!(glb.triangles, 12);
    assert_eq!(glb.image_count, 1);
    assert!(glb.bin_len > 0, "必须自包含 BIN");
    assert!(glb.triangles <= 100_000, "三角面 ≤100000（PRD §5.3）");
    assert!(
        glb.max_texture_dimension <= 4096,
        "贴图单边 ≤4096（PRD §5.3）"
    );

    // 4) 文字型 PDF：2 页、有文字层、无图片对象。
    let text = validate_pdf(&asset_bytes("sample-manual-text.pdf")).expect("文字 PDF 结构校验");
    assert_eq!(text.version, "1.4");
    assert_eq!(text.page_count, 2);
    assert_eq!(text.declared_count, 2);
    assert_eq!(text.xref_entries_checked, 7);
    assert!(text.has_text_operators, "文字型 PDF 必须有文字层");
    assert_eq!(text.image_xobjects, 0);

    // 5) 扫描型 PDF：2 页、页图为栅格、无文字层。
    let scan = validate_pdf(&asset_bytes("sample-manual-scan.pdf")).expect("扫描 PDF 结构校验");
    assert_eq!(scan.page_count, 2);
    assert_eq!(scan.xref_entries_checked, 8);
    assert!(!scan.has_text_operators, "扫描型 PDF 不得有文字层");
    assert_eq!(scan.image_xobjects, 2, "每页一个栅格页图");
    assert!(
        scan.max_image_dimension <= 2000,
        "页图长边 ≤2000 px（PRD §5.3）"
    );

    // 5b) T09 样例：旋转页 / 非拉丁字体 / 加密 / 101 页（准备阶段的边界与拒绝路径）。
    let rotated = validate_pdf(&asset_bytes("sample-manual-rotated.pdf")).expect("旋转页 PDF");
    assert_eq!(rotated.page_count, 2);
    assert!(rotated.has_text_operators, "旋转页也要有文字层");
    let rotated_text =
        String::from_utf8_lossy(&asset_bytes("sample-manual-rotated.pdf")).into_owned();
    assert!(
        rotated_text.contains("/Rotate 90"),
        "第 2 页必须声明 /Rotate 90"
    );

    let nonlatin =
        validate_pdf(&asset_bytes("sample-manual-nonlatin.pdf")).expect("非拉丁字体 PDF");
    assert_eq!(nonlatin.page_count, 1);
    assert!(
        nonlatin.has_text_operators,
        "非拉丁字体页必须有文字层（Type0 + ToUnicode）"
    );
    let nonlatin_text =
        String::from_utf8_lossy(&asset_bytes("sample-manual-nonlatin.pdf")).into_owned();
    assert!(
        nonlatin_text.contains("/UniGB-UCS2-H"),
        "必须使用非 Identity 的 CMap：e2e 据此证明 CMaps 从本地 vendor 目录加载"
    );
    assert!(
        nonlatin_text.contains("/ToUnicode"),
        "文字提取依赖 ToUnicode"
    );

    let encrypted = validate_pdf(&asset_bytes("sample-manual-encrypted.pdf")).expect("加密 PDF");
    assert_eq!(encrypted.page_count, 1);
    let encrypted_text =
        String::from_utf8_lossy(&asset_bytes("sample-manual-encrypted.pdf")).into_owned();
    assert!(
        encrypted_text.contains("/Filter /Standard"),
        "必须带标准安全处理器（V=1/R=2，真实 O/U）"
    );
    assert!(
        encrypted_text.contains("/Encrypt"),
        "trailer 必须声明 /Encrypt：PDF.js 未提供口令时抛 PasswordException"
    );

    let many = validate_pdf(&asset_bytes("sample-manual-many-pages.pdf")).expect("101 页 PDF");
    assert_eq!(many.page_count, 101);
    assert_eq!(
        many.declared_count, 101,
        "校验器必须跟随 /Count 的间接引用（否则 declared_count 会被误读为 3）"
    );
    assert!(many.has_text_operators);

    // 6) 图片可解码（结构级：SOI…EOI / 签名与 chunk CRC）。
    let jpeg = validate_jpeg(&asset_bytes("sample-photo-front.jpg")).expect("JPEG 结构校验");
    assert_eq!((jpeg.width, jpeg.height), (32, 32));
    assert_eq!(jpeg.components, 1);
    assert!(!jpeg.progressive);
    let png = validate_png(&asset_bytes("sample-photo-left.png")).expect("PNG 结构校验");
    assert_eq!((png.width, png.height), (64, 64));
    assert_eq!(png.bit_depth, 8);

    // 7) 资产体积远低于上传上限（PRD §5.3 照片 20 MiB / 原 PDF 50 MiB）。
    for (name, _) in PINNED_ASSETS {
        let size = asset_bytes(name).len();
        assert!(size <= 20 * 1024 * 1024, "{name} 超过照片上限：{size} 字节");
    }
}

#[test]
fn all_committed_scenarios_parse_and_resolve() {
    let scenarios_dir = fixtures_root().join("scenarios");
    let mut names: Vec<String> = std::fs::read_dir(&scenarios_dir)
        .expect("scenarios 目录")
        .map(|entry| {
            entry
                .expect("目录项")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".json"))
        .collect();
    names.sort();
    assert!(
        names.len() >= 3,
        "至少应包含 tripo_happy / manual_ai_happy / behavior_matrix：{names:?}"
    );
    for name in names {
        let scenario = Scenario::load_file(&scenarios_dir.join(&name), &fixtures_root());
        let resolved = scenario.resolve();
        assert!(
            !resolved.routes.is_empty(),
            "{name} 解析后没有路由（脚本写错或字段名漂移）"
        );
    }
}
