//! Tripo v3 HTTP 客户端（reqwest；architecture.md §3 固定"reqwest 0.13 + rustls"）。
//!
//! 约束（T12 卡 / contracts.md §6）：
//! - **不做通用自动重试**：本客户端没有重试层、没有 tower 重试中间件；付费 POST
//!   （`submit_multiview`）在一次调用中只发**一次**请求。是否安全重试由上层按
//!   "能否证明请求未被接受"决定（429 可重试；网络中断/超时/含糊 5xx 一律结果未知）；
//! - **不跟随重定向**（`redirect::Policy::none()`）：避免把 `Authorization` 带到
//!   别的地址；3xx 归入"明确未接受"的配置错误；
//! - **明确超时**：连接超时与整体超时（含响应体读取）；上传图片用更长的整体超时；
//! - **响应体读取有上限**：异常大的响应直接截断并标记，不无界读入内存；
//! - **脱敏**：错误摘要不含 API key、不含完整签名 URL 查询串；响应原文不进日志；
//! - token / task ID 是 opaque string：不做 UUID 校验、不截断；进入 URL 路径时
//!   按 RFC 3986 的 unreserved 集合逐字节百分号编码。

use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use reqwest::redirect::Policy;

use crate::config::SecretString;

use super::dto::{
    Envelope, SubmitData, TaskData, UploadData, parse_envelope, submit_data, task_data, upload_data,
};

/// `POST /files`（上传图片；multipart 字段 `file`）。
pub const TRIPO_UPLOAD_PATH: &str = "/files";
/// `POST /generation/multiview-to-model`（**付费**提交；body 见 [`super::dto::SubmitRequest`]）。
pub const TRIPO_SUBMIT_PATH: &str = "/generation/multiview-to-model";
/// `GET /tasks/{task_id}` 的路径前缀。
pub const TRIPO_TASKS_PATH_PREFIX: &str = "/tasks/";
/// multipart 上传的字段名（contracts.md §6：`file`）。
pub const UPLOAD_FIELD_NAME: &str = "file";

/// 响应体读取上限（API 响应远小于此；防御异常巨大或恶意响应）。
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// 错误诊断里保留的响应文本上限（脱敏后）。
pub const MAX_ERROR_HINT_CHARS: usize = 200;

/// 超时配置（本项目取舍：官方文档未给出 SLA 建议值；T23 按实测调整）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TripoTimeouts {
    /// TCP/TLS 连接超时。
    pub connect: Duration,
    /// 提交/查询等 JSON 请求的整体超时（含响应体读取）。
    pub request: Duration,
    /// 图片上传的整体超时（单图 ≤20 MiB，慢上行需要更长时间）。
    pub upload: Duration,
}

impl Default for TripoTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            request: Duration::from_secs(60),
            upload: Duration::from_secs(180),
        }
    }
}

/// 调用供应商的错误分类（"能否证明请求未被接受"在 [`TripoError::is_definitively_refused`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TripoError {
    /// 传输层：连接失败、超时、连接中断、响应体不完整。
    /// **不能证明请求未被接受**（付费 POST 出现它必须按结果未知处理）。
    Transport { detail: String },
    /// 429 Too Many Requests（可证明未被接受；尊重 `Retry-After`）。
    RateLimited { retry_after_seconds: Option<u64> },
    /// 5xx：含糊的服务器错误，**不能证明请求未被接受**。
    ServerError { status: u16 },
    /// 3xx：本客户端不跟随重定向（不把凭据带到别处）；属配置/端点错误。
    Redirected { status: u16 },
    /// 业务拒绝：HTTP 200 且 `code != 0`，或非 429 的 4xx。
    Business {
        http_status: u16,
        code: Option<i64>,
        message: Option<String>,
        suggestion: Option<String>,
    },
    /// 2xx 且 `code == 0`，但响应结构不满足本卡的协议假设（缺 token/task_id/status）。
    Unexpected { detail: String },
}

impl TripoError {
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

    /// 相对 429 的重试指示（秒；HTTP-date 形式不解析，返回 `None`）。
    pub fn retry_after_seconds(&self) -> Option<u64> {
        match self {
            Self::RateLimited {
                retry_after_seconds,
            } => *retry_after_seconds,
            _ => None,
        }
    }

    /// 能否**证明**请求未被供应商接受（只有这类错误才允许自动再提交付费请求）。
    ///
    /// - 429 / 非 429 的 4xx / 业务错误 code：供应商明确拒绝（无 task ID 产生）；
    /// - 3xx：本客户端不跟随重定向，请求未被处理；
    /// - 传输失败、含糊 5xx、协议不符：无法证明 → `false`。
    pub fn is_definitively_refused(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Business { .. } | Self::Redirected { .. }
        )
    }

    /// 业务错误的可读摘要（脱敏；供 `last_error` 与日志）。
    pub fn business_summary(&self) -> Option<String> {
        match self {
            Self::Business {
                http_status,
                code,
                message,
                suggestion,
            } => {
                let mut text = format!("供应商拒绝请求（HTTP {http_status}");
                if let Some(code) = code {
                    text.push_str(&format!("，code={code}"));
                }
                text.push(')');
                // 供应商文案可能内嵌 URL（含签名查询串）：统一走脱敏入口（OB-9 口径）。
                if let Some(message) = message.as_deref() {
                    text.push_str(&format!("；message={}", redact_text(message)));
                }
                if let Some(suggestion) = suggestion.as_deref() {
                    text.push_str(&format!("；suggestion={}", redact_text(suggestion)));
                }
                Some(text)
            }
            Self::Redirected { status } => Some(format!(
                "供应商返回重定向（HTTP {status}）；本客户端不跟随重定向，请检查 base_url 配置"
            )),
            _ => None,
        }
    }

    /// 一行脱敏摘要（**不含** API key；URL 形式的文本去掉查询串）。
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
            Self::Redirected { status } => format!("供应商返回重定向（HTTP {status}）"),
            Self::Business { .. } => self
                .business_summary()
                .unwrap_or_else(|| "供应商业务错误".to_owned()),
            Self::Unexpected { detail } => format!("响应不符合协议假设：{}", redact_text(detail)),
        }
    }
}

impl std::fmt::Display for TripoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.redacted())
    }
}

impl std::error::Error for TripoError {}

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

/// Tripo v3 客户端（每个 job 执行期间共享；不持有任何业务状态）。
#[derive(Clone)]
pub struct TripoClient {
    http: reqwest::Client,
    base_url: String,
    api_key: SecretString,
    timeouts: TripoTimeouts,
}

impl TripoClient {
    /// 构造客户端；`base_url` 形如 `https://openapi.tripo3d.ai/v3`（区域地址由部署配置）。
    pub fn new(
        base_url: &str,
        api_key: SecretString,
        timeouts: TripoTimeouts,
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
            // 不跟随重定向：3xx 归入"未被接受"，且不把 Authorization 转发到别的地址。
            .redirect(Policy::none())
            // 显式关闭 reqwest 0.13 的默认重试层（T13 落实 QA 回合 14 的 P3 建议）：
            // 默认策略（Classifier::ProtocolNacks）在当前 feature 组合（无 http2/http3）下恒不重试，
            // 但"启用 http2 即改变付费安全边界"——付费 POST 的能否重发只允许由上层按
            // "能否证明请求未被接受"决定（ADR-006/ADR-022），不允许 HTTP 客户端自行重发。
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

    pub fn timeouts(&self) -> TripoTimeouts {
        self.timeouts
    }

    /// `POST /files`：multipart 字段 `file`，返回 opaque token。
    ///
    /// 上传不是付费操作（credits 由生成计费）：传输失败可安全重试，由上层决定。
    pub async fn upload_image(
        &self,
        file_name: &str,
        mime: &str,
        bytes: Vec<u8>,
    ) -> Result<UploadData, TripoError> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name.to_owned())
            .mime_str(mime)
            .map_err(|error| TripoError::Transport {
                detail: format!("构造 multipart 失败：{error}"),
            })?;
        let form = reqwest::multipart::Form::new().part(UPLOAD_FIELD_NAME, part);
        let request = self
            .request(reqwest::Method::POST, TRIPO_UPLOAD_PATH)
            .multipart(form)
            .timeout(self.timeouts.upload);
        let envelope = self.execute(request).await?;
        upload_data(envelope.data.as_ref()).map_err(|detail| TripoError::Unexpected { detail })
    }

    /// `POST /generation/multiview-to-model`：**付费**提交，返回远端 `task_id`。
    ///
    /// `body` 必须是 [`super::dto::SubmitRequest::to_bytes`] 的确定性字节（与落库的
    /// `request_hash` 同源）。本方法在一次调用中只发一次请求，绝不自动重发。
    pub async fn submit_multiview(&self, body: &[u8]) -> Result<SubmitData, TripoError> {
        let request = self
            .request(reqwest::Method::POST, TRIPO_SUBMIT_PATH)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_vec());
        let envelope = self.execute(request).await?;
        submit_data(envelope.data.as_ref()).map_err(|detail| TripoError::Unexpected { detail })
    }

    /// `GET /tasks/{task_id}`：查询远端任务（task ID 为 opaque string，进路径前转义）。
    pub async fn get_task(&self, task_id: &str) -> Result<TaskData, TripoError> {
        let path = format!("{TRIPO_TASKS_PATH_PREFIX}{}", encode_path_segment(task_id));
        let request = self.request(reqwest::Method::GET, &path);
        let envelope = self.execute(request).await?;
        task_data(envelope.data.as_ref()).map_err(|detail| TripoError::Unexpected { detail })
    }

    /// 组装带鉴权与基础地址的请求（**每个请求只加一次** `Authorization`）。
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        debug_assert!(path.starts_with('/'), "路径需以 / 开头：{path}");
        let url = format!("{}{}", self.base_url, path);
        self.http
            .request(method, url)
            .bearer_auth(self.api_key.expose())
    }

    /// 发请求 → 读取有上限的响应体 → 状态/信封分类。
    async fn execute(&self, request: reqwest::RequestBuilder) -> Result<Envelope, TripoError> {
        let response = request.send().await.map_err(classify_transport_error)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let body = read_body_capped(response, MAX_RESPONSE_BYTES).await?;

        if (300..400).contains(&status) {
            return Err(TripoError::Redirected { status });
        }
        if status == 429 {
            return Err(TripoError::RateLimited {
                retry_after_seconds: retry_after,
            });
        }
        if status >= 500 {
            return Err(TripoError::ServerError { status });
        }
        if !(200..300).contains(&status) {
            // 4xx：尽量读取业务信封的 message/suggestion（脱敏）；不保留原始响应体。
            let (code, message, suggestion) = match parse_envelope(&body) {
                Ok(envelope) => (Some(envelope.code), envelope.message, envelope.suggestion),
                Err(_) => (None, bounded_hint(&body), None),
            };
            return Err(TripoError::Business {
                http_status: status,
                code,
                message,
                suggestion,
            });
        }
        let envelope = parse_envelope(&body).map_err(|detail| TripoError::Unexpected { detail })?;
        if envelope.code != 0 {
            // HTTP 200 但 code 非 0 = 业务错误（只判断 HTTP 状态是不够的）。
            return Err(TripoError::Business {
                http_status: status,
                code: Some(envelope.code),
                message: envelope.message.clone(),
                suggestion: envelope.suggestion.clone(),
            });
        }
        Ok(envelope)
    }
}

/// 传输层错误分类（超时/连接/请求/响应读取都归入 `Transport`）。
///
/// reqwest 的 `Display` 会追加 ` for url (<完整 URL，含查询串>)`（BUG-009）：先
/// [`reqwest::Error::without_url`] 去掉 URL，再走统一脱敏入口兜底（同
/// `assets::glb::download::classify_transport`）。
fn classify_transport_error(error: reqwest::Error) -> TripoError {
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
    TripoError::Transport {
        detail: crate::redaction::redact_text_urls(&format!("{kind}：{}", error.without_url())),
    }
}

/// 读取响应体（有上限；超过上限截断并标记，不无界读入内存）。
async fn read_body_capped(
    mut response: reqwest::Response,
    cap: usize,
) -> Result<Vec<u8>, TripoError> {
    let mut body: Vec<u8> = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|error| {
            // 读体中失败：连接中断/超时 → 不能证明请求未被接受。
            classify_transport_error(error)
        })?;
        let Some(chunk) = chunk else {
            break;
        };
        let remaining = cap.saturating_sub(body.len());
        if remaining == 0 {
            // 超过上限：停止读取（不把异常巨大响应读进内存）。
            return Err(TripoError::Unexpected {
                detail: format!("响应体超过上限 {cap} 字节，已停止读取"),
            });
        }
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    Ok(body)
}

/// 非 JSON 错误的文本提示（脱敏 + 截断；仅用于诊断）。
fn bounded_hint(body: &[u8]) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&body[..body.len().min(4096)]).into_owned();
    let printable: String = text
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .collect();
    let hint = redact_text(printable.trim());
    (!hint.is_empty()).then_some(hint)
}

/// 把 opaque 字符串编码成一个 URL 路径段（RFC 3986 unreserved 之外逐字节百分号编码）。
pub fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if unreserved {
            encoded.push(char::from(*byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn client_rejects_invalid_base_urls() {
        let key = || SecretString::new("canary-key");
        assert!(TripoClient::new("", key(), TripoTimeouts::default()).is_err());
        assert!(
            TripoClient::new("openapi.tripo3d.ai/v3", key(), TripoTimeouts::default()).is_err()
        );
        assert!(
            TripoClient::new("https://a.example?v=1", key(), TripoTimeouts::default()).is_err()
        );

        let client = TripoClient::new(
            "https://openapi.tripo3d.ai/v3/",
            key(),
            TripoTimeouts::default(),
        )
        .expect("合法 base_url");
        assert_eq!(client.base_url(), "https://openapi.tripo3d.ai/v3");
    }

    #[test]
    fn error_classification_separates_provable_refusals_from_unknowns() {
        // 可证明未被接受：429 / 4xx / 业务 code / 3xx。
        assert!(
            TripoError::RateLimited {
                retry_after_seconds: Some(3)
            }
            .is_definitively_refused()
        );
        assert!(
            TripoError::Business {
                http_status: 200,
                code: Some(1201),
                message: Some("invalid image token".to_owned()),
                suggestion: None,
            }
            .is_definitively_refused()
        );
        assert!(TripoError::Redirected { status: 302 }.is_definitively_refused());

        // 不能证明未被接受：传输失败 / 5xx / 协议不符。
        assert!(
            !TripoError::Transport {
                detail: "连接失败".to_owned()
            }
            .is_definitively_refused()
        );
        assert!(!TripoError::ServerError { status: 503 }.is_definitively_refused());
        assert!(
            !TripoError::Unexpected {
                detail: "缺少 task_id".to_owned()
            }
            .is_definitively_refused()
        );
    }

    #[test]
    fn redacted_errors_never_expose_signing_urls_or_keys() {
        let error = TripoError::Transport {
            detail: "错误：https://cdn.example.invalid/model.glb?sign=super-secret-token 下载失败"
                .to_owned(),
        };
        let text = error.redacted();
        assert!(!text.contains("super-secret-token"), "{text}");
        // OB-9 口径：错误文本里不允许保留 `scheme://…`（哪怕是 `?[redacted]` 形态）。
        assert!(!text.contains("://"), "{text}");
        assert!(text.contains("host=cdn.example.invalid"), "{text}");
        assert!(
            text.contains("下载失败"),
            "URL 之后的普通文本必须保留：{text}"
        );

        // 供应商业务文案内嵌 URL（HTTP 200 + code!=0 路径）同样要脱敏。
        let business = TripoError::Business {
            http_status: 200,
            code: Some(1201),
            message: Some(
                "model https://cdn.example.invalid/m.glb?sign=super-secret-token 已过期".to_owned(),
            ),
            suggestion: None,
        };
        let summary = business.redacted();
        assert!(!summary.contains("super-secret-token"), "{summary}");
        assert!(!summary.contains("://"), "{summary}");
        assert!(summary.contains("已过期"), "{summary}");

        let long = TripoError::Unexpected {
            detail: "x".repeat(10_000),
        };
        let rendered = long.redacted();
        assert!(
            rendered.chars().count() <= MAX_ERROR_HINT_CHARS + 32,
            "超长诊断必须被截断（实际 {} 字符）",
            rendered.chars().count()
        );
        assert!(rendered.ends_with('…'), "截断需有省略标记");
    }

    #[test]
    fn opaque_task_id_is_percent_encoded_as_a_single_path_segment() {
        assert_eq!(encode_path_segment("task-0001"), "task-0001");
        assert_eq!(encode_path_segment("a/b"), "a%2Fb");
        assert_eq!(encode_path_segment("sp ace"), "sp%20ace");
        assert_eq!(encode_path_segment("前缀-1"), "%E5%89%8D%E7%BC%80-1");
    }

    #[test]
    fn envelope_errors_are_mapped_by_status() {
        // 直接验证分类函数依赖的构造（HTTP 状态语义不在单测里伪造请求）。
        let business = TripoError::Business {
            http_status: 403,
            code: None,
            message: Some("forbidden".to_owned()),
            suggestion: None,
        };
        assert_eq!(business.code(), "business");
        assert!(business.business_summary().unwrap().contains("403"));
        assert_eq!(
            TripoError::RateLimited {
                retry_after_seconds: Some(7)
            }
            .retry_after_seconds(),
            Some(7)
        );
        assert_eq!(
            TripoError::ServerError { status: 500 }.code(),
            "serverError"
        );
        assert_eq!(
            TripoError::Unexpected {
                detail: json!({"dataKeys": []}).to_string()
            }
            .code(),
            "unexpectedResponse"
        );
    }
}
