//! T11 独立验收测试（QA 回合 12，2026-09-12）。
//!
//! 本文件**不复用** RD 的 `generation_requests.rs` 用例：断言目标、数据与期望值
//! 由 QA 独立选择与计算（金额按价格目录与页输入手工复算），用于交叉验证
//! AC-028/AC-029/AC-030/AC-031/AC-032/AC-033/AC-034（服务端侧）与卡内项
//! （快照不可变且无密钥、photo_ids+hashes、报价一次性消费、priceVersionChanged、
//! 幂等键作用域）。并发/重放/事务中断在本地现场执行。
//!
//! 现场性：临时 data-dir + 真实 SQLite；HTTP 走进程内 `oneshot`；
//! 假凭据 canary 字符串用于验证响应/审计不含密钥；无任何真实外网调用。

mod common;

use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::{TestApp, TestDir, TestResponse};
use everything_manual::config::{DEFAULT_MANUAL_AI_BASE_URL, ProviderSettings, SecretString};
use everything_manual::generation::catalog;
use everything_manual::generation::estimate as estimate_service;
use everything_manual::generation::jobs as jobs_service;
use everything_manual::generation::ledger as ledger_service;
use everything_manual::http::dto::{BudgetLimitsDto, EstimateRequest, JobCreateRequest};
use manual_core::timestamps::Timestamp;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tower::ServiceExt;

const PASSWORD: &str = "qa-t11-password-9f2c";
const TRIPO_MODEL: &str = "v3.1-20260211";
const MANUAL_AI_MODEL: &str = "gpt-5-mini";
const PRESET: &str = "tripo-h-v3.1-standard";
const TRIPO_KEY_CANARY: &str = "canary-tripo-key-do-not-leak";
const MANUAL_AI_KEY_CANARY: &str = "canary-manual-ai-key-do-not-leak";

/// QA 价格目录（30 credits；说明书 AI 单价与示例一致，便于手算复核）。
const CATALOG: &str = r#"
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

// ---------------------------------------------------------------------------
// 样例资产与 multipart 上传（独立实现）
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("读取样例资产失败：{error}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new(tag: &str) -> Self {
        Self {
            boundary: format!("----qa-t11-{tag}"),
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

// ---------------------------------------------------------------------------
// 测试应用与登录
// ---------------------------------------------------------------------------

/// QA 应用：默认 Provider 已配置（假凭据）+ 可选价格目录。
async fn qa_app(tag: &str, catalog_toml: Option<&str>) -> (TestApp, String, String) {
    let dir = TestDir::new(tag);
    let mut settings = common::test_settings(dir.path());
    settings.providers.tripo = common::configured_tripo(TRIPO_KEY_CANARY);
    settings.providers.manual_ai = ProviderSettings {
        name: "manual_ai",
        base_url: DEFAULT_MANUAL_AI_BASE_URL.to_owned(),
        model: Some(MANUAL_AI_MODEL.to_owned()),
        api_key: Some(SecretString::new(MANUAL_AI_KEY_CANARY)),
        key_source: Some("QA 测试注入".to_owned()),
    };
    if let Some(toml_text) = catalog_toml {
        settings.price_catalog_path = Some(dir.join("price-catalog.toml"));
        std::fs::write(dir.join("price-catalog.toml"), toml_text).expect("写入测试价格目录");
        settings.price_catalog = Some(catalog::parse(toml_text).expect("测试目录必须可解析"));
    }
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
        .unwrap()
        .to_owned();
    (app, login.session_cookie(), csrf)
}

fn pool(app: &TestApp) -> &SqlitePool {
    app.state().database().pool()
}

async fn count(app: &TestApp, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool(app)).await.unwrap()
}

async fn admin_id(app: &TestApp) -> String {
    sqlx::query_scalar("SELECT id FROM admins LIMIT 1")
        .fetch_one(pool(app))
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// 场景构造：物品 + PDF 准备（自选页输入）+ 多视图照片
// ---------------------------------------------------------------------------

struct Photo {
    view: String,
    photo_id: String,
    sha256: String,
    etag: String,
}

struct Inputs {
    item: String,
    preparation: String,
    photos: Vec<Photo>,
}

impl Inputs {
    fn photo_ids(&self) -> Vec<String> {
        self.photos.iter().map(|p| p.photo_id.clone()).collect()
    }

    fn photo(&self, view: &str) -> &Photo {
        self.photos
            .iter()
            .find(|p| p.view == view)
            .unwrap_or_else(|| panic!("缺少 {view} 照片"))
    }
}

async fn create_item(app: &TestApp, cookie: &str, csrf: &str, model: &str) -> String {
    let response = app
        .call(Method::POST, "/api/v1/items")
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "name": "QA 独立验收物品", "model": model }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()["data"]["id"].as_str().unwrap().to_owned()
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

fn view_fixture(view: &str) -> (&'static str, &'static str) {
    match view {
        "front" | "detail" => ("sample-photo-front.jpg", "image/jpeg"),
        _ => ("sample-photo-left.png", "image/png"),
    }
}

/// 构造完整输入：`pages[i] = Some(字节数)` 为文字页，`None` 为扫描页（无文字层）。
async fn prepare_world(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    model: &str,
    pages: &[Option<usize>],
    views: &[&str],
    complete: bool,
) -> Inputs {
    let item = create_item(app, cookie, csrf, model).await;

    let pdf = fixture("sample-manual-text.pdf");
    let doc_asset = upload_asset(
        app,
        cookie,
        csrf,
        &item,
        "document",
        "qa-manual.pdf",
        "application/pdf",
        &pdf,
    )
    .await;
    let document = app
        .call(Method::POST, &format!("/api/v1/items/{item}/documents"))
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "sourceAssetId": doc_asset, "title": "QA 样例说明书" }))
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

    for (index, text) in pages.iter().enumerate() {
        let page = index as i64 + 1;
        let image = upload_asset(
            app,
            cookie,
            csrf,
            &item,
            "pageImage",
            "page.jpg",
            "image/jpeg",
            &fixture("sample-photo-front.jpg"),
        )
        .await;
        let text_asset = match *text {
            Some(bytes) => Some(
                upload_asset(
                    app,
                    cookie,
                    csrf,
                    &item,
                    "pageText",
                    "page.txt",
                    "text/plain",
                    "q".repeat(bytes).as_bytes(),
                )
                .await,
            ),
            None => None,
        };
        let written = app
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
        assert_eq!(written.status, StatusCode::OK, "{}", written.text());
    }

    if complete {
        let current = app
            .call(Method::GET, &format!("/api/v1/preparations/{preparation}"))
            .cookie(cookie)
            .send()
            .await;
        let etag = current.header("etag").expect("准备详情必须带 ETag");
        let done = app
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
        assert_eq!(done.status, StatusCode::OK, "{}", done.text());
    }

    let mut photos = Vec::new();
    for view in views {
        let (name, content_type) = view_fixture(view);
        let bytes = fixture(name);
        let asset_id = upload_asset(
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
            .json(&json!({ "assetId": asset_id, "view": view }))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        photos.push(Photo {
            view: (*view).to_owned(),
            photo_id: response.json()["data"]["id"].as_str().unwrap().to_owned(),
            sha256: sha256_hex(&bytes),
            etag: response.header("etag").expect("照片创建响应必须带 ETag"),
        });
    }

    Inputs {
        item,
        preparation,
        photos,
    }
}

// ---------------------------------------------------------------------------
// 报价 / 确认 / 建单请求
// ---------------------------------------------------------------------------

async fn estimate(app: &TestApp, cookie: &str, csrf: &str, inputs: &Inputs) -> TestResponse {
    app.call(
        Method::POST,
        &format!("/api/v1/items/{}/estimates", inputs.item),
    )
    .cookie(cookie)
    .csrf(csrf)
    .json(&json!({
        "preparationId": inputs.preparation,
        "photoIds": inputs.photo_ids(),
        "modelPreset": PRESET,
    }))
    .send()
    .await
}

async fn estimate_ok(app: &TestApp, cookie: &str, csrf: &str, inputs: &Inputs) -> Value {
    let response = estimate(app, cookie, csrf, inputs).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()["data"].clone()
}

async fn confirm(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    quote_id: &str,
) -> TestResponse {
    app.call(
        Method::POST,
        &format!("/api/v1/items/{item}/estimates/{quote_id}/confirm"),
    )
    .cookie(cookie)
    .csrf(csrf)
    .send()
    .await
}

fn job_body(quote_id: &str, preparation: &str, photos: &[String], limits: (i64, i64)) -> Value {
    json!({
        "quoteId": quote_id,
        "preparationId": preparation,
        "photoIds": photos,
        "limits": {
            "tripoCreditMinor": limits.0,
            "manualAiUsdMicros": limits.1,
        },
    })
}

async fn submit(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    key: &str,
    body: &Value,
) -> TestResponse {
    app.call(Method::POST, &format!("/api/v1/items/{item}/jobs"))
        .cookie(cookie)
        .csrf(csrf)
        .header("idempotency-key", key)
        .json(body)
        .send()
        .await
}

// ---------------------------------------------------------------------------
// AC-028：报价分列金额（QA 独立手算）、价格版本、快照日期、expiresAt、无费用记录
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_ac028_quote_math_expiry_and_no_cost_records() {
    let (app, cookie, csrf) = qa_app("qa-ac028", Some(CATALOG)).await;
    // 页输入：1 页文字 3000 字节、2 页扫描（无文字层）、3 页文字 1500 字节（1 批）。
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(3000), None, Some(1500)],
        &["front", "left"],
        true,
    )
    .await;

    let response = estimate(&app, &cookie, &csrf, &inputs).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let data = response.json()["data"].clone();

    // ---- QA 独立复算（不使用实现里的常量，直接按目录单价与页输入手算）----
    // 4 个 Token 口径：输入上界 = 批开销 1200 + 页 1 文字 3000 + 页 2 页图 3000 + 页 3 文字 1500。
    let input_tokens_upper = 1200 + 3000 + 3000 + 1500; // = 8700
    let input_upper_micros = (input_tokens_upper as f64 * 0.25).ceil() as i64; // 2175
    let output_tokens_upper = 4096;
    let output_upper_micros = (output_tokens_upper as f64 * 2.00).ceil() as i64; // 8192
    let image_upper_micros = 10_000; // 1 张页图 × 0.01 USD
    let expected_manual_upper = input_upper_micros + output_upper_micros + image_upper_micros;
    assert_eq!(input_upper_micros, 2_175);
    assert_eq!(expected_manual_upper, 20_367);

    // Tripo：30 credits = 3000 creditMinor；预计 == 上界（固定价）。
    assert_eq!(data["amounts"]["tripo"]["currency"], "creditMinor");
    assert_eq!(data["amounts"]["tripo"]["estimatedMinor"], 3_000);
    assert_eq!(data["amounts"]["tripo"]["upperBoundMinor"], 3_000);
    assert_eq!(
        data["amounts"]["tripo"]["upperBoundDisplay"],
        "30.00 credits"
    );

    // Manual AI：USD 分列（不与 credits 相加）；上界 == QA 手算值。
    assert_eq!(data["amounts"]["manualAi"]["currency"], "usdMicros");
    assert_eq!(
        data["amounts"]["manualAi"]["upperBoundMinor"], expected_manual_upper,
        "Manual AI 上界必须等于 QA 独立手算"
    );
    assert_eq!(
        data["amounts"]["manualAi"]["upperBoundDisplay"],
        "0.020367 USD"
    );
    // 上界分项之和 == 上界（无隐藏加价/少算）。
    let lines = data["amounts"]["manualAi"]["upperBoundLines"]
        .as_array()
        .unwrap();
    let line_sum: i64 = lines
        .iter()
        .map(|line| line["amountMinor"].as_i64().unwrap())
        .sum();
    assert_eq!(line_sum, expected_manual_upper);
    let input_line = lines
        .iter()
        .find(|line| line["code"] == "inputTokens")
        .unwrap();
    assert_eq!(input_line["quantity"].as_i64().unwrap(), input_tokens_upper);
    assert_eq!(input_line["amountMinor"].as_i64().unwrap(), 2_175);
    let output_line = lines
        .iter()
        .find(|line| line["code"] == "outputTokens")
        .unwrap();
    assert_eq!(output_line["amountMinor"].as_i64().unwrap(), 8_192);
    let image_line = lines
        .iter()
        .find(|line| line["code"] == "pageImages")
        .unwrap();
    assert_eq!(image_line["quantity"].as_i64().unwrap(), 1);
    assert_eq!(image_line["amountMinor"].as_i64().unwrap(), 10_000);
    // 预计 <= 上界（保守口径）。
    assert!(
        data["amounts"]["manualAi"]["estimatedMinor"]
            .as_i64()
            .unwrap()
            <= expected_manual_upper
    );

    // 价格版本与快照日期、到期时间（QA 独立核对：createdAt + 600 秒 == expiresAt）。
    assert_eq!(data["priceVersion"], "2026-09-11");
    assert_eq!(data["priceSnapshotDate"], "2026-09-11");
    let created_at = Timestamp::from_rfc3339(data["createdAt"].as_str().unwrap()).unwrap();
    let expires_at = Timestamp::from_rfc3339(data["expiresAt"].as_str().unwrap()).unwrap();
    assert_eq!(
        expires_at.as_millis() - created_at.as_millis(),
        600_000,
        "报价默认有效期必须是 10 分钟"
    );

    // 发送范围（确认页数据齐全；本卡核对 API 字段）。
    assert_eq!(
        data["sendScope"]["tripo"]["views"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(data["sendScope"]["tripo"]["views"][0]["view"], "front");
    assert_eq!(
        data["sendScope"]["tripo"]["views"][0]["sha256"],
        inputs.photo("front").sha256
    );
    assert_eq!(data["sendScope"]["tripo"]["model"], TRIPO_MODEL);
    assert_eq!(data["sendScope"]["manualAi"]["pageFrom"], 1);
    assert_eq!(data["sendScope"]["manualAi"]["pageTo"], 3);
    assert_eq!(data["sendScope"]["manualAi"]["textPages"], json!([1, 3]));
    assert_eq!(data["sendScope"]["manualAi"]["imagePages"], json!([2]));
    assert_eq!(data["sendScope"]["manualAi"]["itemName"], "QA 独立验收物品");
    assert_eq!(data["sendScope"]["manualAi"]["itemModel"], "X100V");

    // 未确认/未消费（不存在默认勾选）。
    assert!(data["confirmedAt"].is_null());
    assert!(data["consumedAt"].is_null());

    // ---- 无生成调用、无费用记录 ----
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM provider_attempts").await,
        0
    );
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM generation_snapshots").await,
        0
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM audit_events").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM job_stages").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM quotes").await, 1);

    // ---- 响应与落库同源（服务端回读不依赖前端） + 无密钥 ----
    let stored: String = sqlx::query_scalar("SELECT quote_json FROM quotes")
        .fetch_one(pool(&app))
        .await
        .unwrap();
    let stored_value: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(stored_value, data, "落库载荷与响应必须同源");

    // GET 回读与 POST 响应一致（确认页刷新后不丢信息）。
    let got = app
        .call(
            Method::GET,
            &format!(
                "/api/v1/items/{}/estimates/{}",
                inputs.item,
                data["id"].as_str().unwrap()
            ),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text());
    assert_eq!(got.json()["data"], data);

    let text = response.text();
    for canary in [TRIPO_KEY_CANARY, MANUAL_AI_KEY_CANARY, "apiKey", "api_key"] {
        assert!(!text.contains(canary), "报价响应不得包含密钥线索 {canary}");
    }
    assert!(
        !stored.contains(TRIPO_KEY_CANARY) && !stored.contains(MANUAL_AI_KEY_CANARY),
        "落库报价快照不得包含密钥"
    );
}

// ---------------------------------------------------------------------------
// AC-029：缺价格 → 409；未 ready/缺视图 → 422 具体缺项；不返回伪造金额
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_ac029_missing_price_catalog_and_precondition_gaps() {
    // (a) 未配置价格目录 → 409 PRICE_CATALOG_MISSING（不返回任何金额）。
    let (app, cookie, csrf) = qa_app("qa-ac029-nocatalog", None).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(100)],
        &["front", "left"],
        true,
    )
    .await;
    let response = estimate(&app, &cookie, &csrf, &inputs).await;
    response.assert_contract_error(StatusCode::CONFLICT, "PRICE_CATALOG_MISSING");
    let missing = response.json()["error"]["details"]["missing"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(!missing.is_empty(), "409 必须说明缺项");
    assert!(
        !response.text().contains("upperBoundMinor"),
        "缺价格时不得返回伪造精确金额"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM quotes").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);

    // (b) preparation 未 ready + 缺侧视图：422 一次列出具体缺项。
    let (app2, c2, s2) = qa_app("qa-ac029-notready", Some(CATALOG)).await;
    let inputs2 = prepare_world(&app2, &c2, &s2, "X100V", &[Some(100)], &["front"], false).await;
    let response2 = estimate(&app2, &c2, &s2, &inputs2).await;
    assert_eq!(response2.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body2 = response2.json();
    assert_eq!(
        body2["error"]["details"]["reason"],
        "preconditionsFailed",
        "{}",
        response2.text()
    );
    let codes: Vec<String> = body2["error"]["details"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["code"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        codes.contains(&"preparationNotReady".to_owned()),
        "必须列出准备未完成：{codes:?}"
    );
    assert!(
        codes.contains(&"missingSideView".to_owned()),
        "必须列出缺侧视图：{codes:?}"
    );
    assert!(body2["error"]["details"]["presentViews"].is_array());
    assert!(
        !response2.text().contains("upperBoundMinor"),
        "缺项时不得返回伪造金额"
    );
    assert_eq!(count(&app2, "SELECT COUNT(*) FROM quotes").await, 0);

    // (c) 只有 left（缺 front）→ missingFrontView；detail 进入多视图 → detailViewNotAllowed。
    let (app3, c3, s3) = qa_app("qa-ac029-front-detail", Some(CATALOG)).await;
    let inputs3 = prepare_world(&app3, &c3, &s3, "X100V", &[Some(100)], &["left"], true).await;
    let response3 = estimate(&app3, &c3, &s3, &inputs3).await;
    assert_eq!(response3.status, StatusCode::UNPROCESSABLE_ENTITY);
    let codes3: Vec<String> = response3.json()["error"]["details"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["code"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        codes3.contains(&"missingFrontView".to_owned()),
        "{codes3:?}"
    );

    let (app4, c4, s4) = qa_app("qa-ac029-detail", Some(CATALOG)).await;
    let inputs4 = prepare_world(
        &app4,
        &c4,
        &s4,
        "X100V",
        &[Some(100)],
        &["front", "left", "detail"],
        true,
    )
    .await;
    let response4 = estimate(&app4, &c4, &s4, &inputs4).await;
    assert_eq!(response4.status, StatusCode::UNPROCESSABLE_ENTITY);
    let codes4: Vec<String> = response4.json()["error"]["details"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["code"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        codes4.contains(&"detailViewNotAllowed".to_owned()),
        "{codes4:?}"
    );
    assert_eq!(count(&app4, "SELECT COUNT(*) FROM quotes").await, 0);
}

#[tokio::test]
async fn qa_ac029_provider_missing_and_unsupported_preset() {
    // (a) Provider 未配置（manual_ai 无密钥）→ 409 PROVIDER_NOT_CONFIGURED。
    let dir = TestDir::new("qa-ac029-noprovider");
    let mut settings = common::test_settings(dir.path());
    settings.providers.tripo = common::configured_tripo(TRIPO_KEY_CANARY);
    // manual_ai 保持未配置（无密钥、无模型）。
    settings.price_catalog_path = Some(dir.join("price-catalog.toml"));
    std::fs::write(dir.join("price-catalog.toml"), CATALOG).unwrap();
    settings.price_catalog = Some(catalog::parse(CATALOG).unwrap());
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;
    let login = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&json!({ "password": PASSWORD }))
        .send()
        .await;
    let cookie = login.session_cookie();
    let csrf = login.json()["data"]["csrfToken"]
        .as_str()
        .unwrap()
        .to_owned();
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(100)],
        &["front", "left"],
        true,
    )
    .await;
    let response = estimate(&app, &cookie, &csrf, &inputs).await;
    response.assert_contract_error(StatusCode::CONFLICT, "PROVIDER_NOT_CONFIGURED");
    let missing = response.json()["error"]["details"]["missing"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        missing
            .iter()
            .any(|item| item.as_str().unwrap_or_default().starts_with("manual_ai.")),
        "缺项必须指向 manual_ai.*：{missing:?}"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM quotes").await, 0);

    // (b) 目录里没有该预设 → 422 modelPresetUnsupported（不进入支持清单）。
    let (app2, c2, s2) = qa_app("qa-ac029-preset", Some(CATALOG)).await;
    let inputs2 = prepare_world(
        &app2,
        &c2,
        &s2,
        "X100V",
        &[Some(100)],
        &["front", "left"],
        true,
    )
    .await;
    let response2 = app2
        .call(
            Method::POST,
            &format!("/api/v1/items/{}/estimates", inputs2.item),
        )
        .cookie(&c2)
        .csrf(&s2)
        .json(&json!({
            "preparationId": inputs2.preparation,
            "photoIds": inputs2.photo_ids(),
            "modelPreset": "tripo-h-v9-ultra",
        }))
        .send()
        .await;
    assert_eq!(response2.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response2.json()["error"]["details"]["reason"],
        "modelPresetUnsupported"
    );
    let supported = response2.json()["error"]["details"]["supportedPresets"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(supported, vec![json!(PRESET)]);
    assert_eq!(count(&app2, "SELECT COUNT(*) FROM quotes").await, 0);

    // (c) 目录价格与配置模型不一致 → 409 PRICE_CATALOG_MISSING（不让配置各说各话）。
    let dir3 = TestDir::new("qa-ac029-model-mismatch");
    let mut settings3 = common::test_settings(dir3.path());
    settings3.providers.tripo = ProviderSettings {
        name: "tripo",
        base_url: everything_manual::config::DEFAULT_TRIPO_BASE_URL.to_owned(),
        model: Some("v3.0-mismatch".to_owned()),
        api_key: Some(SecretString::new(TRIPO_KEY_CANARY)),
        key_source: Some("QA 测试注入".to_owned()),
    };
    settings3.providers.manual_ai = ProviderSettings {
        name: "manual_ai",
        base_url: DEFAULT_MANUAL_AI_BASE_URL.to_owned(),
        model: Some(MANUAL_AI_MODEL.to_owned()),
        api_key: Some(SecretString::new(MANUAL_AI_KEY_CANARY)),
        key_source: Some("QA 测试注入".to_owned()),
    };
    settings3.price_catalog_path = Some(dir3.join("price-catalog.toml"));
    std::fs::write(dir3.join("price-catalog.toml"), CATALOG).unwrap();
    settings3.price_catalog = Some(catalog::parse(CATALOG).unwrap());
    let app3 = TestApp::with_settings(dir3, settings3).await;
    app3.set_admin_password(PASSWORD).await;
    let login3 = app3
        .call(Method::POST, "/api/v1/auth/login")
        .json(&json!({ "password": PASSWORD }))
        .send()
        .await;
    let cookie3 = login3.session_cookie();
    let csrf3 = login3.json()["data"]["csrfToken"]
        .as_str()
        .unwrap()
        .to_owned();
    let inputs3 = prepare_world(
        &app3,
        &cookie3,
        &csrf3,
        "X100V",
        &[Some(100)],
        &["front", "left"],
        true,
    )
    .await;
    let response3 = estimate(&app3, &cookie3, &csrf3, &inputs3).await;
    response3.assert_contract_error(StatusCode::CONFLICT, "PRICE_CATALOG_MISSING");
    let missing3 = response3.json()["error"]["details"]["missing"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        missing3.iter().any(|item| item
            .as_str()
            .unwrap_or_default()
            .contains("tripoModelMismatch")),
        "{missing3:?}"
    );
}

// ---------------------------------------------------------------------------
// AC-030：未确认被拒、确认写审计（恰一次）、不默认勾选、确认页数据齐全
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_ac030_unconfirmed_rejected_and_confirmation_audited_once() {
    let (app, cookie, csrf) = qa_app("qa-ac030", Some(CATALOG)).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(3000), None, Some(1500)],
        &["front", "left"],
        true,
    )
    .await;
    let quote = estimate_ok(&app, &cookie, &csrf, &inputs).await;
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    let admin = admin_id(&app).await;

    // 未确认提交 → 422 confirmationRequired；不创建 job、不写费用。
    let body = job_body(
        &quote_id,
        &inputs.preparation,
        &inputs.photo_ids(),
        (3_000, 20_367),
    );
    let denied = submit(&app, &cookie, &csrf, &inputs.item, "qa-ac030-key", &body).await;
    assert_eq!(denied.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        denied.json()["error"]["details"]["reason"],
        "confirmationRequired",
        "{}",
        denied.text()
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM provider_attempts").await,
        0
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM audit_events").await, 0);

    // 显式确认 → 200；确认记录含发送范围（视图/页范围/模型/价格版本/上界）。
    let confirmed = confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.text());
    let confirmation = confirmed.json()["data"].clone();
    assert_eq!(confirmation["quoteId"], quote_id);
    assert!(confirmation["confirmedAt"].is_string());
    assert_eq!(
        confirmation["sendScope"]["tripo"]["views"][0]["view"],
        "front"
    );
    assert_eq!(
        confirmation["sendScope"]["tripo"]["views"][1]["photoId"],
        inputs.photo("left").photo_id
    );
    assert_eq!(confirmation["sendScope"]["manualAi"]["pageFrom"], 1);
    assert_eq!(confirmation["sendScope"]["manualAi"]["pageTo"], 3);
    assert_eq!(confirmation["sendScope"]["manualAi"]["itemModel"], "X100V");
    assert_eq!(confirmation["sendScope"]["priceVersion"], "2026-09-11");
    assert_eq!(
        confirmation["sendScope"]["plannedUpperBound"]["tripo"]["upperBoundMinor"],
        3_000
    );

    // audit_events：恰 1 条，actor 为当前管理员，metadata 含摘要且无密钥。
    let (action, entity_type, actor): (String, String, String) =
        sqlx::query_as("SELECT action, entity_type, actor FROM audit_events")
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert_eq!(action, "generation_send_scope_confirmed");
    assert_eq!(entity_type, "quote");
    assert_eq!(actor, admin);
    let metadata: String =
        sqlx::query_scalar("SELECT metadata_json FROM audit_events WHERE action = ?")
            .bind("generation_send_scope_confirmed")
            .fetch_one(pool(&app))
            .await
            .unwrap();
    let meta: Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(meta["priceVersion"], "2026-09-11");
    assert_eq!(meta["upperBound"]["tripoCreditMinor"], 3_000);
    assert_eq!(meta["tripoViews"].as_array().unwrap().len(), 2);
    assert_eq!(meta["itemModel"], "X100V");
    assert!(!metadata.contains(TRIPO_KEY_CANARY) && !metadata.contains(MANUAL_AI_KEY_CANARY));

    // 重复确认幂等：时间不变、审计仍 1 条。
    let again = confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text());
    assert_eq!(
        again.json()["data"]["confirmedAt"],
        confirmation["confirmedAt"]
    );
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM audit_events WHERE action = 'generation_send_scope_confirmed'"
        )
        .await,
        1
    );

    // GET 回读：报价载荷（金额/发送范围/到期时间）与 DB 事实一致；
    // 注意：确认/消费状态字段的**回读陈旧问题**见下方 `qa_defect_...`（BUG-004）。
    let got = app
        .call(
            Method::GET,
            &format!("/api/v1/items/{}/estimates/{quote_id}", inputs.item),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(got.status, StatusCode::OK);
    assert_eq!(got.json()["data"]["amounts"], quote["amounts"]);
    assert_eq!(got.json()["data"]["expiresAt"], quote["expiresAt"]);
    assert_eq!(got.json()["data"]["sendScope"], confirmation["sendScope"]);
    let db_confirmed: Option<i64> =
        sqlx::query_scalar("SELECT confirmed_at FROM quotes WHERE id = ?")
            .bind(&quote_id)
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert!(db_confirmed.is_some(), "确认必须落库（DB 事实）");
}

/// BUG-004 复现（P2，QA 回合 12 判定）：`GET /items/{id}/estimates/{quoteId}`
/// 永远返回**建单时刻冻结的载荷**（confirmedAt/consumedAt/consumedJobId 恒为 null），
/// 与 handler/OpenAPI 自己声明的"含到期时间与确认/消费状态"及 DB 事实不符。
/// 待 RD 修复后去掉 `#[ignore]` 转正。
///
/// **QA 回合 29（T21）转正**：BUG-004 已在回合 13 由 RD 修复并 CLOSED，本用例在当前
/// 实现下实际通过（回合 29 实测 `--ignored` 运行 ok），断言逐字未改，只去掉 ignore——
/// 避免"缺陷已关闭但回归用例仍被静默跳过"。
#[tokio::test]
async fn qa_defect_get_estimate_reflects_confirmation_and_consumption() {
    let (app, cookie, csrf) = qa_app("qa-defect-get", Some(CATALOG)).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(3000), None, Some(1500)],
        &["front", "left"],
        true,
    )
    .await;
    let quote = estimate_ok(&app, &cookie, &csrf, &inputs).await;
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    let confirmed = confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    let first_confirmed_at = confirmed.json()["data"]["confirmedAt"].clone();

    let after_confirm = app
        .call(
            Method::GET,
            &format!("/api/v1/items/{}/estimates/{quote_id}", inputs.item),
        )
        .cookie(&cookie)
        .send()
        .await;
    println!(
        "AFTER_CONFIRM: GET confirmedAt={} （确认响应={first_confirmed_at}）",
        after_confirm.json()["data"]["confirmedAt"]
    );

    let accepted = submit(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "qa-defect-get-key",
        &job_body(
            &quote_id,
            &inputs.preparation,
            &inputs.photo_ids(),
            (3_000, 20_367),
        ),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED);
    let job_id = accepted.json()["data"]["id"].as_str().unwrap().to_owned();

    let after_job = app
        .call(
            Method::GET,
            &format!("/api/v1/items/{}/estimates/{quote_id}", inputs.item),
        )
        .cookie(&cookie)
        .send()
        .await;
    println!(
        "AFTER_JOB: GET consumedAt={} consumedJobId={} （真实 job={job_id}）",
        after_job.json()["data"]["consumedAt"],
        after_job.json()["data"]["consumedJobId"]
    );
    assert_eq!(
        after_confirm.json()["data"]["confirmedAt"],
        first_confirmed_at,
        "确认后 GET 必须反映已确认（合同声明：含确认/消费状态）"
    );
    assert!(
        after_job.json()["data"]["consumedAt"].is_string(),
        "消费后 GET 必须反映 consumedAt"
    );
    assert_eq!(
        after_job.json()["data"]["consumedJobId"],
        job_id,
        "消费后 GET 必须给出 consumedJobId（UI-026 链接已有任务）"
    );
}

// ---------------------------------------------------------------------------
// AC-031：同键 20 次重放只建 1 job、同键不同 body 409、键作用域、报价一次性消费
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_ac031_twenty_replays_conflict_and_key_scope() {
    let (app, cookie, csrf) = qa_app("qa-ac031", Some(CATALOG)).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(3000), None, Some(1500)],
        &["front", "left"],
        true,
    )
    .await;
    let quote = estimate_ok(&app, &cookie, &csrf, &inputs).await;
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;

    let body = job_body(
        &quote_id,
        &inputs.preparation,
        &inputs.photo_ids(),
        (3_000, 20_367),
    );

    // 同一 Idempotency-Key 连续提交 20 次（QA 现场执行）。
    let mut job_ids = Vec::new();
    let mut first_reservations = Value::Null;
    for round in 1..=20 {
        let response = submit(&app, &cookie, &csrf, &inputs.item, "qa-ac031-replay", &body).await;
        assert_eq!(
            response.status,
            StatusCode::ACCEPTED,
            "第 {round} 次：{}",
            response.text()
        );
        if round == 1 {
            assert!(
                response.header("x-idempotent-replay").is_none(),
                "首次提交不是重放"
            );
        } else {
            assert_eq!(
                response.header("x-idempotent-replay").as_deref(),
                Some("true"),
                "第 {round} 次必须是重放"
            );
        }
        let payload = response.json()["data"].clone();
        // 预留按 provider 排序比较：重放路径按 provider 排序返回，首建按写入顺序返回
        // （数组顺序不是合同承诺；内容必须一致——不重复预留、金额相同）。
        let sorted = |value: &Value| -> Vec<(String, i64, String)> {
            let mut rows: Vec<(String, i64, String)> = value
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| {
                    (
                        entry["provider"].as_str().unwrap().to_owned(),
                        entry["reservedMinor"].as_i64().unwrap(),
                        entry["state"].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            rows.sort();
            rows
        };
        if round == 1 {
            first_reservations = payload["reservations"].clone();
        } else {
            assert_eq!(
                sorted(&payload["reservations"]),
                sorted(&first_reservations),
                "重放必须返回同一份预留（不重复预留、金额相同）"
            );
        }
        job_ids.push(payload["id"].as_str().unwrap().to_owned());
    }
    assert!(
        job_ids.iter().all(|id| id == &job_ids[0]),
        "20 次重放必须返回同一 job id"
    );

    // 只存在 1 个 job / 1 份快照 / 每供应商 1 笔预留 / 1 条幂等记录 / 0 attempt。
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM generation_snapshots").await,
        1
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 2);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM idempotency_records").await,
        1
    );
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM provider_attempts").await,
        0
    );
    assert!(
        count(&app, "SELECT COUNT(*) FROM job_stages").await > 0,
        "建单必须生成阶段 DAG"
    );
    // 重放不重复审计：确认 + 建单各 1 条，20 次重放后不再增长。
    let audit_actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_events ORDER BY created_at, action")
            .fetch_all(pool(&app))
            .await
            .unwrap();
    assert_eq!(
        audit_actions,
        vec![
            "generation_send_scope_confirmed".to_owned(),
            "generation_job_created".to_owned(),
        ]
    );

    // 预留金额 = 服务端计算的保守上界（不是请求里的数字）。
    let mut reservations: Vec<(String, i64, String)> =
        sqlx::query_as("SELECT provider, reserved, state FROM cost_ledger ORDER BY provider")
            .fetch_all(pool(&app))
            .await
            .unwrap();
    reservations.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        reservations,
        vec![
            ("manual_ai".to_owned(), 20_367, "reserved".to_owned()),
            ("tripo".to_owned(), 3_000, "reserved".to_owned()),
        ]
    );

    // 报价一次性消费：consumed_at/consumed_job_id 已写。
    let (consumed_at, consumed_job): (Option<i64>, Option<String>) =
        sqlx::query_as("SELECT consumed_at, consumed_job_id FROM quotes")
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert!(consumed_at.is_some());
    assert_eq!(consumed_job.as_deref(), Some(job_ids[0].as_str()));

    // 同键不同 body → 409 IDEMPOTENCY_CONFLICT（不新建、不重复预留）。
    let other_body = job_body(
        &quote_id,
        &inputs.preparation,
        &inputs.photo_ids(),
        (9_999, 999_999),
    );
    let conflict = submit(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "qa-ac031-replay",
        &other_body,
    )
    .await;
    conflict.assert_contract_error(StatusCode::CONFLICT, "IDEMPOTENCY_CONFLICT");
    assert_eq!(
        conflict.json()["error"]["details"]["reason"],
        "idempotencyKeyReused"
    );
    assert_eq!(
        conflict.json()["error"]["details"]["existingResourceId"],
        job_ids[0]
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 2);

    // 换新键提交同一报价 → 422 quoteAlreadyUsed + jobId（一份报价一份任务）。
    let new_key = submit(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "qa-ac031-new-key",
        &body,
    )
    .await;
    assert_eq!(new_key.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        new_key.json()["error"]["details"]["reason"],
        "quoteAlreadyUsed"
    );
    assert_eq!(new_key.json()["error"]["details"]["jobId"], job_ids[0]);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);

    // 幂等键作用域 = admin + POST + 路由模板 + key（不是每物品独立命名空间）。
    let (record_admin, method, route, key, body_hash, resource, status): (
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
    ) = sqlx::query_as(
        "SELECT admin_id, method, route, key, body_hash, resource_id, response_status \
           FROM idempotency_records",
    )
    .fetch_one(pool(&app))
    .await
    .unwrap();
    assert_eq!(record_admin, admin_id(&app).await);
    assert_eq!(method, "POST");
    assert_eq!(route, "/api/v1/items/{id}/jobs");
    assert_eq!(key, "qa-ac031-replay");
    assert_eq!(body_hash.len(), 64);
    assert_eq!(resource.as_deref(), Some(job_ids[0].as_str()));
    assert_eq!(status, 202);

    // 跨物品复用同一 key（body 不同）→ 409：键不按物品隔离。
    let inputs2 = prepare_world(
        &app,
        &cookie,
        &csrf,
        "QA-2",
        &[Some(100)],
        &["front", "left"],
        true,
    )
    .await;
    let quote2 = estimate_ok(&app, &cookie, &csrf, &inputs2).await;
    let quote2_id = quote2["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs2.item, &quote2_id).await;
    let body2 = job_body(
        &quote2_id,
        &inputs2.preparation,
        &inputs2.photo_ids(),
        (3_000, 20_367),
    );
    let cross = submit(
        &app,
        &cookie,
        &csrf,
        &inputs2.item,
        "qa-ac031-replay",
        &body2,
    )
    .await;
    cross.assert_contract_error(StatusCode::CONFLICT, "IDEMPOTENCY_CONFLICT");
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM idempotency_records").await,
        1
    );
}

// ---------------------------------------------------------------------------
// AC-031：并发同键 10 路 / 同一报价不同键 6 路（现场执行）
// ---------------------------------------------------------------------------

fn job_request(cookie: &str, csrf: &str, item: &str, key: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(format!("/api/v1/items/{item}/jobs"))
        .header("host", "127.0.0.1:8080")
        .header("cookie", cookie)
        .header("x-csrf-token", csrf)
        .header("idempotency-key", key)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .expect("构造请求")
}

#[tokio::test]
async fn qa_ac031_concurrent_same_key_and_same_quote_race() {
    let (app, cookie, csrf) = qa_app("qa-ac031-concurrent", Some(CATALOG)).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(3000), None, Some(1500)],
        &["front", "left"],
        true,
    )
    .await;
    let quote = estimate_ok(&app, &cookie, &csrf, &inputs).await;
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    let body = job_body(
        &quote_id,
        &inputs.preparation,
        &inputs.photo_ids(),
        (3_000, 20_367),
    );

    // (1) 10 路并发、同一 key + 同一 body：全部应回同一 job，库里只有 1 个 job。
    let router: Router = app.router_handle();
    let mut handles = Vec::new();
    for _ in 0..10 {
        let router = router.clone();
        let request = job_request(&cookie, &csrf, &inputs.item, "qa-ac031-parallel", &body);
        handles.push(tokio::spawn(async move { router.oneshot(request).await }));
    }
    let mut statuses = Vec::new();
    let mut ids = Vec::new();
    for handle in handles {
        let response = tokio::time::timeout(Duration::from_secs(30), handle)
            .await
            .expect("并发请求超时")
            .expect("并发任务 panic")
            .expect("router 处理失败");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("读取响应体");
        statuses.push(status);
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if let Some(id) = value["data"]["id"].as_str() {
            ids.push(id.to_owned());
        }
    }
    assert!(
        statuses
            .iter()
            .all(|status| *status == StatusCode::ACCEPTED),
        "并发同键应全部 202：{statuses:?}"
    );
    assert!(
        ids.iter().all(|id| *id == ids[0]) && ids.len() == 10,
        "并发同键必须全部返回同一 job：{ids:?}"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 2);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM idempotency_records").await,
        1
    );

    // (2) 同一报价、6 路并发、各用不同 key：只有 1 个成功（其余 quoteAlreadyUsed）。
    let inputs2 = prepare_world(
        &app,
        &cookie,
        &csrf,
        "QA-race",
        &[Some(100)],
        &["front", "left"],
        true,
    )
    .await;
    let quote2 = estimate_ok(&app, &cookie, &csrf, &inputs2).await;
    let quote2_id = quote2["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs2.item, &quote2_id).await;
    let body2 = job_body(
        &quote2_id,
        &inputs2.preparation,
        &inputs2.photo_ids(),
        (3_000, 20_367),
    );
    let mut handles2 = Vec::new();
    for index in 0..6 {
        let router = router.clone();
        let request = job_request(
            &cookie,
            &csrf,
            &inputs2.item,
            &format!("qa-ac031-race-{index}"),
            &body2,
        );
        handles2.push(tokio::spawn(async move { router.oneshot(request).await }));
    }
    let mut accepted = 0;
    for handle in handles2 {
        let response = tokio::time::timeout(Duration::from_secs(30), handle)
            .await
            .expect("并发请求超时")
            .expect("并发任务 panic")
            .expect("router 处理失败");
        if response.status() == StatusCode::ACCEPTED {
            accepted += 1;
        } else {
            assert_eq!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "并发竞争只允许唯一成功"
            );
        }
    }
    assert_eq!(accepted, 1, "同一报价只允许建一个 job");
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 2);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 4);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM provider_attempts").await,
        0
    );
}

// ---------------------------------------------------------------------------
// AC-031：建单事务中断不留半笔预留（断点现场注入）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_ac031_interrupted_transaction_leaves_no_partial_state() {
    use everything_manual::jobs::failpoints::{self, FailpointAction, GENERATION_BEFORE_COMMIT};

    let (app, cookie, csrf) = qa_app("qa-ac031-tx", Some(CATALOG)).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(3000), None, Some(1500)],
        &["front", "left"],
        true,
    )
    .await;
    let quote = estimate_ok(&app, &cookie, &csrf, &inputs).await;
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    let body = job_body(
        &quote_id,
        &inputs.preparation,
        &inputs.photo_ids(),
        (3_000, 20_367),
    );

    let key = "qa-ac031-tx-key";
    failpoints::set(key, GENERATION_BEFORE_COMMIT, FailpointAction::Panic);
    let router = app.router_handle();
    let request = job_request(&cookie, &csrf, &inputs.item, key, &body);
    let join = tokio::task::spawn(async move { router.oneshot(request).await });
    let outcome = tokio::time::timeout(Duration::from_secs(10), join)
        .await
        .expect("断点请求应在超时前结束");
    assert!(outcome.is_err(), "断点必须中断请求（panic）");
    failpoints::clear_owner(key);

    // 全部写入必须一起回滚：job/快照/预留/阶段/幂等记录都不留，报价也未被消费。
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM generation_snapshots").await,
        0
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM job_stages").await, 0);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM idempotency_records").await,
        0
    );
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM quotes WHERE consumed_at IS NOT NULL"
        )
        .await,
        0
    );

    // 清除断点后同键同 body 重试成功（键可复用，且这次是真实首建）。
    let retry = submit(&app, &cookie, &csrf, &inputs.item, key, &body).await;
    assert_eq!(retry.status, StatusCode::ACCEPTED, "{}", retry.text());
    assert!(retry.header("x-idempotent-replay").is_none());
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 2);
}

// ---------------------------------------------------------------------------
// AC-032：过期报价 / 输入已变 / 价格版本变化 / 预算不足 / 前端费用不被采信
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_ac032_expired_quote_and_input_change_are_rejected_without_jobs() {
    let (app, cookie, csrf) = qa_app("qa-ac032-a", Some(CATALOG)).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(3000), None, Some(1500)],
        &["front", "left"],
        true,
    )
    .await;

    // (1) 过期报价：以 700 秒前的时刻生成报价（HTTP 用当前时间提交时必然过期）。
    let past = Timestamp::now().checked_add_millis(-700_000).unwrap();
    let mut connection = pool(&app).acquire().await.unwrap();
    let expired = estimate_service::create_estimate(
        app.state().settings(),
        &mut connection,
        &inputs.item,
        &EstimateRequest {
            preparation_id: Some(inputs.preparation.clone()),
            photo_ids: Some(inputs.photo_ids()),
            model_preset: Some(PRESET.to_owned()),
        },
        past,
    )
    .await
    .expect("过期报价仍应能创建（供本用例验证提交侧拒绝）");
    // 报价创建本身不写费用记录。
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);
    let body = job_body(
        &expired.id,
        &inputs.preparation,
        &inputs.photo_ids(),
        (3_000, 20_367),
    );
    let denied = submit(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "qa-ac032-expired",
        &body,
    )
    .await;
    assert_eq!(denied.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(denied.json()["error"]["details"]["reason"], "quoteExpired");
    assert!(denied.json()["error"]["details"]["expiresAt"].is_string());
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM provider_attempts").await,
        0
    );

    // 过期报价也不允许确认（确认了也提交不了，提前拒绝）。
    let confirm_expired = confirm(&app, &cookie, &csrf, &inputs.item, &expired.id).await;
    assert_eq!(confirm_expired.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        confirm_expired.json()["error"]["details"]["reason"],
        "quoteExpired"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM audit_events").await, 0);

    // (2) 输入已变（报价后替换照片资产，photo id 不变、内容变化）→ 422 inputChanged。
    let quote = estimate_ok(&app, &cookie, &csrf, &inputs).await;
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    let replacement = upload_asset(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "photo",
        "sample-photo-left.png",
        "image/png",
        &fixture("sample-photo-left.png"),
    )
    .await;
    let front = inputs.photo("front");
    let patched = app
        .call(
            Method::PATCH,
            &format!("/api/v1/items/{}/photos/{}", inputs.item, front.photo_id),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &front.etag)
        .json(&json!({ "assetId": replacement }))
        .send()
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text());
    let body2 = job_body(
        &quote_id,
        &inputs.preparation,
        &inputs.photo_ids(),
        (3_000, 20_367),
    );
    let changed = submit(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "qa-ac032-changed",
        &body2,
    )
    .await;
    assert_eq!(changed.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(changed.json()["error"]["details"]["reason"], "inputChanged");
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM provider_attempts").await,
        0
    );
}

#[tokio::test]
async fn qa_ac032_price_version_budget_and_frontend_fees() {
    let (app, cookie, csrf) = qa_app("qa-ac032-b", Some(CATALOG)).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(3000), None, Some(1500)],
        &["front", "left"],
        true,
    )
    .await;
    let quote = estimate_ok(&app, &cookie, &csrf, &inputs).await;
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    let body = job_body(
        &quote_id,
        &inputs.preparation,
        &inputs.photo_ids(),
        (3_000, 20_367),
    );

    // (1) 允许上限低于服务端上界 → 422 budgetBelowPlannedUpperBound（给两侧数值），不创建 job。
    let low = job_body(
        &quote_id,
        &inputs.preparation,
        &inputs.photo_ids(),
        (2_999, 20_367),
    );
    let denied = submit(&app, &cookie, &csrf, &inputs.item, "qa-ac032-low", &low).await;
    assert_eq!(denied.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        denied.json()["error"]["details"]["reason"],
        "budgetBelowPlannedUpperBound"
    );
    assert_eq!(
        denied.json()["error"]["details"]["tripoUpperBoundCreditMinor"],
        3_000
    );
    assert_eq!(
        denied.json()["error"]["details"]["manualAiUpperBoundUsdMicros"],
        20_367
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);

    // (2) 请求体出现前端费用字段 → 422（未知字段；服务端结构上不采信前端费用）。
    let mut smuggled = body.clone();
    smuggled["tripoCreditMinor"] = json!(1);
    let rejected = submit(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "qa-ac032-smuggle",
        &smuggled,
    )
    .await;
    assert_eq!(
        rejected.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        rejected.text()
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);

    // (3) 价格版本已变（模拟运营者改版但旧报价未重报）→ 422 priceVersionChanged。
    let mut repriced = app.state().settings().clone();
    repriced.price_catalog.as_mut().unwrap().version = "2026-10-01".to_owned();
    let admin = admin_id(&app).await;
    let mut connection = pool(&app).acquire().await.unwrap();
    let error = jobs_service::create_job(
        &repriced,
        &mut connection,
        &inputs.item,
        &JobCreateRequest {
            quote_id: Some(quote_id.clone()),
            preparation_id: Some(inputs.preparation.clone()),
            photo_ids: Some(inputs.photo_ids()),
            limits: Some(BudgetLimitsDto {
                tripo_credit_minor: Some(3_000),
                manual_ai_usd_micros: Some(20_367),
            }),
        },
        "qa-ac032-version",
        &admin,
        Timestamp::now(),
    )
    .await
    .expect_err("价格版本变化必须被拒");
    match error {
        everything_manual::generation::GenerationError::Unprocessable { reason, .. } => {
            assert_eq!(reason, "priceVersionChanged");
        }
        other => panic!("期望 priceVersionChanged，实际 {other:?}"),
    }
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);

    // (4) 宽松上限（远大于上界）→ 202；但预留仍 = 服务端上界，不是用户给的数字。
    let generous = job_body(
        &quote_id,
        &inputs.preparation,
        &inputs.photo_ids(),
        (99_999, 9_999_999),
    );
    let accepted = submit(&app, &cookie, &csrf, &inputs.item, "qa-ac032-ok", &generous).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.text());
    let reservations = accepted.json()["data"]["reservations"].clone();
    let tripo = reservations
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["provider"] == "tripo")
        .unwrap()
        .clone();
    let manual = reservations
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["provider"] == "manual_ai")
        .unwrap()
        .clone();
    assert_eq!(tripo["currency"], "creditMinor");
    assert_eq!(tripo["reservedMinor"], 3_000);
    assert_eq!(tripo["state"], "reserved");
    assert_eq!(manual["currency"], "usdMicros");
    assert_eq!(manual["reservedMinor"], 20_367);
    assert!(
        accepted.json()["data"]["budgetNotice"]
            .as_str()
            .unwrap()
            .contains("不是供应商账户级硬封顶")
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
    let reserved_in_db: i64 =
        sqlx::query_scalar("SELECT reserved FROM cost_ledger WHERE provider = 'tripo'")
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert_eq!(reserved_in_db, 3_000, "账本预留必须是服务端上界");
}

// ---------------------------------------------------------------------------
// AC-033 / AC-034（服务端侧）：unknown 保留预留（不填 0）、释放路径分离、不自动降质量
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_ac033_034_unknown_keeps_reservation_and_release_paths_are_split() {
    let (app, cookie, csrf) = qa_app("qa-ac033", Some(CATALOG)).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(3000), None, Some(1500)],
        &["front", "left"],
        true,
    )
    .await;
    let quote = estimate_ok(&app, &cookie, &csrf, &inputs).await;
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    let accepted = submit(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "qa-ac033-key",
        &job_body(
            &quote_id,
            &inputs.preparation,
            &inputs.photo_ids(),
            (3_000, 20_367),
        ),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.text());

    let (tripo_id, manual_id): (String, String) = (
        sqlx::query_scalar("SELECT id FROM cost_ledger WHERE provider = 'tripo'")
            .fetch_one(pool(&app))
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT id FROM cost_ledger WHERE provider = 'manual_ai'")
            .fetch_one(pool(&app))
            .await
            .unwrap(),
    );

    // unknown：保留预留金额、actual 保持 NULL（不得填 0）。
    let mut connection = pool(&app).acquire().await.unwrap();
    let unknown =
        ledger_service::mark_submission_unknown(&mut connection, &tripo_id, None, Timestamp::now())
            .await
            .unwrap();
    assert!(matches!(
        unknown,
        everything_manual::storage::repo::ledger::LedgerOutcome::Applied
    ));
    let (state, reserved, actual): (String, i64, Option<i64>) =
        sqlx::query_as("SELECT state, reserved, actual FROM cost_ledger WHERE id = ?")
            .bind(&tripo_id)
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert_eq!(state, "unknown");
    assert_eq!(reserved, 3_000, "unknown 仍保留预留");
    assert_eq!(actual, None, "unknown 的 actual 必须保持 NULL（不得填 0）");

    // 直接 SQL 也不能把 unknown 的实际费用填 0（0001 CHECK 兜底）。
    let fill_zero = sqlx::query("UPDATE cost_ledger SET actual = 0 WHERE id = ?")
        .bind(&tripo_id)
        .execute(pool(&app))
        .await;
    assert!(
        fill_zero.is_err(),
        "unknown 状态下不得把 actual 填 0（CHECK 必须拒绝）"
    );

    // 状态仍是"占用预算"（供 AC-033 展示：未决预留 != 0）。
    let entry = everything_manual::storage::repo::ledger::get(&mut connection, &tripo_id)
        .await
        .unwrap()
        .unwrap();
    assert!(ledger_service::holds_budget(&entry));

    // 自动路径不能释放 unknown（必须走管理员对账 / T15）。
    let rejected =
        ledger_service::release_definitely_not_billed(&mut connection, &tripo_id, Timestamp::now())
            .await
            .unwrap();
    assert!(matches!(
        rejected,
        everything_manual::storage::repo::ledger::LedgerOutcome::Rejected { .. }
    ));
    let state_after: String = sqlx::query_scalar("SELECT state FROM cost_ledger WHERE id = ?")
        .bind(&tripo_id)
        .fetch_one(pool(&app))
        .await
        .unwrap();
    assert_eq!(state_after, "unknown", "自动路径不得改变 unknown");

    // unknown → settled（对账后按实际结算）：同值重复结算幂等、改值被拒（不覆盖事实）。
    let settled =
        ledger_service::settle_attempt(&mut connection, &tripo_id, 2_950, Timestamp::now())
            .await
            .unwrap();
    assert!(matches!(
        settled,
        everything_manual::storage::repo::ledger::LedgerOutcome::Applied
    ));
    let idempotent =
        ledger_service::settle_attempt(&mut connection, &tripo_id, 2_950, Timestamp::now())
            .await
            .unwrap();
    assert!(matches!(
        idempotent,
        everything_manual::storage::repo::ledger::LedgerOutcome::Idempotent
    ));
    let overwrite = ledger_service::settle_attempt(&mut connection, &tripo_id, 1, Timestamp::now())
        .await
        .unwrap();
    assert!(matches!(
        overwrite,
        everything_manual::storage::repo::ledger::LedgerOutcome::Rejected { .. }
    ));
    let (state_final, reserved_final, actual_final): (String, i64, Option<i64>) =
        sqlx::query_as("SELECT state, reserved, actual FROM cost_ledger WHERE id = ?")
            .bind(&tripo_id)
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert_eq!(state_final, "settled");
    assert_eq!(reserved_final, 3_000, "结算不改预留金额");
    assert_eq!(actual_final, Some(2_950), "实际费用按供应商事实落账");
    let entry = everything_manual::storage::repo::ledger::get(&mut connection, &tripo_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!ledger_service::holds_budget(&entry), "结算后不再占用预算");

    // 明确未计费的失败：reserved → released（幂等），不再占用预算。
    let released = ledger_service::release_definitely_not_billed(
        &mut connection,
        &manual_id,
        Timestamp::now(),
    )
    .await
    .unwrap();
    assert!(matches!(
        released,
        everything_manual::storage::repo::ledger::LedgerOutcome::Applied
    ));
    let again = ledger_service::release_definitely_not_billed(
        &mut connection,
        &manual_id,
        Timestamp::now(),
    )
    .await
    .unwrap();
    assert!(matches!(
        again,
        everything_manual::storage::repo::ledger::LedgerOutcome::Idempotent
    ));
    let (manual_state, manual_actual): (String, Option<i64>) =
        sqlx::query_as("SELECT state, actual FROM cost_ledger WHERE id = ?")
            .bind(&manual_id)
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert_eq!(manual_state, "released");
    assert_eq!(manual_actual, None, "释放不写实际费用（不是 0）");

    // 不自动降质量/换模型/加阶段：请求体没有这些字段（结构性），出现即 422。
    let inputs2 = prepare_world(
        &app,
        &cookie,
        &csrf,
        "QA-quality",
        &[Some(100)],
        &["front", "left"],
        true,
    )
    .await;
    let quote2 = estimate_ok(&app, &cookie, &csrf, &inputs2).await;
    let quote2_id = quote2["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs2.item, &quote2_id).await;
    let mut quality_body = job_body(
        &quote2_id,
        &inputs2.preparation,
        &inputs2.photo_ids(),
        (3_000, 20_367),
    );
    quality_body["textureQuality"] = json!("low");
    let rejected_quality = submit(
        &app,
        &cookie,
        &csrf,
        &inputs2.item,
        "qa-ac034-quality",
        &quality_body,
    )
    .await;
    assert_eq!(
        rejected_quality.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "质量/模型/阶段字段不在请求体合同内（结构性拒绝）：{}",
        rejected_quality.text()
    );
    let jobs_after_quality = count(&app, "SELECT COUNT(*) FROM jobs").await;
    assert_eq!(jobs_after_quality, 1, "降质量请求不得创建任务");

    // 先正常建单（消耗 quote2），再以新键重提交同一报价：等价"重生成"必须新报价。
    let first = submit(
        &app,
        &cookie,
        &csrf,
        &inputs2.item,
        "qa-ac034-first",
        &job_body(
            &quote2_id,
            &inputs2.preparation,
            &inputs2.photo_ids(),
            (3_000, 20_367),
        ),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{}", first.text());
    let replay_new_key = submit(
        &app,
        &cookie,
        &csrf,
        &inputs2.item,
        "qa-ac034-regen",
        &job_body(
            &quote2_id,
            &inputs2.preparation,
            &inputs2.photo_ids(),
            (3_000, 20_367),
        ),
    )
    .await;
    assert_eq!(replay_new_key.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        replay_new_key.json()["error"]["details"]["reason"],
        "quoteAlreadyUsed"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 2);
}

// ---------------------------------------------------------------------------
// 卡内项：快照不可变、photo_ids+hashes（与真实内容哈希核对）、无密钥、报价冻结
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_card_snapshot_frozen_hashes_match_content_and_no_secrets() {
    let (app, cookie, csrf) = qa_app("qa-card-snapshot", Some(CATALOG)).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(3000), None, Some(1500)],
        &["front", "left"],
        true,
    )
    .await;
    let quote = estimate_ok(&app, &cookie, &csrf, &inputs).await;
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    let accepted = submit(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "qa-card-snapshot-key",
        &job_body(
            &quote_id,
            &inputs.preparation,
            &inputs.photo_ids(),
            (3_000, 20_367),
        ),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.text());
    let job_id = accepted.json()["data"]["id"].as_str().unwrap().to_owned();

    // 快照内容：photo_ids 与 photo_hashes 一一对应（槽位顺序 front→left），
    // 哈希与 QA 独立计算的图片内容 sha256 一致（防"同 id 换资产"）。
    let (
        photo_ids_json,
        photo_hashes_json,
        provider_config,
        budgets,
        prompt_version,
        price_version,
        item_revision,
        snapshot_id,
    ): (String, String, String, String, String, String, i64, String) = sqlx::query_as(
        "SELECT photo_ids, photo_hashes, provider_config, budgets, prompt_version, price_version, \
                item_revision, id FROM generation_snapshots",
    )
    .fetch_one(pool(&app))
    .await
    .unwrap();
    let ids: Vec<String> = serde_json::from_str(&photo_ids_json).unwrap();
    let hashes: Vec<String> = serde_json::from_str(&photo_hashes_json).unwrap();
    assert_eq!(ids.len(), hashes.len());
    assert_eq!(ids.len(), 2);
    assert_eq!(
        ids[0],
        inputs.photo("front").photo_id,
        "槽位顺序 front 优先"
    );
    assert_eq!(ids[1], inputs.photo("left").photo_id);
    assert_eq!(
        hashes[0],
        inputs.photo("front").sha256,
        "快照必须冻结照片内容哈希（QA 独立计算）"
    );
    assert_eq!(hashes[1], inputs.photo("left").sha256);
    assert_eq!(sha256_hex(&fixture("sample-photo-front.jpg")), hashes[0]);
    assert_eq!(prompt_version, "manual_extract_v1");
    assert_eq!(price_version, "2026-09-11");
    assert_eq!(item_revision, 1, "快照冻结报价确认时的物品版本");

    // provider_config 不含 API key（只含模型与参数）。
    for canary in [
        TRIPO_KEY_CANARY,
        MANUAL_AI_KEY_CANARY,
        "apiKey",
        "api_key",
        "secret",
    ] {
        assert!(
            !provider_config.contains(canary),
            "快照 provider_config 不得包含 {canary}：{provider_config}"
        );
    }
    let config: Value = serde_json::from_str(&provider_config).unwrap();
    assert_eq!(config["tripo"]["model"], TRIPO_MODEL);
    assert_eq!(config["manualAi"]["model"], MANUAL_AI_MODEL);
    assert_eq!(config["tripo"]["faceLimit"], 100_000);

    // budgets：授权 = 用户输入的 limits；上界 = 服务端计算值（两者分开记录）。
    let budgets: Value = serde_json::from_str(&budgets).unwrap();
    assert_eq!(budgets["authorized"]["tripoCreditMinor"], 3_000);
    assert_eq!(budgets["authorized"]["manualAiUsdMicros"], 20_367);
    assert_eq!(budgets["upperBound"]["tripoCreditMinor"], 3_000);
    assert_eq!(budgets["upperBound"]["manualAiUsdMicros"], 20_367);
    assert_eq!(budgets["quoteId"], quote_id);

    // 快照不可变：UPDATE 被触发器拒绝。
    let update_snapshot =
        sqlx::query("UPDATE generation_snapshots SET budgets = '{}' WHERE id = ?")
            .bind(&snapshot_id)
            .execute(pool(&app))
            .await;
    assert!(update_snapshot.is_err(), "generation_snapshots 必须不可变");

    // 报价冻结：quote_json / expires_at 不可改写；消费标记只允许 null → 值。
    let update_quote = sqlx::query("UPDATE quotes SET quote_json = '{}' WHERE id = ?")
        .bind(&quote_id)
        .execute(pool(&app))
        .await;
    assert!(update_quote.is_err(), "报价载荷必须冻结");
    let update_expiry = sqlx::query("UPDATE quotes SET expires_at = 1 WHERE id = ?")
        .bind(&quote_id)
        .execute(pool(&app))
        .await;
    assert!(update_expiry.is_err(), "报价到期时间必须冻结");
    let clear_consumed = sqlx::query("UPDATE quotes SET consumed_at = NULL WHERE id = ?")
        .bind(&quote_id)
        .execute(pool(&app))
        .await;
    assert!(clear_consumed.is_err(), "消费标记不得清除");

    // 快照与 job 关联一致。
    let snapshot_of_job: String = sqlx::query_scalar("SELECT snapshot_id FROM jobs WHERE id = ?")
        .bind(&job_id)
        .fetch_one(pool(&app))
        .await
        .unwrap();
    assert_eq!(snapshot_of_job, snapshot_id);
}

// ---------------------------------------------------------------------------
// 卡内项：跨物品引用 404、新路由的认证/CSRF 边界
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_card_cross_item_references_and_auth_boundaries() {
    let (app, cookie, csrf) = qa_app("qa-card-cross", Some(CATALOG)).await;
    let inputs = prepare_world(
        &app,
        &cookie,
        &csrf,
        "X100V",
        &[Some(100)],
        &["front", "left"],
        true,
    )
    .await;
    let quote = estimate_ok(&app, &cookie, &csrf, &inputs).await;
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    confirm(&app, &cookie, &csrf, &inputs.item, &quote_id).await;

    // 跨物品：另一物品的照片不进入报价（404，不泄露存在性）。
    let other = create_item(&app, &cookie, &csrf, "QA-other").await;
    let cross_item = app
        .call(Method::POST, &format!("/api/v1/items/{other}/estimates"))
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({
            "preparationId": inputs.preparation,
            "photoIds": inputs.photo_ids(),
            "modelPreset": PRESET,
        }))
        .send()
        .await;
    assert_eq!(
        cross_item.status,
        StatusCode::NOT_FOUND,
        "{}",
        cross_item.text()
    );

    // 跨物品回读报价 / 确认报价 → 404。
    let got = app
        .call(
            Method::GET,
            &format!("/api/v1/items/{other}/estimates/{quote_id}"),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(got.status, StatusCode::NOT_FOUND);
    let confirmed = confirm(&app, &cookie, &csrf, &other, &quote_id).await;
    assert_eq!(confirmed.status, StatusCode::NOT_FOUND);

    // 跨物品提交任务（本物品报价 + 另一物品路径）→ 404。
    let cross_submit = submit(
        &app,
        &cookie,
        &csrf,
        &other,
        "qa-card-cross-key",
        &job_body(
            &quote_id,
            &inputs.preparation,
            &inputs.photo_ids(),
            (3_000, 20_367),
        ),
    )
    .await;
    assert_eq!(cross_submit.status, StatusCode::NOT_FOUND);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);

    // 新路由未登录 → 401；已登录缺 CSRF 的写操作 → 403；Nonexistent quote → 404。
    let unauth_estimate = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{}/estimates", inputs.item),
        )
        .json(&json!({}))
        .send()
        .await;
    assert_eq!(unauth_estimate.status, StatusCode::UNAUTHORIZED);
    let unauth_jobs = app
        .call(Method::POST, &format!("/api/v1/items/{}/jobs", inputs.item))
        .json(&json!({}))
        .send()
        .await;
    assert_eq!(unauth_jobs.status, StatusCode::UNAUTHORIZED);
    let unauth_confirm = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{}/estimates/{quote_id}/confirm", inputs.item),
        )
        .send()
        .await;
    assert_eq!(unauth_confirm.status, StatusCode::UNAUTHORIZED);

    let no_csrf = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{}/estimates/{quote_id}/confirm", inputs.item),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(no_csrf.status, StatusCode::FORBIDDEN, "{}", no_csrf.text());

    // 不存在的报价 → 404；已确认报价仍未创建 job（确认不等于提交）。
    let unknown_quote = app
        .call(
            Method::GET,
            &format!(
                "/api/v1/items/{}/estimates/01993000-0000-7000-8000-000000000000",
                inputs.item
            ),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(unknown_quote.status, StatusCode::NOT_FOUND);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM audit_events").await, 1);
}
