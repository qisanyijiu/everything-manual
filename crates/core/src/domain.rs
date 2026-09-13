//! 领域数据类型：与 contracts.md §2「数据对象与数据库约束」的字段一一对应。
//!
//! 约定：
//! - Rust／SQL 字段名 snake_case，JSON 线上 camelCase（contracts.md §1）；
//! - 时间字段为 [`Timestamp`]（存储为 Unix 毫秒，线上为 RFC3339）；
//! - 枚举在 SQL 中存 snake_case 字面量（[`JobStatus::as_str`] 等），JSON 为 camelCase；
//! - 金额一律整数最小单位（creditMinor = 1/100 credit、usdMicros = 1/1e6 USD）；
//! - 聚合 JSON（知识、报价快照等）在 T03 用 [`serde_json::Value`] 承载，
//!   其内部结构的校验与演进属 T11/T15/T19；存储层用 `json_valid` 兜底。
//!
//! 本模块只定义数据类型与最小不变量检查，**不含业务规则**（如名称必填校验、
//! 报价计算、发布校验）；业务规则由服务层在后续卡实现。

use serde::{Deserialize, Serialize};

use crate::timestamps::Timestamp;

// ---------------------------------------------------------------------------
// 枚举（SQL 值 snake_case；JSON 值 camelCase，除非另有说明）
// ---------------------------------------------------------------------------

/// 照片视图（contracts.md §2；方向以物品自身为参照，PRD A-04）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhotoView {
    Front,
    Left,
    Back,
    Right,
    Detail,
}

impl PhotoView {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Left => "left",
            Self::Back => "back",
            Self::Right => "right",
            Self::Detail => "detail",
        }
    }

    /// 可进入 Tripo 多视图请求的视图（detail 只用于理解与核对，contracts.md §2）。
    pub const fn is_multiview(self) -> bool {
        !matches!(self, Self::Detail)
    }

    /// 解析线上取值；未知值返回 `None`（调用方映射为字段级 422，而不是静默兜底）。
    ///
    /// 线上取值固定小写：[`Self::as_str`]（serde 的 `rename_all = "lowercase"` 同源）。
    pub fn from_wire(value: &str) -> Option<Self> {
        match value.trim() {
            "front" => Some(Self::Front),
            "left" => Some(Self::Left),
            "back" => Some(Self::Back),
            "right" => Some(Self::Right),
            "detail" => Some(Self::Detail),
            _ => None,
        }
    }

    /// 全部合法取值（错误文案与 OpenAPI 描述共用）。
    pub const ALL: [Self; 5] = [
        Self::Front,
        Self::Left,
        Self::Back,
        Self::Right,
        Self::Detail,
    ];
}

/// 资产用途（contracts.md §3 路由 + 模型资产，见 migrations/0001）。
///
/// 线上值为 camelCase（`pageImage` / `pageText`），SQL 值为 snake_case。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetPurpose {
    Document,
    Photo,
    PageImage,
    PageText,
    Model,
    /// 发布冻结的 manifest 资产（T19 / REQ-035）：不可变、内容寻址、归属物品。
    ReleaseManifest,
}

impl AssetPurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Photo => "photo",
            Self::PageImage => "page_image",
            Self::PageText => "page_text",
            Self::Model => "model",
            Self::ReleaseManifest => "release_manifest",
        }
    }
}

/// blob 的存储状态（内容寻址；文件字节在 `<data-dir>/blobs/...`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlobStorageState {
    /// 文件已落盘且元数据已提交。
    Stored,
    /// 完整性检查失败，已隔离（保留现场，不静默删除）。
    Quarantined,
    /// 元数据存在但文件缺失（需要恢复或重新上传）。
    Missing,
}

impl BlobStorageState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stored => "stored",
            Self::Quarantined => "quarantined",
            Self::Missing => "missing",
        }
    }
}

/// preparation 状态（contracts.md §2）：ready 后不可修改。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreparationState {
    Preparing,
    Ready,
}

impl PreparationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Ready => "ready",
        }
    }
}

/// job / stage 运行时状态（contracts.md §5）。
///
/// 与工作流 state.yaml 的 `pm_ready` 等枚举无关，不共用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobStatus {
    Queued,
    Running,
    WaitingProvider,
    RetryWait,
    NeedsInput,
    SubmissionUnknown,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingProvider => "waiting_provider",
            Self::RetryWait => "retry_wait",
            Self::NeedsInput => "needs_input",
            Self::SubmissionUnknown => "submission_unknown",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// 终态（不再自动推进）；`submission_unknown` / `needs_input` 不是终态，
    /// 需要管理员对账或补齐输入。
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    /// 解析 SQL 中的 snake_case 取值；未知值返回 `None`（调用方按损坏数据处理，
    /// 不静默兜底为某个状态）。
    pub fn from_sql(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "waiting_provider" => Some(Self::WaitingProvider),
            "retry_wait" => Some(Self::RetryWait),
            "needs_input" => Some(Self::NeedsInput),
            "submission_unknown" => Some(Self::SubmissionUnknown),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// 阶段类型（contracts.md §5 的 DAG）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StageKind {
    FreezeInputs,
    ManualExtract,
    ManualMerge,
    TripoUpload,
    TripoSubmit,
    TripoPoll,
    ModelDownload,
    ModelValidate,
    AssembleDraft,
}

impl StageKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FreezeInputs => "freeze_inputs",
            Self::ManualExtract => "manual_extract",
            Self::ManualMerge => "manual_merge",
            Self::TripoUpload => "tripo_upload",
            Self::TripoSubmit => "tripo_submit",
            Self::TripoPoll => "tripo_poll",
            Self::ModelDownload => "model_download",
            Self::ModelValidate => "model_validate",
            Self::AssembleDraft => "assemble_draft",
        }
    }

    /// 是否为按批次展开的阶段（`manual_extract` 的逻辑批，contracts.md §5）。
    /// 非批处理阶段的 `batch_index` 固定为 0。
    pub const fn is_batched(self) -> bool {
        matches!(self, Self::ManualExtract)
    }

    /// 解析 SQL 中的 snake_case 取值；未知值返回 `None`。
    pub fn from_sql(value: &str) -> Option<Self> {
        match value {
            "freeze_inputs" => Some(Self::FreezeInputs),
            "manual_extract" => Some(Self::ManualExtract),
            "manual_merge" => Some(Self::ManualMerge),
            "tripo_upload" => Some(Self::TripoUpload),
            "tripo_submit" => Some(Self::TripoSubmit),
            "tripo_poll" => Some(Self::TripoPoll),
            "model_download" => Some(Self::ModelDownload),
            "model_validate" => Some(Self::ModelValidate),
            "assemble_draft" => Some(Self::AssembleDraft),
            _ => None,
        }
    }

    /// 全部阶段类型（DAG 遍历与集合断言用）。
    pub const ALL: [Self; 9] = [
        Self::FreezeInputs,
        Self::ManualExtract,
        Self::ManualMerge,
        Self::TripoUpload,
        Self::TripoSubmit,
        Self::TripoPoll,
        Self::ModelDownload,
        Self::ModelValidate,
        Self::AssembleDraft,
    ];
}

/// 付费提交 attempt 的提交状态（contracts.md §2/§5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubmitState {
    /// 已持久化 intent，尚未发请求（崩溃后可安全领取）。
    Intent,
    /// 已标记将发请求但结果未知（submitting 且无远端 ID = 必须对账）。
    Submitting,
    /// 已获得远端事实（远端 task id 或已保存的同步响应）。
    Accepted,
    /// 结果未知，禁止自动重购，等待管理员对账。
    Unknown,
    /// 明确失败（可证明未被接受才允许自动重试）。
    Failed,
}

impl SubmitState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Submitting => "submitting",
            Self::Accepted => "accepted",
            Self::Unknown => "unknown",
            Self::Failed => "failed",
        }
    }

    /// 是否属于"未对账"（同一阶段只允许一个未对账 attempt，contracts.md §5）。
    pub const fn is_unresolved(self) -> bool {
        matches!(self, Self::Intent | Self::Submitting | Self::Unknown)
    }
}

/// 供应商维度（费用分列；SQL 值与配置键同为 `tripo` / `manual_ai`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKey {
    Tripo,
    ManualAi,
}

impl ProviderKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tripo => "tripo",
            Self::ManualAi => "manual_ai",
        }
    }
}

/// 金额币种／单位（contracts.md §1：不使用浮点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Currency {
    /// Tripo credit 的 1/100（`creditMinor`）。
    CreditMinor,
    /// USD 的 1/1_000_000（`usdMicros`）。
    UsdMicros,
}

impl Currency {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreditMinor => "credit_minor",
            Self::UsdMicros => "usd_micros",
        }
    }
}

/// 费用账本状态（contracts.md §4：预留／结算／释放在事务内且幂等；
/// unknown 保留预留且不得把实际费用填 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LedgerState {
    Reserved,
    Settled,
    Released,
    Unknown,
}

impl LedgerState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Settled => "settled",
            Self::Released => "released",
            Self::Unknown => "unknown",
        }
    }
}

/// 模型版本校验状态：只有 `Validated` 才可进入阅读器（contracts.md §2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelValidationState {
    Pending,
    Validated,
    Rejected,
}

impl ModelValidationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Validated => "validated",
            Self::Rejected => "rejected",
        }
    }
}

/// 草稿状态：生成完成不等于已发布（REQ-030/ADR-005）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatus {
    NeedsReview,
    Ready,
}

impl DraftStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NeedsReview => "needs_review",
            Self::Ready => "ready",
        }
    }
}

// ---------------------------------------------------------------------------
// 实体
// ---------------------------------------------------------------------------

/// 管理员（单管理员；无默认密码，只存 Argon2 哈希）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Admin {
    pub id: String,
    pub password_hash: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// 会话。明文 token 只返回 cookie，不落库／日志；库中只有哈希。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub admin_id: String,
    pub session_token_hash: String,
    pub csrf_hash: String,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    pub revoked_at: Option<Timestamp>,
}

/// 物品（可编辑聚合根：更新使用整数 `revision` 做 CAS）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub name: String,
    pub brand: Option<String>,
    pub model: String,
    pub variant: Option<String>,
    pub revision: i64,
    /// 归档时间；归档代替物理删除（被引用资产不删除）。
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Item {
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }

    /// `ETag` 值（contracts.md §1：可编辑聚合根 GET 返回 `ETag: "r7"`）。
    pub fn etag(&self) -> String {
        format!("\"r{}\"", self.revision)
    }
}

/// 内容寻址的 blob 元数据（`sha256` 唯一内容存储）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blob {
    pub sha256: String,
    pub size: i64,
    pub mime: String,
    pub storage_state: BlobStorageState,
    pub created_at: Timestamp,
}

/// 资产：引用 blob（去重）并归属到物品。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub blob_id: String,
    pub item_id: String,
    pub purpose: AssetPurpose,
    /// 原文件名只作元数据，不参与路径拼接。
    pub original_name: Option<String>,
    pub created_at: Timestamp,
}

/// 绑定到物品的说明书原件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: String,
    pub item_id: String,
    pub source_asset_id: String,
    pub source_sha256: String,
    pub title: String,
    /// 出处链接；服务端不据此发起抓取。
    pub source_url: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// PDF 页准备（浏览器侧生成，服务端封存）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preparation {
    pub id: String,
    pub document_id: String,
    pub source_sha256: String,
    pub state: PreparationState,
    pub page_count: Option<i64>,
    /// `true` = 页资产由浏览器（PDF.js）派生后上传，不是服务端渲染结果
    /// （contracts.md §2、REQ-015：哈希只证明字节一致，不证明页图来自原 PDF）。
    pub client_derived: bool,
    pub revision: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// 页图 viewport（旋转后的页图尺寸与旋转角）。
///
/// 坐标约定（架构 §5.1、PRD §5.3）：页图坐标原点为该 viewport 的**左上角**；
/// 后续 bbox 归一化（contracts.md §2）以本结构记录的尺寸/旋转为参照，
/// 不用不正确的 transform 伪造字符框。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageViewport {
    /// 页图宽度（像素，渲染时 viewport 已含页面旋转）。
    pub width: u32,
    /// 页图高度（像素）。
    pub height: u32,
    /// 页面旋转角（度，0/90/180/270；PDF.js `viewport.rotation`）。
    pub rotation: u16,
}

impl PageViewport {
    /// 长边像素（页图预算按长边判定，架构 §5.1：长边 ≤2000px）。
    pub fn long_edge(&self) -> u32 {
        self.width.max(self.height)
    }

    /// 旋转角是否为 PDF 规范允许的 90 的倍数。
    pub fn rotation_is_valid(&self) -> bool {
        self.rotation.is_multiple_of(90) && self.rotation <= 270
    }
}

/// 单页资料；`page_number` 1-based 且每 preparation 唯一。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub preparation_id: String,
    pub page_number: i64,
    pub text_asset_id: Option<String>,
    pub image_asset_id: Option<String>,
    /// 页图坐标参照；旧记录（或未渲染的页）为 `None`。
    pub viewport: Option<PageViewport>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// 多视图照片；`detail` 不进入 Tripo 多视图请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    pub id: String,
    pub item_id: String,
    pub asset_id: String,
    pub view: PhotoView,
    pub revision: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// 冻结的生成输入（不可变；不保存 API key）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationSnapshot {
    pub id: String,
    pub item_id: String,
    pub item_revision: i64,
    pub preparation_id: String,
    /// JSON 数组：照片 ID（与 `photo_hashes` 对齐）。
    pub photo_ids: serde_json::Value,
    /// JSON 数组：对应 blob 的 sha256。
    pub photo_hashes: serde_json::Value,
    /// JSON：非密钥的供应商配置快照（模型名、质量参数等）。
    pub provider_config: serde_json::Value,
    pub prompt_version: String,
    pub price_version: String,
    /// JSON：预算与保守上界（分列 credits / USD）。
    pub budgets: serde_json::Value,
    pub created_at: Timestamp,
}

/// 生成任务（父）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub item_id: String,
    pub snapshot_id: String,
    pub status: JobStatus,
    pub revision: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// 持久执行单元（阶段）。每 `job + stage_kind + batch_index` 唯一。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStage {
    pub id: String,
    pub job_id: String,
    pub stage_kind: StageKind,
    pub batch_index: i64,
    /// JSON 数组：本批页号（1-based）；非批处理阶段为 null。
    pub page_set: Option<Vec<i64>>,
    pub input_hash: String,
    pub result_asset_id: Option<String>,
    /// JSON：用量／计费摘要（不保存密钥）。
    pub usage_json: Option<serde_json::Value>,
    pub status: JobStatus,
    pub lease_owner: Option<String>,
    pub lease_epoch: i64,
    pub lease_until: Option<Timestamp>,
    pub next_run_at: Option<Timestamp>,
    /// 安全重试已用次数（超 [`crate::jobs::MAX_SAFE_RETRIES`] → failed）。
    pub attempt_count: i64,
    /// 远端轮询次数：驱动 3 秒起逐步到 15 秒的查询节奏
    /// （迁移 0005 追加列，contracts.md §5）。
    pub poll_count: i64,
    /// 最近一次失败／阻塞原因摘要（不保存密钥、完整响应或堆栈）。
    pub last_error: Option<String>,
    /// `needs_input` 的可行动缺项（JSON 数组 `[{code,message}]`）。
    pub needs_input_json: Option<serde_json::Value>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// 付费提交 attempt（先存 intent 再请求；远端 ID 只允许 null→值或同值）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAttempt {
    pub id: String,
    pub job_id: String,
    pub stage_id: String,
    pub request_hash: String,
    pub submit_state: SubmitState,
    /// 供应商 opaque ID：不校验成 UUID、不截断。
    pub remote_task_id: Option<String>,
    /// 同步响应 ID；不等同于可轮询的远端任务。
    pub response_id: Option<String>,
    pub started_at: Timestamp,
    pub last_error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// 幂等记录：`admin + method + route + key` 范围内唯一。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdempotencyRecord {
    pub id: String,
    pub admin_id: String,
    pub method: String,
    pub route: String,
    pub key: String,
    pub body_hash: String,
    pub resource_id: Option<String>,
    pub response_status: Option<i64>,
    pub created_at: Timestamp,
}

/// 费用账本条目（预留／结算／释放幂等；unknown 保留预留）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostLedgerEntry {
    pub id: String,
    pub snapshot_id: String,
    pub attempt_id: Option<String>,
    pub provider: ProviderKey,
    pub currency: Currency,
    /// 预留金额（最小单位，整数）。
    pub reserved: i64,
    /// 实际结算金额；未结算／unknown 时为 `None`（不得填 0 冒充）。
    pub actual: Option<i64>,
    pub state: LedgerState,
    pub price_version: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// 不可变模型版本；`Validated` 才可进入阅读器。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRevision {
    pub id: String,
    pub item_id: String,
    pub asset_id: String,
    pub sha256: String,
    pub provider_attempt_id: Option<String>,
    /// JSON：包围盒等结构摘要。
    pub bounds: Option<serde_json::Value>,
    pub validation_state: ModelValidationState,
    pub created_at: Timestamp,
}

/// 版本化知识草稿（Part/Step/Evidence/Hotspot 聚合为 JSON，revision CAS 更新）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualDraft {
    pub id: String,
    pub item_id: String,
    pub snapshot_id: String,
    pub model_revision_id: Option<String>,
    pub revision: i64,
    pub status: DraftStatus,
    /// JSON：知识聚合（结构由 T15/T19 定义并校验）。
    pub knowledge_json: serde_json::Value,
    /// JSON：复核状态（含 modelReview）；未复核时为 `None`。
    pub review_json: Option<serde_json::Value>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ManualDraft {
    /// `ETag` 值（合同：`ETag: "r7"`）。
    pub fn etag(&self) -> String {
        format!("\"r{}\"", self.revision)
    }
}

/// 不可变发布版本（发布后不引用会变的 draft 内容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualRelease {
    pub id: String,
    pub item_id: String,
    pub draft_id: String,
    pub draft_revision: i64,
    pub model_revision_id: String,
    pub manifest_asset_id: String,
    pub created_at: Timestamp,
}

/// 审计事件（只存必要摘要）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    /// 管理员 id，或 `system`（服务器动作）。
    pub actor: Option<String>,
    pub action: String,
    pub result: String,
    /// JSON：必要摘要（不含密钥、完整资料或签名 URL）。
    pub metadata_json: Option<serde_json::Value>,
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_sql_and_wire_values() {
        assert_eq!(JobStatus::WaitingProvider.as_str(), "waiting_provider");
        assert_eq!(
            serde_json::to_string(&JobStatus::WaitingProvider).unwrap(),
            "\"waitingProvider\""
        );
        assert_eq!(StageKind::ManualExtract.as_str(), "manual_extract");
        assert!(StageKind::ManualExtract.is_batched());
        assert!(!StageKind::TripoSubmit.is_batched());
        assert_eq!(SubmitState::Submitting.as_str(), "submitting");
        assert!(SubmitState::Unknown.is_unresolved());
        assert!(!SubmitState::Accepted.is_unresolved());
        assert_eq!(PhotoView::Detail.as_str(), "detail");
        assert!(!PhotoView::Detail.is_multiview());
        assert_eq!(PhotoView::from_wire("front"), Some(PhotoView::Front));
        assert_eq!(PhotoView::from_wire(" detail "), Some(PhotoView::Detail));
        assert_eq!(PhotoView::from_wire("top"), None);
        assert_eq!(PhotoView::from_wire("Front"), None, "线上取值区分大小写");
        assert_eq!(PhotoView::ALL.len(), 5);
        assert_eq!(AssetPurpose::PageImage.as_str(), "page_image");
        assert_eq!(
            serde_json::to_string(&AssetPurpose::PageText).unwrap(),
            "\"pageText\""
        );
        assert_eq!(Currency::UsdMicros.as_str(), "usd_micros");
        assert_eq!(ProviderKey::ManualAi.as_str(), "manual_ai");
        assert!(JobStatus::Succeeded.is_terminal());
        assert!(!JobStatus::NeedsInput.is_terminal());
    }

    #[test]
    fn item_etag_uses_integer_revision() {
        let item = Item {
            id: "01993000-0000-7000-8000-000000000001".to_owned(),
            name: "示例".to_owned(),
            brand: None,
            model: "X100V".to_owned(),
            variant: None,
            revision: 7,
            archived_at: None,
            created_at: Timestamp::EPOCH,
            updated_at: Timestamp::EPOCH,
        };
        assert_eq!(item.etag(), "\"r7\"");
        assert!(!item.is_archived());
    }
}
