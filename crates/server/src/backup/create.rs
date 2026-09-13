//! 备份创建（T20 / REQ-005、AC-009；architecture.md §6/§7、validation-release.md §5）。
//!
//! 前置条件（CLI 层保证）：调用方已取得 data-dir 排他锁（`DirLock`），
//! 即**服务必须已停止**——架构明确"备份首版采用停止服务并获取独占锁后的快照"。
//!
//! 产物布局（`<--out 目录>/`，全部为新路径，绝不覆盖）：
//!
//! ```text
//! manifest.json                  备份 manifest（相对路径 + sha256 + 计数 + schema 版本）
//! database/manual.sqlite3        一致 SQLite 快照（VACUUM INTO；已清空会话）
//! blobs/<前 2 位>/<sha256>       全部"在库且文件存在"的被引用 blob（内容寻址）
//! ```
//!
//! **为什么是 `VACUUM INTO` 而不是复制 `manual.sqlite3`**：WAL 模式下已提交事务可能
//! 仍在 `-wal` 里，只复制主文件会丢数据（architecture §7 明令禁止）。`VACUUM INTO`
//! 由 SQLite 自己产生事务一致的完整副本，不需要我们在"复制主文件 + 复制 WAL +
//! 复制 SHM"之间做容易出错的组合，且源库只读、不被修改。
//!
//! **快照中不含会话**：`VACUUM INTO` 之后在同一快照连接上清空 `sessions`
//! （PRD REQ-005/AC-010："备份不含……会话"），并把日志模式归一为 `DELETE`，
//! 保证快照是单文件、无 `-wal/-shm` 边车。管理员口令哈希保留——否则灾备恢复后
//! 无法登录，备份也失去意义。
//!
//! **快照中不含供应商临时/签名 URL**（T20/BUG-008；AC-010）：同一快照连接上对
//! **供应商事实列**（`job_stages` / `provider_attempts` 的全部文本列）做兜底过滤——
//! 修复前落库的历史行里可能仍有 `output.model_url` 这类临时地址，它们被替换为
//! `{"redacted":true,"host":…,"sha256":…}` 摘要（[`crate::redaction`]）。
//! **JSON 列逐字符串值脱敏**（ADR-034 / BUG-011）：句子型字符串只替换 URL 片段，
//! 键、结构与其它文字原样保留；`needs_input_json.message` 等字符串契约列永不改类型。
//! 策略是**过滤而不是失败**：备份是灾备，历史行里有旧 URL 不该让用户无法备份；
//! 过滤只作用于这两张表的系统事实列，用户内容（知识 JSON、`sourceUrl` 等）原样保留。
//!
//! **失败语义**：只读源数据、不删除任何内容；失败时**不清理已创建的部分备份**
//! （保留现场供排查），错误消息明确说明这一点。

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{AssertSqlSafe, Connection, Row};

use manual_core::timestamps::Timestamp;

use crate::assets::blob_store;
use crate::config::datadir;
use crate::storage::BUSY_TIMEOUT;

use super::error::BackupError;
use super::files::{copy_file, fsync_dir, write_new_file};
use super::manifest::{
    BACKUP_BLOBS_DIR, BACKUP_CHECKSUMS_FILE, BACKUP_DATABASE_DIR, BACKUP_DATABASE_FILE,
    BACKUP_DATABASE_RELATIVE, BACKUP_MANIFEST_FILE, BACKUP_SCHEMA_VERSION, BackupBlobEntry,
    BackupCounts, BackupDatabaseEntry, BackupManifest, BackupMissingBlob,
    backup_blob_relative_path,
};

/// 备份完成的摘要（CLI 输出与日志用；不含密钥/会话）。
#[derive(Debug, Clone)]
pub struct BackupOutcome {
    pub out_dir: PathBuf,
    pub database_sha256: String,
    pub database_size: u64,
    pub schema_version: i64,
    pub blobs_copied: usize,
    pub missing_blobs: usize,
    pub total_bytes: u64,
    pub sessions_removed_count: i64,
    /// 快照内被脱敏的供应商临时 URL 处数（T20/BUG-008；0 = 库里没有历史 URL）。
    pub temp_urls_redacted: usize,
}

/// 备份 manifest 面向运维人员的固定说明（随备份一起交付）。
pub fn backup_notes() -> Vec<String> {
    vec![
        "本备份由 everything-manual backup 在服务停止并取得 data-dir 排他锁后生成".to_owned(),
        "快照为 VACUUM INTO 产生的事务一致副本（含 WAL 中已提交事务），不是主文件的字节复制"
            .to_owned(),
        "备份不包含会话（恢复后必须重新登录）；管理员口令哈希保留，用于恢复后登录".to_owned(),
        "SHA256SUMS 可用标准工具校验整个备份：shasum -a 256 -c SHA256SUMS（在备份目录内执行）"
            .to_owned(),
        "恢复要求目标目录不存在或为空：everything-manual restore --from <本目录> --data-dir <新目录>"
            .to_owned(),
        "升级前请先备份；程序回滚不等于数据库回滚（回滚程序不能读取比它新的 schema）".to_owned(),
        "快照内供应商临时/签名 URL 已脱敏为 sha256 摘要 + host（AC-010）；恢复后仍按 task_id 重新查询链接，不重新购买".to_owned(),
    ]
}

/// 创建备份：一致 SQLite 快照 + 全部被引用 blob + manifest + sha256。
///
/// `out_dir` 必须是**不存在的路径**（相对路径由调用方解析为绝对路径）。
pub async fn create_backup(data_dir: &Path, out_dir: &Path) -> Result<BackupOutcome, BackupError> {
    if out_dir.exists() {
        return Err(BackupError::path(format!(
            "备份输出路径已存在，绝不覆盖：{}；请换一个不存在的路径（--out）",
            out_dir.display()
        )));
    }
    if paths_overlap(data_dir, out_dir) {
        return Err(BackupError::path(format!(
            "备份输出路径不能与 data-dir 互相嵌套（data-dir {}，--out {}）：\
             请把备份放在 data-dir 之外（磁盘故障时两者不会一起丢失）",
            data_dir.display(),
            out_dir.display()
        )));
    }

    // 源 data-dir 结构（与 check/serve 同一套校验）与数据库存在性。
    datadir::verify(data_dir).map_err(|error| BackupError::path(error.message))?;
    let source_database = crate::storage::database_path(data_dir);
    if !source_database.is_file() {
        return Err(BackupError::path(format!(
            "data-dir 中没有数据库文件（{}）：请先 init 或 serve 建立数据",
            source_database.display()
        )));
    }

    tokio::fs::create_dir_all(out_dir.join(BACKUP_DATABASE_DIR))
        .await
        .map_err(|error| {
            BackupError::io(format!("创建备份目录失败 {}：{error}", out_dir.display()))
        })?;
    tokio::fs::create_dir_all(out_dir.join(BACKUP_BLOBS_DIR))
        .await
        .map_err(|error| {
            BackupError::io(format!("创建备份目录失败 {}：{error}", out_dir.display()))
        })?;
    restrict_dir_permissions(out_dir);

    // 1) 一致快照（只读源库；SQLite 自己处理 WAL）。
    let snapshot_path = out_dir.join(BACKUP_DATABASE_DIR).join(BACKUP_DATABASE_FILE);
    vacuum_into(&source_database, &snapshot_path).await?;

    // 2) 快照内清空会话 + 脱敏供应商临时 URL + 读取 schema 版本/计数（同一连接完成）。
    let (database_entry, counts, blob_rows, temp_urls_redacted) =
        scrub_and_read_snapshot(&snapshot_path, out_dir).await?;

    // 3) 复制全部被引用 blob（存在才复制；缺失如实记录）。
    let mut blobs = Vec::with_capacity(blob_rows.len());
    let mut missing_blobs = Vec::new();
    let mut total_bytes = database_entry.size;
    for row in &blob_rows {
        let source = blob_store::blob_path(data_dir, &row.sha256);
        if !source.is_file() {
            tracing::warn!(
                event = "backup_blob_missing",
                sha256 = %row.sha256,
                storageState = %row.storage_state,
                "被引用的 blob 文件缺失：备份如实记录，不伪造内容"
            );
            missing_blobs.push(BackupMissingBlob {
                sha256: row.sha256.clone(),
                storage_state: row.storage_state.clone(),
            });
            continue;
        }
        let relative = backup_blob_relative_path(&row.sha256);
        let destination = out_dir.join(&relative);
        let copied = copy_file(&source, &destination).await?;
        if copied.sha256 != row.sha256 {
            return Err(BackupError::integrity(
                "backup_source_blob_corrupt",
                format!(
                    "源 data-dir 的 blob 内容与文件名/hash 不符（期望 {expected}，实际 {actual}）：{path}；\
                     备份中止，源数据未被修改（不删除任何文件）",
                    expected = row.sha256,
                    actual = copied.sha256,
                    path = source.display()
                ),
            ));
        }
        if row.size >= 0 && copied.size != row.size as u64 {
            return Err(BackupError::integrity(
                "backup_source_blob_size_mismatch",
                format!(
                    "源 data-dir 的 blob 大小与元数据不符（期望 {} 字节，实际 {} 字节）：{}",
                    row.size,
                    copied.size,
                    source.display()
                ),
            ));
        }
        total_bytes += copied.size;
        blobs.push(BackupBlobEntry {
            path: relative,
            sha256: copied.sha256,
            size: copied.size,
        });
    }

    // 4) manifest（最后写：它的存在即"备份已完成"的标记）。
    let manifest = BackupManifest {
        schema_version: BACKUP_SCHEMA_VERSION.to_owned(),
        created_at_millis: Timestamp::now().as_millis(),
        program_version: env!("CARGO_PKG_VERSION").to_owned(),
        database: database_entry.clone(),
        blobs: blobs.clone(),
        missing_blobs: missing_blobs.clone(),
        counts,
        notes: backup_notes(),
    };
    let manifest_bytes = manifest.to_bytes()?;
    write_new_file(&out_dir.join(BACKUP_MANIFEST_FILE), &manifest_bytes).await?;

    // SHA256SUMS（标准 `shasum -c` / `sha256sum -c` 可直接校验整个备份；
    // manifest 自身的哈希只能在这里出现——它是"备份完成"的收尾文件）。
    let mut checksums = String::new();
    checksums.push_str(&format!(
        "{sha}  {path}\n",
        sha = database_entry.sha256,
        path = BACKUP_DATABASE_RELATIVE
    ));
    for blob in &blobs {
        checksums.push_str(&format!(
            "{sha}  {path}\n",
            sha = blob.sha256,
            path = blob.path
        ));
    }
    checksums.push_str(&format!(
        "{sha}  {file}\n",
        sha = super::files::hex_digest(&Sha256::digest(&manifest_bytes)),
        file = BACKUP_MANIFEST_FILE
    ));
    write_new_file(&out_dir.join(BACKUP_CHECKSUMS_FILE), checksums.as_bytes()).await?;

    fsync_dir(&out_dir.join(BACKUP_DATABASE_DIR));
    fsync_dir(&out_dir.join(BACKUP_BLOBS_DIR));
    fsync_dir(out_dir);

    let outcome = BackupOutcome {
        out_dir: out_dir.to_path_buf(),
        database_sha256: database_entry.sha256,
        database_size: database_entry.size,
        schema_version: database_entry.schema_version,
        blobs_copied: blobs.len(),
        missing_blobs: missing_blobs.len(),
        total_bytes: total_bytes + manifest_bytes.len() as u64 + checksums.len() as u64,
        sessions_removed_count: database_entry.sessions_removed_count,
        temp_urls_redacted,
    };
    tracing::info!(
        event = "backup_created",
        outDir = %out_dir.display(),
        schemaVersion = outcome.schema_version,
        databaseSha256 = %outcome.database_sha256,
        blobsCopied = outcome.blobs_copied,
        missingBlobs = outcome.missing_blobs,
        totalBytes = outcome.total_bytes,
        sessionsRemoved = outcome.sessions_removed_count,
        tempUrlsRedacted = outcome.temp_urls_redacted,
        "备份完成（一致快照 + 被引用 blob + manifest；不含会话与供应商临时 URL）"
    );
    Ok(outcome)
}

/// `VACUUM INTO`：以只读连接读取源库，由 SQLite 产生事务一致的单文件副本。
///
/// 只读连接保证备份**不修改源库**（不 checkpoint、不写 WAL、不改 journal 模式）。
async fn vacuum_into(source: &Path, snapshot: &Path) -> Result<(), BackupError> {
    let options = SqliteConnectOptions::new()
        .filename(source)
        .create_if_missing(false)
        .read_only(true)
        .busy_timeout(BUSY_TIMEOUT);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|error| {
            BackupError::io(format!(
                "只读打开数据库失败（{}）：{error}",
                source.display()
            ))
        })?;
    let snapshot_path = snapshot.to_str().ok_or_else(|| {
        BackupError::path(format!(
            "备份目标路径不是合法 UTF-8（{}）",
            snapshot.display()
        ))
    })?;
    let result = sqlx::query("VACUUM INTO ?")
        .bind(snapshot_path)
        .execute(&mut connection)
        .await;
    connection.close().await.ok();
    match result {
        Ok(_) => Ok(()),
        Err(error) => Err(BackupError::integrity(
            "backup_snapshot_failed",
            format!(
                "无法生成一致快照（VACUUM INTO）：{error}；\
                 部分备份目录可能已创建，失败现场保留供排查（不删除任何文件）"
            ),
        )),
    }
}

/// 快照内的行（blobs 表）。
struct BlobRow {
    sha256: String,
    size: i64,
    storage_state: String,
}

/// 在快照连接上：清空会话、脱敏供应商临时 URL、归一日志模式、
/// 读取 schema 版本与计数与 blob 行。
async fn scrub_and_read_snapshot(
    snapshot: &Path,
    out_dir: &Path,
) -> Result<(BackupDatabaseEntry, BackupCounts, Vec<BlobRow>, usize), BackupError> {
    let options = SqliteConnectOptions::new()
        .filename(snapshot)
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Delete)
        .busy_timeout(BUSY_TIMEOUT);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|error| {
            BackupError::io(format!(
                "打开备份快照失败（{}）：{error}",
                snapshot.display()
            ))
        })?;

    type SnapshotScan = (i64, i64, (i64, i64, i64, i64), Vec<BlobRow>, usize);
    let result: Result<SnapshotScan, BackupError> = async {
        // 会话不随备份（REQ-005/AC-010）；口令哈希保留（否则恢复后无法登录）。
        let sessions_removed_count = sqlx::query("DELETE FROM sessions")
            .execute(&mut connection)
            .await
            .map_err(|error| {
                BackupError::integrity(
                    "backup_snapshot_scrub_failed",
                    format!("清空快照会话失败（数据库不是本程序创建的？）：{error}"),
                )
            })?
            .rows_affected() as i64;

        // 供应商临时/签名 URL 不随备份（AC-010；T20/BUG-008）：兜底过滤历史行。
        let temp_urls_redacted = redact_temp_urls_in_snapshot(&mut connection).await?;

        let schema_version =
            crate::storage::migrations::applied_schema_version_conn(&mut connection).await?;

        let items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items")
            .fetch_one(&mut connection)
            .await?;
        let assets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assets")
            .fetch_one(&mut connection)
            .await?;
        let releases: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM manual_releases")
            .fetch_one(&mut connection)
            .await?;
        let blobs_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
            .fetch_one(&mut connection)
            .await?;

        let rows = sqlx::query("SELECT sha256, size, storage_state FROM blobs ORDER BY sha256")
            .fetch_all(&mut connection)
            .await
            .map_err(BackupError::from)?;
        let blob_rows: Vec<BlobRow> = rows
            .iter()
            .map(|row| {
                Ok(BlobRow {
                    sha256: row.try_get("sha256")?,
                    size: row.try_get("size")?,
                    storage_state: row.try_get("storage_state")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(BackupError::from)?;

        Ok((
            sessions_removed_count,
            schema_version,
            (items, assets, releases, blobs_count),
            blob_rows,
            temp_urls_redacted,
        ))
    }
    .await;
    connection.close().await.ok();
    let (sessions_removed_count, schema_version, counts, blob_rows, temp_urls_redacted) = result?;

    // 关闭连接后快照必须是单文件（无 -wal/-shm 边车）：日志模式已归一为 DELETE。
    let sidecar = sidecar_files(snapshot);
    if !sidecar.is_empty() {
        return Err(BackupError::integrity(
            "backup_snapshot_sidecar_left",
            format!(
                "快照仍带有 WAL 边车文件（{}）：备份不完整，请重试",
                sidecar
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("、")
            ),
        ));
    }

    let fingerprint = super::files::fingerprint_file(snapshot).await?;
    let entry = BackupDatabaseEntry {
        path: BACKUP_DATABASE_RELATIVE.to_owned(),
        sha256: fingerprint.sha256,
        size: fingerprint.size,
        schema_version,
        sessions_removed: true,
        sessions_removed_count,
    };
    let counts = BackupCounts {
        items: counts.0,
        assets: counts.1,
        releases: counts.2,
        blobs: counts.3,
    };
    tracing::debug!(
        event = "backup_snapshot_ready",
        outDir = %out_dir.display(),
        size = entry.size,
        schemaVersion = schema_version,
        tempUrlsRedacted = temp_urls_redacted,
        "快照已生成（会话已清空、供应商临时 URL 已脱敏）"
    );
    Ok((entry, counts, blob_rows, temp_urls_redacted))
}

/// 快照内的兜底脱敏：供应商事实表（`job_stages` / `provider_attempts`）的
/// **全部文本列**里，URL 形态的字符串替换为 sha256 摘要 + host（[`crate::redaction`]）。
///
/// 为什么按"表 + 全部文本列"而不是"指定字段名"：这是给**未来新增字段**兜底
/// （例如以后再加一个 `previewUrl` 列也不会漏），这两张表按合同只存系统/供应商
/// 事实（contracts §5），没有用户原创内容，过滤不会损失用户数据；
/// 用户内容（items/drafts/releases 的知识 JSON、`sourceUrl`）不在过滤范围内。
/// 只处理含 `://` 的值：绝大多数行零成本跳过（快照不大，且备份要求先停服）。
async fn redact_temp_urls_in_snapshot(
    connection: &mut SqliteConnection,
) -> Result<usize, BackupError> {
    let mut redacted = 0;
    for table in ["job_stages", "provider_attempts"] {
        // 表名来自本函数的固定清单、列名来自 PRAGMA（都是本程序的 schema），
        // 不是用户输入：动态 SQL 在这里是安全的（`AssertSqlSafe` 显式声明已审计）。
        let columns: Vec<String> =
            sqlx::query(AssertSqlSafe(format!("PRAGMA table_info({table})")))
                .fetch_all(&mut *connection)
                .await
                .map_err(|error| scrub_failed(table, &format!("读取表结构失败：{error}")))?
                .iter()
                .filter_map(|row| {
                    let declared_type: String = row.try_get("type").ok()?;
                    let name: String = row.try_get("name").ok()?;
                    declared_type
                        .to_ascii_uppercase()
                        .contains("TEXT")
                        .then_some(name)
                })
                .collect();
        for column in columns {
            redacted += redact_temp_urls_in_column(connection, table, &column).await?;
        }
    }
    if redacted > 0 {
        tracing::warn!(
            event = "backup_temp_urls_redacted",
            count = redacted,
            "快照内有历史供应商临时 URL：已替换为 sha256 摘要 + host（源 data-dir 未修改）"
        );
    }
    Ok(redacted)
}

/// 单列表的脱敏（JSON 值用**结构感知**替换；非 JSON 文本用文本级兜底）。
///
/// JSON 列按逐字符串值处理（ADR-034）：句子型字符串只替换 URL 片段（BUG-011：
/// `needs_input_json` 的整句说明与同列其它条目必须完整保留），纯 URL 值按列的
/// 契约选择形态——`needs_input_json.message` 是字符串契约用
/// [`JsonStringRedaction::KeepString`]，其余事实 JSON（`usage_json` 等）沿用
/// ADR-032 的摘要对象。
async fn redact_temp_urls_in_column(
    connection: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<usize, BackupError> {
    let mode = json_string_mode_for_column(column);
    // 表/列名同上（固定清单 + PRAGMA），值一律走绑定参数。
    let rows = sqlx::query(AssertSqlSafe(format!(
        "SELECT rowid, {column} AS value FROM {table} \
         WHERE {column} IS NOT NULL AND instr(CAST({column} AS TEXT), '://') > 0"
    )))
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| scrub_failed(table, &format!("扫描 {column} 失败：{error}")))?;

    let mut redacted = 0;
    for row in rows {
        let rowid: i64 = row
            .try_get("rowid")
            .map_err(|error| scrub_failed(table, &format!("读取 rowid 失败：{error}")))?;
        let value: String = row
            .try_get("value")
            .map_err(|error| scrub_failed(table, &format!("读取 {column} 失败：{error}")))?;
        let (replacement, count) = match serde_json::from_str::<serde_json::Value>(&value) {
            Ok(mut json) => {
                let count = crate::redaction::redact_urls_in_json_with(&mut json, mode);
                if count == 0 {
                    continue;
                }
                (json.to_string(), count)
            }
            // 非 JSON（损坏的事实文本）也要兜底：文本级替换。
            Err(_) => {
                let (text, count) = crate::redaction::redact_urls_in_text(&value);
                if count == 0 {
                    continue;
                }
                (text, count)
            }
        };
        sqlx::query(AssertSqlSafe(format!(
            "UPDATE {table} SET {column} = ? WHERE rowid = ?"
        )))
        .bind(replacement)
        .bind(rowid)
        .execute(&mut *connection)
        .await
        .map_err(|error| scrub_failed(table, &format!("写回 {column} 失败：{error}")))?;
        redacted += count;
    }
    Ok(redacted)
}

/// JSON 列的字符串脱敏形态（按列契约选择，ADR-034）。
///
/// `needs_input_json` 的条目形状是 `{code, message}`，`message` 必须保持字符串
/// （读取侧按类型反序列化；值变对象会让整列静默失败——BUG-011 的根因侧面）；
/// 其它 JSON 事实列沿用 ADR-032 的"整串 URL → 摘要对象"。
fn json_string_mode_for_column(column: &str) -> crate::redaction::JsonStringRedaction {
    match column {
        "needs_input_json" => crate::redaction::JsonStringRedaction::KeepString,
        _ => crate::redaction::JsonStringRedaction::SummaryObject,
    }
}

/// 快照脱敏失败的统一错误（保留现场，不静默继续）。
fn scrub_failed(table: &str, detail: &str) -> BackupError {
    BackupError::integrity(
        "backup_snapshot_scrub_failed",
        format!("快照供应商事实脱敏失败（{table}）：{detail}；备份中止，源数据未被修改"),
    )
}

/// 快照的 `-wal` / `-shm` 边车（存在即异常；备份要求单文件）。
fn sidecar_files(snapshot: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for suffix in ["-wal", "-shm"] {
        let mut name = snapshot.as_os_str().to_os_string();
        name.push(suffix);
        let path = PathBuf::from(name);
        if path.exists() {
            found.push(path);
        }
    }
    found
}

/// 两个路径是否互相嵌套（都用绝对路径比较；`--out` 由 CLI 层解析为绝对路径）。
fn paths_overlap(data_dir: &Path, out_dir: &Path) -> bool {
    out_dir.starts_with(data_dir) || data_dir.starts_with(out_dir)
}

/// 备份根目录 0700（备份含用户原始资料；PRD §5.7 的隐私要求）。
fn restrict_dir_permissions(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_detection_covers_nesting_both_ways() {
        let data = Path::new("/srv/em/data");
        assert!(paths_overlap(data, Path::new("/srv/em/data/backups/x")));
        assert!(paths_overlap(data, Path::new("/srv/em")));
        assert!(!paths_overlap(data, Path::new("/srv/em/backups")));
        assert!(!paths_overlap(data, Path::new("/other/place")));
    }
}
