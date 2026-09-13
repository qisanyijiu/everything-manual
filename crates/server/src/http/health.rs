//! 存活／就绪探针（REQ-044、REQ-007、REQ-038）。
//!
//! - `GET /health/live`：进程可响应即 ok；不检查数据层，也不依赖云端；
//! - `GET /health/ready`：检查 `process` / `data_directory` / `database` / `migrations`
//!   四项，任一项失败 → `status = "not_ready"` **且 HTTP 503**（同一个 `{data}` 结构，
//!   便于运维直接看到哪一项失败）；**绝不检查云端可达**（供应商断网不影响就绪，
//!   已有说明书仍可读）；不输出配置细节（路径、密钥、版本号都不出现）。

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing};

use super::dto::{
    CheckStatus, HealthLiveResponse, HealthReadyResponse, LivenessData, LivenessStatus,
    ReadinessCheck, ReadinessCheckName, ReadinessData, ReadinessStatus,
};
use super::state::AppState;

/// 健康探针路由（无需会话）。
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/health/live", routing::get(live))
        .route("/health/ready", routing::get(ready))
}

/// `GET /api/v1/health/live` —— 存活探针：进程可响应即为 ok，不输出配置细节。
#[utoipa::path(
    get,
    path = "/api/v1/health/live",
    tag = "health",
    summary = "存活探针",
    description = "进程可响应请求即返回 200。不检查数据层，也不依赖云端可达。",
    responses((status = 200, description = "进程存活", body = HealthLiveResponse))
)]
pub async fn live() -> Json<HealthLiveResponse> {
    Json(HealthLiveResponse {
        data: LivenessData {
            status: LivenessStatus::Ok,
        },
    })
}

/// `GET /api/v1/health/ready` —— 就绪探针（数据层真实状态；云端不可达不影响）。
#[utoipa::path(
    get,
    path = "/api/v1/health/ready",
    tag = "health",
    summary = "就绪探针",
    description = "检查进程、数据目录、数据库与迁移版本；任一项失败返回 503 且 status=not_ready。\
                   不检查云端可达，也不返回配置细节。",
    responses(
        (status = 200, description = "数据层就绪", body = HealthReadyResponse),
        (status = 503, description = "数据层不可用（同一 {data} 结构，status=not_ready）", body = HealthReadyResponse),
    )
)]
pub async fn ready(State(state): State<AppState>) -> Response {
    let checks = vec![
        ReadinessCheck {
            name: ReadinessCheckName::Process,
            status: CheckStatus::Ok,
        },
        ReadinessCheck {
            name: ReadinessCheckName::DataDirectory,
            status: check_data_directory(state.database().data_dir()),
        },
        readiness_check(
            ReadinessCheckName::Database,
            check_database(&state).await.is_ok(),
        ),
        readiness_check(
            ReadinessCheckName::Migrations,
            check_migrations(&state).await.is_ok(),
        ),
    ];
    let all_ok = checks.iter().all(|check| check.status == CheckStatus::Ok);
    let data = ReadinessData {
        status: if all_ok {
            ReadinessStatus::Ready
        } else {
            ReadinessStatus::NotReady
        },
        checks,
    };
    let status = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(HealthReadyResponse { data })).into_response()
}

fn readiness_check(name: ReadinessCheckName, ok: bool) -> ReadinessCheck {
    ReadinessCheck {
        name,
        status: if ok {
            CheckStatus::Ok
        } else {
            CheckStatus::Fail
        },
    }
}

/// 数据目录检查：目录存在、**owner 可写**（Unix 权限位，只读目录会被识别）、
/// 且数据库文件在预期位置。纯只读探测，不写任何文件。
fn check_data_directory(data_dir: &std::path::Path) -> CheckStatus {
    let Ok(metadata) = std::fs::metadata(data_dir) else {
        return CheckStatus::Fail;
    };
    if !metadata.is_dir() || metadata.permissions().readonly() {
        return CheckStatus::Fail;
    }
    if !crate::storage::database_path(data_dir).is_file() {
        return CheckStatus::Fail;
    }
    CheckStatus::Ok
}

/// 数据库检查：连接池可用且能执行最小只读查询。
async fn check_database(state: &AppState) -> Result<(), ()> {
    let pool = state.database().pool();
    if pool.is_closed() {
        return Err(());
    }
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(pool)
        .await
        .map(|_| ())
        .map_err(|_| ())
}

/// 迁移检查：已应用版本必须等于程序支持版本（serve 启动时已自动迁移；不等即异常）。
async fn check_migrations(state: &AppState) -> Result<(), ()> {
    let applied = state
        .database()
        .applied_schema_version()
        .await
        .map_err(|_| ())?;
    if applied == state.database().program_schema_version() {
        Ok(())
    } else {
        Err(())
    }
}
