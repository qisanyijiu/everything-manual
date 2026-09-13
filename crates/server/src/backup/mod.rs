//! 备份、恢复与发布版导出（T20 / REQ-005、REQ-037；AC-009、AC-010、AC-058）。
//!
//! 对外合同：
//! - PRD 修订 2 §3 REQ-005（灾备：备份与恢复到新目录）与 REQ-037（导出发布包）；
//! - `llmdoc/contracts.md` §3（`GET /releases/{releaseId}/export` 的语义）与 §7
//!   最后一条（导出 manifest 的元素、不含绝对路径/密钥/会话/临时云端 URL、
//!   当前不开放 ZIP 导入）；
//! - `llmdoc/architecture.md` §6/§7（data-dir 布局、停服 + 独占锁快照、连同被引用
//!   blob、恢复到新空目录后校验引用与哈希、升级前备份、程序回滚不等于数据库回滚）；
//! - `llmdoc/validation-release.md` §5「管理员运行合同 → 备份和升级」（CLI 步骤）。
//!
//! 模块划分：
//! - [`create`]：`backup --data-dir --out`（停服 + 独占锁 + `VACUUM INTO` 一致快照 +
//!   被引用 blob + manifest + sha256；输出路径必须不存在）；
//! - [`restore`]：`restore --from --data-dir`（目标必须不存在或为空；先全量校验
//!   hash/外键/引用，通过后才写；失败保留现场）；
//! - [`export`]：`GET /api/v1/releases/{releaseId}/export` 的自包含 ZIP 包；
//! - [`manifest`]：备份 manifest 的机器格式与相对路径安全检查；
//! - [`zip`]：STORE 模式的最小确定性 ZIP 写入器（不新增依赖）；
//! - [`files`]：流式哈希/校验复制/目录 fsync 等共用文件原语。
//!
//! **不提供**：ZIP 导入接口（contracts §7 明确"当前不顺手开放"）、网页端备份管理
//! 接口（contracts §3："备份恢复用 CLI，不开放高风险网页管理接口"）、定时备份。

pub mod create;
pub mod error;
pub mod export;
pub mod files;
pub mod manifest;
pub mod restore;
pub mod zip;

pub use create::{BackupOutcome, create_backup};
pub use error::BackupError;
pub use export::{ExportError, ExportPackage, build_release_export};
pub use manifest::{BACKUP_MANIFEST_FILE, BACKUP_SCHEMA_VERSION, BackupManifest};
pub use restore::{RestoreOutcome, restore_backup};
