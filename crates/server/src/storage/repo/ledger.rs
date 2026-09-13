//! `cost_ledger` 表的仓储原语（T11 / contracts.md §4：预留／结算／释放）。
//!
//! **边界**：只做持久化与状态转换的 SQL 条件更新；转换规则来自纯逻辑
//! [`manual_core::cost::next_ledger_state`]（单一来源）。业务编排（建单时预留、
//! 执行器结算/释放、unknown 保留）在 `crate::generation` / `crate::jobs`。
//!
//! 关键语义（QA 按此复核）：
//! - **预留与建单同事务**：调用方在同一个短事务里调用 [`reserve`]；
//! - **幂等**：重复的结算/释放/未决事件不重复写入；同值结算返回
//!   [`LedgerOutcome::Idempotent`]，冲突（已有不同 actual）返回
//!   [`LedgerOutcome::Rejected`]，**不覆盖已落账事实**；
//! - **unknown 保留预留**：`state = unknown` 且 `actual` 保持 NULL
//!   （0001 的 CHECK 也禁止把 actual 填成非 NULL 冒充结算）；
//! - **同一快照 + 供应商至多一笔进行中预留**：0006 的部分唯一索引兜底（并发重试）。

use sqlx::{Row, SqliteConnection};

use manual_core::cost::LedgerEvent;
use manual_core::domain::{CostLedgerEntry, Currency, LedgerState, ProviderKey};
use manual_core::ids;
use manual_core::timestamps::Timestamp;

use crate::storage::error::StorageError;

/// 一笔预留在快照上的输入（金额 = 服务器计算的保守上界，不接受前端数值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReservation {
    pub snapshot_id: String,
    pub provider: ProviderKey,
    pub currency: Currency,
    pub reserved: i64,
    pub price_version: String,
}

/// 转换结果（幂等 / 已生效 / 被拒绝）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerOutcome {
    /// 状态已按事件推进（本次写入）。
    Applied,
    /// 重复事件：状态与金额已与目标一致，未重复写入。
    Idempotent,
    /// 非法转换（例如释放已结算的预留、把已结算的 actual 改成别的值）：
    /// 调用方必须记录并拒绝，**不静默忽略**。
    Rejected { reason: String },
}

/// 插入一笔预留（`state = reserved`、`actual = NULL`）。
pub async fn reserve(
    conn: &mut SqliteConnection,
    new: NewReservation,
    now: Timestamp,
) -> Result<CostLedgerEntry, StorageError> {
    let entry = CostLedgerEntry {
        id: ids::new_id(),
        snapshot_id: new.snapshot_id,
        attempt_id: None,
        provider: new.provider,
        currency: new.currency,
        reserved: new.reserved,
        actual: None,
        state: LedgerState::Reserved,
        price_version: new.price_version,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO cost_ledger \
             (id, snapshot_id, attempt_id, provider, currency, reserved, actual, state, price_version, \
              created_at, updated_at) \
         VALUES (?, ?, NULL, ?, ?, ?, NULL, 'reserved', ?, ?, ?)",
    )
    .bind(&entry.id)
    .bind(&entry.snapshot_id)
    .bind(entry.provider.as_str())
    .bind(entry.currency.as_str())
    .bind(entry.reserved)
    .bind(&entry.price_version)
    .bind(entry.created_at.as_millis())
    .bind(entry.updated_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(entry)
}

/// 按 id 读取账本条目。
pub async fn get(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<CostLedgerEntry>, StorageError> {
    let row = sqlx::query(SELECT_LEDGER_SQL)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|row| entry_from_row(&row)).transpose()
}

/// 列出某快照的全部账本条目（费用展示与测试断言）。
pub async fn list_for_snapshot(
    conn: &mut SqliteConnection,
    snapshot_id: &str,
) -> Result<Vec<CostLedgerEntry>, StorageError> {
    let rows = sqlx::query(
        "SELECT id, snapshot_id, attempt_id, provider, currency, reserved, actual, state, \
                price_version, created_at, updated_at \
           FROM cost_ledger WHERE snapshot_id = ? ORDER BY provider, created_at, id",
    )
    .bind(snapshot_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(entry_from_row).collect()
}

/// 把 attempt 关联到账本条目（执行器建 attempt 后调用；重复关联同一 attempt 幂等）。
pub async fn attach_attempt(
    conn: &mut SqliteConnection,
    entry_id: &str,
    attempt_id: &str,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let changed = sqlx::query(
        "UPDATE cost_ledger SET attempt_id = ?, updated_at = ? \
          WHERE id = ? AND (attempt_id IS NULL OR attempt_id = ?)",
    )
    .bind(attempt_id)
    .bind(now.as_millis())
    .bind(entry_id)
    .bind(attempt_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

/// 按实际金额结算（`reserved|unknown → settled`；`actual` 显式落库）。
///
/// 幂等：已是 `settled` 且 `actual` 相同 → [`LedgerOutcome::Idempotent`]；
/// 已是 `settled` 但金额不同 → [`LedgerOutcome::Rejected`]（不覆盖事实）。
pub async fn settle(
    conn: &mut SqliteConnection,
    entry_id: &str,
    actual_minor: i64,
    now: Timestamp,
) -> Result<LedgerOutcome, StorageError> {
    transition(
        conn,
        entry_id,
        LedgerEvent::Settle { actual_minor },
        actual_minor,
        now,
    )
    .await
}

/// 释放预留（`reserved|unknown → released`）。
///
/// **自动路径只对"明确未计费"的预留调用**；对 `unknown` 的释放必须来自管理员
/// 对账决定（recordNoTask 等，T15）——见 [`manual_core::cost::LedgerEvent`] 注释。
pub async fn release(
    conn: &mut SqliteConnection,
    entry_id: &str,
    now: Timestamp,
) -> Result<LedgerOutcome, StorageError> {
    transition(conn, entry_id, LedgerEvent::Release, 0, now).await
}

/// 标记结果未知（`reserved → unknown`；保留预留、`actual` 保持 NULL）。
pub async fn mark_unknown(
    conn: &mut SqliteConnection,
    entry_id: &str,
    now: Timestamp,
) -> Result<LedgerOutcome, StorageError> {
    transition(conn, entry_id, LedgerEvent::MarkUnknown, 0, now).await
}

/// 共享的状态转换实现：先读当前状态按 [`next_ledger_state`] 判定，再条件更新。
async fn transition(
    conn: &mut SqliteConnection,
    entry_id: &str,
    event: LedgerEvent,
    actual_minor: i64,
    now: Timestamp,
) -> Result<LedgerOutcome, StorageError> {
    let Some(current) = get(conn, entry_id).await? else {
        return Err(StorageError::NotFound {
            entity: "cost_ledger",
            id: entry_id.to_owned(),
        });
    };
    let Some(target) = manual_core::cost::next_ledger_state(current.state, event) else {
        // 幂等读回：终态上的重复事件，值一致才算幂等。
        let idempotent = match (current.state, event) {
            (LedgerState::Settled, LedgerEvent::Settle { actual_minor }) => {
                current.actual == Some(actual_minor)
            }
            (LedgerState::Released, LedgerEvent::Release) => true,
            (LedgerState::Unknown, LedgerEvent::MarkUnknown) => true,
            _ => false,
        };
        return Ok(if idempotent {
            LedgerOutcome::Idempotent
        } else {
            LedgerOutcome::Rejected {
                reason: format!(
                    "账本条目 {} 处于 {}，不接受该事件（不覆盖已落账事实）",
                    current.id,
                    current.state.as_str()
                ),
            }
        });
    };

    // 目标状态与当前一致（例如 unknown 上重复 MarkUnknown）：不再写入，按幂等返回。
    if target == current.state {
        return Ok(LedgerOutcome::Idempotent);
    }

    // 目标状态与金额一起写；条件更新保证并发下只有一个生效。
    let changed = sqlx::query(
        "UPDATE cost_ledger SET state = ?, actual = ?, updated_at = ? \
          WHERE id = ? AND state = ?",
    )
    .bind(target.as_str())
    .bind(match event {
        LedgerEvent::Settle { .. } => Some(actual_minor),
        LedgerEvent::Release | LedgerEvent::MarkUnknown => None,
    })
    .bind(now.as_millis())
    .bind(entry_id)
    .bind(current.state.as_str())
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if changed > 0 {
        return Ok(LedgerOutcome::Applied);
    }
    // 竞争：重读后按当前状态再判一次（幂等或拒绝，不盲目重试写入）。
    let Some(after) = get(conn, entry_id).await? else {
        return Err(StorageError::NotFound {
            entity: "cost_ledger",
            id: entry_id.to_owned(),
        });
    };
    let idempotent = after.state == target
        && match event {
            LedgerEvent::Settle { actual_minor } => after.actual == Some(actual_minor),
            _ => true,
        };
    Ok(if idempotent {
        LedgerOutcome::Idempotent
    } else {
        LedgerOutcome::Rejected {
            reason: format!(
                "账本条目 {} 的并发转换未生效（当前 {}）",
                after.id,
                after.state.as_str()
            ),
        }
    })
}

const SELECT_LEDGER_SQL: &str = "SELECT id, snapshot_id, attempt_id, provider, currency, reserved, actual, \
        state, price_version, created_at, updated_at \
   FROM cost_ledger WHERE id = ?";

fn entry_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<CostLedgerEntry, StorageError> {
    let provider: String = row.try_get("provider")?;
    let currency: String = row.try_get("currency")?;
    let state: String = row.try_get("state")?;
    Ok(CostLedgerEntry {
        id: row.try_get("id")?,
        snapshot_id: row.try_get("snapshot_id")?,
        attempt_id: row.try_get("attempt_id")?,
        provider: parse_provider(&provider)?,
        currency: parse_currency(&currency)?,
        reserved: row.try_get("reserved")?,
        actual: row.try_get("actual")?,
        state: parse_ledger_state(&state)?,
        price_version: row.try_get("price_version")?,
        created_at: Timestamp::from_millis(row.try_get("created_at")?),
        updated_at: Timestamp::from_millis(row.try_get("updated_at")?),
    })
}

fn parse_provider(value: &str) -> Result<ProviderKey, StorageError> {
    match value {
        "tripo" => Ok(ProviderKey::Tripo),
        "manual_ai" => Ok(ProviderKey::ManualAi),
        other => Err(StorageError::Database {
            detail: format!("cost_ledger.provider 取值未知：{other}"),
        }),
    }
}

fn parse_currency(value: &str) -> Result<Currency, StorageError> {
    match value {
        "credit_minor" => Ok(Currency::CreditMinor),
        "usd_micros" => Ok(Currency::UsdMicros),
        other => Err(StorageError::Database {
            detail: format!("cost_ledger.currency 取值未知：{other}"),
        }),
    }
}

fn parse_ledger_state(value: &str) -> Result<LedgerState, StorageError> {
    match value {
        "reserved" => Ok(LedgerState::Reserved),
        "settled" => Ok(LedgerState::Settled),
        "released" => Ok(LedgerState::Released),
        "unknown" => Ok(LedgerState::Unknown),
        other => Err(StorageError::Database {
            detail: format!("cost_ledger.state 取值未知：{other}"),
        }),
    }
}
