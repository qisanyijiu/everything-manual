//! Tripo v3 适配器（T12；REQ-027 / AC-041）。
//!
//! 组成：
//! - [`client`]：reqwest HTTP 客户端（超时、鉴权、**无自动重试**、不跟随重定向、
//!   响应体读取有上限、错误分类与脱敏）；
//! - [`dto`]：线上 DTO（信封 `code/data`、上传/提交/查询解析、多视图请求体构造、
//!   计费精确换算）；**不使用 v2 的 `model_version`/`files` 字段形态**；
//! - [`status`]：任务状态归一化（原始值 + 归一化值；`banned`/`expired` 与未知枚举）；
//! - [`handlers`]：五个阶段处理器（`tripo_upload` / `tripo_submit` / `tripo_poll` /
//!   `model_download` / `model_validate`），由 `serve` 按配置注册进 T10 执行器；
//! - [`links`]：`tripo_poll` → `model_download` 的**易失**临时下载链接（T20/BUG-008：
//!   签名 URL 不落库，下载阶段未命中就按 task ID 重新查询）。
//!
//! 接口分离（contracts.md §6）：`upload_image / submit_multiview / get_task`，
//! 领域层只接收归一化结果。模型下载（`download_model`）属 T13：本卡只保存
//! `output.model_url` 事实，不复用带 `Authorization` 的客户端去下载。
//!
//! 真实调用边界：本卡的验证全部指向本机 fixture（`127.0.0.1`，零外网）；
//! 真实收费调用只在 T23 的授权入口（AC-042）执行。

pub mod client;
pub mod dto;
pub mod handlers;
pub mod links;
pub mod status;

pub use client::{
    MAX_RESPONSE_BYTES, TRIPO_SUBMIT_PATH, TRIPO_TASKS_PATH_PREFIX, TRIPO_UPLOAD_PATH, TripoClient,
    TripoError, TripoTimeouts, UPLOAD_FIELD_NAME,
};
pub use dto::{BillingFact, Envelope, SubmitParameters, SubmitRequest, UploadData, ViewInput};
pub use handlers::{TripoHandlers, is_current_lease_holder, stage_endpoint_summary};
pub use status::{NormalizedStatus, TripoState};
