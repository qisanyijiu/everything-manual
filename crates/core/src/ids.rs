//! 实体 ID 约定（contracts.md §1）：应用实体 ID 由服务器生成 UUIDv7 字符串。
//!
//! 供应商 ID（remote_task_id / response_id 等）是 opaque string：不校验成 UUID、
//! 不截断、不改前缀，因此**不要**对它们调用本模块的校验函数。

use uuid::Uuid;

/// 生成新的实体 ID（UUIDv7，时间有序）。
///
/// 用于物品、资产、任务等应用实体；不要用于供应商返回的 opaque ID。
pub fn new_id() -> String {
    Uuid::now_v7().to_string()
}

/// 是否为合法的 UUID（应用实体 ID 的形状检查）。
pub fn is_valid_id(value: &str) -> bool {
    Uuid::parse_str(value).is_ok()
}

/// 校验应用实体 ID；错误信息只描述形状，不回显可疑内容。
pub fn validate_id(value: &str) -> Result<(), String> {
    if is_valid_id(value) {
        Ok(())
    } else {
        Err("ID 必须是 UUID 字符串（应用实体 ID 由服务器生成）".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_uuid_v7_and_unique() {
        let first = new_id();
        let second = new_id();
        assert!(is_valid_id(&first), "{first}");
        assert_ne!(first, second);
        // UUIDv7 的版本半字节固定为 7（RFC 9562）。
        let uuid = Uuid::parse_str(&first).unwrap();
        assert_eq!(uuid.get_version_num(), 7);
        assert!(validate_id(&first).is_ok());
        assert!(validate_id("task_abc123").is_err());
        assert!(validate_id("").is_err());
    }
}
