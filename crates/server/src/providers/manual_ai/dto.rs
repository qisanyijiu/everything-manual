//! 说明书 AI 的**线上 DTO**：Responses 请求构造与响应解析（T14 / REQ-029）。
//!
//! 请求形态（contracts.md §6、architecture.md §5.2）：
//! - `model` 来自任务快照的冻结配置（**不自动跟随“最新模型”**）；
//! - `input` 含 `input_text` 与必要的 `input_image`（JPEG data URL）；
//! - **`text.format.type = json_schema`**（不使用 Chat Completions 的 `response_format`）、
//!   `name = manual_extract_v1`、`strict = true`，schema 由
//!   [`manual_core::knowledge::manual_extract_json_schema`] 构造（全部 required、
//!   `additionalProperties=false`、可选值 nullable）；
//! - `max_output_tokens` 来自冻结发送范围，且不超过每批上限；
//! - `store = false`：按支持情况关闭远端响应存储（隐私选项不等于供应商零留存承诺）。
//!
//! 响应解析（**另行处理 refusal / incomplete**，不依赖 SDK 便利字段）：
//! - 解析 REST `output[].content[]` 中的 `output_text`（可能多段，按顺序拼接）；
//! - `output[].content[].type = refusal` 单独识别；
//! - `status` / `incomplete_details.reason` 原样保留用于结论；
//! - `usage` 原样保留（token 计量；不是 USD 计费事实）。
//!
//! 响应信封的未知字段一律忽略（供应商会新增字段）；**模型输出本身**相反：必须
//! 严格符合 schema（由 `manual_core::knowledge` 校验，未知字段 = 违规）。

use serde::Serialize;

use manual_core::knowledge::MANUAL_EXTRACT_SCHEMA_NAME;

// ---------------------------------------------------------------------------
// 请求
// ---------------------------------------------------------------------------

/// Responses API 的输入内容项（`input[].content[]`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContent {
    /// 页文字/提示词（`{"type":"input_text","text":…}`）。
    InputText { text: String },
    /// 页图（`{"type":"input_image","image_url":"data:image/jpeg;base64,…"}`）。
    InputImage { image_url: String },
}

/// Responses API 的输入消息（首版为单条 user 消息）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InputMessage {
    pub role: &'static str,
    pub content: Vec<InputContent>,
}

impl InputMessage {
    pub fn user(content: Vec<InputContent>) -> Self {
        Self {
            role: "user",
            content,
        }
    }
}

/// `text.format` 的 JSON Schema 包装。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JsonSchemaFormat {
    #[serde(rename = "type")]
    pub format_type: &'static str,
    pub name: String,
    pub strict: bool,
    pub schema: serde_json::Value,
}

/// `text` 参数（Responses 的 structured outputs 入口）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextFormat {
    pub format: JsonSchemaFormat,
}

/// 一次批次提取请求（字段顺序固定 → 序列化字节确定 → `request_hash` 稳定）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtractRequest {
    pub model: String,
    pub input: Vec<InputMessage>,
    pub text: TextFormat,
    pub max_output_tokens: i64,
    /// 关闭远端响应存储（按模型支持情况；不假定可轮询/重取 `response_id`）。
    pub store: bool,
}

impl ExtractRequest {
    /// 构造请求（`max_output_tokens` 由调用方校验后传入）。
    pub fn new(model: &str, content: Vec<InputContent>, max_output_tokens: i64) -> Self {
        Self {
            model: model.to_owned(),
            input: vec![InputMessage::user(content)],
            text: TextFormat {
                format: JsonSchemaFormat {
                    format_type: "json_schema",
                    name: MANUAL_EXTRACT_SCHEMA_NAME.to_owned(),
                    strict: true,
                    schema: manual_core::knowledge::manual_extract_json_schema(),
                },
            },
            max_output_tokens,
            store: false,
        }
    }

    /// 确定性请求字节（`request_hash` 与线上 body 同源）。
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("提取请求总是可序列化")
    }
}

// ---------------------------------------------------------------------------
// 响应
// ---------------------------------------------------------------------------

/// 供应商报告的 token 用量（原样保留；**不是** USD 计费事实）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
}

/// 解析后的响应（**尽力读取已知字段，不做任何“修复”**；原始字节由调用方保留）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedResponse {
    /// 供应商响应 id（opaque；**不假定可轮询/重取**）。
    pub id: Option<String>,
    pub model: Option<String>,
    /// `completed` / `incomplete` / `failed` / …（原样保留）。
    pub status: Option<String>,
    /// `incomplete_details.reason`（如 `max_output_tokens`）。
    pub incomplete_reason: Option<String>,
    /// 首个 refusal 文本。
    pub refusal: Option<String>,
    /// 全部 `output_text` 按出现顺序拼接（多段时拼接；**不**做局部提取）。
    pub output_text: Option<String>,
    /// `output_text` 段数（诊断用）。
    pub output_text_parts: usize,
    /// 是否出现 `output[].content[].type = refusal`。
    pub has_refusal: bool,
    pub usage: Option<ExtractUsage>,
}

impl ParsedResponse {
    /// 解析响应体。
    ///
    /// 失败（非 JSON / 顶层不是对象）返回描述性错误；调用方按
    /// `BatchOutcome::EnvelopeInvalid` 处理并保留原始响应字节作为诊断路径。
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|error| format!("响应不是合法 JSON：{error}"))?;
        let object = value
            .as_object()
            .ok_or_else(|| "响应顶层不是对象".to_owned())?;

        let mut parsed = ParsedResponse {
            id: object
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            model: object
                .get("model")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            status: object
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            ..Default::default()
        };
        parsed.incomplete_reason = object
            .get("incomplete_details")
            .and_then(|details| details.get("reason"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);

        if let Some(usage) = object.get("usage").and_then(serde_json::Value::as_object) {
            parsed.usage = Some(ExtractUsage {
                input_tokens: usage
                    .get("input_tokens")
                    .and_then(serde_json::Value::as_i64),
                output_tokens: usage
                    .get("output_tokens")
                    .and_then(serde_json::Value::as_i64),
                total_tokens: usage
                    .get("total_tokens")
                    .and_then(serde_json::Value::as_i64),
            });
        }

        let mut texts: Vec<String> = Vec::new();
        if let Some(output) = object.get("output").and_then(serde_json::Value::as_array) {
            for item in output {
                let Some(content) = item.get("content").and_then(serde_json::Value::as_array)
                else {
                    continue;
                };
                for part in content {
                    match part.get("type").and_then(serde_json::Value::as_str) {
                        Some("output_text") => {
                            let text = part
                                .get("text")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default()
                                .to_owned();
                            texts.push(text);
                        }
                        Some("refusal") => {
                            parsed.has_refusal = true;
                            if parsed.refusal.is_none() {
                                parsed.refusal = part
                                    .get("refusal")
                                    .and_then(serde_json::Value::as_str)
                                    .map(str::to_owned);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        parsed.output_text_parts = texts.len();
        if !texts.is_empty() {
            parsed.output_text = Some(texts.join(""));
        }
        Ok(parsed)
    }

    /// 状态是否为 `completed`（缺省 `status` 的兼容实现视为可接受）。
    pub fn is_completed(&self) -> bool {
        match self.status.as_deref() {
            None | Some("completed") => true,
            Some(_) => false,
        }
    }
}

/// JPEG data URL（`input_image` 的 `image_url`）。
pub fn jpeg_data_url(bytes: &[u8]) -> String {
    use base64_encode::encode;
    format!("data:image/jpeg;base64,{}", encode(bytes))
}

/// 最小 base64 编码（只在本项目内部使用；避免为一次编码引入新依赖）。
///
/// 输入是 JPEG 字节（任意二进制）；输出是标准 base64（带 `=` 填充）。
mod base64_encode {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(input: &[u8]) -> String {
        let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let triple = (b0 << 16) | (b1 << 8) | b2;
            output.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
            output.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
            if chunk.len() > 1 {
                output.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
            } else {
                output.push('=');
            }
            if chunk.len() > 2 {
                output.push(ALPHABET[(triple & 0x3F) as usize] as char);
            } else {
                output.push('=');
            }
        }
        output
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn base64_matches_rfc4648_vectors() {
            // RFC 4648 §10 的测试向量。
            assert_eq!(encode(b""), "");
            assert_eq!(encode(b"f"), "Zg==");
            assert_eq!(encode(b"fo"), "Zm8=");
            assert_eq!(encode(b"foo"), "Zm9v");
            assert_eq!(encode(b"foob"), "Zm9vYg==");
            assert_eq!(encode(b"fooba"), "Zm9vYmE=");
            assert_eq!(encode(b"foobar"), "Zm9vYmFy");
            // 任意二进制（JPEG 魔数）。
            assert_eq!(encode(&[0xFF, 0xD8, 0xFF]), "/9j/");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_bytes_use_text_format_not_response_format() {
        let request = ExtractRequest::new(
            "gpt-test",
            vec![InputContent::InputText {
                text: "你好".to_owned(),
            }],
            4096,
        );
        let value: serde_json::Value = serde_json::from_slice(&request.to_bytes()).unwrap();
        assert_eq!(value["model"], json!("gpt-test"));
        assert_eq!(value["input"][0]["role"], json!("user"));
        assert_eq!(value["input"][0]["content"][0]["type"], json!("input_text"));
        assert_eq!(value["text"]["format"]["type"], json!("json_schema"));
        assert_eq!(value["text"]["format"]["name"], json!("manual_extract_v1"));
        assert_eq!(value["text"]["format"]["strict"], json!(true));
        assert_eq!(value["max_output_tokens"], json!(4096));
        assert_eq!(value["store"], json!(false));
        assert!(
            value.get("response_format").is_none(),
            "不得使用 Chat Completions 的 response_format"
        );
        assert!(value.get("tools").is_none(), "模型没有工具执行权限");
        assert!(value.get("functions").is_none());
        assert!(value.get("web_search").is_none());
    }

    #[test]
    fn response_parsing_reads_output_text_and_refusal_separately() {
        let success = json!({
            "id": "resp_1",
            "status": "completed",
            "model": "gpt-test",
            "output": [
                { "type": "message", "content": [
                    { "type": "output_text", "text": "{\"a\":1}" },
                    { "type": "output_text", "text": "{\"b\":2}" }
                ] }
            ],
            "usage": { "input_tokens": 10, "output_tokens": 3, "total_tokens": 13 }
        })
        .to_string();
        let parsed = ParsedResponse::parse(success.as_bytes()).unwrap();
        assert_eq!(parsed.id.as_deref(), Some("resp_1"));
        assert_eq!(parsed.output_text.as_deref(), Some("{\"a\":1}{\"b\":2}"));
        assert_eq!(parsed.output_text_parts, 2);
        assert!(!parsed.has_refusal);
        assert!(parsed.is_completed());
        assert_eq!(parsed.usage.unwrap().total_tokens, Some(13));

        let refusal = json!({
            "id": "resp_2",
            "status": "completed",
            "output": [
                { "type": "message", "content": [
                    { "type": "refusal", "refusal": "无法识别" }
                ] }
            ]
        })
        .to_string();
        let parsed = ParsedResponse::parse(refusal.as_bytes()).unwrap();
        assert!(parsed.has_refusal);
        assert_eq!(parsed.refusal.as_deref(), Some("无法识别"));
        assert!(parsed.output_text.is_none());

        let incomplete = json!({
            "status": "incomplete",
            "incomplete_details": { "reason": "max_output_tokens" },
            "output": []
        })
        .to_string();
        let parsed = ParsedResponse::parse(incomplete.as_bytes()).unwrap();
        assert!(!parsed.is_completed());
        assert_eq!(
            parsed.incomplete_reason.as_deref(),
            Some("max_output_tokens")
        );

        // 非 JSON / 非对象 → 明确报错（调用方按 EnvelopeInvalid 处理）。
        assert!(ParsedResponse::parse(b"<html>").is_err());
        assert!(ParsedResponse::parse(b"[1,2]").is_err());
    }
}
