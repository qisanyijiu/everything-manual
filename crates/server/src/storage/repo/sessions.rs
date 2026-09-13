//! `sessions` 的仓储原语：创建、按 token 哈希查找有效会话、撤销与清理（T04）。
//!
//! 约定（contracts.md §2）：
//! - **会话明文 token 只返回 cookie，不落库/日志**：本表只存 [`session_token_hash`]；
//! - `csrf_hash` 同样只存哈希；
//! - 过期与撤销都是**软状态**：`revoked_at` 置位、`expires_at` 到期即视为无效，
//!   不物理删除（留痕；清理只删除早已过期的行）；
//! - 绝对有效期（A-05，默认 7 天）：本卡不做滑动续期。
//!
//! [`session_token_hash`]: manual_core::domain::Session::session_token_hash

use manual_core::domain::Session;
use manual_core::ids;
use manual_core::timestamps::Timestamp;
use sqlx::{Row, SqliteConnection};

use crate::storage::error::StorageError;

/// 创建会话（token 与 csrf 已在调用方哈希；明文不进入本层）。
#[derive(Debug, Clone)]
pub struct NewSession {
    pub admin_id: String,
    pub session_token_hash: String,
    pub csrf_hash: String,
    pub expires_at: Timestamp,
}

/// 写入新会话行。
pub async fn create(conn: &mut SqliteConnection, new: NewSession) -> Result<Session, StorageError> {
    let created_at = Timestamp::now();
    let session = Session {
        id: ids::new_id(),
        admin_id: new.admin_id,
        session_token_hash: new.session_token_hash,
        csrf_hash: new.csrf_hash,
        created_at,
        expires_at: new.expires_at,
        revoked_at: None,
    };
    sqlx::query(
        "INSERT INTO sessions (id, admin_id, session_token_hash, csrf_hash, created_at, expires_at, revoked_at) \
         VALUES (?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&session.id)
    .bind(&session.admin_id)
    .bind(&session.session_token_hash)
    .bind(&session.csrf_hash)
    .bind(session.created_at.as_millis())
    .bind(session.expires_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(session)
}

/// 按 token 哈希查找**当前有效**的会话：未撤销且未过期。
///
/// 无效状态（已撤销/已过期）与不存在的 token 都返回 `Ok(None)`——上层统一 401，
/// 不区分原因（不给探测者额外信息）。
pub async fn find_active(
    conn: &mut SqliteConnection,
    session_token_hash: &str,
    now: Timestamp,
) -> Result<Option<Session>, StorageError> {
    let row = sqlx::query(
        "SELECT id, admin_id, session_token_hash, csrf_hash, created_at, expires_at, revoked_at \
           FROM sessions \
          WHERE session_token_hash = ? AND revoked_at IS NULL AND expires_at > ?",
    )
    .bind(session_token_hash)
    .bind(now.as_millis())
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| session_from_row(&row)).transpose()
}

/// 撤销单个会话（登出）。已撤销/不存在的会话返回 `false`（登出保持幂等）。
pub async fn revoke(
    conn: &mut SqliteConnection,
    session_id: &str,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let changed =
        sqlx::query("UPDATE sessions SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
            .bind(now.as_millis())
            .bind(session_id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    Ok(changed > 0)
}

/// 撤销某管理员的全部有效会话（重置密码等敏感操作后强制重新登录）。
pub async fn revoke_all_for_admin(
    conn: &mut SqliteConnection,
    admin_id: &str,
    now: Timestamp,
) -> Result<u64, StorageError> {
    let changed =
        sqlx::query("UPDATE sessions SET revoked_at = ? WHERE admin_id = ? AND revoked_at IS NULL")
            .bind(now.as_millis())
            .bind(admin_id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    Ok(changed)
}

/// 删除过期已久的会话行（仅清理；有效性判定始终以 `find_active` 为准）。
pub async fn purge_expired(
    conn: &mut SqliteConnection,
    now: Timestamp,
) -> Result<u64, StorageError> {
    let changed = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now.as_millis())
        .execute(&mut *conn)
        .await?
        .rows_affected();
    Ok(changed)
}

fn session_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Session, StorageError> {
    let created_at: i64 = row.try_get("created_at")?;
    let expires_at: i64 = row.try_get("expires_at")?;
    let revoked_at: Option<i64> = row.try_get("revoked_at")?;
    Ok(Session {
        id: row.try_get("id")?,
        admin_id: row.try_get("admin_id")?,
        session_token_hash: row.try_get("session_token_hash")?,
        csrf_hash: row.try_get("csrf_hash")?,
        created_at: Timestamp::from_millis(created_at),
        expires_at: Timestamp::from_millis(expires_at),
        revoked_at: revoked_at.map(Timestamp::from_millis),
    })
}
