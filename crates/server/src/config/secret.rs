//! 密钥与敏感文本处理（PRD §5.7）。
//!
//! 约定：
//! - 任何密钥/密码在内存中一律用 [`SecretString`] 包装：`Debug` 输出固定为
//!   `[redacted]`，不实现 `Display`/`Serialize`，读取必须显式调用 [`SecretString::expose`]；
//!   使“把密钥打进日志或错误消息”在类型层面成为显式动作。
//! - URL 进入日志前必须经 [`redact_url_query`] 去掉查询串（签名 URL 的签名在查询串里）。
//! - [`redact_text`] 用于对已构建好的文本做兜底替换：把已知密钥字面量替换为 `[redacted]`。

use std::fmt;

/// 敏感的字符串（密码、API 密钥）。不实现 `Serialize`；`Debug` 恒为 `[redacted]`。
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// 显式取出明文。调用点必须确认用途（如发给供应商的 Authorization 头），不得写入日志。
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// 长度（字符数）——仅用于校验提示，不泄露内容。
    pub fn char_count(&self) -> usize {
        self.0.chars().count()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// 去掉 URL 的查询串与片段，保留 scheme/host/path。
///
/// 供应商签名 URL 的凭据在查询串中（REQ-028／PRD §5.7），日志与错误消息只允许出现
/// 去掉查询串后的形式。没有查询串时原样返回。
pub fn redact_url_query(url: &str) -> String {
    let cut = url.find(['?', '#']);
    match cut {
        Some(index) => format!("{}?[redacted]", &url[..index]),
        None => url.to_owned(),
    }
}

/// 把文本中出现的已知密钥字面量替换为 `[redacted]`。
///
/// 兜底手段：正常路径不应把密钥拼进文本；此函数用于“万一”场景（例如第三方错误消息
/// 回显了 token）。空字符串不参与替换，避免把整段文本打散。
pub fn redact_text(text: &str, secrets: &[&SecretString]) -> String {
    let mut output = text.to_owned();
    for secret in secrets {
        let value = secret.expose();
        if !value.is_empty() {
            output = output.replace(value, "[redacted]");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_never_prints_plaintext() {
        let secret = SecretString::new("sk-live-super-secret-value");
        let debug = format!("{secret:?}");
        assert_eq!(debug, "[redacted]");
        assert!(!debug.contains("sk-live"));
        // 结构体派生 Debug 同样被脱敏。
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            api_key: SecretString,
        }
        let holder = Holder {
            api_key: secret.clone(),
        };
        let rendered = format!("{holder:?}");
        assert!(!rendered.contains("super-secret"), "{rendered}");
    }

    #[test]
    fn redact_url_query_strips_query_and_fragment() {
        assert_eq!(
            redact_url_query("https://cdn.example.com/model.glb?token=abc&expires=1"),
            "https://cdn.example.com/model.glb?[redacted]"
        );
        assert_eq!(
            redact_url_query("https://cdn.example.com/a#frag"),
            "https://cdn.example.com/a?[redacted]"
        );
        assert_eq!(
            redact_url_query("https://openapi.tripo3d.ai/v3"),
            "https://openapi.tripo3d.ai/v3"
        );
        assert_eq!(redact_url_query(""), "");
    }

    #[test]
    fn redact_text_replaces_known_secrets_only() {
        let key = SecretString::new("sk-secret-123");
        let text = "上游返回 401：token sk-secret-123 无效";
        let redacted = redact_text(text, &[&key]);
        assert_eq!(redacted, "上游返回 401：token [redacted] 无效");

        // 空密钥不参与替换（否则会把文本替换成碎片）。
        let empty = SecretString::new("");
        assert_eq!(redact_text("abc", &[&empty]), "abc");
    }

    #[test]
    fn expose_and_char_count_are_explicit() {
        let secret = SecretString::new("pässword");
        assert_eq!(secret.expose(), "pässword");
        assert_eq!(secret.char_count(), 8);
    }
}
