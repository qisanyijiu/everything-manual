//! QA 独立验收测试（T10 持久任务执行器；回合 10）。
//!
//! 独立性声明：本文件**不引用** `crates/server/tests/jobs_recovery.rs` 的任何代码、
//! 夹具或断言，也不 import `common` 模块；种子数据、断点动作、子进程入口、
//! 断言与失败诊断全部由 QA 另写。落库事实尽量用**原始 SQL** 直查（不经过仓储解析），
//! 以便发现"仓储层自证"掩盖的漂移。
//!
//! 覆盖：
//! 1. 进程级 SIGKILL（`libc::kill(SIGKILL)`）注入：付费 POST 已发出、响应未到；
//! 2. 进程级 SIGKILL：已知远端 task ID 的查询阶段（重启只查询、不重发付费 POST）；
//! 3. 租约过期的旧 worker：可保存不可变事实、不能推进状态、不能解锁后续阶段；
//!    下手接管（epoch + 1）后可以推进；
//! 4. DAG 依赖边落库与解锁条件；`(job, stage_kind, batch_index)` 唯一性；
//! 5. 并发上限 2（远端生成）/ 2（说明书批次）在领取谓词内生效；
//! 6. 同一阶段只允许一个未对账 attempt（部分唯一索引）。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use everything_manual::config::datadir;
use everything_manual::jobs::executor::fixed_jitter_executor;
use everything_manual::jobs::{
    ExecutorConfig, JobError, JobExecutor, ManualClock, StageContext, StageFuture, StageHandler,
    StageOutcome, StageRegistry,
};
use everything_manual::storage::Database;
use everything_manual::storage::repo::job_stages::{
    ClaimParams, LeaseGuard, NewStage, StageAdvance,
};
use everything_manual::storage::repo::{self, job_stages};
use manual_core::domain::{JobStatus, StageKind};
use manual_core::ids;
use manual_core::timestamps::Timestamp;
use serde_json::json;
use sqlx::SqlitePool;
use test_support::FixtureServer;
use test_support::client::LocalHttpClient;
use test_support::presets::{TRIPO_SUBMIT_PATH, TRIPO_TASKS_PREFIX};
use test_support::scenario::{RouteScript, Scenario, Step};

// ---------------------------------------------------------------------------
// QA 自写基础工具（不复用 RD 的测试模块）
// ---------------------------------------------------------------------------

struct QaDir {
    path: PathBuf,
}

impl QaDir {
    fn new(tag: &str) -> Self {
        let unique = format!(
            "em-qa-t10-{tag}-{}-{}",
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

/// QA 自写种子：插入执行器测试所需的引用链与 job（不 import RD 的 seed_job）。
async fn seed_job(pool: &SqlitePool, tag: &str) -> String {
    let now = Timestamp::now().as_millis();
    let mut conn = pool.acquire().await.expect("连接");
    let item_id = ids::new_id();
    let sha = test_support::assets::sha256_hex(format!("qa-{tag}-source").as_bytes());
    sqlx::query(
        "INSERT INTO items (id, name, brand, model, variant, revision, archived_at, created_at, updated_at) \
         VALUES (?, ?, NULL, 'QAModel', NULL, 1, NULL, ?, ?)",
    )
    .bind(&item_id)
    .bind(format!("QA 物品 {tag}"))
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
    sqlx::query("INSERT INTO assets (id, blob_id, item_id, purpose, original_name, created_at) VALUES (?, ?, ?, 'document', 'qa.pdf', ?)")
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
         VALUES (?, ?, ?, ?, 'QA 说明书', NULL, ?, ?)",
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
    batch_index: i64,
    status: JobStatus,
) -> String {
    let mut conn = pool.acquire().await.expect("连接");
    let stage = job_stages::insert(
        &mut conn,
        NewStage {
            job_id: job_id.to_owned(),
            stage_kind: kind,
            batch_index,
            page_set_json: (kind == StageKind::ManualExtract)
                .then(|| format!("[{}]", batch_index + 1)),
            input_hash: format!("qa-hash-{}-{batch_index}", kind.as_str()),
            status,
        },
        Timestamp::now(),
    )
    .await
    .expect("插入阶段");
    drop(conn);
    stage.id
}

async fn raw_stage_status(pool: &SqlitePool, stage_id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM job_stages WHERE id = ?")
        .bind(stage_id)
        .fetch_one(pool)
        .await
        .expect("读取阶段状态")
}

async fn raw_count(pool: &SqlitePool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.expect("计数")
}

fn qa_claim_params(owner: &str, lease: Duration) -> ClaimParams {
    ClaimParams {
        owner: owner.to_owned(),
        now: Timestamp::now(),
        lease,
        remote_generation_limit: 2,
        manual_ai_batch_limit: 2,
    }
}

async fn wait_until(timeout: Duration, mut check: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("等待超时：{what}");
}

/// 某阶段依赖的 stage id（原始 SQL 直查）。
async fn deps_of(pool: &SqlitePool, stage_id: &str) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT depends_on_stage_id FROM job_stage_deps WHERE stage_id = ? ORDER BY depends_on_stage_id",
    )
    .bind(stage_id)
    .fetch_all(pool)
    .await
    .expect("读依赖");
    rows.into_iter().map(|(id,)| id).collect()
}

// ---------------------------------------------------------------------------
// 1+2. 进程级 SIGKILL 注入（QA 自写子进程入口与夹具）
// ---------------------------------------------------------------------------

const QA_ENV_ROLE: &str = "QA_T10_CHILD_ROLE";
const QA_ENV_DATA_DIR: &str = "QA_T10_CHILD_DATA_DIR";
const QA_ENV_FIXTURE: &str = "QA_T10_CHILD_FIXTURE";

/// QA 自写处理器：真实发一次付费 POST（fixture 挂起不响应），然后被外部 kill -9。
/// 处理器**不自行推进状态**（与真实适配器同构：状态推进属执行器）。
struct QaHangPaySubmit {
    base: String,
    posts: Arc<AtomicUsize>,
}

impl StageHandler for QaHangPaySubmit {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            // contracts §5 顺序：先 intent，再 submitting，最后才允许发请求。
            ctx.submission
                .begin_intent("qa-independent-pay-request")
                .await?;
            ctx.submission.mark_submitting().await?;
            self.posts.fetch_add(1, Ordering::SeqCst);
            let url = format!("{}{}", self.base, TRIPO_SUBMIT_PATH);
            let stage_id = ctx.stage.id.clone();
            let response = tokio::task::spawn_blocking(move || {
                LocalHttpClient::with_read_timeout(Duration::from_secs(300))
                    .post_json(&url, &json!({ "front": "qa-token-front" }))
            })
            .await
            .map_err(|error| JobError::handler(&stage_id, format!("join：{error}")))?;
            match response {
                Err(detail) => {
                    let reason = format!("QA 付费 POST 网络错误：{detail}");
                    ctx.submission.mark_unknown(&reason).await?;
                    Ok(StageOutcome::SubmissionUnknown { reason })
                }
                Ok(_) => Ok(StageOutcome::succeeded()),
            }
        })
    }
}

/// QA 自写处理器：按**已知远端 ID** 继续查询（绝不重新提交）。
struct QaPollKnownTask {
    base: String,
    gets: Arc<AtomicUsize>,
}

impl StageHandler for QaPollKnownTask {
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
            .map_err(|error| JobError::handler("qa-poll", format!("join：{error}")))?;
            let response = joined.map_err(|error| {
                JobError::handler("qa-poll", format!("查询远端任务失败：{error}"))
            })?;
            let body = response.json().unwrap_or(json!({}));
            match body["data"]["status"].as_str().unwrap_or("unknown") {
                "success" => Ok(StageOutcome::succeeded()),
                _ => Ok(StageOutcome::WaitingProvider),
            }
        })
    }
}

/// 子进程入口（无环境变量时是空操作，可被普通测试运行安全包含）。
#[test]
fn qa_t10_child_entry() {
    let Ok(role) = std::env::var(QA_ENV_ROLE) else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()
        .expect("子进程运行时");
    runtime.block_on(async move {
        let data_dir = PathBuf::from(std::env::var(QA_ENV_DATA_DIR).expect("data-dir"));
        let fixture = std::env::var(QA_ENV_FIXTURE).expect("fixture 地址");
        let database = Database::open_and_migrate(&data_dir)
            .await
            .expect("子进程打开数据库");
        let mut registry = StageRegistry::new();
        registry.register(
            StageKind::TripoSubmit,
            QaHangPaySubmit {
                base: fixture.clone(),
                posts: Arc::new(AtomicUsize::new(0)),
            },
        );
        registry.register(
            StageKind::TripoPoll,
            QaPollKnownTask {
                base: fixture,
                gets: Arc::new(AtomicUsize::new(0)),
            },
        );
        let config = ExecutorConfig {
            lease: Duration::from_millis(1_000),
            renew: Duration::from_millis(200),
            ..ExecutorConfig::default()
        };
        config.validate().expect("子进程执行器配置合法");
        let executor = JobExecutor::new(database.pool().clone(), config, registry);
        let handle = executor.start();
        match role.as_str() {
            // 持续运行，等父进程 kill -9（真实硬崩溃）。
            "run" => std::future::pending::<()>().await,
            // 一次性恢复扫描 + 若干 tick 后正常退出。
            "recover" => {
                let report = executor.recover_expired_leases().await.expect("恢复扫描");
                eprintln!("[qa-child] recovery {}", report.summary());
                for _ in 0..3 {
                    let _ = executor.tick().await;
                }
                handle.shutdown().await;
            }
            other => panic!("未知 QA 子进程角色：{other}"),
        }
    });
    std::process::exit(0);
}

fn hanging_post_scenario() -> Scenario {
    Scenario::new(vec![RouteScript {
        method: "POST".to_owned(),
        path: TRIPO_SUBMIT_PATH.to_owned(),
        path_match: Default::default(),
        repeat_last: true,
        // 读完请求后不写任何字节：进程停在"付费 POST 已发出、响应未到"。
        steps: vec![Step::Timeout { hold_ms: 120_000 }],
    }])
}

fn hanging_get_scenario(task_id: &str) -> Scenario {
    Scenario::new(vec![RouteScript {
        method: "GET".to_owned(),
        path: format!("{TRIPO_TASKS_PREFIX}{task_id}"),
        path_match: Default::default(),
        repeat_last: true,
        steps: vec![Step::Timeout { hold_ms: 120_000 }],
    }])
}

fn healthy_get_scenario(task_id: &str) -> Scenario {
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
                    json: json!({ "code": 0, "data": { "status": "success" } }),
                },
            },
        }],
    }])
}

struct QaChild {
    child: Option<Child>,
}

impl QaChild {
    fn spawn(role: &str, dir: &Path, fixture: &str) -> Self {
        let child = Command::new(std::env::current_exe().expect("测试二进制"))
            .args(["--exact", "qa_t10_child_entry", "--nocapture"])
            .env(QA_ENV_ROLE, role)
            .env(QA_ENV_DATA_DIR, dir)
            .env(QA_ENV_FIXTURE, fixture)
            .env_remove("EM_TEST_FAILPOINT")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("启动 QA 子进程");
        Self { child: Some(child) }
    }

    fn pid(&self) -> i32 {
        self.child.as_ref().expect("子进程存活").id() as i32
    }

    /// 用 libc 直接发 SIGKILL（不 shell 出 `kill`，不经过任何优雅退出路径）。
    fn sigkill(&mut self) {
        let pid = self.pid();
        let rc = unsafe { libc::kill(pid, libc::SIGKILL) };
        assert_eq!(rc, 0, "SIGKILL 失败（pid={pid}）");
        if let Some(mut child) = self.child.take() {
            let status = child.wait().expect("回收子进程");
            assert!(
                !status.success(),
                "被 SIGKILL 的进程不应正常退出：{status:?}"
            );
        }
    }
}

impl Drop for QaChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// 正常运行一个 QA 子进程并等待其退出。
async fn run_child_once(role: &str, dir: &Path, fixture: &str) -> bool {
    let role = role.to_owned();
    let dir = dir.to_path_buf();
    let fixture = fixture.to_owned();
    tokio::task::spawn_blocking(move || {
        Command::new(std::env::current_exe().expect("测试二进制"))
            .args(["--exact", "qa_t10_child_entry", "--nocapture"])
            .env(QA_ENV_ROLE, &role)
            .env(QA_ENV_DATA_DIR, &dir)
            .env(QA_ENV_FIXTURE, &fixture)
            .env_remove("EM_TEST_FAILPOINT")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("运行 QA 子进程")
            .success()
    })
    .await
    .expect("子进程任务")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa_sigkill_paid_post_sent_response_lost_never_repurchases() {
    let dir = QaDir::new("kill-pay");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();
    let job_id = seed_job(&pool, "kill-pay").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::TripoUpload,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let submit = insert_stage(&pool, &job_id, StageKind::TripoSubmit, 0, JobStatus::Queued).await;
    let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    drop(database);

    let fixture = FixtureServer::start(hanging_post_scenario());
    let mut child = QaChild::spawn("run", dir.path(), &fixture.base_url());
    wait_until(
        Duration::from_secs(30),
        || fixture.call_count("POST", TRIPO_SUBMIT_PATH) == 1,
        "子进程发出第 1 次付费 POST",
    )
    .await;
    let pid = child.pid();

    // 领取/事实事务不跨 HTTP：付费请求在途时，另一连接仍能完成写事务（不被长期占用的写锁阻塞）。
    let probe_started = Instant::now();
    {
        let database = Database::open_and_migrate(dir.path())
            .await
            .expect("在途时可打开库");
        let pool = database.pool().clone();
        sqlx::query(
            "INSERT INTO audit_events (id, entity_type, entity_id, actor, action, result, metadata_json, created_at) \
             VALUES (?, 'qa_probe', 'qa-inflight', 'qa', 'qa_inflight_write_probe', 'ok', NULL, ?)",
        )
        .bind(ids::new_id())
        .bind(Timestamp::now().as_millis())
        .execute(&pool)
        .await
        .expect("在途 HTTP 期间仍可写入（不跨 HTTP 持事务）");
        drop(database);
    }
    eprintln!(
        "[qa] 付费 POST 在途时的独立写事务耗时={:?}（pid={pid}）",
        probe_started.elapsed()
    );
    assert!(
        probe_started.elapsed() < Duration::from_secs(3),
        "在途 HTTP 不应长期持有写事务"
    );

    child.sigkill();
    eprintln!(
        "[qa] kill -9 pid={pid}；fixture 记录总数={}",
        fixture.request_total()
    );

    // 崩溃现场（kill 之后、恢复之前）：attempt 已 submitting、无远端 ID。
    {
        let database = Database::open_and_migrate(dir.path())
            .await
            .expect("重开库");
        let pool = database.pool().clone();
        let state: String = sqlx::query_scalar(
            "SELECT submit_state FROM provider_attempts WHERE stage_id = ? ORDER BY started_at DESC LIMIT 1",
        )
        .bind(&submit)
        .fetch_one(&pool)
        .await
        .expect("读取 attempt");
        assert_eq!(
            state, "submitting",
            "崩溃现场应为 submitting（POST 已发出、响应未到）"
        );
        let status = raw_stage_status(&pool, &submit).await;
        assert_eq!(status, "running", "崩溃现场阶段应为 running（未推进）");
        drop(database);
    }

    // 等租约过期（子进程租约 1s，真实时间）。
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    // 重启恢复：不重发付费 POST。
    assert!(
        run_child_once("recover", dir.path(), &fixture.base_url()).await,
        "恢复子进程应正常退出"
    );

    let database = Database::open_and_migrate(dir.path())
        .await
        .expect("重开库");
    let pool = database.pool().clone();
    let status = raw_stage_status(&pool, &submit).await;
    assert_eq!(
        status, "submission_unknown",
        "恢复后应为 submission_unknown"
    );
    let (state, remote): (String, Option<String>) = {
        let row = sqlx::query(
            "SELECT submit_state, remote_task_id FROM provider_attempts WHERE stage_id = ? ORDER BY started_at DESC LIMIT 1",
        )
        .bind(&submit)
        .fetch_one(&pool)
        .await
        .expect("读取 attempt");
        use sqlx::Row;
        (row.get("submit_state"), row.get("remote_task_id"))
    };
    assert_eq!(state, "unknown", "attempt 应标记 unknown");
    assert_eq!(remote, None, "没有远端 task ID（不能凭空编造）");
    assert_eq!(
        raw_stage_status(&pool, &poll).await,
        "queued",
        "该分支后续购买暂停（下游未解锁）"
    );
    assert_eq!(
        raw_count(&pool, "SELECT COUNT(*) FROM jobs").await,
        1,
        "重启不产生第二个 job"
    );
    assert_eq!(
        raw_count(&pool, "SELECT COUNT(*) FROM provider_attempts").await,
        1,
        "不产生第二个 attempt"
    );
    assert_eq!(
        fixture.call_count("POST", TRIPO_SUBMIT_PATH),
        1,
        "重启后绝不重发付费 POST"
    );
    assert_eq!(
        raw_count(
        &pool,
        "SELECT COUNT(*) FROM audit_events WHERE action = 'provider_attempt_remote_task_id_conflict'",
    )
    .await,
        0,
        "无远端 ID 冲突（不伪造事实）"
    );
    drop(database);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn qa_sigkill_during_poll_resumes_with_known_task_id_only_query() {
    let dir = QaDir::new("kill-poll");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();
    let job_id = seed_job(&pool, "kill-poll").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::TripoUpload,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let submit = insert_stage(
        &pool,
        &job_id,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    // 已接受的远端事实（上一次运行提交成功）。
    {
        let now = Timestamp::now();
        let mut conn = pool.acquire().await.expect("连接");
        let attempt = repo::attempts::create_intent(
            &mut conn,
            repo::attempts::NewAttempt {
                job_id: job_id.clone(),
                stage_id: submit.clone(),
                request_hash: "qa-seed".to_owned(),
            },
            now,
        )
        .await
        .expect("intent");
        repo::attempts::mark_submitting(&mut conn, &attempt.id, now)
            .await
            .expect("submitting");
        repo::attempts::record_remote_task_id(&mut conn, &attempt.id, "qa-task-known", now)
            .await
            .expect("记录远端 ID");
    }
    drop(database);

    // 第一阶段：查询被挂住 → kill -9。
    let hanging = FixtureServer::start(hanging_get_scenario("qa-task-known"));
    let mut child = QaChild::spawn("run", dir.path(), &hanging.base_url());
    wait_until(
        Duration::from_secs(30),
        || hanging.call_count("GET", &format!("{TRIPO_TASKS_PREFIX}qa-task-known")) == 1,
        "子进程发出已知 task ID 的查询",
    )
    .await;
    child.sigkill();
    assert_eq!(
        hanging.call_count("POST", TRIPO_SUBMIT_PATH),
        0,
        "查询阶段不产生付费 POST"
    );
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    // 第二阶段：供应商恢复 → 必须用同一 task ID 继续查询，绝不重新提交。
    let healthy = FixtureServer::start(healthy_get_scenario("qa-task-known"));
    assert!(
        run_child_once("recover", dir.path(), &healthy.base_url()).await,
        "恢复子进程应正常退出"
    );

    let database = Database::open_and_migrate(dir.path())
        .await
        .expect("重开库");
    let pool = database.pool().clone();
    assert!(
        healthy.call_count("GET", &format!("{TRIPO_TASKS_PREFIX}qa-task-known")) >= 1,
        "恢复后按同一远端 ID 查询：{:?}",
        healthy.recorded_summary()
    );
    assert_eq!(
        healthy.call_count("POST", TRIPO_SUBMIT_PATH),
        0,
        "恢复绝不重新提交付费请求"
    );
    assert_eq!(raw_stage_status(&pool, &poll).await, "succeeded");
    assert_eq!(
        raw_count(&pool, "SELECT COUNT(*) FROM provider_attempts").await,
        1,
        "不产生第二个 attempt"
    );
    assert_eq!(raw_count(&pool, "SELECT COUNT(*) FROM jobs").await, 1);
    drop(database);
}

// ---------------------------------------------------------------------------
// 3. 租约 epoch：过期 worker 只能保存事实，不能推进 / 不能解锁
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_expired_worker_can_save_fact_but_cannot_advance_until_takeover() {
    let dir = QaDir::new("epoch");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();
    let job_id = seed_job(&pool, "epoch").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let stage_id = insert_stage(&pool, &job_id, StageKind::TripoSubmit, 0, JobStatus::Queued).await;

    // 旧 worker 领取（真实短租约，等它自然过期）。
    let claimed = job_stages::claim_next(
        &pool,
        &qa_claim_params("qa-old", Duration::from_millis(300)),
    )
    .await
    .expect("领取")
    .expect("应领取到阶段");
    assert_eq!(claimed.id, stage_id);
    assert_eq!(claimed.lease_epoch, 1, "领取即取得新 epoch");
    tokio::time::sleep(Duration::from_millis(400)).await;

    let old_guard = LeaseGuard {
        stage_id: stage_id.clone(),
        owner: "qa-old".to_owned(),
        epoch: 1,
    };
    let mut conn = pool.acquire().await.expect("连接");

    // 过期 worker 推进状态 → 必须被拒绝。
    let advanced = job_stages::advance(
        &mut conn,
        &old_guard,
        Timestamp::now(),
        &StageAdvance::Succeeded {
            result_asset_id: None,
            usage_json: None,
        },
    )
    .await
    .expect("推进调用");
    assert!(!advanced, "过期租约的业务推进必须被拒绝");
    assert_eq!(
        raw_stage_status(&pool, &stage_id).await,
        "running",
        "推进被拒后状态不变"
    );

    // 过期 worker 保存不可变事实（usage 事实）→ 必须允许。
    job_stages::set_result_fact(
        &mut conn,
        &stage_id,
        None,
        Some("{\"qaExpiredFact\":true}"),
        Timestamp::now(),
    )
    .await
    .expect("事实写入");
    let usage: Option<String> =
        sqlx::query_scalar("SELECT usage_json FROM job_stages WHERE id = ?")
            .bind(&stage_id)
            .fetch_one(&pool)
            .await
            .expect("读取 usage");
    assert_eq!(usage.as_deref(), Some("{\"qaExpiredFact\":true}"));
    assert_eq!(
        raw_stage_status(&pool, &stage_id).await,
        "running",
        "事实写入不改变状态"
    );
    drop(conn);

    // 提前接管也应被拒绝？ 不：此刻租约已过期，接管是合法的——但必须由 epoch 校验保护。
    let mut conn = pool.acquire().await.expect("连接");
    let taken = job_stages::take_over_expired(
        &mut conn,
        &stage_id,
        1,
        "qa-new",
        Timestamp::now(),
        Duration::from_secs(30),
    )
    .await
    .expect("接管");
    assert!(taken, "租约过期后新 worker 应能接管");
    let epoch: i64 = sqlx::query_scalar("SELECT lease_epoch FROM job_stages WHERE id = ?")
        .bind(&stage_id)
        .fetch_one(&pool)
        .await
        .expect("读取 epoch");
    assert_eq!(epoch, 2, "接管即 epoch + 1");

    // 旧 worker 的晚到推进仍然被拒（epoch 已变）。
    let late = job_stages::advance(
        &mut conn,
        &old_guard,
        Timestamp::now(),
        &StageAdvance::Succeeded {
            result_asset_id: None,
            usage_json: None,
        },
    )
    .await
    .expect("晚到推进");
    assert!(!late, "被接管后旧 worker 的推进必须被拒绝");
    assert_eq!(raw_stage_status(&pool, &stage_id).await, "running");

    // 新 worker（epoch 2）可以推进。
    let new_guard = LeaseGuard {
        stage_id: stage_id.clone(),
        owner: "qa-new".to_owned(),
        epoch: 2,
    };
    let ok = job_stages::advance(
        &mut conn,
        &new_guard,
        Timestamp::now(),
        &StageAdvance::Succeeded {
            result_asset_id: None,
            usage_json: None,
        },
    )
    .await
    .expect("新 worker 推进");
    assert!(ok, "接管方应能推进");
    assert_eq!(raw_stage_status(&pool, &stage_id).await, "succeeded");
    // 事实保留。
    let usage: Option<String> =
        sqlx::query_scalar("SELECT usage_json FROM job_stages WHERE id = ?")
            .bind(&stage_id)
            .fetch_one(&pool)
            .await
            .expect("读取 usage");
    assert_eq!(usage.as_deref(), Some("{\"qaExpiredFact\":true}"));
    drop(conn);
    drop(database);
}

// ---------------------------------------------------------------------------
// 4. DAG 依赖边、解锁条件与唯一性
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_dag_edges_unlock_only_after_all_dependencies_succeed() {
    let dir = QaDir::new("dag");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();
    let job_id = seed_job(&pool, "dag").await;
    let freeze = insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let b0 = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        0,
        JobStatus::Queued,
    )
    .await;
    let b1 = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        1,
        JobStatus::Queued,
    )
    .await;
    let merge = insert_stage(&pool, &job_id, StageKind::ManualMerge, 0, JobStatus::Queued).await;

    // 依赖边落库（原始 SQL 直查）。
    assert_eq!(deps_of(&pool, &b0).await, vec![freeze.clone()]);
    assert_eq!(deps_of(&pool, &b1).await, vec![freeze.clone()]);
    let mut merge_deps = deps_of(&pool, &merge).await;
    merge_deps.sort();
    let mut expected = vec![b0.clone(), b1.clone()];
    expected.sort();
    assert_eq!(merge_deps, expected, "manual_merge 依赖两个批次");

    // 领取：两个批次可并发领取（上限 2），merge 尚不可领取。
    let mut claimed = Vec::new();
    while let Some(stage) =
        job_stages::claim_next(&pool, &qa_claim_params("qa-dag", Duration::from_secs(30)))
            .await
            .expect("领取")
    {
        claimed.push(stage.id.clone());
        assert_ne!(stage.id, merge, "依赖未全部 succeeded 时 merge 不得被领取");
    }
    claimed.sort();
    let mut expect_claims = vec![b0.clone(), b1.clone()];
    expect_claims.sort();
    assert_eq!(claimed, expect_claims);

    // 只完成 b0：merge 仍不可领取（b1 未 succeeded）。
    let mut conn = pool.acquire().await.expect("连接");
    for stage_id in [&b0] {
        let ok = job_stages::advance(
            &mut conn,
            &LeaseGuard {
                stage_id: stage_id.clone(),
                owner: "qa-dag".to_owned(),
                epoch: 1,
            },
            Timestamp::now(),
            &StageAdvance::Succeeded {
                result_asset_id: None,
                usage_json: None,
            },
        )
        .await
        .expect("推进");
        assert!(ok, "batch0 推进失败");
    }
    assert!(
        job_stages::claim_next(&pool, &qa_claim_params("qa-dag", Duration::from_secs(30)))
            .await
            .expect("领取")
            .is_none(),
        "b1 仍在运行时 merge 不得被领取"
    );

    // 完成 b1：merge 解锁。
    let ok = job_stages::advance(
        &mut conn,
        &LeaseGuard {
            stage_id: b1.clone(),
            owner: "qa-dag".to_owned(),
            epoch: 1,
        },
        Timestamp::now(),
        &StageAdvance::Succeeded {
            result_asset_id: None,
            usage_json: None,
        },
    )
    .await
    .expect("推进");
    assert!(ok);
    let unlocked =
        job_stages::claim_next(&pool, &qa_claim_params("qa-dag", Duration::from_secs(30)))
            .await
            .expect("领取")
            .expect("两个批次成功后 merge 应被解锁");
    assert_eq!(unlocked.id, merge);
    drop(conn);

    // 唯一性：同一 (job, stage_kind, batch_index) 不得重复。
    let now = Timestamp::now().as_millis();
    let duplicate = sqlx::query(
        "INSERT INTO job_stages (id, job_id, stage_kind, batch_index, page_set, input_hash, status, lease_epoch, attempt_count, created_at, updated_at) \
         VALUES (?, ?, 'manual_extract', 0, '[1]', 'dup', 'queued', 0, 0, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(&job_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(duplicate.is_err(), "重复的 (job, kind, batch) 必须被拒绝");

    // 非批处理阶段 batch_index 必须为 0。
    let bad_batch = sqlx::query(
        "INSERT INTO job_stages (id, job_id, stage_kind, batch_index, input_hash, status, lease_epoch, attempt_count, created_at, updated_at) \
         VALUES (?, ?, 'manual_merge', 1, 'x', 'queued', 0, 0, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(&job_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await;
    assert!(bad_batch.is_err(), "非批处理阶段的 batch_index 必须为 0");
    drop(database);
}

// ---------------------------------------------------------------------------
// 5. 并发上限：远端生成 2 / 说明书批次 2
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_concurrency_caps_gate_admission_at_two_and_two() {
    let dir = QaDir::new("cap");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();

    // 三个 job 各有一个可领取的远端阶段（全局上限 2，第三个必须等待）。
    for tag in ["cap-a", "cap-b", "cap-c"] {
        let job_id = seed_job(&pool, tag).await;
        insert_stage(
            &pool,
            &job_id,
            StageKind::FreezeInputs,
            0,
            JobStatus::Succeeded,
        )
        .await;
        insert_stage(&pool, &job_id, StageKind::TripoUpload, 0, JobStatus::Queued).await;
    }
    // 第四个 job：4 个说明书批次（上限 2）。
    let batch_job = seed_job(&pool, "cap-d").await;
    insert_stage(
        &pool,
        &batch_job,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let mut batches = Vec::new();
    for index in 0..4 {
        batches.push(
            insert_stage(
                &pool,
                &batch_job,
                StageKind::ManualExtract,
                index,
                JobStatus::Queued,
            )
            .await,
        );
    }

    let mut claims = Vec::new();
    while let Some(stage) =
        job_stages::claim_next(&pool, &qa_claim_params("qa-cap", Duration::from_secs(30)))
            .await
            .expect("领取")
    {
        claims.push(stage);
    }
    let remote_running = raw_count(
        &pool,
        "SELECT COUNT(*) FROM job_stages WHERE status = 'running' AND stage_kind IN ('tripo_upload','tripo_submit','tripo_poll','model_download','model_validate')",
    )
    .await;
    let manual_running = raw_count(
        &pool,
        "SELECT COUNT(*) FROM job_stages WHERE status = 'running' AND stage_kind = 'manual_extract'",
    )
    .await;
    assert_eq!(remote_running, 2, "远端生成在飞上限 2");
    assert_eq!(manual_running, 2, "说明书批次在飞上限 2");
    assert_eq!(claims.len(), 4, "首轮只能领取 2 + 2 个阶段");

    // 一个批次完成后，才能再领取下一个批次（同一轮内不突破上限）。
    let mut conn = pool.acquire().await.expect("连接");
    let ok = job_stages::advance(
        &mut conn,
        &LeaseGuard {
            stage_id: batches[0].clone(),
            owner: "qa-cap".to_owned(),
            epoch: 1,
        },
        Timestamp::now(),
        &StageAdvance::Succeeded {
            result_asset_id: None,
            usage_json: None,
        },
    )
    .await
    .expect("推进");
    assert!(ok);
    drop(conn);
    let next = job_stages::claim_next(&pool, &qa_claim_params("qa-cap", Duration::from_secs(30)))
        .await
        .expect("领取")
        .expect("释放一个批次名额后应有下一个批次可领取");
    assert_eq!(next.id, batches[2], "补位的是下一个说明书批次");
    let manual_running = raw_count(
        &pool,
        "SELECT COUNT(*) FROM job_stages WHERE status = 'running' AND stage_kind = 'manual_extract'",
    )
    .await;
    assert_eq!(manual_running, 2, "补位后仍在飞上限 2");
    drop(database);
}

// ---------------------------------------------------------------------------
// 6. 同一阶段只允许一个未对账 attempt
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_single_unresolved_attempt_per_stage_is_enforced() {
    let dir = QaDir::new("attempt");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();
    let job_id = seed_job(&pool, "attempt").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let stage_id = insert_stage(&pool, &job_id, StageKind::TripoSubmit, 0, JobStatus::Queued).await;
    let now = Timestamp::now();
    let mut conn = pool.acquire().await.expect("连接");
    repo::attempts::create_intent(
        &mut conn,
        repo::attempts::NewAttempt {
            job_id: job_id.clone(),
            stage_id: stage_id.clone(),
            request_hash: "qa-1".to_owned(),
        },
        now,
    )
    .await
    .expect("第一个 intent 应成功");
    let second = repo::attempts::create_intent(
        &mut conn,
        repo::attempts::NewAttempt {
            job_id,
            stage_id: stage_id.clone(),
            request_hash: "qa-2".to_owned(),
        },
        now,
    )
    .await;
    assert!(
        second.is_err(),
        "同一阶段第二个未对账 attempt 必须被拒绝（部分唯一索引）"
    );
    drop(conn);
    drop(database);
}

// ---------------------------------------------------------------------------
// 7. AC-036 复核（总等待预算的锚点）：复现 BUG-003，修复后应移除 #[ignore]
// ---------------------------------------------------------------------------

fn running_get_scenario(task_id: &str) -> Scenario {
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

async fn seed_aged_accepted_attempt(
    pool: &SqlitePool,
    job_id: &str,
    stage_id: &str,
    task_id: &str,
    started_at: Timestamp,
) {
    let mut conn = pool.acquire().await.expect("连接");
    let attempt = repo::attempts::create_intent(
        &mut conn,
        repo::attempts::NewAttempt {
            job_id: job_id.to_owned(),
            stage_id: stage_id.to_owned(),
            request_hash: "qa-aged".to_owned(),
        },
        started_at,
    )
    .await
    .expect("老化 intent");
    repo::attempts::mark_submitting(&mut conn, &attempt.id, started_at)
        .await
        .expect("老化 submitting");
    repo::attempts::record_remote_task_id(&mut conn, &attempt.id, task_id, started_at)
        .await
        .expect("老化远端 ID");
}

/// AC-036「总等待超 30 分钟 → needs_input（保留 task_id）」在**真实轮询链路形态**下的可达性。
///
/// 观察 1（对照，RD 用例的形态）：轮询阶段**自带** 31 分钟前的 attempt → 预算触发（needs_input）。
/// 观察 2（真实形态）：只有 `tripo_submit` 有 31 分钟前的 accepted attempt，轮询阶段自身没有
/// attempt（本卡 fixture `FixtureTripoPoll` 与我的 `QaPollKnownTask` 都从 job 的 submit attempt
/// 取 task ID，不建自己的 attempt）→ 预算不触发，阶段永久保持 `waiting_provider`。
///
/// 执行器 `plan_advance` 用 `attempt.started_at`（= 该阶段自己的最近 attempt）作为等待起点，
/// 因此真实链路上 `waited_seconds` 恒为 0。修复方向（交 RD）：锚点回退到该 job 最近的
/// accepted submit attempt（同一份事实已被轮询处理器用来取 task ID），或显式持久化等待起点事实。
///
/// `#[ignore]` 曾用于保留 BUG-003 的复现证据（断言即**合同期望**）。
/// **QA 回合 29（T21）转正**：BUG-003 已在回合 11 由 RD 修复并 CLOSED，本用例在
/// 当前实现下实际通过（回合 29 实测 `--ignored` 运行 ok），断言逐字未改，只去掉 ignore；
/// 保留 `ok` 即为"总等待预算在真实轮询链路形态下可达"的常绿回归。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_total_wait_budget_must_trigger_on_real_poll_shape() {
    let dir = QaDir::new("wait-budget");
    let database = open_db(dir.path()).await;
    let pool = database.pool().clone();
    let fixture = FixtureServer::start(running_get_scenario("qa-task-wait"));

    // 观察 1（对照）：轮询阶段自带 31 分钟前的已接受 attempt。
    let job_a = seed_job(&pool, "wait-a").await;
    insert_stage(
        &pool,
        &job_a,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    insert_stage(
        &pool,
        &job_a,
        StageKind::TripoUpload,
        0,
        JobStatus::Succeeded,
    )
    .await;
    insert_stage(
        &pool,
        &job_a,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let poll_a = insert_stage(&pool, &job_a, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    let aged = Timestamp::now()
        .checked_add_millis(-((manual_core::jobs::REMOTE_WAIT_BUDGET_SECONDS as i64 + 60) * 1000))
        .expect("时间");
    seed_aged_accepted_attempt(&pool, &job_a, &poll_a, "qa-task-wait", aged).await;

    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoPoll,
        QaPollKnownTask {
            base: fixture.base_url(),
            gets: Arc::new(AtomicUsize::new(0)),
        },
    );
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = fixed_jitter_executor(
        pool.clone(),
        ExecutorConfig {
            lease: Duration::from_secs(30),
            renew: Duration::from_secs(5),
            ..ExecutorConfig::default()
        },
        registry,
        clock.clone(),
    );
    let _ = executor.tick().await.expect("对照 tick");
    let status_a = raw_stage_status(&pool, &poll_a).await;
    eprintln!("[qa probe] 观察 1（轮询阶段自带 attempt）status={status_a}");
    assert_eq!(
        status_a, "needs_input",
        "对照：轮询阶段自带 attempt 时预算可触发（RD 用例的形态）"
    );

    // 观察 2（真实形态）：只有 submit 级 accepted attempt（31 分钟前），轮询阶段无自身 attempt。
    let job_b = seed_job(&pool, "wait-b").await;
    insert_stage(
        &pool,
        &job_b,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    insert_stage(
        &pool,
        &job_b,
        StageKind::TripoUpload,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let submit_b = insert_stage(
        &pool,
        &job_b,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let poll_b = insert_stage(&pool, &job_b, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    seed_aged_accepted_attempt(&pool, &job_b, &submit_b, "qa-task-wait", aged).await;

    // 连续轮询并把时钟推过 30 分钟预算（每次按轮询节奏到点重新领取）。
    let mut status_b = String::new();
    for _ in 0..4 {
        clock.advance_millis(16_000);
        let _ = executor.tick().await.expect("真实形态 tick");
        status_b = raw_stage_status(&pool, &poll_b).await;
        eprintln!("[qa probe] 观察 2（submit 级 attempt 才老化）status={status_b}");
    }
    assert_eq!(
        status_b, "needs_input",
        "AC-036：总等待超 30 分钟应转 needs_input 并保留 task_id（真实轮询链路形态）"
    );
    let attempt = repo::attempts::latest_for_stage(&mut pool.acquire().await.unwrap(), &submit_b)
        .await
        .expect("attempt")
        .expect("accepted attempt 仍在");
    assert_eq!(attempt.remote_task_id.as_deref(), Some("qa-task-wait"));
    drop(database);
}
