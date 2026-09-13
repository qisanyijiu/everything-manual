//! 供应商临时 URL 的脱敏规则（T20 · BUG-008；contracts.md §1/§7、PRD AC-010/AC-058）。
//!
//! 规则（本模块是唯一实现点，其他模块不得各写一套）：
//!
//! - **供应商临时/签名 URL 不进入持久化任务元数据、不进入 API 响应、不进入备份快照**
//!   （AC-010「导出与备份内容不含……临时云端 URL」）。签名查询串里的凭据是能力（capability），
//!   不是事实；
//! - 允许保留的**最小诊断信息** = URL 的不可逆摘要（sha256 前 [`URL_SUMMARY_SHA256_CHARS`]
//!   位十六进制）+ host（无 scheme、无 path、无查询串）。保留它们是为了回答
//!   "当时看到的是哪个 CDN / 哪一次链接"，而不是为了重新下载；
//! - URL 形状的字符串在 JSON 里被替换为**自描述的摘要对象**
//!   `{"redacted": true, "host": ..., "sha256": ...}`：键保留在原来的位置
//!   （例如 `modelUrl`），值不再可能是可用的链接；文本列（非 JSON）用
//!   [`redact_urls_in_text`] 的摘要标签；
//! - **历史数据不强制迁移**：修复前落库的行由展示路径（`http` DTO）与备份路径
//!   （`backup` 快照过滤）在**读取/落盘时**脱敏，源 data-dir 不被改写。
//!
//! 为什么不是"备份时失败"：备份是灾备，历史行里有旧 URL 不该让用户无法备份
//! （见 `implementation.md` §T20-12 与 `llmdoc/decisions.md` ADR-032）。
//! 为什么不是"整库删 URL 字符串"：用户自己填写的出处链接（`sourceUrl`）
//! 属于"来源"而非供应商临时地址（contracts §7、T20-10 第 8 条），
//! 备份/导出必须原样保留；本模块只作用于**供应商事实**字段与 JSON 值，不做整库文本清洗。
//!
//! ## JSON 列的结构感知脱敏（BUG-011/BUG-012；完整定义见 `decisions.md`「ADR-034」）
//!
//! JSON 列（`usage_json` / `needs_input_json` / 快照里的同类列）**逐字符串值**脱敏，
//! 键、结构与其它文本原样保留，绝不因脱敏产生非法 JSON：
//!
//! - 字符串**整体就是一个 URL** → 替换为自描述摘要对象
//!   （[`JsonStringRedaction::SummaryObject`]，ADR-032：位置仍在、值不再可用）；
//! - 字符串是**句子**（URL 只是其中片段）→ 只替换 URL 片段为摘要标签
//!   （[`redact_urls_in_text`]），句子与 JSON 形状完整保留（BUG-011：`needs_input`
//!   的整句说明不得丢失）；
//! - 契约规定值必须是字符串的列（`needs_input_json` 的 `message`）用
//!   [`JsonStringRedaction::KeepString`]：整串 URL 也只留摘要标签，**永不改变值的类型**。
//!
//! 序列化 JSON 文本的统一写入入口 = [`redact_json_text_urls`]（解析失败按文本兜底）；
//! 仓储写入（`job_stages`）与备份快照（`backup::create`）共用它，不再各写一套。
//!
//! ## "URL 形态"口径（OB-9 定标，2026-09-13；完整定义见 `decisions.md`「OB-9」）
//!
//! - **需要脱敏的 URL 文本** = 文本中出现的 `scheme://…` 片段（scheme 为 RFC 3986
//!   scheme 字符集；出现 `://` 即命中）。处理 = 整段替换为 [`UrlSummary::to_label`]
//!   摘要标签（host + 不可逆 sha256 前缀）——**错误/失败/任务事实/日志 detail 文本中
//!   不允许保留 `scheme://host/path` 形态**（即使无查询串）；
//! - **裸 host**（`cdn.example.invalid`、`127.0.0.1`，无 `://`）不是 URL 文本，
//!   允许保留（最小诊断信息）；task_id、计数、状态、摘要对象/标签同样原样保留；
//! - **唯一例外：部署者自有配置回显**（provider `baseUrl` 的日志/管理页展示）走
//!   [`crate::config::secret::redact_url_query`]（去查询串保留 `scheme://host/path`）：
//!   它由部署者提供、构造时已校验不含查询串/片段，不是供应商临时地址；
//! - **实现一致性**：文本级规则的唯一实现是 [`redact_urls_in_text`] / [`redact_text_urls`]；
//!   调用点见 `decisions.md`「OB-9」的证据清单（错误构造、仓储写入、HTTP DTO、备份兜底）。

use serde_json::Value;
use sha2::{Digest, Sha256};

/// 摘要保留的 sha256 十六进制位数（64 bit：足以标识，不可逆）。
pub const URL_SUMMARY_SHA256_CHARS: usize = 16;

/// 一个 URL 的不可逆摘要（host + sha256 前缀）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlSummary {
    /// URL 的 host（不含端口以外的任何部分；解析失败为 `None`）。
    pub host: Option<String>,
    /// sha256(url) 的前 [`URL_SUMMARY_SHA256_CHARS`] 位十六进制。
    pub sha256_prefix: String,
}

impl UrlSummary {
    /// 转成持久化/出网使用的自描述摘要对象（绝不包含可用的 URL 字符串）。
    pub fn to_value(&self) -> Value {
        let mut object = serde_json::Map::new();
        object.insert("redacted".to_owned(), Value::Bool(true));
        if let Some(host) = &self.host {
            object.insert("host".to_owned(), Value::String(host.clone()));
        }
        object.insert(
            "sha256".to_owned(),
            Value::String(self.sha256_prefix.clone()),
        );
        Value::Object(object)
    }

    /// 文本形态的摘要标签（非 JSON 文本列用；不含 URL 子串）。
    pub fn to_label(&self) -> String {
        match &self.host {
            Some(host) => format!(
                "（临时供应商地址已脱敏：host={host}；sha256={}…）",
                self.sha256_prefix
            ),
            None => format!("（临时供应商地址已脱敏：sha256={}…）", self.sha256_prefix),
        }
    }
}

/// 计算 URL 的摘要（对**原样字符串**做 sha256：含查询串，仍不可逆）。
pub fn url_summary(url: &str) -> UrlSummary {
    let digest = Sha256::digest(url.as_bytes());
    let prefix = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .chars()
        .take(URL_SUMMARY_SHA256_CHARS)
        .collect();
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned));
    UrlSummary {
        host,
        sha256_prefix: prefix,
    }
}

/// 字符串是否"URL 形态"（含 `://`；与 T15 起沿用的 DTO 判据一致）。
pub fn is_url_like(text: &str) -> bool {
    text.contains("://")
}

/// JSON 里字符串值的脱敏形态（[`redact_urls_in_json_with`] 的策略参数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonStringRedaction {
    /// 整串 URL → 自描述摘要对象（ADR-032；任意事实 JSON 用）。
    SummaryObject,
    /// **永不改变字符串类型**：整串 URL 也只替换为摘要标签。
    /// 用于契约规定值必须是字符串的列（`needs_input_json` 的 `message`）——
    /// 值变成对象会让按类型反序列化的读取侧整列解析失败（BUG-011 的教训）。
    KeepString,
}

/// 递归脱敏 JSON（逐字符串值；[`JsonStringRedaction::SummaryObject`] 形态）；
/// 返回替换数。见 [`redact_urls_in_json_with`]。
pub fn redact_urls_in_json(value: &mut Value) -> usize {
    redact_urls_in_json_with(value, JsonStringRedaction::SummaryObject)
}

/// 递归脱敏 JSON：只动字符串值，不改键名、不动其它类型（id/计数/状态/计费字段原样保留）。
///
/// - **整串 URL**（字符串去掉首尾空白后只有一个 URL 段）：[`JsonStringRedaction::SummaryObject`]
///   下替换为 [`UrlSummary::to_value`] 摘要对象；[`JsonStringRedaction::KeepString`] 下替换为
///   摘要标签（仍是字符串）；
/// - **句子**（URL 只是片段）：只把 URL 片段替换为摘要标签（`redact_urls_in_text`），
///   其余文字原样保留——`needs_input` 的整句说明属于可行动信息，不得丢失（BUG-011）。
pub fn redact_urls_in_json_with(value: &mut Value, mode: JsonStringRedaction) -> usize {
    match value {
        Value::String(text) => {
            if !is_url_like(text) {
                return 0;
            }
            if mode == JsonStringRedaction::SummaryObject
                && let Some(summary) = whole_url_summary(text)
            {
                *value = summary.to_value();
                return 1;
            }
            let (redacted, count) = redact_urls_in_text(text);
            if count > 0 {
                *value = Value::String(redacted);
            }
            count
        }
        Value::Array(items) => items
            .iter_mut()
            .map(|item| redact_urls_in_json_with(item, mode))
            .sum(),
        Value::Object(map) => map
            .values_mut()
            .map(|entry| redact_urls_in_json_with(entry, mode))
            .sum(),
        _ => 0,
    }
}

/// 序列化 JSON 文本的统一脱敏入口（仓储写入与备份快照共用）。
///
/// 解析成功时按 [`redact_urls_in_json_with`] 逐字符串值替换（键、结构、其它文本原样保留，
/// 结果仍是合法 JSON）；解析失败（损坏的文本列）退回 [`redact_urls_in_text`] 兜底。
/// 返回 `(脱敏后文本, 替换数)`；替换数为 0 时文本原样返回。
pub fn redact_json_text_urls(text: &str, mode: JsonStringRedaction) -> (String, usize) {
    match serde_json::from_str::<Value>(text) {
        Ok(mut value) => {
            let count = redact_urls_in_json_with(&mut value, mode);
            if count == 0 {
                (text.to_owned(), 0)
            } else {
                (value.to_string(), count)
            }
        }
        Err(_) => redact_urls_in_text(text),
    }
}

/// 字符串是否"整体就是一个 URL"（去掉首尾空白后只有一个 URL 段）；返回该 URL 的摘要。
fn whole_url_summary(text: &str) -> Option<UrlSummary> {
    match scan_url_segments(text.trim()).as_slice() {
        [(true, url)] => Some(url_summary(url)),
        _ => None,
    }
}

/// URL 右边界：只有 RFC 3986 允许构成 URL 的 ASCII 字符才继续消费。
///
/// `pchar` + `/` `?` `#` `[` `]` `%`（query/fragment 允许的集合）。这样 URL 后
/// 紧邻的其它字符（汉字、全角标点、空白、引号、反斜杠、尖括号等）**保持原样**，
/// 不会被并入 URL 段一起丢掉（BUG-010，「…?sign=x已过期」不得丢"已过期"）。
///
/// 与 RFC 的有意偏离：`'` 虽是 sub-delim，但在自由文本里更常是引号，
/// 因此不消费（避免把 `'…'` 的右引号吞进 URL 段）。
fn is_url_tail_char(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(
            character,
            '-' | '.'
                | '_'
                | '~'
                | ':'
                | '/'
                | '?'
                | '#'
                | '['
                | ']'
                | '@'
                | '!'
                | '$'
                | '&'
                | '('
                | ')'
                | '*'
                | '+'
                | ','
                | ';'
                | '='
                | '%'
        )
}

/// 扫描文本，产出交替的片段序列：`(是否 URL 段, 片段)`（普通文本片段非空）。
///
/// URL 段判定：以 `://` 为锚点向左吃掉 scheme、向右**只消费 RFC 3986 允许的
/// URL 字符**（[`is_url_tail_char`]），并回退尾随句读（`,`/`;`/`.`/`)`/`]`/`}`/`：`/`:`），
/// 因此不会吞掉紧邻的非 URL 字符（BUG-010）。`://` 前无 scheme 时不算 URL 段。
fn scan_url_segments(text: &str) -> Vec<(bool, &str)> {
    let bytes = text.as_bytes();
    let mut segments = Vec::new();
    let mut cursor = 0;
    let mut search = 0;
    while search < bytes.len() {
        let Some(found) = text[search..].find("://") else {
            break;
        };
        let scheme_end = search + found;
        // 向左吃掉 scheme（ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )）。
        let mut start = scheme_end;
        while start > search {
            let previous = bytes[start - 1];
            if previous.is_ascii_alphanumeric() || matches!(previous, b'+' | b'-' | b'.') {
                start -= 1;
            } else {
                break;
            }
        }
        if start == scheme_end {
            // `://` 前面没有 scheme：按普通文本处理（不整段吞掉）。
            search = scheme_end + 3;
            continue;
        }
        // 向右消费 URL 字符（RFC 3986 集合；其余字符一律结束 URL 段）。
        let mut end = scheme_end + 3;
        while end < bytes.len() {
            let current = text[end..].chars().next().unwrap_or('\0');
            if !is_url_tail_char(current) {
                break;
            }
            end += current.len_utf8();
        }
        // 尾随的常见标点不属于 URL（中文/英文句读）。
        let mut trimmed = end;
        while trimmed > scheme_end + 3 {
            let current = text[..trimmed].chars().next_back().unwrap_or('\0');
            if matches!(current, ',' | ';' | '.' | ')' | ']' | '}' | '：' | ':') {
                trimmed -= current.len_utf8();
            } else {
                break;
            }
        }
        // URL 之前的普通文本原样保留，URL 本体只留摘要标签。
        if start > cursor {
            segments.push((false, &text[cursor..start]));
        }
        segments.push((true, &text[start..trimmed]));
        cursor = trimmed;
        search = trimmed;
    }
    if cursor < bytes.len() {
        segments.push((false, &text[cursor..]));
    }
    segments
}

/// 文本级脱敏：把文本里出现的 URL 片段替换为摘要标签；返回 `(新文本, 替换数)`。
///
/// 用于**非 JSON** 的文本列（例如损坏的 JSON 列、错误摘要）与统一脱敏入口
/// （[`redact_text_urls`]）。扫描规则见 [`scan_url_segments`]；替换文本只含
/// ASCII 与中文，不会破坏 JSON 字符串转义。
pub fn redact_urls_in_text(text: &str) -> (String, usize) {
    let mut output = String::with_capacity(text.len());
    let mut replaced = 0;
    for (is_url, segment) in scan_url_segments(text) {
        if is_url {
            output.push_str(&url_summary(segment).to_label());
            replaced += 1;
        } else {
            output.push_str(segment);
        }
    }
    (output, replaced)
}

/// **统一脱敏入口（文本）**：返回"不含 URL 形态片段"的文本。
///
/// 调用约定（`decisions.md`「OB-9」）：
/// - 任何进入持久化列的文本（`job_stages.last_error`、`provider_attempts.last_error`、
///   `needs_input_json` 等）在**写入前**必须经过本函数；
/// - 任何进入对外 DTO 文本字段（任务详情的 `lastError` 等）与日志 detail 的文本，
///   在**构建处**必须经过本函数（读取侧兜底覆盖历史行）；
/// - 幂等：对已脱敏文本再次调用不改变内容（摘要标签不含 `://`）。
pub fn redact_text_urls(text: &str) -> String {
    redact_urls_in_text(text).0
}

/// JSON 里是否仍含 URL 形态的字符串（导出路径的 fail-closed 校验用）。
///
/// 返回第一个命中的字符串（含 URL，**仅用于错误消息的计数/类型提示**，
/// 调用方不得把返回值写进日志）。命中即说明"该出的字段里有临时地址"。
pub fn first_url_like(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if is_url_like(text) => Some(text.clone()),
        Value::Array(items) => items.iter().find_map(first_url_like),
        Value::Object(map) => map.values().find_map(first_url_like),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summary_keeps_host_and_irreversible_digest_only() {
        let url = "https://cdn.example.invalid/model.glb?sign=canary-signature";
        let summary = url_summary(url);
        assert_eq!(summary.host.as_deref(), Some("cdn.example.invalid"));
        assert_eq!(summary.sha256_prefix.len(), URL_SUMMARY_SHA256_CHARS);
        assert!(summary.sha256_prefix.chars().all(|c| c.is_ascii_hexdigit()));
        // 同 URL 同摘要；不同签名不同摘要。
        assert_eq!(url_summary(url).sha256_prefix, summary.sha256_prefix);
        assert_ne!(
            url_summary("https://cdn.example.invalid/model.glb?sign=other").sha256_prefix,
            summary.sha256_prefix
        );
        // 摘要对象与标签都不含可用的 URL 子串。
        let value = summary.to_value();
        let rendered = value.to_string();
        assert_eq!(value["redacted"], true);
        assert!(!rendered.contains("://"), "{rendered}");
        assert!(!rendered.contains("canary-signature"), "{rendered}");
        let label = summary.to_label();
        assert!(!label.contains("://"), "{label}");
        assert!(!label.contains("canary-signature"), "{label}");
    }

    #[test]
    fn json_redaction_replaces_urls_anywhere_and_keeps_other_facts() {
        let mut value = json!({
            "remoteTaskId": "task-1",
            "normalizedStatus": "success",
            "progress": 100,
            "modelUrl": "https://cdn.example.invalid/model.glb?sign=canary",
            "nested": {"renderedImageUrl": "http://127.0.0.1:60643/preview.png"},
            "list": ["https://cdn.example.invalid/a.glb?sign=x", "not-a-url", 7],
            "billing": {"creditMinor": 3000},
        });
        let replaced = redact_urls_in_json(&mut value);
        assert_eq!(replaced, 3);
        assert_eq!(value["remoteTaskId"], "task-1");
        assert_eq!(value["normalizedStatus"], "success");
        assert_eq!(value["progress"], 100);
        assert_eq!(value["billing"]["creditMinor"], 3000);
        assert_eq!(value["modelUrl"]["redacted"], true);
        assert_eq!(value["modelUrl"]["host"], "cdn.example.invalid");
        assert_eq!(value["nested"]["renderedImageUrl"]["host"], "127.0.0.1");
        assert_eq!(value["list"][1], "not-a-url");
        assert_eq!(value["list"][2], 7);
        let rendered = value.to_string();
        assert!(!rendered.contains("://"), "{rendered}");
        assert!(!rendered.contains("canary"), "{rendered}");
    }

    #[test]
    fn json_redaction_keeps_sentence_strings_intact() {
        // BUG-011：句子型字符串（URL 只是片段）只替换 URL 片段，不得整串变对象。
        let mut value = json!([
            {
                "code": "download_insecure_scheme",
                "message": "模型下载必须使用 HTTPS（实际 http://cdn.example.invalid/m.glb）：拒绝下载",
            },
            {"code": "other", "message": "该批不产生正式知识（不自动重试）"}
        ]);
        let replaced = redact_urls_in_json(&mut value);
        assert_eq!(replaced, 1);
        let message = value[0]["message"].as_str().expect("message 仍是字符串");
        assert_eq!(value[0]["code"], "download_insecure_scheme", "code 不变");
        assert!(
            message.starts_with("模型下载必须使用 HTTPS（实际 "),
            "{message}"
        );
        assert!(
            message.ends_with("）：拒绝下载"),
            "URL 之后的文本必须保留：{message}"
        );
        assert!(!message.contains("://"), "{message}");
        assert!(message.contains("host=cdn.example.invalid"), "{message}");
        // 同列无 URL 的其它条目原样保留（整列不因一条命中而丢失）。
        assert_eq!(
            value[1]["message"], "该批不产生正式知识（不自动重试）",
            "{value}"
        );
        // 序列化后仍是合法 JSON（不得因脱敏产生非法 JSON）。
        let parsed: Value = serde_json::from_str(&value.to_string()).expect("脱敏后仍是合法 JSON");
        assert!(parsed[0]["message"].is_string());
    }

    #[test]
    fn json_redaction_keep_strings_never_changes_value_type() {
        // `needs_input_json.message` 的契约是字符串：整串 URL 也只留摘要标签。
        let mut value = json!([
            {"code": "download_transport", "message": "https://cdn.example.invalid/m.glb?sign=canary"}
        ]);
        let replaced = redact_urls_in_json_with(&mut value, JsonStringRedaction::KeepString);
        assert_eq!(replaced, 1);
        let message = value[0]["message"].as_str().expect("仍是字符串");
        assert!(!message.contains("://"), "{message}");
        assert!(!message.contains("canary"), "{message}");
        assert!(message.contains("host=cdn.example.invalid"), "{message}");
        // 对照：SummaryObject 形态下同一输入会变成摘要对象（usage_json 的既有语义）。
        let mut summary_value = json!([
            {"code": "download_transport", "message": "https://cdn.example.invalid/m.glb?sign=canary"}
        ]);
        assert_eq!(
            redact_urls_in_json_with(&mut summary_value, JsonStringRedaction::SummaryObject),
            1
        );
        assert_eq!(summary_value[0]["message"]["redacted"], true);
    }

    #[test]
    fn serialized_json_text_redaction_parses_or_falls_back_to_text() {
        // 合法 JSON：结构不变、逐字符串值替换。
        let text = serde_json::to_string(&json!({
            "errorSummary": "模型拒答（refusal）：下载参考 https://cdn.example.invalid/p.png?sign=canary 也失败",
            "remoteTaskId": "task-9",
            "billing": {"creditMinor": 3000},
        }))
        .expect("序列化");
        let (redacted, count) = redact_json_text_urls(&text, JsonStringRedaction::SummaryObject);
        assert_eq!(count, 1);
        let parsed: Value = serde_json::from_str(&redacted).expect("仍是合法 JSON");
        assert_eq!(parsed["remoteTaskId"], "task-9", "id 事实保留");
        assert_eq!(parsed["billing"]["creditMinor"], 3000, "计费事实保留");
        let summary = parsed["errorSummary"].as_str().expect("句子仍是字符串");
        assert!(summary.ends_with("也失败"), "{summary}");
        assert!(!summary.contains("://"), "{summary}");

        // 文本无 URL：原样返回（零改写）。
        let plain = "{\"errorSummary\":\"该批不产生正式知识\"}";
        assert_eq!(
            redact_json_text_urls(plain, JsonStringRedaction::SummaryObject),
            (plain.to_owned(), 0)
        );

        // 损坏的 JSON 文本：退化为文本级兜底，仍不含 URL 形态。
        let broken = "不是 JSON 但含 https://cdn.example.invalid/x.glb?sign=canary 片段";
        let (redacted, count) = redact_json_text_urls(broken, JsonStringRedaction::SummaryObject);
        assert_eq!(count, 1);
        assert!(!redacted.contains("://"), "{redacted}");
        assert!(redacted.ends_with(" 片段"), "{redacted}");
    }

    #[test]
    fn whole_url_detection_requires_exactly_one_url_segment() {
        // 前后空白不算"其它文本"；多个 URL 或混有文字都不算整串。
        assert!(whole_url_summary("https://cdn.example.invalid/a.glb?x=1").is_some());
        assert!(whole_url_summary("  https://cdn.example.invalid/a.glb  ").is_some());
        assert!(whole_url_summary("https://a.example/x https://b.example/y").is_none());
        assert!(whole_url_summary("下载 https://a.example/x").is_none());
        assert!(whole_url_summary("奇怪的 :// 片段").is_none());
    }

    #[test]
    fn text_redaction_strips_url_runs_and_counts() {
        let (redacted, replaced) = redact_urls_in_text(
            "链接过期：https://cdn.example.invalid/model.glb?sign=canary 返回 403，请重试",
        );
        assert_eq!(replaced, 1);
        assert!(!redacted.contains("://"), "{redacted}");
        assert!(!redacted.contains("canary"), "{redacted}");
        assert!(redacted.contains("cdn.example.invalid"), "{redacted}");
        assert!(redacted.starts_with("链接过期："), "{redacted}");
        assert!(redacted.ends_with("返回 403，请重试"), "{redacted}");

        // 无 URL / 裸 `://` 不被破坏。
        let (same, count) = redact_urls_in_text("没有链接（plain text）");
        assert_eq!((same.as_str(), count), ("没有链接（plain text）", 0));
        let (bare, count) = redact_urls_in_text("奇怪的 :// 片段");
        assert_eq!((bare.as_str(), count), ("奇怪的 :// 片段", 0));
    }

    #[test]
    fn text_redaction_keeps_adjacent_non_url_text() {
        // BUG-010（QA 回合 26）：URL 后紧邻中文，中间无分隔符 → 不得吞掉"已过期"。
        let (redacted, replaced) =
            redact_urls_in_text("链接 https://cdn.example.invalid/x.glb?sign=x已过期，请重试");
        assert_eq!(replaced, 1);
        assert!(redacted.ends_with("已过期，请重试"), "{redacted}");
        assert!(!redacted.contains("://"), "{redacted}");
        assert!(!redacted.contains("sign=x"), "{redacted}");
        assert!(redacted.contains("host=cdn.example.invalid"), "{redacted}");
        assert!(redacted.starts_with("链接 "), "{redacted}");

        // 无查询串、尾部紧邻中文/全角标点/引号/尖括号：同样只替换 URL 本体。
        let (redacted, replaced) = redact_urls_in_text(
            "见（https://cdn.example.invalid/a.glb）与 \"https://x.example/b\"",
        );
        assert_eq!(replaced, 2);
        assert!(redacted.contains("）与 \""), "{redacted}");
        assert!(!redacted.contains("://"), "{redacted}");

        // 尾部句读不属于 URL。
        let (redacted, replaced) = redact_urls_in_text("下载 https://cdn.example.invalid/a.glb。");
        assert_eq!(replaced, 1);
        assert!(redacted.ends_with("。"), "{redacted}");
        let (redacted, _) = redact_urls_in_text("见 https://cdn.example.invalid/a.glb.");
        assert!(redacted.ends_with('.'), "{redacted}");

        // fragment 一起替换（不留 `#frag` 残片）。
        let (redacted, replaced) =
            redact_urls_in_text("https://cdn.example.invalid/a.glb#frag 已失效");
        assert_eq!(replaced, 1);
        assert!(!redacted.contains("#frag"), "{redacted}");
        assert!(redacted.ends_with("已失效"), "{redacted}");

        // 百分号编码属于 URL 本体（不提前截断）。
        let (redacted, replaced) =
            redact_urls_in_text("https://cdn.example.invalid/a%20b.glb?q=1&r=2 完毕");
        assert_eq!(replaced, 1);
        assert!(redacted.ends_with("完毕"), "{redacted}");
        assert!(!redacted.contains("q=1"), "{redacted}");
    }

    #[test]
    fn redact_text_urls_is_the_unified_idempotent_entry() {
        let text =
            "传输失败：error sending request for url (https://cdn.example.invalid/m.glb?sign=s1)";
        let once = redact_text_urls(text);
        assert!(!once.contains("://"), "{once}");
        assert!(!once.contains("sign=s1"), "{once}");
        assert!(once.contains("host=cdn.example.invalid"), "{once}");
        // 幂等：摘要标签再次扫描不产生变化。
        assert_eq!(redact_text_urls(&once), once);
        // 无 URL 的文本原样返回（task_id/计数/摘要保留）。
        let plain = "模型下载链接已过期（HTTP 403）：将重新查询 task-12345（不重新购买）";
        assert_eq!(redact_text_urls(plain), plain);
    }

    #[test]
    fn redacting_serialized_json_text_keeps_it_parseable() {
        // 仓储层对 `needs_input_json` 等序列化文本做兜底：标签不得破坏 JSON 转义。
        let json_text = serde_json::to_string(&json!([
            {
                "code": "download_transport",
                "message": "连接失败：error sending request for url (https://cdn.example.invalid/m.glb?sign=canary)",
            },
            {"code": "retry", "message": "下载可安全重试（task-9）"}
        ]))
        .expect("序列化");
        let redacted = redact_text_urls(&json_text);
        assert!(!redacted.contains("://"), "{redacted}");
        assert!(!redacted.contains("canary"), "{redacted}");
        let parsed: Value = serde_json::from_str(&redacted).expect("脱敏后仍是合法 JSON");
        assert_eq!(parsed[0]["code"], "download_transport");
        assert!(
            parsed[0]["message"]
                .as_str()
                .expect("message 是字符串")
                .contains("host=cdn.example.invalid")
        );
        assert!(parsed[1]["message"].as_str().unwrap().contains("task-9"));
    }

    #[test]
    fn first_url_like_detects_and_reports() {
        assert!(first_url_like(&json!({"a": 1})).is_none());
        assert!(first_url_like(&json!({"a": {"b": "https://x.example/y?q=1"}})).is_some());
    }
}
