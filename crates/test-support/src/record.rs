//! 请求记录与脱敏（AC-014：记录方法/路径/请求头（脱敏）/请求体/次数）。
//!
//! 脱敏规则：敏感头（授权、Cookie、API key 等）的值替换为 `[REDACTED]`；
//! `Authorization: Bearer xxx` 保留 scheme 前缀（`Bearer [REDACTED]`），
//! 这样后续卡仍能断言"是否携带 bearer"而不接触密钥明文。
//! 记录里的 `Debug` 输出同样只含脱敏值，测试可以整体断言不含 canary。

use serde_json::Value;

/// 记录到的请求头（值已按敏感名单脱敏）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderView {
    /// 原始大小写的头名。
    pub name: String,
    /// 脱敏后的值（敏感头为 `[REDACTED]` 或 `<Scheme> [REDACTED]`）。
    pub value: String,
    /// 是否发生了脱敏。
    pub redacted: bool,
}

/// 敏感请求头名单（大小写不敏感）。命中即脱敏，不记录明文。
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "x-auth-token",
];

/// 保留 scheme 前缀的脱敏头（`Bearer x` → `Bearer [REDACTED]`）。
const SCHEME_PREFIXED: &[&str] = &["authorization", "proxy-authorization"];

/// 按名字脱敏一个头值；返回 `(脱敏后的值, 是否脱敏)`。
pub fn redact_header(name: &str, value: &str) -> (String, bool) {
    let lower = name.to_ascii_lowercase();
    if !SENSITIVE_HEADERS.contains(&lower.as_str()) {
        return (value.to_owned(), false);
    }
    if SCHEME_PREFIXED.contains(&lower.as_str())
        && let Some(scheme) = value.split_whitespace().next()
        && value.split_whitespace().count() > 1
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return (format!("{scheme} [REDACTED]"), true);
    }
    ("[REDACTED]".to_owned(), true)
}

/// 记录到的"请求处理结果"（用于断言"缺脚本必须失败"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingOutcome {
    /// 命中第 `route_index` 个路由的第 `step_index` 步。
    Scripted {
        route_index: usize,
        step_index: usize,
    },
    /// 没有任何路由匹配（返回 501 并记录）。
    NoRoute,
    /// 路由匹配但步骤已耗尽且未声明 `repeatLast`（返回 501 并记录）。
    ScriptExhausted { route_index: usize },
}

/// 一次完整记录（按到达顺序）。
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    /// 到达序号（从 0 开始，进程内单调）。
    pub sequence: usize,
    pub method: String,
    /// 含查询串的原始 request-target。
    pub target: String,
    /// 不含查询串的路径。
    pub path: String,
    pub query: Option<String>,
    /// 请求头（脱敏）。名字按原样保存，查用大小写不敏感。
    pub headers: Vec<HeaderView>,
    pub body: Vec<u8>,
    pub outcome: RecordingOutcome,
}

impl RecordedRequest {
    /// 大小写不敏感地取头。
    pub fn header(&self, name: &str) -> Option<&HeaderView> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
    }

    /// 大小写不敏感地取头值（已脱敏）。
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.header(name).map(|header| header.value.as_str())
    }

    /// 请求体文本（lossless 到 UTF-8；非 UTF-8 时替换非法序列）。
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// 解析请求体 JSON；失败时 panic 并带上方法/路径与原文，便于定位"发错了什么"。
    pub fn json_body(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!(
                "{} {} 的请求体不是合法 JSON（{error}）：{}",
                self.method,
                self.path,
                self.body_text()
            )
        })
    }

    /// 一行摘要（断言失败信息里使用）。
    pub fn summary(&self) -> String {
        format!(
            "#{} {} {} ({} 字节)",
            self.sequence,
            self.method,
            self.target,
            self.body.len()
        )
    }
}
