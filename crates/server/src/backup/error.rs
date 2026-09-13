//! 备份／恢复／导出的错误类型（T20）。
//!
//! 分类与 CLI 退出码的对应关系（§T20 的退出码表；`config::commands` 负责转换）：
//!
//! | 变体 | 退出码 | 含义 |
//! | --- | --- | --- |
//! | [`BackupError::Path`] | 4 | 源／目标路径前置条件不满足（不存在、非空、不可写、互相嵌套、库比程序新） |
//! | [`BackupError::Locked`] | 5 | data-dir 排他锁被占用（backup 要求先停服） |
//! | [`BackupError::Integrity`] | 7 | 完整性校验失败（manifest 非法、sha256 不符、blob 损坏或缺失、外键/引用校验失败） |
//! | [`BackupError::Io`] | 1 | 运行时 I/O 失败（磁盘、权限等未归类错误） |
//! | [`BackupError::Storage`] | 4 | 数据库层错误（与 check/serve 的"data-dir 不可用"一致） |
//!
//! 消息面向运维人员，不含密钥、会话与原始资料内容（`code()` 是稳定的日志/测试标识）。

use crate::storage::StorageError;

/// 备份／恢复失败。
#[derive(Debug, Clone, PartialEq)]
pub enum BackupError {
    /// 路径／前置条件问题（CLI 退出码 4）。
    Path { message: String },
    /// 排他锁冲突（CLI 退出码 5；backup 要求先停止服务）。
    Locked { message: String },
    /// 完整性校验失败（CLI 退出码 7；失败不修改源数据、保留现场）。
    Integrity { code: &'static str, message: String },
    /// 运行时 I/O 失败（CLI 退出码 1）。
    Io { message: String },
    /// 数据库层错误（CLI 退出码 4）。
    Storage(StorageError),
}

impl BackupError {
    pub fn path(message: impl Into<String>) -> Self {
        Self::Path {
            message: message.into(),
        }
    }

    pub fn locked(message: impl Into<String>) -> Self {
        Self::Locked {
            message: message.into(),
        }
    }

    pub fn integrity(code: &'static str, message: impl Into<String>) -> Self {
        Self::Integrity {
            code,
            message: message.into(),
        }
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::Io {
            message: message.into(),
        }
    }

    /// 稳定错误码（日志与测试断言用；Integrity 使用自带的 `code`）。
    pub fn code(&self) -> &str {
        match self {
            Self::Path { .. } => "backup_path",
            Self::Locked { .. } => "backup_locked",
            Self::Integrity { code, .. } => code,
            Self::Io { .. } => "backup_io",
            Self::Storage(_) => "backup_storage",
        }
    }

    /// 是否为完整性校验失败（退出码 7）。
    pub fn is_integrity(&self) -> bool {
        matches!(self, Self::Integrity { .. })
    }
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path { message } | Self::Locked { message } | Self::Io { message } => {
                formatter.write_str(message)
            }
            Self::Integrity { message, .. } => formatter.write_str(message),
            Self::Storage(error) => write!(formatter, "存储错误：{error}"),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<StorageError> for BackupError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::Error> for BackupError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(StorageError::from(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrity_errors_are_classified_for_exit_code_7() {
        let error = BackupError::integrity("backup_blob_mismatch", "sha256 不符");
        assert!(error.is_integrity());
        assert_eq!(error.code(), "backup_blob_mismatch");
        assert!(!BackupError::path("x").is_integrity());
        assert!(!BackupError::locked("x").is_integrity());
        assert!(!BackupError::io("x").is_integrity());
    }
}
