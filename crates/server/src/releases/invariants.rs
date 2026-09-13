//! 发布不变量（T19 / REQ-035；contracts.md §7、PRD AC-055/AC-056）。
//!
//! 合同原文（发布事务必须验证）：
//! 1. 所有必需知识已确认或有明确人工修订记录；
//! 2. 引用页存在；
//! 3. 选中模型 validated 且浏览器加载复核通过（`modelReview.loaded` 与
//!    `userConfirmed` 均 true 且 revision/hash 匹配）；
//! 4. 每个要发布的交互部件至少一个 confirmed 热点且 hash 匹配；
//! 5. 步骤引用全部存在；
//! 6. 没有 stale/candidate 热点冒充 confirmed；
//! 7. 无法绑定的知识可在"仅文本条目"模式保留并**明显标识**，不能为了发布自动隐藏必需内容。
//!
//! 本模块是**纯函数**（不访问数据库）：输入冻结的草稿聚合、复核覆盖层与冻结输入的
//! preparation 事实（页数/文档归属），输出逐条不满足项（`code` 稳定，供 UI 定位与
//! 测试断言）。服务层把问题列表映射为 422 + `details.issues`。
//!
//! 为什么要独立成纯函数：发布是最不可退让的写动作（不可变、可追溯）。把判据与
//! 数据库/HTTP 解耦，才能对每条不变量做正/负例单元测试，并且让"界面预检"与
//! "服务端真判"有同一个可对照的语义来源（服务端仍是唯一权威）。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::drafts::aggregate::{Anchor, HotspotStatus, ReviewOverlay};
use crate::drafts::knowledge::DraftKnowledge;

/// 冻结输入的页/文档事实（来自 `generation_snapshots.preparation_id` → `preparations`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenInput {
    pub preparation_id: String,
    pub document_id: String,
    /// ready 准备的页数（`pages` 连续 1..N，封存后不可修改）。
    pub page_count: i64,
    /// 准备是否 ready（未 ready 的输入不可能有合法知识，但仍防御性检查）。
    pub ready: bool,
}

/// 一条发布不满足项（422 `details.issues[]`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublishIssue {
    /// 稳定代码（UI 用它选择"去处理"的定位方式；测试按它断言）。
    pub code: String,
    /// 实体类别（`part` / `step` / `spec` / `hotspot` / `model` / `evidence` / `input`）。
    pub entity_kind: String,
    /// 实体 id（能定位时给出；模型/输入级问题为 null）。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub entity_id: Option<String>,
    /// 面向用户的中文说明（不含内部细节与路径）。
    pub message: String,
}

impl PublishIssue {
    pub fn new(
        code: &str,
        entity_kind: &str,
        entity_id: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.to_owned(),
            entity_kind: entity_kind.to_owned(),
            entity_id,
            message: message.into(),
        }
    }
}

/// 问题代码（稳定；UI/测试与文案分离）。
pub const CODE_INPUT_UNAVAILABLE: &str = "inputUnavailable";
pub const CODE_KNOWLEDGE_MISSING: &str = "knowledgeMissing";
pub const CODE_MODEL_MISSING: &str = "modelMissing";
pub const CODE_MODEL_NOT_VALIDATED: &str = "modelNotValidated";
pub const CODE_MODEL_REVIEW_MISSING: &str = "modelReviewMissing";
pub const CODE_MODEL_REVIEW_INCOMPLETE: &str = "modelReviewIncomplete";
pub const CODE_MODEL_REVIEW_MISMATCH: &str = "modelReviewModelMismatch";
pub const CODE_ENTITY_UNREVIEWED: &str = "knowledgeUnreviewed";
pub const CODE_EVIDENCE_PAGE_MISSING: &str = "evidencePageMissing";
pub const CODE_STEP_PART_REFERENCE_MISSING: &str = "stepPartReferenceMissing";
pub const CODE_HOTSPOT_MISSING: &str = "hotspotMissing";
pub const CODE_HOTSPOT_PART_MISSING: &str = "hotspotPartMissing";
pub const CODE_HOTSPOT_NOT_MATCHING_MODEL: &str = "hotspotNotMatchingModel";

/// 逐条检查发布不变量（顺序稳定：输入 → 模型 → 复核 → 知识 → 引用 → 热点）。
pub fn check_publish_invariants(
    knowledge: &DraftKnowledge,
    overlay: &ReviewOverlay,
    input: Option<&FrozenInput>,
) -> Vec<PublishIssue> {
    let mut issues: Vec<PublishIssue> = Vec::new();

    match input {
        None => issues.push(PublishIssue::new(
            CODE_INPUT_UNAVAILABLE,
            "input",
            None,
            "找不到本草稿冻结的输入（preparation）：无法校验引用页，拒绝发布",
        )),
        Some(frozen) if !frozen.ready => issues.push(PublishIssue::new(
            CODE_INPUT_UNAVAILABLE,
            "input",
            Some(frozen.preparation_id.clone()),
            "冻结输入的准备尚未封存（ready）：拒绝发布",
        )),
        Some(_) => {}
    }

    // 模型：必须是草稿选中的 validated 版本。
    let model = knowledge.model.as_ref();
    match model {
        None => issues.push(PublishIssue::new(
            CODE_MODEL_MISSING,
            "model",
            None,
            "草稿没有可用模型：完成模型分支校验后才能发布（拒绝发布）",
        )),
        Some(model)
            if model.validation_state != manual_core::domain::ModelValidationState::Validated =>
        {
            issues.push(PublishIssue::new(
                CODE_MODEL_NOT_VALIDATED,
                "model",
                Some(model.revision_id.clone()),
                format!(
                    "选中模型未通过校验（{}）：不能进入已发布版本",
                    model.validation_state.as_str()
                ),
            ));
        }
        Some(_) => {}
    }

    // modelReview：双真 + revision/hash 匹配（用户声明，服务器赋值 checkedAt）。
    match (model, overlay.model_review.as_ref()) {
        (Some(model), None) => issues.push(PublishIssue::new(
            CODE_MODEL_REVIEW_MISSING,
            "model",
            Some(model.revision_id.clone()),
            "缺少模型复核声明：请先在 3D 中打开模型并完成「已在浏览器成功打开此模型」与\
             「我已核对模型与资料一致」两个动作",
        )),
        (Some(model), Some(review)) => {
            if !review.matches(&model.revision_id, &model.sha256) {
                issues.push(PublishIssue::new(
                    CODE_MODEL_REVIEW_MISMATCH,
                    "model",
                    Some(model.revision_id.clone()),
                    "模型复核声明的版本与当前模型不一致：换模型后必须重新复核",
                ));
            } else if !review.loaded || !review.user_confirmed {
                let missing_action = if !review.loaded {
                    "「已在浏览器成功打开此模型」"
                } else {
                    "「我已核对模型与资料一致」"
                };
                issues.push(PublishIssue::new(
                    CODE_MODEL_REVIEW_INCOMPLETE,
                    "model",
                    Some(model.revision_id.clone()),
                    format!("模型复核未完成：还需要你声明{missing_action}"),
                ));
            }
        }
        (None, _) => {}
    }

    let Some(merged) = knowledge.knowledge.as_ref() else {
        issues.push(PublishIssue::new(
            CODE_KNOWLEDGE_MISSING,
            "knowledge",
            None,
            "草稿没有知识内容（知识分支未完成）：无法发布",
        ));
        return issues;
    };

    // 必需知识：部件/步骤/规格都必须已确认或有人工修订记录。
    for part in &merged.parts {
        if !overlay.is_reviewed(&part.id) {
            issues.push(PublishIssue::new(
                CODE_ENTITY_UNREVIEWED,
                "part",
                Some(part.id.clone()),
                format!("部件「{}」尚未确认或修订", part.name),
            ));
        }
    }
    for step in &merged.steps {
        if !overlay.is_reviewed(&step.id) {
            issues.push(PublishIssue::new(
                CODE_ENTITY_UNREVIEWED,
                "step",
                Some(step.id.clone()),
                format!("步骤「{}」尚未确认或修订", step.title),
            ));
        }
    }
    for spec in &merged.specs {
        if !overlay.is_reviewed(&spec.id) {
            issues.push(PublishIssue::new(
                CODE_ENTITY_UNREVIEWED,
                "spec",
                Some(spec.id.clone()),
                format!("规格「{}」尚未确认或修订", spec.label),
            ));
        }
    }

    // 引用页必须存在于冻结输入（1-based、同一文档/准备）。
    // 冻结输入缺失时上面已经报过 `inputUnavailable`：这里不重复刷屏。
    let check_evidence = |entity_kind: &str,
                          entity_id: &str,
                          label: &str,
                          evidence: &[manual_core::knowledge::Evidence],
                          issues: &mut Vec<PublishIssue>| {
        let Some(frozen) = input.filter(|frozen| frozen.ready) else {
            return;
        };
        for entry in evidence {
            if entry.preparation_id != frozen.preparation_id
                || entry.document_id != frozen.document_id
                || entry.page_number < 1
                || entry.page_number > frozen.page_count
            {
                issues.push(PublishIssue::new(
                    CODE_EVIDENCE_PAGE_MISSING,
                    entity_kind,
                    Some(entity_id.to_owned()),
                    format!(
                        "{label}引用第 {} 页，不在本次提取范围（1–{}；文档/准备必须与冻结输入一致）",
                        entry.page_number, frozen.page_count
                    ),
                ));
            }
        }
    };
    for part in &merged.parts {
        check_evidence(
            "part",
            &part.id,
            &format!("部件「{}」", part.name),
            &part.evidence,
            &mut issues,
        );
    }
    for step in &merged.steps {
        check_evidence(
            "step",
            &step.id,
            &format!("步骤「{}」", step.title),
            &step.evidence,
            &mut issues,
        );
    }
    for spec in &merged.specs {
        check_evidence(
            "spec",
            &spec.id,
            &format!("规格「{}」", spec.label),
            &spec.evidence,
            &mut issues,
        );
    }

    // 步骤引用全部存在。
    let part_ids: Vec<&str> = merged.parts.iter().map(|part| part.id.as_str()).collect();
    for step in &merged.steps {
        for part_id in &step.part_ids {
            if !part_ids.contains(&part_id.as_str()) {
                issues.push(PublishIssue::new(
                    CODE_STEP_PART_REFERENCE_MISSING,
                    "step",
                    Some(step.id.clone()),
                    format!("步骤「{}」引用了不存在的部件（{}）", step.title, part_id),
                ));
            }
        }
    }

    // 热点：不得冒充 confirmed；每个"要发布的交互部件"至少一个 confirmed 且 hash 匹配。
    let model_identity = model.map(|model| (model.revision_id.as_str(), model.sha256.as_str()));
    for hotspot in &knowledge.hotspots {
        if !part_ids.contains(&hotspot.part_id.as_str()) {
            issues.push(PublishIssue::new(
                CODE_HOTSPOT_PART_MISSING,
                "hotspot",
                Some(hotspot.id.clone()),
                format!("热点引用了不存在的部件（{}）", hotspot.part_id),
            ));
            continue;
        }
        if hotspot.status.requires_anchor() {
            let valid = match (&hotspot.anchor, model_identity) {
                (Some(anchor), Some((revision_id, sha256))) => {
                    anchor_valid(anchor, revision_id, sha256)
                }
                _ => false,
            };
            if !valid {
                issues.push(PublishIssue::new(
                    CODE_HOTSPOT_NOT_MATCHING_MODEL,
                    "hotspot",
                    Some(hotspot.id.clone()),
                    format!(
                        "热点状态为 {} 但其锚点与当前模型不符（不得冒充 confirmed）：请重新绑定",
                        hotspot.status.as_str()
                    ),
                ));
            }
        }
    }
    for part in &merged.parts {
        let text_only = overlay
            .entity(&part.id)
            .is_some_and(|entry| entry.text_only);
        if text_only {
            continue;
        }
        let has_confirmed = knowledge.hotspots.iter().any(|hotspot| {
            hotspot.part_id == part.id
                && hotspot.status == HotspotStatus::Confirmed
                && match (&hotspot.anchor, model_identity) {
                    (Some(anchor), Some((revision_id, sha256))) => {
                        anchor_valid(anchor, revision_id, sha256)
                    }
                    _ => false,
                }
        });
        if !has_confirmed {
            issues.push(PublishIssue::new(
                CODE_HOTSPOT_MISSING,
                "part",
                Some(part.id.clone()),
                format!(
                    "部件「{}」还没有 confirmed 热点：请在模型上点选绑定（或在已确认后标记为\
                     「仅文本条目」并保留文字标识）",
                    part.name
                ),
            ));
        }
    }

    issues
}

/// 锚点是否数值有限且与给定模型身份完全匹配。
fn anchor_valid(anchor: &Anchor, revision_id: &str, sha256: &str) -> bool {
    anchor.has_finite_values() && anchor.matches_model(revision_id, sha256)
}
