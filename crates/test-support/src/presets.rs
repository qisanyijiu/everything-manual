//! 供应商 fixture 预置：路径常量与开箱即用的场景服务器。
//!
//! 场景内容只存在于 `tests/fixtures/scenarios/*.json`（单一来源）；这里仅提供
//! 快捷入口，供 T12（Tripo 契约测试）与 T14（说明书 AI 契约测试）直接使用。
//! 后续卡新增场景时应加 JSON 文件而不是把行为写进 Rust。

use crate::server::FixtureServer;

/// Tripo v3 图片上传路径（contracts.md §6）。
pub const TRIPO_UPLOAD_PATH: &str = "/v3/files";
/// Tripo v3 多视图生成路径（付费 POST）。
pub const TRIPO_SUBMIT_PATH: &str = "/v3/generation/multiview-to-model";
/// Tripo v3 任务查询路径前缀（`/v3/tasks/<id>`）。
pub const TRIPO_TASKS_PREFIX: &str = "/v3/tasks/";
/// 说明书 AI 的 Responses 路径（OpenAI 兼容）。
pub const MANUAL_AI_RESPONSES_PATH: &str = "/v1/responses";

/// `tests/fixtures/scenarios/tripo_happy.json`：上传成功 → 提交成功 → 首次查询运行中、
/// 之后重复返回成功（`repeatLast: true`）。
pub fn tripo_happy() -> FixtureServer {
    FixtureServer::from_scenario_file("tripo_happy.json")
}

/// `tests/fixtures/scenarios/manual_ai_happy.json`：单批提取返回结构化成功响应。
pub fn manual_ai_happy() -> FixtureServer {
    FixtureServer::from_scenario_file("manual_ai_happy.json")
}
