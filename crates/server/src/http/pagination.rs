//! 列表分页原语（contracts.md §1）：`{data, nextCursor}`、默认 20／最多 100、
//! 游标包含稳定排序键与 ID。
//!
//! 为什么游标要绑定查询条件（T07 补齐 T04 遗留的"过滤条件下游标语义"）：
//! 游标是"上次返回的最后一行的位置"，只有在**同一过滤条件**下才有意义。若允许把
//! 默认列表的游标拿到 `archived=true` 列表继续翻页，会静默跳过/重复数据。因此游标
//! 带版本前缀与作用域（scope）：解析失败 → 422 字段级错误；作用域不匹配 → 422 并
//! 明确要求从头分页。游标对客户端始终是不透明字符串，不得自行拼接。
//!
//! 查询参数解析**严格**：未知参数、重复参数、非法取值都返回 422 字段级明细
//! （不静默忽略——与"未知配置键报错"同一原则）。handler 用
//! `Query<Vec<(String, String)>>` 提取（保留重复参数，重复不是"最后一个生效"）。

use manual_core::validation::FieldIssue;

/// 默认页大小（contracts.md §1：默认 20）。
pub const DEFAULT_PAGE_SIZE: u32 = 20;
/// 最大页大小（contracts.md §1：最多 100）。
pub const MAX_PAGE_SIZE: u32 = 100;

/// 游标版本前缀（格式：`v1:<scope>:<sortMillis>:<id>`）。
const CURSOR_PREFIX: &str = "v1";

/// 解析后的列表查询参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListParams {
    pub limit: u32,
    /// 原始游标字符串（由各 handler 用对应 scope 解析）。
    pub cursor: Option<String>,
}

/// 解析 `limit` / `cursor` 与调用方声明的额外参数（如 `archived`）。
///
/// 校验规则：
/// - `limit`：整数且在 `1..=100`；缺失用默认 20；
/// - `cursor`：原样保留（格式校验在 [`Cursor::parse`]）；
/// - `extra` 中的参数名交给调用方自行取值（本函数只做"已声明/不重复"判定）；
/// - 其余参数名、重复参数 → 返回全部字段问题（调用方组装 422 `details.fields`）。
pub fn parse_list_params(
    params: &[(String, String)],
    extra_allowed: &[&str],
) -> Result<ListParams, Vec<FieldIssue>> {
    let mut issues: Vec<FieldIssue> = Vec::new();
    let mut limit = DEFAULT_PAGE_SIZE;
    let mut cursor: Option<String> = None;
    let mut seen: Vec<&str> = Vec::new();

    for (name, value) in params {
        if seen.contains(&name.as_str()) {
            issues.push(FieldIssue::new(name, "参数重复"));
            continue;
        }
        seen.push(name.as_str());
        match name.as_str() {
            "limit" => match value.trim().parse::<u32>() {
                Ok(parsed) if (1..=MAX_PAGE_SIZE).contains(&parsed) => limit = parsed,
                Ok(parsed) => issues.push(FieldIssue::new(
                    "limit",
                    format!("必须在 1..={MAX_PAGE_SIZE} 之间（收到 {parsed}）"),
                )),
                Err(_) => issues.push(FieldIssue::new(
                    "limit",
                    format!("必须是整数（收到 {value:?}）"),
                )),
            },
            "cursor" => cursor = Some(value.clone()),
            other if extra_allowed.contains(&other) => {}
            other => issues.push(FieldIssue::new(other, "未知查询参数")),
        }
    }

    if !issues.is_empty() {
        return Err(issues);
    }
    Ok(ListParams { limit, cursor })
}

/// 取一个已声明参数的原始值（重复/缺失由调用方按自身语义处理）。
pub fn raw_value<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// 稳定排序游标：`v1:<scope>:<sortMillis>:<id>`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub scope: String,
    pub sort_millis: i64,
    pub id: String,
}

impl Cursor {
    /// 用一行的排序键与 ID 生成游标（`scope` 必须与解析时一致）。
    pub fn encode(scope: &str, sort_millis: i64, id: &str) -> String {
        format!("{CURSOR_PREFIX}:{scope}:{sort_millis}:{id}")
    }

    /// 解析并校验作用域；格式错误或作用域不匹配 → `cursor` 字段问题
    /// （调用方组装 422 `details.fields`）。
    pub fn parse(value: &str, expected_scope: &str) -> Result<Self, FieldIssue> {
        let invalid = |message: &str| FieldIssue::new("cursor", message);
        let rest = value
            .strip_prefix(CURSOR_PREFIX)
            .and_then(|rest| rest.strip_prefix(':'))
            .ok_or_else(|| invalid("cursor 格式非法：必须原样回传服务端返回的 nextCursor"))?;
        let (rest, id) = rest
            .rsplit_once(':')
            .ok_or_else(|| invalid("cursor 格式非法：缺少 id"))?;
        let (scope, millis) = rest
            .rsplit_once(':')
            .ok_or_else(|| invalid("cursor 格式非法：缺少排序键"))?;
        let sort_millis = millis
            .parse::<i64>()
            .map_err(|_| invalid("cursor 的排序键必须是整数毫秒"))?;
        if id.is_empty() {
            return Err(invalid("cursor 缺少 id"));
        }
        if scope != expected_scope {
            return Err(invalid(
                "cursor 与当前查询条件不匹配：请从头开始分页（不要跨过滤条件复用游标）",
            ));
        }
        Ok(Self {
            scope: scope.to_owned(),
            sort_millis,
            id: id.to_owned(),
        })
    }

    /// 交给仓储做行值比较的 `(排序键, id)`。
    pub fn into_tuple(self) -> (i64, String) {
        (self.sort_millis, self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips_and_binds_scope() {
        let encoded = Cursor::encode("items:active", 1_789_171_200_000, "abc-1");
        let parsed = Cursor::parse(&encoded, "items:active").unwrap();
        assert_eq!(parsed.sort_millis, 1_789_171_200_000);
        assert_eq!(parsed.id, "abc-1");

        let mismatch = Cursor::parse(&encoded, "items:archived").unwrap_err();
        assert_eq!(mismatch.field, "cursor");
        assert!(mismatch.message.contains("不匹配"), "{}", mismatch.message);

        // 作用域可以包含冒号（documents:<itemId>），ID 与排序键仍能正确切分。
        let scoped = Cursor::encode("documents:item-1", 5, "doc-9");
        assert_eq!(
            Cursor::parse(&scoped, "documents:item-1").unwrap().id,
            "doc-9"
        );
    }

    #[test]
    fn malformed_cursors_report_the_cursor_field() {
        for value in ["", "abc", "v1:items:active", "v1:x:y:z", "v1::1:id"] {
            let issue = Cursor::parse(value, "items:active").unwrap_err();
            assert_eq!(issue.field, "cursor", "值 {value:?}");
            assert!(!issue.message.is_empty());
        }
    }

    #[test]
    fn list_params_reject_unknown_duplicate_and_bad_limit() {
        let ok = parse_list_params(
            &[
                ("limit".to_owned(), "50".to_owned()),
                ("archived".to_owned(), "true".to_owned()),
                ("cursor".to_owned(), "v1:a:1:b".to_owned()),
            ],
            &["archived"],
        )
        .unwrap();
        assert_eq!(ok.limit, 50);
        assert_eq!(ok.cursor.as_deref(), Some("v1:a:1:b"));
        assert_eq!(
            raw_value(&[("archived".to_owned(), "true".to_owned())], "archived"),
            Some("true")
        );

        for (params, field) in [
            (vec![("limit".to_owned(), "0".to_owned())], "limit"),
            (vec![("limit".to_owned(), "101".to_owned())], "limit"),
            (vec![("limit".to_owned(), "abc".to_owned())], "limit"),
            (
                vec![("frobnicate".to_owned(), "1".to_owned())],
                "frobnicate",
            ),
            (
                vec![
                    ("limit".to_owned(), "1".to_owned()),
                    ("limit".to_owned(), "2".to_owned()),
                ],
                "limit",
            ),
        ] {
            let issues = parse_list_params(&params, &[]).unwrap_err();
            assert_eq!(issues[0].field, field, "参数 {params:?}");
        }
    }
}
