//! 原创样例资产的结构级校验（AC-014 第 5 项：GLB 可结构解析、PDF 页数与类型
//! 符合预期、图片可解码）。
//!
//! 这里是**结构校验**而不是完整解码器：GLB 按 glTF 2.0 二进制容器规则逐 chunk
//! 解析并检查 bufferView/accessor 范围；PDF 解析 startxref/xref/trailer/catalog/
//! pages；PNG 校验 chunk CRC；JPEG 走 marker 链。完整像素级解码由实现记录中的
//! 平台解码器（如 macOS `sips`）单独给出证据，测试不依赖外部程序。

use serde_json::Value;
use sha2::{Digest, Sha256};

/// 字节的 sha256 十六进制（小写）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ---------------------------------------------------------------------------
// GLB
// ---------------------------------------------------------------------------

/// GLB 结构信息（面数/贴图尺寸用于对照 PRD §5.3 预算）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlbInfo {
    pub version: u32,
    pub json_len: u32,
    pub bin_len: u32,
    pub triangles: usize,
    pub meshes: usize,
    pub nodes: usize,
    pub image_count: usize,
    /// 内嵌贴图的最大单边像素（无贴图为 0）。
    pub max_texture_dimension: u32,
}

/// 解析 GLB 容器与 glTF JSON，校验合同要求的自包含与范围约束。
pub fn validate_glb(bytes: &[u8]) -> Result<GlbInfo, String> {
    if bytes.len() < 12 {
        return Err("文件短于 GLB 头（12 字节）".to_owned());
    }
    if &bytes[0..4] != b"glTF" {
        return Err(format!("magic 不是 glTF：{:?}", &bytes[0..4]));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("4 字节"));
    if version != 2 {
        return Err(format!("GLB 版本不是 2：{version}"));
    }
    let total_len = u32::from_le_bytes(bytes[8..12].try_into().expect("4 字节"));
    if total_len as usize != bytes.len() {
        return Err(format!(
            "声明长度 {total_len} 与文件实际长度 {} 不一致",
            bytes.len()
        ));
    }

    let mut offset = 12_usize;
    let mut json_chunk: Option<&[u8]> = None;
    let mut bin_chunk: Option<&[u8]> = None;
    let mut chunk_index = 0;
    while offset < bytes.len() {
        if offset + 8 > bytes.len() {
            return Err(format!("chunk #{chunk_index} 头不完整（offset={offset}）"));
        }
        let chunk_len =
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("4 字节")) as usize;
        let chunk_type = &bytes[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start + chunk_len;
        if data_end > bytes.len() {
            return Err(format!(
                "chunk #{chunk_index} 声明长度 {chunk_len} 超出文件（offset={data_start}）"
            ));
        }
        match chunk_type {
            b"JSON" => {
                if json_chunk.is_some() {
                    return Err("出现第二个 JSON chunk".to_owned());
                }
                json_chunk = Some(&bytes[data_start..data_end]);
            }
            b"BIN\0" => {
                if bin_chunk.is_some() {
                    return Err("出现第二个 BIN chunk".to_owned());
                }
                bin_chunk = Some(&bytes[data_start..data_end]);
            }
            other => {
                return Err(format!("未知 chunk 类型：{other:?}"));
            }
        }
        offset = data_end;
        chunk_index += 1;
    }
    let json_chunk = json_chunk.ok_or_else(|| "缺少 JSON chunk".to_owned())?;
    let json: Value = serde_json::from_slice(json_chunk)
        .map_err(|error| format!("JSON chunk 不是合法 JSON：{error}"))?;

    let asset_version = json["asset"]["version"]
        .as_str()
        .ok_or_else(|| "缺少 asset.version".to_owned())?;
    if asset_version != "2.0" {
        return Err(format!("asset.version 不是 2.0：{asset_version}"));
    }

    let buffers = json["buffers"]
        .as_array()
        .ok_or_else(|| "缺少 buffers".to_owned())?;
    if buffers.len() != 1 {
        return Err(format!("期望恰好 1 个 buffer，实际 {}", buffers.len()));
    }
    let declared_buffer_len = buffers[0]["byteLength"]
        .as_u64()
        .ok_or_else(|| "buffer 缺少 byteLength".to_owned())? as usize;
    if buffers[0].get("uri").is_some() {
        return Err("buffer 使用外链 uri（合同要求自包含）".to_owned());
    }
    let bin_chunk = bin_chunk.ok_or_else(|| "缺少 BIN chunk（自包含模型必需）".to_owned())?;
    if bin_chunk.len() < declared_buffer_len || bin_chunk.len() - declared_buffer_len > 3 {
        return Err(format!(
            "BIN chunk 长度 {} 与 buffer.byteLength {} 不符（允许 ≤3 字节对齐填充）",
            bin_chunk.len(),
            declared_buffer_len
        ));
    }
    let declared_buffer_len =
        u32::try_from(declared_buffer_len).map_err(|_| "buffer.byteLength 超出 u32".to_owned())?;
    let bin_len = u32::try_from(bin_chunk.len()).map_err(|_| "BIN chunk 超出 u32".to_owned())?;

    let buffer_views = json["bufferViews"]
        .as_array()
        .ok_or_else(|| "缺少 bufferViews".to_owned())?;
    for (index, view) in buffer_views.iter().enumerate() {
        let buffer = view["buffer"].as_u64().unwrap_or(u64::MAX);
        if buffer != 0 {
            return Err(format!(
                "bufferView #{index} 引用了不存在的 buffer {buffer}"
            ));
        }
        let view_offset = view["byteOffset"].as_u64().unwrap_or(0);
        let view_len = view["byteLength"]
            .as_u64()
            .ok_or_else(|| format!("bufferView #{index} 缺少 byteLength"))?;
        if view_offset + view_len > u64::from(declared_buffer_len) {
            return Err(format!(
                "bufferView #{index} 范围 {view_offset}+{view_len} 超出 buffer（{declared_buffer_len} 字节）"
            ));
        }
    }

    // accessor 范围（含 stride）必须在 bufferView 内。
    let accessors = json["accessors"].as_array().cloned().unwrap_or_default();
    for (index, accessor) in accessors.iter().enumerate() {
        let view_index = accessor["bufferView"]
            .as_u64()
            .ok_or_else(|| format!("accessor #{index} 缺少 bufferView（不支持 sparse/外链）"))?;
        let view = buffer_views
            .get(view_index as usize)
            .ok_or_else(|| format!("accessor #{index} 引用不存在的 bufferView #{view_index}"))?;
        let view_len = view["byteLength"].as_u64().unwrap_or(0);
        let count = accessor["count"]
            .as_u64()
            .ok_or_else(|| format!("accessor #{index} 缺少 count"))?;
        let component_size = match accessor["componentType"].as_u64() {
            Some(5120 | 5121) => 1_u64,
            Some(5122 | 5123) => 2,
            Some(5125 | 5126) => 4,
            other => return Err(format!("accessor #{index} componentType 非法：{other:?}")),
        };
        let components_per_element = match accessor["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            other => return Err(format!("accessor #{index} type 非法：{other:?}")),
        };
        let element_size = component_size * components_per_element;
        let accessor_offset = accessor["byteOffset"].as_u64().unwrap_or(0);
        let stride = accessor["byteStride"].as_u64().unwrap_or(element_size);
        if count > 0 {
            let last_byte = accessor_offset + stride * (count - 1) + element_size;
            if last_byte > view_len {
                return Err(format!(
                    "accessor #{index} 需要 {last_byte} 字节，bufferView #{view_index} 只有 {view_len} 字节"
                ));
            }
        }
    }

    // 图片必须内嵌（bufferView）且尺寸在预算内。
    let images = json["images"].as_array().cloned().unwrap_or_default();
    let mut max_texture_dimension = 0_u32;
    for (index, image) in images.iter().enumerate() {
        if image.get("uri").is_some() {
            return Err(format!("image #{index} 使用外链 uri（合同要求自包含）"));
        }
        let view_index = image["bufferView"]
            .as_u64()
            .ok_or_else(|| format!("image #{index} 缺少 bufferView"))?;
        let view = buffer_views
            .get(view_index as usize)
            .ok_or_else(|| format!("image #{index} 引用不存在的 bufferView #{view_index}"))?;
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = view["byteLength"].as_u64().unwrap_or(0) as usize;
        let data = &bin_chunk[start..start + len];
        let png = validate_png(data)
            .map_err(|error| format!("image #{index} 不是可解析 PNG：{error}"))?;
        max_texture_dimension = max_texture_dimension.max(png.width.max(png.height));
    }

    // 三角面数：mode 缺省 4（TRIANGLES）。
    let mut triangles = 0_usize;
    let meshes = json["meshes"].as_array().cloned().unwrap_or_default();
    for mesh in &meshes {
        for primitive in mesh["primitives"].as_array().cloned().unwrap_or_default() {
            if primitive.get("indices").is_none() {
                return Err("primitive 缺少 indices（fixture 模型要求显式索引）".to_owned());
            }
            if primitive["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            // glTF 中 `primitive.indices` 是 accessor **编号**（整数），不是对象。
            let accessor_index = primitive["indices"]
                .as_u64()
                .ok_or_else(|| "primitive.indices 不是 accessor 编号".to_owned())?;
            let count = accessors
                .get(accessor_index as usize)
                .and_then(|accessor| accessor["count"].as_u64())
                .ok_or_else(|| "indices accessor 不存在".to_owned())?;
            triangles += (count / 3) as usize;
        }
    }
    let nodes = json["nodes"].as_array().map(Vec::len).unwrap_or(0);
    Ok(GlbInfo {
        version,
        json_len: json_chunk.len() as u32,
        bin_len,
        triangles,
        meshes: meshes.len(),
        nodes,
        image_count: images.len(),
        max_texture_dimension,
    })
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

/// PDF 结构信息（页数与"文字层/扫描页"判定用于 T09/T14）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfInfo {
    pub version: String,
    pub page_count: usize,
    pub declared_count: usize,
    pub xref_entries_checked: usize,
    /// 内容流中是否出现 `BT`（begin text）——文字型 PDF 为 true。
    pub has_text_operators: bool,
    /// `/Subtype /Image` 出现的次数——扫描型 PDF 页图为栅格。
    pub image_xobjects: usize,
    /// 页图的最大单边像素。
    pub max_image_dimension: u32,
}

/// 解析 PDF 结构：header、startxref、xref 表、trailer/Root、catalog/Pages/Kids。
///
/// 不做渲染、不做完整对象模型；对测试用的小型线性化无关 PDF 足够严格。
pub fn validate_pdf(bytes: &[u8]) -> Result<PdfInfo, String> {
    let text = ascii_projection(bytes);
    if !text.starts_with("%PDF-") {
        return Err("缺少 %PDF- 头".to_owned());
    }
    let version = text[5..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>();
    if version.is_empty() {
        return Err("PDF 版本号缺失".to_owned());
    }
    if !text.trim_end().ends_with("%%EOF") {
        return Err("缺少 %%EOF 结尾".to_owned());
    }

    // startxref → xref 表。
    let startxref_pos = text
        .rfind("startxref")
        .ok_or_else(|| "缺少 startxref".to_owned())?;
    let tail = &text[startxref_pos + "startxref".len()..];
    let xref_offset: usize = tail
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| "startxref 偏移非法".to_owned())?;
    if xref_offset >= text.len() || !text[xref_offset..].starts_with("xref") {
        return Err(format!("startxref 指向的偏移 {xref_offset} 不是 xref 表"));
    }

    // xref 条目：offset(10) 空格 gen(5) 空格 flag，共计 20 字节一行。
    let mut cursor = xref_offset + "xref".len();
    let mut xref_entries_checked = 0_usize;
    loop {
        while text[cursor..].starts_with(['\r', '\n', ' ']) {
            cursor += 1;
        }
        if text[cursor..].starts_with("trailer") {
            break;
        }
        let line_end = text[cursor..]
            .find(['\r', '\n'])
            .ok_or_else(|| "xref 小节头不完整".to_owned())?
            + cursor;
        let header = &text[cursor..line_end];
        let mut parts = header.split_whitespace();
        let first: usize = parts
            .next()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| format!("xref 小节头非法：{header:?}"))?;
        let count: usize = parts
            .next()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| format!("xref 小节计数非法：{header:?}"))?;
        cursor = line_end + 1;
        for entry_index in 0..count {
            if cursor + 20 > text.len() {
                return Err("xref 条目越界".to_owned());
            }
            let entry = &text[cursor..cursor + 20];
            let offset: usize = entry[..10]
                .trim()
                .parse()
                .map_err(|_| format!("xref 条目偏移非法：{entry:?}"))?;
            let flag = entry.as_bytes()[17];
            if flag == b'n' && !(first == 0 && entry_index == 0) {
                let object_start = text
                    .get(offset..)
                    .ok_or_else(|| format!("xref 偏移 {offset} 越界"))?;
                let looks_like_object = object_start
                    .split_whitespace()
                    .next()
                    .is_some_and(|value| value.chars().all(|c| c.is_ascii_digit()))
                    && object_start.contains(" obj");
                if !looks_like_object {
                    return Err(format!(
                        "xref 偏移 {offset} 未指向对象头：{:?}",
                        &object_start[..object_start.len().min(24)]
                    ));
                }
                xref_entries_checked += 1;
            }
            cursor += 20;
        }
    }

    // trailer 字典 → Root → catalog → Pages。
    let trailer_end = text[cursor..]
        .find(">>")
        .ok_or_else(|| "trailer 字典不完整".to_owned())?
        + cursor;
    let trailer = &text[cursor..trailer_end];
    let root_ref = dict_ref(trailer, "/Root").ok_or_else(|| "trailer 缺少 /Root".to_owned())?;
    let root = object_body(&text, root_ref).ok_or_else(|| "找不到 Root 对象".to_owned())?;
    if !root.contains("/Type /Catalog") && !root.contains("/Type/Catalog") {
        return Err("Root 对象不是 /Catalog".to_owned());
    }
    let pages_ref = dict_ref(root, "/Pages").ok_or_else(|| "catalog 缺少 /Pages".to_owned())?;
    let pages = object_body(&text, pages_ref).ok_or_else(|| "找不到 Pages 对象".to_owned())?;
    // `/Count` 允许是间接引用（`/Count 3 0 R`）：真实 PDF 常见。跟随一次，
    // 否则"声明页数"会被误读（T06 的上传探针刻意不解析间接引用，权威判定在 T09）。
    let declared_count = match indirect_ref_after(pages, "/Count") {
        Some(number) => object_body(&text, number)
            .and_then(|body| body.trim().parse::<usize>().ok())
            .unwrap_or(0),
        None => dict_value_usize(pages, "/Count").unwrap_or(0),
    };
    let kids = dict_array_refs(pages, "/Kids").ok_or_else(|| "Pages 缺少 /Kids".to_owned())?;
    if kids.is_empty() {
        return Err("Pages /Kids 为空".to_owned());
    }
    for kid in &kids {
        let body = object_body(&text, *kid).ok_or_else(|| format!("找不到页对象 {kid} 0 R"))?;
        if !body.contains("/Type /Page") && !body.contains("/Type/Page") {
            // /Type /Pages 出现在嵌套节点时不算页。
            return Err(format!("Kids 中的对象 {kid} 不是 /Type /Page"));
        }
    }
    let page_count = kids.len();

    // 内容流扫描：BT/Tj（文字层）与 /Subtype /Image（栅格页图）。
    let mut has_text_operators = false;
    let mut image_xobjects = 0_usize;
    let mut max_image_dimension = 0_u32;
    let mut search_from = 0_usize;
    while let Some(start) = text[search_from..].find("/Subtype /Image") {
        let absolute = search_from + start;
        image_xobjects += 1;
        let window_end = (absolute + 512).min(text.len());
        let window = &text[absolute..window_end];
        let width = dict_value_usize(window, "/Width").unwrap_or(0) as u32;
        let height = dict_value_usize(window, "/Height").unwrap_or(0) as u32;
        max_image_dimension = max_image_dimension.max(width.max(height));
        search_from = absolute + "/Subtype /Image".len();
    }
    for segment in stream_segments(&text) {
        if segment.contains("BT") || segment.contains("Tj") {
            has_text_operators = true;
        }
    }

    Ok(PdfInfo {
        version,
        page_count,
        declared_count,
        xref_entries_checked,
        has_text_operators,
        image_xobjects,
        max_image_dimension,
    })
}

/// 把字节投影为**等长**的 ASCII 文本：非 ASCII 字节替换为 `?`（1 字节 → 1 字符）。
///
/// 为什么不用 `from_utf8_lossy`：它把非法序列替换成 U+FFFD（3 字节），会改变字符索引与
/// 字节偏移的对应关系，使得**含二进制流的 PDF（例如加密样例的 RC4 密文）**的
/// startxref/xref 偏移校验全部错位。本投影保持偏移 1:1，因此结构校验对二进制流安全；
/// 纯 ASCII 的 PDF 与本函数引入前的结果完全一致。
fn ascii_projection(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        if byte.is_ascii() {
            out.push(byte as char);
        } else {
            out.push('?');
        }
    }
    out
}

/// 取 `stream ... endstream` 之间的段落（原样切片）。
fn stream_segments(text: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut cursor = 0_usize;
    while let Some(start) = text[cursor..].find("stream") {
        let absolute = cursor + start;
        let after_keyword = absolute + "stream".len();
        let data_start = text[after_keyword..]
            .find('\n')
            .map(|offset| after_keyword + offset + 1);
        let Some(data_start) = data_start else { break };
        let Some(end) = text[data_start..].find("endstream") else {
            break;
        };
        segments.push(&text[data_start..data_start + end]);
        cursor = data_start + end + "endstream".len();
    }
    segments
}

/// `dict` 中的 `/Key N 0 R` 引用。
fn dict_ref(dict: &str, key: &str) -> Option<usize> {
    let start = dict.find(key)? + key.len();
    dict[start..]
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
}

/// `dict` 中的 `/Key [a b c]` 引用数组。
fn dict_array_refs(dict: &str, key: &str) -> Option<Vec<usize>> {
    let start = dict.find(key)? + key.len();
    let rest = &dict[start..];
    let open = rest.find('[')?;
    let close = rest.find(']')?;
    let inner = &rest[open + 1..close];
    let tokens: Vec<&str> = inner.split_whitespace().collect();
    let mut refs = Vec::new();
    let mut index = 0;
    while index + 2 < tokens.len() {
        if tokens[index + 1] == "0" && tokens[index + 2] == "R" {
            if let Ok(number) = tokens[index].parse::<usize>() {
                refs.push(number);
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Some(refs)
}

/// `dict` 中的 `/Key N` 数值。
/// 若 `dict` 中 `/Key` 的值是间接引用（`N 0 R`），返回对象号 N。
fn indirect_ref_after(dict: &str, key: &str) -> Option<usize> {
    let start = dict.find(key)? + key.len();
    let tokens: Vec<&str> = dict[start..].split_whitespace().take(3).collect();
    if tokens.len() == 3 && tokens[1] == "0" && tokens[2] == "R" {
        return tokens[0].parse().ok();
    }
    None
}

fn dict_value_usize(dict: &str, key: &str) -> Option<usize> {
    let start = dict.find(key)? + key.len();
    dict[start..]
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
}

/// 按 xref 给出的对象号取 `N 0 obj ... endobj` 正文。
fn object_body(text: &str, object_number: usize) -> Option<&str> {
    let marker = format!("{object_number} 0 obj");
    let start = text.find(&marker)? + marker.len();
    let end = text[start..].find("endobj")? + start;
    Some(&text[start..end])
}

// ---------------------------------------------------------------------------
// PNG / JPEG
// ---------------------------------------------------------------------------

/// PNG 结构信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub chunk_count: usize,
}

/// 校验 PNG 签名、chunk 布局与每个 chunk 的 CRC。
pub fn validate_png(bytes: &[u8]) -> Result<PngInfo, String> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 8 || &bytes[..8] != SIGNATURE {
        return Err("PNG 签名不符".to_owned());
    }
    let mut offset = 8_usize;
    let mut chunk_count = 0_usize;
    let mut info: Option<PngInfo> = None;
    let mut saw_iend = false;
    while offset + 12 <= bytes.len() {
        let length =
            u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("4 字节")) as usize;
        let chunk_type = &bytes[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start + length;
        if data_end + 4 > bytes.len() {
            return Err(format!("chunk 数据越界（type={chunk_type:?}）"));
        }
        let declared_crc =
            u32::from_be_bytes(bytes[data_end..data_end + 4].try_into().expect("4 字节"));
        let actual_crc = crc32_ieee(&bytes[offset + 4..data_end]);
        if declared_crc != actual_crc {
            return Err(format!(
                "chunk {chunk_type:?} CRC 不符：声明 {declared_crc:#010x}，实际 {actual_crc:#010x}"
            ));
        }
        match chunk_type {
            b"IHDR" => {
                if info.is_some() {
                    return Err("出现第二个 IHDR".to_owned());
                }
                if length != 13 {
                    return Err(format!("IHDR 长度应为 13，实际 {length}"));
                }
                let data = &bytes[data_start..data_end];
                info = Some(PngInfo {
                    width: u32::from_be_bytes(data[0..4].try_into().expect("4 字节")),
                    height: u32::from_be_bytes(data[4..8].try_into().expect("4 字节")),
                    bit_depth: data[8],
                    color_type: data[9],
                    chunk_count: 0,
                });
            }
            b"IEND" => {
                if length != 0 {
                    return Err("IEND 数据段应为空".to_owned());
                }
                saw_iend = true;
            }
            _ => {}
        }
        chunk_count += 1;
        offset = data_end + 4;
        if saw_iend {
            break;
        }
    }
    let mut info = info.ok_or_else(|| "缺少 IHDR".to_owned())?;
    if !saw_iend {
        return Err("缺少 IEND".to_owned());
    }
    if offset != bytes.len() {
        return Err("IEND 之后存在多余字节".to_owned());
    }
    info.chunk_count = chunk_count;
    Ok(info)
}

/// JPEG 结构信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegInfo {
    pub width: u16,
    pub height: u16,
    pub components: u8,
    pub progressive: bool,
    pub has_huffman_tables: bool,
    pub has_quantization_tables: bool,
}

/// 走 marker 链校验 JPEG（SOI → …SOF…/DHT/DQT/SOS… → EOI）。
pub fn validate_jpeg(bytes: &[u8]) -> Result<JpegInfo, String> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return Err("缺少 SOI（FFD8）".to_owned());
    }
    if !bytes.ends_with(&[0xFF, 0xD9]) {
        return Err("缺少 EOI（FFD9）".to_owned());
    }
    let mut offset = 2_usize;
    let mut info: Option<JpegInfo> = None;
    let mut has_huffman_tables = false;
    let mut has_quantization_tables = false;
    while offset + 1 < bytes.len() {
        if bytes[offset] != 0xFF {
            return Err(format!("marker 位置 {offset} 不是 0xFF"));
        }
        let marker = bytes[offset + 1];
        offset += 2;
        match marker {
            // 填充字节。
            0xFF => {
                offset -= 1;
                continue;
            }
            // 无长度段：TEM、RSTn。
            0x01 | 0xD0..=0xD7 => continue,
            // EOI 之后不再解析。
            0xD9 => break,
            // SOS：熵编码数据直到 EOI，此处不再逐字节解析。
            0xDA => break,
            _ => {}
        }
        if offset + 2 > bytes.len() {
            return Err(format!("marker {marker:#04x} 长度字段缺失"));
        }
        let length =
            u16::from_be_bytes(bytes[offset..offset + 2].try_into().expect("2 字节")) as usize;
        if length < 2 || offset + length > bytes.len() {
            return Err(format!("marker {marker:#04x} 长度非法：{length}"));
        }
        let data = &bytes[offset + 2..offset + length];
        match marker {
            0xC0..=0xC2 => {
                if data.len() < 6 {
                    return Err("SOF 数据段过短".to_owned());
                }
                info = Some(JpegInfo {
                    height: u16::from_be_bytes(data[1..3].try_into().expect("2 字节")),
                    width: u16::from_be_bytes(data[3..5].try_into().expect("2 字节")),
                    components: data[5],
                    progressive: marker == 0xC2,
                    has_huffman_tables,
                    has_quantization_tables,
                });
            }
            0xC4 => has_huffman_tables = true,
            0xDB => has_quantization_tables = true,
            _ => {}
        }
        offset += length;
    }
    let mut info = info.ok_or_else(|| "缺少 SOF 段".to_owned())?;
    info.has_huffman_tables = has_huffman_tables;
    info.has_quantization_tables = has_quantization_tables;
    Ok(info)
}

/// IEEE CRC-32（PNG chunk 校验用）。
pub fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
