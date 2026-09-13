//! T05 QA 独立探针（回合 5）：不依赖 RD 的 `fixture_harness.rs`，只通过
//! `test-support` 公共 API 驱动 fixture 服务器与客户端，独立复核 AC-014/AC-013
//! 默认入口侧的关键断言。
//!
//! 运行：cargo run --offline --quiet
//! 退出码非 0 = 有 FAIL。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use test_support::client::{ClientError, LocalHttpClient};
use test_support::scenario::{Scenario, fixtures_root};
use test_support::{FixtureServer, ScriptProblemKind};

const CANARY: &str = "sk-canary-qa-t05-not-a-real-key";

struct Probe {
    failures: usize,
    checks: usize,
}

impl Probe {
    fn check(&mut self, name: &str, ok: bool, detail: impl std::fmt::Display) {
        self.checks += 1;
        if ok {
            println!("CHECK {name}: OK  [{detail}]");
        } else {
            self.failures += 1;
            println!("CHECK {name}: FAIL [{detail}]");
        }
    }
}

fn matrix() -> FixtureServer {
    FixtureServer::from_scenario_file("behavior_matrix.json")
}

fn client() -> LocalHttpClient {
    LocalHttpClient::with_read_timeout(Duration::from_millis(400))
}

fn main() {
    let mut p = Probe { failures: 0, checks: 0 };

    // 0) 绑定：只回环 + 随机端口。
    let server = matrix();
    p.check(
        "bind-loopback",
        server.addr().ip().is_loopback() && server.addr().port() != 0,
        format!("addr={}", server.addr()),
    );

    // 1) 成功场景 + 记录（含查询串、请求头、请求体）。
    let s = matrix();
    let r = client().get(&s.url("/fixture/success?qa=1&canary=QA_T05_QUERY"));
    match r {
        Ok(resp) => {
            p.check("success-status", resp.status == 200, format!("status={}", resp.status));
            p.check(
                "success-json-body",
                resp.json().map(|v| v["route"] == "success").unwrap_or(false),
                resp.text(),
            );
        }
        Err(e) => p.check("success-request", false, e.to_string()),
    }
    let recs = s.requests_matching("GET", "/fixture/success");
    p.check("record-count", recs.len() == 1, format!("count={}", recs.len()));
    if let Some(rec) = recs.first() {
        p.check(
            "record-target-query",
            rec.target == "/fixture/success?qa=1&canary=QA_T05_QUERY"
                && rec.query.as_deref() == Some("qa=1&canary=QA_T05_QUERY"),
            format!("target={} query={:?}", rec.target, rec.query),
        );
        p.check(
            "record-host-header",
            rec.header_value("host") == Some(s.addr().to_string().as_str()),
            format!("host={:?}", rec.header_value("host")),
        );
        p.check(
            "record-outcome-scripted",
            format!("{:?}", rec.outcome).contains("Scripted"),
            format!("outcome={:?}", rec.outcome),
        );
    } else {
        p.check("record-fields", false, "无记录可查");
    }
    p.check(
        "summary-api",
        !s.recorded_summary().is_empty(),
        s.recorded_summary().join("；"),
    );

    // 2) 缺脚本：501（不是通用成功）+ script problem 记录。
    let s = matrix();
    match client().get(&s.url("/fixture/qa-not-scripted")) {
        Ok(resp) => {
            p.check("missing-status-501", resp.status == 501, format!("status={}", resp.status));
            let body = resp.json().unwrap_or_default();
            p.check(
                "missing-error-shape",
                body["error"] == "fixture script missing",
                body.to_string(),
            );
        }
        Err(e) => p.check("missing-request", false, e.to_string()),
    }
    let problems = s.script_problems();
    p.check(
        "missing-script-problem-recorded",
        problems.len() == 1 && problems[0].kind == ScriptProblemKind::NoRoute,
        format!("problems={problems:?}"),
    );
    p.check(
        "missing-still-recorded",
        s.call_count("GET", "/fixture/qa-not-scripted") == 1,
        format!("count={}", s.call_count("GET", "/fixture/qa-not-scripted")),
    );

    // 2b) 方法不匹配（POST 到只有 GET 的路由）同样 501，不是通用成功。
    let s = matrix();
    match client().request("POST", &s.url("/fixture/success"), &[], Some(b"{}")) {
        Ok(resp) => p.check(
            "method-mismatch-501",
            resp.status == 501,
            format!("status={}", resp.status),
        ),
        Err(e) => p.check("method-mismatch-501", false, e.to_string()),
    }

    // 3) 步骤耗尽：第二次 501，repeatLast 不受限。
    let s = matrix();
    let first = client().get(&s.url("/fixture/once"));
    let second = client().get(&s.url("/fixture/once"));
    let repeat: Vec<u16> = (0..3)
        .map(|_| client().get(&s.url("/fixture/repeats")).map(|r| r.status).unwrap_or(0))
        .collect();
    p.check(
        "once-first-200-second-501",
        first.as_ref().map(|r| r.status).ok() == Some(200)
            && second.as_ref().map(|r| r.status).ok() == Some(501),
        format!(
            "first={:?} second={:?}",
            first.as_ref().map(|r| r.status),
            second.as_ref().map(|r| r.status)
        ),
    );
    p.check(
        "repeat-last-3x200",
        repeat == vec![200, 200, 200],
        format!("{repeat:?}"),
    );
    let problems = s.script_problems();
    p.check(
        "exhausted-problem-kind",
        problems.len() == 1
            && matches!(problems[0].kind, ScriptProblemKind::ScriptExhausted { .. })
            && problems[0].describe().contains("repeatLast"),
        format!("problems={problems:?}"),
    );

    // 4) 故障矩阵：延迟 / 断连 / RST / 半关闭 / 429 / 5xx / 畸形 JSON / 超时。
    let s = matrix();
    let t0 = Instant::now();
    match client().get(&s.url("/fixture/delay")) {
        Ok(resp) => p.check(
            "delay-observed",
            resp.status == 200 && t0.elapsed() >= Duration::from_millis(250),
            format!("status={} elapsed={:?}", resp.status, t0.elapsed()),
        ),
        Err(e) => p.check("delay-observed", false, e.to_string()),
    }

    let s = matrix();
    let e = client().get(&s.url("/fixture/disconnect")).unwrap_err();
    p.check(
        "disconnect-error",
        matches!(e, ClientError::ConnectionClosed | ClientError::Io(_)),
        e.to_string(),
    );
    p.check(
        "disconnect-recorded",
        s.call_count("GET", "/fixture/disconnect") == 1,
        format!("count={}", s.call_count("GET", "/fixture/disconnect")),
    );

    let s = matrix();
    let e = client().get(&s.url("/fixture/reset")).unwrap_err();
    let reset_like = match &e {
        ClientError::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
        ),
        ClientError::ConnectionClosed => true,
        _ => false,
    };
    p.check("reset-error", reset_like, e.to_string());

    let s = matrix();
    let e = client().get(&s.url("/fixture/half-close")).unwrap_err();
    p.check(
        "half-close-truncated",
        matches!(&e, ClientError::TruncatedBody { expected: 20, actual: 8 }),
        e.to_string(),
    );

    let s = matrix();
    match client().get(&s.url("/fixture/rate-limited")) {
        Ok(resp) => p.check(
            "429-with-retry-after",
            resp.status == 429 && resp.header("retry-after") == Some("2"),
            format!("status={} retry-after={:?}", resp.status, resp.header("retry-after")),
        ),
        Err(e) => p.check("429-with-retry-after", false, e.to_string()),
    }

    let s = matrix();
    match client().get(&s.url("/fixture/server-error")) {
        Ok(resp) => p.check(
            "5xx-served",
            resp.status == 503 && resp.text() == "upstream unavailable",
            format!("status={} body={}", resp.status, resp.text()),
        ),
        Err(e) => p.check("5xx-served", false, e.to_string()),
    }

    let s = matrix();
    match client().get(&s.url("/fixture/malformed-json")) {
        Ok(resp) => p.check(
            "malformed-json-verbatim",
            resp.status == 200 && resp.json().is_err() && resp.text().ends_with("{\"code\": 0, \"data\": {"),
            resp.text(),
        ),
        Err(e) => p.check("malformed-json-verbatim", false, e.to_string()),
    }

    let s = matrix();
    let t0 = Instant::now();
    let e = client().get(&s.url("/fixture/timeout")).unwrap_err();
    p.check(
        "timeout-capped",
        matches!(e, ClientError::Timeout) && t0.elapsed() < Duration::from_secs(5),
        format!("{e} elapsed={:?}", t0.elapsed()),
    );

    // 5) 客户端准入：主机名/公网 IPv4/IPv6/https 在建连前被拒。
    let c = client();
    let cases: Vec<(&str, String)> = vec![
        ("http://example.com/x", "NotIpLiteral".into()),
        ("http://93.184.216.34/", "NotLoopback".into()),
        ("http://[2001:db8::1]/", "NotLoopback".into()),
        ("http://0.0.0.0:9/", "NotLoopback".into()),
        ("https://127.0.0.1:443/", "UnsupportedScheme".into()),
    ];
    for (url, expected) in cases {
        let got = match c.get(url) {
            Err(e) => format!("{e:?}"),
            Ok(r) => format!("UNEXPECTED OK status={}", r.status),
        };
        p.check(
            "guard-reject",
            got.contains(&expected),
            format!("{url} -> {got}"),
        );
    }

    // 6) 记录请求体 + 敏感头脱敏（含 Debug 无明文）。
    let s = test_support::presets::tripo_happy();
    let body = serde_json::json!({"qaProbe": true, "n": 42});
    let bytes = serde_json::to_vec(&body).unwrap();
    let resp = client()
        .request(
            "POST",
            &s.url("/v3/files"),
            &[
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {CANARY}")),
            ],
            Some(&bytes),
        )
        .expect("上传路由");
    p.check("recorded-post-status", resp.status == 200, format!("status={}", resp.status));
    let recs = s.requests_matching("POST", "/v3/files");
    if let Some(rec) = recs.first() {
        p.check(
            "recorded-body-json",
            rec.json_body()["n"] == 42 && rec.body == bytes,
            format!("body={}", rec.body_text()),
        );
        p.check(
            "authorization-redacted",
            rec.header_value("authorization") == Some("Bearer [REDACTED]"),
            format!("authorization={:?}", rec.header_value("authorization")),
        );
        p.check(
            "debug-no-canary",
            !format!("{rec:?}").contains(CANARY),
            "Debug 输出不含密钥明文".to_string(),
        );
    } else {
        p.check("recorded-post", false, "无记录可查");
    }

    // 7) 供应商路径「付费 POST 恰好 1 次」+ 请求体断言（tripo_happy 全流程）。
    let s = test_support::presets::tripo_happy();
    let http = client();
    let _ = http.post_bytes(&s.url("/v3/files"), Some("image/png"), b"img-bytes");
    let submit_body = serde_json::json!({
        "inputs": [{"front": "tok-front"}, {"left": "tok-left"}],
        "model": "v3.1-20260211",
        "face_limit": 100000,
        "quad": false
    });
    let _ = http.post_json(&s.url("/v3/generation/multiview-to-model"), &submit_body);
    let _ = http.get(&s.url("/v3/tasks/task-1"));
    let _ = http.get(&s.url("/v3/tasks/task-1"));
    s.assert_called_once("POST", "/v3/generation/multiview-to-model");
    p.check(
        "paid-post-once",
        s.call_count("POST", "/v3/generation/multiview-to-model") == 1
            && s.call_count("GET", "/v3/tasks/task-1") == 2
            && s.request_total() == 4,
        format!("total={} summary={:?}", s.request_total(), s.recorded_summary()),
    );
    p.check(
        "paid-body-face-limit",
        s.requests_matching("POST", "/v3/generation/multiview-to-model")[0].json_body()["face_limit"]
            == 100000,
        "face_limit=100000".to_string(),
    );
    p.check(
        "no-script-problems-happy",
        s.script_problems().is_empty(),
        format!("{:?}", s.script_problems()),
    );

    // 8) 原始字节能力（服务器文档声称支持）：chunked 请求体、Expect: 100-continue。
    let s = test_support::presets::tripo_happy();
    let raw = format!(
        "POST /v3/files HTTP/1.1\r\nhost: {}\r\ntransfer-encoding: chunked\r\n\r\n\
         6\r\nhello \r\n5\r\nworld\r\n0\r\n\r\n",
        s.addr()
    );
    let out = raw_round_trip(s.addr(), raw.as_bytes());
    let recs = s.requests_matching("POST", "/v3/files");
    p.check(
        "chunked-body-recorded",
        out.contains("200") && recs.first().map(|r| r.body_text()) == Some("hello world".to_string()),
        format!(
            "resp_first_line={:?} body={:?}",
            out.lines().next().unwrap_or(""),
            recs.first().map(|r| r.body_text())
        ),
    );

    let s = test_support::presets::tripo_happy();
    let raw = format!(
        "POST /v3/files HTTP/1.1\r\nhost: {}\r\nexpect: 100-continue\r\ncontent-length: 5\r\n\r\nhello",
        s.addr()
    );
    let out = raw_round_trip(s.addr(), raw.as_bytes());
    let recs = s.requests_matching("POST", "/v3/files");
    p.check(
        "expect-100-continue",
        out.contains("100 Continue") && out.contains("200")
            && recs.first().map(|r| r.body_text()) == Some("hello".to_string()),
        format!("resp={out:?}"),
    );

    // 9) 全部已提交场景可解析。
    let scenarios_dir = fixtures_root().join("scenarios");
    let mut names: Vec<String> = std::fs::read_dir(&scenarios_dir)
        .expect("scenarios 目录")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    let mut all_non_empty = !names.is_empty();
    for name in &names {
        let sc = Scenario::load_file(&scenarios_dir.join(name), &fixtures_root());
        all_non_empty &= !sc.resolve().routes.is_empty();
    }
    p.check(
        "scenarios-parse",
        all_non_empty,
        format!("{names:?}"),
    );

    // 10) 样例资产：生成器确定性 + 与仓库字节一致 + 结构校验。
    let pinned: [(&str, &str); 5] = [
        ("sample-model.glb", "a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb"),
        ("sample-manual-text.pdf", "e18cf61a61c9f9834a73e7454c9f1c3a50e897908474f21edac4759fd3695cda"),
        ("sample-manual-scan.pdf", "4080019bb8f1d0b9db446add474721734d7dbf8f18e0f62bfa5c59c661db2ce7"),
        ("sample-photo-front.jpg", "122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586"),
        ("sample-photo-left.png", "0a74e5a5b542aa62440b48aee24838b664802bf5a98cd73fed45123cd7d2c890"),
    ];
    let generated = test_support::generate::build_all();
    let mut gen_ok = generated.len() == pinned.len();
    for ((name, bytes), (pname, phash)) in generated.iter().zip(pinned.iter()) {
        gen_ok &= name == pname && &test_support::sha256_hex(bytes) == phash;
    }
    p.check("assets-generator-deterministic", gen_ok, format!("count={}", generated.len()));
    let mut committed_ok = true;
    for (name, phash) in pinned {
        let bytes = std::fs::read(fixtures_root().join("assets").join(name)).expect("读取资产");
        committed_ok &= &test_support::sha256_hex(&bytes) == phash;
    }
    p.check("assets-committed-hash", committed_ok, "5 个资产与固定 sha256 一致".to_string());
    let glb = test_support::validate_glb(&std::fs::read(fixtures_root().join("assets/sample-model.glb")).unwrap()).unwrap();
    let text = test_support::validate_pdf(&std::fs::read(fixtures_root().join("assets/sample-manual-text.pdf")).unwrap()).unwrap();
    let scan = test_support::validate_pdf(&std::fs::read(fixtures_root().join("assets/sample-manual-scan.pdf")).unwrap()).unwrap();
    let jpeg = test_support::validate_jpeg(&std::fs::read(fixtures_root().join("assets/sample-photo-front.jpg")).unwrap()).unwrap();
    let png = test_support::validate_png(&std::fs::read(fixtures_root().join("assets/sample-photo-left.png")).unwrap()).unwrap();
    p.check(
        "assets-structure",
        glb.triangles == 12 && glb.image_count == 1 && glb.bin_len > 0
            && text.has_text_operators && text.image_xobjects == 0 && text.page_count == 2
            && !scan.has_text_operators && scan.image_xobjects == 2
            && (jpeg.width, jpeg.height) == (32, 32) && !jpeg.progressive
            && (png.width, png.height) == (64, 64),
        format!(
            "glb(tri={},img={}) text(pages={},text={}) scan(imgs={},text={}) jpeg={}x{} png={}x{}",
            glb.triangles, glb.image_count, text.page_count, text.has_text_operators,
            scan.image_xobjects, scan.has_text_operators, jpeg.width, jpeg.height, png.width, png.height
        ),
    );

    // 11) 响应样例文件带 _fixtureNote（"自建构造、非官方响应原文"）。
    let mut notes_ok = true;
    for entry in std::fs::read_dir(fixtures_root().join("responses")).expect("responses 目录") {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        notes_ok &= text.contains("_fixtureNote");
    }
    p.check("responses-fixture-note", notes_ok, "全部 responses/*.json 带 _fixtureNote".to_string());

    println!(
        "PROBE SUMMARY: {} checks, {} failures",
        p.checks, p.failures
    );
    std::process::exit(if p.failures == 0 { 0 } else { 1 });
}

fn raw_round_trip(addr: SocketAddr, raw: &[u8]) -> String {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).expect("连接 fixture");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("读超时");
    stream.write_all(raw).expect("写请求");
    stream.flush().expect("flush");
    let mut out = Vec::new();
    let mut buf = [0_u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
