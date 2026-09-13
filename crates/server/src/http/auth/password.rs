//! 管理员口令的 Argon2id 哈希与校验（REQ-002、architecture.md §7）。
//!
//! **参数（固定写明，不随环境变化）**：`Argon2::default()`
//! = Argon2id、版本 0x13（v19）、`m_cost = 19456 KiB (19 MiB)`、`t_cost = 2`、
//! `p_cost = 1`、输出 32 字节，编码为 PHC 字符串
//! （`$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`）。这组参数是 OWASP 对
//! Argon2id 的推荐档位，也是 RustCrypto `argon2` 0.5 的默认值。
//!
//! 其他约定：
//! - 盐 16 字节，取自操作系统熵源（`getrandom`），每行独立；
//! - 库里**只存 PHC 字符串**（`admins.password_hash`），明文只存在于进程内存；
//! - 哈希/校验是 CPU 密集操作（约 20 MiB 内存 + 毫秒级），调用方在异步上下文里
//!   必须经 `tokio::task::spawn_blocking`，不阻塞运行时线程。

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

/// 盐长度（字节）。`SaltString::encode_b64` 输出无填充的 B64。
pub const SALT_LENGTH: usize = 16;

/// 人类可读的参数说明（用于 `init` 输出与文档，不含任何口令信息）。
pub const PARAMETERS_SUMMARY: &str = "Argon2id v19、m=19456 KiB、t=2、p=1、16 字节盐";

/// 口令哈希/校验错误。消息不含口令内容。
#[derive(Debug)]
pub enum PasswordError {
    /// 无法获得安全随机盐。
    Entropy(String),
    /// 哈希计算失败（参数或编码问题）。
    Hash(String),
    /// 库中存储的 PHC 字符串无法解析（数据损坏/手工改写）。
    StoredHashInvalid(String),
}

impl std::fmt::Display for PasswordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Entropy(detail) => write!(f, "无法获得随机盐：{detail}"),
            Self::Hash(detail) => write!(f, "口令哈希失败：{detail}"),
            Self::StoredHashInvalid(_) => write!(f, "已存储的口令哈希无法解析（数据可能被损坏）"),
        }
    }
}

impl std::error::Error for PasswordError {}

/// 计算 PHC 字符串（Argon2id，参数见模块文档）。
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    let mut salt_bytes = [0u8; SALT_LENGTH];
    getrandom::fill(&mut salt_bytes).map_err(|error| PasswordError::Entropy(error.to_string()))?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|error| PasswordError::Hash(error.to_string()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| PasswordError::Hash(error.to_string()))
}

/// 校验口令；`Ok(false)` 表示不匹配（正常业务分支），`Err` 表示存储的哈希不可用。
pub fn verify_password(password: &str, stored_phc: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(stored_phc)
        .map_err(|error| PasswordError::StoredHashInvalid(error.to_string()))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(error) => Err(PasswordError::Hash(error.to_string())),
    }
}

/// 存储的哈希是否是本卡认可的 Argon2id PHC 字符串（`init` 与测试的自检）。
pub fn is_supported_hash(stored_phc: &str) -> bool {
    PasswordHash::new(stored_phc)
        .map(|parsed| parsed.algorithm.as_str() == "argon2id")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_argon2id_phc_and_verifies() {
        let phc = hash_password("correct horse battery staple").unwrap();
        assert!(
            phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "PHC 参数必须与文档一致：{phc}"
        );
        assert!(is_supported_hash(&phc));
        assert!(!phc.contains("correct horse battery staple"));
        assert!(verify_password("correct horse battery staple", &phc).unwrap());
        assert!(!verify_password("wrong password", &phc).unwrap());
    }

    #[test]
    fn salt_is_random_per_hash() {
        let first = hash_password("same-password").unwrap();
        let second = hash_password("same-password").unwrap();
        assert_ne!(first, second, "相同口令也必须产生不同盐与哈希");
    }

    #[test]
    fn broken_stored_hash_is_an_error_not_a_match() {
        assert!(!is_supported_hash("not-a-phc-string"));
        assert!(verify_password("x", "not-a-phc-string").is_err());
    }
}
