//! 备份 manifest（`<备份目录>/manifest.json`；T20 / REQ-005、AC-009）。
//!
//! **内容是备份的机器合同**：快照数据库与每个被引用 blob 的相对路径、sha256、大小，
//! 以及数据库 schema 版本与计数。恢复时先按它做全量校验，再写入目标目录。
//!
//! 硬约束（contracts.md §7 与 PRD §5.7 的同一口径）：
//! - 只包含**相对路径**（POSIX 分隔符），绝不写入绝对路径；
//! - 不包含任何密钥、会话（快照内的 `sessions` 行在备份时被清空）、临时云端 URL；
//! - 未知字段在读取时被拒绝（`deny_unknown_fields`）：备份格式是自有的、版本化的，
//!   "多出来的字段"意味着写它的不是本程序，应显式报错而不是猜。

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use super::error::BackupError;

/// 备份目录布局常量。
pub const BACKUP_SCHEMA_VERSION: &str = "manual_backup_v1";
pub const BACKUP_MANIFEST_FILE: &str = "manifest.json";
/// 标准校验和清单（`shasum -c SHA256SUMS` / `sha256sum -c SHA256SUMS` 可直接校验）。
pub const BACKUP_CHECKSUMS_FILE: &str = "SHA256SUMS";
pub const BACKUP_DATABASE_DIR: &str = "database";
pub const BACKUP_DATABASE_FILE: &str = "manual.sqlite3";
/// 快照数据库在备份内的相对路径（manifest 与恢复共用）。
pub const BACKUP_DATABASE_RELATIVE: &str = "database/manual.sqlite3";
/// 被引用 blob 在备份内的目录（与 data-dir 的 `blobs/<前 2 位>/<sha256>` 同构）。
pub const BACKUP_BLOBS_DIR: &str = "blobs";

/// 备份 manifest 根结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupManifest {
    /// 备份格式版本（[`BACKUP_SCHEMA_VERSION`]；不认识的值直接拒绝）。
    pub schema_version: String,
    /// 备份创建时间（Unix 毫秒，UTC）。
    pub created_at_millis: i64,
    /// 生成备份的程序版本（`CARGO_PKG_VERSION`）。
    pub program_version: String,
    /// 一致 SQLite 快照（`VACUUM INTO`；已清空会话）。
    pub database: BackupDatabaseEntry,
    /// 全部被引用且实际存在的 blob（按 sha256 升序，确定性输出）。
    pub blobs: Vec<BackupBlobEntry>,
    /// 元数据在库但文件缺失的 blob（如实记录，不假装备份完整）。
    #[serde(default)]
    pub missing_blobs: Vec<BackupMissingBlob>,
    /// 关键行数（恢复后校验与运维核对用）。
    pub counts: BackupCounts,
    /// 面向运维人员的中文说明（恢复前置条件、会话不随备份等）。
    pub notes: Vec<String>,
}

/// 快照数据库条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupDatabaseEntry {
    /// 相对路径（固定 [`BACKUP_DATABASE_RELATIVE`]）。
    pub path: String,
    pub sha256: String,
    pub size: u64,
    /// 快照里 `_sqlx_migrations` 的最大成功版本（恢复方据此做 schema 门禁）。
    pub schema_version: i64,
    /// 快照是否已清空 `sessions` 表（备份不含会话；恢复后必须重新登录）。
    pub sessions_removed: bool,
    /// 被清空的会话行数（0 表示备份时没有活动会话）。
    pub sessions_removed_count: i64,
}

/// 一个被引用 blob。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupBlobEntry {
    /// 相对路径（`blobs/<前 2 位>/<sha256>`）。
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// 文件缺失的 blob（源 data-dir 的既有状态；备份如实记录，不阻塞其余数据）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupMissingBlob {
    pub sha256: String,
    /// 源库中的 `storage_state`（stored/quarantined/missing）。
    pub storage_state: String,
}

/// 备份的关键行数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupCounts {
    pub items: i64,
    pub assets: i64,
    pub releases: i64,
    pub blobs: i64,
}

impl BackupManifest {
    /// 序列化为确定性的 pretty JSON（末尾换行，便于人工检查与 diff）。
    pub fn to_bytes(&self) -> Result<Vec<u8>, BackupError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|error| {
            BackupError::integrity(
                "backup_manifest_serialization_failed",
                format!("备份 manifest 序列化失败：{error}"),
            )
        })?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// 从字节解析（未知字段/类型不符 → 完整性错误，不猜）。
    pub fn parse(bytes: &[u8]) -> Result<Self, BackupError> {
        let manifest: Self = serde_json::from_slice(bytes).map_err(|error| {
            BackupError::integrity(
                "backup_manifest_invalid",
                format!("备份 manifest 不是本程序认识的格式：{error}"),
            )
        })?;
        if manifest.schema_version != BACKUP_SCHEMA_VERSION {
            return Err(BackupError::integrity(
                "backup_manifest_unsupported_schema",
                format!(
                    "备份格式版本不受支持：{actual}（本程序支持 {expected}）；\
                     请使用创建该备份的（或更新的）程序恢复",
                    actual = manifest.schema_version,
                    expected = BACKUP_SCHEMA_VERSION
                ),
            ));
        }
        Ok(manifest)
    }

    /// 校验所有相对路径都是安全的（恢复前必须调用）。
    ///
    /// 规则：非空、非绝对、不含 `..`/`.` 组件、不含反斜杠与 NUL、不使用盘符/UNC 前缀。
    /// 这是"备份目录可能被人为改动"的第一道防线（与将来 ZIP 导入的 zip-slip
    /// 验收同一思路；本卡不开放 ZIP 导入）。
    pub fn validate_relative_paths(&self) -> Result<(), BackupError> {
        validate_relative_path(&self.database.path)?;
        for blob in &self.blobs {
            validate_relative_path(&blob.path)?;
        }
        Ok(())
    }
}

/// 单个相对路径的安全校验。
pub fn validate_relative_path(value: &str) -> Result<(), BackupError> {
    let invalid = |reason: &str| {
        BackupError::integrity(
            "backup_path_unsafe",
            format!("备份 manifest 中的路径不安全（{reason}）：{value}"),
        )
    };
    if value.is_empty() {
        return Err(invalid("空路径"));
    }
    if value.contains('\0') {
        return Err(invalid("包含 NUL 字节"));
    }
    if value.contains('\\') {
        return Err(invalid("包含反斜杠（只允许 POSIX 分隔符）"));
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(invalid("绝对路径不允许出现在备份中"));
    }
    if value.starts_with("//") {
        return Err(invalid("UNC 前缀不允许"));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => return Err(invalid("包含 .. 组件")),
            Component::RootDir | Component::Prefix(_) => return Err(invalid("绝对路径组件")),
        }
    }
    Ok(())
}

/// blob 在备份内的相对路径（与 data-dir 布局同构：`blobs/<前 2 位>/<sha256>`）。
pub fn backup_blob_relative_path(sha256: &str) -> String {
    format!("{BACKUP_BLOBS_DIR}/{}/{}", &sha256[..2], sha256)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BackupManifest {
        BackupManifest {
            schema_version: BACKUP_SCHEMA_VERSION.to_owned(),
            created_at_millis: 1_789_000_000_000,
            program_version: "0.1.0".to_owned(),
            database: BackupDatabaseEntry {
                path: BACKUP_DATABASE_RELATIVE.to_owned(),
                sha256: "a".repeat(64),
                size: 4096,
                schema_version: 7,
                sessions_removed: true,
                sessions_removed_count: 2,
            },
            blobs: vec![BackupBlobEntry {
                path: backup_blob_relative_path(&"b".repeat(64)),
                sha256: "b".repeat(64),
                size: 12,
            }],
            missing_blobs: Vec::new(),
            counts: BackupCounts {
                items: 1,
                assets: 2,
                releases: 1,
                blobs: 1,
            },
            notes: vec!["恢复目标必须不存在或为空".to_owned()],
        }
    }

    #[test]
    fn manifest_round_trips_and_paths_are_relative() {
        let manifest = sample();
        let bytes = manifest.to_bytes().unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.ends_with('\n'));
        assert!(!text.contains("/Users/"), "manifest 不得含绝对路径");
        let parsed = BackupManifest::parse(&bytes).unwrap();
        assert_eq!(parsed, manifest);
        parsed.validate_relative_paths().unwrap();
    }

    #[test]
    fn unknown_schema_version_is_rejected() {
        let mut manifest = sample();
        manifest.schema_version = "manual_backup_v9".to_owned();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let error = BackupManifest::parse(&bytes).unwrap_err();
        assert_eq!(error.code(), "backup_manifest_unsupported_schema");
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let bytes = br#"{
            "schemaVersion": "manual_backup_v1",
            "createdAtMillis": 0,
            "programVersion": "0.1.0",
            "database": {"path": "database/manual.sqlite3", "sha256": "aa", "size": 1,
                          "schemaVersion": 1, "sessionsRemoved": true, "sessionsRemovedCount": 0},
            "blobs": [],
            "counts": {"items": 0, "assets": 0, "releases": 0, "blobs": 0},
            "notes": [],
            "surprise": true
        }"#;
        let error = BackupManifest::parse(bytes).unwrap_err();
        assert_eq!(error.code(), "backup_manifest_invalid");
        assert!(error.to_string().contains("surprise"), "{error}");
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        for bad in [
            "/etc/passwd",
            "../outside",
            "blobs/../../outside",
            "blobs\\win",
            "",
            "//unc/share",
        ] {
            let error = validate_relative_path(bad).unwrap_err();
            assert_eq!(error.code(), "backup_path_unsafe", "应拒绝：{bad}");
        }
        for good in [
            "database/manual.sqlite3",
            "blobs/ab/abcd",
            "./blobs/ab/abcd",
        ] {
            validate_relative_path(good).unwrap_or_else(|error| panic!("{good}: {error}"));
        }
    }
}
