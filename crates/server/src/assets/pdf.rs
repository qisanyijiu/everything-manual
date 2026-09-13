//! PDF 结构探针（T06）：为上传校验提供"可解析 + 页数"判定。
//!
//! 目标与边界（QA 按此复核）：
//! - **只读探测**：确认 `%PDF-` 头、`%%EOF`、`startxref` 指向合法 xref/对象、trailer
//!   中存在 `/Root`；再沿 `/Root → /Pages → /Count` 读页数。所有读取都有上限，
//!   不把 50 MiB 文件读进内存，也不做渲染/完整对象模型（完整准备属 T09）。
//! - **页数上限是第一道防线，权威判定在 T09**（PRD REQ-014：preparation 明确拒绝
//!   超过 100 页与加密 PDF）。因此这里对"结构完好但页树在对象流（`/ObjStm`）中压缩、
//!   无法在本卡范围内解出页数"的文件返回 [`PdfStructure::Unparsed`]，由调用方放行并
//!   记录日志，而不是把真实厂商 PDF 一律判成 422。
//! - **加密 PDF（trailer 带 `/Encrypt`）**：本卡接受上传（REQ-012 明确"拒绝发生在
//!   准备阶段"），页数不可信因此跳过页数上限判定。
//! - 无法满足上述结构（缺少头/尾/startxref/`/Root`）→ `Err`，调用方返回 422。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// 头部窗口：PDF 规范允许文件开头有最多 1024 字节的非 PDF 内容。
const HEADER_WINDOW: u64 = 1024;
/// 尾部窗口：`startxref` / `%%EOF` 必须落在这里。
const TAIL_WINDOW: u64 = 8192;
/// 从 `startxref` 起读的窗口：覆盖经典 xref 表 + trailer 或 XRef 流对象字典。
const NEAR_WINDOW: u64 = 256 * 1024;
/// 单个对象正文读取上限。
const OBJECT_WINDOW: u64 = 64 * 1024;
/// 线性扫描对象头的窗口大小。
const SCAN_CHUNK: u64 = 256 * 1024;

/// PDF 结构判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfStructure {
    /// 页树可读：页数可信。
    Parsed { page_count: u32 },
    /// 结构合法但页树未在本卡范围内定位（例如页树对象被对象流压缩）：放行，页数未知。
    Unparsed { reason: String },
    /// trailer 带 `/Encrypt`：内容与页数被加密，拒绝发生在准备阶段（T09）。
    Encrypted,
}

/// 探测 PDF 结构。`Err` = 不是可解析的 PDF（调用方给 422）。
pub fn probe(path: &Path) -> Result<PdfStructure, String> {
    let mut file = File::open(path).map_err(|error| format!("打开 PDF 失败：{error}"))?;
    let len = file
        .metadata()
        .map_err(|error| format!("读取 PDF 大小失败：{error}"))?
        .len();
    if len < 16 {
        return Err("文件过短，不可能是 PDF".to_owned());
    }

    let head = read_range(&mut file, 0, HEADER_WINDOW.min(len))?;
    if find_subslice(&head, b"%PDF-").is_none() {
        return Err("缺少 %PDF- 头（不是 PDF 内容）".to_owned());
    }

    let tail_start = len.saturating_sub(TAIL_WINDOW);
    let tail = read_range(&mut file, tail_start, len - tail_start)?;
    if find_subslice(&tail, b"%%EOF").is_none() {
        return Err("缺少 %%EOF 结尾（PDF 结构不完整或已截断）".to_owned());
    }

    let Some(startxref) = last_integer_after(&tail, b"startxref") else {
        return Err("缺少 startxref（PDF 结构不完整）".to_owned());
    };
    if startxref >= len {
        return Err(format!("startxref 偏移 {startxref} 超出文件大小 {len}"));
    }

    let near_len = NEAR_WINDOW.min(len - startxref);
    let near = read_range(&mut file, startxref, near_len)?;
    let trimmed = trim_start_ascii_whitespace(&near);
    let xref_table = trimmed.starts_with(b"xref");
    let object_header = object_header_length(trimmed).is_some();
    if !xref_table && !object_header {
        return Err("startxref 未指向 xref 表或对象（PDF 结构不完整）".to_owned());
    }

    // trailer 字典（经典）或 XRef 流对象字典都在 `near` 窗口内；找不到就去尾部重找。
    let root = find_ref(&near, b"/Root").or_else(|| {
        let window_start = len.saturating_sub(NEAR_WINDOW);
        read_range(&mut file, window_start, len - window_start)
            .ok()
            .and_then(|window| find_ref(&window, b"/Root"))
    });
    let Some(root_number) = root else {
        return Err("找不到 trailer 中的 /Root（PDF 结构不完整）".to_owned());
    };
    if find_subslice(&near, b"/Encrypt").is_some() {
        return Ok(PdfStructure::Encrypted);
    }

    let Some(root_offset) = find_object_offset(&mut file, len, root_number) else {
        return Ok(PdfStructure::Unparsed {
            reason: format!("未定位到目录对象 {root_number} 0 R（可能被对象流压缩）"),
        });
    };
    let root_object = read_range(&mut file, root_offset, OBJECT_WINDOW.min(len - root_offset))?;

    let Some(pages_number) = find_ref(&root_object, b"/Pages") else {
        return Ok(PdfStructure::Unparsed {
            reason: "目录对象未引用页树（/Pages）".to_owned(),
        });
    };
    let Some(pages_offset) = find_object_offset(&mut file, len, pages_number) else {
        return Ok(PdfStructure::Unparsed {
            reason: format!("未定位到页树对象 {pages_number} 0 R（可能被对象流压缩）"),
        });
    };
    let pages_object = read_range(
        &mut file,
        pages_offset,
        OBJECT_WINDOW.min(len - pages_offset),
    )?;

    if let Some(count) = find_integer(&pages_object, b"/Count") {
        return Ok(PdfStructure::Parsed { page_count: count });
    }
    let kids = count_refs(&pages_object, b"/Kids");
    if kids > 0 {
        return Ok(PdfStructure::Parsed { page_count: kids });
    }
    Ok(PdfStructure::Unparsed {
        reason: "页树既无 /Count 也无 /Kids".to_owned(),
    })
}

/// 读取 `offset` 起的 `len` 字节（调用方保证范围，越界即错误）。
fn read_range(file: &mut File, offset: u64, len: u64) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| format!("定位 PDF 偏移 {offset} 失败：{error}"))?;
    let mut buffer = vec![0_u8; len as usize];
    file.read_exact(&mut buffer)
        .map_err(|error| format!("读取 PDF 偏移 {offset} 失败：{error}"))?;
    Ok(buffer)
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn trim_start_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !is_whitespace(*byte))
        .unwrap_or(bytes.len());
    &bytes[start..]
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && is_whitespace(bytes[index]) {
        index += 1;
    }
    index
}

/// 朴素的子串查找（窗口 ≤256 KiB，无需引入额外依赖）。
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// `key` 之后第一个 `N 0 R` 引用的对象号（例如 `/Root 7 0 R`）。
fn find_ref(window: &[u8], key: &[u8]) -> Option<u32> {
    let mut search_from = 0_usize;
    while let Some(position) = find_subslice(&window[search_from..], key) {
        let mut index = search_from + position + key.len();
        // 键必须是完整 token（`/Roots` 不算 `/Root`）。
        let next = window.get(index).copied().unwrap_or(b' ');
        if next.is_ascii_alphanumeric() {
            search_from = index;
            continue;
        }
        index = skip_whitespace(window, index);
        let digits_start = index;
        while index < window.len() && window[index].is_ascii_digit() {
            index += 1;
        }
        if digits_start == index {
            search_from = index;
            continue;
        }
        let number = std::str::from_utf8(&window[digits_start..index])
            .ok()
            .and_then(|text| text.parse::<u32>().ok());
        let after_number = skip_whitespace(window, index);
        let generation_ok = window.get(after_number) == Some(&b'0');
        let after_generation = skip_whitespace(window, after_number + 1);
        let reference_ok = window.get(after_generation) == Some(&b'R');
        if let (Some(number), true, true) = (number, generation_ok, reference_ok) {
            return Some(number);
        }
        search_from = index;
    }
    None
}

/// `key` 之后的第一个整数（例如 `/Count 12`）。
fn find_integer(window: &[u8], key: &[u8]) -> Option<u32> {
    let position = find_subslice(window, key)?;
    let mut index = skip_whitespace(window, position + key.len());
    let digits_start = index;
    while index < window.len() && window[index].is_ascii_digit() {
        index += 1;
    }
    if digits_start == index {
        return None;
    }
    std::str::from_utf8(&window[digits_start..index])
        .ok()
        .and_then(|text| text.parse::<u32>().ok())
}

/// 数组 token 数（`/Kids [1 0 R 2 0 R …]` → 2）；只在本卡探针里用于缺失 `/Count` 的兜底。
fn count_refs(window: &[u8], key: &[u8]) -> u32 {
    let Some(position) = find_subslice(window, key) else {
        return 0;
    };
    let rest = &window[position + key.len()..];
    let Some(open) = rest.iter().position(|byte| *byte == b'[') else {
        return 0;
    };
    let Some(close) = rest[open..].iter().position(|byte| *byte == b']') else {
        return 0;
    };
    let inner = &rest[open + 1..open + close];
    let text = String::from_utf8_lossy(inner);
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut count = 0_u32;
    let mut index = 0_usize;
    while index + 2 < tokens.len() {
        if tokens[index + 2] == "R" && tokens[index + 1] == "0" {
            count += 1;
            index += 3;
        } else {
            index += 1;
        }
    }
    count
}

/// 尾部 `startxref` 之后第一个整数（最后一个 `startxref` 才是有效入口）。
fn last_integer_after(window: &[u8], key: &[u8]) -> Option<u64> {
    let position = window.windows(key.len()).rposition(|w| w == key)?;
    let mut index = skip_whitespace(window, position + key.len());
    let digits_start = index;
    while index < window.len() && window[index].is_ascii_digit() {
        index += 1;
    }
    std::str::from_utf8(&window[digits_start..index])
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
}

/// `<number> <gen> obj` 头部的长度（用于判断 `startxref` 是否指向对象）。
fn object_header_length(window: &[u8]) -> Option<usize> {
    let mut index = 0_usize;
    let digits_start = index;
    while index < window.len() && window[index].is_ascii_digit() {
        index += 1;
    }
    if digits_start == index {
        return None;
    }
    index = skip_whitespace(window, index);
    if window.get(index) != Some(&b'0') {
        return None;
    }
    index = skip_whitespace(window, index + 1);
    let keyword = b"obj";
    if window.get(index..index + keyword.len()) != Some(keyword) {
        return None;
    }
    Some(index + keyword.len())
}

/// 线性扫描定位 `<number> 0 obj` 的偏移（有界内存；文件级别的对象流压缩由调用方降级处理）。
fn find_object_offset(file: &mut File, len: u64, number: u32) -> Option<u64> {
    let needle = format!("{number} 0 obj");
    let needle = needle.as_bytes();
    let mut offset = 0_u64;
    let mut carry: Vec<u8> = Vec::new();
    while offset < len {
        let chunk_len = SCAN_CHUNK.min(len - offset);
        let chunk = read_range(file, offset, chunk_len).ok()?;
        let mut buffer = Vec::with_capacity(carry.len() + chunk.len());
        buffer.extend_from_slice(&carry);
        buffer.extend_from_slice(&chunk);
        let base = offset.saturating_sub(carry.len() as u64);

        let mut search_from = 0_usize;
        while let Some(position) = find_subslice(&buffer[search_from..], needle) {
            let index = search_from + position;
            let absolute = base + index as u64;
            let preceded_ok =
                index == 0 && base == 0 || index > 0 && is_whitespace(buffer[index - 1]);
            if preceded_ok {
                let after = index + needle.len();
                let followed_ok = buffer.get(after).is_none_or(|byte| is_whitespace(*byte));
                if followed_ok {
                    return Some(absolute);
                }
            }
            search_from = index + 1;
        }

        let keep = needle.len().min(buffer.len());
        carry = buffer[buffer.len() - keep..].to_vec();
        offset += chunk_len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_ref_requires_full_token_and_reference_shape() {
        let dict = b"<< /Roots 1 0 R /Root 12 0 R /Other 3 >>";
        assert_eq!(find_ref(dict, b"/Root"), Some(12));
        assert_eq!(find_ref(b"<< /Nothing 1 >>", b"/Root"), None);
        assert_eq!(find_ref(b"<< /Root /Pages >>", b"/Root"), None);
    }

    #[test]
    fn find_integer_reads_count() {
        assert_eq!(
            find_integer(b"<< /Type /Pages /Count 42 /Kids [] >>", b"/Count"),
            Some(42)
        );
        assert_eq!(
            find_integer(b"<< /Type /Pages /Kids [] >>", b"/Count"),
            None
        );
    }

    #[test]
    fn count_refs_counts_array_entries() {
        assert_eq!(count_refs(b"/Kids [ 1 0 R 2 0 R ]", b"/Kids"), 2);
        assert_eq!(count_refs(b"/Kids []", b"/Kids"), 0);
        assert_eq!(count_refs(b"/Type /Pages", b"/Kids"), 0);
    }

    #[test]
    fn last_integer_after_uses_last_startxref() {
        let tail = b"startxref\n999\n%%EOF\nstartxref\r\n1234\n%%EOF";
        assert_eq!(last_integer_after(tail, b"startxref"), Some(1234));
    }

    #[test]
    fn object_header_is_recognised() {
        assert_eq!(
            object_header_length(b"7 0 obj\n<< /Type /Catalog >>"),
            Some(7)
        );
        assert!(object_header_length(b"xref\n0 8").is_none());
    }

    #[test]
    fn probe_rejects_non_pdf_bytes() {
        let dir = std::env::temp_dir().join(format!(
            "em-pdf-probe-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("not-a.pdf");
        std::fs::write(&path, b"just some text that is not a pdf at all").unwrap();
        let error = probe(&path).unwrap_err();
        assert!(error.contains("%PDF-"), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
