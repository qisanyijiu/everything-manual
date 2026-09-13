//! QA 独立复验（T10 · BUG-003 修复；回合 11）。
//!
//! 独立性声明：本文件为 BUG-003 复验**新写**，不引用 `jobs_recovery.rs`、
//! `common` 或 `qa_t10_independent.rs` 的任何代码/夹具/断言；种子、处理器、断言与
//! 失败诊断全部由 QA 另写，落库事实尽量用**原始 SQL** 直查。计时一律用
//! `ManualClock`（可注入时钟）驱动，不真实 sleep；只有真实链路用例访问本机 fixture。
//!
//! 覆盖（对应回合 11 派发复验要点 1–5）：
//! 1. **真实轮询链路形态**：`tripo_poll` 无自身 attempt，task ID 与提交时刻来自
//!    同一 job 的 `tripo_submit` accepted 事实；推过 30 分钟阈值 → `needs_input`、
//!    task_id 保留、缺项可行动；此后 clock 推 2 小时反复 tick 全部 Idle（不无限
//!    轮询、不重购）。
//! 2. **反向核对**：轮询全程不得出现任何 poll 阶段 attempt；该 job attempt 总数
//!    恒为 1；note 中的等待秒数可由 accepted 提交事实的 `started_at` 复算。
//! 3. **非 accepted 事实不得起跑计时**：intent / submitting / unknown /
//!    仅有 `response_id` 的同步 receipt 逐一验证（job 级与阶段级）。
//! 4. **归属**：锚点是同一 job 的 accepted 提交事实，不是行创建时间；多 job 不串用。
//! 5. **阈值**：恰好 1800s 触发、1799s 不触发（阈值仍为 1800，未被降低）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use everything_manual::config::datadir;
use everything_manual::jobs::executor::fixed_jitter_executor;
use everything_manual::jobs::{
    Clock, ExecutorConfig, JobError, JobExecutor, ManualClock, StageContext, StageFuture,
    StageHandler, StageOutcome, StageRegistry, TickOutcome,
};
use everything_manual::storage::Database;
use everything_manual::storage::repo::job_stages::NewStage;
use everything_manual::storage::repo::{self, job_stages};
use manual_core::domain::{JobStatus, StageKind, SubmitState};
use manual_core::ids;
use manual_core::timestamps::Timestamp;
use serde_json::json;
use sqlx::{Row, SqlitePool};
use test_support::FixtureServer;
use test_support::client::LocalHttpClient;
use test_support::presets::TRIPO_TASKS_PREFIX;
use test_support::scenario::{RouteScript, Scenario, Step};

// ---------------------------------------------------------------------------
// QA 自写基础工具（不复用其它测试文件）
// ---------------------------------------------------------------------------

struct QaDir {
    path: PathBuf,
}

impl QaDir {
    fn new(tag: &str) -> Self {
        let unique = format!(
            "em-qa-r11-{tag}-{}-{}",
            std::process::id(),
            Timestamp::now().as_millis()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("创建 QA 临时目录");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for QaDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

async fn open_db(dir: &Path) -> Database {
    datadir::ensure_initialized(dir).expect("初始化 data-dir");
    Database::open_and_migrate(dir).await.expect("打开数据库")
}

/// QA 自写种子：item → blob/asset/document/preparation → snapshot → job。
async fn seed_job(pool: &SqlitePool, tag: &str) -> String {
    let now = Timestamp::now().as_millis();
    let mut conn = pool.acquire().await.expect("连接");
    let item_id = ids::new_id();
    let sha = test_support::assets::sha256_hex(format!("qa-r11-{tag}-source").as_bytes());
    sqlx::query(
        "INSERT INTO items (id, name, brand, model, variant, revision, archived_at, created_at, updated_at) \
         VALUES (?, ?, NULL, 'QAR11Model', NULL, 1, NULL, ?, ?)",
    )
    .bind(&item_id)
    .bind(format!("QA11 物品 {tag}"))
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .expect("插入 item");
    sqlx::query("INSERT INTO blobs (sha256, size, mime, storage_state, created_at) VALUES (?, 64, 'application/pdf', 'stored', ?)")
        .bind(&sha)
        .bind(now)
        .execute(&mut *conn)
        .await
        .expect("插入 blob");
    let asset_id = ids::new_id();
    sqlx::query("INSERT INTO assets (id, blob_id, item_id, purpose, original_name, created_at) VALUES (?, ?, ?, 'document', 'qa-r11.pdf', ?)")
        .bind(&asset_id)
        .bind(&sha)
        .bind(&item_id)
        .bind(now)
        .execute(&mut *conn)
        .await
        .expect("插入 asset");
    let document_id = ids::new_id();
    sqlx::query(
        "INSERT INTO documents (id, item_id, source_asset_id, source_sha256, title, source_url, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'QA11 说明书', NULL, ?, ?)",
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
    let preparation_id = ids::new_id();
    sqlx::query(
        "INSERT INTO preparations (id, document_id, source_sha256, state, page_count, revision, client_derived, created_at, updated_at) \
         VALUES (?, ?, ?, 'ready', 4, 1, 1, ?, ?)",
    )
    .bind(&preparation_id)
    .bind(&document_id)
    .bind(&sha)
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .expect("插入 preparation");
    let snapshot_id = ids::new_id();
    sqlx::query(
        "INSERT INTO generation_snapshots \
             (id, item_id, item_revision, preparation_id, photo_ids, photo_hashes, provider_config, \
              prompt_version, price_version, budgets, created_at) \
         VALUES (?, ?, 1, ?, '[\"p1\"]', '[\"h1\"]', '{}', 'prompt-v1', 'price-v1', '{}', ?)",
    )
    .bind(&snapshot_id)
    .bind(&item_id)
    .bind(&preparation_id)
    .bind(now)
    .execute(&mut *conn)
    .await
    .expect("插入 snapshot");
    let job = repo::jobs::create(
        &mut conn,
        repo::jobs::NewJob {
            item_id,
            snapshot_id,
        },
    )
    .await
    .expect("创建 job");
    drop(conn);
    job.id
}

async fn insert_stage(
    pool: &SqlitePool,
    job_id: &str,
    kind: StageKind,
    status: JobStatus,
) -> String {
    let mut conn = pool.acquire().await.expect("连接");
    let stage = job_stages::insert(
        &mut conn,
        NewStage {
            job_id: job_id.to_owned(),
            stage_kind: kind,
            batch_index: 0,
            page_set_json: None,
            input_hash: format!("qa-r11-{}-{}", kind.as_str(), ids::new_id()),
            status,
        },
        Timestamp::now(),
    )
    .await
    .expect("插入阶段");
    drop(conn);
    stage.id
}

/// 造 freeze/upload/submit 已成功的链（真实形态的常见前置）。
async fn seed_chain(pool: &SqlitePool, job_id: &str) -> String {
    for kind in [StageKind::FreezeInputs, StageKind::TripoUpload] {
        insert_stage(pool, job_id, kind, JobStatus::Succeeded).await;
    }
    insert_stage(pool, job_id, StageKind::TripoSubmit, JobStatus::Succeeded).await
}

struct StageRow {
    status: String,
    poll_count: i64,
    next_run_at: Option<i64>,
    needs_input: Option<serde_json::Value>,
    last_error: Option<String>,
}

async fn read_stage(pool: &SqlitePool, stage_id: &str) -> StageRow {
    let row = sqlx::query(
        "SELECT status, poll_count, next_run_at, needs_input_json, last_error \
           FROM job_stages WHERE id = ?",
    )
    .bind(stage_id)
    .fetch_one(pool)
    .await
    .expect("读阶段行");
    StageRow {
        status: row.try_get("status").expect("status"),
        poll_count: row.try_get("poll_count").expect("poll_count"),
        next_run_at: row.try_get("next_run_at").expect("next_run_at"),
        needs_input: row
            .try_get::<Option<String>, _>("needs_input_json")
            .expect("needs_input_json")
            .map(|text| serde_json::from_str(&text).expect("缺项 JSON 可解析")),
        last_error: row.try_get("last_error").expect("last_error"),
    }
}

async fn count_for(pool: &SqlitePool, sql: &'static str, bind: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(pool)
        .await
        .expect("计数")
}

async fn attempts_of_stage(pool: &SqlitePool, stage_id: &str) -> i64 {
    count_for(
        pool,
        "SELECT COUNT(*) FROM provider_attempts WHERE stage_id = ?",
        stage_id,
    )
    .await
}

async fn attempts_of_job(pool: &SqlitePool, job_id: &str) -> i64 {
    count_for(
        pool,
        "SELECT COUNT(*) FROM provider_attempts WHERE job_id = ?",
        job_id,
    )
    .await
}

async fn job_status(pool: &SqlitePool, job_id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM jobs WHERE id = ?")
        .bind(job_id)
        .fetch_one(pool)
        .await
        .expect("job status")
}

/// 造 accepted 且带远端 task ID 的提交事实（intent → submitting → task ID）。
async fn seed_accepted_remote(
    pool: &SqlitePool,
    job_id: &str,
    stage_id: &str,
    task_id: &str,
    started_at: Timestamp,
) -> String {
    let mut conn = pool.acquire().await.expect("连接");
    let attempt = repo::attempts::create_intent(
        &mut conn,
        repo::attempts::NewAttempt {
            job_id: job_id.to_owned(),
            stage_id: stage_id.to_owned(),
            request_hash: "qa-r11-aged".to_owned(),
        },
        started_at,
    )
    .await
    .expect("intent");
    repo::attempts::mark_submitting(&mut conn, &attempt.id, started_at)
        .await
        .expect("submitting");
    repo::attempts::record_remote_task_id(&mut conn, &attempt.id, task_id, started_at)
        .await
        .expect("远端 task ID");
    attempt.id
}

/// intent 未提交（非 accepted）。
async fn seed_intent_only(
    pool: &SqlitePool,
    job_id: &str,
    stage_id: &str,
    started_at: Timestamp,
) -> String {
    let mut conn = pool.acquire().await.expect("连接");
    let attempt = repo::attempts::create_intent(
        &mut conn,
        repo::attempts::NewAttempt {
            job_id: job_id.to_owned(),
            stage_id: stage_id.to_owned(),
            request_hash: "qa-r11-intent".to_owned(),
        },
        started_at,
    )
    .await
    .expect("intent");
    attempt.id
}

/// submitting（付费 POST 在途，非 accepted）。
async fn seed_submitting(
    pool: &SqlitePool,
    job_id: &str,
    stage_id: &str,
    started_at: Timestamp,
) -> String {
    let id = seed_intent_only(pool, job_id, stage_id, started_at).await;
    let mut conn = pool.acquire().await.expect("连接");
    repo::attempts::mark_submitting(&mut conn, &id, started_at)
        .await
        .expect("submitting");
    id
}

/// submission_unknown（非 accepted）。
async fn seed_unknown(
    pool: &SqlitePool,
    job_id: &str,
    stage_id: &str,
    started_at: Timestamp,
) -> String {
    let id = seed_submitting(pool, job_id, stage_id, started_at).await;
    let mut conn = pool.acquire().await.expect("连接");
    repo::attempts::mark_unknown(&mut conn, &id, "QA：响应未到", started_at)
        .await
        .expect("unknown");
    id
}

/// 同步 receipt：accepted + `response_id`、**无**远端 task ID。
async fn seed_sync_receipt(
    pool: &SqlitePool,
    job_id: &str,
    stage_id: &str,
    response_id: &str,
    started_at: Timestamp,
) -> String {
    let id = seed_submitting(pool, job_id, stage_id, started_at).await;
    let mut conn = pool.acquire().await.expect("连接");
    repo::attempts::record_sync_response(&mut conn, &id, Some(response_id), started_at)
        .await
        .expect("同步 receipt");
    id
}

fn r11_executor(
    pool: SqlitePool,
    registry: StageRegistry,
    clock: Arc<ManualClock>,
) -> Arc<JobExecutor> {
    fixed_jitter_executor(
        pool,
        ExecutorConfig {
            lease: Duration::from_secs(30),
            renew: Duration::from_secs(5),
            ..ExecutorConfig::default()
        },
        registry,
        clock,
    )
}

// ---------------------------------------------------------------------------
// 处理器
// ---------------------------------------------------------------------------

/// 真实形态轮询处理器：task ID 优先取 resume 提示，否则取**同一 job** 的
/// `tripo_submit` accepted 事实（与本卡 fixture 适配器同构；不建 attempt）。
/// 每次调用发 1 次真实 GET 到本机 fixture。
struct QaR11RealPoll {
    base: String,
    gets: Arc<AtomicUsize>,
}

impl StageHandler for QaR11RealPoll {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            let task_id = match ctx.known_remote_task_id().map(str::to_owned) {
                Some(id) => id,
                None => {
                    let mut conn = ctx.pool.acquire().await?;
                    repo::attempts::latest_accepted_for_job(
                        &mut conn,
                        &ctx.job.id,
                        StageKind::TripoSubmit,
                    )
                    .await?
                    .and_then(|attempt| attempt.remote_task_id)
                    .expect("必须已有已接受的远端 task ID")
                }
            };
            self.gets.fetch_add(1, Ordering::SeqCst);
            let url = format!("{}{}{}", self.base, TRIPO_TASKS_PREFIX, task_id);
            let joined = tokio::task::spawn_blocking(move || {
                LocalHttpClient::with_read_timeout(Duration::from_secs(300)).get(&url)
            })
            .await
            .map_err(|error| JobError::handler("qa-r11-poll", format!("join：{error}")))?;
            let response = joined.map_err(|error| {
                JobError::handler("qa-r11-poll", format!("查询远端任务失败：{error}"))
            })?;
            let body = response.json().unwrap_or(json!({}));
            match body["data"]["status"].as_str().unwrap_or("unknown") {
                "success" => Ok(StageOutcome::succeeded()),
                _ => Ok(StageOutcome::WaitingProvider),
            }
        })
    }
}

/// 永远等待（不访问网络）：用于锚点归属/边界的确定性断言。
struct QaR11AlwaysWaiting;

impl StageHandler for QaR11AlwaysWaiting {
    fn run<'a>(&'a self, _ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move { Ok(StageOutcome::WaitingProvider) })
    }
}

fn running_task_scenario(task_id: &str) -> Scenario {
    Scenario::new(vec![RouteScript {
        method: "GET".to_owned(),
        path: format!("{TRIPO_TASKS_PREFIX}{task_id}"),
        path_match: Default::default(),
        repeat_last: true,
        steps: vec![Step::Respond {
            response: test_support::ResponseSpec {
                status: 200,
                headers: Default::default(),
                body: test_support::BodySpec::Json {
                    json: json!({ "code": 0, "data": { "status": "running" } }),
                },
            },
        }],
    }])
}

// ---------------------------------------------------------------------------
// 1+2. 真实轮询链路形态：预算触发、task_id 保留、全程 1 条 attempt、不再重购
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_r11_real_shape_crosses_budget_keeps_task_and_never_repurchases() {
    let dir = QaDir::new("r11-real-shape");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let fixture = FixtureServer::start(running_task_scenario("qa-r11-task"));

    let job = seed_job(&pool, "r11-real-shape").await;
    let submit = seed_chain(&pool, &job).await;
    let poll = insert_stage(&pool, &job, StageKind::TripoPoll, JobStatus::Queued).await;
    // 真实形态：提交事实在 submit 阶段（1790s 前接受，配套节奏下第 4 次裁决越界）；
    // 轮询阶段没有、也不会有自己的 attempt。
    let aged = clock.now().checked_add_millis(-1_790_000).expect("时间");
    let submit_attempt = seed_accepted_remote(&pool, &job, &submit, "qa-r11-task", aged).await;
    assert_eq!(
        attempts_of_stage(&pool, &poll).await,
        0,
        "起始形态：轮询阶段不得有 attempt"
    );

    let gets = Arc::new(AtomicUsize::new(0));
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoPoll,
        QaR11RealPoll {
            base: fixture.base_url(),
            gets: gets.clone(),
        },
    );
    let executor = r11_executor(pool.clone(), registry, clock.clone());

    let mut delays: Vec<i64> = Vec::new();
    let mut ticks = 0usize;
    let mut crossing_note = String::new();
    for _ in 0..10 {
        let TickOutcome::Executed(report) = executor.tick().await.expect("tick") else {
            panic!("真实形态下轮询阶段应被领取执行");
        };
        ticks += 1;
        // 反向核对（要点 2）：任何时候都不允许轮询阶段出现自己的 attempt；
        // 该 job 的 attempt 总数恒为 1（无第二次付费提交）。
        assert_eq!(
            attempts_of_stage(&pool, &poll).await,
            0,
            "轮询阶段全程不得建 attempt"
        );
        assert_eq!(
            attempts_of_job(&pool, &job).await,
            1,
            "付费提交只允许发生一次"
        );
        let stage = read_stage(&pool, &poll).await;
        match stage.status.as_str() {
            "waiting_provider" => {
                let next = stage
                    .next_run_at
                    .expect("waiting_provider 必须写 next_run_at");
                let delay = next - clock.now().as_millis();
                delays.push(delay);
                clock.advance_millis(delay);
            }
            "needs_input" => {
                crossing_note = report.note.clone().unwrap_or_default();
                break;
            }
            other => panic!("真实形态下意外状态 {other}（第 {ticks} 次）"),
        }
    }

    let stage = read_stage(&pool, &poll).await;
    assert_eq!(
        stage.status, "needs_input",
        "推过 30 分钟预算必须转 needs_input（真实链路形态）"
    );
    assert_eq!(ticks, 4, "1790s 起点 + 3/6/12s 节奏后第 4 次裁决越界");
    assert_eq!(
        delays,
        vec![3_000_i64, 6_000, 12_000],
        "预算之前轮询节奏 3/6/12 秒（锚点修复不得改变节奏）"
    );
    // 反向核对（要点 2）：note 中的等待秒数可由 accepted 提交事实的 started_at 复算
    // （1790 + 3 + 6 + 12 = 1811），且列出保留的 task_id。
    assert!(
        crossing_note.contains("1811s"),
        "note 应包含由提交事实复算的等待秒数：{crossing_note}"
    );
    assert!(
        crossing_note.contains("qa-r11-task"),
        "note 应列出保留的 task_id：{crossing_note}"
    );
    // 缺项可行动（要点 1）。
    let items = stage.needs_input.clone().expect("缺项必须落库");
    assert_eq!(items[0]["code"], "remote_wait_budget_exceeded");
    let message = items[0]["message"].as_str().unwrap_or_default().to_owned();
    assert!(message.contains("远端任务 ID 已保留"), "{message}");
    assert!(message.contains("不重新购买"), "{message}");
    assert!(
        stage
            .last_error
            .clone()
            .unwrap_or_default()
            .contains("task_id 已保留"),
        "last_error 应说明 task_id 保留"
    );
    // task_id 保留在 submit 事实里（恢复只查询、不重购的物质基础）。
    let attempt = repo::attempts::latest_for_stage(&mut pool.acquire().await.unwrap(), &submit)
        .await
        .expect("读 attempt")
        .expect("提交事实仍在");
    assert_eq!(attempt.id, submit_attempt);
    assert_eq!(attempt.submit_state, SubmitState::Accepted);
    assert_eq!(attempt.remote_task_id.as_deref(), Some("qa-r11-task"));
    assert_eq!(attempts_of_stage(&pool, &poll).await, 0);
    assert_eq!(attempts_of_job(&pool, &job).await, 1);
    assert_eq!(job_status(&pool, &job).await, "needs_input");
    // 真实 HTTP 计数：4 轮各 1 次查询；除查询外零请求（尤其零付费 POST）。
    assert_eq!(gets.load(Ordering::SeqCst), 4, "每轮轮询 1 次真实 GET");
    assert_eq!(
        fixture.call_count("GET", &format!("{TRIPO_TASKS_PREFIX}qa-r11-task")),
        4
    );
    assert_eq!(
        fixture.request_total(),
        4,
        "不得有任何别的请求（含付费 POST）"
    );
    assert_eq!(stage.poll_count, 3, "预算前完成 3 次等待推进");

    // 不无限轮询、不重购：时钟推 2 小时并连续 tick 全部 Idle，计数不变。
    clock.advance_millis(7_200_000);
    for _ in 0..5 {
        assert!(
            matches!(executor.tick().await.expect("tick"), TickOutcome::Idle),
            "needs_input 之后不得再被领取"
        );
    }
    assert_eq!(
        gets.load(Ordering::SeqCst),
        4,
        "恢复扫描不得产生新的远端查询"
    );
    assert_eq!(fixture.request_total(), 4, "不得重新购买/重发请求");
    assert_eq!(attempts_of_job(&pool, &job).await, 1);
    assert_eq!(attempts_of_stage(&pool, &poll).await, 0);
    assert_eq!(read_stage(&pool, &poll).await.status, "needs_input");
    assert_eq!(job_status(&pool, &job).await, "needs_input");

    // 恢复入口（T15/T17 的端点在服务端的对应物，超出本卡范围；这里用原始 SQL 模拟
    // "用户补齐后触发继续"）：只允许按保留的 task ID **查询**，不得产生第二次付费。
    // 预算已越界（1811s ≥ 1800），补齐预算后重新领取仍会立即回到 needs_input——这正是
    // "不自动继续等待"的语义；关键断言是请求仍是同一 task 的 GET，且零新 attempt。
    sqlx::query("UPDATE job_stages SET status = 'queued' WHERE id = ?")
        .bind(&poll)
        .execute(&pool)
        .await
        .expect("模拟恢复入口");
    let TickOutcome::Executed(resume_report) = executor.tick().await.expect("tick") else {
        panic!("恢复后轮询阶段应被领取执行");
    };
    assert_eq!(
        resume_report.status,
        Some(JobStatus::NeedsInput),
        "预算已越界：恢复查询后仍回到 needs_input"
    );
    assert_eq!(
        gets.load(Ordering::SeqCst),
        5,
        "恢复必须按保留的 task ID 查询一次"
    );
    assert_eq!(
        fixture.call_count("GET", &format!("{TRIPO_TASKS_PREFIX}qa-r11-task")),
        5,
        "查询路径仍是同一 task ID"
    );
    assert_eq!(fixture.request_total(), 5, "不得重新购买（零 POST）");
    assert_eq!(attempts_of_job(&pool, &job).await, 1, "不得新增 attempt");
    assert_eq!(attempts_of_stage(&pool, &poll).await, 0);
    drop(database);
}

// ---------------------------------------------------------------------------
// 3. 非 accepted 事实不得起跑计时（job 级：intent / submitting / unknown /
//    仅有 response_id 的同步 receipt；阶段级：非 accepted 阶段 attempt 被拒后回退）
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_r11_only_accepted_remote_facts_start_the_budget() {
    let dir = QaDir::new("r11-negative-anchors");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let aged = clock.now().checked_add_millis(-1_860_000).expect("时间"); // 31 分钟

    let mut registry = StageRegistry::new();
    registry.register(StageKind::TripoPoll, QaR11AlwaysWaiting);
    let executor = r11_executor(pool.clone(), registry, clock.clone());

    // (a)(b)(c) submit 阶段只有 intent / submitting / unknown —— 都非 accepted。
    for state in ["intent", "submitting", "unknown"] {
        let job = seed_job(&pool, &format!("r11-neg-{state}")).await;
        let submit = seed_chain(&pool, &job).await;
        let poll = insert_stage(&pool, &job, StageKind::TripoPoll, JobStatus::Queued).await;
        match state {
            "intent" => {
                seed_intent_only(&pool, &job, &submit, aged).await;
            }
            "submitting" => {
                seed_submitting(&pool, &job, &submit, aged).await;
            }
            _ => {
                seed_unknown(&pool, &job, &submit, aged).await;
            }
        }
        let TickOutcome::Executed(report) = executor.tick().await.expect("tick") else {
            panic!("{state}：轮询阶段应被领取执行");
        };
        assert_eq!(
            report.status,
            Some(JobStatus::WaitingProvider),
            "{state}：非 accepted 事实不得起跑 30 分钟预算"
        );
        let stage = read_stage(&pool, &poll).await;
        assert_eq!(stage.status, "waiting_provider", "{state}");
        assert!(stage.needs_input.is_none(), "{state}：不得有缺项");
        assert_eq!(
            stage.next_run_at.expect("next_run_at") - clock.now().as_millis(),
            3_000,
            "{state}：按正常节奏 3s 后重查"
        );
        assert_eq!(job_status(&pool, &job).await, "waiting_provider", "{state}");
        assert_eq!(attempts_of_stage(&pool, &poll).await, 0, "{state}");
        assert_eq!(attempts_of_job(&pool, &job).await, 1, "{state}");
    }

    // (d) 同步 receipt：accepted 但只有 response_id、无远端 task ID → 不得起跑。
    {
        let job = seed_job(&pool, "r11-neg-sync").await;
        let submit = seed_chain(&pool, &job).await;
        let poll = insert_stage(&pool, &job, StageKind::TripoPoll, JobStatus::Queued).await;
        let id = seed_sync_receipt(&pool, &job, &submit, "resp-only-1", aged).await;
        let TickOutcome::Executed(report) = executor.tick().await.expect("tick") else {
            panic!("sync：轮询阶段应被领取执行");
        };
        assert_eq!(
            report.status,
            Some(JobStatus::WaitingProvider),
            "同步 receipt（无 task ID）不得起跑预算"
        );
        let stage = read_stage(&pool, &poll).await;
        assert_eq!(stage.status, "waiting_provider");
        assert!(stage.needs_input.is_none());
        assert_eq!(
            stage.next_run_at.expect("next_run_at") - clock.now().as_millis(),
            3_000
        );
        let attempt = repo::attempts::get(&mut pool.acquire().await.unwrap(), &id)
            .await
            .expect("读 attempt")
            .expect("receipt 仍在");
        assert_eq!(attempt.submit_state, SubmitState::Accepted);
        assert_eq!(attempt.response_id.as_deref(), Some("resp-only-1"));
        assert_eq!(
            attempt.remote_task_id, None,
            "同步 receipt 不应有远端 task ID"
        );
    }

    // (e) 阶段级：轮询阶段自身带一个 31 分钟前的 **accepted 但只有 response_id**
    // attempt（阶段级候选，必须被 accepted-remote-fact 过滤器拒绝）——应回退到同一
    // job 的 fresh accepted 提交事实（等待 0s），而不是用该阶段事实起跑预算。
    {
        let job = seed_job(&pool, "r11-neg-stage").await;
        let submit = seed_chain(&pool, &job).await;
        let poll = insert_stage(&pool, &job, StageKind::TripoPoll, JobStatus::Queued).await;
        seed_accepted_remote(&pool, &job, &submit, "task-stage-fallback", clock.now()).await;
        seed_sync_receipt(&pool, &job, &poll, "resp-stage-only", aged).await;
        let TickOutcome::Executed(report) = executor.tick().await.expect("tick") else {
            panic!("stage-fallback：轮询阶段应被领取执行");
        };
        assert_eq!(
            report.status,
            Some(JobStatus::WaitingProvider),
            "阶段自身无远端 task ID 的 accepted attempt 不得起跑计时"
        );
        let note = report.note.clone().unwrap_or_default();
        assert!(
            note.contains("远端已等待 0s"),
            "应回退到 fresh accepted 提交事实（等 0s）：{note}"
        );
        let stage = read_stage(&pool, &poll).await;
        assert_eq!(stage.status, "waiting_provider");
        assert!(stage.needs_input.is_none());
    }
    drop(database);
}

// ---------------------------------------------------------------------------
// 4. 归属：锚点是同一 job 的 accepted 提交事实（不是行创建时间）
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_r11_anchor_is_accepted_submit_fact_not_row_timestamps() {
    let dir = QaDir::new("r11-anchor-provenance");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();
    let clock = Arc::new(ManualClock::new(Timestamp::now()));

    let mut registry = StageRegistry::new();
    registry.register(StageKind::TripoPoll, QaR11AlwaysWaiting);
    let executor = r11_executor(pool.clone(), registry, clock.clone());

    // job A：行创建/更新时间被回拨 40 分钟，但 accepted 提交事实"刚刚"（预算内）。
    // 若实现用行时间戳计时 → 会错误触发；正确实现 → waiting_provider 且等 0s。
    let job_a = seed_job(&pool, "r11-prov-a").await;
    let submit_a = seed_chain(&pool, &job_a).await;
    let poll_a = insert_stage(&pool, &job_a, StageKind::TripoPoll, JobStatus::Queued).await;
    seed_accepted_remote(&pool, &job_a, &submit_a, "task-a-fresh", clock.now()).await;
    let back = clock
        .now()
        .checked_add_millis(-2_400_000)
        .expect("时间")
        .as_millis();
    sqlx::query("UPDATE jobs SET created_at = ?, updated_at = ? WHERE id = ?")
        .bind(back)
        .bind(back)
        .bind(&job_a)
        .execute(&pool)
        .await
        .expect("回拨 job 行时间");
    sqlx::query("UPDATE job_stages SET created_at = ?, updated_at = ? WHERE job_id = ?")
        .bind(back)
        .bind(back)
        .bind(&job_a)
        .execute(&pool)
        .await
        .expect("回拨阶段行时间");

    // job B：行是新的，但 accepted 提交事实老化 31 分钟 → needs_input。
    let job_b = seed_job(&pool, "r11-prov-b").await;
    let submit_b = seed_chain(&pool, &job_b).await;
    let poll_b = insert_stage(&pool, &job_b, StageKind::TripoPoll, JobStatus::Queued).await;
    let aged = clock.now().checked_add_millis(-1_860_000).expect("时间");
    seed_accepted_remote(&pool, &job_b, &submit_b, "task-b-old", aged).await;

    let mut notes: HashMap<String, String> = HashMap::new();
    for _ in 0..2 {
        if let TickOutcome::Executed(report) = executor.tick().await.expect("tick") {
            notes.insert(
                report.stage_id.clone(),
                report.note.clone().unwrap_or_default(),
            );
        }
    }
    let stage_a = read_stage(&pool, &poll_a).await;
    assert_eq!(
        stage_a.status, "waiting_provider",
        "行创建时间回拨 40 分钟不得触发预算（锚点必须是 accepted 提交事实）"
    );
    assert!(stage_a.needs_input.is_none());
    let note_a = notes.get(&poll_a).cloned().unwrap_or_default();
    assert!(
        note_a.contains("远端已等待 0s"),
        "等待秒数应来自 fresh accepted 提交事实：{note_a}"
    );
    let stage_b = read_stage(&pool, &poll_b).await;
    assert_eq!(
        stage_b.status, "needs_input",
        "accepted 提交事实老化 31 分钟 → needs_input"
    );
    assert_eq!(
        stage_b.needs_input.clone().expect("缺项")[0]["code"],
        "remote_wait_budget_exceeded"
    );
    assert_eq!(job_status(&pool, &job_b).await, "needs_input");
    drop(database);
}

// ---------------------------------------------------------------------------
// 5. 阈值边界（恰好 1800s 触发 / 1799s 不触发）与多 job 不串用
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_r11_threshold_boundary_and_cross_job_isolation() {
    let dir = QaDir::new("r11-boundary");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();
    let clock = Arc::new(ManualClock::new(Timestamp::now()));

    let mut registry = StageRegistry::new();
    registry.register(StageKind::TripoPoll, QaR11AlwaysWaiting);
    let executor = r11_executor(pool.clone(), registry, clock.clone());

    // X：恰好 1800s（>= 语义应触发）；Y：1799s（不触发）；Z：刚刚（不触发）。
    let mut jobs = Vec::new();
    for (tag, age_millis) in [
        ("r11-edge-x", -1_800_000_i64),
        ("r11-edge-y", -1_799_000),
        ("r11-edge-z", 0),
    ] {
        let job = seed_job(&pool, tag).await;
        let submit = seed_chain(&pool, &job).await;
        let poll = insert_stage(&pool, &job, StageKind::TripoPoll, JobStatus::Queued).await;
        let started = clock.now().checked_add_millis(age_millis).expect("时间");
        seed_accepted_remote(&pool, &job, &submit, tag, started).await;
        jobs.push((tag.to_owned(), job, poll));
    }

    for _ in 0..3 {
        let _ = executor.tick().await.expect("tick");
    }
    let (_, job_x, poll_x) = &jobs[0];
    let (_, job_y, poll_y) = &jobs[1];
    let (_, job_z, poll_z) = &jobs[2];

    let stage_x = read_stage(&pool, poll_x).await;
    assert_eq!(
        stage_x.status, "needs_input",
        "恰好 1800s 必须触发（阈值语义 >=；常量仍为 1800）"
    );
    let stage_y = read_stage(&pool, poll_y).await;
    assert_eq!(
        stage_y.status, "waiting_provider",
        "1799s 不得触发（阈值未被降低）"
    );
    assert_eq!(
        stage_y.next_run_at.expect("next_run_at") - clock.now().as_millis(),
        3_000
    );
    let stage_z = read_stage(&pool, poll_z).await;
    assert_eq!(
        stage_z.status, "waiting_provider",
        "X 的老化事实不得串到 Z（cross-job 不污染）"
    );
    assert_eq!(
        stage_z.next_run_at.expect("next_run_at") - clock.now().as_millis(),
        3_000
    );
    assert!(stage_y.needs_input.is_none() && stage_z.needs_input.is_none());
    assert_eq!(job_status(&pool, job_x).await, "needs_input");
    assert_eq!(job_status(&pool, job_y).await, "waiting_provider");
    assert_eq!(job_status(&pool, job_z).await, "waiting_provider");
    for (tag, job, poll) in &jobs {
        assert_eq!(
            attempts_of_job(&pool, job).await,
            1,
            "{tag}：不得新增 attempt"
        );
        assert_eq!(attempts_of_stage(&pool, poll).await, 0, "{tag}");
    }
    drop(database);
}
