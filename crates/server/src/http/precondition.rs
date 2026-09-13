//! `If-Match` 乐观锁工具（contracts.md §1）。
//!
//! 协议：
//! - 可编辑聚合根的 `GET` 返回 `ETag: "r<revision>"`；
//! - `PATCH`、确认、发布、重试／取消等修改请求必须携带 `If-Match`，其值必须是
//!   **具体的 revision**；校验发生在业务写入的同一个条件更新里（revision CAS）；
//! - 缺少 → `428` `PRECONDITION_REQUIRED`；值非法 → `422` `VALIDATION_FAILED`；
//!   过期 → `412` `REVISION_CONFLICT` 且 `details.currentRevision` 可见
//!   （412 由 [`super::error::ApiError::from_storage`] 从 `StorageError::RevisionConflict` 映射）。
//!
//! 接受的写法：`"r7"`（标准）与 `r7`（宽容解析，便于命令行调试）。
//! **不接受** `*`（通配会绕过乐观锁）与弱标签 `W/"r7"`（弱比较不适用于 If-Match）。

use axum::http::HeaderMap;

use super::error::ApiError;

/// `If-Match` 头名。
pub const IF_MATCH: &str = "if-match";

/// 生成 `ETag` 值（契约格式 `"r<revision>"`；`manual_core::domain::Item::etag` 同源）。
pub fn etag_value(revision: i64) -> String {
    format!("\"r{revision}\"")
}

/// 解析并校验 `If-Match`，返回期望的 revision。
///
/// 错误语义见模块文档（428 缺失 / 422 非法）。
pub fn parse_if_match(headers: &HeaderMap) -> Result<i64, ApiError> {
    let values: Vec<&str> = headers
        .get_all(IF_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    if values.is_empty() {
        return Err(ApiError::precondition_required(
            "缺少 If-Match：请先 GET 资源并用其 ETag（形如 \"r7\"）再发起修改",
        ));
    }
    let trimmed = values.join(",");
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        return Err(malformed("If-Match 不能为空"));
    }
    if trimmed.contains(',') {
        return Err(malformed(
            "If-Match 只接受单个具体 revision（不支持列表或多个头）",
        ));
    }
    if trimmed == "*" {
        return Err(malformed(
            "不支持 If-Match: *（通配会绕过乐观锁）：请携带具体 revision",
        ));
    }
    if trimmed.starts_with("W/") {
        return Err(malformed(
            "不支持弱标签（W/…）：If-Match 使用强比较，请携带 \"r<n>\"",
        ));
    }
    let inner = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(trimmed);
    let Some(digits) = inner.strip_prefix('r') else {
        return Err(malformed("If-Match 必须是 ETag 形式 \"r<n>\""));
    };
    match digits.parse::<i64>() {
        Ok(revision) if revision >= 1 => Ok(revision),
        _ => Err(malformed("If-Match 的 revision 必须是正整数（\"r1\" 起）")),
    }
}

fn malformed(message: &str) -> ApiError {
    ApiError::validation_failed(format!("{message}（当前值无法用于乐观锁校验）"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(value: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(IF_MATCH, HeaderValue::from_str(value).unwrap());
        map
    }

    #[test]
    fn accepts_quoted_and_bare_revisions() {
        assert_eq!(parse_if_match(&headers("\"r7\"")).unwrap(), 7);
        assert_eq!(parse_if_match(&headers("r1")).unwrap(), 1);
        assert_eq!(parse_if_match(&headers("  \"r12\"  ")).unwrap(), 12);
    }

    #[test]
    fn missing_header_is_precondition_required_428() {
        let error = parse_if_match(&HeaderMap::new()).unwrap_err();
        assert_eq!(error.status, axum::http::StatusCode::PRECONDITION_REQUIRED);
        assert_eq!(error.code.as_str(), "PRECONDITION_REQUIRED");
    }

    #[test]
    fn wildcard_weak_list_and_garbage_are_rejected_422() {
        for value in [
            "*",
            "W/\"r1\"",
            "\"r1\", \"r2\"",
            "\"r0\"",
            "\"r-1\"",
            "\"abc\"",
            "",
        ] {
            let error = parse_if_match(&headers(value)).unwrap_err();
            assert_eq!(
                error.status,
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "值 {value:?} 应被拒绝"
            );
            assert_eq!(error.code.as_str(), "VALIDATION_FAILED");
        }
    }

    #[test]
    fn etag_format_matches_contract() {
        assert_eq!(etag_value(7), "\"r7\"");
    }
}
