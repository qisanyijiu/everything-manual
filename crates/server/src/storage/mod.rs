//! 持久化层：SQLite 连接、迁移与 repository（T03）。
//!
//! 对外合同：PRD REQ-004（数据库初始化与升级）与其 AC-007/AC-008；
//! 设计依据 architecture.md §6（WAL/FULL/busy_timeout/连接池上限 4/短事务）与
//! contracts.md §2（表、唯一键、外键与"必须保证"）。表结构由 `migrations/` 的 SQL
//! 定义（ADR-009：迁移是持久 schema 的机器来源），`crates/core` 的领域类型与之一一对应。
//!
//! 边界：
//! - **不获取 data-dir 排他锁**（T02 的 [`crate::config::datadir::DirLock`] 负责）；
//!   `init`/`serve`/`check` 在持锁后调用本模块。
//! - 不做业务规则（输入校验、状态机、报价、发布），只提供原语与 SQL 层不变量；
//!   业务规则由后续卡的服务层实现。
//! - 所有 SQL 使用静态文本 + bind；需要动态条件时用 `QueryBuilder`，不拼接用户字符串。

pub mod db;
pub mod error;
pub mod migrations;
pub mod repo;
pub mod tx;

pub use db::{
    BUSY_TIMEOUT, ConnectionSettings, DATABASE_FILE_NAME, Database, DatabaseStatus,
    MigrationReport, POOL_MAX_CONNECTIONS, database_path,
};
pub use error::StorageError;
pub use tx::{BEGIN_IMMEDIATE, begin_write, begin_write_pool};
