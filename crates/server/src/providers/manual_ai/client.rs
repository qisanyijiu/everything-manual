//! 说明书 AI HTTP 客户端（Responses；reqwest + rustls，ADR-010 的既有依赖集）。
//!
//! 约束（contracts.md §5/§6、architecture.md §5.2）：
//! - **不做任何自动重试**：本客户端没有重试层；一批提取在一次调用中只发**一次**请求。
//!   结果未知（传输失败/超时/含糊 5xx/响应未读完）一律按 `submission_unknown` 处理
//!   （合同：已发请求但没有持久化完整响应 → 未知，`response_id` **不假定可轮询/重取**）；
//! - **不跟随重定向**（`redirect::Policy::none()`）：避免把 `Authorization` 带到别处；
//! - **超时明确**：连接超时与整体超时（含响应体读取）；
//! - **响应体读取有上限**：异常大的响应停止读取并记为未知（不能证明请求未被接受）；
//! - **脱敏**：错误摘要不含 API key、完整签名 URL 或响应原文。
//!
//! 同步语义（contracts.md §5）：完整响应收到后，调用方必须**先持久化响应结果资产**，
//! 再在同一短事务保存 usage/receipt；`response_id` 只是 opaque 事实，不是可轮询任务。

use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use reqwest::redirect::Policy;

use crate::config::SecretString;

/// `POST {base_url}/responses` 的路径（默认 base_url 为 `https://api.openai.com/v1`）。
pub const MANUAL_AI_RESPONSES_PATH: &str = "/responses";
/// 响应体读取上限（提取结果 JSON 远小于此；防御异常巨大或恶意响应）。
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// 错误诊断里保留的文本上限（脱敏后）。
pub const MAX_ERROR_HINT_CHARS: usize = 200;

/// 超时配置（官方文档未给出 SLA 建议值；T23 按实测调整）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManualAiTimeouts {
    /// TCP/TLS 连接超时。
    pub connect: Duration,
    /// 单次提取请求的整体超时（含响应体读取；模型可能思考较久）。
    pub request: Duration,
}

impl Default for ManualAiTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            request: Duration::from_secs(180),
        }
    }
}

/// 调用供应商的错误分类（能否证明"请求未被接受"是唯一允许自动重试的判据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManualAiError {
    /// 传输层：连接失败、超时、连接中断、响应体不完整/超限。
    /// **不能证明请求未被接受**：同步批次一律按结果未知处理。
    Transport { detail: String },
    /// 429：可证明未被处理（尊重 `Retry-After`）。
    RateLimited { retry_after_seconds: Option<u64> },
    /// 5xx：含糊的服务器错误，**不能证明请求未被接受**。
    ServerError { status: u16 },
    /// 3xx：本客户端不跟随重定向（属配置/端点错误，请求未被处理）。
    Redirected { status: u16 },
    /// 非 429 的 4xx：供应商明确拒绝（可证明未被计费）。
    Business {
        http_status: u16,
        message: Option<String>,
    },
    /// 2xx 但响应不是 JSON 对象（**原始响应已完整收到**，由调用方保留为诊断）。
    Unexpected { detail: String },
}

impl ManualAiError {
    /// 稳定错误码（日志与测试断言用）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Transport { .. } => "transport",
            Self::RateLimited { .. } => "rateLimited",
            Self::ServerError { .. } => "serverError",
            Self::Redirected { .. } => "redirected",
            Self::Business { .. } => "business",
            Self::Unexpected { .. } => "unexpectedResponse",
        }
    }

    /// 相对 429 的重试指示（秒；HTTP-date 形式不解析）。
    pub fn retry_after_seconds(&self) -> Option<u64> {
        match self {
            Self::RateLimited {
                retry_after_seconds,
            } => *retry_after_seconds,
            _ => None,
        }
    }

    /// 能否**证明**请求未被供应商处理（只有这类错误允许自动重试）。
    pub fn is_definitively_refused(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Business { .. } | Self::Redirected { .. }
        )
    }

    /// 一行脱敏摘要（**不含** API key 与完整 URL 查询串）。
    pub fn redacted(&self) -> String {
        match self {
            Self::Transport { detail } => format!("传输失败：{}", redact_text(detail)),
            Self::RateLimited {
                retry_after_seconds,
            } => match retry_after_seconds {
                Some(seconds) => format!("429 限速（Retry-After={seconds}s）"),
                None => "429 限速".to_owned(),
            },
            Self::ServerError { status } => format!("供应商服务器错误（HTTP {status}）"),
            Self::Redirected { status } => format!(
                "供应商返回重定向（HTTP {status}）；本客户端不跟随重定向，请检查 base_url 配置"
            ),
            Self::Business {
                http_status,
                message,
            } => {
                let mut text = format!("供应商明确拒绝（HTTP {http_status}）");
                if let Some(message) = message.as_deref().filter(|value| !value.is_empty()) {
                    text.push_str(&format!("；message={message}"));
                }
                text
            }
            Self::Unexpected { detail } => {
                format!("响应不符合协议假设：{}", redact_text(detail))
            }
        }
    }
}

impl std::fmt::Display for ManualAiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.redacted())
    }
}

impl std::error::Error for ManualAiError {}

/// 文本脱敏：错误/诊断文本里的 URL 形态片段（`scheme://…`，可能含签名查询串）
/// 一律替换为 host + sha256 摘要标签（OB-9 口径，[`crate::redaction`] 是唯一实现点）；
/// 超长截断。裸 host 与 task_id 原样保留。
fn redact_text(text: &str) -> String {
    let redacted = crate::redaction::redact_text_urls(text);
    let mut chars: Vec<char> = redacted.chars().take(MAX_ERROR_HINT_CHARS).collect();
    if redacted.chars().count() > MAX_ERROR_HINT_CHARS {
        chars.push('…');
    }
    chars.into_iter().collect()
}

/// 一次成功到达的响应（完整字节 + HTTP 状态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// 说明书 AI 客户端（不持有业务状态；按配置构造）。
#[derive(Clone)]
pub struct ManualAiClient {
    http: reqwest::Client,
    base_url: String,
    api_key: SecretString,
    timeouts: ManualAiTimeouts,
}

impl ManualAiClient {
    /// 构造客户端；`base_url` 形如 `https://api.openai.com/v1`（测试指向本机 fixture）。
    pub fn new(
        base_url: &str,
        api_key: SecretString,
        timeouts: ManualAiTimeouts,
    ) -> Result<Self, String> {
        let base_url = base_url.trim().trim_end_matches('/');
        if base_url.is_empty() {
            return Err("base_url 为空".to_owned());
        }
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(format!("base_url 必须是 http(s):// 地址：{base_url}"));
        }
        if base_url.contains(['?', '#']) || base_url.contains(char::is_whitespace) {
            return Err(format!("base_url 不允许包含空白、查询串或片段：{base_url}"));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.request)
            // 不跟随重定向：3xx 归入"未被处理"，且不把 Authorization 转发到别的地址。
            .redirect(Policy::none())
            // 显式关闭 reqwest 0.13 的默认重试层（ADR-023 第 9 条）：同步批次的
            // "能否重发"只允许由上层按"能否证明未被接受"决定，客户端不得自行重发。
            .retry(reqwest::retry::never())
            .build()
            .map_err(|error| format!("构造 HTTP 客户端失败：{error}"))?;
        Ok(Self {
            http,
            base_url: base_url.to_owned(),
            api_key,
            timeouts,
        })
    }

    /// 生效的基础地址（脱敏展示用；不含密钥）。
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn timeouts(&self) -> ManualAiTimeouts {
        self.timeouts
    }

    /// `POST {base_url}/responses`：**单次**请求，无重试层。
    ///
    /// 返回完整响应字节（调用方据此解析并**先持久化结果资产**）；传输级失败、
    /// 5xx、429、4xx 分别按 [`ManualAiError`] 分类。
    pub async fn extract_batch(&self, body: &[u8]) -> Result<RawResponse, ManualAiError> {
        let url = format!("{}{MANUAL_AI_RESPONSES_PATH}", self.base_url);
        let request = self
            .http
            .request(reqwest::Method::POST, url)
            .bearer_auth(self.api_key.expose())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_vec());
        let response = request.send().await.map_err(classify_transport_error)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let body = read_body_capped(response, MAX_RESPONSE_BYTES).await?;

        if (300..400).contains(&status) {
            return Err(ManualAiError::Redirected { status });
        }
        if status == 429 {
            return Err(ManualAiError::RateLimited {
                retry_after_seconds: retry_after,
            });
        }
        if status >= 500 {
            return Err(ManualAiError::ServerError { status });
        }
        if !(200..300).contains(&status) {
            // 4xx：尽量读取 message（脱敏、截断）；不保留原始响应体作为诊断？
            // 4xx 是**完整响应**：原始字节仍由调用方保留（诊断路径）。
            let message = extract_error_message(&body);
            return Err(ManualAiError::Business {
                http_status: status,
                message,
            });
        }
        Ok(RawResponse { status, body })
    }

    /// 解析 2xx 响应（信封不可解析 → `Unexpected`；原始字节由调用方保留）。
    pub fn parse_success(
        response: &RawResponse,
    ) -> Result<super::dto::ParsedResponse, ManualAiError> {
        super::dto::ParsedResponse::parse(&response.body).map_err(|detail| {
            ManualAiError::Unexpected {
                detail: redact_text(&detail),
            }
        })
    }
}

/// 4xx 错误消息（脱敏 + 截断；只取 error.message，不保留原文）。
fn extract_error_message(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let message = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("message").and_then(serde_json::Value::as_str))?;
    let hint = redact_text(message.trim());
    (!hint.is_empty()).then_some(hint)
}

/// 传输层错误分类（超时/连接/请求/响应读取都归入 `Transport`）。
///
/// reqwest 的 `Display` 会追加 ` for url (<完整 URL，含查询串>)`（BUG-009）：先
/// [`reqwest::Error::without_url`] 去掉 URL，再走统一脱敏入口兜底。
fn classify_transport_error(error: reqwest::Error) -> ManualAiError {
    let kind = if error.is_timeout() {
        "超时"
    } else if error.is_connect() {
        "连接失败"
    } else if error.is_body() || error.is_decode() {
        "响应体读取失败"
    } else if error.is_request() {
        "请求发送失败"
    } else {
        "传输错误"
    };
    ManualAiError::Transport {
        detail: crate::redaction::redact_text_urls(&format!("{kind}：{}", error.without_url())),
    }
}

/// 读取响应体（有上限；超过上限即失败——不完整响应不能当作结果）。
async fn read_body_capped(
    mut response: reqwest::Response,
    cap: usize,
) -> Result<Vec<u8>, ManualAiError> {
    let mut body: Vec<u8> = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(classify_transport_error)?;
        let Some(chunk) = chunk else {
            break;
        };
        if body.len() + chunk.len() > cap {
            return Err(ManualAiError::Transport {
                detail: format!("响应体超过上限 {cap} 字节：不完整响应按结果未知处理"),
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_rejects_invalid_base_urls() {
        let key = || SecretString::new("canary-key");
        assert!(ManualAiClient::new("", key(), ManualAiTimeouts::default()).is_err());
        assert!(
            ManualAiClient::new("api.openai.com/v1", key(), ManualAiTimeouts::default()).is_err()
        );
        assert!(
            ManualAiClient::new("https://a.example?v=1", key(), ManualAiTimeouts::default())
                .is_err()
        );
        let client = ManualAiClient::new(
            "https://api.openai.com/v1/",
            key(),
            ManualAiTimeouts::default(),
        )
        .expect("合法 base_url");
        assert_eq!(client.base_url(), "https://api.openai.com/v1");
    }

    #[test]
    fn error_classification_separates_provable_refusals_from_unknowns() {
        assert!(
            ManualAiError::RateLimited {
                retry_after_seconds: Some(3)
            }
            .is_definitively_refused()
        );
        assert!(
            ManualAiError::Business {
                http_status: 400,
                message: Some("schema unsupported".to_owned()),
            }
            .is_definitively_refused()
        );
        assert!(ManualAiError::Redirected { status: 302 }.is_definitively_refused());

        assert!(
            !ManualAiError::Transport {
                detail: "连接失败".to_owned()
            }
            .is_definitively_refused()
        );
        assert!(!ManualAiError::ServerError { status: 503 }.is_definitively_refused());
        assert!(
            !ManualAiError::Unexpected {
                detail: "信封不可解析".to_owned()
            }
            .is_definitively_refused()
        );
    }

    #[test]
    fn redacted_errors_never_expose_keys_or_signing_urls() {
        let error = ManualAiError::Transport {
            detail: "错误：https://cdn.example.invalid/x?sign=super-secret-token 失败".to_owned(),
        };
        let text = error.redacted();
        assert!(!text.contains("super-secret-token"), "{text}");
        // OB-9 口径：错误文本里不允许保留 `scheme://…`（哪怕是 `?[redacted]` 形态）；
        // URL 之后的普通文本必须保留。
        assert!(!text.contains("://"), "{text}");
        assert!(text.contains("host=cdn.example.invalid"), "{text}");
        assert!(text.contains("失败"), "{text}");

        let business = ManualAiError::Business {
            http_status: 401,
            message: Some("Incorrect API key provided: sk-canary".to_owned()),
        };
        let text = business.redacted();
        assert!(text.contains("401"), "{text}");
        assert!(text.chars().count() <= MAX_ERROR_HINT_CHARS + 64, "{text}");

        let long = ManualAiError::Transport {
            detail: "x".repeat(10_000),
        };
        // redacted 只截断一次（内部使用），保证诊断不爆炸。
        assert!(long.redacted().chars().count() <= MAX_ERROR_HINT_CHARS + 32);
    }
}
