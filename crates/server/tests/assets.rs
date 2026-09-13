//! T06 集成测试：上传与安全资产服务（PRD 修订 1 / ui_revision 1）。
//!
//! 覆盖的验收条件：
//! - **AC-018**：`POST /items/{id}/assets` 上传 PDF/JPEG/PNG；重复上传同一内容命中同一
//!   blob（去重）；`GET`/`HEAD`/Range 的 200 / 206（Content-Range 与 Length 正确）/
//!   416 / 多区间回落 200 / `If-None-Match` 304 / `If-Range` 不匹配 200；HEAD 无 body；
//!   响应不含磁盘路径。
//! - **AC-019**：伪造类型 415、超大 413、像素炸弹/解码失败 422、路径穿越文件名不越界、
//!   未授权/跨物品 404、磁盘预留不足明确错误且无半提交、DB 事务失败不删除共享 blob、
//!   tmp 残留被隔离。
//!
//! 全部用例使用临时 data-dir + 真实 SQLite；无任何外部网络调用；样例资产来自
//! `tests/fixtures/assets/`（T05 原创、sha256 固定）。物品行经仓储直接创建：
//! `POST /items` 属 T07，本卡不实现。

mod common;

use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::{TestApp, TestDir, TestResponse, test_settings};
use everything_manual::assets::AssetStore;
use everything_manual::assets::blob_store::{
    QUARANTINE_DIR_NAME, SpaceProbe, TMP_DIR_NAME, blob_path,
};
use everything_manual::assets::maintenance;
use everything_manual::config::Settings;
use everything_manual::config::datadir;
use everything_manual::http::router::build_app;
use everything_manual::http::state::AppState;
use everything_manual::storage::{Database, repo};
use http_body_util::BodyExt;
use manual_core::timestamps::Timestamp;
use tower::ServiceExt;

const PASSWORD: &str = "test-password-assets-2b7f";

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
        .json(&serde_json::json!({ "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let csrf = response.json()["data"]["csrfToken"]
        .as_str()
        .expect("登录响应包含 csrfToken")
        .to_owned();
    (response.session_cookie(), csrf)
}

/// 经仓储创建物品（T07 之前没有 `POST /items`）。
async fn create_item(app: &TestApp, name: &str) -> String {
    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    repo::items::create(
        &mut connection,
        repo::items::NewItem {
            name: name.to_owned(),
            brand: Some("测试品牌".to_owned()),
            model: "X100".to_owned(),
            variant: None,
        },
    )
    .await
    .expect("创建物品")
    .id
}

/// multipart/form-data 构造器（固定边界，字段顺序可控）。
struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new(tag: &str) -> Self {
        Self {
            boundary: format!("----em-assets-{tag}"),
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

/// 发起一次上传（成功路径）。
async fn upload(
    app: &TestApp,
    item_id: &str,
    cookie: &str,
    csrf: &str,
    multipart: Multipart,
) -> TestResponse {
    let (content_type, body) = multipart.finish();
    app.call(Method::POST, &format!("/api/v1/items/{item_id}/assets"))
        .cookie(cookie)
        .csrf(csrf)
        .raw_body(Some(&content_type), body)
        .send()
        .await
}

/// 统计 data-dir 下某个目录里的文件数（递归）。
fn count_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() { count_files(&path) } else { 1 }
        })
        .sum()
}

/// tmp 目录中的暂存文件数（上传后应为 0：成功 rename、失败删除）。
fn tmp_files(app: &TestApp) -> usize {
    count_files(&app.dir().join(TMP_DIR_NAME))
}

async fn asset_rows(app: &TestApp) -> i64 {
    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    sqlx::query_scalar("SELECT COUNT(*) FROM assets")
        .fetch_one(&mut *connection)
        .await
        .expect("统计资产行")
}

async fn blob_rows(app: &TestApp) -> i64 {
    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
        .fetch_one(&mut *connection)
        .await
        .expect("统计 blob 行")
}

// ---------------------------------------------------------------------------
// 样例内容生成（与仓库 fixture 生成器同思路；测试内自建，便于构造坏样本）
// ---------------------------------------------------------------------------

/// 尺寸可控的最小 PNG（stored 块，无压缩；IDAT 数据体是占位字节）。
fn png_with_dimensions(width: u32, height: u32) -> Vec<u8> {
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_input = kind.to_vec();
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&test_support::assets::crc32_ieee(&crc_input).to_be_bytes());
        out
    }
    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8bit 真彩，无隔行
    png.extend_from_slice(&chunk(b"IHDR", &ihdr));
    png.extend_from_slice(&chunk(b"IDAT", &[0x78, 0x01, 0x00, 0x00, 0x00, 0x01]));
    png.extend_from_slice(&chunk(b"IEND", &[]));
    png
}

/// 页数可控的最小经典 PDF（与 T05 生成器同为手写 xref，便于构造 >100 页用例）。
fn pdf_with_pages(page_count: usize) -> Vec<u8> {
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets: Vec<usize> = vec![0]; // 下标 = 对象号（0 号占位）
    let push = |out: &mut String, offsets: &mut Vec<usize>, body: String| {
        offsets.push(out.len());
        out.push_str(&body);
    };
    push(
        &mut out,
        &mut offsets,
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_owned(),
    );
    let kids: Vec<String> = (0..page_count)
        .map(|index| format!("{} 0 R", 3 + index))
        .collect();
    push(
        &mut out,
        &mut offsets,
        format!(
            "2 0 obj\n<< /Type /Pages /Count {page_count} /Kids [{}] >>\nendobj\n",
            kids.join(" ")
        ),
    );
    for index in 0..page_count {
        push(
            &mut out,
            &mut offsets,
            format!(
                "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] >>\nendobj\n",
                3 + index
            ),
        );
    }
    let xref_offset = out.len();
    out.push_str(&format!("xref\n0 {}\n", page_count + 3));
    out.push_str("0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
        page_count + 3
    ));
    out.into_bytes()
}

// ---------------------------------------------------------------------------
// AC-018：正常上传、去重、内容服务
// ---------------------------------------------------------------------------

#[tokio::test]
async fn uploads_pdf_png_jpeg_and_deduplicates_by_sha256() {
    let app = TestApp::new("assets-normal").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "正常上传").await;

    // 1) PDF → document
    let pdf = fixture("sample-manual-text.pdf");
    let pdf_sha = sha256(&pdf);
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("pdf")
            .text_field("purpose", "document")
            .file_field("file", "manual.pdf", "application/pdf", &pdf),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let data = response.json()["data"].clone();
    assert_eq!(data["purpose"], "document");
    assert_eq!(data["mime"], "application/pdf");
    assert_eq!(data["sha256"], pdf_sha);
    assert_eq!(data["size"], pdf.len() as i64);
    assert_eq!(data["storageState"], "stored");
    assert_eq!(data["originalName"], "manual.pdf");
    assert_eq!(data["itemId"], item);
    let first_asset_id = data["id"].as_str().unwrap().to_owned();
    // 响应不含磁盘路径（也不含任何路径字段）。
    assert!(!response.text().contains(&app.dir().display().to_string()));
    for key in ["path", "blobPath", "dataDir", "storagePath", "fileName"] {
        assert!(data.get(key).is_none(), "响应不应包含 {key}：{data}");
    }

    // 2) 内容字节与布局：blobs/<前 2 位>/<sha256>，逐字节一致
    let blob_path = blob_path(app.dir(), &pdf_sha);
    assert_eq!(
        blob_path,
        app.dir().join("blobs").join(&pdf_sha[..2]).join(&pdf_sha),
        "内容寻址路径只由 sha256 决定"
    );
    assert_eq!(std::fs::read(&blob_path).expect("blob 文件存在"), pdf);

    // 3) 重复上传同一内容 → 同一 blob（去重），asset 是新的
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("pdf2")
            .text_field("purpose", "document")
            .file_field("file", "manual-copy.pdf", "application/pdf", &pdf),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let second = response.json()["data"].clone();
    assert_eq!(second["sha256"], pdf_sha);
    assert_ne!(second["id"], data["id"], "每次上传是一条新的资产记录");
    assert_eq!(blob_rows(&app).await, 1, "同一内容只有一行 blob");
    assert_eq!(asset_rows(&app).await, 2, "两条资产共享一个 blob");
    assert_eq!(
        count_files(&app.dir().join("blobs")),
        1,
        "同一内容只落一个文件"
    );

    // 4) PNG / JPEG → photo
    for (name, bytes, expected_mime) in [
        ("photo.png", fixture("sample-photo-left.png"), "image/png"),
        ("photo.jpg", fixture("sample-photo-front.jpg"), "image/jpeg"),
    ] {
        let response = upload(
            &app,
            &item,
            &cookie,
            &csrf,
            Multipart::new(name)
                .text_field("purpose", "photo")
                .file_field("file", name, "image/png", &bytes),
        )
        .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        assert_eq!(response.json()["data"]["mime"], expected_mime);
    }

    // 5) pageImage 也走同一路由；pageText 接受 UTF-8 文本
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("pageImage")
            .text_field("purpose", "pageImage")
            .file_field(
                "file",
                "page-1.jpg",
                "image/jpeg",
                &fixture("sample-photo-front.jpg"),
            ),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("pageText")
            .text_field("purpose", "pageText")
            .file_field(
                "file",
                "page-1.txt",
                "text/plain",
                "第 1 页的文字内容".as_bytes(),
            ),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    assert_eq!(response.json()["data"]["mime"], "text/plain; charset=utf-8");

    // 6) 物品累计体积 = 去重后的 blob 字节和（同内容只算一次）
    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    let total = repo::assets::item_total_bytes(&mut connection, &item)
        .await
        .expect("统计物品体积");
    let expected = pdf.len() as u64
        + fixture("sample-photo-left.png").len() as u64
        + fixture("sample-photo-front.jpg").len() as u64
        + "第 1 页的文字内容".len() as u64;
    assert_eq!(total, expected);

    // 7) 上传成功后 tmp 目录不留暂存文件
    assert_eq!(tmp_files(&app), 0, "成功路径不应留下 tmp 文件");

    // 8) 资产内容可读且与源字节一致（授权读取）
    let response = app
        .call(
            Method::GET,
            &format!("/api/v1/assets/{first_asset_id}/content"),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, pdf);
    assert_eq!(
        response.header("content-type").as_deref(),
        Some("application/pdf")
    );
}

#[tokio::test]
async fn upload_over_one_mib_is_not_blocked_by_the_json_body_limit() {
    let app = TestApp::new("assets-large").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "大图").await;

    // 3 MiB 的真彩 PNG：远超 JSON 上限（1 MiB），但低于照片上限（20 MiB）。
    let big_png = test_support::generate::build_png(1024, 1024, |x, y| {
        [x as u8, y as u8, ((x ^ y) & 0xff) as u8]
    });
    assert!(big_png.len() > 1_048_576, "样例应大于 JSON 上限");
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("big")
            .text_field("purpose", "photo")
            .file_field("file", "big.png", "image/png", &big_png),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "上传路由的 body 上限必须覆盖 JSON 的 1 MiB：{}",
        response.text()
    );
    assert_eq!(response.json()["data"]["size"], big_png.len() as i64);
}

// ---------------------------------------------------------------------------
// AC-019：伪造类型、越权、路径穿越、超限
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forged_types_and_invalid_purposes_are_rejected() {
    let app = TestApp::new("assets-forged").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "伪造类型").await;
    let uri = format!("/api/v1/items/{item}/assets");

    // 1) 文本冒充 PDF（扩展名与 Content-Type 都伪造）→ 415
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("fake-pdf")
            .text_field("purpose", "document")
            .file_field("file", "fake.pdf", "application/pdf", b"this is not a pdf"),
    )
    .await;
    response.assert_contract_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE");

    // 2) PNG 当作说明书原件 → 415
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("png-as-doc")
            .text_field("purpose", "document")
            .file_field(
                "file",
                "manual.pdf",
                "application/pdf",
                &fixture("sample-photo-left.png"),
            ),
    )
    .await;
    response.assert_contract_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE");
    let message = response.json()["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(message.contains("PDF"), "错误信息应说明期望类型：{message}");

    // 3) 文本当作照片 → 415
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("text-as-photo")
            .text_field("purpose", "photo")
            .file_field("file", "photo.jpg", "image/jpeg", b"not an image at all"),
    )
    .await;
    response.assert_contract_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE");

    // 4) purpose 非法：model 不允许由上传接口产生、snake_case 不是线上值、未知字段
    for (tag, purpose) in [("model", "model"), ("snake", "page_image")] {
        let response = upload(
            &app,
            &item,
            &cookie,
            &csrf,
            Multipart::new(tag)
                .text_field("purpose", purpose)
                .file_field(
                    "file",
                    "x.pdf",
                    "application/pdf",
                    &fixture("sample-manual-text.pdf"),
                ),
        )
        .await;
        response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    }
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("unknown-field")
            .text_field("purpose", "document")
            .text_field("extra", "x")
            .file_field(
                "file",
                "x.pdf",
                "application/pdf",
                &fixture("sample-manual-text.pdf"),
            ),
    )
    .await;
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    // 缺少 purpose / 缺少 file
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("no-purpose").file_field(
            "file",
            "x.pdf",
            "application/pdf",
            &fixture("sample-manual-text.pdf"),
        ),
    )
    .await;
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("no-file").text_field("purpose", "document"),
    )
    .await;
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 5) Content-Type 不是 multipart → 415；缺 CSRF 的 multipart 修改请求 → 403
    let response = app
        .call(Method::POST, &uri)
        .cookie(&cookie)
        .csrf(&csrf)
        .raw_body(Some("application/json"), b"{}".to_vec())
        .send()
        .await;
    response.assert_contract_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE");

    let (content_type, body) = Multipart::new("no-csrf")
        .text_field("purpose", "document")
        .file_field(
            "file",
            "x.pdf",
            "application/pdf",
            &fixture("sample-manual-text.pdf"),
        )
        .finish();
    let response = app
        .call(Method::POST, &uri)
        .cookie(&cookie)
        .raw_body(Some(&content_type), body)
        .send()
        .await;
    response.assert_contract_error(StatusCode::FORBIDDEN, "CSRF_REJECTED");

    // 以上全部失败路径：零元数据、零文件、无 tmp 残留
    assert_eq!(asset_rows(&app).await, 0, "失败请求不得留下资产行");
    assert_eq!(blob_rows(&app).await, 0, "失败请求不得留下 blob 行");
    assert_eq!(count_files(&app.dir().join("blobs")), 0);
    assert_eq!(tmp_files(&app), 0, "失败请求必须清理 tmp");
}

#[tokio::test]
async fn oversized_uploads_are_rejected_with_413_and_no_trace() {
    // 照片上限降到 4 KiB；物品累计上限单独用例。
    let dir = TestDir::new("assets-oversize");
    let mut settings = test_settings(dir.path());
    settings.limits.max_photo_bytes = 4096;
    settings.limits.max_pdf_bytes = 4096;
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "超限").await;

    let big_photo = test_support::generate::build_png(64, 64, |x, y| [x as u8, y as u8, 7]);
    assert!(big_photo.len() as u64 > 4096, "样例应超过 4 KiB 上限");
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("over-photo")
            .text_field("purpose", "photo")
            .file_field("file", "big.png", "image/png", &big_photo),
    )
    .await;
    response.assert_contract_error(StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE");
    assert_eq!(asset_rows(&app).await, 0);
    assert_eq!(blob_rows(&app).await, 0);
    assert_eq!(tmp_files(&app), 0, "超限请求必须清理 tmp");

    // purpose 在 file 之后到达时，用途上限仍在读完流后兜底（同样 413）
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("over-photo-late")
            .file_field("file", "big.png", "image/png", &big_photo)
            .text_field("purpose", "photo"),
    )
    .await;
    response.assert_contract_error(StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE");
    assert_eq!(tmp_files(&app), 0);

    // 请求体超过路由级上限（最大用途上限 + multipart 开销）→ 解析前 413
    let limit = everything_manual::assets::validate::max_upload_request_bytes(
        &app.state().settings().limits,
    );
    let oversized = vec![b'a'; (limit + 1024) as usize];
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("over-request")
            .text_field("purpose", "pageText")
            .file_field("file", "big.txt", "text/plain", &oversized),
    )
    .await;
    response.assert_contract_error(StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE");
    assert_eq!(asset_rows(&app).await, 0);
    assert_eq!(tmp_files(&app), 0);
}

#[tokio::test]
async fn item_total_limit_is_enforced_with_actionable_details() {
    let dir = TestDir::new("assets-item-total");
    let mut settings = test_settings(dir.path());
    // PDF 上限保持默认，物品累计上限设成"只能放一个样例 PDF"。
    let pdf = fixture("sample-manual-text.pdf");
    settings.limits.max_item_total_bytes = pdf.len() as u64 + 10;
    let item_total_limit = settings.limits.max_item_total_bytes;
    let app = TestApp::with_settings(dir, settings).await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "累计上限").await;

    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("first")
            .text_field("purpose", "document")
            .file_field("file", "a.pdf", "application/pdf", &pdf),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());

    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("second")
            .text_field("purpose", "document")
            .file_field(
                "file",
                "b.pdf",
                "application/pdf",
                &fixture("sample-manual-scan.pdf"),
            ),
    )
    .await;
    response.assert_contract_error(StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE");
    let details = response.json()["error"]["details"].clone();
    assert_eq!(details["reason"], "itemTotalLimit");
    assert_eq!(details["limitBytes"], item_total_limit);

    assert_eq!(asset_rows(&app).await, 1, "第二次上传不得半提交");
    assert_eq!(blob_rows(&app).await, 1);
    assert_eq!(tmp_files(&app), 0);
}

#[tokio::test]
async fn path_traversal_filenames_are_metadata_only() {
    let app = TestApp::new("assets-traversal").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "路径穿越").await;
    let pdf = fixture("sample-manual-text.pdf");

    for evil in [
        "../../../../pwned.pdf",
        "/tmp/pwned.pdf",
        "..\\..\\pwned.pdf",
    ] {
        let response = upload(
            &app,
            &item,
            &cookie,
            &csrf,
            Multipart::new("traversal")
                .text_field("purpose", "document")
                .file_field("file", evil, "application/pdf", &pdf),
        )
        .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        assert_eq!(
            response.json()["data"]["originalName"],
            "pwned.pdf",
            "文件名只保留 basename（{evil}）"
        );
    }

    // 没有越界写入：data-dir 内外都不存在被穿越创建的文件
    assert!(!app.dir().join("pwned.pdf").exists());
    if let Some(parent) = app.dir().parent() {
        assert!(
            !parent.join("pwned.pdf").exists(),
            "文件名不得影响 data-dir 之外的路径"
        );
    }
    // 所有内容都只落在内容寻址路径下
    assert_eq!(
        count_files(&app.dir().join("blobs")),
        1,
        "同一内容仍只落一个文件"
    );
    assert_eq!(
        std::fs::read(blob_path(app.dir(), &sha256(&pdf))).unwrap(),
        pdf
    );
}

#[tokio::test]
async fn unauthorized_and_unknown_asset_access_is_404_without_path_leakage() {
    let app = TestApp::new("assets-authz").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item_a = create_item(&app, "物品 A").await;
    let item_b = create_item(&app, "物品 B").await;
    let pdf = fixture("sample-manual-text.pdf");

    let response = upload(
        &app,
        &item_a,
        &cookie,
        &csrf,
        Multipart::new("owned")
            .text_field("purpose", "document")
            .file_field("file", "a.pdf", "application/pdf", &pdf),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let asset_id = response.json()["data"]["id"].as_str().unwrap().to_owned();

    // 1) 未登录 → 401（不泄露资产是否存在）
    let response = app
        .call(Method::GET, &format!("/api/v1/assets/{asset_id}/content"))
        .send()
        .await;
    response.assert_contract_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED");

    // 2) 不存在的资产 / 未授权 id → 404，且响应不含磁盘路径
    for id in ["01993000-0000-7000-8000-00000000dead", "not-an-id"] {
        let response = app
            .call(Method::GET, &format!("/api/v1/assets/{id}/content"))
            .cookie(&cookie)
            .send()
            .await;
        response.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
        let text = response.text();
        assert!(!text.contains(&app.dir().display().to_string()), "{text}");
        assert!(!text.contains("blobs/"), "{text}");
    }

    // 3) 跨物品归属：仓储原语按物品判定（T07/T09 的绑定必须经过它）
    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    assert!(
        repo::assets::find_for_item(&mut connection, &item_a, &asset_id)
            .await
            .expect("查询")
            .is_some()
    );
    assert!(
        repo::assets::find_for_item(&mut connection, &item_b, &asset_id)
            .await
            .expect("查询")
            .is_none(),
        "其他物品读取同一资产必须为空（跨物品 → 404）"
    );

    // 4) 上传到不存在的物品 → 404
    let response = upload(
        &app,
        "01993000-0000-7000-8000-00000000beef",
        &cookie,
        &csrf,
        Multipart::new("ghost")
            .text_field("purpose", "document")
            .file_field("file", "a.pdf", "application/pdf", &pdf),
    )
    .await;
    response.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
    assert_eq!(asset_rows(&app).await, 1, "幽灵物品不得产生资产行");
    assert_eq!(tmp_files(&app), 0);

    // 5) 元数据在库但文件被外部删除 → 404（并留下服务端错误日志）
    let blob = blob_path(app.dir(), &sha256(&pdf));
    std::fs::remove_file(&blob).expect("删除内容文件");
    let response = app
        .call(Method::GET, &format!("/api/v1/assets/{asset_id}/content"))
        .cookie(&cookie)
        .send()
        .await;
    response.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");
}

// ---------------------------------------------------------------------------
// AC-018/019：解码失败、像素炸弹、页数上限
// ---------------------------------------------------------------------------

#[tokio::test]
async fn decoding_failures_pixel_bombs_and_page_limits_are_422() {
    let app = TestApp::new("assets-decode").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "坏内容").await;

    // 1) 像素炸弹：小文件声明 60000×60000（结构自洽，尺寸超出预算）
    let bomb = png_with_dimensions(60_000, 60_000);
    assert!(bomb.len() < 1024, "像素炸弹样例必须很小");
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("bomb")
            .text_field("purpose", "photo")
            .file_field("file", "bomb.png", "image/png", &bomb),
    )
    .await;
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    let details = response.json()["error"]["details"].clone();
    assert_eq!(details["reason"], "imagePixels");
    assert_eq!(details["width"], 60_000);

    // 2) 截断 PNG（丢 IEND）→ 422
    let good_png = png_with_dimensions(32, 32);
    let truncated = &good_png[..good_png.len() - 8];
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("trunc-png")
            .text_field("purpose", "photo")
            .file_field("file", "t.png", "image/png", truncated),
    )
    .await;
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 3) PNG chunk CRC 损坏（IDAT 数据被改）→ 422
    let mut corrupt = good_png.clone();
    let last = corrupt.len();
    corrupt[last - 20] ^= 0xFF; // IEND 之前的 IDAT 数据区
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("crc-png")
            .text_field("purpose", "photo")
            .file_field("file", "c.png", "image/png", &corrupt),
    )
    .await;
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 4) 截断 JPEG（丢 EOI）→ 422
    let jpeg = fixture("sample-photo-front.jpg");
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("trunc-jpeg")
            .text_field("purpose", "photo")
            .file_field("file", "t.jpg", "image/jpeg", &jpeg[..jpeg.len() - 2]),
    )
    .await;
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 5) 有 %PDF- 头但结构不完整 → 422
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("broken-pdf")
            .text_field("purpose", "document")
            .file_field(
                "file",
                "broken.pdf",
                "application/pdf",
                b"%PDF-1.4\n1 0 obj\ngarbage",
            ),
    )
    .await;
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 6) 页数超限（101 页 > 100）→ 422 + details.pageCount
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("too-many-pages")
            .text_field("purpose", "document")
            .file_field("file", "big.pdf", "application/pdf", &pdf_with_pages(101)),
    )
    .await;
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");
    let details = response.json()["error"]["details"].clone();
    assert_eq!(details["reason"], "pdfPageLimit");
    assert_eq!(details["pageCount"], 101);
    assert_eq!(details["maxPages"], 100);

    // 7) pageText 非 UTF-8 → 422
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("bad-text")
            .text_field("purpose", "pageText")
            .file_field("file", "t.txt", "text/plain", &[0x41, 0xFF, 0xFE]),
    )
    .await;
    response.assert_contract_error(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_FAILED");

    // 全部失败：零元数据与零残留
    assert_eq!(asset_rows(&app).await, 0);
    assert_eq!(blob_rows(&app).await, 0);
    assert_eq!(tmp_files(&app), 0);

    // 对照：正好 100 页的 PDF 应被接受（上限本身不是拒绝）
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("hundred-pages")
            .text_field("purpose", "document")
            .file_field("file", "ok.pdf", "application/pdf", &pdf_with_pages(100)),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());

    // 加密 PDF（trailer 带 /Encrypt）按 REQ-012 在上传阶段接受，拒绝留给 T09 准备阶段
    let encrypted = String::from_utf8(pdf_with_pages(2))
        .unwrap()
        .replace("trailer\n<< /Size", "trailer\n<< /Encrypt 9 0 R /Size")
        .into_bytes();
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("encrypted")
            .text_field("purpose", "document")
            .file_field("file", "enc.pdf", "application/pdf", &encrypted),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
}

// ---------------------------------------------------------------------------
// AC-018：GET / HEAD / Range / ETag / 条件请求
// ---------------------------------------------------------------------------

#[tokio::test]
async fn range_head_etag_and_conditional_requests_follow_contract() {
    let app = TestApp::new("assets-range").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "Range").await;
    let png = fixture("sample-photo-left.png");
    let size = png.len() as u64;
    let sha = sha256(&png);

    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("range")
            .text_field("purpose", "photo")
            .file_field("file", "r.png", "image/png", &png),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let asset = response.json()["data"]["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/v1/assets/{asset}/content");
    let etag = format!("\"{sha}\"");

    // 完整 GET → 200 + 完整字节 + ETag/Accept-Ranges
    let response = app.call(Method::GET, &uri).cookie(&cookie).send().await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, png);
    assert_eq!(
        response.header("content-length").as_deref(),
        Some(size.to_string().as_str())
    );
    assert_eq!(response.header("etag").as_deref(), Some(etag.as_str()));
    assert_eq!(response.header("accept-ranges").as_deref(), Some("bytes"));
    assert_eq!(
        response.header("content-type").as_deref(),
        Some("image/png")
    );
    assert!(response.header("content-encoding").is_none());

    // HEAD → 同头无 body
    let response = app.call(Method::HEAD, &uri).cookie(&cookie).send().await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.is_empty(), "HEAD 不得有 body");
    assert_eq!(
        response.header("content-length").as_deref(),
        Some(size.to_string().as_str())
    );
    assert_eq!(response.header("etag").as_deref(), Some(etag.as_str()));

    // 206：单区间的三种写法
    let cases = [
        ("bytes=0-9", 0_u64, 9_u64),
        ("bytes=10-", 10, size - 1),
        ("bytes=-5", size - 5, size - 1),
    ];
    for (header, start, end) in cases {
        let response = app
            .call(Method::GET, &uri)
            .cookie(&cookie)
            .header("range", header)
            .send()
            .await;
        assert_eq!(response.status, StatusCode::PARTIAL_CONTENT, "{header}");
        assert_eq!(
            response.header("content-range").as_deref(),
            Some(format!("bytes {start}-{end}/{size}").as_str()),
            "{header}"
        );
        let expected_length = end - start + 1;
        assert_eq!(
            response.header("content-length").as_deref(),
            Some(expected_length.to_string().as_str()),
            "{header}"
        );
        assert_eq!(
            response.body,
            png[start as usize..=end as usize],
            "{header}"
        );
        assert!(
            response.header("content-encoding").is_none(),
            "Range 响应不得动态压缩（{header}）"
        );
    }

    // 416：起点越界 / 后缀 0
    for header in ["bytes=999999-", "bytes=-0"] {
        let response = app
            .call(Method::GET, &uri)
            .cookie(&cookie)
            .header("range", header)
            .send()
            .await;
        assert_eq!(
            response.status,
            StatusCode::RANGE_NOT_SATISFIABLE,
            "{header}"
        );
        assert_eq!(
            response.header("content-range").as_deref(),
            Some(format!("bytes */{size}").as_str()),
            "{header}"
        );
        response.assert_contract_error(StatusCode::RANGE_NOT_SATISFIABLE, "VALIDATION_FAILED");
    }

    // 多区间 → 回落完整 200（不做拼接）
    let response = app
        .call(Method::GET, &uri)
        .cookie(&cookie)
        .header("range", "bytes=0-1,4-5")
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, png);

    // If-None-Match：命中 → 304（无 body）；不命中 → 完整 200
    let response = app
        .call(Method::GET, &uri)
        .cookie(&cookie)
        .header("if-none-match", &etag)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::NOT_MODIFIED);
    assert!(response.body.is_empty());
    assert_eq!(response.header("etag").as_deref(), Some(etag.as_str()));

    let response = app
        .call(Method::GET, &uri)
        .cookie(&cookie)
        .header("if-none-match", "\"other\"")
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, png);

    // If-Range：匹配 → 206；不匹配 → 完整 200
    let response = app
        .call(Method::GET, &uri)
        .cookie(&cookie)
        .header("range", "bytes=0-9")
        .header("if-range", &etag)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.body, png[..10]);

    let response = app
        .call(Method::GET, &uri)
        .cookie(&cookie)
        .header("range", "bytes=0-9")
        .header("if-range", "\"stale\"")
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, png);

    // HEAD + Range：头是 206 的语义，但无 body
    let response = app
        .call(Method::HEAD, &uri)
        .cookie(&cookie)
        .header("range", "bytes=0-9")
        .send()
        .await;
    assert_eq!(response.status, StatusCode::PARTIAL_CONTENT);
    assert!(response.body.is_empty());
    assert_eq!(
        response.header("content-range").as_deref(),
        Some(format!("bytes 0-9/{size}").as_str())
    );
}

// ---------------------------------------------------------------------------
// AC-019：磁盘不足（可注入）与共享 blob 安全
// ---------------------------------------------------------------------------

/// 可注入剩余空间探测的最小应用（`TestApp` 固定用真实 `statvfs`，这里需要可控值）。
struct InjectedApp {
    router: Router,
    state: AppState,
    dir: TestDir,
}

impl InjectedApp {
    async fn new(tag: &str, probe: SpaceProbe, tweak: impl FnOnce(&mut Settings)) -> Self {
        let dir = TestDir::new(tag);
        let mut settings = test_settings(dir.path());
        tweak(&mut settings);
        datadir::ensure_initialized(settings.data_dir.as_path()).expect("初始化 data-dir");
        let database = Database::open_and_migrate(&settings.data_dir)
            .await
            .expect("打开数据库");
        let store = AssetStore::with_space_probe(settings.data_dir.clone(), probe);
        let state = AppState::with_asset_store(database, settings, store);
        let router = build_app(state.clone());
        Self { router, state, dir }
    }

    /// 建管理员 + 会话（复用仓储原语，等价 `TestApp` 的测试装配）。
    async fn admin_session(&self) -> (String, String) {
        let hash = everything_manual::http::auth::password::hash_password(PASSWORD).unwrap();
        let mut connection = self.state.database().pool().acquire().await.unwrap();
        let admin = match repo::admins::get_single(&mut connection).await.unwrap() {
            Some(admin) => {
                repo::admins::update_password(&mut connection, &admin.id, &hash)
                    .await
                    .unwrap();
                admin
            }
            None => repo::admins::insert(&mut connection, &hash).await.unwrap(),
        };
        let token = everything_manual::http::auth::tokens::generate_session_token().unwrap();
        let csrf = everything_manual::http::auth::tokens::csrf_token_for(&token);
        let now = Timestamp::now();
        repo::sessions::create(
            &mut connection,
            repo::sessions::NewSession {
                admin_id: admin.id.clone(),
                session_token_hash: everything_manual::http::auth::tokens::session_token_hash(
                    &token,
                ),
                csrf_hash: everything_manual::http::auth::tokens::csrf_hash(&csrf),
                expires_at: now.checked_add_millis(3_600_000).unwrap(),
            },
        )
        .await
        .unwrap();
        (format!("em_session={token}"), csrf)
    }

    async fn send(
        &self,
        method: Method,
        uri: &str,
        cookie: &str,
        csrf: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> TestResponse {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:8080")
            .header("cookie", cookie)
            .header("x-csrf-token", csrf)
            .header("content-type", content_type)
            .header("content-length", body.len().to_string())
            .body(Body::from(body))
            .expect("构造请求");
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("路由处理");
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("读取响应体")
            .to_bytes()
            .to_vec();
        TestResponse {
            status,
            headers,
            body,
        }
    }

    async fn create_item(&self) -> String {
        let mut connection = self.state.database().pool().acquire().await.unwrap();
        repo::items::create(
            &mut connection,
            repo::items::NewItem {
                name: "注入测试".to_owned(),
                brand: None,
                model: "X100".to_owned(),
                variant: None,
            },
        )
        .await
        .unwrap()
        .id
    }
}

#[tokio::test]
async fn insufficient_disk_space_reports_clear_error_without_half_commit() {
    let pdf = fixture("sample-manual-text.pdf");

    // A) 解析前预检失败：第一次探测为 0、之后充足——只有"读请求体之前就预检"
    //    才会得到 413；若预检被移除，落盘前复检会看到充足空间而上传成功（用例即失败）。
    let app = InjectedApp::new(
        "assets-disk-full",
        SpaceProbe::scripted([0, u64::MAX]),
        |_| {},
    )
    .await;
    let (cookie, csrf) = app.admin_session().await;
    let item = app.create_item().await;
    let (content_type, body) = Multipart::new("disk")
        .text_field("purpose", "document")
        .file_field("file", "a.pdf", "application/pdf", &pdf)
        .finish();
    let response = app
        .send(
            Method::POST,
            &format!("/api/v1/items/{item}/assets"),
            &cookie,
            &csrf,
            &content_type,
            body,
        )
        .await;
    response.assert_contract_error(StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE");
    let details = response.json()["error"]["details"].clone();
    assert_eq!(details["reason"], "insufficientStorage");
    assert_eq!(details["availableBytes"], 0);
    let message = response.json()["error"]["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        message.contains("磁盘"),
        "错误必须明确说明磁盘空间不足：{message}"
    );
    assert_eq!(
        count_files(&app.dir.join(TMP_DIR_NAME)),
        0,
        "预检失败不得创建 tmp"
    );
    assert_eq!(count_files(&app.dir.join("blobs")), 0);

    // B) 预检通过、落盘前复检失败（脚本化探测：第一次够、之后为 0）：
    //    已读完的 tmp 必须被清理，且不产生任何元数据（无半提交）。
    let app = InjectedApp::new(
        "assets-disk-late",
        SpaceProbe::scripted([u64::MAX, 0]),
        |_| {},
    )
    .await;
    let (cookie, csrf) = app.admin_session().await;
    let item = app.create_item().await;
    let (content_type, body) = Multipart::new("disk-late")
        .text_field("purpose", "document")
        .file_field("file", "a.pdf", "application/pdf", &pdf)
        .finish();
    let response = app
        .send(
            Method::POST,
            &format!("/api/v1/items/{item}/assets"),
            &cookie,
            &csrf,
            &content_type,
            body,
        )
        .await;
    response.assert_contract_error(StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE");
    assert_eq!(
        response.json()["error"]["details"]["reason"],
        "insufficientStorage"
    );
    assert_eq!(
        count_files(&app.dir.join(TMP_DIR_NAME)),
        0,
        "tmp 必须被清理"
    );
    assert_eq!(
        count_files(&app.dir.join("blobs")),
        0,
        "不得半提交 blob 文件"
    );
    let mut connection = app.state.database().pool().acquire().await.unwrap();
    let assets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assets")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    let blobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert_eq!((assets, blobs), (0, 0), "不得留下半提交元数据");
}

#[tokio::test]
async fn db_failure_after_fsync_does_not_delete_a_shared_blob() {
    let app = TestApp::new("assets-shared-blob").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item_a = create_item(&app, "物品 A").await;
    let pdf = fixture("sample-manual-text.pdf");
    let sha = sha256(&pdf);

    // 1) 先成功上传一次，让 blob 文件 + 元数据都在，并被 asset A1 引用
    let response = upload(
        &app,
        &item_a,
        &cookie,
        &csrf,
        Multipart::new("first")
            .text_field("purpose", "document")
            .file_field("file", "a.pdf", "application/pdf", &pdf),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let asset_a1 = response.json()["data"]["id"].as_str().unwrap().to_owned();
    let blob_file = blob_path(app.dir(), &sha);
    assert!(blob_file.exists());

    // 2) 模拟"fsync 之后元数据事务失败"：用服务层直接对不存在的物品提交
    //    （HTTP 层会在读体前先判 404，因此这里走服务入口以真正触发 DB 外键失败）。
    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    let mut writer = everything_manual::assets::blob_store::StagedWriter::create(
        &app.state().assets().tmp_dir(),
        u64::MAX,
        "document",
    )
    .await
    .expect("创建暂存文件");
    writer.write(&pdf).await.expect("写入暂存内容");
    let staged = writer.finish().await.expect("fsync 暂存文件");
    assert_eq!(staged.sha256, sha);

    let error = app
        .state()
        .assets()
        .finalize(
            &mut connection,
            &everything_manual::assets::UploadRequest {
                item_id: "01993000-0000-7000-8000-00000000ffff",
                purpose: manual_core::domain::AssetPurpose::Document,
                original_name: Some("a.pdf".to_owned()),
            },
            &app.state().settings().limits,
            staged,
        )
        .await
        .expect_err("外键失败必须返回错误");
    assert!(
        matches!(
            error,
            everything_manual::assets::AssetError::NotFound { .. }
        ),
        "物品不存在应映射为 NotFound：{error:?}"
    );

    // 3) 共享 blob 未被删除：文件在、元数据在、原资产仍可读
    assert!(
        blob_file.exists(),
        "元数据失败绝不能删除可能被共享的 blob 文件"
    );
    assert_eq!(blob_rows(&app).await, 1);
    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    assert_eq!(
        repo::assets::count_for_blob(&mut connection, &sha)
            .await
            .expect("统计引用"),
        1,
        "原来的资产仍引用该 blob"
    );
    let response = app
        .call(Method::GET, &format!("/api/v1/assets/{asset_a1}/content"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, pdf, "共享内容仍逐字节可读");
    assert_eq!(tmp_files(&app), 0, "失败的暂存文件必须清理");

    // 4) 同一内容上传到另一个真实物品 → 命中去重，两个资产共享同一 blob
    let item_b = create_item(&app, "物品 B").await;
    let response = upload(
        &app,
        &item_b,
        &cookie,
        &csrf,
        Multipart::new("dedup")
            .text_field("purpose", "document")
            .file_field("file", "b.pdf", "application/pdf", &pdf),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    assert_eq!(blob_rows(&app).await, 1, "同一内容仍只有一行 blob");
    assert_eq!(
        repo::assets::count_for_blob(&mut connection, &sha)
            .await
            .expect("统计引用"),
        2,
        "两个物品共享同一 blob"
    );
}

#[tokio::test]
async fn crash_orphans_are_quarantined_while_referenced_blobs_survive() {
    let app = TestApp::new("assets-orphan").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "孤儿扫描").await;

    // 两个不同内容的资产：一个保留（被引用），一个后面把文件删掉模拟"missing"
    let pdf = fixture("sample-manual-text.pdf");
    let scan_pdf = fixture("sample-manual-scan.pdf");
    let mut asset_ids = Vec::new();
    for (tag, bytes) in [("keep", &pdf), ("missing", &scan_pdf)] {
        let response = upload(
            &app,
            &item,
            &cookie,
            &csrf,
            Multipart::new(tag)
                .text_field("purpose", "document")
                .file_field("file", "doc.pdf", "application/pdf", bytes),
        )
        .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        asset_ids.push(response.json()["data"]["id"].as_str().unwrap().to_owned());
    }
    let (kept_asset, missing_asset) = (asset_ids[0].clone(), asset_ids[1].clone());
    let kept_file = blob_path(app.dir(), &sha256(&pdf));
    let missing_file = blob_path(app.dir(), &sha256(&scan_pdf));
    assert!(kept_file.exists() && missing_file.exists());

    // 崩溃残留：tmp 里的半上传文件 + blobs/ 里的无引用孤儿文件
    let stale_tmp = app.dir().join(TMP_DIR_NAME).join("crashed-upload.part");
    std::fs::write(&stale_tmp, b"half written upload").unwrap();
    let orphan_sha = "ab".repeat(32);
    let orphan_dir = app.dir().join("blobs").join(&orphan_sha[..2]);
    std::fs::create_dir_all(&orphan_dir).unwrap();
    let orphan_file = orphan_dir.join(&orphan_sha);
    std::fs::write(&orphan_file, b"orphan blob bytes").unwrap();

    // 被引用的文件在扫描前先删掉 → 状态应收敛为 missing
    std::fs::remove_file(&missing_file).unwrap();

    let mut connection = app
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("获取连接");
    let report = maintenance::scan_and_quarantine(&mut connection, app.dir())
        .await
        .expect("扫描");
    assert_eq!(report.tmp_quarantined, 1, "tmp 残留必须被隔离");
    assert_eq!(report.blobs_quarantined, 1, "无引用孤儿必须被隔离");
    assert_eq!(report.blobs_kept, 1, "被引用的 blob 必须保留");
    assert_eq!(
        report.blobs_marked_missing, 1,
        "文件不在的 blob 收敛为 missing"
    );

    // 隔离区保留现场（只移动不删除）
    let quarantine = app.dir().join(QUARANTINE_DIR_NAME);
    assert!(!stale_tmp.exists(), "tmp 残留已被移出 tmp");
    assert!(!orphan_file.exists(), "孤儿文件已被移出 blobs");
    let quarantined: Vec<PathBuf> = std::fs::read_dir(&quarantine)
        .expect("隔离目录存在")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(
        quarantined.len(),
        2,
        "两个残留文件都在隔离区：{quarantined:?}"
    );
    let mut contents: Vec<Vec<u8>> = quarantined
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    contents.sort();
    let mut expected = vec![
        b"half written upload".to_vec(),
        b"orphan blob bytes".to_vec(),
    ];
    expected.sort();
    assert_eq!(contents, expected, "隔离内容逐字节保留");

    // 被引用的 blob 照旧可读（隔离不得误伤共享内容）；missing 的资产内容 404
    let response = app
        .call(Method::GET, &format!("/api/v1/assets/{kept_asset}/content"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "被引用资产仍可读");
    assert_eq!(response.body, pdf);
    assert!(kept_file.exists(), "被引用的 blob 不得被隔离");

    let response = app
        .call(
            Method::GET,
            &format!("/api/v1/assets/{missing_asset}/content"),
        )
        .cookie(&cookie)
        .send()
        .await;
    response.assert_contract_error(StatusCode::NOT_FOUND, "NOT_FOUND");

    // 第二次扫描幂等：没有新的残留可隔离
    let report = maintenance::scan_and_quarantine(&mut connection, app.dir())
        .await
        .expect("再次扫描");
    assert_eq!(report.tmp_quarantined, 0);
    assert_eq!(report.blobs_quarantined, 0);
    assert_eq!(report.blobs_kept, 1);
    // 隔离区的文件数不变（不重复隔离）
    assert_eq!(std::fs::read_dir(&quarantine).unwrap().count(), 2);

    // 数据库状态与磁盘一致
    let states: Vec<String> = sqlx::query_scalar("SELECT storage_state FROM blobs ORDER BY sha256")
        .fetch_all(&mut *connection)
        .await
        .unwrap();
    assert_eq!(states, vec!["missing".to_owned(), "stored".to_owned()]);
}
