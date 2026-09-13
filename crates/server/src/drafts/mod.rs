//! 草稿组装与草稿服务（T15 组装 + T19 复核写入；contracts.md §2/§3、ADR-005）。
//!
//! 边界：
//! - **T15**：`assemble_draft` 阶段处理器（两条分支独立完成、部分成功可展示、
//!   幂等不重复创建）、`GET /items/{id}/drafts/{draftId}` 与 PATCH 的 ETag/If-Match 骨架；
//! - **T19**（本模块）：受限字段 PATCH（热点/视角/实体复核/modelReview，见
//!   [`aggregate`]）、组装时的旧绑定 stale 继承（AC-053）；
//! - **发布**（`POST .../publish`）在 [`crate::releases`]：发布不变量与 manifest
//!   属发布服务，草稿模块自身**从不写 `manual_releases`**（ADR-005：无自动发布路径）。
//!
//! 组装语义（REQ-030；机器可读形状见 [`knowledge::DraftKnowledge`]）：
//! - `draft.status = needs_review`：**生成完成 ≠ 已发布**（ADR-005）；
//! - 两条分支任一头被阻塞（`failed` / `needs_input` / `submission_unknown`）时，
//!   组装仍产出草稿并在 `missing[]` 里逐条标明缺项（部分成功可展示）；
//! - 同一快照至多一份草稿（`manual_drafts.snapshot_id` 唯一）：重启/重放/重试
//!   不重复创建，内容变化才递增 revision；
//! - 内容变化时清空 `review_json`（复核声明绑定具体内容/模型，必须重新声明）。

pub mod aggregate;
pub mod knowledge;
pub mod service;

pub use aggregate::{
    Anchor, CameraPose, DraftPatch, EntityReview, EntityReviewPatch, Hotspot, HotspotPatch,
    HotspotStatus, HotspotUpsert, ModelReview, ModelReviewPatch, ReviewOverlay, UserEdit,
};
pub use knowledge::{
    DRAFT_SCHEMA_VERSION, DraftCompleteness, DraftKnowledge, DraftMissingItem, DraftModelInfo,
};
pub use service::{
    AssembleOutcome, DraftServiceError, assemble_draft, parse_review_overlay, patch_draft,
    read_draft,
};
