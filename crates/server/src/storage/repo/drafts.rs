//! `manual_drafts` 表的仓储原语（T15 / REQ-030；contracts.md §2/§3）。
//!
//! **边界**：只做持久化与 SQL 层不变量。组装内容（Part/Step/Evidence/Hotspot 语义）、
//! 缺项口径与审计在 `crate::drafts`（服务层）；发布不变量与 release 属 T19。
//!
//! 关键语义（QA 按此复核）：
//! - **每快照至多一份草稿**（0001 迁移的 `UNIQUE (snapshot_id)`）：`assemble_draft`
//!   的幂等（重启/重放/重试不重复创建）由该唯一键与 [`upsert_assembled`] 的
//!   "内容相同则不动" 规则共同保证；
//! - **聚合更新需要 If-Match**（contracts.md §2）：[`update_status`] 是 revision CAS，
//!   过期 → [`StorageError::RevisionConflict`]（HTTP 层映射 412）；
//! - **草稿不等于已发布版本**：本模块不写 `manual_releases`（发布属 T19）。

use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

use manual_core::domain::{DraftStatus, ManualDraft};
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 组装写入的输入（内容由服务层校验；这里只落库）。
#[derive(Debug, Clone, PartialEq)]
pub struct NewAssembledDraft {
    pub item_id: String,
    pub snapshot_id: String,
    /// 模型分支成功时指向 validated 的不可变版本；分支未完成时为 `None`（部分草稿）。
    pub model_revision_id: Option<String>,
    /// 版本化知识聚合（JSON 文本；由 `crate::drafts` 的类型序列化）。
    pub knowledge_json: String,
}

/// 组装结果。
#[derive(Debug, Clone, PartialEq)]
pub struct AssembledDraft {
    pub draft: ManualDraft,
    /// 是否新建了草稿行。
    pub created: bool,
    /// 内容是否发生变化（新建或覆盖；`false` = 命中已有草稿且内容相同，未写库）。
    pub changed: bool,
}

/// 按 id 读取草稿；不存在返回 `Ok(None)`。
pub async fn get(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<ManualDraft>, StorageError> {
    let row = sqlx::query(SELECT_DRAFT_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| draft_from_row(&row)).transpose()
}

/// 按快照读取草稿（每快照至多一份）。
///
/// 同时用于两处判定：`assemble_draft` 的幂等（已有草稿 → 更新而不是新建）与
/// 任务详情里的 `draftId`（父 job → 快照 → 草稿）。
pub async fn get_by_snapshot(
    conn: &mut SqliteConnection,
    snapshot_id: &str,
) -> Result<Option<ManualDraft>, StorageError> {
    let row = sqlx::query(SELECT_DRAFT_BY_SNAPSHOT_SQL)
        .bind(snapshot_id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| draft_from_row(&row)).transpose()
}

/// 组装草稿（幂等 upsert，按 `snapshot_id` 唯一）。
///
/// 规则：
/// - 不存在 → 插入（`revision = 1`、`status = needs_review`）；
/// - 已存在且 **内容相同**（`model_revision_id` 与 `knowledge_json` 都不变）→ 不动
///   （不递增 revision、不改 status）：重放/重跑不产生新版本；
/// - 已存在且内容变化 → 覆盖知识聚合、递增 revision，把 `status` 拉回
///   `needs_review`（内容变了，此前的"已复核"声明不再适用；这是保守方向，
///   发布不变量最终由 T19 的 publish 校验），并**清空 `review_json`**：
///   实体复核与 modelReview 都绑定具体内容/模型，内容重建后必须重新声明
///   （T19/AC-054「换模型清空 modelReview」在组装路径上的落点）。
pub async fn upsert_assembled(
    conn: &mut SqliteConnection,
    new: NewAssembledDraft,
    now: Timestamp,
) -> Result<AssembledDraft, StorageError> {
    let id = ids::new_id();
    let inserted: Option<String> = sqlx::query_scalar(
        "INSERT INTO manual_drafts \
             (id, item_id, snapshot_id, model_revision_id, revision, status, knowledge_json, review_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 1, 'needs_review', ?, NULL, ?, ?) \
         ON CONFLICT (snapshot_id) DO UPDATE SET \
             model_revision_id = excluded.model_revision_id, \
             knowledge_json = excluded.knowledge_json, \
             review_json = NULL, \
             status = 'needs_review', \
             revision = manual_drafts.revision + 1, \
             updated_at = excluded.updated_at \
         WHERE manual_drafts.model_revision_id IS NOT excluded.model_revision_id \
            OR manual_drafts.knowledge_json <> excluded.knowledge_json \
         RETURNING id",
    )
    .bind(&id)
    .bind(&new.item_id)
    .bind(&new.snapshot_id)
    .bind(&new.model_revision_id)
    .bind(&new.knowledge_json)
    .bind(now.as_millis())
    .bind(now.as_millis())
    .fetch_optional(&mut *conn)
    .await?;

    let changed = inserted.is_some();
    let created = inserted.as_deref() == Some(id.as_str());
    let draft = get_by_snapshot(conn, &new.snapshot_id)
        .await?
        .ok_or_else(|| StorageError::Database {
            detail: format!("组装后读取草稿失败：snapshot={}", new.snapshot_id),
        })?;
    Ok(AssembledDraft {
        draft,
        created,
        changed,
    })
}

/// 带 revision CAS 的状态更新（T15 的草稿 PATCH 最小面）。
///
/// 失败语义与 [`crate::storage::repo::items::update`] 一致：行不存在 → `NotFound`；
/// revision 过期 → `RevisionConflict`（带 `current_revision`）。
pub async fn update_status(
    conn: &mut SqliteConnection,
    id: &str,
    expected_revision: i64,
    status: DraftStatus,
    now: Timestamp,
) -> Result<ManualDraft, StorageError> {
    let changed = sqlx::query(
        "UPDATE manual_drafts SET status = ?, revision = revision + 1, updated_at = ? \
          WHERE id = ? AND revision = ?",
    )
    .bind(status.as_str())
    .bind(now.as_millis())
    .bind(id)
    .bind(expected_revision)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    if changed == 0 {
        return match current_revision(conn, id).await? {
            Some(current_revision) => Err(StorageError::RevisionConflict {
                entity: "manual_draft",
                id: id.to_owned(),
                current_revision,
            }),
            None => Err(StorageError::NotFound {
                entity: "manual_draft",
                id: id.to_owned(),
            }),
        };
    }
    get(conn, id).await?.ok_or_else(|| StorageError::Database {
        detail: format!("更新后读取草稿失败：{id}"),
    })
}

/// T19 受限字段更新：知识聚合（热点/视角）+ 复核覆盖层 + 状态，revision CAS。
///
/// 列的可选性（None = 不动该列）很重要：未变化的列必须保持**原字节**，
/// 否则 `serde_json::Value` 的序列化往返会重排键序，把"无变化"变成假变化
/// （组装幂等与客户端 ETag 都会受影响）。动态列用 `QueryBuilder`（T03 规则：
/// 不拼用户字符串）。`expected_revision` 是 `If-Match` 的具体 revision；
/// 过期 → `RevisionConflict`（HTTP 412，`details.currentRevision`）。
pub async fn update_content(
    conn: &mut SqliteConnection,
    id: &str,
    expected_revision: i64,
    knowledge_json: Option<&str>,
    review_json: Option<Option<&str>>,
    status: DraftStatus,
    now: Timestamp,
) -> Result<ManualDraft, StorageError> {
    let mut builder = QueryBuilder::<Sqlite>::new("UPDATE manual_drafts SET status = ");
    builder.push_bind(status.as_str().to_owned());
    builder.push(", revision = revision + 1, updated_at = ");
    builder.push_bind(now.as_millis());
    if let Some(json) = knowledge_json {
        builder.push(", knowledge_json = ");
        builder.push_bind(json.to_owned());
    }
    if let Some(json) = review_json {
        builder.push(", review_json = ");
        builder.push_bind(json.map(str::to_owned));
    }
    builder.push(" WHERE id = ");
    builder.push_bind(id.to_owned());
    builder.push(" AND revision = ");
    builder.push_bind(expected_revision);
    let changed = builder.build().execute(&mut *conn).await?.rows_affected();

    if changed == 0 {
        return match current_revision(conn, id).await? {
            Some(current_revision) => Err(StorageError::RevisionConflict {
                entity: "manual_draft",
                id: id.to_owned(),
                current_revision,
            }),
            None => Err(StorageError::NotFound {
                entity: "manual_draft",
                id: id.to_owned(),
            }),
        };
    }
    get(conn, id).await?.ok_or_else(|| StorageError::Database {
        detail: format!("更新后读取草稿失败：{id}"),
    })
}

/// 发布事务的聚合更新：只递增 revision（草稿内容不变）。
///
/// 为什么发布要动 draft revision：发布是**聚合根上的显式操作**（contracts §1 把发布
/// 与 PATCH 并列要求 If-Match）。递增后，两个并发发布（或"发布前的编辑"竞态）中
/// 的第二个请求必然拿到过期的 If-Match → 412，与 AC-056「并发修改返回 412」一致。
pub async fn bump_revision_for_publish(
    conn: &mut SqliteConnection,
    id: &str,
    expected_revision: i64,
    now: Timestamp,
) -> Result<ManualDraft, StorageError> {
    let changed = sqlx::query(
        "UPDATE manual_drafts SET revision = revision + 1, updated_at = ? WHERE id = ? AND revision = ?",
    )
    .bind(now.as_millis())
    .bind(id)
    .bind(expected_revision)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if changed == 0 {
        return match current_revision(conn, id).await? {
            Some(current_revision) => Err(StorageError::RevisionConflict {
                entity: "manual_draft",
                id: id.to_owned(),
                current_revision,
            }),
            None => Err(StorageError::NotFound {
                entity: "manual_draft",
                id: id.to_owned(),
            }),
        };
    }
    get(conn, id).await?.ok_or_else(|| StorageError::Database {
        detail: format!("发布后读取草稿失败：{id}"),
    })
}

/// 某物品最近一份草稿（跨快照；热点 stale 继承的来源）。
pub async fn latest_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
) -> Result<Option<ManualDraft>, StorageError> {
    let row = sqlx::query(
        "SELECT id, item_id, snapshot_id, model_revision_id, revision, status, \
                knowledge_json, review_json, created_at, updated_at \
           FROM manual_drafts WHERE item_id = ? ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| draft_from_row(&row)).transpose()
}

/// 某物品的草稿数（测试与诊断用；MVP 每快照一份）。
pub async fn count_for_item(
    conn: &mut SqliteConnection,
    item_id: &str,
) -> Result<i64, StorageError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM manual_drafts WHERE item_id = ?")
        .bind(item_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(count)
}

async fn current_revision(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<i64>, StorageError> {
    let revision: Option<i64> =
        sqlx::query_scalar("SELECT revision FROM manual_drafts WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(revision)
}

/// 列清单与 [`draft_from_row`] 对应（静态 SQL，无拼接）。
const SELECT_DRAFT_SQL: &str = "SELECT id, item_id, snapshot_id, model_revision_id, revision, status, \
        knowledge_json, review_json, created_at, updated_at \
   FROM manual_drafts WHERE id = ?";

const SELECT_DRAFT_BY_SNAPSHOT_SQL: &str = "SELECT id, item_id, snapshot_id, model_revision_id, revision, status, \
        knowledge_json, review_json, created_at, updated_at \
   FROM manual_drafts WHERE snapshot_id = ?";

fn json_column(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<serde_json::Value, StorageError> {
    let text: String = row.try_get(column)?;
    serde_json::from_str(&text).map_err(|error| StorageError::Database {
        detail: format!("manual_drafts.{column} 不是合法 JSON：{error}"),
    })
}

fn optional_json_column(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<serde_json::Value>, StorageError> {
    let text: Option<String> = row.try_get(column)?;
    match text {
        Some(text) => {
            serde_json::from_str(&text)
                .map(Some)
                .map_err(|error| StorageError::Database {
                    detail: format!("manual_drafts.{column} 不是合法 JSON：{error}"),
                })
        }
        None => Ok(None),
    }
}

fn draft_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ManualDraft, StorageError> {
    let status: String = row.try_get("status")?;
    let status = match status.as_str() {
        "needs_review" => DraftStatus::NeedsReview,
        "ready" => DraftStatus::Ready,
        other => {
            return Err(StorageError::Database {
                detail: format!("manual_drafts.status 取值未知：{other}"),
            });
        }
    };
    Ok(ManualDraft {
        id: row.try_get("id")?,
        item_id: row.try_get("item_id")?,
        snapshot_id: row.try_get("snapshot_id")?,
        model_revision_id: row.try_get("model_revision_id")?,
        revision: row.try_get("revision")?,
        status,
        knowledge_json: json_column(row, "knowledge_json")?,
        review_json: optional_json_column(row, "review_json")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
        updated_at: Timestamp::from_millis(row.try_get("updated_at")?),
    })
}
