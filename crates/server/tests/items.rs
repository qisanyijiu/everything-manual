//! T07 集成测试：物品、原始资料与版本（PRD 修订 1 / ui_revision 1）。
//!
//! 覆盖的验收条件：
//! - **AC-016**：创建 201 + UUIDv7 + revision=1；空白/超长 → 422（字段级 `details.fields`）；
//!   列表 `{data, nextCursor}` 分页（默认 20／最多 100）；归档物品默认列表不返回、
//!   `archived=true` 可见；已发布资料的引用（document/photo/资产）归档后仍完整可读；
//!   不存在永久删除 API（删除路由 405）。
//! - **AC-017**：同品牌型号不同配置两条并存互不覆盖；并发 PATCH 只有一个成功，
//!   后到者 412 + `details.currentRevision`。
//! - **AC-020**：document 正常绑定并保存 source_sha256；跨物品引用被拒（404）；
//!   `sourceUrl` 不触发任何服务端抓取（本机计数监听器 0 次连接）。
//! - **AC-021**：view 只接受 front/left/back/right/detail；同一物品每视图最多一张
//!   （第二张被拒：422 `details.reason=viewOccupied`）；PATCH 缺 If-Match 428、
//!   冲突 412；detail 可查询但不属于多视图集合。
//! - T04 QA 遗留的 items 语义缺口：创建 201、超长与字段级 422、归档列表过滤、
//!   PATCH 可选字段清空语义（`{"brand": null}` = 清空，不再静默保留原值）。
//!
//! 全部用例使用临时 data-dir + 真实 SQLite；无外部网络调用；样例资产来自
//! `tests/fixtures/assets/`（T05 原创、sha256 固定）；假凭据。

mod common;

use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestResponse};
use everything_manual::storage::repo;
use manual_core::domain::PhotoView;
use serde_json::json;

const PASSWORD: &str = "test-password-items-7c4e";

// ---------------------------------------------------------------------------
// 通用工具
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("读取样例资产 {} 失败：{error}", path.display()))
}

fn sha256(bytes: &[u8]) -> String {
    test_support::sha256_hex(bytes)
}

/// 登录并返回 `(cookie, csrfToken)`。
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

/// 已登录的测试应用（每个用例独立临时 data-dir）。
async fn logged_in_app(tag: &str) -> (TestApp, String, String) {
    let app = TestApp::new(tag).await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    (app, cookie, csrf)
}

/// `POST /items`（含可选品牌/变体），返回 `(id, revision)`。
async fn create_item(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    name: &str,
    brand: Option<&str>,
    model: &str,
    variant: Option<&str>,
) -> (String, i64) {
    let mut body = json!({ "name": name, "model": model });
    if let Some(brand) = brand {
        body["brand"] = json!(brand);
    }
    if let Some(variant) = variant {
        body["variant"] = json!(variant);
    }
    let response = app
        .call(Method::POST, "/api/v1/items")
        .cookie(cookie)
        .csrf(csrf)
        .json(&body)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    (
        response.json()["data"]["id"].as_str().unwrap().to_owned(),
        response.json()["data"]["revision"].as_i64().unwrap(),
    )
}

/// `PATCH /items/{id}`。
async fn patch_item(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    if_match: Option<&str>,
    body: &serde_json::Value,
) -> TestResponse {
    let mut builder = app
        .call(Method::PATCH, &format!("/api/v1/items/{item}"))
        .cookie(cookie)
        .csrf(csrf)
        .json(body);
    if let Some(if_match) = if_match {
        builder = builder.header("if-match", if_match);
    }
    builder.send().await
}

/// 字段级 422 的 `(field, message)` 列表。
fn field_issues(response: &TestResponse) -> Vec<(String, String)> {
    response.json()["error"]["details"]["fields"]
        .as_array()
        .unwrap_or_else(|| panic!("缺少 details.fields：{}", response.text()))
        .iter()
        .map(|issue| {
            (
                issue["field"].as_str().unwrap_or_default().to_owned(),
                issue["message"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// 断言是「字段级 422」且含有指定字段。
fn assert_field_error(response: &TestResponse, field: &str) {
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    let fields: Vec<String> = field_issues(response)
        .into_iter()
        .map(|(field, _)| field)
        .collect();
    assert!(
        fields.iter().any(|name| name == field),
        "期望字段 {field} 报错，实际 {fields:?}：{}",
        response.text()
    );
}

/// multipart/form-data 构造器（固定边界，字段顺序可控）。
struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new(tag: &str) -> Self {
        Self {
            boundary: format!("----em-items-{tag}"),
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

/// 一次上传的规格（purpose 决定服务端校验分支；文件名/类型只是 multipart 元数据）。
struct UploadSpec<'a> {
    purpose: &'a str,
    filename: &'a str,
    content_type: &'a str,
    bytes: &'a [u8],
}

/// 为物品上传一个资产，返回 `(assetId, sha256)`。
async fn upload(
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

/// 上传 PDF 原件（purpose=document）。
async fn upload_pdf(app: &TestApp, cookie: &str, csrf: &str, item: &str) -> (String, String) {
    let pdf = fixture("sample-manual-text.pdf");
    let sha = sha256(&pdf);
    let (asset, uploaded_sha) = upload(
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
    assert_eq!(uploaded_sha, sha);
    (asset, sha)
}

/// 上传照片（JPEG），返回 assetId。
async fn upload_photo(app: &TestApp, cookie: &str, csrf: &str, item: &str) -> String {
    let jpeg = fixture("sample-photo-front.jpg");
    upload(
        app,
        cookie,
        csrf,
        item,
        UploadSpec {
            purpose: "photo",
            filename: "front.jpg",
            content_type: "image/jpeg",
            bytes: &jpeg,
        },
    )
    .await
    .0
}

/// `POST /items/{id}/documents`。
async fn create_document(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    body: &serde_json::Value,
) -> TestResponse {
    app.call(Method::POST, &format!("/api/v1/items/{item}/documents"))
        .cookie(cookie)
        .csrf(csrf)
        .json(body)
        .send()
        .await
}

/// `POST /items/{id}/photos`。
async fn create_photo(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    body: &serde_json::Value,
) -> TestResponse {
    app.call(Method::POST, &format!("/api/v1/items/{item}/photos"))
        .cookie(cookie)
        .csrf(csrf)
        .json(body)
        .send()
        .await
}

/// `PATCH /items/{id}/photos/{photoId}`。
async fn patch_photo(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    photo: &str,
    if_match: Option<&str>,
    body: &serde_json::Value,
) -> TestResponse {
    let mut builder = app
        .call(
            Method::PATCH,
            &format!("/api/v1/items/{item}/photos/{photo}"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .json(body);
    if let Some(if_match) = if_match {
        builder = builder.header("if-match", if_match);
    }
    builder.send().await
}

/// 分页取 `GET /items`（或带 `archived`）的全部 id。
async fn list_all_ids(app: &TestApp, cookie: &str, archived: bool) -> Vec<String> {
    let mut ids = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let uri = match (&cursor, archived) {
            (None, false) => "/api/v1/items?limit=100".to_owned(),
            (None, true) => "/api/v1/items?limit=100&archived=true".to_owned(),
            (Some(cursor), _) => {
                let suffix = if archived { "&archived=true" } else { "" };
                format!(
                    "/api/v1/items?limit=100&cursor={}{suffix}",
                    urlencode(cursor)
                )
            }
        };
        let response = app.call(Method::GET, &uri).cookie(cookie).send().await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        let body = response.json();
        for row in body["data"].as_array().unwrap() {
            ids.push(row["id"].as_str().unwrap().to_owned());
        }
        match body["nextCursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return ids,
        }
    }
}

/// 最小百分号编码（游标只含 `:`、字母、数字与 `-`，只需处理 `:`）。
fn urlencode(value: &str) -> String {
    value.replace(':', "%3A")
}

// ---------------------------------------------------------------------------
// AC-016：创建与字段级校验
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_item_returns_201_uuidv7_revision_one_and_field_level_422() {
    let (app, cookie, csrf) = logged_in_app("create-422").await;

    let response = app
        .call(Method::POST, "/api/v1/items")
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({
            "name": "  照相机  ",
            "brand": " 富士 ",
            "model": " X100V ",
            "variant": "银色",
        }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let body = response.json();
    assert_eq!(body["data"]["name"], "照相机", "创建应规范化空白");
    assert_eq!(body["data"]["brand"], "富士");
    assert_eq!(body["data"]["model"], "X100V");
    assert_eq!(body["data"]["revision"], 1);
    assert_eq!(body["data"]["archivedAt"], serde_json::Value::Null);
    assert_eq!(response.header("etag").as_deref(), Some("\"r1\""));
    let id = body["data"]["id"].as_str().unwrap();
    let uuid = uuid::Uuid::parse_str(id).expect("id 必须是 UUID");
    assert_eq!(uuid.get_version_num(), 7, "id 必须是 UUIDv7：{id}");

    // 缺失 name/model：一次返回两个字段问题。
    let missing = app
        .call(Method::POST, "/api/v1/items")
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({}))
        .send()
        .await;
    assert_field_error(&missing, "name");
    assert_field_error(&missing, "model");

    // 空白 name + 超长 model + 超长 brand：三个字段问题一起返回。
    let long_model = "M".repeat(201);
    let long_brand = "B".repeat(101);
    let invalid = app
        .call(Method::POST, "/api/v1/items")
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({ "name": "   ", "brand": long_brand, "model": long_model }))
        .send()
        .await;
    invalid.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    let fields: Vec<String> = field_issues(&invalid)
        .into_iter()
        .map(|(field, _)| field)
        .collect();
    assert_eq!(fields, vec!["name", "brand", "model"], "应逐字段报错");

    // 边界：正好 200 字符的 name 可通过。
    let exact = "名".repeat(200);
    let ok = app
        .call(Method::POST, "/api/v1/items")
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({ "name": exact, "model": "M-200" }))
        .send()
        .await;
    assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.text());

    // 未知字段 → 422（不静默忽略）。
    let unknown = app
        .call(Method::POST, "/api/v1/items")
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({ "name": "N", "model": "M", "sourceUrl": "https://example.com" }))
        .send()
        .await;
    unknown.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 失败的创建不落行：只有成功创建的 2 条。
    let ids = list_all_ids(&app, &cookie, false).await;
    assert_eq!(ids.len(), 2, "失败的创建不得写入数据：{ids:?}");
}

// ---------------------------------------------------------------------------
// AC-016：列表分页与归档过滤（含 T04 遗留的游标语义）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_items_paginates_with_cursor_and_hides_archived_by_default() {
    let (app, cookie, csrf) = logged_in_app("list").await;
    let mut created = Vec::new();
    for index in 0..25 {
        let (id, _) = create_item(
            &app,
            &cookie,
            &csrf,
            &format!("物品 {index:02}"),
            Some("示例品牌"),
            "M-100",
            None,
        )
        .await;
        created.push(id);
    }

    // 默认 20 条一页 + nextCursor；游标翻页后剩余 5 条且不再有下一页。
    let first = app
        .call(Method::GET, "/api/v1/items")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let first_body = first.json();
    let first_ids: Vec<String> = first_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(first_ids.len(), 20, "默认页大小 20");
    let cursor = first_body["nextCursor"]
        .as_str()
        .expect("还有下一页时必须返回游标")
        .to_owned();
    assert!(cursor.starts_with("v1:items:active:"), "{cursor}");

    let second = app
        .call(
            Method::GET,
            &format!("/api/v1/items?cursor={}", urlencode(&cursor)),
        )
        .cookie(&cookie)
        .send()
        .await;
    let second_body = second.json();
    let second_ids: Vec<String> = second_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(second_ids.len(), 5);
    assert_eq!(second_body["nextCursor"], serde_json::Value::Null);

    let mut seen: Vec<String> = first_ids.into_iter().chain(second_ids).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 25, "两页合起来不重不漏");
    let mut expected = created.clone();
    expected.sort();
    assert_eq!(seen, expected);

    // limit 上限与非法参数：显式 422（不静默兜底、不 500）。
    let capped = app
        .call(Method::GET, "/api/v1/items?limit=100")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(capped.status, StatusCode::OK);
    assert_eq!(capped.json()["data"].as_array().unwrap().len(), 25);
    for uri in [
        "/api/v1/items?limit=101",
        "/api/v1/items?limit=0",
        "/api/v1/items?limit=abc",
        "/api/v1/items?limit=1&limit=2",
        "/api/v1/items?frobnicate=1",
        "/api/v1/items?cursor=not-a-cursor",
        "/api/v1/items?archived=maybe",
    ] {
        let response = app.call(Method::GET, uri).cookie(&cookie).send().await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{uri} 应 422：{}",
            response.text()
        );
    }

    // `archived=false` 与缺省等价（显式取值也被接受）。
    let explicit_active = app
        .call(Method::GET, "/api/v1/items?archived=false&limit=100")
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(explicit_active.status, StatusCode::OK);
    assert_eq!(explicit_active.json()["data"].as_array().unwrap().len(), 25);

    // 归档一个物品：默认列表不返回，`archived=true` 只返回它。
    let archived_id = created[7].clone();
    let archived = patch_item(
        &app,
        &cookie,
        &csrf,
        &archived_id,
        Some("\"r1\""),
        &json!({ "archived": true }),
    )
    .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", archived.text());
    assert!(archived.json()["data"]["archivedAt"].is_string());

    let active = list_all_ids(&app, &cookie, false).await;
    assert_eq!(active.len(), 24);
    assert!(!active.contains(&archived_id), "归档物品不得出现在默认列表");
    let only_archived = list_all_ids(&app, &cookie, true).await;
    assert_eq!(only_archived, vec![archived_id.clone()]);

    // 归档物品单条仍可读（归档是停用，不是删除）。
    let single = app
        .call(Method::GET, &format!("/api/v1/items/{archived_id}"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(single.status, StatusCode::OK, "{}", single.text());
    assert!(single.json()["data"]["archivedAt"].is_string());

    // 过滤条件变了就不能复用旧游标（否则会静默跳页）。
    let mismatch = app
        .call(
            Method::GET,
            &format!("/api/v1/items?archived=true&cursor={}", urlencode(&cursor)),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_field_error(&mismatch, "cursor");

    // 恢复归档 → 回到默认列表。
    let restored = patch_item(
        &app,
        &cookie,
        &csrf,
        &archived_id,
        Some("\"r2\""),
        &json!({ "archived": false }),
    )
    .await;
    assert_eq!(restored.status, StatusCode::OK, "{}", restored.text());
    assert_eq!(
        restored.json()["data"]["archivedAt"],
        serde_json::Value::Null
    );
    assert_eq!(list_all_ids(&app, &cookie, false).await.len(), 25);
}

// ---------------------------------------------------------------------------
// AC-017：同品牌型号不同配置并存 + 并发 412
// ---------------------------------------------------------------------------

#[tokio::test]
async fn same_brand_model_with_different_variants_coexist_and_do_not_overwrite() {
    let (app, cookie, csrf) = logged_in_app("variants").await;
    let (standard, _) = create_item(
        &app,
        &cookie,
        &csrf,
        "相机 标准版",
        Some("富士"),
        "X100V",
        Some("标准版"),
    )
    .await;
    let (enhanced, _) = create_item(
        &app,
        &cookie,
        &csrf,
        "相机 增强版",
        Some("富士"),
        "X100V",
        Some("增强版"),
    )
    .await;
    assert_ne!(standard, enhanced);

    let listed = list_all_ids(&app, &cookie, false).await;
    assert!(listed.contains(&standard) && listed.contains(&enhanced));

    // 修改其中一条不影响另一条（互不覆盖）。
    let updated = patch_item(
        &app,
        &cookie,
        &csrf,
        &standard,
        Some("\"r1\""),
        &json!({ "variant": "限量版" }),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.json()["data"]["variant"], "限量版");

    let other = app
        .call(Method::GET, &format!("/api/v1/items/{enhanced}"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(other.json()["data"]["variant"], "增强版");
    assert_eq!(other.json()["data"]["revision"], 1);
}

#[tokio::test]
async fn patch_requires_if_match_and_concurrent_patch_has_single_winner() {
    let (app, cookie, csrf) = logged_in_app("concurrent").await;
    let (item, _) = create_item(&app, &cookie, &csrf, "并发物品", None, "C-1", None).await;

    // 缺 If-Match → 428；非法 If-Match → 422。
    let missing = patch_item(
        &app,
        &cookie,
        &csrf,
        &item,
        None,
        &json!({ "name": "改名" }),
    )
    .await;
    missing.assert_contract_error(StatusCode::PRECONDITION_REQUIRED, "PRECONDITION_REQUIRED");
    let malformed = patch_item(
        &app,
        &cookie,
        &csrf,
        &item,
        Some("*"),
        &json!({ "name": "改名" }),
    )
    .await;
    malformed.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 并发两次带同一 If-Match 的 PATCH：恰好一胜一 412。
    let body_a = json!({ "name": "并发 A" });
    let body_b = json!({ "name": "并发 B" });
    let first = patch_item(&app, &cookie, &csrf, &item, Some("\"r1\""), &body_a);
    let second = patch_item(&app, &cookie, &csrf, &item, Some("\"r1\""), &body_b);
    let (first, second) = tokio::join!(first, second);
    let statuses = [first.status, second.status];
    assert!(
        statuses.contains(&StatusCode::OK),
        "应有一个成功：{statuses:?}"
    );
    assert!(
        statuses.contains(&StatusCode::PRECONDITION_FAILED),
        "应有一个 412：{statuses:?}"
    );
    let loser = if first.status == StatusCode::OK {
        &second
    } else {
        &first
    };
    loser.assert_contract_error(StatusCode::PRECONDITION_FAILED, "REVISION_CONFLICT");
    assert_eq!(loser.json()["error"]["details"]["currentRevision"], 2);

    // 胜者的写入可见，后到者没有覆盖它。
    let winner_name = if first.status == StatusCode::OK {
        first.json()["data"]["name"].as_str().unwrap().to_owned()
    } else {
        second.json()["data"]["name"].as_str().unwrap().to_owned()
    };
    let fetched = app
        .call(Method::GET, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(fetched.json()["data"]["name"], winner_name.as_str());
    assert_eq!(fetched.json()["data"]["revision"], 2);
    assert_eq!(fetched.header("etag").as_deref(), Some("\"r2\""));
}

// ---------------------------------------------------------------------------
// T04 QA 遗留：PATCH 可选字段清空语义 + 必填字段拒绝 null
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patch_clears_optional_fields_and_rejects_null_on_required_fields() {
    let (app, cookie, csrf) = logged_in_app("patch-clear").await;
    let (item, _) = create_item(
        &app,
        &cookie,
        &csrf,
        "清空测试",
        Some("品牌"),
        "M-1",
        Some("变体"),
    )
    .await;

    // 正例 1：显式 null = 清空 brand（T04 曾静默保留原值）。
    let cleared = patch_item(
        &app,
        &cookie,
        &csrf,
        &item,
        Some("\"r1\""),
        &json!({ "brand": null }),
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.text());
    assert_eq!(cleared.json()["data"]["brand"], serde_json::Value::Null);
    assert_eq!(
        cleared.json()["data"]["variant"],
        "变体",
        "未提供的字段保持原值"
    );
    assert_eq!(cleared.json()["data"]["revision"], 2);

    // 正例 2：空白字符串 = 清空 variant；同时给 brand 设新值。
    let cleared = patch_item(
        &app,
        &cookie,
        &csrf,
        &item,
        Some("\"r2\""),
        &json!({ "variant": "   ", "brand": " 尼康 " }),
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.text());
    assert_eq!(cleared.json()["data"]["variant"], serde_json::Value::Null);
    assert_eq!(cleared.json()["data"]["brand"], "尼康");
    assert_eq!(cleared.json()["data"]["revision"], 3);

    // 反例：必填字段 null/空白 → 422，且修订号不变（失败不得空递增）。
    for (body, field) in [
        (json!({ "name": null }), "name"),
        (json!({ "model": null }), "model"),
        (json!({ "model": "  " }), "model"),
        (json!({ "name": "N".repeat(201) }), "name"),
        (json!({ "variant": "V".repeat(201) }), "variant"),
        // 状态字段没有"清空"语义：显式 null 同样报错，不静默当作"保持原值"。
        (json!({ "archived": null }), "archived"),
    ] {
        let response = patch_item(&app, &cookie, &csrf, &item, Some("\"r3\""), &body).await;
        assert_field_error(&response, field);
    }

    // 空请求体 → 422（不静默无操作）；未知字段 → 422。
    let empty = patch_item(&app, &cookie, &csrf, &item, Some("\"r3\""), &json!({})).await;
    assert_field_error(&empty, "body");
    let unknown = patch_item(
        &app,
        &cookie,
        &csrf,
        &item,
        Some("\"r3\""),
        &json!({ "nickname": "x" }),
    )
    .await;
    unknown.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 全部失败路径之后：revision 仍是 3，数据未变。
    let fetched = app
        .call(Method::GET, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(fetched.json()["data"]["revision"], 3);
    assert_eq!(fetched.json()["data"]["name"], "清空测试");
    assert_eq!(fetched.json()["data"]["variant"], serde_json::Value::Null);
}

// ---------------------------------------------------------------------------
// AC-020：document 绑定、归属校验与 source_sha256
// ---------------------------------------------------------------------------

#[tokio::test]
async fn document_binding_validates_pdf_ownership_and_stores_sha256() {
    let (app, cookie, csrf) = logged_in_app("documents").await;
    let (item, _) = create_item(&app, &cookie, &csrf, "有说明书的物品", None, "D-1", None).await;
    let (other_item, _) = create_item(&app, &cookie, &csrf, "另一个物品", None, "D-2", None).await;
    let (pdf_asset, pdf_sha) = upload_pdf(&app, &cookie, &csrf, &item).await;
    let photo_asset = upload_photo(&app, &cookie, &csrf, &item).await;

    let created = create_document(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({
            "sourceAssetId": pdf_asset,
            "title": " X100V 使用说明书 ",
            "sourceUrl": "https://example.com/manual/x100v.pdf",
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let body = created.json();
    let document_id = body["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(body["data"]["title"], "X100V 使用说明书");
    assert_eq!(body["data"]["sourceSha256"], pdf_sha.as_str());
    assert_eq!(body["data"]["sourceAssetId"], pdf_asset.as_str());
    assert_eq!(body["data"]["itemId"], item.as_str());

    // 字段级校验：title 缺失；sourceUrl 非 http(s)。
    let no_title = create_document(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "sourceAssetId": pdf_asset }),
    )
    .await;
    assert_field_error(&no_title, "title");
    let bad_url = create_document(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "sourceAssetId": pdf_asset, "title": "t", "sourceUrl": "file:///etc/passwd" }),
    )
    .await;
    assert_field_error(&bad_url, "sourceUrl");

    // 非 PDF 资产（照片）→ 422；跨物品资产 → 404；未知资产 → 404。
    let wrong_type = create_document(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "sourceAssetId": photo_asset, "title": "t" }),
    )
    .await;
    assert_field_error(&wrong_type, "sourceAssetId");

    // 另一个物品也上传 PDF（同内容 → 同一 blob，但资产归属另一物品）。
    let (other_asset, _) = upload_pdf(&app, &cookie, &csrf, &other_item).await;
    let cross_item = create_document(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "sourceAssetId": other_asset, "title": "t" }),
    )
    .await;
    cross_item.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
    let ghost = create_document(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "sourceAssetId": "01993000-0000-7000-8000-0000000000ff", "title": "t" }),
    )
    .await;
    ghost.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
    // 跨物品与不存在的响应必须逐字相同（探测者无法区分"存在但不属于你"和"不存在"）。
    assert_eq!(
        cross_item.json()["error"]["message"],
        ghost.json()["error"]["message"],
        "跨物品拒绝不得泄露存在性"
    );

    // 读取侧：列表返回绑定；未知物品 404；游标不透明。
    let listed = app
        .call(Method::GET, &format!("/api/v1/items/{item}/documents"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text());
    assert_eq!(listed.json()["data"].as_array().unwrap().len(), 1);
    assert_eq!(listed.json()["data"][0]["id"], document_id.as_str());
    assert_eq!(listed.json()["nextCursor"], serde_json::Value::Null);
    let unknown_item = app
        .call(
            Method::GET,
            "/api/v1/items/01993000-0000-7000-8000-0000000000ff/documents",
        )
        .cookie(&cookie)
        .send()
        .await;
    unknown_item.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
}

#[tokio::test]
async fn document_source_url_is_never_fetched_by_the_server() {
    let (app, cookie, csrf) = logged_in_app("no-fetch").await;
    let (item, _) = create_item(&app, &cookie, &csrf, "出处链接物品", None, "S-1", None).await;
    let (pdf_asset, _) = upload_pdf(&app, &cookie, &csrf, &item).await;

    // 本机计数监听器：若服务端抓取 source_url，它必然连到这里。
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定计数监听器");
    listener.set_nonblocking(true).expect("非阻塞监听");
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/steal.pdf");

    let created = create_document(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "sourceAssetId": pdf_asset, "title": "出处", "sourceUrl": url }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    assert_eq!(
        created.json()["data"]["sourceUrl"],
        url.as_str(),
        "原样保存出处"
    );

    // 有界等待：任何异步抓取都应在这段时间内建连。
    tokio::time::sleep(Duration::from_millis(200)).await;
    match listener.accept() {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Ok((_stream, addr)) => panic!("服务端访问了 source_url：{addr}"),
        Err(error) => panic!("计数监听器出错：{error}"),
    }
}

// ---------------------------------------------------------------------------
// AC-021：照片视图枚举、唯一性、If-Match、跨物品与 detail 语义
// ---------------------------------------------------------------------------

#[tokio::test]
async fn photo_views_are_validated_and_second_photo_for_same_view_is_rejected() {
    let (app, cookie, csrf) = logged_in_app("photos").await;
    let (item, _) = create_item(&app, &cookie, &csrf, "拍照物品", None, "P-1", None).await;
    let (other_item, _) = create_item(&app, &cookie, &csrf, "另一个物品", None, "P-2", None).await;
    let jpeg_asset = upload_photo(&app, &cookie, &csrf, &item).await;
    let png_asset = {
        let png = fixture("sample-photo-left.png");
        upload(
            &app,
            &cookie,
            &csrf,
            &item,
            UploadSpec {
                purpose: "photo",
                filename: "left.png",
                content_type: "image/png",
                bytes: &png,
            },
        )
        .await
        .0
    };
    let doc_asset = upload_pdf(&app, &cookie, &csrf, &item).await.0;
    let other_photo = upload_photo(&app, &cookie, &csrf, &other_item).await;

    // 正常添加：201 + revision 1 + ETag。
    let front = create_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "assetId": jpeg_asset, "view": "front" }),
    )
    .await;
    assert_eq!(front.status, StatusCode::CREATED, "{}", front.text());
    let front_id = front.json()["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(front.json()["data"]["view"], "front");
    assert_eq!(front.json()["data"]["revision"], 1);
    assert_eq!(front.header("etag").as_deref(), Some("\"r1\""));

    // 同一视图第二张：被拒（422 + reason=viewOccupied + existingPhotoId），不静默覆盖。
    let second_front = create_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "assetId": png_asset, "view": "front" }),
    )
    .await;
    second_front.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        second_front.json()["error"]["details"]["reason"],
        "viewOccupied"
    );
    assert_eq!(second_front.json()["error"]["details"]["view"], "front");
    assert_eq!(
        second_front.json()["error"]["details"]["existingPhotoId"],
        front_id.as_str()
    );

    // 视图枚举与必填：非法值/缺失都报 view 字段。
    for body in [
        json!({ "assetId": png_asset, "view": "top" }),
        json!({ "assetId": png_asset, "view": "FRONT" }),
        json!({ "assetId": png_asset }),
        json!({ "assetId": png_asset, "view": null }),
    ] {
        let response = create_photo(&app, &cookie, &csrf, &item, &body).await;
        assert_field_error(&response, "view");
    }

    // 资产必须是同物品的照片：别的物品 → 404；PDF 资产 → 422。
    let cross = create_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "assetId": other_photo, "view": "left" }),
    )
    .await;
    cross.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
    let wrong_type = create_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "assetId": doc_asset, "view": "left" }),
    )
    .await;
    assert_field_error(&wrong_type, "assetId");

    // 未知物品 → 404（不泄露归属）。
    let ghost_item = create_photo(
        &app,
        &cookie,
        &csrf,
        "01993000-0000-7000-8000-0000000000ff",
        &json!({ "assetId": jpeg_asset, "view": "left" }),
    )
    .await;
    ghost_item.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");

    // 失败后该视图仍只有原来那一张，且可改选后重新占用（改选路径可用）。
    let listed = app
        .call(Method::GET, &format!("/api/v1/items/{item}/photos"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(listed.json()["data"].as_array().unwrap().len(), 1);
    assert_eq!(listed.json()["nextCursor"], serde_json::Value::Null);

    // 该集合上界 5 条：分页参数显式 422，而不是静默忽略。
    let with_limit = app
        .call(Method::GET, &format!("/api/v1/items/{item}/photos?limit=1"))
        .cookie(&cookie)
        .send()
        .await;
    assert_field_error(&with_limit, "limit");

    let moved = patch_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &front_id,
        Some("\"r1\""),
        &json!({ "view": "left" }),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.text());
    assert_eq!(moved.json()["data"]["view"], "left");
    assert_eq!(moved.json()["data"]["revision"], 2);

    let re_added = create_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "assetId": png_asset, "view": "front" }),
    )
    .await;
    assert_eq!(re_added.status, StatusCode::CREATED, "{}", re_added.text());
}

#[tokio::test]
async fn photo_patch_requires_if_match_and_revalidates_views_and_assets() {
    let (app, cookie, csrf) = logged_in_app("photo-patch").await;
    let (item, _) = create_item(&app, &cookie, &csrf, "改视图物品", None, "P-3", None).await;
    let (other_item, _) = create_item(&app, &cookie, &csrf, "另一个物品", None, "P-4", None).await;
    let jpeg_asset = upload_photo(&app, &cookie, &csrf, &item).await;
    let png_asset = {
        let png = fixture("sample-photo-left.png");
        upload(
            &app,
            &cookie,
            &csrf,
            &item,
            UploadSpec {
                purpose: "photo",
                filename: "left.png",
                content_type: "image/png",
                bytes: &png,
            },
        )
        .await
        .0
    };
    let doc_asset = upload_pdf(&app, &cookie, &csrf, &item).await.0;
    let other_photo = upload_photo(&app, &cookie, &csrf, &other_item).await;

    let front = create_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "assetId": jpeg_asset, "view": "front" }),
    )
    .await;
    let front_id = front.json()["data"]["id"].as_str().unwrap().to_owned();
    let left = create_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "assetId": png_asset, "view": "left" }),
    )
    .await;
    let left_id = left.json()["data"]["id"].as_str().unwrap().to_owned();

    // 缺 If-Match → 428；过期 → 412 + currentRevision；非法 → 422。
    let missing = patch_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &front_id,
        None,
        &json!({ "view": "back" }),
    )
    .await;
    missing.assert_contract_error(StatusCode::PRECONDITION_REQUIRED, "PRECONDITION_REQUIRED");

    // 正常改视图 front → back（r1 → r2）。
    let updated = patch_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &front_id,
        Some("\"r1\""),
        &json!({ "view": "back" }),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.json()["data"]["view"], "back");
    assert_eq!(updated.json()["data"]["revision"], 2);
    assert_eq!(updated.header("etag").as_deref(), Some("\"r2\""));

    let stale = patch_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &front_id,
        Some("\"r1\""),
        &json!({ "view": "right" }),
    )
    .await;
    stale.assert_contract_error(StatusCode::PRECONDITION_FAILED, "REVISION_CONFLICT");
    assert_eq!(stale.json()["error"]["details"]["currentRevision"], 2);

    // 目标视图被同物品另一张照片占用 → 422 + existingPhotoId；修订号不变。
    let occupied = patch_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &front_id,
        Some("\"r2\""),
        &json!({ "view": "left" }),
    )
    .await;
    occupied.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    assert_eq!(
        occupied.json()["error"]["details"]["reason"],
        "viewOccupied"
    );
    assert_eq!(
        occupied.json()["error"]["details"]["existingPhotoId"],
        left_id.as_str()
    );

    // 换资产：同物品照片 200；跨物品 404；PDF 422；显式 null 422；空请求体 422。
    let swapped = patch_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &front_id,
        Some("\"r2\""),
        &json!({ "assetId": png_asset }),
    )
    .await;
    assert_eq!(swapped.status, StatusCode::OK, "{}", swapped.text());
    assert_eq!(swapped.json()["data"]["assetId"], png_asset.as_str());
    assert_eq!(swapped.json()["data"]["revision"], 3);

    let cross = patch_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &front_id,
        Some("\"r3\""),
        &json!({ "assetId": other_photo }),
    )
    .await;
    cross.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
    let wrong_type = patch_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &front_id,
        Some("\"r3\""),
        &json!({ "assetId": doc_asset }),
    )
    .await;
    assert_field_error(&wrong_type, "assetId");
    let null_view = patch_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &front_id,
        Some("\"r3\""),
        &json!({ "view": null }),
    )
    .await;
    assert_field_error(&null_view, "view");
    let empty = patch_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &front_id,
        Some("\"r3\""),
        &json!({}),
    )
    .await;
    assert_field_error(&empty, "body");

    // 跨物品的 photoId → 404（不泄露存在性）。
    let cross_photo = patch_photo(
        &app,
        &cookie,
        &csrf,
        &other_item,
        &front_id,
        Some("\"r3\""),
        &json!({ "view": "back" }),
    )
    .await;
    cross_photo.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");

    // 单张读取带 ETag；失败路径没有改动 revision（仍是 3）。
    let fetched = app
        .call(
            Method::GET,
            &format!("/api/v1/items/{item}/photos/{front_id}"),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(fetched.status, StatusCode::OK, "{}", fetched.text());
    assert_eq!(fetched.header("etag").as_deref(), Some("\"r3\""));
    assert_eq!(fetched.json()["data"]["view"], "back");
}

#[tokio::test]
async fn detail_photo_is_listed_but_outside_the_multiview_set() {
    let (app, cookie, csrf) = logged_in_app("detail").await;
    let (item, _) = create_item(&app, &cookie, &csrf, "特写物品", None, "P-5", None).await;
    let jpeg_asset = upload_photo(&app, &cookie, &csrf, &item).await;
    let png_asset = {
        let png = fixture("sample-photo-left.png");
        upload(
            &app,
            &cookie,
            &csrf,
            &item,
            UploadSpec {
                purpose: "photo",
                filename: "left.png",
                content_type: "image/png",
                bytes: &png,
            },
        )
        .await
        .0
    };
    // 故意先建 detail 再建 front：列表顺序必须是**槽位顺序**而不是插入顺序。
    create_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "assetId": jpeg_asset, "view": "detail" }),
    )
    .await;
    create_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "assetId": png_asset, "view": "front" }),
    )
    .await;

    // GET 列表：两张都在，槽位顺序 front → detail。
    let listed = app
        .call(Method::GET, &format!("/api/v1/items/{item}/photos"))
        .cookie(&cookie)
        .send()
        .await;
    let views: Vec<String> = listed.json()["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["view"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(views, vec!["front", "detail"]);

    // 数据层多视图集合：detail 不进入（T12 的 Tripo 请求体只使用它）。
    let mut connection = app.state().database().pool().acquire().await.unwrap();
    let multiview = repo::photos::list_multiview_for_item(&mut connection, &item)
        .await
        .unwrap();
    assert_eq!(
        multiview.iter().map(|photo| photo.view).collect::<Vec<_>>(),
        vec![PhotoView::Front]
    );
    let all = repo::photos::list_for_item(&mut connection, &item)
        .await
        .unwrap();
    assert_eq!(all.len(), 2, "detail 可查询（只是不进多视图集合）");
}

// ---------------------------------------------------------------------------
// AC-016：归档不破坏引用 + 无永久删除 + REQ-017 的结构性前提
// ---------------------------------------------------------------------------

#[tokio::test]
async fn archive_keeps_references_readable_and_delete_routes_are_absent() {
    let (app, cookie, csrf) = logged_in_app("archive").await;
    let (item, _) = create_item(&app, &cookie, &csrf, "归档物品", Some("品牌"), "A-1", None).await;
    let (pdf_asset, pdf_sha) = upload_pdf(&app, &cookie, &csrf, &item).await;
    let photo_asset = upload_photo(&app, &cookie, &csrf, &item).await;
    let document = create_document(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "sourceAssetId": pdf_asset, "title": "原件" }),
    )
    .await;
    let document_id = document.json()["data"]["id"].as_str().unwrap().to_owned();
    let photo = create_photo(
        &app,
        &cookie,
        &csrf,
        &item,
        &json!({ "assetId": photo_asset, "view": "front" }),
    )
    .await;
    let photo_id = photo.json()["data"]["id"].as_str().unwrap().to_owned();

    // 物品编辑（r1 → r2）：既有 document/photo 引用不变（REQ-017 的结构性前提；
    // 快照冻结本身属 T11）。
    let renamed = patch_item(
        &app,
        &cookie,
        &csrf,
        &item,
        Some("\"r1\""),
        &json!({ "name": "改名后的物品" }),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.text());

    // 归档（r2 → r3）。
    let archived = patch_item(
        &app,
        &cookie,
        &csrf,
        &item,
        Some("\"r2\""),
        &json!({ "archived": true }),
    )
    .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", archived.text());
    assert_eq!(archived.json()["data"]["revision"], 3);
    assert!(archived.json()["data"]["archivedAt"].is_string());

    // 再次归档（archived=true 幂等）：归档时间保持首次值，不刷新。
    let archived_at = archived.json()["data"]["archivedAt"]
        .as_str()
        .unwrap()
        .to_owned();
    let re_archived = patch_item(
        &app,
        &cookie,
        &csrf,
        &item,
        Some("\"r3\""),
        &json!({ "archived": true }),
    )
    .await;
    assert_eq!(re_archived.status, StatusCode::OK, "{}", re_archived.text());
    assert_eq!(re_archived.json()["data"]["revision"], 4);
    assert_eq!(
        re_archived.json()["data"]["archivedAt"],
        archived_at.as_str()
    );

    // 归档后：物品、document、photo、资产内容仍完整可读。
    let fetched = app
        .call(Method::GET, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(fetched.status, StatusCode::OK);

    let documents = app
        .call(Method::GET, &format!("/api/v1/items/{item}/documents"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(documents.json()["data"][0]["id"], document_id.as_str());
    assert_eq!(
        documents.json()["data"][0]["sourceSha256"],
        pdf_sha.as_str()
    );

    let photos = app
        .call(Method::GET, &format!("/api/v1/items/{item}/photos"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(photos.json()["data"][0]["id"], photo_id.as_str());

    for asset in [&pdf_asset, &photo_asset] {
        let content = app
            .call(Method::GET, &format!("/api/v1/assets/{asset}/content"))
            .cookie(&cookie)
            .send()
            .await;
        assert_eq!(content.status, StatusCode::OK, "{}", content.text());
        assert!(!content.body.is_empty());
    }

    // 归档物品默认列表不返回，`archived=true` 可见。
    assert!(!list_all_ids(&app, &cookie, false).await.contains(&item));
    assert_eq!(list_all_ids(&app, &cookie, true).await, vec![item.clone()]);

    // MVP 不提供永久删除：删除路由 405（方法不匹配，不是 404 也不是删除成功）。
    for uri in [
        format!("/api/v1/items/{item}"),
        "/api/v1/items".to_owned(),
        format!("/api/v1/items/{item}/documents"),
        format!("/api/v1/items/{item}/photos"),
        format!("/api/v1/items/{item}/photos/{photo_id}"),
    ] {
        let response = app
            .call(Method::DELETE, &uri)
            .cookie(&cookie)
            .csrf(&csrf)
            .send()
            .await;
        assert_eq!(
            response.status,
            StatusCode::METHOD_NOT_ALLOWED,
            "DELETE {uri} 必须 405：{}",
            response.text()
        );
    }

    // 不存在的删除型路径 → JSON 404（未知 API 不落 SPA，也不是"删除成功"）。
    let unknown_delete = app
        .call(Method::DELETE, &format!("/api/v1/items/{item}/permanent"))
        .cookie(&cookie)
        .csrf(&csrf)
        .send()
        .await;
    unknown_delete.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");

    // 删除尝试后数据仍在。
    let after = app
        .call(Method::GET, &format!("/api/v1/items/{item}"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(after.json()["data"]["revision"], 4);
}
