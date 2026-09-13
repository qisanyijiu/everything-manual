//! `admins` 的仓储原语：单管理员的创建、读取与口令哈希更新（T04）。
//!
//! 边界：
//! - 只做持久化；口令的 Argon2 哈希与校验在 [`crate::http::auth::password`]（不在此处
//!   依赖任何哈希实现，便于测试注入已知 PHC 字符串）；
//! - **无默认密码**：插入必须显式提供非空哈希（ADMIN 行不存在时登录必然失败）；
//! - 单管理员：`get_single` 读取任意一行（MVP 只有一个管理员，见 PRD §5.1）。

use manual_core::domain::Admin;
use manual_core::ids;
use manual_core::timestamps::Timestamp;
use sqlx::{Row, SqliteConnection};

use crate::storage::error::StorageError;

/// 读取唯一的管理员；未初始化时为 `None`。
pub async fn get_single(conn: &mut SqliteConnection) -> Result<Option<Admin>, StorageError> {
    let row = sqlx::query("SELECT id, password_hash, created_at, updated_at FROM admins LIMIT 1")
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| admin_from_row(&row)).transpose()
}

/// 创建管理员（`init` 首次运行时调用）。哈希由调用方按 Argon2id 计算。
pub async fn insert(
    conn: &mut SqliteConnection,
    password_hash: &str,
) -> Result<Admin, StorageError> {
    let now = Timestamp::now();
    let admin = Admin {
        id: ids::new_id(),
        password_hash: password_hash.to_owned(),
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO admins (id, password_hash, created_at, updated_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&admin.id)
    .bind(&admin.password_hash)
    .bind(admin.created_at.as_millis())
    .bind(admin.updated_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(admin)
}

/// 更新管理员口令哈希（再次 `init` 即重置密码路径）。
///
/// 调用方负责同时撤销该管理员的全部既有会话（`init` 在 `commands::run_init` 中完成）。
pub async fn update_password(
    conn: &mut SqliteConnection,
    admin_id: &str,
    password_hash: &str,
) -> Result<Admin, StorageError> {
    let now = Timestamp::now();
    let changed = sqlx::query("UPDATE admins SET password_hash = ?, updated_at = ? WHERE id = ?")
        .bind(password_hash)
        .bind(now.as_millis())
        .bind(admin_id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    if changed == 0 {
        return Err(StorageError::NotFound {
            entity: "admin",
            id: admin_id.to_owned(),
        });
    }
    get_single(conn)
        .await?
        .ok_or_else(|| StorageError::Database {
            detail: "更新管理员后读取失败".to_owned(),
        })
}

fn admin_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Admin, StorageError> {
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;
    Ok(Admin {
        id: row.try_get("id")?,
        password_hash: row.try_get("password_hash")?,
        created_at: Timestamp::from_millis(created_at),
        updated_at: Timestamp::from_millis(updated_at),
    })
}
