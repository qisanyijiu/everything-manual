//! 费用账本的服务级封装（T11 / contracts.md §4）。
//!
//! 与仓储原语（`storage::repo::ledger`）的分工：本模块表达**业务规则**——
//! 哪些路径允许结算、哪些允许释放、unknown 必须如何保留：
//! - [`reserve_for_snapshot`]：建单事务内的分列预留（金额 = 服务器计算的保守上界）；
//! - [`settle_attempt`]：拿到供应商计费事实后按实际金额结算（幂等，不覆盖事实）；
//! - [`release_definitely_not_billed`]：**明确未计费**的失败才允许自动释放
//!   （可证明未被接受的错误）；对 `unknown` 的释放必须走管理员对账（T15），
//!   本函数不提供该入口；
//! - [`mark_submission_unknown`]：结果未知 → 保留预留（`actual` 保持 NULL，不得填 0），
//!   并可把 attempt 关联到账本条目（对账时能追溯）。
//!
//! 全部写在调用方的事务短事务里（"预留/结算/释放在事务内且幂等"）。

use sqlx::SqliteConnection;

use manual_core::domain::{CostLedgerEntry, Currency, LedgerState, ProviderKey};
use manual_core::timestamps::Timestamp;

use crate::storage::repo::ledger::{self, LedgerOutcome, NewReservation};

use crate::storage::StorageError;

/// 按分列金额预留（Tripo credits + Manual AI USD；不相加、不采信前端数值）。
pub async fn reserve_for_snapshot(
    conn: &mut SqliteConnection,
    snapshot_id: &str,
    price_version: &str,
    tripo_credit_minor: i64,
    manual_ai_usd_micros: i64,
    now: Timestamp,
) -> Result<Vec<CostLedgerEntry>, StorageError> {
    let mut entries = Vec::with_capacity(2);
    entries.push(
        ledger::reserve(
            conn,
            NewReservation {
                snapshot_id: snapshot_id.to_owned(),
                provider: ProviderKey::Tripo,
                currency: Currency::CreditMinor,
                reserved: tripo_credit_minor,
                price_version: price_version.to_owned(),
            },
            now,
        )
        .await?,
    );
    entries.push(
        ledger::reserve(
            conn,
            NewReservation {
                snapshot_id: snapshot_id.to_owned(),
                provider: ProviderKey::ManualAi,
                currency: Currency::UsdMicros,
                reserved: manual_ai_usd_micros,
                price_version: price_version.to_owned(),
            },
            now,
        )
        .await?,
    );
    Ok(entries)
}

/// 按实际金额结算（调用方必须已拿到供应商的计费事实；金额为整数最小单位）。
pub async fn settle_attempt(
    conn: &mut SqliteConnection,
    entry_id: &str,
    actual_minor: i64,
    now: Timestamp,
) -> Result<LedgerOutcome, StorageError> {
    ledger::settle(conn, entry_id, actual_minor, now).await
}

/// 释放预留：**只在明确未计费时调用**（可证明未被接受的失败）。
///
/// 语义边界（contracts.md §4）：`unknown` 状态不能由自动路径释放——本函数只接受
/// `reserved` 的条目；对 `unknown` 的释放必须走管理员对账入口
/// （[`release_after_reconciliation`]，T15 的 `recordNoTask` 等）。
pub async fn release_definitely_not_billed(
    conn: &mut SqliteConnection,
    entry_id: &str,
    now: Timestamp,
) -> Result<LedgerOutcome, StorageError> {
    let Some(entry) = ledger::get(conn, entry_id).await? else {
        return Err(StorageError::NotFound {
            entity: "cost_ledger",
            id: entry_id.to_owned(),
        });
    };
    match entry.state {
        // 首次释放：写入 released；重复释放：仓储层按幂等返回（同一决定重复执行不改变事实）。
        LedgerState::Reserved | LedgerState::Released => ledger::release(conn, entry_id, now).await,
        LedgerState::Unknown | LedgerState::Settled => Ok(LedgerOutcome::Rejected {
            reason: format!(
                "自动路径不能释放 {} 的预留：unknown 必须经管理员对账（release_after_reconciliation）、\
                 已结算的金额不得改写",
                entry.state.as_str()
            ),
        }),
    }
}

/// 管理员对账后的释放（`unknown → released`；T15 的 recordNoTask 等显式决定）。
pub async fn release_after_reconciliation(
    conn: &mut SqliteConnection,
    entry_id: &str,
    now: Timestamp,
) -> Result<LedgerOutcome, StorageError> {
    ledger::release(conn, entry_id, now).await
}

/// 结果未知：保留预留、`actual` 保持 NULL；可选地把 attempt 关联到账本条目。
pub async fn mark_submission_unknown(
    conn: &mut SqliteConnection,
    entry_id: &str,
    attempt_id: Option<&str>,
    now: Timestamp,
) -> Result<LedgerOutcome, StorageError> {
    if let Some(attempt_id) = attempt_id {
        let _ = ledger::attach_attempt(conn, entry_id, attempt_id, now).await?;
    }
    ledger::mark_unknown(conn, entry_id, now).await
}

/// 该条目是否仍占用预算（`reserved` / `unknown`；`unknown` 不得当作 0）。
pub fn holds_budget(entry: &CostLedgerEntry) -> bool {
    manual_core::cost::ledger_state_holds_budget(entry.state)
}

/// 供展示/测试的状态摘要：`(provider, currency, reserved, actual, state)`。
pub fn summarize(
    entry: &CostLedgerEntry,
) -> (ProviderKey, Currency, i64, Option<i64>, LedgerState) {
    (
        entry.provider,
        entry.currency,
        entry.reserved,
        entry.actual,
        entry.state,
    )
}
