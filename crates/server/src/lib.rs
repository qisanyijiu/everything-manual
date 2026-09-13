//! everything-manual 服务库。
//!
//! 二进制（`src/main.rs`）只负责命令行派发；配置/CLI/日志（`config`）、
//! 持久化（`storage`）、资产服务（`assets`，T06）、路由/DTO/OpenAPI（`http`）、
//! 持久任务执行器（`jobs`，T10）、生成请求服务（`generation`，T11：报价/确认/
//! 冻结/预留/幂等建单）、外部 Provider 适配器（`providers`，T12：Tripo v3）、
//! 草稿与发布（`drafts`/`releases`，T15/T19：受限复核写入、发布不变量与不可变版本）、
//! 备份/恢复/导出（`backup`，T20）、供应商临时 URL 脱敏（`redaction`，T20/BUG-008）
//! 都在 library 中，便于集成测试与 `xtask` 复用（architecture.md §4）。

pub mod assets;
pub mod backup;
pub mod config;
pub mod drafts;
pub mod generation;
pub mod http;
pub mod jobs;
pub mod providers;
pub mod redaction;
pub mod releases;
pub mod storage;
