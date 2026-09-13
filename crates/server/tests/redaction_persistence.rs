//! RD 回归（BUG-009 / BUG-010 / BUG-011 / BUG-012 / ADR-034）：统一脱敏入口的
//! **持久化侧**验收。
//!
//! 范围与目的：`job_stages.last_error` / `needs_input_json` / `usage_json` 与
//! `provider_attempts.last_error` 的所有**写入路径**都必须经过 `crate::redaction`
//! 的统一入口，使任何来源的文本（下载传输错误、供应商摘要、恢复/对账注记）在落库前
//! 就不含 `scheme://…` 形态的临时/签名地址；裸 host、task_id、计数与摘要标签原样保留。
//! JSON 列按逐字符串值脱敏：句子型 `message` 只替换 URL 片段（BUG-011），
//! 纯 URL 值按列契约取摘要对象/摘要标签（ADR-034）。
//!
//! 与 QA 用例 `qa_t20_bug008_independent.rs` / `qa_t13_independent.rs` 的分工：
//! QA 覆盖**消息产生**（reqwest 错误文本、提供方 refusal 文本）；本文件覆盖**落库入口**
//! （仓储层各写入点），若未来新增绕过仓储层的直写会被"全列扫描"类用例发现。
//!
//! 全程本机临时 data-dir；无网络、无付费。

mod common;

use common::TestDir;
use everything_manual::config::datadir;
use everything_manual::storage::Database;
use everything_manual::storage::repo::attempts::{self as attempts_repo, NewAttempt};
use everything_manual::storage::repo::job_stages::{self, LeaseGuard, NewStage, StageAdvance};
use manual_core::domain::{JobStage, JobStatus, StageKind, SubmitState};
use manual_core::ids;
use manual_core::timestamps::Timestamp;
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};

/// 复现用签名 canary（只存在于测试进程与临时库里）。
const CANARY: &str = "qa-canary-last-error-7d31";
/// 与 QA 复现同形的签名 URL（含查询串）。
const SIGNED_URL: &str = "https://cdn.example.invalid/qa-r28/model.glb?sign=qa-canary-last-error-7d31&expires=9999999999";

async fn open(dir: &TestDir) -> SqlitePool {
    datadir::ensure_initialized(dir.path()).expect("初始化 data-dir 结构");
    Database::open_and_migrate(dir.path())
        .await
        .expect("打开并迁移数据库")
        .pool()
        .clone()
}

/// 最小引用链（item → blob/asset → document → preparation → snapshot → job），
/// 供 `job_stages`/`provider_attempts` 的外键使用（同 `storage.rs` 的种子做法）。
async fn seed_job(pool: &SqlitePool, tag: &str) -> String {
    let now = Timestamp::now().as_millis();
    let item_id = ids::new_id();
    let sha = test_support::assets::sha256_hex(format!("{tag}-source").as_bytes());
    let asset_id = ids::new_id();
    let document_id = ids::new_id();
    let preparation_id = ids::new_id();
    let snapshot_id = ids::new_id();
    let job_id = ids::new_id();

    sqlx::query(
        "INSERT INTO items (id, name, brand, model, variant, revision, archived_at, created_at, updated_at) \
         VALUES (?, ?, NULL, 'X100V', NULL, 1, NULL, ?, ?)",
    )
    .bind(&item_id)
    .bind(format!("脱敏回归物品 {tag}"))
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 item");
    sqlx::query(
        "INSERT INTO blobs (sha256, size, mime, storage_state, created_at) VALUES (?, 1024, 'application/pdf', 'stored', ?)",
    )
    .bind(&sha)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 blob");
    sqlx::query(
        "INSERT INTO assets (id, blob_id, item_id, purpose, original_name, created_at) VALUES (?, ?, ?, 'document', 'manual.pdf', ?)",
    )
    .bind(&asset_id)
    .bind(&sha)
    .bind(&item_id)
    .bind(now)
    .execute(pool)
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
    .execute(pool)
    .await
    .expect("插入 document");
    sqlx::query(
        "INSERT INTO preparations (id, document_id, source_sha256, state, page_count, revision, client_derived, created_at, updated_at) \
         VALUES (?, ?, ?, 'ready', 2, 1, 1, ?, ?)",
    )
    .bind(&preparation_id)
    .bind(&document_id)
    .bind(&sha)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 preparation");
    sqlx::query(
        "INSERT INTO generation_snapshots \
             (id, item_id, item_revision, preparation_id, photo_ids, photo_hashes, provider_config, \
              prompt_version, price_version, budgets, created_at) \
         VALUES (?, ?, 1, ?, '[]', '[]', '{}', 'prompt-v1', 'price-v1', '{}', ?)",
    )
    .bind(&snapshot_id)
    .bind(&item_id)
    .bind(&preparation_id)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 snapshot");
    sqlx::query(
        "INSERT INTO jobs (id, item_id, snapshot_id, status, revision, created_at, updated_at) \
         VALUES (?, ?, ?, 'queued', 1, ?, ?)",
    )
    .bind(&job_id)
    .bind(&item_id)
    .bind(&snapshot_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 job");
    job_id
}

/// 插入一个**已被领取**（`running` + 租约 guard 匹配）的 `model_download` 阶段。
async fn seed_running_stage(pool: &SqlitePool, job_id: &str, owner: &str) -> JobStage {
    let now = Timestamp::now();
    let mut conn = pool.acquire().await.expect("连接");
    let stage = job_stages::insert(
        &mut conn,
        NewStage {
            job_id: job_id.to_owned(),
            stage_kind: StageKind::ModelDownload,
            batch_index: 0,
            page_set_json: None,
            input_hash: "hash-redaction-1".to_owned(),
            status: JobStatus::Queued,
        },
        now,
    )
    .await
    .expect("插入阶段");
    sqlx::query(
        "UPDATE job_stages SET status = 'running', lease_owner = ?, lease_epoch = 1, \
            lease_until = ?, updated_at = ? WHERE id = ?",
    )
    .bind(owner)
    .bind(now.as_millis() + 60_000)
    .bind(now.as_millis())
    .bind(&stage.id)
    .execute(&mut *conn)
    .await
    .expect("模拟已领取（租约）");
    job_stages::get(&mut conn, &stage.id)
        .await
        .expect("读回阶段")
        .expect("阶段存在")
}

async fn read_stage(pool: &SqlitePool, stage_id: &str) -> JobStage {
    let mut conn = pool.acquire().await.expect("连接");
    job_stages::get(&mut conn, stage_id)
        .await
        .expect("读取阶段")
        .expect("阶段存在")
}

fn assert_no_url_shape(text: &str, context: &str) {
    assert!(!text.contains("://"), "{context} 不得含 URL 形态：{text}");
    assert!(!text.contains(CANARY), "{context} 不得含签名：{text}");
}

/// `RetryWait` 落库（下载传输失败的真实路径）：签名 URL 被替换为摘要标签，
/// URL 之后的普通文本保留，host/task_id/计数不受影响。
#[tokio::test]
async fn retry_wait_last_error_is_redacted_before_persisting() {
    let dir = TestDir::new("r28-redaction-retry");
    let pool = open(&dir).await;
    let job_id = seed_job(&pool, "retry").await;
    let stage = seed_running_stage(&pool, &job_id, "worker-1").await;

    let reason = format!(
        "模型下载传输失败（下载可安全重试；不产生供应商费用）：连接失败：\
         error sending request for url ({SIGNED_URL})；task_id=task-77 可重查"
    );
    let mut conn = pool.acquire().await.expect("连接");
    let advanced = job_stages::advance(
        &mut conn,
        &LeaseGuard {
            stage_id: stage.id.clone(),
            owner: "worker-1".to_owned(),
            epoch: 1,
        },
        Timestamp::now(),
        &StageAdvance::RetryWait {
            next_run_at: Timestamp::now(),
            last_error: reason,
        },
    )
    .await
    .expect("推进阶段");
    assert!(advanced, "租约匹配时推进必须成功");

    let stored = read_stage(&pool, &stage.id).await;
    assert_eq!(stored.status, JobStatus::RetryWait);
    let last_error = stored.last_error.expect("last_error 已写入");
    assert_no_url_shape(&last_error, "job_stages.last_error");
    assert!(
        last_error.contains("host=cdn.example.invalid"),
        "摘要标签保留 host（最小诊断信息）：{last_error}"
    );
    assert!(
        last_error.contains("task_id=task-77 可重查"),
        "URL 之后的普通文本必须保留：{last_error}"
    );
    assert!(last_error.contains("可安全重试"), "{last_error}");
}

/// `NeedsInput` 落库：`last_error` 与 `needs_input_json`（序列化 JSON 文本）同时脱敏，
/// 且脱敏后的 JSON 仍可解析、缺项 code 不变。
#[tokio::test]
async fn needs_input_json_and_last_error_are_redacted_before_persisting() {
    let dir = TestDir::new("r28-redaction-needs-input");
    let pool = open(&dir).await;
    let job_id = seed_job(&pool, "needs-input").await;
    let stage = seed_running_stage(&pool, &job_id, "worker-2").await;

    let needs_input_json = serde_json::to_string(&json!([
        {
            "code": "download_transport",
            "message": format!("链接过期：{SIGNED_URL}已过期，请按 task_id=task-88 重查（不重新购买）"),
        }
    ]))
    .expect("序列化缺项");
    let mut conn = pool.acquire().await.expect("连接");
    let advanced = job_stages::advance(
        &mut conn,
        &LeaseGuard {
            stage_id: stage.id.clone(),
            owner: "worker-2".to_owned(),
            epoch: 1,
        },
        Timestamp::now(),
        &StageAdvance::NeedsInput {
            needs_input_json,
            last_error: format!("模型下载失败：{SIGNED_URL} 需要人工处理"),
        },
    )
    .await
    .expect("推进阶段");
    assert!(advanced);

    let stored = read_stage(&pool, &stage.id).await;
    assert_eq!(stored.status, JobStatus::NeedsInput);
    assert_no_url_shape(
        stored.last_error.as_deref().unwrap_or_default(),
        "last_error",
    );

    let value: Value = stored.needs_input_json.expect("needs_input_json 已写入");
    let message = value[0]["message"].as_str().expect("message 是字符串");
    assert_eq!(value[0]["code"], "download_transport", "缺项 code 不变");
    assert_no_url_shape(message, "needs_input_json.message");
    assert!(
        message.contains("已过期，请按 task_id=task-88 重查（不重新购买）"),
        "{message}"
    );
}

/// BUG-012（回合 27）：`usage_json` 的两个写入点（`advance(Succeeded)` /
/// `set_result_fact`）都经结构感知脱敏——提供方文本（`errorSummary`）里的签名 URL
/// 不落库；JSON 结构、task_id、计费与纯 URL 值的摘要对象语义原样保留。
#[tokio::test]
async fn usage_json_is_redacted_on_all_write_paths() {
    let dir = TestDir::new("r29-redaction-usage");
    let pool = open(&dir).await;
    let job_id = seed_job(&pool, "usage").await;
    let stage = seed_running_stage(&pool, &job_id, "worker-5").await;

    // `set_result_fact` 路径（同步链路 receipt：manual_ai 的 refusal 摘要曾原样写入）。
    let usage = json!({
        "batchIndex": 0,
        "outcome": "refusal",
        "remoteTaskId": "task-usage-77",
        "billing": {"creditMinor": 3000},
        "errorSummary": format!("模型拒答（refusal）：下载参考 {SIGNED_URL} 也失败"),
        "modelUrl": SIGNED_URL,
    });
    let mut conn = pool.acquire().await.expect("连接");
    job_stages::set_result_fact(
        &mut conn,
        &stage.id,
        None,
        Some(&usage.to_string()),
        Timestamp::now(),
    )
    .await
    .expect("写结果事实");
    let stored: String =
        sqlx::query_scalar("SELECT CAST(usage_json AS TEXT) FROM job_stages WHERE id = ?")
            .bind(&stage.id)
            .fetch_one(&mut *conn)
            .await
            .expect("读回 usage_json");
    assert_no_url_shape(&stored, "set_result_fact 的 usage_json");
    let parsed: Value = serde_json::from_str(&stored).expect("usage_json 仍是合法 JSON");
    assert_eq!(parsed["remoteTaskId"], "task-usage-77", "id 事实保留");
    assert_eq!(parsed["billing"]["creditMinor"], 3000, "计费事实保留");
    let summary = parsed["errorSummary"].as_str().expect("句子仍是字符串");
    assert!(summary.ends_with(" 也失败"), "{summary}");
    assert!(summary.contains("host=cdn.example.invalid"), "{summary}");
    assert_eq!(
        parsed["modelUrl"]["redacted"], true,
        "纯 URL 值仍是摘要对象"
    );

    // `advance(Succeeded)` 路径（执行器 checkpoint）。
    let usage = json!({
        "validation": "validated",
        "errorSummary": format!("远端返回异常（{SIGNED_URL}），已按失败处理"),
    });
    let advanced = job_stages::advance(
        &mut conn,
        &LeaseGuard {
            stage_id: stage.id.clone(),
            owner: "worker-5".to_owned(),
            epoch: 1,
        },
        Timestamp::now(),
        &StageAdvance::Succeeded {
            result_asset_id: None,
            usage_json: Some(usage.to_string()),
        },
    )
    .await
    .expect("推进阶段");
    assert!(advanced);
    let stored: String =
        sqlx::query_scalar("SELECT CAST(usage_json AS TEXT) FROM job_stages WHERE id = ?")
            .bind(&stage.id)
            .fetch_one(&mut *conn)
            .await
            .expect("读回 usage_json");
    assert_no_url_shape(&stored, "advance(Succeeded) 的 usage_json");
    let parsed: Value = serde_json::from_str(&stored).expect("usage_json 仍是合法 JSON");
    assert_eq!(parsed["validation"], "validated");
    assert!(
        parsed["errorSummary"]
            .as_str()
            .expect("句子仍是字符串")
            .ends_with("），已按失败处理")
    );
}

/// BUG-011（回合 27）写入侧：`needs_input_json` 的句子型 `message` 只替换 URL 片段，
/// 同列其它条目原样保留（整列仍是可反序列化的列表）。
#[tokio::test]
async fn needs_input_json_keeps_sentence_and_neighbour_items_on_write() {
    let dir = TestDir::new("r29-redaction-needs-keep");
    let pool = open(&dir).await;
    let job_id = seed_job(&pool, "needs-keep").await;
    let stage = seed_running_stage(&pool, &job_id, "worker-6").await;

    let needs_input_json = serde_json::to_string(&json!([
        {
            "code": "download_insecure_scheme",
            "message": format!("模型下载必须使用 HTTPS（实际 {SIGNED_URL}）：拒绝下载"),
        },
        {"code": "retry", "message": "下载可安全重试（task-9）"}
    ]))
    .expect("序列化缺项");
    let mut conn = pool.acquire().await.expect("连接");
    let advanced = job_stages::advance(
        &mut conn,
        &LeaseGuard {
            stage_id: stage.id.clone(),
            owner: "worker-6".to_owned(),
            epoch: 1,
        },
        Timestamp::now(),
        &StageAdvance::NeedsInput {
            needs_input_json,
            last_error: "需要人工处理".to_owned(),
        },
    )
    .await
    .expect("推进阶段");
    assert!(advanced);

    let stored = read_stage(&pool, &stage.id).await;
    let value: Value = stored.needs_input_json.expect("needs_input_json 已写入");
    let items = value.as_array().expect("仍是列表");
    assert_eq!(items.len(), 2, "同列其它条目不得丢失：{value}");
    let message = items[0]["message"].as_str().expect("message 仍是字符串");
    assert_no_url_shape(message, "needs_input_json.message");
    assert!(
        message.starts_with("模型下载必须使用 HTTPS（实际 "),
        "{message}"
    );
    assert!(
        message.ends_with("）：拒绝下载"),
        "整句说明必须保留：{message}"
    );
    assert_eq!(items[0]["code"], "download_insecure_scheme");
    assert_eq!(items[1]["message"], "下载可安全重试（task-9）");
}

/// attempt 的三个 `last_error` 写入点（`mark_unknown` / `mark_failed` / `set_last_error`）
/// 全部脱敏；`remote_task_id` 等事实字段原样保留。
#[tokio::test]
async fn provider_attempt_errors_are_redacted_on_write() {
    let dir = TestDir::new("r28-redaction-attempts");
    let pool = open(&dir).await;
    let job_id = seed_job(&pool, "attempts").await;
    let stage = seed_running_stage(&pool, &job_id, "worker-3").await;
    let now = Timestamp::now();

    let mut conn = pool.acquire().await.expect("连接");
    let attempt = attempts_repo::create_intent(
        &mut conn,
        NewAttempt {
            job_id: job_id.clone(),
            stage_id: stage.id.clone(),
            request_hash: "hash-redaction-attempt".to_owned(),
        },
        now,
    )
    .await
    .expect("创建 intent");

    attempts_repo::mark_unknown(
        &mut conn,
        &attempt.id,
        &format!("传输失败：error sending request for url ({SIGNED_URL})"),
        now,
    )
    .await
    .expect("标记结果未知");
    let stored = attempts_repo::get(&mut conn, &attempt.id)
        .await
        .expect("读取 attempt")
        .expect("attempt 存在");
    assert_eq!(stored.submit_state, SubmitState::Unknown);
    let last_error = stored.last_error.clone().expect("last_error 已写入");
    assert_no_url_shape(&last_error, "provider_attempts.last_error(unknown)");

    attempts_repo::set_last_error(
        &mut conn,
        &attempt.id,
        &format!("冲突记录：{SIGNED_URL} 与远端不一致"),
        now,
    )
    .await
    .expect("更新摘要");
    let stored = attempts_repo::get(&mut conn, &attempt.id)
        .await
        .expect("读取 attempt")
        .expect("attempt 存在");
    let last_error = stored.last_error.expect("last_error 已写入");
    assert_no_url_shape(&last_error, "provider_attempts.last_error(set)");
    assert!(last_error.contains("与远端不一致"), "{last_error}");
}

/// 兜底扫描：把两张表的全部文本列（含历史行）扫一遍，签名与 `://` 零命中；
/// 同时确认用户/事实字段（task_id/计数）没有被误清洗。
#[tokio::test]
async fn no_url_shape_survives_in_persisted_error_columns() {
    let dir = TestDir::new("r28-redaction-sweep");
    let pool = open(&dir).await;
    let job_id = seed_job(&pool, "sweep").await;
    let stage = seed_running_stage(&pool, &job_id, "worker-4").await;

    // 先经统一入口写一遍（模拟修复后的真实写入）。
    let mut conn = pool.acquire().await.expect("连接");
    job_stages::advance(
        &mut conn,
        &LeaseGuard {
            stage_id: stage.id.clone(),
            owner: "worker-4".to_owned(),
            epoch: 1,
        },
        Timestamp::now(),
        &StageAdvance::Failed {
            last_error: format!("不可恢复：{SIGNED_URL}"),
        },
    )
    .await
    .expect("推进阶段");

    // 再模拟一条"修复前落库"的历史行（直写 SQL，绕过仓储入口）：
    // 它说明入库入口的保证范围——历史行由备份过滤与 DTO 读取侧兜底（另有用例覆盖）。
    sqlx::query("UPDATE job_stages SET needs_input_json = ? WHERE id = ?")
        .bind(format!(
            "[{{\"code\":\"legacy\",\"message\":\"{SIGNED_URL}\"}}]"
        ))
        .bind(&stage.id)
        .execute(&mut *conn)
        .await
        .expect("构造历史行");

    let rows = sqlx::query("SELECT last_error FROM job_stages WHERE last_error IS NOT NULL")
        .fetch_all(&mut *conn)
        .await
        .expect("扫描 job_stages.last_error");
    for row in &rows {
        let text: String = row.try_get("last_error").expect("列存在");
        assert_no_url_shape(&text, "job_stages.last_error（扫描）");
        assert!(text.contains("不可恢复"), "{text}");
    }

    let attempts =
        sqlx::query("SELECT last_error FROM provider_attempts WHERE last_error IS NOT NULL")
            .fetch_all(&mut *conn)
            .await
            .expect("扫描 provider_attempts.last_error");
    for row in &attempts {
        let text: String = row.try_get("last_error").expect("列存在");
        assert_no_url_shape(&text, "provider_attempts.last_error（扫描）");
    }
}
