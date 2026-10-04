//! 交互式模型层（ADR-042）：分件 GLB + 部件绑定 + 动作 + 姿势。
//!
//! 为什么单独一层：
//! - 热点（`hotspots`）只回答"部件在模型表面的哪一点"；交互需要"哪一块几何属于该部件、
//!   怎么动"。分件 GLB 由 Tripo `mesh/segment` 产生，与草稿模型 revision **同一坐标系**，
//!   因此它是该 revision 的**附件**，不是新的模型版本：锚点身份（revisionId + sha256）不变；
//! - 绑定与动作写在草稿知识外壳里，随发布冻结进 release manifest（不可变），离线导出也带上；
//! - 节点用 GLB 中的**节点名**引用（分件 GLB 的节点名由附件冻结，`partsSha256` 固定内容）。
//!   这与"锚点不得用运行时 mesh.uuid"不冲突：节点名属于不可变附件文件本身。
//!
//! 动作只描述**展示用的刚体变换**（平移/绕轴旋转），不声称机械结构真实，界面须如实说明。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::knowledge::DraftKnowledge;
use manual_core::validation::FieldIssue;

/// 单个草稿允许的上限（防止把知识外壳当大文件存储）。
pub const MAX_BINDINGS: usize = 400;
pub const MAX_ACTIONS: usize = 40;
pub const MAX_POSES: usize = 20;
pub const MAX_STEPS_PER_ACTION: usize = 16;
pub const MAX_NODES_PER_REF: usize = 64;
const MAX_LABEL_CHARS: usize = 60;
const MAX_ID_CHARS: usize = 64;
const MAX_NODE_NAME_CHARS: usize = 128;
/// 平移/枢轴坐标的绝对值上限（asset-root 局部坐标；模型已归一到约 ±0.5）。
const MAX_COORD: f64 = 10.0;
const MAX_ANGLE_DEG: f64 = 360.0;
const MIN_DURATION_MS: u32 = 0;
const MAX_DURATION_MS: u32 = 10_000;

/// 分件模型附件（必须与草稿当前模型 revision + sha256 完全一致才可用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PartsModel {
    pub asset_id: String,
    pub sha256: String,
    /// 分件所依附的模型版本（= 草稿 `model.revisionId`）。
    pub model_revision_id: String,
    pub model_sha256: String,
    /// GLB 中的全部节点名（服务端在附加时从文件读出；绑定只能引用这些名字）。
    pub node_names: Vec<String>,
    /// 来源说明（如 `tripo:mesh_segment v2.0-20260430`）；只作展示与审计。
    pub source: String,
}

/// 部件 → 分件节点的绑定（一个部件可对应多个节点）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PartBinding {
    pub part_id: String,
    pub nodes: Vec<String>,
    /// `auto`（自动绑定产出，待复核）/ `confirmed`（人工确认）。
    pub status: BindingStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BindingStatus {
    Auto,
    Confirmed,
}

/// 一个刚体变换步（相对节点**初始**位姿；按顺序叠加）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransformStep {
    pub nodes: Vec<String>,
    /// `translate`：`vector` 为位移；`rotate`：绕 `pivot` 沿 `axis` 旋转 `angleDeg`。
    pub kind: TransformKind,
    #[serde(default)]
    #[schema(nullable = true)]
    pub vector: Option<[f64; 3]>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub pivot: Option<[f64; 3]>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub axis: Option<[f64; 3]>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub angle_deg: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransformKind {
    Translate,
    Rotate,
}

/// 用户可触发的动作（例如"取下电池盖"）：`toggle` 在初始/目标之间切换，`pulse` 做一次往返。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAction {
    pub id: String,
    pub label: String,
    #[serde(default)]
    #[schema(nullable = true)]
    pub description: Option<String>,
    /// 触发该动作的部件（点击部件/热点时提供该动作）。
    #[serde(default)]
    pub trigger_part_ids: Vec<String>,
    pub mode: ActionMode,
    pub duration_ms: u32,
    pub steps: Vec<TransformStep>,
    /// 关联的说明书步骤（阅读时切到该步骤会提示此动作）。
    #[serde(default)]
    pub step_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionMode {
    Toggle,
    Pulse,
}

/// 整体姿势（例如机器人"趴下/坐下"）：一组变换步，切换姿势时从初始位姿重新叠加。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelPose {
    pub id: String,
    pub label: String,
    #[serde(default)]
    #[schema(nullable = true)]
    pub description: Option<String>,
    pub duration_ms: u32,
    pub steps: Vec<TransformStep>,
}

/// 交互层（`knowledge.interactive`；旧草稿没有该字段 → `None`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Interactive {
    pub parts_model: PartsModel,
    #[serde(default)]
    pub bindings: Vec<PartBinding>,
    #[serde(default)]
    pub actions: Vec<ModelAction>,
    #[serde(default)]
    pub poses: Vec<ModelPose>,
}

/// PATCH 写入：整体替换绑定/动作/姿势（分件附件由专门接口挂载，PATCH 不可改）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractivePatch {
    #[serde(default)]
    #[schema(nullable = true)]
    pub bindings: Option<Vec<PartBinding>>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub actions: Option<Vec<ModelAction>>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub poses: Option<Vec<ModelPose>>,
}

impl InteractivePatch {
    pub fn is_empty(&self) -> bool {
        self.bindings.is_none() && self.actions.is_none() && self.poses.is_none()
    }
}

/// 分件附件是否仍属于草稿当前模型（模型重生成后旧附件失效，交互层不可用）。
pub fn parts_model_matches(knowledge: &DraftKnowledge, parts: &PartsModel) -> bool {
    knowledge.model.as_ref().is_some_and(|model| {
        model.revision_id == parts.model_revision_id && model.sha256 == parts.model_sha256
    })
}

fn finite3(value: &[f64; 3]) -> bool {
    value.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
}

fn short_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max
}

fn validate_steps(
    field: &str,
    owner: &str,
    steps: &[TransformStep],
    nodes: &BTreeSet<&str>,
    issues: &mut Vec<FieldIssue>,
) {
    if steps.is_empty() || steps.len() > MAX_STEPS_PER_ACTION {
        issues.push(FieldIssue::new(
            field,
            format!("{owner}：变换步数须在 1..={MAX_STEPS_PER_ACTION}"),
        ));
    }
    for step in steps {
        if step.nodes.is_empty() || step.nodes.len() > MAX_NODES_PER_REF {
            issues.push(FieldIssue::new(
                field,
                format!("{owner}：每步须引用 1..={MAX_NODES_PER_REF} 个节点"),
            ));
        }
        for node in &step.nodes {
            if !nodes.contains(node.as_str()) {
                issues.push(FieldIssue::new(
                    field,
                    format!("{owner}：分件模型中没有节点 {node}"),
                ));
            }
        }
        match step.kind {
            TransformKind::Translate => {
                if !step.vector.as_ref().is_some_and(finite3) {
                    issues.push(FieldIssue::new(
                        field,
                        format!("{owner}：translate 需要有限的 vector"),
                    ));
                }
            }
            TransformKind::Rotate => {
                let axis_ok = step.axis.as_ref().is_some_and(|axis| {
                    finite3(axis) && axis.iter().map(|v| v * v).sum::<f64>() > 1e-12
                });
                let pivot_ok = step.pivot.as_ref().is_some_and(finite3);
                let angle_ok = step
                    .angle_deg
                    .is_some_and(|angle| angle.is_finite() && angle.abs() <= MAX_ANGLE_DEG);
                if !(axis_ok && pivot_ok && angle_ok) {
                    issues.push(FieldIssue::new(
                        field,
                        format!("{owner}：rotate 需要非零 axis、有限 pivot 与 |angleDeg| ≤ {MAX_ANGLE_DEG}"),
                    ));
                }
            }
        }
    }
}

/// 校验交互写入（字段级问题追加到 `issues`；调用方保证原子应用）。
pub fn validate_interactive_patch(
    knowledge: &DraftKnowledge,
    patch: &InteractivePatch,
    issues: &mut Vec<FieldIssue>,
) {
    let Some(interactive) = knowledge.interactive.as_ref() else {
        issues.push(FieldIssue::new(
            "interactive",
            "草稿还没有分件模型：先挂载分件 GLB（POST …/drafts/{draftId}/parts-model）",
        ));
        return;
    };
    if !parts_model_matches(knowledge, &interactive.parts_model) {
        issues.push(FieldIssue::new(
            "interactive",
            "分件模型属于旧的模型版本：请在当前模型上重新分件后再编辑交互",
        ));
        return;
    }
    let nodes: BTreeSet<&str> = interactive
        .parts_model
        .node_names
        .iter()
        .map(String::as_str)
        .collect();
    let part_ids: BTreeSet<&str> = knowledge
        .knowledge
        .as_ref()
        .map(|merged| merged.parts.iter().map(|part| part.id.as_str()).collect())
        .unwrap_or_default();
    let step_ids: BTreeSet<&str> = knowledge
        .knowledge
        .as_ref()
        .map(|merged| merged.steps.iter().map(|step| step.id.as_str()).collect())
        .unwrap_or_default();

    if let Some(bindings) = &patch.bindings {
        if bindings.len() > MAX_BINDINGS {
            issues.push(FieldIssue::new(
                "interactive.bindings",
                format!("绑定数不得超过 {MAX_BINDINGS}"),
            ));
        }
        let mut seen = BTreeSet::new();
        for binding in bindings {
            if !part_ids.contains(binding.part_id.as_str()) {
                issues.push(FieldIssue::new(
                    "interactive.bindings",
                    format!("部件不存在：{}", binding.part_id),
                ));
            }
            if !seen.insert(binding.part_id.as_str()) {
                issues.push(FieldIssue::new(
                    "interactive.bindings",
                    format!("部件重复绑定：{}", binding.part_id),
                ));
            }
            if binding.nodes.is_empty() || binding.nodes.len() > MAX_NODES_PER_REF {
                issues.push(FieldIssue::new(
                    "interactive.bindings",
                    format!(
                        "部件 {} 须绑定 1..={MAX_NODES_PER_REF} 个节点",
                        binding.part_id
                    ),
                ));
            }
            for node in &binding.nodes {
                if !nodes.contains(node.as_str()) {
                    issues.push(FieldIssue::new(
                        "interactive.bindings",
                        format!("分件模型中没有节点 {node}"),
                    ));
                }
            }
        }
    }
    if let Some(actions) = &patch.actions {
        if actions.len() > MAX_ACTIONS {
            issues.push(FieldIssue::new(
                "interactive.actions",
                format!("动作数不得超过 {MAX_ACTIONS}"),
            ));
        }
        let mut seen = BTreeSet::new();
        for action in actions {
            let owner = format!("动作 {}", action.id);
            if !short_text(&action.id, MAX_ID_CHARS) || !seen.insert(action.id.as_str()) {
                issues.push(FieldIssue::new(
                    "interactive.actions",
                    format!("{owner}：id 为空、过长或重复"),
                ));
            }
            if !short_text(&action.label, MAX_LABEL_CHARS) {
                issues.push(FieldIssue::new(
                    "interactive.actions",
                    format!("{owner}：label 须为 1..={MAX_LABEL_CHARS} 字"),
                ));
            }
            if !(MIN_DURATION_MS..=MAX_DURATION_MS).contains(&action.duration_ms) {
                issues.push(FieldIssue::new(
                    "interactive.actions",
                    format!("{owner}：durationMs 须 ≤ {MAX_DURATION_MS}"),
                ));
            }
            for part_id in &action.trigger_part_ids {
                if !part_ids.contains(part_id.as_str()) {
                    issues.push(FieldIssue::new(
                        "interactive.actions",
                        format!("{owner}：触发部件不存在 {part_id}"),
                    ));
                }
            }
            for step_id in &action.step_ids {
                if !step_ids.contains(step_id.as_str()) {
                    issues.push(FieldIssue::new(
                        "interactive.actions",
                        format!("{owner}：步骤不存在 {step_id}"),
                    ));
                }
            }
            validate_steps("interactive.actions", &owner, &action.steps, &nodes, issues);
        }
    }
    if let Some(poses) = &patch.poses {
        if poses.len() > MAX_POSES {
            issues.push(FieldIssue::new(
                "interactive.poses",
                format!("姿势数不得超过 {MAX_POSES}"),
            ));
        }
        let mut seen = BTreeSet::new();
        for pose in poses {
            let owner = format!("姿势 {}", pose.id);
            if !short_text(&pose.id, MAX_ID_CHARS) || !seen.insert(pose.id.as_str()) {
                issues.push(FieldIssue::new(
                    "interactive.poses",
                    format!("{owner}：id 为空、过长或重复"),
                ));
            }
            if !short_text(&pose.label, MAX_LABEL_CHARS) {
                issues.push(FieldIssue::new(
                    "interactive.poses",
                    format!("{owner}：label 须为 1..={MAX_LABEL_CHARS} 字"),
                ));
            }
            if pose.duration_ms > MAX_DURATION_MS {
                issues.push(FieldIssue::new(
                    "interactive.poses",
                    format!("{owner}：durationMs 须 ≤ {MAX_DURATION_MS}"),
                ));
            }
            validate_steps("interactive.poses", &owner, &pose.steps, &nodes, issues);
        }
    }
}

/// 应用已校验的写入（整体替换给出的集合）。
pub fn apply_interactive_patch(knowledge: &mut DraftKnowledge, patch: &InteractivePatch) {
    let Some(interactive) = knowledge.interactive.as_mut() else {
        return;
    };
    if let Some(bindings) = &patch.bindings {
        interactive.bindings = bindings.clone();
    }
    if let Some(actions) = &patch.actions {
        interactive.actions = actions.clone();
    }
    if let Some(poses) = &patch.poses {
        interactive.poses = poses.clone();
    }
}

/// 节点名合法性（附加分件模型时检查）。
pub fn valid_node_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NODE_NAME_CHARS
        && !name.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drafts::knowledge::DraftModelInfo;
    use manual_core::domain::ModelValidationState;

    fn knowledge_with(parts_model_revision: &str) -> DraftKnowledge {
        let merged: manual_core::knowledge::MergedKnowledge = serde_json::from_value(serde_json::json!({
            "schemaVersion": "manual_extract_v1", "promptVersion": "p", "pageFrom": 1, "pageTo": 1,
            "parts": [{"id":"part-a","name":"电池盖","description":"","evidence":[{"pageNumber":1,"quote":null,"documentId":"d","preparationId":"p","bbox":null,"derived":false}],"reviewStatus":"needs_review","sourceBatches":[0]}],
            "steps": [{"id":"step-1","title":"装电池","orderedActions":["打开"],"partIds":["part-a"],"safetyNotes":[],"evidence":[{"pageNumber":1,"quote":null,"documentId":"d","preparationId":"p","bbox":null,"derived":false}],"reviewStatus":"needs_review","sourceBatches":[0]}],
            "specs": [], "uncertainties": [], "conflicts": [],
            "coverage": {"batches": [], "complete": true, "pageCount": 1, "pages": [1]}
        }))
        .expect("合并知识 fixture");
        let mut knowledge = DraftKnowledge::build(
            "job",
            Some(DraftModelInfo {
                revision_id: "rev-1".into(),
                sha256: "a".repeat(64),
                validation_state: ModelValidationState::Validated,
                asset_id: "asset-1".into(),
                bounds: None,
            }),
            Some(merged),
            Vec::new(),
        );
        knowledge.interactive = Some(Interactive {
            parts_model: PartsModel {
                asset_id: "parts".into(),
                sha256: "b".repeat(64),
                model_revision_id: parts_model_revision.into(),
                model_sha256: "a".repeat(64),
                node_names: vec!["body".into(), "cover".into()],
                source: "test".into(),
            },
            bindings: Vec::new(),
            actions: Vec::new(),
            poses: Vec::new(),
        });
        knowledge
    }

    fn rotate(nodes: &[&str]) -> TransformStep {
        TransformStep {
            nodes: nodes.iter().map(|n| (*n).to_owned()).collect(),
            kind: TransformKind::Rotate,
            vector: None,
            pivot: Some([0.0, 0.1, 0.0]),
            axis: Some([0.0, 0.0, 1.0]),
            angle_deg: Some(-80.0),
        }
    }

    fn action(nodes: &[&str]) -> ModelAction {
        ModelAction {
            id: "open-cover".into(),
            label: "取下电池盖".into(),
            description: None,
            trigger_part_ids: vec!["part-a".into()],
            mode: ActionMode::Toggle,
            duration_ms: 800,
            steps: vec![rotate(nodes)],
            step_ids: vec!["step-1".into()],
        }
    }

    #[test]
    fn valid_patch_is_accepted_and_applied() {
        let mut knowledge = knowledge_with("rev-1");
        let patch = InteractivePatch {
            bindings: Some(vec![PartBinding {
                part_id: "part-a".into(),
                nodes: vec!["cover".into()],
                status: BindingStatus::Auto,
            }]),
            actions: Some(vec![action(&["cover"])]),
            poses: None,
        };
        let mut issues = Vec::new();
        validate_interactive_patch(&knowledge, &patch, &mut issues);
        assert!(issues.is_empty(), "{issues:?}");
        apply_interactive_patch(&mut knowledge, &patch);
        let interactive = knowledge.interactive.as_ref().unwrap();
        assert_eq!(interactive.bindings.len(), 1);
        assert_eq!(interactive.actions[0].label, "取下电池盖");
    }

    #[test]
    fn unknown_nodes_parts_and_bad_transforms_are_rejected() {
        let knowledge = knowledge_with("rev-1");
        let mut bad_rotate = action(&["cover"]);
        bad_rotate.steps[0].axis = Some([0.0, 0.0, 0.0]);
        let patch = InteractivePatch {
            bindings: Some(vec![PartBinding {
                part_id: "part-x".into(),
                nodes: vec!["ghost".into()],
                status: BindingStatus::Auto,
            }]),
            actions: Some(vec![action(&["ghost"]), bad_rotate]),
            poses: None,
        };
        let mut issues = Vec::new();
        validate_interactive_patch(&knowledge, &patch, &mut issues);
        let text = format!("{issues:?}");
        assert!(text.contains("部件不存在：part-x"), "{text}");
        assert!(text.contains("没有节点 ghost"), "{text}");
        assert!(text.contains("id 为空、过长或重复"), "{text}");
        assert!(text.contains("非零 axis"), "{text}");
    }

    #[test]
    fn parts_model_from_old_revision_blocks_editing() {
        let knowledge = knowledge_with("rev-old");
        let patch = InteractivePatch {
            bindings: Some(Vec::new()),
            actions: None,
            poses: None,
        };
        let mut issues = Vec::new();
        validate_interactive_patch(&knowledge, &patch, &mut issues);
        assert!(format!("{issues:?}").contains("旧的模型版本"), "{issues:?}");
    }

    #[test]
    fn old_drafts_without_interactive_still_deserialize_and_omit_the_field() {
        let mut knowledge = knowledge_with("rev-1");
        knowledge.interactive = None;
        let value = serde_json::to_value(&knowledge).unwrap();
        assert!(value.get("interactive").is_none());
        let back: DraftKnowledge = serde_json::from_value(value).unwrap();
        assert!(back.interactive.is_none());
    }
}
