//! Tripo v3 的**线上 DTO**（wire format）：响应信封、上传/提交/查询的数据解析、
//! 计费事实与请求体构造。
//!
//! 设计约束（contracts.md §6「外部 Provider 合同 → Tripo」、T12 卡）：
//! - **同时校验 HTTP 状态与响应体 `code`**：仅 `code == 0` 才读 `data`；非零 `code`
//!   是业务错误，`message`／`suggestion` 只作为**脱敏**错误信息保存；
//! - 供应商 ID／token 一律按 **opaque string** 处理：不校验 UUID、不截断、不去前缀；
//!   数字字面量按 JSON 原样文本接受（不丢精度）；
//! - **容错解析**：未知字段一律忽略并在诊断里记录 `dataKeys`，不用 `deny_unknown_fields`
//!   把供应商新增字段变成解析失败；本模块只对"本卡的协议假设"负责；
//! - **不使用 v2 字段形态**（`model_version`／`files`／`type`）：请求体只由
//!   [`SubmitRequest`] 的字段构成，测试逐字段断言（AC-041）；
//! - 计费：`credits_consumed`（十进制字面量）用**精确 decimal** 换算为 `creditMinor`
//!   （1/100 credit，Ceil），原始字面量与来源字段名一并保留（contracts.md §1）。
//!
//! 官方文档可达性：`developers.tripo3d.ai` 在本机（2026-09-12 实测 curl 20s 超时）
//! 不可达；字段形态依据 `llmdoc/contracts.md` §6 与 architecture §5.3（2026-09-11
//! 由架构阶段核对官方文档后冻结）以及 T05 的构造样例。T23 真实链路必须复核。

use serde::Serialize;
use serde_json::{Map, Value};

use manual_core::cost::{CREDIT_MINOR_SCALE, Rounding, parse_decimal_scaled};

/// 响应信封（`code` 是唯一成功判据；`data` 只在 `code == 0` 时有效）。
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub code: i64,
    pub message: Option<String>,
    pub suggestion: Option<String>,
    /// 原始 `data` 对象（`code == 0` 时由各接口解析）。
    pub data: Option<Value>,
}

impl Envelope {
    /// 业务错误的可读摘要（脱敏：不拼接原始响应、不含密钥）。
    pub fn business_message(&self) -> String {
        let mut text = format!("供应商返回业务错误 code={}", self.code);
        if let Some(message) = self.message.as_deref().filter(|value| !value.is_empty()) {
            text.push_str(&format!("；message={message}"));
        }
        if let Some(suggestion) = self.suggestion.as_deref().filter(|value| !value.is_empty()) {
            text.push_str(&format!("；suggestion={suggestion}"));
        }
        text
    }
}

/// 解析响应信封；失败返回**脱敏**的错误说明（不含响应原文）。
///
/// 失败原因（非 JSON、不是对象、缺 `code`）归入 `Unexpected` 一类：
/// 调用方按"协议出乎本卡假设"处理（付费 POST 一律按结果未知，绝不重发）。
pub fn parse_envelope(bytes: &[u8]) -> Result<Envelope, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("响应不是合法 JSON（{}）：{}", error, describe_bytes(bytes)))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("响应顶层不是对象：{}", describe_bytes(bytes)))?;
    let code = match object.get("code") {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
    .ok_or_else(|| format!("响应缺少可解析的 code 字段：{}", describe_bytes(bytes)))?;
    Ok(Envelope {
        code,
        message: object
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned),
        suggestion: object
            .get("suggestion")
            .and_then(Value::as_str)
            .map(str::to_owned),
        data: object.get("data").cloned(),
    })
}

/// opaque string：字符串原样保留（不做 trim／截断／UUID 校验），数字取 JSON 原样文本。
pub fn opaque_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `data` 对象里按名字取 opaque string。
fn opaque_field(data: &Value, name: &str) -> Option<String> {
    data.as_object()
        .and_then(|object| object.get(name))
        .and_then(opaque_string)
}

/// `data` 对象的字段名清单（诊断用：让"响应形态变了"可被看见，不把值写进日志）。
pub fn data_keys(data: &Value) -> Vec<String> {
    match data.as_object() {
        Some(object) => object.keys().cloned().collect(),
        None => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// POST /files（上传图片）
// ---------------------------------------------------------------------------

/// 上传结果：token + 来源字段名（官方文档字段名以 T23 实测收敛）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadData {
    pub token: String,
    /// `file_token`（historical docs）或 `image_token`（T05 样例）：记录实际来源便于复核。
    pub field: &'static str,
}

/// 上传 token 的候选字段（按优先级）。
pub const UPLOAD_TOKEN_FIELDS: [&str; 2] = ["file_token", "image_token"];

/// 解析上传响应 `data`（`code == 0` 时调用）。
pub fn upload_data(data: Option<&Value>) -> Result<UploadData, String> {
    let Some(data) = data else {
        return Err("响应 code=0 但缺少 data 对象".to_owned());
    };
    for field in UPLOAD_TOKEN_FIELDS {
        if let Some(token) = opaque_field(data, field) {
            return Ok(UploadData { token, field });
        }
    }
    Err(format!(
        "响应缺少上传 token 字段（候选 {}）：dataKeys={:?}",
        UPLOAD_TOKEN_FIELDS.join("/"),
        data_keys(data)
    ))
}

// ---------------------------------------------------------------------------
// POST /generation/multiview-to-model（付费提交）
// ---------------------------------------------------------------------------

/// 多视图生成的请求参数（来自快照 `provider_config.tripo`；**不得**在适配器里
/// 替换为当前配置或默认值——用户确认的是快照里的参数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitParameters {
    pub model: String,
    pub texture: bool,
    pub pbr: bool,
    pub texture_quality: String,
    pub geometry_quality: String,
    pub face_limit: i64,
    pub quad: bool,
    pub generate_parts: bool,
}

impl SubmitParameters {
    /// 从快照的 `provider_config.tripo`（`TripoParametersDto` 形态，**camelCase** 键）解析。
    ///
    /// 缺字段／类型不符一律报错：宁可停在 needs_input，也不发一个"我们猜的参数"的付费请求。
    pub fn from_provider_config(provider_config: &Value) -> Result<Self, String> {
        let tripo = provider_config
            .get("tripo")
            .ok_or_else(|| "快照 provider_config 缺少 tripo 段".to_owned())?;
        let string_field = |name: &str| -> Result<String, String> {
            tripo
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("provider_config.tripo.{name} 缺失或不是字符串"))
        };
        let bool_field = |name: &str| -> Result<bool, String> {
            tripo
                .get(name)
                .and_then(Value::as_bool)
                .ok_or_else(|| format!("provider_config.tripo.{name} 缺失或不是布尔值"))
        };
        let face_limit = tripo
            .get("faceLimit")
            .and_then(Value::as_i64)
            .ok_or_else(|| "provider_config.tripo.faceLimit 缺失或不是整数".to_owned())?;
        Ok(Self {
            model: string_field("model")?,
            texture: bool_field("texture")?,
            pbr: bool_field("pbr")?,
            texture_quality: string_field("textureQuality")?,
            geometry_quality: string_field("geometryQuality")?,
            face_limit,
            quad: bool_field("quad")?,
            generate_parts: bool_field("generateParts")?,
        })
    }
}

/// 一份有方向名的输入（token 为 opaque string）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewInput {
    /// 方向名：只允许 `front/left/back/right`（detail 不进入多视图请求）。
    pub view: String,
    pub token: String,
}

/// 多视图请求体（contracts.md §6 的字段形态；**没有** v2 的 `model_version`/`files`）。
///
/// `inputs` 用"有方向名的对象"形态（`[{"front": token}, {"left": token}]`）；
/// 缺失方向直接不提交对应对象（由调用方过滤）。
#[derive(Debug, Clone, Serialize)]
pub struct SubmitRequest {
    pub inputs: Vec<Map<String, Value>>,
    pub model: String,
    pub texture: bool,
    pub pbr: bool,
    pub texture_quality: String,
    pub geometry_quality: String,
    pub face_limit: i64,
    pub quad: bool,
    pub generate_parts: bool,
}

impl SubmitRequest {
    /// 组装请求体；`inputs` 顺序即视图槽位顺序（front→left→back→right）。
    pub fn new(parameters: &SubmitParameters, views: &[ViewInput]) -> Self {
        let inputs = views
            .iter()
            .map(|view| {
                let mut object = Map::new();
                object.insert(view.view.clone(), Value::String(view.token.clone()));
                object
            })
            .collect();
        Self {
            inputs,
            model: parameters.model.clone(),
            texture: parameters.texture,
            pbr: parameters.pbr,
            texture_quality: parameters.texture_quality.clone(),
            geometry_quality: parameters.geometry_quality.clone(),
            face_limit: parameters.face_limit,
            quad: parameters.quad,
            generate_parts: parameters.generate_parts,
        }
    }

    /// 无缩进的确定性 JSON 字节（`request_hash` 与断言共用同一份字节）。
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("多视图请求体总是可序列化")
    }
}

/// 付费提交响应 `data`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitData {
    /// 远端任务 ID（opaque string，不校验 UUID、不截断）。
    pub task_id: String,
}

/// 解析提交响应 `data`（`code == 0` 时调用）。
pub fn submit_data(data: Option<&Value>) -> Result<SubmitData, String> {
    let Some(data) = data else {
        return Err("响应 code=0 但缺少 data 对象".to_owned());
    };
    match opaque_field(data, "task_id") {
        Some(task_id) => Ok(SubmitData { task_id }),
        None => Err(format!(
            "响应缺少 task_id 字段：dataKeys={:?}",
            data_keys(data)
        )),
    }
}

// ---------------------------------------------------------------------------
// GET /tasks/{task_id}（查询）
// ---------------------------------------------------------------------------

/// 计费事实：原始字面量 + 精确换算的 `creditMinor` + 来源字段名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingFact {
    /// 原始字段名（`credits_consumed` / `credits`；T23 实测后收敛）。
    pub source_field: String,
    /// 供应商原始十进制字面量（原样保留，不重写）。
    pub literal: String,
    /// 1/100 credit（精确 decimal → 整数，Ceil）。
    pub credit_minor: i64,
}

/// 计费字段候选（按优先级；官方文档字段名以 T23 实测收敛）。
pub const BILLING_FIELDS: [&str; 2] = ["credits_consumed", "credits"];

/// 从 `data` 对象提取计费事实。
///
/// - 字段不存在 → `Ok(None)`（"没有计费信息"是允许的事实，不填 0）；
/// - 字段存在但无法精确解析（非十进制字面量、负数、溢出）→ `Err`（诊断文本），
///   调用方记录诊断并**不**据此结算（不得用猜测金额结账）。
pub fn extract_billing(data: &Value) -> Result<Option<BillingFact>, String> {
    let Some(object) = data.as_object() else {
        return Ok(None);
    };
    for field in BILLING_FIELDS {
        let Some(value) = object.get(field) else {
            continue;
        };
        let literal = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            other => {
                return Err(format!(
                    "计费字段 {field} 不是数字或字符串：{}",
                    describe_value(other)
                ));
            }
        };
        let credit_minor = parse_decimal_scaled(&literal, CREDIT_MINOR_SCALE, Rounding::Ceil)
            .map_err(|error| format!("计费字段 {field}={literal} 无法精确换算：{error}"))?;
        return Ok(Some(BillingFact {
            source_field: field.to_owned(),
            literal,
            credit_minor,
        }));
    }
    Ok(None)
}

/// 任务查询 `data` 的解析结果（原始状态与归一化状态的原料）。
#[derive(Debug, Clone, PartialEq)]
pub struct TaskData {
    /// 原始 `status` 字面量（**保留原值**，未知枚举不猜测）。
    pub status_raw: String,
    /// 原始 `progress`（诊断用；不参与判定，数字按 JSON 文本保留）。
    pub progress_raw: Option<String>,
    /// `output.model_url`（success 的必要条件；缺失即"没有可下载模型"）。
    pub model_url: Option<String>,
    /// `output.rendered_image_url`（预览；可选）。
    pub rendered_image_url: Option<String>,
    /// 计费事实（若有）。
    pub billing: Option<BillingFact>,
    /// 计费字段存在但无法精确解析时的诊断（不阻塞其它事实）。
    pub billing_problem: Option<String>,
    /// `data` 的字段名清单（诊断）。
    pub data_keys: Vec<String>,
}

/// 解析任务查询响应 `data`（`code == 0` 时调用）。
///
/// 缺 `status` → `Err`（协议出乎本卡假设：交由调用方按“查询失败”处理，
/// 不得当作生成失败）。
pub fn task_data(data: Option<&Value>) -> Result<TaskData, String> {
    let Some(data) = data else {
        return Err("响应 code=0 但缺少 data 对象".to_owned());
    };
    let Some(status_raw) = opaque_field(data, "status") else {
        return Err(format!(
            "响应缺少 status 字段：dataKeys={:?}",
            data_keys(data)
        ));
    };
    let progress_raw = data
        .as_object()
        .and_then(|object| object.get("progress"))
        .map(describe_value);
    let output = data.as_object().and_then(|object| object.get("output"));
    let model_url = output
        .and_then(|output| opaque_field(output, "model_url"))
        .filter(|value| !value.is_empty());
    let rendered_image_url = output
        .and_then(|output| opaque_field(output, "rendered_image_url"))
        .filter(|value| !value.is_empty());
    let (billing, billing_problem) = match extract_billing(data) {
        Ok(billing) => (billing, None),
        Err(problem) => (None, Some(problem)),
    };
    Ok(TaskData {
        status_raw,
        progress_raw,
        model_url,
        rendered_image_url,
        billing,
        billing_problem,
        data_keys: data_keys(data),
    })
}

/// 值的安全描述（诊断：只写类型与长度，不写长文本/URL 全量）。
pub fn describe_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => format!("字符串（{} 字符）", text.chars().count()),
        Value::Array(items) => format!("数组（{} 项）", items.len()),
        Value::Object(object) => format!("对象（{} 字段）", object.len()),
    }
}

/// 响应字节的安全描述（解析失败时的诊断；不打印原文，只给长度与开头的可打印片段）。
fn describe_bytes(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(120)]);
    format!("{} 字节：{:?}", bytes.len(), text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelope_requires_code_and_ignores_unknown_fields() {
        let envelope =
            parse_envelope(br#"{"code":0,"data":{"task_id":"t-1"},"extra":{"future":true}}"#)
                .unwrap();
        assert_eq!(envelope.code, 0);
        assert_eq!(envelope.data.unwrap()["task_id"], "t-1");

        let business = parse_envelope(
            br#"{"code":1201,"message":"invalid image token","suggestion":"re-upload"}"#,
        )
        .unwrap();
        assert_eq!(business.code, 1201);
        assert!(business.business_message().contains("1201"));
        assert!(business.business_message().contains("re-upload"));

        assert!(parse_envelope(b"not json at all").is_err());
        assert!(parse_envelope(br#"{"message":"no code"}"#).is_err());
    }

    #[test]
    fn opaque_strings_are_preserved_verbatim() {
        assert_eq!(
            opaque_string(&json!(" Token-With Spaces ")),
            Some(" Token-With Spaces ".to_owned()),
            "token 不做 trim／截断／UUID 校验"
        );
        assert_eq!(
            opaque_string(&json!(12345678901234_i64)),
            Some("12345678901234".to_owned())
        );
        assert_eq!(opaque_string(&json!("")), None);
    }

    #[test]
    fn upload_and_submit_data_parse_candidate_fields() {
        let upload = upload_data(Some(&json!({"file_token": "tok-1"}))).expect("file_token 形态");
        assert_eq!(upload.token, "tok-1");
        assert_eq!(upload.field, "file_token");
        let upload = upload_data(Some(&json!({"image_token": "tok-2"}))).expect("image_token 形态");
        assert_eq!(upload.token, "tok-2");
        assert_eq!(upload.field, "image_token");
        assert!(upload_data(Some(&json!({"other": "x"}))).is_err());

        let submit = submit_data(Some(&json!({"task_id": "task-9"}))).unwrap();
        assert_eq!(submit.task_id, "task-9");
        assert!(submit_data(Some(&json!({"id": "task-9"}))).is_err());
    }

    #[test]
    fn task_data_keeps_raw_status_and_optional_output() {
        let task = task_data(Some(&json!({
            "task_id": "t-1",
            "status": "some_new_status",
            "progress": 12,
            "output": {"model_url": "https://cdn.example.invalid/m.glb?sign=x"},
            "credits_consumed": "1.5"
        })))
        .unwrap();
        assert_eq!(task.status_raw, "some_new_status");
        assert_eq!(task.progress_raw.as_deref(), Some("12"));
        assert!(task.model_url.as_deref().unwrap().contains("sign=x"));
        assert_eq!(task.billing.as_ref().unwrap().credit_minor, 150);

        // success 缺模型 URL：解析成功但 model_url = None（调用方不得组装成功）。
        let missing = task_data(Some(&json!({"status": "success", "output": {}}))).unwrap();
        assert!(missing.model_url.is_none());

        // 缺 status：查询失败，不得当作生成失败。
        assert!(task_data(Some(&json!({"task_id": "t"}))).is_err());
    }

    #[test]
    fn billing_parses_exactly_and_reports_problems_without_guessing() {
        let billing = extract_billing(&json!({"credits_consumed": 30}))
            .unwrap()
            .unwrap();
        assert_eq!(billing.credit_minor, 3000);
        assert_eq!(billing.literal, "30");
        assert_eq!(billing.source_field, "credits_consumed");

        // 0.005 credits → 1 creditMinor（Ceil，绝不低于真实值）。
        let billing = extract_billing(&json!({"credits": "0.005"}))
            .unwrap()
            .unwrap();
        assert_eq!(billing.credit_minor, 1);

        // 无计费字段是合法事实（不填 0）。
        assert!(extract_billing(&json!({})).unwrap().is_none());
        // 无法精确解析 → Err（调用方记录诊断，不用猜测金额结算）。
        assert!(extract_billing(&json!({"credits_consumed": "about thirty"})).is_err());
        assert!(extract_billing(&json!({"credits_consumed": -1})).is_err());
    }

    #[test]
    fn submit_request_body_has_no_v2_shape_and_skips_missing_views() {
        let parameters = SubmitParameters {
            model: "v3.1-20260211".to_owned(),
            texture: true,
            pbr: true,
            texture_quality: "standard".to_owned(),
            geometry_quality: "standard".to_owned(),
            face_limit: 100_000,
            quad: false,
            generate_parts: false,
        };
        let request = SubmitRequest::new(
            &parameters,
            &[
                ViewInput {
                    view: "front".to_owned(),
                    token: "tok-front".to_owned(),
                },
                ViewInput {
                    view: "left".to_owned(),
                    token: "tok-left".to_owned(),
                },
            ],
        );
        let body: Value = serde_json::from_slice(&request.to_bytes()).unwrap();
        assert_eq!(body["inputs"][0]["front"], "tok-front");
        assert_eq!(body["inputs"][1]["left"], "tok-left");
        assert_eq!(body["model"], "v3.1-20260211");
        assert_eq!(body["face_limit"], 100_000);
        assert_eq!(body["quad"], false);
        assert_eq!(body["generate_parts"], false);
        // v2 字段形态一个都不能出现。
        for forbidden in [
            "model_version",
            "files",
            "type",
            "prompt",
            "negative_prompt",
        ] {
            assert!(
                body.get(forbidden).is_none(),
                "请求体不得包含 {forbidden}：{body}"
            );
        }
        // 参数解析：快照里的键是 **camelCase**（`TripoParametersDto`）；缺字段必须报错，
        // 不用默认值糊过去。
        assert!(SubmitParameters::from_provider_config(&json!({"tripo": {}})).is_err());
        assert!(
            SubmitParameters::from_provider_config(&json!({
                "tripo": {
                    "preset": "tripo-h-v3.1-standard",
                    "model": "v3.1-20260211",
                    "texture": true,
                    "pbr": true,
                    "texture_quality": "standard",
                    "geometry_quality": "standard",
                    "face_limit": 100000,
                    "quad": false,
                    "generate_parts": false
                }
            }))
            .is_err(),
            "snake_case 字段形态不属于快照合同，必须报错而不是猜"
        );
        let parsed = SubmitParameters::from_provider_config(&json!({
            "tripo": {
                "preset": "tripo-h-v3.1-standard",
                "model": "v3.1-20260211",
                "texture": true,
                "pbr": true,
                "textureQuality": "standard",
                "geometryQuality": "standard",
                "faceLimit": 100000,
                "quad": false,
                "generateParts": false
            }
        }))
        .unwrap();
        assert_eq!(parsed, parameters);
    }
}
