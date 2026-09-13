//! T10 集成测试：**持久任务执行器**（PRD 修订 2，REQ-024 主 / REQ-025、REQ-026 执行器侧）。
//!
//! 覆盖的验收条件（QA 按此复核）：
//! - AC-035：阶段 DAG 按合同落库（`job+stage_kind+batch_index` 唯一、依赖边、批次）；
//!   双 worker 竞争只有一个领取（epoch 递增）；租约过期被接管，**旧 worker 晚到 receipt
//!   可保存事实但不能推进状态/解锁后续**；SIGKILL 后按已知远端 ID 继续查询、不产生第二次
//!   付费提交；并发上限（生成 2、AI 批次 2）生效；
//! - AC-036：429/5xx → `retry_wait` 且退避 2/4/8/16/32 秒 + jitter、尊重 `Retry-After`、
//!   超 5 次 → `failed`；资料/schema 不足 → `needs_input` 列出可行动缺项；
//!   总等待超 30 分钟 → `needs_input` 且保留 task_id（恢复只查询、不重新购买）；
//! - AC-037/AC-038（执行器侧）：付费 POST 已发出但响应未到时 attempt 标记
//!   `submission_unknown`、该分支后续购买暂停、重启不重发；同步批次响应未持久化同样进入
//!   `submission_unknown`；结果已持久化而 checkpoint 未推进时恢复补推进而不重新付费。
//!
//! 隔离与门控：
//! - 所有 HTTP 都发往 T05 的本机 fixture（`127.0.0.1`，无真实外网调用）；
//! - 崩溃断点只在启用 `job-failpoints` feature 的**测试构建**存在（由
//!   `[dev-dependencies]` 自引用开启），生产二进制没有该分支；
//! - SIGKILL 用例通过重新执行本测试二进制启动真实子进程（`child_worker_entry`），
//!   父进程用 `kill -9` 制造硬崩溃，再以第二个进程验证恢复。
//!
//! 本卡用 fixture 阶段验证机制：**未接通 Tripo/说明书 AI**（真实适配器属 T12/T14）。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::TestDir;
use everything_manual::config::datadir;
use everything_manual::jobs::executor::fixed_jitter_executor;
use everything_manual::jobs::failpoints::{
    self, FailpointAction, MANUAL_AFTER_REQUEST_BEFORE_RESPONSE,
    PAID_AFTER_INTENT_BEFORE_SUBMITTING, PAID_AFTER_RECEIPT_BEFORE_ADVANCE,
    PAID_AFTER_RESPONSE_BEFORE_RECEIPT, PAID_AFTER_SUBMITTING_BEFORE_REQUEST,
    RESULT_FACT_BEFORE_CHECKPOINT,
};
use everything_manual::jobs::{
    Clock, ExecutorConfig, JobExecutor, ManualClock, StageContext, StageFuture, StageHandler,
    StageOutcome, StageRegistry, TickOutcome,
};
use everything_manual::storage::repo::job_stages::{self, NewStage};
use everything_manual::storage::repo::{self, jobs as jobs_repo};
use everything_manual::storage::{Database, StorageError};
use manual_core::domain::{
    AssetPurpose, Job, JobStage, JobStatus, ProviderAttempt, StageKind, SubmitState,
};
use manual_core::timestamps::Timestamp;
use manual_core::{ids, jobs as core_jobs};
use serde_json::json;
use sqlx::SqlitePool;
use test_support::client::LocalHttpClient;
use test_support::presets::{MANUAL_AI_RESPONSES_PATH, TRIPO_SUBMIT_PATH, TRIPO_TASKS_PREFIX};
use test_support::scenario::{RouteScript, Scenario, Step};
use test_support::{BodySpec, FixtureServer, ResponseSpec};

// ---------------------------------------------------------------------------
// 基础工具（临时目录 / 数据库 / 种子数据）
// ---------------------------------------------------------------------------

async fn open_database(dir: &TestDir) -> Database {
    datadir::ensure_initialized(dir.path()).expect("初始化 data-dir 结构");
    Database::open_and_migrate(dir.path())
        .await
        .expect("打开并迁移数据库")
}

/// 建单所需的完整引用链（item → blob → asset → document → preparation → snapshot）。
///
/// T11 之前没有建单入口，执行器测试用**最小种子数据**直接写入这些行；
/// 迁移导出的库结构与生产一致（外键全部满足）。
async fn seed_job(pool: &SqlitePool, tag: &str) -> (String, String) {
    let now = Timestamp::now().as_millis();
    let item_id = ids::new_id();
    let sha = test_support::assets::sha256_hex(format!("{tag}-source").as_bytes());
    let asset_id = ids::new_id();
    let document_id = ids::new_id();
    let preparation_id = ids::new_id();
    let snapshot_id = ids::new_id();

    let mut conn = pool.acquire().await.expect("连接");
    sqlx::query(
        "INSERT INTO items (id, name, brand, model, variant, revision, archived_at, created_at, updated_at) \
         VALUES (?, ?, NULL, ?, NULL, 1, NULL, ?, ?)",
    )
    .bind(&item_id)
    .bind(format!("执行器测试物品 {tag}"))
    .bind("X100V")
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

/// 插入阶段（按 core 的 DAG 自动写依赖边）。
async fn insert_stage(
    pool: &SqlitePool,
    job_id: &str,
    kind: StageKind,
    batch_index: i64,
    status: JobStatus,
) -> JobStage {
    let mut conn = pool.acquire().await.expect("连接");
    job_stages::insert(
        &mut conn,
        NewStage {
            job_id: job_id.to_owned(),
            stage_kind: kind,
            batch_index,
            page_set_json: kind.is_batched().then(|| format!("[{}]", batch_index + 1)),
            input_hash: format!("hash-{}-{batch_index}", kind.as_str()),
            status,
        },
        Timestamp::now(),
    )
    .await
    .expect("插入阶段")
}

/// 用默认参数领取一个阶段（测试便捷函数）。
async fn claim_next(pool: &SqlitePool, owner: &str) -> Option<JobStage> {
    job_stages::claim_next(
        pool,
        &job_stages::ClaimParams {
            owner: owner.to_owned(),
            now: Timestamp::now(),
            lease: Duration::from_secs(30),
            remote_generation_limit: 2,
            manual_ai_batch_limit: 2,
        },
    )
    .await
    .expect("领取阶段")
}

async fn read_stage(pool: &SqlitePool, stage_id: &str) -> JobStage {
    let mut conn = pool.acquire().await.expect("连接");
    job_stages::get(&mut conn, stage_id)
        .await
        .expect("读取阶段")
        .expect("阶段存在")
}

async fn stage_status(pool: &SqlitePool, stage_id: &str) -> JobStatus {
    read_stage(pool, stage_id).await.status
}

async fn read_job(pool: &SqlitePool, job_id: &str) -> Job {
    let mut conn = pool.acquire().await.expect("连接");
    jobs_repo::get(&mut conn, job_id)
        .await
        .expect("读取 job")
        .expect("job 存在")
}

async fn latest_attempt(pool: &SqlitePool, stage_id: &str) -> Option<ProviderAttempt> {
    let mut conn = pool.acquire().await.expect("连接");
    repo::attempts::latest_for_stage(&mut conn, stage_id)
        .await
        .expect("读取 attempt")
}

/// 把租约改到过去（模拟"worker 死亡且租约到期"；等价于真实时间流逝）。
async fn expire_lease(pool: &SqlitePool, stage_id: &str) {
    let past = Timestamp::now().as_millis() - 1_000;
    sqlx::query("UPDATE job_stages SET lease_until = ? WHERE id = ?")
        .bind(past)
        .bind(stage_id)
        .execute(pool)
        .await
        .expect("过期租约");
}

async fn force_status(pool: &SqlitePool, stage_id: &str, status: JobStatus) {
    sqlx::query("UPDATE job_stages SET status = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(stage_id)
        .execute(pool)
        .await
        .expect("改写阶段状态");
}

/// 直接写入一个已接受的 attempt（模拟"上一次运行已提交，重启后继续查询"）。
async fn seed_accepted_attempt(
    pool: &SqlitePool,
    job_id: &str,
    stage_id: &str,
    remote_task_id: &str,
    started_at: Timestamp,
) -> String {
    let mut conn = pool.acquire().await.expect("连接");
    let attempt = repo::attempts::create_intent(
        &mut conn,
        repo::attempts::NewAttempt {
            job_id: job_id.to_owned(),
            stage_id: stage_id.to_owned(),
            request_hash: "seed-hash".to_owned(),
        },
        started_at,
    )
    .await
    .expect("创建 intent");
    repo::attempts::mark_submitting(&mut conn, &attempt.id, started_at)
        .await
        .expect("标记 submitting");
    repo::attempts::record_remote_task_id(&mut conn, &attempt.id, remote_task_id, started_at)
        .await
        .expect("记录远端 ID");
    drop(conn);
    attempt.id
}

async fn seed_result_asset(pool: &SqlitePool, item_id: &str, tag: &str) -> String {
    let now = Timestamp::now().as_millis();
    let sha = test_support::assets::sha256_hex(format!("{tag}-result").as_bytes());
    let mut conn = pool.acquire().await.expect("连接");
    sqlx::query(
        "INSERT INTO blobs (sha256, size, mime, storage_state, created_at) VALUES (?, 512, 'application/json', 'stored', ?)",
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
            purpose: AssetPurpose::PageText,
            original_name: None,
        },
    )
    .await
    .expect("插入结果资产");
    asset.id
}

// ---------------------------------------------------------------------------
// fixture 阶段（HTTP 走 T05 的本机 fixture）
// ---------------------------------------------------------------------------

/// 测试暂停闸门：handler 进入后通知测试，等待测试放行。
struct Gate {
    entered_tx: tokio::sync::mpsc::UnboundedSender<()>,
    entered_rx: tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<()>>,
    release: tokio::sync::Notify,
}

impl Gate {
    fn new() -> Arc<Self> {
        let (entered_tx, entered_rx) = tokio::sync::mpsc::unbounded_channel();
        Arc::new(Self {
            entered_tx,
            entered_rx: tokio::sync::Mutex::new(entered_rx),
            release: tokio::sync::Notify::new(),
        })
    }

    async fn pass(&self) {
        let _ = self.entered_tx.send(());
        self.release.notified().await;
    }

    async fn wait_entered(&self) {
        let mut rx = self.entered_rx.lock().await;
        rx.recv().await.expect("闸门通知");
    }

    fn release(&self) {
        self.release.notify_one();
    }

    /// 放行当前所有等待者（`notify_waiters`：只唤醒当下已等待的任务，排空用例配合轮询使用）。
    fn release_all(&self) {
        self.release.notify_waiters();
    }
}

/// 在阻塞线程池里发 HTTP（fixture 客户端是 std 阻塞实现）。
async fn http_post_json(
    base_url: &str,
    path: &str,
    body: serde_json::Value,
    read_timeout: Duration,
) -> Result<test_support::HttpResponse, String> {
    let url = format!("{base_url}{path}");
    tokio::task::spawn_blocking(move || {
        LocalHttpClient::with_read_timeout(read_timeout).post_json(&url, &body)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

async fn http_get(
    base_url: &str,
    path: &str,
    read_timeout: Duration,
) -> Result<test_support::HttpResponse, String> {
    let url = format!("{base_url}{path}");
    tokio::task::spawn_blocking(move || LocalHttpClient::with_read_timeout(read_timeout).get(&url))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

/// fixture 阶段：Tripo 付费提交（异步远端任务）。
///
/// 行为与真实适配器同构（T12 落地后由真实适配器替换）：
/// 429 → attempt failed + `Retryable`（可证明未被接受）；
/// 5xx/网络中断/超时 → attempt unknown + `SubmissionUnknown`（不能证明未被接受）；
/// 200 且有 task_id → 立即持久化事实观察 → `Succeeded`。
struct FixtureTripoSubmit {
    base_url: String,
    read_timeout: Duration,
    calls: Arc<AtomicUsize>,
    gate: Option<Arc<Gate>>,
}

impl FixtureTripoSubmit {
    fn new(base_url: &str, read_timeout: Duration, calls: Arc<AtomicUsize>) -> Self {
        Self {
            base_url: base_url.to_owned(),
            read_timeout,
            calls,
            gate: None,
        }
    }

    fn with_gate(mut self, gate: Arc<Gate>) -> Self {
        self.gate = Some(gate);
        self
    }
}

impl StageHandler for FixtureTripoSubmit {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let window = &mut ctx.submission;
            window.begin_intent("tripo-submit-request-v1").await?;
            window.mark_submitting().await?;
            if let Some(gate) = &self.gate {
                gate.pass().await;
            }
            let response = http_post_json(
                &self.base_url,
                TRIPO_SUBMIT_PATH,
                json!({ "inputs": [{ "front": "token-front" }, { "left": "token-left" }] }),
                self.read_timeout,
            )
            .await;
            match response {
                Err(detail) => {
                    window
                        .mark_unknown(&format!("付费 POST 网络错误：{detail}"))
                        .await?;
                    Ok(StageOutcome::SubmissionUnknown {
                        reason: format!("付费 POST 网络错误（不能证明未被接受）：{detail}"),
                    })
                }
                Ok(response) if response.status == 429 => {
                    let retry_after = response
                        .header("retry-after")
                        .and_then(|value| value.parse::<u64>().ok());
                    window.mark_failed("429 限速：可证明未被接受").await?;
                    Ok(StageOutcome::Retryable {
                        reason: "429 Too Many Requests".to_owned(),
                        retry_after_seconds: retry_after,
                    })
                }
                Ok(response) if response.status >= 500 => {
                    window.mark_unknown("5xx：含糊失败不能证明未被接受").await?;
                    Ok(StageOutcome::SubmissionUnknown {
                        reason: format!(
                            "付费 POST 返回 {}：含糊失败不能证明未被接受",
                            response.status
                        ),
                    })
                }
                Ok(response) => {
                    let body = response.json().unwrap_or(json!({}));
                    let code = body["code"].as_i64().unwrap_or(-1);
                    let task_id = body["data"]["task_id"].as_str().map(str::to_owned);
                    match (code, task_id) {
                        (0, Some(task_id)) => {
                            window.record_remote_task_id(&task_id).await?;
                            Ok(StageOutcome::Succeeded {
                                result_asset_id: None,
                                usage: Some(json!({ "remoteTaskId": task_id })),
                            })
                        }
                        _ => {
                            // 200 但拿不到 task ID：无法证明供应商没有创建任务 → 未知。
                            let detail = format!("响应缺少 task_id：{}", response.text());
                            window.mark_unknown(&detail).await?;
                            Ok(StageOutcome::SubmissionUnknown { reason: detail })
                        }
                    }
                }
            }
        })
    }
}

/// fixture 阶段：说明书 AI 同步批次（整批响应落库才算完成）。
struct FixtureManualBatch {
    base_url: String,
    read_timeout: Duration,
    result_asset_id: String,
    calls: Arc<AtomicUsize>,
}

impl StageHandler for FixtureManualBatch {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let window = &mut ctx.submission;
            window.begin_intent("manual-batch-request-v1").await?;
            window.mark_submitting().await?;
            let response = http_post_json(
                &self.base_url,
                MANUAL_AI_RESPONSES_PATH,
                json!({ "model": "fixture-model", "input": "pages 1-4" }),
                self.read_timeout,
            )
            .await;
            match response {
                Err(detail) => {
                    window
                        .mark_unknown(&format!("同步批次网络错误：{detail}"))
                        .await?;
                    Ok(StageOutcome::SubmissionUnknown {
                        reason: format!("同步批次请求结果未知：{detail}"),
                    })
                }
                Ok(response) if response.status == 429 => {
                    let retry_after = response
                        .header("retry-after")
                        .and_then(|value| value.parse::<u64>().ok());
                    window.mark_failed("429 限速：可证明未被接受").await?;
                    Ok(StageOutcome::Retryable {
                        reason: "429 Too Many Requests".to_owned(),
                        retry_after_seconds: retry_after,
                    })
                }
                Ok(response) if response.status >= 500 => {
                    // 同步链路：含糊 5xx 同样不能证明请求未被接受 → unknown（不重购）。
                    window
                        .mark_unknown("同步批次 5xx：不能证明未被接受")
                        .await?;
                    Ok(StageOutcome::SubmissionUnknown {
                        reason: format!("同步批次返回 {}：不能证明未被接受", response.status),
                    })
                }
                Ok(response) => {
                    let body = response.json().unwrap_or(json!({}));
                    let response_id = body["id"].as_str().map(str::to_owned);
                    // 完整响应已持久化（结果资产 + receipt + usage 在同一短事务）。
                    window
                        .record_sync_response(
                            response_id.as_deref(),
                            Some(r#"{"outputTokens":128}"#),
                            Some(&self.result_asset_id),
                        )
                        .await?;
                    Ok(StageOutcome::Succeeded {
                        result_asset_id: Some(self.result_asset_id.clone()),
                        usage: Some(json!({ "outputTokens": 128 })),
                    })
                }
            }
        })
    }
}

/// fixture 阶段：远端任务查询（把远端原值归一化为等待/成功/临时失败）。
struct FixtureTripoPoll {
    base_url: String,
    read_timeout: Duration,
    calls: Arc<AtomicUsize>,
}

impl StageHandler for FixtureTripoPoll {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            // 本次执行若已带"继续查询"提示则直接用；否则读同一 job 上游
            // `tripo_submit` 落库的已接受事实（重启后仍能找到，绝不重新提交）。
            let task_id = match ctx.known_remote_task_id().map(str::to_owned) {
                Some(task_id) => Some(task_id),
                None => {
                    let mut conn = ctx.pool.acquire().await?;
                    repo::attempts::latest_accepted_for_job(
                        &mut conn,
                        &ctx.job.id,
                        StageKind::TripoSubmit,
                    )
                    .await?
                    .and_then(|attempt| attempt.remote_task_id)
                }
            };
            let Some(task_id) = task_id else {
                // 没有任何已提交事实（本地阶段用途）：直接成功。
                return Ok(StageOutcome::succeeded());
            };
            let path = format!("{TRIPO_TASKS_PREFIX}{task_id}");
            match http_get(&self.base_url, &path, self.read_timeout).await {
                Err(detail) => Ok(StageOutcome::Retryable {
                    reason: format!("查询远端任务失败：{detail}"),
                    retry_after_seconds: None,
                }),
                Ok(response) if response.status == 429 => Ok(StageOutcome::Retryable {
                    reason: "429 Too Many Requests".to_owned(),
                    retry_after_seconds: response
                        .header("retry-after")
                        .and_then(|value| value.parse::<u64>().ok()),
                }),
                Ok(response) if response.status >= 500 => Ok(StageOutcome::Retryable {
                    reason: format!("查询远端任务返回 {}", response.status),
                    retry_after_seconds: None,
                }),
                Ok(response) => {
                    let body = response.json().unwrap_or(json!({}));
                    match body["data"]["status"].as_str().unwrap_or("unknown") {
                        "success" => Ok(StageOutcome::succeeded()),
                        "running" | "queued" => Ok(StageOutcome::WaitingProvider),
                        other => Ok(StageOutcome::NeedsInput {
                            items: vec![everything_manual::jobs::MissingItem::new(
                                "remote_status_unsupported",
                                format!("远端返回无法处理的状态：{other}"),
                            )],
                        }),
                    }
                }
            }
        })
    }
}

/// 通用脚本化处理器：按预设序列返回结果，序列用尽后返回成功。
struct ScriptedHandler {
    outcomes: tokio::sync::Mutex<std::collections::VecDeque<StageOutcome>>,
    calls: Arc<AtomicUsize>,
}

impl ScriptedHandler {
    fn new(outcomes: Vec<StageOutcome>) -> Self {
        Self {
            outcomes: tokio::sync::Mutex::new(outcomes.into()),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl StageHandler for ScriptedHandler {
    fn run<'a>(&'a self, _ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut outcomes = self.outcomes.lock().await;
            Ok(outcomes.pop_front().unwrap_or_else(StageOutcome::succeeded))
        })
    }
}

/// 阻塞处理器：记录并发在飞数量（并发上限用例）。
struct BlockingHandler {
    inflight: Arc<AtomicUsize>,
    max_inflight: Arc<AtomicUsize>,
    gate: Arc<Gate>,
}

impl BlockingHandler {
    fn new(inflight: Arc<AtomicUsize>, max_inflight: Arc<AtomicUsize>, gate: Arc<Gate>) -> Self {
        Self {
            inflight,
            max_inflight,
            gate,
        }
    }
}

impl StageHandler for BlockingHandler {
    fn run<'a>(&'a self, _ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            let current = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_inflight.fetch_max(current, Ordering::SeqCst);
            self.gate.pass().await;
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            Ok(StageOutcome::succeeded())
        })
    }
}

/// 记录续约期间租约状态的处理器（续约用例）。
///
/// 每 `3 × 续约间隔` 采样一次 `lease_until - now`：若续约没有发生，测试时钟推进后
/// 该差值会不断缩小；续约正常时它保持接近租约时长。
struct LeaseWatcherHandler {
    pool: SqlitePool,
    clock: Arc<ManualClock>,
    samples: Arc<tokio::sync::Mutex<Vec<(i64, i64)>>>,
}

impl StageHandler for LeaseWatcherHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            let step_millis = 120_i64; // 3 × 40ms 续约间隔
            let mut samples = Vec::new();
            for _ in 0..4 {
                tokio::time::sleep(Duration::from_millis(step_millis as u64)).await;
                let stage = read_stage(&self.pool, &ctx.stage.id).await;
                let lease_until = stage.lease_until.expect("租约存在").as_millis();
                samples.push((
                    lease_until - self.clock.now().as_millis(),
                    stage.lease_epoch,
                ));
                self.clock.advance_millis(step_millis);
            }
            *self.samples.lock().await = samples;
            Ok(StageOutcome::succeeded())
        })
    }
}

/// 固定 jitter、手动时钟的执行器（测试确定性）。
fn test_executor(
    pool: &SqlitePool,
    registry: StageRegistry,
    clock: Arc<ManualClock>,
    config: ExecutorConfig,
) -> Arc<JobExecutor> {
    fixed_jitter_executor(pool.clone(), config, registry, clock)
}

fn test_config() -> ExecutorConfig {
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

/// 场景构造：提交路由成功返回 task_id，任务查询路由返回 success（均 `repeatLast`）。
fn submit_success_scenario(task_id: &str) -> Scenario {
    Scenario::new(vec![
        RouteScript {
            method: "POST".to_owned(),
            path: TRIPO_SUBMIT_PATH.to_owned(),
            path_match: Default::default(),
            repeat_last: true,
            steps: vec![Step::Respond {
                response: ResponseSpec {
                    status: 200,
                    headers: Default::default(),
                    body: BodySpec::Json {
                        json: json!({ "code": 0, "data": { "task_id": task_id } }),
                    },
                },
            }],
        },
        RouteScript {
            method: "GET".to_owned(),
            path: format!("{TRIPO_TASKS_PREFIX}{task_id}"),
            path_match: Default::default(),
            repeat_last: true,
            steps: vec![Step::Respond {
                response: ResponseSpec {
                    status: 200,
                    headers: Default::default(),
                    body: BodySpec::Json {
                        json: json!({ "code": 0, "data": { "status": "success", "progress": 100 } }),
                    },
                },
            }],
        },
    ])
}

// ---------------------------------------------------------------------------
// AC-035：阶段 DAG、领取与租约
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dag_persists_batches_and_only_unlocks_satisfied_dependencies() {
    let dir = TestDir::new("t10-dag");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "dag").await;

    let freeze = insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let batch0 = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        0,
        JobStatus::Queued,
    )
    .await;
    let batch1 = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        1,
        JobStatus::Queued,
    )
    .await;
    let batch2 = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        2,
        JobStatus::Queued,
    )
    .await;
    let merge = insert_stage(&pool, &job_id, StageKind::ManualMerge, 0, JobStatus::Queued).await;
    let upload = insert_stage(&pool, &job_id, StageKind::TripoUpload, 0, JobStatus::Queued).await;
    let submit = insert_stage(&pool, &job_id, StageKind::TripoSubmit, 0, JobStatus::Queued).await;
    let assemble = insert_stage(
        &pool,
        &job_id,
        StageKind::AssembleDraft,
        0,
        JobStatus::Queued,
    )
    .await;

    // 逻辑批展开为持久执行单元：job + stage_kind + batch_index 唯一。
    assert!(matches!(
        job_stages::insert(
            &mut pool.acquire().await.unwrap(),
            NewStage {
                job_id: job_id.clone(),
                stage_kind: StageKind::ManualExtract,
                batch_index: 0,
                page_set_json: None,
                input_hash: "dup".to_owned(),
                status: JobStatus::Queued,
            },
            Timestamp::now(),
        )
        .await,
        Err(StorageError::UniqueViolation { .. })
    ));
    assert_eq!(batch0.batch_index, 0);
    assert_eq!(batch0.page_set.as_deref(), Some(&[1_i64][..]));

    // 依赖边落库（DAG 不靠内存循环）。
    let mut conn = pool.acquire().await.expect("连接");
    let batch0_deps = job_stages::dependencies_of(&mut conn, &batch0.id)
        .await
        .unwrap();
    assert_eq!(batch0_deps, vec![freeze.id.clone()]);
    let mut merge_deps = job_stages::dependencies_of(&mut conn, &merge.id)
        .await
        .unwrap();
    merge_deps.sort();
    let mut expected = vec![batch0.id.clone(), batch1.id.clone(), batch2.id.clone()];
    expected.sort();
    assert_eq!(merge_deps, expected, "merge 依赖全部批次");
    assert_eq!(
        job_stages::dependencies_of(&mut conn, &submit.id)
            .await
            .unwrap(),
        vec![upload.id.clone()]
    );
    let mut assemble_deps = job_stages::dependencies_of(&mut conn, &assemble.id)
        .await
        .unwrap();
    assemble_deps.sort();
    let mut expected_assemble = [merge.id.clone()];
    expected_assemble.sort();
    assert_eq!(
        assemble_deps.len(),
        1,
        "assemble 直接依赖 manual_merge（model_validate 尚未创建）"
    );
    drop(conn);

    // 依赖未完成的阶段不可领取（merge/assemble/submit），批次与上传可领取。
    let mut claimed_ids = Vec::new();
    for _ in 0..2 {
        match claim_next(&pool, "worker-a").await {
            Some(stage) => claimed_ids.push(stage.id),
            None => break,
        }
    }
    assert_eq!(claimed_ids.len(), 2, "两个批次并发额度都应可领取");
    assert!(
        claimed_ids
            .iter()
            .all(|id| [&batch0.id, &batch1.id, &batch2.id].contains(&id))
    );
    assert!(
        !claimed_ids.contains(&merge.id),
        "依赖未全部 succeeded 的阶段不得解锁"
    );

    // 批次全部成功后：merge 解锁，但 assemble 仍等 model_validate。
    for batch in [&batch0, &batch1, &batch2] {
        force_status(&pool, &batch.id, JobStatus::Succeeded).await;
    }
    let claimed = claim_next(&pool, "worker-a").await;
    assert_eq!(
        claimed.as_ref().map(|stage| stage.id.clone()),
        Some(merge.id.clone()),
        "批次完成后 merge 解锁"
    );
    assert_ne!(claimed.map(|stage| stage.id), Some(assemble.id));
}

#[tokio::test]
async fn concurrent_claims_yield_exactly_one_winner_with_new_epoch() {
    let dir = TestDir::new("t10-race");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "race").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let stage = insert_stage(&pool, &job_id, StageKind::TripoUpload, 0, JobStatus::Queued).await;

    let mut handles = Vec::new();
    for index in 0..8 {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            job_stages::claim_next(
                &pool,
                &job_stages::ClaimParams {
                    owner: format!("worker-{index}"),
                    now: Timestamp::now(),
                    lease: Duration::from_secs(30),
                    remote_generation_limit: 2,
                    manual_ai_batch_limit: 2,
                },
            )
            .await
            .unwrap()
        }));
    }
    let mut winners = Vec::new();
    for handle in handles {
        if let Some(stage) = handle.await.unwrap() {
            winners.push(stage);
        }
    }
    assert_eq!(winners.len(), 1, "同一阶段只能被一个 worker 领取");
    assert_eq!(winners[0].lease_epoch, 1, "领取原子取得新 leaseEpoch");
    assert_eq!(winners[0].status, JobStatus::Running);
    let stored = read_stage(&pool, &stage.id).await;
    assert_eq!(stored.lease_epoch, 1);
    assert!(stored.lease_until.is_some());
}

#[tokio::test]
async fn expired_worker_receipt_is_saved_but_cannot_advance_or_unlock() {
    let dir = TestDir::new("t10-late-receipt");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "late").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let upload = insert_stage(
        &pool,
        &job_id,
        StageKind::TripoUpload,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let submit = insert_stage(&pool, &job_id, StageKind::TripoSubmit, 0, JobStatus::Queued).await;
    let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    assert_eq!(upload.status, JobStatus::Succeeded);

    let fixture = FixtureServer::start(submit_success_scenario("task-late-1"));
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let gate = Gate::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoSubmit,
        FixtureTripoSubmit::new(&fixture.base_url(), Duration::from_secs(5), calls.clone())
            .with_gate(gate.clone()),
    );
    let executor = test_executor(&pool, registry, clock.clone(), test_config());

    // worker A 领取并停在"submitting 之后、发请求之前"。
    let worker_a = executor.clone();
    let task_a = tokio::spawn(async move { worker_a.tick().await });
    gate.wait_entered().await;
    assert_eq!(stage_status(&pool, &submit.id).await, JobStatus::Running);

    // 模拟 A 的租约到期（等价于真实时间流逝/进程失联）。
    expire_lease(&pool, &submit.id).await;

    // A 晚到的 receipt：允许保存**事实**（合同明文），但推进必须被拒绝。
    gate.release();
    let report_a = task_a.await.unwrap().unwrap();
    let TickOutcome::Executed(report) = report_a else {
        panic!("A 应执行了阶段");
    };
    assert!(
        report.status.is_none(),
        "过期 worker 的推进必须被拒绝：{:?}",
        report.note
    );
    assert_eq!(fixture.call_count("POST", TRIPO_SUBMIT_PATH), 1);

    let attempt = latest_attempt(&pool, &submit.id)
        .await
        .expect("attempt 存在");
    assert_eq!(
        attempt.submit_state,
        SubmitState::Accepted,
        "receipt 作为事实被保存"
    );
    assert_eq!(attempt.remote_task_id.as_deref(), Some("task-late-1"));
    assert_eq!(
        stage_status(&pool, &submit.id).await,
        JobStatus::Running,
        "旧 worker 不得推进业务状态"
    );
    assert_eq!(
        stage_status(&pool, &poll.id).await,
        JobStatus::Queued,
        "旧 worker 不得解锁后续阶段"
    );

    // 新 worker 的恢复扫描：按已知远端 ID 收敛为 succeeded（不重发付费请求）。
    let clock_later = Arc::new(ManualClock::new(Timestamp::now()));
    let mut resume_registry = StageRegistry::new();
    resume_registry.register(
        StageKind::TripoSubmit,
        FixtureTripoSubmit::new(&fixture.base_url(), Duration::from_secs(5), calls.clone()),
    );
    let resume = test_executor(&pool, resume_registry, clock_later, test_config());
    let recovery = resume.recover_expired_leases().await.unwrap();
    assert_eq!(recovery.succeeded, 1, "有 task ID → 按已知事实补推进");
    assert_eq!(stage_status(&pool, &submit.id).await, JobStatus::Succeeded);
    assert_eq!(
        read_stage(&pool, &submit.id).await.lease_epoch,
        2,
        "恢复方接管时 lease_epoch + 1（旧 worker 的 epoch=1 已无法推进）"
    );
    assert_eq!(
        fixture.call_count("POST", TRIPO_SUBMIT_PATH),
        1,
        "不重发付费请求"
    );
    assert_eq!(read_job(&pool, &job_id).await.status, JobStatus::Running);
}

#[tokio::test]
async fn cancel_marks_only_unsubmitted_stages_and_keeps_submitted_or_unknown() {
    let dir = TestDir::new("t10-cancel");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "cancel").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let batch = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        0,
        JobStatus::Queued,
    )
    .await;
    let upload = insert_stage(&pool, &job_id, StageKind::TripoUpload, 0, JobStatus::Queued).await;
    let submit = insert_stage(&pool, &job_id, StageKind::TripoSubmit, 0, JobStatus::Queued).await;
    let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    // 已提交（waiting_provider）、结果未知（submission_unknown）与运行中（无 accepted attempt）。
    seed_accepted_attempt(&pool, &job_id, &submit.id, "task-cancel", Timestamp::now()).await;
    force_status(&pool, &submit.id, JobStatus::WaitingProvider).await;
    force_status(&pool, &poll.id, JobStatus::SubmissionUnknown).await;
    force_status(&pool, &upload.id, JobStatus::Running).await;

    let mut conn = pool.acquire().await.expect("连接");
    let outcome = jobs_repo::cancel(&mut conn, &job_id, Timestamp::now())
        .await
        .expect("取消");
    drop(conn);
    assert!(outcome.cancelled);
    assert_eq!(outcome.job.status, JobStatus::Cancelled);
    assert_eq!(
        stage_status(&pool, &batch.id).await,
        JobStatus::Cancelled,
        "未提交阶段取消"
    );
    assert_eq!(
        stage_status(&pool, &upload.id).await,
        JobStatus::Cancelled,
        "running 且无 accepted attempt → 取消（旧 worker 之后无法推进）"
    );
    assert_eq!(
        stage_status(&pool, &submit.id).await,
        JobStatus::WaitingProvider,
        "已提交阶段保留（收尾/对账属 T15）"
    );
    assert_eq!(
        stage_status(&pool, &poll.id).await,
        JobStatus::SubmissionUnknown,
        "unknown 保留现场，等待对账"
    );
    // 取消后不再领取任何阶段（即使恢复也未解锁）。
    let executor = test_executor(
        &pool,
        StageRegistry::new(),
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );
    assert!(matches!(executor.tick().await.unwrap(), TickOutcome::Idle));
    // 已终态 job 再次取消：不写入、不报错。
    let mut conn = pool.acquire().await.expect("连接");
    let again = jobs_repo::cancel(&mut conn, &job_id, Timestamp::now())
        .await
        .expect("重复取消");
    assert!(!again.cancelled);
    assert_eq!(again.stages_cancelled, 0);
}

// ---------------------------------------------------------------------------
// AC-037 / AC-038：付费提交窗口与未知结果
// ---------------------------------------------------------------------------

#[tokio::test]
async fn paid_post_without_response_becomes_submission_unknown_and_never_repurchases() {
    let dir = TestDir::new("t10-unknown");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "unknown").await;
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

    let fixture = FixtureServer::start(submit_success_scenario("task-unknown-1"));
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoSubmit,
        FixtureTripoSubmit::new(&fixture.base_url(), Duration::from_secs(5), calls.clone()),
    );
    let executor = test_executor(&pool, registry, clock, test_config());

    // 断点 4：task ID 已由供应商返回（POST 已发出并收到响应）、事实尚未落库时进程死亡。
    failpoints::set(
        executor.owner(),
        PAID_AFTER_RESPONSE_BEFORE_RECEIPT,
        FailpointAction::Panic,
    );
    let crashed = executor.clone();
    let joined = tokio::spawn(async move { crashed.tick().await }).await;
    assert!(joined.is_err(), "断点应使本次执行崩溃：{joined:?}");
    failpoints::clear_owner(executor.owner());

    // 崩溃现场：attempt=submitting、无远端 ID、阶段仍在 running、租约未过期。
    let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
    assert_eq!(attempt.submit_state, SubmitState::Submitting);
    assert_eq!(attempt.remote_task_id, None);
    assert_eq!(fixture.call_count("POST", TRIPO_SUBMIT_PATH), 1);
    assert_eq!(stage_status(&pool, &submit.id).await, JobStatus::Running);

    // 重启（新进程/新 owner）：租约过期后恢复 → submission_unknown，且**不重发**。
    expire_lease(&pool, &submit.id).await;
    let resumed_calls = Arc::new(AtomicUsize::new(0));
    let mut resume_registry = StageRegistry::new();
    resume_registry.register(
        StageKind::TripoSubmit,
        FixtureTripoSubmit::new(
            &fixture.base_url(),
            Duration::from_secs(5),
            resumed_calls.clone(),
        ),
    );
    let resume = test_executor(
        &pool,
        resume_registry,
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );
    let recovery = resume.recover_expired_leases().await.unwrap();
    assert_eq!(recovery.submission_unknown, 1);

    let stage = read_stage(&pool, &submit.id).await;
    assert_eq!(stage.status, JobStatus::SubmissionUnknown);
    let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
    assert_eq!(attempt.submit_state, SubmitState::Unknown);
    assert_eq!(attempt.remote_task_id, None, "没有远端 ID 可继续查询");
    assert_eq!(
        read_job(&pool, &job_id).await.status,
        JobStatus::SubmissionUnknown,
        "父 job 展示对应状态与阻塞分支"
    );
    assert_eq!(
        stage_status(&pool, &poll.id).await,
        JobStatus::Queued,
        "未知提交不解锁后续购买"
    );

    // 之后任何 tick 都不得领取该阶段（不自动重购）。
    for _ in 0..3 {
        assert!(matches!(resume.tick().await.unwrap(), TickOutcome::Idle));
    }
    assert_eq!(resumed_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.call_count("POST", TRIPO_SUBMIT_PATH), 1);
}

/// 冲突处理器：绑定已存在的 attempt，再观察一个不同 ID（模拟迟到/重复响应）。
struct BindAndConflictHandler {
    new_id: String,
}

impl StageHandler for BindAndConflictHandler {
    fn run<'a>(&'a self, ctx: &'a mut StageContext) -> StageFuture<'a> {
        Box::pin(async move {
            let existing = ctx.attempt.clone().expect("已接受的 attempt");
            ctx.submission.bind_attempt(existing.id.clone());
            let observation = ctx.submission.record_remote_task_id(&self.new_id).await?;
            assert!(
                matches!(
                    observation,
                    everything_manual::jobs::RemoteTaskObservation::Conflict { .. }
                ),
                "不同 ID 必须判定为冲突：{observation:?}"
            );
            // 处理器即使返回"成功"，执行器也必须按冲突覆盖为 submission_unknown。
            Ok(StageOutcome::succeeded())
        })
    }
}

#[tokio::test]
async fn remote_task_id_conflict_is_recorded_never_overwrites_and_pauses_branch() {
    let dir = TestDir::new("t10-conflict");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "conflict").await;
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

    // 该 attempt 已经记录了远端 ID（例如上一次运行成功提交）。
    let attempt_id = seed_accepted_attempt(
        &pool,
        &job_id,
        &submit.id,
        "task-existing",
        Timestamp::now(),
    )
    .await;
    // 阶段被重新领取（例如人工重试后），窗口会在同一 attempt 上再观察一次事实。
    force_status(&pool, &submit.id, JobStatus::Queued).await;

    // fixture 用于证明"没有任何新请求发出"。
    let fixture = FixtureServer::start(submit_success_scenario("task-returned-later"));
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoSubmit,
        BindAndConflictHandler {
            new_id: "task-returned-later".to_owned(),
        },
    );
    let executor = test_executor(
        &pool,
        registry,
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );
    // 双保险：窗口层直接验证一次冲突（不经过执行器）。
    let mut window = everything_manual::jobs::SubmissionWindow::new(
        pool.clone(),
        job_id.clone(),
        submit.id.clone(),
        "test-owner",
        Timestamp::now(),
    );
    window.bind_attempt(attempt_id.clone());
    let observation = window
        .record_remote_task_id("task-returned-later")
        .await
        .expect("冲突可判定");
    assert_eq!(
        observation,
        everything_manual::jobs::RemoteTaskObservation::Conflict {
            existing: "task-existing".to_owned()
        }
    );
    assert!(window.conflict().is_some());
    // window 直接调用把 attempt 标记为 unknown；为了继续验证执行器的归一化，这里恢复现场。
    sqlx::query("UPDATE provider_attempts SET submit_state = 'accepted' WHERE id = ?")
        .bind(&attempt_id)
        .execute(&pool)
        .await
        .expect("恢复现场");

    let outcome = executor.tick().await.unwrap();
    let TickOutcome::Executed(report) = outcome else {
        panic!("应执行提交阶段");
    };
    assert_eq!(
        report.status,
        Some(JobStatus::SubmissionUnknown),
        "冲突必须覆盖处理器的成功结论"
    );
    let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
    assert_eq!(
        attempt.remote_task_id.as_deref(),
        Some("task-existing"),
        "不得覆盖已有远端 ID"
    );
    assert_eq!(attempt.submit_state, SubmitState::Unknown);
    assert!(
        attempt
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("冲突"),
        "{:?}",
        attempt.last_error
    );
    assert_eq!(
        stage_status(&pool, &submit.id).await,
        JobStatus::SubmissionUnknown
    );
    assert_eq!(
        stage_status(&pool, &poll.id).await,
        JobStatus::Queued,
        "分支暂停"
    );
    let audit: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE action = 'provider_attempt_remote_task_id_conflict'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(audit >= 1, "冲突必须留审计（停机告警的可查证据）");
    assert_eq!(fixture.request_total(), 0, "冲突路径不发出任何新请求");
}

#[tokio::test]
async fn manual_batch_without_persisted_response_becomes_unknown_and_sync_branch_has_no_attach() {
    let dir = TestDir::new("t10-manual-unknown");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, item_id) = seed_job(&pool, "manual-unknown").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let batch0 = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let batch1 = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        1,
        JobStatus::Queued,
    )
    .await;
    insert_stage(&pool, &job_id, StageKind::ManualMerge, 0, JobStatus::Queued).await;
    let result_asset = seed_result_asset(&pool, &item_id, "manual-unknown").await;

    let fixture = FixtureServer::from_scenario_file("manual_ai_happy.json");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::ManualExtract,
        FixtureManualBatch {
            base_url: fixture.base_url(),
            read_timeout: Duration::from_secs(5),
            result_asset_id: result_asset,
            calls: calls.clone(),
        },
    );
    let executor = test_executor(
        &pool,
        registry,
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );

    // 断点 5：请求已发出、完整响应尚未持久化 → 进程崩溃。
    failpoints::set(
        executor.owner(),
        MANUAL_AFTER_REQUEST_BEFORE_RESPONSE,
        FailpointAction::Panic,
    );
    let crashed = executor.clone();
    let joined = tokio::spawn(async move { crashed.tick().await }).await;
    assert!(joined.is_err(), "断点应使本次执行崩溃：{joined:?}");
    failpoints::clear_owner(executor.owner());

    let attempt = latest_attempt(&pool, &batch1.id)
        .await
        .expect("attempt 存在");
    assert_eq!(attempt.submit_state, SubmitState::Submitting);
    assert_eq!(fixture.call_count("POST", MANUAL_AI_RESPONSES_PATH), 1);
    assert_eq!(
        stage_status(&pool, &batch0.id).await,
        JobStatus::Succeeded,
        "已完成批次不动"
    );

    expire_lease(&pool, &batch1.id).await;
    let mut resume_registry = StageRegistry::new();
    resume_registry.register(
        StageKind::ManualExtract,
        ScriptedHandler::new(vec![StageOutcome::succeeded()]),
    );
    let resume = test_executor(
        &pool,
        resume_registry,
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );
    let recovery = resume.recover_expired_leases().await.unwrap();
    assert_eq!(
        recovery.submission_unknown, 1,
        "同步批次未持久化完整响应 → unknown"
    );
    let stage = read_stage(&pool, &batch1.id).await;
    assert_eq!(stage.status, JobStatus::SubmissionUnknown);
    let attempt = latest_attempt(&pool, &batch1.id).await.expect("attempt");
    assert_eq!(attempt.submit_state, SubmitState::Unknown);
    assert!(attempt.remote_task_id.is_none());
    assert!(
        attempt.response_id.is_none(),
        "response_id 不假定可轮询/重取：未持久化响应时也不得据此恢复"
    );
    assert_eq!(
        read_job(&pool, &job_id).await.status,
        JobStatus::SubmissionUnknown
    );
    // 已完成批次不重跑、不重复付费。
    assert_eq!(fixture.call_count("POST", MANUAL_AI_RESPONSES_PATH), 1);
    assert!(matches!(resume.tick().await.unwrap(), TickOutcome::Idle));
}

#[tokio::test]
async fn persisted_result_without_checkpoint_is_advanced_on_recovery_without_repaying() {
    let dir = TestDir::new("t10-result-fact");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, item_id) = seed_job(&pool, "result-fact").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let batch = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        0,
        JobStatus::Queued,
    )
    .await;
    let result_asset = seed_result_asset(&pool, &item_id, "result-fact").await;

    let fixture = FixtureServer::from_scenario_file("manual_ai_happy.json");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::ManualExtract,
        FixtureManualBatch {
            base_url: fixture.base_url(),
            read_timeout: Duration::from_secs(5),
            result_asset_id: result_asset.clone(),
            calls: calls.clone(),
        },
    );
    let executor = test_executor(
        &pool,
        registry,
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );

    // 断点 6：结果事实（结果资产 + receipt + usage）已落库、checkpoint 未推进。
    failpoints::set(
        executor.owner(),
        RESULT_FACT_BEFORE_CHECKPOINT,
        FailpointAction::Panic,
    );
    let crashed = executor.clone();
    let joined = tokio::spawn(async move { crashed.tick().await }).await;
    assert!(joined.is_err(), "断点应使本次执行崩溃：{joined:?}");
    failpoints::clear_owner(executor.owner());

    let stage = read_stage(&pool, &batch.id).await;
    assert_eq!(stage.status, JobStatus::Running, "checkpoint 未推进");
    assert_eq!(
        stage.result_asset_id.as_deref(),
        Some(result_asset.as_str()),
        "结果事实已保存"
    );
    let attempt = latest_attempt(&pool, &batch.id).await.expect("attempt");
    assert_eq!(attempt.submit_state, SubmitState::Accepted);
    assert_eq!(fixture.call_count("POST", MANUAL_AI_RESPONSES_PATH), 1);

    expire_lease(&pool, &batch.id).await;
    let resume = test_executor(
        &pool,
        StageRegistry::new(),
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );
    let recovery = resume.recover_expired_leases().await.unwrap();
    assert_eq!(
        recovery.succeeded, 1,
        "结果已持久化 → 校验后补推进，不重新付费"
    );
    let stage = read_stage(&pool, &batch.id).await;
    assert_eq!(stage.status, JobStatus::Succeeded);
    assert_eq!(
        stage.result_asset_id.as_deref(),
        Some(result_asset.as_str())
    );
    assert_eq!(
        fixture.call_count("POST", MANUAL_AI_RESPONSES_PATH),
        1,
        "恢复不得重新请求（不重复付费）"
    );
}

// ---------------------------------------------------------------------------
// AC-036：退避、Retry-After、needs_input 与总等待预算
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transient_failures_back_off_2_4_8_16_32_then_fail_and_respect_retry_after() {
    let dir = TestDir::new("t10-backoff");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "backoff").await;
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
    insert_stage(
        &pool,
        &job_id,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    seed_accepted_attempt(&pool, &job_id, &poll.id, "task-backoff", Timestamp::now()).await;

    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoPoll,
        ScriptedHandler::new(vec![
            StageOutcome::Retryable {
                reason: "503 Service Unavailable".to_owned(),
                retry_after_seconds: None,
            };
            6
        ]),
    );
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = test_executor(&pool, registry, clock.clone(), test_config());

    let base = [2_000_i64, 4_000, 8_000, 16_000, 32_000];
    for (index, expected) in base.iter().enumerate() {
        let outcome = executor.tick().await.unwrap();
        let TickOutcome::Executed(report) = outcome else {
            panic!("第 {} 次重试应执行阶段", index + 1);
        };
        assert_eq!(report.status, Some(JobStatus::RetryWait));
        let stage = read_stage(&pool, &poll.id).await;
        assert_eq!(stage.attempt_count, index as i64 + 1);
        let delay = stage.next_run_at.unwrap().as_millis() - clock.now().as_millis();
        assert_eq!(
            delay, *expected,
            "退避序列必须是 2/4/8/16/32 秒（固定 jitter）"
        );
        // 到点后重新领取。
        clock.advance_millis(*expected + 1);
    }

    // 第 6 次失败：安全重试次数用尽 → failed（父 job failed）。
    let outcome = executor.tick().await.unwrap();
    let TickOutcome::Executed(report) = outcome else {
        panic!("第 6 次应执行并判定失败");
    };
    assert_eq!(report.status, Some(JobStatus::Failed));
    let stage = read_stage(&pool, &poll.id).await;
    assert_eq!(stage.status, JobStatus::Failed);
    assert!(
        stage
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("用尽"),
        "{:?}",
        stage.last_error
    );
    assert_eq!(read_job(&pool, &job_id).await.status, JobStatus::Failed);
}

#[tokio::test]
async fn retry_after_is_respected_and_capped() {
    let dir = TestDir::new("t10-retry-after");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "retry-after").await;
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
    insert_stage(
        &pool,
        &job_id,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    seed_accepted_attempt(
        &pool,
        &job_id,
        &poll.id,
        "task-retry-after",
        Timestamp::now(),
    )
    .await;

    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoPoll,
        ScriptedHandler::new(vec![
            StageOutcome::Retryable {
                reason: "429 Too Many Requests".to_owned(),
                retry_after_seconds: Some(3),
            },
            StageOutcome::Retryable {
                reason: "429 Too Many Requests".to_owned(),
                retry_after_seconds: Some(999_999),
            },
        ]),
    );
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = test_executor(&pool, registry, clock.clone(), test_config());

    let _ = executor.tick().await.unwrap();
    let stage = read_stage(&pool, &poll.id).await;
    assert_eq!(
        stage.next_run_at.unwrap().as_millis() - clock.now().as_millis(),
        3_000,
        "Retry-After: 3 必须被尊重（不加 jitter）"
    );
    clock.advance_millis(3_100);

    let _ = executor.tick().await.unwrap();
    let stage = read_stage(&pool, &poll.id).await;
    let delay = stage.next_run_at.unwrap().as_millis() - clock.now().as_millis();
    assert_eq!(
        delay,
        (core_jobs::RETRY_AFTER_CAP_SECONDS as i64) * 1_000,
        "Retry-After 超过上限时截断"
    );
    assert!(
        stage
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("429"),
        "原因被记录：{:?}",
        stage.last_error
    );
}

#[tokio::test]
async fn insufficient_inputs_go_to_needs_input_with_actionable_items() {
    let dir = TestDir::new("t10-needs-input");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "needs-input").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let upload = insert_stage(&pool, &job_id, StageKind::TripoUpload, 0, JobStatus::Queued).await;

    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoUpload,
        ScriptedHandler::new(vec![StageOutcome::NeedsInput {
            items: vec![
                everything_manual::jobs::MissingItem::new(
                    "missing_photo_view",
                    "缺少左侧视图照片，请补齐后继续",
                ),
                everything_manual::jobs::MissingItem::new(
                    "unsupported_model",
                    "该型号不支持首版自动生成，请换用支持清单内的型号",
                ),
            ],
        }]),
    );
    let executor = test_executor(
        &pool,
        registry,
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );
    let outcome = executor.tick().await.unwrap();
    let TickOutcome::Executed(report) = outcome else {
        panic!("应执行阶段");
    };
    assert_eq!(report.status, Some(JobStatus::NeedsInput));
    let stage = read_stage(&pool, &upload.id).await;
    assert_eq!(stage.status, JobStatus::NeedsInput);
    let items = stage.needs_input_json.expect("缺项必须落库");
    assert_eq!(items.as_array().map(Vec::len), Some(2));
    assert_eq!(items[0]["code"], "missing_photo_view");
    assert!(items[0]["message"].as_str().unwrap().contains("补齐"));
    assert_eq!(read_job(&pool, &job_id).await.status, JobStatus::NeedsInput);
    // 不无休止重试：needs_input 不再是可领取状态。
    assert!(matches!(executor.tick().await.unwrap(), TickOutcome::Idle));
}

/// AC-036 第 4 子句（**阶段自带 accepted 远端事实**的形态）：等待起点优先取该阶段
/// 自己的 accepted attempt（真实链路形态见下一个用例 `wait_budget_triggers_...`）。
#[tokio::test]
async fn remote_wait_budget_turns_poll_into_needs_input_and_keeps_task_id() {
    let dir = TestDir::new("t10-wait-budget");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "wait-budget").await;
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
    insert_stage(
        &pool,
        &job_id,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    // 已提交 30 分钟零 1 秒（超过总等待预算）。
    let started = Timestamp::now()
        .checked_add_millis(-((core_jobs::REMOTE_WAIT_BUDGET_SECONDS as i64 + 1) * 1000))
        .unwrap();
    let attempt_id =
        seed_accepted_attempt(&pool, &job_id, &poll.id, "task-long-wait", started).await;

    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoPoll,
        ScriptedHandler::new(vec![StageOutcome::WaitingProvider]),
    );
    let executor = test_executor(
        &pool,
        registry,
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );
    let outcome = executor.tick().await.unwrap();
    let TickOutcome::Executed(report) = outcome else {
        panic!("应执行查询阶段");
    };
    assert_eq!(report.status, Some(JobStatus::NeedsInput));
    let stage = read_stage(&pool, &poll.id).await;
    assert_eq!(stage.status, JobStatus::NeedsInput);
    let items = stage.needs_input_json.expect("缺项 JSON");
    assert_eq!(items[0]["code"], "remote_wait_budget_exceeded");
    assert!(
        items[0]["message"]
            .as_str()
            .unwrap()
            .contains("远端任务 ID 已保留"),
        "{items}"
    );
    // task_id 保留：恢复只查询、不重新购买。
    let attempt = latest_attempt(&pool, &poll.id).await.expect("attempt");
    assert_eq!(attempt.id, attempt_id);
    assert_eq!(attempt.remote_task_id.as_deref(), Some("task-long-wait"));
    assert_eq!(attempt.submit_state, SubmitState::Accepted);
    assert_eq!(read_job(&pool, &job_id).await.status, JobStatus::NeedsInput);
    assert!(matches!(executor.tick().await.unwrap(), TickOutcome::Idle));
}

/// AC-036 第 4 子句（**真实轮询链路形态**；BUG-003 回归）。
///
/// `tripo_poll` 阶段自身没有 attempt：task ID 与提交时刻都来自同一 job 的
/// `tripo_submit` accepted 事实（与处理器取 task ID 的同一份事实）。用可注入时钟
/// 按 `next_run_at` 节奏推进总等待预算，验证：
/// - 预算之前：保持 `waiting_provider`，轮询节奏 3/6/12/15 秒；
/// - 超预算：转 `needs_input`（缺项 `remote_wait_budget_exceeded`）且**保留 task_id**；
/// - 全程不产生第二次付费提交（全 job 仍只有 1 条 attempt）。
#[tokio::test]
async fn wait_budget_triggers_on_real_chain_shape_without_poll_attempt() {
    let dir = TestDir::new("t10-wait-budget-real");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "wait-budget-real").await;
    for kind in [StageKind::FreezeInputs, StageKind::TripoUpload] {
        insert_stage(&pool, &job_id, kind, 0, JobStatus::Succeeded).await;
    }
    let submit = insert_stage(
        &pool,
        &job_id,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;

    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    // 提交事实（task ID + 提交时刻）落在 submit 阶段；轮询阶段没有自己的 attempt。
    let attempt_id =
        seed_accepted_attempt(&pool, &job_id, &submit.id, "task-real-chain", clock.now()).await;
    assert!(
        latest_attempt(&pool, &poll.id).await.is_none(),
        "真实形态：轮询阶段不得有自己的 attempt"
    );

    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoPoll,
        // 远端一直 running：预算内 waiting_provider，超预算转 needs_input。
        ScriptedHandler::new(vec![StageOutcome::WaitingProvider; 400]),
    );
    let executor = test_executor(&pool, registry, clock.clone(), test_config());

    let mut polls = 0usize;
    let mut delays = Vec::new();
    let final_stage = loop {
        // 走到下一次可领取时点（推进一个最大轮询间隔；真实实现由 next_run_at 驱动）。
        clock.advance_millis(15_000);
        let _ = executor.tick().await.unwrap();
        polls += 1;
        let stage = read_stage(&pool, &poll.id).await;
        match stage.status {
            JobStatus::WaitingProvider => {
                let next_run_at = stage.next_run_at.expect("轮询必须写 next_run_at");
                delays.push(next_run_at.as_millis() - clock.now().as_millis());
                assert!(
                    polls < 400,
                    "预算未在预期轮次内触发（真实节奏下约 120 次轮询）"
                );
            }
            JobStatus::NeedsInput => break stage,
            other => panic!("真实形态下意外状态 {other:?}（第 {polls} 次轮询）"),
        }
    };
    assert!(delays.len() >= 4, "预算之前应观察到多次 waiting_provider");
    assert_eq!(
        &delays[..4],
        &[3_000, 6_000, 12_000, 15_000],
        "预算之前轮询节奏 3/6/12/15 秒（锚点修复不影响节奏）"
    );

    // 阈值行为：needs_input + 可行动缺项 + 保留 task_id。
    let items = final_stage
        .needs_input_json
        .clone()
        .expect("缺项 JSON 必须落库");
    assert_eq!(items[0]["code"], "remote_wait_budget_exceeded");
    assert!(
        items[0]["message"]
            .as_str()
            .unwrap()
            .contains("远端任务 ID 已保留"),
        "{items}"
    );
    assert!(
        final_stage
            .last_error
            .clone()
            .unwrap_or_default()
            .contains("task_id 已保留")
    );
    let attempt = latest_attempt(&pool, &submit.id)
        .await
        .expect("提交事实仍在");
    assert_eq!(attempt.id, attempt_id);
    assert_eq!(attempt.submit_state, SubmitState::Accepted);
    assert_eq!(attempt.remote_task_id.as_deref(), Some("task-real-chain"));
    assert!(
        latest_attempt(&pool, &poll.id).await.is_none(),
        "轮询阶段全程不建自己的 attempt"
    );
    assert_eq!(read_job(&pool, &job_id).await.status, JobStatus::NeedsInput);
    // 恢复只查询、不重新购买：全 job 仍只有 1 条 attempt（无第二次付费提交）。
    let attempt_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM provider_attempts WHERE job_id = ?")
            .bind(&job_id)
            .fetch_one(&pool)
            .await
            .expect("统计 attempt");
    assert_eq!(attempt_rows, 1, "不得产生第二次付费提交");
    // 不无休止轮询：needs_input 不再是可领取状态。
    assert!(matches!(executor.tick().await.unwrap(), TickOutcome::Idle));
    eprintln!(
        "[rd probe] 真实轮询链路形态：{} 次轮询（时钟推进 {}s）后 poll status={:?}，task_id=task-real-chain 保留",
        polls,
        polls * 15,
        final_stage.status
    );
}

/// BUG-003 修复的归属规则：等待锚点按 **job** 归属——同一库内另一个 job 的已老化
/// 提交事实不得把本 job 的轮询阶段推入 `needs_input`（多任务不串用起点）。
#[tokio::test]
async fn wait_budget_anchor_does_not_leak_across_jobs() {
    let dir = TestDir::new("t10-wait-budget-scope");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let clock = Arc::new(ManualClock::new(Timestamp::now()));

    // job A：提交已老化 31 分钟（超预算）→ 轮询阶段应 needs_input。
    let (job_a, _item_a) = seed_job(&pool, "wait-scope-a").await;
    for kind in [StageKind::FreezeInputs, StageKind::TripoUpload] {
        insert_stage(&pool, &job_a, kind, 0, JobStatus::Succeeded).await;
    }
    let submit_a = insert_stage(
        &pool,
        &job_a,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let poll_a = insert_stage(&pool, &job_a, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    let aged = clock
        .now()
        .checked_add_millis(-((core_jobs::REMOTE_WAIT_BUDGET_SECONDS as i64 + 60) * 1000))
        .unwrap();
    seed_accepted_attempt(&pool, &job_a, &submit_a.id, "task-a", aged).await;

    // job B：刚提交（预算内）→ 轮询阶段只应 waiting_provider。
    let (job_b, _item_b) = seed_job(&pool, "wait-scope-b").await;
    for kind in [StageKind::FreezeInputs, StageKind::TripoUpload] {
        insert_stage(&pool, &job_b, kind, 0, JobStatus::Succeeded).await;
    }
    let submit_b = insert_stage(
        &pool,
        &job_b,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let poll_b = insert_stage(&pool, &job_b, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    seed_accepted_attempt(&pool, &job_b, &submit_b.id, "task-b", clock.now()).await;

    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoPoll,
        ScriptedHandler::new(vec![StageOutcome::WaitingProvider; 8]),
    );
    let executor = test_executor(&pool, registry, clock.clone(), test_config());

    // 两次 tick 覆盖两个 job 的轮询阶段（领取顺序与断言无关：A 必 needs_input，B 必等待）。
    let _ = executor.tick().await.unwrap();
    let _ = executor.tick().await.unwrap();

    assert_eq!(
        stage_status(&pool, &poll_a.id).await,
        JobStatus::NeedsInput,
        "A：超预算 → needs_input"
    );
    let stage_b = read_stage(&pool, &poll_b.id).await;
    assert_eq!(
        stage_b.status,
        JobStatus::WaitingProvider,
        "B：不得串用 A 的老化起点（预算内保持等待）"
    );
    assert!(stage_b.needs_input_json.is_none(), "B 不应有缺项");
    assert_eq!(
        stage_b.next_run_at.expect("B 有下一次轮询时刻").as_millis() - clock.now().as_millis(),
        3_000,
        "B 按正常轮询节奏（首次 3s）等待"
    );
    let attempt_b = latest_attempt(&pool, &submit_b.id)
        .await
        .expect("B 提交事实");
    assert_eq!(attempt_b.remote_task_id.as_deref(), Some("task-b"));
    assert_eq!(
        read_job(&pool, &job_b).await.status,
        JobStatus::WaitingProvider
    );
    // 两个 job 都已收敛到各自状态，没有新的可领取工作。
    assert!(matches!(executor.tick().await.unwrap(), TickOutcome::Idle));
}

/// 同源锚点覆盖（QA 回合 10 非阻断建议 4）：`model_download` 等下游远端阶段返回
/// `WaitingProvider` 时，等待起点同样回退到 job 的 accepted 提交事实。
#[tokio::test]
async fn wait_budget_covers_downstream_remote_stages_via_submit_anchor() {
    let dir = TestDir::new("t10-wait-budget-download");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "wait-budget-download").await;
    for kind in [
        StageKind::FreezeInputs,
        StageKind::TripoUpload,
        StageKind::TripoPoll,
    ] {
        insert_stage(&pool, &job_id, kind, 0, JobStatus::Succeeded).await;
    }
    let submit = insert_stage(
        &pool,
        &job_id,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let download = insert_stage(
        &pool,
        &job_id,
        StageKind::ModelDownload,
        0,
        JobStatus::Queued,
    )
    .await;
    let started = Timestamp::now()
        .checked_add_millis(-((core_jobs::REMOTE_WAIT_BUDGET_SECONDS as i64 + 1) * 1000))
        .unwrap();
    seed_accepted_attempt(&pool, &job_id, &submit.id, "task-download", started).await;

    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::ModelDownload,
        ScriptedHandler::new(vec![StageOutcome::WaitingProvider]),
    );
    let executor = test_executor(
        &pool,
        registry,
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );
    let TickOutcome::Executed(report) = executor.tick().await.unwrap() else {
        panic!("model_download 阶段应被领取并执行");
    };
    assert_eq!(report.status, Some(JobStatus::NeedsInput));
    let stage = read_stage(&pool, &download.id).await;
    assert_eq!(stage.status, JobStatus::NeedsInput);
    let items = stage.needs_input_json.expect("缺项 JSON");
    assert_eq!(items[0]["code"], "remote_wait_budget_exceeded");
    let attempt = latest_attempt(&pool, &submit.id).await.expect("提交事实");
    assert_eq!(attempt.remote_task_id.as_deref(), Some("task-download"));
    assert!(
        latest_attempt(&pool, &download.id).await.is_none(),
        "model_download 不得为等待建 attempt"
    );
}

#[tokio::test]
async fn poll_pace_ramps_from_three_to_fifteen_seconds() {
    let dir = TestDir::new("t10-poll-pace");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "poll-pace").await;
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
    insert_stage(
        &pool,
        &job_id,
        StageKind::TripoSubmit,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    seed_accepted_attempt(&pool, &job_id, &poll.id, "task-pace", Timestamp::now()).await;

    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoPoll,
        ScriptedHandler::new(vec![StageOutcome::WaitingProvider; 4]),
    );
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let executor = test_executor(&pool, registry, clock.clone(), test_config());

    for (index, expected_seconds) in [3_i64, 6, 12, 15].iter().enumerate() {
        let outcome = executor.tick().await.unwrap();
        assert!(
            matches!(outcome, TickOutcome::Executed(_)),
            "第 {index} 次查询"
        );
        let stage = read_stage(&pool, &poll.id).await;
        assert_eq!(stage.status, JobStatus::WaitingProvider);
        assert_eq!(stage.poll_count, index as i64 + 1);
        let delay = stage.next_run_at.unwrap().as_millis() - clock.now().as_millis();
        assert_eq!(delay, expected_seconds * 1_000, "轮询节奏 3→6→12→15 秒");
        clock.advance_millis(expected_seconds * 1_000 + 1);
    }
}

// ---------------------------------------------------------------------------
// DAG 分支隔离、并发上限、续约与未注册处理器
// ---------------------------------------------------------------------------

#[tokio::test]
async fn failed_branch_does_not_block_independent_branch_and_job_status_is_aggregated() {
    let dir = TestDir::new("t10-branch");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "branch").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let batch = insert_stage(
        &pool,
        &job_id,
        StageKind::ManualExtract,
        0,
        JobStatus::Queued,
    )
    .await;
    let merge = insert_stage(&pool, &job_id, StageKind::ManualMerge, 0, JobStatus::Queued).await;
    let upload = insert_stage(&pool, &job_id, StageKind::TripoUpload, 0, JobStatus::Queued).await;
    let submit = insert_stage(&pool, &job_id, StageKind::TripoSubmit, 0, JobStatus::Queued).await;
    let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;
    let download = insert_stage(
        &pool,
        &job_id,
        StageKind::ModelDownload,
        0,
        JobStatus::Queued,
    )
    .await;
    let validate = insert_stage(
        &pool,
        &job_id,
        StageKind::ModelValidate,
        0,
        JobStatus::Queued,
    )
    .await;
    let assemble = insert_stage(
        &pool,
        &job_id,
        StageKind::AssembleDraft,
        0,
        JobStatus::Queued,
    )
    .await;
    assert_eq!(batch.status, JobStatus::Queued);

    // 知识分支失败（明确失败，不占用重试额度）。
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::ManualExtract,
        ScriptedHandler::new(vec![StageOutcome::Failed {
            reason: "批次内容无法解析".to_owned(),
        }]),
    );
    for kind in [
        StageKind::TripoUpload,
        StageKind::TripoSubmit,
        StageKind::TripoPoll,
        StageKind::ModelDownload,
        StageKind::ModelValidate,
        StageKind::AssembleDraft,
        StageKind::ManualMerge,
    ] {
        registry.register(kind, ScriptedHandler::new(vec![]));
    }
    let executor = test_executor(
        &pool,
        registry,
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );

    // 驱逐到没有更多可执行阶段：Tripo 分支应全部完成，merge 永不解锁，assemble 保持 queued。
    for _ in 0..40 {
        if matches!(executor.tick().await.unwrap(), TickOutcome::Idle) {
            break;
        }
    }
    assert_eq!(stage_status(&pool, &batch.id).await, JobStatus::Failed);
    for stage in [&upload, &submit, &poll, &download, &validate] {
        assert_eq!(
            stage_status(&pool, &stage.id).await,
            JobStatus::Succeeded,
            "独立分支不受另一分支失败影响：{}",
            stage.stage_kind.as_str()
        );
    }
    assert_eq!(
        stage_status(&pool, &merge.id).await,
        JobStatus::Queued,
        "依赖失败的批次 → merge 不解锁"
    );
    assert_eq!(stage_status(&pool, &assemble.id).await, JobStatus::Queued);
    assert_eq!(read_job(&pool, &job_id).await.status, JobStatus::Failed);
}

#[tokio::test]
async fn concurrency_limits_allow_two_manual_batches_and_two_remote_stages() {
    let dir = TestDir::new("t10-concurrency");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();

    // 3 个说明书批次（同一 job）与 3 个远端上传（不同 job，避免 DAG 相互依赖）。
    let (job_a, _) = seed_job(&pool, "conc-a").await;
    insert_stage(
        &pool,
        &job_a,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    for index in 0..3 {
        insert_stage(
            &pool,
            &job_a,
            StageKind::ManualExtract,
            index,
            JobStatus::Queued,
        )
        .await;
    }

    let mut uploads = Vec::new();
    for tag in ["conc-b", "conc-c", "conc-d"] {
        let (job, _) = seed_job(&pool, tag).await;
        insert_stage(
            &pool,
            &job,
            StageKind::FreezeInputs,
            0,
            JobStatus::Succeeded,
        )
        .await;
        uploads.push(insert_stage(&pool, &job, StageKind::TripoUpload, 0, JobStatus::Queued).await);
    }

    let gate = Gate::new();
    // 两个分组各自统计在飞数量（不要共用一个计数器：峰值含义会串组）。
    let inflight_batch = Arc::new(AtomicUsize::new(0));
    let inflight_remote = Arc::new(AtomicUsize::new(0));
    let max_batch = Arc::new(AtomicUsize::new(0));
    let max_remote = Arc::new(AtomicUsize::new(0));
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::ManualExtract,
        BlockingHandler::new(inflight_batch.clone(), max_batch.clone(), gate.clone()),
    );
    registry.register(
        StageKind::TripoUpload,
        BlockingHandler::new(inflight_remote.clone(), max_remote.clone(), gate.clone()),
    );

    let executor = test_executor(
        &pool,
        registry,
        Arc::new(ManualClock::new(Timestamp::now())),
        test_config(),
    );
    let handle = executor.start();

    // 两个分组的并发额度都应被填满（领取谓词分别限流 2/2）。
    let deadline = Instant::now() + Duration::from_secs(10);
    while (max_batch.load(Ordering::SeqCst) < 2 || max_remote.load(Ordering::SeqCst) < 2)
        && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let statuses: Vec<(String, String)> =
        sqlx::query_as("SELECT stage_kind, status FROM job_stages ORDER BY created_at")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        max_batch.load(Ordering::SeqCst),
        2,
        "说明书批次并发上限 = 2；阶段状态：{statuses:?}"
    );
    assert_eq!(
        max_remote.load(Ordering::SeqCst),
        2,
        "远端生成并发上限 = 2；阶段状态：{statuses:?}"
    );

    // 再等一会，确认没有第 3 个越过上限。
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(max_batch.load(Ordering::SeqCst), 2);
    assert_eq!(max_remote.load(Ordering::SeqCst), 2);
    assert_eq!(
        inflight_batch.load(Ordering::SeqCst) + inflight_remote.load(Ordering::SeqCst),
        4,
        "两组共 4 个在飞（2 批次 + 2 远端）"
    );

    // 放行：第 3 个批次在第 1 个完成后才会被领取（上限是限流，不是阻塞）。
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut pending = 1_i64;
    while Instant::now() < deadline {
        gate.release_all();
        tokio::time::sleep(Duration::from_millis(60)).await;
        pending = sqlx::query_scalar(
            "SELECT COUNT(*) FROM job_stages \
              WHERE stage_kind = 'manual_extract' AND status <> 'succeeded'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if pending == 0 {
            break;
        }
    }
    handle.shutdown().await;
    assert_eq!(pending, 0, "放行后所有批次最终完成");
    assert_eq!(uploads.len(), 3);
    assert_eq!(max_batch.load(Ordering::SeqCst), 2, "全程不超过 2");
    assert_eq!(max_remote.load(Ordering::SeqCst), 2, "全程不超过 2");
}

#[tokio::test]
async fn lease_renewal_keeps_stage_alive_and_unregistered_handler_defers_without_retries() {
    let dir = TestDir::new("t10-renew");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "renew").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let upload = insert_stage(&pool, &job_id, StageKind::TripoUpload, 0, JobStatus::Queued).await;

    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let config = ExecutorConfig {
        lease: Duration::from_millis(400),
        renew: Duration::from_millis(40),
        ..test_config()
    };
    let samples = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoUpload,
        LeaseWatcherHandler {
            pool: pool.clone(),
            clock: clock.clone(),
            samples: samples.clone(),
        },
    );
    let executor = test_executor(&pool, registry, clock.clone(), config.clone());
    let report = match executor.tick().await.unwrap() {
        TickOutcome::Executed(report) => report,
        TickOutcome::Idle => panic!("应执行 upload 阶段"),
    };
    // 执行持续了 4 × 120ms 的测试时钟推进（> 400ms 租约），仍然成功：
    // 说明续约在持续生效（否则 guard 会拒绝推进，report.status 会是 None）。
    assert_eq!(
        report.status,
        Some(JobStatus::Succeeded),
        "续约让租约始终有效"
    );
    let samples = samples.lock().await.clone();
    assert_eq!(samples.len(), 4);
    for (remaining_millis, epoch) in &samples {
        assert!(
            *remaining_millis >= 250,
            "续约后租约剩余时间应保持接近租约时长，实际 {remaining_millis}ms（采样：{samples:?}）"
        );
        assert_eq!(*epoch, 1, "续约不改变 leaseEpoch");
    }
    let stage = read_stage(&pool, &upload.id).await;
    assert_eq!(stage.lease_epoch, 1, "epoch 不被续约改变");
    assert!(stage.lease_owner.is_none(), "推进后释放租约");

    // 未注册处理器：延后而不假成功、不消耗重试额度。
    let validate = insert_stage(
        &pool,
        &job_id,
        StageKind::ModelValidate,
        0,
        JobStatus::Queued,
    )
    .await;
    let executor2 = test_executor(&pool, StageRegistry::new(), clock.clone(), test_config());
    let report = match executor2.tick().await.unwrap() {
        TickOutcome::Executed(report) => report,
        TickOutcome::Idle => panic!("应领取 validate 阶段"),
    };
    assert_eq!(
        report.status,
        Some(JobStatus::Queued),
        "延后，不是成功也不是失败"
    );
    let stage = read_stage(&pool, &validate.id).await;
    assert_eq!(stage.status, JobStatus::Queued);
    assert_eq!(stage.attempt_count, 0, "不消耗安全重试额度");
    assert!(stage.next_run_at.is_some());
    assert!(
        stage
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("未注册"),
        "{:?}",
        stage.last_error
    );
    assert_eq!(
        latest_attempt(&pool, &validate.id).await.map(|a| a.id),
        None,
        "不产生任何 attempt（不假称已提交）"
    );
}

// ---------------------------------------------------------------------------
// 断点注入（≥3 个）后的重启核对：任务数 / 远端请求数 / attempt / 资产引用
// ---------------------------------------------------------------------------

#[tokio::test]
async fn failpoint_breakpoints_recover_without_duplicate_remote_requests() {
    // 断点 1：intent 已落库、尚未标记 submitting（"付费 POST 发出前"）。
    {
        let dir = TestDir::new("t10-fp-intent");
        let database = open_database(&dir).await;
        let pool = database.pool().clone();
        let (job_id, _item_id) = seed_job(&pool, "fp-intent").await;
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
        let submit =
            insert_stage(&pool, &job_id, StageKind::TripoSubmit, 0, JobStatus::Queued).await;
        let fixture = FixtureServer::start(submit_success_scenario("task-fp-intent"));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = StageRegistry::new();
        registry.register(
            StageKind::TripoSubmit,
            FixtureTripoSubmit::new(&fixture.base_url(), Duration::from_secs(5), calls.clone()),
        );
        let executor = test_executor(
            &pool,
            registry,
            Arc::new(ManualClock::new(Timestamp::now())),
            test_config(),
        );
        failpoints::set(
            executor.owner(),
            PAID_AFTER_INTENT_BEFORE_SUBMITTING,
            FailpointAction::Panic,
        );
        let crashed = executor.clone();
        let joined = tokio::spawn(async move { crashed.tick().await }).await;
        assert!(joined.is_err(), "断点应使本次执行崩溃：{joined:?}");
        failpoints::clear_owner(executor.owner());
        assert_eq!(
            fixture.call_count("POST", TRIPO_SUBMIT_PATH),
            0,
            "崩溃在发请求前"
        );
        let attempt = latest_attempt(&pool, &submit.id).await.expect("intent");
        assert_eq!(attempt.submit_state, SubmitState::Intent);

        // 恢复：intent 未标记 submitting → 可安全重领；重领后只发一次请求。
        expire_lease(&pool, &submit.id).await;
        let recovery = executor.recover_expired_leases().await.unwrap();
        assert_eq!(recovery.requeued, 1);
        assert_eq!(stage_status(&pool, &submit.id).await, JobStatus::Queued);
        assert_eq!(fixture.call_count("POST", TRIPO_SUBMIT_PATH), 0);
        let _ = executor.tick().await.unwrap();
        assert_eq!(stage_status(&pool, &submit.id).await, JobStatus::Succeeded);
        assert_eq!(fixture.call_count("POST", TRIPO_SUBMIT_PATH), 1);
        let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
        assert_eq!(attempt.remote_task_id.as_deref(), Some("task-fp-intent"));
    }

    // 断点 2：已标记 submitting、请求未发出（恢复按 unknown，不重发）。
    {
        let dir = TestDir::new("t10-fp-submitting");
        let database = open_database(&dir).await;
        let pool = database.pool().clone();
        let (job_id, _item_id) = seed_job(&pool, "fp-submitting").await;
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
        let submit =
            insert_stage(&pool, &job_id, StageKind::TripoSubmit, 0, JobStatus::Queued).await;
        let fixture = FixtureServer::start(submit_success_scenario("task-fp-submitting"));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = StageRegistry::new();
        registry.register(
            StageKind::TripoSubmit,
            FixtureTripoSubmit::new(&fixture.base_url(), Duration::from_secs(5), calls.clone()),
        );
        let executor = test_executor(
            &pool,
            registry,
            Arc::new(ManualClock::new(Timestamp::now())),
            test_config(),
        );
        failpoints::set(
            executor.owner(),
            PAID_AFTER_SUBMITTING_BEFORE_REQUEST,
            FailpointAction::Panic,
        );
        let crashed = executor.clone();
        let joined = tokio::spawn(async move { crashed.tick().await }).await;
        assert!(joined.is_err(), "断点应使本次执行崩溃：{joined:?}");
        failpoints::clear_owner(executor.owner());
        assert_eq!(fixture.call_count("POST", TRIPO_SUBMIT_PATH), 0);
        expire_lease(&pool, &submit.id).await;
        let recovery = executor.recover_expired_leases().await.unwrap();
        assert_eq!(recovery.submission_unknown, 1);
        assert_eq!(
            stage_status(&pool, &submit.id).await,
            JobStatus::SubmissionUnknown
        );
        assert_eq!(fixture.call_count("POST", TRIPO_SUBMIT_PATH), 0, "不重发");
        let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
        assert_eq!(attempt.submit_state, SubmitState::Unknown);
    }

    // 断点 3：task ID 已到达但状态未推进（重启后按已知 ID 继续，不重复购买）。
    {
        let dir = TestDir::new("t10-fp-receipt");
        let database = open_database(&dir).await;
        let pool = database.pool().clone();
        let (job_id, _item_id) = seed_job(&pool, "fp-receipt").await;
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
        let submit =
            insert_stage(&pool, &job_id, StageKind::TripoSubmit, 0, JobStatus::Queued).await;
        let poll = insert_stage(&pool, &job_id, StageKind::TripoPoll, 0, JobStatus::Queued).await;
        let fixture = FixtureServer::start(submit_success_scenario("task-fp-receipt"));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = StageRegistry::new();
        registry.register(
            StageKind::TripoSubmit,
            FixtureTripoSubmit::new(&fixture.base_url(), Duration::from_secs(5), calls.clone()),
        );
        let executor = test_executor(
            &pool,
            registry,
            Arc::new(ManualClock::new(Timestamp::now())),
            test_config(),
        );
        failpoints::set(
            executor.owner(),
            PAID_AFTER_RECEIPT_BEFORE_ADVANCE,
            FailpointAction::Panic,
        );
        let crashed = executor.clone();
        let joined = tokio::spawn(async move { crashed.tick().await }).await;
        assert!(joined.is_err(), "断点应使本次执行崩溃：{joined:?}");
        failpoints::clear_owner(executor.owner());
        assert_eq!(fixture.call_count("POST", TRIPO_SUBMIT_PATH), 1);
        let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
        assert_eq!(attempt.submit_state, SubmitState::Accepted);
        assert_eq!(attempt.remote_task_id.as_deref(), Some("task-fp-receipt"));
        assert_eq!(
            stage_status(&pool, &poll.id).await,
            JobStatus::Queued,
            "未解锁后续"
        );

        expire_lease(&pool, &submit.id).await;
        // 重启后恢复：已知 ID → 补推进（不再有付费 POST）。
        let mut resume_registry = StageRegistry::new();
        resume_registry.register(
            StageKind::TripoPoll,
            FixtureTripoPoll {
                base_url: fixture.base_url(),
                read_timeout: Duration::from_secs(5),
                calls: Arc::new(AtomicUsize::new(0)),
            },
        );
        let resume = test_executor(
            &pool,
            resume_registry,
            Arc::new(ManualClock::new(Timestamp::now())),
            test_config(),
        );
        let recovery = resume.recover_expired_leases().await.unwrap();
        assert_eq!(recovery.succeeded, 1);
        assert_eq!(stage_status(&pool, &submit.id).await, JobStatus::Succeeded);
        assert_eq!(
            fixture.call_count("POST", TRIPO_SUBMIT_PATH),
            1,
            "不重复购买"
        );
        // 下游继续查询（GET /v3/tasks/task-fp-receipt），不重新提交。
        let outcome = resume.tick().await.unwrap();
        assert!(
            matches!(outcome, TickOutcome::Executed(_)),
            "poll 阶段应被领取并执行：{outcome:?}；fixture 记录：{:?}",
            fixture.recorded_summary()
        );
        assert_eq!(stage_status(&pool, &poll.id).await, JobStatus::Succeeded);
        assert_eq!(
            fixture.call_count("GET", &format!("{TRIPO_TASKS_PREFIX}task-fp-receipt")),
            1,
            "按已知远端 ID 继续查询"
        );
        assert_eq!(fixture.call_count("POST", TRIPO_SUBMIT_PATH), 1);
    }
}

// ---------------------------------------------------------------------------
// SIGKILL（真实子进程 + kill -9）
// ---------------------------------------------------------------------------

const CHILD_ENV_ROLE: &str = "EM_T10_CHILD_ROLE";
const CHILD_ENV_DATA_DIR: &str = "EM_T10_CHILD_DATA_DIR";
const CHILD_ENV_FIXTURE: &str = "EM_T10_CHILD_FIXTURE";
const CHILD_ENV_LEASE_MS: &str = "EM_T10_CHILD_LEASE_MS";
const CHILD_ENV_RENEW_MS: &str = "EM_T10_CHILD_RENEW_MS";

/// 子进程入口：**正常运行（无环境变量）时是空操作**。
///
/// 父进程用 `current_exe --exact child_worker_entry` 重新执行本测试二进制，
/// 让真实执行器在真实进程里跑，再用 `kill -9` 制造硬崩溃（不是优雅退出）。
#[test]
fn child_worker_entry() {
    let Ok(role) = std::env::var(CHILD_ENV_ROLE) else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()
        .expect("子进程运行时");
    runtime.block_on(async move { run_child_worker(&role).await });
}

async fn run_child_worker(role: &str) -> ! {
    let data_dir = PathBuf::from(std::env::var(CHILD_ENV_DATA_DIR).expect("子进程 data-dir"));
    let fixture_base = std::env::var(CHILD_ENV_FIXTURE).expect("子进程 fixture 地址");
    let lease_ms: u64 = std::env::var(CHILD_ENV_LEASE_MS)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000);
    let renew_ms: u64 = std::env::var(CHILD_ENV_RENEW_MS)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200);

    let database = Database::open_and_migrate(&data_dir)
        .await
        .expect("子进程打开数据库");
    let mut registry = StageRegistry::new();
    registry.register(
        StageKind::TripoSubmit,
        FixtureTripoSubmit::new(
            &fixture_base,
            Duration::from_secs(120),
            Arc::new(AtomicUsize::new(0)),
        ),
    );
    registry.register(
        StageKind::TripoPoll,
        FixtureTripoPoll {
            base_url: fixture_base,
            read_timeout: Duration::from_secs(120),
            calls: Arc::new(AtomicUsize::new(0)),
        },
    );
    let config = ExecutorConfig {
        lease: Duration::from_millis(lease_ms),
        renew: Duration::from_millis(renew_ms),
        ..test_config()
    };
    let executor = JobExecutor::new(database.pool().clone(), config, registry);
    let handle = executor.start();

    match role {
        // 一次性恢复 + 若干轮调度，然后正常退出（父进程据此断言恢复结果）。
        "resume-once" => {
            let _ = executor.recover_expired_leases().await;
            for _ in 0..8 {
                let _ = executor.tick().await;
            }
            handle.shutdown().await;
            std::process::exit(0);
        }
        // 持续运行，等父进程 kill -9（真实硬崩溃）。
        "run" => {
            std::future::pending::<()>().await;
            unreachable!()
        }
        other => panic!("未知子进程角色：{other}"),
    }
}

/// 子进程 stderr 落盘（`<data-dir>/child-worker.log`）：失败可诊断，不吞掉原因。
fn child_log(data_dir: &Path) -> Stdio {
    match std::fs::File::create(data_dir.join("child-worker.log")) {
        Ok(file) => Stdio::from(file),
        Err(_) => Stdio::null(),
    }
}

/// 子进程守护：无论断言是否失败都会 kill -9 并回收。
struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn spawn(
        role: &str,
        data_dir: &Path,
        fixture_base: &str,
        lease_ms: u64,
        renew_ms: u64,
    ) -> Self {
        let child = Command::new(std::env::current_exe().expect("测试二进制路径"))
            .args(["--exact", "child_worker_entry", "--nocapture"])
            .env(CHILD_ENV_ROLE, role)
            .env(CHILD_ENV_DATA_DIR, data_dir)
            .env(CHILD_ENV_FIXTURE, fixture_base)
            .env(CHILD_ENV_LEASE_MS, lease_ms.to_string())
            .env(CHILD_ENV_RENEW_MS, renew_ms.to_string())
            .env_remove(failpoints::ENV_FAILPOINT)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("启动子进程");
        Self { child: Some(child) }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().expect("子进程存活").id()
    }

    /// `kill -9`（SIGKILL）：不做任何清理，等价于断电式崩溃。
    fn kill9(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let pid = child.id();
        let status = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .expect("执行 kill -9");
        assert!(status.success(), "kill -9 失败");
        let _ = child.wait();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// 阻塞等待某个条件成立（子进程请求到达 fixture 等）。
async fn wait_for(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("等待条件超时（{timeout:?}）");
}

/// 子进程运行到自然结束（`resume-once` 角色）；阻塞命令放到阻塞线程池，避免占住运行时线程。
async fn run_child_once(
    role: &str,
    data_dir: &Path,
    fixture_base: &str,
    lease_ms: u64,
    renew_ms: u64,
) -> bool {
    let role = role.to_owned();
    let data_dir = data_dir.to_path_buf();
    let fixture_base = fixture_base.to_owned();
    tokio::task::spawn_blocking(move || {
        run_child_once_blocking(&role, &data_dir, &fixture_base, lease_ms, renew_ms)
    })
    .await
    .expect("子进程任务")
}

fn run_child_once_blocking(
    role: &str,
    data_dir: &Path,
    fixture_base: &str,
    lease_ms: u64,
    renew_ms: u64,
) -> bool {
    let child = Command::new(std::env::current_exe().expect("测试二进制路径"))
        .args(["--exact", "child_worker_entry", "--nocapture"])
        .env(CHILD_ENV_ROLE, role)
        .env(CHILD_ENV_DATA_DIR, data_dir)
        .env(CHILD_ENV_FIXTURE, fixture_base)
        .env(CHILD_ENV_LEASE_MS, lease_ms.to_string())
        .env(CHILD_ENV_RENEW_MS, renew_ms.to_string())
        .env_remove(failpoints::ENV_FAILPOINT)
        .stdout(Stdio::null())
        .stderr(child_log(data_dir))
        .output()
        .expect("运行子进程");
    child.status.success()
}

fn hanging_submit_scenario() -> Scenario {
    Scenario::new(vec![RouteScript {
        method: "POST".to_owned(),
        path: TRIPO_SUBMIT_PATH.to_owned(),
        path_match: Default::default(),
        repeat_last: true,
        // 读完请求后不写任何字节：子进程停在"付费 POST 已发出、响应未到"。
        steps: vec![Step::Timeout { hold_ms: 120_000 }],
    }])
}

fn hanging_task_query_scenario(task_id: &str) -> Scenario {
    Scenario::new(vec![RouteScript {
        method: "GET".to_owned(),
        path: format!("{TRIPO_TASKS_PREFIX}{task_id}"),
        path_match: Default::default(),
        repeat_last: true,
        steps: vec![Step::Timeout { hold_ms: 120_000 }],
    }])
}

fn running_task_query_scenario(task_id: &str) -> Scenario {
    Scenario::new(vec![RouteScript {
        method: "GET".to_owned(),
        path: format!("{TRIPO_TASKS_PREFIX}{task_id}"),
        path_match: Default::default(),
        repeat_last: true,
        steps: vec![Step::Respond {
            response: ResponseSpec {
                status: 200,
                headers: Default::default(),
                body: BodySpec::Json {
                    json: json!({ "code": 0, "data": { "status": "success", "progress": 100 } }),
                },
            },
        }],
    }])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sigkill_during_paid_post_makes_submission_unknown_without_repurchasing() {
    let dir = TestDir::new("t10-sigkill-submit");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "sigkill-submit").await;
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

    let fixture = FixtureServer::start(hanging_submit_scenario());
    let mut child = ChildGuard::spawn("run", dir.path(), &fixture.base_url(), 1_000, 200);
    // 请求已被 fixture 记录（POST 已发出），子进程正等响应。
    wait_for(Duration::from_secs(20), || {
        fixture.call_count("POST", TRIPO_SUBMIT_PATH) == 1
    })
    .await;
    assert_eq!(fixture.request_total(), 1);
    let killed_pid = child.pid();
    child.kill9();
    assert!(killed_pid > 0);

    // 等租约过期（子进程租约 1s；真实时间流逝）。
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let post_calls_before = fixture.call_count("POST", TRIPO_SUBMIT_PATH);

    // 重启：新进程做恢复扫描（不改 fixture，若重发会命中同一脚本 → 调用数变化）。
    assert!(
        run_child_once("resume-once", dir.path(), &fixture.base_url(), 1_000, 200).await,
        "恢复子进程应正常退出"
    );

    // 用独立连接核对落库结果（子进程已退出，数据在同一个 data-dir）。
    let database = Database::open_and_migrate(dir.path()).await.unwrap();
    let pool = database.pool().clone();
    assert_eq!(
        stage_status(&pool, &submit.id).await,
        JobStatus::SubmissionUnknown,
        "付费 POST 结果未知 → submission_unknown"
    );
    let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
    assert_eq!(attempt.submit_state, SubmitState::Unknown);
    assert_eq!(attempt.remote_task_id, None);
    assert_eq!(attempt.response_id, None);
    assert_eq!(
        stage_status(&pool, &poll.id).await,
        JobStatus::Queued,
        "该分支后续购买暂停（后续阶段不解锁）"
    );
    assert_eq!(
        read_job(&pool, &job_id).await.status,
        JobStatus::SubmissionUnknown
    );
    assert_eq!(
        fixture.call_count("POST", TRIPO_SUBMIT_PATH),
        post_calls_before,
        "重启后绝不重发付费 POST"
    );
    assert_eq!(fixture.call_count("POST", TRIPO_SUBMIT_PATH), 1);
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(jobs, 1, "重启不产生第二个 job");
    drop(database);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sigkill_during_poll_resumes_with_known_remote_task_and_never_resubmits() {
    let dir = TestDir::new("t10-sigkill-poll");
    let database = open_database(&dir).await;
    let pool = database.pool().clone();
    let (job_id, _item_id) = seed_job(&pool, "sigkill-poll").await;
    insert_stage(
        &pool,
        &job_id,
        StageKind::FreezeInputs,
        0,
        JobStatus::Succeeded,
    )
    .await;
    let upload = insert_stage(
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
    assert_eq!(upload.status, JobStatus::Succeeded);
    assert_eq!(submit.status, JobStatus::Succeeded);
    // 已知远端 ID（上一次运行已提交成功）。
    seed_accepted_attempt(&pool, &job_id, &submit.id, "task-known-1", Timestamp::now()).await;
    drop(database);

    // 第一阶段：查询被"供应商未响应"挂住，父进程 kill -9。
    let hanging = FixtureServer::start(hanging_task_query_scenario("task-known-1"));
    let mut child = ChildGuard::spawn("run", dir.path(), &hanging.base_url(), 1_000, 200);
    wait_for(Duration::from_secs(20), || {
        hanging.call_count("GET", &format!("{TRIPO_TASKS_PREFIX}task-known-1")) == 1
    })
    .await;
    child.kill9();
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    // 第二阶段：重启后供应商可用 → 必须用**同一个** task ID 继续查询，且从不重新提交。
    let healthy = FixtureServer::start(running_task_query_scenario("task-known-1"));
    assert!(
        run_child_once("resume-once", dir.path(), &healthy.base_url(), 1_000, 200).await,
        "恢复子进程应正常退出"
    );

    let database = Database::open_and_migrate(dir.path()).await.unwrap();
    let pool = database.pool().clone();
    assert!(
        healthy.call_count("GET", &format!("{TRIPO_TASKS_PREFIX}task-known-1")) >= 1,
        "恢复后按已知远端 ID 继续查询（fixture 记录：{:?}）",
        healthy.recorded_summary()
    );
    assert_eq!(
        healthy.call_count("POST", TRIPO_SUBMIT_PATH),
        0,
        "恢复过程绝不重新提交付费请求"
    );
    assert_eq!(
        hanging.call_count("POST", TRIPO_SUBMIT_PATH),
        0,
        "第一阶段也没有付费 POST"
    );
    let attempt = latest_attempt(&pool, &submit.id).await.expect("attempt");
    assert_eq!(attempt.remote_task_id.as_deref(), Some("task-known-1"));
    assert_eq!(attempt.submit_state, SubmitState::Accepted);
    assert_eq!(
        stage_status(&pool, &poll.id).await,
        JobStatus::Succeeded,
        "查询成功后阶段完成"
    );
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(jobs, 1, "重启不产生第二个 job");
    drop(database);
}

// ---------------------------------------------------------------------------
// 附加：failpoint 门控与配置校验
// ---------------------------------------------------------------------------

#[test]
fn failpoints_are_available_only_in_test_builds() {
    // 本测试二进制由 [dev-dependencies] 自引用开启了 feature；生产构建没有该分支。
    failpoints::set(
        "test-owner",
        PAID_AFTER_RECEIPT_BEFORE_ADVANCE,
        FailpointAction::Hang(0),
    );
    failpoints::hit("test-owner", PAID_AFTER_RECEIPT_BEFORE_ADVANCE); // 命中 Hang(0)：立即返回
    failpoints::hit("other-owner", PAID_AFTER_RECEIPT_BEFORE_ADVANCE); // 其他 owner 不命中
    failpoints::clear_owner("test-owner");
    for name in [
        PAID_AFTER_INTENT_BEFORE_SUBMITTING,
        PAID_AFTER_SUBMITTING_BEFORE_REQUEST,
        PAID_AFTER_RESPONSE_BEFORE_RECEIPT,
        PAID_AFTER_RECEIPT_BEFORE_ADVANCE,
        MANUAL_AFTER_REQUEST_BEFORE_RESPONSE,
        RESULT_FACT_BEFORE_CHECKPOINT,
    ] {
        assert!(!name.is_empty());
    }
}

#[test]
fn executor_config_defaults_and_validation() {
    let config = ExecutorConfig::default();
    assert_eq!(config.lease, Duration::from_secs(120));
    assert_eq!(config.renew, Duration::from_secs(20));
    assert_eq!(config.remote_generation_limit, 2);
    assert_eq!(config.manual_ai_batch_limit, 2);
    config.validate().expect("默认配置合法");

    let bad = ExecutorConfig {
        renew: Duration::from_secs(120),
        ..config.clone()
    };
    assert!(bad.validate().is_err(), "续约间隔必须小于租约（此处相等）");

    let too_many = ExecutorConfig {
        remote_generation_limit: 3,
        ..config.clone()
    };
    assert!(too_many.validate().is_err(), "并发上限不得提高");

    let zero_lease = ExecutorConfig {
        lease: Duration::ZERO,
        ..config
    };
    assert!(zero_lease.validate().is_err());
}
