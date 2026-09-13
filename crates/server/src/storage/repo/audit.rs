//! `audit_events` 表的仓储原语（T10 起的最小写入面）。
//!
//! **边界**：T10 只用它记录执行器必须留痕的事实（例如 `provider_attempts.remote_task_id`
//! 冲突"停机告警"）。完整的审计事件面（费用确认、人工事实修改、发布、重试、对账）
//! 属 T11/T15/T19；本模块只提供 `record`，不定义业务动作枚举——`action`/`result`
//! 由调用方给出常量字符串，避免在这里发明第二套语义。
//!
//! 记录内容只放必要摘要：不保存密钥、完整资料、签名 URL 或堆栈。

use sqlx::SqliteConnection;

use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 一条审计事件（`metadata_json` 为已序列化的 JSON 文本，可为 `None`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAuditEvent {
    pub entity_type: String,
    pub entity_id: String,
    /// 管理员 id，或 `system`（服务器动作）。
    pub actor: Option<String>,
    pub action: String,
    pub result: String,
    pub metadata_json: Option<String>,
}

/// 写入一条审计事件。
pub async fn record(
    conn: &mut SqliteConnection,
    new: NewAuditEvent,
    now: Timestamp,
) -> Result<String, StorageError> {
    let id = ids::new_id();
    sqlx::query(
        "INSERT INTO audit_events \
             (id, entity_type, entity_id, actor, action, result, metadata_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&new.entity_type)
    .bind(&new.entity_id)
    .bind(&new.actor)
    .bind(&new.action)
    .bind(&new.result)
    .bind(&new.metadata_json)
    .bind(now.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(id)
}
