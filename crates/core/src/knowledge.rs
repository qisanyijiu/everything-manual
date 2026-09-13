//! 说明书知识的领域类型、服务端严格校验与本地确定性合并（T14 / REQ-029）。
//!
//! 合同依据：contracts.md §6「Manual AI」（`extract_batch` / `merge_batches` 的语义）、
//! §2（Part/Step/Evidence/Review 的最小信息与规则）、architecture.md §5.2
//! （≤5 页/批、页覆盖、聚合去重但保留原始出处、拒答/截断/无证据结论进入待复核、
//! 不用正则"抢救"畸形 JSON、资料是待分析数据不是可信指令）。
//!
//! 本模块**纯逻辑**：不读数据库、不发 HTTP、不知道 reqwest。网络与持久化在
//! `crates/server/src/providers/manual_ai/**`。
//!
//! 三个不可退让的规则：
//!
//! 1. **模型输出只是候选**：模型给出局部 id、页号与引文；`documentId`/`preparationId`
//!    由服务端按冻结输入回填（要求模型复述 UUID 既不可靠也无必要）。服务端**再次校验**
//!    schema 形状、字符串长度、实体数量、引用页集合与部件引用关系；任何一项不满足
//!    → 该批不产生正式知识（`needs_input`），**不做任何"正则抢救"**——畸形 JSON 就是畸形；
//! 2. **`confidence` 不是已验真概率**：它既不进入请求 schema，也不参与任何判定；
//!    实体初始 `reviewStatus` 一律 `needsReview`（生成完成 ≠ 事实已核验，ADR-005）；
//! 3. **合并本地、确定、保留出处**：同内容去重但保留**全部**原始出处（evidence 并集）；
//!    同名不同事实**保留双方**并记为冲突待复核，不额外无限循环调用 AI。
//!
//! Schema 与限制常量的单一来源是 [`manual_extract_json_schema`]（由常量构造），
//! 请求构造与校验共用，避免"提示词里的数字与校验里的数字漂移"。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// 版本与限制（单一来源；JSON Schema 由这些常量构造）
// ---------------------------------------------------------------------------

/// 提取 schema 版本：出现在请求 `text.format.name`、模型输出的 `schemaVersion`、
/// 批次/合并结果 JSON 与任务快照（`prompt_version` 同为 `manual_extract_v1`）。
pub const MANUAL_EXTRACT_SCHEMA_VERSION: &str = "manual_extract_v1";
/// OpenAI structured outputs 的 schema 名（`text.format.name`；正则 `[a-zA-Z0-9_-]{1,64}`）。
pub const MANUAL_EXTRACT_SCHEMA_NAME: &str = "manual_extract_v1";

/// 实体数量上限（单批）。
pub const KNOWLEDGE_MAX_PARTS: usize = 64;
pub const KNOWLEDGE_MAX_STEPS: usize = 64;
pub const KNOWLEDGE_MAX_SPECS: usize = 128;
pub const KNOWLEDGE_MAX_UNCERTAINTIES: usize = 32;
/// 单批实体总数上限（parts + steps + specs + uncertainties）。
pub const KNOWLEDGE_MAX_ENTITY_TOTAL: usize = 240;
/// 单实体的 evidence 条数上限。
pub const KNOWLEDGE_MAX_EVIDENCE_PER_ENTITY: usize = 8;

/// 字符串长度上限（字符数，不是字节数；服务端校验与 schema `maxLength` 同源）。
pub const KNOWLEDGE_MAX_ID_CHARS: usize = 64;
pub const KNOWLEDGE_MAX_NAME_CHARS: usize = 120;
pub const KNOWLEDGE_MAX_DESCRIPTION_CHARS: usize = 1200;
pub const KNOWLEDGE_MAX_TITLE_CHARS: usize = 200;
pub const KNOWLEDGE_MAX_ACTION_CHARS: usize = 600;
pub const KNOWLEDGE_MAX_ACTIONS_PER_STEP: usize = 24;
pub const KNOWLEDGE_MAX_SAFETY_NOTES_PER_STEP: usize = 12;
pub const KNOWLEDGE_MAX_SPEC_LABEL_CHARS: usize = 120;
pub const KNOWLEDGE_MAX_SPEC_VALUE_CHARS: usize = 600;
pub const KNOWLEDGE_MAX_QUOTE_CHARS: usize = 400;
pub const KNOWLEDGE_MAX_UNCERTAINTY_TOPIC_CHARS: usize = 120;
pub const KNOWLEDGE_MAX_UNCERTAINTY_DETAIL_CHARS: usize = 600;

/// 稳定错误码（`needs_input` 缺项 / 诊断摘要；QA 与前端按这些码断言）。
pub const CODE_SCHEMA_VIOLATION: &str = "manual_ai_schema_violation";
/// `output_text` 根本不是合法 JSON（**不做正则抢救**）。
pub const CODE_INVALID_FORMAT: &str = "manual_ai_invalid_format";
pub const CODE_STRING_TOO_LONG: &str = "manual_ai_string_too_long";
pub const CODE_ENTITY_LIMIT: &str = "manual_ai_entity_limit";
pub const CODE_DUPLICATE_LOCAL_ID: &str = "manual_ai_duplicate_id";
pub const CODE_PAGE_REFERENCE_INVALID: &str = "manual_ai_page_reference_invalid";
pub const CODE_PART_REFERENCE_INVALID: &str = "manual_ai_part_reference_invalid";
pub const CODE_COVERAGE_INCOMPLETE: &str = "manual_coverage_incomplete";
pub const CODE_BATCH_WITHOUT_KNOWLEDGE: &str = "manual_batch_without_knowledge";
pub const CODE_PROMPT_VERSION_MISMATCH: &str = "manual_prompt_version_mismatch";

// ---------------------------------------------------------------------------
// 模型输出的线上形态（严格：未知字段 = schema 违规，不做容错读取）
// ---------------------------------------------------------------------------

/// 模型给出的 evidence：只含页号与（可空的）引文。
///
/// `documentId`/`preparationId` 由服务端回填——要求模型复述 UUID 既不可靠也无必要，
/// 且会造成"模型编造引用"的假出处风险。`quote` 为**可选值以 nullable 表达**
/// （JSON Schema `"type": ["string", "null"]`），不用省略键绕过 strict 模式。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawEvidence {
    /// 1-based 页号。
    pub page_number: i64,
    pub quote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawPart {
    pub id: String,
    pub name: String,
    pub description: String,
    pub evidence: Vec<RawEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawStep {
    pub id: String,
    pub title: String,
    pub ordered_actions: Vec<String>,
    /// 同一响应内的部件局部 id（服务端映射为应用 id；不存在的引用被拒绝）。
    pub part_ids: Vec<String>,
    pub evidence: Vec<RawEvidence>,
    pub safety_notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawSpec {
    pub id: String,
    pub label: String,
    pub value: String,
    pub evidence: Vec<RawEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawUncertainty {
    pub id: String,
    pub topic: String,
    pub detail: String,
}

/// 模型输出的结构化提取结果（`output_text` 解析后的目标形状）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawBatchResult {
    pub schema_version: String,
    pub parts: Vec<RawPart>,
    pub steps: Vec<RawStep>,
    pub specs: Vec<RawSpec>,
    pub uncertainties: Vec<RawUncertainty>,
}

/// 构造模型输出必须满足的 JSON Schema（`text.format.schema` 的值）。
///
/// 规则（contracts.md §6）：`additionalProperties: false`、**全部属性进入 `required`**、
/// 可选值用 nullable（`"type": ["string", "null"]`）。数字/长度上限由本模块常量构造，
/// 与 [`validate_batch_output`] 的校验同源。
pub fn manual_extract_json_schema() -> serde_json::Value {
    use serde_json::json;

    let evidence = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["pageNumber", "quote"],
        "properties": {
            "pageNumber": { "type": "integer", "minimum": 1 },
            "quote": { "type": ["string", "null"], "maxLength": KNOWLEDGE_MAX_QUOTE_CHARS }
        }
    });

    let string_schema =
        |max_chars: usize| json!({ "type": "string", "minLength": 1, "maxLength": max_chars });

    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["schemaVersion", "parts", "steps", "specs", "uncertainties"],
        "properties": {
            "schemaVersion": { "type": "string", "enum": [MANUAL_EXTRACT_SCHEMA_VERSION] },
            "parts": {
                "type": "array",
                "maxItems": KNOWLEDGE_MAX_PARTS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "name", "description", "evidence"],
                    "properties": {
                        "id": string_schema(KNOWLEDGE_MAX_ID_CHARS),
                        "name": string_schema(KNOWLEDGE_MAX_NAME_CHARS),
                        "description": string_schema(KNOWLEDGE_MAX_DESCRIPTION_CHARS),
                        "evidence": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": KNOWLEDGE_MAX_EVIDENCE_PER_ENTITY,
                            "items": evidence
                        }
                    }
                }
            },
            "steps": {
                "type": "array",
                "maxItems": KNOWLEDGE_MAX_STEPS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "title", "orderedActions", "partIds", "evidence", "safetyNotes"],
                    "properties": {
                        "id": string_schema(KNOWLEDGE_MAX_ID_CHARS),
                        "title": string_schema(KNOWLEDGE_MAX_TITLE_CHARS),
                        "orderedActions": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": KNOWLEDGE_MAX_ACTIONS_PER_STEP,
                            "items": string_schema(KNOWLEDGE_MAX_ACTION_CHARS)
                        },
                        "partIds": {
                            "type": "array",
                            "maxItems": KNOWLEDGE_MAX_PARTS,
                            "items": string_schema(KNOWLEDGE_MAX_ID_CHARS)
                        },
                        "evidence": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": KNOWLEDGE_MAX_EVIDENCE_PER_ENTITY,
                            "items": evidence
                        },
                        "safetyNotes": {
                            "type": "array",
                            "maxItems": KNOWLEDGE_MAX_SAFETY_NOTES_PER_STEP,
                            "items": string_schema(KNOWLEDGE_MAX_ACTION_CHARS)
                        }
                    }
                }
            },
            "specs": {
                "type": "array",
                "maxItems": KNOWLEDGE_MAX_SPECS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "label", "value", "evidence"],
                    "properties": {
                        "id": string_schema(KNOWLEDGE_MAX_ID_CHARS),
                        "label": string_schema(KNOWLEDGE_MAX_SPEC_LABEL_CHARS),
                        "value": string_schema(KNOWLEDGE_MAX_SPEC_VALUE_CHARS),
                        "evidence": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": KNOWLEDGE_MAX_EVIDENCE_PER_ENTITY,
                            "items": evidence
                        }
                    }
                }
            },
            "uncertainties": {
                "type": "array",
                "maxItems": KNOWLEDGE_MAX_UNCERTAINTIES,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "topic", "detail"],
                    "properties": {
                        "id": string_schema(KNOWLEDGE_MAX_ID_CHARS),
                        "topic": string_schema(KNOWLEDGE_MAX_UNCERTAINTY_TOPIC_CHARS),
                        "detail": string_schema(KNOWLEDGE_MAX_UNCERTAINTY_DETAIL_CHARS)
                    }
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// 服务端校验（再次校验；不通过 = 该批不产生正式知识）
// ---------------------------------------------------------------------------

/// 实体级复核状态（contracts.md §2「Review」：`confirmed` / `needs_review`）。
///
/// 自动提取的实体**一律**从 `NeedsReview` 开始：生成完成不等于事实已核验（ADR-005）。
/// 「已验真」只能由人工确认写入（T19），不来自模型自报的任何数值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Confirmed,
    NeedsReview,
}

impl ReviewStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::NeedsReview => "needs_review",
        }
    }
}

/// 规范化后的出处（contracts.md §2「Evidence」）。
///
/// `bbox` 本版本恒为 `None`：模型没有页图尺寸、无法可靠给框，"不得为了满足 schema
/// 捏造框"；字段保留供 T19 的人工修订/后续卡填充。
/// `derived = true` 表示该页以**页图**发送给模型（扫描页/无文字层页），引文是模型
/// 读图得到的派生文字，不是原 PDF 文字层。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub document_id: String,
    pub preparation_id: String,
    pub page_number: i64,
    pub quote: Option<String>,
    pub bbox: Option<[f64; 4]>,
    pub derived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    /// 应用分配的稳定 id（内容派生；模型局部 id 先做映射）。
    pub id: String,
    pub name: String,
    pub description: String,
    pub evidence: Vec<Evidence>,
    pub review_status: ReviewStatus,
    /// 该实体来自哪些批次（`BatchExtractionResult.batch_index`）。
    pub source_batches: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: String,
    pub title: String,
    pub ordered_actions: Vec<String>,
    /// 指向应用分配的部件 id（同批引用经映射；跨批引用不成立）。
    pub part_ids: Vec<String>,
    pub evidence: Vec<Evidence>,
    pub safety_notes: Vec<String>,
    pub review_status: ReviewStatus,
    pub source_batches: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    pub id: String,
    pub label: String,
    pub value: String,
    pub evidence: Vec<Evidence>,
    pub review_status: ReviewStatus,
    pub source_batches: Vec<i64>,
}

/// 单批提取的规范化结果（校验通过；`documentId`/`preparationId` 已回填）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredBatchResult {
    pub parts: Vec<Part>,
    pub steps: Vec<Step>,
    pub specs: Vec<Spec>,
    pub uncertainties: Vec<Uncertainty>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Uncertainty {
    pub id: String,
    pub topic: String,
    pub detail: String,
    /// 相关页（1-based）；去重后升序。
    pub page_numbers: Vec<i64>,
    pub source_batches: Vec<i64>,
}

/// 批次结论（“是否产出正式知识”的唯一判据是 [`BatchOutcome::produced_knowledge`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BatchOutcome {
    /// 完整响应 + schema/引用校验通过（唯一产出正式知识的结论）。
    Completed,
    /// 模型拒答（`output[].content[].type = refusal`）。
    Refusal,
    /// 响应 `status = incomplete`（含 `max_output_tokens` 截断）。
    Incomplete,
    /// `output_text` 不是合法 JSON（**不做正则抢救**）。
    InvalidFormat,
    /// 完整响应里没有 `output_text`。
    EmptyOutput,
    /// JSON 合法但不符合 schema/长度/数量/引用校验。
    SchemaViolation,
    /// 响应信封本身不可解析（保留了原始响应作为诊断路径）。
    EnvelopeInvalid,
    /// 响应 `status = failed` 之类：供应商侧未产出结果。
    ResponseFailed,
    /// HTTP 4xx：供应商明确拒绝（可证明未计费，不自动重试）。
    ProviderRejected,
}

impl BatchOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Refusal => "refusal",
            Self::Incomplete => "incomplete",
            Self::InvalidFormat => "invalidFormat",
            Self::EmptyOutput => "emptyOutput",
            Self::SchemaViolation => "schemaViolation",
            Self::EnvelopeInvalid => "envelopeInvalid",
            Self::ResponseFailed => "responseFailed",
            Self::ProviderRejected => "providerRejected",
        }
    }

    /// 是否产出**正式知识**（拒答/截断/格式错/校验失败一律 `false`）。
    pub const fn produced_knowledge(self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// 校验失败（`needs_input` 缺项与诊断摘要共用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchValidationError {
    /// 稳定错误码（[`CODE_SCHEMA_VIOLATION`] 等）。
    pub code: &'static str,
    /// 不含原文的可读说明（脱敏）。
    pub detail: String,
}

impl BatchValidationError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn schema_violation(detail: impl Into<String>) -> Self {
        Self::new(CODE_SCHEMA_VIOLATION, detail)
    }

    /// `output_text` 不是合法 JSON（与"JSON 合法但不符合 schema"分开，便于诊断）。
    pub fn invalid_format(detail: impl Into<String>) -> Self {
        Self::new(CODE_INVALID_FORMAT, detail)
    }

    pub fn too_long(what: &str, actual: usize, limit: usize) -> Self {
        Self::new(
            CODE_STRING_TOO_LONG,
            format!("{what} 长度 {actual} 超过上限 {limit} 字符"),
        )
    }

    pub fn detail_text(&self) -> String {
        format!("{}：{}", self.code, self.detail)
    }
}

impl std::fmt::Display for BatchValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail_text())
    }
}

impl std::error::Error for BatchValidationError {}

/// 单批校验的上下文（冻结输入：文档/准备/本批页集合/以页图发送的页）。
#[derive(Debug, Clone, Copy)]
pub struct BatchValidationContext<'a> {
    pub document_id: &'a str,
    pub preparation_id: &'a str,
    /// 本批输入页（1-based，升序；来自冻结计划的 `page_set`）。
    pub pages: &'a [i64],
    /// 本批以**页图**发送的页（扫描页/无文字层页；`derived` 标记的依据）。
    pub image_pages: &'a [i64],
    /// 期望的 schema 版本（快照冻结）。
    pub schema_version: &'a str,
}

/// 再次校验模型输出并规范化。
///
/// 顺序（提前失败，不做部分接受）：
/// 1. JSON 解析（失败 → `manual_ai_schema_violation`，不尝试任何修复）；
/// 2. `schemaVersion` 匹配；
/// 3. 实体数量上限；
/// 4. 字符串长度、空白、evidence 形状与数量；
/// 5. **引用页必须存在于本次输入**（1-based）；
/// 6. 局部 id 唯一 + **部件引用关系存在**（step.partIds 必须指向同批 parts）；
/// 7. 局部 id → 应用 id 映射（内容派生稳定 id），`derived` 由页图集合决定。
pub fn validate_batch_output(
    output_text: &str,
    context: &BatchValidationContext<'_>,
) -> Result<StructuredBatchResult, BatchValidationError> {
    // 两步解析：先判断"是不是 JSON"（语法），再判断"是不是本 schema 的形状"
    // （缺字段/未知字段/类型不符）。两者分开是为了可诊断（`invalid_format` vs
    // `schema_violation`），且**任何一步失败都不做正则抢救**。
    let value: serde_json::Value = serde_json::from_str(output_text).map_err(|error| {
        BatchValidationError::invalid_format(format!(
            "output_text 不是合法 JSON（manual_extract_v1 要求 JSON 对象）：{error}"
        ))
    })?;
    let raw: RawBatchResult = serde_json::from_value(value).map_err(|error| {
        BatchValidationError::schema_violation(format!(
            "output_text 不符合 manual_extract_v1 的形状：{error}"
        ))
    })?;

    if raw.schema_version != context.schema_version {
        return Err(BatchValidationError::schema_violation(format!(
            "schemaVersion 必须是 {}（实际 {}）",
            context.schema_version, raw.schema_version
        )));
    }

    let total = raw.parts.len() + raw.steps.len() + raw.specs.len() + raw.uncertainties.len();
    if raw.parts.len() > KNOWLEDGE_MAX_PARTS
        || raw.steps.len() > KNOWLEDGE_MAX_STEPS
        || raw.specs.len() > KNOWLEDGE_MAX_SPECS
        || raw.uncertainties.len() > KNOWLEDGE_MAX_UNCERTAINTIES
        || total > KNOWLEDGE_MAX_ENTITY_TOTAL
    {
        return Err(BatchValidationError::new(
            CODE_ENTITY_LIMIT,
            format!(
                "单批实体数量超限：parts={}（≤{KNOWLEDGE_MAX_PARTS}）、steps={}（≤{KNOWLEDGE_MAX_STEPS}）、\
                 specs={}（≤{KNOWLEDGE_MAX_SPECS}）、uncertainties={}（≤{KNOWLEDGE_MAX_UNCERTAINTIES}）、\
                 合计 {total}（≤{KNOWLEDGE_MAX_ENTITY_TOTAL}）",
                raw.parts.len(),
                raw.steps.len(),
                raw.specs.len(),
                raw.uncertainties.len(),
            ),
        ));
    }

    // 页引用集合：所有 evidence 的页号必须在本批输入页内（1-based）。
    let normalize_evidence =
        |evidence: &[RawEvidence], what: &str| -> Result<Vec<Evidence>, BatchValidationError> {
            if evidence.is_empty() {
                return Err(BatchValidationError::schema_violation(format!(
                    "{what} 至少需要 1 条 evidence（每项事实必须有页出处）"
                )));
            }
            if evidence.len() > KNOWLEDGE_MAX_EVIDENCE_PER_ENTITY {
                return Err(BatchValidationError::schema_violation(format!(
                    "{what} 的 evidence 条数 {} 超过上限 {KNOWLEDGE_MAX_EVIDENCE_PER_ENTITY}",
                    evidence.len()
                )));
            }
            let mut normalized = Vec::with_capacity(evidence.len());
            for item in evidence {
                if !context.pages.contains(&item.page_number) {
                    return Err(BatchValidationError::new(
                        CODE_PAGE_REFERENCE_INVALID,
                        format!(
                            "{what} 引用页 {} 不在本次输入页内（输入页：{}）；不接受跨批或伪造出处",
                            item.page_number,
                            join_pages(context.pages)
                        ),
                    ));
                }
                if let Some(quote) = &item.quote
                    && quote.chars().count() > KNOWLEDGE_MAX_QUOTE_CHARS
                {
                    return Err(BatchValidationError::too_long(
                        &format!("{what} 的引文"),
                        quote.chars().count(),
                        KNOWLEDGE_MAX_QUOTE_CHARS,
                    ));
                }
                normalized.push(Evidence {
                    document_id: context.document_id.to_owned(),
                    preparation_id: context.preparation_id.to_owned(),
                    page_number: item.page_number,
                    quote: item.quote.clone(),
                    // 本版本不产出 bbox（模型无页图尺寸；不得捏造框）。
                    bbox: None,
                    derived: context.image_pages.contains(&item.page_number),
                });
            }
            Ok(normalized)
        };

    // 部件：局部 id → 应用 id（内容派生，稳定且可去重）。
    let mut part_id_map: Vec<(&str, String)> = Vec::with_capacity(raw.parts.len());
    let mut parts: Vec<Part> = Vec::with_capacity(raw.parts.len());
    for part in &raw.parts {
        require_non_blank_id(&part.id, "part.id")?;
        require_text("part.name", &part.name, KNOWLEDGE_MAX_NAME_CHARS)?;
        require_text(
            "part.description",
            &part.description,
            KNOWLEDGE_MAX_DESCRIPTION_CHARS,
        )?;
        if part_id_map.iter().any(|(local, _)| *local == part.id) {
            return Err(BatchValidationError::new(
                CODE_DUPLICATE_LOCAL_ID,
                format!("部件局部 id 重复：{}", part.id),
            ));
        }
        let id = content_id("part", &[&part.name, &part.description]);
        part_id_map.push((part.id.as_str(), id.clone()));
        parts.push(Part {
            id,
            name: part.name.clone(),
            description: part.description.clone(),
            evidence: normalize_evidence(&part.evidence, &format!("部件 {}", part.id))?,
            review_status: ReviewStatus::NeedsReview,
            source_batches: Vec::new(),
        });
    }

    // 步骤：partIds 必须指向**同批**存在的部件（“部件引用关系存在”）。
    let mut step_ids: Vec<&str> = Vec::with_capacity(raw.steps.len());
    let mut steps: Vec<Step> = Vec::with_capacity(raw.steps.len());
    for step in &raw.steps {
        require_non_blank_id(&step.id, "step.id")?;
        if step_ids.contains(&step.id.as_str()) {
            return Err(BatchValidationError::new(
                CODE_DUPLICATE_LOCAL_ID,
                format!("步骤局部 id 重复：{}", step.id),
            ));
        }
        step_ids.push(step.id.as_str());
        require_text("step.title", &step.title, KNOWLEDGE_MAX_TITLE_CHARS)?;
        if step.ordered_actions.is_empty() {
            return Err(BatchValidationError::schema_violation(format!(
                "步骤 {} 至少需要 1 条 orderedActions",
                step.id
            )));
        }
        if step.ordered_actions.len() > KNOWLEDGE_MAX_ACTIONS_PER_STEP {
            return Err(BatchValidationError::schema_violation(format!(
                "步骤 {} 的 orderedActions 条数 {} 超过上限 {KNOWLEDGE_MAX_ACTIONS_PER_STEP}",
                step.id,
                step.ordered_actions.len()
            )));
        }
        for action in &step.ordered_actions {
            require_text("step.orderedActions[]", action, KNOWLEDGE_MAX_ACTION_CHARS)?;
        }
        if step.safety_notes.len() > KNOWLEDGE_MAX_SAFETY_NOTES_PER_STEP {
            return Err(BatchValidationError::schema_violation(format!(
                "步骤 {} 的 safetyNotes 条数 {} 超过上限 {KNOWLEDGE_MAX_SAFETY_NOTES_PER_STEP}",
                step.id,
                step.safety_notes.len()
            )));
        }
        for note in &step.safety_notes {
            require_text("step.safetyNotes[]", note, KNOWLEDGE_MAX_ACTION_CHARS)?;
        }
        let mut part_ids: Vec<String> = Vec::with_capacity(step.part_ids.len());
        for local in &step.part_ids {
            let Some((_, mapped)) = part_id_map
                .iter()
                .find(|(candidate, _)| *candidate == local)
            else {
                return Err(BatchValidationError::new(
                    CODE_PART_REFERENCE_INVALID,
                    format!(
                        "步骤 {} 引用了本批不存在的部件 {}（不接受跨批或伪造引用）",
                        step.id, local
                    ),
                ));
            };
            if !part_ids.contains(mapped) {
                part_ids.push(mapped.clone());
            }
        }
        steps.push(Step {
            id: content_id("step", &[&step.title, &step.ordered_actions.join("\n")]),
            title: step.title.clone(),
            ordered_actions: step.ordered_actions.clone(),
            part_ids,
            evidence: normalize_evidence(&step.evidence, &format!("步骤 {}", step.id))?,
            safety_notes: step.safety_notes.clone(),
            review_status: ReviewStatus::NeedsReview,
            source_batches: Vec::new(),
        });
    }

    // 规格。
    let mut spec_ids: Vec<&str> = Vec::with_capacity(raw.specs.len());
    let mut specs: Vec<Spec> = Vec::with_capacity(raw.specs.len());
    for spec in &raw.specs {
        require_non_blank_id(&spec.id, "spec.id")?;
        if spec_ids.contains(&spec.id.as_str()) {
            return Err(BatchValidationError::new(
                CODE_DUPLICATE_LOCAL_ID,
                format!("规格局部 id 重复：{}", spec.id),
            ));
        }
        spec_ids.push(spec.id.as_str());
        require_text("spec.label", &spec.label, KNOWLEDGE_MAX_SPEC_LABEL_CHARS)?;
        require_text("spec.value", &spec.value, KNOWLEDGE_MAX_SPEC_VALUE_CHARS)?;
        specs.push(Spec {
            id: content_id("spec", &[&spec.label, &spec.value]),
            label: spec.label.clone(),
            value: spec.value.clone(),
            evidence: normalize_evidence(&spec.evidence, &format!("规格 {}", spec.id))?,
            review_status: ReviewStatus::NeedsReview,
            source_batches: Vec::new(),
        });
    }

    // 不确定项（无 evidence 要求；不需要页引用）。
    let mut uncertainty_ids: Vec<&str> = Vec::with_capacity(raw.uncertainties.len());
    let mut uncertainties: Vec<Uncertainty> = Vec::with_capacity(raw.uncertainties.len());
    for item in &raw.uncertainties {
        require_non_blank_id(&item.id, "uncertainty.id")?;
        if uncertainty_ids.contains(&item.id.as_str()) {
            return Err(BatchValidationError::new(
                CODE_DUPLICATE_LOCAL_ID,
                format!("不确定项局部 id 重复：{}", item.id),
            ));
        }
        uncertainty_ids.push(item.id.as_str());
        require_text(
            "uncertainty.topic",
            &item.topic,
            KNOWLEDGE_MAX_UNCERTAINTY_TOPIC_CHARS,
        )?;
        require_text(
            "uncertainty.detail",
            &item.detail,
            KNOWLEDGE_MAX_UNCERTAINTY_DETAIL_CHARS,
        )?;
        uncertainties.push(Uncertainty {
            id: content_id("uncertainty", &[&item.topic, &item.detail]),
            topic: item.topic.clone(),
            detail: item.detail.clone(),
            page_numbers: Vec::new(),
            source_batches: Vec::new(),
        });
    }

    Ok(StructuredBatchResult {
        parts,
        steps,
        specs,
        uncertainties,
    })
}

fn join_pages(pages: &[i64]) -> String {
    pages
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn require_non_blank_id(value: &str, what: &str) -> Result<(), BatchValidationError> {
    if value.trim().is_empty() {
        return Err(BatchValidationError::schema_violation(format!(
            "{what} 不能为空"
        )));
    }
    if value.chars().count() > KNOWLEDGE_MAX_ID_CHARS {
        return Err(BatchValidationError::too_long(
            what,
            value.chars().count(),
            KNOWLEDGE_MAX_ID_CHARS,
        ));
    }
    Ok(())
}

fn require_text(what: &str, value: &str, limit: usize) -> Result<(), BatchValidationError> {
    if value.trim().is_empty() {
        return Err(BatchValidationError::schema_violation(format!(
            "{what} 不能为空白（空内容不构成事实）"
        )));
    }
    if value.chars().count() > limit {
        return Err(BatchValidationError::too_long(
            what,
            value.chars().count(),
            limit,
        ));
    }
    Ok(())
}

/// 内容派生 id：`<kind>-<sha256(kind + 内容)[..12]>`。
///
/// 应用分配（contracts.md §2）且稳定：同一内容跨批得到同一 id（合并去重的锚点），
/// 同名不同事实得到不同 id（冲突双方都保留）。
fn content_id(kind: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    for part in parts {
        hasher.update(b"\n");
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{kind}-{}", &hex[..12])
}

// ---------------------------------------------------------------------------
// 批次结果（阶段持久化的 JSON 形状；merge 的输入）
// ---------------------------------------------------------------------------

/// 单批的完整持久化结果（`job_stages.result_asset_id` 指向的 JSON）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExtractionResult {
    pub schema_version: String,
    pub prompt_version: String,
    pub batch_index: i64,
    pub pages: Vec<i64>,
    pub outcome: BatchOutcome,
    /// 是否产出正式知识（拒答/截断/格式错/校验失败一律 `false`）。
    pub produced_knowledge: bool,
    pub document_id: String,
    pub preparation_id: String,
    pub parts: Vec<Part>,
    pub steps: Vec<Step>,
    pub specs: Vec<Spec>,
    pub uncertainties: Vec<Uncertainty>,
    /// `needs_input` 时的稳定缺项/错误码（诊断）。
    pub error_code: Option<String>,
    /// 短错误摘要（脱敏；不含原文）。
    pub error_summary: Option<String>,
    /// 同步响应的 opaque id（**不假定可轮询/重取**）。
    pub response_id: Option<String>,
    /// 原始响应诊断 blob 的 sha256（受限诊断路径；非成功路径保留）。
    pub diagnostic_sha256: Option<String>,
}

impl BatchExtractionResult {
    /// 未产出正式知识的批次结果（诊断路径）。
    // 参数多但都是持久化字段（批次身份 + 诊断），保持构造显式、不做 builder。
    #[allow(clippy::too_many_arguments)]
    pub fn not_produced(
        batch_index: i64,
        pages: &[i64],
        outcome: BatchOutcome,
        document_id: &str,
        preparation_id: &str,
        schema_version: &str,
        prompt_version: &str,
        error_code: &str,
        error_summary: String,
        response_id: Option<String>,
        diagnostic_sha256: Option<String>,
    ) -> Self {
        debug_assert!(!outcome.produced_knowledge());
        Self {
            schema_version: schema_version.to_owned(),
            prompt_version: prompt_version.to_owned(),
            batch_index,
            pages: pages.to_vec(),
            outcome,
            produced_knowledge: false,
            document_id: document_id.to_owned(),
            preparation_id: preparation_id.to_owned(),
            parts: Vec::new(),
            steps: Vec::new(),
            specs: Vec::new(),
            uncertainties: Vec::new(),
            error_code: Some(error_code.to_owned()),
            error_summary: Some(error_summary),
            response_id,
            diagnostic_sha256,
        }
    }

    /// 由校验通过的结构化结果构造（`outcome = completed`）。
    #[allow(clippy::too_many_arguments)]
    pub fn completed(
        batch_index: i64,
        pages: &[i64],
        document_id: &str,
        preparation_id: &str,
        schema_version: &str,
        prompt_version: &str,
        result: StructuredBatchResult,
        response_id: Option<String>,
    ) -> Self {
        Self {
            schema_version: schema_version.to_owned(),
            prompt_version: prompt_version.to_owned(),
            batch_index,
            pages: pages.to_vec(),
            outcome: BatchOutcome::Completed,
            produced_knowledge: true,
            document_id: document_id.to_owned(),
            preparation_id: preparation_id.to_owned(),
            parts: result.parts,
            steps: result.steps,
            specs: result.specs,
            uncertainties: result.uncertainties,
            error_code: None,
            error_summary: None,
            response_id,
            diagnostic_sha256: None,
        }
    }
}

// ---------------------------------------------------------------------------
// 本地确定性合并（merge_batches）
// ---------------------------------------------------------------------------

/// 合并失败（`manual_merge` 阶段 → `needs_input`，不无限重试、不调用 AI）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeError {
    pub code: &'static str,
    pub detail: String,
}

impl MergeError {
    pub fn detail_text(&self) -> String {
        format!("{}：{}", self.code, self.detail)
    }
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail_text())
    }
}

impl std::error::Error for MergeError {}

/// 批次覆盖记录（“哪些页被哪批覆盖”）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchCoverage {
    pub batch_index: i64,
    pub pages: Vec<i64>,
    pub part_count: usize,
    pub step_count: usize,
    pub spec_count: usize,
    pub uncertainty_count: usize,
}

/// 页覆盖率（全部批次成功且覆盖完整才解锁 merge；此处是**验证结果**）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeCoverage {
    pub complete: bool,
    pub page_count: i64,
    pub pages: Vec<i64>,
    pub batches: Vec<BatchCoverage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntityKind {
    Part,
    Step,
    Spec,
}

/// 冲突的一方（同名不同事实：双方都保留，不丢出处）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictVariant {
    pub entity_id: String,
    pub summary: String,
    pub evidence: Vec<Evidence>,
}

/// 同名不同事实的冲突（保留双方为 `needs_review`，由人工裁决）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityConflict {
    pub entity_kind: EntityKind,
    pub key: String,
    pub review_status: ReviewStatus,
    pub variants: Vec<ConflictVariant>,
}

/// 合并后的知识（`manual_merge` 阶段的持久化结果；T15 组装草稿的输入）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedKnowledge {
    pub schema_version: String,
    pub prompt_version: String,
    pub page_from: i64,
    pub page_to: i64,
    pub coverage: KnowledgeCoverage,
    pub parts: Vec<Part>,
    pub steps: Vec<Step>,
    pub specs: Vec<Spec>,
    pub uncertainties: Vec<Uncertainty>,
    pub conflicts: Vec<EntityConflict>,
}

/// 本地确定性合并（contracts.md §6：去重但保留原始出处；同名不同事实保留冲突）。
///
/// 前置校验（任一不满足 → [`MergeError`]，不产出结果、不调用 AI）：
/// - 计划页非空；每个批次的页都在计划页内；计划页被批次**全覆盖**（漏批检测）；
///   批次间的页不重叠（重叠意味着计划被破坏，需人工核对）；
/// - 每个批次都 `produced_knowledge`（防御：DAG 只解锁全部成功的批次）。
///
/// 确定性：输入顺序无关（按 `batch_index` 排序处理），实体 id 由内容派生，
/// 出处按（页号，引文）去重后稳定排序；同一输入总是得到同一结果字节。
pub fn merge_batches(
    batches: &[BatchExtractionResult],
    expected_pages: &[i64],
    prompt_version: &str,
) -> Result<MergedKnowledge, MergeError> {
    if expected_pages.is_empty() {
        return Err(MergeError {
            code: CODE_COVERAGE_INCOMPLETE,
            detail: "计划页集合为空：没有可合并的输入".to_owned(),
        });
    }
    if batches.is_empty() {
        return Err(MergeError {
            code: CODE_COVERAGE_INCOMPLETE,
            detail: "没有批次结果：不产出合并知识".to_owned(),
        });
    }

    let mut sorted: Vec<&BatchExtractionResult> = batches.iter().collect();
    sorted.sort_by_key(|batch| batch.batch_index);

    // 覆盖校验。
    let mut covered: Vec<i64> = Vec::new();
    let mut coverage_entries: Vec<BatchCoverage> = Vec::with_capacity(sorted.len());
    for batch in &sorted {
        if !batch.produced_knowledge {
            return Err(MergeError {
                code: CODE_BATCH_WITHOUT_KNOWLEDGE,
                detail: format!(
                    "批次 {} 未产出正式知识（{}）：不合并、不猜测内容",
                    batch.batch_index,
                    batch.outcome.as_str()
                ),
            });
        }
        if batch.prompt_version != prompt_version {
            return Err(MergeError {
                code: CODE_PROMPT_VERSION_MISMATCH,
                detail: format!(
                    "批次 {} 的 promptVersion={} 与本次快照 {} 不一致：拒绝合并",
                    batch.batch_index, batch.prompt_version, prompt_version
                ),
            });
        }
        for page in &batch.pages {
            if !expected_pages.contains(page) {
                return Err(MergeError {
                    code: CODE_COVERAGE_INCOMPLETE,
                    detail: format!(
                        "批次 {} 的页 {} 不在计划页集合内（计划页：{}）",
                        batch.batch_index,
                        page,
                        join_pages(expected_pages)
                    ),
                });
            }
            if covered.contains(page) {
                return Err(MergeError {
                    code: CODE_COVERAGE_INCOMPLETE,
                    detail: format!(
                        "页 {page} 被多个批次覆盖（批次 {}）：页集合被破坏，需人工核对",
                        batch.batch_index
                    ),
                });
            }
            covered.push(*page);
        }
        coverage_entries.push(BatchCoverage {
            batch_index: batch.batch_index,
            pages: batch.pages.clone(),
            part_count: batch.parts.len(),
            step_count: batch.steps.len(),
            spec_count: batch.specs.len(),
            uncertainty_count: batch.uncertainties.len(),
        });
    }
    let missing: Vec<i64> = expected_pages
        .iter()
        .copied()
        .filter(|page| !covered.contains(page))
        .collect();
    if !missing.is_empty() {
        return Err(MergeError {
            code: CODE_COVERAGE_INCOMPLETE,
            detail: format!(
                "页覆盖不完整：缺少 {}（计划页：{}）",
                join_pages(&missing),
                join_pages(expected_pages)
            ),
        });
    }
    let mut coverage_pages = expected_pages.to_vec();
    coverage_pages.sort_unstable();

    // 批次号写入实体的 source_batches（最后统一排序）。
    let stamp = |entity_batches: &mut Vec<i64>, batch_index: i64| {
        if !entity_batches.contains(&batch_index) {
            entity_batches.push(batch_index);
        }
    };

    let mut parts: Vec<Part> = Vec::new();
    let mut steps: Vec<Step> = Vec::new();
    let mut specs: Vec<Spec> = Vec::new();
    let mut uncertainties: Vec<Uncertainty> = Vec::new();

    for batch in &sorted {
        for part in &batch.parts {
            let mut entry = Part {
                id: part.id.clone(),
                name: part.name.clone(),
                description: part.description.clone(),
                evidence: part.evidence.clone(),
                review_status: ReviewStatus::NeedsReview,
                source_batches: Vec::new(),
            };
            stamp(&mut entry.source_batches, batch.batch_index);
            match parts.iter_mut().find(|existing| existing.id == entry.id) {
                Some(existing) => {
                    merge_evidence(&mut existing.evidence, entry.evidence);
                    for index in entry.source_batches {
                        stamp(&mut existing.source_batches, index);
                    }
                }
                None => parts.push(entry),
            }
        }
        for step in &batch.steps {
            let mut entry = Step {
                id: step.id.clone(),
                title: step.title.clone(),
                ordered_actions: step.ordered_actions.clone(),
                part_ids: step.part_ids.clone(),
                evidence: step.evidence.clone(),
                safety_notes: step.safety_notes.clone(),
                review_status: ReviewStatus::NeedsReview,
                source_batches: Vec::new(),
            };
            stamp(&mut entry.source_batches, batch.batch_index);
            match steps.iter_mut().find(|existing| existing.id == entry.id) {
                Some(existing) => {
                    merge_evidence(&mut existing.evidence, entry.evidence);
                    for index in entry.source_batches {
                        stamp(&mut existing.source_batches, index);
                    }
                }
                None => steps.push(entry),
            }
        }
        for spec in &batch.specs {
            let mut entry = Spec {
                id: spec.id.clone(),
                label: spec.label.clone(),
                value: spec.value.clone(),
                evidence: spec.evidence.clone(),
                review_status: ReviewStatus::NeedsReview,
                source_batches: Vec::new(),
            };
            stamp(&mut entry.source_batches, batch.batch_index);
            match specs.iter_mut().find(|existing| existing.id == entry.id) {
                Some(existing) => {
                    merge_evidence(&mut existing.evidence, entry.evidence);
                    for index in entry.source_batches {
                        stamp(&mut existing.source_batches, index);
                    }
                }
                None => specs.push(entry),
            }
        }
        for uncertainty in &batch.uncertainties {
            let mut entry = Uncertainty {
                id: uncertainty.id.clone(),
                topic: uncertainty.topic.clone(),
                detail: uncertainty.detail.clone(),
                page_numbers: Vec::new(),
                source_batches: Vec::new(),
            };
            stamp(&mut entry.source_batches, batch.batch_index);
            match uncertainties
                .iter_mut()
                .find(|existing| existing.id == entry.id)
            {
                Some(existing) => {
                    for index in entry.source_batches {
                        stamp(&mut existing.source_batches, index);
                    }
                }
                None => uncertainties.push(entry),
            }
        }
    }

    // 每项事实的出处必须保留（去重但完整）：确定性排序。
    for part in &mut parts {
        part.evidence = sorted_evidence(&part.evidence);
        part.source_batches.sort_unstable();
    }
    for step in &mut steps {
        step.evidence = sorted_evidence(&step.evidence);
        step.source_batches.sort_unstable();
    }
    for spec in &mut specs {
        spec.evidence = sorted_evidence(&spec.evidence);
        spec.source_batches.sort_unstable();
    }
    // 不确定项关联页 = 相关实体/引文页的并集不可得时保持空；至少记录来源批次。
    for uncertainty in &mut uncertainties {
        uncertainty.source_batches.sort_unstable();
    }

    let conflicts = collect_conflicts(&parts, &steps, &specs);

    Ok(MergedKnowledge {
        schema_version: MANUAL_EXTRACT_SCHEMA_VERSION.to_owned(),
        prompt_version: prompt_version.to_owned(),
        page_from: coverage_pages.first().copied().unwrap_or(1),
        page_to: coverage_pages.last().copied().unwrap_or(0),
        coverage: KnowledgeCoverage {
            complete: true,
            page_count: coverage_pages.len() as i64,
            pages: coverage_pages,
            batches: coverage_entries,
        },
        parts,
        steps,
        specs,
        uncertainties,
        conflicts,
    })
}

/// 出处并集（按（页号，引文）判重；保留不同批次的相同出处一次）。
fn merge_evidence(existing: &mut Vec<Evidence>, incoming: Vec<Evidence>) {
    for item in incoming {
        if !existing
            .iter()
            .any(|current| current.page_number == item.page_number && current.quote == item.quote)
        {
            existing.push(item);
        }
    }
}

/// 确定性排序：页号升序、同页按引文、再按 derived。
fn sorted_evidence(evidence: &[Evidence]) -> Vec<Evidence> {
    let mut sorted = evidence.to_vec();
    sorted.sort_by(|left, right| {
        left.page_number
            .cmp(&right.page_number)
            .then_with(|| left.quote.cmp(&right.quote))
            .then_with(|| left.derived.cmp(&right.derived))
    });
    sorted
}

/// 同名不同事实 → 冲突（双方保留；不自动选一个当"对的"）。
fn collect_conflicts(parts: &[Part], steps: &[Step], specs: &[Spec]) -> Vec<EntityConflict> {
    let mut conflicts: Vec<EntityConflict> = Vec::new();

    let mut part_names: Vec<&str> = parts.iter().map(|part| part.name.as_str()).collect();
    part_names.sort_unstable();
    part_names.dedup();
    for name in part_names {
        let variants: Vec<&Part> = parts.iter().filter(|part| part.name == name).collect();
        let descriptions: std::collections::BTreeSet<&str> = variants
            .iter()
            .map(|part| part.description.as_str())
            .collect();
        if descriptions.len() > 1 {
            conflicts.push(EntityConflict {
                entity_kind: EntityKind::Part,
                key: name.to_owned(),
                review_status: ReviewStatus::NeedsReview,
                variants: variants
                    .iter()
                    .map(|part| ConflictVariant {
                        entity_id: part.id.clone(),
                        summary: part.description.clone(),
                        evidence: part.evidence.clone(),
                    })
                    .collect(),
            });
        }
    }

    let mut step_titles: Vec<&str> = steps.iter().map(|step| step.title.as_str()).collect();
    step_titles.sort_unstable();
    step_titles.dedup();
    for title in step_titles {
        let variants: Vec<&Step> = steps.iter().filter(|step| step.title == title).collect();
        let actions: std::collections::BTreeSet<String> = variants
            .iter()
            .map(|step| step.ordered_actions.join("\n"))
            .collect();
        if actions.len() > 1 {
            conflicts.push(EntityConflict {
                entity_kind: EntityKind::Step,
                key: title.to_owned(),
                review_status: ReviewStatus::NeedsReview,
                variants: variants
                    .iter()
                    .map(|step| ConflictVariant {
                        entity_id: step.id.clone(),
                        summary: step.ordered_actions.join(" → "),
                        evidence: step.evidence.clone(),
                    })
                    .collect(),
            });
        }
    }

    let mut spec_labels: Vec<&str> = specs.iter().map(|spec| spec.label.as_str()).collect();
    spec_labels.sort_unstable();
    spec_labels.dedup();
    for label in spec_labels {
        let variants: Vec<&Spec> = specs.iter().filter(|spec| spec.label == label).collect();
        let values: std::collections::BTreeSet<&str> =
            variants.iter().map(|spec| spec.value.as_str()).collect();
        if values.len() > 1 {
            conflicts.push(EntityConflict {
                entity_kind: EntityKind::Spec,
                key: label.to_owned(),
                review_status: ReviewStatus::NeedsReview,
                variants: variants
                    .iter()
                    .map(|spec| ConflictVariant {
                        entity_id: spec.id.clone(),
                        summary: spec.value.clone(),
                        evidence: spec.evidence.clone(),
                    })
                    .collect(),
            });
        }
    }

    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn context<'a>(pages: &'a [i64], image_pages: &'a [i64]) -> BatchValidationContext<'a> {
        BatchValidationContext {
            document_id: "doc-1",
            preparation_id: "prep-1",
            pages,
            image_pages,
            schema_version: MANUAL_EXTRACT_SCHEMA_VERSION,
        }
    }

    fn page(n: i64, quote: Option<&str>) -> serde_json::Value {
        json!({ "pageNumber": n, "quote": quote })
    }

    fn valid_output() -> String {
        json!({
            "schemaVersion": "manual_extract_v1",
            "parts": [
                { "id": "p1", "name": "后盖", "description": "机身背部可拆盖板",
                  "evidence": [page(1, Some("Loosen the four screws"))] }
            ],
            "steps": [
                { "id": "s1", "title": "取下后盖",
                  "orderedActions": ["松开四颗螺钉", "取下后盖"],
                  "partIds": ["p1"],
                  "evidence": [page(1, Some("Loosen the four screws"))],
                  "safetyNotes": [] }
            ],
            "specs": [
                { "id": "sp1", "label": "供电", "value": "DC 12V",
                  "evidence": [page(2, None)] }
            ],
            "uncertainties": [
                { "id": "u1", "topic": "扭矩", "detail": "未给出螺钉扭矩" }
            ]
        })
        .to_string()
    }

    #[test]
    fn schema_is_strict_with_all_required_and_nullable_optionals() {
        let schema = manual_extract_json_schema();
        assert_eq!(schema["additionalProperties"], json!(false));
        for (name, property) in schema["properties"].as_object().unwrap() {
            if let Some(object) = property.as_object()
                && object.get("additionalProperties") == Some(&json!(false))
            {
                let required: Vec<&str> = object["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap())
                    .collect();
                let keys: Vec<&str> = object["properties"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect();
                assert_eq!(
                    required.len(),
                    keys.len(),
                    "{name} 的全部属性必须进入 required：{required:?} vs {keys:?}"
                );
                for key in keys {
                    assert!(required.contains(&key), "{name}.{key} 不在 required 中");
                }
            }
        }
        // 可选值用 nullable（不是省略键）。
        let quote = &schema["properties"]["parts"]["items"]["properties"]["evidence"]["items"]["properties"]
            ["quote"];
        assert_eq!(quote["type"], json!(["string", "null"]));
        // 不提供 confidence 字段（模型自报置信度不是已验真概率）。
        let serialized = schema.to_string();
        assert!(!serialized.contains("confidence"), "{serialized}");
    }

    #[test]
    fn valid_output_is_normalized_with_server_side_provenance_and_mapping() {
        let pages = [1, 2];
        let images = [2];
        let result =
            validate_batch_output(&valid_output(), &context(&pages, &images)).expect("校验通过");

        assert_eq!(result.parts.len(), 1);
        let part = &result.parts[0];
        // 局部 id 被映射为应用 id（内容派生）；document/preparation 由服务端回填。
        assert!(part.id.starts_with("part-"), "{}", part.id);
        assert_ne!(part.id, "p1");
        assert_eq!(part.evidence[0].document_id, "doc-1");
        assert_eq!(part.evidence[0].preparation_id, "prep-1");
        assert!(!part.evidence[0].derived, "文字页不是派生");
        assert_eq!(part.review_status, ReviewStatus::NeedsReview);
        // 步骤引用被映射到同一应用 id。
        assert_eq!(result.steps[0].part_ids, vec![part.id.clone()]);
        // 页图页的引文标记 derived。
        assert_eq!(result.specs[0].evidence[0].page_number, 2);
        assert!(result.specs[0].evidence[0].derived);
        assert!(
            result.specs[0].evidence[0].quote.is_none(),
            "可选值可为 null"
        );
    }

    #[test]
    fn malformed_json_is_rejected_without_any_rescue() {
        let pages = [1];
        let truncated = "{\"schemaVersion\":\"manual_extract_v1\",\"parts\":[{\"id\":\"p1\"";
        let error = validate_batch_output(truncated, &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_INVALID_FORMAT, "{error:?}");
        // 明确不做"正则抢救"：没有任何部分结果可用。
        let error = validate_batch_output("not json at all", &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_INVALID_FORMAT);
    }

    #[test]
    fn unknown_fields_and_missing_required_are_schema_violations() {
        let pages = [1];
        let with_extra = json!({
            "schemaVersion": "manual_extract_v1",
            "parts": [ { "id": "p1", "name": "n", "description": "d",
                         "confidence": 0.99,
                         "evidence": [page(1, None)] } ],
            "steps": [], "specs": [], "uncertainties": []
        })
        .to_string();
        let error = validate_batch_output(&with_extra, &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_SCHEMA_VIOLATION);

        let missing = json!({
            "schemaVersion": "manual_extract_v1",
            "parts": [], "steps": [], "specs": []
        })
        .to_string();
        let error = validate_batch_output(&missing, &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_SCHEMA_VIOLATION);

        let wrong_version = valid_output().replace("manual_extract_v1", "manual_extract_v9");
        let error = validate_batch_output(&wrong_version, &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_SCHEMA_VIOLATION);
    }

    #[test]
    fn length_limits_and_entity_limits_are_enforced() {
        let pages = [1];
        // 超长引文。
        let long_quote = "x".repeat(KNOWLEDGE_MAX_QUOTE_CHARS + 1);
        let mut value: serde_json::Value = serde_json::from_str(&valid_output()).unwrap();
        value["parts"][0]["evidence"][0]["quote"] = json!(long_quote);
        let error = validate_batch_output(&value.to_string(), &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_STRING_TOO_LONG);

        // 超长名称。
        let long_name = "n".repeat(KNOWLEDGE_MAX_NAME_CHARS + 1);
        value["parts"][0]["evidence"][0]["quote"] = json!(null);
        value["parts"][0]["name"] = json!(long_name);
        let error = validate_batch_output(&value.to_string(), &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_STRING_TOO_LONG);

        // 实体总数超限。
        let parts: Vec<serde_json::Value> = (0..KNOWLEDGE_MAX_PARTS + 1)
            .map(|index| {
                json!({ "id": format!("p{index}"), "name": format!("n{index}"),
                        "description": "d", "evidence": [page(1, None)] })
            })
            .collect();
        let too_many = json!({
            "schemaVersion": "manual_extract_v1",
            "parts": parts, "steps": [], "specs": [], "uncertainties": []
        })
        .to_string();
        let error = validate_batch_output(&too_many, &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_ENTITY_LIMIT);
    }

    #[test]
    fn pages_outside_the_batch_input_are_rejected() {
        let pages = [3, 4];
        let mut value: serde_json::Value = serde_json::from_str(&valid_output()).unwrap();
        value["parts"][0]["evidence"][0]["pageNumber"] = json!(5);
        let error = validate_batch_output(&value.to_string(), &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_PAGE_REFERENCE_INVALID);

        // 0 页（0-based 误用）同样被拒。
        value["parts"][0]["evidence"][0]["pageNumber"] = json!(0);
        let error = validate_batch_output(&value.to_string(), &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_PAGE_REFERENCE_INVALID);
    }

    #[test]
    fn part_references_must_exist_in_the_same_batch() {
        let pages = [1];
        let mut value: serde_json::Value = serde_json::from_str(&valid_output()).unwrap();
        value["steps"][0]["partIds"] = json!(["p1", "p-does-not-exist"]);
        let error = validate_batch_output(&value.to_string(), &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_PART_REFERENCE_INVALID);

        // 重复局部 id 也被拒（映射会有歧义）。
        value["steps"][0]["partIds"] = json!([]);
        value["parts"] = json!([
            { "id": "p1", "name": "a", "description": "d", "evidence": [page(1, None)] },
            { "id": "p1", "name": "b", "description": "d2", "evidence": [page(1, None)] }
        ]);
        let error = validate_batch_output(&value.to_string(), &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_DUPLICATE_LOCAL_ID);
    }

    #[test]
    fn evidence_is_required_for_entities_with_pages() {
        let pages = [1];
        let mut value: serde_json::Value = serde_json::from_str(&valid_output()).unwrap();
        value["parts"][0]["evidence"] = json!([]);
        let error = validate_batch_output(&value.to_string(), &context(&pages, &[])).unwrap_err();
        assert_eq!(error.code, CODE_SCHEMA_VIOLATION);
    }

    // -----------------------------------------------------------------------
    // 合并
    // -----------------------------------------------------------------------

    fn batch(
        index: i64,
        pages: &[i64],
        parts: Vec<Part>,
        steps: Vec<Step>,
        specs: Vec<Spec>,
        uncertainties: Vec<Uncertainty>,
    ) -> BatchExtractionResult {
        BatchExtractionResult {
            schema_version: MANUAL_EXTRACT_SCHEMA_VERSION.to_owned(),
            prompt_version: MANUAL_EXTRACT_SCHEMA_VERSION.to_owned(),
            batch_index: index,
            pages: pages.to_vec(),
            outcome: BatchOutcome::Completed,
            produced_knowledge: true,
            document_id: "doc-1".to_owned(),
            preparation_id: "prep-1".to_owned(),
            parts,
            steps,
            specs,
            uncertainties,
            error_code: None,
            error_summary: None,
            response_id: None,
            diagnostic_sha256: None,
        }
    }

    fn evidence(page_number: i64, quote: &str) -> Evidence {
        Evidence {
            document_id: "doc-1".to_owned(),
            preparation_id: "prep-1".to_owned(),
            page_number,
            quote: Some(quote.to_owned()),
            bbox: None,
            derived: false,
        }
    }

    fn part(name: &str, description: &str, page_number: i64, quote: &str) -> Part {
        Part {
            id: content_id("part", &[name, description]),
            name: name.to_owned(),
            description: description.to_owned(),
            evidence: vec![evidence(page_number, quote)],
            review_status: ReviewStatus::NeedsReview,
            source_batches: Vec::new(),
        }
    }

    #[test]
    fn merge_deduplicates_and_keeps_all_provenance() {
        let first = batch(
            0,
            &[1, 2],
            vec![part("后盖", "可拆盖板", 1, "quote-1")],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let second = batch(
            1,
            &[3],
            vec![part("后盖", "可拆盖板", 3, "quote-3")],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let merged = merge_batches(
            &[second.clone(), first.clone()],
            &[1, 2, 3],
            "manual_extract_v1",
        )
        .expect("合并成功");

        assert_eq!(merged.parts.len(), 1, "同内容去重为一条");
        let merged_part = &merged.parts[0];
        assert_eq!(merged_part.source_batches, vec![0, 1]);
        assert_eq!(merged_part.evidence.len(), 2, "原始出处全部保留");
        assert_eq!(merged_part.evidence[0].page_number, 1);
        assert_eq!(merged_part.evidence[1].page_number, 3);
        assert!(merged.conflicts.is_empty());
        assert!(merged.coverage.complete);
        assert_eq!(merged.coverage.page_count, 3);
        assert_eq!(merged.coverage.batches.len(), 2);
        assert_eq!(merged.coverage.batches[0].batch_index, 0);
        assert_eq!(merged.coverage.batches[0].pages, vec![1, 2]);
        assert_eq!(merged.page_from, 1);
        assert_eq!(merged.page_to, 3);

        // 确定性：输入顺序颠倒得到同一结果字节。
        let merged_again =
            merge_batches(&[first, second], &[1, 2, 3], "manual_extract_v1").expect("合并成功");
        assert_eq!(
            serde_json::to_string(&merged).unwrap(),
            serde_json::to_string(&merged_again).unwrap(),
            "同一输入必须得到同一结果字节"
        );
    }

    #[test]
    fn same_name_with_different_facts_keeps_both_as_conflict() {
        let first = batch(
            0,
            &[1],
            vec![part("后盖", "四颗螺钉固定", 1, "quote-1")],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let second = batch(
            1,
            &[2],
            vec![part("后盖", "卡扣固定", 2, "quote-2")],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let merged =
            merge_batches(&[first, second], &[1, 2], "manual_extract_v1").expect("合并成功");

        assert_eq!(merged.parts.len(), 2, "同名不同事实双方都保留（不丢出处）");
        assert_eq!(merged.conflicts.len(), 1);
        let conflict = &merged.conflicts[0];
        assert_eq!(conflict.entity_kind, EntityKind::Part);
        assert_eq!(conflict.key, "后盖");
        assert_eq!(conflict.review_status, ReviewStatus::NeedsReview);
        assert_eq!(conflict.variants.len(), 2);
        assert_eq!(conflict.variants[0].evidence.len(), 1);
        assert!(
            merged
                .parts
                .iter()
                .all(|part| part.review_status == ReviewStatus::NeedsReview)
        );
    }

    #[test]
    fn merge_rejects_missing_coverage_batches_without_knowledge_and_page_overlap() {
        let first = batch(0, &[1, 2], Vec::new(), Vec::new(), Vec::new(), Vec::new());
        // 漏批：计划 3 页、只覆盖 1..2。
        let error = merge_batches(
            std::slice::from_ref(&first),
            &[1, 2, 3],
            "manual_extract_v1",
        )
        .unwrap_err();
        assert_eq!(error.code, CODE_COVERAGE_INCOMPLETE);
        assert!(error.detail.contains("3"), "{}", error.detail);

        // 批次未产出正式知识。
        let mut refused = batch(1, &[3], Vec::new(), Vec::new(), Vec::new(), Vec::new());
        refused.produced_knowledge = false;
        refused.outcome = BatchOutcome::Refusal;
        let error =
            merge_batches(&[first.clone(), refused], &[1, 2, 3], "manual_extract_v1").unwrap_err();
        assert_eq!(error.code, CODE_BATCH_WITHOUT_KNOWLEDGE);

        // 页重叠 / 计划外页。
        let overlapping = batch(1, &[2, 3], Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let error =
            merge_batches(&[first, overlapping], &[1, 2, 3], "manual_extract_v1").unwrap_err();
        assert_eq!(error.code, CODE_COVERAGE_INCOMPLETE);
    }

    #[test]
    fn batch_outcomes_only_completed_produces_knowledge() {
        assert!(BatchOutcome::Completed.produced_knowledge());
        for outcome in [
            BatchOutcome::Refusal,
            BatchOutcome::Incomplete,
            BatchOutcome::InvalidFormat,
            BatchOutcome::EmptyOutput,
            BatchOutcome::SchemaViolation,
            BatchOutcome::EnvelopeInvalid,
            BatchOutcome::ResponseFailed,
            BatchOutcome::ProviderRejected,
        ] {
            assert!(
                !outcome.produced_knowledge(),
                "{outcome:?} 不得产出正式知识"
            );
        }
    }
}
