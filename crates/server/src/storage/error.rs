//! 存储层错误。
//!
//! 设计原则：
//! - 错误消息面向运维/管理员，可读、不含堆栈与密钥；**唯一例外**是 SQLite 的
//!   约束消息（例如 `UNIQUE constraint failed: blobs.sha256`），它不含用户数据；
//! - 乐观锁冲突（[`StorageError::RevisionConflict`]）携带当前 revision，供上层
//!   返回 412 与 `details.currentRevision`（contracts.md §1）；
//! - 本模块不依赖 `config`：CLI 侧用 `CliError::data_dir` 映射（见 `commands.rs`）。

use std::fmt;

/// 存储层错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// 库的 schema 版本高于本程序支持版本：**拒绝打开**，不迁移、不修改数据。
    SchemaTooNew {
        database_version: i64,
        program_version: i64,
    },
    /// 迁移记录处于未完成/损坏状态（例如上一次迁移中途崩溃且未回滚）。
    MigrationInconsistent { detail: String },
    /// 迁移执行失败（sqlx `MigrateError`；单条迁移在事务内，失败不留半升级状态）。
    Migration { detail: String },
    /// 外键约束被违反（引用不存在或 ON DELETE RESTRICT 阻止删除）。
    ForeignKeyViolation { detail: String },
    /// 唯一键/主键冲突。
    UniqueViolation { detail: String },
    /// 其它表约束（CHECK、NOT NULL 等）。
    ConstraintViolation { detail: String },
    /// 一般 SQL 错误（查询失败、语法、连接等）。
    Database { detail: String },
    /// 查询目标不存在。
    NotFound { entity: &'static str, id: String },
    /// 乐观锁冲突：期望的 revision 已过期；当前值为 `current_revision`。
    RevisionConflict {
        entity: &'static str,
        id: String,
        current_revision: i64,
    },
    /// 目标处于终态、不接受写入（T09：`ready` 的 preparation 禁止修改页，
    /// contracts.md §2「ready 后不可修改」）。
    NotWritable {
        entity: &'static str,
        id: String,
        state: String,
    },
    /// 文件系统 IO 错误（数据目录不可写等）。
    Io { detail: String },
}

impl StorageError {
    /// 是否为"库比程序新"的拒绝打开错误（用于测试与 CLI 文案分支）。
    pub fn is_schema_too_new(&self) -> bool {
        matches!(self, Self::SchemaTooNew { .. })
    }

    /// 是否为外键约束错误。
    pub fn is_foreign_key_violation(&self) -> bool {
        matches!(self, Self::ForeignKeyViolation { .. })
    }

    /// 是否为唯一键冲突。
    pub fn is_unique_violation(&self) -> bool {
        matches!(self, Self::UniqueViolation { .. })
    }

    /// 是否为 CHECK/NOT NULL 等表约束冲突。
    pub fn is_constraint_violation(&self) -> bool {
        matches!(self, Self::ConstraintViolation { .. })
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaTooNew {
                database_version,
                program_version,
            } => write!(
                f,
                "数据库 schema 版本（v{database_version}）高于本程序支持的版本（v{program_version}）：\
                 该 data-dir 由更新版本的程序创建。本程序拒绝打开且不会修改其中数据；\
                 请使用创建该 data-dir 的程序版本，或先备份后用旧版本可读的备份恢复"
            ),
            Self::MigrationInconsistent { detail } => write!(
                f,
                "数据库迁移记录不一致（可能存在未完成的迁移）：{detail}；请先用备份恢复或在排障后重试"
            ),
            Self::Migration { detail } => write!(f, "数据库迁移失败：{detail}"),
            Self::ForeignKeyViolation { detail } => write!(f, "外键约束被拒绝：{detail}"),
            Self::UniqueViolation { detail } => write!(f, "唯一键冲突：{detail}"),
            Self::ConstraintViolation { detail } => write!(f, "字段约束被拒绝：{detail}"),
            Self::Database { detail } => write!(f, "数据库错误：{detail}"),
            Self::NotFound { entity, id } => write!(f, "{entity} 不存在：{id}"),
            Self::RevisionConflict {
                entity,
                id,
                current_revision,
            } => write!(
                f,
                "{entity} 已被其他操作更新（id={id}，当前 revision={current_revision}）；请刷新后重试"
            ),
            Self::NotWritable { entity, id, state } => {
                write!(f, "{entity} 处于 {state} 状态，不接受写入（id={id}）")
            }
            Self::Io { detail } => write!(f, "文件系统错误：{detail}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<sqlx::Error> for StorageError {
    /// 把 SQLite 约束错误映射为可判定的类型（上层据此返回 409/412 或内部分支）。
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(database) = &error {
            let detail = database.message().to_owned();
            if database.is_unique_violation() {
                return Self::UniqueViolation { detail };
            }
            if database.is_foreign_key_violation() {
                return Self::ForeignKeyViolation { detail };
            }
            if database.is_check_violation() {
                return Self::ConstraintViolation { detail };
            }
            match database.code().as_deref() {
                // 1299 = SQLITE_CONSTRAINT_NOTNULL（sqlx 无专用判定）。
                Some("1299") => return Self::ConstraintViolation { detail },
                // 1811 = SQLITE_CONSTRAINT_TRIGGER，实测有两类来源（同为 1811，需看消息）：
                // 1) 外键动作 RESTRICT 拒绝父行删除 → "FOREIGN KEY constraint failed"；
                // 2) 迁移定义的不变量触发器 RAISE(ABORT, ...) → 自定义消息。
                // 只有前者是外键语义（上层要按"引用冲突"处理），按 SQLite 的稳定消息区分。
                Some("1811") => {
                    return if detail.contains("FOREIGN KEY") {
                        Self::ForeignKeyViolation { detail }
                    } else {
                        Self::ConstraintViolation { detail }
                    };
                }
                _ => {}
            }
        }
        Self::Database {
            detail: error.to_string(),
        }
    }
}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io {
            detail: error.to_string(),
        }
    }
}

impl From<sqlx::migrate::MigrateError> for StorageError {
    fn from(error: sqlx::migrate::MigrateError) -> Self {
        Self::Migration {
            detail: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_too_new_message_is_readable_and_actionable() {
        let error = StorageError::SchemaTooNew {
            database_version: 99,
            program_version: 2,
        };
        let text = error.to_string();
        assert!(text.contains("v99") && text.contains("v2"), "{text}");
        assert!(text.contains("拒绝打开"), "{text}");
        assert!(error.is_schema_too_new());
    }

    #[test]
    fn conflict_message_carries_current_revision() {
        let error = StorageError::RevisionConflict {
            entity: "item",
            id: "01993000-0000-7000-8000-000000000001".to_owned(),
            current_revision: 8,
        };
        assert!(error.to_string().contains("revision=8"));
    }
}
