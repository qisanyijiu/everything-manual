//! 说明书问答（`POST /items/{id}/releases/{releaseId}/chat`）。
//!
//! - 资料只来自该 release 的**不可变 manifest**（部件/步骤/规格，含人工修订）与其说明书准备的
//!   页文字层；不读会变的草稿；
//! - 对话历史由前端保存并整段回传（服务端不落库），条数与长度有上限；
//! - 未配置说明书 AI → 409 `PROVIDER_NOT_CONFIGURED`（不回落 mock）；上游失败 → 502，
//!   真实原因只进服务端日志；
//! - 外部调用前先归还数据库连接（池只有 4 个）。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing};
use manual_core::ApiErrorCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqliteConnection;
use utoipa::ToSchema;

use crate::providers::manual_ai::chat::{self, ChatRole, ChatTurn};
use crate::releases::service as releases_service;
use crate::storage::repo;

use super::error::{ApiError, RequestId};
use super::state::AppState;

/// 一次请求最多携带的历史消息条数（含本轮问题）。
pub const MAX_MESSAGES: usize = 20;
/// 单条消息字符上限。
pub const MAX_MESSAGE_CHARS: usize = 2000;
/// 原文页文字进入资料的总字符上限（超出部分截断，知识条目始终完整）。
const MAX_PAGE_TEXT_CHARS: usize = 60_000;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/items/{id}/releases/{release_id}/chat",
        routing::post(chat_release),
    )
}

/// 对话消息。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatMessageDto {
    /// `user` 或 `assistant`。
    pub role: String,
    pub content: String,
}

/// 问答请求：完整对话历史，最后一条必须是 `user`。
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessageDto>,
}

/// 问答结果。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatAnswerDto {
    pub answer: String,
    /// 支撑回答的原文页码（1-based，已限定在说明书页数内）。
    pub cited_pages: Vec<i64>,
    /// 页码所属说明书（release 的第一份文档）；没有文档时为 null。
    #[schema(nullable = true)]
    pub document_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatResponse {
    pub data: ChatAnswerDto,
}

fn validate(messages: &[ChatMessageDto]) -> Result<Vec<ChatTurn>, ApiError> {
    if messages.is_empty() || messages.len() > MAX_MESSAGES {
        return Err(ApiError::validation_failed(format!(
            "messages 须为 1..={MAX_MESSAGES} 条"
        )));
    }
    let mut turns = Vec::with_capacity(messages.len());
    for message in messages {
        let role = match message.role.as_str() {
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            _ => return Err(ApiError::validation_failed("role 只能是 user 或 assistant")),
        };
        let content = message.content.trim();
        if content.is_empty() || content.chars().count() > MAX_MESSAGE_CHARS {
            return Err(ApiError::validation_failed(format!(
                "每条消息须为 1..={MAX_MESSAGE_CHARS} 个字符"
            )));
        }
        turns.push(ChatTurn {
            role,
            content: content.to_owned(),
        });
    }
    if turns.last().map(|turn| turn.role) != Some(ChatRole::User) {
        return Err(ApiError::validation_failed("最后一条消息必须是用户的问题"));
    }
    Ok(turns)
}

fn text_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

fn pages_of(entity: &Value) -> String {
    let pages: Vec<String> = entity
        .get("evidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("pageNumber").and_then(Value::as_i64))
        .map(|p| p.to_string())
        .collect();
    if pages.is_empty() {
        String::new()
    } else {
        format!("（出处：第 {} 页）", pages.join("、"))
    }
}

/// 把 manifest 的知识（叠加冻结的人工修订）整理成纯文本资料。
pub fn knowledge_context(manifest: &Value) -> String {
    let knowledge = &manifest["knowledge"]["knowledge"];
    let edits = &manifest["review"]["entities"];
    let edited = |id: &str, key: &str| edits[id]["userEdited"].get(key).cloned();
    let mut out = String::new();
    if let Some(parts) = knowledge["parts"].as_array() {
        out.push_str("【部件】\n");
        for part in parts {
            let id = text_of(part, "id");
            let name = edited(&id, "name")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| text_of(part, "name"));
            let description = edited(&id, "description")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| text_of(part, "description"));
            out.push_str(&format!("- {name}：{description}{}\n", pages_of(part)));
        }
    }
    if let Some(steps) = knowledge["steps"].as_array() {
        out.push_str("【操作步骤】\n");
        for (index, step) in steps.iter().enumerate() {
            let id = text_of(step, "id");
            let title = edited(&id, "title")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| text_of(step, "title"));
            let actions = edited(&id, "orderedActions")
                .map(|v| strings(Some(&v)))
                .unwrap_or_else(|| strings(step.get("orderedActions")));
            let notes = edited(&id, "safetyNotes")
                .map(|v| strings(Some(&v)))
                .unwrap_or_else(|| strings(step.get("safetyNotes")));
            out.push_str(&format!("{}. {title}{}\n", index + 1, pages_of(step)));
            for (n, action) in actions.iter().enumerate() {
                out.push_str(&format!("   {}) {action}\n", n + 1));
            }
            for note in notes {
                out.push_str(&format!("   注意：{note}\n"));
            }
        }
    }
    if let Some(specs) = knowledge["specs"].as_array() {
        out.push_str("【规格】\n");
        for spec in specs {
            let id = text_of(spec, "id");
            let label = edited(&id, "label")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| text_of(spec, "label"));
            let value = edited(&id, "value")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| text_of(spec, "value"));
            out.push_str(&format!("- {label}：{value}{}\n", pages_of(spec)));
        }
    }
    out
}

/// 读取准备记录的页文字层（缺失/非 UTF-8 的页跳过；总量有上限）。
async fn page_texts(
    conn: &mut SqliteConnection,
    data_dir: &std::path::Path,
    preparation_id: &str,
) -> String {
    let Ok(pages) = repo::preparations::list_pages(&mut *conn, preparation_id).await else {
        return String::new();
    };
    let mut out = String::new();
    let mut budget = MAX_PAGE_TEXT_CHARS;
    for page in pages {
        let Some(asset_id) = page.text_asset_id.as_deref() else {
            continue;
        };
        let Ok(Some((_asset, blob))) = repo::assets::get_with_blob(&mut *conn, asset_id).await
        else {
            continue;
        };
        let Ok(bytes) =
            crate::providers::manual_ai::store::read_blob_bytes(&mut *conn, data_dir, &blob.sha256)
                .await
        else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let taken: String = text.chars().take(budget).collect();
        budget -= taken.chars().count();
        out.push_str(&format!("—— 第 {} 页 ——\n{taken}\n", page.page_number));
        if budget == 0 {
            out.push_str("（后续页文字过长已省略）\n");
            break;
        }
    }
    out
}

/// `POST /api/v1/items/{id}/releases/{releaseId}/chat`。
#[utoipa::path(
    post,
    path = "/api/v1/items/{id}/releases/{releaseId}/chat",
    tag = "releases",
    summary = "说明书问答（依据该发布版本的知识与原文页文字）",
    description = "把完整对话历史（最后一条为用户问题）发给已配置的说明书 AI，回答只依据该 release \
                   冻结的部件/步骤/规格与说明书页文字，并给出原文页码。对话不在服务端保存；\
                   每次调用都会产生一次说明书 AI 请求费用。",
    params(
        ("id" = String, Path, description = "物品 ID（UUIDv7）"),
        ("releaseId" = String, Path, description = "发布版本 ID（UUIDv7）"),
    ),
    request_body = ChatRequest,
    security(("sessionCookie" = [])),
    responses(
        (status = 200, description = "回答", body = ChatResponse),
        (status = 401, description = "未登录", body = super::dto::ApiErrorResponse),
        (status = 404, description = "发布版本不存在", body = super::dto::ApiErrorResponse),
        (status = 409, description = "说明书 AI 未配置", body = super::dto::ApiErrorResponse),
        (status = 422, description = "消息不合法", body = super::dto::ApiErrorResponse),
        (status = 502, description = "说明书 AI 调用失败", body = super::dto::ApiErrorResponse),
    )
)]
pub async fn chat_release(
    State(state): State<AppState>,
    request_id: RequestId,
    Path((item_id, release_id)): Path<(String, String)>,
    Json(body): Json<ChatRequest>,
) -> Response {
    let turns = match validate(&body.messages) {
        Ok(turns) => turns,
        Err(error) => return error.render(&request_id),
    };
    let provider = state
        .provider_config()
        .read()
        .await
        .active_providers()
        .manual_ai
        .clone();
    let (Some(key), Some(model)) = (provider.api_key.clone(), provider.model.clone()) else {
        let mut missing = Vec::new();
        if provider.api_key.is_none() {
            missing.push("manual_ai.api_key".to_owned());
        }
        if provider.model.is_none() {
            missing.push("manual_ai.model".to_owned());
        }
        return ApiError::provider_not_configured(missing).render(&request_id);
    };

    let (context, page_count, document_id) = {
        let mut conn = match super::estimates::acquire(&state).await {
            Ok(conn) => conn,
            Err(error) => return error.render(&request_id),
        };
        let release = match releases_service::read_release(&mut conn, &item_id, &release_id).await {
            Ok(release) => release,
            Err(error) => return ApiError::from(error).render(&request_id),
        };
        let data_dir = state.settings().data_dir.clone();
        let manifest = match releases_service::read_manifest(&mut conn, &data_dir, &release).await {
            Ok((manifest, _)) => manifest,
            Err(error) => return ApiError::from(error).render(&request_id),
        };
        let document = manifest["documents"].get(0).cloned().unwrap_or(Value::Null);
        let document_id = document["documentId"].as_str().map(str::to_owned);
        let mut page_count = 0;
        let mut pages = String::new();
        if let Some(preparation_id) = document["preparationId"].as_str() {
            if let Ok(Some(preparation)) = repo::preparations::get(&mut conn, preparation_id).await
            {
                page_count = preparation.page_count.unwrap_or(0);
            }
            pages = page_texts(&mut conn, &data_dir, preparation_id).await;
        }
        let title = document["title"].as_str().unwrap_or("产品说明书");
        let mut context = format!(
            "说明书：{title}（共 {page_count} 页）\n\n{}",
            knowledge_context(&manifest)
        );
        if !pages.is_empty() {
            context.push_str("\n【原文页文字】\n");
            context.push_str(&pages);
        }
        (context, page_count, document_id)
        // conn 在此归还：下面是可能持续数十秒的外部调用。
    };

    match chat::ask(
        &provider.base_url,
        &key,
        &model,
        &context,
        &turns,
        page_count,
    )
    .await
    {
        Ok(answer) => (
            StatusCode::OK,
            Json(ChatResponse {
                data: ChatAnswerDto {
                    answer: answer.answer,
                    cited_pages: answer.cited_pages,
                    document_id,
                },
            }),
        )
            .into_response(),
        Err(reason) => {
            tracing::warn!(requestId = %request_id, releaseId = %release_id, reason = %reason, "说明书问答失败");
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                ApiErrorCode::Internal,
                "说明书 AI 暂时无法回答，请稍后重试",
            )
            .render(&request_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_requires_trailing_user_and_limits() {
        let msg = |role: &str, content: &str| ChatMessageDto {
            role: role.into(),
            content: content.into(),
        };
        assert!(validate(&[]).is_err());
        assert!(validate(&[msg("assistant", "hi")]).is_err());
        assert!(validate(&[msg("system", "hi")]).is_err());
        assert!(validate(&[msg("user", "   ")]).is_err());
        assert!(validate(&[msg("user", &"字".repeat(MAX_MESSAGE_CHARS + 1))]).is_err());
        assert_eq!(validate(&[msg("user", "怎么充电")]).unwrap().len(), 1);
    }

    #[test]
    fn knowledge_context_applies_frozen_user_edits() {
        let manifest = json!({
            "knowledge": { "knowledge": {
                "parts": [{ "id": "p1", "name": "头", "description": "相机", "evidence": [{ "pageNumber": 3 }] }],
                "steps": [{ "id": "s1", "title": "充电", "orderedActions": ["插线"], "safetyNotes": ["勿运动"], "evidence": [] }],
                "specs": [{ "id": "k1", "label": "重量", "value": "8kg", "evidence": [] }]
            }},
            "review": { "entities": { "p1": { "userEdited": { "name": "头部" } } } }
        });
        let text = knowledge_context(&manifest);
        assert!(text.contains("- 头部：相机（出处：第 3 页）"), "{text}");
        assert!(
            text.contains("1. 充电\n   1) 插线\n   注意：勿运动"),
            "{text}"
        );
        assert!(text.contains("- 重量：8kg"), "{text}");
    }
}
