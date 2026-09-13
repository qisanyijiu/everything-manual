//! 时间约定（contracts.md §1）。
//!
//! - **存储**：SQLite 列为 INTEGER，Unix epoch 毫秒（UTC）。统一整数让租约、
//!   会话过期、`next_run_at` 等比较不依赖文本格式（见 decisions.md ADR-012）。
//! - **线上**：JSON / `ETag` / 日志中的时间为 UTC RFC3339 字符串；由本类型的
//!   serde 实现负责两种表示的转换。
//!
//! 不引入 `chrono`/`time::serde` 依赖到存储层：唯一的时间表示就是本类型。

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// 当前时间（Unix epoch 毫秒）。
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// UTC 时间戳，精度毫秒。
///
/// 序列化为 RFC3339 字符串（如 `2026-09-12T01:02:03.456Z`），反序列化同样接受
/// RFC3339；`ETag` 与日志直接使用 [`Timestamp::to_rfc3339`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Timestamp(i64);

impl Timestamp {
    /// Unix epoch（1970-01-01T00:00:00Z）。
    pub const EPOCH: Self = Self(0);

    /// 从 Unix epoch 毫秒构造。
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// 当前时间。
    pub fn now() -> Self {
        Self(now_millis())
    }

    /// Unix epoch 毫秒。
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// 加上（或减去）毫秒数；溢出返回 `None`（避免租约时间静默回绕）。
    pub const fn checked_add_millis(self, millis: i64) -> Option<Self> {
        match self.0.checked_add(millis) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// 是否已到／超过本时间戳（`now >= self`）。
    pub fn is_past(self) -> bool {
        Self::now() >= self
    }

    /// 转为 UTC RFC3339 字符串。
    pub fn to_rfc3339(self) -> String {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(self.0) * 1_000_000)
            .expect("i64 毫秒总能表示为 OffsetDateTime")
            .format(&Rfc3339)
            .expect("RFC3339 格式化不会失败")
    }

    /// 解析 UTC RFC3339 字符串（必须是 UTC 或带偏移量的合法时间）。
    pub fn from_rfc3339(text: &str) -> Result<Self, TimestampParseError> {
        let parsed = OffsetDateTime::parse(text, &Rfc3339)
            .map_err(|_| TimestampParseError(text.to_owned()))?;
        let nanos = parsed.unix_timestamp_nanos();
        // 毫秒精度：向下取整到毫秒，避免解析出存储层无法表达的精度。
        let millis = nanos.div_euclid(1_000_000);
        i64::try_from(millis)
            .map(Self)
            .map_err(|_| TimestampParseError(text.to_owned()))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_rfc3339())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_rfc3339(&text).map_err(serde::de::Error::custom)
    }
}

/// RFC3339 解析失败。错误信息包含原始输入（时间戳不是秘密，便于表单回显定位）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimestampParseError(String);

impl fmt::Display for TimestampParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "不是合法的 UTC RFC3339 时间：{}", self.0)
    }
}

impl std::error::Error for TimestampParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trip_and_serde() {
        let stamp = Timestamp::from_millis(1_787_000_000_123);
        let text = stamp.to_rfc3339();
        assert_eq!(text, "2026-08-17T20:53:20.123Z");
        assert_eq!(Timestamp::from_rfc3339(&text).unwrap(), stamp);

        let json = serde_json::to_string(&stamp).unwrap();
        assert_eq!(json, format!("\"{text}\""));
        assert_eq!(serde_json::from_str::<Timestamp>(&json).unwrap(), stamp);
        assert!(Timestamp::from_rfc3339("2026-08-16 02:13:20").is_err());
        assert!(serde_json::from_str::<Timestamp>("\"nope\"").is_err());
    }

    #[test]
    fn plain_and_fractional_seconds_both_parse() {
        let plain = Timestamp::from_rfc3339("2026-09-12T00:00:00Z").unwrap();
        assert_eq!(plain.as_millis(), 1_789_171_200_000);
        assert_eq!(Timestamp::from_rfc3339(&plain.to_rfc3339()).unwrap(), plain);
    }

    #[test]
    fn arithmetic_is_checked() {
        let base = Timestamp::from_millis(1_000);
        assert_eq!(base.checked_add_millis(500).unwrap().as_millis(), 1_500);
        assert!(
            Timestamp::from_millis(i64::MAX)
                .checked_add_millis(1)
                .is_none()
        );
        let now = Timestamp::now();
        assert!(
            now.as_millis() > 1_700_000_000_000,
            "当前时间应晚于 2023 年"
        );
    }
}
