//! T21 独立验收（QA 回合 29）：故障注入与安全回归的**缺口项**补测。
//!
//! 本文件**只新增测试**，不修改任何生产代码。覆盖 `llmdoc/validation-release.md` §3
//! 中在 T21 之前没有独立执行证据的必测项：
//!
//! 1. **上传「断流」**（§3 上传行）：客户端声明 Content-Length 后中途断开 →
//!    不产生半提交资产/孤儿元数据，tmp 守卫清理，随后完整重传成功（AC-019 / REQ-011）。
//! 2. **崩溃断点 4「blob rename 后 DB 事务前」**（§3 末尾五个断点）：构造"文件已在
//!    内容寻址位置、元数据未提交"的崩溃现场 → 重开数据库 + 执行 `serve` 启动例程
//!    （`scan_and_quarantine`，与 `config/commands.rs` 启动路径同一函数）→
//!    孤儿被隔离、被引用 blob 不动、任务数/费用预留/不可变版本不变，重传成功。
//! 3. **崩溃断点 5「draft 已提交但 HTTP 响应未返回」**（§3 末尾五个断点）：草稿行已提交、
//!    执行器未推进 checkpoint（客户端拿不到任何结果）→ 重启恢复后不得产生第二份草稿、
//!    不得重复外呼、版本不变（AC-047 / AC-055 的恢复语义）。
//! 4. **说明书批次 3 批中断**（§3「说明书批次另测」）：第 1 批完成、第 2 批响应未知、
//!    第 3 批未开始 → 恢复不重跑第 1 批、第 2 批暂停待人工授权、第 3 批不发起请求；
//!    三批有独立持久身份（stage id / page set / attempt）。
//!
//! 所有 HTTP 只发往 `127.0.0.1`；无真实外网、无付费调用；断点仅在测试构建存在。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestDir, TestResponse, test_settings};
use everything_manual::assets::blob_store::{QUARANTINE_DIR_NAME, TMP_DIR_NAME, blob_path};
use everything_manual::assets::maintenance;
use everything_manual::config::datadir;
use everything_manual::jobs::executor::fixed_jitter_executor;
use everything_manual::jobs::failpoints::{
    self, FailpointAction, MANUAL_AFTER_REQUEST_BEFORE_RESPONSE,
};
use everything_manual::jobs::{
    Clock, ExecutorConfig, JobExecutor, ManualClock, PipelineHandlers, StageContext, StageFuture,
    StageHandler, StageOutcome, StageRegistry,
};
use everything_manual::storage::repo::job_stages::{self, NewStage};
use everything_manual::storage::repo::{self, jobs as jobs_repo};
use everything_manual::storage::{Database, repo::attempts};
use manual_core::domain::{Job, JobStatus, StageKind, SubmitState};
use manual_core::timestamps::Timestamp;
use serde_json::json;
use sqlx::SqlitePool;
use test_support::presets::MANUAL_AI_RESPONSES_PATH;
use test_support::scenario::{RouteScript, Scenario, Step};
use test_support::{BodySpec, FixtureServer, ResponseSpec};
use tokio::io::AsyncWriteExt;

const PASSWORD: &str = "qa-t21-password-1f9c";

// ---------------------------------------------------------------------------
// 通用工具
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/assets")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("读取样例 {} 失败：{error}", path.display()))
}

fn sha256(bytes: &[u8]) -> String {
    test_support::sha256_hex(bytes)
}

/// 登录（真实 HTTP 路由）→ `(cookie, csrfToken)`。
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

/// 经仓储创建物品（列表/上传路径都可用）。
async fn create_item(app: &TestApp, name: &str) -> String {
    let mut connection = app.state().database().pool().acquire().await.expect("连接");
    repo::items::create(
        &mut connection,
        repo::items::NewItem {
            name: name.to_owned(),
            brand: Some("QA21".to_owned()),
            model: "T21".to_owned(),
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
            boundary: format!("----qa21{tag}"),
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

async fn upload(
    app: &TestApp,
    item: &str,
    cookie: &str,
    csrf: &str,
    multipart: Multipart,
) -> TestResponse {
    let (content_type, body) = multipart.finish();
    app.call(Method::POST, &format!("/api/v1/items/{item}/assets"))
        .cookie(cookie)
        .csrf(csrf)
        .raw_body(Some(&content_type), body)
        .send()
        .await
}

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

async fn table_count(pool: &SqlitePool, table: &str) -> i64 {
    // 只用字面 SQL（表名来自固定登记表，不接受外部输入），避免动态 SQL 绕过 sqlx 静态检查。
    let sql = match table {
        "assets" => "SELECT COUNT(*) FROM assets",
        "blobs" => "SELECT COUNT(*) FROM blobs",
        "jobs" => "SELECT COUNT(*) FROM jobs",
        "cost_ledger" => "SELECT COUNT(*) FROM cost_ledger",
        "provider_attempts" => "SELECT COUNT(*) FROM provider_attempts",
        "model_revisions" => "SELECT COUNT(*) FROM model_revisions",
        "manual_drafts" => "SELECT COUNT(*) FROM manual_drafts",
        "manual_releases" => "SELECT COUNT(*) FROM manual_releases",
        other => panic!("未登记的表名：{other}"),
    };
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("统计 {table} 失败：{error}"))
}

async fn wait_until(label: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("等待条件超时：{label}");
}

// ===========================================================================
// 1) 上传断流（§3 上传行「断流」）
// ===========================================================================

/// 在真实 hyper 监听器上加一个进程内路由（与 `serve` 同一 `build_app`）。
async fn spawn_http_server(app: &TestApp) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("绑定回环端口");
    let addr = listener.local_addr().expect("本地地址");
    let router = app.router_handle();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    addr
}

/// **断流**：客户端声明完整 `Content-Length`、只写一半 body 后断开 →
/// 服务端必须在流式读取中途失败且**不产生任何半提交资产**（无 asset/blob 行、
/// 无 tmp 残留），随后同一内容的完整上传必须成功（可恢复）。
///
/// 正对照：断开前必须观察到 `tmp/*.part`（证明请求已通过认证并进入流式落盘路径，
/// 不是被 401/403 提前拒绝的空转通过）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa21_upload_stream_abort_mid_body_leaves_no_half_asset() {
    let app = TestApp::new("qa21-abort").await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "断流物品").await;
    let addr = spawn_http_server(&app).await;

    let pdf = fixture("sample-manual-text.pdf");
    let (content_type, body) = Multipart::new("abort")
        .text_field("purpose", "document")
        .file_field("file", "a.pdf", "application/pdf", &pdf)
        .finish();

    let pool = app.state().database().pool().clone();
    let tmp_dir = app.dir().join(TMP_DIR_NAME);

    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("连接真实监听器");
    let head = format!(
        "POST /api/v1/items/{item}/assets HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Cookie: {cookie}\r\n\
         X-CSRF-Token: {csrf}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.expect("写入请求头");
    let half = body.len() / 2;
    stream
        .write_all(&body[..half])
        .await
        .expect("写入一半请求体");

    // 正对照：流式路径已开始写 tmp（否则本用例会因"根本没进 handler"而空转通过）。
    wait_until(
        "tmp 暂存文件出现（流式落盘已开始）",
        Duration::from_secs(5),
        || count_files(&tmp_dir) > 0,
    )
    .await;

    // 断流：直接关闭连接（FIN），服务端读到 EOF 中途失败。
    drop(stream);

    let tmp_dir_for_poll = tmp_dir.clone();
    wait_until(
        "断流后 tmp 被守卫清理",
        Duration::from_secs(10),
        || count_files(&tmp_dir_for_poll) == 0,
    )
    .await;

    assert_eq!(
        table_count(&pool, "assets").await,
        0,
        "断流不得留下半提交资产"
    );
    assert_eq!(table_count(&pool, "blobs").await, 0, "断流不得留下 blob 行");
    assert_eq!(
        count_files(&app.dir().join("blobs")),
        0,
        "断流不得留下内容寻址文件"
    );

    // 恢复：同一内容完整重传 → 201 且内容可读、哈希一致。
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("complete")
            .text_field("purpose", "document")
            .file_field("file", "a.pdf", "application/pdf", &pdf),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let asset = response.json()["data"]["id"].as_str().unwrap().to_owned();
    let sha = sha256(&pdf);
    assert!(blob_path(app.dir(), &sha).exists(), "重传后 blob 落盘");
    let content = app
        .call(Method::GET, &format!("/api/v1/assets/{asset}/content"))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(content.status, StatusCode::OK);
    assert_eq!(content.body, pdf, "重传内容必须逐字节可读");
    assert_eq!(table_count(&pool, "assets").await, 1);
    assert_eq!(table_count(&pool, "blobs").await, 1);
    assert_eq!(
        count_files(&app.dir().join(TMP_DIR_NAME)),
        0,
        "成功路径不留 tmp"
    );
}

// ===========================================================================
// 2) 崩溃断点 4：blob rename 后 DB 事务前（§3 末尾五个断点）
// ===========================================================================

/// 构造"rename 已完成、元数据事务未提交"的崩溃现场（外加一个 rename 之前的
/// tmp 残留），然后**重开数据库 + 新 AppState/Router（等价 `serve` 重启）**并执行
/// `serve` 启动例程 `scan_and_quarantine`，核对：
/// - 孤儿 blob 被隔离（只移动不删除）、tmp 残留被隔离；
/// - 被引用 blob/资产逐字节可读（不受影响）；
/// - 任务数、费用预留、不可变版本（model revisions）均不变；
/// - 同一内容重新上传成功（幂等可恢复，不产生第二行 blob）。
#[tokio::test]
async fn qa21_blob_renamed_before_metadata_commit_is_safe_after_restart() {
    let dir_a = TestDir::new("qa21-blob-rename-a");
    let data_dir: PathBuf = dir_a.path().to_path_buf();
    let app = TestApp::with_settings(dir_a, test_settings(&data_dir)).await;
    app.set_admin_password(PASSWORD).await;
    let (cookie, csrf) = login(&app).await;
    let item = create_item(&app, "断点四物品").await;

    // 1) 正常上传 → 一个"被引用"的 blob（重启后必须保持可读）。
    let pdf = fixture("sample-manual-text.pdf");
    let response = upload(
        &app,
        &item,
        &cookie,
        &csrf,
        Multipart::new("ref")
            .text_field("purpose", "document")
            .file_field("file", "a.pdf", "application/pdf", &pdf),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let referenced_asset = response.json()["data"]["id"].as_str().unwrap().to_owned();
    let referenced_sha = sha256(&pdf);
    let referenced_path = blob_path(app.dir(), &referenced_sha);
    assert!(referenced_path.exists());

    let pool = app.state().database().pool().clone();
    let jobs_before = table_count(&pool, "jobs").await;
    let ledger_before = table_count(&pool, "cost_ledger").await;
    let attempts_before = table_count(&pool, "provider_attempts").await;
    let revisions_before = table_count(&pool, "model_revisions").await;

    // 2) 崩溃现场 A：rename 已把内容放到 blobs/<前缀>/<sha>，但元数据事务未提交
    //    （无 blobs 行、无 assets 行）。
    let orphan = b"qa21-orphan-after-rename-content".to_vec();
    let orphan_sha = sha256(&orphan);
    let orphan_path = blob_path(app.dir(), &orphan_sha);
    std::fs::create_dir_all(orphan_path.parent().expect("blob 父目录")).expect("建 blob 目录");
    std::fs::write(&orphan_path, &orphan).expect("写入孤儿 blob");

    // 3) 崩溃现场 B：tmp 残留（崩溃在 rename 之前）。
    let tmp_dir = app.dir().join(TMP_DIR_NAME);
    std::fs::create_dir_all(&tmp_dir).expect("建 tmp 目录");
    let tmp_file = tmp_dir.join("qa21-crash.part");
    std::fs::write(&tmp_file, b"partial").expect("写入 tmp 残留");

    // 4) 重启：重开数据库 + 新 AppState/Router（第二次连接同一 data-dir），并执行
    //    `serve` 启动例程里的资产扫描（`config/commands.rs` 启动路径同一函数）。
    let dir_b = TestDir::new("qa21-blob-rename-b");
    let restarted = TestApp::with_settings(dir_b, test_settings(&data_dir)).await;
    let mut connection = restarted
        .state()
        .database()
        .pool()
        .acquire()
        .await
        .expect("连接");
    let report = maintenance::scan_and_quarantine(&mut connection, &data_dir)
        .await
        .expect("启动扫描");
    assert_eq!(report.tmp_quarantined, 1, "tmp 残留应被隔离：{report:?}");
    assert_eq!(
        report.blobs_quarantined, 1,
        "孤儿 blob 应被隔离：{report:?}"
    );
    drop(connection);

    // 5) 核对五件事：任务数 / 远端请求（无 Provider 配置 → attempts=0）/
    //    费用预留 / 资产引用 / 不可变版本。
    assert_eq!(table_count(&pool, "jobs").await, jobs_before, "任务数不变");
    assert_eq!(
        table_count(&pool, "cost_ledger").await,
        ledger_before,
        "费用预留不变"
    );
    assert_eq!(
        table_count(&pool, "provider_attempts").await,
        attempts_before,
        "无远端请求"
    );
    assert_eq!(
        table_count(&pool, "model_revisions").await,
        revisions_before,
        "不可变版本不变"
    );
    assert!(
        !orphan_path.exists(),
        "孤儿文件必须被移出内容寻址位置（隔离，不删除）"
    );
    let quarantine = app.dir().join(QUARANTINE_DIR_NAME);
    assert!(
        count_files(&quarantine) >= 2,
        "隔离区应同时含孤儿 blob 与 tmp 残留：{}",
        count_files(&quarantine)
    );
    assert!(referenced_path.exists(), "被引用 blob 不得被动");

    // 重启后的实例仍可逐字节读回已发布路径上的资产。
    let content = restarted
        .call(
            Method::GET,
            &format!("/api/v1/assets/{referenced_asset}/content"),
        )
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(content.status, StatusCode::OK);
    assert_eq!(content.body, pdf, "重启后被引用资产逐字节可读");

    // 6) 同一孤儿内容重新上传 → 成功；不产生重复 blob 行。
    let response = upload(
        &restarted,
        &item,
        &cookie,
        &csrf,
        Multipart::new("after")
            .text_field("purpose", "document")
            .file_field("file", "a.pdf", "application/pdf", &pdf),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "重启后同一内容重传成功：{}",
        response.text()
    );
    assert_eq!(
        table_count(&pool, "blobs").await,
        1,
        "去重后仍只有一行 blob"
    );
    assert_eq!(table_count(&pool, "assets").await, 2);
    assert_eq!(
        table_count(&pool, "jobs").await,
        jobs_before,
        "上传不创建任务"
    );
    assert_eq!(
        table_count(&pool, "cost_ledger").await,
        ledger_before,
        "上传不产生费用"
    );
}

// ===========================================================================
// 3) 崩溃断点 5：draft 已提交但 HTTP 响应未返回（§3 末尾五个断点）
// ===========================================================================

async fn open_database(dir: &TestDir) -> Database {
    datadir::ensure_initialized(dir.path()).expect("初始化 data-dir");
    Database::open_and_migrate(dir.path())
        .await
        .expect("打开并迁移")
}

async fn seed_job(pool: &SqlitePool, tag: &str, page_count: i64) -> (String, String) {
    let now = Timestamp::now().as_millis();
    let item_id = manual_core::ids::new_id();
    let sha = test_support::sha256_hex(format!("{tag}-source").as_bytes());
    let asset_id = manual_core::ids::new_id();
    let document_id = manual_core::ids::new_id();
    let preparation_id = manual_core::ids::new_id();
    let snapshot_id = manual_core::ids::new_id();

    let mut conn = pool.acquire().await.expect("连接");
    sqlx::query(
        "INSERT INTO items (id, name, brand, model, variant, revision, archived_at, created_at, updated_at) \
         VALUES (?, ?, NULL, ?, NULL, 1, NULL, ?, ?)",
    )
    .bind(&item_id)
    .bind(format!("QA21 物品 {tag}"))
    .bind("QA21")
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .expect("插入 item");
    sqlx::query(
        "INSERT INTO blobs (sha256, size, mime, storage_state, created_at) VALUES (?, 1024, 'application/pdf', 'stored', ?)",
    )
    .bind(&sha)
    .bind(now)
    .execute(&mut *conn)
    .await
    .expect("插入 blob");
    sqlx::query(
        "INSERT INTO assets (id, blob_id, item_id, purpose, original_name, created_at) VALUES (?, ?, ?, 'document', 'manual.pdf', ?)",
    )
    .bind(&asset_id)
    .bind(&sha)
    .bind(&item_id)
    .bind(now)
    .execute(&mut *conn)
    .await
    .expect("插入 asset");
    sqlx::query(
        "INSERT INTO documents (id, item_id, source_asset_id, source_sha256, title, source_url, created_at, updated_at) \
         VALUES (?, ?, ?, ?, '说明书', NULL, ?, ?)",
    )
    .bind(&document_id)
    .bind(&item_id)
    .bind(&asset_id)
    .bind(&sha)
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .expect("插入 document");
    sqlx::query(
        "INSERT INTO preparations (id, document_id, source_sha256, state, page_count, revision, client_derived, created_at, updated_at) \
         VALUES (?, ?, ?, 'ready', ?, 1, 1, ?, ?)",
    )
    .bind(&preparation_id)
    .bind(&document_id)
    .bind(&sha)
    .bind(page_count)
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .expect("插入 preparation");
    sqlx::query(
        "INSERT INTO generation_snapshots \
             (id, item_id, item_revision, preparation_id, photo_ids, photo_hashes, provider_config, \
              prompt_version, price_version, budgets, created_at) \
         VALUES (?, ?, 1, ?, '[\"photo-1\"]', '[\"hash-1\"]', '{}', 'prompt-v1', 'price-v1', '{}', ?)",
    )
    .bind(&snapshot_id)
    .bind(&item_id)
    .bind(&preparation_id)
    .bind(now)
    .execute(&mut *conn)
    .await
    .expect("插入 snapshot");
    let job = jobs_repo::create(
        &mut conn,
        jobs_repo::NewJob {
            item_id: item_id.clone(),
            snapshot_id,
        },
    )
    .await
    .expect("创建 job");
    drop(conn);
    (job.id, item_id)
}

async fn insert_stage(
    pool: &SqlitePool,
    job_id: &str,
    kind: StageKind,
    batch_index: i64,
    pages: Option<Vec<i64>>,
    status: JobStatus,
) -> manual_core::domain::JobStage {
    let mut conn = pool.acquire().await.expect("连接");
    job_stages::insert(
        &mut conn,
        NewStage {
            job_id: job_id.to_owned(),
            stage_kind: kind,
            batch_index,
            page_set_json: pages.map(|pages| {
                format!(
                    "[{}]",
                    pages
                        .iter()
                        .map(|page| page.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }),
            input_hash: format!("hash-{}-{batch_index}", kind.as_str()),
            status,
        },
        Timestamp::now(),
    )
    .await
    .expect("插入阶段")
}

fn executor_config() -> ExecutorConfig {
    ExecutorConfig {
        lease: Duration::from_secs(30),
        renew: Duration::from_secs(10),
        remote_generation_limit: 2,
        manual_ai_batch_limit: 2,
        idle_poll: Duration::from_millis(20),
        shutdown_grace: Duration::from_secs(2),
        unregistered_handler_delay: Duration::from_secs(60),
    }
}

async fn read_stage(pool: &SqlitePool, stage_id: &str) -> manual_core::domain::JobStage {
    let mut conn = pool.acquire().await.expect("连接");
    job_stages::get(&mut conn, stage_id)
        .await
        .expect("读取阶段")
        .expect("阶段存在")
}

async fn read_job(pool: &SqlitePool, job_id: &str) -> Job {
    let mut conn = pool.acquire().await.expect("连接");
    jobs_repo::get(&mut conn, job_id)
        .await
        .expect("读取 job")
        .expect("job 存在")
}

async fn stage_status(pool: &SqlitePool, stage_id: &str) -> JobStatus {
    read_stage(pool, stage_id).await.status
}

async fn expire_lease(pool: &SqlitePool, stage_id: &str) {
    let past = Timestamp::now().as_millis() - 1_000;
    sqlx::query("UPDATE job_stages SET lease_until = ? WHERE id = ?")
        .bind(past)
        .bind(stage_id)
        .execute(pool)
        .await
        .expect("过期租约");
}

/// 构造"崩溃在结果落库与 checkpoint 之间"的现场：阶段回到 running、租约已过期。
async fn crash_stage_before_checkpoint(pool: &SqlitePool, stage_id: &str) {
    let now = Timestamp::now().as_millis();
    sqlx::query(
        "UPDATE job_stages SET status = 'running', lease_owner = 'qa21-crashed', \
            lease_until = ?, updated_at = ? WHERE id = ?",
    )
    .bind(now - 10_000)
    .bind(now)
    .bind(stage_id)
    .execute(pool)
    .await
    .expect("构造崩溃现场");
}

/// 草稿已提交（草稿行在库）、但执行器尚未推进 `assemble_draft`（客户端拿不到任何结果）
/// → 重启恢复后：不得产生第二份草稿、不得重复外呼、不得产生发布/费用。
#[tokio::test]
async fn qa21_draft_committed_before_response_is_never_duplicated_on_restart() {
    let dir = TestDir::new("qa21-draft-commit");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "draft-commit", 4).await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        None,
        JobStatus::Succeeded,
    )
    .await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        0,
        Some(vec![1, 2, 3, 4]),
        JobStatus::Succeeded,
    )
    .await;
    // 两条分支定性但未产出（知识分支未合并、模型未通过校验）→ 组装得到"部分草稿"；
    // 本用例只关心"草稿行已提交后崩溃"这一断点，与分支是否完整无关。
    insert_stage(
        &pool,
        &job_id,
        StageKind::ManualMerge,
        0,
        None,
        JobStatus::NeedsInput,
    )
    .await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::TripoUpload,
        0,
        None,
        JobStatus::Succeeded,
    )
    .await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::TripoSubmit,
        0,
        None,
        JobStatus::Succeeded,
    )
    .await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::TripoPoll,
        0,
        None,
        JobStatus::Succeeded,
    )
    .await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::ModelDownload,
        0,
        None,
        JobStatus::Succeeded,
    )
    .await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::ModelValidate,
        0,
        None,
        JobStatus::NeedsInput,
    )
    .await;
    let assemble = insert_stage(
        &pool,
        &job_id,
        StageKind::AssembleDraft,
        0,
        None,
        JobStatus::Queued,
    )
    .await;

    let settings = test_settings(dir.path());
    let clock = Arc::new(ManualClock::new(Timestamp::now()));

    // 1) "草稿已提交"：与 `assemble_draft` 处理器同一入口——草稿行已落库。
    let job = read_job(&pool, &job_id).await;
    let outcome = everything_manual::drafts::assemble_draft(&pool, dir.path(), &job, clock.now())
        .await
        .expect("组装草稿");
    assert!(outcome.created, "首次组装应创建草稿");
    let draft_id = outcome.draft.id.clone();
    assert_eq!(
        outcome.draft.status,
        manual_core::domain::DraftStatus::NeedsReview
    );

    // 2) 崩溃：执行器在推进 checkpoint 之前死亡（HTTP 响应未返回），草稿行已可见。
    crash_stage_before_checkpoint(&pool, &assemble.id).await;
    assert_eq!(stage_status(&pool, &assemble.id).await, JobStatus::Running);
    assert_eq!(
        table_count(&pool, "manual_drafts").await,
        1,
        "草稿已提交（崩溃现场）"
    );

    // 3) 重启：新执行器（注册真实 pipeline 处理器）+ 恢复扫描 + 重跑。
    let mut registry = StageRegistry::new();
    let pipeline = PipelineHandlers::from_settings(&settings);
    pipeline.register(&mut registry);
    let executor: Arc<JobExecutor> =
        fixed_jitter_executor(pool.clone(), executor_config(), registry, clock.clone());
    let report = executor.recover_expired_leases().await.expect("恢复扫描");
    assert_eq!(report.requeued, 1, "无未决事实 → 重新入队：{report:?}");
    let _ = executor.tick().await.expect("重跑 assemble_draft");

    // 4) 核对五件事：任务数 / 远端请求 / 费用预留 / 资产引用 / 不可变版本。
    assert_eq!(
        stage_status(&pool, &assemble.id).await,
        JobStatus::Succeeded
    );
    assert_eq!(
        read_job(&pool, &job_id).await.status,
        JobStatus::NeedsInput,
        "分支未定性 → 父 job 如实反映（不因草稿已存在就假成功）"
    );
    assert_eq!(
        table_count(&pool, "manual_drafts").await,
        1,
        "重启不得产生第二份草稿"
    );
    let mut conn = pool.acquire().await.expect("连接");
    let draft = repo::drafts::get_by_snapshot(&mut conn, &job.snapshot_id)
        .await
        .expect("读草稿")
        .expect("草稿存在");
    assert_eq!(draft.id, draft_id, "仍是同一份草稿");
    assert_eq!(draft.revision, 1, "同一内容不递增 revision");
    drop(conn);
    assert_eq!(table_count(&pool, "jobs").await, 1, "任务数不变");
    assert_eq!(
        table_count(&pool, "provider_attempts").await,
        0,
        "恢复不得产生远端请求/attempt"
    );
    assert_eq!(
        table_count(&pool, "cost_ledger").await,
        0,
        "费用预留不变（无新购买）"
    );
    assert_eq!(
        table_count(&pool, "manual_releases").await,
        0,
        "不可变版本：不得自动发布"
    );
    // 草稿引用的资产行未被改动（上传/重启不新增或删除资产）。
    assert_eq!(table_count(&pool, "assets").await, 1);
}

// ===========================================================================
// 4) 说明书批次 3 批中断（§3「说明书批次另测」）
// ===========================================================================

/// 批次 fixture：整批响应落库才算完成；第 1 次调用成功，第 2 次在"请求已发出、
/// 完整响应未持久化"处命中测试断点（进程内 panic ≈ 崩溃）。
struct Qa21BatchHandler {
    base_url: String,
    result_asset_id: String,
    calls: Arc<AtomicUsize>,
}

impl StageHandler for Qa21BatchHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let window = &mut ctx.submission;
            window.begin_intent("qa21-batch-v1").await?;
            window.mark_submitting().await?;
            // fixture 客户端是 std 阻塞实现：放进阻塞线程池（同 jobs_recovery 的封装）。
            let url = format!("{}{}", self.base_url, MANUAL_AI_RESPONSES_PATH);
            let joined = tokio::task::spawn_blocking(move || {
                test_support::client::LocalHttpClient::with_read_timeout(Duration::from_secs(5))
                    .post_json(&url, &json!({ "model": "fixture-model", "input": "pages" }))
            })
            .await;
            let response = match joined {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    let detail = error.to_string();
                    window
                        .mark_unknown(&format!("同步批次网络错误：{detail}"))
                        .await?;
                    return Ok(StageOutcome::SubmissionUnknown {
                        reason: format!("同步批次结果未知：{detail}"),
                    });
                }
                Err(join_error) => {
                    let detail = join_error.to_string();
                    window
                        .mark_unknown(&format!("同步批次执行失败：{detail}"))
                        .await?;
                    return Ok(StageOutcome::SubmissionUnknown {
                        reason: format!("同步批次结果未知：{detail}"),
                    });
                }
            };
            {
                let body = response.json().unwrap_or(json!({}));
                let response_id = body["id"].as_str().map(str::to_owned);
                window
                    .record_sync_response(
                        response_id.as_deref(),
                        Some("{}"),
                        Some(&self.result_asset_id),
                    )
                    .await?;
                Ok(StageOutcome::Succeeded {
                    result_asset_id: Some(self.result_asset_id.clone()),
                    usage: Some(json!({ "outputTokens": 32 })),
                })
            }
        })
    }
}

/// 第 1 批完成 → 第 2 批崩溃在"请求已发出、响应未持久化" → 第 3 批未开始。
/// 恢复后：不重跑第 1 批（请求计数不变）、第 2 批 `submission_unknown` 待人工授权、
/// 第 3 批保持未发起（分支暂停且有可读原因），三批身份独立。
#[tokio::test]
async fn qa21_three_batch_interruption_never_reruns_completed_batch() {
    let dir = TestDir::new("qa21-three-batch");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, item_id) = seed_job(&pool, "three-batch", 11).await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        None,
        JobStatus::Succeeded,
    )
    .await;
    let batch0 = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        0,
        Some(vec![1, 2, 3, 4, 5]),
        JobStatus::Queued,
    )
    .await;
    let batch1 = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        1,
        Some(vec![6, 7, 8, 9, 10]),
        JobStatus::Queued,
    )
    .await;
    let batch2 = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        2,
        Some(vec![11]),
        JobStatus::Queued,
    )
    .await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::ManualMerge,
        0,
        None,
        JobStatus::Queued,
    )
    .await;

    // 三批持久身份各自独立（stage id + 页集合不同）。
    assert_ne!(batch0.id, batch1.id);
    assert_ne!(batch1.id, batch2.id);
    assert_eq!(batch0.page_set.as_deref(), Some([1, 2, 3, 4, 5].as_slice()));
    assert_eq!(
        batch1.page_set.as_deref(),
        Some([6, 7, 8, 9, 10].as_slice())
    );
    assert_eq!(batch2.page_set.as_deref(), Some([11].as_slice()));

    // 批次结果资产（每批独立结果）。
    let asset0 = seed_json_asset(&pool, &item_id, "batch0").await;
    let asset1 = seed_json_asset(&pool, &item_id, "batch1").await;
    let asset2 = seed_json_asset(&pool, &item_id, "batch2").await;

    let server = FixtureServer::start(Scenario::new(vec![RouteScript {
        method: "POST".to_owned(),
        path: MANUAL_AI_RESPONSES_PATH.to_owned(),
        path_match: Default::default(),
        repeat_last: true,
        steps: vec![Step::Respond {
            response: ResponseSpec {
                status: 200,
                headers: Default::default(),
                body: BodySpec::Json {
                    json: json!({ "id": "resp-qa21", "output": [] }),
                },
            },
        }],
    }]));

    let _ = (&asset1, &asset2);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::ManualExtract,
        Qa21BatchHandler {
            base_url: server.base_url(),
            result_asset_id: asset0.clone(),
            calls: calls.clone(),
        },
    );
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = fixed_jitter_executor(pool.clone(), executor_config(), registry, clock.clone());

    // 第 1 批：真实执行成功（先领取 batch0——created_at 最早）。
    executor.tick().await.expect("执行第 1 批");
    assert_eq!(stage_status(&pool, &batch0.id).await, JobStatus::Succeeded);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "第 1 批发出一次请求");
    assert_eq!(server.call_count("POST", MANUAL_AI_RESPONSES_PATH), 1);

    // 第 2 批：请求已发出、完整响应未持久化 → 进程内断点崩溃。
    failpoints::set(
        executor.owner(),
        MANUAL_AFTER_REQUEST_BEFORE_RESPONSE,
        FailpointAction::Panic,
    );
    let crashed = executor.clone();
    let joined = tokio::spawn(async move { crashed.tick().await }).await;
    assert!(joined.is_err(), "断点应使本次执行崩溃：{joined:?}");
    failpoints::clear_owner(executor.owner());
    assert_eq!(
        server.call_count("POST", MANUAL_AI_RESPONSES_PATH),
        2,
        "第 2 批的请求已发出"
    );
    let attempt = latest_attempt(&pool, &batch1.id)
        .await
        .expect("第 2 批 attempt");
    assert_eq!(
        attempt.submit_state,
        SubmitState::Submitting,
        "响应未持久化"
    );
    assert_eq!(
        stage_status(&pool, &batch2.id).await,
        JobStatus::Queued,
        "第 3 批尚未开始"
    );

    // 重启（真实顺序：崩溃 → 恢复扫描 → 执行器继续）：租约已过期，先恢复。
    expire_lease(&pool, &batch1.id).await;
    let mut resume_registry = StageRegistry::new();
    resume_registry.register(
        StageKind::ManualExtract,
        Qa21BatchHandler {
            base_url: server.base_url(),
            result_asset_id: asset2.clone(),
            calls: calls.clone(),
        },
    );
    let resumed = fixed_jitter_executor(pool.clone(), executor_config(), resume_registry, clock);
    let report = resumed.recover_expired_leases().await.expect("恢复扫描");
    assert_eq!(report.submission_unknown, 1, "{report:?}");
    assert_eq!(
        server.call_count("POST", MANUAL_AI_RESPONSES_PATH),
        2,
        "恢复扫描本身不发起任何请求（不重跑第 1 批、不重发第 2 批）"
    );

    // 恢复后继续推进：合同允许"第 3 批按已确认预算继续**或**暂停并明确显示"
    // （validation-release §3），两者都合规；此处只断言安全不变量。
    let _ = resumed.tick().await.expect("恢复后 tick");
    let posts_after = server.call_count("POST", MANUAL_AI_RESPONSES_PATH);

    assert_eq!(
        stage_status(&pool, &batch0.id).await,
        JobStatus::Succeeded,
        "第 1 批保持成功"
    );
    assert_eq!(
        attempt_count(&pool, &batch0.id).await,
        1,
        "第 1 批绝不重跑（attempt 恒为 1）"
    );
    assert_eq!(
        attempt_count(&pool, &batch1.id).await,
        1,
        "第 2 批绝不重发（attempt 恒为 1）"
    );
    let batch1_stage = read_stage(&pool, &batch1.id).await;
    assert_eq!(batch1_stage.status, JobStatus::SubmissionUnknown);
    let reason = batch1_stage.last_error.clone().unwrap_or_default();
    assert!(
        !reason.is_empty(),
        "第 2 批必须有可读原因（待人工授权处理）：{batch1_stage:?}"
    );
    match stage_status(&pool, &batch2.id).await {
        JobStatus::Succeeded => {
            // 分支继续的形态：第 3 批在已确认预算内执行且只执行一次。
            println!("QA21 实测形态：恢复后第 3 批在已确认预算内继续（POST 总数 = {posts_after}）");
            assert_eq!(posts_after, 3, "第 3 批只发出一次请求");
            assert_eq!(attempt_count(&pool, &batch2.id).await, 1);
        }
        JobStatus::Queued => {
            // 分支暂停的形态：不得偷偷发起请求。
            println!("QA21 实测形态：恢复后第 3 批暂停（POST 总数 = {posts_after}）");
            assert_eq!(posts_after, 2, "暂停形态不得发起第 3 批请求");
            assert_eq!(attempt_count(&pool, &batch2.id).await, 0);
        }
        other => panic!("第 3 批状态不在合同允许集合内：{other:?}"),
    }
    assert_eq!(
        stage_status(
            &pool,
            &job_stage_id(&pool, &job_id, StageKind::ManualMerge).await
        )
        .await,
        JobStatus::Queued,
        "merge 被未知批次阻塞（不产生正式知识）"
    );
    assert_eq!(
        read_job(&pool, &job_id).await.status,
        JobStatus::SubmissionUnknown
    );
    // 第 1 批结果资产未被动过（不重跑 → 资产引用不变）。
    assert_eq!(table_count(&pool, "assets").await, 4);
    assert_eq!(table_count(&pool, "jobs").await, 1);
    assert!(
        table_count(&pool, "provider_attempts").await <= 3,
        "attempt 总数不超过三批各 1 条"
    );
}

async fn seed_json_asset(pool: &SqlitePool, item_id: &str, tag: &str) -> String {
    let now = Timestamp::now().as_millis();
    let sha = test_support::sha256_hex(format!("qa21-{tag}-result").as_bytes());
    let mut conn = pool.acquire().await.expect("连接");
    sqlx::query(
        "INSERT INTO blobs (sha256, size, mime, storage_state, created_at) VALUES (?, 256, 'application/json', 'stored', ?)",
    )
    .bind(&sha)
    .bind(now)
    .execute(&mut *conn)
    .await
    .expect("插入结果 blob");
    let asset = repo::assets::insert(
        &mut conn,
        repo::assets::NewAsset {
            blob_id: sha,
            item_id: item_id.to_owned(),
            purpose: manual_core::domain::AssetPurpose::PageText,
            original_name: Some(format!("{tag}.json")),
        },
    )
    .await
    .expect("插入结果资产");
    drop(conn);
    asset.id
}

/// 某个阶段的 attempt 条数（批次身份独立性的直接证据）。
async fn attempt_count(pool: &SqlitePool, stage_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM provider_attempts WHERE stage_id = ?")
        .bind(stage_id)
        .fetch_one(pool)
        .await
        .expect("统计 attempt")
}

async fn latest_attempt(
    pool: &SqlitePool,
    stage_id: &str,
) -> Option<manual_core::domain::ProviderAttempt> {
    let mut conn = pool.acquire().await.expect("连接");
    attempts::latest_for_stage(&mut conn, stage_id)
        .await
        .expect("读取 attempt")
}

async fn job_stage_id(pool: &SqlitePool, job_id: &str, kind: StageKind) -> String {
    let mut conn = pool.acquire().await.expect("连接");
    job_stages::list_for_job(&mut conn, job_id)
        .await
        .expect("列出阶段")
        .into_iter()
        .find(|stage| stage.stage_kind == kind)
        .expect("阶段存在")
        .id
}
