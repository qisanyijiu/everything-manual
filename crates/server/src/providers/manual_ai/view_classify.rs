//! 视图候选分类（ADR-044）：把一张从说明书 PDF 拆出的候选图交给说明书 AI 判断
//! "它是不是产品外观图、若是更像 front/left/back/right/detail 中哪一个"。
//!
//! - 走与说明书提取相同的 Responses 适配（同一 base_url / 模型 / 服务端密钥，密钥不出服务端）；
//! - 严格 JSON Schema 输出；结果只作**建议**（候选的 `suggested_view` + `confidence`），
//!   最终排列由用户在视图排列页决定；
//! - 失败不阻塞：调用方把候选保存为"未判断"，用户仍可手动拖到槽位。

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::config::SecretString;

/// 分类结果。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewGuess {
    /// 是否为产品外观视图（线稿/照片均可）；表格、文字页、二维码、配件小图为 false。
    pub is_product_view: bool,
    /// front/left/back/right/detail；`is_product_view = false` 时为 null。
    pub view: Option<String>,
    pub confidence: f64,
    /// ≤ 60 字的简短理由（展示给用户核对）。
    pub reason: String,
}

const PROMPT: &str = "You are sorting candidate images cut out of a product user manual. \
Decide whether this image shows the PRODUCT ITSELF from the outside (a photo or a line drawing of the whole product or a large part of it). \
Tables, text, QR codes, icons, packaging, accessories alone, hands-only, and screenshots are NOT product views. \
An image that shows the product MORE THAN ONCE (several poses, before/after, step sequences or side-by-side comparisons) is NOT a product view either: \
it would be reconstructed as several products. \
If it is a product view, pick the camera direction relative to the product's own front (the side with the lens, screen, face or main controls): \
front (front or front three-quarter), left, back (rear or rear three-quarter), right, or detail (a close-up of one part / a cropped area). \
Give confidence 0..1 and a very short reason in Chinese (<=30 characters).";

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["isProductView", "view", "confidence", "reason"],
        "properties": {
            "isProductView": { "type": "boolean" },
            "view": { "type": ["string", "null"], "enum": ["front", "left", "back", "right", "detail", null] },
            "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
            "reason": { "type": "string", "maxLength": 60 }
        }
    })
}

/// 调用一次分类。`image` 为 JPEG/PNG 字节。
pub async fn classify(
    base_url: &str,
    api_key: &SecretString,
    model: &str,
    image: &[u8],
    mime: &str,
) -> Result<ViewGuess, String> {
    let base = crate::config::validate_origin_like("base_url", base_url).map_err(|e| e.message)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("构造 HTTP 客户端失败：{error}"))?;
    let data_url = super::dto::image_data_url(mime, image);
    let body = json!({
        "model": model,
        "input": [{ "role": "user", "content": [
            { "type": "input_text", "text": PROMPT },
            { "type": "input_image", "image_url": data_url }
        ]}],
        "text": { "format": { "type": "json_schema", "name": "view_guess", "strict": true, "schema": schema() } },
        "max_output_tokens": 400,
        "store": false
    });
    let response = client
        .post(format!("{base}/responses"))
        .bearer_auth(api_key.expose())
        .json(&body)
        .send()
        .await
        .map_err(|error| {
            format!(
                "请求失败：{}",
                crate::redaction::redact_text_urls(&error.without_url().to_string())
            )
        })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|_| "读取响应失败".to_owned())?;
    if !status.is_success() {
        return Err(format!("说明书 AI 返回 HTTP {}", status.as_u16()));
    }
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| "响应不是 JSON".to_owned())?;
    let output = value
        .get("output")
        .and_then(|o| o.as_array())
        .into_iter()
        .flatten()
        .flat_map(|item| {
            item.get("content")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default()
        })
        .filter(|content| content.get("type").and_then(|t| t.as_str()) == Some("output_text"))
        .filter_map(|content| {
            content
                .get("text")
                .and_then(|t| t.as_str())
                .map(str::to_owned)
        })
        .collect::<String>();
    let mut guess: ViewGuess =
        serde_json::from_str(&output).map_err(|_| "模型输出不符合约定格式".to_owned())?;
    if !guess.confidence.is_finite() {
        guess.confidence = 0.0;
    }
    guess.confidence = guess.confidence.clamp(0.0, 1.0);
    if !guess.is_product_view {
        guess.view = None;
    }
    if let Some(view) = &guess.view
        && !matches!(
            view.as_str(),
            "front" | "left" | "back" | "right" | "detail"
        )
    {
        guess.view = None;
    }
    guess.reason = guess.reason.chars().take(60).collect();
    Ok(guess)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_strict_and_closed() {
        let s = schema();
        assert_eq!(s["additionalProperties"], json!(false));
        assert_eq!(s["required"].as_array().unwrap().len(), 4);
    }
}
