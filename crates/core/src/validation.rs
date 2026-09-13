//! 字段级校验与规范化（T07）：物品、说明书绑定与照片输入的纯规则层。
//!
//! 设计（RD 取舍，见 decisions.md ADR-016；QA 按此复核）：
//! - **无静默行为**：每个输入字段要么被接受（规范化后写入）、要么得到一条
//!   [`FieldIssue`]（HTTP 层映射为 422 的 `details.fields`）；不存在"字段被忽略但
//!   请求仍成功"的路径（T04 QA 记录的 `{"brand":null}` 静默保留原值即此类问题）。
//! - **PATCH 清空语义**：可选字段（brand/variant）显式 `null` 或空白字符串 = **清空**；
//!   必填字段（name/model）显式 `null` 或空白 = 字段错误（停用物品请用归档）。
//! - **规范化**：文本先 `trim`（Unicode 空白），空白可选字段落库为 NULL；
//!   长度按 `chars().count()`（字符数，不是字节数）计算。
//! - **全部错误一起返回**：一次请求收集所有字段问题，前端可一次聚焦全部错误。
//! - 本模块不依赖 HTTP/数据库：`server` 的 handler 负责把 [`FieldIssue`] 放进
//!   统一错误结构的 `details.fields`，repository 负责持久化。

use serde::{Deserialize, Serialize};

use crate::domain::{PageViewport, PhotoView};

/// 物品名称最大字符数（PRD 只写"超长 422"，未给数值；T07 取值，见 ADR-016）。
pub const ITEM_NAME_MAX_CHARS: usize = 200;
/// 品牌最大字符数。
pub const ITEM_BRAND_MAX_CHARS: usize = 100;
/// 型号最大字符数。
pub const ITEM_MODEL_MAX_CHARS: usize = 200;
/// 变体/配置最大字符数。
pub const ITEM_VARIANT_MAX_CHARS: usize = 200;
/// 说明书标题最大字符数。
pub const DOCUMENT_TITLE_MAX_CHARS: usize = 200;
/// `sourceUrl` 最大字符数（只作记载的出处链接）。
pub const SOURCE_URL_MAX_CHARS: usize = 2000;

/// 请求体整体（不是某个字段）的问题所使用的 `field` 值（例如空 PATCH）。
pub const BODY_FIELD: &str = "body";

/// 一条字段级校验问题（HTTP 层放入 `error.details.fields[]`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldIssue {
    /// 字段名（线上 camelCase；请求体整体问题为 [`BODY_FIELD`]）。
    pub field: String,
    /// 面向用户的中文说明；不含内部细节。
    pub message: String,
}

impl FieldIssue {
    pub fn new(field: &str, message: impl Into<String>) -> Self {
        Self {
            field: field.to_owned(),
            message: message.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// 物品
// ---------------------------------------------------------------------------

/// `POST /items` 的原始输入（每个字段都可缺失，便于逐字段报错）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemCreateInput {
    pub name: Option<String>,
    pub brand: Option<String>,
    pub model: Option<String>,
    pub variant: Option<String>,
}

/// 校验并规范化后的物品字段（name/model 必非空；brand/variant 已 trim，空 → None）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidItemFields {
    pub name: String,
    pub brand: Option<String>,
    pub model: String,
    pub variant: Option<String>,
}

/// 校验创建输入；返回全部字段问题或规范化字段。
pub fn validate_item_create(input: ItemCreateInput) -> Result<ValidItemFields, Vec<FieldIssue>> {
    let mut issues = Vec::new();
    let name = collect(
        &mut issues,
        required("name", input.name.as_deref(), ITEM_NAME_MAX_CHARS),
    );
    let brand = collect(
        &mut issues,
        optional("brand", input.brand.as_deref(), ITEM_BRAND_MAX_CHARS),
    );
    let model = collect(
        &mut issues,
        required("model", input.model.as_deref(), ITEM_MODEL_MAX_CHARS),
    );
    let variant = collect(
        &mut issues,
        optional("variant", input.variant.as_deref(), ITEM_VARIANT_MAX_CHARS),
    );
    match (name, brand, model, variant) {
        (Some(name), Some(brand), Some(model), Some(variant)) => Ok(ValidItemFields {
            name,
            brand,
            model,
            variant,
        }),
        _ => Err(issues),
    }
}

/// `PATCH /items/{id}` 的原始输入。
///
/// 文本字段是**双层 Option**：外层 `None` = 字段缺失（保持原值）；
/// `Some(None)` = 显式 `null`（可选字段清空 / 必填字段报错）；
/// `Some(Some(v))` = 提供新值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemPatchInput {
    pub name: Option<Option<String>>,
    pub brand: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub variant: Option<Option<String>>,
    /// 双层 Option：`Some(None)` = 显式 null（422，状态字段没有清空语义）。
    pub archived: Option<Option<bool>>,
}

/// 校验并规范化后的 PATCH 字段；`None` = 保持原值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidItemPatch {
    pub name: Option<String>,
    /// `Some(None)` = 清空；`Some(Some(v))` = 设置新值。
    pub brand: Option<Option<String>>,
    pub model: Option<String>,
    pub variant: Option<Option<String>>,
    /// 校验后的归档开关：`None` = 保持原值；`Some(bool)` = 设为该值。
    pub archived: Option<bool>,
}

impl ValidItemPatch {
    /// 是否至少提供了一个可修改字段（空 PATCH 在 [`validate_item_patch`] 即被拒绝，
    /// 本方法供调用方自检）。
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.brand.is_none()
            && self.model.is_none()
            && self.variant.is_none()
            && self.archived.is_none()
    }
}

/// 校验 PATCH 输入；空请求体与显式 `null` 的必填字段都是字段问题。
pub fn validate_item_patch(input: ItemPatchInput) -> Result<ValidItemPatch, Vec<FieldIssue>> {
    if input.name.is_none()
        && input.brand.is_none()
        && input.model.is_none()
        && input.variant.is_none()
        && input.archived.is_none()
    {
        return Err(vec![FieldIssue::new(
            BODY_FIELD,
            "请求体不能为空：至少提供一个可修改字段（name/brand/model/variant/archived）",
        )]);
    }

    let mut issues = Vec::new();
    let name = match input.name {
        None => None,
        Some(None) => {
            issues.push(FieldIssue::new(
                "name",
                "不能为 null（名称必填；如需停用物品请用 archived 归档）",
            ));
            None
        }
        Some(Some(raw)) => collect(
            &mut issues,
            required("name", Some(&raw), ITEM_NAME_MAX_CHARS),
        ),
    };
    let brand = match input.brand {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => collect(
            &mut issues,
            optional("brand", Some(&raw), ITEM_BRAND_MAX_CHARS),
        ),
    };
    let model = match input.model {
        None => None,
        Some(None) => {
            issues.push(FieldIssue::new(
                "model",
                "不能为 null（型号必填；如需停用物品请用 archived 归档）",
            ));
            None
        }
        Some(Some(raw)) => collect(
            &mut issues,
            required("model", Some(&raw), ITEM_MODEL_MAX_CHARS),
        ),
    };
    let variant = match input.variant {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => collect(
            &mut issues,
            optional("variant", Some(&raw), ITEM_VARIANT_MAX_CHARS),
        ),
    };
    let archived = match input.archived {
        None => None,
        Some(None) => {
            issues.push(FieldIssue::new(
                "archived",
                "不能为 null（只能是 true/false；保持原值请省略该字段）",
            ));
            None
        }
        Some(Some(archived)) => Some(archived),
    };

    if !issues.is_empty() {
        return Err(issues);
    }
    Ok(ValidItemPatch {
        name,
        brand,
        model,
        variant,
        archived,
    })
}

// ---------------------------------------------------------------------------
// 说明书绑定（source_url 只作出处记录，服务端不据此发起任何请求）
// ---------------------------------------------------------------------------

/// 校验并规范化后的 document 字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidDocumentFields {
    pub title: String,
    pub source_url: Option<String>,
}

/// 校验 document 的 `title` 与可选 `sourceUrl`。
pub fn validate_document_fields(
    title: Option<&str>,
    source_url: Option<&str>,
) -> Result<ValidDocumentFields, Vec<FieldIssue>> {
    let mut issues = Vec::new();
    let title = collect(
        &mut issues,
        required("title", title, DOCUMENT_TITLE_MAX_CHARS),
    );
    let source_url = match source_url {
        None => Some(None),
        Some(raw) => match validate_source_url(raw) {
            Ok(url) => Some(url),
            Err(issue) => {
                issues.push(issue);
                None
            }
        },
    };
    match (title, source_url) {
        (Some(title), Some(source_url)) => Ok(ValidDocumentFields { title, source_url }),
        _ => Err(issues),
    }
}

/// 校验出处链接：空 = 不记录；只接受绝对 `http(s)` URL；绝不据此发起抓取。
pub fn validate_source_url(raw: &str) -> Result<Option<String>, FieldIssue> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let count = trimmed.chars().count();
    if count > SOURCE_URL_MAX_CHARS {
        return Err(FieldIssue::new(
            "sourceUrl",
            format!("长度不能超过 {SOURCE_URL_MAX_CHARS} 个字符（当前 {count}）"),
        ));
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err(FieldIssue::new("sourceUrl", "URL 不能包含空白字符"));
    }
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    let Some(rest) = rest else {
        return Err(FieldIssue::new(
            "sourceUrl",
            "只接受绝对 http(s) URL（服务端不会访问该地址，仅作为出处记录）",
        ));
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Err(FieldIssue::new("sourceUrl", "URL 缺少主机名"));
    }
    if authority.contains('@') {
        return Err(FieldIssue::new(
            "sourceUrl",
            "URL 不能包含用户凭据（user:password@host 形式）",
        ));
    }
    Ok(Some(trimmed.to_owned()))
}

// ---------------------------------------------------------------------------
// 照片视图
// ---------------------------------------------------------------------------

/// 解析照片 `view` 字段：缺失或不在枚举内 → 字段问题（消息列出全部合法取值）。
///
/// 视图方向以**物品自身**为参照（PRD A-04）；`detail`（特写）不属于多视图集合
/// （T12 构造 Tripo 请求体时用 `PhotoView::is_multiview()` 过滤）。
pub fn validate_photo_view(value: Option<&str>) -> Result<PhotoView, FieldIssue> {
    let allowed = PhotoView::ALL
        .iter()
        .map(|view| view.as_str())
        .collect::<Vec<_>>()
        .join("/");
    match value {
        None => Err(FieldIssue::new("view", "必填：不能缺失或为 null")),
        Some(raw) => PhotoView::from_wire(raw).ok_or_else(|| {
            FieldIssue::new("view", format!("只接受 {allowed}（收到 {:?}）", raw.trim()))
        }),
    }
}

// ---------------------------------------------------------------------------
// PDF 页准备（T09 / REQ-014、REQ-015）
// ---------------------------------------------------------------------------

/// 原 PDF 页数上限（PRD §5.3、架构 §5.1：≤100 页）。
///
/// 客户端（PDF.js）在打开 PDF 后先拒绝超过该值的文件；服务端在 `PUT .../pages/{n}`
/// 与 `complete` 里再次校验（不信任客户端），Web 与 Rust 两侧共用同一常量语义。
pub const MAX_PDF_PAGES: i64 = 100;

/// 页图长边上限（像素；架构 §5.1：长边 ≤2000px）。
pub const MAX_PAGE_IMAGE_LONG_EDGE: u32 = 2000;

/// 页图允许的 MIME（白底 JPEG，架构 §5.1）。
pub const PAGE_IMAGE_MIME: &str = "image/jpeg";

/// 解析 `PUT /preparations/{id}/pages/{n}` 的页号：必须是 ≥1 且 ≤[`MAX_PDF_PAGES`] 的整数。
///
/// 页码 1-based（contracts.md §1）；超上界按"超出 100 页上限"拒绝（REQ-014/AC-024）。
pub fn validate_page_number(value: i64) -> Result<i64, FieldIssue> {
    if value < 1 {
        return Err(FieldIssue::new(
            "pageNumber",
            "页码从 1 开始（1-based）：不接受 0 或负页号",
        ));
    }
    if value > MAX_PDF_PAGES {
        return Err(FieldIssue::new(
            "pageNumber",
            format!("第 {value} 页超出页数上限（最多 {MAX_PDF_PAGES} 页）"),
        ));
    }
    Ok(value)
}

/// 校验页图 viewport：尺寸非零、长边 ≤[`MAX_PAGE_IMAGE_LONG_EDGE`]、旋转为 90 的倍数。
pub fn validate_page_viewport(viewport: PageViewport) -> Result<PageViewport, Vec<FieldIssue>> {
    let mut issues = Vec::new();
    if viewport.width == 0 || viewport.height == 0 {
        issues.push(FieldIssue::new(
            "viewport",
            "viewport 的 width/height 必须为正整数（页图坐标以旋转后 viewport 左上角为原点）",
        ));
    }
    if viewport.long_edge() > MAX_PAGE_IMAGE_LONG_EDGE {
        issues.push(FieldIssue::new(
            "viewport",
            format!(
                "页图长边 {}px 超过上限 {MAX_PAGE_IMAGE_LONG_EDGE}px",
                viewport.long_edge()
            ),
        ));
    }
    if !viewport.rotation_is_valid() {
        issues.push(FieldIssue::new(
            "viewport",
            format!("rotation 只接受 0/90/180/270（收到 {}）", viewport.rotation),
        ));
    }
    if issues.is_empty() {
        Ok(viewport)
    } else {
        Err(issues)
    }
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

/// 收集结果：出错时把问题放入列表并返回 `None`（继续校验其余字段）。
fn collect<T>(issues: &mut Vec<FieldIssue>, result: Result<T, FieldIssue>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(issue) => {
            issues.push(issue);
            None
        }
    }
}

/// 必填文本：缺失/`null`、空白、超长 → 字段问题；成功返回 trim 后的值。
fn required(field: &str, value: Option<&str>, max_chars: usize) -> Result<String, FieldIssue> {
    let Some(raw) = value else {
        return Err(FieldIssue::new(field, "必填：不能缺失或为 null"));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FieldIssue::new(field, "必填：不能为空白"));
    }
    let count = trimmed.chars().count();
    if count > max_chars {
        return Err(FieldIssue::new(
            field,
            format!("长度不能超过 {max_chars} 个字符（当前 {count}）"),
        ));
    }
    Ok(trimmed.to_owned())
}

/// 可选文本：缺失/`null`、空白 → `None`（清空或不设置）；超长 → 字段问题。
fn optional(
    field: &str,
    value: Option<&str>,
    max_chars: usize,
) -> Result<Option<String>, FieldIssue> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let count = trimmed.chars().count();
    if count > max_chars {
        return Err(FieldIssue::new(
            field,
            format!("长度不能超过 {max_chars} 个字符（当前 {count}）"),
        ));
    }
    Ok(Some(trimmed.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(name: Option<&str>, brand: Option<&str>, model: Option<&str>) -> ItemCreateInput {
        ItemCreateInput {
            name: name.map(str::to_owned),
            brand: brand.map(str::to_owned),
            model: model.map(str::to_owned),
            variant: None,
        }
    }

    #[test]
    fn create_requires_name_and_model_and_trims() {
        let ok = validate_item_create(create(Some("  相机  "), Some("  "), Some("X100V"))).unwrap();
        assert_eq!(ok.name, "相机");
        assert_eq!(ok.brand, None, "空白可选字段应规范化为 None");
        assert_eq!(ok.model, "X100V");

        let issues = validate_item_create(create(Some("   "), None, None)).unwrap_err();
        let fields: Vec<&str> = issues.iter().map(|issue| issue.field.as_str()).collect();
        assert_eq!(fields, vec!["name", "model"], "缺失与空白都要报出");
    }

    #[test]
    fn create_reports_too_long_fields_by_char_count() {
        let long = "长".repeat(ITEM_NAME_MAX_CHARS + 1);
        let issues = validate_item_create(create(Some(&long), None, Some("M"))).unwrap_err();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].field, "name");
        assert!(issues[0].message.contains("200"), "{}", issues[0].message);

        // 正好在限制内：接受（按字符数，不按字节数）。
        let exact = "字".repeat(ITEM_NAME_MAX_CHARS);
        assert!(validate_item_create(create(Some(&exact), None, Some("M"))).is_ok());
    }

    #[test]
    fn patch_empty_body_is_rejected() {
        let issues = validate_item_patch(ItemPatchInput::default()).unwrap_err();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].field, BODY_FIELD);
    }

    #[test]
    fn patch_null_clears_optional_fields() {
        let patch = validate_item_patch(ItemPatchInput {
            brand: Some(None),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(patch.brand, Some(None), "显式 null = 清空");

        let patch = validate_item_patch(ItemPatchInput {
            variant: Some(Some("  ".to_owned())),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(patch.variant, Some(None), "空白字符串 = 清空");

        let patch = validate_item_patch(ItemPatchInput {
            brand: Some(Some(" 尼康 ".to_owned())),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(patch.brand, Some(Some("尼康".to_owned())));

        // 缺失 = 保持原值。
        let patch = validate_item_patch(ItemPatchInput {
            archived: Some(Some(true)),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(patch.brand, None);
        assert_eq!(patch.archived, Some(true));

        // archived 显式 null 也要报错（状态字段没有"清空"语义）。
        let issues = validate_item_patch(ItemPatchInput {
            archived: Some(None),
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(issues[0].field, "archived");
    }

    #[test]
    fn patch_null_on_required_fields_is_an_error() {
        for (name_field, model_field) in [(true, false), (false, true)] {
            let issues = validate_item_patch(ItemPatchInput {
                name: name_field.then_some(None),
                model: model_field.then_some(None),
                ..Default::default()
            })
            .unwrap_err();
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].field, if name_field { "name" } else { "model" });
            assert!(issues[0].message.contains("null"), "{}", issues[0].message);
        }
    }

    #[test]
    fn photo_view_field_lists_allowed_values() {
        assert_eq!(
            validate_photo_view(Some("front")).unwrap(),
            PhotoView::Front
        );
        assert_eq!(
            validate_photo_view(Some("detail")).unwrap(),
            PhotoView::Detail
        );
        let issue = validate_photo_view(Some("top")).unwrap_err();
        assert_eq!(issue.field, "view");
        for allowed in ["front", "left", "back", "right", "detail"] {
            assert!(issue.message.contains(allowed), "{}", issue.message);
        }
        assert_eq!(validate_photo_view(None).unwrap_err().field, "view");
    }

    #[test]
    fn document_title_and_url_rules() {
        let fields = validate_document_fields(Some(" 说明书 "), None).unwrap();
        assert_eq!(fields.title, "说明书");
        assert_eq!(fields.source_url, None);

        let fields =
            validate_document_fields(Some("t"), Some(" https://example.com/a.pdf ")).unwrap();
        assert_eq!(
            fields.source_url.as_deref(),
            Some("https://example.com/a.pdf")
        );

        assert!(validate_document_fields(None, None).is_err());
        let issues = validate_document_fields(Some("t"), Some("file:///etc/passwd")).unwrap_err();
        assert_eq!(issues[0].field, "sourceUrl");
        assert!(validate_document_fields(Some("t"), Some("javascript:alert(1)")).is_err());
        assert!(validate_document_fields(Some("t"), Some("https://")).is_err());
        assert!(
            validate_document_fields(Some("t"), Some("https://user:pw@example.com/x")).is_err(),
            "不允许在出处链接中保存凭据"
        );
        assert!(
            validate_document_fields(Some("t"), Some("http:// example.com")).is_err(),
            "空白字符非法"
        );
        let long_url = format!("https://example.com/{}", "a".repeat(SOURCE_URL_MAX_CHARS));
        assert!(validate_document_fields(Some("t"), Some(&long_url)).is_err());
    }

    #[test]
    fn page_number_is_one_based_and_bounded() {
        assert_eq!(validate_page_number(1).unwrap(), 1);
        assert_eq!(validate_page_number(MAX_PDF_PAGES).unwrap(), MAX_PDF_PAGES);

        assert_eq!(validate_page_number(0).unwrap_err().field, "pageNumber");
        assert_eq!(validate_page_number(-3).unwrap_err().field, "pageNumber");
        let over = validate_page_number(MAX_PDF_PAGES + 1).unwrap_err();
        assert_eq!(over.field, "pageNumber");
        assert!(
            over.message.contains("100"),
            "超页数消息必须写明上限：{}",
            over.message
        );
    }

    #[test]
    fn viewport_rejects_zero_size_oversize_and_bad_rotation() {
        let ok = validate_page_viewport(PageViewport {
            width: 2000,
            height: 1414,
            rotation: 90,
        })
        .unwrap();
        assert_eq!(ok.long_edge(), 2000);

        let issues = validate_page_viewport(PageViewport {
            width: 0,
            height: 100,
            rotation: 0,
        })
        .unwrap_err();
        assert_eq!(issues[0].field, "viewport");

        let issues = validate_page_viewport(PageViewport {
            width: 2001,
            height: 100,
            rotation: 0,
        })
        .unwrap_err();
        assert!(issues[0].message.contains("2000px"));

        let issues = validate_page_viewport(PageViewport {
            width: 100,
            height: 100,
            rotation: 45,
        })
        .unwrap_err();
        assert!(issues[0].message.contains("0/90/180/270"));
    }
}
