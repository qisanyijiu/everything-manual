//! 认证与会话 DTO（contracts.md §3 / REQ-002）。
//!
//! 安全约定：
//! - 明文会话 token **只出现在 `Set-Cookie`**，任何响应体/DTO 都不包含它；
//! - `csrfToken` 由会话 token 派生（见 `http::auth::tokens`），同样不落库（库里只有哈希）；
//! - `LoginRequest.password` 标记 `write_only`，不进入生成类型的读模型。

use manual_core::timestamps::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `POST /api/v1/auth/login` 请求体。**服务端不记录请求体**（PRD §5.7）。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct LoginRequest {
    /// 管理员密码。示例仅为占位符，禁止写入真实凭据。
    #[schema(write_only = true, example = "placeholder-not-a-real-password")]
    pub password: String,
}

/// 登录成功响应（`200`）。会话 token 只经 `Set-Cookie` 返回，响应体不含它。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoginResponse {
    pub data: SessionData,
}

/// `GET /api/v1/auth/session` 响应（`200`；`Cache-Control: no-store`）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionResponse {
    pub data: SessionData,
}

/// 会话恢复数据：供刷新页面后继续调用受保护 API。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionData {
    pub admin: AdminSummary,
    /// CSRF token：所有修改请求需通过 `X-CSRF-Token` 头回传（contracts.md §3）。
    pub csrf_token: String,
    /// 会话绝对过期时间（UTC RFC3339；默认 7 天，假设 A-05，无滑动续期）。
    #[schema(value_type = String, example = "2026-09-19T00:00:00Z")]
    pub expires_at: Timestamp,
}

/// 管理员摘要（只暴露公开 ID；口令哈希永不出现在任何响应）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminSummary {
    #[schema(example = "01993000-0000-7000-8000-000000000001")]
    pub id: String,
}
