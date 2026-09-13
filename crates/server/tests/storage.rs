//! T03 集成测试：SQLite 迁移、schema 兼容门禁与仓储原语（PRD 修订 1）。
//!
//! 覆盖的验收条件：
//! - AC-007：空库初始化（表/唯一键/外键/索引与连接设置）、重复迁移幂等、
//!   外键拒绝非法引用、唯一键冲突被拒、revision CAS 生效、回滚不留半状态、
//!   关闭后重开数据仍在；
//! - AC-008：旧 schema 自动迁移且数据保留；比程序更新的 schema 被拒绝打开、
//!   库文件字节未被修改、错误可读；迁移 SQL 变更触发重编译（见手工验证记录）。
//!
//! 所有用例使用独立临时 data-dir，不接触真实数据；无任何网络调用。
//! CLI 层用例通过真实二进制子进程执行（与 `config_cli.rs` 同一手法）。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use everything_manual::config::datadir;
use everything_manual::storage::repo::items::{ItemUpdate, NewItem};
use everything_manual::storage::{
    ConnectionSettings, DATABASE_FILE_NAME, Database, DatabaseStatus, POOL_MAX_CONNECTIONS,
    StorageError, database_path, repo,
};
use manual_core::domain::{
    AssetPurpose, BlobStorageState, Currency, DraftStatus, JobStatus, LedgerState,
    ModelValidationState, PhotoView, PreparationState, ProviderKey, StageKind, SubmitState,
};
use manual_core::{ids, timestamps::Timestamp};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::{Row, SqlitePool};

const BIN: &str = env!("CARGO_BIN_EXE_everything-manual");

/// 内嵌迁移的真实文件内容（测试用它构造"旧版本程序"或"坏迁移"的迁移目录；
/// 内容与生产文件逐字节一致，因此 SQLx checksum 与内嵌迁移相同）。
const MIGRATION_0001: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../migrations/0001_core_schema.sql"
));
const MIGRATION_0002: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../migrations/0002_invariants.sql"
));
const MIGRATION_0003: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../migrations/0003_photos_view_unique.sql"
));
const MIGRATION_0004: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../migrations/0004_preparation_pages.sql"
));
const MIGRATION_0005: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../migrations/0005_job_execution.sql"
));
const MIGRATION_0006: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../migrations/0006_generation_requests.sql"
));
const MIGRATION_0007: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../migrations/0007_release_manifest.sql"
));

// ---------------------------------------------------------------------------
// 基础工具
// ---------------------------------------------------------------------------

/// 自动清理的临时目录（每个用例一个，互不干扰）。
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let path =
            std::env::temp_dir().join(format!("em-storage-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).expect("创建测试临时目录");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 建出与 `init` 相同的 data-dir 结构（复用生产代码，避免测试与 init 漂移）。
fn init_structure(dir: &Path) {
    datadir::ensure_initialized(dir).expect("初始化 data-dir 结构");
}

/// 打开并迁移测试 data-dir。
async fn open(dir: &Path) -> Database {
    Database::open_and_migrate(dir)
        .await
        .expect("打开并迁移数据库")
}

/// 把 sqlx 错误映射成存储错误（用于断言约束分类）。
fn expect_error<T>(result: Result<T, sqlx::Error>) -> StorageError {
    match result {
        Ok(_) => panic!("期望约束拒绝，但语句成功了"),
        Err(error) => StorageError::from(error),
    }
}

async fn count(pool: &SqlitePool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.expect(sql)
}

async fn names(pool: &SqlitePool, sql: &'static str) -> Vec<String> {
    sqlx::query_scalar(sql).fetch_all(pool).await.expect(sql)
}

/// `PRAGMA foreign_key_list(<table>)` 引用的目标表。
///
/// 表名来自本测试文件的常量（不来自用户输入），属于显式审计过的动态 SQL。
async fn foreign_key_targets(pool: &SqlitePool, table: &'static str) -> Vec<String> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "PRAGMA foreign_key_list('{table}')"
    )))
    .fetch_all(pool)
    .await
    .expect("读取外键");
    rows.iter()
        .map(|row| row.try_get::<String, _>("table").expect("外键目标表"))
        .collect()
}

async fn create_item(pool: &SqlitePool, name: &str, model: &str) -> manual_core::domain::Item {
    let mut conn = pool.acquire().await.expect("取连接");
    repo::items::create(
        &mut conn,
        NewItem {
            name: name.to_owned(),
            brand: Some("Fuji".to_owned()),
            model: model.to_owned(),
            variant: None,
        },
    )
    .await
    .expect("创建物品")
}

/// 固定的合法 sha256（内容寻址键）。
fn sha_hex(n: u32) -> String {
    format!("{n:064x}")
}

/// 插入一个 blob，返回其 sha256。
async fn insert_blob(pool: &SqlitePool, content_id: u32) -> String {
    let sha = sha_hex(content_id);
    sqlx::query(
        "INSERT INTO blobs (sha256, size, mime, storage_state, created_at) VALUES (?, ?, ?, 'stored', ?)",
    )
    .bind(&sha)
    .bind(1024_i64)
    .bind("application/pdf")
    .bind(Timestamp::now().as_millis())
    .execute(pool)
    .await
    .expect("插入 blob");
    sha
}

/// 插入一个属于 item 的资产，返回 asset id。
async fn insert_asset(pool: &SqlitePool, item_id: &str, sha: &str, purpose: &str) -> String {
    let asset_id = ids::new_id();
    sqlx::query(
        "INSERT INTO assets (id, blob_id, item_id, purpose, original_name, created_at) \
         VALUES (?, ?, ?, ?, 'manual.pdf', ?)",
    )
    .bind(&asset_id)
    .bind(sha)
    .bind(item_id)
    .bind(purpose)
    .bind(Timestamp::now().as_millis())
    .execute(pool)
    .await
    .expect("插入资产");
    asset_id
}

/// 端到端种子链路：item → blob/asset → document → preparation → snapshot → job → stage。
struct Chain {
    item_id: String,
    document_id: String,
    preparation_id: String,
    snapshot_id: String,
    job_id: String,
    stage_id: String,
    asset_id: String,
}

async fn seed_chain(pool: &SqlitePool) -> Chain {
    let item = create_item(pool, "相机", "X100V").await;
    let sha = insert_blob(pool, 0xA1).await;
    let asset_id = insert_asset(pool, &item.id, &sha, "document").await;

    let document_id = ids::new_id();
    let now = Timestamp::now().as_millis();
    sqlx::query(
        "INSERT INTO documents (id, item_id, source_asset_id, source_sha256, title, source_url, created_at, updated_at) \
         VALUES (?, ?, ?, ?, '说明书', NULL, ?, ?)",
    )
    .bind(&document_id)
    .bind(&item.id)
    .bind(&asset_id)
    .bind(&sha)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 document");

    let preparation_id = ids::new_id();
    sqlx::query(
        "INSERT INTO preparations (id, document_id, source_sha256, state, page_count, revision, created_at, updated_at) \
         VALUES (?, ?, ?, 'preparing', NULL, 1, ?, ?)",
    )
    .bind(&preparation_id)
    .bind(&document_id)
    .bind(&sha)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 preparation");

    let snapshot_id = ids::new_id();
    sqlx::query(
        "INSERT INTO generation_snapshots \
         (id, item_id, item_revision, preparation_id, photo_ids, photo_hashes, provider_config, prompt_version, price_version, budgets, created_at) \
         VALUES (?, ?, 1, ?, '[]', '[]', '{}', 'prompt-v1', 'price-v1', '{}', ?)",
    )
    .bind(&snapshot_id)
    .bind(&item.id)
    .bind(&preparation_id)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 generation_snapshot");

    let job_id = ids::new_id();
    sqlx::query(
        "INSERT INTO jobs (id, item_id, snapshot_id, status, revision, created_at, updated_at) \
         VALUES (?, ?, ?, 'queued', 1, ?, ?)",
    )
    .bind(&job_id)
    .bind(&item.id)
    .bind(&snapshot_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 job");

    let stage_id = ids::new_id();
    sqlx::query(
        "INSERT INTO job_stages \
         (id, job_id, stage_kind, batch_index, page_set, input_hash, status, lease_epoch, attempt_count, created_at, updated_at) \
         VALUES (?, ?, 'manual_extract', 0, '[1,2]', 'hash-1', 'queued', 0, 0, ?, ?)",
    )
    .bind(&stage_id)
    .bind(&job_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 job_stage");

    Chain {
        item_id: item.id,
        document_id,
        preparation_id,
        snapshot_id,
        job_id,
        stage_id,
        asset_id,
    }
}

/// 在给定链路上插入 model_revision / draft / release（发布不可变性测试用）。
async fn seed_release(pool: &SqlitePool, chain: &Chain) -> String {
    let now = Timestamp::now().as_millis();
    let model_sha = insert_blob(pool, 0xB2).await;
    let model_asset = insert_asset(pool, &chain.item_id, &model_sha, "model").await;

    let model_revision_id = ids::new_id();
    sqlx::query(
        "INSERT INTO model_revisions (id, item_id, asset_id, sha256, provider_attempt_id, bounds, validation_state, created_at) \
         VALUES (?, ?, ?, ?, NULL, '{\"min\":[0,0,0],\"max\":[1,1,1]}', 'validated', ?)",
    )
    .bind(&model_revision_id)
    .bind(&chain.item_id)
    .bind(&model_asset)
    .bind(&model_sha)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 model_revision");

    let draft_id = ids::new_id();
    sqlx::query(
        "INSERT INTO manual_drafts (id, item_id, snapshot_id, model_revision_id, revision, status, knowledge_json, review_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 1, 'needs_review', '{\"parts\":[]}', NULL, ?, ?)",
    )
    .bind(&draft_id)
    .bind(&chain.item_id)
    .bind(&chain.snapshot_id)
    .bind(&model_revision_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 manual_draft");

    let release_id = ids::new_id();
    sqlx::query(
        "INSERT INTO manual_releases (id, item_id, draft_id, draft_revision, model_revision_id, manifest_asset_id, created_at) \
         VALUES (?, ?, ?, 1, ?, ?, ?)",
    )
    .bind(&release_id)
    .bind(&chain.item_id)
    .bind(&draft_id)
    .bind(&model_revision_id)
    .bind(&chain.asset_id)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 manual_release");

    release_id
}

/// 用一个只含指定迁移文件的临时目录建库（模拟"旧版本程序"或"坏迁移"）。
async fn connect_raw(db_path: &Path, create: bool) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(create)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(everything_manual::storage::BUSY_TIMEOUT)
        .foreign_keys(true);
    sqlx::pool::PoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("连接测试数据库")
}

// ---------------------------------------------------------------------------
// AC-007：空库初始化
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fresh_data_dir_initializes_full_schema_constraints_and_connection_settings() {
    let dir = TestDir::new("fresh");
    init_structure(dir.path());
    let database = open(dir.path()).await;

    // 程序 schema 版本 = 内嵌迁移集的最大版本（T19 起为 7：0007_release_manifest）。
    // 事实更新（T19 交付回合，2026-09-12）：0007 给 assets.purpose 增加
    // release_manifest 值（发布 manifest 资产）；旧值 6 是 0006 时代的断言。
    assert_eq!(database.program_schema_version(), 7);
    assert_eq!(database.applied_schema_version().await.unwrap(), 7);
    // 库文件落在 data-dir 内，且 WAL/SHM 由 SQLite 管理（T20 备份必须处理）。
    assert!(database_path(dir.path()).is_file());
    assert!(
        dir.path()
            .join(format!("{DATABASE_FILE_NAME}-wal"))
            .exists()
    );

    let pool = database.pool();
    let tables = names(
        pool,
        "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
    )
    .await;
    for expected in [
        "_sqlx_migrations",
        "admins",
        "sessions",
        "items",
        "blobs",
        "assets",
        "documents",
        "preparations",
        "pages",
        "photos",
        "generation_snapshots",
        "jobs",
        "job_stage_deps",
        "job_stages",
        "provider_attempts",
        "idempotency_records",
        "cost_ledger",
        "quotes",
        "model_revisions",
        "manual_drafts",
        "manual_releases",
        "audit_events",
    ] {
        assert!(
            tables.iter().any(|name| name == expected),
            "缺少表 {expected}（实际：{tables:?}）"
        );
    }

    // 外键：引用目标与 contracts.md §2 一致。
    let expectations: [(&str, &[&str]); 14] = [
        ("sessions", &["admins"]),
        ("assets", &["blobs", "items"]),
        ("documents", &["items", "assets", "blobs"]),
        ("preparations", &["documents", "blobs"]),
        ("pages", &["preparations", "assets"]),
        ("photos", &["items", "assets"]),
        ("generation_snapshots", &["items", "preparations"]),
        ("jobs", &["items", "generation_snapshots"]),
        ("job_stages", &["jobs", "assets"]),
        ("provider_attempts", &["jobs", "job_stages"]),
        ("idempotency_records", &["admins"]),
        (
            "cost_ledger",
            &["generation_snapshots", "provider_attempts"],
        ),
        ("model_revisions", &["items", "assets", "provider_attempts"]),
        (
            "manual_releases",
            &["items", "manual_drafts", "model_revisions", "assets"],
        ),
    ];
    for (table, targets) in expectations {
        let actual = foreign_key_targets(pool, table).await;
        for target in targets {
            assert!(
                actual.iter().any(|name| name == target),
                "{table} 缺少指向 {target} 的外键（实际：{actual:?}）"
            );
        }
    }

    // 命名索引（含 0002 的部分唯一索引、0003 的照片视图唯一索引与 0001 的查询索引）。
    let indexes = names(
        pool,
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_autoindex%'",
    )
    .await;
    for expected in [
        "sessions_admin",
        "sessions_expires",
        "items_created",
        "assets_item",
        "assets_blob",
        "documents_item",
        "preparations_document",
        "photos_item",
        "photos_item_view_unique",
        "generation_snapshots_item",
        "jobs_item",
        "jobs_status",
        "job_stages_job",
        "job_stages_due",
        "provider_attempts_job",
        "provider_attempts_stage",
        "provider_attempts_unresolved_stage",
        "idempotency_records_resource",
        "cost_ledger_snapshot",
        "cost_ledger_attempt",
        "cost_ledger_active_reservation",
        "quotes_item",
        "quotes_expires",
        "model_revisions_item",
        "manual_drafts_item",
        "manual_releases_item",
        "manual_releases_draft",
        "audit_events_entity",
        "audit_events_created",
    ] {
        assert!(
            indexes.iter().any(|name| name == expected),
            "缺少索引 {expected}（实际：{indexes:?}）"
        );
    }
    let triggers = names(
        pool,
        "SELECT name FROM sqlite_master WHERE type = 'trigger'",
    )
    .await;
    assert_eq!(triggers.len(), 7, "不变量触发器数量：{triggers:?}");
    for expected in [
        "generation_snapshots_immutable",
        "manual_releases_immutable",
        "manual_releases_no_delete",
        "provider_attempts_remote_task_id_monotonic",
        "quotes_snapshot_immutable",
        "quotes_confirmation_frozen",
        "quotes_consumption_frozen",
    ] {
        assert!(
            triggers.iter().any(|name| name == expected),
            "缺少触发器 {expected}（实际：{triggers:?}）"
        );
    }

    // 连接设置（architecture.md §6、PRD §5.4）。
    let settings: ConnectionSettings = database.connection_settings().await.unwrap();
    assert_eq!(settings.journal_mode.to_ascii_lowercase(), "wal");
    assert_eq!(settings.synchronous, 2, "synchronous 必须是 FULL(2)");
    assert_eq!(settings.busy_timeout_ms, 5_000);
    assert!(settings.foreign_keys, "foreign_keys 必须为 ON");
    assert!(settings.meets_contract(), "{settings:?}");
    assert_eq!(
        pool.options().get_max_connections(),
        POOL_MAX_CONNECTIONS,
        "连接池上限必须是 4"
    );

    // 文件权限：数据库文件收紧到 0600（data-dir 本身 0700，见 T02）。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(database_path(dir.path()))
            .expect("读取数据库文件权限")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "数据库文件权限应为 0600，实际 {mode:o}");
    }

    database.close().await;
}

#[tokio::test]
async fn embedded_migrations_match_repository_files() {
    // 迁移随二进制内嵌（REQ-004）：内嵌内容必须与 migrations/ 下的文件逐字节一致，
    // 否则二进制会带着与仓库不同的 schema 运行。
    let embedded: Vec<String> = everything_manual::storage::migrations::MIGRATOR
        .migrations
        .iter()
        .map(|migration| migration.sql.as_str().to_owned())
        .collect();
    assert_eq!(
        embedded,
        vec![
            MIGRATION_0001.to_owned(),
            MIGRATION_0002.to_owned(),
            MIGRATION_0003.to_owned(),
            MIGRATION_0004.to_owned(),
            MIGRATION_0005.to_owned(),
            MIGRATION_0006.to_owned(),
            MIGRATION_0007.to_owned(),
        ]
    );
}

#[tokio::test]
async fn migrations_are_idempotent_across_reopens() {
    let dir = TestDir::new("idempotent");
    init_structure(dir.path());

    let first = open(dir.path()).await;
    let versions = names(
        first.pool(),
        "SELECT CAST(version AS TEXT) FROM _sqlx_migrations ORDER BY version",
    )
    .await;
    assert_eq!(
        versions,
        vec![
            "1".to_owned(),
            "2".to_owned(),
            "3".to_owned(),
            "4".to_owned(),
            "5".to_owned(),
            "6".to_owned(),
            "7".to_owned()
        ]
    );
    first.close().await;

    let second = open(dir.path()).await;
    assert_eq!(second.applied_schema_version().await.unwrap(), 7);
    assert_eq!(
        count(second.pool(), "SELECT COUNT(*) FROM _sqlx_migrations").await,
        7,
        "重复打开不得重复写入迁移记录"
    );
    second.close().await;
}

// ---------------------------------------------------------------------------
// AC-007：外键与唯一键
// ---------------------------------------------------------------------------

#[tokio::test]
async fn foreign_keys_reject_invalid_references_and_restrict_deletes() {
    let dir = TestDir::new("fk");
    init_structure(dir.path());
    let database = open(dir.path()).await;
    let pool = database.pool();

    let item = create_item(pool, "相机", "X100V").await;
    let sha = insert_blob(pool, 0xC3).await;
    let asset_id = insert_asset(pool, &item.id, &sha, "photo").await;
    let now = Timestamp::now().as_millis();

    // 引用不存在的 item：拒绝。
    let missing_item = expect_error(
        sqlx::query(
            "INSERT INTO photos (id, item_id, asset_id, view, revision, created_at, updated_at) \
             VALUES (?, ?, ?, 'front', 1, ?, ?)",
        )
        .bind(ids::new_id())
        .bind(ids::new_id())
        .bind(&asset_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await,
    );
    assert!(missing_item.is_foreign_key_violation(), "{missing_item}");

    // 引用不存在的 asset：拒绝。
    let missing_asset = expect_error(
        sqlx::query(
            "INSERT INTO photos (id, item_id, asset_id, view, revision, created_at, updated_at) \
             VALUES (?, ?, ?, 'front', 1, ?, ?)",
        )
        .bind(ids::new_id())
        .bind(&item.id)
        .bind(ids::new_id())
        .bind(now)
        .bind(now)
        .execute(pool)
        .await,
    );
    assert!(missing_asset.is_foreign_key_violation(), "{missing_asset}");

    // 合法引用成功；删除被引用的物品被 RESTRICT 拒绝，且物品仍在。
    sqlx::query(
        "INSERT INTO photos (id, item_id, asset_id, view, revision, created_at, updated_at) \
         VALUES (?, ?, ?, 'front', 1, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(&item.id)
    .bind(&asset_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("合法照片应能插入");

    let delete_blocked = expect_error(
        sqlx::query("DELETE FROM items WHERE id = ?")
            .bind(&item.id)
            .execute(pool)
            .await,
    );
    assert!(
        delete_blocked.is_foreign_key_violation(),
        "{delete_blocked}"
    );
    assert_eq!(
        count(pool, "SELECT COUNT(*) FROM items").await,
        1,
        "RESTRICT 必须阻止删除被引用物品"
    );

    // 级联：删除 admins 会删除其 sessions（ON DELETE CASCADE；T04 之后才会真实发生）。
    let admin_id = ids::new_id();
    sqlx::query("INSERT INTO admins (id, password_hash, created_at, updated_at) VALUES (?, '$argon2id$test', ?, ?)")
        .bind(&admin_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .expect("插入 admin");
    sqlx::query(
        "INSERT INTO sessions (id, admin_id, session_token_hash, csrf_hash, created_at, expires_at, revoked_at) \
         VALUES (?, ?, 'token-hash-1', 'csrf-hash-1', ?, ?, NULL)",
    )
    .bind(ids::new_id())
    .bind(&admin_id)
    .bind(now)
    .bind(now + 60_000)
    .execute(pool)
    .await
    .expect("插入 session");
    sqlx::query("DELETE FROM admins WHERE id = ?")
        .bind(&admin_id)
        .execute(pool)
        .await
        .expect("删除 admin 应级联删除 session");
    assert_eq!(count(pool, "SELECT COUNT(*) FROM sessions").await, 0);

    database.close().await;
}

#[tokio::test]
async fn unique_keys_reject_duplicates() {
    let dir = TestDir::new("unique");
    init_structure(dir.path());
    let database = open(dir.path()).await;
    let pool = database.pool();
    let now = Timestamp::now().as_millis();

    // blobs：sha256 唯一内容存储。
    let sha = insert_blob(pool, 0xD4).await;
    let duplicate_blob = expect_error(
        sqlx::query(
            "INSERT INTO blobs (sha256, size, mime, storage_state, created_at) VALUES (?, 1, 'application/pdf', 'stored', ?)",
        )
        .bind(&sha)
        .bind(now)
        .execute(pool)
        .await,
    );
    assert!(duplicate_blob.is_unique_violation(), "{duplicate_blob}");

    // items：同品牌型号不强制唯一（允许不同配置并存）。
    let first = create_item(pool, "相机", "X100V").await;
    let second = create_item(pool, "相机", "X100V").await;
    assert_ne!(first.id, second.id);
    assert_eq!(count(pool, "SELECT COUNT(*) FROM items").await, 2);

    let chain = seed_chain(pool).await;
    assert!(ids::is_valid_id(&chain.document_id));

    // pages：同一 preparation 的页号唯一。
    sqlx::query(
        "INSERT INTO pages (preparation_id, page_number, text_asset_id, image_asset_id, created_at, updated_at) \
         VALUES (?, 1, NULL, NULL, ?, ?)",
    )
    .bind(&chain.preparation_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入第一页");
    let duplicate_page = expect_error(
        sqlx::query(
            "INSERT INTO pages (preparation_id, page_number, text_asset_id, image_asset_id, created_at, updated_at) \
             VALUES (?, 1, NULL, NULL, ?, ?)",
        )
        .bind(&chain.preparation_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await,
    );
    assert!(duplicate_page.is_unique_violation(), "{duplicate_page}");

    // job_stages：job + stage_kind + batch_index 唯一。
    let duplicate_stage = expect_error(
        sqlx::query(
            "INSERT INTO job_stages (id, job_id, stage_kind, batch_index, input_hash, status, lease_epoch, attempt_count, created_at, updated_at) \
             VALUES (?, ?, 'manual_extract', 0, 'hash-1', 'queued', 0, 0, ?, ?)",
        )
        .bind(ids::new_id())
        .bind(&chain.job_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await,
    );
    assert!(duplicate_stage.is_unique_violation(), "{duplicate_stage}");

    sqlx::query(
        "INSERT INTO job_stages (id, job_id, stage_kind, batch_index, input_hash, status, lease_epoch, attempt_count, created_at, updated_at) \
         VALUES (?, ?, 'manual_extract', 1, 'hash-2', 'queued', 0, 0, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(&chain.job_id)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("第二批 manual_extract 允许");

    // 非批处理阶段 batch_index 必须为 0。
    let bad_batch = expect_error(
        sqlx::query(
            "INSERT INTO job_stages (id, job_id, stage_kind, batch_index, input_hash, status, lease_epoch, attempt_count, created_at, updated_at) \
             VALUES (?, ?, 'tripo_submit', 1, 'hash-3', 'queued', 0, 0, ?, ?)",
        )
        .bind(ids::new_id())
        .bind(&chain.job_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await,
    );
    assert!(bad_batch.is_constraint_violation(), "{bad_batch}");

    // provider_attempts：同一阶段只允许一个未对账 attempt。
    let attempt_id = ids::new_id();
    sqlx::query(
        "INSERT INTO provider_attempts (id, job_id, stage_id, request_hash, submit_state, remote_task_id, response_id, started_at, last_error, created_at, updated_at) \
         VALUES (?, ?, ?, 'req-1', 'intent', NULL, NULL, ?, NULL, ?, ?)",
    )
    .bind(&attempt_id)
    .bind(&chain.job_id)
    .bind(&chain.stage_id)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入未对账 attempt");
    let duplicate_attempt = expect_error(
        sqlx::query(
            "INSERT INTO provider_attempts (id, job_id, stage_id, request_hash, submit_state, remote_task_id, response_id, started_at, last_error, created_at, updated_at) \
             VALUES (?, ?, ?, 'req-2', 'submitting', NULL, NULL, ?, NULL, ?, ?)",
        )
        .bind(ids::new_id())
        .bind(&chain.job_id)
        .bind(&chain.stage_id)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await,
    );
    assert!(
        duplicate_attempt.is_unique_violation(),
        "{duplicate_attempt}"
    );
    // 已定性（accepted）后允许新建 attempt（重试分支）。
    sqlx::query("UPDATE provider_attempts SET submit_state = 'accepted' WHERE id = ?")
        .bind(&attempt_id)
        .execute(pool)
        .await
        .expect("标记 accepted");
    sqlx::query(
        "INSERT INTO provider_attempts (id, job_id, stage_id, request_hash, submit_state, remote_task_id, response_id, started_at, last_error, created_at, updated_at) \
         VALUES (?, ?, ?, 'req-3', 'intent', NULL, NULL, ?, NULL, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(&chain.job_id)
    .bind(&chain.stage_id)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("accepted 后允许新 attempt");

    // idempotency_records：admin + method + route + key 唯一。
    let admin_id = ids::new_id();
    sqlx::query("INSERT INTO admins (id, password_hash, created_at, updated_at) VALUES (?, '$argon2id$test', ?, ?)")
        .bind(&admin_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .expect("插入 admin");
    let insert_idempotency = |id: String| {
        let admin_id = admin_id.clone();
        async move {
            sqlx::query(
                "INSERT INTO idempotency_records (id, admin_id, method, route, \"key\", body_hash, resource_id, response_status, created_at) \
                 VALUES (?, ?, 'POST', '/api/v1/items', 'key-1', 'body-hash', NULL, NULL, ?)",
            )
            .bind(id)
            .bind(admin_id)
            .bind(now)
            .execute(pool)
            .await
        }
    };
    insert_idempotency(ids::new_id())
        .await
        .expect("插入幂等记录");
    let duplicate_key = expect_error(insert_idempotency(ids::new_id()).await);
    assert!(duplicate_key.is_unique_violation(), "{duplicate_key}");

    // manual_drafts：每个快照至多一份草稿（assemble_draft 幂等）。
    let _release = seed_release(pool, &chain).await;
    let duplicate_draft = expect_error(
        sqlx::query(
            "INSERT INTO manual_drafts (id, item_id, snapshot_id, model_revision_id, revision, status, knowledge_json, review_json, created_at, updated_at) \
             VALUES (?, ?, ?, NULL, 1, 'needs_review', '{}', NULL, ?, ?)",
        )
        .bind(ids::new_id())
        .bind(&chain.item_id)
        .bind(&chain.snapshot_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await,
    );
    assert!(duplicate_draft.is_unique_violation(), "{duplicate_draft}");

    database.close().await;
}

// ---------------------------------------------------------------------------
// AC-007：不变量触发器、revision CAS、事务回滚、重开保留数据
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invariant_triggers_protect_frozen_inputs_releases_and_remote_task_ids() {
    let dir = TestDir::new("triggers");
    init_structure(dir.path());
    let database = open(dir.path()).await;
    let pool = database.pool();
    let chain = seed_chain(pool).await;

    // 冻结输入不可变（editing 原物品不改变已开始任务）。
    let frozen = expect_error(
        sqlx::query("UPDATE generation_snapshots SET price_version = 'price-v2' WHERE id = ?")
            .bind(&chain.snapshot_id)
            .execute(pool)
            .await,
    );
    assert!(frozen.is_constraint_violation(), "{frozen}");
    assert!(frozen.to_string().contains("不允许修改"), "{frozen}");

    // 已发布 release 不可修改、不可删除。
    let release_id = seed_release(pool, &chain).await;
    let release_update = expect_error(
        sqlx::query("UPDATE manual_releases SET draft_revision = 2 WHERE id = ?")
            .bind(&release_id)
            .execute(pool)
            .await,
    );
    assert!(release_update.is_constraint_violation(), "{release_update}");
    assert!(
        release_update.to_string().contains("不允许修改"),
        "{release_update}"
    );
    let release_delete = expect_error(
        sqlx::query("DELETE FROM manual_releases WHERE id = ?")
            .bind(&release_id)
            .execute(pool)
            .await,
    );
    assert!(release_delete.is_constraint_violation(), "{release_delete}");

    // 远端 task id：null→值 或 同值 允许；覆盖不同值或清空被拒。
    let now = Timestamp::now().as_millis();
    let attempt_id = ids::new_id();
    sqlx::query(
        "INSERT INTO provider_attempts (id, job_id, stage_id, request_hash, submit_state, remote_task_id, response_id, started_at, last_error, created_at, updated_at) \
         VALUES (?, ?, ?, 'req-1', 'submitting', NULL, NULL, ?, NULL, ?, ?)",
    )
    .bind(&attempt_id)
    .bind(&chain.job_id)
    .bind(&chain.stage_id)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("插入 attempt");

    sqlx::query(
        "UPDATE provider_attempts SET remote_task_id = 'task-from-provider-1' WHERE id = ?",
    )
    .bind(&attempt_id)
    .execute(pool)
    .await
    .expect("null→值 必须允许（过期租约补写事实观察）");
    sqlx::query(
        "UPDATE provider_attempts SET remote_task_id = 'task-from-provider-1' WHERE id = ?",
    )
    .bind(&attempt_id)
    .execute(pool)
    .await
    .expect("同值重复写入必须允许");
    let overwrite = expect_error(
        sqlx::query("UPDATE provider_attempts SET remote_task_id = 'task-2' WHERE id = ?")
            .bind(&attempt_id)
            .execute(pool)
            .await,
    );
    assert!(overwrite.is_constraint_violation(), "{overwrite}");
    let clear = expect_error(
        sqlx::query("UPDATE provider_attempts SET remote_task_id = NULL WHERE id = ?")
            .bind(&attempt_id)
            .execute(pool)
            .await,
    );
    assert!(clear.is_constraint_violation(), "{clear}");

    database.close().await;
}

#[tokio::test]
async fn revision_cas_rejects_stale_updates_and_reports_current_revision() {
    let dir = TestDir::new("cas");
    init_structure(dir.path());
    let database = open(dir.path()).await;
    let pool = database.pool();

    let item = create_item(pool, "相机", "X100V").await;
    assert_eq!(item.revision, 1);

    let mut conn = pool.acquire().await.unwrap();
    let updated = repo::items::update(
        &mut conn,
        &item.id,
        1,
        ItemUpdate {
            name: "相机（已编辑）".to_owned(),
            brand: Some("Fuji".to_owned()),
            model: "X100V".to_owned(),
            variant: Some("银色".to_owned()),
            archived: false,
        },
    )
    .await
    .expect("revision=1 更新成功");
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.name, "相机（已编辑）");

    // 过期 revision：冲突且带 current_revision；内容不被部分写入。
    let stale = repo::items::update(
        &mut conn,
        &item.id,
        1,
        ItemUpdate {
            name: "不应写入".to_owned(),
            brand: None,
            model: "X100V".to_owned(),
            variant: None,
            archived: false,
        },
    )
    .await
    .expect_err("过期 revision 必须失败");
    match &stale {
        StorageError::RevisionConflict {
            current_revision, ..
        } => assert_eq!(*current_revision, 2),
        other => panic!("期望 RevisionConflict，实际：{other}"),
    }
    let stored = repo::items::get(&mut conn, &item.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.name, "相机（已编辑）", "失败更新不得写入任何字段");

    // 不存在：NotFound。
    let missing = repo::items::update(
        &mut conn,
        &ids::new_id(),
        1,
        ItemUpdate {
            name: "x".to_owned(),
            brand: None,
            model: "y".to_owned(),
            variant: None,
            archived: false,
        },
    )
    .await
    .expect_err("不存在的物品必须报错");
    assert!(
        matches!(missing, StorageError::NotFound { .. }),
        "{missing}"
    );

    // 归档：记录时间，取消归档清除。
    let archived = repo::items::update(
        &mut conn,
        &item.id,
        2,
        ItemUpdate {
            name: "相机（已编辑）".to_owned(),
            brand: Some("Fuji".to_owned()),
            model: "X100V".to_owned(),
            variant: Some("银色".to_owned()),
            archived: true,
        },
    )
    .await
    .expect("归档成功");
    assert!(archived.archived_at.is_some());
    let unarchived = repo::items::update(
        &mut conn,
        &item.id,
        3,
        ItemUpdate {
            name: "相机（已编辑）".to_owned(),
            brand: Some("Fuji".to_owned()),
            model: "X100V".to_owned(),
            variant: Some("银色".to_owned()),
            archived: false,
        },
    )
    .await
    .expect("取消归档成功");
    assert!(unarchived.archived_at.is_none());
    drop(conn);

    // 并发 CAS：同一期望 revision 的两个并发更新只允许一个成功。
    let target = create_item(pool, "并发", "X100V").await;
    let patch = || ItemUpdate {
        name: "并发（胜者）".to_owned(),
        brand: None,
        model: "X100V".to_owned(),
        variant: None,
        archived: false,
    };
    let mut first_conn = pool.acquire().await.unwrap();
    let mut second_conn = pool.acquire().await.unwrap();
    let first = repo::items::update(&mut first_conn, &target.id, 1, patch());
    let second = repo::items::update(&mut second_conn, &target.id, 1, patch());
    let (first, second) = tokio::join!(first, second);
    let successes = [first.as_ref(), second.as_ref()]
        .iter()
        .filter(|result| result.is_ok())
        .count();
    let conflicts = [first.as_ref(), second.as_ref()]
        .iter()
        .filter(|result| matches!(result, Err(StorageError::RevisionConflict { .. })))
        .count();
    assert_eq!(
        successes, 1,
        "并发更新必须恰好一个成功：{first:?} / {second:?}"
    );
    assert_eq!(
        conflicts, 1,
        "并发更新必须恰好一个冲突：{first:?} / {second:?}"
    );

    drop(first_conn);
    drop(second_conn);
    database.close().await;
}

#[tokio::test]
async fn transaction_rollback_leaves_no_partial_state() {
    let dir = TestDir::new("rollback");
    init_structure(dir.path());
    let database = open(dir.path()).await;
    let pool = database.pool();
    let item = create_item(pool, "相机", "X100V").await;
    let now = Timestamp::now().as_millis();

    // 事务内先写一行合法数据，再违反外键：回滚后两行都不存在。
    let mut tx = pool.begin().await.expect("开启事务");
    let item_id = ids::new_id();
    sqlx::query(
        "INSERT INTO items (id, name, brand, model, variant, revision, archived_at, created_at, updated_at) \
         VALUES (?, '事务物品', NULL, 'T-1', NULL, 1, NULL, ?, ?)",
    )
    .bind(&item_id)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("事务内插入 items");
    let bad = sqlx::query(
        "INSERT INTO photos (id, item_id, asset_id, view, revision, created_at, updated_at) \
         VALUES (?, ?, ?, 'front', 1, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(&item.id)
    .bind(ids::new_id())
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await;
    assert!(bad.is_err(), "外键违规必须让语句失败");
    tx.rollback().await.expect("回滚事务");

    let mut conn = pool.acquire().await.unwrap();
    assert!(
        repo::items::get(&mut conn, &item_id)
            .await
            .unwrap()
            .is_none(),
        "回滚后不得残留事务内写入"
    );
    assert_eq!(count(pool, "SELECT COUNT(*) FROM items").await, 1);

    // 提交路径对照：同一事务内的两行都应持久化。
    let mut tx = pool.begin().await.expect("开启事务");
    let committed_id = ids::new_id();
    sqlx::query(
        "INSERT INTO items (id, name, brand, model, variant, revision, archived_at, created_at, updated_at) \
         VALUES (?, '已提交物品', NULL, 'T-2', NULL, 1, NULL, ?, ?)",
    )
    .bind(&committed_id)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("事务内插入 items");
    tx.commit().await.expect("提交事务");
    assert!(
        repo::items::get(&mut conn, &committed_id)
            .await
            .unwrap()
            .is_some()
    );

    drop(conn);
    database.close().await;
}

#[tokio::test]
async fn closing_and_reopening_data_dir_preserves_data() {
    let dir = TestDir::new("reopen");
    init_structure(dir.path());

    let database = open(dir.path()).await;
    let item = create_item(database.pool(), "相机", "X100V").await;
    let sha = insert_blob(database.pool(), 0xE5).await;
    let asset_id = insert_asset(database.pool(), &item.id, &sha, "photo").await;
    let now = Timestamp::now().as_millis();
    sqlx::query(
        "INSERT INTO photos (id, item_id, asset_id, view, revision, created_at, updated_at) \
         VALUES (?, ?, ?, 'front', 1, ?, ?)",
    )
    .bind(ids::new_id())
    .bind(&item.id)
    .bind(&asset_id)
    .bind(now)
    .bind(now)
    .execute(database.pool())
    .await
    .expect("插入照片");
    database.close().await;

    let reopened = open(dir.path()).await;
    let pool = reopened.pool();
    let mut conn = pool.acquire().await.unwrap();
    let stored = repo::items::get(&mut conn, &item.id)
        .await
        .unwrap()
        .expect("重开后物品仍在");
    assert_eq!(stored, item);
    assert_eq!(count(pool, "SELECT COUNT(*) FROM photos").await, 1);
    assert_eq!(count(pool, "SELECT COUNT(*) FROM blobs").await, 1);
    drop(conn);
    reopened.close().await;
}

// ---------------------------------------------------------------------------
// AC-008：schema 兼容门禁与迁移
// ---------------------------------------------------------------------------

#[tokio::test]
async fn future_schema_is_rejected_without_modifying_database_bytes() {
    let dir = TestDir::new("future-schema");
    init_structure(dir.path());

    // 构造"由更新版本程序写下的库"：插入一条版本远大于程序的成功迁移记录。
    let database = open(dir.path()).await;
    let mut conn = database.pool().acquire().await.unwrap();
    let item = repo::items::create(
        &mut conn,
        NewItem {
            name: "保留数据".to_owned(),
            brand: None,
            model: "X100V".to_owned(),
            variant: None,
        },
    )
    .await
    .expect("写入用户数据");
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (9999, 'future_program_migration', 1, X'00', 0)",
    )
    .execute(&mut *conn)
    .await
    .expect("插入未来迁移记录");
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&mut *conn)
        .await
        .expect("checkpoint");
    drop(conn);
    database.close().await;

    let before = std::fs::read(database_path(dir.path())).expect("读取库文件字节");

    // 拒绝打开：可读错误 + 不修改数据。
    let error = Database::open_and_migrate(dir.path())
        .await
        .expect_err("比程序更新的 schema 必须被拒绝");
    assert!(error.is_schema_too_new(), "{error}");
    let message = error.to_string();
    assert!(message.contains("v9999"), "{message}");
    assert!(message.contains("v7"), "{message}");
    assert!(message.contains("拒绝打开"), "{message}");

    let after = std::fs::read(database_path(dir.path())).expect("再次读取库文件字节");
    assert_eq!(before, after, "拒绝打开时不得修改库文件字节");

    // 用户数据与未来迁移记录保持原样（用能读懂 v9999 的方式只读检查）。
    let pool = connect_raw(&database_path(dir.path()), false).await;
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM items").await, 1);
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 9999"
        )
        .await,
        1
    );
    pool.close().await;

    // `check` 同样拒绝（退出码 4，data-dir 错误）且不改文件。
    let check = run_cli(
        dir.path(),
        &["check", "--data-dir", dir.path().to_str().unwrap()],
    );
    assert_eq!(check.status, 4, "{check:?}");
    assert!(check.stderr.contains("v9999"), "{check:?}");
    assert_eq!(
        std::fs::read(database_path(dir.path())).unwrap(),
        before,
        "check 拒绝时同样不得修改库文件"
    );
    let _ = item;
}

#[tokio::test]
async fn older_schema_is_migrated_automatically_and_data_preserved() {
    let dir = TestDir::new("older-schema");
    init_structure(dir.path());

    // 模拟"旧版本程序"：迁移目录只有 0001（内容与生产文件逐字节一致）。
    let legacy_dir = dir.join("legacy-migrations");
    std::fs::create_dir_all(&legacy_dir).unwrap();
    std::fs::write(legacy_dir.join("0001_core_schema.sql"), MIGRATION_0001).unwrap();
    let legacy_migrator = sqlx::migrate::Migrator::new(legacy_dir.as_path())
        .await
        .expect("解析旧迁移目录");

    let db_path = database_path(dir.path());
    let pool = connect_raw(&db_path, true).await;
    legacy_migrator.run(&pool).await.expect("应用旧迁移");
    assert_eq!(
        everything_manual::storage::migrations::applied_schema_version(&pool)
            .await
            .unwrap(),
        1,
        "旧库只能到 v1"
    );

    // 旧库中的用户数据（升级必须保留）。
    let now = Timestamp::now().as_millis();
    let item_id = ids::new_id();
    sqlx::query(
        "INSERT INTO items (id, name, brand, model, variant, revision, archived_at, created_at, updated_at) \
         VALUES (?, '升级前数据', 'Fuji', 'X100V', NULL, 3, NULL, ?, ?)",
    )
    .bind(&item_id)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("写入旧数据");
    let stage_columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('job_stages') ORDER BY cid")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!stage_columns.is_empty());
    pool.close().await;

    // 程序升级：同一 data-dir 打开即自动迁移到程序版本。
    let database = open(dir.path()).await;
    assert_eq!(database.applied_schema_version().await.unwrap(), 7);
    let mut conn = database.pool().acquire().await.unwrap();
    let migrated = repo::items::get(&mut conn, &item_id)
        .await
        .unwrap()
        .expect("升级后旧数据仍在");
    assert_eq!(migrated.name, "升级前数据");
    assert_eq!(migrated.revision, 3);
    drop(conn);

    // v2 的新不变量在升级后的库中生效。
    let triggers = names(
        database.pool(),
        "SELECT name FROM sqlite_master WHERE type = 'trigger'",
    )
    .await;
    assert_eq!(
        triggers.len(),
        7,
        "升级后必须应用 0002/0006 的触发器：{triggers:?}"
    );
    assert_eq!(
        count(database.pool(), "SELECT COUNT(*) FROM _sqlx_migrations").await,
        7
    );

    database.close().await;
}

#[tokio::test]
async fn failed_migration_leaves_no_partial_state() {
    let dir = TestDir::new("broken-migration");
    init_structure(dir.path());

    // 0001 正常 + 0002 故意写坏（第二条语句引用不存在的表）：0..N 中第 2 条失败。
    let broken_dir = dir.join("broken-migrations");
    std::fs::create_dir_all(&broken_dir).unwrap();
    std::fs::write(broken_dir.join("0001_core_schema.sql"), MIGRATION_0001).unwrap();
    std::fs::write(
        broken_dir.join("0002_broken.sql"),
        "CREATE TABLE should_not_exist (id TEXT);\nINSERT INTO missing_table VALUES (1);\n",
    )
    .unwrap();
    let broken = sqlx::migrate::Migrator::new(broken_dir.as_path())
        .await
        .expect("解析坏迁移目录");

    let db_path = database_path(dir.path());
    let pool = connect_raw(&db_path, true).await;
    let error = broken.run(&pool).await.expect_err("坏迁移必须失败");
    let error = StorageError::from(error);
    assert!(matches!(error, StorageError::Migration { .. }), "{error}");

    // 失败迁移的语句整体回滚：临时表不存在，迁移记录只保留成功的 0001。
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'should_not_exist'"
        )
        .await,
        0,
        "失败迁移不得留下半创建的表"
    );
    assert_eq!(
        everything_manual::storage::migrations::applied_schema_version(&pool)
            .await
            .unwrap(),
        1
    );
    pool.close().await;

    // 修复后（生产迁移集）可继续升级，且不重复应用 0001。
    let database = open(dir.path()).await;
    assert_eq!(database.applied_schema_version().await.unwrap(), 7);
    assert_eq!(
        count(database.pool(), "SELECT COUNT(*) FROM _sqlx_migrations").await,
        7
    );
    database.close().await;
}

// ---------------------------------------------------------------------------
// check / serve 的 CLI 行为（退出码 3/4 沿用 T02 约定）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_reports_migration_state_without_creating_or_modifying_database() {
    let dir = TestDir::new("check-state");
    init_structure(dir.path());
    let data_dir = dir.path().to_str().unwrap().to_owned();

    // 未建库：check 通过且不创建数据库（T20 的只读检查前提）。
    let missing = run_cli(dir.path(), &["check", "--data-dir", &data_dir]);
    assert_eq!(missing.status, 0, "{missing:?}");
    assert!(
        missing.stdout.contains("数据库 schema：尚未建立"),
        "{missing:?}"
    );
    assert!(!database_path(dir.path()).exists(), "check 不得创建数据库");

    // 旧 schema：check 报告"待迁移"且不修改数据（迁移只发生在 init/serve）。
    let legacy_dir = dir.join("legacy-migrations");
    std::fs::create_dir_all(&legacy_dir).unwrap();
    std::fs::write(legacy_dir.join("0001_core_schema.sql"), MIGRATION_0001).unwrap();
    let legacy_migrator = sqlx::migrate::Migrator::new(legacy_dir.as_path())
        .await
        .unwrap();
    let pool = connect_raw(&database_path(dir.path()), true).await;
    legacy_migrator.run(&pool).await.unwrap();
    pool.close().await;

    let pending = run_cli(dir.path(), &["check", "--data-dir", &data_dir]);
    assert_eq!(pending.status, 0, "{pending:?}");
    assert!(pending.stdout.contains("待迁移"), "{pending:?}");
    assert!(pending.stdout.contains("库 v1 → 程序 v7"), "{pending:?}");
    let pool = connect_raw(&database_path(dir.path()), false).await;
    assert_eq!(
        everything_manual::storage::migrations::applied_schema_version(&pool)
            .await
            .unwrap(),
        1,
        "check 不得应用迁移"
    );
    pool.close().await;

    // in-process 只读检查的枚举与 CLI 输出一致。
    match everything_manual::storage::db::inspect(dir.path())
        .await
        .unwrap()
    {
        DatabaseStatus::Pending { applied, program } => {
            assert_eq!((applied, program), (1, 7));
        }
        other => panic!("期望 Pending，实际：{other:?}"),
    }
}

#[tokio::test]
async fn init_creates_database_and_check_reports_ready_twice() {
    let dir = TestDir::new("init-check");
    let password = dir.join("pw.txt");
    write_restricted(&password, "test-password-123\n");

    let init = run_cli(
        dir.path(),
        &["init", "--data-dir", "data", "--password-file", "pw.txt"],
    );
    assert_eq!(init.status, 0, "{init:?}");
    assert!(
        database_path(&dir.join("data")).is_file(),
        "init 必须创建并迁移数据库（T03）：{init:?}"
    );
    assert!(init.stdout.contains("schema v7"), "{init:?}");

    for _ in 0..2 {
        let check = run_cli(dir.path(), &["check", "--data-dir", "data"]);
        assert_eq!(check.status, 0, "{check:?}");
        assert!(
            check.stdout.contains("数据库 schema：已就绪（v7"),
            "{check:?}"
        );
        assert!(check.stdout.contains("foreign_keys=ON"), "{check:?}");
    }

    // 幂等：两次 check 后迁移记录不重复（T19 起迁移集为 1/2/3/4/5/6/7）。
    let pool = connect_raw(&database_path(&dir.join("data")), false).await;
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM _sqlx_migrations").await,
        7
    );
    pool.close().await;

    // `serve` 也能在同一 data-dir 上自动迁移并启动（库比程序新时拒绝，见下一用例）。
    let serve = run_serve_and_stop(&dir);
    assert_eq!(serve.status, 0, "{serve:?}");
}

// ---------------------------------------------------------------------------
// 枚举值 ↔ SQL CHECK 约束的一致性（防止两边漂移）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn enum_sql_values_match_schema_check_constraints() {
    let dir = TestDir::new("enum-drift");
    init_structure(dir.path());
    let database = open(dir.path()).await;
    let pool = database.pool();

    let schema_sql = |table: &str| {
        let pool = pool.clone();
        let table = table.to_owned();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .expect("读取建表 SQL")
        }
    };

    let cases: [(&str, Vec<&str>); 12] = [
        (
            "jobs",
            [
                JobStatus::Queued,
                JobStatus::Running,
                JobStatus::WaitingProvider,
                JobStatus::RetryWait,
                JobStatus::NeedsInput,
                JobStatus::SubmissionUnknown,
                JobStatus::Succeeded,
                JobStatus::Failed,
                JobStatus::Cancelled,
            ]
            .iter()
            .map(|value| value.as_str())
            .collect(),
        ),
        (
            "job_stages",
            [
                StageKind::FreezeInputs,
                StageKind::ManualExtract,
                StageKind::ManualMerge,
                StageKind::TripoUpload,
                StageKind::TripoSubmit,
                StageKind::TripoPoll,
                StageKind::ModelDownload,
                StageKind::ModelValidate,
                StageKind::AssembleDraft,
            ]
            .iter()
            .map(|value| value.as_str())
            .collect(),
        ),
        (
            "provider_attempts",
            [
                SubmitState::Intent,
                SubmitState::Submitting,
                SubmitState::Accepted,
                SubmitState::Unknown,
                SubmitState::Failed,
            ]
            .iter()
            .map(|value| value.as_str())
            .collect(),
        ),
        (
            "cost_ledger",
            [
                LedgerState::Reserved,
                LedgerState::Settled,
                LedgerState::Released,
                LedgerState::Unknown,
            ]
            .iter()
            .map(|value| value.as_str())
            .collect(),
        ),
        (
            "cost_ledger",
            [ProviderKey::Tripo, ProviderKey::ManualAi]
                .iter()
                .map(|value| value.as_str())
                .collect(),
        ),
        (
            "cost_ledger",
            [Currency::CreditMinor, Currency::UsdMicros]
                .iter()
                .map(|value| value.as_str())
                .collect(),
        ),
        (
            "blobs",
            [
                BlobStorageState::Stored,
                BlobStorageState::Quarantined,
                BlobStorageState::Missing,
            ]
            .iter()
            .map(|value| value.as_str())
            .collect(),
        ),
        (
            "preparations",
            [PreparationState::Preparing, PreparationState::Ready]
                .iter()
                .map(|value| value.as_str())
                .collect(),
        ),
        (
            "photos",
            [
                PhotoView::Front,
                PhotoView::Left,
                PhotoView::Back,
                PhotoView::Right,
                PhotoView::Detail,
            ]
            .iter()
            .map(|value| value.as_str())
            .collect(),
        ),
        (
            "assets",
            [
                AssetPurpose::Document,
                AssetPurpose::Photo,
                AssetPurpose::PageImage,
                AssetPurpose::PageText,
                AssetPurpose::Model,
                AssetPurpose::ReleaseManifest,
            ]
            .iter()
            .map(|value| value.as_str())
            .collect(),
        ),
        (
            "model_revisions",
            [
                ModelValidationState::Pending,
                ModelValidationState::Validated,
                ModelValidationState::Rejected,
            ]
            .iter()
            .map(|value| value.as_str())
            .collect(),
        ),
        (
            "manual_drafts",
            [DraftStatus::NeedsReview, DraftStatus::Ready]
                .iter()
                .map(|value| value.as_str())
                .collect(),
        ),
    ];

    for (table, values) in cases {
        let sql = schema_sql(table).await;
        for value in values {
            let quoted = format!("'{value}'");
            assert!(
                sql.contains(&quoted),
                "{table} 的建表 SQL 不含枚举字面量 {quoted}（枚举与迁移可能漂移）：{sql}"
            );
        }
    }

    database.close().await;
}

// ---------------------------------------------------------------------------
// CLI 子进程工具（与 config_cli.rs 同一手法：清空环境变量 + 0600 密码文件）
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CliOutput {
    status: i32,
    stdout: String,
    stderr: String,
}

fn write_restricted(path: &Path, content: &str) {
    std::fs::write(path, content).expect("写受限文件");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let output = Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("二进制应可启动");
    let output = CliOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    };
    assert!(
        !output.stderr.contains("panicked at"),
        "错误路径不得 panic：{output:?}"
    );
    output
}

/// 启动 `serve`（端口随机），解析出监听地址后立即以 SIGTERM 停止。
fn run_serve_and_stop(dir: &TestDir) -> CliOutput {
    use std::io::{BufRead, BufReader};
    use std::process::Child;

    let mut child: Child = Command::new(BIN)
        .args(["serve", "--data-dir", "data", "--listen", "127.0.0.1:0"])
        .current_dir(dir.path())
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 serve");

    // 等待 `listening on http://...` 协议行（与 xtask smoke-bootstrap 相同）。
    let stdout = child.stdout.take().expect("取 stdout");
    let mut reader = BufReader::new(stdout);
    let mut captured = String::new();
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).expect("读取 serve 输出");
        if read == 0 {
            break;
        }
        captured.push_str(&line);
        if line.starts_with("listening on ") {
            break;
        }
    }
    assert!(
        captured.contains("listening on http://"),
        "serve 未输出监听行：{captured}"
    );

    // `listening on` 在 SIGTERM 处理器注册前打印：稍等片刻再发信号，
    // 避免测试测到"信号处理器尚未安装"的进程启动竞态（不是产品缺陷）。
    std::thread::sleep(std::time::Duration::from_millis(300));
    let status = terminate(&mut child);
    CliOutput {
        status,
        stdout: captured,
        stderr: String::new(),
    }
}

#[cfg(unix)]
fn terminate(child: &mut std::process::Child) -> i32 {
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return status.code().unwrap_or(-1);
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("SIGTERM 后未在 10 秒内退出");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(not(unix))]
fn terminate(child: &mut std::process::Child) -> i32 {
    let _ = child.kill();
    let _ = child.wait();
    -1
}
