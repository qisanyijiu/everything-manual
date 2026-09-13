//! 迁移内嵌与 schema 兼容门禁（REQ-004 / AC-007 / AC-008）。
//!
//! - `migrations/` 是持久 schema 的机器来源（ADR-009），**只追加**；SQL 变更通过
//!   `build.rs` 的 `rerun-if-changed` 触发重编译，随二进制内嵌（`migrate!` 宏）。
//! - 启动时自动检测并升级到程序支持版本：`applied < program` → 执行未应用迁移；
//!   `applied > program` → **拒绝打开**（[`StorageError::SchemaTooNew`]），不修改数据。
//! - 每条迁移由 SQLx 包在事务里执行（文件首行没有 `-- no-transaction`）：失败回滚，
//!   不留半升级状态；升级的原子性单位是"单条迁移"，失败点之前的迁移保持已提交。

use sqlx::migrate::Migrator;
use sqlx::{SqliteConnection, SqlitePool};

use super::error::StorageError;

/// 随二进制内嵌的迁移集合（`migrations/` 目录在编译期读入，运行时不读文件系统）。
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// 程序支持的 schema 版本 = 内嵌迁移的最大版本。
pub fn program_schema_version() -> i64 {
    MIGRATOR
        .migrations
        .last()
        .map(|migration| migration.version)
        .unwrap_or(0)
}

/// 库中已应用的 schema 版本（`_sqlx_migrations` 中最大的成功版本）。
///
/// 库不存在迁移表（全新库或空文件）时为 0。
pub async fn applied_schema_version(pool: &SqlitePool) -> Result<i64, StorageError> {
    let mut connection = pool.acquire().await?;
    applied_schema_version_conn(&mut connection).await
}

/// 单连接版本的 [`applied_schema_version`]（T20 备份快照读取用：
/// 备份只持有一条只读连接，不建立连接池）。
pub async fn applied_schema_version_conn(
    connection: &mut SqliteConnection,
) -> Result<i64, StorageError> {
    let has_table: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(&mut *connection)
    .await?;
    if has_table.is_none() {
        return Ok(0);
    }

    let dirty: Option<i64> =
        sqlx::query_scalar("SELECT MIN(version) FROM _sqlx_migrations WHERE success = 0")
            .fetch_one(&mut *connection)
            .await?;
    if let Some(version) = dirty {
        return Err(StorageError::MigrationInconsistent {
            detail: format!("迁移 v{version} 上次执行未完成（success=0），数据库可能处于中间状态"),
        });
    }

    let version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(&mut *connection)
    .await?;
    Ok(version)
}

/// schema 兼容门禁：库比程序新 → 拒绝（先于任何迁移执行，不写数据库）。
pub async fn ensure_compatible(pool: &SqlitePool) -> Result<(), StorageError> {
    let applied = applied_schema_version(pool).await?;
    let program = program_schema_version();
    if applied > program {
        return Err(StorageError::SchemaTooNew {
            database_version: applied,
            program_version: program,
        });
    }
    Ok(())
}

/// 应用所有未执行的迁移（自动升级旧 schema）。
///
/// `Migrator::run` 会先校验已应用迁移的 checksum：有人改过历史迁移文件时明确报错，
/// 不静默重放。
pub async fn migrate(pool: &SqlitePool) -> Result<(), StorageError> {
    ensure_compatible(pool).await?;
    MIGRATOR.run(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_migrations_are_ordered_and_versioned() {
        let versions: Vec<i64> = MIGRATOR
            .migrations
            .iter()
            .map(|migration| migration.version)
            .collect();
        assert!(!versions.is_empty(), "至少应内嵌一条迁移");
        let mut sorted = versions.clone();
        sorted.sort_unstable();
        assert_eq!(versions, sorted, "迁移必须按版本升序内嵌");
        assert_eq!(program_schema_version(), *versions.last().unwrap());
        for migration in MIGRATOR.migrations.iter() {
            assert!(
                !migration.sql.as_str().trim().is_empty(),
                "迁移 {} 内容为空",
                migration.version
            );
        }
    }
}
