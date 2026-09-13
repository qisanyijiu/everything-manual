//! T12 QA 回合 14 独立验收补充测试（QA 所有；RD 的 tripo_contract.rs 之外的独立断言）。
//!
//! 覆盖 RD 用例未直接驱动、但属 AC-041 卡内项的行为：
//! 1. **整体超时真的会触发**，且付费 POST 的超时被归类为"不能证明未被接受"
//!    （`Transport`，非 `is_definitively_refused`），一次调用只发出一次请求（HTTP 栈不重发）；
//! 2. **上传超时**同样是 `Transport`（上层按"可安全重试"处理，不产生费用）；
//! 3. **不跟随重定向**：3xx 直接失败、不把 `Authorization` 带到 Location 目标，
//!    且只发出一次请求。
//!
//! 全部 HTTP 只指向本机 fixture（`test_support` 只绑定 `127.0.0.1`，随机端口），
//! 零真实外网调用；凭据为测试假值。

use std::collections::BTreeMap;
use std::time::Duration;

use everything_manual::config::SecretString;
use everything_manual::providers::tripo::{TripoClient, TripoError, TripoTimeouts};
use serde_json::json;
use test_support::FixtureServer;
use test_support::scenario::{BodySpec, PathMatchSpec, ResponseSpec, RouteScript, Scenario, Step};

const CANARY: &str = "qa-t12-canary-not-a-real-key";

fn route(method: &str, path: &str, steps: Vec<Step>) -> RouteScript {
    RouteScript {
        method: method.to_owned(),
        path: path.to_owned(),
        path_match: PathMatchSpec::Exact,
        repeat_last: false,
        steps,
    }
}

fn client(server: &FixtureServer, timeouts: TripoTimeouts) -> TripoClient {
    TripoClient::new(
        &format!("{}/v3", server.base_url()),
        SecretString::new(CANARY),
        timeouts,
    )
    .expect("构造 Tripo 客户端（只指向本机 fixture）")
}

fn short_timeouts() -> TripoTimeouts {
    TripoTimeouts {
        connect: Duration::from_secs(2),
        request: Duration::from_millis(300),
        upload: Duration::from_millis(300),
    }
}

/// 付费 POST 整体超时：归类为传输失败（不能证明未被接受 → 上层必须按结果未知处理），
/// 且一次调用只发一次请求。
#[tokio::test]
async fn qa_submit_timeout_is_transport_not_refused_and_sent_once() {
    let server = FixtureServer::start(Scenario::new(vec![route(
        "POST",
        "/v3/generation/multiview-to-model",
        vec![Step::Timeout { hold_ms: 2_000 }],
    )]));
    let client = client(&server, short_timeouts());

    let error = client
        .submit_multiview(br#"{"inputs":[{"front":"t"}]}"#)
        .await
        .expect_err("读超时必须失败（不得当作成功）");
    assert!(
        matches!(error, TripoError::Transport { .. }),
        "超时应归入 Transport：{error:?}"
    );
    assert!(
        !error.is_definitively_refused(),
        "超时不能证明请求未被接受（付费提交必须按结果未知处理）"
    );
    assert!(
        error.redacted().contains("超时"),
        "脱敏摘要说明超时：{}",
        error.redacted()
    );
    assert_eq!(
        server.call_count("POST", "/v3/generation/multiview-to-model"),
        1,
        "一次调用只发一次请求（超时后 HTTP 栈/客户端不得重发）：{}",
        server.recorded_summary().join("；")
    );
    assert!(
        !format!("{error:?}").contains(CANARY),
        "错误 Debug 输出不得包含密钥"
    );
    server.assert_no_script_problems();
}

/// 上传整体超时：同样是传输类失败（上层可安全重试，上传不产生费用）。
#[tokio::test]
async fn qa_upload_timeout_is_transport_and_safe_to_retry() {
    let server = FixtureServer::start(Scenario::new(vec![route(
        "POST",
        "/v3/files",
        vec![Step::Timeout { hold_ms: 2_000 }],
    )]));
    let client = client(&server, short_timeouts());

    let error = client
        .upload_image("front.jpg", "image/jpeg", b"\xFF\xD8\xFF-x".to_vec())
        .await
        .expect_err("读超时必须失败");
    assert!(matches!(error, TripoError::Transport { .. }), "{error:?}");
    assert_eq!(
        server.call_count("POST", "/v3/files"),
        1,
        "一次调用只发一次请求：{}",
        server.recorded_summary().join("；")
    );
    server.assert_no_script_problems();
}

/// 不跟随重定向：3xx 直接失败（可证明未被接受），不把凭据带到 Location 目标。
#[tokio::test]
async fn qa_redirect_is_not_followed_and_bearer_is_not_forwarded() {
    let server = FixtureServer::start(Scenario::new(vec![route(
        "POST",
        "/v3/generation/multiview-to-model",
        vec![Step::Respond {
            response: ResponseSpec {
                status: 302,
                headers: BTreeMap::from([(
                    "location".to_owned(),
                    "https://redirect.example.invalid/take-the-bearer".to_owned(),
                )]),
                body: BodySpec::Json {
                    json: json!({ "code": 0, "data": {} }),
                },
            },
        }],
    )]));
    let client = client(&server, TripoTimeouts::default());

    let error = client
        .submit_multiview(br#"{}"#)
        .await
        .expect_err("3xx 必须失败（不跟随重定向）");
    assert!(
        matches!(error, TripoError::Redirected { status: 302 }),
        "{error:?}"
    );
    assert!(
        error.is_definitively_refused(),
        "3xx 未被处理，可证明未被接受"
    );
    assert_eq!(
        server.request_total(),
        1,
        "只发出一次请求（不跟随 Location）：{}",
        server.recorded_summary().join("；")
    );
    server.assert_no_script_problems();
}
