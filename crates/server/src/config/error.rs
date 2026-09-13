//! CLI 错误与退出码约定（T02 对外合同，validation-release.md §5 的运维入口）。
//!
//! 退出码是机器可判定合同，CI／运维脚本依赖它；`--help`／`--version` 为 0，
//! 未知子命令与参数错误由 clap 归入 [`ExitCode::Usage`]（2）。
//!
//! | 退出码 | 含义 |
//! | --- | --- |
//! | 0 | 成功 |
//! | 1 | 运行时错误（监听失败、I/O 失败等未归类错误） |
//! | 2 | 用法错误（未知子命令/参数、缺少必需参数、密码来源不可用） |
//! | 3 | 配置错误（未知配置键、非法取值、路径不存在、密码/密钥文件不合规） |
//! | 4 | data-dir／路径错误（不存在、结构缺失、不可写、数据库/schema 不可用或比程序新；备份输出已存在；恢复目标非空） |
//! | 5 | 排他锁冲突（同一 data-dir 已被另一进程持有；`backup` 要求先停服） |
//! | 6 | 安全拒绝（非 loopback 且无 TLS/可信代理；内置 TLS 监听未实现） |
//! | 7 | **备份/恢复完整性校验失败**（原"功能未实现"，T20 起 backup/restore 已实现并改用此码：manifest 非法、sha256 不符、blob 损坏或缺失、外键/引用校验失败；失败不修改源数据、保留现场） |
//!
//! 错误消息本身是可读中文，不含堆栈与内部细节；是否敏感由调用方保证
//! （密码与密钥一律经 [`super::secret::SecretString`] 包装，不进入消息）。

use std::fmt;

/// 进程退出码。数值是稳定合同，测试与 QA 脚本按此断言。
///
/// 7 的含义在 T20 由"功能未实现"更新为"备份/恢复完整性校验失败"（见模块文档表），
/// 数值本身不变：备份/恢复脚本按"非零 = 失败、7 = 数据损坏"区分处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Success = 0,
    Internal = 1,
    Usage = 2,
    Config = 3,
    DataDir = 4,
    Locked = 5,
    InsecureListen = 6,
    Integrity = 7,
}

impl ExitCode {
    /// 转为进程退出码（0–7，均在 u8 范围内）。
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// 带退出码的 CLI 错误。`message` 面向运维人员，禁止包含密码、密钥或原始资料。
#[derive(Debug, Clone)]
pub struct CliError {
    pub exit_code: ExitCode,
    pub message: String,
}

impl CliError {
    pub fn new(exit_code: ExitCode, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ExitCode::Usage, message)
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new(ExitCode::Config, message)
    }

    pub fn data_dir(message: impl Into<String>) -> Self {
        Self::new(ExitCode::DataDir, message)
    }

    pub fn locked(message: impl Into<String>) -> Self {
        Self::new(ExitCode::Locked, message)
    }

    pub fn insecure_listen(message: impl Into<String>) -> Self {
        Self::new(ExitCode::InsecureListen, message)
    }

    /// 备份/恢复完整性校验失败（退出码 7；失败保留现场，不修改源数据）。
    pub fn integrity(message: impl Into<String>) -> Self {
        Self::new(ExitCode::Integrity, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ExitCode::Internal, message)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CliError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_stable_contract_values() {
        assert_eq!(ExitCode::Success.as_u8(), 0);
        assert_eq!(ExitCode::Internal.as_u8(), 1);
        assert_eq!(ExitCode::Usage.as_u8(), 2);
        assert_eq!(ExitCode::Config.as_u8(), 3);
        assert_eq!(ExitCode::DataDir.as_u8(), 4);
        assert_eq!(ExitCode::Locked.as_u8(), 5);
        assert_eq!(ExitCode::InsecureListen.as_u8(), 6);
        // T20：7 从"未实现"更新为"备份/恢复完整性校验失败"（数值不变）。
        assert_eq!(ExitCode::Integrity.as_u8(), 7);
    }

    #[test]
    fn helpers_set_expected_codes() {
        assert_eq!(CliError::usage("x").exit_code, ExitCode::Usage);
        assert_eq!(CliError::config("x").exit_code, ExitCode::Config);
        assert_eq!(CliError::locked("x").exit_code, ExitCode::Locked);
        assert_eq!(CliError::integrity("x").exit_code, ExitCode::Integrity);
    }
}
