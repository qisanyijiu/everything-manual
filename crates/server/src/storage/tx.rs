//! 写事务开启（`BEGIN IMMEDIATE`）。
//!
//! 背景（BUG-006，2026-09-12）：WAL 下 deferred 事务（sqlx 的 `begin()` /
//! `pool.begin()` 发出裸 `BEGIN`）在**先读后写**时无法安全升级：第一条读语句建立快照，
//! 随后的写需要写锁；若此时存在活跃写者（或快照已落后于 WAL），SQLite **立即**返回
//! `SQLITE_BUSY` / `SQLITE_BUSY_SNAPSHOT`（sqlx 报 `database is locked`），
//! **不等待** `busy_timeout`。表现为 HTTP 500，而合同承诺的并发冲突语义
//! （409/412/422，例如"同视图第二张" → 422 `viewOccupied`）在并发窗口退化为通用 500。
//! QA 复现：`POST /photos` 400 并发 → 236×500；同视图 4 并发 28/40 = 500。
//!
//! 统一做法：**每个可能写库的事务都用 `BEGIN IMMEDIATE` 先取写锁**。
//! - 写锁在事务开始时取，`busy_timeout=5s`（[`super::BUSY_TIMEOUT`]）正常生效；
//! - 事务仍是短事务（不跨 HTTP、不跨外部调用、不跨连接池获取），不降低并发上限——
//!   SQLite 本来同一时刻只允许一个写者；
//! - 首条语句就是 INSERT/UPDATE 的"纯写"事务同样使用：语义不变，且避免后续语句重排
//!   时静默退化回"读→写升级"（本缺陷的根因就是这种顺序依赖）。
//!
//! 例外：**只读**事务继续用 `begin()`（不取写锁），本仓库目前所有 `begin()` 都是写事务。
//! 依据：<https://www.sqlite.org/lang_transaction.html>（DEFERRED/IMMEDIATE 语义）与
//! <https://www.sqlite.org/wal.html>（SQLITE_BUSY_SNAPSHOT 不可等待）。

use sqlx::{Connection as _, Sqlite, SqliteConnection, SqlitePool, Transaction};

/// `BEGIN IMMEDIATE`：事务一开始就取写锁（等待窗口 = 连接的 `busy_timeout`）。
pub const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

/// 在**连接**上开启写事务（`BEGIN IMMEDIATE`）。
pub async fn begin_write(
    conn: &mut SqliteConnection,
) -> Result<Transaction<'_, Sqlite>, sqlx::Error> {
    conn.begin_with(BEGIN_IMMEDIATE).await
}

/// 在**连接池**上开启写事务（`BEGIN IMMEDIATE`；事务借用池直到提交/回滚）。
pub async fn begin_write_pool(pool: &SqlitePool) -> Result<Transaction<'_, Sqlite>, sqlx::Error> {
    pool.begin_with(BEGIN_IMMEDIATE).await
}
