//! SQLite 连接、连接设置与打开/校验。
//!
//! 连接设置是硬约束（architecture.md §6、PRD §5.4）：
//! - `journal_mode=WAL`（要求本地文件系统；不支持 NFS/共享盘多实例，A-09）；
//! - `synchronous=FULL`（首版默认，崩溃一致性优先）；
//! - `busy_timeout=5s`（短事务重试窗口，而不是立即报 SQLITE_BUSY）；
//! - `foreign_keys=ON`（外键拒绝非法引用）；
//! - 连接池上限 4：事务短小，不跨 HTTP 请求持有事务。
//!
//! data-dir 排他锁由 `config::datadir::DirLock` 负责（T02）；本模块**不获取锁**，
//! 调用方（`init`/`serve`/`check`）必须自行保证"持锁后打开数据库"的顺序。

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use super::error::StorageError;
use super::migrations;

/// 数据库文件名（architecture.md §6：data-dir 内的 `manual.sqlite3`）。
pub const DATABASE_FILE_NAME: &str = "manual.sqlite3";
/// 连接池上限（architecture.md §6）。
pub const POOL_MAX_CONNECTIONS: u32 = 4;
/// `busy_timeout`（architecture.md §6）。
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// data-dir 中的数据库路径。
pub fn database_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DATABASE_FILE_NAME)
}

/// 连接层实际生效的设置（供 `check` 输出与测试断言）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSettings {
    pub journal_mode: String,
    pub synchronous: i64,
    pub busy_timeout_ms: i64,
    pub foreign_keys: bool,
}

impl ConnectionSettings {
    /// 是否满足合同要求（WAL / FULL=2 / 5000ms / ON）。
    pub fn meets_contract(&self) -> bool {
        self.journal_mode.eq_ignore_ascii_case("wal")
            && self.synchronous == 2
            && self.busy_timeout_ms == BUSY_TIMEOUT.as_millis() as i64
            && self.foreign_keys
    }

    /// 一行摘要（`check` 输出用，不含敏感信息）。
    pub fn summary(&self) -> String {
        format!(
            "WAL={} synchronous={} foreign_keys={} busy_timeout={}ms",
            self.journal_mode,
            self.synchronous,
            if self.foreign_keys { "ON" } else { "OFF" },
            self.busy_timeout_ms
        )
    }
}

/// `check` 的数据库状态（只读检查结果；不创建、不迁移）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseStatus {
    /// 数据库尚未建立（`init` 或首次 `serve` 时创建并迁移）。
    Missing { program_version: i64 },
    /// 已就绪：已应用版本 = 程序版本。
    Ready {
        version: i64,
        settings: ConnectionSettings,
    },
    /// 旧 schema：`serve`/`init` 启动时会自动迁移（本命令不修改数据）。
    Pending { applied: i64, program: i64 },
}

/// 数据库句柄（连接池 + data-dir）。持有它即保持连接；`close` 显式关闭。
///
/// `Clone` 只克隆连接池句柄（`SqlitePool` 内部是 Arc）：`serve` 把同一数据库交给
/// 应用状态（路由）并保留一份用于退出时关闭。
#[derive(Debug, Clone)]
pub struct Database {
    pool: SqlitePool,
    data_dir: PathBuf,
}

/// 打开数据库时实际发生的 schema 迁移（`serve`/`init` 打印"升级前请备份"提示的依据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    /// 打开前库中的 schema 版本（0 = 全新库）。
    pub from: i64,
    /// 打开后库中的 schema 版本（= 程序支持版本）。
    pub to: i64,
    /// 是否发生了**旧库升级**（from ≥ 1 且 from < to；全新库不算升级）。
    pub upgraded: bool,
}

impl Database {
    /// 打开（必要时创建）data-dir 中的数据库，并把 schema 自动迁移到程序支持版本。
    ///
    /// 顺序：连接（WAL/FULL/busy_timeout/foreign_keys）→ schema 兼容门禁（库比程序新
    /// 则拒绝，不修改数据）→ 执行未应用迁移（事务内）。调用方须已持有 data-dir 排他锁。
    pub async fn open_and_migrate(data_dir: &Path) -> Result<Self, StorageError> {
        Ok(Self::open_and_migrate_reporting(data_dir).await?.0)
    }

    /// 同 [`Self::open_and_migrate`]，另返回迁移前后版本（T20：升级提示与审计）。
    pub async fn open_and_migrate_reporting(
        data_dir: &Path,
    ) -> Result<(Self, MigrationReport), StorageError> {
        let pool = connect(data_dir, true).await?;
        let from = match migrations::applied_schema_version(&pool).await {
            Ok(version) => version,
            Err(error) => {
                pool.close().await;
                return Err(error);
            }
        };
        if let Err(error) = migrations::migrate(&pool).await {
            pool.close().await;
            return Err(error);
        }
        // 迁移会创建/写 WAL，可能新建边车文件：迁移后再收紧一次权限。
        restrict_file_permissions(&database_path(data_dir));
        let to = migrations::program_schema_version();
        let database = Self {
            pool,
            data_dir: data_dir.to_path_buf(),
        };
        Ok((
            database,
            MigrationReport {
                from,
                to,
                upgraded: from >= 1 && from < to,
            },
        ))
    }

    /// 只打开已有数据库：不创建、不迁移；库文件不存在时返回 `Ok(None)`。
    ///
    /// 仍执行 schema 兼容门禁（库比程序新 → [`StorageError::SchemaTooNew`]），
    /// 供 `check` 使用。
    pub async fn open_existing(data_dir: &Path) -> Result<Option<Self>, StorageError> {
        if !database_path(data_dir).is_file() {
            return Ok(None);
        }
        let pool = connect(data_dir, false).await?;
        if let Err(error) = migrations::ensure_compatible(&pool).await {
            pool.close().await;
            return Err(error);
        }
        Ok(Some(Self {
            pool,
            data_dir: data_dir.to_path_buf(),
        }))
    }

    /// 连接池（repository 与后续卡的任务执行器共用）。
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// data-dir 路径。
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 程序支持的 schema 版本。
    pub fn program_schema_version(&self) -> i64 {
        migrations::program_schema_version()
    }

    /// 库中已应用的 schema 版本。
    pub async fn applied_schema_version(&self) -> Result<i64, StorageError> {
        migrations::applied_schema_version(&self.pool).await
    }

    /// 读取连接层实际生效的设置（合同自检与测试断言）。
    pub async fn connection_settings(&self) -> Result<ConnectionSettings, StorageError> {
        read_connection_settings(&self.pool).await
    }

    /// 显式关闭连接池（测试与 CLI 退出前使用）。
    pub async fn close(self) {
        self.pool.close().await;
    }
}

/// 打开连接池（`create_if_missing=false` 时库文件必须已存在，不会静默创建）。
async fn connect(data_dir: &Path, create_if_missing: bool) -> Result<SqlitePool, StorageError> {
    let path = database_path(data_dir);
    if !create_if_missing && !path.is_file() {
        return Err(StorageError::Database {
            detail: format!("数据库文件不存在：{}", path.display()),
        });
    }
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(create_if_missing)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(BUSY_TIMEOUT)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(POOL_MAX_CONNECTIONS)
        .connect_with(options)
        .await
        .map_err(|error| StorageError::Database {
            detail: format!("无法打开数据库 {}：{error}", path.display()),
        })?;
    restrict_file_permissions(&path);
    Ok(pool)
}

/// 把数据库文件及其 WAL/SHM 边车收紧到 0600（与 T02 的锁文件/日志、0700 data-dir 一致）。
///
/// 保护边界仍是 0700 的 data-dir；这里只是不依赖 umask。设置失败被忽略
/// （不支持 POSIX 权限的文件系统不能让服务启动失败，T02 同样处理锁文件）。
#[cfg(unix)]
fn restrict_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    for candidate in sidecar_paths(path) {
        if candidate.exists() {
            let _ = std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600));
        }
    }
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) {}

/// 数据库主文件与 SQLite WAL 模式的边车文件（`-wal` / `-shm`）。
fn sidecar_paths(path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![path.to_path_buf()];
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        paths.push(PathBuf::from(name));
    }
    paths
}

/// 读取当前连接上的 pragma 值。
pub async fn read_connection_settings(
    pool: &SqlitePool,
) -> Result<ConnectionSettings, StorageError> {
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(pool)
        .await?;
    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(pool)
        .await?;
    let busy_timeout_ms: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(pool)
        .await?;
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await?;
    Ok(ConnectionSettings {
        journal_mode,
        synchronous,
        busy_timeout_ms,
        foreign_keys: foreign_keys != 0,
    })
}

/// `check` 的只读检查：库不存在 → [`DatabaseStatus::Missing`]；存在 → 校验兼容性并报告
/// 迁移状态。**不创建数据库、不应用迁移、不修改数据。**
pub async fn inspect(data_dir: &Path) -> Result<DatabaseStatus, StorageError> {
    let program = migrations::program_schema_version();
    let Some(database) = Database::open_existing(data_dir).await? else {
        return Ok(DatabaseStatus::Missing {
            program_version: program,
        });
    };
    let applied = database.applied_schema_version().await?;
    let status = if applied >= program {
        DatabaseStatus::Ready {
            version: applied,
            settings: database.connection_settings().await?,
        }
    } else {
        DatabaseStatus::Pending { applied, program }
    };
    database.close().await;
    Ok(status)
}
