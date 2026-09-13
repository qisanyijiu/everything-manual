//! T19 草稿聚合的**受限字段写入与校验**（REQ-033 热点/视角、REQ-034 知识确认与修订）。
//!
//! 边界（contracts.md §2/§3；PRD 修订 2 AC-052/AC-053/AC-054）：
//!
//! 1. **供应商事实快照不可改**：`knowledge.knowledge`（AI 提取并合并的 Part/Step/Spec/
//!    Evidence）在本模块里只读。人工修订写在独立的 `review_json` 覆盖层
//!    （`entities[].userEdited`，带 `editedAt/editedBy`），原文本与出处始终保留，
//!    UI 展示"原文本对照"（UI-051）。请求里也没有任何可以直改快照的字段
//!    （`deny_unknown_fields`：发送 `knowledgeJson`/`parts` 等 → 422）。
//! 2. **热点写入**（`knowledge.hotspots`）：
//!    - 人工直接拾取可 `unbound → confirmed`（一次请求即 confirmed + 非空 anchor）；
//!    - `unbound` 时 `anchor` 必须为 `null`（**禁止 [0,0,0] 占位**）；
//!    - `candidate`/`confirmed` 必须有非空 anchor，且 `modelRevisionId + modelSha256`
//!      与草稿当前模型**完全一致**——旧模型 sha 的 confirmed 提交一律 422
//!      （AC-053「API 拒绝 confirmed 热点与当前模型 sha 不符」）；
//!    - 全部数值必须有限（拒绝 NaN/Infinity）；
//!    - 已确认/候选热点与当前模型不一致时**降级为 `stale`**（不静默复用；
//!      重新绑定 = 用同一热点 id 提交匹配新模型的 anchor）。
//! 3. **引用校验**：`partId`/`stepId`/实体 id 必须存在于本次冻结知识；部件引用
//!    （Step.partIds）由服务端在提取阶段校验，此处校验热点与视角的宿主引用。
//! 4. **modelReview 是用户声明**：`loaded` / `userConfirmed` 由用户声明（UI-052 的
//!    两个独立动作），`checkedAt`（及各自的完成时间）由服务器赋值；`userConfirmed`
//!    不能在没有 `loaded` 时成立；换模型由组装阶段清空整条记录。
//! 5. **「事实确认」与「几何校准」分离**：实体确认/修订（本模块 `entities` 部分）与
//!    热点/视角（几何校准）是不同的操作与写入路径，错误码与文案不共用同一"确认"字样。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use manual_core::knowledge::ReviewStatus;
use manual_core::timestamps::Timestamp;
use manual_core::validation::FieldIssue;

use super::knowledge::DraftKnowledge;

// ---------------------------------------------------------------------------
// 数值限制（与 contracts §2 / PRD UI-050 的"FOV 合理范围"一致）
// ---------------------------------------------------------------------------

/// 相机垂直视场角合法区间（度，开区间端点）：超出即字段错误。
pub const FOV_MIN_DEG: f64 = 1.0;
pub const FOV_MAX_DEG: f64 = 179.0;

/// 人工修订的字段长度上限（与提取阶段的 KNOWLEDGE_MAX_* 同量级，防止绕过校验写入超长文本）。
pub const EDIT_MAX_NAME_CHARS: usize = 120;
pub const EDIT_MAX_DESCRIPTION_CHARS: usize = 1200;
pub const EDIT_MAX_TITLE_CHARS: usize = 200;
pub const EDIT_MAX_ACTION_CHARS: usize = 600;
pub const EDIT_MAX_LABEL_CHARS: usize = 120;
pub const EDIT_MAX_VALUE_CHARS: usize = 600;
pub const EDIT_MAX_ACTIONS_PER_STEP: usize = 24;

// ---------------------------------------------------------------------------
// 锚点、热点、相机位姿（contracts §2「Hotspot / CameraPose」）
// ---------------------------------------------------------------------------

/// 热点锚点：不可变模型版本 + 内容哈希 + asset-root 局部点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Anchor {
    pub model_revision_id: String,
    pub model_sha256: String,
    /// asset-root 局部坐标（`[x, y, z]`；全部数值有限）。
    pub position_local: [f64; 3],
}

impl Anchor {
    /// 数值有限性（禁止 NaN/Infinity）。
    pub fn has_finite_values(&self) -> bool {
        self.position_local.iter().all(|value| value.is_finite())
    }

    /// 是否与给定模型版本完全匹配（revision 与 sha 都必须一致）。
    pub fn matches_model(&self, revision_id: &str, sha256: &str) -> bool {
        self.model_revision_id == revision_id && self.model_sha256 == sha256
    }
}

/// 热点状态机：`unbound → candidate → confirmed`；人工直接拾取可 `unbound → confirmed`；
/// 模型 revision 变化后旧绑定进入 `stale`（不得作为有效热点显示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HotspotStatus {
    /// 未绑定：`anchor = null`（不得用 [0,0,0] 占位）。
    Unbound,
    /// 候选：有 anchor 且与当前模型匹配（MVP 不自动生成，仅 API 支持状态机）。
    Candidate,
    /// 已确认：有 anchor 且与当前模型匹配，可用于发布。
    Confirmed,
    /// 失效：anchor 属于旧模型版本（保留用于解释，不显示为有效热点、不可发布）。
    Stale,
}

impl HotspotStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unbound => "unbound",
            Self::Candidate => "candidate",
            Self::Confirmed => "confirmed",
            Self::Stale => "stale",
        }
    }

    /// 需要非空且匹配当前模型的 anchor。
    pub const fn requires_anchor(self) -> bool {
        matches!(self, Self::Candidate | Self::Confirmed)
    }

    pub const fn is_stale(self) -> bool {
        matches!(self, Self::Stale)
    }
}

/// 一条热点（`knowledge.hotspots[]`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hotspot {
    pub id: String,
    pub part_id: String,
    pub status: HotspotStatus,
    /// `unbound` 时为 `null`；其余状态非空（数值有限）。
    #[schema(nullable = true)]
    pub anchor: Option<Anchor>,
}

/// 步骤视角（`CameraPose`；全部相对同一 asset-root，不是机械动作）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraPose {
    pub position_local: [f64; 3],
    pub target_local: [f64; 3],
    pub up_local: [f64; 3],
    pub fov: f64,
}

impl CameraPose {
    /// 结构合法性与数值有限性（`up` 不得为零向量）。
    pub fn is_valid(&self) -> bool {
        self.position_local.iter().all(|value| value.is_finite())
            && self.target_local.iter().all(|value| value.is_finite())
            && self.up_local.iter().all(|value| value.is_finite())
            && self.fov.is_finite()
            && (FOV_MIN_DEG..=FOV_MAX_DEG).contains(&self.fov)
            && self.up_local.iter().any(|value| *value != 0.0)
    }
}

// ---------------------------------------------------------------------------
// 复核覆盖层（`review_json`；contracts §2「Review」）
// ---------------------------------------------------------------------------

/// 人工修订（本地覆盖；原快照不动，出处保留在 `knowledge.knowledge` 里）。
///
/// 字段是显式的（不是任意 JSON）：发送实体类型不支持的字段 → 422，
/// 避免"顺手改了供应商快照的形状"。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserEdit {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub ordered_actions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub value: Option<String>,
}

impl UserEdit {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// 实体级复核覆盖（`review_json.entities[<entityId>]`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityReview {
    #[serde(default)]
    pub review_status: Option<ReviewStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_edited: Option<UserEdit>,
    #[serde(default)]
    pub text_only: bool,
    /// 服务器赋值：最后一次触及本条目的人工复核时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_by: Option<String>,
}

impl EntityReview {
    /// 该实体是否已复核：`confirmed` 或有人工修订记录（发布不变量）。
    pub fn is_reviewed(&self) -> bool {
        self.review_status == Some(ReviewStatus::Confirmed) || self.user_edited.is_some()
    }
}

/// 模型复核记录（`review_json.modelReview`）。
///
/// 语义（contracts §2）：**已认证用户的复核声明**，不是服务端可证明的 GPU 测试。
/// `loaded` = 用户声明"已在浏览器成功打开此模型"；`userConfirmed` = 用户声明
/// "我已核对模型与资料一致"。`checkedAt`/`loadedAt`/`userConfirmedAt` 由服务器赋值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelReview {
    pub model_revision_id: String,
    pub model_sha256: String,
    pub loaded: bool,
    pub user_confirmed: bool,
    /// 服务器赋值：最近一次状态变化时间（毫秒 since epoch）。
    pub checked_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_confirmed_at: Option<i64>,
}

impl ModelReview {
    /// 发布要求的"双真 + 版本匹配"。
    pub fn is_complete_for(&self, revision_id: &str, sha256: &str) -> bool {
        self.loaded && self.user_confirmed && self.matches(revision_id, sha256)
    }

    pub fn matches(&self, revision_id: &str, sha256: &str) -> bool {
        self.model_revision_id == revision_id && self.model_sha256 == sha256
    }
}

/// 复核覆盖层（`review_json` 的顶层形状）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOverlay {
    /// 实体 id → 复核覆盖（Part/Step/Spec 共用一个命名空间，id 由内容派生已避免碰撞）。
    #[serde(default)]
    pub entities: BTreeMap<String, EntityReview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_review: Option<ModelReview>,
}

impl ReviewOverlay {
    pub fn entity(&self, id: &str) -> Option<&EntityReview> {
        self.entities.get(id)
    }

    /// 该实体是否已复核（无覆盖 = 未复核：AI 产出默认 `needs_review`）。
    pub fn is_reviewed(&self, id: &str) -> bool {
        self.entity(id).is_some_and(EntityReview::is_reviewed)
    }
}

// ---------------------------------------------------------------------------
// PATCH 输入（受限字段；HTTP DTO 直接复用这些类型，避免手抄两份）
// ---------------------------------------------------------------------------

/// 热点写入：`upsert`（无 id = 新建，服务器分配 id）+ `remove`（解绑）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HotspotPatch {
    #[serde(default)]
    pub upsert: Vec<HotspotUpsert>,
    #[serde(default)]
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HotspotUpsert {
    /// 既有热点 id（重新绑定/解绑用）；省略 = 新建。
    #[serde(default)]
    #[schema(nullable = true)]
    pub id: Option<String>,
    pub part_id: String,
    pub status: HotspotStatus,
    #[serde(default)]
    #[schema(nullable = true)]
    pub anchor: Option<Anchor>,
}

/// 实体级复核写入（确认/取消确认、人工修订、仅文本条目）。
///
/// `review_status` 用 core 的 [`ReviewStatus`]（服务端语义单一来源）；HTTP DTO
/// （`http::dto::drafts::EntityReviewPatchDto`）持有同值集的线上枚举并在 handler 里
/// 转换——core 不依赖 utoipa，这是两处唯一需要镜像的字段。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntityReviewPatch {
    #[serde(default)]
    pub review_status: Option<ReviewStatus>,
    #[serde(default)]
    pub user_edited: Option<UserEdit>,
    #[serde(default)]
    pub text_only: Option<bool>,
}

/// modelReview 的两个用户声明（服务器补 `checkedAt` 与模型身份）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelReviewPatch {
    pub loaded: bool,
    pub user_confirmed: bool,
}

/// 草稿 PATCH（T19 受限字段集；HTTP 层 [`crate::http::dto::drafts::DraftPatchRequest`] 持有
/// 线上形状，这里只保留服务端语义类型）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftPatch {
    #[serde(default)]
    pub status: Option<manual_core::domain::DraftStatus>,
    #[serde(default)]
    pub hotspots: Option<HotspotPatch>,
    /// 步骤视角写入：`{ "<stepId>": CameraPose }`（保存/覆盖）。
    #[serde(default)]
    pub step_poses: Option<BTreeMap<String, CameraPose>>,
    /// 要清除视角的步骤 id 列表（与写入分开，避免 `null` 值的 oneOf 结构；
    /// 保存与清除是两个动作，UI-050 的按钮也分开）。
    #[serde(default)]
    pub clear_step_poses: Option<Vec<String>>,
    #[serde(default)]
    pub entities: Option<BTreeMap<String, EntityReviewPatch>>,
    #[serde(default)]
    pub model_review: Option<ModelReviewPatch>,
}

impl DraftPatch {
    pub fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.hotspots.is_none()
            && self.step_poses.is_none()
            && self.clear_step_poses.is_none()
            && self.entities.is_none()
            && self.model_review.is_none()
    }
}

// ---------------------------------------------------------------------------
// 应用与校验
// ---------------------------------------------------------------------------

/// 应用受限字段变更（校验失败返回字段级问题；调用方映射 422 `details.fields`）。
///
/// 原子应用：**校验全部通过后才写入任何字段**，任一字段非法 → 不产生部分修改，
/// 也不递增 revision。
pub fn apply_draft_patch(
    knowledge: &mut DraftKnowledge,
    overlay: &mut ReviewOverlay,
    patch: &DraftPatch,
    actor: &str,
    now: Timestamp,
) -> Result<(), Vec<FieldIssue>> {
    let mut issues: Vec<FieldIssue> = Vec::new();

    if let Some(hotspots) = &patch.hotspots {
        validate_hotspot_patch(knowledge, hotspots, &mut issues);
    }
    let step_exists = |step_id: &str| {
        knowledge
            .knowledge
            .as_ref()
            .is_some_and(|merged| merged.steps.iter().any(|step| step.id == step_id))
    };
    if let Some(poses) = &patch.step_poses {
        for (step_id, pose) in poses {
            if !step_exists(step_id) {
                issues.push(FieldIssue::new(
                    "stepPoses",
                    format!("步骤不存在：{step_id}（视角只能挂在本次冻结知识的步骤上）"),
                ));
                continue;
            }
            if !pose.is_valid() {
                issues.push(FieldIssue::new(
                    "stepPoses",
                    format!(
                        "步骤 {step_id} 的视角不是合法 CameraPose：数值必须有限、up 不得为零向量、\
                         fov 需在 {FOV_MIN_DEG}–{FOV_MAX_DEG} 度之间"
                    ),
                ));
            }
        }
    }
    if let Some(clear) = &patch.clear_step_poses {
        for step_id in clear {
            if !step_exists(step_id) {
                issues.push(FieldIssue::new(
                    "clearStepPoses",
                    format!("步骤不存在：{step_id}"),
                ));
            }
        }
    }
    if let Some(entities) = &patch.entities {
        for (entity_id, entity_patch) in entities {
            validate_entity_patch(knowledge, overlay, entity_id, entity_patch, &mut issues);
        }
    }
    if let Some(model_review) = &patch.model_review {
        validate_model_review(knowledge, overlay, model_review, &mut issues);
    }

    if !issues.is_empty() {
        return Err(issues);
    }

    // 校验全部通过：依次写入（热点/视角进知识外壳；复核进覆盖层）。
    if let Some(hotspots) = &patch.hotspots {
        commit_hotspot_patch(knowledge, hotspots);
    }
    if let Some(poses) = &patch.step_poses {
        for (step_id, pose) in poses {
            knowledge.step_poses.insert(step_id.clone(), pose.clone());
        }
    }
    if let Some(clear) = &patch.clear_step_poses {
        for step_id in clear {
            knowledge.step_poses.remove(step_id);
        }
    }
    if let Some(entities) = &patch.entities {
        for (entity_id, entity_patch) in entities {
            commit_entity_patch(overlay, entity_id, entity_patch, actor, now);
        }
    }
    if let Some(model_review_patch) = &patch.model_review
        && let Some(model) = knowledge.model.as_ref()
    {
        // modelReview 的模型身份由**服务器**赋值（不接受客户端自称的 revision/sha）。
        let stale_identity = overlay
            .model_review
            .as_ref()
            .is_some_and(|review| !review.matches(&model.revision_id, &model.sha256));
        if overlay.model_review.is_none() || stale_identity {
            overlay.model_review = Some(ModelReview {
                model_revision_id: model.revision_id.clone(),
                model_sha256: model.sha256.clone(),
                loaded: false,
                user_confirmed: false,
                checked_at: now.as_millis(),
                loaded_at: None,
                user_confirmed_at: None,
            });
        }
        if let Some(review) = overlay.model_review.as_mut() {
            let mut touched = false;
            if model_review_patch.loaded && !review.loaded {
                review.loaded = true;
                review.loaded_at = Some(now.as_millis());
                touched = true;
            }
            if model_review_patch.user_confirmed && !review.user_confirmed {
                review.user_confirmed = true;
                review.user_confirmed_at = Some(now.as_millis());
                touched = true;
            }
            if touched {
                review.checked_at = now.as_millis();
            }
        }
    }
    // 防御性规整：任何 confirmed/candidate 热点若不匹配当前模型（或缺少 anchor）
    // 一律降级 stale——绝不把"确认过的旧绑定"留在有效状态里（AC-053）。
    normalize_stale_hotspots(knowledge);
    Ok(())
}

fn validate_hotspot_patch(
    knowledge: &DraftKnowledge,
    patch: &HotspotPatch,
    issues: &mut Vec<FieldIssue>,
) {
    let parts: Vec<&str> = knowledge
        .knowledge
        .as_ref()
        .map(|merged| merged.parts.iter().map(|part| part.id.as_str()).collect())
        .unwrap_or_default();
    let model = knowledge.model.as_ref();

    for item in &patch.upsert {
        if !parts.iter().any(|id| *id == item.part_id) {
            issues.push(FieldIssue::new(
                "hotspots",
                format!(
                    "部件不存在：{}（热点只能绑定本次冻结知识里的部件）",
                    item.part_id
                ),
            ));
            continue;
        }
        if let Some(id) = &item.id
            && !knowledge.hotspots.iter().any(|hotspot| hotspot.id == *id)
        {
            issues.push(FieldIssue::new(
                "hotspots",
                format!("热点不存在：{id}（新建热点请省略 id）"),
            ));
            continue;
        }
        match (item.status, &item.anchor) {
            (HotspotStatus::Unbound, Some(_)) => {
                issues.push(FieldIssue::new(
                    "hotspots",
                    format!(
                        "热点未绑定时 anchor 必须为 null（不得用 [0,0,0] 占位）：{}",
                        item.part_id
                    ),
                ));
            }
            (HotspotStatus::Unbound, None) => {}
            (_, None) => {
                issues.push(FieldIssue::new(
                    "hotspots",
                    format!(
                        "状态 {} 的热点必须携带非空 anchor：{}",
                        item.status.as_str(),
                        item.part_id
                    ),
                ));
            }
            (_, Some(anchor)) => {
                if !anchor.has_finite_values() {
                    issues.push(FieldIssue::new(
                        "hotspots",
                        "热点坐标必须是有限数值（禁止 NaN/Infinity）".to_owned(),
                    ));
                    continue;
                }
                let matches = model
                    .is_some_and(|model| anchor.matches_model(&model.revision_id, &model.sha256));
                if !matches {
                    issues.push(FieldIssue::new(
                        "hotspots",
                        format!(
                            "该绑定属于旧模型版本（anchor 的 modelRevisionId/modelSha256 与当前模型不符）：\
                             请在新模型上重新绑定（{}）",
                            item.part_id
                        ),
                    ));
                }
            }
        }
    }

    for id in &patch.remove {
        if !knowledge.hotspots.iter().any(|hotspot| hotspot.id == *id) {
            issues.push(FieldIssue::new(
                "hotspots",
                format!("要解绑的热点不存在：{id}"),
            ));
        }
    }
}

fn commit_hotspot_patch(knowledge: &mut DraftKnowledge, patch: &HotspotPatch) {
    for item in &patch.upsert {
        match item.id.as_deref().and_then(|id| {
            knowledge
                .hotspots
                .iter_mut()
                .find(|hotspot| hotspot.id == id)
        }) {
            Some(existing) => {
                existing.part_id = item.part_id.clone();
                existing.status = item.status;
                existing.anchor = item.anchor.clone();
            }
            None => knowledge.hotspots.push(Hotspot {
                id: manual_core::ids::new_id(),
                part_id: item.part_id.clone(),
                status: item.status,
                anchor: item.anchor.clone(),
            }),
        }
    }
    if !patch.remove.is_empty() {
        knowledge
            .hotspots
            .retain(|hotspot| !patch.remove.contains(&hotspot.id));
    }
}

/// 组装新草稿时的**旧绑定继承**（AC-053：「重新生成模型后打开新草稿 → 旧绑定进入 stale」）。
///
/// 规则（不静默复用、不静默丢弃）：
/// - 继承上一份草稿（同一快照的重组装优先；否则同物品最近一份草稿）的热点，
///   但只继承**部件仍存在**的那些（部件消失 → 丢弃并在 `missing[]` 里报告）；
/// - 继承后由 [`normalize_stale_hotspots`] 统一规整：anchor 与当前模型
///   `revisionId + sha256` 不一致的一律降级 `stale`（旧 anchor 保留用于解释，
///   界面显示"需在新模型上重新绑定"，不得当有效热点、不得发布）；
/// - 步骤视角只在**模型身份完全一致**（revision + sha256）时继承：视角是几何量，
///   换模型后必须重新保存（丢弃时报告缺项，不静默）。
pub fn carry_forward_previous(
    envelope: &mut DraftKnowledge,
    previous: &DraftKnowledge,
) -> CarryForwardReport {
    let mut report = CarryForwardReport::default();
    // 拥有所有权的 id 集合：下面要可变借用 `envelope`（push 热点/视角）。
    let part_ids: Vec<String> = envelope
        .knowledge
        .as_ref()
        .map(|merged| merged.parts.iter().map(|part| part.id.clone()).collect())
        .unwrap_or_default();
    let step_ids: Vec<String> = envelope
        .knowledge
        .as_ref()
        .map(|merged| merged.steps.iter().map(|step| step.id.clone()).collect())
        .unwrap_or_default();

    for hotspot in &previous.hotspots {
        if part_ids.contains(&hotspot.part_id) {
            envelope.hotspots.push(hotspot.clone());
            report.hotspots_carried += 1;
            if hotspot.status.is_stale() {
                report.hotspots_already_stale += 1;
            }
        } else {
            report.hotspots_dropped += 1;
        }
    }
    normalize_stale_hotspots(envelope);
    report.hotspots_stale = envelope
        .hotspots
        .iter()
        .filter(|hotspot| hotspot.status.is_stale())
        .count();

    let same_model = match (&envelope.model, &previous.model) {
        (Some(current), Some(previous_model)) => {
            current.revision_id == previous_model.revision_id
                && current.sha256 == previous_model.sha256
        }
        _ => false,
    };
    for (step_id, pose) in &previous.step_poses {
        if step_ids.contains(step_id) {
            if same_model {
                envelope.step_poses.insert(step_id.clone(), pose.clone());
                report.poses_carried += 1;
            } else {
                report.poses_dropped += 1;
            }
        } else {
            report.poses_dropped += 1;
        }
    }
    report
}

/// 继承结果（组装阶段按此补 `missing[]`；测试断言用）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CarryForwardReport {
    pub hotspots_carried: usize,
    pub hotspots_already_stale: usize,
    pub hotspots_stale: usize,
    pub hotspots_dropped: usize,
    pub poses_carried: usize,
    pub poses_dropped: usize,
}

/// 把不匹配当前模型的 confirmed/candidate 热点降级为 `stale`（保留 anchor 作解释）。
pub fn normalize_stale_hotspots(knowledge: &mut DraftKnowledge) {
    let model_identity = knowledge
        .model
        .as_ref()
        .map(|model| (model.revision_id.clone(), model.sha256.clone()));
    for hotspot in &mut knowledge.hotspots {
        if !hotspot.status.requires_anchor() {
            continue;
        }
        let matches = match (&hotspot.anchor, &model_identity) {
            (Some(anchor), Some((revision_id, sha256))) => {
                anchor.has_finite_values() && anchor.matches_model(revision_id, sha256)
            }
            _ => false,
        };
        if !matches {
            hotspot.status = HotspotStatus::Stale;
        }
    }
}

/// 实体复核（确认/修订/仅文本条目）的字段级校验（不修改任何内容）。
fn validate_entity_patch(
    knowledge: &DraftKnowledge,
    overlay: &ReviewOverlay,
    entity_id: &str,
    patch: &EntityReviewPatch,
    issues: &mut Vec<FieldIssue>,
) {
    let Some(merged) = knowledge.knowledge.as_ref() else {
        issues.push(FieldIssue::new(
            "entities",
            format!("草稿没有知识内容：无法复核实体 {entity_id}"),
        ));
        return;
    };

    enum Kind {
        Part,
        Step,
        Spec,
    }
    let kind = if merged.parts.iter().any(|part| part.id == entity_id) {
        Kind::Part
    } else if merged.steps.iter().any(|step| step.id == entity_id) {
        Kind::Step
    } else if merged.specs.iter().any(|spec| spec.id == entity_id) {
        Kind::Spec
    } else {
        issues.push(FieldIssue::new(
            "entities",
            format!("实体不存在：{entity_id}（只能复核本次冻结知识里的部件/步骤/规格）"),
        ));
        return;
    };

    if let Some(edit) = &patch.user_edited {
        if edit.is_empty() {
            issues.push(FieldIssue::new(
                "entities",
                format!("实体 {entity_id} 的人工修订为空：请提供至少一个字段"),
            ));
        }
        let (allowed, allow_actions): (Vec<&str>, bool) = match kind {
            Kind::Part => (vec!["name", "description"], false),
            Kind::Step => (vec!["title", "orderedActions"], true),
            Kind::Spec => (vec!["label", "value"], false),
        };
        let provided: Vec<(&str, bool)> = vec![
            ("name", edit.name.is_some()),
            ("description", edit.description.is_some()),
            ("title", edit.title.is_some()),
            ("orderedActions", edit.ordered_actions.is_some()),
            ("label", edit.label.is_some()),
            ("value", edit.value.is_some()),
        ];
        for (field, present) in provided {
            if present && !allowed.contains(&field) {
                issues.push(FieldIssue::new(
                    "entities",
                    format!("实体 {entity_id} 不支持人工修订字段 {field}（不改变供应商事实快照）"),
                ));
            }
        }
        if let Some(text) = &edit.name {
            check_edit_text(entity_id, "name", text, EDIT_MAX_NAME_CHARS, issues);
        }
        if let Some(text) = &edit.description {
            check_edit_text(
                entity_id,
                "description",
                text,
                EDIT_MAX_DESCRIPTION_CHARS,
                issues,
            );
        }
        if let Some(text) = &edit.title {
            check_edit_text(entity_id, "title", text, EDIT_MAX_TITLE_CHARS, issues);
        }
        if let Some(text) = &edit.label {
            check_edit_text(entity_id, "label", text, EDIT_MAX_LABEL_CHARS, issues);
        }
        if let Some(text) = &edit.value {
            check_edit_text(entity_id, "value", text, EDIT_MAX_VALUE_CHARS, issues);
        }
        if let Some(actions) = &edit.ordered_actions {
            if !allow_actions {
                // 已由 allowed 检查覆盖（这里防御未来新增实体类型）。
                issues.push(FieldIssue::new(
                    "entities",
                    format!("实体 {entity_id} 不支持 orderedActions"),
                ));
            }
            if actions.is_empty() {
                issues.push(FieldIssue::new(
                    "entities",
                    format!("实体 {entity_id} 的 orderedActions 不能为空列表"),
                ));
            }
            if actions.len() > EDIT_MAX_ACTIONS_PER_STEP {
                issues.push(FieldIssue::new(
                    "entities",
                    format!(
                        "实体 {entity_id} 的 orderedActions 条数 {} 超过上限 {EDIT_MAX_ACTIONS_PER_STEP}",
                        actions.len()
                    ),
                ));
            }
            for action in actions {
                check_edit_text(
                    entity_id,
                    "orderedActions",
                    action,
                    EDIT_MAX_ACTION_CHARS,
                    issues,
                );
            }
        }
    }

    // 「仅文本条目」只能用于部件；且必须同时确认或有人工修订（不能用来绕过复核）。
    if let Some(text_only) = patch.text_only
        && text_only
    {
        {
            if !matches!(kind, Kind::Part) {
                issues.push(FieldIssue::new(
                    "entities",
                    format!("「仅文本条目」只适用于部件：{entity_id}"),
                ));
            }
            let existing = overlay.entity(entity_id);
            let will_be_confirmed = patch.review_status == Some(ReviewStatus::Confirmed)
                || (patch.review_status.is_none()
                    && existing
                        .is_some_and(|entry| entry.review_status == Some(ReviewStatus::Confirmed)));
            let will_have_edit = patch
                .user_edited
                .as_ref()
                .is_some_and(|edit| !edit.is_empty())
                || existing.is_some_and(|entry| entry.user_edited.is_some());
            if !(will_be_confirmed || will_have_edit) {
                issues.push(FieldIssue::new(
                    "entities",
                    format!(
                        "把部件 {entity_id} 标记为「仅文本条目」需要同时确认该部件或提供人工修订\
                         （不能用来跳过复核）"
                    ),
                ));
            }
        }
    }
}

fn check_edit_text(
    entity_id: &str,
    field: &str,
    text: &str,
    limit: usize,
    issues: &mut Vec<FieldIssue>,
) {
    if text.trim().is_empty() {
        issues.push(FieldIssue::new(
            "entities",
            format!("实体 {entity_id} 的 {field} 不能为空白"),
        ));
    } else if text.chars().count() > limit {
        issues.push(FieldIssue::new(
            "entities",
            format!("实体 {entity_id} 的 {field} 超过 {limit} 字符上限"),
        ));
    }
}

fn validate_model_review(
    knowledge: &DraftKnowledge,
    overlay: &ReviewOverlay,
    patch: &ModelReviewPatch,
    issues: &mut Vec<FieldIssue>,
) {
    if knowledge.model.is_none() {
        issues.push(FieldIssue::new(
            "modelReview",
            "草稿没有可用的模型版本：无法记录模型复核声明".to_owned(),
        ));
        return;
    }
    let existing_loaded = overlay
        .model_review
        .as_ref()
        .is_some_and(|review| review.loaded);
    if patch.user_confirmed && !(patch.loaded || existing_loaded) {
        issues.push(FieldIssue::new(
            "modelReview",
            "userConfirmed 需要先声明 loaded（请先确认已在浏览器成功打开此模型）".to_owned(),
        ));
    }
}

/// 把（已校验的）实体复核写入覆盖层；时间戳只在**内容实际变化**时盖章
/// （重复提交同一内容保持幂等：不递增 revision）。
fn commit_entity_patch(
    overlay: &mut ReviewOverlay,
    entity_id: &str,
    patch: &EntityReviewPatch,
    actor: &str,
    now: Timestamp,
) -> bool {
    let entry = overlay
        .entities
        .entry(entity_id.to_owned())
        .or_insert_with(|| EntityReview {
            review_status: None,
            user_edited: None,
            text_only: false,
            edited_at: None,
            edited_by: None,
        });
    let before = entry.clone();
    let mut touched = false;
    if let Some(status) = patch.review_status
        && entry.review_status != Some(status)
    {
        entry.review_status = Some(status);
        touched = true;
    }
    if let Some(edit) = &patch.user_edited
        && entry.user_edited.as_ref() != Some(edit)
    {
        entry.user_edited = Some(edit.clone());
        touched = true;
    }
    if let Some(text_only) = patch.text_only
        && entry.text_only != text_only
    {
        entry.text_only = text_only;
        touched = true;
    }
    if touched {
        entry.edited_at = Some(now.as_millis());
        entry.edited_by = Some(actor.to_owned());
    }
    *entry != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drafts::knowledge::{DraftKnowledge, DraftModelInfo};
    use manual_core::domain::ModelValidationState;
    use manual_core::knowledge::MergedKnowledge;

    fn model(revision: &str, sha: &str) -> DraftModelInfo {
        DraftModelInfo {
            revision_id: revision.to_owned(),
            sha256: sha.to_owned(),
            validation_state: ModelValidationState::Validated,
            asset_id: "asset-1".to_owned(),
            bounds: None,
        }
    }

    fn knowledge_with_parts() -> DraftKnowledge {
        let merged: MergedKnowledge = serde_json::from_value(serde_json::json!({
            "schemaVersion": "manual_extract_v1",
            "promptVersion": "manual_extract_v1",
            "pageFrom": 1,
            "pageTo": 1,
            "coverage": { "complete": true, "pageCount": 1, "pages": [1], "batches": [] },
            "parts": [{
                "id": "part-1", "name": "后盖", "description": "可拆盖板",
                "evidence": [{ "documentId": "doc", "preparationId": "prep", "pageNumber": 1,
                               "quote": null, "bbox": null, "derived": false }],
                "reviewStatus": "needs_review", "sourceBatches": [0]
            }],
            "steps": [], "specs": [], "uncertainties": [], "conflicts": []
        }))
        .expect("测试知识");
        DraftKnowledge::build(
            "job-1",
            Some(model("rev-1", &"a".repeat(64))),
            Some(merged),
            vec![],
        )
    }

    #[test]
    fn unbound_hotspot_requires_null_anchor_and_teleport_placeholder_is_rejected() {
        let mut knowledge = knowledge_with_parts();
        let mut overlay = ReviewOverlay::default();
        // [0,0,0] 占位（带 anchor 的 unbound）必须被拒。
        let patch = DraftPatch {
            hotspots: Some(HotspotPatch {
                upsert: vec![HotspotUpsert {
                    id: None,
                    part_id: "part-1".to_owned(),
                    status: HotspotStatus::Unbound,
                    anchor: Some(Anchor {
                        model_revision_id: "rev-1".to_owned(),
                        model_sha256: "a".repeat(64),
                        position_local: [0.0, 0.0, 0.0],
                    }),
                }],
                remove: Vec::new(),
            }),
            ..DraftPatch::default()
        };
        let issues = apply_draft_patch(
            &mut knowledge,
            &mut overlay,
            &patch,
            "admin",
            Timestamp::now(),
        )
        .unwrap_err();
        assert!(issues.iter().any(|issue| issue.field == "hotspots"));
        assert!(knowledge.hotspots.is_empty(), "校验失败不得写入");
    }

    #[test]
    fn confirmed_hotspot_with_stale_sha_is_rejected_and_finite_values_enforced() {
        let mut knowledge = knowledge_with_parts();
        let mut overlay = ReviewOverlay::default();
        // 旧 sha 的 confirmed → 拒绝。
        let stale = DraftPatch {
            hotspots: Some(HotspotPatch {
                upsert: vec![HotspotUpsert {
                    id: None,
                    part_id: "part-1".to_owned(),
                    status: HotspotStatus::Confirmed,
                    anchor: Some(Anchor {
                        model_revision_id: "rev-0".to_owned(),
                        model_sha256: "b".repeat(64),
                        position_local: [1.0, 2.0, 3.0],
                    }),
                }],
                remove: Vec::new(),
            }),
            ..DraftPatch::default()
        };
        let issues = apply_draft_patch(
            &mut knowledge,
            &mut overlay,
            &stale,
            "admin",
            Timestamp::now(),
        )
        .unwrap_err();
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("旧模型版本")),
            "{issues:?}"
        );

        // 非有限数值的两条真实路径：
        // a) 客户端发送超范围字面量（如 1e400）：JSON 解析层直接拒绝（HTTP 422）；
        // b) 内部产生的 NaN（JSON 无法表达）：由 `has_finite_values` 拒绝。
        let raw = format!(
            r#"{{"modelRevisionId":"rev-1","modelSha256":"{}","positionLocal":[1e400,0.0,0.0]}}"#,
            "a".repeat(64)
        );
        let parsed: Result<Anchor, _> = serde_json::from_str(&raw);
        assert!(
            parsed.is_err(),
            "超范围字面量必须在解析层被拒绝（number out of range）"
        );
        let nan = Anchor {
            model_revision_id: "rev-1".to_owned(),
            model_sha256: "a".repeat(64),
            position_local: [f64::NAN, 0.0, 0.0],
        };
        assert!(!nan.has_finite_values(), "NaN 必须被拒绝");
        let patch = DraftPatch {
            hotspots: Some(HotspotPatch {
                upsert: vec![HotspotUpsert {
                    id: None,
                    part_id: "part-1".to_owned(),
                    status: HotspotStatus::Confirmed,
                    anchor: Some(nan),
                }],
                remove: Vec::new(),
            }),
            ..DraftPatch::default()
        };
        let issues = apply_draft_patch(
            &mut knowledge,
            &mut overlay,
            &patch,
            "admin",
            Timestamp::now(),
        )
        .unwrap_err();
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("有限数值"))
        );
    }

    #[test]
    fn direct_pick_confirms_and_model_change_marks_stale() {
        let mut knowledge = knowledge_with_parts();
        let mut overlay = ReviewOverlay::default();
        let sha = "a".repeat(64);
        let patch = DraftPatch {
            hotspots: Some(HotspotPatch {
                upsert: vec![HotspotUpsert {
                    id: None,
                    part_id: "part-1".to_owned(),
                    status: HotspotStatus::Confirmed,
                    anchor: Some(Anchor {
                        model_revision_id: "rev-1".to_owned(),
                        model_sha256: sha.clone(),
                        position_local: [0.5, 0.25, -0.75],
                    }),
                }],
                remove: Vec::new(),
            }),
            ..DraftPatch::default()
        };
        apply_draft_patch(
            &mut knowledge,
            &mut overlay,
            &patch,
            "admin",
            Timestamp::now(),
        )
        .expect("人工直接拾取 unbound → confirmed");
        assert_eq!(knowledge.hotspots.len(), 1);
        assert_eq!(knowledge.hotspots[0].status, HotspotStatus::Confirmed);

        // 换模型（新 revision + 新 sha）→ 旧绑定必须降级 stale。
        knowledge.model = Some(model("rev-2", &"c".repeat(64)));
        normalize_stale_hotspots(&mut knowledge);
        assert_eq!(knowledge.hotspots[0].status, HotspotStatus::Stale);
        assert_eq!(
            knowledge.hotspots[0]
                .anchor
                .as_ref()
                .map(|a| a.model_revision_id.clone()),
            Some("rev-1".to_owned()),
            "旧 anchor 保留用于解释"
        );
    }

    #[test]
    fn entity_edits_cannot_touch_supplier_snapshot_and_text_only_needs_review() {
        let mut knowledge = knowledge_with_parts();
        let mut overlay = ReviewOverlay::default();
        // 只支持本实体类型的字段。
        let patch = DraftPatch {
            entities: Some(BTreeMap::from([(
                "part-1".to_owned(),
                EntityReviewPatch {
                    review_status: None,
                    user_edited: Some(UserEdit {
                        title: Some("错字段".to_owned()),
                        ..UserEdit::default()
                    }),
                    text_only: None,
                },
            )])),
            ..DraftPatch::default()
        };
        let issues = apply_draft_patch(
            &mut knowledge,
            &mut overlay,
            &patch,
            "admin",
            Timestamp::now(),
        )
        .unwrap_err();
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("不支持人工修订字段"))
        );

        // 仅文本条目必须先复核。
        let patch = DraftPatch {
            entities: Some(BTreeMap::from([(
                "part-1".to_owned(),
                EntityReviewPatch {
                    review_status: None,
                    user_edited: None,
                    text_only: Some(true),
                },
            )])),
            ..DraftPatch::default()
        };
        let issues = apply_draft_patch(
            &mut knowledge,
            &mut overlay,
            &patch,
            "admin",
            Timestamp::now(),
        )
        .unwrap_err();
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("仅文本条目"))
        );

        // 确认 + 仅文本条目可以。
        let patch = DraftPatch {
            entities: Some(BTreeMap::from([(
                "part-1".to_owned(),
                EntityReviewPatch {
                    review_status: Some(ReviewStatus::Confirmed),
                    user_edited: None,
                    text_only: Some(true),
                },
            )])),
            ..DraftPatch::default()
        };
        apply_draft_patch(
            &mut knowledge,
            &mut overlay,
            &patch,
            "admin",
            Timestamp::now(),
        )
        .expect("确认 + 仅文本条目");
        assert!(overlay.is_reviewed("part-1"));
        assert!(overlay.entity("part-1").unwrap().text_only);
        // 供应商快照未变。
        assert_eq!(knowledge.knowledge.as_ref().unwrap().parts[0].name, "后盖");
    }

    #[test]
    fn model_review_requires_loaded_then_confirmation_and_server_stamps_time() {
        let mut knowledge = knowledge_with_parts();
        let mut overlay = ReviewOverlay::default();
        let unconfirmed = DraftPatch {
            model_review: Some(ModelReviewPatch {
                loaded: false,
                user_confirmed: true,
            }),
            ..DraftPatch::default()
        };
        let issues = apply_draft_patch(
            &mut knowledge,
            &mut overlay,
            &unconfirmed,
            "admin",
            Timestamp::now(),
        )
        .unwrap_err();
        assert!(issues.iter().any(|issue| issue.message.contains("loaded")));

        let patch = DraftPatch {
            model_review: Some(ModelReviewPatch {
                loaded: true,
                user_confirmed: true,
            }),
            ..DraftPatch::default()
        };
        apply_draft_patch(
            &mut knowledge,
            &mut overlay,
            &patch,
            "admin",
            Timestamp::now(),
        )
        .expect("loaded + userConfirmed");
        let review = overlay.model_review.as_ref().expect("modelReview 已写入");
        assert!(review.loaded && review.user_confirmed);
        assert!(review.checked_at > 0, "checkedAt 由服务器赋值");
        assert!(review.loaded_at.is_some() && review.user_confirmed_at.is_some());
        assert_eq!(review.model_revision_id, "rev-1");
        assert!(review.is_complete_for("rev-1", &"a".repeat(64)));
        assert!(!review.is_complete_for("rev-2", &"c".repeat(64)));
    }
}
