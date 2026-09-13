//! 资产服务（T06）：流式上传、内容校验、原子落盘、内容去重与授权内容服务。
//!
//! 合同依据：REQ-011（主）/AC-018/AC-019；contracts.md §2（blobs/assets 表与"必须保证"）、
//! §3（`POST /items/{id}/assets`、`GET/HEAD /assets/{id}/content`）、§7（输入限制、Range 合同）；
//! architecture.md §6（`blobs/<sha256 前缀>/<sha256>`、tmp 流写 → 校验/fsync/原子 rename →
//! 短事务提交元数据、崩溃残留按引用扫描隔离）与 §7（不把 data-dir 整目录暴露、Range 语义）。
//!
//! 文件状态机（一次上传）：
//!
//! ```text
//! tmp/<uuid>.part            流式写入 + 计数 + sha256（同时校验本用途体积上限）
//!   │  校验（magic/尺寸/像素/PDF 页数，基于 tmp 文件读，不把整个文件读进内存）
//!   │  剩余空间检查（不足 → 明确错误，不半提交）
//!   └─ fsync → 原子 rename ──► blobs/<sha256 前 2 位>/<sha256>
//!                                 │  短事务：blobs 幂等插入 + assets 归属行
//!                                 └─ 提交成功 = 资产可见；文件先于元数据存在（崩溃时是孤儿文件）
//! ```
//!
//! 三条不可退让的规则（QA 按此复核）：
//! 1. **请求路径从不删除 blob 文件**：DB 事务失败/回滚时只丢弃自己的 tmp 文件，
//!    已 rename 的 blob 若与既有内容同 sha256（共享 blob）绝不动它——孤儿文件由
//!    [`maintenance::scan_and_quarantine`] 按引用扫描隔离，不在这里顺手删；
//! 2. **原文件名只是元数据**：只取 basename、截断长度，从不参与路径拼接；
//! 3. **所有读取先校验归属**：`GET/HEAD /assets/{id}/content` 未授权（无会话）401、
//!    资产不存在/已隔离/文件缺失 404，跨物品归属由 `repo::assets::find_for_item` 判定，
//!    响应与日志都不含磁盘路径。

pub mod blob_store;
pub mod error;
pub mod glb;
pub mod maintenance;
pub mod pdf;
pub mod range;
pub mod upload;
pub mod validate;

pub use blob_store::{
    BLOBS_DIR_NAME, QUARANTINE_DIR_NAME, SpaceProbe, Staged, StagedWriter, TMP_DIR_NAME, blob_path,
};
pub use error::AssetError;
pub use upload::{AssetStore, UploadOutcome, UploadRequest};
