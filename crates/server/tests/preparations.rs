//! T09 集成测试：浏览器 PDF 准备、逐页上传与封存（PRD 修订 2 / ui_revision 2）。
//!
//! 覆盖的验收条件：
//! - **AC-022（服务端侧）**：`PUT /preparations/{id}/pages/{n}` 使用 **1-based** 页号，
//!   页文字/页图资产按 purpose 校验归属；相同内容（blob sha256 + viewport）重复 PUT 幂等
//!   （不自增 revision、不产生第二行）；`GET /preparations/{id}` 反映已完成页与 revision。
//! - **AC-024（服务端侧）**：页号与 `pageCount` 的 >100 页上限被权威拒绝
//!   （422 `details.reason=pageLimitExceeded`）；拒绝路径不创建 job / 费用记录。
//!   （加密 PDF 与"真实页数"只有浏览器解析器能判定，权威拒绝在 e2e，见 ADR-003。）
//! - **AC-025**：`complete` 需要 `If-Match` + `pageCount`，事务内校验页号连续 1..N、
//!   资产归属与内容可用；缺页/内容不符 → 422 并列出缺项；成功后标记 `clientDerived`
//!   并保留原件；ready 后写入被拒；**complete 不创建 job、不产生费用**。
//!
//! 全部用例使用临时 data-dir + 真实 SQLite，无外部网络调用；样例资产来自
//! `tests/fixtures/assets/`（T05 原创、sha256 固定）；假凭据。

mod common;

use std::path::Path;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestResponse};
use manual_core::validation::MAX_PDF_PAGES;
use serde_json::json;

const PASSWORD: &str = "test-password-prep-9f21";

// ---------------------------------------------------------------------------
// 通用工具（与 items.rs 同风格；测试二进制之间不共享代码）
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("读取样例资产 {} 失败：{error}", path.display()))
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

async fn logged_in_app(tag: &str) -> (TestApp, String, String) {
    let app = TestApp::new(tag).await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    (app, cookie, csrf)
}

async fn create_item(app: &TestApp, cookie: &str, csrf: &str, name: &str) -> String {
    let response = app
        .call(Method::POST, "/api/v1/items")
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "name": name, "model": format!("{name}-model") }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()["data"]["id"].as_str().unwrap().to_owned()
}

struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new(tag: &str) -> Self {
        Self {
            boundary: format!("----em-prep-{tag}"),
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

/// 上传资产（purpose 决定服务端校验分支），返回 `(assetId, sha256)`。
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

/// 绑定说明书原件（PDF asset → document），返回 `(documentId, sourceSha256)`。
async fn bind_document(app: &TestApp, cookie: &str, csrf: &str, item: &str) -> (String, String) {
    let pdf = fixture("sample-manual-text.pdf");
    let (asset, sha) = upload_asset(
        app,
        cookie,
        csrf,
        item,
        UploadSpec {
            purpose: "document",
            filename: "manual.pdf",
            content_type: "application/pdf",
            bytes: &pdf,
        },
    )
    .await;
    let response = app
        .call(Method::POST, &format!("/api/v1/items/{item}/documents"))
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "sourceAssetId": asset, "title": "样例说明书" }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    (
        response.json()["data"]["id"].as_str().unwrap().to_owned(),
        sha,
    )
}

/// 上传一张页图（JPEG），每次调用产生一个新的 asset 行（blob 内容去重）。
async fn upload_page_image(app: &TestApp, cookie: &str, csrf: &str, item: &str) -> String {
    let jpeg = fixture("sample-photo-front.jpg");
    upload_asset(
        app,
        cookie,
        csrf,
        item,
        UploadSpec {
            purpose: "pageImage",
            filename: "page.jpg",
            content_type: "image/jpeg",
            bytes: &jpeg,
        },
    )
    .await
    .0
}

async fn upload_page_text(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    text: &str,
) -> String {
    upload_asset(
        app,
        cookie,
        csrf,
        item,
        UploadSpec {
            purpose: "pageText",
            filename: "page.txt",
            content_type: "text/plain",
            bytes: text.as_bytes(),
        },
    )
    .await
    .0
}

/// `POST /documents/{id}/preparations`。
async fn create_preparation(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    document: &str,
    source_sha256: &str,
) -> TestResponse {
    app.call(
        Method::POST,
        &format!("/api/v1/documents/{document}/preparations"),
    )
    .cookie(cookie)
    .csrf(csrf)
    .json(&json!({ "sourceSha256": source_sha256 }))
    .send()
    .await
}

/// `PUT /preparations/{id}/pages/{n}`。
async fn put_page(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    preparation: &str,
    page_number: i64,
    body: &serde_json::Value,
    if_match: Option<&str>,
) -> TestResponse {
    let mut builder = app
        .call(
            Method::PUT,
            &format!("/api/v1/preparations/{preparation}/pages/{page_number}"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .json(body);
    if let Some(if_match) = if_match {
        builder = builder.header("if-match", if_match);
    }
    builder.send().await
}

async fn get_preparation(app: &TestApp, cookie: &str, preparation: &str) -> TestResponse {
    app.call(Method::GET, &format!("/api/v1/preparations/{preparation}"))
        .cookie(cookie)
        .send()
        .await
}

async fn complete_preparation(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    preparation: &str,
    page_count: i64,
    if_match: Option<&str>,
) -> TestResponse {
    let mut builder = app
        .call(
            Method::POST,
            &format!("/api/v1/preparations/{preparation}/complete"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "pageCount": page_count }));
    if let Some(if_match) = if_match {
        builder = builder.header("if-match", if_match);
    }
    builder.send().await
}

fn viewport(width: u32, height: u32, rotation: u16) -> serde_json::Value {
    json!({ "width": width, "height": height, "rotation": rotation })
}

fn page_body(
    text_asset_id: Option<&str>,
    image_asset_id: Option<&str>,
    viewport: serde_json::Value,
) -> serde_json::Value {
    let mut body = json!({ "viewport": viewport });
    if let Some(text) = text_asset_id {
        body["textAssetId"] = json!(text);
    }
    if let Some(image) = image_asset_id {
        body["imageAssetId"] = json!(image);
    }
    body
}

/// 直接查询某表的行数（用于断言"没有创建 job / 费用记录"）。
///
/// 只接受写死的表名（消除动态 SQL 注入面），与生产代码的"不拼用户字符串"约定一致。
async fn table_count(app: &TestApp, table: TableName) -> i64 {
    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    let sql: &'static str = match table {
        TableName::Jobs => "SELECT COUNT(*) FROM jobs",
        TableName::JobStages => "SELECT COUNT(*) FROM job_stages",
        TableName::CostLedger => "SELECT COUNT(*) FROM cost_ledger",
        TableName::ProviderAttempts => "SELECT COUNT(*) FROM provider_attempts",
    };
    sqlx::query_scalar(sql)
        .fetch_one(&mut *connection)
        .await
        .expect("统计行数")
}

/// 测试用表名白名单。
#[derive(Debug, Clone, Copy)]
enum TableName {
    Jobs,
    JobStages,
    CostLedger,
    ProviderAttempts,
}

/// 完成一次"item + document + preparing preparation"的最小前置流程。
async fn preparation_fixture(tag: &str) -> (TestApp, String, String, String, String, String) {
    let (app, cookie, csrf) = logged_in_app(tag).await;
    let item = create_item(&app, &cookie, &csrf, "样例仪器").await;
    let (document, sha) = bind_document(&app, &cookie, &csrf, &item).await;
    let response = create_preparation(&app, &cookie, &csrf, &document, &sha).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let preparation = response.json()["data"]["id"].as_str().unwrap().to_owned();
    (app, cookie, csrf, item, document, preparation)
}

// ---------------------------------------------------------------------------
// 创建与复用
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_preparation_is_one_per_document_until_ready() {
    let (app, cookie, csrf, item, document, preparation) = preparation_fixture("create").await;
    let _ = item;

    // 同一原件再次创建 → 复用未完成记录（200，同一 id）。
    let pdf = fixture("sample-manual-text.pdf");
    let sha = test_support::sha256_hex(&pdf);
    let again = create_preparation(&app, &cookie, &csrf, &document, &sha).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text());
    assert_eq!(
        again.json()["data"]["id"].as_str().unwrap(),
        preparation,
        "续传必须复用同一条未完成记录"
    );
    assert_eq!(again.json()["data"]["state"], "preparing");
    assert_eq!(again.json()["data"]["clientDerived"], false);

    // 原件哈希不符 → 422 sourceChanged（不新建记录）。
    let wrong = create_preparation(&app, &cookie, &csrf, &document, &"a".repeat(64)).await;
    wrong.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(wrong.json()["error"]["details"]["reason"], "sourceChanged");

    // document 不存在 → 404；缺失 sourceSha256 → 字段级 422。
    let missing_doc = create_preparation(
        &app,
        &cookie,
        &csrf,
        "01993000-0000-7000-8000-0000000000ff",
        &sha,
    )
    .await;
    missing_doc.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
    let no_sha = app
        .call(
            Method::POST,
            &format!("/api/v1/documents/{document}/preparations"),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({}))
        .send()
        .await;
    no_sha.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        no_sha.json()["error"]["details"]["fields"][0]["field"],
        "sourceSha256"
    );
}

// ---------------------------------------------------------------------------
// 页上传：1-based、幂等、If-Match、ready 门禁、页号与 viewport 校验
// ---------------------------------------------------------------------------

#[tokio::test]
async fn page_upload_is_idempotent_for_identical_content() {
    let (app, cookie, csrf, item, _document, preparation) = preparation_fixture("idem").await;
    let image = upload_page_image(&app, &cookie, &csrf, &item).await;
    let text = upload_page_text(&app, &cookie, &csrf, &item, "第 1 页的文字").await;

    let first = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(Some(&text), Some(&image), viewport(800, 1000, 0)),
        None,
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(first.json()["data"]["pageNumber"], 1);

    let after_first = get_preparation(&app, &cookie, &preparation).await;
    let revision_after_first = after_first.header("etag").expect("GET 返回 ETag");
    assert_eq!(
        after_first.json()["data"]["pages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // 相同内容再次提交（不带 If-Match）：幂等，不报错、不改 revision。
    let second = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(Some(&text), Some(&image), viewport(800, 1000, 0)),
        None,
    )
    .await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text());

    let after_second = get_preparation(&app, &cookie, &preparation).await;
    assert_eq!(
        after_second.header("etag").as_deref(),
        Some(revision_after_first.as_str()),
        "相同内容的重复 PUT 不得自增 revision"
    );
    assert_eq!(
        after_second.json()["data"]["pages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // 同一个 blob 内容用另一份上传（新的 asset id、相同内容哈希）同样幂等。
    let same_content_asset = upload_page_image(&app, &cookie, &csrf, &item).await;
    assert_ne!(same_content_asset, image, "示例：另一条资产行");
    let third = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(
            Some(&text),
            Some(&same_content_asset),
            viewport(800, 1000, 0),
        ),
        None,
    )
    .await;
    assert_eq!(third.status, StatusCode::OK, "{}", third.text());
    let after_third = get_preparation(&app, &cookie, &preparation).await;
    assert_eq!(
        after_third.header("etag").as_deref(),
        Some(revision_after_first.as_str()),
        "相同内容哈希（不同 asset id）仍应幂等"
    );
}

#[tokio::test]
async fn page_overwrite_requires_if_match_and_checks_revision() {
    let (app, cookie, csrf, item, _document, preparation) = preparation_fixture("cas").await;
    let image_a = upload_page_image(&app, &cookie, &csrf, &item).await;
    // 页图内容不变，用页文字的变化触发"内容变化"分支（更贴近真实失败场景：
    // 同一页重新提取得到不同文字/不同渲染结果）。
    let text_a = upload_page_text(&app, &cookie, &csrf, &item, "第一版文字").await;
    let text_b = upload_page_text(&app, &cookie, &csrf, &item, "第二版文字").await;

    let first = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        2,
        &page_body(Some(&text_a), Some(&image_a), viewport(600, 800, 90)),
        None,
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let revision = get_preparation(&app, &cookie, &preparation)
        .await
        .header("etag")
        .expect("ETag");

    // 内容变化但缺 If-Match → 428。
    let without = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        2,
        &page_body(Some(&text_b), Some(&image_a), viewport(600, 800, 90)),
        None,
    )
    .await;
    without.assert_contract_error(StatusCode::PRECONDITION_REQUIRED, "PRECONDITION_REQUIRED");

    // 过期 revision → 412 + details.currentRevision。
    let stale = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        2,
        &page_body(Some(&text_b), Some(&image_a), viewport(600, 800, 90)),
        Some("\"r1\""),
    )
    .await;
    stale.assert_contract_error(StatusCode::PRECONDITION_FAILED, "REVISION_CONFLICT");
    assert_eq!(stale.json()["error"]["details"]["currentRevision"], 2);

    // 正确 If-Match → 覆盖成功。
    let ok = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        2,
        &page_body(Some(&text_b), Some(&image_a), viewport(600, 800, 90)),
        Some(&revision),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());
    let after = get_preparation(&app, &cookie, &preparation).await;
    assert_ne!(after.header("etag").as_deref(), Some(revision.as_str()));
    assert_eq!(after.json()["data"]["pages"][0]["textAssetId"], text_b);
}

#[tokio::test]
async fn page_number_and_viewport_are_validated() {
    let (app, cookie, csrf, item, _document, preparation) = preparation_fixture("validate").await;
    let image = upload_page_image(&app, &cookie, &csrf, &item).await;

    // 0 基页号 → 422 invalidPageNumber（1-based）。
    let zero = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        0,
        &page_body(None, Some(&image), viewport(800, 1000, 0)),
        None,
    )
    .await;
    zero.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        zero.json()["error"]["details"]["reason"],
        "invalidPageNumber"
    );

    // 超出 100 页上限 → 422 pageLimitExceeded。
    let over = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        MAX_PDF_PAGES + 1,
        &page_body(None, Some(&image), viewport(800, 1000, 0)),
        None,
    )
    .await;
    over.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        over.json()["error"]["details"]["reason"],
        "pageLimitExceeded"
    );

    // 缺 viewport → 字段级 422。
    let no_viewport = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &json!({ "imageAssetId": image }),
        None,
    )
    .await;
    no_viewport.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        no_viewport.json()["error"]["details"]["fields"][0]["field"],
        "viewport"
    );

    // 长边超过 2000px → 422。
    let too_big = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(None, Some(&image), viewport(2001, 1000, 0)),
        None,
    )
    .await;
    too_big.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        too_big.json()["error"]["details"]["fields"][0]["field"],
        "viewport"
    );

    // 旋转角非 90 的倍数 → 422。
    let bad_rotation = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(None, Some(&image), viewport(800, 1000, 45)),
        None,
    )
    .await;
    bad_rotation.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 缺页图 → 字段级 422（每页都需要页图）。
    let no_image = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(None, None, viewport(800, 1000, 0)),
        None,
    )
    .await;
    no_image.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        no_image.json()["error"]["details"]["fields"][0]["field"],
        "imageAssetId"
    );
}

#[tokio::test]
async fn page_assets_must_belong_to_item_and_match_purpose() {
    let (app, cookie, csrf, item, _document, preparation) = preparation_fixture("own").await;
    let other_item = create_item(&app, &cookie, &csrf, "另一件仪器").await;
    let foreign_image = upload_page_image(&app, &cookie, &csrf, &other_item).await;
    let own_image = upload_page_image(&app, &cookie, &csrf, &item).await;

    // 跨物品的页图 → 404（不泄露存在性）。
    let cross = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(None, Some(&foreign_image), viewport(800, 1000, 0)),
        None,
    )
    .await;
    cross.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");

    // purpose 不符（把照片资产当页文字）→ 422。
    let photo = upload_asset(
        &app,
        &cookie,
        &csrf,
        &item,
        UploadSpec {
            purpose: "photo",
            filename: "front.jpg",
            content_type: "image/jpeg",
            bytes: &fixture("sample-photo-front.jpg"),
        },
    )
    .await
    .0;
    let wrong_purpose = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(Some(&photo), Some(&own_image), viewport(800, 1000, 0)),
        None,
    )
    .await;
    wrong_purpose.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        wrong_purpose.json()["error"]["details"]["fields"][0]["field"],
        "textAssetId"
    );

    // 页图必须是 JPEG（pageImage 允许 PNG 上传，但页图合同要求白底 JPEG）。
    let png_asset = upload_asset(
        &app,
        &cookie,
        &csrf,
        &item,
        UploadSpec {
            purpose: "pageImage",
            filename: "page.png",
            content_type: "image/png",
            bytes: &fixture("sample-photo-left.png"),
        },
    )
    .await
    .0;
    let png_page = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(None, Some(&png_asset), viewport(800, 1000, 0)),
        None,
    )
    .await;
    png_page.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        png_page.json()["error"]["details"]["fields"][0]["field"],
        "imageAssetId"
    );

    // preparation 不存在 → 404。
    let missing = put_page(
        &app,
        &cookie,
        &csrf,
        "01993000-0000-7000-8000-0000000000aa",
        1,
        &page_body(None, Some(&own_image), viewport(800, 1000, 0)),
        None,
    )
    .await;
    missing.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
}

// ---------------------------------------------------------------------------
// 封存（complete）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn complete_requires_if_match_and_continuous_pages() {
    let (app, cookie, csrf, item, _document, preparation) = preparation_fixture("complete").await;
    let image = upload_page_image(&app, &cookie, &csrf, &item).await;
    for number in [1_i64, 3] {
        let response = put_page(
            &app,
            &cookie,
            &csrf,
            &preparation,
            number,
            &page_body(None, Some(&image), viewport(800, 1000, 0)),
            None,
        )
        .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    }

    // 缺 If-Match → 428。
    let no_if_match = complete_preparation(&app, &cookie, &csrf, &preparation, 3, None).await;
    no_if_match.assert_contract_error(StatusCode::PRECONDITION_REQUIRED, "PRECONDITION_REQUIRED");

    let etag = get_preparation(&app, &cookie, &preparation)
        .await
        .header("etag")
        .expect("ETag");

    // 第 2 页缺失 → 422 incompletePages 并列出缺项。
    let missing_page =
        complete_preparation(&app, &cookie, &csrf, &preparation, 3, Some(&etag)).await;
    missing_page.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        missing_page.json()["error"]["details"]["reason"],
        "incompletePages"
    );
    assert_eq!(
        missing_page.json()["error"]["details"]["missingPages"],
        json!([2])
    );

    // 声明 101 页 → 422 pageLimitExceeded（服务端权威上限，不信任客户端）。
    let over = complete_preparation(
        &app,
        &cookie,
        &csrf,
        &preparation,
        MAX_PDF_PAGES + 1,
        Some(&etag),
    )
    .await;
    over.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        over.json()["error"]["details"]["reason"],
        "pageLimitExceeded"
    );

    // 补齐第 2 页后封存成功。
    let page_two = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        2,
        &page_body(None, Some(&image), viewport(800, 1000, 0)),
        None,
    )
    .await;
    assert_eq!(page_two.status, StatusCode::OK, "{}", page_two.text());
    let etag = get_preparation(&app, &cookie, &preparation)
        .await
        .header("etag")
        .expect("ETag");
    let sealed = complete_preparation(&app, &cookie, &csrf, &preparation, 3, Some(&etag)).await;
    assert_eq!(sealed.status, StatusCode::OK, "{}", sealed.text());
    assert_eq!(sealed.json()["data"]["state"], "ready");
    assert_eq!(sealed.json()["data"]["pageCount"], 3);
    assert_eq!(sealed.json()["data"]["clientDerived"], true);
}

#[tokio::test]
async fn complete_lists_asset_problems_when_content_is_unavailable() {
    let (app, cookie, csrf, item, _document, preparation) = preparation_fixture("asset").await;
    let image = upload_page_image(&app, &cookie, &csrf, &item).await;
    let response = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(None, Some(&image), viewport(800, 1000, 0)),
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());

    // 人为把该页图 blob 隔离（模拟内容不可用 / 哈希不可核）→ 封存被拒并列出该页。
    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    sqlx::query(
        "UPDATE blobs SET storage_state = 'quarantined' \
          WHERE sha256 IN (SELECT blob_id FROM assets WHERE id = ?)",
    )
    .bind(&image)
    .execute(&mut *connection)
    .await
    .expect("隔离 blob");
    drop(connection);

    let etag = get_preparation(&app, &cookie, &preparation)
        .await
        .header("etag")
        .expect("ETag");
    let rejected = complete_preparation(&app, &cookie, &csrf, &preparation, 1, Some(&etag)).await;
    rejected.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        rejected.json()["error"]["details"]["reason"],
        "assetMismatch"
    );
    assert_eq!(
        rejected.json()["error"]["details"]["pages"][0]["pageNumber"],
        1
    );

    // 失败后仍是 preparing（没有留下半封存状态）。
    let state = get_preparation(&app, &cookie, &preparation).await;
    assert_eq!(state.json()["data"]["state"], "preparing");
}

#[tokio::test]
async fn complete_seals_without_jobs_or_fees_and_blocks_further_writes() {
    let (app, cookie, csrf, item, _document, preparation) = preparation_fixture("seal").await;
    let image = upload_page_image(&app, &cookie, &csrf, &item).await;
    let text = upload_page_text(&app, &cookie, &csrf, &item, "扫描页之外的第 1 页文字").await;
    for number in 1..=2 {
        let response = put_page(
            &app,
            &cookie,
            &csrf,
            &preparation,
            number,
            &page_body(
                if number == 1 { Some(&text) } else { None },
                Some(&image),
                viewport(1000, 1414, 0),
            ),
            None,
        )
        .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    }

    let etag = get_preparation(&app, &cookie, &preparation)
        .await
        .header("etag")
        .expect("ETag");
    let sealed = complete_preparation(&app, &cookie, &csrf, &preparation, 2, Some(&etag)).await;
    assert_eq!(sealed.status, StatusCode::OK, "{}", sealed.text());

    // complete 不创建 job / 不写费用账本 / 不留 attempt（AC-025）。
    assert_eq!(
        table_count(&app, TableName::Jobs).await,
        0,
        "complete 不得创建 job"
    );
    assert_eq!(
        table_count(&app, TableName::JobStages).await,
        0,
        "complete 不得创建阶段"
    );
    assert_eq!(
        table_count(&app, TableName::CostLedger).await,
        0,
        "complete 不得产生费用记录"
    );
    assert_eq!(
        table_count(&app, TableName::ProviderAttempts).await,
        0,
        "complete 不得产生付费提交"
    );

    // ready 后页写入被拒（422 preparationReady），重复封存同样被拒。
    let write_after = put_page(
        &app,
        &cookie,
        &csrf,
        &preparation,
        1,
        &page_body(None, Some(&image), viewport(800, 1000, 0)),
        None,
    )
    .await;
    write_after.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        write_after.json()["error"]["details"]["reason"],
        "preparationReady"
    );

    let etag_after = get_preparation(&app, &cookie, &preparation)
        .await
        .header("etag")
        .expect("ETag");
    let reseal =
        complete_preparation(&app, &cookie, &csrf, &preparation, 2, Some(&etag_after)).await;
    reseal.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        reseal.json()["error"]["details"]["reason"],
        "preparationReady"
    );

    // ready 详情：缺页为空、标记 clientDerived、保留原件引用。
    let detail = get_preparation(&app, &cookie, &preparation).await;
    let data = &detail.json()["data"];
    assert_eq!(data["state"], "ready");
    assert_eq!(data["clientDerived"], true);
    assert_eq!(data["missingPages"], json!([]));
    assert_eq!(data["pages"].as_array().unwrap().len(), 2);
    assert_eq!(data["pages"][1]["pageNumber"], 2);
    assert_eq!(data["pages"][1]["textAssetId"], serde_json::Value::Null);
    assert_eq!(data["pages"][0]["viewport"]["rotation"], 0);
}

#[tokio::test]
async fn preparation_endpoints_require_session() {
    let (app, _cookie, _csrf, _item, _document, preparation) = preparation_fixture("auth").await;
    for (method, uri) in [
        (Method::GET, format!("/api/v1/preparations/{preparation}")),
        (
            Method::PUT,
            format!("/api/v1/preparations/{preparation}/pages/1"),
        ),
        (
            Method::POST,
            format!("/api/v1/preparations/{preparation}/complete"),
        ),
    ] {
        let response = app.call(method, &uri).json(&json!({})).send().await;
        response.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");
    }
}
