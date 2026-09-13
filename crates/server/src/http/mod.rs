//! HTTP 层：路由、DTO、认证、错误映射与 OpenAPI 导出。
//!
//! 机器合同唯一来源是这里的 Rust DTO（ADR-009）：`cargo xtask contracts` 从
//! [`openapi::openapi_pretty_json`] 导出 `contracts/openapi.json`，再生成前端类型。
//!
//! T04 起模块划分：
//! - [`auth`]：登录/会话/注销、会话守卫（CSRF + Origin）、限速、Argon2 口令、token 派生；
//! - [`state`]：路由共享的应用状态（数据库 + 配置 + 限速器）；
//! - [`health`]：存活/就绪探针（就绪检查真实数据层，不检查云端）；
//! - [`settings`]：`/settings/status`（只返回配置状态，不含密钥）；
//! - [`items`]：物品增查改归档、字段级校验、归档过滤（T07 补齐 T04 的最小载体）；
//! - [`documents`]：说明书原件绑定与读取（T07，REQ-012）；
//! - [`photos`]：多视图照片与视图语义（T07，REQ-013）；
//! - [`preparations`]：PDF 准备的创建/读取/逐页上传/封存（T09，REQ-014、REQ-015）；
//! - [`estimates`]：生成前报价与云端发送确认（T11，REQ-020、REQ-021）；
//! - [`jobs`]：冻结输入 + 预留费用 + 幂等建单（T11，REQ-022）与任务读取/取消/重试/
//!   对账（T15，REQ-025/026/031）；
//! - [`drafts`]：版本化草稿读取、受限字段复核写入与发布（T15/T19，REQ-030/033/034/035）；
//! - [`releases`]：发布版本列表与完整 manifest 读取（T19，REQ-035/REQ-036）；
//! - [`assets`]：T06 的上传与内容服务（流式 multipart、Range/HEAD/ETag）；
//! - [`pagination`]：`{data, nextCursor}` 游标分页与严格查询参数解析（T07）；
//! - [`precondition`]：`If-Match` / `ETag` 工具；
//! - [`body`]：合同化的 JSON 请求体提取器（统一 413/415/422 错误结构）。

pub mod assets;
pub mod auth;
pub mod body;
pub mod documents;
pub mod drafts;
pub mod dto;
pub mod error;
pub mod estimates;
pub mod health;
pub mod items;
pub mod jobs;
pub mod logging;
pub mod openapi;
pub mod pagination;
pub mod photos;
pub mod precondition;
pub mod preparations;
pub mod releases;
pub mod router;
pub mod settings;
pub mod state;

#[cfg(feature = "embedded-ui")]
pub mod embedded;
