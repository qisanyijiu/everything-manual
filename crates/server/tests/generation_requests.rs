//! T11 集成测试：输入快照、报价、预算与幂等（PRD 修订 2 / ui_revision 2）。
//!
//! 覆盖的验收条件（命令 ↔ AC 映射见 implementation.md §T11-8）：
//! - **AC-028**：estimate 返回分列金额（creditMinor / usdMicros）、价格版本与快照日期、
//!   保守上界、expiresAt（默认 10 分钟）；**不调用生成服务、不产生费用记录**；
//! - **AC-029**：缺价格 → 409 `PRICE_CATALOG_MISSING`；缺 Provider → 409
//!   `PROVIDER_NOT_CONFIGURED`；未 ready / 缺 front / 缺侧视图 → 422 列出具体缺项；
//! - **AC-030**：未确认提交被拒（不创建 job）；确认写 `audit_events`；不默认勾选；
//! - **AC-031**：同 key 同 body 重放 20 次只建 1 个 job、1 份预留（每供应商 1 行）；
//!   同 key 不同 body → 409；事务中断不留半笔预留（断点注入）；
//! - **AC-032**：过期报价 / 输入已变 / 允许上限低于服务端上界 → 拒绝且不创建 job、
//!   不产生远端请求；服务端不采信前端传入费用；
//! - **AC-033/AC-034（服务端侧）**：unknown 保留预留（不填 0）、结算/释放幂等、
//!   自动路径不释放 unknown、不自动降质量/换模型。
//!
//! 隔离与门控：临时 data-dir + 真实 SQLite；全部 HTTP 都是进程内 `oneshot`，
//! **没有任何真实外网调用**（服务端依赖树无 HTTP 客户端，T02 QA 知识 1）；
//! 付费调用计数以 `provider_attempts` 行数代替（T11 不产生 attempt）。

mod common;

use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::{TestApp, TestDir, TestResponse};
use everything_manual::config::{DEFAULT_MANUAL_AI_BASE_URL, ProviderSettings, SecretString};
use everything_manual::generation::catalog;
use everything_manual::storage::repo::{attempts as attempts_repo, ledger};
use manual_core::timestamps::Timestamp;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;

const PASSWORD: &str = "test-password-gen-4b71";

/// 测试价格目录（与仓库根的示例同价：Tripo 30 credits；说明书 AI 按示例单价）。
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

/// 舍入边界目录：金额小数位多于最小单位（必须向上取整，绝不低估）。
const BOUNDARY_CATALOG: &str = r#"
version = "2026-09-11"
snapshot_date = "2026-09-11"

[[tripo.presets]]
preset = "tripo-h-v3.1-standard"
model = "v3.1-20260211"
credits = "0.005"

[manual_ai.models.gpt-5-mini]
input_usd_per_million_tokens = "0.0000005"
output_usd_per_million_tokens = "0.0000004"
image_usd_per_image = "0.0000001"
"#;

const TRIPO_MODEL: &str = "v3.1-20260211";
const MANUAL_AI_MODEL: &str = "gpt-5-mini";
const PRESET: &str = "tripo-h-v3.1-standard";
/// 服务端按目录计算的 Tripo 保守上界（30 credits = 3000 creditMinor）。
const TRIPO_UPPER_BOUND: i64 = 3000;

// ---------------------------------------------------------------------------
// 样例资产与上传工具（与 preparations.rs 同风格；测试二进制之间不共享代码）
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("读取样例资产 {} 失败：{error}", path.display()))
}

struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new(tag: &str) -> Self {
        Self {
            boundary: format!("----em-gen-{tag}"),
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

/// 带已配置 Provider 与价格目录的测试应用。
async fn generation_app(tag: &str, catalog_toml: &str) -> TestApp {
    let dir = TestDir::new(tag);
    let mut settings = common::test_settings(dir.path());
    settings.providers.tripo = common::configured_tripo("canary-tripo-key");
    settings.providers.manual_ai = ProviderSettings {
        name: "manual_ai",
        base_url: DEFAULT_MANUAL_AI_BASE_URL.to_owned(),
        model: Some(MANUAL_AI_MODEL.to_owned()),
        api_key: Some(SecretString::new("canary-manual-ai-key")),
        key_source: Some("测试注入".to_owned()),
    };
    settings.price_catalog_path = Some(dir.join("price-catalog.toml"));
    std::fs::write(dir.join("price-catalog.toml"), catalog_toml).expect("写入测试价格目录");
    settings.price_catalog = Some(catalog::parse(catalog_toml).expect("测试价格目录必须可解析"));
    TestApp::with_settings(dir, settings).await
}

async fn login(app: &TestApp) -> (String, String) {
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

async fn logged_in_generation_app(tag: &str, catalog_toml: &str) -> (TestApp, String, String) {
    let app = generation_app(tag, catalog_toml).await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    (app, cookie, csrf)
}

async fn create_item(app: &TestApp, cookie: &str, csrf: &str, model: &str) -> String {
    let response = app
        .call(Method::POST, "/api/v1/items")
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "name": "报价测试物品", "model": model }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()["data"]["id"].as_str().unwrap().to_owned()
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
) -> (String, String) {
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
    (
        response.json()["data"]["id"].as_str().unwrap().to_owned(),
        response.json()["data"]["sha256"]
            .as_str()
            .unwrap()
            .to_owned(),
    )
}

/// 准备一份合格的生成输入：物品 + PDF 文档 + 3 页准备（1/3 文字、2 扫描）+ front/left 照片。
struct ReadyInputs {
    item: String,
    preparation: String,
    photos: Vec<(String, String)>, // (view, photoId)
}

async fn build_ready_inputs(app: &TestApp, cookie: &str, csrf: &str, model: &str) -> ReadyInputs {
    // 默认 3 页：1/3 有文字（3000/120 字节），第 2 页是扫描页（无文字层）。
    build_ready_inputs_with_pages(app, cookie, csrf, model, &[Some(3000), None, Some(120)]).await
}

/// 指定每页输入（`Some(字节数)` = 文字页、`None` = 扫描页）的准备输入。
async fn build_ready_inputs_with_pages(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    model: &str,
    pages: &[Option<usize>],
) -> ReadyInputs {
    let item = create_item(app, cookie, csrf, model).await;

    // 文档 + 准备记录
    let pdf = fixture("sample-manual-text.pdf");
    let (doc_asset, _sha) = upload_asset(
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

    let page_jpeg = fixture("sample-photo-front.jpg");
    for (index, text) in pages.iter().enumerate() {
        let page = index as i64 + 1;
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
        .await
        .0;
        let text_asset = match *text {
            Some(bytes) => Some(
                upload_asset(
                    app,
                    cookie,
                    csrf,
                    &item,
                    UploadSpec {
                        purpose: "pageText",
                        filename: "page.txt",
                        content_type: "text/plain",
                        bytes: "a".repeat(bytes).as_bytes(),
                    },
                )
                .await
                .0,
            ),
            None => None,
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
    // 页写入会自增 preparation.revision：先 GET 取当前 ETag 再封存。
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
        .json(&json!({ "pageCount": pages.len() as i64 }))
        .send()
        .await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text());

    // 照片：front + left（多视图最小合格集合）
    let mut photos = Vec::new();
    for (view, name) in [
        ("front", "sample-photo-front.jpg"),
        ("left", "sample-photo-left.png"),
    ] {
        let bytes = fixture(name);
        let content_type = if name.ends_with(".png") {
            "image/png"
        } else {
            "image/jpeg"
        };
        let (asset, _sha) = upload_asset(
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
        photos.push((
            view.to_owned(),
            response.json()["data"]["id"].as_str().unwrap().to_owned(),
        ));
    }

    ReadyInputs {
        item,
        preparation,
        photos,
    }
}

fn photo_ids(inputs: &ReadyInputs) -> Vec<String> {
    inputs.photos.iter().map(|(_, id)| id.clone()).collect()
}

async fn create_estimate(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    preparation: &str,
    photos: &[String],
    preset: &str,
) -> TestResponse {
    app.call(Method::POST, &format!("/api/v1/items/{item}/estimates"))
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({
            "preparationId": preparation,
            "photoIds": photos,
            "modelPreset": preset,
        }))
        .send()
        .await
}

async fn confirm_quote(
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

async fn create_job(
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

/// 从 estimate 响应取出的关键字段（测试断言用）。
struct QuoteView {
    id: String,
    tripo_upper: i64,
    manual_ai_upper: i64,
}

async fn estimate_and_view(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    inputs: &ReadyInputs,
) -> QuoteView {
    let response = create_estimate(
        app,
        cookie,
        csrf,
        &inputs.item,
        &inputs.preparation,
        &photo_ids(inputs),
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let body = response.json();
    let data = &body["data"];
    QuoteView {
        id: data["id"].as_str().unwrap().to_owned(),
        tripo_upper: data["amounts"]["tripo"]["upperBoundMinor"]
            .as_i64()
            .unwrap(),
        manual_ai_upper: data["amounts"]["manualAi"]["upperBoundMinor"]
            .as_i64()
            .unwrap(),
    }
}

// ---------------------------------------------------------------------------
// 数据库辅助
// ---------------------------------------------------------------------------

fn pool(app: &TestApp) -> &SqlitePool {
    app.state().database().pool()
}

/// 单管理员的 id（幂等记录有 `admin_id` 外键，服务层用例必须用真实值）。
async fn admin_id(app: &TestApp) -> String {
    sqlx::query_scalar("SELECT id FROM admins LIMIT 1")
        .fetch_one(pool(app))
        .await
        .unwrap()
}

async fn count(app: &TestApp, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool(app)).await.unwrap()
}

// ---------------------------------------------------------------------------
// AC-028：estimate 正常路径
// ---------------------------------------------------------------------------

#[tokio::test]
async fn estimate_returns_itemized_amounts_and_writes_no_cost_or_attempt_records() {
    let (app, cookie, csrf) = logged_in_generation_app("estimate-ok", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;

    let before = Timestamp::now();
    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &photo_ids(&inputs),
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let body = response.json();
    let data = &body["data"];

    // 分列金额：整数最小单位 + 单位明确 + 展示串（不相加、无无单位数字）。
    let tripo = &data["amounts"]["tripo"];
    assert_eq!(tripo["currency"], "creditMinor");
    assert_eq!(tripo["estimatedMinor"], TRIPO_UPPER_BOUND);
    assert_eq!(tripo["upperBoundMinor"], TRIPO_UPPER_BOUND);
    assert_eq!(tripo["upperBoundDisplay"], "30.00 credits");
    let manual = &data["amounts"]["manualAi"];
    assert_eq!(manual["currency"], "usdMicros");
    // 1 批：输入上界 = 1200（批次开销）+ 3000 + 120（两页文字 1 token/字节）
    //       + 3000（一张扫描页图）；输出上界 = 1 × 4096。
    assert_eq!(manual["upperBoundLines"][0]["quantity"], 7320);
    assert_eq!(manual["upperBoundLines"][0]["amountMinor"], 1830);
    assert_eq!(manual["upperBoundLines"][1]["quantity"], 4096);
    assert_eq!(manual["upperBoundLines"][1]["amountMinor"], 8192);
    assert_eq!(manual["upperBoundLines"][2]["quantity"], 1);
    assert_eq!(manual["upperBoundLines"][2]["amountMinor"], 10_000);
    assert_eq!(
        manual["upperBoundMinor"].as_i64().unwrap(),
        manual["upperBoundLines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line["amountMinor"].as_i64().unwrap())
            .sum::<i64>()
    );
    assert!(
        manual["estimatedMinor"].as_i64().unwrap() <= manual["upperBoundMinor"].as_i64().unwrap()
            && tripo["estimatedMinor"].as_i64().unwrap()
                <= tripo["upperBoundMinor"].as_i64().unwrap()
    );

    // 价格版本与快照日期、页数与输出 token 上界。
    assert_eq!(data["priceVersion"], "2026-09-11");
    assert_eq!(data["priceSnapshotDate"], "2026-09-11");
    assert_eq!(data["pageCount"], 3);
    assert_eq!(data["pageRange"]["from"], 1);
    assert_eq!(data["pageRange"]["to"], 3);
    assert_eq!(data["maxOutputTokens"], 4096);

    // expiresAt 默认 10 分钟（A-02）。
    let expires =
        manual_core::timestamps::Timestamp::from_rfc3339(data["expiresAt"].as_str().unwrap())
            .unwrap();
    let delta = expires.as_millis() - before.as_millis();
    assert!(
        (600_000..=601_000).contains(&delta),
        "expiresAt 应为 now+10min（实测 {delta} ms）"
    );
    assert!(data["confirmedAt"].is_null(), "不得默认确认");
    assert!(data["consumedAt"].is_null());

    // 发送范围（确认页数据）：视图/页范围/页图/型号文本/模型名/上界说明。
    assert_eq!(data["sendScope"]["tripo"]["model"], TRIPO_MODEL);
    assert_eq!(data["sendScope"]["tripo"]["preset"], PRESET);
    let views: Vec<&str> = data["sendScope"]["tripo"]["views"]
        .as_array()
        .unwrap()
        .iter()
        .map(|view| view["view"].as_str().unwrap())
        .collect();
    assert_eq!(views, vec!["front", "left"], "槽位顺序 front→left");
    assert_eq!(data["sendScope"]["manualAi"]["pageFrom"], 1);
    assert_eq!(data["sendScope"]["manualAi"]["pageTo"], 3);
    assert_eq!(data["sendScope"]["manualAi"]["textPages"], json!([1, 3]));
    assert_eq!(data["sendScope"]["manualAi"]["imagePages"], json!([2]));
    assert_eq!(data["sendScope"]["manualAi"]["model"], MANUAL_AI_MODEL);
    // REQ-021：发送内容必须包含物品身份文本（名称/型号）与 Tripo 参数。
    assert_eq!(data["sendScope"]["manualAi"]["itemName"], "报价测试物品");
    assert_eq!(data["sendScope"]["manualAi"]["itemModel"], "X100V");
    assert_eq!(
        data["sendScope"]["tripo"]["parameters"]["faceLimit"],
        100000
    );
    assert_eq!(
        data["sendScope"]["tripo"]["parameters"]["textureQuality"],
        "standard"
    );
    assert!(
        data["budgetNotice"]
            .as_str()
            .unwrap()
            .contains("不是供应商账户级硬封顶"),
        "预算说明必须如实表述（REQ-023）"
    );

    // 只计算计划：无费用记录、无 job、无 attempt、无确认审计；响应不含密钥。
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM provider_attempts").await,
        0
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM quotes").await, 1);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM audit_events").await, 0);
    let text = response.text();
    assert!(!text.contains("canary-tripo-key"), "响应泄露密钥：{text}");
    assert!(!text.contains("canary-manual-ai-key"), "响应泄露密钥");
    assert!(!text.contains("api_key"), "响应不得含配置键");

    // 报价落库载荷与响应同源（同一份 JSON；回读不信任前端）。
    let stored: String = sqlx::query_scalar("SELECT quote_json FROM quotes WHERE id = ?")
        .bind(data["id"].as_str().unwrap())
        .fetch_one(pool(&app))
        .await
        .unwrap();
    let stored: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(stored, *data);

    // GET 回读不丢信息（确认页刷新路径）。
    let fetched = app
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
    assert_eq!(fetched.status, StatusCode::OK, "{}", fetched.text());
    assert_eq!(fetched.json()["data"]["amounts"], data["amounts"]);
}

// ---------------------------------------------------------------------------
// AC-029：缺配置 / 缺项
// ---------------------------------------------------------------------------

#[tokio::test]
async fn estimate_without_price_catalog_returns_409_price_catalog_missing() {
    let (app, cookie, csrf) = logged_in_generation_app("estimate-noprice", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    // 去掉价格目录（模拟未配置 price_catalog_path），复用同一数据库句柄。
    let mut settings = app.state().settings().clone();
    settings.providers.tripo = common::configured_tripo("canary-tripo-key");
    settings.providers.manual_ai = ProviderSettings {
        name: "manual_ai",
        base_url: DEFAULT_MANUAL_AI_BASE_URL.to_owned(),
        model: Some(MANUAL_AI_MODEL.to_owned()),
        api_key: Some(SecretString::new("canary-manual-ai-key")),
        key_source: None,
    };
    settings.price_catalog = None;
    settings.price_catalog_path = None;
    let state =
        everything_manual::http::state::AppState::new(app.state().database().clone(), settings);
    let router = everything_manual::http::router::build_app(state);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v1/items/{}/estimates", inputs.item))
                .header("host", "127.0.0.1:8080")
                .header("cookie", &cookie)
                .header("x-csrf-token", &csrf)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "preparationId": inputs.preparation,
                        "photoIds": photo_ids(&inputs),
                        "modelPreset": PRESET,
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: Value = serde_json::from_slice(
        &http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes(),
    )
    .unwrap();
    assert_eq!(body["error"]["code"], "PRICE_CATALOG_MISSING");
    assert!(body["error"]["details"]["missing"].is_array());
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
}

#[tokio::test]
async fn estimate_without_provider_configuration_returns_409() {
    let dir = TestDir::new("estimate-noprovider");
    let mut settings = common::test_settings(dir.path());
    // 价格目录在场，但供应商未配置（无密钥）。
    settings.price_catalog_path = Some(dir.join("price-catalog.toml"));
    std::fs::write(dir.join("price-catalog.toml"), TEST_CATALOG).unwrap();
    settings.price_catalog = Some(catalog::parse(TEST_CATALOG).unwrap());
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;

    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &photo_ids(&inputs),
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.text());
    let body = response.json();
    assert_eq!(body["error"]["code"], "PROVIDER_NOT_CONFIGURED");
    let missing = body["error"]["details"]["missing"].as_array().unwrap();
    assert!(
        missing
            .iter()
            .any(|item| item.as_str().unwrap().starts_with("tripo.")),
        "{missing:?}"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM quotes").await, 0);
}

#[tokio::test]
async fn estimate_rejects_unsupported_preset_and_catalog_without_model_price() {
    let (app, cookie, csrf) = logged_in_generation_app("estimate-catalog-gaps", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;

    // 目录里没有的预设 → 422 modelPresetUnsupported（不进入支持清单）。
    let unsupported = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &photo_ids(&inputs),
        "tripo-h-nonexistent",
    )
    .await;
    assert_eq!(unsupported.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        unsupported.json()["error"]["details"]["reason"],
        "modelPresetUnsupported"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM quotes").await, 0);

    // 目录缺少该说明书 AI 模型的单价 → 409 PRICE_CATALOG_MISSING（不按猜测价格计算）。
    let catalog_without_model = TEST_CATALOG.replace(
        "[manual_ai.models.gpt-5-mini]",
        "[manual_ai.models.other-model]",
    );
    let (app2, cookie2, csrf2) =
        logged_in_generation_app("estimate-catalog-model-gap", &catalog_without_model).await;
    let inputs2 = build_ready_inputs(&app2, &cookie2, &csrf2, "X100V").await;
    let response = create_estimate(
        &app2,
        &cookie2,
        &csrf2,
        &inputs2.item,
        &inputs2.preparation,
        &photo_ids(&inputs2),
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.text());
    assert_eq!(response.json()["error"]["code"], "PRICE_CATALOG_MISSING");
    assert!(
        response.json()["error"]["details"]["missing"][0]
            .as_str()
            .unwrap()
            .contains("manualAiModel"),
        "{}",
        response.text()
    );
}

#[tokio::test]
async fn estimate_lists_specific_precondition_gaps() {
    let (app, cookie, csrf) =
        logged_in_generation_app("estimate-preconditions", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;

    // 1) 只带 front：缺侧视图。
    let front_only: Vec<String> = inputs
        .photos
        .iter()
        .filter(|(view, _)| view == "front")
        .map(|(_, id)| id.clone())
        .collect();
    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &front_only,
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.json();
    assert_eq!(body["error"]["details"]["reason"], "preconditionsFailed");
    let codes: Vec<&str> = body["error"]["details"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, vec!["missingSideView"], "{body}");

    // 2) 只带 left：缺 front。
    let left_only: Vec<String> = inputs
        .photos
        .iter()
        .filter(|(view, _)| view == "left")
        .map(|(_, id)| id.clone())
        .collect();
    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &left_only,
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let codes: Vec<String> = response.json()["error"]["details"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["code"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(codes, vec!["missingFrontView"]);

    // 3) detail 照片不进入多视图：与"缺侧视图"一起列出（一次列全）。
    let detail_asset = fixture("sample-photo-front.jpg");
    let (asset, _) = upload_asset(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        UploadSpec {
            purpose: "photo",
            filename: "detail.jpg",
            content_type: "image/jpeg",
            bytes: &detail_asset,
        },
    )
    .await;
    let detail = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{}/photos", inputs.item),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({ "assetId": asset, "view": "detail" }))
        .send()
        .await;
    assert_eq!(detail.status, StatusCode::CREATED);
    let detail_id = detail.json()["data"]["id"].as_str().unwrap().to_owned();
    let mut with_detail = front_only.clone();
    with_detail.push(detail_id);
    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &with_detail,
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let codes: Vec<String> = response.json()["error"]["details"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["code"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(codes, vec!["detailViewNotAllowed", "missingSideView"]);

    // 4) preparation 未 ready：与视图缺项一起列出。
    let pdf = fixture("sample-manual-text.pdf");
    let (doc_asset, _) = upload_asset(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        UploadSpec {
            purpose: "document",
            filename: "manual2.pdf",
            content_type: "application/pdf",
            bytes: &pdf,
        },
    )
    .await;
    let document = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{}/documents", inputs.item),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({ "sourceAssetId": doc_asset, "title": "第二份" }))
        .send()
        .await;
    let source_sha = document.json()["data"]["sourceSha256"]
        .as_str()
        .unwrap()
        .to_owned();
    let preparing = app
        .call(
            Method::POST,
            &format!(
                "/api/v1/documents/{}/preparations",
                document.json()["data"]["id"].as_str().unwrap()
            ),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({ "sourceSha256": source_sha }))
        .send()
        .await;
    let preparing_id = preparing.json()["data"]["id"].as_str().unwrap().to_owned();
    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &preparing_id,
        &photo_ids(&inputs),
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let codes: Vec<String> = response.json()["error"]["details"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["code"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(codes, vec!["preparationNotReady"]);

    // 缺项路径不产生任何报价/费用记录。
    assert_eq!(count(&app, "SELECT COUNT(*) FROM quotes").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);

    // 5) 字段级：缺字段与重复照片。
    let response = create_estimate(&app, &cookie, &csrf, &inputs.item, "", &[], "").await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let fields: Vec<String> = response.json()["error"]["details"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["field"].as_str().unwrap().to_owned())
        .collect();
    assert!(fields.contains(&"preparationId".to_owned()), "{fields:?}");
    assert!(fields.contains(&"photoIds".to_owned()), "{fields:?}");
    assert!(fields.contains(&"modelPreset".to_owned()), "{fields:?}");

    let mut duplicated = photo_ids(&inputs);
    if let Some(first) = duplicated.first().cloned() {
        duplicated.push(first);
    }
    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &duplicated,
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.json();
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
    assert!(
        body["error"]["details"]["fields"][0]["message"]
            .as_str()
            .unwrap()
            .contains("只能出现一次")
    );
}

// ---------------------------------------------------------------------------
// AC-030：云端发送确认
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unconfirmed_submission_is_rejected_and_confirmation_is_audited_once() {
    let (app, cookie, csrf) = logged_in_generation_app("confirm-audit", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;

    // 未确认（不勾选）提交 → 422，不创建 job、不产生预留。
    let body = job_body(
        &quote.id,
        &inputs.preparation,
        &photo_ids(&inputs),
        (10_000, 100_000),
    );
    let response = create_job(&app, &cookie, &csrf, &inputs.item, "key-unconfirmed", &body).await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let error = response.json();
    assert_eq!(
        error["error"]["details"]["reason"],
        "confirmationRequired",
        "{}",
        response.text()
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);

    // 确认（显式动作）→ 200 + 发送范围；audit_events 记录一次。
    let confirmed = confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.text());
    let confirmation = confirmed.json();
    let data = &confirmation["data"];
    assert_eq!(data["quoteId"], quote.id.as_str());
    assert_eq!(data["sendScope"]["tripo"]["model"], TRIPO_MODEL);
    assert_eq!(data["sendScope"]["tripo"]["views"][0]["view"], "front");
    assert_eq!(data["sendScope"]["manualAi"]["itemModel"], "X100V");
    assert_eq!(data["sendScope"]["manualAi"]["model"], MANUAL_AI_MODEL);
    assert!(data["confirmedAt"].is_string());
    assert!(
        data["summary"].as_str().unwrap().contains("已确认发送范围"),
        "{data}"
    );

    let audit_count = count(
        &app,
        "SELECT COUNT(*) FROM audit_events WHERE action = 'generation_send_scope_confirmed'",
    )
    .await;
    assert_eq!(audit_count, 1);
    let metadata: String = sqlx::query_scalar(
        "SELECT metadata_json FROM audit_events WHERE action = 'generation_send_scope_confirmed'",
    )
    .fetch_one(pool(&app))
    .await
    .unwrap();
    let metadata: Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["tripoModel"], TRIPO_MODEL);
    assert_eq!(metadata["itemModel"], "X100V");
    assert_eq!(metadata["tripoParameters"]["faceLimit"], 100000);
    assert_eq!(metadata["priceVersion"], "2026-09-11");
    assert_eq!(metadata["tripoViews"][0]["view"], "front");
    assert_eq!(metadata["pageFrom"], 1);
    assert_eq!(metadata["pageTo"], 3);
    assert_eq!(
        metadata["upperBound"]["tripoCreditMinor"],
        TRIPO_UPPER_BOUND
    );
    assert!(!metadata.to_string().contains("canary"), "审计不得含密钥");

    // 重复确认幂等：时间不变、审计仍只有一条。
    let again = confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.json()["data"]["confirmedAt"], data["confirmedAt"]);
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM audit_events WHERE action = 'generation_send_scope_confirmed'"
        )
        .await,
        1
    );

    // 确认后提交 → 202（同事务冻结 + 预留）。
    let response = create_job(&app, &cookie, &csrf, &inputs.item, "key-confirmed", &body).await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    let job = response.json();
    assert_eq!(job["data"]["status"], "queued");
    assert_eq!(job["data"]["reservations"].as_array().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// AC-031：幂等重放、冲突与事务性
// ---------------------------------------------------------------------------

#[tokio::test]
async fn same_key_same_body_replayed_twenty_times_creates_single_job_and_reservation() {
    let (app, cookie, csrf) = logged_in_generation_app("idempotent-replay", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;

    let body = job_body(
        &quote.id,
        &inputs.preparation,
        &photo_ids(&inputs),
        (quote.tripo_upper, quote.manual_ai_upper),
    );
    let mut first_job: Option<String> = None;
    for attempt in 0..20 {
        let response = create_job(&app, &cookie, &csrf, &inputs.item, "replay-key", &body).await;
        assert_eq!(
            response.status,
            StatusCode::ACCEPTED,
            "第 {attempt} 次提交：{}",
            response.text()
        );
        let job_id = response.json()["data"]["id"].as_str().unwrap().to_owned();
        match &first_job {
            None => first_job = Some(job_id),
            Some(expected) => assert_eq!(&job_id, expected, "重放必须返回同一 job"),
        }
        if attempt > 0 {
            assert_eq!(
                response.header("x-idempotent-replay").as_deref(),
                Some("true"),
                "重放应显式标记"
            );
        }
    }

    // 只存在 1 个 job、1 个快照、每供应商恰好 1 笔预留、1 条幂等记录。
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM generation_snapshots").await,
        1
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 2);
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM cost_ledger WHERE provider = 'tripo' AND state = 'reserved'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM cost_ledger WHERE provider = 'manual_ai' AND state = 'reserved'"
        )
        .await,
        1
    );
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM idempotency_records").await,
        1
    );
    // 预留金额 = 服务端计算的保守上界（不是请求里更大的授权值）。
    let reserved: i64 =
        sqlx::query_scalar("SELECT reserved FROM cost_ledger WHERE provider = 'tripo'")
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert_eq!(reserved, TRIPO_UPPER_BOUND);
    // 报价只能建一份任务：消费标记指向该 job。
    let consumed_job: Option<String> = sqlx::query_scalar("SELECT consumed_job_id FROM quotes")
        .fetch_one(pool(&app))
        .await
        .unwrap();
    assert_eq!(consumed_job.as_deref(), first_job.as_deref());
    // T11 不产生任何付费 attempt（真实提交属 T12/T14）。
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM provider_attempts").await,
        0
    );
}

#[tokio::test]
async fn same_key_with_different_body_returns_409_and_keeps_single_job() {
    let (app, cookie, csrf) = logged_in_generation_app("idempotent-conflict", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;

    let body = job_body(
        &quote.id,
        &inputs.preparation,
        &photo_ids(&inputs),
        (quote.tripo_upper, quote.manual_ai_upper),
    );
    let first = create_job(&app, &cookie, &csrf, &inputs.item, "conflict-key", &body).await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{}", first.text());

    // 同 key、不同 body（提高预算）→ 409 IDEMPOTENCY_CONFLICT。
    let changed = job_body(
        &quote.id,
        &inputs.preparation,
        &photo_ids(&inputs),
        (quote.tripo_upper + 1, quote.manual_ai_upper),
    );
    let conflict = create_job(&app, &cookie, &csrf, &inputs.item, "conflict-key", &changed).await;
    assert_eq!(conflict.status, StatusCode::CONFLICT, "{}", conflict.text());
    let error = conflict.json();
    assert_eq!(error["error"]["code"], "IDEMPOTENCY_CONFLICT");
    assert_eq!(error["error"]["details"]["reason"], "idempotencyKeyReused");
    assert_eq!(
        error["error"]["details"]["existingResourceId"].as_str(),
        Some(first.json()["data"]["id"].as_str().unwrap())
    );

    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 2);
}

#[tokio::test]
async fn interrupted_creation_transaction_leaves_no_half_reservation() {
    use everything_manual::jobs::failpoints::{self, FailpointAction, GENERATION_BEFORE_COMMIT};

    let (app, cookie, csrf) = logged_in_generation_app("failpoint-tx", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;
    let body = job_body(
        &quote.id,
        &inputs.preparation,
        &photo_ids(&inputs),
        (quote.tripo_upper, quote.manual_ai_upper),
    );

    // 断点在"快照 + 预留 + job + 阶段 + 幂等记录已写入、事务尚未提交"处 panic。
    // owner = 本请求的幂等键（owner 分区），并行测试互不干扰。
    failpoints::set("tx-key", GENERATION_BEFORE_COMMIT, FailpointAction::Panic);
    let router: Router = app.router_handle();
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/v1/items/{}/jobs", inputs.item))
        .header("host", "127.0.0.1:8080")
        .header("cookie", &cookie)
        .header("x-csrf-token", &csrf)
        .header("idempotency-key", "tx-key")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let join = tokio::task::spawn(async move { router.oneshot(request).await });
    let outcome = tokio::time::timeout(Duration::from_secs(10), join)
        .await
        .expect("断点请求应在超时前结束");
    assert!(
        outcome.is_err(),
        "断点必须中断请求（panic），实际 {outcome:?}"
    );
    failpoints::clear_owner("tx-key");

    // 事务中断：不留半笔预留、不产生 job/快照/幂等记录，报价也未被消费。
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM jobs").await,
        0,
        "不得留下半个 job"
    );
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM generation_snapshots").await,
        0,
        "不得留下孤儿快照"
    );
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM cost_ledger").await,
        0,
        "不得留下半笔预留"
    );
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

    // 清除断点后同键同 body 重试成功（证明失败完全回滚、键可复用）。
    let retry = create_job(&app, &cookie, &csrf, &inputs.item, "tx-key", &body).await;
    assert_eq!(retry.status, StatusCode::ACCEPTED, "{}", retry.text());
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 2);
}

#[tokio::test]
async fn concurrent_submissions_never_create_duplicate_jobs_or_reservations() {
    let (app, cookie, csrf) = logged_in_generation_app("concurrent-jobs", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;
    let body = job_body(
        &quote.id,
        &inputs.preparation,
        &photo_ids(&inputs),
        (quote.tripo_upper, quote.manual_ai_upper),
    );

    // 同一 key、同一 body、10 路并发：全部 202 且指向同一 job。
    let mut handles = Vec::new();
    for _ in 0..10 {
        let router: Router = app.router_handle();
        let request = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/v1/items/{}/jobs", inputs.item))
            .header("host", "127.0.0.1:8080")
            .header("cookie", &cookie)
            .header("x-csrf-token", &csrf)
            .header("idempotency-key", "concurrent-same-key")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        handles.push(tokio::spawn(async move {
            let response = router.oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = http_body_util::BodyExt::collect(response.into_body())
                .await
                .unwrap()
                .to_bytes();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        }));
    }
    let mut job_ids = Vec::new();
    for handle in handles {
        let (status, body) = handle.await.expect("并发请求不应 panic");
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        job_ids.push(body["data"]["id"].as_str().unwrap().to_owned());
    }
    job_ids.dedup();
    assert_eq!(job_ids.len(), 1, "并发同键必须只产生 1 个 job：{job_ids:?}");
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 2);

    // 同一报价、不同 key、5 路并发：只有 1 个成功（其余 quoteAlreadyUsed），
    // 且总预留仍恰好 2 笔（不会因竞争重复预留）。
    let second_inputs = build_ready_inputs(&app, &cookie, &csrf, "X200").await;
    let quote2 = estimate_and_view(&app, &cookie, &csrf, &second_inputs).await;
    confirm_quote(&app, &cookie, &csrf, &second_inputs.item, &quote2.id).await;
    let body2 = job_body(
        &quote2.id,
        &second_inputs.preparation,
        &photo_ids(&second_inputs),
        (quote2.tripo_upper, quote2.manual_ai_upper),
    );
    let mut handles = Vec::new();
    for index in 0..5 {
        let router: Router = app.router_handle();
        let request = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/v1/items/{}/jobs", second_inputs.item))
            .header("host", "127.0.0.1:8080")
            .header("cookie", &cookie)
            .header("x-csrf-token", &csrf)
            .header("idempotency-key", format!("distinct-key-{index}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body2).unwrap()))
            .unwrap();
        handles.push(tokio::spawn(async move {
            let response = router.oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = http_body_util::BodyExt::collect(response.into_body())
                .await
                .unwrap()
                .to_bytes();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        }));
    }
    let mut accepted = 0;
    for handle in handles {
        let (status, body) = handle.await.unwrap();
        match status {
            StatusCode::ACCEPTED => accepted += 1,
            StatusCode::UNPROCESSABLE_ENTITY => {
                assert_eq!(body["error"]["details"]["reason"], "quoteAlreadyUsed");
            }
            other => panic!("意外状态：{other} {body}"),
        }
    }
    assert_eq!(accepted, 1, "同一报价并发不同 key 只能成功一次");
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 2);
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM cost_ledger").await,
        4,
        "每个 job 恰好 2 笔预留（共 2 个 job）"
    );
}

// ---------------------------------------------------------------------------
// AC-032：过期报价 / 输入变化 / 预算
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expired_quote_is_rejected_and_requires_a_new_estimate() {
    let (app, cookie, csrf) = logged_in_generation_app("quote-expired", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;

    // 服务层用"报价到期之后"的时刻提交（HTTP 层无法篡改冻结的 expires_at）。
    let expires_at: i64 = sqlx::query_scalar("SELECT expires_at FROM quotes")
        .fetch_one(pool(&app))
        .await
        .unwrap();
    let later = Timestamp::from_millis(expires_at + 1_000);
    let mut connection = pool(&app).acquire().await.unwrap();
    let request = everything_manual::http::dto::JobCreateRequest {
        quote_id: Some(quote.id.clone()),
        preparation_id: Some(inputs.preparation.clone()),
        photo_ids: Some(photo_ids(&inputs)),
        limits: Some(everything_manual::http::dto::BudgetLimitsDto {
            tripo_credit_minor: Some(quote.tripo_upper),
            manual_ai_usd_micros: Some(quote.manual_ai_upper),
        }),
    };
    let admin = admin_id(&app).await;
    let error = everything_manual::generation::jobs::create_job(
        app.state().settings(),
        &mut connection,
        &inputs.item,
        &request,
        "expired-key",
        &admin,
        later,
    )
    .await
    .expect_err("过期报价必须被拒绝");
    match error {
        everything_manual::generation::GenerationError::Unprocessable { reason, .. } => {
            assert_eq!(reason, "quoteExpired");
        }
        other => panic!("期望 quoteExpired，实际 {other:?}"),
    }
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);

    // 重新报价 + 重新确认后才可提交（重生成必须新快照 + 新预算确认）。
    let new_quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "fresh-key",
        &job_body(
            &new_quote.id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (new_quote.tripo_upper, new_quote.manual_ai_upper),
        ),
    )
    .await;
    // 未确认仍然被拒（新报价需要新的确认）。
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.json()["error"]["details"]["reason"],
        "confirmationRequired"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
}

#[tokio::test]
async fn input_changes_after_estimate_are_rejected_without_creating_jobs() {
    let (app, cookie, csrf) = logged_in_generation_app("input-changed", TEST_CATALOG).await;

    // 1) 照片换资产（photoId 不变、内容变了）→ inputChanged。
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;
    let front_photo = inputs
        .photos
        .iter()
        .find(|(view, _)| view == "front")
        .map(|(_, id)| id.clone())
        .unwrap();
    let replacement = fixture("sample-photo-left.png");
    let (asset, _) = upload_asset(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        UploadSpec {
            purpose: "photo",
            filename: "front-2.png",
            content_type: "image/png",
            bytes: &replacement,
        },
    )
    .await;
    let patched = app
        .call(
            Method::PATCH,
            &format!("/api/v1/items/{}/photos/{front_photo}", inputs.item),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", "\"r1\"")
        .json(&json!({ "assetId": asset, "view": "front" }))
        .send()
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text());
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "photo-changed",
        &job_body(
            &quote.id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (quote.tripo_upper, quote.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    assert_eq!(
        response.json()["error"]["details"]["reason"],
        "inputChanged"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);

    // 2) 物品型号变化 → inputChanged。
    let inputs2 = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote2 = estimate_and_view(&app, &cookie, &csrf, &inputs2).await;
    confirm_quote(&app, &cookie, &csrf, &inputs2.item, &quote2.id).await;
    let item_revision = app
        .call(Method::GET, &format!("/api/v1/items/{}", inputs2.item))
        .cookie(&cookie)
        .send()
        .await;
    let etag = item_revision.header("etag").unwrap();
    let edit = app
        .call(Method::PATCH, &format!("/api/v1/items/{}", inputs2.item))
        .cookie(&cookie)
        .csrf(&csrf)
        .header("if-match", &etag)
        .json(&json!({ "model": "X100V-改款" }))
        .send()
        .await;
    assert_eq!(edit.status, StatusCode::OK, "{}", edit.text());
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs2.item,
        "model-changed",
        &job_body(
            &quote2.id,
            &inputs2.preparation,
            &photo_ids(&inputs2),
            (quote2.tripo_upper, quote2.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.json()["error"]["details"]["reason"],
        "inputChanged"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);

    // 3) 提交集合与报价不一致（少一张照片）→ inputChanged。
    let inputs3 = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote3 = estimate_and_view(&app, &cookie, &csrf, &inputs3).await;
    confirm_quote(&app, &cookie, &csrf, &inputs3.item, &quote3.id).await;
    let mut subset = photo_ids(&inputs3);
    subset.pop();
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs3.item,
        "subset-changed",
        &job_body(
            &quote3.id,
            &inputs3.preparation,
            &subset,
            (quote3.tripo_upper, quote3.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.json()["error"]["details"]["reason"],
        "inputChanged"
    );
}

#[tokio::test]
async fn price_version_change_invalidates_the_quote() {
    let (app, cookie, csrf) = logged_in_generation_app("price-version", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;

    // 运营者更新价格目录（版本变化）：旧报价不允许继续提交。
    let newer_catalog = TEST_CATALOG.replace("2026-09-11", "2026-10-01");
    let mut settings = app.state().settings().clone();
    settings.price_catalog = Some(catalog::parse(&newer_catalog).unwrap());
    let mut connection = pool(&app).acquire().await.unwrap();
    let request = everything_manual::http::dto::JobCreateRequest {
        quote_id: Some(quote.id.clone()),
        preparation_id: Some(inputs.preparation.clone()),
        photo_ids: Some(photo_ids(&inputs)),
        limits: Some(everything_manual::http::dto::BudgetLimitsDto {
            tripo_credit_minor: Some(quote.tripo_upper),
            manual_ai_usd_micros: Some(quote.manual_ai_upper),
        }),
    };
    let admin = admin_id(&app).await;
    let error = everything_manual::generation::jobs::create_job(
        &settings,
        &mut connection,
        &inputs.item,
        &request,
        "version-key",
        &admin,
        Timestamp::now(),
    )
    .await
    .expect_err("价格版本变化后必须拒绝");
    match error {
        everything_manual::generation::GenerationError::Unprocessable { reason, .. } => {
            assert_eq!(reason, "priceVersionChanged");
        }
        other => panic!("期望 priceVersionChanged，实际 {other:?}"),
    }
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
}

#[tokio::test]
async fn budget_below_server_upper_bound_is_rejected_and_frontend_fees_are_not_accepted() {
    let (app, cookie, csrf) = logged_in_generation_app("budget", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;

    // 1) 授权上限低于服务端保守上界 → 422（不自动降质量/换模型）。
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "budget-low",
        &job_body(
            &quote.id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (quote.tripo_upper - 1, quote.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let error = response.json();
    assert_eq!(
        error["error"]["details"]["reason"],
        "budgetBelowPlannedUpperBound"
    );
    assert_eq!(
        error["error"]["details"]["tripoUpperBoundCreditMinor"],
        TRIPO_UPPER_BOUND
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);

    // 2) 请求体里的费用数值字段不存在：未知字段 → 422（服务端不采信前端费用）。
    let mut body_with_fee = job_body(
        &quote.id,
        &inputs.preparation,
        &photo_ids(&inputs),
        (quote.tripo_upper, quote.manual_ai_upper),
    );
    body_with_fee["tripoCreditMinor"] = json!(1);
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "budget-fee-field",
        &body_with_fee,
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    assert_eq!(response.json()["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);

    // 3) 宽松上限：预留仍等于服务端上界（不按授权值预留）。
    let generous = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "budget-generous",
        &job_body(
            &quote.id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (quote.tripo_upper * 100, quote.manual_ai_upper * 100),
        ),
    )
    .await;
    assert_eq!(generous.status, StatusCode::ACCEPTED, "{}", generous.text());
    let reserved: i64 =
        sqlx::query_scalar("SELECT reserved FROM cost_ledger WHERE provider = 'tripo'")
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert_eq!(
        reserved, TRIPO_UPPER_BOUND,
        "预留取服务端上界，不取前端授权值"
    );
    let budget_authorized: String = sqlx::query_scalar("SELECT budgets FROM generation_snapshots")
        .fetch_one(pool(&app))
        .await
        .unwrap();
    let budgets: Value = serde_json::from_str(&budget_authorized).unwrap();
    assert_eq!(
        budgets["authorized"]["tripoCreditMinor"],
        quote.tripo_upper * 100
    );
    assert_eq!(budgets["upperBound"]["tripoCreditMinor"], TRIPO_UPPER_BOUND);
}

// ---------------------------------------------------------------------------
// 小数换算边界 / 快照冻结 / 阶段 DAG
// ---------------------------------------------------------------------------

#[tokio::test]
async fn decimal_boundary_prices_round_up_never_down() {
    let (app, cookie, csrf) = logged_in_generation_app("decimal-bounds", BOUNDARY_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &photo_ids(&inputs),
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let data = response.json();
    let data = &data["data"];
    // "0.005" credits → 1 creditMinor（向上取整，不低估）。
    assert_eq!(data["amounts"]["tripo"]["upperBoundMinor"], 1);
    assert_eq!(
        data["amounts"]["tripo"]["upperBoundDisplay"],
        "0.01 credits"
    );
    // 单价 0.0000005 USD/1M token → 任意非零用量至少 1 micro。
    let input_line = &data["amounts"]["manualAi"]["upperBoundLines"][0];
    assert_eq!(
        input_line["amountMinor"], 1,
        "0.5 micros 必须进 1 micro：{input_line}"
    );
    let output_line = &data["amounts"]["manualAi"]["upperBoundLines"][1];
    assert_eq!(output_line["amountMinor"], 1);
    // 0.0000001 USD/张（0.1 micro）→ 1 micro/张。
    let image_line = &data["amounts"]["manualAi"]["upperBoundLines"][2];
    assert_eq!(image_line["quantity"], 1);
    assert_eq!(image_line["amountMinor"], 1);
    assert_eq!(data["amounts"]["manualAi"]["upperBoundMinor"], 3);
}

#[tokio::test]
async fn snapshot_freezes_photo_ids_hashes_and_provider_config_without_secrets() {
    let (app, cookie, csrf) = logged_in_generation_app("snapshot", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "snapshot-key",
        &job_body(
            &quote.id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (quote.tripo_upper, quote.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    let job_id = response.json()["data"]["id"].as_str().unwrap().to_owned();

    let (
        snapshot_id,
        item_revision,
        photo_ids_json,
        photo_hashes_json,
        provider_config,
        budgets,
        prompt_version,
        price_version,
    ): (String, i64, String, String, String, String, String, String) = sqlx::query_as(
        "SELECT snapshot_id, s.item_revision, s.photo_ids, s.photo_hashes, s.provider_config, \
                s.budgets, s.prompt_version, s.price_version \
           FROM jobs j JOIN generation_snapshots s ON s.id = j.snapshot_id WHERE j.id = ?",
    )
    .bind(&job_id)
    .fetch_one(pool(&app))
    .await
    .unwrap();

    // 照片 id + 内容哈希一一对应（T07 QA 前置约束：photos 行可变）。
    let ids: Vec<String> = serde_json::from_str(&photo_ids_json).unwrap();
    let hashes: Vec<String> = serde_json::from_str(&photo_hashes_json).unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(hashes.len(), 2);
    for (index, photo_id) in ids.iter().enumerate() {
        let expected: String = sqlx::query_scalar(
            "SELECT b.sha256 FROM photos p JOIN assets a ON a.id = p.asset_id \
              JOIN blobs b ON b.sha256 = a.blob_id WHERE p.id = ?",
        )
        .bind(photo_id)
        .fetch_one(pool(&app))
        .await
        .unwrap();
        assert_eq!(&hashes[index], &expected, "photo {photo_id} 的哈希必须冻结");
    }
    // 槽位顺序：front 在前、left 在后。
    let views: Vec<String> = sqlx::query_scalar(
        "SELECT view FROM photos WHERE id IN (?, ?) ORDER BY CASE view WHEN 'front' THEN 1 ELSE 2 END",
    )
    .bind(&ids[0])
    .bind(&ids[1])
    .fetch_all(pool(&app))
    .await
    .unwrap();
    assert_eq!(views, vec!["front".to_owned(), "left".to_owned()]);
    let front_photo: String =
        sqlx::query_scalar("SELECT id FROM photos WHERE view = 'front' AND item_id = ?")
            .bind(&inputs.item)
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert_eq!(ids[0], front_photo, "front 必须在槽位顺序第一位");

    assert_eq!(item_revision, 1);
    assert_eq!(price_version, "2026-09-11");
    assert_eq!(prompt_version, "manual_extract_v1");
    let provider_config: Value = serde_json::from_str(&provider_config).unwrap();
    assert_eq!(provider_config["tripo"]["model"], TRIPO_MODEL);
    assert_eq!(provider_config["tripo"]["preset"], PRESET);
    assert_eq!(provider_config["manualAi"]["model"], MANUAL_AI_MODEL);
    assert!(
        !provider_config.to_string().contains("canary"),
        "快照不含密钥：{provider_config}"
    );
    let budgets: Value = serde_json::from_str(&budgets).unwrap();
    assert_eq!(budgets["quoteId"], quote.id.as_str());
    assert_eq!(budgets["upperBound"]["tripoCreditMinor"], TRIPO_UPPER_BOUND);
    assert_eq!(
        budgets["upperBound"]["manualAiUsdMicros"],
        quote.manual_ai_upper
    );
    assert_eq!(budgets["authorized"]["tripoCreditMinor"], quote.tripo_upper);

    // 输入不可变：直接 UPDATE 被触发器拒绝（编辑物品不改变已开始任务）。
    let immutable =
        sqlx::query("UPDATE generation_snapshots SET price_version = 'tampered' WHERE id = ?")
            .bind(&snapshot_id)
            .execute(pool(&app))
            .await;
    assert!(immutable.is_err(), "快照必须拒绝 UPDATE");
    let still: String =
        sqlx::query_scalar("SELECT price_version FROM generation_snapshots WHERE id = ?")
            .bind(&snapshot_id)
            .fetch_one(pool(&app))
            .await
            .unwrap();
    assert_eq!(still, "2026-09-11");

    // 建单审计：记录授权与上界，不记录密钥。
    let audit: String = sqlx::query_scalar(
        "SELECT metadata_json FROM audit_events WHERE action = 'generation_job_created'",
    )
    .fetch_one(pool(&app))
    .await
    .unwrap();
    assert!(audit.contains(&quote.id));
    assert!(!audit.contains("canary"));
}

#[tokio::test]
async fn job_creation_builds_the_stage_dag_with_batches() {
    let (app, cookie, csrf) = logged_in_generation_app("stages", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "stages-key",
        &job_body(
            &quote.id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (quote.tripo_upper, quote.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    let job_id = response.json()["data"]["id"].as_str().unwrap().to_owned();

    // 阶段集合：freeze_inputs(succeeded) + 1 个批次（3 页 ≤5）+ 其余 queued。
    // （建单在同一毫秒内完成，行的物理顺序不承载语义；按 stage_kind 断言集合与状态。）
    let stages: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT stage_kind, batch_index, status FROM job_stages WHERE job_id = ?           ORDER BY stage_kind, batch_index",
    )
    .bind(&job_id)
    .fetch_all(pool(&app))
    .await
    .unwrap();
    let kinds: Vec<&str> = stages.iter().map(|(kind, _, _)| kind.as_str()).collect();
    let mut sorted_kinds = kinds.clone();
    sorted_kinds.sort_unstable();
    let mut expected = vec![
        "freeze_inputs",
        "manual_extract",
        "manual_merge",
        "tripo_upload",
        "tripo_submit",
        "tripo_poll",
        "model_download",
        "model_validate",
        "assemble_draft",
    ];
    expected.sort_unstable();
    assert_eq!(sorted_kinds, expected, "{stages:?}");
    let status_of = |kind: &str| {
        stages
            .iter()
            .find(|(stage_kind, _, _)| stage_kind == kind)
            .map(|(_, _, status)| status.as_str())
            .unwrap()
    };
    assert_eq!(
        status_of("freeze_inputs"),
        "succeeded",
        "入队事务里直接成功"
    );
    for kind in [
        "manual_extract",
        "manual_merge",
        "tripo_upload",
        "tripo_submit",
        "tripo_poll",
        "model_download",
        "model_validate",
        "assemble_draft",
    ] {
        assert_eq!(status_of(kind), "queued", "{kind} 应排队");
    }

    // 批次页集合与依赖边（merge → 批次；assemble → merge + model_validate）。
    let page_set: String = sqlx::query_scalar(
        "SELECT page_set FROM job_stages WHERE job_id = ? AND stage_kind = 'manual_extract'",
    )
    .bind(&job_id)
    .fetch_one(pool(&app))
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<i64>>(&page_set).unwrap(),
        vec![1, 2, 3]
    );

    let merge_deps: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM job_stage_deps d JOIN job_stages s ON s.id = d.stage_id \
          WHERE s.job_id = ? AND s.stage_kind = 'manual_merge'",
    )
    .bind(&job_id)
    .fetch_one(pool(&app))
    .await
    .unwrap();
    assert_eq!(merge_deps, 1, "merge 依赖全部批次（当前 1 批）");
    let assemble_deps: Vec<String> = sqlx::query_scalar(
        "SELECT dep.stage_kind FROM job_stage_deps d \
           JOIN job_stages s ON s.id = d.stage_id \
           JOIN job_stages dep ON dep.id = d.depends_on_stage_id \
          WHERE s.job_id = ? AND s.stage_kind = 'assemble_draft' ORDER BY dep.stage_kind",
    )
    .bind(&job_id)
    .fetch_all(pool(&app))
    .await
    .unwrap();
    assert_eq!(assemble_deps, vec!["manual_merge", "model_validate"]);

    // 任务详情里的费用预留分列（credits 与 USD，各自带单位，不相加）。
    let data = response.json();
    let reservations = data["data"]["reservations"].as_array().unwrap();
    assert_eq!(reservations[0]["provider"], "tripo");
    assert_eq!(reservations[0]["currency"], "creditMinor");
    assert_eq!(reservations[0]["reservedMinor"], TRIPO_UPPER_BOUND);
    assert_eq!(reservations[0]["reservedDisplay"], "30.00 credits");
    assert_eq!(reservations[0]["state"], "reserved");
    assert_eq!(reservations[1]["provider"], "manual_ai");
    assert_eq!(reservations[1]["currency"], "usdMicros");
    assert_eq!(reservations[1]["reservedMinor"], quote.manual_ai_upper);
    assert!(
        reservations[1]["reservedDisplay"]
            .as_str()
            .unwrap()
            .ends_with(" USD")
    );
}

// ---------------------------------------------------------------------------
// 账本：结算 / 释放 / unknown 保留预留
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ledger_settlement_release_and_unknown_keep_reservations_intact() {
    use everything_manual::generation::ledger as ledger_service;
    use everything_manual::storage::repo::ledger::LedgerOutcome;

    let (app, cookie, csrf) = logged_in_generation_app("ledger", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "ledger-key",
        &job_body(
            &quote.id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (quote.tripo_upper, quote.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    let ledger_job_id = response.json()["data"]["id"].as_str().unwrap().to_owned();

    let snapshot_id: String = sqlx::query_scalar("SELECT snapshot_id FROM jobs WHERE id = ?")
        .bind(&ledger_job_id)
        .fetch_one(pool(&app))
        .await
        .unwrap();
    let mut connection = pool(&app).acquire().await.unwrap();
    let entries = ledger::list_for_snapshot(&mut connection, &snapshot_id)
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    let tripo_entry = entries
        .iter()
        .find(|entry| entry.provider == manual_core::domain::ProviderKey::Tripo)
        .unwrap()
        .clone();
    let manual_entry = entries
        .iter()
        .find(|entry| entry.provider == manual_core::domain::ProviderKey::ManualAi)
        .unwrap()
        .clone();
    assert_eq!(tripo_entry.reserved, TRIPO_UPPER_BOUND);
    assert_eq!(manual_entry.reserved, quote.manual_ai_upper);
    assert!(tripo_entry.actual.is_none() && manual_entry.actual.is_none());

    // 与 T10 执行器对接：为 tripo_submit 造 attempt，标记 unknown → 保留预留（actual 仍 NULL）。
    let submit_stage_id: String =
        sqlx::query_scalar("SELECT id FROM job_stages WHERE stage_kind = 'tripo_submit'")
            .fetch_one(pool(&app))
            .await
            .unwrap();
    let attempt = attempts_repo::create_intent(
        &mut connection,
        attempts_repo::NewAttempt {
            job_id: ledger_job_id.clone(),
            stage_id: submit_stage_id.clone(),
            request_hash: "attempt-hash".to_owned(),
        },
        Timestamp::now(),
    )
    .await
    .unwrap();
    assert!(
        attempts_repo::mark_submitting(&mut connection, &attempt.id, Timestamp::now())
            .await
            .unwrap()
    );

    let outcome = ledger_service::mark_submission_unknown(
        &mut connection,
        &tripo_entry.id,
        Some(&attempt.id),
        Timestamp::now(),
    )
    .await
    .unwrap();
    assert_eq!(outcome, LedgerOutcome::Applied);
    let after = ledger::get(&mut connection, &tripo_entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.state, manual_core::domain::LedgerState::Unknown);
    assert_eq!(after.actual, None, "unknown 不得把实际费用填 0");
    assert_eq!(after.reserved, TRIPO_UPPER_BOUND, "unknown 保留预留");
    assert_eq!(after.attempt_id.as_deref(), Some(attempt.id.as_str()));
    assert!(ledger_service::holds_budget(&after));

    // unknown 幂等 + 自动路径不释放 unknown。
    let repeat = ledger_service::mark_submission_unknown(
        &mut connection,
        &tripo_entry.id,
        None,
        Timestamp::now(),
    )
    .await
    .unwrap();
    assert_eq!(repeat, LedgerOutcome::Idempotent);
    let auto_release = ledger_service::release_definitely_not_billed(
        &mut connection,
        &tripo_entry.id,
        Timestamp::now(),
    )
    .await
    .unwrap();
    assert!(
        matches!(auto_release, LedgerOutcome::Rejected { .. }),
        "自动路径不得释放 unknown：{auto_release:?}"
    );
    assert_eq!(
        ledger::get(&mut connection, &tripo_entry.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        manual_core::domain::LedgerState::Unknown
    );

    // 管理员对账后才允许释放（T15 的 recordNoTask 入口）。
    let reconciled = ledger_service::release_after_reconciliation(
        &mut connection,
        &tripo_entry.id,
        Timestamp::now(),
    )
    .await
    .unwrap();
    assert_eq!(reconciled, LedgerOutcome::Applied);
    assert_eq!(
        ledger::get(&mut connection, &tripo_entry.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        manual_core::domain::LedgerState::Released
    );

    // 结算幂等且不覆盖事实：先按实际 2800 结算，再重复同值是幂等、改值被拒。
    let settled =
        ledger_service::settle_attempt(&mut connection, &manual_entry.id, 15_000, Timestamp::now())
            .await
            .unwrap();
    assert_eq!(settled, LedgerOutcome::Applied);
    let idempotent =
        ledger_service::settle_attempt(&mut connection, &manual_entry.id, 15_000, Timestamp::now())
            .await
            .unwrap();
    assert_eq!(idempotent, LedgerOutcome::Idempotent);
    let conflicting =
        ledger_service::settle_attempt(&mut connection, &manual_entry.id, 1, Timestamp::now())
            .await
            .unwrap();
    assert!(matches!(conflicting, LedgerOutcome::Rejected { .. }));
    let final_entry = ledger::get(&mut connection, &manual_entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_entry.state, manual_core::domain::LedgerState::Settled);
    assert_eq!(final_entry.actual, Some(15_000), "已结算金额不被改写");
    assert!(!ledger_service::holds_budget(&final_entry));

    // 明确未计费的失败：自动释放 reserved 允许。
    let second_inputs = build_ready_inputs(&app, &cookie, &csrf, "X200").await;
    let quote2 = estimate_and_view(&app, &cookie, &csrf, &second_inputs).await;
    confirm_quote(&app, &cookie, &csrf, &second_inputs.item, &quote2.id).await;
    let second_job = create_job(
        &app,
        &cookie,
        &csrf,
        &second_inputs.item,
        "ledger-key-2",
        &job_body(
            &quote2.id,
            &second_inputs.preparation,
            &photo_ids(&second_inputs),
            (quote2.tripo_upper, quote2.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(
        second_job.status,
        StatusCode::ACCEPTED,
        "{}",
        second_job.text()
    );
    let snapshot2: String =
        sqlx::query_scalar("SELECT snapshot_id FROM jobs j WHERE j.item_id = ?")
            .bind(&second_inputs.item)
            .fetch_one(pool(&app))
            .await
            .unwrap();
    let entries2 = ledger::list_for_snapshot(&mut connection, &snapshot2)
        .await
        .unwrap();
    let released = ledger_service::release_definitely_not_billed(
        &mut connection,
        &entries2[0].id,
        Timestamp::now(),
    )
    .await
    .unwrap();
    assert_eq!(released, LedgerOutcome::Applied);
    let released_again = ledger_service::release_definitely_not_billed(
        &mut connection,
        &entries2[0].id,
        Timestamp::now(),
    )
    .await
    .unwrap();
    assert_eq!(released_again, LedgerOutcome::Idempotent, "释放幂等");

    // 重放 20 次不产生第二次预留（已有 job 的重放路径不再写账本）。
    let replay = create_job(
        &app,
        &cookie,
        &csrf,
        &second_inputs.item,
        "ledger-key-2",
        &job_body(
            &quote2.id,
            &second_inputs.preparation,
            &photo_ids(&second_inputs),
            (quote2.tripo_upper, quote2.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(replay.status, StatusCode::ACCEPTED);
    for _ in 0..19 {
        let replay = create_job(
            &app,
            &cookie,
            &csrf,
            &second_inputs.item,
            "ledger-key-2",
            &job_body(
                &quote2.id,
                &second_inputs.preparation,
                &photo_ids(&second_inputs),
                (quote2.tripo_upper, quote2.manual_ai_upper),
            ),
        )
        .await;
        assert_eq!(replay.status, StatusCode::ACCEPTED);
    }
    let ledger_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cost_ledger l JOIN generation_snapshots s ON s.id = l.snapshot_id \
          WHERE s.id = ?",
    )
    .bind(&snapshot2)
    .fetch_one(pool(&app))
    .await
    .unwrap();
    assert_eq!(ledger_rows, 2, "重放不得产生第二次预留");
}

// ---------------------------------------------------------------------------
// 跨物品引用与 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cross_item_references_and_unknown_ids_are_rejected_as_404() {
    let (app, cookie, csrf) = logged_in_generation_app("cross-item", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let other = build_ready_inputs(&app, &cookie, &csrf, "X200").await;

    // 其他物品的照片 → 404（不泄露存在性，与 T07 语义一致）。
    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &photo_ids(&other),
        PRESET,
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::NOT_FOUND,
        "{}",
        response.text()
    );

    // 其他物品的准备记录 → 404。
    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &other.preparation,
        &photo_ids(&inputs),
        PRESET,
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::NOT_FOUND,
        "{}",
        response.text()
    );

    // 其他物品的报价 → 404（跨物品与不存在同响应）。
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &other.item,
        "cross-item-key",
        &job_body(
            &quote.id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (quote.tripo_upper, quote.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::NOT_FOUND,
        "{}",
        response.text()
    );

    // 未知物品 → 404；未知报价 → 404（请求形状合法时才轮到存在性判定）。
    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        "不存在的物品",
        &inputs.preparation,
        &photo_ids(&inputs),
        PRESET,
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::NOT_FOUND,
        "{}",
        response.text()
    );
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "unknown-quote-key",
        &job_body(
            "不存在的报价",
            &inputs.preparation,
            &photo_ids(&inputs),
            (quote.tripo_upper, quote.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);
}

/// 报价/建单路由同样受会话与 CSRF 保护（未登录 401 先于 CSRF；缺 CSRF 403）。
#[tokio::test]
async fn generation_routes_require_authentication_and_csrf() {
    let (app, cookie, csrf) = logged_in_generation_app("auth-guard", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    let body = job_body(
        &quote.id,
        &inputs.preparation,
        &photo_ids(&inputs),
        (quote.tripo_upper, quote.manual_ai_upper),
    );

    let estimate_body = json!({
        "preparationId": inputs.preparation,
        "photoIds": photo_ids(&inputs),
        "modelPreset": PRESET,
    });
    let unauthenticated = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{}/estimates", inputs.item),
        )
        .json(&estimate_body)
        .send()
        .await;
    unauthenticated.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");

    let unauthenticated_job = app
        .call(Method::POST, &format!("/api/v1/items/{}/jobs", inputs.item))
        .header("idempotency-key", "no-session")
        .json(&body)
        .send()
        .await;
    unauthenticated_job.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");

    // 有会话但缺 CSRF：403（建单是修改请求）。
    let no_csrf = app
        .call(Method::POST, &format!("/api/v1/items/{}/jobs", inputs.item))
        .cookie(&cookie)
        .header("idempotency-key", "no-csrf")
        .json(&body)
        .send()
        .await;
    no_csrf.assert_contract_error(StatusCode::FORBIDDEN, "CSRF_REJECTED");

    // 未授权路径不产生任何记录（报价仍未被确认、未被消费）。
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM cost_ledger").await, 0);
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM quotes WHERE confirmed_at IS NOT NULL"
        )
        .await,
        0
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM audit_events").await, 0);
}

/// 多批次（7 页 → 2 批）：`manual_merge` 的依赖边必须指向全部批次（T10 建单顺序约束）。
#[tokio::test]
async fn multi_batch_estimate_and_stage_dependencies_cover_all_batches() {
    let (app, cookie, csrf) = logged_in_generation_app("multi-batch", TEST_CATALOG).await;
    let pages = [Some(3000), None, Some(120), None, Some(10), Some(20), None];
    let inputs = build_ready_inputs_with_pages(&app, &cookie, &csrf, "X100V", &pages).await;

    let response = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &photo_ids(&inputs),
        PRESET,
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let data = response.json();
    let data = &data["data"];
    // 2 批：输出上界 = 2 × 4096；页图页 = 扫描页（2/4/7）。
    assert_eq!(data["maxOutputTokens"], 8192);
    assert_eq!(
        data["sendScope"]["manualAi"]["textPages"],
        json!([1, 3, 5, 6])
    );
    assert_eq!(
        data["sendScope"]["manualAi"]["imagePages"],
        json!([2, 4, 7])
    );

    let quote = QuoteView {
        id: data["id"].as_str().unwrap().to_owned(),
        tripo_upper: data["amounts"]["tripo"]["upperBoundMinor"]
            .as_i64()
            .unwrap(),
        manual_ai_upper: data["amounts"]["manualAi"]["upperBoundMinor"]
            .as_i64()
            .unwrap(),
    };
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;
    let job = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "multi-batch-key",
        &job_body(
            &quote.id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (quote.tripo_upper, quote.manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(job.status, StatusCode::ACCEPTED, "{}", job.text());
    let job_id = job.json()["data"]["id"].as_str().unwrap().to_owned();

    let batches: Vec<(i64, String)> = sqlx::query_as(
        "SELECT batch_index, page_set FROM job_stages           WHERE job_id = ? AND stage_kind = 'manual_extract' ORDER BY batch_index",
    )
    .bind(&job_id)
    .fetch_all(pool(&app))
    .await
    .unwrap();
    assert_eq!(batches.len(), 2, "7 页应展开为 2 批");
    assert_eq!(
        serde_json::from_str::<Vec<i64>>(&batches[0].1).unwrap(),
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(
        serde_json::from_str::<Vec<i64>>(&batches[1].1).unwrap(),
        vec![6, 7]
    );
    let merge_deps: Vec<String> = sqlx::query_scalar(
        "SELECT dep.page_set FROM job_stage_deps d            JOIN job_stages s ON s.id = d.stage_id            JOIN job_stages dep ON dep.id = d.depends_on_stage_id           WHERE s.job_id = ? AND s.stage_kind = 'manual_merge' ORDER BY dep.batch_index",
    )
    .bind(&job_id)
    .fetch_all(pool(&app))
    .await
    .unwrap();
    assert_eq!(
        merge_deps.len(),
        2,
        "merge 必须依赖全部批次：{merge_deps:?}"
    );
}

/// 幂等键缺失 → 422 字段级（且不创建任何记录）。
#[tokio::test]
async fn missing_idempotency_key_is_rejected() {
    let (app, cookie, csrf) = logged_in_generation_app("missing-key", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let quote = estimate_and_view(&app, &cookie, &csrf, &inputs).await;
    confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote.id).await;
    let body = job_body(
        &quote.id,
        &inputs.preparation,
        &photo_ids(&inputs),
        (quote.tripo_upper, quote.manual_ai_upper),
    );
    let response = app
        .call(Method::POST, &format!("/api/v1/items/{}/jobs", inputs.item))
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&body)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let fields: Vec<String> = response.json()["error"]["details"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["field"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(fields, vec!["idempotencyKey"]);
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);
}

/// `/settings/status` 的内容与 estimate/jobs 的错误码保持一致（AC-012/AC-029）。
#[tokio::test]
async fn settings_status_reports_price_catalog_and_generation_capability() {
    let (app, cookie, _csrf) = logged_in_generation_app("settings-catalog", TEST_CATALOG).await;
    let response = app
        .call(Method::GET, "/api/v1/settings/status")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let data = response.json();
    assert_eq!(data["data"]["priceCatalog"]["configured"], true);
    assert_eq!(data["data"]["priceCatalog"]["version"], "2026-09-11");
    assert_eq!(data["data"]["priceCatalog"]["snapshotDate"], "2026-09-11");
    assert_eq!(data["data"]["capabilities"]["generation"], true);
    assert!(
        !response.text().contains("canary"),
        "设置状态不得泄露密钥：{}",
        response.text()
    );

    // 未配置价格目录时：configured=false、generation=false（与 409 保持一致）。
    let dir = TestDir::new("settings-no-catalog");
    let settings = common::test_settings(dir.path());
    let app2 = TestApp::with_settings(dir, settings).await;
    app2.set_admin_password(PASSWORD).await;
    let (cookie2, _) = login(&app2).await;
    let response = app2
        .call(Method::GET, "/api/v1/settings/status")
        .cookie(&cookie2)
        .send()
        .await;
    assert_eq!(response.json()["data"]["priceCatalog"]["configured"], false);
    assert_eq!(response.json()["data"]["capabilities"]["generation"], false);
}

// ---------------------------------------------------------------------------
// BUG-004 修复回归：GET 回读必须反映确认/消费/过期事实（REQ-020/021/022 回读语义）
// ---------------------------------------------------------------------------

/// `GET /items/{id}/estimates/{quoteId}`（回读不携带 CSRF）。
async fn read_estimate(app: &TestApp, cookie: &str, item: &str, quote_id: &str) -> TestResponse {
    app.call(
        Method::GET,
        &format!("/api/v1/items/{item}/estimates/{quote_id}"),
    )
    .cookie(cookie)
    .send()
    .await
}

/// 读回 `quotes` 表的确认/消费列（回读断言必须对照 DB 事实，不能只看响应）。
async fn db_quote_status(
    app: &TestApp,
    quote_id: &str,
) -> (Option<i64>, Option<i64>, Option<String>) {
    sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<String>)>(
        "SELECT confirmed_at, consumed_at, consumed_job_id FROM quotes WHERE id = ?",
    )
    .bind(quote_id)
    .fetch_one(pool(app))
    .await
    .unwrap()
}

/// 服务层创建报价（可传入过去时刻生成"已过期"报价；HTTP 层无法篡改冻结的 expiresAt）。
async fn create_estimate_at(app: &TestApp, inputs: &ReadyInputs, now: Timestamp) -> String {
    let mut connection = pool(app).acquire().await.unwrap();
    let request = everything_manual::http::dto::EstimateRequest {
        preparation_id: Some(inputs.preparation.clone()),
        photo_ids: Some(photo_ids(inputs)),
        model_preset: Some(PRESET.to_owned()),
    };
    everything_manual::generation::estimate::create_estimate(
        app.state().settings(),
        &mut connection,
        &inputs.item,
        &request,
        now,
    )
    .await
    .expect("服务层创建报价")
    .id
}

/// BUG-004：回读的 `confirmedAt`/`consumedAt`/`consumedJobId` 必须来自持久层当前值。
///
/// 三种活跃状态逐个转换后回读（只断言创建态 null 会漏掉"永远返回冻结载荷"的缺陷）：
/// 未确认未消费 → 已确认未消费 → 已消费；每一步都同时对照 DB 列，并确认冻结输入部分
/// （金额/上界/价格版本/expiresAt）与落库 `quote_json` 不被回读路径改写。
#[tokio::test]
async fn get_estimate_reflects_confirmation_and_consumption_from_database() {
    let (app, cookie, csrf) = logged_in_generation_app("get-quote-state", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;

    // 创建报价（HTTP 201），保留创建响应作为"冻结载荷"基准。
    let created = create_estimate(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        &inputs.preparation,
        &photo_ids(&inputs),
        PRESET,
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let created = created.json();
    let quote = &created["data"];
    let quote_id = quote["id"].as_str().unwrap().to_owned();
    let tripo_upper = quote["amounts"]["tripo"]["upperBoundMinor"]
        .as_i64()
        .unwrap();
    let manual_ai_upper = quote["amounts"]["manualAi"]["upperBoundMinor"]
        .as_i64()
        .unwrap();

    // 状态 1｜未确认未消费：三字段为 null，与 DB 列一致；冻结输入部分与创建响应同源。
    let (db_confirmed, db_consumed, db_job) = db_quote_status(&app, &quote_id).await;
    assert!(db_confirmed.is_none() && db_consumed.is_none() && db_job.is_none());
    let fetched = read_estimate(&app, &cookie, &inputs.item, &quote_id).await;
    assert_eq!(fetched.status, StatusCode::OK, "{}", fetched.text());
    let data = fetched.json();
    let data = &data["data"];
    assert!(data["confirmedAt"].is_null(), "不得默认确认：{data}");
    assert!(data["consumedAt"].is_null(), "{data}");
    assert!(data["consumedJobId"].is_null(), "{data}");
    assert_eq!(data["amounts"], quote["amounts"]);
    assert_eq!(data["expiresAt"], quote["expiresAt"]);
    assert_eq!(data["priceVersion"], quote["priceVersion"]);
    assert_eq!(data["sendScope"], quote["sendScope"]);

    // 状态 2｜已确认未消费：回读的 confirmedAt = 确认响应 = DB 列；消费仍为 null。
    let confirmed = confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.text());
    let confirmed_at = confirmed.json()["data"]["confirmedAt"].clone();
    assert!(confirmed_at.is_string(), "{confirmed_at}");
    let fetched = read_estimate(&app, &cookie, &inputs.item, &quote_id).await;
    assert_eq!(fetched.status, StatusCode::OK, "{}", fetched.text());
    let data = fetched.json();
    let data = &data["data"];
    assert_eq!(
        data["confirmedAt"], confirmed_at,
        "确认后回读必须反映 DB 事实（BUG-004）"
    );
    assert!(data["consumedAt"].is_null(), "{data}");
    assert!(data["consumedJobId"].is_null(), "{data}");
    let (db_confirmed, db_consumed, db_job) = db_quote_status(&app, &quote_id).await;
    assert_eq!(
        Timestamp::from_millis(db_confirmed.expect("确认必须落库")).to_rfc3339(),
        confirmed_at.as_str().unwrap(),
        "回读值与 DB 列必须逐字一致"
    );
    assert!(db_consumed.is_none() && db_job.is_none());

    // 状态 3｜已消费：consumedAt/consumedJobId = DB 列 = 真实 job；确认时间不变。
    let response = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "get-quote-state-key",
        &job_body(
            &quote_id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (tripo_upper, manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    let job_id = response.json()["data"]["id"].as_str().unwrap().to_owned();
    let fetched = read_estimate(&app, &cookie, &inputs.item, &quote_id).await;
    assert_eq!(fetched.status, StatusCode::OK, "{}", fetched.text());
    let data = fetched.json();
    let data = &data["data"];
    assert_eq!(data["confirmedAt"], confirmed_at, "消费不改变确认时间");
    assert_eq!(
        data["consumedJobId"].as_str(),
        Some(job_id.as_str()),
        "消费后回读必须给出消费它的任务 id（BUG-004）"
    );
    let (db_confirmed, db_consumed, db_job) = db_quote_status(&app, &quote_id).await;
    assert_eq!(
        Timestamp::from_millis(db_consumed.expect("消费必须落库")).to_rfc3339(),
        data["consumedAt"].as_str().unwrap(),
        "回读的 consumedAt 与 DB 列必须逐字一致"
    );
    assert_eq!(db_job.as_deref(), Some(job_id.as_str()));
    assert_eq!(
        Timestamp::from_millis(db_confirmed.unwrap()).to_rfc3339(),
        confirmed_at.as_str().unwrap()
    );
    // 冻结载荷未被回读路径回写：quote_json 仍等于创建响应（状态字段仍为 null）。
    let stored: String = sqlx::query_scalar("SELECT quote_json FROM quotes WHERE id = ?")
        .bind(&quote_id)
        .fetch_one(pool(&app))
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stored).unwrap(),
        *quote,
        "回读不得改写 quote_json（冻结语义）"
    );
    // 回读不影响可用性判定：已消费报价再提交（新键）仍被拒，不产生第二个 job。
    let again = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "get-quote-state-key-2",
        &job_body(
            &quote_id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (tripo_upper, manual_ai_upper),
        ),
    )
    .await;
    assert_eq!(again.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        again.json()["error"]["details"]["reason"],
        "quoteAlreadyUsed"
    );
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 1);
}

/// BUG-004｜过期边界：回读如实返回状态字段（不把"过期"与"未确认/未消费"混淆），
/// 报价是否可用仍由 `POST jobs` / `confirm` 在服务端判定（过期即拒绝）。
#[tokio::test]
async fn get_expired_quote_reports_status_facts_but_stays_unusable() {
    let (app, cookie, csrf) = logged_in_generation_app("get-quote-expired", TEST_CATALOG).await;
    let inputs = build_ready_inputs(&app, &cookie, &csrf, "X100V").await;
    let admin = admin_id(&app).await;

    // (a) 已过期、从未确认/消费：服务层用 700 秒前的时刻报价（HTTP 无法篡改 expiresAt）。
    let past = Timestamp::now().checked_add_millis(-700_000).unwrap();
    let quote_id = create_estimate_at(&app, &inputs, past).await;
    let expires_at_db: i64 = sqlx::query_scalar("SELECT expires_at FROM quotes WHERE id = ?")
        .bind(&quote_id)
        .fetch_one(pool(&app))
        .await
        .unwrap();
    assert!(
        expires_at_db < Timestamp::now().as_millis(),
        "前置：报价必须已过期"
    );
    let fetched = read_estimate(&app, &cookie, &inputs.item, &quote_id).await;
    assert_eq!(fetched.status, StatusCode::OK, "{}", fetched.text());
    let data = fetched.json();
    let data = &data["data"];
    assert!(
        data["confirmedAt"].is_null(),
        "未确认不能因过期而变值：{data}"
    );
    assert!(data["consumedAt"].is_null(), "{data}");
    assert!(data["consumedJobId"].is_null(), "{data}");
    assert_eq!(
        data["expiresAt"].as_str().unwrap(),
        Timestamp::from_millis(expires_at_db).to_rfc3339(),
        "回读的 expiresAt 必须等于 DB 冻结值"
    );
    // 可用性由服务端在动作时判定：过期报价确认/提交均被拒，状态列仍为 null。
    let denied = confirm_quote(&app, &cookie, &csrf, &inputs.item, &quote_id).await;
    assert_eq!(denied.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        denied.json()["error"]["details"]["reason"],
        "quoteExpired",
        "{}",
        denied.text()
    );
    let denied_job = create_job(
        &app,
        &cookie,
        &csrf,
        &inputs.item,
        "expired-readback-key",
        &job_body(
            &quote_id,
            &inputs.preparation,
            &photo_ids(&inputs),
            (10_000, 100_000),
        ),
    )
    .await;
    assert_eq!(denied_job.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        denied_job.json()["error"]["details"]["reason"],
        "quoteExpired"
    );
    let (db_confirmed, db_consumed, db_job) = db_quote_status(&app, &quote_id).await;
    assert!(db_confirmed.is_none() && db_consumed.is_none() && db_job.is_none());
    assert_eq!(count(&app, "SELECT COUNT(*) FROM jobs").await, 0);

    // (b) 过期前已确认、现在已过期：回读仍必须给出确认时间（事实不因过期而消失）。
    let quote_id = create_estimate_at(&app, &inputs, past).await;
    let mut connection = pool(&app).acquire().await.unwrap();
    everything_manual::generation::estimate::confirm_quote(
        &mut connection,
        &inputs.item,
        &quote_id,
        &admin,
        past.checked_add_millis(120_000).unwrap(),
    )
    .await
    .expect("过期前的确认必须成功（服务层注入过期前时刻）");
    let (db_confirmed, _, _) = db_quote_status(&app, &quote_id).await;
    let fetched = read_estimate(&app, &cookie, &inputs.item, &quote_id).await;
    assert_eq!(fetched.status, StatusCode::OK, "{}", fetched.text());
    let data = fetched.json();
    assert_eq!(
        data["data"]["confirmedAt"].as_str().unwrap(),
        Timestamp::from_millis(db_confirmed.expect("确认必须落库")).to_rfc3339(),
        "已确认但过期的报价回读仍必须反映确认事实（BUG-004）"
    );
    assert!(data["data"]["consumedAt"].is_null(), "{data}");
}
