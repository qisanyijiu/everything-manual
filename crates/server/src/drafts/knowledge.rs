//! 草稿知识聚合的版本化外壳（T15）。
//!
//! `manual_drafts.knowledge_json` 的机器可读形状。为什么需要一层外壳而不是直接存
//! `MergedKnowledge`：草稿可能来自**部分成功**的组装（某条分支被阻塞），
//! 必须能表达"哪条分支缺、缺什么"，同时保持两条分支各自的原始形状可被 T19
//! 继续校验（Part/Step/Evidence/Hotspot 引用）。
//!
//! 版本与兼容：`schemaVersion` 是外壳版本（`manual_draft_v1`），内部 `knowledge`
//! 保留合并阶段自己的 `schemaVersion`（`manual_extract_v1`）。
//!
//! **T19 为何仍写 `manual_draft_v1`（2026-09-12 记录，替代此处原先"扩展即递增版本"的
//! 注释）**：T19 的新增字段（`hotspots`、`stepPoses`）是**向后兼容的追加**——旧读者
//! 忽略未知字段、新读者用 `serde(default)` 读旧草稿（空集合）；语义版本只在"旧读者
//! 会误读新数据"时才有必要递增，本卡不满足该条件。递增字符串反而会让已验收的
//! QA 证据（`qa_t15_independent.rs` 断言 `schemaVersion == "manual_draft_v1"`）失效，
//! 却不增加任何保护。读取器对两个版本字符串都接受（字段级宽容，见上）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use manual_core::domain::ModelValidationState;
use manual_core::knowledge::MergedKnowledge;

/// 外壳版本（写进每一份新草稿；T19 变更时递增）。
pub const DRAFT_SCHEMA_VERSION: &str = "manual_draft_v1";

/// 缺项代码（稳定标识；UI/测试按它定位"去补齐"入口）。
pub const CODE_MODEL_BRANCH_INCOMPLETE: &str = "model_branch_incomplete";
pub const CODE_KNOWLEDGE_BRANCH_INCOMPLETE: &str = "knowledge_branch_incomplete";
pub const CODE_MODEL_REVISION_MISSING: &str = "model_revision_missing";

/// 草稿完备性（部分成功可展示的核心字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftCompleteness {
    /// 两条分支都产出了产物。
    Complete,
    /// 至少一条分支被阻塞：草稿仍可复核，`missing[]` 逐条说明缺什么。
    Partial,
}

impl DraftCompleteness {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }
}

/// 模型分支的产物引用（不可变 revision；`validated` 才进入这里）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftModelInfo {
    pub revision_id: String,
    pub sha256: String,
    pub validation_state: ModelValidationState,
    pub asset_id: String,
    /// 结构摘要（包围盒等；来自 `model_validate` 的事实，不含 URL）。
    pub bounds: Option<Value>,
}

/// 一条缺项（部分成功时展示；`code` 稳定，`message` 面向用户、不含路径与密钥）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftMissingItem {
    pub code: String,
    pub message: String,
}

impl DraftMissingItem {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// 草稿知识外壳（`knowledge_json` 的反序列化形状）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftKnowledge {
    pub schema_version: String,
    /// 产生这份草稿的任务（审计与追溯；job 不随草稿删除）。
    pub source_job_id: String,
    pub completeness: DraftCompleteness,
    /// 模型分支未完成时为 `None`。
    pub model: Option<DraftModelInfo>,
    /// 知识分支未完成时为 `None`；完成时是合并阶段的确定性结果。
    pub knowledge: Option<MergedKnowledge>,
    /// 热点（T19；`unbound → candidate → confirmed`，模型变化进 `stale`）。
    /// 旧草稿（T15 落库的 v1 形状）没有该字段 → 空集合（见模块文档的兼容说明）。
    #[serde(default)]
    pub hotspots: Vec<super::aggregate::Hotspot>,
    /// 步骤视角（T19；`{ "<stepId>": CameraPose }`，相对同一 asset-root）。
    #[serde(default)]
    pub step_poses: std::collections::BTreeMap<String, super::aggregate::CameraPose>,
    /// 缺项清单（`complete` 时为空）。
    pub missing: Vec<DraftMissingItem>,
}

impl DraftKnowledge {
    /// 由两条分支的产物构造（`model` / `knowledge` 至少其一为 `Some` 时调用方仍会带上 `missing`）。
    pub fn build(
        source_job_id: &str,
        model: Option<DraftModelInfo>,
        knowledge: Option<MergedKnowledge>,
        missing: Vec<DraftMissingItem>,
    ) -> Self {
        Self {
            schema_version: DRAFT_SCHEMA_VERSION.to_owned(),
            source_job_id: source_job_id.to_owned(),
            completeness: if missing.is_empty() {
                DraftCompleteness::Complete
            } else {
                DraftCompleteness::Partial
            },
            model,
            knowledge,
            hotspots: Vec::new(),
            step_poses: std::collections::BTreeMap::new(),
            missing,
        }
    }

    /// 缺项（UI 直接展示；`complete` 时为空）。
    pub fn missing_items(&self) -> &[DraftMissingItem] {
        &self.missing
    }

    /// 追加缺项后重算完备性（T19 的继承逻辑会补充缺项）。
    pub fn add_missing(&mut self, item: DraftMissingItem) {
        self.missing.push(item);
        self.completeness = DraftCompleteness::Partial;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completeness_follows_missing_items() {
        let complete = DraftKnowledge::build("job-1", None, None, Vec::new());
        assert_eq!(complete.completeness, DraftCompleteness::Complete);
        assert!(complete.missing_items().is_empty());

        let partial = DraftKnowledge::build(
            "job-1",
            None,
            None,
            vec![DraftMissingItem::new(
                CODE_KNOWLEDGE_BRANCH_INCOMPLETE,
                "知识分支未完成",
            )],
        );
        assert_eq!(partial.completeness, DraftCompleteness::Partial);
        assert_eq!(partial.missing.len(), 1);
    }

    #[test]
    fn envelope_round_trips_and_tolerates_new_fields() {
        let draft = DraftKnowledge::build(
            "job-7",
            None,
            None,
            vec![DraftMissingItem::new(
                CODE_MODEL_BRANCH_INCOMPLETE,
                "模型缺失",
            )],
        );
        let text = serde_json::to_string(&draft).unwrap();
        let parsed: DraftKnowledge = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, draft);
        // T19 允许在旧版本上追加字段：反序列化不得因未知字段失败。
        let mut value: Value = serde_json::from_str(&text).unwrap();
        value["futureField"] = serde_json::json!({ "hotspots": [] });
        let parsed: DraftKnowledge = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.completeness, DraftCompleteness::Partial);
    }
}
