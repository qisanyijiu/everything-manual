//! BUG-006 并发回归守卫（AC-021 并发语义 / contracts.md §1 状态码合同）。
//!
//! 背景：`POST/PATCH /items/{id}/photos` 等写路径原先用 deferred 事务（裸 `BEGIN`）
//! **先读后写**（占用检查 SELECT → INSERT/UPDATE）。WAL 下 deferred 事务的读→写升级
//! 遇活跃写者会**立即**返回 `SQLITE_BUSY`/`SQLITE_BUSY_SNAPSHOT`（`database is locked`，
//! `busy_timeout` 不等待）→ HTTP 500；修复是把所有写事务统一为 `BEGIN IMMEDIATE`
//! （`storage::tx::begin_write`，与 `job_stages::claim_next`、`SubmissionWindow::begin_intent`
//! 同一模式），让 `busy_timeout=5s` 正常等待、真冲突按合同返回 422。
//!
//! 守卫内容（修复前这些用例必然出现 500）：
//! 1. 外部写者持 `BEGIN IMMEDIATE` 期间 `POST /photos` 必须等待后 201，不得 500；
//! 2. 同一视图 4 并发（含执行器常驻写者）→ 恰好 1×201 + 3×422 `viewOccupied`，0×500；
//! 3. 跨物品并发写 → 全部 201，0×500；
//! 4. PATCH 改视图撞同一目标视图 → 一个 200、一个 422，0×500；
//! 5. 真实 `JobExecutor`（250ms tick，即使空闲也每次取写锁）常驻时，读写路径 0×500。
//!
//! 全部用例用临时 data-dir + 真实 SQLite + 进程内路由（`Router::oneshot`），
//! 只依赖本机 CPU 与 `busy_timeout`，不访问外网、不产生付费调用。

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestResponse};
use everything_manual::jobs::{ExecutorConfig, JobExecutor, StageRegistry};
use everything_manual::storage::{Database, begin_write};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

const PASSWORD: &str = "test-password-photos-concurrency-9b31";
/// 常驻写者单次持锁时长（远小于 busy_timeout=5s；用于在测试窗口内制造稳定竞争）。
const WRITER_HOLD: Duration = Duration::from_millis(5);
/// 常驻写者两次持锁之间的空隙（给请求留出进入窗口）。
const WRITER_GAP: Duration = Duration::from_millis(5);

// ---------------------------------------------------------------------------
// 通用工具（与 items.rs 同构的最小集合；不依赖其私有函数）
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("读取样例资产 {} 失败：{error}", path.display()))
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

async fn logged_in_app(tag: &str) -> (TestApp, String, String) {
    let app = TestApp::new(tag).await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    (app, cookie, csrf)
}

/// multipart/form-data 构造器（固定边界）。
fn multipart_photo(purpose: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----em-photos-concurrency";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\n{purpose}\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
             filename=\"front.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// `POST /items` → itemId。
async fn create_item(app: &TestApp, cookie: &str, csrf: &str, name: &str) -> String {
    let response = app
        .call(Method::POST, "/api/v1/items")
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "name": name, "model": "CONC-1" }))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()["data"]["id"].as_str().unwrap().to_owned()
}

/// 上传一张照片资产 → assetId。
async fn upload_photo(app: &TestApp, cookie: &str, csrf: &str, item: &str) -> String {
    let (content_type, body) = multipart_photo("photo", &fixture("sample-photo-front.jpg"));
    let response = app
        .call(Method::POST, &format!("/api/v1/items/{item}/assets"))
        .cookie(cookie)
        .csrf(csrf)
        .raw_body(Some(&content_type), body)
        .send()
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()["data"]["id"].as_str().unwrap().to_owned()
}

/// 建物品 + 上传照片，返回 `(itemId, assetId)`。
async fn item_with_photo(app: &TestApp, cookie: &str, csrf: &str, name: &str) -> (String, String) {
    let item = create_item(app, cookie, csrf, name).await;
    let asset = upload_photo(app, cookie, csrf, &item).await;
    (item, asset)
}

/// `POST /items/{id}/photos`（返回原始响应，供状态码断言）。
async fn post_photo(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    asset: &str,
    view: &str,
) -> TestResponse {
    app.call(Method::POST, &format!("/api/v1/items/{item}/photos"))
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "assetId": asset, "view": view }))
        .send()
        .await
}

/// 断言不是 500（并发路径不得退化为通用内部错误）。
fn assert_not_500(response: &TestResponse, context: &str) {
    assert_ne!(
        response.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{context}：并发写不得返回 500（BUG-006）：{}",
        response.text()
    );
}

/// 422 且 `details.reason = viewOccupied`（合同承诺的并发冲突语义）。
fn assert_view_occupied(response: &TestResponse) {
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "应为视图占用 422：{}",
        response.text()
    );
    let body = response.json();
    assert_eq!(
        body["error"]["details"]["reason"],
        "viewOccupied",
        "422 必须携带 details.reason=viewOccupied：{}",
        response.text()
    );
}

/// 统计状态码（断言分布用）。
fn tally(responses: &[TestResponse]) -> Vec<(u16, usize)> {
    let mut counts: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
    for response in responses {
        *counts.entry(response.status.as_u16()).or_default() += 1;
    }
    counts.into_iter().collect()
}

// ---------------------------------------------------------------------------
// 常驻写者（模拟执行器 tick 的 BEGIN IMMEDIATE 写事务）
// ---------------------------------------------------------------------------

/// 后台常驻写者：循环 `BEGIN IMMEDIATE` + 一条真实 UPDATE + 短持锁。
///
/// 与执行器 `claim_next` 同形：即使没有可领取阶段也周期性取写锁；修复前
/// 用它足以让 `photos.rs` 的 deferred 读→写升级立即失败（500）。
struct ResidentWriter {
    stop: Arc<AtomicBool>,
    task: JoinHandle<()>,
    /// 至少成功持锁一次（保证竞争真的发生过，而不是写者没起来）。
    engaged: Arc<AtomicBool>,
}

impl ResidentWriter {
    async fn start(database: &Database, item_id: String) -> Self {
        let pool = database.pool().clone();
        let stop = Arc::new(AtomicBool::new(false));
        let engaged = Arc::new(AtomicBool::new(false));
        let task_stop = stop.clone();
        let task_engaged = engaged.clone();
        let task = tokio::spawn(async move {
            while !task_stop.load(Ordering::Relaxed) {
                let Ok(mut conn) = pool.acquire().await else {
                    break;
                };
                let Ok(mut tx) = begin_write(&mut conn).await else {
                    break;
                };
                let updated =
                    sqlx::query("UPDATE items SET updated_at = updated_at + 1 WHERE id = ?")
                        .bind(&item_id)
                        .execute(&mut *tx)
                        .await;
                if updated.is_ok() {
                    task_engaged.store(true, Ordering::Relaxed);
                }
                tokio::time::sleep(WRITER_HOLD).await;
                let _ = tx.commit().await;
                // 持锁后即使请求方已排队，也留一点空隙让请求进入（避免测试空转）。
                tokio::time::sleep(WRITER_GAP).await;
            }
        });
        Self {
            stop,
            task,
            engaged,
        }
    }

    /// 等到写者至少持锁一次。
    async fn wait_engaged(&self) {
        for _ in 0..200 {
            if self.engaged.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("常驻写者未能在超时内取得写锁（测试前提不成立）");
    }

    async fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.task.await;
    }
}

// ---------------------------------------------------------------------------
// 1) 外部写者持锁：POST /photos 等待后成功（不得立即 500）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_photos_waits_for_active_writer_and_returns_201() {
    let (app, cookie, csrf) = logged_in_app("photos-conc-active-writer").await;
    let (item, asset) = item_with_photo(&app, &cookie, &csrf, "并发一致性 A").await;
    // 对照组：无写者（快速路径不受影响）。
    let baseline = post_photo(&app, &cookie, &csrf, &item, &asset, "front").await;
    assert_eq!(baseline.status, StatusCode::CREATED, "{}", baseline.text());

    // 持锁写者：另一连接 `BEGIN IMMEDIATE` 并保持 600ms（< busy_timeout 5s）。
    let pool = app.state().database().pool().clone();
    let engaged = Arc::new(AtomicBool::new(false));
    let writer_item = item.clone();
    let writer_engaged = engaged.clone();
    let release = tokio::spawn(async move {
        let mut writer_conn = pool.acquire().await.expect("写者连接");
        let mut writer_tx = begin_write(&mut writer_conn).await.expect("写者取写锁");
        sqlx::query("UPDATE items SET updated_at = updated_at + 1 WHERE id = ?")
            .bind(&writer_item)
            .execute(&mut *writer_tx)
            .await
            .expect("写者更新");
        writer_engaged.store(true, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(600)).await;
        writer_tx.commit().await.expect("写者提交");
    });
    // 等待写者真的持锁（其后 600ms 内写锁不可得）。
    for _ in 0..400 {
        if engaged.load(Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(engaged.load(Ordering::Relaxed), "写者未能在超时内取得写锁");

    // 锁内请求（异视图）：修复前 3ms 内 500 `database is locked`；修复后等待并 201。
    let started = std::time::Instant::now();
    let response = post_photo(&app, &cookie, &csrf, &item, &asset, "left").await;
    let waited = started.elapsed();
    release.await.expect("写者结束");

    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "持锁写者下 POST /photos 必须等待后 201（BUG-006）：{}",
        response.text()
    );
    assert!(
        waited >= Duration::from_millis(300),
        "应为等待写锁（实测 {waited:?}）：立即返回意味着读→写升级立即失败"
    );
}

// ---------------------------------------------------------------------------
// 2) 同视图并发（含常驻写者）：1×201 + 3×422，0×500
// ---------------------------------------------------------------------------

#[tokio::test]
async fn same_view_concurrent_posts_return_contract_status_codes() {
    let (app, cookie, csrf) = logged_in_app("photos-conc-same-view").await;
    let (item, asset) = item_with_photo(&app, &cookie, &csrf, "并发一致性 B").await;
    let writer = ResidentWriter::start(app.state().database(), item.clone()).await;
    writer.wait_engaged().await;

    let router = app.router_handle();
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let router = router.clone();
        let cookie = cookie.clone();
        let csrf = csrf.clone();
        let item = item.clone();
        let asset = asset.clone();
        tasks.push(tokio::spawn(async move {
            use axum::body::Body;
            use axum::http::Request;
            use http_body_util::BodyExt;
            use tower::ServiceExt;
            let body = serde_json::to_vec(&json!({ "assetId": asset, "view": "front" })).unwrap();
            let request = Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v1/items/{item}/photos"))
                .header("host", "127.0.0.1:8080")
                .header("cookie", cookie)
                .header("x-csrf-token", csrf)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap();
            let response = router.oneshot(request).await.expect("路由处理请求");
            let status = response.status();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            (status, bytes.to_vec())
        }));
    }
    let mut results = Vec::new();
    for task in tasks {
        let (status, body) = task.await.expect("并发请求任务");
        results.push((status, body));
    }
    writer.stop().await;

    let created = results
        .iter()
        .filter(|(status, _)| *status == StatusCode::CREATED)
        .count();
    let occupied = results
        .iter()
        .filter(|(status, _)| *status == StatusCode::UNPROCESSABLE_ENTITY)
        .count();
    let failures: Vec<&(StatusCode, Vec<u8>)> = results
        .iter()
        .filter(|(status, _)| {
            *status != StatusCode::CREATED && *status != StatusCode::UNPROCESSABLE_ENTITY
        })
        .collect();
    assert!(
        failures.is_empty(),
        "同视图并发只允许 201/422（不得 500）：{:?}",
        failures
            .iter()
            .map(|(status, body)| (*status, String::from_utf8_lossy(body).into_owned()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        (created, occupied),
        (1, 3),
        "同视图 4 并发应为 1×201 + 3×422，实际：{:?}",
        tally_from(&results)
    );
    // 422 必须是合同承诺的 viewOccupied（不是"字段非法"等其它理由）。
    for (status, body) in &results {
        if *status == StatusCode::UNPROCESSABLE_ENTITY {
            let parsed: Value = serde_json::from_slice(body).expect("错误体 JSON");
            assert_eq!(
                parsed["error"]["details"]["reason"],
                "viewOccupied",
                "422 必须携带 details.reason=viewOccupied：{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    // 落库一致性：该视图恰好一张照片。
    let list = app
        .call(Method::GET, &format!("/api/v1/items/{item}/photos"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    let photos = list.json()["data"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        photos.len(),
        1,
        "同视图并发后应恰好一张照片：{}",
        list.text()
    );
}

fn tally_from(results: &[(StatusCode, Vec<u8>)]) -> Vec<(u16, usize)> {
    let mut counts: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
    for (status, _) in results {
        *counts.entry(status.as_u16()).or_default() += 1;
    }
    counts.into_iter().collect()
}

// ---------------------------------------------------------------------------
// 3) 跨物品并发写 + 常驻写者：全部 201，0×500
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cross_item_concurrent_writes_survive_resident_writer() {
    let (app, cookie, csrf) = logged_in_app("photos-conc-cross-item").await;
    let mut items = Vec::new();
    for index in 0..4 {
        items.push(item_with_photo(&app, &cookie, &csrf, &format!("并发一致性 C{index}")).await);
    }
    let writer = ResidentWriter::start(app.state().database(), items[0].0.clone()).await;
    writer.wait_engaged().await;

    let mut tasks = Vec::new();
    for (item, asset) in &items {
        for view in ["front", "left"] {
            let app_router = app.router_handle();
            let cookie = cookie.clone();
            let csrf = csrf.clone();
            let view = view.to_owned();
            let item = item.clone();
            let asset = asset.clone();
            tasks.push(tokio::spawn(async move {
                let uri = format!("/api/v1/items/{item}/photos");
                let body = json!({ "assetId": asset, "view": view });
                router_json_post(app_router, &uri, &cookie, &csrf, body).await
            }));
        }
    }
    let mut statuses = Vec::new();
    for task in tasks {
        statuses.push(task.await.expect("并发请求任务"));
    }
    writer.stop().await;

    let failures: Vec<&(StatusCode, String)> = statuses
        .iter()
        .filter(|(status, _)| *status != StatusCode::CREATED)
        .collect();
    assert!(
        failures.is_empty(),
        "跨物品并发写应全部 201（不得 500）：{failures:?}"
    );
}

/// 进程内路由 JSON POST（并发任务用；返回状态码与响应体文本）。
async fn router_json_post(
    router: axum::Router,
    uri: &str,
    cookie: &str,
    csrf: &str,
    body: Value,
) -> (StatusCode, String) {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("host", "127.0.0.1:8080")
        .header("cookie", cookie)
        .header("x-csrf-token", csrf)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let response = router.oneshot(request).await.expect("路由处理请求");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------
// 4) PATCH 改视图撞车（常驻写者）：一个 200、一个 422，0×500
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patch_photo_view_conflict_under_resident_writer() {
    let (app, cookie, csrf) = logged_in_app("photos-conc-patch").await;
    let (item, asset) = item_with_photo(&app, &cookie, &csrf, "并发一致性 D").await;

    let front = post_photo(&app, &cookie, &csrf, &item, &asset, "front").await;
    assert_eq!(front.status, StatusCode::CREATED, "{}", front.text());
    let left = post_photo(&app, &cookie, &csrf, &item, &asset, "left").await;
    assert_eq!(left.status, StatusCode::CREATED, "{}", left.text());
    let front_id = front.json()["data"]["id"].as_str().unwrap().to_owned();
    let left_id = left.json()["data"]["id"].as_str().unwrap().to_owned();

    let writer = ResidentWriter::start(app.state().database(), item.clone()).await;
    writer.wait_engaged().await;

    // 两张照片同时 PATCH 到同一目标视图 back：只允许一个成功；另一个 422 viewOccupied。
    let mut tasks = Vec::new();
    for photo in [front_id, left_id] {
        let app_router = app.router_handle();
        let cookie = cookie.clone();
        let csrf = csrf.clone();
        let uri = format!("/api/v1/items/{item}/photos/{photo}");
        tasks.push(tokio::spawn(async move {
            router_json_patch(app_router, &uri, &cookie, &csrf, json!({ "view": "back" })).await
        }));
    }
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.expect("并发 PATCH 任务"));
    }
    writer.stop().await;

    for (status, body) in &results {
        assert_ne!(
            *status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "并发 PATCH 不得 500（BUG-006）：{body}"
        );
    }
    let ok = results
        .iter()
        .filter(|(status, _)| *status == StatusCode::OK)
        .count();
    let occupied = results
        .iter()
        .filter(|(status, _)| *status == StatusCode::UNPROCESSABLE_ENTITY)
        .count();
    assert_eq!(
        (ok, occupied),
        (1, 1),
        "PATCH 撞同一视图应 1×200 + 1×422，实际：{:?}",
        results
            .iter()
            .map(|(status, body)| (*status, body.clone()))
            .collect::<Vec<_>>()
    );
}

async fn router_json_patch(
    router: axum::Router,
    uri: &str,
    cookie: &str,
    csrf: &str,
    body: Value,
) -> (StatusCode, String) {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let request = Request::builder()
        .method(Method::PATCH)
        .uri(uri)
        .header("host", "127.0.0.1:8080")
        .header("cookie", cookie)
        .header("x-csrf-token", csrf)
        .header("if-match", "\"r1\"")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let response = router.oneshot(request).await.expect("路由处理请求");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------
// 5) 真实执行器常驻（250ms tick 取写锁）：读写路径 0×500
// ---------------------------------------------------------------------------

#[tokio::test]
async fn photos_crud_survives_real_executor_writer() {
    let (app, cookie, csrf) = logged_in_app("photos-conc-executor").await;
    // 与生产同一执行器（空注册表：无可领取阶段，但每次 tick 仍 `BEGIN IMMEDIATE` 取写锁）。
    let executor = JobExecutor::new(
        app.state().database().pool().clone(),
        ExecutorConfig {
            idle_poll: Duration::from_millis(20),
            ..ExecutorConfig::default()
        },
        StageRegistry::new(),
    );
    let handle = executor.start();
    // 给执行器几个 tick 建立"常驻写者"节奏。
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut responses = Vec::new();
    for index in 0..6 {
        let (item, asset) =
            item_with_photo(&app, &cookie, &csrf, &format!("执行器并发 {index}")).await;
        responses.push(post_photo(&app, &cookie, &csrf, &item, &asset, "front").await);
        responses.push(post_photo(&app, &cookie, &csrf, &item, &asset, "left").await);
        // 同视图第二张：必须是 422（合同），不是 500。
        responses.push(post_photo(&app, &cookie, &csrf, &item, &asset, "front").await);
    }
    handle.shutdown().await;

    for response in &responses {
        assert_not_500(response, "执行器常驻写者");
    }
    // 每轮第 3 个请求（同视图第二张）必须是 422 viewOccupied。
    for round in 0..6 {
        assert_view_occupied(&responses[round * 3 + 2]);
    }
    let codes = tally(&responses);
    assert_eq!(
        codes,
        vec![(201, 12), (422, 6)],
        "12 个视图写入 + 6 个同视图重复：{:?}",
        responses
            .iter()
            .map(|response| (response.status, response.text()))
            .collect::<Vec<_>>()
    );
}
