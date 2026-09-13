//! Range 与条件请求解析（contracts.md §7 的 Range 合同）。
//!
//! 合同（QA 按此复核）：
//! - 完整 `GET` → 200；
//! - 合法**单区间** → 206 + 正确 `Content-Range` / `Content-Length`；
//! - 不可满足区间（起点 ≥ 文件大小，或 `-0` 后缀）→ 416 + `Content-Range: bytes */<size>`；
//! - 多区间与语法非法 → 首版忽略 Range 返回完整 200（**不**做错误拼接）；
//! - `HEAD` 无 body；`If-None-Match` 命中 → 304；`If-Range` 不匹配 → 返回完整 200；
//! - 不对 Range 响应做动态压缩（本应用不挂压缩中间件，测试断言无 `Content-Encoding`）。
//!
//! 资产不可变，因此 ETag 直接用内容 sha256 的强校验器：同内容任意 asset 共享同一 ETag。

/// 闭区间 `[start, end]`（字节偏移，含两端）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    /// 区间字节数。
    pub fn length(&self) -> u64 {
        self.end - self.start + 1
    }
}

/// `Range` 头的判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeDecision {
    /// 忽略 Range，返回完整表示。
    Full,
    /// 单个可满足区间 → 206。
    Partial(ByteRange),
    /// 可满足性失败 → 416。
    Unsatisfiable,
}

/// 解析 `Range` 头。任何语法问题都退化为 [`RangeDecision::Full`]（RFC 9110 允许忽略）。
pub fn parse_range_header(value: &str, len: u64) -> RangeDecision {
    let Some(spec) = value.trim().strip_prefix("bytes=") else {
        return RangeDecision::Full;
    };
    let mut parts = spec.split(',');
    let (Some(first), None) = (parts.next(), parts.next()) else {
        // 多区间：首版忽略并返回完整 200（不做拼接）。
        return RangeDecision::Full;
    };
    let first = first.trim();
    let Some((start_text, end_text)) = first.split_once('-') else {
        return RangeDecision::Full;
    };
    let start_text = start_text.trim();
    let end_text = end_text.trim();

    if start_text.is_empty() {
        // 后缀区间：`-N` 表示最后 N 字节。
        let Ok(suffix) = end_text.parse::<u64>() else {
            return RangeDecision::Full;
        };
        if suffix == 0 || len == 0 {
            return RangeDecision::Unsatisfiable;
        }
        let start = len.saturating_sub(suffix);
        return RangeDecision::Partial(ByteRange {
            start,
            end: len - 1,
        });
    }

    let Ok(start) = start_text.parse::<u64>() else {
        return RangeDecision::Full;
    };
    if end_text.is_empty() {
        if start >= len {
            return RangeDecision::Unsatisfiable;
        }
        return RangeDecision::Partial(ByteRange {
            start,
            end: len - 1,
        });
    }
    let Ok(end) = end_text.parse::<u64>() else {
        return RangeDecision::Full;
    };
    if start > end {
        // 语法非法（拒绝而非 416）。
        return RangeDecision::Full;
    }
    if start >= len {
        return RangeDecision::Unsatisfiable;
    }
    RangeDecision::Partial(ByteRange {
        start,
        end: end.min(len - 1),
    })
}

/// 内容 sha256 对应的强 ETag（带引号，符合 HTTP 校验器语法）。
pub fn etag_for_sha256(sha256: &str) -> String {
    format!("\"{sha256}\"")
}

/// `If-None-Match` 是否命中（命中 → 304）。支持 `*` 与逗号列表；弱比较。
pub fn if_none_match_hits(header: Option<&str>, etag: &str) -> bool {
    let Some(header) = header else {
        return false;
    };
    let header = header.trim();
    if header == "*" {
        return true;
    }
    let target = strip_weak(etag);
    header
        .split(',')
        .map(str::trim)
        .any(|candidate| strip_weak(candidate) == target)
}

/// `If-Range` 是否允许使用 Range：只有**强**校验器与当前 ETag 完全一致才返回 206。
///
/// 日期形式的 `If-Range`（或任何不一致的值）一律视为不匹配 → 返回完整 200。
pub fn if_range_allows(header: Option<&str>, etag: &str) -> bool {
    let Some(header) = header else {
        return false;
    };
    let header = header.trim();
    if header.starts_with("W/") {
        // 弱校验器不能用于 If-Range（RFC 9110 §13.1.5）。
        return false;
    }
    header == etag
}

fn strip_weak(tag: &str) -> &str {
    tag.trim().strip_prefix("W/").unwrap_or(tag.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_range_forms_are_satisfiable() {
        assert_eq!(
            parse_range_header("bytes=0-9", 100),
            RangeDecision::Partial(ByteRange { start: 0, end: 9 })
        );
        assert_eq!(
            parse_range_header("bytes=90-", 100),
            RangeDecision::Partial(ByteRange { start: 90, end: 99 })
        );
        assert_eq!(
            parse_range_header("bytes=-5", 100),
            RangeDecision::Partial(ByteRange { start: 95, end: 99 })
        );
        // 尾部超出：截到文件末尾。
        assert_eq!(
            parse_range_header("bytes=99-500", 100),
            RangeDecision::Partial(ByteRange { start: 99, end: 99 })
        );
        // 后缀长于文件：整份。
        assert_eq!(
            parse_range_header("bytes=-500", 100),
            RangeDecision::Partial(ByteRange { start: 0, end: 99 })
        );
    }

    #[test]
    fn unsatisfiable_ranges_are_reported() {
        assert_eq!(
            parse_range_header("bytes=100-", 100),
            RangeDecision::Unsatisfiable
        );
        assert_eq!(
            parse_range_header("bytes=1000-2000", 100),
            RangeDecision::Unsatisfiable
        );
        assert_eq!(
            parse_range_header("bytes=-0", 100),
            RangeDecision::Unsatisfiable
        );
        assert_eq!(
            parse_range_header("bytes=0-", 0),
            RangeDecision::Unsatisfiable
        );
    }

    #[test]
    fn multi_range_and_broken_syntax_fall_back_to_full() {
        assert_eq!(
            parse_range_header("bytes=0-1,5-6", 100),
            RangeDecision::Full
        );
        assert_eq!(parse_range_header("items=0-1", 100), RangeDecision::Full);
        assert_eq!(parse_range_header("bytes=abc", 100), RangeDecision::Full);
        assert_eq!(parse_range_header("bytes=5-1", 100), RangeDecision::Full);
        assert_eq!(parse_range_header("bytes=", 100), RangeDecision::Full);
    }

    #[test]
    fn conditional_headers_compare_strong_etags() {
        let etag = etag_for_sha256("abc123");
        assert_eq!(etag, "\"abc123\"");
        assert!(if_none_match_hits(Some("\"abc123\""), &etag));
        assert!(if_none_match_hits(Some("W/\"abc123\""), &etag));
        assert!(if_none_match_hits(Some("\"other\", \"abc123\""), &etag));
        assert!(if_none_match_hits(Some("*"), &etag));
        assert!(!if_none_match_hits(Some("\"other\""), &etag));
        assert!(!if_none_match_hits(None, &etag));

        assert!(if_range_allows(Some("\"abc123\""), &etag));
        assert!(!if_range_allows(Some("\"other\""), &etag));
        assert!(
            !if_range_allows(Some("W/\"abc123\""), &etag),
            "弱校验器不能用于 If-Range"
        );
        assert!(
            !if_range_allows(Some("Wed, 21 Oct 2015 07:28:00 GMT"), &etag),
            "日期形式无法与强 ETag 比较 → 返回完整表示"
        );
        assert!(!if_range_allows(None, &etag));
    }

    #[test]
    fn range_length_is_inclusive() {
        assert_eq!(ByteRange { start: 0, end: 9 }.length(), 10);
        assert_eq!(ByteRange { start: 99, end: 99 }.length(), 1);
    }
}
